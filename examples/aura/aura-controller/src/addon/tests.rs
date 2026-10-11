//! **What a person sees, pinned.**
//!
//! [`super::card`] and [`super::status_of`] are the whole of the translation
//! between a source that knows nothing about Aura and a card on a wall. Both
//! are pure, so every case is an assertion rather than something watched for
//! twenty minutes on a board.
//!
//! Nothing here opens an addon. [`super::Source`] needs a real object, a real
//! `RIVET_APP_DATA` and a real loader, and what it does with all three is the
//! first article `examples/probes/addon-probe-host` proved on hardware; what
//! is covered here is the part that would be wrong on the screen rather than
//! absent from it.

use super::*;

/// An arbitrary instant to measure ages from, in milliseconds.
const NOW_MS: i64 = 1_757_000_000_000;

/// **Hand-composed: a river gauge**, standing in for whatever a real reading
/// source measures. The host does not know what that is, and these tests are
/// written so that nothing in them could tell it either (`.rulings/` R-375 D1).
const METERS: &str = "m";
/// The same gauge, read in another unit — what a source answers in after it
/// has converted.
const FEET: &str = "ft";

fn reading_taken(seconds_ago: i64, unit: &str) -> Reading {
    Reading {
        value: 2.4,
        unit: unit.to_string(),
        taken_at_ms: NOW_MS - seconds_ago * 1_000,
        trend: Some(Trend::Flat),
        attention: false,
    }
}

fn polled(status: Status, reading: Option<Reading>) -> Polled {
    Polled {
        reading,
        status,
        call_again_in_ms: 300_000,
        complaint: String::new(),
        // A gauge measures one thing. `also` is for the sources that do not —
        // see `indoor` below.
        also: Vec::new(),
    }
}

/// A poll from a source that measured more than one thing.
///
/// `temp_f` is the role's own measurement and `humidity` rides in `also`,
/// which is the shape `aura-indoor-source` answers in.
fn polled_indoor(temp_f: f64, humidity: Option<f64>) -> Polled {
    let at = 1_789_529_336_720;
    Polled {
        reading: Some(Reading {
            value: temp_f,
            unit: "°F".to_string(),
            taken_at_ms: at,
            trend: None,
            attention: false,
        }),
        status: Status::Ok,
        call_again_in_ms: 60_000,
        complaint: String::new(),
        also: humidity
            .map(|pct| rivetos_addon::Measured {
                id: super::INDOOR_HUMIDITY.to_string(),
                reading: Reading {
                    value: pct,
                    unit: "%".to_string(),
                    taken_at_ms: at,
                    trend: None,
                    attention: false,
                },
            })
            .into_iter()
            .collect(),
    }
}

/// **The indoor column, drawn from the addon's answer.**
#[test]
fn the_indoor_card_carries_both_measurements() {
    let card = super::indoor_card(&polled_indoor(71.0, Some(59.0)), "Sensor B");
    assert!(card.online);
    assert_eq!(card.label, "Sensor B");
    assert_eq!(card.temp_f, Some(71.0));
    assert_eq!(card.humidity_pct, Some(59.0));
    assert_eq!(card.unit, "°F");
}

/// **A source that reports no humidity still draws a temperature.**
///
/// The panel shows what it has. An absent second measurement is ordinary — the
/// protocol's `also` is empty for every source that measures one thing — and
/// must not cost the first one.
#[test]
fn a_missing_humidity_does_not_cost_the_temperature() {
    let card = super::indoor_card(&polled_indoor(68.0, None), "Sensor A");
    assert!(card.online);
    assert_eq!(card.temp_f, Some(68.0));
    assert_eq!(card.humidity_pct, None);
}

/// **Humidity is found by id and never by position.**
///
/// A source that answered with something else first would otherwise put that
/// number under the humidity label — the failure an index would cause the
/// first time the order changed.
#[test]
fn humidity_is_matched_by_its_id_and_not_by_being_first() {
    let mut polled = polled_indoor(70.0, Some(50.0));
    polled.also.insert(
        0,
        rivetos_addon::Measured {
            id: "co2".to_string(),
            reading: Reading {
                value: 900.0,
                unit: "ppm".to_string(),
                taken_at_ms: 1_789_529_336_720,
                trend: None,
                attention: false,
            },
        },
    );
    let card = super::indoor_card(&polled, "Sensor B");
    assert_eq!(
        card.humidity_pct,
        Some(50.0),
        "the co2 reading came first and must not have been read as humidity"
    );
}

/// **Anything short of `Ok` draws nothing at all.**
///
/// A dead sensor and a perfectly stable room produce the same flat line, and
/// `online` is the only thing that tells them apart.
#[test]
fn an_unhealthy_indoor_poll_is_not_drawn() {
    for status in [Status::Stale, Status::Unavailable, Status::Faulted] {
        let mut polled = polled_indoor(71.0, Some(59.0));
        polled.status = status;
        let card = super::indoor_card(&polled, "Sensor B");
        assert!(!card.online, "{status:?} should not be drawn");
        assert_eq!(card.temp_f, None, "{status:?} must carry no number");
    }
}

/// **The mapping the maintainer asked for by name**, because it is the whole
/// of what a person sees: four statuses, four words, and a wrong one is a
/// working sensor drawn as a fault or a dead one drawn as fresh.
#[test]
fn each_status_maps_to_the_word_the_renderer_reads() {
    assert_eq!(status_of(Status::Ok), wire::status::OK);
    assert_eq!(status_of(Status::Stale), wire::status::STALE);
    assert_eq!(status_of(Status::Unavailable), wire::status::CONNECTING);
    assert_eq!(status_of(Status::Faulted), wire::status::ERROR);
}

/// The two that are easiest to collapse into each other, stated on their own:
/// a cold start is not a fault, and a fault is not a cold start.
#[test]
fn nothing_yet_and_will_not_fix_itself_are_different_words() {
    assert_ne!(
        status_of(Status::Unavailable),
        status_of(Status::Faulted),
        "a cold start must not announce itself as a fault"
    );
    assert_eq!(status_of(Status::Unavailable), "connecting");
    assert_eq!(status_of(Status::Faulted), "error");
}

#[test]
fn a_fresh_reading_is_drawn_with_its_value_unit_trend_and_age() {
    let card = card(
        &polled(Status::Ok, Some(reading_taken(90, METERS))),
        NOW_MS,
        "",
    );
    assert_eq!(card.status, wire::status::OK);
    assert_eq!(card.value, Some(2.4));
    assert_eq!(card.unit, METERS);
    assert_eq!(card.trend, "flat");
    assert_eq!(card.age_seconds, Some(90));
    assert!(!card.out_of_range);
}

/// The label comes from the READING when there is one, so a unit the addon
/// converted into cannot disagree with the unit the card is stamped with —
/// whatever unit a bare card was last labelled with.
#[test]
fn the_unit_drawn_is_the_one_the_addon_measured_in() {
    let card = card(
        &polled(Status::Ok, Some(reading_taken(0, FEET))),
        NOW_MS,
        METERS,
    );
    assert_eq!(card.unit, FEET, "the reading's unit wins");
}

/// **A cold start's bare card carries no unit at all**: there has been no
/// reading this run to take one from, and the host holds no unit of its own
/// for this role (`.rulings/` R-375 D1). The renderer draws the state's mark,
/// which needs none.
#[test]
fn a_cold_start_card_with_no_reading_carries_no_unit() {
    let card = card(&polled(Status::Unavailable, None), NOW_MS, "");
    assert_eq!(card.unit, "");
    assert_eq!(card.value, None);
    assert_eq!(card.status, wire::status::CONNECTING);
}

/// **Mid-run, a bare card is labelled with the unit of the last reading this
/// run saw** — the one label the host can know without being told what the
/// card shows.
#[test]
fn a_card_with_no_reading_is_labelled_with_the_last_unit_seen() {
    let card = card(&polled(Status::Faulted, None), NOW_MS, FEET);
    assert_eq!(card.unit, FEET);
    assert_eq!(card.value, None);
    assert_eq!(card.status, wire::status::ERROR);
}

/// **The remembered unit follows the readings and nothing else.** A card that
/// carried a value teaches its unit; a bare card teaches nothing, so the
/// remembered unit survives it — and a cold start remembers none.
#[test]
fn the_last_unit_is_learned_from_a_drawn_reading_and_kept_through_a_bare_card() {
    let mut last = String::new();

    let bare = card(&polled(Status::Unavailable, None), NOW_MS, &last);
    last = unit_after(&last, &bare);
    assert_eq!(last, "", "a cold start has seen no unit");

    let drawn = card(&polled(Status::Ok, Some(reading_taken(0, FEET))), NOW_MS, &last);
    last = unit_after(&last, &drawn);
    assert_eq!(last, FEET, "a drawn reading teaches its unit");

    let bare_again = card(&polled(Status::Faulted, None), NOW_MS, &last);
    assert_eq!(bare_again.unit, FEET, "the bare card after it is labelled with it");
    last = unit_after(&last, &bare_again);
    assert_eq!(last, FEET, "and a bare card does not unlearn it");

    let converted = card(&polled(Status::Ok, Some(reading_taken(0, METERS))), NOW_MS, &last);
    assert_eq!(unit_after(&last, &converted), METERS, "a new unit replaces the old one");

    // A value that came with no unit at all says nothing about the unit, so
    // the remembered one is kept rather than erased.
    let unitless = card(&polled(Status::Ok, Some(reading_taken(0, ""))), NOW_MS, &last);
    assert_eq!(unitless.unit, "");
    assert_eq!(unit_after(FEET, &unitless), FEET, "an empty unit does not unlearn one");
}

/// **The promise `wire::ReadingData` makes to the renderer**, kept on this
/// side rather than assumed of the addon: a renderer that ignored `status`
/// still could not draw a stale number.
#[test]
fn error_and_connecting_carry_no_number_even_when_one_is_offered() {
    for status in [Status::Faulted, Status::Unavailable] {
        let card = card(
            &polled(status, Some(reading_taken(0, METERS))),
            NOW_MS,
            METERS,
        );
        assert_eq!(card.value, None, "{status:?} drew a number");
        assert_eq!(card.age_seconds, None, "{status:?} drew an age");
        assert!(card.trend.is_empty(), "{status:?} drew an arrow");
    }
}

/// `attention` crosses as one bool and lands as `out_of_range`. What made the
/// reading worth marking is the addon's business and this program never learns
/// it.
#[test]
fn the_addons_verdict_becomes_the_out_of_range_mark() {
    let mut low = reading_taken(0, METERS);
    low.value = 0.3;
    low.attention = true;
    let card = card(&polled(Status::Ok, Some(low)), NOW_MS, METERS);
    assert!(card.out_of_range);
    assert_eq!(card.value, Some(0.3));
}

/// A stale reading is still a reading: the number and the arrow stay, and the
/// status is what says nothing newer has arrived.
#[test]
fn a_stale_reading_keeps_its_number() {
    let card = card(
        &polled(Status::Stale, Some(reading_taken(15 * 60, METERS))),
        NOW_MS,
        METERS,
    );
    assert_eq!(card.status, wire::status::STALE);
    assert_eq!(card.value, Some(2.4));
    assert_eq!(card.age_seconds, Some(15 * 60));
}

/// **A source whose clock is ahead of ours.** Not worth a failure; reporting
/// it as a large positive age would be a lie, and the lie would be on the wall.
#[test]
fn a_reading_from_the_future_is_zero_seconds_old_and_not_a_huge_number() {
    let ahead = reading_taken(-600, METERS);
    let card = card(&polled(Status::Ok, Some(ahead)), NOW_MS, METERS);
    assert_eq!(card.age_seconds, Some(0));
}

/// Seconds, not milliseconds — the schema's field is `age_seconds` and a
/// thousand-fold error here draws a two-minute-old reading as two days old.
#[test]
fn the_age_is_in_seconds() {
    let card = card(
        &polled(Status::Ok, Some(reading_taken(1, METERS))),
        NOW_MS,
        METERS,
    );
    assert_eq!(card.age_seconds, Some(1));
}

#[test]
fn every_trend_has_the_slug_the_renderer_maps() {
    assert_eq!(trend_of(Some(Trend::RisingQuickly)), "rising_fast");
    assert_eq!(trend_of(Some(Trend::Rising)), "rising");
    assert_eq!(trend_of(Some(Trend::Flat)), "flat");
    assert_eq!(trend_of(Some(Trend::Falling)), "falling");
    assert_eq!(trend_of(Some(Trend::FallingQuickly)), "falling_fast");
}

/// Three programs agreeing not to claim something: the source does not know,
/// the addon declines to invent a sixth variant, and the renderer draws no
/// arrow for `unknown`.
#[test]
fn an_unknown_trend_says_unknown_rather_than_guessing() {
    assert_eq!(trend_of(None), "unknown");
}

/// **Hand-composed: a gauge addon's `Describe` field set** — a unit Choice,
/// two Number thresholds, an account name, a Bool and a Secret. Invented for
/// this file; it is the shape that matters, and the host must handle any
/// shape without knowing what one key means.
fn gauge_fields() -> Vec<Field> {
    vec![
        Field {
            key: "unit".to_string(),
            label: "Unit".to_string(),
            help: String::new(),
            kind: FieldKind::Choice {
                default: METERS.to_string(),
                options: vec![
                    (METERS.to_string(), "Meters".to_string()),
                    (FEET.to_string(), "Feet".to_string()),
                ],
            },
        },
        Field {
            key: "low".to_string(),
            label: "Low".to_string(),
            help: String::new(),
            kind: FieldKind::Number {
                default: 0.5,
                min: 0.0,
                max: 10.0,
                unit: METERS.to_string(),
            },
        },
        Field {
            key: "high".to_string(),
            label: "High".to_string(),
            help: String::new(),
            kind: FieldKind::Number {
                default: 4.0,
                min: 0.0,
                max: 10.0,
                unit: METERS.to_string(),
            },
        },
        Field {
            key: "username".to_string(),
            label: "Username".to_string(),
            help: String::new(),
            kind: FieldKind::Text {
                default: String::new(),
                max_len: 254,
            },
        },
        Field {
            key: "show_trend".to_string(),
            label: "Show the trend".to_string(),
            help: String::new(),
            kind: FieldKind::Bool { default: true },
        },
        Field {
            key: "password".to_string(),
            label: "Password".to_string(),
            help: String::new(),
            kind: FieldKind::Secret {
                name: "gauge.password".to_string(),
            },
        },
    ]
}

/// **A fresh install sends every declared field at its own default** — the
/// host holds no default of its own for any of them (`.rulings/` R-375 D1).
#[test]
fn with_nothing_saved_every_field_goes_out_as_its_own_default() {
    let settings = settings_for(&gauge_fields(), &serde_json::Map::new());
    assert_eq!(settings.len(), 5, "five settings and no secret: {settings:?}");
    assert_eq!(settings["unit"], serde_json::json!(METERS));
    assert_eq!(settings["low"], serde_json::json!(0.5));
    assert_eq!(settings["high"], serde_json::json!(4.0));
    assert_eq!(settings["username"], serde_json::json!(""));
    assert_eq!(settings["show_trend"], serde_json::json!(true));
}

/// **What was saved is what is sent, untouched** — and a key the map lacks is
/// filled from `Describe`, never left as a hole the addon would have to guess
/// about.
#[test]
fn a_saved_value_is_sent_as_saved_and_a_missing_key_takes_its_default() {
    let mut saved = serde_json::Map::new();
    saved.insert("unit".to_string(), serde_json::json!(FEET));
    saved.insert("high".to_string(), serde_json::json!(3.5));
    saved.insert("username".to_string(), serde_json::json!("someone@example.com"));
    let settings = settings_for(&gauge_fields(), &saved);
    assert_eq!(settings["unit"], serde_json::json!(FEET));
    assert_eq!(settings["high"], serde_json::json!(3.5));
    assert_eq!(settings["username"], serde_json::json!("someone@example.com"));
    assert_eq!(settings["low"], serde_json::json!(0.5), "absent, so the field's own default");
    assert_eq!(settings["show_trend"], serde_json::json!(true));
}

/// **`enabled` is not a setting, and a secret is not either**, and this is
/// where both are enforced rather than merely written down: whether a source
/// is drawn is the host's decision about its own screen, and a credential
/// crosses as material resolved by name, never in the settings map.
#[test]
fn neither_enabled_nor_a_secret_is_ever_sent_as_a_setting() {
    let mut saved = serde_json::Map::new();
    // Even a saved map carrying them — which nothing writes — does not leak.
    saved.insert("enabled".to_string(), serde_json::json!(true));
    saved.insert("password".to_string(), serde_json::json!("hunter2"));
    let settings = settings_for(&gauge_fields(), &saved);
    assert!(
        !settings.contains_key("enabled"),
        "showing a card is the host's decision, not the source's"
    );
    assert!(
        !settings.contains_key("password"),
        "the password is a secret, written to the secret store, never a setting"
    );
}

/// **Only what `Describe` declares crosses.** A key saved for a field the
/// addon no longer has is left behind rather than sent.
#[test]
fn a_saved_key_the_addon_no_longer_declares_is_not_sent() {
    let mut saved = serde_json::Map::new();
    saved.insert("retired_field".to_string(), serde_json::json!(7));
    let settings = settings_for(&gauge_fields(), &saved);
    assert!(!settings.contains_key("retired_field"), "{settings:?}");
}

/// **A field's default is read off its kind and nothing else**, one per kind.
#[test]
fn each_kind_of_field_defaults_to_its_own_declared_value() {
    for field in gauge_fields() {
        let expected = match &field.kind {
            FieldKind::Bool { default } => serde_json::json!(default),
            FieldKind::Number { default, .. } => serde_json::json!(default),
            FieldKind::Choice { default, .. } | FieldKind::Text { default, .. } => {
                serde_json::json!(default)
            }
            FieldKind::Secret { .. } => serde_json::Value::Null,
        };
        assert_eq!(field_default(&field.kind), expected, "{}", field.key);
    }
}

/// **An absent secret is left OUT, not sent empty.** The protocol
/// distinguishes them: absent is *nothing is installed yet*, which a fresh
/// install is in and recovers from, and empty is *a credential naming nobody*.
#[test]
fn a_secret_that_is_not_installed_is_omitted_rather_than_sent_blank() {
    let names = vec!["definitely-not-installed.env".to_string()];
    let material = material(&names);
    assert!(
        material.is_empty(),
        "an uninstalled secret must not appear at all"
    );
}

/// **A name reaches the secret store and nowhere else**, which is the whole of
/// [`material`] since T5.1 removed the config key that could point one file
/// somewhere else. The name comes from `Describe`; the host has no second
/// opinion about where the file is.
///
/// This is asserted through `RIVET_APP_DATA`, because that is the one input
/// the SDK's shared call takes — and it is the same reason `crate::config`'s
/// tests do not touch `load` or `save`: a process-wide environment variable is
/// a race against every other test in the binary. So this test is `#[ignore]`d
/// and run alone, and the ordinary suite keeps the two assertions above it that
/// need no environment at all.
///
/// ```text
/// cargo test -- --ignored --test-threads=1 a_named_secret_is_read_from
/// ```
///
/// **The bytes here are opaque to this program.** They cross to the addon
/// unparsed — `crate::addon::material`'s doc comment says so — so this fixture
/// is deliberately not shaped like anything this repository still knows how to
/// read.
#[test]
#[ignore = "sets RIVET_APP_DATA, which every other test in this binary shares"]
fn a_named_secret_is_read_from_the_application_data_directory() {
    let dir = std::env::temp_dir().join("aura-addon-material");
    std::fs::create_dir_all(&dir).expect("scratch");
    std::fs::write(
        dir.join("reading-source.env"),
        "SOME_KEY='opaque-to-this-program'\n",
    )
    .expect("written");
    // SAFETY: this test is `#[ignore]`d for exactly this reason and is run on
    // its own thread-count-one invocation; nothing else in the binary reads
    // the environment concurrently with it.
    unsafe { std::env::set_var("RIVET_APP_DATA", &dir) };

    let material = material(&["reading-source.env".to_string()]);

    assert_eq!(material.len(), 1);
    assert!(material["reading-source.env"].contains("opaque-to-this-program"));
}

/// **The portal's validated map becomes the saved settings, whole**, and a
/// change is reported so the poller re-configures the addon.
#[test]
fn apply_settings_stores_the_validated_map_and_reports_the_change() {
    let mut config = crate::config::Config::default();
    let parsed = settings_for(&gauge_fields(), &serde_json::Map::new());
    assert!(apply_settings(&mut config.reading_settings, parsed.clone()));
    assert_eq!(config.reading_settings, parsed);
}

/// Saving the same form twice is not a change the second time — which is what
/// a caller uses to decide whether the addon needs a fresh `Configure`.
#[test]
fn apply_settings_reports_no_change_when_nothing_moved() {
    let parsed = settings_for(&gauge_fields(), &serde_json::Map::new());
    let mut saved = parsed.clone();
    assert!(!apply_settings(&mut saved, parsed));
}

/// **The submission REPLACES what was saved**, so a key the addon stopped
/// declaring — which the portal no longer renders or submits — is dropped at
/// the next save instead of being carried in the file for ever.
#[test]
fn apply_settings_replaces_rather_than_merges() {
    let mut saved = serde_json::Map::new();
    saved.insert("retired_field".to_string(), serde_json::json!(7));
    saved.insert("unit".to_string(), serde_json::json!(METERS));
    let mut parsed = serde_json::Map::new();
    parsed.insert("unit".to_string(), serde_json::json!(FEET));
    assert!(apply_settings(&mut saved, parsed));
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_eq!(saved["unit"], serde_json::json!(FEET));
}

// ---- apply_indoor_settings — `.rulings/` R-219 -------------------------

/// **The property `crate::portal`'s own form round-trip rests on.** The
/// portal always pre-fills the (possibly hidden) `verify_as` control with
/// `Config::tempmon_name` exactly as saved, never the resolved name — see
/// that field's own doc — so resubmitting an untouched form must report NO
/// change here, or a hidden field could silently rewrite the file every time
/// the rest of the page was saved. This sends it through
/// [`indoor_settings`], the same function `crate::portal` calls, rather than
/// building the map by hand.
#[test]
fn apply_indoor_settings_reports_no_change_when_the_form_round_trips_unchanged() {
    let mut config = crate::config::Config {
        tempmon_url: "https://127.0.0.1:8081".to_string(),
        tempmon_name: String::new(),
        indoor_sensor: "Sensor A".to_string(),
        ..crate::config::Config::default()
    };
    let parsed = indoor_settings(&config.tempmon_url, &config.tempmon_name, &config.indoor_sensor);
    assert!(
        !apply_indoor_settings(&mut config, &parsed),
        "an untouched round trip must not report a change"
    );
    assert_eq!(config.tempmon_name, "", "an untouched hidden field must not be rewritten");
}

/// **The other half: an operator who DID type something is never ignored.**
/// A real edit to any of the three keys is still a change, so this is not
/// proving the function inert — only that a value equal to what is already
/// held is a no-op.
#[test]
fn apply_indoor_settings_reports_a_change_when_the_name_is_actually_edited() {
    let mut config = crate::config::Config {
        tempmon_url: "https://rivet-01.local:8081".to_string(),
        tempmon_name: String::new(),
        indoor_sensor: "Sensor A".to_string(),
        ..crate::config::Config::default()
    };
    let parsed = indoor_settings(&config.tempmon_url, "rivet-01.local", &config.indoor_sensor);
    assert!(apply_indoor_settings(&mut config, &parsed));
    assert_eq!(config.tempmon_name, "rivet-01.local");
}

/// The two variants exist so a journal can say *there is no addon here* at
/// `info` and *something was refused* at `warn`, without a reader telling them
/// apart by wording.
#[test]
fn an_absent_addon_and_a_refused_one_are_different_outcomes() {
    let absent = NotOpened::Absent("no addon".to_string());
    let refused = NotOpened::Refused("REFUSED".to_string());
    assert_ne!(absent, refused);
    assert_eq!(absent.to_string(), "no addon");
    assert_eq!(refused.to_string(), "REFUSED");
}

/// **The identity a reinstall changes**, with exactly one library joined —
/// the ordinary case, and the one `Source::open` writes into the trial note.
#[test]
fn identity_for_one_joined_library_is_its_own_reference() {
    let joined = vec![rivetos_addon::host::Installed {
        class: role::READING.to_string(),
        path: std::path::PathBuf::from(
            "/state/lib/aura-reading-source-1.0.0-aaaa/lib/libaura-reading-source.so.1",
        ),
    }];
    assert_eq!(
        identity_for(role::READING, &joined),
        "aura-reading-source-1.0.0-aaaa"
    );
}

/// **The class this program opens for the corner card, and the soname it
/// asks the loader for.** The private addon is built to exactly these
/// strings; a host asking for any other would find the addon installed,
/// matched by nothing and never opened — a card that silently stops
/// appearing. `Cargo.toml`'s `accepts-class` is read back too, because the
/// supervisor joins on the manifest's spelling and not on this constant.
#[test]
fn the_reading_role_is_the_class_the_manifest_accepts() {
    assert_eq!(role::READING, "aura-reading-source");
    assert_eq!(
        rivetos_addon::abi::soname_for_class(role::READING),
        "libaura-reading-source.so.1"
    );
    let manifest = include_str!("../../Cargo.toml");
    let accepts = manifest
        .lines()
        .find(|line| line.trim_start().starts_with("accepts-class"))
        .expect("the manifest accepts classes");
    for class in [role::READING, role::INDOOR, role::BACKGROUND] {
        assert!(accepts.contains(&format!("\"{class}\"")), "{accepts} lacks {class}");
    }
}

/// **Nothing joined** falls back to the derived soname rather than a panic or
/// an empty string — `Loaded::open` is about to refuse this on its own
/// grounds and clear the note straight back off, so this only has to be
/// stable, not precise.
#[test]
fn identity_for_no_joined_library_falls_back_to_the_derived_soname() {
    assert_eq!(
        identity_for(role::READING, &[]),
        rivetos_addon::abi::soname_for_class(role::READING)
    );
}

/// **Two joined libraries of one class** is `Loaded::open`'s `Ambiguous`
/// case, refused there rather than chosen between here — so this also falls
/// back rather than picking one of the two arbitrarily.
#[test]
fn identity_for_two_joined_libraries_falls_back_rather_than_choosing_one() {
    let joined = vec![
        rivetos_addon::host::Installed {
            class: role::READING.to_string(),
            path: std::path::PathBuf::from(
                "/state/lib/aura-reading-source-1.0.0-aaaa/lib/libaura-reading-source.so.1",
            ),
        },
        rivetos_addon::host::Installed {
            class: role::READING.to_string(),
            path: std::path::PathBuf::from(
                "/state/lib/aura-reading-source-1.0.1-bbbb/lib/libaura-reading-source.so.1",
            ),
        },
    ];
    assert_eq!(
        identity_for(role::READING, &joined),
        rivetos_addon::abi::soname_for_class(role::READING)
    );
}
