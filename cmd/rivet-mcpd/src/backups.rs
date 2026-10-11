//! **Backups and restores, over the control plane** — `ADR-051` §8 and §12,
//! `ADR-035` §3, `.rulings/` R-234.
//!
//! Two routes on the bulk endpoint's listener, decided by the target like
//! every other handler here:
//!
//! ```text
//! GET  /backup/<service>    HTTP/1.1
//! POST /restore/<service>   HTTP/1.1   Content-Length: <n>
//! ```
//!
//! For each, this machine looks the application up in its own backup table
//! (`rivet_core::backup::PATH`), finds the loopback port `rivet-proxyd` routes
//! it to, dials it over TLS exactly as the proxy does, and sends the declared
//! request with the route's prefix removed — `GET /<backup-path>`, or
//! `POST /<restore-path>?confirm=true` carrying the caller's body. **The
//! application's answer comes back verbatim**, status line, headers and body,
//! copied in 64 KiB pieces as it arrives: nothing here holds a whole backup,
//! because a Reliquary archive is tens of megabytes on a board with under a
//! gigabyte of memory.
//!
//! # Who may ask
//!
//! A connection that presented a pinned key — the operator, `ADR-025` §2. The
//! plaintext loopback transport names nobody and is refused, as the repository
//! store refuses it.
//!
//! # Which certificate reaches the application, and why it is not a new one
//!
//! Every application behind the proxy requires a caller certificate chaining
//! to the fleet authority (`ADR-046` §4). The proxy presents the machine's own
//! fleet certificate, from its data directory, and so does this: the same
//! three files (`rivet_core::fleet_root::HANDED_TO`), verified against the same
//! name (`rivet_core::proxy::UPSTREAM_NAME_SETTING`, or the first DNS name on
//! that certificate). Nothing is minted and nothing is handed anywhere new —
//! this daemon runs as root and could already read them; the claim made to the
//! application is the one the proxy makes, *a program the fleet authority
//! signed a key for connected*.
//!
//! # What the machine checks, and what it leaves to the two ends
//!
//! It checks the request names an application whose signed manifest declared
//! the surface asked for, and refuses by name otherwise. It does not validate a
//! backup: the workstation runs `ADR-051` §7's ladder on what arrives, and an
//! application validates a restore body whole before it changes anything
//! (§12). A refusal composed here carries
//! `rivet_core::backup::ANSWERED_BY_HEADER`, so a workstation can tell *this
//! machine refused* from *the application answered*.

use std::io::{Read as _, Write as _};
use std::sync::Arc;

use rivet_core::backup::{self, Declaration, Requested};
use rivet_core::package::application::BackupSurface;
use rivet_core::service::ServiceName;

use crate::bulk::{Request, Response};

/// The value [`backup::ANSWERED_BY_HEADER`] carries on this daemon's refusals.
pub const ANSWERED_BY: &str = "rivet-mcpd";

/// The size of one piece copied in either direction.
const PIECE: usize = 64 * 1024;

/// Whether `target` is one of the two routes this module serves.
///
/// **By prefix, before the name is checked**, so a malformed name on these
/// routes is refused here by name rather than falling through to the payload
/// endpoint's refusal about `.rvt` files.
#[must_use]
pub fn is_backup(target: &str) -> bool {
    let path = target.split('?').next().unwrap_or(target);
    path.starts_with(backup::CONTROL_PLANE_BACKUP) || path.starts_with(backup::CONTROL_PLANE_RESTORE)
}

/// Where this machine keeps what the relay reads.
///
/// **Every path a parameter**, for `crate::repository::Machine`'s reason: the
/// tests point it at a scratch directory and a fake application, and never at
/// a machine's real files.
#[derive(Debug, Clone)]
pub struct Machine {
    /// The backup table — `rivet_core::backup::PATH`.
    table: String,
    /// The proxy's routing table — `rivet_core::proxy::path`.
    routes: String,
    /// The proxy's settings file — `rivet_core::settings::path`.
    settings: String,
    /// The proxy's data directory, holding its certificate, key and root.
    proxy_data: String,
    /// Where an application listens. Loopback on a machine; a test may name
    /// the same.
    host: String,
}

impl Machine {
    /// This machine's files: the table in `/run/rivet`, and `rivet-proxyd`'s
    /// home on the built-in volume, where `cmd/rivet-sysd` defines it
    /// (`ADR-060`).
    #[must_use]
    pub fn on_this_machine() -> Self {
        let volume = rivet_core::service::BUILT_IN_VOLUME;
        let proxy = ServiceName::new(rivet_core::service::PROXYD_SERVICE_NAME);
        let (routes, settings, proxy_data) = proxy.as_ref().map_or_else(
            // `PROXYD_SERVICE_NAME` is a valid name, which `rivet-core`'s own
            // tests assert; the fallback spells the same paths rather than
            // panicking in the daemon that is the machine's way in.
            || {
                let home = format!(
                    "{}/{}",
                    rivet_core::service::app_root(volume),
                    rivet_core::service::PROXYD_SERVICE_NAME
                );
                (
                    format!("{home}/{}", rivet_core::proxy::FILE),
                    format!("{home}/{}", rivet_core::settings::FILE),
                    format!("{home}/{}", rivet_core::service::DATA_DIR),
                )
            },
            |name| {
                (
                    rivet_core::proxy::path(volume, name),
                    rivet_core::settings::path(volume, name),
                    rivet_core::service::data_dir(volume, name),
                )
            },
        );
        Self {
            table: backup::PATH.to_owned(),
            routes,
            settings,
            proxy_data,
            host: "127.0.0.1".to_owned(),
        }
    }

    /// Every file under `root`, laid out as a test writes it: `backups`,
    /// `routes`, `settings`, and the proxy's three files in `data/`.
    #[cfg(test)]
    #[must_use]
    pub fn at(root: &std::path::Path) -> Self {
        let root = root.to_string_lossy();
        Self {
            table: format!("{root}/backups"),
            routes: format!("{root}/routes"),
            settings: format!("{root}/settings"),
            proxy_data: format!("{root}/data"),
            host: "127.0.0.1".to_owned(),
        }
    }
}

/// The proxy's names for its three files — `rivet_core::fleet_root::HANDED_TO`,
/// which is where the minted material is handed and under which names.
fn proxy_files() -> (&'static str, &'static str, &'static str) {
    rivet_core::fleet_root::HANDED_TO
        .iter()
        .find(|h| h.application == rivet_core::service::PROXYD_SERVICE_NAME)
        .map_or(("tls-cert", "tls-key", "tls-root"), |h| (h.certificate, h.key, h.root))
}

/// **One relay, decided** — everything [`relay`] needs, worked out before a
/// byte of body is read or a socket is opened.
pub struct Upstream {
    /// The application, for the journal and for refusals.
    pub service: String,
    /// Which of the two routes.
    pub requested: Requested,
    port: u16,
    name: rustls::pki_types::ServerName<'static>,
    config: Arc<rustls::ClientConfig>,
    host: String,
    /// The request head sent to the application.
    head: String,
    /// How many bytes of body follow the head — the caller's `Content-Length`
    /// for a restore, zero for a backup.
    body: u64,
}

/// A refusal from this daemon, never one an application sent.
fn refuse(status: u16, reason: &'static str, body: String) -> Response {
    Response { status, reason, body }
}

/// **A refusal on these routes, as bytes** — [`Response::render`] with
/// [`backup::ANSWERED_BY_HEADER`] added, so a workstation can tell this
/// machine's refusal from an application's answer of the same status.
#[must_use]
pub fn render_refusal(response: &Response) -> String {
    format!(
        "HTTP/1.1 {} {}\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         {}: {ANSWERED_BY}\r\n\
         Connection: close\r\n\
         \r\n\
         {}",
        response.status,
        response.reason,
        response.body.len(),
        backup::ANSWERED_BY_HEADER,
        response.body
    )
}

/// **Decide one request**: who asked, which application, which route, which
/// port, which certificate.
///
/// `operator` is whether the connection presented a pinned key.
///
/// # Errors
///
/// A [`Response`] to send instead — every refusal here is made before a byte of
/// body is read and before the application is dialled.
pub fn plan(machine: &Machine, request: &Request, operator: bool) -> Result<Upstream, Response> {
    let path = request.target.split('?').next().unwrap_or(&request.target);
    let Some((requested, service)) = backup::requested(path) else {
        return Err(refuse(
            404,
            "Not Found",
            format!(
                "{:?} names no application. This endpoint serves GET {}<service> and POST \
                 {}<service>, where <service> is one application's name as the machine's backup \
                 table carries it (ADR-051 §8, §12).\n",
                request.target,
                backup::CONTROL_PLANE_BACKUP,
                backup::CONTROL_PLANE_RESTORE,
            ),
        ));
    };
    let method = match requested {
        Requested::Backup => "GET",
        Requested::Restore => backup::RESTORE_METHOD,
    };
    if request.method != method {
        return Err(refuse(
            405,
            "Method Not Allowed",
            format!(
                "{} {path} is not a route here: {method} is ({}{service} is fetched with GET and \
                 {}{service} sent with POST).\n",
                request.method,
                backup::CONTROL_PLANE_BACKUP,
                backup::CONTROL_PLANE_RESTORE,
            ),
        ));
    }
    if !operator {
        return Err(refuse(
            403,
            "Forbidden",
            "this connection presented no pinned key, and an application's data is fetched or \
             replaced only for the operator (ADR-025 §2). Connect over the control plane's TLS \
             with an identity this machine's peers file names.\n"
                .to_owned(),
        ));
    }

    let table = std::fs::read_to_string(&machine.table).map_err(|e| {
        refuse(
            503,
            "Service Unavailable",
            format!(
                "this machine's backup table at {} could not be read ({e}). The supervisor writes \
                 it every convergence, even when it is empty, so its absence means a build from \
                 before ADR-051 or a supervisor that has not converged yet.\n",
                machine.table
            ),
        )
    })?;
    let declarations = backup::parse(&table);
    let Some(declaration) = declarations.iter().find(|d| d.service == service) else {
        let offered: Vec<&str> = declarations.iter().map(|d| d.service.as_str()).collect();
        return Err(refuse(
            404,
            "Not Found",
            format!(
                "{service} declares no backup surface on this machine, so there is nothing to \
                 fetch from it or restore to it.\n\
                 \x20 offers  {}\n\
                 \x20 why     an application offers a backup by declaring backup-path and \
                 backup-format in its signed manifest (ADR-051 §3); nothing configures one.\n",
                if offered.is_empty() {
                    "nothing -- no application on this machine offers a backup".to_owned()
                } else {
                    offered.join(", ")
                }
            ),
        ));
    };

    let (application_path, body, content_type) = match (requested, &declaration.surface) {
        (_, BackupSurface::Feed { .. }) => {
            return Err(refuse(
                409,
                "Conflict",
                format!(
                    "{service} offers a backup FEED, which is not fetched or restored in one \
                     request: this build implements the artifact half only (ADR-051 §8, §12).\n"
                ),
            ));
        }
        (Requested::Backup, BackupSurface::Artifact { .. }) => {
            (declaration.application_path(), 0, None)
        }
        (Requested::Restore, BackupSurface::Artifact { format, .. }) => {
            let Some(restore) = declaration.application_restore_path() else {
                return Err(refuse(
                    409,
                    "Conflict",
                    format!(
                        "{service} offers a backup and NO RESTORE: its signed manifest declares no \
                         restore-path, so nothing was sent to it (ADR-051 §12).\n"
                    ),
                ));
            };
            let length = restore_length(request, service)?;
            (
                format!("{restore}?{}", backup::RESTORE_QUERY),
                length,
                Some(format.media_type()),
            )
        }
    };

    let port = route_port(machine, declaration)?;
    let (config, name, host_name) = client(machine)?;
    let mut head = format!(
        "{method} {application_path} HTTP/1.1\r\n\
         Host: {host_name}\r\n"
    );
    if let Some(media) = content_type {
        head.push_str(&format!("Content-Type: {media}\r\nContent-Length: {body}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");

    Ok(Upstream {
        service: service.to_owned(),
        requested,
        port,
        name,
        config,
        host: machine.host.clone(),
        head,
        body,
    })
}

/// The body a restore declares, refused before a byte of it is read when it
/// cannot be relayed.
fn restore_length(request: &Request, service: &str) -> Result<u64, Response> {
    if request.chunked {
        return Err(refuse(
            411,
            "Length Required",
            "a restore is relayed with the length it declared; this build does not decode \
             Transfer-Encoding: chunked, and an application behind the proxy does not either.\n"
                .to_owned(),
        ));
    }
    let Some(length) = request.length else {
        return Err(refuse(
            411,
            "Length Required",
            format!("a restore to {service} carries a Content-Length, and this one carried none.\n"),
        ));
    };
    if length == 0 {
        return Err(refuse(
            400,
            "Bad Request",
            format!(
                "a restore to {service} declared an empty body. An empty file is never a backup of \
                 anything (ADR-051 §7's first rung), so nothing was sent.\n"
            ),
        ));
    }
    if length > crate::bulk::MAX_BYTES {
        return Err(refuse(
            413,
            "Content Too Large",
            format!(
                "Content-Length is {length} and this endpoint relays at most {} bytes.\n",
                crate::bulk::MAX_BYTES
            ),
        ));
    }
    Ok(length)
}

/// The loopback port the proxy routes this application to.
fn route_port(machine: &Machine, declaration: &Declaration) -> Result<u16, Response> {
    let text = std::fs::read_to_string(&machine.routes).map_err(|e| {
        refuse(
            503,
            "Service Unavailable",
            format!(
                "the routing table at {} could not be read ({e}), so this machine does not know \
                 which loopback port {} listens on. The supervisor writes it while rivet-proxyd is \
                 part of this machine (ADR-046 §5).\n",
                machine.routes, declaration.service
            ),
        )
    })?;
    rivet_core::proxy::parse(&text)
        .into_iter()
        .find(|r| r.path == declaration.route && r.service == declaration.service)
        .map(|r| r.port)
        .ok_or_else(|| {
            refuse(
                503,
                "Service Unavailable",
                format!(
                    "{} declares a backup at /{} and the routing table routes nothing of its at \
                     that path, so it has no loopback port to reach. The supervisor journals why \
                     an application has no route — two applications asking for one path, or no TCP \
                     socket declared (ADR-046 §5).\n",
                    declaration.service, declaration.route
                ),
            )
        })
}

/// **The client the proxy is**: its certificate and key presented, the fleet
/// authority as the only root, and the name the proxy verifies backends
/// against.
fn client(
    machine: &Machine,
) -> Result<(Arc<rustls::ClientConfig>, rustls::pki_types::ServerName<'static>, String), Response> {
    use rustls::pki_types::pem::PemObject as _;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};

    let (cert_file, key_file, root_file) = proxy_files();
    let read = |file: &str| -> Result<Vec<u8>, Response> {
        let at = format!("{}/{file}", machine.proxy_data);
        std::fs::read(&at).map_err(|e| {
            refuse(
                503,
                "Service Unavailable",
                format!(
                    "{at} could not be read ({e}). It is the machine's fleet certificate as the \
                     proxy holds it, and every application behind the proxy requires a caller \
                     to present it (ADR-046 §4), so no application can be reached without it.\n"
                ),
            )
        })
    };
    let unusable = |what: String| {
        refuse(
            500,
            "Internal Server Error",
            format!("the proxy's fleet certificate is not usable here: {what}\n"),
        )
    };

    let cert_pem = read(cert_file)?;
    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&cert_pem)
        .collect::<Result<_, _>>()
        .map_err(|e| unusable(format!("{cert_file}: {e}")))?;
    let leaf = chain.first().ok_or_else(|| unusable(format!("{cert_file} carries no certificate")))?;
    let key = PrivateKeyDer::from_pem_slice(&read(key_file)?)
        .map_err(|e| unusable(format!("{key_file}: {e}")))?;
    let mut roots = rustls::RootCertStore::empty();
    for root in CertificateDer::pem_slice_iter(&read(root_file)?) {
        let root = root.map_err(|e| unusable(format!("{root_file}: {e}")))?;
        roots.add(root).map_err(|e| unusable(format!("{root_file}: {e}")))?;
    }
    if roots.is_empty() {
        return Err(unusable(format!("{root_file} carries no certificate")));
    }

    // **The proxy's own rule for the name**: the setting when the document
    // gives one, otherwise the first DNS name on the certificate.
    let setting = std::fs::read_to_string(&machine.settings)
        .ok()
        .and_then(|text| {
            rivet_core::settings::parse(&text)
                .into_iter()
                .find(|(key, _)| key == rivet_core::proxy::UPSTREAM_NAME_SETTING)
                .map(|(_, value)| value)
        });
    let upstream = match setting {
        Some(name) => name,
        None => rivetos_tls::dns_names(leaf)
            .and_then(|names| names.into_iter().next())
            .ok_or_else(|| {
                unusable(format!(
                    "the `{}` setting is not set and {cert_file} names no DNS host, so there is \
                     no name to verify the application against",
                    rivet_core::proxy::UPSTREAM_NAME_SETTING
                ))
            })?,
    };
    let name = ServerName::try_from(upstream.clone())
        .map_err(|e| unusable(format!("{upstream:?} is not a server name: {e}")))?;

    let config = rustls::ClientConfig::builder_with_provider(rivetos_tls::provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| unusable(format!("protocol versions: {e}")))?
        .with_root_certificates(roots)
        .with_client_auth_cert(chain, key)
        .map_err(|e| unusable(format!("{cert_file} and {key_file} do not go together: {e}")))?;
    Ok((Arc::new(config), name, upstream))
}

/// **Carry one request to the application and its answer back.**
///
/// `already` is whatever body arrived with the caller's head and `more` reads
/// the rest; `out` writes to the caller. The application's answer is copied to
/// `out` piece by piece as it arrives, and nothing here holds more than one
/// piece of it.
///
/// `Ok` carries the application's status, read off its first line, for the
/// journal — the answer itself has already been written.
///
/// # Errors
///
/// A [`Response`] to send instead, **only while nothing of the application's
/// answer has been written**: the application could not be reached, refused the
/// handshake, or closed before answering. Once its answer has started, a
/// failure is the caller's to notice — its length, and the ladder, are what
/// catch a short body — and this returns `Ok`.
pub fn relay(
    upstream: &Upstream,
    already: &[u8],
    more: &mut dyn FnMut(&mut [u8]) -> std::io::Result<usize>,
    out: &mut dyn FnMut(&[u8]) -> std::io::Result<()>,
) -> Result<u16, Response> {
    let gateway = |what: String| {
        refuse(
            502,
            "Bad Gateway",
            format!("{} could not be reached on loopback: {what}\n", upstream.service),
        )
    };
    let socket = std::net::TcpStream::connect((upstream.host.as_str(), upstream.port))
        .map_err(|e| gateway(format!("connecting to port {}: {e}", upstream.port)))?;
    let connection = rustls::ClientConnection::new(Arc::clone(&upstream.config), upstream.name.clone())
        .map_err(|e| gateway(format!("starting TLS: {e}")))?;
    let mut tls = rustls::StreamOwned::new(connection, socket);

    tls.write_all(upstream.head.as_bytes())
        .map_err(|e| gateway(format!("sending the request: {e}")))?;
    let mut left = upstream.body;
    // **A failed send is not yet the answer.** An application may refuse a
    // restore before it has read the body and close; its refusal is then on
    // the wire, and is what the caller should be shown.
    let mut send_failed: Option<String> = None;
    let first = &already[..already.len().min(usize::try_from(left).unwrap_or(usize::MAX))];
    if !first.is_empty() {
        if let Err(e) = tls.write_all(first) {
            send_failed = Some(format!("sending the body: {e}"));
        }
        left -= first.len() as u64;
    }
    let mut piece = vec![0u8; PIECE];
    while left > 0 && send_failed.is_none() {
        let want = usize::try_from(left.min(PIECE as u64)).unwrap_or(PIECE);
        let got = match more(&mut piece[..want]) {
            Ok(0) => {
                return Err(refuse(
                    400,
                    "Bad Request",
                    format!(
                        "the connection closed {left} byte(s) short of the body it declared. The \
                         application was sent a request it will refuse as incomplete, and \
                         nothing of its data is changed by one (ADR-051 §12).\n"
                    ),
                ));
            }
            Ok(n) => n,
            Err(e) => {
                return Err(refuse(
                    400,
                    "Bad Request",
                    format!("the body stopped arriving with {left} byte(s) to go: {e}\n"),
                ));
            }
        };
        if let Err(e) = tls.write_all(&piece[..got]) {
            send_failed = Some(format!("sending the body: {e}"));
        }
        left -= got as u64;
    }
    if send_failed.is_none()
        && let Err(e) = tls.flush()
    {
        send_failed = Some(format!("sending the request: {e}"));
    }

    // **The answer, verbatim.** The first piece is read before anything is
    // written, so an application that closed without answering is a refusal
    // composed here rather than an empty reply.
    let silent = |what: String| gateway(send_failed.clone().unwrap_or(what));
    let first = loop {
        match tls.read(&mut piece) {
            Ok(0) => return Err(silent("it closed the connection without answering".to_owned())),
            Ok(n) => break n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(silent(format!("reading its answer: {e}"))),
        }
    };
    let status = status_of(&piece[..first]);
    if out(&piece[..first]).is_err() {
        return Ok(status);
    }
    loop {
        match tls.read(&mut piece) {
            // A close_notify, or a peer that simply closed after its body: the
            // caller's length check is what tells a whole answer from a cut one.
            Ok(0) => break,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => {
                eprintln!(
                    "rivet-mcpd: {}'s answer stopped part-way: {e}",
                    upstream.service
                );
                break;
            }
            Ok(n) => {
                if let Err(e) = out(&piece[..n]) {
                    eprintln!(
                        "rivet-mcpd: {}'s answer could not be passed on: {e}",
                        upstream.service
                    );
                    break;
                }
            }
        }
    }
    Ok(status)
}

/// The status code on an HTTP response's first line, or `0` when it has none.
fn status_of(bytes: &[u8]) -> u16 {
    let line = bytes.split(|b| *b == b'\n').next().unwrap_or_default();
    std::str::from_utf8(line)
        .ok()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|code| code.trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "backups_tests.rs"]
mod tests;
