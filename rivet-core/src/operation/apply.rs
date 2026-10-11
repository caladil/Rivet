//! **The `apply` verb**, and the EIGHT subjects this build applies.
//!
//! **Do not read the count out of this sentence — count [`subject_table`].** It
//! has said four, five, six and seven, and each time the number was wrong within
//! a day (`AGENTS.md` §7). The table below is likewise a snapshot of what the
//! first four were and is left as written; the authority is the function.
//!
//! `ADR-006` §6.4's verb set is closed and the catalogue is keyed by verb, so
//! this module exists because **a second catalogue key would be an amendment to
//! that section and `AGENTS.md` §1 puts that out of an agent's reach.** One verb,
//! one entry, and the subject is a declared [`Input`] whose value selects the
//! implementation:
//!
//! | Subject | Module | What it does |
//! | :-- | :-- | :-- |
//! | `trial-boot` | [`super::trial`] | arms `ADR-018` §5's one-shot flag. Changes a file on the card; stops nothing |
//! | `power-off` | [`super::stop`] | asks pid 1 to shut down and stay off — `ADR-017` §3.1.10 |
//! | `restart` | [`super::stop`] | asks pid 1 to shut down and come back on the default slot |
//! | `trial-restart` | [`super::stop`] | asks pid 1 to shut down and come back once in the armed slot |
//!
//! `ADR-006` §8.4 predicted this shape and priced it: *"`subject` is a declared
//! input rather than a second axis of the catalogue, so a second subject is an
//! added match arm."* Three arrived at once and the price was as quoted.
//!
//! # The catalogue entry moved here, and that is the whole of the restructure
//!
//! It was `trial::OPERATION`, declared beside the only implementation there was.
//! With four subjects behind one verb, *the verb* is the thing that has an
//! implementation — this module — and the entry is declared beside it. Each
//! subject module still declares its own `Field` list, and [`subject_table`]
//! below is what ties the four to the one entry.
//!
//! # The two costs of one entry for four subjects, said rather than discovered
//!
//! **1. The second positional input means different things to different
//! subjects.** `rivet apply trial-boot b confirm` puts the slot second;
//! `rivet apply power-off confirm` puts the confirmation second, because the
//! three stop subjects have no slot. So [`OPERATION`]'s `slot` input is optional
//! and its description says exactly which subject requires it. An operator never
//! sees the wart — each command reads naturally — and an *agent* reading the
//! catalogue does, which is the wrong way round for `ADR-006` §7.
//!
//! **2. The declared outputs were a union, written by hand.** No single result
//! carries all ten fields, and the flat list that named them all described none
//! of the four exactly — weaker than §7's implied *"these are the fields you
//! get"*. The alternative this module named — *outputs per subject, a change to
//! [`super::Operation`]'s type and therefore to §7 itself* — **was ruled and
//! built on 2026-08-24**. The entry now publishes [`subject_table`] with each
//! subject's own outputs, and the flat union is generated from it by
//! [`super::Operation::outputs`]. So cost 2 is paid rather than borne, and the
//! hand-maintained list that `AGENTS.md` §7 would eventually have caught stale
//! is gone.
//!
//! **The first is still the maintainer's to overturn**, and the overturn is to
//! key the catalogue by verb *and* noun, which §6.4 says is done by amending the
//! ADR. `ADR-006` §8.5 records this.

use super::edit;
use super::install;
use super::machine::{Board, Stop};
// `ADR-006` §8.10's subjects, 2026-08-29.
use super::create;
use super::enroll_member;
use super::install_secret;
use super::permissions;
use super::restore_secret;
use super::remove;
use super::remove_member;
use super::rename;
use super::rename_phone;
use super::repo_delete;
use super::revoke_phone;
use super::passkey_code;
use super::input;
use super::reload::{self, Reloading};
// `ADR-017` §6.3.2, 2026-08-29.
use super::service_restart;
use super::service_hold;
use super::clock_floor;
use super::stop;
use super::trial;
use super::{ErrorCode, Input, Operation, OperationError, Results, Subject, Verb};
use super::uninstall;
use super::uninstall_library;
use crate::value::Value;

/// The word that turns a plan into a change. `ADR-012` §7.1.
///
/// **One definition for all four subjects**, which is `ADR-011` in miniature: a
/// second copy of this word is a second answer to *what does an operator type to
/// say yes*, and the two would eventually disagree in a build where only one of
/// them was tested.
///
/// A literal word rather than `--confirm`, and the difference is the one §7.1
/// draws: a flag is something *"anyone can simply pass"*, and more practically a
/// flag would be parsed by `cmd/rivet` and no operation would ever see it. A
/// declared input is part of the catalogue, so every adapter binds it and the
/// network API of `ADR-006` §5.1 cannot quietly omit it.
pub const CONFIRMATION: &str = "confirm";

// ---------------------------------------------------------------------------
// The second tier — ADR-012 §7.3, ruled 2026-08-29
// ---------------------------------------------------------------------------

/// **The word that acknowledges a specific hazard**, beside [`CONFIRMATION`].
///
/// `ADR-012` §7.3, ruled by the maintainer, Joel Lindell, 2026-08-29
/// (`.rulings/` R-029): a change that can do damage is confirmed by a line led
/// by `<WARNING>`, ending in a default-no choice, `(N/yes)`. The doctrine that
/// decides *when* matters more than the mechanism (`.rulings/` R-030): the
/// WARNING tier is kept for the exceptional case, the operating system's chance
/// to object before an operator does themselves harm.
///
/// **`confirm` is the same single word for `apply restart`, `apply install` and
/// every routine `apply edit`.** A word typed by reflex is not a safety
/// mechanism for the dangerous case: plan-before-apply gets the warning in
/// front of the operator, and then terminates in a token that carries no
/// information about what is being accepted.
///
/// So the second tier's token **echoes the specific hazard**, which is
/// `super::edit`'s `--expect` idiom exactly: that operation makes a caller state
/// what it believes is *there*, and this makes a caller state what it believes
/// it is about to *break*. It cannot be appended blindly the way a fixed word
/// can, because producing it requires having read the plan.
pub const ACCEPT_FLAG: &str = "--accept";

/// **One exceptional consequence, in the words a caller must echo back.**
///
/// See [`ACCEPT_FLAG`]. The rarity is the whole of the value — `PLAN.md` §94.2
/// found `ADR-029` §9.3's F5 warning firing on *every* install targeting slot A
/// and concluded that it *"became wallpaper"*, which is this failure discovered
/// before the doctrine was stated. **A warning that fires every time is not a
/// warning**, so the test for reaching this type is never *"is this important"*
/// or *"would I want to know"* — both of those are answered by the plan's prose
/// — but *"is something exceptional happening, and is this the machine's chance
/// to object before the operator does themselves harm."*
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hazard {
    /// **What breaks**, as one clause, in the second person's absence: *"the
    /// application will not start until it is reinstalled"*. This is the string
    /// the caller must echo, so it is written to be typed: no punctuation that a
    /// shell eats, no capitals to reproduce, and stable for a given hazard.
    pub phrase: String,
    /// Why, at whatever length it takes. **Not echoed** — it is the plan's
    /// prose, and a hazard whose reason had to be retyped would be one nobody
    /// reads.
    pub because: String,
}

impl Hazard {
    /// The maintainer's own rendering, for a terminal (`.rulings/` R-029): the
    /// word `<WARNING>`, the hazard's phrase as a question, and a choice that
    /// defaults to no, `(N/yes)`.
    ///
    /// **Default no, and the whole word.** It is a *rendering* and not a prompt:
    /// `ADR-006` §3's co-equality and `ADR-012`'s first-class agent mean the
    /// mechanism cannot BE a TTY prompt, because there is no TTY over the
    /// control plane. One mechanism, two renderings — this is the human one, and
    /// [`Self::accept_words`] is the other.
    pub fn warning(&self) -> String {
        format!("<WARNING> {phrase}? (N/yes)", phrase = self.phrase)
    }

    /// The two words a caller adds to accept it, ready to be spliced into the
    /// command a plan says to repeat.
    ///
    /// Single-quoted for `super::edit::retype`'s reason and its evidence: a
    /// POSIX `sh` and `nushell` both read a single-quoted string literally, and
    /// a plan whose command could not be pasted back was a real defect found by
    /// running one on a guest on 2026-08-24.
    pub fn accept_words(&self) -> [String; 2] {
        [
            ACCEPT_FLAG.to_string(),
            format!("'{phrase}'", phrase = self.phrase),
        ]
    }
}

/// **Read the acknowledgement.** One implementation for every subject that can
/// reach the second tier.
///
/// `confirming` is whether [`CONFIRMATION`] was given, and it decides only
/// whether a *missing* acknowledgement is a refusal — on the plan path it is
/// not, because the plan is what tells the caller the phrase.
///
/// # A superfluous acknowledgement is refused, and that is the point
///
/// A caller that appends `--accept` to everything has turned the second tier
/// back into the first. So an acknowledgement given for a change that has no
/// hazard is an **argument error**, by name — the same rule
/// [`confirmed`] applies to `confrim`, for the same reason: a token that is
/// accepted when it means nothing is a token that means nothing.
///
/// # Errors
///
/// [`ErrorCode::Arguments`] for an acknowledgement that is superfluous or does
/// not match, and [`ErrorCode::ConfirmationRequired`]'s sibling
/// [`ErrorCode::Unsafe`] for one that is missing on a confirmed command —
/// `Unsafe`, because the machine is objecting rather than the arguments being
/// malformed.
pub(super) fn accepted(
    given: Option<&str>,
    hazard: Option<&Hazard>,
    confirming: bool,
) -> Result<(), OperationError> {
    match (given, hazard) {
        (None, None) => Ok(()),
        (Some(given), None) => Err(OperationError::about(
            ErrorCode::Arguments,
            ACCEPT_FLAG,
            format!(
                "`{ACCEPT_FLAG} {given:?}` was given and THIS CHANGE HAS NO WARNING to accept, \
                 so nothing has been done. This is refused rather than ignored on purpose: \
                 `{ACCEPT_FLAG}` exists for the rare change where this machine objects before \
                 you do yourself harm (ADR-012 §7.3), and a caller that appends it to \
                 everything has turned it back into `{CONFIRMATION}`. Leave it out."
            ),
        )),
        (None, Some(hazard)) if confirming => Err(OperationError::about(
            ErrorCode::Unsafe,
            ACCEPT_FLAG,
            format!(
                "{warning} NOTHING HAS BEEN DONE. `{CONFIRMATION}` alone does not accept this: \
                 it is the same word you type for a restart and for every routine edit, and \
                 this is not one of those. {because} TO PROCEED, ADD THE ACKNOWLEDGEMENT — \
                 `{ACCEPT_FLAG} '{phrase}'` — which repeats what breaks, in those words, so \
                 that saying yes requires having read what you are saying yes to.",
                warning = hazard.warning(),
                because = hazard.because,
                phrase = hazard.phrase,
            ),
        )),
        (None, Some(_)) => Ok(()),
        (Some(given), Some(hazard)) if given == hazard.phrase => Ok(()),
        (Some(given), Some(hazard)) => Err(OperationError::about(
            ErrorCode::Arguments,
            ACCEPT_FLAG,
            format!(
                "`{ACCEPT_FLAG}` carries {given:?} and this change's warning is {expected:?}, so \
                 NOTHING HAS BEEN DONE. It must match exactly, because the point of it is that \
                 it cannot be produced without having read the plan. If the text you gave is \
                 the warning of some OTHER change, then this is not the command you meant.",
                expected = hazard.phrase,
            ),
        )),
    }
}

/// **Every subject `apply` accepts, and what each one's result carries.**
///
/// The one place this verb's subjects are enumerated. [`subjects`] reads the
/// names off it, [`super::Operation::outputs`] generates the published union
/// from it, and [`perform`] dispatches on the same two implementations — so a
/// subject that exists and is not catalogued cannot happen, and neither can the
/// reverse.
///
/// A function rather than a `const` for the reason [`super::Results::PerSubject`]
/// gives: [`Stop::ALL`] is already the one place the three stop words are
/// written, and a const table would be a second.
pub(super) fn subject_table() -> Vec<Subject> {
    let mut table = vec![Subject {
        name: trial::SUBJECT,
        outputs: trial::OUTPUTS,
    }];
    // All three stops produce the same shape, so all three name **one**
    // declaration rather than carrying three copies that could disagree.
    table.extend(Stop::ALL.iter().map(|stop| Subject {
        name: stop.as_str(),
        outputs: stop::OUTPUTS,
    }));
    // The fifth subject, added by the maintainer's install-request ruling. It is `stop`'s shape
    // rather than `trial`'s: a request delivered to another process, which
    // this one cannot then observe.
    table.push(Subject {
        name: install::SUBJECT,
        outputs: install::APPLY_OUTPUTS,
    });
    // The sixth, 2026-08-24 — `ADR-006` §8.9, and the first subject of any verb
    // that **does the thing itself**. `trial` changes a file on the card and the
    // other four hand a request to another process; this one replaces a file and
    // returns having done it, which is why its result has no `requested` in it.
    table.push(Subject {
        name: edit::SUBJECT,
        outputs: edit::OUTPUTS,
    });
    // The seventh, 2026-08-25 — `crate::uninstall`, which is `ADR-015` §7 the
    // other way round. `install`'s shape rather than `edit`'s: a request handed
    // to another process, which this one cannot then observe. That other
    // process is the SUPERVISOR here rather than the installer, because an
    // application may be running and nothing else on the machine can stop a
    // service.
    table.push(Subject {
        name: uninstall::SUBJECT,
        outputs: uninstall::OUTPUTS,
    });
    // `uninstall`'s twin for a LIBRARY build — `crate::uninstall_library`. The
    // same shape for the same reason: a request handed to the supervisor,
    // which checks every installed application's declared closure again before
    // it removes anything and is the only thing that can recompose the library
    // set every service is given without a reboot.
    table.push(Subject {
        name: uninstall_library::SUBJECT,
        outputs: uninstall_library::OUTPUTS,
    });
    // The eighth, 2026-08-26 — `ADR-032` §3.2's reload, and the sending half of
    // a signal `cmd/rivet-sysd` could already receive and nothing could send.
    // `stop`'s shape: a request delivered to another process, which this one
    // cannot then observe — and here that is the ADR's own limit rather than
    // this operation's, since a signal carries no reply.
    // The ninth, 2026-08-26 - `crate::repo_delete`, and the only subject in
    // this catalogue that DESTROYS DATA on purpose. `uninstall`'s shape: a
    // request handed to the supervisor, which owns the repository root and is
    // the only thing that can see the two conditions this process cannot.
    table.push(Subject {
        name: repo_delete::SUBJECT,
        outputs: repo_delete::OUTPUTS,
    });
    table.push(Subject {
        name: reload::SUBJECT,
        outputs: reload::OUTPUTS,
    });
    // **`ADR-017` §6.3.2's converge-only service control, 2026-08-29.** The
    // second operation ever to cross the supervisor's socket and the first that
    // changes anything — `reload`'s shape, with a reply. Named
    // `service-restart` and not `restart` because `restart` is already a
    // subject here and means *restart the machine*; `service_restart`'s
    // `the_two_restarts_cannot_be_confused` holds the two apart.
    table.push(Subject {
        name: service_restart::SUBJECT,
        outputs: service_restart::OUTPUTS,
    });
    // **Stop one application and hold it stopped, and release that hold** —
    // `ADR-017` §6, `.rulings/` R-183. `service-restart`'s shape and its
    // socket, with a hold the supervisor keeps until `service-start` or the
    // next boot. Both name one declaration of outputs, as the three stops do.
    table.push(Subject {
        name: service_hold::STOP_SUBJECT,
        outputs: service_hold::OUTPUTS,
    });
    table.push(Subject {
        name: service_hold::START_SUBJECT,
        outputs: service_hold::OUTPUTS,
    });
    // **`ADR-021` §3's floor, lowered by the operator.** `service-restart`'s
    // shape and its socket: the supervisor owns the stored clock and is the
    // only writer of it, so the request crosses to it and the answer comes back.
    table.push(Subject {
        name: clock_floor::SUBJECT,
        outputs: clock_floor::OUTPUTS,
    });
    // **`ADR-012` §9's press, 2026-09-01.** The subject that makes this
    // project's own title true: until it existed an agent could install to a
    // machine, reconfigure it and reboot it, and could not press ENTER on it.
    //
    // `reload`'s shape — a request delivered to another process, which this one
    // cannot then observe — because a press carries no reply, exactly as a real
    // one does not.
    table.push(Subject {
        name: input::SUBJECT,
        outputs: input::OUTPUTS,
    });
    // **`ADR-006` §8.10's six, 2026-08-29** — the maintainer's ruling that
    // §8.9's exclusion of file management was an instruction about *shape*
    // rather than a prohibition. Each is a separate subject with its own plan
    // and its own refusals, which is what §8.9 asked for; none of them is a flag
    // on `edit`, and `edit` is unchanged.
    //
    // Two of them DESTROY, which no subject of this verb did before except
    // `repo-delete` — and unlike that one, these do the thing themselves rather
    // than asking the supervisor. So the plan is the whole of the safety
    // property and the confirmation is the only gate.
    table.push(Subject {
        name: create::FILE_SUBJECT,
        outputs: create::OUTPUTS,
    });
    table.push(Subject {
        name: create::DIRECTORY_SUBJECT,
        outputs: create::OUTPUTS,
    });
    table.push(Subject {
        name: permissions::MODE_SUBJECT,
        outputs: permissions::OUTPUTS,
    });
    table.push(Subject {
        name: permissions::OWNER_SUBJECT,
        outputs: permissions::OUTPUTS,
    });
    table.push(Subject {
        name: rename::SUBJECT,
        outputs: rename::OUTPUTS,
    });
    table.push(Subject {
        name: remove::FILE_SUBJECT,
        outputs: remove::OUTPUTS,
    });
    table.push(Subject {
        name: remove::DIRECTORY_SUBJECT,
        outputs: remove::OUTPUTS,
    });
    // **`install-secret`, 2026-09-01**, and it is the file surface's first
    // subject that carries *bytes* and an *owner* in one act. Every subject
    // above either writes text as root or changes an attribute of a file that
    // already exists; putting a per-machine credential where an application can
    // read it needed all three, and two of the three succeeding left a
    // credential nothing could read and nothing reporting it. See
    // `super::install_secret` for the whole argument.
    table.push(Subject {
        name: install_secret::SUBJECT,
        outputs: install_secret::OUTPUTS,
    });
    // **`restore-secret`, 2026-09-14** — `ADR-030` §4, `.rulings/` R-183 D5.
    // `install-secret`'s act with the path composed under `/state/secrets` and
    // the owner fixed at root: the board half of `cargo rivet restore-secrets`.
    table.push(Subject {
        name: restore_secret::SUBJECT,
        outputs: restore_secret::OUTPUTS,
    });
    // **`enroll-member`, 2026-09-22** — `ADR-059` §6. `install-secret`'s act
    // with the path composed under a fleet server's `register/devices/` and the
    // owner fixed at `rivet-fleetd`'s uid: the control-plane half of member
    // enrollment that `ADR-034` §2 promised and nothing built.
    table.push(Subject {
        name: enroll_member::SUBJECT,
        outputs: enroll_member::OUTPUTS,
    });
    // **`remove-member`, 2026-09-22** — `ADR-059` §6, the exact inverse of
    // `enroll-member` above. `remove-file`'s act with the path composed the
    // SAME way `enroll-member` composes it, so an operator never hand-deletes
    // a register file to retract a member.
    table.push(Subject {
        name: remove_member::SUBJECT,
        outputs: remove_member::OUTPUTS,
    });
    // **`rename-phone`, 2026-09-27** — `ADR-061` §6. `enroll-member`'s act
    // with the path composed under a fleet server's `register/phones/`, the
    // owner fixed at `rivet-fleetd`'s uid, and REQUIRING the record already
    // exist: the control-plane half of `cargo rivet fleet phones rename`.
    table.push(Subject {
        name: rename_phone::SUBJECT,
        outputs: rename_phone::OUTPUTS,
    });
    // **`revoke-phone`, 2026-09-27** — `ADR-061` §6, the exact inverse of
    // `rename-phone` above. `remove-member`'s act with the path composed the
    // SAME way `rename-phone` composes it.
    table.push(Subject {
        name: revoke_phone::SUBJECT,
        outputs: revoke_phone::OUTPUTS,
    });
    // **`passkey-code`** — `ADR-063` §4. `install-secret`'s act with the
    // path composed under `rivet-proxyd`'s data directory and the bytes drawn
    // on the machine: the one-time code that enrolls a first passkey.
    table.push(Subject {
        name: passkey_code::SUBJECT,
        outputs: passkey_code::OUTPUTS,
    });
    table
}

/// Every subject `apply` accepts, in the order the catalogue names them.
pub fn subjects() -> Vec<&'static str> {
    subject_table().into_iter().map(|s| s.name).collect()
}

/// The catalogue entry. Declared beside the implementation of the *verb* — see
/// this module's documentation for why that is no longer a subject's module.
pub(super) const OPERATION: Operation = Operation {
    verb: Verb::Apply,
    summary: "Arm a one-shot trial boot of the other A/B slot, ask this machine to power off, \
              restart, or restart once into the armed slot, hand a signed .rvt to the installer, \
              change one file, remove an installed application or an installed library, make \
              this machine re-read its \
              configuration, restart ONE SERVICE without restarting the machine, or stop one \
              application and hold it stopped until it is started again. Confirms first, every \
              time.",
    inputs: &[
        Input {
            name: "subject",
            required: true,
            several: false,
            description: "What to apply. This build accepts `trial-boot`, `power-off`, `restart`, \
                          `trial-restart`, `install`, `edit`, `uninstall`, \
                          `uninstall-library`, `reload`, \
                          `repo-delete`, `service-restart`, `service-stop`, `service-start`, \
                          `input`, and ADR-006 §8.10's file surface: \
                          `create-file`, `create-directory`, `set-mode`, `set-owner`, `rename`, \
                          `remove-file`, `remove-directory`, `install-secret` and \
                          `restore-secret`; `enroll-member`, which writes one fleet \
                          member's record into this fleet server's register; \
                          `remove-member`, which retracts one, the exact inverse; \
                          `rename-phone`, which rewrites one enrolled phone's record; \
                          `revoke-phone`, which removes one, the exact inverse; \
                          `passkey-code`, which mints the one-time code that enrolls a \
                          first passkey in this machine's proxy; and `clock-floor-reset`, \
                          which makes the running clock this machine's clock floor. NOTE that \
                          `restart` restarts the MACHINE and `service-restart` restarts ONE \
                          SERVICE.",
        },
        Input {
            name: "slot",
            required: false,
            several: false,
            description: "`trial-boot` REQUIRES this and no other subject accepts it: which of \
                          ADR-018 §4's two slots to trial, `a` or `b`. It may not be the slot \
                          this machine is running. `install` REQUIRES this position too and \
                          means something else by it — the ABSOLUTE path of the .rvt to install; \
                          `edit` REQUIRES it too and means the ABSOLUTE path of the file to \
                          change. `uninstall-library` REQUIRES it too and means the LIBRARY TO \
                          REMOVE: a PRODUCT NAME, which selects every build of that product \
                          under /state/lib, or a FULL REFERENCE `<name>-<version>-<64 hex>`, \
                          which selects exactly one. `service-restart` REQUIRES it too and means \
                          the NAME OF ONE \
                          SERVICE, as `get status` spells it, and so does it for `service-stop` \
                          and `service-start`. `input` REQUIRES it too and means ONE ACTION - \
                          up, down, left, right, enter, back, long-back or capture (ADR-012 \
                          §9.3; there is deliberately no way to type text). \
                          `install-secret` REQUIRES it too and means the NAME OF ONE \
                          APPLICATION — never a path: that subject COMPOSES the path from the \
                          name, which is what confines it to that application's own data \
                          directory. `restore-secret` REQUIRES it too and means the NAME OF ONE \
                          FILE inside /state/secrets — never a path, for the same reason. \
                          `enroll-member` REQUIRES it too and means the member's REGISTER \
                          IDENTIFIER (sixteen lowercase hex characters) — never a path: that \
                          subject COMPOSES the path from it, under this fleet server's \
                          register/devices/. `remove-member` REQUIRES it too and means the SAME \
                          thing: the member's REGISTER IDENTIFIER, composed into the SAME path, \
                          to retract rather than write it. \
                          `rename-phone` REQUIRES it too and means the phone's IDENTITY \
                          (43 characters of the base64url alphabet, ADR-061 §5) — never a \
                          path: that subject COMPOSES the path from it, under this fleet \
                          server's register/phones/. `revoke-phone` REQUIRES it too and means \
                          the SAME thing: the phone's IDENTITY, composed into the SAME path, to \
                          retract rather than rewrite it. \
                          For the three stop subjects this \
                          position carries the confirmation instead — see this operation's \
                          `confirmation` input.",
        },
        Input {
            name: "change",
            required: false,
            // **The first several-word input in this catalogue.** See
            // `super::Input::several` for what declaring one costs, and
            // `ADR-006` §8.9 for why the alternative — four positional inputs
            // describing flag words — would have been a catalogue that lies.
            several: true,
            description: "`edit`, `install-secret`, `restore-secret` and `rename-phone` REQUIRE \
                          this and no other \
                          subject accepts \
                          it, and unlike every other input here it is SEVERAL WORDS. For \
                          `edit`, either `--expect <text> --replace <text>`, which replaces text \
                          that must occur EXACTLY ONCE in the file and refuses when it occurs \
                          zero times or more than once; or `--append <text>`, which adds text to \
                          the end of the file and creates the file (mode 0600) and its parent \
                          directories if they are not there. The two are mutually exclusive, \
                          there is no `=` spelling, and every word that is neither of these nor \
                          the confirmation is refused by name. For `install-secret` it is \
                          `<name> --hex <hex>`: the FILE NAME the secret gets inside the \
                          application's data directory, then its content as an EVEN number of \
                          hexadecimal characters. Hex and not base64 because base64 has more \
                          than one spelling of one value and this is the one place a misread \
                          value is a wrong credential; an empty secret is `--hex \"\"`. For \
                          `restore-secret` it is `--hex <hex>` alone: the name was the word \
                          before it. For `enroll-member` it is `--hex <hex>` alone too: the \
                          hex is the member's register record (its TOML), and the id was the \
                          word before it. For `rename-phone` it is `--hex <hex>` alone as well: \
                          the hex is the phone's WHOLE register record (its TOML) with the \
                          label changed, rendered on the workstation, and the identity was the \
                          word before it.",
        },
        Input {
            // One spelling, and `Operation::confirms` reads the same constant
            // so that `get catalogue` can say which verbs change the machine
            // without anyone maintaining a list of them (`ADR-011` §3).
            name: super::CONFIRMATION_INPUT,
            required: false,
            several: false,
            description: "The literal word `confirm`, as the LAST word of the command. Without \
                          it every subject returns its plan and changes nothing (ADR-012 §7.1). \
                          A misspelling is refused rather than read as absence.",
        },
    ],
    results: Results::PerSubject(subject_table),
    errors: &[
        ErrorCode::UnknownSubject,
        ErrorCode::Arguments,
        ErrorCode::ConfirmationRequired,
        ErrorCode::Unsafe,
        ErrorCode::MachineRefused,
    ],
};

/// Read the confirmation from the word that should be the last one.
///
/// **One implementation for all four subjects**, and the rule it carries is the
/// one `trial` established: *a third word that is not the confirmation is refused
/// rather than ignored.* An operator who typed `confrim` must not be shown the
/// plan again as though they had typed nothing, because the second refusal would
/// look identical to the first and they would repeat the typo — and on
/// `trial-restart` they would then power-cycle a board wondering why nothing
/// happened.
pub(super) fn confirmed(word: Option<&str>) -> Result<bool, OperationError> {
    match word {
        None => Ok(false),
        Some(CONFIRMATION) => Ok(true),
        Some(other) => Err(OperationError::about(
            ErrorCode::Arguments,
            other,
            format!(
                "the last word confirms the change and must be exactly `{CONFIRMATION}`. Leave \
                 it out to see the plan."
            ),
        )),
    }
}

/// Perform `apply`. Arity is [`super::perform`]'s; the subject selects the
/// implementation and nothing else here decides anything.
/// **It takes the whole board rather than one port**, since 2026-08-24.
///
/// `trial` and `stop` need [`super::machine::Machine`] and `install` needs
/// [`super::machine::Deployer`] **and** `Machine` — it reports which slot a
/// system update would land in, which is `running_slot`'s. So the dispatcher
/// takes the object every adapter already has and narrows at each call, which
/// is the shape `super::perform` uses one level up and for the same reason:
/// `get log` still cannot reboot anything, and `trial-boot` still cannot hand
/// a payload to an installer.
pub(super) fn perform(
    arguments: &[&str],
    board: &mut dyn Board,
) -> Result<Value, OperationError> {
    let subject = arguments.first().copied().unwrap_or_default();

    // **The catalogue gates the dispatch, and it did not until 2026-08-26.**
    //
    // This module's own documentation claims *"a subject that exists and is not
    // catalogued cannot happen, and neither can the reverse"* — and only the
    // second half was true. `subjects()` is generated from [`subject_table`], so
    // a listed subject that had no arm was caught; an **arm with no entry** was
    // not, because the chain below reaches its implementation without consulting
    // anything. `apply reload` shipped in exactly that state for the length of
    // one commit: reachable, absent from `rivet --version`, and absent from the
    // union `super::Operation::outputs` generates.
    //
    // One `contains` closes it. An arm added without a table entry now refuses
    // with `E_UNKNOWN_SUBJECT` — loudly, at the first test that performs it —
    // rather than working and being invisible.
    if !subjects().contains(&subject) {
        return Err(unknown(subject));
    }

    if subject == trial::SUBJECT {
        return trial::perform(arguments, board).map(|armed| armed.value());
    }
    if let Some(how) = stop::parse_subject(subject) {
        return stop::perform(how, arguments, board).map(|s| s.value());
    }
    if subject == install::SUBJECT {
        return install::perform_apply(arguments, board).map(|requested| requested.value());
    }
    if subject == uninstall::SUBJECT {
        return uninstall::perform(arguments, board);
    }
    if subject == uninstall_library::SUBJECT {
        return uninstall_library::perform(arguments, board);
    }
    if subject == edit::SUBJECT {
        return edit::perform(arguments, board).map(|edited| edited.value());
    }
    if subject == repo_delete::SUBJECT {
        return repo_delete::perform(arguments, board);
    }
    if subject == reload::SUBJECT {
        return reload::perform(arguments, board).map(Reloading::value);
    }
    // `ADR-017` §6.3.2. The `true` is *this was the confirmed apply* — the
    // plan path never reaches here, because an unconfirmed change returns its
    // plan as a refusal (`super::plan::resolve`).
    if subject == service_restart::SUBJECT {
        return service_restart::perform(arguments, board).map(|r| r.value(true));
    }
    // `ADR-017` §6's hold. The `true` is `service-restart`'s: an unconfirmed
    // change returns its plan as a refusal and never reaches here.
    if let Some(kind) = service_hold::Kind::parse(subject) {
        return service_hold::perform(kind, arguments, board).map(|h| h.value(true));
    }
    // `ADR-021` §3. The `true` is `service-restart`'s.
    if subject == clock_floor::SUBJECT {
        return clock_floor::perform(arguments, board).map(|f| f.value(true));
    }
    // `ADR-012` §9. A press, delivered to the launcher and not observable from
    // here — `reload`'s shape, and for the same reason: a press carries no
    // reply, exactly as a real one does not.
    if subject == input::SUBJECT {
        return input::perform(arguments, board).map(input::Pressed::value);
    }
    // `ADR-006` §8.10's six. Each parses its own words and owns its own
    // refusals — this chain decides nothing except which one runs.
    if let Some(kind) = create::Kind::parse(subject) {
        return create::perform(kind, arguments, board).map(|created| created.value());
    }
    if let Some(kind) = permissions::Kind::parse(subject) {
        return permissions::perform(kind, arguments, board).map(|changed| changed.value());
    }
    if subject == rename::SUBJECT {
        return rename::perform(arguments, board).map(|renamed| renamed.value());
    }
    if let Some(kind) = remove::Kind::parse(subject) {
        return remove::perform(kind, arguments, board).map(|removed| removed.value());
    }
    // 2026-09-01. Composes its own path from the application's name rather than
    // taking one, which is what confines it to one directory.
    if subject == install_secret::SUBJECT {
        return install_secret::perform(arguments, board).map(|installed| installed.value());
    }
    // 2026-09-14. Composes its path under `/state/secrets` from the name.
    if subject == restore_secret::SUBJECT {
        return restore_secret::perform(arguments, board).map(|restored| restored.value());
    }
    // 2026-09-22. Composes its path under the fleet server's register/devices/
    // from the member id, owner fixed at rivet-fleetd's uid (ADR-059 §6).
    if subject == enroll_member::SUBJECT {
        return enroll_member::perform(arguments, board).map(|enrolled| enrolled.value());
    }
    // 2026-09-22. The exact inverse of enroll-member, above: composes the SAME
    // path from the member id and retracts it (ADR-059 §6).
    if subject == remove_member::SUBJECT {
        return remove_member::perform(arguments, board).map(|removed| removed.value());
    }
    // 2026-09-27. Composes its path under the fleet server's register/phones/
    // from the phone's identity, owner fixed at rivet-fleetd's uid, and
    // requires the record already exist (ADR-061 §6).
    if subject == rename_phone::SUBJECT {
        return rename_phone::perform(arguments, board).map(|renamed| renamed.value());
    }
    // 2026-09-27. The exact inverse of rename-phone, above: composes the SAME
    // path from the phone's identity and retracts it (ADR-061 §6).
    if subject == revoke_phone::SUBJECT {
        return revoke_phone::perform(arguments, board).map(|revoked| revoked.value());
    }
    // Mints the proxy's one-time passkey enrollment code (ADR-063 §4).
    if subject == passkey_code::SUBJECT {
        return passkey_code::perform(arguments, board).map(|issued| issued.value());
    }
    // Unreachable through the gate above, which already refused anything not in
    // `subjects()`. Written as the same refusal rather than an `unreachable!`
    // for `super::perform`'s reason: a panic in `rivet-core` would be a panic in
    // pid 1's dependency tree, and if the table and this chain ever disagree the
    // honest answer is still that this build applies the subjects it lists.
    Err(unknown(subject))
}

/// **The one refusal for a subject this build does not apply**, naming the whole
/// vocabulary rather than only that this word was wrong.
///
/// One function because there are two callers — the gate at the top of
/// [`perform`] and its unreachable tail — and two copies of a sentence listing
/// the subjects would be `ADR-011` §3 in the small.
fn unknown(subject: &str) -> OperationError {
    OperationError::about(
        ErrorCode::UnknownSubject,
        subject,
        format!(
            "`{}` applies {} in this build. The desired-state apply of ADR-006 §6 is not built — \
             `rivet --version` publishes the operation catalogue, which is generated from the \
             types and is the authority for what exists.",
            Verb::Apply,
            subjects()
                .iter()
                .map(|s| format!("`{s}`"))
                .collect::<Vec<_>>()
                .join(", "),
        ),
    )
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    // The two tests that stood here until 2026-08-24 — that every subject's
    // fields are a subsequence of the published union, and that nothing is
    // published no subject carries — are gone because the union is no longer
    // written down. `super::Operation::outputs` generates it from
    // [`subject_table`], so both properties are now facts about the generator
    // rather than about this module, and
    // `every_subjects_fields_survive_the_generated_union` in `super` checks them
    // once for the whole catalogue instead of once per verb.

    /// The subject list is generated from the implementations, so a subject that
    /// exists and is not listed cannot happen — and neither can the reverse.
    #[test]
    fn every_listed_subject_is_reachable_and_every_reachable_subject_is_listed() {
        let listed = subjects();
        // Counted off the implementations rather than written as a number: a
        // literal here is `AGENTS.md` §7's staleness, and this one went stale
        // on 2026-08-24 when `install` arrived, again the same day when `edit`
        // did, and again on 2026-08-25 for `uninstall`. **Three times, which is
        // the argument for the shape rather than against it** — each was a
        // one-word edit here and none of them was a subject that shipped
        // uncatalogued.
        // `ADR-006` §8.10's seven arrived on 2026-08-29, which is the fourth
        // time this number has moved. It is still counted off the
        // implementations rather than written as a literal, for the reason
        // below.
        // `ADR-017` §6.3.2's `service-restart` arrived on 2026-08-29, the fifth
        // move, and it is the one this shape was most worth having for: the
        // number and the vocabulary of the two `restart`s had to change
        // together.
        // `ADR-012` §9's `input` arrived on 2026-09-01, the sixth move — and it
        // is the one that made this project's own title true.
        // `install-secret` arrived on 2026-09-01, the seventh move, and it is
        // counted with §8.10's file surface because it is one of them: it
        // writes a path. It is written as `+ 1` on its own rather than as an
        // `8` so that the seven §8.10 named stay countable against the ADR.
        // `service-stop` and `service-start` are the `+ 2`, `ADR-017` §6.
        // `restore-secret` is the `+ 1` after the `+ 2`, `ADR-030` §4.
        // `enroll-member` is the next `+ 1`, `ADR-059` §6 — `install-secret`'s
        // act composed under a fleet server's register.
        // `remove-member` is the next `+ 1`, the same day — its exact inverse,
        // `remove-file`'s act composed the same way.
        // `rename-phone` and `revoke-phone` are the last `+ 2`, `ADR-061` §6 —
        // `enroll-member`'s and `remove-member`'s acts, composed under a fleet
        // server's register/phones/ instead of register/devices/.
        // `passkey-code` is the last `+ 1`, `ADR-063` §4 — `install-secret`'s
        // act composed under the proxy's data directory.
        // `clock-floor-reset` is the last `+ 1`, `ADR-021` §3 — `service-
        // restart`'s shape and socket, with the clock floor as its object.
        // `uninstall-library` is the final `+ 1`, `uninstall`'s twin for a
        // library build.
        assert_eq!(
            listed.len(),
            1 + Stop::ALL.len() + 6 + 7 + 1 + 1 + 2 + 1 + 1 + 1 + 2 + 1 + 1 + 1
        );
        assert!(listed.contains(&uninstall_library::SUBJECT));
        assert!(listed.contains(&super::clock_floor::SUBJECT));
        assert!(listed.contains(&passkey_code::SUBJECT));
        assert!(listed.contains(&enroll_member::SUBJECT));
        assert!(listed.contains(&remove_member::SUBJECT));
        assert!(listed.contains(&rename_phone::SUBJECT));
        assert!(listed.contains(&revoke_phone::SUBJECT));
        assert!(listed.contains(&restore_secret::SUBJECT));
        assert!(listed.contains(&service_restart::SUBJECT));
        assert!(listed.contains(&service_hold::STOP_SUBJECT));
        assert!(listed.contains(&service_hold::START_SUBJECT));
        assert!(listed.contains(&input::SUBJECT));
        assert!(listed.contains(&install_secret::SUBJECT));
        for subject in [
            create::FILE_SUBJECT,
            create::DIRECTORY_SUBJECT,
            permissions::MODE_SUBJECT,
            permissions::OWNER_SUBJECT,
            rename::SUBJECT,
            remove::FILE_SUBJECT,
            remove::DIRECTORY_SUBJECT,
        ] {
            assert!(listed.contains(&subject), "{subject} is not listed");
            assert_eq!(
                stop::parse_subject(subject),
                None,
                "{subject} must not also be a stop"
            );
        }
        assert!(listed.contains(&reload::SUBJECT));
        assert!(listed.contains(&repo_delete::SUBJECT));
        assert!(listed.contains(&trial::SUBJECT));
        assert!(listed.contains(&install::SUBJECT));
        assert!(listed.contains(&edit::SUBJECT));
        assert!(listed.contains(&uninstall::SUBJECT));
        assert_eq!(
            stop::parse_subject(edit::SUBJECT),
            None,
            "edit must not also be a stop"
        );
        assert_eq!(
            stop::parse_subject(install::SUBJECT),
            None,
            "install must not also be a stop"
        );
        for stop in Stop::ALL {
            assert!(listed.contains(&stop.as_str()), "{stop} is not listed");
            assert_eq!(stop::parse_subject(stop.as_str()), Some(*stop));
        }
        assert_eq!(
            stop::parse_subject(trial::SUBJECT),
            None,
            "the two sets overlap"
        );
        // No duplicates: two subjects with one word is two implementations
        // behind one request, and which one runs would depend on match order.
        let mut seen: Vec<&str> = Vec::new();
        for word in &listed {
            assert!(!seen.contains(word), "{word} is listed twice");
            seen.push(word);
        }
    }

    /// The confirmation rule, once, for every subject that uses it.
    #[test]
    fn only_the_exact_word_confirms_and_a_misspelling_is_an_argument_error() {
        assert_eq!(confirmed(None), Ok(false));
        assert_eq!(confirmed(Some(CONFIRMATION)), Ok(true));
        for word in ["confrim", "yes", "--confirm", "CONFIRM", "", "y", "Confirm"] {
            let refusal = confirmed(Some(word)).expect_err("only `confirm` may confirm");
            assert_eq!(refusal.code(), ErrorCode::Arguments, "{word:?}");
            assert_eq!(refusal.subject(), Some(word));
            assert!(
                refusal.message().contains(CONFIRMATION),
                "{}",
                refusal.message()
            );
        }
    }

    /// **A subject the table does not list is refused before it can be reached.**
    ///
    /// The property this module's documentation always claimed and only half
    /// had. `subjects()` is generated from [`subject_table`], so a *listed*
    /// subject with no arm was caught by the test above; an **arm with no
    /// entry** was not, because [`perform`]'s chain reached its implementation
    /// without consulting anything.
    ///
    /// `apply reload` shipped in exactly that state for the length of one commit
    /// on 2026-08-26: reachable, absent from `rivet --version`, and absent from
    /// the union `super::Operation::outputs` generates. The gate at the top of
    /// `perform` is what closes it, and this is that gate — asserted through the
    /// refusal an operator would actually see.
    #[test]
    fn a_subject_that_is_not_in_the_table_is_refused_however_it_is_spelled() {
        let mut board = super::super::tests::no_machine();
        for word in ["reboot", "shutdown", "trial", "", "Reload", "reload "] {
            let refusal = perform(&[word], &mut *board)
                .expect_err("an unlisted subject must be refused");
            assert_eq!(
                refusal.code(),
                ErrorCode::UnknownSubject,
                "{word:?} reached an implementation"
            );
            assert_eq!(refusal.subject(), Some(word));
            // And the refusal names the whole vocabulary, so the next attempt
            // can be right rather than another guess.
            for listed in subjects() {
                assert!(
                    refusal.message().contains(listed),
                    "{listed} is not offered: {}",
                    refusal.message()
                );
            }
        }
    }

    /// The catalogue names every subject, so an operator who ran a wrong one is
    /// told the whole vocabulary rather than only that theirs was wrong.
    #[test]
    fn the_operations_own_description_lists_every_subject_it_accepts() {
        let described = OPERATION.inputs[0].description;
        for subject in subjects() {
            assert!(
                described.contains(subject),
                "{subject} is not described: {described}"
            );
        }
    }
}
