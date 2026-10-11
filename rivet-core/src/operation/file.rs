//! `rivet get file <path>` — **what is at a path, and what is in it.**
//!
//! `ADR-006` §8.10.5, ruled by the maintainer, Joel Lindell, 2026-08-29. It
//! closes the gap `super::edit`'s own module documentation names against
//! itself, and which `docs/design-candidates/Admin-Surface-Survey.md` §3.2
//! quotes:
//!
//! > an agent can change a file it cannot read back.
//!
//! # One subject, and the argument is about who is guessing
//!
//! The alternatives were three subjects — `get file`, `get stat`,
//! `get directory` — or one with a `--mode` selector. Both make the **caller**
//! say what is at the path before asking what is at the path, and *"what is
//! this"* is very often the question. A caller who guesses wrong gets a refusal
//! instead of an answer, and a surface that refuses a correct question because
//! the asker mis-anticipated the answer is one nobody can use without already
//! knowing.
//!
//! A `--mode` flag is worse: it is `ADR-006` §6.4's two-spellings failure, and
//! the catalogue would publish a selector whose values *are* the answer.
//!
//! So the shape of the answer follows what is actually there — a file gets its
//! contents, a directory gets its entries, a symbolic link gets its target and
//! is never followed, and a path with nothing at it gets `present: false`, which
//! is `Ok` and not an error on `super::document`'s precedent.
//!
//! # It reuses `Editor::read_file` and adds a stat, not a second reader
//!
//! `ADR-011` in the small: two readers of one file are two answers about
//! whether it could be read. The bytes come from the reader `apply edit`
//! already uses; the meta-info comes from `Editor::stat_path`, which is a
//! *stat* and produces no bytes.
//!
//! # `content` is present, windowed, or explained — never lossy
//!
//! **Present** for any regular file, however large: the bytes come back as
//! text when they are valid UTF-8 and as base64 when they are not, with
//! `content_encoding` saying which — never a replacement character, which
//! would be the encoding guess `AGENTS.md` §5 forbids.
//!
//! **Windowed** rather than cut short when the file is larger than
//! `super::edit::FILE_MAX_BYTES`: the answer carries as much as one answer
//! may hold, `content_complete: false`, and `content_offset`/`content_bytes`
//! naming exactly which slice — `content_offset + content_bytes` is the
//! offset to ask again with, for the rest. A file under the cap still comes
//! back whole in one answer, with `content_complete: true`, so a caller that
//! never asks for a window sees nothing different.
//!
//! **Omitted** — with `content_omitted` saying exactly why, never silently —
//! only when there is no window to return at all: the path is not a regular
//! file, or it stopped being a readable regular file between the stat above
//! and the read itself.
//!
//! # It answers about writing without writing
//!
//! The answer carries `changeable`, and where that is false, the refusal an
//! `apply` subject would give — from `super::filesystem`'s own gate rather than
//! from a second opinion. **A caller diagnosing why a change was refused should
//! not have to attempt the change**, and on a headless board the difference
//! between *"I may not"* and *"I tried and something happened"* is the whole
//! difference — there is nothing to walk over to and look at.
//!
//! It also carries `generated` in `super::explain`'s own words when the path is
//! a view, rather than refusing it: reading a projection is exactly what somebody
//! diagnosing one wants to do.

use super::machine::{Editor, PathKind, PathStat};
use super::{ErrorCode, Field, OperationError};
use crate::value::Value;

/// The subject. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "file";

/// How this subject is named in a refusal — the words an operator typed.
const COMMAND: &str = "get file";

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was read. Always `file` in this build.",
    },
    Field {
        name: "path",
        description: "The path, exactly as it was given. Nothing is resolved: a symbolic link \
                      is reported AS a link and never followed (ADR-006 §8.10.4.3).",
    },
    Field {
        name: "present",
        description: "Whether anything at all is at this path. FALSE IS AN ANSWER AND NOT AN \
                      ERROR, and it is the only field besides `subject` and `path` that a \
                      missing path carries.",
    },
    Field {
        name: "type",
        description: "`file`, `directory`, `symlink`, or `other` for a device node, socket or \
                      fifo. Absent when nothing is there.",
    },
    Field {
        name: "size_bytes",
        description: "Bytes, for a regular file. For a directory it is what the filesystem \
                      says the DIRECTORY weighs and NOT the size of what is in it — read \
                      `entry_count` for that.",
    },
    Field {
        name: "mode",
        description: "Permission bits as four octal digits. Absent on a platform with no file \
                      modes — which is NOT the same as mode 0000, and is why it is absent \
                      rather than zero.",
    },
    Field {
        name: "mode_rendered",
        description: "The same mode as `rwxr-xr-x`, and `+setuid`/`+setgid`/`+sticky` appended \
                      where one of those is set. Both forms are published because one reader \
                      branches on the octal and another reads the letters, and which is which \
                      differs by reader.",
    },
    Field {
        name: "uid",
        description: "The owning user's number, which is what the filesystem holds.",
    },
    Field {
        name: "user",
        description: "The name for `uid`, resolved through ADR-007's projected /etc/passwd. \
                      ABSENT when this machine has no name for that number — which is a fact \
                      about the identity table and not about the file.",
    },
    Field {
        name: "gid",
        description: "The owning group's number.",
    },
    Field {
        name: "group",
        description: "The name for `gid`. See `user`.",
    },
    Field {
        name: "modified_ms",
        description: "Milliseconds since the Unix epoch. TREAT THIS WITH SUSPICION ON A PI: \
                      there is no RTC, so a machine whose clock has not yet been corrected \
                      stamps files with times near the epoch and the wall clock JUMPS when \
                      something does correct it (ADR-021). `rivet get log` reports which boot \
                      a record came from for exactly this reason.",
    },
    Field {
        name: "link_target",
        description: "`symlink` ONLY: what the link names, verbatim and unresolved. This \
                      operation does not follow it, and neither do the writing subjects of \
                      ADR-006 §8.10.",
    },
    Field {
        name: "content",
        description: "`file` ONLY, and only when it can be given honestly: the whole file as \
                      text. ABSENT rather than partial or lossy — `content_omitted` says why \
                      whenever it is absent, so a caller never has to infer the reason.",
    },
    Field {
        name: "content_encoding",
        description: "How to read `content`: `text` for a file that is valid UTF-8, or \
                      `base64` (RFC 4648 §4, padded, no line breaks) for one that is not. \
                      NEVER absent when `content` is present. Until 2026-09-01 a file that \
                      was not UTF-8 had its content REFUSED rather than encoded; a Rivet \
                      now produces binary an operator legitimately reads off the machine, \
                      and base64 is not a guess about an encoding the way replacement \
                      characters would be (AGENTS.md §5).",
    },
    Field {
        name: "content_offset",
        description: "Where `content` starts, in bytes from the beginning of the file. `0` \
                      unless a window was asked for.",
    },
    Field {
        name: "content_bytes",
        description: "How many bytes of the FILE `content` covers — NOT the length of \
                      `content`, which is about a third longer when the encoding is base64.",
    },
    Field {
        name: "content_complete",
        description: "Whether the file ENDS at `content_offset + content_bytes`. FALSE IS THE \
                      SIGNAL TO ASK AGAIN with that sum as the offset; a caller that ignores \
                      it has a truncated file and nothing saying so. One answer carries at \
                      most a mebibyte, so any larger file answers `false` here however it \
                      was asked for.",
    },
    Field {
        name: "content_omitted",
        description: "Why `content` is absent: the file is too large to read whole, it is not \
                      UTF-8 and the offset of the first byte that is not, or the path is not a \
                      regular file. NEVER absent when `content` is.",
    },
    Field {
        name: "entry_count",
        description: "`directory` ONLY: how many entries it holds. This is the TRUE count even \
                      when `entries` was cut.",
    },
    Field {
        name: "entries",
        description: "`directory` ONLY: name, type and size per entry, sorted by name, capped. \
                      A link inside a directory is listed as `symlink` and not as what it \
                      points at.",
    },
    Field {
        name: "entries_truncated",
        description: "`directory` ONLY: whether `entries` is shorter than `entry_count`. A \
                      truncated list that did not say so is the same lie as a wrong one.",
    },
    Field {
        name: "symlinked_ancestor",
        description: "A component ON THE WAY to this path that is itself a symbolic link. \
                      Present or absent, and when it is present EVERY WRITING SUBJECT REFUSES \
                      THIS PATH (ADR-006 §8.10.4.3): the permitted-root check is a test on the \
                      text, and a link in the middle means the path resolves somewhere that \
                      test never looked.",
    },
    Field {
        name: "mount_point",
        description: "Whether something is mounted here. FALSE ALSO MEANS 'this build could \
                      not tell', which is the honest reading on a platform with no device \
                      numbers — it is one signal among several and not the guard.",
    },
    Field {
        name: "generated",
        description: "Present when Rivet GENERATES this path — `rivet explain <path>` in \
                      short, and the same table. Reading a projection is not refused; what \
                      this says is that writing it is pointless or refused, and what to change \
                      instead.",
    },
    Field {
        name: "application",
        description: "Present when this path is inside ADR-015's application tree: which zone \
                      it is in and what follows from that — a program is re-verified at every \
                      launch, a file in a version directory is discarded by the next update, \
                      `data` persists, and so does an image service's home, which has no \
                      version directories (ADR-006 §8.10.4.4, ADR-060).",
    },
    Field {
        name: "changeable",
        description: "Whether the WRITING subjects of ADR-006 §8.10 — `remove-file`, `rename`, \
                      `set-mode` and the rest — would accept this path. It is answered from \
                      their own gate rather than from a second opinion, and NOTHING IS \
                      WRITTEN to find out. False does not mean the kernel would refuse: it \
                      means this surface does.",
    },
    Field {
        name: "unchangeable_because",
        description: "The refusal a writing subject would give, verbatim. Absent when \
                      `changeable` is true. It is here so that a caller diagnosing a refused \
                      change does not have to attempt the change to read it.",
    },
    Field {
        name: "next",
        description: "What this answer does and does not settle, and what to run next.",
    },
];

/// Perform `get file`.
///
/// # Errors
///
/// A path that is not absolute, carries a control character, has a `.` or `..`
/// component or a trailing slash; a second word; or a machine that could not
/// answer. **A path with nothing at it is not an error** — see this module's
/// documentation.
pub(super) fn perform(
    arguments: &[&str],
    editor: &mut dyn Editor,
) -> Result<Value, OperationError> {
    let Some(path) = arguments.get(1).copied() else {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "`{COMMAND}` needs the ABSOLUTE path to read, as the word after `{SUBJECT}`. \
                 There is no default and nothing is searched for: a path is named."
            ),
        ));
    };
    // **A WINDOW, added 2026-09-21** — `ADR-006` §8, and the defect that
    // forced it is worth naming because it read as a property of the data.
    //
    // This operation read a file whole or refused it, and the cap is one
    // mebibyte because it runs in the machine's login shell on a board with no
    // swap. A 1080p screen capture is usually under that and **sometimes is
    // not**: `rivet-01` drawing Contours produced a 1,090,017-byte QOI, and
    // `cargo rivet screenshot` — whose entire job is to fetch that file —
    // failed. Not for a broken screen, but for an interesting one.
    //
    // *"Screenshots work unless the picture is busy"* is a broken tool, and
    // the cap was the right instinct applied at the wrong level: it bounds
    // what one ANSWER may carry, which is a memory question, and it was being
    // used to bound what a FILE may be, which is not.
    //
    // So the cap stays exactly where it is and a caller may now say WHERE to
    // start. Each answer is still at most `FILE_MAX_BYTES`; a large file takes
    // several. `content_offset` and `content_complete` are how an answer says
    // which piece this is and whether there is more.
    let at = match arguments.get(2).copied() {
        Some(word) => Some(window_number(word, "the third word", "an offset in bytes")?),
        None => None,
    };
    let want = match arguments.get(3).copied() {
        Some(word) => Some(window_number(word, "the fourth word", "a length in bytes")?),
        None => None,
    };
    if let Some(extra) = arguments.get(4) {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "`{COMMAND}` takes a path, an optional byte offset and an optional length, and \
                 {extra:?} is a fifth word. Select further with a JSON tool rather than with \
                 an argument this build does not have."
            ),
        ));
    }

    super::filesystem::check_readable_path(COMMAND, path)?;

    let stat = editor
        .stat_path(path)
        .map_err(|e| super::filesystem::refused(e, path))?;

    let mut fields = vec![
        ("subject", Value::str(SUBJECT)),
        ("path", Value::str(path)),
        ("present", Value::Bool(stat.present())),
    ];

    let Some(kind) = stat.kind else {
        // **Nothing is there, and that is the whole answer.** The meta-info
        // fields are absent rather than zeroed, because a mode of 0000 and a
        // size of 0 are real values and this path has neither.
        if let Some(link) = &stat.symlinked_ancestor {
            fields.push(("symlinked_ancestor", Value::str(link.clone())));
        }
        push_advice(&mut fields, path, &stat);
        fields.push(("next", Value::str(next_for_absent(path))));
        return Ok(Value::object(fields));
    };

    fields.push(("type", Value::str(kind.as_str())));
    fields.push(("size_bytes", Value::Integer(count(stat.size_bytes))));
    if let Some(mode) = stat.mode {
        fields.push(("mode", Value::str(format!("{mode:04o}"))));
        fields.push((
            "mode_rendered",
            Value::str(super::filesystem::render_mode(mode)),
        ));
    }
    if let Some(uid) = stat.uid {
        fields.push(("uid", Value::Integer(i64::from(uid))));
    }
    if let Some(user) = &stat.user {
        fields.push(("user", Value::str(user.clone())));
    }
    if let Some(gid) = stat.gid {
        fields.push(("gid", Value::Integer(i64::from(gid))));
    }
    if let Some(group) = &stat.group {
        fields.push(("group", Value::str(group.clone())));
    }
    if let Some(ms) = stat.modified_ms {
        fields.push(("modified_ms", Value::Integer(count(ms))));
    }
    if let Some(target) = &stat.link_target {
        fields.push(("link_target", Value::str(target.clone())));
    }

    if kind == PathKind::File {
        match content(path, editor, at, want)? {
            // **The encoding travels with the value**, never separately: a
            // caller that read `content` without `content_encoding` would be
            // guessing, which is the thing this subject refuses to make anybody
            // do.
            Ok(read) => {
                fields.push(("content", Value::str(read.encoded.value().to_string())));
                fields.push((
                    "content_encoding",
                    Value::str(read.encoded.word().to_string()),
                ));
                fields.push(("content_offset", Value::Integer(count(read.at))));
                fields.push(("content_bytes", Value::Integer(count(read.bytes))));
                fields.push(("content_complete", Value::Bool(read.complete)));
            }
            Err(why) => fields.push(("content_omitted", Value::str(why))),
        }
    } else {
        fields.push((
            "content_omitted",
            Value::str(format!(
                "this is a {kind} and not a regular file, so it has no contents to read"
            )),
        ));
    }

    if kind == PathKind::Directory {
        fields.push((
            "entry_count",
            Value::Integer(i64::try_from(stat.entry_count).unwrap_or(i64::MAX)),
        ));
        fields.push((
            "entries",
            Value::List(
                stat.entries
                    .iter()
                    .map(|entry| {
                        Value::object(vec![
                            ("name", Value::str(entry.name.clone())),
                            ("type", Value::str(entry.kind.as_str())),
                            ("size_bytes", Value::Integer(count(entry.size_bytes))),
                        ])
                    })
                    .collect(),
            ),
        ));
        fields.push(("entries_truncated", Value::Bool(stat.entries_truncated)));
    }

    if let Some(link) = &stat.symlinked_ancestor {
        fields.push(("symlinked_ancestor", Value::str(link.clone())));
    }
    fields.push(("mount_point", Value::Bool(stat.mount_point)));
    push_advice(&mut fields, path, &stat);
    fields.push(("next", Value::str(next_for(kind))));

    debug_assert!(
        super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)),
        "a `get file` result's keys must be a subsequence of OUTPUTS, in order"
    );
    Ok(Value::object(fields))
}

/// The three fields that say what may be done with this path, in `OUTPUTS`'s
/// order.
///
/// One function because a present and an absent path both carry them, and the
/// question they answer is the same either way — *may I change this* is worth
/// asking about a path that is not there, since that is what `create-file` and
/// `rename` need to know.
fn push_advice(fields: &mut Vec<(&'static str, Value)>, path: &str, stat: &PathStat) {
    if let Ok(explanation) = super::explain::perform(&[path]) {
        fields.push((
            "generated",
            Value::object(vec![
                ("kind", Value::str(explanation.kind_word())),
                ("derived_from", Value::str(explanation.derived_from())),
                ("writable", Value::Bool(explanation.writable())),
                ("change_with", Value::str(explanation.changed_by())),
            ]),
        ));
    }

    if let Some(zone) = super::filesystem::app_zone(path) {
        let mut app = vec![("zone", Value::str(zone_word(zone)))];
        if let Some(note) = zone.note() {
            app.push(("note", Value::str(note)));
        }
        fields.push(("application", Value::object(app)));
    }

    // **The gate's own answer, never a second opinion.** If this ever disagreed
    // with what `apply remove-file` does, the disagreement would be discovered
    // by an operator who had been told they could.
    match super::filesystem::check_writable_path("apply remove-file", path) {
        Ok(()) => {
            // The machine's half of the gate is not run here — nothing is being
            // changed — so a symlinked ancestor is reported as the refusal it
            // will be rather than left for the attempt to discover.
            if let Some(link) = &stat.symlinked_ancestor {
                fields.push(("changeable", Value::Bool(false)));
                fields.push((
                    "unchangeable_because",
                    Value::str(format!(
                        "{link}, on the way to this path, is a SYMBOLIC LINK. Every writing \
                         subject of ADR-006 §8.10 refuses such a path: they decide what they \
                         may touch by testing the text against {root}, and a link in the \
                         middle means the path resolves somewhere that test never looked.",
                        root = super::filesystem::PERMITTED_ROOT,
                    )),
                ));
            } else {
                fields.push(("changeable", Value::Bool(true)));
            }
        }
        Err(refusal) => {
            fields.push(("changeable", Value::Bool(false)));
            fields.push(("unchangeable_because", Value::str(refusal.message())));
        }
    }
}

/// The word an `application` answer carries for its zone.
const fn zone_word(zone: super::filesystem::AppZone) -> &'static str {
    use super::filesystem::AppZone;
    match zone {
        AppZone::Mechanism(_) => "mechanism",
        AppZone::Program => "program",
        AppZone::Manifest => "manifest",
        AppZone::Version => "version",
        AppZone::Data => "data",
        AppZone::Settings => "settings",
        AppZone::Routes => "routes",
        AppZone::NameService => "name-service",
        AppZone::ImageService => "image-service",
        AppZone::Scratch => "scratch",
    }
}

// ~~`fn base64` lived here~~ — **moved to [`crate::base64`] on 2026-09-01.**
//
// It was written here rather than depended on for `ADR-009` §3.4's reason, and
// that reason is unchanged and is now stated in the new module. What changed is
// that a *decoder* for the same format had been sitting in another module since
// long before this encoder was written, and neither file mentioned the other.
// `cargo rivet screenshot` reads what this function writes — a captured frame,
// off a board, over the control plane — and it was about to be the third copy.
//
// `ADR-011`: one fact, one owner. The vectors that checked this encoder moved
// with it and are in `crate::base64`'s tests.

/// The file's text, or the sentence saying why it is not here.
///
/// The outer `Result` is a machine refusal — the file is there and could not be
/// read — and the inner one is *"it was read and cannot honestly be given as
/// text"*. **Two different things and two different shapes**, because the first
/// means something is wrong with the machine and the second means the file is
/// not what the caller assumed.
/// One of the window's two numbers, or a refusal that says which word was
/// wrong and what it should have been.
///
/// **Refused rather than clamped.** A caller that wrote `1e6` or `1_048_576`
/// meant a number, and reading it as zero would hand back the first piece of
/// the file and look like it worked.
fn window_number(word: &str, which: &str, what: &str) -> Result<usize, OperationError> {
    word.parse::<usize>().map_err(|_| {
        OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "`{COMMAND}` takes {what} as {which} and {word:?} is not one. Digits only, \
                 with no sign, no separators and no unit."
            ),
        )
    })
}

/// One window of a file, and where in the file it came from.
pub struct Read {
    pub encoded: Encoded,
    /// Where this window starts, in bytes from the beginning.
    pub at: u64,
    /// How many bytes of the FILE this window covers — not how long
    /// `content` is, which is longer when the encoding is base64.
    pub bytes: u64,
    /// Whether the file ends here. **`false` is the signal to ask again**
    /// with `at + bytes` as the offset.
    pub complete: bool,
}

fn content(
    path: &str,
    editor: &mut dyn Editor,
    at: Option<usize>,
    want: Option<usize>,
) -> Result<Result<Read, String>, OperationError> {
    let at = at.unwrap_or(0);
    // **The cap bounds the ANSWER and no longer bounds the file.** A caller
    // that names no length gets as much as one answer may carry, which keeps
    // every existing caller working unchanged: a file under the cap still
    // comes back whole and `content_complete` is `true`. Computed BEFORE the
    // read, so the read below asks the port for at most this many bytes
    // rather than reading everything and deciding how much to keep
    // afterward — the allocation a caller windowing a large file is trying
    // to avoid in the first place.
    let most = super::edit::FILE_MAX_BYTES;
    let take = want.unwrap_or(most).min(most);

    let window = editor
        .read_file_window(path, at, take)
        .map_err(|e| super::filesystem::refused(e, path))?;
    let Some(window) = window else {
        // The stat said there is a regular file here and the reader says there
        // is not. Reported rather than smoothed over: it is a race with
        // something else on the machine, and saying so is more useful than an
        // empty `content`.
        return Ok(Err(
            "this path held a regular file when it was examined and held nothing when it was \
             read, a moment later. Something else on this machine removed it in between."
                .to_string(),
        ));
    };

    let held = window.total_bytes;
    if at > held {
        return Ok(Err(format!(
            "this file holds {held} bytes and the offset asked for starts at {at}, which is \
             past its end. `size_bytes` is what to compare an offset against."
        )));
    }
    let remaining = held - at;
    if take == 0 && remaining > 0 {
        return Ok(Err(format!(
            "a length of zero reads nothing, and this file holds {remaining} more bytes from \
             offset {at}. Name a length, or name none and get the most one answer carries."
        )));
    }
    // **However much the window actually held** — bounded by `take` above
    // and, at the end of the file, by however many bytes were left; the two
    // together are the identical amount `held.min(take).min(remaining)`
    // would have named, without this function having to compute `remaining`
    // before knowing whether the read even succeeded.
    let bytes = window.bytes;
    let complete = at + bytes.len() == held;

    // **Text if it is text, base64 if it is not** — changed 2026-09-01,
    // ruled by the maintainer (`ADR-006` §8.10).
    //
    // ~~This build will not guess an encoding.~~ It still does not: base64
    // is not a guess, it is the bytes, and `content_encoding` says which of
    // the two a caller is holding. The old behaviour was a REFUSAL — the
    // sentence is kept in `content_encoding`'s description — and the reason
    // it changed is that a Rivet now produces binary files an operator has
    // a legitimate need to read off the machine: a captured frame is the
    // case that forced it (the maintainer's 2026-09-01 kiosk-lessee ruling,
    // `ADR-036` §2), and a core dump is the next one.
    //
    // Replacement characters are still refused, and that was the actual
    // rule: `AGENTS.md` §5 forbids a guess that does not fail. Base64 does
    // not guess and cannot be mistaken for text.
    //
    // **A window of a text file may split a character**, and that is why the
    // fallback matters here as well as for binary: bytes 0..N of valid UTF-8
    // can end mid-sequence, and the piece comes back as base64 rather than as
    // a refusal or a replacement character. A caller reassembling windows
    // concatenates the BYTES and decodes once at the end.
    let encoded = match core::str::from_utf8(&bytes) {
        Ok(text) => Encoded::Text(text.to_string()),
        Err(_) => Encoded::Base64(crate::base64::encode(&bytes)),
    };
    Ok(Ok(Read {
        encoded,
        at: at as u64,
        bytes: bytes.len() as u64,
        complete,
    }))
}

/// What `content` holds, and how the caller must read it.
///
/// **Two variants and not a flag beside a string**, so a caller cannot read
/// the value without having been told which it is. That is the same reason
/// `content_omitted` exists rather than an empty `content`.
pub enum Encoded {
    /// Valid UTF-8, verbatim.
    Text(String),
    /// `RFC 4648` §4 base64 of the file's bytes, padded, no line breaks.
    Base64(String),
}

impl Encoded {
    /// The value to put in `content`.
    #[must_use]
    pub fn value(&self) -> &str {
        match self {
            Self::Text(t) | Self::Base64(t) => t,
        }
    }

    /// What to put in `content_encoding`.
    #[must_use]
    pub const fn word(&self) -> &'static str {
        match self {
            Self::Text(_) => "text",
            Self::Base64(_) => "base64",
        }
    }
}

/// What this answer settles, for a path with something at it.
fn next_for(kind: PathKind) -> String {
    let common = "THIS IS A READING TAKEN AT ONE MOMENT and nothing holds it still: another \
                  process on this machine can change or remove this path between this answer \
                  and whatever you do next. Every writing subject of ADR-006 §8.10 re-runs its \
                  refusals at the moment it acts rather than trusting an earlier reading — but \
                  none of them compares against THIS answer, because nothing carries a reading \
                  between commands (ADR-006 §8.10.16).";
    match kind {
        PathKind::File => format!(
            "{common} `rivet apply edit <path> --expect <text> --replace <text>` changes what \
             is in this file, and `rivet get diff <path> <text>` says what such a change would \
             do before it is made."
        ),
        PathKind::Directory => format!(
            "{common} `entries` is what is directly inside — this operation does not walk the \
             tree, and there is no operation that does."
        ),
        PathKind::Symlink => format!(
            "{common} THIS IS A LINK AND IT WAS NOT FOLLOWED: `link_target` is what it names, \
             and asking about that path is a second `rivet get file`. No writing subject of \
             ADR-006 §8.10 will touch a link or a path through one."
        ),
        PathKind::Other => format!(
            "{common} This is neither a file nor a directory nor a link — a device node, a \
             socket or a fifo — and no subject in this build reads or changes one."
        ),
    }
}

/// What this answer settles, for a path with nothing at it.
fn next_for_absent(path: &str) -> String {
    format!(
        "NOTHING IS AT {path}, which is an answer and not a failure — the path was examined \
         and there is nothing there. It is NOT a claim that there never was: this operation \
         reads the machine and keeps no history, and `rivet get log` is where a record of a \
         removal would be. `changeable` says whether the writing subjects of ADR-006 §8.10 \
         would accept this path, which is the question worth asking before `apply create-file` \
         or `apply rename`."
    )
}

/// A count that has to become an `i64` for [`Value::Integer`].
///
/// Saturating rather than `as`, on `super::edit`'s reasoning: a length that
/// wrapped negative would make the one field saying how big a file is
/// unreadable, and this crate denies the lint that would have caught it.
fn count(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

#[cfg(test)]
#[path = "file_tests.rs"]
mod tests;
