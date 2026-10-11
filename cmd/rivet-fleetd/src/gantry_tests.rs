//! **`SPEC-06` §7's provider conformance**, over a real Unix socket.
//!
//! A scripted client drives every operation and every failure code against
//! [`serve`], exactly as `gantry` would reach it. The uid the gate admits is a
//! parameter, because the test process is not `gantry`'s uid: the admitting
//! case passes this process's own uid and the refusing case any other.
//!
//! **Every fixture is hand-composed**: the artifacts are built with
//! `rivet_core`'s own package builder and an unsigned header, the phone keys are
//! the P-256 SubjectPublicKeyInfo prefix around arbitrary point bytes, and the
//! store keys are made-up digests. The socket asserts only what the store holds
//! and never recomputes a digest or checks a signature, so nothing here depends
//! on either being real.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::io::{BufRead as _, BufReader};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rivet_core::json::Json;
use rivet_core::package::{
    Builder, DigestAlgorithm, Kind, NewMember, PublicKey, Sign, Signature, SignatureAlgorithm,
};

use super::*;

/// A digest port that CLAIMS BLAKE3 and computes SHA-256. The socket never
/// recomputes a member digest, so which function filled the field is
/// irrelevant here; it only has to be a function of the bytes.
struct ClaimsBlake3(sha2::Sha256);

impl ClaimsBlake3 {
    fn new() -> Self {
        use sha2::Digest as _;
        Self(sha2::Sha256::new())
    }
}

impl rivet_core::package::Digest for ClaimsBlake3 {
    fn algorithm(&self) -> DigestAlgorithm {
        DigestAlgorithm::BLAKE3
    }
    fn start(&mut self) {
        use sha2::Digest as _;
        self.0 = sha2::Sha256::new();
    }
    fn write(&mut self, bytes: &[u8]) {
        use sha2::Digest as _;
        self.0.update(bytes);
    }
    fn finish(&mut self) -> [u8; 32] {
        use sha2::Digest as _;
        std::mem::take(&mut self.0).finalize().into()
    }
}

/// A signer whose signature is sixty-four zero bytes. The socket does not
/// verify a header (`SPEC-06` §6 leaves that to `gantry`), so a signature
/// nothing could verify is the honest fixture: a test passing over it proves
/// the socket did not look.
struct Unsigned;

impl Sign for Unsigned {
    fn algorithm(&self) -> SignatureAlgorithm {
        SignatureAlgorithm::ED25519
    }
    fn public_key(&self) -> PublicKey {
        PublicKey([7; 32])
    }
    fn sign(&mut self, _message: &[u8]) -> Signature {
        Signature([0; 64])
    }
}

/// A scratch directory under the OS temporary directory, removed at the end.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rivet-fleetd-gantry-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch directory");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// This process's own uid, as the kernel reports it on a socket pair.
fn our_uid() -> u32 {
    let (a, _b) = UnixStream::pair().expect("a socket pair");
    rivet_sdk::peer::credentials(&a).expect("the kernel answers").uid
}

/// A fixed enrollment time: 2000-01-02T03:04:05Z.
fn fixed_clock() -> u64 {
    946_782_245
}

/// A running provider over `data`, listening at a fresh path.
fn start(data: &Path, gate: Gate) -> PathBuf {
    let path = data.join(format!(
        "gantry-{}.sock",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after the epoch")
            .as_nanos()
    ));
    let listener = UnixListener::bind(&path).expect("a listening socket");
    let provider = Provider::with_clock(data.to_path_buf(), fixed_clock);
    std::thread::spawn(move || serve(&listener, &provider, gate));
    path
}

fn admitting() -> Gate {
    Gate {
        uid: our_uid(),
        idle: IDLE,
    }
}

/// A scripted client.
struct Client {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Client {
    fn connect(path: &Path) -> Self {
        let writer = UnixStream::connect(path).expect("connects");
        writer
            .set_read_timeout(Some(Duration::from_secs(20)))
            .expect("a timeout");
        let reader = BufReader::new(writer.try_clone().expect("a second handle"));
        Self { writer, reader }
    }

    fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).expect("sent");
    }

    fn line(&mut self) -> String {
        let mut line = String::new();
        self.reader.read_line(&mut line).expect("a reply");
        assert!(line.ends_with('\n'), "a reply is one terminated line: {line:?}");
        line
    }

    fn ask(&mut self, request: &str) -> Json {
        self.send(format!("{request}\n").as_bytes());
        let line = self.line();
        rivet_core::json::parse(line.trim_end()).expect("a reply is JSON")
    }

    /// The failure code of a reply, asserting it is a failure with a message.
    fn fails(&mut self, request: &str) -> String {
        let reply = self.ask(request);
        assert_eq!(reply.get("ok"), Some(&Json::Bool(false)), "{reply:?}");
        assert!(reply.get("message").and_then(Json::as_str).is_some(), "{reply:?}");
        reply.get("error").and_then(Json::as_str).expect("a code").to_owned()
    }

    fn member(&mut self, digest: &str, name: &str) -> Vec<u8> {
        let reply = self.ask(&format!(
            "{{\"v\":1,\"op\":\"member\",\"digest\":\"{digest}\",\"name\":\"{name}\"}}"
        ));
        assert_eq!(reply.get("ok"), Some(&Json::Bool(true)), "{reply:?}");
        let length = usize::try_from(reply.get("length").and_then(Json::as_integer).expect("a length"))
            .expect("fits");
        let mut bytes = vec![0u8; length];
        self.reader.read_exact(&mut bytes).expect("exactly the promised bytes");
        bytes
    }

    /// Whether the far end has closed with nothing more to say: the next read
    /// is end of file, or — when the far end closed with bytes of ours still
    /// unread, which is how a `too-long` line ends — a connection reset.
    fn closed(&mut self) -> bool {
        let mut rest = Vec::new();
        match self.reader.read_to_end(&mut rest) {
            Ok(0) => true,
            Ok(_) => false,
            Err(e) => e.kind() == std::io::ErrorKind::ConnectionReset && rest.is_empty(),
        }
    }
}

/// A hand-composed artifact: `kind`, with the named members, sealed by the
/// toy signer.
fn artifact(kind: Kind, members: &[(&str, &[u8])]) -> Vec<u8> {
    let (arch, platform) = if kind == Kind::apk() {
        ("any", Some("android"))
    } else {
        ("aarch64", None)
    };
    let mut builder = Builder::new("gantry", "0.1.0", arch, platform, kind).expect("a builder");
    let mut digest = ClaimsBlake3::new();
    for (name, bytes) in members {
        builder
            .add(NewMember::stored(name, 0, 0, bytes, &mut digest).expect("a member"))
            .expect("added");
    }
    let bytes: Vec<&[u8]> = members.iter().map(|(_, b)| *b).collect();
    builder
        .seal_in_memory(&mut ClaimsBlake3::new(), &mut Unsigned, &bytes)
        .expect("sealed")
}

/// Hold `bytes` in the store under `digest`, as the control plane lays it out.
fn hold(data: &Path, digest: &str, bytes: &[u8]) {
    let dir = data
        .join(rivet_core::repository_objects::DIR)
        .join("objects")
        .join(digest);
    std::fs::create_dir_all(&dir).expect("creatable");
    std::fs::write(dir.join("gantry-0.1.0.rvt"), bytes).expect("writable");
}

fn register(data: &Path, fleet_name: &str) {
    let root = data.join(crate::fleet::REGISTER_DIR);
    std::fs::create_dir_all(&root).expect("creatable");
    std::fs::write(root.join("register.toml"), format!("name = \"{fleet_name}\"\n"))
        .expect("writable");
}

/// A hand-composed P-256 SubjectPublicKeyInfo: the fixed prefix, `04`, and 64
/// bytes of `fill` that are not a real point.
fn p256_key(fill: u8) -> Vec<u8> {
    let mut key = P256_SPKI_PREFIX.to_vec();
    key.push(0x04);
    key.extend(std::iter::repeat_n(fill, 64));
    key
}

fn b64(bytes: &[u8]) -> String {
    rivet_core::base64::encode(bytes)
}

const APK_DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const APK_WITHOUT_ICON: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const APPLICATION_DIGEST: &str = "5555555555555555555555555555555555555555555555555555555555555555";
const ABSENT_DIGEST: &str = "9999999999999999999999999999999999999999999999999999999999999999";

/// A data directory holding a fleet name, two apks (one without an icon) and
/// one application the socket must not list.
fn fleet_server(label: &str) -> (Scratch, Vec<u8>) {
    let scratch = Scratch::new(label);
    register(&scratch.0, "home");
    let with_icon = artifact(
        Kind::apk(),
        &[
            ("app.apk", b"PK\x03\x04 an apk"),
            ("apk.json", b"{\"application_id\":\"x\"}"),
            ("icon.png", b"\x89PNG"),
        ],
    );
    hold(&scratch.0, APK_DIGEST, &with_icon);
    hold(
        &scratch.0,
        APK_WITHOUT_ICON,
        &artifact(Kind::apk(), &[("app.apk", b"PK\x03\x04"), ("apk.json", b"{}")]),
    );
    hold(
        &scratch.0,
        APPLICATION_DIGEST,
        &artifact(Kind::application(), &[("app.apk", b"not an apk")]),
    );
    (scratch, with_icon)
}

// ---- every operation -------------------------------------------------------

#[test]
fn fleet_answers_the_name_in_register_toml() {
    let (scratch, _) = fleet_server("fleet");
    let mut client = Client::connect(&start(&scratch.0, admitting()));
    let reply = client.ask("{\"v\":1,\"op\":\"fleet\"}");
    assert_eq!(reply.get("ok"), Some(&Json::Bool(true)));
    assert_eq!(reply.get("name").and_then(Json::as_str), Some("home"));
}

#[test]
fn artifacts_lists_only_the_held_apks_sorted_with_their_headers_as_stored() {
    let (scratch, with_icon) = fleet_server("artifacts");
    let mut client = Client::connect(&start(&scratch.0, admitting()));
    let reply = client.ask("{\"v\":1,\"op\":\"artifacts\"}");
    let listed = reply.get("artifacts").and_then(Json::as_array).expect("an array");
    let digests: Vec<&str> =
        listed.iter().map(|a| a.get("digest").and_then(Json::as_str).expect("a digest")).collect();
    assert_eq!(digests, [APK_WITHOUT_ICON, APK_DIGEST], "sorted, and the application is absent");
    let header = listed[1].get("header").and_then(Json::as_str).expect("a header");
    assert_eq!(
        rivet_core::base64::decode(header).expect("padded base64"),
        with_icon[..HEADER_BYTES].to_vec()
    );
}

#[test]
fn an_empty_store_is_an_empty_array() {
    let scratch = Scratch::new("empty");
    let mut client = Client::connect(&start(&scratch.0, admitting()));
    let reply = client.ask("{\"v\":1,\"op\":\"artifacts\"}");
    assert_eq!(reply.get("artifacts"), Some(&Json::Array(Vec::new())));
    let reply = client.ask("{\"v\":1,\"op\":\"phones\"}");
    assert_eq!(reply.get("phones"), Some(&Json::Array(Vec::new())));
}

#[test]
fn member_answers_a_length_line_and_exactly_those_bytes_and_the_connection_goes_on() {
    let (scratch, _) = fleet_server("member");
    let mut client = Client::connect(&start(&scratch.0, admitting()));
    assert_eq!(client.member(APK_DIGEST, "apk.json"), b"{\"application_id\":\"x\"}");
    assert_eq!(client.member(APK_DIGEST, "app.apk"), b"PK\x03\x04 an apk");
    assert_eq!(client.member(APK_DIGEST, "icon.png"), b"\x89PNG");
    // The next reply begins exactly where the last member ended.
    let reply = client.ask("{\"v\":1,\"op\":\"fleet\"}");
    assert_eq!(reply.get("name").and_then(Json::as_str), Some("home"));
}

/// An APK is megabytes, and it must arrive byte for byte.
#[test]
fn a_multi_megabyte_member_arrives_exact() {
    let scratch = Scratch::new("big");
    let big: Vec<u8> = (0..3 * 1024 * 1024 + 17).map(|i: u32| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    hold(&scratch.0, APK_DIGEST, &artifact(Kind::apk(), &[("apk.json", b"{}"), ("app.apk", &big)]));
    let mut client = Client::connect(&start(&scratch.0, admitting()));
    let got = client.member(APK_DIGEST, "app.apk");
    assert_eq!(got.len(), big.len());
    assert!(got == big, "the bytes differ");
}

#[test]
fn enroll_derives_the_identity_and_the_record_is_listed_renamed_and_revoked() {
    let (scratch, _) = fleet_server("lifecycle");
    let mut client = Client::connect(&start(&scratch.0, admitting()));
    let key = p256_key(0x11);
    let reply = client.ask(&format!(
        "{{\"v\":1,\"op\":\"enrol\",\"public_key\":\"{}\",\"label\":\"Pixel 8\"}}",
        b64(&key)
    ));
    assert_eq!(reply.get("ok"), Some(&Json::Bool(true)), "{reply:?}");
    let phone = reply.get("phone").expect("the record");
    let identity = phone.get("identity").and_then(Json::as_str).expect("an identity").to_owned();

    use sha2::Digest as _;
    let hashed: [u8; 32] = sha2::Sha256::digest(&key).into();
    assert_eq!(identity, rivetos_register::phones::identity_from_sha256(&hashed));
    assert_eq!(phone.get("label").and_then(Json::as_str), Some("Pixel 8"));
    assert_eq!(phone.get("public_key").and_then(Json::as_str), Some(b64(&key).as_str()));
    assert_eq!(phone.get("enrolled").and_then(Json::as_str), Some("2000-01-02T03:04:05Z"));
    assert!(
        scratch.0.join("register/phones").join(format!("{identity}.toml")).is_file(),
        "the record is on disk"
    );

    let listed = client.ask("{\"v\":1,\"op\":\"phones\"}");
    assert_eq!(listed.get("phones"), Some(&Json::Array(vec![phone.clone()])));

    let renamed = client.ask(&format!(
        "{{\"v\":1,\"op\":\"rename\",\"identity\":\"{identity}\",\"label\":\"Kitchen tablet\"}}"
    ));
    let renamed = renamed.get("phone").expect("the record");
    assert_eq!(renamed.get("label").and_then(Json::as_str), Some("Kitchen tablet"));
    assert_eq!(renamed.get("enrolled"), phone.get("enrolled"), "only the label changes");
    assert_eq!(renamed.get("public_key"), phone.get("public_key"), "only the label changes");

    let revoked = client.ask(&format!("{{\"v\":1,\"op\":\"revoke\",\"identity\":\"{identity}\"}}"));
    assert_eq!(revoked, Json::Object(vec![("ok".to_owned(), Json::Bool(true))]));
    let listed = client.ask("{\"v\":1,\"op\":\"phones\"}");
    assert_eq!(listed.get("phones"), Some(&Json::Array(Vec::new())));
}

/// `SPEC-06` §7, item 3: nothing is held in memory, so a new server over the same
/// data directory answers with the phone the old one enrolled.
#[test]
fn a_restart_between_enroll_and_phones_still_returns_the_phone() {
    let (scratch, _) = fleet_server("restart");
    let first = start(&scratch.0, admitting());
    let enrolled = Client::connect(&first).ask(&format!(
        "{{\"v\":1,\"op\":\"enrol\",\"public_key\":\"{}\",\"label\":\"Pixel 8\"}}",
        b64(&p256_key(0x22))
    ));
    let phone = enrolled.get("phone").expect("enrolled").clone();

    let second = start(&scratch.0, admitting());
    let listed = Client::connect(&second).ask("{\"v\":1,\"op\":\"phones\"}");
    assert_eq!(listed.get("phones"), Some(&Json::Array(vec![phone])));
}

/// A phone written by the other writer of `phones/` — the control plane — is
/// seen at the next request, because nothing is cached.
#[test]
fn a_phone_file_written_behind_the_socket_is_seen_at_the_next_request() {
    let (scratch, _) = fleet_server("nocache");
    let mut client = Client::connect(&start(&scratch.0, admitting()));
    let empty = client.ask("{\"v\":1,\"op\":\"phones\"}");
    assert_eq!(empty.get("phones"), Some(&Json::Array(Vec::new())));
    let key = p256_key(0x33);
    let phone = Phone::new(&identity_of(&key), "behind", key, "2000-01-02T03:04:05Z").expect("a phone");
    rivetos_register::phones::write(&scratch.0.join("register"), &phone).expect("writable");
    let listed = client.ask("{\"v\":1,\"op\":\"phones\"}");
    assert_eq!(listed.get("phones").and_then(Json::as_array).map(Vec::len), Some(1));
}

// ---- every failure code ----------------------------------------------------

/// Every code of `SPEC-06` §4 but `refused` and `too-long`, on ONE connection,
/// which stays usable after each.
#[test]
fn every_recoverable_failure_code_is_answered_and_the_connection_stays_usable() {
    let (scratch, _) = fleet_server("failures");
    let mut c = Client::connect(&start(&scratch.0, admitting()));
    let key = b64(&p256_key(0x44));

    // malformed
    for bad in [
        "not json",
        "[1,2]",
        "{\"v\":1,\"op\":\"fleet\"} {}",
        "{\"v\":1,\"op\":7}",
        "{\"v\":1}",
        "{\"v\":1,\"op\":\"fleet\",\"op\":\"phones\"}",
        "{\"v\":1,\"op\":\"member\",\"digest\":1,\"name\":\"app.apk\"}",
        "{\"v\":1,\"op\":\"enrol\",\"public_key\":\"AAAA\"}",
        "{\"v\":1,\"op\":\"rename\",\"identity\":\"x\",\"label\":[]}",
    ] {
        assert_eq!(c.fails(bad), "malformed", "{bad}");
    }
    let with_identity = format!(
        "{{\"v\":1,\"op\":\"enrol\",\"public_key\":\"{key}\",\"label\":\"p\",\"identity\":\"x\"}}"
    );
    assert_eq!(c.fails(&with_identity), "malformed");
    c.send(b"\xff\xfe\n");
    let line = c.line();
    assert!(line.contains("\"error\":\"malformed\""), "{line}");

    // version
    for bad in [
        "{\"op\":\"fleet\"}",
        "{\"v\":2,\"op\":\"fleet\"}",
        "{\"v\":\"1\",\"op\":\"fleet\"}",
        "{\"v\":1.5,\"op\":\"fleet\"}",
    ] {
        assert_eq!(c.fails(bad), "version", "{bad}");
    }

    // unknown-op
    assert_eq!(c.fails("{\"v\":1,\"op\":\"install\"}"), "unknown-op");

    // unknown-artifact: not a digest, not held, and held but not an apk
    for digest in ["nothex", ABSENT_DIGEST, APPLICATION_DIGEST, &APK_DIGEST.to_uppercase()] {
        let request = format!("{{\"v\":1,\"op\":\"member\",\"digest\":\"{digest}\",\"name\":\"app.apk\"}}");
        assert_eq!(c.fails(&request), "unknown-artifact", "{digest}");
    }

    // unknown-member: not a member name, and an icon the artifact lacks
    let request = format!("{{\"v\":1,\"op\":\"member\",\"digest\":\"{APK_DIGEST}\",\"name\":\"classes.dex\"}}");
    assert_eq!(c.fails(&request), "unknown-member");
    let request = format!("{{\"v\":1,\"op\":\"member\",\"digest\":\"{APK_WITHOUT_ICON}\",\"name\":\"icon.png\"}}");
    assert_eq!(c.fails(&request), "unknown-member");

    // unknown-phone
    let nobody = "A".repeat(43);
    for request in [
        format!("{{\"v\":1,\"op\":\"rename\",\"identity\":\"{nobody}\",\"label\":\"p\"}}"),
        format!("{{\"v\":1,\"op\":\"revoke\",\"identity\":\"{nobody}\"}}"),
        "{\"v\":1,\"op\":\"revoke\",\"identity\":\"../../register\"}".to_owned(),
    ] {
        assert_eq!(c.fails(&request), "unknown-phone", "{request}");
    }

    // bad-key: not base64, too short, wrong prefix, a compressed point, one byte long
    let mut wrong_prefix = p256_key(0x44);
    wrong_prefix[5] ^= 1;
    let mut compressed = p256_key(0x44);
    compressed[26] = 0x02;
    let mut long = p256_key(0x44);
    long.push(0);
    for bad in [
        "not base64!".to_owned(),
        b64(&p256_key(0x44)[..90]),
        b64(&wrong_prefix),
        b64(&compressed),
        b64(&long),
    ] {
        let request = format!("{{\"v\":1,\"op\":\"enrol\",\"public_key\":\"{bad}\",\"label\":\"p\"}}");
        assert_eq!(c.fails(&request), "bad-key", "{bad}");
    }

    // bad-label, at enroll and at rename
    for label in [String::new(), "x".repeat(65), "a\\nb".to_owned(), "a\\u0007b".to_owned()] {
        let request = format!("{{\"v\":1,\"op\":\"enrol\",\"public_key\":\"{key}\",\"label\":\"{label}\"}}");
        assert_eq!(c.fails(&request), "bad-label", "{label}");
    }
    let enrolled = c.ask(&format!("{{\"v\":1,\"op\":\"enrol\",\"public_key\":\"{key}\",\"label\":\"p\"}}"));
    let identity = enrolled
        .get("phone")
        .and_then(|p| p.get("identity"))
        .and_then(Json::as_str)
        .expect("enrolled")
        .to_owned();
    let request = format!("{{\"v\":1,\"op\":\"rename\",\"identity\":\"{identity}\",\"label\":\"\"}}");
    assert_eq!(c.fails(&request), "bad-label");

    // exists
    let again = format!("{{\"v\":1,\"op\":\"enrol\",\"public_key\":\"{key}\",\"label\":\"other\"}}");
    assert_eq!(c.fails(&again), "exists");

    // store: phones/ is a file, so the register cannot be read
    let phones = scratch.0.join("register/phones");
    std::fs::remove_dir_all(&phones).expect("removable");
    std::fs::write(&phones, b"not a directory").expect("writable");
    let code = c.fails("{\"v\":1,\"op\":\"phones\"}");
    assert_eq!(code, "store");
    let reply = c.ask("{\"v\":1,\"op\":\"phones\"}");
    assert!(
        reply.get("message").and_then(Json::as_str).is_some_and(|m| m.contains("phones")),
        "the message names the path: {reply:?}"
    );

    // And after all of that the connection still answers.
    let reply = c.ask("{\"v\":1,\"op\":\"fleet\"}");
    assert_eq!(reply.get("name").and_then(Json::as_str), Some("home"));
}

/// A line over 4096 bytes is `too-long`, and the connection is closed.
#[test]
fn a_request_line_over_4096_bytes_is_too_long_and_the_connection_closes() {
    let scratch = Scratch::new("too-long");
    let path = start(&scratch.0, admitting());

    // 4096 bytes including the newline is the limit, and is answered.
    let mut c = Client::connect(&path);
    let head = "{\"v\":1,\"op\":\"fleet\",\"pad\":\"";
    let tail = "\"}";
    let pad = "x".repeat(MAX_REQUEST - 1 - head.len() - tail.len());
    let exact = format!("{head}{pad}{tail}");
    assert_eq!(exact.len() + 1, MAX_REQUEST);
    let reply = c.ask(&exact);
    assert_eq!(reply.get("ok"), Some(&Json::Bool(true)), "{reply:?}");

    // One byte more is refused, and nothing further is answered.
    let over = format!("{head}{pad}x{tail}");
    assert_eq!(c.fails(&over), "too-long");
    assert!(c.closed(), "the connection is closed after too-long");

    // A line with no newline at all, past the limit, is the same.
    let mut c = Client::connect(&path);
    c.send(&vec![b'{'; MAX_REQUEST + 10]);
    let line = c.line();
    assert!(line.contains("\"error\":\"too-long\""), "{line}");
    assert!(c.closed());
}

/// `SPEC-06` §2: any other uid gets one `refused` line and is closed, before
/// anything it sends is read — here it sends nothing at all and still hears.
#[test]
fn a_caller_under_another_uid_is_refused_once_and_closed_before_its_request_is_read() {
    let (scratch, _) = fleet_server("refused");
    let gate = Gate {
        uid: our_uid().wrapping_add(1),
        idle: IDLE,
    };
    let mut c = Client::connect(&start(&scratch.0, gate));
    let line = c.line();
    let reply = rivet_core::json::parse(line.trim_end()).expect("JSON");
    assert_eq!(reply.get("ok"), Some(&Json::Bool(false)));
    assert_eq!(reply.get("error").and_then(Json::as_str), Some("refused"));
    assert!(c.closed(), "exactly one line, then the connection is closed");

    // And a refused caller that did send a request changes nothing.
    let mut c = Client::connect(&start(&scratch.0, gate));
    let key = b64(&p256_key(0x55));
    // **The write may meet a closed socket**, and that is the behaviour under
    // test rather than a failure of it: the server refuses and closes before
    // it reads a request, so a request written after the close gets EPIPE.
    // The refusal line is already in this end's buffer either way.
    let request = format!("{{\"v\":1,\"op\":\"enrol\",\"public_key\":\"{key}\",\"label\":\"p\"}}\n");
    match c.writer.write_all(request.as_bytes()) {
        Ok(()) => {}
        Err(e) if matches!(e.kind(), std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset) => {}
        Err(e) => panic!("sent: {e}"),
    }
    assert!(c.line().contains("\"error\":\"refused\""));
    assert!(!scratch.0.join("register/phones").exists(), "a refused request was acted on");
}

/// The same fixture under this process's own uid is admitted, so what the
/// refusal above turns on is the gate's uid.
#[test]
fn the_expected_uid_is_admitted() {
    let (scratch, _) = fleet_server("admitted");
    let mut c = Client::connect(&start(&scratch.0, admitting()));
    let reply = c.ask("{\"v\":1,\"op\":\"fleet\"}");
    assert_eq!(reply.get("ok"), Some(&Json::Bool(true)));
}

/// A connection that sends nothing is closed after the idle period. The
/// period is a parameter so this does not wait sixty seconds.
#[test]
fn a_connection_that_sends_nothing_is_closed_after_the_idle_period() {
    let scratch = Scratch::new("idle");
    let gate = Gate {
        uid: our_uid(),
        idle: Duration::from_millis(300),
    };
    let mut c = Client::connect(&start(&scratch.0, gate));
    let started = std::time::Instant::now();
    assert!(c.closed(), "the server closed the idle connection");
    assert!(started.elapsed() >= Duration::from_millis(250), "closed too soon");
}

/// The production gate is `gantry`'s derived uid and sixty seconds.
#[test]
fn the_production_gate_is_gantrys_uid_and_sixty_seconds() {
    let gate = Gate::for_consumer().expect("gantry is a valid service name");
    let name = rivet_core::service::ServiceName::new("gantry").expect("valid");
    assert_eq!(gate.uid, rivet_core::service::app_identity(&name).uid);
    assert_eq!(gate.idle, Duration::from_secs(60));
}

// ---- the key rule, without a socket ----------------------------------------

#[test]
fn only_a_91_byte_uncompressed_p256_spki_is_a_key() {
    assert!(is_p256_spki(&p256_key(0)));
    assert_eq!(p256_key(0).len(), P256_SPKI_BYTES);
    let mut compressed = p256_key(0);
    compressed[26] = 0x03;
    assert!(!is_p256_spki(&compressed));
    assert!(!is_p256_spki(&p256_key(0)[..90]));
    assert!(!is_p256_spki(&[]));
}
