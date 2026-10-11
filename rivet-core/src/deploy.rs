//! `rivet-deploy`, as a name, a path and an outcome — never as a running
//! install.
//!
//! The maintainer's 2026-08-24 ruling made the installer its own supervised binary and
//! `PLAN.md` §0.4.6 gave it its name. That binary is `cmd/rivet-deploy`; what
//! this module holds is the handful of facts **four crates have to agree
//! about**, none of which any of them can own:
//!
//! | Fact | Constant | Read by |
//! | :-- | :-- | :-- |
//! | what the service is called | [`SERVICE_NAME`] | `cmd/rivet-deploy` (its `svc` column), `cmd/rivet-sysd` (the definition, the cgroup), `rivet-core`'s `get install` |
//! | where the binary lives | [`PROGRAM_PATH`] | `cmd/rivet-image` (installs it), `cmd/rivet-sysd` (starts it) |
//! | what `cmd/rivet-deploy` labels its own records with, before the supervisor overwrites the label on capture | [`ACTOR`] | `cmd/rivet-deploy` |
//! | **how an install's last record says how it ended** | [`OUTCOME_FIELD`], [`Outcome`] | `cmd/rivet-deploy` writes it, `rivet-core`'s `get install` reads it |
//!
//! **The shape is `ADR-011`'s, and it follows from a fact about binaries.**
//! Three of the four consumers are **binary** crates, which have no library
//! target and therefore cannot `use` a constant out of one another; this
//! module is the standing record of what happens when a path is spelled in
//! two of them instead. Every one of those four crates already depends on
//! `rivet-core`.
//!
//! Nothing here performs I/O, and [`crate::state`] performs none either, for
//! the same reason (`ADR-014` §3.1): these are decisions expressed so a program
//! can act on them and a test can check them without a card, without root and
//! without a kernel.
//!
//! # Why a record needs a token as well as a sentence
//!
//! `ADR-006` §4.1, applied to a journal record rather than to a refusal:
//!
//! > Every failure carries a stable machine-readable code, a structured
//! > payload, and a human string — in that order of authority. **An agent
//! > branches on the code; a human reads the string.**
//!
//! `cmd/rivet-deploy` already emits exactly one record at the end of an
//! install saying, in English, which of five ways it ended. `rivet get
//! install` has to answer *did it finish, and how* — and matching on that
//! English is the failure §4.1 exists to prevent, in the form `AGENTS.md` §7 is
//! about: the prose is edited, the reader keeps compiling, and the answer
//! silently becomes *"still running"* for every install that ever succeeded.
//!
//! So the terminal record carries [`OUTCOME_FIELD`] beside its message. **It is
//! not a new record and it is not a progress protocol** — no record is added,
//! none is removed, and nothing here says anything about the *middle* of an
//! install. It is a name for what the last line already said.
//!
//! # The request, added 2026-08-24 — the maintainer's 2026-08-24 ruling
//!
//! **Ruled 2026-08-24: option A.** The channel is a file. `cmd/rivet` writes
//! [`REQUEST_PATH`], naming the payload; `cmd/rivet-sysd` looks for one
//! on every convergence pass, and if there is one it **resolves what it names in
//! this machine's artifact store**, builds its `rivet-deploy` definition around
//! the resolved path, **removes the request**, and converges — which starts it
//! once. `ADR-017` §3.2.2 is the amendment that makes a service instance created
//! at run time a thing this design has.
//!
//! **The thing it names changed on 2026-09-12** — `ADR-052` §7. It was an
//! absolute path and it is now a BLAKE3 digest, resolved in
//! [`crate::artifact`]'s store. See [`Request`] for what that changes and, more
//! importantly, what it does not.
//!
//! So a fifth fact joins the table above, and it is the widest of them: the
//! **path, the mode, the size cap and the one line inside** are agreed between
//! two binaries that never link against one another, and a disagreement about
//! any of the four is an install that silently never happens.
//!
//! | Fact | Constant | Written by | Read by |
//! | :-- | :-- | :-- | :-- |
//! | where a request lives | [`REQUEST_PATH`] | `cmd/rivet` | `cmd/rivet-sysd` |
//! | how it is written without a torn read | [`REQUEST_TEMP_PATH`] | `cmd/rivet` | nothing — it is renamed over |
//! | what it may contain | [`render_request`], [`parse_request`] | `cmd/rivet` | `cmd/rivet-sysd` |
//! | how much of it will ever be read | [`REQUEST_MAX_BYTES`] | — | `cmd/rivet-sysd`, **before** it reads |
//! | **who asked** | [`Principal`] | `cmd/rivet`, from its environment | `cmd/rivet-sysd`, into a record |
//!
//! # The request says who asked — added 2026-08-24, `ADR-028` §4.5
//!
//! The channel as first built carried **one absolute path and nothing else**, so
//! the only answer this machine had to *"who asked for that install"* was
//! *"somebody who was root"*. `ADR-028` §4 wants an authenticated principal;
//! [`ACTOR`] documented the gap for a day, and the maintainer ruled it closed.
//!
//! ```text
//! from=pinned-3f7c1a9e04b2d85f6a3c9017e2b48d5f1c6a09b3e7d248fa5601c93b8ed72a4f -- 8c4e9b21f0a37d5e6b18c92470fd3a85e1c7b0946d2af38e5710cb639ad84f2c
//! ```
//!
//! **Read [`Principal`]'s own documentation before drawing any conclusion from
//! that line.** The one-sentence version, which every other place this is
//! written down repeats: it is **attribution and not authorisation**, the trust
//! boundary is still *root on `/state`*, and anything running as uid 0 can write
//! the file itself and claim whatever it likes.
//!
//! What is genuinely new is that the claim is **checkable**.
//! [`Origin::ControlPlane`] carries the Ed25519 public key the caller was pinned
//! by (`ADR-025` §2), and that is the same key the control plane's own records
//! name for the connection it arrived on (`ADR-022`). So a reader gets from
//! *this install* to *that caller* by comparing two strings, and an origin
//! naming a key that never connected is contradicted by the same journal it was
//! written into. That is a chain of custody assembled entirely out of things
//! the machine observed, and it required inventing no cryptography.
//!
//! **What the CLI cannot know, and therefore does not say.** Its own uid is root
//! on every invocation, so it distinguishes nothing on its own. A process whose
//! environment names no caller at all says exactly that ([`NO_SESSION_TOKEN`])
//! rather than claiming a console it cannot see.
//!
//! **The parser is here rather than in the supervisor because the writer is not
//! the reader.** `AGENTS.md` §4: a reader round-tripped against its own writer
//! proves the two agree with each other and nothing about the format. One
//! function that both sides call is the stronger version of that — there is no
//! second implementation to disagree — and the tests below check literal bytes
//! as well as the round trip, because the file outlives both processes.
//!
//! # It is attacker-reachable state, and the parser is written that way
//!
//! `/state` is writable and survives reboots, so the request is the first thing
//! in this system that is **both** persistent and able to make the supervisor
//! execute something. Three properties follow, and each is a test below rather
//! than an intention:
//!
//! 1. **A size cap that is checked before the read.** [`REQUEST_MAX_BYTES`] is
//!    one digest plus a bounded principal; the supervisor asks for the
//!    file's size first and refuses a larger one without reading it. A
//!    `read_file` on a file somebody grew to fill the card is an allocation
//!    inside PID 1's only child.
//! 2. **Refuse rather than guess** (`AGENTS.md` §5). Anything that is not one
//!    principal and one digest on one line is a [`RequestProblem`], not a
//!    best effort at what was probably meant — **including a request that names
//!    no principal at all**, which is the pre-2026-08-24 format and is refused
//!    rather than installed unattributed. `ADR-028` §4.5 argues that one, and
//!    [`RequestProblem::NoPrincipal`] carries the short form.
//! 3. **The digest is not the payload.** A request says *which of the artifacts
//!    this machine already holds to install*, and everything about the artifact
//!    itself — signature, digests, product, architecture — is still refused by
//!    `cmd/rivet-deploy` under `ADR-029` §11's single-verifier rule. Writing a
//!    request buys an attacker the ability to name something that is already
//!    here; it does not buy them the ability to install it.
//!
//!    **This got narrower on 2026-09-12 and it is worth saying how much.** While
//!    the field was a path, writing a request bought the ability to name *any
//!    file on `/state`* — including one an attacker had just put there. Now the
//!    only nameable things are objects under `/state/artifacts/objects`, which
//!    is a directory nothing writes but the transfer that hashed the bytes. The
//!    boundary is still root on `/state` and an attacker who has it can write
//!    that directory too; what is gone is the shape where naming and placing
//!    were the same act performed with different tools.
//!
//! What it does **not** defend against is somebody who can already write
//! `/state` as uid 0, and nothing at this layer could: that is a machine whose
//! `/state/app` they can also write (`ADR-015` §7). The boundary being held is
//! the *unprivileged* one — `cmd/rivet-image` creates `/state` with
//! `root_owner=0:0`, so an application's own uid (`ADR-015` §5.1) cannot create
//! a file in it at all.
//!
//! # What this module deliberately does NOT hold
//!
//! **Anything about progress.** An install emits records as it goes and
//! `ADR-022` §5's correlation identifier ties them into one chain; that is the
//! whole of the mechanism and it needs no vocabulary of its own. A `stage`
//! field with a closed set of values *would* be a protocol, so there is not
//! one.

use crate::artifact::Digest;

/// `ADR-017` §2.1's name for the installer service.
///
/// Also its cgroup (`ADR-017` §3.1.3), its notify socket (`ADR-017` §3.4) and
/// the `svc` column of every record it writes (`ADR-022` §3.2) — which is what
/// makes `rivet get install` able to find them among everything else on the
/// machine.
///
/// It satisfies [`crate::service::ServiceName`]'s rule, and a test below pins
/// that rather than a comment: a name a cgroup could not hold would make the
/// definition unstartable on a board and correct in every unit test that
/// supplied its own.
pub const SERVICE_NAME: &str = "rivet-deploy";

/// Where `cmd/rivet-image` installs the binary and `cmd/rivet-sysd`
/// starts it from.
///
/// `/system/bin` for `ADR-008` §3's reason rather than the language's: `/system`
/// is read-only vendor content replaced wholesale by an update, and the
/// installer has exactly that lifetime. It is **not** on `/state` — an installer
/// living on the partition it writes into would be a machine that cannot repair
/// itself after the write it was performing went wrong.
pub const PROGRAM_PATH: &str = "/system/bin/rivet-deploy";

/// What `cmd/rivet-deploy` labels its own records with — **not** what reaches
/// the journal file.
///
/// The supervisor captures this process's stdout and stderr and, before
/// committing a line to disk, stamps it with
/// [`crate::journal::SUPERVISOR_ACTOR`] instead — the same value it stamps on
/// its own notes, and the one an install's chain reader actually filters on.
/// This constant never reaches a persisted record; it is overwritten on
/// capture every time.
///
/// **`update`, and not the operator who asked** — the gap is named rather than
/// papered over. `ADR-028` §4 rules that *authority* comes from an
/// authenticated principal and `ADR-012` §4 requires actor provenance from the
/// start; nothing carries a principal into that process. So the honest label
/// is the process acting on the instruction it was given, and a record naming
/// a person would be inventing one.
///
/// **The request itself does carry one.** [`Principal`] is what closes that half
/// of the gap and `ADR-028` §4.5 is where the reasoning lives. **Read the next
/// paragraph before concluding anything from it.**
///
/// **This constant does not change, and the reason is the honest half.** A
/// principal reaches the *request*, and from there the supervisor's own record
/// of taking it up. It does **not** reach `cmd/rivet-deploy`: that process is
/// handed one argument, the payload path (`cmd/rivet-sysd`'s
/// `definitions::deploy`), so every record *inside* an install is still
/// labelled, before capture, by a process that was told what to do and not who
/// by. Changing `ACTOR` to a person here would make every one of those labels
/// claim a provenance the process performing them does not have — even though
/// the label itself never survives to be read back.
///
/// It stays `update` rather than becoming `deploy` with the binary's rename:
/// nothing reads this constant back, so renaming it would be churn with no
/// reader to notice either spelling.
pub const ACTOR: &str = "update";

/// The field an install's **last** record carries, naming how it ended.
///
/// See this module's documentation for why a token exists beside the sentence.
/// A record without it is a record from the *middle* of an install, which is
/// the whole of how [`Outcome::from_record_field`]'s caller tells a finished
/// chain from an unfinished one.
pub const OUTCOME_FIELD: &str = "outcome";

/// How an install ended, as its last record says.
///
/// **Five, and they are `cmd/rivet-deploy`'s five** — the arms of the `match`
/// that chooses its exit code. This enum does not invent a taxonomy; it names
/// one that already exists and was previously legible only as English.
///
/// The distinctions are the ones an operator acts on, which is `ADR-006` §4.1's
/// test for whether two things deserve separate codes:
///
/// * a **plan** changed nothing and was asked not to;
/// * a **refused payload** is a fact about the payload, and nothing was
///   written, so retrying with the same file cannot help;
/// * a **refused machine** is a fact about the card or the kernel, may have
///   written something, and might work on the next attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// A system update was written into the inactive slot and the next boot is
    /// armed as a trial of it. `ADR-029` §9.2 steps 1–10.
    ///
    /// **The machine has not rebooted and nothing here will reboot it**
    /// (`ADR-029` §9.1).
    Installed,
    /// An application was unpacked, verified and made current. `ADR-015` §7.
    ///
    /// **Nothing started it**: the supervisor does, on its next convergence,
    /// and only if the document enables it.
    InstalledApplication,
    /// A dry run of a system update. Everything was verified and planned;
    /// **nothing was written** (`ADR-006` §6.2).
    Planned,
    /// A dry run of an application install, on [`Self::Planned`]'s terms.
    PlannedApplication,
    /// **Shared Rivets, 2026-08-31.** A library artifact was unpacked and
    /// verified into its content-addressed directory.
    ///
    /// Nothing runs a library and nothing enables one: it sits there until an
    /// application that declares it is started, which is the whole of its
    /// lifecycle.
    InstalledLibrary,
    /// A library whose directory was already on the machine.
    ///
    /// **A success and not a refusal**, which is the one place a library
    /// deliberately differs from an application. A library's directory name
    /// carries a digest of its own bytes, so a directory that is there holds
    /// the bytes being installed and there is nothing to write. Two
    /// applications declaring one library install it once.
    LibraryAlreadyInstalled,
    /// A dry run of a library install, on [`Self::Planned`]'s terms.
    PlannedLibrary,
    /// The payload is not one this machine will install, and **nothing was
    /// written**. A bad signature, a member whose digest does not match, a
    /// product or an architecture that is not this machine's.
    RefusedPayload,
    /// The machine, the card or the kernel refused. Something may have been
    /// written — the record's own `wrote` field is what says.
    RefusedMachine,
}

impl Outcome {
    /// Every outcome, in the order `cmd/rivet-deploy`'s `match` reaches them.
    /// Written out rather than counted, because it is a contract between a
    /// writer and a reader in two different binaries.
    pub const ALL: &'static [Outcome] = &[
        Outcome::Installed,
        Outcome::InstalledApplication,
        Outcome::Planned,
        Outcome::PlannedApplication,
        Outcome::InstalledLibrary,
        Outcome::LibraryAlreadyInstalled,
        Outcome::PlannedLibrary,
        Outcome::RefusedPayload,
        Outcome::RefusedMachine,
    ];

    /// The wire form — what the record carries and what a pipeline matches on.
    ///
    /// **Part of the interface contract, and never renamed.** A renamed token
    /// makes every install written by an older build unreadable to a newer
    /// `get install`, on a machine whose journal is the only account of what
    /// happened (`ADR-022` §8).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::InstalledApplication => "installed-application",
            Self::Planned => "planned",
            Self::PlannedApplication => "planned-application",
            Self::InstalledLibrary => "installed-library",
            Self::LibraryAlreadyInstalled => "library-already-installed",
            Self::PlannedLibrary => "planned-library",
            Self::RefusedPayload => "refused-payload",
            Self::RefusedMachine => "refused-machine",
        }
    }

    /// Read an outcome from a record's [`OUTCOME_FIELD`].
    ///
    /// **Exact, and without aliases**, for `Verb::parse`'s reason. A token this
    /// build does not know is `None`, which a reader must report as *"this
    /// install ended in a way this build cannot name"* rather than as *"it did
    /// not end"* — a newer Rivet writing a sixth outcome is the ordinary way
    /// that happens, and the two answers send an operator in opposite
    /// directions.
    pub fn from_record_field(token: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|o| o.as_str() == token)
    }

    /// Whether anything reached the card.
    ///
    /// **A plan and a refused payload wrote nothing** and that is a promise
    /// `ADR-029` §5's ordering makes structural: every member is verified
    /// before the first byte is written, and `cmd/rivet-deploy`'s end-to-end
    /// run watched a flipped bit leave the slot byte-identical.
    /// [`Self::RefusedMachine`] is the one that cannot promise it, because the
    /// refusal may have come from the card halfway through the write.
    pub const fn wrote_nothing(self) -> Option<bool> {
        match self {
            Self::Planned
            | Self::PlannedApplication
            | Self::PlannedLibrary
            | Self::RefusedPayload => Some(true),
            // `LibraryAlreadyInstalled` wrote nothing AND is a success, which
            // no other outcome is. Both halves are the honest answer: the files
            // are there, and this run did not put them there.
            Self::LibraryAlreadyInstalled => Some(true),
            Self::Installed | Self::InstalledApplication | Self::InstalledLibrary => Some(false),
            // Unknown rather than `false`: the record's own `wrote` field is
            // the authority, and guessing here would be this module inventing
            // an answer the process that was there already gave.
            Self::RefusedMachine => None,
        }
    }

    /// Whether the install did what it was asked to do.
    ///
    /// # It existed and the ANSWER did not carry it — corrected 2026-08-30
    ///
    /// **`finished` says the chain stopped. This says how it stopped.** A
    /// refused payload finishes — the installer read it, refused it, and wrote
    /// its last record — so a caller polling `finished` alone reads a refusal
    /// as a success.
    ///
    /// **That has happened twice.** `PLAN.md` §73.2 recorded it costing a
    /// deploy cycle, and on 2026-08-30 it caught the session that had read
    /// §73.2 and written the polling loop anyway. The predicate was here the
    /// whole time; `get install`'s answer simply never published it, so the only
    /// defence was a warning in prose. A hazard being written down is not the
    /// same as it being closed.
    ///
    /// **A plan counts as success here** and that is deliberate: this asks
    /// whether the operation did what it was *asked* to do, and a plan was
    /// asked for. A caller who confirmed an install never sees `planned`.
    pub const fn succeeded(self) -> bool {
        matches!(
            self,
            Self::Installed
                | Self::InstalledApplication
                | Self::InstalledLibrary
                | Self::LibraryAlreadyInstalled
                | Self::Planned
                | Self::PlannedApplication
                | Self::PlannedLibrary
        )
    }

    /// What it means, in a sentence an operator reads.
    ///
    /// **The six must not read alike.** An operator who reads `planned` as
    /// `installed` reboots a board expecting a new system and gets the one it
    /// already had, which looks exactly like an update that did nothing.
    pub const fn means(self) -> &'static str {
        match self {
            Self::Installed => {
                "a system update was written into the INACTIVE slot and the next boot is armed \
                 as a one-shot trial of it. The machine has NOT rebooted — ADR-029 §9.1 keeps \
                 the reboot separate, so an armed machine is inert until `rivet apply \
                 trial-restart confirm`"
            }
            Self::InstalledApplication => {
                "an application was unpacked, verified and made current under /state/app. \
                 NOTHING STARTED IT: the supervisor does that on its next convergence, and \
                 only if the desired-state document enables it (ADR-015 §3.2.1)"
            }
            Self::Planned => {
                "a dry run of a system update. It was verified and planned in full and NOTHING \
                 WAS WRITTEN (ADR-006 §6.2), so the machine is exactly as it was"
            }
            Self::PlannedApplication => {
                "a dry run of an application install. It was verified and planned in full and \
                 NOTHING WAS WRITTEN, so the machine is exactly as it was"
            }
            Self::InstalledLibrary => {
                "a library was unpacked and verified into its own content-addressed directory \
                 under /state/lib. NOTHING RUNS A LIBRARY: it is there for the applications \
                 that declare it, and an application declaring it can now be installed"
            }
            Self::LibraryAlreadyInstalled => {
                "this library's directory was already on the machine, so nothing was written \
                 and nothing needed to be. A library directory is named by a digest of its own \
                 bytes, so a directory that exists holds exactly these bytes — which is why two \
                 applications needing one library install it once"
            }
            Self::PlannedLibrary => {
                "a dry run of a library install. It was verified and planned in full and \
                 NOTHING WAS WRITTEN, so the machine is exactly as it was"
            }
            Self::RefusedPayload => {
                "the payload is not one this machine will install and NOTHING WAS WRITTEN. This \
                 is a fact about the file rather than about the machine, so the same file will \
                 be refused again; the record says which check refused it"
            }
            Self::RefusedMachine => {
                "the machine, the card or the kernel refused. Whether anything was written is \
                 the record's own `wrote` field — a refusal can arrive part-way through a \
                 640 MiB write, which is the one case where an inactive slot is left neither \
                 old nor new. The inactive slot is never the running one, so the machine still \
                 boots what it booted"
            }
        }
    }
}

impl core::fmt::Display for Outcome {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// The request — the maintainer's 2026-08-24 ruling
// ---------------------------------------------------------------------------

/// The file `cmd/rivet` writes and `cmd/rivet-sysd` consumes.
///
/// **A bare file in `/state`, on `/state/clock`'s precedent** rather than a
/// directory of its own. There is at most one request at a time — an install
/// writes into *the* inactive slot and there is only one — so a queue would be a
/// mechanism for a case that cannot arise, and a directory would need creating,
/// which is a second failure mode on the path of a command whose whole promise
/// is that it hands over and returns.
///
/// **On `/state` rather than `/run`**, and that is the half worth arguing.
/// `/run` is a tmpfs, so a request written there would be lost by the reboot
/// `ADR-029` §9.1 makes a separate operation — and the failure would be silent,
/// because nothing would remain to say an install had ever been asked for.
/// `/state` survives, which means a request outlives a supervisor crash and a
/// power cut. That is the property the ruling named, and it is also the reason
/// the consuming side removes the request **before** it starts anything.
pub const REQUEST_PATH: &str = "/state/deploy-request";

/// Where a request is written before it is renamed onto [`REQUEST_PATH`].
///
/// **A rename, because the reader is a different process and is not waiting for
/// this one.** The supervisor may look at exactly the moment the write is half
/// done; `rename(2)` within one directory is atomic, so it sees either no
/// request or a whole one, and never a truncated path that names a file nobody
/// asked for.
///
/// It is deliberately **not** a prefix of [`REQUEST_PATH`] in the direction that
/// matters: the reader opens one exact path and nothing globs, so a leftover
/// temporary file from a CLI that died mid-write is inert rather than a second
/// request.
pub const REQUEST_TEMP_PATH: &str = "/state/deploy-request.new";

/// The mode both files are created with.
///
/// `0600`: nothing but uid 0 needs to read or write it, and `ADR-015` §5.1's
/// application identities must not be able to do either. It is a defence in
/// depth rather than the defence — the containing directory is what actually
/// stops them — and it is cheap.
pub const REQUEST_MODE: u32 = 0o600;

/// **The words a machine uses when [`REQUEST_PATH`] is already occupied**, so
/// that a tool can recognise the one refusal it is allowed to wait out.
///
/// `cmd/rivet`'s `apply install` interlock composes its refusal around this, and
/// `rivet_remote::deploy` matches on it: a machine that already has a request
/// waiting will take it up on the supervisor's next convergence pass, so a
/// second deploy should wait rather than fail. Every other refusal from that
/// operation means *stop*.
///
/// **One constant rather than a string in each place**, and the reason is what
/// the alternative costs: a caller matching on prose it copied would keep
/// waiting out a refusal long after the machine had started giving a different
/// one, or — worse — stop waiting out this one the first time somebody reworded
/// a sentence. `ADR-011`: one authority for one fact, and the fact here is
/// *which refusal is temporary*.
///
/// It is deliberately narrow: no path, no digest, no punctuation. The refusal
/// around it names both and may keep changing.
pub const REQUEST_WAITING: &str = "an install request is already waiting";

/// What the *payload half* of a request may be, in bytes.
///
/// **Sixty-four, since 2026-09-12, and it used to be one Linux `PATH_MAX`.**
/// `ADR-052` §7 made the payload half a BLAKE3 digest rather than a path, and a
/// cap that stayed at 4096 would have been a cap about nothing: the field is now
/// a fixed-width token that [`parse_request`] checks exactly, so the size test
/// the supervisor performs before it reads is a formality standing in front of a
/// parser that refuses anything longer anyway.
///
/// **It is kept as a named constant rather than folded away**, because the
/// supervisor's `stat(2)`-before-`read(2)` discipline is the property, not the
/// number: a request file grown to fill the card must be refused without being
/// allocated inside PID 1's only child, and that check needs a bound to compare
/// against whatever the field happens to be.
pub const PAYLOAD_MAX_BYTES: u64 = crate::artifact::DIGEST_CHARS as u64;

/// The most the principal prefix may add to a request, in bytes.
///
/// **A ceiling with room in it, checked by a test rather than trusted.** The
/// longest prefix this build can render is
/// `from=` + [`PINNED_PREFIX`] + 64 hex characters + ` -- `, which is 80;
/// `a_rendered_principal_always_fits_its_own_ceiling` pins that against the
/// constants rather than against this arithmetic, so widening a field without
/// widening this fails a test instead of truncating an operator's path on a
/// machine.
pub const PRINCIPAL_MAX_BYTES: u64 = 256;

/// The most of a request that will ever be read, in bytes.
///
/// [`PAYLOAD_MAX_BYTES`] plus [`PRINCIPAL_MAX_BYTES`]. The number is not really
/// about its two halves: it is about the reader being a process that **must not
/// fail** (`ADR-017` §3.1), running on a machine whose `/state` any privileged
/// fault could have filled. The consuming side asks for the file's size and
/// refuses a larger one *without reading it*, so an enormous request costs one
/// `stat(2)`.
///
/// **It grew on 2026-08-24 and shrank on 2026-09-12**, and both movements are
/// the same discipline. It grew because the principal had to fit beside the
/// payload rather than eat into it — *"a path this machine can hold"* and *"a
/// path a request can name"* must not be two lengths. It shrank because the
/// payload stopped being a path: `ADR-052` §7 made it a digest, and a cap sized
/// for a `PATH_MAX` that nothing can contain any more would be a number nobody
/// could explain.
pub const REQUEST_MAX_BYTES: u64 = PAYLOAD_MAX_BYTES + PRINCIPAL_MAX_BYTES;

// ---------------------------------------------------------------------------
// Who asked — `ADR-028` §4.5, ruled 2026-08-24
// ---------------------------------------------------------------------------

/// What [`Origin::NoSession`] is spelled as in a request and in a record.
///
/// **It says what was observed, not what it implies.** *"Nothing in this
/// process's environment named a caller"* is the fact; *"somebody was at the
/// console"* is a guess, and a wrong one for a script, a `cron`-shaped caller, or
/// any parent that scrubbed its environment. The token is deliberately not the
/// word `console` for that reason — and since 2026-08-30 there IS a token that
/// means somebody was at the console, [`CONSOLE_TOKEN`], which is what makes
/// keeping these two apart worth the paragraph.
///
/// # The rule about renaming a wire token, and this one's exception
///
/// **A wire token is part of the interface contract and is not renamed.** A
/// reader matching this string may be reading a record years old, on a machine
/// whose journal is the only account of what happened (`ADR-022` §8), so a
/// rename makes the past unreadable rather than merely inconvenient.
///
/// **This token was renamed once, on 2026-09-06**, when the thing it was named
/// after stopped being part of this system at all — a token naming a mechanism
/// the machine no longer has is not a contract, it is a fossil. The cost is
/// bounded and it is paid out loud: a request written by a build from before
/// that date is [`RequestProblem::BadPrincipal`], which says in a record that
/// the value was not one this build accepts, rather than being installed
/// unattributed.
pub const NO_SESSION_TOKEN: &str = "no-session";

/// The wire token for [`Origin::Console`] — `ADR-035` §8A.1.2's break-glass.
///
/// **Part of the interface contract and never renamed**, on its siblings' rule:
/// a reader matching this string may be reading a record years old.
///
/// It is the *observed* fact, exactly as [`NO_SESSION_TOKEN`] is: a request
/// arrived on the serial console, which nothing reaches without being able to
/// touch the machine. That is a claim about access and not about identity, and
/// [`Origin::Console`]'s own text says so.
pub const CONSOLE_TOKEN: &str = "serial-console";

/// The field naming [`Principal::origin`] in a request.
///
/// **The only field there is**, and the parser says so rather than looping over
/// a set of one: a request is one origin, the marker, and a path. A newer Rivet
/// adding a second field is what [`RequestProblem::UnknownField`] exists for.
pub const FROM_FIELD: &str = "from=";

/// The token that ends the fields and begins the payload path.
///
/// **Everything after it and one space is the path, verbatim.** That is the
/// whole reason there is a marker rather than a field count: a path may contain
/// spaces, `=`, and every byte except NUL and a line ending, so no amount of
/// splitting from the right is safe. Scanning left to right until this token and
/// then stopping is the only rule that cannot be confused by a path.
pub const PAYLOAD_MARKER: &str = "--";

/// Where an install was asked for, as far as the asking process could honestly
/// tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// The caller reached `cmd/rivet-mcpd` over `ADR-035`'s control plane, and
    /// this is the Ed25519 public key it was pinned by — `ADR-025` §2.
    ///
    /// **The pinned key IS the identity**, which is what makes this the
    /// strongest thing a request can say about who asked. Nothing has to be
    /// matched against a log line to reach a key, because the key is what was
    /// written down; `ADR-025` §2's map turns it into a device or an operator
    /// without anything here inventing cryptography or carrying any secret.
    ///
    /// **It is a fact about the connection and not about a person.** Whoever
    /// holds the private half is who this names, and `ADR-028` §4.5 is the
    /// standing statement that holding it is not the same as being authorised
    /// by it — see [`Principal`]'s first paragraph.
    ///
    /// It carries the whole token, `pinned-<hex>`, so [`Origin::as_token`] can
    /// hand back a borrow. [`Origin::pinned_key`] takes the prefix off.
    ControlPlane(String),
    /// **Somebody was physically at the machine** — `ADR-035` §8A.1.2's
    /// break-glass over the serial console, added 2026-08-30.
    ///
    /// Physical presence IS the credential (§8A.1), so there is no session to
    /// name and no key to record. This is the one origin that means what
    /// [`Origin::NoSession`]'s own text says it does **not** mean, which is why
    /// it had to be its own variant rather than reuse: a process with a cleared
    /// environment and a person with a cable are different facts about who
    /// changed a machine, and the record must not merge them.
    Console,
    /// Nothing in the environment named a caller at all — see
    /// [`NO_SESSION_TOKEN`] for what that is and is not allowed to mean.
    NoSession,
}

impl Origin {
    /// The wire form. **Part of the interface contract and never renamed** —
    /// [`Outcome::as_str`]'s rule, for [`Outcome::as_str`]'s reason.
    pub fn as_token(&self) -> &str {
        match self {
            Self::ControlPlane(token) => token,
            Self::Console => CONSOLE_TOKEN,
            Self::NoSession => NO_SESSION_TOKEN,
        }
    }

    /// The Ed25519 key this caller was pinned by, if it came over the control
    /// plane.
    #[must_use]
    pub fn pinned_key(&self) -> Option<&str> {
        match self {
            Self::ControlPlane(token) => token.strip_prefix(PINNED_PREFIX),
            _ => None,
        }
    }

    /// Build one from a peer's key, as `cmd/rivet-mcpd` knows it.
    ///
    /// `None` when the key is not one this build will repeat — the same rule
    /// every other token here follows, and for the same reason: a value that
    /// cannot be echoed safely must not be echoed.
    #[must_use]
    pub fn pinned(hex: &str) -> Option<Self> {
        let hex = hex.trim().to_ascii_lowercase();
        is_pinned_key(&hex).then(|| Self::ControlPlane(format!("{PINNED_PREFIX}{hex}")))
    }

    /// Read an origin from a request's [`FROM_FIELD`], or `None` if it is not
    /// one this build will accept.
    ///
    /// **Every origin this build can render is one this build can read back**,
    /// and that is a test rather than a hope
    /// (`every_origin_this_build_renders_is_one_it_reads_back`). A variant that
    /// renders and does not parse is a request the writer thinks it sent and the
    /// supervisor refuses — invisible until the one occasion it matters, which
    /// for [`Origin::Console`] is a machine somebody has already had to walk
    /// over to.
    ///
    /// **The reserved words and a pinned key cannot collide**: a pinned token
    /// is 64 hex characters after [`PINNED_PREFIX`], neither reserved word has
    /// that shape, and `the_reserved_words_are_only_ever_themselves` pins it.
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            NO_SESSION_TOKEN => Some(Self::NoSession),
            CONSOLE_TOKEN => Some(Self::Console),
            _ => token
                .strip_prefix(PINNED_PREFIX)
                .filter(|hex| is_pinned_key(hex))
                .map(|_| Self::ControlPlane(token.to_string())),
        }
    }
}

/// What a control-plane origin is spelled with.
///
/// **A prefix rather than a bare hex string**, so a token says what kind of
/// thing it is before a reader has decided whether the rest of it parses — and
/// so a reserved word can never be read as a key, or a key as a reserved word,
/// by shape rather than by which check happened to run first.
pub const PINNED_PREFIX: &str = "pinned-";

/// The environment variable `cmd/rivet-mcpd` sets for the CLI it spawns.
///
/// **Named for what it carries.** The peer's Ed25519 public key as lower-case
/// hex, and nothing else may be read from it: [`Principal::from_environment`]
/// refuses a value that is not one.
pub const PEER_KEY_ENV: &str = "RIVET_CONTROL_PLANE_PEER";

/// Whether a token is a pinned Ed25519 key this build will repeat.
fn is_pinned_key(hex: &str) -> bool {
    hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// Who asked for an install, as far as the machine actually observed.
///
/// # What this is, stated before anything else
///
/// **Attribution, not authorisation.** Nothing here is checked against anything,
/// nothing here grants anything, and the trust boundary is exactly where it was:
/// *root on `/state`*. Anything running as uid 0 can write
/// [`REQUEST_PATH`] directly and put whatever it likes in these fields, and
/// nothing at this layer could stop it — `ADR-015` §7 already gives that
/// attacker `/state/app`. What changes is the **record**: the journal can now
/// answer *"who asked for this install"*, where the only previous answer was
/// *"somebody who was root"*.
///
/// The improvement that is real, and it is worth naming precisely: a claim that
/// can be checked is different from no claim at all. [`Origin::ControlPlane`]
/// names the same key the control plane's own records name for the connection
/// it arrived on, so an origin naming a caller that never connected is
/// **visible in the same journal** — a forgery that leaves evidence, rather
/// than a blank where the evidence would be.
///
/// # `ADR-028` §4, and what is still missing from it
///
/// §4 rules that authority comes from an authenticated principal. This is not
/// that. §4's principal is one whose authority is *checked*; nothing checks
/// anything here, and `ADR-028` §7.1's table is amended rather than deleted for
/// that reason. What this supplies is `ADR-012` §4's **actor provenance** and
/// `ADR-022` §5's actor, for the one event that had neither.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    origin: Origin,
}

impl Principal {
    /// A principal from an origin that has already been checked.
    pub const fn new(origin: Origin) -> Self {
        Self { origin }
    }

    /// What a process with no environment worth reading says about itself.
    pub const fn no_session() -> Self {
        Self::new(Origin::NoSession)
    }

    /// Where it came from.
    pub const fn origin(&self) -> &Origin {
        &self.origin
    }

    /// What a CLI process can honestly say about its own invocation.
    ///
    /// **Pure, and it takes the environment rather than reading it**, for
    /// [`crate::state`]'s reason (`ADR-014` §3.1): every branch below is a
    /// decision, and a decision reachable only by setting a variable in a real
    /// process is a decision with no test. `cmd/rivet` does the `std::env::var`
    /// call and hands the answer here.
    ///
    /// **An empty variable is read as unset.** That is the repository's existing
    /// idiom (`cmd/rivet-deploy`'s `bench`, `rivet-crypto`'s `loopback`) and it
    /// is the right one here: a variable set to nothing is a statement that
    /// there is nothing.
    ///
    /// **A value that is not a key is read as no key, and that is deliberate.**
    /// [`PEER_KEY_ENV`] reaches this process through an environment anything
    /// running as uid 0 can write, so the check is here rather than trusted from
    /// the daemon that set it; a value that fails it produces
    /// [`Origin::NoSession`] — *nothing named a caller* — rather than a token
    /// this build would then have to repeat into a record.
    pub fn from_environment(peer_key: Option<&str>) -> Self {
        peer_key
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .and_then(Origin::pinned)
            .map_or_else(Self::no_session, Self::new)
    }

    /// The prefix of a request line, without the trailing [`PAYLOAD_MARKER`].
    ///
    /// **Public since 2026-08-25**, when `crate::uninstall` grew a second
    /// request kind that names a principal the same way. One renderer, so the
    /// two files cannot disagree about what a principal looks like on disk.
    pub fn render_fields(&self) -> String {
        let mut out = String::with_capacity(FROM_FIELD.len() + self.origin.as_token().len());
        out.push_str(FROM_FIELD);
        out.push_str(self.origin.as_token());
        out
    }

    /// What it says, in a sentence a record carries and an operator reads.
    ///
    /// **Every one of them ends by saying what it is not.** `ADR-006` §4.1's
    /// human string, on a subject where the failure mode is a reader promoting
    /// an observation into an authorisation.
    ///
    /// This is the **install request's** framing of [`Self::describes`]. See
    /// that method for why the framing is a parameter.
    pub fn describe(&self) -> String {
        self.describes(
            "the request",
            "`rivet apply install`",
            &format!(
                "anything running as uid 0 can write {REQUEST_PATH} itself and claim anything \
                 in it"
            ),
        )
    }

    /// The same sentence, for an act that is not an install request.
    ///
    /// **Added 2026-08-24 with `rivet apply edit`** (`ADR-006` §8.9), and the
    /// parameters are exactly the three words that were install-specific:
    /// what was written, what wrote it, and how somebody could have forged it.
    /// Everything that is actually about the *principal* — what a client address
    /// is worth, what a key label is and is not, that none of it was checked —
    /// is written **once**, here, which is the point.
    ///
    /// `ADR-011`: a second operation recording who asked must not grow a second
    /// account of what "who asked" means. The two acts differ in what they wrote
    /// and in nothing else.
    pub fn describes(&self, what: &str, by: &str, forgeable: &str) -> String {
        let who = match &self.origin {
            Origin::ControlPlane(_) => format!(
                "{what} was written by {by} over ADR-035's control plane, by a caller holding \
                 the pinned Ed25519 key {key}. That key IS the identity rather than a pointer \
                 to one: nothing has to be matched against a log line to reach it, and \
                 ADR-025 §2's map turns it into a device or an operator",
                key = self.origin.pinned_key().unwrap_or("<unreadable>"),
            ),
            Origin::Console => format!(
                "{what} was written by {by} over the SERIAL CONSOLE — `ADR-035` §8A.1.2's \
                 break-glass. There is no key and no session in this record because there was \
                 none to have: physical presence is the credential, so what this line attests \
                 is that somebody could reach the machine, and nothing about who they were"
            ),
            Origin::NoSession => format!(
                "{what} was written by a process whose environment named no caller at all. That \
                 is NOT the same as \"somebody was at the console\": a script, or any parent \
                 that cleared its environment, looks identical from here"
            ),
        };
        format!(
            "{who}. THIS IS ATTRIBUTION AND NOT AUTHORISATION: {forgeable}, and nothing \
             checked this (ADR-028 §4.5). What it buys is a record that can be contradicted by \
             the journal rather than no record at all."
        )
    }
}

/// Both halves of a request.
///
/// **The payload half is a digest, since 2026-09-12** — `ADR-052` §7. It was an
/// absolute path, and that is what made the file worth more to an attacker than
/// it is now: a request could name *any* file on `/state`, so writing one bought
/// the ability to name something that never passed through the fleet
/// repository. A digest resolves in `crate::artifact`'s store and nowhere else,
/// so what writing a request now buys is the ability to name **something this
/// machine already has**. The trust boundary is unchanged and is still *root on
/// `/state`* — this is a narrower capability, not a new wall.
///
/// It no longer borrows, because neither half does any more: [`Digest`] owns its
/// characters because it had to check them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The digest of the artifact to install, resolved in this machine's own
    /// store.
    pub digest: Digest,
    /// Who the writing process said asked for it. See [`Principal`] — and read
    /// its first paragraph before believing any of it.
    pub principal: Principal,
}

/// Why a request is not one.
///
/// `ADR-006` §4.1's shape, one layer down: a token to branch on and a sentence
/// to read. The consuming side does not act on the distinctions — every one of
/// them ends the same way, with the request removed and a record written — but
/// **the record has to say which**, because "the supervisor threw your install
/// away" is only actionable if it says what was wrong with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestProblem {
    /// Nothing, or only a newline. A file that exists and says nothing.
    Empty,
    /// More than [`REQUEST_MAX_BYTES`].
    TooLarge { bytes: usize },
    /// The payload half is not a BLAKE3 digest.
    ///
    /// **This replaced `NotAbsolute` on 2026-09-12**, and the replacement is the
    /// whole of `ADR-052` §7 arriving in the parser: a request used to be
    /// refused for naming a *relative* path, which meant every absolute one was
    /// accepted. Now nothing that is not sixty-four lowercase hex characters is
    /// a request at all, so there is no path — relative, absolute or otherwise —
    /// that this file can carry.
    NotADigest { payload: String },
    /// A control character — a NUL, a tab, an escape.
    ControlCharacter { at: usize },
    /// Something after the first line.
    ///
    /// Refused rather than ignored, because a second line is the shape a
    /// *format change* would arrive in, and silently reading the first line of a
    /// richer request would act on half of what somebody meant.
    MoreThanOneLine,
    /// The line does not begin with [`FROM_FIELD`], so it says nothing about who
    /// asked.
    ///
    /// **Refused, and this is the decision worth arguing** — `ADR-028` §4.5
    /// carries it in full. The short form is that there is exactly one writer of
    /// this format, so a file that is not in it was not written by
    /// `rivet apply install`; and that accepting a principal-less request
    /// would leave a permanent, silent way to install with no attribution, which
    /// would make the field decorative. The cost of refusing is one install that
    /// has to be asked for again, said out loud in the journal.
    ///
    /// This is also what an install requested by a build from **before**
    /// 2026-08-24 looks like — one bare absolute path — which is why the message
    /// names that case rather than only describing the grammar.
    NoPrincipal { head: String },
    /// A [`FROM_FIELD`] whose value this build will not accept.
    BadPrincipal { field: String, value: String },
    /// A token before [`PAYLOAD_MARKER`] that is not a field this build knows.
    ///
    /// A newer Rivet adding a field is the ordinary way this happens, and it
    /// is [`Outcome::from_record_field`]'s rule in the other direction: refuse
    /// the whole request rather than act on the part of it that parsed.
    UnknownField { field: String },
    /// The same field twice.
    ///
    /// Refused rather than last-wins or first-wins, because either choice is a
    /// rule an attacker gets to know and a reader does not.
    RepeatedField { field: String },
    /// Fields, and then the line ended without [`PAYLOAD_MARKER`] and a path.
    NoPayload,
}

impl RequestProblem {
    /// What it means, for the record that reports it.
    pub fn why(&self) -> String {
        match self {
            Self::Empty => "it is empty. A request is who asked and one artifact digest"
                .to_string(),
            Self::TooLarge { bytes } => format!(
                "it is {bytes} bytes, and a request may be at most {REQUEST_MAX_BYTES}. A request \
                 is one line: who asked, and one {chars}-character digest",
                chars = crate::artifact::DIGEST_CHARS,
            ),
            Self::NotADigest { payload } => format!(
                "{payload:?} is not an artifact digest. A request names {chars} lowercase hex \
                 characters, which the supervisor resolves in this machine's own artifact store \
                 — there is no form of it that names a path (ADR-052 §7). A request naming a \
                 path is either a build from before 2026-09-12 or something written by hand"
                , chars = crate::artifact::DIGEST_CHARS,
            ),
            Self::ControlCharacter { at } => format!(
                "there is a control character at byte {at}. A digest has none, so this file \
                 was not written by `rivet apply install`"
            ),
            Self::MoreThanOneLine => "there is more than one line in it. A request is exactly one \
                                      line, and a second line would be a format this build does \
                                      not know rather than something to read half of"
                .to_string(),
            Self::NoPrincipal { head } => format!(
                "it begins with {head:?} and a request begins with `{FROM_FIELD}`, so this one \
                 does not say who asked for the install. A bare path is the format Rivet wrote \
                 before 2026-08-24; it is refused rather than installed unattributed, because an \
                 install nobody is named for is the thing ADR-028 §4.5 exists to stop being \
                 possible by accident. Ask again with `rivet apply install <digest> confirm`, \
                 which writes the whole line"
            ),
            Self::BadPrincipal { field, value } => format!(
                "the `{field}` field says {value:?}, which is not a value this build will repeat \
                 into a record. An origin is `{PINNED_PREFIX}` and 64 hex characters, or one of \
                 the two reserved words `{NO_SESSION_TOKEN}` and `{CONSOLE_TOKEN}` — see \
                 rivet_core::deploy"
            ),
            Self::UnknownField { field } => format!(
                "there is a `{field}` before the `{PAYLOAD_MARKER}`, and this build knows \
                 `{FROM_FIELD}` and nothing else. A newer Rivet writing a field this one does \
                 not know is the ordinary reason; the whole request is refused rather than the \
                 part of it that parsed"
            ),
            Self::RepeatedField { field } => format!(
                "`{field}` appears twice. Neither first-wins nor last-wins is a rule worth \
                 having, because whoever wrote the second one would know it and whoever reads \
                 the record would not"
            ),
            Self::NoPayload => format!(
                "it names who asked and then stops: there is no `{PAYLOAD_MARKER}` followed by \
                 an artifact digest, so nothing was named to install"
            ),
        }
    }
}

impl core::fmt::Display for RequestProblem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.why())
    }
}

/// The bytes of a request from `principal` naming `digest`.
///
/// ```text
/// from=pinned-3f7c1a9e04b2d85f6a3c9017e2b48d5f1c6a09b3e7d248fa5601c93b8ed72a4f -- 8c4e9b21f0a37d5e6b18c92470fd3a85e1c7b0946d2af38e5710cb639ad84f2c
/// ```
///
/// **Still one line, and the shape of it survived the payload changing.** The
/// fields come first because every one of them is drawn from a bounded alphabet
/// with no spaces in it; the payload comes last because it *was* the one field
/// that could contain anything, and `everything after the marker` was the only
/// rule a path could not break. Since 2026-09-12 the payload is a digest and
/// could go anywhere, and the order is kept unchanged: the grammar being
/// positional is what lets [`parse_request`] refuse the pre-2026-08-24 format
/// with one comparison, and re-ordering it to celebrate a narrower field would
/// buy nothing and cost that.
///
/// A trailing newline, because a file without one is the one an operator's `cat`
/// runs together with their prompt — and because [`parse_request`] has to accept
/// both anyway.
pub fn render_request(digest: &Digest, principal: &Principal) -> String {
    let fields = principal.render_fields();
    let payload = digest.as_str();
    let mut out = String::with_capacity(fields.len() + PAYLOAD_MARKER.len() + payload.len() + 3);
    out.push_str(&fields);
    out.push(' ');
    out.push_str(PAYLOAD_MARKER);
    out.push(' ');
    out.push_str(payload);
    out.push('\n');
    out
}

/// The artifact digest and the principal a request names, or why it is not a
/// request.
///
/// **Pure, and it is the only reader.** `cmd/rivet-sysd` calls this
/// rather than splitting the string itself, so there is no second understanding
/// of the format to drift from the first.
///
/// It accepts a trailing newline and a trailing `\r\n`, and nothing else that
/// looks like whitespace: a path may legitimately begin or end with a space, so
/// trimming one would silently install a different file from the one named.
///
/// # The grammar is positional, and that is on purpose
///
/// [`FROM_FIELD`] first, then [`PAYLOAD_MARKER`], then the digest. A fixed order
/// costs nothing to write and buys two things: the pre-2026-08-24 format — one
/// bare path — is detected by a single comparison and refused with a sentence
/// that says so, and there is no ordering an attacker can vary to find a reader
/// that behaves differently from the one that was tested.
pub fn parse_request(contents: &str) -> Result<Request, RequestProblem> {
    let cap = usize::try_from(REQUEST_MAX_BYTES).unwrap_or(usize::MAX);
    if contents.len() > cap {
        return Err(RequestProblem::TooLarge {
            bytes: contents.len(),
        });
    }

    // At most ONE trailing line ending, stripped in the order a writer produced
    // it. Anything else after the first line is refused below rather than
    // dropped.
    let body = contents
        .strip_suffix('\n')
        .map_or(contents, |s| s.strip_suffix('\r').unwrap_or(s));

    if body.is_empty() {
        return Err(RequestProblem::Empty);
    }
    if body.contains('\n') || body.contains('\r') {
        return Err(RequestProblem::MoreThanOneLine);
    }
    // **Before anything is split**, so a NUL smuggled into a field value is a
    // control character rather than a value some later reader gets to see.
    if let Some(at) = body.bytes().position(|b| b < 0x20 || b == 0x7f) {
        return Err(RequestProblem::ControlCharacter { at });
    }

    let (principal, payload) = split_principal(body)?;

    let Some(digest) = Digest::new(payload) else {
        return Err(RequestProblem::NotADigest {
            payload: payload.to_string(),
        });
    };
    Ok(Request { digest, principal })
}

/// The fields at the head of a request line, and the rest of the line after the
/// marker.
///
/// Left to right, one token at a time, and **it never looks at the payload** —
/// the moment [`PAYLOAD_MARKER`] is seen the remainder is handed back untouched.
///
/// **Public since 2026-08-25** as `split_principal`, because `crate::uninstall`
/// is a second request kind that names a principal the same way and must read
/// it with the same code. The name changed with the visibility: `parse_` is
/// what a whole request gets, and this is one field of one.
pub fn split_principal(body: &str) -> Result<(Principal, &str), RequestProblem> {
    let Some(rest) = body.strip_prefix(FROM_FIELD) else {
        return Err(RequestProblem::NoPrincipal {
            head: body.split(' ').next().unwrap_or(body).to_string(),
        });
    };
    let (origin_token, rest) = split_token(rest);
    let Some(origin) = Origin::from_token(origin_token) else {
        return Err(RequestProblem::BadPrincipal {
            field: FROM_FIELD.to_string(),
            value: origin_token.to_string(),
        });
    };

    // **One field, so the next token is the marker or the request is refused.**
    // There is no loop over a set of fields because there is no set: a second
    // token that is not `--` is either a repeated `from=` or a field this build
    // does not know, and both refuse the whole request rather than the part of
    // it that parsed.
    let Some(remainder) = rest else {
        return Err(RequestProblem::NoPayload);
    };
    let (token, after) = split_token(remainder);

    if token == PAYLOAD_MARKER {
        // Everything after the marker and its one space, verbatim. `None` is a
        // line that ends at the marker, which named no payload.
        return match after {
            Some(payload) => Ok((Principal::new(origin), payload)),
            None => Err(RequestProblem::NoPayload),
        };
    }

    if token.starts_with(FROM_FIELD) {
        return Err(RequestProblem::RepeatedField {
            field: FROM_FIELD.to_string(),
        });
    }

    Err(RequestProblem::UnknownField {
        field: token.to_string(),
    })
}

// ---------------------------------------------------------------------------
// What this machine is already running — PLAN-Feature-Set-2026-09-12 §3
// ---------------------------------------------------------------------------

/// **The words a machine uses when an install names what it is already
/// running**, so a host tool can stop there and say so rather than report a
/// failure.
///
/// [`REQUEST_WAITING`]'s arrangement exactly, and for its reason: one constant,
/// stated by the machine that composes the refusal and matched by the tool that
/// reads it. `cmd/rivet`'s install channel and `cmd/rivet-deploy` both compose
/// their refusal through [`already_running`], and `rivet_remote::deploy` matches
/// on this.
pub const ALREADY_RUNNING: &str = "this machine is already running this artifact";

/// Where the signed header of what each A/B slot holds is retained.
///
/// **The slot's counterpart to an application's retained header**
/// (`crate::package::install::HEADER_NAME` beside the version `current`
/// selects). An application's version directory carries the header that
/// describes it, so what `current` points at is answerable by reading 4 KiB. A
/// slot cannot carry its own: its system partition is a filesystem image the
/// header describes, and its boot partition is — for slot A — the FAT
/// filesystem holding `autoboot.txt`, which `ADR-029` §9 keeps an update from
/// opening for writing whenever it can. So the header lives on `/state`, which
/// an update never touches (`ADR-018` §5), keyed by slot.
///
/// **Written by `cmd/rivet-deploy` and nothing else, which is what makes it
/// current.** On a running machine the installer is what writes a build into a
/// slot — arming and committing rewrite `autoboot.txt`, which belongs to no build
/// — and it empties a slot's record **before** its first byte reaches that slot and
/// writes the header only once the system member and the boot half are written
/// (and read back, unless that was switched off). So a record names a build
/// exactly while the slot holds it, for every write a running machine can make.
/// What it cannot see is a slot written from outside Rivet — a card imaged or a
/// partition copied on another machine — and an imaged card has an empty
/// `/state`, so such a slot has **no** record and reads as not known to be
/// running anything, which is the behaviour every card had before this existed.
pub const SLOT_HEADERS: &str = "/state/slots";

/// The directory holding `slot`'s retained header.
#[must_use]
pub fn slot_header_dir(slot: crate::state::Slot) -> String {
    format!("{SLOT_HEADERS}/{}", slot.as_str())
}

/// The retained header of what `slot` holds. See [`SLOT_HEADERS`].
#[must_use]
pub fn slot_header_path(slot: crate::state::Slot) -> String {
    format!(
        "{}/{}",
        slot_header_dir(slot),
        crate::package::install::HEADER_NAME
    )
}

/// The retained header of the version of `name` that `current` selects on
/// `volume` — what that application is running.
#[must_use]
pub fn running_application_header_path(volume: &str, name: &crate::service::ServiceName) -> String {
    format!(
        "{}/{}/{}",
        crate::service::app_dir(volume, name),
        crate::service::CURRENT_LINK,
        crate::package::install::HEADER_NAME
    )
}

/// Something this machine is running, as the thing whose retained header was
/// compared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Running {
    /// The version of this application `current` selects.
    Application(String),
    /// The build this A/B slot holds, and the machine booted it.
    System(crate::state::Slot),
}

/// **Is `head` — an artifact's first 4 KiB — the retained header of something
/// this machine is running?** The first match, or `None`.
///
/// # Why the header is the artifact's identity
///
/// The signed header carries the kind, product, version and architecture and
/// **every member's offset, length and digest**, and `package::read::open`
/// refuses a file with bytes past its last member. So two artifacts whose
/// headers are byte-identical are the same signed statement about the same
/// bytes, and a retained header equal to an artifact's is that artifact. **It is
/// compared as bytes and not parsed**: nothing here reads a field of it, so this
/// is not a second reader of the container (`ADR-029` §11), and a header that
/// does not verify cannot be retained in the first place, because only the
/// installer writes one and only after it verified.
///
/// A header shorter than [`crate::package::format::HEADER_BYTES`] matches
/// nothing, so a truncated read can never be taken for a match.
#[must_use]
pub fn already_running<'a>(head: &[u8], retained: &'a [(Running, Vec<u8>)]) -> Option<&'a Running> {
    if head.len() != crate::package::format::HEADER_BYTES {
        return None;
    }
    retained
        .iter()
        .find(|(_, bytes)| bytes.as_slice() == head)
        .map(|(running, _)| running)
}

/// The refusal for an install of what this machine is already running, in the
/// words both `cmd/rivet` and `cmd/rivet-deploy` say it. Carries
/// [`ALREADY_RUNNING`].
#[must_use]
pub fn already_running_refusal(artifact: &str, running: &Running) -> String {
    let (what, consequence) = match running {
        Running::Application(name) => (
            format!("the version of {name} that `current` selects"),
            "Installing it again would change nothing: `current` already points at these bytes, \
             and nothing is ever unpacked over a version directory that is there (ADR-015 §7)."
                .to_string(),
        ),
        Running::System(slot) => (
            format!("the build in slot {}, which this machine booted", slot.as_str()),
            format!(
                "Installing it would write a second copy of the running build into slot {}, the \
                 INACTIVE slot — replacing whatever older build is there, which is this machine's \
                 only rollback.",
                slot.other().as_str()
            ),
        ),
    };
    format!(
        "REFUSED: {ALREADY_RUNNING} — {artifact} is {what}, byte for byte (its signed header is \
         the one retained beside it). {consequence} NOTHING WAS INSTALLED AND NOTHING NEEDS TO BE. \
         The artifact stays in this machine's store. PLAN-Feature-Set-2026-09-12 §3."
    )
}

// ---------------------------------------------------------------------------
// The trial window — PLAN-Feature-Set-2026-09-12 §3
// ---------------------------------------------------------------------------

/// **The words a machine uses when a system update is asked for while this
/// boot's trial is undecided**, so a host tool can stop there and say the
/// request was refused rather than delivered.
///
/// [`ALREADY_RUNNING`]'s arrangement, for the same reason: `cmd/rivet`'s
/// install channel composes its refusal through [`uncommitted_trial`], and
/// `cmd/rivet-deploy`'s installer answers the identical question in its own
/// words, independently, after a request has already reached it — defence in
/// depth rather than a second reader of one decision.
pub const UNCOMMITTED_TRIAL: &str = "this boot is not committed";

/// **Is a system update, right now, about to write the slot an ordinary reset
/// falls back to?** `ADR-018` §5's hazard, stated as a comparison: an update
/// writes the slot that is not running ([`crate::state::Slot::other`]), and a
/// machine is committed exactly when the default slot is the running one. So
/// the update would write the default slot exactly when the two differ, and
/// that is the comparison made here.
///
/// `Some(default)` names the slot an ordinary reset boots — the slot this
/// update would target — when the two differ. `None` when the boot is
/// committed.
#[must_use]
pub fn uncommitted_trial(
    running: crate::state::Slot,
    default: crate::state::Slot,
) -> Option<crate::state::Slot> {
    (default != running).then_some(default)
}

/// The refusal for a system-update install requested while [`uncommitted_trial`]
/// says the trial is undecided. Carries [`UNCOMMITTED_TRIAL`].
///
/// **Worded for the moment before anything is delivered.** `cmd/rivet`'s
/// install channel is the one caller, consulted before a request is written to
/// `/state` at all — `PLAN-Feature-Set-2026-09-12` §3: on `rivet-01`, `apply
/// install` answered `requested: true` and exit 0 during exactly this window,
/// and only `get install`, read afterwards, said the installer had refused it.
#[must_use]
pub fn uncommitted_trial_refusal(
    running: crate::state::Slot,
    default: crate::state::Slot,
) -> String {
    let running = running.as_str();
    let default = default.as_str();
    format!(
        "REFUSED: {UNCOMMITTED_TRIAL} — this machine is running slot {running}, and an ordinary \
         reset boots slot {default}, which is what an uncommitted one-shot trial looks like \
         (ADR-018 §5). This update would write slot {default}, the slot that reset falls back to, \
         while the trial is undecided: a reset or a power cut part-way through that write boots a \
         half-written system, and a card is what recovers that. NOTHING HAS BEEN DELIVERED. Either \
         let the trial commit — PID 1 confirms a healthy boot, and `rivet get slot` then says \
         `committed: true` — or reset to roll back to slot {default}; then install again. \
         PLAN-Feature-Set-2026-09-12 §3."
    )
}

/// One space-separated token, and whatever follows the space.
///
/// `None` for the second half means the token ran to the end of the line, which
/// is **not** the same as an empty remainder: a line ending in `"--"` named no
/// payload at all ([`RequestProblem::NoPayload`]), and one ending in `"-- "`
/// named the empty path ([`RequestProblem::NotAbsolute`]). Both are refused, and
/// keeping them distinct is what lets each say the true thing.
fn split_token(text: &str) -> (&str, Option<&str>) {
    text.split_once(' ')
        .map_or((text, None), |(token, rest)| (token, Some(rest)))
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::service::ServiceName;

    // -- what this machine is already running --------------------------------

    fn header(fill: u8) -> Vec<u8> {
        vec![fill; crate::package::format::HEADER_BYTES]
    }

    /// **The whole rule**: an artifact whose header is a retained header of
    /// something running is that thing, and one whose header differs is not.
    #[test]
    fn an_artifact_is_running_exactly_when_its_header_is_one_retained_beside_what_runs() {
        let retained = vec![
            (Running::Application("photo-index".to_string()), header(1)),
            (Running::System(crate::state::Slot::B), header(2)),
        ];
        assert_eq!(
            already_running(&header(2), &retained),
            Some(&Running::System(crate::state::Slot::B))
        );
        assert_eq!(
            already_running(&header(1), &retained),
            Some(&Running::Application("photo-index".to_string()))
        );
        assert_eq!(already_running(&header(3), &retained), None);
        assert_eq!(already_running(&header(2), &[]), None);
    }

    /// **A short read is never a match**, even against a short record: an empty
    /// slot record is how the installer marks a slot it is about to write.
    #[test]
    fn a_header_that_is_not_a_whole_header_matches_nothing() {
        let retained = vec![(Running::System(crate::state::Slot::A), Vec::new())];
        assert_eq!(already_running(&[], &retained), None);
        let short = vec![(Running::System(crate::state::Slot::A), header(7)[..100].to_vec())];
        assert_eq!(already_running(&header(7)[..100], &short), None);
    }

    /// The refusal carries the constant a host tool matches on, and for a
    /// system update it names the rollback it would have cost.
    #[test]
    fn the_refusal_carries_the_words_a_tool_matches_and_names_what_it_would_cost() {
        let system = already_running_refusal("rivet-0.1.85.rvt", &Running::System(crate::state::Slot::B));
        assert!(system.contains(ALREADY_RUNNING), "{system}");
        assert!(system.contains("slot b"), "{system}");
        assert!(system.contains("slot a"), "{system}");
        assert!(system.contains("rollback"), "{system}");
        let app = already_running_refusal("x.rvt", &Running::Application("photo-index".to_string()));
        assert!(app.contains(ALREADY_RUNNING), "{app}");
        assert!(app.contains("current"), "{app}");
    }

    // -- the trial window -----------------------------------------------------

    /// **The whole rule**: differing slots is an uncommitted trial; the same
    /// slot twice is a committed boot and nothing here applies.
    #[test]
    fn a_default_that_differs_from_running_is_an_uncommitted_trial() {
        assert_eq!(
            uncommitted_trial(crate::state::Slot::A, crate::state::Slot::B),
            Some(crate::state::Slot::B)
        );
        assert_eq!(
            uncommitted_trial(crate::state::Slot::B, crate::state::Slot::A),
            Some(crate::state::Slot::A)
        );
        assert_eq!(
            uncommitted_trial(crate::state::Slot::A, crate::state::Slot::A),
            None
        );
        assert_eq!(
            uncommitted_trial(crate::state::Slot::B, crate::state::Slot::B),
            None
        );
    }

    /// The refusal carries the constant a host tool matches on, names both
    /// slots, and says plainly that nothing reached the card.
    #[test]
    fn the_trial_refusal_carries_the_words_a_tool_matches_and_says_nothing_was_delivered() {
        let refusal = uncommitted_trial_refusal(crate::state::Slot::A, crate::state::Slot::B);
        assert!(refusal.contains(UNCOMMITTED_TRIAL), "{refusal}");
        assert!(refusal.contains("slot a"), "{refusal}");
        assert!(refusal.contains("slot b"), "{refusal}");
        assert!(refusal.contains("NOTHING HAS BEEN DELIVERED"), "{refusal}");
    }

    #[test]
    fn the_retained_headers_live_where_the_installer_writes_them() {
        assert_eq!(
            slot_header_path(crate::state::Slot::A),
            "/state/slots/a/package-header"
        );
        let name = ServiceName::new("photo-index").expect("a name");
        assert_eq!(
            running_application_header_path(crate::service::BUILT_IN_VOLUME, &name),
            "/state/app/photo-index/current/package-header"
        );
    }

    /// **The name has to be one a cgroup and a socket can hold**, or the
    /// definition is unstartable on a board and perfect in every unit test that
    /// supplies its own bytes. `ADR-017` §3.1.3 and §3.4.
    #[test]
    fn the_service_name_is_one_a_cgroup_and_a_socket_can_hold() {
        assert!(
            ServiceName::new(SERVICE_NAME).is_some(),
            "{SERVICE_NAME} is not a usable service name: {}",
            ServiceName::RULE
        );
    }

    /// The binary lives on the read-only image, not on the partition it writes.
    #[test]
    fn the_installer_lives_on_system_and_is_named_after_the_service() {
        assert!(PROGRAM_PATH.starts_with("/system/bin/"), "{PROGRAM_PATH}");
        assert!(PROGRAM_PATH.ends_with(SERVICE_NAME), "{PROGRAM_PATH}");
        assert!(
            !PROGRAM_PATH.starts_with(crate::state::STATE_MOUNT_POINT),
            "an installer on /state cannot repair a /state its own write broke"
        );
    }

    /// Both directions round-trip, so a token read out of a record is the
    /// outcome that wrote it.
    #[test]
    fn every_outcome_has_a_token_and_every_token_is_an_outcome() {
        for outcome in Outcome::ALL {
            assert_eq!(Outcome::from_record_field(outcome.as_str()), Some(*outcome));
            assert_eq!(outcome.to_string(), outcome.as_str());
        }
    }

    /// **A token this build does not know is `None`, not a guess.** A newer
    /// Rivet writing a sixth outcome is the ordinary way this happens, and
    /// reading it as "unfinished" would tell an operator an install is still
    /// running on a machine where it ended hours ago.
    #[test]
    fn a_token_this_build_does_not_know_is_refused_rather_than_guessed() {
        for word in [
            "",
            "ok",
            "success",
            "failed",
            "Installed",
            "INSTALLED",
            "installed ",
            "install",
            "refused",
        ] {
            assert_eq!(
                Outcome::from_record_field(word),
                None,
                "{word:?} was read as an outcome"
            );
        }
    }

    /// No two outcomes may collide in either reading: an operator who read a
    /// plan as an install reboots expecting a new system and gets the old one.
    #[test]
    fn no_two_outcomes_read_alike_in_either_reading() {
        for (i, a) in Outcome::ALL.iter().enumerate() {
            assert!(!a.as_str().is_empty());
            assert!(a.means().len() > 40, "{a} does not say what it means");
            for b in Outcome::ALL.iter().skip(i + 1) {
                assert_ne!(a.as_str(), b.as_str());
                assert_ne!(a.means(), b.means(), "{a} and {b} say the same thing");
            }
        }
    }

    /// **The promise `ADR-029` §5's ordering makes**, as an assertion: the two
    /// dry runs and the refused payload wrote nothing, and exactly one outcome
    /// cannot say.
    #[test]
    fn only_a_machine_refusal_is_unable_to_say_whether_anything_was_written() {
        let unknown: Vec<&Outcome> = Outcome::ALL
            .iter()
            .filter(|o| o.wrote_nothing().is_none())
            .collect();
        assert_eq!(unknown, vec![&Outcome::RefusedMachine]);

        for outcome in [
            Outcome::Planned,
            Outcome::PlannedApplication,
            Outcome::RefusedPayload,
        ] {
            assert_eq!(outcome.wrote_nothing(), Some(true), "{outcome}");
        }
        for outcome in [Outcome::Installed, Outcome::InstalledApplication] {
            assert_eq!(outcome.wrote_nothing(), Some(false), "{outcome}");
        }
    }

    /// The two refusals are the only failures, and both dry runs are successes
    /// — a dry run that reported failure would make `--dry-run` useless as the
    /// thing you run first.
    #[test]
    fn exactly_the_two_refusals_are_failures() {
        let failed: Vec<&str> = Outcome::ALL
            .iter()
            .filter(|o| !o.succeeded())
            .map(|o| o.as_str())
            .collect();
        assert_eq!(failed, ["refused-payload", "refused-machine"]);
    }

    /// **The contract, written out**, so that renaming a token is a decision
    /// rather than a refactor that compiles. A record written by an older build
    /// is the only account of an install that has already happened.
    #[test]
    fn the_outcome_tokens_are_the_contract_and_this_is_the_list() {
        let tokens: Vec<&str> = Outcome::ALL.iter().map(|o| o.as_str()).collect();
        assert_eq!(
            tokens,
            [
                "installed",
                "installed-application",
                "planned",
                "planned-application",
                // Shared Rivets, 2026-08-31. **Appended**, never inserted: the
                // order here is `Outcome::ALL`'s and a reader of an older
                // record matches on the token rather than the position, but a
                // list that reordered would still make this test read as though
                // something had been renamed.
                "installed-library",
                "library-already-installed",
                "planned-library",
                "refused-payload",
                "refused-machine",
            ]
        );
    }

    // -- the request, the maintainer's 2026-08-24 ruling ------------------------------

    /// A pinned key, spelled out once, for every test that needs a real one.
    const A_KEY: &str = "3f7c1a9e04b2d85f6a3c9017e2b48d5f1c6a09b3e7d248fa5601c93b8ed72a4f";

    /// The principal every test below uses when it is not the thing under test.
    fn over_the_control_plane() -> Principal {
        Principal::new(Origin::pinned(A_KEY).expect("64 hex characters"))
    }

    /// The rendered origin of [`over_the_control_plane`], for the tests that
    /// spell a whole request line out.
    fn a_pinned_token() -> String {
        format!("{PINNED_PREFIX}{A_KEY}")
    }

    /// The artifact digest every request test below names.
    ///
    /// **Deliberately not [`A_KEY`]**, though both are 64 hex characters: a test
    /// whose principal and payload were the same string could not tell a parser
    /// that swapped them from one that did not.
    const A_DIGEST: &str = "8c4e9b21f0a37d5e6b18c92470fd3a85e1c7b0946d2af38e5710cb639ad84f2c";

    fn a_digest() -> crate::artifact::Digest {
        crate::artifact::Digest::new(A_DIGEST).expect("a fixture that is a digest")
    }

    /// **Against literal bytes, not against the writer.** `AGENTS.md` §4: a
    /// reader round-tripped only against its own writer proves the two agree
    /// with each other and nothing about the format — and this format is a file
    /// on a card that outlives both processes and both builds.
    #[test]
    fn a_request_is_one_principal_and_one_artifact_digest_on_one_line() {
        let expected = Request {
            digest: a_digest(),
            principal: over_the_control_plane(),
        };
        let from = a_pinned_token();
        assert_eq!(
            parse_request(&format!("from={from} -- {A_DIGEST}\n")),
            Ok(expected.clone())
        );
        // No trailing newline: what an operator's `echo -n` or a partial write
        // that happened to stop on a boundary leaves.
        assert_eq!(
            parse_request(&format!("from={from} -- {A_DIGEST}")),
            Ok(expected.clone())
        );
        // CRLF: `AGENTS.md` §10.5 — every multi-line thing on this project's
        // development host arrives with CR endings, so a request typed there and
        // copied over is the ordinary case rather than the exotic one.
        assert_eq!(
            parse_request(&format!("from={from} -- {A_DIGEST}\r\n")),
            Ok(expected)
        );
    }

    /// **The whole line, spelled out**, because a reader in another binary and
    /// another build has only these bytes to go on.
    #[test]
    fn the_bytes_of_a_request_are_these_bytes() {
        assert_eq!(
            render_request(&a_digest(), &over_the_control_plane()),
            "from=pinned-3f7c1a9e04b2d85f6a3c9017e2b48d5f1c6a09b3e7d248fa5601c93b8ed72a4f -- \
             8c4e9b21f0a37d5e6b18c92470fd3a85e1c7b0946d2af38e5710cb639ad84f2c\n"
        );
        assert_eq!(
            render_request(&a_digest(), &Principal::no_session()),
            format!("from=no-session -- {A_DIGEST}\n")
        );
        assert_eq!(
            render_request(&a_digest(), &Principal::new(Origin::Console)),
            format!("from=serial-console -- {A_DIGEST}\n")
        );
    }

    /// The writer's output is what the reader accepts, which is the *other*
    /// half — necessary, and on its own not sufficient.
    #[test]
    fn what_the_cli_writes_is_what_the_supervisor_reads() {
        let principals = [
            over_the_control_plane(),
            Principal::no_session(),
            Principal::new(Origin::Console),
            Principal::new(Origin::pinned(&"a".repeat(64)).expect("64 hex characters")),
        ];
        for principal in &principals {
            for text in [A_DIGEST, &"0".repeat(64), &"f".repeat(64)] {
                let digest =
                    crate::artifact::Digest::new(text).expect("a fixture that is a digest");
                let bytes = render_request(&digest, principal);
                assert_eq!(
                    parse_request(&bytes),
                    Ok(Request {
                        digest,
                        principal: principal.clone(),
                    }),
                    "{bytes:?}"
                );
            }
        }
    }

    /// **Nothing after the marker is trimmed**, which used to matter because a
    /// path could begin or end with a space and now matters for the opposite
    /// reason: a digest cannot, so a line with stray whitespace in it was not
    /// written by `rivet apply install` and is refused rather than tidied into
    /// something installable.
    #[test]
    fn whitespace_around_the_payload_is_refused_rather_than_trimmed_away() {
        for line in [
            format!("from=no-session --  {A_DIGEST}\n"),
            format!("from=no-session -- {A_DIGEST} \n"),
        ] {
            assert!(
                matches!(parse_request(&line), Err(RequestProblem::NotADigest { .. })),
                "{line:?}"
            );
        }
    }

    /// **A path in the payload half is refused by the parser**, not only by the
    /// command line — `ADR-052` §7. The operation is one writer of this file;
    /// anything running as uid 0 is another, and this is the rule that binds
    /// both.
    #[test]
    fn a_request_naming_a_path_is_refused_however_it_was_written() {
        for path in [
            "/state/x.rvt",
            "/state/carried/tempmon/payload.rvt",
            "../../etc/x",
            "state/x.rvt",
        ] {
            let line = format!("from=no-session -- {path}\n");
            assert_eq!(
                parse_request(&line),
                Err(RequestProblem::NotADigest {
                    payload: path.to_string(),
                }),
                "{line:?}"
            );
        }
        // And the refusal says which rule it is, so a record naming it is
        // actionable rather than merely negative.
        let why = RequestProblem::NotADigest {
            payload: "/state/x.rvt".to_string(),
        }
        .why();
        assert!(why.contains("ADR-052 §7"), "{why}");
        assert!(why.contains("artifact store"), "{why}");
    }

    /// Every way a file on a writable filesystem can fail to be a request, and
    /// **each is refused rather than read past** (`AGENTS.md` §5).
    #[test]
    fn a_file_that_is_not_a_request_is_refused_rather_than_guessed_at() {
        let cases: &[(String, RequestProblem)] = &[
            (String::new(), RequestProblem::Empty),
            ("\n".to_string(), RequestProblem::Empty),
            ("\r\n".to_string(), RequestProblem::Empty),
            (
                format!("from=no-session -- {A_DIGEST}\nfrom=no-session -- {A_DIGEST}\n"),
                RequestProblem::MoreThanOneLine,
            ),
            (
                format!("from=no-session -- {A_DIGEST}\n\n"),
                RequestProblem::MoreThanOneLine,
            ),
            (
                format!("from=no-session -- {}\0{}\n", &A_DIGEST[..10], &A_DIGEST[11..]),
                RequestProblem::ControlCharacter { at: 29 },
            ),
            (
                format!("\tfrom=no-session -- {A_DIGEST}\n"),
                RequestProblem::ControlCharacter { at: 0 },
            ),
        ];
        for (bytes, expected) in cases {
            assert_eq!(parse_request(bytes), Err(expected.clone()), "{bytes:?}");
        }
    }

    /// **The format that came before this one is refused, not accepted.** One
    /// bare absolute path is what `rivet apply install` wrote until
    /// 2026-08-24, and it is also what somebody's `echo` writes — see `ADR-028`
    /// §4.5 for why the answer is *refuse* rather than *install it and say
    /// nobody was named*.
    #[test]
    fn a_request_that_names_nobody_is_refused_rather_than_installed_unattributed() {
        for bare in ["/state/x.rvt\n", "/state/x.rvt", "/state/a b.rvt\n"] {
            let Err(RequestProblem::NoPrincipal { head }) = parse_request(bare) else {
                unreachable!("{bare:?} was accepted, or refused for the wrong reason")
            };
            assert!(head.starts_with("/state/"), "{head:?}");
        }
        // ...and the refusal names the command that writes the whole line,
        // because an operator holding a refused install needs the next step and
        // not a grammar.
        let problem = RequestProblem::NoPrincipal {
            head: "/state/x.rvt".to_string(),
        };
        assert!(problem.why().contains("rivet apply install"), "{problem:?}");
    }

    /// Every shape of a principal this build will not repeat, and **each is
    /// refused as a whole request** rather than read as far as it parsed.
    #[test]
    fn a_principal_this_build_will_not_repeat_refuses_the_whole_request() {
        let cases: &[(&str, RequestProblem)] = &[
            (
                // Not a reserved word and not a pinned key.
                "from=somebody -- /state/x.rvt\n",
                RequestProblem::BadPrincipal {
                    field: FROM_FIELD.to_string(),
                    value: "somebody".to_string(),
                },
            ),
            (
                "from= -- /state/x.rvt\n",
                RequestProblem::BadPrincipal {
                    field: FROM_FIELD.to_string(),
                    value: String::new(),
                },
            ),
            (
                // A `/` would let an origin impersonate a path in a record.
                "from=/state/x:1 -- /state/x.rvt\n",
                RequestProblem::BadPrincipal {
                    field: FROM_FIELD.to_string(),
                    value: "/state/x:1".to_string(),
                },
            ),
            (
                // The prefix without a key behind it. A partial match must not
                // be a match.
                "from=pinned- -- /state/x.rvt\n",
                RequestProblem::BadPrincipal {
                    field: FROM_FIELD.to_string(),
                    value: "pinned-".to_string(),
                },
            ),
            (
                "no-session -- /state/x.rvt\n",
                RequestProblem::NoPrincipal {
                    head: "no-session".to_string(),
                },
            ),
            (
                "from=no-session who=root -- /state/x.rvt\n",
                RequestProblem::UnknownField {
                    field: "who=root".to_string(),
                },
            ),
            (
                // The old `key=` field, which this build does not know: refused
                // as a whole request rather than read as far as it parsed.
                "from=no-session key=a -- /state/x.rvt\n",
                RequestProblem::UnknownField {
                    field: "key=a".to_string(),
                },
            ),
            (
                "from=no-session from=serial-console -- /state/x.rvt\n",
                RequestProblem::RepeatedField {
                    field: FROM_FIELD.to_string(),
                },
            ),
            ("from=no-session\n", RequestProblem::NoPayload),
            ("from=no-session --\n", RequestProblem::NoPayload),
        ];
        for (bytes, expected) in cases {
            assert_eq!(parse_request(bytes), Err(expected.clone()), "{bytes:?}");
        }
    }

    /// **An origin cannot grow without bound**, which is what keeps the prefix
    /// inside [`PRINCIPAL_MAX_BYTES`] no matter what a file on a writable
    /// partition says. Every origin this build accepts is a fixed length: two
    /// reserved words, and a prefix followed by exactly 64 hex characters.
    #[test]
    fn an_origin_longer_than_a_key_is_refused_rather_than_shortened() {
        for token in [
            format!("{PINNED_PREFIX}{}", "a".repeat(65)),
            format!("{PINNED_PREFIX}{}", "a".repeat(63)),
            // Upper case is normalised by `Origin::pinned` on the way in and is
            // NOT accepted on the way back, because a record carries what was
            // written and two spellings of one key would compare unequal.
            format!("{PINNED_PREFIX}{}", "A".repeat(64)),
        ] {
            assert!(
                matches!(
                    parse_request(&format!("from={token} -- {A_DIGEST}\n")),
                    Err(RequestProblem::BadPrincipal { .. })
                ),
                "{token} was accepted as an origin"
            );
        }

        // **And a wildly long one is refused by the SIZE cap, not by the origin
        // rule.** The two guards are in that order and the order is worth
        // asserting: since `REQUEST_MAX_BYTES` shrank to a digest plus a
        // principal (2026-09-12, `ADR-052` §7), a 4 KiB origin no longer
        // reaches the parser's field checks at all. Either refusal is correct
        // and neither is an acceptance, which is what this asserts.
        let enormous = format!("{PINNED_PREFIX}{}", "a".repeat(4096));
        assert!(
            parse_request(&format!("from={enormous} -- {A_DIGEST}\n")).is_err(),
            "a 4 KiB origin was accepted"
        );
    }

    /// **The bug [`Origin::ControlPlane`] exists for**, kept as an assertion
    /// about the survivor: a caller that reaches the machine over the control
    /// plane is recorded by the key it was pinned by, and by nothing else that
    /// happened to be in the daemon's environment.
    #[test]
    fn a_pinned_peer_is_what_a_control_plane_caller_is_recorded_as() {
        let hex = "a".repeat(64);
        let principal = Principal::from_environment(Some(&hex));
        assert_eq!(principal.origin().pinned_key(), Some(hex.as_str()));
        assert_eq!(principal.origin().as_token(), format!("{PINNED_PREFIX}{hex}"));
    }

    /// With no peer key at all, the honest answer is that nothing named a
    /// caller — **and not a console**, which is a different fact with its own
    /// token.
    #[test]
    fn without_a_peer_key_the_answer_is_that_nothing_named_a_caller() {
        for absent in [None, Some(""), Some("   ")] {
            assert_eq!(Principal::from_environment(absent), Principal::no_session());
        }
        // A value that is not a key is read as no key rather than repeated: the
        // variable reaches this process through an environment anything running
        // as uid 0 can write.
        for junk in ["banana", "10.0.0.1 57859 10.0.0.2 22", &"g".repeat(64)] {
            assert_eq!(
                Principal::from_environment(Some(junk)),
                Principal::no_session(),
                "{junk:?}"
            );
        }
    }

    /// A peer token round-trips through the wire form, and a key this build
    /// will not repeat is refused rather than echoed.
    #[test]
    fn a_pinned_token_round_trips_and_a_bad_one_is_refused() {
        let hex = "0123456789abcdef".repeat(4);
        let origin = Origin::pinned(&hex).expect("64 hex characters");
        assert_eq!(Origin::from_token(origin.as_token()), Some(origin.clone()));
        assert_eq!(origin.pinned_key(), Some(hex.as_str()));

        assert_eq!(Origin::pinned("short"), None);
        assert_eq!(Origin::pinned(&"g".repeat(64)), None);
        // Upper case is normalised rather than refused, because an operator
        // pasting a key from a tool that shouts is not making a mistake.
        assert!(Origin::pinned(&"AB".repeat(32)).is_some());
    }

    /// **Every origin this build renders is one it reads back.** A variant that
    /// renders and does not parse is a request the writer believes it sent and
    /// the supervisor refuses — and the one it would bite is
    /// [`Origin::Console`], on a machine somebody has already had to walk over
    /// to in order to reach at all.
    #[test]
    fn every_origin_this_build_renders_is_one_it_reads_back() {
        let all = [
            Origin::pinned(A_KEY).expect("64 hex characters"),
            Origin::Console,
            Origin::NoSession,
        ];
        for origin in &all {
            assert_eq!(
                Origin::from_token(origin.as_token()).as_ref(),
                Some(origin),
                "{} renders and does not read back",
                origin.as_token()
            );
        }
    }

    /// **The reserved words are only ever themselves.** Neither has a pinned
    /// key's shape, so nothing that names a caller can claim to be one of them
    /// and neither can be read as a key.
    #[test]
    fn the_reserved_words_are_only_ever_themselves() {
        for word in [NO_SESSION_TOKEN, CONSOLE_TOKEN] {
            assert!(
                !word.starts_with(PINNED_PREFIX),
                "{word} could be read as a key"
            );
            assert_eq!(
                Origin::from_token(word).and_then(|o| o.pinned_key().map(str::to_string)),
                None
            );
        }
        assert_ne!(NO_SESSION_TOKEN, CONSOLE_TOKEN);
        assert_eq!(Origin::from_token(NO_SESSION_TOKEN), Some(Origin::NoSession));
        assert_eq!(Origin::from_token(CONSOLE_TOKEN), Some(Origin::Console));
    }

    /// **What `cmd/rivet` can honestly say about its own invocation**, every
    /// branch, without a process or an environment.
    #[test]
    fn a_principal_says_only_what_the_environment_actually_showed() {
        let hex = "a".repeat(64);
        assert_eq!(
            Principal::from_environment(Some(&hex)),
            Principal::new(Origin::pinned(&hex).expect("64 hex characters"))
        );
        // **Nothing named a caller is not an error and is not a console.**
        assert_eq!(Principal::from_environment(None), Principal::no_session());
        // An empty variable reads as unset — the repository's existing idiom.
        assert_eq!(Principal::from_environment(Some("   ")), Principal::no_session());
        // Upper case is normalised rather than refused, because an operator
        // pasting a key from a tool that shouts is not making a mistake — and
        // what lands in the record is the one lower-case spelling.
        let shouted = "AB".repeat(32);
        let quiet = "ab".repeat(32);
        let normalised = Principal::from_environment(Some(&shouted));
        assert_eq!(normalised.origin().pinned_key(), Some(quiet.as_str()));
        // **Set to something that is not a key is read as no key**, rather than
        // repeated into a record. The variable arrives through an environment
        // anything running as uid 0 can write.
        let short = "a".repeat(63);
        for odd in ["10.0.0.1", "banana", "1.2.3.4 has/a/slash", &short] {
            assert_eq!(
                *Principal::from_environment(Some(odd)).origin(),
                Origin::NoSession,
                "{odd:?}"
            );
        }
    }

    /// A principal's sentence has to end by saying what it is **not**, in every
    /// branch. `ADR-028` §4.5's whole risk is a reader promoting an observation
    /// into an authorisation.
    #[test]
    fn every_principal_says_that_it_is_attribution_and_not_authorisation() {
        let all = [
            over_the_control_plane(),
            Principal::no_session(),
            Principal::new(Origin::Console),
        ];
        for (i, a) in all.iter().enumerate() {
            let said = a.describe();
            assert!(
                said.contains("ATTRIBUTION AND NOT AUTHORISATION"),
                "{said}"
            );
            assert!(said.contains("ADR-028 §4.5"), "{said}");
            for b in all.iter().skip(i + 1) {
                assert_ne!(said, b.describe(), "{a:?} and {b:?} read alike");
            }
        }
        // The one that is easiest to over-read has to deny the console in as
        // many words.
        assert!(
            Principal::no_session()
                .describe()
                .contains("was at the console"),
            "{}",
            Principal::no_session().describe()
        );
    }

    /// **The prefix fits its own ceiling**, so widening a field without widening
    /// [`PRINCIPAL_MAX_BYTES`] fails here rather than truncating a real path on
    /// a machine.
    #[test]
    fn a_rendered_principal_always_fits_its_own_ceiling() {
        // The widest origin there is: a pinned key. Both reserved words are
        // shorter, and nothing else parses.
        let widest = Principal::new(Origin::pinned(&"f".repeat(64)).expect("64 hex characters"));
        let whole = render_request(&a_digest(), &widest).len() - 1; // less the newline
        let prefix = whole - A_DIGEST.len();
        assert!(
            u64::try_from(prefix).unwrap_or(u64::MAX) <= PRINCIPAL_MAX_BYTES,
            "the widest principal renders {prefix} bytes and the ceiling is {PRINCIPAL_MAX_BYTES}"
        );
        // ...and the widest whole request still fits the whole cap. Every
        // request is now the same length, which is what makes this an equality
        // about the format rather than an inequality about a path.
        assert!(
            u64::try_from(whole).unwrap_or(u64::MAX) <= REQUEST_MAX_BYTES,
            "the widest request is {whole} bytes and the cap is {REQUEST_MAX_BYTES}"
        );
        assert!(parse_request(&render_request(&a_digest(), &widest)).is_ok());
    }

    /// **The cap is enforced by the parser too**, not only by the `stat(2)` the
    /// supervisor does first. Two guards rather than one, because the size check
    /// and the read are separated by a window in which a file can grow.
    ///
    /// **Spelled by hand rather than rendered**, since 2026-09-12:
    /// [`render_request`] takes a checked [`crate::artifact::Digest`] and can no
    /// longer be asked to write an over-long payload. The file is what is under
    /// test and the file is written by whoever has root on `/state`, so the
    /// bytes are composed here the way an attacker would compose them.
    #[test]
    fn a_request_larger_than_the_cap_is_refused_by_the_parser_as_well() {
        let cap = usize::try_from(REQUEST_MAX_BYTES).unwrap_or(usize::MAX);
        let enormous = format!(
            "{}{} {PAYLOAD_MARKER} {}\n",
            FROM_FIELD,
            a_pinned_token(),
            "a".repeat(cap),
        );
        assert!(matches!(
            parse_request(&enormous),
            Err(RequestProblem::TooLarge { .. })
        ));
    }

    /// Every problem says something different and something useful. A refusal
    /// that only says "bad request" leaves an operator with a file they cannot
    /// see anything wrong with.
    #[test]
    fn every_problem_says_which_one_it_is() {
        let all = [
            RequestProblem::Empty,
            RequestProblem::TooLarge { bytes: 99_999 },
            RequestProblem::NotADigest {
                payload: "x".to_string(),
            },
            RequestProblem::ControlCharacter { at: 3 },
            RequestProblem::MoreThanOneLine,
            RequestProblem::NoPrincipal {
                head: "/state/x.rvt".to_string(),
            },
            RequestProblem::BadPrincipal {
                field: FROM_FIELD.to_string(),
                value: "banana".to_string(),
            },
            RequestProblem::UnknownField {
                field: "who=root".to_string(),
            },
            RequestProblem::RepeatedField {
                field: FROM_FIELD.to_string(),
            },
            RequestProblem::NoPayload,
        ];
        for (i, a) in all.iter().enumerate() {
            assert!(a.why().len() > 30, "{a:?} says nothing useful");
            assert_eq!(a.to_string(), a.why());
            for b in all.iter().skip(i + 1) {
                assert_ne!(a.why(), b.why(), "{a:?} and {b:?} read alike");
            }
        }
    }

    /// The two paths are on the persistent filesystem, are distinct, and the
    /// temporary one is in the **same directory** — a rename across filesystems
    /// is `EXDEV`, and a rename across directories is not the atomic swap this
    /// depends on.
    #[test]
    fn the_request_is_renamed_into_place_from_the_same_directory() {
        assert!(REQUEST_PATH.starts_with(&format!("{}/", crate::state::STATE_MOUNT_POINT)));
        assert!(REQUEST_TEMP_PATH.starts_with(&format!("{}/", crate::state::STATE_MOUNT_POINT)));
        assert_ne!(REQUEST_PATH, REQUEST_TEMP_PATH);
        let dir = |p: &str| p.rsplit_once('/').map(|(d, _)| d.to_string());
        assert_eq!(dir(REQUEST_PATH), dir(REQUEST_TEMP_PATH));
        // Neither is inside the journal's directory, which is rotated by a
        // component that renames and removes files it did not write.
        for path in [REQUEST_PATH, REQUEST_TEMP_PATH] {
            assert!(
                !path.starts_with(&format!("{}/", crate::state::LOG_DIR)),
                "{path} is inside the journal's directory"
            );
        }
    }

    /// **The temporary file must not itself parse as a request path the reader
    /// would find.** The reader opens one exact path, so this is a statement
    /// about that path rather than about globbing — pinned because the cheap way
    /// to name a temporary file is a suffix, and a reader that ever grew a
    /// prefix match would start consuming half-written requests.
    #[test]
    fn the_temporary_file_is_not_the_request() {
        assert!(REQUEST_TEMP_PATH.starts_with(REQUEST_PATH));
        assert_ne!(REQUEST_TEMP_PATH, REQUEST_PATH);
    }

    /// The one that is easiest to misread has to say the surprising thing:
    /// installing a system update does not reboot, and installing an
    /// application does not start it.
    #[test]
    fn the_two_installs_say_what_they_did_not_do() {
        assert!(Outcome::Installed.means().contains("has NOT rebooted"));
        assert!(Outcome::Installed.means().contains("ADR-029 §9.1"));
        assert!(
            Outcome::InstalledApplication
                .means()
                .contains("NOTHING STARTED IT")
        );
    }
}
