//! **The relying party's state, files and refusals**, driven through
//! [`Passkeys::answer`] exactly as a browser's requests reach it.
//!
//! PROVENANCE: most ceremonies here come from a software authenticator in this
//! file — SELF-GENERATED, proving agreement with the author only
//! (`AGENTS.md` §4). The exception is `a_real_chrome_passkey_*`, which drives
//! `rivet-webauthn`'s ceremony CAPTURED FROM A REAL BROWSER (Chrome on
//! Windows, Windows Hello, at `dev-01.local`) through the relying party.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::captured_ceremony as captured;
use ring::signature::KeyPair as _;

/// The machine's `upstream-name`, and the first name on its certificate.
const HOST: &str = "dev-01.local";
/// A second name the same certificate carries.
const OTHER: &str = "dev-01.home-fleet.home.arpa";

/// A data directory of its own, removed when the test ends.
struct Dir(PathBuf);

impl Dir {
    fn new(what: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rivet-proxyd-passkey-{}-{what}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("a directory");
        Self(path)
    }
    fn code(&self) -> PathBuf {
        self.0.join(rivet_core::passkey::ENROLLMENT_FILE)
    }
    fn issue(&self, code: &str) {
        std::fs::write(self.code(), code).expect("a code");
    }
    fn age_code(&self, by: Duration) {
        let file = std::fs::File::options().write(true).open(self.code()).expect("open");
        file.set_modified(SystemTime::now() - by).expect("mtime");
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A relying party whose certificate carries [`HOST`] and [`OTHER`], and whose
/// `upstream-name` is [`HOST`].
fn relying_party(dir: &Dir) -> Passkeys {
    Passkeys::new(
        &dir.0,
        &[HOST.to_string(), OTHER.to_string()],
        HOST,
        rivet_sdk::journal::Journal::for_service("rivet-proxyd-test"),
    )
}

/// A string field of a flat JSON answer.
fn field(json: &str, name: &str) -> String {
    let key = format!("\"{name}\":\"");
    let at = json.find(&key).unwrap_or_else(|| panic!("{name} in {json}")) + key.len();
    json[at..].split('"').next().expect("a value").to_string()
}

fn head_at(host: &str, cookie: Option<&str>) -> String {
    match cookie {
        Some(c) => format!("POST /x HTTP/1.1\r\nHost: {host}\r\nCookie: {c}\r\n\r\n"),
        None => format!("POST /x HTTP/1.1\r\nHost: {host}\r\n\r\n"),
    }
}

fn head_with(cookie: Option<&str>) -> String {
    head_at(HOST, cookie)
}

/// `rivet-session=<token>` out of a `Set-Cookie`.
fn cookie_of(reply: &Reply) -> String {
    reply
        .set_cookie
        .as_ref()
        .expect("a cookie")
        .split(';')
        .next()
        .expect("a pair")
        .to_string()
}

// --- a software authenticator ----------------------------------------------------

struct Key {
    pair: ring::signature::Ed25519KeyPair,
    id: Vec<u8>,
}

impl Key {
    fn new(id: &[u8]) -> Self {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("a key");
        Self {
            pair: ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("a pair"),
            id: id.to_vec(),
        }
    }

    /// COSE_Key for Ed25519, by hand: {1: 1, 3: -8, -1: 6, -2: x}.
    fn cose(&self) -> Vec<u8> {
        let mut out = vec![0xa4, 0x01, 0x01, 0x03, 0x27, 0x20, 0x06, 0x21, 0x58, 0x20];
        out.extend_from_slice(self.pair.public_key().as_ref());
        out
    }

    fn auth_data(&self, name: &str, flags: u8, count: u32, attested: bool) -> Vec<u8> {
        let mut out = ring::digest::digest(&ring::digest::SHA256, name.as_bytes())
            .as_ref()
            .to_vec();
        out.push(flags);
        out.extend_from_slice(&count.to_be_bytes());
        if attested {
            out.extend_from_slice(&[0u8; 16]);
            out.extend_from_slice(&u16::try_from(self.id.len()).expect("short").to_be_bytes());
            out.extend_from_slice(&self.id);
            out.extend_from_slice(&self.cose());
        }
        out
    }

    /// The body `enroll.html` posts at [`HOST`], for a challenge.
    fn registration(&self, challenge: &str, flags: u8) -> Vec<u8> {
        self.registration_at(HOST, challenge, flags)
    }

    /// The body `enroll.html` posts at `name`, for a challenge.
    fn registration_at(&self, name: &str, challenge: &str, flags: u8) -> Vec<u8> {
        let origin = format!("https://{name}");
        let data = self.auth_data(name, flags, 0, true);
        // {"fmt": "none", "attStmt": {}, "authData": <bytes>}, by hand.
        let mut object = b"\xa3\x63fmt\x64none\x67attStmt\xa0\x68authData".to_vec();
        object.push(0x58);
        object.push(u8::try_from(data.len()).expect("under 256"));
        object.extend_from_slice(&data);
        let client = format!(
            r#"{{"type":"webauthn.create","challenge":"{challenge}","origin":"{origin}","crossOrigin":false}}"#
        );
        format!(
            r#"{{"clientDataJSON":"{}","attestationObject":"{}"}}"#,
            b64url(client.as_bytes()),
            b64url(&object)
        )
        .into_bytes()
    }

    /// The body `login.html` posts at [`HOST`], for a challenge.
    fn assertion(&self, challenge: &str, count: u32) -> Vec<u8> {
        self.assertion_at(HOST, challenge, count)
    }

    /// The body `login.html` posts at `name`, for a challenge.
    fn assertion_at(&self, name: &str, challenge: &str, count: u32) -> Vec<u8> {
        let data = self.auth_data(name, 0x05, count, false);
        let client = format!(
            r#"{{"type":"webauthn.get","challenge":"{challenge}","origin":"https://{name}"}}"#
        );
        let mut signed = data.clone();
        signed.extend_from_slice(
            ring::digest::digest(&ring::digest::SHA256, client.as_bytes()).as_ref(),
        );
        let signature = self.pair.sign(&signed);
        format!(
            r#"{{"id":"{}","clientDataJSON":"{}","authenticatorData":"{}","signature":"{}"}}"#,
            b64url(&self.id),
            b64url(client.as_bytes()),
            b64url(&data),
            b64url(signature.as_ref())
        )
        .into_bytes()
    }
}

const UP_UV_AT: u8 = 0x45;

fn post(rp: &Passkeys, path: &str, cookie: Option<&str>, body: &[u8]) -> Reply {
    rp.answer("POST", path, &head_with(cookie), body)
}

/// Enroll `key` with the code on file; the reply to the finishing request.
fn enroll_with_code(rp: &Passkeys, key: &Key, code: &str) -> Reply {
    let options = post(rp, "/passkey/enroll/options", None, format!(r#"{{"code":"{code}"}}"#).as_bytes());
    assert_eq!(options.status, 200, "{}", options.body);
    let challenge = field(&options.body, "challenge");
    post(rp, "/passkey/enroll", None, &key.registration(&challenge, UP_UV_AT))
}

fn sign_in(rp: &Passkeys, key: &Key, count: u32) -> Reply {
    let options = post(rp, "/passkey/login/options", None, b"");
    assert_eq!(options.status, 200, "{}", options.body);
    let challenge = field(&options.body, "challenge");
    post(rp, "/passkey/login", None, &key.assertion(&challenge, count))
}

// --- enrollment -------------------------------------------------------------------

#[test]
fn a_code_enrolls_one_passkey_signs_the_browser_in_and_is_gone() {
    let dir = Dir::new("enroll");
    let rp = relying_party(&dir);
    dir.issue("ABCDEFGHJK");
    let key = Key::new(b"first");

    let done = enroll_with_code(&rp, &key, "abcde-fghjk");
    assert_eq!(done.status, 200, "{}", done.body);
    let cookie = done.set_cookie.clone().expect("a session");
    for attribute in ["Secure", "HttpOnly", "SameSite=Strict", "Path=/", "Max-Age=43200"] {
        assert!(cookie.contains(attribute), "{cookie}");
    }
    assert!(!dir.code().exists(), "the code must be single-use");
    let stored = store::parse(&std::fs::read_to_string(dir.0.join(STORE_FILE)).expect("a store"))
        .expect("readable");
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].credential.id, b"first");
    assert!(rp.signed_in(&head_with(Some(&cookie_of(&done)))));

    // The same code again finds nothing waiting.
    let again = post(&rp, "/passkey/enroll/options", None, br#"{"code":"ABCDE-FGHJK"}"#);
    assert_eq!(again.status, 403, "{}", again.body);
}

#[test]
fn enrollment_without_a_code_or_a_session_is_refused() {
    let dir = Dir::new("nothing");
    let rp = relying_party(&dir);
    dir.issue("ABCDEFGHJK");
    let reply = post(&rp, "/passkey/enroll/options", None, b"{}");
    assert_eq!(reply.status, 403, "{}", reply.body);
    assert!(dir.code().exists(), "an empty offer is not a guess");
    // A session cookie nobody issued is no session.
    let forged = post(&rp, "/passkey/enroll/options", Some("rivet-session=00ff"), b"{}");
    assert_eq!(forged.status, 403);
}

#[test]
fn five_wrong_codes_delete_the_code() {
    let dir = Dir::new("guess");
    let rp = relying_party(&dir);
    dir.issue("ABCDEFGHJK");
    for n in 1..rivet_core::passkey::ENROLLMENT_ATTEMPTS {
        let reply = post(&rp, "/passkey/enroll/options", None, br#"{"code":"ZZZZZ-ZZZZZ"}"#);
        assert_eq!(reply.status, 403, "{n}");
        assert!(dir.code().exists(), "still there after {n}");
    }
    let last = post(&rp, "/passkey/enroll/options", None, br#"{"code":"ZZZZZ-ZZZZZ"}"#);
    assert_eq!(last.status, 403);
    assert!(!dir.code().exists(), "the code must be deleted at the limit");
    // And the right code is now worth nothing.
    let right = post(&rp, "/passkey/enroll/options", None, br#"{"code":"ABCDE-FGHJK"}"#);
    assert_eq!(right.status, 403);
}

#[test]
fn an_expired_code_is_refused_and_deleted() {
    let dir = Dir::new("expired");
    let rp = relying_party(&dir);
    dir.issue("ABCDEFGHJK");
    dir.age_code(Duration::from_secs(rivet_core::passkey::ENROLLMENT_LIFETIME_SECONDS + 5));
    let reply = post(&rp, "/passkey/enroll/options", None, br#"{"code":"ABCDE-FGHJK"}"#);
    assert_eq!(reply.status, 403, "{}", reply.body);
    assert!(reply.body.contains("expired"), "{}", reply.body);
    assert!(!dir.code().exists());
}

#[test]
fn a_code_replaced_during_the_ceremony_is_not_honoured() {
    let dir = Dir::new("replaced");
    let rp = relying_party(&dir);
    dir.issue("ABCDEFGHJK");
    let key = Key::new(b"k");
    let options = post(&rp, "/passkey/enroll/options", None, br#"{"code":"ABCDE-FGHJK"}"#);
    let challenge = field(&options.body, "challenge");
    // A new code is issued between the two halves.
    std::fs::remove_file(dir.code()).expect("gone");
    dir.issue("0123456789");
    dir.age_code(Duration::from_secs(1));
    let done = post(&rp, "/passkey/enroll", None, &key.registration(&challenge, UP_UV_AT));
    assert_eq!(done.status, 403, "{}", done.body);
    assert!(dir.code().exists(), "the new code is untouched");
    assert!(!dir.0.join(STORE_FILE).exists());
}

#[test]
fn a_registration_without_user_verification_is_refused_and_keeps_the_code() {
    let dir = Dir::new("no-uv");
    let rp = relying_party(&dir);
    dir.issue("ABCDEFGHJK");
    let key = Key::new(b"k");
    let options = post(&rp, "/passkey/enroll/options", None, br#"{"code":"ABCDEFGHJK"}"#);
    let challenge = field(&options.body, "challenge");
    let done = post(&rp, "/passkey/enroll", None, &key.registration(&challenge, 0x41));
    assert_eq!(done.status, 403, "{}", done.body);
    assert!(dir.code().exists(), "a refused ceremony does not spend the code");
}

#[test]
fn a_signed_in_operator_enrolls_another_passkey_without_a_code() {
    let dir = Dir::new("second");
    let rp = relying_party(&dir);
    dir.issue("ABCDEFGHJK");
    let first = enroll_with_code(&rp, &Key::new(b"first"), "ABCDEFGHJK");
    let cookie = cookie_of(&first);

    let options = post(&rp, "/passkey/enroll/options", Some(&cookie), b"{}");
    assert_eq!(options.status, 200, "{}", options.body);
    assert!(options.body.contains(&b64url(b"first")), "the first is excluded");
    let challenge = field(&options.body, "challenge");
    let second = Key::new(b"second");
    let done = post(&rp, "/passkey/enroll", Some(&cookie), &second.registration(&challenge, UP_UV_AT));
    assert_eq!(done.status, 200, "{}", done.body);
    let stored = rp.load().expect("readable");
    assert_eq!(stored.len(), 2);
}

// --- sign-in ---------------------------------------------------------------------

#[test]
fn an_enrolled_passkey_signs_in_and_its_counter_is_kept() {
    let dir = Dir::new("login");
    let rp = relying_party(&dir);
    dir.issue("ABCDEFGHJK");
    let key = Key::new(b"k");
    enroll_with_code(&rp, &key, "ABCDEFGHJK");

    let done = sign_in(&rp, &key, 7);
    assert_eq!(done.status, 200, "{}", done.body);
    assert!(rp.signed_in(&head_with(Some(&cookie_of(&done)))));
    assert_eq!(rp.load().expect("store")[0].credential.sign_count, 7);

    // A counter that does not increase is refused.
    let stale = sign_in(&rp, &key, 7);
    assert_eq!(stale.status, 403, "{}", stale.body);
    assert!(stale.body.contains("counter"), "{}", stale.body);
    assert_eq!(sign_in(&rp, &key, 8).status, 200);
}

#[test]
fn sign_in_is_refused_for_each_way_it_can_be_wrong() {
    let dir = Dir::new("login-wrong");
    let rp = relying_party(&dir);
    // Nothing enrolled: the options say so, and where to enroll one.
    let none = post(&rp, "/passkey/login/options", None, b"");
    assert_eq!(none.status, 409);
    assert!(none.body.contains(&format!("https://{HOST}/sign-in/enroll")), "{}", none.body);

    dir.issue("ABCDEFGHJK");
    let key = Key::new(b"k");
    enroll_with_code(&rp, &key, "ABCDEFGHJK");

    // A challenge is single-use.
    let options = post(&rp, "/passkey/login/options", None, b"");
    let challenge = field(&options.body, "challenge");
    assert_eq!(post(&rp, "/passkey/login", None, &key.assertion(&challenge, 1)).status, 200);
    assert_eq!(post(&rp, "/passkey/login", None, &key.assertion(&challenge, 2)).status, 403);

    // A challenge this machine never issued.
    let invented = b64url(&[7u8; 32]);
    assert_eq!(post(&rp, "/passkey/login", None, &key.assertion(&invented, 3)).status, 403);

    // A key that is not enrolled.
    let stranger = Key::new(b"stranger");
    let options = post(&rp, "/passkey/login/options", None, b"");
    let challenge = field(&options.body, "challenge");
    assert_eq!(post(&rp, "/passkey/login", None, &stranger.assertion(&challenge, 1)).status, 403);

    // The right ID signed by the wrong key.
    let impostor = Key::new(b"k");
    let options = post(&rp, "/passkey/login/options", None, b"");
    let challenge = field(&options.body, "challenge");
    assert_eq!(post(&rp, "/passkey/login", None, &impostor.assertion(&challenge, 9)).status, 403);

    // An enrollment challenge cannot finish a sign-in.
    dir.issue("0123456789");
    let options = post(&rp, "/passkey/enroll/options", None, br#"{"code":"0123456789"}"#);
    let challenge = field(&options.body, "challenge");
    assert_eq!(post(&rp, "/passkey/login", None, &key.assertion(&challenge, 10)).status, 403);

    // A body that is not an assertion.
    assert_eq!(post(&rp, "/passkey/login", None, b"{}").status, 400);
}

#[test]
fn logging_out_ends_the_session() {
    let dir = Dir::new("logout");
    let rp = relying_party(&dir);
    dir.issue("ABCDEFGHJK");
    let done = enroll_with_code(&rp, &Key::new(b"k"), "ABCDEFGHJK");
    let cookie = cookie_of(&done);
    assert!(rp.signed_in(&head_with(Some(&cookie))));
    let out = post(&rp, "/logout", Some(&cookie), b"");
    assert!(out.set_cookie.as_deref().is_some_and(|c| c.contains("Max-Age=0")));
    assert!(!rp.signed_in(&head_with(Some(&cookie))));
}

#[test]
fn a_store_that_does_not_read_is_an_error_and_not_an_empty_store() {
    let dir = Dir::new("broken");
    let rp = relying_party(&dir);
    std::fs::write(dir.0.join(STORE_FILE), "passkey nonsense\n").expect("write");
    assert_eq!(post(&rp, "/passkey/login/options", None, b"").status, 500);
}

// --- one relying party per certificate name ------------------------------------------

fn post_at(rp: &Passkeys, host: &str, path: &str, cookie: Option<&str>, body: &[u8]) -> Reply {
    rp.answer("POST", path, &head_at(host, cookie), body)
}

fn enroll_at(rp: &Passkeys, host: &str, key: &Key, code: &str) -> Reply {
    let options = post_at(rp, host, "/passkey/enroll/options", None, format!(r#"{{"code":"{code}"}}"#).as_bytes());
    assert_eq!(options.status, 200, "{}", options.body);
    assert!(options.body.contains(&format!(r#""rp":{{"id":"{host}""#)), "{}", options.body);
    let challenge = field(&options.body, "challenge");
    post_at(rp, host, "/passkey/enroll", None, &key.registration_at(host, &challenge, UP_UV_AT))
}

fn sign_in_at(rp: &Passkeys, host: &str, key: &Key, count: u32) -> Reply {
    let options = post_at(rp, host, "/passkey/login/options", None, b"");
    assert_eq!(options.status, 200, "{}", options.body);
    let challenge = field(&options.body, "challenge");
    post_at(rp, host, "/passkey/login", None, &key.assertion_at(host, &challenge, count))
}

#[test]
fn a_host_the_certificate_does_not_carry_is_refused_before_anything_is_issued() {
    let dir = Dir::new("stranger-host");
    let rp = relying_party(&dir);
    dir.issue("ABCDEFGHJK");
    for head in [
        "GET /x HTTP/1.1\r\nHost: evil.example\r\n\r\n".to_string(),
        "GET /x HTTP/1.1\r\nHost: 192.168.50.66\r\n\r\n".to_string(),
        "GET /x HTTP/1.1\r\nHost: dev-01.local.\r\n\r\n".to_string(),
        "GET /x HTTP/1.1\r\n\r\n".to_string(),
        format!("GET /x HTTP/1.1\r\nHost: {HOST}\r\nHost: {OTHER}\r\n\r\n"),
    ] {
        for (method, path) in [
            ("POST", "/passkey/login/options"),
            ("POST", "/passkey/enroll/options"),
        ] {
            let reply = rp.answer(method, path, &head, br#"{"code":"ABCDEFGHJK"}"#);
            assert_eq!(reply.status, 421, "{head:?} {path}: {}", reply.body);
            assert!(reply.body.contains(HOST) && reply.body.contains(OTHER), "{}", reply.body);
        }
        for page in ["", "/enroll"] {
            let reply = rp.sign_in("GET", page, &head);
            assert_eq!(reply.status, 421, "{head:?} /sign-in{page}: {}", reply.body);
            assert!(reply.body.contains(HOST) && reply.body.contains(OTHER), "{}", reply.body);
        }
    }
    assert!(dir.code().exists(), "a refused host spends nothing");
    assert!(rp.state().challenges.is_empty(), "a refused host is issued nothing");

    // Its port and its case are not part of the name.
    for host in [format!("{HOST}:443"), "DEV-01.Local".to_string(), format!("{OTHER}:8443")] {
        let head = format!("GET /x HTTP/1.1\r\nHost: {host}\r\n\r\n");
        assert_eq!(rp.sign_in("GET", "", &head).status, 200, "{host}");
    }
}

#[test]
fn a_passkey_enrolled_at_one_name_signs_in_there_and_is_refused_at_the_other() {
    let dir = Dir::new("two-names");
    let rp = relying_party(&dir);
    dir.issue("ABCDEFGHJK");
    let phone = Key::new(b"phone");
    let done = enroll_at(&rp, OTHER, &phone, "ABCDEFGHJK");
    assert_eq!(done.status, 200, "{}", done.body);

    // The store records the name.
    let text = std::fs::read_to_string(dir.0.join(STORE_FILE)).expect("a store");
    assert!(text.lines().nth(1).is_some_and(|l| l.ends_with(&format!(" {OTHER}"))), "{text}");
    assert_eq!(rp.load().expect("store")[0].rp_id.as_deref(), Some(OTHER));

    // Offered and accepted at its own name, with that name as the RP ID.
    let options = post_at(&rp, OTHER, "/passkey/login/options", None, b"");
    assert_eq!(field(&options.body, "rpId"), OTHER);
    assert!(options.body.contains(&b64url(b"phone")), "{}", options.body);
    assert_eq!(sign_in_at(&rp, OTHER, &phone, 1).status, 200);

    // Not offered at the other name, where nothing is enrolled yet.
    let elsewhere = post_at(&rp, HOST, "/passkey/login/options", None, b"");
    assert_eq!(elsewhere.status, 409, "{}", elsewhere.body);
    assert!(elsewhere.body.contains(HOST), "{}", elsewhere.body);

    // And refused there even when a login challenge is live: the store says
    // which name it belongs to, before any signature is checked.
    dir.issue("0123456789");
    let desk = Key::new(b"desk");
    assert_eq!(enroll_at(&rp, HOST, &desk, "0123456789").status, 200);
    let options = post_at(&rp, HOST, "/passkey/login/options", None, b"");
    assert_eq!(field(&options.body, "rpId"), HOST);
    assert!(options.body.contains(&b64url(b"desk")), "{}", options.body);
    assert!(!options.body.contains(&b64url(b"phone")), "{}", options.body);
    let challenge = field(&options.body, "challenge");
    let refused = post_at(&rp, HOST, "/passkey/login", None, &phone.assertion_at(HOST, &challenge, 2));
    assert_eq!(refused.status, 403, "{}", refused.body);
    assert!(refused.body.contains(&format!("enrolled at {OTHER}")), "{}", refused.body);
}

#[test]
fn a_challenge_is_answered_only_at_the_name_it_was_issued_at() {
    let dir = Dir::new("challenge-name");
    let rp = relying_party(&dir);
    dir.issue("ABCDEFGHJK");
    let key = Key::new(b"k");
    assert_eq!(enroll_at(&rp, OTHER, &key, "ABCDEFGHJK").status, 200);
    let options = post_at(&rp, OTHER, "/passkey/login/options", None, b"");
    let challenge = field(&options.body, "challenge");
    // Signed correctly for OTHER, posted at HOST: the challenge is not HOST's.
    let moved = post_at(&rp, HOST, "/passkey/login", None, &key.assertion_at(OTHER, &challenge, 1));
    assert_eq!(moved.status, 403, "{}", moved.body);
    assert!(moved.body.contains("another name"), "{}", moved.body);
    // And it was spent by the attempt.
    let late = post_at(&rp, OTHER, "/passkey/login", None, &key.assertion_at(OTHER, &challenge, 1));
    assert_eq!(late.status, 403, "{}", late.body);
}

#[test]
fn a_session_counts_only_at_the_name_it_was_granted_at() {
    let dir = Dir::new("session-name");
    let rp = relying_party(&dir);
    dir.issue("ABCDEFGHJK");
    let done = enroll_at(&rp, HOST, &Key::new(b"desk"), "ABCDEFGHJK");
    let cookie = cookie_of(&done);
    assert!(!done.set_cookie.as_deref().is_some_and(|c| c.contains("Domain")), "host-only");
    assert!(rp.signed_in(&head_at(HOST, Some(&cookie))));
    assert!(!rp.signed_in(&head_at(OTHER, Some(&cookie))));

    // So it enrolls another passkey at its own name without a code, and not
    // at the other name.
    let here = post_at(&rp, HOST, "/passkey/enroll/options", Some(&cookie), b"{}");
    assert_eq!(here.status, 200, "{}", here.body);
    let there = post_at(&rp, OTHER, "/passkey/enroll/options", Some(&cookie), b"{}");
    assert_eq!(there.status, 403, "{}", there.body);
    assert!(there.body.contains(OTHER), "{}", there.body);
}

/// **A store written before names were recorded belongs to `upstream-name`.**
/// The line is the one dev-01's proxy wrote when the maintainer enrolled
/// from Chrome on Windows (a public key; nothing secret).
#[test]
fn an_old_store_is_offered_at_the_upstream_name_only_and_is_not_rewritten() {
    let dir = Dir::new("old-store");
    let old = "# rivet-proxyd passkeys: passkey <id> <algorithm> <public key> <counter> <enrolled>\n\
               passkey fAbOmZ85O5HvF6gegifR4w es256 BGNEoOCgId5XDI7EjIdHuaEYKMixYDNrDFSN8er6i6zsBtL4Ef54DA-xR8Iuu6xjv449TJwHhkGuQHOorLrLs-A 0 1790730159\n";
    std::fs::write(dir.0.join(STORE_FILE), old).expect("write");
    let rp = relying_party(&dir);
    let here = post_at(&rp, HOST, "/passkey/login/options", None, b"");
    assert_eq!(here.status, 200, "{}", here.body);
    assert_eq!(field(&here.body, "rpId"), HOST);
    assert!(here.body.contains("fAbOmZ85O5HvF6gegifR4w"), "{}", here.body);
    let there = post_at(&rp, OTHER, "/passkey/login/options", None, b"");
    assert_eq!(there.status, 409, "{}", there.body);
    assert_eq!(std::fs::read_to_string(dir.0.join(STORE_FILE)).expect("read"), old);
}

/// **A REAL passkey still signs in at its own name and fails at the other.**
/// PROVENANCE: CAPTURED FROM A REAL BROWSER — `rivet-webauthn`'s Chrome on
/// Windows ceremony at `dev-01.local`, stored in the format written before
/// names were recorded, and answered through the relying party's own state.
#[test]
fn a_real_chrome_passkey_signs_in_at_its_name_and_is_refused_at_the_other() {
    let dir = Dir::new("captured");
    let b = |text: &str| rivet_webauthn::from_b64url(text).expect("base64url");
    let origin = RelyingParty::origin_for(captured::RP_ID);
    let credential = rivet_webauthn::verify_registration(
        RelyingParty { id: captured::RP_ID, origin: &origin },
        UserVerification::Required,
        &b(captured::CREATE_CHALLENGE),
        &b(captured::CREATE_CLIENT_DATA),
        &b(captured::CREATE_ATTESTATION),
    )
    .expect("the captured registration verifies");
    let old = store::render(&[store::Stored { credential, enrolled: 1, rp_id: None }]);
    assert_eq!(old.lines().nth(1).map(|l| l.split(' ').count()), Some(6), "the old format");
    std::fs::write(dir.0.join(STORE_FILE), &old).expect("write");
    assert_eq!(HOST, captured::RP_ID);
    let rp = relying_party(&dir);

    let body = format!(
        r#"{{"id":"{}","clientDataJSON":"{}","authenticatorData":"{}","signature":"{}"}}"#,
        captured::CREDENTIAL_ID,
        captured::GET_CLIENT_DATA,
        captured::GET_AUTH_DATA,
        captured::GET_SIGNATURE
    );

    // At the other name it is not offered, and the same bytes are refused.
    assert_eq!(post_at(&rp, OTHER, "/passkey/login/options", None, b"").status, 409);
    rp.plant_login_challenge(b(captured::GET_CHALLENGE), OTHER);
    let there = post_at(&rp, OTHER, "/passkey/login", None, body.as_bytes());
    assert_eq!(there.status, 403, "{}", there.body);
    assert!(there.body.contains(&format!("enrolled at {HOST}")), "{}", there.body);

    // At its own name it signs in.
    rp.plant_login_challenge(b(captured::GET_CHALLENGE), HOST);
    let here = post_at(&rp, HOST, "/passkey/login", None, body.as_bytes());
    assert_eq!(here.status, 200, "{}", here.body);
    assert!(rp.signed_in(&head_at(HOST, Some(&cookie_of(&here)))));
    // A counter of zero is unchanged, so the old line is not rewritten.
    assert_eq!(std::fs::read_to_string(dir.0.join(STORE_FILE)).expect("read"), old);
}

// --- the pages and the paths -------------------------------------------------------

#[test]
fn the_pages_are_read_and_the_endpoints_are_posted() {
    let dir = Dir::new("paths");
    let rp = relying_party(&dir);
    let head = head_with(None);
    for page in ["", "/"] {
        let login = rp.sign_in("GET", page, &head);
        assert_eq!(login.status, 200, "/sign-in{page}");
        assert!(login.content_type.starts_with("text/html"));
        assert!(login.body.contains("navigator.credentials.get"));
        assert_eq!(rp.sign_in("HEAD", page, &head).status, 200, "/sign-in{page}");
    }
    for page in ["/enroll", "/enroll/"] {
        let enroll = rp.sign_in("GET", page, &head);
        assert_eq!(enroll.status, 200, "/sign-in{page}");
        assert!(enroll.body.contains("navigator.credentials.create"));
    }
    assert_eq!(rp.sign_in("POST", "", &head).status, 405);
    assert_eq!(rp.sign_in("POST", "/enroll", &head).status, 405);
    assert_eq!(rp.sign_in("GET", "/nothing", &head).status, 404);
    // The pages are not answered under the hidden prefix any longer; the
    // proxy moves those addresses before they reach here.
    assert_eq!(rp.answer("GET", "/login", &head, b"").status, 404);
    assert_eq!(rp.answer("GET", "/enrol", &head, b"").status, 404);
    assert_eq!(rp.answer("GET", "/passkey/login", &head, b"").status, 405);
    assert_eq!(rp.answer("GET", "/nothing", &head, b"").status, 404);
    assert_eq!(rp.answer("GET", "", &head, b"").status, 404);
}

/// **The pages link to each other at their new addresses**, and the calls
/// they make stay under the hidden prefix — `.rulings/` R-249.
#[test]
fn the_pages_link_to_each_other_at_the_sign_in_addresses() {
    assert!(LOGIN_HTML.contains("href=\"/sign-in/enroll\""));
    assert!(ENROLL_HTML.contains("href=\"/sign-in\""));
    for page in [LOGIN_HTML, ENROLL_HTML] {
        assert!(!page.contains("/.rivet/login") && !page.contains("/.rivet/enrol\""));
    }
    assert!(LOGIN_HTML.contains("\"/.rivet/passkey/login/options\""));
    assert!(ENROLL_HTML.contains("\"/.rivet/passkey/enroll/options\""));
}

/// **The pages fetch nothing from anywhere else** — `ADR-046` §6's rule for the
/// index, for the same reason: a house LAN may have no internet.
#[test]
fn the_pages_fetch_nothing_from_another_origin() {
    for page in [LOGIN_HTML, ENROLL_HTML] {
        assert!(!page.contains("http://") && !page.contains("https://"), "an absolute URL");
        assert!(!page.contains("<script src"), "an external script");
    }
}

// --- the cookie --------------------------------------------------------------------

#[test]
fn the_session_cookie_is_found_and_removed_by_name_alone() {
    assert_eq!(
        session_token("GET / HTTP/1.1\r\nCookie: a=1; rivet-session=abc; b=2\r\n\r\n").as_deref(),
        Some("abc")
    );
    assert_eq!(session_token("GET / HTTP/1.1\r\nCookie: rivet-sessionx=abc\r\n\r\n"), None);
    assert_eq!(session_token("GET / HTTP/1.1\r\nCookie: rivet-session=\r\n\r\n"), None);
    assert_eq!(without_session(" a=1; rivet-session=abc; b=2").as_deref(), Some("a=1; b=2"));
    assert_eq!(without_session(" rivet-session=abc"), None);
    assert_eq!(
        without_session(" rivet-sessionx=1").as_deref(),
        Some("rivet-sessionx=1")
    );
}

#[test]
fn json_strings_are_escaped() {
    assert_eq!(json_string("a\"b\\c\n"), "\"a\\\"b\\\\c\\u000a\"");
}
