//! **The migrations are what these test**, because they are the part that runs
//! against a file somebody has already got.
//!
//! [`super::load`] and [`super::save`] are not tested here: both need
//! `RIVET_APP_DATA`, which is a process-wide environment variable, and a test
//! that sets one races every other test in the binary. What they do beyond
//! resolving that path is read a file and parse it, and [`super::migrate`] —
//! which is the part with rules in it — takes its input by argument for exactly
//! this reason.

use super::*;

#[test]
fn a_default_config_needs_no_migration() {
    let mut cfg = Config::default();
    let before = cfg.clone();
    let notes = migrate(&mut cfg, "{}");
    assert!(notes.is_empty(), "a fresh config was migrated: {notes:?}");
    assert_eq!(cfg, before);
}

/// The round trip is the thing the portal depends on: it writes what it read,
/// and a field that did not survive would be a setting silently reset.
#[test]
fn a_config_survives_being_written_and_read_back() {
    let cfg = Config {
        brightness: 35,
        zip_code: "20500".to_string(),
        screensaver: "matrix".to_string(),
        screensaver_theme: "ice_blue".to_string(),
        geo_cache: Some(GeoCache {
            zip: "20500".to_string(),
            lat: 38.90,
            lng: -77.04,
        }),
        schedule: vec![ScheduledEvent {
            time: "22:30".to_string(),
            days: vec!["Mon".to_string(), "Fri".to_string()],
            actions: vec![ScheduledAction {
                kind: "brightness".to_string(),
                value: "20".to_string(),
                theme: String::new(),
            }],
        }],
        ..Config::default()
    };
    let text = serde_json::to_string(&cfg).expect("a config renders");
    let back: Config = serde_json::from_str(&text).expect("what we wrote, we read");
    assert_eq!(back, cfg);
}

/// **Reporting the key is what makes the caller save, and saving is what
/// removes it.** A secret merely ignored is a secret still on the disk.
#[test]
fn a_retired_portal_password_is_reported_so_that_the_file_gets_rewritten() {
    let mut cfg = Config::default();
    let notes = migrate(&mut cfg, r#"{"portal_password":"hunter2","brightness":60}"#);
    assert!(
        notes.iter().any(|n| n.starts_with("portal_password:")),
        "the retired password was not reported: {notes:?}"
    );
}

/// The machine owns the zone now. A file still carrying Aura's own copy must be
/// told, or somebody edits it and watches nothing happen.
#[test]
fn a_retired_timezone_is_reported() {
    let mut cfg = Config::default();
    let notes = migrate(&mut cfg, r#"{"timezone":"America/Chicago"}"#);
    assert!(
        notes.iter().any(|n| n.starts_with("timezone:")),
        "the retired timezone was not reported: {notes:?}"
    );
}

/// The CEC pair went to the machine, and a file still carrying it must be told
/// where — otherwise somebody edits `cec_enabled` and waits for a television.
#[test]
fn the_retired_cec_keys_are_reported_and_say_where_the_setting_went() {
    let mut cfg = Config::default();
    let notes = migrate(&mut cfg, r#"{"cec_enabled":true,"cec_hdmi_input":1}"#);
    let note = notes
        .iter()
        .find(|n| n.starts_with("cec_enabled"))
        .unwrap_or_else(|| panic!("the retired CEC keys were not reported: {notes:?}"));
    assert!(note.contains("[display]"), "{note}");
    assert!(note.contains("ADR-039"), "{note}");
}

/// A schedule carrying a `cec` action fires nothing, so it is dropped and said
/// so rather than left to look like a setting that works.
#[test]
fn a_retired_cec_action_is_dropped_and_says_where_the_setting_went() {
    let mut cfg = Config {
        schedule: vec![ScheduledEvent {
            time: "19:00".to_string(),
            days: Vec::new(),
            actions: vec![
                ScheduledAction {
                    kind: "cec".to_string(),
                    value: "off".to_string(),
                    theme: String::new(),
                },
                ScheduledAction {
                    kind: "brightness".to_string(),
                    value: "20".to_string(),
                    theme: String::new(),
                },
            ],
        }],
        ..Config::default()
    };
    let notes = migrate(&mut cfg, "{}");
    assert_eq!(cfg.schedule[0].actions.len(), 1);
    assert_eq!(cfg.schedule[0].actions[0].kind, "brightness");
    let note = notes
        .iter()
        .find(|n| n.contains("\"cec\""))
        .unwrap_or_else(|| panic!("the dropped CEC action was not reported: {notes:?}"));
    assert!(note.contains("ADR-039"), "{note}");
}

/// A key this build does not know must not stop the file being read — that is
/// what lets an older controller and a newer config coexist during a rollback.
#[test]
fn an_unknown_key_is_ignored_rather_than_refused() {
    let cfg: Config = serde_json::from_str(r#"{"brightness":42,"something_new":true}"#)
        .expect("an unknown key is not a parse failure");
    assert_eq!(cfg.brightness, 42);
}

#[test]
fn a_retired_action_is_dropped_and_reported() {
    let mut cfg = Config {
        schedule: vec![ScheduledEvent {
            time: "07:00".to_string(),
            days: Vec::new(),
            actions: vec![
                ScheduledAction {
                    kind: "weather_text".to_string(),
                    value: "on".to_string(),
                    theme: String::new(),
                },
                ScheduledAction {
                    kind: "brightness".to_string(),
                    value: "80".to_string(),
                    theme: String::new(),
                },
            ],
        }],
        ..Config::default()
    };
    let notes = migrate(&mut cfg, "{}");
    assert_eq!(cfg.schedule.len(), 1, "the event itself was removed");
    assert_eq!(cfg.schedule[0].actions.len(), 1);
    assert_eq!(cfg.schedule[0].actions[0].kind, "brightness");
    assert!(
        notes.iter().any(|n| n.contains("weather_text")),
        "the dropped action was not reported: {notes:?}"
    );
}

/// An event whose every action was retired would sit in the file firing
/// nothing, which is worse than being gone and having said so.
#[test]
fn an_event_left_with_no_actions_is_removed() {
    let mut cfg = Config {
        schedule: vec![ScheduledEvent {
            time: "07:00".to_string(),
            days: Vec::new(),
            actions: vec![ScheduledAction {
                kind: "weather_text".to_string(),
                value: "on".to_string(),
                theme: String::new(),
            }],
        }],
        ..Config::default()
    };
    let notes = migrate(&mut cfg, "{}");
    assert!(cfg.schedule.is_empty());
    assert!(
        notes.iter().any(|n| n.contains("no actions left")),
        "the removed event was not reported: {notes:?}"
    );
}

/// **The reading source's settings are the addon's, and the host leaves them
/// exactly as they were saved** — `.rulings/` R-375 D1. A band whose ends are
/// crossed is the addon's to refuse in its own words; a host that repaired it
/// would have to know which key is a low end, which is what it must not.
#[test]
fn the_reading_sources_settings_are_never_rewritten_by_a_migration() {
    let mut saved = serde_json::Map::new();
    saved.insert("low".to_string(), serde_json::json!(9.0));
    saved.insert("high".to_string(), serde_json::json!(1.0));
    saved.insert("unit".to_string(), serde_json::json!("furlongs"));
    let mut cfg = Config {
        reading_settings: saved.clone(),
        ..Config::default()
    };
    let notes = migrate(&mut cfg, "{}");
    assert_eq!(cfg.reading_settings, saved);
    assert!(notes.is_empty(), "{notes:?}");
}

/// **An empty sensor nickname is a choice, not a hole** — it means *the first
/// sensor TempMon reports* — so a load never writes a name over it.
#[test]
fn an_empty_indoor_sensor_is_left_empty() {
    let mut cfg = Config {
        indoor_sensor: String::new(),
        ..Config::default()
    };
    let notes = migrate(&mut cfg, "{}");
    assert_eq!(cfg.indoor_sensor, "");
    assert!(notes.is_empty(), "{notes:?}");
}

/// **A fresh install names no room.** Any default nickname would be somebody's
/// room; empty selects the first sensor reported, which is right for a machine
/// with one and a place to start for a machine with several.
#[test]
fn the_default_indoor_sensor_names_nothing() {
    assert_eq!(Config::default().indoor_sensor, "");
    assert_eq!(DEFAULT_INDOOR_SENSOR, "");
}

/// **A fresh install's reading card is on, and every addon field goes out as
/// its own default** — the host holds no default of its own for any of them.
#[test]
fn a_fresh_reading_card_is_shown_and_carries_no_settings_of_its_own() {
    let cfg = Config::default();
    assert!(cfg.reading_enabled);
    assert!(cfg.reading_settings.is_empty());
}

/// The port's own default, and the assertion exists to catch it being
/// *corrected* back to the address the Go original hard-codes by somebody
/// reading its source.
///
/// **HTTPS, not HTTP** — `.rulings/` R-216: TempMon on a Rivet serves only
/// HTTPS on this port, so an `http://` default could never actually connect.
#[test]
fn the_tempmon_default_is_this_board_and_not_the_machine_being_replaced() {
    assert_eq!(Config::default().tempmon_url, "https://127.0.0.1:8083");
}

/// **`.rulings/` R-216: the changed default is a compile-time constant, not a
/// migration.** A configuration already holding the OLD (`http://`) default —
/// non-empty, so [`migrate`]'s own `is_empty` guard already protected it —
/// must survive untouched. This pins that directly against the exact
/// regression the ruling forbids, rather than trusting the guard by
/// inspection.
#[test]
fn migrate_never_rewrites_an_already_saved_tempmon_url() {
    let mut cfg = Config { tempmon_url: "http://127.0.0.1:8081".to_string(), ..Config::default() };
    let notes = migrate(&mut cfg, "{}");
    assert_eq!(
        cfg.tempmon_url, "http://127.0.0.1:8081",
        "an already-saved tempmon_url must never be rewritten to the new default"
    );
    assert!(notes.iter().all(|n| !n.contains("tempmon_url")), "{notes:?}");
}

// ---- url_is_loopback — `.rulings/` R-219 -----------------------------------

/// **Every loopback spelling this program is expected to meet** — bare, with
/// a port, and with a path, for each of the three forms the acceptance
/// criteria names — reads as loopback, and a handful of everyday non-loopback
/// URLs read as exactly that and nothing else.
#[test]
fn every_loopback_spelling_is_recognised_with_and_without_a_port_or_a_path() {
    for url in [
        "https://127.0.0.1:8081",
        "https://127.0.0.1",
        "https://127.0.0.1:8081/status",
        "https://127.5.6.7:8081",
        "http://localhost:8081",
        "http://localhost",
        "http://localhost/status",
        "https://LOCALHOST:8081",
        "https://[::1]:8081",
        "https://[::1]",
        "https://[::1]:8081/status",
    ] {
        assert!(url_is_loopback(url), "{url} should be loopback");
    }
    for url in [
        "https://rivet-01.local:8081",
        "http://192.168.50.52:8080",
        "https://192.168.50.52",
        "https://127.0.0.1.example.com:8081",
        "https://127.0.1:8081",
    ] {
        assert!(!url_is_loopback(url), "{url} should not be loopback");
    }
}

/// **Nonsense is not loopback.** A URL malformed enough to confuse the parser
/// will already fail to connect on its own account, so the refusal here is to
/// invent a machine identity rather than to guess one.
#[test]
fn an_unparseable_url_is_not_loopback() {
    assert!(!url_is_loopback(""));
    assert!(!url_is_loopback("not a url at all"));
}

// ---- url_is_plain_http / tempmon_name_is_asked — `.rulings/` R-219 ---------

/// **`http://` is recognised case-insensitively**, matching every other
/// place a scheme is compared in this file.
#[test]
fn a_plain_http_url_is_recognised_regardless_of_case() {
    assert!(url_is_plain_http("http://192.168.1.5:8081"));
    assert!(url_is_plain_http("HTTP://192.168.1.5:8081"));
    assert!(url_is_plain_http("Http://192.168.1.5:8081"));
}

#[test]
fn an_https_url_is_not_plain_http() {
    assert!(!url_is_plain_http("https://192.168.1.5:8081"));
}

/// A string too short to carry a scheme, or with none at all, is not plain
/// HTTP — `url_is_plain_http` must not panic on it.
#[test]
fn a_url_too_short_for_a_scheme_is_not_plain_http() {
    assert!(!url_is_plain_http(""));
    assert!(!url_is_plain_http("http"));
}

/// **The one predicate the field's visibility and the name's use both read
/// from.** Plain HTTP and loopback each turn it off; anything else turns it
/// on.
#[test]
fn tempmon_name_is_asked_is_false_for_plain_http_and_for_loopback_and_true_otherwise() {
    assert!(!tempmon_name_is_asked("http://192.168.1.5:8081"));
    assert!(!tempmon_name_is_asked("https://127.0.0.1:8081"));
    assert!(tempmon_name_is_asked("https://192.168.1.5:8081"));
}

// ---- effective_tempmon_name — `.rulings/` R-219 ----------------------------

/// **An operator's own value is never second-guessed, for another
/// machine** — not even one that happens to look like it could have been
/// auto-filled, and not by asking the machine anything at all.
#[test]
fn a_configured_name_is_returned_unchanged_for_a_non_loopback_host() {
    assert_eq!(
        effective_tempmon_name("https://rivet-01.local:8081", "rivet-08ecc.local"),
        Ok("rivet-08ecc.local".to_string())
    );
}

/// **A plain `http://` URL needs no name at all — there is no certificate on
/// the wire to verify one against.** `Ok`, not `Err`: this is not a question
/// that failed to answer, it is not a question in the first place, so an
/// operator who never saved one is never told anything is missing.
#[test]
fn a_plain_http_url_needs_no_name_even_with_none_saved() {
    assert_eq!(effective_tempmon_name("http://192.168.1.5:8081", ""), Ok(String::new()));
}

/// **Checked before loopback.** A plain-HTTP loopback URL needs no name
/// either, and a stale name saved from some earlier setup is ignored exactly
/// as it is for HTTPS loopback — proven here by giving it one and getting
/// back empty regardless.
#[test]
fn a_plain_http_loopback_url_also_needs_no_name() {
    assert_eq!(
        effective_tempmon_name("http://127.0.0.1:8081", "stale-name.local"),
        Ok(String::new())
    );
}

/// **`HTTP://` counts as `http://`.** The scheme comparison is
/// case-insensitive, matching [`url_is_plain_http`]'s own test.
#[test]
fn an_uppercase_http_scheme_also_needs_no_name() {
    assert_eq!(
        effective_tempmon_name("HTTP://192.168.1.5:8081", "some-name.local"),
        Ok(String::new())
    );
}

/// **Empty, for another machine, refuses and says to enter that machine's
/// own name** — never this board's own, which is certainly wrong for a
/// certificate that is not this board's.
#[test]
fn an_empty_name_for_a_non_loopback_host_refuses_and_names_the_other_machine() {
    let err = effective_tempmon_name("https://rivet-01.local:8081", "")
        .expect_err("an empty name for another machine must refuse");
    assert!(err.contains("another machine"), "{err}");
}

/// **A saved name is never even READ for a loopback URL — proven by
/// contrast.** The exact same `configured` value that the test above returns
/// unchanged for another machine is not what a loopback host returns here;
/// no supervisor answers a test process, so this refuses exactly as
/// `an_empty_name_on_loopback_refuses_with_no_machine_to_ask` does below — a
/// NON-empty stale name refuses the very same way, which it would not if it
/// had been read at all.
#[test]
fn a_loopback_url_ignores_a_saved_name_even_a_stale_one() {
    let err = effective_tempmon_name("https://127.0.0.1:8081", "some-other-machine.local")
        .expect_err("no supervisor answers a test process");
    // **Not the weaker `contains("own name")`.** Both refusal messages
    // happen to share those two words ("this machine's OWN NAME could not
    // be read" / "enter that machine's OWN NAME"), so that check alone
    // cannot tell the loopback branch actually ran rather than the
    // non-loopback one reading `configured` as before. "could not be read"
    // is the loopback message's own, and only its own.
    assert!(err.contains("could not be read"), "{err}");
}

/// **Refuse rather than guess, proven rather than asserted.** No supervisor
/// answers a test process, so `rivet_sdk::facts::ask` genuinely fails here —
/// this is the real refusal path, not a restated rule.
#[test]
fn an_empty_name_on_loopback_refuses_with_no_machine_to_ask() {
    let err = effective_tempmon_name("https://127.0.0.1:8081", "")
        .expect_err("no supervisor answers a test process");
    // See `a_loopback_url_ignores_a_saved_name_even_a_stale_one`'s own note:
    // "could not be read" is what is unique to this branch's message.
    assert!(err.contains("could not be read"), "{err}");
}

/// **The two refusals read differently**, so a person — or a test — can tell
/// which one fired without inspecting which branch ran.
#[test]
fn the_two_refusal_messages_are_distinguishable() {
    let loopback = effective_tempmon_name("https://127.0.0.1:8081", "").unwrap_err();
    let elsewhere = effective_tempmon_name("https://rivet-01.local:8081", "").unwrap_err();
    assert_ne!(loopback, elsewhere);
}

// ---- The backup surface, `ADR-051` §12 -------------------------------------

/// **What `GET /backup` writes, `POST /restore` reads back unchanged.**
///
/// The whole contract in one property. `backup-format = "json"` promises that
/// what comes out of the one route goes into the other, and this is what makes
/// that true rather than nearly true: a field that serialised but did not
/// deserialise would lose a setting on restore and the page would look right
/// afterwards, because the default it fell back to is a plausible value.
#[test]
fn a_backup_restores_to_the_configuration_it_was_taken_from() {
    let mut reading_settings = serde_json::Map::new();
    reading_settings.insert("unit".to_string(), serde_json::json!("ft"));
    reading_settings.insert("low".to_string(), serde_json::json!(1.5));
    reading_settings.insert("show_trend".to_string(), serde_json::json!(true));
    let original = Config {
        zip_code: "20500".to_string(),
        brightness: 73,
        screensaver: "matrix".to_string(),
        screensaver_theme: "ice_blue".to_string(),
        indoor_sensor: "Sensor B".to_string(),
        tempmon_name: "rivet-01.local".to_string(),
        reading_enabled: false,
        reading_settings,
        weather_contact: "ops@example.com".to_string(),
        ..Config::default()
    };

    let backup = serde_json::to_string_pretty(&original).expect("a backup renders");
    let restored: Config = serde_json::from_str(&backup).expect("a backup parses");

    assert_eq!(
        restored, original,
        "a setting did not survive the round trip, which on a board would be a \
         restore that silently reset it to its default"
    );
}

/// **A backup taken before a setting existed is still a usable backup.**
///
/// `Config` carries `#[serde(default)]`, which is the same tolerance
/// `config::load` gives a file written by an older build. Without it, adding
/// any setting would make every backup taken before that day un-restorable —
/// which is exactly when a person reaches for one.
#[test]
fn a_backup_missing_a_key_restores_that_key_to_its_default() {
    let older = r#"{"zip_code":"20500","brightness":50}"#;
    let restored: Config = serde_json::from_str(older).expect("an older backup parses");

    assert_eq!(restored.zip_code, "20500");
    assert_eq!(restored.brightness, 50);
    assert_eq!(
        restored.tempmon_url,
        Config::default().tempmon_url,
        "a key the backup never had should arrive as its default"
    );
    assert_eq!(
        restored.weather_contact, "",
        "a file from before the weather contact existed sends no contact"
    );
    assert!(restored.reading_enabled, "an older backup shows the reading card");
    assert!(
        restored.reading_settings.is_empty(),
        "an older backup carries no reading settings, so every field goes out as its default"
    );
}

/// **A document that is not a configuration is refused, not coerced.**
///
/// `crate::portal`'s restore route parses before it writes anything, so this
/// is the failure that keeps a machine unchanged. JSON that happens to be
/// valid but is not this shape must not become a `Config` of defaults —
/// restoring a stranger's file would then silently wipe every setting.
#[test]
fn json_that_is_not_a_configuration_is_refused() {
    assert!(
        serde_json::from_str::<Config>("[1, 2, 3]").is_err(),
        "an array became a Config"
    );
    assert!(
        serde_json::from_str::<Config>("\"hello\"").is_err(),
        "a string became a Config"
    );
    assert!(
        serde_json::from_str::<Config>("not json at all").is_err(),
        "unparseable text became a Config"
    );
}
