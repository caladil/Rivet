//! **The wire, held in place.**
//!
//! `AGENTS.md` §4: *"a test that round-trips a reader against its own writer
//! proves the two agree with each other and nothing about the format."* So the
//! round-trip tests are here, and so is [`HAND_WRITTEN_WEATHER_CARD`] — bytes
//! composed by hand from `SPEC-04` §6, which is what an addon author would type
//! and is therefore the only thing that tests the *format* rather than the pair
//! of functions.
//!
//! **Every fixture in this file is hand-composed**, and that is stated rather
//! than left to be discovered (`AGENTS.md` §4, *label fixtures by provenance*).
//! None of it was captured from a running producer, because there is not one
//! yet. The field values are lifted from real Aura output — the JSON example in
//! `Aura/docs/ipc-schema.md` and the `BusDep` struct — but the framing around
//! them was written here.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// The shipped weather card, written the way `SPEC-04` §6 says to write it.
///
/// Values from `Aura/docs/ipc-schema.md`'s worked example: 70°F, a dewpoint of
/// 70, the `bkn` icon, and the statement *"Chance Rain Showers"*.
const HAND_WRITTEN_WEATHER_CARD: &str = "\
card 1
slot: secondary
ttl: 5400
block: reading
glyph: cloudy
value: 70
unit: °F
secondary: 62
secondary-unit: °F
caption: Cloudy
state: ok
role: normal
block: statement
text: Chance Rain Showers
role: muted
end
";

/// A departures board, and the point of it is that nothing in the format says
/// *bus*. Values shaped like `BusDep{route, destination, minutes}`.
const HAND_WRITTEN_DEPARTURES_CARD: &str = "\
card 1
slot: tertiary
ttl: 90
block: list
caption: Snelling & University
glyph: bus
row
lead: 21
label: Rosedale Center
trail: 7 min
role: warn
row
lead: 63
label: Grand Ave
trail: 21 min
role: normal
end
";

fn weather_card() -> Message {
    Message::Card(Card {
        slot: Slot::Secondary,
        ttl_seconds: 5400,
        blocks: vec![
            Block::Reading(Reading {
                glyph: Some(Glyph::Cloudy),
                value: Some("70".to_string()),
                unit: Some("°F".to_string()),
                secondary: Some("62".to_string()),
                secondary_unit: Some("°F".to_string()),
                caption: Some("Cloudy".to_string()),
                ..Reading::default()
            }),
            Block::Statement(Statement {
                text: "Chance Rain Showers".to_string(),
                role: Role::Muted,
            }),
        ],
    })
}

// ---------------------------------------------------------------------------
// The format itself, tested against bytes the writer did not produce
// ---------------------------------------------------------------------------

/// The one test that says anything about the **format**. Everything else in
/// this file that round-trips proves only that two functions agree.
#[test]
fn hand_written_bytes_decode_to_the_card_they_describe() {
    let got = Message::decode(HAND_WRITTEN_WEATHER_CARD).expect("the fixture should decode");
    assert_eq!(got, weather_card());
}

/// And the other direction: what the encoder writes is what §6 says to write.
#[test]
fn the_encoder_writes_the_form_the_spec_documents() {
    let written = weather_card().encode().expect("a valid card should encode");
    assert_eq!(written, HAND_WRITTEN_WEATHER_CARD);
}

// ---------------------------------------------------------------------------
// The list is not secretly a vocabulary of one
// ---------------------------------------------------------------------------

/// **The honest test of the whole design.**
///
/// The corner card's value and indoor temperature are both readings, so a
/// vocabulary derived only from Aura's existing cards could be far too narrow
/// and still look complete. A departures board is a genuinely different shape.
///
/// What this asserts is not that a bus fits — that would be a test of the
/// fixture. It is that **the same shape carries a day's agenda**, whose fields
/// have nothing to do with routes, destinations or minutes, and that a renderer
/// laying out either one can be described without either noun: a lead token, a
/// label, a trailing token.
#[test]
fn a_list_is_not_bus_shaped() {
    let departures = Message::decode(HAND_WRITTEN_DEPARTURES_CARD).expect("departures");
    let Message::Card(card) = departures else {
        panic!("expected a card")
    };
    let [Block::List(board)] = card.blocks.as_slice() else {
        panic!("expected one list")
    };
    assert_eq!(board.rows[0].lead.as_deref(), Some("21"));
    assert_eq!(board.rows[0].label, "Rosedale Center");
    assert_eq!(board.rows[0].trail.as_deref(), Some("7 min"));

    // The same shape, nothing to do with buses.
    let agenda = Message::Card(Card {
        slot: Slot::Quaternary,
        ttl_seconds: 3600,
        blocks: vec![Block::List(List {
            caption: Some("Today".to_string()),
            glyph: None,
            rows: vec![
                Row {
                    lead: Some("09:30".to_string()),
                    label: "Standup".to_string(),
                    trail: Some("30m".to_string()),
                    role: Role::Normal,
                },
                Row {
                    lead: Some("14:00".to_string()),
                    label: "Dentist".to_string(),
                    trail: None,
                    role: Role::Warn,
                },
            ],
        })],
    });
    let text = agenda.encode().expect("an agenda is a list");
    assert_eq!(Message::decode(&text).expect("round trip"), agenda);

    // A list with no lead and no trail at all — the degenerate case, which must
    // still be a list rather than needing a fourth shape.
    let plain = Message::Card(Card {
        slot: Slot::Quaternary,
        ttl_seconds: 60,
        blocks: vec![Block::List(List {
            caption: None,
            glyph: None,
            rows: vec![Row {
                label: "Milk".to_string(),
                ..Row::default()
            }],
        })],
    });
    let text = plain.encode().expect("a bare list is a list");
    assert_eq!(Message::decode(&text).expect("round trip"), plain);
}

// ---------------------------------------------------------------------------
// Round trips
// ---------------------------------------------------------------------------

/// Every shipped card shape, through both functions.
#[test]
fn every_shape_survives_a_round_trip() {
    let clock = Message::Card(Card {
        slot: Slot::Primary,
        ttl_seconds: 90,
        blocks: vec![Block::Reading(Reading {
            glyph: Some(Glyph::Clock),
            value: Some("22:15".to_string()),
            caption: Some("Monday, September 1".to_string()),
            ..Reading::default()
        })],
    });
    let gauge = Message::Card(Card {
        slot: Slot::Tertiary,
        ttl_seconds: 900,
        blocks: vec![Block::Reading(Reading {
            value: Some("2.4".to_string()),
            unit: Some("m".to_string()),
            trend: Some(Trend::Rising),
            state: State::Stale,
            role: Role::Alarm,
            age_seconds: Some(95),
            ..Reading::default()
        })],
    });
    let withdraw = Message::Withdraw {
        slot: Slot::Quaternary,
    };
    let pair = Message::decode(HAND_WRITTEN_PAIR_CARD).expect("the pair fixture decodes");
    for message in [clock, gauge, weather_card(), pair, withdraw] {
        let text = message.encode().expect("encodes");
        assert_eq!(Message::decode(&text).expect("decodes"), message);
    }
}

/// Every enumerated value has a name, and the name reads back. The failure this
/// catches is a variant added to one of the five `ALL` arrays and not to its
/// `as_str`, which produces a value nothing can parse.
#[test]
fn every_enumerated_value_round_trips_through_its_name() {
    for slot in Slot::ALL {
        assert_eq!(Slot::parse(slot.as_str()), Some(slot));
    }
    for role in Role::ALL {
        assert_eq!(Role::parse(role.as_str()), Some(role));
    }
    for trend in Trend::ALL {
        assert_eq!(Trend::parse(trend.as_str()), Some(trend));
    }
    for state in State::ALL {
        assert_eq!(State::parse(state.as_str()), Some(state));
    }
    for glyph in Glyph::ALL {
        assert_eq!(Glyph::parse(glyph.as_str()), Some(glyph));
    }
    for shape in Shape::ALL {
        assert_eq!(Shape::parse(shape.as_str()), Some(shape));
    }
}

/// A slot's wire name **is** its manifest endpoint name. If these ever diverge,
/// an addon declaring `connects = ["aura-renderer:secondary"]` would be granted
/// a socket for a slot it cannot name on the wire.
#[test]
fn a_slots_wire_name_is_a_legal_manifest_endpoint_name() {
    for slot in Slot::ALL {
        let name = slot.as_str();
        assert!(!name.is_empty());
        assert!(
            name.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "{name} would not survive rivet_core::package::application::Endpoint::parse"
        );
        assert!(!name.contains('/'), "{name} would be a path traversal");
    }
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

/// An addon built against a newer vocabulary than the renderer knows.
///
/// **The direction that matters.** The other way round — an old card, a new
/// renderer — is the ordinary case and must keep working, which the second half
/// asserts.
#[test]
fn a_newer_vocabulary_is_refused_by_number_and_an_older_one_is_not() {
    let newer = HAND_WRITTEN_WEATHER_CARD.replace("card 1", "card 2");
    assert_eq!(
        Message::decode(&newer),
        Err(Fault::UnknownVocabulary {
            theirs: 2,
            ours: VOCABULARY
        })
    );
    // Refusing a card is not refusing the producer.
    assert!(
        !Fault::UnknownVocabulary {
            theirs: 2,
            ours: VOCABULARY
        }
        .is_fatal(),
        "one card from the future must not close the connection"
    );
    // Vocabulary 0 does not exist, but a card claiming it is *not newer*, so it
    // is read as far as it can be. The refusal it earns is about its contents.
    let older = HAND_WRITTEN_WEATHER_CARD.replace("card 1", "card 0");
    assert!(Message::decode(&older).is_ok());
}

/// The three ways a line can fail to be a line.
#[test]
fn a_line_that_is_not_a_line_is_refused_rather_than_guessed_at() {
    for (bad, why) in [
        ("slot secondary", "no colon at all"),
        ("slot:secondary", "no space after the colon"),
        ("slot: ", "an empty value, which is spelled by omitting the key"),
        ("Slot: secondary", "a capital in a key"),
        ("slot two: x", "a space in a key"),
    ] {
        let text = format!("card 1\n{bad}\nttl: 60\nblock: statement\ntext: x\nend\n");
        assert!(
            matches!(
                Message::decode(&text),
                Err(Fault::NotALine { .. } | Fault::UnknownKey { .. })
            ),
            "{bad:?} was accepted, and it should not be: {why}"
        );
    }
}

/// Unknown keys and unknown values, told apart from keys that exist elsewhere.
#[test]
fn an_unknown_key_and_a_misplaced_one_are_different_refusals() {
    let unknown = "card 1\nslot: primary\nttl: 60\nblock: reading\nvalue: 1\nzork: 3\nend\n";
    assert_eq!(
        Message::decode(unknown),
        Err(Fault::UnknownKey {
            key: "zork".to_string()
        })
    );
    // `text` is a real key of a real shape, in the wrong shape. An author who
    // gets this back looks at their `block:` line, not at their spelling.
    let misplaced = "card 1\nslot: primary\nttl: 60\nblock: reading\nvalue: 1\ntext: hi\nend\n";
    assert_eq!(
        Message::decode(misplaced),
        Err(Fault::KeyOutOfScope {
            key: "text".to_string()
        })
    );
}

#[test]
fn an_unknown_shape_is_refused_and_the_vocabulary_is_named() {
    let text = "card 1\nslot: primary\nttl: 60\nblock: sparkline\nend\n";
    assert_eq!(
        Message::decode(text),
        Err(Fault::UnknownShape {
            shape: "sparkline".to_string()
        })
    );
}

#[test]
fn an_unknown_enumerated_value_names_the_key_and_the_word() {
    for (line, key) in [
        ("role: puce", "role"),
        ("trend: sideways", "trend"),
        ("state: probably", "state"),
        ("glyph: sharknado", "glyph"),
    ] {
        let text = format!("card 1\nslot: primary\nttl: 60\nblock: reading\nvalue: 1\n{line}\nend\n");
        assert_eq!(
            Message::decode(&text),
            Err(Fault::BadValue {
                key,
                value: line.split_once(": ").expect("a pair").1.to_string()
            }),
            "{line:?}"
        );
    }
    // `slot` lives in the card's scope, so its bad value is caught there.
    assert_eq!(
        Message::decode("card 1\nslot: corner\nttl: 60\nblock: reading\nvalue: 1\nend\n"),
        Err(Fault::BadValue {
            key: "slot",
            value: "corner".to_string()
        })
    );
}

#[test]
fn a_repeated_key_is_refused_because_two_answers_is_not_a_card() {
    let text = "card 1\nslot: primary\nslot: secondary\nttl: 60\nblock: reading\nvalue: 1\nend\n";
    assert_eq!(
        Message::decode(text),
        Err(Fault::RepeatedKey {
            key: "slot".to_string()
        })
    );
}

#[test]
fn a_missing_required_key_names_itself() {
    for (text, key) in [
        ("card 1\nttl: 60\nblock: reading\nvalue: 1\nend\n", "slot"),
        ("card 1\nslot: primary\nblock: reading\nvalue: 1\nend\n", "ttl"),
        ("card 1\nslot: primary\nttl: 60\nblock: statement\nrole: muted\nend\n", "text"),
        ("card 1\nslot: primary\nttl: 60\nblock: list\nrow\nlead: 1\nend\n", "label"),
        ("withdraw 1\nend\n", "slot"),
    ] {
        assert_eq!(
            Message::decode(text),
            Err(Fault::MissingKey { key }),
            "{text:?}"
        );
    }
}

/// The two contradictions, and both are rules Aura already enforced from the
/// producer's side.
#[test]
fn a_card_that_contradicts_itself_is_refused() {
    // A number nobody vouches for.
    let stale_number =
        "card 1\nslot: primary\nttl: 60\nblock: reading\nvalue: 2.4\nstate: unavailable\nend\n";
    assert!(matches!(
        Message::decode(stale_number),
        Err(Fault::Contradiction { .. })
    ));
    // A reading that would draw nothing.
    let nothing = "card 1\nslot: primary\nttl: 60\nblock: reading\nunit: m\nend\n";
    assert!(matches!(
        Message::decode(nothing),
        Err(Fault::Contradiction { .. })
    ));
    // The same two, refused by the *encoder*, so a producer cannot emit them.
    let bad = Message::Card(Card {
        slot: Slot::Primary,
        ttl_seconds: 60,
        blocks: vec![Block::Reading(Reading {
            value: Some("2.4".to_string()),
            state: State::Connecting,
            ..Reading::default()
        })],
    });
    assert!(matches!(bad.encode(), Err(Fault::Contradiction { .. })));
}

#[test]
fn an_empty_card_and_an_empty_list_are_refused() {
    assert_eq!(
        Message::decode("card 1\nslot: primary\nttl: 60\nend\n"),
        Err(Fault::Empty { what: "card" })
    );
    assert_eq!(
        Message::decode("card 1\nslot: primary\nttl: 60\nblock: list\nend\n"),
        Err(Fault::Empty { what: "list" })
    );
}

#[test]
fn a_row_outside_a_list_has_nowhere_to_go() {
    let text = "card 1\nslot: primary\nttl: 60\nblock: reading\nvalue: 1\nrow\nend\n";
    assert_eq!(
        Message::decode(text),
        Err(Fault::Misplaced { word: "row" })
    );
}

/// Control characters, including the one that would silently become framing.
#[test]
fn a_value_carrying_a_control_character_is_not_text() {
    // A bare `\r` inside a value — not the `\r\n` a line may end with.
    let text = "card 1\nslot: primary\nttl: 60\nblock: statement\ntext: a\u{7}b\nend\n";
    assert_eq!(Message::decode(text), Err(Fault::NotText));
    // And the encoder refuses to produce one.
    let bad = Message::Card(Card {
        slot: Slot::Primary,
        ttl_seconds: 60,
        blocks: vec![Block::Statement(Statement {
            text: "two\nlines".to_string(),
            role: Role::Normal,
        })],
    });
    assert_eq!(bad.encode(), Err(Fault::NotText));
}

/// Non-ASCII is fine and must stay fine: a forecast is prose somebody else
/// wrote, and `°` is in every temperature Aura has ever drawn.
#[test]
fn text_that_is_not_ascii_is_carried_unchanged() {
    let message = Message::Card(Card {
        slot: Slot::Secondary,
        ttl_seconds: 60,
        blocks: vec![Block::Statement(Statement {
            text: "Nubes — 21 °C, mostly cloudy… 大丈夫".to_string(),
            role: Role::Normal,
        })],
    });
    let text = message.encode().expect("UTF-8 is admitted");
    assert_eq!(Message::decode(&text).expect("round trip"), message);
}

#[test]
fn every_bound_in_limits_is_enforced() {
    let long = "x".repeat(LIMITS.value_bytes + 1);
    let text = format!("card 1\nslot: primary\nttl: 60\nblock: statement\ntext: {long}\nend\n");
    assert_eq!(
        Message::decode(&text),
        Err(Fault::TooLong {
            what: "value",
            limit: LIMITS.value_bytes
        })
    );

    let mut blocks = String::new();
    for _ in 0..=LIMITS.blocks {
        blocks.push_str("block: statement\ntext: x\n");
    }
    let text = format!("card 1\nslot: primary\nttl: 60\n{blocks}end\n");
    assert!(matches!(
        Message::decode(&text),
        Err(Fault::TooLong { what: "card's blocks", .. })
    ));

    let mut rows = String::new();
    for i in 0..=LIMITS.rows {
        rows.push_str(&format!("row\nlabel: row {i}\n"));
    }
    let text = format!("card 1\nslot: primary\nttl: 60\nblock: list\n{rows}end\n");
    assert!(matches!(
        Message::decode(&text),
        Err(Fault::TooLong { what: "list's rows", .. })
    ));
}

#[test]
fn a_message_with_no_end_is_unterminated_rather_than_half_read() {
    assert_eq!(
        Message::decode("card 1\nslot: primary\nttl: 60\nblock: reading\nvalue: 1\n"),
        Err(Fault::Unterminated)
    );
    assert_eq!(Message::decode(""), Err(Fault::Unterminated));
}

#[test]
fn a_first_line_that_is_not_a_verb_says_so() {
    for bad in ["hello 1\nend\n", "card\nend\n", "card x\nend\n", "1 card\nend\n"] {
        assert!(
            matches!(Message::decode(bad), Err(Fault::UnknownVerb { .. })),
            "{bad:?} was accepted"
        );
    }
}

// ---------------------------------------------------------------------------
// Framing
// ---------------------------------------------------------------------------

/// A socket delivers whatever it delivers. This is the test that the framer
/// does not care.
#[test]
fn a_message_split_across_reads_is_reassembled() {
    let whole = HAND_WRITTEN_WEATHER_CARD.as_bytes();
    for split in 1..whole.len() {
        let mut framer = Framer::new();
        framer.push(&whole[..split]).expect("first half");
        // A `blit`-sized truth: `next()` says None until the `end` line lands.
        let early = framer.next_message();
        framer.push(&whole[split..]).expect("second half");
        let got = match early {
            Some(m) => m,
            None => framer.next_message().expect("the whole message has arrived now"),
        };
        assert_eq!(got.expect("decodes"), weather_card(), "split at {split}");
    }
}

#[test]
fn several_messages_in_one_read_are_all_delivered() {
    let mut framer = Framer::new();
    let both = format!("{HAND_WRITTEN_WEATHER_CARD}{HAND_WRITTEN_DEPARTURES_CARD}");
    framer.push(both.as_bytes()).expect("push");
    assert_eq!(framer.next_message().expect("first").expect("decodes"), weather_card());
    let second = framer.next_message().expect("second").expect("decodes");
    assert_eq!(second.slot(), Slot::Tertiary);
    assert!(framer.next_message().is_none(), "there were only two");
}

/// **One bad card does not end the conversation.** The distinction
/// `Fault::is_fatal` draws, tested from the framer's side.
#[test]
fn a_refused_card_is_followed_by_the_next_one() {
    let mut framer = Framer::new();
    let bad = HAND_WRITTEN_WEATHER_CARD.replace("card 1", "card 99");
    framer
        .push(format!("{bad}{HAND_WRITTEN_DEPARTURES_CARD}").as_bytes())
        .expect("push");
    let refused = framer.next_message().expect("a message").expect_err("refused");
    assert!(!refused.is_fatal());
    let next = framer.next_message().expect("the next one").expect("decodes");
    assert_eq!(next.slot(), Slot::Tertiary);
}

/// A producer that never sends an `end` is answered with a sentence rather than
/// an out-of-memory kill, and the stream is then poisoned because there is no
/// position to resume from.
#[test]
fn a_message_that_never_ends_is_bounded_and_fatal() {
    let mut framer = Framer::new();
    let filler = format!("caption: {}\n", "x".repeat(200));
    let mut sent = 0;
    while sent <= LIMITS.message_bytes {
        framer.push(filler.as_bytes()).expect("push");
        sent += filler.len();
    }
    let fault = framer.next_message().expect("a fault").expect_err("refused");
    assert!(fault.is_fatal(), "{fault}");
    assert!(matches!(fault, Fault::Overran { .. }), "{fault:?}");
    // Poisoned: it says the same thing rather than pretending it recovered.
    framer.push(b"card 1\nslot: primary\nttl: 1\nend\n").expect_err("poisoned");
}

/// **A bound broken INSIDE a finished message is one bad card, not a bad
/// producer** — `SPEC-04` §6.5, corrected 2026-09-04.
///
/// Every `Fault::TooLong` used to answer `is_fatal() == true`, so a single
/// forecast string over 256 bytes closed the connection and had the supervisor
/// restart the addon that sent it. The framer had already drained that message
/// at its `end`: it knew exactly where the next card began and threw the
/// conversation away anyway. `Fault::Overran` — a message that passed the bound
/// with no `end` at all — is the only one where that response is right, because
/// it is the only one with no boundary left to resume from.
#[test]
fn a_bound_broken_inside_a_finished_message_refuses_the_card_and_keeps_the_producer() {
    let long = "x".repeat(LIMITS.value_bytes + 1);
    let over = format!("card 1\nslot: primary\nttl: 60\nblock: statement\ntext: {long}\nend\n");

    // Every content bound, decoded on its own: none of them is fatal.
    let mut blocks = String::new();
    for _ in 0..=LIMITS.blocks {
        blocks.push_str("block: statement\ntext: x\n");
    }
    let mut rows = String::new();
    for i in 0..=LIMITS.rows {
        rows.push_str(&format!("row\nlabel: row {i}\n"));
    }
    let mut lines = String::new();
    for _ in 0..=LIMITS.lines {
        lines.push_str("text: x\n");
    }
    for (text, what) in [
        (over.clone(), "value"),
        (
            format!("card 1\nslot: primary\nttl: 60\n{blocks}end\n"),
            "card's blocks",
        ),
        (
            format!("card 1\nslot: primary\nttl: 60\nblock: list\n{rows}end\n"),
            "list's rows",
        ),
        (
            format!("card 1\nslot: primary\nttl: 60\nblock: statement\n{lines}end\n"),
            "message's lines",
        ),
    ] {
        let fault = Message::decode(&text).expect_err("over its bound");
        assert!(
            matches!(fault, Fault::TooLong { .. }),
            "{what}: {fault:?}"
        );
        assert!(
            !fault.is_fatal(),
            "{what} is one bad card and must not disconnect the producer: {fault}"
        );
    }

    // And from the framer's side, which is where it costs something: the
    // over-long card is refused and the next one still arrives.
    let mut framer = Framer::new();
    framer
        .push(format!("{over}{HAND_WRITTEN_DEPARTURES_CARD}").as_bytes())
        .expect("push");
    let refused = framer.next_message().expect("a message").expect_err("refused");
    assert!(!refused.is_fatal(), "{refused}");
    let next = framer
        .next_message()
        .expect("the producer was not disconnected")
        .expect("the next card decodes");
    assert_eq!(next.slot(), Slot::Tertiary);
}

/// **A complete message over 8 KiB is content; an unfinished one is framing.**
///
/// The two used to be the same `Fault::TooLong { what: "message" }` and they are
/// not the same event. This pins the pair so the next tidy cannot merge them
/// again.
#[test]
fn a_whole_oversized_message_is_content_and_an_unfinished_one_is_framing() {
    // Complete: it has its `end`, so the framer knows where the next one starts.
    let filler = "caption: ".to_string() + &"x".repeat(200) + "\n";
    let mut body = String::new();
    while body.len() <= LIMITS.message_bytes {
        body.push_str(&filler);
    }
    let whole = format!("card 1\nslot: primary\nttl: 60\n{body}end\n");
    let fault = Message::decode(&whole).expect_err("over the message bound");
    assert!(matches!(fault, Fault::TooLong { what: "message", .. }), "{fault:?}");
    assert!(!fault.is_fatal(), "a message with an `end` has a boundary: {fault}");

    // Unfinished: no `end` anywhere, so there is nothing to resume from.
    assert!(Fault::Overran { limit: LIMITS.message_bytes }.is_fatal());
}

/// A whole message that is not text, told apart from a character split across
/// two reads — which is what the test above covers and which must **not** be
/// treated as a fault.
#[test]
fn a_message_that_is_not_utf8_is_fatal() {
    let mut framer = Framer::new();
    framer.push(b"card 1\nslot: primary\nttl: 60\nblock: statement\ntext: ").expect("push");
    framer.push(&[0xff, 0xfe]).expect("bytes are not judged on arrival");
    framer.push(b"\nend\n").expect("push");
    let fault = framer.next_message().expect("a message ended").expect_err("not text");
    assert_eq!(fault, Fault::NotText);
    assert!(fault.is_fatal());
    framer.push(b"anything").expect_err("poisoned");
}

/// `\r\n` line endings, because somebody will write a producer on Windows.
#[test]
fn windows_line_endings_are_accepted() {
    let crlf = HAND_WRITTEN_WEATHER_CARD.replace('\n', "\r\n");
    let mut framer = Framer::new();
    framer.push(crlf.as_bytes()).expect("push");
    assert_eq!(
        framer.next_message().expect("a message").expect("decodes"),
        weather_card()
    );
}

// ---------------------------------------------------------------------------
// The regression this file exists to prevent
// ---------------------------------------------------------------------------

/// **Aura's most expensive rendering bug, restated as a protocol rule.**
///
/// `cpp/src/main.cpp` records it: `.` was missing from the bold font atlas, and
/// raylib draws an absent codepoint as the atlas's *first* glyph, which was
/// `0`. A reading of 6.5 therefore appeared on the wall as `605`.
/// *"Nothing failed and nothing logged; the number was simply wrong."*
///
/// The lesson is not about fonts. It is that **a partial answer that looks
/// whole is worse than a refusal**, which is why an unknown key refuses the
/// card rather than being skipped the way Aura's JSON reader skipped unknown
/// fields. This test asserts the choice so that a later kindness cannot quietly
/// reverse it.
#[test]
fn an_unknown_key_refuses_the_card_rather_than_drawing_the_rest_of_it() {
    let text = "card 1\nslot: primary\nttl: 60\nblock: reading\n\
                value: 6.5\nunit: m\nprecision: 1\nend\n";
    assert!(
        Message::decode(text).is_err(),
        "a card with one key the renderer does not understand must not be drawn \
         with that key ignored"
    );
}

// ---------------------------------------------------------------------------
// `pair` — SPEC-04 §5.8, added 2026-09-02
// ---------------------------------------------------------------------------

/// The shipped weather card **as it is actually composed**, written by hand
/// from `SPEC-04` §5.8.1 and §6.4. Hand-composed, like every fixture here.
///
/// The `right` column's values are the ones a **contributor** supplies (§4.5):
/// a room temperature over a humidity, which is why its second unit is `%` and
/// not a degree.
const HAND_WRITTEN_PAIR_CARD: &str = "\
card 1
slot: secondary
ttl: 5400
block: pair
left
glyph: cloudy
value: 70
unit: °F
secondary: 62
secondary-unit: °F
right
glyph: indoor
value: 76
unit: °F
secondary: 54
secondary-unit: %
block: statement
text: Chance Rain Showers
end
";

/// The `pair` half of [`hand_written_bytes_decode_to_the_card_they_describe`],
/// and it is the same argument: bytes the writer did not produce.
#[test]
fn hand_written_pair_bytes_decode_to_the_two_columns_they_describe() {
    let got = Message::decode(HAND_WRITTEN_PAIR_CARD).expect("the fixture should decode");
    let Message::Card(card) = got else {
        panic!("expected a card")
    };
    let [Block::Pair(pair), Block::Statement(statement)] = card.blocks.as_slice() else {
        panic!("expected a pair and a statement, got {:?}", card.blocks)
    };
    assert_eq!(pair.left.value.as_deref(), Some("70"));
    assert_eq!(pair.left.glyph, Some(Glyph::Cloudy));
    assert_eq!(pair.right.value.as_deref(), Some("76"));
    assert_eq!(pair.right.secondary_unit.as_deref(), Some("%"));
    assert_eq!(statement.text, "Chance Rain Showers");
    // One block, not two. §5.7: a comparison drawn as one unit is one piece of
    // composition, and it counts once against the bound.
    assert_eq!(card.blocks.len(), 2);
}

/// A pair survives the writer and the reader, both sides intact and in order.
#[test]
fn a_pair_survives_a_round_trip_with_its_sides_the_right_way_round() {
    let message = Message::decode(HAND_WRITTEN_PAIR_CARD).expect("decodes");
    let text = message.encode().expect("encodes");
    assert_eq!(Message::decode(&text).expect("decodes again"), message);
    // The canonical order the encoder writes.
    assert!(text.contains("block: pair\nleft\n"), "{text}");
    assert!(text.contains("\nright\n"), "{text}");
    assert!(
        text.find("\nleft\n") < text.find("\nright\n"),
        "left is written before right: {text}"
    );
}

/// **The test §5.3.1 applied to `list`, applied to `pair` for the same reason.**
///
/// The corner card's value and indoor temperature are both readings, so a
/// vocabulary derived only from Aura's cards can look complete and be far too
/// narrow. `pair` was
/// added for a card that compares an outdoor temperature with an indoor one; if
/// the shape had absorbed anything about *weather*, these three would not fit.
///
/// None of them is weather, none of them is a temperature, and the decoder
/// cannot tell which it has.
#[test]
fn a_pair_is_not_weather_shaped() {
    let fillings = [
        // A two-zone thermostat.
        ("68", "°F", "71", "°F"),
        // A battery bank: charge beside draw.
        ("94", "%", "1.4", "kW"),
        // One match, home and away.
        ("2", "", "1", ""),
    ];
    for (lv, lu, rv, ru) in fillings {
        let pair = Pair {
            left: Reading {
                value: Some(lv.to_string()),
                unit: (!lu.is_empty()).then(|| lu.to_string()),
                ..Reading::default()
            },
            right: Reading {
                value: Some(rv.to_string()),
                unit: (!ru.is_empty()).then(|| ru.to_string()),
                ..Reading::default()
            },
        };
        let message = Message::Card(Card {
            slot: Slot::Secondary,
            ttl_seconds: 300,
            blocks: vec![Block::Pair(pair)],
        });
        let text = message
            .encode()
            .expect("every one of these is an ordinary pair");
        assert_eq!(Message::decode(&text).expect("decodes"), message);
    }
}

/// **Both sides are required**, and the refusal names the one that is missing.
///
/// §5.8.1: an optional `right` and a `state: unavailable` `right` would be two
/// ways to say one thing, and only one of them says why.
#[test]
fn a_pair_with_one_side_is_refused_by_name() {
    let one_side = "card 1\nslot: secondary\nttl: 60\nblock: pair\nleft\nvalue: 70\nend\n";
    assert_eq!(
        Message::decode(one_side),
        Err(Fault::MissingKey { key: "right" })
    );
    let neither = "card 1\nslot: secondary\nttl: 60\nblock: pair\nend\n";
    assert_eq!(
        Message::decode(neither),
        Err(Fault::MissingKey { key: "left" })
    );
}

/// A side given twice is two answers to one question, exactly as a repeated key
/// is — so it is the same refusal.
#[test]
fn a_side_given_twice_is_refused_the_way_a_repeated_key_is() {
    let text = "card 1\nslot: secondary\nttl: 60\nblock: pair\n\
                left\nvalue: 70\nleft\nvalue: 71\nright\nvalue: 76\nend\n";
    assert_eq!(
        Message::decode(text),
        Err(Fault::RepeatedKey {
            key: "left".to_string()
        })
    );
}

/// `left` and `right` belong to a `pair` and nowhere else. Inside a `list` the
/// mistake would otherwise be silent — a row would simply stop being read.
#[test]
fn left_and_right_are_misplaced_outside_a_pair() {
    for (text, word) in [
        (
            "card 1\nslot: secondary\nttl: 60\nblock: reading\nleft\nvalue: 70\nend\n",
            "left",
        ),
        (
            "card 1\nslot: tertiary\nttl: 60\nblock: list\nrow\nlabel: a\nright\nend\n",
            "right",
        ),
        ("card 1\nslot: secondary\nttl: 60\nleft\nend\n", "left"),
    ] {
        assert_eq!(
            Message::decode(text),
            Err(Fault::Misplaced { word }),
            "{text}"
        );
    }
}

/// **A pair has no keys of its own** — no widths, no alignment, no rule colour.
/// Those are the renderer's, and a key between `block: pair` and its first side
/// has nothing it could mean.
#[test]
fn a_pair_carries_no_keys_of_its_own() {
    let text = "card 1\nslot: secondary\nttl: 60\nblock: pair\nvalue: 70\nleft\n\
                value: 70\nright\nvalue: 76\nend\n";
    assert_eq!(
        Message::decode(text),
        Err(Fault::KeyOutOfScope {
            key: "value".to_string()
        })
    );
}

/// Each side is held to **the reading rules**, both of them, unchanged. A right
/// column carrying a value its state does not admit is the `605` failure with a
/// column in front of it.
#[test]
fn each_side_of_a_pair_is_held_to_the_reading_rules() {
    let contradiction = "card 1\nslot: secondary\nttl: 60\nblock: pair\n\
                         left\nvalue: 70\nright\nvalue: 76\nstate: unavailable\nend\n";
    assert!(matches!(
        Message::decode(contradiction),
        Err(Fault::Contradiction { .. })
    ));
    // And the empty-reading rule, which is what a producer hits if it opens a
    // side and says nothing at all.
    let empty_side = "card 1\nslot: secondary\nttl: 60\nblock: pair\n\
                      left\nvalue: 70\nright\nend\n";
    assert!(matches!(
        Message::decode(empty_side),
        Err(Fault::Contradiction { .. })
    ));
}

/// **A side with nothing to report is spelled with its `state`**, which is the
/// form `SPEC-04` §5.8.1 requires and the one the shipped producer sends when a
/// contributor has gone quiet. It must be accepted.
#[test]
fn a_side_that_has_nothing_to_report_says_so_with_its_state() {
    let text = "card 1\nslot: secondary\nttl: 60\nblock: pair\n\
                left\nglyph: cloudy\nvalue: 70\nunit: °F\nright\nstate: unavailable\nend\n";
    let Message::Card(card) = Message::decode(text).expect("this is a legal pair") else {
        panic!("expected a card")
    };
    let [Block::Pair(pair)] = card.blocks.as_slice() else {
        panic!("expected one pair")
    };
    assert_eq!(pair.right.value, None);
    assert_eq!(pair.right.state, State::Unavailable);
    assert_eq!(
        pair.right.glyph, None,
        "no glyph, so the column is not drawn"
    );
}

/// The shape is in `Shape::ALL`, so
/// `every_enumerated_value_round_trips_through_its_name` covers its name — this
/// asserts the array itself did not stay at three, which is the mistake that
/// would leave `pair` unparseable on the wire.
#[test]
fn the_vocabulary_of_shapes_is_the_four_the_spec_lists() {
    let names: Vec<&str> = Shape::ALL.iter().map(|s| s.as_str()).collect();
    assert_eq!(names, ["reading", "statement", "list", "pair"]);
}
