//! The aggregator holds and hands back. These assert that, and that a snapshot
//! is a copy rather than a view — the tick loop writes a snapshot to a socket
//! without the lock, and a view would be a use-after-unlock.

use super::*;

#[test]
fn a_cold_state_snapshots_a_state_message_with_an_empty_clock() {
    let state = State::new();
    let message = state.snapshot();
    assert_eq!(message.kind, "state");
    let clock = message.clock.expect("the clock member is always present");
    assert_eq!(clock.time, "");
    assert_eq!(clock.date, "");
}

#[test]
fn the_clock_that_was_set_is_the_clock_that_comes_back() {
    let state = State::new();
    state.set_clock(wire::Clock {
        time: "22:15".to_string(),
        date: "Friday, August 14".to_string(),
    });
    let clock = state.snapshot().clock.expect("a clock");
    assert_eq!(clock.time, "22:15");
    assert_eq!(clock.date, "Friday, August 14");
}

/// A snapshot taken before a write must not change when the write happens.
#[test]
fn a_snapshot_is_a_copy_and_not_a_view() {
    let state = State::new();
    state.set_clock(wire::Clock {
        time: "22:15".to_string(),
        date: String::new(),
    });
    let taken = state.snapshot();
    state.set_clock(wire::Clock {
        time: "22:16".to_string(),
        date: String::new(),
    });
    assert_eq!(taken.clock.expect("a clock").time, "22:15");
}

/// Shared across threads is the whole reason the lock is there: the tick loop
/// reads while the weather, indoor and reading-card threads write.
#[test]
fn it_can_be_written_from_another_thread() {
    let state = std::sync::Arc::new(State::new());
    let writer = std::sync::Arc::clone(&state);
    std::thread::spawn(move || {
        writer.set_clock(wire::Clock {
            time: "07:00".to_string(),
            date: String::new(),
        });
    })
    .join()
    .expect("the writing thread finishes");
    assert_eq!(state.snapshot().clock.expect("a clock").time, "07:00");
}

/// **Clearing the card is the only thing that removes it** — a switched-off
/// card, or no source installed — and the snapshot then carries no `reading`
/// member at all, which the renderer reads as *switched off*.
#[test]
fn a_cleared_reading_card_leaves_the_snapshot() {
    let state = State::new();
    state.set_reading(wire::ReadingData {
        status: wire::status::CONNECTING,
        ..wire::ReadingData::default()
    });
    assert!(state.snapshot().reading.is_some());
    state.clear_reading();
    assert!(state.snapshot().reading.is_none());
}
