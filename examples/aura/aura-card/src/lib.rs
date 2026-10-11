//! **The Aura card vocabulary** — `SPEC-04`, the normative document, and this
//! crate is its Rust implementation.
//!
//! # What an addon publishes
//!
//! > An addon publishes a **card**, not pixels.
//!
//! `rivet-launcher` holds DRM master and cuts exactly one display lease to
//! exactly one application (`PLAN.md` §106). There can never be a second
//! process drawing on that screen, so an addon that wanted to draw could not
//! be given the means. What it is given instead is a socket and a vocabulary:
//! it says *a reading, this value, this unit, this trend, and it is alarming*,
//! and the renderer decides what that looks like.
//!
//! Three rules follow, and they are the whole design:
//!
//! * **Slots and shapes, not pixels.** [`Shape`] is closed. An addon chooses a
//!   shape and occupies a [`Slot`]; it cannot invent a visual. A future addon
//!   that needs a shape this list lacks is a change to Aura, not a licence to
//!   draw.
//! * **A card carries a meaning, never a colour.** [`Role`] names what
//!   something *is* — ordinary, an aside, emphasised, not right yet, wrong —
//!   and the theme decides the pixels. This is what keeps an addon written
//!   today looking right under a theme written next year.
//! * **Every card expires.** [`Card::ttl_seconds`] is required, because Aura's
//!   own history says so: `IndoorData.online` and the status on the original
//!   renderer's reading card both exist in the C++ renderer for one reason — *a
//!   dead sensor and a perfectly stable room produce the same flat line*, and a
//!   reading that is no longer true is worse on a wall than no reading at all.
//!
//! # Nothing here knows what a bus is
//!
//! The vocabulary is derived from Aura's C++ renderer (`cpp/src/message.hpp`,
//! `cpp/src/widgets.hpp`), and the risk in deriving it from that source is that
//! the corner card's value and indoor temperature are **both** readings, so a
//! vocabulary that
//! only ever has to carry a big number could look complete and be far too
//! narrow. [`List`] is the guard against that, and its field names —
//! [`Row::lead`], [`Row::label`], [`Row::trail`] — deliberately say nothing
//! about routes, destinations or minutes. `tests::a_list_is_not_bus_shaped`
//! holds it: the same shape carries a departure board and a day's agenda, and
//! the renderer's layout rule can be stated without naming either.
//!
//! # There is no privileged producer
//!
//! Weather ships in the box and is an addon in every respect except
//! distribution: same socket, same vocabulary, same slot rules, no fast path.
//! A private channel for the cards that ship is how an addon API rots, because
//! the shipped cards quietly acquire reach that a third party cannot get and
//! nobody notices until the fourth card is written.
//!
//! **The clock is the one exception, and it is an exception about distance
//! rather than about privilege** — `SPEC-04` §4.3. The renderer already holds
//! the time, because it is drawing frames; interposing a process and a socket
//! between it and the current time would add a failure mode to the only card
//! that cannot otherwise fail. So the renderer *synthesises* a clock [`Card`]
//! in this same vocabulary and hands it to the same layout path. One drawing
//! path, no special cases; the only difference is that nothing had to arrive
//! over a socket.

#![deny(unsafe_code)]

mod wire;

pub use wire::{Fault, Framer, LIMITS, Limits};

// ---------------------------------------------------------------------------
// Versioning
// ---------------------------------------------------------------------------

/// The vocabulary this crate speaks.
///
/// Carried on the first line of every message, so an addon and a renderer that
/// were built years apart can each say what they know. The rule, in full, is
/// `SPEC-04` §8:
///
/// * A renderer **must** accept every vocabulary at or below its own. Growing
///   the vocabulary is adding shapes, roles, glyphs and optional keys; a card
///   written against 1 is still a valid card under 2.
/// * A renderer **must refuse** a card declaring a vocabulary above its own,
///   by number, in the journal, and keep drawing whatever that slot last held.
///   It cannot know what it is not being told.
pub const VOCABULARY: u32 = 1;

// ---------------------------------------------------------------------------
// Slots
// ---------------------------------------------------------------------------

/// Where a card sits, by **prominence** — never by position.
///
/// # Why four
///
/// Because the C++ renderer has four corners and rotates through them
/// (`cpp/src/main.cpp`, `RandomCorner`), which is a burn-in rule on a panel
/// that is on all day. The clock and the weather panel own one diagonal
/// between them and the reading card takes a corner of the other, so the real
/// arrangement has always been four cards at most. A fifth card would have
/// nowhere to go.
///
/// # Why prominence and not position
///
/// Because the position rotates. An addon that asked for the top left would be
/// asking for something that is true for four minutes at a time, and the burn-in
/// rule would then be an addon's business rather than the renderer's. Ordinal
/// names say the one thing an addon actually has an opinion about: how much of
/// the screen this deserves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Slot {
    /// The largest card. The clock, by default — see the crate note.
    Primary,
    /// The second. Conditions, in the shipped configuration.
    Secondary,
    /// The third.
    Tertiary,
    /// The fourth.
    Quaternary,
}

impl Slot {
    /// Every slot, most prominent first.
    pub const ALL: [Self; 4] = [
        Self::Primary,
        Self::Secondary,
        Self::Tertiary,
        Self::Quaternary,
    ];

    /// The name on the wire, and **the endpoint name in a manifest**.
    ///
    /// They are the same string on purpose. An addon claims a slot by
    /// declaring `connects = ["aura-renderer:secondary"]`, and the supervisor
    /// enforces one consumer per endpoint (`SPEC-03` §3.4). So the answer to
    /// *what happens when two addons want the same slot* is that the second one
    /// is refused by name and **is not started** — by the platform, at launch,
    /// before either program runs. Aura arbitrates nothing.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Secondary => "secondary",
            Self::Tertiary => "tertiary",
            Self::Quaternary => "quaternary",
        }
    }

    /// Read a slot name off the wire, or out of a manifest.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|slot| slot.as_str() == s)
    }
}

// ---------------------------------------------------------------------------
// Roles — the semantic colour set
// ---------------------------------------------------------------------------

/// **What a thing means**, from which the theme derives what colour it is.
///
/// # Why an addon may not send a colour
///
/// The C++ renderer already learned this and wrote it down beside
/// `CardTheme::alert`: contrast is part of the theme, because a red that reads
/// on a black ground is weak on a white one, and a theme drawing black text
/// needs a darker red beside it. Every one of Aura's ten screensavers carries
/// its own pair of signal colours for exactly that reason — the two light
/// themes ship `{186, 26, 26}` where the dark ones ship `{255, 69, 58}`.
///
/// An addon cannot know which theme is on the wall, and an addon written today
/// certainly cannot know about a theme written later. So it sends the meaning
/// and the theme resolves it. **Four roles, and each earns its place from
/// something the existing renderer already distinguishes** — which is the test,
/// and `accent` is the one that failed it.
///
/// # There were five, and the fifth was invented
///
/// `accent` was removed on 2026-09-02, on the maintainer's question: *where do
/// you need this accent colour that the Aura screensaver themes aren't already
/// providing?* Nowhere. `CardTheme` in `cpp/src/main.cpp` carries `bg`, `text`,
/// `alert` and `caution` — **four colours, because four is what Aura draws.**
///
/// Its own doc gave the game away and was left standing for a day anyway: *"no
/// card in the shipped set uses it yet, and it is here because a theme with no
/// way to be itself is a palette, not a theme."* That is an aesthetic argument,
/// not a requirement, and it produced a role no producer could use and no theme
/// could resolve — `widgets::role_ink` drew it as `normal` and `SPEC-04` §11
/// (15) recorded that as a defect to be fixed by choosing a hue.
///
/// **Removed rather than given a colour**, which closes that question by
/// deletion. Now is the moment it costs nothing: the vocabulary is at version 1
/// and no third party has written a producer against it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Role {
    /// The ordinary ink. `CardTheme::text`, and the default for everything.
    #[default]
    Normal,
    /// An aside: a unit beside a value, a caption under one, a rule between two
    /// columns. The C++ panel drew its divider at 35% of the text alpha, which
    /// is this role by another name.
    Muted,
    /// **Not right, yet.** `CardTheme::caution` — the amber connection question
    /// the reading card draws on `status: connecting`. A cold start is not a
    /// fault, and drawing it in the fault colour would teach a household to
    /// ignore the fault colour.
    Warn,
    /// **Wrong.** `CardTheme::alert` — a reading outside its band, or a
    /// connection that has gone quiet for good.
    Alarm,
}

impl Role {
    /// Every role.
    pub const ALL: [Self; 4] = [Self::Normal, Self::Muted, Self::Warn, Self::Alarm];

    /// The name on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Muted => "muted",
            Self::Warn => "warn",
            Self::Alarm => "alarm",
        }
    }

    /// Read a role off the wire.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.as_str() == s)
    }
}

// ---------------------------------------------------------------------------
// Trend
// ---------------------------------------------------------------------------

/// Which way a reading is going.
///
/// The five states `cpp/src/widgets.hpp` draws arrows for, and **no sixth**.
/// That file's own note is the reason: *a sixth state the relay grows must not
/// arrive on screen as a reassurance nobody sent.* On the wire an unknown trend
/// is expressed by **omitting the key**, so there is no `unknown` variant here
/// to be defaulted to by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Trend {
    /// Falling fast — the C++ `arrow_downward`.
    FallingFast,
    /// Falling — `south_east`.
    Falling,
    /// Flat — `arrow_forward`.
    Flat,
    /// Rising — `north_east`.
    Rising,
    /// Rising fast — `arrow_upward`.
    RisingFast,
}

impl Trend {
    /// Every trend.
    pub const ALL: [Self; 5] = [
        Self::FallingFast,
        Self::Falling,
        Self::Flat,
        Self::Rising,
        Self::RisingFast,
    ];

    /// The name on the wire. Aura's controller already spells these
    /// `falling_fast` and `rising_fast`; this vocabulary uses `-` throughout so
    /// that one rule covers every enumerated value on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FallingFast => "falling-fast",
            Self::Falling => "falling",
            Self::Flat => "flat",
            Self::Rising => "rising",
            Self::RisingFast => "rising-fast",
        }
    }

    /// Read a trend off the wire.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|trend| trend.as_str() == s)
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// **How much this reading can be believed**, decided by the producer.
///
/// Lifted whole from the status on the original renderer's reading card, and
/// for the reason that file gives: *how old a reading may be before it stops
/// being worth showing is a rule about data*, and the producer owns the data. A
/// renderer that had to decide this would be deciding it identically for a
/// river's level and a room's temperature, which are not the same question.
///
/// The renderer's obligation for each is `SPEC-04` §5.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum State {
    /// Current. Draw it.
    #[default]
    Ok,
    /// Real, but old enough to say so. Draw it **with a mark beside it** —
    /// never silently.
    Stale,
    /// Nothing yet, and nothing is wrong. A cold start.
    Connecting,
    /// Nothing, and something is wrong.
    Unavailable,
}

impl State {
    /// Every state.
    pub const ALL: [Self; 4] = [
        Self::Ok,
        Self::Stale,
        Self::Connecting,
        Self::Unavailable,
    ];

    /// The name on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Stale => "stale",
            Self::Connecting => "connecting",
            Self::Unavailable => "unavailable",
        }
    }

    /// Read a state off the wire.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == s)
    }

    /// Whether a [`Reading`] in this state may show a value at all.
    ///
    /// The C++ card enforced the same rule from the other side —
    /// *"a renderer that ignored the status still could not put a stale number
    /// on the wall"* — because the controller omitted the value. Stating it
    /// here as well means a producer that sends both is refused at the wire
    /// rather than trusted.
    #[must_use]
    pub const fn admits_a_value(self) -> bool {
        matches!(self, Self::Ok | Self::Stale)
    }
}

// ---------------------------------------------------------------------------
// Glyphs
// ---------------------------------------------------------------------------

/// The closed icon set.
///
/// Thirteen marks, and every one of the first eleven is a branch of
/// `DrawWeatherIcon` or `DrawIndoorIcon` in `cpp/src/widgets.hpp` — that is
/// where the list came from, rather than from imagination. The NWS slug space
/// is much larger; the C++ renderer already collapses it to these, so the
/// mapping from a slug to a glyph is a **producer's** job and always was.
///
/// Two are new and each is named by the card that needs it: [`Glyph::Clock`]
/// for the shipped clock card, and [`Glyph::Bus`] for a departures list.
///
/// **The connection marks are deliberately absent.** `cpp/src/widgets.hpp`
/// draws `android_wifi_3_bar_alert` and `android_wifi_3_bar_question`; those
/// are [`State::Unavailable`] and [`State::Connecting`] respectively, and the
/// renderer derives them. A producer that could send the mark *and* the state
/// could send two different answers to one question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Glyph {
    /// Clear sky. The C++ `skc`/`hot` branch — a disc and eight rays.
    Clear,
    /// Sun behind cloud. `few`/`sct`.
    PartlyCloudy,
    /// Overcast. `bkn`/`ovc`.
    Cloudy,
    /// `rain`/`shower`/`drizzle`.
    Rain,
    /// `tsra`/`thunder`.
    Thunder,
    /// `snow`/`blizzard`/`flurr`.
    Snow,
    /// `sleet`/`fzra`/`ice`.
    Sleet,
    /// `fog`/`haze`/`smoke`/`dust`.
    Fog,
    /// `wind`.
    Wind,
    /// A room rather than a sky — `DrawIndoorIcon`'s house with a thermometer
    /// in it. Kept although indoor temperature leaves for an addon: the addon
    /// needs it more than the shipped renderer did.
    Indoor,
    /// The report is that there is no report. The C++ `unavailable` branch, and
    /// **not** the same thing as [`State::Unavailable`]: this is *the weather
    /// service said nothing*, which is itself a condition worth drawing.
    Unavailable,
    /// A clock face. New — the shipped clock card.
    Clock,
    /// A bus. New — a departures list.
    Bus,
}

impl Glyph {
    /// Every glyph.
    pub const ALL: [Self; 13] = [
        Self::Clear,
        Self::PartlyCloudy,
        Self::Cloudy,
        Self::Rain,
        Self::Thunder,
        Self::Snow,
        Self::Sleet,
        Self::Fog,
        Self::Wind,
        Self::Indoor,
        Self::Unavailable,
        Self::Clock,
        Self::Bus,
    ];

    /// The name on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Clear => "clear",
            Self::PartlyCloudy => "partly-cloudy",
            Self::Cloudy => "cloudy",
            Self::Rain => "rain",
            Self::Thunder => "thunder",
            Self::Snow => "snow",
            Self::Sleet => "sleet",
            Self::Fog => "fog",
            Self::Wind => "wind",
            Self::Indoor => "indoor",
            Self::Unavailable => "unavailable",
            Self::Clock => "clock",
            Self::Bus => "bus",
        }
    }

    /// Read a glyph off the wire.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|glyph| glyph.as_str() == s)
    }
}

// ---------------------------------------------------------------------------
// The shapes
// ---------------------------------------------------------------------------

/// **One measured quantity, read from across the room.**
///
/// The shape the existing renderer draws three times over, in three functions
/// that are the same composition with different fields present:
///
/// | card | `value` | `unit` | `secondary` | `caption` | `trend` |
/// | :-- | :-- | :-- | :-- | :-- | :-- |
/// | the clock (`DrawClockWidget`) | `22:15` | — | — | `Monday, September 1` | — |
/// | the corner reading card | `2.4` | `m` | — | — | `rising` |
/// | outdoor (`DrawWeatherPanel`) | `70°` | `F` | `62°F` | `Cloudy` | — |
/// | indoor (the same, other column) | `76°` | `F` | `54%` | — | — |
///
/// # The clock is a reading, and the alternative was considered
///
/// A separate `Clock` shape would carry no field this one lacks: a time is a
/// value and a date is a caption. It was rejected on that ground alone. The one
/// real difference — that a clock's digits must not jitter as the minute
/// changes, which is why Aura loads a *monospaced* face for every number it
/// draws — is not a field, because it applies to **every** reading. It is a
/// renderer obligation instead (`SPEC-04` §5.2): a reading's value and
/// secondary are drawn with tabular figures.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reading {
    /// The mark beside the reading, if the producer has one.
    pub glyph: Option<Glyph>,
    /// The number, **already formatted by the producer**.
    ///
    /// A string and not a float, deliberately. The producer already owns
    /// every formatting rule there is — a source's own rounding, °F to °C,
    /// *"unit conversion happens once, in the controller"* — and a renderer
    /// that reformatted would be a second place those rules live. `ADR-011`.
    ///
    /// Absent when [`Reading::state`] does not admit one.
    pub value: Option<String>,
    /// What the value is measured in. Drawn beside the value, [`Role::Muted`].
    pub unit: Option<String>,
    /// The second line under the value: a wind chill, a dewpoint, a humidity.
    pub secondary: Option<String>,
    /// The second line's unit, when it differs from [`Reading::unit`] — which
    /// it does for the indoor column, where the first line is a temperature and
    /// the second is a percentage.
    pub secondary_unit: Option<String>,
    /// A short line under the reading. The clock's date; the weather's
    /// condition text.
    pub caption: Option<String>,
    /// Which way it is going. Absent means unknown, and draws nothing.
    pub trend: Option<Trend>,
    /// How much of this can be believed.
    pub state: State,
    /// What the value means. `out_of_range` in the old schema is
    /// [`Role::Alarm`] here.
    pub role: Role,
    /// How old the reading is, in seconds.
    ///
    /// Carried because the brief for this shape names it and because an
    /// operator reading a captured frame wants it. The renderer is **not**
    /// obliged to draw it and the shipped one does not: the C++ renderer
    /// deliberately excluded `age_seconds` from its redraw comparison, because
    /// a field that ticks every second and is not drawn repaints the screen
    /// once a second for ever.
    pub age_seconds: Option<u32>,
}

/// **Wrapped prose.**
///
/// One card needs it and it is the NWS `shortForecast`: `WeatherData.statement`,
/// which `DrawWeatherPanel` puts through `WrapText` at 44px under the columns.
/// It is the only free text in the whole of Aura's wire schema, and it is free
/// text because a forecast is a sentence somebody else wrote.
///
/// The second use is an addon explaining itself — *"the sensor has not reported
/// since 14:02"* — which is what a [`State::Unavailable`] reading cannot say on
/// its own.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Statement {
    /// The text, wrapped by the renderer to the card's width. Never
    /// pre-wrapped by the producer: the width is the renderer's fact.
    pub text: String,
    /// What it means. A forecast is [`Role::Normal`]; an explanation of a
    /// failure is [`Role::Warn`] or [`Role::Alarm`].
    pub role: Role,
}

/// **One row of a list.**
///
/// Three positions and no nouns. `BusDep{route, destination, minutes}` is
/// exactly this shape — `21` / `Rosedale Center` / `7 min` — and the field
/// names here deliberately refuse to say so, because a departure board is one
/// list and a day's agenda is another and the renderer must be unable to tell.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Row {
    /// A short token at the left: a route number, a time, a rank. Optional,
    /// and a list where no row has one draws no left column at all.
    pub lead: Option<String>,
    /// The row itself, and the only required part. Truncated by the renderer if
    /// it does not fit; never by the producer, which does not know the width.
    pub label: String,
    /// A short token at the right, aligned down the column: a countdown, a
    /// duration, a quantity.
    pub trail: Option<String>,
    /// What this row means. Per row, so *this bus leaves in two minutes* can be
    /// [`Role::Warn`] while the rest of the board is ordinary.
    pub role: Role,
}

/// **Rows.**
///
/// The shape that proves the vocabulary is not secretly a vocabulary of one.
/// See [`Row`], and `tests::a_list_is_not_bus_shaped`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct List {
    /// A heading over the rows. The stop's name; the day.
    pub caption: Option<String>,
    /// A mark beside the heading.
    pub glyph: Option<Glyph>,
    /// The rows, in the order they should be read. At most
    /// [`Limits::rows`] of them; a producer with more sends the ones that
    /// matter, because it knows which those are and the renderer does not.
    pub rows: Vec<Row>,
}

/// **Two readings whose meaning is the comparison between them.**
///
/// `SPEC-04` §5.8. Drawn side by side as one unit, with a rule between them,
/// their contents on a common band, and the group centred — so that a column
/// with nothing to report leaves the other in the middle rather than against an
/// edge.
///
/// # Why it is a shape and not two blocks
///
/// Because the composition is the meaning. The card that forces it asks *what
/// is the difference between these two numbers*, and two readings stacked or
/// split across two slots answer a different question. A renderer given two
/// blocks would have to guess that they belonged together; given one it cannot
/// get it wrong.
///
/// # Two, and not N
///
/// A row of four readings at these sizes does not fit on a 1080p panel, and a
/// shape admitting an unbounded row is a shape whose failure mode is a card
/// that silently truncates. A producer with three facts sends a [`List`].
///
/// # Nothing here knows what weather is
///
/// [`Pair::left`] and [`Pair::right`] are positions. The same shape carries a
/// two-zone thermostat, today's high beside tomorrow's, a battery bank's charge
/// beside its draw, or the home and away score of one match —
/// `tests::a_pair_is_not_weather_shaped` carries three of them, for the reason
/// `tests::a_list_is_not_bus_shaped` exists.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pair {
    /// The first column.
    pub left: Reading,
    /// The second. **Required**, like the first: a side with nothing to report
    /// is spelled with its [`Reading::state`], which says *why* where an absent
    /// key would not, and keeps the card the same shape from one minute to the
    /// next. That is what `cpp/src/main.cpp` does when `IndoorData.online` goes
    /// false — it clears the column and keeps the panel.
    pub right: Reading,
}

/// **The closed vocabulary of shapes.**
///
/// ~~Three, and the fourth case in the brief for this work — *whatever the clock
/// and weather cards actually need* — turned out to need nothing new.~~ **Four,
/// corrected 2026-09-02** (`SPEC-04` §5, §5.8, `.rulings/` R-126). The fourth
/// case did need something new, and what hid it was that the weather card had
/// already been trimmed to fit the first three. The clock is a [`Reading`]; the
/// weather card is a [`Pair`] and a [`Statement`] stacked. That second fact is
/// why a [`Card`] holds an ordered sequence of blocks rather than being one
/// shape: the composition is the card's, and the shapes are the alphabet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// A [`Reading`].
    Reading(Reading),
    /// A [`Statement`].
    Statement(Statement),
    /// A [`List`].
    List(List),
    /// A [`Pair`].
    Pair(Pair),
}

/// The name of a [`Block`]'s shape, for the wire and for a refusal message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Shape {
    /// [`Block::Reading`].
    Reading,
    /// [`Block::Statement`].
    Statement,
    /// [`Block::List`].
    List,
    /// [`Block::Pair`].
    Pair,
}

impl Shape {
    /// Every shape. **Closed** — this array is the vocabulary.
    pub const ALL: [Self; 4] = [Self::Reading, Self::Statement, Self::List, Self::Pair];

    /// The name on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reading => "reading",
            Self::Statement => "statement",
            Self::List => "list",
            Self::Pair => "pair",
        }
    }

    /// Read a shape name off the wire.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|shape| shape.as_str() == s)
    }
}

impl Block {
    /// Which shape this is.
    #[must_use]
    pub const fn shape(&self) -> Shape {
        match self {
            Self::Reading(_) => Shape::Reading,
            Self::Statement(_) => Shape::Statement,
            Self::List(_) => Shape::List,
            Self::Pair(_) => Shape::Pair,
        }
    }
}

// ---------------------------------------------------------------------------
// The card
// ---------------------------------------------------------------------------

/// **What an addon publishes.**
///
/// A slot, a lifetime, and an ordered sequence of blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    /// Which slot this occupies.
    ///
    /// It must be the slot whose endpoint the producer was granted. The
    /// renderer checks that and refuses a card that names another, because
    /// **the socket is the authority**: a producer that could name any slot
    /// could paint over a neighbour it was never granted.
    pub slot: Slot,
    /// How long this card remains true, in seconds. Required.
    ///
    /// # Why every card must expire
    ///
    /// This is the answer to *what happens when an addon goes silent*, and it
    /// is not a heartbeat. A silent addon is indistinguishable from a
    /// perfectly stable one — the exact confusion `IndoorData.online` exists to
    /// resolve — so the producer states, in advance, how long its own answer
    /// stays true. A gauge that reports every five minutes might say 900; a
    /// forecast refreshed every twenty might say 5400.
    ///
    /// When it expires the renderer **withdraws the card**, leaving the slot
    /// empty. It does not dim it, freeze it or draw a stale marker: those are
    /// [`State::Stale`], which is the producer's word, and a renderer inventing
    /// it would be inventing a claim about data it does not hold.
    pub ttl_seconds: u32,
    /// The blocks, drawn top to bottom. At most [`Limits::blocks`].
    ///
    /// **Never empty.** A card with nothing in it is a withdrawal, and a
    /// withdrawal has its own verb ([`Message::Withdraw`]) so that an encoder
    /// with a bug cannot express one by accident.
    pub blocks: Vec<Block>,
}

/// One message on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Show this. Replaces whatever the slot held.
    Card(Card),
    /// Show nothing in this slot, now, rather than when the last card expires.
    ///
    /// The polite half of a shutdown: an addon that is stopping cleanly says
    /// so, instead of leaving a card on the wall for the rest of its TTL.
    Withdraw {
        /// Which slot to clear.
        slot: Slot,
    },
}

impl Message {
    /// Which slot this message is about.
    #[must_use]
    pub const fn slot(&self) -> Slot {
        match self {
            Self::Card(card) => card.slot,
            Self::Withdraw { slot } => *slot,
        }
    }
}

#[cfg(test)]
mod tests;
