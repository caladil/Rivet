//! `rivet apply restore-secret <name> --hex <hex> [confirm]` — **put one of
//! this machine's own secrets back into `/state/secrets`.** `ADR-030` §4,
//! `.rulings/` R-183 D5.
//!
//! `cargo rivet backup-secrets` pulls the files of `/state/secrets` to a
//! workstation over `get file`, and `cargo rivet restore-secrets` puts them back
//! onto the same board. This is the board half of the second: the narrowest
//! write that can do it.
//!
//! # Why a subject of its own, and not `install-secret` or `edit`
//!
//! * **`install-secret` composes `/state/app/<app>/data/<name>`** and gives the
//!   file to that application's uid. A machine secret is root's and lives in
//!   `/state/secrets` (`ADR-020` §5), so no spelling of that command reaches it
//!   — which is that subject's safety property and is not loosened here.
//! * **`edit` is text-only.** `device.seed` is 32 raw bytes and loses its null
//!   bytes through a `&str`.
//!
//! So this is `install-secret`'s act — [`PathChange::InstallSecret`], a
//! temporary written, owned and renamed into place — with the path composed as
//! `/state/secrets/<name>` and the owner fixed at root. **No adapter changed to
//! add it**, and a journal record of it names that act, `install-secret`, with
//! the `/state/secrets` path and uid 0.
//!
//! # The path is composed, never given
//!
//! `<name>` is one file directly inside `/state/secrets`, by `install-secret`'s
//! own name rule. There is no argument that reaches another directory, so the
//! traversal question is answered by the shape of the command.
//!
//! # What it refuses that `install-secret` does not
//!
//! **A `peers` file that would lock out the caller.** `rivet-mcpd` admits only
//! the keys in `/state/secrets/peers`, reading it when it starts and again as
//! each answer passes through it (`ADR-063` §3). It declines to load a file
//! that drops the key of the session it observed the change in, but it loads
//! whatever is there at its next start — so a restored `peers` that lacks the
//! key this session was pinned by is a board that answers nobody after its
//! next boot, and a headless board has no other way back but its card. When
//! the session came over the control
//! plane, the new content must name its key. A session with no pinned key
//! (the serial console) has no control-plane session to lose, and its plan says
//! the check did not run.
//!
//! # What it does NOT do
//!
//! * **It checks nothing about which board this is.** That refusal is the
//!   workstation's — `cargo rivet restore-secrets` compares the board key a
//!   backup recorded with the key this board proved in the handshake
//!   (`ADR-030` §5). A command that carries the bytes cannot tell where they
//!   came from.
//! * **It restarts nothing.** The consumers of `/state/secrets` are
//!   `rivet-mcpd` and `rivet-netd`, both in the reachability set that
//!   `service-stop` refuses. `peers` is live for the next connection, because
//!   `rivet-mcpd` reads it again before this command's answer reaches the
//!   caller; every other file is read when its consumer starts, so it is live
//!   at the next boot.
//! * **It does not create `/state/secrets`.** `rivet-initd` makes it, mode
//!   `0700`, when it installs the device identity; a board without one was never
//!   given an identity, and this refuses rather than making a directory nothing
//!   provisioned.
//! * **It encrypts nothing.** `ADR-020` §11 rules the store plain text; this
//!   writes the bytes it was given, root-owned, mode `0600`.

use super::apply::CONFIRMATION;
use super::filesystem::{
    arguments, check_writable_path, confirmation_words, describe_mode, read_tail, refused,
    unsafe_for,
};
use super::install_secret::{self, HEX_FLAG, REDACTED_VALUE, SECRET_MODE};
use super::machine::{Board, PathChange, PathKind, PathStat};
use super::plan::{Change, Effect, Plan, TakesEffect, resolve};
use super::{Field, OperationError, Verb};
use crate::deploy::Principal;
use crate::state::SECRETS;
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "restore-secret";

/// The file in `/state/secrets` that names who may reach the control plane.
///
/// Spelled here because this crate depends on nothing that owns it:
/// `rivetos_tls::PEERS_FILE` is the spelling `cmd/rivet-mcpd`'s side reads, and
/// `cmd/cargo-rivet`'s tests hold the two equal.
pub const PEERS_NAME: &str = "peers";

/// The owner every restored secret gets: root, as `rivet-initd` writes them.
const ROOT: u32 = 0;

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was applied. Always `restore-secret`.",
    },
    Field {
        name: "name",
        description: "The secret's file name inside /state/secrets, as it was given.",
    },
    Field {
        name: "path",
        description: "The ABSOLUTE path written. Composed from the name and never given, so no \
                      caller can put a file outside /state/secrets.",
    },
    Field {
        name: "bytes",
        description: "How many bytes were written, DECODED. Zero is a real answer.",
    },
    Field {
        name: "replaced-bytes",
        description: "How many bytes were there before. ABSENT when nothing was there. THE OLD \
                      BYTES ARE GONE.",
    },
    Field {
        name: "mode",
        description: "The mode it was written with. Always 0600, owner root.",
    },
    Field {
        name: "principal",
        description: "What this machine can say about who asked (ADR-028 §4). ATTRIBUTION AND \
                      NOT AUTHORITY.",
    },
    Field {
        name: "journalled",
        description: "Whether a record reached the journal. The record names the path, the \
                      length and the owner and NEVER THE CONTENT (ADR-020 §4).",
    },
    Field {
        name: "unrecorded",
        description: "Why the journal has no record of it. Absent when it has one.",
    },
    Field {
        name: "next",
        description: "What this answer does and does not settle — in particular that nothing \
                      was restarted: `peers` is read again by rivet-mcpd for the next \
                      connection, and every other file is read when its consumer next starts.",
    },
];

/// One secret that was put back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    name: String,
    path: String,
    bytes: usize,
    replaced_bytes: Option<u64>,
    principal: Principal,
    unrecorded: Option<String>,
}

impl Restored {
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
            ("name", Value::str(self.name.clone())),
            ("path", Value::str(self.path.clone())),
            (
                "bytes",
                Value::Integer(i64::try_from(self.bytes).unwrap_or(i64::MAX)),
            ),
        ];
        if let Some(replaced) = self.replaced_bytes {
            fields.push((
                "replaced-bytes",
                Value::Integer(i64::try_from(replaced).unwrap_or(i64::MAX)),
            ));
        }
        fields.push(("mode", Value::str(describe_mode(SECRET_MODE))));
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
                "The file is there, owned by root with mode {mode}, and THAT IS ALL THIS \
                 ASSERTS. Nothing was restarted. rivet-mcpd reads peers again before this \
                 answer reaches you, so a new peers decides the NEXT connection — or, if it \
                 refused to load it, its journal says why and the previous keys still apply; \
                 sessions already open are untouched. Its identity, and rivet-netd's \
                 pre-shared key, are read when they start, so those are live at the next boot. \
                 `rivet {get} file {path}` says what is on disk.",
                mode = describe_mode(SECRET_MODE),
                get = Verb::Get,
                path = self.path,
            )),
        ));
        Value::object(fields)
    }
}

/// The command's shape, for every refusal to quote.
fn forms() -> String {
    format!(
        "`rivet {apply} {SUBJECT} <name> {HEX_FLAG} <hex> [{CONFIRMATION}]`",
        apply = Verb::Apply,
    )
}

/// Perform it.
///
/// # Errors
///
/// `Arguments` for a malformed command; `Unsafe` for a missing
/// `/state/secrets`, something other than a file at the path, or a `peers`
/// file that would lock the caller out; `ConfirmationRequired` carrying the
/// plan; `MachineRefused` when the machine could not do it.
pub(super) fn perform(
    arguments_in: &[&str],
    board: &mut dyn Board,
) -> Result<Restored, OperationError> {
    let Some(name) = arguments_in.get(1).copied() else {
        return Err(arguments(
            SUBJECT,
            format!(
                "`{apply} {SUBJECT}` needs the NAME of the file inside {SECRETS}, as the word \
                 after `{SUBJECT}`. {forms}.",
                apply = Verb::Apply,
                forms = forms(),
            ),
        ));
    };
    if !install_secret::is_secret_name(name) {
        return Err(arguments(
            name,
            format!(
                "`{name}` is not a secret's name. It names ONE FILE directly inside {SECRETS}: \
                 1 to 64 characters of ASCII letters, digits, `-`, `_` or `.`, starting with a \
                 letter or a digit, so it cannot contain a path separator and cannot be `.` or \
                 `..`. {forms}.",
                forms = forms(),
            ),
        ));
    }

    match arguments_in.get(2).copied() {
        Some(HEX_FLAG) => {}
        Some(other) => {
            return Err(arguments(
                other,
                format!(
                    "`{other}` is where `{HEX_FLAG}` should be. The content is always named by \
                     its encoding. {forms}.",
                    forms = forms(),
                ),
            ));
        }
        None => {
            return Err(arguments(
                SUBJECT,
                format!(
                    "`{HEX_FLAG} <hex>` is missing. An EMPTY secret is `{HEX_FLAG} \"\"`. {forms}.",
                    forms = forms(),
                ),
            ));
        }
    }
    let Some(hex) = arguments_in.get(3).copied() else {
        return Err(arguments(
            HEX_FLAG,
            format!(
                "`{HEX_FLAG}` is the last word and has no value after it. {forms}.",
                forms = forms(),
            ),
        ));
    };
    if hex == CONFIRMATION || hex == super::apply::ACCEPT_FLAG {
        return Err(arguments(
            hex,
            format!(
                "`{HEX_FLAG}` is followed by `{hex}`, which is one of this command's own words \
                 rather than a hexadecimal value — so `{HEX_FLAG}` has no content and `{hex}` \
                 has been eaten. An empty secret is spelled `{HEX_FLAG} \"\"`."
            ),
        ));
    }
    let contents = install_secret::decode(hex)?;

    let tail = read_tail(SUBJECT, &forms(), arguments_in, 4)?;
    // No hazard exists for a path under /state/secrets, so an acknowledgement
    // is refused as superfluous — `super::apply::accepted`'s rule.
    super::apply::accepted(tail.accept, None, tail.confirmed)?;

    let path = format!("{SECRETS}/{name}");
    check_writable_path(&format!("{} {SUBJECT}", Verb::Apply), &path)?;

    resolve(
        Restoration {
            name,
            path,
            contents,
            typed: arguments_in,
        },
        tail.confirmed,
        board,
    )
}

/// **Whether a `peers` file's text names `key`.** `rivetos_tls::parse_pins`'s
/// reading — one key per line, `#` starts a comment, blanks are skipped, case
/// does not matter — restated because this crate does not depend on that one.
/// A line that is not a key is simply not a match; the full parse, and its
/// refusal, happen in `rivet-mcpd` when it starts.
///
/// **Membership only, not grammar.** A text with the right key on one line
/// and garbage on another still answers `true` — a caller that must refuse a
/// malformed file needs `parse_peers_grammar` as well.
#[must_use]
pub fn peers_name_key(text: &str, key: &str) -> bool {
    text.lines()
        .map(|line| line.split('#').next().unwrap_or("").trim())
        .any(|line| !line.is_empty() && line.eq_ignore_ascii_case(key))
}

/// How many ASCII hex characters spell an Ed25519 public key — two per byte
/// of `rivetos_tls::KEY_BYTES`, restated because this crate does not depend
/// on that one.
const PEER_KEY_HEX_CHARS: usize = 64;

/// **Validate a whole `peers` file's grammar, before any of it is trusted.**
/// Every line, after stripping from the first `#` and trimming, must be
/// empty or exactly `PEER_KEY_HEX_CHARS` ASCII hex digits, upper or lower
/// case.
///
/// Deliberately a SUBSET of `rivetos_tls::parse_pins`'s reading, which this
/// crate cannot call without depending on it (`AGENTS.md` §3): this refuses
/// oddities such as a `+`-signed pair, which `u8::from_str_radix` happens to
/// accept as a single hex digit. A file this function accepts therefore
/// always parses with `parse_pins` to the same keys in the same order — the
/// reverse does not hold.
///
/// `Ok` carries every key's line, verbatim after stripping and trimming, in
/// the order they appear — what `check_peers` compares the pinned key
/// against.
///
/// # Errors
///
/// A sentence naming the first line that breaks the grammar, or that the
/// text is not UTF-8 at all.
pub fn parse_peers_grammar(contents: &[u8]) -> Result<Vec<String>, String> {
    let text = core::str::from_utf8(contents).map_err(|_| "is not UTF-8 text".to_string())?;
    let mut keys = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.len() != PEER_KEY_HEX_CHARS || !line.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!(
                "has line {}, {line:?}, which is not {PEER_KEY_HEX_CHARS} ASCII hex digits",
                n + 1
            ));
        }
        keys.push(line.to_string());
    }
    Ok(keys)
}

/// **The change this subject is**, as `ADR-006` §6 defines one.
struct Restoration<'a> {
    name: &'a str,
    path: String,
    contents: Vec<u8>,
    typed: &'a [&'a str],
}

/// What planning worked out.
struct Prepared {
    principal: Principal,
}

impl Change for Restoration<'_> {
    type Prepared = Prepared;
    type Applied = Restored;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, Prepared), OperationError> {
        let path = self.path.as_str();

        let directory = board
            .stat_path(SECRETS)
            .map_err(|e| refused(e, SECRETS))?;
        check_directory(&directory)?;

        let stat = board.stat_path(path).map_err(|e| refused(e, path))?;
        let was_bytes = self.check_replaceable(&stat)?;
        let principal = board.principal();
        let lockout_note = self.check_peers(&principal)?;

        board
            .can_change_path(&self.change(was_bytes.unwrap_or(0)))
            .map_err(|e| refused(e, path))?;

        let effect = match was_bytes {
            None => Effect::create(path)
                .becomes(format!("{} bytes", self.contents.len()))
                .noting(format!(
                    "NOTHING IS THERE NOW, so `{name}` is written where this machine has none.",
                    name = self.name,
                )),
            Some(was) => Effect::set(
                path,
                format!("{was} bytes"),
                format!("{} bytes", self.contents.len()),
            )
            .noting(
                "THIS REPLACES WHAT IS THERE AND THE OLD BYTES ARE GONE — nothing on this \
                 machine can return them, and no diff is shown because neither value may be \
                 printed (ADR-020 §4).",
            ),
        };

        let mut plan = Plan::new(
            path,
            format!(
                "puts `{name}` back into {SECRETS}, owned by root",
                name = self.name
            ),
        )
        .doing(effect)
        .doing(
            Effect::set(
                path,
                "whatever owner and mode it has now",
                format!("uid {ROOT}, gid {ROOT}, mode {}", describe_mode(SECRET_MODE)),
            )
            .noting(
                "Written to a temporary, owned and renamed into place, so a reader sees the old \
                 file or the new one and never half of either.",
            ),
        )
        .leaving(
            "NOTHING IS RESTARTED. rivet-mcpd reads peers again before this command's answer \
             reaches its caller, so a new peers decides the next connection and leaves open \
             sessions alone; its identity, and rivet-netd's pre-shared key, are read when they \
             start — so those are live at the next boot",
        )
        .leaving(format!(
            "every other file in {SECRETS} is untouched, and this subject checks nothing about \
             which board the bytes came from: that is `cargo rivet restore-secrets`'s refusal"
        ))
        .taking_effect(TakesEffect::NotAsserted {
            because: "the file is on disk as soon as this returns, and it takes effect when the \
                      program that reads it next opens it — for peers that is rivet-mcpd before \
                      this answer is passed on, unless it refuses the file; for anything else it \
                      is the next boot, and this build does not know every reader",
        });
        if let Some(note) = lockout_note {
            plan = plan.leaving(note);
        }
        let plan = plan
            .leaving(format!(
                "THE VALUE IS NOT ECHOED in the line below and `{REDACTED_VALUE}` stands where it \
                 was (ADR-020 §4)"
            ))
            .confirmed_by(&self.confirmation_words_without_the_secret());

        Ok((plan, Prepared { principal }))
    }

    fn apply(self, prepared: Prepared, board: &mut dyn Board) -> Result<Restored, OperationError> {
        let path = self.path.clone();

        let directory = board
            .stat_path(SECRETS)
            .map_err(|e| refused(e, SECRETS))?;
        check_directory(&directory)?;
        let now = board.stat_path(&path).map_err(|e| refused(e, &path))?;
        let was_bytes = self.check_replaceable(&now)?;

        let wrote = board
            .change_path(&self.change(was_bytes.unwrap_or(0)))
            .map_err(|e| refused(e, &path))?;

        Ok(Restored {
            name: self.name.to_string(),
            path,
            bytes: self.contents.len(),
            replaced_bytes: was_bytes,
            principal: prepared.principal,
            unrecorded: wrote.unrecorded,
        })
    }
}

impl Restoration<'_> {
    /// The words the plan tells an operator to repeat, with the value taken out.
    fn confirmation_words_without_the_secret(&self) -> Vec<String> {
        let mut words = confirmation_words(self.typed, None);
        if let Some(at) = words.iter().position(|word| word == HEX_FLAG)
            && let Some(value) = words.get_mut(at + 1)
        {
            *value = REDACTED_VALUE.to_string();
        }
        words
    }

    /// The act, built in one place so the plan's gate and the write cannot
    /// describe two different things.
    fn change(&self, was_bytes: u64) -> PathChange<'_> {
        PathChange::InstallSecret {
            path: &self.path,
            contents: &self.contents,
            mode: SECRET_MODE,
            uid: ROOT,
            gid: ROOT,
            was_bytes,
        }
    }

    /// **The lock-out check.** See this module's documentation.
    ///
    /// The grammar is validated first, **whether or not this session has a
    /// pinned key to protect** — a session with none must still not be able
    /// to write a `peers` that `rivet-mcpd` will refuse wholesale at its next
    /// start, so the grammar check does not live behind the membership check.
    ///
    /// `Ok(Some(note))` when the grammar is valid but the membership check
    /// could not run, and the plan must say so.
    fn check_peers(&self, principal: &Principal) -> Result<Option<String>, OperationError> {
        if self.name != PEERS_NAME {
            return Ok(None);
        }
        let keys = parse_peers_grammar(&self.contents).map_err(|why| {
            unsafe_for(
                &self.path,
                format!(
                    "REFUSED the new `{PEERS_NAME}` {why} and NOTHING HAS BEEN WRITTEN. \
                     rivet-mcpd reads {SECRETS}/{PEERS_NAME} as text, one hex key per line, and \
                     refuses the WHOLE file on a line like this — which exits before its \
                     listener binds and leaves the board answering nobody after the next boot."
                ),
            )
        })?;
        let Some(key) = principal.origin().pinned_key() else {
            return Ok(Some(format!(
                "this session did not come over the control plane, so there is no pinned key to \
                 look for in the new `{PEERS_NAME}` — whether it still admits whoever manages \
                 this board was NOT checked"
            )));
        };
        if keys.iter().any(|line| line.eq_ignore_ascii_case(key)) {
            return Ok(None);
        }
        Err(unsafe_for(
            &self.path,
            format!(
                "REFUSED the new `{PEERS_NAME}` does not name the key this session was pinned by \
                 ({key}), and NOTHING HAS BEEN WRITTEN. rivet-mcpd admits only the keys in \
                 {SECRETS}/{PEERS_NAME} and reads it whole when it starts, so after the next \
                 boot this board would refuse the connection this command arrived on — and a \
                 headless \
                 board has no other way back but its card. Restore a `{PEERS_NAME}` that \
                 names your key, or add it to the file first."
            ),
        ))
    }

    /// A file or nothing at the path; anything else is refused.
    fn check_replaceable(&self, stat: &PathStat) -> Result<Option<u64>, OperationError> {
        match stat.kind {
            None => Ok(None),
            Some(PathKind::File) => Ok(Some(stat.size_bytes)),
            Some(other) => Err(unsafe_for(
                &self.path,
                format!(
                    "REFUSED there is a {other} at {path} and NOTHING HAS BEEN WRITTEN. This \
                     subject puts back a FILE; writing over a {other} would either destroy \
                     something nothing backed up or follow a link out of {SECRETS}.",
                    other = other.as_str(),
                    path = self.path,
                ),
            )),
        }
    }
}

/// `/state/secrets` must already be a directory. See the module documentation.
fn check_directory(stat: &PathStat) -> Result<(), OperationError> {
    match stat.kind {
        Some(PathKind::Directory) => Ok(()),
        other => Err(unsafe_for(
            SECRETS,
            format!(
                "REFUSED {SECRETS} is {what} and NOTHING HAS BEEN WRITTEN. rivet-initd makes it, \
                 root-owned and mode 0700, when it installs this board's identity; a board \
                 without one was never given an identity, and this does not make the directory \
                 for it.",
                what = other.map_or_else(
                    || "not there".to_string(),
                    |kind| format!("a {} and not a directory", kind.as_str())
                ),
            ),
        )),
    }
}

#[cfg(test)]
#[path = "restore_secret_tests.rs"]
mod tests;
