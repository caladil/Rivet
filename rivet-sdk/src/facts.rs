//! **Ask this machine what it is** — `ADR-057`, `.rulings/` R-199 D4.
//!
//! One bounded call, answered by the supervisor out of memory it already
//! holds. An application does not go and read `/etc`, walk `/state`, or open a
//! connection to its own control plane: `SPEC-03` §2 is that it is **handed**
//! what it needs, and this is the handing.
//!
//! ```no_run
//! match rivet_sdk::facts::ask() {
//!     Ok(facts) => {
//!         println!("{} running {}", facts.hostname.as_deref().unwrap_or("this machine"),
//!                  facts.version.as_deref().unwrap_or("an unknown build"));
//!         for service in &facts.services {
//!             println!("  {} {} ({})", service.name, service.state, service.health.word());
//!         }
//!     }
//!     // **The one that matters.** See [`NotAnswered::Silent`].
//!     Err(rivet_sdk::facts::NotAnswered::Silent) => {
//!         println!("the supervisor did not answer within {:?}", rivet_sdk::facts::PATIENCE);
//!     }
//!     Err(other) => println!("{other}"),
//! }
//! ```
//!
//! # Silence is the answer this exists for
//!
//! A Rivet has wedged four times with the same signature: the supervisor stops
//! and **everything already running carries on**, so from across a room the
//! machine looks perfect, and the control plane does not go quiet for another
//! eleven minutes. This call is bounded by [`PATIENCE`], so a supervisor that
//! has stopped produces [`NotAnswered::Silent`] in two seconds — immediately,
//! and unambiguously.
//!
//! That is the whole argument for a call over a file. A file written by a
//! wedged supervisor is byte-identical to one written by a healthy supervisor
//! that has not ticked yet, so telling them apart needs a staleness threshold
//! somebody has to pick and everybody has to agree on. Silence needs nothing.
//!
//! # The one thing this call cannot tell you, and where to get it
//!
//! **Whether the supervisor is alive, and where it is stuck.** An instrument
//! may not depend on the thing it measures (`.rulings/` R-199 D5), so the
//! supervisor does not answer that about itself. [`crate::supervisor`] reads it
//! from `/proc`, in this process, with nothing in between.
//!
//! **The pairing is the point**: when the supervisor wedges, this call goes
//! quiet and that read keeps working, so a screen says *the supervisor stopped,
//! and it is parked in `unix_wait_for_peer`* rather than merely going stale.
//!
//! # Every fact carries how old it is
//!
//! An **age**, never an instant, so the two processes need no agreement about
//! a clock and no skew can make one negative. [`Facts::received`] is when this
//! application got the answer, and [`Facts::health_age`] adds the elapsed time
//! since to the age the supervisor sent — which is what turns *4 s ago* into
//! *6 minutes ago* with nothing having to arrive to say so.
//!
//! Three facts carry no age at all — [`Facts::hostname`], [`Facts::version`]
//! and [`Facts::slot`] — because they are fixed for the life of the boot and
//! stamping them would invite a screen to draw *updated 0 s ago* beside
//! something that has been true since the machine came up.
//!
//! # Why this re-declares the wire form instead of importing it
//!
//! `ADR-036` §8: **the SDK links no Rivet crate.** The words and the format
//! below are a second copy of `rivet_core::facts`'s, and `tests/agreement.rs`
//! puts an answer rendered by the real supervisor's own code through the
//! parser here. That test is not ceremony — a drifted *encoding* fails
//! silently, and a dashboard reading one field as another would be wrong
//! confidently.

/// **How long a caller waits before concluding the supervisor has stopped.**
///
/// Two seconds, and the number is chosen from both ends. A healthy supervisor
/// answers inside one pass of its own loop, which is milliseconds — so
/// anything approaching this is already abnormal. And a screen that took
/// longer to notice would be an instrument a person walks past.
///
/// **It is not configurable, deliberately.** A knob here would let one
/// application decide how long the machine looks healthy for.
pub const PATIENCE: std::time::Duration = std::time::Duration::from_secs(2);

/// Where the supervisor listens. A second copy of
/// `rivet_core::facts::SOCKET_PATH`, held honest by `tests/agreement.rs`.
pub const SOCKET_PATH: &str = "/run/rivet/facts";

/// What this application sends. A second copy of
/// `rivet_core::facts::REQUEST`.
pub const REQUEST: &str = "get facts\n";

/// The answer version this build reads. A second copy of
/// `rivet_core::facts::PROTOCOL`.
pub const PROTOCOL: u32 = 1;

/// The most an answer may be, and the size of the buffer one is read into. A
/// second copy of `rivet_core::facts::ANSWER_MAX_BYTES`.
pub const ANSWER_MAX_BYTES: usize = 64 * 1024;

/// **What a thing IS, as an observer concludes it** — `ADR-045` §4.
///
/// Four words where [`crate::health::Reported`] has three, and the extra one
/// is the whole difference between the two types: `unknown` is what a
/// *watcher* says about something that has not spoken, so an application
/// cannot report it about itself and a dashboard must be able to draw it.
/// Agreement with `rivet_core::health::State` is asserted by
/// `tests/agreement.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// Doing its job.
    Healthy,
    /// Working, but something is wrong that a person should eventually know.
    Degraded,
    /// Not doing its job.
    Down,
    /// **Has not said**, and it is too early or too quiet to tell.
    Unknown,
}

impl Health {
    /// The word this state is spelled with, on the wire and on a screen.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Down => "down",
            Self::Unknown => "unknown",
        }
    }

    fn from_word(text: &str) -> Option<Self> {
        match text {
            "healthy" => Some(Self::Healthy),
            "degraded" => Some(Self::Degraded),
            "down" => Some(Self::Down),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

/// What the machine says about one service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceFact {
    /// Its name.
    pub name: String,
    /// What the operator's document says about it.
    pub enabled: bool,
    /// The supervisor's own word — `idle`, `started`, `done`, `failed`.
    /// **A `String` and not an enum**: it is the supervisor's vocabulary
    /// carried through unchanged, and an enum here would map a word a newer
    /// machine used onto the nearest one this build knew.
    pub state: String,
    /// Its process id while it is running, and `None` otherwise. Absent rather
    /// than zero, because zero is a pid.
    pub pid: Option<u32>,
    /// How many times the supervisor has restarted it after a failure. `0` for
    /// a service that started first time and is still running, so this says
    /// *is this thing flapping*.
    pub restarts: u32,
    /// Whether it has declared readiness. **`false` for a service that never
    /// declares one**, which is most of them — this is not a health check.
    pub ready: bool,
    /// What it last answered when the supervisor asked, and `Unknown` when
    /// nothing has ever answered.
    pub health: Health,
    /// **How old that answer was when the supervisor sent this**, and `None`
    /// when no answer has arrived.
    ///
    /// Use [`Facts::health_age`] rather than this directly: the supervisor
    /// polls every thirty seconds, and a screen must add its own elapsed time
    /// to this or it will draw a reading as live that is a minute old.
    pub health_age_ms: Option<u64>,
}

/// **Everything this machine will tell an application about itself.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    /// The answer's own version, which this build has already checked.
    pub protocol: u32,
    /// This machine's name, and `None` on one that has no name.
    /// **Fixed for the life of the boot.**
    pub hostname: Option<String>,
    /// The running image's version — the same string `rivet --version`
    /// carries. **Fixed for the life of the boot.**
    pub version: Option<String>,
    /// Which A/B slot is running, `a` or `b`, and `None` on a machine that is
    /// not running off a card. **Fixed for the life of the boot.**
    pub slot: Option<String>,
    /// What the supervisor is doing as a whole — `running`, `shutting-down`
    /// or `finished`. A machine on an older image says `converging` for
    /// `running`, so an application that acts on this treats the two alike.
    pub phase: String,
    /// **Whether this machine is doing its job**, folded over the services the
    /// operator marked critical. Not the worst of *every* service: a degraded
    /// weather probe does not make a machine degraded.
    pub machine: Health,
    /// What this machine is for — `kiosk`, `headless`, or `unknown` for one
    /// whose own document could not be read. **Three words and not a boolean**:
    /// drawing `unknown` as `headless` would state something confidently about
    /// a document nobody parsed.
    pub mode: String,
    /// **How old the supervisor's view was when it answered**, in
    /// milliseconds. See [`Facts::view_age`], which adds this application's own
    /// elapsed time.
    pub view_age_ms: u64,
    /// One per service, in the order the supervisor holds them.
    pub services: Vec<ServiceFact>,
    /// **How many services did not fit in the answer**, and `0` on any machine
    /// anybody runs. A dashboard drawing part of a machine can say so.
    pub truncated: u32,
    /// **When this application received the answer.**
    ///
    /// Monotonic, so it is unaffected by the wall clock being set — which on a
    /// board with no battery-backed clock happens some seconds after boot, in
    /// the middle of exactly the window a dashboard is first drawing.
    pub received: std::time::Instant,
}

impl Facts {
    /// **How old the supervisor's view is now**, counting the time since this
    /// answer arrived.
    #[must_use]
    pub fn view_age(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.view_age_ms) + self.received.elapsed()
    }

    /// **How old a service's health word is now**, and `None` when nothing has
    /// ever answered for it.
    ///
    /// The number a tile draws. It is the age the supervisor measured plus the
    /// time since this answer arrived, and a screen that showed only the first
    /// would report a reading as current for as long as it failed to ask
    /// again — which is precisely the failure mode this whole surface is
    /// built to make visible.
    #[must_use]
    pub fn health_age(&self, service: &ServiceFact) -> Option<std::time::Duration> {
        service
            .health_age_ms
            .map(|ms| std::time::Duration::from_millis(ms) + self.received.elapsed())
    }

    /// How long ago this answer arrived. *Last updated 4 s ago.*
    #[must_use]
    pub fn age(&self) -> std::time::Duration {
        self.received.elapsed()
    }

    /// One service by name, for a screen that draws a particular tile.
    #[must_use]
    pub fn service(&self, name: &str) -> Option<&ServiceFact> {
        self.services.iter().find(|s| s.name == name)
    }
}

/// **Why there is no answer**, and the variants are not interchangeable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotAnswered {
    /// **There is no socket at all**, so this program is not running on a
    /// Rivet — a workstation, a test, a container. **Not an error**, and the
    /// same answer [`crate::health::Delivered::NoChannel`] gives for the other
    /// direction: there is nothing an application can do about it and nothing
    /// it should stop for.
    NotARivet,
    /// **THE SUPERVISOR DID NOT ANSWER WITHIN [`PATIENCE`].**
    ///
    /// The variant this surface exists for. The socket is there, the request
    /// went out, and nothing came back — which on a machine whose supervisor
    /// is turning cannot happen, because it answers inside one pass of its own
    /// loop.
    ///
    /// **Do not retry this into a spinner.** It is a reading, and a screen
    /// should draw it: *the supervisor stopped*, beside what
    /// [`crate::supervisor`] says about where. Retrying is how an instrument
    /// turns the one signal it has into a blank tile.
    Silent,
    /// The request could not be sent or a reply socket could not be made, with
    /// the system's own words. Distinct from [`Self::Silent`] because they mean
    /// opposite things: this is *this application could not ask*, and that is
    /// *the machine did not answer*.
    CouldNotAsk(String),
    /// An answer arrived and could not be read, with the reason. The
    /// supervisor is alive — which is itself worth knowing — and this build
    /// cannot understand what it said.
    Unreadable(String),
}

impl std::fmt::Display for NotAnswered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotARivet => write!(
                f,
                "{SOCKET_PATH} is not there, so this process is not running on a Rivet. See \
                 ADR-036 §3 for what a Rivet hands an application."
            ),
            Self::Silent => write!(
                f,
                "the supervisor did not answer within {PATIENCE:?}. A supervisor that is turning \
                 answers inside one pass of its own loop, so this is the machine's supervisor \
                 having stopped — read /proc for where it is parked (rivet_sdk::supervisor)."
            ),
            Self::CouldNotAsk(why) => {
                write!(f, "this application could not ask the supervisor: {why}")
            }
            Self::Unreadable(why) => write!(
                f,
                "the supervisor answered and this build could not read what it said: {why}"
            ),
        }
    }
}

impl std::error::Error for NotAnswered {}

/// **Ask, and wait no longer than [`PATIENCE`].**
///
/// # Errors
///
/// [`NotAnswered`], and the variants are worth telling apart — see
/// [`NotAnswered::Silent`] in particular, which is a reading about the machine
/// rather than a failure of this call.
#[cfg(unix)]
pub fn ask() -> Result<Facts, NotAnswered> {
    ask_at(SOCKET_PATH)
}

/// **The call, with the socket as an argument**, so a test can stand a
/// supervisor up in a temporary directory and drive the whole exchange on the
/// build host. `AGENTS.md` §8.3 — paths are arguments, never constants.
///
/// # Errors
///
/// See [`ask`].
#[cfg(unix)]
pub fn ask_at(socket: &str) -> Result<Facts, NotAnswered> {
    if !std::path::Path::new(socket).exists() {
        return Err(NotAnswered::NotARivet);
    }

    // **A socket of this application's own, in the directory the machine made
    // for it.** A datagram needs an address to be answered at, and an unbound
    // socket has none.
    //
    // `std::env::temp_dir` reads `TMPDIR`, which on a Rivet the supervisor sets
    // to `RIVET_APP_TMP` — the one scratch directory an application is granted
    // and the one it is emptied out of at every spawn. So this is a handed path
    // rather than a guessed one, and on a workstation it is `/tmp`, which is
    // the right answer there.
    //
    // **One address per CALL, not per process.** Two threads asking at once
    // would otherwise share one path, and the second call's clean-up below
    // would delete the first call's live socket out from under it.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mine = std::env::temp_dir()
        .join(format!("rivet-facts-{}-{seq}.sock", std::process::id()));
    ask_via(socket, &mine)
}

/// [`ask_at`], replying at `mine`: the exchange and its clean-up, apart from
/// choosing the reply address, so a test can name the address it then checks.
#[cfg(unix)]
fn ask_via(socket: &str, mine: &std::path::Path) -> Result<Facts, NotAnswered> {
    use std::os::unix::net::UnixDatagram;

    // A socket left behind by a previous run of this program would make `bind`
    // refuse the path. The supervisor's own `open_facts` has the same line for
    // the same reason.
    let _ = std::fs::remove_file(mine);
    let reply = UnixDatagram::bind(mine).map_err(|e| NotAnswered::CouldNotAsk(e.to_string()))?;

    let out = exchange(&reply, socket);
    // **Cleaned up on every path, including the failures.** A program that
    // asks once a second and left one of these behind each time would fill its
    // own scratch directory with the evidence of a wedged supervisor.
    let _ = std::fs::remove_file(mine);
    out
}

#[cfg(unix)]
fn exchange(reply: &std::os::unix::net::UnixDatagram, socket: &str) -> Result<Facts, NotAnswered> {
    // **Both deadlines, and the write one is not decoration.** A supervisor
    // that has stopped reading leaves its receive queue full, and a blocking
    // `send_to` into a full queue waits with no bound — which is the same
    // `unix_wait_for_peer` wait that stopped the supervisor in the first place,
    // arriving at the instrument instead. The whole call is bounded or it is
    // not bounded.
    reply
        .set_write_timeout(Some(PATIENCE))
        .map_err(|e| NotAnswered::CouldNotAsk(e.to_string()))?;
    reply
        .set_read_timeout(Some(PATIENCE))
        .map_err(|e| NotAnswered::CouldNotAsk(e.to_string()))?;

    if let Err(e) = reply.send_to(REQUEST.as_bytes(), socket) {
        return Err(match e.kind() {
            // A full queue at the far end is the supervisor not reading, which
            // is the same reading as no answer and must not be reported as a
            // failure of this application.
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => NotAnswered::Silent,
            // Nothing is bound there any more: the socket file outlived the
            // supervisor that bound it, which `/run` being a tmpfs makes
            // unusual and a restart makes possible.
            std::io::ErrorKind::ConnectionRefused => NotAnswered::NotARivet,
            _ => NotAnswered::CouldNotAsk(e.to_string()),
        });
    }

    let mut buf = vec![0_u8; ANSWER_MAX_BYTES];
    let n = match reply.recv(&mut buf) {
        Ok(n) => n,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            return Err(NotAnswered::Silent);
        }
        Err(e) => return Err(NotAnswered::CouldNotAsk(e.to_string())),
    };
    let text = String::from_utf8_lossy(&buf[..n]).into_owned();
    parse(&text).map_err(NotAnswered::Unreadable)
}

/// Off Unix there is no socket to ask on, which is the same answer as a machine
/// that is not a Rivet. This exists so the SDK still compiles on the
/// development host — `crate`'s module note has the rule.
///
/// # Errors
///
/// Always [`NotAnswered::NotARivet`].
#[cfg(not(unix))]
pub fn ask() -> Result<Facts, NotAnswered> {
    Err(NotAnswered::NotARivet)
}

/// **Read an answer.** Public so a test can put one rendered by the
/// supervisor's own code through it — see `tests/agreement.rs`.
///
/// # Errors
///
/// One sentence saying what was wrong with it.
pub fn parse(text: &str) -> Result<Facts, String> {
    let mut out = Facts {
        protocol: PROTOCOL,
        hostname: None,
        version: None,
        slot: None,
        phase: String::new(),
        machine: Health::Unknown,
        mode: "unknown".to_string(),
        view_age_ms: 0,
        services: Vec::new(),
        truncated: 0,
        received: std::time::Instant::now(),
    };
    let mut first = true;
    let mut ended = false;

    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let mut fields = line.split(' ');
        let kind = fields.next().unwrap_or("");
        if first {
            if kind != "facts" {
                return Err("this does not begin with a `facts` line, so it is not an answer from \
                            a Rivet supervisor"
                    .to_string());
            }
            let found: u32 = fields
                .next()
                .and_then(|t| t.parse().ok())
                .ok_or_else(|| "the `facts` line carries no protocol number".to_string())?;
            // **Refused, never parsed hopefully.** A reader that went on would
            // draw a number out of whichever field happened to sit in that
            // position, and be wrong confidently.
            if found != PROTOCOL {
                return Err(format!(
                    "this machine answers facts protocol {found} and this application reads \
                     {PROTOCOL}; the machine is newer than the program running on it"
                ));
            }
            out.protocol = found;
            first = false;
            continue;
        }
        match kind {
            "host" => out.hostname = optional(fields.next()),
            "version" => out.version = optional(fields.next()),
            "slot" => out.slot = optional(fields.next()),
            "phase" => out.phase = fields.next().unwrap_or("").to_string(),
            "machine" => {
                out.machine = Health::from_word(fields.next().unwrap_or(""))
                    .ok_or_else(|| "the `machine` line is not a health word".to_string())?;
            }
            "mode" => out.mode = fields.next().unwrap_or("unknown").to_string(),
            "view" => {
                out.view_age_ms = fields
                    .next()
                    .and_then(|t| t.parse().ok())
                    .ok_or_else(|| "the `view` line carries no age".to_string())?;
            }
            "svc" => {
                let rest: Vec<&str> = fields.collect();
                if rest.len() != 8 {
                    return Err(format!(
                        "a `svc` line carries {} fields and this build reads 8",
                        rest.len()
                    ));
                }
                out.services.push(ServiceFact {
                    name: rest[0].to_string(),
                    enabled: rest[1] == "yes",
                    state: rest[2].to_string(),
                    pid: number(rest[3]),
                    restarts: number(rest[4]).unwrap_or(0),
                    ready: rest[5] == "yes",
                    health: Health::from_word(rest[6])
                        .ok_or_else(|| format!("`{}` is not a health word", rest[6]))?,
                    health_age_ms: number(rest[7]),
                });
            }
            "end" => {
                let _held = fields.next();
                out.truncated = number(fields.next().unwrap_or("0")).unwrap_or(0);
                ended = true;
            }
            // **A line kind this build does not know is skipped**, where an
            // unknown protocol is refused. Within one protocol a new line can
            // only be an addition, and taking a dashboard down at an upgrade
            // over a field it does not draw would be the instrument failing for
            // a reason that has nothing to do with the machine.
            _ => {}
        }
    }
    if first {
        return Err("this is not an answer from a Rivet supervisor".to_string());
    }
    if !ended {
        // **The check a truncated datagram needs.** An answer cut off after
        // four services parses perfectly as a machine running four services,
        // and a screen would draw it with no hint anything was missing.
        return Err(
            "the answer carries no `end` line, so it was cut short and the machine has more in it \
             than this says"
                .to_string(),
        );
    }
    Ok(out)
}

fn optional(text: Option<&str>) -> Option<String> {
    match text {
        None | Some("-") | Some("") => None,
        Some(value) => Some(value.to_string()),
    }
}

fn number<T: std::str::FromStr>(text: &str) -> Option<T> {
    if text == "-" {
        return None;
    }
    text.parse().ok()
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    const ANSWER: &str = "facts 1\n\
                          host rivet-06f5c\n\
                          version 0.1.92\n\
                          slot b\n\
                          phase converging\n\
                          machine healthy\n\
                          mode kiosk\n\
                          view 212\n\
                          svc aura-controller yes started 678 0 no healthy 4120\n\
                          svc rivet-netd yes started 141 2 yes unknown -\n\
                          end 2 0\n";

    #[test]
    fn a_whole_answer_reads_field_for_field() {
        let facts = parse(ANSWER).expect("an answer");
        assert_eq!(facts.hostname.as_deref(), Some("rivet-06f5c"));
        assert_eq!(facts.version.as_deref(), Some("0.1.92"));
        assert_eq!(facts.slot.as_deref(), Some("b"));
        assert_eq!(facts.phase, "converging");
        assert_eq!(facts.machine, Health::Healthy);
        assert_eq!(facts.mode, "kiosk");
        assert_eq!(facts.view_age_ms, 212);
        assert_eq!(facts.services.len(), 2);

        let aura = facts.service("aura-controller").expect("the controller");
        assert_eq!(aura.pid, Some(678));
        assert_eq!(aura.health, Health::Healthy);
        assert_eq!(aura.health_age_ms, Some(4_120));
        assert!(!aura.ready);

        let netd = facts.service("rivet-netd").expect("the network service");
        assert_eq!(netd.restarts, 2);
        assert!(netd.ready);
        assert_eq!(netd.health, Health::Unknown);
        assert_eq!(
            netd.health_age_ms, None,
            "a service nothing has ever answered for has no age, and reporting 0 would say it \
             answered this instant"
        );
    }

    /// **The age a screen draws is the supervisor's plus this program's own.**
    ///
    /// The whole feature, as a test: a reading the supervisor measured at 4 s
    /// is older than 4 s by the time anything draws it, and an instrument that
    /// reported the number it was sent would show a stale reading as live for
    /// as long as it failed to ask again.
    #[test]
    fn an_age_grows_after_the_answer_arrives() {
        let facts = parse(ANSWER).expect("an answer");
        let aura = facts.service("aura-controller").expect("the controller");
        let age = facts.health_age(aura).expect("an age");
        assert!(
            age >= std::time::Duration::from_millis(4_120),
            "{age:?} is younger than the age the supervisor measured"
        );
        assert!(facts.view_age() >= std::time::Duration::from_millis(212));
        assert_eq!(facts.health_age(&facts.services[1]), None);
    }

    /// **A machine newer than the application is refused, not guessed at.**
    #[test]
    fn an_answer_from_a_protocol_this_build_does_not_read_is_refused() {
        let text = ANSWER.replacen("facts 1", "facts 7", 1);
        let why = parse(&text).expect_err("refused");
        assert!(why.contains('7'), "{why}");
        assert!(why.contains("newer"), "{why}");
    }

    /// **A cut-short answer is refused rather than read as a small machine.**
    #[test]
    fn an_answer_without_its_end_line_is_refused() {
        let cut = ANSWER.replacen("end 2 0\n", "", 1);
        let why = parse(&cut).expect_err("refused");
        assert!(why.contains("cut short"), "{why}");
    }

    #[test]
    fn something_that_is_not_an_answer_is_not_read_as_one() {
        assert!(parse("").is_err());
        assert!(parse("HTTP/1.1 200 OK\r\n\r\n").is_err());
        assert!(parse("phase converging\nend 0 0\n").is_err());
    }

    /// **A field a later supervisor added does not take the answer down**, so
    /// a dashboard survives the machine under it being updated.
    #[test]
    fn a_line_kind_from_a_later_machine_is_skipped() {
        let text = ANSWER.replacen("end 2 0", "thermal 54200 no\nend 2 0", 1);
        let facts = parse(&text).expect("an answer");
        assert_eq!(facts.services.len(), 2);
        assert_eq!(facts.hostname.as_deref(), Some("rivet-06f5c"));
    }

    /// **A truncated fact set says how much it left out.**
    #[test]
    fn a_partial_machine_says_that_it_is_partial() {
        let text = ANSWER.replacen("end 2 0", "end 402 400", 1);
        let facts = parse(&text).expect("an answer");
        assert_eq!(facts.truncated, 400);
    }

    /// **A machine that is not one answers, and the call says which kind of
    /// nothing it got.** The three failures mean different things and a screen
    /// draws them differently: *not a Rivet*, *the supervisor stopped*, and
    /// *this program could not ask*.
    #[cfg(unix)]
    #[test]
    fn no_socket_at_all_is_not_a_wedged_supervisor() {
        let missing = std::env::temp_dir()
            .join(format!("rivet-facts-absent-{}", std::process::id()))
            .to_string_lossy()
            .into_owned();
        let _ = std::fs::remove_file(&missing);
        assert_eq!(ask_at(&missing), Err(NotAnswered::NotARivet));
    }

    /// **A supervisor that binds the socket and never answers is `Silent`**,
    /// and this is the case the whole surface exists for — measured over a real
    /// socket rather than argued about.
    ///
    /// It takes [`PATIENCE`], which is the point: the bound is what makes
    /// silence a reading rather than a hang.
    #[cfg(unix)]
    #[test]
    fn a_supervisor_that_does_not_answer_is_silent_within_the_deadline() {
        let dir = std::env::temp_dir().join(format!("rivet-facts-wedged-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temporary directory");
        let path = dir.join("facts");
        // The supervisor's end: bound, and never read from. That IS the wedge —
        // the process is alive, the socket is there, and nothing is turning.
        let _wedged = std::os::unix::net::UnixDatagram::bind(&path).expect("the bound socket");

        let began = std::time::Instant::now();
        let answer = ask_at(&path.to_string_lossy());
        let waited = began.elapsed();

        assert_eq!(answer, Err(NotAnswered::Silent));
        assert!(
            waited >= PATIENCE && waited < PATIENCE * 3,
            "waited {waited:?}, and the whole value of this call is that it is bounded"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The whole exchange, over a real pair of sockets.**
    ///
    /// A supervisor's end: receive the request, check it is the one this socket
    /// takes, answer the address it came from. This asserts the thing an
    /// application depends on — that asking produces the machine's own answer —
    /// and it asserts the reply goes to the *sender's* address rather than to
    /// anything in the payload.
    #[cfg(unix)]
    #[test]
    fn asking_a_supervisor_that_answers_returns_the_machine() {
        let dir = std::env::temp_dir().join(format!("rivet-facts-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temporary directory");
        let path = dir.join("facts");
        let supervisor =
            std::os::unix::net::UnixDatagram::bind(&path).expect("the supervisor's end");

        let answering = std::thread::spawn(move || {
            let mut buf = [0_u8; 512];
            let (n, from) = supervisor.recv_from(&mut buf).expect("the request");
            assert_eq!(&buf[..n], REQUEST.as_bytes());
            let to = from.as_pathname().expect("the caller bound a socket").to_path_buf();
            supervisor.send_to(ANSWER.as_bytes(), &to).expect("the answer");
        });

        let facts = ask_at(&path.to_string_lossy()).expect("an answer");
        answering.join().expect("the answering thread");
        assert_eq!(facts.hostname.as_deref(), Some("rivet-06f5c"));
        assert_eq!(facts.services.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The reply socket is cleaned up**, on the answered path and on the
    /// silent one. A dashboard asking once a second would otherwise fill its
    /// own scratch directory with the evidence of a wedge.
    ///
    /// Both paths are exercised against a real bound socket, because a call to
    /// an absent socket returns before it binds anything and so proves nothing
    /// about clean-up.
    #[cfg(unix)]
    #[test]
    fn a_call_leaves_no_socket_behind() {
        let dir = std::env::temp_dir().join(format!("rivet-facts-cleanup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temporary directory");

        // Silent: the supervisor's end is bound and never read from.
        let wedged_path = dir.join("wedged");
        let _wedged = std::os::unix::net::UnixDatagram::bind(&wedged_path).expect("bound");
        let mine = dir.join("reply-silent.sock");
        assert_eq!(ask_via(&wedged_path.to_string_lossy(), &mine), Err(NotAnswered::Silent));
        assert!(!mine.exists(), "{} was left behind on the silent path", mine.display());

        // Answered: the supervisor's end replies to the sender's address.
        let live_path = dir.join("live");
        let supervisor = std::os::unix::net::UnixDatagram::bind(&live_path).expect("bound");
        let answering = std::thread::spawn(move || {
            let mut buf = [0_u8; 512];
            let (_, from) = supervisor.recv_from(&mut buf).expect("the request");
            let to = from.as_pathname().expect("a bound caller").to_path_buf();
            supervisor.send_to(ANSWER.as_bytes(), &to).expect("the answer");
        });
        let mine = dir.join("reply-answered.sock");
        assert!(ask_via(&live_path.to_string_lossy(), &mine).is_ok());
        answering.join().expect("the answering thread");
        assert!(!mine.exists(), "{} was left behind on the answered path", mine.display());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Two calls from one process never share a reply address**, so one
    /// call's clean-up cannot delete another's live socket.
    #[cfg(unix)]
    #[test]
    fn concurrent_calls_do_not_share_a_reply_socket() {
        let dir = std::env::temp_dir().join(format!("rivet-facts-pair-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temporary directory");
        let path = dir.join("facts");
        let supervisor = std::os::unix::net::UnixDatagram::bind(&path).expect("bound");

        let answering = std::thread::spawn(move || {
            let mut buf = [0_u8; 512];
            for _ in 0..2 {
                let (_, from) = supervisor.recv_from(&mut buf).expect("a request");
                let to = from.as_pathname().expect("a bound caller").to_path_buf();
                supervisor.send_to(ANSWER.as_bytes(), &to).expect("the answer");
            }
        });
        let socket = path.to_string_lossy().into_owned();
        let other = socket.clone();
        let first = std::thread::spawn(move || ask_at(&other));
        let second = ask_at(&socket);
        let first = first.join().expect("the first caller");
        answering.join().expect("the answering thread");
        assert!(first.is_ok(), "the first call: {first:?}");
        assert!(second.is_ok(), "the second call: {second:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
