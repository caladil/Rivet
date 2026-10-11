//! **The configuration file, and the migrations that keep an old one usable.**
//!
//! The direct port of `internal/config/config.go`. Same fields, same defaults,
//! same migration notes — a note per change, returned to the caller, which
//! decides whether to log them and save. Saving is what actually applies a
//! migration; reporting alone would leave the file as it was.
//!
//! # Where it lives, and why there is no `$HOME` branch
//!
//! `RIVET_APP_DATA/config.json`, always. The Go program keeps three paths —
//! `AURA_CONFIG`, then `RIVET_APP_DATA`, then `$HOME/.config/aura` — because
//! one binary runs on a developer's machine, on Raspberry Pi OS and on a Rivet.
//! This binary runs on a Rivet and nowhere else, so the other two are gone: a
//! supervisor spawns with fixed arguments and an environment it controls, and a
//! fallback that can never be taken is a path nothing tests.
//!
//! # The two places this DIFFERS from the Go config, both deliberate
//!
//! **`timezone` is gone.** The Go program carries an IANA name and an embedded
//! copy of the zone database to resolve it. On a Rivet the machine already has
//! a `timezone` document key, which the supervisor projects to `/etc/localtime`
//! (`ADR-021` §9) — so keeping Aura's own would be a second answer to a
//! question the platform answers, which `ADR-011` is against. A file carrying
//! the old key is migrated: the value is reported and dropped, rather than left
//! in place to be edited by somebody who would then watch it do nothing. See
//! `crate::clock`.
//!
//! **`tempmon_url` defaults to the loopback address, not another machine's.**
//! The Go program's default was a fixed LAN address: the TempMon on the machine
//! this appliance is replacing. On a Rivet, TempMon is an application on the
//! same board and the supervisor opens **8081** for it — 8080 being taken, by
//! this program's own portal. A default pointing at another host would be a
//! fresh install reaching across the LAN for a reading available locally.
//!
//! **And it defaults to `https://`, not `http://`** — `.rulings/` R-216.
//! TempMon on a Rivet serves only HTTPS on that port, so the old default
//! could never actually connect on a fresh install; it read as "working"
//! only because nobody had changed it yet. [`effective_tempmon_name`] is the
//! other half of making that connection succeed: only an HTTPS URL needs a
//! name to verify a certificate against at all, and **which name depends on
//! whose certificate it is** — this machine's own, when the URL is this
//! machine's own loopback address, or the saved [`Config::tempmon_name`] when
//! it names another machine. A plain `http://` URL needs neither, and
//! `effective_tempmon_name` says so directly rather than picking one. See
//! [`Config::tempmon_name`]'s own doc, and [`url_is_loopback`] and
//! [`url_is_plain_http`] for the two questions that decide between the three.
//!
//! # And one more, which is a capability rather than a value: HDMI-CEC is gone
//!
//! The Go controller switches a television on and off by shelling out to
//! `cec-client`, and `cec_enabled`, `cec_hdmi_input` and a `cec` scheduled
//! action configure it. **None of that can port, and none of it should.**
//! `rivet-adapter-cec` says why in its own first paragraphs: a kiosk
//! application is leased the CRTC exclusively, so CEC is deliberately a
//! *different device* the machine keeps for itself, and `ADR-039` §3.1's
//! `[display] off/on` table is the policy — *"a crate that read a schedule
//! would be a second authority for the hours"*. There is also no shell and no
//! `cec-client` to run.
//!
//! So the three keys are **migrated out with a note naming where the setting
//! went**, rather than kept as forms that save nothing. That is the failure
//! this project has already paid for once: a portal that stored settings which
//! reached no code.
//!
//! The `display` scheduled action is a different thing and **stays** — it is
//! `display_sleep`/`display_wake` on the wire, which blanks the renderer's
//! overlay widgets and touches no television.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// TempMon on this same board, over HTTPS. See the module note — neither the
/// host nor the scheme is the Go default.
///
/// TempMon's machine-local read port, not its routed one: the routed port is
/// guarded by Rivet's firewall (`ADR-023`) and refuses every caller but the
/// proxy, so a reader on the same machine uses the port TempMon declares for it.
pub const DEFAULT_TEMPMON_URL: &str = "https://127.0.0.1:8083";
/// The sensor nickname a fresh install looks for: **none**, which the indoor
/// source reads as *the first sensor TempMon reports*.
///
/// Empty rather than a name because any name is somebody's room, and a
/// default naming one would be a fresh install looking for a sensor that only
/// exists in the house it was written in. A machine with one sensor needs no
/// setting at all this way; a machine with several picks one in the portal.
pub const DEFAULT_INDOOR_SENSOR: &str = "";

/// The one writable directory this process has.
///
/// # Errors
///
/// [`rivet_sdk::NotHanded`] when `RIVET_APP_DATA` is unset, which means this
/// program was started by something that is not a Rivet supervisor. It is a
/// refusal to start rather than a fallback: every path below is under here, and
/// a guess would be a program writing somewhere nobody granted.
pub fn data_dir() -> Result<PathBuf, rivet_sdk::NotHanded> {
    rivet_sdk::app::data()
}

/// `RIVET_APP_DATA/config.json`.
///
/// # Errors
///
/// As [`data_dir`].
pub fn path() -> Result<PathBuf, rivet_sdk::NotHanded> {
    Ok(data_dir()?.join("config.json"))
}

/// Every user-configurable setting.
///
/// `#[serde(default)]` on the struct is what makes an older file readable: a
/// key that is absent takes the value [`Config::default`] gives it, which is
/// exactly what Go's *unmarshal over a defaults value* does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 5–100, the global widget overlay brightness.
    pub brightness: i32,

    // ---- Clock ----------------------------------------------------------
    pub clock_enabled: bool,
    pub use_24_hour: bool,
    pub show_date: bool,

    // ---- Weather (NWS) --------------------------------------------------
    /// **The panel's one switch.**
    ///
    /// Its composition is fixed: condition icon, outdoor temperature, the
    /// indoor column and the NWS statement are drawn together or not at all.
    /// The former per-element toggles were retired; see [`migrate`].
    pub weather_enabled: bool,
    pub zip_code: String,
    /// `imperial` or `metric`.
    pub temp_unit: String,
    /// **How the National Weather Service can reach this appliance's
    /// operator** — an e-mail address or a URL, sent in the `User-Agent` of
    /// every weather request because the NWS asks each client for one.
    ///
    /// **Empty by default, and that is the point of it being a setting.** A
    /// contact compiled into the program would be the same person's address on
    /// every Aura anybody builds. Empty, a request names only the program
    /// (`crate::weather::user_agent`).
    pub weather_contact: String,

    // ---- Indoor conditions (TempMon) ------------------------------------
    pub indoor_enabled: bool,
    /// Base URL, e.g. `https://127.0.0.1:8083`.
    pub tempmon_url: String,
    /// **The other machine's own name**, for when [`Config::tempmon_url`]
    /// points at one over HTTPS — **stored exactly as the operator left it,
    /// empty included.** [`effective_tempmon_name`] is where this is read for
    /// USE, and it never writes back here: see that function's own doc for
    /// the three cases. The portal's own form round-trips this field
    /// unchanged for the same reason — never the resolved name, which would
    /// otherwise get saved back the moment somebody hit Save.
    ///
    /// **Never read at all for a plain `http://` URL** — `.rulings/` R-219.
    /// There is no certificate on the wire to verify a name against, so a
    /// name is not merely unused there, it is not a question, and
    /// [`crate::portal::html`] never shows the field for one.
    ///
    /// **Never read at all while the URL is loopback either** — `.rulings/`
    /// R-219. TempMon reached at `127.0.0.1` is THIS board, whose certificate
    /// carries this board's own name and nothing an operator could usefully
    /// type here, so [`effective_tempmon_name`] asks the machine directly
    /// instead and this field sits unused, whatever it holds. A name saved
    /// here from some earlier setup, or never cleared after the URL changed,
    /// must not be able to break TLS while it is hidden from the form for
    /// exactly that reason.
    ///
    /// **Consulted, and required, once the URL points at another machine over
    /// HTTPS.** The machine's certificate carries one name — `rivet-01.local`
    /// — and musl cannot resolve it, so the URL is that machine's bare
    /// address and this is the name to verify instead. `rivet-proxyd` solves
    /// the same problem the same way with its `upstream-name` setting. Empty
    /// here then refuses rather than guessing this board's own name for a
    /// certificate that is certainly not this board's.
    pub tempmon_name: String,
    /// Sensor nickname or id — a sensor's label as TempMon reports it. **Empty
    /// means the first sensor TempMon reports**, which is also the default; see
    /// [`DEFAULT_INDOOR_SENSOR`].
    pub indoor_sensor: String,

    // ---- The corner reading card (an addon, R-197 D1 and R-375 D1) -----
    /// **The host's own decision about its own screen, and the only setting
    /// of this role the host holds an opinion about** — see `crate::addon`'s
    /// module note on why the addon itself has no `enabled` field (R-198 D2).
    pub reading_enabled: bool,
    /// **The reading source's own settings, in the addon's own shape** —
    /// keyed by the field keys its `Describe` declared, values as JSON.
    ///
    /// The host knows nothing of what any of them mean (`.rulings/` R-375 D1):
    /// the portal renders and validates them from `Describe`
    /// (`crate::portal::schema`) and writes the validated map here
    /// (`crate::addon::apply_settings`); the poller sends it back through
    /// `Configure` with every missing key filled from the field's own default
    /// (`crate::addon::settings_for`). It is persisted because the addon keeps
    /// nothing across a restart and the host re-sends it every time it loads.
    ///
    /// Empty on a fresh install, which sends every field's default.
    ///
    /// **There is no secret in it, and there must not be one.** A `Secret`
    /// field's contents are written to the platform's secret store by
    /// `crate::portal::schema::store_secrets` and handed over as material,
    /// resolved by name (`crate::addon::material`) — never stored here, never
    /// in a backup.
    //
    // **There is no credential path either, and there must not be one
    // again.** The addon names the material it needs in its `Describe`, and a
    // key here naming one of those files would be a second authority over a
    // fact the addon already supplies, which is what `ADR-011` is about. A
    // file written by an older build still carries the key; `Config` no
    // longer has the field, so the next save drops it.
    pub reading_settings: serde_json::Map<String, serde_json::Value>,

    /// Derived from [`Config::zip_code`] by geocoding — **never user-entered**.
    /// A cache whose `zip` differs from the configured one is stale and is
    /// refreshed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geo_cache: Option<GeoCache>,

    // ---- Screensaver ----------------------------------------------------
    /// `solid`, `matrix` or `boxes`.
    pub screensaver: String,
    /// **The theme is not a field on the wire.** It is folded together with
    /// [`Config::screensaver`] into one value by
    /// `crate::screensaver::ipc_value`, which is why grepping the schema for
    /// `theme` finds nothing.
    pub screensaver_theme: String,

    // **There is no `theme` key, and the distinction is worth stating because
    // this file briefly had one.**
    //
    // How this PAGE is painted is a preference belonging to the browser
    // looking at it, and it lives in that browser's `localStorage`. A
    // configuration is what this APPLIANCE is — `temp_unit` decides what the
    // wall renders and must persist, which is exactly why it IS here — and a
    // viewing preference kept beside it would follow a person from their
    // laptop to their phone, which is not what choosing dark mode means.
    //
    // The test is whose preference it is, not which app it is in.

    // ---- Schedule -------------------------------------------------------
    //
    // **There is no `cec_enabled` and no `cec_hdmi_input`.** See the module
    // note; HDMI-CEC is the machine's and not this application's.
    pub schedule: Vec<ScheduledEvent>,
    //
    // **There is no `portal_password` and there must never be one again.**
    // `R-152.1` retired it: a shared secret, in cleartext, in this file — so
    // anything able to read the config could mint a portal session, and the
    // shipped default was in the source. What authenticates a caller now is a
    // certificate chaining to the fleet authority, checked during the TLS
    // handshake. A file written by an older build still carries the key;
    // [`migrate`] reports it, and the save that follows is what removes it.
    //
    // **There is no `timezone` either.** See the module note.
}

impl Default for Config {
    fn default() -> Self {
        Self {
            brightness: 60,
            clock_enabled: true,
            use_24_hour: false,
            show_date: true,
            weather_enabled: true,
            zip_code: String::new(),
            temp_unit: "imperial".to_string(),
            weather_contact: String::new(),
            indoor_enabled: true,
            tempmon_url: DEFAULT_TEMPMON_URL.to_string(),
            // Empty, and deliberately not `rivet-01.local`: a default naming
            // the maintainer's own machine is the mistake the Go program's
            // fixed LAN address was, in a different costume. `DEFAULT_TEMPMON_URL`
            // is loopback, so `effective_tempmon_name` never even reads this for
            // a fresh install — only once the URL is changed to point at
            // another machine is it consulted at all.
            tempmon_name: String::new(),
            indoor_sensor: DEFAULT_INDOOR_SENSOR.to_string(),
            reading_enabled: true,
            // Empty: every field the addon declares goes out as its own
            // default until somebody saves the form.
            reading_settings: serde_json::Map::new(),
            geo_cache: None,
            // **The pack's ids, since T7** — `black_white` was the pre-T7
            // spelling and named no theme at all; the fold read it as *not
            // white and not navy*, i.e. black. `crate::screensaver::migrate`
            // translates a file that still holds it.
            screensaver: "solid".to_string(),
            screensaver_theme: "black".to_string(),
            schedule: Vec::new(),
        }
    }
}

/// The result of geocoding a zip code.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeoCache {
    pub zip: String,
    pub lat: f64,
    pub lng: f64,
}

/// One or more actions fired at a time of day, optionally on certain days.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ScheduledEvent {
    /// `HH:MM`, 24-hour.
    pub time: String,
    /// `["Mon", "Tue", …]`. **Absent means every day**, which is why it is
    /// omitted when empty rather than written as `[]`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub days: Vec<String>,
    pub actions: Vec<ScheduledAction>,
}

/// A single operation fired by a [`ScheduledEvent`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ScheduledAction {
    /// `screensaver`, `brightness`, `display`, `cec` or `weather`.
    #[serde(rename = "type")]
    pub kind: String,
    /// `solid`, `on`, `off`, `75` — whatever `kind` takes.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub value: String,
    /// `ice_blue`, `dark`, … — only `screensaver` uses it.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub theme: String,
}

/// Action types that no longer do anything, and why.
///
/// They are **dropped on load** rather than left in place: an action that
/// silently does nothing is worse than one that is gone and said so.
const RETIRED_ACTIONS: &[(&str, &str)] = &[
    (
        "weather_text",
        "the forecast statement is now a fixed part of the weather panel",
    ),
    (
        "cec",
        "switching a television on and off is the machine's, not this \
         application's — set `[display] off` and `on` in the machine document \
         (ADR-039 §3.1)",
    ),
];

/// Read the config, returning defaults when the file does not exist.
///
/// Any migration the file needs is applied to the returned value and reported
/// in the notes; **the caller decides whether to log them and save**, and it is
/// the save that makes a migration real.
///
/// # Errors
///
/// A sentence, when the path cannot be resolved, the file cannot be read for a
/// reason other than absence, or its contents are not the JSON this understands.
/// A config that cannot be parsed is a refusal to start rather than a silent
/// reset: overwriting somebody's settings with defaults because a byte was
/// wrong is the worse of the two failures.
pub fn load() -> Result<(Config, Vec<String>), String> {
    let path = path().map_err(|e| e.to_string())?;
    let raw = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut cfg = Config::default();
            let notes = migrate(&mut cfg, "");
            return Ok((cfg, notes));
        }
        Err(e) => return Err(format!("config: read {}: {e}", path.display())),
    };
    let mut cfg: Config = serde_json::from_str(&raw)
        .map_err(|e| format!("config: parse {}: {e}", path.display()))?;
    let notes = migrate(&mut cfg, &raw);
    Ok((cfg, notes))
}

/// Bring a config written by an older build up to date, one note per change.
///
/// `raw` is the bytes the file held, which is how a field that no longer exists
/// on [`Config`] can still be noticed: a JSON reader drops an unknown key
/// silently, and **a secret that is merely ignored is a secret still sitting on
/// the disk**.
///
/// **Nothing here reads the environment**, which is why every test below can
/// call it directly. That was not true while a credential path was resolved
/// here; the key is gone and so is the exception.
pub fn migrate(cfg: &mut Config, raw: &str) -> Vec<String> {
    let mut notes = Vec::new();

    // **The screensaver vocabulary, T7.** `matrix`/`ice_blue` become
    // `digital-rain`/`ice-blue` — the ids `aura-background-pack` publishes and
    // the renderer now takes on the wire. An operator's saved choice has to
    // survive the update that changes what the words are, and this is the one
    // place it can: both halves stopped holding a table of the ten old words.
    //
    // `crate::screensaver::migrate` passes an unrecognised value through, so
    // this is safe to run on every load rather than once behind a flag — see
    // `migrating_an_already_migrated_pair_changes_nothing`.
    {
        let (background, theme) =
            crate::screensaver::migrate(&cfg.screensaver, &cfg.screensaver_theme);
        if background != cfg.screensaver || theme != cfg.screensaver_theme {
            notes.push(format!(
                "screensaver: `{}`/`{}` is the pre-T7 spelling and was read as \
                 `{background}`/`{theme}`, which is what the installed background pack calls \
                 it. The next save writes the new spelling",
                cfg.screensaver, cfg.screensaver_theme,
            ));
            cfg.screensaver = background;
            cfg.screensaver_theme = theme;
        }
    }

    // `portal_password`, retired by `R-152.1`. Reporting it is what makes the
    // caller save, and saving is what removes it: `Config` no longer has the
    // field, so the rewrite cannot carry it forward.
    if raw.contains("\"portal_password\"") {
        notes.push(
            "portal_password: REMOVED from this file — the portal now authenticates its \
             caller with a certificate chaining to the fleet authority, and the old value \
             was a shared secret stored here in cleartext"
                .to_string(),
        );
    }
    // `timezone`, retired here rather than in Go. The machine owns it.
    if raw.contains("\"timezone\"") {
        notes.push(
            "timezone: REMOVED from this file — the machine's own `timezone` key is \
             projected to /etc/localtime and is what this program now reads, so a second \
             copy here could only disagree with it"
                .to_string(),
        );
    }
    // The CEC pair, retired for a reason that is about capability rather than
    // authority: this application has no way to reach a television, and the
    // machine does. See the module note.
    if raw.contains("\"cec_enabled\"") || raw.contains("\"cec_hdmi_input\"") {
        notes.push(
            "cec_enabled / cec_hdmi_input: REMOVED from this file — a kiosk application \
             holds the display lease and cannot touch /dev/cec, so switching a television \
             on and off is the machine's. Set `[display] off` and `on` in the machine \
             document instead (ADR-039 §3.1)"
                .to_string(),
        );
    }

    for event in &mut cfg.schedule {
        let time = event.time.clone();
        event.actions.retain(|action| {
            match RETIRED_ACTIONS.iter().find(|(kind, _)| *kind == action.kind) {
                Some((kind, why)) => {
                    notes.push(format!(
                        "schedule {time}: dropped retired action {kind:?}={:?} — {why}",
                        action.value
                    ));
                    false
                }
                None => true,
            }
        });
    }
    // An event whose only actions were retired would now fire nothing.
    cfg.schedule.retain(|event| {
        if event.actions.is_empty() {
            notes.push(format!(
                "schedule {}: removed — it had no actions left",
                event.time
            ));
            return false;
        }
        true
    });

    if cfg.tempmon_url.is_empty() {
        cfg.tempmon_url = DEFAULT_TEMPMON_URL.to_string();
        notes.push(format!("tempmon_url: set to {DEFAULT_TEMPMON_URL}"));
    }
    // **An empty `indoor_sensor` is not repaired, because it is not broken.**
    // It is the default, and it means *the first sensor TempMon reports* — so
    // an operator who empties the field gets exactly that, rather than a name
    // written back over their choice on the next start.

    // **The reading card's settings are not checked here, and that is the
    // point of them being the addon's.** A unit, a band or an account is the
    // addon's to validate: the portal holds a submission to the bounds the
    // addon's `Describe` states, and the addon refuses in `Configure` what no
    // single field can express — a band with its ends crossed, say — in its
    // own words. A host-side repair would need the host to know what the keys
    // mean, which is what `.rulings/` R-375 D1 rules it must not.

    // **The retired credential-path key gets no note, and that is deliberate.**
    // The other removals above are reported because each changed what the
    // program does: a cleartext secret stopped being read, a timezone stopped
    // being obeyed, a television stopped being switched. This one cannot,
    // because no build ever offered a way to set it to anything but the value
    // the program computed itself — so on every machine that has one, the key
    // names the same file `crate::addon::material` now resolves by name. A
    // note would announce a change to an operator who has not had one.

    notes
}

/// Write the config, atomically.
///
/// Atomically because the portal saves on every form submission and this
/// program reads the file at start: a truncated write that is never read back
/// is a config nobody knows is broken until the next restart.
///
/// # Errors
///
/// A sentence naming the path.
pub fn save(cfg: &Config) -> Result<(), String> {
    let path = path().map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(cfg)
        .map_err(|e| format!("config: render: {e}"))?;
    let temporary = path.with_extension("json.new");
    std::fs::write(&temporary, text.as_bytes())
        .map_err(|e| format!("config: write {}: {e}", temporary.display()))?;
    std::fs::rename(&temporary, &path)
        .map_err(|e| format!("config: rename into {}: {e}", path.display()))
}

/// Whether `url`'s host names this same machine over the loopback interface
/// — `127.0.0.0/8`, `::1`, or the literal name `localhost`, with or without a
/// port or a path. `.rulings/` R-219: this is one of the two questions
/// [`effective_tempmon_name`] and the portal's own rendering need answered,
/// and it is asked of the URL's TEXT and nothing else — no resolution, no
/// socket, because a URL this program itself wrote never needs either.
///
/// **Anything this cannot make sense of is NOT loopback.** A URL malformed
/// enough to confuse this will already fail to connect on its own account,
/// and treating an unparsed guess as loopback would be inventing a machine
/// identity rather than refusing to guess one.
#[must_use]
pub fn url_is_loopback(url: &str) -> bool {
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    // The authority ends at the first `/`, `?` or `#` — whichever comes
    // first, exactly as a browser's own address bar parses one.
    let authority_end = after_scheme.find(['/', '?', '#']).unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_end];

    let host = if let Some(rest) = authority.strip_prefix('[') {
        // A bracketed IPv6 literal: `[::1]` or `[::1]:8081`.
        rest.split(']').next().unwrap_or("")
    } else {
        // `host:port`, or just `host`. The port is trusted only when every
        // byte after the last `:` is a digit, so a bare (bracket-less) IPv6
        // literal — which this program never writes, and which is not valid
        // URL syntax to begin with — falls through to being checked whole
        // rather than having its last group of digits mistaken for a port.
        authority.rsplit_once(':').map_or(authority, |(host, port)| {
            if port.chars().all(|c| c.is_ascii_digit()) { host } else { authority }
        })
    };

    let host = host.to_ascii_lowercase();
    host == "localhost"
        || host == "::1"
        || host.strip_prefix("127.").is_some_and(|rest| {
            // Exactly three more dot-separated octets — `127.0.0.1`, never
            // `127.0.1` (too few) or `127.0.0.1.2` (too many).
            let parts: Vec<&str> = rest.split('.').collect();
            parts.len() == 3 && parts.iter().all(|part| part.parse::<u8>().is_ok())
        })
}

/// Whether `url`'s scheme is `http://`, case-insensitively. `.rulings/`
/// R-219: the other question a name is conditioned on — there is no
/// certificate at all on a plain HTTP connection, so there is nothing for a
/// name to verify.
#[must_use]
pub fn url_is_plain_http(url: &str) -> bool {
    url.get(..7).is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
}

/// **Whether the operator should be asked for [`Config::tempmon_name`] at
/// all.** `.rulings/` R-219: the single predicate `crate::portal::html`'s
/// hidden key and the page's own `syncTempmonNameField` script both answer
/// this exact question from, so the three places a name might or might not
/// matter cannot drift apart into three separate opinions about it.
#[must_use]
pub fn tempmon_name_is_asked(url: &str) -> bool {
    !url_is_plain_http(url) && !url_is_loopback(url)
}

/// **The name to verify TempMon's certificate against, for USE — never for
/// storage.** `.rulings/` R-219: the name only matters once it is known
/// WHETHER there is a certificate to check and WHOSE it is, so `url` decides
/// which of three answers applies — `configured` alone was never enough to
/// say.
///
/// **`url`'s scheme is plain `http://`** ([`url_is_plain_http`]): there is no
/// certificate on the wire at all, so a name is not merely unresolved here,
/// it is not a question — `Ok(String::new())`, without reading the hostname
/// or `configured`. This is checked first, because a loopback `http://` URL
/// is still a URL with nothing to verify.
///
/// **Otherwise, `url`'s host is loopback** ([`url_is_loopback`]): TempMon is
/// this same board, reached over its own loopback address under whatever
/// name the wire happens to call it, and this board's certificate carries
/// exactly one name — its own. So the effective name is ALWAYS this board's
/// own `<hostname>.local`, asked fresh of `rivet_sdk::facts`, and
/// `configured` is never even read: a name saved for some earlier setup, or
/// never cleared after the URL changed to point back here, must not be able
/// to silently break TLS while it sits hidden from the form
/// (`crate::portal::html`). `Err` when the machine cannot say what it is
/// called — refused rather than guessed, on the same footing as an empty
/// `zip_code` refusing a weather fetch rather than resolving some other
/// address.
///
/// **Anything else:** TempMon is some other machine, whose certificate this
/// board cannot possibly carry the name of, so the only correct answer is
/// `configured`, exactly as saved — an operator who typed a name, or emptied
/// one deliberately and is about to type another, is never second-guessed.
/// `Err`, naming the reason, when that is empty: never this board's own
/// name, which is the one name that is certainly wrong for a certificate
/// that is not this board's.
pub fn effective_tempmon_name(url: &str, configured: &str) -> Result<String, String> {
    if url_is_plain_http(url) {
        return Ok(String::new());
    }
    if url_is_loopback(url) {
        return rivet_sdk::facts::ask()
            .ok()
            .and_then(|facts| facts.hostname)
            .map(|hostname| format!("{hostname}.local"))
            .ok_or_else(|| "this machine's own name could not be read".to_string());
    }
    if configured.is_empty() {
        return Err(
            "TempMon here is on another machine — enter that machine's own name so its \
             certificate can be verified"
                .to_string(),
        );
    }
    Ok(configured.to_string())
}

#[cfg(test)]
mod tests;
