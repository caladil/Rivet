//! **Do browsers accept a passkey on a `.local` name?** See `Cargo.toml`.
//!
//! `GET /` serves `page.html`; `POST /result` journals what the page reports.
//! Both are also answered under `/passkey/`, so the page works whether or not
//! the proxy strips its prefix. Served over TLS with the machine's own
//! certificate, because that is how `rivet-proxyd` reaches a backend.

use std::sync::Arc;

use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::Router;
use rivet_sdk::journal::{Journal, Value};

const SERVICE: &str = "passkey-probe";
const PAGE: &str = include_str!("page.html");

fn main() -> std::process::ExitCode {
    let journal = Journal::for_service(SERVICE);
    match run(&journal) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(why) => {
            journal.emit(
                rivet_sdk::Severity::Error,
                "could not start",
                &[("why", Value::Str(why))],
            );
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(journal: &Journal) -> Result<(), String> {
    let mut handed = rivet_sdk::listen::sockets();
    if handed.is_empty() {
        return Err("no listening socket was handed over".to_owned());
    }
    let socket = handed.remove(0);
    let tls = rivet_sdk::serve::Tls::handed("tls-cert", "tls-key")
        .map_err(|why| why.to_string())?
        .server_config()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|why| format!("could not build a runtime: {why}"))?;
    runtime.block_on(serve(socket, tls, journal.clone()))
}

async fn serve(
    socket: rivet_sdk::listen::Socket,
    tls: rustls::ServerConfig,
    journal: Journal,
) -> Result<(), String> {
    let listener = socket
        .into_tcp_listener()
        .ok_or("the handed descriptor was already taken inside this process")?;
    listener
        .set_nonblocking(true)
        .map_err(|why| format!("the handed socket would not go non-blocking: {why}"))?;
    let listener = tokio::net::TcpListener::from_std(listener)
        .map_err(|why| format!("the handed socket would not join the runtime: {why}"))?;

    let recorder = journal.clone();
    let record = move |headers: HeaderMap, body: String| {
        let journal = recorder.clone();
        async move { result(&journal, &headers, &body) }
    };
    let app = Router::new()
        .route("/", get(page))
        .route("/passkey/", get(page))
        .route("/result", post(record.clone()))
        .route("/passkey/result", post(record));

    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls));
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|why| format!("could not listen for SIGTERM: {why}"))?;

    journal.info("listening");
    loop {
        let (tcp, _) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(pair) => pair,
                Err(_) => continue,
            },
            _ = term.recv() => return Ok(()),
        };
        let acceptor = acceptor.clone();
        let service = hyper_util::service::TowerToHyperService::new(app.clone());
        tokio::spawn(async move {
            // A failed handshake or a dropped connection is one caller's
            // problem and not the server's; there is nothing to report upward.
            if let Ok(stream) = acceptor.accept(tcp).await {
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                    .await;
            }
        });
    }
}

async fn page() -> (StatusCode, [(&'static str, &'static str); 2], &'static str) {
    (
        StatusCode::OK,
        [
            ("content-type", "text/html; charset=utf-8"),
            ("cache-control", "no-store"),
        ],
        PAGE,
    )
}

/// The largest outcome journalled. A ceremony's capture — `clientDataJSON`,
/// the attestation object or authenticator data, and a signature, in base64url
/// — is a few kilobytes; the journal takes a record up to 16 KiB.
const MAX_OUTCOME: usize = 12 * 1024;

/// One outcome from one browser, journalled as the page sent it.
///
/// The body is the page's own JSON, kept whole: the question is what each
/// browser said, and a record that paraphrased it would be a guess.
///
/// **Its `capture` is the source of `rivet-webauthn`'s "captured from a real
/// browser" vectors** (`ADR-063` §4): every byte a relying party verifies, from
/// a real authenticator, with the challenge and RP ID that produced them. Read
/// it back with `rivet get log latest all` and filter `passkey-probe`.
fn result(journal: &Journal, headers: &HeaderMap, body: &str) -> StatusCode {
    if body.len() > MAX_OUTCOME || serde_json::from_str::<serde_json::Value>(body).is_err() {
        return StatusCode::BAD_REQUEST;
    }
    let agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    journal.emit(
        rivet_sdk::Severity::Info,
        "passkey outcome",
        &[
            ("outcome", Value::Str(body.to_owned())),
            ("agent", Value::Str(agent)),
        ],
    );
    StatusCode::NO_CONTENT
}
