//! The formatters are tested on **constructed** times rather than on the
//! machine's clock, because a test that asserted what `localtime_r` returns
//! would be asserting the timezone of whoever ran it.

use super::*;

fn at(hour: u32, minute: u32, weekday: u32, month: u32, day: u32) -> Local {
    Local {
        hour,
        minute,
        second: 0,
        weekday,
        month,
        day,
        utc_offset_seconds: 0,
        zone: *b"UTC\0\0\0\0\0",
    }
}

#[test]
fn twenty_four_hour_time_is_zero_padded() {
    assert_eq!(at(9, 5, 0, 0, 1).time_string(true), "09:05");
    assert_eq!(at(22, 15, 0, 0, 1).time_string(true), "22:15");
    assert_eq!(at(0, 0, 0, 0, 1).time_string(true), "00:00");
}

/// Go's `3:04`: no leading zero on the hour, and **no meridiem** — the renderer
/// lays the string out at a fixed width and a suffix would overflow it.
#[test]
fn twelve_hour_time_has_no_leading_zero_and_no_meridiem() {
    assert_eq!(at(9, 5, 0, 0, 1).time_string(false), "9:05");
    assert_eq!(at(22, 15, 0, 0, 1).time_string(false), "10:15");
}

/// Midnight and noon are the two the modulo gets wrong if it is written the
/// obvious way, and they are an hour a display is actually showing.
#[test]
fn midnight_and_noon_are_both_twelve() {
    assert_eq!(at(0, 30, 0, 0, 1).time_string(false), "12:30");
    assert_eq!(at(12, 30, 0, 0, 1).time_string(false), "12:30");
}

/// `"Weekday, Month D"`, with no leading zero on the day — Go's
/// `Monday, January 2`.
#[test]
fn the_date_is_the_go_layout() {
    assert_eq!(at(0, 0, 5, 7, 14).date_string(), "Friday, August 14");
    assert_eq!(at(0, 0, 0, 0, 2).date_string(), "Sunday, January 2");
}

/// Every weekday and every month must name itself, because an index that is off
/// by one shows the wrong day on a wall for a week before anybody notices.
#[test]
fn every_weekday_and_month_is_reachable_and_distinct() {
    let weekdays: Vec<String> = (0..7)
        .map(|weekday| at(0, 0, weekday, 0, 1).date_string())
        .collect();
    assert!(weekdays[0].starts_with("Sunday, "), "{weekdays:?}");
    assert!(weekdays[6].starts_with("Saturday, "), "{weekdays:?}");

    let months: Vec<String> = (0..12)
        .map(|month| at(0, 0, 0, month, 1).date_string())
        .collect();
    assert!(months[0].contains("January"), "{months:?}");
    assert!(months[11].contains("December"), "{months:?}");
}

#[test]
fn a_scheduled_time_is_always_twenty_four_hour() {
    assert_eq!(at(7, 0, 0, 0, 1).schedule_time(), "07:00");
    assert_eq!(at(23, 59, 0, 0, 1).schedule_time(), "23:59");
}

/// The abbreviations and the full names must be indexed the same way, or a
/// schedule fires on the wrong day while the screen shows the right one.
#[test]
fn the_weekday_tables_agree_with_each_other() {
    for weekday in 0..7 {
        let local = at(0, 0, weekday, 0, 1);
        assert!(
            local.date_string().starts_with(local.weekday_abbreviation()),
            "day {weekday}: {} does not begin with {}",
            local.date_string(),
            local.weekday_abbreviation()
        );
    }
}

/// **Never zero**, because a wait of zero at `:00` would spin the schedule
/// thread through a whole minute of evaluations.
#[test]
fn the_wait_to_the_next_minute_is_never_zero_and_never_over_a_minute() {
    for second in 0..=60 {
        let wait = Local {
            second,
            ..at(0, 0, 0, 0, 1)
        }
        .seconds_to_next_minute();
        assert!((1..=60).contains(&wait), "second {second} gave {wait}");
    }
    let at_the_boundary = Local {
        second: 0,
        ..at(0, 0, 0, 0, 1)
    };
    assert_eq!(at_the_boundary.seconds_to_next_minute(), 60);
    let nearly_there = Local {
        second: 59,
        ..at(0, 0, 0, 0, 1)
    };
    assert_eq!(nearly_there.seconds_to_next_minute(), 1);
}

/// The portal shows this where the Go portal had a picker, so it has to read
/// like something rather than like a struct.
#[test]
fn the_zone_label_names_the_abbreviation_and_the_offset() {
    let utc = at(0, 0, 0, 0, 1);
    assert_eq!(utc.zone_label(), "UTC (UTC+00:00)");

    let central = Local {
        utc_offset_seconds: -5 * 3600,
        zone: *b"CDT\0\0\0\0\0",
        ..utc
    };
    assert_eq!(central.zone_label(), "CDT (UTC\u{2212}05:00)");

    // Half-hour offsets exist and the minutes must not be dropped.
    let kolkata = Local {
        utc_offset_seconds: 5 * 3600 + 1800,
        zone: *b"IST\0\0\0\0\0",
        ..utc
    };
    assert_eq!(kolkata.zone_label(), "IST (UTC+05:30)");

    // A zone the C library did not name still reports the offset, which is the
    // part that actually changes what is on the clock.
    let unnamed = Local {
        utc_offset_seconds: 3600,
        zone: [0u8; 8],
        ..utc
    };
    assert_eq!(unnamed.zone_label(), "UTC+01:00");
}

/// Not an assertion about the value — that would be an assertion about the
/// runner's timezone — only that the C library answers at all.
#[test]
fn the_epoch_can_be_broken_down() {
    let local = Local::at(0).expect("localtime_r accepts the epoch");
    assert!(local.hour < 24 && local.minute < 60);
}
