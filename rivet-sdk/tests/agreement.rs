//! **`rivet-sdk` re-declares; this proves it still agrees** — `ADR-036` §8.3.
//!
//! An application is a foreign program and must not link the operating system,
//! so `rivet_sdk` cannot import `rivet_core`'s constants — it restates them. The
//! cost of restating is drift, and this file is the whole of what buys the right
//! to do it.
//!
//! `rivetos-core` is a **dev-dependency** of `rivetos-sdk` and appears nowhere
//! else. Dev-dependencies are not linked into consumers, so an application that
//! depends on `rivetos-sdk` never sees `rivet_core`; the edge exists only here.
//!
//! # Why the encoding is tested and not just the constants
//!
//! Because the two fail differently. A drifted **path** fails loudly — a socket
//! that is not there, a directory that cannot be opened. A drifted **encoding**
//! does not fail at all: `journal::ingest` turns a line it cannot parse into a
//! `warn` record carrying the raw bytes, so every record would quietly become a
//! complaint about itself and the log would still look busy.
//!
//! That is the failure this file exists for, and it is why §8.3 was widened from
//! *"assert the constants match"* to *"assert a record round-trips"*.

// A test asserts; `unwrap`, `expect` and `panic!` ARE the assertion. The same
// allowance `rivet-core`'s own test modules carry, spelled at file scope
// because an integration test is not a `#[cfg(test)] mod`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use rivet_sdk::journal::{Journal, Severity, Value};

/// The paths and variables a Rivet promises. A wrong one here is an application
/// writing to somewhere nobody is looking.
#[test]
fn every_re_declared_constant_matches_the_platform() {
    assert_eq!(
        rivet_sdk::app::APP_DIR_ENV,
        rivet_core::service::APP_DIR_ENV
    );
    assert_eq!(
        rivet_sdk::app::APP_DATA_ENV,
        rivet_core::service::APP_DATA_ENV
    );
    assert_eq!(
        rivet_sdk::app::CONNECT_ENV,
        rivet_core::service::CONNECT_ENV
    );
    assert_eq!(
        rivet_sdk::app::APP_SETTINGS_ENV,
        rivet_core::service::APP_SETTINGS_ENV
    );
    assert_eq!(
        rivet_sdk::journal::INGEST_SOCKET_PATH,
        rivet_core::journal::INGEST_SOCKET_PATH
    );
    assert_eq!(
        rivet_sdk::journal::INGEST_MAX_BYTES,
        rivet_core::journal::INGEST_MAX_BYTES
    );
}

/// **The three system typefaces, and the one directory a confined application
/// may read them from** — `ADR-042` §4.2, added 2026-09-05.
///
/// These constants were re-declared like every other, and the re-declaration
/// became load-bearing the day `rivet_core::confinement` started granting
/// `rivet_core::etc::SYSTEM_FONT_DIR` and nothing else under `/system`. An SDK
/// that named a face outside that directory would hand every kiosk application
/// a path the kernel refuses — which is exactly the regression of 2026-09-05,
/// where the launcher printed *"NO SYSTEM FONT — Permission denied"* and drew a
/// screen with no text on it.
///
/// Behind `cfg(feature = "draw")` because that is where `rivet_sdk::text` is;
/// `cargo test -p rivetos-sdk --features draw` is what runs it, and
/// `tools/test-all.sh` does.
#[cfg(feature = "draw")]
#[test]
fn the_typeface_paths_agree_and_are_inside_the_granted_directory() {
    assert_eq!(rivet_sdk::text::SYSTEM_FONT, rivet_core::etc::SYSTEM_FONT);
    assert_eq!(rivet_sdk::text::SYSTEM_MONO, rivet_core::etc::SYSTEM_MONO);
    assert_eq!(rivet_sdk::text::ICON_FONT, rivet_core::etc::ICON_FONT);
    for face in [
        rivet_sdk::text::SYSTEM_FONT,
        rivet_sdk::text::SYSTEM_MONO,
        rivet_sdk::text::ICON_FONT,
    ] {
        assert!(
            face.starts_with(&format!("{}/", rivet_core::etc::SYSTEM_FONT_DIR)),
            "{face} is outside rivet_core::confinement's FONT_DIR grant, so every application \
             that asks this SDK to load it is refused with EACCES on a confined kernel"
        );
    }
}

/// **The compositor's socket, which `crate::composited` spells by hand** —
/// `ADR-058` §5.
///
/// The SDK cannot import `rivet_core::compositor::socket_path`, so it restates
/// the path, and this is what buys the right to. A drift here is the loud kind
/// — an application connects to nothing and says so — but it is loud **on a
/// board**, after a card build and an install, which is the most expensive
/// place in this project to find out anything.
///
/// Behind the feature because the module is: with `composited` off there is
/// nothing to disagree with.
#[cfg(feature = "composited")]
#[test]
fn the_compositors_socket_is_the_one_the_supervisor_binds() {
    assert_eq!(
        rivet_sdk::composited::SOCKET,
        rivet_core::compositor::socket_path(),
        "the SDK connects somewhere the supervisor does not bind"
    );
}

/// The severity vocabulary, both directions.
///
/// A severity the platform does not know is parsed as `None` there, so an SDK
/// that invented one would emit records the journal silently refused to grade.
#[test]
fn the_severity_vocabulary_is_the_same_on_both_sides() {
    assert_eq!(
        rivet_sdk::journal::Severity::ALL.len(),
        rivet_core::journal::Severity::ALL.len()
    );
    for sev in Severity::ALL {
        let parsed = rivet_core::journal::Severity::parse(sev.as_str());
        assert!(
            parsed.is_some(),
            "the platform does not recognise the SDK's {:?}",
            sev.as_str()
        );
    }
    // And the other direction, so neither side can grow a level alone.
    for sev in rivet_core::journal::Severity::ALL {
        assert!(
            Severity::parse(sev.as_str()).is_some(),
            "the SDK does not recognise the platform's {:?}",
            sev.as_str()
        );
    }
}

/// **The central test.** A record the SDK writes must parse back through the
/// platform's own reader to the same values.
#[test]
fn a_record_the_sdk_writes_parses_back_identically() {
    let journal = Journal::for_service("demo").with_correlation("corr-1");
    let line = journal.line(
        Severity::Warn,
        "a message with \"quotes\", a \\backslash and a \nnewline",
        &[
            ("count", Value::Int(-7)),
            ("ratio", Value::Float(0.5)),
            ("ok", Value::Bool(true)),
            ("name", Value::Str("sensor \u{1}one".to_string())),
        ],
    );

    let record = rivet_core::journal::Record::from_jsonl(&line)
        .unwrap_or_else(|why| panic!("the platform could not read the SDK's record: {why:?}\n{line}"));

    assert_eq!(record.sev, rivet_core::journal::Severity::Warn);
    assert_eq!(record.svc, "demo");
    assert_eq!(record.corr, "corr-1");
    assert_eq!(
        record.msg,
        "a message with \"quotes\", a \\backslash and a \nnewline"
    );

    let field = |name: &str| {
        record
            .fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    };
    assert_eq!(field("count"), Some(rivet_core::journal::Value::Int(-7)));
    assert_eq!(field("ok"), Some(rivet_core::journal::Value::Bool(true)));
    assert_eq!(
        field("name"),
        Some(rivet_core::journal::Value::Str("sensor \u{1}one".to_string()))
    );
    match field("ratio") {
        Some(rivet_core::journal::Value::Float(f)) => assert!((f - 0.5).abs() < f64::EPSILON),
        other => panic!("ratio came back as {other:?}"),
    }
}

/// Every severity, not just the one the test above happened to use.
#[test]
fn every_severity_survives_the_round_trip() {
    let journal = Journal::for_service("demo");
    for sev in Severity::ALL {
        let line = journal.line(sev, "m", &[]);
        let record = rivet_core::journal::Record::from_jsonl(&line)
            .unwrap_or_else(|why| panic!("{sev:?} did not parse: {why:?}"));
        assert_eq!(record.sev.as_str(), sev.as_str());
    }
}

/// A record with no fields is the common case and must not lose a column.
#[test]
fn the_minimal_record_still_carries_every_required_column() {
    let line = Journal::for_service("demo").line(Severity::Info, "", &[]);
    assert!(
        rivet_core::journal::Record::from_jsonl(&line).is_ok(),
        "an empty message dropped a column: {line}"
    );
}

/// The SDK must not emit a name the journal composes itself — the key would be
/// on the line twice, and RFC 8259 lets a parser resolve that either way.
#[test]
fn no_reserved_name_the_platform_uses_is_emittable_by_the_sdk() {
    for name in rivet_core::journal::INGEST_FIELD_NAMES {
        assert!(
            rivet_sdk::journal::RESERVED_FIELD_NAMES.contains(&name),
            "the platform composes {name:?} and the SDK would let an application set it"
        );
    }

    // And it actually refuses, rather than merely listing them.
    let line = Journal::for_service("demo").line(
        Severity::Info,
        "m",
        &[("ingest_pid", Value::Int(1)), ("emitted_corr", Value::Int(2))],
    );
    let record = rivet_core::journal::Record::from_jsonl(&line).expect("still valid JSON");
    assert!(record.fields.iter().all(|(n, _)| n != "ingest_pid"));
    assert!(record.fields.iter().all(|(n, _)| n != "emitted_corr"));
}

/// The endpoint format is `owner:name=path`, space-separated — and the platform
/// is the side that writes it.
#[test]
fn the_endpoint_format_matches_what_the_platform_renders() {
    let granted = vec![
        (
            "aura".to_string(),
            "kiosk".to_string(),
            "/run/rivet/connect/aura/kiosk".to_string(),
        ),
        (
            "photo-index".to_string(),
            "api".to_string(),
            "/run/rivet/connect/photo-index/api".to_string(),
        ),
    ];
    let rendered = rivet_core::service::connect_env(&granted)
        .expect("two granted endpoints render to a value");

    let parsed = rivet_sdk::app::parse_endpoints(&rendered);
    assert_eq!(parsed.len(), 2, "parsed {rendered:?} into {parsed:?}");
    assert_eq!(parsed[0].owner, "aura");
    assert_eq!(parsed[0].name, "kiosk");
    assert_eq!(
        parsed[0].path,
        std::path::PathBuf::from("/run/rivet/connect/aura/kiosk")
    );
    assert_eq!(parsed[1].owner, "photo-index");
    assert_eq!(parsed[1].name, "api");
}

/// Nothing granted renders to `None`, not to an empty string — and the SDK's
/// reader must agree that nothing is not the same as empty.
#[test]
fn nothing_granted_is_absent_on_both_sides() {
    assert!(rivet_core::service::connect_env(&[]).is_none());
    assert!(rivet_sdk::app::parse_endpoints("").is_empty());
}

/// **The display window, both halves of it** — `ADR-039` §8.3.
///
/// The SDK re-declares the path, the `HH:MM` parse and the midnight-wrap
/// predicate, and every one of the three can drift silently. The path fails
/// loudly if it drifts — a file that is not there — but **the predicate does
/// not fail at all**: a wrong wrap keeps a building lit for twelve hours or
/// dark for twelve hours, on a machine nobody is standing in front of after
/// seven in the evening. So it is asserted at every minute of the day rather
/// than at a sample.
#[test]
fn the_display_hours_path_is_the_one_the_supervisor_writes() {
    assert_eq!(
        rivet_sdk::schedule::HOURS_PATH,
        rivet_core::display::HOURS_PATH,
        "an application would read a file the machine does not write"
    );
    // **Under /run/rivet and NOT under /etc**, asserted rather than left to a
    // comment: /etc is a compatibility surface for C runtimes that demand an
    // exact path, and the first build of this put the window there. ADR-039
    // §3.1; `cmd/rivet-sysd/src/runtime.rs` carries the rule.
    assert!(
        rivet_sdk::schedule::HOURS_PATH.starts_with("/run/rivet/"),
        "the window belongs in Rivet's runtime namespace"
    );
    assert!(!rivet_sdk::schedule::HOURS_PATH.starts_with("/etc/"));
}

#[test]
fn both_sides_parse_a_time_of_day_the_same_way() {
    let cases = [
        "00:00", "07:00", "12:34", "19:00", "23:59", // valid
        "7:00", "07:0", "24:00", "23:60", "25:00", "0700", "", ":", "ab:cd", " 07:00",
    ];
    for text in cases {
        let sdk = rivet_sdk::schedule::TimeOfDay::parse(text);
        let core = rivet_core::display::TimeOfDay::parse(text);
        assert_eq!(
            sdk.is_some(),
            core.is_some(),
            "{text:?}: one side accepted it and the other refused"
        );
        if let (Some(sdk), Some(core)) = (sdk, core) {
            assert_eq!(
                sdk.minutes_since_midnight(),
                core.minutes_since_midnight(),
                "{text:?}: the two sides read a different minute"
            );
        }
    }
}

#[test]
fn both_sides_agree_on_whether_the_screen_is_lit_at_every_minute() {
    for (off, on) in [
        ("19:00", "07:00"), // crosses midnight — the case this was built for
        ("02:00", "07:00"), // does not
        ("00:00", "12:00"),
        ("12:00", "00:00"),
        ("23:59", "00:00"),
        ("00:00", "23:59"),
    ] {
        let parse = |t: &str| {
            (
                rivet_sdk::schedule::TimeOfDay::parse(t).expect("sdk"),
                rivet_core::display::TimeOfDay::parse(t).expect("core"),
            )
        };
        let (sdk_off, core_off) = parse(off);
        let (sdk_on, core_on) = parse(on);
        let sdk = rivet_sdk::schedule::Hours::new(sdk_off, sdk_on).expect("sdk window");
        let core = rivet_core::display::Hours::new(core_off, core_on).expect("core window");
        for minute in 0..24 * 60u16 {
            let (h, m) = ((minute / 60) as u8, (minute % 60) as u8);
            let sdk_now = rivet_sdk::schedule::TimeOfDay::new(h, m).expect("sdk minute");
            let core_now = rivet_core::display::TimeOfDay::new(h, m).expect("core minute");
            assert_eq!(
                sdk.is_on(sdk_now),
                core.is_on(core_now),
                "{off}/{on} at {h:02}:{m:02}: the two sides disagree about a lit screen"
            );
        }
    }
    // A window of no width is refused identically, so neither side can be
    // handed one the other would have rejected.
    let sdk_t = rivet_sdk::schedule::TimeOfDay::new(19, 0).expect("sdk");
    let core_t = rivet_core::display::TimeOfDay::new(19, 0).expect("core");
    assert!(rivet_sdk::schedule::Hours::new(sdk_t, sdk_t).is_none());
    assert!(rivet_core::display::Hours::new(core_t, core_t).is_none());
}

/// **The file the supervisor writes is the file the SDK reads.**
///
/// The supervisor composes this in `cmd/rivet-sysd/src/runtime.rs` and cannot be
/// linked from here — an application never sees the operating system — so the
/// FORMAT is restated and asserted, which is the same bargain `ADR-036` §8.3
/// strikes for every other constant in this file.
#[test]
fn the_projected_file_round_trips_from_the_supervisors_format_to_the_reader() {
    for (off, on) in [("19:00", "07:00"), ("02:00", "07:00"), ("00:00", "12:00")] {
        let written = format!(
            "# Generated by Rivet from the `[display]` table of the configuration document.\n\
             # Do not edit: this is a view of the document, never an interface (ADR-007 §3.6.2).\n\
             # The hours this screen is DARK, off first, in this machine's local time.\n\
             {off} {on}\n"
        );
        let read = rivet_sdk::schedule::Hours::parse(&written)
            .expect("the SDK should parse what the supervisor writes");
        assert_eq!(read.off().to_string(), off);
        assert_eq!(read.on().to_string(), on);
        // And the core type agrees about the same two times, so the document
        // half and the application half cannot drift apart.
        let core = rivet_core::display::Hours::new(
            rivet_core::display::TimeOfDay::parse(off).expect("off"),
            rivet_core::display::TimeOfDay::parse(on).expect("on"),
        )
        .expect("a valid window");
        assert_eq!(core.off().to_string(), off);
        assert_eq!(core.on().to_string(), on);
    }
}

/// **The settings file, both ends of it** — `ADR-038` §8.
///
/// `rivet_core::settings::render` composes what the supervisor writes;
/// `rivet_sdk::settings::parse` is what the application reads, restated because
/// an application cannot link the operating system. A drift between them does
/// not fail loudly: the application reads FEWER settings than it was given and
/// behaves as though an operator never set them, which is exactly the silent
/// failure the whole declaration mechanism exists to prevent.
#[test]
fn the_file_the_supervisor_writes_is_the_file_the_application_reads() {
    let declared: Vec<String> = ["brightness", "background", "show-weather", "a", "slot-2"]
        .into_iter()
        .map(String::from)
        .collect();
    let values: Vec<(String, String)> = [
        ("brightness", "60"),
        ("background", "digital-rain"),
        ("show-weather", "true"),
        // The awkward ones: an equals sign, spaces at both ends, an empty
        // value, and a lone character.
        ("a", "x=y z=w"),
        ("slot-2", "  padded  "),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();

    let (text, rejected) = rivet_core::settings::render(&declared, &values);
    assert!(rejected.is_empty(), "{rejected:?}");

    let read = rivet_sdk::settings::parse(&text);
    assert_eq!(read.len(), values.len(), "a setting was lost in transit: {text}");
    for (key, value) in &values {
        assert_eq!(
            read.get(key).map(String::as_str),
            Some(value.as_str()),
            "{key} arrived as something else"
        );
    }

    // And the core's own reader agrees with the SDK's, key for key - so the
    // two restatements cannot drift apart while both agreeing with a third.
    let core_read: std::collections::BTreeMap<String, String> =
        rivet_core::settings::parse(&text).into_iter().collect();
    assert_eq!(core_read, read);
}

/// An empty delivery reads as an empty set on both sides — the state of every
/// machine nobody has configured.
#[test]
fn a_delivery_with_nothing_in_it_reads_as_nothing_on_both_sides() {
    let (text, rejected) = rivet_core::settings::render(&[], &[]);
    assert!(rejected.is_empty());
    assert!(rivet_sdk::settings::parse(&text).is_empty());
    assert!(rivet_core::settings::parse(&text).is_empty());
}

/// **Both sides agree on what a setting NAME is.** The SDK restates
/// `is_setting_name`, and a disagreement would mean the supervisor writing a
/// line the application skips - a setting that is set, delivered, and silently
/// not read.
#[test]
fn both_sides_agree_on_what_a_setting_name_is() {
    for text in [
        "brightness", "show-weather", "slot-2", "a",
        "", "Brightness", "brightness_level", "brightness.level", "2slots", "-leading",
        "with space", "caf\u{e9}", "has=equals",
    ] {
        assert_eq!(
            rivet_sdk::settings::is_setting_name(text),
            rivet_core::package::application::is_setting_name(text),
            "{text:?}: the two sides disagree about whether that is a name"
        );
    }
}

/// The filename is the same on both sides, or the supervisor writes a file
/// nothing opens.
#[test]
fn the_settings_file_has_one_name() {
    assert_eq!(rivet_sdk::settings::FILE, rivet_core::settings::FILE);
}

// **The Bluetooth agreement is in `src/bluetooth.rs`, not here, and the reason
// is a fact about cargo rather than a preference.**
//
// `ADR-041`'s module is behind the optional `bluetooth` feature, and an
// INTEGRATION test links the library as an ordinary consumer would — with
// whatever features the build turned on, which under `cargo test --workspace`
// is none. So an agreement test written in this file would not compile with the
// feature off and would be silently absent with it on by accident, which is the
// worst of both.
//
// A UNIT test inside the crate does compile it: `lib.rs` gates the module on
// `any(feature = "bluetooth", test)`, exactly as it gates `serve`, and
// `rivetos-core` is a dev-dependency so a unit test can reach the platform's
// own encoder. The assertions are therefore in that module, under the same
// names they would have had here — `the_endpoint_is_spelled_the_same_on_both_sides`,
// `a_line_the_platform_writes_decodes_here_to_the_same_values`, and
// `every_word_in_the_wire_vocabulary_survives_the_crossing`.
/// **The application is TOLD where its settings are, and the path it is told is
/// the one the supervisor wrote** — `ADR-038` §8.1, 2026-09-04.
///
/// The two ends used to agree by composition: the supervisor wrote
/// `data_dir/settings` and the SDK joined `FILE` onto `RIVET_APP_DATA`, so the
/// agreement was two spellings of one layout. The file has now moved out of that
/// directory, into a place nothing hands an application, so the agreement is on
/// the VARIABLE — and this is what makes reading it here mean something rather
/// than being a restatement.
#[test]
fn the_variable_an_application_reads_names_the_file_the_supervisor_writes() {
    let name = rivet_core::service::ServiceName::new("aura-renderer").expect("a name");
    let volume = rivet_core::service::BUILT_IN_VOLUME;
    let written = rivet_core::settings::path(volume, &name);

    // What the supervisor puts in the environment is exactly this path — see
    // `cmd/rivet-sysd`'s `applications::definition`, which is asserted there.
    // What THIS end must not do is compose one of its own.
    assert!(
        written.ends_with(&format!("/{}", rivet_sdk::settings::FILE)),
        "{written}"
    );
    assert!(
        !written.starts_with(&rivet_core::service::data_dir(volume, &name)),
        "{written} is still inside the directory the application owns, and an SDK that joined \
         the filename onto RIVET_APP_DATA would still have worked - which is how this move gets \
         silently undone"
    );

}

/// **A health line the SDK writes is one the supervisor's parser reads back
/// identically** — `ADR-036` §8.3's round-trip rule, applied to `ADR-045`'s
/// wire form.
///
/// The SDK re-declares the vocabulary because it links no Rivet crate, so this
/// is the whole of what makes that safe. It asserts the ROUND TRIP rather than
/// the string, for the reason §8.3 gives about the journal record: a drifted
/// constant fails loudly and a drifted encoding does not — a health line the
/// supervisor cannot parse is one an application believes it sent.
#[test]
fn every_health_report_the_sdk_can_write_parses_as_what_it_meant() {
    let cases = [
        (rivet_sdk::health::Reported::Healthy, rivet_core::health::Reported::Healthy),
        (rivet_sdk::health::Reported::Degraded, rivet_core::health::Reported::Degraded),
        (rivet_sdk::health::Reported::Down, rivet_core::health::Reported::Down),
    ];
    for (sdk, core) in cases {
        for caption in [None, Some("no broadcast from GVH5075_2222 in 34 minutes")] {
            let line = rivet_sdk::health::line(sdk, caption);
            let value = line
                .strip_prefix(&format!("{}=", rivet_sdk::health::KEY))
                .unwrap_or_else(|| panic!("{line} does not start with the key the core expects"));
            let (state, text) = rivet_core::health::parse(value)
                .unwrap_or_else(|e| panic!("the supervisor could not read {line:?}: {e}"));
            assert_eq!(state, core, "{line}");
            assert_eq!(text.as_deref(), caption, "{line}");
        }
    }
}

/// **The two spellings of the caption cap are one number.**
///
/// A larger cap on the SDK side would let an application write a sentence the
/// supervisor then truncates a second time — which is not a failure, but it
/// means the length an author was told is not the length that survives.
#[test]
fn the_caption_cap_is_the_same_number_on_both_sides() {
    assert_eq!(
        rivet_sdk::health::CAPTION_MAX_BYTES,
        rivet_core::health::CAPTION_MAX_BYTES
    );
    assert_eq!(rivet_sdk::health::KEY, rivet_core::health::KEY);
}

/// **The reserved channel name and the request are one string each** —
/// `ADR-045` §3.
///
/// This is the drift that would be silent. An SDK looking up a name the
/// supervisor does not hand out finds no socket and answers nothing, which
/// `get status` reports as `unknown` — an application that appears wedged for a
/// reason that is a typo. And a supervisor asking a question the SDK's answering
/// loop does not recognise is the same failure from the other end.
#[test]
fn the_answering_channel_is_the_same_name_and_the_same_question_on_both_sides() {
    assert_eq!(
        rivet_sdk::health::CHANNEL_NAME,
        rivet_core::health::CHANNEL_NAME
    );
    assert_eq!(rivet_sdk::health::REQUEST, rivet_core::health::REQUEST);
}

/// **An answer the SDK writes fits inside the bound the supervisor reads it
/// with**, caption and all — or the supervisor would cut a line the SDK
/// considered whole.
#[test]
fn the_longest_answer_the_sdk_can_write_fits_the_supervisors_bound() {
    let longest = rivet_sdk::health::line(
        rivet_sdk::health::Reported::Degraded,
        Some(&"x".repeat(rivet_sdk::health::CAPTION_MAX_BYTES * 2)),
    );
    assert!(
        longest.len() + 1 < rivet_core::health::ANSWER_MAX_BYTES,
        "an answer of {} bytes against a bound of {}",
        longest.len() + 1,
        rivet_core::health::ANSWER_MAX_BYTES
    );
}

/// **The secret-naming rule, both sides** — `ADR-038` §5, `.rulings/` R-169.
///
/// `rivet_sdk::secrets` restates `rivet_core::operation::is_secret_name`
/// rather than importing it, so this is what buys the restatement: a name
/// this SDK would compose a path for and the platform's own `install-secret`
/// would have refused is a defect that shows itself as a file that silently
/// never exists, not as a loud disagreement.
#[test]
fn both_sides_agree_on_what_a_secrets_name_is() {
    for text in [
        "relay.env", "write-token", "a", "A1", "cert.pem", "x",
        "", ".", "..", "-leading", "has/slash", "has\\backslash", "has space",
        "caf\u{e9}", &"x".repeat(64), &"x".repeat(65),
    ] {
        assert_eq!(
            rivet_sdk::secrets::is_secret_name(text),
            rivet_core::operation::is_secret_name(text),
            "{text:?}: the two sides disagree about whether that is a secret's name"
        );
    }
}

/// The cap is one number, not two — a caller told 64 by this SDK must not be
/// refused at 32 by the machine it eventually reaches.
#[test]
fn the_secret_name_length_cap_is_the_same_number_on_both_sides() {
    assert_eq!(
        rivet_sdk::secrets::NAME_MAX_LEN,
        rivet_core::operation::SECRET_NAME_MAX_LEN
    );
}

/// **The mode is one number too.** `secrets::install` writes a secret file
/// from inside the application; `apply install-secret` writes the same file
/// from the control plane. A mode that differed would make one of the two
/// writers produce a file the other's reader could not read back, or leave a
/// credential wider than the platform intends.
#[test]
fn the_secret_file_mode_is_the_same_on_both_sides() {
    assert_eq!(
        rivet_sdk::secrets::MODE,
        rivet_core::operation::SECRET_MODE
    );
}

/// The size cap `secrets::install` enforces is the one the operator's channel
/// enforces, so a value the dialog accepts is one the platform would have.
#[test]
fn the_secret_size_cap_is_the_same_on_both_sides() {
    assert_eq!(
        rivet_sdk::secrets::MAX_BYTES,
        rivet_core::operation::SECRET_MAX_BYTES
    );
}

/// **An over-long caption survives the round trip as the same shorter
/// sentence**, cut once rather than twice and cut in the same place.
#[test]
fn an_over_long_caption_is_cut_the_same_way_on_both_sides() {
    let long = "é".repeat(300);
    let line = rivet_sdk::health::line(rivet_sdk::health::Reported::Degraded, Some(&long));
    let value = line.strip_prefix("HEALTH=").expect("the key");
    let (_, text) = rivet_core::health::parse(value).expect("a health line");
    let text = text.expect("a caption");
    let (core_cut, _) = rivet_core::health::truncate_caption(&long);
    assert_eq!(
        text, core_cut,
        "the two ends cut a long sentence in different places"
    );
}

/// **The facts call's constants** — `ADR-057`. A wrong socket path here is an
/// application talking to nothing and concluding that the supervisor has
/// stopped, which is the one wrong answer this whole surface exists to avoid.
#[test]
fn the_facts_call_names_the_same_socket_and_speaks_the_same_protocol() {
    assert_eq!(
        rivet_sdk::facts::SOCKET_PATH,
        rivet_core::facts::SOCKET_PATH
    );
    assert_eq!(rivet_sdk::facts::REQUEST, rivet_core::facts::REQUEST);
    assert_eq!(rivet_sdk::facts::PROTOCOL, rivet_core::facts::PROTOCOL);
    assert_eq!(
        rivet_sdk::facts::ANSWER_MAX_BYTES,
        rivet_core::facts::ANSWER_MAX_BYTES
    );
    assert_eq!(
        rivet_sdk::supervisor::UPTIME_PATH,
        rivet_core::confinement::UPTIME_PATH
    );
}

/// **The request this SDK sends is the one the supervisor takes.**
///
/// Asserted through the machine's own reader rather than by comparing two
/// strings, because that is the failure worth catching: a request the
/// supervisor refuses comes back as nothing at all, and nothing at all is how
/// an application concludes the supervisor has wedged. A drifted request would
/// therefore make every dashboard on the machine report a failure that has not
/// happened.
#[test]
fn the_request_the_sdk_sends_is_the_one_the_supervisor_answers() {
    assert_eq!(
        rivet_core::facts::read_request(rivet_sdk::facts::REQUEST),
        Ok(())
    );
}

/// **An answer the supervisor renders is an answer this SDK reads, field for
/// field.**
///
/// The round trip `ADR-036` §8 asks for, and the reason it is the encoding
/// and not the constants: a drifted path fails loudly, and a drifted format
/// does not fail at all — a dashboard would draw one field's number under
/// another field's label and be wrong with complete confidence.
#[test]
fn an_answer_the_machine_renders_is_read_back_field_for_field() {
    let machine = rivet_core::facts::Facts {
        protocol: rivet_core::facts::PROTOCOL,
        hostname: Some("rivet-06f5c".to_string()),
        version: Some("0.1.92-g1234abcd".to_string()),
        slot: Some("b".to_string()),
        phase: "converging".to_string(),
        machine: rivet_core::health::State::Degraded,
        mode: rivet_core::operation::status::MODE_KIOSK.to_string(),
        view_age_ms: 212,
        services: vec![
            rivet_core::facts::ServiceFact {
                name: "aura-controller".to_string(),
                enabled: true,
                state: "started".to_string(),
                pid: Some(678),
                restarts: 0,
                ready: false,
                health: rivet_core::health::State::Healthy,
                health_age_ms: Some(4_120),
            },
            // The service nothing has ever answered for: `unknown` with no age
            // beside it, which must not arrive as `0` at the other end.
            rivet_core::facts::ServiceFact {
                name: "rivet-netd".to_string(),
                enabled: false,
                state: "idle".to_string(),
                pid: None,
                restarts: 3,
                ready: true,
                health: rivet_core::health::State::Unknown,
                health_age_ms: None,
            },
        ],
        truncated: 0,
    };

    let read = rivet_sdk::facts::parse(&machine.render()).expect("the SDK reads the machine");

    assert_eq!(read.protocol, machine.protocol);
    assert_eq!(read.hostname, machine.hostname);
    assert_eq!(read.version, machine.version);
    assert_eq!(read.slot, machine.slot);
    assert_eq!(read.phase, machine.phase);
    assert_eq!(read.mode, machine.mode);
    assert_eq!(read.view_age_ms, machine.view_age_ms);
    assert_eq!(read.truncated, machine.truncated);
    assert_eq!(read.machine.word(), machine.machine.word());
    assert_eq!(read.services.len(), machine.services.len());
    for (sdk, core) in read.services.iter().zip(machine.services.iter()) {
        assert_eq!(sdk.name, core.name);
        assert_eq!(sdk.enabled, core.enabled);
        assert_eq!(sdk.state, core.state);
        assert_eq!(sdk.pid, core.pid);
        assert_eq!(sdk.restarts, core.restarts);
        assert_eq!(sdk.ready, core.ready);
        assert_eq!(sdk.health.word(), core.health.word());
        assert_eq!(
            sdk.health_age_ms, core.health_age_ms,
            "an absent age must not arrive as zero, which reads as `it answered this instant`"
        );
    }
}

/// **The four health words are the same four, spelled the same way.**
///
/// The SDK has a three-word `Reported` for what a service says about itself
/// and a four-word `facts::Health` for what an observer concludes about one,
/// and only the second can say `unknown`. A word that drifted would make a
/// dashboard refuse an answer it should have drawn.
#[test]
fn the_observers_four_health_words_are_the_machines_four() {
    use rivet_core::health::State;
    use rivet_sdk::facts::Health;
    for (sdk, core) in [
        (Health::Healthy, State::Healthy),
        (Health::Degraded, State::Degraded),
        (Health::Down, State::Down),
        (Health::Unknown, State::Unknown),
    ] {
        assert_eq!(sdk.word(), core.word());
    }
}
