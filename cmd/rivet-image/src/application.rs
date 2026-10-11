//! **The payloads a card carries, verified before it is written** — `ADR-050`
//! §4, ruled 2026-09-07.
//!
//! # Why the verification is here and not on the board
//!
//! `ADR-050` §4 is explicit: a payload that will not verify must be refused
//! *where a person can see it*. The board is the wrong end — a card carrying a
//! payload signed by a key its own trust store will not accept boots to a
//! machine with no fleet server, and the only account of why is a journal
//! record on a headless box somebody has to go and read.
//!
//! This crate already writes that trust store (`ADR-028` §5 — an image says
//! what it trusts), so it is the one place in the world that can answer *will
//! this card accept this payload* before the card exists. It is also the
//! cheapest place: the bytes are on the workstation's disk already.
//!
//! # It is not a second verifier
//!
//! `ADR-011`. [`rivet_core::package::read::open`] is the verifier and
//! `cmd/rivet-deploy` calls the same function on the machine; what this module
//! adds is a driver that reads a file in chunks and a refusal written for the
//! person running a build. `cmd/cargo-rivet`'s `inspect` is the third caller of
//! the same code, and the three agree because they are the same code.
//!
//! **The trust set is the IMAGE's**, built from the same `--trusted-key`
//! entries that are about to be serialised onto `/system`. That is the whole
//! value of doing it here rather than passing a key on the command line: what
//! is asserted is not *this payload has a valid signature* but **this card will
//! install this payload**.
//!
//! # What is checked, in order
//!
//! 1. The header verifies against the image's own trust store, including
//!    `ADR-028` §5's entitlement — so an application signed with the SYSTEM
//!    key is refused here exactly as `cmd/rivet-deploy` would refuse it.
//! 2. **Every member's digest**, before anything is reported — `ADR-029` §9
//!    step 6's order, and `cmd/cargo-rivet`'s `inspect` gives the reason a
//!    reader needs it too: reporting a product name out of a package whose
//!    bytes have not been checked would be describing something nobody signed.
//! 3. The `kind` is `application`, and every other kind is refused **by name**
//!    with the reason. An `update` is `ADR-018` §5's A/B mechanism and does not
//!    arrive this way; a `library` is a real gap and the refusal says so, since
//!    a library installs to a content-addressed directory the supervisor cannot
//!    name from the product alone — which is the question its carried watch
//!    asks the filesystem to decide whether an install landed.
//! 4. The header's `product` is a name a machine will accept, because it
//!    becomes the file name on the boot partition and a directory under
//!    `/state/carried` (`rivet_core::carried`).
//!
//! # What is NOT checked, and is not an omission
//!
//! **Whether the machine will run it.** An application's manifest can ask for a
//! device, an endpoint or a shared library this card does not have, and the
//! supervisor refuses that at discovery with a message naming what was missing.
//! Reproducing those rules here would be `ADR-011`'s second authority for a
//! question the machine answers better, since the answer depends on the
//! document as well as the card.

use std::path::Path;

use rivet_core::package::Kind;
use rivet_core::package::format::HEADER_BYTES;
use rivet_core::package::port::{Digest, TrustSet};
use rivet_core::package::read::{Member, Package, open};
use rivet_core::package::trust_store::TrustStore;

/// How much of a member is read at once. `cmd/cargo-rivet`'s `inspect` uses the
/// same figure, and for the same reason: a payload is megabytes and a build
/// host is not a Raspberry Pi, but neither is a reason to hold a whole member
/// twice.
const CHUNK: usize = 1024 * 1024;

/// A payload that verified, and what the card will call it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Carried {
    /// The header's `product`, checked against `ServiceName`. **This and not
    /// the source file's name** — `ADR-027` §5 refuses to read a fact out of a
    /// file name, and `ADR-015` §7 makes the signed header the one
    /// authority for what a machine will call this.
    pub product: String,
    /// The header's `version`, for the build's report.
    pub version: String,
    /// `application`, and nothing else is carried — see [`verify`].
    pub kind: String,
    /// The whole file, to be written to the boot partition unchanged.
    pub bytes: Vec<u8>,
}

impl Carried {
    /// What this payload is called on the boot partition.
    #[must_use]
    pub fn card_file(&self) -> String {
        rivet_core::carried::card_file(&self.product)
    }
}

/// Read a `.rvt`, verify it against the image's own trust store, and say what
/// the card will call it.
///
/// # Errors
///
/// Every refusal is a card that would boot to a machine missing this
/// application, so each one names the file and what was wrong with it.
pub fn verify(path: &Path, trust_store: &TrustStore) -> Result<Carried, String> {
    if trust_store.is_empty() {
        return Err(format!(
            "{} was asked for and this image trusts no signing key, so the card could not \
             install it. A --no-trust image verifies nothing (ADR-028 §5), and a payload \
             on it is a program that reaches the machine and is refused there. Pass \
             --trusted-key for the key this payload was signed with, or drop --application.",
            path.display()
        ));
    }

    let bytes = std::fs::read(path).map_err(|e| {
        format!(
            "cannot read the application payload at {}: {e}. `cargo rivet package` writes one",
            path.display()
        )
    })?;
    let file_len = bytes.len() as u64;
    let head = bytes.get(..HEADER_BYTES).ok_or_else(|| {
        format!(
            "{} is {file_len} bytes and a package header is {HEADER_BYTES}; there is nothing to \
             verify",
            path.display()
        )
    })?;

    // **The image's own keys, with the entitlements the image will carry.**
    // `read::open` checks the header's kind against the key's `may_sign`, so an
    // application signed by the system key is refused HERE by the same rule
    // that refuses it on the board — `ADR-028` §5, and this crate's own test
    // `the_system_key_is_not_entitled_to_sign_applications` already pins the
    // half of it that lives in the store.
    let mut digest = rivet_crypto::Blake3::new();
    let verifier = rivet_crypto::Ed25519Verifier::new();
    let mut trust = TrustSet::new();
    for entry in trust_store.entries() {
        trust.trust(
            &mut digest,
            entry.key,
            &entry.label,
            entry.may_sign.clone(),
        );
    }

    let package = open(head, file_len, &mut digest, &verifier, &trust).map_err(|e| {
        format!(
            "{}: refused — {e}\n  \
             This is the refusal the BOARD would give, from the same verifier, against the \
             trust store this very image is about to carry. A card written now would boot \
             and never install this application.",
            path.display()
        )
    })?;

    // Every member, before anything is read out of the metadata.
    verify_every_member(path, &bytes, &package, &mut digest)?;

    let kind = package.header().kind().to_string();
    // **The kind's own structure, through the same reader the board uses.**
    // `ADR-029` §11: the container is one and the interpretation is per-kind, so
    // a container that verifies is not thereby an application — an `application`
    // with no manifest member, or two programs, is refused by
    // `Application::open` on the machine and is refused here by the same
    // function. `cmd/cargo-rivet`'s `inspect` dispatches identically.
    if kind == Kind::application().as_str() {
        rivet_core::package::application::Application::open(&package)
            .map_err(|e| format!("{}: refused — {e}", path.display()))?;
        // **An application under an image service's name is refused too**, by
        // the rule the board's install door applies (`ADR-060` §4): this image
        // already provides the service, so the machine would refuse the payload
        // on its first boot.
        rivet_core::package::verify::not_an_image_service(&package).map_err(|e| {
            format!(
                "{}: refused — {e}. This image provides the service itself, so a card does not \
                 carry it (ADR-060).",
                path.display()
            )
        })?;
    } else {
        return Err(format!(
            "{}: its <kind> is {kind:?}, and a card carries {:?} and nothing else.\n  \
             An UPDATE is `ADR-018` §5's A/B mechanism and does not arrive as something the \
             supervisor hands to cmd/rivet-deploy on a running machine.\n  \
             A LIBRARY is refused for a narrower reason and it is a real gap: a library \
             installs to a CONTENT-ADDRESSED directory whose name carries a digest \
             (`rivet_core::package::library::Reference`), so nothing on the machine can work \
             out from the product name alone whether the install landed — and \
             `cmd/rivet-sysd`'s carried watch decides whether to keep a payload by asking the \
             filesystem exactly that. Deploy a library with `cargo rivet deploy` after the \
             first boot; carrying one needs that question answered first (`ADR-050` §10).",
            path.display(),
            Kind::application().as_str(),
        ));
    }

    let product = package.header().product();
    let name = rivet_core::carried::product_of(&rivet_core::carried::card_file(product))
        .ok_or_else(|| {
            format!(
                "{}: its product is {product:?}, which is not a name this machine can make a \
                 service of — so nothing could install it and there is no directory it could \
                 land in. The name is chosen by whoever built the package (ADR-015 §7); \
                 rebuild it with one.",
                path.display()
            )
        })?;

    Ok(Carried {
        product: name,
        version: package.header().version().to_string(),
        kind,
        bytes,
    })
}

/// `ADR-029` §9 step 6 — **every member, before anything is reported.**
fn verify_every_member(
    path: &Path,
    bytes: &[u8],
    package: &Package,
    digest: &mut dyn Digest,
) -> Result<(), String> {
    for member in package.members() {
        verify_member(bytes, package, member, digest).map_err(|e| {
            format!(
                "{}: refused — member {} ({}) does not match its digest: {e}",
                path.display(),
                member.index(),
                member.name(),
            )
        })?;
    }
    Ok(())
}

/// One member, streamed through the digest the header committed to.
///
/// The file is already resident — a payload is megabytes and this is a build
/// host — so this reads out of the slice rather than seeking, and the chunking
/// exists to keep the streaming interface honest rather than to save memory.
fn verify_member(
    bytes: &[u8],
    package: &Package,
    member: &Member,
    digest: &mut dyn Digest,
) -> Result<(), rivet_core::package::PackageError> {
    let mut stream = package.member_stream(member.index(), digest)?;
    let start = usize::try_from(member.offset()).unwrap_or(usize::MAX);
    let mut offset = start;
    let end = start.saturating_add(usize::try_from(member.stored_len()).unwrap_or(usize::MAX));
    while offset < end {
        let stop = end.min(offset.saturating_add(CHUNK));
        let Some(slice) = bytes.get(offset..stop) else {
            // The header says these bytes exist and the file ended. `finish`
            // is what turns that into `MemberShort`, which names both numbers;
            // breaking rather than erroring here keeps one refusal per fault.
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

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use rivet_core::package::application::{
        Manifest, ProgramKind, TargetClass, VARIANT_NONE,
    };
    use rivet_core::package::port::{MaySign, PublicKey, Sign};
    use rivet_core::package::trust_store::Entry;
    use rivet_core::package::write::{Builder, NewMember};

    /// Two distinct seeds, so a payload can be signed by the WRONG one and the
    /// refusal is about entitlement or about an unknown key rather than about
    /// a coincidence.
    const APP_SEED: [u8; 32] = [7u8; 32];
    const SYSTEM_SEED: [u8; 32] = [9u8; 32];

    fn public_of(seed: &[u8; 32]) -> PublicKey {
        rivet_crypto::Ed25519Signer::from_seed(seed).public_key()
    }

    /// A trust store shaped like the one `tools/card/build.sh` builds: the
    /// system key for updates and extensions, the application key for
    /// applications and libraries. `ADR-028` §5 — two keys on purpose, so
    /// that a machine holding an application key cannot ship an operating
    /// system.
    fn a_cards_trust() -> TrustStore {
        TrustStore::new(vec![
            Entry {
                key: public_of(&SYSTEM_SEED),
                label: "rivet-system".to_string(),
                // `extension` has no constructor — `ADR-016`'s kind is unruled
                // (`ADR-027` §9) — so this is `update` alone, which is the half
                // that matters: the system key may not sign an application.
                may_sign: MaySign::these(&[Kind::update()]),
            },
            Entry {
                key: public_of(&APP_SEED),
                label: "rivet-app".to_string(),
                may_sign: MaySign::these(&[Kind::application(), Kind::library()]),
            },
        ])
    }

    /// A real, signed application payload — built through the writer that
    /// `cargo rivet package` uses and sealed with the real Ed25519.
    ///
    /// **Not a fake.** `rivet_core::package::testing` has stand-ins for the two
    /// algorithms, and using them here would prove that this module agrees with
    /// a fake rather than that a card would accept a payload.
    fn a_payload(product: &str, seed: &[u8; 32]) -> Vec<u8> {
        let manifest = Manifest::new(ProgramKind::Daemon).render().into_bytes();
        let program = b"\x7fELF not really, and nothing here execs it".to_vec();

        let mut digest = rivet_crypto::Blake3::new();
        let mut signer = rivet_crypto::Ed25519Signer::from_seed(seed);
        let mut builder = Builder::new(
            product,
            "1.2.3",
            "aarch64",
            // An application carries no platform — the maintainer's 2026-08-24
            // ruling, enforced by `Application::open`.
            None,
            Kind::application(),
        )
        .expect("the header fields are valid");
        builder
            .add(
                NewMember::stored(
                    rivet_core::package::application::MANIFEST_NAME,
                    TargetClass::Manifest.as_byte(),
                    VARIANT_NONE,
                    &manifest,
                    &mut digest,
                )
                .expect("a manifest member"),
            )
            .expect("the manifest fits");
        builder
            .add(
                NewMember::stored(
                    product,
                    TargetClass::Program.as_byte(),
                    VARIANT_NONE,
                    &program,
                    &mut digest,
                )
                .expect("a program member"),
            )
            .expect("the program fits");
        builder
            .seal_in_memory(&mut digest, &mut signer, &[&manifest, &program])
            .expect("the payload seals")
    }

    fn written(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rivet-image-carried-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let at = dir.join(name);
        std::fs::write(&at, bytes).expect("the payload is written");
        at
    }

    /// **The ordinary case**, and the property that makes the rest meaningful:
    /// the name on the card comes out of the SIGNED header, never out of the
    /// file the operator named.
    #[test]
    fn a_payload_the_card_will_accept_is_named_by_its_own_header() {
        let at = written(
            "something-else-entirely.rvt",
            &a_payload("rivet-fleetd", &APP_SEED),
        );
        let carried = verify(&at, &a_cards_trust()).expect("the app key is trusted for this");

        assert_eq!(carried.product, "rivet-fleetd");
        assert_eq!(carried.version, "1.2.3");
        assert_eq!(carried.kind, Kind::application().as_str());
        assert_eq!(carried.card_file(), "app-rivet-fleetd.rvt");
        assert_eq!(
            rivet_core::carried::product_of(&carried.card_file()).as_deref(),
            Some("rivet-fleetd"),
            "the name the card carries does not read back as the product"
        );
    }

    /// **A payload under an image service's name is refused on the
    /// workstation** — `ADR-060` §4. This image provides the proxy, and the
    /// board's install door would refuse it on the first boot.
    #[test]
    fn a_payload_named_like_a_service_the_image_provides_is_refused() {
        let at = written("old-proxy.rvt", &a_payload("rivet-proxyd", &APP_SEED));
        let why = verify(&at, &a_cards_trust()).expect_err("the image provides rivet-proxyd");
        assert!(why.contains("old-proxy.rvt"), "{why}");
        assert!(why.contains("rivet-proxyd"), "{why}");
        assert!(why.contains("ADR-060"), "{why}");
    }

    /// **The build-time refusal `ADR-050` §4 exists for.** A payload signed by
    /// a key this card's trust store will not accept is a card that boots to
    /// nothing, and it is discoverable on the workstation.
    ///
    /// The key here is one the store **has**, and it is refused on
    /// ENTITLEMENT: `ADR-028` §5 says the system key may not sign an
    /// application. That is the sharper case than an unknown key, because it is
    /// the mistake somebody actually makes — signing with the key they already
    /// had.
    #[test]
    fn a_payload_signed_by_a_key_this_card_will_not_accept_is_refused() {
        let at = written("wrong-key.rvt", &a_payload("rivet-fleetd", &SYSTEM_SEED));
        let why = verify(&at, &a_cards_trust()).expect_err("the system key signed an application");
        assert!(
            why.contains("wrong-key.rvt"),
            "the refusal does not name the file: {why}"
        );
        assert!(
            why.contains("refused"),
            "the refusal does not say it refused: {why}"
        );
        assert!(
            why.to_lowercase().contains("entitle") || why.to_lowercase().contains("sign"),
            "the refusal does not name the cause: {why}"
        );
    }

    /// A key the store does not hold, which is the other half of the same
    /// gate.
    #[test]
    fn a_payload_signed_by_a_key_this_card_has_never_heard_of_is_refused() {
        let store = TrustStore::new(vec![Entry {
            key: public_of(&SYSTEM_SEED),
            label: "rivet-system".to_string(),
            may_sign: MaySign::these(&[Kind::update(), Kind::application()]),
        }]);
        let at = written("unknown-key.rvt", &a_payload("rivet-fleetd", &APP_SEED));
        let why = verify(&at, &store).expect_err("that key is not in the store");
        assert!(why.contains("unknown-key.rvt"), "{why}");
    }

    /// **`ADR-050` §4's failure, at the moment it can still be fixed.** A
    /// `--no-trust` card verifies nothing, so a payload on it is a program that
    /// reaches the machine and is refused there — on a headless board, at 11pm.
    #[test]
    fn a_payload_asked_for_on_a_card_that_trusts_nobody_is_refused_by_name() {
        let at = written("no-trust.rvt", &a_payload("rivet-fleetd", &APP_SEED));
        let why =
            verify(&at, &TrustStore::new(Vec::new())).expect_err("an empty store trusts nobody");
        assert!(why.contains("no-trust.rvt"), "{why}");
        assert!(
            why.contains("trusts no signing key"),
            "the refusal does not say what is missing: {why}"
        );
        assert!(
            why.contains("--trusted-key"),
            "the refusal does not say how to fix it: {why}"
        );
    }

    /// A file that is not a package at all — the shape a truncated copy or a
    /// wrong path takes.
    #[test]
    fn a_file_that_is_not_a_package_is_refused_rather_than_carried() {
        let at = written("not-a-package.rvt", b"this is not a .rvt");
        let why = verify(&at, &a_cards_trust()).expect_err("18 bytes is not a package");
        assert!(why.contains("not-a-package.rvt"), "{why}");
    }

    /// **A payload whose bytes were edited after it was signed.** The header
    /// still verifies — it is signed and unchanged — and the MEMBER digest is
    /// what catches it, which is why every member is checked before anything is
    /// reported (`ADR-029` §9 step 6).
    #[test]
    fn a_payload_whose_program_was_edited_after_signing_is_refused() {
        let mut bytes = a_payload("rivet-fleetd", &APP_SEED);
        let last = bytes.len().saturating_sub(1);
        bytes[last] ^= 0xff;
        let at = written("tampered.rvt", &bytes);
        let why = verify(&at, &a_cards_trust()).expect_err("a member was changed");
        assert!(why.contains("tampered.rvt"), "{why}");
        assert!(
            why.contains("member"),
            "the refusal does not name the member that failed: {why}"
        );
    }

    /// A missing file says which file, rather than `No such file or directory`
    /// — `AGENTS.md` §3.
    #[test]
    fn a_payload_that_is_not_there_names_itself_and_the_tool_that_makes_one() {
        let why = verify(
            std::path::Path::new("no/such/payload.rvt"),
            &a_cards_trust(),
        )
        .expect_err("that path names no file");
        assert!(why.contains("payload.rvt"), "{why}");
        assert!(why.contains("cargo rivet package"), "{why}");
    }
}
