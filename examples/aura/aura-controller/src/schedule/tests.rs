//! **The matching rules and every directive**, without waiting a minute.

use super::*;

fn at(hour: u32, minute: u32, weekday: u32) -> Local {
    Local {
        hour,
        minute,
        second: 0,
        weekday,
        month: 0,
        day: 1,
        utc_offset_seconds: 0,
        zone: *b"UTC\0\0\0\0\0",
    }
}

fn action(kind: &str, value: &str) -> ScheduledAction {
    ScheduledAction {
        kind: kind.to_string(),
        value: value.to_string(),
        theme: String::new(),
    }
}

fn event(time: &str, days: &[&str], actions: Vec<ScheduledAction>) -> ScheduledEvent {
    ScheduledEvent {
        time: time.to_string(),
        days: days.iter().map(|d| (*d).to_string()).collect(),
        actions,
    }
}

#[test]
fn an_event_at_this_minute_fires_all_of_its_actions_in_order() {
    let events = vec![event(
        "19:00",
        &[],
        vec![action("brightness", "20"), action("weather", "off")],
    )];
    let fired = due(&events, &at(19, 0, 1));
    assert_eq!(fired.len(), 2);
    assert_eq!(fired[0].value, "20");
    assert_eq!(fired[1].kind, "weather");
}

#[test]
fn an_event_at_another_minute_fires_nothing() {
    let events = vec![event("19:00", &[], vec![action("brightness", "20")])];
    assert!(due(&events, &at(19, 1, 1)).is_empty());
    assert!(due(&events, &at(18, 0, 1)).is_empty());
}

/// **No days means every day**, which is the common case and the one an empty
/// list would get wrong in the most damaging direction.
#[test]
fn an_event_with_no_days_fires_on_every_day() {
    let events = vec![event("07:00", &[], vec![action("brightness", "80")])];
    for weekday in 0..7 {
        assert_eq!(
            due(&events, &at(7, 0, weekday)).len(),
            1,
            "nothing fired on day {weekday}"
        );
    }
}

#[test]
fn a_day_restriction_is_honoured() {
    // Weekdays only. 1 is Monday, 5 is Friday, 6 is Saturday.
    let events = vec![event(
        "07:00",
        &["Mon", "Tue", "Wed", "Thu", "Fri"],
        vec![action("brightness", "80")],
    )];
    assert_eq!(due(&events, &at(7, 0, 1)).len(), 1, "Monday");
    assert_eq!(due(&events, &at(7, 0, 5)).len(), 1, "Friday");
    assert!(due(&events, &at(7, 0, 6)).is_empty(), "Saturday");
    assert!(due(&events, &at(7, 0, 0)).is_empty(), "Sunday");
}

/// The portal writes `Mon`; a person editing the file by hand writes what they
/// write, and a schedule that silently never fired would be very hard to see.
#[test]
fn a_day_is_matched_without_regard_to_case() {
    let events = vec![event("07:00", &["MON"], vec![action("brightness", "80")])];
    assert_eq!(due(&events, &at(7, 0, 1)).len(), 1);
}

#[test]
fn several_events_at_the_same_minute_all_fire() {
    let events = vec![
        event("19:00", &[], vec![action("brightness", "20")]),
        event("19:00", &[], vec![action("weather", "off")]),
    ];
    assert_eq!(due(&events, &at(19, 0, 3)).len(), 2);
}

// ---- The directives -------------------------------------------------------

fn cmd_of(action: &ScheduledAction) -> (String, String) {
    let message = directive(action).expect("a directive");
    (message.cmd.to_string(), message.value.clone())
}

/// **A schedule written before T7 still changes the background tonight.**
///
/// `config::migrate` translates the two top-level keys, and a scheduled action
/// is a value inside a list rather than a field on `Config` — so it is never
/// reached by that pass. Migrating here is what stops an event saved a month
/// ago from sending `matrix` to a pack that has `digital-rain`.
#[test]
fn a_scheduled_screensaver_written_before_t7_still_resolves() {
    let mut action = action("screensaver", "matrix");
    action.theme = "ice_blue".to_string();
    let message = directive(&action).expect("a directive");
    assert_eq!(message.cmd, "set_screensaver");
    assert_eq!(message.value, "digital-rain");
    assert_eq!(message.theme, "ice-blue");
}

/// And one written after it passes straight through.
#[test]
fn a_scheduled_screensaver_carries_the_pair_the_pack_speaks() {
    let mut action = action("screensaver", "digital-rain");
    action.theme = "ice-blue".to_string();
    let message = directive(&action).expect("a directive");
    assert_eq!(message.value, "digital-rain");
    assert_eq!(message.theme, "ice-blue");
}

#[test]
fn brightness_passes_its_value_through() {
    assert_eq!(
        cmd_of(&action("brightness", "35")),
        ("brightness".to_string(), "35".to_string())
    );
}

/// Both spellings of each direction, because the config has carried both.
#[test]
fn display_takes_two_spellings_of_each_direction() {
    for value in ["on", "wake"] {
        assert_eq!(cmd_of(&action("display", value)).0, "display_wake");
    }
    for value in ["off", "sleep"] {
        assert_eq!(cmd_of(&action("display", value)).0, "display_sleep");
    }
}

#[test]
fn weather_sends_the_panel_switch() {
    assert_eq!(
        cmd_of(&action("weather", "on")),
        ("show_weather".to_string(), "on".to_string())
    );
    assert_eq!(cmd_of(&action("weather", "off")).1, "off");
}

/// **An action nobody can act on is named rather than skipped**, because the
/// alternative is a person editing a schedule and watching nothing happen.
#[test]
fn an_unknown_action_type_is_a_named_failure() {
    let e = directive(&action("teleport", "yes")).expect_err("no such action");
    assert!(e.contains("teleport"), "{e}");
}

#[test]
fn a_known_action_with_an_unknown_value_is_a_named_failure() {
    let e = directive(&action("display", "sideways")).expect_err("no such value");
    assert!(e.contains("sideways"), "{e}");

    let e = directive(&action("weather", "maybe")).expect_err("no such value");
    assert!(e.contains("maybe"), "{e}");
}

/// `cec` is retired in `crate::config`, which drops it on load — so by the time
/// an action reaches here it should not exist. This asserts the second half of
/// that: if one ever did arrive, it is named rather than silently ignored.
#[test]
fn a_retired_cec_action_that_somehow_survives_is_still_named() {
    let e = directive(&action("cec", "on")).expect_err("cec is not this program's");
    assert!(e.contains("cec"), "{e}");
}
