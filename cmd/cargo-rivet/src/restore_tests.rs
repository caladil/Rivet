//! `cargo rivet restore`, against a fake machine and a fake control plane
//! behind `crate::backup::ReachesApplications`. The relay on the machine is
//! tested against a real TLS application in `cmd/rivet-mcpd`, and the transport
//! end to end by `crate::backup`'s control-plane test.
//!
//! Every fixture here is hand-composed: the table text is what
//! `rivet_core::backup::render` writes (asserted, not assumed) and the SQLite
//! files are headers built to the format's own arithmetic. None of it is a
//! capture from a real application.

// A test asserts; `unwrap`, `expect` and `panic!` ARE the assertion.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use rivet_core::backup::Wanted;
use rivet_core::package::application::{BackupFormat, BackupSurface};

use super::*;

/// A fresh directory per call, told apart by a counter rather than the clock.
///
/// Tests run on parallel threads, and three of them build an `authority()`
/// under the same label: two calls can read the same timestamp, and
/// `crate::scratch::dir` empties a name that is asked for twice, so one test
/// would lose its certificates while it was using them and fail its handshake
/// with `BadSignature`.
fn scratch(label: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    crate::scratch::dir(&format!("restore-{label}-{n}"))
}

/// A whole SQLite file by the header's own arithmetic: `pages` pages of 1024
/// bytes, the page count at 28–31, and `marker` written after the header so
/// two databases can be told apart.
fn sqlite(pages: u32, marker: u8) -> Vec<u8> {
    let mut bytes = vec![marker; 1024 * pages as usize];
    bytes[..16].copy_from_slice(b"SQLite format 3\0");
    bytes[16..18].copy_from_slice(&1024u16.to_be_bytes());
    bytes[18..28].fill(0);
    bytes[28..32].copy_from_slice(&pages.to_be_bytes());
    bytes
}

/// The machine's table as the supervisor writes it: TempMon accepting a
/// restore, a notes application offering a backup and no restore, Reliquary a feed.
fn machine_table() -> String {
    let artifact = |service: &str, route: &str, format, restore: Option<&str>| Wanted {
        service: service.to_string(),
        route: route.to_string(),
        surface: BackupSurface::Artifact {
            path: "api/v1/backup".to_string(),
            format,
            restore: restore.map(ToString::to_string),
        },
    };
    let table = rivet_core::backup::render(&rivet_core::backup::build(&[
        artifact("tempmon-rivet", "tempmon", BackupFormat::Sqlite, Some("api/v1/restore")),
        artifact("notes-rivet", "notes", BackupFormat::Json, None),
        artifact("ledger-rivet", "ledger", BackupFormat::Csv, Some("import")),
        Wanted {
            service: "reliquary-rivet".to_string(),
            route: "reliquary".to_string(),
            surface: BackupSurface::Feed {
                path: "api/v1/blobs".to_string(),
            },
        },
    ]));
    assert!(table.contains("tempmon-rivet tempmon restore api/v1/restore\n"), "{table}");
    table
}

/// A machine that answers `get file /run/rivet/backups` and nothing else.
struct FakeMachine {
    table: Option<String>,
    calls: Cell<usize>,
}

impl FakeMachine {
    fn with(table: &str) -> Self {
        Self {
            table: Some(table.to_string()),
            calls: Cell::new(0),
        }
    }
}

impl CallsTools for FakeMachine {
    fn call_tool(&self, tool: &str, arguments: &str) -> Result<String, String> {
        self.calls.set(self.calls.get() + 1);
        assert_eq!(tool, "rivet_get", "restore reads and never applies: {arguments}");
        assert!(arguments.contains(rivet_core::backup::PATH), "{arguments}");
        let path = rivet_core::backup::PATH;
        Ok(match &self.table {
            Some(text) => format!(
                r#"{{"subject":"file","path":"{path}","present":true,"content":"{}","content_encoding":"text"}}"#,
                text.replace('\n', "\\n")
            ),
            None => format!(r#"{{"subject":"file","path":"{path}","present":false}}"#),
        })
    }
}

/// What the fake control plane received — the service and the body — and the
/// answer it relays.
struct FakeApp {
    answer: Result<Relayed, String>,
    seen: RefCell<Vec<(String, Vec<u8>)>>,
}

fn relayed(status: u16, content_type: Option<&str>, body: &[u8], extra: &[(&str, &str)]) -> Relayed {
    let mut headers: Vec<(String, String)> = content_type
        .map(|t| vec![("content-type".to_string(), t.to_string())])
        .unwrap_or_default();
    headers.extend(extra.iter().map(|(n, v)| ((*n).to_ascii_lowercase(), (*v).to_string())));
    Relayed {
        status,
        headers,
        body: body.to_vec(),
        truncated: false,
    }
}

impl FakeApp {
    fn answering(status: u16, body: &str) -> Self {
        Self {
            answer: Ok(relayed(status, Some("application/json"), body.as_bytes(), &[])),
            seen: RefCell::new(Vec::new()),
        }
    }
}

impl ReachesApplications for FakeApp {
    fn fetch(&self, _: &str) -> Result<Relayed, String> {
        panic!("restore never fetches");
    }

    fn restore(&self, service: &str, body: &[u8], _: u64) -> Result<Relayed, String> {
        self.seen.borrow_mut().push((service.to_string(), body.to_vec()));
        self.answer.clone()
    }
}

/// `--service`/`--from`/`confirm` over a fixed endpoint.
fn args_for(service: &str, from: &Path, confirm: bool) -> RestoreArgs {
    let mut words: Vec<String> = [
        "--host",
        "rivet-01.local:7777",
        "--identity",
        "./id",
        "--pin",
        "ab",
        "--service",
        service,
        "--from",
    ]
    .iter()
    .map(|w| (*w).to_string())
    .collect();
    words.push(from.display().to_string());
    if confirm {
        words.push("confirm".to_string());
    }
    parse(&words).expect("valid arguments")
}

/// Run the verb with `app` behind the seam; `opened` says whether anything was
/// sent to it.
fn run(
    args: &RestoreArgs,
    machine: &FakeMachine,
    app: &'static FakeApp,
    opened: &Cell<bool>,
) -> Result<String, String> {
    let result = restore_via(args, machine, app, "rivet-01.local:7777");
    opened.set(!app.seen.borrow().is_empty());
    result
}

fn leak(app: FakeApp) -> &'static FakeApp {
    Box::leak(Box::new(app))
}

/// A backup file where `cargo rivet backup` would have put it.
fn stored(dir: &Path, service: &str, name: &str, bytes: &[u8]) -> PathBuf {
    let at = dir.join(service);
    std::fs::create_dir_all(&at).expect("a directory");
    let file = at.join(name);
    std::fs::write(&file, bytes).expect("write");
    file
}

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

#[test]
fn service_and_from_are_required_and_service_is_given_once() {
    let base = ["--host", "pi:7777", "--identity", "./id", "--pin", "ab"];
    let words = |extra: &[&str]| -> Vec<String> {
        base.iter().chain(extra).map(|w| (*w).to_string()).collect()
    };
    let refused = parse(&words(&["--from", "x.db"])).expect_err("no --service");
    assert!(refused.contains("--service is required"), "{refused}");
    let refused = parse(&words(&["--service", "tempmon-rivet"])).expect_err("no --from");
    assert!(refused.contains("--from is required"), "{refused}");
    let refused = parse(&words(&[
        "--service", "a", "--service", "b", "--from", "x.db",
    ]))
    .expect_err("twice");
    assert!(refused.contains("given twice"), "{refused}");
    let refused = parse(&words(&["--service", "a", "--from", "x", "--into", "d"]))
        .expect_err("not an option");
    assert!(refused.contains("--into"), "{refused}");
    // **The flags that reached the proxy are refused by name** (R-234).
    for flag in ["--authority", "--name", "--port"] {
        let refused = parse(&words(&["--service", "a", "--from", "x", flag, "v"]))
            .expect_err("retired");
        assert!(refused.contains(flag), "{refused}");
        assert!(refused.contains("control plane"), "{refused}");
    }
    let refused = parse(&[
        "--host", "pi:7777", "--identity", "./id", "--service", "a", "--from", "x",
    ]
    .map(ToString::to_string))
    .expect_err("no pin");
    assert!(refused.contains("--pin"), "{refused}");
}

#[test]
fn the_usage_says_nothing_is_stopped_and_what_is_refused() {
    let text = usage();
    assert!(text.contains("NOTHING ON THE MACHINE IS STOPPED"), "{text}");
    assert!(text.contains("WITHOUT `confirm` NOTHING IS SENT"), "{text}");
    assert!(text.contains("NOTHING HERE VALIDATES A SCHEMA"), "{text}");
    assert!(text.contains("--from FILE"), "{text}");
}

// ---------------------------------------------------------------------------
// The plan
// ---------------------------------------------------------------------------

/// **Without `confirm` it is a plan**: the table is read, the file checked,
/// the route named — and nothing is sent.
#[test]
fn without_confirm_it_checks_and_plans_and_opens_nothing() {
    let dir = scratch("plan");
    let file = stored(&dir, "tempmon-rivet", "tempmon.sqlite", &sqlite(3, 7));
    let machine = FakeMachine::with(&machine_table());
    let app = leak(FakeApp::answering(200, "{}"));
    let opened = Cell::new(false);
    let plan = run(&args_for("tempmon-rivet", &file, false), &machine, app, &opened)
        .expect("a plan");
    assert!(!opened.get(), "a plan must not open the sender");
    assert!(app.seen.borrow().is_empty());
    assert_eq!(machine.calls.get(), 1);
    assert!(
        plan.contains(
            "POST /restore/tempmon-rivet on rivet-01.local:7777, which relays it over loopback \
             to the application's restore route, POST /tempmon/api/v1/restore?confirm=true"
        ),
        "{plan}"
    );
    assert!(!plan.contains("https://"), "nothing in the plan goes to the proxy: {plan}");
    assert!(plan.contains("Content-Type: application/vnd.sqlite3"), "{plan}");
    assert!(plan.contains("3072 bytes"), "{plan}");
    assert!(
        plan.contains("the format's own internal length claim"),
        "the plan names the rungs that ran: {plan}"
    );
    assert!(plan.contains("bytes read == the file's size"), "{plan}");
    assert!(plan.contains("Nothing was sent"), "{plan}");
    assert!(plan.contains("Nothing on the machine is stopped"), "{plan}");
    std::fs::remove_dir_all(&dir).ok();
}

/// CSV makes no statement about its own size, and the plan says so rather
/// than printing the same words it prints for SQLite.
#[test]
fn a_format_with_no_length_claim_is_said_to_have_none() {
    let dir = scratch("csv");
    let file = stored(&dir, "ledger-rivet", "ledger.csv", b"when,amount\n1,2\n");
    let machine = FakeMachine::with(&machine_table());
    let app = leak(FakeApp::answering(200, "{}"));
    let plan = run(&args_for("ledger-rivet", &file, false), &machine, app, &Cell::new(false))
        .expect("a plan");
    assert!(plan.contains("makes no statement about its own size"), "{plan}");
    assert!(plan.contains("/ledger/import?confirm=true"), "{plan}");
    assert!(plan.contains("Content-Type: text/csv"), "{plan}");
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// Refusals — every one sends nothing and opens nothing
// ---------------------------------------------------------------------------

fn refused(service: &str, file: &Path, table: Option<&str>) -> (String, usize) {
    let machine = FakeMachine {
        table: table.map(ToString::to_string),
        calls: Cell::new(0),
    };
    let app = leak(FakeApp::answering(200, "{}"));
    let opened = Cell::new(false);
    let why = run(&args_for(service, file, true), &machine, app, &opened)
        .expect_err("must be refused");
    assert!(!opened.get(), "a refusal must not open the sender: {why}");
    assert!(app.seen.borrow().is_empty(), "a refusal must send nothing: {why}");
    (why, machine.calls.get())
}

#[test]
fn a_missing_file_is_refused_before_the_machine_is_asked() {
    let dir = scratch("missing");
    let (why, calls) = refused("tempmon-rivet", &dir.join("nope.sqlite"), Some(&machine_table()));
    assert!(why.contains("NOTHING WAS SENT"), "{why}");
    assert_eq!(calls, 0, "a typo in --from costs no network");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_application_the_machine_does_not_offer_is_refused_naming_both_sides() {
    let dir = scratch("unknown");
    let file = stored(&dir, "x", "a.sqlite", &sqlite(2, 1));
    let (why, _) = refused("logbook-rivet", &file, Some(&machine_table()));
    assert!(why.contains("logbook-rivet"), "{why}");
    for offered in ["tempmon-rivet", "notes-rivet", "reliquary-rivet"] {
        assert!(why.contains(offered), "{why}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_application_that_declares_no_restore_path_is_refused() {
    let dir = scratch("norestore");
    let file = stored(&dir, "notes-rivet", "t.json", b"{\"notes\":[]}");
    let (why, _) = refused("notes-rivet", &file, Some(&machine_table()));
    assert!(why.contains("NO RESTORE"), "{why}");
    assert!(why.contains("/notes/api/v1/backup"), "{why}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_application_offering_a_feed_is_refused() {
    let dir = scratch("feed");
    let file = stored(&dir, "reliquary-rivet", "r.tar", &[0u8; 1024]);
    let (why, _) = refused("reliquary-rivet", &file, Some(&machine_table()));
    assert!(why.contains("FEED"), "{why}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_machine_with_no_table_is_refused() {
    let dir = scratch("notable");
    let file = stored(&dir, "tempmon-rivet", "a.sqlite", &sqlite(2, 1));
    let (why, _) = refused("tempmon-rivet", &file, None);
    assert!(why.contains("there is nothing at /run/rivet/backups"), "{why}");
    std::fs::remove_dir_all(&dir).ok();
}

/// **A file that is not the declared format is refused** — JSON handed to a
/// SQLite application, at rung 3.
#[test]
fn a_file_of_another_format_is_refused() {
    let dir = scratch("format");
    let file = stored(&dir, "tempmon-rivet", "wrong.sqlite", b"{\"not\":\"sqlite\"}");
    let (why, _) = refused("tempmon-rivet", &file, Some(&machine_table()));
    assert!(why.contains("NOT a whole sqlite file"), "{why}");
    assert!(why.contains("does not begin the way a sqlite file begins"), "{why}");
    std::fs::remove_dir_all(&dir).ok();
}

/// **The trap the ladder exists for, in the other direction.** A SQLite file
/// cut short still starts with `SQLite format 3\0`; rung 4 is what refuses to
/// hand it to an application as a restore.
#[test]
fn a_truncated_file_that_still_carries_the_magic_is_refused() {
    let dir = scratch("truncated");
    let mut bytes = sqlite(4, 9);
    bytes.truncate(2048);
    let file = stored(&dir, "tempmon-rivet", "half.sqlite", &bytes);
    let (why, _) = refused("tempmon-rivet", &file, Some(&machine_table()));
    assert!(why.contains("its own header says it is 4096 bytes and 2048 arrived"), "{why}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_empty_file_is_refused() {
    let dir = scratch("empty");
    let file = stored(&dir, "tempmon-rivet", "empty.sqlite", b"");
    let (why, _) = refused("tempmon-rivet", &file, Some(&machine_table()));
    assert!(why.contains("zero bytes"), "{why}");
    std::fs::remove_dir_all(&dir).ok();
}

/// Over the bound, refused on its size alone — the file is never read.
#[test]
fn a_file_over_the_bound_is_refused_without_being_read() {
    let dir = scratch("huge");
    let at = dir.join("tempmon-rivet");
    std::fs::create_dir_all(&at).expect("dir");
    let file = at.join("huge.sqlite");
    std::fs::File::create(&file)
        .expect("create")
        .set_len(MAX_BODY + 1)
        .expect("a sparse length");
    let (why, _) = refused("tempmon-rivet", &file, Some(&machine_table()));
    assert!(why.contains("over the"), "{why}");
    std::fs::remove_dir_all(&dir).ok();
}

/// **The one "which application" check, and it is a layout check.** A valid
/// SQLite file inside `notes-rivet/` — where `backup` puts the notes application's — is
/// refused for TempMon. The same file anywhere else is not, and the module says
/// how weak that makes it.
#[test]
fn a_file_in_another_applications_backup_directory_is_refused() {
    let dir = scratch("other");
    let bytes = sqlite(2, 3);
    let in_other = stored(&dir, "notes-rivet", "x.sqlite", &bytes);
    let (why, _) = refused("tempmon-rivet", &in_other, Some(&machine_table()));
    assert!(why.contains("directory named notes-rivet"), "{why}");

    let elsewhere = stored(&dir, "copied-out", "x.sqlite", &bytes);
    let machine = FakeMachine::with(&machine_table());
    let app = leak(FakeApp::answering(200, "{}"));
    run(&args_for("tempmon-rivet", &elsewhere, false), &machine, app, &Cell::new(false))
        .expect("a directory named for no application is not refused");
    std::fs::remove_dir_all(&dir).ok();
}

// ---------------------------------------------------------------------------
// Sending, and the application's answer
// ---------------------------------------------------------------------------

/// **Success: exactly the bytes that were checked, to the declared route, with
/// the format's type — and the application's answer printed as it gave it.**
#[test]
fn confirmed_it_sends_the_checked_bytes_and_reports_the_answer() {
    let dir = scratch("send");
    let bytes = sqlite(3, 5);
    let file = stored(&dir, "tempmon-rivet", "t.sqlite", &bytes);
    let machine = FakeMachine::with(&machine_table());
    let app = leak(FakeApp::answering(200, "{\"loaded\":{\"readings\":12}}"));
    let opened = Cell::new(false);
    let report = run(&args_for("tempmon-rivet", &file, true), &machine, app, &opened)
        .expect("a 200 is success");
    assert!(opened.get());
    let seen = app.seen.borrow();
    assert_eq!(seen.len(), 1, "exactly one send");
    assert_eq!(seen[0].0, "tempmon-rivet");
    assert_eq!(seen[0].1, bytes, "what is sent is byte for byte what was checked");
    assert!(report.contains("tempmon-rivet answered 200 (application/json)"), "{report}");
    assert!(report.contains("| {\"loaded\":{\"readings\":12}}"), "{report}");
    std::fs::remove_dir_all(&dir).ok();
}

/// **Anything but 2xx is a failure, and the application's own words travel
/// with it.**
#[test]
fn a_non_2xx_answer_is_a_failure_that_carries_the_applications_words() {
    let dir = scratch("refusal");
    let file = stored(&dir, "tempmon-rivet", "t.sqlite", &sqlite(2, 5));
    for status in [400u16, 401, 403, 409, 500, 503, 301] {
        let machine = FakeMachine::with(&machine_table());
        let app = leak(FakeApp::answering(
            status,
            "{\"error\":\"schema version 7, this installation is 9\"}",
        ));
        let why = run(&args_for("tempmon-rivet", &file, true), &machine, app, &Cell::new(false))
            .expect_err("not 2xx");
        assert!(why.contains(&format!("answered {status}")), "{why}");
        assert!(why.contains("THE RESTORE FAILED"), "{why}");
        assert!(why.contains("schema version 7, this installation is 9"), "{why}");
        assert_eq!(app.seen.borrow().len(), 1);
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// 204 and 202 are 2xx. A 202 says it has not finished, and the report says
/// that rather than printing it as done.
#[test]
fn a_204_is_success_and_a_202_is_success_with_a_note() {
    let dir = scratch("2xx");
    let file = stored(&dir, "tempmon-rivet", "t.sqlite", &sqlite(2, 5));
    let machine = FakeMachine::with(&machine_table());
    let app = leak(FakeApp::answering(204, ""));
    let report = run(&args_for("tempmon-rivet", &file, true), &machine, app, &Cell::new(false))
        .expect("204");
    assert!(report.contains("answered 204"), "{report}");
    assert!(report.contains("(an empty body)"), "{report}");
    assert!(!report.contains("NOTE: 202"), "{report}");

    let app = leak(FakeApp::answering(202, "queued"));
    let report = run(&args_for("tempmon-rivet", &file, true), &machine, app, &Cell::new(false))
        .expect("202");
    assert!(report.contains("NOTE: 202"), "{report}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_transport_failure_is_a_failure() {
    let dir = scratch("transport");
    let file = stored(&dir, "tempmon-rivet", "t.sqlite", &sqlite(2, 5));
    let machine = FakeMachine::with(&machine_table());
    let app = leak(FakeApp {
        answer: Err("rivet-01.local:7777: connection reset".to_string()),
        seen: RefCell::new(Vec::new()),
    });
    let why = run(&args_for("tempmon-rivet", &file, true), &machine, app, &Cell::new(false))
        .expect_err("no answer");
    assert!(why.contains("connection reset"), "{why}");
    std::fs::remove_dir_all(&dir).ok();
}

/// **The machine's own refusal is not the application's answer**: a 404 the
/// machine composed is reported as the machine's, never as "tempmon-rivet
/// answered 404".
#[test]
fn the_machines_refusal_is_reported_as_the_machines() {
    let dir = scratch("machine-refused");
    let file = stored(&dir, "tempmon-rivet", "t.sqlite", &sqlite(2, 5));
    let machine = FakeMachine::with(&machine_table());
    let app = leak(FakeApp {
        answer: Ok(relayed(
            404,
            Some("text/plain"),
            b"tempmon-rivet declares no backup surface on this machine",
            &[(rivet_core::backup::ANSWERED_BY_HEADER, "rivet-mcpd")],
        )),
        seen: RefCell::new(Vec::new()),
    });
    let why = run(&args_for("tempmon-rivet", &file, true), &machine, app, &Cell::new(false))
        .expect_err("refused");
    assert!(why.contains("the machine answered 404 itself"), "{why}");
    assert!(!why.contains("tempmon-rivet answered"), "{why}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_non_text_or_long_answer_is_described_rather_than_dumped() {
    let mut answer = relayed(200, None, &[0xff, 0xfe, 0x00], &[]);
    answer.truncated = true;
    let text = render_answer("svc", &answer);
    assert!(text.contains("3 bytes that are not text"), "{text}");
    assert!(text.contains("longer than"), "{text}");
}

