//! **The three questions, their answers, and the vocabulary a setting is
//! described in.**
//!
//! # Why an addon states its own cadence
//!
//! [`Polled::call_again_in_ms`] is the field that decides this protocol's
//! shape, and it exists because of a defect rather than a preference.
//! A networked reading source polled by `aura-controller` makes its first
//! request before DHCP has finished, fails to resolve the name, and then waits
//! the whole 300-second poll interval — so the card is empty for five minutes
//! after every cold boot. The maintainer ruled it is fixed HERE rather than in
//! place (`.rulings/` R-197 D4).
//!
//! A fixed interval cannot express *try again shortly, the network was not up*
//! and cannot express *the relay told us to wait 900 seconds* either. Both are
//! the same field, answered by whoever knows: the addon.
//!
//! # Why a reading is held raw
//!
//! [`Reading`] carries a value and the instant it was taken, never a rendered
//! string and never a card. How old a reading is changes every second without
//! anything new arriving, so a status frozen at poll time would leave a number
//! looking fresh long after it stopped being so. The addon says what it
//! measured and when; what that means on a screen is the host's, and Aura's
//! renderer composes it from the host's own wire.

use serde::{Deserialize, Serialize};

/// What a host asks an addon.
///
/// Serialized with a `v` alongside it — see [`Envelope`] — so an addon that
/// is handed a request from a newer host can say so rather than guess.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum Request {
    /// **Who are you, and what do you need?** The first call after the
    /// handshake, and the only one whose answer a host may cache for the life
    /// of the load.
    Describe,
    /// **Here are your settings, and the material you asked for.**
    ///
    /// Sent once before the first [`Request::Poll`], and again whenever the
    /// operator saves — which is what makes a saved setting take effect at
    /// once instead of at the next interval. `aura-controller` calls that a
    /// nudge today and it is the same idea.
    Configure {
        /// Keyed by [`Field::key`]. A key the addon did not declare is not an
        /// error: it is ignored, which is how a host that still holds an old
        /// setting stops being a migration problem.
        settings: serde_json::Map<String, serde_json::Value>,
        /// The **contents** of each secret the addon named through
        /// [`FieldKind::Secret`], keyed by the secret's name. The host reads
        /// them; an addon never opens a file. Empty when the addon asked for
        /// none, or when the operator has installed none yet — which the
        /// addon reports through [`Failure`] rather than by failing to start.
        #[serde(default)]
        material: std::collections::BTreeMap<String, String>,
    },
    /// **What do you say right now?** — and not *go and fetch*.
    ///
    /// An addon does I/O only when its own schedule says so, and answers from
    /// what it already holds otherwise. That distinction is what lets a host
    /// ask as often as it likes: **how stale a reading is changes every second
    /// without anything new arriving**, and the staleness rule belongs to the
    /// addon, so a host that wants a current verdict has to be able to ask for
    /// one cheaply.
    ///
    /// [`Polled::call_again_in_ms`] is therefore a **ceiling** — ask again by
    /// then. A host with nothing to redraw waits exactly that long; one with a
    /// clock on a wall asks every second and costs the addon a comparison.
    ///
    /// Called off the host's critical path either way, because the call that
    /// does reach the network blocks until it returns.
    Poll {
        /// Wall-clock milliseconds, from the host, because a Rivet has no
        /// real-time clock of its own and an addon must not go looking for
        /// one (`ADR-021`). It is also what lets a host test an addon at an
        /// hour that is not now.
        now_ms: i64,
    },
}

/// What an addon answers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "kebab-case")]
pub enum Response {
    /// The answer to [`Request::Describe`].
    Described(Description),
    /// The answer to [`Request::Configure`].
    Configured {
        /// `false` means the addon cannot work with what it was given — a
        /// missing credential, a setting out of range. The host keeps the
        /// addon loaded and shows the complaint; it does not unload it,
        /// because an operator fixing a setting is the expected next event.
        usable: bool,
        /// One sentence for a person. Empty when `usable`.
        #[serde(default)]
        complaint: String,
    },
    /// The answer to [`Request::Poll`].
    Polled(Polled),
    /// The addon could not answer the question it was asked. Distinct from a
    /// [`Polled`] with no reading, which is a successful poll that found
    /// nothing.
    Fault(Failure),
}

/// An addon's own account of itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Description {
    /// The product name, for the journal and for a person. Not an identifier:
    /// the class is what joined this addon to this host.
    pub name: String,
    /// The addon's own version string, as its package carries it.
    pub version: String,
    /// The class it provides — the same string its signed manifest declares
    /// (`ADR-053` §2). A host refuses an addon whose stated class is not the
    /// one it opened, which catches a mislabelled build before it is trusted.
    pub class: String,
    /// The settings this addon has, in the order an operator should see them.
    #[serde(default)]
    pub fields: Vec<Field>,
    /// Secrets, by name, that the addon needs the host to read for it. The
    /// host resolves each through its own mechanism and sends the contents in
    /// [`Request::Configure`]. An addon never touches the secret store.
    #[serde(default)]
    pub material: Vec<String>,
    /// **The pack's own catalogue: what it can draw.** Empty for an addon that
    /// draws nothing, which is every reading source.
    ///
    /// This is the field that makes a **multipack** possible — `.rulings/`
    /// R-199 D1 ruled that the default backgrounds ship as one library
    /// describing several, and a host cannot offer a picker over a library
    /// that can only say its own name. A pack lists what it has here; the host
    /// renders two pickers from it and hands the chosen pair of ids back
    /// through [`crate::frame::Frame`].
    ///
    /// **Two hosts read this differently, and both are ordinary.**
    /// `aura-renderer` reads it to know what ids are legal and then draws;
    /// `aura-controller` reads it and never draws a pixel, because `Describe`
    /// is cheap and has no side effect. That is the arrangement
    /// `docs/design-candidates/Background-Addons-2026-09-19.md` §2 describes,
    /// and it needs nothing from the platform beyond two applications
    /// accepting one class.
    #[serde(default)]
    pub backgrounds: Vec<Background>,
}

/// **One thing a pack can draw**, and the themes it can draw it in.
///
/// # The id is the contract and the name is not
///
/// [`Background::id`] is what crosses at draw time, in
/// [`crate::frame::Frame::background`], and it is stable for the life of the
/// product: an operator's saved choice is this string, so renaming it changes
/// what a board draws after an update. [`Background::name`] is what a person
/// reads in a picker and is never compared to anything.
///
/// **Position is not an identity.** `.rulings/` R-201 D1 ruled that the order
/// of a list is a weak contract for something two signed artifacts depend on;
/// [`crate::frame::ID_BYTES`] carries that reasoning across to this catalogue,
/// which is why a frame names an id rather than an index into this list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Background {
    /// Stable and machine-read: `digital-rain`. The alphabet is
    /// [`crate::abi::class_is_nameable`]'s, bounded by
    /// [`crate::frame::ID_BYTES`], because this string has to fit a fixed-width
    /// field in a C struct.
    pub id: String,
    /// What the picker shows: `Digital Rain`. For a person, in whatever case
    /// and spacing suits them.
    pub name: String,
    /// The second picker. A background with one theme still lists it, so that
    /// a host renders one list and not two shapes of list.
    #[serde(default)]
    pub themes: Vec<Theme>,
    /// **Whether the cards drawn over this background should be dark**, for a
    /// pack that says nothing more precise.
    ///
    /// **[`Theme::cards`] supersedes this and a host should prefer it**, added
    /// 2026-09-20 when `aura-renderer` became a host for real. This field is a
    /// per-*background* bool, and the thing it describes varies per *theme*:
    /// the pack that shipped with it had to answer for `solid` as a whole,
    /// whose Black and Navy want a light card and whose White wants a dark
    /// one, and it answered wrong for White by construction.
    ///
    /// It is kept because a pack built against the older protocol still sends
    /// it and nothing else, and a host that dropped it would draw that pack's
    /// cards unreadable rather than merely plain.
    #[serde(default)]
    pub prefers_dark_cards: bool,
    /// **Which drawing boundary this background can be asked through** —
    /// `.rulings/` R-206 D2, *"a pack DECLARES that it draws on the GPU, in
    /// its Description, so a host can skip it rather than discover it"*.
    ///
    /// **Per background and not per pack**, because R-207 ships one pack
    /// whose Contours, Life and Boxes draw through [`crate::gl`] and whose
    /// Digital Rain and Starfield draw through [`crate::frame`]. A flag on
    /// the object could not describe that pack at all.
    ///
    /// Absent in a pack built against the older protocol, and absent means
    /// [`Drawn::Cpu`] — which is what every pack that predates this field
    /// actually is.
    #[serde(default)]
    pub drawn: Drawn,
}

/// **Where a background can draw** — see [`Background::drawn`].
///
/// # Why an enum rather than two bools
///
/// Two bools have four states and one of them is `drawn on neither`, which is
/// a background that cannot be drawn and therefore is not one. The same
/// argument `rivet_sdk::text::Format` makes about a pixel with no fourth byte
/// whose fourth byte is alpha: the type should not be able to spell the
/// nonsense, so the host never has to decide what it meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Drawn {
    /// Through [`crate::frame`] only — the host hands it pixels.
    ///
    /// **The default, and it is the right default**: a pack that says nothing
    /// was built before this field existed, and every one of those draws on
    /// the CPU.
    #[default]
    Cpu,
    /// Through [`crate::gl`] only — the host hands it a bound framebuffer.
    ///
    /// A host with no render node **skips** this background rather than
    /// offering it and failing, which is the whole of D2.
    Gpu,
    /// Either, and the host picks. R-206 D3's case, and what the shipped pack
    /// says about the three backgrounds R-207 moved to the GPU: they keep
    /// their CPU paths, because the x86 machine in this project's long-term
    /// targets has no V3D.
    Either,
}

impl Drawn {
    /// Whether the host may ask for this background through [`crate::frame`].
    #[must_use]
    pub const fn on_cpu(self) -> bool {
        matches!(self, Self::Cpu | Self::Either)
    }

    /// Whether the host may ask for this background through [`crate::gl`].
    #[must_use]
    pub const fn on_gpu(self) -> bool {
        matches!(self, Self::Gpu | Self::Either)
    }
}

/// One theme of one background.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Theme {
    /// Stable and machine-read: `ice-blue`. Same alphabet and same bound as
    /// [`Background::id`], and for the same reason.
    pub id: String,
    /// What the picker shows: `Ice Blue`.
    pub label: String,
    /// **What a card drawn over this theme should look like** — or the
    /// fallback, for a pack that sends no opinion.
    ///
    /// # Why the whole appearance and not a flag
    ///
    /// A host draws its own widgets and the pack draws the ground under them,
    /// so *legibility is a joint property of two programs* and exactly one of
    /// them knows what the ground is doing. Aura's ten original combinations
    /// are not two cards and a switch: Digital Rain's green ramp gets green-
    /// tinted ink on near-black at alpha 155, its Ice Blue ramp gets blue-
    /// tinted ink, Born This Way gets a transparent rainbow card with black
    /// ink, and the signal colours differ because contrast does. A bool cannot
    /// carry that, and a host holding the ten-way table itself is a host that
    /// has to be edited before the pack may grow an eleventh — which is the
    /// duplication a pack exists to remove.
    ///
    /// # Three answers, and `null` is one of them
    ///
    /// An omitted `cards` is *no opinion* ([`Cards::Unstated`]) and takes
    /// [`Background::prefers_dark_cards`]'s fallback, exactly as a pack built
    /// before this field existed does. An explicit `cards: null` is *draw no
    /// card over this theme* ([`Cards::NoCard`]), whatever the background
    /// prefers. An object is the card ([`Cards::Card`]). [`Cards`] says why the
    /// three are kept apart and why that needed no [`crate::abi::VERSION`]
    /// bump.
    #[serde(default, skip_serializing_if = "Cards::is_unstated")]
    pub cards: Cards,
    /// **Whether this theme changes the screen when nothing else has.**
    ///
    /// A host redraws when its own content is dirty; an animated background is
    /// the whole screen changing thirty times a second, which no dirty flag
    /// can see. `aura-renderer` has asked its own backgrounds this since the
    /// port (`isAnimated()`), and it has to keep asking once the answer lives
    /// in another library — a still background redrawn at 30 fps is a board
    /// spending its night's power on an unchanging picture.
    ///
    /// **Defaults to `false`, which is the safe direction for the machine and
    /// the wrong one for the picture.** A pack that forgets this field gets a
    /// background that draws once and then freezes, which is visible in a
    /// second and costs nothing; the reverse default would silently burn power
    /// on every still background ever shipped.
    #[serde(default)]
    pub animated: bool,
}

/// **What a theme says about the card drawn over it** — [`Theme::cards`].
///
/// # Why three states and not an `Option`
///
/// A pack has three different things to say, and an `Option` can carry only
/// two of them: *I have no opinion, use your fallback*, *draw no card here*,
/// and *draw this card*. With an `Option`, `null` and an omitted key both
/// parsed to `None`, so a pack could not ask for no card over a background
/// whose [`Background::prefers_dark_cards`] is set — the request and the
/// silence were the same value by the time a host read them.
///
/// On the wire the three are an omitted key, `null`, and an object. Each
/// spelling has exactly one meaning, so a host never has to guess which one a
/// pack meant.
///
/// # Why this needed no [`crate::abi::VERSION`] bump
///
/// The two hosts and the packs disagree about exactly one combination:
/// `null` over a background that prefers dark cards. A host built before
/// this type reads that `null` as *no opinion* and draws its plain dark
/// fallback card instead of none — legible, and only plainer than asked. A
/// pack built before this type serialized *no opinion* as `null`, so a host
/// built after it reads that pack's `null` as *no card*; the shipped
/// background pack only ever sent `null` over Solid Black, whose background
/// does not prefer dark cards, so its picture is the same under either host.
/// Every other combination reads identically both ways. A version bump would
/// have made every installed addon, including the reading sources that never
/// send a theme at all, unloadable by a new host until it was rebuilt — a
/// cost far larger than the one combination it would protect.
///
/// # Serializing [`Cards::Unstated`]
///
/// [`Theme::cards`] skips it, so a pack that says nothing omits the key. On
/// its own it **refuses to serialize**: the only spelling of *no opinion* is
/// an absent key, and writing `null` for it would say *no card* instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Cards {
    /// The key was absent: no opinion, so the host falls back to
    /// [`Background::prefers_dark_cards`].
    #[default]
    Unstated,
    /// `null`: draw no card over this theme — no rectangle, the widgets' own
    /// ink used as it is.
    NoCard,
    /// The card to draw.
    Card(CardAppearance),
}

impl Cards {
    /// Whether the pack said nothing — what [`Theme::cards`] skips when it
    /// serializes.
    #[must_use]
    pub const fn is_unstated(&self) -> bool {
        matches!(self, Self::Unstated)
    }
}

impl Serialize for Cards {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Unstated => Err(serde::ser::Error::custom(
                "an unstated card is written by omitting `cards`; `null` would ask for no card",
            )),
            Self::NoCard => serializer.serialize_none(),
            Self::Card(card) => serializer.serialize_some(card),
        }
    }
}

impl<'de> Deserialize<'de> for Cards {
    /// Called only when the key is present: an absent key never reaches this,
    /// because [`Theme::cards`] is `#[serde(default)]`.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Option::<CardAppearance>::deserialize(deserializer)? {
            None => Self::NoCard,
            Some(card) => Self::Card(card),
        })
    }
}

/// **The card a host draws behind its widgets, described by the pack drawing
/// the ground** — [`Theme::cards`], which is where the argument for this
/// crossing the boundary at all is written down.
///
/// Colours are `[r, g, b]` and never a packed `u32`: the packing order is a
/// surface format's business ([`crate::frame::FORMAT_XRGB8888`]), and a
/// catalogue that had to know it would be a catalogue coupled to a pixel
/// layout it never touches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardAppearance {
    /// The rectangle's fill.
    pub bg: [u8; 3],
    /// **How opaque that fill is, out of 255, and it is load-bearing.** 155 is
    /// the ported value for every dark card: the background stays visible
    /// through it and the type on it is still legible. `0` means the
    /// rectangle is not drawn at all while the rest of this appearance still
    /// applies — which is what the two transparent-card themes do.
    pub bg_alpha: u8,
    /// The ink on the card.
    pub text: [u8; 3],
    /// Draw a six-stop gradient card instead of a flat fill — the Born This
    /// Way themes. A host that cannot draw one falls back to [`Self::bg`],
    /// which is why this is a flag beside the colours rather than instead of
    /// them.
    pub rainbow: bool,
    /// **The two signal colours, which are per theme because contrast is.**
    /// `[255, 69, 58]` and `[255, 197, 0]` read on a dark card; the same two
    /// on a light one do not, and `[186, 26, 26]` and `[150, 95, 0]` are what
    /// the light themes use instead.
    pub alert: [u8; 3],
    /// See [`Self::alert`].
    pub caution: [u8; 3],
}

/// One setting, described so a portal can render and validate it without
/// knowing what the addon is.
///
/// **Typed rather than markup**, ruled 2026-09-19 (`.rulings/` R-197 D3). A
/// manifest is a typed permission request and a document is a typed
/// configuration surface with `get schema` to describe it; an addon's
/// settings are the same idea one level down. The portal keeps control of its
/// own page, every addon's settings look like the rest of the machine's, and
/// the vocabulary grows when a real addon needs a control it cannot express.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field {
    /// The key this field's value is carried under in
    /// [`Request::Configure::settings`].
    pub key: String,
    /// What a person reads beside the control.
    pub label: String,
    /// One sentence under it, or empty.
    #[serde(default)]
    pub help: String,
    pub kind: FieldKind,
}

/// The control an operator is given, and the rule the host validates against.
///
/// Five, which is what the three addons in front of this design need. A sixth
/// arrives when an addon needs it — `ADR-053` §4's argument for free-text
/// class names applies here too: both sides are already coordinating.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum FieldKind {
    /// A switch.
    Bool { default: bool },
    /// A number, with the range the host refuses outside of.
    Number {
        default: f64,
        min: f64,
        max: f64,
        /// For the label only — `ft`, `°F`, `s`. Never parsed.
        #[serde(default)]
        unit: String,
    },
    /// One of a fixed set. `(value, label)` — the value crosses, the label is
    /// read.
    Choice {
        default: String,
        options: Vec<(String, String)>,
    },
    /// Free text, bounded.
    Text {
        #[serde(default)]
        default: String,
        max_len: usize,
    },
    /// **Names an installed secret; never carries one.** The operator
    /// installs it through `cargo rivet secrets`, the host reads it, and the
    /// contents reach the addon in [`Request::Configure::material`] — so a
    /// credential is never a setting, never in a document, and never in
    /// anything a portal renders. `ADR-052` §4 refuses seeds in documents for
    /// the same reason.
    Secret { name: String },
}

/// The answer to a poll.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Polled {
    /// What was measured, or `None` when this poll found nothing. `None` with
    /// [`Status::Ok`] is legitimate: a source that has no reading yet is not
    /// a fault.
    #[serde(default)]
    pub reading: Option<Reading>,
    pub status: Status,
    /// **When to call again, from the addon.** A host clamps this to its own
    /// floor and ceiling — an addon asking to be called every millisecond is
    /// not obeyed — but within them the addon decides, which is what lets a
    /// failed name lookup retry in seconds while a good reading waits out the
    /// sensor's five minutes.
    pub call_again_in_ms: u64,
    /// One sentence for the journal when something is not right. **Never the
    /// reading itself**: the host's log is not where a series of somebody's
    /// readings accumulates, and an addon that puts a value here has put it in
    /// the journal.
    #[serde(default)]
    pub complaint: String,
    /// **Further measurements the same source took at the same moment**, each
    /// named, and empty for every source that measures one thing.
    ///
    /// [`Polled::reading`] is the role's own measurement — what the card is
    /// *for* — and it stays alone, because a host that had to search a list
    /// for the number it draws would be doing the work the class already did.
    /// This is for the readings that come with it: a room's sensor reports
    /// temperature **and** humidity from one poll, and Aura's weather panel
    /// draws both.
    ///
    /// **What an id means is the ROLE's business, not this protocol's.** The
    /// class already told the host what kind of source it opened, so the host
    /// knows that an `aura-indoor-source` answers `humidity` and what to do
    /// with it. Nothing here defines a vocabulary of ids, exactly as nothing
    /// here defines what a setting key means.
    ///
    /// Added 2026-09-20 for TempMon's indoor source. **No version bump**, on
    /// [`crate::abi::VERSION`]'s own rule: this is a new optional field, an
    /// older host ignores it rather than refusing, and
    /// `a_field_this_build_does_not_know_is_ignored_rather_than_refused`
    /// is the test that says so.
    #[serde(default)]
    pub also: Vec<Measured>,
}

/// **One further measurement, named** — see [`Polled::also`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measured {
    /// What it measures, in whatever words the role's two ends agreed on.
    ///
    /// Free text for the same reason `ADR-053` leaves a class free text: the
    /// addon author and the host author already coordinate on every other byte
    /// of the join, and a registry here would be a second contract to keep in
    /// step with the first.
    pub id: String,
    /// The measurement itself, carrying its own unit and instant — a reading
    /// that came with another is not a lesser one, and staleness is per
    /// sensor.
    pub reading: Reading,
}

/// One measurement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    pub value: f64,
    /// The unit the value is in, as the addon measured it — the host converts
    /// if it wants a different one.
    pub unit: String,
    /// When the SENSOR took it, not when the addon was asked. The difference
    /// is the whole reason a card can say *stale*.
    pub taken_at_ms: i64,
    #[serde(default)]
    pub trend: Option<Trend>,
    /// **The addon's own verdict that this value is outside the band its
    /// settings define**, so a host can mark it without knowing what the band
    /// is or what the reading means.
    ///
    /// Orthogonal to [`Status`], which says how much the number can be
    /// believed rather than what it says: a reading can be stale and in range,
    /// or fresh and far out of it.
    ///
    /// **Added when the first networked source moved out of its host** — the
    /// thresholds are the addon's (R-198 D2 keeps the policy with the addon),
    /// and without this field the verdict had nowhere to go but back into the
    /// host, which is the coupling the move exists to remove.
    #[serde(default)]
    pub attention: bool,
}

/// Which way the last few readings went, when the source says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Trend {
    RisingQuickly,
    Rising,
    Flat,
    Falling,
    FallingQuickly,
}

/// How much the reading beside it can be believed.
///
/// **Four, and the fourth is the one the first networked source added.**
/// *Nothing yet* and *this will not fix itself* look identical to a host and
/// are completely different to a person: one is a sensor that has not
/// reported, the other is a rejected credential that a human has to go and
/// repair. Aura has drawn
/// them differently since before it was ported, and a protocol that collapsed
/// them would have made the card worse than the program it replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Fresh, and the addon stands behind it.
    Ok,
    /// Real but old enough to say so. The addon decides the threshold,
    /// because the addon knows how often its sensor publishes.
    Stale,
    /// Nothing to show yet, and no reason to think anything is wrong —
    /// a source that has not reported since the host started.
    Unavailable,
    /// **The source has refused in a way that will not resolve on its own**:
    /// a rejected credential, an unaccepted terms-of-use update, an account
    /// following nobody. A host shows this at once rather than waiting for a
    /// held reading to age out, because there is no reading coming.
    Faulted,
}

/// The addon could not answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Failure {
    /// A short kind, for a host that wants to branch: `panicked`,
    /// `not-configured`, `unsupported-request`.
    pub kind: String,
    /// One sentence for a person.
    pub sentence: String,
}

/// **The versioned envelope every message crosses inside.**
///
/// The version is on the wire as well as behind [`crate::abi::VERSION`]'s own
/// symbol, and the duplication is deliberate: the symbol is what a host reads
/// before it trusts a single byte, and this is what makes a captured message
/// self-describing afterwards — in a journal line, in a test fixture, in a
/// bug report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope<T> {
    pub v: u32,
    #[serde(flatten)]
    pub body: T,
}

impl<T> Envelope<T> {
    pub fn new(body: T) -> Self {
        Self {
            v: crate::abi::VERSION,
            body,
        }
    }
}

/// Encode a request or an answer for the crossing.
///
/// # Errors
/// When the value cannot be serialized, which for these types means a `f64`
/// that is not a number.
pub fn encode<T: Serialize>(body: T) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&Envelope::new(body))
}

/// Read one back.
///
/// **Unknown fields are ignored**, which is what makes a newer addon's answer
/// readable by an older host. The version is checked by the caller rather
/// than here, so that a host can report *what* it could not read.
///
/// # Errors
/// When the bytes are not this message.
pub fn decode<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<Envelope<T>, serde_json::Error> {
    serde_json::from_slice(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_poll_survives_the_crossing_unchanged() {
        let answer = Response::Polled(Polled {
            reading: Some(Reading {
                value: 4.2,
                unit: "ft".to_string(),
                taken_at_ms: 1_789_529_336_720,
                trend: Some(Trend::Flat),
                attention: false,
            }),
            status: Status::Ok,
            call_again_in_ms: 300_000,
            complaint: String::new(),
            also: Vec::new(),
        });
        let bytes = encode(answer.clone()).expect("encodes");
        let back: Envelope<Response> = decode(&bytes).expect("decodes");
        assert_eq!(back.v, crate::abi::VERSION);
        assert_eq!(back.body, answer);
    }

    /// **A source that measures two things answers with both in one poll.**
    ///
    /// TempMon's indoor addon reports a room's temperature and its humidity
    /// from one sensor read, and Aura draws both. Each carries its OWN unit
    /// and its own instant, which is the point of `Measured` holding a whole
    /// `Reading` rather than a bare number: two sensors in one device can go
    /// stale independently and a card that showed one as fresh because the
    /// other was would be lying.
    #[test]
    fn further_measurements_cross_with_their_own_units_and_instants() {
        let answer = Response::Polled(Polled {
            reading: Some(Reading {
                value: 71.0,
                unit: "°F".to_string(),
                taken_at_ms: 1_789_529_336_720,
                trend: None,
                attention: false,
            }),
            status: Status::Ok,
            call_again_in_ms: 60_000,
            complaint: String::new(),
            also: vec![Measured {
                id: "humidity".to_string(),
                reading: Reading {
                    value: 59.0,
                    unit: "%".to_string(),
                    taken_at_ms: 1_789_529_336_700,
                    trend: None,
                    attention: false,
                },
            }],
        });
        let bytes = encode(answer.clone()).expect("encodes");
        let back: Envelope<Response> = decode(&bytes).expect("decodes");
        assert_eq!(back.body, answer);
    }

    /// **An addon built before `also` existed still answers this one.**
    ///
    /// The other half of [`crate::abi::VERSION`]'s rule, and the reason that
    /// field needed no version bump: the objects already installed on boards
    /// send no `also` at all, and their absence has to read as *measured one
    /// thing* rather than as a malformed answer.
    #[test]
    fn a_poll_from_before_also_existed_decodes_with_none_of_it() {
        let older = br#"{"v":1,"answer":"polled","status":"ok","call_again_in_ms":1000}"#;
        let back: Envelope<Response> =
            decode(older).expect("an absent `also` is not an error");
        match back.body {
            Response::Polled(polled) => assert!(polled.also.is_empty()),
            other => panic!("expected a poll, got {other:?}"),
        }
    }

    /// The property the whole version story rests on: a host built today can
    /// read an answer from an addon that learned a new field tomorrow.
    #[test]
    fn a_field_this_build_does_not_know_is_ignored_rather_than_refused() {
        let newer = br#"{"v":1,"answer":"polled","status":"ok","call_again_in_ms":1000,
            "confidence":"high","reading":{"value":5.0,"unit":"ft",
            "taken_at_ms":1,"origin":"gauge"}}"#;
        let back: Envelope<Response> = decode(newer).expect("an unknown field is not an error");
        match back.body {
            Response::Polled(p) => {
                assert_eq!(p.status, Status::Ok);
                assert_eq!(p.reading.expect("a reading").value, 5.0);
            }
            other => panic!("read as {other:?}"),
        }
    }

    #[test]
    fn a_secret_field_carries_a_name_and_never_a_value() {
        let f = Field {
            key: "account".to_string(),
            label: "Gauge service account".to_string(),
            help: String::new(),
            kind: FieldKind::Secret {
                name: "gauge.env".to_string(),
            },
        };
        let text = serde_json::to_string(&f).expect("encodes");
        assert!(text.contains("gauge.env"), "names the secret");
        assert!(
            !text.contains("password") && !text.contains("value"),
            "a Secret field has nowhere to put one: {text}"
        );
    }

    #[test]
    fn describe_round_trips_with_every_field_kind() {
        let d = Description {
            name: "river-gauge".to_string(),
            version: "0.1.0".to_string(),
            class: "aura-reading-source".to_string(),
            material: vec!["gauge.env".to_string()],
            backgrounds: Vec::new(),
            fields: vec![
                Field {
                    key: "enabled".to_string(),
                    label: "Show the card".to_string(),
                    help: String::new(),
                    kind: FieldKind::Bool { default: true },
                },
                Field {
                    key: "low".to_string(),
                    label: "Low".to_string(),
                    help: "Below this reads low.".to_string(),
                    kind: FieldKind::Number {
                        default: 2.0,
                        min: 0.0,
                        max: 20.0,
                        unit: "ft".to_string(),
                    },
                },
                Field {
                    key: "unit".to_string(),
                    label: "Unit".to_string(),
                    help: String::new(),
                    kind: FieldKind::Choice {
                        default: "ft".to_string(),
                        options: vec![
                            ("ft".to_string(), "feet".to_string()),
                            ("m".to_string(), "meters".to_string()),
                        ],
                    },
                },
            ],
        };
        let bytes = encode(Response::Described(d.clone())).expect("encodes");
        let back: Envelope<Response> = decode(&bytes).expect("decodes");
        assert_eq!(back.body, Response::Described(d));
    }

    /// **A pack describes several backgrounds, each with its own themes** —
    /// the shape a host builds two pickers from, and the one `.rulings/`
    /// R-199 D1's multipack needs.
    #[test]
    fn a_pack_describes_a_catalogue_and_it_survives_the_crossing() {
        let theme = |id: &str, label: &str| Theme {
            id: id.to_string(),
            label: label.to_string(),
            cards: Cards::Unstated,
            animated: false,
        };
        let d = Description {
            name: "aura-background-pack".to_string(),
            version: "0.1.0".to_string(),
            class: "aura-background".to_string(),
            fields: Vec::new(),
            material: Vec::new(),
            backgrounds: vec![
                Background {
                    id: "digital-rain".to_string(),
                    name: "Digital Rain".to_string(),
                    themes: vec![theme("ice-blue", "Ice Blue"), theme("phosphor", "Phosphor")],
                    prefers_dark_cards: false,
                    drawn: Drawn::Cpu,
                },
                Background {
                    id: "solid".to_string(),
                    name: "Solid".to_string(),
                    themes: vec![theme("slate", "Slate")],
                    prefers_dark_cards: true,
                    drawn: Drawn::Either,
                },
            ],
        };
        let bytes = encode(Response::Described(d.clone())).expect("encodes");
        let back: Envelope<Response> = decode(&bytes).expect("decodes");
        assert_eq!(back.body, Response::Described(d));
    }

    /// **A theme's card appearance and its animation survive the crossing** —
    /// the two fields added 2026-09-20 so that `aura-renderer` could stop
    /// holding a ten-way card table the pack cannot reach.
    ///
    /// Asserted on the whole value rather than field by field: the point of
    /// carrying six colour components instead of a bool is that all six come
    /// back, and a test that checked `bg` alone would pass for a build that
    /// dropped `caution`.
    #[test]
    fn a_themes_card_appearance_survives_the_crossing() {
        let theme = Theme {
            id: "ice-blue".to_string(),
            label: "Ice Blue".to_string(),
            cards: Cards::Card(CardAppearance {
                bg: [3, 5, 14],
                bg_alpha: 155,
                text: [190, 205, 247],
                rainbow: false,
                alert: [255, 69, 58],
                caution: [255, 197, 0],
            }),
            animated: true,
        };
        let d = Description {
            name: "aura-background-pack".to_string(),
            version: "0.1.0".to_string(),
            class: "aura-background-pack".to_string(),
            fields: Vec::new(),
            material: Vec::new(),
            backgrounds: vec![Background {
                id: "digital-rain".to_string(),
                name: "Digital Rain".to_string(),
                themes: vec![theme],
                prefers_dark_cards: false,
                drawn: Drawn::Cpu,
            }],
        };
        let bytes = encode(Response::Described(d.clone())).expect("encodes");
        let back: Envelope<Response> = decode(&bytes).expect("decodes");
        assert_eq!(back.body, Response::Described(d));
    }

    /// **A pack built before those two fields existed still decodes**, and
    /// decodes to the two defaults this protocol chose rather than to an
    /// error — `Theme::cards`'s and `Theme::animated`'s own notes on why
    /// those are the defaults.
    ///
    /// This is the same property `Polled::also` has and the reason neither
    /// addition bumped `crate::abi::VERSION`. It is asserted from a JSON
    /// literal and not from a struct, because a struct written in this build
    /// cannot omit a field this build knows about — the older pack's absence
    /// is exactly what has to be reproduced.
    #[test]
    fn a_pack_that_predates_the_card_fields_still_describes_itself() {
        let json = r#"{
            "name": "old-pack",
            "version": "0.1.0",
            "class": "aura-background-pack",
            "backgrounds": [
                {
                    "id": "solid",
                    "name": "Solid",
                    "themes": [{ "id": "black", "label": "Black" }],
                    "prefers_dark_cards": false
                }
            ]
        }"#;
        let d: Description = serde_json::from_str(json).expect("an older pack still decodes");
        let theme = &d.backgrounds[0].themes[0];
        assert_eq!(
            theme.cards,
            Cards::Unstated,
            "no opinion, so the host falls back to prefers_dark_cards"
        );
        assert!(
            !theme.animated,
            "a forgotten field freezes the picture rather than burning the night's power"
        );
    }

    /// **An explicit `null` and an omitted `cards` are two different answers**
    /// — the omitted one is *no opinion* and the null is *draw no card here*.
    /// Asserted from bytes a pack sends, because the distinction exists only
    /// on the wire: a struct written in this build cannot omit a field.
    #[test]
    fn an_explicit_null_card_is_not_the_same_answer_as_an_omitted_one() {
        let null: Theme =
            serde_json::from_slice(br#"{"id":"black","label":"Black","cards":null}"#)
                .expect("an explicit null decodes");
        let omitted: Theme = serde_json::from_slice(br#"{"id":"black","label":"Black"}"#)
            .expect("an omitted field decodes");
        assert_ne!(
            null.cards, omitted.cards,
            "a pack asking for no card reads the same as a pack saying nothing"
        );
    }

    /// **Each of the three spellings decodes to its own state**, from the
    /// bytes of a whole `Describe` answer as a pack sends it.
    #[test]
    fn the_three_card_spellings_decode_to_three_states() {
        let bytes = br#"{"v":1,"answer":"described","name":"pack","version":"0.1.0",
            "class":"aura-background","backgrounds":[{"id":"aurora","name":"Aurora",
            "prefers_dark_cards":true,"themes":[
              {"id":"silent","label":"Silent"},
              {"id":"bare","label":"Bare","cards":null},
              {"id":"ember","label":"Ember","cards":{"bg":[10,5,3],"bg_alpha":235,
                "text":[250,225,205],"rainbow":false,"alert":[255,69,58],
                "caution":[255,197,0]}}]}]}"#;
        let back: Envelope<Response> = decode(bytes).expect("decodes");
        let Response::Described(d) = back.body else {
            panic!("not a description");
        };
        let themes = &d.backgrounds[0].themes;
        assert_eq!(themes[0].cards, Cards::Unstated, "an omitted key is no opinion");
        assert_eq!(themes[1].cards, Cards::NoCard, "null asks for no card");
        assert_eq!(
            themes[2].cards,
            Cards::Card(CardAppearance {
                bg: [10, 5, 3],
                bg_alpha: 235,
                text: [250, 225, 205],
                rainbow: false,
                alert: [255, 69, 58],
                caution: [255, 197, 0],
            })
        );
    }

    /// **And each state is written as its own spelling**: no opinion omits
    /// the key, no card writes `null`, a card writes the object. Asserted on
    /// the encoded text, because an omitted key and a `null` round-trip to
    /// different values only if the writer keeps them apart.
    #[test]
    fn the_three_card_states_encode_to_three_spellings() {
        let theme = |cards: Cards| Theme {
            id: "t".to_string(),
            label: "T".to_string(),
            cards,
            animated: false,
        };
        let unstated = serde_json::to_string(&theme(Cards::Unstated)).expect("encodes");
        assert!(!unstated.contains("cards"), "no opinion omits the key: {unstated}");
        let none = serde_json::to_string(&theme(Cards::NoCard)).expect("encodes");
        assert!(none.contains(r#""cards":null"#), "no card is null: {none}");
        let card = CardAppearance {
            bg: [0, 0, 0],
            bg_alpha: 0,
            text: [0, 0, 0],
            rainbow: true,
            alert: [255, 69, 58],
            caution: [255, 197, 0],
        };
        let some = serde_json::to_string(&theme(Cards::Card(card))).expect("encodes");
        assert!(some.contains(r#""cards":{"bg":[0,0,0]"#), "a card is the object: {some}");
        for cards in [Cards::Unstated, Cards::NoCard, Cards::Card(card)] {
            let text = serde_json::to_string(&theme(cards)).expect("encodes");
            let back: Theme = serde_json::from_str(&text).expect("decodes");
            assert_eq!(back.cards, cards, "{text}");
        }
    }

    /// **No opinion has no spelling of its own outside a theme**, so writing
    /// it alone is refused rather than written as the `null` that means
    /// something else.
    #[test]
    fn an_unstated_card_alone_refuses_to_serialize_rather_than_writing_null() {
        assert!(serde_json::to_string(&Cards::Unstated).is_err());
        assert_eq!(serde_json::to_string(&Cards::NoCard).expect("encodes"), "null");
    }

    /// **Every id in a catalogue has to fit the frame descriptor**, which is
    /// the one coupling between the JSON half and the C half. A pack whose id
    /// is too long or outside the alphabet would describe itself perfectly and
    /// then be undrawable, and the two facts would be discovered a long way
    /// apart.
    #[test]
    fn a_catalogue_id_is_one_the_frame_descriptor_can_carry() {
        assert!(crate::frame::id_bytes("digital-rain").is_some());
        assert!(crate::frame::id_bytes("ice-blue").is_some());
        assert!(
            crate::frame::id_bytes("Digital Rain").is_none(),
            "a display name is not an id"
        );
    }

    /// A host built before packs existed reads a pack's description and finds
    /// a reading source with no settings, rather than refusing it — the same
    /// property `a_field_this_build_does_not_know_is_ignored_rather_than_refused`
    /// proves for a poll, restated for the catalogue because this is the field
    /// most likely to arrive at an old host.
    #[test]
    fn a_host_that_has_no_catalogue_in_its_vocabulary_reads_the_rest() {
        let newer = br#"{"v":1,"answer":"described","name":"pack","version":"0.1.0",
            "class":"aura-background","backgrounds":[{"id":"solid","name":"Solid",
            "themes":[{"id":"slate","label":"Slate"}],"prefers_dark_cards":true,
            "gamma":2.2}]}"#;
        let back: Envelope<Response> = decode(newer).expect("decodes");
        match back.body {
            Response::Described(d) => {
                assert_eq!(d.backgrounds.len(), 1);
                assert_eq!(d.backgrounds[0].themes[0].label, "Slate");
            }
            other => panic!("read as {other:?}"),
        }
    }

    /// **A pack built before R-206 says nothing, and nothing means CPU.**
    ///
    /// This is the compatibility property the whole field rests on: every
    /// pack that predates it draws with the CPU, so the absent case and the
    /// [`Drawn::Cpu`] case have to be the same case. A default of `Gpu` or a
    /// required field would make every existing pack either unreadable or
    /// unusable, and the failure would appear as a host with no backgrounds
    /// rather than as a parse error anyone could follow.
    #[test]
    fn a_background_that_says_nothing_about_where_it_draws_draws_on_the_cpu() {
        let older = br#"{"v":1,"answer":"described","name":"pack","version":"0.1.0",
            "class":"aura-background","backgrounds":[{"id":"digital-rain","name":"Rain",
            "themes":[{"id":"ice-blue","label":"Ice Blue"}]}]}"#;
        let back: Envelope<Response> = decode(older).expect("decodes");
        let Response::Described(d) = back.body else {
            panic!("not a description");
        };
        assert_eq!(d.backgrounds[0].drawn, Drawn::Cpu);
        assert!(d.backgrounds[0].drawn.on_cpu(), "a silent pack must be askable");
        assert!(!d.backgrounds[0].drawn.on_gpu(), "a silent pack must not be asked for GL");
    }

    /// The three spellings, over the wire, in the kebab-case the rest of this
    /// vocabulary uses.
    #[test]
    fn where_a_background_draws_crosses_as_a_word_and_not_a_pair_of_flags() {
        for (word, expected, cpu, gpu) in [
            ("cpu", Drawn::Cpu, true, false),
            ("gpu", Drawn::Gpu, false, true),
            ("either", Drawn::Either, true, true),
        ] {
            let json = format!(
                r#"{{"v":1,"answer":"described","name":"p","version":"0","class":"c",
                "backgrounds":[{{"id":"x","name":"X","themes":[],"drawn":"{word}"}}]}}"#
            );
            let back: Envelope<Response> = decode(json.as_bytes()).expect("decodes");
            let Response::Described(d) = back.body else {
                panic!("not a description");
            };
            assert_eq!(d.backgrounds[0].drawn, expected, "{word}");
            assert_eq!(d.backgrounds[0].drawn.on_cpu(), cpu, "{word} on_cpu");
            assert_eq!(d.backgrounds[0].drawn.on_gpu(), gpu, "{word} on_gpu");
        }
    }
}
