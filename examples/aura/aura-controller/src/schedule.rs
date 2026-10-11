//! **The schedule** — `internal/scheduler/scheduler.go`, ported.
//!
//! A list of events, each with a wall-clock `HH:MM`, an optional set of days,
//! and the actions to fire. Evaluated once a minute, on the minute.
//!
//! # It fires DIRECTIVES, and it is not the machine's schedule
//!
//! This is worth stating because Rivet has one of its own. `ADR-039` §3.1's
//! `[display] off/on` decides **when the screens are lit**, and it belongs to
//! the machine because a kiosk application holds the display lease exclusively
//! and cannot reach `/dev/cec` to switch a television. What this schedule fires
//! is the other kind of thing entirely: brightness, which screensaver is
//! running, and whether the weather panel is drawn — all of them directives to
//! a renderer, over a socket, about pixels that are already lit.
//!
//! The Go schedule also carried a `cec` action. It is retired here, with a note
//! naming where the setting went; `crate::config` argues it.
//!
//! # Minute-aligned, so an event set for 19:00 fires at 19:00
//!
//! The thread sleeps to the next `:00` before its first evaluation and then a
//! minute at a time, rather than sleeping sixty seconds from whenever the
//! process happened to start.
//!
//! **A minute can therefore be missed, and that is accepted.** A clock stepped
//! forward by NTP, or a machine that was asleep, skips whatever was scheduled
//! in the interval — the Go program has the same property for the same reason,
//! and catching up would mean firing a *switch the screensaver on* directive at
//! a screen somebody is currently reading.

use crate::clock::Local;
use crate::config::{ScheduledAction, ScheduledEvent};

/// Every action due at this moment, in the order the config lists them.
///
/// **Split from the thread that calls it** so the matching rules can be
/// asserted without waiting a minute for each one.
pub fn due(events: &[ScheduledEvent], now: &Local) -> Vec<ScheduledAction> {
    let time = now.schedule_time();
    let day = now.weekday_abbreviation();
    events
        .iter()
        .filter(|event| event.time == time && matches_day(&event.days, day))
        .flat_map(|event| event.actions.iter().cloned())
        .collect()
}

/// The directive one action becomes.
///
/// **Pure, and separate from sending it**, so that every mapping below is
/// asserted rather than watched. The `Err` is the sentence to journal: an
/// action this does not understand is a config a person wrote and a directive
/// nobody will see happen, so it says so by name instead of being skipped.
///
/// # Errors
///
/// A sentence, for an action type this build does not know or a value the type
/// does not take.
pub fn directive(action: &ScheduledAction) -> Result<crate::wire::Message, String> {
    use crate::wire::Message;

    match action.kind.as_str() {
        // **Migrated on the way out, not only on the way in** — T7. A schedule
        // saved before T7 holds `matrix`/`ice_blue` in its actions, and
        // `config::migrate` only translates the two top-level keys, because a
        // scheduled action is a value inside a list rather than a field on
        // `Config`. Doing it here costs one call and means an event written a
        // month ago still changes the background tonight.
        "screensaver" => {
            let (background, theme) =
                crate::screensaver::migrate(&action.value, &action.theme);
            Ok(Message::screensaver(background, theme))
        }
        // Not validated here. The renderer holds the 5–100 range and refusing a
        // number this program cannot check would be a second opinion about
        // somebody else's field (`ADR-011`).
        "brightness" => Ok(Message::cmd("brightness", action.value.clone())),
        "display" => match action.value.as_str() {
            "on" | "wake" => Ok(Message::cmd("display_wake", "")),
            "off" | "sleep" => Ok(Message::cmd("display_sleep", "")),
            other => Err(format!("display: unknown value {other:?}")),
        },
        "weather" => match action.value.as_str() {
            "on" | "off" => Ok(Message::cmd("show_weather", action.value.clone())),
            other => Err(format!("show_weather: unknown value {other:?}")),
        },
        other => Err(format!("unknown action type {other:?}")),
    }
}

/// Whether `day` is in `days`.
///
/// **An empty list is every day**, not no days — that is what an event with no
/// day restriction looks like, and it is the common case.
///
/// Matched case-insensitively: the portal writes `Mon`, and a person editing
/// the file by hand writes whatever they write.
fn matches_day(days: &[String], day: &str) -> bool {
    days.is_empty() || days.iter().any(|d| d.eq_ignore_ascii_case(day))
}

#[cfg(test)]
mod tests;
