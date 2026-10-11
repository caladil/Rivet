//! The `application` kind's suite.
//!
//! **Provenance (`AGENTS.md` §4): every vector here is hand-composed**, and that
//! is the ceiling on what it proves. There is no `.rvt` from another
//! implementation to test against because there is no other implementation.
//!
//! What this suite is *for* is the half a round trip cannot reach. The manifest
//! has a writer and a reader in the same file, so
//! [`manifest_text_is_exactly_this`] pins the **bytes** rather than the
//! agreement — a change to either side that keeps them agreeing with each other
//! still fails it, which is the failure a round trip cannot see. Everything
//! below it drives text and headers the emitter did not produce.
//!
//! The real algorithms are not here and cannot be: `rivet-core` has an empty
//! dependency table (`ADR-014` §3). `cmd/cargo-rivet`'s suite drives the same
//! shapes through BLAKE3 and Ed25519.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::package::port::MaySign;
use crate::package::port::TrustSet;
use crate::package::read::{Package, open};
use crate::package::testing::{ToyDigest, ToyKey, ToySigner, ToyVerifier};
use crate::package::write::{Builder, NewMember};

const SIGNING_KEY: ToyKey = ToyKey(0x4d);

/// Trusted for **both** kinds, deliberately.
///
/// These tests exercise `Application::open` — the *interpretation* layer, which
/// refuses a payload that is not an application (`PackageError::WrongKind`).
/// `ADR-028` §5.5's entitlement check sits earlier and refuses a *key* that may
/// not sign a kind (`KindNotPermitted`), and one of the tests below deliberately
/// builds an `update` payload to watch the interpretation reject it. A trust set
/// limited to `application` would refuse that payload one layer too early and
/// the test would pass for the wrong reason.
///
/// The entitlement check has its own tests, against their own trust sets, in
/// `crate::package::tests`.
fn trust() -> TrustSet {
    let mut digest = ToyDigest::new();
    let mut trust = TrustSet::new();
    trust.trust(
        &mut digest,
        SIGNING_KEY.public(),
        "test-apps",
        MaySign::these(&[Kind::application(), Kind::update()]),
    );
    trust
}

/// Build a package out of `(name, class, variant, bytes)` and read it back
/// through the real verifier. The toy crypto proves ordering and refusals only —
/// see [`crate::package::testing`].
fn packaged(
    product: &str,
    version: &str,
    arch: &str,
    platform: Option<&str>,
    kind: Kind,
    members: &[(&str, u8, u8, Vec<u8>)],
) -> Result<Package, PackageError> {
    let mut digest = ToyDigest::new();
    let mut signer = ToySigner::new(SIGNING_KEY);
    let mut builder = Builder::new(product, version, arch, platform, kind)?;
    for (name, class, variant, bytes) in members {
        builder.add(NewMember::stored(
            name,
            *class,
            *variant,
            bytes,
            &mut digest,
        )?)?;
    }
    let slices: Vec<&[u8]> = members.iter().map(|(_, _, _, b)| b.as_slice()).collect();
    let file = builder.seal_in_memory(&mut digest, &mut signer, &slices)?;

    let mut digest = ToyDigest::new();
    let verifier = ToyVerifier::new();
    open(&file, file.len() as u64, &mut digest, &verifier, &trust())
}

fn manifest_bytes() -> Vec<u8> {
    sample_manifest().render().into_bytes()
}

fn sample_manifest() -> Manifest {
    Manifest::new(ProgramKind::Daemon)
        .with_restart(Restart::OnFailure)
        .with_ready(Ready::Listening)
        .with_network(Network::Group("media".to_string()))
        .with_state(StateGrant::Private)
        .with_memory(Some(Memory::parse("128M").unwrap()))
        .with_listen(vec![Socket::Tcp(8080), Socket::Unix("api".to_string())])
        .unwrap()
        .with_provides(vec![Endpoint::parse("provides", "socket:api").unwrap()])
        .unwrap()
        .with_connects(vec![
            Endpoint::parse("connects", "photo-index:api").unwrap(),
        ])
        .unwrap()
        // Shared Rivets, 2026-08-31. A declared closure of one, so the pinned
        // bytes below cover a non-empty `library` rather than only the empty
        // spelling — an empty list would have passed whatever the element rules
        // turned out to be.
        .with_library(vec![
            crate::package::library::Reference::parse(&format!("libgreet-1.0.0-{}", "ab".repeat(32)))
                .unwrap(),
        ])
        .unwrap()
        // `ADR-015` §6.2, 2026-08-31. Both classes, so the pinned bytes below
        // cover a non-empty list AND its canonical order.
        .with_devices(vec![Device::Input, Device::Display])
}

/// The members of a minimal, well-formed application.
fn well_formed() -> Vec<(&'static str, u8, u8, Vec<u8>)> {
    vec![
        (
            MANIFEST_NAME,
            TargetClass::Manifest.as_byte(),
            VARIANT_NONE,
            manifest_bytes(),
        ),
        (
            "hello",
            TargetClass::Program.as_byte(),
            VARIANT_NONE,
            b"\x7fELF not really".to_vec(),
        ),
        (
            "banner.txt",
            TargetClass::Asset.as_byte(),
            VARIANT_NONE,
            b"hello\n".to_vec(),
        ),
    ]
}

fn app_package() -> Package {
    packaged(
        "hello",
        "0.1.0",
        "aarch64",
        None,
        Kind::application(),
        &well_formed(),
    )
    .unwrap()
}

// ---------------------------------------------------------------------------
// The manifest's bytes.
// ---------------------------------------------------------------------------

/// **The bytes, pinned.** A round trip cannot see a change that moves both
/// sides; this can.
#[test]
fn manifest_text_is_exactly_this() {
    let text = sample_manifest().render();
    assert_eq!(
        text,
        "# Rivet application manifest — ADR-015 §3.2.\n\
         # Emitted by `cargo rivet package`; not written by hand.\n\
         kind = \"daemon\"\n\
         restart = \"on-failure\"\n\
         ready = \"listening\"\n\
         network = \"group:media\"\n\
         state = \"private\"\n\
         memory = \"128M\"\n\
         listen = [\"tcp:8080\", \"unix:api\"]\n\
         provides = [\"socket:api\"]\n\
         connects = [\"photo-index:api\"]\n\
         library = [\"libgreet-1.0.0-abababababababababababababababababababababababababababababababab\"]\n\
         devices = [\"display\", \"input\"]\n"
    );
}

#[test]
fn a_manifest_survives_its_own_emitter() {
    let manifest = sample_manifest();
    assert_eq!(Manifest::parse(manifest.render().as_bytes()), Ok(manifest));
}

/// `memory` is the one key that may be absent, and the empty lists are still
/// written, so a reader never depends on a default table.
#[test]
fn the_minimal_manifest_still_spells_every_field() {
    let manifest = Manifest::new(ProgramKind::Oneshot);
    let text = manifest.render();
    assert!(!text.contains("memory"));
    for key in [
        "kind", "restart", "ready", "network", "state", "listen", "provides", "connects",
    ] {
        assert!(text.contains(key), "{key} is not in {text}");
    }
    assert_eq!(Manifest::parse(text.as_bytes()), Ok(manifest));
}

#[test]
fn every_enumerated_value_survives_the_round_trip() {
    for kind in [
        ProgramKind::Daemon,
        ProgramKind::Oneshot,
        ProgramKind::SocketActivated,
    ] {
        for restart in [Restart::Always, Restart::OnFailure, Restart::Never] {
            for ready in [Ready::None, Ready::Notify, Ready::Listening] {
                for network in [Network::None, Network::Egress, Network::Group("m".into())] {
                    for state in [StateGrant::None, StateGrant::Private] {
                        let manifest = Manifest::new(kind)
                            .with_restart(restart)
                            .with_ready(ready)
                            .with_network(network.clone())
                            .with_state(state);
                        assert_eq!(Manifest::parse(manifest.render().as_bytes()), Ok(manifest));
                    }
                }
            }
        }
    }
}

#[test]
fn a_key_this_build_does_not_define_is_refused() {
    let text = "kind = \"daemon\"\nsyscalls = \"strict\"\n";
    assert_eq!(
        Manifest::parse(text.as_bytes()),
        Err(ManifestFault::UnknownKey)
    );
}

#[test]
fn a_repeated_or_reordered_key_is_refused() {
    let twice = "kind = \"daemon\"\nkind = \"oneshot\"\n";
    assert_eq!(
        Manifest::parse(twice.as_bytes()),
        Err(ManifestFault::OutOfOrder { key: "kind" })
    );
    let backwards = "restart = \"never\"\nkind = \"daemon\"\n";
    assert_eq!(
        Manifest::parse(backwards.as_bytes()),
        Err(ManifestFault::OutOfOrder { key: "kind" })
    );
}

#[test]
fn every_required_key_is_required() {
    let full = sample_manifest().render();
    for key in [
        "kind", "restart", "ready", "network", "state", "listen", "provides", "connects",
    ] {
        let without: String = full
            .lines()
            .filter(|line| !line.starts_with(&format!("{key} = ")))
            .map(|line| format!("{line}\n"))
            .collect();
        assert_eq!(
            Manifest::parse(without.as_bytes()),
            Err(ManifestFault::MissingKey { key }),
            "dropping {key} was accepted"
        );
    }
    // And `memory` is the exception, so dropping it must NOT fail — otherwise
    // the loop above would be asserting a rule the type does not have.
    let without_memory: String = full
        .lines()
        .filter(|line| !line.starts_with("memory = "))
        .map(|line| format!("{line}\n"))
        .collect();
    assert!(Manifest::parse(without_memory.as_bytes()).is_ok());
}

#[test]
fn a_value_outside_the_enumeration_is_refused() {
    for (key, line) in [
        ("kind", "kind = \"service\"\n"),
        ("restart", "restart = \"sometimes\"\n"),
        ("ready", "ready = \"soon\"\n"),
        ("network", "network = \"all\"\n"),
        ("state", "state = \"shared\"\n"),
    ] {
        let text = sample_manifest()
            .render()
            .lines()
            .map(|existing| {
                if existing.starts_with(&format!("{key} = ")) {
                    line.to_string()
                } else {
                    format!("{existing}\n")
                }
            })
            .collect::<String>();
        assert_eq!(
            Manifest::parse(text.as_bytes()),
            Err(ManifestFault::BadValue { key }),
            "{key} accepted a word it does not define"
        );
    }
}

#[test]
fn a_group_with_no_name_is_not_a_group() {
    assert_eq!(
        parse_network("group:"),
        Err(ManifestFault::BadValue { key: "network" })
    );
    assert_eq!(
        parse_network("group"),
        Err(ManifestFault::BadValue { key: "network" })
    );
    assert_eq!(
        parse_network("group:a:b"),
        Err(ManifestFault::BadValue { key: "network" })
    );
    assert_eq!(
        parse_network("group:media"),
        Ok(Network::Group("media".into()))
    );
}

#[test]
fn a_port_that_cannot_be_passed_as_a_descriptor_is_refused() {
    assert_eq!(
        parse_socket("tcp:0"),
        Err(ManifestFault::BadValue { key: "listen" })
    );
    assert_eq!(
        parse_socket("tcp:65536"),
        Err(ManifestFault::BadValue { key: "listen" })
    );
    assert_eq!(
        parse_socket("tcp:08080"),
        Err(ManifestFault::BadValue { key: "listen" })
    );
    assert_eq!(
        parse_socket("tcp:-1"),
        Err(ManifestFault::BadValue { key: "listen" })
    );
    assert_eq!(
        parse_socket("unix:a/b"),
        Err(ManifestFault::BadValue { key: "listen" })
    );
    assert_eq!(parse_socket("tcp:65535"), Ok(Socket::Tcp(65535)));
    assert_eq!(parse_socket("tcp:1"), Ok(Socket::Tcp(1)));
}

/// **`udp:<port>` obeys `tcp:<port>`'s rule exactly** — `PLAN.md` §150.1 added
/// the variant so a DNS service can declare `listen = ["udp:53"]`, and a port
/// this build cannot pass as a descriptor is refused by name rather than
/// accepted into a signed artifact. `udp:53` itself is the case that mattered:
/// it was refused before this change (`ADR-015` §5.2's descriptor handoff had
/// no UDP variant to produce), and a manifest that could not say the one port
/// DNS actually needs is the gap `PLAN.md` §150.1 found.
#[test]
fn a_udp_port_follows_the_same_rule_as_tcp_and_is_refused_the_same_way() {
    assert_eq!(
        parse_socket("udp:0"),
        Err(ManifestFault::BadValue { key: "listen" })
    );
    assert_eq!(
        parse_socket("udp:65536"),
        Err(ManifestFault::BadValue { key: "listen" })
    );
    assert_eq!(
        parse_socket("udp:065536"),
        Err(ManifestFault::BadValue { key: "listen" })
    );
    assert_eq!(
        parse_socket("udp:-1"),
        Err(ManifestFault::BadValue { key: "listen" })
    );
    assert_eq!(parse_socket("udp:53"), Ok(Socket::Udp(53)));
    assert_eq!(parse_socket("udp:65535"), Ok(Socket::Udp(65535)));
    assert_eq!(parse_socket("udp:1"), Ok(Socket::Udp(1)));
}

/// **The manifest round-trips a UDP socket** — the failure a round trip alone
/// cannot catch is a writer and a reader that agree on the wrong thing, so
/// this also pins the rendered text rather than only asserting `parse(render())
/// == original`.
#[test]
fn a_udp_socket_round_trips_through_the_canonical_text() {
    let manifest = Manifest::new(ProgramKind::Daemon)
        .with_listen(vec![Socket::Udp(53)])
        .expect("one socket is under the limit");
    let text = manifest.render();
    assert!(
        text.contains("listen = [\"udp:53\"]"),
        "udp:53 must render exactly like tcp:<port> does: {text}"
    );
    let back = Manifest::parse(text.as_bytes()).expect("round trip");
    assert_eq!(back, manifest);
}

/// The shape is checked and the *number* is deliberately not computed —
/// no formal ruling settles whether `128M` is binary or decimal, though
/// `rivet-config` has read it as binary since 2026-08-22.
#[test]
fn memory_is_a_validated_string_and_never_a_count() {
    assert_eq!(
        Memory::parse("128M").map(|m| m.as_str().to_string()),
        Ok("128M".to_string())
    );
    assert_eq!(
        Memory::parse("512").map(|m| m.as_str().to_string()),
        Ok("512".to_string())
    );
    assert!(Memory::parse("128MB").is_err());
    assert!(Memory::parse("M").is_err());
    assert!(Memory::parse("").is_err());
    assert!(Memory::parse("0128M").is_err());
    assert!(Memory::parse("-1M").is_err());
    // There is no `as_bytes`, `as_u64` or `into()` on Memory, and that absence
    // is the design. If one is ever added, the binary-or-decimal question
    // must be ruled first.
}

/// [`memory_is_a_validated_string_and_never_a_count`]'s footing, for [`Cpu`]:
/// the shape is checked and the arithmetic is deliberately elsewhere
/// (`rivet_core::service::CpuLimit`), which `rivet-core`'s empty
/// `[dependencies]` table does not stop this crate from owning — unlike
/// `Memory`, there is no ruling left open here, only a crate boundary.
#[test]
fn cpu_is_a_validated_string_and_never_a_count() {
    assert_eq!(
        Cpu::parse("0.5").map(|c| c.as_str().to_string()),
        Ok("0.5".to_string())
    );
    assert_eq!(
        Cpu::parse("2").map(|c| c.as_str().to_string()),
        Ok("2".to_string())
    );
    assert!(Cpu::parse("0.50").is_err(), "a trailing zero is a second spelling");
    assert!(Cpu::parse("2.0").is_err(), "a trailing zero is a second spelling");
    assert!(Cpu::parse("").is_err());
    assert!(Cpu::parse("02").is_err());
    assert!(Cpu::parse("-1").is_err());
    assert!(Cpu::parse("0.123456").is_err(), "six decimal digits: PERIOD_US cannot hold it");
    // There is no `as_quota_us` or `into()` on `Cpu`, mirroring `Memory`: the
    // conversion is `rivet_core::service::CpuLimit::of`'s alone.
}

/// **The manifest round-trips a CPU request** — on `a_udp_socket_round_
/// trips_through_the_canonical_text`'s footing: this also pins the rendered
/// text, because a round trip alone cannot catch a writer and a reader that
/// agree on the wrong thing.
#[test]
fn a_cpu_request_round_trips_through_the_canonical_text() {
    let manifest = Manifest::new(ProgramKind::Daemon)
        .with_cpu(Some(Cpu::parse("0.5").expect("a valid fixture")));
    let text = manifest.render();
    assert!(
        text.contains("cpu = \"0.5\"\n"),
        "cpu must render as a plain quoted decimal: {text}"
    );
    // **Last line**, after every other optional key — `KEYS`' comment on
    // `cpu` says why it sits at the end rather than beside `memory`.
    assert!(text.trim_end().ends_with("cpu = \"0.5\""), "{text}");
    let back = Manifest::parse(text.as_bytes()).expect("round trip");
    assert_eq!(back, manifest);
}

/// **`.rulings/` R-210 D22 — PINNED so nobody later tightens this parser.**
/// `cargo rivet package` refuses a below-minimum `cpu` now, but an
/// application already installed with one — signed before this ruling, or by
/// an older `cargo rivet` — must have its manifest keep reading back exactly
/// as installed, both from the builder and from bytes on a card.
#[test]
fn a_manifest_cpu_request_under_the_kernel_minimum_still_parses() {
    let cpu = Cpu::parse("0.005").expect("Cpu::parse itself is unchanged by D22");
    assert!(
        crate::service::CpuLimit::of(&cpu).is_some_and(crate::service::CpuLimit::is_below_kernel_minimum),
        "0.005 must value below CpuLimit::KERNEL_MIN_QUOTA_US"
    );

    let manifest = Manifest::new(ProgramKind::Daemon).with_cpu(Some(cpu));
    let text = manifest.render();
    let back = Manifest::parse(text.as_bytes()).expect("a below-minimum cpu request must still parse");
    assert_eq!(back, manifest);
}

/// `the_minimal_manifest_still_spells_every_field`'s exception, for `cpu`:
/// absent is the ordinary answer and nothing writes it.
#[test]
fn a_manifest_with_no_cpu_request_does_not_mention_it() {
    let manifest = Manifest::new(ProgramKind::Oneshot);
    assert!(!manifest.render().contains("cpu"));
}

#[test]
fn text_the_emitter_could_not_have_produced_is_refused() {
    // No `=` with spaces around it.
    assert_eq!(
        Manifest::parse(b"kind=\"daemon\"\n"),
        Err(ManifestFault::NotKeyValue)
    );
    // Unquoted.
    assert_eq!(
        Manifest::parse(b"kind = daemon\n"),
        Err(ManifestFault::BadValue { key: "kind" })
    );
    // An escape the emitter has no way to write.
    assert_eq!(
        Manifest::parse(b"kind = \"dae\\mon\"\n"),
        Err(ManifestFault::BadCharacter)
    );
    // Not UTF-8 at all.
    assert_eq!(Manifest::parse(&[0xff, 0xfe]), Err(ManifestFault::NotText));
    // Longer than the bound.
    let huge = vec![b'#'; MAX_MANIFEST_BYTES + 1];
    assert_eq!(Manifest::parse(&huge), Err(ManifestFault::TooLarge));
    // A value longer than a value may be.
    let long = format!("kind = \"{}\"\n", "d".repeat(MAX_VALUE + 1));
    assert_eq!(
        Manifest::parse(long.as_bytes()),
        Err(ManifestFault::ValueTooLong)
    );
}

#[test]
fn a_list_longer_than_the_bound_is_refused_at_both_ends() {
    let too_many: Vec<Socket> = (1..=(MAX_LIST as u16 + 1)).map(Socket::Tcp).collect();
    assert_eq!(
        Manifest::new(ProgramKind::Daemon).with_listen(too_many),
        Err(ManifestFault::ListTooLong { key: "listen" })
    );

    let elements: Vec<String> = (1..=(MAX_LIST + 1))
        .map(|n| format!("\"tcp:{n}\""))
        .collect();
    let text = format!(
        "kind = \"daemon\"\nrestart = \"never\"\nready = \"none\"\nnetwork = \"none\"\n\
         state = \"none\"\nlisten = [{}]\nprovides = []\nconnects = []\n",
        elements.join(", ")
    );
    assert_eq!(
        Manifest::parse(text.as_bytes()),
        Err(ManifestFault::ListTooLong { key: "listen" })
    );
}

/// `ADR-015` §6.1: *"an unenforced declaration must be visible"*. ~~Today the
/// answer is "all of them"~~ — **not since 2026-08-31**, and this test is what
/// makes the list shrink by deletion rather than by somebody remembering.
///
/// **`listen` was the first key to leave it**, then `provides` and
/// `connects` the same day. `ADR-015` §5.2's descriptor
/// handoff is built, so a manifest that declares a socket gets one — see
/// `cmd/rivet-sysd`'s `Kernel::open_listeners`. Both halves are asserted: the
/// keys that are still only declarations, and the one that is not, because a
/// test that only checked the first would pass on a build that had quietly
/// stopped enforcing anything.
#[test]
fn what_is_still_only_declared_and_what_is_not() {
    let unenforced = sample_manifest().unenforced();
    for key in ["ready", "network", "memory"] {
        assert!(unenforced.contains(&key), "{key} is claimed to be enforced");
    }
    // **`state` left the list on 2026-09-04** — the fourth deletion, and the
    // first about a kernel mechanism rather than about plumbing. `ADR-042`'s
    // Landlock ruleset grants write on `RIVET_APP_DATA` and on nothing else,
    // so `state = "private"` is now a sentence the kernel keeps: a write
    // anywhere else is `EACCES`, including to `/tmp`, which is mode 1777 and
    // which Unix permissions never separated two applications on.
    assert!(
        !unenforced.contains(&"state"),
        "`state` IS enforced since 2026-09-04: the ruleset grants write on RIVET_APP_DATA and \
         nowhere else, so reporting it as a declaration that does nothing would tell an author \
         to write elsewhere and be refused"
    );
    // **`restart` left the list on 2026-09-03** — the third deletion, and the
    // first one found by somebody reading rather than by somebody building.
    // `cmd/rivet-sysd/src/applications.rs` maps every variant into the service
    // definition and `decide` acts on it, and it has for as long as
    // applications have been started.
    assert!(
        !unenforced.contains(&"restart"),
        "`restart` IS enforced: the supervisor maps it into RestartPolicy and decides on it"
    );
    // **The narrowing is deliberate and is not what this asserts away.**
    // `provides`/`connects` are enforced as a chown to ONE consumer, not as
    // the ADR's bind mount into a mount namespace — there are none here.
    // They are off the list because the operator's question, *is this
    // declaration doing anything*, is now answered yes; the limit is stated
    // by the supervisor's own ENFORCED record and by
    // `cmd/rivet-sysd/src/endpoints.rs`.
    for key in ["listen", "provides", "connects"] {
        assert!(
            !unenforced.contains(&key),
            "`{key}` IS enforced since 2026-08-31 and reporting it as unenforced would send \
             an application author to build their own, which is now the wrong advice"
        );
    }
}

/// **`network = "none"` is reported as unenforced, and it is the value that
/// most has to be.**
///
/// `PLAN.md` §123.2 item 3. This function used to report `network` only when
/// the value was NOT `none` — so the one value `ADR-015` §5.1 defines as "no
/// interfaces at all", and therefore the one an operator would trust, was the
/// one that said nothing. There is no network namespace on this build, so
/// `none` confines exactly as much as `egress`: nothing.
///
/// Asserted for every value rather than for `none` alone, because the defect
/// was a special case and a test with its own special case could not catch the
/// next one.
#[test]
fn every_network_value_is_reported_as_unenforced_including_none() {
    for value in ["none", "egress", "lan", "any"] {
        let text = format!(
            "kind = \"daemon\"\nrestart = \"never\"\nready = \"none\"\nnetwork = \"{value}\"\n\
             state = \"none\"\nlisten = []\nprovides = []\nconnects = []\n"
        );
        let Ok(manifest) = Manifest::parse(text.as_bytes()) else {
            continue; // a value this build does not have is not this test's subject
        };
        assert!(
            manifest.unenforced().contains(&"network"),
            "network = \"{value}\" is claimed to be enforced, and nothing enforces any of them"
        );
    }
}

/// An endpoint name reaches a filesystem path, so a manifest may not put a
/// separator in one — `Endpoint::parse`'s note.
///
/// Asserted from the PARSER rather than from the path builder: the rule is
/// that a value is validated where it is parsed, so a test that only checked
/// the path would pass on a build that had moved the check to the wrong place.
#[test]
fn an_endpoint_may_not_carry_a_path_separator() {
    use crate::package::application::Endpoint;
    for bad in [
        "socket:../../system/bin/rivet",
        "socket:a/b",
        "some/app:api",
        "socket:/etc",
    ] {
        assert_eq!(
            Endpoint::parse("provides", bad),
            Err(ManifestFault::BadValue { key: "provides" }),
            "{bad} was accepted, and it names a file outside the listen directory"
        );
    }
    assert!(Endpoint::parse("provides", "socket:kiosk.sock").is_ok());
    assert!(Endpoint::parse("connects", "aura-controller:kiosk.sock").is_ok());
}

/// **And refusing a separator does not refuse `.` or `..`**, which is the half
/// the test above reads as covering and does not.
///
/// Audit of 2026-09-03, register R-30. `socket:..` holds no slash and names
/// `LISTEN_DIR`'s parent, so the supervisor would create, unlink and **chown
/// `/run/rivet` itself** to a consumer; `socket:.` names the directory. The
/// same hole was in `parse_socket`'s `unix:` arm, which the audit did not name
/// — one rule now answers for both, so neither can drift from the other.
#[test]
fn an_endpoint_name_may_not_be_a_relative_path_component() {
    use crate::package::application::{Endpoint, Socket, parse_socket};
    for bad in ["socket:..", "socket:.", "..:api", ".:api", "socket:\0"] {
        assert_eq!(
            Endpoint::parse("provides", bad),
            Err(ManifestFault::BadValue { key: "provides" }),
            "{bad:?} was accepted, and it names the listen directory or its parent rather \
             than a socket inside it"
        );
    }
    // A name may still CONTAIN a dot — `kiosk.sock` is the shipped case, and a
    // rule that refused those would break every manifest in the tree.
    assert!(Endpoint::parse("provides", "socket:a.b").is_ok());
    assert!(Endpoint::parse("provides", "socket:...").is_ok());

    for bad in ["unix:..", "unix:.", "unix:a/b", "unix:"] {
        assert_eq!(
            parse_socket(bad),
            Err(ManifestFault::BadValue { key: "listen" }),
            "{bad:?} was accepted by `listen`, which reaches the same path builder"
        );
    }
    assert_eq!(
        parse_socket("unix:api.sock"),
        Ok(Socket::Unix("api.sock".to_string()))
    );
}

// ---------------------------------------------------------------------------
// The package, read as an application.
// ---------------------------------------------------------------------------

#[test]
fn a_well_formed_application_opens_and_names_itself_from_the_header() {
    let package = app_package();
    let app = Application::open(&package).unwrap();
    assert_eq!(app.name(), "hello");
    assert_eq!(app.version(), "0.1.0");
    assert_eq!(app.manifest_member().name(), MANIFEST_NAME);
    assert_eq!(app.manifest_member().index(), 0);
    assert_eq!(app.program_member().name(), "hello");
    let assets: Vec<&str> = app
        .assets()
        .map(crate::package::read::Member::name)
        .collect();
    assert_eq!(assets, vec!["banner.txt"]);
    assert_eq!(app.accepts("aarch64"), Ok(()));
    assert!(app.accepts("x86_64").is_err());
}

/// The manifest is parsed only from bytes that carry proof of verification, and
/// the round trip closes through the reader rather than through the emitter.
#[test]
fn the_manifest_is_read_back_from_the_member_the_writer_put_there() {
    let package = app_package();
    let app = Application::open(&package).unwrap();
    let bytes = manifest_bytes();
    let mut digest = ToyDigest::new();
    let verified = package
        .verify_member(app.manifest_member().index(), &mut digest, &bytes)
        .unwrap();
    assert_eq!(app.read_manifest(verified, &bytes), Ok(sample_manifest()));
}

/// **The install door refuses a signed package asking for the machine's own
/// sign-in path** — `ADR-046` §4. The manifest member verifies; what it asks
/// for is refused when it is read, before anything is installed.
#[test]
fn the_install_door_refuses_a_package_asking_for_a_reserved_path() {
    for reserved in RESERVED_PROXY_PATHS {
        // Hand-composed: the emitter refuses this path, so the bytes are the
        // canonical text with the value replaced, as a foreign tool could write.
        let claimed = sample_manifest()
            .with_proxy_path(Some("aura".to_string()))
            .expect("valid")
            .render()
            .replace("proxy-path = \"aura\"", &format!("proxy-path = \"{reserved}\""))
            .into_bytes();
        let mut members = well_formed();
        members[0].3 = claimed.clone();
        let package = packaged("hello", "0.1.0", "aarch64", None, Kind::application(), &members)
            .expect("a package");
        let app = Application::open(&package).expect("it opens; the refusal is the manifest's");
        let mut digest = ToyDigest::new();
        let verified = package
            .verify_member(app.manifest_member().index(), &mut digest, &claimed)
            .expect("the member verifies");
        assert_eq!(
            app.read_manifest(verified, &claimed),
            Err(PackageError::MalformedManifest {
                fault: ManifestFault::ReservedProxyPath
            }),
            "{reserved:?}"
        );
    }
}

/// A [`crate::package::read::VerifiedMember`] proves *some* member was checked.
/// This is the check that it was the right one.
#[test]
fn a_verified_member_for_the_wrong_member_is_refused() {
    let package = app_package();
    let app = Application::open(&package).unwrap();
    let program = b"\x7fELF not really".to_vec();
    let mut digest = ToyDigest::new();
    let verified = package
        .verify_member(app.program_member().index(), &mut digest, &program)
        .unwrap();
    assert_eq!(
        app.read_manifest(verified, &manifest_bytes()),
        Err(PackageError::WrongMemberVerified {
            expected: 0,
            found: 1
        })
    );
}

/// `ADR-015` §4.1's argument, as a test: *"a legitimately signed application
/// could have its manifest rewritten to ask for `network = "egress"` … and the
/// binary's signature would still verify."* Here it cannot, in both the shapes
/// a rewrite takes.
#[test]
fn a_rewritten_manifest_cannot_be_parsed_because_it_cannot_be_verified() {
    let package = app_package();
    let app = Application::open(&package).unwrap();
    let at = app.manifest_member().index();

    // 1. Same length, so only the digest can catch it. `socket:api` becomes
    //    `socket:ALL` — ten bytes either way.
    let mut edited = manifest_bytes();
    let offset = String::from_utf8(edited.clone())
        .unwrap()
        .find("socket:api")
        .unwrap();
    edited[offset + 7..offset + 10].copy_from_slice(b"ALL");
    let mut digest = ToyDigest::new();
    assert!(matches!(
        package.verify_member(at, &mut digest, &edited),
        Err(PackageError::MemberDigestMismatch { index: 0, .. })
    ));

    // 2. A different length — the widening §4.1 actually names. Caught one step
    //    earlier still, on the count, before a byte of it is compared.
    let widened = sample_manifest()
        .with_network(Network::Egress)
        .render()
        .into_bytes();
    assert_ne!(widened.len(), manifest_bytes().len());
    let mut digest = ToyDigest::new();
    assert!(matches!(
        package.verify_member(at, &mut digest, &widened),
        Err(PackageError::MemberShort { index: 0, .. })
    ));

    // And the honest converse: the untouched bytes still verify, so neither
    // refusal above is the fixture being broken.
    let mut digest = ToyDigest::new();
    assert!(
        package
            .verify_member(at, &mut digest, &manifest_bytes())
            .is_ok()
    );
}

#[test]
fn an_update_payload_is_not_an_application() {
    let package = packaged(
        "rivet",
        "0.2.0",
        "aarch64",
        Some("rpi4"),
        Kind::update(),
        &well_formed(),
    )
    .unwrap();
    assert!(matches!(
        Application::open(&package),
        Err(PackageError::WrongKind { .. })
    ));
}

/// The maintainer's 2026-08-24 ruling, enforced rather than documented.
#[test]
fn an_application_that_names_a_platform_is_refused() {
    let package = packaged(
        "hello",
        "0.1.0",
        "aarch64",
        Some("rpi4"),
        Kind::application(),
        &well_formed(),
    )
    .unwrap();
    assert_eq!(
        Application::open(&package),
        Err(PackageError::UnexpectedPlatform {
            found: "rpi4".to_string()
        })
    );
}

#[test]
fn the_manifest_must_be_member_zero_and_must_be_named_so() {
    let mut members = well_formed();
    members.swap(0, 1);
    let package = packaged(
        "hello",
        "0.1.0",
        "aarch64",
        None,
        Kind::application(),
        &members,
    )
    .unwrap();
    assert_eq!(
        Application::open(&package),
        Err(PackageError::ManifestNotFirst { index: 1 })
    );

    let mut members = well_formed();
    members[0].0 = "app.toml";
    let package = packaged(
        "hello",
        "0.1.0",
        "aarch64",
        None,
        Kind::application(),
        &members,
    )
    .unwrap();
    assert_eq!(
        Application::open(&package),
        Err(PackageError::ManifestMisnamed {
            name: "app.toml".to_string()
        })
    );
}

#[test]
fn exactly_one_manifest_and_exactly_one_program() {
    // None of either.
    let package = packaged(
        "hello",
        "0.1.0",
        "aarch64",
        None,
        Kind::application(),
        &[(
            "banner.txt",
            TargetClass::Asset.as_byte(),
            VARIANT_NONE,
            b"x".to_vec(),
        )],
    )
    .unwrap();
    assert_eq!(
        Application::open(&package),
        Err(PackageError::ManifestMemberCount { found: 0 })
    );

    // Two programs: one static binary, and no `exec` field to choose between
    // them (`ADR-015` §3.2).
    let mut members = well_formed();
    members.push((
        "hello2",
        TargetClass::Program.as_byte(),
        VARIANT_NONE,
        b"\x7fELF".to_vec(),
    ));
    let package = packaged(
        "hello",
        "0.1.0",
        "aarch64",
        None,
        Kind::application(),
        &members,
    )
    .unwrap();
    assert_eq!(
        Application::open(&package),
        Err(PackageError::ProgramMemberCount { found: 2 })
    );

    // A manifest and no program.
    let members = vec![well_formed().swap_remove(0)];
    let package = packaged(
        "hello",
        "0.1.0",
        "aarch64",
        None,
        Kind::application(),
        &members,
    )
    .unwrap();
    assert_eq!(
        Application::open(&package),
        Err(PackageError::ProgramMemberCount { found: 0 })
    );
}

/// The variant byte is half of a member's identity. An application has no A/B,
/// so a non-zero value is a builder with a different idea of the format.
#[test]
fn an_application_member_may_not_be_variant() {
    let mut members = well_formed();
    members[2].2 = 1;
    let package = packaged(
        "hello",
        "0.1.0",
        "aarch64",
        None,
        Kind::application(),
        &members,
    )
    .unwrap();
    assert_eq!(
        Application::open(&package),
        Err(PackageError::UnknownVariant { index: 2, found: 1 })
    );
}

#[test]
fn a_target_class_this_kind_does_not_define_is_refused() {
    let mut members = well_formed();
    members[2].1 = 7;
    let package = packaged(
        "hello",
        "0.1.0",
        "aarch64",
        None,
        Kind::application(),
        &members,
    )
    .unwrap();
    assert_eq!(
        Application::open(&package),
        Err(PackageError::UnknownTargetClass { index: 2, found: 7 })
    );
}

/// `ADR-015` §5.4's own worked example fits now, and that is the assertion.
///
/// **This test used to assert the opposite.** `product` was 16 bytes,
/// `metrics-collector` is 17 characters, and the ADR's own example could not be
/// packaged — refused rather than truncated, which was right, but the refusal
/// was the format's fault rather than the name's. The maintainer ruled on
/// 2026-08-24 to grow the field to 32 *while it was still free to do so*: no
/// image carried a trust store, so nothing had ever verified an `.rvt` on a
/// Rivet and there were zero deployed artifacts to keep compatible.
/// `PLAN.md` §0.4.5 carries the argument.
///
/// The refusal still has to exist — a name that genuinely does not fit must
/// never be silently shortened (`AGENTS.md` §3), because that renames somebody
/// else's application.
#[test]
fn the_adrs_own_example_packages_and_an_over_long_name_is_still_refused() {
    assert!(
        packaged(
            "metrics-collector",
            "0.3.1",
            "aarch64",
            None,
            Kind::application(),
            &well_formed(),
        )
        .is_ok(),
        "ADR-015 §5.4's example must package; growing product to 32 was the point"
    );

    // Thirty-two fits exactly, so the boundary is legal rather than one short.
    assert!(
        packaged(
            "product-name-of-thirty-two-chars",
            "0.3.1",
            "aarch64",
            None,
            Kind::application(),
            &well_formed(),
        )
        .is_ok()
    );

    // Thirty-three does not, and is refused with the limit named.
    assert_eq!(
        packaged(
            "product-name-with-thirty-three-ch",
            "0.3.1",
            "aarch64",
            None,
            Kind::application(),
            &well_formed(),
        ),
        Err(PackageError::TextTooLong {
            field: "product",
            len: 33,
            capacity: 32
        })
    );
}

// ---------------------------------------------------------------------------
// `ADR-015` §6.2's declared device capability — kiosk mode, 2026-08-31.
// ---------------------------------------------------------------------------

/// **The order is canonical, not the author's.** Two orderings of one request
/// must be one artifact: the digest of this member is in a signed header, so a
/// second spelling would be a different artifact asking for the same thing.
#[test]
fn devices_are_sorted_and_deduplicated_however_they_are_declared() {
    let one = Manifest::new(ProgramKind::Daemon)
        .with_devices(vec![Device::Input, Device::Display, Device::Input]);
    let other = Manifest::new(ProgramKind::Daemon)
        .with_devices(vec![Device::Display, Device::Input]);
    assert_eq!(one.devices(), &[Device::Display, Device::Input]);
    assert_eq!(one.render(), other.render());
}

/// A manifest that is not in canonical order is refused rather than sorted.
/// The emitter is the only legitimate producer, so anything else is a second
/// one — the same argument `parse` makes about key order.
#[test]
fn a_devices_list_out_of_order_or_with_a_repeat_is_refused() {
    for bad in [
        "devices = [\"input\", \"display\"]",
        "devices = [\"display\", \"display\"]",
    ] {
        let text = sample_manifest().render().replace("devices = [\"display\", \"input\"]", bad);
        assert_eq!(
            Manifest::parse(text.as_bytes()),
            Err(ManifestFault::BadValue { key: "devices" }),
            "{bad}"
        );
    }
}

#[test]
fn a_device_class_this_build_does_not_define_is_refused() {
    let text = sample_manifest()
        .render()
        .replace("devices = [\"display\", \"input\"]", "devices = [\"gpio\"]");
    assert_eq!(
        Manifest::parse(text.as_bytes()),
        Err(ManifestFault::BadValue { key: "devices" })
    );
}

/// **Which classes are exclusive, one by one, because the answers differ and
/// each has its own argument.**
///
/// This asserted *"every class is exclusive"* until 2026-09-20, and it caught
/// the change that made one shareable — which is what it was for: the comment
/// then read *"the assertion exists so that making one shareable is a decision
/// with a failing test in front of it."* `.rulings/` R-206 D4 is that decision,
/// so the test now states the outcome rather than the prohibition.
///
/// * `Display` — `ADR-015` §6, ruled: two programs writing a framebuffer
///   interleave and neither is wrong, so the second must be refused.
/// * `Input` — beyond that ruling and deliberately closed: evdev broadcasts, so
///   a second reader is not a race, it is a keylogger.
/// * `Gpu` — **shareable.** A render node has no framebuffer to interleave and
///   no broadcast stream to read; it is opened by many processes at once on
///   every ordinary system.
#[test]
fn a_device_class_is_exclusive_only_where_sharing_would_be_wrong() {
    assert!(Device::Display.exclusive(), "the screen is ADR-015 6.2's");
    assert!(Device::Input.exclusive(), "a second evdev reader is a keylogger");
    assert!(
        !Device::Gpu.exclusive(),
        "a render node cannot modeset and cannot be interleaved: refusing a second opener would \
         mean one GPU program per machine, which is not a property anybody asked for"
    );
    assert_eq!(Device::ALL, &[Device::Display, Device::Input, Device::Gpu]);
}

/// **The GPU token grants a render node and NEVER a card node**, in either
/// display role — `.rulings/` R-206 D4.
///
/// `rivet-core/src/service.rs` gates this at compile time as well, and this is
/// the runtime half saying why: the entire safety of letting an application
/// declare `gpu` is that the token cannot widen into DRM master. If it could,
/// `ADR-058`'s *"an application does not get the screen"* would be one manifest
/// keyword away from false.
#[test]
fn the_gpu_token_can_never_reach_a_card_node() {
    for role in [
        crate::service::DisplayRole::Arbiter,
        crate::service::DisplayRole::Lessee,
    ] {
        let globs = crate::service::device_globs(Device::Gpu, role);
        assert!(!globs.is_empty(), "declaring `gpu` must grant something");
        for glob in globs {
            assert!(
                glob.starts_with("/dev/dri/renderD"),
                "`gpu` in role {role:?} granted {glob}, which is not a render node"
            );
        }
    }
}

#[test]
fn a_device_token_round_trips() {
    for device in Device::ALL {
        assert_eq!(Device::parse(device.as_str()), Some(*device));
    }
    assert_eq!(Device::parse("Display"), None);
    assert_eq!(Device::parse(""), None);
}

// ---------------------------------------------------------------------------
// Settings — ADR-038 §3
// ---------------------------------------------------------------------------

/// **A manifest without the key parses unchanged**, which is the promise that
/// let this key be added without rebuilding every `.rvt` on the board.
///
/// `library` and `devices` are REQUIRED and their arrival invalidated every
/// existing package — deliberately, because a capability that defaulted to
/// absent would be indistinguishable from one an author never considered. A
/// setting is not that: declaring none **accepts none**, which is the closed
/// end of the axis (`ADR-015` §3.3). This is that promise in executable form,
/// and `manifest_text_is_exactly_this` above is the other half — it pins the
/// bytes of a manifest that declares no settings and did not change.
#[test]
fn a_manifest_from_before_the_settings_key_parses_and_declares_nothing() {
    let text = sample_manifest().render();
    assert!(
        !text.contains("settings"),
        "an empty declaration must not be written, or an older reader meets a new key: {text}"
    );
    let read = Manifest::parse(text.as_bytes()).expect("a manifest with no settings must parse");
    assert!(read.settings().is_empty());
    assert!(!read.declares_setting("brightness"));
    assert_eq!(read.render(), text, "the artifact changed for an application that declares none");
}

/// What an application declares survives the round trip, in order.
#[test]
fn a_settings_declaration_round_trips_in_the_order_it_was_written() {
    let declared = vec![
        "brightness".to_string(),
        "background".to_string(),
        "show-weather".to_string(),
    ];
    let manifest = sample_manifest()
        .with_settings(declared.clone())
        .expect("valid names");
    let text = manifest.render();
    let read = Manifest::parse(text.as_bytes()).expect("round trip");
    assert_eq!(read.settings(), declared.as_slice());
    assert!(read.declares_setting("brightness"));
    assert!(!read.declares_setting("brightnes"), "a near miss must not match");
    assert_eq!(read.render(), text);
}

/// **The rule is narrow on purpose**: a setting name is written by an operator
/// into a machine document and read back by a program, so every character it
/// admits is one two implementations have to agree about.
#[test]
fn a_setting_name_is_lowercase_letters_digits_and_hyphens() {
    for good in ["brightness", "show-weather", "slot-2", "a"] {
        assert!(is_setting_name(good), "{good} should be a name");
        assert!(sample_manifest().with_settings(vec![good.to_string()]).is_ok(), "{good}");
    }
    for bad in [
        "",
        "Brightness",       // an operator writing a document does not shift
        "brightness_level", // two spellings of one name
        "brightness.level",
        "2slots",           // must begin with a letter
        "-leading",
        "with space",
        "café",
    ] {
        assert!(!is_setting_name(bad), "{bad:?} should be refused");
        assert!(
            sample_manifest().with_settings(vec![bad.to_string()]).is_err(),
            "{bad:?} reached a manifest"
        );
    }
}

/// **A duplicate is refused rather than deduplicated.** Two declarations of one
/// name is not a longer list; it is a manifest that cannot say which it meant,
/// and quietly keeping one is choosing on the author's behalf.
#[test]
fn declaring_one_setting_name_twice_is_refused() {
    let twice = vec!["brightness".to_string(), "brightness".to_string()];
    assert!(sample_manifest().with_settings(twice).is_err());
    // And on the way IN as well, so a hand-written manifest cannot smuggle one
    // past the builder.
    let text = sample_manifest().render()
        + "settings = [\"brightness\", \"brightness\"]\n";
    assert!(Manifest::parse(text.as_bytes()).is_err(), "a duplicate parsed: {text}");
}

/// **A credential-shaped name is refused, on both ends** — `ADR-038` §5,
/// `.rulings/` R-210 D16. TempMon's own `write-token` is the fixture that
/// found this; the negative cases are `upstream-name` and `sensor-labels`,
/// named in the original ruling as what must NOT be caught.
///
/// `apitoken`, `secretary` and `tokenizer` are here because D16 moved them
/// IN: `token` and `secret` now match anywhere in a name, not only as a
/// whole hyphen-separated word, so `cargo rivet package` and this parser
/// must refuse them exactly as they refuse `write-token`.
#[test]
fn a_credential_shaped_setting_name_is_refused() {
    for bad in [
        "write-token", "api-key", "password", "passwd", "credential", "secret", "token", "key",
        "relay-credential-file", "secret-santa", "apitoken", "db-passwd", "usercredentials",
        "secretary", "tokenizer",
    ] {
        assert!(is_credential_like_name(bad), "{bad:?} should read as credential-shaped");
        match sample_manifest().with_settings(vec![bad.to_string()]) {
            Err(ManifestFault::CredentialLikeSetting) => {}
            other => panic!("{bad:?}: expected CredentialLikeSetting, got {other:?}"),
        }
        // And on the way IN, so a hand-written manifest cannot smuggle one
        // past the builder either — the supervisor reads a manifest through
        // this parser, never through `with_settings`.
        let text = sample_manifest().render() + &format!("settings = [{bad:?}]\n");
        assert!(
            matches!(Manifest::parse(text.as_bytes()), Err(ManifestFault::CredentialLikeSetting)),
            "{bad:?} parsed: {text}"
        );
    }
}

/// **Don't match too broadly** — the negative cases the ruling named by name,
/// plus the shapes that would trip a substring check on `key`:
/// `keyboard-layout`, `monkey` and `turkey-count`.
#[test]
fn a_setting_name_that_merely_contains_a_credential_word_is_not_credential_shaped() {
    for good in [
        "upstream-name",
        "sensor-labels",
        "keyboard-layout",
        "monkey",
        "turkey-count",
        "brightness",
    ] {
        assert!(!is_credential_like_name(good), "{good:?} should NOT read as credential-shaped");
        assert!(
            sample_manifest().with_settings(vec![good.to_string()]).is_ok(),
            "{good:?} was refused"
        );
    }
}

/// **D16, in full: one rule, matched case-insensitively, with `key` as the
/// only whole-word exception** — `.rulings/` R-210 D16. This is the pure
/// predicate, not the manifest round trip: several of these spellings
/// (`API_TOKEN`, `apiKey`, `Password`, `keyboard_layout`, `keyboardLayout`)
/// use a character [`is_setting_name`] itself refuses, so they can never
/// reach [`is_credential_like_name`] through a manifest — they reach it
/// through `rivet-fleetd`'s free-text fleet-document keys instead, which is
/// exactly why the rule has to be case-insensitive at all.
#[test]
fn is_credential_like_name_matches_case_insensitively_and_key_only_as_a_whole_word() {
    for bad in [
        "apitoken",
        "authToken",
        "API_TOKEN",
        "db-passwd",
        "db_passwd",
        "usercredentials",
        "write-token",
        "api-key",
        "api_key",
        "apiKey",
        "key",
        "secret-santa",
        "secretary",
        "tokenizer",
        "Password",
    ] {
        assert!(is_credential_like_name(bad), "{bad:?} should read as credential-shaped");
    }
    for good in [
        "keyboard-layout",
        "keyboard_layout",
        "keyboardLayout",
        "monkey",
        "turkey-count",
        "upstream-name",
        "sensor-labels",
        "brightness",
    ] {
        assert!(!is_credential_like_name(good), "{good:?} should NOT read as credential-shaped");
    }
}

/// The list is bounded like every other, and the refusal names the key.
#[test]
fn a_settings_declaration_longer_than_the_bound_is_refused_by_name() {
    let many: Vec<String> = (0..=MAX_LIST).map(|i| format!("setting-{i}")).collect();
    match sample_manifest().with_settings(many) {
        Err(ManifestFault::ListTooLong { key }) => assert_eq!(key, "settings"),
        other => panic!("expected a bounded refusal naming the key, got {other:?}"),
    }
}

/// **The last key in `KEYS` is the last one written**, which is what makes
/// appending safe: the canonical order is `KEYS`, and a key at the end cannot
/// put an older manifest out of order.
///
/// **Generalised 2026-09-03**, when `proxy-path` was appended after `settings`
/// and this test failed. It had asserted the NAME of the last key, so every
/// append breaks it and the fix is to retype the name — which would make it a
/// test of what somebody last typed rather than of the invariant. It now
/// asserts the invariant: whatever is last in `KEYS` renders last.
///
/// **It has to set every optional key that CAN be set at once**, because they
/// are all written conditionally — one left at its default renders nothing and
/// the last line then belongs to the key before it. `proxy-auth` (2026-09-05)
/// is the second append this test has seen, `proxy-auth-paths` the third, and
/// `ADR-051`'s backup keys (2026-09-08) the fourth.
///
/// **The fourth append could not set every key at once**, and that is the
/// declaration rather than an omission: `backup-feed` is mutually exclusive
/// with `backup-path` and `backup-format`, so no manifest can carry all three.
/// The feed was set because it was, at the time, the key last in `KEYS`.
///
/// **`cpu` (2026-09-09) is the fifth append, and sets cleanly** — nothing
/// about it excludes any other key — so it is simply added on top rather than
/// replacing the feed.
///
/// **`accepts-class` (2026-09-13, `ADR-053`) is the sixth append and sets
/// cleanly too**, for `cpu`'s reason.
///
/// **`restore-path` (2026-09-14, `ADR-051` §12) is the seventh, and it swaps
/// the feed back for an artifact**, because a restore exists only beside one.
/// The route is `proxy-auth = "none"` with `admin` tightened back to
/// `operator`, and the restore lives under `admin` — so this manifest is also
/// the one that shows a tightening rule is what makes a restore declarable on
/// an otherwise open route.
///
/// **`material` (2026-09-15, `ADR-055` §3) is the eighth, and sets cleanly**
/// for `cpu`'s reason: it excludes no other key and is not guarded by the
/// route, so it is added on top.
#[test]
fn the_last_key_in_keys_is_the_last_one_rendered() {
    let last = *KEYS.last().expect("KEYS is not empty");
    let text = sample_manifest()
        .with_settings(vec!["brightness".to_string()])
        .expect("valid")
        .with_proxy_path(Some("aura".to_string()))
        .expect("valid")
        .with_proxy_auth(ProxyAuth::None)
        .with_proxy_auth_paths(vec![
            ProxyRule::new(ProxyAuth::Operator, "admin").expect("valid")
        ])
        .expect("valid")
        .with_backup(Some(BackupSurface::Artifact {
            path: "api/v1/backup".to_string(),
            format: BackupFormat::Json,
            restore: Some("admin/restore".to_string()),
        }))
        .expect("valid")
        .with_cpu(Some(Cpu::parse("0.5").expect("valid")))
        .with_accepts_classes(vec!["temperature-sensor".to_string()])
        .expect("valid")
        .with_material(vec![Material::FleetTls])
        .render();
    let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    assert!(
        lines
            .last()
            .expect("rendered something")
            .starts_with(&format!("{last} = ")),
        "the last key in KEYS is {last}, so it must render last: {lines:?}"
    );
}

/// The two optional trailing keys are written in `KEYS` order relative to each
/// other, which a manifest carrying both is the only thing that checks.
#[test]
fn settings_still_precedes_proxy_path_when_both_are_set() {
    let text = sample_manifest()
        .with_settings(vec!["brightness".to_string()])
        .expect("valid")
        .with_proxy_path(Some("aura".to_string()))
        .expect("valid")
        .render();
    let settings_at = text.find("settings = ").expect("settings rendered");
    let proxy_at = text.find("proxy-path = ").expect("proxy-path rendered");
    assert!(settings_at < proxy_at, "KEYS order is not the render order:\n{text}");
    Manifest::parse(text.as_bytes()).expect("what the emitter writes, the parser reads");
}

// ---------------------------------------------------------------------------
// `proxy-path` — R-133
// ---------------------------------------------------------------------------

/// **Absence means NOT PROXIED**, and it is the state every manifest emitted
/// before `R-133` is in. A default of *the service name* would mount every
/// application on the machine the moment a proxy was enabled, which is the
/// opposite of `ADR-015` §3.3's closed end.
#[test]
fn a_manifest_that_says_nothing_is_not_proxied_and_renders_no_key() {
    let manifest = sample_manifest();
    assert_eq!(manifest.proxy_path(), None);
    let text = manifest.render();
    assert!(
        !text.contains("proxy-path"),
        "an absent path must render NO key, or an old manifest stops round-tripping:\n{text}"
    );
    let back = Manifest::parse(text.as_bytes()).expect("round trip");
    assert_eq!(back.proxy_path(), None);
}

/// **The defect this test exists for**, found on 2026-09-05 by the first
/// application ever to declare a title: `title` was in `KEYS` and in `render`
/// and in no arm of `parse`, so a payload carrying one was refused by every
/// build as *a key is not one this build defines* — including the tool that had
/// just written it. A round trip is the one shape that catches a writer and a
/// reader disagreeing.
#[test]
fn a_title_round_trips_through_the_canonical_text() {
    let text = sample_manifest()
        .with_title(Some("Aura Controller".to_string()))
        .expect("printable ASCII")
        .render();
    assert!(text.contains("title = \"Aura Controller\""), "{text}");
    let back = Manifest::parse(text.as_bytes()).expect("round trip");
    assert_eq!(back.title("aura-controller"), "Aura Controller");
}

/// Absence means *call it by its service name*, and that has to survive the
/// round trip too — a parser that invented an empty title would put a blank row
/// in a package list.
#[test]
fn a_manifest_with_no_title_is_called_by_its_service_name() {
    let text = sample_manifest().render();
    assert!(!text.contains("title"), "{text}");
    let back = Manifest::parse(text.as_bytes()).expect("round trip");
    assert_eq!(back.title("aura-renderer"), "aura-renderer");
}

/// **The builder and the parser hold one rule between them**, so a title one
/// accepts the other must accept. They had two copies and then one; this is
/// what keeps it one.
#[test]
fn the_builder_and_the_parser_agree_about_every_title() {
    // A quote or a backslash would end the rendered string early or start an
    // escape the reader does not implement; a newline would end the line.
    let hostile = [
        "",
        "has \" quote",
        "has \\ backslash",
        "has \n newline",
        "has \t tab",
        "café",
    ];
    for title in hostile {
        assert!(
            sample_manifest().with_title(Some(title.to_string())).is_err(),
            "the builder accepted {title:?}"
        );
        let text = format!("{}title = \"{title}\"\n", sample_manifest().render());
        assert!(
            Manifest::parse(text.as_bytes()).is_err(),
            "the parser accepted {title:?}"
        );
    }
}

#[test]
fn a_path_round_trips_through_the_canonical_text() {
    let text = sample_manifest()
        .with_proxy_path(Some("reliquary".to_string()))
        .expect("a legal segment")
        .render();
    let back = Manifest::parse(text.as_bytes()).expect("round trip");
    assert_eq!(back.proxy_path(), Some("reliquary"));
}

/// **The refusals that matter are the traversal ones.** A proxy that had to
/// normalise a path before trusting it is a proxy with a traversal bug waiting
/// in it, so the shapes that could escape a prefix are refused at the manifest
/// rather than defended against later.
#[test]
fn a_path_that_could_escape_its_own_prefix_is_refused() {
    for hostile in ["..", "../etc", "a/b", "/aura", "aura/", "a//b", ".", "./x"] {
        assert!(
            sample_manifest()
                .with_proxy_path(Some(hostile.to_string()))
                .is_err(),
            "{hostile:?} must be refused: it can escape or nest a route"
        );
        assert!(
            !is_proxy_path(hostile),
            "{hostile:?} must not satisfy the predicate either"
        );
    }
}

#[test]
fn a_path_is_one_lowercase_segment_and_nothing_else() {
    for legal in ["a", "aura", "reliquary", "trends", "a1", "a-b", "x9-y"] {
        assert!(is_proxy_path(legal), "{legal:?} should be legal");
        assert!(sample_manifest().with_proxy_path(Some(legal.to_string())).is_ok());
    }
    for illegal in ["", "Aura", "1aura", "-aura", "au_ra", "au.ra", "aura ", " aura", "aur a"] {
        assert!(!is_proxy_path(illegal), "{illegal:?} should be refused");
    }
}

/// The alphabet agrees with a service name's, which is what makes the obvious
/// default — *mount it at its own name* — always expressible.
#[test]
fn every_service_name_is_a_legal_proxy_path() {
    for name in ["reliquary-rivet", "aura-controller", "rivet-fleetd", "trends"] {
        assert!(
            crate::service::ServiceName::new(name).is_some(),
            "{name:?} should be a service name"
        );
        assert!(
            is_proxy_path(name),
            "{name:?} is a service name and must therefore be a legal proxy path"
        );
    }
}

/// An oversized path is refused by length rather than by alphabet, so the
/// message names the real problem.
#[test]
fn an_oversized_path_is_refused_as_too_long() {
    let long = "a".repeat(MAX_VALUE + 1);
    match sample_manifest().with_proxy_path(Some(long)) {
        Err(ManifestFault::ValueTooLong) => {}
        other => panic!("expected ValueTooLong, got {other:?}"),
    }
}

/// A hostile *parse* rather than a hostile builder call: the bytes could come
/// from a `.rvt` somebody else made, and the parser is the only thing between
/// them and a routing table.
#[test]
fn the_parser_refuses_a_hostile_path_in_bytes_it_did_not_emit() {
    let good = sample_manifest()
        .with_proxy_path(Some("aura".to_string()))
        .expect("valid")
        .render();
    let hostile = good.replace("proxy-path = \"aura\"", "proxy-path = \"../etc\"");
    assert!(
        Manifest::parse(hostile.as_bytes()).is_err(),
        "the parser must refuse what the builder refuses; it reads bytes from elsewhere"
    );
}

/// **`sign-in` is the machine's own page, and no application is mounted there**
/// — `ADR-046` §4. Refused by the builder packaging uses and by the parser an
/// installing machine reads with, for its NAME: its shape is a legal path.
#[test]
fn a_reserved_path_is_refused_by_the_builder_and_by_the_parser() {
    assert!(RESERVED_PROXY_PATHS.contains(&"sign-in"));
    let good = sample_manifest()
        .with_proxy_path(Some("aura".to_string()))
        .expect("valid")
        .render();
    for reserved in RESERVED_PROXY_PATHS {
        assert!(is_proxy_path(reserved), "{reserved:?} is refused for its name, not its shape");
        assert!(!is_mountable_proxy_path(reserved), "{reserved:?}");
        match sample_manifest().with_proxy_path(Some((*reserved).to_string())) {
            Err(ManifestFault::ReservedProxyPath) => {}
            other => panic!("{reserved:?}: expected ReservedProxyPath, got {other:?}"),
        }
        let claimed = good.replace(
            "proxy-path = \"aura\"",
            &format!("proxy-path = \"{reserved}\""),
        );
        assert_ne!(claimed, good, "the fixture must carry the key being replaced");
        assert_eq!(
            Manifest::parse(claimed.as_bytes()).err(),
            Some(ManifestFault::ReservedProxyPath),
            "{reserved:?}: the parser must refuse what the builder refuses"
        );
    }
    assert!(ManifestFault::ReservedProxyPath.to_string().contains("sign-in pages"));
}

/// The pages the proxy serves are under the path this list reserves, so the
/// two cannot drift apart.
#[test]
fn the_sign_in_pages_live_under_a_reserved_path() {
    let page = crate::passkey::SIGN_IN_PAGE;
    assert!(is_reserved_proxy_path(page.strip_prefix('/').expect("absolute")), "{page}");
    assert!(
        crate::passkey::ENROLL_PAGE.starts_with(&format!("{page}/")),
        "{}",
        crate::passkey::ENROLL_PAGE
    );
}

/// The reservation is of a whole route's name. A near miss is an ordinary
/// path, and an application's own prefix under its mount may be spelled
/// `sign-in`, because `/<proxy-path>/sign-in` is inside that application.
#[test]
fn a_reserved_name_reserves_only_the_route_itself() {
    for near in ["sign-ins", "signin", "sign-in-help", "my-sign-in"] {
        assert!(is_mountable_proxy_path(near), "{near:?}");
        assert!(sample_manifest().with_proxy_path(Some(near.to_string())).is_ok(), "{near:?}");
    }
    assert!(is_proxy_prefix("sign-in"));
    assert!(is_proxy_prefix("account/sign-in"));
}

// ---------------------------------------------------------------------------
// `proxy-auth` — R-152, ADR-046 §4.3
// ---------------------------------------------------------------------------

/// **The whole design of the key in one assertion.** `ADR-046` §4.3: a key that
/// must be PRESENT to tighten security is one an author forgets, so absence is
/// *require an operator* and the key exists only to relax.
///
/// This is what makes the change safe for every `.rvt` already on a board: none
/// of them carries the key, and every one of them therefore becomes gated
/// rather than open the moment a proxy starts checking.
#[test]
fn a_manifest_that_says_nothing_requires_an_operator() {
    let manifest = sample_manifest();
    assert_eq!(manifest.proxy_auth(), ProxyAuth::Operator);
    assert!(manifest.proxy_auth().requires_operator());

    let text = manifest.render();
    assert!(
        !text.contains("proxy-auth"),
        "the default must render NO key, or an old manifest stops round-tripping:\n{text}"
    );
    let back = Manifest::parse(text.as_bytes()).expect("round trip");
    assert!(back.proxy_auth().requires_operator());
}

/// The same for a manifest that HAS a route: declaring `proxy-path` and nothing
/// else is the gated case, which is what `notes-rivet` and every other
/// application shipped before today looks like.
#[test]
fn a_route_declared_before_this_key_existed_is_gated() {
    let text = sample_manifest()
        .with_proxy_path(Some("notes".to_string()))
        .expect("a legal segment")
        .render();
    assert!(!text.contains("proxy-auth"), "{text}");
    let back = Manifest::parse(text.as_bytes()).expect("round trip");
    assert_eq!(back.proxy_path(), Some("notes"));
    assert!(
        back.proxy_auth().requires_operator(),
        "a route that says nothing about callers must require one"
    );
}

/// `.rulings/` R-152's TempMon case — an application served over TLS without
/// authenticating its callers — the one thing the key is for.
#[test]
fn a_route_may_relax_and_it_round_trips() {
    let text = sample_manifest()
        .with_proxy_path(Some("tempmon".to_string()))
        .expect("a legal segment")
        .with_proxy_auth(ProxyAuth::None)
        .render();
    assert!(text.contains("proxy-auth = \"none\"\n"), "{text}");
    let back = Manifest::parse(text.as_bytes()).expect("round trip");
    assert_eq!(back.proxy_path(), Some("tempmon"));
    assert_eq!(back.proxy_auth(), ProxyAuth::None);
    assert!(!back.proxy_auth().requires_operator());
}

/// **A relaxation attached to no route is refused**, because it is a key that
/// would change meaning by an edit elsewhere in the file — and because it is
/// what a manifest looks like when an author deleted the wrong line.
#[test]
fn relaxing_a_route_that_does_not_exist_is_refused() {
    let text = sample_manifest().render();
    let orphan = format!("{text}proxy-auth = \"none\"\n");
    match Manifest::parse(orphan.as_bytes()) {
        Err(ManifestFault::MissingKey { key: "proxy-path" }) => {}
        other => panic!("expected the refusal to name proxy-path, got {other:?}"),
    }
}

/// The emitter never writes the default, so the parser must never be handed it
/// by the emitter — but bytes come from elsewhere, and the explicit safe
/// spelling must not be a refusal an author cannot understand.
#[test]
fn the_explicit_safe_value_is_accepted_from_bytes_the_emitter_did_not_write() {
    let good = sample_manifest()
        .with_proxy_path(Some("aura".to_string()))
        .expect("valid")
        .render();
    let explicit = format!("{good}proxy-auth = \"operator\"\n");
    let back = Manifest::parse(explicit.as_bytes()).expect("operator is a value this key admits");
    assert!(back.proxy_auth().requires_operator());
}

/// **A value this build does not define must not become a relaxation.** The
/// bytes could come from a `.rvt` somebody else made, and `ProxyAuth::parse`
/// is the only thing between them and an ungated route.
#[test]
fn an_unknown_value_is_refused_rather_than_guessed() {
    let good = sample_manifest()
        .with_proxy_path(Some("aura".to_string()))
        .expect("valid")
        .render();
    for hostile in ["", "None", "public", "anonymous", "operator ", "no"] {
        let text = format!("{good}proxy-auth = \"{hostile}\"\n");
        match Manifest::parse(text.as_bytes()) {
            Err(ManifestFault::BadValue { key: "proxy-auth" } | ManifestFault::BadCharacter) => {}
            other => panic!("{hostile:?} must be refused, got {other:?}"),
        }
        assert!(ProxyAuth::parse(hostile).is_none(), "{hostile:?}");
    }
}

/// `proxy-auth` is last in `KEYS`, so it renders after `proxy-path` — the only
/// manifest that checks the pair is one carrying both.
#[test]
fn proxy_path_still_precedes_proxy_auth_when_both_are_set() {
    let text = sample_manifest()
        .with_settings(vec!["brightness".to_string()])
        .expect("valid")
        .with_proxy_path(Some("tempmon".to_string()))
        .expect("valid")
        .with_proxy_auth(ProxyAuth::None)
        .render();
    let path_at = text.find("proxy-path = ").expect("proxy-path rendered");
    let auth_at = text.find("proxy-auth = ").expect("proxy-auth rendered");
    assert!(path_at < auth_at, "KEYS order is not the render order:\n{text}");
    Manifest::parse(text.as_bytes()).expect("what the emitter writes, the parser reads");
}

// ---------------------------------------------------------------------------
// `app` and `plain` — ADR-048 §4, ADR-061 §7, `.rulings/` R-221
// ---------------------------------------------------------------------------

/// **`app` and `plain` answer for a PREFIX, never for the whole route** —
/// `ADR-046` §4's grammar for the bare key is `operator` and `none` only.
#[test]
fn app_and_plain_are_refused_as_the_whole_routes_gate() {
    let good = sample_manifest()
        .with_proxy_path(Some("gantry".to_string()))
        .expect("valid")
        .render();
    for hostile in ["app", "plain"] {
        let text = format!("{good}proxy-auth = \"{hostile}\"\n");
        match Manifest::parse(text.as_bytes()) {
            Err(ManifestFault::BadValue { key: "proxy-auth" }) => {}
            other => panic!("{hostile:?} must be refused as a whole-route gate, got {other:?}"),
        }
        assert!(
            !ProxyAuth::parse(hostile).expect("a real verdict").allowed_for_whole_route(),
            "{hostile:?}"
        );
    }
}

/// The two new verdicts parse, emit and round-trip as per-path rules, exactly
/// as `none` and `operator` do.
#[test]
fn app_and_plain_parse_emit_and_round_trip_as_rules() {
    let text = with_rules(&["app:api", "plain:install"]).render();
    assert!(
        text.contains("proxy-auth-paths = [\"app:api\", \"plain:install\"]\n"),
        "{text}"
    );
    let back = Manifest::parse(text.as_bytes()).expect("round trip");
    assert_eq!(back.proxy_auth_paths()[0].prefix(), "api");
    assert_eq!(back.proxy_auth_paths()[0].auth(), ProxyAuth::App);
    assert_eq!(back.proxy_auth_paths()[1].prefix(), "install");
    assert_eq!(back.proxy_auth_paths()[1].auth(), ProxyAuth::Plain);
    assert!(!back.proxy_auth_paths()[0].auth().requires_operator());
    assert!(!back.proxy_auth_paths()[1].auth().requires_operator());
}

/// **A restore route is refused under `app:` or `plain:`, exactly as under
/// `none:`** — `ADR-051` §12 does not distinguish the three: none of them
/// requires an operator, and a restore replaces the application's data.
#[test]
fn a_restore_path_under_app_or_plain_is_refused_like_none() {
    for verdict in ["app", "plain"] {
        let text = with_restore("api/v1/restore")
            .with_proxy_auth_paths(vec![
                ProxyRule::parse(&format!("{verdict}:api")).expect("valid"),
            ])
            .expect("valid")
            .render();
        match Manifest::parse(text.as_bytes()) {
            Err(ManifestFault::BadValue {
                key: "restore-path",
            }) => {}
            other => panic!("{verdict:?}: expected restore-path refused, got {other:?}\n{text}"),
        }
    }
}

// ---------------------------------------------------------------------------
// `proxy-auth-paths` — ADR-048 §4
// ---------------------------------------------------------------------------

/// A route with one open door and one gated island in it — Reliquary's shape.
fn with_rules(rules: &[&str]) -> Manifest {
    let rules = rules
        .iter()
        .map(|text| ProxyRule::parse(text).expect("a rule this test meant to write"))
        .collect();
    sample_manifest()
        .with_proxy_path(Some("reliquary".to_string()))
        .expect("valid")
        .with_proxy_auth_paths(rules)
        .expect("valid")
}

/// **The widening adds a state rather than reinterpreting one.** A manifest
/// that names no prefix says exactly what it said before `ADR-048`, renders no
/// key, and round-trips byte-identically — which is what keeps every `.rvt`
/// already signed meaning what it meant.
#[test]
fn a_manifest_that_names_no_prefix_renders_no_key_and_takes_its_route_gate() {
    let manifest = sample_manifest();
    assert!(manifest.proxy_auth_paths().is_empty());
    let text = manifest.render();
    assert!(!text.contains("proxy-auth-paths"), "{text}");
    assert!(
        Manifest::parse(text.as_bytes())
            .expect("round trip")
            .proxy_auth_paths()
            .is_empty()
    );
}

/// The key round-trips, and the manifest spells a rule the manifest's way.
#[test]
fn a_route_may_open_one_prefix_and_re_gate_another_under_it() {
    let text = with_rules(&["none:pair", "operator:pair/admin"]).render();
    assert!(
        text.contains("proxy-auth-paths = [\"none:pair\", \"operator:pair/admin\"]\n"),
        "{text}"
    );
    let back = Manifest::parse(text.as_bytes()).expect("round trip");
    assert_eq!(back.proxy_auth_paths().len(), 2);
    assert_eq!(back.proxy_auth_paths()[0].prefix(), "pair");
    assert_eq!(back.proxy_auth_paths()[0].auth(), ProxyAuth::None);
    assert_eq!(back.proxy_auth_paths()[1].prefix(), "pair/admin");
    assert_eq!(back.proxy_auth_paths()[1].auth(), ProxyAuth::Operator);
}

/// **Two orderings of one intent must not be two artifacts.** The digest of the
/// manifest member is in a signed header, so a second spelling is a different
/// package — and the author who wrote the two lines the other way round would
/// have shipped one.
#[test]
fn the_declaration_order_of_two_rules_does_not_reach_the_bytes() {
    let one_way = with_rules(&["none:pair", "operator:pair/admin"]).render();
    let other = with_rules(&["operator:pair/admin", "none:pair"]).render();
    assert_eq!(one_way, other, "the emitter must canonicalise the order");
}

/// A prefix carrying two verdicts is refused rather than resolved, whichever
/// verdicts they are: picking one would make the answer depend on which line an
/// author typed first.
#[test]
fn one_prefix_may_not_carry_two_verdicts() {
    let rules = vec![
        ProxyRule::new(ProxyAuth::None, "pair").expect("valid"),
        ProxyRule::new(ProxyAuth::Operator, "pair").expect("valid"),
    ];
    match sample_manifest().with_proxy_auth_paths(rules) {
        Err(ManifestFault::BadValue {
            key: "proxy-auth-paths",
        }) => {}
        other => panic!("a duplicate prefix must be refused, got {other:?}"),
    }
}

/// **A rule attached to no route is refused**, for the reason `proxy-auth`
/// alone is: it names a prefix under a route the manifest never declared, and
/// would start meaning something the day somebody added a `proxy-path` beside
/// it.
#[test]
fn a_rule_without_a_proxy_path_is_refused_and_the_refusal_names_the_missing_key() {
    let good = sample_manifest().render();
    let orphan = format!("{good}proxy-auth-paths = [\"none:pair\"]\n");
    match Manifest::parse(orphan.as_bytes()) {
        Err(ManifestFault::MissingKey { key: "proxy-path" }) => {}
        other => panic!("expected the refusal to name proxy-path, got {other:?}"),
    }
}

/// **A prefix that could escape the application it belongs to is refused at the
/// manifest**, so nothing downstream has to normalise one. Text the emitter
/// would never write, driven at the parser directly.
#[test]
fn a_prefix_that_could_escape_its_application_is_refused() {
    let good = with_rules(&["none:pair"]).render();
    for hostile in [
        "none:/pair",
        "none:pair/",
        "none:../etc",
        "none:pair/../admin",
        "none:pair//admin",
        "none:Pair",
        "none:pair%2fadmin",
        "none:",
        "operator:.",
        "pair",
        "yes:pair",
        "none:pair:admin",
    ] {
        let text = good.replace("\"none:pair\"", &format!("\"{hostile}\""));
        assert!(
            text != good,
            "the substitution for {hostile:?} did not happen"
        );
        match Manifest::parse(text.as_bytes()) {
            Err(ManifestFault::BadValue {
                key: "proxy-auth-paths",
            }) => {}
            other => panic!("{hostile:?} must be refused, got {other:?}"),
        }
    }
}

/// The prefix rule is a predicate anybody can ask, and the refusals above are
/// what it says.
#[test]
fn the_prefix_predicate_admits_segments_and_refuses_everything_else() {
    for legal in ["pair", "a", "api/v1/pair", "a-b/c-d", "x2/y3"] {
        assert!(is_proxy_prefix(legal), "{legal:?} should be legal");
    }
    for illegal in [
        "", "/pair", "pair/", "//", "..", ".", "pair/../x", "Pair", "1pair", "pair_x", "pair.x",
        "a/b/c/d/e/f/g/h/i",
    ] {
        assert!(!is_proxy_prefix(illegal), "{illegal:?} should be refused");
    }
}

/// A rule set the emitter would never write — unsorted, or naming one prefix
/// twice — is a second producer, and is refused rather than canonicalised on
/// the way in.
#[test]
fn a_rule_set_the_emitter_would_not_have_written_is_refused() {
    let good = with_rules(&["none:pair"]).render();
    for hostile in [
        "\"operator:pair/admin\", \"none:pair\"",
        "\"none:pair\", \"operator:pair\"",
    ] {
        let text = good.replace("\"none:pair\"", hostile);
        match Manifest::parse(text.as_bytes()) {
            Err(ManifestFault::BadValue {
                key: "proxy-auth-paths",
            }) => {}
            other => panic!("{hostile:?} must be refused, got {other:?}"),
        }
    }
}

/// `proxy-auth-paths` is last in `KEYS`, so it renders after `proxy-auth` — the
/// only manifest that checks the pair is one carrying both.
#[test]
fn proxy_auth_still_precedes_proxy_auth_paths_when_both_are_set() {
    let text = sample_manifest()
        .with_proxy_path(Some("tempmon".to_string()))
        .expect("valid")
        .with_proxy_auth(ProxyAuth::None)
        .with_proxy_auth_paths(vec![
            ProxyRule::new(ProxyAuth::Operator, "admin").expect("valid")
        ])
        .expect("valid")
        .render();
    let auth_at = text.find("proxy-auth = ").expect("proxy-auth rendered");
    let paths_at = text
        .find("proxy-auth-paths = ")
        .expect("proxy-auth-paths rendered");
    assert!(auth_at < paths_at, "KEYS order is not the render order:\n{text}");
    Manifest::parse(text.as_bytes()).expect("what the emitter writes, the parser reads");
}

// ---------------------------------------------------------------------------
// `backup-path`, `backup-format`, `backup-feed` — ADR-051
// ---------------------------------------------------------------------------

/// TempMon's shape: a route, and one fetch that returns the whole database.
fn with_artifact() -> Manifest {
    sample_manifest()
        .with_proxy_path(Some("tempmon".to_string()))
        .expect("valid")
        .with_backup(Some(BackupSurface::Artifact {
            path: "api/v1/backup".to_string(),
            format: BackupFormat::Sqlite,
            restore: None,
        }))
        .expect("valid")
}

/// **The addition adds a state rather than reinterpreting one.** A manifest
/// that offers nothing renders no key and round-trips byte-identically, which
/// is what keeps every `.rvt` already signed meaning what it meant.
#[test]
fn a_manifest_that_offers_no_backup_renders_no_key() {
    let text = sample_manifest().render();
    assert!(!text.contains("backup-"), "{text}");
    let back = Manifest::parse(text.as_bytes()).expect("round trip");
    assert!(back.backup().is_none());
}

#[test]
fn an_artifact_surface_round_trips_through_the_canonical_text() {
    let text = with_artifact().render();
    assert!(text.contains("backup-path = \"api/v1/backup\"\n"), "{text}");
    assert!(text.contains("backup-format = \"sqlite\"\n"), "{text}");
    assert!(!text.contains("backup-feed"), "{text}");
    let back = Manifest::parse(text.as_bytes()).expect("round trip");
    assert_eq!(
        back.backup(),
        Some(&BackupSurface::Artifact {
            path: "api/v1/backup".to_string(),
            format: BackupFormat::Sqlite,
            restore: None,
        })
    );
    assert_eq!(
        back.backup().and_then(BackupSurface::format),
        Some(BackupFormat::Sqlite)
    );
}

#[test]
fn a_feed_surface_round_trips_and_carries_no_format() {
    let text = sample_manifest()
        .with_proxy_path(Some("reliquary".to_string()))
        .expect("valid")
        .with_backup(Some(BackupSurface::Feed {
            path: "api/v1/blobs".to_string(),
        }))
        .expect("valid")
        .render();
    assert!(text.contains("backup-feed = \"api/v1/blobs\"\n"), "{text}");
    assert!(!text.contains("backup-path"), "{text}");
    assert!(!text.contains("backup-format"), "{text}");
    let back = Manifest::parse(text.as_bytes()).expect("round trip");
    assert_eq!(back.backup().and_then(BackupSurface::format), None);
    assert_eq!(back.backup().map(BackupSurface::path), Some("api/v1/blobs"));
}

/// **The two shapes are mutually exclusive and the manifest refuses both.** A
/// manifest carrying an artifact and a feed is two contracts with no rule for
/// choosing between them.
#[test]
fn a_manifest_declaring_both_shapes_is_refused() {
    let good = with_artifact().render();
    let both = format!("{good}backup-feed = \"api/v1/blobs\"\n");
    match Manifest::parse(both.as_bytes()) {
        Err(ManifestFault::BadValue { key: "backup-feed" }) => {}
        other => panic!("both shapes at once must be refused, got {other:?}"),
    }
}

/// **Half the artifact shape is refused, in both directions.** A route with no
/// format is bytes nothing can check past their length; a format with no route
/// is a claim about something unreachable.
#[test]
fn either_half_of_the_artifact_shape_without_the_other_is_refused() {
    let route = sample_manifest()
        .with_proxy_path(Some("tempmon".to_string()))
        .expect("valid")
        .render();

    let no_format = format!("{route}backup-path = \"api/v1/backup\"\n");
    match Manifest::parse(no_format.as_bytes()) {
        Err(ManifestFault::MissingKey {
            key: "backup-format",
        }) => {}
        other => panic!("a path with no format must be refused, got {other:?}"),
    }

    let no_path = format!("{route}backup-format = \"sqlite\"\n");
    match Manifest::parse(no_path.as_bytes()) {
        Err(ManifestFault::MissingKey { key: "backup-path" }) => {}
        other => panic!("a format with no path must be refused, got {other:?}"),
    }
}

/// **A backup surface with no route is unreachable, and accepting it is the
/// same defect as enabling `rivet-proxyd` with no `upstream-name`** —
/// `SPEC-05` §5. The path is RELATIVE to the mount, so with no mount it names
/// nothing until somebody adds one.
#[test]
fn a_backup_surface_without_a_proxy_path_is_refused_and_names_the_missing_key() {
    let good = sample_manifest().render();
    for orphan in [
        format!("{good}backup-path = \"api/v1/backup\"\nbackup-format = \"sqlite\"\n"),
        format!("{good}backup-feed = \"api/v1/blobs\"\n"),
    ] {
        match Manifest::parse(orphan.as_bytes()) {
            Err(ManifestFault::MissingKey { key: "proxy-path" }) => {}
            other => panic!("expected the refusal to name proxy-path, got {other:?}"),
        }
    }
}

/// A backup path that could escape the application it belongs to is refused at
/// the manifest, so nothing downstream has to normalise one.
#[test]
fn a_backup_path_that_could_escape_its_application_is_refused() {
    let good = with_artifact().render();
    for hostile in [
        "/api/v1/backup",
        "api/v1/backup/",
        "../etc/shadow",
        "api/../../state",
        "api//v1",
        "API/v1",
        "1api/v1",
        ".",
    ] {
        let text = good.replace("\"api/v1/backup\"", &format!("\"{hostile}\""));
        assert!(
            text != good,
            "the substitution for {hostile:?} did not happen"
        );
        match Manifest::parse(text.as_bytes()) {
            Err(ManifestFault::BadValue { key: "backup-path" } | ManifestFault::BadCharacter) => {}
            other => panic!("{hostile:?} must be refused, got {other:?}"),
        }
    }
}

/// A format this build does not define is a refusal rather than a guess: a
/// client that guessed would validate a file against the wrong ladder.
#[test]
fn a_format_this_build_does_not_define_is_refused() {
    let good = with_artifact().render();
    for hostile in ["parquet", "SQLITE", "db", "", "sqlite3"] {
        let text = good.replace("\"sqlite\"", &format!("\"{hostile}\""));
        assert!(
            text != good,
            "the substitution for {hostile:?} did not happen"
        );
        match Manifest::parse(text.as_bytes()) {
            Err(ManifestFault::BadValue {
                key: "backup-format",
            }) => {}
            other => panic!("{hostile:?} must be refused, got {other:?}"),
        }
    }
}

/// The builder refuses the same paths the parser does — `ADR-011`, one rule
/// with two callers rather than two rules that agree today.
#[test]
fn the_builder_refuses_a_backup_path_the_parser_would() {
    for hostile in ["/api", "api/", "..", "", "Api"] {
        assert!(
            sample_manifest()
                .with_backup(Some(BackupSurface::Feed {
                    path: hostile.to_string()
                }))
                .is_err(),
            "{hostile:?} should be refused by the builder"
        );
    }
}

/// `backup-*` is at the end of `KEYS`, so it renders after `proxy-auth-paths` —
/// the only manifest that checks the pair is one carrying both.
#[test]
fn proxy_auth_paths_still_precedes_the_backup_keys_when_both_are_set() {
    let text = sample_manifest()
        .with_proxy_path(Some("reliquary".to_string()))
        .expect("valid")
        .with_proxy_auth_paths(vec![ProxyRule::new(ProxyAuth::None, "pair").expect("valid")])
        .expect("valid")
        .with_backup(Some(BackupSurface::Feed {
            path: "api/v1/blobs".to_string(),
        }))
        .expect("valid")
        .render();
    let rules_at = text.find("proxy-auth-paths = ").expect("rendered");
    let feed_at = text.find("backup-feed = ").expect("rendered");
    assert!(
        rules_at < feed_at,
        "KEYS order is not the render order:\n{text}"
    );
    Manifest::parse(text.as_bytes()).expect("what the emitter writes, the parser reads");
}

/// The two backup keys of the artifact shape are themselves in `KEYS` order.
#[test]
fn backup_path_still_precedes_backup_format() {
    let text = with_artifact().render();
    let path_at = text.find("backup-path = ").expect("rendered");
    let format_at = text.find("backup-format = ").expect("rendered");
    assert!(
        path_at < format_at,
        "KEYS order is not the render order:\n{text}"
    );
}

// ---------------------------------------------------------------------------
// `restore-path` — ADR-051 §12, `.rulings/` R-183 D1
// ---------------------------------------------------------------------------

/// TempMon's shape again, accepting its backup back.
fn with_restore(restore: &str) -> Manifest {
    sample_manifest()
        .with_proxy_path(Some("tempmon".to_string()))
        .expect("valid")
        .with_backup(Some(BackupSurface::Artifact {
            path: "api/v1/backup".to_string(),
            format: BackupFormat::Sqlite,
            restore: Some(restore.to_string()),
        }))
        .expect("valid")
}

/// **It adds a state rather than reinterpreting one.** A manifest with a
/// backup and no restore renders no `restore-path` line — which is what every
/// `.rvt` already signed carries — and a restore round-trips through the
/// canonical text, last, onto the artifact it belongs to.
#[test]
fn a_restore_path_round_trips_and_its_absence_renders_nothing() {
    let without = with_artifact().render();
    assert!(!without.contains("restore-path"), "{without}");

    let text = with_restore("api/v1/restore").render();
    assert!(text.contains("restore-path = \"api/v1/restore\"\n"), "{text}");
    let last = text.lines().rfind(|l| !l.is_empty()).expect("rendered");
    assert!(last.starts_with("restore-path = "), "{text}");
    let back = Manifest::parse(text.as_bytes()).expect("round trip");
    assert_eq!(
        back.backup().and_then(BackupSurface::restore_path),
        Some("api/v1/restore")
    );
    assert_eq!(
        back.backup().and_then(BackupSurface::format),
        Some(BackupFormat::Sqlite)
    );
    assert_eq!(back.render(), text, "render is canonical");
}

/// **The same route as the backup is allowed.** `GET` and `POST` on one path
/// is an ordinary HTTP shape, and nothing about a restore needs it elsewhere.
#[test]
fn a_restore_path_may_be_the_backup_path_itself() {
    let text = with_restore("api/v1/backup").render();
    let back = Manifest::parse(text.as_bytes()).expect("the same route is legal");
    assert_eq!(
        back.backup().and_then(BackupSurface::restore_path),
        Some("api/v1/backup")
    );
}

/// **Without the artifact shape there is nothing to check a restore against**,
/// so a restore on its own names the missing key and a restore beside a feed
/// is a value this manifest cannot admit.
#[test]
fn a_restore_path_without_an_artifact_is_refused() {
    let route = sample_manifest()
        .with_proxy_path(Some("tempmon".to_string()))
        .expect("valid")
        .render();
    let alone = format!("{route}restore-path = \"api/v1/restore\"\n");
    match Manifest::parse(alone.as_bytes()) {
        Err(ManifestFault::MissingKey { key: "backup-path" }) => {}
        other => panic!("a restore with no backup must name backup-path, got {other:?}"),
    }

    let feed = sample_manifest()
        .with_proxy_path(Some("reliquary".to_string()))
        .expect("valid")
        .with_backup(Some(BackupSurface::Feed {
            path: "api/v1/blobs".to_string(),
        }))
        .expect("valid")
        .render();
    let beside_a_feed = format!("{feed}restore-path = \"api/v1/restore\"\n");
    match Manifest::parse(beside_a_feed.as_bytes()) {
        Err(ManifestFault::BadValue {
            key: "restore-path",
        }) => {}
        other => panic!("a restore beside a feed must be refused, got {other:?}"),
    }

    // Half the artifact is refused naming the artifact's own missing key, as
    // it was before `restore-path` existed.
    let half = format!(
        "{route}backup-path = \"api/v1/backup\"\nrestore-path = \"api/v1/restore\"\n"
    );
    match Manifest::parse(half.as_bytes()) {
        Err(ManifestFault::MissingKey {
            key: "backup-format",
        }) => {}
        other => panic!("half an artifact must still name backup-format, got {other:?}"),
    }
}

/// `backup-path`'s predicate, exactly — the same hostile list.
#[test]
fn a_restore_path_that_could_escape_its_application_is_refused() {
    let good = with_restore("api/v1/restore").render();
    for hostile in [
        "/api/v1/restore",
        "api/v1/restore/",
        "../etc/shadow",
        "api/../../state",
        "api//v1",
        "API/v1",
        "1api/v1",
        ".",
    ] {
        let text = good.replace("\"api/v1/restore\"", &format!("\"{hostile}\""));
        assert!(text != good, "the substitution for {hostile:?} did not happen");
        match Manifest::parse(text.as_bytes()) {
            Err(ManifestFault::BadValue { key: "restore-path" } | ManifestFault::BadCharacter) => {}
            other => panic!("{hostile:?} must be refused, got {other:?}"),
        }
        let built = sample_manifest().with_backup(Some(BackupSurface::Artifact {
            path: "api/v1/backup".to_string(),
            format: BackupFormat::Sqlite,
            restore: Some(hostile.to_string()),
        }));
        assert!(built.is_err(), "{hostile:?} should be refused by the builder");
    }
}

/// **A restore on a route the proxy serves to anybody is refused**, in every
/// spelling the proxy's longest-match can produce — and allowed in the one
/// where a deeper `operator` rule gates it on an otherwise open route.
#[test]
fn a_restore_path_the_proxy_would_serve_without_an_operator_is_refused() {
    let refused = |manifest: Manifest, why: &str| {
        let text = manifest.render();
        match Manifest::parse(text.as_bytes()) {
            Err(ManifestFault::BadValue {
                key: "restore-path",
            }) => {}
            other => panic!("{why}: expected restore-path refused, got {other:?}\n{text}"),
        }
    };
    refused(
        with_restore("api/v1/restore").with_proxy_auth(ProxyAuth::None),
        "the whole route open",
    );
    refused(
        with_restore("api/v1/restore")
            .with_proxy_auth_paths(vec![ProxyRule::new(ProxyAuth::None, "api").expect("valid")])
            .expect("valid"),
        "an open rule covering the restore",
    );
    refused(
        with_restore("api/v1/restore")
            .with_proxy_auth(ProxyAuth::None)
            .with_proxy_auth_paths(vec![
                ProxyRule::new(ProxyAuth::Operator, "api").expect("valid"),
                ProxyRule::new(ProxyAuth::None, "api/v1").expect("valid"),
            ])
            .expect("valid"),
        "a deeper open rule beating a gated one",
    );

    // Allowed: the route is open, and a deeper operator rule covers the
    // restore. Segment matching, not string matching: `api/v1/restore` is
    // covered by `api/v1/restore`, and `api/v1/restores` would not be.
    let gated = with_restore("api/v1/restore")
        .with_proxy_auth(ProxyAuth::None)
        .with_proxy_auth_paths(vec![
            ProxyRule::new(ProxyAuth::Operator, "api/v1/restore").expect("valid"),
        ])
        .expect("valid");
    assert!(gated.requires_operator_at("api/v1/restore"));
    assert!(!gated.requires_operator_at("api/v1/restores"));
    Manifest::parse(gated.render().as_bytes()).expect("a gated restore on an open route is legal");

    // And the default — no proxy-auth key at all — is gated.
    assert!(with_restore("api/v1/restore").requires_operator_at("api/v1/restore"));
    Manifest::parse(with_restore("api/v1/restore").render().as_bytes())
        .expect("the default route requires an operator");
}

/// A manifest with a restore and no route renders neither, for the backup
/// keys' reason — the builder does not check the pair, and the emitter must
/// not write a path relative to nothing.
#[test]
fn a_restore_with_no_route_is_not_rendered() {
    let text = sample_manifest()
        .with_backup(Some(BackupSurface::Artifact {
            path: "api/v1/backup".to_string(),
            format: BackupFormat::Sqlite,
            restore: Some("api/v1/restore".to_string()),
        }))
        .expect("valid")
        .render();
    assert!(!text.contains("restore-path"), "{text}");
    assert!(!text.contains("backup-path"), "{text}");
}

// ---------------------------------------------------------------------------
// `accepts-class` — PLAN-Feature-Set-2026-09-12 §6.2, ADR-053, made a list
// by `.rulings/` R-162 R28
// ---------------------------------------------------------------------------

/// **Absence means TAKES NO ADDON**, and it is the state every manifest
/// emitted before `ADR-053` is in — the same promise `settings` and
/// `proxy-path` made when they arrived. An empty list is the same state.
#[test]
fn a_manifest_that_names_no_class_accepts_no_addon_and_renders_no_key() {
    let manifest = sample_manifest();
    assert!(manifest.accepts_classes().is_empty());
    let text = manifest.render();
    assert!(
        !text.contains("accepts-class"),
        "an absent class must render NO key, or an old manifest stops round-tripping:\n{text}"
    );
    let back = Manifest::parse(text.as_bytes()).expect("a manifest naming no class must parse");
    assert!(back.accepts_classes().is_empty());
    assert_eq!(back.render(), text, "the artifact changed for an application that accepts nothing");
}

/// **This IS the old-format manifest**, proven directly rather than by
/// omission: bytes with no `accepts-class` line at all — exactly what every
/// `.rvt` signed before `ADR-053` carries — still parse.
#[test]
fn a_manifest_from_before_accepts_class_existed_still_parses() {
    let text = sample_manifest().render();
    assert!(!text.contains("accepts-class"), "the fixture must predate the key:\n{text}");
    let read = Manifest::parse(text.as_bytes()).expect("an older manifest must still parse");
    assert!(read.accepts_classes().is_empty());
}

/// A single declared class round-trips as a one-element list — the list form
/// ships alone (`.rulings/` R-162 R28), so even one class is written as
/// `["…"]`, never as a bare string.
#[test]
fn a_single_accepted_class_round_trips_as_a_one_element_list() {
    let text = sample_manifest()
        .with_accepts_classes(vec!["temperature-sensor".to_string()])
        .expect("valid")
        .render();
    assert!(text.contains("accepts-class = [\"temperature-sensor\"]\n"), "{text}");
    let back = Manifest::parse(text.as_bytes()).expect("round trip");
    assert_eq!(back.accepts_classes().to_vec(), vec!["temperature-sensor".to_string()]);
    assert_eq!(back.render(), text);
}

/// **R-162 R28's whole point**: two declared classes round-trip as a
/// two-element list, in the order they were declared.
#[test]
fn two_accepted_classes_round_trip_as_a_list() {
    let text = sample_manifest()
        .with_accepts_classes(vec![
            "temperature-sensor".to_string(),
            "humidity-sensor".to_string(),
        ])
        .expect("valid")
        .render();
    assert!(
        text.contains("accepts-class = [\"temperature-sensor\", \"humidity-sensor\"]\n"),
        "{text}"
    );
    let back = Manifest::parse(text.as_bytes()).expect("round trip");
    assert_eq!(
        back.accepts_classes().to_vec(),
        vec!["temperature-sensor".to_string(), "humidity-sensor".to_string()]
    );
    assert_eq!(back.render(), text);
}

/// **The rule is the same alphabet `settings` uses**, kept as a separate
/// predicate deliberately — see [`is_class_name`]'s own documentation.
#[test]
fn a_class_name_is_lowercase_letters_digits_and_hyphens() {
    for good in ["temperature-sensor", "pressure", "addon-2"] {
        assert!(is_class_name(good), "{good} should be a class name");
        assert!(
            sample_manifest()
                .with_accepts_classes(vec![good.to_string()])
                .is_ok(),
            "{good}"
        );
    }
    for bad in [
        "",
        "Temperature",
        "temperature_sensor",
        "temperature.sensor",
        "2sensors",
        "-leading",
        "with space",
        "café",
    ] {
        assert!(!is_class_name(bad), "{bad:?} should be refused");
        assert!(
            sample_manifest()
                .with_accepts_classes(vec![bad.to_string()])
                .is_err(),
            "{bad:?} reached a manifest"
        );
    }
}

// ---------------------------------------------------------------------------
// `material` — ADR-055 §3, `.rulings/` R-194
// ---------------------------------------------------------------------------

/// **Absence means NEEDS NOTHING PLACED**, and it is the state every manifest
/// emitted before `ADR-055` is in — `accepts-class`' promise, one key later.
#[test]
fn a_manifest_that_declares_no_material_needs_none_and_renders_no_key() {
    let manifest = sample_manifest();
    assert!(manifest.material().is_empty());
    let text = manifest.render();
    assert!(
        !text.contains("material"),
        "an absent declaration must render NO key, or an old manifest stops round-tripping:\n{text}"
    );
    let back = Manifest::parse(text.as_bytes()).expect("a manifest declaring no material parses");
    assert!(back.material().is_empty());
    assert_eq!(
        back.render(),
        text,
        "the artifact changed for an application that asked for nothing"
    );
}

/// The key parses, round-trips, and reads back as the kind that was declared.
#[test]
fn a_declared_kind_round_trips_as_a_one_element_list() {
    let text = sample_manifest()
        .with_material(vec![Material::FleetTls])
        .render();
    assert!(text.contains("material = [\"fleet-tls\"]\n"), "{text}");
    let back = Manifest::parse(text.as_bytes()).expect("round trip");
    assert_eq!(back.material(), [Material::FleetTls]);
    assert_eq!(back.render(), text);
}

/// **A kind this build does not define is REFUSED, naming the key** — the
/// posture `ADR-015` takes everywhere: an unknown token is an author's typo,
/// and skipping it would leave an application asking for material nothing ever
/// provides, with nothing saying so.
#[test]
fn a_material_kind_this_build_does_not_define_is_refused_by_key() {
    let good = sample_manifest()
        .with_material(vec![Material::FleetTls])
        .render();
    let hostile = good.replace("fleet-tls", "fleet-tsl");
    match Manifest::parse(hostile.as_bytes()) {
        Err(ManifestFault::BadValue { key: "material" }) => {}
        other => panic!("expected the refusal to name material, got {other:?}"),
    }
}

/// The three files are the PLATFORM's, in the order `cargo rivet secrets`
/// installs them and `ADR-049` §8 stages them.
#[test]
fn the_fleet_tls_kind_is_three_files_under_the_platforms_own_names() {
    assert_eq!(
        Material::FleetTls.files(),
        ["tls-cert", "tls-key", "tls-root"]
    );
    assert_eq!(Material::FleetTls.as_str(), "fleet-tls");
    assert_eq!(Material::parse("fleet-tls"), Some(Material::FleetTls));
    assert_eq!(Material::parse("tls-cert"), None, "a FILE is not a KIND");
}

/// `devices`' rule, and for its reason: the emitter sorts and dedups, so a
/// repeated kind is a second producer — and it would make *what did this ask
/// for* have two answers.
#[test]
fn a_repeated_kind_is_refused_and_the_builder_cannot_emit_one() {
    let text = sample_manifest()
        .with_material(vec![Material::FleetTls, Material::FleetTls])
        .render();
    assert!(text.contains("material = [\"fleet-tls\"]\n"), "{text}");
    let hostile = text.replace(
        "material = [\"fleet-tls\"]",
        "material = [\"fleet-tls\", \"fleet-tls\"]",
    );
    match Manifest::parse(hostile.as_bytes()) {
        Err(ManifestFault::BadValue { key: "material" }) => {}
        other => panic!("expected a repeated kind to be refused, got {other:?}"),
    }
}

/// **It is NOT guarded by the route.** The same pair is a client certificate
/// (`ADR-046` §4), so an application that serves nothing may still need it —
/// unlike `backup-path`, which names a route and is refused without one.
#[test]
fn material_is_declarable_without_a_proxy_path() {
    let text = sample_manifest()
        .with_material(vec![Material::FleetTls])
        .render();
    assert!(!text.contains("proxy-path"), "the fixture must have no route:\n{text}");
    let back = Manifest::parse(text.as_bytes()).expect("no route is not a reason to refuse");
    assert_eq!(back.material(), [Material::FleetTls]);
}

// ---------------------------------------------------------------------------
// `:direct` — ADR-046 §7, `.rulings/` R-188
// ---------------------------------------------------------------------------

/// Reliquary's shape: a routed administrative port first, and a phone's
/// listener after it.
fn reliquary_shape(listen: Vec<Socket>) -> Manifest {
    sample_manifest()
        .with_listen(listen)
        .expect("valid")
        .with_proxy_path(Some("reliquary".to_string()))
        .expect("valid")
}

/// **The grammar**: `:direct` after a TCP or UDP port, and nothing else after
/// one. A misspelling is refused rather than read as the ordinary reach, and a
/// unix name still may not hold a colon.
#[test]
fn a_direct_flag_parses_on_tcp_and_udp_and_nothing_else_does() {
    assert_eq!(parse_socket("tcp:8443:direct"), Ok(Socket::TcpDirect(8443)));
    assert_eq!(parse_socket("udp:53:direct"), Ok(Socket::UdpDirect(53)));
    for bad in [
        "tcp:8443:drect",
        "tcp:8443:",
        "tcp:8443:direct:direct",
        "tcp::direct",
        "tcp:0:direct",
        "tcp:08443:direct",
        "tcp:direct",
        "udp:53:public",
        "unix:api:direct",
    ] {
        assert_eq!(
            parse_socket(bad),
            Err(ManifestFault::BadValue { key: "listen" }),
            "{bad:?} must be refused"
        );
    }
}

/// **The flag is inside the signature**: it is part of `listen`'s rendered
/// line, so it is in the manifest member whose digest the header signs, and it
/// round-trips byte for byte.
#[test]
fn a_direct_port_round_trips_through_the_canonical_text() {
    let manifest = reliquary_shape(vec![Socket::Tcp(8444), Socket::TcpDirect(8443)]);
    let text = manifest.render();
    assert!(
        text.contains("listen = [\"tcp:8444\", \"tcp:8443:direct\"]\n"),
        "{text}"
    );
    let back = Manifest::parse(text.as_bytes()).expect("round trip");
    assert_eq!(back, manifest);
    assert_eq!(back.render(), text);
    assert_eq!(back.route_port(), Some(8444));
}

/// **The rule, one case per row** — `Socket::bind_address`.
#[test]
fn a_proxied_applications_ports_bind_loopback_unless_direct() {
    use std::net::Ipv4Addr;
    let loopback = Some(Ipv4Addr::LOCALHOST);
    let every = Some(Ipv4Addr::UNSPECIFIED);
    assert_eq!(Socket::Tcp(8444).bind_address(true), loopback);
    assert_eq!(Socket::Udp(5353).bind_address(true), loopback);
    assert_eq!(Socket::TcpDirect(8443).bind_address(true), every);
    assert_eq!(Socket::UdpDirect(53).bind_address(true), every);
    // No route: every address, as before R-188 — `rivet-proxyd`'s 443 and
    // `rivet-fleetd`'s 4443 are this row.
    assert_eq!(Socket::Tcp(443).bind_address(false), every);
    assert_eq!(Socket::Udp(53).bind_address(false), every);
    assert_eq!(Socket::Unix("api".to_string()).bind_address(true), None);
}

/// **A direct flag on the route's backend is refused** — it is the bypass the
/// ruling closed — in the parser a card-read manifest goes through, whatever
/// reach the later ports have. The backend is the first TCP port even when a
/// UDP or unix socket comes before it.
#[test]
fn a_direct_flag_on_the_routed_port_is_refused() {
    for listen in [
        vec![Socket::TcpDirect(8444), Socket::Tcp(8443)],
        vec![Socket::TcpDirect(8080)],
        vec![
            Socket::UdpDirect(53),
            Socket::Unix("api".to_string()),
            Socket::TcpDirect(8080),
        ],
    ] {
        let manifest = reliquary_shape(listen.clone());
        assert!(manifest.refuses_direct(), "{listen:?}");
        match Manifest::parse(manifest.render().as_bytes()) {
            Err(ManifestFault::BadValue { key: "listen" }) => {}
            other => panic!("{listen:?}: expected listen refused, got {other:?}"),
        }
    }
    // A direct UDP port beside an ordinary routed TCP port is legal.
    let legal = reliquary_shape(vec![Socket::UdpDirect(53), Socket::Tcp(8080)]);
    assert!(!legal.refuses_direct());
    Manifest::parse(legal.render().as_bytes()).expect("udp is never a route's backend");
}

/// **A direct flag with no route is refused**, `proxy-auth`'s footing: it is a
/// declaration attached to nothing, and a second spelling of a port that binds
/// every address anyway.
#[test]
fn a_direct_flag_without_a_proxy_path_is_refused() {
    let manifest = sample_manifest()
        .with_listen(vec![Socket::TcpDirect(443)])
        .expect("the builder does not check the pair");
    assert!(manifest.refuses_direct());
    match Manifest::parse(manifest.render().as_bytes()) {
        Err(ManifestFault::MissingKey { key: "proxy-path" }) => {}
        other => panic!("expected proxy-path named, got {other:?}"),
    }
}

/// **Every manifest signed before the flag still parses, and now means
/// loopback for a proxied application** — the ruling's *applications do not
/// change*. Nothing is refused for lacking the flag.
#[test]
fn a_manifest_from_before_the_flag_still_parses_and_its_routed_port_is_loopback() {
    let text = reliquary_shape(vec![Socket::Tcp(8444), Socket::Tcp(8443)]).render();
    assert!(!text.contains(":direct"), "{text}");
    let back = Manifest::parse(text.as_bytes()).expect("an older manifest must still parse");
    assert!(back.proxied());
    for socket in back.listen() {
        assert_eq!(
            socket.bind_address(back.proxied()),
            Some(std::net::Ipv4Addr::LOCALHOST)
        );
    }
}

/// The builder refuses the same class the parser would — `ADR-011`.
#[test]
fn the_builder_refuses_a_class_the_parser_would() {
    let text = sample_manifest().render() + "accepts-class = [\"Bad Class\"]\n";
    assert!(
        Manifest::parse(text.as_bytes()).is_err(),
        "a hand-written manifest smuggled a bad class past the parser"
    );
    assert!(
        sample_manifest()
            .with_accepts_classes(vec!["Bad Class".to_string()])
            .is_err()
    );
}
