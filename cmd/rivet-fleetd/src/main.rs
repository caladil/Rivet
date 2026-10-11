//! **`rivet-fleetd`** — HTTPS for people, MCP to machines.
//!
//! `ADR-034` §2.2's Remote mode, finally built, and built as an **ordinary
//! application**: a signed `.rvt` that asks for a listening socket and an
//! outbound network the way any other application does.
//!
//! # What this first slice proves
//!
//! Three things that had never been done and that everything above them
//! depends on:
//!
//! 1. **An application can hold a privileged port.** `listen = ["tcp:4443"]`
//!    — `tcp:443` until 2026-09-05, when `rivet-proxyd` took the machine's
//!    front door (`R-152.3`, `ADR-046` §7) —
//!    bound by the supervisor as root and handed over as a descriptor
//!    (`ADR-015` §5.2). This program never holds `CAP_NET_BIND_SERVICE`.
//! 2. **An application can terminate TLS**, with a certificate chaining to the
//!    fleet's authority rather than to a pin.
//! 3. **A browser can be the thing that authenticates.** `ADR-025` §2 makes a
//!    pinned Ed25519 key the whole trust model for a machine, and **a browser
//!    cannot participate in that** — it will not pin, it only trusts chains.
//!    So the operator presents a client certificate minted by the same
//!    authority, which is the same *shape* of trust: something minted
//!    deliberately, held by one party, checkable without a third.
//!
//! # What it does NOT do yet
//!
//! Speak MCP, proxy to nodes, or serve an application. Those are more code.
//! The three above were unknowns.
//!
//! # Where its certificate comes from
//!
//! `rivet-provision` mints it on the workstation when a machine is provisioned
//! as a fleet server, because that is the first moment its hostname exists —
//! and a browser validates the name it asked for. The operator places the
//! result in this application's own data directory. **It is per-machine
//! material and is deliberately not in the payload**, which would put one
//! machine's identity in every copy.

// **Fleet as code** — `ADR-044`, `.rulings/` R-145. The fleet's desired-state
// documents live in a git repository and a push is what changes the fleet;
// these three modules are the repository, the document transport, and the pass
// that joins them. The DECISION is `rivet_core::converge`, on purpose.
mod code;
// **R39.** Where `POST /api/commit/<device>` lands — the credential guard,
// and the call into `repo::Repository::commit_file`. Its own module
// documentation argues why this is a genuine new route.
mod commit;
mod document;
mod fleet;
// **Test-only, and deliberately.** `formfields` is the ordering/condition/
// `mode`-`[kiosk]` algorithm `web/config.js` implements in the browser,
// kept here only so `cargo test` can prove it against the real schema
// (`docs/PLAN-Feature-Set-2026-09-12.md` §5.3, `.rulings/` R-162) — see its
// own module documentation for why this program never calls it at run time.
// `cfg(test)` rather than a normal module: nothing in the shipped binary
// calls a walker whose whole job is to run where `cargo test` can hold it,
// and a `pub fn` nobody calls in a binary crate is dead code, not API.
#[cfg(test)]
mod formfields;
// **`SPEC-06`: the socket `gantry` reads the store and the phones through.**
// The protocol is testable without a socket; the socket edge is Linux's.
mod gantry;
// **Who the 4443 handshake admits**, over a real TLS connection to
// `serve_one`, with certificates minted by provisioning's own code.
#[cfg(test)]
#[path = "gate_tests.rs"]
mod gate_tests;
// **`cmd/rivet-provision`'s authority module, compiled into this test build**
// so the certificates `gate_tests` presents are minted by the functions that
// mint real ones, not by a copy of them. `cmd/cargo-rivet` takes
// `rivet-sdk/src/qoi.rs` the same way; neither crate has a library target.
#[cfg(test)]
#[path = "../../rivet-provision/src/ca.rs"]
mod provision_ca;
mod node;
mod page;
mod repo;
// **`ADR-056` §3.** The one page in this server that reaches no machine: it
// reads `/proc` in this process, because `rivet-mcpd` is the first thing to go
// silent and every other page here goes through it.
mod supervisor;
// **R39.** The round-trip reader/writer `web/config.js`'s Save composes a
// document with, and the credential-name guard `commit` calls before it
// ever reaches `repo::Repository::commit_file`. NOT `cfg(test)`, unlike
// `formfields` above — this one runs at request time, not only under
// `cargo test`.
mod toml_doc;

use std::io::Write as _;
use std::sync::Arc;

/// The files this program is given, in `RIVET_APP_DATA`.
const SERVER_CERTIFICATE: &str = "server.crt";
/// Its key.
const SERVER_KEY: &str = "server.key";
/// The authority both ends are checked against.
const AUTHORITY: &str = "ca.crt";

/// Read a PEM file out of the data directory the supervisor handed over.
fn handed(name: &str) -> Result<Vec<u8>, String> {
    let dir = rivet_sdk::app::data().map_err(|e| e.to_string())?;
    let path = dir.join(name);
    std::fs::read(&path).map_err(|e| {
        format!(
            "{}: {e}. `rivet-provision` mints these when a machine is provisioned as a fleet \
             server; they are per-machine and are not in the payload",
            path.display()
        )
    })
}

/// Build the TLS configuration for the browser side.
///
/// **Client certificates are REQUIRED, not optional.** An unauthenticated
/// administrative surface on a LAN is the failure this whole design is trying
/// to avoid, and `rustls` will happily build a server that asks for nothing.
fn browser_tls() -> Result<rustls::ServerConfig, String> {
    let certificate = handed(SERVER_CERTIFICATE)?;
    let key = handed(SERVER_KEY)?;
    let authority = handed(AUTHORITY)?;
    browser_tls_from(&certificate, &key, &authority)
}

/// [`browser_tls`], from the three files' bytes already in hand.
///
/// Split out so `gate_tests` can build the configuration this program serves
/// with, over a throwaway authority, with no `RIVET_APP_DATA`.
fn browser_tls_from(
    certificate: &[u8],
    key: &[u8],
    authority: &[u8],
) -> Result<rustls::ServerConfig, String> {
    let chain: Vec<_> = rustls_pemfile::certs(&mut &*certificate)
        .collect::<Result<_, _>>()
        .map_err(|e| format!("{SERVER_CERTIFICATE}: {e}"))?;
    let private = rustls_pemfile::private_key(&mut &*key)
        .map_err(|e| format!("{SERVER_KEY}: {e}"))?
        .ok_or_else(|| format!("{SERVER_KEY} carries no private key"))?;

    let mut roots = rustls::RootCertStore::empty();
    for root in rustls_pemfile::certs(&mut &*authority) {
        let root = root.map_err(|e| format!("{AUTHORITY}: {e}"))?;
        roots
            .add(root)
            .map_err(|e| format!("{AUTHORITY}: {e}"))?;
    }

    let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
        Arc::new(roots),
        rivetos_tls::provider(),
    )
    .build()
    .map_err(|e| format!("client verifier: {e}"))?;

    rustls::ServerConfig::builder_with_provider(rivetos_tls::provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("protocol versions: {e}"))?
        .with_client_cert_verifier(verifier)
        .with_single_cert(chain, private)
        .map_err(|e| format!("certificate and key do not go together: {e}"))
}

/// An HTML body, as an HTTP answer.
///
/// **`page::html` returns a document, not a response**, and the difference cost
/// a deploy: handing the bare HTML to `write_all` made curl say *"Received
/// HTTP/0.9 when not allowed"*, which is what a client says when the first line
/// of the answer is not a status line.
fn html(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// A JSON body, as an HTTP answer.
fn json(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// **Ask the machine a request line names**, and return its own answer.
///
/// The device is resolved through `Register::resolve`, which takes an
/// identifier or a label and refuses an ambiguous label rather than choosing —
/// and the credential comes from that record, because `ADR-034` §6 is where a
/// device's pin lives.
///
/// # Errors
///
/// A sentence. A machine that is in the register and does not answer is a
/// different thing from one that is not in the register, and both say which.
fn reach(request: &str, body: &str) -> Result<String, String> {
    let target = request
        .split_whitespace()
        .nth(1)
        .and_then(|path| path.strip_prefix("/mcp/"))
        .map(|rest| rest.split(['?', '#']).next().unwrap_or(rest))
        .filter(|rest| !rest.is_empty())
        .ok_or_else(|| "POST /mcp/<device>: no machine was named".to_owned())?;
    let asked = percent_decoded(target);

    let register = fleet::load()?;
    let device = register.resolve(&asked).map_err(|e| e.to_string())?;
    let pin = device.pin.as_deref().ok_or_else(|| {
        format!(
            "{}'s record carries no pin, so there is nothing to trust at the other end. \
             ADR-025 section 2 makes the pinned key the whole trust model, and connecting \
             without one would trust whatever answered at that address.",
            device.name
        )
    })?;
    // **`dial_host`, not `address`.** This is the browser's reach into a
    // machine; the strict `address()` refuses a DHCP member outright (no
    // address in the record), which is why the Overview showed "nowhere to
    // reach it" even though the sidebar had already derived `<name>.local`.
    // `dial_host` derives that name and `node::ask` resolves it over mDNS.
    let address = format!("{}:{}", device.dial_host()?, device.control_plane_port());
    node::ask(&address, pin, body)
}

/// `%XX` back to bytes, because a label may carry a space.
///
/// Only what a path segment needs — this is not a URL library and does not
/// pretend to be one.
fn percent_decoded(text: &str) -> String {
    let raw = text.as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' && i + 2 < raw.len() {
            let hex = std::str::from_utf8(&raw[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(raw[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whoever presented a certificate, by its fingerprint.
///
/// **It was `rivetos_tls::public_key_in_certificate` and that stopped working
/// the moment the authority moved to ECDSA.** That function looks for an
/// Ed25519 `SubjectPublicKeyInfo`, finds none in a P-256 certificate, and
/// returns `None` — so this page told the operator they were *"a
/// certificate"*, which is true and useless.
///
/// A SHA-256 fingerprint identifies any certificate whatever it is signed
/// with, and needs no parser. **Nothing here is an authorisation decision** —
/// the handshake refused anybody without a certificate from this fleet's
/// authority, and [`admit`] decides whether that certificate is the operator's.
/// This is a label.
fn who(connection: &rustls::ServerConnection) -> String {
    use sha2::Digest as _;
    connection
        .peer_certificates()
        .and_then(<[_]>::first)
        .map_or_else(
            || "unidentified".to_owned(),
            |first| {
                let digest = sha2::Sha256::digest(first.as_ref());
                let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
                hex
            },
        )
}

/// Fleet-as-code's configuration, out of what this application was handed.
///
/// `None` means it is not configured: `fleet-repository` unset is the off
/// switch, and it is off by default rather than by an extra flag.
fn fleet_as_code(dry_run: bool) -> Option<code::Fleet> {
    let repository = rivet_sdk::settings::get(code::setting::REPOSITORY)?;
    let data = match rivet_sdk::app::data() {
        Ok(dir) => dir.join(code::DIR),
        Err(why) => {
            println!(
                "rivet-fleetd: fleet-as-code is configured and there is no data directory to \
                 keep its baselines in: {why}"
            );
            return None;
        }
    };
    Some(code::Fleet {
        repository,
        branch: rivet_sdk::settings::get(code::setting::BRANCH)
            .unwrap_or_else(|| code::DEFAULT_BRANCH.to_owned()),
        baselines: data.join(code::BASELINES),
        dry_run,
    })
}

/// How often the poller looks, out of the same settings.
///
/// A value that is not a number is the default and a line saying so, rather
/// than a refusal: `ADR-038` §10.2 makes settings text and leaves each
/// application to decide what a bad one means, and stopping a fleet server
/// from starting because somebody typed `every minute` would be the wrong
/// answer to a typo.
fn poll_interval() -> std::time::Duration {
    match rivet_sdk::settings::get(code::setting::INTERVAL) {
        None => code::DEFAULT_INTERVAL,
        Some(text) => match text.trim().parse::<u64>() {
            Ok(seconds) if seconds > 0 => std::time::Duration::from_secs(seconds),
            _ => {
                println!(
                    "rivet-fleetd: {} is {text:?}, which is not a number of seconds. Using {}s.",
                    code::setting::INTERVAL,
                    code::DEFAULT_INTERVAL.as_secs()
                );
                code::DEFAULT_INTERVAL
            }
        },
    }
}

/// **One pass, and then exit** — `rivet-fleetd converge [--dry-run]`.
///
/// The same code path the poller runs, driven once from a command line. It
/// exists for two callers and both matter: an operator who has pushed and does
/// not want to wait for the next cycle, and `ADR-044`'s proof, which needs a
/// pass it can assert about rather than a daemon it has to watch.
///
/// **It does not need the listening socket**, which is why it is checked before
/// `main` asks for one: a fleet server driven this way is not serving anybody.
fn converge_once(dry_run: bool) -> std::process::ExitCode {
    let Some(fleet) = fleet_as_code(dry_run) else {
        println!(
            "rivet-fleetd: fleet-as-code is not configured on this machine. Set `{}` under \
             [applications.rivet-fleetd] in this machine's own document to the repository \
             holding the fleet's desired-state documents.",
            code::setting::REPOSITORY
        );
        return std::process::ExitCode::FAILURE;
    };
    let register = match fleet::load() {
        Ok(register) => register,
        Err(why) => {
            println!("rivet-fleetd: {why}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let control = match repo::this_server(&register) {
        Ok(control) => control,
        Err(why) => {
            println!("rivet-fleetd: fleet-as-code: {why}");
            return std::process::ExitCode::FAILURE;
        }
    };
    match code::pass(&fleet, &register, &control) {
        Ok(pass) => {
            if let Some(said) = code::summary(&pass) {
                println!("rivet-fleetd: {said}");
            } else {
                println!("rivet-fleetd: fleet-as-code: the fleet agrees with the repository.");
            }
            // **A refusal is a non-zero exit and an unreachable machine is
            // not.** They are different things and a script that could not
            // tell them apart would retry the one that needs a person.
            if pass.needs_reading() {
                std::process::ExitCode::FAILURE
            } else {
                std::process::ExitCode::SUCCESS
            }
        }
        Err(why) => {
            println!("rivet-fleetd: fleet-as-code: {why}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// The poller, for as long as this program runs.
///
/// It starts only when fleet-as-code is configured, so a fleet server that has
/// never been told about a repository grows no thread and says nothing.
fn start_polling() {
    let Some(fleet) = fleet_as_code(false) else {
        return;
    };
    let every = poll_interval();
    println!(
        "rivet-fleetd: fleet-as-code: polling repository {} ({}) on this server's control plane \
         every {}s",
        fleet.repository,
        fleet.branch,
        every.as_secs()
    );
    std::thread::spawn(move || {
        loop {
            let outcome = fleet::load().and_then(|register| {
                let control = repo::this_server(&register)?;
                code::pass(&fleet, &register, &control)
            });
            match outcome {
                Ok(pass) => {
                    if let Some(said) = code::summary(&pass) {
                        println!("rivet-fleetd: {said}");
                    }
                }
                // **The repository being unreachable is reported once per
                // cycle and stops nothing.** It is the same category as a
                // machine that is off: the fleet is still whatever it was.
                Err(why) => println!("rivet-fleetd: fleet-as-code: {why}"),
            }
            std::thread::sleep(every);
        }
    });
}

/// Serve `SPEC-06` on the `gantry` socket, on a thread of its own.
///
/// **A missing socket stops the gantry socket and nothing else.** The operator's
/// page does not depend on it, and a fleet server that refused to start because
/// a second application's endpoint was absent would take the administration
/// surface down over something the administration surface is needed to fix.
/// Each way it can be absent is said once, at start.
fn start_gantry(socket: Option<rivet_sdk::listen::Socket>) {
    let Some(socket) = socket else {
        println!(
            "rivet-fleetd: gantry socket: none was handed over, so gantry cannot reach this \
             server. This application declares provides = [\"socket:{}\"] and the supervisor \
             binds it",
            gantry::ENDPOINT
        );
        return;
    };
    let Some(listener) = socket.into_unix_listener() else {
        println!(
            "rivet-fleetd: gantry socket: the descriptor was already taken by something else in \
             this process, so it is not served"
        );
        return;
    };
    let data = match rivet_sdk::app::data() {
        Ok(dir) => dir,
        Err(why) => {
            println!("rivet-fleetd: gantry socket: not served: {why}");
            return;
        }
    };
    let Some(gate) = gantry::Gate::for_consumer() else {
        println!(
            "rivet-fleetd: gantry socket: {:?} is not a service name, so there is no uid to \
             admit and the socket is not served",
            gantry::CONSUMER
        );
        return;
    };
    println!(
        "rivet-fleetd: gantry socket: serving uid {} only, idle connections closed after {}s",
        gate.uid,
        gate.idle.as_secs()
    );
    let provider = gantry::Provider::new(data);
    std::thread::spawn(move || gantry::serve(&listener, &provider, gate));
}

fn main() -> std::process::ExitCode {
    // **Before the socket**, because this form serves nobody and a machine
    // where the supervisor holds the port must still be able to run it by hand.
    let words: Vec<String> = std::env::args().skip(1).collect();
    if words.first().map(String::as_str) == Some("converge") {
        let dry_run = words.iter().any(|word| word == "--dry-run");
        return converge_once(dry_run);
    }
    if !words.is_empty() {
        println!(
            "rivet-fleetd: {:?} is not something this program takes. It is a daemon the \
             supervisor starts with no arguments; the one other form is `rivet-fleetd \
             converge [--dry-run]`, which makes one fleet-as-code pass and exits.",
            words.join(" ")
        );
        return std::process::ExitCode::FAILURE;
    }

    println!("rivet-fleetd: starting");

    // **The parent pid, recorded before anything can reparent this process** —
    // `ADR-056` §3. The supervisor started this program, so the parent IS the
    // supervisor; comparing this against the parent at render is what lets the
    // page tell *the supervisor stopped* from *the supervisor was replaced*.
    supervisor::remember_parent();

    // **Handed the socket, not opening one.** A privileged port is unreachable
    // to this process's uid; the supervisor bound it and passed the descriptor.
    // **This program never names the number**, which is why moving 4443 was a
    // manifest edit and not a code change.
    //
    // **The TCP one, found by kind rather than by position**, because the
    // manifest also provides the `gantry` unix socket and the supervisor hands
    // both over in one list.
    let mut handed = rivet_sdk::listen::sockets();
    let gantry_socket = handed
        .iter()
        .position(|s| s.name == gantry::ENDPOINT)
        .map(|at| handed.remove(at));
    let Some(socket) = handed.into_iter().find(rivet_sdk::listen::Socket::is_tcp) else {
        println!(
            "rivet-fleetd: no listening socket was handed over. This application declares \
             listen = [\"tcp:4443\"] and the supervisor binds it — an absent socket means it \
             was started outside that, and there is nothing to serve on"
        );
        return std::process::ExitCode::FAILURE;
    };
    // `None` means something in this process already took that descriptor,
    // which for a program with one socket means a defect in this program —
    // see `rivet_sdk::listen::Socket::into_tcp_listener`.
    let Some(listener) = socket.into_tcp_listener() else {
        println!(
            "rivet-fleetd: the listening descriptor was already taken by something else in \
             this process, so it cannot be served on twice"
        );
        return std::process::ExitCode::FAILURE;
    };

    let config = match browser_tls() {
        Ok(c) => Arc::new(c),
        Err(why) => {
            println!("rivet-fleetd: {why}");
            return std::process::ExitCode::FAILURE;
        }
    };
    println!("rivet-fleetd: serving, and a client certificate is required");

    println!(
        "rivet-fleetd: at most {MAX_CONNECTIONS} connections at once, {}s each",
        PATIENCE.as_secs()
    );

    // **After the listener is up, not before.** Serving the operator's page is
    // this program's first job, and a first convergence pass that took twenty
    // seconds against an unreachable machine would delay it for no reason.
    start_polling();

    start_gantry(gantry_socket);

    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        // **The deadline goes on before the handshake, as a single absolute
        // instant rather than a per-read budget.** Until the handshake
        // completes the peer is nobody, and a peer that sends one byte just
        // inside `PATIENCE`, forever, must not hold this connection for the
        // life of the process — see `read_request` for where this is spent.
        let deadline = std::time::Instant::now() + PATIENCE;
        // Claimed before a thread exists — see `Slot`. A peer whose address
        // cannot be read is weighed as one unknown source, never as none.
        let src = stream
            .peer_addr()
            .map_or(std::net::IpAddr::from([0, 0, 0, 0]), |peer| peer.ip());
        let slot = match Slot::claim(src) {
            Ok(slot) => slot,
            Err(Full::Global) => {
                println!(
                    "rivet-fleetd: refused: {MAX_CONNECTIONS} connections are already open. This \
                     is the limit working"
                );
                continue;
            }
            Err(Full::PerSource) => {
                println!(
                    "rivet-fleetd: refused {src}: it already holds {} of the {MAX_CONNECTIONS} \
                     connections, and the rest are kept for other addresses. This is the limit \
                     working",
                    rivet_core::connection_slots::PER_SOURCE_SLOTS
                );
                continue;
            }
        };
        // **The response write keeps its own flat patience.** Unrelated to the
        // read deadline above: this is how long the operator's browser has to
        // accept the answer, once there is one to send.
        if let Err(why) = stream.set_write_timeout(Some(PATIENCE)) {
            println!("rivet-fleetd: refused: no deadline could be set: {why}");
            continue;
        }
        let config = Arc::clone(&config);
        // **One thread per connection**, added 2026-09-03. This loop used to be
        // the whole server: one client that connected and fell silent stopped
        // every operator's page, and one node that was slow to answer a
        // `POST /mcp/<device>` did the same for as long as `node::PATIENCE`.
        std::thread::spawn(move || {
            let _slot = slot;
            serve_one(&stream, &config, deadline);
        });
    }
    std::process::ExitCode::SUCCESS
}

/// One connection, from the handshake to the answer.
///
/// `deadline` is the absolute instant by which the handshake and the request
/// head must both be in hand — see `read_request`.
fn serve_one(
    stream: &std::net::TcpStream,
    config: &Arc<rustls::ServerConfig>,
    deadline: std::time::Instant,
) {
    let mut socket = stream;
    let Ok(mut connection) = rustls::ServerConnection::new(Arc::clone(config)) else {
        return;
    };
    let mut tls = rustls::Stream::new(&mut connection, &mut socket);

    // **A handshake that fails is the point, not an error.** A browser with
    // no certificate is refused here, which is the mechanism working.
    let set_read_deadline = |d| stream.set_read_timeout(Some(d));
    let (text, body) = match read_request(&mut tls, deadline, PATIENCE, set_read_deadline) {
        Ok(both) => both,
        Err(why) => {
            println!("rivet-fleetd: turned away: {why}");
            return;
        }
    };
    let peer = Peer {
        label: who(tls.conn),
        // The leaf is `chain[0]`. A verified handshake always has one; a
        // connection that somehow reached here without it is not the operator.
        caller: tls
            .conn
            .peer_certificates()
            .and_then(<[_]>::first)
            .map_or(rivetos_tls::Caller::Unreadable, |leaf| rivetos_tls::classify_caller(leaf)),
    };
    // The request line, so a route that does not match says why.
    println!("rivet-fleetd: <- {}", text.lines().next().unwrap_or("(nothing)").trim());

    let reply = match admit(&peer, &text) {
        Ok(operator) => dispatch(&text, body.trim(), &operator),
        Err(refusal) => refusal,
    };
    if let Err(why) = tls.write_all(reply.as_bytes()) {
        println!("rivet-fleetd: {why}");
    }
    let _ = tls.flush();
}

/// Whoever completed the handshake: a label for the log, and what their
/// certificate says about who holds it.
struct Peer {
    /// [`who`]'s fingerprint.
    label: String,
    /// `rivetos_tls::classify_caller` of the leaf.
    caller: rivetos_tls::Caller,
}

/// **A caller [`admit`] let through**, and the only way into [`dispatch`].
///
/// A type rather than a flag, so that a handler cannot be reached with a
/// caller nobody checked: every route, including one added later under any
/// prefix, receives one of these, and there is one function that makes them.
struct Operator {
    /// The fingerprint, for the log lines.
    label: String,
}

impl std::fmt::Display for Operator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// **The operator gate: this server answers the operator, and nobody else** —
/// `ADR-043` §5, `.rulings/` R-187 D1.
///
/// The handshake admits any certificate the fleet authority issued, and
/// provisioning issues one to every fleet machine's front door. So a verified
/// chain is not enough: the leaf must name no host, which is the operator's
/// browser certificate and never a machine's (`rivetos_tls::classify_caller`).
///
/// **Every request, not a list of routes.** `/api/*` and `/mcp/*` drive the
/// fleet, and the portal page is the operator's page; no fleet machine has a
/// reason to ask this server for any of it. A gate keyed on the whole server
/// cannot be missed by a route added under a new prefix, and refusing the page
/// too means a machine's key learns nothing here but the refusal.
///
/// A refusal is `403 Forbidden` with a JSON-RPC error `-32003` saying why, and a
/// log line naming the certificate and the request, as the cross-site refusal
/// is logged.
fn admit(peer: &Peer, text: &str) -> Result<Operator, String> {
    let Some(why) = peer.caller.refusal() else {
        return Ok(Operator {
            label: peer.label.clone(),
        });
    };
    println!(
        "rivet-fleetd: {}: REFUSED {}: {why}",
        peer.label,
        first_line(text)
    );
    let mut body = String::from("{\"error\":{\"code\":-32003,\"message\":");
    rivet_core::journal::write_json_string(&format!("refused: {why}"), &mut body);
    body.push_str("}}");
    Err(format!(
        "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    ))
}

/// **The router.** Decides the reply to one already-read request.
///
/// Split out of [`serve_one`] — added for R41 (`.rulings/` R-169) — so the
/// DISPATCH can be exercised directly against request text with no TLS
/// handshake and no socket: every branch here is a function of the request
/// head and body plus this server's own configuration (the register,
/// fleet-as-code), never of the connection that carried it.
///
/// **It takes an [`Operator`], which only [`admit`] makes**, so nothing here
/// runs for a caller the gate refused. `peer` is otherwise only for the log
/// lines.
fn dispatch(text: &str, body: &str, peer: &Operator) -> String {
    let mutates = text.starts_with("POST /mcp") || text.starts_with("POST /api/commit");
    if mutates && !drivable(text) {
        // **Cross-site request forgery, refused by the two headers that can
        // say so** — the finding at `PLAN.md` §123, Wave 1.5.
        //
        // The browser attaches this operator's client certificate to ANY
        // request to this origin, including one a page on another site caused.
        // So without this check a hostile page could `POST /mcp/<device>` as
        // `text/plain` — which is a "simple request" and gets no CORS preflight
        // — and run `rivet_apply` against every machine in the register, with
        // the operator's own identity, while they had the page open in another
        // tab.
        //
        // See `drivable` for which header does the work and which is only
        // corroboration.
        println!("rivet-fleetd: {peer}: REFUSED a cross-site request to {}", first_line(text));
        json(
            "{\"error\":{\"code\":-32600,\"message\":\"refused: this endpoint requires \
             Content-Type: application/json, and is refused outright when the browser reports \
             that another site caused the request (Sec-Fetch-Site, or an Origin that is not \
             this server's). It drives real machines or writes to the fleet \
             repository with the operator's own identity, and a browser attaches that identity \
             to any request to this origin - including one another site caused.\"}}",
        )
    } else if text.starts_with("GET /supervisor") {
        // **`ADR-056` §3, and `.rulings/` R-195 D2.** The one route here that
        // reaches no machine and holds no client identity: it opens `/proc` in
        // this process and returns HTML.
        //
        // **Among the human pages and not under `/api/`, deliberately.** §3.7's
        // constraint is that this must not become something an agent reaches for
        // instead of `rivet-mcpd`, and access control cannot deliver it — anyone
        // who can open this page holds the same operator certificate the control
        // plane takes. So the page is made useless as a machine interface rather
        // than merely discouraged: no JSON on the path, no inputs to vary, no
        // version in the route, and no stable contract at all.
        //
        // **It costs a second**, because §3.5's whole question is a difference
        // and one sample of a counter says nothing. That is also what stops it
        // being cheap enough to poll.
        let reading = supervisor::read();
        println!(
            "rivet-fleetd: {peer} asked whether the supervisor is alive, and it is {}",
            if reading.moved.turned() { "turning" } else { "NOT TURNING" }
        );
        html(&supervisor::html(&reading))
    } else if text.starts_with("GET /api/fleet") {
            // **The register, and only the register — plus one boolean this
            // server was TOLD about ITSELF.** `fleetAsCode` is not a second
            // opinion about a machine's state (principle 2, `ADR-043` §2): it
            // is this server's own `fleet-repository` setting, read the same
            // way `code::pass` already does, so the Config screen can say
            // WHY Save is disabled instead of a button that just fails.
            match fleet::load() {
                Ok(register) => {
                    println!("rivet-fleetd: {peer} read the register, {} device(s)", register.devices.len());
                    let configured = rivet_sdk::settings::get(code::setting::REPOSITORY).is_some();
                    json(&fleet::as_json(&register, configured))
                }
                Err(why) => {
                    println!("rivet-fleetd: {peer}: {why}");
                    json(&format!(
                        "{{\"error\":{{\"code\":-32001,\"message\":{why:?}}}}}"
                    ))
                }
            }
        } else if text.starts_with("POST /api/commit/") {
            // **R39.** The write half of Save — see `commit`'s own module
            // documentation for why this is a new route rather than a reuse
            // of `POST /mcp`.
            let target = text
                .split_whitespace()
                .nth(1)
                .and_then(|path| path.strip_prefix("/api/commit/"))
                .map(|rest| rest.split(['?', '#']).next().unwrap_or(rest))
                .filter(|rest| !rest.is_empty());
            match target {
                None => json(
                    "{\"error\":{\"code\":-32602,\"message\":\"POST /api/commit/<device> - name \
                     the machine by its identifier or its label\"}}",
                ),
                Some(target) => {
                    let device = percent_decoded(target);
                    let fleet = fleet_as_code(false);
                    match fleet::load().and_then(|register| {
                        let control = repo::this_server(&register);
                        let control = control.as_ref().map(|c| c as &dyn repo::ControlPlane).map_err(String::clone);
                        commit::handle(fleet.as_ref(), &register, control, &device, body.trim())
                    }) {
                        Ok(answer) => {
                            println!("rivet-fleetd: {peer} committed to the fleet repository for {device}");
                            json(&answer)
                        }
                        Err(why) => {
                            println!("rivet-fleetd: {peer}: {why}");
                            json(&format!(
                                "{{\"error\":{{\"code\":-32010,\"message\":{why:?}}}}}"
                            ))
                        }
                    }
                }
            }
        } else if text.starts_with("POST /mcp/") {
            // **The join.** The browser's JSON-RPC goes to the machine over
            // ADR-035's control plane and the machine's answer comes back
            // unchanged — no parsing, no translation, no cache.
            match reach(text, body.trim()) {
                Ok(answer) => {
                    println!("rivet-fleetd: {peer} asked the machine, {} bytes back", answer.len());
                    json(&answer)
                }
                Err(why) => {
                    println!("rivet-fleetd: {peer}: {why}");
                    json(&format!(
                        "{{\"error\":{{\"code\":-32000,\"message\":{:?}}}}}",
                        why
                    ))
                }
            }
        } else if text.starts_with("POST /mcp") {
            // **Named, never guessed.** A fleet server with a register reaches
            // whichever machine it is asked for, and picking one for a caller
            // who did not say is the mistake `ADR-034` §10.2 calls expensive to
            // undo — the same reason `Register::resolve` refuses an ambiguous
            // label rather than taking the first.
            json(
                "{\"error\":{\"code\":-32602,\"message\":\"POST /mcp/<device> - name the machine \
                 by its identifier or its label, as GET /api/fleet spells them\"}}",
            )
    } else {
        println!("rivet-fleetd: served {peer}");
        html(&page::html())
    }
}

/// The most connections this server will hold at once, and how long each gets.
///
/// A fleet page is one browser and a handful of requests. Sixteen is several
/// times that; the point of the number is that it is a number.
const MAX_CONNECTIONS: usize = 16;

/// How long a connection may block on any one read or write.
///
/// Longer than a browser needs and shorter than "for ever", which is what it
/// was: this loop had no deadline at all, so a client that connected and said
/// nothing held the whole server.
const PATIENCE: std::time::Duration = std::time::Duration::from_secs(30);

/// The largest request head this will buffer, and the largest body.
///
/// A JSON-RPC call to a machine is small. These are the point past which the
/// caller is no longer sending a request, not a working limit.
const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 1024 * 1024;

use rivet_core::connection_slots::{Full, Slots};

/// Which source holds how many of this server's slots.
static SLOTS: std::sync::Mutex<Slots<std::net::IpAddr>> = std::sync::Mutex::new(Slots::new(
    MAX_CONNECTIONS,
    rivet_core::connection_slots::PER_SOURCE_SLOTS,
));

/// The pool, locked. Nothing under the lock can panic, so a poisoned lock is
/// recovered rather than taking the server down with it.
fn slots() -> std::sync::MutexGuard<'static, Slots<std::net::IpAddr>> {
    SLOTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// One connection's claim on a slot, released when it is dropped.
struct Slot {
    src: std::net::IpAddr,
}

impl Slot {
    /// A slot for a connection from `src`, or why there is none: every slot
    /// held, or `src` already holding its share.
    fn claim(src: std::net::IpAddr) -> Result<Self, Full> {
        slots().take(src).map(|()| Self { src })
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        slots().give_back(self.src);
    }
}

/// The first line of a request, for a log.
fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("(nothing)").trim()
}

// One header's value, and every value of one header, case-insensitively, from
// a request head — the same reading `rivet-proxyd`'s cross-site guard uses, so
// the two cannot disagree about which header a request carried.
use rivet_core::cross_site::{header, header_values};

/// Whether this request may drive a machine: really JSON, and not from
/// somewhere else.
///
/// # The two checks are not symmetrical, and the asymmetry is the design
///
/// **`Content-Type: application/json` is REQUIRED.** It is what actually stops
/// the attack: `application/json` is not one of the three content types a
/// cross-site "simple request" may carry, so a page on another site cannot send
/// it without a CORS preflight, and this server answers no preflight. Requiring
/// it costs a non-browser caller one header. This half is this server's own
/// policy about its administrative endpoints (`ADR-040` §8 keeps it here).
///
/// **Whether another site caused the request is
/// `rivet_core::cross_site::is_cross_site`**, the rule `rivet-proxyd` applies
/// to every application (`ADR-046` §4, `.rulings/` R-186 D1): `Sec-Fetch-Site`
/// when the browser sent it, `Origin` against this request's own origin when it
/// sent only that, and a pass for a request carrying neither. `curl`, and any
/// tool that is not a browser, sends neither, and requiring one would mean an
/// operator's own script had to impersonate a browser to talk to their own
/// fleet server.
fn drivable(head: &str) -> bool {
    let json = header(head, "content-type")
        .is_some_and(|v| v.trim_start().to_ascii_lowercase().starts_with("application/json"));
    json && !rivet_core::cross_site::is_cross_site(head, "https")
}

/// Read one HTTP request: the head under one absolute `deadline`, then
/// exactly `Content-Length` bytes of body under `patience` per read.
///
/// **This is a framing fix and not only a size limit.** It was one
/// `read(&mut [0u8; 1024])`: a request larger than a kilobyte was silently
/// truncated, a body that arrived in a second TCP segment was silently missing,
/// and the body was recovered by splitting the same 1024 bytes on `\r\n\r\n`
/// and trimming NULs off whatever was left. That worked because a fleet page's
/// requests are small, which is not the same as being correct.
///
/// **The head phase has one deadline; the body phase goes back to a flat
/// per-read patience.** A flat per-read timeout applied to the whole
/// connection lets a peer that sends one byte just inside it, forever, hold a
/// slot forever — the handshake and the head are small and have no reason to
/// take long, so `deadline` bounds them together as one budget from accept.
/// A body already declared by `Content-Length` and capped by `MAX_BODY` is
/// not an indefinite hold, it is upload speed, so it is deliberately NOT
/// re-bound by `deadline`: a legitimate slow upload must not be cut off by
/// how long the handshake happened to take.
///
/// `set_read_deadline` applies `patience` (or less, while the head deadline is
/// closing in) however the caller's reader is bounded — a real socket's
/// `set_read_timeout`, or nothing for a fixture with no clock of its own.
fn read_request(
    tls: &mut impl std::io::Read,
    deadline: std::time::Instant,
    patience: std::time::Duration,
    mut set_read_deadline: impl FnMut(std::time::Duration) -> std::io::Result<()>,
) -> Result<(String, String), String> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];

    let head_end = loop {
        if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
        if buf.len() > MAX_HEAD {
            return Err(format!("a request head passed {MAX_HEAD} bytes with no blank line"));
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(format!(
                "the handshake and request head did not complete within {}s",
                patience.as_secs()
            ));
        }
        set_read_deadline(remaining).map_err(|e| e.to_string())?;
        let got = tls.read(&mut chunk).map_err(|e| e.to_string())?;
        if got == 0 {
            return Err("the peer closed before finishing its request head".to_string());
        }
        buf.extend_from_slice(&chunk[..got]);
    };

    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    // **A repeated `Content-Length` is refused whether or not the two values
    // agree** (`.rulings/` R-210 D17). Taking the first and ignoring the rest
    // is exactly the smuggling precondition: this reader and whatever else
    // looks at the same bytes could honour different fields and disagree
    // about where the body ends.
    let lengths: Vec<&str> = header_values(&head, "content-length").collect();
    if lengths.len() > 1 {
        return Err(format!(
            "two Content-Length fields, {}: this request has no one length",
            lengths.join(" and ")
        ));
    }
    let length: usize = header(&head, "content-length")
        .map_or(Ok(0), |v| v.trim().parse())
        .map_err(|_| "Content-Length is not a number".to_string())?;
    if length > MAX_BODY {
        return Err(format!("a request body of {length} bytes is past the {MAX_BODY} limit"));
    }

    // **Per read again, and unbounded in total.** The deadline above covered
    // the handshake and the head; a body whose length was already declared
    // and capped is bounded by its own promise, not by how long that took.
    set_read_deadline(patience).map_err(|e| e.to_string())?;

    let mut body = buf.split_off(head_end);
    while body.len() < length {
        let got = tls.read(&mut chunk).map_err(|e| e.to_string())?;
        if got == 0 {
            return Err(format!(
                "the peer promised {length} bytes of body and sent {}",
                body.len()
            ));
        }
        body.extend_from_slice(&chunk[..got]);
    }
    body.truncate(length);

    Ok((head, String::from_utf8_lossy(&body).into_owned()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// A reader that hands over `at_a_time` bytes per call, so a request can be
    /// made to arrive in pieces the way TCP really delivers one.
    struct Dribble {
        bytes: Vec<u8>,
        at_a_time: usize,
    }

    impl std::io::Read for Dribble {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.bytes.len().min(buf.len()).min(self.at_a_time);
            buf[..n].copy_from_slice(&self.bytes[..n]);
            self.bytes.drain(..n);
            Ok(n)
        }
    }

    /// [`read_request`], with a deadline far enough away that `Dribble` — which
    /// never blocks and so never exercises it — cannot trip it, and a no-op
    /// deadline setter, since a fixture with no real clock has nothing to set.
    fn request(head_and_body: &str, at_a_time: usize) -> Result<(String, String), String> {
        read_request(
            &mut Dribble { bytes: head_and_body.as_bytes().to_vec(), at_a_time },
            std::time::Instant::now() + PATIENCE,
            PATIENCE,
            |_| Ok(()),
        )
    }

    const CALL: &str = "POST /mcp/board HTTP/1.1\r\n\
                        Host: fleet\r\n\
                        Content-Type: application/json\r\n\
                        Sec-Fetch-Site: same-origin\r\n\
                        Content-Length: 40\r\n\
                        \r\n\
                        {\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}";

    /// **A body split across TCP segments is still read.** The single
    /// `read(&mut [0u8; 1024])` this replaced took whatever one call happened to
    /// return; a body in a second segment was simply missing, and the request
    /// went to the machine truncated.
    #[test]
    fn a_request_delivered_one_byte_at_a_time_is_read_whole() {
        for chunk in [1usize, 7, 64, 4096] {
            let (head, body) = request(CALL, chunk).unwrap_or_else(|e| panic!("chunk {chunk}: {e}"));
            assert!(head.starts_with("POST /mcp/board"), "chunk {chunk}: {head}");
            assert_eq!(body, "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}", "chunk {chunk}");
        }
    }

    /// Only `Content-Length` bytes are taken, so a pipelined second request
    /// cannot be glued onto the first one's body.
    #[test]
    fn the_body_stops_at_content_length() {
        let two = format!("{CALL}POST /mcp/other HTTP/1.1\r\n\r\n");
        let (_, body) = request(&two, 4096).expect("read");
        assert_eq!(body.len(), 40, "{body}");
        assert!(!body.contains("other"), "a second request leaked into the first's body: {body}");
    }

    /// A head that never ends is refused rather than buffered for ever.
    #[test]
    fn a_head_with_no_blank_line_is_refused() {
        let endless = format!("GET / HTTP/1.1\r\nX: {}\r\n", "a".repeat(MAX_HEAD + 1));
        let why = request(&endless, 4096).expect_err("refused");
        assert!(why.contains("no blank line"), "{why}");
    }

    /// A body larger than the limit is refused on the header, before a byte of
    /// it is read.
    #[test]
    fn an_oversized_body_is_refused_from_its_declared_length() {
        let head = format!(
            "POST /mcp/board HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY + 1
        );
        let why = request(&head, 4096).expect_err("refused");
        assert!(why.contains("past the"), "{why}");
    }

    /// **A repeated `Content-Length` is refused even when both copies agree**
    /// — `.rulings/` R-210 D17. Matching values are no safer than disagreeing
    /// ones: this reader would otherwise have to decide which of two
    /// same-named headers was the real one.
    #[test]
    fn two_identical_content_length_headers_are_refused() {
        let head = "POST /mcp/board HTTP/1.1\r\nContent-Length: 5\r\nContent-Length: 5\r\n\r\nhello";
        let why = request(head, 4096).expect_err("refused");
        assert!(why.contains("Content-Length"), "{why}");
    }

    /// **Two `Content-Length` fields that disagree are refused** — the classic
    /// smuggling shape: this reader honouring one value and something in front
    /// of it honouring the other would disagree about where the body ends.
    #[test]
    fn two_disagreeing_content_length_headers_are_refused() {
        let head = "POST /mcp/board HTTP/1.1\r\nContent-Length: 5\r\nContent-Length: 7\r\n\r\nhello";
        let why = request(head, 4096).expect_err("refused");
        assert!(why.contains("Content-Length"), "{why}");
    }

    /// A request with no `Content-Length` at all still reads as an empty body
    /// — the refusal above must not have touched this path.
    #[test]
    fn no_content_length_is_still_an_empty_body() {
        let head = "GET /api/fleet HTTP/1.1\r\nHost: fleet\r\n\r\n";
        let (parsed, body) = request(head, 4096).expect("read");
        assert!(parsed.starts_with("GET /api/fleet"), "{parsed}");
        assert_eq!(body, "");
    }

    /// A peer that promises a body and then hangs up is an error, not a short
    /// request that gets forwarded to a machine.
    #[test]
    fn a_truncated_body_is_an_error_and_not_a_shorter_request() {
        let cut = &CALL[..CALL.len() - 10];
        let why = request(cut, 4096).expect_err("refused");
        assert!(why.contains("promised"), "{why}");
    }

    /// **The forgery, refused.** A hostile page can only send a content type
    /// that needs no preflight, and this is the list of them.
    #[test]
    fn a_cross_site_post_cannot_drive_a_machine() {
        for content_type in ["text/plain", "application/x-www-form-urlencoded", "multipart/form-data"] {
            let head = format!(
                "POST /mcp/board HTTP/1.1\r\nContent-Type: {content_type}\r\n\
                 Sec-Fetch-Site: cross-site\r\nContent-Length: 2\r\n\r\n{{}}"
            );
            let (head, _) = request(&head, 4096).expect("read");
            assert!(!drivable(&head), "{content_type} was allowed to drive a machine");
        }
    }

    /// And a browser reporting `cross-site` is refused even when the content
    /// type is right — the corroborating check, doing its job.
    #[test]
    fn a_cross_site_report_is_refused_even_with_the_right_content_type() {
        let head = "POST /mcp/board HTTP/1.1\r\nContent-Type: application/json\r\n\
                    Sec-Fetch-Site: cross-site\r\nContent-Length: 2\r\n\r\n{}";
        let (head, _) = request(head, 4096).expect("read");
        assert!(!drivable(&head));
    }

    /// **The page this server ships still works**, which is the other half of
    /// the check: `page.rs` sends `Content-Type: application/json` and a browser
    /// adds `Sec-Fetch-Site: same-origin`.
    #[test]
    fn the_fleet_pages_own_request_is_allowed() {
        let (head, _) = request(CALL, 4096).expect("read");
        assert!(drivable(&head), "the shipped page's own request was refused");
    }

    /// **An engine that sends `Origin` and no `Sec-Fetch-Site` is judged on
    /// `Origin`**, against this server's own origin, port included.
    #[test]
    fn an_origin_from_another_site_is_refused_when_no_sec_fetch_site_came_with_it() {
        // `.rulings/` R-186 D1: the rule `rivet-proxyd` applies, shared.
        let hostile = "POST /mcp/board HTTP/1.1\r\nHost: rivet-01.local:4443\r\n\
                       Content-Type: application/json\r\nOrigin: https://evil.example\r\n\
                       Content-Length: 2\r\n\r\n{}";
        let (head, _) = request(hostile, 4096).expect("read");
        assert!(!drivable(&head), "an Origin from another site drove a machine");

        // This server's own page, from an engine that sends Origin alone.
        let own = "POST /mcp/board HTTP/1.1\r\nHost: rivet-01.local:4443\r\n\
                   Content-Type: application/json\r\nOrigin: https://rivet-01.local:4443\r\n\
                   Content-Length: 2\r\n\r\n{}";
        let (head, _) = request(own, 4096).expect("read");
        assert!(drivable(&head), "this server's own origin was refused");

        // The proxy's origin on 443 is a different origin from this one on 4443.
        let proxy = "POST /mcp/board HTTP/1.1\r\nHost: rivet-01.local:4443\r\n\
                     Content-Type: application/json\r\nOrigin: https://rivet-01.local\r\n\
                     Content-Length: 2\r\n\r\n{}";
        let (head, _) = request(proxy, 4096).expect("read");
        assert!(!drivable(&head), "a page on the proxy's origin drove a machine");
    }

    /// And a non-browser caller that sends no `Sec-Fetch-Site` is allowed on the
    /// content type alone — an operator's script must not have to pretend to be
    /// a browser to reach their own fleet server.
    #[test]
    fn a_non_browser_caller_needs_only_the_content_type() {
        let head = "POST /mcp/board HTTP/1.1\r\nContent-Type: application/json\r\n\
                    Content-Length: 2\r\n\r\n{}";
        let (head, _) = request(head, 4096).expect("read");
        assert!(drivable(&head));
    }

    /// Header names are case-insensitive and browsers disagree about the case
    /// they send, so a check that folded case in one place and not another
    /// would work in one browser.
    #[test]
    fn header_names_and_values_are_matched_without_regard_to_case() {
        let head = "POST /mcp/board HTTP/1.1\r\nCONTENT-TYPE: Application/JSON; charset=utf-8\r\n\
                    sec-fetch-site: Same-Origin\r\nContent-Length: 2\r\n\r\n{}";
        let (head, _) = request(head, 4096).expect("read");
        assert!(drivable(&head));
    }

    // ---- `POST /api/commit/<device>`'s dispatch and cross-site guard — R41,
    // `.rulings/` R-169. `dispatch` is what `serve_one` calls once the TLS
    // handshake and the request read are already done, so these drive it
    // directly with request text — no socket and no certificate needed —
    // exactly as `drivable` and `read_request` are already exercised above.
    //
    // **How "reached the handler" is told apart from "was refused" or
    // "fell through to the page".** Every branch answers with its own JSON-RPC
    // error CODE: `-32600` is the cross-site refusal, `-32010` is whatever
    // `fleet::load` or `commit::handle` itself said, and anything that is not
    // a `-32...` JSON-RPC error at all is the ordinary fleet page — this test
    // process has no `RIVET_APP_DATA`, so `fleet::load` itself fails before
    // `commit::handle` ever runs, but that failure still carries `-32010`
    // because `serve_one`'s `Err` arm wraps BOTH of those in one code. Which
    // of the two produced the message is not this test's question; that
    // `commit::handle` refuses correctly is `commit::tests`' job (already
    // covered) — this proves the ROUTE gets there at all.

    /// An operator, as [`admit`] would hand one to `dispatch`.
    fn test_operator() -> Operator {
        Operator {
            label: "test-peer".to_owned(),
        }
    }

    /// **The gate refuses every request from a caller that is not the
    /// operator** — each operator route, a route nothing serves, and the page —
    /// with a `403` saying why; and admits the operator to the same requests.
    /// Over real TLS, `gate_tests` drives the same through `serve_one`.
    #[test]
    fn the_gate_refuses_every_request_from_a_machine_and_admits_the_operator() {
        let requests = [
            "GET /api/fleet HTTP/1.1\r\n\r\n",
            "POST /api/commit/rivet-01 HTTP/1.1\r\nContent-Type: application/json\r\n\r\n",
            "POST /mcp/rivet-01 HTTP/1.1\r\nContent-Type: application/json\r\n\r\n",
            "GET /api/something-added-later HTTP/1.1\r\n\r\n",
            "GET / HTTP/1.1\r\n\r\n",
        ];
        let machine = Peer {
            label: "member".to_owned(),
            caller: rivetos_tls::Caller::Machine(vec!["member-01.local".to_owned()]),
        };
        let unreadable = Peer {
            label: "odd".to_owned(),
            caller: rivetos_tls::Caller::Unreadable,
        };
        let operator = Peer {
            label: "person".to_owned(),
            caller: rivetos_tls::Caller::Operator,
        };
        for request in requests {
            let refused = admit(&machine, request).err().expect("a machine is refused");
            assert!(refused.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{refused}");
            assert!(refused.contains("\"code\":-32003"), "{refused}");
            assert!(refused.contains("member-01.local"), "{refused}");

            let refused = admit(&unreadable, request).err().expect("unreadable is refused");
            assert!(refused.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{refused}");

            let admitted = admit(&operator, request).expect("the operator is admitted");
            assert_eq!(admitted.label, "person");
        }
    }

    /// **The route reaches the handler.** A same-origin, JSON-typed POST to
    /// `/api/commit/<device>` is not turned away by the cross-site guard and
    /// is not silently served the fleet page — it comes back as `-32010`,
    /// the code this server's commit branch uses for whatever `fleet::load`
    /// or `commit::handle` said.
    #[test]
    fn the_commit_route_reaches_its_handler() {
        let head = "POST /api/commit/rivet-01 HTTP/1.1\r\nContent-Type: application/json\r\n\
                    Sec-Fetch-Site: same-origin\r\nContent-Length: 2\r\n\r\n{}";
        let (head, body) = request(head, 4096).expect("read");
        let reply = dispatch(&head, body.trim(), &test_operator());
        assert!(reply.contains("\"code\":-32010"), "{reply}");
        assert!(!reply.contains("\"code\":-32600"), "the cross-site guard fired: {reply}");
        assert!(!reply.starts_with("HTTP/1.1 200 OK\r\nContent-Type: text/html"), "{reply}");
    }

    /// **A cross-site request is refused before any commit is even
    /// attempted** — same guard `/mcp` already uses, proven here against
    /// `/api/commit` specifically rather than assumed from `drivable`'s own
    /// tests. The reply is the CSRF refusal, never the commit branch's
    /// `-32010` — so a hostile page cannot learn anything about whether a
    /// repository is configured, let alone write to one.
    #[test]
    fn a_cross_site_post_to_commit_is_refused_before_any_commit_is_attempted() {
        let head = "POST /api/commit/rivet-01 HTTP/1.1\r\nContent-Type: application/json\r\n\
                    Sec-Fetch-Site: cross-site\r\nContent-Length: 2\r\n\r\n{}";
        let (head, body) = request(head, 4096).expect("read");
        let reply = dispatch(&head, body.trim(), &test_operator());
        assert!(reply.contains("\"code\":-32600"), "{reply}");
        assert!(!reply.contains("\"code\":-32010"), "the commit branch ran anyway: {reply}");
    }

    /// **Missing `Content-Type: application/json` refuses it too**, on the
    /// same footing as `/mcp` — a hostile page can only send a content type
    /// that needs no CORS preflight, and `text/plain` is one of them.
    #[test]
    fn a_commit_post_with_the_wrong_content_type_is_refused() {
        let head = "POST /api/commit/rivet-01 HTTP/1.1\r\nContent-Type: text/plain\r\n\
                    Content-Length: 2\r\n\r\n{}";
        let (head, body) = request(head, 4096).expect("read");
        let reply = dispatch(&head, body.trim(), &test_operator());
        assert!(reply.contains("\"code\":-32600"), "{reply}");
    }

    /// **A non-POST method is refused** — `GET /api/commit/<device>` matches
    /// none of `dispatch`'s `starts_with("POST ...")` branches, so it falls
    /// all the way through to the ordinary fleet page rather than reaching
    /// `commit::handle` under a method nothing declared safe for it.
    #[test]
    fn a_get_to_the_commit_route_is_not_dispatched_to_the_handler() {
        let head = "GET /api/commit/rivet-01 HTTP/1.1\r\nContent-Length: 0\r\n\r\n";
        let (head, body) = request(head, 4096).expect("read");
        let reply = dispatch(&head, body.trim(), &test_operator());
        assert!(!reply.contains("\"code\":-32010"), "a GET reached the commit handler: {reply}");
        assert!(!reply.contains("\"code\":-32600"), "a GET tripped the mutation guard: {reply}");
        assert!(
            reply.starts_with("HTTP/1.1 200 OK\r\nContent-Type: text/html"),
            "a GET should fall through to the ordinary page: {reply}"
        );
    }

    /// **The limits hold and a finished connection gives its slot back**: one
    /// source stops at its share, another still gets the reserved slots, then
    /// the pool is full for anyone. One test, because the pool is process-wide —
    /// see the same argument in `rivet-mcpd`.
    #[test]
    fn connections_are_capped_per_source_and_in_total_and_released() {
        use rivet_core::connection_slots::{PER_SOURCE_SLOTS, RESERVED};
        let one = std::net::IpAddr::from([192, 168, 50, 10]);
        let other = std::net::IpAddr::from([192, 168, 50, 11]);
        let third = std::net::IpAddr::from([192, 168, 50, 12]);

        let mut held: Vec<Slot> = (0..PER_SOURCE_SLOTS)
            .map(|n| Slot::claim(one).unwrap_or_else(|_| panic!("slot {n}")))
            .collect();
        assert_eq!(Slot::claim(one).err(), Some(Full::PerSource), "one source took more than its share");
        for n in 0..RESERVED {
            held.push(Slot::claim(other).unwrap_or_else(|_| panic!("reserved slot {n}")));
        }
        assert_eq!(Slot::claim(third).err(), Some(Full::Global), "the pool did not hold");
        assert_eq!(held.len(), MAX_CONNECTIONS);

        drop(held.remove(0));
        let one_more = Slot::claim(one).expect("the released slot came back");
        assert!(Slot::claim(one).is_err(), "releasing one let two in");
        drop(one_more);
        drop(held);
        assert_eq!(slots().live(), 0, "a slot leaked");
        assert!(slots().is_empty(), "a source entry leaked");
    }
}
