//! **The ladder, tested as the trap it exists for.**
//!
//! The assertions that matter here are not *"a good file passes"*. They are the
//! ones where a file that is wrong looks right: a SQLite database cut in half
//! that still carries `SQLite format 3\0`, a tar with its end-of-archive blocks
//! gone, a JSON document that stops mid-object. Each of those is a backup that
//! a client believing rung 3 would store and rotate a real one out for.

// A test asserts; `unwrap`, `expect` and `panic!` ARE the assertion.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

// --- the control plane's routes ------------------------------------------------

/// **Both ends compose the route from these functions**, and the parser reads
/// back exactly what they write.
#[test]
fn a_control_plane_route_names_one_service_and_reads_back() {
    assert_eq!(backup_route("tempmon-rivet"), "/backup/tempmon-rivet");
    assert_eq!(restore_route("tempmon-rivet"), "/restore/tempmon-rivet");
    assert_eq!(
        requested(&backup_route("tempmon-rivet")),
        Some((Requested::Backup, "tempmon-rivet"))
    );
    assert_eq!(
        requested(&restore_route("tempmon-rivet")),
        Some((Requested::Restore, "tempmon-rivet"))
    );
}

/// **Anything that is not one service name is no route at all**, so nothing
/// past the parser is handed a separator, a dot segment or an empty name.
#[test]
fn a_route_naming_anything_but_one_service_is_not_a_route() {
    for path in [
        "/backup/",
        "/backup/../state",
        "/backup/a/b",
        "/backup/.",
        "/restore/x%2fy",
        "/backups/tempmon-rivet",
        "/payload/x.rvt",
    ] {
        assert_eq!(requested(path), None, "{path:?} must not name a service");
    }
}

/// **The application is asked for its own path**, the route prefix removed as
/// `rivet-proxyd` removes it, so it sees one request line whichever way the
/// backup arrived.
#[test]
fn the_application_path_drops_the_route_prefix_as_the_proxy_does() {
    let table = build(&[restorable(
        "tempmon-rivet",
        "tempmon",
        BackupFormat::Sqlite,
        "api/v1/restore",
    )]);
    let one = &table[0];
    assert_eq!(one.request_path(), "/tempmon/api/v1/backup");
    assert_eq!(one.application_path(), "/api/v1/backup");
    assert_eq!(one.application_restore_path().as_deref(), Some("/api/v1/restore"));
    let routes = [crate::proxy::Route {
        path: "tempmon".to_string(),
        service: "tempmon-rivet".to_string(),
        port: 8080,
        requires_operator: true,
        rules: Vec::new(),
    }];
    let (_, rest) = crate::proxy::route_for(&routes, &one.request_path()).expect("routed");
    assert_eq!(rest, one.application_path(), "the proxy and the control plane must agree");
}

// --- the table ---------------------------------------------------------------

fn artifact(service: &str, route: &str, path: &str, format: BackupFormat) -> Wanted {
    Wanted {
        service: service.to_string(),
        route: route.to_string(),
        surface: BackupSurface::Artifact {
            path: path.to_string(),
            format,
            restore: None,
        },
    }
}

/// An artifact that also accepts a restore — `ADR-051` §12.
fn restorable(service: &str, route: &str, format: BackupFormat, restore: &str) -> Wanted {
    Wanted {
        service: service.to_string(),
        route: route.to_string(),
        surface: BackupSurface::Artifact {
            path: "api/v1/backup".to_string(),
            format,
            restore: Some(restore.to_string()),
        },
    }
}

fn feed(service: &str, route: &str, path: &str) -> Wanted {
    Wanted {
        service: service.to_string(),
        route: route.to_string(),
        surface: BackupSurface::Feed {
            path: path.to_string(),
        },
    }
}

#[test]
fn the_table_is_a_function_of_the_set_and_not_of_its_order() {
    let a = build(&[
        artifact("tempmon-rivet", "tempmon", "api/v1/backup", BackupFormat::Sqlite),
        feed("reliquary-rivet", "reliquary", "api/v1/blobs"),
    ]);
    let b = build(&[
        feed("reliquary-rivet", "reliquary", "api/v1/blobs"),
        artifact("tempmon-rivet", "tempmon", "api/v1/backup", BackupFormat::Sqlite),
    ]);
    assert_eq!(a, b);
    assert_eq!(a[0].service, "reliquary-rivet");
}

#[test]
fn a_rendered_table_parses_back_identically() {
    let table = build(&[
        artifact("tempmon-rivet", "tempmon", "api/v1/backup", BackupFormat::Sqlite),
        feed("reliquary-rivet", "reliquary", "api/v1/blobs"),
    ]);
    let text = render(&table);
    assert!(
        text.contains("tempmon-rivet tempmon artifact sqlite api/v1/backup\n"),
        "{text}"
    );
    assert!(
        text.contains("reliquary-rivet reliquary feed api/v1/blobs\n"),
        "{text}"
    );
    assert_eq!(parse(&text), table);
}

/// The request path is composed in one place, so a client cannot get the slash
/// wrong on its own.
#[test]
fn the_request_path_joins_the_route_and_the_backup_path() {
    let table = build(&[artifact(
        "tempmon-rivet",
        "tempmon",
        "api/v1/backup",
        BackupFormat::Sqlite,
    )]);
    assert_eq!(table[0].request_path(), "/tempmon/api/v1/backup");
}

/// A client that refused the whole file over one bad row would fetch nothing
/// from any application on the machine.
#[test]
fn a_malformed_line_is_skipped_and_the_rest_still_load() {
    let text = "tempmon-rivet tempmon artifact sqlite api/v1/backup\n\
                this line is nonsense\n\
                \n\
                # a comment\n\
                reliquary-rivet reliquary feed api/v1/blobs\n";
    let table = parse(text);
    assert_eq!(table.len(), 2);
}

/// The reader enforces the same grammar the manifest does, so a table written
/// by something other than `render` cannot introduce a route that escapes its
/// own prefix.
#[test]
fn a_row_that_could_escape_its_prefix_is_dropped_on_read() {
    for hostile in [
        "svc ../etc artifact sqlite api/v1/backup",
        "svc tempmon artifact sqlite ../../../state/app",
        "svc tempmon artifact sqlite /api/v1/backup",
        "svc tempmon artifact sqlite api/v1/backup/",
        "svc tempmon feed ..",
        "svc a/b artifact sqlite x",
    ] {
        assert!(
            parse(hostile).is_empty(),
            "{hostile:?} must not become a declaration"
        );
    }
}

/// **The service field is a directory component on the reading end**
/// (`cmd/cargo-rivet`), so a row naming one that is not a checked service
/// name is dropped exactly like any other malformed row — never kept
/// verbatim on the theory that a route and a path were enough to check.
#[test]
fn a_row_whose_service_is_not_a_checked_name_is_dropped_on_read() {
    for hostile in ["../../outside", "/etc/x", "C:\\x", "C:/x", "a/b", "a\\b", "..", "."] {
        let line = format!("{hostile} tempmon artifact csv api/v1/backup");
        assert!(
            parse(&line).is_empty(),
            "{hostile:?} must not become a declaration's service"
        );
    }
    for valid in ["photo-index", "aura-renderer"] {
        let line = format!("{valid} tempmon artifact csv api/v1/backup");
        let table = parse(&line);
        assert_eq!(table.len(), 1, "{valid:?} is a real service name and must still be read");
        assert_eq!(table[0].service, valid);
    }
}

/// A reader cannot know how many fields follow a word it does not define, so
/// an unknown shape word takes the line with it.
#[test]
fn an_unknown_shape_word_drops_the_row_rather_than_guessing() {
    for hostile in [
        "svc tempmon stream sqlite api/v1/backup",
        "svc tempmon ARTIFACT sqlite api/v1/backup",
        "svc tempmon artifact parquet api/v1/backup",
        "svc tempmon artifact api/v1/backup",
        "svc tempmon feed",
        "svc tempmon feed api/v1/blobs extra",
        "svc tempmon artifact sqlite api/v1/backup extra",
    ] {
        assert!(
            parse(hostile).is_empty(),
            "{hostile:?} must not become a declaration"
        );
    }
}

// --- the restore line — ADR-051 §12 -----------------------------------------

#[test]
fn a_restore_route_is_its_own_line_and_parses_back_onto_its_artifact() {
    let table = build(&[
        restorable("tempmon-rivet", "tempmon", BackupFormat::Sqlite, "api/v1/restore"),
        artifact("notes-rivet", "notes", "api/backup", BackupFormat::Json),
        feed("reliquary-rivet", "reliquary", "api/v1/blobs"),
    ]);
    let text = render(&table);
    assert!(
        text.contains(
            "tempmon-rivet tempmon artifact sqlite api/v1/backup\n\
             tempmon-rivet tempmon restore api/v1/restore\n"
        ),
        "{text}"
    );
    assert_eq!(text.matches(" restore ").count(), 1, "{text}");
    let back = parse(&text);
    assert_eq!(back, table);
    let tempmon = back.iter().find(|d| d.service == "tempmon-rivet").expect("tempmon");
    assert_eq!(
        tempmon.restore_request_path().as_deref(),
        Some("/tempmon/api/v1/restore")
    );
    let notes = back.iter().find(|d| d.service == "notes-rivet").expect("notes");
    assert_eq!(notes.restore_request_path(), None);
}

/// **The compatibility argument, as bytes.** A reader built before `restore`
/// existed drops an unknown shape word and keeps every other line. This
/// replays that reader's rule — the arm list as it stood — over a table this
/// build writes, and the backup declaration must survive it.
#[test]
fn a_reader_that_does_not_know_restore_still_sees_the_backup() {
    let text = render(&build(&[restorable(
        "tempmon-rivet",
        "tempmon",
        BackupFormat::Sqlite,
        "api/v1/restore",
    )]));
    let old_reader_keeps: Vec<&str> = text
        .lines()
        .filter(|line| matches!(line.split(' ').nth(2), Some(ARTIFACT | FEED)))
        .collect();
    assert_eq!(
        old_reader_keeps,
        vec!["tempmon-rivet tempmon artifact sqlite api/v1/backup"],
        "{text}"
    );
    // And the line itself still has exactly the five fields an old reader
    // requires — a sixth would have dropped the declaration whole.
    assert_eq!(old_reader_keeps[0].split(' ').count(), 5);
}

/// A restore line attaches to its artifact whichever order the lines are in.
#[test]
fn a_restore_line_before_its_artifact_still_attaches() {
    let table = parse(
        "tempmon-rivet tempmon restore api/v1/restore\n\
         tempmon-rivet tempmon artifact sqlite api/v1/backup\n",
    );
    assert_eq!(table.len(), 1);
    assert_eq!(table[0].surface.restore_path(), Some("api/v1/restore"));
}

/// **A restore that cannot be attached unambiguously is dropped, and the
/// backup is kept.** Where a restore is sent must not depend on which line a
/// reader met first, and a malformed restore must not cost the backup.
#[test]
fn a_restore_line_that_cannot_attach_is_dropped_and_the_backup_is_kept() {
    for (text, why) in [
        (
            "svc tempmon artifact sqlite api/v1/backup\nother tempmon restore api/v1/restore\n",
            "another service's restore",
        ),
        (
            "svc tempmon artifact sqlite api/v1/backup\nsvc elsewhere restore api/v1/restore\n",
            "the same service on another route",
        ),
        (
            "svc tempmon artifact sqlite api/v1/backup\nsvc tempmon restore a/b\nsvc tempmon restore c/d\n",
            "two restore routes for one service",
        ),
        (
            "svc tempmon artifact sqlite api/v1/backup\nsvc tempmon restore ../escape\n",
            "a restore path that escapes the route",
        ),
        (
            "svc tempmon artifact sqlite api/v1/backup\nsvc tempmon restore api/v1/restore extra\n",
            "a trailing field",
        ),
        (
            "svc tempmon artifact sqlite api/v1/backup\nsvc tempmon restore\n",
            "no path",
        ),
    ] {
        let table = parse(text);
        assert_eq!(table.len(), 1, "{why}: the backup must be kept");
        assert_eq!(table[0].surface.restore_path(), None, "{why}: must not attach");
    }
    for (text, why) in [
        ("svc reliquary feed api/v1/blobs\nsvc reliquary restore api/v1/restore\n", "a feed"),
        ("svc tempmon restore api/v1/restore\n", "no artifact at all"),
    ] {
        let table = parse(text);
        assert!(
            table.iter().all(|d| d.surface.restore_path().is_none()),
            "{why}: a restore line must attach to nothing"
        );
    }
}

// --- the ladder --------------------------------------------------------------

/// A SQLite header for a database of `pages` pages of `page_size` bytes, and
/// the bytes to go with it.
///
/// **Built here rather than taken from a fixture** so that the page count and
/// the length can be made to disagree, which is the whole point of rung 4.
fn sqlite_of(page_size: u16, pages: u32) -> Vec<u8> {
    let mut out = vec![0u8; page_size as usize * pages as usize];
    out[..16].copy_from_slice(b"SQLite format 3\0");
    out[16..18].copy_from_slice(&page_size.to_be_bytes());
    out[28..32].copy_from_slice(&pages.to_be_bytes());
    out
}

fn ok(format: BackupFormat, bytes: &[u8]) -> Report {
    validate(format, bytes, Some(bytes.len() as u64)).expect("a file this test meant to be whole")
}

#[test]
fn a_whole_sqlite_database_climbs_every_rung() {
    let db = sqlite_of(4096, 8);
    let report = ok(BackupFormat::Sqlite, &db);
    assert_eq!(
        report.rungs,
        [Rung::NonEmpty, Rung::Length, Rung::Magic, Rung::Internal]
    );
    assert!(report.proved_completeness());
    assert_eq!(report.bytes, 4096 * 8);
}

/// **The trap the whole ladder exists for.** The file still begins
/// `SQLite format 3\0`, so rung 3 passes; only the header's own arithmetic
/// catches it.
#[test]
fn a_truncated_sqlite_passes_the_magic_and_is_caught_by_the_length_claim() {
    let db = sqlite_of(4096, 8);
    let half = &db[..db.len() / 2];
    assert!(
        half.starts_with(b"SQLite format 3\0"),
        "the premise of this test is that rung 3 still passes"
    );
    let fault = validate(BackupFormat::Sqlite, half, Some(half.len() as u64))
        .expect_err("a half database is not a database");
    assert_eq!(
        fault,
        Fault::Incomplete {
            claimed: 4096 * 8,
            received: 4096 * 4
        }
    );
}

/// **Rung 2 is what catches a connection cut mid-transfer**, and
/// it runs before the format is looked at at all — so it answers for CSV, which
/// has no other rung.
#[test]
fn fewer_bytes_than_the_server_declared_is_refused_for_every_format() {
    for format in BackupFormat::ALL {
        let fault = validate(*format, b"abc", Some(9))
            .expect_err("three bytes against a declared nine");
        assert_eq!(
            fault,
            Fault::Truncated {
                received: 3,
                declared: 9
            }
        );
    }
}

/// A response with no declared length has no rung 2, and for CSV nothing after
/// it applies — so the fetch is refused rather than stored on the strength of
/// nothing.
#[test]
fn a_response_with_no_content_length_is_refused_rather_than_trusted() {
    for format in BackupFormat::ALL {
        assert_eq!(
            validate(*format, b"anything", None).expect_err("no length"),
            Fault::NoLength
        );
    }
}

#[test]
fn an_empty_body_is_refused_for_every_format() {
    for format in BackupFormat::ALL {
        assert_eq!(
            validate(*format, b"", Some(0)).expect_err("empty"),
            Fault::Empty
        );
    }
}

/// **CSV gets two rungs and the report says so**, rather than one word that
/// would mean the same thing as a SQLite database's four.
#[test]
fn csv_climbs_two_rungs_and_does_not_claim_completeness() {
    let report = ok(BackupFormat::Csv, b"when,celsius\n2026-09-08,21.5\n");
    assert_eq!(report.rungs, [Rung::NonEmpty, Rung::Length]);
    assert!(
        !report.proved_completeness(),
        "CSV has no statement about its own size and must not claim one"
    );
}

/// A file that is not the declared format is refused at rung 3, whatever else
/// is true of it.
#[test]
fn a_file_that_is_not_the_declared_format_is_refused() {
    let db = sqlite_of(4096, 2);
    for (format, bytes) in [
        (BackupFormat::Sqlite, b"not a database at all, but long".as_slice()),
        (BackupFormat::Tar, db.as_slice()),
        (BackupFormat::Json, b"42".as_slice()),
        (BackupFormat::Json, b"\"a bare string is legal JSON\"".as_slice()),
    ] {
        assert_eq!(
            validate(format, bytes, Some(bytes.len() as u64)).expect_err("wrong format"),
            Fault::NotTheFormat { format },
            "{format} accepted something that is not one"
        );
    }
}

/// A tar with no end-of-archive blocks is exactly the shape a cut transfer
/// leaves behind, and it still carries `ustar`.
#[test]
fn a_tar_missing_its_end_of_archive_blocks_is_refused() {
    let mut tar = vec![0u8; 512 * 4];
    tar[257..262].copy_from_slice(b"ustar");
    tar[0..5].copy_from_slice(b"data\0");
    // Whole and correct: header, one data block, two zero blocks.
    let report = ok(BackupFormat::Tar, &tar);
    assert!(report.proved_completeness());

    // Now cut the terminator off. `ustar` is still at 257.
    let cut = &tar[..512 * 2];
    assert_eq!(&cut[257..262], b"ustar");
    assert!(
        matches!(
            validate(BackupFormat::Tar, cut, Some(cut.len() as u64)),
            Err(Fault::Incomplete { .. })
        ),
        "a tar with no terminator is a partial archive"
    );

    // And one cut mid-block, which is not even a whole number of blocks.
    let ragged = &tar[..512 * 3 + 100];
    assert!(matches!(
        validate(BackupFormat::Tar, ragged, Some(ragged.len() as u64)),
        Err(Fault::Incomplete { .. })
    ));
}

#[test]
fn a_json_document_that_stops_mid_object_is_refused() {
    let whole = br#"{"readings":[1,2,3]}"#;
    assert!(ok(BackupFormat::Json, whole).proved_completeness());
    let cut = br#"{"readings":[1,2"#;
    assert!(matches!(
        validate(BackupFormat::Json, cut, Some(cut.len() as u64)),
        Err(Fault::Incomplete { .. })
    ));
    // An array closes with `]`, and a `}` where one was expected is a real
    // byte that is the wrong one — nothing here is missing, so a full syntax
    // check refuses it as `NotTheFormat` rather than `Incomplete`.
    let mismatched = br#"[1,2,3}"#;
    assert_eq!(
        validate(BackupFormat::Json, mismatched, Some(mismatched.len() as u64)),
        Err(Fault::NotTheFormat {
            format: BackupFormat::Json
        })
    );
}

/// **Every case a full JSON syntax check exists to catch**, matched against
/// the specific reason each one is refused: [`Fault::Incomplete`] where the
/// scanner ran out of bytes with a container still open — the same shape as a
/// stream cut mid record — and [`Fault::NotTheFormat`] everywhere else, a
/// real byte that is present and wrong.
#[test]
fn a_full_syntax_check_refuses_every_way_a_json_body_can_be_wrong() {
    let incomplete: &[&[u8]] = &[
        // The finding's own example: truncated mid-array, mid-object.
        br#"{"records":[{"a":1}"#,
        b"[1,2",
        // The string itself never closes.
        br#"{"a":"b"#,
    ];
    for bad in incomplete {
        assert!(
            matches!(
                validate(BackupFormat::Json, bad, Some(bad.len() as u64)),
                Err(Fault::Incomplete { .. })
            ),
            "{bad:?} must be Incomplete"
        );
    }

    let not_the_format: &[&[u8]] = &[
        br#"{"a":}"#,          // no value after `:`
        br#"{"a" 1}"#,         // no `:` after the key
        br#"{"a":"\q"}"#,      // `\q` is not a JSON escape
        br#"{"a":1}x"#,        // data after the top level closed
        br#"{"a":1}{"b":2}"#,  // a second document
        b"[01]",               // a leading zero is a complete number on its own
    ];
    for bad in not_the_format {
        assert_eq!(
            validate(BackupFormat::Json, bad, Some(bad.len() as u64)),
            Err(Fault::NotTheFormat {
                format: BackupFormat::Json
            }),
            "{bad:?}"
        );
    }

    // Invalid UTF-8 fails whatever its brackets look like.
    let invalid_utf8: &[u8] = &[b'[', b'"', 0xFF, b'"', b']'];
    assert_eq!(
        validate(BackupFormat::Json, invalid_utf8, Some(invalid_utf8.len() as u64)),
        Err(Fault::NotTheFormat {
            format: BackupFormat::Json
        })
    );

    // A bare scalar is legal JSON and still not a backup of anything — rung 3,
    // unchanged, and rung 4 never runs.
    let scalar = br#""just a string""#;
    assert_eq!(
        validate(BackupFormat::Json, scalar, Some(scalar.len() as u64)),
        Err(Fault::NotTheFormat {
            format: BackupFormat::Json
        })
    );
}

/// **Nothing a full syntax check should refuse.** Nested containers, strings
/// carrying the very characters that are structural everywhere else, empty
/// objects and arrays, and trailing whitespace — all whole JSON, and every
/// one of them still climbs to `Rung::Internal`.
#[test]
fn a_full_syntax_check_still_passes_everything_whole() {
    // Built by concatenation, with the escape's six ASCII bytes spelled as an
    // ordinary (non-raw) byte string, so the source keeps the literal JSON
    // escape for U+00E9 rather than the one character it names.
    let unicode_escape: Vec<u8> = [
        br#"{"a":"{}[]\"quoted\"","b":""#.as_slice(),
        b"\\u00e9",
        br#""}"#.as_slice(),
    ]
    .concat();

    let good: Vec<&[u8]> = vec![
        br#"{"a":{"b":[1,2,{"c":[]}]}}"#,
        br#"[{"a":1},{"b":2}]"#,
        // `{}[]` unescaped inside the string, an escaped quote pair, and a
        // `\u` escape for U+00E9.
        &unicode_escape,
        b"{}",
        b"[]",
        b"{}\n  \t\r\n",
        b"  \n{\"a\":1}\n",
    ];
    for one in good {
        assert!(ok(BackupFormat::Json, one).proved_completeness(), "{one:?}");
    }
}

/// **A hundred thousand levels, and no stack overflow** — the whole reason
/// this is an explicit `Vec` rather than recursive descent over
/// [`crate::json`], which caps at [`crate::json::MAX_DEPTH`] for exactly the
/// opposite reason.
#[test]
fn deep_nesting_does_not_overflow_the_stack() {
    let depth = 100_000;
    let deep = "[".repeat(depth) + &"]".repeat(depth);
    assert!(ok(BackupFormat::Json, deep.as_bytes()).proved_completeness());
}

/// **SQLite's escape for a 65536-byte page.** A reader taking the `1` literally
/// computes a length 65,535 times too small and refuses every database written
/// with the largest page size.
#[test]
fn a_page_size_of_one_means_sixty_five_thousand_five_hundred_and_thirty_six() {
    // `sqlite_of` cannot build this one — it would allocate one byte per page —
    // so it is assembled directly.
    let mut real = vec![0u8; 65_536 * 2];
    real[..16].copy_from_slice(b"SQLite format 3\0");
    real[16..18].copy_from_slice(&1u16.to_be_bytes());
    real[28..32].copy_from_slice(&2u32.to_be_bytes());
    assert!(ok(BackupFormat::Sqlite, &real).proved_completeness());
}

/// A page count of zero is what SQLite before 3.7.0 wrote, and it is not a
/// claim about anything — so rung 4 does not run rather than passing vacuously.
#[test]
fn a_sqlite_with_no_page_count_climbs_three_rungs_and_claims_no_completeness() {
    let mut db = sqlite_of(4096, 4);
    db[28..32].copy_from_slice(&0u32.to_be_bytes());
    let report = ok(BackupFormat::Sqlite, &db);
    assert_eq!(report.rungs, [Rung::NonEmpty, Rung::Length, Rung::Magic]);
    assert!(!report.proved_completeness());
}
