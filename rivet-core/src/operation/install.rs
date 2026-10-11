//! `rivet apply install <digest> confirm` and `rivet get install` — **asking a
//! machine to install something, and finding out what happened.**
//!
//! # It takes a digest, and there is no form that takes a path
//!
//! **Changed 2026-09-12, `ADR-052` §7, and it is a removal rather than a
//! restriction.** Until that day the argument was an absolute path and the
//! machine handed it to the installer unread. That made every file on `/state`
//! installable, which made the fleet repository one route among several rather
//! than *the* route — `ADR-052`'s organising sentence is *"nothing reaches a
//! device without passing through the repository"*, and a path argument is the
//! sentence's exception written into the command line.
//!
//! So the argument is a BLAKE3 digest, resolved in this machine's own artifact
//! store ([`crate::artifact`]). **There is nothing to enforce, because the
//! capability is gone**: the only bytes a digest can name are bytes that
//! arrived through a transfer that hashed them, and an install naming a digest
//! the store does not hold is refused by name with the command that would put
//! it there.
//!
//! File transfer is untouched and stays what it was, for secrets and
//! configuration. What it can no longer do is lead to an install.
//!
//! One module for two verbs, which is the first time that has happened here.
//! The reason is `ADR-011`: `install` is one noun and the two operations are
//! the *request* and the *report* over it, so [`SUBJECT`] is written once and
//! both halves read it. Splitting them across `install.rs` and, say,
//! `installs.rs` would have put the word in two files and the vocabulary in
//! neither.
//!
//! # The ruling, and the one thing about it that is easy to lose
//!
//! The maintainer's 2026-08-24 ruling, which is
//! *"`apply install` requests and returns; `get install` reports"*:
//!
//! > **`rivet apply install <path> confirm` hands over the path, answers, and
//! > returns — before a byte is written.** … The write then proceeds without the
//! > operator's session holding it, which is the reason the maintainer's 2026-08-24 ruling made the
//! > installer a separate supervised program in the first place: *an update
//! > writes 640 MiB, and doing that from the caller's own process means a
//! > connection owns a long write.* **An answer that waited would hand the
//! > hazard straight back.**
//!
//! **Quoted as it was given.** The argument became a digest on 2026-09-12 and
//! everything the ruling decided is untouched: the operation still hands the
//! artifact over, answers, and returns before a byte is written, for exactly
//! that reason. What changed is what may be handed over.
//!
//! So this operation is a **sibling of [`super::stop`]**, not of
//! [`super::trial`]. Both deliver a request to another process and return;
//! neither can observe what that process then does; both say so in the same
//! words. `requested: true`, `installed: false`, and a `next` field saying in
//! plain words that the request was *delivered* and not *completed* —
//! `ADR-006` §4.2, which is what makes that enforceable rather than polite:
//!
//! > an unimplemented operation returns a specific, catalogued `Unimplemented`
//! > variant that no caller can mistake for a result.
//!
//! Applied here: **a request that was not delivered is a refusal, never a
//! result with a `false` in it.**
//!
//! # This operation does not open the payload. Deliberately
//!
//! It does not check the signature, the digests, the product or the
//! architecture. **It does check that the store holds the digest**, which is a
//! `readdir` and not a read: it answers *does this machine have these bytes*
//! and opens none of them. [`super::machine::Deployer::can_request_install`]
//! carries the argument in full; the short form is `ADR-029` §11's:
//!
//! > three formats means three verifiers each with a third of the scrutiny
//!
//! A check written here would be a **second implementation** of reading the
//! container, and the weaker of two is the one that eventually gets believed.
//!
//! **What the store's door changed, 2026-09-12 (`ADR-052` §7).** An
//! artifact is now in the store only if `rivet_core::package::verify` — the one
//! implementation, which the installer also calls — admitted it: signed by a
//! key this machine's trust store holds, entitled to its kind, every member
//! intact. So the plan can say the held artifact *was verified when it
//! arrived*. It still does not open the file itself, for two reasons: that
//! verdict was reached against the trust store in force **then**, and an image
//! update can have replaced it since; and whether the artifact *applies* — its
//! product, its architecture, a slot that will take it — was never the door's
//! question. Both are the installer's, in the journal, read back with
//! `get install`.
//!
//! **The consequence reaches the operator rather than staying in this
//! comment**: the plan says what was checked on arrival, what the installer
//! checks again, and the `next` field says where that verdict will appear. The
//! plan also says **what it is about to install** — the store answers with the
//! artifact's `ADR-027` name, because sixty-four hex characters in a
//! confirmation prompt tell an operator nothing.
//!
//! # Neither verb reports who asked, and both of them say where it is
//!
//! `ADR-028` §4.5, added 2026-08-24, made the request carry a principal. It does
//! **not** appear as a field of either verb here, and that is a consequence of
//! the shape rather than an omission: the principal reaches
//! `cmd/rivet-sysd`'s own record of taking the request up, and this
//! module's `get` answer is the installer's records grouped by `ADR-022` §5's
//! correlation identifier — a different service and a different chain, with **no
//! key to join them on**. [`WHO_ASKED`] carries the argument in full, and both
//! verbs point an operator at the SUPERVISOR's records rather than
//! leaving them to notice a field that is not there. ~~`rivet get log
//! rivet-sysd`~~ **— corrected 2026-08-30: `get log` has no service filter, so
//! that command selected a BOOT named `rivet-sysd` and answered nothing.** The
//! advice every one of these sites now gives is [`WHO_ASKED`]'s, in one place.
//!
//! # What the plan can say about a slot, and what it must not
//!
//! An `.rvt` carries three kinds (`ADR-029` §11) and only one of them —
//! a system update — has a slot at all. **Which kind this payload is cannot be
//! known without reading it**, which is exactly what the paragraph above
//! forbids. So the plan reports the slot an update *would* be written to, from
//! [`super::machine::Machine::running_slot`], and says that it applies only if
//! this turns out to be one:
//!
//! * on a machine that can read its firmware, `slot` is the **other** slot —
//!   `ADR-018` §4, and an install never touches the running one;
//! * on a machine that cannot, `slot` is **omitted** rather than guessed. A
//!   machine with no device tree is a development host or an emulator
//!   (`ADR-026` §4), and an application install works there perfectly well.
//!
//! A failure to read the slot is therefore **not** a refusal. That is a
//! judgement and it is the one this module makes that the record did not
//! decide: refusing would make `apply install` unusable for applications on
//! every machine that is not a board, and `ADR-015` §7's application install has
//! no slot in it anywhere.
//!
//! # The cost this pays for a third time
//!
//! `ADR-006` §8.5.1 priced two costs of keying the catalogue by verb alone and
//! §8.7 paid the second. **The first lands again here**, in both verbs:
//! `apply install <digest> confirm` puts a *digest* where
//! `apply trial-boot b confirm` puts a slot and `apply power-off confirm` puts
//! the confirmation, and `get install` takes no second word where `get log`
//! takes two. The overturn is unchanged and still the maintainer's — key the
//! catalogue by verb **and** noun, which §6.4 says is done by amending
//! `ADR-006`.

use super::apply::{CONFIRMATION, confirmed};
use super::machine::{Board, Held, MachineError, Observed};
use super::plan::{Change, Effect, Plan, TakesEffect, resolve};
use super::{ErrorCode, Field, JournalRead, OperationError, Verb};
use crate::artifact::Digest;
use crate::deploy::{OUTCOME_FIELD, Outcome, SERVICE_NAME};
use crate::journal::{Record, parse_lines};
use crate::state::Slot;
use crate::value::Value;

/// The subject both verbs accept. `ADR-006` §6.4's nouns are values, not
/// catalogue keys — `ADR-006` §8.5.1.
///
/// **One constant for two operations**, which is this module's whole reason for
/// being one module.
pub const SUBJECT: &str = "install";

/// What `apply install`'s result carries.
///
/// A near-copy of [`super::stop`]'s shape, on purpose: the two are the same
/// kind of thing — a request delivered to another process — and the
/// maintainer's 2026-08-24 install-request ruling says so: a single mental model
/// should cover every use of the verb (`.rulings/` R-264).
pub(super) const APPLY_OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was applied. Always `install` here.",
    },
    Field {
        name: "digest",
        description: "The BLAKE3 digest that was handed over, exactly as it was given — 64 \
                      lowercase hex characters. THIS MACHINE'S ARTIFACT STORE HOLDS IT; nothing \
                      has read what is inside it. There is no form of this operation that names \
                      a path (ADR-052 §7).",
    },
    Field {
        name: "artifact",
        description: "What that digest is, as THIS MACHINE'S STORE names it — the ADR-027 file \
                      name the artifact arrived under. Read back from the store rather than \
                      supplied by the caller, so it says what will be installed and not what \
                      somebody meant to install. It is a FILE NAME and not the signed header: \
                      the authoritative product, version and kind are read by the installer.",
    },
    Field {
        name: "requested",
        description: "Always true in a result. It means the request was DELIVERED, not that \
                      anything was installed (ADR-006 §4.2: a request that was not delivered \
                      is a refusal, never a result with a false in it).",
    },
    Field {
        name: "installed",
        description: "Always false. This operation returns BEFORE the first byte is written — \
                      that is the maintainer's 2026-08-24 ruling, because an update writes 640 MiB \
                      and THE CALLER'S CONNECTION must not own that write. A control-plane \
                      call and a login session are equally short-lived beside 640 MiB.",
    },
    Field {
        name: "slot",
        description: "Which of ADR-018 §4's two slots a SYSTEM UPDATE would be written into — \
                      always the one this machine is not running. ABSENT when this machine \
                      cannot read which slot it booted, and absent is not a failure: an \
                      application payload has no slot at all, and a development host has no \
                      device tree. Which kind this payload is is not known here.",
    },
    Field {
        name: "means",
        description: "What the machine will now do, in a sentence.",
    },
    Field {
        name: "next",
        description: "How to find out what happened — `rivet get install` — and what this \
                      answer does and does not say.",
    },
];

/// What `get install`'s result carries.
///
/// **The order is constrained rather than chosen.** `super::get`'s published
/// union is generated by merging the subjects' lists
/// (`super::Operation::outputs`), and two subjects that order the same pair of
/// fields differently produce no valid union at all. So the six names shared
/// with [`super::log`] — `subject`, `boot`, `records`, `returned`, `unreadable`,
/// `source` — appear here in that same relative order, and
/// `every_subjects_fields_survive_the_generated_union` in `super` is what fails
/// if a later edit forgets.
pub(super) const GET_OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was read. Always `install` in this build.",
    },
    Field {
        name: "installs",
        description: "How many separate installs this machine's journal still holds, across \
                      every boot it has kept. Retention drops the oldest file (ADR-022 §8), \
                      so this is what the machine has rather than what it has ever done.",
    },
    Field {
        name: "correlation",
        description: "ADR-022 §5's identifier for the LATEST install — the one every field \
                      below is about. Absent when `installs` is 0.",
    },
    Field {
        name: "boot",
        description: "Which boot that install happened in. Compare it against `rivet get \
                      log`'s `boots`: an install from an EARLIER boot is a finished story, \
                      and one from this boot may not be.",
    },
    Field {
        name: "began",
        description: "When its first record was stamped, RFC 3339. Absent on a machine that \
                      had no wall clock at the time (ADR-021) — the records carry `mono` \
                      either way.",
    },
    Field {
        name: "latest",
        description: "When its last record was stamped. Absent for `began`'s reason.",
    },
    Field {
        name: "finished",
        description: "Whether the journal holds this install's LAST record. **False does not \
                      mean it is running.** An installer that was killed, or whose machine \
                      lost power, leaves an unfinished chain for ever; the authority for \
                      what is running is the supervisor's cgroup (ADR-017 §3.1.3) and this \
                      operation cannot reach it. **AND TRUE DOES NOT MEAN IT WORKED — read \
                      `succeeded`.** A refused payload finishes.",
    },
    Field {
        name: "outcome",
        description: "How it ended, as the last record's own token: `installed`, \
                      `installed-application`, `planned`, `planned-application`, \
                      `refused-payload` or `refused-machine`. Absent when `finished` is \
                      false. Reported VERBATIM, so a token a newer Rivet wrote appears \
                      here rather than being read as one of these six.",
    },
    Field {
        name: "succeeded",
        description: "Whether this install ENDED WELL. Present exactly when `finished` is \
                      true, and it is the field to poll. `finished` says the chain stopped; \
                      this says how. A refused payload is `finished: true, succeeded: false` \
                      — the two are different questions and a caller that asks only the first \
                      reads a refusal as a success. Added 2026-08-30 because that is not \
                      hypothetical: it had happened twice, once to somebody who had read the \
                      warning against it.",
    },
    Field {
        name: "means",
        description: "What that outcome means, in a sentence. Absent when `outcome` is a \
                      token this build does not know — which is what tells you a newer \
                      Rivet wrote the record.",
    },
    Field {
        name: "errors",
        description: "How many of this install's records are at severity `error`.",
    },
    Field {
        name: "warnings",
        description: "How many are at `warn`. An install can succeed with warnings — \
                      ADR-029 §9.3's F5 is one, and a trusted key supplied on the command \
                      line is another.",
    },
    Field {
        name: "last",
        description: "The message of its most recent record, for a human. An agent should \
                      read `outcome` instead (ADR-006 §4.1).",
    },
    Field {
        name: "records",
        description: "Every record of this install, oldest first, each as the JSONL line the \
                      machine wrote (ADR-022 §3). In nushell: \
                      `rivet get install | from json | get records | each { from json }`.",
    },
    Field {
        name: "returned",
        description: "How many records are in `records`.",
    },
    Field {
        name: "unreadable",
        description: "Lines the journal holds that this build could not read AT ALL, across \
                      the whole journal rather than this install. NOT dropped silently \
                      (AGENTS.md §5): a non-zero count means some install's records may be \
                      missing from the count above.",
    },
    Field {
        name: "source",
        description: "Where this machine keeps its journal. Reported even when nothing was \
                      found, so an empty answer names a place rather than being a shrug.",
    },
    Field {
        name: "next",
        description: "What this answer does and does not settle, and what to do about it.",
    },
];

// ---------------------------------------------------------------------------
// apply install
// ---------------------------------------------------------------------------

/// An install request that was delivered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requested {
    digest: String,
    /// What the store said that digest is.
    artifact: String,
    /// The slot a system update would land in, where this machine could say.
    slot: Option<Slot>,
}

impl Requested {
    /// The digest that was handed over.
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// The artifact's own name, as this machine's store holds it.
    pub fn artifact(&self) -> &str {
        &self.artifact
    }

    /// Which slot a system update would be written into, if this machine could
    /// say which one it is running.
    pub const fn slot(&self) -> Option<Slot> {
        self.slot
    }

    /// The structured result, whose keys are a subsequence of `APPLY_OUTPUTS`
    /// — checked here rather than only in a test, because the catalogue naming
    /// a field no result carries is `ADR-006` §7's *"trusted and wrong"*.
    pub fn value(&self) -> Value {
        let mut fields = vec![
            ("subject", Value::str(SUBJECT)),
            ("digest", Value::str(self.digest.clone())),
            ("artifact", Value::str(self.artifact.clone())),
            // Said rather than implied: what happened is that a request was
            // accepted by the thing that installs on this machine.
            ("requested", Value::Bool(true)),
            ("installed", Value::Bool(false)),
        ];
        if let Some(slot) = self.slot {
            fields.push(("slot", Value::str(super::trial::slot_word(slot))));
        }
        fields.push(("means", Value::str(means(self.slot))));
        fields.push(("next", Value::str(next_after_request())));
        debug_assert!(
            super::get::in_declared_order(&fields, APPLY_OUTPUTS.iter().map(|f| f.name)),
            "an install result's keys must be a subsequence of APPLY_OUTPUTS, in order"
        );
        Value::object(fields)
    }
}

/// What the machine will do, in a sentence an operator reads before confirming.
///
/// **It has to carry what has and has not been checked**, because that is the
/// surprising half: the store verified the artifact when it arrived, and whether
/// this machine will install it is still the installer's verdict, which arrives
/// in the journal a moment later.
fn means(slot: Option<Slot>) -> String {
    format!(
        "the artifact is handed to {SERVICE_NAME}, which verifies the payload and installs it. \
         {target} If it is an APPLICATION it is unpacked under /state/app and made current, and \
         NOTHING STARTS IT (ADR-015 §3.2.1) — an install on its own leaves the application \
         idle, with no failure to diagnose. What makes it run is `enabled` in THIS MACHINE'S \
         DOCUMENT, which is the operator's and not the manifest's (ADR-015 §3.2.2): \
         `rivet apply edit /state/config/machine.toml --append '\n[services.<name>]\nenabled = \
         true\n' confirm` (the leading newline starts the table on a line of its own), then \
         `rivet apply reload confirm`. From a workstation \
         `cargo rivet install --enable` writes it for you. The artifact store VERIFIED the \
         signature and every member when the artifact arrived (ADR-052 §7); the installer \
         checks them again against the trust store as it is now — and, for a SYSTEM UPDATE, \
         its product, architecture and platform — with the same implementation: one verifier, \
         consulted twice, is \
         not the second verifier ADR-028 §2 warns against. A payload this machine will not \
         accept is refused there, with nothing written.",
        target = where_a_system_update_would_land(slot),
    )
}

/// **Which slot a system update would be written into**, in one sentence, and
/// the one place that sentence exists.
///
/// [`means`] puts it in the RESULT and [`RequestInstall::plan`] puts it in the
/// PLAN. Until 2026-08-26 the plan reached it by calling `means` whole; the
/// typed diff decomposes the rest of that paragraph into fields, and this clause
/// is the part that survives as prose in both. Written twice it would be two
/// accounts of one fact — `ADR-011` §3 — and they would drift the first time a
/// slot rule changed.
fn where_a_system_update_would_land(slot: Option<Slot>) -> String {
    match slot {
        // **The installer arms the trial** (`cmd/rivet-deploy`'s step 10), so
        // this names the one operation left: the restart that takes it. An
        // ordinary restart is named as the wrong one because it does not carry
        // the one-shot, and rebooting into the slot already running reads as an
        // update that did nothing.
        Some(slot) => format!(
            "If it is a SYSTEM UPDATE it will be written into slot {slot}, which is the one \
             this machine is not running (ADR-018 §4), and the next boot is armed as a \
             one-shot trial of it. NOTHING REBOOTS: `apply trial-restart confirm` starts the \
             trial, and PID 1 commits a healthy one — an ordinary `apply restart confirm` \
             does not carry the one-shot and would boot the default slot.",
            slot = super::trial::slot_word(slot),
        ),
        None => "This machine cannot say which slot it booted, so a system update would be \
                 refused by the installer and an application install is unaffected."
            .to_string(),
    }
}

/// What the operator should do and expect after a request is delivered.
fn next_after_request() -> String {
    format!(
        "the request was DELIVERED and the install has NOT happened — this answer returns \
         before the first byte is written, which is the whole point (the maintainer's 2026-08-24 install-request ruling): an \
         update writes 640 MiB and the connection asking must not own that write. \
         verified the payload yet, so a refusal is still ahead rather than behind. WATCH IT \
         WITH `rivet {get} {SUBJECT}`, which reports the install's own records and how it \
         ended; `rivet {get} log` has the same records among everything else. READ THAT \
         ANSWER'S `correlation` BEFORE ASKING FOR AN INSTALL AND WAIT FOR IT TO CHANGE: it \
         names ADR-022 §5's identifier for the LATEST install, this answer cannot carry it \
         because the installer has not started and none exists yet, and a machine that \
         installed something an hour ago ALREADY answers `finished: true`. Polling \
         `finished` on its own is a completion signal about somebody else's install, and it \
         has cost this project a whole deploy cycle (PLAN.md §73.2). An install \
         that succeeds does NOT reboot (ADR-029 §9.1) — a system update arms the next boot \
         as a one-shot trial of the inactive slot, and `rivet {apply} trial-restart \
         {CONFIRMATION}` starts it. THE REQUEST \
         RECORDS WHO ASKED (ADR-028 §4.5) — over ADR-035's control plane that is the \
         caller's PINNED ED25519 KEY, which IS the identity rather than a pointer to one. \
         A request that reached the machine no other way says exactly that, rather than \
         naming a caller it did not see — \
         {WHO_ASKED_SHORT} It is attribution and not \
         authority: nothing checked it, and anything running as uid 0 could have written the \
         request itself.",
        get = Verb::Get,
        apply = Verb::Apply,
    )
}

/// Perform `apply install`.
///
/// `arguments` is the whole of what followed the verb, so `arguments[0]` is the
/// subject, `arguments[1]` is the payload and `arguments[2]` is the
/// confirmation. **Nothing may follow it** — see [`confirmed`].
///
/// # Why this one takes the whole [`Board`] when its siblings take one port
///
/// Because it needs **two**: [`Deployer`] to deliver the request, and
/// [`Machine::running_slot`] to say which slot a system update would land in.
/// A single `&mut dyn Board` cannot be split into two `&mut dyn` at once, and
/// the alternatives were both worse than saying this plainly:
///
/// * a `Machine + Deployer` trait would be a **fourth** port existing only to
///   narrow, for one caller — and [`Board`]'s own documentation prices adding
///   ports;
/// * computing the slot in the dispatcher and passing it in would read the
///   firmware *before* the arguments are checked, so `apply install` with no
///   payload would mount and read a card to produce an argument error.
///
/// **The cost is an over-grant and it is stated rather than hidden**: this
/// operation is handed [`super::machine::Observed`] as well and has no business
/// with it. It is the only operation in this catalogue that is, and the day a
/// second one appears is the day the fourth trait is worth its keep.
pub(super) fn perform_apply(
    arguments: &[&str],
    board: &mut dyn Board,
) -> Result<Requested, OperationError> {
    let Some(word) = arguments.get(1).copied() else {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "`{apply} {SUBJECT}` needs the BLAKE3 DIGEST of the artifact to install, as the \
                 word after `{SUBJECT}` — `rivet {apply} {SUBJECT} <64 hex characters> \
                 {CONFIRMATION}`. {NAMES_NO_PATH} Read what this machine holds with `rivet \
                 {get} {SUBJECT}`; `cargo rivet install` prints the digest it sent.",
                apply = Verb::Apply,
                get = Verb::Get,
            ),
        ));
    };

    // **A word that is only the confirmation is a forgotten artifact**, not a
    // digest that happens to read `confirm`. Said separately from the digest
    // rule below because the two failures are different mistakes: this one
    // typed the whole command and left the middle out.
    if word == CONFIRMATION {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            word,
            format!(
                "`{CONFIRMATION}` is the confirmation word and cannot be the artifact, so this \
                 command names nothing to install. The digest goes first: `rivet {apply} \
                 {SUBJECT} <64 hex characters> {CONFIRMATION}`.",
                apply = Verb::Apply,
            ),
        ));
    }

    // **A path is refused by name**, and this arm exists entirely so that the
    // refusal is the right sentence. Until 2026-09-12 this operation took a
    // path; every note, script and habit that says `rivet apply install
    // /state/thing.rvt confirm` still exists, and answering one of them with
    // *"that is not 64 hex characters"* would be true and useless.
    let Some(digest) = Digest::new(word) else {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            word,
            if looks_like_a_path(word) {
                format!(
                    "`{word}` is a PATH, and `{apply} {SUBJECT}` does not take one any more — \
                     ruled 2026-09-12, ADR-052 §7. {NAMES_NO_PATH} Nothing has been delivered. \
                     Send the artifact with `cargo rivet install`, which transfers it into this \
                     machine's store and asks for the install by the digest it just sent.",
                    apply = Verb::Apply,
                )
            } else {
                format!(
                    "`{word}` is not a BLAKE3 digest. A digest is exactly {chars} LOWERCASE hex \
                     characters — upper case is refused rather than folded, because two \
                     spellings of one digest would be two entries in the store. Nothing has \
                     been delivered.",
                    chars = crate::artifact::DIGEST_CHARS,
                )
            },
        ));
    };

    if let Some(extra) = arguments.get(3).copied() {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            extra,
            format!(
                "`{apply} {SUBJECT}` takes one digest and then the confirmation. `{extra}` is \
                 a fourth word. An install writes into one slot and there is only one inactive \
                 slot, so there is no form of this command that takes two artifacts.",
                apply = Verb::Apply,
            ),
        ));
    }

    let confirmed = confirmed(arguments.get(2).copied())?;

    // Everything above this line is argument parsing; the plan/apply split is
    // `super::plan`'s driver — `ADR-006` §6.5.
    resolve(RequestInstall { digest }, confirmed, board)
}

/// The sentence that says what this operation now is, written once because it
/// belongs in three refusals.
const NAMES_NO_PATH: &str = "THERE IS NO FORM OF THIS OPERATION THAT NAMES A PATH: an install \
                             resolves a digest in THIS MACHINE'S OWN ARTIFACT STORE \
                             (/state/artifacts), which is filled by the transfer that hashed the \
                             bytes on the way in. That is ADR-052's rule — nothing reaches a \
                             device without passing through the fleet repository — and it is a \
                             capability that was removed rather than a check that was added.";

/// Whether a word was probably meant as a path.
///
/// **Deliberately generous.** It decides which of two refusals to print and
/// nothing else, so a false positive costs a slightly wrong sentence and a
/// false negative costs an operator re-reading a message that did not address
/// what they typed. A digest cannot match any of these — it is hex.
fn looks_like_a_path(word: &str) -> bool {
    word.contains('/')
        || word.contains('\\')
        || word.ends_with(crate::package::kind::SUFFIX)
        || word.starts_with('.')
}

/// **The change `apply install` is**, as `ADR-006` §6.5 defines one.
struct RequestInstall {
    digest: Digest,
}

impl Change for RequestInstall {
    /// The slot a system update would land in where this machine can say, and
    /// **what the store holds under this digest**.
    ///
    /// **Read once, on the way to the plan**, and the associated type is what
    /// makes that structural. It used to be read before the branch precisely
    /// *"because the plan is where an operator decides whether to confirm and
    /// which slot is the fact they are deciding about"* — the same argument,
    /// now expressed as a type rather than as a comment above an `if`, and now
    /// carrying a second fact for the same reason: the artifact's name is the
    /// other thing the operator is deciding about, and it must be the one the
    /// STORE reported at plan time rather than a second lookup that could
    /// answer differently.
    type Prepared = (Option<Slot>, Held);
    type Applied = Requested;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, Self::Prepared), OperationError> {
        // A machine that cannot say which slot it booted is **not** refused —
        // this module's documentation is where that judgement is recorded.
        let slot = board.running_slot().ok().map(Slot::other);
        let held = board.can_request_install(&self.digest).map_err(refused)?;

        let plan = Plan::new(self.digest.as_str(), "installs software on this machine")
            .doing(
                // The slot sentence comes from the one place it is written, so
                // the plan and the result cannot describe it differently.
                Effect::request(
                    SERVICE_NAME,
                    format!("verify and install {name}", name = held.name),
                )
                .noting(where_a_system_update_would_land(slot)),
            )
            .leaving(
                "NOTHING HAS BEEN HANDED OVER — the request is delivered only if this is \
                 confirmed",
            )
            .leaving(format!(
                "THIS MACHINE ALREADY HOLDS THE ARTIFACT — {name} is in the store at this \
                 digest, and it was VERIFIED ON ARRIVAL: signed by a key this machine's trust \
                 store held, and every member intact (ADR-052 §7). Nothing unverified is \
                 admitted to the store — though an artifact a build from before 2026-09-12 \
                 stored was not checked, and is checked when it is next sent. That is why this \
                 operation can tell you WHAT it would install before you confirm",
                name = held.name,
            ))
            .leaving(
                "THIS OPERATION HAS NOT READ INSIDE THE FILE — the installer verifies it again, \
                 against the trust store as it is now, with the same implementation the store \
                 used, and checks a SYSTEM UPDATE's product, architecture and platform (ADR-028 §2: one \
                 verifier, consulted twice). A payload this machine will not accept is refused \
                 there, with nothing written",
            )
            .leaving("NOTHING REBOOTS, and nothing is started (ADR-015 §3.2.1)")
            // **The second `NotAsserted` in this build, and it is the honest
            // answer rather than a gap.** Whether a restart is needed depends on
            // whether the payload is a system update or an application, and the
            // paragraph above says why nothing here has opened it to find out.
            .taking_effect(TakesEffect::NotAsserted {
                because: "this operation has not read the payload's kind, and a SYSTEM UPDATE lands in the \
                          inactive slot while an APPLICATION is unpacked and not started",
            })
            .confirmed_by(&[SUBJECT, self.digest.as_str()]);

        Ok((plan, (slot, held)))
    }

    fn apply(
        self,
        prepared: Self::Prepared,
        board: &mut dyn Board,
    ) -> Result<Requested, OperationError> {
        let (slot, _planned) = prepared;
        // **Asked again rather than believed**, `AGENTS.md` §5: the store is the
        // authority for what it holds and it is asked at the point of use. The
        // name in the RESULT is therefore the one that was true when the
        // request was written, not the one the plan printed.
        let held = board.request_install(&self.digest).map_err(refused)?;
        Ok(Requested {
            digest: self.digest.as_str().to_string(),
            artifact: held.name,
            slot,
        })
    }
}

/// The one place a [`MachineError`] becomes this operation's refusal.
///
/// The same mapping [`super::stop`] and [`super::trial`] use, for the same
/// reason — an operator does two different things next — and **nothing is
/// reworded on the way through**.
fn refused(error: MachineError) -> OperationError {
    let code = match error {
        MachineError::Unsafe(_) => ErrorCode::Unsafe,
        MachineError::Refused(_) => ErrorCode::MachineRefused,
    };
    OperationError::about(code, SUBJECT, error.why())
}

// ---------------------------------------------------------------------------
// get install
// ---------------------------------------------------------------------------

/// Perform `get install`.
///
/// **It reads the journal and nothing else**, which is `ADR-011` rather than
/// economy. The installer is a separate process that has usually already
/// exited; what it did is written in `ADR-022` §3's records, under one
/// `ADR-022` §5 correlation identifier, and those records are the account. A
/// second source — a status file the installer wrote, a marker on `/state` —
/// would be a second answer to *what happened*, and the two would eventually
/// disagree about an install that was interrupted, which is precisely the case
/// somebody would be asking about.
pub(super) fn perform_get(
    arguments: &[&str],
    observed: &mut dyn Observed,
) -> Result<Value, OperationError> {
    // `get storage`'s rule, for `get storage`'s reason: an operator typing
    // `rivet get install /state/x.rvt` expecting a per-payload answer must be
    // told it was not read, rather than shown the machine's latest install and
    // left to assume it was about their file.
    if let Some(extra) = arguments.get(1).copied() {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            extra,
            format!(
                "`{get} {SUBJECT}` takes no second word and `{extra}` was not read. It reports \
                 this machine's latest install, whatever it was; there is no per-payload form, \
                 because a payload is not a thing this machine keeps. For an older one, read \
                 the records: `rivet {get} log all`.",
                get = Verb::Get,
            ),
        ));
    }

    let read: JournalRead = observed
        .read_journal()
        .map_err(|e| OperationError::about(ErrorCode::MachineRefused, SUBJECT, e.to_string()))?;

    let (records, refused_lines) = parse_lines(read.lines.iter().map(String::as_str));
    let chains = chains_of(&records);
    let latest = chains.last();

    let mut fields = vec![
        ("subject", Value::str(SUBJECT)),
        (
            "installs",
            Value::Integer(i64::try_from(chains.len()).unwrap_or(i64::MAX)),
        ),
    ];

    let mut selected: Vec<&Record> = Vec::new();
    if let Some(chain) = latest {
        selected = chain.clone();
        let ended = terminal(chain);
        fields.push((
            "correlation",
            Value::str(chain.first().map_or(String::new(), |r| r.corr.clone())),
        ));
        fields.push((
            "boot",
            Value::str(chain.first().map_or(String::new(), |r| r.boot.clone())),
        ));
        if let Some(began) = chain.first().and_then(|r| r.ts.clone()) {
            fields.push(("began", Value::str(began)));
        }
        if let Some(latest_ts) = chain.last().and_then(|r| r.ts.clone()) {
            fields.push(("latest", Value::str(latest_ts)));
        }
        fields.push(("finished", Value::Bool(ended.is_some())));
        if let Some(token) = &ended {
            // Verbatim — `AGENTS.md` §5. A token this build does not know is
            // reported as the machine wrote it rather than mapped onto one of
            // the six it does know.
            fields.push(("outcome", Value::str(token.clone())));
            if let Some(known) = Outcome::from_record_field(token) {
                // **Beside `outcome`, and before `means`.** A caller scanning
                // this answer meets the verdict as a BOOLEAN at the same moment
                // it meets the token, which is the point: `finished` is the
                // field people poll and it answers a different question.
                //
                // Absent for a token this build does not know, for `means`'s
                // reason exactly — a newer Rivet's outcome cannot be judged
                // here, and guessing `false` would report a refusal that did
                // not happen.
                fields.push(("succeeded", Value::Bool(known.succeeded())));
                fields.push(("means", Value::str(known.means())));
            }
        }
        fields.push((
            "errors",
            Value::Integer(count_at(chain, crate::journal::Severity::Error)),
        ));
        fields.push((
            "warnings",
            Value::Integer(count_at(chain, crate::journal::Severity::Warn)),
        ));
        if let Some(last) = chain.last() {
            fields.push(("last", Value::str(last.msg.clone())));
        }
    }

    fields.push((
        "records",
        Value::List(
            selected
                .iter()
                .map(|r| Value::str(r.to_jsonl().trim_end()))
                .collect(),
        ),
    ));
    fields.push((
        "returned",
        Value::Integer(i64::try_from(selected.len()).unwrap_or(i64::MAX)),
    ));
    fields.push((
        "unreadable",
        Value::Integer(i64::try_from(refused_lines.len()).unwrap_or(i64::MAX)),
    ));
    fields.push(("source", Value::str(read.source)));
    fields.push((
        "next",
        Value::str(next_after_report(latest.map(|c| terminal(c)))),
    ));

    debug_assert!(
        super::get::in_declared_order(&fields, GET_OUTPUTS.iter().map(|f| f.name)),
        "a get install result's keys must be a subsequence of GET_OUTPUTS, in order"
    );
    Ok(Value::object(fields))
}

/// Every install this journal holds, **oldest first**, each as its own records
/// in the order they were written.
///
/// # Why the correlation identifier is the grouping and the boot is not
///
/// `ADR-022` §5 makes the correlation identifier the thing that ties one causal
/// chain together, and `cmd/rivet-deploy` mints exactly one per run — its
/// `Journal::next_correlation` says why in as many words: *"two installs in one
/// boot is an ordinary thing to do — install, find the payload was the wrong
/// one, install again — and a reader following one chain must not splice the
/// two."* Grouping by boot would do exactly that splicing.
///
/// # Order is the journal's, and it is the only order there is
///
/// [`super::machine::Observed::read_journal`] returns lines oldest first and
/// makes that a contract, for the reason [`crate::journal::boots_in_order`]
/// already depends on: a Pi has no RTC, so timestamps cannot order records
/// across a boot and the sequence is the authority (`ADR-021`). Chains are
/// therefore ordered by where their **first** record appeared, which is what
/// makes `last()` the latest install rather than the one with the largest
/// identifier.
///
/// # `svc` alone is a claim; `actor` is what the supervisor observed
///
/// The ingest socket is world-writable by design (`ADR-022` §11.1), and any
/// local sender may claim `svc = "rivet-deploy"` — [`crate::journal::
/// SUPERVISOR_SVC`]'s own documentation names this trade and says a reader who
/// checked `actor` would see through it. So a record only joins a chain here
/// when it also carries [`crate::journal::SUPERVISOR_ACTOR`] — the value the
/// supervisor stamps on its own notes and on everything it captured from a
/// service's stdout or stderr, and the one value an ingested record can never
/// carry: the kernel overrides a sender's claimed actor with its own uid,
/// unconditionally, before the record is ever written.
fn chains_of(records: &[Record]) -> Vec<Vec<&Record>> {
    let mut order: Vec<String> = Vec::new();
    let mut chains: Vec<Vec<&Record>> = Vec::new();
    for record in records {
        if record.svc != SERVICE_NAME || record.actor != crate::journal::SUPERVISOR_ACTOR {
            continue;
        }
        match order.iter().position(|corr| *corr == record.corr) {
            Some(at) => chains[at].push(record),
            None => {
                order.push(record.corr.clone());
                chains.push(vec![record]);
            }
        }
    }
    chains
}

/// The [`OUTCOME_FIELD`] token of a chain's terminal record, if it has one.
///
/// **Searched rather than read off the last record**, and the difference
/// matters on a machine whose journal is being appended to while this runs: a
/// record written after the terminal one — there is none today, and "there is
/// none today" is exactly the kind of claim `AGENTS.md` §7 is about — would
/// otherwise make a finished install report itself unfinished.
fn terminal(chain: &[&Record]) -> Option<String> {
    chain.iter().rev().find_map(|record| {
        match record.field(OUTCOME_FIELD) {
            Some(crate::journal::Value::Str(token)) => Some(token.clone()),
            // A non-string `outcome` is not this build's token and is not
            // reported as one. Rendering it is better than dropping it: a
            // reader is told the field was there and was not a token.
            Some(other) => Some(other.to_display()),
            None => None,
        }
    })
}

/// How many of a chain's records are at `severity`.
fn count_at(chain: &[&Record], severity: crate::journal::Severity) -> i64 {
    i64::try_from(chain.iter().filter(|r| r.sev == severity).count()).unwrap_or(i64::MAX)
}

/// **Where "who asked" is, since it is not here.**
///
/// `ADR-028` §4.5 made the install request carry a principal, and this operation
/// still cannot report one. That is a fact about the shape of the mechanism
/// rather than an oversight, and it is said out loud rather than left for
/// somebody to discover from an absent field:
///
/// * the principal reaches `cmd/rivet-sysd`, which writes it into **its
///   own** record when it takes the request up — under its own service name and
///   its own correlation identifier;
/// * this operation's answer is [`chains_of`], which is every record whose `svc`
///   is [`SERVICE_NAME`], grouped by `ADR-022` §5's identifier. The supervisor's
///   record is neither, so it is not in the chain;
/// * and **there is no key to join them on.** `cmd/rivet-deploy` mints its own
///   correlation identifier when it starts, and nothing carries the request's
///   principal into it — `definitions::deploy` hands it one argument, the
///   payload path. A join on *"the last request note before this chain began"*
///   would be a guess dressed as an answer, which is exactly what
///   [`chains_of`]'s own documentation refuses to do with boots.
///
/// **So the answer is a pointer, not a field**, and the honest fix is for the
/// principal to reach the installer's argument vector so that every record of an
/// install carries it. That is `cmd/rivet-deploy`'s argument contract and it
/// is the maintainer's to rule.
const WHO_ASKED: &str = "WHO ASKED FOR IT IS NOT IN THIS ANSWER: the request's principal reaches \
                         the SUPERVISOR's record of taking the request up, which is a different \
                         service and a different correlation chain (ADR-028 §4.5). Read it with \
                         `rivet get log` and look for a record whose `svc` is `rivet-sysd`  \
                         and which carries `principal`. Read what \
                         it says there before believing it — it is attribution and not authority, \
                         and anything running as uid 0 could have written it.";

/// The same pointer in one clause, for a sentence that already has a subject.
///
/// **Both spellings say `get log` and neither names a service as a word**, and
/// that is the whole of the 2026-08-30 correction. `get log`'s inputs are the
/// subject, the boot, the limit and a cursor; a service name in the second
/// position selects a BOOT, and until that day the machine answered it with an
/// empty `records` rather than a refusal. `super::log`'s `refuse_unknown_boot`
/// is the other half of the fix, and it is the half that survives this sentence
/// going stale again.
const WHO_ASKED_SHORT: &str = "a `rivet-sysd` record in `rivet get log` is where it appears — that operation has no service filter, so select on `svc`.";

/// What this answer settles and what it does not.
///
/// Three cases, and the middle one is the reason this field exists at all:
/// **an unfinished chain is not a running install**, and reading it as one is
/// how somebody waits an hour for a process that died with the boot.
fn next_after_report(ended: Option<Option<String>>) -> String {
    let common = format!(
        "these are the records {SERVICE_NAME} wrote, read out of this machine's journal \
         (ADR-022 §9). They are the whole account: the installer is a separate process \
         (the maintainer's 2026-08-24 ruling) and there is no second place it reports to. {WHO_ASKED}"
    );
    match ended {
        None => format!(
            "{common} THIS MACHINE'S JOURNAL HOLDS NO INSTALL. That is not the same as \
             \"nothing has ever been installed\" — retention drops the oldest file (ADR-022 §8) \
             and a journal that never reached the card holds nothing at all, which is ordinary \
             under QEMU. Ask a machine to install something with `rivet {apply} {SUBJECT} \
             <path.rvt> {CONFIRMATION}`.",
            apply = Verb::Apply,
        ),
        Some(None) => format!(
            "{common} THIS INSTALL HAS NO LAST RECORD, and that does NOT mean it is running: an \
             installer that was killed, or a machine that lost power mid-install, leaves a \
             chain that never ends. The authority for what is running is the supervisor's \
             cgroup (ADR-017 §3.1.3) and this operation cannot reach it — compare `boot` \
             against `rivet {get} log`'s `boots` instead, because a chain from an earlier \
             boot is certainly not running now.",
            get = Verb::Get,
        ),
        Some(Some(token)) => match Outcome::from_record_field(&token) {
            Some(known) if known.succeeded() => format!(
                "{common} It finished: {means}. `records` is the whole chain if you want the \
                 detail.",
                means = known.means(),
            ),
            Some(known) => format!(
                "{common} It FAILED: {means}. `last` is the refusal in the installer's own \
                 words, and `records` is everything that led to it.",
                means = known.means(),
            ),
            None => format!(
                "{common} It finished with an outcome THIS BUILD DOES NOT KNOW — `{token}`. The \
                 ordinary reason is that a newer Rivet wrote these records; read `last` and \
                 `records`, which are the installer's own words and do not depend on this build \
                 recognising anything."
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::journal::{Record, Severity};
    use crate::operation::machine::{Deployer, Machine, MemoryRead, Reboot, StorageRead};

    // -- fakes -----------------------------------------------------------------

    /// A machine that answers as scripted and records every request.
    ///
    /// **Its store answers with a name**, because that is what the port does
    /// now: a digest resolves to the artifact this machine holds under it, or
    /// it does not resolve at all.
    struct FakeBoard {
        slot: Result<Slot, MachineError>,
        allowed: Result<Held, MachineError>,
        delivered: Result<Held, MachineError>,
        planned: Vec<String>,
        requested: Vec<String>,
        journal: Result<JournalRead, MachineError>,
    }

    crate::operation::machine::refuses_to_edit!(FakeBoard);

    impl FakeBoard {
        fn healthy() -> Self {
            Self {
                slot: Ok(Slot::A),
                allowed: Ok(Held::named(ARTIFACT)),
                delivered: Ok(Held::named(ARTIFACT)),
                planned: Vec::new(),
                requested: Vec::new(),
                journal: Ok(JournalRead::default()),
            }
        }

        /// A build with no installer to reach — a development host.
        fn no_installer() -> Self {
            let why = MachineError::Unsafe("this build cannot reach an installer".to_string());
            Self {
                allowed: Err(why.clone()),
                delivered: Err(why),
                ..Self::healthy()
            }
        }

        /// A machine whose store does not hold what was asked for — the state
        /// every machine is in for every digest but a handful.
        fn holds_nothing() -> Self {
            let why = MachineError::Refused(
                "REFUSED this machine's artifact store does not hold that digest".to_string(),
            );
            Self {
                allowed: Err(why.clone()),
                delivered: Err(why),
                ..Self::healthy()
            }
        }

        fn holding(lines: &[String]) -> Self {
            Self {
                journal: Ok(JournalRead {
                    lines: lines.to_vec(),
                    files: vec!["/state/log/journal.jsonl".to_string()],
                    source: "/state/log".to_string(),
                }),
                ..Self::healthy()
            }
        }
    }

    /// **The reload port refuses in every fake but `super::reload`'s.**
    /// `AGENTS.md` §3: a stub refuses and never fakes success, so a test in
    /// this module that started reloading fails loudly.
    fn no_reload() -> MachineError {
        MachineError::Refused(
            "this fake does not reload; an operation that reached this port has changed"
                .to_string(),
        )
    }

    impl Machine for FakeBoard {
        // **The two `ADR-032` §3.2 added on 2026-08-26.** This fake does not
        // reload anything, and refusing is the honest stand-in (`AGENTS.md` §3):
        // a test here that started reloading fails loudly rather than passing on
        // a fiction.
        fn can_request_reload(&mut self) -> Result<(), MachineError> {
            Err(no_reload())
        }
        fn request_reload(&mut self) -> Result<(), MachineError> {
            Err(no_reload())
        }

        fn can_request_service_restart(
            &mut self,
            _service: &str,
        ) -> Result<crate::operation::Restarting, MachineError> {
            Err(no_reload())
        }

        fn request_service_restart(
            &mut self,
            _service: &str,
        ) -> Result<crate::operation::Restarting, MachineError> {
            Err(no_reload())
        }

        fn running_slot(&mut self) -> Result<Slot, MachineError> {
            self.slot.clone()
        }
        fn can_arm_trial_boot(&mut self, _slot: Slot) -> Result<(), MachineError> {
            panic!("an install must not arm anything; the installer does that at the end")
        }
        fn arm_trial_boot(&mut self, _slot: Slot) -> Result<Reboot, MachineError> {
            panic!("an install must not arm anything")
        }
        fn can_request_stop(&mut self, _stop: super::super::Stop) -> Result<(), MachineError> {
            panic!("an install must not stop the machine")
        }
        fn request_stop(&mut self, _stop: super::super::Stop) -> Result<(), MachineError> {
            panic!("an install must not stop the machine")
        }
    }

    impl Deployer for FakeBoard {
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

        fn can_request_install(&mut self, digest: &Digest) -> Result<Held, MachineError> {
            self.planned.push(digest.as_str().to_string());
            self.allowed.clone()
        }
        fn can_request_uninstall(&mut self, _name: &str) -> Result<(), MachineError> {
            Ok(())
        }
        fn request_uninstall(&mut self, _name: &str, _purge: bool) -> Result<(), MachineError> {
            Ok(())
        }
        /// This fake removes no library, and refusing is the honest stand-in
        /// (`AGENTS.md` §3).
        fn can_request_library_uninstall(
            &mut self,
            _selector: &crate::uninstall_library::Selector,
        ) -> Result<crate::uninstall_library::Removal, MachineError> {
            Err(MachineError::Refused(
                "this fake removes no library; an operation that reached this port has changed"
                    .to_string(),
            ))
        }
        fn request_library_uninstall(
            &mut self,
            selector: &crate::uninstall_library::Selector,
        ) -> Result<crate::uninstall_library::Removal, MachineError> {
            self.can_request_library_uninstall(selector)
        }
        fn request_install(&mut self, digest: &Digest) -> Result<Held, MachineError> {
            self.requested.push(digest.as_str().to_string());
            self.delivered.clone()
        }
    }

    impl Observed for FakeBoard {
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
    fn read_memory(&mut self) -> Result<MemoryRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_desired_state(&mut self) -> Result<Option<String>, MachineError> {
        Ok(None)
    }
        /// `ADR-032` §3.2's query socket, added 2026-08-26. This fake asks no
        /// supervisor and refuses rather than pretending (`AGENTS.md` §3).
        fn supervisor_status(
            &mut self,
        ) -> Result<crate::operation::status::Status, MachineError> {
            Err(MachineError::Refused(
                "this fake answers no supervisor; an operation that reached this port has changed"
                    .to_string(),
            ))
        }

        fn read_journal(&mut self) -> Result<JournalRead, MachineError> {
            self.journal.clone()
        }
        /// The fake supplies no schema: `rivet-core` cannot name one
        /// (`ADR-014` §3.2), so a test that needs the real document belongs in
        /// the adapter that has both crates. Loud rather than empty, so an
        /// assertion against real schema content fails instead of passing.
        fn configuration_schema(&mut self) -> Value {
            Value::str("<no schema: this is a test fake>")
        }

        fn read_filesystems(&mut self) -> Result<StorageRead, MachineError> {
            panic!("get install does not read the mount table")
        }
    }

    /// The digest every test below asks for, spelled once.
    const DIGEST: &str = "3f7c1a9e04b2d85f6a3c9017e2b48d5f1c6a09b3e7d248fa5601c93b8ed72a4f";

    /// What this machine's store says that digest is.
    const ARTIFACT: &str = "photo-index-0.1.0-aarch64-application.rvt";

    /// The path form, kept as a fixture **because it must be refused**. Until
    /// 2026-09-12 it was the argument; it is here so that a build which quietly
    /// started accepting one again fails.
    const A_PATH: &str = "/state/photo-index.rvt";

    /// A record shaped exactly as the supervisor writes one for a service it
    /// captured — `svc` is the installer's, `actor` is the supervisor's own
    /// (`note_output` overrides a captured line's claimed actor the same way
    /// it overrides `svc` and `boot`), never [`crate::deploy::ACTOR`], which
    /// is only what `cmd/rivet-deploy` claims on its own stdout before the
    /// supervisor ever sees it.
    fn record(corr: &str, sev: Severity, msg: &str, fields: Vec<(&str, crate::journal::Value)>) -> String {
        Record {
            ts: Some("2026-08-24T21:00:00.000Z".to_string()),
            boot: "b0".to_string(),
            mono: 1.0,
            sev,
            svc: SERVICE_NAME.to_string(),
            corr: corr.to_string(),
            actor: crate::journal::SUPERVISOR_ACTOR.to_string(),
            msg: msg.to_string(),
            fields: fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }
        .to_jsonl()
        .trim_end()
        .to_string()
    }

    /// A record shaped as one the ingest socket admitted: `svc` is whatever
    /// the sender claimed (`ADR-022` §11.1 lets it claim almost anything), and
    /// `actor` is the kernel's own answer — [`crate::journal::
    /// INGEST_ACTOR_PREFIX`] plus a uid — never the supervisor's. J1.
    fn ingest_claim(
        corr: &str,
        actor: &str,
        svc: &str,
        msg: &str,
        fields: Vec<(&str, crate::journal::Value)>,
    ) -> String {
        Record {
            ts: Some("2026-08-24T21:00:00.000Z".to_string()),
            boot: "b0".to_string(),
            mono: 1.0,
            sev: Severity::Info,
            svc: svc.to_string(),
            corr: corr.to_string(),
            actor: actor.to_string(),
            msg: msg.to_string(),
            fields: fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }
        .to_jsonl()
        .trim_end()
        .to_string()
    }

    fn other_service(corr: &str, msg: &str) -> String {
        Record {
            ts: Some("2026-08-24T21:00:00.000Z".to_string()),
            boot: "b0".to_string(),
            mono: 1.0,
            sev: Severity::Info,
            svc: "rivet-netd".to_string(),
            corr: corr.to_string(),
            actor: crate::journal::SUPERVISOR_ACTOR.to_string(),
            msg: msg.to_string(),
            fields: Vec::new(),
        }
        .to_jsonl()
        .trim_end()
        .to_string()
    }

    fn get(board: &mut FakeBoard, args: &[&str]) -> Result<Value, OperationError> {
        perform_get(args, board)
    }

    fn fields_of(value: &Value) -> Vec<(&'static str, Value)> {
        match value {
            Value::Object(fields) => fields.clone(),
            other => panic!("a result must be an object, got {other:?}"),
        }
    }

    fn field_of(value: &Value, key: &str) -> Option<Value> {
        fields_of(value)
            .into_iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v)
    }

    // -- apply install: the confirmation ---------------------------------------

    /// **`ADR-012` §7.1, as the property that matters**: without the word,
    /// nothing is handed to anything.
    #[test]
    fn without_the_confirmation_word_no_install_is_ever_requested() {
        let mut board = FakeBoard::healthy();
        let refusal = perform_apply(&[SUBJECT, DIGEST], &mut board)
            .expect_err("it installed without confirmation");
        assert_eq!(refusal.code(), ErrorCode::ConfirmationRequired);
        assert!(board.requested.is_empty(), "{:?}", board.requested);
        assert_eq!(board.planned, vec![DIGEST.to_string()]);
    }

    /// And the refusal has to be a *plan*: what would happen, that nothing has
    /// been handed over, and the exact next command.
    #[test]
    fn the_unconfirmed_refusal_says_what_happens_and_the_command_that_does_it() {
        let mut board = FakeBoard::healthy();
        let refusal = perform_apply(&[SUBJECT, DIGEST], &mut board)
            .expect_err("unconfirmed");
        let message = refusal.message();
        assert!(message.contains("ADR-012 §7.1"), "{message}");
        assert!(message.contains("NOTHING HAS BEEN HANDED OVER"), "{message}");
        assert!(
            message.contains(&format!("rivet apply install {DIGEST} confirm")),
            "{message}"
        );
        assert_eq!(refusal.subject(), Some(DIGEST));
    }

    /// **The plan names the artifact, not only the digest.** Sixty-four hex
    /// characters say nothing about what is in the file, and the machine is the
    /// only thing that can say — this is the fact `apply install` gained when it
    /// stopped taking a path, and it is the one an operator is deciding on.
    #[test]
    fn the_plan_says_what_this_machine_would_actually_install() {
        let mut board = FakeBoard::healthy();
        let refusal = perform_apply(&[SUBJECT, DIGEST], &mut board)
            .expect_err("unconfirmed");
        let message = refusal.message();
        assert!(
            message.contains(ARTIFACT),
            "the plan did not say what it would install: {message}"
        );
        assert!(
            message.contains("THIS MACHINE ALREADY HOLDS THE ARTIFACT"),
            "{message}"
        );
    }

    /// **The plan says exactly what has been checked, and by whom.** The store
    /// verified the artifact on arrival — since 2026-09-12 nothing unverified is
    /// admitted — and this operation has not read it, so the installer's verdict
    /// against the trust store as it is NOW is still to come. An operator who
    /// reads *"verified on arrival"* must not read *"will install"*.
    #[test]
    fn the_plan_says_the_store_verified_it_and_the_installer_checks_again() {
        let mut board = FakeBoard::healthy();
        let refusal = perform_apply(&[SUBJECT, DIGEST], &mut board)
            .expect_err("unconfirmed");
        let message = refusal.message();
        assert!(message.contains("VERIFIED ON ARRIVAL"), "{message}");
        assert!(message.contains("ADR-052 §7)"), "{message}");
        assert!(
            message.contains("THIS OPERATION HAS NOT READ INSIDE THE FILE"),
            "{message}"
        );
        assert!(message.contains("the installer verifies it again"), "{message}");
        assert!(message.contains("ADR-028 §2"), "{message}");
    }

    /// With the word, the request is delivered — and it is the digest that was
    /// named, never one worked out.
    #[test]
    fn with_the_confirmation_word_the_named_digest_is_handed_over_and_only_that_one() {
        let mut board = FakeBoard::healthy();
        let requested = perform_apply(
            &[SUBJECT, DIGEST, CONFIRMATION],
            &mut board,
        )
        .expect("a healthy machine");
        assert_eq!(board.requested, vec![DIGEST.to_string()]);
        assert_eq!(requested.digest(), DIGEST);
        // And the result carries what the STORE said it is, which the caller
        // never supplied.
        assert_eq!(requested.artifact(), ARTIFACT);
        // ~~The confirmed path does not re-run the plan: one check, at the
        // moment of the act.~~ **Inverted 2026-08-26**: `super::plan::resolve`
        // plans on both paths, so `can_request_install` now runs BEFORE the
        // request it guards. The old shape left the confirmed path as the one
        // path the interlock never covered.
        assert_eq!(
            board.planned,
            vec![DIGEST.to_string()],
            "the interlock was skipped on the confirmed path"
        );
    }

    /// A misspelt confirmation is refused rather than treated as absent — the
    /// rule `super::apply::confirmed` owns, asserted through this operation so
    /// that a subject that stopped using it would fail.
    #[test]
    fn a_word_that_is_not_the_confirmation_is_refused_rather_than_ignored() {
        for word in ["confrim", "yes", "--confirm", "CONFIRM", ""] {
            let mut board = FakeBoard::healthy();
            let refusal = perform_apply(
                &[SUBJECT, DIGEST, word],
                &mut board,
            )
            .expect_err("only the exact confirmation word may confirm");
            assert_eq!(refusal.code(), ErrorCode::Arguments, "{word:?}");
            assert_eq!(refusal.subject(), Some(word));
            assert!(board.requested.is_empty());
        }
    }

    // -- apply install: the arguments ------------------------------------------

    /// **THE RULING, as a test.** `ADR-052` §7: there is no form of this
    /// operation that names a path. Every shape a path arrives in is refused
    /// with a sentence that says the capability was removed, and nothing is
    /// planned or delivered.
    #[test]
    fn a_path_is_refused_by_name_and_the_refusal_says_the_form_no_longer_exists() {
        for path in [
            A_PATH,
            "/state/carried/tempmon/payload.rvt",
            "x.rvt",
            "./x.rvt",
            "../../etc/shadow",
            "state/x.rvt",
            "~/x.rvt",
            r"C:\payload.rvt",
        ] {
            let mut board = FakeBoard::healthy();
            let refusal = perform_apply(&[SUBJECT, path, CONFIRMATION], &mut board)
                .expect_err("a path must be refused");
            assert_eq!(refusal.code(), ErrorCode::Arguments, "{path}");
            assert_eq!(refusal.subject(), Some(path));
            assert!(
                refusal.message().contains("ADR-052 §7"),
                "{path}: {}",
                refusal.message()
            );
            assert!(
                refusal
                    .message()
                    .contains("THERE IS NO FORM OF THIS OPERATION THAT NAMES A PATH"),
                "{path}: {}",
                refusal.message()
            );
            assert!(board.requested.is_empty(), "{path} was handed over");
            assert!(board.planned.is_empty(), "{path} reached the store");
        }
    }

    /// The refusal for a path says how to get the artifact onto the machine,
    /// per the plan's §8.3: a refusal that only says *no* leaves an operator
    /// with a board and no route to it.
    #[test]
    fn the_path_refusal_says_how_to_get_the_artifact_there() {
        let mut board = FakeBoard::healthy();
        let refusal = perform_apply(&[SUBJECT, A_PATH, CONFIRMATION], &mut board)
            .expect_err("a path must be refused");
        assert!(
            refusal.message().contains("cargo rivet install"),
            "{}",
            refusal.message()
        );
    }

    /// **A digest that is not a digest is refused as one**, and it is a
    /// different sentence from the path refusal because it is a different
    /// mistake — a truncated copy-paste rather than an out-of-date habit.
    #[test]
    fn something_that_is_neither_a_digest_nor_a_path_is_refused_as_a_digest() {
        for word in [
            &DIGEST[..63],
            &format!("{DIGEST}a"),
            &DIGEST.to_ascii_uppercase(),
            &DIGEST.replace("3f", "zz"),
            "photo-index",
        ] {
            let mut board = FakeBoard::healthy();
            let refusal = perform_apply(&[SUBJECT, word, CONFIRMATION], &mut board)
                .expect_err("not a digest");
            assert_eq!(refusal.code(), ErrorCode::Arguments, "{word}");
            assert!(
                refusal.message().contains("not a BLAKE3 digest"),
                "{word}: {}",
                refusal.message()
            );
            assert!(board.planned.is_empty(), "{word} reached the store");
        }
    }

    /// **Upper case is refused rather than folded**, and the refusal says why:
    /// two spellings of one digest would be two entries in the store.
    #[test]
    fn an_upper_case_digest_is_refused_with_the_reason_rather_than_lowercased() {
        let mut board = FakeBoard::healthy();
        let refusal = perform_apply(
            &[SUBJECT, &DIGEST.to_ascii_uppercase(), CONFIRMATION],
            &mut board,
        )
        .expect_err("upper case");
        assert!(refusal.message().contains("LOWERCASE"), "{}", refusal.message());
        assert!(board.planned.is_empty());
    }

    /// **An artifact is named, never found.** `apply install confirm` names
    /// nothing, and taking `confirm` as the argument would report a delivered
    /// request for a command that said nothing.
    #[test]
    fn the_confirmation_word_alone_is_a_forgotten_argument_rather_than_a_digest() {
        let mut board = FakeBoard::healthy();
        let refusal = perform_apply(
            &[SUBJECT, CONFIRMATION],
            &mut board,
        )
        .expect_err("`apply install confirm` names nothing");
        assert_eq!(refusal.code(), ErrorCode::Arguments);
        assert_eq!(refusal.subject(), Some(CONFIRMATION));
        assert!(board.requested.is_empty());
        assert!(board.planned.is_empty());
    }

    /// No argument at all is an argument error naming what to type, not a
    /// mysterious refusal from an installer that was never reached.
    #[test]
    fn no_digest_is_refused_with_the_command_that_would_have_worked() {
        let mut board = FakeBoard::healthy();
        let refusal = perform_apply(&[SUBJECT], &mut board)
            .expect_err("a digest is required");
        assert_eq!(refusal.code(), ErrorCode::Arguments);
        assert!(refusal.message().contains("hex"), "{}", refusal.message());
        assert!(
            refusal.message().contains("cargo rivet install"),
            "{}",
            refusal.message()
        );
        assert!(board.planned.is_empty());
    }

    /// **One artifact at a time.** There is one inactive slot, so a second is
    /// refused rather than the last one winning.
    #[test]
    fn a_fourth_word_is_refused_because_there_is_only_one_inactive_slot() {
        let mut board = FakeBoard::healthy();
        let other = "aa7c1a9e04b2d85f6a3c9017e2b48d5f1c6a09b3e7d248fa5601c93b8ed72a4f";
        let refusal = perform_apply(
            &[SUBJECT, DIGEST, CONFIRMATION, other],
            &mut board,
        )
        .expect_err("a fourth word must be refused");
        assert_eq!(refusal.code(), ErrorCode::Arguments);
        assert_eq!(refusal.subject(), Some(other));
        assert!(board.requested.is_empty());
    }

    /// **A digest this machine does not hold is refused by name**, on both
    /// paths, and nothing is delivered. This is `ADR-052` §7's other half: the
    /// store is what an install can name, so an install that names something
    /// else stops here rather than at the installer.
    #[test]
    fn a_digest_the_store_does_not_hold_is_refused_and_delivers_nothing() {
        for args in [
            vec![SUBJECT, DIGEST],
            vec![SUBJECT, DIGEST, CONFIRMATION],
        ] {
            let mut board = FakeBoard::holds_nothing();
            let refusal = perform_apply(&args, &mut board)
                .expect_err("the store holds nothing");
            assert_eq!(refusal.code(), ErrorCode::MachineRefused, "{args:?}");
            assert!(
                refusal.message().contains("does not hold that digest"),
                "the port's own words must survive: {}",
                refusal.message()
            );
            assert!(board.requested.is_empty(), "{args:?}");
        }
    }

    // -- apply install: the machine --------------------------------------------

    /// **The refusal that matters most.** A build that cannot reach an
    /// installer refuses on BOTH paths — the plan and the confirmed act — and
    /// nothing is ever delivered.
    #[test]
    fn a_build_with_no_installer_is_refused_on_either_path_and_hands_over_nothing() {
        for args in [
            vec![SUBJECT, DIGEST],
            vec![SUBJECT, DIGEST, CONFIRMATION],
        ] {
            let mut board = FakeBoard::no_installer();
            let refusal = perform_apply(&args, &mut board)
                .expect_err("the installer is unreachable");
            assert_eq!(refusal.code(), ErrorCode::Unsafe, "{args:?}");
            assert!(
                refusal.message().contains("cannot reach an installer"),
                "the port's own words must survive: {}",
                refusal.message()
            );
        }
    }

    /// The two kinds of refusal stay two codes, because an operator does two
    /// different things next.
    #[test]
    fn an_unreachable_installer_and_a_failed_delivery_are_two_different_codes() {
        let mut refused_by_machine = FakeBoard::healthy();
        refused_by_machine.delivered = Err(MachineError::Refused("EROFS".to_string()));
        let refusal = perform_apply(&[SUBJECT, DIGEST, CONFIRMATION], &mut refused_by_machine)
            .expect_err("the machine refused");
        assert_eq!(refusal.code(), ErrorCode::MachineRefused);
        assert_eq!(refusal.message(), "EROFS");
    }

    // -- apply install: the result ---------------------------------------------

    /// **A result never says anything was installed.** It reports a delivered
    /// request, which is all this operation can observe.
    #[test]
    fn a_result_reports_a_request_that_was_delivered_and_nothing_more() {
        let mut board = FakeBoard::healthy();
        let requested = perform_apply(
            &[SUBJECT, DIGEST, CONFIRMATION],
            &mut board,
        )
        .expect("a healthy machine");
        let value = requested.value();
        assert_eq!(field_of(&value, "requested"), Some(Value::Bool(true)));
        assert_eq!(field_of(&value, "installed"), Some(Value::Bool(false)));
        assert_eq!(field_of(&value, "digest"), Some(Value::str(DIGEST)));
        assert_eq!(field_of(&value, "artifact"), Some(Value::str(ARTIFACT)));
        assert_eq!(field_of(&value, "subject"), Some(Value::str(SUBJECT)));
    }

    /// **`next` names the correct poll, because the obvious one is wrong.**
    /// `PLAN.md` §73.2: a poll of `rivet get install | get finished` matched a
    /// record from an install that had finished an hour and twenty minutes
    /// earlier, and the deploy it was watching had never happened. Every field
    /// needed to tell the two apart was in the same answer and the pipeline
    /// discarded them.
    ///
    /// This answer cannot hand back a correlation — it returns before the
    /// installer starts, which is the whole point of `next_after_request` — so
    /// naming read-before-and-compare is the only fix available here.
    #[test]
    fn a_delivered_request_says_how_to_poll_it_and_not_merely_where() {
        let mut board = FakeBoard::healthy();
        let requested = perform_apply(&[SUBJECT, DIGEST, CONFIRMATION], &mut board)
            .expect("a healthy machine");
        let Some(Value::Str(next)) = field_of(&requested.value(), "next") else {
            unreachable!("a result with no `next`")
        };
        assert!(next.contains("`correlation`"), "{next}");
        assert!(next.contains("BEFORE ASKING FOR AN INSTALL"), "{next}");
        assert!(
            next.contains("ALREADY answers `finished: true`"),
            "the trap itself has to be stated, not just the remedy: {next}"
        );
    }

    /// **Neither verb reports who asked, and both say where it is.** `ADR-028`
    /// §4.5 put a principal in the request; the record it reaches is the
    /// supervisor's, in a different correlation chain, so an operator who is not
    /// told that would look for a field here and conclude nothing was recorded.
    #[test]
    fn both_verbs_say_where_who_asked_is_recorded_rather_than_leaving_a_hole() {
        let mut board = FakeBoard::healthy();
        let requested = perform_apply(&[SUBJECT, DIGEST, CONFIRMATION], &mut board)
            .expect("a healthy machine");
        let Some(Value::Str(next)) = field_of(&requested.value(), "next") else {
            unreachable!("a result with no `next`")
        };
        assert!(next.contains("WHO ASKED"), "{next}");
        assert!(next.contains("ADR-028 §4.5"), "{next}");
        assert!(next.contains("rivet-sysd"), "{next}");

        // ...and `get install` says the same thing, whatever the journal held —
        // an answer that only mentioned it for finished installs would leave the
        // case somebody is most likely to be asking about.
        let mut observed = FakeBoard::healthy();
        let Ok(value) = perform_get(&[SUBJECT], &mut observed) else {
            unreachable!("a readable journal")
        };
        let Some(Value::Str(next)) = field_of(&value, "next") else {
            unreachable!("an answer with no `next`")
        };
        assert!(next.contains("WHO ASKED FOR IT IS NOT IN THIS ANSWER"), "{next}");
        assert!(
            next.contains("attribution and not authority"),
            "the pointer did not say what it is not: {next}"
        );
    }

    /// The result's keys are a subsequence of what this module declares, in the
    /// declared order — `ADR-006` §7's *"trusted and wrong"* in the smallest
    /// available form.
    #[test]
    fn the_result_carries_only_fields_this_module_declares_in_order() {
        for slot in [Some(Slot::B), None] {
            let requested = Requested {
                digest: DIGEST.to_string(),
                artifact: ARTIFACT.to_string(),
                slot,
            };
            let keys: Vec<&str> = fields_of(&requested.value())
                .iter()
                .map(|(k, _)| *k)
                .collect();
            let declared: Vec<&str> = APPLY_OUTPUTS.iter().map(|o| o.name).collect();
            assert!(
                super::super::get::in_declared_order(
                    &fields_of(&requested.value()),
                    declared.iter().copied()
                ),
                "{keys:?} is not a subsequence of {declared:?}"
            );
        }
    }

    /// **The slot is the one this machine is NOT running.** An install that
    /// wrote the running slot would overwrite the system performing it.
    #[test]
    fn the_slot_reported_is_always_the_one_this_machine_is_not_running() {
        for (running, expected) in [(Slot::A, Slot::B), (Slot::B, Slot::A)] {
            let mut board = FakeBoard::healthy();
            board.slot = Ok(running);
            let requested = perform_apply(&[SUBJECT, DIGEST, CONFIRMATION], &mut board)
                .expect("a healthy machine");
            assert_eq!(
                requested.slot(),
                Some(expected),
                "running {}",
                crate::operation::trial::slot_word(running)
            );
        }
    }

    /// **A machine that cannot say which slot it booted is NOT refused**, and
    /// the field is omitted rather than guessed. An application install needs
    /// no slot, and a development host has no device tree.
    #[test]
    fn a_machine_with_no_readable_slot_still_delivers_the_request_and_omits_the_field() {
        let mut board = FakeBoard::healthy();
        board.slot = Err(MachineError::Refused("no device tree".to_string()));
        let requested = perform_apply(&[SUBJECT, DIGEST, CONFIRMATION], &mut board)
            .expect("an application install needs no slot");
        assert_eq!(requested.slot(), None);
        assert_eq!(board.requested, vec![DIGEST.to_string()]);
        let value = requested.value();
        assert!(
            field_of(&value, "slot").is_none(),
            "an unknown slot must be omitted, never guessed: {value:?}"
        );
        assert!(
            field_of(&value, "means")
                .map(|v| format!("{v:?}").contains("cannot say which slot"))
                .unwrap_or_default(),
            "{value:?}"
        );
    }

    /// **Installing an application does not run it, and the answer says how to
    /// make it run.**
    ///
    /// It always said the first half. The second half is the 2026-08-30
    /// finding: a cold agent following the README's *"using it"* section
    /// installed an application and it *"would have installed and sat there"* —
    /// no failure, nothing to diagnose, and `--enable` fifty lines away under a
    /// different heading. A machine that says *nothing starts it* and stops
    /// there has described the trap without naming the way out.
    ///
    /// It is asserted on the MACHINE's answer rather than on `cargo rivet`'s
    /// help, because this reaches every client and that reaches one.
    #[test]
    fn the_install_answer_says_what_makes_an_installed_application_run() {
        let said = means(None);
        assert!(said.contains("NOTHING STARTS IT"), "{said}");
        // The authority for `enabled`, and where it lives.
        assert!(said.contains("ADR-015 \u{a7}3.2.2"), "{said}");
        assert!(said.contains("enabled"), "{said}");
        assert!(said.contains("apply edit"), "{said}");
        assert!(said.contains("apply reload"), "{said}");
        // And the one-command form, for a caller on a workstation.
        assert!(said.contains("cargo rivet install --enable"), "{said}");
    }

    /// **The surprising fact reaches the operator**: this answer returns before
    /// the write, so a refusal is still ahead of it.
    #[test]
    fn every_result_says_the_request_was_delivered_and_not_completed() {
        let requested = Requested {
            digest: DIGEST.to_string(),
            artifact: ARTIFACT.to_string(),
            slot: Some(Slot::B),
        };
        let Some(Value::Str(next)) = field_of(&requested.value(), "next") else {
            panic!("no next field");
        };
        assert!(next.contains("DELIVERED"), "{next}");
        assert!(next.contains("has NOT happened"), "{next}");
        assert!(next.contains("rivet get install"), "{next}");
        assert!(next.contains("ADR-029 §9.1"), "{next}");
    }

    // -- get install ------------------------------------------------------------

    #[test]
    fn a_journal_with_no_install_says_so_rather_than_inventing_one() {
        let mut board = FakeBoard::holding(&[other_service("aa", "hello")]);
        let value = get(&mut board, &[SUBJECT]).expect("an empty answer is an answer");
        assert_eq!(field_of(&value, "installs"), Some(Value::Integer(0)));
        assert_eq!(field_of(&value, "returned"), Some(Value::Integer(0)));
        assert!(field_of(&value, "correlation").is_none());
        assert!(field_of(&value, "finished").is_none());
        let Some(Value::Str(next)) = field_of(&value, "next") else {
            panic!("no next")
        };
        assert!(next.contains("HOLDS NO INSTALL"), "{next}");
        // And it names where it looked, so an empty answer is not a shrug.
        assert_eq!(field_of(&value, "source"), Some(Value::str("/state/log")));
    }

    /// **J1.** The ingest socket is world-writable by design (`ADR-022`
    /// §11.1), and any sender may claim `svc = "rivet-deploy"` — only
    /// [`crate::journal::SUPERVISOR_SVC`] is refused. A confined application
    /// exploiting exactly that must not be able to make this machine report a
    /// system update as installed: a record whose `actor` is the kernel's
    /// ingest stamp, never the supervisor's own, must not be counted at all.
    #[test]
    fn a_record_only_claiming_the_installers_name_over_ingest_is_not_counted_as_an_install() {
        let mut board = FakeBoard::holding(&[ingest_claim(
            "forged",
            "uid:4021",
            SERVICE_NAME,
            "installed",
            vec![(OUTCOME_FIELD, crate::journal::Value::Str("installed".into()))],
        )]);
        let value = get(&mut board, &[SUBJECT]).expect("an empty answer is an answer");
        assert_eq!(field_of(&value, "installs"), Some(Value::Integer(0)));
        assert_eq!(field_of(&value, "returned"), Some(Value::Integer(0)));
        assert!(field_of(&value, "correlation").is_none());
        assert!(field_of(&value, "finished").is_none());
    }

    /// **J1, the uid-0 case.** The forgery does not depend on the sender being
    /// unprivileged — the ingest socket stamps `uid:<n>` for **any** sender,
    /// root included, because the kernel's own uid is never the supervisor's
    /// fixed actor. The check that closes J1 must not be mistaken for "refuses
    /// non-root senders": it refuses ingest provenance, full stop.
    #[test]
    fn a_record_ingested_from_uid_zero_is_still_not_counted_as_an_install() {
        let mut board = FakeBoard::holding(&[ingest_claim(
            "forged",
            "uid:0",
            SERVICE_NAME,
            "installed",
            vec![(OUTCOME_FIELD, crate::journal::Value::Str("installed".into()))],
        )]);
        let value = get(&mut board, &[SUBJECT]).expect("an empty answer is an answer");
        assert_eq!(field_of(&value, "installs"), Some(Value::Integer(0)));
    }

    /// **J1.** A genuine install must still be reported, even sitting beside a
    /// forged claim of the same service and the same outcome — the fix tells
    /// the two apart by `actor`, not by refusing the service name outright.
    #[test]
    fn a_genuine_install_is_reported_beside_a_forged_claim_of_the_same_service() {
        let mut board = FakeBoard::holding(&[
            ingest_claim(
                "forged",
                "uid:4021",
                SERVICE_NAME,
                "installed",
                vec![(OUTCOME_FIELD, crate::journal::Value::Str("installed".into()))],
            ),
            record(
                "genuine",
                Severity::Info,
                "the install is finished",
                vec![(
                    OUTCOME_FIELD,
                    crate::journal::Value::Str("refused-payload".into()),
                )],
            ),
        ]);
        let value = get(&mut board, &[SUBJECT]).expect("an install is there");
        assert_eq!(field_of(&value, "installs"), Some(Value::Integer(1)));
        assert_eq!(field_of(&value, "correlation"), Some(Value::str("genuine")));
        assert_eq!(field_of(&value, "outcome"), Some(Value::str("refused-payload")));
    }

    /// **Only this service's records.** A journal is mostly other things, and
    /// an install report that swept in the supervisor's lines would be an
    /// account of the machine rather than of the install.
    #[test]
    fn only_the_installers_records_are_read_and_other_services_are_left_alone() {
        let mut board = FakeBoard::holding(&[
            other_service("aa", "rivet-netd starting"),
            record("c1", Severity::Info, "verifying", vec![]),
            other_service("aa", "a login"),
            record(
                "c1",
                Severity::Info,
                "done",
                vec![(OUTCOME_FIELD, crate::journal::Value::Str("installed".into()))],
            ),
        ]);
        let value = get(&mut board, &[SUBJECT]).expect("an install is there");
        assert_eq!(field_of(&value, "installs"), Some(Value::Integer(1)));
        assert_eq!(field_of(&value, "returned"), Some(Value::Integer(2)));
        assert_eq!(field_of(&value, "correlation"), Some(Value::str("c1")));
    }

    /// **Two installs in one boot must not be spliced.** `cmd/rivet-deploy`
    /// mints one correlation identifier per run for exactly this reason, and
    /// the latest is the one reported.
    #[test]
    fn two_installs_are_two_chains_and_the_latest_is_the_one_reported() {
        let mut board = FakeBoard::holding(&[
            record("first", Severity::Info, "verifying", vec![]),
            record(
                "first",
                Severity::Error,
                "refused",
                vec![(
                    OUTCOME_FIELD,
                    crate::journal::Value::Str("refused-payload".into()),
                )],
            ),
            record("second", Severity::Info, "verifying", vec![]),
            record(
                "second",
                Severity::Info,
                "the install is finished",
                vec![(OUTCOME_FIELD, crate::journal::Value::Str("installed".into()))],
            ),
        ]);
        let value = get(&mut board, &[SUBJECT]).expect("two installs");
        assert_eq!(field_of(&value, "installs"), Some(Value::Integer(2)));
        assert_eq!(field_of(&value, "correlation"), Some(Value::str("second")));
        assert_eq!(field_of(&value, "outcome"), Some(Value::str("installed")));
        assert_eq!(field_of(&value, "returned"), Some(Value::Integer(2)));
        assert_eq!(field_of(&value, "errors"), Some(Value::Integer(0)));
    }

    /// **An unfinished chain is reported as unfinished and never as running.**
    /// That distinction is the whole reason the `next` field exists here.
    #[test]
    fn an_install_with_no_last_record_is_unfinished_and_is_not_called_running() {
        let mut board = FakeBoard::holding(&[
            record("c1", Severity::Info, "verifying", vec![]),
            record("c1", Severity::Info, "writing the slot", vec![]),
        ]);
        let value = get(&mut board, &[SUBJECT]).expect("an install is there");
        assert_eq!(field_of(&value, "finished"), Some(Value::Bool(false)));
        assert!(field_of(&value, "outcome").is_none());
        assert!(field_of(&value, "means").is_none());
        let Some(Value::Str(next)) = field_of(&value, "next") else {
            panic!("no next")
        };
        assert!(next.contains("does NOT mean it is running"), "{next}");
        assert!(next.contains("ADR-017 §3.1.3"), "{next}");
        assert!(
            !next.to_lowercase().contains("still running"),
            "the report must not claim liveness it cannot observe: {next}"
        );
    }

    /// **A token this build does not know is reported verbatim**, and the
    /// answer says so instead of reading it as one of the six it knows.
    #[test]
    fn an_outcome_this_build_does_not_know_is_reported_as_the_machine_wrote_it() {
        let mut board = FakeBoard::holding(&[record(
            "c1",
            Severity::Info,
            "something new",
            vec![(
                OUTCOME_FIELD,
                crate::journal::Value::Str("installed-extension".into()),
            )],
        )]);
        let value = get(&mut board, &[SUBJECT]).expect("an install is there");
        assert_eq!(field_of(&value, "finished"), Some(Value::Bool(true)));
        assert_eq!(
            field_of(&value, "outcome"),
            Some(Value::str("installed-extension"))
        );
        assert!(
            field_of(&value, "means").is_none(),
            "this build must not explain a token it does not know"
        );
        let Some(Value::Str(next)) = field_of(&value, "next") else {
            panic!("no next")
        };
        assert!(next.contains("DOES NOT KNOW"), "{next}");
    }

    /// Every outcome this build knows is explained, and a failure reads as a
    /// failure rather than as a completion.
    #[test]
    fn every_known_outcome_is_explained_and_the_failures_say_they_failed() {
        for outcome in Outcome::ALL {
            let mut board = FakeBoard::holding(&[record(
                "c1",
                Severity::Info,
                "the end",
                vec![(
                    OUTCOME_FIELD,
                    crate::journal::Value::Str(outcome.as_str().into()),
                )],
            )]);
            let value = get(&mut board, &[SUBJECT]).expect("an install is there");
            assert_eq!(
                field_of(&value, "means"),
                Some(Value::str(outcome.means())),
                "{outcome}"
            );
            let Some(Value::Str(next)) = field_of(&value, "next") else {
                panic!("no next")
            };
            if outcome.succeeded() {
                assert!(next.contains("It finished"), "{outcome}: {next}");
            } else {
                assert!(next.contains("It FAILED"), "{outcome}: {next}");
            }
        }
    }

    /// The severities are counted from the chain, not from the journal: an
    /// error somewhere else on the machine is not this install's.
    #[test]
    fn severities_are_counted_within_the_install_and_not_across_the_journal() {
        let mut board = FakeBoard::holding(&[
            record("other", Severity::Error, "a different install failed", vec![]),
            record("c1", Severity::Warn, "F5 applies", vec![]),
            record("c1", Severity::Warn, "a key was supplied", vec![]),
            record(
                "c1",
                Severity::Info,
                "done",
                vec![(OUTCOME_FIELD, crate::journal::Value::Str("installed".into()))],
            ),
        ]);
        let value = get(&mut board, &[SUBJECT]).expect("an install is there");
        assert_eq!(field_of(&value, "warnings"), Some(Value::Integer(2)));
        assert_eq!(field_of(&value, "errors"), Some(Value::Integer(0)));
    }

    /// `get install` takes no second word and **refuses** one rather than
    /// ignoring it — `get storage`'s rule, for `get storage`'s reason.
    #[test]
    fn a_second_word_is_refused_rather_than_ignored() {
        for extra in ["/state/x.rvt", "latest", "all", "1"] {
            let mut board = FakeBoard::holding(&[]);
            let refusal =
                get(&mut board, &[SUBJECT, extra]).expect_err("a second word must be refused");
            assert_eq!(refusal.code(), ErrorCode::Arguments, "{extra}");
            assert_eq!(refusal.subject(), Some(extra));
        }
    }

    /// A journal that cannot be read is a machine refusal carrying what the
    /// system said, never an empty answer — the difference between *"no
    /// install"* and *"no journal"* is the whole question.
    #[test]
    fn a_journal_that_cannot_be_read_refuses_rather_than_answering_nothing() {
        let mut board = FakeBoard::healthy();
        board.journal = Err(MachineError::Refused("EACCES /state/log".to_string()));
        let refusal = get(&mut board, &[SUBJECT]).expect_err("an unreadable journal");
        assert_eq!(refusal.code(), ErrorCode::MachineRefused);
        assert!(refusal.message().contains("EACCES"), "{}", refusal.message());
    }

    /// Lines this build cannot parse are counted, never dropped silently —
    /// `AGENTS.md` §5, and the same rule `get log` obeys.
    #[test]
    fn lines_that_cannot_be_read_are_counted_rather_than_dropped() {
        let mut board = FakeBoard::holding(&[
            "{not json".to_string(),
            record("c1", Severity::Info, "verifying", vec![]),
        ]);
        let value = get(&mut board, &[SUBJECT]).expect("an install is there");
        assert_eq!(field_of(&value, "unreadable"), Some(Value::Integer(1)));
        assert_eq!(field_of(&value, "installs"), Some(Value::Integer(1)));
    }

    /// The records come back as the lines the machine wrote, so a nushell
    /// pipeline reads them without this operation re-encoding anything —
    /// `ADR-022` §3 makes that form authoritative and `get log` takes the same
    /// decision.
    #[test]
    fn the_records_are_the_lines_the_machine_wrote() {
        let line = record("c1", Severity::Info, "verifying", vec![]);
        let mut board = FakeBoard::holding(std::slice::from_ref(&line));
        let value = get(&mut board, &[SUBJECT]).expect("an install is there");
        let Some(Value::List(records)) = field_of(&value, "records") else {
            panic!("no records")
        };
        assert_eq!(records, vec![Value::str(line)]);
    }
}
