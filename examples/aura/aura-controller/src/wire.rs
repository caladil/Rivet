//! **The wire, and it is somebody else's document.**
//!
//! `Aura/docs/ipc-schema.md` is normative for what leaves this program, and the
//! reader at the other end is `examples/aura/aura-renderer/src/controller.rs`.
//! These are the Go structs in `internal/ipc/ipc.go` transliterated, field for
//! field, including which fields are omitted when empty — because the reader
//! distinguishes *absent* from *zero* and the schema says so in as many words:
//!
//! > A station that is not reporting sends no `temp_f` at all, because 0°F is
//! > an ordinary January reading in Minnesota and a bare number cannot
//! > distinguish the two.
//!
//! # `skip_serializing_if` is the whole of the contract, and it is per-field
//!
//! Go's `omitempty` is on some of these fields and not others, and the pattern
//! is not decorative. `condition`, `icon`, `label`, `online` and `status` are
//! written **always**, empty string and all; every reading is written **only
//! when present**. Copying the tags rather than reasoning about them is
//! deliberate: the renderer was written against what the Go program emits, not
//! against what it ought to.
//!
//! # There is no deserialiser here
//!
//! This is a one-way push (`ipc-schema.md`, *"Direction: Go controller → C++
//! renderer"*). Nothing arrives on this socket, so nothing here derives
//! `Deserialize`; a type that could be parsed would invite a reply channel the
//! protocol does not have.

use serde::Serialize;

/// The top-level envelope. `kind` is always `state` or `cmd`.
///
/// One struct for both message types, as in Go, rather than an enum: the
/// schema's envelope is a `type` field beside a flat set of optional members,
/// and an enum would render the same bytes through a `#[serde(tag)]` dance that
/// obscures rather than clarifies which fields belong to which message.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Message {
    #[serde(rename = "type")]
    pub kind: &'static str,

    // ---- `type: "state"` ------------------------------------------------
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clock: Option<Clock>,
    /// **Reserved and never populated**, exactly as in Go. The schema carries
    /// the row; no producer has ever filled it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bus: Vec<BusDeparture>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weather: Option<Weather>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indoor: Option<Indoor>,
    /// The corner reading card, under the key `reading`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reading: Option<ReadingData>,

    // ---- `type: "cmd"` --------------------------------------------------
    #[serde(skip_serializing_if = "str::is_empty")]
    pub cmd: &'static str,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub corner: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub value: String,
    /// **`set_screensaver`'s theme, since T7.**
    ///
    /// The theme used to be folded into [`Message::value`] — *Digital Rain* +
    /// *Ice Blue* went as the single word `construct_ice`, and both programs
    /// held a table of the ten words that could be spelled. That table is what
    /// stopped a pack growing an eleventh background without an edit in two
    /// programs, so it is gone: `value` carries the background id and this
    /// carries the theme id, exactly as the pack spells them.
    ///
    /// Skipped when empty, so every other directive's message is byte for byte
    /// what it was.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub theme: String,
}

impl Message {
    /// A `state` message carrying nothing but the clock.
    ///
    /// The aggregator fills the rest; this is what a tick starts from.
    pub fn state() -> Self {
        Self {
            kind: "state",
            ..Self::default()
        }
    }

    /// A directive. `cmd` names it, `value` is its argument where it has one.
    ///
    /// The renderer acts on receipt. Every directive in the schema except
    /// `rotate_corner` is expressible here; that one takes `corner` and is
    /// built by [`Message::rotate_corner`].
    pub fn cmd(cmd: &'static str, value: impl Into<String>) -> Self {
        Self {
            kind: "cmd",
            cmd,
            value: value.into(),
            ..Self::default()
        }
    }

    /// **`set_screensaver`, as the pair of ids the pack speaks.**
    ///
    /// Its own constructor rather than a second argument on [`Message::cmd`],
    /// for the reason `rotate_corner` has one: exactly one directive carries
    /// this field, and giving every call site an empty string to pass would
    /// be a field nine directives had to be given and none would use.
    pub fn screensaver(background: impl Into<String>, theme: impl Into<String>) -> Self {
        Self {
            kind: "cmd",
            cmd: "set_screensaver",
            value: background.into(),
            theme: theme.into(),
            ..Self::default()
        }
    }
}

/// Pre-formatted time and date. The controller formats; the renderer prints.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Clock {
    /// `HH:MM`, 12- or 24-hour per the config.
    pub time: String,
    /// `"Weekday, Month D"`, or empty when the date is switched off.
    pub date: String,
}

/// One bus departure row. Reserved; see [`Message::bus`].
#[derive(Debug, Clone, Serialize)]
pub struct BusDeparture {
    pub route: String,
    pub destination: String,
    pub minutes: i32,
}

/// Current outdoor conditions.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Weather {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temp_f: Option<f64>,
    /// The second outdoor reading, in the same unit as `temp_f`: a wind chill
    /// when the NWS reports one, a dewpoint otherwise.
    ///
    /// The NWS computes a wind chill only when it is cold and windy enough to
    /// mean anything, so **the season selects the reading** without Aura owning
    /// a calendar rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_f: Option<f64>,
    /// `windchill` or `dewpoint` — which one `secondary_f` is.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub secondary_kind: String,
    /// `F` or `C`. **Absent means Fahrenheit**, which the schema states.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub unit: String,
    /// The NWS `textDescription`. Carried, not drawn.
    pub condition: String,
    /// The NWS icon slug (`bkn`, `ovc`, `rain_showers`, …), or `unavailable`.
    pub icon: String,
    /// The NWS `shortForecast`. The only free text in Aura's whole schema.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub statement: String,
}

/// The latest reading from one TempMon sensor.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Indoor {
    /// The sensor's nickname. Carried for logs; not drawn.
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temp_f: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub humidity_pct: Option<f64>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub unit: String,
    /// **TempMon's own staleness verdict**, and `false` means *do not draw
    /// this*. A dead sensor and a perfectly stable room produce the same flat
    /// line, and nothing but this flag tells them apart.
    pub online: bool,
}

/// **The corner reading card**, from whatever source fills
/// `crate::addon::role::READING` — the `reading` object on the wire.
///
/// `ReadingData` and not `Reading`, after the original renderer's own
/// `WeatherData` and `IndoorData`, because `rivetos_addon::Reading` is the
/// protocol's reading this is translated FROM and the two must not be
/// confused in `crate::addon`, where both are in hand.
///
/// `crate::addon` fills it, translating from a protocol vocabulary an addon
/// can answer in without knowing what a card looks like — which is why a
/// different source, measuring something else entirely, needs no change on
/// either side of this struct.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReadingData {
    /// Already in [`ReadingData::unit`] — the source converts, the renderer
    /// prints. Omitted whenever `status` is `error` or `connecting`, so a
    /// renderer that ignored `status` still could not draw a stale number.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// Whatever the source measured in — `m` for a river gauge, `min` for a
    /// bus stop. Absent on a cold start's bare card, which has no reading to
    /// take one from (`crate::addon::card`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub unit: String,
    /// The source's arrow **as a name, not a number**: a sixth state it grows
    /// arrives as `unknown` and draws no arrow, rather than silently becoming
    /// one of the five.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub trend: String,
    /// The low/high verdict, **taken by the source against a band this program
    /// never sees**.
    ///
    /// A source that converts units for display decides the verdict in the
    /// unit its band is stated in, and only then converts the value — so the
    /// same reading is flagged identically whatever it is shown in. That rule
    /// belongs to the source, which is the side that knows what makes a
    /// reading worth marking; what crosses is the conclusion (`.rulings/`
    /// R-198 D2).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub out_of_range: bool,
    /// One of [`status`]'s four words. Always written.
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_seconds: Option<i64>,
}

/// What the corner reading card draws.
///
/// **These four words are Aura's and the choice between them is not.** How old
/// a reading may be before it stops being worth showing is a rule about data,
/// and the source is what owns the data — so it answers with
/// `rivetos_addon::Status` and `crate::addon::status_of` is the one place the
/// two vocabularies meet.
pub mod status {
    /// A reading under ten minutes old. Number, arrow, unit.
    pub const OK: &str = "ok";
    /// Ten to twenty minutes. The same, with the connection alert beside it:
    /// the number is probably still roughly true, and the fact that nothing
    /// newer has arrived is its own piece of information.
    pub const STALE: &str = "stale";
    /// Past twenty minutes, or a refusal that will keep happening. The alert
    /// alone — past that point a reading is not evidence about the present.
    pub const ERROR: &str = "error";
    /// A cold start, or a poll in flight after a routine failure. **Distinct
    /// from [`ERROR`] so a cold start does not announce itself as a fault.**
    pub const CONNECTING: &str = "connecting";
}

#[cfg(test)]
mod tests;
