//! A fleet server's repository store, end to end through [`serve`]: real signed
//! artifacts, a real trust store and a real git repository, in a temporary
//! directory rather than under `/state` and `/system`.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::path::PathBuf;

use rivet_core::package::kind::Kind;
use rivet_core::package::port::{Digest as _, MaySign, Sign as _};
use rivet_core::package::trust_store::{Entry, TrustStore};
use rivet_core::package::write::{Builder, NewMember};

use super::*;
use crate::bulk::TrustStoreDoor;

const TRUSTED: &[u8] = b"rivet-mcpd repository test: the key this machine trusts";
const STRANGER: &[u8] = b"rivet-mcpd repository test: a key nobody trusts";
/// A second trusted key, the only one permitted to sign cards — the shape of a
/// system key beside an application key.
const SYSTEM: &[u8] = b"rivet-mcpd repository test: the key trusted for cards";

fn signer(label: &[u8]) -> rivet_crypto::Ed25519Signer {
    let mut digest = rivet_crypto::Blake3::new();
    rivet_crypto::Ed25519Signer::from_seed(&digest.of(label))
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut h = rivet_crypto::Blake3::new();
    h.start();
    h.write(bytes);
    bulk::hex(&h.finish())
}

/// A real signed artifact of `kind`, built for `arch`.
fn sealed(by: &[u8], product: &str, version: &str, arch: &str, kind: Kind) -> Vec<u8> {
    let mut digest = rivet_crypto::Blake3::new();
    let platform = (kind == Kind::update() || kind == Kind::card()).then_some("rpi4");
    let mut builder = Builder::new(product, version, arch, platform, kind).expect("a header");
    let parts: Vec<(&str, Vec<u8>)> = vec![
        ("manifest.toml", b"kind = \"daemon\"\n".to_vec()),
        ("program", (0..70_000usize).map(|i| (i % 251) as u8).collect()),
    ];
    for (i, (name, bytes)) in parts.iter().enumerate() {
        builder
            .add(NewMember::stored(name, u8::try_from(i).unwrap(), 0, bytes, &mut digest).expect("a member"))
            .expect("room");
    }
    let slices: Vec<&[u8]> = parts.iter().map(|(_, b)| b.as_slice()).collect();
    builder.seal_in_memory(&mut digest, &mut signer(by), &slices).expect("sealed")
}

fn application(by: &[u8], product: &str, version: &str) -> Vec<u8> {
    sealed(by, product, version, "aarch64", Kind::application())
}

/// A real signed `apk` — `ADR-061` §3: `platform = "android"`, `arch = "any"`,
/// `app.apk` and `apk.json` as its only two members.
fn apk(by: &[u8], product: &str, version: &str) -> Vec<u8> {
    use rivet_core::package::apk::{APK_NAME, METADATA_NAME};
    let mut digest = rivet_crypto::Blake3::new();
    let mut builder =
        Builder::new(product, version, "any", Some("android"), Kind::apk()).expect("a header");
    let metadata = format!(
        "{{\"application_id\":\"com.example.{product}\",\"version_code\":1,\
         \"version_name\":\"{version}\",\"label\":\"{product}\",\"min_sdk\":21,\
         \"target_sdk\":34,\"signer_sha256\":\"{}\",\"development\":true}}",
        "ab".repeat(32)
    );
    let parts: Vec<(&str, Vec<u8>)> = vec![
        (APK_NAME, b"pretend-this-is-a-zip".to_vec()),
        (METADATA_NAME, metadata.into_bytes()),
    ];
    for (i, (name, bytes)) in parts.iter().enumerate() {
        builder
            .add(NewMember::stored(name, u8::try_from(i).unwrap(), 0, bytes, &mut digest).expect("a member"))
            .expect("room");
    }
    let slices: Vec<&[u8]> = parts.iter().map(|(_, b)| b.as_slice()).collect();
    builder.seal_in_memory(&mut digest, &mut signer(by), &slices).expect("sealed")
}

/// A scratch machine: `rivet-fleetd`'s data directory, a trust store trusting
/// [`TRUSTED`] for applications and updates, an `aarch64` record, and a git
/// repository standing in for `/state/git/fleet.git`.
struct Scratch {
    root: PathBuf,
    machine: Machine,
    door: TrustStoreDoor,
    work: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!("rivet-repository-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let data = root.join("app").join("rivet-fleetd").join("data");
        std::fs::create_dir_all(&data).expect("a data directory");
        let trust = root.join("trust");
        let arch = root.join("arch");
        std::fs::write(&arch, rivet_core::package::arch::render("aarch64").expect("a token")).expect("arch");
        let store = TrustStore::new(vec![Entry {
            key: signer(TRUSTED).public_key(),
            label: "test-fleet".to_string(),
            may_sign: MaySign::these(&[
                Kind::application(),
                Kind::update(),
                Kind::library(),
                Kind::apk(),
            ]),
        }, Entry {
            key: signer(SYSTEM).public_key(),
            label: "test-system".to_string(),
            may_sign: MaySign::these(&[Kind::update(), Kind::card()]),
        }]);
        std::fs::write(&trust, store.to_bytes().expect("bytes")).expect("trust");
        let work = root.join("fleet");
        let machine = Machine::at(&data, &work.join(".git"));
        let door = TrustStoreDoor::at(trust.to_string_lossy(), arch.to_string_lossy())
            .for_purpose(bulk::Purpose::Mirror);
        Self { root, machine, door, work }
    }

    fn request(method: &str, target: &str, digest: Option<String>, length: Option<u64>) -> Request {
        Request {
            method: method.to_owned(),
            target: target.to_owned(),
            digest,
            length,
            content_type: None,
            content_encoding: None,
            chunked: false,
        }
    }

    fn put(&self, name: &str, body: &[u8]) -> Response {
        let request = Self::request(
            "PUT",
            &format!("{}{name}", objects::PREFIX),
            Some(hex_digest(body)),
            Some(body.len() as u64),
        );
        let mut empty = |_: &mut [u8]| Ok(0);
        serve(&self.machine, &self.door, &request, body, &mut empty, Some(Caller::Operator))
    }

    fn get(&self) -> Response {
        let request = Self::request("GET", objects::PREFIX, None, None);
        let mut empty = |_: &mut [u8]| Ok(0);
        serve(&self.machine, &self.door, &request, &[], &mut empty, Some(Caller::Operator))
    }

    fn prune(&self) -> Response {
        let request = Self::request("POST", objects::PRUNE, None, Some(0));
        let mut empty = |_: &mut [u8]| Ok(0);
        serve(&self.machine, &self.door, &request, &[], &mut empty, Some(Caller::Operator))
    }

    fn git(&self, args: &[&str]) {
        let status = Command::new("git")
            .args(["-c", "user.name=test", "-c", "user.email=test@rivet.invalid"])
            .args(args)
            .current_dir(&self.work)
            .output()
            .expect("git runs");
        assert!(status.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&status.stderr));
    }

    /// Commit a manifest naming `(digest, kind)` pairs on `main`.
    fn commit_manifest(&self, entries: &[(&str, &str)]) {
        let entries: Vec<(&str, &str, &str)> = entries.iter().map(|(d, k)| (*d, "p", *k)).collect();
        self.commit_manifest_hiding(&entries, &[]);
    }

    /// Commit a manifest naming `(digest, product, kind)` entries and a hidden
    /// set of `(product, kind)` on `main`, in the shape `fleet::render` writes.
    fn commit_manifest_hiding(&self, entries: &[(&str, &str, &str)], hidden: &[(&str, &str)]) {
        std::fs::create_dir_all(&self.work).expect("a work tree");
        if !self.work.join(".git").is_dir() {
            self.git(&["init", "-q", "-b", "main"]);
        }
        let mut text = String::from("# test manifest\n");
        for (digest, product, kind) in entries {
            text.push_str(&format!(
                "\n[[artifact]]\ndigest  = \"{digest}\"\nproduct = \"{product}\"\nversion = \"0.1.0\"\n\
                 arch    = \"aarch64\"\nkind    = \"{kind}\"\nsize    = 1\nadded   = \"x\"\n"
            ));
        }
        for (product, kind) in hidden {
            text.push_str(&format!("\n[[hidden]]\nproduct = \"{product}\"\nkind    = \"{kind}\"\n"));
        }
        std::fs::write(self.work.join("manifest.toml"), text).expect("manifest");
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--allow-empty", "-m", "manifest"]);
    }

    fn listing(&self) -> Listing {
        let response = self.get();
        assert_eq!(response.status, 200, "{}", response.body);
        Listing::parse(&response.body).expect("a listing this build reads")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn name(product: &str, version: &str) -> String {
    format!("{product}-{version}-aarch64-application.rvt")
}

#[test]
fn only_the_stores_routes_reach_it() {
    assert!(is_repository_objects("/repository-objects/"));
    assert!(is_repository_objects("/repository-objects"));
    assert!(is_repository_objects("/repository-objects/prune"));
    assert!(is_repository_objects("/repository-objects/x-0.1.0-aarch64-application.rvt"));
    assert!(!is_repository_objects("/payload/x.rvt"));
    assert!(!is_repository_objects("/fleet.git/info/refs"));
}

/// **R-184 D2**: a connection that presented no pinned key is not the operator.
#[test]
fn a_caller_with_no_pinned_key_is_refused() {
    let scratch = Scratch::new("no-caller");
    let request = Scratch::request("GET", objects::PREFIX, None, None);
    let mut empty = |_: &mut [u8]| Ok(0);
    let response = serve(&scratch.machine, &scratch.door, &request, &[], &mut empty, None);
    assert_eq!(response.status, 403, "{}", response.body);
    assert!(response.body.contains("R-184 D2"), "{}", response.body);
}

/// A machine without `rivet-fleetd` keeps no store and creates nothing.
#[test]
fn a_machine_without_rivet_fleetd_creates_nothing() {
    let scratch = Scratch::new("no-fleetd");
    std::fs::remove_dir_all(scratch.root.join("app")).expect("remove the data directory");
    let response = scratch.put(&name("hello", "0.1.0"), &application(TRUSTED, "hello", "0.1.0"));
    assert_eq!(response.status, 404, "{}", response.body);
    assert!(!scratch.root.join("app").exists(), "the refusal created a directory");
}

/// **A trusted application lands under its digest**, the answer says what was
/// verified, and the listing shows it held.
#[test]
fn a_trusted_application_is_verified_and_held() {
    let scratch = Scratch::new("trusted");
    let body = application(TRUSTED, "hello", "0.1.0");
    let response = scratch.put(&name("hello", "0.1.0"), &body);
    assert_eq!(response.status, 201, "{}", response.body);
    assert!(response.body.contains("\"test-fleet\""), "{}", response.body);
    assert!(response.body.contains("repository store"), "{}", response.body);
    assert!(!response.body.contains("apply install"), "{}", response.body);
    let digest = artifact::Digest::new(&hex_digest(&body)).expect("digest");
    assert!(Path::new(&scratch.machine.store().object_path(&digest, &name("hello", "0.1.0"))).is_file());
    assert!(scratch.listing().holds(&digest));
}

/// **`ADR-061` §3: an application-signed apk is admitted and held**, exactly
/// as an application or a library is — the door's `holds_kind` and
/// `mirror_structure` both learned the kind together.
#[test]
fn a_trusted_apk_is_verified_and_held() {
    let scratch = Scratch::new("trusted-apk");
    let body = apk(TRUSTED, "gantry", "0.1.17");
    let file_name = "gantry-0.1.17-any-android-apk.rvt";
    let response = scratch.put(file_name, &body);
    assert_eq!(response.status, 201, "{}", response.body);
    let digest = artifact::Digest::new(&hex_digest(&body)).expect("digest");
    assert!(Path::new(&scratch.machine.store().object_path(&digest, file_name)).is_file());
    assert!(scratch.listing().holds(&digest));
}

/// Tampered bytes are refused on the wire, and an untrusted signature at the
/// door; neither is held.
#[test]
fn a_tampered_or_untrusted_object_is_refused_and_not_held() {
    let scratch = Scratch::new("refused");
    let body = application(TRUSTED, "hello", "0.1.0");
    let mut tampered = body.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0xff;
    // The digest of the ORIGINAL, the tampered bytes: the wire check.
    let request = Scratch::request(
        "PUT",
        &format!("{}{}", objects::PREFIX, name("hello", "0.1.0")),
        Some(hex_digest(&body)),
        Some(tampered.len() as u64),
    );
    let mut empty = |_: &mut [u8]| Ok(0);
    let response = serve(&scratch.machine, &scratch.door, &request, &tampered, &mut empty, Some(Caller::Operator));
    assert_eq!(response.status, 409, "{}", response.body);

    // Honestly digested, and signed the same way with the member altered
    // AFTER sealing: the door's member check.
    let digest_of_tampered = hex_digest(&tampered);
    let request = Scratch::request(
        "PUT",
        &format!("{}{}", objects::PREFIX, name("hello", "0.1.0")),
        Some(digest_of_tampered),
        Some(tampered.len() as u64),
    );
    let response = serve(&scratch.machine, &scratch.door, &request, &tampered, &mut empty, Some(Caller::Operator));
    assert_eq!(response.status, 422, "{}", response.body);

    let stranger = application(STRANGER, "hello", "0.2.0");
    let response = scratch.put(&name("hello", "0.2.0"), &stranger);
    assert_eq!(response.status, 422, "{}", response.body);
    assert!(scratch.listing().held.is_empty());
}

/// **`ADR-063` §6: a system update and a card are admitted and held**, signed
/// by a key this machine trusts for each, so a fleet server can bring a member
/// current and write a card for a host it recovers.
#[test]
fn a_trusted_update_and_card_are_verified_and_held() {
    let scratch = Scratch::new("update-and-card");
    let update = sealed(TRUSTED, "rivet", "0.1.0", "aarch64", Kind::update());
    let card = sealed(SYSTEM, "rivet", "0.1.0", "aarch64", Kind::card());
    for (body, file_name) in [
        (&update, "rivet-0.1.0-aarch64-rpi4-update.rvt"),
        (&card, "rivet-0.1.0-aarch64-rpi4-card.rvt"),
    ] {
        let response = scratch.put(file_name, body);
        assert_eq!(response.status, 201, "{file_name}: {}", response.body);
        let digest = artifact::Digest::new(&hex_digest(body)).expect("digest");
        assert!(Path::new(&scratch.machine.store().object_path(&digest, file_name)).is_file());
        assert!(scratch.listing().holds(&digest), "{file_name} is not listed as held");
    }
}

/// **An update that is not a whole payload is refused at the mirror's door**,
/// as the installer that would one day open it refuses it: here, two system
/// members, so which one a slot becomes is ambiguous.
#[test]
fn an_update_that_is_not_a_whole_payload_is_refused() {
    let scratch = Scratch::new("update-malformed");
    let mut digest = rivet_crypto::Blake3::new();
    let mut builder =
        Builder::new("rivet", "0.1.0", "aarch64", Some("rpi4"), Kind::update()).expect("a header");
    let parts: [(&str, &[u8]); 2] = [("system-a", b"one"), ("system-b", b"two")];
    for (name, bytes) in parts {
        builder
            .add(NewMember::stored(name, 0, 0, bytes, &mut digest).expect("a member"))
            .expect("room");
    }
    let slices: Vec<&[u8]> = parts.iter().map(|(_, b)| *b).collect();
    let body = builder.seal_in_memory(&mut digest, &mut signer(TRUSTED), &slices).expect("sealed");
    let response = scratch.put("rivet-0.1.0-aarch64-rpi4-update.rvt", &body);
    assert_eq!(response.status, 422, "{}", response.body);
    assert!(scratch.listing().held.is_empty());
}

/// **A card is held only where the trust store lets its signer sign cards.**
/// The door asks the machine's own trust store for every kind, so a key this
/// machine trusts for applications and updates and not for cards has its card
/// refused — the store is the authority, and widening what the fleet server
/// holds widens nothing it trusts.
#[test]
fn a_card_from_a_key_not_trusted_for_cards_is_refused() {
    let scratch = Scratch::new("card-untrusted-kind");
    let card = sealed(TRUSTED, "rivet", "0.1.0", "aarch64", Kind::card());
    let response = scratch.put("rivet-0.1.0-aarch64-rpi4-card.rvt", &card);
    assert_eq!(response.status, 422, "{}", response.body);
    assert!(scratch.listing().held.is_empty());
}

/// **Another architecture is held, and said.**
#[test]
fn an_object_for_another_architecture_is_held_and_recorded() {
    let scratch = Scratch::new("arch");
    let body = sealed(TRUSTED, "hello", "0.1.0", "armv6", Kind::application());
    let response = scratch.put("hello-0.1.0-armv6-application.rvt", &body);
    assert_eq!(response.status, 201, "{}", response.body);
    assert!(response.body.contains("HELD ANYWAY"), "{}", response.body);
    assert!(response.body.contains("armv6"), "{}", response.body);
}

/// A re-send of an object already held answers `201` without writing it again.
#[test]
fn a_re_send_of_a_held_object_short_circuits() {
    let scratch = Scratch::new("re-send");
    let body = application(TRUSTED, "hello", "0.1.0");
    assert_eq!(scratch.put(&name("hello", "0.1.0"), &body).status, 201);
    let again = scratch.put(&name("hello", "0.1.0"), &body);
    assert_eq!(again.status, 201, "{}", again.body);
    assert!(again.body.contains("already holds this digest"), "{}", again.body);
}

/// **The listing reads the repository's manifest**, and names what is missing.
#[test]
fn the_listing_names_the_manifest_and_what_it_still_lacks() {
    let scratch = Scratch::new("listing");
    let one = application(TRUSTED, "hello", "0.1.0");
    let two = application(TRUSTED, "hello", "0.2.0");
    let update = "c".repeat(64);
    let card = "d".repeat(64);
    scratch.commit_manifest(&[
        (&hex_digest(&one), "application"),
        (&hex_digest(&two), "application"),
        (&update, "update"),
        (&card, "card"),
    ]);
    assert_eq!(scratch.put(&name("hello", "0.1.0"), &one).status, 201);
    let listing = scratch.listing();
    assert!(matches!(listing.manifest, Manifest::Read { .. }), "{listing:?}");
    let missing: Vec<String> = listing.missing().iter().map(|d| d.as_str().to_owned()).collect();
    // 0.1.0 is held; 0.2.0, the update and the card are named and not held
    // (`ADR-063` §6: a fleet server holds updates and cards too).
    let mut want = vec![hex_digest(&two), update, card];
    want.sort();
    assert_eq!(missing, want);
}

/// With no repository the listing says why, and a prune is refused.
#[test]
fn with_no_repository_nothing_is_pruned() {
    let scratch = Scratch::new("no-repository");
    assert_eq!(scratch.put(&name("hello", "0.1.0"), &application(TRUSTED, "hello", "0.1.0")).status, 201);
    assert!(matches!(scratch.listing().manifest, Manifest::Unavailable(_)));
    let response = scratch.prune();
    assert_eq!(response.status, 409, "{}", response.body);
    assert_eq!(scratch.listing().held.len(), 1);
}

/// **The store root itself, planted as a symlink before the store has ever
/// been created.** An application owns its own data directory, so it can
/// replace `repository-objects` with a link before the very first `PUT`
/// ever runs `create_dir_all` over it. `create_dir_all`'s own `AlreadyExists`
/// handling calls `is_dir()`, which follows the link, so without this check
/// the first object a fleet server ever pushes would be written wherever the
/// application pointed it -- before `hand_over`'s own symlink guard (see the
/// test below) is ever reached. `.rulings/` R-184.
#[test]
fn a_put_refuses_a_store_root_an_application_replaced_with_a_symlink() {
    let scratch = Scratch::new("planted-root");

    // Outside the store entirely, and empty: what a compromised rivet-fleetd
    // wants root to write into on its behalf.
    let outside = scratch.root.join("outside");
    std::fs::create_dir_all(&outside).expect("the outside directory");

    let store_root = scratch.machine.store().root().to_string();
    assert!(
        !std::path::Path::new(&store_root).exists(),
        "the store already exists before the first PUT; the test's premise is wrong"
    );
    std::os::unix::fs::symlink(&outside, &store_root).expect("the planted symlink");

    let response = scratch.put(&name("hello", "0.1.0"), &application(TRUSTED, "hello", "0.1.0"));

    assert_eq!(response.status, 500, "{}", response.body);
    assert!(
        response.body.contains(&store_root) && response.body.contains("symbolic link"),
        "the refusal did not name the planted path: {:?}",
        response.body
    );
    assert_eq!(
        std::fs::read_dir(&outside).expect("still there").count(),
        0,
        "something was created through the link"
    );
}

/// **SB1**: a symlink planted where a digest directory belongs must not be
/// followed. `hand_over` legitimately leaves every real directory in the
/// store owned by the application — that is the whole point of the call — so
/// once the first object has landed, a compromised `rivet-fleetd` owns
/// `objects/` itself and can create entries inside it, including a link to
/// anywhere at all. `.rulings/` R-184.
#[test]
fn hand_over_does_not_chown_or_descend_through_a_planted_symlink() {
    use std::os::unix::fs::MetadataExt as _;

    let root =
        std::env::temp_dir().join(format!("rivet-repository-{}-symlink", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let data = root.join("app").join("rivet-fleetd").join("data");
    std::fs::create_dir_all(&data).expect("a data directory");

    // Outside the store entirely -- what a compromised rivet-fleetd wants
    // handed to its own uid, per SB1's `/run/rivet` example.
    let outside = root.join("outside");
    std::fs::create_dir_all(&outside).expect("the outside directory");
    let secret = outside.join("secret");
    std::fs::write(&secret, b"not rivet-fleetd's").expect("the outside file");
    let before = std::fs::metadata(&secret).expect("stat before");

    if before.uid() != 0 {
        // `lchown` to an arbitrary uid needs root. Rather than assert nothing,
        // this reports which seam it fell back to: the file this test just
        // created is proof of the identity it is running as, without a new
        // dependency on `libc::getuid`.
        eprintln!(
            "not root (a file this test just created is owned by uid {}): lchown to a foreign uid \
             cannot be exercised, so this proves nothing about the fix",
            before.uid()
        );
        let _ = std::fs::remove_dir_all(&root);
        return;
    }

    let machine = Machine::at(&data, &root.join("fleet.git")).owned_by(65_534, 65_534);
    std::fs::create_dir_all(machine.store().objects_dir()).expect("the objects directory");
    // Planted exactly where SB1 describes: a digest directory replaced by a
    // link to somewhere else entirely.
    let planted = format!("{}/planted-digest", machine.store().objects_dir());
    std::os::unix::fs::symlink(&outside, &planted).expect("the planted symlink");

    let report = hand_over(&machine);
    assert!(
        report.contains("planted-digest") && report.contains("symbolic link"),
        "the walk did not report the planted link: {report:?}"
    );

    let after = std::fs::metadata(&secret).expect("stat after");
    assert_eq!(
        after.uid(),
        before.uid(),
        "the outside file's owner changed: the walk followed the planted link"
    );
    assert_eq!(
        after.gid(),
        before.gid(),
        "the outside file's group changed: the walk followed the planted link"
    );
    let entries: Vec<_> = std::fs::read_dir(&outside)
        .expect("the outside directory is still there")
        .collect();
    assert_eq!(entries.len(), 1, "something was created through the link: {entries:?}");

    let _ = std::fs::remove_dir_all(&root);
}

/// **The store's directories stay root's; only object files are handed
/// over**, and `rivet-fleetd` reads the directories through its group. An
/// application that owned a directory root writes into could plant a link in
/// it, so it owns none: the root, `objects/`, `incoming/` and every digest
/// directory are root's, mode `0750`, group the application's. Each object file
/// is the application's.
///
/// A store a build before this one handed over whole is put back the same way
/// by the next `PUT`.
#[cfg(unix)]
#[test]
fn only_object_files_are_handed_over_and_the_directories_stay_roots() {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let mut scratch = Scratch::new("ownership");
    let probe = scratch.root.join("probe");
    std::fs::write(&probe, b"who am I").expect("probe");
    let me = std::fs::metadata(&probe).expect("probe").uid();
    if me != 0 {
        eprintln!(
            "not root (a file this test just created is owned by uid {me}): chown to a foreign \
             uid cannot be exercised, so this proves nothing about the hand-over"
        );
        return;
    }
    scratch.machine = scratch.machine.clone().owned_by(65_534, 65_533);

    // A store as an earlier build left it: every directory the application's.
    let store_root = scratch.machine.store().root().to_owned();
    std::fs::create_dir_all(scratch.machine.store().incoming_dir()).expect("incoming");
    std::os::unix::fs::lchown(&store_root, Some(65_534), Some(65_534)).expect("an old owner");

    let body = application(TRUSTED, "hello", "0.1.0");
    let response = scratch.put(&name("hello", "0.1.0"), &body);
    assert_eq!(response.status, 201, "{}", response.body);
    assert!(!response.body.contains("NOT every entry"), "{}", response.body);

    let digest = artifact::Digest::new(&hex_digest(&body)).expect("digest");
    let store = scratch.machine.store().clone();
    for dir in [store.root().to_owned(), store.objects_dir(), store.incoming_dir(), store.object_dir(&digest)] {
        let meta = std::fs::symlink_metadata(&dir).expect("a directory");
        assert!(meta.is_dir(), "{dir}");
        assert_eq!((meta.uid(), meta.gid()), (0, 65_533), "{dir} is not root's with the application's group");
        assert_eq!(meta.permissions().mode() & 0o7777, 0o750, "{dir}");
    }
    let file = std::fs::symlink_metadata(store.object_path(&digest, &name("hello", "0.1.0"))).expect("the object");
    assert_eq!((file.uid(), file.gid()), (65_534, 65_533), "the object file was not handed over");
}

/// **`ADR-052` §7: a hidden product's object leaves the store at the prune.**
/// The manifest still lists it as an `[[artifact]]`; the `[[hidden]]` entry is
/// what makes it not named.
#[test]
fn a_hidden_products_object_is_pruned() {
    let scratch = Scratch::new("prune-hidden");
    let shown = application(TRUSTED, "hello", "0.1.0");
    let hidden = apk(TRUSTED, "testbed", "0.1.3");
    let hidden_name = "testbed-0.1.3-any-android-apk.rvt";
    scratch.commit_manifest_hiding(
        &[(&hex_digest(&shown), "hello", "application"), (&hex_digest(&hidden), "testbed", "apk")],
        &[],
    );
    assert_eq!(scratch.put(&name("hello", "0.1.0"), &shown).status, 201);
    assert_eq!(scratch.put(hidden_name, &hidden).status, 201);

    scratch.commit_manifest_hiding(
        &[(&hex_digest(&shown), "hello", "application"), (&hex_digest(&hidden), "testbed", "apk")],
        &[("testbed", "apk")],
    );
    let pruned = scratch.prune();
    assert_eq!(pruned.status, 200, "{}", pruned.body);
    assert!(pruned.body.contains(&format!("removed {}", hex_digest(&hidden))), "{}", pruned.body);
    let listing = scratch.listing();
    assert_eq!(listing.held.len(), 1, "{listing:?}");
    assert_eq!(listing.held[0].digest.as_str(), hex_digest(&shown));
}

/// **A hidden object the store never held is not missing**, so a store holding
/// everything else is complete and a prune runs.
#[test]
fn a_hidden_object_not_held_is_not_missing() {
    let scratch = Scratch::new("hidden-not-missing");
    let shown = application(TRUSTED, "hello", "0.1.0");
    let never_sent = "d".repeat(64);
    scratch.commit_manifest_hiding(
        &[(&hex_digest(&shown), "hello", "application"), (&never_sent, "testbed", "apk")],
        &[("testbed", "apk")],
    );
    assert_eq!(scratch.put(&name("hello", "0.1.0"), &shown).status, 201);
    let listing = scratch.listing();
    assert!(listing.missing().is_empty(), "{:?}", listing.missing());
    let pruned = scratch.prune();
    assert_eq!(pruned.status, 200, "{}", pruned.body);
}

/// A `[[hidden]]` entry without a string `kind` is a manifest that does not
/// read, and nothing is decided from it.
#[test]
fn a_hidden_entry_that_does_not_read_makes_the_manifest_unavailable() {
    let scratch = Scratch::new("hidden-malformed");
    scratch.commit_manifest(&[]);
    std::fs::write(
        scratch.work.join("manifest.toml"),
        "[[hidden]]\nproduct = \"testbed\"\n",
    )
    .expect("manifest");
    scratch.git(&["add", "-A"]);
    scratch.git(&["commit", "-q", "-m", "malformed"]);
    let listing = scratch.listing();
    assert!(
        matches!(&listing.manifest, Manifest::Unavailable(why) if why.contains("[[hidden]] 1")),
        "{listing:?}"
    );
}

/// **Three cards are kept, as the repository keeps them** (`ADR-063` §6). The
/// store holds four cards of one product; the repository's retention has
/// stopped naming the oldest, so the prune removes exactly that one and the
/// three the manifest names stay.
#[test]
fn a_fourth_card_leaves_the_store_when_the_manifest_keeps_three() {
    let scratch = Scratch::new("prune-cards");
    let versions = ["0.1.0", "0.1.1", "0.1.2", "0.1.3"];
    let cards: Vec<Vec<u8>> =
        versions.iter().map(|v| sealed(SYSTEM, "rivet", v, "aarch64", Kind::card())).collect();
    let digests: Vec<String> = cards.iter().map(|c| hex_digest(c)).collect();
    let all: Vec<(&str, &str)> = digests.iter().map(|d| (d.as_str(), "card")).collect();
    scratch.commit_manifest(&all);
    for (card, version) in cards.iter().zip(versions) {
        let file_name = format!("rivet-{version}-aarch64-rpi4-card.rvt");
        assert_eq!(scratch.put(&file_name, card).status, 201, "{file_name}");
    }

    scratch.commit_manifest(&all[1..]);
    let pruned = scratch.prune();
    assert_eq!(pruned.status, 200, "{}", pruned.body);
    assert!(pruned.body.contains(&format!("removed {}", digests[0])), "{}", pruned.body);
    let listing = scratch.listing();
    let mut held: Vec<&str> = listing.held.iter().map(|h| h.digest.as_str()).collect();
    held.sort_unstable();
    let mut kept: Vec<&str> = digests[1..].iter().map(String::as_str).collect();
    kept.sort_unstable();
    assert_eq!(held, kept, "exactly the three the manifest names stay");
}

/// `GET /repository-objects/<digest>` as the composition root receives it.
fn get_object(scratch: &Scratch, digest: &str, caller: Option<Caller>) -> Option<Result<Object, Response>> {
    let request = Scratch::request("GET", &format!("{}{digest}", objects::PREFIX), None, None);
    object(&scratch.machine, &request, caller)
}

/// **A held object reads back byte for byte, under its digest and its name**
/// (`ADR-063` §6) — the half a workstation's clone fetches through.
#[test]
fn a_held_object_reads_back_whole_under_its_digest_and_name() {
    use std::io::Read as _;
    let scratch = Scratch::new("read-back");
    let body = sealed(SYSTEM, "rivet", "0.1.0", "aarch64", Kind::card());
    let file_name = "rivet-0.1.0-aarch64-rpi4-card.rvt";
    assert_eq!(scratch.put(file_name, &body).status, 201);

    let mut found = get_object(&scratch, &hex_digest(&body), Some(Caller::Operator))
        .expect("an object request")
        .expect("held");
    assert_eq!(found.name, file_name);
    assert_eq!(found.length, body.len() as u64);
    assert_eq!(found.digest.as_str(), hex_digest(&body));
    let head = object_head(&found);
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
    assert!(head.contains(&format!("Content-Length: {}\r\n", body.len())), "{head}");
    assert!(head.contains(&format!("Rivet-Blake3: {}\r\n", hex_digest(&body))), "{head}");
    assert!(head.contains(&format!("Rivet-Name: {file_name}\r\n")), "{head}");
    let mut read = Vec::new();
    found.file.read_to_end(&mut read).expect("the object reads");
    assert_eq!(read, body, "the bytes sent are not the bytes held");
}

/// **What is not held is 404, a caller with no pinned key is 403, and a
/// request that is not an object's is not answered here at all.**
#[test]
fn an_object_not_held_or_asked_for_by_nobody_is_refused() {
    let scratch = Scratch::new("read-refused");
    let body = application(TRUSTED, "hello", "0.1.0");
    assert_eq!(scratch.put(&name("hello", "0.1.0"), &body).status, 201);

    let absent = get_object(&scratch, &"c".repeat(64), Some(Caller::Operator)).expect("an object request");
    assert_eq!(absent.expect_err("not held").status, 404);
    let nobody = get_object(&scratch, &hex_digest(&body), None).expect("an object request");
    assert_eq!(nobody.expect_err("no caller").status, 403);

    let listing = Scratch::request("GET", objects::PREFIX, None, None);
    assert!(object(&scratch.machine, &listing, Some(Caller::Operator)).is_none());
    let put = Scratch::request("PUT", &format!("{}{}", objects::PREFIX, hex_digest(&body)), None, None);
    assert!(object(&scratch.machine, &put, Some(Caller::Operator)).is_none());
}

/// **A link planted in the store is never followed.** `rivet-fleetd` owns the
/// directory and this daemon reads as root, so an object replaced by a link
/// to another file is refused rather than read.
#[cfg(unix)]
#[test]
fn a_planted_link_in_place_of_an_object_is_not_read() {
    let scratch = Scratch::new("read-link");
    let body = application(TRUSTED, "hello", "0.1.0");
    assert_eq!(scratch.put(&name("hello", "0.1.0"), &body).status, 201);
    let digest = artifact::Digest::new(&hex_digest(&body)).expect("digest");
    let held = scratch.machine.store().object_path(&digest, &name("hello", "0.1.0"));
    let secret = scratch.root.join("secret");
    std::fs::write(&secret, b"not the store's").expect("a file outside the store");
    std::fs::remove_file(&held).expect("remove the object");
    std::os::unix::fs::symlink(&secret, &held).expect("a planted link");

    let refused = get_object(&scratch, digest.as_str(), Some(Caller::Operator))
        .expect("an object request")
        .expect_err("a link is not an object");
    assert_eq!(refused.status, 500, "{}", refused.body);
    assert!(refused.body.contains("not a regular file"), "{}", refused.body);
}

/// **A machine's own store refuses a card** even from a key its trust store
/// lets sign cards (`ADR-063` §6): the system key is trusted for `card` so a
/// fleet server's store can hold one, and `/payload`'s door asks the kind too.
/// The mirror's door, asked the same bytes, holds it.
#[test]
fn a_card_is_refused_at_a_machines_own_door_and_held_by_the_mirror() {
    let scratch = Scratch::new("card-install-door");
    let card = sealed(SYSTEM, "rivet", "0.1.0", "aarch64", Kind::card());
    let path = scratch.root.join("card.rvt");
    std::fs::write(&path, &card).expect("the card");
    let path = path.to_string_lossy().into_owned();

    let install = scratch.door.clone().for_purpose(bulk::Purpose::Install);
    match install.admit(&path, card.len() as u64) {
        Err(bulk::Refusal::Artifact(why)) => assert!(why.contains("never installed"), "{why}"),
        other => panic!("a card was not refused at a machine's own door: {other:?}"),
    }
    assert!(scratch.door.admit(&path, card.len() as u64).is_ok(), "the mirror refused a trusted card");

    // An update from the same key still passes the machine's own door as far
    // as its kind: the refusal is the card's alone.
    let update = sealed(SYSTEM, "rivet", "0.1.0", "aarch64", Kind::update());
    let update_path = scratch.root.join("update.rvt");
    std::fs::write(&update_path, &update).expect("the update");
    if let Err(bulk::Refusal::Artifact(why)) =
        install.admit(&update_path.to_string_lossy(), update.len() as u64)
    {
        assert!(!why.contains("never installed"), "an update was refused as a card: {why}");
    }
}

/// **A prune runs only against a complete store, and removes exactly what the
/// manifest stops naming.**
#[test]
fn a_prune_is_refused_mid_sync_and_then_removes_exactly_the_unnamed_object() {
    let scratch = Scratch::new("prune");
    let one = application(TRUSTED, "hello", "0.1.0");
    let two = application(TRUSTED, "hello", "0.2.0");
    scratch.commit_manifest(&[(&hex_digest(&one), "application"), (&hex_digest(&two), "application")]);
    assert_eq!(scratch.put(&name("hello", "0.1.0"), &one).status, 201);

    let refused = scratch.prune();
    assert_eq!(refused.status, 409, "{}", refused.body);
    assert!(refused.body.contains("not complete"), "{}", refused.body);

    assert_eq!(scratch.put(&name("hello", "0.2.0"), &two).status, 201);
    // The manifest stops naming 0.1.0.
    scratch.commit_manifest(&[(&hex_digest(&two), "application")]);
    let pruned = scratch.prune();
    assert_eq!(pruned.status, 200, "{}", pruned.body);
    assert!(pruned.body.contains(&format!("removed {}", hex_digest(&one))), "{}", pruned.body);
    let listing = scratch.listing();
    assert_eq!(listing.held.len(), 1, "{listing:?}");
    assert_eq!(listing.held[0].digest.as_str(), hex_digest(&two));
}
