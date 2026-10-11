//! What `crate::stage` composes, checked against the manifest `SPEC-05` §10
//! read back off a real fleet server rather than against what this module
//! believes it wrote.
//!
//! **Every test builds its own authority in a scratch directory.** Nothing here
//! reads `~/.rivet`, and nothing here touches a certificate store.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

use crate::ca::Authority;

/// A scratch directory that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let at = std::env::temp_dir().join(format!(
            "rivet-stage-{tag}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&at).expect("a scratch directory");
        Self(at)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An authority in a scratch directory, permitting one suffix.
fn authority(at: &Path, suffix: &str) -> PathBuf {
    let dir = at.join("ca");
    std::fs::create_dir_all(&dir).expect("a directory");
    let root = Authority::root("test fleet", &[suffix.to_owned()])
        .expect("a root")
        .pem()
        .expect("PEM");
    std::fs::write(dir.join("root.crt"), &root.certificate).expect("written");
    std::fs::write(dir.join("root.key"), &root.key).expect("written");
    dir
}

/// A device identity in a scratch directory: `device.crt` and a `peers` file,
/// and deliberately **no `device.seed`** — nothing in this module opens one, and
/// a test that provided one could not prove that.
fn identity(at: &Path, peers: &str) -> PathBuf {
    let dir = at.join("identity");
    std::fs::create_dir_all(&dir).expect("a directory");
    let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("a keypair");
    let params = rcgen::CertificateParams::new(vec!["a-board".to_owned()]).expect("params");
    let certificate = params.self_signed(&pair).expect("self-signed");
    std::fs::write(dir.join("device.crt"), certificate.der()).expect("written");
    std::fs::write(dir.join("peers"), peers).expect("written");
    dir
}

fn machine<'a>(fleet: &'a str, ca: &'a Path, id: Option<&'a Path>) -> Machine<'a> {
    Machine {
        fleet,
        hostname: "rivet-01",
        listen: "0.0.0.0:7777",
        method: "static",
        address: "192.168.50.50/24",
        gateway: "192.168.50.1",
        authority: ca,
        identity: id,
        staged_before: None,
        operator_key: None,
        fleet_root: None,
    }
}

fn named<'a>(plan: &'a Plan, app: &str, path: &str) -> Option<&'a Staged> {
    plan.items
        .iter()
        .find(|i| i.application == app && i.path == path)
}

/// **The manifest, item by item.** `SPEC-05` §10 lists what a hand did to make
/// a fleet server work; this is that list, composed at card time instead.
#[test]
fn a_fleet_server_stages_every_file_the_hand_installed() {
    let scratch = Scratch::new("manifest");
    let ca = authority(&scratch.0, "local");
    let id = identity(&scratch.0, "");
    let plan = compose(&machine("server", &ca, Some(&id))).expect("composed");

    for path in [
        "server.crt",
        "server.key",
        "ca.crt",
        "operator-device.seed.hex",
        "operator-device.crt.hex",
        "register/register.toml",
        "register/devices",
    ] {
        assert!(
            plan.items
                .iter()
                .any(|i| i.application == "rivet-fleetd" && i.path.starts_with(path)),
            "rivet-fleetd was not given {path}: {:?}",
            plan.items.iter().map(|i| &i.path).collect::<Vec<_>>()
        );
    }
    for path in ["tls-cert", "tls-key", "tls-root"] {
        assert!(
            named(&plan, "rivet-proxyd", path).is_some(),
            "rivet-proxyd was not given {path}"
        );
    }
}

/// `rivet-fleetd` reads these two as **hex text**, not as bytes
/// (`cmd/rivet-fleetd/src/node.rs`). Installing raw bytes under those names is
/// a file it cannot decode.
#[test]
fn the_operator_credential_is_hex_text_and_the_two_halves_agree() {
    let scratch = Scratch::new("hex");
    let ca = authority(&scratch.0, "local");
    let id = identity(&scratch.0, "");
    let plan = compose(&machine("server", &ca, Some(&id))).expect("composed");

    let seed = named(&plan, "rivet-fleetd", "operator-device.seed.hex").expect("a seed");
    let text = core::str::from_utf8(&seed.bytes).expect("text");
    assert_eq!(text.trim().len(), 64, "an Ed25519 seed is 64 hex characters");
    assert!(
        text.trim().chars().all(|c| c.is_ascii_hexdigit()),
        "the seed was written as bytes rather than as hex"
    );

    let certificate =
        named(&plan, "rivet-fleetd", "operator-device.crt.hex").expect("a certificate");
    let der: Vec<u8> = {
        let hex = core::str::from_utf8(&certificate.bytes).expect("text");
        let hex = hex.trim();
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()), "not hex");
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"))
            .collect()
    };

    // **The pair has to belong together**, and the way to check that without
    // the private key is to re-derive the public half from the seed and compare
    // it with the one in the certificate. A mismatched pair is what a wrongly
    // sliced PKCS#8 blob produces, and rustls reports it as `KeyMismatch` on
    // the board rather than here.
    let public = rivetos_tls::public_key_in_certificate(&der).expect("a public key");
    let seed_bytes: Vec<u8> = {
        let hex = text.trim();
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex"))
            .collect()
    };
    let rebuilt = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(
        &pkcs8_from_seed(&seed_bytes).into(),
        &rcgen::PKCS_ED25519,
    )
    .expect("the seed rebuilds a keypair");
    assert_eq!(
        rebuilt.public_key_raw(),
        public.as_slice(),
        "the staged seed and the staged certificate are different keys"
    );
}

/// A minimal PKCS#8 v1 wrapper around a raw Ed25519 seed, so a test can rebuild
/// a keypair from what was staged.
fn pkcs8_from_seed(seed: &[u8]) -> Vec<u8> {
    let mut out = vec![
        0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04,
        0x20,
    ];
    out.extend_from_slice(seed);
    out
}

/// The register is one record — its own — and it carries the pin, or nothing
/// can dial the machine.
#[test]
fn the_register_holds_one_record_and_it_carries_the_machines_own_pin() {
    let scratch = Scratch::new("register");
    let ca = authority(&scratch.0, "local");
    let id = identity(&scratch.0, "");
    let plan = compose(&machine("server", &ca, Some(&id))).expect("composed");

    let records: Vec<&Staged> = plan
        .items
        .iter()
        .filter(|i| i.path.starts_with("register/devices/"))
        .collect();
    assert_eq!(records.len(), 1, "a fleet server carries its OWN record only");

    let text = core::str::from_utf8(&records[0].bytes).expect("text");
    let der = std::fs::read(id.join("device.crt")).expect("the card's certificate");
    let pin = rivetos_tls::encode_hex(
        &rivetos_tls::public_key_in_certificate(&der).expect("a public key"),
    );
    assert!(text.contains(&pin), "the record carries no pin: {text}");
    assert!(text.contains("rivet-01"), "{text}");
    assert!(text.contains("0.0.0.0:7777"), "{text}");
    assert!(text.contains("192.168.50.50/24"), "{text}");
    // The role word `rivet-fleetd` finds its own control plane by.
    assert!(
        text.contains(&format!("role = \"{}\"", rivet_core::fleet_as_code::SERVER_ROLE)),
        "{text}"
    );

    // And it is filed under its own id, which is what `store::load` refuses a
    // record for if the two disagree.
    let filed = records[0]
        .path
        .trim_start_matches("register/devices/")
        .trim_end_matches(".toml");
    assert!(text.contains(filed), "the record is misfiled: {text}");
    assert!(rivetos_register::is_identifier(filed), "{filed}");
}

/// **What is staged reads back through the parser `cmd/rivet-image` will use.**
#[test]
fn everything_staged_survives_the_bundle_format() {
    let scratch = Scratch::new("roundtrip");
    let ca = authority(&scratch.0, "local");
    let id = identity(&scratch.0, "");
    let plan = compose(&machine("server", &ca, Some(&id))).expect("composed");

    let text = rivet_core::staging::render(&plan.items);
    let back = rivet_core::staging::parse(&text).expect("a bundle");
    assert_eq!(back.len(), plan.items.len());
    for item in &plan.items {
        assert!(
            back.iter()
                .any(|b| b.application == item.application
                    && b.path == item.path
                    && b.bytes == item.bytes),
            "{} {} did not survive",
            item.application,
            item.path
        );
    }
}

/// **The peers file only ever gains a line.** Dropping one from a headless
/// board's card is a lockout with no recovery but a new card.
#[test]
fn the_peers_file_gains_the_fleet_servers_key_and_keeps_every_other() {
    let scratch = Scratch::new("peers");
    let ca = authority(&scratch.0, "local");
    let mine = "a".repeat(64);
    let id = identity(&scratch.0, &format!("{mine}\n"));

    let first = compose(&machine("server", &ca, Some(&id))).expect("composed");

    let peers = std::fs::read_to_string(id.join("peers")).expect("read back");
    let keys: Vec<&str> = peers.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(keys.contains(&mine.as_str()), "an existing key was dropped: {peers}");
    assert_eq!(keys.len(), 2, "exactly one key should have been added: {peers}");
    assert!(keys.iter().all(|k| k.len() == 64), "{peers}");

    // **A second `w` for the same machine adds nothing** (`.rulings/` R-189):
    // it reuses the operator credential the first one staged, so the key is
    // already there.
    let bundle = rivet_core::staging::render(&first.items);
    let mut again_machine = machine("server", &ca, Some(&id));
    again_machine.staged_before = Some(&bundle);
    let second = compose(&again_machine).expect("composed again");
    let again = std::fs::read_to_string(id.join("peers")).expect("read back");
    assert_eq!(again, peers, "a second `w` changed the peers file");
    assert_eq!(
        named(&second, "rivet-fleetd", "operator-device.seed.hex").map(|s| &s.bytes),
        named(&first, "rivet-fleetd", "operator-device.seed.hex").map(|s| &s.bytes),
        "the operator credential was not reused"
    );
}

/// A bundle whose operator halves are not one key is not reused: a fresh
/// credential is minted rather than a pair the machine would refuse to serve.
#[test]
fn a_mismatched_staged_operator_is_not_reused() {
    let scratch = Scratch::new("mismatch");
    let ca = authority(&scratch.0, "local");
    let id = identity(&scratch.0, "");
    let one = compose(&machine("server", &ca, Some(&id))).expect("composed");
    let two = compose(&machine("server", &ca, Some(&id))).expect("composed");
    let mut items: Vec<Staged> = one.items.clone();
    for item in &mut items {
        if item.path == "operator-device.crt.hex" {
            item.bytes = named(&two, "rivet-fleetd", "operator-device.crt.hex")
                .expect("a certificate")
                .bytes
                .clone();
        }
    }
    let bundle = rivet_core::staging::render(&items);
    let mut again = machine("server", &ca, Some(&id));
    again.staged_before = Some(&bundle);
    let three = compose(&again).expect("composed");
    let seed = |plan: &Plan| named(plan, "rivet-fleetd", "operator-device.seed.hex").map(|s| s.bytes.clone());
    assert_ne!(seed(&three), seed(&one), "a mismatched pair was reused");
}

/// A stand-alone machine carries no register and no front door. Ruled
/// 2026-09-07: the workstation owns its register, and a second copy would be
/// the second authority `ADR-034` dropped Local mode to avoid.
#[test]
fn a_stand_alone_machine_stages_nothing_and_says_why() {
    let scratch = Scratch::new("standalone");
    let ca = authority(&scratch.0, "local");
    let id = identity(&scratch.0, "");
    let plan = compose(&machine("stand-alone", &ca, Some(&id))).expect("composed");
    assert!(plan.items.is_empty(), "stand-alone staged something: {:?}", plan.items);
    assert!(
        plan.notes.iter().any(|n| n.contains("nothing staged")),
        "stand-alone said nothing about it: {:?}",
        plan.notes
    );
}

/// **R-160 R25: a member gets its own leaf certificate and key from the fleet
/// CA, for its own name, plus the same public root — exactly the three files a
/// server's front door gets, and nothing else.**
#[test]
fn a_member_stages_exactly_the_three_proxy_files_for_its_own_name() {
    let scratch = Scratch::new("member");
    let ca = authority(&scratch.0, "local");
    let mut member = machine("member", &ca, None);
    member.hostname = "member-01";
    let plan = compose(&member).expect("composed");

    assert_eq!(
        plan.items.len(),
        3,
        "a member should stage only its front door: {:?}",
        plan.items
            .iter()
            .map(|i| (&i.application, &i.path))
            .collect::<Vec<_>>()
    );
    for path in ["tls-cert", "tls-key", "tls-root"] {
        assert!(
            named(&plan, "rivet-proxyd", path).is_some(),
            "a member was not given {path}"
        );
    }
    assert!(
        !plan.items.iter().any(|i| i.application == "rivet-fleetd"),
        "a member was given rivet-fleetd files, which are server-only"
    );
    assert!(
        !plan.items.iter().any(|i| i.path.starts_with("register/")),
        "a member was given a register, which is server-only"
    );

    let cert = named(&plan, "rivet-proxyd", "tls-cert").expect("a certificate");
    let pem = core::str::from_utf8(&cert.bytes).expect("text");
    assert!(pem.starts_with("-----BEGIN CERTIFICATE-----"), "{pem}");
    assert!(
        plan.notes.iter().any(|n| n.contains("member-01.local")),
        "{:?}",
        plan.notes
    );
}

/// **A member joining a Fleet whose operator key is known gains it in its
/// peers, and keeps the human operator.** `docs/PLAN.md` §183.2: without this
/// line the fleet server can list the member but never manage it — the real
/// "enrol never worked". The peers file only ever gains a line, like the
/// server's own.
#[test]
fn a_member_with_a_known_fleet_operator_key_adds_it_to_its_peers() {
    let scratch = Scratch::new("member-peers-known");
    let ca = authority(&scratch.0, "local");
    let human = "a".repeat(64);
    let id = identity(&scratch.0, &format!("{human}\n"));
    let fleet_key = "b".repeat(64);

    let mut member = machine("member", &ca, Some(&id));
    member.hostname = "member-01";
    member.operator_key = Some(&fleet_key);
    let plan = compose(&member).expect("composed");

    // Still only its three front-door files; the peers change is on disk, not
    // in the staged bundle.
    assert_eq!(plan.items.len(), 3, "{:?}", plan.items.iter().map(|i| &i.path).collect::<Vec<_>>());

    let peers = std::fs::read_to_string(id.join("peers")).expect("read back");
    let keys: Vec<&str> = peers.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(keys.contains(&human.as_str()), "the human operator was dropped: {peers}");
    assert!(keys.contains(&fleet_key.as_str()), "the fleet server key was not added: {peers}");
    assert_eq!(keys.len(), 2, "exactly one key should have been added: {peers}");
    assert!(
        plan.notes.iter().any(|n| n.contains("fleet server's operator key")),
        "{:?}",
        plan.notes
    );

    // Additive-only and idempotent: composing again adds nothing.
    let mut again = machine("member", &ca, Some(&id));
    again.hostname = "member-01";
    again.operator_key = Some(&fleet_key);
    compose(&again).expect("composed again");
    let after = std::fs::read_to_string(id.join("peers")).expect("read back");
    assert_eq!(after, peers, "a second compose changed the member's peers");
}

/// **A member whose Fleet key is not known stages its peers exactly as before
/// this existed — the human operator alone.** The backward-compatibility
/// guarantee: an unknown Fleet, or a server provisioned before the key was
/// recorded, must not change a member's peers at all.
#[test]
fn a_member_with_no_fleet_operator_key_leaves_its_peers_as_it_was() {
    let scratch = Scratch::new("member-peers-unknown");
    let ca = authority(&scratch.0, "local");
    let human = "a".repeat(64);
    let id = identity(&scratch.0, &format!("{human}\n"));

    let mut member = machine("member", &ca, Some(&id));
    member.hostname = "member-01";
    // operator_key stays None (the machine() default).
    let plan = compose(&member).expect("composed");

    let peers = std::fs::read_to_string(id.join("peers")).expect("read back");
    let keys: Vec<&str> = peers.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(keys, vec![human.as_str()], "the peers file was changed: {peers}");
    assert!(
        plan.notes.iter().any(|n| n.contains("only the human operator")),
        "{:?}",
        plan.notes
    );
}

/// A known fleet key with no identity directory writes no peers file and says
/// so, rather than panicking or inventing a location.
#[test]
fn a_member_with_a_key_but_no_identity_writes_no_peers_and_names_the_gap() {
    let scratch = Scratch::new("member-peers-noidentity");
    let ca = authority(&scratch.0, "local");
    let fleet_key = "b".repeat(64);
    let mut member = machine("member", &ca, None);
    member.hostname = "member-01";
    member.operator_key = Some(&fleet_key);
    let plan = compose(&member).expect("composed");
    assert_eq!(plan.items.len(), 3, "{:?}", plan.items);
    assert!(
        plan.notes.iter().any(|n| n.contains("no identity directory")),
        "{:?}",
        plan.notes
    );
}

/// **A server ignores `Machine::operator_key`.** The server branch adds its
/// OWN minted operator key to its OWN peers; a stray value on the field must
/// not reach a server's peers as a second, foreign key. The server's peers
/// gains exactly one key — its own.
#[test]
fn a_server_ignores_the_machine_operator_key_field() {
    let scratch = Scratch::new("server-ignores-field");
    let ca = authority(&scratch.0, "local");
    let human = "a".repeat(64);
    let id = identity(&scratch.0, &format!("{human}\n"));
    let stray = "b".repeat(64);

    let mut server = machine("server", &ca, Some(&id));
    server.operator_key = Some(&stray);
    compose(&server).expect("composed");

    let peers = std::fs::read_to_string(id.join("peers")).expect("read back");
    let keys: Vec<&str> = peers.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(!keys.contains(&stray.as_str()), "a server staged the foreign field key: {peers}");
    assert!(keys.contains(&human.as_str()), "{peers}");
    // Its own minted operator key is the only addition — two keys total.
    assert_eq!(keys.len(), 2, "{peers}");
}

/// A server's plan is unchanged by R25: it still gets every file the manifest
/// names, through the same three-proxy-file staging a member now shares.
#[test]
fn a_servers_plan_is_unchanged_by_a_members_front_door() {
    let scratch = Scratch::new("server-unchanged");
    let ca = authority(&scratch.0, "local");
    let id = identity(&scratch.0, "");
    let plan = compose(&machine("server", &ca, Some(&id))).expect("composed");

    for path in [
        "server.crt",
        "server.key",
        "ca.crt",
        "operator-device.seed.hex",
        "operator-device.crt.hex",
    ] {
        assert!(
            named(&plan, "rivet-fleetd", path).is_some(),
            "a server lost {path}"
        );
    }
    for path in ["tls-cert", "tls-key", "tls-root"] {
        assert!(
            named(&plan, "rivet-proxyd", path).is_some(),
            "a server lost {path}"
        );
    }
    assert!(
        plan.items.iter().any(|i| i.path.starts_with("register/devices/")),
        "a server lost its register"
    );
}

/// A member's key is its own, never the server's — even from the same
/// authority. The public root, though, is shared, because that is what lets
/// one operator certificate open every machine's pages (`ADR-046`).
#[test]
fn a_members_key_is_not_the_servers_and_the_root_is_shared() {
    let scratch = Scratch::new("member-key");
    let ca = authority(&scratch.0, "local");
    let id = identity(&scratch.0, "");

    let server_plan = compose(&machine("server", &ca, Some(&id))).expect("composed");
    let mut member = machine("member", &ca, None);
    member.hostname = "member-01";
    let member_plan = compose(&member).expect("composed");

    let server_key = named(&server_plan, "rivet-proxyd", "tls-key").expect("a server key");
    let member_key = named(&member_plan, "rivet-proxyd", "tls-key").expect("a member key");
    assert_ne!(
        server_key.bytes, member_key.bytes,
        "a member was issued the server's own key"
    );

    let server_cert = named(&server_plan, "rivet-proxyd", "tls-cert").expect("a server cert");
    let member_cert = named(&member_plan, "rivet-proxyd", "tls-cert").expect("a member cert");
    assert_ne!(
        server_cert.bytes, member_cert.bytes,
        "a member was issued the server's own certificate"
    );

    let server_root = named(&server_plan, "rivet-proxyd", "tls-root").expect("a server root");
    let member_root = named(&member_plan, "rivet-proxyd", "tls-root").expect("a member root");
    assert_eq!(
        server_root.bytes, member_root.bytes,
        "the public root must be the same for every machine's front door"
    );
}

/// **The seed is never opened.** The public half is in the certificate; a tool
/// that reads a private key it does not need is one whose failures can leak it.
#[test]
fn no_device_seed_is_needed_or_read() {
    let scratch = Scratch::new("noseed");
    let ca = authority(&scratch.0, "local");
    let id = identity(&scratch.0, "");
    assert!(
        !id.join("device.seed").exists(),
        "the fixture provided a seed, so this test proves nothing"
    );
    compose(&machine("server", &ca, Some(&id))).expect("composed without a seed present");
}

/// Without an identity directory there is no pin and no peers entry, and the
/// tool says so rather than composing a register nothing can dial.
#[test]
fn a_server_with_no_identity_stages_credentials_and_names_what_is_missing() {
    let scratch = Scratch::new("noidentity");
    let ca = authority(&scratch.0, "local");
    let plan = compose(&machine("server", &ca, None)).expect("composed");

    assert!(
        !plan.items.iter().any(|i| i.path.starts_with("register/")),
        "a register was staged with no pin in it"
    );
    assert!(named(&plan, "rivet-fleetd", "server.crt").is_some());
    assert!(
        plan.notes.iter().any(|n| n.contains("NO REGISTER")),
        "{:?}",
        plan.notes
    );
}

/// No authority is a refusal with the path in it, not a card that boots without
/// certificates.
#[test]
fn no_authority_is_refused_by_path() {
    let scratch = Scratch::new("noca");
    let ca = scratch.0.join("nothing-here");
    let err = compose(&machine("server", &ca, None)).expect_err("refused");
    assert!(err.contains("root.crt"), "{err}");
}

/// Every certificate a plan stages, as DER, with where it was staged.
fn staged_certificates(plan: &Plan) -> Vec<(String, Vec<u8>)> {
    plan.items
        .iter()
        .filter_map(|item| {
            let text = core::str::from_utf8(&item.bytes).ok()?;
            // Only a certificate: `ca.crt` and `tls-root` are the authority's
            // own, which names no host because it is not a leaf.
            if !text.starts_with("-----BEGIN CERTIFICATE-----")
                || matches!(item.path.as_str(), "ca.crt" | "tls-root")
            {
                return None;
            }
            let (_, pem) =
                x509_parser::pem::parse_x509_pem(&item.bytes).expect("a PEM certificate");
            Some((format!("{} {}", item.application, item.path), pem.contents))
        })
        .collect()
}

/// **The pin, per role: every machine certificate provisioning stages names a
/// host** — a server's two front doors (`rivet-fleetd`'s `server.crt` and
/// `rivet-proxyd`'s `tls-cert`), a member's one, and a stand-alone machine's
/// none. `rivet-fleetd` and `rivet-proxyd` take a certificate naming no host for
/// the operator's (`.rulings/` R-187), so a role that staged one would hand
/// every machine carrying it the operator's routes.
#[test]
fn every_machine_certificate_staged_for_every_role_names_its_host() {
    let scratch = Scratch::new("names-a-host");
    let ca = authority(&scratch.0, "local");
    let id = identity(&scratch.0, "");

    let mut member = machine("member", &ca, None);
    member.hostname = "member-01";
    let server = compose(&machine("server", &ca, Some(&id))).expect("composed");
    let stand_alone = compose(&machine("stand-alone", &ca, Some(&id))).expect("composed");
    for (role, plan, expected) in [
        ("server", server, 2),
        ("member", compose(&member).expect("composed"), 1),
        ("stand-alone", stand_alone, 0),
    ] {
        let certificates = staged_certificates(&plan);
        let staged: Vec<&String> = certificates.iter().map(|c| &c.0).collect();
        assert_eq!(certificates.len(), expected, "{role}: {staged:?}");
        for (where_, der) in certificates {
            match rivetos_tls::classify_caller(&der) {
                rivetos_tls::Caller::Machine(names) => {
                    let host = if role == "member" { "member-01.local" } else { "rivet-01.local" };
                    assert_eq!(names, vec![host.to_owned()], "{role}: {where_}");
                }
                other => panic!("{role}: {where_} is {other:?}, and would pass as the operator"),
            }
        }
    }
}

/// The certificate staged is the one a browser will be shown, and it answers to
/// every name the authority permits.
#[test]
fn the_staged_certificate_carries_the_fleet_name() {
    let scratch = Scratch::new("names");
    let ca = authority(&scratch.0, "local");
    let plan = compose(&machine("server", &ca, None)).expect("composed");
    let cert = named(&plan, "rivet-fleetd", "server.crt").expect("a certificate");
    let pem = core::str::from_utf8(&cert.bytes).expect("text");
    assert!(pem.starts_with("-----BEGIN CERTIFICATE-----"), "{pem}");
    assert!(
        plan.notes.iter().any(|n| n.contains("rivet-01.local")),
        "{:?}",
        plan.notes
    );

    // And `rivet-proxyd` gets the SAME certificate, under its own names — two
    // certificates for one machine is a browser trusting one page and not
    // another.
    let proxy = named(&plan, "rivet-proxyd", "tls-cert").expect("the proxy's copy");
    assert_eq!(proxy.bytes, cert.bytes);
}

// ---------------------------------------------------------------------------
// Fix 2 — `compose` carries the machine's own leaf out for the workstation to
// keep a copy of, and `crate::write_server_leaf` (shared with
// `rivet-provision ca server`) is what actually writes it.
// ---------------------------------------------------------------------------

/// **A server's `Plan` carries the same leaf it staged onto the card**,
/// named for the workstation to write under `server-<fleet name>.crt`/`.key`
/// — not a second, independently-keyed mint.
#[test]
fn a_server_plan_carries_its_own_leaf_out_for_the_workstation() {
    let scratch = Scratch::new("workstation-leaf-server");
    let ca = authority(&scratch.0, "local");
    let id = identity(&scratch.0, "");
    let plan = compose(&machine("server", &ca, Some(&id))).expect("composed");

    let (subject, leaf) = plan.workstation_leaf.as_ref().expect("a server mints a leaf");
    assert_eq!(subject, "rivet-01.local");

    // The SAME bytes staged onto the card for `rivet-proxyd`'s `tls-cert` —
    // proof this is the one leaf carried out, not a fresh mint.
    let staged = named(&plan, "rivet-proxyd", "tls-cert").expect("the proxy's own leaf");
    assert_eq!(leaf.certificate.as_bytes(), staged.bytes.as_slice());
}

/// **A member's `Plan` carries one too** — Fix 2 is not server-only, because
/// `cargo rivet fleet` deploys to members, and a member's leaf is exactly what
/// its "stage material (fleet-tls)" step needs on the workstation.
#[test]
fn a_member_plan_carries_its_own_leaf_out_for_the_workstation() {
    let scratch = Scratch::new("workstation-leaf-member");
    let ca = authority(&scratch.0, "local");
    let mut member = machine("member", &ca, None);
    member.hostname = "member-01";
    let plan = compose(&member).expect("composed");

    let (subject, leaf) = plan.workstation_leaf.as_ref().expect("a member mints a leaf too");
    assert_eq!(subject, "member-01.local");
    let staged = named(&plan, "rivet-proxyd", "tls-cert").expect("the proxy's own leaf");
    assert_eq!(leaf.certificate.as_bytes(), staged.bytes.as_slice());
}

/// A stand-alone machine mints no leaf at all — `compose` returns before ever
/// reading the authority — so there is nothing for the workstation to keep a
/// copy of either.
#[test]
fn a_stand_alone_plan_carries_no_workstation_leaf() {
    let scratch = Scratch::new("workstation-leaf-standalone");
    let ca = authority(&scratch.0, "local");
    let id = identity(&scratch.0, "");
    let plan = compose(&machine("stand-alone", &ca, Some(&id))).expect("composed");
    assert!(plan.workstation_leaf.is_none());
}

/// **What `write_server_leaf` (shared with `rivet-provision ca server`,
/// `crate::mint_server` via `crate::write_server_leaf`) actually leaves on
/// disk**: `server-<subject>.crt` and `server-<subject>.key`, holding exactly
/// the leaf's own PEM — the two files `cargo rivet fleet`'s "stage material
/// (fleet-tls)" step reads at `~/.rivet/ca/server-<name>.crt`/`.key`.
#[test]
fn write_server_leaf_writes_the_two_files_a_deploy_reads() {
    let scratch = Scratch::new("write-server-leaf");
    let ca = authority(&scratch.0, "local");
    let id = identity(&scratch.0, "");
    let plan = compose(&machine("server", &ca, Some(&id))).expect("composed");
    let (subject, leaf) = plan.workstation_leaf.as_ref().expect("a leaf");

    let written = crate::write_server_leaf(&ca, subject, leaf).expect("writes");
    assert_eq!(written.len(), 2, "{written:?}");

    let crt = std::fs::read_to_string(ca.join(format!("server-{subject}.crt"))).expect("crt written");
    let key = std::fs::read_to_string(ca.join(format!("server-{subject}.key"))).expect("key written");
    assert_eq!(crt, leaf.certificate);
    assert_eq!(key, leaf.key);
}
