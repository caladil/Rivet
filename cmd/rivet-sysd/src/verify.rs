//! `ADR-015` §4.2: **an application's signature is verified at every launch.**
//!
//! Ruled 2026-08-25 by the maintainer, and the argument that decided it was
//! revocation rather than tampering: verifying at launch is the only way to
//! revoke the certificate of an installed application so that it stops running
//! once it is found to be malicious or corrupted (.rulings/ R-276).
//!
//! It is. `ADR-028` §5.5.4 rules that **revocation is an A/B image update** —
//! ship an image whose trust store no longer holds the key — and calls that a
//! complete answer. It is complete for *updates*, which are verified every time
//! one is installed. It was silently incomplete for *applications*, which were
//! verified once and then run for ever: dropping the key stopped future
//! installs and did nothing about what that key had already put on the machine.
//!
//! **So §5.5.4 and §4.2 are only consistent with each other once this exists**,
//! and neither ADR said so. `PLAN.md` §15 carries the finding.
//!
//! # What is checked, in order
//!
//! | | | catches |
//! | :-- | :-- | :-- |
//! | 1 | the retained header verifies against the trust store **as it is now** | a revoked key, an unknown key, a forged header |
//! | 2 | it is an `application` | a system update dressed as one |
//! | 3 | the program on disk hashes to the digest that header covers | a swapped or corrupted binary |
//!
//! Step 1 is the one the ruling is about. The trust store is read **on every
//! launch** rather than cached at start-up, because a cached one would answer
//! with the store the machine booted under and revocation arrives by replacing
//! that file.
//!
//! # What it does not check, said plainly
//!
//! **The manifest.** It is a signed member and its digest is in the same
//! header, so checking it is a few lines — but the supervisor has *already
//! acted* on the manifest by the time anything launches: it built the
//! `ServiceDef` from it at discovery. Verifying it here would report a fault
//! about a decision already taken. That belongs at discovery, and
//! `PLAN.md` §15 records it as the obvious next step rather than pretending
//! this covers it.
//!
//! **Anything after `execve`.** This is a check on the bytes at a path, at the
//! moment they are read. `ADR-015` §5.2's descriptor handoff does not exist, so
//! the kernel opens the file again when it execs it, and between those two
//! opens the file could change. Closing that needs the supervisor to hold the
//! descriptor it verified and exec *that*, which is `fexecve` and a different
//! change. **On this machine the window is not reachable by the application
//! itself** — the program is `0555` root-owned in a `0755` root-owned tree, so
//! the identity that runs it cannot write it — but it is reachable by anything
//! running as root, which is exactly who a launch check is meant to catch.

use rivet_core::package::application::Application;
use rivet_core::package::format::HEADER_BYTES;
use rivet_core::package::install::HEADER_NAME;
use rivet_core::package::kind::Kind;
use rivet_core::package::port::TrustSet;
use rivet_core::package::read::open_installed;
use rivet_core::package::trust_store;
use rivet_crypto::{Blake3, Ed25519Verifier};

use crate::kernel::Kernel;

/// The largest program this will read in order to hash it.
///
/// **The number is not decided here.** It moved to
/// `rivet_core::package::application` on 2026-09-01 so that `cargo rivet` can
/// refuse an oversized program at the moment it is built, rather than leaving
/// the first mention of the limit to be this module's `EFBIG` on somebody
/// else's machine. See that constant for the policy and for what provoked the
/// move.
pub use rivet_core::package::application::MAX_PROGRAM_BYTES;

/// The largest trust store this will read, derived rather than chosen:
/// `ADR-028` §5.5's format is a fixed header plus at most `MAX_ENTRIES` fixed
/// records, so a file bigger than that is not a trust store whatever else it is.
pub const MAX_TRUST_STORE_BYTES: usize =
    trust_store::HEADER_BYTES + trust_store::MAX_ENTRIES * trust_store::ENTRY_BYTES;

/// Why a launch was refused. Closed, per `ADR-006` §6.4 — each variant names a
/// distinct thing an operator would do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No header beside the install. An application installed by a Rivet
    /// older than 2026-08-25, or one somebody deleted it from.
    NoHeader { path: String, detail: String },
    /// The trust store could not be read. **Fails closed**: a machine that
    /// cannot say what it trusts does not get to run unverified code because of
    /// it.
    NoTrustStore { detail: String },
    /// The program could not be read to hash it.
    NoProgram { path: String, detail: String },
    /// The header did not verify against the trust store as it is now. **This
    /// is the revocation case** and also the forged-header case; they are
    /// indistinguishable from here and both mean the same thing.
    HeaderRejected { detail: String },
    /// The header verified and describes something that is not an application.
    NotAnApplication { kind: String },
    /// The program's bytes are not the ones the signature covers.
    ProgramChanged { detail: String },
}

impl Refusal {
    /// One line for the journal, in the words an operator can act on.
    pub fn describe(&self) -> String {
        match self {
            Self::NoHeader { path, detail } => format!(
                "REFUSED TO LAUNCH: no signed header at {path} ({detail}), so nothing can be \
                 verified. An application installed before 2026-08-25 has none — reinstall it. \
                 ADR-015 §4.2"
            ),
            Self::NoTrustStore { detail } => format!(
                "REFUSED TO LAUNCH: this machine's trust store at {} could not be read \
                 ({detail}). A machine that cannot say what it trusts does not run unverified \
                 code because of it (ADR-028 §5.5.6)",
                trust_store::PATH
            ),
            Self::NoProgram { path, detail } => format!(
                "REFUSED TO LAUNCH: the program at {path} could not be read to hash it \
                 ({detail}). ADR-015 §4.2 verifies at every launch, so a program this \
                 cannot read is a program it will not start"
            ),
            Self::HeaderRejected { detail } => format!(
                "REFUSED TO LAUNCH: the signed header does not verify against this machine's \
                 trust store AS IT IS NOW ({detail}). If this application ran before, ITS KEY \
                 HAS BEEN REVOKED by an image update — which is what revocation IS on this \
                 system (ADR-028 §5.5.4). Otherwise the header has been tampered with"
            ),
            Self::NotAnApplication { kind } => format!(
                "REFUSED TO LAUNCH: the header beside this install says `{kind}`, not \
                 `application` (ADR-029 §11). One kind of artifact is not another dressed \
                 differently"
            ),
            Self::ProgramChanged { detail } => format!(
                "REFUSED TO LAUNCH: the program on disk is NOT the one the signature covers \
                 ({detail}). It has been replaced or corrupted since it was installed — \
                 /state is writable and this check is the only thing that notices \
                 (ADR-015 §4.2)"
            ),
        }
    }

    /// A short word for the record's `fault` field.
    pub const fn word(&self) -> &'static str {
        match self {
            Self::NoHeader { .. } => "no-header",
            Self::NoTrustStore { .. } => "no-trust-store",
            Self::NoProgram { .. } => "no-program",
            Self::HeaderRejected { .. } => "header-rejected",
            Self::NotAnApplication { .. } => "not-an-application",
            Self::ProgramChanged { .. } => "program-changed",
        }
    }
}

/// Verify the program an application is about to run.
///
/// `version_dir` is the directory `current` resolves to and `program` is the
/// binary inside it. Both are the supervisor's, derived from the name — nothing
/// here takes a path from the artifact.
pub fn program(
    k: &mut dyn Kernel,
    version_dir: &str,
    program_path: &str,
) -> Result<(), Refusal> {
    let header_path = format!("{version_dir}/{HEADER_NAME}");
    let head = k
        .read_file_bytes(&header_path, HEADER_BYTES)
        .map_err(|e| Refusal::NoHeader {
            path: header_path.clone(),
            detail: e.to_string(),
        })?;

    // Read every launch, never cached. `ADR-028` §5.5.4 makes revocation an
    // image update, which replaces this file; a store cached at start-up would
    // answer with the one the machine booted under and the revocation would not
    // bite until something else restarted the supervisor.
    let store_bytes = k
        .read_file_bytes(trust_store::PATH, MAX_TRUST_STORE_BYTES)
        .map_err(|e| Refusal::NoTrustStore {
            detail: e.to_string(),
        })?;
    let store = trust_store::TrustStore::from_bytes(&store_bytes).map_err(|e| {
        Refusal::NoTrustStore {
            detail: e.to_string(),
        }
    })?;
    // Empty is a refusal and not "trusts nobody, so run it": a store that says
    // nothing is `--no-trust`'s development card, and `ADR-028` §5.5.5 already
    // makes that a state the machine announces rather than one it acts on.
    if store.is_empty() {
        return Err(Refusal::NoTrustStore {
            detail: "it is empty, so this machine trusts no signing key".to_string(),
        });
    }
    let mut trust = TrustSet::new();
    for entry in store.entries() {
        // **The entry's own mask**, exactly as `cmd/rivet-deploy` does it:
        // the store says what each key may sign and `read::open` enforces that
        // against the header's kind, so an extension key is refused here by the
        // same rule that refuses it at install.
        let _ = trust.trust(
            &mut Blake3::new(),
            entry.key,
            &entry.label,
            entry.may_sign.clone(),
        );
    }

    let package = open_installed(&head, &mut Blake3::new(), &Ed25519Verifier::new(), &trust)
        .map_err(|e| Refusal::HeaderRejected {
            detail: e.to_string(),
        })?;

    if *package.header().kind() != Kind::application() {
        return Err(Refusal::NotAnApplication {
            kind: package.header().kind().as_str().to_string(),
        });
    }

    let application = Application::open(&package).map_err(|e| Refusal::HeaderRejected {
        detail: e.to_string(),
    })?;
    let index = application.program_member().index();

    let bytes = k
        .read_file_bytes(program_path, MAX_PROGRAM_BYTES)
        .map_err(|e| Refusal::NoProgram {
            path: program_path.to_string(),
            detail: e.to_string(),
        })?;

    // The digest is over the member's STORED bytes, and `read.rs` refuses any
    // compression but `none` — so the stored form and the installed file are the
    // same bytes, and this comparison is the whole check rather than an
    // approximation of it.
    package
        .verify_member(index, &mut Blake3::new(), &bytes)
        .map_err(|e| Refusal::ProgramChanged {
            detail: e.to_string(),
        })?;

    Ok(())
}

/// A really signed application, for the tests of **two** modules.
///
/// `crate::supervise`'s launch path runs this module's check before it opens a
/// notification socket, so a test there that wants to reach the socket has to
/// get past verification first — and getting past it means a genuinely signed
/// package, not a stub. Rather than have two fixtures that could drift, there is
/// one, here, beside the thing it satisfies.
#[cfg(test)]
pub(crate) mod fixture {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use rivet_core::package::application::{Manifest, ProgramKind, TargetClass, VARIANT_NONE};
    use rivet_core::package::format::HEADER_BYTES;
    use rivet_core::package::install::HEADER_NAME;
    use rivet_core::package::kind::Kind;
    use rivet_core::package::port::MaySign;
    use rivet_core::package::trust_store::{self, Entry, TrustStore};
    use rivet_core::package::write::{Builder, NewMember};
    use rivet_crypto::{Blake3, Ed25519Signer, SEED_BYTES};

    use crate::testkernel::FakeKernel;

    /// Not a real key. Thirty-two identical bytes, so a failure reproduces.
    pub fn seed(byte: u8) -> [u8; SEED_BYTES] {
        [byte; SEED_BYTES]
    }

    /// The program's bytes. Deterministic, and not all one value, so a digest
    /// over them means something.
    pub fn program_bytes() -> Vec<u8> {
        (0u32..4096).map(|n| (n % 251) as u8).collect()
    }

    /// A signed application, as `(whole file, program bytes)`.
    pub fn signed_application(name: &str, with: [u8; SEED_BYTES]) -> (Vec<u8>, Vec<u8>) {
        let manifest_bytes = Manifest::new(ProgramKind::Daemon).render().into_bytes();
        let program = program_bytes();

        let mut digest = Blake3::new();
        let mut builder =
            Builder::new(name, "0.1.0", "aarch64", None, Kind::application()).expect("a header");
        builder
            .add(
                NewMember::stored(
                    rivet_core::package::application::MANIFEST_NAME,
                    TargetClass::Manifest.as_byte(),
                    VARIANT_NONE,
                    &manifest_bytes,
                    &mut digest,
                )
                .expect("a manifest member"),
            )
            .expect("added");
        builder
            .add(
                NewMember::stored(
                    name,
                    TargetClass::Program.as_byte(),
                    VARIANT_NONE,
                    &program,
                    &mut digest,
                )
                .expect("a program member"),
            )
            .expect("added");

        let mut signer = Ed25519Signer::from_seed(&with);
        let whole = builder
            .seal_in_memory(&mut digest, &mut signer, &[&manifest_bytes, &program])
            .expect("a sealed package");
        (whole, program)
    }

    /// A trust store holding one key, entitled for applications.
    pub fn store_trusting(seed_byte: u8) -> Vec<u8> {
        TrustStore::new(vec![Entry {
            key: rivet_crypto::public_key_for(&seed(seed_byte)),
            label: "test-app-key".to_string(),
            may_sign: MaySign::these(&[Kind::application()]),
        }])
        .to_bytes()
        .expect("a trust store")
    }

    /// Put a correctly installed, correctly signed application on a fake
    /// machine: the retained header, the program, and a store that trusts it.
    pub fn install_signed(k: &mut FakeKernel, name: &str, version_dir: &str, program: &str) {
        let (whole, bytes) = signed_application(name, seed(7));
        k.blobs.insert(
            format!("{version_dir}/{HEADER_NAME}"),
            whole[..HEADER_BYTES].to_vec(),
        );
        k.blobs.insert(program.to_string(), bytes);
        k.blobs
            .insert(trust_store::PATH.to_string(), store_trusting(7));
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    //! **The whole point of this module is the revocation test**, and the rest
    //! of these exist so that it cannot pass by accident.
    //!
    //! Every one drives a REAL signed package through the REAL verifier: the
    //! package is built with `rivet_core::package::write`, signed with a
    //! seed, and its header put where the supervisor will look for it. Nothing
    //! is faked below the `Kernel` port, so a change to the container format or
    //! to the signature scheme fails here rather than passing against a stub.
    use super::fixture::{install_signed, program_bytes, store_trusting};
    use super::*;
    use crate::testkernel::FakeKernel;
    use rivet_core::package::trust_store::TrustStore;

    const NAME: &str = "photo-index";
    const VERSION_DIR: &str = "/state/app/photo-index/current";
    const PROGRAM: &str = "/state/app/photo-index/current/bin/photo-index";

    /// A machine with a correctly installed, correctly signed application.
    fn healthy() -> FakeKernel {
        let mut k = FakeKernel::new();
        install_signed(&mut k, NAME, VERSION_DIR, PROGRAM);
        k
    }

    #[test]
    fn a_correctly_signed_application_launches() {
        let mut k = healthy();
        assert_eq!(program(&mut k, VERSION_DIR, PROGRAM), Ok(()));
    }

    /// **THE RULING, AS A TEST.**
    ///
    /// `ADR-028` §5.5.4 makes revocation an image update — ship an image whose
    /// trust store no longer holds the key. That is only true if something
    /// re-checks, and this is the something. The application is untouched: same
    /// bytes, same header, same signature. Only the machine's trust store
    /// changed, which is exactly what an image update does.
    #[test]
    fn revoking_the_key_stops_an_already_installed_application_from_launching() {
        let mut k = healthy();
        assert_eq!(
            program(&mut k, VERSION_DIR, PROGRAM),
            Ok(()),
            "it must run before the revocation, or this test proves nothing"
        );

        // The image update: a trust store that holds a different key. Nothing
        // on /state is touched.
        k.blobs
            .insert(trust_store::PATH.to_string(), store_trusting(9));

        let refused = program(&mut k, VERSION_DIR, PROGRAM)
            .expect_err("a revoked key must not launch");
        assert_eq!(refused.word(), "header-rejected");
        assert!(
            refused.describe().contains("REVOKED"),
            "the operator has to be told this is what revocation looks like: {}",
            refused.describe()
        );
    }

    /// One bit of the program, and the launch is refused.
    #[test]
    fn a_program_that_changed_after_it_was_installed_does_not_launch() {
        let mut k = healthy();
        let mut tampered = program_bytes();
        tampered[2000] ^= 0x01;
        k.blobs.insert(PROGRAM.to_string(), tampered);

        let refused =
            program(&mut k, VERSION_DIR, PROGRAM).expect_err("a changed program must not launch");
        assert_eq!(refused.word(), "program-changed");
    }

    /// An application installed by a Rivet that predates the retained header.
    /// **Refused, not permitted** — the whole guarantee is that a launch either
    /// verified or did not happen.
    #[test]
    fn an_application_with_no_retained_header_does_not_launch() {
        let mut k = healthy();
        k.blobs.remove(&format!("{VERSION_DIR}/{HEADER_NAME}"));

        let refused = program(&mut k, VERSION_DIR, PROGRAM).expect_err("no header, no launch");
        assert_eq!(refused.word(), "no-header");
        assert!(
            refused.describe().contains("reinstall"),
            "a refusal an operator cannot act on is half a refusal: {}",
            refused.describe()
        );
    }

    /// **Fails closed.** A machine that cannot read its own trust store does
    /// not get to run unverified code because of it.
    #[test]
    fn an_unreadable_trust_store_refuses_rather_than_trusting_everything() {
        let mut k = healthy();
        k.blobs.remove(trust_store::PATH);

        let refused =
            program(&mut k, VERSION_DIR, PROGRAM).expect_err("no trust store, no launch");
        assert_eq!(refused.word(), "no-trust-store");
    }

    /// An empty store is `--no-trust`'s development card. It trusts nobody, so
    /// nothing verifies, so nothing launches — rather than "nobody is
    /// forbidden, so everything runs".
    #[test]
    fn an_empty_trust_store_refuses_rather_than_permitting_everything() {
        let mut k = healthy();
        k.blobs.insert(
            trust_store::PATH.to_string(),
            TrustStore::new(vec![]).to_bytes().expect("an empty store"),
        );

        let refused =
            program(&mut k, VERSION_DIR, PROGRAM).expect_err("an empty store must not launch");
        assert_eq!(refused.word(), "no-trust-store");
    }

    /// The trust store is read **every** launch, never cached — because
    /// revocation arrives by replacing that file and a cached copy would answer
    /// with the one the machine booted under.
    #[test]
    fn the_trust_store_is_read_on_every_launch() {
        let mut k = healthy();
        let before = k.reads.iter().filter(|p| p == &trust_store::PATH).count();
        let _ = program(&mut k, VERSION_DIR, PROGRAM);
        let _ = program(&mut k, VERSION_DIR, PROGRAM);
        assert!(
            k.reads.iter().filter(|p| p == &trust_store::PATH).count() >= before + 2,
            "two launches must read the trust store twice"
        );
    }

    /// A program larger than [`MAX_PROGRAM_BYTES`] is a **refusal**, never a
    /// truncated read that hashes three quarters of a binary and calls it
    /// verified.
    #[test]
    fn a_program_over_the_bound_is_refused_rather_than_partly_hashed() {
        let mut k = healthy();
        k.blobs
            .insert(PROGRAM.to_string(), vec![0u8; MAX_PROGRAM_BYTES + 1]);

        let refused = program(&mut k, VERSION_DIR, PROGRAM).expect_err("over the bound");
        assert_eq!(refused.word(), "no-program");
    }

    /// Every refusal says what it is and what to do, and none of them is empty.
    #[test]
    fn every_refusal_names_itself_and_cites_something() {
        let refusals = [
            Refusal::NoHeader {
                path: "/p".to_string(),
                detail: "d".to_string(),
            },
            Refusal::NoTrustStore {
                detail: "d".to_string(),
            },
            Refusal::NoProgram {
                path: "/p".to_string(),
                detail: "d".to_string(),
            },
            Refusal::HeaderRejected {
                detail: "d".to_string(),
            },
            Refusal::NotAnApplication {
                kind: "update".to_string(),
            },
            Refusal::ProgramChanged {
                detail: "d".to_string(),
            },
        ];
        for refusal in refusals {
            let text = refusal.describe();
            assert!(
                text.starts_with("REFUSED TO LAUNCH"),
                "a refusal must say so first: {text}"
            );
            assert!(text.contains("ADR-0"), "{text} cites nothing");
            assert!(!refusal.word().is_empty());
        }
    }
}
