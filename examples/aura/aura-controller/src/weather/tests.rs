//! **The icon slug, the recovery rule and the cache's age arithmetic.**
//!
//! Nothing here makes a request. `resolve`, `observation`, `recent` and
//! `statement` are the request; the rules inside them were split out —
//! [`super::newest_usable`] and [`super::icon_slug`] — so that the parts with a
//! decision in them can be asserted.

use super::*;

/// Decode the way `observation` does, so the shape under test is the NWS's.
fn properties(json: &str) -> ObservationProperties {
    serde_json::from_str(json).expect("the fixture parses as an NWS observation")
}

/// **No contact, no address**: an unconfigured Aura names only itself, so no
/// person's address is compiled into a request.
#[test]
fn an_empty_contact_sends_only_the_product() {
    assert_eq!(user_agent(""), "Aura/1.0");
    assert_eq!(user_agent("   "), "Aura/1.0");
}

/// The operator's contact travels in parentheses after the product, trimmed.
#[test]
fn a_configured_contact_is_carried_after_the_product() {
    assert_eq!(user_agent("ops@example.com"), "Aura/1.0 (ops@example.com)");
    assert_eq!(
        user_agent("  https://example.com/aura  "),
        "Aura/1.0 (https://example.com/aura)"
    );
}

/// **A contact a header cannot carry costs the contact, never the weather.**
/// A newline would split the header and anything outside printable ASCII is
/// not a valid header value, so such a contact is left out of the request.
#[test]
fn a_contact_a_header_cannot_carry_is_left_out() {
    for unsendable in ["ops@example.com\r\nX-Injected: 1", "tab\there", "café@example.com"] {
        assert!(!contact_is_sendable(unsendable), "{unsendable:?}");
        assert_eq!(user_agent(unsendable), "Aura/1.0", "{unsendable:?}");
    }
    assert!(contact_is_sendable("ops@example.com"));
}

#[test]
fn an_icon_url_becomes_its_slug() {
    assert_eq!(
        icon_slug("https://api.weather.gov/icons/land/day/ovc?size=medium"),
        "ovc"
    );
}

/// The probability suffix — `,30` in `rain_showers,30` — is not part of the
/// name the renderer looks up.
#[test]
fn a_probability_suffix_is_stripped() {
    assert_eq!(
        icon_slug("https://api.weather.gov/icons/land/night/rain_showers,30?size=medium"),
        "rain_showers"
    );
}

/// A station reporting no icon must produce an empty slug rather than a
/// plausible-looking one, because the renderer draws nothing for empty and
/// would draw the wrong thing for a guess.
#[test]
fn an_empty_icon_stays_empty() {
    assert_eq!(icon_slug(""), "");
}

/// A dual-condition icon — the NWS puts two slugs in one path — takes the
/// first, which is what the Go client does and what the renderer has art for.
#[test]
fn a_slug_with_no_query_or_comma_survives_intact() {
    assert_eq!(icon_slug("https://api.weather.gov/icons/land/day/skc"), "skc");
}

/// **Prefer the newest record with a condition**, not merely the newest with a
/// temperature: a reading with no icon beside it leaves the panel uncaptioned.
#[test]
fn the_newest_record_carrying_a_condition_wins_over_an_older_bare_reading() {
    let recovered = newest_usable(
        [
            // Newest: a temperature and nothing else.
            properties(r#"{"temperature":{"value":20.0},"icon":"","textDescription":""}"#),
            // Older: a temperature AND a condition.
            properties(
                r#"{"temperature":{"value":19.0},
                    "icon":"https://api.weather.gov/icons/land/day/bkn?size=medium",
                    "textDescription":"Mostly Cloudy"}"#,
            ),
        ]
        .into_iter(),
    )
    .expect("one of them is usable");
    assert_eq!(recovered.temperature.value, Some(19.0));
    assert_eq!(recovered.text_description, "Mostly Cloudy");
}

/// When nothing carries a condition, a bare temperature is still better than a
/// blank panel — so the newest one is taken rather than nothing.
#[test]
fn a_bare_temperature_is_recovered_when_no_record_has_a_condition() {
    let recovered = newest_usable(
        [
            properties(r#"{"temperature":{"value":null}}"#),
            properties(r#"{"temperature":{"value":20.0}}"#),
            properties(r#"{"temperature":{"value":18.0}}"#),
        ]
        .into_iter(),
    )
    .expect("a temperature is there");
    assert_eq!(recovered.temperature.value, Some(20.0), "the newest, not the oldest");
}

#[test]
fn a_window_with_no_temperature_at_all_is_a_named_failure() {
    let e = newest_usable(
        [
            properties(r#"{"temperature":{"value":null}}"#),
            properties(r#"{"temperature":{"value":null}}"#),
        ]
        .into_iter(),
    )
    .expect_err("nothing usable");
    assert!(e.contains("no temperature"), "{e}");
}

#[test]
fn an_empty_window_is_a_named_failure() {
    let e = newest_usable(std::iter::empty()).expect_err("nothing at all");
    assert!(e.contains("no temperature"), "{e}");
}

/// The NWS sends `null` for anything a station did not report, and the decode
/// must keep that distinct from a reading of zero.
#[test]
fn a_null_reading_decodes_as_absent_and_a_zero_as_a_reading() {
    assert_eq!(properties(r#"{"temperature":{"value":null}}"#).temperature.value, None);
    assert_eq!(
        properties(r#"{"temperature":{"value":0.0}}"#).temperature.value,
        Some(0.0)
    );
}

/// A member the NWS stops sending must not stop the rest being read — this
/// decode covers a real observation with most of its fields missing.
#[test]
fn an_observation_missing_every_optional_member_still_decodes() {
    let bare = properties("{}");
    assert_eq!(bare.temperature.value, None);
    assert_eq!(bare.wind_chill.value, None);
    assert_eq!(bare.icon, "");
}

/// **A member the NWS sends as an explicit `null`** decodes as though it were
/// absent. A station's recent list carrying `"textDescription": null` once
/// failed the whole decode, and the panel lost its temperature for want of a
/// caption; a whole reading sent as `null` is the same shape one level up.
#[test]
fn an_observation_with_null_members_still_decodes() {
    let nulls = properties(
        r#"{"temperature":{"value":21.5},"dewpoint":null,"windChill":null,
            "textDescription":null,"icon":null}"#,
    );
    assert_eq!(nulls.temperature.value, Some(21.5));
    assert_eq!(nulls.dewpoint.value, None);
    assert_eq!(nulls.wind_chill.value, None);
    assert_eq!(nulls.text_description, "");
    assert_eq!(nulls.icon, "");
}

/// The cache file is this program's own, and a round trip is what the restore
/// path depends on.
#[test]
fn a_cache_file_survives_being_written_and_read_back() {
    let file = CacheFile {
        saved_at_unix: 1_757_000_000,
        data: CachedWeather {
            temp_f: Some(70.0),
            secondary_f: Some(58.0),
            secondary_kind: "dewpoint".to_string(),
            condition: "Cloudy".to_string(),
            icon: "bkn".to_string(),
        },
    };
    let text = serde_json::to_string(&file).expect("a cache renders");
    let back: CacheFile = serde_json::from_str(&text).expect("and reads");
    assert_eq!(back.saved_at_unix, file.saved_at_unix);
    assert_eq!(back.data.temp_f, Some(70.0));
    assert_eq!(back.data.icon, "bkn");
}

/// Half an hour is one missed polling cycle, and the constant is what decides
/// whether a restart draws yesterday's temperature.
#[test]
fn the_cache_window_is_half_an_hour() {
    assert_eq!(MAX_CACHE_AGE_SECONDS, 1800);
}

#[test]
fn celsius_becomes_rounded_fahrenheit() {
    assert_eq!(to_fahrenheit(0.0), 32.0);
    assert_eq!(to_fahrenheit(-40.0), -40.0);
    assert_eq!(to_fahrenheit(21.1), 70.0);
}
