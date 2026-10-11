//! **Wave C3 — everything beyond `rivet.toml` that goes onto a card's boot
//! partitions at write time**: the `ADR-049` staging bundle, this card's own
//! identity, one `app-<product>.rvt` per `[carry.<product>]` pin, and — for a
//! fleet server's document — the fleet repository's TEXT, which R-178 P1 as
//! revised by R-182 rules it carries. Its application objects are not carried:
//! they follow over the control plane after first boot.
//!
//! # Byte-compatible with a fresh build, on purpose
//!
//! `cmd/rivet-image/src/boot.rs::contents` already places every one of these
//! file KINDS onto a freshly-assembled boot partition, under names
//! `rivet_core::staging`/`rivet_core::carried` own (`STAGED_FILE`,
//! `IDENTITY_SEED_FILE`, `IDENTITY_CERTIFICATE_FILE`, `IDENTITY_PEERS_FILE`,
//! `card_file`). This module gathers the SAME bytes under the SAME names for
//! `write_image.rs::inject` to place onto an EXISTING image's partitions —
//! `write_image.rs`'s own module doc explains why that tool, and not a second
//! "edit an existing image" mode of the bin-only `cmd/rivet-image`, is where
//! this belongs.
//!
//! # What this module does NOT do
//!
//! It performs no signature check against the image's own `/system/trust` —
//! it cannot; `/system` is ext4 and only the FAT boot partitions are
//! reachable from here (`write_image.rs::trust_note`'s own finding, restated
//! for a carried payload rather than a document's signing key). What it DOES
//! check — `.rulings/` R-178 P5 — is a carried payload's signature against
//! THIS WORKSTATION's own established trust anchor, `<rivet root>/keys/rivet-app.pub`
//! (`cargo rivet setup`'s own output — `crate::provision_image::materialise`'s
//! own convention for the system key, matched here for the application one),
//! and refuses before anything is written if that fails.

use crate::credentials::{BOARD_CREDENTIAL, Survey};
use crate::write_image::ProvisionArgs;
use std::path::{Path, PathBuf};

/// This workstation's own established trust anchor: `<rivet root>/keys/rivet-app.pub`
/// — `cargo rivet setup`'s own output, `credentials::app_seed`'s sibling for
/// the PUBLIC half. **Not `write_image::trust_note`'s repo-root-relative
/// `keys/rivet-app.pub`** — that path names the checked-out tree's own
/// COMMITTED key (what a fresh card build trusts by default,
/// `tools/card/build.sh`'s `RIVET_APP_KEY`), a different question from *what
/// does THIS OPERATOR trust*. `crate::provision_image::materialise` already
/// answers the identical question for the system key the same way
/// (`rivet_root.join("keys").join("rivet-system.pub")`), so this converges
/// with that precedent rather than `trust_note`'s.
fn workstation_app_key_path(rivet_root: &Path) -> PathBuf {
    rivet_root.join("keys").join("rivet-app.pub")
}

/// Where a carried fleet repository lands inside `rivet-fleetd`'s own data
/// directory, once the supervisor has delivered the staging bundle —
/// `ADR-049`'s general mechanism, addressed at this one prefix so it can
/// never collide with `register/register.toml` (`SPEC-05` §4's own staged
/// entry, addressed to the same application).
const REPOSITORY_PREFIX: &str = "fleet-repository";

/// Everything [`gather`] collected, ready for `write_image::inject`.
#[derive(Debug)]
pub(crate) struct Extras {
    /// `(name on the boot partition, bytes)` — never `rivet.toml`, which
    /// `write_image::run` still handles itself; every name here is a flat
    /// file name, `ADR-018` §4's boot partition holding no directories.
    pub files: Vec<(String, Vec<u8>)>,
    /// Printed before injection: what was carried, and the trust-store note
    /// (R-178 P5) — `write_image::trust_note`'s own posture, a fact stated
    /// once rather than a refusal for a limit this tool cannot lift.
    pub report: String,
}

/// Gather everything beyond `rivet.toml` this run places onto the card —
/// see the module doc for the four kinds and where each one's bytes and name
/// come from.
///
/// # Errors
///
/// [`crate::provision::require_staging`]'s own refusal; a `[carry.<product>]`
/// pin (or an unpinned, ambiguous role requirement) the fleet repository
/// cannot resolve (`crate::fleet::resolve_carried_object`); a carried
/// payload whose signature this workstation's own `keys/rivet-app.pub` does
/// not accept, or that key itself being unreadable (R-178 P5 — never a
/// silent pass); a staging bundle, a fleet server's repository text included,
/// too large for the `ADR-049` bundle's own cap, named with the measured bytes.
pub(crate) fn gather(
    document_path: &Path,
    doc: &rivet_config::Document,
    survey: &Survey,
    args: &ProvisionArgs,
    rivet_root: &Path,
) -> Result<Extras, String> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut report = String::new();

    // --- identity: device.seed, device.crt, peers --------------------------
    //
    // **Always present by the time this runs.** `credentials::Survey::take`
    // lists `BOARD_CREDENTIAL` as `Need::Required` unconditionally, and
    // `write_image::run` already refused on `survey.refusal()` before ANY of
    // this module is reached — so `named` returning `None` here would be a
    // survey this run never actually consulted, not a real absence.
    if let Some(board) = survey.named(BOARD_CREDENTIAL) {
        let seed = std::fs::read(board.path.join(rivetos_tls::SEED_FILE))
            .map_err(|e| format!("cannot read {}: {e}", board.path.join(rivetos_tls::SEED_FILE).display()))?;
        let certificate = std::fs::read(board.path.join(rivetos_tls::CERTIFICATE_FILE)).map_err(|e| {
            format!("cannot read {}: {e}", board.path.join(rivetos_tls::CERTIFICATE_FILE).display())
        })?;
        let peers = std::fs::read(board.path.join(crate::credentials::PEERS_FILE)).map_err(|e| {
            format!("cannot read {}: {e}", board.path.join(crate::credentials::PEERS_FILE).display())
        })?;
        files.push((rivet_image_names::IDENTITY_SEED_FILE.to_owned(), seed));
        files.push((rivet_image_names::IDENTITY_CERTIFICATE_FILE.to_owned(), certificate));
        files.push((rivet_image_names::IDENTITY_PEERS_FILE.to_owned(), peers));
        report.push_str(&format!(
            "--- identity: device.seed, device.crt, peers, from {}\n",
            board.path.display()
        ));
    }

    // --- the staging bundle, widened by P1 for a fleet server --------------
    let staging_path =
        crate::provision::require_staging(document_path, doc, args.staging_path.as_deref())?;
    let mut staged_items: Vec<rivet_core::staging::Staged> = Vec::new();
    if let Some(path) = &staging_path {
        let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|e| format!("{} is not valid UTF-8: {e}", path.display()))?;
        staged_items = rivet_core::staging::parse(text)
            .map_err(|e| format!("{} does not parse as a staging bundle: {e:?}", path.display()))?;
    }

    // --- carried applications: [carry.<product>] / role requirement --------
    let fleet_repo_root =
        args.fleet_repo_root.clone().or_else(|| crate::provision::fleet_repository_root(rivet_root));
    let mut carried_any = false;
    for service in doc.carried_services() {
        let product = service.name().as_str();
        let Some(root) = &fleet_repo_root else {
            return Err(format!(
                "{product:?} must be carried (ADR-050 §5) and this run has no fleet repository \
                 to resolve it from. Set RIVET_FLEET_REPOSITORY, or create one with `cargo rivet \
                 fleet init`."
            ));
        };
        let pinned = doc.carried_version(product);
        let (entry, object_path) = crate::fleet::resolve_carried_object(root, product, pinned)?;
        let bytes = std::fs::read(&object_path)
            .map_err(|e| format!("cannot read {}: {e}", object_path.display()))?;
        verify_carried(&bytes, &object_path.display().to_string(), rivet_root)?;
        carried_any = true;
        files.push((rivet_core::carried::card_file(product), bytes));
        report.push_str(&format!(
            "--- carried: {} {} -> {} ({} bytes)\n",
            entry.product,
            entry.version,
            rivet_core::carried::card_file(product),
            entry.size,
        ));
    }
    if carried_any {
        report.push_str(&carried_trust_note(&workstation_app_key_path(rivet_root)));
    }

    // --- R-178 P1, revised by R-182: a fleet server's card carries the
    //     repository's TEXT. Its application objects follow over the control
    //     plane after first boot, so none of them rides in the bundle.
    if crate::provision::role_of(doc) == Some(rivetos_register::FleetRole::Server)
        && let Some(root) = &fleet_repo_root
    {
        let added = repository_carry(root)?;
        let added_bytes: u64 = added.iter().map(|s| s.bytes.len() as u64).sum();
        report.push_str(&format!(
            "--- carrying the fleet repository's text (R-182): {} file(s), {added_bytes} bytes, \
             staged under {REPOSITORY_PREFIX}/ in rivet-fleetd's own data directory. Its \
             application objects are NOT on this card; they follow over the control plane \
             after first boot.\n",
            added.len()
        ));
        staged_items.extend(added);
    }

    if !staged_items.is_empty() {
        let rendered = rivet_core::staging::render(&staged_items);
        if rendered.len() > rivet_core::staging::MAX_BUNDLE_BYTES {
            return Err(format!(
                "the staging bundle this card would carry is {} bytes and the limit is {} \
                 (rivet_core::staging::MAX_BUNDLE_BYTES) — PID 1 reads it whole, before /state is \
                 known to be writable. A fleet server's repository text rides in it too, so a \
                 very large documents/ or register/ tree is one reason; nothing was written.",
                rendered.len(),
                rivet_core::staging::MAX_BUNDLE_BYTES,
            ));
        }
        files.push((rivet_core::staging::BUNDLE_FILE.to_owned(), rendered.into_bytes()));
    }

    Ok(Extras { files, report })
}

/// **R-178 P5's note, stated once for every carried payload.** Under `R-175`
/// the image comes from the fleet repository and the operator builds nothing,
/// so the gap it names is the PUBLISHED image's trust store, not a build's:
/// `/system` is ext4 and cannot be read from here.
fn carried_trust_note(app_key: &Path) -> String {
    format!(
        "--- NOTE: the carried applications were checked against {}. The image's own trust \
         store cannot be read from here; if it was published trusting a different application \
         key, its installer will refuse them.\n",
        app_key.display(),
    )
}

/// The four generated-file names [`gather`] uses, restated from
/// `cmd/rivet-image/src/boot.rs` — that crate is bin-only and exports
/// nothing (`ADR-011`, the same trade `crate::fleet::resolve_carried_object`'s
/// own doc names for `fleet_payloads.rs`). Three constants, not four:
/// `rivet_core::staging::BUNDLE_FILE` and `rivet_core::carried::card_file`
/// are ALREADY the one spelling both crates read, so only the identity
/// trio — which `rivet_core` does not own — needs restating here.
mod rivet_image_names {
    pub(super) const IDENTITY_SEED_FILE: &str = "device.seed";
    pub(super) const IDENTITY_CERTIFICATE_FILE: &str = "device.crt";
    pub(super) const IDENTITY_PEERS_FILE: &str = "peers";
}

/// **R-178 P5.** Verify `bytes` — a carried application payload's whole file
/// — against this workstation's own `keys/rivet-app.pub`, refusing before
/// anything is written. Mirrors `cmd/rivet-image/src/application.rs::verify`,
/// against a one-entry trust set built from a single public-key FILE rather
/// than the image's own `--trusted-key` list — that crate is bin-only
/// (`ADR-011`, restated once more for the same reason `resolve_carried_object`
/// already gives it).
///
/// # Errors
///
/// `keys/rivet-app.pub` cannot be read or parsed (never a silent pass —
/// `AGENTS.md` §7), the header does not verify against it, a member's bytes
/// do not match its digest, or the package is not a valid `application`.
fn verify_carried(bytes: &[u8], label: &str, rivet_root: &Path) -> Result<(), String> {
    let key_path = workstation_app_key_path(rivet_root);
    let key_bytes = std::fs::read(&key_path).map_err(|e| {
        format!(
            "{label} must be carried and its signature cannot be checked: cannot read {}: {e}. \
             `cargo rivet setup` writes the application signing seed this is the public half of; \
             nothing was written.",
            key_path.display(),
        )
    })?;
    let raw = rivet_core::package::key_material::parse(&key_bytes)
        .map_err(|e| format!("{}: {e}", key_path.display()))?;
    let trusted = rivet_core::package::port::PublicKey(raw);

    let file_len = bytes.len() as u64;
    let head = bytes.get(..rivet_core::package::format::HEADER_BYTES).ok_or_else(|| {
        format!(
            "{label} is {file_len} bytes and a package header is {}; there is nothing to verify",
            rivet_core::package::format::HEADER_BYTES,
        )
    })?;

    let mut digest = rivet_crypto::Blake3::new();
    let verifier = rivet_crypto::Ed25519Verifier::new();
    let mut trust = rivet_core::package::port::TrustSet::new();
    // `application,library` — `tools/card/build.sh`'s own entitlement for
    // `rivet-app`, matched so this refuses exactly what a card's own
    // installer would refuse for entitlement, not a narrower or wider set.
    trust.trust(
        &mut digest,
        trusted,
        "rivet-app",
        rivet_core::package::port::MaySign::these(&[
            rivet_core::package::Kind::application(),
            rivet_core::package::Kind::library(),
        ]),
    );

    let package =
        rivet_core::package::read::open(head, file_len, &mut digest, &verifier, &trust).map_err(|e| {
            format!(
                "{label}: refused — {e}\n  This is the check a board's own installer would make \
                 against the application key, run here before anything is written."
            )
        })?;

    for member in package.members() {
        verify_member(bytes, &package, member, &mut digest)
            .map_err(|e| format!("{label}: refused — member {} does not match its digest: {e}", member.name()))?;
    }

    let kind = package.header().kind().to_string();
    if kind != rivet_core::package::Kind::application().as_str() {
        return Err(format!(
            "{label}: its <kind> is {kind:?}, and a carried payload is an application (ADR-050 §4)."
        ));
    }
    rivet_core::package::application::Application::open(&package)
        .map_err(|e| format!("{label}: refused — {e}"))?;
    Ok(())
}

/// One member, streamed through the digest the header committed to — the
/// slice-based reader `cmd/rivet-image/src/application.rs::verify_member`
/// already uses, restated for this crate's own copy (see [`verify_carried`]'s
/// own doc for why there are two).
fn verify_member(
    bytes: &[u8],
    package: &rivet_core::package::read::Package,
    member: &rivet_core::package::read::Member,
    digest: &mut dyn rivet_core::package::port::Digest,
) -> Result<(), rivet_core::package::PackageError> {
    const CHUNK: usize = 1024 * 1024;
    let mut stream = package.member_stream(member.index(), digest)?;
    let start = usize::try_from(member.offset()).unwrap_or(usize::MAX);
    let mut offset = start;
    let end = start.saturating_add(usize::try_from(member.stored_len()).unwrap_or(usize::MAX));
    while offset < end {
        let stop = end.min(offset.saturating_add(CHUNK));
        let Some(slice) = bytes.get(offset..stop) else {
            break;
        };
        if slice.is_empty() {
            break;
        }
        stream.write(slice)?;
        offset = stop;
    }
    stream.finish()?;
    Ok(())
}

/// **R-178 P1, as revised by R-182.** The fleet repository's text half
/// (`manifest.toml`, `documents/`, and the register projection under
/// `register/`), staged to `rivet-fleetd` under [`REPOSITORY_PREFIX`].
///
/// **No object bytes.** Until R-182 this also staged every
/// `application`/`library` object the manifest named under
/// `fleet-repository/objects/`; the staging bundle is hex text capped at
/// `rivet_core::staging::MAX_BUNDLE_BYTES`, and a real server's two
/// applications alone overran it. The objects follow over the control plane
/// after first boot instead, so this function reads none of them.
///
/// # Errors
///
/// A directory under `root` cannot be listed or a file under it cannot be
/// read, or a path this produces is not one [`rivet_core::staging::Staged`]
/// accepts (defensive — a hostname or a digest is never expected to trip
/// this).
fn repository_carry(root: &Path) -> Result<Vec<rivet_core::staging::Staged>, String> {
    const APPLICATION: &str = "rivet-fleetd";
    let mut out = Vec::new();

    // **The entries a server's manifest may name.** `card` and `update`
    // entries are excluded by kind — a fleet server never holds either — and
    // an `application`/`library` entry whose object this workstation's store
    // does not actually hold — a manifest ahead of a pruned or partial
    // `objects/` directory, `ADR-052` §7's own retention rule — is excluded
    // too: nothing can ever deliver bytes this workstation lacks, and naming a
    // bare product/version the server can look up but never obtain is worse
    // than not naming it (`AGENTS.md` §7). The object is only LOOKED FOR here;
    // its bytes are not read (R-182).
    let entries = crate::fleet::read_manifest(root)?;
    let carried: Vec<crate::fleet::Entry> = entries
        .into_iter()
        .filter(|e| {
            (e.kind == "application" || e.kind == "library")
                && root.join(crate::fleet::OBJECTS_DIR).join(&e.digest).is_file()
        })
        .collect();

    // **`manifest.toml` is re-rendered from `carried` rather than copied
    // verbatim**, so a server never reads an entry of a kind it cannot hold or
    // one this workstation has no bytes to send it. The filtered list and the
    // manifest text are the SAME list, run through `crate::fleet::render`, the
    // one function that already owns this file's shape (`ADR-011`).
    let manifest = root.join(crate::fleet::MANIFEST_FILE);
    if manifest.is_file() {
        let rendered = crate::fleet::render(&carried).into_bytes();
        out.push(
            rivet_core::staging::Staged::new(
                APPLICATION,
                &format!("{REPOSITORY_PREFIX}/{}", crate::fleet::MANIFEST_FILE),
                rendered,
            )
            .map_err(|e| format!("{}: {e}", manifest.display()))?,
        );
    }

    for (dir, prefix) in [
        (root.join(rivetos_fleet_repo::DOCUMENTS_DIR), rivetos_fleet_repo::DOCUMENTS_DIR),
        (root.join(rivetos_fleet_repo::REGISTER_DIR), rivetos_fleet_repo::REGISTER_DIR),
    ] {
        for (relative, bytes) in walk(&dir)? {
            let path = format!("{REPOSITORY_PREFIX}/{prefix}/{relative}");
            out.push(rivet_core::staging::Staged::new(APPLICATION, &path, bytes).map_err(|e| format!("{path}: {e}"))?);
        }
    }

    Ok(out)
}

/// Every file under `dir`, as `(path relative to dir, using '/', bytes)` —
/// never descending into `.git/`, `crate::fleet::find_seed`'s own exclusion,
/// restated because a fleet repository's text half is exactly what this
/// walks and a `.git/` tree has nothing this module wants to carry.
///
/// `Ok(Vec::new())` for an absent directory — a repository with no document
/// recorded yet, or no register projection yet, is a real, empty state, not
/// a refusal.
fn walk(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = std::fs::read_dir(&current).map_err(|e| format!("cannot read {}: {e}", current.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("cannot read {}: {e}", current.display()))?;
            let path = entry.path();
            let is_dir = entry
                .file_type()
                .map_err(|e| format!("cannot stat {}: {e}", path.display()))?
                .is_dir();
            if is_dir {
                if path.file_name().is_some_and(|n| n == ".git") {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let relative = path
                .strip_prefix(dir)
                .map_err(|_| format!("{} is not under {}", path.display(), dir.display()))?
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            out.push((relative, bytes));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

#[cfg(test)]
#[path = "carry_tests.rs"]
mod tests;
