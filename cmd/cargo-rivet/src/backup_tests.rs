//! `cargo rivet backup` — arguments, the plan, the filter, the ladder at the
//! writer, the answer reader, and the whole verb against a fake control plane:
//! once behind the `ReachesApplications` seam, and once over a real pinned TLS
//! listener that records what the client presented.
//!
//! Every fixture is hand-composed: the tables are what
//! `rivet_core::backup::render` writes, the SQLite files are headers built to
//! the format's own arithmetic, and the identities are minted per run.

// A test asserts; `unwrap`, `expect` and `panic!` ARE the assertion.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::cell::RefCell;

use super::*;

fn parse_of(list: &[&str]) -> Result<BackupArgs, String> {
    let owned: Vec<String> = list.iter().map(|w| (*w).to_string()).collect();
    parse(&owned)
}

fn full(extra: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = [
        "--host",
        "rivet-01.local:7777",
        "--identity",
        "./id",
        "--pin",
        "ab",
        "--into",
        "./backups",
    ]
    .iter()
    .map(|w| (*w).to_string())
    .collect();
    out.extend(extra.iter().map(|w| (*w).to_string()));
    out
}

fn sqlite(pages: u32, marker: u8) -> Vec<u8> {
    let mut bytes = vec![marker; 1024 * pages as usize];
    bytes[..16].copy_from_slice(b"SQLite format 3\0");
    bytes[16..18].copy_from_slice(&1024u16.to_be_bytes());
    bytes[18..28].fill(0);
    bytes[28..32].copy_from_slice(&pages.to_be_bytes());
    bytes
}

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

#[test]
fn without_confirm_it_is_a_plan() {
    let parsed = parse(&full(&[])).expect("valid");
    assert!(!parsed.confirm);
    let parsed = parse(&full(&["confirm"])).expect("valid");
    assert!(parsed.confirm);
}

/// **A default would be this tool choosing where somebody's backups live.**
#[test]
fn without_into_it_refuses_rather_than_choosing_a_directory() {
    let refused = parse_of(&["--host", "pi:7777", "--identity", "./id", "--pin", "ab"])
        .expect_err("no --into");
    assert!(refused.contains("--into is required"), "{refused}");
}

/// `ADR-025` §2's pairing, checked by the same code `run` and `deploy` use.
#[test]
fn half_the_trust_model_is_refused_before_anything_connects() {
    let refused = parse_of(&["--host", "pi:7777", "--identity", "./id", "--into", "."])
        .expect_err("no pin");
    assert!(refused.contains("--pin"), "{refused}");
}

#[test]
fn an_option_this_verb_does_not_take_is_refused() {
    let refused = parse(&full(&["--keep", "7"])).expect_err("not an option here");
    assert!(refused.contains("--keep"), "{refused}");
}

/// **The three flags that reached the proxy are refused BY NAME**, not
/// ignored: a script still passing them believes it is choosing a certificate,
/// and must find out that nothing presents one (`.rulings/` R-234).
#[test]
fn the_flags_that_reached_the_proxy_are_refused_by_name() {
    for (flag, value) in [("--authority", "./ca"), ("--name", "rivet-01.local"), ("--port", "443")] {
        let refused = parse(&full(&[flag, value])).expect_err("retired");
        assert!(refused.contains(flag), "{refused}");
        assert!(refused.contains("control plane"), "{refused}");
        assert!(refused.contains("remove"), "{refused}");
    }
}

/// **There is no schedule and no retention here**, and the usage says so
/// rather than leaving somebody to discover it by not finding a flag.
#[test]
fn the_usage_says_retention_and_cadence_are_not_this_verbs() {
    let text = usage();
    assert!(text.contains("NO SCHEDULE AND NO RETENTION"), "{text}");
    assert!(text.contains("NOTHING HERE VALIDATES A SCHEMA"), "{text}");
    assert!(text.contains("A FEED IS REPORTED AND NOT FETCHED"), "{text}");
    assert!(text.contains("--service"), "{text}");
    assert!(text.contains("repeatable"), "{text}");
    assert!(text.contains("REFUSED, not ignored"), "{text}");
}

/// **A filename from another machine is DECLINED rather than repaired.**
#[test]
fn a_disposition_filename_that_could_escape_the_directory_is_declined() {
    for hostile in [
        "attachment; filename=\"../../id_ed25519\"",
        "attachment; filename=\"/etc/shadow\"",
        "attachment; filename=\"..\"",
        "attachment; filename=\".\"",
        "attachment; filename=\".hidden\"",
        "attachment; filename=\"\"",
        "attachment; filename=\"a b\"",
        "attachment; filename=\"x\\\\y\"",
        "attachment",
    ] {
        assert_eq!(disposition_filename(hostile), None, "{hostile:?} must not name a file");
    }
}

/// TempMon's own header, which names the backup for the instant it was taken.
#[test]
fn a_services_own_filename_is_used_when_it_is_a_safe_component() {
    assert_eq!(
        disposition_filename("attachment; filename=\"tempmon-backup-20260908-143000.db\""),
        Some("tempmon-backup-20260908-143000.db".to_string())
    );
}

// ---------------------------------------------------------------------------
// --service — filters the fetch, not the report
// ---------------------------------------------------------------------------

fn declaration(service: &str, route: &str, format: BackupFormat) -> Declaration {
    Declaration {
        service: service.to_string(),
        route: route.to_string(),
        surface: BackupSurface::Artifact {
            path: "api/v1/backup".to_string(),
            format,
            restore: None,
        },
    }
}

fn sample_declarations() -> Vec<Declaration> {
    vec![
        declaration("reliquary-rivet", "reliquary", BackupFormat::Tar),
        declaration("tempmon-rivet", "tempmon", BackupFormat::Sqlite),
        declaration("notes-rivet", "notes", BackupFormat::Json),
    ]
}

/// **The unfiltered path is the identity**: everything the machine offers,
/// in the table's own order, and nothing excluded.
#[test]
fn with_no_service_named_every_declaration_is_included_and_none_is_excluded() {
    let declarations = sample_declarations();
    let (included, excluded) = filter_by_service(&declarations, &[]).expect("no filter");
    assert!(excluded.is_empty());
    assert_eq!(included.len(), declarations.len());
    for (from_filter, from_table) in included.iter().zip(declarations.iter()) {
        assert_eq!(**from_filter, *from_table, "order and identity must be preserved");
    }
}

#[test]
fn one_service_name_includes_only_that_application() {
    let declarations = sample_declarations();
    let wanted = vec!["notes-rivet".to_string()];
    let (included, excluded) = filter_by_service(&declarations, &wanted).expect("offered");
    assert_eq!(included.len(), 1);
    assert_eq!(included[0].service, "notes-rivet");
    assert_eq!(excluded.len(), 2);
}

/// **A name matching nothing is an ERROR naming both sides.** `ADR-051` §8.
#[test]
fn a_service_name_matching_nothing_is_an_error_naming_both_sides() {
    let declarations = sample_declarations();
    let wanted = vec!["tempmon-rivet".to_string(), "logbook-rivet".to_string()];
    let refused = filter_by_service(&declarations, &wanted).expect_err("not offered");
    for name in ["logbook-rivet", "tempmon-rivet", "notes-rivet", "reliquary-rivet"] {
        assert!(refused.contains(name), "{refused}");
    }
    let refused = filter_by_service(&[], &["tempmon-rivet".to_string()]).expect_err("nothing");
    assert!(refused.contains("tempmon-rivet"), "{refused}");
}

/// The plan names what will be fetched, how it travels, and what the filter
/// excluded; the unfiltered plan carries no trace of the filter.
#[test]
fn the_plan_names_the_control_plane_route_and_what_service_excluded() {
    let declarations = sample_declarations();
    let wanted = vec!["notes-rivet".to_string()];
    let (included, excluded) = filter_by_service(&declarations, &wanted).expect("offered");
    let args = parse(&full(&["--service", "notes-rivet"])).expect("valid");
    let text = plan(&included, &excluded, &args, "rivet-01.local:7777");
    assert!(text.starts_with("1 application(s) on this machine"), "{text}");
    assert!(text.contains("\n  notes-rivet\n"), "{text}");
    assert!(text.contains("GET /backup/notes-rivet on rivet-01.local:7777"), "{text}");
    assert!(text.contains("/notes/api/v1/backup"), "{text}");
    assert!(text.contains("excluded by --service"), "{text}");
    assert!(!text.contains("https://"), "nothing in the plan goes to the proxy: {text}");

    let (included, excluded) = filter_by_service(&declarations, &[]).expect("no filter");
    let text = plan(&included, &excluded, &parse(&full(&[])).expect("valid"), "h:7777");
    assert!(!text.contains("excluded"), "{text}");
}

/// **A feed's plan line still begins `feed `** — the four-space line Rivet
/// Monitor reads to know an application is a feed and not to schedule it.
#[test]
fn a_feed_is_planned_on_a_line_beginning_feed() {
    let feed = Declaration {
        service: "reliquary-rivet".to_string(),
        route: "reliquary".to_string(),
        surface: BackupSurface::Feed {
            path: "api/v1/blobs".to_string(),
        },
    };
    let text = plan(&[&feed], &[], &parse(&full(&[])).expect("valid"), "h:7777");
    assert!(text.contains("\n  reliquary-rivet\n    feed   /reliquary/api/v1/blobs\n"), "{text}");
}

// ---------------------------------------------------------------------------
// store — a service name is a directory component
// ---------------------------------------------------------------------------

#[test]
fn is_single_plain_component_accepts_only_one_plain_name() {
    for hostile in ["../../outside", "/etc/x", "C:\\x", "C:/x", "a/b", "a\\b", "..", ".", ""] {
        assert!(!is_single_plain_component(hostile), "{hostile:?}");
    }
    for valid in ["photo-index", "aura-renderer", "tempmon-rivet"] {
        assert!(is_single_plain_component(valid), "{valid:?}");
    }
}

fn csv_declaration(service: &str) -> Declaration {
    Declaration {
        service: service.to_string(),
        route: "tempmon".to_string(),
        surface: BackupSurface::Artifact {
            path: "api/v1/backup".to_string(),
            format: BackupFormat::Csv,
            restore: None,
        },
    }
}

/// **Containment, driven at the writer — independent of the parser.**
#[test]
fn a_hostile_service_creates_nothing_outside_into_and_a_valid_one_writes_beneath_it() {
    let parent = crate::scratch::dir("backup-hostile-service-containment");
    let into = parent.join("into");
    std::fs::create_dir_all(&into).expect("into");
    let bytes = b"a,b\n1,2\n";
    for hostile in ["../escape", "a/b", "a\\b", "..", "."] {
        let result = store(
            &csv_declaration(hostile),
            &into,
            BackupFormat::Csv,
            bytes,
            Some(bytes.len() as u64),
            None,
        );
        assert!(result.is_err(), "{hostile:?} must be refused");
    }
    assert!(!parent.join("escape").exists());
    let stored = store(&csv_declaration("photo-index"), &into, BackupFormat::Csv, bytes, Some(8), None)
        .expect("a valid service name must still be writable");
    assert_eq!(stored.file.parent(), Some(into.join("photo-index").as_path()));
    assert_eq!(std::fs::read(&stored.file).expect("written"), bytes);
}

#[test]
fn a_truncated_json_body_is_not_stored_as_a_successful_backup() {
    let into = crate::scratch::dir("backup-truncated-json");
    let truncated = br#"{"records":[{"a":1}"#;
    let one = Declaration {
        service: "notes-rivet".to_string(),
        route: "notes".to_string(),
        surface: BackupSurface::Artifact {
            path: "api/backup".to_string(),
            format: BackupFormat::Json,
            restore: None,
        },
    };
    let result = store(&one, &into, BackupFormat::Json, truncated, Some(truncated.len() as u64), None);
    let Err(message) = result else {
        panic!("a truncated JSON body must not be stored");
    };
    assert!(message.contains("NOT stored"), "{message}");
    assert_eq!(std::fs::read_dir(&into).expect("into").count(), 0);
}

// ---------------------------------------------------------------------------
// read_answer — the application's answer as the machine relayed it
// ---------------------------------------------------------------------------

fn answer_of(bytes: &[u8], limit: u64, overflow: Overflow) -> Result<Relayed, String> {
    read_answer(&mut &bytes[..], limit, overflow)
}

#[test]
fn an_answer_with_a_content_length_is_read_to_that_length() {
    let got = answer_of(
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Disposition: attachment; filename=\"a.db\"\r\n\r\nhelloTRAILING",
        100,
        Overflow::Refuse,
    )
    .expect("read");
    assert_eq!(got.status, 200);
    assert_eq!(got.body, b"hello", "nothing past the declared length is read");
    assert_eq!(got.declared_length(), Some(5));
    assert_eq!(got.header("content-disposition"), Some("attachment; filename=\"a.db\""));
    assert!(!got.answered_by_the_machine());
}

/// **A repeated `Content-Length` is refused, agreeing or not**: two readers of
/// the same bytes that honour different fields disagree about where the body
/// ends, and the ladder's length rung would be judging one of them.
#[test]
fn an_answer_with_a_repeated_content_length_is_refused() {
    for second in ["5", "2"] {
        let answer = format!("HTTP/1.1 200 OK\r\nContent-Length: 5\r\ncontent-length: {second}\r\n\r\nhello");
        let refused = answer_of(answer.as_bytes(), 100, Overflow::Refuse).expect_err("a repeated length");
        assert!(refused.contains("Content-Length"), "{refused}");
    }
}

/// **A body cut short is kept, not refused here** — the ladder's second rung
/// is where that is judged, with the numbers.
#[test]
fn a_body_short_of_its_length_is_kept_for_the_ladder_to_judge() {
    let got = answer_of(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nhalf", 100, Overflow::Refuse)
        .expect("read");
    assert_eq!(got.body, b"half");
    let fault = rivet_core::backup::validate(BackupFormat::Csv, &got.body, got.declared_length())
        .expect_err("short");
    assert!(fault.to_string().contains("10"), "{fault}");
}

#[test]
fn a_chunked_answer_is_decoded_and_declares_no_length() {
    let got = answer_of(
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6;x=y\r\n world\r\n0\r\nTrailer: t\r\n\r\n",
        100,
        Overflow::Refuse,
    )
    .expect("read");
    assert_eq!(got.body, b"hello world");
    assert_eq!(got.declared_length(), None);
}

#[test]
fn an_answer_with_no_framing_is_read_to_the_end() {
    let got = answer_of(b"HTTP/1.0 200 OK\r\n\r\nall of it", 100, Overflow::Refuse).expect("read");
    assert_eq!(got.body, b"all of it");
}

#[test]
fn a_body_over_the_bound_is_refused_or_cut_as_asked() {
    let long = b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\n\r\n01234567890123456789";
    let refused = answer_of(long, 10, Overflow::Refuse).expect_err("over");
    assert!(refused.contains("bound"), "{refused}");
    let cut = answer_of(long, 10, Overflow::Truncate).expect("cut");
    assert_eq!(cut.body, b"0123456789");
    assert!(cut.truncated);
    let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n14\r\n01234567890123456789\r\n0\r\n\r\n";
    assert!(answer_of(chunked, 10, Overflow::Refuse).is_err());
    assert!(answer_of(chunked, 10, Overflow::Truncate).expect("cut").truncated);
}

#[test]
fn something_that_is_not_an_http_answer_is_refused() {
    assert!(answer_of(b"SSH-2.0-nope\r\n\r\n", 10, Overflow::Refuse).is_err());
    assert!(answer_of(b"HTTP/1.1 200 OK\r\nno end", 10, Overflow::Refuse).is_err());
}

/// The machine's own refusal is told from the application's by its header.
#[test]
fn the_machines_refusal_is_told_from_the_applications() {
    let got = answer_of(
        format!(
            "HTTP/1.1 404 Not Found\r\nContent-Length: 3\r\n{}: rivet-mcpd\r\n\r\nno.",
            rivet_core::backup::ANSWERED_BY_HEADER
        )
        .as_bytes(),
        100,
        Overflow::Refuse,
    )
    .expect("read");
    assert!(got.answered_by_the_machine());
}

// ---------------------------------------------------------------------------
// The whole verb, behind the seam
// ---------------------------------------------------------------------------

/// A machine that answers `get file /run/rivet/backups` and nothing else.
pub(crate) struct FakeMachine(pub(crate) String);

impl CallsTools for FakeMachine {
    fn call_tool(&self, tool: &str, arguments: &str) -> Result<String, String> {
        assert_eq!(tool, "rivet_get", "backup reads and never applies: {arguments}");
        let path = rivet_core::backup::PATH;
        Ok(format!(
            r#"{{"subject":"file","path":"{path}","present":true,"content":"{}","content_encoding":"text"}}"#,
            self.0.replace('\n', "\\n")
        ))
    }
}

/// What the fake control plane answers per service, and which it was asked.
struct FakeReach {
    answers: Vec<(String, Vec<u8>)>,
    asked: RefCell<Vec<String>>,
}

impl ReachesApplications for FakeReach {
    fn fetch(&self, service: &str) -> Result<Relayed, String> {
        self.asked.borrow_mut().push(service.to_string());
        let (_, bytes) = self
            .answers
            .iter()
            .find(|(s, _)| s == service)
            .ok_or_else(|| format!("{service}: nothing answered"))?;
        read_answer(&mut &bytes[..], MAX_BODY, Overflow::Refuse)
    }

    fn restore(&self, _: &str, _: &[u8], _: u64) -> Result<Relayed, String> {
        panic!("backup never restores");
    }
}

fn ok_sqlite(bytes: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Disposition: attachment; filename=\"probe.sqlite\"\r\n\r\n",
        bytes.len()
    )
    .into_bytes();
    out.extend_from_slice(bytes);
    out
}

fn sqlite_table() -> String {
    rivet_core::backup::render(&[
        declaration("probe-rivet", "probe", BackupFormat::Sqlite),
        declaration("broken-rivet", "broken", BackupFormat::Sqlite),
        declaration("missing-rivet", "missing", BackupFormat::Sqlite),
    ])
}

/// **Confirmed, every application is fetched over the seam, checked and
/// stored; one failing does not stop the others; the machine's refusal and
/// the application's are each reported as what they are.**
#[test]
fn confirmed_it_fetches_checks_and_stores_and_reports_every_failure() {
    let into = crate::scratch::dir("backup-via");
    let good = sqlite(3, 0x11);
    let mut cut = sqlite(4, 0x22);
    cut.truncate(2048);
    let reach = FakeReach {
        answers: vec![
            ("probe-rivet".to_string(), ok_sqlite(&good)),
            ("broken-rivet".to_string(), ok_sqlite(&cut)),
            (
                "missing-rivet".to_string(),
                format!(
                    "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 11\r\n{}: rivet-mcpd\r\n\r\nunreachable",
                    rivet_core::backup::ANSWERED_BY_HEADER
                )
                .into_bytes(),
            ),
        ],
        asked: RefCell::new(Vec::new()),
    };
    let mut args = parse(&full(&["confirm"])).expect("valid");
    args.into = into.clone();
    let failed = backup_via(&args, &FakeMachine(sqlite_table()), &reach, "h:7777")
        .expect_err("two of three failed");
    assert!(failed.contains("1 stored, 0 deferred, 2 failed"), "{failed}");
    assert!(failed.contains("broken-rivet: what was fetched is NOT stored"), "{failed}");
    assert!(failed.contains("missing-rivet: the machine refused"), "{failed}");
    assert_eq!(
        std::fs::read(into.join("probe-rivet").join("probe.sqlite")).expect("stored"),
        good
    );
    assert!(!into.join("broken-rivet").exists(), "a refused body leaves nothing behind");
}

/// **`--service` narrows the FETCH**: an application not named is never asked
/// for, so its bytes never cross the network.
#[test]
fn an_application_service_excluded_is_never_fetched() {
    let into = crate::scratch::dir("backup-filtered");
    let reach = FakeReach {
        answers: vec![("probe-rivet".to_string(), ok_sqlite(&sqlite(2, 1)))],
        asked: RefCell::new(Vec::new()),
    };
    let mut args = parse(&full(&["--service", "probe-rivet", "confirm"])).expect("valid");
    args.into = into;
    backup_via(&args, &FakeMachine(sqlite_table()), &reach, "h:7777").expect("stored");
    assert_eq!(*reach.asked.borrow(), vec!["probe-rivet".to_string()]);
}

/// Without `confirm`, nothing is fetched at all.
#[test]
fn a_plan_fetches_nothing() {
    let reach = FakeReach {
        answers: Vec::new(),
        asked: RefCell::new(Vec::new()),
    };
    let text = backup_via(&parse(&full(&[])).expect("valid"), &FakeMachine(sqlite_table()), &reach, "h:7777")
        .expect("a plan");
    assert!(text.contains("Nothing was fetched"), "{text}");
    assert!(reach.asked.borrow().is_empty());
}

// ---------------------------------------------------------------------------
// The whole verb, over a real pinned TLS control plane
// ---------------------------------------------------------------------------

/// Over a real control plane: `execute` itself, with a minted identity on disk.
mod over_the_control_plane {
    use std::io::Write as _;
    use std::sync::{Arc, Mutex};

    use super::*;

    /// An Ed25519 identity as `cargo rivet identity new` mints one — the seed
    /// and a self-signed DER certificate.
    fn an_identity() -> (rivetos_tls::Identity, rivetos_tls::Pin) {
        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("an Ed25519 pair");
        let mut params = rcgen::CertificateParams::new(vec!["rivet".to_string()]).expect("params");
        params.is_ca = rcgen::IsCa::NoCa;
        let leaf = params.self_signed(&key).expect("a self-signed leaf");
        let der = key.serialize_der();
        let at = der
            .windows(4)
            .position(|w| w == [0x04, 0x22, 0x04, 0x20])
            .expect("PKCS#8 for Ed25519 wraps the seed in 04 22 04 20")
            + 4;
        let mut seed = [0_u8; 32];
        seed.copy_from_slice(&der[at..at + 32]);
        let certificate = leaf.der().to_vec();
        let pin = rivetos_tls::public_key_in_certificate(&certificate).expect("a key");
        (rivetos_tls::Identity { certificate, seed }, pin)
    }

    /// **The control plane**: answers an MCP connection with the table, and an
    /// HTTP one with `answer`, recording every certificate chain presented.
    fn control_plane(
        identity: rivetos_tls::Identity,
        accepts: Vec<rivetos_tls::Pin>,
        table: String,
        answer: Vec<u8>,
        presented: Arc<Mutex<Vec<Vec<Vec<u8>>>>>,
    ) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        let host = listener.local_addr().expect("an address").to_string();
        let config = Arc::new(
            rivetos_tls::server_config(&identity, accepts, rivetos_tls::provider()).expect("config"),
        );
        std::thread::spawn(move || {
            for socket in listener.incoming() {
                let Ok(socket) = socket else { continue };
                let Ok(connection) = rustls::ServerConnection::new(config.clone()) else {
                    continue;
                };
                let mut stream = rustls::StreamOwned::new(connection, socket);
                let mut first = [0u8; 1];
                if stream.read_exact(&mut first).is_err() {
                    continue;
                }
                presented.lock().expect("lock").push(
                    stream
                        .conn
                        .peer_certificates()
                        .map(|c| c.iter().map(|d| d.as_ref().to_vec()).collect())
                        .unwrap_or_default(),
                );
                if first[0] == b'{' {
                    // MCP: the client ends its request, then reads.
                    let mut rest = Vec::new();
                    let _ = stream.read_to_end(&mut rest);
                    let inner = format!(
                        r#"{{"subject":"file","path":"{}","present":true,"content":"{}","content_encoding":"text"}}"#,
                        rivet_core::backup::PATH,
                        table.replace('\n', "\\n")
                    );
                    let text = inner.replace('\\', "\\\\").replace('"', "\\\"");
                    let line = format!(
                        "{{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{text}\"}}]}}}}\n"
                    );
                    let _ = stream.write_all(line.as_bytes());
                } else {
                    let mut head = vec![first[0]];
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        if stream.read_exact(&mut byte).is_err() {
                            break;
                        }
                        head.push(byte[0]);
                    }
                    assert!(
                        head.starts_with(b"GET /backup/probe-rivet HTTP/1.1\r\n"),
                        "{}",
                        String::from_utf8_lossy(&head)
                    );
                    let _ = stream.write_all(&answer);
                }
                stream.conn.send_close_notify();
                let _ = stream.flush();
            }
        });
        host
    }

    /// **A backup presents the operator's pinned control-plane identity and NO
    /// OTHER CERTIFICATE, on every connection it makes** — `.rulings/` R-234's
    /// regression guard. A fleet browser certificate sits in a directory the
    /// verb could find, and is never read: if a backup ever presented a
    /// certificate chaining to a fleet authority again, the chain recorded here
    /// would differ.
    #[test]
    fn a_backup_presents_only_the_operators_control_plane_identity() {
        let (board, board_pin) = an_identity();
        let (operator, operator_pin) = an_identity();
        let identity_dir = crate::scratch::dir("backup-e2e-identity");
        std::fs::write(identity_dir.join(rivetos_tls::SEED_FILE), operator.seed).expect("seed");
        std::fs::write(identity_dir.join(rivetos_tls::CERTIFICATE_FILE), &operator.certificate)
            .expect("certificate");
        let operator_certificate = operator.certificate.clone();

        let table = rivet_core::backup::render(&[declaration("probe-rivet", "probe", BackupFormat::Sqlite)]);
        let body = sqlite(5, 0x33);
        let presented = Arc::new(Mutex::new(Vec::new()));
        let host = control_plane(board, vec![operator_pin], table, ok_sqlite(&body), presented.clone());

        let into = crate::scratch::dir("backup-e2e-into");
        let args: Vec<String> = [
            "--host",
            &host,
            "--identity",
            &identity_dir.display().to_string(),
            "--pin",
            &rivetos_tls::encode_hex(&board_pin),
            "--into",
            &into.display().to_string(),
            "confirm",
        ]
        .iter()
        .map(|w| (*w).to_string())
        .collect();
        let report = execute(&args).expect("the backup over the control plane");
        assert!(report.contains("1 stored, 0 deferred, 0 failed"), "{report}");
        assert_eq!(std::fs::read(into.join("probe-rivet").join("probe.sqlite")).expect("stored"), body);

        let presented = presented.lock().expect("lock");
        assert_eq!(presented.len(), 2, "one connection for the table, one for the payload");
        for chain in presented.iter() {
            assert_eq!(
                chain,
                &vec![operator_certificate.clone()],
                "every connection presents exactly the operator's pinned identity"
            );
        }
    }
}
