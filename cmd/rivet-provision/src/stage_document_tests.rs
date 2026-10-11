//! `rivet-provision stage`, driven from a document on disk — one test per
//! role, and the refusals named.
//!
//! **Every test builds its own authority, identity and Armada in a scratch
//! directory.** Nothing here reads `~/.rivet`, and nothing here touches a
//! certificate store.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

use crate::ca::Authority;

/// A scratch credential root that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let at = std::env::temp_dir().join(format!(
            "rivet-stage-document-{tag}-{}-{:?}",
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

/// `<root>/ca`, permitting `local`.
fn authority(root: &Path) {
    let dir = root.join("ca");
    std::fs::create_dir_all(&dir).expect("a directory");
    let minted = Authority::root("test fleet", &["local".to_owned()])
        .expect("a root")
        .pem()
        .expect("PEM");
    std::fs::write(dir.join("root.crt"), &minted.certificate).expect("written");
    std::fs::write(dir.join("root.key"), &minted.key).expect("written");
}

/// A machine identity whose peers file names one operator key, and no seed —
/// nothing here opens one.
fn identity(root: &Path, host: &str, operator: &str) -> PathBuf {
    let dir = root.join("identity").join(host);
    std::fs::create_dir_all(&dir).expect("a directory");
    let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("a keypair");
    let params = rcgen::CertificateParams::new(vec!["rivet-device".to_owned()]).expect("params");
    let certificate = params.self_signed(&pair).expect("self-signed");
    std::fs::write(dir.join("device.crt"), certificate.der()).expect("written");
    std::fs::write(dir.join("peers"), format!("{operator}\n")).expect("written");
    dir
}

/// A fictional operator key — 64 hex characters, the shape a peers line is.
const OPERATOR: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// A kiosk fleet server on a static address, as the interview composes one.
const SERVER: &str = "\
version = 1
hostname = \"example-server\"
timezone = \"America/Chicago\"

[network.eth0]
method = \"static\"
address = \"192.0.2.10/24\"
gateway = \"192.0.2.1\"

[control-plane]
listen = \"0.0.0.0:7777\"

[kiosk]
auto-run = \"rivet-launcher\"

[applications.rivet-proxyd]
upstream-name = \"example-server.local\"

[services.rivet-displayd]
enabled = true

[services.rivet-fleetd]
enabled = true

[services.rivet-launcher]
enabled = true

[services.rivet-mdnsd]
enabled = true

[services.rivet-proxyd]
enabled = true
";

/// A kiosk fleet member on DHCP.
const MEMBER: &str = "\
version = 1
hostname = \"example-member\"
timezone = \"America/Chicago\"

[network.eth0]
method = \"dhcp\"

[control-plane]
listen = \"0.0.0.0:7777\"

[kiosk]
auto-run = \"rivet-launcher\"

[applications.rivet-proxyd]
upstream-name = \"example-member.local\"

[services.rivet-displayd]
enabled = true

[services.rivet-launcher]
enabled = true

[services.rivet-mdnsd]
enabled = true

[services.rivet-proxyd]
enabled = true
";

fn write_document(root: &Path, host: &str, text: &str) -> PathBuf {
    let dir = root.join("provision");
    std::fs::create_dir_all(&dir).expect("a directory");
    let path = dir.join(format!("{host}.provisioning.toml"));
    std::fs::write(&path, text).expect("written");
    path
}

fn read_bundle(path: &Path) -> Vec<Staged> {
    let text = std::fs::read_to_string(path).expect("the bundle is on disk");
    rivet_core::staging::parse(&text).expect("the bundle parses")
}

fn has(items: &[Staged], app: &str, path: &str) -> bool {
    items.iter().any(|i| i.application == app && i.path == path)
}

fn peers(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("peers"))
        .expect("peers")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

/// **A fleet server's document stages everything `w` would have**: the
/// front door's triple, fleetd's five files, a one-record register naming the
/// server itself, and a peers file that now names fleetd's operator key.
#[test]
fn a_server_document_is_staged_with_fleetd_the_proxy_and_its_own_register() {
    let scratch = Scratch::new("server");
    authority(&scratch.0);
    let id = identity(&scratch.0, "example-server", OPERATOR);
    let document = write_document(&scratch.0, "example-server", SERVER);

    let report = stage(&document, &id, &scratch.0).expect("staged");
    assert_eq!(report.role, Some("server"));
    let bundle = report.path.clone().expect("a bundle was written");
    assert_eq!(bundle, scratch.0.join("provision").join("example-server.staged.txt"));

    let items = read_bundle(&bundle);
    for file in ["tls-cert", "tls-key", "tls-root"] {
        assert!(has(&items, "rivet-proxyd", file), "rivet-proxyd has no {file}");
    }
    for file in [
        "server.crt",
        "server.key",
        "ca.crt",
        "operator-device.seed.hex",
        "operator-device.crt.hex",
    ] {
        assert!(has(&items, "rivet-fleetd", file), "rivet-fleetd has no {file}");
    }
    let records: Vec<&Staged> =
        items.iter().filter(|i| i.path.starts_with("register/devices/")).collect();
    assert_eq!(records.len(), 1, "a fresh server's register holds itself only");
    let record = core::str::from_utf8(&records[0].bytes).expect("text");
    assert!(record.contains("example-server"), "{record}");
    assert!(record.contains("192.0.2.10"), "the static address is recorded: {record}");

    // The operator key fleetd presents is in its register and in the peers
    // file the card will carry, beside the human operator's.
    let certificate = items
        .iter()
        .find(|i| i.path == "operator-device.crt.hex")
        .expect("the operator certificate");
    let der = decode_hex(core::str::from_utf8(&certificate.bytes).expect("hex text").trim())
        .expect("hex");
    let fleetd_key =
        rivetos_tls::encode_hex(&rivetos_tls::public_key_in_certificate(&der).expect("a key"));
    let listed = peers(&id);
    assert!(listed.contains(&OPERATOR.to_owned()), "the operator is kept: {listed:?}");
    assert!(listed.contains(&fleetd_key), "fleetd's key is added: {listed:?}");
    assert_eq!(listed.len(), 2, "{listed:?}");

    // Names and sizes are printed; no private byte is.
    let seed = items.iter().find(|i| i.path == "operator-device.seed.hex").expect("the seed");
    let seed_text = core::str::from_utf8(&seed.bytes).expect("text").trim().to_owned();
    let printed = report.lines.join("\n");
    assert!(printed.contains("rivet-fleetd/operator-device.seed.hex"), "{printed}");
    assert!(!printed.contains(&seed_text), "the seed was printed");
    let key = items.iter().find(|i| i.path == "tls-key").expect("the key");
    let key_text = core::str::from_utf8(&key.bytes).expect("PEM");
    let key_body = key_text.lines().nth(1).expect("a PEM body line");
    assert!(!printed.contains(key_body), "the proxy's private key was printed");

    // The workstation keeps this machine's leaf, as `w` does.
    assert!(scratch.0.join("ca").join("server-example-server.local.crt").is_file());
}

/// **A second run reuses the operator credential**, so peers gains no second
/// fleetd key — `w`'s own rule, reached from a bundle already beside the
/// document.
#[test]
fn staging_a_server_again_reuses_its_operator_credential() {
    let scratch = Scratch::new("again");
    authority(&scratch.0);
    let id = identity(&scratch.0, "example-server", OPERATOR);
    let document = write_document(&scratch.0, "example-server", SERVER);
    stage(&document, &id, &scratch.0).expect("staged once");
    let first = peers(&id);
    stage(&document, &id, &scratch.0).expect("staged twice");
    assert_eq!(peers(&id), first);
}

/// **A fleet member's document stages the front door only**, and its peers
/// file gains its fleet server's operator key, read from the Armada under
/// `--at`.
#[test]
fn a_member_document_is_staged_with_the_proxy_and_trusts_its_server() {
    let scratch = Scratch::new("member");
    authority(&scratch.0);
    let server_key = "fe".repeat(32);
    let mut armada = rivetos_register::armada::Armada::empty(scratch.0.join("armada"));
    armada.fleets.insert(
        "example-server".to_owned(),
        rivetos_register::armada::Fleet {
            name: "example-server".to_owned(),
            server: rivetos_register::armada::Server {
                host: "192.0.2.10:7777".to_owned(),
                pin: "ab".repeat(32),
                operator_key: Some(server_key.clone()),
            },
        },
    );
    rivetos_register::armada::store::save(&armada).expect("an Armada");
    let id = identity(&scratch.0, "example-member", OPERATOR);
    let document = write_document(&scratch.0, "example-member", MEMBER);

    let report = stage(&document, &id, &scratch.0).expect("staged");
    assert_eq!(report.role, Some("member"));
    let items = read_bundle(&report.path.expect("a bundle"));
    for file in ["tls-cert", "tls-key", "tls-root"] {
        assert!(has(&items, "rivet-proxyd", file), "rivet-proxyd has no {file}");
    }
    assert!(
        items.iter().all(|i| i.application == "rivet-proxyd"),
        "a member stages nothing for fleetd: {:?}",
        items.iter().map(|i| format!("{}/{}", i.application, i.path)).collect::<Vec<_>>()
    );
    let listed = peers(&id);
    assert_eq!(listed, vec![OPERATOR.to_owned(), server_key], "{listed:?}");
}

/// A member with no Armada is still staged — its front door needs the
/// certificate either way — and the note says the server's key is missing.
#[test]
fn a_member_with_no_armada_is_staged_and_says_its_server_cannot_manage_it() {
    let scratch = Scratch::new("member-alone");
    authority(&scratch.0);
    let id = identity(&scratch.0, "example-member", OPERATOR);
    let document = write_document(&scratch.0, "example-member", MEMBER);
    let report = stage(&document, &id, &scratch.0).expect("staged");
    assert_eq!(peers(&id), vec![OPERATOR.to_owned()]);
    assert!(report.lines.join("\n").contains("no fleet server operator key"), "{:?}", report.lines);
}

/// A document named some other way gets `<hostname>.staged.txt` beside it.
#[test]
fn a_hand_named_document_gets_a_bundle_named_for_its_host() {
    let scratch = Scratch::new("hand-named");
    authority(&scratch.0);
    let id = identity(&scratch.0, "example-member", OPERATOR);
    let document = scratch.0.join("dev.toml");
    std::fs::write(&document, MEMBER).expect("written");
    let report = stage(&document, &id, &scratch.0).expect("staged");
    assert_eq!(report.path, Some(scratch.0.join("example-member.staged.txt")));
}

#[test]
fn a_missing_authority_is_refused_by_name_and_nothing_is_written() {
    let scratch = Scratch::new("no-ca");
    let id = identity(&scratch.0, "example-server", OPERATOR);
    let document = write_document(&scratch.0, "example-server", SERVER);
    let why = stage(&document, &id, &scratch.0).expect_err("no authority");
    assert!(why.contains("no certificate authority at"), "{why}");
    assert!(why.contains("root.crt"), "{why}");
    assert!(!scratch.0.join("provision").join("example-server.staged.txt").exists());
    assert_eq!(peers(&id), vec![OPERATOR.to_owned()], "peers was touched");
}

#[test]
fn a_missing_identity_is_refused_by_name_and_nothing_is_written() {
    let scratch = Scratch::new("no-identity");
    authority(&scratch.0);
    let document = write_document(&scratch.0, "example-member", MEMBER);
    let absent = scratch.0.join("identity").join("nobody");
    let why = stage(&document, &absent, &scratch.0).expect_err("no identity");
    assert!(why.contains("no identity at"), "{why}");
    assert!(why.contains("device.crt"), "{why}");
    assert!(!scratch.0.join("provision").join("example-member.staged.txt").exists());
}

#[test]
fn a_stand_alone_document_stages_nothing() {
    let scratch = Scratch::new("stand-alone");
    let document = write_document(&scratch.0, "example-alone", "version = 1\nhostname = \"example-alone\"\n");
    let report = stage(&document, &scratch.0.join("nowhere"), &scratch.0).expect("nothing to do");
    assert_eq!(report.path, None);
    assert_eq!(report.role, None);
    assert!(!scratch.0.join("provision").join("example-alone.staged.txt").exists());
}

/// Hex text into bytes, for reading a staged certificate back.
fn decode_hex(text: &str) -> Option<Vec<u8>> {
    (0..text.len())
        .step_by(2)
        .map(|i| text.get(i..i + 2).and_then(|pair| u8::from_str_radix(pair, 16).ok()))
        .collect()
}

// -- I07: write_bundle publishes through crate::publish_bundle, not fs::write --

/// One legal entry, enough to render and read back.
fn one_item() -> Vec<Staged> {
    vec![Staged::new("rivet-fleetd", "server.crt", b"cert bytes".to_vec()).expect("a legal entry")]
}

/// **A write that cannot even start must leave a prior bundle exactly as it
/// was.** The temporary `crate::publish_bundle` needs is pre-empted as a
/// directory, so the write it must do before any rename is refused on every
/// real filesystem — no fake, no permission trick a container's root user
/// could ignore.
///
/// Fails on the code this replaces: a direct `std::fs::write(path, ..)` does
/// not look at `<path>.new` at all, so it would overwrite `path` with the new
/// bytes and return `Ok`, and the assertion below that the call is refused
/// would panic.
#[test]
fn a_refused_write_bundle_leaves_the_old_bundle_byte_identical() {
    let scratch = Scratch::new("write-bundle-refused-write");
    let path = scratch.0.join("member.staged.txt");
    std::fs::write(&path, "old bundle bytes").expect("the old bundle is written");
    let temp = scratch.0.join("member.staged.txt.new");
    std::fs::create_dir_all(&temp).expect("a directory standing where the temp file would go");

    write_bundle(&path, &one_item()).expect_err("the write is refused");

    let after = std::fs::read_to_string(&path).expect("the old bundle is still there");
    assert_eq!(after, "old bundle bytes", "a failure before the rename must not touch it");
}

/// **A successful `write_bundle` leaves exactly the new bundle**, and absorbs
/// whatever temporary an earlier interrupted run left behind rather than
/// letting it accumulate beside the real file forever.
///
/// Fails on the code this replaces: a direct `std::fs::write` never looks at
/// `<path>.new`, so the stale temporary seeded below would still be sitting
/// there afterward and the assertion that it is gone would fail.
#[test]
fn a_successful_write_bundle_leaves_exactly_the_new_bundle() {
    let scratch = Scratch::new("write-bundle-success");
    let path = scratch.0.join("member.staged.txt");
    std::fs::write(&path, "old bundle bytes").expect("the old bundle is written");
    let temp = scratch.0.join("member.staged.txt.new");
    std::fs::write(&temp, "garbage left by an interrupted run").expect("a stale temporary");

    let items = one_item();
    let count = write_bundle(&path, &items).expect("nothing here should refuse");
    assert_eq!(count, 1);

    let after = std::fs::read_to_string(&path).expect("the new bundle is there");
    assert_eq!(after, rivet_core::staging::render(&items), "neither the old bytes nor a mixture");
    assert!(!temp.exists(), "the stale temporary must not survive a successful publish");
}

fn fleet_root(fleet: &str) -> Extras {
    Extras {
        fleet_root: Some(fleet.to_owned()),
        ..Extras::default()
    }
}

/// **A fleet-root server's card carries the request and NO workstation leaf**,
/// with no authority on the workstation at all — and everything else a server
/// needs, as before.
#[test]
fn a_fleet_root_server_carries_the_request_and_no_workstation_leaf() {
    let scratch = Scratch::new("fleet-root");
    // Deliberately no `authority(...)`: fleet-root mode needs none.
    let id = identity(&scratch.0, "example-server", OPERATOR);
    let document = write_document(&scratch.0, "example-server", SERVER);

    let report = stage_with(&document, &id, &scratch.0, &fleet_root("dev-fleet")).expect("staged");
    let items = read_bundle(&report.path.expect("a bundle"));

    let request = items
        .iter()
        .find(|i| i.application == "rivet-initd" && i.path == "fleet-root")
        .expect("the request is carried");
    let parsed = rivet_core::fleet_root::parse(core::str::from_utf8(&request.bytes).unwrap())
        .expect("the board will read it");
    assert_eq!(parsed.fleet, "dev-fleet");
    assert_eq!(parsed.host, "example-server");
    assert_eq!(parsed.suffixes, vec!["local", "dev-fleet.home.arpa"]);

    for (app, path) in rivet_core::fleet_root::handed_paths() {
        assert!(!has(&items, app, path), "{app}/{path} came from the workstation");
    }
    for file in ["operator-device.seed.hex", "operator-device.crt.hex"] {
        assert!(has(&items, "rivet-fleetd", file), "rivet-fleetd has no {file}");
    }
    assert!(items.iter().any(|i| i.path.starts_with("register/devices/")));
    assert_eq!(peers(&id).len(), 2, "fleetd's operator key joins the peers file");
    assert!(!scratch.0.join("ca").exists(), "the workstation gained an authority");
}

/// The suffixes are the fleet's own names and nothing else, checked here as
/// the board checks them.
#[test]
fn a_fleet_root_suffix_outside_the_fleet_is_refused_before_anything_is_written() {
    let scratch = Scratch::new("fleet-root-bad");
    let id = identity(&scratch.0, "example-server", OPERATOR);
    let document = write_document(&scratch.0, "example-server", SERVER);
    let extras = Extras {
        fleet_root: Some("dev-fleet".to_owned()),
        suffixes: Some(vec!["local".to_owned(), "example.com".to_owned()]),
        passkeys: None,
    };
    assert!(stage_with(&document, &id, &scratch.0, &extras).is_err());
    assert!(!scratch.0.join("provision").join("example-server.staged.txt").exists());
    assert_eq!(peers(&id), vec![OPERATOR.to_owned()], "peers changed on a refusal");
}

/// Fleet-root mode is a server's; a member's document is refused.
#[test]
fn a_fleet_root_member_is_refused() {
    let scratch = Scratch::new("fleet-root-member");
    authority(&scratch.0);
    let id = identity(&scratch.0, "example-member", OPERATOR);
    let document = write_document(&scratch.0, "example-member", MEMBER);
    let why = stage_with(&document, &id, &scratch.0, &fleet_root("dev-fleet")).expect_err("refused");
    assert!(why.contains("FLEET SERVER"), "{why}");
}

/// **A machine's passkeys ride to its new card**, verbatim, for its proxy.
#[test]
fn passkeys_are_carried_to_the_proxy_verbatim() {
    let scratch = Scratch::new("passkeys");
    let id = identity(&scratch.0, "example-server", OPERATOR);
    let document = write_document(&scratch.0, "example-server", SERVER);
    let file = scratch.0.join("passkeys");
    std::fs::write(&file, "one passkey line\nanother\n").expect("written");
    let extras = Extras {
        fleet_root: Some("dev-fleet".to_owned()),
        suffixes: None,
        passkeys: Some(file),
    };
    let report = stage_with(&document, &id, &scratch.0, &extras).expect("staged");
    let items = read_bundle(&report.path.expect("a bundle"));
    let carried = items
        .iter()
        .find(|i| i.application == "rivet-proxyd" && i.path == "passkeys")
        .expect("carried");
    assert_eq!(carried.bytes, b"one passkey line\nanother\n");

    let empty = scratch.0.join("empty");
    std::fs::write(&empty, "").expect("written");
    let extras = Extras { passkeys: Some(empty), ..fleet_root("dev-fleet") };
    assert!(stage_with(&document, &id, &scratch.0, &extras).is_err());
}
