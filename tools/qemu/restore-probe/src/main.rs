//! **A backup-and-restore application, for one proof** — `ADR-051` §12.
//!
//! See `Cargo.toml` for what this is and is not. The routes, as `rivet-proxyd`
//! forwards them with the application's own prefix removed:
//!
//! * `GET  /api/v1/backup` — the data file, whole, with a `Content-Length`.
//! * `POST /api/v1/restore?confirm=true` — the contract's restore: refuse
//!   without the query (409), refuse a body that is not a whole file of the
//!   declared format (400) having changed nothing, otherwise write it beside
//!   the live file, sync, rename it into place, and answer 200.
//! * `POST /api/v1/mutate` — replace the data with different bytes, so the
//!   proof can show a restore undoing a change.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The data file, in the application's own data directory.
const DATA: &str = "probe.sqlite";
/// A request head larger than this is refused.
const MAX_HEAD: usize = 32 * 1024;
/// A body larger than this is refused — the fixture's bound, far above what
/// the proof sends.
const MAX_BODY: usize = 64 * 1024 * 1024;

fn main() -> std::process::ExitCode {
    let journal = rivet_sdk::journal::Journal::for_service("restore-probe");
    match serve(&journal) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(why) => {
            journal.error(&why);
            eprintln!("restore-probe: {why}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// A file built to SQLite's header arithmetic: `pages` pages of 1024 bytes,
/// filled with `marker` past the header so two versions differ in every page.
fn sqlite_bytes(pages: u32, marker: u8) -> Vec<u8> {
    let mut bytes = vec![marker; 1024 * pages as usize];
    bytes[..16].copy_from_slice(b"SQLite format 3\0");
    bytes[16..18].copy_from_slice(&1024u16.to_be_bytes());
    bytes[18..28].fill(0);
    bytes[28..32].copy_from_slice(&pages.to_be_bytes());
    bytes
}

/// Whether `body` is a whole file of the format this application declares —
/// the magic, and the header's page size times page count equal to the length.
fn is_whole_sqlite(body: &[u8]) -> bool {
    if body.len() < 32 || !body.starts_with(b"SQLite format 3\0") {
        return false;
    }
    let raw = u32::from(u16::from_be_bytes([body[16], body[17]]));
    let page_size = if raw == 1 { 65_536 } else { raw };
    let pages = u32::from_be_bytes([body[28], body[29], body[30], body[31]]);
    pages != 0 && u64::from(page_size) * u64::from(pages) == body.len() as u64
}

/// Replace the data file as one step: write beside it, sync, rename.
fn replace(data: &Path, bytes: &[u8]) -> Result<(), String> {
    let live = data.join(DATA);
    let staged = data.join(format!("{DATA}.restoring"));
    let mut file = std::fs::File::create(&staged)
        .map_err(|e| format!("{}: {e}", staged.display()))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("{}: {e}", staged.display()))?;
    drop(file);
    std::fs::rename(&staged, &live).map_err(|e| format!("rename into {}: {e}", live.display()))
}

fn serve(journal: &rivet_sdk::journal::Journal) -> Result<(), String> {
    let data: PathBuf = rivet_sdk::app::data()
        .map_err(|_| String::from("RIVET_APP_DATA is not set; this was not started by a Rivet"))?;
    if !data.join(DATA).exists() {
        replace(&data, &sqlite_bytes(2, 0x11))?;
        journal.info("no data yet: wrote the first version, 2048 bytes");
    }

    // **Required, never merely requested** — `ADR-046` §4's instruction to a
    // backend. The caller is `rivet-proxyd`, presenting its certificate.
    let config = rivet_sdk::serve::Tls::handed("tls-cert", "tls-key")
        .map_err(|e| e.to_string())?
        .requiring_a_caller_certificate_from("tls-root")
        .server_config()?;
    let config = Arc::new(config);

    // **Every TCP socket handed over, whatever its port**, each on its own
    // thread and answering the same routes: `adr-051-restore-proof.sh`
    // installs this binary twice on two ports, and
    // `adr-046-backend-port-proof.sh` declares a routed port and a `:direct`
    // one (`ADR-046` §7) and asks both from outside the guest.
    let listeners: Vec<std::net::TcpListener> = rivet_sdk::listen::sockets()
        .into_iter()
        .filter(rivet_sdk::listen::Socket::is_tcp)
        .filter_map(rivet_sdk::listen::Socket::into_tcp_listener)
        .collect();
    if listeners.is_empty() {
        return Err(String::from("no TCP socket was handed over"));
    }
    journal.info(&format!(
        "serving the backup and restore routes on {} TCP socket(s)",
        listeners.len()
    ));

    std::thread::scope(|scope| {
        for listener in &listeners {
            let (config, data) = (&config, &data);
            scope.spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { continue };
                    let Ok(connection) = rustls::ServerConnection::new(config.clone()) else {
                        continue;
                    };
                    let mut tls = rustls::StreamOwned::new(connection, stream);
                    if let Err(why) = answer(&mut tls, data, journal) {
                        journal.warn(&format!("a connection ended without an answer: {why}"));
                    }
                }
            });
        }
    });
    Ok(())
}

type Tls = rustls::StreamOwned<rustls::ServerConnection, std::net::TcpStream>;

fn answer(tls: &mut Tls, data: &Path, journal: &rivet_sdk::journal::Journal) -> Result<(), String> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= MAX_HEAD {
            return Err("request head too large".to_string());
        }
        let read = tls.read(&mut byte).map_err(|e| e.to_string())?;
        if read == 0 {
            return Err("closed before a request".to_string());
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).to_string();
    let request_line = head.lines().next().unwrap_or_default().to_string();
    let length = match content_length(&head) {
        Ok(length) => length,
        Err(why) => return respond(tls, "400 Bad Request", "text/plain", why.as_bytes()),
    };
    if length > MAX_BODY {
        return respond(tls, "413 Payload Too Large", "text/plain", b"too large");
    }
    let mut body = vec![0u8; length];
    tls.read_exact(&mut body).map_err(|e| e.to_string())?;

    let mut parts = request_line.split(' ');
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    match (method, target) {
        ("GET", "/api/v1/backup") => {
            let bytes = std::fs::read(data.join(DATA)).map_err(|e| e.to_string())?;
            respond(tls, "200 OK", "application/vnd.sqlite3", &bytes)
        }
        ("POST", "/api/v1/mutate") => {
            replace(data, &sqlite_bytes(3, 0x22))?;
            journal.info("mutated: the data is now the second version, 3072 bytes");
            respond(tls, "200 OK", "application/json", b"{\"mutated\":3072}")
        }
        ("POST", path) if path.starts_with("/api/v1/restore") => {
            if path != "/api/v1/restore?confirm=true" {
                return respond(
                    tls,
                    "409 Conflict",
                    "application/json",
                    b"{\"error\":\"a restore replaces this application's data; send ?confirm=true\"}",
                );
            }
            // **Checked whole, before anything changes** — the contract.
            if !is_whole_sqlite(&body) {
                journal.warn("refused a restore: the body is not a whole SQLite-format file");
                return respond(
                    tls,
                    "400 Bad Request",
                    "application/json",
                    b"{\"error\":\"not a whole SQLite-format file; nothing was changed\"}",
                );
            }
            replace(data, &body)?;
            journal.info(&format!("restored {} bytes", body.len()));
            respond(
                tls,
                "200 OK",
                "application/json",
                format!("{{\"restored\":{}}}", body.len()).as_bytes(),
            )
        }
        _ => respond(tls, "404 Not Found", "text/plain", b"no such route"),
    }
}

/// A request's `Content-Length`, `0` when it states none.
///
/// **A repeated one is refused whether or not the values agree**, as every HTTP
/// parser in the tree refuses one: two readers of the same bytes that honour
/// different fields disagree about where the body ends.
fn content_length(head: &str) -> Result<usize, String> {
    let values: Vec<&str> = head
        .lines()
        .skip(1)
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim().eq_ignore_ascii_case("content-length").then(|| value.trim())
        })
        .collect();
    match values.as_slice() {
        [] => Ok(0),
        [one] => one.parse().map_err(|_| format!("Content-Length {one:?} is not a number")),
        many => Err(format!("{} Content-Length headers in one request", many.len())),
    }
}

fn respond(tls: &mut Tls, status: &str, content_type: &str, body: &[u8]) -> Result<(), String> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    tls.write_all(head.as_bytes())
        .and_then(|()| tls.write_all(body))
        .map_err(|e| e.to_string())?;
    tls.conn.send_close_notify();
    tls.flush().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::content_length;

    #[test]
    fn a_stated_length_is_read_and_none_is_zero() {
        assert_eq!(content_length("POST /x HTTP/1.1\r\nContent-Length: 12\r\n\r\n"), Ok(12));
        assert_eq!(content_length("GET /x HTTP/1.1\r\n\r\n"), Ok(0));
    }

    /// **A repeated `Content-Length` is refused, agreeing or not**: two readers
    /// of the same bytes that honour different fields disagree about where the
    /// body ends.
    #[test]
    fn a_repeated_content_length_is_refused() {
        for second in ["12", "3"] {
            let head = format!("POST /x HTTP/1.1\r\nContent-Length: 12\r\ncontent-length: {second}\r\n\r\n");
            let why = content_length(&head).expect_err("a repeated length");
            assert!(why.contains("Content-Length"), "{why}");
        }
    }
}
