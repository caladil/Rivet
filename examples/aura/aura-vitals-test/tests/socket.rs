//! **The producer, as a process, against a real socket.**
//!
//! Everything in `src/` is tested as functions: compose a `Vitals`, encode it,
//! compare the bytes. That leaves one thing unchecked, and it is the half that
//! only exists at runtime — **`main`'s loop**: reading `RIVET_CONNECT`, finding
//! the granted endpoint, connecting, and writing a card somebody can read.
//!
//! This spawns the **actual binary** this crate ships, hands it a socket the
//! way a supervisor would, and reads what arrives.
//!
//! # Why this is not the same test twice
//!
//! `card.rs`'s tests call `encode`. This one never touches it: it starts a
//! process, and the bytes it asserts on came off a file descriptor. Between the
//! two lies everything `main` does — the endpoint lookup that could match the
//! wrong owner, the `UnixStream::connect` that could go to the wrong path, and
//! the write that could be buffered and never flushed.
//!
//! **The flush is the one that would actually happen.** `write_all` followed by
//! no `flush` on a `UnixStream` happens to work today because `UnixStream` is
//! unbuffered; wrap it in a `BufWriter` one afternoon and the card stops
//! arriving with every unit test still green.
//!
//! # Unix only
//!
//! `RIVET_CONNECT` names a Unix socket and the producer is a Linux program.
//! This crate does not build for Windows at all (`std::os::unix::net`), so
//! there is no `cfg` here — a target that cannot run this cannot compile the
//! program it is testing either.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::io::Read as _;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long to wait for the producer to connect and send.
///
/// Generous: it is a cold process start on whatever this is running on, and the
/// producer's own first action is a `/proc` read. A tight bound here would make
/// this test flaky on a loaded machine, which is worse than a slow one.
const PATIENCE: Duration = Duration::from_secs(20);

/// A socket path in the target directory rather than `/tmp`.
///
/// `/tmp` is shared and this test may run beside another checkout; the target
/// directory is this build's own. The name carries the process id for the same
/// reason.
fn socket_path() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target");
    std::fs::create_dir_all(&dir).expect("target/ should exist");
    dir.join(format!("vitals-test-{}.sock", std::process::id()))
}

/// Kill the child on the way out however this test ends.
///
/// **Not a nicety.** The producer loops for ever by design, so a test that
/// asserted and returned would leave it running until the machine was rebooted
/// — and the next run would find its socket taken.
struct Reaped(Child);

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// **A card arrives on the socket the supervisor granted.**
#[test]
fn the_producer_connects_to_its_granted_endpoint_and_sends_a_card() {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("this test should be able to bind");
    listener
        .set_nonblocking(true)
        .expect("so the accept loop can time out rather than hang for ever");

    // Exactly what a supervisor sets: `owner:name=path`, space separated
    // (`SPEC-03` §3.4). A second, unrelated grant is included so the endpoint
    // lookup has to actually match rather than take the first thing it sees.
    let connect = format!(
        "some-other-app:api=/run/rivet/connect/some-other-app/api aura-renderer:tertiary={}",
        path.display()
    );
    let child = Command::new(env!("CARGO_BIN_EXE_aura-vitals-test"))
        .env("RIVET_CONNECT", &connect)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the producer binary should be built before this test runs");
    let _reaped = Reaped(child);

    let deadline = Instant::now() + PATIENCE;
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(why) if why.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(
                    Instant::now() < deadline,
                    "the producer never connected to {}, though RIVET_CONNECT named it. \
                     Either the endpoint lookup did not match `aura-renderer:tertiary` or the \
                     connect went somewhere else",
                    path.display()
                );
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(why) => panic!("accept failed: {why}"),
        }
    };

    stream
        .set_read_timeout(Some(PATIENCE))
        .expect("a read timeout, so a producer that connects and says nothing fails loudly");
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let text = loop {
        let read = stream.read(&mut chunk).expect("the producer should send a card");
        assert!(read > 0, "the producer connected and then closed without sending");
        buffer.extend_from_slice(&chunk[..read]);
        let text = String::from_utf8(buffer.clone()).expect("a card is UTF-8 — SPEC-04 §6.3");
        // The message ends at `end`, and the producer keeps the connection open
        // for its next tick — so this reads until the framing is complete
        // rather than until EOF, which would never come.
        if text.contains("\nend\n") {
            break text;
        }
        assert!(buffer.len() < 8192, "SPEC-04 §6.6 bounds a message at 8 KiB");
    };

    // **Decoded by the renderer's own decoder**, not string-matched: what this
    // asserts is that the bytes off the wire are a card `aura-renderer` would
    // accept, which is the only property that matters here.
    let message = aura_card::Message::decode(&text)
        .unwrap_or_else(|why| panic!("the renderer would refuse what arrived: {why}\n{text}"));
    let aura_card::Message::Card(card) = message else {
        panic!("the producer sent a withdrawal on its first tick: {text}");
    };
    assert_eq!(
        card.slot,
        aura_card::Slot::Tertiary,
        "the card must name the slot whose socket it arrived on, or the renderer refuses it"
    );
    assert!(!card.blocks.is_empty(), "SPEC-04 forbids a card with no blocks");
    assert_eq!(card.ttl_seconds, 60);

    let _ = std::fs::remove_file(&path);
}

/// **A producer that was granted nothing exits instead of guessing a path.**
///
/// The failure this rules out is the one Aura's C++ renderer actually had: a
/// hard-coded `/run/aura/kiosk.sock`, which is not where a Rivet puts it. A
/// producer that invented a path would sit for ever connecting to nothing, and
/// the only symptom would be an empty slot.
#[test]
fn a_producer_granted_nothing_exits_rather_than_guessing() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_aura-vitals-test"))
        .env("RIVET_CONNECT", "")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the producer binary should be built");

    let deadline = Instant::now() + PATIENCE;
    loop {
        match child.try_wait().expect("try_wait") {
            Some(status) => {
                // It exits, and it exits SUCCESSFULLY: a grant that was not made
                // is an ordinary state of a machine, not a crash. The journal
                // carries which endpoint was wanted.
                assert!(
                    status.success(),
                    "an ungranted producer should exit cleanly, not fail: {status}"
                );
                return;
            }
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "the producer was granted nothing and kept running. It is either \
                     connecting to a path it invented or waiting for a socket that will \
                     never exist"
                );
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}
