//! `rivet-proxyd` — **one name and one certificate, for every application.**
//!
//! `R-133`. `https://<host>/reliquary/api/v1/blobs` reaches whichever
//! application declared `proxy-path = "reliquary"`, on the loopback port its
//! manifest declared.
//!
//! # It IS the machine's authentication gate — `R-152`, `ADR-046` §4
//!
//! `R-133` left one question open and called it the one with real money on it:
//! whether this proxy should check the operator's client certificate on behalf
//! of the applications behind it. **`R-152` ruled that it should**, because the
//! alternative was structurally excluded: path routing requires reading the
//! request line, reading it requires terminating TLS, and whoever terminates
//! TLS is who the client certificate was presented to.
//!
//! So this program **requires a certificate chaining to the fleet authority on
//! every connection**, and refuses a caller that presents none — except on a
//! route whose signed manifest declared `proxy-auth = "none"` (`ADR-046` §4.3).
//! The requirement is enforced **per route rather than in the handshake**, and
//! that is not a weakening: the route is not known until the request line has
//! been read, which is after the handshake by definition. `rustls` is asked for
//! a certificate, verifies one that is presented against the fleet authority,
//! and tells this program whether the peer authenticated;
//! [`rivet_sdk::serve::Tls::asking_for_a_caller_certificate_from`] is that
//! arrangement and says at its definition why almost nothing else should use
//! it.
//!
//! **The operator is a certificate that names no host** (`.rulings/` R-187
//! D1). The fleet authority signs every fleet machine's front door as well as
//! the operator's browser, so a chain that verified is not enough on its own:
//! [`Presented::of`] asks `rivetos_tls::classify_caller`, and a certificate
//! naming a host is refused wherever the operator is required and served only
//! where no certificate is.
//!
//! **It does not authorise.** It establishes that a caller holds an operator
//! certificate. What that operator may do is the application's question, and
//! *which* operator called is not forwarded — `ADR-046` §4.4, and there is one
//! operator today.
//!
//! # What it does not do, and why each is deliberate
//!
//! **It does not keep connections alive.** Every hop is `Connection: close` in
//! both directions, which is slower and is what makes the forwarding correct
//! without a response parser: once a request head is accepted, this program
//! copies bytes in both directions until they stop, and a response's own
//! chunked encoding, trailers and content length pass through untouched
//! because nothing on that side interprets them.
//!
//! **The request head is a different matter, and is refused rather than
//! forwarded when its framing is ambiguous** — RFC 9112 §6.3, `.rulings/`
//! R-210 D17. A chunked request body is refused with `411`, because nothing
//! behind this proxy decodes `Transfer-Encoding: chunked` and forwarding the
//! encoded bytes with the header removed would hand a backend a body with no
//! framing left at all. `Transfer-Encoding` beside `Content-Length`, a
//! `Transfer-Encoding` naming anything but exactly `chunked`, and a repeated
//! `Content-Length` — matching values included — are each refused with `400`:
//! every one of them is a request this proxy and the backend behind it could
//! read as having a different length, which is the request-smuggling
//! precondition. **`Upgrade` is stripped as a hop-by-hop header in
//! `rewrite_head`**, so a WebSocket cannot pass through this proxy until
//! WebSockets through the authentication gate are designed.
//!
//! **It does not rewrite anything but the request line.** An application
//! mounted at `/reliquary` that emits an absolute URL of its own emits a wrong
//! one, and no amount of body rewriting fixes that in general. `R-133` records
//! the answer: the declaration is two-sided, and the application has to be TOLD
//! its base so it can emit correct URLs itself. That half is not built here.
//!
//! # Both sides are TLS, and both sides authenticate
//!
//! Reliquary is HTTPS-only by design — `SPEC-01` §4 has no plaintext listener
//! and no toggle for one — so a proxy that assumed plaintext backends could not
//! reach it at all. Both ends chain to the fleet authority, which is what makes
//! verifying the backend cheap rather than a new trust decision.
//!
//! **And this proxy presents its own certificate to the backend, so the backend
//! can verify it** — `ADR-046` §4.2, `R-152.1`. That is what contains the cost
//! `R-133` named: an application behind this proxy trusts the proxy's word
//! about who called, so a local process must not be able to *be* the proxy.
//! `ADR-024` §8.1 is honest that the sandbox is not enforced, so *"the
//! connection came from 127.0.0.1"* is not an authentication claim — but *"the
//! caller holds a key chaining to the fleet authority"* is, and it is the same
//! claim this proxy's own callers make.
//!
//! **What a backend must check, exactly** — this is the other half of the
//! contract and it lives here because this is the half that is built:
//!
//! 1. Require a client certificate chaining to the fleet authority
//!    (`R-129`) — `rivet_sdk::serve::Tls::handed(…)
//!    .requiring_a_caller_certificate_from("tls-root")`, which is one line and
//!    already in the SDK. **`requiring`, not `asking_for`**: the `asking_for`
//!    constructor exists for this program alone.
//! 2. Serve a certificate for the name the proxy verifies it against — the
//!    machine's one fleet name, the `upstream-name` setting below. A backend
//!    presenting a certificate for anything else is refused by this program at
//!    the handshake, and the refusal is per request rather than at startup.
//! 3. **Do not check WHICH certificate.** There is one operator and the
//!    forwarded identity is not built (`ADR-046` §4.4); a backend that pinned
//!    the proxy's leaf would break the day a certificate was reissued and would
//!    be asserting something this design does not promise.
//!
//! A backend that does none of this is not broken — it accepts the proxy — it
//! is merely accepting anything else on its loopback port too.

use std::sync::Arc;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

/// **Passkeys beside client certificates** — `ADR-063` §4, `ADR-046` §4.
mod passkey;

/// The manifest's socket, as `LISTEN_FDNAMES` spells it.
///
/// **443 since 2026-09-05** — `R-152.1`, `ADR-046` §7. `rivet-fleetd` held it
/// and moved to 4443; this is the machine's front door and the only port a
/// browser is asked to know.
const LISTEN_NAME: &str = "tcp-443";

/// **The plain-HTTP socket, handed over only when the routing table carries a
/// `plain:` rule** — `ADR-061` §7, `ADR-046` §7, `.rulings/` R-221.
///
/// `rivet_sdk::listen::socket(LISTEN_NAME_PLAIN)` answers `None` on every
/// other machine, which is the whole mechanism: this program never decides
/// whether port 80 exists, it only serves the descriptor it was handed or
/// serves nothing at all.
const LISTEN_NAME_PLAIN: &str = "tcp-80";

/// The certificate this proxy serves, in its own data directory.
const TLS_CERT: &str = "tls-cert";
/// Its key.
const TLS_KEY: &str = "tls-key";
/// The fleet authority, used to verify the applications behind it.
const TLS_ROOT: &str = "tls-root";

/// The setting naming the certificate the BACKENDS present.
///
/// Every application on this machine is served the same fleet certificate, for
/// the machine's one name — so the proxy connects to `127.0.0.1:<port>` and
/// verifies the name it was told rather than the address it dialled. That is
/// the ordinary arrangement for a reverse proxy. With no setting, the name is
/// the first DNS name on this proxy's own certificate — see [`upstream_name`] —
/// and never one composed from the hostname: appending `.local` to
/// `/proc/sys/kernel/hostname` would be assuming the mDNS suffix, which
/// `ADR-034` makes a fleet property.
const UPSTREAM_NAME: &str = "upstream-name";

/// The largest request head this will read before giving up.
///
/// A bound exists because the bytes come from whoever connected. 32 KiB is
/// several times any real header block and small enough that a client cannot
/// make this program allocate by talking slowly.
const MAX_HEAD: usize = 32 * 1024;

/// **How long a client has at each step before its request is in hand** —
/// `ADR-046` §10.
///
/// Three deadlines rather than one, because one step waits on a person. A
/// browser sends its `ClientHello` the moment it connects, so a client that has
/// sent nothing after [`Patience::first_byte`] is silent and is dropped. The
/// handshake itself can pause while the browser asks its user which client
/// certificate to present, so it keeps the longer [`Patience::handshake`],
/// counted from the accept. Once the handshake is done the request head follows
/// at once, so [`Patience::head`] is short again, counted from the handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Patience {
    /// From the accept to the first byte the client sends.
    first_byte: std::time::Duration,
    /// From the accept to a completed TLS handshake.
    handshake: std::time::Duration,
    /// From the completed handshake (or, on port 80, the accept) to a complete
    /// request head.
    head: std::time::Duration,
}

/// **The most connections this proxy holds at once**, across both listeners —
/// `ADR-046` §10.
///
/// Sixty-four, four times the control plane's sixteen, because the control
/// plane is sized for operators and this proxy fronts every application for
/// every device on the LAN. One source may hold
/// `rivet_core::connection_slots::PER_SOURCE_SLOTS` of them, as at every
/// listener.
const PROXY_CONNECTIONS: usize = 64;

/// The governor both listeners weigh every connection with, before it costs a
/// task or a log line: the shared per-source rate, the pool and one source's
/// share.
fn governor() -> rivet_core::connection_slots::Governor<std::net::IpAddr> {
    rivet_core::connection_slots::Governor::new(
        PROXY_CONNECTIONS,
        rivet_core::connection_slots::PER_SOURCE_SLOTS,
    )
}

/// **Weigh one accepted connection**: its slot, or `None` to drop it. A refusal
/// is journalled at most once per source per
/// `rivet_core::connection_slots::REPORT_EVERY_MS`, carrying the count, because a
/// line per refused connection would be the flood written into the journal.
fn admit(
    governor: &rivet_core::connection_slots::Governor<std::net::IpAddr>,
    peer: std::net::SocketAddr,
    journal: &rivet_sdk::journal::Journal,
) -> Option<rivet_core::connection_slots::SlotGuard<std::net::IpAddr>> {
    match governor.arrive(peer.ip(), now_ms()) {
        rivet_core::connection_slots::Outcome::Admit(slot) => Some(slot),
        rivet_core::connection_slots::Outcome::Refuse { reason, report } => {
            if let Some(dropped) = report {
                journal.warn(&format!(
                    "{dropped} connection(s) from {} refused: {} — at most one line per {}s",
                    peer.ip(),
                    reason.describe(),
                    rivet_core::connection_slots::REPORT_EVERY_MS / 1000,
                ));
            }
            None
        }
    }
}

/// Milliseconds on the monotonic clock since the first call, for the governor.
fn now_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(std::time::Instant::now);
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// The deadlines every listener of this program uses, before the document's
/// `handshake-seconds` replaces [`Patience::handshake`] (`ADR-046` §10).
const PATIENCE: Patience = Patience {
    first_byte: std::time::Duration::from_secs(3),
    handshake: std::time::Duration::from_secs(15),
    head: std::time::Duration::from_secs(3),
};

impl Patience {
    /// One duration for every step, for a test that wants a stall caught fast.
    #[cfg(test)]
    const fn uniform(each: std::time::Duration) -> Self {
        Self { first_byte: each, handshake: each, head: each }
    }
}

fn main() -> std::process::ExitCode {
    let journal = rivet_sdk::journal::Journal::for_service("rivet-proxyd");

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(why) => {
            journal.error(&format!("no runtime: {why}"));
            return std::process::ExitCode::FAILURE;
        }
    };

    match runtime.block_on(serve(&journal)) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(why) => {
            journal.error(&why);
            eprintln!("rivet-proxyd: {why}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn serve(journal: &rivet_sdk::journal::Journal) -> Result<(), String> {
    let data = rivet_sdk::app::data()
        .map_err(|_| String::from("RIVET_APP_DATA is not set; this was not started by a Rivet"))?;

    // **The table, and an empty one is not an error.** A machine where nothing
    // has declared a `proxy-path` yet is the ordinary machine, and a proxy that
    // refused to start there would be a service an operator has to remember to
    // enable second. It serves 404 to everything and says so once.
    // **Beside `data`, not inside it.** `rivet_core::proxy::path` puts the
    // table in the root-owned `<volume>/app/<name>` directory rather than in
    // the `0700` one this process owns — `ADR-038` §8.1's move, applied to this
    // file — so a program running as this uid cannot rewrite its own routes.
    // The parent of `RIVET_APP_DATA` is that directory; deriving it is legal
    // here for the reason this crate links `rivet-core` at all (see its
    // manifest): this is Rivet's own machinery that happens to ship as an
    // application, not a foreign program.
    let table = data
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", data.display()))?
        .join(rivet_core::proxy::FILE);
    let (routes, admin_portal) = match std::fs::read_to_string(&table) {
        // **The portal is read from the same bytes as the routes**, so the page
        // offers a `rivet-fleetd` row exactly when the table `rivet-sysd`
        // derived from installed manifests named one — a fleet server, never a
        // member (`rivet_core::proxy::admin_portal`).
        Ok(text) => (
            rivet_core::proxy::parse(&text),
            rivet_core::proxy::admin_portal(&text),
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Vec::new(), None),
        Err(e) => return Err(format!("the routing table {} could not be read: {e}", table.display())),
    };

    // **When the supervisor last derived the table**, taken from the file this
    // process just read rather than from the clock now — the two differ by
    // however long this proxy has been up, and the page is answering *how old
    // is this list*.
    //
    // **Read once, here, and never again.** The routes are read once at start
    // too (see the module note), so a timestamp refreshed per request would
    // drift away from the list it describes and claim a freshness the page
    // does not have.
    let derived_at = table
        .metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        // `human` and not `rfc3339`: a person reads this page, and
        // `2026-09-06T11:41:43.812Z` is four pieces of precision nobody wanted
        // with a `T` in the middle of a sentence.
        .map(|since| rivet_core::journal::human(since.as_millis().try_into().unwrap_or(u64::MAX)));

    if routes.is_empty() {
        journal.warn(
            "no routes: nothing on this machine declares a proxy-path, or the table has not been \
             delivered yet. Every request will be answered 404.",
        );
    }
    for route in &routes {
        // Typed fields rather than a sentence: `rivet get log` can then filter
        // on `service` and `port` as values, which is the whole reason
        // `ADR-022` has a field map at all.
        journal.emit(
            rivet_sdk::journal::Severity::Info,
            "routing",
            &[
                ("path", rivet_sdk::journal::Value::Str(route.path.clone())),
                (
                    "service",
                    rivet_sdk::journal::Value::Str(route.service.clone()),
                ),
                ("port", rivet_sdk::journal::Value::Int(i64::from(route.port))),
                // **A typed field rather than a sentence**, so `rivet get log`
                // can answer *which routes on this machine are public* as a
                // filter rather than as a grep.
                (
                    "operator",
                    rivet_sdk::journal::Value::Str(
                        if route.requires_operator { "required" } else { "not required" }
                            .to_string(),
                    ),
                ),
            ],
        );
    }

    // **Said once, loudly, at the top of every start** — `ADR-046` §4.3. A
    // route served without an operator certificate is a signed declaration
    // nobody can turn off on this machine, so the journal saying so is the only
    // thing between it and a public route nobody remembers agreeing to.
    let public: Vec<&str> = routes
        .iter()
        .filter(|r| !r.requires_operator)
        .map(|r| r.path.as_str())
        .collect();
    if public.is_empty() {
        journal.info(
            "every route requires an operator certificate issued by this machine's fleet \
             authority (ADR-046 §4.1)",
        );
    } else {
        journal.warn(&format!(
            "these routes are served to callers presenting NO operator certificate, because \
             their signed manifests declare proxy-auth = \"none\" (ADR-046 §4): /{}. \
             It takes a new artifact to change that; there is no setting for it",
            public.join("/, /")
        ));
    }

    // **And the open PREFIXES, named the same way** — `ADR-048` §4. An open
    // prefix on an otherwise gated route is the one this warning exists for:
    // the route reads as authenticated everywhere it is summarised, and the
    // door is a line in a signed manifest nobody re-reads.
    let doors: Vec<String> = routes
        .iter()
        .flat_map(|route| {
            route
                .open_prefixes()
                .into_iter()
                .map(|prefix| format!("/{}/{prefix}", route.path))
                .collect::<Vec<_>>()
        })
        .collect();
    if !doors.is_empty() {
        journal.warn(&format!(
            "these PREFIXES are served to callers presenting NO operator certificate, because \
             their applications' signed manifests declare proxy-auth-paths (ADR-048 §4): {}. \
             The routes they sit under are otherwise gated. It takes a new artifact to change \
             that; there is no setting for it",
            doors.join(", ")
        ));
    }

    let cert_pem = read_pem(&data, TLS_CERT)?;
    let (upstream_name, from) = upstream_name(rivet_sdk::settings::get(UPSTREAM_NAME), &cert_pem)?;
    journal.info(&format!(
        "verifying the applications behind this proxy as {upstream_name:?}, taken from {from}"
    ));

    // **Whether the operator's client certificate still admits a caller** —
    // `ADR-046` §4, `.rulings/` R-234. Absent is accepted, so a machine whose
    // document says nothing serves exactly as it always has.
    let (certificate, misspelt) = rivet_core::proxy::operator_certificate(
        rivet_sdk::settings::get(rivet_core::proxy::OPERATOR_CERTIFICATE_SETTING).as_deref(),
    );
    if let Some(why) = misspelt {
        journal.error(&why);
    }

    // **How long a client has to finish its handshake** — `ADR-046` §10. The
    // document may lengthen or shorten it; absent, or unusable, is the default,
    // and the front door still starts.
    let (handshake_seconds, unusable) = rivet_core::proxy::handshake_seconds(
        rivet_sdk::settings::get(rivet_core::proxy::HANDSHAKE_SECONDS_SETTING).as_deref(),
    );
    if let Some(why) = unusable {
        journal.warn(&why);
    }
    let patience = Patience {
        handshake: std::time::Duration::from_secs(handshake_seconds),
        ..PATIENCE
    };
    if certificate == rivet_core::proxy::OperatorCertificate::Refused {
        journal.warn(&format!(
            "the operator's client certificate is NOT accepted on this machine: its document sets \
             `{}`, so no client certificate is asked for and only a passkey session admits \
             the operator at the gate (ADR-046 §4). Sign in at {}",
            rivet_core::proxy::OPERATOR_CERTIFICATE_SETTING,
            passkey::SIGN_IN_PAGE,
        ));
    }

    let server = server_config(&data, certificate)?;
    let client = client_config(&data, &upstream_name)?;

    // **A relying party at each name the certificate carries** — `ADR-063`
    // §4. The names are read off the same certificate the proxy serves, so a
    // browser that reached this machine by a name without a certificate
    // warning reached it by one of these. A passkey is bound to the name it
    // was enrolled at; one enrolled before names were recorded belongs to
    // `upstream-name`, which was then the only relying party.
    //
    // A certificate whose names cannot be read leaves no name to answer at,
    // which refuses every ceremony and nothing else: the certificate path
    // and every route are unaffected, so it is journalled rather than fatal.
    let names = certificate_names(&cert_pem).unwrap_or_else(|why| {
        journal.error(&format!("no passkey ceremony can be answered: {why}"));
        Vec::new()
    });
    let passkeys = Arc::new(passkey::Passkeys::new(&data, &names, &upstream_name, journal.clone()));
    journal.info(&format!(
        "passkeys are accepted beside client certificates at each name this machine's \
         certificate carries ({}); a passkey belongs to the name it was enrolled at. Sign in at \
         {login}, enroll at {enroll} (ADR-063 §4)",
        names.join(", "),
        login = passkey::SIGN_IN_PAGE,
        enroll = passkey::ENROLL_PAGE,
    ));

    let listener = rivet_sdk::listen::socket(LISTEN_NAME).ok_or_else(|| {
        let handed: Vec<String> = rivet_sdk::listen::sockets()
            .into_iter()
            .map(|s| s.name)
            .collect();
        format!("no socket named {LISTEN_NAME:?} was handed over; got {handed:?}")
    })?;
    if !listener.is_tcp() {
        return Err(format!("the socket {LISTEN_NAME:?} is not a TCP one"));
    }
    let listener = listener.into_tcp_listener().ok_or_else(|| {
        format!("the descriptor named {LISTEN_NAME:?} was already taken inside this process")
    })?;
    let listener = adopt(listener, LISTEN_NAME)?;

    journal.info(&format!(
        "serving {} route(s) on the descriptor the supervisor handed over",
        routes.len()
    ));
    println!("rivet-proxyd: serving {} route(s)", routes.len());

    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server));
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client));
    let routes = Arc::new(routes);
    let admin_portal = Arc::new(admin_portal);
    let upstream_name = Arc::new(upstream_name);
    let derived = Arc::new(derived_at);
    // One governor for both listeners, so the pool is the program's.
    let governor = governor();
    journal.info(&format!(
        "at most {PROXY_CONNECTIONS} connections at once, {} per source address; {}s to send a \
         first byte, {}s to finish a handshake, {}s for the request head after it",
        rivet_core::connection_slots::PER_SOURCE_SLOTS,
        patience.first_byte.as_secs(),
        patience.handshake.as_secs(),
        patience.head.as_secs(),
    ));

    // **Port 80, and only when the supervisor handed one over** — `ADR-061`
    // §7, `.rulings/` R-221. `rivet-sysd` binds it only when the routing
    // table carries a `plain:` rule; this program never decides that for
    // itself, it only asks whether the descriptor exists. Every plain route is
    // journalled here, by name, for `ADR-048` §4's reason: a signed
    // declaration nobody can turn off on the machine is worth having only if
    // somebody can see it.
    let plain_prefixes: Vec<String> = routes
        .iter()
        .flat_map(|route| {
            route
                .plain_prefixes()
                .into_iter()
                .map(|prefix| format!("/{}/{prefix}", route.path))
                .collect::<Vec<_>>()
        })
        .collect();
    match rivet_sdk::listen::socket(LISTEN_NAME_PLAIN) {
        Some(handed) => {
            if !handed.is_tcp() {
                return Err(format!("the socket {LISTEN_NAME_PLAIN:?} is not a TCP one"));
            }
            let plain_listener = handed.into_tcp_listener().ok_or_else(|| {
                format!("the descriptor named {LISTEN_NAME_PLAIN:?} was already taken inside this process")
            })?;
            let plain_listener = adopt(plain_listener, LISTEN_NAME_PLAIN)?;
            journal.warn(&format!(
                "serving plain HTTP on port 80 (GET and HEAD only, no TLS): these prefixes are \
                 reachable with no certificate at all, because their signed manifests declare \
                 proxy-auth-paths plain: (ADR-061 §7): {}",
                if plain_prefixes.is_empty() {
                    "(none recorded, but the descriptor was handed over)".to_string()
                } else {
                    plain_prefixes.join(", ")
                }
            ));
            let plain_connector = connector.clone();
            let plain_routes = Arc::clone(&routes);
            let plain_upstream_name = Arc::clone(&upstream_name);
            let plain_journal = journal.clone();
            let plain_governor = governor.clone();
            tokio::spawn(async move {
                serve_plain(
                    plain_listener,
                    plain_connector,
                    plain_routes,
                    plain_upstream_name,
                    plain_journal,
                    patience.head,
                    plain_governor,
                )
                .await;
            });
        }
        None => {
            if !plain_prefixes.is_empty() {
                // **The version skew this cannot fix by itself.** A route
                // declares `plain:` but no descriptor arrived, which means the
                // supervisor has not derived a table with this route in it
                // yet, or predates the verdict. Said once so an operator does
                // not conclude nothing is wrong.
                journal.warn(&format!(
                    "these prefixes declare proxy-auth-paths plain:, but no port-80 descriptor \
                     was handed over, so they are unreachable over plain HTTP until the \
                     supervisor binds one: {}",
                    plain_prefixes.join(", ")
                ));
            }
        }
    }

    let mut stopping = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|e| format!("cannot listen for SIGTERM: {e}"))?;

    loop {
        let accepted = tokio::select! {
            () = async { stopping.recv().await; } => {
                journal.info("SIGTERM received; no longer accepting");
                return Ok(());
            }
            accepted = listener.accept() => accepted,
        };

        let Ok((stream, peer)) = accepted else {
            // One failed accept is not a reason to stop serving every other
            // application on the machine.
            continue;
        };
        let Some(slot) = admit(&governor, peer, journal) else {
            continue;
        };

        let acceptor = acceptor.clone();
        let connector = connector.clone();
        let routes = Arc::clone(&routes);
        let admin_portal = Arc::clone(&admin_portal);
        let upstream_name = Arc::clone(&upstream_name);
        let derived = Arc::clone(&derived);
        let passkeys = Arc::clone(&passkeys);
        let journal = journal.clone();
        tokio::spawn(async move {
            // Held for the life of the connection; its `Drop` gives the slot
            // back on every way out of this task.
            let _slot = slot;
            let _ = handle(
                stream,
                &acceptor,
                &connector,
                &routes,
                (*admin_portal).as_ref(),
                &upstream_name,
                derived.as_deref(),
                &passkeys,
                &journal,
                patience,
                certificate,
            )
            .await;
        });
    }
}

/// **A descriptor the supervisor handed over, as a tokio listener.**
///
/// The supervisor hands a socket over blocking, and `rivet_sdk::listen`
/// leaves it so (its own note says why an async caller must change it). A
/// blocking listener under tokio parks the worker that accepts on it inside
/// `accept(2)`, and the task each accepted connection spawns waits in that
/// worker's own slot, which no other worker takes, until the next connection
/// arrives. Port 80's accept loop is a spawned task, so without this its
/// first request is accepted and never answered.
///
/// # Errors
///
/// The descriptor's mode could not be set, or tokio would not adopt it; either
/// names `name`.
fn adopt(listener: std::net::TcpListener, name: &str) -> Result<TcpListener, String> {
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("the handed descriptor {name:?} cannot be made non-blocking: {e}"))?;
    TcpListener::from_std(listener)
        .map_err(|e| format!("the handed descriptor {name:?} is not usable: {e}"))
}

/// **What this proxy does with one request**, decided before any of it
/// touches a socket.
///
/// Separated from [`handle`] so that the gate is testable without a
/// handshake — and, more usefully, so that the three answers are in one `match`
/// a reader can check against `ADR-046` §4 rather than spread through an async
/// function.
#[derive(Debug, PartialEq, Eq)]
enum Decision<'a> {
    /// Reach the application, with the remainder of the path.
    Forward(&'a rivet_core::proxy::Route, String),
    /// Draw the index — `ADR-046` §6.
    Index,
    /// **Send the browser one character along**, to the same prefix with a
    /// trailing slash. See [`decide`]'s note; the string is the `Location`.
    Redirect(String),
    /// Answer the caller directly, and never reach an application.
    Refuse(u16, &'static str),
    /// **A certificate the fleet authority signed that is not the operator's**
    /// — one that names a host, `.rulings/` R-187 D1. Answered `403` with
    /// `rivetos_tls::Caller::refusal`, journalled, and never forwarded. A
    /// variant of its own because the words depend on the certificate, and
    /// because a machine's key at the operator gate is worth a record.
    RefuseNotOperator,
    /// **A write another site caused** — `ADR-046` §4, `.rulings/` R-186 D1.
    /// Answered `403` with [`CROSS_SITE_REFUSAL`], journalled, and never
    /// forwarded. A variant of its own rather than one more `Refuse`, because
    /// it is the one refusal [`handle`] records and the one whose request
    /// carries a body by definition.
    RefuseCrossSite,
    /// **The proxy's own passkey pages and ceremonies**, under the reserved
    /// `/.rivet/` — `ADR-063` §4. Served to anybody: signing in is how a
    /// caller with no certificate becomes the operator. The string is the path
    /// after the prefix.
    Passkey(String),
    /// **The sign-in and enrollment pages**, under `/sign-in` — `ADR-046` §4.
    /// Served to anybody, for `Passkey`'s reason. The string is the path after
    /// `/sign-in`.
    SignIn(String),
    /// **An address that moved for good**: answered `308 Permanent Redirect`
    /// to this path, the query string carried across. The old `/.rivet/` page
    /// addresses, which bookmarks and notes still name.
    Moved(String),
}

/// What the caller presented, as far as the gate is concerned.
///
/// **`rustls` decides whether a certificate verified; this decides whether it
/// is the operator's.** A verified chain only says the fleet authority signed
/// the key, and the authority signs every fleet machine's front door as well as
/// the operator's browser. `rivetos_tls::classify_caller` tells them apart: the
/// operator's names no host (`.rulings/` R-187 D1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Presented {
    /// No certificate.
    Nothing,
    /// A verified certificate that names no host.
    Operator,
    /// A verified certificate that names a host, or could not be read far
    /// enough to tell.
    NotOperator,
    /// **No operator certificate, and a live passkey session** — `ADR-063`
    /// §4. The gate's second way in: admitted wherever
    /// [`Self::Operator`] is.
    Passkey,
}

impl Presented {
    /// Classify what `rustls` verified: `None` is no certificate.
    fn of(caller: Option<&rivetos_tls::Caller>) -> Self {
        match caller {
            None => Self::Nothing,
            Some(caller) if caller.is_operator() => Self::Operator,
            Some(_) => Self::NotOperator,
        }
    }
}

/// **The gate, and the whole of `ADR-046` §4.**
///
/// `presented` is what the peer presented: nothing, the operator's certificate,
/// or another certificate the fleet authority signed. `rustls` answers whether a
/// chain verified, and [`Presented::of`] whether it is the operator's.
///
/// `cross_site_write` is `rivet_core::cross_site::refuses` asked of the request
/// head: a method other than `GET`, `HEAD` or `OPTIONS`, from a browser that
/// says a page on another site caused it.
fn decide<'a>(
    routes: &'a [rivet_core::proxy::Route],
    path: &str,
    presented: Presented,
    cross_site_write: bool,
) -> Decision<'a> {
    // **The operator gate's answer, for the index and for every gated route
    // alike**, so the two cannot disagree about who the operator is.
    let refused = match presented {
        // **Either a client certificate or a passkey session** — `ADR-063`
        // §4. The certificate path is unchanged; a session is the second key
        // to the same door.
        Presented::Operator | Presented::Passkey => None,
        Presented::Nothing => Some(Decision::Refuse(403, GATE_REFUSAL)),
        Presented::NotOperator => Some(Decision::RefuseNotOperator),
    };

    // **A path this proxy would have to normalise is refused, first, before
    // anything looks at a route** — `ADR-048` §5.
    //
    // `/reliquary/pair/../blobs` matches an open prefix at `pair` segment by
    // segment, and a backend that resolves the dot segment then serves
    // `/blobs`. The gate and the server would be answering about two different
    // paths, and the gate's would be the wrong one. Normalising here instead
    // would mean agreeing with every backend about a normalisation nobody
    // wrote down.
    //
    // **Before the route lookup and before the gate**, so it binds a gated
    // route as well as an open one: a traversal that today's verdict happens to
    // refuse would otherwise become live the day that route relaxed.
    if !rivet_core::proxy::is_traversal_free(path) {
        return Decision::Refuse(400, TRAVERSAL_REFUSAL);
    }

    // **A write another site caused is refused next, for every application and
    // whoever presented what** — `.rulings/` R-186 D1.
    //
    // A browser holding the operator's certificate presents it to any request
    // to this machine, whichever page caused it, and a page on another site can
    // `POST` a `text/plain` body with no preflight. Without this, that page
    // replaces an application's data through its restore route with the
    // operator's own identity. Applications behind this proxy rely on it and
    // carry no `Content-Type` rule of their own (`ADR-051` §12).
    //
    // **Before the index, the route lookup and the gate**, like the traversal
    // guard and for its reason: it is a property of the request, so it binds
    // gated and public routes alike, and a public route relaxing its gate does
    // not relax this. It answers before the redirect too, so a `308` never
    // invites the browser to send the body again.
    if cross_site_write {
        return Decision::RefuseCrossSite;
    }

    // **The reserved passkey paths, before the gate and the route lookup** —
    // `ADR-063` §4. No application can be mounted here, because a
    // `proxy-path` must begin with a lowercase letter; and they must be
    // reachable with no credential at all, because they are how one is
    // obtained. After the two guards above, so a traversal or a write another
    // site caused is refused here as everywhere.
    //
    // **The pages a person visits come first**: `/sign-in`, which no
    // application can claim either (`RESERVED_PROXY_PATHS`), and the two old
    // addresses under the hidden prefix, which move there for good.
    if path == passkey::SIGN_IN_PAGE {
        return Decision::SignIn(String::new());
    }
    if let Some(rest) = path.strip_prefix(passkey::SIGN_IN_PAGE)
        && rest.starts_with('/')
    {
        return Decision::SignIn(rest.to_string());
    }
    for (old, new) in [
        (passkey::OLD_SIGN_IN_PAGE, passkey::SIGN_IN_PAGE),
        (passkey::OLD_ENROLL_PAGE, passkey::ENROLL_PAGE),
    ] {
        if path.strip_prefix(old).is_some_and(|rest| rest.is_empty() || rest == "/") {
            return Decision::Moved(new.to_string());
        }
    }
    if path == passkey::PREFIX {
        return Decision::Passkey(String::new());
    }
    if let Some(rest) = path.strip_prefix(passkey::PREFIX)
        && rest.starts_with('/')
    {
        return Decision::Passkey(rest.to_string());
    }

    // **The index is behind the gate like everything else** — `ADR-046` §6.
    // The list of what a machine runs is not public.
    if path == "/" {
        return refused.unwrap_or(Decision::Index);
    }

    let Some((route, rest)) = rivet_core::proxy::route_for(routes, path) else {
        // **404 before the gate, and it is the right order.** The alternative
        // leaks nothing either way: a caller with no certificate learns only
        // that *some* route exists at that prefix, which the index would tell
        // them anyway once they had a certificate. Answering 403 for a path
        // nothing serves would instead tell an authenticated operator that
        // their typo was a permission problem.
        return Decision::Refuse(404, "no application on this machine is mounted at that path");
    };

    // **Per prefix, not per application** — `ADR-048` §4. The route's own
    // verdict is the answer where no prefix under it says otherwise, which is
    // what keeps every manifest written before that ADR meaning what it meant.
    //
    // **A certificate that names a host passes only where no certificate would**
    // (`.rulings/` R-187 D1): a public route or an open prefix serves anybody,
    // a machine included, and a gated one serves the operator alone.
    if route.requires_operator_for(&rest)
        && let Some(refusal) = refused
    {
        return refusal;
    }

    // **`/notes` is redirected to `/notes/`, and this is not tidiness.**
    //
    // `ADR-046` §8's first bullet says an application mounted at a prefix
    // cannot emit an absolute URL of its own, because `R-133`'s two-sided
    // declaration is unbuilt and nothing tells it its base path. The only
    // answer available to an application is **relative** links — and a relative
    // link resolves against the browser's base, which is the request URL up to
    // its last `/`. At `/notes/` that base is `/notes/` and `style.css`
    // means `/notes/style.css`; at `/notes` the base is `/` and the same
    // markup asks the machine root for a stylesheet nothing serves.
    //
    // Without this the prefix is one missing character away from a page with
    // no styling and no working button: an enormous logo on an unstyled page
    // (.rulings/ R-342). Forwarding would answer `200` to a request that could
    // not work.
    //
    // **After the gate, deliberately.** An unauthenticated caller gets the same
    // `403` here as at `/notes/`, so the two spellings reveal exactly the
    // same thing.
    //
    // **308 and not 301**: it preserves the method and the body, so a `POST` to
    // the bare prefix arrives as a `POST`. A 301 is widely implemented as
    // *retry it as a GET*, which would turn one missing slash into a silently
    // discarded form.
    if rest == "/" && !path.ends_with('/') {
        return Decision::Redirect(format!("/{}/", route.path));
    }
    Decision::Forward(route, rest)
}

/// What a caller with no operator certificate is told.
///
/// **It says what to present rather than only that something was missing.** The
/// reader is an operator at a browser that declined a certificate prompt, and
/// *Forbidden* alone sends them looking for a password.
const GATE_REFUSAL: &str = "this machine requires an operator certificate issued by its fleet \
                            authority or a passkey session, and neither was presented. Sign in \
                            with a passkey at /sign-in, or install the fleet client \
                            certificate in this browser and reload";

/// What a caller with no passkey session is told **on a machine whose document
/// retired the operator's client certificate** — `ADR-046` §4, `.rulings/`
/// R-234. [`GATE_REFUSAL`] would tell them to install the certificate, which
/// this machine no longer asks for.
const CERTIFICATE_RETIRED_REFUSAL: &str = "this machine no longer accepts the operator's client \
                                           certificate: its document sets operator-certificate = \
                                           \"refuse\", so it does not ask for one, and only a \
                                           passkey session admits the operator. Sign in with a \
                                           passkey at /sign-in";

/// What a browser is told when another site caused a write — `.rulings/` R-186.
///
/// **Its reader is the operator**, looking at a failed request in a browser's
/// network panel or at `curl -H 'Sec-Fetch-Site: cross-site'`: the page that
/// caused the request cannot read the answer. So it names the headers that
/// decided and where the same change can be made instead.
const CROSS_SITE_REFUSAL: &str = "this request changes something, and the browser that sent it \
                                  reports that a page on another site caused it (Sec-Fetch-Site, \
                                  or an Origin that is not this machine's). A browser presents \
                                  the operator certificate to any request to this machine, \
                                  whichever page asked, so no application behind this proxy \
                                  was reached. Make the change from the application's own page \
                                  or with cargo rivet";

/// What a caller sending a path this proxy will not match is told —
/// `ADR-048` §5.
///
/// **It names the segment shape rather than saying "bad request".** The reader
/// is usually a program that built a URL by joining strings, and the fix is in
/// the joining.
const TRAVERSAL_REFUSAL: &str = "this path holds a dot segment or an encoded separator. Which \
                                 application prefix a request belongs to is decided on whole \
                                 path segments, and a path that would have to be normalised \
                                 first is refused rather than guessed at. Send the resolved \
                                 path";

/// What a caller sending a chunked request body is told.
///
/// **`411`, the same choice `rivet-mcpd`'s git handler makes for the same
/// reason.** Nothing behind this proxy decodes `Transfer-Encoding: chunked`,
/// and forwarding the encoded bytes with the header removed leaves a body
/// with no framing declaration at all.
const CHUNKED_REFUSAL: &str = "this proxy does not decode Transfer-Encoding: chunked, and will \
                               not forward a chunked body to a backend with its framing \
                               removed. Send the request with a Content-Length instead";

/// One client connection, from TLS handshake to both sides closed.
///
/// `patience` bounds the first byte, the handshake and the request head, each
/// step on its own deadline — see [`Patience`].
async fn handle(
    stream: TcpStream,
    acceptor: &tokio_rustls::TlsAcceptor,
    connector: &tokio_rustls::TlsConnector,
    routes: &[rivet_core::proxy::Route],
    admin_portal: Option<&rivet_core::proxy::AdminPortal>,
    upstream_name: &str,
    derived_at: Option<&str>,
    passkeys: &passkey::Passkeys,
    journal: &rivet_sdk::journal::Journal,
    patience: Patience,
    certificate: rivet_core::proxy::OperatorCertificate,
) -> Result<(), String> {
    // **Every step before the request is in hand has a deadline** — see
    // [`Patience`]. A peer that opens a socket and says nothing is dropped
    // after `first_byte`; one that begins a handshake and never finishes it
    // after `handshake`, counted from the accept; one that finishes the
    // handshake and sends no head is answered 408 after `head`. A caller that
    // never finishes the handshake gets no answer — there is no TLS session to
    // answer on — so a stall there is dropped rather than logged: one line per
    // stalled socket is a journal an attacker can flood.
    let accepted = tokio::time::Instant::now();
    match tokio::time::timeout(patience.first_byte, stream.readable()).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(format!("client socket: {e}")),
        Err(_) => return Err(String::from("the client sent nothing within the first-byte patience")),
    }
    let handshake_deadline = accepted + patience.handshake;
    let mut client = match tokio::time::timeout_at(handshake_deadline, acceptor.accept(stream)).await {
        Ok(Ok(client)) => client,
        Ok(Err(e)) => return Err(format!("client handshake: {e}")),
        Err(_) => {
            return Err(String::from(
                "the client handshake did not complete within the handshake patience",
            ));
        }
    };

    // **Whether the operator authenticated, asked of `rustls` and not
    // inferred.** `peer_certificates` answers `Some` only for a chain that
    // verified against the authority the server configuration was built with —
    // so this is not *did they send bytes*, it is *did the fleet authority
    // sign this*. A caller that presented nothing reaches here because
    // `asking_for_a_caller_certificate_from` admits one; `decide` is what
    // refuses it. The leaf is `chain[0]`.
    //
    // **And whether what it signed is the operator's** (`.rulings/` R-187 D1):
    // the authority signs every fleet machine's front door too, and a
    // certificate naming a host is a machine's.
    let leaf = client.get_ref().1.peer_certificates().and_then(<[_]>::first);
    let verified = leaf.map(|leaf| rivetos_tls::classify_caller(leaf));
    // **A machine that refuses the certificate never asked for one**
    // ([`server_config`]), so nothing should be here; were a peer to send one
    // anyway, it counts for nothing.
    let by_certificate = match certificate {
        rivet_core::proxy::OperatorCertificate::Accepted => Presented::of(verified.as_ref()),
        rivet_core::proxy::OperatorCertificate::Refused => Presented::Nothing,
    };

    // **The name on the certificate, for the index to show — a LABEL and never
    // an authorisation** (`ADR-028` §4). `presented` above is what decides;
    // this only says which certificate did it, so an operator can see that
    // their browser sent the one they meant. It is read with `rivetos_tls`'s
    // walker rather than searched, because a certificate carries the ISSUER's
    // common name first and a search would print the fleet authority's name
    // where the caller's belongs.
    let mut caller = leaf.and_then(|leaf| rivetos_tls::subject_common_name(leaf));

    let head_deadline = tokio::time::Instant::now() + patience.head;
    // A passkey ceremony's small body keeps what it always had: the handshake's
    // allowance from the accept, or the head's if that ends later.
    let deadline = handshake_deadline.max(head_deadline);
    let head = match tokio::time::timeout_at(head_deadline, read_head(&mut client)).await {
        Ok(Ok(head)) => head,
        Ok(Err(why)) => return respond(&mut client, 400, &why).await,
        Err(_) => return respond(&mut client, 408, "the request head did not arrive in time").await,
    };

    // **A client certificate first, unchanged; a passkey session second** —
    // `ADR-063` §4. The session is asked only when the certificate did not
    // already make the caller the operator, so the certificate path decides
    // exactly what it decided before sessions existed.
    let presented = if by_certificate != Presented::Operator && passkeys.signed_in(&head) {
        caller = Some(String::from("a passkey"));
        Presented::Passkey
    } else {
        by_certificate
    };

    // **Refuse malformed message framing before anything reaches a
    // backend** — RFC 9112 §6.3. `transfer_coding` answers whether the body
    // is `chunked`; a lone `chunked` is well-formed and still refused, just
    // below, because nothing behind this proxy decodes it.
    let chunked = match transfer_coding(&head) {
        Ok(chunked) => chunked,
        Err((status, why)) => return respond(&mut client, status, &why).await,
    };
    if chunked {
        return respond(&mut client, 411, CHUNKED_REFUSAL).await;
    }

    let Some(target) = request_target(&head) else {
        return respond(&mut client, 400, "the request line is not one this proxy understands").await;
    };

    // The query string is carried through untouched; only the path selects a
    // route, which is what keeps matching a question about whole segments.
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (target.as_str(), None),
    };

    // This proxy terminates TLS and has no plaintext listener, so the scheme a
    // browser's `Origin` must carry to be this machine's is always `https`.
    let cross_site_write = rivet_core::cross_site::refuses(&head, "https");

    let (route, rest) = match decide(routes, path, presented, cross_site_write) {
        Decision::Forward(route, rest) => (route, rest),
        Decision::RefuseNotOperator => {
            let Some(why) = verified.as_ref().and_then(rivetos_tls::Caller::refusal) else {
                // `decide` answers this only for a certificate that is not the
                // operator's, and every such caller has a refusal.
                return respond(&mut client, 403, GATE_REFUSAL).await;
            };
            // **Typed fields**, so `rivet get log` can filter a machine's key at
            // the gate by path or by the names it carried.
            let names = match verified.as_ref() {
                Some(rivetos_tls::Caller::Machine(names)) => names.join(", "),
                _ => "(unreadable)".to_string(),
            };
            journal.emit(
                rivet_sdk::journal::Severity::Warn,
                "refused a certificate that is not the operator's at the operator gate; no \
                 application was reached",
                &[
                    ("path", rivet_sdk::journal::Value::Str(path.to_string())),
                    ("names", rivet_sdk::journal::Value::Str(names)),
                ],
            );
            return respond(&mut client, 403, &why).await;
        }
        Decision::RefuseCrossSite => {
            // **Typed fields**, so `rivet get log` can filter the refusals by
            // path or by the header that decided, rather than grepping a
            // sentence. The values are the browser's own words, escaped by the
            // journal's encoder.
            let said = |name: &str| {
                rivet_sdk::journal::Value::Str(
                    rivet_core::cross_site::header(&head, name)
                        .unwrap_or("(absent)")
                        .to_string(),
                )
            };
            journal.emit(
                rivet_sdk::journal::Severity::Warn,
                "refused a cross-site request that changes something; no application was reached",
                &[
                    (
                        "method",
                        rivet_sdk::journal::Value::Str(
                            rivet_core::cross_site::method(&head).to_string(),
                        ),
                    ),
                    ("path", rivet_sdk::journal::Value::Str(path.to_string())),
                    ("sec_fetch_site", said("sec-fetch-site")),
                    ("origin", said("origin")),
                ],
            );
            return refuse_unread_body(&mut client, 403, CROSS_SITE_REFUSAL).await;
        }
        Decision::Index => {
            let page = index_page(routes, admin_portal, upstream_name, derived_at, caller.as_deref());
            return respond_html(&mut client, &page).await;
        }
        // The query string is carried across, so `/notes?x=1` does not lose
        // its parameters on the way to `/notes/?x=1`.
        Decision::Redirect(location) => {
            let location = match query {
                Some(q) => format!("{location}?{q}"),
                None => location,
            };
            return redirect(&mut client, &location).await;
        }
        Decision::Passkey(rest) => {
            let method = rivet_core::cross_site::method(&head).to_string();
            let body = match read_body(&mut client, &head, deadline).await {
                Ok(body) => body,
                Err((status, why)) => return respond(&mut client, status, &why).await,
            };
            let reply = passkeys.answer(&method, &rest, &head, &body);
            return respond_reply(&mut client, &reply, method == "HEAD").await;
        }
        Decision::SignIn(rest) => {
            let method = rivet_core::cross_site::method(&head).to_string();
            // Read and discarded, so a body sent to a page cannot be taken
            // for the next request on this connection.
            if let Err((status, why)) = read_body(&mut client, &head, deadline).await {
                return respond(&mut client, status, &why).await;
            }
            let reply = passkeys.sign_in(&method, &rest, &head);
            return respond_reply(&mut client, &reply, method == "HEAD").await;
        }
        Decision::Moved(location) => {
            let location = match query {
                Some(q) => format!("{location}?{q}"),
                None => location,
            };
            return moved(&mut client, &location).await;
        }
        // **A browser sent to the sign-in page rather than shown a refusal**:
        // a `GET` that asks for HTML and presented neither a certificate nor
        // a session. Every other caller — `curl`, a tool, a write — is still
        // answered `403` with the words, exactly as before passkeys.
        Decision::Refuse(403, GATE_REFUSAL)
            if presented == Presented::Nothing && wants_a_page(&head) =>
        {
            let next = match query {
                Some(q) => format!("{path}?{q}"),
                None => path.to_string(),
            };
            return see_other(
                &mut client,
                &format!("{}?next={}", passkey::SIGN_IN_PAGE, percent_encode(&next)),
            )
            .await;
        }
        // **On a machine that retired the certificate, the gate says so** — a
        // caller holding one cannot present it here, and the refusal must not
        // send them to install it.
        Decision::Refuse(status, why) => {
            let why = if why == GATE_REFUSAL
                && certificate == rivet_core::proxy::OperatorCertificate::Refused
            {
                CERTIFICATE_RETIRED_REFUSAL
            } else {
                why
            };
            return respond(&mut client, status, why).await;
        }
    };

    let upstream = match TcpStream::connect(("127.0.0.1", route.port)).await {
        Ok(upstream) => upstream,
        Err(e) => {
            let why = upstream_failed(journal, route, UpstreamStep::Connect, &e);
            return respond(&mut client, 502, &why).await;
        }
    };
    let name = rustls_pki_types::ServerName::try_from(upstream_name.to_string())
        .map_err(|e| format!("{upstream_name:?} is not a server name: {e}"))?;
    let mut upstream = match connector.connect(name, upstream).await {
        Ok(upstream) => upstream,
        Err(e) => {
            let why = upstream_failed(journal, route, UpstreamStep::Handshake, &e);
            return respond(&mut client, 502, &why).await;
        }
    };

    let forwarded = rewrite_head(&head, &rest, query);
    upstream
        .write_all(forwarded.as_bytes())
        .await
        .map_err(|e| format!("writing to {}: {e}", route.service))?;

    // **Nothing below interprets the response.** After the head is rewritten
    // this is two byte pipes, which is what makes chunked bodies, trailers and
    // upgrades pass through without this program knowing they exist.
    let (mut client_read, mut client_write) = tokio::io::split(client);
    let (mut up_read, mut up_write) = tokio::io::split(upstream);
    let to_upstream = async {
        let _ = tokio::io::copy(&mut client_read, &mut up_write).await;
        let _ = up_write.shutdown().await;
    };
    let to_client = async {
        let _ = tokio::io::copy(&mut up_read, &mut client_write).await;
        let _ = client_write.shutdown().await;
    };
    tokio::join!(to_upstream, to_client);
    Ok(())
}

// --- port 80, plain HTTP — `ADR-061` §7, `ADR-046` §7 ------------------------

/// **What this proxy does with one request on port 80**, decided before any
/// of it touches a socket — [`decide`]'s shape, narrowed to what a plain
/// listener may ever do.
///
/// There is no [`Decision::Index`] here: the index is never served over plain
/// HTTP (`ADR-061` §7). There is no redirect either — a plain route is a
/// bare prefix a phone fetches directly, and a `308` would be one more thing
/// an unpaired phone has to follow correctly. There is no operator gate and no
/// cross-site refusal: nothing on port 80 ever writes anything (only `GET` and
/// `HEAD` are served), so `ADR-046` §4's question does not arise.
#[derive(Debug, PartialEq, Eq)]
enum PlainDecision<'a> {
    /// Reach the application, with the remainder of the path.
    Forward(&'a rivet_core::proxy::Route, String),
    /// Answer the caller directly, and never reach an application.
    Refuse(u16, &'static str),
}

/// What a caller reaching a path plain HTTP does not serve is told.
///
/// **`404` for everything but a `plain:` route, the index included** —
/// `ADR-061` §7. Port 80 must never read as a second front door, so a
/// gated or public-on-443 route answers exactly as nothing were there.
const PLAIN_NOT_FOUND: &str = "no application on this machine serves this path over plain HTTP. \
                               Port 80 serves only a route whose signed manifest declares \
                               proxy-auth-paths plain: (ADR-061 §7); every other path, the \
                               index included, is not found here";

/// What a caller sending anything but `GET` or `HEAD` to a plain route is
/// told.
const PLAIN_METHOD_REFUSAL: &str = "a plain route serves GET and HEAD only. A request that \
                                    changes something is made over 443, after pairing \
                                    (ADR-061 §7)";

/// [`decide`]'s gate, narrowed to port 80: the traversal refusal first
/// (`ADR-048` §5), then only a path whose longest-matching rule is `plain:`,
/// then the method.
fn decide_plain<'a>(
    routes: &'a [rivet_core::proxy::Route],
    path: &str,
    method: &str,
) -> PlainDecision<'a> {
    // **Before the route lookup**, `decide`'s reason exactly: a traversal that
    // today's table happens to refuse would otherwise become live the day a
    // route relaxed.
    if !rivet_core::proxy::is_traversal_free(path) {
        return PlainDecision::Refuse(400, TRAVERSAL_REFUSAL);
    }

    let Some((route, rest)) = rivet_core::proxy::route_for(routes, path) else {
        return PlainDecision::Refuse(404, PLAIN_NOT_FOUND);
    };

    // **The longest-matching rule must be `plain:`.** A gated or public route
    // with no `plain:` rule at this path — the index included, since `/` never
    // matches any route — is exactly as unreachable here as one nothing
    // declared at all: port 80 must never read as a second front door.
    if !route.is_plain_for(&rest) {
        return PlainDecision::Refuse(404, PLAIN_NOT_FOUND);
    }

    if method != "GET" && method != "HEAD" {
        return PlainDecision::Refuse(405, PLAIN_METHOD_REFUSAL);
    }

    PlainDecision::Forward(route, rest)
}

/// One client connection on port 80, from the request head to both sides
/// closed. [`handle`]'s shape with the TLS handshake removed and the decision
/// narrowed to [`decide_plain`].
///
/// **Forwarded exactly as a `none:` route is forwarded on 443** — the same
/// backend dial over the SAME mutual TLS `connector`, and the same
/// [`rewrite_head`]. Only the CLIENT side has no TLS; the proxy still presents
/// its own certificate to the backend (`ADR-046` §4), so a backend cannot
/// tell a plain-HTTP caller from an ordinary one.
async fn handle_plain(
    mut stream: TcpStream,
    connector: &tokio_rustls::TlsConnector,
    routes: &[rivet_core::proxy::Route],
    upstream_name: &str,
    journal: &rivet_sdk::journal::Journal,
    head_patience: std::time::Duration,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + head_patience;
    let head = match tokio::time::timeout_at(deadline, read_head(&mut stream)).await {
        Ok(Ok(head)) => head,
        Ok(Err(why)) => return respond(&mut stream, 400, &why).await,
        Err(_) => return respond(&mut stream, 408, "the request head did not arrive in time").await,
    };

    let chunked = match transfer_coding(&head) {
        Ok(chunked) => chunked,
        Err((status, why)) => return respond(&mut stream, status, &why).await,
    };
    if chunked {
        return respond(&mut stream, 411, CHUNKED_REFUSAL).await;
    }

    let Some((method, target)) = request_line(&head) else {
        return respond(&mut stream, 400, "the request line is not one this proxy understands").await;
    };
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (target, None),
    };

    let (route, rest) = match decide_plain(routes, path, method) {
        PlainDecision::Refuse(status, why) => return respond(&mut stream, status, why).await,
        PlainDecision::Forward(route, rest) => (route, rest),
    };

    let upstream = match TcpStream::connect(("127.0.0.1", route.port)).await {
        Ok(upstream) => upstream,
        Err(e) => {
            let why = upstream_failed(journal, route, UpstreamStep::Connect, &e);
            return respond(&mut stream, 502, &why).await;
        }
    };
    let name = rustls_pki_types::ServerName::try_from(upstream_name.to_string())
        .map_err(|e| format!("{upstream_name:?} is not a server name: {e}"))?;
    let mut upstream = match connector.connect(name, upstream).await {
        Ok(upstream) => upstream,
        Err(e) => {
            let why = upstream_failed(journal, route, UpstreamStep::Handshake, &e);
            return respond(&mut stream, 502, &why).await;
        }
    };

    let forwarded = rewrite_head(&head, &rest, query);
    upstream
        .write_all(forwarded.as_bytes())
        .await
        .map_err(|e| format!("writing to {}: {e}", route.service))?;

    // **Nothing below interprets the response** — `handle`'s reason, identical.
    let (mut client_read, mut client_write) = tokio::io::split(stream);
    let (mut up_read, mut up_write) = tokio::io::split(upstream);
    let to_upstream = async {
        let _ = tokio::io::copy(&mut client_read, &mut up_write).await;
        let _ = up_write.shutdown().await;
    };
    let to_client = async {
        let _ = tokio::io::copy(&mut up_read, &mut client_write).await;
        let _ = client_write.shutdown().await;
    };
    tokio::join!(to_upstream, to_client);
    Ok(())
}

/// The port-80 accept loop — [`serve`]'s main loop, narrowed to no TLS
/// acceptor and [`handle_plain`] in place of [`handle`].
///
/// A function of its own rather than a branch in [`serve`]'s loop, because it
/// is spawned as its own task: [`serve`] returns to its own accept loop on 443
/// immediately after handing this one its listener, and the two run
/// concurrently for as long as the process does.
async fn serve_plain(
    listener: TcpListener,
    connector: tokio_rustls::TlsConnector,
    routes: Arc<Vec<rivet_core::proxy::Route>>,
    upstream_name: Arc<String>,
    journal: rivet_sdk::journal::Journal,
    head_patience: std::time::Duration,
    governor: rivet_core::connection_slots::Governor<std::net::IpAddr>,
) {
    let mut stopping = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        Ok(s) => s,
        Err(why) => {
            journal.error(&format!("the plain listener cannot watch for SIGTERM: {why}"));
            return;
        }
    };

    loop {
        let accepted = tokio::select! {
            () = async { stopping.recv().await; } => {
                journal.info("SIGTERM received; the plain listener is no longer accepting");
                return;
            }
            accepted = listener.accept() => accepted,
        };

        let Ok((stream, peer)) = accepted else {
            continue;
        };
        let Some(slot) = admit(&governor, peer, &journal) else {
            continue;
        };

        let connector = connector.clone();
        let routes = Arc::clone(&routes);
        let upstream_name = Arc::clone(&upstream_name);
        let journal = journal.clone();
        tokio::spawn(async move {
            let _slot = slot;
            let _ = handle_plain(stream, &connector, &routes, &upstream_name, &journal, head_patience)
                .await;
        });
    }
}

/// Read bytes until the end of the request head, bounded.
async fn read_head<S>(stream: &mut S) -> Result<String, String>
where
    S: tokio::io::AsyncRead + Unpin,
{
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while buf.len() < MAX_HEAD {
        let read = stream
            .read(&mut byte)
            .await
            .map_err(|e| format!("reading the request: {e}"))?;
        if read == 0 {
            return Err(String::from("the connection closed before a complete request"));
        }
        buf.push(byte[0]);
        if buf.ends_with(b"\r\n\r\n") {
            return String::from_utf8(buf).map_err(|_| String::from("the request head is not text"));
        }
    }
    Err(String::from("the request head is larger than this proxy will read"))
}

/// Whether the request head declares a `chunked` body, refusing any framing
/// this proxy cannot forward honestly — RFC 9112 §6.3.
///
/// **A repeated `Content-Length` is refused first, identical values
/// included** (`.rulings/` R-210 D17). This proxy copies bytes and closes each
/// hop rather than re-framing anything, so two same-named fields would be
/// forwarded exactly as they arrived — leaving the backend to pick one, which
/// is the request-smuggling precondition whether or not the two values
/// happen to match.
///
/// **`Transfer-Encoding` together with `Content-Length` is two disagreeing
/// claims about where the body ends** — the classic request-smuggling shape —
/// and a `Transfer-Encoding` naming anything other than exactly `chunked`
/// names a coding nothing behind this proxy understands. Both are refused
/// with the returned `400`, before a byte of the request reaches a backend.
///
/// **A lone `chunked` is well-formed and is not refused here.** It comes back
/// as `Ok(true)`; the caller refuses it separately, because nothing behind
/// this proxy decodes chunked transfer coding, and forwarding the encoded
/// bytes with the header removed is a body with no framing declaration left
/// at all.
fn transfer_coding(head: &str) -> Result<bool, (u16, String)> {
    let declared: Vec<&str> =
        rivet_core::cross_site::header_values(head, "transfer-encoding").collect();
    let chunked = if declared.is_empty() {
        false
    } else {
        let joined = declared.join(",");
        let codings: Vec<&str> =
            joined.split(',').map(str::trim).filter(|c| !c.is_empty()).collect();
        if codings.len() != 1 || !codings[0].eq_ignore_ascii_case("chunked") {
            return Err((
                400,
                format!("Transfer-Encoding: {joined} names a coding this proxy does not forward"),
            ));
        }
        true
    };

    let lengths: Vec<&str> =
        rivet_core::cross_site::header_values(head, "content-length").collect();
    if lengths.len() > 1 {
        return Err((
            400,
            format!(
                "two Content-Length fields, {}: this request has no one length",
                lengths.join(" and ")
            ),
        ));
    }

    if chunked && !lengths.is_empty() {
        return Err((
            400,
            String::from("both Transfer-Encoding and Content-Length: two framings is no framing"),
        ));
    }

    Ok(chunked)
}

/// The method and target from a request line, or `None` if either is
/// missing, the version is not HTTP/1.x, or the target does not start with
/// `/`.
///
/// [`request_target`]'s parsing, generalised: 443's [`decide`] never needed
/// the method because every verdict there answers the same way regardless of
/// it, but port 80 refuses everything except `GET` and `HEAD` (`ADR-061`
/// §7.1), so [`decide_plain`] needs both fields from the one line.
fn request_line(head: &str) -> Option<(&str, &str)> {
    let line = head.lines().next()?;
    let mut parts = line.split(' ');
    let method = parts.next()?;
    let target = parts.next()?;
    let version = parts.next()?;
    if !version.starts_with("HTTP/1.") || !target.starts_with('/') {
        return None;
    }
    Some((method, target))
}

/// The target from a request line, or `None` if it is not one we handle.
fn request_target(head: &str) -> Option<String> {
    request_line(head).map(|(_method, target)| target.to_string())
}

/// Replace the request line's target with the remainder, and close the hop.
///
/// **`Connection: close` is forced in both directions and every hop-by-hop
/// header the client sent is dropped.** A `Connection: keep-alive` forwarded to
/// a backend would leave this program holding a socket it has stopped reading.
fn rewrite_head(head: &str, rest: &str, query: Option<&str>) -> String {
    let mut out = String::with_capacity(head.len());
    let mut lines = head.lines();

    if let Some(request_line) = lines.next() {
        let mut parts = request_line.split(' ');
        let method = parts.next().unwrap_or("GET");
        let _old = parts.next();
        let target = match query {
            Some(q) => format!("{rest}?{q}"),
            None => rest.to_string(),
        };
        out.push_str(&format!("{method} {target} HTTP/1.1\r\n"));
    }

    for line in lines {
        if line.is_empty() {
            continue;
        }
        let name = line.split(':').next().unwrap_or("").trim().to_ascii_lowercase();
        // Hop-by-hop headers are this connection's, not the next one's.
        if matches!(
            name.as_str(),
            "connection" | "keep-alive" | "proxy-connection" | "transfer-encoding" | "upgrade"
        ) {
            continue;
        }
        // **This proxy's session cookie is this proxy's** — `ADR-063` §4. It
        // is sent to every path on the origin, a public route's included, so
        // an application that saw it could replay it at the gate. The rest of
        // the header is the application's and passes untouched.
        if name == "cookie" {
            let value = line.split_once(':').map_or("", |(_, v)| v);
            if value.contains(passkey::COOKIE) {
                if let Some(kept) = passkey::without_session(value) {
                    out.push_str(&format!("Cookie: {kept}\r\n"));
                }
                continue;
            }
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str("Connection: close\r\n\r\n");
    out
}

// --- the index, ADR-046 §6 ---------------------------------------------------

/// **The style, embedded because the board is offline** — `ADR-046` §6.
///
/// No framework and no CDN: this machine is on a house LAN with no guarantee of
/// a route to the internet, and a stylesheet fetched from one would make the
/// only page that says what a machine runs the one page that needs the machine
/// to have working DNS.
///
/// It follows `prefers-color-scheme` in the one way that does not need
/// JavaScript — two colour blocks and nothing else varying — because the page
/// is served by a program with no session, no storage and no reason to have a
/// preference of its own.
const INDEX_STYLE: &str = "\
:root{color-scheme:light dark;--bg:#fbfbfa;--fg:#1c1c1a;--dim:#6a6a64;\
--line:#e2e2dd;--card:#fff;--accent:#8a5a2b;--direct:#3f6d8c;\
--direct-line:#c9d8e2}\
@media(prefers-color-scheme:dark){:root{--bg:#16161a;--fg:#e8e8e4;--dim:#95958d;\
--line:#2b2b30;--card:#1e1e23;--accent:#d29a5e;--direct:#78aecd;\
--direct-line:#31414d}}\
*{box-sizing:border-box}\
body{margin:0;padding:2.5rem 1.25rem;background:var(--bg);color:var(--fg);\
font:16px/1.55 system-ui,-apple-system,Segoe UI,Roboto,sans-serif}\
main{max-width:34rem;margin:0 auto}\
h1{font-size:1.15rem;font-weight:700;margin:0 0 1.75rem;text-align:center}\
p.sub{margin:0 0 1.75rem;color:var(--dim);font-size:.875rem}\
ul{list-style:none;margin:0;padding:0;border:1px solid var(--line);\
border-radius:10px;overflow:hidden;background:var(--card)}\
li+li{border-top:1px solid var(--line)}\
li a{display:flex;align-items:baseline;gap:.6rem;padding:.85rem 1.1rem;\
color:inherit;text-decoration:none}\
li a:hover,li a:focus{background:rgba(127,127,127,.09);outline:none}\
li a:focus-visible{outline:2px solid var(--accent);outline-offset:-2px}\
.path{font:.95rem/1.4 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;\
color:var(--accent)}\
.svc{color:var(--dim);font-size:.8125rem;margin-left:auto}\
.posture{display:inline-flex;align-items:center;gap:.3rem;color:var(--dim);\
border:1px solid var(--line);border-radius:4px;padding:.15rem .35rem}\
.posture svg{display:block;width:.875rem;height:.875rem;stroke:currentColor;\
fill:none;stroke-width:2;stroke-linecap:round;stroke-linejoin:round}\
.posture svg:hover{color:var(--accent)}\
p.none{color:var(--dim);font-size:.875rem;border:1px dashed var(--line);\
border-radius:10px;padding:1.1rem;margin:0}\
ul.direct{margin-top:.75rem;border-color:var(--direct-line)}\
ul.direct .path{color:var(--direct)}\
ul.direct .posture{border-color:var(--direct-line)}\
footer{color:var(--dim);font-size:.75rem;margin-top:1.75rem;text-align:center}";

/// Escape the five characters that would let a value out of its element.
///
/// **Every value on this page comes from a signed manifest**, so this is not a
/// hostile-input path — and it is here anyway for the reason
/// `rivet_core::package::application`'s own validators give: a value is escaped
/// where it is *emitted*, not where somebody argues it cannot be dangerous, and
/// a rule that holds only as long as nobody changes where the table comes from
/// is not a rule.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// **The routes that exist right now, as a list a person can click** —
/// `ADR-046` §6.
///
/// Built from the same table the forwarding uses, because the proxy already
/// holds the only source of truth about what is routable and an index built
/// from anything else would be a second answer (`ADR-011`).
///
/// **A machine with no routes says so** rather than drawing an empty box: that
/// is the ordinary machine (`ADR-046` §5), and a page that looked broken would
/// send its reader looking for a fault that is not there.
/// **What a route asks of whoever reaches it**, as icons a person can read at a
/// glance.
///
/// It replaced a `NO OPERATOR` label on 2026-09-06. The label was a note about
/// an *absence*, so the ordinary row said nothing and the reader had to know
/// that silence meant *gated* — which is the wrong way round for a security
/// posture. **Every row now states what it has**: a padlock for the transport,
/// and a person beside it when a caller must prove who they are.
///
/// # The padlock is unconditional, and that is a fact about this program
///
/// It is not read from the route, because there is nothing to read: **this
/// proxy has no plaintext mode in either direction.** It terminates TLS on 443
/// and reaches every backend over TLS presenting its own fleet certificate
/// (`ADR-046` §4.2), so *secured by SSL* is true of every row by construction.
///
/// **If a plaintext mode is ever added, this is one of the places that has to
/// learn about it** — a badge that looks computed and is constant is exactly
/// how a page starts lying. Saying so here is cheaper than a `bool` that is
/// always `true`.
///
/// # Native tooltips, and no script
///
/// `title` is the browser's own, needs no JavaScript, and survives `ADR-046`
/// §6's rule that this page fetches and runs nothing. `aria-label` carries the
/// same words for a reader that does not hover.
/// # A route may be gated with an open door in it — `ADR-048` §4
///
/// `open` names the prefixes under this row that are served to a caller
/// presenting nothing. A third icon appears when there are any, and its title
/// names them: the list an operator came to read is where they are most likely
/// to notice one, and a row that showed only its ROOT verdict would say
/// *authenticated* about an application with a public prefix on it.
fn posture_badge(requires_operator: bool, open: &[String]) -> String {
    // A shackle over a body, and a head over shoulders. Drawn here rather than
    // fetched, for the reason the stylesheet is embedded: this machine may have
    // no route to the internet, and the page that says what it runs must not be
    // the page that needs DNS.
    const PADLOCK: &str = "<rect x=\"3\" y=\"11\" width=\"18\" height=\"11\" rx=\"2\"/>\
                           <path d=\"M7 11V7a5 5 0 0 1 10 0v4\"/>";
    const PERSON: &str = "<path d=\"M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2\"/>\
                          <circle cx=\"12\" cy=\"7\" r=\"4\"/>";
    // The same shackle, swung open, over the same body — so the two read as one
    // pair rather than as two unrelated marks.
    const UNLATCHED: &str = "<rect x=\"3\" y=\"11\" width=\"18\" height=\"11\" rx=\"2\"/>\
                             <path d=\"M7 11V7a5 5 0 0 1 9.9-1\"/>";

    let icon = |title: &str, paths: &str| {
        format!(
            "<svg viewBox=\"0 0 24 24\" role=\"img\" aria-label=\"{t}\"><title>{t}</title>\
             {paths}</svg>",
            t = escape(title)
        )
    };

    let mut badge = String::from("<span class=\"posture\">");
    badge.push_str(&icon("Endpoint secured by SSL", PADLOCK));
    if requires_operator {
        badge.push_str(&icon(
            "Endpoint requires user authentication certificate",
            PERSON,
        ));
    }
    if !open.is_empty() {
        badge.push_str(&icon(
            &format!(
                "Served without a user authentication certificate: {}",
                open.join(", ")
            ),
            UNLATCHED,
        ));
    }
    badge.push_str("</span>");
    badge
}

fn index_page(
    routes: &[rivet_core::proxy::Route],
    admin_portal: Option<&rivet_core::proxy::AdminPortal>,
    machine: &str,
    derived_at: Option<&str>,
    caller: Option<&str>,
) -> String {
    let mut body = String::new();
    body.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">");
    body.push_str("<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
    body.push_str(&format!("<title>{}</title>", escape(machine)));
    body.push_str(&format!("<style>{INDEX_STYLE}</style></head><body><main>"));
    body.push_str(&format!("<h1>{}</h1>", escape(machine)));

    if routes.is_empty() {
        body.push_str(
            "<p class=\"sub\">Nothing on this machine asks to be proxied.</p>\
             <p class=\"none\">An application is routed here because its signed manifest \
             declares a <code>proxy-path</code>. None of the applications installed on this \
             machine does, so there is nothing to list — which is the ordinary state of a \
             Rivet rather than a fault.</p>",
        );
    } else {
        // **No count.** It said "N applications, routed by path" until
        // 2026-09-06, and the maintainer's answer to that was that he could
        // count the entries himself (.rulings/ R-342). The list is directly below
        // and says both things itself.
        body.push_str("<ul>");
        let host = escape(machine);
        for route in routes {
            let path = escape(&route.path);
            body.push_str(&format!(
                // **The whole address, not the suffix.** A person reading this
                // page is about to type or paste one somewhere, and `/aura/`
                // on its own is not something you can give to anybody. The
                // HREF stays root-relative — it is already on this machine.
                "<li><a href=\"/{path}/\"><span class=\"path\">{host}/{path}/</span>"
            ));
            // **The whole address of each open prefix, not the suffix** — same
            // reason the row shows the whole address: a reader is about to
            // check one against what they thought they had agreed to.
            let open: Vec<String> = route
                .open_prefixes()
                .iter()
                .map(|prefix| format!("/{}/{prefix}", route.path))
                .collect();
            body.push_str(&posture_badge(route.requires_operator, &open));
            body.push_str(&format!(
                "<span class=\"svc\">{}</span></a></li>",
                escape(&route.service)
            ));
        }
        body.push_str("</ul>");
    }

    // **The administration portal — a row, in its own frame, and ONLY when there
    // is one to reach.**
    //
    // The maintainer asked twice why fleet was missing, so its absence does not
    // explain itself. It was a paragraph of prose for about an hour, and his
    // answer to that was fair: nobody wants to read prose like that every time
    // they log in (.rulings/ R-342).
    //
    // So it is a row, in the same visual language as the others — address,
    // posture, service — inside a **separately framed list in a cooler
    // colour**. The distinction is carried by the frame rather than by a
    // sentence, and the sentence is a `title` for whoever wants it.
    //
    // **It is not, and must not become, a route.** `rivet-fleetd` declares no
    // `proxy-path`: it moved OFF 443 so this proxy could have it, and keeps its
    // own mTLS on 4443 because the administration surface must not depend on
    // what it administers (`ADR-046` §7). Putting it in the routed list would
    // say *this is proxied*, which is the one thing untrue of it — hence a
    // second list, asserted by a test to be a second list.
    //
    // **And it is conditional now.** A fleet SERVER runs `rivet-fleetd`; a
    // MEMBER runs none. The row was rendered unconditionally until 2026-09-23,
    // which drew a fleet server on a member that had none — the one dishonesty
    // this page's `derived from installed manifests` footer forbids. It is now
    // present only when `rivet-sysd` named the portal in the table, from the
    // installed set (`rivet_core::proxy::admin_portal`), exactly as the routes
    // are. A member offers none, so its page shows none.
    if let Some(portal) = admin_portal {
        body.push_str(&format!(
            "<ul class=\"direct\"><li><a href=\"https://{host}:{port}/\" \
             title=\"Reached directly rather than through this proxy\">\
             <span class=\"path\">{host}:{port}/</span>",
            host = escape(machine),
            port = portal.port
        ));
        // The portal verifies an operator certificate of its own, so it earns
        // both icons honestly rather than by inheriting the row above it.
        body.push_str(&posture_badge(true, &[]));
        body.push_str(&format!(
            "<span class=\"svc\">{}</span></a></li></ul>",
            escape(&portal.service)
        ));
    }

    // **Two facts, each naming where it came from**, replacing a paragraph that
    // asserted both without evidence. The maintainer asked for exactly this
    // shape on 2026-09-06.
    //
    // `derived_at` is the routing table's own modification time, so it answers
    // *how old is this list* rather than *what time is it*. `caller` is the
    // `commonName` from the certificate this request presented — **a label and
    // never an authorisation** (`ADR-028` §4.5): the handshake already decided
    // whether to serve, and this only says which certificate did it.
    // **The two VALUES are bold; the labels around them are not.** They are the
    // only things on this line that change, and they are what somebody came to
    // read — the words either side are there to say what they mean.
    body.push_str("<footer>Source: derived from installed application manifests");
    if let Some(when) = derived_at {
        body.push_str(&format!(": <strong>{}</strong>", escape(when)));
    }
    body.push_str(".<br>Authenticated as: ");
    match caller {
        Some(name) => body.push_str(&format!("<strong>{}</strong>", escape(name))),
        // A certificate with no `commonName` is legal, and a public route is
        // reached with no certificate at all — neither is a fault, and neither
        // may print an empty space where a name goes. **Not bold**: it is a
        // sentence about an absence rather than a value.
        None => body.push_str("a certificate carrying no common name"),
    }
    body.push_str(".</footer></main></body></html>");
    body
}

/// Answer the client with a page, without reaching an application.
async fn respond_html<S>(stream: &mut S, page: &str) -> Result<(), String>
where
    S: tokio::io::AsyncWrite + Unpin,
{
    let answer = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{page}",
        page.len()
    );
    let _ = stream.write_all(answer.as_bytes()).await;
    let _ = stream.shutdown().await;
    Ok(())
}

/// Answer the client directly, without reaching an application.
/// **One character along** — the same prefix with a trailing slash.
///
/// `308` keeps the method and the body; see [`decide`] for why that matters and
/// why the redirect exists at all. The body is there for a client that shows it
/// rather than following, which is every command-line tool run without `-L`.
///
/// # Errors
///
/// Never — a caller that has gone away is not this function's problem, and the
/// signature matches [`respond`] so the two are interchangeable at a call site.
async fn redirect<S>(stream: &mut S, location: &str) -> Result<(), String>
where
    S: tokio::io::AsyncWrite + Unpin,
{
    let body = format!(
        "308 Permanent Redirect\nthis application is mounted at {location} — a request \
         without the trailing slash would resolve its relative links against the machine \
         root (ADR-046 §8)\n"
    );
    let answer = format!(
        "HTTP/1.1 308 Permanent Redirect\r\nLocation: {location}\r\n\
         Content-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(answer.as_bytes()).await;
    let _ = stream.shutdown().await;
    Ok(())
}

/// **`308 Permanent Redirect` for an address that moved** — [`Decision::Moved`].
/// `308` rather than `301` for [`redirect`]'s reason: it keeps the method.
async fn moved<S>(stream: &mut S, location: &str) -> Result<(), String>
where
    S: tokio::io::AsyncWrite + Unpin,
{
    let body = format!("308 Permanent Redirect\nthis page moved to {location}\n");
    let answer = format!(
        "HTTP/1.1 308 Permanent Redirect\r\nLocation: {location}\r\n\
         Content-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(answer.as_bytes()).await;
    let _ = stream.shutdown().await;
    Ok(())
}

/// Whether a request is a browser asking for a page: a `GET` whose `Accept`
/// names HTML. Only such a caller is sent to the login page rather than
/// answered `403` (`ADR-063` §4).
fn wants_a_page(head: &str) -> bool {
    rivet_core::cross_site::method(head) == "GET"
        && rivet_core::cross_site::header_values(head, "accept").any(|v| v.contains("text/html"))
}

/// Percent-encode everything but the unreserved characters and `/`, for a
/// path carried inside a query string.
fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// **`303 See Other`** — a browser sent on to `location` with a `GET`.
async fn see_other<S>(stream: &mut S, location: &str) -> Result<(), String>
where
    S: tokio::io::AsyncWrite + Unpin,
{
    let body = format!(
        "303 See Other\nthis machine requires an operator certificate or a passkey session; \
         sign in at {location}\n"
    );
    let answer = format!(
        "HTTP/1.1 303 See Other\r\nLocation: {location}\r\n\
         Content-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(answer.as_bytes()).await;
    let _ = stream.shutdown().await;
    Ok(())
}

/// **The body a passkey ceremony sent**, by its `Content-Length`, bounded by
/// [`passkey::MAX_BODY`] and by the same deadline as the head. A chunked body
/// never reaches here: [`transfer_coding`] refused it.
async fn read_body<S>(
    stream: &mut S,
    head: &str,
    deadline: tokio::time::Instant,
) -> Result<Vec<u8>, (u16, String)>
where
    S: tokio::io::AsyncRead + Unpin,
{
    let length = match rivet_core::cross_site::header(head, "content-length") {
        None => 0,
        Some(value) => value
            .trim()
            .parse::<usize>()
            .map_err(|_| (400, format!("Content-Length {value:?} is not a number")))?,
    };
    if length > passkey::MAX_BODY {
        return Err((
            413,
            format!("a passkey ceremony sends at most {} bytes", passkey::MAX_BODY),
        ));
    }
    let mut body = vec![0u8; length];
    match tokio::time::timeout_at(deadline, stream.read_exact(&mut body)).await {
        Ok(Ok(_)) => Ok(body),
        Ok(Err(e)) => Err((400, format!("the body could not be read: {e}"))),
        Err(_) => Err((408, String::from("the body did not arrive in time"))),
    }
}

/// Write a passkey [`passkey::Reply`]: its status, type, cookie and body.
async fn respond_reply<S>(stream: &mut S, reply: &passkey::Reply, head_only: bool) -> Result<(), String>
where
    S: tokio::io::AsyncWrite + Unpin,
{
    let cookie = reply
        .set_cookie
        .as_ref()
        .map_or_else(String::new, |c| format!("Set-Cookie: {c}\r\n"));
    let answer = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {length}\r\n\
         Cache-Control: no-store\r\nX-Frame-Options: DENY\r\n\
         Content-Security-Policy: frame-ancestors 'none'\r\n{cookie}Connection: close\r\n\r\n",
        status = reply.status,
        reason = reason(reply.status),
        kind = reply.content_type,
        length = reply.body.len(),
    );
    let _ = stream.write_all(answer.as_bytes()).await;
    if !head_only {
        let _ = stream.write_all(reply.body.as_bytes()).await;
    }
    let _ = stream.shutdown().await;
    Ok(())
}

/// The reason phrase for a status this proxy answers with itself.
const fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Content Too Large",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        _ => "Error",
    }
}

/// Backend dials and handshakes that failed, keyed by the backend's port, so each
/// application's failure is journalled at most once per
/// `rivet_core::connection_slots::REPORT_EVERY_MS` — the cadence every other
/// refusal here keeps. One process-wide set: both listeners dial the same
/// backends, and a browser retrying a broken application is one failure to
/// report, not one per listener.
static UPSTREAM_FAILURES: std::sync::Mutex<rivet_core::journal::TokenBuckets<u16>> =
    std::sync::Mutex::new(rivet_core::journal::TokenBuckets::new(
        1,
        1,
        rivet_core::connection_slots::TRACKED_SOURCES,
        rivet_core::connection_slots::REPORT_EVERY_MS,
    ));

/// One failed dial or handshake with the backend on `port`: `Some(n)` when it is time
/// to journal, `n` being the failures since the last line.
fn note_upstream_failure(
    failures: &mut rivet_core::journal::TokenBuckets<u16>,
    port: u16,
    now_ms: u64,
) -> Option<u64> {
    match failures.refuse(port, now_ms) {
        rivet_core::journal::Admission::Report { dropped } => Some(dropped),
        _ => None,
    }
}

/// Which step of reaching a backend failed — [`upstream_failed`]'s two causes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpstreamStep {
    /// The TCP dial: nothing accepted it, which is an application that is not
    /// running or has stopped listening.
    Connect,
    /// The mutual-TLS handshake over a dial that succeeded.
    Handshake,
}

/// **Reaching `route`'s backend failed at `step`: what the caller is told**,
/// and the journal line, at most once per backend per window.
///
/// The caller is answered `502` with this sentence rather than having its
/// connection closed, because a closed connection reaches a browser as an
/// empty response that names nothing. A refused or unreachable dial says the
/// application is not answering. Of the handshake failures, the commonest
/// gets its own sentence: a backend whose certificate this fleet's authority
/// did not issue — one staged from another authority — which this proxy
/// refuses by design (`ADR-046` §4), so the request is never handed to it.
fn upstream_failed(
    journal: &rivet_sdk::journal::Journal,
    route: &rivet_core::proxy::Route,
    step: UpstreamStep,
    error: &std::io::Error,
) -> String {
    // `UnknownIssuer` when the issuing authority's name is not this fleet's;
    // `BadSignature` when it carries the same name and another key — a root
    // minted again under the same name, which is the same cause.
    let another_authority = matches!(
        error.get_ref().and_then(|inner| inner.downcast_ref::<rustls::Error>()),
        Some(rustls::Error::InvalidCertificate(
            rustls::CertificateError::UnknownIssuer | rustls::CertificateError::BadSignature
        ))
    );
    let why = if step == UpstreamStep::Connect {
        format!(
            "the application {} is not answering, so this proxy could not hand it the request: \
             {error}",
            route.service
        )
    } else if another_authority {
        format!(
            "the application {} answered with a certificate not issued by this fleet's authority, \
             so this proxy did not hand it the request",
            route.service
        )
    } else {
        format!(
            "the TLS handshake with the application {} failed, so this proxy did not hand it the \
             request: {error}",
            route.service
        )
    };
    let report = {
        let mut failures = UPSTREAM_FAILURES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        note_upstream_failure(&mut failures, route.port, now_ms())
    };
    if let Some(failures) = report {
        journal.emit(
            rivet_sdk::journal::Severity::Warn,
            &format!(
                "answered 502: {why} — at most one line per {}s for this application",
                rivet_core::connection_slots::REPORT_EVERY_MS / 1000
            ),
            &[
                ("service", rivet_sdk::journal::Value::Str(route.service.clone())),
                ("port", rivet_sdk::journal::Value::Int(i64::from(route.port))),
                (
                    "failures",
                    rivet_sdk::journal::Value::Int(i64::try_from(failures).unwrap_or(i64::MAX)),
                ),
            ],
        );
    }
    why
}

async fn respond<S>(stream: &mut S, status: u16, why: &str) -> Result<(), String>
where
    S: tokio::io::AsyncWrite + Unpin,
{
    let reason = reason(status);
    let body = format!("{status} {reason}\n{why}\n");
    let answer = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(answer.as_bytes()).await;
    let _ = stream.shutdown().await;
    Ok(())
}

/// The most of a refused request's body [`refuse_unread_body`] reads and
/// discards, and how long it waits for it.
///
/// Bounds on bytes an unauthenticated page chose to send, so neither is a
/// working limit: they are the point past which the answer's legibility is not
/// worth holding a connection for.
const DRAIN_BYTES: u64 = 1024 * 1024;
const DRAIN_TIME: std::time::Duration = std::time::Duration::from_secs(2);

/// Answer directly, then **read and discard what is left of the request body**
/// for a moment before closing.
///
/// A refused write arrives with a body this proxy never reads. Closing a socket
/// with unread bytes in it makes TCP send a reset, and a reset can reach the
/// client before it has read the answer — so the operator testing the refusal
/// would see *connection reset* instead of the `403` naming why. Reading the
/// rest, bounded by [`DRAIN_BYTES`] and [`DRAIN_TIME`], is what lets the answer
/// land (RFC 9112 §9.6). **Nothing read here goes anywhere.**
async fn refuse_unread_body<S>(stream: &mut S, status: u16, why: &str) -> Result<(), String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    respond(stream, status, why).await?;
    let drain = async {
        let mut sink = tokio::io::sink();
        let _ = tokio::io::copy(&mut (&mut *stream).take(DRAIN_BYTES), &mut sink).await;
    };
    let _ = tokio::time::timeout(DRAIN_TIME, drain).await;
    Ok(())
}

/// The certificate this proxy serves to browsers and phones, and **the gate**.
///
/// `ADR-046` §4.1. A client certificate is asked for and verified against the
/// fleet authority; a caller presenting none reaches [`handle`], which refuses
/// it on every route that did not declare otherwise. The check is per route
/// because the route is not known until the request line has been read, and
/// [`rivet_sdk::serve::Tls::asking_for_a_caller_certificate_from`] says at its
/// own definition why almost nothing else may take that shape.
///
/// **The whole configuration is the SDK's, including the crypto provider.**
/// This function used to build a `ServerConfig` by hand and name its own
/// `ring::default_provider()` with a comment saying it matched the others —
/// which is `ADR-011`'s second answer with a note attached rather than one
/// answer, and `rivet-sdk/src/serve/tls.rs`'s module header says so by name.
///
/// # A machine that no longer accepts the certificate does not ask for one
///
/// [`rivet_core::proxy::OperatorCertificate::Refused`] — the machine's document
/// turned the certificate off (`ADR-046` §4, `.rulings/` R-234). The handshake
/// then requests no client certificate at all, so a browser holding one is
/// never asked to choose it and only a passkey session admits the operator; a caller
/// with neither is told why by [`CERTIFICATE_RETIRED_REFUSAL`].
fn server_config(
    data: &std::path::Path,
    certificate: rivet_core::proxy::OperatorCertificate,
) -> Result<rustls::ServerConfig, String> {
    let tls = rivet_sdk::serve::Tls::in_directory(data, TLS_CERT, TLS_KEY);
    match certificate {
        rivet_core::proxy::OperatorCertificate::Accepted => {
            tls.asking_for_a_caller_certificate_from(TLS_ROOT).server_config()
        }
        rivet_core::proxy::OperatorCertificate::Refused => tls.server_config(),
    }
}

/// How this proxy reaches the applications behind it, **and how they reach
/// back at it**.
///
/// **It verifies them**, rather than trusting loopback. The sandbox is not
/// enforced on this hardware (`ADR-024` §8.1), so *"it came from 127.0.0.1"* is
/// not an assertion about who is listening — and the fleet authority is already
/// there, so checking costs a root certificate rather than a trust decision.
///
/// **And it presents its own certificate, so the backend can verify IT** —
/// `ADR-046` §4.2, `R-152.1`, added 2026-09-05. It is the same file this proxy
/// serves to browsers: the machine has one name and one certificate, so the
/// identity a backend checks is the machine's, which is precisely the claim
/// being made — *the thing that connected is a program the fleet authority
/// signed a key for*, not *the connection came from 127.0.0.1*.
///
/// **Failing to present one is not silently tolerable and this does not
/// tolerate it**: `with_client_auth_cert` refuses at startup rather than at the
/// first request, for the reason every other check in this file is at startup.
fn client_config(data: &std::path::Path, upstream_name: &str) -> Result<rustls::ClientConfig, String> {
    let root_pem = read_pem(data, TLS_ROOT)?;
    let mut roots = rustls::RootCertStore::empty();
    for root in rustls_pemfile::certs(&mut root_pem.as_slice()) {
        let root = root.map_err(|e| format!("{TLS_ROOT}: {e}"))?;
        roots.add(root).map_err(|e| format!("{TLS_ROOT}: {e}"))?;
    }
    if roots.is_empty() {
        return Err(format!("{TLS_ROOT} carries no certificate"));
    }
    // Fail here rather than per-connection: a name that is not a name would
    // make every request fail with a handshake error at a moment nobody is
    // watching, which is the failure this whole file's startup order avoids.
    rustls_pki_types::ServerName::try_from(upstream_name.to_string())
        .map_err(|e| format!("the `{UPSTREAM_NAME}` setting {upstream_name:?} is not a name: {e}"))?;

    let chain_pem = read_pem(data, TLS_CERT)?;
    let key_pem = read_pem(data, TLS_KEY)?;
    let chain: Vec<_> = rustls_pemfile::certs(&mut chain_pem.as_slice())
        .collect::<Result<_, _>>()
        .map_err(|e| format!("{TLS_CERT}: {e}"))?;
    let key = rustls_pemfile::private_key(&mut key_pem.as_slice())
        .map_err(|e| format!("{TLS_KEY}: {e}"))?
        .ok_or_else(|| format!("{TLS_KEY} carries no private key"))?;

    rustls::ClientConfig::builder_with_provider(rivet_sdk::serve::provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("protocol versions: {e}"))?
        .with_root_certificates(roots)
        .with_client_auth_cert(chain, key)
        .map_err(|e| {
            format!(
                "{TLS_CERT} and {TLS_KEY} do not go together, so this proxy could not present an \
                 identity to the applications behind it (ADR-046 §4.2): {e}"
            )
        })
}

/// **The name the applications behind this proxy are verified against, and
/// where it came from** — `ADR-060` §3.
///
/// The `upstream-name` setting when there is one. Otherwise **the first DNS
/// name on this proxy's own certificate**: the fleet authority issued it for
/// the names this machine answers to, and every application here is served
/// the same certificate for the same name, so it is the name a backend
/// presents. It is read off the certificate rather than composed from the
/// hostname, because appending `.local` would assume a suffix `ADR-034` makes
/// a fleet property.
///
/// A certificate with no DNS name and no setting is refused, naming both.
fn upstream_name(
    setting: Option<String>,
    cert_pem: &[u8],
) -> Result<(String, &'static str), String> {
    if let Some(name) = setting {
        return Ok((name, "the upstream-name setting"));
    }
    let first = certificate_names(cert_pem)?.into_iter().next();
    first.map(|name| (name, "the first DNS name on this proxy's certificate")).ok_or_else(|| {
        format!(
            "the `{UPSTREAM_NAME}` setting is not set and {TLS_CERT} names no DNS host, so there \
             is no name to verify the applications behind this proxy against. Set it in the \
             machine document:\n\n    [applications.rivet-proxyd]\n    {UPSTREAM_NAME} = \
             \"<the name on the fleet certificate>\"\n\nor serve a certificate issued for this \
             machine's name (ADR-060)."
        )
    })
}

/// **The DNS names on this proxy's own certificate**, in the order it carries
/// them — the leaf's `subjectAltName`, read with `rivetos_tls`'s walker. The
/// one source for which names this machine answers at: [`upstream_name`]
/// takes the first, and the passkey relying party answers at each.
fn certificate_names(cert_pem: &[u8]) -> Result<Vec<String>, String> {
    let leaf = rustls_pemfile::certs(&mut &cert_pem[..])
        .next()
        .ok_or_else(|| format!("{TLS_CERT} carries no certificate"))?
        .map_err(|e| format!("{TLS_CERT}: {e}"))?;
    rivetos_tls::dns_names(&leaf)
        .ok_or_else(|| format!("{TLS_CERT} could not be read far enough to find its DNS names"))
}

fn read_pem(data: &std::path::Path, name: &str) -> Result<Vec<u8>, String> {
    std::fs::read(data.join(name)).map_err(|e| {
        format!(
            "{name}: {e}\n\nInstall it with:\n    rivet apply install-secret rivet-proxyd {name} \
             --hex <the PEM, as hex> confirm"
        )
    })
}

// ~~`rivet_provider`~~ **deleted 2026-09-05.** It returned
// `ring::default_provider()` with a comment saying it matched `rivet-tls` and
// `rivet-fleetd`, which is `ADR-011`'s *second answer with a note attached*
// rather than one answer — and `rivet-sdk/src/serve/tls.rs`'s module header
// names this function as the example. Both configurations above now take
// `rivet_sdk::serve::provider()`, which delegates to `rivet-tls`, which is
// where the provider is chosen.

// **`cmd/rivet-provision`'s authority module, compiled into this test build**,
// so the gate can be driven with certificates minted by the functions that mint
// real ones (`tests::handshake`'s provisioning-shaped callers). `cmd/cargo-rivet`
// takes `rivet-sdk/src/qoi.rs` the same way; neither crate has a library target.
#[cfg(test)]
#[path = "../../rivet-provision/src/ca.rs"]
mod provision_ca;

// **`rivet-webauthn`'s ceremony captured from Chrome on Windows**, compiled
// into this test build by path, so the relying party's state — the store, the
// challenge table, the name a request arrived at — is driven with bytes a real
// authenticator wrote.
#[cfg(test)]
#[path = "../../../rivet-webauthn/src/captured.rs"]
mod captured_ceremony;

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **A handed listener is adopted NON-BLOCKING.** The supervisor hands a
    /// socket over in its default, blocking mode, and a blocking listener
    /// under tokio parks the worker that accepts on it inside `accept(2)`.
    /// The task each accepted connection spawns then waits in that same
    /// worker's LIFO slot, which no other worker may steal, until the NEXT
    /// connection arrives — so port 80's first request is never answered.
    /// A non-blocking accept with nothing pending answers `WouldBlock` at
    /// once; a blocking one would wait for ever, which the channel turns into
    /// a failure rather than a hung test.
    #[test]
    fn a_handed_listener_is_adopted_non_blocking() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        let _inside = runtime.enter();
        let handed = std::net::TcpListener::bind("127.0.0.1:0").expect("a listener, blocking as handed");
        let adopted = adopt(handed, "tcp-80").expect("adopted");
        let back = adopted.into_std().expect("the descriptor back");
        let (sent, answer) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sent.send(back.accept().map(|_| ()).map_err(|e| e.kind()));
        });
        match answer.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok(Err(std::io::ErrorKind::WouldBlock)) => {}
            other => panic!("an accept with nothing pending did not answer WouldBlock at once: {other:?}"),
        }
    }

    #[test]
    fn a_request_line_yields_its_target() {
        assert_eq!(
            request_target("GET /reliquary/health HTTP/1.1\r\nHost: x\r\n\r\n").as_deref(),
            Some("/reliquary/health")
        );
    }

    /// An absolute-form target is what a client talking to a FORWARD proxy
    /// sends. This is a reverse proxy and answering one would be answering for
    /// a host it does not serve.
    #[test]
    fn anything_but_an_origin_form_target_is_refused() {
        for line in [
            "GET http://elsewhere/ HTTP/1.1\r\n\r\n",
            "CONNECT host:443 HTTP/1.1\r\n\r\n",
            "GET /x HTTP/0.9\r\n\r\n",
            "nonsense\r\n\r\n",
            "",
        ] {
            assert!(request_target(line).is_none(), "{line:?} must be refused");
        }
    }

    #[test]
    fn the_request_line_is_rewritten_to_the_remainder() {
        let head = "GET /reliquary/api/v1/blobs HTTP/1.1\r\nHost: rivet-01.local\r\n\r\n";
        let out = rewrite_head(head, "/api/v1/blobs", None);
        assert!(out.starts_with("GET /api/v1/blobs HTTP/1.1\r\n"), "{out}");
        assert!(out.contains("Host: rivet-01.local\r\n"), "{out}");
    }

    #[test]
    fn a_query_string_survives_the_rewrite() {
        let head = "GET /trends/api?since=1 HTTP/1.1\r\nHost: x\r\n\r\n";
        let out = rewrite_head(head, "/api", Some("since=1"));
        assert!(out.starts_with("GET /api?since=1 HTTP/1.1\r\n"), "{out}");
    }

    /// **A forwarded `keep-alive` would leave this program holding a socket it
    /// has stopped reading.** Every hop-by-hop header is this connection's,
    /// not the next one's. `Transfer-Encoding` is dropped here too, as this
    /// function's own contract for any head handed to it — but `handle`
    /// itself never reaches this function with one, because every request
    /// carrying a `Transfer-Encoding` is refused by `transfer_coding` first.
    #[test]
    fn hop_by_hop_headers_are_dropped_and_close_is_forced() {
        let head = "GET /x/y HTTP/1.1\r\nHost: h\r\nConnection: keep-alive\r\n\
                    Transfer-Encoding: chunked\r\nUpgrade: websocket\r\nAccept: */*\r\n\r\n";
        let out = rewrite_head(head, "/y", None);
        let lower = out.to_ascii_lowercase();
        assert_eq!(lower.matches("connection:").count(), 1, "{out}");
        assert!(lower.contains("connection: close"), "{out}");
        assert!(!lower.contains("keep-alive"), "{out}");
        assert!(!lower.contains("transfer-encoding"), "{out}");
        assert!(!lower.contains("upgrade:"), "{out}");
        assert!(out.contains("Accept: */*"), "an ordinary header must survive: {out}");
    }

    #[test]
    fn the_head_always_ends_with_a_blank_line() {
        let out = rewrite_head("GET /a/b HTTP/1.1\r\nHost: h\r\n\r\n", "/b", None);
        assert!(out.ends_with("\r\n\r\n"), "{out:?}");
    }

    // -----------------------------------------------------------------------
    // The gate, as a decision — ADR-046 §4.1, §4.3, §6
    // -----------------------------------------------------------------------

    fn route(path: &str, port: u16, requires_operator: bool) -> rivet_core::proxy::Route {
        rivet_core::proxy::Route {
            path: path.to_string(),
            service: format!("{path}-rivet"),
            port,
            requires_operator,
            rules: Vec::new(),
        }
    }

    /// One route with `ADR-048` §4 rules on it, as the manifest spells them.
    fn route_with(
        path: &str,
        port: u16,
        requires_operator: bool,
        rules: &[&str],
    ) -> rivet_core::proxy::Route {
        let mut rules: Vec<_> = rules
            .iter()
            .map(|text| {
                rivet_core::package::application::ProxyRule::parse(text)
                    .expect("a rule this test meant to write")
            })
            .collect();
        rules.sort_by(|a, b| a.prefix().cmp(b.prefix()));
        rivet_core::proxy::Route {
            rules,
            ..route(path, port, requires_operator)
        }
    }

    fn table() -> Vec<rivet_core::proxy::Route> {
        vec![route("notes", 8082, true), route("tempmon", 8081, false)]
    }

    /// The administration portal a FLEET SERVER offers — what `rivet-sysd` names
    /// in the table when `rivet-fleetd` is installed. A member passes `None`
    /// instead, and its page shows no portal row.
    fn portal() -> rivet_core::proxy::AdminPortal {
        rivet_core::proxy::AdminPortal {
            service: "rivet-fleetd".to_string(),
            port: rivet_core::proxy::ADMIN_PORTAL_PORT,
        }
    }

    /// [`super::decide`] for a request no other site caused — every test of the
    /// gate, the index and the redirect below. The cross-site guard has its own
    /// tests, which call `super::decide` directly.
    fn decide<'a>(
        routes: &'a [rivet_core::proxy::Route],
        path: &str,
        authenticated: bool,
    ) -> Decision<'a> {
        let presented = if authenticated {
            Presented::Operator
        } else {
            Presented::Nothing
        };
        super::decide(routes, path, presented, false)
    }

    /// Every posture a caller can arrive in.
    const EVERY_CALLER: [Presented; 4] = [
        Presented::Nothing,
        Presented::Operator,
        Presented::NotOperator,
        Presented::Passkey,
    ];

    // -----------------------------------------------------------------------
    // Passkeys beside client certificates — ADR-063 §4
    // -----------------------------------------------------------------------

    /// **A passkey session opens exactly the doors an operator certificate
    /// opens**, the index included — and nothing about the certificate path
    /// changed to allow it.
    #[test]
    fn a_passkey_session_is_admitted_wherever_the_operator_certificate_is() {
        let routes = table();
        for path in ["/", "/notes/api", "/notes", "/tempmon/api/v1/sensors", "/nothing/here"] {
            assert_eq!(
                super::decide(&routes, path, Presented::Passkey, false),
                super::decide(&routes, path, Presented::Operator, false),
                "{path}"
            );
        }
        assert_eq!(
            super::decide(&routes, "/notes/api", Presented::Nothing, false),
            Decision::Refuse(403, GATE_REFUSAL)
        );
    }

    /// **`/.rivet/` is the proxy's own, served to anybody, and no application
    /// can be mounted there**: a `proxy-path` begins with a lowercase letter.
    #[test]
    fn the_reserved_passkey_paths_are_served_to_anybody_and_cannot_be_claimed() {
        let routes = table();
        for presented in EVERY_CALLER {
            assert_eq!(
                super::decide(&routes, "/.rivet/passkey/login/options", presented, false),
                Decision::Passkey("/passkey/login/options".to_string())
            );
            assert_eq!(
                super::decide(&routes, "/.rivet", presented, false),
                Decision::Passkey(String::new())
            );
            assert!(matches!(
                super::decide(&routes, "/.rivetx/login", presented, false),
                Decision::Refuse(404, _)
            ));
        }
        for claim in [".rivet", "_rivet", "Rivet"] {
            assert!(!rivet_core::package::application::is_proxy_path(claim), "{claim}");
        }
    }

    /// **The sign-in pages are `/sign-in` and `/sign-in/enroll`, served to
    /// anybody, and no application can be mounted at `sign-in`** — `ADR-046`
    /// §4. A path that only begins with the same letters is an ordinary route
    /// lookup.
    #[test]
    fn the_sign_in_pages_are_served_to_anybody_and_cannot_be_claimed() {
        let routes = table();
        for presented in EVERY_CALLER {
            for (path, rest) in [
                ("/sign-in", ""),
                ("/sign-in/", "/"),
                ("/sign-in/enroll", "/enroll"),
                ("/sign-in/enroll/", "/enroll/"),
                ("/sign-in/elsewhere", "/elsewhere"),
            ] {
                assert_eq!(
                    super::decide(&routes, path, presented, false),
                    Decision::SignIn(rest.to_string()),
                    "{path} {presented:?}"
                );
            }
            assert!(matches!(
                super::decide(&routes, "/sign-inx", presented, false),
                Decision::Refuse(404, _)
            ));
        }
        assert!(!rivet_core::package::application::is_mountable_proxy_path("sign-in"));
        assert_eq!(passkey::SIGN_IN_PAGE, "/sign-in");
        assert_eq!(passkey::ENROLL_PAGE, "/sign-in/enroll");
    }

    /// **The old addresses answer a permanent redirect to the new ones**, for
    /// every caller and before the gate, since they are how a caller with no
    /// credential obtains one. `handle` carries the query string across.
    #[test]
    fn the_old_sign_in_addresses_move_to_the_new_ones() {
        let routes = table();
        for presented in EVERY_CALLER {
            for (old, new) in [
                ("/.rivet/login", "/sign-in"),
                ("/.rivet/login/", "/sign-in"),
                ("/.rivet/enrol", "/sign-in/enroll"),
                ("/.rivet/enrol/", "/sign-in/enroll"),
            ] {
                assert_eq!(
                    super::decide(&routes, old, presented, false),
                    Decision::Moved(new.to_string()),
                    "{old} {presented:?}"
                );
            }
        }
    }

    /// The two guards before it bind the reserved paths too.
    #[test]
    fn the_reserved_paths_are_behind_the_traversal_and_cross_site_guards() {
        let routes = table();
        assert_eq!(
            super::decide(&routes, "/.rivet/passkey/login", Presented::Nothing, true),
            Decision::RefuseCrossSite
        );
        assert!(matches!(
            super::decide(&routes, "/.rivet/../notes/api", Presented::Nothing, false),
            Decision::Refuse(400, _)
        ));
    }

    /// **The session cookie never reaches an application**, and any other
    /// cookie does, untouched.
    #[test]
    fn the_session_cookie_is_stripped_before_a_backend_sees_the_head() {
        let head = "GET /tempmon/ HTTP/1.1\r\nHost: x\r\nCookie: theme=dark; rivet-session=abc\r\n\
                    Cookie: rivet-session=def\r\nCookie: other=1\r\n\r\n";
        let out = rewrite_head(head, "/", None);
        assert!(!out.contains("rivet-session"), "{out}");
        assert!(out.contains("Cookie: theme=dark\r\n"), "{out}");
        assert!(out.contains("Cookie: other=1\r\n"), "{out}");
    }

    /// A browser asking for a page is sent to the login page; anything else
    /// still gets the words.
    #[test]
    fn only_a_get_that_asks_for_html_is_a_page_request() {
        assert!(wants_a_page("GET / HTTP/1.1\r\nAccept: text/html,*/*\r\n\r\n"));
        assert!(!wants_a_page("GET / HTTP/1.1\r\nAccept: */*\r\n\r\n"));
        assert!(!wants_a_page("GET / HTTP/1.1\r\n\r\n"));
        assert!(!wants_a_page("POST / HTTP/1.1\r\nAccept: text/html\r\n\r\n"));
        assert_eq!(percent_encode("/notes/a b?x=1&y"), "/notes/a%20b%3Fx%3D1%26y");
    }

    // -----------------------------------------------------------------------
    // `decide_plain` — port 80, ADR-061 §7
    // -----------------------------------------------------------------------

    #[test]
    fn a_plain_rule_is_served_over_get_and_head() {
        let route = route_with("gantry", 8080, true, &["plain:install"]);
        for method in ["GET", "HEAD"] {
            match decide_plain(std::slice::from_ref(&route), "/gantry/install/apk", method) {
                PlainDecision::Forward(r, rest) => {
                    assert_eq!(r.service, "gantry-rivet");
                    assert_eq!(rest, "/install/apk");
                }
                other => panic!("{method}: expected Forward, got {other:?}"),
            }
        }
    }

    /// Everything that is not a `plain:` route is `404` — the index, a route
    /// with no matching rule, a gated island under an otherwise plain route,
    /// and a path nothing serves at all.
    #[test]
    fn everything_but_a_plain_rule_is_404_on_port_80() {
        let routes = vec![
            route_with("gantry", 8080, true, &["plain:install", "operator:install/admin"]),
            route("notes", 8082, true),
        ];
        for path in [
            "/",
            "/gantry",
            "/gantry/other",
            "/gantry/install/admin",
            "/notes/api",
            "/nothing/here",
        ] {
            match decide_plain(&routes, path, "GET") {
                PlainDecision::Refuse(404, _) => {}
                other => panic!("{path}: expected 404, got {other:?}"),
            }
        }
    }

    #[test]
    fn a_write_to_a_plain_rule_is_405() {
        let route = route_with("gantry", 8080, true, &["plain:install"]);
        for method in ["POST", "PUT", "DELETE", "PATCH"] {
            match decide_plain(std::slice::from_ref(&route), "/gantry/install/apk", method) {
                PlainDecision::Refuse(405, _) => {}
                other => panic!("{method}: expected 405, got {other:?}"),
            }
        }
    }

    /// The traversal refusal answers before the route lookup, exactly as on
    /// 443 — `ADR-048` §5.
    #[test]
    fn a_traversal_is_refused_400_before_the_route_lookup() {
        let route = route_with("gantry", 8080, true, &["plain:install"]);
        match decide_plain(&[route], "/gantry/install/../etc", "GET") {
            PlainDecision::Refuse(400, _) => {}
            other => panic!("expected the traversal refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_request_line_yields_its_method_and_target() {
        assert_eq!(
            request_line("GET /reliquary/health HTTP/1.1\r\nHost: x\r\n\r\n"),
            Some(("GET", "/reliquary/health"))
        );
        assert_eq!(
            request_line("HEAD /gantry/install HTTP/1.1\r\n\r\n"),
            Some(("HEAD", "/gantry/install"))
        );
        assert_eq!(request_line("nonsense\r\n\r\n"), None);
    }

    // -----------------------------------------------------------------------
    // The cross-site guard, as a decision — ADR-046 §4, `.rulings/` R-186 D1
    //
    // `rivet_core::cross_site`'s own tests are the decision table over methods
    // and headers. These are about WHERE the answer lands in the gate's order.
    // -----------------------------------------------------------------------

    /// **Every path, every posture, refused.** A gated route, a public one, an
    /// open prefix, the index, a bare prefix that would redirect, and a path
    /// nothing serves — with and without a certificate.
    #[test]
    fn a_cross_site_write_is_refused_whatever_the_route_and_whoever_called() {
        let mut routes = table();
        routes.push(route_with("reliquary", 8443, true, &["none:pair"]));
        for path in [
            "/notes/api/import",
            "/tempmon/api/v1/restore",
            "/reliquary/restore",
            "/reliquary/pair/redeem",
            "/",
            "/notes",
            "/nothing/here",
        ] {
            for presented in EVERY_CALLER {
                assert_eq!(
                    super::decide(&routes, path, presented, true),
                    Decision::RefuseCrossSite,
                    "{path} ({presented:?})"
                );
            }
        }
    }

    /// The traversal guard still answers first: a path that cannot be matched
    /// is a `400` whatever else is wrong with the request.
    #[test]
    fn the_traversal_guard_still_answers_before_the_cross_site_guard() {
        match super::decide(&table(), "/notes/../tempmon", Presented::Operator, true) {
            Decision::Refuse(400, _) => {}
            other => panic!("expected the traversal refusal, got {other:?}"),
        }
    }

    /// **Wired to the real header rule**, not only to a boolean: the head the
    /// attack sends is refused and the head TempMon's own page sends is
    /// forwarded.
    #[test]
    fn the_guard_is_asked_of_the_request_head_the_way_handle_asks_it() {
        let routes = table();
        let attack = "POST /tempmon/api/v1/restore?confirm=true HTTP/1.1\r\nHost: rivet-01.local\r\n\
                      Sec-Fetch-Site: cross-site\r\nContent-Type: text/plain\r\n\r\n";
        let page = "POST /tempmon/api/v1/restore?confirm=true HTTP/1.1\r\nHost: rivet-01.local\r\n\
                    Sec-Fetch-Site: same-origin\r\nOrigin: https://rivet-01.local\r\n\
                    Content-Type: application/vnd.sqlite3\r\n\r\n";
        let flag = |head: &str| rivet_core::cross_site::refuses(head, "https");
        assert_eq!(
            super::decide(&routes, "/tempmon/api/v1/restore", Presented::Nothing, flag(attack)),
            Decision::RefuseCrossSite
        );
        assert!(matches!(
            super::decide(&routes, "/tempmon/api/v1/restore", Presented::Nothing, flag(page)),
            Decision::Forward(..)
        ));
    }

    /// The refusal is for the operator reading it, so it names what decided.
    #[test]
    fn the_cross_site_refusal_names_the_headers_that_decided() {
        assert!(CROSS_SITE_REFUSAL.contains("Sec-Fetch-Site"), "{CROSS_SITE_REFUSAL}");
        assert!(CROSS_SITE_REFUSAL.contains("Origin"), "{CROSS_SITE_REFUSAL}");
        assert!(CROSS_SITE_REFUSAL.contains("another site"), "{CROSS_SITE_REFUSAL}");
    }

    /// **`ADR-046` §4.1.** The route did not say otherwise, so an operator is
    /// required and a caller without one never reaches the application.
    #[test]
    fn a_gated_route_refuses_a_caller_with_no_certificate() {
        let routes = table();
        assert_eq!(
            decide(&routes, "/notes/api", false),
            Decision::Refuse(403, GATE_REFUSAL)
        );
        match decide(&routes, "/notes/api", true) {
            Decision::Forward(r, rest) => {
                assert_eq!(r.service, "notes-rivet");
                assert_eq!(rest, "/api");
            }
            other => panic!("an authenticated caller must be forwarded, got {other:?}"),
        }
    }

    /// **`ADR-046` §4, and the one worked example of `.rulings/` R-152**: TempMon
    /// may be served over TLS with no authentication.
    #[test]
    fn a_route_that_declared_proxy_auth_none_is_served_to_anybody() {
        let routes = table();
        for authenticated in [false, true] {
            match decide(&routes, "/tempmon/api/v1/sensors", authenticated) {
                Decision::Forward(r, rest) => {
                    assert_eq!(r.service, "tempmon-rivet");
                    assert_eq!(rest, "/api/v1/sensors");
                }
                other => panic!("a public route must be forwarded, got {other:?}"),
            }
        }
    }

    /// **The defect this exists for**: a prefix opened without its trailing
    /// slash drew an enormous logo on an unstyled page (.rulings/ R-342).
    ///
    /// An application mounted at a prefix can only emit RELATIVE links
    /// (`ADR-046` §8), and a relative link resolves against the request URL up
    /// to its last `/`. Without the trailing slash that base is the machine
    /// root, so every stylesheet, script and API call goes somewhere nothing
    /// serves — and forwarding answers `200` to a request that cannot work.
    #[test]
    fn a_prefix_without_its_trailing_slash_is_redirected_and_not_served() {
        let routes = table();
        assert_eq!(
            decide(&routes, "/notes", true),
            Decision::Redirect("/notes/".to_string())
        );
        // With the slash it is the application's, untouched.
        match decide(&routes, "/notes/", true) {
            Decision::Forward(r, rest) => {
                assert_eq!(r.service, "notes-rivet");
                assert_eq!(rest, "/");
            }
            other => panic!("the slashed form must forward, got {other:?}"),
        }
    }

    /// **After the gate**, so both spellings reveal exactly the same thing to a
    /// caller who has presented nothing.
    #[test]
    fn a_bare_prefix_on_a_gated_route_still_refuses_a_caller_with_no_certificate() {
        let routes = table();
        assert_eq!(
            decide(&routes, "/notes", false),
            Decision::Refuse(403, GATE_REFUSAL)
        );
        // A public route has no gate to be after, so it redirects for anybody.
        assert_eq!(
            decide(&routes, "/tempmon", false),
            Decision::Redirect("/tempmon/".to_string())
        );
    }

    /// A deeper path is the application's business and is never redirected —
    /// `/notes/api` is a resource, not a directory this proxy knows about.
    #[test]
    fn only_the_bare_prefix_is_redirected() {
        let routes = table();
        for path in ["/notes/", "/notes/api", "/notes/api/"] {
            assert!(
                matches!(decide(&routes, path, true), Decision::Forward(..)),
                "{path} must be forwarded"
            );
        }
    }

    /// **`ADR-046` §6.** The list of what a machine runs is not public.
    #[test]
    fn the_index_is_behind_the_gate_like_everything_else() {
        let routes = table();
        assert_eq!(decide(&routes, "/", false), Decision::Refuse(403, GATE_REFUSAL));
        assert_eq!(decide(&routes, "/", true), Decision::Index);
    }

    /// **A certificate that is not the operator's is refused wherever the
    /// operator is required, and served wherever nobody is** — `.rulings/`
    /// R-187 D1. The index, a gated route and its bare prefix refuse it; a
    /// public route, its bare prefix and an open prefix under a gated route
    /// serve it as they would serve no certificate; a path nothing serves is
    /// still `404` and a traversal still `400`.
    #[test]
    fn a_certificate_naming_a_host_passes_only_where_no_certificate_would() {
        let mut routes = table();
        routes.push(route_with("reliquary", 8443, true, &["none:pair"]));
        let machine = |path: &str| super::decide(&routes, path, Presented::NotOperator, false);

        for gated in ["/", "/notes/api", "/notes", "/reliquary/blobs"] {
            assert_eq!(machine(gated), Decision::RefuseNotOperator, "{gated}");
        }
        for open in ["/tempmon/api/v1/sensors", "/reliquary/pair/redeem"] {
            assert!(matches!(machine(open), Decision::Forward(..)), "{open}");
        }
        assert_eq!(machine("/tempmon"), Decision::Redirect("/tempmon/".to_string()));
        assert!(matches!(machine("/nothing/here"), Decision::Refuse(404, _)));
        assert!(matches!(machine("/notes/../tempmon"), Decision::Refuse(400, _)));
    }

    /// What a verified certificate classifies as, at the gate.
    #[test]
    fn only_a_certificate_naming_no_host_is_presented_as_the_operator() {
        use rivetos_tls::Caller;
        assert_eq!(Presented::of(None), Presented::Nothing);
        assert_eq!(Presented::of(Some(&Caller::Operator)), Presented::Operator);
        assert_eq!(
            Presented::of(Some(&Caller::Machine(vec!["rivet-02.local".to_owned()]))),
            Presented::NotOperator
        );
        assert_eq!(Presented::of(Some(&Caller::Unreadable)), Presented::NotOperator);
    }

    /// **404 before 403, and the order is deliberate** — see `decide`.
    #[test]
    fn a_path_nothing_serves_is_a_404_whoever_asks() {
        let routes = table();
        for authenticated in [false, true] {
            match decide(&routes, "/nothing/here", authenticated) {
                Decision::Refuse(404, _) => {}
                other => panic!("expected 404, got {other:?}"),
            }
        }
    }

    // -----------------------------------------------------------------------
    // Per-path auth at the gate — ADR-048 §4 and §5
    //
    // `rivet_core::proxy`'s tests attack the MATCHER. These attack the
    // DECISION, because a correct matcher consulted at the wrong moment — after
    // a redirect, or on a path the proxy was about to normalise — is a hole
    // with a green unit test behind it.
    // -----------------------------------------------------------------------

    /// Reliquary's shape: a gated application with one open door for a phone.
    fn phone_door() -> Vec<rivet_core::proxy::Route> {
        vec![route_with("reliquary", 8443, true, &["none:pair"])]
    }

    /// The door opens, and nothing else on the application does.
    #[test]
    fn an_open_prefix_is_served_with_no_certificate_and_the_rest_is_not() {
        let routes = phone_door();
        assert!(matches!(
            decide(&routes, "/reliquary/pair/redeem", false),
            Decision::Forward(..)
        ));
        assert_eq!(
            decide(&routes, "/reliquary/blobs", false),
            Decision::Refuse(403, GATE_REFUSAL)
        );
        assert!(matches!(
            decide(&routes, "/reliquary/blobs", true),
            Decision::Forward(..)
        ));
    }

    /// **The three sibling attacks, at the gate rather than at the matcher.**
    /// A `starts_with` opens the first two and nothing about the served page
    /// looks wrong.
    #[test]
    fn a_neighbour_of_the_open_prefix_is_refused_at_the_gate() {
        let routes = phone_door();
        for hostile in [
            "/reliquary/pairings",
            "/reliquary/pair-admin",
            "/reliquary/Pair",
            "/reliquary/%70air",
            "/reliquary/pair-admin/revoke",
        ] {
            assert_eq!(
                decide(&routes, hostile, false),
                Decision::Refuse(403, GATE_REFUSAL),
                "{hostile} must not be served without a certificate"
            );
        }
    }

    /// **`ADR-048` §5, and the reason the guard is before everything.**
    /// `/reliquary/pair/../blobs` matches the open prefix segment by segment;
    /// a backend that resolves the dot segment serves the records.
    #[test]
    fn a_traversal_through_the_open_prefix_is_refused_before_any_route_is_found() {
        let routes = phone_door();
        for hostile in [
            "/reliquary/pair/../blobs",
            "/reliquary/pair/%2e%2e/blobs",
            "/reliquary/pair%2fadmin",
        ] {
            match decide(&routes, hostile, false) {
                Decision::Refuse(400, _) => {}
                other => panic!("{hostile} must be refused 400, got {other:?}"),
            }
        }
        // **It binds an authenticated caller and an unrouted path too**, because
        // it is a property of the request rather than of the gate's verdict.
        match decide(&routes, "/reliquary/pair/../blobs", true) {
            Decision::Refuse(400, _) => {}
            other => panic!("a certificate does not make a traversal matchable, got {other:?}"),
        }
        match decide(&routes, "/nothing/../here", true) {
            Decision::Refuse(400, _) => {}
            other => panic!("the guard is before the route lookup, got {other:?}"),
        }
    }

    /// **The redirect is still after the gate, and the gate is now per path.**
    /// `/reliquary` bare is the application's root, which no open prefix
    /// covers — so an anonymous caller gets the gate's answer rather than a
    /// `308` that would have told them the route exists.
    #[test]
    fn the_bare_prefix_takes_the_routes_own_gate_and_not_an_open_prefix_under_it() {
        let routes = phone_door();
        assert_eq!(
            decide(&routes, "/reliquary", false),
            Decision::Refuse(403, GATE_REFUSAL)
        );
        assert_eq!(
            decide(&routes, "/reliquary", true),
            Decision::Redirect("/reliquary/".to_string())
        );
        // And the open prefix's own bare form redirects for anybody, because
        // the gate it is after has already let them through.
        assert!(matches!(
            decide(&routes, "/reliquary/pair", false),
            Decision::Forward(..)
        ));
    }

    /// Longest match wins at the gate as well: a gated island inside the open
    /// door still refuses.
    #[test]
    fn a_gated_prefix_under_the_open_door_still_refuses() {
        let routes = vec![route_with(
            "reliquary",
            8443,
            true,
            &["none:pair", "operator:pair/admin"],
        )];
        assert!(matches!(
            decide(&routes, "/reliquary/pair/redeem", false),
            Decision::Forward(..)
        ));
        assert_eq!(
            decide(&routes, "/reliquary/pair/admin", false),
            Decision::Refuse(403, GATE_REFUSAL)
        );
    }

    /// The refusal has to be actionable: its reader is an operator at a browser
    /// that declined a certificate prompt, and *Forbidden* alone sends them
    /// looking for a password.
    #[test]
    fn the_refusal_says_what_to_present() {
        assert!(GATE_REFUSAL.contains("operator certificate"), "{GATE_REFUSAL}");
        assert!(GATE_REFUSAL.contains("fleet authority"), "{GATE_REFUSAL}");
        assert!(GATE_REFUSAL.contains("browser"), "{GATE_REFUSAL}");
    }

    // -----------------------------------------------------------------------
    // The index — ADR-046 §6
    // -----------------------------------------------------------------------

    #[test]
    fn the_index_lists_every_route_as_a_link_and_names_the_public_ones() {
        let page = index_page(&table(), Some(&portal()), "rivet-01.local", Some("09/06/2026 11:24 UTC"), Some("operator"));
        assert!(page.starts_with("<!doctype html>"), "{page}");
        assert!(page.contains("<title>rivet-01.local</title>"), "{page}");
        assert!(page.contains("href=\"/notes/\""), "{page}");
        assert!(page.contains("href=\"/tempmon/\""), "{page}");
        assert!(page.contains("notes-rivet"), "{page}");
    }

    /// **A row shows the whole address and links to the path.** `/aura/` alone
    /// is not something a person can give to anybody, and the maintainer asked
    /// for the machine name in front of it on 2026-09-06 — but the `href` is
    /// already on this machine and stays root-relative.
    #[test]
    fn a_row_displays_the_full_address_and_links_relatively() {
        let page = index_page(&table(), Some(&portal()), "rivet-01.local", None, None);
        assert!(page.contains(">rivet-01.local/notes/</span>"), "{page}");
        assert!(page.contains("href=\"/notes/\""), "{page}");
        assert!(
            !page.contains("href=\"rivet-01.local/notes/\""),
            "the link must not become an absolute URL: {page}"
        );
    }

    /// The machine name comes from a setting, so it is escaped in the row for
    /// the same reason it is escaped in the heading.
    #[test]
    fn a_hostile_machine_name_is_escaped_in_the_row_as_well_as_the_heading() {
        let page = index_page(&table(), None, "<script>alert(1)</script>", None, None);
        assert!(!page.contains("<script>alert(1)</script>"), "{page}");
        assert!(page.contains("&lt;script&gt;alert(1)&lt;/script&gt;/notes/"), "{page}");
    }

    /// **Every row states what it HAS**, which is the point of replacing a
    /// `NO OPERATOR` label on 2026-09-06: an absence said nothing on the
    /// ordinary row and left the reader to know that silence meant gated.
    #[test]
    fn every_route_shows_a_padlock_and_only_a_gated_one_shows_a_person() {
        let page = index_page(&table(), Some(&portal()), "rivet-01.local", None, None);
        // **The ROUTED list only** — the administration row below it has a
        // padlock and a person of its own, and counting those here would make
        // this test about two lists at once.
        let routed = &page[..page.find("</ul>").expect("the routed list ends")];
        // `table()` is a gated notes and a public tempmon, in that order.
        let tempmon = routed.find("/tempmon/").expect("the tempmon row");
        let notes = routed.find("/notes/").expect("the notes row");
        let gated = &routed[notes..tempmon];
        let public = &routed[tempmon..];

        // `aria-label`, not the bare words — each icon carries them twice.
        assert_eq!(
            routed.matches("aria-label=\"Endpoint secured by SSL\"").count(),
            2,
            "{routed}"
        );
        assert!(gated.contains("Endpoint requires user authentication certificate"), "{gated}");
        assert!(
            !public.contains("Endpoint requires user authentication certificate"),
            "a public route must not claim to require a certificate: {public}"
        );
    }

    /// The badge is icons and native tooltips — no label text to translate, no
    /// script to run, and nothing fetched.
    #[test]
    fn the_posture_badge_is_self_contained_and_carries_its_words_twice() {
        let gated = posture_badge(true, &[]);
        let public = posture_badge(false, &[]);
        // Once for a pointer, once for a reader that does not hover.
        assert_eq!(gated.matches("Endpoint secured by SSL").count(), 2, "{gated}");
        assert!(gated.contains("aria-label=\"Endpoint secured by SSL\""), "{gated}");
        assert!(gated.contains("<title>Endpoint secured by SSL</title>"), "{gated}");
        // A padlock on both; a person on one.
        assert_eq!(public.matches("<svg").count(), 1, "{public}");
        assert_eq!(gated.matches("<svg").count(), 2, "{gated}");
        assert!(!public.contains("NO OPERATOR"), "{public}");
    }

    /// **A gated row with an open door in it says so** — `ADR-048` §4. A badge
    /// that showed only the route's ROOT verdict would report *authenticated*
    /// about an application with a public prefix on it, which is the one row on
    /// this page a reader most needs to be able to see.
    #[test]
    fn a_route_with_an_open_prefix_carries_a_third_icon_naming_it() {
        let door = [String::from("/reliquary/pair")];
        let mixed = posture_badge(true, &door);
        assert_eq!(mixed.matches("<svg").count(), 3, "{mixed}");
        assert!(
            mixed.contains("Served without a user authentication certificate: /reliquary/pair"),
            "{mixed}"
        );
        // The route is still gated, so the person icon stays: both facts are
        // true of it at once and the badge must not pick one.
        assert!(mixed.contains("Endpoint requires user authentication certificate"), "{mixed}");
        // And a route with no open prefix gains nothing.
        assert!(!posture_badge(true, &[]).contains("Served without"));
    }

    /// The page, not the badge: an open prefix reaches the list an operator
    /// came to read, with its whole address rather than its suffix.
    #[test]
    fn the_index_names_an_open_prefix_beside_the_route_it_sits_under() {
        let page = index_page(
            &[route_with("reliquary", 8443, true, &["none:pair"])],
            Some(&portal()),
            "rivet-01.local",
            None,
            Some("operator"),
        );
        assert!(
            page.contains("Served without a user authentication certificate: /reliquary/pair"),
            "{page}"
        );
    }

    /// **The footer names where each of its two facts came from**, which is the
    /// shape the maintainer asked for on 2026-09-06.
    #[test]
    fn the_footer_names_when_the_table_was_derived_and_who_is_connected() {
        let page = index_page(
            &table(),
            Some(&portal()),
            "rivet-01.local",
            Some("09/06/2026 11:24 UTC"),
            Some("operator"),
        );
        assert!(page.contains("Source: derived from installed application manifests"), "{page}");
        // **The two values are bold and the labels are not** — they are the only
        // things on the line that change.
        assert!(page.contains("<strong>09/06/2026 11:24 UTC</strong>"), "{page}");
        assert!(page.contains("Authenticated as: <strong>operator</strong>"), "{page}");
        assert!(!page.contains("<strong>Source"), "the label must not be bold: {page}");
    }

    /// **No count.** It said *"N applications, routed by path"* until
    /// 2026-09-06, and the maintainer's answer was that he could count the
    /// entries himself (.rulings/ R-342).
    #[test]
    fn the_index_does_not_count_its_own_list() {
        let page = index_page(
            &table(),
            Some(&portal()),
            "rivet-01.local",
            Some("09/06/2026 11:24 UTC"),
            Some("operator"),
        );
        assert!(!page.contains("applications, routed"), "{page}");
        assert!(!page.contains("2 application"), "{page}");
    }

    /// Neither missing fact may leave a blank where a value goes: a public
    /// route is reached with no certificate at all, and a certificate with no
    /// `commonName` is legal.
    #[test]
    fn a_missing_timestamp_or_name_reads_as_a_sentence_rather_than_a_gap() {
        let page = index_page(&table(), Some(&portal()), "rivet-01.local", None, None);
        assert!(page.contains("Source: derived from installed application manifests."), "{page}");
        assert!(page.contains("Authenticated as: a certificate carrying no common name"), "{page}");
        assert!(!page.contains("Authenticated as: ."), "{page}");
    }

    /// The caller's name comes off a certificate somebody else wrote, so it is
    /// escaped like every other value on this page.
    #[test]
    fn a_hostile_name_on_a_certificate_is_escaped() {
        let page = index_page(&[], Some(&portal()), "rivet-01.local", None, Some("<script>alert(1)</script>"));
        assert!(!page.contains("<script>alert(1)</script>"), "{page}");
        assert!(page.contains("&lt;script&gt;"), "{page}");
    }

    /// **No framework and no CDN**, because the board is offline — `ADR-046`
    /// §6. This is the assertion that catches somebody adding a convenience.
    ///
    /// **It bans what FETCHES, not the substring `https://`.** It banned the
    /// substring until 2026-09-06, when the page grew a link to the
    /// administration portal on `:4443` — a different port, so it cannot be
    /// written relatively. That is a place for a person to *click*, and a
    /// navigation is not a subresource: nothing is fetched while this page
    /// renders. The distinction is asserted below rather than assumed, by
    /// requiring every absolute URL on the page to be an `<a href>` naming this
    /// same machine.
    #[test]
    fn the_index_fetches_nothing_and_runs_nothing() {
        let page = index_page(&table(), Some(&portal()), "rivet-01.local", Some("09/06/2026 11:24 UTC"), Some("operator"));
        // Every one of these causes a fetch, or runs something, or both.
        for forbidden in ["<script", "<link", "src=", "url(", "@import", "//cdn"] {
            assert!(
                !page.contains(forbidden),
                "the index must embed what it serves; found {forbidden:?}"
            );
        }
        assert!(page.contains("prefers-color-scheme"), "{page}");
    }

    /// The one absolute URL the page is allowed is a link a person clicks, and
    /// it points at this same machine.
    #[test]
    fn the_only_absolute_url_is_a_link_to_this_machines_own_administration_port() {
        let page = index_page(&table(), Some(&portal()), "rivet-01.local", None, None);
        assert!(!page.contains("http://"), "nothing on this page is plaintext: {page}");

        let mut rest = page.as_str();
        let mut found = 0;
        while let Some(at) = rest.find("https://") {
            // It must be the value of an `<a href>`, and nothing else.
            let before = &rest[..at];
            assert!(
                before.ends_with("<a href=\""),
                "an absolute URL that is not a link: …{}",
                before.chars().rev().take(30).collect::<String>()
            );
            let value = &rest[at..];
            assert!(
                value.starts_with("https://rivet-01.local:4443/"),
                "an absolute URL leaving this machine: {}",
                value.chars().take(60).collect::<String>()
            );
            found += 1;
            rest = &rest[at + 8..];
        }
        assert_eq!(found, 1, "exactly one, and it is the administration portal");
    }

    /// **The administration portal is a row in a SECOND list**, which is the
    /// whole of how it is both legible and honest.
    ///
    /// It is a row because the maintainer asked twice where fleet was, and then
    /// — when it was a paragraph — because nobody wants to read prose like that
    /// every time they log in (.rulings/ R-342). It is a **second** list because
    /// putting it in the routed one would say *this is proxied*, and that is the
    /// one thing untrue of it.
    #[test]
    fn the_administration_portal_is_a_row_in_its_own_list_and_not_in_the_routed_one() {
        let page = index_page(&table(), Some(&portal()), "rivet-01.local", None, None);

        let routed_ends = page.find("</ul>").expect("the routed list ends");
        let portal = page.find("rivet-01.local:4443/").expect("the portal row");
        assert!(portal > routed_ends, "the portal is inside the routed list: {page}");
        assert!(page.contains("<ul class=\"direct\">"), "{page}");
        assert!(page.contains("rivet-fleetd"), "{page}");

        // It reads like the rows above it: an address, a posture, a service.
        let row = &page[portal..];
        assert!(row.contains("class=\"path\""), "{row}");
        assert!(row.contains("class=\"posture\""), "{row}");
        assert!(row.contains("class=\"svc\""), "{row}");
        // Both icons, because the portal verifies an operator certificate of
        // its own rather than inheriting the gate in front of the routed list.
        assert!(row.contains("Endpoint requires user authentication certificate"), "{row}");
    }

    /// **No paragraph.** The difference is carried by the frame and by a
    /// tooltip for whoever wants it, not by prose on every visit.
    #[test]
    fn the_page_does_not_explain_the_portal_in_prose() {
        let page = index_page(&table(), Some(&portal()), "rivet-01.local", None, None);
        for lecture in [
            "must not depend on what it administers",
            "deliberately not proxied",
            "Administration is at",
        ] {
            assert!(!page.contains(lecture), "the page lectures: {lecture:?}");
        }
        // The sentence survives where it costs nothing: a hover.
        assert!(
            page.contains("title=\"Reached directly rather than through this proxy\""),
            "{page}"
        );
    }

    /// **A MEMBER offers no administration portal, and its page shows none.**
    /// This is the honesty the page's footer claims and the bug this closes: the
    /// `rivet-fleetd` row used to render on every machine, drawing a fleet
    /// server on a member that runs none. `None` is what a member's table yields
    /// — `rivet-sysd` names no portal when `rivet-fleetd` is not installed — and
    /// the page must then have no direct list, no `:4443`, and no `rivet-fleetd`.
    #[test]
    fn a_member_with_no_portal_shows_no_administration_row() {
        let page = index_page(&table(), None, "dev-01.local", None, Some("operator"));
        assert!(
            !page.contains("class=\"direct\""),
            "a member drew the administration frame: {page}"
        );
        assert!(!page.contains("rivet-fleetd"), "a member named a fleet server: {page}");
        assert!(!page.contains(":4443"), "a member linked an admin port: {page}");
        // The routed applications it DOES have are still there — the fix removes
        // only the false row, not the honest ones.
        assert!(page.contains("dev-01.local/notes/"), "{page}");
    }

    /// **A machine with no routes is the ordinary machine** and the page says
    /// so, rather than drawing an empty box that reads like a fault.
    #[test]
    fn an_index_with_no_routes_explains_itself() {
        let page = index_page(&[], Some(&portal()), "rivet-01.local", Some("09/06/2026 11:24 UTC"), Some("operator"));
        assert!(page.contains("Nothing on this machine asks to be proxied"), "{page}");
        assert!(page.contains("ordinary state"), "{page}");
        assert!(!page.contains("<ul>"), "there is no list to draw: {page}");
    }

    /// Every value on this page comes from a signed manifest, so this is not a
    /// hostile-input path — and a value is escaped where it is emitted, not
    /// where somebody argues it cannot be dangerous.
    #[test]
    fn a_value_is_escaped_where_it_reaches_the_page() {
        let page = index_page(&[], None, "<script>alert(1)</script>", None, Some("<script>alert(2)</script>"));
        assert!(!page.contains("<script>"), "{page}");
        assert!(page.contains("&lt;script&gt;"), "{page}");
    }

    // -----------------------------------------------------------------------
    // A caller field colliding with the SDK's own drop counter — this crate
    // depends on both rivetos-sdk (it sends journal lines) and rivetos-core
    // (it calls rivet_core::journal::human above), so this is where the two
    // meet: the exact bytes rivet-sdk writes, through the exact parser and
    // ingest boundary the supervisor uses on them.
    // -----------------------------------------------------------------------

    /// **A caller field literally named `sdk_dropped_fields` must not make
    /// the writer's own counter collide with it on the wire.** Before the
    /// fix, the line carried `"sdk_dropped_fields"` twice —
    /// `Record::from_jsonl` refuses a duplicate custom key, so
    /// `rivet_core::journal::ingest` demoted this ERROR event to an
    /// ingestion WARNING and buried the real message inside a raw-bytes
    /// blob instead of leaving it queryable as `rivet-proxyd-test`'s own
    /// error.
    #[test]
    fn a_caller_field_named_like_the_sdk_counter_still_parses_and_keeps_its_severity() {
        let journal = rivet_sdk::journal::Journal::for_service("rivet-proxyd-test");
        let line = journal.line(
            rivet_sdk::journal::Severity::Error,
            "something bad happened",
            &[
                ("sdk_dropped_fields", rivet_sdk::journal::Value::Int(99)),
                ("svc", rivet_sdk::journal::Value::Int(1)),
            ],
        );

        let peer = rivet_core::journal::Peer { uid: 1000, gid: 1000, pid: 4242 };
        let ingested = rivet_core::journal::ingest(&line, &peer);

        assert_eq!(
            ingested.sev,
            rivet_core::journal::Severity::Error,
            "a parseable record must not be demoted to an ingestion warning: {ingested:?}"
        );
        assert_eq!(ingested.svc, "rivet-proxyd-test");
        assert_eq!(ingested.msg, "something bad happened");
    }

    // -----------------------------------------------------------------------
    // The gate, over a real handshake — ADR-046 §4.1, §4.2
    //
    // **Three callers are three tests.** A gate nobody has watched refuse a
    // certificate-less connection is not a gate that has been tested, and the
    // decision above is only half of it: the other half is that `rustls` was
    // configured to ASK, to verify against the fleet authority, and to tell
    // this program the answer. Everything below drives real TLS in-process.
    // -----------------------------------------------------------------------

    mod handshake {
        #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

        use super::super::*;
        use super::{route, route_with, table};

        /// The journal every in-process `handle` is given. Nothing listens for
        /// it here, so a record falls back to stderr, which is where a test's
        /// output goes.
        fn journal() -> rivet_sdk::journal::Journal {
            rivet_sdk::journal::Journal::for_service("rivet-proxyd-test")
        }
        // `AsyncReadExt` and `AsyncWriteExt` come in with `super::super::*`.

        /// The machine's one name, as `upstream-name` would carry it.
        const MACHINE: &str = "rivet-01.local";

        /// A directory of its own, removed when the test ends.
        ///
        /// **The suite mints its own material** — `AGENTS.md` §8.3 — so nothing
        /// here is committed and nothing reads a real key.
        struct Scratch(std::path::PathBuf);

        impl Scratch {
            fn new(what: &str) -> Self {
                static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let path = std::env::temp_dir()
                    .join(format!("rivet-proxyd-{}-{what}-{n}", std::process::id()));
                std::fs::create_dir_all(&path).expect("a scratch directory");
                Self(path)
            }
            fn write(&self, name: &str, text: &str) {
                std::fs::write(self.0.join(name), text).expect("write");
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        /// An authority and its key, kept so that leaves can be signed BY it.
        ///
        /// **Signed leaves rather than self-signed ones, and that is the point
        /// of this module.** A `WebPkiClientVerifier` built over a root only
        /// admits a chain that verifies to it; a suite that minted self-signed
        /// leaves would prove that the gate refuses everybody, which is not the
        /// claim.
        struct Ca {
            /// The root's own PEM, which is what goes into `tls-root`.
            pem: String,
            /// The same root as something `rcgen` will sign leaves with.
            issuer: rcgen::Issuer<'static, rcgen::KeyPair>,
        }

        fn authority(name: &str) -> Ca {
            let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("a pair");
            let mut params =
                rcgen::CertificateParams::new(vec![name.to_owned()]).expect("params");
            params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
            let pem = params.self_signed(&key).expect("a root").pem();
            Ca {
                pem,
                issuer: rcgen::Issuer::new(params, key),
            }
        }

        /// A leaf for `name`, signed by `ca`, as `(certificate PEM, key PEM)`.
        fn leaf(ca: &Ca, name: &str) -> (String, String) {
            let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("a pair");
            let mut params =
                rcgen::CertificateParams::new(vec![name.to_owned()]).expect("params");
            params.is_ca = rcgen::IsCa::NoCa;
            let cert = params.signed_by(&key, &ca.issuer).expect("a signed leaf");
            (cert.pem(), key.serialize_pem())
        }

        /// **With no setting, the proxy verifies its backends as the first DNS
        /// name on its own certificate** — `ADR-060` §3.
        #[test]
        fn with_no_setting_the_upstream_name_is_the_first_name_on_the_certificate() {
            let ca = authority("fleet authority");
            let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("a pair");
            let mut params = rcgen::CertificateParams::new(vec![
                MACHINE.to_owned(),
                "rivet-01.my-fleet.home.arpa".to_owned(),
            ])
            .expect("params");
            params.is_ca = rcgen::IsCa::NoCa;
            let cert = params.signed_by(&key, &ca.issuer).expect("a signed leaf").pem();

            let (name, from) = upstream_name(None, cert.as_bytes()).expect("a name");
            assert_eq!(name, MACHINE);
            assert!(from.contains("certificate"), "{from}");
        }

        /// A setting still wins over the certificate.
        #[test]
        fn a_setting_names_the_upstream_whatever_the_certificate_says() {
            let ca = authority("fleet authority");
            let (cert, _) = leaf(&ca, MACHINE);
            let (name, from) =
                upstream_name(Some("backends.example".to_owned()), cert.as_bytes()).expect("a name");
            assert_eq!(name, "backends.example");
            assert!(from.contains("setting"), "{from}");
        }

        /// No setting and no DNS name is refused, naming both ways out.
        #[test]
        fn no_setting_and_no_dns_name_is_refused_naming_both() {
            let ca = authority("fleet authority");
            let (cert, _) = operator_leaf(&ca);
            let why = upstream_name(None, cert.as_bytes()).expect_err("nothing to verify against");
            assert!(why.contains(UPSTREAM_NAME), "{why}");
            assert!(why.contains(TLS_CERT), "{why}");
        }

        /// An operator's leaf signed by `ca`: **no `subjectAltName`**, a
        /// person's name as the subject — the shape `Authority::operator`
        /// mints, and the only one the gate serves as the operator
        /// (`.rulings/` R-187 D1). A leaf from [`leaf`] names a host and is a
        /// machine's.
        fn operator_leaf(ca: &Ca) -> (String, String) {
            let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("a pair");
            let mut params = rcgen::CertificateParams::new(Vec::new()).expect("params");
            params.distinguished_name = {
                let mut dn = rcgen::DistinguishedName::new();
                dn.push(rcgen::DnType::CommonName, "operator");
                dn
            };
            params.is_ca = rcgen::IsCa::NoCa;
            let cert = params.signed_by(&key, &ca.issuer).expect("a signed leaf");
            (cert.pem(), key.serialize_pem())
        }

        /// The proxy's own data directory: the machine's certificate, its key
        /// and the fleet authority, exactly as `apply install-secret` puts them
        /// there.
        fn proxy_directory(ca: &Ca, what: &str) -> Scratch {
            let scratch = Scratch::new(what);
            let (cert, key) = leaf(ca, MACHINE);
            scratch.write(TLS_CERT, &cert);
            scratch.write(TLS_KEY, &key);
            scratch.write(TLS_ROOT, &ca.pem);
            scratch
        }

        /// A client, with a certificate from `signer` if one is given.
        fn caller(trusts: &Ca, signer: Option<&Ca>) -> rustls::ClientConfig {
            let mut roots = rustls::RootCertStore::empty();
            for der in rustls_pemfile::certs(&mut trusts.pem.as_bytes()) {
                roots.add(der.expect("a certificate")).expect("a root");
            }
            let builder = rustls::ClientConfig::builder_with_provider(
                rivet_sdk::serve::provider(),
            )
            .with_safe_default_protocol_versions()
            .expect("versions")
            .with_root_certificates(roots);
            match signer {
                None => builder.with_no_client_auth(),
                Some(ca) => {
                    let (cert, key) = operator_leaf(ca);
                    let chain: Vec<_> = rustls_pemfile::certs(&mut cert.as_bytes())
                        .collect::<Result<_, _>>()
                        .expect("a chain");
                    let key = rustls_pemfile::private_key(&mut key.as_bytes())
                        .expect("readable")
                        .expect("a key");
                    builder.with_client_auth_cert(chain, key).expect("a client")
                }
            }
        }

        /// **A backend that verifies the proxy** — `ADR-046` §4.2, built the
        /// way that section tells an application to build it: one call to the
        /// SDK, `requiring` rather than `asking_for`.
        ///
        /// Returns the port it is listening on. It answers one request and
        /// says which path reached it, so a test can tell a forwarded request
        /// from a refused one.
        async fn backend(ca: &Ca) -> u16 {
            backend_counting(ca).await.0
        }

        /// [`backend`], and **how many connections it has accepted** — which is
        /// how a test proves a request never reached an application at all,
        /// rather than that it came back refused.
        async fn backend_counting(ca: &Ca) -> (u16, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
            let reached = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let counter = std::sync::Arc::clone(&reached);
            let scratch = Scratch::new("backend");
            let (cert, key) = leaf(ca, MACHINE);
            scratch.write("tls-cert", &cert);
            scratch.write("tls-key", &key);
            scratch.write("tls-root", &ca.pem);
            let config = rivet_sdk::serve::Tls::in_directory(&scratch.0, "tls-cert", "tls-key")
                .requiring_a_caller_certificate_from("tls-root")
                .server_config()
                .expect("a backend that requires its caller to be the proxy");

            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .expect("a port");
            let port = listener.local_addr().expect("an address").port();
            let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config));
            tokio::spawn(async move {
                // Held for the life of the task so the files outlive the
                // handshake.
                let _scratch = scratch;
                while let Ok((stream, _)) = listener.accept().await {
                    counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let acceptor = acceptor.clone();
                    tokio::spawn(async move {
                        let Ok(mut tls) = acceptor.accept(stream).await else {
                            return;
                        };
                        // The whole head, and then the whole body its
                        // `Content-Length` declared, before answering: a
                        // server that closed with a body unread would reset
                        // the connection under its own answer.
                        let mut seen = Vec::new();
                        let mut buf = [0u8; 2048];
                        let head_end = loop {
                            if let Some(at) = seen.windows(4).position(|w| w == b"\r\n\r\n") {
                                break at + 4;
                            }
                            let read = tls.read(&mut buf).await.unwrap_or(0);
                            if read == 0 {
                                break seen.len();
                            }
                            seen.extend_from_slice(&buf[..read]);
                        };
                        let head = String::from_utf8_lossy(&seen[..head_end]).to_string();
                        let length: usize = rivet_core::cross_site::header(&head, "content-length")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0);
                        while seen.len() < head_end + length {
                            let read = tls.read(&mut buf).await.unwrap_or(0);
                            if read == 0 {
                                break;
                            }
                            seen.extend_from_slice(&buf[..read]);
                        }
                        let target = head
                            .lines()
                            .next()
                            .and_then(|l| l.split(' ').nth(1))
                            .unwrap_or("?")
                            .to_string();
                        let body = format!("the backend was reached at {target}");
                        let answer = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = tls.write_all(answer.as_bytes()).await;
                        let _ = tls.shutdown().await;
                    });
                }
            });
            (port, reached)
        }

        /// [`backend`], answering with the request HEAD it received, verbatim
        /// — so a test can assert a header arrived byte-for-byte rather than
        /// only that some request reached the backend.
        async fn backend_echoing_head(ca: &Ca) -> u16 {
            let scratch = Scratch::new("echo-head");
            let (cert, key) = leaf(ca, MACHINE);
            scratch.write("tls-cert", &cert);
            scratch.write("tls-key", &key);
            scratch.write("tls-root", &ca.pem);
            let config = rivet_sdk::serve::Tls::in_directory(&scratch.0, "tls-cert", "tls-key")
                .requiring_a_caller_certificate_from("tls-root")
                .server_config()
                .expect("a backend that requires its caller to be the proxy");

            let listener =
                tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.expect("a port");
            let port = listener.local_addr().expect("an address").port();
            let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config));
            tokio::spawn(async move {
                let _scratch = scratch;
                while let Ok((stream, _)) = listener.accept().await {
                    let acceptor = acceptor.clone();
                    tokio::spawn(async move {
                        let Ok(mut tls) = acceptor.accept(stream).await else {
                            return;
                        };
                        let mut seen = Vec::new();
                        let mut buf = [0u8; 4096];
                        let head_end = loop {
                            if let Some(at) = seen.windows(4).position(|w| w == b"\r\n\r\n") {
                                break at + 4;
                            }
                            let read = tls.read(&mut buf).await.unwrap_or(0);
                            if read == 0 {
                                break seen.len();
                            }
                            seen.extend_from_slice(&buf[..read]);
                        };
                        let head = seen[..head_end].to_vec();
                        let answer_head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            head.len()
                        );
                        let _ = tls.write_all(answer_head.as_bytes()).await;
                        let _ = tls.write_all(&head).await;
                        let _ = tls.shutdown().await;
                    });
                }
            });
            port
        }

        /// Run one request through the real `handle`, and give back whatever
        /// the caller received — or the handshake's own error.
        async fn request(
            fleet: &Ca,
            routes: Vec<rivet_core::proxy::Route>,
            client: rustls::ClientConfig,
            target: &str,
        ) -> Result<String, String> {
            let head = format!("GET {target} HTTP/1.1\r\nHost: {MACHINE}\r\n\r\n");
            exchange(fleet, routes, client, &head, b"").await
        }

        /// Send `head`, then `body` as a SEPARATE write, through the real
        /// `handle`, and give back whatever the caller received.
        ///
        /// Two writes because a browser's upload is not one TLS record with
        /// its head, and the refusal must be readable when the body is still
        /// arriving.
        async fn exchange(
            fleet: &Ca,
            routes: Vec<rivet_core::proxy::Route>,
            client: rustls::ClientConfig,
            head: &str,
            body: &[u8],
        ) -> Result<String, String> {
            let scratch = proxy_directory(fleet, "gate");
            // The files must outlive the handshake, which is what this holds
            // them for.
            exchange_in(&scratch, routes, client, head, body).await
        }

        /// A relying party with nothing enrolled and nobody signed in, in a
        /// directory of its own that nothing is written to.
        fn no_passkeys() -> passkey::Passkeys {
            passkey::Passkeys::new(
                &std::env::temp_dir().join("rivet-proxyd-no-passkeys"),
                &[MACHINE.to_string()],
                MACHINE,
                journal(),
            )
        }

        /// [`exchange`], through a proxy whose data directory is `scratch`.
        async fn exchange_in(
            scratch: &Scratch,
            routes: Vec<rivet_core::proxy::Route>,
            client: rustls::ClientConfig,
            head: &str,
            body: &[u8],
        ) -> Result<String, String> {
            exchange_with(scratch, routes, client, head, body, no_passkeys()).await
        }

        /// [`exchange_in`], with `passkeys` as the proxy's relying party.
        async fn exchange_with(
            scratch: &Scratch,
            routes: Vec<rivet_core::proxy::Route>,
            client: rustls::ClientConfig,
            head: &str,
            body: &[u8],
            passkeys: passkey::Passkeys,
        ) -> Result<String, String> {
            exchange_as(
                scratch,
                routes,
                client,
                head,
                body,
                passkeys,
                rivet_core::proxy::OperatorCertificate::Accepted,
            )
            .await
        }

        /// [`exchange_with`], through a proxy that accepts or refuses the
        /// operator's certificate as `certificate` says.
        async fn exchange_as(
            scratch: &Scratch,
            routes: Vec<rivet_core::proxy::Route>,
            client: rustls::ClientConfig,
            head: &str,
            body: &[u8],
            passkeys: passkey::Passkeys,
            certificate: rivet_core::proxy::OperatorCertificate,
        ) -> Result<String, String> {
            let server =
                server_config(&scratch.0, certificate).expect("the proxy's server configuration");
            let upstream = client_config(&scratch.0, MACHINE)
                .expect("the proxy's client configuration");

            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .expect("a port");
            let address = listener.local_addr().expect("an address");
            let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(server));
            let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(upstream));
            let served = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.expect("a connection");
                let _ = handle(
                    stream, &acceptor, &connector, &routes, None, MACHINE, None, &passkeys,
                    &journal(), PATIENCE, certificate,
                )
                .await;
            });

            let outcome = async {
                let stream = tokio::net::TcpStream::connect(address)
                    .await
                    .map_err(|e| format!("connect: {e}"))?;
                let name = rustls_pki_types::ServerName::try_from(MACHINE).expect("a name");
                let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(client));
                let mut tls = connector
                    .connect(name.to_owned(), stream)
                    .await
                    .map_err(|e| format!("handshake: {e}"))?;
                tls.write_all(head.as_bytes())
                    .await
                    .map_err(|e| format!("write: {e}"))?;
                tls.flush().await.map_err(|e| format!("flush: {e}"))?;
                if !body.is_empty() {
                    // A refused request's body may meet a closed connection;
                    // the answer already sent is what the test reads.
                    let _ = tls.write_all(body).await;
                    let _ = tls.flush().await;
                }
                let mut answer = Vec::new();
                tls.read_to_end(&mut answer)
                    .await
                    .map_err(|e| format!("read: {e}"))?;
                Ok(String::from_utf8_lossy(&answer).to_string())
            }
            .await;

            let _ = served.await;
            outcome
        }

        /// **Caller one of three: no certificate at all.** The gate's whole
        /// reason to exist, and the thing that had never been watched happen.
        #[tokio::test]
        async fn a_caller_presenting_no_certificate_is_refused_on_a_gated_route() {
            let fleet = authority("rivet-fleet-ca");
            let answer = request(&fleet, table(), caller(&fleet, None), "/notes/api")
                .await
                .expect("the connection itself succeeds; the REQUEST is refused");
            assert!(answer.starts_with("HTTP/1.1 403 Forbidden"), "{answer}");
            assert!(answer.contains("operator certificate"), "{answer}");
        }

        /// **Caller two of three: a certificate from another authority.** It
        /// does not reach a 403 — it does not get a TLS session at all, which
        /// is `WebPkiClientVerifier` doing the verifying rather than this
        /// program.
        #[tokio::test]
        async fn a_caller_from_another_authority_never_completes_a_handshake() {
            let fleet = authority("rivet-fleet-ca");
            let elsewhere = authority("somebody-elses-ca");
            let why = request(
                &fleet,
                table(),
                caller(&fleet, Some(&elsewhere)),
                "/notes/api",
            )
            .await
            .expect_err("a chain to another authority must not be admitted");
            assert!(why.contains("handshake") || why.contains("read"), "{why}");
        }

        /// **Caller three of three: a fleet certificate.** It reaches the
        /// application — and the application is one that REQUIRED the proxy to
        /// present a certificate of its own, so this is `ADR-046` §4.2 proved
        /// in the same request.
        #[tokio::test]
        async fn a_caller_with_a_fleet_certificate_reaches_a_backend_that_verified_the_proxy() {
            let fleet = authority("rivet-fleet-ca");
            let port = backend(&fleet).await;
            let routes = vec![route("notes", port, true)];
            let answer = request(
                &fleet,
                routes,
                caller(&fleet, Some(&fleet)),
                "/notes/api/v1/notes",
            )
            .await
            .expect("the request is forwarded");
            assert!(answer.starts_with("HTTP/1.1 200 OK"), "{answer}");
            assert!(
                answer.contains("the backend was reached at /api/v1/notes"),
                "the prefix must be stripped and the backend must have accepted the proxy: \
                 {answer}"
            );
        }

        /// **`ADR-046` §4.3 over the wire.** No certificate, and the request
        /// still reaches TempMon — which is `R-152`'s one worked example and
        /// the thing the gate must not break.
        #[tokio::test]
        async fn a_public_route_reaches_its_backend_with_no_certificate_at_all() {
            let fleet = authority("rivet-fleet-ca");
            let port = backend(&fleet).await;
            let routes = vec![route("tempmon", port, false)];
            let answer = request(
                &fleet,
                routes,
                caller(&fleet, None),
                "/tempmon/api/v1/sensors",
            )
            .await
            .expect("a public route is served");
            assert!(answer.starts_with("HTTP/1.1 200 OK"), "{answer}");
            assert!(answer.contains("/api/v1/sensors"), "{answer}");
        }

        // --- passkeys beside client certificates — `ADR-063` §4 ----------

        /// **A passkey session reaches a gated route with no certificate at
        /// all**, and the backend — which requires the PROXY's certificate —
        /// never sees the session cookie.
        #[tokio::test]
        async fn a_passkey_session_reaches_a_gated_route_and_its_cookie_stays_here() {
            let fleet = authority("rivet-fleet-ca");
            let port = backend_echoing_head(&fleet).await;
            let scratch = proxy_directory(&fleet, "session");
            let passkeys = no_passkeys();
            let session = passkeys.session_for_test(MACHINE);
            let head = format!(
                "GET /notes/api HTTP/1.1\r\nHost: {MACHINE}\r\nCookie: a=1; {session}\r\n\r\n"
            );
            let answer = exchange_with(
                &scratch,
                vec![route("notes", port, true)],
                caller(&fleet, None),
                &head,
                b"",
                passkeys,
            )
            .await
            .expect("forwarded");
            assert!(answer.starts_with("HTTP/1.1 200 OK"), "{answer}");
            assert!(answer.contains("GET /api HTTP/1.1"), "{answer}");
            assert!(answer.contains("Cookie: a=1"), "{answer}");
            assert!(!answer.contains("rivet-session"), "the session reached a backend: {answer}");
        }

        // --- the certificate retired by the machine's document — R-234 ------

        /// **A machine that refuses the operator's certificate refuses a
        /// caller holding it, and says why** — `ADR-046` §4, `.rulings/`
        /// R-234. The same caller and route are admitted when the certificate
        /// is accepted, which is the control that the refusal is the setting's.
        #[tokio::test]
        async fn a_retired_certificate_is_refused_with_its_reason_and_reaches_nothing() {
            let fleet = authority("rivet-fleet-ca");
            let (port, reached) = backend_counting(&fleet).await;
            let head = format!("GET /notes/api HTTP/1.1\r\nHost: {MACHINE}\r\n\r\n");

            let accepted = exchange_as(
                &proxy_directory(&fleet, "accepted"),
                vec![route("notes", port, true)],
                caller(&fleet, Some(&fleet)),
                &head,
                b"",
                no_passkeys(),
                rivet_core::proxy::OperatorCertificate::Accepted,
            )
            .await
            .expect("forwarded");
            assert!(accepted.starts_with("HTTP/1.1 200 OK"), "{accepted}");
            let before = reached.load(std::sync::atomic::Ordering::SeqCst);

            let refused = exchange_as(
                &proxy_directory(&fleet, "retired"),
                vec![route("notes", port, true)],
                caller(&fleet, Some(&fleet)),
                &head,
                b"",
                no_passkeys(),
                rivet_core::proxy::OperatorCertificate::Refused,
            )
            .await
            .expect("the connection succeeds; the REQUEST is refused");
            assert!(refused.starts_with("HTTP/1.1 403 Forbidden"), "{refused}");
            assert!(refused.contains("no longer accepts the operator's client certificate"), "{refused}");
            assert!(refused.contains("passkey at /sign-in"), "{refused}");
            assert!(!refused.contains("/.rivet/"), "{refused}");
            assert_eq!(
                reached.load(std::sync::atomic::Ordering::SeqCst),
                before,
                "a retired certificate must reach no application"
            );
        }

        /// **A passkey session still admits the operator where the certificate
        /// is retired**, and a public route still serves anybody.
        #[tokio::test]
        async fn a_passkey_session_still_works_where_the_certificate_is_retired() {
            let fleet = authority("rivet-fleet-ca");
            let port = backend_echoing_head(&fleet).await;
            let passkeys = no_passkeys();
            let session = passkeys.session_for_test(MACHINE);
            let head = format!("GET /notes/api HTTP/1.1\r\nHost: {MACHINE}\r\nCookie: {session}\r\n\r\n");
            let answer = exchange_as(
                &proxy_directory(&fleet, "retired-session"),
                vec![route("notes", port, true)],
                caller(&fleet, None),
                &head,
                b"",
                passkeys,
                rivet_core::proxy::OperatorCertificate::Refused,
            )
            .await
            .expect("forwarded");
            assert!(answer.starts_with("HTTP/1.1 200 OK"), "{answer}");

            let public = exchange_as(
                &proxy_directory(&fleet, "retired-public"),
                vec![route("tempmon", port, false)],
                caller(&fleet, None),
                &format!("GET /tempmon/api HTTP/1.1\r\nHost: {MACHINE}\r\n\r\n"),
                b"",
                no_passkeys(),
                rivet_core::proxy::OperatorCertificate::Refused,
            )
            .await
            .expect("served");
            assert!(public.starts_with("HTTP/1.1 200 OK"), "{public}");
        }

        /// A cookie this proxy never issued is no session: `403`, as with
        /// nothing at all.
        #[tokio::test]
        async fn a_session_cookie_this_proxy_did_not_issue_is_refused() {
            let fleet = authority("rivet-fleet-ca");
            let (port, reached) = backend_counting(&fleet).await;
            let scratch = proxy_directory(&fleet, "forged");
            let head = format!(
                "GET /notes/api HTTP/1.1\r\nHost: {MACHINE}\r\nCookie: rivet-session={}\r\n\r\n",
                "ab".repeat(32)
            );
            let answer = exchange_with(
                &scratch,
                vec![route("notes", port, true)],
                caller(&fleet, None),
                &head,
                b"",
                no_passkeys(),
            )
            .await
            .expect("answered");
            assert!(answer.starts_with("HTTP/1.1 403 Forbidden"), "{answer}");
            assert!(answer.contains("passkey at /sign-in"), "{answer}");
            assert!(!answer.contains("/.rivet/"), "{answer}");
            assert_eq!(reached.load(std::sync::atomic::Ordering::SeqCst), 0);
        }

        /// A browser with neither is sent to the sign-in page, carrying where
        /// it was going.
        #[tokio::test]
        async fn a_browser_with_neither_is_sent_to_the_sign_in_page() {
            let fleet = authority("rivet-fleet-ca");
            let scratch = proxy_directory(&fleet, "browser");
            let head = format!(
                "GET /notes/api?x=1 HTTP/1.1\r\nHost: {MACHINE}\r\nAccept: text/html\r\n\r\n"
            );
            let answer = exchange_with(
                &scratch,
                vec![route("notes", 1, true)],
                caller(&fleet, None),
                &head,
                b"",
                no_passkeys(),
            )
            .await
            .expect("answered");
            assert!(answer.starts_with("HTTP/1.1 303 See Other"), "{answer}");
            assert!(
                answer.contains("Location: /sign-in?next=/notes/api%3Fx%3D1\r\n"),
                "{answer}"
            );
        }

        /// The sign-in page is served over TLS to a caller with no certificate,
        /// and links to the enrollment page at its new address.
        #[tokio::test]
        async fn the_sign_in_page_is_served_to_a_caller_with_nothing() {
            let fleet = authority("rivet-fleet-ca");
            for target in ["/sign-in", "/sign-in/", "/sign-in?next=/notes/"] {
                let answer = request(&fleet, table(), caller(&fleet, None), target)
                    .await
                    .expect("served");
                assert!(answer.starts_with("HTTP/1.1 200 OK"), "{target}: {answer}");
                assert!(answer.contains("Content-Type: text/html"), "{target}: {answer}");
                assert!(answer.contains("navigator.credentials.get"), "{target}: {answer}");
                assert!(answer.contains("href=\"/sign-in/enroll\""), "{target}: {answer}");
            }
        }

        /// The enrollment page is served over TLS to a caller with no
        /// certificate, and links back to the sign-in page.
        #[tokio::test]
        async fn the_enroll_page_is_served_to_a_caller_with_nothing() {
            let fleet = authority("rivet-fleet-ca");
            let answer = request(&fleet, table(), caller(&fleet, None), "/sign-in/enroll")
                .await
                .expect("served");
            assert!(answer.starts_with("HTTP/1.1 200 OK"), "{answer}");
            assert!(answer.contains("Content-Type: text/html"), "{answer}");
            assert!(answer.contains("navigator.credentials.create"), "{answer}");
            assert!(answer.contains("href=\"/sign-in\""), "{answer}");
            assert!(answer.contains("/.rivet/passkey/enroll/options"), "{answer}");
        }

        /// Nothing else is served under `/sign-in/`.
        #[tokio::test]
        async fn nothing_else_is_served_under_sign_in() {
            let fleet = authority("rivet-fleet-ca");
            let answer = request(&fleet, table(), caller(&fleet, None), "/sign-in/elsewhere")
                .await
                .expect("answered");
            assert!(answer.starts_with("HTTP/1.1 404"), "{answer}");
        }

        /// **The old addresses answer `308 Permanent Redirect` to the new
        /// ones, the query string kept** — `.rulings/` R-249.
        #[tokio::test]
        async fn the_old_addresses_redirect_permanently_keeping_the_query() {
            let fleet = authority("rivet-fleet-ca");
            for (old, location) in [
                ("/.rivet/login", "/sign-in"),
                ("/.rivet/login?next=/notes/api%3Fx%3D1", "/sign-in?next=/notes/api%3Fx%3D1"),
                ("/.rivet/enrol", "/sign-in/enroll"),
                ("/.rivet/enrol?a=1&b=2", "/sign-in/enroll?a=1&b=2"),
            ] {
                let answer = request(&fleet, table(), caller(&fleet, None), old)
                    .await
                    .expect("answered");
                assert!(answer.starts_with("HTTP/1.1 308 Permanent Redirect"), "{old}: {answer}");
                assert!(answer.contains(&format!("Location: {location}\r\n")), "{old}: {answer}");
            }
        }

        /// A ceremony's body is read by its length and answered as JSON.
        #[tokio::test]
        async fn a_ceremony_is_posted_and_answered_over_tls() {
            let fleet = authority("rivet-fleet-ca");
            let body = br#"{"code":"ZZZZZ-ZZZZZ"}"#;
            let head = format!(
                "POST /.rivet/passkey/enroll/options HTTP/1.1\r\nHost: {MACHINE}\r\n\
                 Content-Type: application/json\r\nContent-Length: {}\r\n\
                 Sec-Fetch-Site: same-origin\r\n\r\n",
                body.len()
            );
            let answer = exchange(&fleet, table(), caller(&fleet, None), &head, body)
                .await
                .expect("answered");
            assert!(answer.starts_with("HTTP/1.1 403 Forbidden"), "{answer}");
            assert!(answer.contains("application/json"), "{answer}");
            assert!(answer.contains("no enrollment code is waiting"), "{answer}");
        }

        // --- port 80, plain HTTP — `ADR-061` §7, `ADR-046` §7 -------------

        /// [`request`]'s shape with no client TLS at all — port 80 has none —
        /// through the real [`handle_plain`].
        async fn request_plain(
            fleet: &Ca,
            routes: Vec<rivet_core::proxy::Route>,
            method: &str,
            target: &str,
        ) -> Result<String, String> {
            let scratch = proxy_directory(fleet, "plain");
            let upstream =
                client_config(&scratch.0, MACHINE).expect("the proxy's client configuration");
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .expect("a port");
            let address = listener.local_addr().expect("an address");
            let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(upstream));
            let served = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.expect("a connection");
                let _ =
                    handle_plain(stream, &connector, &routes, MACHINE, &journal(), PATIENCE.head)
                        .await;
            });

            let outcome = async {
                let mut stream = tokio::net::TcpStream::connect(address)
                    .await
                    .map_err(|e| format!("connect: {e}"))?;
                let head = format!("{method} {target} HTTP/1.1\r\nHost: {MACHINE}\r\n\r\n");
                stream
                    .write_all(head.as_bytes())
                    .await
                    .map_err(|e| format!("write: {e}"))?;
                stream.flush().await.map_err(|e| format!("flush: {e}"))?;
                let mut answer = Vec::new();
                stream
                    .read_to_end(&mut answer)
                    .await
                    .map_err(|e| format!("read: {e}"))?;
                Ok(String::from_utf8_lossy(&answer).to_string())
            }
            .await;

            let _ = served.await;
            outcome
        }

        /// **A backend whose certificate another authority issued is answered
        /// `502`, with the cause**, on 443 and on port 80 alike — rather than
        /// a closed connection, which a browser reports as an empty response
        /// and names nothing. The backend is never handed the request.
        #[tokio::test]
        async fn a_backend_whose_certificate_another_authority_issued_is_answered_502() {
            let fleet = authority("rivet-fleet-ca");
            // Another authority under the same subject name (`authority`
            // names none, so every one shares rcgen's default): the verifier
            // finds a root by name and refuses the signature.
            a_backend_from(&fleet, &authority("a-workstation-ca")).await;
            // And under a name of its own, as a workstation's authority is
            // beside a fleet-root machine's: no root by that name at all.
            let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("a pair");
            let mut params = rcgen::CertificateParams::new(Vec::new()).expect("params");
            params.distinguished_name = {
                let mut dn = rcgen::DistinguishedName::new();
                dn.push(rcgen::DnType::CommonName, "a-workstation-ca");
                dn
            };
            params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
            let pem = params.self_signed(&key).expect("a root").pem();
            let named = Ca { pem, issuer: rcgen::Issuer::new(params, key) };
            a_backend_from(&fleet, &named).await;
        }

        /// [`a_backend_whose_certificate_another_authority_issued_is_answered_502`]
        /// for one authority the backend's certificate came from.
        async fn a_backend_from(fleet: &Ca, elsewhere: &Ca) {
            let port = backend(elsewhere).await;

            let routes = vec![route("aura", port, false)];
            let answer = request(fleet, routes, caller(fleet, None), "/aura/")
                .await
                .expect("the caller's own connection succeeds; the BACKEND's handshake fails");
            assert!(answer.starts_with("HTTP/1.1 502 Bad Gateway"), "{answer}");
            assert!(answer.contains("aura"), "{answer}");
            assert!(answer.contains("not issued by this fleet"), "{answer}");
            assert!(!answer.contains("the backend was reached"), "{answer}");

            let routes = vec![route_with("aura", port, true, &["plain:install"])];
            let plain = request_plain(fleet, routes, "GET", "/aura/install/x")
                .await
                .expect("the caller's own connection succeeds");
            assert!(plain.starts_with("HTTP/1.1 502 Bad Gateway"), "{plain}");
            assert!(plain.contains("not issued by this fleet"), "{plain}");
        }

        /// **A backend that refuses the connection — its application is not
        /// running — is answered `502` saying so**, on 443 and on port 80
        /// alike, rather than a closed connection that a browser reports as an
        /// empty response.
        #[tokio::test]
        async fn a_backend_that_refuses_the_connection_is_answered_502() {
            let fleet = authority("rivet-fleet-ca");
            // A port that was bound and released: nothing listens there, so
            // the dial is refused exactly as a stopped application's is.
            let port = {
                let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a port");
                listener.local_addr().expect("an address").port()
            };

            let routes = vec![route("aura", port, false)];
            let answer = request(&fleet, routes, caller(&fleet, None), "/aura/")
                .await
                .expect("the caller's own connection succeeds; the BACKEND refuses");
            assert!(answer.starts_with("HTTP/1.1 502 Bad Gateway"), "{answer}");
            assert!(answer.contains("is not answering"), "{answer}");

            let routes = vec![route_with("aura", port, true, &["plain:install"])];
            let plain = request_plain(&fleet, routes, "GET", "/aura/install/x")
                .await
                .expect("the caller's own connection succeeds");
            assert!(plain.starts_with("HTTP/1.1 502 Bad Gateway"), "{plain}");
            assert!(plain.contains("is not answering"), "{plain}");
        }

        /// **A backend's failed handshake is journalled once per route per
        /// window**, carrying the count since the last line — a browser
        /// retrying a broken application must not fill the journal.
        #[test]
        fn a_failing_backend_is_journalled_at_most_once_per_window() {
            let mut failures = rivet_core::journal::TokenBuckets::new(
                1,
                1,
                rivet_core::connection_slots::TRACKED_SOURCES,
                rivet_core::connection_slots::REPORT_EVERY_MS,
            );
            assert_eq!(note_upstream_failure(&mut failures, 8090, 1_000), Some(1));
            for at in [1_001, 2_000, 30_000, 60_999] {
                assert_eq!(note_upstream_failure(&mut failures, 8090, at), None, "{at}");
            }
            // Another application's failure is its own line.
            assert_eq!(note_upstream_failure(&mut failures, 8091, 2_000), Some(1));
            assert_eq!(
                note_upstream_failure(&mut failures, 8090, 1_000 + rivet_core::connection_slots::REPORT_EVERY_MS),
                Some(5)
            );
        }

        /// **A `plain:` route is served with no TLS at all, GET and HEAD both**
        /// — `ADR-061` §7. The backend still requires the proxy's OWN
        /// certificate ([`backend`]'s shape): a plain caller reaches it only
        /// because the proxy dials it exactly as it dials a `none:` route on
        /// 443, never because the backend relaxed anything.
        #[tokio::test]
        async fn a_plain_route_is_served_on_port_80_with_no_tls_get_and_head() {
            let fleet = authority("rivet-fleet-ca");
            let port = backend(&fleet).await;
            let routes = vec![route_with("gantry", port, true, &["plain:install"])];

            let get = request_plain(&fleet, routes.clone(), "GET", "/gantry/install/apk")
                .await
                .expect("a plain route is served over plain HTTP");
            assert!(get.starts_with("HTTP/1.1 200 OK"), "{get}");
            assert!(get.contains("the backend was reached at /install/apk"), "{get}");

            let head = request_plain(&fleet, routes, "HEAD", "/gantry/install/apk")
                .await
                .expect("HEAD is served exactly like GET");
            assert!(head.starts_with("HTTP/1.1 200 OK"), "{head}");
        }

        /// **Everything that is not a `plain:` route is `404`, the index
        /// included** — `ADR-061` §7. Port 80 must never read as a second
        /// front door: a gated root, a root with no rule at all, and `/`
        /// itself all answer identically to a path nothing serves.
        #[tokio::test]
        async fn a_non_plain_path_and_the_index_are_404_on_port_80() {
            let fleet = authority("rivet-fleet-ca");
            let routes = vec![route_with("gantry", 1, true, &["plain:install"])];

            for (method, target) in [
                ("GET", "/gantry/admin"),
                ("GET", "/gantry"),
                ("GET", "/"),
                ("GET", "/nothing/here"),
            ] {
                let answer = request_plain(&fleet, routes.clone(), method, target)
                    .await
                    .expect("the connection itself succeeds; the request is refused");
                assert!(
                    answer.starts_with("HTTP/1.1 404 Not Found"),
                    "{method} {target}: {answer}"
                );
            }
        }

        /// **Only `GET` and `HEAD` are served on port 80** — `ADR-061` §7. A
        /// write over plain HTTP is refused `405` rather than forwarded: a
        /// request that changes something is made over 443, after pairing.
        #[tokio::test]
        async fn a_write_to_a_plain_route_on_port_80_is_refused_405() {
            let fleet = authority("rivet-fleet-ca");
            let routes = vec![route_with("gantry", 1, true, &["plain:install"])];
            let answer = request_plain(&fleet, routes, "POST", "/gantry/install/apk")
                .await
                .expect("the connection itself succeeds; the request is refused");
            assert!(answer.starts_with("HTTP/1.1 405 Method Not Allowed"), "{answer}");
        }

        /// **The traversal refusal binds port 80 too, before any route is
        /// found** — `ADR-048` §5, `ADR-061` §7.
        #[tokio::test]
        async fn a_traversal_on_port_80_is_refused_400_before_any_route_is_found() {
            let fleet = authority("rivet-fleet-ca");
            let routes = vec![route_with("gantry", 1, true, &["plain:install"])];
            let answer = request_plain(&fleet, routes, "GET", "/gantry/install/../etc")
                .await
                .expect("the connection itself succeeds; the request is refused");
            assert!(answer.starts_with("HTTP/1.1 400 Bad Request"), "{answer}");
        }

        /// **On 443, `app:` forwards with no client certificate at all, and
        /// the `Authorization` header reaches the backend byte-for-byte** —
        /// `ADR-061` §7. Nothing about the header is inspected, stripped or
        /// rewritten: only the hop-by-hop headers `rewrite_head` already drops
        /// are ever touched.
        #[tokio::test]
        async fn an_app_route_forwards_with_no_certificate_and_the_authorization_header_survives() {
            let fleet = authority("rivet-fleet-ca");
            let port = backend_echoing_head(&fleet).await;
            let routes = vec![route_with("gantry", port, true, &["app:api"])];
            let head = format!(
                "GET /gantry/api/phones HTTP/1.1\r\nHost: {MACHINE}\r\n\
                 Authorization: Bearer a-phone-signed-token\r\n\r\n"
            );
            let answer = exchange(&fleet, routes, caller(&fleet, None), &head, b"")
                .await
                .expect("an app: route is forwarded with no certificate");
            assert!(answer.starts_with("HTTP/1.1 200 OK"), "{answer}");
            assert!(
                answer.contains("GET /api/phones HTTP/1.1"),
                "the prefix must be stripped: {answer}"
            );
            assert!(
                answer.contains("Authorization: Bearer a-phone-signed-token"),
                "the Authorization header must reach the backend byte-for-byte: {answer}"
            );
        }

        /// A restore-shaped `POST` with the headers given, with a body.
        fn restore_post(extra_headers: &str, content_type: &str) -> String {
            format!(
                "POST /probe/api/v1/restore?confirm=true HTTP/1.1\r\nHost: {MACHINE}\r\n\
                 {extra_headers}Content-Type: {content_type}\r\nContent-Length: 4096\r\n\r\n"
            )
        }

        /// **`.rulings/` R-186 D1 over the wire: the attack never reaches the
        /// application.** An operator certificate, a gated route with a
        /// backend behind it, and the three ways a page on another site can
        /// make a browser send a write — each answered `403` by the proxy, and
        /// the backend accepted **no connection at all**, so no byte of any
        /// body reached it.
        #[tokio::test]
        async fn a_cross_site_post_never_reaches_the_application() {
            let fleet = authority("rivet-fleet-ca");
            let (port, reached) = backend_counting(&fleet).await;
            let routes = vec![route("probe", port, true)];
            let body = vec![b'x'; 4096];

            for (headers, content_type) in [
                ("Sec-Fetch-Site: cross-site\r\nOrigin: https://evil.example\r\n", "text/plain"),
                ("Sec-Fetch-Site: same-site\r\n", "application/x-www-form-urlencoded"),
                ("Origin: https://evil.example\r\n", "multipart/form-data; boundary=x"),
                ("Origin: null\r\n", "text/plain"),
            ] {
                let answer = exchange(
                    &fleet,
                    routes.clone(),
                    caller(&fleet, Some(&fleet)),
                    &restore_post(headers, content_type),
                    &body,
                )
                .await
                .expect("the connection succeeds; the REQUEST is refused");
                assert!(answer.starts_with("HTTP/1.1 403 Forbidden"), "{headers}: {answer}");
                assert!(answer.contains("another site"), "{headers}: {answer}");
            }
            assert_eq!(
                reached.load(std::sync::atomic::Ordering::SeqCst),
                0,
                "a cross-site write reached the application"
            );
        }

        /// **And the writes that must keep working do reach it**, through the
        /// same proxy, route and backend: `cargo rivet restore` (no browser
        /// header), an application's own page (`same-origin` and its own
        /// `Origin`), and an engine that sends `Origin` alone. A cross-site
        /// `GET` is forwarded too, because only writes are refused.
        #[tokio::test]
        async fn a_same_origin_post_and_a_non_browser_post_reach_the_application() {
            let fleet = authority("rivet-fleet-ca");
            let (port, reached) = backend_counting(&fleet).await;
            let routes = vec![route("probe", port, true)];

            let own = format!("Sec-Fetch-Site: same-origin\r\nOrigin: https://{MACHINE}\r\n");
            let origin_only = format!("Origin: https://{MACHINE}\r\n");
            for headers in ["", own.as_str(), origin_only.as_str()] {
                let answer = exchange(
                    &fleet,
                    routes.clone(),
                    caller(&fleet, Some(&fleet)),
                    &restore_post(headers, "application/vnd.sqlite3"),
                    &[b'x'; 4096],
                )
                .await
                .expect("the request is forwarded");
                assert!(answer.starts_with("HTTP/1.1 200 OK"), "{headers:?}: {answer}");
                assert!(
                    answer.contains("the backend was reached at /api/v1/restore?confirm=true"),
                    "{headers:?}: {answer}"
                );
            }

            let navigation = format!(
                "GET /probe/api/v1/backup HTTP/1.1\r\nHost: {MACHINE}\r\nSec-Fetch-Site: cross-site\r\n\r\n"
            );
            let answer = exchange(&fleet, routes, caller(&fleet, Some(&fleet)), &navigation, b"")
                .await
                .expect("a cross-site GET is forwarded");
            assert!(answer.starts_with("HTTP/1.1 200 OK"), "{answer}");
            assert_eq!(reached.load(std::sync::atomic::Ordering::SeqCst), 4);
        }

        /// **`ADR-048` §4 over the wire, and it is Reliquary's whole case.** One
        /// gated application, one listening socket, and one prefix under it a
        /// phone can reach with no certificate at all — while the record beside
        /// it is `403` to the same client on the same connection settings.
        ///
        /// **The two requests differ only in their path**, so the thing being
        /// proved is the per-path decision rather than two different clients.
        #[tokio::test]
        async fn an_open_prefix_is_served_over_a_real_handshake_and_its_neighbours_are_not() {
            let fleet = authority("rivet-fleet-ca");
            let port = backend(&fleet).await;
            let routes = vec![route_with("reliquary", port, true, &["none:pair"])];

            let paired = request(
                &fleet,
                routes.clone(),
                caller(&fleet, None),
                "/reliquary/pair/redeem",
            )
            .await
            .expect("the open prefix is served");
            assert!(paired.starts_with("HTTP/1.1 200 OK"), "{paired}");
            assert!(paired.contains("/pair/redeem"), "{paired}");

            // The record, the sibling sharing a character prefix, and the
            // sibling with a trailing hyphen — all `403` to the same caller.
            for hostile in ["/reliquary/blobs", "/reliquary/pairings", "/reliquary/pair-admin"] {
                let refused = request(&fleet, routes.clone(), caller(&fleet, None), hostile)
                    .await
                    .expect("the connection succeeds");
                assert!(
                    refused.starts_with("HTTP/1.1 403 Forbidden"),
                    "{hostile} must be refused: {refused}"
                );
            }

            // And the traversal through the open door — `ADR-048` §5 — never
            // reaches a route at all.
            let traversal = request(
                &fleet,
                routes,
                caller(&fleet, None),
                "/reliquary/pair/../blobs",
            )
            .await
            .expect("the connection succeeds");
            assert!(traversal.starts_with("HTTP/1.1 400 Bad Request"), "{traversal}");
        }

        /// **The index, drawn for an authenticated caller and refused to
        /// everybody else** — `ADR-046` §6, over the real handshake.
        #[tokio::test]
        async fn the_index_is_served_only_to_a_caller_with_a_certificate() {
            let fleet = authority("rivet-fleet-ca");
            let refused = request(&fleet, table(), caller(&fleet, None), "/")
                .await
                .expect("the connection succeeds");
            assert!(refused.starts_with("HTTP/1.1 403 Forbidden"), "{refused}");

            let page = request(&fleet, table(), caller(&fleet, Some(&fleet)), "/")
                .await
                .expect("the index is served");
            assert!(page.starts_with("HTTP/1.1 200 OK"), "{page}");
            assert!(page.contains("text/html"), "{page}");
            assert!(page.contains("href=\"/notes/\""), "{page}");
        }

        /// **`ADR-046` §4.2 from the other side.** A proxy that presented no
        /// certificate would be refused by a backend that requires one — which
        /// is what makes `client_config`'s `with_client_auth_cert` the thing
        /// doing the work rather than a line that happens to compile.
        #[tokio::test]
        async fn a_backend_refuses_a_caller_that_is_not_the_proxy() {
            let fleet = authority("rivet-fleet-ca");
            let port = backend(&fleet).await;

            let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .expect("the backend is listening");
            let name = rustls_pki_types::ServerName::try_from(MACHINE).expect("a name");
            let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(caller(
                &fleet, None,
            )));
            let mut tls = connector
                .connect(name.to_owned(), stream)
                .await
                .expect("TLS 1.3 tells the client about a client-auth refusal on the first read");
            let mut buf = [0u8; 64];
            let refused = tls.write_all(b"GET / HTTP/1.1\r\n\r\n").await.is_err()
                || tls.read(&mut buf).await.is_err();
            assert!(
                refused,
                "a backend built with requiring_a_caller_certificate_from served a caller \
                 that presented none"
            );
        }

        /// **A raw connection that never completes the TLS handshake holds no
        /// task and no descriptor past its patience.** Before this, only
        /// the request head that FOLLOWS a handshake had a deadline;
        /// `acceptor.accept` itself would wait for as long as a peer cared to
        /// hold it open. Both shapes of "never finishes" are driven here: no
        /// bytes at all, and a `ClientHello` begun and then abandoned — the
        /// same deadline must catch each, since a real attacker picks
        /// whichever costs it least.
        ///
        /// The patience is a parameter precisely so this test can use one
        /// short enough to run in well under a second rather than the real
        /// fifteen.
        #[tokio::test]
        async fn a_stalled_handshake_is_dropped_after_the_head_patience() {
            let patience = std::time::Duration::from_millis(300);
            let tolerance = std::time::Duration::from_secs(3);

            for payload in [&b""[..], &[0x16, 0x03, 0x01][..]] {
                let fleet = authority("rivet-fleet-ca");
                let scratch = proxy_directory(&fleet, "stall");
                let server = server_config(&scratch.0, rivet_core::proxy::OperatorCertificate::Accepted)
                    .expect("the proxy's server configuration");
                let upstream = client_config(&scratch.0, MACHINE)
                    .expect("the proxy's client configuration");
                let listener =
                    tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.expect("a port");
                let address = listener.local_addr().expect("an address");
                let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(server));
                let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(upstream));
                let routes: Vec<rivet_core::proxy::Route> = Vec::new();

                let served = tokio::spawn(async move {
                    let (stream, _) = listener.accept().await.expect("a connection");
                    let passkeys = no_passkeys();
                    handle(
                        stream, &acceptor, &connector, &routes, None, MACHINE, None, &passkeys,
                        &journal(), Patience::uniform(patience),
                        rivet_core::proxy::OperatorCertificate::Accepted,
                    )
                    .await
                });

                let mut stalling =
                    tokio::net::TcpStream::connect(address).await.expect("connect");
                if !payload.is_empty() {
                    stalling.write_all(payload).await.expect("a few bytes");
                }

                let mut buf = [0u8; 16];
                let read = tokio::time::timeout(patience + tolerance, stalling.read(&mut buf))
                    .await
                    .unwrap_or_else(|_| {
                        panic!(
                            "{payload:?}: the server must close within the head patience plus \
                             tolerance"
                        )
                    });
                assert_eq!(
                    read.unwrap_or_else(|e| {
                        panic!("{payload:?}: a read after close should be Ok(0), not {e}")
                    }),
                    0,
                    "{payload:?}: the server must close rather than answer a caller that never \
                     finished the handshake"
                );

                let outcome = served.await.expect("the handling task must not panic");
                assert!(
                    outcome.is_err(),
                    "{payload:?}: a stalled handshake must not be treated as a served request"
                );
            }
        }

        /// A proxy on `patience` that serves one connection, and its address.
        async fn one_connection(patience: Patience) -> (std::net::SocketAddr, Ca, tokio::task::JoinHandle<Result<(), String>>) {
            let fleet = authority("rivet-fleet-ca");
            let scratch = proxy_directory(&fleet, "patience");
            let server = server_config(&scratch.0, rivet_core::proxy::OperatorCertificate::Accepted)
                .expect("the proxy's server configuration");
            let upstream = client_config(&scratch.0, MACHINE).expect("the proxy's client configuration");
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.expect("a port");
            let address = listener.local_addr().expect("an address");
            let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(server));
            let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(upstream));
            let served = tokio::spawn(async move {
                let _scratch = scratch;
                let routes: Vec<rivet_core::proxy::Route> = Vec::new();
                let (stream, _) = listener.accept().await.expect("a connection");
                let passkeys = no_passkeys();
                handle(
                    stream, &acceptor, &connector, &routes, None, MACHINE, None, &passkeys,
                    &journal(), patience, rivet_core::proxy::OperatorCertificate::Accepted,
                )
                .await
            });
            (address, fleet, served)
        }

        /// **A client that sends nothing is dropped after the first-byte
        /// patience**, however long the handshake is allowed: a browser sends
        /// its `ClientHello` the moment it connects, so silence is not a person
        /// choosing a certificate.
        #[tokio::test]
        async fn a_silent_client_is_dropped_after_the_first_byte_patience() {
            let short = std::time::Duration::from_millis(300);
            let long = std::time::Duration::from_secs(30);
            let (address, _fleet, served) =
                one_connection(Patience { first_byte: short, handshake: long, head: long }).await;
            let mut silent = tokio::net::TcpStream::connect(address).await.expect("connect");
            let mut buf = [0u8; 16];
            let read = tokio::time::timeout(short + std::time::Duration::from_secs(3), silent.read(&mut buf))
                .await
                .expect("a silent client must be dropped within the first-byte patience plus tolerance");
            assert_eq!(read.expect("a read after close"), 0, "the proxy must close, not answer");
            assert!(served.await.expect("no panic").is_err());
        }

        /// **A request head that does not follow a completed handshake within
        /// the head patience is answered 408**, however long the handshake was
        /// allowed.
        #[tokio::test]
        async fn a_head_that_does_not_follow_the_handshake_promptly_is_answered_408() {
            let short = std::time::Duration::from_millis(300);
            let long = std::time::Duration::from_secs(30);
            let (address, fleet, served) =
                one_connection(Patience { first_byte: long, handshake: long, head: short }).await;
            let stream = tokio::net::TcpStream::connect(address).await.expect("connect");
            let name = rustls_pki_types::ServerName::try_from(MACHINE).expect("a name");
            let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(caller(&fleet, None)));
            let mut tls = connector.connect(name.to_owned(), stream).await.expect("a handshake");
            let mut answer = Vec::new();
            tokio::time::timeout(short + std::time::Duration::from_secs(3), tls.read_to_end(&mut answer))
                .await
                .expect("the head patience must end the wait within tolerance")
                .expect("an answer");
            let answer = String::from_utf8_lossy(&answer);
            assert!(answer.starts_with("HTTP/1.1 408"), "{answer}");
            let _ = served.await;
        }

        /// The proxy's own numbers: sixty-four connections, twelve from one
        /// source, the rest kept for everyone else.
        #[test]
        fn the_proxy_holds_sixty_four_connections_and_twelve_per_source() {
            use rivet_core::connection_slots::{Outcome, Reason};
            let g = governor();
            let one = std::net::IpAddr::from([192, 168, 50, 10]);
            let mut held = Vec::new();
            for n in 0..PROXY_CONNECTIONS {
                // A source per twelve, so the pool fills without the share or
                // the rate deciding first.
                let src = std::net::IpAddr::from([192, 168, 50, u8::try_from(20 + n / 12).expect("small")]);
                match g.arrive(src, 1) {
                    Outcome::Admit(guard) => held.push(guard),
                    Outcome::Refuse { reason, .. } => panic!("connection {n} refused with {reason:?}"),
                }
            }
            assert_eq!(PROXY_CONNECTIONS, 64);
            assert!(matches!(g.arrive(one, 1), Outcome::Refuse { reason: Reason::Global, .. }));
            drop(held);
            for _ in 0..rivet_core::connection_slots::PER_SOURCE_SLOTS {
                match g.arrive(one, 1) {
                    Outcome::Admit(guard) => std::mem::forget(guard),
                    Outcome::Refuse { reason, .. } => panic!("refused within the share: {reason:?}"),
                }
            }
            assert!(matches!(g.arrive(one, 1), Outcome::Refuse { reason: Reason::PerSource, .. }));
        }

        // -------------------------------------------------------------------
        // Message framing — RFC 9112 §6.3, `transfer_coding`
        // -------------------------------------------------------------------

        /// [`backend`], answering with the EXACT bytes of the body it
        /// received rather than a fixed sentence — so a test can assert on
        /// content, not only on having been reached.
        async fn backend_echoing(ca: &Ca) -> u16 {
            let scratch = Scratch::new("echo");
            let (cert, key) = leaf(ca, MACHINE);
            scratch.write("tls-cert", &cert);
            scratch.write("tls-key", &key);
            scratch.write("tls-root", &ca.pem);
            let config = rivet_sdk::serve::Tls::in_directory(&scratch.0, "tls-cert", "tls-key")
                .requiring_a_caller_certificate_from("tls-root")
                .server_config()
                .expect("a backend that requires its caller to be the proxy");

            let listener =
                tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.expect("a port");
            let port = listener.local_addr().expect("an address").port();
            let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config));
            tokio::spawn(async move {
                let _scratch = scratch;
                while let Ok((stream, _)) = listener.accept().await {
                    let acceptor = acceptor.clone();
                    tokio::spawn(async move {
                        let Ok(mut tls) = acceptor.accept(stream).await else {
                            return;
                        };
                        let mut seen = Vec::new();
                        let mut buf = [0u8; 4096];
                        let head_end = loop {
                            if let Some(at) = seen.windows(4).position(|w| w == b"\r\n\r\n") {
                                break at + 4;
                            }
                            let read = tls.read(&mut buf).await.unwrap_or(0);
                            if read == 0 {
                                break seen.len();
                            }
                            seen.extend_from_slice(&buf[..read]);
                        };
                        let head = String::from_utf8_lossy(&seen[..head_end]).to_string();
                        let length: usize = rivet_core::cross_site::header(&head, "content-length")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0);
                        while seen.len() < head_end + length {
                            let read = tls.read(&mut buf).await.unwrap_or(0);
                            if read == 0 {
                                break;
                            }
                            seen.extend_from_slice(&buf[..read]);
                        }
                        let body = seen[head_end..seen.len().min(head_end + length)].to_vec();
                        let answer_head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = tls.write_all(answer_head.as_bytes()).await;
                        let _ = tls.write_all(&body).await;
                        let _ = tls.shutdown().await;
                    });
                }
            });
            port
        }

        /// **A chunked request body is refused rather than forwarded with its
        /// framing removed.** Nothing behind this proxy decodes
        /// `Transfer-Encoding: chunked` — `rivet-mcpd`'s git handler makes the
        /// same choice for the same reason — so the request never reaches the
        /// backend at all.
        #[tokio::test]
        async fn a_chunked_request_is_refused_and_never_reaches_the_backend() {
            let fleet = authority("rivet-fleet-ca");
            let (port, reached) = backend_counting(&fleet).await;
            let routes = vec![route("probe", port, true)];
            let head = format!(
                "POST /probe/api/v1/restore HTTP/1.1\r\nHost: {MACHINE}\r\n\
                 Transfer-Encoding: chunked\r\n\r\n"
            );
            let chunked_body = b"4\r\ntest\r\n0\r\n\r\n";
            let answer = exchange(&fleet, routes, caller(&fleet, Some(&fleet)), &head, chunked_body)
                .await
                .expect("the connection succeeds; the REQUEST is refused");
            assert!(answer.starts_with("HTTP/1.1 411 Length Required"), "{answer}");
            assert_eq!(
                reached.load(std::sync::atomic::Ordering::SeqCst),
                0,
                "a chunked request reached the application"
            );
        }

        /// **`Transfer-Encoding` and `Content-Length` together is refused
        /// outright** — RFC 9112 §6.3's request-smuggling shape — whatever the
        /// backend would have made of either one alone.
        #[tokio::test]
        async fn a_request_with_both_transfer_encoding_and_content_length_is_refused() {
            let fleet = authority("rivet-fleet-ca");
            let (port, reached) = backend_counting(&fleet).await;
            let routes = vec![route("probe", port, true)];
            let head = format!(
                "POST /probe/api/v1/restore HTTP/1.1\r\nHost: {MACHINE}\r\n\
                 Transfer-Encoding: chunked\r\nContent-Length: 14\r\n\r\n"
            );
            let answer = exchange(
                &fleet,
                routes,
                caller(&fleet, Some(&fleet)),
                &head,
                b"4\r\ntest\r\n0\r\n\r\n",
            )
            .await
            .expect("the connection succeeds; the REQUEST is refused");
            assert!(answer.starts_with("HTTP/1.1 400 Bad Request"), "{answer}");
            assert_eq!(reached.load(std::sync::atomic::Ordering::SeqCst), 0);
        }

        /// **A `Transfer-Encoding` naming anything other than exactly
        /// `chunked` is refused**, whether it names a coding nobody here
        /// speaks or names `chunked` beside something else.
        #[tokio::test]
        async fn an_unsupported_transfer_encoding_is_refused() {
            let fleet = authority("rivet-fleet-ca");
            let (port, reached) = backend_counting(&fleet).await;
            let routes = vec![route("probe", port, true)];
            for coding in ["gzip", "identity", "chunked, gzip", "gzip, chunked"] {
                let head = format!(
                    "POST /probe/api/v1/restore HTTP/1.1\r\nHost: {MACHINE}\r\n\
                     Transfer-Encoding: {coding}\r\n\r\n"
                );
                let answer = exchange(&fleet, routes.clone(), caller(&fleet, Some(&fleet)), &head, b"")
                    .await
                    .expect("the connection succeeds; the REQUEST is refused");
                assert!(answer.starts_with("HTTP/1.1 400 Bad Request"), "{coding}: {answer}");
            }
            assert_eq!(reached.load(std::sync::atomic::Ordering::SeqCst), 0);
        }

        /// **A `Content-Length` body still arrives at the backend
        /// unchanged**, byte for byte — the framing refusal above must not
        /// have touched the path every existing request already takes.
        #[tokio::test]
        async fn a_content_length_body_arrives_at_the_backend_unchanged() {
            let fleet = authority("rivet-fleet-ca");
            let port = backend_echoing(&fleet).await;
            let routes = vec![route("probe", port, true)];
            let body = b"the quick brown fox jumps over the lazy dog, then a little more padding";
            let head = format!(
                "POST /probe/api/v1/restore HTTP/1.1\r\nHost: {MACHINE}\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            let answer = exchange(&fleet, routes, caller(&fleet, Some(&fleet)), &head, body)
                .await
                .expect("the request is forwarded");
            assert!(answer.starts_with("HTTP/1.1 200 OK"), "{answer}");
            let sent = String::from_utf8_lossy(body);
            assert!(
                answer.ends_with(sent.as_ref()),
                "the backend must receive the exact body bytes unchanged: {answer}"
            );
        }

        /// **A repeated `Content-Length` is refused even when both copies
        /// agree** — `.rulings/` R-210 D17. Forwarding two same-named fields
        /// unread would leave the backend to pick one; matching values are no
        /// safer than disagreeing ones, so the request is refused before
        /// either copy reaches it.
        #[tokio::test]
        async fn a_repeated_identical_content_length_is_refused_and_never_reaches_the_backend() {
            let fleet = authority("rivet-fleet-ca");
            let (port, reached) = backend_counting(&fleet).await;
            let routes = vec![route("probe", port, true)];
            let head = format!(
                "POST /probe/api/v1/restore HTTP/1.1\r\nHost: {MACHINE}\r\n\
                 Content-Length: 5\r\nContent-Length: 5\r\n\r\n"
            );
            let answer = exchange(&fleet, routes, caller(&fleet, Some(&fleet)), &head, b"hello")
                .await
                .expect("the connection succeeds; the REQUEST is refused");
            assert!(answer.starts_with("HTTP/1.1 400 Bad Request"), "{answer}");
            assert_eq!(reached.load(std::sync::atomic::Ordering::SeqCst), 0);
        }

        /// **Two `Content-Length` fields that disagree are refused** — the
        /// classic smuggling shape: this proxy and the backend behind it could
        /// honour different ones and disagree about where the request ends.
        #[tokio::test]
        async fn two_disagreeing_content_lengths_are_refused_and_never_reach_the_backend() {
            let fleet = authority("rivet-fleet-ca");
            let (port, reached) = backend_counting(&fleet).await;
            let routes = vec![route("probe", port, true)];
            let head = format!(
                "POST /probe/api/v1/restore HTTP/1.1\r\nHost: {MACHINE}\r\n\
                 Content-Length: 5\r\nContent-Length: 7\r\n\r\n"
            );
            let answer = exchange(&fleet, routes, caller(&fleet, Some(&fleet)), &head, b"hello")
                .await
                .expect("the connection succeeds; the REQUEST is refused");
            assert!(answer.starts_with("HTTP/1.1 400 Bad Request"), "{answer}");
            assert_eq!(reached.load(std::sync::atomic::Ordering::SeqCst), 0);
        }

        // -------------------------------------------------------------------
        // Provisioning-shaped callers — the operator gate, `ADR-046` §4
        //
        // The fixtures above mint an unconstrained Ed25519 authority. These
        // mint with `crate::provision_ca`, the functions `cmd/rivet-provision`
        // mints real certificates with: a constrained ECDSA root, a machine's
        // front-door leaf composed as `stage::compose` composes one, and a
        // browser certificate with a person's name and no DNS name.
        // -------------------------------------------------------------------

        /// A machine's front-door leaf, as `stage::compose` composes it.
        fn provisioned_front_door(
            root: &crate::provision_ca::Minted,
            hostname: &str,
        ) -> crate::provision_ca::Minted {
            let suffixes = crate::provision_ca::permitted_suffixes(&root.certificate)
                .expect("the constraint reads back");
            let names: Vec<String> = suffixes.iter().map(|s| format!("{hostname}.{s}")).collect();
            crate::provision_ca::issue_from(&root.certificate, &root.key, &names, &names[0])
                .expect("a front-door leaf")
        }

        /// A client trusting `root`, presenting `leaf`.
        fn presenting(
            root: &crate::provision_ca::Minted,
            leaf: &crate::provision_ca::Minted,
        ) -> rustls::ClientConfig {
            let mut roots = rustls::RootCertStore::empty();
            roots
                .add(rustls_pki_types::CertificateDer::from(root.certificate_der.clone()))
                .expect("the root");
            rustls::ClientConfig::builder_with_provider(rivet_sdk::serve::provider())
                .with_safe_default_protocol_versions()
                .expect("versions")
                .with_root_certificates(roots)
                .with_client_auth_cert(
                    vec![rustls_pki_types::CertificateDer::from(leaf.certificate_der.clone())],
                    rustls_pki_types::PrivateKeyDer::Pkcs8(leaf.key_der.clone().into()),
                )
                .expect("a client identity")
        }

        /// `rivet-01`'s proxy, carrying what `stage::compose` stages for it.
        fn provisioned_proxy(root: &crate::provision_ca::Minted) -> Scratch {
            let scratch = Scratch::new("provisioned");
            let leaf = provisioned_front_door(root, "rivet-01");
            scratch.write(TLS_CERT, &leaf.certificate);
            scratch.write(TLS_KEY, &leaf.key);
            scratch.write(TLS_ROOT, &root.certificate);
            scratch
        }

        fn provisioned_authority() -> (crate::provision_ca::Authority, crate::provision_ca::Minted) {
            let authority = crate::provision_ca::Authority::root("gate-test-fleet", &["local".to_owned()])
                .expect("an authority");
            let root = authority.pem().expect("the root, serialised");
            (authority, root)
        }

        /// **The operator's provisioned browser certificate opens the index.**
        /// The control for the test below, with the certificate shapes a person
        /// actually mints.
        #[tokio::test]
        async fn a_provisioned_operator_certificate_is_served_the_index() {
            let (authority, root) = provisioned_authority();
            let person = authority.operator("Test Operator").expect("a browser certificate");
            let proxy = provisioned_proxy(&root);
            let head = format!("GET / HTTP/1.1\r\nHost: {MACHINE}\r\n\r\n");
            let page = exchange_in(&proxy, table(), presenting(&root, &person), &head, b"")
                .await
                .expect("the index is served");
            assert!(page.starts_with("HTTP/1.1 200 OK"), "{page}");
            assert!(page.contains("Test Operator"), "{page}");
        }

        /// **Another fleet machine's front-door certificate must not pass the
        /// gate.** `stage::compose` gives every fleet machine a leaf from the
        /// authority this proxy checks callers against, so the chain verifies;
        /// the leaf names a host, so it is a machine's and not the operator's
        /// (`.rulings/` R-187 D1). The index answers `403` naming the host.
        #[tokio::test]
        async fn another_machines_front_door_certificate_is_refused_the_index() {
            let (_authority, root) = provisioned_authority();
            let member = provisioned_front_door(&root, "rivet-02");
            let proxy = provisioned_proxy(&root);
            let head = format!("GET / HTTP/1.1\r\nHost: {MACHINE}\r\n\r\n");
            let page = exchange_in(&proxy, table(), presenting(&root, &member), &head, b"")
                .await
                .expect("the handshake succeeds; the REQUEST is refused");
            let said = page
                .find("Authenticated as")
                .map_or(page.as_str(), |at| &page[at..]);
            assert!(
                page.starts_with("HTTP/1.1 403 Forbidden"),
                "rivet-02's front-door certificate was served rivet-01's index, which says: {}",
                said.split("</footer>").next().unwrap_or(said)
            );
            assert!(page.contains("rivet-02.local"), "{page}");
            assert!(page.contains("names a host"), "{page}");
        }

        /// **And a gated route never reaches its application for it, while a
        /// public route serves it as it serves anybody.** The backend counts
        /// connections, so "refused" means no byte reached it — and the public
        /// route's backend accepting the proxy is `ADR-046` §4's direction
        /// working with a machine's certificate on both ends.
        #[tokio::test]
        async fn a_machines_certificate_reaches_a_public_route_and_no_gated_one() {
            let (_authority, root) = provisioned_authority();
            let member = provisioned_front_door(&root, "rivet-02");
            let (gated_port, gated_reached) = provisioned_backend(&root).await;
            let (public_port, public_reached) = provisioned_backend(&root).await;
            let routes = vec![
                route("notes", gated_port, true),
                route("tempmon", public_port, false),
            ];

            let proxy = provisioned_proxy(&root);
            let head = format!(
                "POST /notes/api/notes HTTP/1.1\r\nHost: {MACHINE}\r\nContent-Length: 0\r\n\r\n"
            );
            let client = presenting(&root, &member);
            let refused = exchange_in(&proxy, routes.clone(), client, &head, b"")
                .await
                .expect("the handshake succeeds");
            assert!(refused.starts_with("HTTP/1.1 403 Forbidden"), "{refused}");
            assert!(refused.contains("rivet-02.local"), "{refused}");

            let proxy = provisioned_proxy(&root);
            let head = format!("GET /tempmon/api/v1/sensors HTTP/1.1\r\nHost: {MACHINE}\r\n\r\n");
            let served = exchange_in(&proxy, routes, presenting(&root, &member), &head, b"")
                .await
                .expect("a public route is served");
            assert!(served.starts_with("HTTP/1.1 200 OK"), "{served}");
            assert!(served.contains("/api/v1/sensors"), "{served}");

            assert_eq!(gated_reached.load(std::sync::atomic::Ordering::SeqCst), 0);
            assert_eq!(public_reached.load(std::sync::atomic::Ordering::SeqCst), 1);
        }

        /// A backend holding `rivet-01`'s provisioned front-door leaf and
        /// requiring a caller certificate from `root`, counting connections.
        async fn provisioned_backend(
            root: &crate::provision_ca::Minted,
        ) -> (u16, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
            let reached = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let counter = std::sync::Arc::clone(&reached);
            let scratch = provisioned_proxy(root);
            let config = rivet_sdk::serve::Tls::in_directory(&scratch.0, TLS_CERT, TLS_KEY)
                .requiring_a_caller_certificate_from(TLS_ROOT)
                .server_config()
                .expect("a backend that requires its caller to hold a fleet certificate");
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.expect("a port");
            let port = listener.local_addr().expect("an address").port();
            let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config));
            tokio::spawn(async move {
                let _scratch = scratch;
                while let Ok((stream, _)) = listener.accept().await {
                    counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let acceptor = acceptor.clone();
                    tokio::spawn(async move {
                        let Ok(mut tls) = acceptor.accept(stream).await else {
                            return;
                        };
                        let head = read_head(&mut tls).await.unwrap_or_default();
                        let target = head.split(' ').nth(1).unwrap_or("?").to_string();
                        let body = format!("the backend was reached at {target}");
                        let answer = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = tls.write_all(answer.as_bytes()).await;
                        let _ = tls.shutdown().await;
                    });
                }
            });
            (port, reached)
        }
    }
}
