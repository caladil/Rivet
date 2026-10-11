//! The relay against a fake application on loopback — a real TLS server that
//! REQUIRES a caller certificate chaining to the fleet authority, which is what
//! every application behind `rivet-proxyd` is told to require (`ADR-046` §4).
//!
//! Every fixture is hand-composed: the authority, the proxy's leaf and the
//! application's leaf are minted per run into a scratch directory, the tables
//! are what `rivet_core::backup::render` and `rivet_core::proxy::render` write,
//! and the "application" is forty lines below. None of it is a capture.

// A test asserts; `unwrap`, `expect` and `panic!` ARE the assertion.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rivet_core::backup::Wanted;
use rivet_core::package::application::{BackupFormat, BackupSurface};

use super::*;

/// The machine's one fleet name, which the application's certificate carries.
const MACHINE: &str = "backup-test.local";

fn scratch(label: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "rivet-mcpd-backups-{label}-{}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("data")).expect("a scratch directory");
    dir
}

fn pem(label: &str, der: &[u8]) -> String {
    let text = rivet_core::base64::encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for chunk in text.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).expect("base64 is ASCII"));
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

/// A fleet authority; the proxy's leaf, written where the proxy keeps it; and
/// the application's server configuration, which requires a caller certificate
/// from that authority.
struct Fleet {
    application: Arc<rustls::ServerConfig>,
}

fn fleet(dir: &Path, proxy_names: &[&str]) -> Fleet {
    use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair};
    let alg = &rcgen::PKCS_ECDSA_P256_SHA256;
    let root_key = KeyPair::generate_for(alg).expect("a root key");
    let mut root_params = CertificateParams::new(Vec::<String>::new()).expect("params");
    root_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let root = root_params.self_signed(&root_key).expect("a root");
    let issuer = Issuer::new(root_params, root_key);
    let leaf = |names: Vec<String>| {
        let key = KeyPair::generate_for(alg).expect("a key");
        let mut params = CertificateParams::new(names).expect("params");
        params.is_ca = IsCa::NoCa;
        (params.signed_by(&key, &issuer).expect("a leaf"), key)
    };
    let (proxy, proxy_key) = leaf(proxy_names.iter().map(|n| (*n).to_owned()).collect());
    let (server, server_key) = leaf(vec![MACHINE.to_owned()]);

    let (cert, key, root_file) = proxy_files();
    let data = dir.join("data");
    std::fs::write(data.join(cert), pem("CERTIFICATE", proxy.der())).expect("cert");
    std::fs::write(data.join(key), pem("PRIVATE KEY", &proxy_key.serialize_der())).expect("key");
    std::fs::write(data.join(root_file), pem("CERTIFICATE", root.der())).expect("root");

    let mut roots = rustls::RootCertStore::empty();
    roots.add(root.der().clone()).expect("a root");
    let provider = rivetos_tls::provider();
    let verifier =
        rustls::server::WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider.clone())
            .build()
            .expect("a verifier that REQUIRES a caller certificate");
    let application = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("versions")
        .with_client_cert_verifier(verifier)
        .with_single_cert(
            vec![server.der().clone()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(server_key.serialize_der().into()),
        )
        .expect("a server config");
    Fleet {
        application: Arc::new(application),
    }
}

/// What the application saw of one request: its head and its body.
type Seen = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

/// **The application**: answers every connection with `answer`, after reading
/// the head and exactly the body it declares. Returns its port.
fn application(config: Arc<rustls::ServerConfig>, answer: Vec<u8>, seen: Seen) -> u16 {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a port");
    let port = listener.local_addr().expect("an address").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let Ok(connection) = rustls::ServerConnection::new(config.clone()) else {
                continue;
            };
            let mut tls = rustls::StreamOwned::new(connection, stream);
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            let mut whole = true;
            while !head.ends_with(b"\r\n\r\n") {
                if !matches!(tls.read(&mut byte), Ok(1)) {
                    whole = false;
                    break;
                }
                head.push(byte[0]);
            }
            if !whole {
                continue;
            }
            let head = String::from_utf8_lossy(&head).to_string();
            let length: usize = head
                .lines()
                .find_map(|l| {
                    let (name, value) = l.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse().ok())?
                })
                .unwrap_or(0);
            let mut body = vec![0u8; length];
            if tls.read_exact(&mut body).is_err() {
                continue;
            }
            seen.lock().expect("lock").push((head, body));
            let _ = tls.write_all(&answer);
            tls.conn.send_close_notify();
            let _ = tls.flush();
        }
    });
    port
}

fn http(status: &str, body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/vnd.sqlite3\r\nContent-Length: {}\r\n\
         Content-Disposition: attachment; filename=\"probe-1.sqlite\"\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

/// The machine's two tables: `probe-rivet` routed at `port` and accepting a
/// restore, `plain-rivet` routed and accepting none, `feed-rivet` a feed, and
/// `unrouted-rivet` declared but given no route.
fn tables(dir: &Path, port: u16) {
    let artifact = |service: &str, route: &str, restore: Option<&str>| Wanted {
        service: service.to_owned(),
        route: route.to_owned(),
        surface: BackupSurface::Artifact {
            path: "api/v1/backup".to_owned(),
            format: BackupFormat::Sqlite,
            restore: restore.map(ToOwned::to_owned),
        },
    };
    let table = rivet_core::backup::render(&rivet_core::backup::build(&[
        artifact("probe-rivet", "probe", Some("api/v1/restore")),
        artifact("plain-rivet", "plain", None),
        artifact("unrouted-rivet", "unrouted", None),
        Wanted {
            service: "feed-rivet".to_owned(),
            route: "feed".to_owned(),
            surface: BackupSurface::Feed {
                path: "api/v1/blobs".to_owned(),
            },
        },
    ]));
    std::fs::write(dir.join("backups"), table).expect("the table");
    let route = |path: &str, service: &str| rivet_core::proxy::Route {
        path: path.to_owned(),
        service: service.to_owned(),
        port,
        requires_operator: true,
        rules: Vec::new(),
    };
    let routes = rivet_core::proxy::render(
        &[route("probe", "probe-rivet"), route("plain", "plain-rivet")],
        None,
    );
    std::fs::write(dir.join("routes"), routes).expect("the routes");
}

fn request(method: &str, target: &str, length: Option<u64>) -> Request {
    Request {
        method: method.to_owned(),
        target: target.to_owned(),
        digest: None,
        length,
        content_type: None,
        content_encoding: None,
        chunked: false,
    }
}

/// Plan and relay, the caller's body in `body`; what reached the caller back.
fn run(machine: &Machine, request: &Request, body: &[u8]) -> Result<(u16, Vec<u8>, usize), Response> {
    let upstream = plan(machine, request, true)?;
    let mut sent = 0usize;
    let mut more = |into: &mut [u8]| -> std::io::Result<usize> {
        let n = into.len().min(body.len() - sent);
        into[..n].copy_from_slice(&body[sent..sent + n]);
        sent += n;
        Ok(n)
    };
    let mut answer = Vec::new();
    let mut pieces = 0usize;
    let mut out = |bytes: &[u8]| -> std::io::Result<()> {
        assert!(bytes.len() <= PIECE, "a piece larger than {PIECE} was held");
        pieces += 1;
        answer.extend_from_slice(bytes);
        Ok(())
    };
    let status = relay(&upstream, &[], &mut more, &mut out)?;
    Ok((status, answer, pieces))
}

// ---------------------------------------------------------------------------
// The relay
// ---------------------------------------------------------------------------

/// **A backup is fetched from the application on loopback, presenting the
/// proxy's certificate, and its answer comes back byte for byte** — status,
/// headers and body.
#[test]
fn a_backup_reaches_the_application_as_the_proxy_does_and_comes_back_verbatim() {
    let dir = scratch("fetch");
    let fleet = fleet(&dir, &[MACHINE]);
    let body = vec![0x5a; 3000];
    let answer = http("200 OK", &body);
    let seen = Seen::default();
    let port = application(fleet.application, answer.clone(), seen.clone());
    tables(&dir, port);

    let machine = Machine::at(&dir);
    let (status, got, _) =
        run(&machine, &request("GET", "/backup/probe-rivet", Some(0)), &[]).expect("relayed");
    assert_eq!(status, 200);
    assert_eq!(got, answer, "the application's answer is relayed untouched");

    let seen = seen.lock().expect("lock");
    assert_eq!(seen.len(), 1);
    assert!(
        seen[0].0.starts_with("GET /api/v1/backup HTTP/1.1\r\n"),
        "the route's prefix is removed as the proxy removes it: {}",
        seen[0].0
    );
    assert!(seen[0].0.contains(&format!("Host: {MACHINE}\r\n")), "{}", seen[0].0);
    std::fs::remove_dir_all(dir).ok();
}

/// **The answer is streamed, never held**: a body much larger than one piece
/// reaches the caller in pieces no larger than [`PIECE`].
#[test]
fn a_large_backup_is_passed_on_in_pieces_rather_than_held() {
    let dir = scratch("large");
    let fleet = fleet(&dir, &[MACHINE]);
    let body: Vec<u8> = (0..(1024 * 1024 + 7)).map(|i| (i % 251) as u8).collect();
    let answer = http("200 OK", &body);
    let port = application(fleet.application, answer.clone(), Seen::default());
    tables(&dir, port);

    let (_, got, pieces) =
        run(&Machine::at(&dir), &request("GET", "/backup/probe-rivet", None), &[]).expect("relayed");
    assert_eq!(got, answer);
    assert!(pieces > 1, "a megabyte arrived in {pieces} piece(s)");
    std::fs::remove_dir_all(dir).ok();
}

/// **A restore carries the caller's body, whole, to the declared route with the
/// confirmation and the format's type** — and the application's answer back.
#[test]
fn a_restore_streams_the_body_to_the_declared_route_and_relays_the_answer() {
    let dir = scratch("restore");
    let fleet = fleet(&dir, &[MACHINE]);
    let reply = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 17\r\n\r\n{\"restored\":4096}".to_vec();
    let seen = Seen::default();
    let port = application(fleet.application, reply.clone(), seen.clone());
    tables(&dir, port);

    let body: Vec<u8> = (0..200_000u32).map(|i| (i % 7) as u8).collect();
    let (status, got, _) = run(
        &Machine::at(&dir),
        &request("POST", "/restore/probe-rivet", Some(body.len() as u64)),
        &body,
    )
    .expect("relayed");
    assert_eq!(status, 200);
    assert_eq!(got, reply);
    let seen = seen.lock().expect("lock");
    let (head, received) = &seen[0];
    assert!(head.starts_with("POST /api/v1/restore?confirm=true HTTP/1.1\r\n"), "{head}");
    assert!(head.contains("Content-Type: application/vnd.sqlite3\r\n"), "{head}");
    assert!(head.contains(&format!("Content-Length: {}\r\n", body.len())), "{head}");
    assert_eq!(received, &body, "the body arrives byte for byte");
    std::fs::remove_dir_all(dir).ok();
}

/// **An application's refusal is relayed as the application's** — a 409 with
/// its own words, and no header saying the machine composed it.
#[test]
fn an_applications_refusal_is_relayed_as_its_own() {
    let dir = scratch("app-refuses");
    let fleet = fleet(&dir, &[MACHINE]);
    let reply = b"HTTP/1.1 409 Conflict\r\nContent-Length: 11\r\n\r\nnot today\r\n".to_vec();
    let port = application(fleet.application, reply.clone(), Seen::default());
    tables(&dir, port);
    let (status, got, _) =
        run(&Machine::at(&dir), &request("POST", "/restore/probe-rivet", Some(3)), b"abc")
            .expect("relayed");
    assert_eq!(status, 409);
    assert_eq!(got, reply);
    assert!(!String::from_utf8_lossy(&got).contains(rivet_core::backup::ANSWERED_BY_HEADER));
    std::fs::remove_dir_all(dir).ok();
}

/// **The name the application is verified against is the proxy's**: the
/// `upstream-name` setting when the document gives one. A setting naming
/// something the application's certificate does not carry is a handshake the
/// relay refuses — it does not fall back to trusting loopback.
#[test]
fn the_application_is_verified_against_the_proxys_upstream_name() {
    let dir = scratch("upstream");
    // The proxy's leaf names nothing useful, so only the setting can be right.
    let fleet = fleet(&dir, &["proxy-only.test"]);
    let port = application(fleet.application, http("200 OK", b"x"), Seen::default());
    tables(&dir, port);
    let machine = Machine::at(&dir);

    let refused = run(&machine, &request("GET", "/backup/probe-rivet", None), &[])
        .expect_err("the certificate's first name is not the application's");
    assert_eq!(refused.status, 502, "{}", refused.body);

    std::fs::write(
        dir.join("settings"),
        format!("# generated\n{}={MACHINE}\n", rivet_core::proxy::UPSTREAM_NAME_SETTING),
    )
    .expect("settings");
    let (status, _, _) =
        run(&machine, &request("GET", "/backup/probe-rivet", None), &[]).expect("the setting names it");
    assert_eq!(status, 200);
    std::fs::remove_dir_all(dir).ok();
}

/// **Nothing at the port is a 502 composed here**, carrying the header that
/// says so.
#[test]
fn an_application_that_is_not_listening_is_a_bad_gateway() {
    let dir = scratch("down");
    fleet(&dir, &[MACHINE]);
    let port = {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a port");
        listener.local_addr().expect("an address").port()
    };
    tables(&dir, port);
    let refused = run(&Machine::at(&dir), &request("GET", "/backup/probe-rivet", None), &[])
        .expect_err("nothing is listening");
    assert_eq!(refused.status, 502);
    assert!(refused.body.contains("probe-rivet could not be reached"), "{}", refused.body);
    let rendered = render_refusal(&refused);
    assert!(
        rendered.contains(&format!("{}: {ANSWERED_BY}\r\n", rivet_core::backup::ANSWERED_BY_HEADER)),
        "{rendered}"
    );
    std::fs::remove_dir_all(dir).ok();
}

// ---------------------------------------------------------------------------
// Refusals made before anything is dialled
// ---------------------------------------------------------------------------

fn refused(dir: &Path, request: &Request, operator: bool) -> Response {
    match plan(&Machine::at(dir), request, operator) {
        Ok(upstream) => panic!("{} must be refused, and was planned", upstream.service),
        Err(response) => response,
    }
}

fn prepared(label: &str) -> PathBuf {
    let dir = scratch(label);
    fleet(&dir, &[MACHINE]);
    tables(&dir, 9);
    dir
}

#[test]
fn a_connection_that_named_nobody_is_refused() {
    let dir = prepared("nobody");
    let why = refused(&dir, &request("GET", "/backup/probe-rivet", None), false);
    assert_eq!(why.status, 403, "{}", why.body);
    std::fs::remove_dir_all(dir).ok();
}

/// **An application declaring no backup surface is refused by name**, and the
/// refusal says what the machine does offer.
#[test]
fn an_application_with_no_backup_surface_is_refused_by_name() {
    let dir = prepared("unknown");
    let why = refused(&dir, &request("GET", "/backup/logbook-rivet", None), true);
    assert_eq!(why.status, 404);
    assert!(why.body.contains("logbook-rivet declares no backup surface"), "{}", why.body);
    assert!(why.body.contains("probe-rivet"), "{}", why.body);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_feed_and_a_missing_restore_path_are_refused() {
    let dir = prepared("shapes");
    let why = refused(&dir, &request("GET", "/backup/feed-rivet", None), true);
    assert_eq!(why.status, 409);
    assert!(why.body.contains("FEED"), "{}", why.body);
    let why = refused(&dir, &request("POST", "/restore/plain-rivet", Some(3)), true);
    assert_eq!(why.status, 409);
    assert!(why.body.contains("NO RESTORE"), "{}", why.body);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn the_wrong_method_or_a_name_that_is_not_a_service_is_refused() {
    let dir = prepared("method");
    assert_eq!(refused(&dir, &request("POST", "/backup/probe-rivet", Some(1)), true).status, 405);
    assert_eq!(refused(&dir, &request("GET", "/restore/probe-rivet", None), true).status, 405);
    assert_eq!(refused(&dir, &request("GET", "/backup/../etc", None), true).status, 404);
    assert_eq!(refused(&dir, &request("GET", "/backup/", None), true).status, 404);
    std::fs::remove_dir_all(dir).ok();
}

/// A restore body is relayed with the length it declared, and an empty one is
/// never a backup.
#[test]
fn a_restore_with_no_length_a_chunked_body_or_an_empty_body_is_refused() {
    let dir = prepared("length");
    assert_eq!(refused(&dir, &request("POST", "/restore/probe-rivet", None), true).status, 411);
    let mut chunked = request("POST", "/restore/probe-rivet", None);
    chunked.chunked = true;
    assert_eq!(refused(&dir, &chunked, true).status, 411);
    assert_eq!(refused(&dir, &request("POST", "/restore/probe-rivet", Some(0)), true).status, 400);
    assert_eq!(
        refused(&dir, &request("POST", "/restore/probe-rivet", Some(crate::bulk::MAX_BYTES + 1)), true)
            .status,
        413
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn an_application_the_proxy_does_not_route_is_refused() {
    let dir = prepared("unrouted");
    let why = refused(&dir, &request("GET", "/backup/unrouted-rivet", None), true);
    assert_eq!(why.status, 503);
    assert!(why.body.contains("routes nothing"), "{}", why.body);
    std::fs::remove_dir_all(dir).ok();
}

/// **No fleet certificate where the proxy keeps it, no relay** — and the
/// refusal names the file.
#[test]
fn a_machine_without_the_proxys_certificate_is_refused_naming_the_file() {
    let dir = prepared("nocert");
    let (cert, _, _) = proxy_files();
    std::fs::remove_file(dir.join("data").join(cert)).expect("removed");
    let why = refused(&dir, &request("GET", "/backup/probe-rivet", None), true);
    assert_eq!(why.status, 503);
    assert!(why.body.contains(cert), "{}", why.body);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_machine_with_no_table_is_refused() {
    let dir = prepared("notable");
    std::fs::remove_file(dir.join("backups")).expect("removed");
    let why = refused(&dir, &request("GET", "/backup/probe-rivet", None), true);
    assert_eq!(why.status, 503);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn the_two_routes_are_recognised_and_nothing_else_is() {
    assert!(is_backup("/backup/probe-rivet"));
    assert!(is_backup("/restore/probe-rivet?x=1"));
    assert!(is_backup("/backup/../x"), "refused by name later, not handed to the payload endpoint");
    assert!(!is_backup("/payload/x.rvt"));
    assert!(!is_backup("/repository-objects/"));
    assert!(!is_backup("/fleet.git/info/refs"));
}
