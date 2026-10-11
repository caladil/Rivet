//! **The reading sources, loaded rather than compiled in** — `ADR-053`,
//! `rivetos_addon`, and `.rulings/` R-196 through R-198.
//!
//! A source's client, its credential parser and its card policy live in the
//! addon's own repository, not here. The reason is not architectural tidiness:
//! **Rivet is public, and a relay credential's client is not a thing that goes
//! with it** (R-197 D1). The card on the wall does not depend on where that
//! code lives.
//!
//! What is left here is the host's half, and it is deliberately small:
//!
//! * the **trial note**, so an addon that took this process down is skipped
//!   and named on the next start rather than taken down with it again;
//! * **opening by class** — a name, never a path;
//! * **`Describe`**, whose answer a host may cache for the life of the load,
//!   because an addon cannot be reloaded without restarting the host that
//!   opened it;
//! * **`Configure`**, with this program's settings and the CONTENTS of every
//!   secret the addon asked for by name;
//! * **`Poll`**, and the map from what comes back to Aura's own wire.
//!
//! # The host knows nothing of what the reading card shows
//!
//! That is the test of whether the move worked, and `.rulings/` R-375 D1 is
//! the ruling that set it. This file names classes and the indoor role's three
//! keys, and nothing about the corner card's measurement at all: no unit, no
//! threshold, no account field and no word for the quantity. The relay, its
//! refusals, the staleness rule, the band and the units are all the addon's,
//! so any source of *a value, a unit and an instant* — a river gauge, a bus
//! stop's next departure, a meter read over Bluetooth — drops in here with no
//! change at all.
//!
//! **The addon's settings are stored in the addon's shape.** `Describe` lists
//! the fields; the portal renders and validates them generically
//! (`crate::portal::schema`); the validated map is what `crate::config` keeps,
//! keyed exactly as the addon spelled its keys; and [`settings_for`] hands the
//! same map back in `Configure`, filling any key it lacks from the field's own
//! declared default. Nothing in between reads a value.
//!
//! **The class is what puts a source on the reading card rather than in the
//! weather panel.** It names a ROLE, one class per role (R-200). The corner
//! card's source and the room's temperature are both *a value, a unit, an
//! instant*, so a class named after that shape would have left this program
//! unable to tell which addon belongs on the corner card and which is a column
//! of the weather panel. `ADR-053`'s class is already the joining contract
//! between two signed manifests, so the role rides on the thing that does the
//! joining rather than on a second contract inside it — and the role is named
//! for the card it fills, never for what the card shows.
//!
//! # The value never reaches the journal
//!
//! The addon keeps that rule for what it measures — a `complaint` says what is
//! wrong and never what was read — and this module keeps it for everything it
//! writes: the controller's log is not the place a series of readings
//! accumulates, and nor is a setting's value, which can name an account.

use std::collections::BTreeMap;

use rivetos_addon::host::{CallFailure, Loaded, OpenFailure};
use rivetos_addon::{
    Description, Field, FieldKind, Polled, Reading, Request, Response, Status, Trend,
};

use crate::wire;

/// **The roles this program fills from an addon**, and the same strings
/// `Cargo.toml`'s `accepts-class` declares.
///
/// None of these can drift from the manifest: the supervisor matches on the
/// manifest's spelling and [`Source::open`] asks the loader for the object
/// named after the string it is given, so a mismatch is an addon that is
/// installed, matched, placed on the library path, and never opened.
///
/// # Why a role is named here and the addon is not
///
/// [`Source`] takes a class rather than holding one, so nothing in the loading
/// machinery knows which addon it is loading. What this program does have an
/// opinion about is the ROLE — it draws a corner reading card and it draws a
/// temperature column, and those are different pictures. `ADR-053`'s class is
/// the joining contract between two signed manifests, and R-200 put the role
/// on it precisely so a host can tell the two apart: the corner card's value
/// and a room's temperature are both *a value, a unit, an instant*, and a
/// class named after that shape would leave this program unable to say which
/// belongs where.
///
/// So the host knows the roles it fills and knows nothing about who fills
/// them — not the relay, not the sensor, not the credential, not the
/// thresholds, not the quantity. Any source drops into [`role::READING`] with
/// no change here, which is the test of whether this stayed abstract.
pub mod role {
    /// The corner reading card's source — one value, read from across the
    /// room, with its unit, its trend and how far it can be believed.
    ///
    /// **Named for the card and not for what it shows** (`.rulings/` R-375
    /// D1): what the value measures is the addon's to know, and the host
    /// neither needs it nor holds it.
    pub const READING: &str = "aura-reading-source";
    /// The indoor temperature shown beside the outdoor one on the weather
    /// panel — `TempMon`'s addon, published from that project's repository
    /// (Rivet `.rulings/` R-197 D2).
    pub const INDOOR: &str = "aura-indoor-source";
    /// **The backgrounds the renderer draws**, which this program opens only
    /// to read the catalogue for its two pickers — T7.
    ///
    /// The odd one out, and worth saying so: the other two roles name a source
    /// of readings this program polls. This one names a pack this program
    /// never calls again after `Describe`, because `aura-renderer` is the host
    /// that draws it. Two hosts, one object, neither knowing about the other —
    /// `docs/design-candidates/Background-Addons-2026-09-19.md` §2.
    pub const BACKGROUND: &str = "aura-background";
}

/// **A field's own declared default, as the value `Configure` would carry.**
///
/// Read off the [`FieldKind`] and nothing else, which is what keeps the
/// defaults the addon's: the host holds none of its own for any addon field.
/// `crate::portal::schema` pre-fills a form from the same function, so a fresh
/// install's page shows exactly what the first `Configure` sends.
///
/// A [`FieldKind::Secret`] has no value to default — its contents travel as
/// material, never as a setting — so it answers `Null`, and [`settings_for`]
/// leaves it out altogether.
#[must_use]
pub(crate) fn field_default(kind: &FieldKind) -> serde_json::Value {
    match kind {
        FieldKind::Bool { default } => serde_json::Value::Bool(*default),
        FieldKind::Number { default, .. } => serde_json::json!(default),
        FieldKind::Choice { default, .. } | FieldKind::Text { default, .. } => {
            serde_json::Value::String(default.clone())
        }
        FieldKind::Secret { .. } => serde_json::Value::Null,
    }
}

/// **The settings this host sends, from what a person saved.**
///
/// `saved` is the map `crate::config::Config` keeps for the role, keyed the way
/// the addon spelled its keys. Every field `Describe` declared is sent: the
/// saved value when there is one, the field's own default ([`field_default`])
/// when there is not — a fresh install, or a newer addon that grew a field —
/// so the addon is never handed a hole it has to guess about.
///
/// **Only declared fields cross.** A key `saved` carries that the addon no
/// longer declares is left behind rather than sent, because the protocol says
/// an undeclared key is ignored and sending it would only make the map look
/// meaningful. A [`FieldKind::Secret`] never crosses as a setting: it is
/// material, resolved by name in [`material`].
///
/// **`enabled` is not among them, and that is a decision rather than an
/// omission** (R-198 D2). Whether a source is drawn at all is the host's
/// judgement about its own screen, not a fact about the source — so the addon
/// is never told, and `crate::config`'s `reading_enabled` is acted on by not
/// showing a card.
///
/// No value is read here, only copied: what a key means is the addon's
/// business.
#[must_use]
pub fn settings_for(
    fields: &[Field],
    saved: &serde_json::Map<String, serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut settings = serde_json::Map::new();
    for field in fields {
        if matches!(field.kind, FieldKind::Secret { .. }) {
            continue;
        }
        let value = saved
            .get(&field.key)
            .cloned()
            .unwrap_or_else(|| field_default(&field.kind));
        settings.insert(field.key.clone(), value);
    }
    settings
}

/// **Store what the portal validated, as the role's saved settings.**
///
/// `parsed` is `crate::portal::schema::validate_fields`'s answer: every
/// non-secret field `Describe` declared, each already checked against the
/// bound the addon itself stated. It REPLACES what was saved rather than being
/// merged into it, so a key the addon stopped declaring is dropped at the next
/// save instead of being carried for ever.
///
/// Returns whether anything actually changed, which is what a caller uses to
/// decide whether the addon needs a fresh `Configure` at all.
#[must_use]
pub fn apply_settings(
    saved: &mut serde_json::Map<String, serde_json::Value>,
    parsed: serde_json::Map<String, serde_json::Value>,
) -> bool {
    if *saved == parsed {
        return false;
    }
    *saved = parsed;
    true
}

/// **How often the host asks, and it is not how often the addon fetches.**
///
/// [`Request::Poll`] asks *what do you say right now*, not *go and fetch*: an
/// addon does I/O when its own schedule says so and answers from what it
/// already holds otherwise. So a host with a clock on a wall may ask as often
/// as it redraws, and this one redraws every second.
///
/// That is worth one second of a thread because **how stale a reading is
/// changes every second without anything new arriving**, and the staleness
/// rule belongs to the addon. Asking once every five minutes and holding the
/// answer would put the old design back — a status frozen at fetch time,
/// looking fresh long after it stopped being so — with an extra process
/// boundary in the middle of it.
///
/// [`Polled::call_again_in_ms`] is therefore read as the ceiling it is
/// documented to be, and this host is always inside it.
pub const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// An addon that is open, agreed and has described itself.
pub struct Source {
    /// The role it was opened for — one of [`role`]'s strings.
    ///
    /// Held rather than assumed, because this type loads whatever class it is
    /// handed and a program with two of them open needs each to be able to say
    /// which it is when it appears in a log line.
    class: &'static str,
    loaded: Loaded,
    description: Description,
}

/// Why there is no reading source.
///
/// Two variants and not a string, because the two are journalled at different
/// levels and a person reading a board's log should not have to tell them
/// apart by wording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotOpened {
    /// **No addon of this class is installed**, which is the ordinary state of
    /// a machine nobody has given one to. `ADR-053` joins by class and an
    /// absent class is silent, so this is information rather than a fault:
    /// whatever that role would have drawn is simply not drawn.
    Absent(String),
    /// Something was there and was not used: a mislabelled build, a protocol
    /// this program cannot speak, or a trial note saying the last attempt to
    /// load it did not survive.
    Refused(String),
}

impl std::fmt::Display for NotOpened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Absent(s) | Self::Refused(s) => f.write_str(s),
        }
    }
}

/// **What identifies the object [`Source::open`] is about to load**, for
/// `rivetos_addon::trial::left_behind` to compare a stale note against.
///
/// Reads no file and opens nothing: `joined` is whatever
/// `rivetos_addon::host::installed(class)` already answered, from the
/// environment the supervisor handed over, before `Loaded::open` is ever
/// called. That ordering matters — this has to be knowable before the note
/// is written, and the note is written before the object is opened.
///
/// **Exactly one match** is the ordinary case and gets that object's own
/// [`rivetos_addon::host::Installed::identity`] — the reference directory
/// under `/state/lib`, which changes on every reinstall or upgrade. **Zero or
/// several matches** fall back to the class's derived soname: `Loaded::open`
/// is about to refuse either of those for its own reasons (`NotLoaded` or
/// `Ambiguous`) and clear the note right back off on the way out, so nothing
/// downstream reads this identity in that case — it only has to be *some*
/// stable string, not a precise one.
fn identity_for(class: &str, joined: &[rivetos_addon::host::Installed]) -> String {
    match joined {
        [only] => only.identity(),
        _ => rivetos_addon::abi::soname_for_class(class),
    }
}

impl Source {
    /// **Write the trial note, open the addon, describe it, clear the note.**
    ///
    /// The order is the mechanism and not a preference. `rivetos_addon::trial`
    /// is `ADR-018`'s trial boot one level down: a loaded addon shares
    /// this process's address space, so one that writes through a wild pointer
    /// takes the display's controller with it and nothing at the boundary can
    /// prevent that. What the note prevents is the SECOND time — the crash
    /// loop — and it works because **withholding the confirmation is the
    /// rollback**, so nothing has to run on the way down for it to take
    /// effect.
    ///
    /// The note is cleared after the addon has ANSWERED, not after the
    /// `dlopen`. An object that loads and then faults on its first call has
    /// not settled, and settling it there would retire the whole mechanism for
    /// exactly the addon it exists to catch.
    ///
    /// # Errors
    ///
    /// [`NotOpened`], whose two variants are the two things to say about it.
    pub fn open(class: &'static str) -> Result<Self, NotOpened> {
        let data_dir = crate::config::data_dir().map_err(|e| {
            NotOpened::Refused(format!(
                "addon: no data directory, so the trial note has nowhere to go and no addon \
                 will be loaded: {e}"
            ))
        })?;

        // **What the note is compared against, computed before anything is
        // touched.** `identity_for` reads only the environment the supervisor
        // handed over (`rivetos_addon::host::installed`) — no `dlopen`, so
        // this is safe to call whether or not a trial note ends up mattering.
        let identity = identity_for(class, &rivetos_addon::host::installed(class));

        match rivetos_addon::trial::left_behind(&data_dir, class, &identity) {
            rivetos_addon::trial::LeftBehind::Clear => {}
            rivetos_addon::trial::LeftBehind::Same => {
                return Err(NotOpened::Refused(format!(
                    "addon: SKIPPED the {class} addon was being loaded when this program last \
                     stopped, so it is not loaded now and nothing will fill that role. \
                     Reinstalling the addon clears that."
                )));
            }
            rivetos_addon::trial::LeftBehind::Unidentified => {
                // **A note from before this build recorded identity.**
                // Skipped exactly as it always was — see `rivet_addon::trial`
                // for why guessing either "same" or "different" would be
                // worse — and the recovery named is the one that is actually
                // true for THIS note: a reinstall changes what gets loaded
                // but not what is already written down, so only removing the
                // file clears it.
                return Err(NotOpened::Refused(format!(
                    "addon: SKIPPED the {class} addon was being loaded when this program last \
                     stopped, so it is not loaded now and nothing will fill that role. This \
                     note predates identity tracking, so a reinstall cannot be told apart from \
                     the crash that wrote it; delete {} to clear it.",
                    rivetos_addon::trial::note_path(&data_dir, class).display()
                )));
            }
            rivetos_addon::trial::LeftBehind::Different => {
                // **The escape hatch, automatic.** The note names an object
                // that is not the one about to be loaded — an upgrade or a
                // reinstall happened since the crash — so what crashed is
                // gone and there is nothing left to protect against.
                if let Err(e) = rivetos_addon::trial::forgive(&data_dir, class) {
                    return Err(NotOpened::Refused(format!(
                        "addon: REFUSED a stale trial note for {class} names an object other \
                         than what is installed now, and could not be cleared to load the new \
                         one: {e}"
                    )));
                }
            }
        }

        rivetos_addon::trial::begin(&data_dir, class, &identity).map_err(|e| {
            // **A host that cannot write the note must not load the addon.**
            // An unrecorded load is one that cannot be rolled back, which is
            // the whole mechanism gone — and a silent crash loop on a wall
            // display is precisely the failure it buys protection from.
            NotOpened::Refused(format!(
                "addon: REFUSED to load the {class} addon without recording it first: {e}"
            ))
        })?;

        let loaded = match Loaded::open(class) {
            Ok(loaded) => loaded,
            Err(e @ OpenFailure::NotLoaded { .. }) => {
                let _ = rivetos_addon::trial::settled(&data_dir, class);
                return Err(NotOpened::Absent(format!("addon: {e}")));
            }
            Err(e) => {
                let _ = rivetos_addon::trial::settled(&data_dir, class);
                return Err(NotOpened::Refused(format!("addon: {e}")));
            }
        };

        let described = match loaded.call(&Request::Describe) {
            Ok(Response::Described(d)) => d,
            Ok(Response::Fault(f)) => {
                let _ = rivetos_addon::trial::settled(&data_dir, class);
                return Err(NotOpened::Refused(format!(
                    "addon: {} could not describe itself: {} — {}",
                    loaded.soname(),
                    f.kind,
                    f.sentence
                )));
            }
            Ok(other) => {
                let _ = rivetos_addon::trial::settled(&data_dir, class);
                return Err(NotOpened::Refused(format!(
                    "addon: {} answered Describe with {other:?}",
                    loaded.soname()
                )));
            }
            Err(e) => {
                let _ = rivetos_addon::trial::settled(&data_dir, class);
                return Err(NotOpened::Refused(format!(
                    "addon: {} could not be asked to describe itself: {e}",
                    loaded.soname()
                )));
            }
        };

        // **A mislabelled build is not trusted.** The class is what joined
        // this object to this program, and an object whose own account of
        // itself disagrees with the manifest the supervisor matched on is one
        // of the two that is wrong — which of them cannot be told from here.
        if described.class != class {
            let _ = rivetos_addon::trial::settled(&data_dir, class);
            return Err(NotOpened::Refused(format!(
                "addon: REFUSED {} was opened as {class} and calls itself {}",
                loaded.soname(),
                described.class
            )));
        }

        // It has answered once, so the load has settled: from here a crash is
        // this program's to own rather than a reason to skip this addon next
        // time.
        if let Err(e) = rivetos_addon::trial::settled(&data_dir, class) {
            // Not fatal, and the direction of the failure is the safe one: an
            // uncleared note means the addon is skipped on the next start,
            // which costs a card rather than a crash loop.
            return Err(NotOpened::Refused(format!(
                "addon: {} loaded and answered, but the trial note could not be cleared, so \
                 the next start will skip it: {e}",
                loaded.soname()
            )));
        }

        Ok(Self {
            class,
            loaded,
            description: described,
        })
    }

    /// What the addon calls itself, its version, the role it fills and the
    /// object it came from — one line for the journal.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "{} {} as {} from {} ({} field(s), {} secret(s))",
            self.description.name,
            self.description.version,
            self.class,
            self.loaded.soname(),
            self.description.fields.len(),
            self.description.material.len()
        )
    }

    /// The secrets this addon asked the host to read for it, by name.
    #[must_use]
    pub fn material_names(&self) -> &[String] {
        &self.description.material
    }

    /// **What `Describe` answered**, so a caller outside this module can build
    /// a page from it without learning anything about what the addon measures.
    ///
    /// `crate::portal` reads this once per request to render the addon's own
    /// field set — see `crate::portal::schema` — and never sees `Field` or
    /// `FieldKind` values this module invented; every one of them is the
    /// addon's own account of itself, cached for the life of the load exactly
    /// as [`Request::Describe`]'s doc comment says a host may.
    #[must_use]
    pub fn description(&self) -> &Description {
        &self.description
    }

    /// **Hand the addon its settings and its material.**
    ///
    /// `Ok(())` means usable. `Err` carries the addon's own sentence for a
    /// person — a missing credential, a band with its ends crossed — and
    /// **does not mean unload it**: the addon stays open and the complaint is
    /// shown, because an operator fixing a setting is the expected next event
    /// and unloading is a restart (there is no `dlclose` here, by design).
    ///
    /// # Errors
    ///
    /// The addon's complaint, or a sentence about the call itself.
    pub fn configure(
        &self,
        settings: serde_json::Map<String, serde_json::Value>,
        material: BTreeMap<String, String>,
    ) -> Result<(), String> {
        match self.loaded.call(&Request::Configure { settings, material }) {
            Ok(Response::Configured { usable: true, .. }) => Ok(()),
            Ok(Response::Configured { complaint, .. }) => Err(if complaint.is_empty() {
                "the addon cannot work with the settings it was given and said no more".to_string()
            } else {
                complaint
            }),
            Ok(Response::Fault(f)) => Err(format!("{}: {}", f.kind, f.sentence)),
            Ok(other) => Err(format!("configure answered {other:?}")),
            Err(e) => Err(e.to_string()),
        }
    }

    /// **Ask what it says right now.**
    ///
    /// Blocks until the addon answers, which on the pass where it does reach
    /// the network is a whole HTTP exchange — see this module's caller in
    /// `main.rs` for why that is on a thread of its own.
    ///
    /// # Errors
    ///
    /// A sentence, for a call that produced no answer. A [`Response::Fault`]
    /// is one of these: the addon answered, and what it said was that it could
    /// not do the thing.
    pub fn poll(&self, now_ms: i64) -> Result<Polled, String> {
        match self.loaded.call(&Request::Poll { now_ms }) {
            Ok(Response::Polled(p)) => Ok(p),
            Ok(Response::Fault(f)) => Err(format!("{}: {}", f.kind, f.sentence)),
            Ok(other) => Err(format!("poll answered {other:?}")),
            Err(e @ CallFailure::Refused(_)) => Err(format!("the addon refused the call: {e}")),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// **Read every secret the addon named, and hand back the CONTENTS.**
///
/// The addon never opens a file: it names what it needs through a
/// `FieldKind::Secret` and the host resolves each through its own mechanism —
/// here `rivet_sdk::secrets::get_bytes`, the shared call `ADR-038` §5 and
/// `.rulings/` R-169.1 put every application on, which reads
/// `RIVET_APP_DATA/<name>` and is exactly where
/// `rivet apply install-secret aura-controller <name>` writes.
///
/// **A secret that is not installed is left out rather than sent empty.** The
/// protocol distinguishes the two: an absent key is *nothing has been
/// installed*, which the addon reports as a complaint and recovers from the
/// moment one appears, while an empty string is a credential that names
/// nobody. Sending the second for the first would make a fresh install look
/// like a broken one.
///
/// **A name is the whole of what this side knows.** There is no setting that
/// points one of these files somewhere else, and there was: the config carried
/// a path key for the reading addon's credential until T5.1 took it out. It
/// resolved to the value this function computes anyway, and it was a second
/// authority in the host over a fact `Describe` already supplies, which is what
/// `ADR-011` is about. If an operator ever does need a named secret read from
/// somewhere else, the mechanism is keyed by the name the addon gave — never by
/// a key naming one particular source.
///
/// **Nothing read here is ever formatted into a message.** The bytes go
/// straight into the map and the map goes straight across the boundary; what
/// this function reports is names and lengths and never contents.
#[must_use]
pub fn material(names: &[String]) -> BTreeMap<String, String> {
    let mut material = BTreeMap::new();
    for name in names {
        let Some(bytes) = rivet_sdk::secrets::get_bytes(name) else {
            continue;
        };
        // Not valid UTF-8 is the same answer as not installed: this side has
        // no way to report which without risking the bytes on the way past,
        // and the addon's own complaint covers both.
        if let Ok(text) = String::from_utf8(bytes) {
            material.insert(name.clone(), text);
        }
    }
    material
}

/// **The addon's answer, as the card Aura's renderer draws.**
///
/// This is the whole of what a person sees, so every line of it is pinned by a
/// test in `src/addon/tests.rs`.
///
/// `now_ms` is this host's clock. `last_unit` is what a card with no reading
/// is labelled with, and it is **the unit of the last reading this run saw**,
/// or empty before there has been one — see [`unit_after`].
///
/// # Why a bare card's label is not a setting
///
/// It used to be: the host kept a typed unit setting and stamped a bare card
/// with it. That setting is gone (`.rulings/` R-375 D1 — the host holds no
/// unit of its own for this role), and the only unit the host can still know
/// without being told what the card shows is one the source has already
/// answered in. So a bare card mid-run carries the unit it was last drawn
/// with, and a cold start carries none — the renderer draws only the state's
/// mark in either case, because a unit with no value beside it is not drawn.
#[must_use]
pub fn card(polled: &Polled, now_ms: i64, last_unit: &str) -> wire::ReadingData {
    let status = status_of(polled.status);
    let bare = || wire::ReadingData {
        status,
        unit: last_unit.to_string(),
        ..wire::ReadingData::default()
    };

    // **`error` and `connecting` carry no number, whatever came with them.**
    // `wire::ReadingData` states it as a promise the renderer is written
    // against: a renderer that ignored `status` still could not draw a stale
    // number. An addon answering `Faulted` with a reading attached is not
    // expected, but this is the side that owes the guarantee.
    if status == wire::status::ERROR || status == wire::status::CONNECTING {
        return bare();
    }
    let Some(reading) = polled.reading.as_ref() else {
        return bare();
    };

    wire::ReadingData {
        value: Some(reading.value),
        // The addon's unit, always: it measured and converted, and a label
        // taken from anywhere else would be a second source of truth that is
        // right only while the two agree.
        unit: reading.unit.clone(),
        trend: trend_of(reading.trend),
        // **The addon's own verdict**, which it reached against a band this
        // program never sees. That is R-198 D2: the thresholds are the
        // source's knowledge, and what crosses is the conclusion.
        out_of_range: reading.attention,
        status,
        age_seconds: Some(age_seconds(reading, now_ms)),
    }
}

/// **The label a later bare card takes, once `drawn` has been drawn** — the
/// other half of [`card`]'s `last_unit`.
///
/// A card that carried a value carried the source's own unit, and that is the
/// unit to remember. A bare card teaches nothing — its unit is only the one it
/// was handed — so the remembered unit is kept as it was. Before any reading
/// has arrived this is empty, which is a cold start's honest answer.
#[must_use]
pub fn unit_after(last_unit: &str, drawn: &wire::ReadingData) -> String {
    if drawn.value.is_some() && !drawn.unit.is_empty() {
        drawn.unit.clone()
    } else {
        last_unit.to_string()
    }
}

/// The [`role::INDOOR`] addon's setting keys, as this host spells them.
///
/// Restated here rather than imported: these cross as settings between two
/// repositories, and a wire is a thing both ends state and a test pins, not a
/// thing one end takes from the other. The indoor role keeps typed settings
/// because its source is TempMon, whose URL and certificate name this host
/// itself has to reason about (`crate::config::effective_tempmon_name`); the
/// reading card's source has nothing the host reasons about, so its settings
/// are kept as the addon declared them ([`settings_for`]).
pub mod indoor_key {
    /// Where TempMon answers — the whole URL, scheme included, because the
    /// scheme is what chooses TLS.
    pub const URL: &str = "url";
    /// The name to verify TempMon's certificate against, when it is not the
    /// name in the URL.
    ///
    /// The board's certificate carries one name and TempMon is reached on the
    /// loopback address, so on this machine those genuinely differ. Left
    /// empty, the URL's own host is verified, which is right for a TempMon
    /// reached by a name that resolves.
    pub const VERIFY_AS: &str = "verify_as";
    /// Which sensor's reading to answer with.
    pub const SENSOR: &str = "sensor";
}

/// The id [`role::INDOOR`] answers humidity under, in [`Polled::also`].
///
/// The role's business and not the protocol's — `rivetos_addon` deliberately
/// defines no vocabulary of ids, exactly as `ADR-053` defines no vocabulary of
/// classes.
pub const INDOOR_HUMIDITY: &str = "humidity";

/// **The indoor addon's answer, as the column Aura's renderer draws.**
///
/// `label` comes from this host rather than the addon: it is the sensor this
/// program ASKED for, carried for the journal and not drawn
/// ([`wire::Indoor::label`] says so), and taking it from the answer would let
/// a card claim a different sensor than the one configured.
///
/// **`online` is the whole of whether this is drawn at all.** A dead sensor
/// and a perfectly stable room produce the same flat line, so anything short
/// of [`Status::Ok`] with a reading in hand is `false` — the renderer then
/// draws nothing rather than a number nobody should trust.
#[must_use]
pub fn indoor_card(polled: &Polled, label: &str) -> wire::Indoor {
    let bare = || wire::Indoor {
        label: label.to_string(),
        online: false,
        ..wire::Indoor::default()
    };

    if polled.status != Status::Ok {
        return bare();
    }
    let Some(reading) = polled.reading.as_ref() else {
        return bare();
    };

    // **Humidity is looked up and not positional.** A source that stops
    // reporting it, or reports it second, must not turn into a card showing
    // the wrong number under the wrong label — which is what an index would
    // do the first time the order changed.
    let humidity = polled
        .also
        .iter()
        .find(|m| m.id == INDOOR_HUMIDITY)
        .map(|m| m.reading.value);

    wire::Indoor {
        label: label.to_string(),
        // The addon's unit, for `card`'s reason: it converted, and a label
        // from anywhere else is a second source of truth that is right only
        // while the two agree.
        unit: reading.unit.clone(),
        temp_f: Some(reading.value),
        humidity_pct: humidity,
        online: true,
    }
}

/// The settings this host hands [`role::INDOOR`], from what a person set.
#[must_use]
pub fn indoor_settings(
    url: &str,
    verify_as: &str,
    sensor: &str,
) -> serde_json::Map<String, serde_json::Value> {
    let mut out = serde_json::Map::new();
    out.insert(indoor_key::URL.to_string(), url.into());
    out.insert(indoor_key::VERIFY_AS.to_string(), verify_as.into());
    out.insert(indoor_key::SENSOR.to_string(), sensor.into());
    out
}

/// **Write what a person typed into the indoor addon's form back into the
/// config**, the indoor role's counterpart of [`apply_settings`].
///
/// The values live in this program's config rather than in the addon because
/// the addon keeps no state across a restart and something has to remember
/// them — the same arrangement the reading card's source has. What the PORTAL
/// renders is the addon's own `Describe`, so the meaning of these three keys
/// belongs here and not to `crate::portal`, which never looks inside either
/// map.
///
/// **A key the addon did not describe is not written**, because it was never
/// rendered and so was never submitted; `schema::validate_fields` has already
/// refused anything outside the declared set by the time this runs.
///
/// Returns whether anything changed, so the caller knows whether to save.
#[must_use]
pub fn apply_indoor_settings(
    config: &mut crate::config::Config,
    parsed: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    let mut changed = false;
    if let Some(url) = parsed.get(indoor_key::URL).and_then(serde_json::Value::as_str) {
        // Trailing slashes are trimmed here rather than in the addon: it is
        // this side that stores the value, and a URL that differed only by a
        // slash would read as a change on every save.
        let url = url.trim().trim_end_matches('/');
        if url != config.tempmon_url {
            config.tempmon_url = url.to_string();
            changed = true;
        }
    }
    if let Some(name) = parsed
        .get(indoor_key::VERIFY_AS)
        .and_then(serde_json::Value::as_str)
    {
        // **Emptied deliberately means empty**, and it always has: this is how
        // a person turns the name check off again after trying it, and a field
        // that could only ever be set would trap them.
        let name = name.trim();
        if name != config.tempmon_name {
            config.tempmon_name = name.to_string();
            changed = true;
        }
    }
    if let Some(sensor) = parsed
        .get(indoor_key::SENSOR)
        .and_then(serde_json::Value::as_str)
    {
        let sensor = sensor.trim();
        if sensor != config.indoor_sensor {
            config.indoor_sensor = sensor.to_string();
            changed = true;
        }
    }
    changed
}

/// How old the reading is, in seconds, **clamped at zero**.
///
/// A negative age means the source's clock is ahead of ours. It is not worth a
/// failure, but reporting it as a large positive age would be a lie — and the
/// card draws its staleness mark from a number this program computed, so the
/// lie would be on the wall rather than in a log.
///
/// Computed by the HOST from [`Reading::taken_at_ms`] rather than sent by the
/// addon, because it changes every second without anything new arriving and a
/// number sent at fetch time would be wrong by the time it was drawn.
#[must_use]
fn age_seconds(reading: &Reading, now_ms: i64) -> i64 {
    now_ms.saturating_sub(reading.taken_at_ms).max(0) / 1_000
}

/// The protocol's four statuses, as the four words Aura's schema uses.
///
/// **The mapping is not one-to-one in spirit and the two middle rows are why.**
/// `Unavailable` is *nothing yet, and no reason to think anything is wrong*,
/// which Aura has always drawn as `connecting` — a cold start must not
/// announce itself as a fault. `Faulted` is *this will not fix itself*, which
/// is `error` and is shown at once rather than waiting twenty minutes for a
/// held reading to age out, because there is no reading coming.
///
/// Aura drew those two differently since before it was ported, and a protocol
/// that collapsed them would have made the card worse than the program it
/// replaced — which is the argument `rivetos_addon::Status` carries for having
/// a fourth variant at all.
#[must_use]
pub fn status_of(status: Status) -> &'static str {
    match status {
        Status::Ok => wire::status::OK,
        Status::Stale => wire::status::STALE,
        Status::Unavailable => wire::status::CONNECTING,
        Status::Faulted => wire::status::ERROR,
    }
}

/// The protocol's trend, as the slug the renderer reads.
///
/// **`None` becomes `unknown` rather than an empty string**, which is what the
/// Go controller emitted and what `aura-renderer`'s `trend_for` turns back
/// into *no arrow*. The round trip is: the source says it does not know, the
/// addon declines to invent a sixth variant, and the renderer draws nothing —
/// three programs agreeing not to claim something about a reading.
#[must_use]
pub fn trend_of(trend: Option<Trend>) -> String {
    match trend {
        Some(Trend::RisingQuickly) => "rising_fast",
        Some(Trend::Rising) => "rising",
        Some(Trend::Flat) => "flat",
        Some(Trend::Falling) => "falling",
        Some(Trend::FallingQuickly) => "falling_fast",
        None => "unknown",
    }
    .to_string()
}

#[cfg(test)]
mod tests;
