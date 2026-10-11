//! Every administrative operation as a typed thing, and the catalogue that
//! describes them.
//!
//! `ADR-006` §3 is what this module is:
//!
//! > * **`rivet-core` owns the operations.** Every administrative action is a
//! >   typed request producing a typed result or a typed error.
//! > * **Every interface is an adapter containing no logic.** The CLI parses
//! >   arguments and renders results. That is all it does.
//!
//! and §7 is why the catalogue is generated rather than written:
//!
//! > **An operation catalogue** enumerating every administrative operation with
//! > its inputs, outputs and error codes, likewise generated. … If they are
//! > hand-maintained documents, they will be wrong within a month and worse than
//! > absent, because they will be trusted.
//!
//! **Ruled 2026-08-22: required from Stage 1, generated from the types.**
//!
//! # How "generated from the types" is made true rather than claimed
//!
//! [`CATALOGUE`] is not a list of descriptions kept beside the operations. Each
//! entry is the `const OPERATION` declared by the module that *implements* that
//! operation, and [`perform`] dispatches on the same list. Three properties
//! follow, and each is a test rather than a sentence:
//!
//! 1. **A catalogued operation is reachable.** `every_catalogued_operation_is_
//!    reachable_and_none_of_them_answers_unimplemented` walks the catalogue and
//!    performs each entry. This is `ADR-006` §3.2's conformance property, at the
//!    core; `cmd/rivet` carries the adapter's half of it.
//! 2. **An operation that is not catalogued cannot be performed.** [`perform`]
//!    looks the verb up in [`CATALOGUE`] and answers
//!    [`ErrorCode::Unimplemented`] when it is absent, so an operation reachable
//!    but unlisted does not exist.
//! 3. **A catalogued error code is one an operation can actually produce.**
//!    Every code named by an entry is checked against [`ErrorCode::ALL`], and
//!    the ones the catalogue's own tests can provoke are provoked.
//!
//! # What is deliberately absent — amended 2026-08-23
//!
//! ~~**Anything that changes the machine.**~~ `ADR-006` §6 rules a plan/apply
//! model and §6.5 rules that a mutating operation is a *type that computes its
//! diff separately from applying it*, "so a command that cannot be dry-run does
//! not compile rather than merely being undocumented". ~~**That framework still
//! does not exist**, and none of it is stubbed here (`AGENTS.md` §3).~~
//!
//! ~~**What changed is that one mutating operation does** — `trial`, which
//! arms a one-shot trial boot. It supplies §6.2's dry-run property *by hand, for
//! itself*, and its module documentation says which of the two that is rather
//! than letting the difference be inferred. `ADR-006` §6.5's framework is still
//! the thing that would make it structural.~~
//!
//! **BUILT 2026-08-26 — [`plan`].** Seven `apply` subjects supplied the property
//! by hand and each put its plan in the human message of a refusal; all seven
//! implement [`plan::Change`] now and reach it through [`plan::resolve`]. The
//! separation is the associated type: [`plan::Change::apply`] cannot be called
//! without a `Prepared`, and the only thing that produces one is
//! [`plan::Change::plan`]. That is §6.5's *does not compile*.
//!
//! **What is still absent** is `ADR-006` §6's *desired-state* apply — `rivet
//! apply` with no subject, "make the machine match the document". Nothing here
//! is stubbed for it (`AGENTS.md` §3).
//!
//! [`Operation`] still has no field saying which entries are mutating, and the
//! original reason is unchanged: a field saying "read-only: true" would be a
//! claim about a distinction the type system does not make. **What tells the two
//! apart today is [`plan::Change`]** — a mutating operation implements it, and
//! `explain` and `get` cannot. That is the structural distinction the paragraph
//! above said §6.5's decision would have to make; it lives one level below the
//! catalogue, in the operations, rather than in [`Operation`]'s fields.
//!
//! # The first domain port, and why `perform` now takes one
//!
//! `AGENTS.md` §3.1: *"The first operation that must read something … is the one
//! that will need a domain port, and that port goes in `rivet-core`."*
//! [`machine::Machine`] is that port and `trial` is that operation.
//!
//! [`perform`] takes `&mut dyn Machine` rather than there being a second entry
//! point for the operations that need one. `ADR-006` §3.1's co-equality is the
//! reason: two entry points is two lists of operations, and an adapter would
//! eventually reach one and not the other — *"an operation that exists in only
//! one interface is a defect in that interface, not a feature of it."*
//! `explain` ignores its argument and its own tests prove it needs nothing
//! (`the_answer_does_not_depend_on_this_machine_having_the_file`).
//!
//! **A noun.** `ADR-006` §6.4 pairs a closed verb set with "a noun vocabulary
//! that matches the configuration schema exactly … the nouns *are* the schema
//! paths". So a noun is a *value* an operator supplies — an [`Input`] — and not
//! a second axis of the catalogue. `explain`'s subject is exactly that.

use crate::value::Value;

mod apply;
/// `get catalogue` — the machine's own account of what it can be asked.
/// Ruled 2026-08-30, `ADR-006` §8.13. It reads nothing and takes no port;
/// the module's own documentation carries why.
mod catalogue;
mod boot;
mod cpu;
mod device;
/// **A unified diff, computed here rather than in an adapter.** `ADR-006`
/// §8.10.6. Public because it is a *value* the plan of every content-changing
/// subject carries and `rivet get diff` returns — and because `ADR-014` §3.1
/// makes the domain the only place it can be tested with no I/O.
pub mod diff;
mod desired_state;
mod edit;
/// `ADR-006` §8.10.5's `get file`.
mod file;
/// `ADR-006` §8.10.6's `get diff`. Named for the *subject* and not for
/// [`diff`], which is the value it computes — one module per operation is this
/// directory's rule and the two are different things.
mod file_diff;
/// `apply input` — `ADR-012` §9.
pub mod input;
mod memory;
mod network;
mod process;
mod thermal;
mod explain;
/// **`ADR-006` §8.10's shared gate**, and the second module here that no
/// operation *is*. Public because `render_mode`, `describe_mode` and
/// `describe_id` are how an adapter renders what a plan already decided, and a
/// second rendering of a mode would be a second answer to *what does `0644`
/// mean* at the moment an operator is deciding.
pub mod filesystem;
/// `ADR-006` §8.10.8's `apply create-file` and `apply create-directory`.
mod create;
/// `ADR-006` §8.10.9 and §8.10.10's `apply set-mode` and `apply set-owner`.
mod permissions;
/// `ADR-006` §8.10.11's `apply rename`.
mod rename;
/// `ADR-006` §8.10.12's `apply remove-file` and `apply remove-directory`.
mod remove;
/// `apply install-secret` — the file surface's one WRITE that carries bytes and
/// an owner, 2026-09-01. Its module documentation carries why `apply edit` plus
/// two `apply set-*` is not the same operation.
mod install_secret;
/// `apply restore-secret` — one of this machine's own secrets back into
/// `/state/secrets`, root-owned. `ADR-030` §4; `install-secret`'s act with the
/// path composed under the machine's store instead of an application's.
pub mod restore_secret;
/// `apply enroll-member` — one fleet member's register record into this fleet
/// server's register, owned by `rivet-fleetd`. `ADR-059` §6; `install-secret`'s
/// act with the path composed under the fleet server's register/devices/.
mod enroll_member;
/// `apply remove-member` — the exact inverse of [`enroll_member`]: retracts
/// one fleet member's register record from this fleet server's register.
/// `ADR-059` §6; `remove-file`'s act with the path composed the SAME way
/// `enroll_member` composes it.
mod remove_member;
/// `get phones` — every phone this machine's fleet server has enrolled, as
/// raw record bytes. `ADR-061` §5 and §6.
mod phones;
/// `apply rename-phone` — rewrite one enrolled phone's label. `ADR-061` §6;
/// `enroll-member`'s act with the path composed under the fleet server's
/// register/phones/ and REQUIRING the record already exist.
mod rename_phone;
/// `apply revoke-phone` — the exact inverse of [`rename_phone`]: removes one
/// enrolled phone's record. `ADR-061` §6; `remove-member`'s act with the
/// path composed the SAME way `rename_phone` composes it.
mod revoke_phone;
/// `apply passkey-code` — mints the proxy's one-time passkey enrollment code.
/// `ADR-063` §4.
mod passkey_code;
/// `get fleet-root` — the public certificate of the root a fleet-root mode
/// fleet server minted, and its fingerprint. `ADR-063` §4.
mod fleet_root;
mod get;
mod install;
/// **Public where every other subject's module is private**, and for one
/// reason: `rivet get log --follow` is a *transport* over this operation
/// (`ADR-022` §9.5), and a transport that read `latest` or `all` with a parser
/// of its own would be a second reader of one word — `ADR-011`, in the small.
///
/// What is public is the argument vocabulary and nothing else: the subject's
/// name, its two limit words, and the two functions that read them. `OUTPUTS`
/// and `perform` stay `pub(super)`, so an adapter still cannot perform the
/// operation except through [`perform`].
pub mod log;
pub mod machine;
/// **`ADR-006` §6.5's dry-run framework**, and the only module here that no
/// operation *is*. Public because a mutating operation is defined by
/// implementing [`plan::Change`], and because the plan an adapter renders is a
/// type rather than a sentence.
pub mod plan;
mod repo_delete;
mod reload;
mod schema;
/// **`ADR-017` §6.3.2's converge-only service control**, built 2026-08-29 —
/// the second operation ever to cross the supervisor's socket, and the first
/// that changes anything. Public for [`status`]'s reason: the wire format has
/// two ends in two crates and both read the one definition here.
pub mod service_restart;
/// **`apply service-stop` and `apply service-start`** — pause one application
/// without uninstalling it (`ADR-017` §6). Public for [`service_restart`]'s
/// reason: the answer's wire format has two ends in two crates.
pub mod service_hold;
/// **`apply clock-floor-reset`** — lower the clock floor to the running clock
/// (`ADR-021` §3). Public for [`service_restart`]'s reason: the answer's wire
/// format has two ends in two crates.
pub mod clock_floor;
mod slot;
/// **`ADR-032` §3.2's query socket, and the answer it carries.** Public
/// because the wire format has two ends in two crates — `cmd/rivet-sysd`
/// renders and `cmd/rivet` parses — and both read the one definition here.
pub mod status;
mod stop;
mod storage;
mod trial;
/// `ADR-028` §5.5.6's `get trust` — which signing keys this machine accepts.
mod trust;
mod uninstall;
/// `apply uninstall-library` — `uninstall`'s twin for an installed library
/// build, over `crate::uninstall_library`'s channel.
mod uninstall_library;

/// `uninstall`'s own confirmation word, re-exported under its own name so
/// `crate::service::reserved_application_names` (`.rulings/` R-210 D21) can
/// read the grammar's own constant rather than a second copy of it. Aliased
/// apart from [`CONFIRMATION`] because the two are re-exports of two
/// different modules' constants that happen to hold the same word.
pub(crate) use uninstall::CONFIRMATION as UNINSTALL_CONFIRMATION;

pub use apply::CONFIRMATION;
/// `ADR-012` §7.3's second confirmation tier, ruled 2026-08-29 — see
/// [`apply::Hazard`].
pub use apply::{ACCEPT_FLAG, Hazard};
/// `ADR-006` §8.10's typed results. Re-exported for the reason [`Edited`] is:
/// an adapter reading one of these operations programmatically gets fields
/// rather than a document it has to index into.
pub use create::{Created, FILE_SUBJECT as CREATE_FILE, DIRECTORY_SUBJECT as CREATE_DIRECTORY};
// **Exported 2026-09-05**, when a second program started composing the call.
// `rivet-remote` had carried a literal `"desired-state"` with a comment saying
// *"rivet-core … has no entry for this one, so there is nothing to import"* —
// true when it was written. `ADR-044`'s fleet server asks the same subject, and
// a third spelling of one word that both ends have to agree on is the second
// authority `ADR-011` refuses.
pub use desired_state::SUBJECT as DESIRED_STATE;
pub use edit::{Edited, NEW_DIRECTORY_MODE, NEW_FILE_MODE, SUBJECT as EDIT, reserved_paths};
pub use file::SUBJECT as FILE;
pub use file_diff::SUBJECT as DIFF;
pub use permissions::{Changed, MODE_SUBJECT as SET_MODE, OWNER_SUBJECT as SET_OWNER};
pub use remove::{
    Removed, DIRECTORY_SUBJECT as REMOVE_DIRECTORY, FILE_SUBJECT as REMOVE_FILE,
};
pub use rename::{Renamed, SUBJECT as RENAME};
pub use explain::Explanation;
pub use install::{Requested, SUBJECT as INSTALL};
// `.rulings/` R-169: `rivet_sdk::secrets` needs the naming rule
// `apply install-secret` writes by, so an application can tell whether a
// value it wants to read back is a name that operation could have used.
pub use install_secret::{
    NAME_MAX_LEN as SECRET_NAME_MAX_LEN, SECRET_MAX_BYTES, SECRET_MODE, is_secret_name,
};
pub use log::SUBJECT as LOG;
pub use plan::{Act, Change, Effect, Plan, TakesEffect, resolve};
pub use service_restart::{
    Doing, Refusal as ServiceRestartRefusal, Restarting, SUBJECT as SERVICE_RESTART,
};
pub use service_hold::{
    Hold as ServiceHold, Kind as ServiceHoldKind, Refusal as ServiceHoldRefusal,
    START_SUBJECT as SERVICE_START, STOP_SUBJECT as SERVICE_STOP,
};
pub use status::{QUERY_SOCKET_PATH, REQUEST_MAX_BYTES, RESPONSE_MAX_BYTES, ServiceStatus, Status, StatusError, SUBJECT as STATUS};
pub use machine::{
    BlockDevice, BlockIo, Board, BootRead, CpuFrequency, Deployer, DeviceRead, DirectoryEntry,
    Editor, FileRead, FileWindow, Held, Interface, InterfaceAddress, InterfaceCounters,
    JournalRead, Machine, MachineError, MountedFilesystem, NetworkRead, Observed, OwnerIds,
    PathChange, PathKind, PathStat, Reboot, Route, SlotRead, Stop, StorageRead, ThermalRead,
    ThermalZone, Throttle, Unmeasured, UnreadableFilesystem, UnreadableInterface,
    UnreadableZone, Write, Wrote,
};
pub use stop::Stopping;
pub use storage::SUBJECT as STORAGE;
pub use trial::{Armed, SUBJECT as TRIAL_BOOT};

// ---------------------------------------------------------------------------
// The vocabulary
// ---------------------------------------------------------------------------

/// `ADR-006` §6.4's closed set of verbs, in the order the ADR lists them.
///
/// > Administrative commands are drawn from a **closed set of verbs** — `get`,
/// > `set`, `list`, `add`, `remove`, `apply`, `plan`, `export`, `explain` — and
/// > a noun vocabulary that matches the configuration schema exactly.
///
/// > The verb set is part of the interface contract (section 4.1) and is
/// > extended only by amending this ADR.
///
/// **It lived in `cmd/rivet/src/cli.rs` until 2026-08-23**, with a comment
/// saying where it belonged: *"The day an operation exists, the verb and noun
/// belong to the operation's type in the core and this list becomes a
/// projection of it."* That day is this change. A verb list inside one adapter
/// is a vocabulary a second adapter would have to copy, and `ADR-011` §3's
/// table already names a CLI that owns its own logic as the failure.
///
/// Note what this is **not**: it is not `clap`, an argument parser, or anything
/// CLI-shaped, which `ADR-014` §3's forbidden-edge table rules out of this
/// crate. It is the administrative vocabulary itself, which every adapter reads
/// and none owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verb {
    Get,
    Set,
    List,
    Add,
    Remove,
    Apply,
    Plan,
    Export,
    Explain,
}

impl Verb {
    /// The wire form. Part of the interface contract; never renamed.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Set => "set",
            Self::List => "list",
            Self::Add => "add",
            Self::Remove => "remove",
            Self::Apply => "apply",
            Self::Plan => "plan",
            Self::Export => "export",
            Self::Explain => "explain",
        }
    }

    /// Every verb, in `ADR-006` §6.4's order.
    pub const ALL: &'static [Verb] = &[
        Verb::Get,
        Verb::Set,
        Verb::List,
        Verb::Add,
        Verb::Remove,
        Verb::Apply,
        Verb::Plan,
        Verb::Export,
        Verb::Explain,
    ];

    /// Read a verb from a word, or refuse in a way an agent can act on.
    ///
    /// **The refusal is core's rather than each adapter's**, and the reason is
    /// the message: it lists the vocabulary, so the next attempt can be right,
    /// and a second adapter writing its own version of that sentence would be a
    /// second list of verbs written in prose.
    ///
    /// Case-sensitive, and deliberately: `Get` is refused rather than aliased,
    /// because `ADR-006` §6.4's value is that "a reader who knows one command
    /// can predict the others", and a surface with two spellings of one verb is
    /// one a reader has to memorise instead.
    pub fn parse(word: &str) -> Result<Self, OperationError> {
        Self::ALL
            .iter()
            .copied()
            .find(|v| v.as_str() == word)
            .ok_or_else(|| {
                OperationError::about(
                    ErrorCode::UnknownVerb,
                    word,
                    format!(
                        "not a Rivet verb. The vocabulary is closed (ADR-006 §6.4) and is: {}.",
                        Self::ALL
                            .iter()
                            .map(|v| v.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                )
            })
    }
}

impl core::fmt::Display for Verb {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Typed, coded, stable errors — ADR-006 §4.1
// ---------------------------------------------------------------------------

/// The stable machine-readable code an **operation** can answer with.
///
/// `ADR-006` §4.1:
///
/// > Every failure carries a stable machine-readable code, a structured
/// > payload, and a human string — in that order of authority. … Error codes
/// > are part of the interface contract and are not renumbered.
///
/// # Two vocabularies, and exactly one owner for each code
///
/// This enum is not the whole of Rivet's error surface. `cmd/rivet` owns
/// two codes of its own — `E_NOT_A_SHELL` and `E_NO_INTERACTIVE_SHELL` — which
/// belong to the login-shell contract and can only ever be
/// produced by the binary `/etc/shells` names. Putting them here would make
/// `rivet-core` describe how a login shell is invoked, which is the
/// `core → anything CLI-shaped` edge `ADR-014` §3 forbids.
///
/// So the split is: **an operation's failures are catalogued here; an adapter's
/// own invocation contract is catalogued by that adapter.** `ADR-011` is
/// satisfied because each code has exactly one definition — not because there is
/// exactly one enum — and `cmd/rivet`'s
/// `no_error_code_is_defined_in_two_places` asserts the two sets are disjoint.
/// `rivet --version` publishes the union, each half generated from its own
/// definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    /// A verb from `ADR-006` §6.4's closed set that this build has no operation
    /// behind. **Not an error in the operator's input** — the command was
    /// well-formed and there is nothing behind it.
    ///
    /// `ADR-006` §4.2 is what makes this a variant rather than a message: "an
    /// unimplemented operation returns a specific, catalogued `Unimplemented`
    /// variant that no caller can mistake for a result."
    Unimplemented,
    /// A word that is not one of `ADR-006` §6.4's verbs at all.
    ///
    /// Separate from [`Self::Unimplemented`] deliberately: an agent that gets
    /// `E_UNIMPLEMENTED` has constructed a valid Rivet command and should try
    /// it again on a later build; one that gets `E_UNKNOWN_VERB` has constructed
    /// something that will never be valid and should stop.
    UnknownVerb,
    /// The inputs an operation was given do not match the ones it declares.
    Arguments,
    /// The operation understood the request and has nothing to say about the
    /// thing it names.
    ///
    /// Distinct from [`Self::Arguments`] because the request was well-formed:
    /// `rivet explain /etc/motd` is a valid invocation of a real operation
    /// about a path Rivet does not generate, and an operator who has typed a
    /// path needs to be told *that* rather than that their arguments were
    /// unusable.
    UnknownSubject,
    /// The request would change the machine and was not confirmed.
    ///
    /// **Added 2026-08-23 with the first mutating operation.** `ADR-012` §7.1
    /// requires confirmation before apply and makes unattended apply *"a scope
    /// on a credential (§5), never a default and never a flag anyone can simply
    /// pass"*. Rivet has no credentials and no scopes, so there is nothing an
    /// operator can pass to skip this and nothing this code can mean except
    /// *ask again, saying yes*.
    ///
    /// ~~The message carries the plan, which is a limitation rather than a
    /// design: `ADR-006` §6.5's typed diff does not exist, so a plan is prose an
    /// agent cannot read structurally.~~ **Struck 2026-08-26.** §6.5's framework
    /// is built ([`plan`]), and this is the one code that carries a
    /// [`plan::Plan`] in `ADR-006` §4.1's structured-payload slot —
    /// [`OperationError::plan`]. The message is *generated from* that plan, so
    /// the two cannot disagree, and `OperationError::refuse` makes a refusal
    /// with this code and no plan impossible to spell.
    ConfirmationRequired,
    /// A safety interlock refused. **Nothing was changed, and retrying will not
    /// help** until what the message names has been changed first.
    ///
    /// Distinct from [`Self::Arguments`] because the request was well-formed,
    /// and from [`Self::MachineRefused`] because the machine is not at fault: it
    /// is *this request, on this machine, as it stands* that is unsafe. An agent
    /// that gets this must stop rather than retry, which is the opposite of what
    /// it should do with a transient failure — and telling those two apart
    /// without reading English is `ADR-006` §4.1's whole argument.
    Unsafe,
    /// The machine could not be read or changed, and said why.
    ///
    /// The message is what the *system* said (`AGENTS.md` §5), not what it was
    /// expected to say: an errno from a card that has gone read-only, a
    /// device-tree property that is not there. The next step is to look at
    /// hardware rather than at the request.
    MachineRefused,
}

impl ErrorCode {
    /// The wire form. **Part of the interface contract** — never renumbered,
    /// never renamed (`ADR-006` §4.1). Strings rather than integers precisely so
    /// that there is no numbering to renumber.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unimplemented => "E_UNIMPLEMENTED",
            Self::UnknownVerb => "E_UNKNOWN_VERB",
            Self::Arguments => "E_ARGUMENTS",
            Self::UnknownSubject => "E_UNKNOWN_SUBJECT",
            Self::ConfirmationRequired => "E_CONFIRMATION_REQUIRED",
            Self::Unsafe => "E_UNSAFE",
            Self::MachineRefused => "E_MACHINE_REFUSED",
        }
    }

    /// What a process exits with when this is the outermost failure.
    ///
    /// `1` is *the operation refused*; `2` is *the request was not usable*. The
    /// numbers are `cmd/rivet`'s table and they moved here with the codes so
    /// that a second adapter cannot map the same code to a different status.
    /// The three codes added 2026-08-23 are all `1`: each is an operation that
    /// **refused**, having understood a well-formed request. `2` stays what it
    /// was — the request itself was not usable.
    pub const fn exit_code(self) -> u8 {
        match self {
            Self::Unimplemented
            | Self::UnknownSubject
            | Self::ConfirmationRequired
            | Self::Unsafe
            | Self::MachineRefused => 1,
            Self::UnknownVerb | Self::Arguments => 2,
        }
    }

    /// Every code an operation can answer with, so the catalogue and a test can
    /// walk them.
    pub const ALL: &'static [ErrorCode] = &[
        ErrorCode::Unimplemented,
        ErrorCode::UnknownVerb,
        ErrorCode::Arguments,
        ErrorCode::UnknownSubject,
        ErrorCode::ConfirmationRequired,
        ErrorCode::Unsafe,
        ErrorCode::MachineRefused,
    ];
}

impl core::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One refused operation: the code, the structured payload, and the human
/// string, in `ADR-006` §4.1's order of authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationError {
    code: ErrorCode,
    /// What the caller said, carried back verbatim. `AGENTS.md` §5 wants what
    /// the operator said reported rather than paraphrased.
    subject: Option<String>,
    /// **The rest of §4.1's structured payload**, added 2026-08-26 with
    /// `ADR-006` §6.5's framework.
    ///
    /// Until then the only structured thing a refusal carried was the subject,
    /// and a [`ErrorCode::ConfirmationRequired`] put its whole plan in
    /// [`Self::message`] — so *"an agent can branch on the code and re-issue the
    /// command but cannot read the diff structurally"* (`ADR-006` §8.9). It is
    /// `Some` for exactly that code and `None` for every other, which
    /// [`plan::resolve`] makes structural: a plan is the only thing that
    /// constructs the refusal.
    /// **Boxed, and not for tidiness.** Every operation in this crate returns
    /// `Result<_, OperationError>`, so the size of this struct is the size of
    /// every one of those results — moved on every call, on the success path
    /// as well as the refusal.
    ///
    /// A [`plan::Plan`] inline made that **200 bytes**, and a plan is carried
    /// by exactly one of the seven codes ([`ErrorCode::ConfirmationRequired`])
    /// on a path that is already about to print a diff for a human. Boxing puts
    /// one allocation on that path and takes ~144 bytes off every other call in
    /// the operation surface.
    ///
    /// Found 2026-09-02 by `clippy::result_large_err`, which had been reporting
    /// it **97 times** into a container that had no clippy in it
    /// (`SPEC-04` §12.2).
    plan: Option<Box<plan::Plan>>,
    message: String,
}

impl OperationError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::refuse(code, None, message.into())
    }

    pub fn about(code: ErrorCode, subject: impl Into<String>, message: impl Into<String>) -> Self {
        Self::refuse(code, Some(subject.into()), message.into())
    }

    /// **Both public constructors, and the one place the plan-only rule is
    /// enforced.**
    ///
    /// [`ErrorCode::ConfirmationRequired`] means *here is what would happen*,
    /// and since 2026-08-26 that answer has a type — so a refusal carrying that
    /// code and no [`plan::Plan`] would be the old prose-only plan
    /// reintroduced one operation at a time. [`plan::Plan`]'s `From` is the only
    /// thing that reaches [`Self::carrying`], and this assertion fails the
    /// build's own test suite for anything else.
    ///
    /// **This is weaker than [`plan::Change`]'s associated type and is said so
    /// rather than left to be assumed** (`AGENTS.md` §3): a `debug_assert!` is
    /// compiled out of a release build, so what it buys is that the suite
    /// refuses, not that the compiler does. It is what a `pub` enum variant
    /// allows — `ErrorCode` is the interface contract and cannot have a variant
    /// nobody outside may name.
    fn refuse(code: ErrorCode, subject: Option<String>, message: String) -> Self {
        debug_assert!(
            code != ErrorCode::ConfirmationRequired,
            "a confirmation refusal is built from a plan (ADR-006 §6.5), never from a message: \
             {message}"
        );
        Self {
            code,
            subject,
            plan: None,
            message,
        }
    }

    /// Attach the plan this refusal is about. `plan::Plan`'s `From` is the only
    /// caller, so the payload cannot be attached to a code it does not belong to.
    #[must_use]
    fn carrying(mut self, plan: plan::Plan) -> Self {
        self.code = ErrorCode::ConfirmationRequired;
        self.plan = Some(Box::new(plan));
        self
    }

    /// **What this refusal would have done**, for a caller that would rather
    /// read fields than a sentence. `None` for every code but
    /// [`ErrorCode::ConfirmationRequired`].
    pub fn plan(&self) -> Option<&plan::Plan> {
        self.plan.as_deref()
    }

    pub const fn code(&self) -> ErrorCode {
        self.code
    }

    pub fn subject(&self) -> Option<&str> {
        self.subject.as_deref()
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub const fn exit_code(&self) -> u8 {
        self.code.exit_code()
    }

    /// The refusal as a [`Value`], so a failure renders through the same two
    /// readers as a result (`ADR-006` §4). An error path with its own formatting
    /// is the second source §4 forbids.
    ///
    /// The absent subject is **omitted**, never emitted as a null: a caller
    /// would then have two ways to read "there is no subject".
    pub fn value(&self) -> Value {
        let mut fields = vec![("error", Value::str(self.code.as_str()))];
        if let Some(subject) = &self.subject {
            fields.push(("subject", Value::str(subject.clone())));
        }
        // Between the subject and the message, which is `ADR-006` §4.1's order
        // of authority written into the object: an agent reads down and stops
        // at the first field it understands.
        if let Some(plan) = &self.plan {
            fields.push(("plan", plan.value()));
        }
        fields.push(("message", Value::str(self.message.clone())));
        Value::Object(fields)
    }
}

impl core::fmt::Display for OperationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match &self.subject {
            Some(subject) => write!(f, "{}: {subject}: {}", self.code, self.message),
            None => write!(f, "{}: {}", self.code, self.message),
        }
    }
}

impl std::error::Error for OperationError {}

// ---------------------------------------------------------------------------
// The catalogue
// ---------------------------------------------------------------------------

/// **The name of the input that carries a confirming word.**
///
/// One spelling, in `rivet-core`, because two things read it: `apply` declares
/// it, and [`Operation::confirms`] answers whether an operation has one so that
/// `get catalogue` can say *this changes the machine* without anybody
/// maintaining a list of which verbs do. `ADR-011` §3.
pub const CONFIRMATION_INPUT: &str = "confirmation";

/// One value an operation takes.
///
/// Positional, and the position is the declaration order. That is enough for
/// every adapter there is or is planned: a CLI has words after the verb, and a
/// future MCP or REST adapter has named inputs it binds to these names. What an
/// adapter may **not** do is invent an input, which is what makes the catalogue
/// worth publishing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Input {
    pub name: &'static str,
    /// `false` means an operation has a defined answer without it. Nothing
    /// declares an optional input yet, and the field exists because the
    /// alternative is discovering the need at the moment an adapter has to
    /// publish arity it cannot describe.
    pub required: bool,
    /// **Whether this input is one word or several.**
    ///
    /// Every input in this catalogue was exactly one word until 2026-08-24, and
    /// [`perform`]'s arity check said so structurally: a command with more words
    /// than the operation declares inputs was refused before any operation saw
    /// it. `apply edit` is the first input that is genuinely several words —
    /// `--expect <text> --replace <text>` — and the alternative to saying so was
    /// to declare four positional inputs that describe flag words, which would
    /// be a catalogue that *lies about its own shape*. `ADR-006` §7's whole
    /// argument is that a catalogue which is trusted and wrong is worse than
    /// none.
    ///
    /// **What it costs:** an operation declaring a several-word input has no
    /// arity ceiling in [`perform`], so it owns the refusal of a word it does
    /// not understand. `crate::operation::edit` refuses every unknown word by
    /// name, which is a stronger check than the count it replaces — but it is
    /// **that operation's** check rather than the dispatcher's, and that is the
    /// price. `ADR-006` §8.9 records the decision and the alternative that was
    /// not taken.
    pub several: bool,
    pub description: &'static str,
}

/// One field an operation's result carries.
///
/// Names only. A *type* per field would be a second schema — this workspace
/// already has exactly one, `rivet_config::schema`, and it describes the
/// configuration document rather than operation results. Publishing the field
/// names is what an agent needs to know what it may read; publishing a type
/// vocabulary invented for the purpose is what `ADR-006` §7 warns will be
/// trusted and wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    pub name: &'static str,
    pub description: &'static str,
}

/// **One subject of a verb, and what that subject's result carries.**
///
/// A subject is the value of an operation's first [`Input`] — `log`, `storage`,
/// `trial-boot`, `power-off`. `ADR-006` §6.4's verb set is closed and the
/// catalogue is keyed by [`Verb`], so a verb with several subjects has **one**
/// entry, and this is how that entry describes the several result shapes behind
/// it rather than flattening them into one list that describes none of them
/// exactly.
///
/// Several subjects may share one `outputs` list — `apply`'s three stop
/// subjects do — and that is the point: they produce the same shape, so they
/// name the same declaration rather than three copies of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Subject {
    pub name: &'static str,
    pub outputs: &'static [Field],
}

/// **What an operation's result carries**, and whether a subject selects it.
///
/// Before 2026-08-24 [`Operation`] had a flat `outputs` list, and a verb with
/// several subjects declared **the union by hand** — every field any subject
/// could carry, ordered so that each subject's own list was a subsequence of it.
/// That hand-maintained list was the wart: `AGENTS.md` §7's staleness in its
/// purest form, guarded by a drift test in every module that fed it.
///
/// It is now generated. [`Operation::outputs`] merges the subjects' lists, and
/// the drift test is gone because there is nothing left to drift.
#[derive(Debug, Clone, Copy)]
pub enum Results {
    /// One shape, whatever the arguments — `explain`.
    Same(&'static [Field]),
    /// **The first input is a subject and its value selects the shape.**
    ///
    /// A function rather than a `&'static [Subject]` because the table is itself
    /// generated: `apply`'s three stop subjects come from `Stop::ALL`, which is
    /// already the one place those words are written. A const table would be a
    /// second place, which is the thing this whole change removes.
    PerSubject(fn() -> Vec<Subject>),
}

/// Equality by **what the subjects are**, never by where the function that
/// produces them happens to live.
///
/// Hand-written rather than derived, because a derived `PartialEq` compares
/// function *pointers* — which the compiler warns about for the good reason that
/// two identical functions may be merged to one address and one function may
/// have two. Comparing the tables they produce is the only comparison that
/// means anything, and it is what a caller asking whether two operations are the
/// same operation is asking.
impl PartialEq for Results {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Same(ours), Self::Same(theirs)) => ours == theirs,
            (Self::PerSubject(ours), Self::PerSubject(theirs)) => ours() == theirs(),
            _ => false,
        }
    }
}

impl Eq for Results {}

/// One field of an operation's **published** result shape.
///
/// Distinct from [`Field`] in one way that matters: its description is owned,
/// because for a verb with several subjects it is *assembled* — see
/// [`Operation::outputs`]. A subject's own declarations stay [`Field`]s and stay
/// `const`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub name: &'static str,
    pub description: String,
}

/// One administrative operation, described.
///
/// **Declared by the module that implements it**, never beside it — see the
/// module documentation's three properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Operation {
    pub verb: Verb,
    pub summary: &'static str,
    pub inputs: &'static [Input],
    /// What the result carries — one shape, or one per subject.
    pub results: Results,
    /// Every way this operation refuses. [`ErrorCode::Unimplemented`] is never
    /// among them: an operation that is in this catalogue is implemented, and
    /// listing the code for not being implemented would say the opposite.
    pub errors: &'static [ErrorCode],
}

impl Operation {
    /// **Every subject this operation accepts**, in the order the catalogue
    /// names them. Empty for a verb whose result shape does not depend on one.
    pub fn subjects(&self) -> Vec<Subject> {
        match self.results {
            Results::Same(_) => Vec::new(),
            Results::PerSubject(table) => table(),
        }
    }

    /// **The published result shape, generated.**
    ///
    /// For [`Results::Same`] it is the declared list, unchanged. For
    /// [`Results::PerSubject`] it is the **union** of the subjects' lists,
    /// assembled twice over:
    ///
    /// **The order** is a merge, not a concatenation. Each subject's list is
    /// walked against what has been placed so far; a field already there
    /// advances the cursor, a new one is inserted *at* the cursor. What comes
    /// out contains every subject's list as a subsequence, which is the property
    /// the hand-written union was maintained to have and the reason it kept
    /// needing a drift test.
    ///
    /// **Each description** comes from the subjects that carry the field:
    /// carried by all of them in the same words, those words; otherwise each
    /// distinct description prefixed by the subjects that give it. So a field
    /// only one subject carries names that subject, which is how the flat list
    /// says *`slot` belongs to `trial-boot`* without anyone writing it down.
    pub fn outputs(&self) -> Vec<Output> {
        let subjects = self.subjects();
        if let Results::Same(fields) = self.results {
            return fields
                .iter()
                .map(|f| Output {
                    name: f.name,
                    description: f.description.to_owned(),
                })
                .collect();
        }

        let mut order: Vec<&'static str> = Vec::new();
        for subject in &subjects {
            let mut at = 0usize;
            for field in subject.outputs {
                match order.iter().skip(at).position(|name| *name == field.name) {
                    Some(found) => at += found + 1,
                    // Placed already, but EARLIER than this subject wants it:
                    // two subjects order the same pair of fields differently and
                    // no single sequence holds both. Left alone rather than
                    // duplicated — a repeated name would be a worse answer than
                    // a wrong order — and refused by
                    // `every_subjects_fields_survive_the_generated_union`.
                    None if order.contains(&field.name) => {}
                    None => {
                        order.insert(at, field.name);
                        at += 1;
                    }
                }
            }
        }

        order
            .into_iter()
            .map(|name| Output {
                name,
                description: describe(name, &subjects),
            })
            .collect()
    }

    /// The catalogue entry, as the structured value an adapter publishes.
    pub fn value(&self) -> Value {
        Value::object(vec![
            ("verb", Value::str(self.verb.as_str())),
            ("summary", Value::str(self.summary)),
            (
                "inputs",
                Value::List(
                    self.inputs
                        .iter()
                        .map(|i| {
                            Value::object(vec![
                                ("name", Value::str(i.name)),
                                ("required", Value::Bool(i.required)),
                                // Published rather than left for an adapter to
                                // discover by being refused: an agent binding
                                // this input has to know whether it supplies one
                                // word or a phrase.
                                ("several", Value::Bool(i.several)),
                                ("description", Value::str(i.description)),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "outputs",
                Value::List(
                    self.outputs()
                        .into_iter()
                        .map(|o| {
                            Value::object(vec![
                                ("name", Value::str(o.name)),
                                ("description", Value::str(o.description)),
                            ])
                        })
                        .collect(),
                ),
            ),
            // The per-subject breakdown, and the authority: `outputs` above is
            // generated from THIS. Empty for a verb whose result shape does not
            // depend on a subject, which says so uniformly rather than by the
            // key being absent.
            (
                "subjects",
                Value::List(
                    self.subjects()
                        .into_iter()
                        .map(|s| {
                            Value::object(vec![
                                ("name", Value::str(s.name)),
                                (
                                    "outputs",
                                    Value::List(
                                        s.outputs
                                            .iter()
                                            .map(|o| {
                                                Value::object(vec![
                                                    ("name", Value::str(o.name)),
                                                    ("description", Value::str(o.description)),
                                                ])
                                            })
                                            .collect(),
                                    ),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "errors",
                Value::strings(self.errors.iter().map(|e| e.as_str())),
            ),
        ])
    }

    /// **Whether this operation changes the machine and confirms first.**
    ///
    /// Derived from the declared inputs rather than from a list of verbs:
    /// an operation confirms exactly when it declares an input named
    /// [`CONFIRMATION_INPUT`], which is the same declaration `perform` binds
    /// the operator's last word to. `ADR-012` §7.1 makes confirmation per
    /// *change*, so a verb that grows one cannot be described anywhere as a
    /// verb that does not.
    #[must_use]
    pub fn confirms(&self) -> bool {
        self.inputs.iter().any(|i| i.name == CONFIRMATION_INPUT)
    }

    /// How many inputs must be supplied.
    fn required_inputs(&self) -> usize {
        self.inputs.iter().filter(|i| i.required).count()
    }

    /// The most words this operation will accept, or `None` when it declares a
    /// several-word input and therefore has no ceiling.
    ///
    /// See [`Input::several`] for what that costs and who then owns the
    /// refusal.
    fn word_ceiling(&self) -> Option<usize> {
        if self.inputs.iter().any(|i| i.several) {
            return None;
        }
        Some(self.inputs.len())
    }
}

/// The published description of one field of a multi-subject result. See
/// [`Operation::outputs`] for the rule; this is it in code.
fn describe(name: &'static str, subjects: &[Subject]) -> String {
    // (description, the subjects that give it), in first-appearance order.
    let mut groups: Vec<(&'static str, Vec<&'static str>)> = Vec::new();
    for subject in subjects {
        let Some(field) = subject.outputs.iter().find(|f| f.name == name) else {
            continue;
        };
        match groups
            .iter_mut()
            .find(|(text, _)| *text == field.description)
        {
            Some((_, who)) => who.push(subject.name),
            None => groups.push((field.description, vec![subject.name])),
        }
    }

    let carriers: usize = groups.iter().map(|(_, who)| who.len()).sum();
    if groups.len() == 1 && carriers == subjects.len() {
        return groups[0].0.to_owned();
    }

    groups
        .iter()
        .map(|(text, who)| {
            let who: Vec<String> = who.iter().map(|w| format!("`{w}`")).collect();
            format!("{}: {text}", who.join(", "))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// **Every administrative operation this build implements.**
///
/// ~~One entry today.~~ **Two, since 2026-08-23.** `ADR-006` §8.1 recorded the
/// empty case honestly — *"all nine verbs answer `E_UNIMPLEMENTED`"* — `explain`
/// was the first thing to change it and `apply` is the second.
///
/// ~~**Two entries and four operations, since later the same day.**~~ **Three
/// entries and seven operations, 2026-08-24** — struck rather than edited
/// because `AGENTS.md` §7 is about exactly this sentence, and because the number
/// has now moved twice. `apply` has four subjects — `trial`'s arming and
/// `stop`'s three ways to stop the machine — and `get` has two, [`log`]'s
/// journal and `storage`'s filesystems, each set behind **one entry** because
/// `ADR-006` §6.4's verb set is closed and the catalogue is keyed by verb.
/// `apply`'s and `get`'s module documentation each carry what that costs and
/// whose call it is to overturn. The six verbs with no entry still answer
/// [`ErrorCode::Unimplemented`], which remains the honest answer for them and is
/// produced by looking in this list rather than by a branch that says so.
///
/// **Do not read the count out of this comment — count [`CATALOGUE`].** It went
/// stale within a day, twice.
///
/// In `ADR-006` §6.4's verb order, which is the order an operator reads them in
/// `rivet --version`.
pub const CATALOGUE: &[Operation] = &[apply::OPERATION, explain::OPERATION, get::OPERATION];

/// The catalogue as the value `rivet --version` publishes.
///
/// Generated from [`CATALOGUE`], which is generated from the operations. There
/// is no third place the list of operations is written down, which is the whole
/// of `ADR-006` §7's requirement.
pub fn catalogue() -> Value {
    Value::List(CATALOGUE.iter().map(Operation::value).collect())
}

/// The entry for a verb, if this build implements one.
pub fn operation(verb: Verb) -> Option<&'static Operation> {
    CATALOGUE.iter().find(|op| op.verb == verb)
}

/// **Perform an operation.** The one entry point every adapter uses.
///
/// `ADR-006` §3.1's co-equality is the reason this takes a verb and a list of
/// values rather than being a method on a CLI type: every adapter binds its own
/// input form to [`Operation::inputs`] and calls this, so an operation cannot be
/// reachable from one interface and not another.
///
/// Arity is checked here rather than by each caller — an adapter that got it
/// wrong would produce a refusal the catalogue does not describe.
///
/// **`machine` is the domain port of [`machine::Machine`]**, supplied by the
/// composition root. Every adapter passes one, including for operations that
/// ignore it, so that there is exactly one entry point and therefore exactly one
/// list of operations — see this module's documentation.
pub fn perform(
    verb: Verb,
    arguments: &[&str],
    board: &mut dyn Board,
) -> Result<Value, OperationError> {
    let Some(op) = operation(verb) else {
        return Err(OperationError::about(
            ErrorCode::Unimplemented,
            verb.as_str(),
            format!(
                "`{verb}` is a Rivet verb (ADR-006 §6.4) and this build implements no \
                 operation behind it. `rivet --version` publishes the operation catalogue, \
                 which is generated from the types and is the authority for what exists."
            ),
        ));
    };

    let required = op.required_inputs();
    let too_many = op.word_ceiling().is_some_and(|most| arguments.len() > most);
    if arguments.len() < required || too_many {
        let expected: Vec<&str> = op.inputs.iter().map(|i| i.name).collect();
        return Err(OperationError::about(
            ErrorCode::Arguments,
            verb.as_str(),
            format!(
                "`{verb}` takes {}, and was given {}. `rivet --version` publishes what each \
                 operation takes.",
                if expected.is_empty() {
                    "no arguments".to_owned()
                } else {
                    format!("{} ({})", expected.len(), expected.join(", "))
                },
                arguments.len()
            ),
        ));
    }

    match verb {
        Verb::Explain => explain::perform(arguments).map(|e| e.value()),
        // The one verb that needs the world — four subjects behind it, and
        // `apply` dispatches. `explain` is handed the same port and ignores it,
        // which is the difference §6.5's framework would eventually make
        // structural.
        Verb::Apply => apply::perform(arguments, board),
        // The second verb that needs the world, and the only one that only
        // **reads** it — `ADR-022` §9's journal query and `ADR-005` §4.1.3's
        // storage facts, behind one entry. It is `get log` rather than `log`
        // because §6.4's verb set is closed and adding a tenth is the
        // maintainer's; `get`'s own module argues it and the maintainer's
        // ruling settled it: `get log` stays.
        Verb::Get => get::perform(arguments, board),
        // Every other verb is absent from `CATALOGUE`, so `operation` above
        // already returned `Unimplemented` and this arm is unreachable *through
        // the catalogue*. It is written as the same refusal rather than as an
        // `unreachable!` because a panic in `rivet-core` would be a panic in
        // PID 1's dependency tree, and because if the two ever disagree the
        // honest answer is still "this build implements no operation behind it".
        other => Err(OperationError::about(
            ErrorCode::Unimplemented,
            other.as_str(),
            format!("`{other}` is catalogued and has no implementation; that is a defect."),
        )),
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::state::Slot;

    /// A machine that refuses everything, for the tests in *this* module.
    ///
    /// Every test here is about the catalogue — arity, reachability, which verb
    /// has an entry — and each of them is answered before any operation reaches
    /// the port. So the honest stand-in is one that refuses rather than one that
    /// pretends (`AGENTS.md` §3), and a test that started depending on it would
    /// fail loudly rather than pass on a fiction. `operation::trial`'s own tests
    /// carry the machine that answers.
    pub(super) struct NoMachine;

    /// A board that refuses everything, for a sibling module's tests.
    ///
    /// `super::apply`'s gate test needs a `Board` and needs nothing FROM it: the
    /// subject is refused before any port is touched, which is the property
    /// being asserted. A fake that refuses is the honest one (`AGENTS.md` §3) —
    /// if that test ever reaches a port it fails loudly instead of passing.
    pub(super) fn no_machine() -> Box<dyn Board> {
        Box::new(NoMachine)
    }

    impl NoMachine {
        fn refusal() -> MachineError {
            MachineError::Refused(
                "these tests supply no machine; an operation that needed one has changed"
                    .to_string(),
            )
        }
    }

    impl Machine for NoMachine {
        // Added 2026-08-26 with `ADR-032` §3.2's reload. Refuses like every
        // other method here: these tests are about the catalogue, and one that
        // started reloading would be testing something else.
        fn can_request_reload(&mut self) -> Result<(), MachineError> {
            Err(Self::refusal())
        }
        fn request_reload(&mut self) -> Result<(), MachineError> {
            Err(Self::refusal())
        }

        // Added 2026-08-29 with `ADR-017` §6.3.2's service control, and for the
        // same reason as the two above.
        fn can_request_service_restart(
            &mut self,
            _service: &str,
        ) -> Result<Restarting, MachineError> {
            Err(Self::refusal())
        }
        fn request_service_restart(
            &mut self,
            _service: &str,
        ) -> Result<Restarting, MachineError> {
            Err(Self::refusal())
        }

        fn running_slot(&mut self) -> Result<Slot, MachineError> {
            Err(Self::refusal())
        }

        fn can_arm_trial_boot(&mut self, _slot: Slot) -> Result<(), MachineError> {
            Err(Self::refusal())
        }

        fn arm_trial_boot(&mut self, _slot: Slot) -> Result<Reboot, MachineError> {
            Err(Self::refusal())
        }

        fn can_request_stop(&mut self, _stop: Stop) -> Result<(), MachineError> {
            Err(Self::refusal())
        }

        fn request_stop(&mut self, _stop: Stop) -> Result<(), MachineError> {
            Err(Self::refusal())
        }
    }

    /// These tests drive `perform`, which reaches both ports, so this fake
    /// answers on both — and refuses on both. It is the one place a single type
    /// still implements everything, because the dispatcher is the one caller
    /// that legitimately needs everything.
    impl Observed for NoMachine {
        /// **Refuses rather than answering that this machine trusts nobody.**
        /// An empty trust store is a REAL reading -- it is what `ADR-028` §5.5.5's
        /// `--no-trust` image produces -- and this fake has not taken it. The
        /// direction is the dangerous one: *"trusts nobody"* is the answer
        /// somebody acts on by re-signing an artifact.
        fn read_trust(&mut self) -> Result<crate::operation::machine::TrustRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering an idle machine.** Zeroed counters
        /// are what a machine that has run nothing since it booted looks like,
        /// and a load average of zero is what an idle one looks like — both are
        /// readings, and this fake has taken neither (`AGENTS.md` §3, §5).
        fn read_cpu(&mut self) -> Result<crate::operation::machine::CpuRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering that this machine has no block
        /// devices**, which is not a state a machine that booted off one can
        /// be in.
        fn read_devices(&mut self) -> Result<crate::operation::machine::DeviceRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering an empty network.** No interfaces at
        /// all reads as a machine with none, which is not a state a running
        /// Linux is in.
        fn read_network(&mut self) -> Result<crate::operation::machine::NetworkRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than naming a slot.** Naming the wrong one is how
        /// an update overwrites the running system, so a fake that observes
        /// nothing names none.
        fn read_slots(&mut self) -> Result<crate::operation::machine::SlotRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering an uptime of zero.** `0` would say
        /// this machine booted a moment ago, which is a reading nobody took.
        fn read_boot(&mut self) -> Result<crate::operation::machine::BootRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering with unset flags.** Reporting a
        /// `Throttle` of all-`false` would assert that this machine has never
        /// been under-volted since it booted — the one claim this subject must
        /// never make without a measurement (`AGENTS.md` §3, §5).
        fn read_thermal(
            &mut self,
        ) -> Result<crate::operation::machine::ThermalRead, MachineError> {
            Err(MachineError::Refused(
                    "not this fake's port; unread throttle flags are never reported as false"
                        .to_string(),
            ))
        }
    fn read_processes(
        &mut self,
    ) -> Result<crate::operation::machine::ProcessRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_memory(&mut self) -> Result<crate::operation::machine::MemoryRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_desired_state(&mut self) -> Result<Option<String>, MachineError> {
        Ok(None)
    }
        /// `ADR-032` §3.2's query socket, added 2026-08-26. Refuses like every
        /// other method here: these tests are about the catalogue.
        fn supervisor_status(
            &mut self,
        ) -> Result<crate::operation::status::Status, MachineError> {
            Err(Self::refusal())
        }

        fn read_journal(&mut self) -> Result<JournalRead, MachineError> {
            Err(Self::refusal())
        }

        /// **The one method here that cannot refuse**, which is the port's own
        /// point: everything else on `Observed` reads a running machine and can
        /// be told no; this is a value the build computes. A fake that refused
        /// it would be modelling a state that has no way to occur.
        fn configuration_schema(&mut self) -> Value {
            Value::str("<no schema: this is a test fake>")
        }

        fn read_filesystems(&mut self) -> Result<machine::StorageRead, MachineError> {
            Err(Self::refusal())
        }
    }

    /// And on the fourth, later on 2026-08-24. Same posture throughout: it
    /// refuses, so a test in this module that started depending on an edit would
    /// fail loudly rather than pass on a fiction (`AGENTS.md` §3).
    impl machine::Editor for NoMachine {
        fn principal(&mut self) -> crate::deploy::Principal {
            crate::deploy::Principal::no_session()
        }

        fn read_file(&mut self, _path: &str) -> Result<machine::FileRead, MachineError> {
            Err(Self::refusal())
        }

        fn can_write_file(&mut self, _path: &str, _contents: &str) -> Result<(), MachineError> {
            Err(Self::refusal())
        }

        fn write_file(
            &mut self,
            _write: &machine::Write<'_>,
        ) -> Result<machine::Wrote, MachineError> {
            Err(Self::refusal())
        }

        // `ADR-006` §8.10's four, added 2026-08-29. Same posture as the three
        // above: these tests are about the catalogue and every one of them is
        // answered before any port is touched, so a test here that started
        // removing a file fails loudly rather than passing on a fiction.
        fn stat_path(&mut self, _path: &str) -> Result<machine::PathStat, MachineError> {
            Err(Self::refusal())
        }

        fn resolve_owner(
            &mut self,
            _user: Option<&str>,
            _group: Option<&str>,
        ) -> Result<machine::OwnerIds, MachineError> {
            Err(Self::refusal())
        }

        fn can_change_path(
            &mut self,
            _change: &machine::PathChange<'_>,
        ) -> Result<(), MachineError> {
            Err(Self::refusal())
        }

        fn change_path(
            &mut self,
            _change: &machine::PathChange<'_>,
        ) -> Result<machine::Wrote, MachineError> {
            Err(Self::refusal())
        }
    }

    /// And on the third, since 2026-08-24. Same posture: it refuses, so a test
    /// in this module that started depending on an install would fail loudly
    /// rather than pass on a fiction (`AGENTS.md` §3).
    impl Deployer for NoMachine {
        /// **`crate::repo_delete`'s two, added 2026-08-26.** This fake destroys
        /// nothing and refusing is the honest stand-in (`AGENTS.md` §3): a test
        /// in this module that reached this port has changed what it is about.
        fn can_request_repo_delete(
            &mut self,
            _name: &str,
        ) -> Result<(), crate::operation::machine::MachineError> {
            Err(crate::operation::machine::MachineError::Refused(
                "this fake destroys no repository; an operation that reached this port has \
                 changed"
                    .to_string(),
            ))
        }
        fn request_repo_delete(
            &mut self,
            _name: &str,
            _id: &str,
        ) -> Result<(), crate::operation::machine::MachineError> {
            self.can_request_repo_delete(_name)
        }

        fn can_request_install(&mut self, _digest: &crate::artifact::Digest) -> Result<crate::operation::machine::Held, MachineError> {
            Err(Self::refusal())
        }

        fn request_install(&mut self, _digest: &crate::artifact::Digest) -> Result<crate::operation::machine::Held, MachineError> {
            Err(Self::refusal())
        }

        fn can_request_uninstall(&mut self, _name: &str) -> Result<(), MachineError> {
            Err(Self::refusal())
        }

        fn request_uninstall(&mut self, _name: &str, _purge: bool) -> Result<(), MachineError> {
            Err(Self::refusal())
        }

        fn can_request_library_uninstall(
            &mut self,
            _selector: &crate::uninstall_library::Selector,
        ) -> Result<crate::uninstall_library::Removal, MachineError> {
            Err(Self::refusal())
        }

        fn request_library_uninstall(
            &mut self,
            _selector: &crate::uninstall_library::Selector,
        ) -> Result<crate::uninstall_library::Removal, MachineError> {
            Err(Self::refusal())
        }
    }

    fn attempt(verb: Verb, arguments: &[&str]) -> Result<Value, OperationError> {
        perform(verb, arguments, &mut NoMachine)
    }

    /// The fields of an object value, or a failure naming what it was instead.
    fn fields(value: &Value) -> &[(&'static str, Value)] {
        match value {
            Value::Object(fields) => fields,
            other => panic!("expected an object, got {other:?}"),
        }
    }

    // -- the vocabulary --------------------------------------------------------

    /// **The contract, written out**, because `ADR-006` §6.4 makes the verb set
    /// part of the interface and extensible only by amending the ADR. A test
    /// that counted them would let one be swapped for another.
    #[test]
    fn the_verb_set_is_adr_006_6_4s_closed_list_in_its_order() {
        let verbs: Vec<&str> = Verb::ALL.iter().map(|v| v.as_str()).collect();
        assert_eq!(
            verbs,
            [
                "get", "set", "list", "add", "remove", "apply", "plan", "export", "explain"
            ]
        );
    }

    #[test]
    fn every_verb_parses_back_to_itself_and_none_of_them_looks_like_a_flag() {
        for verb in Verb::ALL {
            assert_eq!(Verb::parse(verb.as_str()), Ok(*verb));
            assert!(!verb.as_str().starts_with('-'), "{verb} looks like a flag");
        }
    }

    /// **Near-misses matter more than nonsense.** `status`, `show` and `info`
    /// are what a person types from habit, and `ADR-006` §6.4 names exactly that
    /// habit as the thing the closed vocabulary is for.
    #[test]
    fn the_verbs_other_tools_use_are_refused_rather_than_quietly_aliased() {
        for word in ["status", "show", "info", "ls", "GET", "Get", "", "schema"] {
            let refusal = Verb::parse(word)
                .expect_err(&format!("{word:?} is not in ADR-006 §6.4's closed set"));
            assert_eq!(refusal.code(), ErrorCode::UnknownVerb, "{word:?}");
            assert_eq!(refusal.subject(), Some(word));
            // And the message lists the vocabulary, so the next attempt can be
            // right without reading the source.
            for verb in Verb::ALL {
                assert!(
                    refusal.message().contains(verb.as_str()),
                    "{verb} missing from {}",
                    refusal.message()
                );
            }
        }
    }

    // -- the error codes -------------------------------------------------------

    /// **The contract, written out.** `ADR-006` §4.1 says error codes are part
    /// of the interface and are not renumbered; this is what makes changing one
    /// a decision rather than a rename refactor that compiles.
    #[test]
    fn the_operation_error_codes_are_the_contract_and_this_is_the_list() {
        let codes: Vec<&str> = ErrorCode::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(
            codes,
            [
                "E_UNIMPLEMENTED",
                "E_UNKNOWN_VERB",
                "E_ARGUMENTS",
                "E_UNKNOWN_SUBJECT",
                // Added 2026-08-23 with the first mutating operation. Appended
                // rather than inserted: §4.1 says codes are not renumbered, and
                // although these are strings and have no numbers, the *list* is
                // what an operator reads in `rivet --version` and reordering
                // it would make a diff that looks like a rename.
                "E_CONFIRMATION_REQUIRED",
                "E_UNSAFE",
                "E_MACHINE_REFUSED",
            ]
        );
    }

    #[test]
    fn every_code_starts_with_e_underscore_and_is_distinct() {
        let mut seen: Vec<&str> = Vec::new();
        for code in ErrorCode::ALL {
            let s = code.as_str();
            assert!(s.starts_with("E_"), "{s} does not look like a code");
            assert!(!seen.contains(&s), "{s} appears twice");
            seen.push(s);
        }
    }

    #[test]
    fn no_refusal_exits_zero_because_a_refusal_is_not_a_result() {
        for code in ErrorCode::ALL {
            assert_ne!(code.exit_code(), 0, "{code} exits 0");
        }
    }

    /// An unimplemented operation and an unknown word must not be the same
    /// answer: one is "not yet", the other is "never".
    #[test]
    fn not_yet_and_never_are_different_codes_and_different_statuses() {
        assert_ne!(
            ErrorCode::Unimplemented.as_str(),
            ErrorCode::UnknownVerb.as_str()
        );
        assert_ne!(
            ErrorCode::Unimplemented.exit_code(),
            ErrorCode::UnknownVerb.exit_code()
        );
    }

    #[test]
    fn a_refusal_with_no_subject_omits_the_field_rather_than_carrying_an_empty_one() {
        let plain = OperationError::new(ErrorCode::Arguments, "no");
        assert_eq!(plain.subject(), None);
        let rendered = plain.value();
        let fields = fields(&rendered);
        assert!(!fields.iter().any(|(k, _)| *k == "subject"), "{fields:?}");
    }

    // -- the catalogue ---------------------------------------------------------

    /// **`ADR-006` §3.2's conformance property, at the core.** An operation in
    /// the catalogue that cannot be performed is a catalogue that lies, and a
    /// catalogue that lies is worse than an empty one — §7's own argument.
    #[test]
    fn every_catalogued_operation_is_reachable_and_none_answers_unimplemented() {
        for op in CATALOGUE {
            // Every declared input, supplied as a value the operation is
            // guaranteed to accept: its own name is not one, so a refusal here
            // must not be `Unimplemented` and must not be `Arguments`.
            let arguments: Vec<&str> = op.inputs.iter().map(|_| "/etc/passwd").collect();
            let got = attempt(op.verb, &arguments);
            if let Err(e) = &got {
                assert_ne!(
                    e.code(),
                    ErrorCode::Unimplemented,
                    "{} is catalogued and answers E_UNIMPLEMENTED",
                    op.verb
                );
                assert_ne!(
                    e.code(),
                    ErrorCode::Arguments,
                    "{} refused the arity its own catalogue entry declares",
                    op.verb
                );
            }
        }
    }

    /// The other direction: a verb with no entry is answered from the catalogue
    /// rather than from a branch that happens to agree with it.
    #[test]
    fn every_verb_with_no_catalogue_entry_answers_unimplemented() {
        for verb in Verb::ALL {
            if operation(*verb).is_some() {
                continue;
            }
            let refusal = attempt(*verb, &[]).expect_err(&format!(
                "{verb} produced a value with no operation behind it"
            ));
            assert_eq!(refusal.code(), ErrorCode::Unimplemented, "{verb}");
            assert_eq!(refusal.subject(), Some(verb.as_str()));
        }
    }

    /// `ADR-006` §8.1 recorded `"operations":[]` as "the true count and the one
    /// thing about it that cannot be wrong". It is no longer the true count, and
    /// this is what stops the catalogue silently emptying again.
    #[test]
    fn the_catalogue_is_not_empty_and_every_entry_describes_itself() {
        assert!(
            !CATALOGUE.is_empty(),
            "ADR-006 §7 is unmet with an empty catalogue"
        );
        for op in CATALOGUE {
            assert!(!op.summary.is_empty(), "{} has no summary", op.verb);
            assert!(!op.outputs().is_empty(), "{} declares no output", op.verb);
            for input in op.inputs {
                assert!(!input.name.is_empty(), "{}: an input has no name", op.verb);
                assert!(
                    !input.description.is_empty(),
                    "{}: input {} has no description",
                    op.verb,
                    input.name
                );
            }
            for output in op.outputs() {
                assert!(
                    !output.description.is_empty(),
                    "{}: output {} has no description",
                    op.verb,
                    output.name
                );
            }
        }
    }

    // -- the generated union ---------------------------------------------------

    /// **The union is generated, and this is what makes it a union.**
    ///
    /// Until 2026-08-24 each multi-subject verb wrote its union by hand and each
    /// carried its own drift test. [`Operation::outputs`] now merges the subject
    /// table, so the three properties those tests checked per verb are checked
    /// here once for the whole catalogue:
    ///
    /// 1. every subject's own order survives inside the union — a *subsequence*,
    ///    never a reordering, because each subject emits its fields in its own
    ///    declared order and an agent reading the flat list must not be told a
    ///    different one;
    /// 2. nothing is published that no subject carries — `ADR-006` §7's "trusted
    ///    and wrong" in the smallest available form;
    /// 3. no name appears twice, which the merge can only produce if two
    ///    subjects order the same pair of fields differently.
    #[test]
    fn every_subjects_fields_survive_the_generated_union() {
        for op in CATALOGUE {
            let subjects = op.subjects();
            if subjects.is_empty() {
                continue;
            }
            let published: Vec<&str> = op.outputs().iter().map(|o| o.name).collect();

            for (position, name) in published.iter().enumerate() {
                assert!(
                    !published[..position].contains(name),
                    "{}: `{name}` is published twice — two subjects order it \
                     differently and no single sequence holds both: {published:?}",
                    op.verb
                );
                assert!(
                    subjects
                        .iter()
                        .any(|s| s.outputs.iter().any(|f| f.name == *name)),
                    "{}: `{name}` is published and no subject carries it",
                    op.verb
                );
            }

            for subject in &subjects {
                let mut at = 0usize;
                for field in subject.outputs {
                    let found = published
                        .iter()
                        .skip(at)
                        .position(|name| *name == field.name)
                        .unwrap_or_else(|| {
                            panic!(
                                "{} {}: `{}` is not in the union after position {at}: \
                                 {published:?}",
                                op.verb, subject.name, field.name
                            )
                        });
                    at += found + 1;
                }
            }
        }
    }

    /// **A generated description says who carries the field**, which is the one
    /// thing the flat list cannot otherwise tell you and the one thing the
    /// hand-written union spent most of its words on.
    #[test]
    fn a_field_only_some_subjects_carry_names_them_and_one_they_all_share_does_not() {
        for op in CATALOGUE {
            let subjects = op.subjects();
            if subjects.is_empty() {
                continue;
            }
            for output in op.outputs() {
                let carriers: Vec<&str> = subjects
                    .iter()
                    .filter(|s| s.outputs.iter().any(|f| f.name == output.name))
                    .map(|s| s.name)
                    .collect();
                assert!(
                    !output.description.is_empty(),
                    "{}: `{}` has no description",
                    op.verb,
                    output.name
                );
                if carriers.len() < subjects.len() {
                    for carrier in &carriers {
                        assert!(
                            output.description.contains(&format!("`{carrier}`")),
                            "{}: `{}` is carried by {carriers:?} and the published \
                             description names none of them: {}",
                            op.verb,
                            output.name,
                            output.description
                        );
                    }
                }
            }
        }
    }

    /// A verb with no subject publishes its declared list unchanged, and an
    /// empty `subjects` — which says *the shape does not depend on one* rather
    /// than leaving a reader to infer it from a missing key.
    #[test]
    fn a_verb_without_subjects_publishes_what_it_declared_and_an_empty_subject_list() {
        let explain = operation(Verb::Explain).expect("explain is catalogued");
        assert!(explain.subjects().is_empty());
        let Results::Same(declared) = explain.results else {
            panic!("explain's result shape does not depend on a subject");
        };
        let published = explain.outputs();
        assert_eq!(published.len(), declared.len());
        for (out, field) in published.iter().zip(declared) {
            assert_eq!(out.name, field.name);
            assert_eq!(out.description, field.description);
        }
    }

    /// A verb may appear once. Two entries for one verb would make
    /// [`operation`]'s answer depend on declaration order, which is the kind of
    /// second authority `ADR-011` §3 is about.
    #[test]
    fn no_verb_is_catalogued_twice() {
        let mut seen: Vec<Verb> = Vec::new();
        for op in CATALOGUE {
            assert!(!seen.contains(&op.verb), "{} is catalogued twice", op.verb);
            seen.push(op.verb);
        }
    }

    /// Every code an entry claims to produce must be a real code, and must not
    /// be `E_UNIMPLEMENTED` — which would say the operation is not implemented
    /// in the same breath as cataloguing it.
    #[test]
    fn every_catalogued_error_code_is_a_real_one_and_none_of_them_is_unimplemented() {
        for op in CATALOGUE {
            assert!(!op.errors.is_empty(), "{} claims it cannot fail", op.verb);
            for code in op.errors {
                assert!(
                    ErrorCode::ALL.contains(code),
                    "{code} is not a Rivet code"
                );
                assert_ne!(
                    *code,
                    ErrorCode::Unimplemented,
                    "{} is catalogued and claims it may answer E_UNIMPLEMENTED",
                    op.verb
                );
            }
        }
    }

    /// The catalogue an adapter publishes is the catalogue that is performed —
    /// asserted through the rendered value rather than through the constant, so
    /// a projection that dropped a field would fail here.
    #[test]
    fn the_published_catalogue_names_every_operation_and_its_declared_inputs() {
        let published = catalogue();
        let Value::List(entries) = &published else {
            panic!("the catalogue must render as a list, got {published:?}");
        };
        assert_eq!(entries.len(), CATALOGUE.len());
        for (entry, op) in entries.iter().zip(CATALOGUE) {
            let entry = fields(entry);
            let verb = entry.iter().find(|(k, _)| *k == "verb").map(|(_, v)| v);
            assert_eq!(verb, Some(&Value::str(op.verb.as_str())));
            for key in ["verb", "summary", "inputs", "outputs", "subjects", "errors"] {
                assert!(
                    entry.iter().any(|(k, _)| *k == key),
                    "{} dropped {key}",
                    op.verb
                );
            }
        }
    }

    // -- arity -----------------------------------------------------------------

    /// Too few arguments is an argument error naming what the operation takes,
    /// rather than an operation refusing something it never saw.
    #[test]
    fn an_operation_given_too_few_inputs_refuses_with_arguments_and_names_them() {
        for op in CATALOGUE {
            if op.required_inputs() == 0 {
                continue;
            }
            let refusal =
                attempt(op.verb, &[]).expect_err(&format!("{} accepted no arguments", op.verb));
            assert_eq!(refusal.code(), ErrorCode::Arguments, "{}", op.verb);
            for input in op.inputs {
                assert!(
                    refusal.message().contains(input.name),
                    "{} did not name its input {}: {}",
                    op.verb,
                    input.name,
                    refusal.message()
                );
            }
        }
    }

    #[test]
    fn an_operation_given_too_many_inputs_refuses_rather_than_ignoring_the_extra() {
        for op in CATALOGUE {
            // An operation that declares a several-word input has no ceiling and
            // owns this refusal itself — see
            // `a_verb_with_no_word_ceiling_still_refuses_a_word_it_has_no_meaning_for`,
            // which is where the property moved rather than being dropped.
            let Some(most) = op.word_ceiling() else {
                continue;
            };
            let too_many: Vec<&str> = (0..=most).map(|_| "/etc/passwd").collect();
            let refusal = attempt(op.verb, &too_many)
                .expect_err(&format!("{} silently ignored an extra argument", op.verb));
            assert_eq!(refusal.code(), ErrorCode::Arguments, "{}", op.verb);
        }
    }

    /// **An operation with no arity ceiling owns the refusal the ceiling used to
    /// give**, and this is what says so rather than leaving it to that
    /// operation's own tests.
    ///
    /// [`Input::several`] removes the dispatcher's word count for the verb that
    /// declares one, so the property has to be checked from here: a word this
    /// build has no meaning for must still be refused, by name, rather than
    /// silently ignored. `apply edit` is the only such verb today and
    /// `crate::operation::edit` is where the refusal lives.
    #[test]
    fn a_verb_with_no_word_ceiling_still_refuses_a_word_it_has_no_meaning_for() {
        for op in CATALOGUE {
            if op.word_ceiling().is_some() {
                continue;
            }
            let subjects = op.subjects();
            let several = op
                .inputs
                .iter()
                .find(|i| i.several)
                .expect("an operation with no ceiling declares a several-word input");
            // The word is nonsense on purpose: no subject of any verb may read
            // it as anything, so the only correct answer is a refusal.
            let refusal = attempt(op.verb, &["edit", "/state/x", "--nonsense-word"])
                .expect_err(&format!("{} ignored a word it has no meaning for", op.verb));
            assert_eq!(refusal.code(), ErrorCode::Arguments, "{}", op.verb);
            assert!(
                !subjects.is_empty(),
                "{}: a several-word input belongs to a subject",
                op.verb
            );
            assert!(
                several.description.contains("SEVERAL WORDS"),
                "{}: the catalogue does not say that `{}` is several words: {}",
                op.verb,
                several.name,
                several.description
            );
        }
    }

    /// **The catalogue may name exactly one way in, and it is `ADR-035`'s
    /// control plane.**
    ///
    /// Prose naming any other tells a caller about a route they cannot take, and
    /// the cost is not cosmetic: it teaches a reader to discount the rest of the
    /// answer, which is the failure `ADR-017` §3.1.11 fixed on the console.
    ///
    /// **This is stated positively, and that is forced rather than chosen.** The
    /// obvious guard is a list of names that must not appear — and a list like
    /// that is itself the thing being removed, kept in a tracked file where the
    /// next reader finds it. So the assertion is the other way round: a sentence
    /// that talks about *how a caller reaches this machine* must name the
    /// control plane, and a sentence that names some other route fails on the
    /// word it uses to introduce one.
    ///
    /// **A sweep rather than a list of files**, for `AGENTS.md` §7's reason: the
    /// first time this was fixed it repaired the sentence that had been quoted,
    /// and the sibling copies in `edit` and `install` were found by looking
    /// rather than by being told. This walks everything the catalogue publishes,
    /// so the next one is found by the suite.
    #[test]
    fn any_catalogue_prose_about_reaching_this_machine_names_the_control_plane() {
        let mut checked = 0usize;
        let mut inspect = |what: String, text: &str| {
            checked += 1;
            let lowered = text.to_lowercase();
            // The phrases a sentence uses when it is telling a caller how they
            // got here or how to get back. A sentence using one of these and not
            // naming the control plane is describing something else.
            // "reached over" is deliberately NOT here: `get network` uses it
            // about the protocol an address was observed on, which is a fact
            // about the network and not a claim about how a caller got in.
            for reaching in ["arrives over", "arrived over", "logs in over", "connects over"] {
                if lowered.contains(reaching) {
                    assert!(
                        lowered.contains("control plane"),
                        "{what} says {reaching:?} and does not name ADR-035's control \
                         plane, which is the only way in this machine has:\n{text}"
                    );
                }
            }
        };

        for op in CATALOGUE {
            inspect(format!("{}'s summary", op.verb), op.summary);
            for input in op.inputs {
                inspect(format!("{}'s input `{}`", op.verb, input.name), input.description);
            }
            for subject in op.subjects() {
                for field in subject.outputs {
                    inspect(
                        format!("{} {}'s output `{}`", op.verb, subject.name, field.name),
                        field.description,
                    );
                }
            }
            for output in op.outputs() {
                inspect(
                    format!("{}'s published output `{}`", op.verb, output.name),
                    &output.description,
                );
            }
        }
        // A sweep that swept nothing would pass silently, which is the one way
        // this test could stop meaning anything.
        assert!(checked > 50, "the sweep only reached {checked} sentences");
    }
}
