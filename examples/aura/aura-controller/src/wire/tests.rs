//! **The wire's tests are about what is written and what is left out**, because
//! that distinction is the whole contract with the renderer.

use super::*;

/// Parse what we rendered, so an assertion is about the JSON and not about the
/// order serde happened to emit fields in.
fn rendered(message: &Message) -> serde_json::Value {
    serde_json::from_str(&serde_json::to_string(message).expect("a message renders"))
        .expect("what serde wrote, serde reads")
}

#[test]
fn a_state_message_with_nothing_in_it_is_just_its_type() {
    let json = rendered(&Message::state());
    assert_eq!(json, serde_json::json!({ "type": "state" }));
}

#[test]
fn a_command_carries_its_name_and_value_and_no_state_members() {
    let json = rendered(&Message::cmd("brightness", "100"));
    assert_eq!(
        json,
        serde_json::json!({ "type": "cmd", "cmd": "brightness", "value": "100" })
    );
}

/// The schema: *"omitted picks a random different corner"*. An empty `corner`
/// must therefore not be written, or the renderer would be told to go to the
/// corner named by the empty string.
#[test]
fn an_unnamed_corner_is_omitted_rather_than_sent_as_an_empty_one() {
    let json = rendered(&Message {
        kind: "cmd",
        cmd: "rotate_corner",
        ..Message::default()
    });
    assert_eq!(
        json,
        serde_json::json!({ "type": "cmd", "cmd": "rotate_corner" })
    );
}

/// **Absent is not zero**, and this is the assertion that says so. A station
/// that is not reporting must send no `temp_f` at all, because 0°F is an
/// ordinary January reading in Minnesota.
#[test]
fn an_absent_reading_is_omitted_and_a_zero_reading_is_written() {
    let absent = rendered(&Message {
        weather: Some(Weather::default()),
        ..Message::state()
    });
    assert!(
        absent["weather"].get("temp_f").is_none(),
        "an absent temperature was written: {absent}"
    );

    let zero = rendered(&Message {
        weather: Some(Weather {
            temp_f: Some(0.0),
            ..Weather::default()
        }),
        ..Message::state()
    });
    assert_eq!(zero["weather"]["temp_f"], 0.0);
}

/// `condition` and `icon` carry no `omitempty` in Go and must be written even
/// when empty; `statement`, `unit` and `secondary_kind` carry one and must not.
#[test]
fn the_weather_fields_that_are_always_written_are_written_when_empty() {
    let json = rendered(&Message {
        weather: Some(Weather::default()),
        ..Message::state()
    });
    let weather = &json["weather"];
    assert_eq!(weather["condition"], "");
    assert_eq!(weather["icon"], "");
    assert!(weather.get("statement").is_none());
    assert!(weather.get("unit").is_none());
    assert!(weather.get("secondary_kind").is_none());
}

/// `online: false` means *do not draw this*, so it must be written rather than
/// omitted as a falsy value — the renderer cannot act on a field it never sees.
#[test]
fn indoor_writes_online_even_when_it_is_false() {
    let json = rendered(&Message {
        indoor: Some(Indoor::default()),
        ..Message::state()
    });
    assert_eq!(json["indoor"]["online"], false);
    assert_eq!(json["indoor"]["label"], "");
}

/// `value` is omitted for `error` and `connecting`, *"so a renderer that
/// ignored `status` entirely still could not put a stale number on the wall"*.
#[test]
fn a_reading_card_with_no_reading_carries_a_status_and_no_value() {
    let json = rendered(&Message {
        reading: Some(ReadingData {
            status: "connecting",
            ..ReadingData::default()
        }),
        ..Message::state()
    });
    assert_eq!(json["reading"]["status"], "connecting");
    assert!(json["reading"].get("value").is_none());
    assert!(json["reading"].get("out_of_range").is_none());
    // A cold start's bare card has no unit to carry and writes none.
    assert!(json["reading"].get("unit").is_none());
}

/// **The card travels under `reading`, and `aura-renderer` reads that key.**
///
/// The two programs are deployed together and this key is the whole of their
/// agreement about the corner card: a renderer reading any other key would
/// silently draw no card at all. The renderer's own tests parse this same
/// shape from the other side.
#[test]
fn the_reading_card_is_written_under_the_key_reading() {
    let json = rendered(&Message {
        reading: Some(ReadingData {
            value: Some(2.4),
            unit: "m".to_string(),
            trend: "rising".to_string(),
            status: "ok",
            age_seconds: Some(90),
            ..ReadingData::default()
        }),
        ..Message::state()
    });
    assert_eq!(json["reading"]["value"], 2.4);
    assert_eq!(json["reading"]["unit"], "m");
    assert_eq!(json["reading"]["trend"], "rising");
    assert_eq!(json["reading"]["age_seconds"], 90);
    let keys: Vec<&String> = json.as_object().expect("an object").keys().collect();
    assert_eq!(keys.len(), 2, "only `type` and `reading`: {keys:?}");
}

/// The bus row is reserved and never populated; an empty list must not appear
/// on the wire, where it would be a member the renderer has to skip for ever.
#[test]
fn the_reserved_bus_list_is_omitted_when_empty() {
    let json = rendered(&Message::state());
    assert!(json.get("bus").is_none());
}
