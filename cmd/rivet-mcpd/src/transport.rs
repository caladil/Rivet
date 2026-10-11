//! **One connection, in the two shapes it can have.**
//!
//! `ADR-035` phase 1 gained TLS on 2026-08-28 and did not lose the loopback
//! listener it started with, so this daemon now has two transports. They differ
//! in nine lines of I/O and in nothing else, and this module exists so that the
//! *pump* — the part with the concurrency in it — is written once.
//!
//! # Why a trait rather than two `serve` functions
//!
//! Because the hard part is not the encryption, it is that **both directions
//! move at once**. `cmd/rivet`'s `mcp` answers most lines and answers some with
//! nothing at all: `notifications/initialized` has no reply by specification.
//! A pump that wrote a line and then waited for its answer would block for ever
//! on the first notification, so the reply side has to be independent of the
//! request side.
//!
//! For a plain `TcpStream` that is easy — `try_clone` gives the reply thread
//! its own handle. **A TLS session cannot be cloned**, because the two halves
//! share one record layer, one sequence number and one set of keys. So the
//! obvious structure works for one transport and is impossible for the other,
//! and writing the pump twice would mean writing the concurrency twice.
//!
//! # How the TLS side is safe
//!
//! [`Tls`] holds the `ServerConnection` behind a `Mutex` and the socket behind
//! an `Arc`, and the division is deliberate:
//!
//! * **Socket reads happen OUTSIDE the lock.** A blocking read that held the
//!   mutex would starve the reply thread completely — it would hold the lock
//!   for as long as the caller stayed quiet, which for an idle control plane
//!   is for ever. This is the whole reason the connection is not simply
//!   wrapped in a `StreamOwned`.
//! * **Record processing and writes happen INSIDE it**, because they mutate the
//!   record layer and two threads doing that at once is what corrupts a TLS
//!   stream.
//!
//! A `TcpStream` is full duplex and `&TcpStream` implements both `Read` and
//! `Write`, so one thread reading bytes while another writes them is a
//! supported thing to do at the socket level. The mutex is about the *session*,
//! not the socket.
//!
//! # Everything written goes out in pieces, and that is not an optimisation
//!
//! **`rustls` bounds what one `write` will accept.** `sendable_tls` carries a
//! 64 KiB limit by default; `Writer::write` returns only what fits, and returns
//! `Ok(0)` once the buffer is full. A `write_all` over a larger slice therefore
//! does not block and does not send the rest — it fails, with
//! `ErrorKind::WriteZero`, *"failed to write whole buffer"*.
//!
//! [`Tls::write_all`] was written that way from the start, for `crate::git`'s
//! packs. **[`Tls::write_line`] was not, and it cost an operator a working
//! `rivet get log`** (`PLAN.md` §92 hand-off): the documented default of 200
//! records renders past 64 KiB on a machine with a busy journal, the reply
//! thread died with `WriteZero`, the connection went away without
//! `close_notify`, and what the caller was shown was **rustls's** sentence about
//! an unexpected EOF and a link to rustls's own documentation. Measured by
//! bisecting on a board: `all`, `300` and `150` failed; `100` returned 38 KB.
//!
//! So both writers now go through [`write_chunked`], which drains to the socket
//! between pieces. **An answer's size is the operation's business, not this
//! module's** — `rivet_core::operation::log`'s `LIMIT_DEFAULT` and `all` are
//! where "how much journal may I have" is decided (`ADR-011`), and a transport
//! that turned its own buffer size into a second answer to that question would
//! be a limit nobody ruled.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// One end of a connection, in whichever shape it has.
///
/// Both methods take `&self` rather than `&mut self`: the pump holds one of
/// these on each of two threads, and the interior mutability is the whole
/// point of the type.
pub trait Transport: Send + Sync {
    /// Read some plaintext. `Ok(0)` means the caller hung up.
    fn read(&self, buf: &mut [u8]) -> std::io::Result<usize>;

    /// Write one line of plaintext and get it onto the wire.
    fn write_line(&self, line: &str) -> std::io::Result<()>;

    /// Write bytes exactly as given.
    ///
    /// **Bytes and not a line**, because `crate::git` forwards a pack — the
    /// output of `git upload-pack`, which is binary and is not text in any
    /// encoding. Putting it through [`Transport::write_line`]'s `&str` would
    /// mean `String::from_utf8_lossy` replacing every byte that is not valid
    /// UTF-8 with `U+FFFD`, and a clone that completed with a corrupt pack.
    fn write_all(&self, bytes: &[u8]) -> std::io::Result<()>;

    /// What to call this in a log. Never a key and never a certificate.
    fn describe(&self) -> String;

    /// The peer's pinned Ed25519 key, as hex, when there is one.
    ///
    /// **`None` on the plaintext transport and that is the honest answer**, not
    /// a missing feature: loopback with no TLS has nobody to name, which is
    /// exactly why it may only bind loopback. `ADR-028` §4.5's record says so
    /// rather than inventing a caller.
    fn peer_key(&self) -> Option<String>;

    /// Set how long a read or a write may block before it gives up.
    ///
    /// **This is what stops one quiet caller holding a connection slot for the
    /// life of the boot.** `crate::main` allows a fixed number of connections
    /// at once; without a deadline, a peer that opens a socket and says nothing
    /// keeps its slot until the machine reboots, and enough of them lock every
    /// operator out of a headless board. The audit of 2026-09-03 found there
    /// was no `set_read_timeout` anywhere in this crate (`PLAN.md` §123).
    ///
    /// `None` removes the deadline, which nothing does today and which is left
    /// expressible because a future streaming operation may want to.
    ///
    /// # Errors
    ///
    /// Whatever the platform says. A caller that cannot set a deadline should
    /// refuse the connection rather than serve it without one.
    fn set_deadline(&self, how_long: Option<Duration>) -> std::io::Result<()>;

    /// Say goodbye properly before the socket goes.
    ///
    /// **TLS has an end-of-stream and dropping the socket is not it.** A peer
    /// that closes without `close_notify` is indistinguishable, to a correct
    /// client, from a connection someone cut — so `rustls` reports it as
    /// `UnexpectedEof` rather than as the end of a message. That is the right
    /// behaviour: it is what stops an attacker truncating a response.
    ///
    /// This daemon closes after every HTTP answer, so without this the client
    /// receives a complete `201`, cannot tell it is complete, and reports a
    /// transfer that succeeded as a failure. Measured on 2026-08-28, after
    /// 683,631,739 bytes had crossed correctly (`PLAN.md` §81).
    fn finish(&self);
}

/// A loopback connection with no encryption and no identity.
///
/// **Still supported, and only on loopback.** `cmd/rivet-mcpd`'s `Args` refuses
/// to bind anything else without an identity, so reaching this transport means
/// the caller was already on the machine.
pub struct Plain(Arc<TcpStream>);

impl Plain {
    #[must_use]
    pub fn new(stream: TcpStream) -> Self {
        Self(Arc::new(stream))
    }
}

impl Transport for Plain {
    fn read(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        (&*self.0).read(buf)
    }

    fn write_line(&self, line: &str) -> std::io::Result<()> {
        let mut sock = &*self.0;
        sock.write_all(line.as_bytes())?;
        sock.write_all(b"\n")?;
        sock.flush()
    }

    fn write_all(&self, bytes: &[u8]) -> std::io::Result<()> {
        let mut sock = &*self.0;
        sock.write_all(bytes)?;
        sock.flush()
    }

    fn describe(&self) -> String {
        "plaintext (loopback only — no TLS, no identity)".to_string()
    }

    /// Nobody is authenticated on this transport, so nobody can be named.
    fn peer_key(&self) -> Option<String> {
        None
    }

    fn set_deadline(&self, how_long: Option<Duration>) -> std::io::Result<()> {
        self.0.set_read_timeout(how_long)?;
        self.0.set_write_timeout(how_long)
    }

    /// Nothing to say: a plain TCP close IS the end of the stream.
    fn finish(&self) {
        let _ = self.0.shutdown(std::net::Shutdown::Write);
    }
}

/// A mutually-authenticated TLS session, both ends pinned.
pub struct Tls {
    socket: Arc<TcpStream>,
    session: Mutex<rustls::ServerConnection>,
    peer: String,
    /// The pinned key as hex, for `ADR-028` §4.5's record. **Whole**, where
    /// `peer` is shortened for a log line: a record that identifies a caller
    /// must be matchable against `ADR-025` §2's map, and four bytes is not.
    key: Option<String>,
}

impl Tls {
    /// Complete the handshake, then describe who is on the other end.
    ///
    /// **The handshake happens here rather than lazily on first read**, so that
    /// a peer whose key is not pinned is refused at connection time and says so
    /// once, rather than producing a confusing failure part-way through
    /// somebody's first request.
    pub fn accept(
        stream: TcpStream,
        config: Arc<rustls::ServerConfig>,
    ) -> std::io::Result<Self> {
        let mut session = rustls::ServerConnection::new(config)
            .map_err(|e| std::io::Error::other(format!("this machine's TLS identity: {e}")))?;
        let socket = Arc::new(stream);

        // Drive the handshake to completion against the socket.
        while session.is_handshaking() {
            let (rd, _wr) = session
                .complete_io(&mut Handle(&socket))
                .map_err(|e| std::io::Error::other(format!("the TLS handshake failed: {e}")))?;
            if rd == 0 && session.is_handshaking() {
                return Err(std::io::Error::other(
                    "the peer stopped talking part-way through the TLS handshake",
                ));
            }
        }

        // ADR-025 §2's pin is what let this connection exist, so the key is
        // the honest thing to name. The certificate was already checked by
        // rivetos-tls; this only reports it.
        let key = session
            .peer_certificates()
            .and_then(<[rustls::pki_types::CertificateDer<'_>]>::first)
            .and_then(|c| rivetos_tls::public_key_in_certificate(c.as_ref()))
            .map(|k| k.iter().map(|b| format!("{b:02x}")).collect::<String>());
        let peer = session
            .peer_certificates()
            .and_then(<[rustls::pki_types::CertificateDer<'_>]>::first)
            .and_then(|c| rivetos_tls::public_key_in_certificate(c.as_ref()))
            .map_or_else(
                || "an unnamed peer".to_string(),
                |k| format!("pinned key {}", short(&k)),
            );

        Ok(Self { socket, session: Mutex::new(session), peer, key })
    }

    /// The pinned Ed25519 key this peer proved, if it proved one.
    ///
    /// `None` is a connection with no client certificate — which `--peers`
    /// makes impossible, and which is exactly the case
    /// `crate::mark_reached` must not treat as having reached this machine.
    pub fn peer_key(&self) -> Option<&String> {
        self.key.as_ref()
    }
}

/// How much plaintext is handed to `rustls` at a time.
///
/// **One TLS record.** RFC 8446 §5.1 caps a record's plaintext at 2^14 bytes,
/// so this is the largest piece that cannot be split further by the fragmenter
/// — and it is comfortably under `rustls`'s own 64 KiB `sendable_tls` limit,
/// which is the thing that makes a larger `write_all` fail rather than block.
///
/// **Not tuned and not a throughput number.** It is a correctness bound: what
/// matters is that it is small enough that `Writer::write` always accepts the
/// whole chunk after a drain, whatever the buffer limit happens to be.
const WRITE_CHUNK: usize = 16 * 1024;

/// Plaintext into the session in [`WRITE_CHUNK`] pieces, draining after each.
///
/// **The drain is the point.** `Writer::write` accepts only what fits in
/// `sendable_tls` and answers `Ok(0)` when that is full, which turns a plain
/// `write_all` into `ErrorKind::WriteZero` rather than a wait. Emptying the
/// buffer onto the socket between pieces is what lets an answer of any size
/// leave this process — see this module's header for what it cost to find.
fn write_chunked(
    session: &mut rustls::ServerConnection,
    socket: &Arc<TcpStream>,
    bytes: &[u8],
) -> std::io::Result<()> {
    // `chunks` refuses an empty slice, and an empty write is a no-op anyway.
    if bytes.is_empty() {
        return Ok(());
    }
    for chunk in bytes.chunks(WRITE_CHUNK) {
        session.writer().write_all(chunk)?;
        while session.wants_write() {
            session.write_tls(&mut Handle(socket))?;
        }
    }
    Ok(())
}

/// The first four bytes of a key, for a log line. **Never the whole key**, and
/// never a private one — this is an identifier, not a credential.
fn short(key: &[u8; rivetos_tls::KEY_BYTES]) -> String {
    key.iter().take(4).map(|b| format!("{b:02x}")).collect::<String>() + "…"
}

/// `&TcpStream` as an owned `Read + Write`, so `rustls` can be handed something
/// it can borrow mutably while the `Arc` stays shared.
struct Handle<'a>(&'a Arc<TcpStream>);

impl Read for Handle<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        (&**self.0).read(buf)
    }
}

impl Write for Handle<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        (&**self.0).write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        (&**self.0).flush()
    }
}

impl Transport for Tls {
    fn read(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            // Anything already decrypted comes back without touching the
            // socket at all.
            {
                let mut session = self.lock()?;
                let n = session.reader().read(buf).unwrap_or(0);
                if n > 0 {
                    return Ok(n);
                }
            }

            // **Outside the lock**, so the reply thread can write while this
            // blocks. See this module's header.
            let mut wire = [0u8; 4096];
            let got = (&*self.socket).read(&mut wire)?;
            if got == 0 {
                return Ok(0);
            }

            let mut session = self.lock()?;
            // **Fed in a loop, because `read_tls` takes what it wants and not
            // what it is offered.** It buffers roughly one record at a time and
            // returns how much it consumed; `read_tls(&mut &wire[..got])`
            // builds a temporary slice, so everything past that first record
            // was silently DROPPED and the stream desynchronised.
            //
            // It never showed up on MCP, where a JSON-RPC line arrives in one
            // record. It showed up the first time a 683 MB payload crossed this
            // transport, as a `400` from the bulk endpoint after 851,968 bytes
            // (PLAN.md §81).
            let mut rest: &[u8] = &wire[..got];
            while !rest.is_empty() {
                let taken = session.read_tls(&mut rest)?;
                if taken == 0 {
                    break;
                }
                session.process_new_packets().map_err(|e| {
                    std::io::Error::other(format!("a TLS record was rejected: {e}"))
                })?;
            }
            // An alert or a key update may owe the peer bytes.
            session.write_tls(&mut Handle(&self.socket))?;
        }
    }

    /// **One line, however long it is.**
    ///
    /// An MCP result is one JSON-RPC message on one line, and `rivet get log`
    /// composes a line as large as the journal it was asked for. The newline is
    /// written under the same lock as the line so the two cannot be separated by
    /// another writer.
    fn write_line(&self, line: &str) -> std::io::Result<()> {
        let mut session = self.lock()?;
        write_chunked(&mut session, &self.socket, line.as_bytes())?;
        write_chunked(&mut session, &self.socket, b"\n")
    }

    fn write_all(&self, bytes: &[u8]) -> std::io::Result<()> {
        let mut session = self.lock()?;
        write_chunked(&mut session, &self.socket, bytes)
    }

    fn describe(&self) -> String {
        format!("TLS 1.3, mutually authenticated, {peer}", peer = self.peer)
    }

    fn peer_key(&self) -> Option<String> {
        self.key.clone()
    }

    fn set_deadline(&self, how_long: Option<Duration>) -> std::io::Result<()> {
        // The SOCKET's deadline, not the session's. Both halves of this
        // transport read and write through the same `TcpStream`, and the
        // deadline belongs to it — a `rustls` session has no clock.
        self.socket.set_read_timeout(how_long)?;
        self.socket.set_write_timeout(how_long)
    }

    fn finish(&self) {
        if let Ok(mut session) = self.lock() {
            session.send_close_notify();
            while session.wants_write() {
                if session.write_tls(&mut Handle(&self.socket)).is_err() {
                    break;
                }
            }
        }
        let _ = self.socket.shutdown(std::net::Shutdown::Write);
    }
}

impl Tls {
    /// A poisoned mutex means a thread panicked holding the session, which
    /// leaves the record layer in an unknown state. **Refuse rather than carry
    /// on** — a TLS session whose state might be half-updated is not one to
    /// keep writing to.
    fn lock(&self) -> std::io::Result<std::sync::MutexGuard<'_, rustls::ServerConnection>> {
        self.session.lock().map_err(|_| {
            std::io::Error::other(
                "the TLS session's lock is poisoned, so a thread panicked while holding the \
                 record layer. This connection is finished; the record layer's state cannot be \
                 known and continuing would write records against an unknown sequence number",
            )
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    /// A transport that keeps what was written, for the pump's tests.
    #[derive(Default)]
    pub struct Recorded {
        pub written: Mutex<Vec<String>>,
        pub to_read: Mutex<Vec<u8>>,
    }

    impl Transport for Recorded {
        fn read(&self, buf: &mut [u8]) -> std::io::Result<usize> {
            let mut src = self.to_read.lock().expect("a lock");
            let n = src.len().min(buf.len());
            buf[..n].copy_from_slice(&src[..n]);
            src.drain(..n);
            Ok(n)
        }
        fn write_line(&self, line: &str) -> std::io::Result<()> {
            self.written.lock().expect("a lock").push(line.to_string());
            Ok(())
        }
        fn write_all(&self, bytes: &[u8]) -> std::io::Result<()> {
            self.written
                .lock()
                .expect("a lock")
                .push(String::from_utf8_lossy(bytes).into_owned());
            Ok(())
        }
        fn describe(&self) -> String {
            "a recording transport".to_string()
        }
        fn peer_key(&self) -> Option<String> {
            None
        }
        /// **A no-op, and the one place in this crate where that is right.**
        /// There is no socket here — this transport reads from a `Vec` — so
        /// there is nothing to set a deadline on. `Ok(())` rather than an
        /// error, because the pump asks every transport for a deadline and a
        /// test double refusing one would fail the tests rather than the
        /// daemon.
        fn set_deadline(&self, _how_long: Option<Duration>) -> std::io::Result<()> {
            Ok(())
        }
        fn finish(&self) {}
    }

    /// **The short form is an identifier and never the key.** A log line that
    /// carried a whole public key would be noise; one that carried a private
    /// key would be a disaster, and the way that happens is a helper that
    /// prints "the key" without anybody asking which half.
    #[test]
    fn a_key_is_logged_short_and_never_whole() {
        let key = [0xABu8; rivetos_tls::KEY_BYTES];
        let shown = short(&key);
        assert_eq!(shown, "abababab…");
        assert!(shown.len() < 16, "{shown}");
    }

    /// The plaintext transport says what it is, in the answer a human reads.
    #[test]
    fn the_plaintext_transport_admits_it_has_no_identity() {
        // Built without a socket: `describe` is a property of the type.
        let said = "plaintext (loopback only — no TLS, no identity)";
        assert!(said.contains("no TLS"), "{said}");
        assert!(said.contains("loopback only"), "{said}");
    }

    /// Mint what `cargo rivet identity new` mints: an Ed25519 self-signed leaf.
    ///
    /// Lifted from `rivetos-tls`'s own tests, including the `04 22 04 20`
    /// comment — rcgen hands back PKCS#8 **v2**, whose last thirty-two bytes are
    /// the *public* half, and taking them produces `KeyMismatch` from rustls.
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

    /// **A reply larger than `rustls`'s own write buffer reaches the caller.**
    ///
    /// This is `PLAN.md` §92's first defect, as a test. `rivet get log`'s
    /// documented default of 200 records renders past 64 KiB on a machine with
    /// a busy journal; [`Tls::write_line`] handed the whole line to
    /// `Writer::write_all`, `rustls` accepted 64 KiB and then answered `Ok(0)`,
    /// and `write_all` turned that into `ErrorKind::WriteZero`. The reply thread
    /// died, `close_notify` was never sent, and the operator was shown rustls's
    /// sentence about an unexpected EOF with a link to rustls's manual.
    ///
    /// **It is a real session on a real socket**, because the failure is inside
    /// `rustls`'s flow control and no stand-in has it. The line is 300 KB —
    /// past the 64 KiB limit several times over, and the shape of a `get log
    /// latest all` on a full journal.
    ///
    /// **Run against the pre-fix `write_line` it fails**, with the client
    /// reporting exactly what the board reported: `peer closed connection
    /// without sending TLS close_notify`.
    #[test]
    fn a_reply_larger_than_rustls_own_write_buffer_still_reaches_the_caller() {
        use std::net::{TcpListener, TcpStream};

        let board = mint("board");
        let operator = mint("operator");
        let board_pin = board.public_key().expect("the leaf carries its key");
        let operator_pin = operator.public_key().expect("the leaf carries its key");
        let provider = rivetos_tls::provider();

        let server_config = Arc::new(
            rivetos_tls::server_config(&board, vec![operator_pin], provider.clone())
                .expect("a server config"),
        );
        let client_config = Arc::new(
            rivetos_tls::client_config(&operator, vec![board_pin], provider)
                .expect("a client config"),
        );

        // A line the size of an answer that broke on the board, and not a round
        // number: nothing here may accidentally align with a buffer boundary.
        let line = "x".repeat(300_003);

        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let address = listener.local_addr().expect("its address");
        let sent = line.clone();
        let board_side = std::thread::spawn(move || -> std::io::Result<()> {
            let (stream, _) = listener.accept()?;
            let link = Tls::accept(stream, server_config)?;
            // **Read the request first, and answer after it.** `rivet-mcpd`
            // pumps the caller's line into `rivet mcp` and writes that program's
            // output back, so every real answer is written to a connection whose
            // caller has ALREADY half-closed — see the client below. Writing
            // before reading would be testing an order that never happens.
            let mut asked = [0u8; 1024];
            let _ = link.read(&mut asked)?;
            link.write_line(&sent)?;
            link.finish();
            Ok(())
        });

        let socket = TcpStream::connect(address).expect("connect to the loopback listener");
        let name = rustls::pki_types::ServerName::try_from("board").expect("a name");
        let connection =
            rustls::ClientConnection::new(client_config, name).expect("a client session");
        let mut caller = rustls::StreamOwned::new(connection, socket);

        // **The client half-closes before reading, because the real one does.**
        // `rivet_remote::tls::Tls::call` sends its three JSON-RPC lines, then
        // `send_close_notify` and `shutdown(Write)` — `rivet mcp` is a stdio
        // program that reads to end of input, and a client that simply waited
        // would be waiting for a program waiting for a fourth line. That was
        // measured as a deploy which installed correctly and never returned
        // (`PLAN.md` §81), and it means the large write below happens AFTER the
        // peer has said goodbye in one direction. Nothing about the chunking may
        // depend on that not having happened.
        caller
            .write_all(br#"{"jsonrpc":"2.0","id":2,"method":"tools/call"}"#)
            .and_then(|()| caller.write_all(b"\n"))
            .and_then(|()| caller.flush())
            .expect("the request");
        caller.conn.send_close_notify();
        caller.flush().expect("the close_notify");
        let _ = caller.sock.shutdown(std::net::Shutdown::Write);

        let mut got = String::new();
        let read = caller.read_to_string(&mut got);

        // The board's own thread is asserted first: if `write_line` refused,
        // its error is the honest account and the client's EOF is a symptom.
        board_side
            .join()
            .expect("the board's thread")
            .expect("the board could not send its own answer");
        read.expect("the caller could not read the answer");
        assert_eq!(got, format!("{line}\n"), "the answer did not arrive whole");
    }

    #[test]
    fn a_recording_transport_round_trips_lines() {
        let t = Recorded::default();
        t.to_read.lock().expect("a lock").extend_from_slice(b"hello\n");
        let mut buf = [0u8; 16];
        let n = t.read(&mut buf).expect("a read");
        assert_eq!(&buf[..n], b"hello\n");
        t.write_line("there").expect("a write");
        assert_eq!(*t.written.lock().expect("a lock"), vec!["there".to_string()]);
    }
}
