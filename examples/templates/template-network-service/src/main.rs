//! **A network service on a Rivet — the template.** `ADR-036` §6.2.
//!
//! A tiny notes API: `POST /notes` to add one, `GET /notes` to read them back,
//! persisted in SQLite. Deliberately basic. What is worth copying is not what it
//! does — it is the four things every Rivet service does, all of them in
//! [`main`] and none of them more than a few lines:
//!
//! 1. **It is handed its socket** and never binds one.
//! 2. **It is told where it may write** and never guesses.
//! 3. **It says things to the journal**, which is how anyone sees it.
//! 4. **It stops when asked**, and finishes what is in flight first.
//!
//! # There is no `unsafe` in this file
//!
//! Turning a handed descriptor into a listener needs `FromRawFd`, which is
//! `unsafe` wherever it happens — so `rivet-sdk` does it once, on the grounds
//! that the safety argument is the platform's and not an application author's.
//! That is the whole reason [`rivet_sdk::listen::Socket::into_tcp_listener`]
//! exists. An application should never write that block.
//!
//! # What would go wrong without a Rivet
//!
//! Run this on an ordinary Linux box and it refuses, naming the variable it
//! wanted. That is deliberate (`ADR-036` §5.2): a service that silently fell
//! back to binding `0.0.0.0:8080` and writing to the working directory would
//! *appear* to work, and would put a database somewhere nobody is looking.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use rivet_sdk::journal::{Journal, Value};
use serde::{Deserialize, Serialize};

/// The name this service is known by — its journal records, and the directory
/// the supervisor made for it.
///
/// It matches the `[package] name` in `Cargo.toml`, because that is what
/// `cargo rivet package` derives the service name from.
const SERVICE: &str = "template-network-service";

/// The application whose endpoint this one asks for, and the endpoint's name.
///
/// **Both halves are the manifest's, restated here.** `connects` names
/// `template-go-service:api`, and these two constants are how the code finds
/// what the manifest asked for. They have to agree, and a mismatch shows up as
/// `/upstream` reporting *not granted* on a machine where it plainly was —
/// which is why they are named rather than inlined.
const UPSTREAM_OWNER: &str = "template-go-service";
const UPSTREAM_NAME: &str = "api";

#[derive(Debug, Serialize)]
struct Note {
    id: i64,
    text: String,
}

#[derive(Debug, Deserialize)]
struct NewNote {
    text: String,
}

/// Everything the handlers share.
///
/// **One connection behind a mutex**, which is the honest shape for a LAN
/// service answering a few requests a minute. `rusqlite` is blocking, so a
/// busier service should reach for `spawn_blocking` or a pool — the note is
/// here rather than the machinery, because a template that hid this decision
/// would be teaching it by omission.
struct Api {
    db: tokio::sync::Mutex<rusqlite::Connection>,
    journal: Journal,
}

fn main() -> std::process::ExitCode {
    // **The journal first, before anything that can fail.** Under a supervisor
    // there is no console, so a failure from here on exists in the journal or
    // nowhere. `Journal` opens no socket until something is emitted, so this
    // costs nothing if the process dies immediately.
    let journal = Journal::for_service(SERVICE);

    match run(&journal) {
        Ok(()) => {
            journal.info("stopped");
            std::process::ExitCode::SUCCESS
        }
        Err(why) => {
            // Reported as a record, not just a non-zero exit: `rivet get log`
            // is where a person looks, and an exit code alone is a number.
            journal.emit(
                rivet_sdk::Severity::Error,
                "could not start",
                &[("why", Value::Str(why.clone()))],
            );
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(journal: &Journal) -> Result<(), String> {
    // ---- 1. The socket this service was handed ----------------------------
    //
    // `sockets()` returns them in the manifest's order. This service declares
    // exactly one, so it takes the first and refuses if it is not there —
    // rather than binding a port, which is what a Linux program would do and
    // what an application on a Rivet must never do.
    let mut handed = rivet_sdk::listen::sockets();
    if handed.is_empty() {
        return Err(format!(
            "no listening socket was handed over. This program declares a `listen` \
             socket in its manifest and expects a Rivet supervisor to open it and \
             pass the descriptor ({} was not set, or named another process).",
            rivet_sdk::listen::LISTEN_FDS_ENV
        ));
    }
    let socket = handed.remove(0);
    let name = socket.name.clone();

    // ---- 2. Where this service may write ----------------------------------
    let data = rivet_sdk::app::data().map_err(|why| why.to_string())?;
    let database = data.join("notes.db");

    // ---- 3. Open the database ---------------------------------------------
    let db = rusqlite::Connection::open(&database)
        .map_err(|why| format!("{}: {why}", database.display()))?;
    // WAL, which `sqlite-demo` proved on `/state` before this example existed.
    db.pragma_update(None, "journal_mode", "WAL")
        .map_err(|why| format!("could not enable WAL: {why}"))?;
    db.execute(
        "CREATE TABLE IF NOT EXISTS notes (
             id   INTEGER PRIMARY KEY AUTOINCREMENT,
             text TEXT NOT NULL
         )",
        [],
    )
    .map_err(|why| format!("could not create the schema: {why}"))?;

    journal.emit(
        rivet_sdk::Severity::Info,
        "starting",
        &[
            ("socket", Value::Str(name)),
            ("database", Value::Str(database.display().to_string())),
        ],
    );

    // ---- 4. Serve ---------------------------------------------------------
    //
    // The runtime is built here rather than by `#[tokio::main]` so that
    // everything above can fail *before* a runtime exists — a failure the
    // supervisor is told about, rather than one that happens inside an
    // executor nobody is watching.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|why| format!("could not build a runtime: {why}"))?;

    runtime.block_on(serve(socket, db, journal.clone()))
}

async fn serve(
    socket: rivet_sdk::listen::Socket,
    db: rusqlite::Connection,
    journal: Journal,
) -> Result<(), String> {
    // **The one conversion, and it is not `unsafe` here.** `into_tcp_listener`
    // consumes the `Socket` AND claims the descriptor, so it cannot be owned
    // twice — `None` is the SDK saying somebody in this process already has it.
    // For a service with one socket that is a defect in the service.
    let listener = socket
        .into_tcp_listener()
        .ok_or("the handed descriptor was already taken inside this process")?;
    listener
        .set_nonblocking(true)
        .map_err(|why| format!("the handed socket would not go non-blocking: {why}"))?;
    let listener = tokio::net::TcpListener::from_std(listener)
        .map_err(|why| format!("the handed socket would not join the runtime: {why}"))?;

    let state = Arc::new(Api {
        db: tokio::sync::Mutex::new(db),
        journal: journal.clone(),
    });

    let app = Router::new()
        .route("/", get(index))
        .route("/notes", get(list).post(add))
        .route("/health", post(health).get(health))
        .route("/upstream", get(upstream))
        .with_state(state);

    journal.info("listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(stopping(journal.clone()))
        .await
        .map_err(|why| format!("the server stopped badly: {why}"))
}

/// Resolves when the supervisor asks this service to stop.
///
/// `SIGTERM` is what a supervisor sends; Ctrl-C is what a person does when
/// running it by hand. **Both, because a template that only handled one would
/// teach the wrong lesson** — and neither is `SIGHUP`, which conventionally
/// means *reload*.
async fn stopping(journal: Journal) {
    let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(signal) => signal,
        Err(why) => {
            // A failed handler must not be mistaken for a signal. Logging and
            // parking is the safe read: the service keeps answering and the
            // supervisor can still stop the process.
            journal.emit(
                rivet_sdk::Severity::Error,
                "could not listen for SIGTERM; stop this process externally",
                &[("why", Value::Str(why.to_string()))],
            );
            std::future::pending::<()>().await;
            return;
        }
    };

    tokio::select! {
        _ = term.recv() => journal.info("SIGTERM; draining"),
        result = tokio::signal::ctrl_c() => match result {
            Ok(()) => journal.info("interrupted; draining"),
            Err(why) => journal.emit(
                rivet_sdk::Severity::Warn,
                "could not listen for Ctrl-C",
                &[("why", Value::Str(why.to_string()))],
            ),
        },
    }
}

/// What this is, in the one place someone will look first.
async fn index() -> (StatusCode, [(&'static str, &'static str); 1], String) {
    (
        StatusCode::OK,
        [("content-type", "text/plain; charset=utf-8")],
        concat!(
            "template-network-service - a Rivet network service template\n",
            "\n",
            "  GET  /notes    every note, newest last\n",
            "  POST /notes    {\"text\": \"...\"}\n",
            "  GET  /health   200 while the database answers\n",
            "\n",
            "It was handed its socket and told where to write.\n",
            "See docs/ADR-036-The-Rivet-SDK.md.\n"
        )
        .to_string(),
    )
}

/// **Whatever the application we `connects` to has to say.**
///
/// This is the other half of `ADR-015` §5.3, and the half a provider cannot
/// prove on its own: the socket file belongs to `template-go-service`, and this
/// process can open it only because the supervisor **chowned it to this
/// application's uid** on the strength of the `connects` line in the manifest.
///
/// # The path is told, never guessed
///
/// `RIVET_CONNECT` carries `owner:name=path`. Aura's renderer was the
/// cautionary case — it hardcoded `/run/aura/kiosk.sock`, which was not where a
/// Rivet puts it, until it was patched to read `RIVET_CONNECT` instead.
///
/// # The 503 branch is a backstop, not the ordinary path
///
/// This was written believing an ungranted `connects` left the service running
/// and this endpoint reporting 503. **It does not** — measured 2026-09-01,
/// `PLAN.md` §105.7: a `connects` the supervisor cannot satisfy stops the
/// service **starting**, because *"starting it anyway would produce a program
/// failing at a socket nobody bound"*.
///
/// So by the time this code runs, the endpoint has been granted. The branch
/// stays as a backstop for the one case the supervisor cannot cover — a
/// `RIVET_CONNECT` that is somehow absent — and because a handler that
/// `unwrap`ped a promise would be a worse thing to copy.
async fn upstream(State(api): State<Arc<Api>>) -> (StatusCode, String) {
    let Some(endpoint) = rivet_sdk::app::endpoint(UPSTREAM_OWNER, UPSTREAM_NAME) else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "{UPSTREAM_OWNER}:{UPSTREAM_NAME} was not granted to this application. \
                 The manifest asks for it; the operator decides. Nothing is wrong with \
                 this service.\n"
            ),
        );
    };

    match read_endpoint(&endpoint.path).await {
        Ok(said) => (StatusCode::OK, said),
        Err(why) => {
            api.journal.emit(
                rivet_sdk::Severity::Warn,
                "the granted endpoint could not be read",
                &[
                    ("path", Value::Str(endpoint.path.display().to_string())),
                    ("why", Value::Str(why.clone())),
                ],
            );
            (
                StatusCode::BAD_GATEWAY,
                format!("{} was granted but did not answer: {why}\n", endpoint.path.display()),
            )
        }
    }
}

/// Connect, read what the provider says, close.
///
/// The provider writes one JSON object and closes, so reading to end is the
/// whole protocol.
async fn read_endpoint(path: &std::path::Path) -> Result<String, String> {
    use tokio::io::AsyncReadExt as _;

    let mut socket = tokio::net::UnixStream::connect(path)
        .await
        .map_err(|why| why.to_string())?;
    let mut said = String::new();
    socket
        .read_to_string(&mut said)
        .await
        .map_err(|why| why.to_string())?;
    Ok(said)
}

/// 200 while the database answers, 503 when it does not.
///
/// **It asks the database rather than returning a constant.** A health check
/// that cannot fail is a health check that tells you nothing.
async fn health(State(api): State<Arc<Api>>) -> StatusCode {
    let db = api.db.lock().await;
    match db.query_row("SELECT 1", [], |row| row.get::<_, i64>(0)) {
        Ok(_) => StatusCode::OK,
        Err(why) => {
            api.journal.emit(
                rivet_sdk::Severity::Error,
                "health check failed",
                &[("why", Value::Str(why.to_string()))],
            );
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

async fn list(State(api): State<Arc<Api>>) -> Result<Json<Vec<Note>>, StatusCode> {
    let db = api.db.lock().await;
    let mut statement = db
        .prepare("SELECT id, text FROM notes ORDER BY id")
        .map_err(|why| failed(&api, "could not prepare the query", &why))?;
    let rows = statement
        .query_map([], |row| {
            Ok(Note {
                id: row.get(0)?,
                text: row.get(1)?,
            })
        })
        .map_err(|why| failed(&api, "could not run the query", &why))?;

    let mut notes = Vec::new();
    for row in rows {
        notes.push(row.map_err(|why| failed(&api, "could not read a row", &why))?);
    }
    Ok(Json(notes))
}

async fn add(
    State(api): State<Arc<Api>>,
    Json(new): Json<NewNote>,
) -> Result<(StatusCode, Json<Note>), StatusCode> {
    // Refused here rather than stored and puzzled over later. An empty note is
    // not a smaller note; it is a request that meant nothing.
    if new.text.trim().is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let db = api.db.lock().await;
    db.execute("INSERT INTO notes (text) VALUES (?1)", [&new.text])
        .map_err(|why| failed(&api, "could not insert", &why))?;
    let id = db.last_insert_rowid();

    api.journal.emit(
        rivet_sdk::Severity::Info,
        "note added",
        &[("id", Value::Int(id))],
    );

    Ok((StatusCode::CREATED, Json(Note { id, text: new.text })))
}

/// Record why, then answer 500.
///
/// **The reason goes to the journal and not to the caller.** A client gets a
/// status; a maintainer gets the sentence, in the place they are already
/// looking.
fn failed(api: &Api, what: &str, why: &rusqlite::Error) -> StatusCode {
    api.journal.emit(
        rivet_sdk::Severity::Error,
        what,
        &[("why", Value::Str(why.to_string()))],
    );
    StatusCode::INTERNAL_SERVER_ERROR
}
