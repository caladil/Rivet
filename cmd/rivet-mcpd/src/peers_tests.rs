//! The live `peers` reload: the decision, every refusal, and a real handshake
//! through [`crate::transport::Tls`] before and after the file changes.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};

use super::*;
use crate::transport::{Tls, Transport as _};

const A: Pin = [0xAA; rivetos_tls::KEY_BYTES];
const B: Pin = [0xBB; rivetos_tls::KEY_BYTES];
const C: Pin = [0xCC; rivetos_tls::KEY_BYTES];

fn hex(key: &Pin) -> String {
    rivetos_tls::encode_hex(key)
}

fn file_of(keys: &[Pin]) -> String {
    keys.iter().map(|k| format!("{}\n", hex(k))).collect()
}

/// A pin file in its own directory, and a [`LivePeers`] over it that has
/// loaded `keys`.
struct Fixture {
    dir: PathBuf,
    live: LivePeers,
    verifier: Arc<PinnedPeer>,
}

impl Fixture {
    fn new(label: &str, keys: &[Pin]) -> Self {
        let dir = std::env::temp_dir()
            .join(format!("rivet-mcpd-peers-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let path = dir.join("peers");
        let text = file_of(keys);
        std::fs::write(&path, &text).expect("the pin file");
        let verifier = Arc::new(
            PinnedPeer::new(keys.to_vec(), rivetos_tls::provider()).expect("a verifier"),
        );
        let live = LivePeers::new(&path, Arc::clone(&verifier), text);
        Self { dir, live, verifier }
    }

    fn write(&self, text: &str) {
        std::fs::write(self.live.path(), text).expect("rewrite the pin file");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// --- the decision ------------------------------------------------------------

#[test]
fn a_valid_file_naming_the_caller_is_its_keys() {
    assert_eq!(judge(&file_of(&[A, B]), Some(&A)), Ok(vec![A, B]));
}

#[test]
fn a_file_that_does_not_parse_is_refused() {
    let why = judge("not a key\n", Some(&A)).expect_err("garbage was accepted");
    assert!(why.contains("does not parse"), "{why}");
}

#[test]
fn a_file_naming_no_keys_is_refused() {
    let why = judge("# nobody\n\n", None).expect_err("an empty set was accepted");
    assert!(why.contains("names no keys"), "{why}");
}

#[test]
fn a_file_omitting_the_caller_is_refused_and_names_the_key() {
    let why = judge(&file_of(&[B]), Some(&A)).expect_err("the caller was locked out");
    assert!(why.contains(&hex(&A)), "{why}");
    assert!(why.contains("lock that operator out"), "{why}");
}

#[test]
fn additions_and_removals_are_counted_as_a_set() {
    assert_eq!(difference(&[A, B], &[A, C]), (1, 1));
    assert_eq!(difference(&[A], &[A, B, C]), (2, 0));
    assert_eq!(difference(&[A, B], &[B, A]), (0, 0));
}

// --- the file, the lock and the verifier -------------------------------------

#[test]
fn an_unchanged_file_is_not_reloaded() {
    let f = Fixture::new("unchanged", &[A]);
    assert_eq!(f.live.look(Some(&hex(&A))), Outcome::Unchanged);
}

#[test]
fn an_added_key_is_in_force_after_one_look() {
    let f = Fixture::new("added", &[A]);
    f.write(&file_of(&[A, B]));
    assert_eq!(
        f.live.look(Some(&hex(&A))),
        Outcome::Loaded { added: 1, removed: 0, now: 2 }
    );
    assert_eq!(f.verifier.pins(), vec![A, B]);
    assert_eq!(f.live.look(Some(&hex(&A))), Outcome::Unchanged, "loaded twice");
}

#[test]
fn a_removed_key_is_out_of_force_after_one_look() {
    let f = Fixture::new("removed", &[A, B]);
    f.write(&file_of(&[A]));
    assert_eq!(
        f.live.look(Some(&hex(&A))),
        Outcome::Loaded { added: 0, removed: 1, now: 1 }
    );
    assert_eq!(f.verifier.pins(), vec![A]);
}

#[test]
fn a_malformed_file_leaves_the_old_set_and_is_refused_once() {
    let f = Fixture::new("malformed", &[A, B]);
    f.write("aabb\n");
    assert!(matches!(f.live.look(Some(&hex(&A))), Outcome::Refused(_)));
    assert_eq!(f.verifier.pins(), vec![A, B], "a refused file changed the set");
    assert_eq!(f.live.look(Some(&hex(&A))), Outcome::Unchanged, "refused into the journal twice");
}

#[test]
fn an_empty_file_leaves_the_old_set() {
    let f = Fixture::new("empty", &[A]);
    f.write("");
    assert!(matches!(f.live.look(Some(&hex(&A))), Outcome::Refused(_)));
    assert_eq!(f.verifier.pins(), vec![A]);
}

#[test]
fn a_missing_file_leaves_the_old_set() {
    let f = Fixture::new("missing", &[A]);
    std::fs::remove_file(f.live.path()).expect("remove the pin file");
    match f.live.look(Some(&hex(&A))) {
        Outcome::Refused(why) => assert!(why.contains("cannot be read"), "{why}"),
        other => panic!("a missing file was {other:?}"),
    }
    assert_eq!(f.verifier.pins(), vec![A]);
}

#[test]
fn a_file_dropping_the_callers_key_leaves_the_old_set() {
    let f = Fixture::new("lockout", &[A, B]);
    f.write(&file_of(&[B]));
    assert!(matches!(f.live.look(Some(&hex(&A))), Outcome::Refused(_)));
    assert_eq!(f.verifier.pins(), vec![A, B], "the caller was locked out");
}

/// **A refusal that depends on who looked is not a verdict on the file.** B
/// removing A, observed first by A's own session, is refused there — and
/// loaded when B's session answers, because B is the one the file keeps.
#[test]
fn a_change_refused_for_one_caller_is_loaded_for_the_caller_it_keeps() {
    let f = Fixture::new("two-callers", &[A, B]);
    f.write(&file_of(&[B]));
    assert!(matches!(f.live.look(Some(&hex(&A))), Outcome::Refused(_)));
    assert_eq!(
        f.live.look(Some(&hex(&B))),
        Outcome::Loaded { added: 0, removed: 1, now: 1 }
    );
    assert_eq!(f.verifier.pins(), vec![B]);
}

// --- a real handshake, through this crate's transport -------------------------

/// Mint an Ed25519 self-signed leaf, as `cargo rivet identity new` does. See
/// `crate::transport`'s tests for the `04 22 04 20`.
fn mint(name: &str) -> rivetos_tls::Identity {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("an Ed25519 pair");
    let params = rcgen::CertificateParams::new(vec![name.to_string()]).expect("params");
    let cert = params.self_signed(&key).expect("a self-signed leaf");
    let der = key.serialize_der();
    let at = der
        .windows(4)
        .position(|w| w == [0x04, 0x22, 0x04, 0x20])
        .expect("PKCS#8 for Ed25519 wraps the seed in 04 22 04 20")
        + 4;
    let mut seed = [0u8; rivetos_tls::KEY_BYTES];
    seed.copy_from_slice(&der[at..at + rivetos_tls::KEY_BYTES]);
    rivetos_tls::Identity { certificate: cert.der().to_vec(), seed }
}

/// One connection from `who` to a board listening with `config`: `Ok` with the
/// key the board saw, or the board's refusal.
fn connect(
    config: &Arc<rustls::ServerConfig>,
    who: &rivetos_tls::Identity,
    board: &rivetos_tls::Identity,
) -> Result<String, String> {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let address = listener.local_addr().expect("its address");
    let config = Arc::clone(config);
    let board_side = std::thread::spawn(move || -> Result<String, String> {
        let (stream, _) = listener.accept().map_err(|e| e.to_string())?;
        let link = Tls::accept(stream, config).map_err(|e| e.to_string())?;
        let mut asked = [0u8; 16];
        let _ = link.read(&mut asked);
        link.write_line("answered").map_err(|e| e.to_string())?;
        link.finish();
        link.peer_key().cloned().ok_or_else(|| "no key".to_string())
    });

    let client = rivetos_tls::client_config(
        who,
        vec![board.public_key().expect("a key")],
        rivetos_tls::provider(),
    )
    .expect("a client config");
    let socket = TcpStream::connect(address).expect("connect");
    let name = rustls::pki_types::ServerName::try_from("board").expect("a name");
    let session = rustls::ClientConnection::new(Arc::new(client), name).expect("a session");
    let mut caller = rustls::StreamOwned::new(session, socket);
    let _ = caller.write_all(b"ping\n").and_then(|()| caller.flush());
    let mut got = String::new();
    let _ = caller.read_to_string(&mut got);
    board_side.join().expect("the board's thread")
}

/// **The whole of `ADR-063` §3's reload on a real socket**: a newcomer is
/// refused, the file gains its key, one look puts it in force, the newcomer is
/// admitted without anything being rebuilt — and a later removal refuses it
/// again.
#[test]
fn a_key_written_to_the_file_is_admitted_after_one_look_and_refused_after_removal() {
    let board = mint("board");
    let operator = mint("operator");
    let newcomer = mint("newcomer");
    let op = operator.public_key().expect("a key");
    let new = newcomer.public_key().expect("a key");

    let f = Fixture::new("handshake", &[op]);
    let config = Arc::new(
        rivetos_tls::live_server_config(&board, Arc::clone(&f.verifier), rivetos_tls::provider())
            .expect("a live config"),
    );

    connect(&config, &newcomer, &board).expect_err("an unpinned key was admitted");

    f.write(&file_of(&[op, new]));
    assert_eq!(f.live.look(Some(&hex(&op))), Outcome::Loaded { added: 1, removed: 0, now: 2 });
    assert_eq!(connect(&config, &newcomer, &board), Ok(hex(&new)));
    assert_eq!(connect(&config, &operator, &board), Ok(hex(&op)));

    f.write(&file_of(&[op]));
    assert_eq!(f.live.look(Some(&hex(&op))), Outcome::Loaded { added: 0, removed: 1, now: 1 });
    connect(&config, &newcomer, &board).expect_err("a removed key was admitted");
    assert_eq!(connect(&config, &operator, &board), Ok(hex(&op)));
}
