//! **Taking a product out of circulation** — `cargo rivet fleet retire`,
//! `hide` and `unhide`, and the core `cargo rivet artifacts` drives
//! (`ADR-052` §7, `.rulings/` R-225).
//!
//! A product leaves circulation from THIS repository, never from a fleet: a
//! fleet server's store is a copy of it and holds no decision of its own.
//!
//! # Retire
//!
//! Removes a product's entries, or one version's, from `manifest.toml`, and
//! their objects from `objects/`, and commits the manifest alone. Nothing
//! installed changes. **Refused for an `application` a machine's card carries
//! or pins** — a document under `documents/`, or the register — because the
//! card builder resolves every carried application from this repository, and
//! that machine's next card build would refuse. It reads documents and the
//! register only, and reaches no machine. An `apk` product's signing-key pin
//! lives in its held versions, so the retire prints the signer it drops.
//!
//! # Hide and unhide
//!
//! Adds or removes a `(product, kind)` in the manifest's hidden set. A hide is
//! always allowed: the objects stay here, so a card build still finds them.
//!
//! # Reaching the fleet
//!
//! `--sync` pushes the text, after sending anything the fleet's server lacks,
//! and prints the list the server's prune would delete. **It never prunes**:
//! the deletion from a store is always its own command.

use std::path::Path;
use std::sync::Arc;

use rivet_core::package::port::PublicKey;
use rivet_core::repository_objects::{self as objects, Listing, Manifest};

use super::{Entry, Hidden};

// ---------------------------------------------------------------------------
// which kind
// ---------------------------------------------------------------------------

/// The one kind `product` names here, or `kind` checked against what is held.
///
/// `candidates` is every kind the product is known under — in the manifest
/// for a retire or a hide, in the hidden set for an unhide.
///
/// # Errors
///
/// No kind at all, `kind` not among them, or several with no `kind` to choose
/// between them — each naming what was found.
fn one_kind(product: &str, kind: Option<&str>, mut candidates: Vec<String>, where_: &str) -> Result<String, String> {
    candidates.sort();
    candidates.dedup();
    match kind {
        Some(kind) if candidates.iter().any(|k| k == kind) => Ok(kind.to_owned()),
        Some(kind) if candidates.is_empty() => {
            Err(format!("{where_} holds no {product:?} of kind {kind:?}, nor of any other kind. Nothing was changed."))
        }
        Some(kind) => Err(format!(
            "{where_} holds no {product:?} of kind {kind:?}; it holds it as {}. Nothing was changed.",
            candidates.join(", ")
        )),
        None => match candidates.as_slice() {
            [] => Err(format!(
                "{where_} holds no {product:?}. `cargo rivet fleet list --hidden` shows what it holds. \
                 Nothing was changed."
            )),
            [one] => Ok(one.clone()),
            many => Err(format!(
                "{where_} holds {product:?} as {} kinds — {} — and this will not choose between them. \
                 Name one with --kind. Nothing was changed.",
                many.len(),
                many.join(", ")
            )),
        },
    }
}

fn describe(entry: &Entry) -> String {
    format!(
        "{} {} {} ({}, {}{})",
        entry.digest,
        entry.product,
        entry.version,
        entry.kind,
        entry.arch,
        entry.platform.as_deref().map(|p| format!(", {p}")).unwrap_or_default(),
    )
}

// ---------------------------------------------------------------------------
// who carries it
// ---------------------------------------------------------------------------

/// **One machine whose card depends on a product** — `ADR-050` §5.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Carrier {
    /// The machine, or the fleet as a whole for a fleet-wide pin.
    pub machine: String,
    /// The version it is pinned to, or `None` when it carries whichever single
    /// version this repository holds.
    pub pinned: Option<String>,
    /// Where that was read.
    pub source: String,
    /// How to undo it.
    pub unpin: String,
}

/// **Every machine whose card carries or pins `product`**, from the documents
/// under `<root>/documents/` and from `register` — nothing else is read, and
/// no machine is asked.
///
/// A document carries what it enables and the image does not provide
/// (`rivet_config::Document::carried_services`), and pins what its
/// `[carry.<product>]` names — an inert pin included, since it is still what
/// the document says. The register pins through a device's or the fleet's
/// `carry`, and carries what a device enables or names as its kiosk
/// application. A product the image provides is never carried.
///
/// # Errors
///
/// The documents directory or one document cannot be read, or a document
/// does not parse: what it carries cannot then be known, and a retire decided
/// without it could break that machine's next card build.
pub(super) fn carriers(root: &Path, register: &rivetos_register::Register, product: &str) -> Result<Vec<Carrier>, String> {
    let mut out = Vec::new();
    if rivet_core::service::IMAGE_SERVICE_NAMES.contains(&product) {
        return Ok(out);
    }

    let documents = root.join(rivetos_fleet_repo::DOCUMENTS_DIR);
    if documents.is_dir() {
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&documents)
            .map_err(|e| format!("cannot read {}: {e}", documents.display()))?
            .map(|e| e.map(|e| e.path()).map_err(|e| format!("cannot read {}: {e}", documents.display())))
            .collect::<Result<_, _>>()?;
        files.sort();
        for path in files {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(machine) = name.strip_suffix(rivetos_fleet_repo::DOCUMENT_SUFFIX) else {
                continue;
            };
            let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let document = rivet_config::Document::parse(&text).map_err(|e| {
                format!(
                    "{} does not parse ({e}), so whether {machine}'s card carries {product} cannot be \
                     known. Nothing was changed.",
                    path.display()
                )
            })?;
            let shown = format!("{}/{name}", rivetos_fleet_repo::DOCUMENTS_DIR);
            if let Some(version) = document.carried_version(product) {
                out.push(Carrier {
                    machine: machine.to_owned(),
                    pinned: Some(version.to_owned()),
                    source: format!("{shown} pins it: [carry.{product}] version = {version:?}"),
                    unpin: format!(
                        "remove [carry.{product}] from {shown} (or pin another version) and commit it; a \
                         register-composed document is recomposed by `rivet-fleet set carry --device \
                         {machine} --product {product} --version none`"
                    ),
                });
            }
            if document.carried_services().iter().any(|s| s.name().as_str() == product) {
                out.push(Carrier {
                    machine: machine.to_owned(),
                    pinned: None,
                    source: format!("{shown} enables [services.{product}], so its card carries it"),
                    unpin: format!(
                        "disable [services.{product}] in {shown} and commit it, or `rivet-fleet set \
                         service --device {machine} --service {product} --enabled false`"
                    ),
                });
            }
        }
    }

    let fleet_enables = register.fleet.services.get(product).copied().unwrap_or(false);
    for device in register.devices.values() {
        let name = &device.name;
        if let Some(version) = device.carry.get(product) {
            out.push(Carrier {
                machine: name.clone(),
                pinned: Some(version.clone()),
                source: format!("the register's record for {name} pins it to {version}"),
                unpin: format!("rivet-fleet set carry --device {name} --product {product} --version none"),
            });
        }
        let enabled = device.services.get(product).copied().unwrap_or(fleet_enables);
        if enabled {
            out.push(Carrier {
                machine: name.clone(),
                pinned: None,
                source: format!("the register enables {product} on {name}, so its card carries it"),
                unpin: format!("rivet-fleet set service --device {name} --service {product} --enabled false"),
            });
        }
        if device.kiosk_auto_run.as_deref() == Some(product) {
            out.push(Carrier {
                machine: name.clone(),
                pinned: None,
                source: format!("the register names {product} as {name}'s kiosk auto-run application"),
                unpin: format!("change {name}'s kiosk auto-run in the register (`rivet-fleet`)"),
            });
        }
    }
    if let Some(version) = register.fleet.carry.get(product) {
        out.push(Carrier {
            machine: "every machine (the fleet-wide pin)".to_owned(),
            pinned: Some(version.clone()),
            source: format!("the register's fleet record pins it to {version}"),
            unpin: format!("rivet-fleet set fleet-carry --product {product} --version none"),
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// retire
// ---------------------------------------------------------------------------

/// **Retire `product`, or one `version` of it**, from the fleet repository at
/// `root` — see the module documentation.
///
/// `register` is the workstation's register, as loaded by whoever knows where
/// it lives. `apk_key` is the application key an `apk` product's held versions
/// verify against; it is asked only when an `apk` product's last version goes.
///
/// # Errors
///
/// `root` is not a fleet repository; the product, kind or version is not held,
/// or the kind is ambiguous; a machine carries or pins what would go (naming
/// each and how to unpin); or the manifest cannot be written and committed.
/// Nothing is changed by any refusal.
pub(super) fn retire(
    root: &Path,
    product: &str,
    kind: Option<&str>,
    version: Option<&str>,
    register: &rivetos_register::Register,
    apk_key: &mut dyn FnMut() -> Result<PublicKey, String>,
) -> Result<String, String> {
    super::require_repository(root)?;
    let entries = super::read_manifest(root)?;
    let mut hidden = super::read_hidden(root)?;
    let where_ = format!("the fleet repository at {}", root.display());
    let kind = one_kind(
        product,
        kind,
        entries.iter().filter(|e| e.product == product).map(|e| e.kind.clone()).collect(),
        &where_,
    )?;
    let of_product: Vec<&Entry> = entries.iter().filter(|e| e.product == product && e.kind == kind).collect();
    let removed: Vec<Entry> = match version {
        None => of_product.iter().map(|e| (*e).clone()).collect(),
        Some(version) => {
            let matching: Vec<Entry> = of_product.iter().filter(|e| e.version == version).map(|e| (*e).clone()).collect();
            if matching.is_empty() {
                let mut held: Vec<&str> = of_product.iter().map(|e| e.version.as_str()).collect();
                held.sort_unstable();
                held.dedup();
                return Err(format!(
                    "{where_} holds no {product} ({kind}) at version {version:?}; it holds {}. Nothing \
                     was changed.",
                    held.join(", ")
                ));
            }
            matching
        }
    };
    let remaining: Vec<Entry> = entries.iter().filter(|e| !removed.contains(e)).cloned().collect();
    let none_left = !remaining.iter().any(|e| e.product == product && e.kind == kind);

    // **A card's dependency is refused** — only an `application` is carried.
    if kind == "application" {
        let blocking: Vec<Carrier> = carriers(root, register, product)?
            .into_iter()
            .filter(|c| match (&c.pinned, version) {
                (_, None) => true,
                (Some(pin), Some(v)) => pin == v,
                (None, Some(_)) => none_left,
            })
            .collect();
        if !blocking.is_empty() {
            let what = version.map_or_else(|| product.to_owned(), |v| format!("{product} {v}"));
            let mut refusal = format!(
                "REFUSED: {what} is what {} machine card(s) carry. The card builder resolves every \
                 carried application from this repository, so deleting it would make that machine's \
                 next card build refuse (ADR-050 §5). Nothing was changed.\n",
                blocking.len()
            );
            for c in &blocking {
                refusal.push_str(&format!(
                    "  {}\n    {}\n    unpin: {}\n",
                    c.machine, c.source, c.unpin
                ));
            }
            refusal.push_str("Hiding it instead is always allowed: `cargo rivet fleet hide` keeps it here.\n");
            return Err(refusal);
        }
    }

    // **An apk's signer pin goes with its last version**, so it is read while
    // the versions are still here to read it from.
    let dropped_signer = if kind == "apk" && none_left {
        Some(apk_key().and_then(|key| super::publish_apk::pinned_signer(root, product, key)))
    } else {
        None
    };

    let was_hidden = hidden.iter().any(|h| h.product == product && h.kind == kind);
    if none_left {
        hidden.retain(|h| !(h.product == product && h.kind == kind));
    }
    let mut message = match version {
        None => format!("Retire {product} ({kind}) from the fleet repository"),
        Some(v) => format!("Retire {product} {v} ({kind}) from the fleet repository"),
    };
    for one in &removed {
        message.push_str(&format!("\nRemove {} {} ({})", one.product, one.version, one.digest));
    }
    super::commit_manifest(root, &remaining, &hidden, &message)?;
    let failures = super::remove_objects(root, &remaining, &removed);

    let mut report = format!(
        "RETIRED  {product} ({kind}){} — {} entr{} removed from {}, and the manifest committed alone\n",
        version.map(|v| format!(" {v}")).unwrap_or_default(),
        removed.len(),
        if removed.len() == 1 { "y" } else { "ies" },
        root.display(),
    );
    for one in &removed {
        report.push_str(&format!("  removed   {}\n", describe(one)));
    }
    for failure in &failures {
        report.push_str(&format!("  {:<9} {failure}; nothing names it now, so it is only a stray file\n", "note"));
    }
    if none_left && was_hidden {
        report.push_str(&format!("  unhidden  nothing of {product} ({kind}) remains, so the hidden set no longer names it\n"));
    }
    match dropped_signer {
        Some(Ok(Some((newest, signer)))) => report.push_str(&format!(
            "  signer    {signer} (pinned by {product} {newest}) is dropped with its versions; a later \
             publish of {product} pins afresh and says so (ADR-061 §4)\n"
        )),
        Some(Ok(None)) => {}
        Some(Err(why)) => report.push_str(&format!(
            "  signer    could not be read, so the key this drops is not named: {why}\n"
        )),
        None if kind == "apk" => report.push_str(&format!(
            "  signer    {product}'s signing-key pin stays with the versions still held\n"
        )),
        None => {}
    }

    let held_by_fleets = rivet_core::package::kind::Kind::parse(&kind).is_ok_and(|k| objects::holds_kind(&k));
    if held_by_fleets && !was_hidden {
        report.push_str(&format!(
            "\nWhat the fleets lose: a fleet server's store removes {} object(s) at its next prune, once \
             this text reaches it — `cargo rivet fleet push <server> confirm`, then `cargo rivet fleet \
             sync <server> --prune confirm` (or `--sync confirm` here, which pushes and shows the list).\n",
            removed.len()
        ));
        if kind == "apk" {
            report.push_str(&format!(
                "A phone holding {product} keeps it and its data, and gets no further update from the fleet.\n"
            ));
        }
    } else if held_by_fleets {
        report.push_str("\nWhat the fleets lose: nothing more — it was hidden, so no fleet was offered it.\n");
    } else {
        report.push_str(&format!("\nWhat the fleets lose: nothing — no fleet server holds a {kind}.\n"));
    }
    report.push_str(
        "Nothing installed changes: a machine running it keeps running it, and `cargo rivet fleet drift` \
         names it as no longer in this repository.\n",
    );
    Ok(report)
}

// ---------------------------------------------------------------------------
// hide and unhide
// ---------------------------------------------------------------------------

/// **Withhold `product` from every fleet**, keeping it here. Always allowed.
///
/// # Errors
///
/// `root` is not a fleet repository, the product or kind is not held or the
/// kind is ambiguous, or the manifest cannot be committed.
pub(super) fn hide(root: &Path, product: &str, kind: Option<&str>) -> Result<String, String> {
    super::require_repository(root)?;
    let entries = super::read_manifest(root)?;
    let mut hidden = super::read_hidden(root)?;
    let where_ = format!("the fleet repository at {}", root.display());
    let kind = one_kind(
        product,
        kind,
        entries.iter().filter(|e| e.product == product).map(|e| e.kind.clone()).collect(),
        &where_,
    )?;
    let one = Hidden { product: product.to_owned(), kind: kind.clone() };
    if hidden.contains(&one) {
        return Ok(format!("{product} ({kind}) is already hidden. Nothing was changed.\n"));
    }
    hidden.push(one);
    super::commit_manifest(root, &entries, &hidden, &format!("Hide {product} ({kind}) from every fleet"))?;
    let objects: Vec<&Entry> = entries.iter().filter(|e| e.product == product && e.kind == kind).collect();
    let mut report = format!(
        "HIDDEN  {product} ({kind}) — withheld from every fleet, and still in {} (the manifest committed \
         alone)\n",
        root.display()
    );
    for entry in &objects {
        report.push_str(&format!("  {:<9} {}\n", "kept", describe(entry)));
    }
    report.push_str(
        "A card build that carries it still finds it here. `fleet sync` stops sending it, and a fleet \
         server's store drops it at its next prune once this text reaches it — `cargo rivet fleet push \
         <server> confirm`, then `cargo rivet fleet sync <server> --prune confirm`. The deploy menus and \
         `fleet list` leave it out; `cargo rivet fleet unhide` returns it.\n",
    );
    Ok(report)
}

/// **Return `product` to the fleets** at their next sync.
///
/// # Errors
///
/// `root` is not a fleet repository, the hidden set does not name the product
/// (or names it under several kinds and `kind` is not given), or the manifest
/// cannot be committed.
pub(super) fn unhide(root: &Path, product: &str, kind: Option<&str>) -> Result<String, String> {
    super::require_repository(root)?;
    let entries = super::read_manifest(root)?;
    let mut hidden = super::read_hidden(root)?;
    let candidates: Vec<String> = hidden.iter().filter(|h| h.product == product).map(|h| h.kind.clone()).collect();
    if candidates.is_empty() {
        return Ok(format!("{product} is not hidden. Nothing was changed.\n"));
    }
    let kind = one_kind(product, kind, candidates, &format!("the hidden set in {}", root.display()))?;
    hidden.retain(|h| !(h.product == product && h.kind == kind));
    super::commit_manifest(root, &entries, &hidden, &format!("Unhide {product} ({kind}): the fleets are offered it again"))?;
    Ok(format!(
        "UNHIDDEN  {product} ({kind}) — offered to the fleets again (the manifest committed alone). The \
         next `cargo rivet fleet push <server> confirm` and `cargo rivet fleet sync <server> confirm` send \
         it.\n"
    ))
}

// ---------------------------------------------------------------------------
// reaching the fleet's server
// ---------------------------------------------------------------------------

/// **The fleet's server, as a retire, a hide or the interview reaches it**:
/// send what it lacks and push the text, read its store, and — only when asked
/// as its own step — prune it.
pub(super) trait FleetServer {
    /// The name the report calls it by.
    fn label(&self) -> &str;

    /// `fleet publish --sync`'s composition: send every object this
    /// repository names that the server lacks, then push `main`. Without
    /// `confirm`, what that would do.
    ///
    /// # Errors
    ///
    /// The send or the push did not complete; the report is the error.
    fn send_and_push(&self, root: &Path, confirm: bool) -> Result<String, String>;

    /// The server's listing.
    ///
    /// # Errors
    ///
    /// The server could not be asked or answered unreadably.
    fn listing(&self) -> Result<Listing, String>;

    /// `POST /repository-objects/prune`.
    ///
    /// # Errors
    ///
    /// The server refused or could not be asked.
    fn prune(&self) -> Result<String, String>;
}

/// The real server, over the pinned control plane.
struct Connected {
    label: String,
    tls: Arc<rivet_remote::tls::Tls>,
}

impl FleetServer for Connected {
    fn label(&self) -> &str {
        &self.label
    }

    fn send_and_push(&self, root: &Path, confirm: bool) -> Result<String, String> {
        super::sync_then_push(root, Arc::clone(&self.tls), &self.label, confirm)
    }

    fn listing(&self) -> Result<Listing, String> {
        super::sync::listing(self.tls.as_ref(), &self.label)
    }

    fn prune(&self) -> Result<String, String> {
        super::sync::ObjectStore::prune(self.tls.as_ref())
    }
}

/// **Find the fleet's server in the register and open the control plane to
/// it** — `fleet publish --sync`'s resolution, with `given`'s flags winning.
///
/// # Errors
///
/// [`super::open_fleet_server`]'s.
pub(super) fn connect(given: &rivet_remote::endpoint::Endpoint, verb: &str) -> Result<Box<dyn FleetServer>, String> {
    let (label, tls) = super::open_fleet_server(given, verb)?;
    Ok(Box::new(Connected { label, tls: Arc::new(tls) }))
}

/// Every object this repository's manifest names for a fleet, by the server's
/// own rule — what the server will name once this text is pushed.
fn named_here(root: &Path) -> Result<std::collections::BTreeSet<rivet_core::artifact::Digest>, String> {
    let entries = super::read_manifest(root)?;
    let hidden = super::read_hidden(root)?;
    let hidden: Vec<(&str, &str)> = hidden.iter().map(|h| (h.product.as_str(), h.kind.as_str())).collect();
    objects::named(entries.iter().map(|e| (e.digest.as_str(), e.product.as_str(), e.kind.as_str())), &hidden)
}

/// **`--sync [confirm]` after a local change**: send and push, then say what
/// the server's prune would delete — and stop. The prune itself is always its
/// own command.
///
/// Without `confirm` nothing is sent or pushed, and the list is what the
/// server holds that this manifest will not name once pushed.
///
/// # Errors
///
/// [`FleetServer::send_and_push`]'s or [`FleetServer::listing`]'s, carrying
/// everything reported before them.
pub(super) fn then_sync(root: &Path, server: &dyn FleetServer, confirm: bool) -> Result<String, String> {
    let label = server.label().to_owned();
    let mut report = server.send_and_push(root, confirm)?;
    let listing = server.listing().map_err(|why| format!("{report}\n{why}"))?;
    report.push('\n');
    if confirm {
        report.push_str(&prune_plan(&listing, &label));
    } else {
        let named = named_here(root).map_err(|why| format!("{report}{why}"))?;
        let would: Vec<_> = listing.held.iter().filter(|h| !named.contains(&h.digest)).collect();
        if would.is_empty() {
            report.push_str(&format!("Once pushed, {label}'s prune would remove nothing.\n"));
        } else {
            report.push_str(&format!("Once pushed, {label}'s prune would remove {} object(s):\n", would.len()));
            for one in would {
                report.push_str(&format!("  would prune {} {}\n", one.digest, one.name));
            }
        }
    }
    report.push_str(&format!(
        "NOTHING WAS PRUNED. The deletion from a store is its own command: `cargo rivet fleet sync {label} \
         --prune confirm`.\n"
    ));
    Ok(report)
}

/// What `listing`'s own prune rule would delete, as a person reads it.
pub(super) fn prune_plan(listing: &Listing, label: &str) -> String {
    match listing.prune() {
        Ok(remove) if remove.is_empty() => {
            format!("{label}'s prune would remove nothing: every object it holds is one its manifest names.\n")
        }
        Ok(remove) => {
            let mut out = format!("{label}'s prune would remove {} object(s):\n", remove.len());
            for one in &remove {
                out.push_str(&format!("  would prune {} {}\n", one.digest, one.name));
            }
            out
        }
        Err(why) => format!("{label} would not prune yet: {why}\n"),
    }
}

/// Whether `listing` was read at this repository's own `main` — a prune is
/// decided by the server's text, so it is offered only when that text is
/// this one.
pub(super) fn read_at_local_main(root: &Path, listing: &Listing) -> bool {
    let local = super::git(root, &["rev-parse", "--verify", "--quiet", "refs/heads/main^{commit}"])
        .map(|s| s.trim().to_owned())
        .unwrap_or_default();
    matches!(&listing.manifest, Manifest::Read { commit, .. } if !local.is_empty() && *commit == local)
}

#[cfg(test)]
#[path = "fleet_retire_tests.rs"]
mod tests;
