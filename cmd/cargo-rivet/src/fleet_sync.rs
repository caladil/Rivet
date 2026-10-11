//! `cargo rivet fleet sync <host>` — **the fleet repository's objects, sent to a
//! fleet server's repository store over `ADR-035`'s control plane.** `.rulings/`
//! R-182, R-184 D1 and D2; `ADR-052` §9 step 4; `ADR-063` §6 for cards and
//! updates.
//!
//! A server's card carries the repository's text and none of its objects
//! (R-182); `cargo rivet fleet push` puts the text in its `fleet` repository;
//! this sends what that text names. The store is `rivet-fleetd`'s own, kept by
//! the machine's control plane (`cmd/rivet-mcpd/src/repository.rs` argues why
//! the control plane and not `rivet-fleetd` receives into it).
//!
//! # What it does, in order
//!
//! 1. Read THIS repository's manifest, and select every object of a kind the
//!    store holds (`rivet_core::repository_objects::holds_kind` — applications,
//!    libraries, apks, system updates and cards) that is not hidden
//!    (`ADR-052` §7).
//! 2. Ask the server what it holds and what ITS manifest names —
//!    `GET /repository-objects/`.
//! 3. Send only the difference, each object streamed from `objects/<digest>`,
//!    each verified at the server's door before it is named.
//! 4. Ask again, and report what the server now holds against what the manifest
//!    names — the completion signal.
//! 5. With `--prune` and `confirm`, and only after 4 found the store complete,
//!    ask the server to prune: it removes what its own manifest stops naming,
//!    and refuses while anything named is missing.
//!
//! **Without `confirm` nothing is sent and nothing is pruned**: steps 1 and 2
//! run and the plan is printed.
//!
//! # Resumable by construction
//!
//! Step 3 sends what step 2 says is not held, so a sync that died after two of
//! five objects sends three on its rerun and a complete one sends none. There
//! is no state file: the server's listing is the state.
//!
//! # What it refuses, conservatively
//!
//! * **An object this workstation's manifest names and its `objects/` lacks** is
//!   reported as `missing`, the rest are still sent, and the run exits non-zero.
//!   A sync that quietly skipped it would report a store as complete that is not.
//! * **`--prune` when the server's manifest is not this workstation's `main`.**
//!   The server prunes by the text it holds, so a prune decided while that text
//!   is behind would remove what the operator's newer manifest still names:
//!   `cargo rivet fleet push` first.
//! * **`--prune` after anything was refused, failed or missing.** Pruning never
//!   runs against a sync that did not complete, and the server refuses it too.

use std::collections::BTreeSet;
use std::path::Path;

use rivet_core::artifact::Digest;
use rivet_core::package::kind::Kind;
use rivet_core::repository_objects::{self as objects, Listing, Manifest};
use rivet_remote::endpoint::Endpoint;

/// **A fleet server's repository store, as this verb reaches it.**
///
/// `rivet_remote::tls::Tls` on a real run; an in-memory store in the tests.
pub(super) trait ObjectStore {
    /// The listing's text — `GET /repository-objects/`.
    ///
    /// # Errors
    ///
    /// The machine could not be asked, or answered other than `200`.
    fn list(&self) -> Result<String, String>;

    /// Send one object — `PUT /repository-objects/<name>`.
    ///
    /// # Errors
    ///
    /// The transfer failed, or the machine refused the object (a message
    /// carrying `REFUSED THE ARTIFACT`).
    fn put(&self, local: &Path, name: &str) -> Result<String, String>;

    /// Ask for a prune — `POST /repository-objects/prune`.
    ///
    /// # Errors
    ///
    /// The machine could not be asked, or refused.
    fn prune(&self) -> Result<String, String>;
}

impl ObjectStore for rivet_remote::tls::Tls {
    fn list(&self) -> Result<String, String> {
        http(self, "GET", objects::PREFIX)
    }

    fn put(&self, local: &Path, name: &str) -> Result<String, String> {
        self.put_repository_object(local, name)
    }

    fn prune(&self) -> Result<String, String> {
        http(self, "POST", objects::PRUNE)
    }
}

/// One bodiless request, and its `200` body or its refusal.
fn http(tls: &rivet_remote::tls::Tls, method: &str, path: &str) -> Result<String, String> {
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: rivet\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let answer = tls.exchange(&head, &[])?;
    let text = String::from_utf8_lossy(&answer);
    let status = text.lines().next().and_then(|l| l.split(' ').nth(1)).unwrap_or_default().to_owned();
    let body = text.split_once("\r\n\r\n").map_or("", |(_, b)| b).to_owned();
    if status == "200" {
        Ok(body)
    } else if status == "404" && body.contains("keeps no repository objects") {
        Err(format!("the machine answered {status}: {body}"))
    } else if status == "404" || status == "405" {
        Err(format!(
            "the machine answered {status} for {method} {path}, which is what a rivet-mcpd built \
             before a fleet server's repository store existed says. Update the machine.\n{body}"
        ))
    } else {
        Err(format!("the machine answered {status} for {method} {path}:\n{body}"))
    }
}

/// `cargo rivet fleet sync [<host>] [--host …] [--identity …] [--pin …]
/// [--at DIR] [--prune] [confirm]` — resolve the machine as `fleet push` does,
/// open the pinned transport, and [`sync`].
///
/// # Errors
///
/// The resolution's, the transport's, or [`sync`]'s.
pub(super) fn run(
    root: &Path,
    device: Option<&str>,
    given: &Endpoint,
    prune: bool,
    confirm: bool,
) -> Result<String, String> {
    given.paired()?;
    let endpoint = super::push::resolve_endpoint(device, given, &crate::device::default_register_dir)
        .map_err(|e| e.replace("`fleet push`", "`fleet sync`"))?;
    let transport = endpoint.open()?.ok_or_else(|| {
        "no operator identity to present: `fleet sync` speaks ADR-035's control plane and has no \
         second transport. `cargo rivet identity new` mints one; --identity DIR names it."
            .to_owned()
    })?;
    let label = device.map_or_else(|| endpoint.host.clone().unwrap_or_default(), str::to_owned);
    sync(root, &transport, &label, prune, confirm)
}

/// One object this workstation's manifest names.
struct Wanted {
    digest: Digest,
    name: String,
}

/// [`wanted`]'s answer: what is sent, what the image provides, and what is
/// hidden.
type Selected = (Vec<Wanted>, Vec<Wanted>, Vec<Wanted>);

/// **The sync** — see the module documentation for the order.
///
/// # Errors
///
/// `root` is not a fleet repository or its manifest does not read; the server
/// could not be asked or its listing does not read; or the run did not
/// complete — anything missing here, refused or failed there, or a prune that
/// was asked for and not run. The report is the error's text in that case, so
/// nothing that happened is lost.
pub(super) fn sync(
    root: &Path,
    server: &dyn ObjectStore,
    label: &str,
    prune: bool,
    confirm: bool,
) -> Result<String, String> {
    super::require_repository(root)?;
    let (wanted, image_services, withheld) = wanted(root)?;
    let local_main = super::git(root, &["rev-parse", "--verify", "--quiet", "refs/heads/main^{commit}"])
        .map(|s| s.trim().to_owned())
        .unwrap_or_default();

    let before = listing(server, label)?;
    let server_main = match &before.manifest {
        Manifest::Read { commit, .. } => commit.clone(),
        Manifest::Unavailable(why) => format!("(unavailable: {why})"),
    };

    let mut report = format!(
        "fleet sync  {label}\n\
         \x20 from          {root} (objects/)\n\
         \x20 to            {label}'s repository store, over the control plane (R-184 D1)\n\
         \x20 local main    {local}\n\
         \x20 server main   {server_main}\n\
         \x20 named here    {count} object(s): applications, libraries, apks, updates and cards\n\n",
        root = root.display(),
        local = if local_main.is_empty() { "(none)" } else { &local_main },
        count = wanted.len(),
    );

    for one in &image_services {
        report.push_str(&format!(
            "  image     {} {}  (the image provides this service, so it is not sent — ADR-060)\n",
            one.digest, one.name
        ));
    }
    for one in &withheld {
        report.push_str(&format!(
            "  hidden    {} {}  (withheld from every fleet, so it is not sent — ADR-052 §7)\n",
            one.digest, one.name
        ));
    }

    // What the server already holds is counted, not listed: a settled store
    // would otherwise print one line per object and bury the few that matter.
    let mut to_send = Vec::new();
    let mut missing_here = Vec::new();
    let mut already_held = 0usize;
    for one in &wanted {
        if before.holds(&one.digest) {
            already_held += 1;
        } else if !root.join(super::OBJECTS_DIR).join(one.digest.as_str()).is_file() {
            report.push_str(&format!(
                "  missing   {} {}  (not in {} — nothing to send)\n",
                one.digest,
                one.name,
                root.join(super::OBJECTS_DIR).display()
            ));
            missing_here.push(one);
        } else {
            to_send.push(one);
        }
    }
    if already_held > 0 {
        report.push_str(&row("already", &format!("{already_held} object(s) the server holds, not sent")));
    }

    let text_behind = match &before.manifest {
        Manifest::Read { commit, .. } => local_main.is_empty() || *commit != local_main,
        Manifest::Unavailable(_) => true,
    };
    if text_behind {
        report.push('\n');
        report.push_str(&row(
            "note",
            "the server's fleet main is not this workstation's main. This sends what THIS \
             manifest names; the server prunes by ITS manifest, so `cargo rivet fleet push <host> \
             confirm` first, and --prune is refused until they agree.",
        ));
    }

    if !confirm {
        for one in &to_send {
            report.push_str(&format!("  would send {} {}\n", one.digest, one.name));
        }
        if prune {
            report.push_str(&prune_plan(&before));
        }
        report.push_str(&format!(
            "\n{} to send, {} already held, {} missing here. Nothing was sent. Add `confirm` to \
             send{}.\n",
            to_send.len(),
            wanted.len() - to_send.len() - missing_here.len(),
            missing_here.len(),
            if prune { " and prune" } else { "" },
        ));
        return if missing_here.is_empty() { Ok(report) } else { Err(report) };
    }

    let mut refused = 0usize;
    let mut failed = 0usize;
    for one in &to_send {
        let local = root.join(super::OBJECTS_DIR).join(one.digest.as_str());
        match server.put(&local, &one.name) {
            Ok(_) => report.push_str(&row("sent", &format!("{} {}", one.digest, one.name))),
            Err(why) if why.contains("REFUSED THE ARTIFACT") => {
                refused += 1;
                report.push_str(&format!("  refused   {} {}\n{}", one.digest, one.name, indented(&why)));
            }
            Err(why) => {
                failed += 1;
                report.push_str(&format!("  failed    {} {}\n{}", one.digest, one.name, indented(&why)));
            }
        }
    }

    // **Read back**, rather than trusting the answers above: the question is
    // what the machine now holds.
    let after = listing(server, label)?;
    let held_now = wanted.iter().filter(|w| after.holds(&w.digest)).count();
    report.push('\n');
    report.push_str(&row(
        "held",
        &format!(
            "{held_now} of {} object(s) this manifest names, read back from {label}",
            wanted.len()
        ),
    ));
    let server_missing = after.missing();
    if !server_missing.is_empty() {
        report.push_str(&format!(
            "  server    its own manifest names {} object(s) it does not hold\n",
            server_missing.len()
        ));
    }

    let complete = missing_here.is_empty() && refused == 0 && failed == 0 && held_now == wanted.len();
    let mut ok = complete;
    if prune {
        if !complete {
            report.push_str("\nPRUNE NOT RUN: this sync did not complete, and pruning runs only against a complete store.\n");
            ok = false;
        } else if text_behind {
            report.push_str(
                "\nPRUNE NOT RUN: the server's fleet main is not this workstation's main, and the \
                 server prunes by its own manifest. `cargo rivet fleet push <host> confirm` first.\n",
            );
            ok = false;
        } else {
            match server.prune() {
                Ok(body) => {
                    report.push_str("\nPRUNED, by the server against its own manifest:\n");
                    let mut removed = 0usize;
                    for line in body.lines() {
                        if let Some(rest) = line.strip_prefix("removed ") {
                            removed += 1;
                            report.push_str(&format!("  pruned    {rest}\n"));
                        }
                    }
                    if removed == 0 {
                        report.push_str("  nothing: every object held is one the manifest names\n");
                    }
                }
                Err(why) => {
                    report.push_str(&format!("\nPRUNE REFUSED by the server:\n{}", indented(&why)));
                    ok = false;
                }
            }
        }
    }

    report.push_str(&format!(
        "\n{} sent, {} already held, {} missing here, {refused} refused, {failed} failed. {}\n",
        to_send.len() - refused - failed,
        wanted.len() - to_send.len() - missing_here.len(),
        missing_here.len(),
        if complete {
            "SYNCED: the store holds every object this manifest names."
        } else {
            "NOT COMPLETE: a rerun sends only what is still not held."
        }
    ));
    if ok { Ok(report) } else { Err(report) }
}

/// Every object of a kind the store holds that the manifest names, once each —
/// and, apart, every application it names under an image service's name.
///
/// **Those are not sent** (`ADR-060` §4): the machine's own door refuses an
/// application whose name the image provides, so sending one would only be
/// refused there, and a repository published before the proxy moved into the
/// image still holds proxy payloads until retention ages them out. They are
/// named in the report so that skipping them is not silent.
///
/// **A hidden product's objects are not wanted either** (`ADR-052` §7): the
/// product stays in this repository and is withheld from every fleet. They are
/// the third list, named in the report for the same reason.
fn wanted(root: &Path) -> Result<Selected, String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let mut image_services = Vec::new();
    let mut withheld = Vec::new();
    let hidden = super::read_hidden(root)?;
    for entry in super::read_manifest(root)? {
        let Ok(kind) = Kind::parse(&entry.kind) else {
            continue;
        };
        if !objects::holds_kind(&kind) {
            continue;
        }
        if super::is_hidden(&hidden, &entry) {
            if let Some(digest) = Digest::new(&entry.digest) {
                withheld.push(Wanted { name: entry.file_name()?, digest });
            }
            continue;
        }
        if kind == Kind::application()
            && rivet_core::service::IMAGE_SERVICE_NAMES.contains(&entry.product.as_str())
        {
            if let Some(digest) = Digest::new(&entry.digest) {
                image_services.push(Wanted { name: entry.file_name()?, digest });
            }
            continue;
        }
        let digest = Digest::new(&entry.digest).ok_or_else(|| {
            format!(
                "the manifest names {:?}, which is not a BLAKE3 digest. Nothing was sent.",
                entry.digest
            )
        })?;
        if seen.insert(digest.clone()) {
            out.push(Wanted { name: entry.file_name()?, digest });
        }
    }
    Ok((out, image_services, withheld))
}

/// The server's listing, read.
///
/// # Errors
///
/// The server could not be asked, or its listing does not read.
pub(super) fn listing(server: &dyn ObjectStore, label: &str) -> Result<Listing, String> {
    let text = server.list().map_err(|why| {
        format!("could not ask {label} what its repository store holds, so nothing was sent.\n{why}")
    })?;
    Listing::parse(&text).map_err(|why| {
        format!("{label}'s listing does not read, so nothing was sent: {why}\n{text}")
    })
}

/// What a prune would remove, as the server's own listing says — for a plan.
fn prune_plan(listing: &Listing) -> String {
    match listing.prune() {
        Ok(remove) if remove.is_empty() => {
            "  would prune nothing: every object held is one the server's manifest names\n".to_owned()
        }
        Ok(remove) => remove
            .iter()
            .map(|h| format!("  would prune {} {}\n", h.digest, h.name))
            .collect(),
        Err(why) => format!("  would not prune yet: {why}\n"),
    }
}

/// One report line: a word padded to the report's column, then the rest.
fn row(word: &str, rest: &str) -> String {
    format!("  {word:<9} {rest}\n")
}

fn indented(text: &str) -> String {
    text.lines().map(|l| format!("            {l}\n")).collect()
}

#[cfg(test)]
#[path = "fleet_sync_tests.rs"]
mod tests;
