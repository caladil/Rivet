//! **The wire form** — one function that writes and one that reads, both
//! visible.
//!
//! `SPEC-04` §6 is the normative statement of everything here.
//!
//! # Why a line format and not JSON
//!
//! `rivet_core::json` already argued this for the control plane and the
//! argument transfers: this workspace carries no serialisation framework, and
//! *"every wire format in it is a function that writes and a function that
//! reads, both visible."* That crate then had to write a JSON reader anyway,
//! because JSON-RPC is what a client speaks.
//!
//! Nothing forces JSON here. The one thing that does apply pressure is
//! `SPEC-03` §1's standard for a Rivet contract — a second language must be
//! able to consume it in about two hundred lines — and a format that is
//! `key: value` lines between two sentinels meets that in any language with a
//! string split. A JSON one would meet it too, in languages that ship a
//! parser, and not at all in the ones that do not.
//!
//! # It is legible in a journal, which is the deciding property
//!
//! A card that arrives wrong will be diagnosed by a person reading a log line,
//! and `value: 70` beats `{"v":"70"}` at three in the morning. `ADR-022`'s
//! journal is where a refused card is going to be reported, and a wire form
//! that survives being pasted into that report is worth more than a compact one.
//!
//! # The rules, in full
//!
//! * **UTF-8**, and no control character anywhere — nothing below `0x20`
//!   except the `\n` that ends a line, and not `0x7f`. A weather statement is
//!   prose somebody else wrote, so this is not restricted to ASCII the way
//!   `rivet_core::package::application`'s manifest is; `°` and `–` must
//!   survive.
//! * **Every line is one of four shapes**: the opening `card <n>` or
//!   `withdraw <n>`, the bare words `row`, `left`, `right` and `end`, or
//!   `key: value`.
//! * **The separator is a colon and exactly one space.** Everything after it,
//!   to the end of the line, is the value — no quoting, no escapes, and
//!   therefore nothing for two implementations to disagree about.
//! * **A value is never empty.** Absent is spelled by omitting the key, which
//!   is the same rule `SPEC-03` §3.0 states for the handed environment:
//!   *absent, not empty*, throughout.
//! * **A key never repeats within its scope.** Two answers to one question is
//!   the failure the house's canonical-order rule actually guards against.
//! * **Key order is not enforced.** This is a deliberate departure from
//!   `Manifest::parse`, which demands the canonical order because a manifest is
//!   *emitted* and anything else means a second producer. A card is emitted by
//!   programs Aura did not build, in languages Aura did not choose, and
//!   refusing a correct card because its author sorted the keys differently
//!   would be refusing for a cosmetic reason. [`Message::encode`] still writes
//!   one canonical order, so Aura's own output is stable and diffable.

use crate::{
    Block, Card, Glyph, List, Message, Pair, Reading, Role, Row, Shape, Slot, State, Statement,
    Trend, VOCABULARY,
};

// ---------------------------------------------------------------------------
// Bounds
// ---------------------------------------------------------------------------

/// Every bound the wire has.
///
/// A boundary is bounded in this repository as a matter of course —
/// `rivet_core::json`'s note is the reference: the bytes reaching a reader were
/// written by something else, so it is strict rather than generous, and it is
/// bounded in ways a hand-written parser usually is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most one message may be, in bytes, framing included.
    pub message_bytes: usize,
    /// The most one value may be, in bytes.
    pub value_bytes: usize,
    /// The most lines one message may have.
    pub lines: usize,
    /// The most blocks one card may carry.
    pub blocks: usize,
    /// The most rows one list may carry.
    pub rows: usize,
}

/// The bounds this build enforces.
///
/// `value_bytes` is sized on the longest thing Aura has ever put on the wire: an
/// NWS `shortForecast` such as *"Chance Rain Showers then Slight Chance Rain
/// Showers"*, which is under sixty. 256 leaves room for a language whose words
/// are longer and for a `caption` a producer thought was clever.
///
/// `blocks` is 4 because the richest card in the shipped set is two — a reading
/// and a statement — and because a card that needs five blocks is a card that
/// wants a slot of its own.
///
/// `rows` is 12 because a departure board that does not fit on a quarter of a
/// 1080p screen has stopped being glanceable, and choosing which twelve is the
/// producer's job.
pub const LIMITS: Limits = Limits {
    message_bytes: 8 * 1024,
    value_bytes: 256,
    lines: 128,
    blocks: 4,
    rows: 12,
};

// ---------------------------------------------------------------------------
// Faults
// ---------------------------------------------------------------------------

/// Why a message could not be read, or could not be written.
///
/// Every variant names the key, the shape or the number, per `AGENTS.md` §5: a
/// reader that says *"bad card"* sends its author back to the encoder with
/// nothing to look for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// Bytes that are not UTF-8, or that carry a control character.
    NotText,
    /// A line that is none of the four shapes the format has.
    NotALine {
        /// The offending line, truncated to [`Limits::value_bytes`].
        line: String,
    },
    /// A first line that is neither `card <n>` nor `withdraw <n>`.
    UnknownVerb {
        /// What was there instead.
        verb: String,
    },
    /// A card written against a vocabulary this build does not know.
    ///
    /// **The forward direction only.** A card declaring a vocabulary at or
    /// below [`VOCABULARY`] is accepted; one above it is refused, because a
    /// reader cannot know what it is not being told.
    UnknownVocabulary {
        /// What the producer declared.
        theirs: u32,
        /// What this build speaks.
        ours: u32,
    },
    /// A key this vocabulary does not define, in the scope it appeared in.
    UnknownKey {
        /// The key.
        key: String,
    },
    /// A key that belongs to a different scope — `text` outside a statement,
    /// `label` outside a row.
    KeyOutOfScope {
        /// The key.
        key: String,
    },
    /// A key that must be present and is not.
    MissingKey {
        /// The key.
        key: &'static str,
    },
    /// A key given twice in one scope.
    RepeatedKey {
        /// The key.
        key: String,
    },
    /// A value that is not one of the words this key admits.
    BadValue {
        /// The key.
        key: &'static str,
        /// What was there.
        value: String,
    },
    /// A `block:` naming a shape that is not in [`Shape::ALL`].
    UnknownShape {
        /// What was named.
        shape: String,
    },
    /// A `row` outside a list, or an `end` before anything was opened.
    Misplaced {
        /// The word.
        word: &'static str,
    },
    /// Two keys that cannot both be true.
    Contradiction {
        /// What is wrong, in a sentence.
        why: &'static str,
    },
    /// A card with no blocks, or a list with no rows.
    Empty {
        /// What was empty.
        what: &'static str,
    },
    /// A value, a line or a message past its bound in [`LIMITS`].
    ///
    /// **Content, not framing** — `SPEC-04` §6.5. Every one of these is raised
    /// about a message whose `end` has already been found, so the reader knows
    /// exactly where the next one starts. See [`Overran`] for the case that
    /// really is fatal, and [`Fault::is_fatal`] for why the two were once one.
    ///
    /// [`Overran`]: Fault::Overran
    TooLong {
        /// Which bound.
        what: &'static str,
        /// The bound.
        limit: usize,
    },
    /// A message that ran out before its `end`.
    Unterminated,
    /// A message that passed [`LIMITS`]`.message_bytes` **without ending**.
    ///
    /// The framing failed rather than one card being bad: there is no `end`, so
    /// there is no position in the stream to resume from. Distinct from
    /// [`TooLong`] with `what: "message"`, which is a complete message that is
    /// merely too big — that one the framer has already drained and can carry
    /// on past.
    ///
    /// [`TooLong`]: Fault::TooLong
    Overran {
        /// The bound the unfinished message passed.
        limit: usize,
    },
}

impl Fault {
    /// Whether the **stream** is unreadable, rather than one message being bad.
    ///
    /// The split matters to a caller and is `SPEC-04` §6.5:
    ///
    /// * A **fatal** fault means the framing itself failed — bytes that are not
    ///   text, or a message that overran its bound without ending. There is no
    ///   position from which to resume, so the renderer closes the connection
    ///   and lets the supervisor restart the producer.
    /// * Anything else is one bad card. The renderer refuses **that card**,
    ///   says so once in the journal, leaves the slot holding whatever it held,
    ///   and reads the next message. A producer that sent one card from a newer
    ///   vocabulary is not a producer that should be disconnected.
    ///
    /// # Every `TooLong` used to be fatal, and that was the wrong half
    ///
    /// Corrected 2026-09-04. `TooLong` covered two unrelated things: a bound
    /// broken **inside a message the framer had already drained** — a value over
    /// 256 bytes, a fifth block, a thirteenth row — and a producer that never
    /// sent an `end` at all. Only the second is a framing failure, and grouping
    /// them meant one over-long forecast string **disconnected the producer**
    /// and had the supervisor restart it, for a card the reader could have
    /// refused and stepped over. That is the widest possible response to the
    /// narrowest possible fault, and §6.5's table says the opposite.
    ///
    /// The second case is now [`Fault::Overran`], which is what this matches on.
    #[must_use]
    pub const fn is_fatal(&self) -> bool {
        matches!(
            self,
            Self::NotText | Self::Unterminated | Self::Overran { .. }
        )
    }
}

impl core::fmt::Display for Fault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotText => write!(
                f,
                "a card is UTF-8 text and carries no control characters; these bytes are not that"
            ),
            Self::NotALine { line } => write!(
                f,
                "`{line}` is not a line: a card has `card <n>`, `withdraw <n>`, `row`, `end`, \
                 and `key: value` with one space after the colon"
            ),
            Self::UnknownVerb { verb } => write!(
                f,
                "a message begins `card <n>` or `withdraw <n>`, not `{verb}`"
            ),
            Self::UnknownVocabulary { theirs, ours } => write!(
                f,
                "this card declares vocabulary {theirs} and this renderer speaks {ours}; \
                 it is refused because a reader cannot draw what it has not been told about. \
                 The slot keeps what it had"
            ),
            Self::UnknownKey { key } => write!(f, "`{key}` is not a key this vocabulary defines"),
            Self::KeyOutOfScope { key } => write!(
                f,
                "`{key}` is a key of a different shape and does not belong here"
            ),
            Self::MissingKey { key } => write!(f, "`{key}` is required and is absent"),
            Self::RepeatedKey { key } => write!(
                f,
                "`{key}` appears twice, and two answers to one question is not a card"
            ),
            Self::BadValue { key, value } => {
                write!(f, "`{value}` is not a value the `{key}` key admits")
            }
            Self::UnknownShape { shape } => write!(
                f,
                "`{shape}` is not a shape. The vocabulary is closed: reading, statement, list, pair"
            ),
            Self::Misplaced { word } => write!(f, "`{word}` is not in a place it can appear"),
            Self::Contradiction { why } => write!(f, "{why}"),
            Self::Empty { what } => write!(f, "a {what} with nothing in it is not a {what}"),
            Self::TooLong { what, limit } => write!(f, "a {what} is at most {limit}"),
            Self::Unterminated => write!(f, "the message ran out before its `end`"),
            Self::Overran { limit } => write!(
                f,
                "a message passed {limit} bytes with no `end` line, so there is no message \
                 boundary left to read from"
            ),
        }
    }
}

impl std::error::Error for Fault {}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

impl Message {
    /// Write this message, framing included, ready to put on a socket.
    ///
    /// # Why writing can fail
    ///
    /// Because it validates exactly what [`Message::decode`] validates, which
    /// buys one property worth the awkwardness: **anything this function
    /// produces, that function accepts.** A producer cannot emit a card its own
    /// renderer will refuse, and the two halves cannot drift, because there is
    /// one set of rules and both of them run it.
    ///
    /// # Errors
    ///
    /// [`Fault`], naming the field.
    pub fn encode(&self) -> Result<String, Fault> {
        let mut out = String::new();
        match self {
            Self::Withdraw { slot } => {
                out.push_str(&format!("withdraw {VOCABULARY}\n"));
                out.push_str(&format!("slot: {}\n", slot.as_str()));
            }
            Self::Card(card) => {
                validate(card)?;
                out.push_str(&format!("card {VOCABULARY}\n"));
                out.push_str(&format!("slot: {}\n", card.slot.as_str()));
                out.push_str(&format!("ttl: {}\n", card.ttl_seconds));
                for block in &card.blocks {
                    out.push_str(&format!("block: {}\n", block.shape().as_str()));
                    write_block(&mut out, block);
                }
            }
        }
        out.push_str("end\n");
        if out.len() > LIMITS.message_bytes {
            return Err(Fault::TooLong {
                what: "message",
                limit: LIMITS.message_bytes,
            });
        }
        Ok(out)
    }
}

/// One key, written only if it is there. The `absent, not empty` rule, in the
/// one place that could break it.
fn write_opt(out: &mut String, key: &str, value: Option<&str>) {
    if let Some(v) = value {
        out.push_str(&format!("{key}: {v}\n"));
    }
}

fn write_block(out: &mut String, block: &Block) {
    match block {
        Block::Reading(r) => write_reading(out, r),
        Block::Pair(p) => {
            // The two bare words are `row`'s twins: they open a scope, and the
            // keys after one belong to it. Written in this order because the
            // encoder writes one canonical order (§6.3 rule 5) even though a
            // reader accepts either.
            out.push_str("left\n");
            write_reading(out, &p.left);
            out.push_str("right\n");
            write_reading(out, &p.right);
        }
        Block::Statement(s) => {
            out.push_str(&format!("text: {}\n", s.text));
            out.push_str(&format!("role: {}\n", s.role.as_str()));
        }
        Block::List(l) => {
            write_opt(out, "caption", l.caption.as_deref());
            write_opt(out, "glyph", l.glyph.map(Glyph::as_str));
            for row in &l.rows {
                out.push_str("row\n");
                write_opt(out, "lead", row.lead.as_deref());
                out.push_str(&format!("label: {}\n", row.label));
                write_opt(out, "trail", row.trail.as_deref());
                out.push_str(&format!("role: {}\n", row.role.as_str()));
            }
        }
    }
}

/// One reading's keys, in canonical order.
///
/// Separated from [`write_block`] when [`Pair`] arrived, because a pair writes
/// two of them and a second copy of this list is a second answer to *what is a
/// reading* (`ADR-011`).
fn write_reading(out: &mut String, r: &Reading) {
    write_opt(out, "glyph", r.glyph.map(Glyph::as_str));
    write_opt(out, "value", r.value.as_deref());
    write_opt(out, "unit", r.unit.as_deref());
    write_opt(out, "secondary", r.secondary.as_deref());
    write_opt(out, "secondary-unit", r.secondary_unit.as_deref());
    write_opt(out, "caption", r.caption.as_deref());
    write_opt(out, "trend", r.trend.map(Trend::as_str));
    // `state` and `role` are always written, even at their defaults. A default
    // that is implicit on the wire is a default two implementations can
    // disagree about; a default that is spelled out is one they cannot.
    out.push_str(&format!("state: {}\n", r.state.as_str()));
    out.push_str(&format!("role: {}\n", r.role.as_str()));
    if let Some(age) = r.age_seconds {
        out.push_str(&format!("age: {age}\n"));
    }
}

// ---------------------------------------------------------------------------
// The rules both halves run
// ---------------------------------------------------------------------------

/// Every value carried by a card, checked once.
///
/// Called by [`Message::encode`] before it writes and by [`Message::decode`]
/// after it reads, so the two cannot disagree about what a valid card is.
fn validate(card: &Card) -> Result<(), Fault> {
    if card.blocks.is_empty() {
        return Err(Fault::Empty { what: "card" });
    }
    if card.blocks.len() > LIMITS.blocks {
        return Err(Fault::TooLong {
            what: "card's blocks",
            limit: LIMITS.blocks,
        });
    }
    for block in &card.blocks {
        match block {
            Block::Reading(r) => validate_reading(r)?,
            Block::Statement(s) => {
                check_value("text", &s.text)?;
            }
            Block::List(l) => validate_list(l)?,
            // **Both sides, by the same rules, in the same order.** A pair whose
            // right column were held to something weaker would be a second
            // definition of a reading, and the shape exists precisely so that
            // the two are comparable.
            Block::Pair(p) => {
                validate_reading(&p.left)?;
                validate_reading(&p.right)?;
            }
        }
    }
    Ok(())
}

fn validate_reading(r: &Reading) -> Result<(), Fault> {
    for (key, value) in [
        ("value", &r.value),
        ("unit", &r.unit),
        ("secondary", &r.secondary),
        ("secondary-unit", &r.secondary_unit),
        ("caption", &r.caption),
    ] {
        if let Some(v) = value {
            check_value(key, v)?;
        }
    }
    // The rule `cpp/src/message.hpp` states from the producer's side —
    // *"value is omitted for error and connecting, so a renderer that ignored
    // the status still could not put a stale number on the wall."* Stated here
    // as well, so a producer that sends both is refused rather than trusted.
    if r.value.is_some() && !r.state.admits_a_value() {
        return Err(Fault::Contradiction {
            why: "a reading in `connecting` or `unavailable` has no value to show, \
                  and sending one anyway is how a number nobody vouched for reaches a wall",
        });
    }
    // A reading that would draw nothing at all. The original renderer's reading
    // card returns early in exactly this case; refusing it here means the producer hears about it
    // instead of watching an empty corner.
    if r.value.is_none() && r.state == State::Ok {
        return Err(Fault::Contradiction {
            why: "a reading with no value and nothing wrong has nothing to draw; \
                  say `state: connecting` or withdraw the card",
        });
    }
    Ok(())
}

fn validate_list(l: &List) -> Result<(), Fault> {
    if l.rows.is_empty() {
        return Err(Fault::Empty { what: "list" });
    }
    if l.rows.len() > LIMITS.rows {
        return Err(Fault::TooLong {
            what: "list's rows",
            limit: LIMITS.rows,
        });
    }
    if let Some(c) = &l.caption {
        check_value("caption", c)?;
    }
    for row in &l.rows {
        check_value("label", &row.label)?;
        if let Some(v) = &row.lead {
            check_value("lead", v)?;
        }
        if let Some(v) = &row.trail {
            check_value("trail", v)?;
        }
    }
    Ok(())
}

/// One value, against the three things a value may not be.
fn check_value(key: &'static str, value: &str) -> Result<(), Fault> {
    if value.is_empty() {
        return Err(Fault::BadValue {
            key,
            value: String::new(),
        });
    }
    if value.len() > LIMITS.value_bytes {
        return Err(Fault::TooLong {
            what: "value",
            limit: LIMITS.value_bytes,
        });
    }
    if value.chars().any(is_control) {
        return Err(Fault::NotText);
    }
    Ok(())
}

/// A character no value may carry.
///
/// `\n` is the framing, so a value holding one would silently become two lines
/// and a key the reader has never heard of. Every other C0 control and `0x7f`
/// go with it: they cannot be drawn, they cannot be typed into a bug report,
/// and a terminal reading a journal line does unpredictable things with them.
fn is_control(c: char) -> bool {
    c < '\u{20}' || c == '\u{7f}'
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// Which set of keys is currently in scope.
enum Scope {
    Card,
    Reading(Reading),
    Statement(Option<String>, Role),
    List(List),
    Row(List, Row),
    /// A `pair` block whose sides have not been opened yet.
    Pair(PairUnderConstruction),
    /// A `pair` with one of its two sides open. The `bool` is *this is the
    /// right one*, rather than a `Side` enum, because it is read in exactly one
    /// place and a two-variant enum for it would be a type nobody else uses.
    PairSide(PairUnderConstruction, Reading, bool),
}

/// A `pair` being read, remembering which sides have been given.
///
/// **Not a `Pair`**, because a `Pair` has two readings and a half-read one has
/// none, one, or two — and defaulting the missing side would be inventing a
/// column. `SPEC-04` §5.8: both are required, and a side with nothing to report
/// says so with its `state`.
#[derive(Default)]
struct PairUnderConstruction {
    left: Option<Reading>,
    right: Option<Reading>,
}

impl PairUnderConstruction {
    /// Close one side, refusing a second `left` or a second `right`.
    ///
    /// The same rule as [`once`] for keys, one level up: two answers to *which
    /// reading is on the left* is not a pair.
    fn put(&mut self, reading: Reading, is_right: bool) -> Result<(), Fault> {
        let (slot, word) = if is_right {
            (&mut self.right, "right")
        } else {
            (&mut self.left, "left")
        };
        if slot.is_some() {
            return Err(Fault::RepeatedKey {
                key: word.to_string(),
            });
        }
        *slot = Some(reading);
        Ok(())
    }

    fn finish(self) -> Result<Pair, Fault> {
        Ok(Pair {
            left: self.left.ok_or(Fault::MissingKey { key: "left" })?,
            right: self.right.ok_or(Fault::MissingKey { key: "right" })?,
        })
    }
}

impl Message {
    /// Read one whole message — the `card`/`withdraw` line through the `end`.
    ///
    /// [`Framer`] is what finds the boundaries in a byte stream; this is what
    /// happens once one has been found, and it is separate so that every branch
    /// is testable without a socket.
    ///
    /// # Errors
    ///
    /// [`Fault`]. See [`Fault::is_fatal`] for what a caller does about it.
    pub fn decode(text: &str) -> Result<Self, Fault> {
        if text.len() > LIMITS.message_bytes {
            return Err(Fault::TooLong {
                what: "message",
                limit: LIMITS.message_bytes,
            });
        }
        let lines: Vec<&str> = text
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
            .filter(|l| !l.is_empty())
            .collect();
        if lines.len() > LIMITS.lines {
            return Err(Fault::TooLong {
                what: "message's lines",
                limit: LIMITS.lines,
            });
        }
        let Some((&first, rest)) = lines.split_first() else {
            return Err(Fault::Unterminated);
        };
        if rest.last() != Some(&"end") {
            return Err(Fault::Unterminated);
        }
        let body = &rest[..rest.len() - 1];

        let (verb, vocabulary) = first.split_once(' ').unwrap_or((first, ""));
        let Ok(theirs) = vocabulary.parse::<u32>() else {
            return Err(Fault::UnknownVerb {
                verb: first.to_string(),
            });
        };
        if theirs > VOCABULARY {
            return Err(Fault::UnknownVocabulary {
                theirs,
                ours: VOCABULARY,
            });
        }
        match verb {
            "withdraw" => decode_withdraw(body),
            "card" => decode_card(body),
            _ => Err(Fault::UnknownVerb {
                verb: verb.to_string(),
            }),
        }
    }
}

/// One line, split into a key and a value, with the format's one separator.
fn split_pair(line: &str) -> Result<(&str, &str), Fault> {
    // `": "` and not `':'`: the value begins immediately after one space, so
    // there is no leading-whitespace question and no trimming for two
    // implementations to do differently.
    let Some((key, value)) = line.split_once(": ") else {
        return Err(Fault::NotALine {
            line: truncated(line),
        });
    };
    if key.is_empty()
        || !key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(Fault::NotALine {
            line: truncated(line),
        });
    }
    if value.is_empty() {
        return Err(Fault::NotALine {
            line: truncated(line),
        });
    }
    if value.len() > LIMITS.value_bytes {
        return Err(Fault::TooLong {
            what: "value",
            limit: LIMITS.value_bytes,
        });
    }
    if value.chars().any(is_control) {
        return Err(Fault::NotText);
    }
    Ok((key, value))
}

fn truncated(line: &str) -> String {
    line.chars().take(60).collect()
}

/// Record a key as seen, or say it repeated.
fn once(seen: &mut Vec<String>, key: &str) -> Result<(), Fault> {
    if seen.iter().any(|k| k == key) {
        return Err(Fault::RepeatedKey {
            key: key.to_string(),
        });
    }
    seen.push(key.to_string());
    Ok(())
}

fn decode_withdraw(body: &[&str]) -> Result<Message, Fault> {
    let mut slot = None;
    let mut seen = Vec::new();
    for line in body {
        let (key, value) = split_pair(line)?;
        once(&mut seen, key)?;
        match key {
            "slot" => {
                slot = Some(Slot::parse(value).ok_or_else(|| Fault::BadValue {
                    key: "slot",
                    value: value.to_string(),
                })?);
            }
            _ => {
                return Err(Fault::UnknownKey {
                    key: key.to_string(),
                });
            }
        }
    }
    Ok(Message::Withdraw {
        slot: slot.ok_or(Fault::MissingKey { key: "slot" })?,
    })
}

fn decode_card(body: &[&str]) -> Result<Message, Fault> {
    let mut slot: Option<Slot> = None;
    let mut ttl: Option<u32> = None;
    let mut blocks: Vec<Block> = Vec::new();
    let mut scope = Scope::Card;
    let mut seen: Vec<String> = Vec::new();

    for line in body {
        // The two bare words first: they change scope, and everything else is
        // a key inside whatever scope is current.
        if *line == "row" {
            scope = match scope {
                Scope::List(list) => {
                    seen.clear();
                    Scope::Row(list, Row::default())
                }
                Scope::Row(mut list, row) => {
                    push_row(&mut list, row)?;
                    seen.clear();
                    Scope::Row(list, Row::default())
                }
                _ => return Err(Fault::Misplaced { word: "row" }),
            };
            continue;
        }

        // `left` and `right` are `row`'s twins — `SPEC-04` §5.8.1. They open a
        // reading inside a `pair` and are misplaced anywhere else, including
        // inside a `list`, where the mistake would otherwise be silent.
        if *line == "left" || *line == "right" {
            let is_right = *line == "right";
            scope = match scope {
                Scope::Pair(pair) => {
                    seen.clear();
                    Scope::PairSide(pair, Reading::default(), is_right)
                }
                Scope::PairSide(mut pair, reading, was_right) => {
                    pair.put(reading, was_right)?;
                    seen.clear();
                    Scope::PairSide(pair, Reading::default(), is_right)
                }
                _ => {
                    return Err(Fault::Misplaced {
                        word: if is_right { "right" } else { "left" },
                    });
                }
            };
            continue;
        }

        let (key, value) = split_pair(line)?;

        if key == "block" {
            let shape = Shape::parse(value).ok_or_else(|| Fault::UnknownShape {
                shape: value.to_string(),
            })?;
            close_scope(scope, &mut blocks)?;
            seen.clear();
            scope = match shape {
                Shape::Reading => Scope::Reading(Reading::default()),
                Shape::Statement => Scope::Statement(None, Role::default()),
                Shape::List => Scope::List(List::default()),
                Shape::Pair => Scope::Pair(PairUnderConstruction::default()),
            };
            if blocks.len() >= LIMITS.blocks {
                return Err(Fault::TooLong {
                    what: "card's blocks",
                    limit: LIMITS.blocks,
                });
            }
            continue;
        }

        once(&mut seen, key)?;
        match &mut scope {
            Scope::Card => match key {
                "slot" => {
                    slot = Some(Slot::parse(value).ok_or_else(|| Fault::BadValue {
                        key: "slot",
                        value: value.to_string(),
                    })?);
                }
                "ttl" => {
                    ttl = Some(value.parse::<u32>().map_err(|_| Fault::BadValue {
                        key: "ttl",
                        value: value.to_string(),
                    })?);
                }
                _ => return Err(scope_fault(key)),
            },
            Scope::Reading(r) => read_reading_key(r, key, value)?,
            Scope::Statement(text, role) => match key {
                "text" => *text = Some(value.to_string()),
                "role" => *role = parse_role(value)?,
                _ => return Err(scope_fault(key)),
            },
            Scope::List(l) => match key {
                "caption" => l.caption = Some(value.to_string()),
                "glyph" => l.glyph = Some(parse_glyph(value)?),
                _ => return Err(scope_fault(key)),
            },
            Scope::Row(_, row) => match key {
                "lead" => row.lead = Some(value.to_string()),
                "label" => row.label = value.to_string(),
                "trail" => row.trail = Some(value.to_string()),
                "role" => row.role = parse_role(value)?,
                _ => return Err(scope_fault(key)),
            },
            // A key straight after `block: pair` and before `left` or `right`.
            // A pair has no keys of its own — no widths, no alignment, no rule
            // colour — so there is nothing this could be.
            Scope::Pair(_) => return Err(scope_fault(key)),
            Scope::PairSide(_, reading, _) => read_reading_key(reading, key, value)?,
        }
    }

    close_scope(scope, &mut blocks)?;

    let card = Card {
        slot: slot.ok_or(Fault::MissingKey { key: "slot" })?,
        ttl_seconds: ttl.ok_or(Fault::MissingKey { key: "ttl" })?,
        blocks,
    };
    // The same function the encoder ran. A card that arrived is held to exactly
    // the rules a card that leaves is held to.
    validate(&card)?;
    Ok(Message::Card(card))
}

/// A key that exists in the vocabulary but not here, told apart from one that
/// does not exist at all — because they send an author to different places.
fn scope_fault(key: &str) -> Fault {
    const EVERY_KEY: [&str; 15] = [
        "slot",
        "ttl",
        "block",
        "glyph",
        "value",
        "unit",
        "secondary",
        "secondary-unit",
        "caption",
        "trend",
        "state",
        "role",
        "age",
        "text",
        "label",
    ];
    if EVERY_KEY.contains(&key) || key == "lead" || key == "trail" {
        Fault::KeyOutOfScope {
            key: key.to_string(),
        }
    } else {
        Fault::UnknownKey {
            key: key.to_string(),
        }
    }
}

fn read_reading_key(r: &mut Reading, key: &str, value: &str) -> Result<(), Fault> {
    match key {
        "glyph" => r.glyph = Some(parse_glyph(value)?),
        "value" => r.value = Some(value.to_string()),
        "unit" => r.unit = Some(value.to_string()),
        "secondary" => r.secondary = Some(value.to_string()),
        "secondary-unit" => r.secondary_unit = Some(value.to_string()),
        "caption" => r.caption = Some(value.to_string()),
        "trend" => {
            r.trend = Some(Trend::parse(value).ok_or_else(|| Fault::BadValue {
                key: "trend",
                value: value.to_string(),
            })?);
        }
        "state" => {
            r.state = State::parse(value).ok_or_else(|| Fault::BadValue {
                key: "state",
                value: value.to_string(),
            })?;
        }
        "role" => r.role = parse_role(value)?,
        "age" => {
            r.age_seconds = Some(value.parse::<u32>().map_err(|_| Fault::BadValue {
                key: "age",
                value: value.to_string(),
            })?);
        }
        _ => return Err(scope_fault(key)),
    }
    Ok(())
}

fn parse_role(value: &str) -> Result<Role, Fault> {
    Role::parse(value).ok_or_else(|| Fault::BadValue {
        key: "role",
        value: value.to_string(),
    })
}

fn parse_glyph(value: &str) -> Result<Glyph, Fault> {
    Glyph::parse(value).ok_or_else(|| Fault::BadValue {
        key: "glyph",
        value: value.to_string(),
    })
}

fn push_row(list: &mut List, row: Row) -> Result<(), Fault> {
    if row.label.is_empty() {
        return Err(Fault::MissingKey { key: "label" });
    }
    if list.rows.len() >= LIMITS.rows {
        return Err(Fault::TooLong {
            what: "list's rows",
            limit: LIMITS.rows,
        });
    }
    list.rows.push(row);
    Ok(())
}

fn close_scope(scope: Scope, blocks: &mut Vec<Block>) -> Result<(), Fault> {
    match scope {
        Scope::Card => {}
        Scope::Reading(r) => blocks.push(Block::Reading(r)),
        Scope::Statement(text, role) => blocks.push(Block::Statement(Statement {
            text: text.ok_or(Fault::MissingKey { key: "text" })?,
            role,
        })),
        Scope::List(list) => blocks.push(Block::List(list)),
        Scope::Row(mut list, row) => {
            push_row(&mut list, row)?;
            blocks.push(Block::List(list));
        }
        Scope::Pair(pair) => blocks.push(Block::Pair(pair.finish()?)),
        Scope::PairSide(mut pair, reading, is_right) => {
            pair.put(reading, is_right)?;
            blocks.push(Block::Pair(pair.finish()?));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Framing a byte stream
// ---------------------------------------------------------------------------

/// **Finds message boundaries in a stream, and holds the line.**
///
/// A socket delivers whatever it delivers: half a card, three cards, a card and
/// half of the next. This is the piece that turns that into messages, and it is
/// a type rather than a loop in the renderer so that the bound on an unfinished
/// message is enforced in one place.
///
/// It is deliberately I/O-free, so every one of its failure modes is testable
/// without a socket — `AGENTS.md` §4's rule that the core must test with no I/O,
/// applied to the one part of this crate that has a stream in mind.
/// # The buffer holds BYTES, and that is load-bearing
///
/// A socket splits wherever it splits, including **in the middle of a
/// character**. `°` is two bytes and Aura puts one in every temperature it has
/// ever drawn, so a framer that validated UTF-8 on each `push` would reject a
/// perfectly good card because the kernel returned the first byte of a degree
/// sign and not the second. Found by
/// `tests::a_message_split_across_reads_is_reassembled`, which splits the same
/// card at every single byte offset — the first version of this type failed at
/// the two offsets inside that character.
///
/// So the buffer is `Vec<u8>`, boundaries are found by scanning for `\n` (which
/// cannot occur inside a multi-byte sequence), and the text is validated once,
/// when a whole message is taken out.
#[derive(Debug, Default)]
pub struct Framer {
    buffer: Vec<u8>,
    /// Set by a fatal fault. Once the framing has failed there is no position
    /// to resume from, so every later call says the same thing rather than
    /// pretending the stream recovered.
    poisoned: Option<Fault>,
}

impl Framer {
    /// A new framer with an empty buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add bytes read from the socket.
    ///
    /// # Errors
    ///
    /// Only the fault a previous call already poisoned this framer with. The
    /// bytes themselves are not inspected here — see the note on the type for
    /// why validating them at this point would be wrong.
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), Fault> {
        if let Some(fault) = &self.poisoned {
            return Err(fault.clone());
        }
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    /// The next complete message, if one has arrived.
    ///
    /// `None` means **not yet** — keep reading. A content fault is returned and
    /// the framer carries on at the next message; a fatal one poisons it. See
    /// [`Fault::is_fatal`].
    pub fn next_message(&mut self) -> Option<Result<Message, Fault>> {
        if let Some(fault) = &self.poisoned {
            return Some(Err(fault.clone()));
        }
        // A message ends at the first line that is exactly `end`. `\n` is 0x0a
        // and no byte of a multi-byte UTF-8 sequence is below 0x80, so this
        // scan is safe on bytes that have not been validated yet.
        let mut at = 0usize;
        let end_at = loop {
            let Some(offset) = self.buffer[at..].iter().position(|b| *b == b'\n') else {
                break None;
            };
            let line = &self.buffer[at..at + offset];
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line == b"end" {
                break Some(at + offset + 1);
            }
            at += offset + 1;
        };
        let Some(end_at) = end_at else {
            // Nothing complete yet. The bound is on the *unfinished* message,
            // which is the case a hostile or broken producer reaches by never
            // sending an `end` — and the one a growing buffer would answer with
            // an out-of-memory kill instead of a sentence.
            if self.buffer.len() > LIMITS.message_bytes {
                return Some(Err(self.poison(Fault::Overran {
                    limit: LIMITS.message_bytes,
                })));
            }
            return None;
        };
        let message: Vec<u8> = self.buffer.drain(..end_at).collect();
        let Ok(text) = String::from_utf8(message) else {
            // A whole message that is not text. Fatal: this is not a split
            // character, it is a stream that is not the thing it claims to be.
            return Some(Err(self.poison(Fault::NotText)));
        };
        Some(Message::decode(&text))
    }

    fn poison(&mut self, fault: Fault) -> Fault {
        self.poisoned = Some(fault.clone());
        fault
    }
}
