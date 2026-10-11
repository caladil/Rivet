//! **The controller, and everything that arrives from it.**
//!
//! `Aura/docs/ipc-schema.md` is the normative statement of what arrives here.
//! This is a DIRECT PORT of what `cpp/src/main.cpp` did: this process connects
//! to `aura-controller`'s socket and handles both message types the controller
//! sends — `state`, which becomes cards, and `cmd`, which is a directive acted
//! on at once.
//!
//! # Why this is in the renderer and not in a program of its own
//!
//! It was in a program of its own — `aura-bridge` — and that was the mistake.
//! A translator existed only because the renderer had stopped speaking Aura's
//! protocol, and `cmd` fell down the gap between them: the bridge skipped
//! directives as *"the controller talking to a renderer this program is not"*,
//! and the renderer had no code for them. Brightness and the screensaver theme
//! were compile-time constants, so the portal saved settings that reached
//! nothing. Ruled by the maintainer 2026-09-02: **two programs, and this one
//! speaks the controller's language.**
//!
//! The four slot sockets in `crate::ingest` are unaffected and stay: an addon
//! in somebody else's repository still publishes a card the same way.
//!
//! # Every field is optional, and absent is not zero
//!
//! The schema says so and gives the reason: *"a station that is not reporting
//! sends no `temp_f` at all, because 0°F is an ordinary January reading in
//! Minnesota and a bare number cannot distinguish the two."* So every reading
//! below is an `Option` and none of them defaults.
//!
//! # Unknown fields are IGNORED here and REFUSED one door along
//!
//! That looks inconsistent and is not. The schema this reads is somebody
//! else's, is unversioned, and states its own rule — *"unknown types, and
//! unknown fields within a known type, are ignored by the renderer"* — so a
//! reader that refused would break the day the controller grew a field. The
//! card this writes goes to a renderer whose rule is the opposite (`SPEC-04`
//! §6.5), because a card is a contract with a version number on every message.
//!
//! **The refusal that matters happens between them**: anything this module
//! cannot map is dropped from the card rather than guessed at, and the card is
//! then held to `aura-card`'s rules in full.

use aura_card::{Block, Card, Glyph, Pair, Reading, Role, Slot, State, Statement, Trend};
use serde::Deserialize;

use crate::widgets::Corner;

/// One `type: "state"` message from `cmd/aura-controller`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct StateMessage {
    #[serde(rename = "type")]
    pub kind: String,
    pub weather: Option<Weather>,
    pub indoor: Option<Indoor>,
    /// **The corner reading card, read here rather than by a producer of its
    /// own**, and the reason is the platform's rather than a preference:
    /// `aura-controller:kiosk.sock` has ONE consumer, because the grant is a
    /// chown and chown names one uid (`ADR-015` §5.3). An addon asking for the
    /// same endpoint would have both it and this renderer refused by name at
    /// launch — which is the rule working, not a defect. So the one program
    /// that is granted the controller fans its facts out across the slots it
    /// was granted. See `reading_card_for`.
    pub reading: Option<ReadingData>,
    /// **The clock, formatted by the controller** — `wire.rs`'s `Clock`, whose
    /// own words are *"the controller formats; the renderer prints."*
    ///
    /// **This field did not exist until 2026-09-07 and that was the whole
    /// defect.** The controller has always sent this block, on every state
    /// message; `serde` deserialised it into nothing and the screen drew
    /// `crate::clock`'s constant instead. So the portal's 24-hour toggle
    /// reformatted a string that reached no pixel, and the maintainer set it,
    /// saved it, and watched a 12-hour clock.
    pub clock: Option<Clock>,
}

#[cfg(test)]
mod clock_tests {
    #![allow(clippy::expect_used)]
    use super::*;

    fn state(json: &str) -> StateMessage {
        serde_json::from_str(json).expect("the controller's own shape")
    }

    /// **The defect this was written for**, stated as an assertion: the
    /// controller sends `24:00`-shaped text and the renderer must print THAT,
    /// not reformat it and not ignore it.
    #[test]
    fn the_controllers_clock_is_printed_verbatim() {
        let card = clock_card_for(&state(
            r#"{"type":"state","clock":{"time":"18:19","date":"Monday, September 7"}}"#,
        ))
        .expect("a clock card");
        assert_eq!(card.slot, Slot::Primary);
        let Block::Reading(reading) = &card.blocks[0] else {
            panic!("the clock is a reading");
        };
        assert_eq!(reading.value.as_deref(), Some("18:19"));
        assert_eq!(reading.caption.as_deref(), Some("Monday, September 7"));
    }

    /// `Show date` off is an EMPTY string on the wire, not an absent key.
    #[test]
    fn an_empty_date_draws_no_caption() {
        let card = clock_card_for(&state(
            r#"{"type":"state","clock":{"time":"6:19","date":""}}"#,
        ))
        .expect("a clock card");
        let Block::Reading(reading) = &card.blocks[0] else {
            panic!("the clock is a reading");
        };
        assert_eq!(reading.value.as_deref(), Some("6:19"));
        assert_eq!(reading.caption, None);
    }

    /// **A controller mid-start must not blank a good clock.** No block and an
    /// empty time are both "nothing to say", not "show nothing".
    #[test]
    fn nothing_to_say_is_not_a_blank_clock() {
        assert!(clock_card_for(&state(r#"{"type":"state"}"#)).is_none());
        assert!(clock_card_for(&state(r#"{"type":"state","clock":{"time":"","date":""}}"#)).is_none());
    }
}

/// `HH:MM` and a long date, both already formatted.
///
/// **Nothing here is parsed back into a time.** The controller owns the format
/// — 12- or 24-hour, date shown or not — and an empty `date` is how it says the
/// date is switched off, which is a caption this end simply does not draw.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Clock {
    #[serde(default)]
    pub time: String,
    #[serde(default)]
    pub date: String,
}

/// `WeatherData` — `cpp/src/message.hpp`, and the `weather` object in the JSON.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Weather {
    pub temp_f: Option<f64>,
    /// A wind chill or a dewpoint; `secondary_kind` says which, and neither the
    /// C++ nor this draws the distinction.
    pub secondary_f: Option<f64>,
    /// `"F"` or `"C"`. **Absent means Fahrenheit**, which the schema states.
    pub unit: Option<String>,
    /// The NWS icon slug. Collapsed to a [`Glyph`] here, which `SPEC-04` §5.6
    /// says is a producer's job and always was.
    pub icon: Option<String>,
    /// The NWS `shortForecast`. The only free text in Aura's whole schema.
    pub statement: Option<String>,
}

/// `IndoorData`. **`online` is the field the whole indoor column turns on**, and
/// `cpp/src/message.hpp` says why: *a dead sensor and a perfectly stable room
/// produce the same flat line, and nothing but this flag tells them apart.*
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Indoor {
    pub temp_f: Option<f64>,
    pub humidity_pct: Option<f64>,
    pub unit: Option<String>,
    #[serde(default)]
    pub online: bool,
}

/// How long a card of this shape stays true, in seconds.
///
/// The forecast refreshes every twenty minutes and the observation every twenty;
/// five is short enough that a bridge which dies takes its card off the wall
/// while a person is still standing in front of it, and long enough that a
/// controller restart does not blank the panel. `SPEC-04` §3.4.
pub const TTL_SECONDS: u32 = 300;

/// **The corner reading card** — the `reading` object in a `state` message,
/// and the controller's `wire::ReadingData`, field for field.
///
/// `ReadingData` and not `Reading`, after the original renderer's own
/// `WeatherData` and `IndoorData`, because [`aura_card::Reading`] is the shape
/// this becomes and the two are both in hand in this file.
///
/// **Nothing here knows what the value measures.** The controller does not
/// know either (`.rulings/` R-375 D1): the source converts, the controller
/// relays, and this prints.
#[derive(Debug, Clone, Deserialize)]
pub struct ReadingData {
    /// Already in `unit`. Absent unless `status` is `ok` or `stale`, which is
    /// what stops a renderer that ignored `status` putting a stale number up.
    pub value: Option<f64>,
    pub unit: Option<String>,
    pub trend: Option<String>,
    /// The source's verdict against its own band. Not recomputed here: the
    /// band is the source's configuration and the source owns it (`ADR-011`).
    #[serde(default)]
    pub out_of_range: bool,
    pub status: Option<String>,
    pub age_seconds: Option<u32>,
}

/// **Fifteen minutes.** `SPEC-04` §3.4 offers 900 for exactly this card: a
/// source that reports every five minutes has missed three reports by then,
/// which is the point at which the number stops being worth the wall.
pub const READING_TTL_SECONDS: u32 = 900;

/// `unknown` is a value the schema names, and it means the producer has no
/// arrow rather than that the arrow is flat. Those are different.
fn trend_for(slug: Option<&str>) -> Option<Trend> {
    match slug? {
        "falling_fast" => Some(Trend::FallingFast),
        "falling" => Some(Trend::Falling),
        "flat" => Some(Trend::Flat),
        "rising" => Some(Trend::Rising),
        "rising_fast" => Some(Trend::RisingFast),
        _ => None,
    }
}

/// `status` decides what the card draws, because how old a reading may be
/// before it stops being worth showing is a rule about DATA, and the source
/// owns the data.
fn reading_state(status: Option<&str>) -> State {
    match status {
        Some("ok") => State::Ok,
        Some("stale") => State::Stale,
        Some("connecting") => State::Connecting,
        // `error`, and anything this build has not been taught.
        _ => State::Unavailable,
    }
}

/// **The value as the source rounded it, to at most one decimal** — `42`
/// stays `42`, `2.4` stays `2.4`, and `21.3333` becomes `21.3`.
///
/// # No unit decides it, and nothing guesses from magnitude
///
/// The host knows no unit for this card (`.rulings/` R-375 D1), so the rule
/// cannot be keyed on one; and a rule keyed on how big the number is would be
/// a guess about what it measures. So the source's own rounding is what is
/// printed: a source that sends whole numbers draws whole numbers, and one
/// that rounds to a tenth draws tenths. One decimal is the ceiling, so a
/// source that does not round at all still fits the card.
///
/// **What this cannot do is tell `6.0` from `6`**: the protocol carries a
/// number and not a precision, so a tenths source whose value lands on a whole
/// number draws without the `.0` — `2.0` is drawn `2`.
fn reading_value(value: f64) -> String {
    let tenths = format!("{value:.1}");
    match tenths.strip_suffix(".0") {
        Some("-0") => "0".to_string(),
        Some(whole) => whole.to_string(),
        None => tenths,
    }
}

/// **The same card with `age_seconds` cleared, for CHANGE DETECTION ONLY.**
///
/// `age_seconds` advances every second, so a card compared with it in place is
/// never equal to the last one and the producer sends thirty times more often
/// than it has anything to say. The renderer excludes it from its own redraw
/// comparison for exactly this reason, and says so beside the code that does —
/// *"the same reason `age_seconds` is not part of the C++'s redraw
/// comparison"*. The card that is SENT still carries the true age.
pub fn ageless(card: &Card) -> Card {
    let mut card = card.clone();
    for block in &mut card.blocks {
        if let Block::Reading(reading) = block {
            reading.age_seconds = None;
        }
    }
    card
}

/// **The corner reading card**, or `None` when the card is switched off.
///
/// # An enabled card with nothing to show still draws
///
/// **The unconfigured case is the ABSENT BLOCK**, which `?` below handles —
/// the controller sends no `reading` object at all when the card is off. The
/// original controller decided it, for a reason that is the whole answer: a
/// card that simply vanishes when a sensor drops out is indistinguishable from
/// one nobody enabled. So everything else — including having nothing to show
/// — is a state with a drawing.
///
/// A block that is present with `status: "error"` means the opposite of
/// absent: the card is enabled, and the reading has dropped out.
/// `Aura/docs/ipc-schema.md`'s table is normative and says what each status
/// draws — `error` is *"the connection alert alone"*, `connecting` is *"the
/// connection question alone"*. Returning `None` for either would take the
/// card off the wall at exactly the moment it has something to say: a source
/// that goes quiet would age its reading past the point of showing, report
/// `error`, and vanish instead of showing the alert, coming back only when a
/// fresh reading arrived.
///
/// The renderer already knows how to draw this: `layout`'s
/// `a_reading_that_cannot_be_believed_draws_its_mark_instead_of_a_value` proves
/// a `Reading` with no value and a `Connecting` or `Unavailable` state draws
/// its mark.
pub fn reading_card_for(message: &StateMessage) -> Option<Card> {
    // **The only `None`.** An absent block is the card switched off; every
    // present one is a state with a drawing, including the ones with no value.
    let r = message.reading.as_ref()?;
    let state = reading_state(r.status.as_deref());
    let value = r.value.map(reading_value);

    Some(Card {
        slot: Slot::Tertiary,
        ttl_seconds: READING_TTL_SECONDS,
        blocks: vec![Block::Reading(Reading {
            value: value.clone(),
            unit: value.is_some().then(|| r.unit.clone()).flatten(),
            trend: trend_for(r.trend.as_deref()),
            state,
            // **Out of range is an alarm, in range is normal.** The colour is
            // the theme's; the MEANING is the source's, and this is the one
            // place it crosses (`SPEC-04` §2 — a card carries a meaning, never
            // a colour).
            role: if r.out_of_range {
                Role::Alarm
            } else {
                Role::Normal
            },
            age_seconds: r.age_seconds,
            ..Reading::default()
        })],
    })
}

/// `DrawWeatherIcon`'s branch table, which is where the closed glyph set came
/// from — `SPEC-04` §5.6.
///
/// **The order is the C++'s `if`/`else if` order and must stay that way.** NWS
/// slugs compose (`rain_showers`, `tsra_sct`), so a slug can match two of these
/// and the first one wins in both implementations or in neither.
///
/// A slug that matches nothing yields `None`. The C++ has an `else` that draws
/// a dotted circle for an unrecognised slug; sending a glyph this vocabulary
/// does not have would be inventing one, and omitting the mark is the failure
/// `SPEC-04` §7 rule 5 chooses.
#[must_use]
pub fn glyph_for(slug: &str) -> Option<Glyph> {
    let has = |needle: &str| slug.contains(needle);
    if has("skc") || has("hot") {
        Some(Glyph::Clear)
    } else if has("few") || has("sct") {
        Some(Glyph::PartlyCloudy)
    } else if has("bkn") || has("ovc") {
        Some(Glyph::Cloudy)
    } else if has("rain") || has("shower") || has("drizzle") {
        Some(Glyph::Rain)
    } else if has("tsra") || has("thunder") {
        Some(Glyph::Thunder)
    } else if has("snow") || has("blizzard") || has("flurr") {
        Some(Glyph::Snow)
    } else if has("sleet") || has("fzra") || has("ice") {
        Some(Glyph::Sleet)
    } else if has("fog") || has("haze") || has("smoke") || has("dust") {
        Some(Glyph::Fog)
    } else if has("wind") {
        Some(Glyph::Wind)
    } else if has("unavailable") {
        Some(Glyph::Unavailable)
    } else {
        None
    }
}

/// `"%.0f"` — the C++'s own format for every temperature on this panel.
fn whole(v: f64) -> String {
    format!("{v:.0}")
}

/// The degree form of a unit: `"F"` becomes `"°F"`, which is what
/// `"%.0f\xC2\xB0%s"` produced. Absent means Fahrenheit, per the schema.
fn degrees(unit: Option<&str>) -> String {
    format!("°{}", unit.unwrap_or("F"))
}

/// The outdoor column.
fn outdoor(weather: Option<&Weather>) -> Reading {
    let Some(w) = weather else {
        return unavailable();
    };
    let unit = degrees(w.unit.as_deref());
    let glyph = w.icon.as_deref().and_then(glyph_for);
    let value = w.temp_f.map(whole);
    let secondary = w.secondary_f.map(whole);
    if value.is_none() && secondary.is_none() && glyph.is_none() {
        return unavailable();
    }
    Reading {
        glyph,
        value: value.clone(),
        unit: value.is_some().then(|| unit.clone()),
        secondary: secondary.clone(),
        secondary_unit: secondary.is_some().then_some(unit),
        // `state` says how much of a reading can be believed. A column carrying
        // a glyph and no number is not a fault — it is the weather service
        // having said nothing, which `glyph: unavailable` already reports — so
        // this is `unavailable` only when the state admits no value.
        state: if value.is_some() {
            State::Ok
        } else {
            State::Unavailable
        },
        ..Reading::default()
    }
}

/// The indoor column.
///
/// **`online: false` clears it**, which is `cpp/src/main.cpp`'s
/// `if (msg->indoor && msg->indoor->online)` and its `else` that resets both
/// readings. The column then reports `unavailable` rather than vanishing from
/// the card, because `SPEC-04` §5.8.1 requires both sides and a `state` says
/// why where an absent key would not.
fn indoor(indoor: Option<&Indoor>) -> Reading {
    let Some(i) = indoor.filter(|i| i.online) else {
        return unavailable();
    };
    let value = i.temp_f.map(whole);
    let secondary = i.humidity_pct.map(whole);
    if value.is_none() && secondary.is_none() {
        return unavailable();
    }
    Reading {
        // The house. `DrawIndoorIcon` is chosen by a `bool indoor` flag in the
        // C++; here it is data like every other glyph, so a column with no room
        // reading carries no room glyph and reserves no width for one.
        glyph: Some(Glyph::Indoor),
        value: value.clone(),
        unit: value.is_some().then(|| degrees(i.unit.as_deref())),
        secondary: secondary.clone(),
        // A humidity is a percentage and a temperature is not, which is exactly
        // the case `secondary-unit` exists for (`SPEC-04` §5.2).
        secondary_unit: secondary.is_some().then(|| "%".to_string()),
        state: if value.is_some() {
            State::Ok
        } else {
            State::Unavailable
        },
        ..Reading::default()
    }
}

fn unavailable() -> Reading {
    Reading {
        state: State::Unavailable,
        ..Reading::default()
    }
}

/// **The card, or nothing.**
///
/// `None` when there is neither a reading nor a forecast to show. The caller
/// withdraws in that case rather than sending an empty card, which the
/// vocabulary refuses to express by design (`SPEC-04` §5.7).
#[must_use]
/// The clock card the controller sent, or `None` when it sent none.
///
/// **`Slot::Primary`, the same slot `crate::clock` writes**, and the two never
/// run at once: `Controller::clock_is_live` is what stands the local producer
/// down while this one is arriving.
///
/// An empty `time` is treated as no clock rather than as a blank one — a
/// controller mid-start has nothing to say yet, and drawing an empty card would
/// replace a good clock with a hole.
pub fn clock_card_for(message: &StateMessage) -> Option<Card> {
    let clock = message.clock.as_ref()?;
    if clock.time.trim().is_empty() {
        return None;
    }
    Some(Card {
        slot: Slot::Primary,
        // The same two minutes `crate::clock` uses, and for the same reason:
        // the controller replaces this every minute, so a controller that
        // stopped producing one shows an empty slot rather than a frozen time.
        ttl_seconds: 120,
        blocks: vec![Block::Reading(Reading {
            glyph: Some(Glyph::Clock),
            value: Some(clock.time.clone()),
            caption: if clock.date.trim().is_empty() {
                None
            } else {
                Some(clock.date.clone())
            },
            ..Reading::default()
        })],
    })
}

pub fn card_for(message: &StateMessage) -> Option<Card> {
    let left = outdoor(message.weather.as_ref());
    let right = indoor(message.indoor.as_ref());
    let statement = message
        .weather
        .as_ref()
        .and_then(|w| w.statement.as_deref())
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string);

    let nothing_to_show = left.state == State::Unavailable
        && right.state == State::Unavailable
        && statement.is_none();
    if nothing_to_show {
        return None;
    }

    let mut blocks = vec![Block::Pair(Pair { left, right })];
    if let Some(text) = statement {
        blocks.push(Block::Statement(Statement {
            text,
            // A forecast is an ordinary sentence, not a warning. Whether it is
            // drawn `muted` is the renderer's, and it draws every line of this
            // card in one ink because `DrawWeatherPanel` does.
            ..Statement::default()
        }));
    }
    Some(Card {
        slot: Slot::Secondary,
        ttl_seconds: TTL_SECONDS,
        blocks,
    })
}

/// `type: "cmd"` — `Aura/docs/ipc-schema.md` §Commands. *"Behavioural
/// directives; the renderer acts on receipt."*
#[derive(Debug, Clone, Deserialize)]
pub struct CmdMessage {
    pub cmd: String,
    #[serde(default)]
    pub value: Option<String>,
    /// `rotate_corner`'s corner. Parsed and not yet read, because
    /// `rotate_corner` is [`Command::Unhandled`] — kept so the wire this build
    /// accepts matches the wire the schema documents, rather than silently
    /// narrowing to the fields that happen to be used today.
    #[serde(default)]
    #[allow(dead_code)]
    pub corner: Option<String>,
    /// **`set_screensaver`'s theme, since T7.**
    ///
    /// The theme used to be folded into `value` — *Digital Rain* + *Ice Blue*
    /// arrived as the single word `construct_ice`, and both halves held a
    /// table of the ten words that could be spelled. That table is what stops
    /// a pack growing an eleventh background without an edit in two programs,
    /// so it is gone: `value` now carries the background id and this carries
    /// the theme id, exactly as `rivetos_addon` spells them.
    ///
    /// `None` means an older controller is talking, and
    /// [`selection_for`] unfolds the ten legacy words instead.
    #[serde(default)]
    pub theme: Option<String>,
}

/// One directive, already understood. `main` applies it; this module decides
/// nothing about what it means on screen.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// `"5"`–`"100"` percent.
    Brightness(u8),
    /// `set_screensaver`, as the pair of ids the pack speaks. **Owned
    /// strings**, because these come off the wire and this build no longer has
    /// a fixed set of names to intern them against — which is the whole point
    /// of T7: a pack installed tomorrow names backgrounds this binary has
    /// never heard of, and it must reach the pack unaltered.
    Screensaver { background: String, theme: String },
    /// `show_weather` — the whole panel, indoor column included.
    ShowWeather(bool),
    /// `rotate_corner`. `None` is the schema's *"omitted picks a random
    /// different corner"*, which is the rotation this renderer already runs.
    RotateCorner(Option<Corner>),
    /// `display_sleep` / `display_wake` — *"blanks or restores the overlay
    /// widgets"*. The SCREEN stays on and the background keeps drawing; this is
    /// not the display-hours switch-off, which is the machine's (`ADR-039`).
    ShowWidgets(bool),
    /// A directive this build understands the name of and does not act on yet.
    /// Carried rather than dropped so the journal can say so once.
    Unhandled(String),
}

/// `top-left`, `top-right`, `bottom-left`, `bottom-right` — the schema's
/// spelling, which is not this crate's variant names.
fn corner_for(value: &str) -> Option<Corner> {
    Some(match value {
        "top-left" => Corner::TopLeft,
        "top-right" => Corner::TopRight,
        "bottom-left" => Corner::BottomLeft,
        "bottom-right" => Corner::BottomRight,
        _ => return None,
    })
}

/// **The ten words an older controller sends, unfolded into a pair.**
///
/// `screensaverIPCValue` in `cmd/aura-controller/main.go` folded the portal's
/// screensaver AND its theme into one wire word — *Digital Rain* + *Ice Blue*
/// became `construct_ice`. T7 retired that: a controller of this vintage sends
/// the two ids the pack itself uses, and this table is only reached when
/// `theme` is absent from the message.
///
/// # Why it is kept at all
///
/// The renderer and the controller are two separately deployed artifacts. They
/// are meant to be deployed together and the board rebooted, which leaves no
/// window at all — but *meant to be* is not a guarantee, and the failure this
/// costs twelve lines to avoid is a wall showing a black screen because one
/// half of a pair landed. It is a compatibility shim with a job that ends, not
/// a vocabulary: **nothing may be added to it.** A background the pack grows
/// has no legacy word, by definition.
fn legacy_selection(value: &str) -> Option<(&'static str, &'static str)> {
    Some(match value {
        "solid" => ("solid", "black"),
        "solid_white" => ("solid", "white"),
        "solid_navy" => ("solid", "navy"),
        "construct_green" => ("digital-rain", "phosphor-green"),
        "construct_ice" => ("digital-rain", "ice-blue"),
        "born_this_way" => ("digital-rain", "born-this-way"),
        // The bare family name was its first palette, which is the order
        // `theme::BACKGROUND_PALETTES` lists and the default the Go switch
        // fell through to.
        "boxes" => ("boxes", "ice-blue"),
        "boxes_green" => ("boxes", "chartreuse"),
        "boxes_dark" => ("boxes", "dark"),
        "boxes_pride" => ("boxes", "born-this-way"),
        _ => return None,
    })
}

/// **What `set_screensaver` selects** — the pair from the message, or the
/// unfolded legacy word.
///
/// An unrecognised `value` with no `theme` is `None` and the caller refuses
/// the directive by name. An unrecognised `value` WITH a `theme` is passed
/// through untouched: this build cannot know what a pack installed tomorrow
/// calls its backgrounds, and refusing here on a list this binary holds would
/// reintroduce exactly the coupling T7 removed. The pack refuses an id it does
/// not have, which is where that judgement belongs.
fn selection_for(value: &str, theme: Option<&str>) -> Option<(String, String)> {
    match theme {
        Some(theme) => Some((value.to_string(), theme.to_string())),
        None => legacy_selection(value).map(|(b, t)| (b.to_string(), t.to_string())),
    }
}

/// Turn one `cmd` message into something to do, or a sentence saying why not.
fn command_for(c: &CmdMessage) -> Result<Command, String> {
    match c.cmd.as_str() {
        "brightness" => {
            let raw = c.value.as_deref().ok_or("brightness carried no value")?;
            let percent: u8 = raw
                .parse()
                .map_err(|_| format!("brightness `{raw}` is not a number"))?;
            // The schema's own range. Clamped rather than refused: a directive
            // that is nearly right should still dim the screen.
            Ok(Command::Brightness(percent.clamp(5, 100)))
        }
        "set_screensaver" => {
            let raw = c.value.as_deref().ok_or("set_screensaver carried no value")?;
            selection_for(raw, c.theme.as_deref())
                .map(|(background, theme)| Command::Screensaver { background, theme })
                .ok_or_else(|| {
                    format!(
                        "set_screensaver `{raw}` carried no theme and is not one of the ten \
                         words an older controller sends, so there is no pair to select"
                    )
                })
        }
        "show_weather" => Ok(Command::ShowWeather(
            c.value.as_deref().is_some_and(|v| v == "on"),
        )),
        "rotate_corner" => {
            // An unknown corner name is a directive that meant something; the
            // schema's own fallback is a random different corner, so take it
            // rather than refusing the rotation altogether.
            let named = c.corner.as_deref().and_then(corner_for);
            Ok(Command::RotateCorner(named))
        }
        "display_sleep" => Ok(Command::ShowWidgets(false)),
        "display_wake" => Ok(Command::ShowWidgets(true)),
        // **`screenshot` is deliberately not built.** It names an absolute path
        // for the renderer to write a PNG to. This process runs as its own
        // unprivileged uid and may write only its own data directory, and it
        // has a QOI encoder rather than a PNG one — `Screen::capture`, which an
        // operator already reaches with `rivet apply input capture`. Honouring
        // the path would need both a PNG encoder and a write this machine's
        // sandbox is meant to refuse.
        "screenshot" => Ok(Command::Unhandled(c.cmd.clone())),
        other => Err(format!("`{other}` is not a directive this build knows")),
    }
}

/// **The connection to the controller**, polled once a frame and never blocking.
///
/// The same rule `crate::ingest` states for the slot sockets: a blocking read
/// would stop the frame loop, and the frame loop is the thing holding the
/// screen.
pub struct Controller {
    path: std::path::PathBuf,
    stream: Option<std::os::unix::net::UnixStream>,
    /// Bytes read but not yet a whole line.
    pending: Vec<u8>,
    /// When to try connecting again. The controller is a separate service and
    /// may restart; a renderer that gave up would need one too.
    retry_at: std::time::Instant,
    /// When a `state` message last carried a clock.
    ///
    /// **This is what stands `crate::clock` down.** Two producers writing
    /// `Slot::Primary` on their own cadences would fight, so the local one
    /// draws only while this says nothing has arrived — see
    /// [`Controller::clock_is_live`].
    clock_seen_at: Option<std::time::Instant>,
}

/// How stale the controller's clock may be before the local one takes over.
///
/// **Longer than the controller's minute and shorter than the card's ttl.** The
/// controller sends a clock every minute, so 90 seconds tolerates one missed
/// tick without flapping; the card expires at 120, so the local producer is
/// drawing again before the slot could empty.
const CLOCK_GOES_STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(90);

/// How long to wait before trying the controller again — `aura-bridge`'s value,
/// and its reasoning: the supervisor's backoff is for a service that cannot
/// run, and this is one running correctly with nothing to read.
const RECONNECT_AFTER: std::time::Duration = std::time::Duration::from_secs(5);

/// A line longer than this is a controller that has stopped making sense; the
/// connection is dropped rather than buffered without bound.
const MAX_LINE: usize = 256 * 1024;

impl Controller {
    /// `None` when `aura-controller:kiosk.sock` was not granted — an ordinary
    /// state of a machine running the renderer alone, and not a fault.
    pub fn take(now: std::time::Instant) -> Option<Self> {
        let endpoint = rivet_sdk::app::endpoint("aura-controller", "kiosk.sock")?;
        Some(Self {
            path: endpoint.path,
            stream: None,
            pending: Vec::new(),
            retry_at: now,
            clock_seen_at: None,
        })
    }

    /// Whether the controller's clock is current enough to be the one drawn.
    ///
    /// `false` before the controller has ever spoken, which is what makes a
    /// renderer with no controller show a clock at all.
    #[must_use]
    pub fn clock_is_live(&self, now: std::time::Instant) -> bool {
        self.clock_seen_at
            .is_some_and(|seen| now.saturating_duration_since(seen) < CLOCK_GOES_STALE_AFTER)
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Read whatever is ready. Cards go into the store; directives come back.
    pub fn poll(
        &mut self,
        store: &mut crate::store::Store,
        now: std::time::Instant,
    ) -> (Vec<String>, Vec<Command>) {
        let mut notes = Vec::new();
        let mut commands = Vec::new();
        let mut saw_a_clock = false;

        if self.stream.is_none() {
            if now < self.retry_at {
                return (notes, commands);
            }
            match std::os::unix::net::UnixStream::connect(&self.path) {
                Ok(stream) => {
                    if stream.set_nonblocking(true).is_ok() {
                        notes.push("connected to the controller".to_string());
                        self.stream = Some(stream);
                        self.pending.clear();
                    } else {
                        self.retry_at = now + RECONNECT_AFTER;
                    }
                }
                Err(_) => {
                    self.retry_at = now + RECONNECT_AFTER;
                    return (notes, commands);
                }
            }
        }

        let Some(stream) = self.stream.as_mut() else {
            return (notes, commands);
        };
        let mut buffer = [0u8; 8192];
        let mut hung_up = false;
        loop {
            match std::io::Read::read(stream, &mut buffer) {
                Ok(0) => {
                    hung_up = true;
                    break;
                }
                Ok(n) => {
                    self.pending.extend_from_slice(&buffer[..n]);
                    if self.pending.len() > MAX_LINE {
                        notes.push(
                            "the controller sent a line longer than this build will buffer; the \
                             connection is dropped and retried"
                                .to_string(),
                        );
                        hung_up = true;
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    notes.push(format!("the controller's socket failed: {e}"));
                    hung_up = true;
                    break;
                }
            }
        }

        while let Some(at) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=at).collect();
            let Ok(text) = std::str::from_utf8(&line) else {
                notes.push("a line from the controller is not UTF-8 and was skipped".to_string());
                continue;
            };
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }
            if self.line(trimmed, store, now, &mut notes, &mut commands) {
                saw_a_clock = true;
            }
        }
        if saw_a_clock {
            self.clock_seen_at = Some(now);
        }

        if hung_up {
            self.stream = None;
            self.pending.clear();
            self.retry_at = now + RECONNECT_AFTER;
            notes.push(
                "the controller closed the connection; its cards stand until their ttl expires"
                    .to_string(),
            );
        }
        (notes, commands)
    }

    /// One line from the controller. Returns whether it carried a clock.
    ///
    /// The return value is how `poll` learns to stand the local clock down —
    /// see [`Controller::clock_is_live`]. It is a return rather than a field
    /// written here because this runs while `self.stream` is borrowed.
    fn line(
        &self,
        text: &str,
        store: &mut crate::store::Store,
        now: std::time::Instant,
        notes: &mut Vec<String>,
        commands: &mut Vec<Command>,
    ) -> bool {
        let Ok(message) = serde_json::from_str::<StateMessage>(text) else {
            notes.push("a line from the controller did not parse and was skipped".to_string());
            return false;
        };
        let mut carried_a_clock = false;
        match message.kind.as_str() {
            "state" => {
                // **The clock first, because it is the card that must not
                // blink.** `wire.rs`: *"the controller formats; the renderer
                // prints."*
                if let Some(card) = clock_card_for(&message) {
                    carried_a_clock = true;
                    self.apply_card(Some(card), Slot::Primary, store, now, notes);
                }
                self.apply_card(card_for(&message), Slot::Secondary, store, now, notes);
                // **Age is stripped before the store sees it.** `age_seconds`
                // advances every second and nothing draws it, so a card
                // carrying it dirties the store once a second and forces a
                // redraw that changes no pixel. The C++ excludes it from its
                // redraw comparison for the same reason.
                self.apply_card(
                    reading_card_for(&message).as_ref().map(ageless),
                    Slot::Tertiary,
                    store,
                    now,
                    notes,
                );
            }
            "cmd" => match serde_json::from_str::<CmdMessage>(text) {
                Ok(c) => match command_for(&c) {
                    Ok(command) => commands.push(command),
                    Err(why) => notes.push(why),
                },
                Err(why) => notes.push(format!("a cmd from the controller did not parse: {why}")),
            },
            // Unknown types are skipped, not refused: the schema is the
            // controller's and may gain kinds this build has never heard of.
            _ => {}
        }
        carried_a_clock
    }

    /// A card the controller's state produced.
    ///
    /// # Nothing to show is NOT a withdrawal
    ///
    /// `None` here means *this `state` message carried nothing for that slot* —
    /// no `weather` block, or no `reading` block. It does **not** mean the slot
    /// should be emptied, and treating the two as the same thing was a defect
    /// with a wide blast radius.
    ///
    /// Every `state` message used to produce a `Message::Withdraw` for any slot
    /// it had no content for. Those slots are **also the addon endpoints** —
    /// `SPEC-04` §4.4 makes all four claimable, `aura-vitals-test` claims
    /// `tertiary`, and this manifest still declares all four in `provides`. So
    /// an addon would place a card and the controller's next state message,
    /// arriving seconds later with no `reading` block, would delete it. A
    /// controller with no reading source installed sends no `reading` block
    /// at all, which makes that the ordinary case rather than the corner
    /// one: the slot an addon was granted was erased on a cadence. Found in the
    /// audit of 2026-09-03 (`PLAN.md` §123, register R-21).
    ///
    /// **What replaces it is the model this file already uses elsewhere: the
    /// ttl.** `SPEC-04` gives every card an expiry, and the disconnect path a
    /// few lines above says the same thing in its own note — *"its cards stand
    /// until their ttl expires"*. A controller that stops having weather stops
    /// refreshing the card, and the card ages out. That is slower than an
    /// explicit withdrawal by at most one ttl, and it cannot reach into a slot
    /// this process does not own.
    fn apply_card(
        &self,
        card: Option<Card>,
        slot: Slot,
        store: &mut crate::store::Store,
        now: std::time::Instant,
        notes: &mut Vec<String>,
    ) {
        let Some(card) = card else {
            return;
        };
        if let Err(why) = store.apply(slot, &aura_card::Message::Card(card), now) {
            notes.push(format!("{}: {why}", slot.as_str()));
        }
    }
}

#[cfg(test)]
mod cmd_tests {
    #![allow(clippy::expect_used)]
    use super::*;

    #[test]
    fn the_schemas_two_worked_examples_are_understood() {
        // `{ "type": "cmd", "cmd": "brightness", "value": "100" }`
        let c = CmdMessage {
            cmd: "brightness".to_string(),
            value: Some("100".to_string()),
            corner: None,
            theme: None,
        };
        assert_eq!(command_for(&c), Ok(Command::Brightness(100)));
    }

    /// **A current controller sends the pair and it arrives unaltered.**
    ///
    /// The `starfield` case is the one that matters: this binary carries no
    /// list of backgrounds and must pass it through anyway, because the pack
    /// on the board decides what exists. A build that validated against its own list
    /// here would be the coupling T7 removed, reintroduced on the wire.
    #[test]
    fn a_named_pair_reaches_the_pack_untouched() {
        assert_eq!(
            selection_for("digital-rain", Some("ice-blue")),
            Some(("digital-rain".to_string(), "ice-blue".to_string()))
        );
        assert_eq!(
            selection_for("starfield", Some("amber")),
            Some(("starfield".to_string(), "amber".to_string())),
            "a background this build has never heard of still reaches the pack"
        );
    }

    /// **Every word an older controller can send still unfolds**, and to the
    /// ids the shipped pack actually uses.
    ///
    /// The two spelled out are the pair that caused the original defect — a
    /// portal set to *Ice Blue* left a green screen because nothing mapped the
    /// word — and they are spelled out so a careless edit to the table cannot
    /// pass by mapping everything to the same place.
    #[test]
    fn every_legacy_screensaver_word_still_unfolds() {
        assert_eq!(
            legacy_selection("construct_ice"),
            Some(("digital-rain", "ice-blue"))
        );
        assert_eq!(
            legacy_selection("construct_green"),
            Some(("digital-rain", "phosphor-green"))
        );
        for value in [
            "solid",
            "solid_white",
            "solid_navy",
            "construct_green",
            "construct_ice",
            "born_this_way",
            "boxes",
            "boxes_green",
            "boxes_pride",
            "boxes_dark",
        ] {
            assert!(legacy_selection(value).is_some(), "{value} did not unfold");
        }
    }

    /// An unknown word with no theme beside it selects nothing, so the caller
    /// can say so by name rather than silently keeping the old background.
    #[test]
    fn an_unknown_word_with_no_theme_selects_nothing() {
        assert_eq!(selection_for("starfield", None), None);
    }

    #[test]
    fn brightness_is_clamped_to_the_schemas_range() {
        let at = |v: &str| {
            command_for(&CmdMessage {
                cmd: "brightness".to_string(),
                value: Some(v.to_string()),
                corner: None,
                theme: None,
            })
        };
        assert_eq!(at("0"), Ok(Command::Brightness(5)));
        assert_eq!(at("60"), Ok(Command::Brightness(60)));
    }

    #[test]
    fn show_weather_is_on_or_it_is_not() {
        let at = |v: &str| {
            command_for(&CmdMessage {
                cmd: "show_weather".to_string(),
                value: Some(v.to_string()),
                corner: None,
                theme: None,
            })
        };
        assert_eq!(at("on"), Ok(Command::ShowWeather(true)));
        assert_eq!(at("off"), Ok(Command::ShowWeather(false)));
    }

    /// A directive named in the schema and not yet built is carried, not
    /// dropped — so the journal can say so once rather than going quiet.
    ///
    /// **`screenshot`, not `rotate_corner`.** This asserted `rotate_corner`
    /// until 2026-09-03, and it had been failing since the day the rotation was
    /// built: `command_for` returns `Command::RotateCorner`, `Command` derives
    /// `PartialEq`, and the assertion could not hold. Nobody saw it because
    /// this crate is its own workspace and `cargo test` at the repository root
    /// never compiled it — the gap `tools/test-all.sh` now closes
    /// (`PLAN.md` §123). `screenshot` is the directive that really is carried
    /// and not built, for the reason `command_for` gives at it.
    #[test]
    fn a_named_but_unbuilt_directive_is_carried() {
        let c = CmdMessage {
            cmd: "screenshot".to_string(),
            value: Some("/tmp/somewhere.png".to_string()),
            corner: None,
            theme: None,
        };
        assert_eq!(command_for(&c), Ok(Command::Unhandled("screenshot".to_string())));
    }

    /// And the rotation IS built, which is what the test above used to deny.
    #[test]
    fn a_named_corner_is_rotated_to_and_an_unknown_one_is_still_a_rotation() {
        let at = |corner: Option<&str>| {
            command_for(&CmdMessage {
                cmd: "rotate_corner".to_string(),
                value: None,
                corner: corner.map(str::to_string),
                theme: None,
            })
        };
        assert_eq!(at(Some("top-right")), Ok(Command::RotateCorner(Some(Corner::TopRight))));
        assert_eq!(at(Some("bottom-left")), Ok(Command::RotateCorner(Some(Corner::BottomLeft))));
        // An unknown corner is still a rotation — the schema's own fallback is
        // a random different corner, and refusing would be doing nothing when
        // something was asked for.
        assert_eq!(at(Some("middle")), Ok(Command::RotateCorner(None)));
        assert_eq!(at(None), Ok(Command::RotateCorner(None)));
    }

    /// A directive this build has never heard of is refused by name, which is
    /// the other half of the pair above: carried when the schema names it,
    /// refused when nothing does.
    #[test]
    fn a_directive_from_no_schema_at_all_is_refused_by_name() {
        let c = CmdMessage { cmd: "reboot_the_board".to_string(), value: None, corner: None, theme: None };
        let why = command_for(&c).expect_err("refused");
        assert!(why.contains("reboot_the_board"), "{why}");
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// **Hand-composed after the worked example in `Aura/docs/ipc-schema.md`**
    /// — the `clock`, `weather` and `indoor` blocks are that example's bytes,
    /// the indoor `label` is a neutral one, and the corner card's block is
    /// the controller's `reading` object carrying a river gauge's value,
    /// because what the original example measured is the source's business
    /// and not this repository's (`.rulings/` R-375 D1). `AGENTS.md` §4: a
    /// fixture's provenance is part of it.
    const SCHEMA_EXAMPLE: &str = r#"{
      "type": "state",
      "clock":  { "time": "22:15", "date": "Friday, August 14" },
      "weather": {
        "temp_f":         70,
        "secondary_f":    70,
        "secondary_kind": "dewpoint",
        "unit":           "F",
        "condition":      "Cloudy",
        "icon":           "bkn",
        "statement":      "Chance Rain Showers"
      },
      "indoor": {
        "label":        "Sensor A",
        "temp_f":       76,
        "humidity_pct": 54,
        "unit":         "F",
        "online":       true
      },
      "reading": {
        "value":        2.4,
        "unit":         "m",
        "trend":        "flat",
        "out_of_range": false,
        "status":       "ok",
        "age_seconds":  95
      }
    }"#;

    fn card(json: &str) -> Card {
        let message: StateMessage = serde_json::from_str(json).expect("the fixture parses");
        card_for(&message).expect("there is something to show")
    }

    /// **The schema's own example becomes the panel `SPEC-04` §6.4 documents.**
    /// One card, one pair, one statement — and the card survives `aura-card`'s
    /// encoder, which is what proves it is a card and not merely a struct.
    #[test]
    fn the_documented_example_becomes_the_documented_card() {
        let card = card(SCHEMA_EXAMPLE);
        assert_eq!(card.slot, Slot::Secondary);
        let [Block::Pair(pair), Block::Statement(s)] = card.blocks.as_slice() else {
            panic!("expected a pair and a statement, got {:?}", card.blocks)
        };
        assert_eq!(pair.left.glyph, Some(Glyph::Cloudy));
        assert_eq!(pair.left.value.as_deref(), Some("70"));
        assert_eq!(pair.left.unit.as_deref(), Some("°F"));
        assert_eq!(pair.left.secondary.as_deref(), Some("70"));
        assert_eq!(pair.right.glyph, Some(Glyph::Indoor));
        assert_eq!(pair.right.value.as_deref(), Some("76"));
        assert_eq!(pair.right.secondary.as_deref(), Some("54"));
        // The one place the two columns differ: a room's second line is a
        // percentage and the sky's is another temperature.
        assert_eq!(pair.right.secondary_unit.as_deref(), Some("%"));
        assert_eq!(s.text, "Chance Rain Showers");
        aura_card::Message::Card(card).encode().expect("a legal card");
    }

    /// **`online: false` clears the indoor column and keeps the panel**, which
    /// is what `main.cpp` does and is the case `IndoorData.online` exists for.
    #[test]
    fn a_sensor_that_is_not_online_leaves_a_column_that_says_so() {
        let json = r#"{"type":"state",
            "weather":{"temp_f":64,"unit":"F","icon":"ovc","statement":"Cloudy"},
            "indoor":{"temp_f":75,"humidity_pct":67,"unit":"F","online":false}}"#;
        let card = card(json);
        let [Block::Pair(pair), _] = card.blocks.as_slice() else {
            panic!("expected a pair and a statement")
        };
        assert_eq!(pair.right.state, State::Unavailable);
        assert_eq!(pair.right.value, None);
        assert_eq!(
            pair.right.glyph, None,
            "no room reading means no room glyph, so no width is reserved for one"
        );
        assert_eq!(pair.left.value.as_deref(), Some("64"));
        aura_card::Message::Card(card).encode().expect("a legal card");
    }

    /// A message with no weather and no indoor is **nothing to show**, and the
    /// caller withdraws rather than sending an empty card — which the
    /// vocabulary cannot express anyway.
    #[test]
    fn a_message_with_nothing_in_it_produces_no_card() {
        assert!(card_for(&StateMessage::default()).is_none());
        let empty: StateMessage =
            serde_json::from_str(r#"{"type":"state","indoor":{"online":false}}"#).expect("parses");
        assert!(card_for(&empty).is_none());
    }

    /// **Unknown fields are ignored**, because the schema being read says they
    /// are and it is not this program's schema. The failure this prevents is a
    /// bridge that stops working the day the controller grows a field.
    #[test]
    fn a_field_this_build_has_never_heard_of_does_not_stop_the_card() {
        let json = r#"{"type":"state","pressure_mb":1013,
            "weather":{"temp_f":64,"unit":"F","icon":"ovc","uv_index":3}}"#;
        let card = card(json);
        assert_eq!(card.blocks.len(), 1, "no statement was sent");
    }

    /// **The slug table is `DrawWeatherIcon`'s, in its order**, and the order is
    /// the part that is easy to get wrong: NWS slugs compose, so `rain_showers`
    /// has to reach the rain branch and `tsra_sct` the thunder one even though
    /// `sct` appears in it.
    #[test]
    fn every_slug_reaches_the_branch_the_c_would_have_drawn() {
        for (slug, expect) in [
            ("skc", Some(Glyph::Clear)),
            ("hot", Some(Glyph::Clear)),
            ("few", Some(Glyph::PartlyCloudy)),
            ("sct", Some(Glyph::PartlyCloudy)),
            ("bkn", Some(Glyph::Cloudy)),
            ("ovc", Some(Glyph::Cloudy)),
            ("rain_showers", Some(Glyph::Rain)),
            ("drizzle", Some(Glyph::Rain)),
            ("tsra_hi", Some(Glyph::Thunder)),
            ("blizzard", Some(Glyph::Snow)),
            ("fzra", Some(Glyph::Sleet)),
            ("haze", Some(Glyph::Fog)),
            ("wind_bkn", Some(Glyph::Cloudy)),
            ("unavailable", Some(Glyph::Unavailable)),
            ("", None),
            ("something_nws_grew_last_year", None),
        ] {
            assert_eq!(glyph_for(slug), expect, "slug {slug:?}");
        }
    }

    /// A temperature is drawn whole, and the unit carries the degree sign — so
    /// the renderer concatenates and gets `"70°F"`, which is the string
    /// `"%.0f\xC2\xB0%s"` produced. A fractional reading rounds rather than
    /// showing a decimal the C++ never showed.
    #[test]
    fn a_temperature_is_whole_and_its_unit_carries_the_degree_sign() {
        let json = r#"{"type":"state","weather":{"temp_f":63.7,"secondary_f":-2.4,"unit":"C"}}"#;
        let card = card(json);
        let [Block::Pair(pair)] = card.blocks.as_slice() else {
            panic!("expected one pair")
        };
        assert_eq!(pair.left.value.as_deref(), Some("64"));
        assert_eq!(pair.left.unit.as_deref(), Some("°C"));
        assert_eq!(pair.left.secondary.as_deref(), Some("-2"));
    }

    /// A controller with nothing for a slot leaves an ADDON's card alone.
    ///
    /// **The defect this pins** — audit of 2026-09-03, register R-21. Every
    /// `state` message produced a `Message::Withdraw` for each slot it had no
    /// content for, and those slots are the addon endpoints (`SPEC-04` §4.4;
    /// `aura-vitals-test` claims `tertiary`). A controller with no reading
    /// source sends no `reading` block at all, so an addon's card was deleted
    /// on the controller's own cadence — the ordinary case, not a corner one.
    ///
    /// Both slots the controller touches are checked, because `card_for`
    /// returns `None` for a message with no usable weather in exactly the way
    /// `reading_card_for` does, and the audit named only the second.
    #[test]
    fn a_state_message_with_nothing_in_it_does_not_evict_an_addons_card() {
        let now = std::time::Instant::now();
        let addon_card = |slot: Slot| aura_card::Card {
            slot,
            ttl_seconds: 900,
            blocks: vec![Block::Statement(Statement {
                text: "an addon put this here".to_string(),
                ..Statement::default()
            })],
        };

        let mut store = crate::store::Store::default();
        for slot in [Slot::Secondary, Slot::Tertiary] {
            store
                .apply(slot, &aura_card::Message::Card(addon_card(slot)), now)
                .expect("an addon's card is accepted on the slot it was granted");
        }

        let controller = Controller {
            path: std::path::PathBuf::from("/nonexistent"),
            stream: None,
            pending: Vec::new(),
            retry_at: now,
            clock_seen_at: None,
        };
        let (mut notes, mut commands) = (Vec::new(), Vec::new());
        // A `state` message with neither a `weather` block nor a `reading` one
        // — what a controller with no reading source installed sends.
        controller.line(
            r#"{"type":"state","clock":{"time":"22:15","date":"Friday"}}"#,
            &mut store,
            now,
            &mut notes,
            &mut commands,
        );

        for slot in [Slot::Secondary, Slot::Tertiary] {
            assert!(
                store.get(slot).is_some(),
                "the controller had nothing for {} and erased the addon's card anyway; \
                 nothing to show is not a withdrawal",
                slot.as_str()
            );
        }
    }
}

#[cfg(test)]
mod reading_tests {
    #![allow(clippy::expect_used, clippy::panic)]
    use super::*;

    /// **Hand-composed: the `reading` object the controller's
    /// `wire::ReadingData` serializes, carrying a river gauge's value** — the
    /// field set and the omissions are the controller's own (its
    /// `wire::tests::the_reading_card_is_written_under_the_key_reading` pins
    /// the other side); the quantity is invented, because what a real source
    /// measures is not this repository's to say (`.rulings/` R-375 D1).
    /// `AGENTS.md` §4: a fixture's provenance is part of it.
    const READING_EXAMPLE: &str = r#"{
      "type": "state",
      "reading": {
        "value":        2.4,
        "unit":         "m",
        "trend":        "flat",
        "out_of_range": false,
        "status":       "ok",
        "age_seconds":  95
      }
    }"#;

    fn parse(text: &str) -> StateMessage {
        serde_json::from_str(text).expect("the fixture is a state message")
    }

    fn reading_of(text: &str) -> Reading {
        let card = reading_card_for(&parse(text)).expect("a card");
        assert_eq!(card.slot, Slot::Tertiary);
        assert_eq!(card.ttl_seconds, READING_TTL_SECONDS);
        match &card.blocks[..] {
            [Block::Reading(r)] => r.clone(),
            _ => panic!("the corner card is exactly one reading"),
        }
    }

    #[test]
    fn the_controllers_reading_becomes_a_reading() {
        let r = reading_of(READING_EXAMPLE);
        assert_eq!(r.value.as_deref(), Some("2.4"));
        assert_eq!(r.unit.as_deref(), Some("m"));
        assert_eq!(r.trend, Some(Trend::Flat));
        assert_eq!(r.state, State::Ok);
        assert_eq!(r.role, Role::Normal);
        assert_eq!(r.age_seconds, Some(95));
    }

    /// **The card is read from the key `reading` and from no other.** The
    /// controller writes that key and nothing else for the corner card; a
    /// message carrying the card under any other key is a controller this
    /// build does not speak to, and draws no card rather than a wrong one.
    #[test]
    fn the_card_is_read_from_the_key_reading_and_no_other() {
        let renamed = READING_EXAMPLE.replace("\"reading\"", "\"something_else\"");
        assert!(reading_card_for(&parse(&renamed)).is_none());
        assert!(reading_card_for(&parse(READING_EXAMPLE)).is_some());
    }

    #[test]
    fn out_of_range_is_the_only_thing_that_makes_it_an_alarm() {
        let text = READING_EXAMPLE.replace("\"out_of_range\": false", "\"out_of_range\": true");
        assert_eq!(reading_of(&text).role, Role::Alarm);
    }

    /// `value` is omitted for `connecting`, and the card must still compose —
    /// the state is what it says.
    #[test]
    fn connecting_draws_without_a_number() {
        let r = reading_of(r#"{"type":"state","reading":{"status":"connecting"}}"#);
        assert!(r.value.is_none());
        assert_eq!(r.state, State::Connecting);
    }

    /// A controller with no reading source, or with the card switched off,
    /// sends no `reading` block. That is not a card and not a fault.
    #[test]
    fn no_reading_block_is_no_card() {
        assert!(reading_card_for(&parse(r#"{"type":"state"}"#)).is_none());
    }

    /// **An enabled card whose reading has dropped out still draws.**
    /// `Aura/docs/ipc-schema.md` makes `error` *"the connection alert alone"*
    /// and the controller only omits the block when the card is switched off —
    /// so a present block with no value is an ENABLED card, and it must draw.
    /// Drawing nothing for it is the defect that took the card off the wall
    /// for half an hour while a source was quiet.
    #[test]
    fn an_error_with_no_value_still_draws_the_alert() {
        let card = reading_card_for(&parse(r#"{"type":"state","reading":{"status":"error"}}"#))
            .expect("an enabled card with nothing to show is a state with a drawing");
        assert_eq!(card.slot, Slot::Tertiary);
        let Some(Block::Reading(reading)) = card.blocks.first() else {
            panic!("the alert is a reading with no value, not an empty card");
        };
        assert_eq!(reading.value, None, "there is no number to show, and none is shown");
        assert_eq!(
            reading.state,
            State::Unavailable,
            "the state is what makes `layout` draw the mark instead of a value"
        );
    }

    /// And `connecting` — a cold start rather than a fault — draws too, which
    /// it already did. Kept beside the case above so the pair reads as one
    /// rule: an absent block is off, a present one always draws something.
    #[test]
    fn connecting_with_no_value_draws_the_question() {
        let card =
            reading_card_for(&parse(r#"{"type":"state","reading":{"status":"connecting"}}"#))
                .expect("a cold start is a state with a drawing");
        let Some(Block::Reading(reading)) = card.blocks.first() else {
            panic!("expected a reading");
        };
        assert_eq!(reading.value, None);
        assert_eq!(reading.state, State::Connecting);
    }

    /// **The source's own rounding is printed, to at most one decimal**, and no
    /// unit is consulted — there is none the host knows.
    ///
    /// **No value here sits on a rounding boundary, and that is deliberate.**
    /// `6.55` is not representable in binary — it is a hair below — so `{:.1}`
    /// gives `6.5`, and asserting `6.6` would test the platform's half-rounding
    /// rather than this function. `§115.2` records what that class of fixture
    /// cost the two vitals producers: agreement that held except at whichever
    /// value happened to land on the boundary.
    #[test]
    fn a_value_keeps_the_sources_rounding_up_to_one_decimal() {
        // A source that sends whole numbers draws whole numbers.
        assert_eq!(reading_value(42.0), "42");
        assert_eq!(reading_value(0.0), "0");
        // A source that rounds to a tenth draws tenths.
        assert_eq!(reading_value(6.6), "6.6");
        assert_eq!(reading_value(2.4), "2.4");
        // A source that does not round at all is cut to the card's one decimal.
        assert_eq!(reading_value(6.62), "6.6");
        assert_eq!(reading_value(21.333_333), "21.3");
        // Negative values keep their sign, and a negative zero is just zero.
        assert_eq!(reading_value(-1.5), "-1.5");
        assert_eq!(reading_value(-0.04), "0");
    }

    /// **The one thing the rule cannot do, stated so nobody mistakes it for a
    /// defect**: a tenths source whose value lands on a whole number draws
    /// without the `.0`, because the protocol carries a number and not a
    /// precision.
    #[test]
    fn a_whole_value_from_a_tenths_source_draws_without_its_trailing_zero() {
        assert_eq!(reading_value(2.0), "2");
    }

    #[test]
    fn an_unknown_trend_is_no_arrow_rather_than_a_flat_one() {
        assert_eq!(trend_for(Some("unknown")), None);
        assert_eq!(trend_for(Some("rising_fast")), Some(Trend::RisingFast));
        assert_eq!(trend_for(None), None);
    }
}
