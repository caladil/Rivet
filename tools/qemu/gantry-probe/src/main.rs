//! **A client of `rivet-fleetd`'s `gantry` socket, for one proof** — `ADR-061`
//! §6 and §7, `SPEC-06`.
//!
//! See `Cargo.toml` for what this is and is not. At every start it makes one
//! **round** over the socket and writes what it was told into its data
//! directory, where `tools/qemu/adr-061-gantry-proof.sh` reads it with
//! `get file`:
//!
//! * `round` — the round's number, written first;
//! * `last-<op>.json` — each reply line, verbatim, for `fleet`, `artifacts`,
//!   `enrol` and `phones`;
//! * `last-round.txt` — which socket was dialled, and why, and what each
//!   request's write and read returned;
//! * `done` — the round's number again, written LAST, so a reader that sees it
//!   change knows every other file of that round is complete.
//!
//! `enrol` is sent once per data directory (`enrolled` records that it
//! succeeded), so a phone revoked from outside stays revoked across a restart.
//!
//! Then, when a TCP socket was handed over, it serves two routes behind
//! `rivet-proxyd` with the proxy's prefix removed:
//!
//! * `GET`/`HEAD`/`POST /install…` — a page, `200`, for every method: a `405`
//!   on port 80 can therefore only have come from the proxy;
//! * `GET /api/echo` — `200`, whose body is `authorization=` and the
//!   `Authorization` header exactly as it arrived, or `(none)`.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// The provider and its endpoint: `connects = ["rivet-fleetd:gantry"]`.
const PROVIDER: &str = "rivet-fleetd";
const ENDPOINT: &str = "gantry";

/// Where the provider's endpoint is bound — `rivet_core::service::
/// provided_socket_path` for `rivet-fleetd` and `gantry`. Dialled only by a
/// probe the supervisor granted nothing, to record what the socket's own mode
/// does to a caller that was never granted it.
const PROVIDER_PATH: &str = "/run/rivet/listen/rivet-fleetd/gantry";

/// The label the hand-composed phone is enrolled under.
const LABEL: &str = "Hand-composed probe phone";

/// How long one reply may take.
const PATIENCE: Duration = Duration::from_secs(20);

/// A request head larger than this is refused.
const MAX_HEAD: usize = 32 * 1024;

/// **HAND-COMPOSED**: the DER SubjectPublicKeyInfo of an uncompressed EC P-256
/// point whose X is 32 bytes of `0x11` and whose Y is 32 bytes of `0x22`. The
/// prefix is `id-ecPublicKey`, `prime256v1` and a 66-byte BIT STRING. It is 91
/// bytes, the only shape `SPEC-06` §6's structural check admits; the point is
/// not on the curve and no phone holds its private half.
fn hand_composed_spki() -> Vec<u8> {
    let mut der = vec![
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
        0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00, 0x04,
    ];
    der.extend_from_slice(&[0x11; 32]);
    der.extend_from_slice(&[0x22; 32]);
    der
}

/// Standard base64, padded — what `SPEC-06` §5 asks an enroll's `public_key` to
/// be.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (index, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            if index <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> shift) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn main() -> std::process::ExitCode {
    let Ok(data) = rivet_sdk::app::data() else {
        eprintln!("gantry-probe: RIVET_APP_DATA is not set; this was not started by a Rivet");
        return std::process::ExitCode::FAILURE;
    };
    // `/state/app/<name>/data`: the name this copy was installed as.
    let name = data.parent().and_then(Path::file_name).map_or_else(
        || "gantry-probe".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    );
    let journal = rivet_sdk::journal::Journal::for_service(&name);
    match run(&data, &journal) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(why) => {
            journal.error(&why);
            eprintln!("{name}: {why}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(data: &Path, journal: &rivet_sdk::journal::Journal) -> Result<(), String> {
    let number = round(data)?;
    journal.info(&format!(
        "gantry-probe: round {number} finished; see last-round.txt"
    ));

    let listeners: Vec<std::net::TcpListener> = rivet_sdk::listen::sockets()
        .into_iter()
        .filter(rivet_sdk::listen::Socket::is_tcp)
        .filter_map(rivet_sdk::listen::Socket::into_tcp_listener)
        .collect();
    if listeners.is_empty() {
        // Nothing to serve: stay up, so the supervisor has nothing to restart.
        loop {
            std::thread::park();
        }
    }
    serve(&listeners, journal)
}

/// The number in `data/<file>`, or zero when there is none.
fn number_in(data: &Path, file: &str) -> u64 {
    std::fs::read_to_string(data.join(file))
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

fn write(data: &Path, file: &str, contents: &str) -> Result<(), String> {
    let path = data.join(file);
    std::fs::write(&path, contents).map_err(|e| format!("{}: {e}", path.display()))
}

/// One round over the socket. Returns its number.
fn round(data: &Path) -> Result<u64, String> {
    let number = number_in(data, "round") + 1;
    write(data, "round", &format!("{number}\n"))?;

    let (path, how) = match rivet_sdk::app::endpoint(PROVIDER, ENDPOINT) {
        Some(granted) => (granted.path, "granted by the supervisor"),
        None => (
            PathBuf::from(PROVIDER_PATH),
            "NOT granted, so the provider's socket path was dialled directly",
        ),
    };
    let mut summary = format!(
        "round {number}: {PROVIDER}:{ENDPOINT} {how}: {}\n",
        path.display()
    );

    match UnixStream::connect(&path) {
        Err(e) => summary.push_str(&format!("connect: refused by the kernel: {e}\n")),
        Ok(stream) => {
            summary.push_str("connect: accepted\n");
            converse(data, stream, &mut summary)?;
        }
    }
    write(data, "last-round.txt", &summary)?;
    write(data, "done", &format!("{number}\n"))?;
    Ok(number)
}

/// The requests of one round, and each reply written where the proof reads it.
fn converse(data: &Path, stream: UnixStream, summary: &mut String) -> Result<(), String> {
    stream
        .set_read_timeout(Some(PATIENCE))
        .and_then(|()| stream.set_write_timeout(Some(PATIENCE)))
        .map_err(|e| e.to_string())?;
    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);

    let mut requests: Vec<(&str, String)> = vec![
        ("fleet", r#"{"v":1,"op":"fleet"}"#.to_owned()),
        ("artifacts", r#"{"v":1,"op":"artifacts"}"#.to_owned()),
    ];
    let enrolled = data.join("enrolled");
    if !enrolled.exists() {
        requests.push((
            "enrol",
            format!(
                r#"{{"v":1,"op":"enrol","public_key":"{}","label":"{LABEL}"}}"#,
                base64(&hand_composed_spki())
            ),
        ));
    }
    requests.push(("phones", r#"{"v":1,"op":"phones"}"#.to_owned()));

    for (op, request) in requests {
        let wrote = writer
            .write_all(format!("{request}\n").as_bytes())
            .and_then(|()| writer.flush());
        let mut reply = String::new();
        let read = reader.read_line(&mut reply);
        summary.push_str(&format!(
            "{op}: write {}, read {}\n",
            wrote
                .as_ref()
                .map_or_else(ToString::to_string, |()| "ok".to_owned()),
            read.as_ref()
                .map_or_else(ToString::to_string, |n| format!("{n} bytes"))
        ));
        write(data, &format!("last-{op}.json"), &reply)?;
        if op == "enrol" && reply.starts_with(r#"{"ok":true"#) {
            write(data, "enrolled", "yes\n")?;
        }
        // The connection is over: closed, or refused (`SPEC-06` §2 closes it).
        if reply.is_empty() || reply.starts_with(r#"{"ok":false,"error":"refused""#) {
            summary.push_str("the provider closed the connection\n");
            break;
        }
    }
    Ok(())
}

/// Serve every handed TCP socket, once the TLS files are there.
fn serve(
    listeners: &[std::net::TcpListener],
    journal: &rivet_sdk::journal::Journal,
) -> Result<(), String> {
    let mut said = false;
    let config = loop {
        // **Required, never merely requested** — `ADR-046` §4's instruction to
        // a backend. The caller is `rivet-proxyd`, presenting its certificate.
        let built = rivet_sdk::serve::Tls::handed("tls-cert", "tls-key")
            .map_err(|e| e.to_string())
            .and_then(|tls| {
                tls.requiring_a_caller_certificate_from("tls-root")
                    .server_config()
            });
        match built {
            Ok(config) => break Arc::new(config),
            Err(why) => {
                if !said {
                    journal.info(&format!("gantry-probe: waiting for the TLS files: {why}"));
                    said = true;
                }
                std::thread::sleep(Duration::from_secs(3));
            }
        }
    };
    journal.info(&format!(
        "gantry-probe: serving on {} TCP socket(s)",
        listeners.len()
    ));

    std::thread::scope(|scope| {
        for listener in listeners {
            let config = &config;
            scope.spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { continue };
                    let Ok(connection) = rustls::ServerConnection::new(config.clone()) else {
                        continue;
                    };
                    let mut tls = rustls::StreamOwned::new(connection, stream);
                    if let Err(why) = answer(&mut tls) {
                        journal.warn(&format!("a connection ended without an answer: {why}"));
                    }
                }
            });
        }
    });
    Ok(())
}

type Tls = rustls::StreamOwned<rustls::ServerConnection, std::net::TcpStream>;

fn answer(tls: &mut Tls) -> Result<(), String> {
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
    let header = |wanted: &str| {
        head.lines().skip(1).find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case(wanted)
                .then(|| value.trim().to_owned())
        })
    };
    // A body is read and discarded, so the connection stays in step.
    let length: usize = header("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if length > MAX_HEAD {
        return respond(tls, "413 Payload Too Large", b"too large", false);
    }
    let mut body = vec![0u8; length];
    tls.read_exact(&mut body).map_err(|e| e.to_string())?;

    let mut parts = request_line.split(' ');
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    match (method, target) {
        ("GET" | "HEAD" | "POST", path) if path.starts_with("/install") => respond(
            tls,
            "200 OK",
            format!("gantry-probe install page, {method} {path}\n").as_bytes(),
            method == "HEAD",
        ),
        ("GET", "/api/echo") => respond(
            tls,
            "200 OK",
            format!(
                "authorization={}\n",
                header("authorization").unwrap_or_else(|| "(none)".to_owned())
            )
            .as_bytes(),
            false,
        ),
        _ => respond(tls, "404 Not Found", b"no such route\n", method == "HEAD"),
    }
}

fn respond(tls: &mut Tls, status: &str, body: &[u8], head_only: bool) -> Result<(), String> {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    tls.write_all(head.as_bytes()).map_err(|e| e.to_string())?;
    if !head_only {
        tls.write_all(body).map_err(|e| e.to_string())?;
    }
    tls.conn.send_close_notify();
    tls.flush().map_err(|e| e.to_string())
}
