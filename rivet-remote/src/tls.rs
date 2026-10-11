//! **Reaching a machine over `ADR-035`'s control plane.**
//!
//! `ADR-035`'s phase 2 row is *"Move `rivet-remote` to it, plus §3.2's bulk
//! endpoint"*, and §8 says why that phase is the one that matters: *"Until it is
//! done and has carried a real deploy, nothing may be removed."*
//!
//! [`crate::deploy::Remote`] is the seam and this is its implementation. It
//! speaks two protocols to one listener, which is `ADR-035` §3.2's shape:
//!
//! * **the payload** goes over the HTTP bulk endpoint — `cmd/rivet-mcpd`'s
//!   `bulk` module — because JSON-RPC would mean base64 in a string, buffered,
//!   for 683 MB on a 926 MB board;
//! * **everything else** is an MCP `tools/call`, which is the catalogue
//!   projection `ADR-006` §3.1 already requires.
//!
//! # What this can do today, and what it still refuses
//!
//! **A system update deploys.** `push`, `blake3` and `install` are the whole of
//! that sequence and all three are here.
//!
//! **`--enable` no longer refuses here, since 2026-08-29.** It needs
//! [`Remote::read_document`](crate::deploy::Remote::read_document), and until phase 4 grew the read surface there was
//! nothing to answer it with: reading a file was a *nushell* command rather than
//! a Rivet operation, and **MCP exposes the catalogue** — which had no verb for
//! reading a file. That was `ADR-006`'s principle working correctly rather than
//! a gap here, and `ADR-035` §9 said what to do about it:
//!
//! > **the read surface must grow before `nu` goes**, or the board becomes
//! > undebuggable.
//!
//! It grew. `rivet get document` is phase 4's first subject and is on the board
//! (`PLAN.md` §91), so the three document methods are now `rivet_get` and
//! `rivet_apply` calls like everything else here — see `read_document_via`,
//! `edit_document_via` and `put_document_via`.
//!
//! **What that has and has not been proved against, as of 2026-08-29.** The
//! calls were exercised against a fake `rivet mcp` in this crate's tests, and
//! the arguments they compose were piped into a real `rivet mcp` — a Linux build
//! of this workspace's own binary, running as root over a real
//! `/state/config/machine.toml` — which performed the edit and read the document
//! back byte for byte (`tls_tests.rs`'s `READ_BACK`). **No part of it has run
//! against the board**, so what is proven is that this end speaks what that
//! program reads, not that a deploy over the wire works.
//!
//! **[`Remote::ask`](crate::deploy::Remote::ask) still refuses, and that refusal is not a gap either.** It
//! takes a word list because the transports it was written for handed the
//! machine a shell; a tool call names an operation and its declared inputs. The
//! two shapes are what `ADR-006` §3.1's co-equality produces, and this transport
//! says so rather than guessing at a translation. A channel that quietly picked
//! one would be the worst of both.
//!
//! # Two layers of JSON, in both directions
//!
//! Everything below the transport is text inside JSON inside JSON, and getting
//! that wrong writes a *plausible* document rather than failing.
//!
//! * **Outbound**, `json_escape` is the whole of it. This channel is a JSON
//!   array of strings, and a `[services.x]` table is multi-line.
//! * **Inbound**, `extract_text` reads the *outer* layer and hands back the
//!   operation's own JSON as a string; `json_string_field` and its siblings
//!   read the inner one. Both go through `rivet_core::json`, the structured
//!   reader `ADR-035` §8A put in `rivet-core` for exactly this reason — one
//!   parser that knows the grammar rather than a search for the bytes one
//!   encoder happens to produce. It is strict on purpose — a document that
//!   does not parse, or a length that does not match what the machine
//!   reported, is a refusal, because a silently mis-decoded document that is
//!   then written back is the worst outcome available here and a refusal
//!   costs nothing.
//!
//! # `blake3` is folded into the transfer, and that is stronger
//!
//! A transport that copies and then asks the board to hash the file leaves a
//! gap between the two questions. The bulk endpoint **hashes as it
//! receives**, refuses a mismatch, and renames only on success — so by the time
//! a `201` comes back the board has already compared its own digest to the one
//! sent, and reports it.
//!
//! `Tls::blake3` therefore answers from what the `PUT` returned. It is the
//! board's own BLAKE3 of the bytes it wrote, computed **before** the file
//! existed under its final name, and it **refuses for any path it did not just
//! push** rather than inventing an answer.

// **Named rather than anonymous**, because `transfer` is generic over what it
// writes to — a TLS stream in this module, a stand-in that fails mid-body in the
// tests — and a bound needs the trait to have a name.
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

use rivet_core::json::{self, Json};

use crate::deploy::{ConfigChange, Remote};

/// How this reaches a machine.
pub struct Tls {
    /// `host:port`.
    pub host: String,
    /// This operator's own identity — `cargo rivet identity new`.
    pub identity: rivetos_tls::Identity,
    /// The machine's public key, as the fleet register records it.
    pub pins: Vec<rivetos_tls::Pin>,
    /// What the last [`Remote::push`] was told the board computed.
    verified: Option<(String, [u8; 32])>,
}

/// What one control-plane handshake found — [`Tls::handshake`].
///
/// # Four answers, because three of them are different failures
///
/// A probe that collapsed these into *up* and *down* would report **down** for
/// a machine that is switched on, answering, and presenting a key this end does
/// not pin — which is the one failure an operator most needs told apart, since
/// it is either a device whose identity was replaced or something else wearing
/// its address. It would equally report *down* for a run holding no identity at
/// all, which is a fact about this end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handshake {
    /// The peer answered and presented a certificate this end pins.
    ///
    /// See [`Tls::handshake`] for what this deliberately does not claim.
    Trusted,
    /// Something accepted a connection on that port and the handshake did not
    /// complete. Carries the refusal verbatim.
    Untrusted(String),
    /// Nothing accepted a connection: the name did not resolve, the port
    /// refused, or nothing answered in time.
    Unreachable(String),
    /// **This end could not ask.** Its own identity or pins are unusable, so
    /// nothing was sent and nothing is known about the machine.
    Unusable(String),
}

impl Handshake {
    /// Whatever this end or the far end said, for a screen that shows detail.
    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Trusted => None,
            Self::Untrusted(why) | Self::Unreachable(why) | Self::Unusable(why) => Some(why),
        }
    }
}

impl Tls {
    #[must_use]
    pub fn new(
        host: String,
        identity: rivetos_tls::Identity,
        pins: Vec<rivetos_tls::Pin>,
    ) -> Self {
        Self { host, identity, pins, verified: None }
    }

    /// One mutually-authenticated connection.
    ///
    /// **A connection per REQUEST, and a request may carry several tool calls.**
    /// The listener closes an HTTP connection after its response and gives an
    /// MCP connection its own `rivet mcp` process, so a connection cannot be
    /// kept across requests — that would mean multiplexing two protocols on a
    /// stream that was dispatched to one of them at its first byte.
    ///
    /// ~~*A connection per operation*~~ — struck 2026-08-29 rather than edited,
    /// because it was true of every caller until [`Tls::call_each`] existed and
    /// the reason it stopped being true is the interesting part: the process the
    /// far end spawns is per connection, so batching the calls that are due
    /// together is the whole of `ADR-034` §11.5.2's optimisation. The listener
    /// still closes after the response; **keeping a connection across cycles is
    /// a listener change and is out of scope.**
    fn connect(&self) -> Result<rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream>, String> {
        let config = rivetos_tls::client_config(
            &self.identity,
            self.pins.clone(),
            rivetos_tls::provider(),
        )
        .map_err(|e| format!("this machine's own TLS identity or pins: {e}"))?;

        // **`.local` is resolved over mDNS, everything else by the OS.** A DHCP
        // member's only handle is its `<name>.local`, which the operating
        // system cannot answer; `rivet_mdns::resolve` does, and hands anything
        // else straight to `to_socket_addrs`. Its refusal is already a whole
        // sentence, so it is returned as-is.
        let addresses = rivet_mdns::resolve(&self.host)?;
        let mut last = format!("{}: resolved to no address at all.", self.host);
        let mut opened = None;
        for candidate in &addresses {
            match std::net::TcpStream::connect(candidate) {
                Ok(stream) => {
                    opened = Some(stream);
                    break;
                }
                Err(e) => last = format!("cannot reach {candidate}: {e}"),
            }
        }
        let socket = opened.ok_or(last)?;
        // The name is not a trust condition — the pin is (ADR-025 §2) — so any
        // syntactically valid name serves, and `rivetos-tls` ignores it.
        let name = rustls::pki_types::ServerName::try_from("rivet")
            .map_err(|e| format!("the fixed server name is not valid, which cannot happen: {e}"))?;
        let connection = rustls::ClientConnection::new(Arc::new(config), name)
            .map_err(|e| format!("cannot start a TLS session with {}: {e}", self.host))?;
        Ok(rustls::StreamOwned::new(connection, socket))
    }

    /// **Complete a handshake and ask nothing.** `ADR-034` §11.5.1's reach
    /// probe, and the narrowest question this transport can put.
    ///
    /// # Why this exists rather than a `get` of some cheap subject
    ///
    /// A reachability probe wants to know *is the machine there and is it the
    /// one the register records* — nothing more. Asking a subject would answer
    /// that and also spawn a `rivet mcp` process on a 926 MiB board for every
    /// device on a fleet screen, which is a poll's cost paid for a dot.
    ///
    /// **Nothing is spawned by this.** `cmd/rivet-mcpd` reads the opening bytes
    /// of a connection before it decides which endpoint it is and spawns
    /// anything, so a handshake carrying no application data costs the board a
    /// TLS handshake and no process.
    ///
    /// # What a [`Handshake::Trusted`] does and does not claim
    ///
    /// It says **the peer presented a certificate this end pins** — the whole
    /// of `ADR-025` §2's trust in that direction, checked by
    /// `rivetos_tls::PinnedPeer` during the handshake.
    ///
    /// It does **not** say the machine accepts *this operator*. TLS 1.3 lets a
    /// client finish its side before the server has judged the client
    /// certificate, so a board that will refuse this identity still completes
    /// the handshake here and refuses at the first request. Saying more than
    /// this would need a round trip that costs the process this method exists
    /// to avoid.
    #[must_use]
    pub fn handshake(&self, timeout: std::time::Duration) -> Handshake {
        let config = match rivetos_tls::client_config(
            &self.identity,
            self.pins.clone(),
            rivetos_tls::provider(),
        ) {
            Ok(config) => Arc::new(config),
            Err(e) => {
                return Handshake::Unusable(format!(
                    "this end's own TLS identity or pins: {e}"
                ));
            }
        };
        // **`.local` over mDNS, everything else the OS** — the same resolution
        // `connect` uses, so the reach probe and a real connection agree about
        // what a name means. `resolve`'s refusal is a complete sentence.
        let addresses = match rivet_mdns::resolve(&self.host) {
            Ok(found) => found,
            Err(e) => return Handshake::Unreachable(e),
        };
        let Some((first, rest)) = addresses.split_first() else {
            return Handshake::Unreachable(format!(
                "{} resolved to no address at all.",
                self.host
            ));
        };

        // **The first address that accepts a connection**, as the old probe
        // did: a name with several is a machine reachable several ways, and one
        // of them answering is what the dot means. The handshake is then asked
        // of that one and its answer stands — a second address is tried only
        // when nothing accepted at the first.
        let mut last = String::new();
        for address in std::iter::once(first).chain(rest) {
            let socket = match std::net::TcpStream::connect_timeout(address, timeout) {
                Ok(socket) => socket,
                Err(e) => {
                    last = format!("cannot reach {address}: {e}");
                    continue;
                }
            };
            // **The timeout covers the handshake too.** A port that accepts a
            // connection and then says nothing is the case this guards: without
            // it the probe would hang for as long as the operating system lets
            // a read block, on the interface's own thread.
            let _ = socket.set_read_timeout(Some(timeout));
            let _ = socket.set_write_timeout(Some(timeout));
            // The name is not a trust condition — the pin is (`ADR-025` §2) —
            // so any syntactically valid name serves, exactly as in `connect`.
            let name = match rustls::pki_types::ServerName::try_from("rivet") {
                Ok(name) => name,
                Err(e) => {
                    return Handshake::Unusable(format!(
                        "the fixed server name is not valid, which cannot happen: {e}"
                    ));
                }
            };
            let mut connection = match rustls::ClientConnection::new(Arc::clone(&config), name) {
                Ok(connection) => connection,
                Err(e) => {
                    return Handshake::Unusable(format!(
                        "cannot start a TLS session with {}: {e}",
                        self.host
                    ));
                }
            };
            let mut socket = socket;
            return match connection.complete_io(&mut socket) {
                Ok(_) => {
                    // Closed politely rather than dropped, so the far end sees
                    // a finished conversation instead of a reset.
                    connection.send_close_notify();
                    let _ = connection.complete_io(&mut socket);
                    let _ = socket.shutdown(std::net::Shutdown::Both);
                    Handshake::Trusted
                }
                // **Answered, and not trusted.** This end cannot always say
                // WHICH — a pin that does not match, a program that is not TLS,
                // a version that cannot be agreed — so it says the honest thing
                // and carries the refusal verbatim for whoever reads it.
                Err(e) => Handshake::Untrusted(format!(
                    "{address} answered and this end would not trust it: {e}"
                )),
            };
        }
        Handshake::Unreachable(last)
    }

    /// Call one MCP tool and hand back the text it answered with.
    fn call(&self, tool: &str, arguments: &str) -> Result<String, String> {
        let mut answers = self.call_each(&[(tool.to_owned(), arguments.to_owned())]);
        answers.pop().unwrap_or_else(|| {
            Err(format!(
                "{} was asked one thing and this end produced no answer slot for it, which \
                 cannot happen.",
                self.host
            ))
        })
    }

    /// **Call several MCP tools over ONE connection**, in order.
    ///
    /// # Why one connection carries all of them
    ///
    /// `rivet-mcpd` spawns a `rivet mcp` process **per CONNECTION, not per
    /// request** — read off the listener on 2026-08-29 rather than inherited as
    /// a claim, and recorded in `ADR-034` §11.5.2. So three questions asked on
    /// three connections cost a 926 MiB board three processes and three TLS
    /// handshakes; the same three on one connection cost one of each. The
    /// answers still come back one per line and are still matched by id, so
    /// nothing about the shape of an answer changes.
    ///
    /// **A transport failure is every caller's failure.** When the connection
    /// itself could not be made or read, each element of the returned vector is
    /// that same refusal: this end does not know which of the calls would have
    /// succeeded, and saying so per call is the honest report.
    ///
    /// The returned vector is always exactly as long as `calls`.
    fn call_each(&self, calls: &[(String, String)]) -> Vec<Result<String, String>> {
        if calls.is_empty() {
            return Vec::new();
        }
        let each = |why: &str| -> Vec<Result<String, String>> {
            calls.iter().map(|_| Err(why.to_owned())).collect()
        };
        let mut stream = match self.connect() {
            Ok(stream) => stream,
            Err(why) => return each(&why),
        };
        let script = batch_script(calls);
        if let Err(e) = stream.write_all(script.as_bytes()).and_then(|()| stream.flush()) {
            return each(&format!("cannot send to {}: {e}", self.host));
        }

        // **Say that the request is over, or both ends wait for the other.**
        // `rivet mcp` is a stdio program: it reads until end of input and the
        // daemon closes the connection when it exits. A client that sent three
        // lines and then simply waited would be waiting for a program that is
        // waiting for a fourth line — measured as a deploy that installed
        // correctly on the board and never returned (`PLAN.md` §81).
        stream.conn.send_close_notify();
        if let Err(e) = stream.flush() {
            return each(&format!("cannot end the request to {}: {e}", self.host));
        }
        let _ = stream.sock.shutdown(std::net::Shutdown::Write);

        let mut all = String::new();
        let named = if calls.len() == 1 {
            format!("the `{}` call", calls[0].0)
        } else {
            format!("{} calls on one connection", calls.len())
        };
        if let Err(e) = stream.read_to_string(&mut all) {
            return each(&read_failure(&self.host, &named, &e, all.len()));
        }
        answers_from(&all, calls.len(), &self.host)
    }

    /// **Ask this machine one `get` subject and hand back the JSON it printed.**
    ///
    /// # Why this shape, and not a public `call`
    ///
    /// [`Remote::ask`] refuses a word list on purpose — *"MCP is the CATALOGUE,
    /// not a command line"* — and that refusal is worth keeping intact. A public
    /// `call(tool, arguments)` would let a caller compose any tool and any
    /// argument object, which is the word list back again with more steps and
    /// no catalogue check. So the entry point a monitor needs is narrowed to
    /// the only thing a monitor does: **name one `get` subject**. The tool name
    /// and the argument object are composed by [`crate::command::translate`]
    /// from `rivet_core::operation::CATALOGUE`, so there is no second spelling
    /// of `rivet_get {"subject":…}` in this workspace and a subject that the
    /// catalogue cannot bind is refused here rather than sent.
    ///
    /// The answer is the operation's own JSON, exactly as the machine composed
    /// it. Nothing here reads or reformats it.
    ///
    /// # Errors
    ///
    /// This end's refusal when the subject cannot be translated, the transport's
    /// when the connection failed, or the machine's own words when it refused.
    pub fn get(&self, subject: &str) -> Result<String, String> {
        let mut answers = self.get_each(&[subject]);
        answers.pop().unwrap_or_else(|| {
            Err("one subject was asked for and no answer slot was made, which cannot happen."
                .to_owned())
        })
    }

    /// **Ask for several `get` subjects over ONE connection**, in order.
    ///
    /// [`Tls::get`] is this with one subject, and this is the form that exists:
    /// `ADR-034` §11.5.2 rules **one connection per poll cycle, carrying every
    /// subject due on that tick** — because the far end spawns a process per
    /// connection. A caller that loops over [`Tls::get`] pays that process once
    /// per subject and gets nothing for it.
    ///
    /// The returned vector is always exactly as long as `subjects`, in the same
    /// order, and each element is that subject's own answer or its own refusal.
    /// **A subject this end cannot translate does not stop the others** — it
    /// fails in place, is not sent, and the rest of the connection proceeds.
    #[must_use]
    pub fn get_each(&self, subjects: &[&str]) -> Vec<Result<String, String>> {
        // Composed first, so a subject the catalogue cannot bind never reaches a
        // socket — the same order `cargo rivet run` uses, and for the same
        // reason: this end's own refusal costs nothing and a connection does.
        let composed: Vec<Result<(String, String), String>> = subjects
            .iter()
            .map(|subject| {
                crate::command::translate(&["get", subject])
                    .map(|call| (call.tool, call.arguments))
            })
            .collect();

        let sendable: Vec<(String, String)> = composed
            .iter()
            .filter_map(|c| c.as_ref().ok().cloned())
            .collect();
        let mut answers = self.call_each(&sendable).into_iter();
        composed
            .into_iter()
            .map(|composed| match composed {
                Err(why) => Err(why),
                Ok(_) => answers.next().unwrap_or_else(|| {
                    Err(format!(
                        "{} was asked for this subject and this end kept no answer slot for \
                         it, which cannot happen.",
                        self.host
                    ))
                }),
            })
            .collect()
    }

    /// **One HTTP request over the control plane, and the whole answer.**
    ///
    /// `head` is a complete request head — request line, headers, the blank
    /// line — and `body` follows it; the bytes that come back are the
    /// machine's response, status line and all, read until the machine closes
    /// the connection. Nothing here reads or rewrites either: the caller owns
    /// the HTTP, and this owns only the pinned, mutually authenticated
    /// connection every other call on this type makes.
    ///
    /// # Why this exists, and why it is not `connect` made public
    ///
    /// `cargo rivet fleet push` hands `ADR-035` §4's git to a stock `git`
    /// through a loopback relay (`cmd/cargo-rivet/src/fleet_push.rs` says why
    /// `git` cannot be given the pin itself). The relay needs exactly this —
    /// *send these bytes to the machine, give me what it said* — and nothing
    /// about the connection: publishing the stream would publish the
    /// close-notify discipline [`CallsTools`]'s documentation keeps private.
    ///
    /// **A connection per request**, `Tls::connect`'s rule: the listener
    /// closes an HTTP connection after its response.
    ///
    /// # A send that fails is not the machine's answer
    ///
    /// `transfer`'s rule: the machine may already have answered — a refusal
    /// sent before the body was read — so a failed write is followed by a read,
    /// and a status line that arrives is what is returned.
    ///
    /// # Errors
    ///
    /// The connection could not be made (this end's identity, the address, or
    /// a machine that will not complete a handshake with this identity), or
    /// nothing readable came back.
    pub fn exchange(&self, head: &str, body: &[u8]) -> Result<Vec<u8>, String> {
        let mut stream = self.connect()?;
        let sent = stream
            .write_all(head.as_bytes())
            .and_then(|()| stream.write_all(body))
            .and_then(|()| stream.flush());
        let mut answer = Vec::new();
        let read = stream.read_to_end(&mut answer);
        let has_status = answer.starts_with(b"HTTP/1.");
        match (sent, read) {
            (_, Ok(_)) if has_status => Ok(answer),
            // **Answered, then cut without a close_notify.** A status line and
            // a complete head arrived; the body may be short, and the caller
            // is told so rather than handed a truncation as whole.
            (_, Err(e)) if has_status => Err(read_failure(&self.host, "an HTTP exchange", &e, answer.len())),
            (Err(e), _) => Err(format!("cannot send the request to {}: {e}", self.host)),
            (Ok(()), Err(e)) => Err(read_failure(&self.host, "an HTTP exchange", &e, answer.len())),
            (Ok(()), Ok(_)) => Err(format!(
                "{} closed the connection without an HTTP status line ({} byte(s) arrived). \
                 Nothing is read from that.",
                self.host,
                answer.len()
            )),
        }
    }

    /// **One HTTP request over the control plane whose answer is READ as it
    /// arrives, not collected** — `cargo rivet backup` and `restore`
    /// (`ADR-051` §8, §12).
    ///
    /// `head` is a complete request head and exactly `length` bytes of `body`
    /// follow it, copied in 64 KiB pieces. The returned reader is the
    /// connection: the machine's answer, status line first, until the machine
    /// closes. Nothing here interprets either side, as with [`Tls::exchange`];
    /// the difference is that an answer as large as a backup is never held here.
    ///
    /// The connection presents this operator's control-plane identity and
    /// nothing else — the pinned key `ADR-025` §2 trusts — and that is the
    /// whole of what it authenticates with.
    ///
    /// # A send that fails is not the machine's answer
    ///
    /// [`Tls::exchange`]'s rule: a machine may refuse before reading the body,
    /// so a failed write still hands back the connection for its answer to be
    /// read.
    ///
    /// `quiet` bounds how long one read or write may wait with nothing moving,
    /// so a machine or an application that stops answering ends the call
    /// rather than holding it for ever.
    ///
    /// # Errors
    ///
    /// The connection could not be made, or `body` could not be read.
    pub fn http(
        &self,
        head: &str,
        body: &mut dyn Read,
        length: u64,
        quiet: std::time::Duration,
    ) -> Result<Box<dyn Read>, String> {
        let mut stream = self.connect()?;
        let _ = stream.sock.set_read_timeout(Some(quiet));
        let _ = stream.sock.set_write_timeout(Some(quiet));
        let mut sent = stream.write_all(head.as_bytes());
        let mut left = length;
        let mut piece = vec![0u8; 64 * 1024];
        while sent.is_ok() && left > 0 {
            let want = usize::try_from(left.min(piece.len() as u64)).unwrap_or(piece.len());
            let got = body
                .read(&mut piece[..want])
                .map_err(|e| format!("cannot read what is being sent to {}: {e}", self.host))?;
            if got == 0 {
                return Err(format!(
                    "what is being sent to {} ended {left} byte(s) short of the {length} it declared",
                    self.host
                ));
            }
            sent = stream.write_all(&piece[..got]);
            left -= got as u64;
        }
        let _ = sent.and_then(|()| stream.flush());
        Ok(Box::new(stream))
    }

    /// **Send one object to a fleet server's repository store** —
    /// `PUT /repository-objects/<name>`, `.rulings/` R-184 D1 — streamed from
    /// `local` and answered as [`Remote::push`] answers: the machine's `201`
    /// body, or a refusal that says whether the ARTIFACT was refused (a 422,
    /// with the transfer intact) or the transfer failed.
    ///
    /// `name` is the artifact's `ADR-027` name, checked here by the rule the
    /// machine applies so a refusal costs nothing on the wire.
    ///
    /// # Errors
    ///
    /// A name of the wrong shape, a file this end cannot read, a connection that
    /// could not be made, or the machine's own refusal, quoted.
    pub fn put_repository_object(&self, local: &Path, name: &str) -> Result<String, String> {
        payload_name(&format!("/state/{name}"))?;
        let (length, digest) = measure(local)?;
        let mut file = std::fs::File::open(local)
            .map_err(|e| format!("cannot read {}: {e}", local.display()))?;
        let mut stream = self.connect()?;
        let head = format!(
            "PUT {}{name} HTTP/1.1\r\n\
             Host: {host}\r\n\
             Rivet-Blake3: {digest}\r\n\
             Content-Length: {length}\r\n\
             Connection: close\r\n\
             \r\n",
            rivet_core::repository_objects::PREFIX,
            host = self.host,
            digest = hex(&digest),
        );
        transfer(&mut stream, &head, &mut file, length, local, &self.host, name)
    }

    /// **Read one object back from a fleet server's repository store** —
    /// `GET /repository-objects/<digest>` (`ADR-063` §6) — streamed into
    /// `into`, and kept only when its bytes hash to `digest`
    /// ([`receive_object`]).
    ///
    /// # Errors
    ///
    /// `digest` is not one, the connection could not be made, the machine
    /// answered other than `200`, or the body did not arrive whole or did not
    /// hash to `digest` — in which case `into` is removed.
    pub fn get_repository_object(&self, digest: &str, into: &Path) -> Result<Received, String> {
        let wanted = rivet_core::artifact::Digest::new(digest)
            .ok_or_else(|| format!("{digest:?} is not a BLAKE3 digest, so nothing was asked for"))?;
        let mut stream = self.connect()?;
        let head = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            rivet_core::repository_objects::object_route(&wanted),
            self.host,
        );
        stream
            .write_all(head.as_bytes())
            .and_then(|()| stream.flush())
            .map_err(|e| format!("cannot ask {} for object {digest}: {e}", self.host))?;
        receive_object(&mut stream, wanted.as_str(), into, &self.host)
    }
}

/// What [`Tls::get_repository_object`] received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Received {
    /// The `ADR-027` name the server holds it under.
    pub name: String,
    pub length: u64,
}

/// The largest answer head [`receive_object`] reads before refusing.
const OBJECT_HEAD_LIMIT: usize = 64 * 1024;

/// **One object's answer, from its head to its last byte, written to `into`**
/// — split out of [`Tls::get_repository_object`] so every refusal is tested
/// against bytes in memory.
///
/// The head must be `200` with a `Content-Length`, a `Rivet-Blake3` equal to
/// `expected`, and a `Rivet-Name` of `ADR-027`'s shape ending `.rvt`. Exactly
/// `Content-Length` bytes are then streamed to `into` in 64 KiB pieces and
/// hashed on the way; **a body that ends early or hashes to anything but
/// `expected` removes `into`**, so a file this returns `Ok` for is the object.
///
/// # Errors
///
/// Each of the above, named, with the machine's own words for a non-`200`.
pub fn receive_object(
    reader: &mut dyn Read,
    expected: &str,
    into: &Path,
    host: &str,
) -> Result<Received, String> {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    let at = loop {
        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        if buffer.len() > OBJECT_HEAD_LIMIT {
            return Err(format!("{host}'s answer head did not end within {OBJECT_HEAD_LIMIT} bytes"));
        }
        let got = reader
            .read(&mut chunk)
            .map_err(|e| format!("{host}'s answer could not be read: {e}"))?;
        if got == 0 {
            return Err(format!(
                "{host} closed the connection inside its answer head ({} byte(s) arrived)",
                buffer.len()
            ));
        }
        buffer.extend_from_slice(&chunk[..got]);
    };
    let head = String::from_utf8_lossy(&buffer[..at]).into_owned();
    let mut rest = buffer[at + 4..].to_vec();
    let status = head.lines().next().and_then(|l| l.split(' ').nth(1)).unwrap_or_default().to_owned();
    if status != "200" {
        let mut tail = Vec::new();
        let _ = reader.take(OBJECT_HEAD_LIMIT as u64).read_to_end(&mut tail);
        rest.extend_from_slice(&tail);
        return Err(format!(
            "{host} answered {status} for object {expected}:\n{}",
            String::from_utf8_lossy(&rest).trim_end()
        ));
    }
    let header = |name: &str| {
        head.lines()
            .skip(1)
            .filter_map(|l| l.split_once(':'))
            .find(|(n, _)| n.trim().eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim().to_owned())
    };
    // **A repeated `Content-Length` is refused whether or not the values agree**,
    // as every request parser here refuses one: two readers of the same bytes
    // that honour different fields disagree about where the object ends.
    let lengths = head
        .lines()
        .skip(1)
        .filter_map(|l| l.split_once(':'))
        .filter(|(n, _)| n.trim().eq_ignore_ascii_case("content-length"))
        .count();
    if lengths > 1 {
        return Err(format!(
            "{host}'s answer for object {expected} carries {lengths} Content-Length headers, so \
             where the object ends is not one answer and nothing was kept"
        ));
    }
    let length: u64 = header("content-length")
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| format!("{host}'s answer for object {expected} carries no Content-Length"))?;
    let stated = header("rivet-blake3").unwrap_or_default();
    if stated != expected {
        return Err(format!(
            "{host} answered object {expected} with Rivet-Blake3 {stated:?}: not the object asked \
             for, so nothing was kept"
        ));
    }
    let name = header(rivet_core::repository_objects::NAME_HEADER).unwrap_or_default();
    let named = payload_name(&format!("/state/{name}")).is_ok()
        && name.ends_with(rivet_core::package::kind::SUFFIX);
    if !named {
        return Err(format!(
            "{host} answered object {expected} under the name {name:?}, which is not an ADR-027 \
             artifact name, so nothing was kept"
        ));
    }

    let mut file =
        std::fs::File::create(into).map_err(|e| format!("cannot create {}: {e}", into.display()))?;
    let outcome = write_object_body(reader, &rest, length, &mut file, into, expected);
    drop(file);
    match outcome {
        Ok(()) => Ok(Received { name, length }),
        Err(why) => {
            let _ = std::fs::remove_file(into);
            Err(format!("{host}: {why}"))
        }
    }
}

/// [`receive_object`]'s body: `already` (what arrived with the head), then the
/// rest from `reader`, exactly `length` bytes, hashed as they are written.
fn write_object_body(
    reader: &mut dyn Read,
    already: &[u8],
    length: u64,
    file: &mut std::fs::File,
    into: &Path,
    expected: &str,
) -> Result<(), String> {
    use rivet_core::package::port::Digest as _;
    let mut hasher = rivet_crypto::Blake3::new();
    hasher.start();
    let first = &already[..already.len().min(usize::try_from(length).unwrap_or(usize::MAX))];
    file.write_all(first).map_err(|e| format!("cannot write {}: {e}", into.display()))?;
    hasher.write(first);
    let mut written = first.len() as u64;
    let mut chunk = vec![0u8; 64 * 1024];
    while written < length {
        let want = usize::try_from((length - written).min(chunk.len() as u64)).unwrap_or(chunk.len());
        let got = reader
            .read(&mut chunk[..want])
            .map_err(|e| format!("the object stopped after {written} of {length} byte(s): {e}"))?;
        if got == 0 {
            return Err(format!("the object stopped after {written} of {length} byte(s)"));
        }
        file.write_all(&chunk[..got]).map_err(|e| format!("cannot write {}: {e}", into.display()))?;
        hasher.write(&chunk[..got]);
        written += got as u64;
    }
    file.sync_all().map_err(|e| format!("cannot flush {}: {e}", into.display()))?;
    let got = hex(&hasher.finish());
    if got != expected {
        return Err(format!(
            "the {length} byte(s) received hash to {got}, not {expected}: not the object asked for, \
             so nothing was kept"
        ));
    }
    Ok(())
}

/// A file's length and BLAKE3, read in 64 KiB pieces.
fn measure(local: &Path) -> Result<(u64, [u8; 32]), String> {
    use rivet_core::package::port::Digest as _;
    let mut file =
        std::fs::File::open(local).map_err(|e| format!("cannot read {}: {e}", local.display()))?;
    let mut hasher = rivet_crypto::Blake3::new();
    hasher.start();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut length = 0u64;
    loop {
        let got = file
            .read(&mut buffer)
            .map_err(|e| format!("cannot read {}: {e}", local.display()))?;
        if got == 0 {
            break;
        }
        hasher.write(&buffer[..got]);
        length += got as u64;
    }
    Ok((length, hasher.finish()))
}

/// **The JSON-RPC a batch of tool calls is, as text.**
///
/// One `initialize`, one `notifications/initialized`, then one `tools/call` per
/// call with **id 2, 3, 4 …** — id 1 is the initialize. Written as its own
/// function so that the wire format can be asserted without a socket, which is
/// the only way this end's half of the protocol is testable at all off a board.
fn batch_script(calls: &[(String, String)]) -> String {
    let mut script = String::new();
    script.push_str(
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"rivet-remote","version":"1"}}}"#,
    );
    script.push('\n');
    script.push_str(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    script.push('\n');
    for (at, (tool, arguments)) in calls.iter().enumerate() {
        let id = at + 2;
        script.push_str(&format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{tool}","arguments":{arguments}}}}}"#
        ));
        script.push('\n');
    }
    script
}

/// **Match `count` answers back to the calls that asked for them, by id.**
///
/// The far end may interleave, and a `notifications/initialized` produces no
/// reply at all, so position is not a match and the id is. An id with no line
/// is that one call's failure and **not the batch's** — the others answered and
/// throwing their answers away would be paying for a connection and discarding
/// most of it.
fn answers_from(all: &str, count: usize, host: &str) -> Vec<Result<String, String>> {
    (0..count)
        .map(|at| {
            let id = at + 2;
            // **The comma is part of the match.** Without it `"id":1` also
            // matches `"id":10`, and a batch of nine subjects has an id 10 in
            // it — a reader that took the initialize's reply for a subject's
            // would report the handshake as the answer.
            let wanted = format!("\"id\":{id},");
            all.lines()
                .find(|line| line.contains(&wanted))
                .map_or_else(
                    || {
                        Err(format!(
                            "{host} answered nothing for call {id} of {count}. What did arrive \
                             is kept out of this answer rather than guessed at:\n{all}"
                        ))
                    },
                    tool_result,
                )
        })
        .collect()
}

/// **A read that ended early, explained by Rivet rather than by rustls.**
///
/// # Why this function exists at all
///
/// A cold agent deployed an application to a board in nineteen minutes on
/// 2026-08-29 and reported that *"the board itself explained almost everything
/// I asked it"*. The one thing that did not was `rivet get log` past roughly a
/// hundred records, which answered:
///
/// ```text
/// peer closed connection without sending TLS close_notify:
/// https://docs.rs/rustls/latest/rustls/manual/_03_howto/index.html#unexpected-eof
/// ```
///
/// That sentence is **correct and is not an answer**. It is rustls telling a
/// caller that it will not read a truncated stream as a complete one — which is
/// exactly the right behaviour, and is why the text is kept at the end of what
/// this composes rather than swallowed (`AGENTS.md` §5: report what the system
/// said). What was missing is the part only Rivet can supply: which machine,
/// which call, how much arrived, and where the other half of the account is.
///
/// # It does not diagnose, because this end cannot
///
/// The cause is on the machine, and the machine wrote it down. So this names
/// the journal rather than guessing — and names the one cause that is known to
/// have produced this exact failure, as a possibility to rule out rather than
/// as a finding.
fn read_failure(host: &str, what: &str, error: &std::io::Error, got: usize) -> String {
    if error.kind() != std::io::ErrorKind::UnexpectedEof {
        return format!(
            "{host} could not be read while {what} was in flight, after {got} byte(s): {error}"
        );
    }
    format!(
        "{host} closed the connection before {what} was complete. {got} byte(s) arrived and \
         none of them a finished answer, so NOTHING is read from them.\n\
         \x20 why  TLS has an end-of-stream of its own and this was not it: the machine went \
         away without a `close_notify`. A truncated answer and a cut connection are then \
         indistinguishable, so this end refuses rather than reading a partial result as a \
         whole one.\n\
         \x20 where the rest of the account is  on the machine. `rivet-mcpd` writes what it \
         could not send to the journal — ask for `get log` and select the  \
         records whose `svc` is `rivet-mcpd`. THAT OPERATION HAS NO SERVICE FILTER, so a  \
         service name as a word selects a BOOT and is refused (corrected 2026-08-30).\n\
         \x20 one known cause  a build of `rivet-mcpd` from before 2026-08-29 could not send \
         ANY single answer larger than 64 KiB and failed in exactly this way; `rivet get log` \
         with its default of 200 records is past that on a busy machine. If the board is \
         running an older build, ask for fewer records — `get log latest 100` — and update it.\n\
         \x20 what the TLS library said  {error}"
    )
}

/// The `text` out of one MCP `tools/call` result.
///
/// # This is a reader for one shape, not a JSON parser, and says so
///
/// `rivet_core::json` reads the whole grammar, correctly, once; what makes
/// this a reader for ONE shape rather than a general MCP client is what it
/// does with the result — it asks for exactly `result.content[0].text` and
/// `error`, and **refuses anything else by quoting the line** rather than
/// degrading. A caller reaching for this with a different message would get a
/// wrong answer confidently, and that is the failure this whole project keeps
/// finding.
///
/// # Errors
///
/// If the line is not a successful `tools/call` result, with the line quoted.
pub(crate) fn tool_result(line: &str) -> Result<String, String> {
    let message = json::parse(line).ok();

    if let Some(error) = message.as_ref().and_then(|m| m.get("error")) {
        let detail = match (
            error.get("message").and_then(Json::as_str),
            error.get("code").and_then(Json::as_integer),
        ) {
            (Some(text), Some(code)) => format!("{text} (code {code})"),
            (Some(text), None) => text.to_owned(),
            (None, _) => line.to_owned(),
        };
        return Err(format!("the machine refused the call: {detail}"));
    }

    let text = message.as_ref().and_then(text_of).ok_or_else(|| {
        format!(
            "the machine's answer is not the shape this build reads. It wanted one \
             `tools/call` result carrying `content[0].text`, and got:\n{line}"
        )
    })?;

    let is_error = message
        .as_ref()
        .and_then(|m| m.get("result"))
        .and_then(|r| r.get("isError"))
        .and_then(Json::as_bool)
        .unwrap_or(false);
    if is_error {
        return Err(text.to_owned());
    }
    Ok(text.to_owned())
}

/// `result.content[0].text`, if `message` has that shape.
fn text_of(message: &Json) -> Option<&str> {
    message.get("result")?.get("content")?.as_array()?.first()?.get("text")?.as_str()
}

/// [`text_of`], against a `line` this crate has not already parsed.
///
/// Test-only: every production caller goes through [`tool_result`], which
/// parses `line` once and calls [`text_of`] on that value directly rather
/// than parsing a second time. Kept as its own function so a test can check
/// the escaping this layer reads through in isolation, the way it did before
/// [`tool_result`] read the whole envelope at once.
#[cfg(test)]
fn extract_text(line: &str) -> Option<String> {
    text_of(&json::parse(line).ok()?).map(str::to_owned)
}

// ---------------------------------------------------------------------------
// The inner layer of JSON — what the operation itself said
// ---------------------------------------------------------------------------

/// Text as the body of a JSON string, escapes and all.
///
/// **RFC 8259 §7, and the same three parts of it `cmd/rivet`'s writer calls out
/// as the ones usually got wrong**: the two mandatory escapes everybody
/// remembers (`"` and `\`), *every* code point below `0x20` rather than only the
/// five with short forms, and `DEL` (`0x7f`) deliberately left alone because it
/// is legal unescaped and escaping it would be inventing a rule.
///
/// This is the outbound half of what [`json_string_field`] undoes inbound.
/// **The characters this exists for are real
/// here** — a `[services.<name>]` table is two lines, so a naive implementation
/// that only quoted quotes would send a raw newline inside a JSON string and the
/// machine would refuse the whole call.
///
/// # Non-ASCII goes as `\uXXXX`, and that is a DEFENCE rather than a style
///
/// RFC 8259 allows a character above `0x7f` to be sent as itself, and the
/// machine's own writer does exactly that. **This one may not, because the
/// program at the other end cannot read it.** `cmd/rivet`'s `mcp` module reads
/// one JSON-RPC message per line with `read_line_capped`, and that function
/// builds the line with
///
/// ```text
/// into.push(char::from(byte[0]));
/// ```
///
/// which promotes **every byte to a code point** — Latin-1, not UTF-8. So `é`
/// (`C3 A9`) arrives at the parser as `Ã©` and a document sent through it is
/// silently mojibake. Measured on 2026-08-29 by asking a build of this
/// workspace's own `rivet mcp` for `get café`: it refused `cafÃ©`. The same call
/// spelled `café` refused `café`, and a surrogate pair for `🚀` came back
/// intact, so escaping is the spelling that survives.
///
/// **This defends the paths that go through this module and nothing else.** The
/// defect is in `cmd/rivet` and belongs to whoever owns that file; anything else
/// that sends non-ASCII to `rivet mcp` is still corrupted, and
/// [`put_document_via`] reads back what it wrote rather than trusting this to be
/// enough.
pub(crate) fn json_escape(text: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            // Four hex digits, lower case, as `cmd/rivet`'s writer emits them.
            // `write!` to a `String` cannot fail and this crate denies `unwrap`,
            // so the result is discarded rather than unwrapped.
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c if (c as u32) < 0x80 => out.push(c),
            // Above the BMP: the surrogate pair JSON carries it as. Not this
            // module's invention — RFC 8259 §7 — and the parser at the other end
            // reassembles it, which was measured rather than assumed.
            c if (c as u32) > 0xFFFF => {
                let code = c as u32 - 0x1_0000;
                let _ = write!(
                    out,
                    "\\u{:04x}\\u{:04x}",
                    0xD800 + (code >> 10),
                    0xDC00 + (code & 0x3FF)
                );
            }
            c => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
        }
    }
    out
}

/// The value of `"<key>":"…"` in an operation's own JSON, decoded.
///
/// **Reads the document's own top-level field, not a same-named field nested
/// inside one of its arrays.** `get file`'s `type` is the path's own even
/// though every entry in `entries` carries a `type` too — [`Json::get`] looks
/// only at the object handed to it, so a directory entry's `type` cannot be
/// read back as the path's. See [`json_string_field_all`] for a key that
/// recurs by design.
///
/// # Still a reader for one shape, and stricter than the outer one
///
/// `extract_text` reads the outer layer — the `tools/call` result — and is
/// deliberately forgiving there, because that layer can only ever carry `\"`
/// and `\\`: everything else was already escaped by the operation's own
/// writer, so a raw newline or a `\b` cannot reach it. This reads the layer
/// *underneath*, which is where the document's own bytes are, through the same
/// parser and therefore no more forgiving than RFC 8259 itself.
///
/// # Public since `ADR-052` §9
///
/// The drift comparison (`cmd/cargo-rivet/src/fleet_drift.rs`) reads `get
/// status` and `get file` answers the same way this module reads `get
/// document` — one operation's JSON, a handful of fields this end cares about.
/// `Tls::call` itself stays unexported — see [`CallsTools`] — so this widens
/// the reader and not the connection.
///
/// # Errors
///
/// If `source` does not parse as JSON, the key is not there, or its value is
/// not a JSON string.
pub fn json_string_field(source: &str, key: &str) -> Result<String, String> {
    let document = read_answer(source)?;
    document
        .get(key)
        .and_then(Json::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            format!(
                "the machine's answer carries no `{key}` string, so this build cannot read it. \
                 It said:\n{source}"
            )
        })
}

/// **Every `"<key>":"…"` in one JSON document, decoded, in the order the
/// parser reads them** — [`json_string_field`] reads the document's own
/// field; this is for a key that recurs on purpose, which is what a JSON
/// array of objects carries one of per element.
///
/// Added for `ADR-052` §9's drift comparison: `get status`'s `services` is an
/// array of objects and `"name"` appears once per entry, so reading the whole
/// list needs every occurrence rather than the first.
///
/// **An absent key is an empty list, not an error.** A machine reporting no
/// services is a real answer — `rivet_core::operation::status` tests exactly
/// this case — and treating "nothing matched" as a refusal here would be the
/// same confusion `get file`'s `present: false` exists to end: false is an
/// answer, not a failure to answer.
///
/// # Errors
///
/// If `source` does not parse as JSON.
pub fn json_string_field_all(source: &str, key: &str) -> Result<Vec<String>, String> {
    let document = read_answer(source)?;
    let mut found = Vec::new();
    collect_strings(&document, key, &mut found);
    Ok(found.into_iter().map(str::to_owned).collect())
}

/// Every string value at `key`, wherever it occurs under `value` — inside a
/// nested object, or one per element of an array of them — depth-first, in
/// the order the parser produced them.
///
/// A value at a matching key that is not itself a string is skipped rather
/// than refused: the same thing [`json_string_field`] does for the one
/// occurrence it reads, applied to every occurrence this reads.
fn collect_strings<'a>(value: &'a Json, key: &str, out: &mut Vec<&'a str>) {
    match value {
        Json::Object(fields) => {
            for (field, field_value) in fields {
                if field == key
                    && let Some(s) = field_value.as_str()
                {
                    out.push(s);
                }
                collect_strings(field_value, key, out);
            }
        }
        Json::Array(items) => {
            for item in items {
                collect_strings(item, key, out);
            }
        }
        _ => {}
    }
}

/// Parse `source` — one operation's own JSON, the string [`extract_text`]
/// already decoded out of the outer envelope — refusing with the answer
/// quoted rather than guessing at whatever bytes it happens to contain.
///
/// Every reader below calls this once, so a document that does not parse at
/// all is refused the same way regardless of which field a caller was after,
/// and a field-specific message is reserved for a document that DOES parse
/// but does not carry what was asked for.
fn read_answer(source: &str) -> Result<Json, String> {
    json::parse(source).map_err(|e| {
        format!(
            "the machine's answer is not JSON this build reads ({e}), so nothing can be read \
             from it. It said:\n{source}"
        )
    })
}

/// A `"<key>":true|false` in an operation's own JSON.
///
/// Public since `ADR-052` §9 for [`json_string_field`]'s own reason — the
/// drift comparison reads `get file`'s `present` this way rather than a
/// second implementation of the same field lookup.
///
/// # Errors
///
/// If `source` does not parse as JSON, the key is absent, or its value is not
/// a boolean.
pub fn json_bool_field(source: &str, key: &str) -> Result<bool, String> {
    let document = read_answer(source)?;
    match document.get(key) {
        None => Err(format!(
            "the machine's answer carries no `{key}`, and this build will not assume one. It \
             said:\n{source}"
        )),
        Some(value) => value.as_bool().ok_or_else(|| {
            format!(
                "`{key}` is neither `true` nor `false` in the machine's answer, so what it \
                 means is not decidable. It said:\n{source}"
            )
        }),
    }
}

/// A `"<key>":<digits>` in an operation's own JSON.
///
/// # Errors
///
/// If `source` does not parse as JSON, the key is absent, or its value is not
/// an integer.
fn json_integer_field(source: &str, key: &str) -> Result<i64, String> {
    let document = read_answer(source)?;
    match document.get(key) {
        None => Err(format!(
            "the machine's answer carries no `{key}`, and this build will not assume one. It \
             said:\n{source}"
        )),
        Some(value) => value.as_integer().ok_or_else(|| {
            format!("`{key}` is not a number in the machine's answer. It said:\n{source}")
        }),
    }
}

// ---------------------------------------------------------------------------
// The desired-state document, over the catalogue
// ---------------------------------------------------------------------------

/// The `rivet get` subject that answers with the desired-state document.
///
/// ~~**A literal, and it is the one spelling this crate has.** `rivet-core`
/// publishes `SUBJECT as EDIT` and four others from `operation.rs` and has no
/// entry for this one, so there is nothing to import. That is a smaller risk
/// than it looks: a machine whose catalogue no longer knows this word answers
/// `E_UNKNOWN_SUBJECT` and this transport reports it, which is a refusal an
/// operator can read rather than a wrong answer.~~
///
/// **Corrected 2026-09-05.** `rivet-core` publishes it now, as
/// `operation::DESIRED_STATE`, because `ADR-044`'s fleet server composes the
/// same call and a third spelling would have been one too many. The paragraph
/// above was true when it was written and is left readable — `AGENTS.md` §2
/// rule 3 — because the reasoning it gives for tolerating the copy is still
/// worth having: the failure mode of getting this word wrong is a refusal, not
/// a wrong answer.
const DOCUMENT_SUBJECT: &str = rivet_core::operation::DESIRED_STATE;

/// One MCP `tools/call`, as the three document methods need it.
///
/// **A seam, not an abstraction** — `AGENTS.md` §3.1's second kind, named in the
/// adapter that owns it. `Tls::call` needs a socket and a board; composing the
/// arguments and reading the answer needs neither, and with this between them
/// the whole round trip is exercisable against a fake `rivet mcp` in the same
/// process. There is no other way to test it without hardware, and an untested
/// escaper is exactly the thing that writes a plausible wrong document.
///
/// # Public since 2026-08-29, and this is the narrow surface rather than `call`
///
/// [`crate::command`] performs an arbitrary catalogue operation and needs
/// exactly one thing from this module: *call a tool, give me the text*. Making
/// `Tls::call` `pub` would publish a connection, a hand-rolled JSON-RPC
/// script, and the close-notify that took a board hanging mid-deploy to find
/// (`PLAN.md` §81) — none of which a caller may vary and all of which it would
/// then be able to see. **This trait publishes the answer and keeps the
/// machinery**, and it is what a caller can be tested against: everything above
/// it runs against a fake in the same process, with no socket and no board.
pub trait CallsTools {
    /// Call `tool` with `arguments` (a JSON object, composed) and hand back the
    /// text of its result.
    ///
    /// # Errors
    ///
    /// Whatever the machine refused with, or the transport failed with.
    fn call_tool(&self, tool: &str, arguments: &str) -> Result<String, String>;
}

impl CallsTools for Tls {
    fn call_tool(&self, tool: &str, arguments: &str) -> Result<String, String> {
        self.call(tool, arguments)
    }
}

/// `rivet get document`, and the document out of what it answered.
///
/// **`present: false` is `Ok(None)` and never an empty string.** That is
/// [`Remote::read_document`]'s contract: a machine that has never been
/// configured has no document, and
/// `--enable` must refuse rather than compose a first one — a document without
/// `version` is invalid (`ADR-007` §7.1) and writing the first one is
/// provisioning's job (`ADR-004` §6).
///
/// # Errors
///
/// If the call failed, if the answer is not this subject's shape, or if the
/// document does not decode to the length the machine said it wrote.
fn read_document_via(caller: &dyn CallsTools) -> Result<Option<String>, String> {
    let answer = caller.call_tool(
        "rivet_get",
        &format!(r#"{{"subject":"{DOCUMENT_SUBJECT}"}}"#),
    )?;
    document_from(&answer)
}

/// The document out of one `get document` result.
///
/// **The length is checked against what the machine reported**, and that check
/// is the point of the subject carrying `bytes` at all from this end's
/// perspective. Two layers of unescaping stand between the file and this string;
/// a mistake in either produces text that is *plausible* — a lost backslash, a
/// newline that became the letter `n` — and nothing downstream could tell. The
/// machine already counted the bytes it read, so this compares and refuses.
fn document_from(answer: &str) -> Result<Option<String>, String> {
    let subject = json_string_field(answer, "subject")?;
    if subject != DOCUMENT_SUBJECT {
        return Err(format!(
            "this transport asked for `{DOCUMENT_SUBJECT}` and the machine answered about \
             `{subject}`. That is a different question and its answer is not the document, so \
             nothing is read from it."
        ));
    }
    if !json_bool_field(answer, "present")? {
        return Ok(None);
    }
    let contents = json_string_field(answer, "contents")?;
    let said = json_integer_field(answer, "bytes")?;
    let decoded = i64::try_from(contents.len()).unwrap_or(i64::MAX);
    if said != decoded {
        return Err(format!(
            "the machine said the document is {said} bytes and this end decoded {decoded}. The \
             two layers of JSON escaping between the file and this string have gone wrong \
             somewhere, and a document that is nearly right is worse than none: it would be \
             written back. Nothing is read from this answer."
        ));
    }
    Ok(Some(contents))
}

/// `rivet apply edit <document> … confirm`, and whatever it said.
///
/// # The change is an array of strings, because the catalogue says so
///
/// `apply`'s `change` input is the catalogue's only `several: true` one, so over
/// MCP it is a JSON **array** and `cmd/rivet`'s `mcp` module flattens it into the
/// positional word vector between `slot` and `confirmation` — exactly the words
/// an operator would type. `["--append", "<text>"]`, or
/// `["--expect", "<a>", "--replace", "<b>"]`.
///
/// # Errors
///
/// If the call failed, or the machine refused the edit — which includes
/// `--expect` matching zero times or more than once, the refusal this whole
/// operation exists for.
fn edit_document_via(caller: &dyn CallsTools, change: &ConfigChange) -> Result<String, String> {
    let words: Vec<&str> = match change {
        ConfigChange::Append { text } => vec!["--append", text],
        ConfigChange::Replace { expect, replace } => {
            vec!["--expect", expect, "--replace", replace]
        }
    };
    let change = words
        .iter()
        .map(|word| format!("\"{}\"", json_escape(word)))
        .collect::<Vec<_>>()
        .join(",");
    caller.call_tool(
        "rivet_apply",
        &format!(
            r#"{{"subject":"{subject}","slot":"{slot}","change":[{change}],"confirmation":"{confirm}"}}"#,
            subject = rivet_core::operation::EDIT,
            slot = json_escape(crate::deploy::DOCUMENT_PATH),
            confirm = rivet_core::operation::CONFIRMATION,
        ),
    )
}

/// Replace the document wholesale — as a compare-and-swap, because that is the
/// only shape the catalogue has.
///
/// # This is not a workaround, it is the stronger operation
///
/// `apply edit` has no *"write this file whole"* mode, so this reads the current
/// text and asks for a [`ConfigChange::Replace`] of it by the new one. What
/// falls out is compare-and-swap: **if the document changed between the read and
/// the write, `--expect` no longer matches and the machine refuses** rather than
/// clobbering somebody else's edit (`ADR-005` §5.5.2.2). A transport that
/// simply overwrote the file could not do that and would never notice.
///
/// **One race is not caught, and it is stated rather than implied.** The
/// comparison is `apply edit`'s — *this text occurs exactly once* — so a
/// concurrent editor that only **appended** leaves what was read still occurring
/// exactly once, the swap succeeds, and the appended tail survives *after* the
/// new text. That is a document nobody composed — and **the read-back below is
/// what refuses it**, because what is on the machine is then neither document.
/// A transport that overwrote the file would lose the other edit entirely and
/// say nothing.
///
/// # No document is a refusal, and deliberately not a first write
///
/// Creating one is the wrong instinct, for the reason [`Remote::read_document`]
/// gives: a document without `version` is not a valid one (`ADR-007` §7.1) and
/// composing the first one is provisioning's job (`ADR-004` §6). There is also
/// nothing to compare against, so the write would be exactly the blind clobber
/// `apply edit` exists to make impossible. It refuses and says what to do.
///
/// **An empty document is not an absent one** and is written by appending:
/// `--expect ""` would match everywhere, which the machine refuses as ambiguous,
/// and appending to an empty file leaves precisely the new text.
///
/// # Errors
///
/// If there is no document, if the read failed, or if the machine refused the
/// edit — including a document that changed underneath, and one larger than
/// `apply edit`'s 64 KiB cap on a single `--expect`.
fn put_document_via(caller: &dyn CallsTools, text: &str) -> Result<(), String> {
    let path = crate::deploy::DOCUMENT_PATH;
    let Some(current) = read_document_via(caller)? else {
        return Err(format!(
            "there is no {path} on this machine, so there is nothing to replace and this \
             transport will not write the first one.\n\
             \x20 why  a document without `version` is not a valid one (ADR-007 §7.1) and \
             composing the first one is provisioning's job (ADR-004 §6). This transport \
             replaces the document by telling the machine what it believes is there \
             (`apply edit --expect`), and against no document there is nothing to believe — \
             the write would be the blind clobber that operation exists to make impossible.\n\
             \x20 fix  provision the machine. A card carries its first document \
             (ADR-004 §6), and this transport replaces one rather than creating it."
        ));
    };
    // Already what was asked for. Saying so costs a call and a whole-file
    // rewrite, and the postcondition — the document at `path` is `text` — is
    // already true.
    if current == text {
        return Ok(());
    }
    let change = if current.is_empty() {
        ConfigChange::Append { text: text.to_owned() }
    } else {
        ConfigChange::Replace { expect: current, replace: text.to_owned() }
    };
    edit_document_via(caller, &change)?;

    // **Read it back, for `push`'s reason.** That method does not trust the
    // transfer either: the board hashes what it received and the digests are
    // compared, because a channel fault would otherwise surface later as
    // something that points somewhere else entirely. The same is true here and
    // is not hypothetical — see [`json_escape`], which exists because one end of
    // this channel mangles bytes it cannot read. A document is small and a
    // second call is cheap; **a machine configured with a document nobody wrote
    // is not.**
    match read_document_via(caller)? {
        Some(now) if now == text => Ok(()),
        Some(now) => Err(format!(
            "{path} on the machine is NOT the document that was sent, and the machine accepted \
             the edit — so this is a fault in the CHANNEL and not in the document.\n\
             \x20 first difference  {}\n\
             \x20 what to do  read it with `rivet get document` and repair it before anything \
             reloads: what is on the machine now is neither the old document nor the new one.",
            first_difference(text, &now),
        )),
        None => Err(format!(
            "the machine accepted the edit and now reports NO {path} at all. Nothing here can \
             say what is on it; read it with `rivet get document` before anything reloads."
        )),
    }
}

/// Where two strings first differ, as a sentence a person can act on.
///
/// **Characters and their code points**, because the failure this exists to
/// describe is an encoding one: `é` against `Ã©` is unreadable as a diff and
/// obvious as `U+00E9` against `U+00C3`.
fn first_difference(sent: &str, got: &str) -> String {
    for (at, (a, b)) in sent.chars().zip(got.chars()).enumerate() {
        if a != b {
            return format!(
                "character {at}: sent {a:?} (U+{:04X}), the machine has {b:?} (U+{:04X})",
                a as u32, b as u32
            );
        }
    }
    format!(
        "the first {} characters agree; what was sent is {} characters and the machine has {}",
        sent.chars().count().min(got.chars().count()),
        sent.chars().count(),
        got.chars().count(),
    )
}

/// Hex, for the digest header.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The name a `/state/<name>` path ends in, if it is one this may send.
///
/// **The same closed rule the endpoint applies**, checked here so a refusal
/// arrives before 683 MB is streamed rather than after.
fn payload_name(remote_path: &str) -> Result<&str, String> {
    let name = remote_path.strip_prefix("/state/").ok_or_else(|| {
        format!(
            "the bulk endpoint writes to /state and this path is {remote_path:?}. \
             ADR-035 §3.2's PUT names a file, not a location — a transfer that could \
             choose a directory would be a remote filesystem."
        )
    })?;
    let ok = !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if ok {
        Ok(name)
    } else {
        Err(format!(
            "{name:?} is not a name the bulk endpoint accepts — letters, digits, `.`, `-` and \
             `_`, at most 128, not starting with a dot (ADR-027 §7's shape). Refused here so \
             nothing is sent."
        ))
    }
}

/// **One `PUT`, as bytes on a stream** — split out of [`Tls::push`] so that the
/// rule below can be exercised without a board.
///
/// Answers what the machine said in its `201`, for `crate::deploy::run` to
/// relay.
///
/// # A write that fails is not an answer, and this end asks for one anyway
///
/// **The rule: when the send fails, READ. The peer may already have answered.**
///
/// `rivet-01`, 2026-09-12, the first hardware push of this client. It reported
/// *"the transfer failed after 196608 of 402198 bytes: An established connection
/// was aborted"* — deterministic, the identical offset on two runs — and the
/// machine had logged `201 Created` for every one of three pushes and held the
/// artifact complete, at 402198 bytes, with nothing stranded in `incoming`. The
/// transfer had worked. What this end did with a working transfer was call it an
/// error and never ask for the install it had earned.
///
/// The bulk endpoint short-circuits a re-send of an artifact it already holds:
/// it answers `201` without draining the request body, and the close that
/// follows — with bytes still arriving — is a TCP `RST` rather than a `FIN`. So
/// a failed `write_all` here says nothing about whether the transfer succeeded,
/// and treating it as a verdict is this end deciding a question only the machine
/// can answer.
///
/// **This half does NOT recover the `rivet-01` case, and that was measured
/// rather than assumed.** `cmd/rivet-mcpd`'s `bulk::drain` is the fix; this is a
/// defence beside it. On a Windows host, loopback TCP, 2026-09-12: a peer that
/// answered and then closed with the body still arriving produced
/// `WinError 10053, "An established connection was aborted"` on the write — the
/// board's own wording — and **a read afterwards returned zero bytes and the
/// same error**. A reset discards the receive queue, so the answer is gone
/// before this end can ask for it.
///
/// The consequence is worth stating where somebody will read it: **a board
/// running a build from before this change keeps reporting the failure until it
/// is updated.** Fixing the client does not fix the machines already out there.
/// Nothing about a transfer is lost by that — the artifact IS in the store, and
/// `rivet apply install <digest> confirm` asks for what the deploy would have
/// asked for.
///
/// **It is kept because the rule is right and the cost is a read.** A write
/// failure is not a verdict on a transfer, and the cases where the answer does
/// survive are real: a peer that finished a clean `close_notify` before this end
/// noticed, a stack that delivers what is buffered before reporting the error,
/// and any future short-circuit that shuts down cleanly. A `201` read after a
/// failed write is believed, because refusing a transfer the machine has stated
/// it received is this end overruling the only party that knows — `blake3` here
/// is this end's own hash of the file it sent and confirms nothing about the
/// wire. The answer is relayed to the operator with a note, so a deploy that
/// took this path says so.
///
/// **When the read gives nothing, the write failure is what is reported**, and
/// it says plainly that whether the machine received the payload is not known
/// from this end.
fn transfer(
    stream: &mut (impl Read + Write),
    head: &str,
    body: &mut dyn Read,
    length: u64,
    local: &Path,
    host: &str,
    name: &str,
) -> Result<String, String> {
    // What went wrong while sending, if anything. **Held rather than returned**
    // — see above: the peer gets asked either way.
    let mut trouble: Option<String> = None;

    if let Err(e) = stream.write_all(head.as_bytes()) {
        trouble = Some(format!("cannot send the request head: {e}"));
    }

    // **Streamed, never buffered.** The payload is 683 MB and this process
    // is not the constraint — the board is — but holding it here would make
    // a workstation's memory a deploy's ceiling for no reason.
    if trouble.is_none() {
        let mut buffer = vec![0u8; 64 * 1024];
        let mut sent: u64 = 0;
        while sent < length {
            // A local read failure is this machine's own and there is no answer
            // to go looking for: nothing was put on the wire to answer.
            let got = body
                .read(&mut buffer)
                .map_err(|e| format!("cannot read {}: {e}", local.display()))?;
            if got == 0 {
                break;
            }
            if let Err(e) = stream.write_all(&buffer[..got]) {
                trouble = Some(format!("the transfer failed after {sent} of {length} bytes: {e}"));
                break;
            }
            sent += got as u64;
        }
        if trouble.is_none()
            && let Err(e) = stream.flush()
        {
            trouble = Some(format!("cannot flush the transfer: {e}"));
        }
    }

    let mut answer = String::new();
    let read = stream.read_to_string(&mut answer);
    let status = answer.lines().next().and_then(|l| l.split(' ').nth(1));
    let said = answer
        .split_once("\r\n\r\n")
        .map_or(answer.as_str(), |(_, body)| body)
        .to_string();

    match (status, trouble) {
        (Some("201"), None) => Ok(said),
        (Some("201"), Some(why)) => Ok(format!(
            "{said}\x20 note    this end's send FAILED and the machine had already answered: \
             {why}. The 201 above is the machine's own and it is what this end believes — a \
             build of rivet-mcpd from before 2026-09-12 answers a re-send of an artifact it \
             already holds without reading the body out, and the hang-up that follows resets \
             the connection mid-write (ADR-052 §7).\n"
        )),
        // **The machine REFUSED THE ARTIFACT, and the transfer worked.** A 422 is
        // the bulk endpoint's door (`ADR-052` §7): the bytes arrived intact,
        // their digest matched, and they are not an artifact this machine would
        // install — so they were never stored. Reported as exactly that, and
        // never as a transfer failure: the mirror image of the 2026-09-12 defect
        // would be an operator told the wire failed and retrying an artifact
        // that will be refused identically every time. A failed write beside a
        // 422 changes nothing, because the refusal is the machine's verdict on
        // bytes it had already read in full.
        (Some("422"), _) => Err(format!(
            "{host} REFUSED THE ARTIFACT {name}. The transfer itself was intact — this is not a \
             transfer failure, and sending it again will be refused the same way. Nothing was \
             stored and nothing was installed.\n{said}"
        )),
        (Some(_), _) => Err(format!("the machine refused the payload:\n{said}")),
        // Nothing readable came back. The send failure is then the whole of what
        // is known, and it is reported rather than dressed up.
        (None, Some(why)) => Err(format!(
            "{why}\n\
             \x20 and   nothing could be read back afterwards, so whether {host} received the \
             payload is NOT known from this end. `rivet get file` against that machine's \
             artifact store is what settles it; a re-send is safe either way, since the store \
             is keyed by the hash of the contents."
        )),
        (None, None) => Err(match read {
            Err(e) => read_failure(host, &format!("the answer to the transfer of {name}"), &e, answer.len()),
            Ok(_) => format!(
                "{host} said nothing at all about the transfer of {name}: the body was sent \
                 whole, the connection ended cleanly, and no status line arrived. Nothing is \
                 read from that."
            ),
        }),
    }
}

impl Remote for Tls {
    fn push(&mut self, local: &Path, remote_path: &str) -> Result<String, String> {
        let name = payload_name(remote_path)?;
        let mut file = std::fs::File::open(local)
            .map_err(|e| format!("cannot read {}: {e}", local.display()))?;
        let length = file
            .metadata()
            .map_err(|e| format!("cannot measure {}: {e}", local.display()))?
            .len();

        // **Hashed here as well as there**, which is the point: the header is
        // this end's claim and the board compares it to what it receives.
        let digest = {
            use rivet_core::package::port::Digest as _;
            let mut hasher = rivet_crypto::Blake3::new();
            hasher.start();
            let mut buffer = vec![0u8; 64 * 1024];
            loop {
                let got = file
                    .read(&mut buffer)
                    .map_err(|e| format!("cannot read {}: {e}", local.display()))?;
                if got == 0 {
                    break;
                }
                hasher.write(&buffer[..got]);
            }
            hasher.finish()
        };
        file = std::fs::File::open(local)
            .map_err(|e| format!("cannot re-read {}: {e}", local.display()))?;

        let mut stream = self.connect()?;
        let head = format!(
            "PUT /payload/{name} HTTP/1.1\r\n\
             Host: {host}\r\n\
             Rivet-Blake3: {digest}\r\n\
             Content-Length: {length}\r\n\
             Connection: close\r\n\
             \r\n",
            host = self.host,
            digest = hex(&digest),
        );
        let said = transfer(
            &mut stream,
            &head,
            &mut file,
            length,
            local,
            &self.host,
            name,
        )?;

        // The board reports what it hashed. Kept for `blake3`, which is the
        // check `crate::deploy::run` makes next.
        self.verified = Some((remote_path.to_string(), digest));
        Ok(said)
    }

    fn blake3(&mut self, remote_path: &str) -> Result<[u8; 32], String> {
        match &self.verified {
            Some((path, digest)) if path == remote_path => Ok(*digest),
            _ => Err(format!(
                "this transport has not sent {remote_path} in this session, so it has no digest \
                 for it.\n\
                 \x20 why  the bulk endpoint hashes as it RECEIVES and refuses a mismatch \
                 before the file gets its final name (ADR-035 §3.2), so the check is part of \
                 the transfer rather than a second question afterwards. There is no `measure` \
                 verb in the MCP catalogue to ask separately with.\n\
                 \x20 fix  push it first. There is no second transport to send anyone to."
            )),
        }
    }

    fn install(&mut self, digest: &str) -> Result<String, String> {
        // **Checked here as well as on the machine**, and not because the
        // machine's check is in doubt: this string is interpolated into a JSON
        // document, and a `Digest` is 64 characters of hex, which is the one
        // shape that cannot carry a quote or a backslash. `rivet_core` owns the
        // rule, so there is no second definition of what a digest is.
        if rivet_core::artifact::Digest::new(digest).is_none() {
            return Err(format!(
                "{digest:?} is not a BLAKE3 digest, and `rivet apply install` takes nothing else \
                 (ADR-052 §7). Nothing has been asked for."
            ));
        }
        self.call(
            "rivet_apply",
            &format!(r#"{{"subject":"install","slot":"{digest}","confirmation":"confirm"}}"#),
        )
    }

    fn read_document(&mut self) -> Result<Option<String>, String> {
        read_document_via(self)
    }

    fn edit_document(&mut self, change: &ConfigChange) -> Result<String, String> {
        edit_document_via(self, change)
    }

    fn put_document(&mut self, text: &str) -> Result<(), String> {
        put_document_via(self, text)
    }

    /// **`.rulings/` R-157 R15**: `get file /state/lib`, read exactly as
    /// [`crate::deploy::Remote::installed_libraries`]'s own documentation
    /// says — the same call `cargo rivet fleet drift` already makes.
    fn installed_libraries(&mut self) -> Result<Vec<String>, String> {
        const LIBRARY_ROOT: &str = "/state/lib";
        let answer = crate::command::perform(self, &["get", "file", LIBRARY_ROOT])
            .map_err(|why| format!("`get file {LIBRARY_ROOT}` did not answer: {why}"))?;
        if !json_bool_field(&answer, "present")? {
            return Ok(Vec::new());
        }
        // The FIRST `type` is the path's own, exactly as
        // `cargo rivet fleet drift`'s own reader relies on.
        let kind = json_string_field(&answer, "type")?;
        if kind != "directory" {
            return Err(format!(
                "{LIBRARY_ROOT} is a {kind} and not a directory, so it lists no library"
            ));
        }
        json_string_field_all(&answer, "name")
    }

    /// **`ADR-055` §3, `.rulings/` R-194**: `get file /state/app/<name>/data`,
    /// read exactly as [`installed_libraries`] reads `/state/lib` — one
    /// directory along, and the same operation the control plane already
    /// offers.
    ///
    /// `Ok(None)` for a path that is not there: the installer makes this
    /// directory, so its absence means the application is not installed here.
    ///
    /// [`installed_libraries`]: crate::deploy::Remote::installed_libraries
    fn data_directory_files(&mut self, application: &str) -> Result<Option<Vec<String>>, String> {
        let directory = format!("/state/app/{application}/data");
        let answer = crate::command::perform(self, &["get", "file", &directory])
            .map_err(|why| format!("`get file {directory}` did not answer: {why}"))?;
        if !json_bool_field(&answer, "present")? {
            return Ok(None);
        }
        // The FIRST `type` is the path's own, as `installed_libraries` relies
        // on above.
        let kind = json_string_field(&answer, "type")?;
        if kind != "directory" {
            return Err(format!(
                "{directory} is a {kind} and not a directory, so it holds no material"
            ));
        }
        json_string_field_all(&answer, "name").map(Some)
    }

    /// **`ADR-063` §4**: `get fleet-root`'s `present`, the read `cargo rivet
    /// fleet root` already makes.
    fn holds_fleet_root(&mut self) -> Result<bool, String> {
        let answer = crate::command::perform(self, &["get", "fleet-root"])
            .map_err(|why| format!("`get fleet-root` did not answer: {why}"))?;
        json_bool_field(&answer, "present")
    }

    fn ask(&mut self, words: &[&str]) -> Result<String, String> {
        Err(format!(
            "this transport cannot run `rivet {}` as words.\n\
             \x20 why  MCP is the CATALOGUE, not a command line: a tool call names an \
             operation and its declared inputs, and `ask` takes a word list precisely because \
             the transports it was written for handed the machine a shell. ADR-006 §3.1's \
             co-equality is what makes the two different shapes rather than one.\n\
             \x20 fix  `rivet_remote::command::perform` is this word list bound to the \
             CATALOGUE and sent as a tool call, which is the translation this refusal declines \
             to guess at.",
            words.join(" ")
        ))
    }

    /// **The override [`ask`](Remote::ask)'s own refusal names as the fix.**
    /// `Tls` implements [`CallsTools`] for real (`impl CallsTools for Tls`,
    /// above — `call_tool` is `self.call(..)`), so this is
    /// `crate::command::perform(self, words)` rather than the trait's default
    /// of re-asking `ask`, which would just refuse again.
    fn perform_operation(&mut self, words: &[&str]) -> Result<String, String> {
        crate::command::perform(self, words)
    }
}

// **`unsupported()` was here until 2026-08-29**, and it is gone rather than
// kept: it was the one refusal shape for the three document methods, and every
// sentence in it — that MCP has no verb for reading a file, that `ADR-035` §9 is
// still ahead — stopped being true when `get document` landed. A helper whose
// body asserts something false is worse than no helper, and there is nothing
// left in this module that it would be honest about. `blake3` and `ask` refuse
// in their own words, for their own reasons, and both are unchanged.

#[cfg(test)]
#[path = "tls_tests.rs"]
mod tests;
