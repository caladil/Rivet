//! `rivet get status` — **what the supervisor is doing, right now.**
//! `ADR-032` §3.2's query socket, and the answer it carries.
//!
//! `PLAN.md` §28.5 Phase 1 item 7. `ADR-032` §6's closing note is why this and
//! the socket are one change: *"the query socket is what `rivet status` needs,
//! and `rivet status` is what makes the socket worth having — they land together
//! or neither is testable."*
//!
//! # It is `get status`, and the ADR says `rivet status`
//!
//! **Ruled by the maintainer 2026-08-26**, and it corrects `ADR-032` §6's own
//! spelling the way `ADR-006` §7.4 corrected `rivet schema` the day before: a
//! bare noun is not a command, and a word the login shell matches itself is
//! invisible to the catalogue every adapter reads. As a subject of `get` it
//! costs nothing new — an operator who knows `get log` predicts it, and §3.3
//! item 1's *"carry an operation rather than a second command language"* is
//! satisfied by the operation already existing.
//!
//! # The one fact that is only in the supervisor's memory
//!
//! Every other `get` subject reads something that is *written down*: the journal
//! is a file, storage is the kernel's table, the schema is computed. **This one
//! is not.** Which services are running, what their pids are, how many times one
//! has been restarted and whether it has declared readiness exist only in
//! `cmd/rivet-sysd`'s own `Supervisor`, and no file holds them — `AGENTS.md`
//! §5's *derive, never store* is why, and it is right.
//!
//! That is the whole justification for a socket. **Ruled 2026-08-26: only what
//! the supervisor alone knows crosses it.** A general read-only channel was
//! considered and refused — it would put the journal and the storage table
//! behind a process `ADR-017` §3.1 explicitly permits to be dead, and duplicate
//! two paths that already work without it.
//!
//! # The wire format is here, both halves, and that is deliberate
//!
//! **This workspace has no JSON parser and that is a ruling rather than a gap**
//! (`rivet-config`'s `Cargo.toml` records it). `rivet get log` sidesteps the
//! problem by returning the journal's lines as *strings* for `nu` to parse; that
//! is fine for records an operator pipes, and wrong for a result whose fields
//! `--output text` has to render.
//!
//! So [`Status::render`] and [`Status::parse`] are **one definition of one
//! format**, in the domain, with a round trip asserted. The supervisor renders;
//! the CLI parses; neither owns the format and there is nothing for them to
//! disagree about. Tab-separated because a service name cannot contain a tab —
//! `ServiceName` refuses a path separator and every other control character —
//! so there is no escaping to get wrong, which is the property JSON would have
//! cost a parser to buy.

use super::machine::Observed;
use super::{ErrorCode, Field, OperationError, Verb};
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "status";

/// **Where the supervisor listens.** `ADR-032` §3.2, `0600` per §3.3 item 2.
///
/// Beside `ADR-022` §11's ingest socket and deliberately not the same one — §3.4
/// is a whole section on why widening that one is the tempting trap: its `0666`
/// is a *ruled consequence* of any process legitimately filing a record, and
/// none of that reasoning applies to reading the supervisor's state.
pub const QUERY_SOCKET_PATH: &str = "/run/rivet/query";

/// **Proof that something reached this machine on THIS boot.**
///
/// `PLAN.md` §104.20. Written by `cmd/rivet-mcpd` the first time a caller
/// completes a mutually-authenticated handshake, and read by `cmd/rivet-initd`
/// before it will confirm a trial boot.
///
/// # On `/run`, and that is the whole design
///
/// `/run` is a tmpfs (`ADR-017` §3.1.5), so this file cannot survive a reboot —
/// and it must not. The question it answers is *"has anybody reached the
/// machine since it came up"*, and a marker that outlived the boot would
/// confirm the NEXT one on the strength of the last. That is the same mistake
/// `PLAN.md` §73.2 records about polling `finished` on an install: a true
/// answer about the wrong occasion.
///
/// # Why the file's contents do not matter
///
/// Only its existence does, so nothing here parses. What it means is *one
/// pinned key completed a handshake*, and that fact has no degrees.
pub const REACHED_MARKER_PATH: &str = "/run/rivet/reached";

/// **What a caller sends.** `ADR-032` §3.3 item 1: *"the socket should carry an
/// operation and its arguments"* — so it carries the words an operator types,
/// and this is the one place they are spelled.
///
/// **It is a whole operation and not the bare subject**, even though `status` is
/// the only thing that crosses today. A request of `status` alone would be a
/// second command language with one word in it, which is exactly what §3.3 item
/// 1 ruled against; a request of `get status` is a Rivet command, and the day a
/// second one is ruled across it needs no new grammar.
pub const REQUEST: &str = "get status\n";

/// The most a request may be. A request is one short line today; the bound
/// exists so a listener cannot be made to allocate by a caller.
pub const REQUEST_MAX_BYTES: usize = 256;

/// The most a response may be, and it is generous rather than tight: a machine
/// with a hundred applications is inside it, and a supervisor that produced more
/// would be reporting something this operation does not describe.
pub const RESPONSE_MAX_BYTES: usize = 64 * 1024;

/// The fields this subject's result carries.
pub(super) const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was read. Always `status` here.",
    },
    Field {
        name: "phase",
        description: "What the supervisor is doing as a whole. `running` is its normal state for \
                      as long as the machine is up: on every pass it reads the desired-state \
                      document, observes what is actually running, and starts or stops services \
                      until the two agree (ADR-017 §3). That loop never finishes, so there is no \
                      `converged`; whether one service is where the document wants it is that \
                      service's own `enabled` and `state`. `shutting-down` means every service is \
                      being brought down. `finished` means all of them are down, and is never seen \
                      over the control plane, which is down by then. A build older than this one \
                      says `converging` for the state now called `running` (ADR-032 §8).",
    },
    Field {
        name: "version",
        description: "THE RUNNING IMAGE'S OWN VERSION — the same string `rivet --version` prints, \
                      read once and carried through rather than recomputed. ABSENT on a \
                      supervisor from before this field existed, which said nothing about its \
                      version rather than reporting an empty one.",
    },
    Field {
        name: "health",
        description: "WHETHER THIS MACHINE IS DOING ITS JOB — `healthy`, `degraded`, `down` or \
                      `unknown` (ADR-045 §4). It is the WORST state among the services the \
                      document marks `critical`, ordered `down > unknown > degraded > healthy`, \
                      and it is NOT the worst of all of them: a `degraded` weather probe must not \
                      make a machine `degraded` when everything that matters is fine (ADR-045 \
                      §4.4). SO A MACHINE WITH NO `critical` SERVICES ANSWERS `healthy` — that is \
                      the honest answer to an operator who has told it that nothing matters, and \
                      the remedy is to mark what matters. `unknown` outranks `degraded` \
                      deliberately: a critical service that cannot answer has stopped \
                      participating, where a `degraded` one is still talking to you. Every \
                      service's own state is in `services[].health` and none of it is hidden.",
    },
    Field {
        name: "mode",
        description: "WHAT THIS MACHINE IS FOR, and the document's `mode` key DECIDES it \
                      (`.rulings/` R-164): `kiosk` when the document names `mode = \"kiosk\"`, \
                      or when it names no `mode` at all and has a `[kiosk]` table — `headless` \
                      the same way — or `unknown` when the machine could not read its own \
                      document. A `[kiosk]` table's PRESENCE is configuration, never the \
                      decision: a document naming `mode = \"headless\"` answers `headless` here \
                      even if it carries a `[kiosk]` table, and that table is neither cleared \
                      nor refused (R-165) — it is simply not acted on. NEVER INFER THIS FROM \
                      `get desired-state`: before 2026-09-01 the fleet page did, by looking for \
                      the text `[kiosk]` in the document, which is true of a comment mentioning \
                      it and of a table with no keys, and false for a machine whose document \
                      would not parse — the answer there was `headless`, stated confidently, \
                      about a machine nobody had read.",
    },
    Field {
        name: "locked",
        description: "KIOSK MACHINES ONLY, and ABSENT otherwise: whether `[kiosk] locked` is \
                      set, which is what stops a person leaving the application from the kiosk \
                      (PLAN.md §108). It is BESIDE `mode` rather than a third word for it, \
                      because they answer different questions and a caller that wanted only \
                      `is this a kiosk` would otherwise be matching prefixes — which is the \
                      string-matching this field exists to end.",
    },
    Field {
        name: "auto_run",
        description: "KIOSK MACHINES ONLY, and ABSENT when the table names none: the \
                      application `[kiosk] auto-run` says a launcher starts instead of showing \
                      its menu. A machine that is `locked` with no `auto_run` is locked to an \
                      application it was never told, and its launcher puts a fault screen up — \
                      the two fields say so together and nothing here predicts the launcher.",
    },
    Field {
        name: "kiosk_unconfigured_because",
        description: "Present only when `mode` is `kiosk` and the document carries no `[kiosk]` \
                      table (R-164): a kiosk was asked for and nothing configures it, in the \
                      machine's own words. NOT A REFUSAL — the machine behaves exactly as it \
                      does today with no `[kiosk]` table (no lock, no auto-run) while still \
                      saying `kiosk` here, so this field is what tells a reader the two facts \
                      disagree.",
    },
    Field {
        name: "display_master",
        description: "WHO HOLDS THE SCREEN, and ABSENT on a machine where nobody does: the \
                      running service whose `DisplayRole` is `Arbiter` — it holds DRM master and \
                      leases the display out, and today that is `rivet-launcher`. The role is \
                      decided by `connects = [\"rivet-sysd:kiosk\"]`, the same declaration the \
                      supervisor chowns the kiosk socket by, so the program given the socket is \
                      the program given the card. TWO CLAIMANTS MEANS NOBODY, deliberately — a \
                      machine in that state has no launcher that can act, and this field is \
                      absent rather than guessing between them.",
    },
    Field {
        name: "display_leased",
        description: "WHAT IS ACTUALLY DRAWING, and ABSENT when nothing is: the application the \
                      arbiter has leased the display to. THIS IS LIVE STATE AND IS NOT \
                      `auto_run`. `auto_run` is what the document asks for; this is what the \
                      launcher did, and they disagree whenever a selection has not happened yet, \
                      has been revoked, or was made by a person at the menu. NEVER INFER THIS \
                      FROM `auto_run`, FROM `devices = [\"display\"]`, OR FROM WHICH KIOSK \
                      APPLICATION IS `started` — every one of those is a declaration, several \
                      services carry them at once, and a reader that guessed would be wrong on \
                      exactly the machine it mattered on. `PLAN.md` §133 is a rollback caused by \
                      believing a display was down when it was drawing, and `mode` above carries \
                      the other half of the same lesson.",
    },
    Field {
        name: "mode_unknown_because",
        description: "Why `mode` is `unknown`, in the machine's own words. ABSENT otherwise. \
                      Reading this before acting on an absent `[kiosk]` is the whole point of \
                      the three-valued answer.",
    },
    Field {
        name: "services",
        description: "One entry per service the supervisor holds, image-defined and installed \
                      alike, in the order it holds them.",
    },
];

/// **The three words `mode` may be**, defined once.
///
/// `ADR-011` in miniature: the supervisor produces one of these and the CLI
/// renders it, and a second spelling in either would be a second answer. They
/// are `pub` so `cmd/rivet-sysd` names the constant rather than the literal.
pub const MODE_KIOSK: &str = "kiosk";
/// See [`MODE_KIOSK`].
pub const MODE_HEADLESS: &str = "headless";
/// See [`MODE_KIOSK`]. **Not a failure and not a default** — it is the answer
/// for a machine whose document is there and could not be read, and the reason
/// this is three-valued rather than a boolean: `headless` would be a claim
/// about a document nobody managed to parse.
pub const MODE_UNKNOWN: &str = "unknown";

/// **What this machine is for**, as the supervisor read it out of the
/// desired-state document.
///
/// # Why the supervisor answers this, and not `get desired-state`
///
/// Because the document is *text* and the mode is a *fact*, and every caller
/// that had to get from one to the other was writing its own parser. The fleet
/// page's was `doc.contents.includes("[kiosk]")` — which is true of a comment,
/// true of a table with no keys, and **false for a machine whose document will
/// not parse**, where it then reported `headless` with no hedge at all.
///
/// # Why the supervisor rather than a new port on `Observed`
///
/// `rivet-core` has an empty `[dependencies]` table by rule (`AGENTS.md`
/// §10.1), so it cannot parse TOML and could only have string-matched — the
/// defect, moved one crate inward. The supervisor **already parses the
/// document**, and it already reports document-derived facts through this same
/// answer: [`ServiceStatus::enabled`] is one. So this is the shape that exists
/// rather than a new one, and it costs no new port on the twenty-two
/// implementors of [`Observed`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KioskMode {
    /// One of [`MODE_KIOSK`], [`MODE_HEADLESS`] or [`MODE_UNKNOWN`].
    ///
    /// A `String` and not an enum, for the reason [`Status::phase`] is one:
    /// **the word is the supervisor's and is carried through unchanged**
    /// (`AGENTS.md` §5). A domain enum would silently map a word this build did
    /// not expect onto the nearest one it did.
    pub mode: String,
    /// `[kiosk] locked`. Meaningless off a kiosk and reported only there.
    pub locked: bool,
    /// `[kiosk] auto-run`, where the table names one.
    pub auto_run: Option<String>,
    /// Why the mode could not be established. `Some` exactly when the mode is
    /// [`MODE_UNKNOWN`].
    pub why_unknown: Option<String>,
    /// **Why `mode` is `kiosk` with nothing configuring it.** `Some` exactly
    /// when the document's `mode` asked for [`MODE_KIOSK`] and carries no
    /// `[kiosk]` table (`.rulings/` R-164). Not a failure: the machine behaves
    /// exactly as it does today with no `[kiosk]` table (`locked: false`, no
    /// `auto_run`) — this field is what makes the mismatch between what was
    /// asked and what is configured visible rather than silent.
    pub why_unconfigured: Option<String>,
}

impl KioskMode {
    /// **The answer a supervisor that was never told gives**, and it is
    /// deliberately not `headless`.
    ///
    /// This is what [`Status`] carries before anything sets it, so a code path
    /// that forgets to read the document reports *"nobody said"* rather than
    /// asserting that a kiosk is a headless box (`AGENTS.md` §3 — a stub
    /// refuses; it never fakes success).
    #[must_use]
    pub fn unknown(because: impl Into<String>) -> Self {
        Self {
            mode: MODE_UNKNOWN.to_string(),
            locked: false,
            auto_run: None,
            why_unknown: Some(because.into()),
            why_unconfigured: None,
        }
    }

    /// A machine whose `mode` is `headless`, or whose document names no
    /// `mode` and carries no `[kiosk]` table. **A real reading, not an
    /// absence** — and, since R-164, the answer whenever `mode = "headless"`
    /// is written, whether or not a `[kiosk]` table is also present: `mode`
    /// decides, the table is configuration `headless` does not act on.
    #[must_use]
    pub fn headless() -> Self {
        Self {
            mode: MODE_HEADLESS.to_string(),
            locked: false,
            auto_run: None,
            why_unknown: None,
            why_unconfigured: None,
        }
    }

    /// A machine whose `mode` is `kiosk` and whose `[kiosk]` table configures
    /// it.
    #[must_use]
    pub fn kiosk(locked: bool, auto_run: Option<String>) -> Self {
        Self {
            mode: MODE_KIOSK.to_string(),
            locked,
            auto_run,
            why_unknown: None,
            why_unconfigured: None,
        }
    }

    /// **A machine whose `mode` is `kiosk` and whose document carries no
    /// `[kiosk]` table to configure it.** `.rulings/` R-164: tolerated and
    /// reported, not refused. `mode` still reads `kiosk` — that is the fact
    /// worth surfacing — while `locked` and `auto_run` carry the same values
    /// an empty or absent table always has, because there is nothing here to
    /// lock to or auto-run.
    #[must_use]
    pub fn kiosk_unconfigured() -> Self {
        Self {
            mode: MODE_KIOSK.to_string(),
            locked: false,
            auto_run: None,
            why_unknown: None,
            why_unconfigured: Some(
                "this document's mode is \"kiosk\" but it carries no [kiosk] table, so nothing \
                 configures what the screen should show; the machine behaves as it does today \
                 with no kiosk configured"
                    .to_string(),
            ),
        }
    }

    /// Whether this machine is in kiosk mode. The predicate a caller wants,
    /// **so that no caller writes `mode == "kiosk"` for itself.**
    #[must_use]
    pub fn is_kiosk(&self) -> bool {
        self.mode == MODE_KIOSK
    }
}

/// **What the supervisor knows about one service.**
///
/// A domain type rather than `cmd/rivet-sysd`'s own `Managed`: `ADR-014` §3
/// forbids `rivet-core` naming anything in a binary crate, and the supervisor
/// maps onto this on the way out. That mapping is the only place the two
/// vocabularies meet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceStatus {
    pub name: String,
    /// What the document and the definition together say (`ADR-017` §6).
    pub enabled: bool,
    /// The supervisor's own word for what it is doing — `idle`, `started`,
    /// `done`, `failed`. **Its vocabulary, carried through unchanged**
    /// (`AGENTS.md` §5): a second spelling here would be a second answer.
    pub state: String,
    /// `None` unless it is running. Absent rather than zero, because zero is a
    /// pid.
    pub pid: Option<i32>,
    /// **How many times it has been RESTARTED**, which is `0` for a service
    /// that started first time and is still running.
    ///
    /// Not "how many attempts": the supervisor's counter is incremented by
    /// `schedule_retry` and by nothing else, so a first spawn leaves it at zero.
    /// The journal's `attempt` column is this number **plus one**, because there
    /// it labels the spawn about to happen — and the two must not be confused,
    /// which is why this one is named for what it counts rather than for what
    /// the journal calls its neighbour.
    pub restarts: u32,
    /// **Why this service is not running**, when the supervisor knows.
    ///
    /// `PLAN.md` §104.22, and it is `§104.7`'s open item closed. A status that
    /// said `state: idle` and nothing else made every refusal look identical:
    /// an application refused for a device collision, one refused because its
    /// signed header did not verify and one that simply is not enabled all read
    /// the same, and the difference was only ever in `get log … all` — behind a
    /// two-hundred-record default that hides the beginning of a boot
    /// (`§104.10`).
    ///
    /// **Absent when there is nothing to say.** A running service carries none,
    /// and a field that was always present and usually empty is a field a
    /// reader learns to skip. It is `Option` for `pid`'s reason: absence is a
    /// different claim from a value.
    ///
    /// This is the supervisor's own sentence, carried through unchanged
    /// (`AGENTS.md` §5). It is deliberately not a code: the reasons are prose
    /// because they are read by a person deciding what to do next, and a
    /// vocabulary would have to be extended every time a new refusal appeared.
    pub reason: Option<String>,
    /// Whether it has declared readiness over `NOTIFY_SOCKET` (`ADR-017` §3.3).
    /// **`false` for a service that never declares one**, which is most of them
    /// — this is not a health check.
    pub ready: bool,
    /// **What this service last answered when the supervisor asked** —
    /// `ADR-045` §3, added 2026-09-08.
    ///
    /// One of the four words in [`crate::health::State`]. **`unknown` until
    /// there is a successful answer on record** (§4.2), which is what a service
    /// that has just started, one that speaks no health, and one that has
    /// stopped answering all read as — and the difference between the last two
    /// is in the journal, where the supervisor says it asked and got nothing.
    ///
    /// It is deliberately **not** an `Option`: `unknown` is a state and a hole
    /// in the data is not, and an `Option` beside a four-valued word would be a
    /// second way to spell the same absence.
    pub health: crate::health::State,
    /// **The sentence the service sent with that state**, and `None` when it
    /// sent none — `ADR-045` §5.
    ///
    /// Four states are the right vocabulary for *"is this healthy"* and are not
    /// enough for *"the battery is getting low"*, because that sentence is
    /// useless without a sensor's name in it.
    ///
    /// **UNTRUSTED DISPLAY TEXT.** An application wrote it; carry it verbatim,
    /// render it as text and never as markup, and **never branch on its
    /// contents** (§5.3) — the moment a reader matches on the words,
    /// applications have an accidental protocol nobody designed and nobody can
    /// change.
    pub caption: Option<String>,
    /// **Whether the operator's document marks this service `critical`** —
    /// `ADR-045` §4.
    ///
    /// Reported so that a reader can tell *why* the machine's `health` above is
    /// what it is. A machine answering `healthy` with a `down` service in the
    /// list is not a contradiction, and this column is what says so.
    pub critical: bool,
}

/// **The whole answer.** See [`Status::render`] for the wire format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub phase: String,
    /// **The image's own version string** — the same one `rivet --version`
    /// carries (`ADR-027` §10), read once and carried through rather than
    /// recomputed. `R33`, `.rulings/` R-163.
    ///
    /// `None` rather than a sentinel word, for [`Self::display_master`]'s
    /// reason: the only way to reach it is a supervisor from before
    /// 2026-09-13, which sent no `version` line at all, and that is a
    /// different claim from a build that somehow has no version.
    pub version: Option<String>,
    /// **What this machine is for.** See [`KioskMode`], and note that it is not
    /// an `Option`: the three-valued word already carries *"nobody said"*, and
    /// an `Option` beside it would be a second way to spell the same absence.
    pub kiosk: KioskMode,
    /// **The running arbiter — DRM master.** `None` when nobody holds the
    /// screen: no arbiter is running, or two services claim the role and the
    /// supervisor therefore recognises neither.
    ///
    /// Beside [`Self::kiosk`] rather than inside it, and that placement is the
    /// point: `KioskMode` is *carried through, not recomputed* from the
    /// document, while this and [`Self::display_leased`] are **what is true of
    /// the running machine**. A declaration and an observation in one struct
    /// field would be the confusion this pair exists to end.
    pub display_master: Option<String>,
    /// **The application currently holding a lease**, and `None` when none is.
    /// See [`Self::display_master`] for why these are not in `kiosk`, and the
    /// `display_leased` entry in `OUTPUTS` for what must never be substituted
    /// for it.
    pub display_leased: Option<String>,
    /// **Whether this machine is doing its job** — `ADR-045` §4's fold, which
    /// the supervisor computes with [`crate::health::machine`] and this carries.
    ///
    /// **Folded by the producer, not by the reader.** The rule is normative and
    /// lives in one function; a `get status` that shipped the rows and let the
    /// CLI, the portal and the fleet server each fold them would be three
    /// answers to a question `ADR-011` says has one — and the ordering clause
    /// most likely to be got wrong (`unknown` above `degraded`) is exactly the
    /// one each of them would have had to reimplement.
    pub machine: crate::health::State,
    pub services: Vec<ServiceStatus>,
}

/// A response that could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusError {
    /// A line whose first field is not one this build knows.
    UnknownLine { line: usize, kind: String },
    /// A line with the wrong number of fields for its kind.
    Fields { line: usize, kind: String, found: usize, wanted: usize },
    /// A field that should have been a number or a boolean and was not.
    Value { line: usize, field: &'static str, found: String },
    /// No `phase` line at all. A response without one is not a status.
    NoPhase,
}

impl core::fmt::Display for StatusError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownLine { line, kind } => write!(
                f,
                "line {line} begins with `{kind}`, which this build does not know. The supervisor \
                 is newer than this CLI, or the response is not a status."
            ),
            Self::Fields { line, kind, found, wanted } => write!(
                f,
                "line {line} is a `{kind}` with {found} fields and this build wants {wanted}."
            ),
            Self::Value { line, field, found } => {
                write!(f, "line {line}'s `{field}` is `{found}`, which is not a value it may take.")
            }
            Self::NoPhase => write!(
                f,
                "the response carries no `phase` line, so it does not say what the supervisor is \
                 doing. A status without one is not an answer."
            ),
        }
    }
}

const PHASE: &str = "phase";
const SVC: &str = "svc";
/// The line kind carrying [`KioskMode`]. Added 2026-09-01.
///
/// **A line of its own rather than three more fields on `phase`**, because the
/// phase line is what a reader looks at first and every field added to it is a
/// field a truncated read can lose. A missing `kiosk` line parses as
/// [`KioskMode::unknown`], which is the honest reading of a supervisor that did
/// not send one — see [`Status::parse`].
const KIOSK: &str = "kiosk";
/// The line kind carrying who holds the screen and who is drawing through it.
/// Added 2026-09-05.
///
/// **A line of its own, for `KIOSK`'s reason and one more.** These two facts are
/// observations of the running machine and the `kiosk` line is the document
/// read back; keeping them apart on the wire is the same separation the struct
/// makes, so a reader cannot pick up a declaration believing it read a
/// measurement.
///
/// **A missing `display` line is two `None`s, and that is honest** — it means a
/// supervisor older than the CLI asking, which said nothing about the screen
/// rather than saying nothing holds it. The absence is indistinguishable from
/// *nobody is drawing* only in the sense that both mean **do not put a badge
/// up**, which is the correct behaviour for both.
const DISPLAY: &str = "display";
/// The line kind carrying the running image's version. Added 2026-09-13 —
/// `R33`, `.rulings/` R-163.
///
/// **After `display` and before the services, not among the first two.**
/// `phase` and `machine` are first because they change tick to tick and a
/// truncated read must not lose them; `kiosk` and `display` are next because
/// they are what an operator asking `get status` usually wants. A build's
/// version is none of those — it is fixed for the life of the boot — so it
/// sits after everything that can change and before the list that can grow
/// without bound.
///
/// **A missing `version` line is `None`, never an empty string.** The only way
/// to reach that is a supervisor from before this line existed, which is the
/// same version-skew reading [`KIOSK`] and [`MACHINE`] already give — an
/// older supervisor said nothing, and an empty string would look like an
/// answer rather than its absence.
const VERSION: &str = "version";
/// The line kind carrying `ADR-045` §4's fold. Added 2026-09-08.
///
/// **A line of its own, for [`KIOSK`]'s reason.** It is one word that a reader
/// looks at before anything else, and every field added to the `phase` line is a
/// field a truncated read can lose.
///
/// **A missing `machine` line is `unknown`, never `healthy`** — see
/// [`Status::parse`]. That is the same choice `kiosk` makes for the same reason:
/// a supervisor older than the CLI asking said nothing about health, and
/// answering `healthy` on its behalf is the *trusted and wrong* failure
/// (`ADR-006` §7) in the one place a version skew can produce it.
const MACHINE: &str = "machine";

/// Read a health word off the wire, refusing anything outside the closed
/// vocabulary — `ADR-045` §4.
///
/// **Refused rather than defaulted.** The set is closed and adding to it is an
/// amendment to that ADR, so a word this build does not know means the response
/// is not a status or the supervisor is newer than this CLI. Either way, a
/// reader that quietly substituted `unknown` would be reporting *nobody said*
/// about a machine that did say.
fn health_word(
    line: usize,
    field: &'static str,
    text: &str,
) -> Result<crate::health::State, StatusError> {
    use crate::health::State;
    match text {
        "healthy" => Ok(State::Healthy),
        "degraded" => Ok(State::Degraded),
        "down" => Ok(State::Down),
        "unknown" => Ok(State::Unknown),
        other => Err(StatusError::Value {
            line,
            field,
            found: other.to_string(),
        }),
    }
}

/// Replace the characters this frame is built from, so a sentence cannot break
/// the record carrying it.
///
/// **Replaced rather than refused.** A supervisor that declined to report a
/// service because its reason contained a tab would be withholding the one
/// thing the reader wanted, over a character.
fn scrub(text: &str) -> String {
    text.replace(['\t', '\n', '\r'], " ")
}

impl Status {
    /// **The wire form.** One line per fact, tab-separated:
    ///
    /// ```text
    /// phase\trunning
    /// machine\tdegraded
    /// kiosk\tkiosk\ttrue\taura-renderer\t-\t-
    /// display\trivet-launcher\taura-renderer
    /// version\t0.1.86
    /// svc\trivet-netd\ttrue\tstarted\t214\t1\ttrue\t-\thealthy\tfalse\t-
    /// svc\ttempmon\ttrue\tstarted\t311\t0\ttrue\t-\tdegraded\ttrue\tone sensor is quiet
    /// ```
    ///
    /// `-` is *absent*, and it is unambiguous because the only optional field is
    /// a pid and a pid is never `-`.
    ///
    /// **The phase comes first and exactly once.** A reader that got services
    /// and no phase would have a list and no idea whether the supervisor was
    /// still starting them.
    ///
    /// **The `kiosk` line comes second, before the services**, so a truncated
    /// read loses services rather than the mode — a machine with a hundred of
    /// them still says what it is for in its first two lines.
    pub fn render(&self) -> String {
        let mut out = format!("{PHASE}\t{}\n", self.phase);
        // **Second, immediately after the phase.** It is one word answering the
        // question an operator asked the machine for, and a truncated read
        // should lose a service before it loses that.
        out.push_str(&format!("{MACHINE}\t{}\n", self.machine.word()));
        out.push_str(&format!(
            "{KIOSK}\t{}\t{}\t{}\t{}\t{}\n",
            scrub(&self.kiosk.mode),
            self.kiosk.locked,
            self.kiosk
                .auto_run
                .as_deref()
                .map_or_else(|| "-".to_string(), scrub),
            // Scrubbed for the reason a service's `reason` is: it is prose
            // written for a person and this frame is tab-separated.
            self.kiosk
                .why_unknown
                .as_deref()
                .map_or_else(|| "-".to_string(), scrub),
            // **Appended, not inserted** — `ADR-045`'s three fields on `svc`
            // set the precedent: a widened line keeps every earlier index
            // where it was, which is what makes it cheaper to read than a
            // reordered one. `.rulings/` R-164.
            self.kiosk
                .why_unconfigured
                .as_deref()
                .map_or_else(|| "-".to_string(), scrub),
        ));
        // **After `kiosk` and before the services**, for the reason the `kiosk`
        // line is where it is: a truncated read should lose a service rather
        // than lose what the machine is doing with its screen.
        out.push_str(&format!(
            "{DISPLAY}\t{}\t{}\n",
            self.display_master
                .as_deref()
                .map_or_else(|| "-".to_string(), scrub),
            self.display_leased
                .as_deref()
                .map_or_else(|| "-".to_string(), scrub),
        ));
        // **After `display` and before the services** — see [`VERSION`] for why
        // it sits here rather than among the first two lines.
        out.push_str(&format!(
            "{VERSION}\t{}\n",
            self.version.as_deref().map_or_else(|| "-".to_string(), scrub),
        ));
        for s in &self.services {
            out.push_str(&format!(
                "{SVC}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                s.name,
                s.enabled,
                s.state,
                s.pid.map_or_else(|| "-".to_string(), |p| p.to_string()),
                s.restarts,
                s.ready,
                // **`-` for absent, as `pid` does**, and the sentence SCRUBBED
                // of the two characters that would corrupt the frame. A reason
                // is prose written for a person, so it may contain anything a
                // sentence contains — and this format is tab-separated,
                // newline-terminated. A refusal that split its own record into
                // two lines would take the whole answer down with it.
                s.reason
                    .as_deref()
                    .map_or_else(|| "-".to_string(), scrub),
                // **After `reason`, so the columns before it did not move.**
                // `ADR-045`'s three arrived together on 2026-09-08; appending
                // them keeps every earlier index where it was, which is what
                // makes a widened line cheaper to read than a reordered one.
                s.health.word(),
                s.critical,
                // **A caption is an application's own sentence**, so it is
                // scrubbed exactly as `reason` is and for the same reason: this
                // frame is tab-separated and newline-terminated, and a sentence
                // may contain anything a sentence contains.
                s.caption
                    .as_deref()
                    .map_or_else(|| "-".to_string(), scrub),
            ));
        }
        out
    }

    /// **The wire form, read back.** [`Self::render`]'s inverse, and the round
    /// trip is asserted — see this module's documentation for why both halves
    /// live here.
    ///
    /// **An unknown line kind is refused rather than skipped.** A CLI older than
    /// the supervisor it is talking to must say so, not quietly report a status
    /// missing whatever it did not understand — which is `ADR-006` §7's *trusted
    /// and wrong* in the one place a version skew can produce it.
    pub fn parse(text: &str) -> Result<Self, StatusError> {
        let mut phase: Option<String> = None;
        let mut services = Vec::new();
        // **Absent by default**, which is what a supervisor from before
        // 2026-09-05 leaves them — see [`DISPLAY`] for why that is the honest
        // reading rather than a loss.
        let mut display_master: Option<String> = None;
        let mut display_leased: Option<String> = None;
        // **`None` by default**, which is what a supervisor from before
        // 2026-09-13 leaves it — see [`VERSION`] for why that is the honest
        // reading rather than a loss.
        let mut version: Option<String> = None;
        // **`unknown`, never `healthy`, when no `machine` line arrives.** See
        // [`MACHINE`]: a supervisor older than the CLI asking said nothing about
        // health, and `unknown` is precisely the word for *nobody said*
        // (`ADR-045` §4).
        let mut machine = crate::health::State::Unknown;
        // **A missing `kiosk` line is `unknown`, never `headless`.** The only
        // way to reach this is a supervisor from before 2026-09-01 — the two
        // halves ship in one image, so it takes an A/B slot older than the CLI
        // asking. Answering `headless` there would be the defect this line
        // exists to end, arriving through a version skew instead of a string
        // match.
        let mut kiosk = KioskMode::unknown(
            "this machine's supervisor sent no kiosk line, which means it is older than the CLI \
             asking. The document was not consulted and NOTHING here says this machine is not a \
             kiosk",
        );

        for (n, line) in text.lines().enumerate() {
            let line_no = n + 1;
            if line.trim().is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.first().copied().unwrap_or_default() {
                PHASE => {
                    if fields.len() != 2 {
                        return Err(StatusError::Fields {
                            line: line_no,
                            kind: PHASE.to_string(),
                            found: fields.len(),
                            wanted: 2,
                        });
                    }
                    phase = Some(fields[1].to_string());
                }
                MACHINE => {
                    if fields.len() != 2 {
                        return Err(StatusError::Fields {
                            line: line_no,
                            kind: MACHINE.to_string(),
                            found: fields.len(),
                            wanted: 2,
                        });
                    }
                    machine = health_word(line_no, "machine", fields[1])?;
                }
                KIOSK => {
                    if fields.len() != 6 {
                        return Err(StatusError::Fields {
                            line: line_no,
                            kind: KIOSK.to_string(),
                            found: fields.len(),
                            wanted: 6,
                        });
                    }
                    kiosk = KioskMode {
                        mode: fields[1].to_string(),
                        locked: boolean(line_no, "locked", fields[2])?,
                        auto_run: match fields[3] {
                            "-" => None,
                            text => Some(text.to_string()),
                        },
                        why_unknown: match fields[4] {
                            "-" => None,
                            text => Some(text.to_string()),
                        },
                        why_unconfigured: match fields[5] {
                            "-" => None,
                            text => Some(text.to_string()),
                        },
                    };
                }
                DISPLAY => {
                    if fields.len() != 3 {
                        return Err(StatusError::Fields {
                            line: line_no,
                            kind: DISPLAY.to_string(),
                            found: fields.len(),
                            wanted: 3,
                        });
                    }
                    display_master = match fields[1] {
                        "-" => None,
                        text => Some(text.to_string()),
                    };
                    display_leased = match fields[2] {
                        "-" => None,
                        text => Some(text.to_string()),
                    };
                }
                VERSION => {
                    if fields.len() != 2 {
                        return Err(StatusError::Fields {
                            line: line_no,
                            kind: VERSION.to_string(),
                            found: fields.len(),
                            wanted: 2,
                        });
                    }
                    version = match fields[1] {
                        "-" => None,
                        text => Some(text.to_string()),
                    };
                }
                SVC => {
                    if fields.len() != 11 {
                        return Err(StatusError::Fields {
                            line: line_no,
                            kind: SVC.to_string(),
                            found: fields.len(),
                            wanted: 11,
                        });
                    }
                    services.push(ServiceStatus {
                        name: fields[1].to_string(),
                        enabled: boolean(line_no, "enabled", fields[2])?,
                        state: fields[3].to_string(),
                        pid: match fields[4] {
                            "-" => None,
                            text => Some(text.parse().map_err(|_| StatusError::Value {
                                line: line_no,
                                field: "pid",
                                found: text.to_string(),
                            })?),
                        },
                        restarts: fields[5].parse().map_err(|_| StatusError::Value {
                            line: line_no,
                            field: "restarts",
                            found: fields[5].to_string(),
                        })?,
                        ready: boolean(line_no, "ready", fields[6])?,
                        reason: match fields[7] {
                            "-" => None,
                            text => Some(text.to_string()),
                        },
                        health: health_word(line_no, "health", fields[8])?,
                        critical: boolean(line_no, "critical", fields[9])?,
                        caption: match fields[10] {
                            "-" => None,
                            text => Some(text.to_string()),
                        },
                    });
                }
                other => {
                    return Err(StatusError::UnknownLine {
                        line: line_no,
                        kind: other.to_string(),
                    });
                }
            }
        }

        Ok(Self {
            phase: phase.ok_or(StatusError::NoPhase)?,
            version,
            kiosk,
            display_master,
            display_leased,
            machine,
            services,
        })
    }

    /// The structured result, whose keys are `OUTPUTS`.
    pub fn value(&self) -> Value {
        let mut fields = vec![
            ("subject", Value::str(SUBJECT)),
            ("phase", Value::str(self.phase.clone())),
        ];
        // **Absent on a supervisor older than this field** (`R33`,
        // `.rulings/` R-163) — the same convention `display_master` and
        // `display_leased` already follow, and for the same reason: a
        // machine that never sends the word is a different claim from one
        // whose version happens to be empty.
        if let Some(version) = &self.version {
            fields.push(("version", Value::str(version.clone())));
        }
        fields.extend([
            // **Always present, never omitted.** `unknown` is a state and every
            // other optional field here is absent when there is nothing to say;
            // this one always has something to say, because a machine that has
            // been asked nothing still answers *nobody said*.
            ("health", Value::str(self.machine.word())),
            ("mode", Value::str(self.kiosk.mode.clone())),
        ]);
        // **`locked` and `auto_run` only on a kiosk.** `locked: false` beside
        // `mode: headless` would invite a reader to conclude the machine is a
        // kiosk somebody left unlocked; the field is meaningless off a kiosk
        // and absence says so, which is the convention `pid` and `reason`
        // already follow above.
        if self.kiosk.is_kiosk() {
            fields.push(("locked", Value::Bool(self.kiosk.locked)));
            if let Some(auto_run) = &self.kiosk.auto_run {
                fields.push(("auto_run", Value::str(auto_run.clone())));
            }
            // **`.rulings/` R-164: `mode = "kiosk"` with no `[kiosk]` table is
            // reported, not refused.** Present only then — a configured kiosk
            // has nothing to say here, which is the same absence-means-nothing-
            // to-report convention every other optional field above follows.
            if let Some(why) = &self.kiosk.why_unconfigured {
                fields.push(("kiosk_unconfigured_because", Value::str(why.clone())));
            }
        }
        // **Absent when nobody holds the screen, and absent on a headless
        // machine**, which is the same convention `auto_run` and `pid` follow.
        // A `display_leased: null` beside a running kiosk would read as *the
        // machine says nothing is drawing* on a build that simply never
        // answered, and those want different reactions from a reader.
        //
        // **After `auto_run` and before `mode_unknown_because`**, because
        // `OUTPUTS` declares them there and a result's keys must be a
        // subsequence of it, in order.
        if let Some(master) = &self.display_master {
            fields.push(("display_master", Value::str(master.clone())));
        }
        if let Some(leased) = &self.display_leased {
            fields.push(("display_leased", Value::str(leased.clone())));
        }
        if let Some(why) = &self.kiosk.why_unknown {
            fields.push(("mode_unknown_because", Value::str(why.clone())));
        }
        fields.extend([
            (
                "services",
                Value::List(
                    self.services
                        .iter()
                        .map(|s| {
                            let mut entry = vec![
                                ("name", Value::str(s.name.clone())),
                                ("enabled", Value::Bool(s.enabled)),
                                ("state", Value::str(s.state.clone())),
                            ];
                            // Omitted rather than emitted as a zero or a null —
                            // `crate::value`'s standing convention, and here a
                            // zero would be a plausible pid.
                            if let Some(pid) = s.pid {
                                entry.push(("pid", Value::Integer(i64::from(pid))));
                            }
                            entry.push(("restarts", Value::Integer(i64::from(s.restarts))));
                            entry.push(("ready", Value::Bool(s.ready)));
                            // **Before `reason`**, because `reason` is the last
                            // thing a reader reaches and these two are part of
                            // the row's identity: what the service says about
                            // itself, and whether the operator asked anyone to
                            // care. `critical` is present always — it is a
                            // document fact with a default, not an absence.
                            entry.push(("health", Value::str(s.health.word())));
                            entry.push(("critical", Value::Bool(s.critical)));
                            // Omitted rather than null when there is none, the
                            // convention `pid` and `reason` already follow.
                            if let Some(caption) = &s.caption {
                                entry.push(("caption", Value::str(caption.clone())));
                            }
                            // **Last, and only when there is one.** A reader
                            // scanning a list of services sees the name and the
                            // state first; the sentence explaining a refusal is
                            // what they read after one of them is wrong.
                            if let Some(reason) = &s.reason {
                                entry.push(("reason", Value::str(reason.clone())));
                            }
                            Value::object(entry)
                        })
                        .collect(),
                ),
            ),
        ]);
        // ~~exactly OUTPUTS, in the declared order~~ — **a SUBSEQUENCE of
        // OUTPUTS since 2026-09-01**, because three of the fields above are now
        // conditional. It is `super::get::in_declared_order`, which is what
        // every other subject with an optional field already asserts, and it
        // still catches the two things this ever caught: a key that is not
        // published, and a published key emitted out of order.
        debug_assert!(
            super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)),
            "a status result's keys must be a subsequence of OUTPUTS, in the declared order"
        );
        Value::object(fields)
    }
}

fn boolean(line: usize, field: &'static str, text: &str) -> Result<bool, StatusError> {
    match text {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(StatusError::Value {
            line,
            field,
            found: other.to_string(),
        }),
    }
}

/// Perform `get status`. Arity is `super::perform`'s.
pub(super) fn perform(
    arguments: &[&str],
    observed: &mut dyn Observed,
) -> Result<Value, OperationError> {
    debug_assert_eq!(arguments.first().copied(), Some(SUBJECT));

    // **A second word is refused rather than ignored**, which is `super::storage`'s
    // rule and the same hazard: an operator typing `rivet get status rivet-netd`
    // expecting one service must be told that it was not read, rather than
    // handed the whole machine and left to assume it was filtered.
    if let Some(extra) = arguments.get(1).copied() {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            extra,
            format!(
                "`{get} {SUBJECT}` takes no second word and `{extra}` is one. It answers for the \
                 WHOLE machine; select in the shell — `rivet {get} {SUBJECT} | from json | get \
                 services | where name == {extra}`.",
                get = Verb::Get,
            ),
        ));
    }

    observed
        .supervisor_status()
        .map(|s| Status::value(&s))
        .map_err(refused)
}

/// The one place a [`super::MachineError`] becomes this subject's refusal — the
/// same mapping every other operation uses, and **nothing is reworded on the way
/// through**.
///
/// `ADR-032` §3.3 item 5 rules what the refusal must say, and the adapter says
/// it: *the socket, and that the supervisor is not answering.* A machine whose
/// supervisor is down is exactly when somebody asks, so this is the ordinary
/// path rather than an exceptional one.
fn refused(error: super::MachineError) -> OperationError {
    let code = match error {
        super::MachineError::Unsafe(_) => ErrorCode::Unsafe,
        super::MachineError::Refused(_) => ErrorCode::MachineRefused,
    };
    OperationError::about(code, SUBJECT, error.why())
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn sample() -> Status {
        Status {
            phase: "running".to_string(),
            // `Some` here so the round trip below carries a line with
            // something in every field; `a_supervisor_with_no_services_
            // still_reports_its_phase` covers the `None` half.
            version: Some("0.1.86".to_string()),
            // The board this suite is written against: `[kiosk] auto-run =
            // "aura-renderer"`, unlocked.
            kiosk: KioskMode::kiosk(false, Some("aura-renderer".to_string())),
            // **The launcher holds the card and has leased it out**, which is
            // that board's ordinary state. Deliberately NOT equal to `auto_run`
            // in the interesting cases below — see
            // `the_lease_is_not_the_auto_run_and_the_round_trip_keeps_them_apart`.
            display_master: Some("rivet-launcher".to_string()),
            display_leased: Some("aura-renderer".to_string()),
            // **`degraded`, and one of the two services below is why.** A
            // fixture whose machine word were `healthy` would round-trip the
            // default and say nothing about the field.
            machine: crate::health::State::Degraded,
            services: vec![
                ServiceStatus {
                    name: "rivet-netd".to_string(),
                    enabled: true,
                    state: "started".to_string(),
                    pid: Some(214),
                    restarts: 0,
                    ready: true,
                    // A running service has nothing to explain.
                    reason: None,
                    // Nothing has asked it and it speaks no health — the
                    // ordinary state of an image service (`ADR-045` §4).
                    health: crate::health::State::Unknown,
                    caption: None,
                    critical: false,
                },
                ServiceStatus {
                    name: "rivet-timed".to_string(),
                    enabled: false,
                    state: "idle".to_string(),
                    pid: None,
                    restarts: 0,
                    ready: false,
                    reason: Some("the document says this service should not run".to_string()),
                    health: crate::health::State::Degraded,
                    caption: Some("no broadcast from GVH5075_2222 in 34 minutes".to_string()),
                    critical: true,
                },
            ],
        }
    }

    /// **The round trip, which is the whole reason both halves live here.**
    /// The supervisor renders and the CLI parses; if these two ever disagree a
    /// board answers nothing and says the response was malformed.
    #[test]
    fn what_the_supervisor_writes_is_what_a_reader_reads() {
        let status = sample();
        assert_eq!(Status::parse(&status.render()), Ok(status));
    }

    /// **The lease is not the `auto_run`, and the wire must keep them apart.**
    ///
    /// This is the whole reason `display_leased` exists as a field rather than
    /// as something a reader derives. `auto_run` is what the document asks for;
    /// the lease is what the arbiter did. On the maintainer's own board they
    /// disagree routinely — `kiosk-demo` sits `enabled`, `started`, declaring
    /// `devices = ["display"]`, and its journal says `screen DENIED` every tick
    /// because `aura-renderer` holds the lease. A reader that took `auto_run`,
    /// or *started plus declares a display*, would badge both.
    #[test]
    fn the_lease_is_not_the_auto_run_and_the_round_trip_keeps_them_apart() {
        let mut disagreeing = sample();
        disagreeing.kiosk = KioskMode::kiosk(false, Some("kiosk-demo".to_string()));
        disagreeing.display_leased = Some("aura-renderer".to_string());

        let back = Status::parse(&disagreeing.render()).expect("a round trip");
        assert_eq!(
            back.kiosk.auto_run.as_deref(),
            Some("kiosk-demo"),
            "the document's answer was lost or overwritten"
        );
        assert_eq!(
            back.display_leased.as_deref(),
            Some("aura-renderer"),
            "the lease was lost, or was taken from `auto_run`"
        );
        assert_ne!(
            back.kiosk.auto_run, back.display_leased,
            "the two collapsed into one answer, which is the defect this field exists to prevent"
        );
    }

    /// **Nothing drawing is an answer, and it is not the same as no arbiter.**
    ///
    /// A launcher showing its own menu holds the card with nothing leased —
    /// exactly the state the maintainer's board is in whenever he backs out of
    /// an application. `display_master` is `Some` and `display_leased` is
    /// `None`, and the two must survive independently.
    #[test]
    fn a_launcher_at_its_menu_holds_the_card_with_nothing_leased() {
        let mut menu = sample();
        menu.display_leased = None;

        let back = Status::parse(&menu.render()).expect("a round trip");
        assert_eq!(back.display_master.as_deref(), Some("rivet-launcher"));
        assert_eq!(
            back.display_leased, None,
            "an absent lease came back as something"
        );
    }

    /// **`R33` (`.rulings/` R-163): the image's version crosses the wire and
    /// reaches the result.** Break `Status::render`, `Status::parse` or
    /// `Status::value`'s handling of `version` and this fails.
    #[test]
    fn the_version_survives_the_round_trip_and_reaches_the_result() {
        let status = sample();
        assert_eq!(status.version.as_deref(), Some("0.1.86"));

        let back = Status::parse(&status.render()).expect("a round trip");
        assert_eq!(back.version.as_deref(), Some("0.1.86"));

        assert_eq!(
            text(&status.value(), "version"),
            "0.1.86",
            "the version this machine is running must be readable straight off the result"
        );
    }

    /// **Absent, not empty.** A supervisor older than this field sends no
    /// `version` line, and the result must say nothing rather than say `""`.
    #[test]
    fn a_missing_version_is_absent_from_the_result_not_empty() {
        let mut status = sample();
        status.version = None;

        let back = Status::parse(&status.render()).expect("a round trip");
        assert_eq!(back.version, None);

        assert_eq!(
            field(&status.value(), "version"),
            None,
            "an absent version must be OMITTED, the same convention display_master follows"
        );
    }

    /// An empty machine still answers, and the phase is what makes it an answer.
    #[test]
    fn a_supervisor_with_no_services_still_reports_its_phase() {
        let empty = Status {
            phase: "finished".to_string(),
            // `None`, so this fixture also covers the wire's `-` for a line
            // this build always sends but a value it sometimes has nothing to
            // say about — see `VERSION`.
            version: None,
            kiosk: KioskMode::headless(),
            display_master: None,
            display_leased: None,
            // **`healthy` with nothing running**, which is `ADR-045` §4's
            // second stated consequence rather than an oversight: an operator
            // who has marked nothing `critical` is told that nothing they asked
            // the machine to watch is wrong.
            machine: crate::health::State::Healthy,
            services: Vec::new(),
        };
        assert_eq!(
            empty.render(),
            "phase	finished
machine	healthy
kiosk	headless	false	-	-	-
display	-	-
version	-
"
        );
        assert_eq!(Status::parse(&empty.render()), Ok(empty));
    }

    /// **A response with no phase is not a status.** A reader that accepted one
    /// would have a service list and no idea whether the supervisor was still
    /// starting them.
    #[test]
    fn a_response_without_a_phase_is_refused() {
        assert_eq!(
            Status::parse("svc\ta\ttrue\tidle\t-\t0\tfalse\t-\tunknown\tfalse\t-\n"),
            Err(StatusError::NoPhase)
        );
        assert_eq!(Status::parse(""), Err(StatusError::NoPhase));
    }

    // -- what this machine is for ------------------------------------------

    /// **The defect, in one test.** A caller is TOLD the mode; it does not have
    /// to read a document to find out.
    #[test]
    fn a_kiosk_machine_says_it_is_one_and_names_what_runs_on_it() {
        let value = sample().value();
        assert_eq!(text(&value, "mode"), MODE_KIOSK);
        assert!(!flag(&value, "locked"), "this fixture is an UNLOCKED kiosk");
        assert_eq!(text(&value, "auto_run"), "aura-renderer");
        assert!(
            field(&value, "mode_unknown_because").is_none(),
            "a known mode must carry no excuse"
        );
    }

    /// `locked` is BESIDE the mode and not a third word for it, so a caller
    /// asking *is this a kiosk* reads one field and never a prefix.
    #[test]
    fn locking_a_kiosk_does_not_change_the_word_the_mode_is() {
        let mut locked = sample();
        locked.kiosk = KioskMode::kiosk(true, Some("aura-renderer".to_string()));
        assert_eq!(text(&locked.value(), "mode"), MODE_KIOSK);
        assert!(flag(&locked.value(), "locked"));
        assert!(locked.kiosk.is_kiosk());
    }

    /// A machine locked to an application it was never told: `locked` with no
    /// `auto_run`. Both facts are there and neither is invented.
    #[test]
    fn a_kiosk_locked_with_nothing_to_run_reports_both_halves_of_that() {
        let mut stranded = sample();
        stranded.kiosk = KioskMode::kiosk(true, None);
        let value = stranded.value();
        assert_eq!(text(&value, "mode"), MODE_KIOSK);
        assert!(flag(&value, "locked"));
        assert!(
            field(&value, "auto_run").is_none(),
            "an absent auto-run must be absent, not an empty string"
        );
    }

    /// **`locked` is meaningless off a kiosk and is therefore absent**, rather
    /// than a `false` that invites *this is a kiosk somebody left unlocked*.
    #[test]
    fn a_headless_machine_carries_no_lock_and_no_application() {
        let mut headless = sample();
        headless.kiosk = KioskMode::headless();
        let value = headless.value();
        assert_eq!(text(&value, "mode"), MODE_HEADLESS);
        assert!(field(&value, "locked").is_none());
        assert!(field(&value, "auto_run").is_none());
        assert!(field(&value, "mode_unknown_because").is_none());
    }

    /// **The whole reason the answer is three-valued.** A machine whose
    /// document could not be read must not be reported as headless — that was
    /// the old string match's answer, given confidently about a document nobody
    /// had parsed.
    #[test]
    fn a_machine_that_could_not_read_its_document_says_unknown_and_why() {
        let mut blind = sample();
        blind.kiosk = KioskMode::unknown("/state/config/machine.toml is not valid TOML at line 4");
        let value = blind.value();
        assert_eq!(text(&value, "mode"), MODE_UNKNOWN);
        assert!(text(&value, "mode_unknown_because").contains("line 4"));
        assert!(!blind.kiosk.is_kiosk());
        assert!(field(&value, "locked").is_none());
    }

    /// The mode survives the wire, in every one of its shapes.
    #[test]
    fn every_mode_round_trips_through_the_wire_format() {
        for kiosk in [
            KioskMode::headless(),
            KioskMode::kiosk(false, None),
            KioskMode::kiosk(true, Some("aura-renderer".to_string())),
            KioskMode::unknown("a reason with\ttabs and\nnewlines in it"),
            KioskMode::kiosk_unconfigured(),
        ] {
            let mut status = sample();
            status.kiosk = kiosk;
            let rendered = status.render();
            let back = Status::parse(&rendered).expect("a status");
            assert_eq!(back.kiosk.mode, status.kiosk.mode);
            assert_eq!(back.kiosk.locked, status.kiosk.locked);
            assert_eq!(back.kiosk.auto_run, status.kiosk.auto_run);
            // The framing characters do not survive and the words do — the
            // rule `reason` follows, applied to the two other fields here that
            // carry prose.
            assert_eq!(
                back.kiosk.why_unknown.is_some(),
                status.kiosk.why_unknown.is_some()
            );
            assert_eq!(
                back.kiosk.why_unconfigured.is_some(),
                status.kiosk.why_unconfigured.is_some()
            );
            assert!(!rendered.lines().any(|l| l.is_empty()));
        }
    }

    /// **`.rulings/` R-164: `mode = "kiosk"` with no `[kiosk]` table is
    /// tolerated and reported, not refused.** `mode` still says `kiosk` — that
    /// is the fact worth surfacing — while `locked` and `auto_run` carry the
    /// same values an empty table always has, and the new field names why.
    #[test]
    fn a_kiosk_asked_for_and_not_configured_says_so_and_is_not_refused() {
        let mut unconfigured = sample();
        unconfigured.kiosk = KioskMode::kiosk_unconfigured();
        let value = unconfigured.value();
        assert_eq!(text(&value, "mode"), MODE_KIOSK, "asked for is still reported");
        assert!(!flag(&value, "locked"));
        assert!(field(&value, "auto_run").is_none());
        assert!(
            text(&value, "kiosk_unconfigured_because").contains("no [kiosk] table"),
            "{value:?}"
        );
        assert!(
            field(&value, "mode_unknown_because").is_none(),
            "unconfigured is not the same fact as unknown"
        );
        assert!(unconfigured.kiosk.is_kiosk());
    }

    /// **The version-skew case, and it must not answer `headless`.** A
    /// supervisor from before 2026-09-01 sends no `kiosk` line at all, and a
    /// reader that filled the gap with a default would report a running kiosk
    /// as a headless box on the strength of nothing.
    #[test]
    fn a_status_with_no_kiosk_line_is_unknown_and_never_headless() {
        let old = "phase\tconverging\nsvc\ta\ttrue\tidle\t-\t0\tfalse\t-\tunknown\tfalse\t-\n";
        let parsed = Status::parse(old).expect("an older supervisor's answer still parses");
        assert_eq!(parsed.kiosk.mode, MODE_UNKNOWN);
        assert!(!parsed.kiosk.is_kiosk());
        assert!(
            parsed
                .kiosk
                .why_unknown
                .as_deref()
                .is_some_and(|why| why.contains("older than the CLI")),
            "{:?}",
            parsed.kiosk.why_unknown
        );
    }

    #[test]
    fn a_kiosk_line_of_the_wrong_width_is_refused_rather_than_padded() {
        let short = "phase\tconverging\nkiosk\tkiosk\ttrue\n";
        assert_eq!(
            Status::parse(short),
            Err(StatusError::Fields {
                line: 2,
                kind: "kiosk".to_string(),
                found: 3,
                wanted: 6,
            })
        );
    }

    #[test]
    fn a_lock_that_is_not_a_boolean_is_refused() {
        let bad = "phase\tconverging\nkiosk\tkiosk\tyes\t-\t-\t-\n";
        assert!(matches!(
            Status::parse(bad),
            Err(StatusError::Value { field: "locked", .. })
        ));
    }

    /// **The mode and the screen come before the services.** A machine with a
    /// hundred of them still says what it is for, and what is drawing, in what
    /// a truncated read keeps.
    #[test]
    fn the_mode_is_rendered_before_the_services() {
        let rendered = sample().render();
        let lines: Vec<&str> = rendered.lines().collect();
        assert!(lines[0].starts_with("phase\t"), "{rendered}");
        assert!(lines[1].starts_with("machine\t"), "{rendered}");
        assert!(lines[2].starts_with("kiosk\t"), "{rendered}");
        assert!(lines[3].starts_with("display\t"), "{rendered}");
        assert!(lines[4].starts_with("version\t"), "{rendered}");
        assert!(lines[5].starts_with("svc\t"), "{rendered}");
    }

    // Accessors for the assertions above. `Value` has no getters of its own;
    // every subject's tests grow the two or three they need.
    fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
        match value {
            Value::Object(fields) => fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn text(value: &Value, key: &str) -> String {
        match field(value, key) {
            Some(Value::Str(s)) => s.clone(),
            other => panic!("{key} is {other:?} and not a string"),
        }
    }

    fn flag(value: &Value, key: &str) -> bool {
        match field(value, key) {
            Some(Value::Bool(b)) => *b,
            other => panic!("{key} is {other:?} and not a boolean"),
        }
    }

    /// **An unknown line kind is refused, never skipped.** This is the version
    /// skew case: a CLI older than the supervisor it is talking to must say so
    /// rather than report a status missing whatever it did not understand.
    #[test]
    fn a_line_this_build_does_not_know_is_refused_rather_than_skipped() {
        let newer = "phase\tconverging\nvolume\t/state\ttrue\n";
        let refused = Status::parse(newer).expect_err("an unknown line must refuse");
        assert_eq!(
            refused,
            StatusError::UnknownLine {
                line: 2,
                kind: "volume".to_string()
            }
        );
        assert!(
            refused.to_string().contains("newer than this CLI"),
            "{refused}"
        );
    }

    /// A malformed field says which line and which field, because the operator's
    /// next move is to look at the supervisor rather than to retry.
    #[test]
    fn a_malformed_field_names_the_line_and_the_field() {
        let bad = "phase\tconverging\nsvc\ta\tyes\tidle\t-\t0\tfalse\t-\tunknown\tfalse\t-\n";
        assert_eq!(
            Status::parse(bad),
            Err(StatusError::Value {
                line: 2,
                field: "enabled",
                found: "yes".to_string()
            })
        );
        let short = "phase\tconverging\nsvc\ta\ttrue\n";
        assert_eq!(
            Status::parse(short),
            Err(StatusError::Fields {
                line: 2,
                kind: "svc".to_string(),
                found: 3,
                wanted: 11
            })
        );
    }

    /// **A health word outside `ADR-045` §4's closed set is refused, never
    /// defaulted.** A reader that substituted `unknown` would report *nobody
    /// said* about a machine that did say — which is `ADR-006` §7's *trusted and
    /// wrong* in the field an operator acts on.
    #[test]
    fn a_health_word_this_build_does_not_know_is_refused_rather_than_defaulted() {
        let machine = "phase\tconverging\nmachine\twobbly\n";
        assert_eq!(
            Status::parse(machine),
            Err(StatusError::Value {
                line: 2,
                field: "machine",
                found: "wobbly".to_string()
            })
        );
        let service = "phase\tconverging\nsvc\ta\ttrue\tidle\t-\t0\tfalse\t-\twobbly\tfalse\t-\n";
        assert_eq!(
            Status::parse(service),
            Err(StatusError::Value {
                line: 2,
                field: "health",
                found: "wobbly".to_string()
            })
        );
    }

    /// **A supervisor that sends no `machine` line is `unknown`, not
    /// `healthy`.** The only way to reach it is an A/B slot older than the CLI
    /// asking; answering `healthy` on its behalf is the failure this default
    /// exists to prevent.
    #[test]
    fn a_status_with_no_machine_line_is_unknown_and_never_healthy() {
        let old = Status::parse("phase\tconverging\n").expect("a status");
        assert_eq!(old.machine, crate::health::State::Unknown);
        // An older supervisor's word for `running` is carried as it said it,
        // never rewritten on the way through (`ADR-032` §8).
        assert_eq!(old.phase, "converging");
    }

    /// **The caption survives the wire with its state**, which is `ADR-045`
    /// §5's whole claim: `degraded` on its own tells a reader only that they
    /// must go and look.
    #[test]
    fn a_caption_travels_with_the_state_it_describes() {
        let back = Status::parse(&sample().render()).expect("a status");
        assert_eq!(back.services[1].health, crate::health::State::Degraded);
        assert_eq!(
            back.services[1].caption.as_deref(),
            Some("no broadcast from GVH5075_2222 in 34 minutes")
        );
        assert!(back.services[1].critical);
        // And a service with no caption comes back with none, rather than with
        // an empty sentence a reader has to interpret.
        assert_eq!(back.services[0].caption, None);
        assert!(!back.services[0].critical);
    }

    /// An absent pid is **absent**, not a zero — and zero is a plausible pid,
    /// which is why this is asserted rather than assumed.
    #[test]
    fn a_service_that_is_not_running_omits_its_pid_rather_than_reporting_zero() {
        let Value::Object(fields) = sample().value() else {
            panic!("a status is an object");
        };
        let Some((_, Value::List(services))) = fields.iter().find(|(k, _)| *k == "services") else {
            panic!("services is a list");
        };
        let Value::Object(idle) = &services[1] else {
            panic!("a service is an object");
        };
        assert!(
            !idle.iter().any(|(k, _)| *k == "pid"),
            "an idle service published a pid: {idle:?}"
        );
        let Value::Object(running) = &services[0] else {
            panic!("a service is an object");
        };
        assert!(running.iter().any(|(k, v)| *k == "pid" && *v == Value::Integer(214)));
    }

    /// The socket's mode is `ADR-032` §3.3 item 2's ruling and the path is §3.2's;
    /// both are the interface contract.
    #[test]
    fn the_socket_path_is_the_one_the_adr_names() {
        assert_eq!(QUERY_SOCKET_PATH, "/run/rivet/query");
        assert_ne!(
            QUERY_SOCKET_PATH,
            crate::journal::INGEST_SOCKET_PATH,
            "ADR-032 §3.4: widening the ingest socket is the trap, not the shortcut"
        );
    }
}

#[cfg(test)]
mod reason_tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    fn one(reason: Option<&str>) -> Status {
        Status {
            phase: "running".to_string(),
            version: None,
            kiosk: KioskMode::headless(),
            // A headless machine: nothing holds a screen it does not have.
            display_master: None,
            display_leased: None,
            machine: crate::health::State::Healthy,
            services: vec![ServiceStatus {
                name: "mesa-demo".to_string(),
                enabled: true,
                state: "idle".to_string(),
                pid: None,
                restarts: 0,
                ready: false,
                reason: reason.map(str::to_string),
                health: crate::health::State::Unknown,
                caption: None,
                critical: false,
            }],
        }
    }

    /// **The whole point** — `PLAN.md` §104.22. A refusal survives the wire, so
    /// `get status` can say why a service is not running instead of leaving the
    /// answer in `get log … all`.
    #[test]
    fn a_reason_survives_the_round_trip() {
        let said = "REFUSED another service already declares this device class";
        let back = Status::parse(&one(Some(said)).render()).expect("a status");
        assert_eq!(back.services[0].reason.as_deref(), Some(said));
    }

    /// Absent is a different claim from empty, and it travels as one.
    #[test]
    fn no_reason_comes_back_as_no_reason() {
        let back = Status::parse(&one(None).render()).expect("a status");
        assert_eq!(back.services[0].reason, None);
    }

    /// **A sentence cannot break the frame carrying it.** The wire is
    /// tab-separated and newline-terminated, and a reason is prose — so the two
    /// characters that would split one record into two are replaced rather than
    /// refused.
    #[test]
    fn tabs_and_newlines_in_a_reason_cannot_corrupt_the_record() {
        let nasty = "first\tsecond\nthird\rfourth";
        let rendered = one(Some(nasty)).render();
        assert_eq!(
            rendered.lines().count(),
            6,
            "a phase, a machine, a kiosk, a display, a version and one service line: {rendered:?}"
        );
        let back = Status::parse(&rendered).expect("a status");
        assert_eq!(
            back.services[0].reason.as_deref(),
            Some("first second third fourth"),
            "the words survive; only the framing characters do not"
        );
    }

    /// A reason that is literally `-` would be read back as absent, and that is
    /// a collision worth knowing about rather than discovering. It is accepted:
    /// no refusal this supervisor writes is one character long.
    #[test]
    fn a_lone_dash_reads_back_as_absent_and_that_is_documented() {
        let back = Status::parse(&one(Some("-")).render()).expect("a status");
        assert_eq!(back.services[0].reason, None);
    }
}
