//! `rivet apply install-secret <app> <name> --hex <hex>` — **put a credential
//! where an application can read it.**
//!
//! `ADR-006` §8.10's file surface, extended 2026-09-01. `ADR-015` §3.2 gives
//! every application one writable directory, `/state/app/<name>/data`, owned by
//! [`crate::service::app_identity`]'s uid and gid, mode `0700`. Getting a
//! per-machine credential into it was possible and it was painful, and the pain
//! was structural rather than a missing convenience.
//!
//! # The two things `apply edit` cannot do, and why neither is a small gap
//!
//! ## It writes as root, mode 0600
//!
//! `super::edit::NEW_FILE_MODE` is `0600` and the writer is `rivet`, running as
//! root. An application running as uid 5830 then gets `EACCES` reading **its
//! own certificate** — and it reports that from inside itself, at a moment
//! nobody is watching, which is the failure `crate::service::app_identity`
//! already names as the reason a uid is derived rather than allocated.
//!
//! Closing it by hand needed three more commands, each with its own plan and
//! its own confirmation:
//!
//! ```text
//! rivet apply edit      /state/app/fleetd/data/cert --append <text> confirm
//! rivet apply set-owner /state/app/fleetd/data/cert fleetd confirm
//! rivet apply set-mode  /state/app/fleetd/data/cert 0400 confirm
//! ```
//!
//! **Two of the three succeeding is worse than none of them**: the file is at
//! its final name, owned by root, and the application that needs it cannot see
//! it. Nothing reports that state, because from each command's own point of
//! view nothing went wrong.
//!
//! ## It is text-only
//!
//! `super::machine::Write::contents` is a `&str`. An Ed25519 seed is 32 bytes
//! of no particular encoding and loses its null bytes on the way through one.
//! `cmd/rivet-fleetd`'s node module documents the workaround it was forced
//! into, and names this operation as the thing that was missing:
//!
//! > **Hex because a control plane cannot carry bytes.** `apply edit` is the
//! > only way to put a file on a machine and it is text — a raw seed loses its
//! > null bytes on the way. Recorded as a gap rather than hidden: this wants an
//! > operation that installs binary material into an application's data
//! > directory, and there is not one.
//!
//! # The encoding on the wire: hex, and the reasoning is not "it is shorter"
//!
//! The control plane is JSON (`ADR-035` §3.2), so bytes have to be spelled. The
//! two candidates are base64 and hex, and **base64 is smaller and hex is
//! stricter**. Size decides nothing here — a certificate is a couple of
//! kilobytes and the cap below is 64 KiB — so strictness does:
//!
//! | | **hex — chosen** | base64 |
//! | :-- | :-- | :-- |
//! | Alphabets | one | two in common use (`+/` and `-_`), and a decoder that accepts both accepts two spellings of one value |
//! | Padding | none | `=`, optional in several implementations, so two encodings of one value again |
//! | Whitespace | refused | ignored by many decoders, which makes a mangled paste decode to something |
//! | A decoder in this workspace | [`crate::secret::bytes_from_hex`], public and tested | one private encoder in `super::file`, and **no** decoder at all |
//! | What the material already is | `cargo rivet key`, `crate::secret::Secret::to_stored` and `cmd/rivet-fleetd`'s own files are all hex | — |
//!
//! Every row but the last is the same argument: **base64 has more than one way
//! to write one value, and a decoder that quietly accepts them all is the "wrong
//! guess succeeds" failure `AGENTS.md` §5 is about** — arriving at the one place
//! where a wrong answer is a wrong credential.
//!
//! **The asymmetry with `get file` is deliberate and is a cost, not an
//! oversight.** That subject answers base64, because it reads whole arbitrary
//! files and size is real there. So copying a file off one machine and
//! installing it on another needs a conversion in between. That is one step in
//! a workstation tool, against a decoder that cannot be fooled at the one point
//! where being fooled means an application presenting the wrong identity.
//!
//! # What this does NOT do
//!
//! * **It does not create the application's data directory.** A directory that
//!   is not there means the application is not installed, and creating one for
//!   a name nobody installed would put a credential where nothing will ever
//!   read it and report success. The refusal says which application and what to
//!   do instead.
//! * **It does not encrypt anything.** `ADR-020` §11.2.3 ruled encoding rather
//!   than encryption for secrets at rest, and this writes the bytes it was
//!   given. `mode 0600` and the application's own uid are the whole of the
//!   protection, which is the same protection `ADR-015` §3.2 gives the
//!   directory.
//! * **It does not restart the application.** A credential is read when the
//!   program next opens it, and this build does not know when that is — see
//!   [`TakesEffect::NotAsserted`] in the plan.

use super::apply::{ACCEPT_FLAG, CONFIRMATION, Hazard};
use super::filesystem::{
    arguments, check_writable_path, confirmation_words, describe_mode, hazard_for, read_tail,
    refused, unsafe_for,
};
use super::machine::{Board, PathChange, PathKind, PathStat};
use super::plan::{Change, Effect, Plan, TakesEffect, resolve};
use super::{Field, OperationError, Verb};
use crate::deploy::Principal;
use crate::secret::{StoredError, bytes_from_hex};
use crate::service::{ServiceName, app_identity, data_dir};
use crate::state::STATE_MOUNT_POINT;
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "install-secret";

/// The flag that carries the content, **named for its encoding**.
///
/// A flag rather than a bare positional so that the encoding is visible in the
/// command, in the plan and in the journal — and so that a second encoding, if
/// one is ever ruled, is a second flag rather than a change in meaning of an
/// unlabelled word.
pub const HEX_FLAG: &str = "--hex";

/// The mode an installed secret gets.
///
/// `0600` — the owner reads and writes it, nobody else sees it, and the owner
/// is the application. **The same number `super::edit::NEW_FILE_MODE` uses**,
/// and for `ADR-006` §8.9.3's reason: private first, widened deliberately,
/// never the reverse. `apply set-mode` is how it is widened, and there is no
/// form of this command that opens it.
///
/// It is not `0400`. A read-only credential is tempting and it is the
/// application's decision rather than this operation's: several programs
/// rewrite their own state file beside a key, and a mode that refuses that
/// would be this subject deciding how an application manages its own directory.
pub const SECRET_MODE: u32 = 0o600;

/// The longest secret this build installs, in **decoded** bytes.
///
/// `super::edit::TEXT_MAX_BYTES`'s number and its argument: material this size
/// is not one word of a command. An Ed25519 seed is 32 bytes and a certificate
/// chain is a few thousand, so the cap is two orders of magnitude clear of
/// anything this subject is for — and `ADR-035` §3.2's bulk endpoint is what a
/// payload rather than a credential goes through.
pub const SECRET_MAX_BYTES: usize = super::edit::TEXT_MAX_BYTES;

/// **What stands in for the secret in the line the plan tells you to repeat.**
///
/// Every other subject's plan closes with the command an operator pastes back,
/// assembled from the words they typed — `super::filesystem::confirmation_words`
/// — and for every other subject that is exactly right. Here it would print the
/// credential.
///
/// **Found by a test, not by reading it**, and it is worth saying which one:
/// `neither_the_plan_nor_the_answer_ever_contains_the_secret` asserted the hex
/// was absent from the narrative, and the narrative ended
/// `… --hex 4e45572053454352455420434f4e54454e54 confirm`. The rest of this
/// module was already careful — the plan states two *lengths* and refuses to
/// diff — and the leak was in the one line nobody writes, which is the shape
/// `AGENTS.md` §7 is about.
///
/// A plan is not private the way a shell's history is. It is rendered to a
/// terminal, returned over the control plane, and read by an agent whose
/// transcript outlives the session, so the value would land in several places
/// the operator never chose. **They already have it — they just typed it** —
/// which is why standing something in its place costs a paste and nothing else.
pub const REDACTED_VALUE: &str = "<the same hexadecimal value, again>";

/// The longest name a secret may have.
pub const NAME_MAX_LEN: usize = 64;

/// The whole naming rule, in one sentence, for a diagnostic to quote.
pub const NAME_RULE: &str =
    "a secret's name is 1 to 64 characters of ASCII letters, digits, `-`, `_` or `.`, starting \
     with a letter or a digit; it names ONE FILE directly inside the application's data \
     directory, so it cannot contain a path separator and cannot be `.` or `..`";

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was applied. Always `install-secret`.",
    },
    Field {
        name: "application",
        description: "The application whose data directory it went into, as it was given.",
    },
    Field {
        name: "path",
        description: "The ABSOLUTE path the secret was written to. Composed by this operation \
                      from the application's name — it is never given, so that no caller can \
                      put a credential outside the directory it names.",
    },
    Field {
        name: "bytes",
        description: "How many bytes were written, DECODED. Not the length of the hex. ZERO IS \
                      A REAL ANSWER — an empty secret is a thing somebody may mean, and this \
                      says which it was.",
    },
    Field {
        name: "replaced-bytes",
        description: "How many bytes were there before, when this replaced an existing secret. \
                      ABSENT when nothing was there, which is the first install. THE OLD BYTES \
                      ARE GONE and nothing on this machine can return them.",
    },
    Field {
        name: "mode",
        description: "The mode it was written with, octal. Always 0600 in this build: the owner \
                      reads and writes it and nobody else sees it (ADR-006 §8.9.3). `rivet apply \
                      set-mode` is how it is widened.",
    },
    Field {
        name: "uid",
        description: "The uid that owns it — the APPLICATION'S, derived from its name by \
                      ADR-015 §5.1's rule and not read from any record. This is the whole point \
                      of the subject: a credential owned by root is one the application gets \
                      `Permission denied` on.",
    },
    Field {
        name: "gid",
        description: "The gid that owns it. Equal to `uid` — the user-private-group idiom \
                      (ADR-015 §5.1).",
    },
    Field {
        name: "principal",
        description: "What this machine can say about who asked, in a sentence (ADR-028 §4.5). \
                      ATTRIBUTION AND NOT AUTHORITY: nothing checked it.",
    },
    Field {
        name: "journalled",
        description: "Whether a record of this reached the journal. FALSE DOES NOT MEAN NOTHING \
                      WAS WRITTEN. The record names the path, the length and the owner and \
                      NEVER THE CONTENT (ADR-020 §4).",
    },
    Field {
        name: "unrecorded",
        description: "Why the journal has no record of it. Absent when it has one.",
    },
    Field {
        name: "next",
        description: "What this answer does and does not settle — in particular that the \
                      application is NOT restarted and has not been shown to have read this.",
    },
];

/// One secret that was installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    application: String,
    path: String,
    bytes: usize,
    replaced_bytes: Option<u64>,
    uid: u32,
    gid: u32,
    principal: Principal,
    unrecorded: Option<String>,
}

impl Installed {
    /// Whether a record of it reached the journal.
    #[must_use]
    pub const fn journalled(&self) -> bool {
        self.unrecorded.is_none()
    }

    /// The structured answer. `ADR-006` §4.
    #[must_use]
    pub fn value(&self) -> Value {
        let mut fields = vec![
            ("subject", Value::str(SUBJECT)),
            ("application", Value::str(self.application.clone())),
            ("path", Value::str(self.path.clone())),
            (
                "bytes",
                Value::Integer(i64::try_from(self.bytes).unwrap_or(i64::MAX)),
            ),
        ];
        // **Absent when nothing was replaced**, rather than zero. A `0` here
        // would be a size, and every first install would look like one that
        // overwrote an empty file — which is a different event.
        if let Some(replaced) = self.replaced_bytes {
            fields.push((
                "replaced-bytes",
                Value::Integer(i64::try_from(replaced).unwrap_or(i64::MAX)),
            ));
        }
        fields.push(("mode", Value::str(describe_mode(SECRET_MODE))));
        fields.push(("uid", Value::Integer(i64::from(self.uid))));
        fields.push(("gid", Value::Integer(i64::from(self.gid))));
        fields.push((
            "principal",
            Value::str(self.principal.describes(
                "the secret",
                &format!("`rivet {apply} {SUBJECT}`", apply = Verb::Apply),
                "anything running as uid 0 could have written this file another way and left no \
                 record at all",
            )),
        ));
        fields.push(("journalled", Value::Bool(self.journalled())));
        if let Some(why) = &self.unrecorded {
            fields.push(("unrecorded", Value::str(why.clone())));
        }
        fields.push((
            "next",
            Value::str(format!(
                "The file is there, owned by uid {uid} with mode {mode}, and THAT IS ALL THIS \
                 ASSERTS. The application was NOT restarted and has NOT been shown to have read \
                 it — a program reads a credential when it next opens one, and nothing in this \
                 build knows when that is. `rivet {apply} service-restart {app} {CONFIRMATION}` \
                 is how it is made to, and `rivet {get} file {path}` says what is there now \
                 WITHOUT saying anything about whether the application can parse it.",
                uid = self.uid,
                mode = describe_mode(SECRET_MODE),
                apply = Verb::Apply,
                get = Verb::Get,
                app = self.application,
                path = self.path,
            )),
        ));
        Value::object(fields)
    }
}

/// The command's shape, for every refusal to quote.
fn forms() -> String {
    format!(
        "`rivet {apply} {SUBJECT} <application> <name> {HEX_FLAG} <hex> [{ACCEPT_FLAG} <text>] \
         [{CONFIRMATION}]`",
        apply = Verb::Apply,
    )
}

/// Perform it.
///
/// # Errors
///
/// [`ErrorCode::Arguments`] for a malformed command — a name that is not one, a
/// value that is not hexadecimal, a secret over [`SECRET_MAX_BYTES`];
/// [`ErrorCode::Unsafe`] for a data directory that is not there or is not a
/// directory; [`ErrorCode::ConfirmationRequired`] carrying the plan; and
/// [`ErrorCode::MachineRefused`] when the machine could not do it.
pub(super) fn perform(
    arguments_in: &[&str],
    board: &mut dyn Board,
) -> Result<Installed, OperationError> {
    let Some(application) = arguments_in.get(1).copied() else {
        return Err(arguments(
            SUBJECT,
            format!(
                "`{apply} {SUBJECT}` needs the APPLICATION whose data directory to write into, \
                 as the word after `{SUBJECT}`. {forms}.",
                apply = Verb::Apply,
                forms = forms(),
            ),
        ));
    };
    let Some(service) = ServiceName::new(application) else {
        return Err(arguments(
            application,
            format!(
                "`{application}` is not a service name, so it names no application's data \
                 directory. {rule}. `rivet {get} status` spells every service this machine \
                 holds.",
                rule = ServiceName::RULE,
                get = Verb::Get,
            ),
        ));
    };

    let Some(name) = arguments_in.get(2).copied() else {
        return Err(arguments(
            SUBJECT,
            format!(
                "`{apply} {SUBJECT} {application}` needs the NAME the secret gets inside that \
                 application's data directory, as the next word. {forms}.",
                apply = Verb::Apply,
                forms = forms(),
            ),
        ));
    };
    check_name(name)?;

    // **The flag is read positionally and by name**, so that a missing value
    // cannot swallow the confirmation: `--hex confirm` is refused below rather
    // than installing a secret whose content is the word `confirm`. It is the
    // rule `super::create` applies to its optional text and `super::edit` to
    // its `--replace`.
    match arguments_in.get(3).copied() {
        Some(HEX_FLAG) => {}
        Some(other) => {
            return Err(arguments(
                other,
                format!(
                    "`{other}` is where `{HEX_FLAG}` should be. The content is always named by \
                     its encoding, so that what arrives is never guessed from what it looks \
                     like. {forms}.",
                    forms = forms(),
                ),
            ));
        }
        None => {
            return Err(arguments(
                SUBJECT,
                format!(
                    "`{HEX_FLAG} <hex>` is missing, and there is no form of this command that \
                     installs an unnamed encoding. To install an EMPTY secret, give an empty \
                     value: `{HEX_FLAG} \"\"`. {forms}.",
                    forms = forms(),
                ),
            ));
        }
    }
    let Some(hex) = arguments_in.get(4).copied() else {
        return Err(arguments(
            HEX_FLAG,
            format!(
                "`{HEX_FLAG}` is the last word and has no value after it. {forms}.",
                forms = forms(),
            ),
        ));
    };
    if hex == CONFIRMATION || hex == ACCEPT_FLAG {
        return Err(arguments(
            hex,
            format!(
                "`{HEX_FLAG}` is followed by `{hex}`, which is one of this command's own words \
                 rather than a hexadecimal value — so `{HEX_FLAG}` has no content and `{hex}` \
                 has been eaten. An empty secret is spelled `{HEX_FLAG} \"\"`."
            ),
        ));
    }
    let contents = decode(hex)?;

    let tail = read_tail(SUBJECT, &forms(), arguments_in, 5)?;

    // **The path is COMPOSED, never given.** That is what confines this subject
    // to one directory: there is no argument a caller could spell that puts a
    // file anywhere else, so the traversal question is answered by the shape of
    // the command rather than by a check on a string. `check_name` above refuses
    // a separator, so the composition below cannot be steered.
    let directory = data_dir(STATE_MOUNT_POINT, &service);
    let path = format!("{directory}/{name}");
    check_writable_path(&format!("{} {SUBJECT}", Verb::Apply), &path)?;
    let hazard = hazard_for(&path);
    super::apply::accepted(tail.accept, hazard.as_ref(), tail.confirmed)?;

    let identity = app_identity(&service);
    resolve(
        Installation {
            application,
            directory,
            path,
            name,
            contents,
            uid: identity.uid,
            gid: identity.gid,
            typed: arguments_in,
            hazard,
        },
        tail.confirmed,
        board,
    )
}

/// Whether `text` is a name `perform` could have used to install a secret —
/// `.rulings/` R-169, added for `rivet_sdk::secrets` to read one back by.
///
/// A yes/no restatement of `check_name` below: a reader of a secret only
/// needs to know whether a name is one `install-secret` could have written,
/// not which of that function's several reasons a bad one fails for.
#[must_use]
pub fn is_secret_name(text: &str) -> bool {
    check_name(text).is_ok()
}

/// **The name rule, and it is deliberately narrower than a path check.**
///
/// A name that could not name a file directly inside the data directory is
/// refused before anything is composed — so the traversal case is not "caught
/// later by a path test", it is unrepresentable. `..` and a separator are the
/// two that matter and they are refused by the same rule that refuses a space.
pub(super) fn check_name(name: &str) -> Result<(), OperationError> {
    let refuse = |why: &str| -> OperationError {
        arguments(
            name,
            format!("`{name}` is not a secret's name: {why}. {NAME_RULE}."),
        )
    };
    if name.is_empty() {
        return Err(refuse("it is empty"));
    }
    if name.len() > NAME_MAX_LEN {
        return Err(refuse(&format!(
            "it is {} characters and the most is {NAME_MAX_LEN}",
            name.len()
        )));
    }
    if name.contains('/') || name.contains('\\') {
        return Err(refuse(
            "it contains a path separator, and this subject writes ONE file directly inside the \
             application's data directory. `rivet apply create-directory` and `rivet apply edit` \
             are how anything deeper is made, and they are separate acts on purpose",
        ));
    }
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return Err(refuse("it is empty"));
    };
    if !first.is_ascii_alphanumeric() {
        return Err(refuse(
            "it starts with something that is not a letter or a digit — which is what makes `.`, \
             `..` and a name that looks like a flag impossible here rather than merely unlikely",
        ));
    }
    for character in characters {
        if !(character.is_ascii_alphanumeric()
            || character == '-'
            || character == '_'
            || character == '.')
        {
            return Err(refuse(&format!("`{character}` is not one of its characters")));
        }
    }
    Ok(())
}

/// Hexadecimal to the bytes it spells, with this subject's refusals.
///
/// **The length is reported and the value never is** (`ADR-020` §4). A refusal
/// that quoted the hex it could not read would put a credential into a
/// diagnostic, a terminal's scrollback and possibly a journal — and the
/// commonest reason a value is unreadable is that it is the *right* value with
/// something stuck to it.
pub(super) fn decode(hex: &str) -> Result<Vec<u8>, OperationError> {
    let bytes = bytes_from_hex(hex).map_err(|why| {
        let detail = match why {
            StoredError::NotHex => format!(
                "it is {given} characters, and it must be an EVEN number of them, each one of \
                 0-9 a-f A-F and nothing else — no `0x`, no separators, no internal whitespace. \
                 The value is not quoted back here on purpose (ADR-020 §4): what could not be \
                 read is usually the right credential with something stuck to it, and a refusal \
                 that printed it would put it in a terminal's scrollback",
                given = hex.trim().len(),
            ),
            // Unreachable: `bytes_from_hex` never decodes text. Written as a
            // refusal rather than an `unreachable!` for `super::apply`'s
            // reason — a panic in `rivet-core` is a panic in pid 1's
            // dependency tree.
            StoredError::NotText => why.to_string(),
        };
        arguments(
            HEX_FLAG,
            format!("the value after `{HEX_FLAG}` is not hexadecimal: {detail}."),
        )
    })?;
    if bytes.len() > SECRET_MAX_BYTES {
        return Err(arguments(
            HEX_FLAG,
            format!(
                "the secret decodes to {given} bytes and the most this build installs is \
                 {most} — `apply {edit}`'s cap and the same argument: material that size is not \
                 one word of a command, and ADR-035 §3.2's bulk endpoint is what a payload goes \
                 through.",
                given = bytes.len(),
                most = SECRET_MAX_BYTES,
                edit = super::edit::SUBJECT,
            ),
        ));
    }
    Ok(bytes)
}

/// **The change this subject is**, as `ADR-006` §6.5 defines one.
struct Installation<'a> {
    application: &'a str,
    directory: String,
    path: String,
    name: &'a str,
    contents: Vec<u8>,
    uid: u32,
    gid: u32,
    typed: &'a [&'a str],
    hazard: Option<Hazard>,
}

/// What planning an installation worked out.
///
/// **The planned size is deliberately not in here.** It would be the one thing
/// this type could carry that must not be believed: `apply` re-reads the path
/// at the moment it acts, and a `replaced-bytes` taken from the plan would
/// report a size the file had when somebody typed the first command rather than
/// the one it had when the bytes were destroyed.
struct Prepared {
    principal: Principal,
}

impl Change for Installation<'_> {
    type Prepared = Prepared;
    type Applied = Installed;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, Prepared), OperationError> {
        let path = self.path.as_str();

        // **The directory is checked before the file.** An application that is
        // not installed has no data directory, and a credential written into a
        // directory this operation had created itself would sit where nothing
        // will ever read it — reported as a success.
        let directory = board
            .stat_path(&self.directory)
            .map_err(|e| refused(e, &self.directory))?;
        self.check_directory(&directory)?;

        let stat = board.stat_path(path).map_err(|e| refused(e, path))?;
        let was_bytes = self.check_replaceable(&stat)?;
        let principal = board.principal();

        board
            .can_change_path(&self.change(was_bytes.unwrap_or(0)))
            .map_err(|e| refused(e, path))?;

        let warning = self
            .hazard
            .as_ref()
            .map_or_else(String::new, |h| format!(" {}", h.warning()));

        let effect = match was_bytes {
            None => Effect::create(path)
                .becomes(format!("{} bytes", self.contents.len()))
                .noting(format!(
                    "NOTHING IS THERE NOW, so this is the first install of `{name}` for \
                     `{app}`.{warning}",
                    name = self.name,
                    app = self.application,
                )),
            // **`was` is a length and never a value.** The plan is printed, and
            // the one thing that must not be printed is the credential being
            // replaced (`ADR-020` §4).
            Some(was) => Effect::set(path, format!("{was} bytes"), format!("{} bytes", self.contents.len()))
                .noting(format!(
                    "THIS REPLACES AN EXISTING SECRET AND THE OLD BYTES ARE GONE — nothing on \
                     this machine can return them, and no diff is shown because neither value \
                     may be printed (ADR-020 §4). If `{app}` is running, it is holding whatever \
                     it read at start-up until it is restarted.{warning}",
                    app = self.application,
                )),
        };

        let plan = Plan::new(
            path,
            format!(
                "installs a credential into `{app}`'s data directory, owned by that application",
                app = self.application,
            ),
        )
        .doing(effect)
        .doing(
            Effect::set(
                path,
                "root:root, which is what every other way of writing this file produces",
                format!("uid {uid}, gid {gid}, mode {mode}", uid = self.uid, gid = self.gid, mode = describe_mode(SECRET_MODE)),
            )
            .noting(format!(
                "The uid is `{app}`'s own, DERIVED FROM ITS NAME (ADR-015 §5.1) and not read \
                 from any record — so it is the same on this machine and on every other one. \
                 THE OWNERSHIP IS SET BEFORE THE FILE REACHES ITS NAME, by writing a temporary \
                 and renaming it: there is no instant at which `{path}` exists owned by root, \
                 which is the window that made the three-command version of this unsafe.",
                app = self.application,
            )),
        )
        .leaving(format!(
            "`{app}` is NOT restarted and nothing asks it to re-read anything. A program reads \
             a credential when it next opens one",
            app = self.application,
        ))
        .leaving("nothing else in the data directory is touched, and the directory's own mode and owner are unchanged")
        .taking_effect(TakesEffect::NotAsserted {
            because: "the file is there as soon as this returns, and nothing in this build knows \
                      when the application next opens it — which is the moment the credential \
                      actually takes effect",
        })
        .leaving(
            "THE PATH IS EXAMINED AGAIN AT THE MOMENT THIS ACTS: what is reported as replaced is \
             read then, not now, because nothing carries a plan from one command to the next",
        )
        .leaving(format!(
            "THE VALUE IS NOT ECHOED in the line below and `{REDACTED_VALUE}` stands where it \
             was. A plan is printed, returned over the control plane and read by agents, and a \
             credential in it would end up in several places nobody chose (ADR-020 §4) — so this \
             is the one command in this catalogue whose confirmation cannot simply be pasted \
             back whole"
        ))
        .confirmed_by(&self.confirmation_words_without_the_secret());

        Ok((plan, Prepared { principal }))
    }

    fn apply(self, prepared: Prepared, board: &mut dyn Board) -> Result<Installed, OperationError> {
        let path = self.path.clone();

        // **Re-run rather than trusted from the plan.** The directory could
        // have been removed by an uninstall between the plan and the
        // confirmation, and writing into a path whose parent went away would
        // either fail obscurely or recreate a directory nobody asked for.
        let directory = board
            .stat_path(&self.directory)
            .map_err(|e| refused(e, &self.directory))?;
        self.check_directory(&directory)?;

        let now = board.stat_path(&path).map_err(|e| refused(e, &path))?;
        // **The size reported is the one read HERE**, so a file that grew
        // between the plan and the confirmation is recorded at its real size
        // rather than at the planned one.
        let was_bytes = self.check_replaceable(&now)?;

        let wrote = board
            .change_path(&self.change(was_bytes.unwrap_or(0)))
            .map_err(|e| refused(e, &path))?;

        Ok(Installed {
            application: self.application.to_string(),
            path,
            bytes: self.contents.len(),
            replaced_bytes: was_bytes,
            uid: self.uid,
            gid: self.gid,
            principal: prepared.principal,
            unrecorded: wrote.unrecorded,
        })
    }
}

impl Installation<'_> {
    /// The words the plan tells an operator to repeat, **with the value taken
    /// out**. See [`REDACTED_VALUE`].
    ///
    /// It goes through `super::filesystem::confirmation_words` rather than
    /// around it, so the hazard-acknowledgement rule stays in the one place
    /// every other subject reads it from; only the value is substituted, and
    /// only where the flag actually put it.
    fn confirmation_words_without_the_secret(&self) -> Vec<String> {
        let mut words = confirmation_words(self.typed, self.hazard.as_ref());
        // **The word AFTER the flag**, found rather than assumed at an index:
        // `confirmation_words` has already dropped the confirmation and may
        // have truncated an acknowledgement, so a position computed here would
        // be a second account of that function's output.
        if let Some(at) = words.iter().position(|word| word == HEX_FLAG) {
            if let Some(value) = words.get_mut(at + 1) {
                *value = REDACTED_VALUE.to_string();
            }
        }
        words
    }

    /// The act, built in one place so the plan's gate and the write cannot
    /// describe two different things — `super::permissions`' arrangement.
    fn change(&self, was_bytes: u64) -> PathChange<'_> {
        PathChange::InstallSecret {
            path: &self.path,
            contents: &self.contents,
            mode: SECRET_MODE,
            uid: self.uid,
            gid: self.gid,
            was_bytes,
        }
    }

    /// **The application's data directory must already be there.** See this
    /// module's documentation for why this is a refusal rather than a `mkdir`.
    fn check_directory(&self, stat: &PathStat) -> Result<(), OperationError> {
        let advice = format!(
            "`{directory}` is made by the INSTALLER when `{app}` is installed (ADR-015 §3.2), \
             owned by that application and mode 0700. Install the application first: `rivet \
             {apply} install <path to its .rvt> {CONFIRMATION}`. Creating this directory by \
             hand would put a credential where nothing is going to read it and report that as \
             a success, so this refuses instead.",
            directory = self.directory,
            app = self.application,
            apply = Verb::Apply,
        );
        match stat.kind {
            Some(PathKind::Directory) => Ok(()),
            None => Err(unsafe_for(
                &self.directory,
                format!(
                    "REFUSED there is no data directory for `{app}` and NOTHING HAS BEEN \
                     WRITTEN. {advice}",
                    app = self.application,
                ),
            )),
            Some(other) => Err(unsafe_for(
                &self.directory,
                format!(
                    "REFUSED `{directory}` is a {other} and not a directory, and NOTHING HAS \
                     BEEN WRITTEN. {advice}",
                    directory = self.directory,
                    other = other.as_str(),
                ),
            )),
        }
    }

    /// What is at the path now, in bytes — and a refusal for anything that is
    /// not a regular file.
    ///
    /// **A directory or a symbolic link at this path is refused rather than
    /// replaced.** `check_writable_path` and the adapter's own symlink gate both
    /// look at the path's *ancestors*; a link at the leaf is this operation's
    /// own question, and replacing one would write through it to wherever it
    /// points — outside the data directory the whole subject is confined to.
    fn check_replaceable(&self, stat: &PathStat) -> Result<Option<u64>, OperationError> {
        match stat.kind {
            None => Ok(None),
            Some(PathKind::File) => Ok(Some(stat.size_bytes)),
            Some(other) => Err(unsafe_for(
                &self.path,
                format!(
                    "REFUSED there is a {other} at {path} and NOTHING HAS BEEN WRITTEN. This \
                     subject installs and replaces a FILE; a {other} there is either something \
                     the application made for its own reasons or a way out of the directory \
                     this operation is confined to, and neither is something to overwrite \
                     without being told. `rivet {get} file {path}` says what it is.",
                    other = other.as_str(),
                    path = self.path,
                    get = Verb::Get,
                ),
            )),
        }
    }
}

#[cfg(test)]
#[path = "install_secret_tests.rs"]
mod tests;
