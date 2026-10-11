//! `rivet-mcpd` — the control plane's listener. `ADR-035` phase 1.
//!
//! # This is a transport and not a second MCP implementation
//!
//! **The decision this crate exists to record.** `cmd/rivet`'s `mcp` module is
//! 1069 lines that know what a tool is, how `CATALOGUE` projects into one, and
//! what JSON-RPC looks like. None of that is repeated here and none of it may
//! be: `ADR-011` gives a fact one home, and *what the MCP surface is* is that
//! module's.
//!
//! `ADR-032` §4.1 chose its transport because it was *"already how a caller
//! gets a program's stdin and stdout on this machine"*. `ADR-035` replaces the
//! transport and not the program, so this daemon does exactly what that
//! transport did for `rivet mcp` and nothing more: it accepts a connection and
//! gives the caller that program's stdin and stdout.
//!
//! **The alternative, and why it was not taken first.** `cmd/rivet`'s `mcp`,
//! `json` and `value` modules could move into a library the way `rivet-remote`
//! did, and both composition roots could link it. That is the right end state
//! if the spawn ever becomes the wrong shape — but `json` alone is 762 lines
//! used by six modules, so it is a large mechanical refactor to perform *before*
//! the transport it is meant to serve has ever carried a byte. It is recorded
//! here rather than done, and `PLAN.md` §71 carries the condition that would
//! trigger it.
//!
//! **One process per connection is not a compromise, it is what happens today.**
//! Every way of running `rivet mcp` spawns one, and `ADR-017` §3.1's reaping
//! already deals with the children. It also gives each session its own address
//! space for free.
//!
//! # TLS — built 2026-08-28
//!
//! `ADR-035` §7.2's self-signed certificate pinned by public key, over
//! `ADR-025` §2's mutual trust model, implemented in `rivetos-tls` and wired up
//! here. `ADR-009` §3.7 chose the library and measured it on the board.
//!
//! **There are two transports and the difference between them is which
//! addresses they may bind:**
//!
//! * **With `--identity` and `--peers`**, every connection is mutually
//!   authenticated — this machine proves its Ed25519 key to the caller and the
//!   caller proves one this machine pins — and **any address may be bound**.
//! * **Without them**, there is no encryption and no identity, so this binds
//!   **loopback only** and refuses anything else by name. That is the mode
//!   every card shipped before today ran in, and it still works, because
//!   `ADR-035` requires each phase to leave a working board.
//!
//! **Half is refused.** An `--identity` with no `--peers` is a machine that
//! proves who it is to callers it will accept unconditionally; a `--peers` with
//! no `--identity` is the reverse. Neither is a smaller version of the trust
//! model, so [`Args::parse`] refuses both.
//!
//! # What phase 1 still does NOT include
//!
//! **The serial break-glass** (`ADR-035` §8A.1), which belongs to the same
//! phase and is not built. That absence is a refusal that names itself rather
//! than a gap to be discovered.

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use crate::dispatch::Endpoint;
use crate::transport::{Plain, Tls, Transport};

mod backups;
mod beneath;
mod bulk;
mod dispatch;
mod fleet_as_code;
mod git;
mod govern;
mod peers;
mod repository;
mod transport;

use crate::peers::LivePeers;

/// The program this hands a caller, read out of `rivet-core` rather than
/// spelled here.
///
/// `ADR-011`: `cmd/rivet-image` installs the CLI at a path and
/// `rivet_core::etc` is where that path is written down. A second literal here
/// would be a daemon that spawns nothing on the day the image moves it.
const CLI: &str = rivet_core::etc::CLI_PATH;

/// The one argument the CLI is given.
///
/// **Not configurable, and that is the point.** This daemon exists to expose
/// `rivet mcp` and nothing else; a version that could spawn an arbitrary
/// subcommand would be a remote shell with extra steps, which is precisely what
/// `ADR-035` removes.
const SUBCOMMAND: &str = "mcp";

/// What this process was asked to do.
#[derive(Debug, PartialEq, Eq)]
struct Args {
    /// Where to bind, once the document has been consulted.
    ///
    /// **Resolved at parse time rather than carried as a path**, so that a
    /// document naming an address this build cannot bind is refused before a
    /// socket exists — and so `Args` stays a statement of what this process
    /// will do rather than of where to look it up.
    listen: SocketAddr,
    /// Whether a document actually **carried** the address, for the line this
    /// prints at startup.
    ///
    /// **Not "was a document given".** That was the first version and it lied
    /// on a board: `rivet-mcpd` announced *"listening on 127.0.0.1:7777 (from
    /// the document's [control-plane] table)"* against a document with no such
    /// table, because the flag recorded that `--document` had been passed
    /// rather than that it had said anything (`PLAN.md` §82). An operator
    /// reading that line has exactly one question — *did it read my
    /// configuration* — and the first version answered yes when the answer was
    /// no.
    from_document: bool,
    /// The directory holding this machine's `device.seed` and `device.crt`.
    ///
    /// **A path, which `ADR-011` permits where an address would not be**: a
    /// path says where to read a statement from, and the statement here is an
    /// identity this daemon does not mint and cannot choose.
    identity: Option<PathBuf>,
    /// The file listing the Ed25519 public keys that may connect.
    peers: Option<PathBuf>,
}

fn usage() -> String {
    format!(
        "rivet-mcpd — the MCP control plane (ADR-035 phase 1)\n\
         \n\
         \x20 --document PATH  the configuration document. Its [control-plane]\n\
         \x20                  table says where to bind; without the table, or\n\
         \x20                  without this flag, the address is 127.0.0.1:7777.\n\
         \x20 --listen ADDR    bind THIS instead, whatever the document says.\n\
         \x20                  For running one by hand; the supervisor never\n\
         \x20                  passes it.\n\
         \x20 --identity DIR   this machine\'s TLS identity: {seed} and\n\
         \x20                  {cert}, as `cargo rivet identity new` writes them.\n\
         \x20 --peers FILE     the Ed25519 public keys allowed to connect, one\n\
         \x20                  hex key per line. `#` starts a comment.\n\
         \n\
         It hands each connection the stdin and stdout of `{CLI} {SUBCOMMAND}`.\n\
         It speaks no JSON-RPC of its own and never will (ADR-011).\n\
         \n\
         WITH --identity AND --peers every connection is mutually authenticated\n\
         (ADR-035 §7.2 over ADR-025 §2) and ANY address may be bound.\n\
         \n\
         WITHOUT THEM there is no encryption and no identity, so this binds\n\
         LOOPBACK ONLY and refuses any other address by name. A control plane\n\
         reachable from a network with no authentication is not a smaller\n\
         version of the design.\n\
         \n\
         ONE WITHOUT THE OTHER IS REFUSED: half a trust model is not one.\n",
        seed = rivetos_tls::SEED_FILE,
        cert = rivetos_tls::CERTIFICATE_FILE,
    )
}

impl Args {
    /// **Refuses a non-loopback address, and that refusal is the whole of this
    /// phase's security.**
    ///
    /// Not a warning, not a flag that can turn it off. Until `ADR-035` §7.2's
    /// certificate exists there is no way for this daemon to know who is
    /// calling, and every operation in `CATALOGUE` is reachable through it —
    /// including the three stops in `ADR-017` §3.1.5 and `apply install`.
    ///
    /// `AGENTS.md` §3: a stub refuses, it never fakes success.
    fn parse(argv: &[String]) -> Result<Self, String> {
        let mut listen: Option<SocketAddr> = None;
        let mut document: Option<PathBuf> = None;
        let mut identity: Option<PathBuf> = None;
        let mut peers: Option<PathBuf> = None;
        let mut i = 0;
        while i < argv.len() {
            match argv[i].as_str() {
                "--listen" => {
                    i += 1;
                    let raw = argv.get(i).ok_or("--listen needs an address")?;
                    listen = Some(
                        raw.parse::<SocketAddr>()
                            .map_err(|e| format!("--listen {raw:?} is not an address and port: {e}"))?,
                    );
                }
                "--document" => {
                    i += 1;
                    document =
                        Some(PathBuf::from(argv.get(i).ok_or("--document needs a path")?));
                }
                "--identity" => {
                    i += 1;
                    identity =
                        Some(PathBuf::from(argv.get(i).ok_or("--identity needs a directory")?));
                }
                "--peers" => {
                    i += 1;
                    peers = Some(PathBuf::from(argv.get(i).ok_or("--peers needs a file")?));
                }
                "-h" | "--help" => return Err(usage()),
                other => return Err(format!("{other:?} is not an option\n\n{}", usage())),
            }
            i += 1;
        }

        // **`--listen` wins, and it is the only way two answers are avoided.**
        // `ADR-011` §3's failure is two things that both look authoritative; a
        // flag that silently lost to a file, or a file that silently lost to a
        // flag, would be exactly that. So the precedence is stated here, in the
        // usage, and in what this prints at startup.
        let (listen, from_document) = match listen {
            Some(explicit) => (explicit, false),
            None => match &document {
                Some(path) => read_listen(path)?,
                None => (
                    rivet_config::control_plane::DEFAULT_LISTEN
                        .parse::<SocketAddr>()
                        .map_err(|e| format!("the built-in default is not an address: {e}"))?,
                    false,
                ),
            },
        };

        // **Half a trust model is not a trust model.** An identity with no
        // peers accepts everyone while proving who it is; peers with no
        // identity is the reverse. Both are refused by name rather than
        // silently degrading to the loopback mode, because a machine that was
        // MEANT to be authenticated and quietly is not is the worst of the
        // three outcomes.
        match (&identity, &peers) {
            (Some(_), None) => {
                return Err("--identity was given without --peers, so this machine would prove \
                            its own key to callers and then accept ANY of them. ADR-025 §2's \
                            trust is mutual and symmetric; half of it is not a smaller version \
                            of it. Give --peers a file of the Ed25519 keys allowed to connect"
                    .to_string());
            }
            (None, Some(_)) => {
                return Err("--peers was given without --identity, so this machine would demand \
                            a certificate from every caller and have none of its own to offer. \
                            The handshake cannot start. Give --identity the directory holding \
                            this machine's seed and certificate"
                    .to_string());
            }
            _ => {}
        }

        if identity.is_none() && !is_loopback(listen.ip()) {
            let source = if from_document {
                "the document's [control-plane] listen"
            } else {
                "--listen"
            };
            return Err(format!(
                "{source} names {listen}, which is not a loopback address, and no TLS \
                 identity was given.\n\n\
                 ADR-035 §7.2's certificate IS built (ADR-009 §3.7) — this daemon simply was \
                 not handed one, so it cannot tell who is calling, and EVERY operation in the \
                 catalogue is reachable through it, including `apply power-off` and `apply \
                 install`.\n\n\
                 Pass --identity and --peers to bind this address, or bind 127.0.0.1 and reach \
                 it through a tunnel you trust. Refusing rather than warning, because a warning \
                 is a thing that gets ignored once and then never read again."
            ));
        }
        Ok(Self { listen, from_document, identity, peers })
    }
}

/// Loopback, asked of the address rather than matched against a literal.
///
/// `127.0.0.1` is not the only loopback address — the whole of `127.0.0.0/8` is,
/// and `::1` is. A string comparison would refuse `127.0.0.2` and accept
/// nothing sensible in IPv6.
/// The `[control-plane]` address in a document, or the default.
///
/// # Errors
///
/// If the document cannot be read or does not parse. **A document that is
/// there and wrong is a refusal**, where a document that is absent is not: the
/// first is an operator who said something this build could not act on, and
/// the second is a machine nobody has configured.
fn read_listen(path: &PathBuf) -> Result<(SocketAddr, bool), String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        // Absent is the ordinary case on a machine with no document, and
        // `ADR-007` §7.1 will not have this program write one.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return rivet_config::control_plane::DEFAULT_LISTEN
                .parse()
                .map(|address| (address, false))
                .map_err(|e| format!("the built-in default is not an address: {e}"));
        }
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let document = rivet_config::Document::parse(&text)
        .map_err(|e| format!("{} is not a document this build can read: {e}", path.display()))?;
    // **Whether the table was there**, not whether the default applied — the
    // two differ exactly when an operator wrote the default address by hand,
    // and telling them their configuration was ignored would be wrong.
    let stated = document.control_plane().is_some();
    let listen = document.effective_control_plane().listen().to_string();
    listen.parse().map(|address| (address, stated)).map_err(|e| {
        format!(
            "[control-plane] listen = {listen:?} in {} is not an address this build can bind: \
             {e}. The schema checks this when the document is written, so a value that gets \
             here came from somewhere that did not",
            path.display()
        )
    })
}

/// Loopback, asked of the address rather than matched against a literal.
fn is_loopback(ip: IpAddr) -> bool {
    ip.is_loopback()
}

/// Give one connection the CLI's stdin and stdout.
///
/// **Line-oriented in both directions**, because that is what MCP is over
/// stdio: `cmd/rivet`'s `mcp` reads one JSON-RPC message per line and answers
/// the same way. Nothing here parses those lines — a line is an opaque string
/// that arrived from one side and is written to the other.
///
/// **`live` is looked at before each answer is passed on** (`crate::peers`):
/// an answer to a command that rewrote `peers` reaches its caller only after
/// the new set governs the next handshake.
fn serve(
    link: Arc<dyn Transport>,
    peer: SocketAddr,
    live: Option<Arc<LivePeers>>,
) -> std::io::Result<()> {
    // **Which endpoint, before anything is spawned** — ADR-035 §3.2's two
    // endpoints on one listener, and `crate::dispatch` carries the argument.
    // Spawning the CLI first and then discovering the caller wanted HTTP would
    // fork a process to throw away.
    let mut opening = [0u8; 4096];
    let read = link.read(&mut opening)?;
    if read == 0 {
        return Ok(());
    }
    match dispatch::classify(&opening[..read]) {
        Endpoint::Mcp => {}
        Endpoint::Http => {
            // **Which handler, not merely "HTTP".** Until 2026-08-29 this said
            // §3.2's bulk endpoint for every request including git's, which
            // `crate::bulk` never saw — the journal is the record and it must
            // not name a handler that did not run (`PLAN.md` §88.11).
            eprintln!(
                "rivet-mcpd: {peer}: an HTTP request — {}",
                dispatch::describe_http(&opening[..read])
            );
            let answer = bulk_transfer(&*link, &opening[..read]);
            let (status, reason) = answer.outcome();
            eprintln!("rivet-mcpd: {peer}: {status} {reason}");
            let sent = send(&*link, &answer);
            // **`close_notify` before the socket goes.** Without it a correct
            // client cannot tell a complete answer from a truncated one.
            link.finish();
            return sent;
        }
        Endpoint::Unrecognised(what) => {
            eprintln!("rivet-mcpd: {peer}: not MCP and not HTTP, refused: {what:?}");
            return link.write_line(&format!(
                "{{\"error\":\"E_PROTOCOL\",\"message\":\"this port speaks MCP: JSON-RPC, one \
                 message per line. ADR-035 3.2 rules an HTTP endpoint on this same listener \
                 for bulk payloads and git, and it is not built. What arrived was neither: \
                 {what}\"}}"
            ));
        }
    }

    let mut child = spawn(link.peer_key().as_deref())?;

    let mut to_child = child.stdin.take().ok_or_else(|| {
        std::io::Error::other("the spawned CLI accepted no stdin, so nothing can be asked of it")
    })?;
    let from_child = child.stdout.take().ok_or_else(|| {
        std::io::Error::other("the spawned CLI produced no stdout, so nothing it says can be read")
    })?;

    // The reply pump runs on its own thread, because a request and its answer
    // are not lockstep: MCP allows a notification with no reply at all, so a
    // read-then-write loop would block for an answer that is never coming.
    //
    // **It writes through the `Transport` rather than a cloned socket**, which
    // is the change TLS forced: a TLS session cannot be cloned, because both
    // directions share one record layer and one sequence number. See
    // `transport`'s header for how the two threads stay out of each other's
    // way.
    let out = Arc::clone(&link);
    let caller = link.peer_key();
    let pump = std::thread::spawn(move || -> std::io::Result<u64> {
        let mut lines = 0;
        let reader = BufReader::new(from_child);
        for line in reader.lines() {
            let line = line?;
            if let Some(live) = &live {
                live.look_and_journal(caller.as_deref(), &peer.to_string());
            }
            out.write_line(&line)?;
            lines += 1;
        }
        Ok(lines)
    });

    // Line splitting is done here rather than by a `BufReader` over the
    // transport, because `Transport` is deliberately not `Read`: it takes
    // `&self` so that both threads can hold it, and `Read` wants `&mut self`.
    // **Seeded with the bytes the dispatch already consumed.** They are the
    // caller's first message and are not re-readable from the transport.
    let mut pending: Vec<u8> = opening[..read].to_vec();
    let mut chunk = [0u8; 4096];
    loop {
        while let Some(at) = pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = pending.drain(..=at).collect();
            to_child.write_all(&line)?;
            to_child.flush()?;
        }
        let got = link.read(&mut chunk)?;
        if got == 0 {
            break;
        }
        pending.extend_from_slice(&chunk[..got]);
        // **A line has a ceiling, and reaching it ends the connection.**
        // Without this, a peer that sends bytes and never a newline grows this
        // buffer until the machine is out of memory — and on a Rivet that is
        // the supervisor's problem, not just this daemon's. The limit is far
        // above any real request: MCP carries JSON-RPC here, and a payload goes
        // through `crate::bulk` and never through this loop.
        if pending.len() > MAX_LINE_BYTES {
            return Err(std::io::Error::other(format!(
                "a single request line passed {MAX_LINE_BYTES} bytes with no newline in it. \
                 This connection is dropped rather than buffered further: MCP over this \
                 transport is one JSON-RPC message per line, and a payload belongs on the \
                 bulk endpoint"
            )));
        }
        while let Some(at) = pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = pending.drain(..=at).collect();
            to_child.write_all(&line)?;
            to_child.flush()?;
        }
    }
    // The caller hung up. Closing stdin is how the CLI is told to finish, and it
    // is what makes the pump's loop end rather than leaking a thread per
    // connection.
    drop(to_child);

    let answered = pump.join().map_err(|_| {
        std::io::Error::other("the reply thread panicked; this connection's answers are lost")
    })?;
    let status = child.wait()?;
    // **`close_notify` whatever the pump did**, and the `?` that used to be on
    // the line above is why this is written out rather than chained. A failed
    // reply returned early, `finish` never ran, and the socket simply went away
    // — so a client that had been mid-answer was told, by rustls and not by
    // this machine, that its peer *"closed connection without sending TLS
    // close_notify"*. That is a library's sentence about a machine which is
    // otherwise expected to explain its own refusals, and it named neither the
    // connection nor the reason (`PLAN.md` §92 hand-off, defect 1).
    link.finish();
    match answered {
        Ok(lines) => {
            eprintln!("rivet-mcpd: {peer} closed after {lines} answer(s); {CLI} exited {status}");
            Ok(())
        }
        // The caller cannot be told — the channel that would carry the telling
        // is the one that failed. **The journal is where it goes**, and it is
        // the other half of the answer for whoever reads `rivet get log`
        // afterwards.
        Err(e) => {
            eprintln!(
                "rivet-mcpd: {peer}: this machine could not send {CLI}'s answer, so what \
                 reached the caller is incomplete: {e}. {CLI} exited {status}."
            );
            Err(e)
        }
    }
}

/// Read the identity and the pins, and build the server's TLS configuration.
///
/// **Every refusal here happens at STARTUP**, before a socket is bound. A
/// control plane that started, accepted a connection and only then discovered
/// its pin file was unreadable would report the failure to whoever connected —
/// which is the one party that must not be told about it.
///
/// **The pins stay changeable** — the [`LivePeers`] returned beside the
/// configuration replaces them when `peers` changes (`ADR-063` §3). A
/// reload that fails leaves this startup's set in force; it never stops the
/// daemon, because the listener is already the board's only way in.
fn tls_config(
    identity_at: &std::path::Path,
    peers_at: &std::path::Path,
) -> Result<(Arc<rustls::ServerConfig>, Arc<LivePeers>), String> {
    let identity = rivetos_tls::read_identity(identity_at)?;
    let text = std::fs::read_to_string(peers_at)
        .map_err(|e| format!("cannot read the pin file at {}: {e}", peers_at.display()))?;
    let pins = rivetos_tls::parse_pins(&text)
        .map_err(|e| format!("the pin file at {}: {e}", peers_at.display()))?;

    let stated = identity.public_key().ok_or_else(|| {
        format!(
            "this machine's certificate at {} carries no Ed25519 public key, so there is \
             nothing for a caller to pin (ADR-025 §2)",
            identity_at.display()
        )
    })?;
    eprintln!(
        "rivet-mcpd: this machine's pinned key is {}",
        rivetos_tls::encode_hex(&stated)
    );
    eprintln!(
        "rivet-mcpd: {} key(s) may connect; {} is read again as each answer passes, and a \
         change is in force for the next connection",
        pins.len(),
        peers_at.display()
    );

    let provider = rivetos_tls::provider();
    let verifier = Arc::new(
        rivetos_tls::PinnedPeer::new(pins, provider.clone()).map_err(|e| e.to_string())?,
    );
    let config = rivetos_tls::live_server_config(&identity, Arc::clone(&verifier), provider)
        .map_err(|e| e.to_string())?;
    Ok((Arc::new(config), Arc::new(LivePeers::new(peers_at, verifier, text))))
}

/// One bulk transfer, from the bytes already read to the response.
///
/// **Split out of `serve` so each refusal is one expression.** Every early
/// return here is a request that will not be accepted, and each is decided
/// before a byte of body is read — see [`bulk::accept`].
/// What an HTTP request produced, and **whether it is still to be sent**.
///
/// `crate::bulk` hands back a [`bulk::Response`] for its caller to render;
/// `crate::git` writes its own bytes, because a pack is binary and that type's
/// body is a `String`. Two different things, and for one commit they had the
/// same return type with a comment saying which was which.
///
/// The comment was not enough: the git arm returned an empty `Response` meaning
/// *already sent*, the caller rendered and wrote it anyway, and the client got a
/// second status line inside the pack — `git` said **"bad line length
/// character: HTTP"**, another error nowhere near its cause (`PLAN.md` §88.7).
///
/// So the distinction is a type now, and [`send`] is the only thing that writes.
enum Answer {
    /// Already on the wire. Carried only so the caller can log it.
    Sent(u16, &'static str),
    /// To be rendered and written by [`send`].
    Compose(bulk::Response),
}

impl Answer {
    /// What to put in the log, either way.
    const fn outcome(&self) -> (u16, &'static str) {
        match self {
            Self::Sent(status, reason) => (*status, reason),
            Self::Compose(response) => (response.status, response.reason),
        }
    }
}

/// Write an [`Answer`], if it is not already written.
///
/// **The `Sent` arm writes nothing, and that is the whole job.** A test asserts
/// it, because the failure it prevents is invisible to every type check: two
/// well-formed responses on one connection, the second buried in the first's
/// body.
fn send(link: &dyn Transport, answer: &Answer) -> std::io::Result<()> {
    match answer {
        Answer::Sent(..) => Ok(()),
        // `trim_end` because the rendered response already ends with the
        // newline `write_line` is about to add, and Content-Length counts it.
        Answer::Compose(response) => link.write_line(response.render().trim_end()),
    }
}

fn bulk_transfer(link: &dyn Transport, opening: &[u8]) -> Answer {
    let mut buffer = opening.to_vec();
    let mut chunk = [0u8; 4096];

    // The head may not have arrived whole. Read until the blank line, with a
    // ceiling: a caller that sends headers for ever is one that would otherwise
    // grow this buffer for ever.
    let (request, body_at) = loop {
        match bulk::parse_head(&buffer) {
            Err(response) => return Answer::Compose(response),
            Ok(Some(found)) => break found,
            Ok(None) => {}
        }
        if buffer.len() > 64 * 1024 {
            return Answer::Compose(bulk::Response {
                status: 431,
                reason: "Request Header Fields Too Large",
                body: "the head did not end within 64 KiB
".to_string(),
            });
        }
        match link.read(&mut chunk) {
            Ok(0) | Err(_) => {
                return Answer::Compose(bulk::Response {
                    status: 400,
                    reason: "Bad Request",
                    body: "the connection closed before the request head was complete
"
                        .to_string(),
                });
            }
            Ok(got) => buffer.extend_from_slice(&chunk[..got]),
        }
    };

    // **An application's backup or restore** — `ADR-051` §8 and §12, `.rulings/`
    // R-234. Decided before git's handler, whose routes are suffixes a service
    // name could end in.
    if backups::is_backup(&request.target) {
        let already = buffer.get(body_at..).unwrap_or_default().to_vec();
        return backup_request(link, &request, &already);
    }

    // **Fleet-as-code's git operations** — `ADR-044`, a fourth handler decided
    // by the target. The body is small and read whole before git runs.
    if fleet_as_code::is_fleet_as_code(&request.target) {
        let already = buffer.get(body_at..).unwrap_or_default();
        return Answer::Compose(fleet_as_code_request(link, &request, already));
    }

    // **A fleet server's repository objects** — `.rulings/` R-184 D1, a third
    // handler on the same listener, decided by the target like the other two.
    if repository::is_repository_objects(&request.target) {
        let already = buffer.get(body_at..).unwrap_or_default().to_vec();
        let mut more = |into: &mut [u8]| link.read(into);
        // **Any pinned key in `peers` is the operator** (R-184 D2), and a
        // connection that presented none is nobody.
        let caller = link
            .peer_key()
            .map(|_| rivet_core::repository_objects::Caller::Operator);
        let machine = repository::Machine::on_this_machine();
        // **One object read back** (`ADR-063` §6) is binary and as large as a
        // system update, so it is written here in pieces, as git's answer is,
        // rather than through a `bulk::Response`'s `String` body.
        if let Some(found) = repository::object(&machine, &request, caller) {
            return match found {
                Ok(object) => send_object(link, object),
                Err(response) => Answer::Compose(response),
            };
        }
        let door = bulk::TrustStoreDoor::mirror_on_this_machine();
        let response = repository::serve(&machine, &door, &request, &already, &mut more, caller);
        if response.status == 422 {
            eprintln!(
                "rivet-mcpd: REFUSED AT THE DOOR of the repository store, not stored: {}",
                response
                    .body
                    .lines()
                    .find_map(|line| line.trim_start().strip_prefix("why"))
                    .map_or(response.body.as_str(), str::trim)
            );
        }
        return Answer::Compose(response);
    }

    // **Two handlers on one listener, and the target decides.** ADR-035 §3.2's
    // payload transfer and §4's git are both HTTP and are nothing else alike:
    // one is this daemon's own code writing status lines, the other is a CGI
    // whose `Status:` header has to be translated (crate::git's header).
    if git::is_git(&request.target) {
        let already = buffer.get(body_at..).unwrap_or_default().to_vec();
        let declared = request.length.unwrap_or(0);
        let remaining = declared.saturating_sub(already.len() as u64);
        let mut more = |into: &mut [u8]| link.read(into);
        // **Refused by name rather than mishandled.** A chunked body handed
        // on unread arrives at git with its length prefixes still in it, and
        // git would report a malformed pack — an error about the wrong thing,
        // which is the failure mode this whole section of PLAN.md is about.
        if request.chunked {
            return Answer::Compose(bulk::Response {
                status: 411,
                reason: "Length Required",
                body: "this build does not decode Transfer-Encoding: chunked, and will not \
                       hand a chunked body to git with its framing still in it: git \
                       would read the chunk sizes as pack data.\n\
                       \x20 note  git chunks a request body larger than http.postBuffer, which \
                       defaults to 1 MB. `cargo rivet fleet push` sends every body with a \
                       Content-Length whatever its size; a stock git client must raise \
                       http.postBuffer above the body instead.\n"
                    .to_string(),
            });
        }

        // The caller's pin, in the same `pinned-<hex>` form `ADR-028` §4.5
        // records for an install — one principal, one spelling, everywhere.
        let principal = link
            .peer_key()
            .and_then(|hex| rivet_core::deploy::Origin::pinned(&hex))
            .map(|origin| origin.as_token().to_string());
        return match git::serve(
            &request,
            &already,
            &mut more,
            remaining,
            principal.as_deref(),
        ) {
            Ok((status, reason, headers, body)) => {
                // **Written HERE rather than returned as a `bulk::Response`,**
                // and the difference is not tidiness. That type renders a
                // `String` body under `Content-Type: text/plain`; git answers
                // `application/x-git-upload-pack-result` and a PACK, which is
                // binary and is not text in any encoding. Putting one through
                // the other means `from_utf8_lossy` replacing every byte that
                // is not valid UTF-8 with `U+FFFD` — a clone that reports
                // success and leaves a corrupt repository.
                //
                // git's own headers are forwarded whole, `Content-Length`
                // included: it wrote one or it did not, and writing one here
                // would be this build describing a body it did not produce.
                let head = git::response_head(status, reason, &headers);
                if let Err(e) = link
                    .write_all(head.as_bytes())
                    .and_then(|()| link.write_all(&body))
                {
                    eprintln!("rivet-mcpd: git response could not be sent: {e}");
                }
                Answer::Sent(status, reason)
            }
            Err(response) => Answer::Compose(response),
        };
    }

    let (name, expected, length) = match bulk::accept(&request) {
        Ok(accepted) => accepted,
        Err(response) => return Answer::Compose(response),
    };

    let already = buffer.get(body_at..).unwrap_or_default().to_vec();
    let mut more = |into: &mut [u8]| link.read(into);
    // **The composition root names the store**, which is the one place the real
    // `/state/artifacts` appears in this binary — see
    // `rivet_core::artifact::Store` for why it is a parameter everywhere below.
    let store = rivet_core::artifact::Store::on_this_machine();
    // **And the door**, which reads `/system/trust` on every admission — see
    // `bulk::TrustStoreDoor` for why this daemon can and why nothing is cached.
    let door = bulk::TrustStoreDoor::on_this_machine();
    match bulk::receive(&store, &door, &name, expected, length, &already, &mut more) {
        Ok(response) => Answer::Compose(response),
        Err(response) => {
            // **A refusal at the door is written down here**, because the
            // status line the caller logs says only `422` — and "why did this
            // machine refuse my artifact" should be answerable from the
            // machine's own journal, not only from the client that was told.
            if response.status == 422 {
                eprintln!(
                    "rivet-mcpd: REFUSED AT THE DOOR, not stored: {}",
                    response
                        .body
                        .lines()
                        .find_map(|line| line.trim_start().strip_prefix("why"))
                        .map_or(response.body.as_str(), str::trim)
                );
            }
            Answer::Compose(response)
        }
    }
}

/// **One backup or restore, relayed** — [`backups::plan`], then
/// [`backups::relay`] with this connection on both sides.
///
/// A refusal is written here with [`backups::render_refusal`], so it carries
/// the header that tells it from an application's answer; a relayed answer has
/// already been written piece by piece by the time this returns.
fn backup_request(link: &dyn Transport, request: &bulk::Request, already: &[u8]) -> Answer {
    let refused = |response: &bulk::Response| -> Answer {
        if let Err(e) = link.write_all(backups::render_refusal(response).as_bytes()) {
            eprintln!("rivet-mcpd: a backup refusal could not be sent: {e}");
        }
        Answer::Sent(response.status, response.reason)
    };
    let machine = backups::Machine::on_this_machine();
    let upstream = match backups::plan(&machine, request, link.peer_key().is_some()) {
        Ok(upstream) => upstream,
        Err(response) => {
            eprintln!(
                "rivet-mcpd: {} {} refused: {}",
                request.method,
                request.target,
                response.body.lines().next().unwrap_or_default()
            );
            return refused(&response);
        }
    };
    let mut more = |into: &mut [u8]| link.read(into);
    let mut out = |bytes: &[u8]| link.write_all(bytes);
    match backups::relay(&upstream, already, &mut more, &mut out) {
        Ok(status) => {
            eprintln!(
                "rivet-mcpd: {:?} of {} relayed; the application answered {status}",
                upstream.requested, upstream.service
            );
            Answer::Sent(status, "relayed from the application")
        }
        Err(response) => {
            eprintln!(
                "rivet-mcpd: {:?} of {} failed: {}",
                upstream.requested,
                upstream.service,
                response.body.lines().next().unwrap_or_default()
            );
            refused(&response)
        }
    }
}

/// **One repository object, streamed** — [`repository::object_head`], then the
/// file in 64 KiB pieces. A failure part-way is logged; the caller's length and
/// digest checks are what notice a short body.
fn send_object(link: &dyn Transport, mut object: repository::Object) -> Answer {
    use std::io::Read as _;
    let head = repository::object_head(&object);
    if let Err(e) = link.write_all(head.as_bytes()) {
        eprintln!("rivet-mcpd: repository object {} could not be sent: {e}", object.digest);
        return Answer::Sent(200, "OK");
    }
    let mut buffer = vec![0u8; 64 * 1024];
    let mut sent = 0u64;
    loop {
        match object.file.read(&mut buffer) {
            Ok(0) => break,
            Ok(got) => {
                if let Err(e) = link.write_all(&buffer[..got]) {
                    eprintln!(
                        "rivet-mcpd: repository object {} stopped after {sent} of {} byte(s): {e}",
                        object.digest, object.length
                    );
                    return Answer::Sent(200, "OK");
                }
                sent += got as u64;
            }
            Err(e) => {
                eprintln!(
                    "rivet-mcpd: repository object {} could not be read after {sent} byte(s): {e}",
                    object.digest
                );
                return Answer::Sent(200, "OK");
            }
        }
    }
    Answer::Sent(200, "OK")
}

/// **One fleet-as-code request**: its whole body read, its caller named by the
/// pinned key the handshake saw, and [`fleet_as_code::serve`]'s answer.
///
/// Refused before a byte of body is read when the body is chunked or declares
/// more than `rivet_core::fleet_as_code::MAX_REQUEST_BYTES`.
fn fleet_as_code_request(link: &dyn Transport, request: &bulk::Request, already: &[u8]) -> bulk::Response {
    use rivet_core::fleet_as_code::{Caller, Kind, MAX_REQUEST_BYTES, Refusal};
    let refuse = |refusal: Refusal| {
        let (status, reason) = refusal.kind.status();
        bulk::Response { status, reason, body: refusal.render() }
    };
    if request.chunked {
        return refuse(Refusal::new(
            Kind::LengthRequired,
            "a fleet-as-code request carries a Content-Length; this build does not decode a chunked body",
        ));
    }
    let length = request.length.unwrap_or(0);
    let Ok(length) = usize::try_from(length) else {
        return refuse(Refusal::new(Kind::TooLarge, format!("a body of {length} bytes is past every limit")));
    };
    if length > MAX_REQUEST_BYTES {
        return refuse(Refusal::new(
            Kind::TooLarge,
            format!("a body of {length} bytes is past the {MAX_REQUEST_BYTES}-byte limit"),
        ));
    }
    let mut body = already.get(..length.min(already.len())).unwrap_or_default().to_vec();
    let mut chunk = [0u8; 4096];
    while body.len() < length {
        match link.read(&mut chunk) {
            Ok(0) | Err(_) => {
                return refuse(Refusal::new(
                    Kind::BadRequest,
                    format!("the request declared {length} bytes of body and the connection closed after {}", body.len()),
                ));
            }
            Ok(got) => body.extend_from_slice(chunk.get(..got.min(length - body.len())).unwrap_or_default()),
        }
    }

    let caller = link.peer_key().map(|_| Caller::Operator);
    let machine = fleet_as_code::Machine::on_this_machine();
    let response = fleet_as_code::serve(&machine, &request.method, &request.target, &body, caller);
    let first = response.body.lines().next().unwrap_or_default();
    eprintln!(
        "rivet-mcpd: fleet-as-code: {} {} over {}: {first}",
        request.method,
        request.target,
        link.describe()
    );
    response
}

/// Give one connection the CLI, telling it who is on the other end.
///
/// **`peer` is the pinned Ed25519 key, or `None` on the plaintext loopback
/// transport where there is nobody to name.** `ADR-028` §4.5 has the machine
/// record who asked for an install, and until 2026-08-28 it read a variable the
/// old transport exported — which this daemon INHERITS from whatever started it
/// and hands to every child. So a deploy over TLS was attributed to whichever
/// session started the daemon (`PLAN.md` §81.3). This is the fix, and it is one
/// environment variable because that is how the CLI already learns the other
/// two.
fn spawn(peer: Option<&str>) -> std::io::Result<Child> {
    let mut command = Command::new(CLI);
    command
        .arg(SUBCOMMAND)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // **stderr is inherited on purpose.** `cmd/rivet`'s mcp module puts
        // everything that is not a JSON-RPC message there, and this daemon's
        // own stderr is the supervisor's capture (ADR-022 §4). Piping it would
        // mean this process deciding what to do with a diagnostic, which is a
        // decision it has no basis for.
        .stderr(Stdio::inherit());
    match peer {
        Some(key) => command.env(rivet_core::deploy::PEER_KEY_ENV, key),
        // **Removed rather than left**, so a daemon started from some other
        // session cannot leak that session's variable into a plaintext connection's
        // child and have it read as a control-plane peer.
        None => command.env_remove(rivet_core::deploy::PEER_KEY_ENV),
    };
    command.spawn()
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match Args::parse(&argv) {
        Ok(args) => args,
        Err(why) => {
            eprintln!("rivet-mcpd: {why}");
            std::process::exit(2);
        }
    };

    // **Built before the socket is bound**, so a bad pin file is a refusal to
    // start rather than a surprise for the first caller.
    let tls = match (&args.identity, &args.peers) {
        (Some(identity), Some(peers)) => match tls_config(identity, peers) {
            Ok(config) => Some(config),
            Err(why) => {
                eprintln!("rivet-mcpd: {why}");
                std::process::exit(2);
            }
        },
        _ => None,
    };

    let listener = match TcpListener::bind(args.listen) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("rivet-mcpd: cannot bind {}: {e}", args.listen);
            std::process::exit(1);
        }
    };

    eprintln!(
        "rivet-mcpd: listening on {} ({})",
        args.listen,
        if args.from_document {
            "from the document's [control-plane] table"
        } else {
            "the default — no [control-plane] table said otherwise"
        }
    );
    eprintln!("rivet-mcpd: each connection gets the stdin and stdout of {CLI} {SUBCOMMAND}");
    if tls.is_some() {
        eprintln!(
            "rivet-mcpd: MUTUAL TLS — ADR-035 §7.2 over ADR-025 §2. Both ends are pinned by\n\
             \x20           Ed25519 public key; no chain is walked and CERTIFICATE EXPIRY IS\n\
             \x20           IGNORED (ADR-035 §7.1, and rivetos-tls says so in as many words)."
        );
    } else {
        eprintln!(
            "rivet-mcpd: NO TLS AND NO IDENTITY. Loopback only, and every operation in the\n\
             \x20           catalogue is reachable by anything that can reach this port.\n\
             \x20           Pass --identity and --peers for a control plane that knows who is\n\
             \x20           calling."
        );
    }

    eprintln!(
        "rivet-mcpd: at most {} connections at once, {} per source address ({} always kept for \
         other sources); {}s to finish a handshake, {}s idle after one",
        govern::MAX_CONNECTIONS,
        govern::PER_SOURCE_SLOTS,
        govern::RESERVED,
        BEFORE_AUTHENTICATED.as_secs(),
        AFTER_AUTHENTICATED.as_secs()
    );

    let governor = govern::governor();

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let peer = stream
                    .peer_addr()
                    .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));

                // **Weighed HERE, before a thread exists and before a line is
                // logged.** Accounting a connection after the thread is spawned
                // means the stack is already allocated by the time the daemon
                // decides it did not want it, and logging every refusal is what
                // let a flood churn the journal (see `govern`). A
                // refusal collapses to one line per source per window.
                let slot = match governor.arrive(peer.ip(), govern::now_ms()) {
                    govern::Outcome::Admit(slot) => slot,
                    govern::Outcome::Refuse { reason, report } => {
                        if let Some(dropped) = report {
                            eprintln!(
                                "rivet-mcpd: {} connection(s) from {} refused: {} — at most one \
                                 line per {}s. A control plane that accepted every caller could be \
                                 locked shut by anyone able to reach the port, and a headless board \
                                 has no other way in",
                                dropped,
                                peer.ip(),
                                reason.describe(),
                                govern::REPORT_EVERY_MS / 1000,
                            );
                        }
                        // Dropped, which closes it.
                        continue;
                    }
                };

                // **The deadline goes on before the handshake, not after.** An
                // unauthenticated peer holds this socket for as long as the
                // handshake takes, and `Tls::accept` drives that handshake to
                // completion against a blocking socket — so with no deadline a
                // peer that connects and says nothing owns the thread for ever.
                if let Err(e) = stream
                    .set_read_timeout(Some(BEFORE_AUTHENTICATED))
                    .and_then(|()| stream.set_write_timeout(Some(BEFORE_AUTHENTICATED)))
                {
                    eprintln!("rivet-mcpd: {peer}: refused: no deadline could be set: {e}");
                    continue;
                }

                let config = tls.as_ref().map(|(config, _)| Arc::clone(config));
                let live = tls.as_ref().map(|(_, live)| Arc::clone(live));
                // Into the thread, so a handshake failure there collapses per
                // source instead of logging a line each — see `govern`.
                let gov = governor.clone();
                // One thread per connection, and a failure on one is that
                // connection's failure. A daemon that exited because a caller
                // hung up mid-message would be a control plane a client could
                // stop by disconnecting rudely — and with TLS that now
                // includes a caller whose key is not pinned, which is a thing
                // that WILL happen and must cost one connection and no more.
                std::thread::spawn(move || {
                    // Held for the length of the connection and released by its
                    // own `Drop`, so every exit from this closure — the early
                    // returns, a panic, the ordinary end — gives the slot back.
                    let _slot = slot;
                    let link: Arc<dyn Transport> = match config {
                        Some(config) => match Tls::accept(stream, config) {
                            Ok(tls) => {
                                // **This machine has been reached** — see
                                // `mark_reached`. After the handshake, so it
                                // records a caller that PROVED a pinned key
                                // rather than one that opened a socket.
                                mark_reached(tls.peer_key().is_some());
                                Arc::new(tls)
                            }
                            Err(e) => {
                                // Collapsed per source: the first failure from a
                                // source logs at once, then at most one line per
                                // window. Only an unauthenticated caller reaches
                                // here, so this is where a flood would otherwise
                                // roll the journal.
                                if let Some(n) =
                                    gov.note_handshake_failure(peer.ip(), govern::now_ms())
                                {
                                    eprintln!(
                                        "rivet-mcpd: {} handshake failure(s) from {} — latest: \
                                         {} (at most one line per {}s)",
                                        n,
                                        peer.ip(),
                                        e,
                                        govern::REPORT_EVERY_MS / 1000,
                                    );
                                }
                                return;
                            }
                        },
                        None => Arc::new(Plain::new(stream)),
                    };
                    // **Relaxed now, and only now.** Everything past this line
                    // is a caller that proved a pinned key (`ADR-025` §2) or is
                    // on loopback with no routable path to here, so the short
                    // deadline has done its job and a working session may be
                    // quiet for a while between requests.
                    if let Err(e) = link.set_deadline(Some(AFTER_AUTHENTICATED)) {
                        eprintln!("rivet-mcpd: {peer}: no idle deadline could be set: {e}");
                        return;
                    }
                    eprintln!("rivet-mcpd: {peer}: {}", link.describe());
                    if let Err(e) = serve(link, peer, live) {
                        eprintln!("rivet-mcpd: {peer}: {e}");
                    }
                });
            }
            Err(e) => eprintln!("rivet-mcpd: a connection could not be accepted: {e}"),
        }
    }
}

/// The longest one JSON-RPC request line may be.
///
/// Four megabytes is several hundred times the largest request the catalogue
/// produces and is not a working limit — it is the point past which a caller is
/// no longer sending a line at all.
const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

/// How long an unauthenticated caller has to finish its TLS handshake.
///
/// **This is the deadline that matters.** Before the handshake completes, the
/// peer is nobody: anyone who can reach the port can open a socket, and without
/// a deadline each one holds a slot and a thread until the machine reboots.
/// `govern::MAX_CONNECTIONS` bounds how many; the per-source caps bound how many
/// one address may take; this bounds how long. **Three seconds** is several
/// times a mutual-TLS handshake across a home LAN, and short enough that a peer
/// which opens sockets and says nothing gives each slot back quickly.
const BEFORE_AUTHENTICATED: Duration = Duration::from_secs(3);

/// How long an authenticated session may sit idle.
///
/// **Generous on purpose, and safe to be.** Past the handshake the peer holds a
/// pinned key, which `ADR-025` §2 makes the whole of what this machine trusts
/// about a caller. An agent thinking between two MCP calls, or an operator
/// reading output, is an ordinary quiet minute and not an attack — so this is
/// long enough not to interrupt one, while still ensuring a session that has
/// gone away releases its slot without a reboot.
const AFTER_AUTHENTICATED: Duration = Duration::from_secs(900);

/// **Record that something reached this machine on this boot.**
///
/// `PLAN.md` §104.20. `cmd/rivet-initd` will not confirm a trial boot until
/// this file exists, so an update that leaves a machine unreachable reverts on
/// the next reset instead of committing itself. The gate that used to decide
/// it asked three questions — mounts, provisioning, supervisor liveness — and
/// none of them was *can anybody talk to this machine*.
///
/// # Only a PROVEN caller counts
///
/// `authenticated` is `false` for a connection with no client certificate, and
/// nothing is written for one. A boot confirmed by an unauthenticated TCP
/// connection would be a boot confirmed by a port scan, and `ADR-025` §2's pin
/// is the whole of what this machine trusts about a caller.
///
/// # Every failure is dropped, deliberately
///
/// `/run` is a tmpfs this process may not be able to write if pid 1's mount
/// plan degraded, and a control plane that refused to serve a request because
/// it could not write a marker would have turned a diagnostic into an outage.
/// The cost of a silent failure is one trial boot that is not confirmed —
/// which is the SAFE direction, and which pid 1 says out loud on the console.
fn mark_reached(authenticated: bool) {
    if !authenticated {
        return;
    }
    let path = rivet_core::operation::status::REACHED_MARKER_PATH;
    if let Some(directory) = path.rsplit_once('/').map(|(d, _)| d) {
        let _ = std::fs::create_dir_all(directory);
    }
    let _ = std::fs::write(path, b"");
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    // The connection caps — global, per-source, and release — are tested in
    // `crate::govern`, where the state and the policy live.

    /// The deadlines are ordered the way the argument at them says: short while
    /// the peer is nobody, long once it has proved a pinned key.
    #[test]
    fn an_unauthenticated_caller_gets_the_shorter_deadline() {
        assert!(
            BEFORE_AUTHENTICATED < AFTER_AUTHENTICATED,
            "an unauthenticated peer must not be given the more patient deadline"
        );
        assert!(BEFORE_AUTHENTICATED.as_secs() > 0, "a zero deadline refuses every caller");
    }

    /// **Three seconds before authentication**: long enough for a mutual-TLS
    /// handshake over the LAN, short enough that a peer which opens a socket and
    /// says nothing gives its slot back quickly.
    #[test]
    fn the_deadline_before_authentication_is_three_seconds() {
        assert_eq!(BEFORE_AUTHENTICATED, Duration::from_secs(3));
    }

    /// **An `Answer::Sent` writes nothing**, and this is the test standing in
    /// for a defect that no type check could see: `crate::git` writes its own
    /// bytes, the caller wrote the returned `Response` as well, and the client
    /// got a second complete HTTP response buried inside the first's body.
    /// `git` reported `bad line length character: HTTP` — true, useless, and a
    /// long way from the cause (`PLAN.md` §88.7).
    #[test]
    fn an_answer_already_on_the_wire_is_not_written_twice() {
        let link = Recorder::new();
        send(&link, &Answer::Sent(200, "OK")).expect("writing nothing cannot fail");
        assert!(
            link.written().is_empty(),
            "a response already on the wire was written again: {:?}",
            link.written()
        );

        // ...and the other arm does write, so the test above is not passing
        // because `send` writes nothing at all.
        let link = Recorder::new();
        send(
            &link,
            &Answer::Compose(bulk::Response {
                status: 404,
                reason: "Not Found",
                body: "no\n".to_string(),
            }),
        )
        .expect("a composed response is written");
        let written = link.written();
        assert_eq!(written.matches("HTTP/1.1").count(), 1, "{written:?}");
        assert!(written.starts_with("HTTP/1.1 404 Not Found"), "{written:?}");
    }

    /// A transport that keeps what was written instead of sending it.
    struct Recorder(std::sync::Mutex<Vec<u8>>);

    impl Recorder {
        fn new() -> Self {
            Self(std::sync::Mutex::new(Vec::new()))
        }
        fn written(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().expect("a lock")).into_owned()
        }
    }

    impl crate::transport::Transport for Recorder {
        fn read(&self, _: &mut [u8]) -> std::io::Result<usize> {
            Ok(0)
        }
        fn write_line(&self, line: &str) -> std::io::Result<()> {
            self.0.lock().expect("a lock").extend_from_slice(line.as_bytes());
            Ok(())
        }
        fn write_all(&self, bytes: &[u8]) -> std::io::Result<()> {
            self.0.lock().expect("a lock").extend_from_slice(bytes);
            Ok(())
        }
        fn describe(&self) -> String {
            "a test".to_string()
        }
        fn peer_key(&self) -> Option<String> {
            None
        }
        /// No socket, so no deadline to set — see the same method on
        /// `transport::tests::Recorded`.
        fn set_deadline(&self, _: Option<Duration>) -> std::io::Result<()> {
            Ok(())
        }
        fn finish(&self) {}
    }

    /// **The routing, driven end to end — and this is the test §86 did not
    /// have.** Every git test asserted `is_git`, `translate` and `environment`
    /// in isolation; all three were correct, and the board still answered
    /// `crate::bulk`'s 405 to a real `git clone` (`PLAN.md` §88). A unit that is
    /// right proves nothing about whether anything calls it.
    ///
    /// So this drives `bulk_transfer` with the bytes a real `git` sends —
    /// captured off the wire, not composed from the routing's own idea of the
    /// shape — and asserts only that **`bulk` did not answer**. What git's own
    /// program then says is git's business and needs a board.
    #[test]
    fn a_real_git_request_does_not_reach_the_payload_endpoint() {
        // Verbatim from `git/2.54.0` cloning over http — captured off a socket
        // that printed what arrived, rather than composed from the routing's
        // own idea of the shape (PLAN.md §88.5).
        let opening = b"GET /example-repo.git/info/refs?service=git-upload-pack HTTP/1.1\r\n\
                        Host: 10.0.0.2:7777\r\nUser-Agent: git/2.54.0\r\n\
                        Accept: */*\r\nGit-Protocol: version=2\r\n\r\n";

        let link = Recorder::new();
        let answer = bulk_transfer(&link, opening);

        // On a machine with no `git-http-backend` — every development host —
        // this is `crate::git` failing to run it, which is still `crate::git`.
        // The assertion is about WHO answered, not about what git said.
        if let Answer::Compose(response) = &answer {
            assert!(
                !response.body.contains("this endpoint takes PUT"),
                "the payload endpoint answered a git request, so the routing was not \
                 reached: {} {}",
                response.status,
                response.body
            );
            assert!(
                !response.body.contains("/payload/<file-name>"),
                "the payload endpoint answered a git request: {}",
                response.body
            );
        }
    }

    fn parse(words: &[&str]) -> Result<Args, String> {
        Args::parse(&words.iter().map(|w| (*w).to_string()).collect::<Vec<_>>())
    }

    /// **Nothing is required any more, and the default is the constant that
    /// was deleted.** `MCPD_LISTEN` lived in `cmd/rivet-sysd` until TLS made a
    /// routable bind legal; the address is now `[control-plane]`'s and this is
    /// the fallback for a machine whose document does not say.
    #[test]
    fn with_nothing_given_it_binds_the_loopback_default() {
        let args = parse(&[]).expect("no flags is a complete request now");
        assert_eq!(args.listen.to_string(), rivet_config::control_plane::DEFAULT_LISTEN);
        assert!(!args.from_document, "nothing was read, so nothing should claim it was");
    }

    /// **`--listen` wins over the document, and that is stated rather than
    /// discovered.** `ADR-011` §3's failure is two things that both look
    /// authoritative; a flag that silently lost to a file would be exactly
    /// that.
    #[test]
    fn an_explicit_address_beats_the_document_and_says_it_did_not_read_one() {
        let args = parse(&[
            "--listen", "0.0.0.0:9999", "--document", "/does/not/exist",
            "--identity", "/i", "--peers", "/p",
        ])
        .expect("an explicit address needs no document");
        assert_eq!(args.listen.to_string(), "0.0.0.0:9999");
        assert!(
            !args.from_document,
            "the document was not consulted and the startup line must not imply it was"
        );
    }

    /// A document that is ABSENT is not a refusal — that is a machine nobody
    /// has configured, and `ADR-007` §7.1 will not have this program write one.
    #[test]
    fn a_document_that_is_not_there_falls_back_rather_than_refusing() {
        let args = parse(&["--document", "/definitely/not/here.toml"])
            .expect("an absent document is the ordinary case");
        assert_eq!(args.listen.to_string(), rivet_config::control_plane::DEFAULT_LISTEN);
    }

    /// A document that is THERE and wrong is a refusal, because that is an
    /// operator who said something this build could not act on.
    #[test]
    fn a_document_that_will_not_parse_is_refused_and_names_itself() {
        let path = std::env::temp_dir().join(format!("rivet-mcpd-{}.toml", std::process::id()));
        std::fs::write(&path, "this is not toml = = =
").expect("a scratch file");
        let refused = parse(&["--document", &path.display().to_string()])
            .expect_err("a malformed document must be refused");
        assert!(refused.contains("not a document this build can read"), "{refused}");
        std::fs::remove_file(&path).expect("cleanup");
    }

    /// **The refusal names where the address came from.** An operator told
    /// `--listen 0.0.0.0:7788 is not a loopback address` when they never typed
    /// `--listen` would go looking for a flag nobody passed.
    #[test]
    fn a_routable_address_from_the_document_is_refused_naming_the_document() {
        let path = std::env::temp_dir().join(format!("rivet-mcpd-r-{}.toml", std::process::id()));
        std::fs::write(&path, "version = 1
[control-plane]
listen = \"0.0.0.0:7788\"
")
            .expect("a scratch file");
        let refused = parse(&["--document", &path.display().to_string()])
            .expect_err("routable with no identity must be refused");
        assert!(refused.contains("the document's [control-plane] listen"), "{refused}");
        assert!(!refused.starts_with("--listen"), "it blamed a flag nobody passed: {refused}");
        std::fs::remove_file(&path).expect("cleanup");
    }

    /// **The board caught this one.** `from_document` used to mean *a document
    /// path was given*, so the daemon announced it had read a `[control-plane]`
    /// table against a document that had none. A document with no table gets
    /// the default and must SAY it got the default.
    #[test]
    fn a_document_with_no_control_plane_table_does_not_claim_to_have_been_read() {
        let path = std::env::temp_dir().join(format!("rivet-mcpd-n-{}.toml", std::process::id()));
        std::fs::write(&path, "version = 1
[services.rivet-mcpd]
enabled = true
")
            .expect("a scratch file");
        let args = parse(&["--document", &path.display().to_string()])
            .expect("a document with no table is fine");
        assert_eq!(args.listen.to_string(), rivet_config::control_plane::DEFAULT_LISTEN);
        assert!(
            !args.from_document,
            "it claimed the document said something the document does not say"
        );
        std::fs::remove_file(&path).expect("cleanup");
    }

    /// And a document that says where to listen is obeyed.
    #[test]
    fn the_documents_address_is_the_one_it_binds() {
        let path = std::env::temp_dir().join(format!("rivet-mcpd-ok-{}.toml", std::process::id()));
        std::fs::write(&path, "version = 1
[control-plane]
listen = \"0.0.0.0:7788\"
")
            .expect("a scratch file");
        let args = parse(&[
            "--document", &path.display().to_string(), "--identity", "/i", "--peers", "/p",
        ])
        .expect("a document naming an address");
        assert_eq!(args.listen.to_string(), "0.0.0.0:7788");
        assert!(args.from_document, "it read the document and should say so");
        std::fs::remove_file(&path).expect("cleanup");
    }

    /// **The whole of the loopback mode's security, as a test.** Without an
    /// identity a non-loopback bind is a catalogue — including `apply
    /// power-off` — exposed to anything that can route to the port.
    ///
    /// Amended 2026-08-28: this used to say *"until `ADR-035` §7.2's
    /// certificate exists"*. It exists now, so the condition is no longer
    /// whether TLS is BUILT but whether this daemon was HANDED an identity.
    #[test]
    fn a_non_loopback_address_is_refused_when_no_identity_was_given() {
        for addr in ["0.0.0.0:7777", "10.0.0.2:7777", "[::]:7777"] {
            let err = parse(&["--listen", addr]).expect_err("must refuse");
            assert!(err.contains("has no TLS") || err.contains("no TLS"), "{err}");
            assert!(err.contains("apply power-off"), "{err}");
        }
    }

    /// And there is no flag that turns it off — asserted so that adding one has
    /// to be a deliberate act rather than a convenience someone reaches for.
    #[test]
    fn there_is_no_option_that_permits_an_insecure_bind() {
        for attempt in ["--insecure", "--no-tls", "--allow-any-address", "--force"] {
            let err = parse(&["--listen", "0.0.0.0:7777", attempt]).expect_err("must refuse");
            assert!(err.contains("is not an option"), "{err}");
        }
    }

    #[test]
    fn every_loopback_address_is_accepted_and_not_just_the_usual_one() {
        for addr in ["127.0.0.1:7777", "127.0.0.2:7777", "[::1]:7777"] {
            assert!(parse(&["--listen", addr]).is_ok(), "{addr} should be accepted");
        }
    }

    #[test]
    fn a_malformed_address_is_refused_by_name() {
        let err = parse(&["--listen", "not-an-address"]).expect_err("must refuse");
        assert!(err.contains("not-an-address"), "{err}");
    }

    /// **Half a trust model is refused, in both directions.** An identity with
    /// no peers accepts everybody while proving who it is; peers with no
    /// identity cannot start a handshake at all. Silently degrading either to
    /// the loopback mode would be the worst outcome available: a machine that
    /// was MEANT to be authenticated and quietly is not.
    #[test]
    fn an_identity_without_peers_and_peers_without_an_identity_are_both_refused() {
        let one = parse(&["--listen", "127.0.0.1:7777", "--identity", "/x"])
            .expect_err("half a trust model");
        assert!(one.contains("accept ANY of them"), "{one}");
        assert!(one.contains("ADR-025 §2"), "{one}");

        let other =
            parse(&["--listen", "127.0.0.1:7777", "--peers", "/y"]).expect_err("the other half");
        assert!(other.contains("none of its own to offer"), "{other}");
    }

    /// **With both, a routable address becomes legal** — which is the whole
    /// point of building TLS, and the thing `ADR-035` phase 2 needs.
    #[test]
    fn an_identity_and_peers_together_permit_any_address() {
        for addr in ["0.0.0.0:7777", "10.0.0.2:7777", "[::]:7777"] {
            let args = parse(&["--listen", addr, "--identity", "/i", "--peers", "/p"])
                .unwrap_or_else(|e| panic!("{addr} should be accepted with an identity: {e}"));
            assert_eq!(args.identity, Some(PathBuf::from("/i")));
            assert_eq!(args.peers, Some(PathBuf::from("/p")));
        }
    }

    /// And loopback still works with an identity — TLS is not a reason to stop
    /// supporting the tunnel that phase 1 was proved over.
    #[test]
    fn loopback_is_still_accepted_when_an_identity_is_given() {
        assert!(
            parse(&["--listen", "127.0.0.1:7777", "--identity", "/i", "--peers", "/p"]).is_ok()
        );
    }

    /// The usage names both files by the constants `rivetos-tls` publishes, so
    /// what the operator is told to create is what the daemon reads.
    #[test]
    fn the_usage_names_the_files_rivetos_tls_actually_looks_for() {
        let usage = usage();
        assert!(usage.contains(rivetos_tls::SEED_FILE), "{usage}");
        assert!(usage.contains(rivetos_tls::CERTIFICATE_FILE), "{usage}");
    }

    /// **`ADR-011`, as a test.** The program this spawns is `rivet-core`'s
    /// constant and not a literal, so an image that moves the CLI moves this
    /// with it.
    #[test]
    fn the_program_spawned_is_the_one_rivet_core_names() {
        assert_eq!(CLI, rivet_core::etc::CLI_PATH);
        assert!(CLI.starts_with('/'), "{CLI}");
        assert_eq!(SUBCOMMAND, "mcp");
    }

    /// **Agreement with this crate's own pin parser.**
    /// `rivet_core::operation::restore_secret`'s `restore-secret peers`
    /// grammar check is a deliberate SUBSET of `parse_pins`, the reader that
    /// decides whether this daemon starts at all. So: every text the core
    /// validator accepts must parse here to the same keys in the same order,
    /// and every malformed text this crate refuses, the core validator must
    /// refuse too — the reverse is not claimed, because the core is
    /// deliberately stricter (it refuses a `+`-signed pair
    /// `u8::from_str_radix` happens to accept).
    #[test]
    fn the_core_peers_grammar_check_agrees_with_this_crates_pin_parser() {
        let key_a = "ab".repeat(rivetos_tls::KEY_BYTES);
        let key_b = "CD".repeat(rivetos_tls::KEY_BYTES);

        let accepted = [
            String::new(),
            "\n\n".to_string(),
            "# nothing but a comment\n".to_string(),
            format!("{key_a}\n"),
            format!("{key_b}\n"),
            format!("# fleet keys\n\n{key_a}  # a teammate\n\n{key_b}\n# trailing\n"),
            format!("  {key_a}  \n"),
        ];
        for text in &accepted {
            let core_keys = rivet_core::operation::restore_secret::parse_peers_grammar(text.as_bytes())
                .unwrap_or_else(|e| panic!("the core refused a text this corpus expects it to accept {text:?}: {e}"));
            let core_reparsed = rivetos_tls::parse_pins(&core_keys.join("\n")).unwrap_or_else(|e| {
                panic!("the core's own accepted keys do not parse as pins {core_keys:?}: {e}")
            });
            let direct = rivetos_tls::parse_pins(text)
                .unwrap_or_else(|e| panic!("parse_pins refused a text the core accepted {text:?}: {e}"));
            assert_eq!(core_reparsed, direct, "{text:?}: different keys, or a different order");
        }

        let refused = [
            "not-a-key\n".to_string(),
            "a".repeat(62),
            "a".repeat(66),
            format!("{key_a} extra-token"),
            "00 ff\n".to_string(),
        ];
        for text in &refused {
            assert!(rivetos_tls::parse_pins(text).is_err(), "{text:?}: parse_pins accepted this");
            assert!(
                rivet_core::operation::restore_secret::parse_peers_grammar(text.as_bytes()).is_err(),
                "{text:?}: the core accepted a text parse_pins refuses"
            );
        }
    }
}
