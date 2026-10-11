//! **The portal — HTTPS, and it verifies its caller** — `internal/portal`,
//! ported, including the TLS half a stopped agent left in the Go tree.
//!
//! # What replaced the password
//!
//! The portal used to authenticate with `portal_password`: a shared secret,
//! stored in cleartext in `config.json`, compared against a form field, and
//! carried afterwards in a cookie holding an HMAC of that same secret. Anything
//! that could read the config could mint a session, and the default shipped in
//! the source.
//!
//! `R-152.1` retires it rather than wrapping TLS around it. What authenticates
//! a caller now is **a certificate chaining to the fleet authority, checked
//! during the handshake, before a byte of HTTP is parsed.** There is no login
//! page, no session cookie and no shared secret anywhere in this module — and
//! no per-route check either, because a second answer to a question the
//! handshake has already answered is exactly what was retired.
//!
//! # Two connections, not one
//!
//! The operator reaches `https://<host>/aura/…`; `rivet-proxyd` terminates
//! **that** session and verifies the operator's certificate, then opens a
//! second TLS connection here presenting its own fleet-issued certificate.
//! What this program verifies is **the proxy** (`ADR-046` §4.2).
//!
//! It deliberately does not verify the operator and must not learn how: path
//! routing means reading the request line, reading the request line means
//! terminating TLS, and whoever terminates TLS is who the client certificate
//! was presented to. `/aura` as an address and Aura checking your certificate
//! are mutually exclusive, and `R-152` chose the address.
//!
//! # Why *it came from 127.0.0.1* is not the check
//!
//! `ADR-024` §8.1 is explicit that the application sandbox is not enforced, so
//! any local process can open a loopback socket and the peer address proves
//! nothing. *The caller holds a key chaining to the fleet authority* is a
//! claim, and it is the same one the proxy's own callers make. **Nothing in
//! this module looks at a remote address.**
//!
//! # There is no plaintext branch
//!
//! A build that fell back to HTTP when the files were missing would put the
//! portal — which writes the schedule and the display state, and validates a
//! reading source's settings against its own stated bounds before sending them
//! anywhere — back on the network in the clear on exactly the machines where
//! somebody forgot to install a certificate. **Missing files are a refusal to
//! start that names them**, and the refusal happens before a socket is served
//! on, because a server that discovered its key was unreadable at the first
//! request would report that to whoever connected — the one party that must not
//! be told.
//!
//! # The socket is handed over, which is what makes the route exist
//!
//! `ADR-046` §9.4 named this application as the live case of *the platform
//! failing to be a platform*: the Go controller declares `network = "none"` and
//! `listen = []` and then serves HTTP on 8080 by opening its own socket, so
//! there is no port for the proxy to route to and no gate in front of it. This
//! module binds nothing. The supervisor opens 8080 and hands the descriptor
//! over, which is what puts `/aura/` in the routing table.

mod form;
mod html;
mod schema;

use std::io::Write as _;
use std::sync::{Arc, RwLock};

use rivet_sdk::journal::Journal;
use rivet_sdk::serve::{self, Bounds, Limit};
use rivetos_addon::Description;

use crate::config::{self, Config, ScheduledAction, ScheduledEvent};
use crate::{addon, kiosk, screensaver, wire};

/// The listening socket's name.
///
/// **Written `tcp:8080` in the manifest and arriving as `tcp-8080`**, because
/// `LISTEN_FDNAMES` is colon-separated and `tcp:8080` would reach the program
/// as two names. TempMon's adapter carries the same hyphen and the same note.
pub const LISTEN_NAME: &str = "tcp-8080";

/// The three files, by the names the rest of the fleet uses for them.
///
/// `rivet-proxyd` calls its own copies `tls-cert`, `tls-key` and `tls-root`,
/// and so do the other applications the fleet serves over TLS, TempMon among
/// them. **One name for one file across the machine**, so an operator
/// installing three secrets into several applications types the same word
/// each time.
const CERTIFICATE: &str = "tls-cert";
/// Its private key.
const KEY: &str = "tls-key";
/// The authority a caller must chain to.
const AUTHORITY: &str = "tls-root";

/// Who to wake when a setting changes.
///
/// **A saved credential must take effect now**, not at the next poll — which
/// for a reading source's credential could be half an hour away. Each of these
/// is a one-slot channel the matching poller waits on instead of sleeping.
#[derive(Clone)]
pub struct Nudges {
    /// Re-geocode and then re-fetch the weather.
    pub weather: std::sync::mpsc::SyncSender<()>,
    /// Re-read TempMon.
    pub indoor: std::sync::mpsc::SyncSender<()>,
    /// Re-configure the corner reading card's source and poll it at once.
    pub reading: std::sync::mpsc::SyncSender<()>,
}

impl Nudges {
    /// Wake a poller, and do nothing if it is already awake.
    ///
    /// A full channel means a nudge is already queued, which is the same
    /// outcome — so this never blocks the request that triggered it.
    fn wake(channel: &std::sync::mpsc::SyncSender<()>) {
        let _ = channel.try_send(());
    }
}

/// Everything a handler needs.
struct Portal {
    config: Arc<RwLock<Config>>,
    sender: Arc<kiosk::Sender>,
    nudges: Nudges,
    journal: Journal,
    /// **What `Describe` answered, read once per request.**
    ///
    /// `None` until the reading card's addon has opened and answered — a
    /// brief window at startup — or for the rest of the process's life if no
    /// addon of that class is installed at all. Either way the dashboard has a
    /// single, honest thing to say: *no reading source is connected*, rather
    /// than a form built from fields that do not exist.
    ///
    /// Set exactly once, by `main::start_reading` after its addon answers —
    /// this module never opens one itself, because an addon cannot be
    /// reloaded without restarting the process that loaded it (`crate::addon`
    /// argues why), so there is nothing for a second opener to do but race the
    /// first.
    description: Arc<RwLock<Option<Description>>>,
    /// The same, for whatever is filling [`crate::addon::role::INDOOR`].
    ///
    /// **One per role and not one shared**, because each addon describes its
    /// own fields and a machine may well have one role filled and the other
    /// not. Set exactly once, by `main::start_indoor`.
    indoor_description: Arc<RwLock<Option<Description>>>,
    /// **What the background pack described**, for the two screensaver
    /// pickers and every screensaver name on the dashboard — T7. `None` on a
    /// machine with no pack installed, which is the case the portal renders as
    /// no picker at all rather than as an empty one, with a sentence in its
    /// place naming the library to install.
    ///
    /// Set exactly once, by `main`, before this server starts. Unlike the two
    /// above it is not filled by a polling thread: a catalogue is asked for
    /// once and cannot change for the life of the process.
    backgrounds: Arc<RwLock<Option<Description>>>,
}

/// **Everything [`prepare`] produces**: the listening socket, already
/// claimed, and the TLS configuration, already validated. Nothing about an
/// add-on is reachable from either — see [`prepare`]'s own note on why that
/// separation exists.
pub struct Prepared {
    listener: std::net::TcpListener,
    server_config: Arc<rustls::ServerConfig>,
}

/// **Claim the listening socket and load and validate the TLS material —
/// nothing else.**
///
/// # Call this before opening any add-on
///
/// `main` used to open the background pack and start the indoor and reading
/// card threads — each of which writes a `rivetos_addon::trial` note before it
/// touches a shared object — and call [`start`] last. On a machine missing
/// an installed `tls-cert`, that meant every one of those loads had already
/// begun, mid-flight and unsettled, by the time this function's old, unsplit
/// form finally discovered the certificate was missing and returned an
/// error: `run` died with one or more trial notes still on disk, each of
/// which then made the next start SKIP that add-on too, forever, long after
/// the certificate arrived.
///
/// Splitting the failure-prone half from the half that opens a thread lets
/// `main` call this FIRST. Every way this can fail is checked here, before an
/// add-on has been asked to load anything, so a missing certificate is once
/// again just a certificate to install — never a stale note to find and
/// delete by hand first.
///
/// # Errors
///
/// A sentence, for a socket the supervisor did not hand over, a descriptor
/// already taken, or TLS files that are missing or unusable. Every one of
/// them is a refusal to start.
pub fn prepare() -> Result<Prepared, String> {
    let socket = rivet_sdk::listen::socket(LISTEN_NAME).ok_or_else(|| {
        format!(
            "portal: this machine handed no socket named {LISTEN_NAME}. The manifest must \
             declare `listen = [\"tcp:8080\"]`; this program does not bind one of its own"
        )
    })?;
    let listener = socket.into_tcp_listener().ok_or_else(|| {
        format!("portal: the descriptor for {LISTEN_NAME} was already taken in this process")
    })?;

    let directory = rivet_sdk::app::data().map_err(|e| format!("portal: {e}{}", install_hint()))?;
    let server_config = tls_server_config(&directory)?;

    Ok(Prepared {
        listener,
        server_config,
    })
}

/// The TLS half of [`prepare`], split out so a test can point it at a
/// directory it made rather than at `RIVET_APP_DATA` — the same split
/// [`rivet_sdk::serve::Tls::in_directory`] exists for, one level up.
fn tls_server_config(directory: &std::path::Path) -> Result<Arc<rustls::ServerConfig>, String> {
    let tls = rivet_sdk::serve::Tls::in_directory(directory, CERTIFICATE, KEY)
        // **`requiring`, never `asking_for`.** The latter serves whoever
        // declines to present a certificate, which is an unauthenticated
        // administrative surface on a LAN and the exact failure this whole
        // arrangement exists to avoid.
        .requiring_a_caller_certificate_from(AUTHORITY);
    Ok(Arc::new(
        tls.server_config()
            .map_err(|e| format!("portal: {e}{}", install_hint()))?,
    ))
}

/// **Serve, from what [`prepare`] already claimed and validated.**
///
/// Everything that could fail to start already has — see [`prepare`] — so the
/// only error left here is the accept thread itself refusing to spawn.
///
/// # Errors
///
/// A sentence, when the accept thread could not be started.
#[allow(clippy::too_many_arguments)] // `prepared` plus the same six `start` always took
pub fn start(
    prepared: Prepared,
    config: &Arc<RwLock<Config>>,
    sender: &Arc<kiosk::Sender>,
    nudges: Nudges,
    description: Arc<RwLock<Option<Description>>>,
    indoor_description: Arc<RwLock<Option<Description>>>,
    backgrounds: Arc<RwLock<Option<Description>>>,
    journal: &Journal,
) -> Result<(), String> {
    let Prepared {
        listener,
        server_config,
    } = prepared;

    let portal = Arc::new(Portal {
        config: Arc::clone(config),
        sender: Arc::clone(sender),
        nudges,
        journal: Journal::for_service(crate::SERVICE),
        description,
        indoor_description,
        backgrounds,
    });
    let bounds = Bounds::default();
    let limit = Limit::of(bounds.connections);

    std::thread::Builder::new()
        .name("portal-accept".to_string())
        .spawn(move || {
            for incoming in listener.incoming() {
                let Ok(stream) = incoming else {
                    continue;
                };
                // **Claimed before the thread exists**, because claiming after
                // spawning bounds nothing.
                let Some(slot) = limit.claim() else {
                    // At the ceiling. The connection is dropped rather than
                    // queued: a caller that waits is a caller holding a
                    // descriptor, which is the resource being protected.
                    continue;
                };
                let serving = Arc::clone(&portal);
                let config = Arc::clone(&server_config);
                let bounds = bounds.clone();
                let spawned = std::thread::Builder::new()
                    .name("portal".to_string())
                    .spawn(move || {
                        let _slot = slot;
                        serving.serve(stream, &config, &bounds);
                    });
                if spawned.is_err() {
                    // The slot's guard is dropped with the closure that never
                    // ran, so the count is right either way.
                    portal.journal.warn("portal: a connection thread could not be started");
                }
            }
        })
        .map_err(|e| format!("portal: the accept thread could not be started: {e}"))?;

    journal.info(&format!(
        "portal: serving HTTPS on the handed socket named {LISTEN_NAME}; a caller \
         certificate from the fleet authority is REQUIRED"
    ));
    Ok(())
}

impl Portal {
    /// One connection: handshake, one request, one answer, close.
    ///
    /// **`Connection: close` and one request per connection.** The Go portal
    /// keeps connections alive; the proxy in front of this closes every hop in
    /// both directions anyway (`ADR-046` §8), and a page load of a handful of
    /// requests against a sixteen-connection ceiling does not need pipelining.
    fn serve(
        &self,
        stream: std::net::TcpStream,
        config: &Arc<rustls::ServerConfig>,
        bounds: &Bounds,
    ) {
        // **The deadline goes on before the handshake**, which is the point of
        // it: until the handshake completes the peer is nobody.
        if let Err(e) = bounds.apply_to(&stream) {
            self.journal.warn(&format!("portal: no deadline on a connection: {e}"));
            return;
        }
        // **One absolute deadline covers the handshake and the head
        // together**, carried through `read_head_by_deadline` below rather
        // than trusted to the flat per-read timeout `apply_to` just set — a
        // peer that sends one byte just inside that timeout, repeatedly,
        // would otherwise hold the connection well past `head_patience`. A
        // second handle on the SAME socket lets the deadline-bound phase set
        // its own shrinking read timeout without fighting the borrow `tls`
        // holds on `stream` for the rest of the connection.
        let deadline = std::time::Instant::now() + bounds.head_patience;
        let clock = match stream.try_clone() {
            Ok(clock) => clock,
            Err(e) => {
                self.journal.warn(&format!("portal: no deadline clock on a connection: {e}"));
                return;
            }
        };
        let connection = match rustls::ServerConnection::new(Arc::clone(config)) {
            Ok(connection) => connection,
            Err(e) => {
                self.journal.warn(&format!("portal: TLS session: {e}"));
                return;
            }
        };
        let mut tls = rustls::StreamOwned::new(connection, stream);

        let (head, already) = match serve::read_head_by_deadline(&mut tls, bounds, deadline, |d| {
            clock.set_read_timeout(Some(d))
        }) {
            Ok(parsed) => parsed,
            Err(refusal) => {
                // A caller that presented no acceptable certificate never
                // reaches here — the handshake refused it — so this is a
                // malformed request from somebody the fleet authority vouched
                // for, and it is worth a line.
                if let Some(response) = refusal.response() {
                    let _ = tls.write_all(response.as_bytes());
                }
                self.journal.warn(&format!("portal: {}", refusal.why()));
                return;
            }
        };
        // **Back to the flat per-read patience `apply_to` set**, now that the
        // deadline-bound phase is over. A body already declared by
        // `Content-Length` is bounded by its own promise, not by how long the
        // handshake and head happened to take — preserving that separate
        // policy is `read_head_by_deadline`'s own caller's job, not its own.
        if let Err(e) = clock.set_read_timeout(Some(bounds.head_patience)) {
            self.journal.warn(&format!("portal: no deadline on a connection: {e}"));
            return;
        }

        let body = if head.method == "POST" {
            match serve::read_body(&mut tls, &head, &already) {
                Ok(body) => body,
                Err(refusal) => {
                    if let Some(response) = refusal.response() {
                        let _ = tls.write_all(response.as_bytes());
                    }
                    return;
                }
            }
        } else {
            Vec::new()
        };

        let response = self.route(&head, &body);
        let _ = tls.write_all(&response);
        let _ = tls.flush();
    }

    /// Which handler, by method and path.
    ///
    /// The paths are what reaches this program **after `rivet-proxyd` strips
    /// its prefix**, so they are the same whether the portal was reached
    /// through the front door or on its own port.
    /// **Bytes and not a `String`, because a font is not text.**
    ///
    /// Every other response here is UTF-8 and was one until the portal started
    /// serving Barlow from its own binary; a `.ttf` pushed through a `String`
    /// is either a panic or a mangled file, depending on how it is built. The
    /// text routes still build a `String` and hand back its bytes, which is
    /// the same work they always did.
    fn route(&self, head: &serve::Head, body: &[u8]) -> Vec<u8> {
        let path = form::path_of(&head.target);
        let query = form::Form::parse(form::query_of(&head.target));
        // A `POST` body is a form; a `GET`'s parameters are its query string.
        let submitted = if head.method == "POST" {
            form::Form::parse(&String::from_utf8_lossy(body))
        } else {
            form::Form::parse("")
        };

        match (head.method.as_str(), path) {
            ("GET", "/") => self.dashboard(&query).into_bytes(),
            ("GET", "/static/style.css") => {
                page(200, "OK", "text/css; charset=utf-8", html::STYLE).into_bytes()
            }
            // **Barlow, vendored and served from this binary.** `style.css`
            // says why it is not a webfont. `include_bytes!` rather than a
            // file read: an application's data directory is not where a
            // program's own assets live, and a font that could go missing
            // would be a page that silently changed typeface.
            //
            // **`immutable`, unlike every other response here.** `page` sends
            // `no-store`, which is right for a configuration page and wasteful
            // for 314 KB of font that cannot change without the binary
            // changing. A re-render would otherwise re-fetch all three.
            ("GET", "/static/fonts/Barlow-Regular.ttf") => {
                font(include_bytes!("portal/assets/fonts/Barlow-Regular.ttf"))
            }
            ("GET", "/static/fonts/Barlow-SemiBold.ttf") => {
                font(include_bytes!("portal/assets/fonts/Barlow-SemiBold.ttf"))
            }
            ("GET", "/static/fonts/Barlow-Bold.ttf") => {
                font(include_bytes!("portal/assets/fonts/Barlow-Bold.ttf"))
            }
            // **The licence travels with the font**, which the SIL Open Font
            // License requires of anyone redistributing it — and serving it
            // from a binary on a wall is redistributing it.
            ("GET", "/static/fonts/OFL.txt") => page(
                200,
                "OK",
                "text/plain; charset=utf-8",
                include_str!("portal/assets/fonts/OFL.txt"),
            )
            .into_bytes(),
            // **`ADR-051` §12's backup surface.** `backup-path` and
            // `backup-format` in the manifest name this route and what comes
            // back; `restore-path` names the one below. TempMon and Reliquary
            // have declared theirs since the contract was built and Aura had
            // not, which was an omission rather than a decision.
            ("GET", "/backup") => self.backup(),
            ("POST", "/restore") => self.restore(body).into_bytes(),
            ("POST", "/config") => self.save_config(&submitted).into_bytes(),
            ("GET", "/schedule/new") => self.schedule_editor(None).into_bytes(),
            ("GET", "/schedule/edit") => self.schedule_editor(index_of(&query)).into_bytes(),
            ("POST", "/schedule/save") => self.save_schedule(&submitted).into_bytes(),
            ("POST", "/schedule/delete") => self.delete_schedule(index_of(&query)).into_bytes(),
            // **Named, not blank.** A 404 from behind a proxy is nearly always
            // a prefix that was not stripped or a link written absolutely, and
            // saying which path arrived is what makes that a one-look
            // diagnosis.
            (method, path) => page(
                404,
                "Not Found",
                "text/plain; charset=utf-8",
                &format!("no route for {method} {path}\n"),
            )
            .into_bytes(),
        }
    }

    /// `GET /`.
    fn dashboard(&self, query: &form::Form) -> String {
        let config = self.config.read().unwrap_or_else(|e| e.into_inner()).clone();
        let description = self.description.read().unwrap_or_else(|e| e.into_inner()).clone();
        let flash = match query.value("flash") {
            "saved" => "Settings saved.".to_string(),
            "no-actions" => "Event must have at least one action.".to_string(),
            // **The one flash whose text depends on what the addon said.**
            // `validate_fields` refuses a submission with a sentence built
            // from the addon's own `Describe` — see `save_reading` — and
            // carrying it as a query parameter rather than losing it to a
            // fixed string is what lets a person read the actual reason
            // rather than "something was wrong". It is titled the way the
            // section is, with the addon's own name.
            "reading-invalid" => format!(
                "{} settings not saved — {}",
                html::reading_title(description.as_ref()),
                query.value("msg")
            ),
            "indoor-invalid" => format!("Indoor settings not saved — {}", query.value("msg")),
            _ => String::new(),
        };
        // **The values the form is pre-filled with**: what was saved, in the
        // addon's own shape, completed from `Describe`'s defaults. This module
        // hands `Description::fields` and this map to `crate::portal::schema`
        // and never looks inside either.
        let current = description.as_ref().map_or_else(serde_json::Map::new, |d| {
            addon::settings_for(&d.fields, &config.reading_settings)
        });
        let indoor_description = self
            .indoor_description
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        // The same arrangement as `current` above, for the other role:
        // `crate::addon` is the one place allowed to know these three keys
        // mean anything, and this module hands the map on without looking in.
        //
        // **`.rulings/` R-219: this is the RAW saved name, never the resolved
        // one.** `crate::addon::apply_indoor_settings` writes back whatever
        // this field carries the moment the rest of the form is saved, so
        // pre-filling it with a name `effective_tempmon_name` computed — this
        // board's own, while the URL is loopback — would bake that computed
        // value into the file the first time somebody hit Save, which is the
        // rewrite `Config::tempmon_name`'s own doc says must never happen.
        // Showing (or hiding) what will actually be used is
        // `indoor_name_hint`'s job below, in words rather than in the value
        // of a control that round-trips.
        let indoor_current = indoor_description.as_ref().map_or_else(
            serde_json::Map::new,
            |_| {
                addon::indoor_settings(
                    &config.tempmon_url,
                    &config.tempmon_name,
                    &config.indoor_sensor,
                )
            },
        );
        // **Refuse rather than guess, on the page itself.** Only when there
        // is a form to show it on at all (an addon is installed) and
        // resolving a name for actual use failed — never when an operator's
        // own entry is simply absent for a machine with no addon installed
        // at all (`INDOOR_UNAVAILABLE` is that case), and never merely
        // because a name is not asked for — a loopback URL or a plain
        // `http://` one, `config::tempmon_name_is_asked`'s own question —
        // where an empty field is exactly right and there is nothing to say.
        let indoor_name_hint = indoor_description.as_ref().and_then(|_| {
            config::effective_tempmon_name(&config.tempmon_url, &config.tempmon_name).err()
        });
        let zone = crate::clock::Local::now()
            .map_or_else(|_| "unknown".to_string(), |now| now.zone_label());

        let section = match query.value("section") {
            section @ ("general" | "display" | "weather" | "reading" | "schedule"
            | "dashboard") => section,
            // Anything else is discarded rather than reflected. It reaches a
            // JavaScript string literal, and a fixed set is a better answer
            // than an escape that has to be right.
            _ => "",
        };

        let backgrounds = self
            .backgrounds
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let rendered = html::dashboard(
            &config,
            description.as_ref(),
            &current,
            indoor_description.as_ref(),
            &indoor_current,
            indoor_name_hint.as_deref(),
            backgrounds.as_ref(),
            self.sender.is_connected(),
            &flash,
            section,
            &zone,
        );
        page(200, "OK", "text/html; charset=utf-8", &rendered)
    }

    /// `POST /config`.
    fn save_config(&self, submitted: &form::Form) -> String {
        let (
            replayed,
            zip_changed,
            indoor_changed,
            reading_changed,
            reading_error,
            indoor_error,
        ) = {
            let mut config = self.config.write().unwrap_or_else(|e| e.into_inner());

            // **No timezone.** The machine owns it — `crate::clock` argues it —
            // and a field here would be a second answer.
            let zip = submitted.value("zip_code").trim().to_string();
            let zip_changed = zip != config.zip_code;
            config.zip_code = zip;
            config.use_24_hour = submitted.value("use_24_hour") == "1";
            config.show_date = submitted.value("show_date") == "1";

            // Out-of-range is **ignored rather than clamped**: the renderer
            // holds the 5–100 rule, and a value silently changed is a setting a
            // person did not choose.
            if let Ok(brightness) = submitted.value("brightness").parse::<i32>()
                && (5..=100).contains(&brightness)
            {
                config.brightness = brightness;
            }
            // **Appearance is not read here, and that is the point.** It is a
            // preference belonging to the browser and it never leaves it —
            // the control carries no `name`, so nothing about it is posted.
            config.screensaver = submitted.value("screensaver").to_string();
            config.screensaver_theme = submitted.value("screensaver_theme").to_string();

            config.weather_enabled = submitted.value("weather_enabled") == "1";
            config.temp_unit = submitted.value("temp_unit").to_string();
            // A contact a request header cannot carry is **ignored rather than
            // saved**, as an out-of-range brightness is above: storing it would
            // keep a value every request then has to leave out.
            let contact = submitted.value("weather_contact").trim();
            if crate::weather::contact_is_sendable(contact) {
                config.weather_contact = contact.to_string();
            } else {
                self.journal.warn(
                    "portal: the weather contact holds a character a request header cannot \
                     carry, so the saved one is kept",
                );
            }

            let (indoor_changed, indoor_error) = match self.save_indoor(&mut config, submitted) {
                Ok(changed) => (changed, None),
                Err(complaint) => {
                    // Refused against the addon's own declared bounds, before
                    // anything was written and before the addon was asked —
                    // `save_reading` below says why that ordering matters.
                    self.journal
                        .warn(&format!("portal: indoor settings rejected: {complaint}"));
                    (false, Some(complaint))
                }
            };

            let (reading_changed, reading_error) = match self.save_reading(&mut config, submitted) {
                Ok(changed) => (changed, None),
                Err(complaint) => {
                    // **Not saved, and the addon is never asked.** The whole
                    // point of validating against `Describe`'s own bounds
                    // first is that a refusal here costs nothing — no
                    // `Configure`, no round trip, no held reading disturbed.
                    self.journal.warn(&format!("portal: reading settings rejected: {complaint}"));
                    (false, Some(complaint))
                }
            };

            if zip_changed {
                // The cache is keyed by the zip it came from, so this is
                // belt-and-braces — but leaving a stale one would have the
                // weather poller fetch the old coordinates once before the
                // geocode lands.
                config.geo_cache = None;
            }

            let replayed = (
                config.brightness,
                config.screensaver.clone(),
                config.screensaver_theme.clone(),
                config.weather_enabled,
            );
            if let Err(e) = config::save(&config) {
                self.journal.warn(&format!("portal: config save: {e}"));
            }
            (
                replayed,
                zip_changed,
                indoor_changed,
                reading_changed,
                reading_error,
                indoor_error,
            )
        };

        // Applied to the renderer immediately, so a saved brightness is visible
        // on the wall before the page has finished reloading.
        let (brightness, saver, theme, weather) = replayed;
        for message in [
            wire::Message::cmd("brightness", brightness.to_string()),
            wire::Message::screensaver(&saver, &theme),
            wire::Message::cmd("show_weather", screensaver::on_off(weather)),
        ] {
            if let Err(e) = self.sender.send(&message) {
                self.journal.warn(&format!("portal: {e}"));
            }
        }

        if zip_changed {
            Nudges::wake(&self.nudges.weather);
        }
        if indoor_changed {
            Nudges::wake(&self.nudges.indoor);
        }
        if reading_changed {
            Nudges::wake(&self.nudges.reading);
        }

        let section = match submitted.value("section") {
            "" => "general",
            section => section,
        };
        // **The reading card's complaint wins when both refused**, and one is
        // reported rather than both: the flash carries a single sentence, and
        // a page that said "two things were wrong" without saying which would
        // be worse than naming one. Each section's own form is still filled
        // from what was submitted, so nothing a person typed is lost either
        // way. **Each names its own section**: an indoor refusal reported
        // under the reading card's heading would send a person to the wrong
        // form.
        let refused = match (reading_error, indoor_error) {
            (Some(complaint), _) => Some(("reading-invalid", complaint)),
            (None, Some(complaint)) => Some(("indoor-invalid", complaint)),
            (None, None) => None,
        };
        match refused {
            None => redirect(&format!("./?flash=saved&section={}", html::escape(section))),
            Some((flash, complaint)) => redirect(&format!(
                "./?flash={flash}&section={}&msg={}",
                html::escape(section),
                encode_query(&complaint),
            )),
        }
    }

    /// The reading card's half of `POST /config`, with the config already
    /// held for writing. Returns whether the poller needs to know.
    ///
    /// **Nothing here knows what the card shows** (`.rulings/` R-375 D1). The
    /// enable switch is the host's own decision about its own screen (R-198
    /// D2, and `crate::addon`'s module note) and is read directly; everything
    /// else is the addon's own field set, validated by `crate::portal::schema`
    /// against exactly the bounds `Describe` stated and stored as the map it
    /// produces, keyed as the addon spelled its keys
    /// (`crate::addon::apply_settings`). No value is read on the way.
    ///
    /// **A `Secret` field is stored, not sent to the addon.**
    /// `schema::store_secrets` writes a typed credential to the platform secret
    /// file (the same one `rivet apply install-secret` writes); it is never put
    /// in the settings map handed to `Configure`, and `crate::addon::material`
    /// resolves it back by name. A blank field keeps the stored value — see
    /// `crate::portal::schema`'s module note.
    ///
    /// # Errors
    ///
    /// The addon's own complaint, from [`schema::validate_fields`], when a
    /// submitted value is outside a bound the addon itself stated. Nothing is
    /// written and nothing is sent to the addon: a refusal it would have made
    /// anyway is caught before the round trip.
    fn save_reading(&self, config: &mut Config, submitted: &form::Form) -> Result<bool, String> {
        let mut changed = false;
        let enabled = submitted.value("reading_enabled") == "1";
        if enabled != config.reading_enabled {
            config.reading_enabled = enabled;
            changed = true;
        }

        let description = self.description.read().unwrap_or_else(|e| e.into_inner()).clone();
        let Some(description) = description else {
            // No addon is open, so there is no field set to validate against
            // and nothing more to save — the enable switch above still took,
            // which is what lets a person turn the (empty) card off.
            return Ok(changed);
        };

        let parsed = schema::validate_fields(&description.fields, submitted)?;
        // Store any typed credential before the addon is configured, so the
        // `Configure` that follows resolves fresh material. A blank field
        // leaves the stored secret untouched — see `schema::store_secrets`.
        if schema::store_secrets(&description.fields, submitted)? {
            changed = true;
        }
        if addon::apply_settings(&mut config.reading_settings, parsed) {
            changed = true;
        }
        Ok(changed)
    }

    /// **`GET /backup`** — this machine's whole Aura configuration, as one
    /// JSON document (`ADR-051` §12's `Artifact` surface).
    ///
    /// **It is the live config and not the file on disk.** The two agree
    /// whenever nothing is mid-save, and reading the one already in hand
    /// cannot fail — a backup that could answer *could not read my own
    /// configuration* would be a worse answer than the bytes.
    ///
    /// **Nothing here is a secret, and that is a property of Aura rather than
    /// of this route.** A reading source's credential and the three TLS files are
    /// INSTALLED SECRETS, resolved by name through the SDK and never written
    /// into the document (`crate::addon::material` argues why). A backup of
    /// this file therefore carries settings and no credentials, and can be
    /// handed about as ordinary configuration — which is not true of
    /// everything the family backs up, so it is said here rather than assumed.
    fn backup(&self) -> Vec<u8> {
        let config = self.config.read().unwrap_or_else(|e| e.into_inner()).clone();
        match serde_json::to_string_pretty(&config) {
            Ok(text) => download("aura-backup.json", text.as_bytes()),
            Err(e) => {
                self.journal.warn(&format!("portal: backup: render: {e}"));
                page(
                    500,
                    "Internal Server Error",
                    "text/plain; charset=utf-8",
                    &format!("backup: this configuration could not be rendered: {e}\n"),
                )
                .into_bytes()
            }
        }
    }

    /// **`POST /restore`** — replace this machine's Aura configuration with an
    /// uploaded backup (`ADR-051` §12, `restore-path`).
    ///
    /// **The old configuration or the new, each whole, never a mix** — R-186
    /// D2. That is not this function being careful: `config::save` writes a
    /// temporary and renames it, and a rename on one filesystem is atomic, so
    /// a restore interrupted halfway leaves the previous file untouched. What
    /// this function adds is refusing to call it at all on a document that
    /// does not parse.
    ///
    /// **Validated before anything is written.** A body that is not a
    /// configuration is a 400 naming why, and the machine is unchanged — the
    /// same ordering `save_reading` uses for a field set, and for the same
    /// reason: a refusal should cost nothing.
    ///
    /// **The live configuration is replaced too, not just the file.** A
    /// restore that only wrote the disk would leave the running program on the
    /// old settings until something restarted it, which is the kind of
    /// half-applied state that makes a person restore twice.
    fn restore(&self, body: &[u8]) -> String {
        let text = match std::str::from_utf8(body) {
            Ok(text) => text,
            Err(e) => {
                return page(
                    400,
                    "Bad Request",
                    "text/plain; charset=utf-8",
                    &format!("restore: this is not text, so it is not a backup: {e}\n"),
                );
            }
        };

        // `Config` carries `#[serde(default)]`, so an older backup missing a
        // key restores with that key's default rather than being refused --
        // the same tolerance `config::load` gives a file written by an older
        // build, and the reason a backup taken before a new setting existed is
        // still a usable backup.
        let restored: config::Config = match serde_json::from_str(text) {
            Ok(config) => config,
            Err(e) => {
                self.journal
                    .warn(&format!("portal: restore refused: {e}"));
                return page(
                    400,
                    "Bad Request",
                    "text/plain; charset=utf-8",
                    &format!(
                        "restore: this is not an Aura configuration: {e}\n  \
                         Nothing was changed.\n"
                    ),
                );
            }
        };

        if let Err(e) = config::save(&restored) {
            self.journal.warn(&format!("portal: restore: save: {e}"));
            return page(
                500,
                "Internal Server Error",
                "text/plain; charset=utf-8",
                &format!("restore: {e}\n  Nothing was changed.\n"),
            );
        }

        // Written, so the running program adopts it. Order matters: the file
        // is the durable answer and the lock is the live one, and a crash
        // between these two lines leaves a machine that restores correctly on
        // its next start rather than one that forgot.
        {
            let mut held = self.config.write().unwrap_or_else(|e| e.into_inner());
            *held = restored.clone();
        }

        // **Every poller, because a restore can have changed anything.** A
        // saved form knows which section it touched; a restored document does
        // not, so each is woken rather than guessed at.
        Nudges::wake(&self.nudges.weather);
        Nudges::wake(&self.nudges.indoor);
        Nudges::wake(&self.nudges.reading);

        // The renderer is told the three things it holds itself, exactly as a
        // save does -- a restored brightness that only took effect on the next
        // reboot would look like the restore had not worked.
        for message in [
            wire::Message::cmd("brightness", restored.brightness.to_string()),
            wire::Message::screensaver(&restored.screensaver, &restored.screensaver_theme),
            wire::Message::cmd("show_weather", screensaver::on_off(restored.weather_enabled)),
        ] {
            if let Err(e) = self.sender.send(&message) {
                self.journal.warn(&format!("portal: restore: {e}"));
            }
        }

        self.journal.info("portal: configuration restored from a backup");
        page(
            200,
            "OK",
            "text/plain; charset=utf-8",
            "restored\n",
        )
    }

    /// The indoor section, from whatever is filling that role.
    ///
    /// **The mirror of [`Portal::save_reading`], and deliberately identical in
    /// shape.** Until 2026-09-20 this section was three fields parsed by name
    /// right here — `tempmon_url`, `tempmon_name`, `indoor_sensor` — because
    /// the indoor reading was compiled into this program. It is an addon now,
    /// so the form is built from what that addon DESCRIBES and validated
    /// against the bounds it declared, exactly as the reading card's is. This
    /// module still knows none of the three key names; `crate::addon` does.
    ///
    /// **The enable switch is this program's and stays here.** It is the
    /// host's decision about its own screen, which is why no addon has an
    /// `enabled` field to be told — and why turning the section off has to
    /// work when nothing is installed to configure.
    ///
    /// # Errors
    ///
    /// A sentence, when what was submitted is outside the field set the addon
    /// declared. Nothing is written and the addon is never asked.
    fn save_indoor(&self, config: &mut Config, submitted: &form::Form) -> Result<bool, String> {
        let mut changed = false;
        let enabled = submitted.value("indoor_enabled") == "1";
        if enabled != config.indoor_enabled {
            config.indoor_enabled = enabled;
            changed = true;
        }

        let description = self
            .indoor_description
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(description) = description else {
            // No indoor source is open, so there is no field set to validate
            // against. The switch above still took, which is what lets a
            // person turn the (empty) section off.
            return Ok(changed);
        };

        let parsed = schema::validate_fields(&description.fields, submitted)?;
        // The same credential path as the reading card's: a typed secret is
        // stored, a blank field keeps what is there. Addon-agnostic — this
        // section knows no more about its addon's secrets than the reading
        // card's section knows about its own.
        if schema::store_secrets(&description.fields, submitted)? {
            changed = true;
        }
        if addon::apply_indoor_settings(config, &parsed) {
            changed = true;
        }
        Ok(changed)
    }

    /// `GET /schedule/new` and `GET /schedule/edit?index=N`.
    fn schedule_editor(&self, index: Option<usize>) -> String {
        let backgrounds = self
            .backgrounds
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(index) = index else {
            return page(
                200,
                "OK",
                "text/html; charset=utf-8",
                &html::schedule_editor(
                    backgrounds.as_ref(),
                    None,
                    &ScheduledEvent::default(),
                ),
            );
        };
        let event = {
            let config = self.config.read().unwrap_or_else(|e| e.into_inner());
            config.schedule.get(index).cloned()
        };
        match event {
            Some(event) => page(
                200,
                "OK",
                "text/html; charset=utf-8",
                &html::schedule_editor(backgrounds.as_ref(), Some(index), &event),
            ),
            // An index that is not there is a stale link — somebody deleted the
            // event in another tab — and the list is where they wanted to be.
            None => redirect("../?section=schedule"),
        }
    }

    /// `POST /schedule/save`.
    fn save_schedule(&self, submitted: &form::Form) -> String {
        let index = submitted.value("index").parse::<i64>().unwrap_or(-1);
        let mut event = ScheduledEvent {
            time: submitted.value("time").to_string(),
            days: submitted.all("days").iter().map(|d| (*d).to_string()).collect(),
            actions: Vec::new(),
        };
        // The editor renumbers its rows on every add and remove, so the indices
        // are contiguous from zero and the first gap is the end.
        for position in 0.. {
            let kind = submitted.value(&format!("action.{position}.type"));
            if kind.is_empty() {
                break;
            }
            event.actions.push(ScheduledAction {
                kind: kind.to_string(),
                value: submitted.value(&format!("action.{position}.value")).to_string(),
                theme: submitted.value(&format!("action.{position}.theme")).to_string(),
            });
        }
        if event.actions.is_empty() {
            return redirect("../?flash=no-actions&section=schedule");
        }

        {
            let mut config = self.config.write().unwrap_or_else(|e| e.into_inner());
            if index < 0 {
                config.schedule.push(event);
            } else if let Ok(index) = usize::try_from(index)
                && index < config.schedule.len()
            {
                config.schedule[index] = event;
            }
            if let Err(e) = config::save(&config) {
                self.journal.warn(&format!("portal: schedule save: {e}"));
            }
        }
        // The schedule thread re-reads the config every minute, so there is
        // nothing to reload — which is the one callback the Go portal has that
        // this does not need.
        redirect("../?section=schedule&flash=saved")
    }

    /// `POST /schedule/delete?index=N`.
    fn delete_schedule(&self, index: Option<usize>) -> String {
        if let Some(index) = index {
            let mut config = self.config.write().unwrap_or_else(|e| e.into_inner());
            if index < config.schedule.len() {
                config.schedule.remove(index);
                if let Err(e) = config::save(&config) {
                    self.journal.warn(&format!("portal: schedule delete: {e}"));
                }
            }
        }
        redirect("../?section=schedule")
    }
}

/// An `index` query parameter, when it is a number.
fn index_of(query: &form::Form) -> Option<usize> {
    query.value("index").parse::<usize>().ok()
}

/// Percent-encode a value for a query string — the one direction
/// `crate::portal::form` does not need, because everything else this module
/// puts in a redirect's query string is already one of a fixed set of words.
///
/// **Used for exactly one thing**: carrying an addon's own complaint back to
/// the dashboard after a rejected submission (`save_config`), so the
/// message a person reads is the addon's sentence and not a generic one. A
/// space or an ampersand inside it must not be read back as a second
/// parameter or split the query string in two.
fn encode_query(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// One complete HTTP response.
fn page(status: u16, reason: &str, content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    )
}

/// A response the browser saves rather than renders — the backup.
///
/// **`Content-Disposition: attachment` is the whole of it.** Without it a
/// browser shows the JSON in a tab, which looks like it worked and leaves a
/// person with no file; `filename` is what they get when they say yes.
///
/// `no-store`, like every other answer that is not a font: a backup is a
/// reading of this machine at one moment, and a cached one handed back later
/// would be a different machine's configuration wearing today's date.
fn download(filename: &str, body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: application/json; charset=utf-8\r\n\
         Content-Disposition: attachment; filename=\"{filename}\"\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

/// One complete HTTP response carrying **bytes** — a vendored font.
///
/// Separate from [`page`] rather than a generalisation of it, because the two
/// differ in more than the body type: this one is cacheable and that one must
/// never be. A configuration page sends `no-store` because what it shows can
/// change between two loads; a font compiled into this binary cannot change
/// without the binary changing, and `no-store` on 314 KB of it would mean
/// fetching all three weights again on every save.
fn font(body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: font/ttf\r\n\
         Content-Length: {}\r\n\
         Cache-Control: public, max-age=31536000, immutable\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

/// A `303 See Other`, which is what a form submission answers with.
///
/// **The location is relative**, so it survives being mounted under `/aura/`.
/// An absolute one would send the browser to the machine's root the moment the
/// portal was reached through the proxy — see `html`'s note.
fn redirect(location: &str) -> String {
    format!(
        "HTTP/1.1 303 See Other\r\n\
         Location: {location}\r\n\
         Content-Length: 0\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\
         \r\n"
    )
}

/// What to do about the three files, printed with the refusal that needs them.
fn install_hint() -> String {
    format!(
        "\n\nThe portal serves HTTPS and verifies its caller; there is no plaintext mode.\n\
         These three files are per-machine and are deliberately not in the payload:\n\
         \x20   {CERTIFICATE}  the certificate this machine serves\n\
         \x20   {KEY}   its private key\n\
         \x20   {AUTHORITY}  the fleet authority a caller must chain to\n\
         Install each into this application's data directory with:\n\
         \x20   rivet apply install-secret aura-controller <name> --hex <the PEM, as hex> confirm"
    )
}

#[cfg(test)]
mod tests;
