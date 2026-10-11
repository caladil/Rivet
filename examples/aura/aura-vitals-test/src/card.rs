//! **The vitals card** — one shape, two languages.
//!
//! `SPEC-04` §5, `PLAN.md` §112.2. This file and
//! `examples/aura/aura-vitals-go/card.go` compose **the same card from the same
//! facts**: `examples/aura/vitals-fixture` holds one machine's `/proc` and `/sys`
//! and one `expected-card.txt`, and **both producers' test suites assert against
//! that same file** rather than against a string of their own. Two suites each
//! carrying a private copy would prove only that each matches itself.
//!
//! That shared expectation is the whole demonstration: `ADR-036` §6.2B ruled two
//! producers because *"`provides` is an interface, and two implementations are
//! what prove it"*, and an interface is only proven by two implementations that
//! agree.
//!
//! # The shape, and why these blocks
//!
//! ```text
//!    52.4 °C            a reading — the number that decides whether to worry
//!    SoC temperature
//!
//!    rivet-01           a list — the numbers you check second
//!    uptime      3d 4h
//!    memory      412 MiB free of 924
//!    load         0.14
//!    tasks         128
//! ```
//!
//! A `reading` and a `list`, which is deliberate: they are the two shapes
//! `aura-renderer`'s generic composition draws (`SPEC-04` §5.9), and `tertiary`
//! is the slot that composition serves. A producer exercising the path that was
//! built for producers is the point rather than a coincidence.
//!
//! **No `glyph` anywhere.** The vocabulary's thirteen are weather and a house
//! (`SPEC-04` §5.6); none of them means a thermometer or a clock this build can
//! draw, and `icon_is_drawn` is false for the two that are named and unported.
//! Asking for one would reserve no width and draw nothing, which is a worse
//! answer than not asking.

use std::time::Duration;

use aura_card::{Block, Card, List, Message, Reading, Role, Row, Slot, State};

use crate::vitals::Vitals;

/// **The slot this producer claims**, and it is `tertiary` on purpose.
///
/// `primary` is the clock and `secondary` is the weather panel — both are
/// Aura's own cards with hard-coded compositions (R-125). `tertiary` is drawn by
/// the generic path, which is what an addon's card actually gets, so a producer
/// meant to demonstrate the addon contract belongs there rather than somewhere
/// flattering. `PLAN.md` §113 asked for exactly this.
pub const SLOT: Slot = Slot::Tertiary;

/// **How long this card stays true**, in seconds.
///
/// Four times [`crate::TICK`], so three consecutive failed sends do not blank
/// the panel — but a producer that has actually died takes its card off the
/// wall within a minute rather than leaving a temperature nobody is vouching
/// for. `SPEC-04` `Card::ttl_seconds` argues the principle; this is the number.
pub const TTL_SECONDS: u32 = 60;

/// Above this, the SoC is hot enough to say so — `Role::Warn`.
///
/// **60 °C, and it is a Pi number rather than a round one.** A Raspberry Pi 4
/// begins soft-throttling at 80 °C and hard-throttling at 85 °C; 60 is where a
/// panel in a warm room stops being unremarkable and starts being worth a
/// second look, which is what an amber reading means (`SPEC-04` §5.1).
pub const WARN_TENTHS: i64 = 600;

/// Above this it is `Role::Alarm` — 80 °C, where the firmware itself starts
/// taking clock away. Not a guess: it is the number the hardware acts on.
pub const ALARM_TENTHS: i64 = 800;

/// How often a card is sent.
pub const TICK: Duration = Duration::from_secs(15);

/// Compose the card.
///
/// **Every block is conditional on having something to say.** A machine with no
/// thermal zone (QEMU has none) sends a reading whose `state` is `unavailable`
/// and no value, which `SPEC-04` §5.4 makes the producer's word; a machine that
/// could read nothing at all sends a card with an explaining statement rather
/// than an empty one, because `Card::blocks` may not be empty and a withdrawal
/// has its own verb.
#[must_use]
pub fn compose(v: &Vitals) -> Card {
    let mut blocks = Vec::new();

    // --- the temperature, as a reading ------------------------------------
    let (state, role) = match v.temperature_tenths {
        None => (State::Unavailable, Role::Muted),
        Some(t) if t >= ALARM_TENTHS => (State::Ok, Role::Alarm),
        Some(t) if t >= WARN_TENTHS => (State::Ok, Role::Warn),
        Some(_) => (State::Ok, Role::Normal),
    };
    blocks.push(Block::Reading(Reading {
        value: v.temperature_text(),
        // **The unit is omitted with the value**, not left dangling. A card
        // reading `°C` with no number in front of it is a rendering bug on a
        // wall, and `SPEC-04` §6.3 rule 3 says absent is spelled by omitting.
        unit: v.temperature_tenths.map(|_| "°C".to_string()),
        caption: Some(
            if v.temperature_tenths.is_some() {
                "SoC temperature"
            } else {
                "no thermal zone on this machine"
            }
            .to_string(),
        ),
        state,
        role,
        ..Reading::default()
    }));

    // --- everything else, as a list ---------------------------------------
    let mut rows = Vec::new();
    let mut row = |label: &str, trail: Option<String>| {
        if let Some(trail) = trail {
            rows.push(Row {
                label: label.to_string(),
                trail: Some(trail),
                ..Row::default()
            });
        }
    };
    row("uptime", v.uptime_text());
    row("memory", v.memory_text());
    row("load", v.load_1m.clone());
    row("tasks", v.tasks.map(|n| n.to_string()));

    if !rows.is_empty() {
        blocks.push(Block::List(List {
            // The machine's own name, so a photograph of a wall of panels says
            // which one is which. Absent on a machine that could not read it,
            // rather than a placeholder that would look like a hostname.
            caption: v.hostname.clone(),
            glyph: None,
            rows,
        }));
    }

    Card {
        slot: SLOT,
        ttl_seconds: TTL_SECONDS,
        blocks,
    }
}

/// The card, encoded and ready for the socket.
///
/// # Errors
///
/// [`aura_card::Fault`], naming the field — `Message::encode` validates exactly
/// what the renderer's decoder validates, so a card this refuses is a card that
/// would have been refused on arrival.
pub fn encode(v: &Vitals) -> Result<String, aura_card::Fault> {
    Message::Card(compose(v)).encode()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    fn healthy() -> Vitals {
        Vitals {
            hostname: Some("rivet-01".to_string()),
            temperature_tenths: Some(524),
            uptime_seconds: Some(3 * 86400 + 4 * 3600),
            memory_total_kib: Some(946_684),
            memory_available_kib: Some(422_948),
            load_1m: Some("0.14".to_string()),
            tasks: Some(128),
        }
    }

    /// The shared fixture tree — `examples/aura/vitals-fixture`.
    ///
    /// One machine, frozen, so that this producer and the Go one can be run
    /// against **the same facts**. `CARGO_MANIFEST_DIR`-relative, because a
    /// path resolved from the working directory is how
    /// `rivet-sdk`'s font tests spent weeks passing by doing nothing
    /// (`SPEC-04` §12.1).
    const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../vitals-fixture");

    fn fixture_expectation() -> String {
        let path = std::path::Path::new(FIXTURE).join("expected-card.txt");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|why| panic!("the shared expectation at {} is missing: {why}", path.display()));
        assert!(
            !text.contains("\r\n"),
            "expected-card.txt has CRLF line endings; the wire format is LF"
        );
        text
    }

    /// **What the renderer will actually receive**, pinned byte for byte —
    /// against the SAME file the Go producer's own test asserts against.
    ///
    /// One expectation and two readers, deliberately. Two suites each carrying
    /// a private copy of the string would prove only that each matches itself,
    /// which is the failure mode this whole pair exists to rule out
    /// (`ADR-036` §6.2B).
    #[test]
    fn the_fixture_machine_encodes_to_the_card_both_producers_must_write() {
        let read = Vitals::read(std::path::Path::new(FIXTURE));
        assert_eq!(
            encode(&read).expect("a valid card"),
            fixture_expectation(),
            "the Rust producer no longer writes the shared expectation"
        );
    }

    /// Each fact, read from the fixture as the Go half reads it. A failure here
    /// localises a disagreement the whole-card test can only report as a diff.
    #[test]
    fn every_fact_is_read_from_the_fixture() {
        let v = Vitals::read(std::path::Path::new(FIXTURE));
        assert_eq!(v.hostname.as_deref(), Some("rivet-01"));
        assert_eq!(v.temperature_tenths, Some(524));
        assert_eq!(v.uptime_seconds, Some(273_600));
        assert_eq!(v.memory_total_kib, Some(946_684));
        assert_eq!(v.memory_available_kib, Some(422_948));
        assert_eq!(v.load_1m.as_deref(), Some("0.14"));
        assert_eq!(v.tasks, Some(128));
    }

    /// The composed card and the read one agree, so the hand-built `healthy()`
    /// fixture below cannot drift away from the tree on disk.
    #[test]
    fn the_hand_built_machine_and_the_fixture_machine_are_the_same_machine() {
        assert_eq!(Vitals::read(std::path::Path::new(FIXTURE)), healthy());
    }

    /// Anything this writes, the renderer's own decoder accepts — which is the
    /// property `Message::encode` exists to guarantee, checked here rather than
    /// assumed.
    #[test]
    fn every_card_this_producer_can_write_survives_a_round_trip() {
        let cases = [
            healthy(),
            // No thermal zone — QEMU.
            Vitals {
                temperature_tenths: None,
                ..healthy()
            },
            // A machine that could read nothing but its name.
            Vitals {
                hostname: Some("bare".to_string()),
                ..Vitals::default()
            },
            // And one that could not read even that.
            Vitals::default(),
            // Hot enough to worry about, and hot enough to act on.
            Vitals {
                temperature_tenths: Some(WARN_TENTHS),
                ..healthy()
            },
            Vitals {
                temperature_tenths: Some(ALARM_TENTHS),
                ..healthy()
            },
        ];
        for v in cases {
            let text = encode(&v).expect("this producer must not compose a card it cannot send");
            let decoded = Message::decode(&text).expect("the renderer must accept it");
            assert_eq!(decoded, Message::Card(compose(&v)));
        }
    }

    /// The thresholds are the hardware's, and each is inclusive at its edge.
    #[test]
    fn the_temperature_role_changes_at_the_numbers_the_firmware_acts_on() {
        let role_at = |tenths: i64| {
            let v = Vitals {
                temperature_tenths: Some(tenths),
                ..healthy()
            };
            match compose(&v).blocks.first() {
                Some(Block::Reading(r)) => r.role,
                other => panic!("the first block should be the reading, got {other:?}"),
            }
        };
        assert_eq!(role_at(WARN_TENTHS - 1), Role::Normal);
        assert_eq!(role_at(WARN_TENTHS), Role::Warn);
        assert_eq!(role_at(ALARM_TENTHS - 1), Role::Warn);
        assert_eq!(role_at(ALARM_TENTHS), Role::Alarm);
    }

    /// **A card is never empty**, whatever could not be read — `Card::blocks`
    /// forbids it and a withdrawal has its own verb.
    #[test]
    fn a_machine_that_can_read_nothing_still_sends_a_card_that_says_so() {
        let card = compose(&Vitals::default());
        assert!(!card.blocks.is_empty());
        let text = encode(&Vitals::default()).expect("still valid");
        assert!(
            text.contains("no thermal zone"),
            "the card should explain itself: {text}"
        );
        // And no `unit:` line dangling without a value.
        assert!(!text.contains("unit:"), "a unit with no number: {text}");
    }

    /// The slot is the one the generic composition draws, which is the whole
    /// reason this producer exists — `PLAN.md` §113.
    #[test]
    fn the_card_claims_the_slot_an_addon_actually_gets() {
        assert_eq!(compose(&healthy()).slot, Slot::Tertiary);
    }
}
