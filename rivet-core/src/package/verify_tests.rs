//! The whole-artifact check, against a source that is a buffer.
//!
//! **Provenance, `AGENTS.md` §4:** every artifact here is built by this crate's
//! own writer and signed with [`ToySigner`], so what these prove is the
//! *sequence* — which reads happen, in what order, which failure is whose — and
//! nothing about Ed25519 or BLAKE3. `cmd/rivet-deploy`'s suite and
//! `cmd/rivet-mcpd`'s drive the same functions with the real algorithms.
//!
//! The trust-store refusals were tested in `cmd/rivet-deploy` against its fake
//! kernel until 2026-09-12, when the code that decides them moved here. Those
//! tests stay where they were, because they assert the installer's own remedy
//! text; the decisions are asserted below, once.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::package::kind::Kind;
use crate::package::port::MaySign;
use crate::package::testing::{Event, ToyDigest, ToyKey, ToySigner, ToyVerifier, Trace};
use crate::package::trust_store::{Entry, TrustStore};
use crate::package::write::{Builder, NewMember};

const TRUSTED: ToyKey = ToyKey(0x21);
const STRANGER: ToyKey = ToyKey(0x77);

/// Bytes, and a record of every range asked for.
struct Buffer {
    bytes: Vec<u8>,
    asked: Vec<(u64, u64)>,
    /// Fail any read that reaches this offset, to stand in for a card that
    /// stops answering part-way.
    fail_at: Option<u64>,
}

impl Buffer {
    fn of(bytes: &[u8]) -> Self {
        Self {
            bytes: bytes.to_vec(),
            asked: Vec::new(),
            fail_at: None,
        }
    }
}

impl Source for Buffer {
    type Error = String;

    fn read_range(
        &mut self,
        offset: u64,
        len: u64,
        sink: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<(), String> {
        self.asked.push((offset, len));
        let end = offset.saturating_add(len);
        if let Some(at) = self.fail_at
            && end > at
        {
            return Err(format!("the medium stopped answering at {at}"));
        }
        let (start, stop) = (offset as usize, end as usize);
        if stop > self.bytes.len() {
            return Err(format!("only {} bytes, asked for {start}..{stop}", self.bytes.len()));
        }
        // Several chunks, so a stream that refuses part-way is exercised.
        for chunk in self.bytes[start..stop].chunks(1000) {
            if !sink(chunk) {
                break;
            }
        }
        Ok(())
    }
}

fn members() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("manifest.toml", b"kind = \"daemon\"\n".to_vec()),
        ("program", (0..5000usize).map(|i| (i % 251) as u8).collect()),
        ("banner.txt", b"a banner\n".to_vec()),
    ]
}

fn built_by(key: ToyKey, kind: Kind) -> Vec<u8> {
    built_for(key, kind, "aarch64")
}

fn built_for(key: ToyKey, kind: Kind, arch: &str) -> Vec<u8> {
    let mut digest = ToyDigest::new();
    let mut signer = ToySigner::new(key);
    let mut builder = Builder::new("photo-index", "0.1.0", arch, None, kind).unwrap();
    let parts = members();
    for (i, (name, bytes)) in parts.iter().enumerate() {
        builder
            .add(NewMember::stored(name, i as u8, 0, bytes, &mut digest).unwrap())
            .unwrap();
    }
    let slices: Vec<&[u8]> = parts.iter().map(|(_, b)| b.as_slice()).collect();
    builder
        .seal_in_memory(&mut digest, &mut signer, &slices)
        .unwrap()
}

/// Like [`built_for`], with the product name the caller chose instead of the
/// fixed `"photo-index"` — for [`not_an_image_service`], which that name never
/// exercises.
fn built_named(key: ToyKey, kind: Kind, product: &str) -> Vec<u8> {
    let mut digest = ToyDigest::new();
    let mut signer = ToySigner::new(key);
    let mut builder = Builder::new(product, "0.1.0", "aarch64", None, kind).unwrap();
    let parts = members();
    for (i, (name, bytes)) in parts.iter().enumerate() {
        builder
            .add(NewMember::stored(name, i as u8, 0, bytes, &mut digest).unwrap())
            .unwrap();
    }
    let slices: Vec<&[u8]> = parts.iter().map(|(_, b)| b.as_slice()).collect();
    builder
        .seal_in_memory(&mut digest, &mut signer, &slices)
        .unwrap()
}

fn store(kinds: &[Kind]) -> Vec<u8> {
    TrustStore::new(vec![Entry {
        key: TRUSTED.public(),
        label: "test-apps".to_string(),
        may_sign: MaySign::these(kinds),
    }])
    .to_bytes()
    .unwrap()
}

fn trusting(kinds: &[Kind]) -> TrustSet {
    trust_set::<String>(Ok(store(kinds)), &mut ToyDigest::new()).expect("a usable store")
}

fn check(bytes: &[u8], trust: &TrustSet) -> (Result<Opened, Fault<String>>, Buffer) {
    let mut source = Buffer::of(bytes);
    let outcome = artifact(
        &mut source,
        bytes.len() as u64,
        &mut ToyDigest::new(),
        &ToyVerifier::new(),
        trust,
    );
    (outcome, source)
}

// ---------------------------------------------------------------------------
// The trust set, out of the machine's own store — ADR-028 §5.
// ---------------------------------------------------------------------------

#[test]
fn a_store_that_cannot_be_read_is_refused_and_names_the_path() {
    let refused = trust_set::<&str>(Err("No such file or directory"), &mut ToyDigest::new())
        .expect_err("no store, no trust");
    assert!(matches!(refused, TrustRefusal::Unreadable(_)), "{refused:?}");
    let said = refused.to_string();
    assert!(said.contains(trust_store::PATH), "{said}");
    assert!(said.contains("No such file"), "{said}");
    assert!(said.contains("--no-trust"), "{said}");
}

/// **A store that cannot be parsed is not a store that trusts nobody.**
#[test]
fn a_corrupt_store_is_refused_rather_than_treated_as_trusting_nobody() {
    let mut damaged = store(&[Kind::application()]);
    damaged[0] ^= 0xff;
    let refused = trust_set::<String>(Ok(damaged), &mut ToyDigest::new())
        .expect_err("a corrupt store must not yield a trust set");
    assert!(matches!(refused, TrustRefusal::Corrupt(_)), "{refused:?}");
    assert!(refused.to_string().contains("does not parse"), "{refused}");
}

#[test]
fn an_empty_store_is_refused_as_the_machine_trusting_nobody() {
    let empty = TrustStore::new(Vec::new()).to_bytes().unwrap();
    let refused = trust_set::<String>(Ok(empty), &mut ToyDigest::new())
        .expect_err("an empty store verifies nothing");
    assert_eq!(refused, TrustRefusal::Empty);
    assert!(refused.to_string().contains("empty"), "{refused}");
    assert!(refused.to_string().contains("--no-trust"), "{refused}");
}

/// **Each entry keeps its own mask.** A key the store trusts for applications
/// only is refused for an update by the entitlement check, naming the label —
/// so this cannot pass for the wrong reason (`UntrustedKey` would mean the
/// store's key was never added).
#[test]
fn an_entry_keeps_its_own_mask_and_its_label_reaches_the_refusal() {
    let update = built_by(TRUSTED, Kind::update());
    let (outcome, _) = check(&update, &trusting(&[Kind::application()]));
    let Err(Fault::Artifact(PackageError::KindNotPermitted { label, kind, .. })) = outcome else {
        panic!("an application-only key authorised an update: {outcome:?}");
    };
    assert_eq!(label, "test-apps");
    assert_eq!(kind, "update");
}

/// **`R-177`/`ADR-028` §5: only `rivet-system` may sign a `card`.** A key
/// entitled to `application` only is refused by name — the same entitlement
/// check every other kind already goes through, exercised here for the new
/// one so its own wiring is proven rather than assumed from the others'.
#[test]
fn a_card_signed_by_a_key_not_entitled_to_card_is_refused_by_name() {
    let card = built_by(TRUSTED, Kind::card());
    let (outcome, _) = check(&card, &trusting(&[Kind::application()]));
    let Err(Fault::Artifact(PackageError::KindNotPermitted { label, kind, may_sign, .. })) =
        outcome
    else {
        panic!("an application-only key authorised a card: {outcome:?}");
    };
    assert_eq!(label, "test-apps");
    assert_eq!(kind, "card");
    assert_eq!(may_sign, vec!["application".to_string()]);
}

/// A `card` artifact whose one member is zstd-compressed — `R-177.1`. The
/// container never runs zstd itself (`ADR-029` §5.1: the decompressor never
/// sees bytes the signer did not produce), so what this proves is only that
/// the format carries a `stored_len` different from `plain_len` and digests
/// the *stored* bytes — exactly as an uncompressed member does, one field
/// wider.
fn built_compressed_card(key: ToyKey, stored: &[u8], plain_len: u64) -> Vec<u8> {
    let mut digest = ToyDigest::new();
    let mut signer = ToySigner::new(key);
    let mut builder =
        Builder::new("rivet", "0.1.87", "aarch64", Some("rpi4"), Kind::card()).unwrap();
    let member = NewMember::streamed(
        "image",
        0,
        0,
        crate::package::read::COMPRESSION_ZSTD,
        digest.of(stored),
        stored.len() as u64,
        plain_len,
    )
    .unwrap();
    builder.add(member).unwrap();
    builder
        .seal_in_memory(&mut digest, &mut signer, &[stored])
        .unwrap()
}

#[test]
fn a_cards_compressed_member_verifies_against_its_stored_bytes_and_keeps_its_plain_len() {
    let stored = b"pretend-this-is-zstd-compressed-bytes";
    let card = built_compressed_card(TRUSTED, stored, 2_147_483_648);
    let (outcome, _) = check(&card, &trusting(&[Kind::card()]));
    let opened = outcome.expect("a card entitled key verifies its own compressed member");
    let member = &opened.package().members()[0];
    assert_eq!(member.compression(), crate::package::read::COMPRESSION_ZSTD);
    assert_eq!(member.stored_len(), stored.len() as u64);
    assert_eq!(member.plain_len(), 2_147_483_648);
}

// ---------------------------------------------------------------------------
// The whole artifact.
// ---------------------------------------------------------------------------

#[test]
fn a_trusted_artifact_verifies_header_and_every_member() {
    let bytes = built_by(TRUSTED, Kind::application());
    let (outcome, source) = check(&bytes, &trusting(&[Kind::application()]));
    let opened = outcome.expect("a trusted artifact");
    assert_eq!(opened.package().header().product(), "photo-index");
    assert_eq!(opened.head(), &bytes[..HEADER_BYTES]);
    // The header, then each member once, in file order — and nothing else.
    let mut expected = vec![(0, HEADER_BYTES as u64)];
    expected.extend(
        opened
            .package()
            .members()
            .iter()
            .map(|m| (m.offset(), m.stored_len())),
    );
    assert_eq!(source.asked, expected);
}

/// **An untrusted signature reads no member**: the refusal is the header's, and
/// nothing past 4 KiB is asked of the medium.
#[test]
fn an_artifact_signed_by_a_stranger_is_refused_without_reading_a_member() {
    let bytes = built_by(STRANGER, Kind::application());
    let (outcome, source) = check(&bytes, &trusting(&[Kind::application()]));
    assert!(
        matches!(outcome, Err(Fault::Artifact(PackageError::UntrustedKey { .. }))),
        "{outcome:?}"
    );
    assert_eq!(source.asked, vec![(0, HEADER_BYTES as u64)]);
}

/// **A genuine header over altered bytes is refused**, and it is the reason the
/// door checks members rather than stopping at the signature: without it, a
/// correctly signed header grafted onto anything would be admitted.
#[test]
fn a_member_whose_bytes_were_altered_is_refused_by_its_digest() {
    let mut bytes = built_by(TRUSTED, Kind::application());
    let last = bytes.len() - 3;
    bytes[last] ^= 0x01;
    let (outcome, _) = check(&bytes, &trusting(&[Kind::application()]));
    assert!(
        matches!(
            outcome,
            Err(Fault::Artifact(PackageError::MemberDigestMismatch { index: 2, .. }))
        ),
        "{outcome:?}"
    );
}

/// A file shorter than a header is refused on its length, and **the medium is
/// not asked for bytes it does not have** — which would turn the artifact's
/// fault into the medium's.
#[test]
fn a_file_shorter_than_a_header_is_the_artifacts_fault_and_reads_nothing() {
    let bytes = vec![0u8; 100];
    let (outcome, source) = check(&bytes, &trusting(&[Kind::application()]));
    assert!(
        matches!(outcome, Err(Fault::Artifact(PackageError::TooShortForHeader { file_len: 100 }))),
        "{outcome:?}"
    );
    assert!(source.asked.is_empty(), "{:?}", source.asked);
}

/// A file cut short after a verified header is refused by the manifest's own
/// length check — the artifact's fault, before a member is read.
#[test]
fn a_file_cut_short_after_its_header_is_refused_as_the_artifacts_fault() {
    let bytes = built_by(TRUSTED, Kind::application());
    let cut = &bytes[..bytes.len() - 10];
    let (outcome, source) = check(cut, &trusting(&[Kind::application()]));
    assert!(
        matches!(outcome, Err(Fault::Artifact(PackageError::MemberOverflowsFile { .. }))),
        "{outcome:?}"
    );
    assert_eq!(source.asked, vec![(0, HEADER_BYTES as u64)]);
}

/// **A medium that fails is the medium's fault**, and says which read failed —
/// a caller that discards on refusal must be able to tell the two apart.
#[test]
fn a_medium_that_stops_answering_is_a_medium_fault_naming_the_member() {
    let bytes = built_by(TRUSTED, Kind::application());
    let trust = trusting(&[Kind::application()]);
    let mut source = Buffer::of(&bytes);
    let opened = check(&bytes, &trust).0.expect("the unbroken artifact verifies");
    let program = &opened.package().members()[1];
    let program_ends = program.offset() + program.stored_len();
    source.fail_at = Some(program_ends - 1);
    let outcome = artifact(
        &mut source,
        bytes.len() as u64,
        &mut ToyDigest::new(),
        &ToyVerifier::new(),
        &trust,
    );
    assert!(
        matches!(
            outcome,
            Err(Fault::Medium {
                reading: Reading::Member(1),
                ..
            })
        ),
        "{outcome:?}"
    );
}

/// **Nothing about a member is digested before the signature has been
/// checked** — `ADR-029` §5's order, asserted on the trace rather than trusted.
#[test]
fn no_member_is_digested_before_the_signature_is_checked() {
    let bytes = built_by(TRUSTED, Kind::application());
    let trace = Trace::new();
    let mut digest = ToyDigest::new().traced(&trace);
    let verifier = ToyVerifier::new().traced(&trace);
    // Built with an untraced digest, so the trace holds only what `artifact` did.
    let trust = trusting(&[Kind::application()]);
    let mut source = Buffer::of(&bytes);
    artifact(&mut source, bytes.len() as u64, &mut digest, &verifier, &trust)
        .expect("a trusted artifact");
    let events = trace.events();
    let verified = trace.first_verification().expect("the signature was checked");
    assert_eq!(events[verified], Event::Verified(true));
    assert!(
        !events[..verified]
            .iter()
            .any(|e| matches!(e, Event::DigestWrite(_))),
        "a digest absorbed bytes before the signature was checked: {events:?}"
    );
}

// ---------------------------------------------------------------------------
// The architecture — PLAN-Feature-Set-2026-09-12 §3.
// ---------------------------------------------------------------------------

/// **Every kind is asked**, and an artifact built for the machine's own
/// architecture is accepted whatever its kind.
#[test]
fn an_artifact_for_this_machines_architecture_is_accepted_whatever_its_kind() {
    for kind in [Kind::application(), Kind::library(), Kind::update()] {
        let bytes = built_for(TRUSTED, kind.clone(), "aarch64");
        let (opened, _) = check(&bytes, &trusting(&[kind.clone()]));
        let opened = opened.unwrap_or_else(|e| panic!("{kind}: {e:?}"));
        assert_eq!(architecture(opened.package(), "aarch64"), Ok(()), "{kind}");
    }
}

/// **The defect**: an application or a library built for another architecture
/// was admitted by the door and installed, because `Application::accepts` and
/// `Library::accepts` had no caller. A system update was the only kind compared.
#[test]
fn an_artifact_for_another_architecture_is_refused_whatever_its_kind_naming_both() {
    for kind in [Kind::application(), Kind::library(), Kind::update()] {
        let bytes = built_for(TRUSTED, kind.clone(), "armv6");
        let (opened, _) = check(&bytes, &trusting(&[kind.clone()]));
        let opened = opened.unwrap_or_else(|e| panic!("{kind}: {e:?}"));
        let refusal = architecture(opened.package(), "aarch64")
            .expect_err("an armv6 artifact on an aarch64 machine");
        let PackageError::WrongKind { expected, found } = &refusal else {
            panic!("{kind}: {refusal:?}");
        };
        assert!(expected.contains("aarch64"), "{kind}: {expected}");
        assert!(found.contains("armv6"), "{kind}: {found}");
    }
}

/// **`ADR-061` §3: architecture is not a machine's question to ask of an
/// `apk`.** Its header always carries `arch = "any"`, and `verify::architecture`
/// accepts it whatever the caller asks about — there is nothing on this
/// machine to compare it against.
#[test]
fn an_apk_is_accepted_whatever_architecture_is_asked_about() {
    let mut digest = ToyDigest::new();
    let mut signer = ToySigner::new(TRUSTED);
    let mut builder =
        Builder::new("gantry", "0.1.17", "any", Some("android"), Kind::apk()).unwrap();
    let parts: Vec<(&str, Vec<u8>)> = vec![
        ("app.apk", b"pretend-this-is-a-zip".to_vec()),
        ("apk.json", b"{}".to_vec()),
    ];
    for (name, bytes) in &parts {
        builder.add(NewMember::stored(name, 0, 0, bytes, &mut digest).unwrap()).unwrap();
    }
    let slices: Vec<&[u8]> = parts.iter().map(|(_, b)| b.as_slice()).collect();
    let bytes = builder.seal_in_memory(&mut digest, &mut signer, &slices).unwrap();
    let (opened, _) = check(&bytes, &trusting(&[Kind::apk()]));
    let opened = opened.expect("a trusted apk");
    for arch in ["aarch64", "armv6", "anything-at-all"] {
        assert_eq!(architecture(opened.package(), arch), Ok(()), "{arch}");
    }
}

/// **The compiler's word is not the machine's.** `std::env::consts::ARCH` is
/// `arm` on an `armv6` board; compared against it, a correct `armv6` artifact
/// would be refused. The token has to be the image's.
#[test]
fn an_armv6_artifact_is_refused_by_the_compilers_word_and_accepted_by_the_images() {
    let bytes = built_for(TRUSTED, Kind::application(), "armv6");
    let (opened, _) = check(&bytes, &trusting(&[Kind::application()]));
    let opened = opened.expect("a trusted artifact");
    assert!(architecture(opened.package(), "arm").is_err());
    assert_eq!(architecture(opened.package(), "armv6"), Ok(()));
}

// ---------------------------------------------------------------------------
// An application shadowing an image service — PLAN-Feature-Set-2026-09-12
// §3, `.rulings/` R-156 R10.
// ---------------------------------------------------------------------------

/// **The whole rule**: an application named exactly like one of
/// `rivet_core::service::IMAGE_SERVICE_NAMES` is refused, naming the clash.
#[test]
fn an_application_named_like_an_image_service_is_refused_naming_it() {
    for name in crate::service::IMAGE_SERVICE_NAMES {
        let bytes = built_named(TRUSTED, Kind::application(), name);
        let (opened, _) = check(&bytes, &trusting(&[Kind::application()]));
        let opened = opened.unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let refusal = not_an_image_service(opened.package())
            .expect_err(&format!("{name} is an image service"));
        assert_eq!(
            refusal,
            PackageError::NamesAnImageService {
                name: name.to_string()
            },
            "{name}"
        );
    }
}

/// **A name the network's derived services take is refused the same way**:
/// a DHCP client's or a supplicant's, for any interface, since the supervisor
/// makes those names from the document and an application holding one would
/// stop that interface's client from being defined.
#[test]
fn an_application_named_like_a_network_mechanism_service_is_refused() {
    for name in [
        crate::service::dhcp_service_name("eth0"),
        crate::service::supplicant_service_name("wlan0"),
    ] {
        let bytes = built_named(TRUSTED, Kind::application(), &name);
        let (opened, _) = check(&bytes, &trusting(&[Kind::application()]));
        let opened = opened.unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(
            not_an_image_service(opened.package()),
            Err(PackageError::NamesAnImageService { name: name.clone() }),
            "{name}"
        );
    }
}

/// **Only an application's name is ever compared, and only against the real
/// list.** A system update or a library named exactly like an image service is
/// untouched — neither kind ever becomes a shadow the supervisor could find —
/// and an application under an ordinary name is unaffected.
#[test]
fn only_an_applications_name_is_checked_and_only_against_a_real_clash() {
    for kind in [Kind::update(), Kind::library()] {
        let bytes = built_named(TRUSTED, kind.clone(), crate::service::MCPD_SERVICE_NAME);
        let (opened, _) = check(&bytes, &trusting(std::slice::from_ref(&kind)));
        let opened = opened.unwrap_or_else(|e| panic!("{kind}: {e:?}"));
        assert_eq!(not_an_image_service(opened.package()), Ok(()), "{kind}");
    }

    let bytes = built_for(TRUSTED, Kind::application(), "aarch64");
    let (opened, _) = check(&bytes, &trusting(&[Kind::application()]));
    let opened = opened.expect("a trusted application");
    assert_eq!(not_an_image_service(opened.package()), Ok(()));
}

// ---------------------------------------------------------------------------
// An application named for something this project's own machinery already
// means — `.rulings/` R-210 D21.
// ---------------------------------------------------------------------------

/// **The whole rule**: an application named exactly like one of
/// `rivet_core::service::reserved_application_names` is refused, naming the
/// clash.
#[test]
fn an_application_named_like_a_reserved_name_is_refused_naming_it() {
    for name in crate::service::reserved_application_names() {
        let bytes = built_named(TRUSTED, Kind::application(), name);
        let (opened, _) = check(&bytes, &trusting(&[Kind::application()]));
        let opened = opened.unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let refusal =
            not_a_reserved_name(opened.package()).expect_err(&format!("{name} is a reserved name"));
        assert_eq!(refusal, PackageError::NamesAReservedName { name: name.to_string() }, "{name}");
    }
}

/// **A name added to the tables this rule reads becomes reserved with no
/// second edit** — the point of D21's "iterate the tables, never restate
/// them." This reads `crate::etc::SYSTEM_IDENTITIES`, `crate::etc::SYSTEM_GROUPS`
/// and the uninstall grammar's own keywords directly, rather than through
/// `reserved_application_names`, so a future rewrite of that function that
/// drops one of its sources fails HERE rather than passing by agreeing with
/// itself.
#[test]
fn every_system_identity_and_group_and_uninstall_keyword_is_reserved() {
    for identity in crate::etc::SYSTEM_IDENTITIES {
        assert!(
            crate::service::is_reserved_application_name(identity.name),
            "{} is a system identity and must be reserved",
            identity.name
        );
    }
    for group in crate::etc::SYSTEM_GROUPS {
        assert!(
            crate::service::is_reserved_application_name(group.name),
            "{} is a system group and must be reserved",
            group.name
        );
    }
    for keyword in [crate::operation::UNINSTALL_CONFIRMATION, crate::uninstall::PURGE] {
        assert!(
            crate::service::is_reserved_application_name(keyword),
            "{keyword} is an uninstall grammar keyword and must be reserved"
        );
    }
}

/// **Only an application's name is ever compared, and only against the real
/// set.** A system update or a library named exactly like a reserved name is
/// untouched, and an application under an ordinary name is unaffected.
#[test]
fn only_an_applications_name_is_checked_against_the_reserved_set() {
    for kind in [Kind::update(), Kind::library()] {
        let bytes = built_named(TRUSTED, kind.clone(), "root");
        let (opened, _) = check(&bytes, &trusting(std::slice::from_ref(&kind)));
        let opened = opened.unwrap_or_else(|e| panic!("{kind}: {e:?}"));
        assert_eq!(not_a_reserved_name(opened.package()), Ok(()), "{kind}");
    }

    let bytes = built_for(TRUSTED, Kind::application(), "aarch64");
    let (opened, _) = check(&bytes, &trusting(&[Kind::application()]));
    let opened = opened.expect("a trusted application");
    assert_eq!(not_a_reserved_name(opened.package()), Ok(()));
}

/// [`header`] alone reads the header alone, so a caller with cheap refusals to
/// make first — the installer's slot and fit checks — pays for no member.
#[test]
fn the_header_on_its_own_reads_no_member() {
    let bytes = built_by(TRUSTED, Kind::application());
    let mut source = Buffer::of(&bytes);
    let opened = header(
        &mut source,
        bytes.len() as u64,
        &mut ToyDigest::new(),
        &ToyVerifier::new(),
        &trusting(&[Kind::application()]),
    )
    .expect("a trusted header");
    assert_eq!(source.asked, vec![(0, HEADER_BYTES as u64)]);
    // And every member can then be taken, one at a time or all at once.
    let one = member(&mut source, opened.package(), 0, &mut ToyDigest::new()).expect("member 0");
    assert_eq!(one.member().index(), 0);
    assert!(matches!(
        member(&mut source, opened.package(), 9, &mut ToyDigest::new()),
        Err(Fault::Artifact(PackageError::NoSuchMember { index: 9, .. }))
    ));
}
