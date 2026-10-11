//! **Who the 4443 handshake admits**, over a real TLS connection.
//!
//! Every connection here is served by [`super::serve_one`] with the
//! configuration [`super::browser_tls_from`] builds — the listener this program
//! runs, minus the socket the supervisor hands it. Every certificate is minted
//! by `cmd/rivet-provision`'s own authority module (`crate::provision_ca`),
//! composed the way `cmd/rivet-provision/src/stage.rs` composes a machine's
//! front door and the way `rivet-provision`'s authority mint composes the
//! operator's browser certificate. Nothing here reads a real key.
//!
//! **What "refused" means.** `admit` answers a caller whose certificate names
//! a host `403 Forbidden` with JSON-RPC error `-32003` before `dispatch` runs,
//! so a reply carrying that code and none of the codes below was turned away
//! by the operator gate (`ADR-043` §5, `.rulings/` R-187 D1).
//!
//! **What "reached a handler" means.** This test process has no
//! `RIVET_APP_DATA`, so every `/api/*` and `/mcp/*` handler answers with its
//! own JSON-RPC error code once `fleet::load` fails: `-32001` for
//! `GET /api/fleet`, `-32000` for `POST /mcp/<device>`, `-32010` for
//! `POST /api/commit/<device>`. Those codes come from `dispatch`, after the
//! handshake and after the cross-site guard, so a reply carrying one is a
//! request this program accepted from that caller.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::io::{Read as _, Write as _};
use std::sync::Arc;

use crate::provision_ca::{Authority, Minted, issue_from, permitted_suffixes};

/// A fleet's authority, as `rivet-provision`'s mint writes it.
struct Fleet {
    authority: Authority,
    root: Minted,
}

fn mint_fleet(name: &str) -> Fleet {
    let authority = Authority::root(name, &["local".to_owned()]).expect("an authority");
    let root = authority.pem().expect("the root, serialised");
    Fleet { authority, root }
}

/// A machine's front-door leaf, composed as `stage::compose` composes it:
/// every permitted suffix read back out of the root, one name per suffix, the
/// first as the subject, through `issue_from`.
fn front_door(fleet: &Fleet, hostname: &str) -> Minted {
    let suffixes = permitted_suffixes(&fleet.root.certificate).expect("the constraint reads back");
    let names: Vec<String> = suffixes.iter().map(|s| format!("{hostname}.{s}")).collect();
    issue_from(&fleet.root.certificate, &fleet.root.key, &names, &names[0])
        .expect("a front-door leaf")
}

/// The operator's browser certificate, as the authority mint issues it: no
/// DNS name, a person's name as the subject.
fn operator(fleet: &Fleet, person: &str) -> Minted {
    fleet.authority.operator(person).expect("a browser certificate")
}

/// The fleet server this suite talks to.
const SERVER: &str = "fleet-server";

/// Serve ONE connection with this program's own listener code, and send it
/// `request` as `caller` (or as nobody). The reply as text, or where the
/// exchange failed.
fn ask(fleet: &Fleet, caller: Option<&Minted>, request: &str) -> Result<String, String> {
    let server = front_door(fleet, SERVER);
    let config = Arc::new(
        super::browser_tls_from(
            server.certificate.as_bytes(),
            server.key.as_bytes(),
            fleet.root.certificate.as_bytes(),
        )
        .expect("the configuration rivet-fleetd serves with"),
    );

    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a port");
    let address = listener.local_addr().expect("an address");
    let served = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("a connection");
        // `serve_one` sets its own read deadline from `deadline` below, on
        // every read including the first; the ten seconds here only ever
        // bounded the answer this thread waits to receive, not anything
        // `serve_one` reads.
        let deadline = std::time::Instant::now() + super::PATIENCE;
        super::serve_one(&stream, &config, deadline);
    });

    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from(fleet.root.certificate_der.clone()))
        .expect("the root");
    let builder = rustls::ClientConfig::builder_with_provider(rivetos_tls::provider())
        .with_safe_default_protocol_versions()
        .expect("versions")
        .with_root_certificates(roots);
    let client = match caller {
        None => builder.with_no_client_auth(),
        Some(leaf) => builder
            .with_client_auth_cert(
                vec![rustls::pki_types::CertificateDer::from(leaf.certificate_der.clone())],
                rustls::pki_types::PrivateKeyDer::Pkcs8(leaf.key_der.clone().into()),
            )
            .expect("a client identity"),
    };

    let outcome = (|| {
        let name = rustls::pki_types::ServerName::try_from(format!("{SERVER}.local"))
            .map_err(|e| format!("name: {e}"))?;
        let connection = rustls::ClientConnection::new(Arc::new(client), name)
            .map_err(|e| format!("client: {e}"))?;
        let socket = std::net::TcpStream::connect(address).map_err(|e| format!("connect: {e}"))?;
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .map_err(|e| format!("deadline: {e}"))?;
        let mut tls = rustls::StreamOwned::new(connection, socket);
        tls.write_all(request.as_bytes()).map_err(|e| format!("write: {e}"))?;
        tls.flush().map_err(|e| format!("flush: {e}"))?;
        let mut answer = Vec::new();
        match tls.read_to_end(&mut answer) {
            Ok(_) => Ok(String::from_utf8_lossy(&answer).into_owned()),
            // A close without `close_notify` after a whole answer is how this
            // server ends a connection; the bytes in hand are the answer.
            Err(_) if !answer.is_empty() => Ok(String::from_utf8_lossy(&answer).into_owned()),
            Err(e) => Err(format!("read: {e}")),
        }
    })();
    let _ = served.join();
    outcome
}

/// The operator routes, each with the code its handler answers with here.
fn operator_routes() -> Vec<(&'static str, String, &'static str)> {
    let json = |line: &str| {
        format!(
            "{line} HTTP/1.1\r\nHost: {SERVER}.local:4443\r\nContent-Type: application/json\r\n\
             Content-Length: 2\r\n\r\n{{}}"
        )
    };
    vec![
        (
            "GET /api/fleet",
            format!("GET /api/fleet HTTP/1.1\r\nHost: {SERVER}.local:4443\r\n\r\n"),
            "\"code\":-32001",
        ),
        ("POST /api/commit/<device>", json("POST /api/commit/member-01"), "\"code\":-32010"),
        ("POST /mcp/<device>", json("POST /mcp/member-01"), "\"code\":-32000"),
    ]
}

/// **The operator's certificate reaches every operator route.** The control
/// for the test below: the listener, the configuration and the certificates
/// are the real ones, and an operator is served.
#[test]
fn the_operators_browser_certificate_reaches_every_operator_route() {
    let fleet = mint_fleet("gate-test-fleet");
    let person = operator(&fleet, "Test Operator");
    for (route, request, handled) in operator_routes() {
        let reply = ask(&fleet, Some(&person), &request)
            .unwrap_or_else(|why| panic!("{route}: the operator was refused: {why}"));
        assert!(reply.starts_with("HTTP/1.1 200 OK"), "{route}: {reply}");
        assert!(reply.contains(handled), "{route} did not reach its handler: {reply}");
    }
    let page = format!("GET / HTTP/1.1\r\nHost: {SERVER}.local:4443\r\n\r\n");
    let reply = ask(&fleet, Some(&person), &page).expect("the operator is served the page");
    assert!(reply.starts_with("HTTP/1.1 200 OK\r\nContent-Type: text/html"), "{reply}");
}

/// **A certificate from another authority never completes a handshake**, so
/// the admission above is the fleet authority's and not an open door.
#[test]
fn a_certificate_from_another_authority_is_refused_at_the_handshake() {
    let fleet = mint_fleet("gate-test-fleet");
    let elsewhere = mint_fleet("somebody-elses-fleet");
    let stranger = operator(&elsewhere, "Test Operator");
    let why = ask(&fleet, Some(&stranger), "GET /api/fleet HTTP/1.1\r\n\r\n")
        .expect_err("a chain to another authority must not be served");
    assert!(why.starts_with("read") || why.starts_with("write"), "{why}");
}

/// And no certificate at all is refused in the handshake too.
#[test]
fn no_certificate_is_refused_at_the_handshake() {
    let fleet = mint_fleet("gate-test-fleet");
    let why = ask(&fleet, None, "GET /api/fleet HTTP/1.1\r\n\r\n")
        .expect_err("a caller with no certificate must not be served");
    assert!(why.starts_with("read") || why.starts_with("write"), "{why}");
}

/// **A fleet member's front-door certificate must not drive the fleet.**
///
/// `stage::compose` mints every fleet machine's `rivet-proxyd` leaf from the
/// same authority this listener trusts, so the handshake admits it. Without
/// `admit`, whoever held one committed to the fleet repository and reached
/// every machine in the register with this server's operator identity. Each
/// operator route, and the page, answers it `403` naming the host its
/// certificate names, and no handler runs.
#[test]
fn a_members_front_door_certificate_is_refused_by_every_operator_route() {
    let fleet = mint_fleet("gate-test-fleet");
    let member = front_door(&fleet, "member-01");
    let page = format!("GET / HTTP/1.1\r\nHost: {SERVER}.local:4443\r\n\r\n");
    let mut routes = operator_routes();
    routes.push(("GET / (the portal page)", page, "text/html"));
    let mut admitted = Vec::new();
    for (route, request, handled) in routes {
        let reply = ask(&fleet, Some(&member), &request)
            .unwrap_or_else(|why| panic!("{route}: the handshake should succeed: {why}"));
        if reply.contains(handled) || !reply.starts_with("HTTP/1.1 403 Forbidden") {
            admitted.push(format!("{route} -> {}", reply.lines().last().unwrap_or("")));
        }
        assert!(reply.contains("\"code\":-32003"), "{route}: {reply}");
        assert!(reply.contains("member-01.local"), "{route}: {reply}");
    }
    assert!(
        admitted.is_empty(),
        "a member's front-door certificate reached {} operator route(s):\n{}",
        admitted.len(),
        admitted.join("\n")
    );
}

/// **This server's own front-door certificate is refused too.** A fleet
/// server's `rivet-proxyd` holds the same leaf this listener serves, so a
/// process that can read that one file must not become the operator either.
#[test]
fn the_servers_own_front_door_certificate_is_refused() {
    let fleet = mint_fleet("gate-test-fleet");
    let own = front_door(&fleet, SERVER);
    let reply = ask(&fleet, Some(&own), "GET /api/fleet HTTP/1.1\r\n\r\n")
        .expect("the handshake succeeds");
    assert!(reply.starts_with("HTTP/1.1 403 Forbidden"), "{reply}");
    assert!(!reply.contains("\"code\":-32001"), "the register handler ran: {reply}");
}

/// **The read deadline is absolute, not per read.** A peer that sends one
/// byte at a time, each comfortably inside any reasonable per-read timeout,
/// must still be disconnected once the WHOLE exchange has taken too long —
/// otherwise the gap between reads is exactly what a slot can be held open
/// with. The handshake here is real and admitted; the request line that
/// follows it is deliberately never FINISHED — no blank line ever arrives —
/// so the only thing that can end this connection is the absolute deadline.
///
/// **The trickle keeps going well past the tolerance this test allows for a
/// disconnect.** That is what makes this a real test of the deadline rather
/// than of how long the trickle happens to run: a per-read-only patience
/// would still be sitting inside each 40ms gap when this test's own patience
/// runs out waiting for a disconnect that a flat per-read timeout would
/// never produce.
#[test]
fn a_trickled_request_head_is_disconnected_at_the_absolute_deadline_not_never() {
    let fleet = mint_fleet("gate-test-fleet");
    let server_leaf = front_door(&fleet, SERVER);
    let config = Arc::new(
        super::browser_tls_from(
            server_leaf.certificate.as_bytes(),
            server_leaf.key.as_bytes(),
            fleet.root.certificate.as_bytes(),
        )
        .expect("the configuration rivet-fleetd serves with"),
    );

    // Short enough that this test does not wait thirty real seconds for it.
    let patience = std::time::Duration::from_millis(300);
    let tolerance = std::time::Duration::from_secs(3);
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a port");
    let address = listener.local_addr().expect("an address");

    // A signal rather than `JoinHandle::join`, which has no timeout: this
    // test must be able to say "no disconnect arrived in time" rather than
    // block forever if the deadline is not honoured.
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("a connection");
        let deadline = std::time::Instant::now() + patience;
        super::serve_one(&stream, &config, deadline);
        let _ = done_tx.send(());
    });

    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from(fleet.root.certificate_der.clone()))
        .expect("the root");
    let person = operator(&fleet, "Test Operator");
    let client = rustls::ClientConfig::builder_with_provider(rivetos_tls::provider())
        .with_safe_default_protocol_versions()
        .expect("versions")
        .with_root_certificates(roots)
        .with_client_auth_cert(
            vec![rustls::pki_types::CertificateDer::from(person.certificate_der.clone())],
            rustls::pki_types::PrivateKeyDer::Pkcs8(person.key_der.clone().into()),
        )
        .expect("a client identity");

    let name = rustls::pki_types::ServerName::try_from(format!("{SERVER}.local")).expect("a name");
    let connection = rustls::ClientConnection::new(Arc::new(client), name).expect("a client");
    let socket = std::net::TcpStream::connect(address).expect("connect");
    let mut tls = rustls::StreamOwned::new(connection, socket);

    // The trickling client runs on its own thread: the point under test is
    // that the SERVER ends the connection unasked, not that this side gives
    // up sending. One real request line, with no terminating blank line, then
    // one byte every 40ms for far longer than `patience` plus `tolerance` —
    // 150 * 40ms = 6s against a 3.3s allowance.
    std::thread::spawn(move || {
        let _ = tls.write_all(b"GET /api/fleet HTTP/1.1\r\n");
        let _ = tls.flush();
        for _ in 0..150 {
            std::thread::sleep(std::time::Duration::from_millis(40));
            if tls.write_all(b"X").is_err() || tls.flush().is_err() {
                break;
            }
        }
    });

    if done_rx.recv_timeout(patience + tolerance).is_err() {
        panic!(
            "the server did not disconnect a trickled, never-finished request head within the \
             absolute deadline ({patience:?}) plus tolerance ({tolerance:?}) — a per-read-only \
             patience would still be waiting inside each 40ms gap"
        );
    }
}
