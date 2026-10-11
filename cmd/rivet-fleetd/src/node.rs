//! **Asking a machine something** — the other half of `rivet-fleetd`.
//!
//! A browser talks to this program over HTTPS with a certificate; this program
//! talks to a machine over `ADR-035`'s control plane with a pinned Ed25519 key.
//! Two different trust models, joined here, and that join is the whole
//! component:
//!
//! ```text
//!   browser  --HTTPS + client certificate-->  rivet-fleetd
//!   rivet-fleetd  --TLS + pinned key-->  rivet-mcpd  -->  the machine's answer
//! ```
//!
//! # Why it asks rather than answers
//!
//! `rivet-fleetd` could link `rivet-core` and answer questions about the
//! machine itself. **It must not.** `ADR-011` gives a fact one home, and what a
//! machine's state IS belongs to that machine — a second implementation would
//! be a second source of truth about somebody else's status, which `§6` of the
//! design ruled out from the other direction.
//!
//! So every answer here is the node's own words, forwarded unchanged.
//!
//! # Even for its own machine
//!
//! There is no shortcut for self-management. `/run/rivet/query` is `0600
//! root:root` and serves only `status`; this program runs as its own
//! unprivileged uid. So a fleet server asks itself the same way it asks
//! anything else, over the control plane, holding a credential — which is what
//! makes "self" not a special case.

use std::io::Write as _;

/// Where this program's own credentials live, in `RIVET_APP_DATA`.
const OPERATOR_SEED: &str = "operator-device.seed.hex";
/// Its certificate, DER, as hex.
const OPERATOR_CERT: &str = "operator-device.crt.hex";

/// Bytes from a hex file.
///
/// **Hex because a control plane cannot carry bytes.** `apply edit` is the only
/// way to put a file on a machine and it is text — a raw seed loses its null
/// bytes on the way. ~~Recorded as a gap rather than hidden: this wants an
/// operation that installs binary material into an application's data
/// directory, and there is not one.~~
///
/// **The gap was closed 2026-09-01 and this file has not moved yet**, which is
/// two separate facts and both belong here (`AGENTS.md` §2 rule 3, §7).
/// `rivet apply install-secret <app> <name> --hex <hex>` writes bytes into an
/// application's data directory owned by that application, in one act; the
/// paragraph above named it and it now exists.
///
/// What has **not** changed is what is on disk. These two files are hex on
/// every machine that already has them, and rewriting this reader would strand
/// those machines. So the migration is a separate act with its own decision:
/// install the RAW seed and certificate under new names, teach this to prefer
/// them and fall back to the `.hex` pair, and remove the fallback once no board
/// carries one. Until somebody does that, this reader is correct and the
/// operation is what places the files it reads — the hex text, which is exactly
/// what `--hex` carries.
///
/// **`crate::secret::bytes_from_hex` is the same decoder the operation uses**
/// and this is a fourth copy of it. It is not shared because `rivet-core` is
/// not a dependency of this application: the SDK's whole point (`ADR-036`) is
/// that an application links what it needs and not the system's domain crate.
fn hex_file(name: &str) -> Result<Vec<u8>, String> {
    let dir = rivet_sdk::app::data().map_err(|e| e.to_string())?;
    let path = dir.join(name);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let text = text.trim();
    if text.len() % 2 != 0 {
        return Err(format!("{}: an odd number of hex digits", path.display()));
    }
    (0..text.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&text[i..i + 2], 16)
                .map_err(|_| format!("{}: not hex", path.display()))
        })
        .collect()
}

/// This program's operator identity.
///
/// # Errors
///
/// A sentence naming the file, because the fix is always to place one.
pub fn identity() -> Result<rivetos_tls::Identity, String> {
    let seed_bytes = hex_file(OPERATOR_SEED)?;
    let seed: [u8; 32] = seed_bytes
        .as_slice()
        .try_into()
        .map_err(|_| format!("{OPERATOR_SEED}: an Ed25519 seed is 32 bytes"))?;
    Ok(rivetos_tls::Identity {
        certificate: hex_file(OPERATOR_CERT)?,
        seed,
    })
}

/// # ~~`node_pin`~~ — removed 2026-09-01
///
/// It read a `node.pin` file placed beside this program's credentials, which
/// was right when this program knew one machine and wrong the moment it knew a
/// register. A device's pin is a field of its record; see [`ask`].

/// **Ask a machine one thing**, and return exactly what it said.
///
/// `request` is a JSON-RPC line. The answer is the machine's own bytes — this
/// function does not parse, translate, summarise or cache them, and that is
/// deliberate: `§6` of the design forbids a second representation of a node's
/// state, and the cheapest way to honour that is to be a pipe.
///
/// # Errors
///
/// Anything the connection said, with the address attached.
pub fn ask(address: &str, pin: &str, request: &str) -> Result<String, String> {
    let (mut connection, mut socket) = open(address, pin)?;
    let mut tls = rustls::Stream::new(&mut connection, &mut socket);

    tls.write_all(request.as_bytes())
        .and_then(|()| tls.write_all(b"\n"))
        .and_then(|()| tls.flush())
        .map_err(|e| format!("{address}: {e}"))?;

    read_one_line(&mut tls).map_err(|e| format!("{address}: {e}"))
}

/// **One HTTP request to a machine's control plane**, `POST route` with
/// `body`, and the status and body that came back.
///
/// The control plane's HTTP endpoints close a connection after one response
/// (`ADR-035`), so the answer is read to the end and framed by its
/// `Content-Length`.
///
/// # Errors
///
/// Anything the connection said, or a response that is not HTTP, with the
/// address attached.
pub fn exchange(address: &str, pin: &str, route: &str, body: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let (mut connection, mut socket) = open(address, pin)?;
    let mut tls = rustls::Stream::new(&mut connection, &mut socket);
    let head = format!(
        "POST {route} HTTP/1.1\r\nHost: rivet\r\nContent-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let sent = tls
        .write_all(head.as_bytes())
        .and_then(|()| tls.write_all(body))
        .and_then(|()| tls.flush());

    let mut answer = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    loop {
        match std::io::Read::read(&mut tls, &mut chunk) {
            Ok(0) => break,
            Ok(got) => answer.extend_from_slice(&chunk[..got]),
            // A close without `close_notify` after a whole response is still
            // that response; `http_response` checks it is whole.
            Err(_) if !answer.is_empty() => break,
            Err(e) => {
                return Err(match sent {
                    Err(send) => format!("{address}: {send}"),
                    Ok(()) => format!("{address}: {e}"),
                });
            }
        }
    }
    http_response(&answer).map_err(|why| format!("{address}: {why}"))
}

/// A response's status and body, the body cut at `Content-Length`.
///
/// # Errors
///
/// No status line, no end to the head, or a body shorter than it declared.
fn http_response(bytes: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let end = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("the answer has no end to its HTTP head")?;
    let head = String::from_utf8_lossy(&bytes[..end]);
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.strip_prefix("HTTP/1."))
        .and_then(|rest| rest.split(' ').nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| format!("the answer does not open with a status line: {:?}", head.lines().next().unwrap_or("")))?;
    let body = &bytes[end + 4..];
    // **A repeated `Content-Length` is refused whether or not the values agree**,
    // as every request parser here refuses one: two readers of the same bytes
    // that honour different fields disagree about where the body ends.
    let lengths: Vec<&str> = lines
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.trim())
        .collect();
    if lengths.len() > 1 {
        return Err(format!(
            "the answer carries {} Content-Length headers ({}), so where its body ends is not \
             one answer",
            lengths.len(),
            lengths.join(", ")
        ));
    }
    let length = lengths
        .first()
        .map(|value| value.parse::<usize>().map_err(|_| format!("Content-Length {value:?} is not a number")))
        .transpose()?;
    match length {
        Some(n) if body.len() < n => Err(format!("the answer declared {n} bytes of body and {} arrived", body.len())),
        Some(n) => Ok((status, body[..n].to_vec())),
        None => Ok((status, body.to_vec())),
    }
}

/// A pinned, mutually authenticated connection to a machine's control plane,
/// with this program's operator identity and a deadline on every read and
/// write.
///
/// # Errors
///
/// The identity, the pin, or the connection, with the address attached.
fn open(address: &str, pin: &str) -> Result<(rustls::ClientConnection, std::net::TcpStream), String> {
    let identity = identity()?;
    // **The pin comes from the register now, not from a file.** `ADR-034` §6
    // gives a device record the machine's own public key, so a fleet server
    // that held a separate `node.pin` would be the second place a device's
    // credential is written — `ADR-011`, and the disagreement would present as
    // a handshake failure against a machine the page can see.
    let pins = rivetos_tls::parse_pins(pin).map_err(|e| format!("{address}: {e}"))?;
    let config = rivetos_tls::client_config(&identity, pins, rivetos_tls::provider())
        .map_err(|e| format!("{address}: {e}"))?;

    // **The name is not what is checked; the pin is.** `ADR-025` §2 makes the
    // pinned key the whole of the trust model, so the server name here is a
    // formality the verifier ignores.
    let name = rustls::pki_types::ServerName::try_from("rivet")
        .map_err(|e| format!("{address}: {e}"))?;
    let connection = rustls::ClientConnection::new(std::sync::Arc::new(config), name)
        .map_err(|e| format!("{address}: {e}"))?;
    // **`connect_timeout`, not `connect`** — added 2026-09-03 (`PLAN.md` §123).
    // A plain `connect` to a machine that is powered off does not fail quickly:
    // it retries the SYN until the kernel gives up, which on Linux is over two
    // minutes with the defaults. The read and write deadlines below never
    // applied to it, because there was no socket yet.
    let socket = {
        let mut last = format!("{address}: no address resolved");
        // **A member may be reachable only by name.** A DHCP member's record
        // carries no address; `Device::dial_host` hands this an `<name>.local`,
        // and `rivet_mdns::resolve` answers it over the same mDNS the card
        // already runs (`cmd/rivet-mdnsd`), falling through to the OS resolver
        // for any other address. Before this a fleet server reported every
        // DHCP member `unreachable`, because the OS on a card cannot resolve a
        // `.local` name.
        let resolved = rivet_mdns::resolve(address).map_err(|e| format!("{address}: {e}"))?;
        let mut opened = None;
        for candidate in resolved {
            match std::net::TcpStream::connect_timeout(&candidate, PATIENCE) {
                Ok(s) => {
                    opened = Some(s);
                    break;
                }
                Err(e) => last = format!("{address}: {e}"),
            }
        }
        opened.ok_or(last)?
    };
    // **A node that never answers must not take this program with it.** The
    // accept loop upstream is one thread, so an unbounded read here is an
    // outage of the whole fleet server rather than of one request.
    socket
        .set_read_timeout(Some(PATIENCE))
        .and_then(|()| socket.set_write_timeout(Some(PATIENCE)))
        .map_err(|e| format!("{address}: {e}"))?;
    Ok((connection, socket))
}

/// How long a node gets to answer before this gives up on it.
const PATIENCE: std::time::Duration = std::time::Duration::from_secs(20);

/// Read exactly one JSON-RPC line.
///
/// **The frame is a newline, not end-of-file, and getting that wrong deadlocks
/// both ends.** `rivet-mcpd` runs `rivet mcp` as a child and pumps lines in
/// both directions until *the caller* hangs up (`cmd/rivet-mcpd/src/main.rs`,
/// the `got == 0` break). So a reader that waits for end-of-file waits for a
/// close that the writer is waiting on it to perform. Observed: a `POST /mcp`
/// that logged its arrival and then never returned.
///
/// One line per request is also the right frame for what this does — a browser
/// asks one question and gets one answer.
///
/// # It reads in chunks, and until 2026-09-05 it read one byte at a time
///
/// The single-byte loop was correct and it did not scale to what the portal
/// asks for. `rivet get log latest all` off a board with a full journal is
/// **393 644 bytes**, measured, and every byte of it was a separate `read`
/// through the `rustls` state machine. The whole-journal read is not a corner
/// case: it is what the trial-boot confirmation and any real search need, and
/// `node::PATIENCE` gives the whole answer twenty seconds.
///
/// **Bytes after the newline are discarded, and that is what this always did.**
/// The frame is one request, one connection, one line — nothing reads this
/// stream after the answer, and `ask` drops it.
fn read_one_line(tls: &mut impl std::io::Read) -> std::io::Result<String> {
    let mut line: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    loop {
        match tls.read(&mut chunk) {
            Ok(0) => break,
            Ok(got) => {
                if let Some(at) = chunk[..got].iter().position(|b| *b == b'\n') {
                    line.extend_from_slice(&chunk[..at]);
                    break;
                }
                line.extend_from_slice(&chunk[..got]);
            }
            // Bytes already in hand are the answer; a close without
            // `close_notify` is how the other end usually goes.
            Err(_) if !line.is_empty() => break,
            Err(e) => return Err(e),
        }
    }
    String::from_utf8(line).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::{http_response, read_one_line};

    /// A control-plane answer is its status and exactly `Content-Length` bytes
    /// of body, and one that stops short is not an answer.
    #[test]
    fn an_http_answer_is_framed_by_its_content_length() {
        let whole = b"HTTP/1.1 409 Conflict\r\nContent-Type: text/plain\r\nContent-Length: 5\r\nConnection: close\r\n\r\nstaleEXTRA";
        assert_eq!(http_response(whole).expect("an answer"), (409, b"stale".to_vec()));
        let short = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nshort";
        assert!(http_response(short).expect_err("short").contains("declared 10"));
        assert!(http_response(b"{\"jsonrpc\":\"2.0\"}\n").is_err());
        assert!(http_response(b"SSH-2.0\r\n\r\n").is_err());
    }

    /// **A repeated `Content-Length` is refused, agreeing or not**: two readers
    /// of the same bytes that honour different fields disagree about where the
    /// body ends.
    #[test]
    fn an_answer_with_a_repeated_content_length_is_refused() {
        let conflicting = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 5\r\n\r\nhello";
        assert!(http_response(conflicting).expect_err("conflicting").contains("Content-Length"));
        let agreeing = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\ncontent-length: 5\r\n\r\nhello";
        assert!(http_response(agreeing).expect_err("agreeing").contains("Content-Length"));
    }

    /// A reader that hands over `at_a_time` bytes per call, so an answer can be
    /// made to arrive in pieces the way TLS really delivers one.
    struct Dribble {
        bytes: Vec<u8>,
        at_a_time: usize,
        fail_after: Option<usize>,
        served: usize,
    }

    impl std::io::Read for Dribble {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.fail_after == Some(self.served) {
                return Err(std::io::Error::other("the peer went away"));
            }
            let n = self.bytes.len().min(buf.len()).min(self.at_a_time);
            buf[..n].copy_from_slice(&self.bytes[..n]);
            self.bytes.drain(..n);
            self.served += n;
            Ok(n)
        }
    }

    fn dribbled(text: &str, at_a_time: usize) -> std::io::Result<String> {
        read_one_line(&mut Dribble {
            bytes: text.as_bytes().to_vec(),
            at_a_time,
            fail_after: None,
            served: 0,
        })
    }

    /// **An answer split across TLS records is still read whole**, whatever
    /// size the pieces arrive in. The chunked read this replaced a
    /// byte-at-a-time loop with has a boundary the loop did not — a newline
    /// that lands in the middle of a chunk, and one that lands at its edge.
    #[test]
    fn an_answer_arriving_in_pieces_is_read_up_to_the_newline() {
        let answer = format!("{{\"records\":[\"{}\"]}}", "x".repeat(50_000));
        for at_a_time in [1usize, 7, 4096, 16 * 1024, 64 * 1024] {
            let got = dribbled(&format!("{answer}\n"), at_a_time)
                .unwrap_or_else(|e| panic!("{at_a_time} bytes at a time: {e}"));
            assert_eq!(got, answer, "{at_a_time} bytes at a time");
        }
    }

    /// **Nothing past the newline comes back.** One connection carries one
    /// answer, and a second line glued onto the first would be handed to a
    /// browser as part of the machine's reply.
    #[test]
    fn the_answer_stops_at_the_first_newline() {
        let got = dribbled("{\"a\":1}\n{\"b\":2}\n", 4096).expect("read");
        assert_eq!(got, "{\"a\":1}");
    }

    /// A peer that hangs up mid-answer gives back what arrived rather than an
    /// error — a close without `close_notify` is how the other end usually
    /// goes, and the bytes in hand are the answer.
    #[test]
    fn bytes_already_in_hand_survive_a_broken_connection() {
        let got = read_one_line(&mut Dribble {
            bytes: b"{\"partial\":true".to_vec(),
            at_a_time: 4,
            fail_after: Some(8),
            served: 0,
        })
        .expect("what arrived");
        assert_eq!(got, "{\"partia");
    }

    /// And a connection that fails before a single byte is an error, not an
    /// empty answer that a caller would try to parse.
    #[test]
    fn a_failure_before_any_byte_is_an_error() {
        let why = read_one_line(&mut Dribble {
            bytes: b"anything".to_vec(),
            at_a_time: 4,
            fail_after: Some(0),
            served: 0,
        })
        .expect_err("an error");
        assert!(why.to_string().contains("went away"), "{why}");
    }

    /// Hex decoding is the one piece of logic here worth a test, because a
    /// wrong seed fails as a TLS error a long way from its cause.
    #[test]
    fn hex_round_trips() {
        let raw: Vec<u8> = (0u8..=255).collect();
        let text: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        let back: Vec<u8> = (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
            .collect();
        assert_eq!(raw, back);
    }
}
