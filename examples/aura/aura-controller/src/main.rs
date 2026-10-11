//! **Aura's controller, as a Rivet application** — the composition root.
//!
//! Read this beside `cmd/aura-controller/main.go` in the Go tree. The wiring
//! below is the same wiring in the same order, and every line that differs
//! differs because something is **handed** here that is **taken** there. The
//! table in `Cargo.toml` is the list.
//!
//! # What this program is
//!
//! It holds every fact the display shows — the time, the outdoor conditions,
//! the room's temperature, the corner reading card's value — and pushes them
//! to `aura-renderer` once a second over a socket the supervisor opened. It
//! also serves the portal a person configures it from. It draws nothing and
//! never touches a display; that is the renderer's half, and the two are
//! separate programs because the maintainer ruled them so on 2026-09-02.
//!
//! # Some of those facts are not this program's
//!
//! The corner card's reading and the room's temperature arrive from **shared
//! rivets** the supervisor joined to this application by class — `ADR-053`,
//! and `.rulings/` R-197 D1 for why the code lives outside this repository
//! rather than being tidied inside it. `crate::addon` is this program's half
//! of that boundary, and it is kept to one module on purpose: everything else
//! here sees a `wire::ReadingData` and has no way to learn where it came from
//! or what it measures (`.rulings/` R-375 D1).
//!
//! # Threads, where Go has goroutines
//!
//! One per poller, one for the schedule, the accept thread inside
//! `crate::kiosk`, the portal's accept thread and one per portal connection,
//! and this one. Threads that spend their lives asleep are not a design worth
//! avoiding, and it keeps every module in this crate ordinary blocking code —
//! which is also what the SDK's `serve` helpers are written against.
//!
//! **A poller waits on a channel rather than sleeping**, so the portal can wake
//! it. Saving a zip code, a sensor name or a credential takes effect at once
//! instead of at the next interval — which for a reading source's credential
//! could otherwise be half an hour away. A timeout on that wait is the
//! ordinary cadence; a message is somebody having changed something.
//!
//! # HDMI-CEC is not here, and is not coming
//!
//! It is the machine's. A kiosk application holds the display lease exclusively
//! and cannot reach `/dev/cec`, so `ADR-039` §3.1's `[display] off/on` table is
//! where those hours live; `crate::config` carries the argument and the
//! migration that tells an existing config where its setting went.

mod addon;
mod clock;
mod config;
mod geocode;
mod kiosk;
mod portal;
mod schedule;
mod screensaver;
mod state;
mod units;
mod weather;
mod wire;

use std::sync::{Arc, RwLock};
use std::time::Duration;

use rivet_sdk::journal::Journal;

/// The service name the journal files records under, and the application's own
/// name in the machine document.
///
/// **`pub(crate)`**: `crate::portal::schema` prints it in the command that
/// installs a `Secret` field's material — `rivet apply install-secret
/// <this> <name> …` — and it is the one name that must never drift from what
/// the manifest declares.
pub(crate) const SERVICE: &str = "aura-controller";

/// How often the NWS is asked. Twenty minutes, as in Go: the station publishes
/// hourly, so this is already three times more often than there is news.
const WEATHER_INTERVAL: Duration = Duration::from_secs(20 * 60);

/// How long to wait after a FAILED fetch before retrying, distinct from the
/// twenty minutes a success earns.
///
/// A cold boot races the network: the fetch at [`WEATHER_STARTUP_DELAY`]
/// regularly loses to a DNS resolver that is not ready yet (`EAI_AGAIN`), and
/// retrying that on the success cadence left the panel `unavailable` for a full
/// twenty minutes after every boot. The failure wait starts here and doubles up
/// to [`WEATHER_INTERVAL`], so a boot-time miss recovers in tens of seconds
/// without hammering a free service through a real outage.
const WEATHER_RETRY_MIN: Duration = Duration::from_secs(30);

/// How long the weather poller waits before its first fetch.
///
/// Fifteen seconds, as in Go. It is what lets the geocode finish, and on a
/// board it is also what lets DHCP and NTP land — a fetch issued in the first
/// second of a cold boot fails on a machine that simply has no route yet.
const WEATHER_STARTUP_DELAY: Duration = Duration::from_secs(15);

/// How often TempMon is asked. The sensor reports about once a minute, so a
/// sixty-second poll is as fresh as the data can be.
const INDOOR_INTERVAL: Duration = Duration::from_secs(60);

/// How often the reading source's material is re-read when nothing has asked
/// for it.
///
/// **This is a sweep for a change that gives no signal**, and there is exactly
/// one: `rivet apply install-secret aura-controller <name>`, where `<name>` is
/// whatever the addon's `Describe` asked for. It writes a credential into this
/// application's data directory without going anywhere near the portal that
/// would otherwise nudge. Half a minute is short enough
/// that installing one feels immediate and long enough that it is a file read
/// a person would never notice.
///
/// Everything else already has a signal — the portal nudges, and a changed
/// setting is visible in the config — so this is a floor on how late the
/// silent case can be, not the mechanism any of them relies on.
const MATERIAL_RECHECK: Duration = Duration::from_secs(30);

fn main() -> std::process::ExitCode {
    let journal = Journal::for_service(SERVICE);
    match run(&journal) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            journal.error(&e);
            std::process::ExitCode::FAILURE
        }
    }
}

/// Everything, with a return type that lets a failure be reported once.
///
/// # Errors
///
/// A sentence. Every one of them is a refusal to start: a missing data
/// directory, an unreadable config, a kiosk socket the supervisor did not
/// hand over, or a portal socket or TLS file `portal::prepare` could not
/// claim or read. None of these has a sensible fallback, and a controller
/// that started anyway would be a blank display with no explanation.
///
/// **`portal::prepare` is called before any add-on is opened**, precisely so
/// that this list of refusals is the whole list: an add-on's own load writes
/// a `rivet_addon::trial` note before it touches a shared object, and a
/// refusal reached after that note was written and before it settled leaves
/// it behind for the next start to trip over. See `portal::prepare`'s own doc
/// comment.
fn run(journal: &Journal) -> Result<(), String> {
    // ---- Config -------------------------------------------------------
    let (loaded, notes) = config::load()?;
    let path = config::path().map_err(|e| e.to_string())?;
    journal.info(&format!(
        "config loaded: path={} brightness={} screensaver={}",
        path.display(),
        loaded.brightness,
        loaded.screensaver
    ));
    if !notes.is_empty() {
        for note in &notes {
            journal.info(&format!("config migrate: {note}"));
        }
        // **The save is what applies the migration.** Reporting alone would
        // leave a retired key sitting on the disk — which, for
        // `portal_password`, means leaving a shared secret there.
        match config::save(&loaded) {
            Ok(()) => journal.info(&format!(
                "config migrate: applied {} change(s) and saved",
                notes.len()
            )),
            Err(e) => journal.warn(&format!("config migrate: save: {e}")),
        }
    }

    // Shared because the portal will write it while the tick loop reads it.
    // An `RwLock` and not a `Mutex`: this is read once a second for ever and
    // written when a person submits a form.
    let config = Arc::new(RwLock::new(loaded));
    let state = Arc::new(state::State::new());

    // ---- The portal's TLS material, checked FIRST --------------------
    //
    // **Before the kiosk socket, before the background pack, before the
    // indoor and reading-card threads — before anything that opens an
    // add-on.** `portal::prepare` claims the listening socket and validates
    // the three TLS files; it touches no add-on and starts no thread. This
    // ordering fixes a real defect: a missing `tls-cert` used to be discovered
    // only after the background pack, the indoor thread and the reading-card
    // thread had already written `rivet_addon::trial` notes for objects that had
    // not yet settled, so a crash here — which is exactly what a missing
    // certificate caused, since `run` returned an error and the process
    // exited — left every one of those add-ons skipped on every later
    // start, long after the certificate was installed. Checking the one
    // thing that is per-machine and easy to forget before touching anything
    // that keeps its own crash history means a missing certificate is just
    // a certificate to install.
    let prepared_portal = portal::prepare()?;

    // ---- The kiosk socket ---------------------------------------------
    //
    // The replay closure is the schema's promise that a renderer never persists
    // display configuration. It runs on the accept thread, after the connection
    // is installed, so what it sends reaches the renderer that triggered it.
    let sender = start_kiosk(&config)?;
    journal.info(&format!(
        "kiosk: accepting on the handed socket named {}",
        kiosk::ENDPOINT
    ));

    // ---- The pollers --------------------------------------------------
    //
    // Each hands back the channel that wakes it, and the portal holds them all.
    // The weather nudge goes to the GEOCODE rather than to the weather poller,
    // because a changed zip code needs resolving before it needs fetching —
    // which is the same order the Go portal's `triggerWeather` uses.
    let weather = start_weather(&config, &state);
    // **What `Describe` answered, shared with the portal.** `start_reading`
    // fills this in once its addon has opened and answered; the portal reads
    // it on every request to build the reading card's section — its heading
    // included — from the addon's own description (R-197 D3) rather than from
    // anything this program hard-codes. `None` until then, and for good — a
    // machine with no reading source installed says so honestly rather than
    // showing a form for fields that do not exist.
    let addon_description: Arc<RwLock<Option<rivetos_addon::Description>>> =
        Arc::new(RwLock::new(None));
    // **One per ROLE, and for the same reason there is one per role at all.**
    // Each addon describes its own settings, and the portal renders each
    // section from the description of whatever is filling that role — so a
    // machine with an indoor source and no reading source shows one form and
    // one "nothing installed" notice, without this program knowing what either
    // addon's fields are.
    let indoor_description: Arc<RwLock<Option<rivetos_addon::Description>>> =
        Arc::new(RwLock::new(None));

    // **The background pack's catalogue, for the two pickers** — T7.
    //
    // Opened here on the main thread rather than in a thread of its own,
    // because unlike a reading source there is nothing to poll: the catalogue
    // is asked for once and never changes for the life of the process. The
    // `Source` is held for the same life — an addon cannot be unloaded, and a
    // dropped handle would be a `dlclose` this program has no reason to risk.
    //
    // **A machine with no pack shows no screensaver picker**, which is honest:
    // the renderer on such a machine draws black and has nothing to select
    // between. The portal says so in one sentence, naming the library to
    // install, rather than offering ten options that reach a program which has
    // never heard of them.
    let backgrounds: Arc<RwLock<Option<rivetos_addon::Description>>> = Arc::new(RwLock::new(None));
    let _background_pack = match addon::Source::open(addon::role::BACKGROUND) {
        Ok(source) => {
            journal.info(&format!("addon: opened {}", source.summary()));
            *backgrounds.write().unwrap_or_else(|e| e.into_inner()) =
                Some(source.description().clone());
            Some(source)
        }
        Err(addon::NotOpened::Absent(sentence)) => {
            journal.info(&format!(
                "{sentence} — the portal will show no screensaver picker, because there are no \
                 backgrounds to pick between"
            ));
            None
        }
        Err(addon::NotOpened::Refused(sentence)) => {
            journal.warn(&sentence);
            None
        }
    };

    let nudges = portal::Nudges {
        weather: start_geocode(&config, &weather),
        indoor: start_indoor(&config, &state, Arc::clone(&indoor_description)),
        reading: start_reading(&config, &state, Arc::clone(&addon_description)),
    };
    start_schedule(&config, &sender);

    // ---- The portal ---------------------------------------------------
    //
    // Everything that could fail to start about the portal already has —
    // `prepared_portal` above — so this only starts the accept thread.
    portal::start(
        prepared_portal,
        &config,
        &sender,
        nudges,
        addon_description,
        indoor_description,
        backgrounds,
        journal,
    )?;

    // ---- The tick loop ------------------------------------------------
    //
    // One second, as in Go. The renderer redraws at frame rate from what it
    // last received, so this rate decides how quickly a changed fact reaches
    // the screen and nothing else.
    let mut last_logged_minute = String::new();
    loop {
        std::thread::sleep(Duration::from_secs(1));

        let (use_24_hour, show_date, weather_enabled, metric) = {
            let held = config.read().unwrap_or_else(|e| e.into_inner());
            (
                held.use_24_hour,
                held.show_date,
                held.weather_enabled,
                held.temp_unit == "metric",
            )
        };

        let now = match clock::Local::now() {
            Ok(now) => now,
            Err(e) => {
                // Not fatal, and deliberately so: a clock this program cannot
                // read is a machine-level fault, and dropping one tick is
                // better than taking the display down over it.
                journal.warn(&e);
                continue;
            }
        };
        let time = now.time_string(use_24_hour);
        let date = if show_date {
            now.date_string()
        } else {
            String::new()
        };

        state.set_clock(wire::Clock {
            time: time.clone(),
            date: date.clone(),
        });
        let mut message = state.snapshot();

        // **The weather panel is one unit**: turning it off takes the indoor
        // column with it, so the two halves can never disagree on screen.
        if weather_enabled {
            if let Some(outdoor) = message.weather.as_mut() {
                (outdoor.temp_f, outdoor.unit) = units::to_display(outdoor.temp_f, metric);
                // A wind chill and a dewpoint are temperatures too, and must
                // not be left in Fahrenheit beside a Celsius reading.
                (outdoor.secondary_f, _) = units::to_display(outdoor.secondary_f, metric);
            }
            if let Some(indoor) = message.indoor.as_mut() {
                (indoor.temp_f, indoor.unit) = units::to_display(indoor.temp_f, metric);
            }
        } else {
            message.weather = None;
            message.indoor = None;
        }

        // **The corner reading card is carried in the snapshot and composed
        // nowhere.** Staleness changes every second, and the reading source
        // owns that rule, so `start_reading` asks it once a second and the
        // answer is already current when this loop picks it up. The card is
        // also deliberately outside the weather panel's switch: the two are
        // separate cards and one is not the other's.

        if let Err(e) = sender.send(&message) {
            journal.warn(&format!("send: {e}"));
            continue;
        }
        // Once a minute, not once a second: this is the heartbeat a person
        // reads in `rivet get log` to see that the controller is alive, and at
        // one line a second it would be the only thing in the journal.
        if time != last_logged_minute {
            journal.info(&format!("clock: time={time} date={date}"));
            last_logged_minute = time;
        }
    }
}

/// Take the kiosk socket and arrange the replay.
fn start_kiosk(
    config: &Arc<RwLock<config::Config>>,
) -> Result<Arc<kiosk::Sender>, String> {
    let config = Arc::clone(config);
    let journal = Journal::for_service(SERVICE);
    kiosk::Sender::take(move |sender| {
        let (brightness, saver, theme, weather) = {
            let held = config.read().unwrap_or_else(|e| e.into_inner());
            (
                held.brightness,
                held.screensaver.clone(),
                held.screensaver_theme.clone(),
                held.weather_enabled,
            )
        };
        let replayed = [
            wire::Message::cmd("brightness", brightness.to_string()),
            wire::Message::screensaver(&saver, &theme),
            wire::Message::cmd("show_weather", screensaver::on_off(weather)),
        ];
        for message in &replayed {
            if let Err(e) = sender.send(message) {
                journal.warn(&format!("onconnect: {e}"));
                return;
            }
        }
        journal.info(&format!(
            "onconnect: applied brightness={brightness} screensaver={saver}/{theme} weather={}",
            screensaver::on_off(weather)
        ));
    })
}

/// Resolve the zip code to coordinates, once, on a thread of its own.
///
/// It is a thread rather than an inline call because it reaches somebody else's
/// free service over the internet, and a start that blocked on it would be a
/// display that shows nothing until a stranger's server answers.
///
/// **A cache whose zip matches the configured one is not refreshed**, which is
/// the whole of the invalidation rule.
fn start_geocode(
    config: &Arc<RwLock<config::Config>>,
    weather: &std::sync::mpsc::SyncSender<()>,
) -> std::sync::mpsc::SyncSender<()> {
    let config = Arc::clone(config);
    let weather = weather.clone();
    let journal = Journal::for_service(SERVICE);
    let (nudge, wake) = nudge_channel();
    let spawned = std::thread::Builder::new()
        .name("geocode".to_string())
        .spawn(move || {
            loop {
                resolve_geocode(&config, &weather, &journal);
                // **Waits rather than exits**, because the portal changes the
                // zip code and a resolve that had already run would leave the
                // weather panel on the old coordinates until a restart.
                if wake.recv().is_err() {
                    return;
                }
            }
        });
    if let Err(e) = spawned {
        Journal::for_service(SERVICE).warn(&format!("geocode: no thread: {e}"));
    }
    nudge
}

/// One geocode pass.
fn resolve_geocode(
    config: &Arc<RwLock<config::Config>>,
    weather: &std::sync::mpsc::SyncSender<()>,
    journal: &Journal,
) {
    let (zip, cached) = {
        let held = config.read().unwrap_or_else(|e| e.into_inner());
        (held.zip_code.clone(), held.geo_cache.clone())
    };
    if zip.is_empty() {
        // Not a fault: a fresh install has no zip until somebody sets one, and
        // the weather poller reports its own absence.
        return;
    }
    if let Some(cache) = cached.filter(|cache| cache.zip == zip) {
        journal.info(&format!(
            "geocache: zip={zip} lat={:.4} lng={:.4} (cached)",
            cache.lat, cache.lng
        ));
        // **`.rulings/` R-216.** The freshly-resolved branch below always
        // wakes the weather poller; this branch used not to. A restore (or
        // any other nudge) can land here on a poller that has never had a
        // usable cache to read — a fresh boot's own "geocache not ready" —
        // and without this it waited out its own full interval before
        // trying again, up to twenty minutes for no reason the cache gave.
        let _ = weather.try_send(());
        return;
    }
    let (lat, lng) = match geocode::zip(&zip) {
        Ok(coordinates) => coordinates,
        Err(e) => {
            // The existing cache, if any, is left exactly as it was: a stale
            // coordinate is a working weather panel, and no coordinate is a
            // blank one.
            journal.warn(&format!("geocache: {e}"));
            return;
        }
    };
    let saved = {
        let mut held = config.write().unwrap_or_else(|e| e.into_inner());
        held.geo_cache = Some(config::GeoCache {
            zip: zip.clone(),
            lat,
            lng,
        });
        config::save(&held)
    };
    if let Err(e) = saved {
        journal.warn(&format!("geocache save: {e}"));
    }
    journal.info(&format!("geocache: resolved {zip} → lat={lat:.4} lng={lng:.4}"));
    // Coordinates the weather poller did not have a moment ago. Waking it is
    // what makes a corrected zip code show a temperature straight away rather
    // than twenty minutes later.
    let _ = weather.try_send(());
}

/// A one-slot wake channel.
///
/// **One slot, and a send that never blocks.** A full channel means a nudge is
/// already queued, which is the same outcome — so the request thread that sent
/// it is never held up by a poller that is mid-fetch.
fn nudge_channel() -> (
    std::sync::mpsc::SyncSender<()>,
    std::sync::mpsc::Receiver<()>,
) {
    std::sync::mpsc::sync_channel(1)
}

/// Poll the NWS, and restore the last good reading before the first fetch.
///
/// The restore is what stops a restart drawing a blank panel for the length of
/// the startup delay plus a fetch. Anything older than
/// [`weather::MAX_CACHE_AGE_SECONDS`] is discarded rather than drawn: the panel
/// gives no sign of a reading's age, so a stale temperature is
/// indistinguishable on screen from a current one.
fn start_weather(
    config: &Arc<RwLock<config::Config>>,
    state: &Arc<state::State>,
) -> std::sync::mpsc::SyncSender<()> {
    let config = Arc::clone(config);
    let state = Arc::clone(state);
    let journal = Journal::for_service(SERVICE);
    let (nudge, wake) = nudge_channel();
    let spawned = std::thread::Builder::new()
        .name("weather".to_string())
        .spawn(move || {
            match weather::load_cache(unix_now()) {
                Ok(Some((cached, age))) => {
                    state.set_weather(cached);
                    journal.info(&format!("weather: restored last reading, {age}s old"));
                }
                Ok(None) => {}
                Err(e) => journal.warn(&format!("weather: {e}")),
            }

            std::thread::sleep(WEATHER_STARTUP_DELAY);

            let mut client = weather::Client::new();
            // Whether anything good has ever arrived. Until it has, a failure
            // puts the `unavailable` icon up; after it has, a failure leaves
            // the last good reading alone, because one missed poll does not
            // make yesterday's temperature wrong.
            let mut has_good_data = false;
            // The failure wait, which starts short and doubles up to
            // [`WEATHER_INTERVAL`] so a cold-boot DNS miss recovers in seconds
            // rather than leaving the panel blank for a whole poll cycle. Reset
            // to the floor on every success.
            let mut retry = WEATHER_RETRY_MIN;
            loop {
                // Read on every poll, so a contact saved in the portal is
                // carried by the next request without a restart.
                let (coordinates, contact) = {
                    let held = config.read().unwrap_or_else(|e| e.into_inner());
                    (held.geo_cache.clone(), held.weather_contact.clone())
                };
                let Some(cache) = coordinates else {
                    journal.info("weather: geocache not ready, skipping");
                    let _ = wake.recv_timeout(WEATHER_INTERVAL);
                    continue;
                };
                let wait = match client.fetch(cache.lat, cache.lng, &contact) {
                    Ok(fetched) => {
                        has_good_data = true;
                        retry = WEATHER_RETRY_MIN;
                        journal.info(&format!(
                            "weather: {}°F {}={}°F {} | {}",
                            reading(fetched.temp_f),
                            if fetched.secondary_kind.is_empty() {
                                "secondary"
                            } else {
                                &fetched.secondary_kind
                            },
                            reading(fetched.secondary_f),
                            fetched.condition,
                            fetched.statement
                        ));
                        // Best-effort: the cache is an optimisation, and a
                        // controller that cannot write it works exactly as it
                        // did before there was one.
                        if let Err(e) = weather::save_cache(&fetched, unix_now()) {
                            journal.warn(&format!("weather: {e}"));
                        }
                        state.set_weather(fetched);
                        WEATHER_INTERVAL
                    }
                    Err(e) => {
                        journal.warn(&format!("weather: {e}"));
                        if !has_good_data {
                            state.set_weather(wire::Weather {
                                icon: "unavailable".to_string(),
                                ..wire::Weather::default()
                            });
                        }
                        let wait = retry;
                        retry = (retry * 2).min(WEATHER_INTERVAL);
                        wait
                    }
                };
                let _ = wake.recv_timeout(wait);
            }
        });
    if let Err(e) = spawned {
        Journal::for_service(SERVICE).warn(&format!("weather: no thread: {e}"));
    }
    nudge
}

/// Poll TempMon for the indoor reading.
///
/// **Every failure clears the reading** rather than letting the panel show a
/// stale one as if it were current, and the message is logged only when it
/// changes — a sensor that has been unreachable for a day should not be a day
/// of identical journal lines.
/// Drive the indoor reading source — `crate::addon`, role [`addon::role::INDOOR`].
///
/// **This polled TempMon directly until 2026-09-20.** `src/tempmon.rs` held
/// 404 lines of HTTP, TLS and sensor selection in this repository; Rivet
/// `.rulings/` R-197 D2 moved them to an addon published from the TempMon
/// project, and this thread now asks a library the same question it used to
/// ask a service. What a person sees did not change.
///
/// **A thread of its own, for `start_reading`'s reason** and not the one the
/// poller it replaced had: it is no longer that an HTTP fetch blocks, it is
/// that *somebody else's code* blocks, in this process, for as long as it
/// likes. Containing that is this thread's whole job.
///
/// **Every failure clears the reading** rather than letting the panel show a
/// stale one as if it were current, and a message is journalled only when it
/// changes — a sensor unreachable for a day should not be a day of identical
/// lines.
fn start_indoor(
    config: &Arc<RwLock<config::Config>>,
    state: &Arc<state::State>,
    description: Arc<RwLock<Option<rivetos_addon::Description>>>,
) -> std::sync::mpsc::SyncSender<()> {
    let config = Arc::clone(config);
    let state = Arc::clone(state);
    let journal = Journal::for_service(SERVICE);
    let (nudge, wake) = nudge_channel();
    let spawned = std::thread::Builder::new()
        .name("indoor".to_string())
        .spawn(move || {
            let source = match addon::Source::open(addon::role::INDOOR) {
                Ok(source) => {
                    journal.info(&format!("indoor: opened {}", source.summary()));
                    // **Told to the portal exactly once, here**, for
                    // `start_reading`'s reason: an addon cannot be reloaded
                    // without restarting the process that loaded it, so this
                    // is the only place its `Description` can become known.
                    *description.write().unwrap_or_else(|e| e.into_inner()) =
                        Some(source.description().clone());
                    source
                }
                Err(addon::NotOpened::Absent(sentence)) => {
                    // **The ordinary state of a machine with no indoor source
                    // installed.** No column, one line, and the thread ends:
                    // an addon is loaded for the life of the process, so
                    // nothing here could pick one up later and a retry loop
                    // would only repeat this line.
                    journal.info(&sentence);
                    state.clear_indoor();
                    return;
                }
                Err(addon::NotOpened::Refused(sentence)) => {
                    journal.warn(&sentence);
                    state.clear_indoor();
                    return;
                }
            };

            // What was last sent, so a change is what triggers a re-send.
            let mut configured: Option<(String, String, String)> = None;
            // **And what material went with it.** The three PEMs are not among
            // the settings, so a reinstalled certificate would otherwise look
            // like no change at all — and `cargo rivet secrets` reaches no
            // portal and rings no bell, which is why this is swept for rather
            // than waited on.
            let mut sent_material: std::collections::BTreeMap<String, String> =
                std::collections::BTreeMap::new();
            let mut material_read_at = std::time::Instant::now();
            let mut last_complaint = String::new();
            // One line when the column changes, not one a minute.
            let mut last_note = String::new();

            // The first pass happens at once.
            let mut wait = Duration::ZERO;
            loop {
                let nudged = wake.recv_timeout(wait).is_ok();
                wait = INDOOR_INTERVAL;

                let (enabled, url, name_result, sensor) = {
                    let held = config.read().unwrap_or_else(|e| e.into_inner());
                    (
                        held.indoor_enabled,
                        held.tempmon_url.clone(),
                        // `.rulings/` R-219: which name is needed, if any,
                        // depends on the URL — see
                        // `config::effective_tempmon_name`'s own doc for its
                        // three cases and `Err`'s meaning in the two that can
                        // refuse.
                        config::effective_tempmon_name(&held.tempmon_url, &held.tempmon_name),
                        held.indoor_sensor.clone(),
                    )
                };

                if !enabled {
                    // **The host's decision about its own screen**, which is
                    // why the addon has no `enabled` field to be told.
                    state.clear_indoor();
                    configured = None;
                    continue;
                }

                let verify_as = match name_result {
                    Ok(name) => name,
                    Err(message) => {
                        // **Refused rather than guessed, and never sent to
                        // the addon at all.** A resolved name that could not
                        // be determined must not reach `Configure` as an
                        // empty `verify_as` that TLS would then verify
                        // against the URL's own bare host instead — which
                        // for a loopback URL is exactly the silent break
                        // this ruling exists to stop.
                        state.clear_indoor();
                        configured = None;
                        complain(&journal, &mut last_complaint, "indoor", &message);
                        continue;
                    }
                };

                let wanted = (url.clone(), verify_as.clone(), sensor.clone());
                let settings_changed = configured.as_ref() != Some(&wanted);
                if nudged || settings_changed || material_read_at.elapsed() >= MATERIAL_RECHECK {
                    material_read_at = std::time::Instant::now();
                    let material = addon::material(source.material_names());
                    if nudged || settings_changed || material != sent_material {
                        let named = material.len();
                        match source.configure(
                            addon::indoor_settings(&url, &verify_as, &sensor),
                            material.clone(),
                        ) {
                            Ok(()) => {
                                configured = Some(wanted);
                                sent_material = material;
                                last_complaint.clear();
                                journal.info(&format!(
                                    "indoor: configured with {named} material file(s) — \
                                     {url} sensor={}",
                                    if sensor.is_empty() { "(first)" } else { &sensor }
                                ));
                            }
                            Err(e) => {
                                // **Recorded, so this is not a configure per
                                // pass against a setting that will be refused
                                // just as firmly next time.** What makes it
                                // try again is the operator changing
                                // something.
                                configured = Some(wanted);
                                sent_material = material;
                                complain(&journal, &mut last_complaint, "indoor", &e);
                            }
                        }
                    }
                }

                match source.poll(unix_now().saturating_mul(1_000)) {
                    Ok(polled) => {
                        if !polled.complaint.is_empty() {
                            complain(&journal, &mut last_complaint, "indoor", &polled.complaint);
                        } else {
                            last_complaint.clear();
                        }

                        let card = addon::indoor_card(&polled, &sensor);
                        if card.online {
                            // **Keyed on the reading and not on its age**, the
                            // trap `start_reading` walked into: an age changes
                            // every pass by definition, so keying a journal on
                            // it writes a line per pass for a sensor that
                            // reports once a minute.
                            let note = format!(
                                "{}/{}/{}",
                                card.label,
                                self::reading(card.temp_f),
                                self::reading(card.humidity_pct)
                            );
                            if note != last_note {
                                journal.info(&format!(
                                    "indoor: {} {}°F {}%",
                                    card.label,
                                    self::reading(card.temp_f),
                                    self::reading(card.humidity_pct)
                                ));
                                last_note = note;
                            }
                            state.set_indoor(card);
                        } else {
                            // `online` false is *do not draw this*: a dead
                            // sensor and a perfectly stable room produce the
                            // same flat line.
                            state.clear_indoor();
                            last_note.clear();
                        }

                        // **The addon decides the cadence, within this host's
                        // bounds.** It asks for a minute after a good reading
                        // and ten seconds after a failure, and clamping is
                        // what stops an addon asking to be called every
                        // millisecond from being obeyed.
                        wait = Duration::from_millis(polled.call_again_in_ms)
                            .clamp(Duration::from_secs(5), INDOOR_INTERVAL);
                    }
                    Err(e) => {
                        state.clear_indoor();
                        last_note.clear();
                        complain(&journal, &mut last_complaint, "indoor", &e.to_string());
                    }
                }
            }
        });
    if let Err(e) = spawned {
        Journal::for_service(SERVICE).warn(&format!("indoor: no thread: {e}"));
    }
    nudge
}

/// Drive the corner reading card's source — `crate::addon`, role
/// [`addon::role::READING`].
///
/// **A thread of its own, because *somebody else's code* blocks**, in this
/// process, for as long as it likes — on the pass where it reaches the network,
/// a whole HTTP exchange. `rivetos_addon::host::Loaded::call`
/// cannot contain a hang and says so — the caller's job is to make an addon
/// that never returns cost a card rather than the clock on the wall
/// (`.rulings/` R-198 D3). This is that containment and it is the whole of it.
///
/// **No waits live here.** The three a poller would balance — the source's
/// cadence, a routine backoff, a slow retry after a refusal that will not fix
/// itself — belong to the source, which is the only side that knows what a 429
/// meant or whether a name failed to resolve because the network was not up
/// yet. This asks once a second and uses what comes back, which is what
/// [`addon::POLL_INTERVAL`] argues: a poll is *what do you say now*, not *go
/// and fetch*, and **how stale a reading is changes every second without
/// anything new arriving**.
///
/// The nudge channel means *a person saved something*, and what it does here
/// is re-send `Configure` — which is how a credential or a setting typed into
/// the portal takes effect at once instead of at the addon's next fetch.
///
/// **The settings sent are the addon's own, as saved**, completed from
/// `Describe`'s defaults by [`addon::settings_for`]; nothing here reads one.
fn start_reading(
    config: &Arc<RwLock<config::Config>>,
    state: &Arc<state::State>,
    description: Arc<RwLock<Option<rivetos_addon::Description>>>,
) -> std::sync::mpsc::SyncSender<()> {
    let config = Arc::clone(config);
    let state = Arc::clone(state);
    let journal = Journal::for_service(SERVICE);
    let (nudge, wake) = nudge_channel();
    let spawned = std::thread::Builder::new()
        .name("reading".to_string())
        .spawn(move || {
            let source = match addon::Source::open(addon::role::READING) {
                Ok(source) => {
                    journal.info(&format!("addon: opened {}", source.summary()));
                    // **Told to the portal exactly once, here.** This is the
                    // only place `Source::open` is ever called — an addon
                    // cannot be reloaded without restarting the process that
                    // loaded it — so this is also the only place its
                    // `Description` can become known, and everything the
                    // portal renders about this addon starts from this write.
                    *description.write().unwrap_or_else(|e| e.into_inner()) =
                        Some(source.description().clone());
                    source
                }
                Err(addon::NotOpened::Absent(sentence)) => {
                    // **The ordinary state of a machine nobody installed one
                    // on.** No card, one line, and the thread ends: an addon
                    // is loaded for the life of the process (there is no
                    // `dlclose`), so nothing here could pick one up later
                    // anyway and a retry loop would only repeat this line.
                    journal.info(&sentence);
                    state.clear_reading();
                    return;
                }
                Err(addon::NotOpened::Refused(sentence)) => {
                    journal.warn(&sentence);
                    state.clear_reading();
                    return;
                }
            };

            // What was last sent, so a change is what triggers a re-send.
            let mut configured: Option<serde_json::Map<String, serde_json::Value>> = None;
            // **And what material went with it**, because a credential is not
            // among the settings and a replaced one would otherwise look like
            // no change at all.
            //
            // The portal's nudge covers a person typing one in. This covers
            // the OTHER documented way to install one —
            // `rivet apply install-secret aura-controller <name>`, the name
            // being the addon's own — which reaches no portal and rings no
            // bell, so the portal cannot be the only way a credential arrives.
            //
            // **Compared, never sent again unchanged**, and that is the part
            // that matters rather than the part that is convenient: the addon
            // treats a `Configure` as a new credential and clears the backoff
            // it has earned, so a host that re-sent identical material every
            // half minute would hold a failing source at its two-second
            // first-failure retry for ever. That is how a client gets banned
            // rather than throttled, which is the sentence the addon's own
            // throttle handling is written under.
            let mut sent_material: std::collections::BTreeMap<String, String> =
                std::collections::BTreeMap::new();
            let mut material_read_at = std::time::Instant::now();
            // A complaint is journalled once per distinct message, so a source
            // that has been down for an hour costs one line rather than three
            // thousand six hundred.
            let mut last_complaint = String::new();
            // The same discipline for what is going right: one line when the
            // card changes, not one a second.
            let mut last_note = String::new();
            // **The unit the last reading carried**, which is what a card with
            // no reading is labelled with — `addon::card`'s doc says why it is
            // not a setting. Empty until the first reading of this run.
            let mut last_unit = String::new();

            // The first pass happens at once.
            let mut wait = Duration::ZERO;
            loop {
                let nudged = wake.recv_timeout(wait).is_ok();
                wait = addon::POLL_INTERVAL;

                let (enabled, wanted) = {
                    let held = config.read().unwrap_or_else(|e| e.into_inner());
                    (
                        held.reading_enabled,
                        addon::settings_for(&source.description().fields, &held.reading_settings),
                    )
                };

                if !enabled {
                    // **The host's decision about its own screen**, which is
                    // why the addon has no `enabled` field to be told. It
                    // keeps its state and its cadence; nothing is drawn.
                    state.clear_reading();
                    configured = None;
                    continue;
                }

                let settings_changed = configured.as_ref() != Some(&wanted);
                // Reading the credential is a file read, so it is done when
                // there is a reason to and on a slow sweep for the one case
                // that gives no signal at all.
                if nudged
                    || settings_changed
                    || material_read_at.elapsed() >= MATERIAL_RECHECK
                {
                    material_read_at = std::time::Instant::now();
                    let material = addon::material(source.material_names());
                    if nudged || settings_changed || material != sent_material {
                        let named = material.len();
                        let fields = wanted.len();
                        match source.configure(wanted.clone(), material.clone()) {
                            Ok(()) => {
                                configured = Some(wanted);
                                sent_material = material;
                                last_complaint.clear();
                                // Counts and never values: a setting can name
                                // an account, and the journal is not where one
                                // is kept.
                                journal.info(&format!(
                                    "addon: configured with {fields} setting(s) and {named} \
                                     secret(s)"
                                ));
                            }
                            Err(e) => {
                                // **Not unloaded, and not retried on a timer
                                // either.** The addon stays open and says the
                                // same thing on every poll below, which is
                                // where a person reads it; what makes it try
                                // again is the operator changing something,
                                // which is a nudge or new material. Recording
                                // the attempt is what stops this becoming a
                                // configure per second against a setting that
                                // will be refused just as firmly next time.
                                configured = Some(wanted);
                                sent_material = material;
                                complain(&journal, &mut last_complaint, "reading", &e);
                            }
                        }
                    }
                }

                match source.poll(unix_now().saturating_mul(1_000)) {
                    Ok(polled) => {
                        let card =
                            addon::card(&polled, unix_now().saturating_mul(1_000), &last_unit);
                        last_unit = addon::unit_after(&last_unit, &card);
                        // **The value is never logged**, a rule the addon keeps
                        // on its own side too: the controller's log is not the
                        // place a series of readings accumulates.
                        //
                        // **And the AGE is not what decides whether to write a
                        // line**, which is the trap this loop walked into on
                        // its first run against `rivet-06f5c`: the age changes
                        // every second by definition, so keying the journal on
                        // it produced a record per second — 86,400 a day, from
                        // a source that publishes twelve times an hour. What
                        // is keyed on is the reading's OWN instant plus the
                        // status and the arrow, so a line is written when
                        // something actually happened: a new measurement
                        // arrived, or a held one crossed into stale.
                        //
                        // The age is still printed, because it is the first
                        // thing a person diagnosing a blank card wants — it is
                        // just printed on the lines that were going to be
                        // written anyway.
                        let taken_at_ms = polled.reading.as_ref().map_or(0, |r| r.taken_at_ms);
                        let note = format!("{}/{}/{taken_at_ms}", card.status, card.trend);
                        if note != last_note {
                            journal.info(&format!(
                                "addon: {} trend={} age={}s",
                                card.status,
                                if card.trend.is_empty() { "-" } else { &card.trend },
                                card.age_seconds.unwrap_or(-1)
                            ));
                            last_note = note;
                        }
                        if polled.complaint.is_empty() {
                            last_complaint.clear();
                        } else {
                            complain(&journal, &mut last_complaint, "reading", &polled.complaint);
                        }
                        state.set_reading(card);
                    }
                    Err(e) => {
                        // The addon could not answer at all — a panic caught
                        // at the boundary, or bytes this build cannot read.
                        // **The last card stays up**: it is still the last
                        // thing known, and it ages out on its own because the
                        // age is computed from the instant the sensor took it.
                        complain(&journal, &mut last_complaint, "reading", &e);
                    }
                }
            }
        });
    if let Err(e) = spawned {
        Journal::for_service(SERVICE).warn(&format!("reading: no thread: {e}"));
    }
    nudge
}

/// Fire the schedule's directives, on the minute.
///
/// **The sleep is recomputed every pass** rather than run off a ticker, so the
/// thread re-aligns itself after a clock step instead of drifting a minute off
/// for ever. The extra half-second lands the evaluation safely inside the new
/// minute: waking at exactly `:00` and reading the clock can still see `:59`,
/// which would fire the previous minute's events twice and the new minute's
/// never.
fn start_schedule(config: &Arc<RwLock<config::Config>>, sender: &Arc<kiosk::Sender>) {
    let config = Arc::clone(config);
    let sender = Arc::clone(sender);
    let journal = Journal::for_service(SERVICE);
    let spawned = std::thread::Builder::new()
        .name("schedule".to_string())
        .spawn(move || {
            loop {
                let wait = match clock::Local::now() {
                    Ok(now) => now.seconds_to_next_minute(),
                    Err(e) => {
                        journal.warn(&format!("schedule: {e}"));
                        60
                    }
                };
                std::thread::sleep(
                    Duration::from_secs(wait) + Duration::from_millis(500),
                );

                let Ok(now) = clock::Local::now() else {
                    // Already journalled above on the way in; a second line per
                    // minute would bury everything else.
                    continue;
                };
                let events = {
                    let held = config.read().unwrap_or_else(|e| e.into_inner());
                    held.schedule.clone()
                };
                for action in schedule::due(&events, &now) {
                    let message = match schedule::directive(&action) {
                        Ok(message) => message,
                        Err(e) => {
                            journal.warn(&format!("scheduled: {e}"));
                            continue;
                        }
                    };
                    let label = format!("{}={}", message.cmd, message.value);
                    if !sender.is_connected() {
                        journal.info(&format!(
                            "sent cmd: {label} (renderer not connected — dropped)"
                        ));
                        continue;
                    }
                    match sender.send(&message) {
                        Ok(()) => journal.info(&format!("sent cmd → {label}")),
                        Err(e) => journal.warn(&format!("sent cmd: {label}: {e}")),
                    }
                }
            }
        });
    if let Err(e) = spawned {
        Journal::for_service(SERVICE).warn(&format!("schedule: no thread: {e}"));
    }
}

/// Journal a reading source's complaint **once per distinct message**.
///
/// A source that has been unhappy for an hour costs one line rather than three
/// thousand six hundred — which is a real number now that the poll is once a
/// second rather than once every five minutes, and is why this stayed when the
/// waits it used to carry went to the addon.
///
/// **It no longer says when the next attempt is**, because this side no longer
/// decides: the source states its own cadence and this program asks inside it.
/// A sentence promising a retry in thirty minutes while the loop ran every
/// second would be the kind of log line that sends a reader to the wrong file.
///
/// The state is passed in rather than captured, because the success path also
/// clears it and one closure cannot own what two arms write.
///
/// `panel` names which one this complaint is FROM — `.rulings/` R-216: this
/// is shared by both the reading-card loop and the indoor (TempMon) loop, and
/// a fixed prefix here once meant every indoor complaint was journalled under
/// the reading card's name, naming the wrong panel entirely.
fn complain(journal: &Journal, last: &mut String, panel: &str, message: &str) {
    if message != last {
        journal.warn(&complaint_line(panel, message));
        last.clear();
        last.push_str(message);
    }
}

/// [`complain`]'s own line, pulled out because it is the part worth proving:
/// the panel that is unhappy is the panel the line names.
#[must_use]
fn complaint_line(panel: &str, message: &str) -> String {
    format!("{panel}: {message}")
}

/// An optional reading, for a log line, where absent prints as `-`.
///
/// Go prints absent as `0`, which in a temperature log is a reading. A dash is
/// the same information without the ambiguity, and nothing parses these lines.
fn reading(value: Option<f64>) -> String {
    value.map_or_else(|| "-".to_string(), |v| format!("{v:.0}"))
}

/// Seconds since the epoch, for the weather cache's age arithmetic.
///
/// Falls back to zero, which makes every cache look impossibly old and is
/// therefore the safe direction: a cache that is ignored costs one fetch, and
/// one that is wrongly trusted puts an old temperature on a wall.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    // ---- complaint_line — `.rulings/` R-216 --------------------------------

    /// **The defect itself, pinned directly.** Before this, every complaint —
    /// the reading card's and indoor's alike — was journalled under the
    /// reading card's name, so an indoor (TempMon) failure named the wrong
    /// panel entirely.
    #[test]
    fn each_panels_complaint_names_that_panel_and_no_other() {
        assert_eq!(complaint_line("indoor", "tempmon: connection refused"), "indoor: tempmon: connection refused");
        assert_eq!(complaint_line("reading", "no account configured"), "reading: no account configured");
    }

    // ---- resolve_geocode — `.rulings/` R-216 -------------------------------

    fn geo_config(zip: &str, cache: Option<config::GeoCache>) -> Arc<RwLock<config::Config>> {
        Arc::new(RwLock::new(config::Config {
            zip_code: zip.to_string(),
            geo_cache: cache,
            ..config::Config::default()
        }))
    }

    /// **The defect itself.** A restore (or any other nudge) that lands on a
    /// zip code the cache already answers must still wake the weather
    /// poller — the freshly-resolved branch always did; this is the branch
    /// that did not, and a poller stuck on "geocache not ready" had no other
    /// way to learn the cache was ready after all.
    #[test]
    fn a_cache_hit_still_wakes_the_weather_poller() {
        let config = geo_config(
            "20500",
            Some(config::GeoCache { zip: "20500".to_string(), lat: 38.9, lng: -77.0 }),
        );
        let (weather, woken) = nudge_channel();
        let journal = Journal::for_service("test");

        resolve_geocode(&config, &weather, &journal);

        assert!(
            woken.try_recv().is_ok(),
            "a zip the cache already answers must still wake the weather poller"
        );
    }

    /// An empty zip code is a fresh install, not a fault — nothing to
    /// resolve and nothing to wake a poller about.
    #[test]
    fn an_empty_zip_code_wakes_nothing() {
        let config = geo_config("", None);
        let (weather, woken) = nudge_channel();
        let journal = Journal::for_service("test");

        resolve_geocode(&config, &weather, &journal);

        assert!(woken.try_recv().is_err(), "there is nothing to wake a poller about yet");
    }
}
