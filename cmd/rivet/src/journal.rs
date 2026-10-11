//! The journal, from this binary's two sides: **reading** `/state/log`
//! (`ADR-022` §9) and **sending** a record to the one process that writes it
//! (`ADR-022` §11).
//!
//! # Reading is a read of files; writing is not a write of files
//!
//! ~~`crate::edit` appends its record to `ADR-022` §8's active file directly,
//! and that is a second writer of a file the record gives to one owner.~~
//! **Removed 2026-08-24, and the removal is the point of this module's second
//! half.** `PLAN.md` §10.6 put four options to the maintainer and he rejected
//! the obvious one — amending `ADR-022` §7 to permit the CLI — as a bandaid, on
//! two grounds that are both right: it answers *may the CLI write too* when the
//! question is *how does a process the supervisor did not spawn file a record*,
//! which needs an amendment per writer for ever; and two processes appending
//! JSONL concurrently can interleave a partial line, manufacturing exactly the
//! unparsable lines `ADR-022` §9.4's `unreadable` counter exists to count.
//!
//! *(These three cited ~~`ADR-022` §4.2~~ until 2026-08-29. **There is no §4.2
//! in `ADR-022`** — the document noticed that about itself on 2026-08-24 and
//! struck it inside §9.4, and the three sentences here that quoted it were not
//! followed through. `AGENTS.md` §7: when correcting one, grep for the claim
//! rather than the paragraph. The field itself is declared in
//! `rivet_core::operation::log`, which is the authority; §9.4 is where the
//! correction lives.)*
//!
//! **Ruled 2026-08-24: the supervisor holds the journal open and offers an
//! ingest socket.** `ADR-022` §7 stays unamended — one writer, still the
//! supervisor — and [`send`] is how this binary reaches it.
//!
//! So the two halves of this module are not symmetrical, and that asymmetry is
//! the design rather than an inconsistency:
//!
//! | | Reading | Filing a record |
//! | :-- | :-- | :-- |
//! | Reaches | the files on `/state/log` | `rivet_core::journal::INGEST_SOCKET_PATH` |
//! | Because | a reader cannot corrupt what it reads | a second writer can, and did not have to exist |
//! | When it cannot | reports a refusal naming what it could not read | writes the record to **stderr** and says so ([`send`]) |
//!
//! # Reading `/state/log` — the adapter half of `ADR-022` §9
//!
//! `rivet_core::operation::get` is the operation; nothing else on the machine
//! knows the journal is *files* — that knowledge stays behind the seam
//! between them, `Machine::read_journal`, which hands back **lines** rather
//! than records, and
//! that trait's own documentation carries the argument: `ADR-022` §3 makes the
//! JSONL form authoritative and `rivet_core::journal` is its one reader, so
//! an adapter that parsed would be a second one.
//!
//! # What this module decides, and it is exactly two things
//!
//! **Which files, and in what order.** `rivet_core::state::log_files_oldest_
//! first` names them and the order is its contract, not this module's choice —
//! `rivet_core::journal::boots_in_order` treats the sequence as the authority
//! for which boot came first, because a boot identifier is a random UUID and a
//! machine with no RTC starts every boot at the epoch (`ADR-021`).
//!
//! Everything else — what a line means, which boot to keep, how to render one —
//! is the domain's.
//!
//! # A missing file is not a failure, and a missing *directory* is not either
//!
//! `ADR-022` §8 rotates and drops the oldest, so most of the names are usually
//! absent; a machine that has just been provisioned has none of them. Both are
//! ordinary, and `get log`'s answer distinguishes them from a real fault by
//! reporting `files` and `source` alongside the records.
//!
//! **What IS reported as a refusal** is a file that exists and cannot be read —
//! a permission problem, or `/state` mounted in a way that refuses. That is not
//! "this machine logged nothing" and must not be shown as it (`AGENTS.md` §5).
//!
//! # It runs on a workstation, and answers honestly there
//!
//! Nothing here needs `mount(2)`, a device tree or root — it is `read_to_string`
//! over absolute paths. So a detached build reads the same way and finds
//! nothing, which is a *true* statement about a machine that is not a Rivet.
//! `crate::boot::Detached` refuses the operations that genuinely cannot work
//! there; this is not one of them, and refusing it would have been the kind of
//! stub `AGENTS.md` §3 calls worse than useless — an error where the honest
//! answer is "no journal here, and here is where I looked".

use rivet_core::operation::{JournalRead, MachineError};
use rivet_core::state::{ACTIVE_LOG, LOG_DIR, log_files_oldest_first, rotated_log};

/// Read every journal file this machine has, oldest first.
///
/// The lines of each file, concatenated in file order — which is append order,
/// which is what makes `--boot previous` mean the boot before this one.
pub fn read() -> Result<JournalRead, MachineError> {
    read_with(|path| std::fs::read_to_string(path))
}

/// The same, over an injected reader.
///
/// The seam exists because the interesting cases are a file that is **absent**
/// and a file that is **unreadable**, and the two must not be confused — a
/// developer's machine can produce the first on demand and not the second.
fn read_with<F>(mut open: F) -> Result<JournalRead, MachineError>
where
    F: FnMut(&str) -> std::io::Result<String>,
{
    let mut lines = Vec::new();
    let mut files = Vec::new();
    for path in log_files_oldest_first() {
        match open(&path) {
            Ok(text) => {
                lines.extend(text.lines().map(str::to_string));
                files.push(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(MachineError::Refused(format!(
                    "{path} is there and could not be read: {e}. That is not an empty journal \
                     — the records are on the card and something is between them and this \
                     process."
                )));
            }
        }
    }
    Ok(JournalRead {
        lines,
        files,
        source: LOG_DIR.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Following — reading only what has been appended since last time
// ---------------------------------------------------------------------------

/// What one read of the appended bytes produced.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Appended {
    /// Complete lines only, in append order. **A line without its newline is
    /// left where it is** and read again next time: the supervisor appends a
    /// record with one `write`, but a reader that took a half-written line
    /// would manufacture exactly the unparsable lines `ADR-022` §9.4's
    /// `unreadable` counter exists to count.
    pub lines: Vec<String>,
    /// The active file was rotated between this read and the last one
    /// (`ADR-022` §8). Reported because it explains a gap if there was one, and
    /// because it is the moment a follower's offset stops meaning anything.
    pub rotated: bool,
    /// Records that were written and this follower did not see, and why.
    ///
    /// `None` is the ordinary case, **including across a rotation**: the tail
    /// of the file that was just rotated away is read out of its new name
    /// before the new active file is. It is `Some` when even that was gone —
    /// two rotations inside one poll interval, which takes a service writing
    /// two megabytes in half a second.
    pub missed: Option<String>,
}

/// Where a follower's bytes come from.
///
/// **An adapter's own I/O seam, not a domain port** (`AGENTS.md` §3.1): its
/// vocabulary is files and byte offsets, which `rivet-core` is forbidden to
/// name. What the core decides is which of these lines' records to emit —
/// `rivet_core::journal::Follow`.
pub trait Appending {
    /// Everything appended since the previous call.
    ///
    /// **The first call is the backfill** and returns the whole active file, so
    /// that the bytes a follower opens with and the offset it continues from
    /// come from one read and cannot race each other. `PLAN.md` §21.2 carries
    /// why the backfill is the active file rather than the whole journal.
    fn appended(&mut self) -> Result<Appended, MachineError>;

    /// Which file the lines are coming from, for the follower's summary.
    fn source(&self) -> &str;
}

/// One read of one file: its bytes from an offset, its length, and **which
/// file it actually is**.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Chunk {
    text: String,
    end: u64,
    /// The inode. `ADR-022` §8 rotates by `rename`
    /// (`cmd/rivet-sysd`'s `Journal::rotate`), so the file at `ACTIVE_LOG`
    /// after a rotation is a *different file* with the same name — and this is
    /// the only thing that says so. A length comparison cannot: the new file
    /// grows, and a poll that arrives after it has passed the old offset sees a
    /// longer file at the same name and reads from the middle of it.
    ///
    /// **That is not hypothetical — it is what the first version of this did**,
    /// and `a_rotation_carries_the_tail_of_the_old_file_before_the_new_one`
    /// caught it.
    ///
    /// `0` on a platform with no inodes, which is the development host and
    /// never a board. Rotation there falls back to the length check, and there
    /// is no journal on a Windows machine to be wrong about.
    id: u64,
}

/// [`Appending`] over `ADR-022` §8's active file.
#[derive(Debug, Clone, Default)]
pub struct ActiveFile {
    /// How many bytes of complete lines have been consumed.
    at: u64,
    /// Which file those bytes came out of — see [`Chunk::id`].
    id: Option<u64>,
    /// Whether [`Appending::appended`] has been called at all.
    opened: bool,
}

impl ActiveFile {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Appending for ActiveFile {
    fn appended(&mut self) -> Result<Appended, MachineError> {
        self.read_with(read_from)
    }

    fn source(&self) -> &str {
        ACTIVE_LOG
    }
}

/// The bytes of `path` from `from` onwards, its length, and its identity.
///
/// All three are taken **from one open file**, so a record appended between a
/// `metadata` call and a read cannot make them disagree.
fn read_from(path: &str, from: u64) -> std::io::Result<Chunk> {
    use std::io::{Read as _, Seek as _, SeekFrom};

    let mut file = std::fs::File::open(path)?;
    let id = identity(&file);
    let end = file.seek(SeekFrom::End(0))?;
    if end <= from {
        return Ok(Chunk {
            text: String::new(),
            end,
            id,
        });
    }
    file.seek(SeekFrom::Start(from))?;
    let mut text = String::new();
    // `read_to_string` refuses a file that is not UTF-8 rather than replacing
    // the bytes it cannot read: a lossy conversion inside a `msg` field would
    // produce a record that parses and is not what the machine wrote, which is
    // the one failure mode `AGENTS.md` §5 rules out entirely.
    file.read_to_string(&mut text)?;
    Ok(Chunk { text, end, id })
}

/// The inode, where there is one. See [`Chunk::id`].
#[cfg(unix)]
fn identity(file: &std::fs::File) -> u64 {
    use std::os::unix::fs::MetadataExt as _;

    file.metadata().map(|m| m.ino()).unwrap_or(0)
}

/// See [`Chunk::id`]: no inodes here, and no journal here either.
#[cfg(not(unix))]
fn identity(_file: &std::fs::File) -> u64 {
    0
}

impl ActiveFile {
    /// The whole of [`Appending::appended`], over an injected reader.
    ///
    /// The seam is here for [`read_with`]'s reason and one more: **rotation is
    /// the interesting case and it cannot be provoked on demand** — it needs a
    /// megabyte of records — so the test drives it directly.
    fn read_with<F>(&mut self, mut open: F) -> Result<Appended, MachineError>
    where
        F: FnMut(&str, u64) -> std::io::Result<Chunk>,
    {
        let first = !self.opened;
        self.opened = true;

        let chunk = match open(ACTIVE_LOG, self.at) {
            Ok(got) => got,
            // A machine whose journal has never been written, or a build that
            // is not on a board at all. Not a failure — `read_with`'s reason.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Appended::default());
            }
            Err(e) => return Err(refused(ACTIVE_LOG, &e)),
        };

        // **Two ways to notice the same thing**, and neither is redundant. The
        // file under this name is a different file from the one that was read
        // last time (`ADR-022` §8 rotates by renaming, so it always is), or it
        // is shorter than what has already been consumed (which is what a
        // truncation would look like, and what an inode-less platform has left
        // to go on). Nothing about either is an error.
        let replaced = self.id.is_some_and(|was| was != chunk.id);
        if !first && (replaced || chunk.end < self.at) {
            return self.after_a_rotation(&mut open);
        }
        self.id = Some(chunk.id);

        let (lines, used) = complete_lines(&chunk.text);
        self.at += used;
        Ok(Appended {
            lines,
            rotated: false,
            missed: None,
        })
    }

    /// Read the tail of the file that was just rotated away, then the new one.
    ///
    /// **The order is the whole point.** The records this follower had not
    /// reached are not lost by rotation — they are in `journal.1.jsonl` under a
    /// new name, at the same offsets — so they are read first and emitted
    /// first, and append order survives.
    fn after_a_rotation<F>(&mut self, open: &mut F) -> Result<Appended, MachineError>
    where
        F: FnMut(&str, u64) -> std::io::Result<Chunk>,
    {
        let carried = rotated_log(1);
        let mut lines = Vec::new();
        let mut missed = None;

        match open(&carried, self.at) {
            // The identity is what makes this the file that was rotated away
            // rather than one that was rotated away two polls ago.
            Ok(chunk) if self.id.is_none_or(|was| was == chunk.id) && chunk.end >= self.at => {
                let (mut tail, _) = complete_lines(&chunk.text);
                lines.append(&mut tail);
            }
            // `journal.1.jsonl` is not the file this follower was reading
            // either — it rotated twice inside one poll. What was between the
            // two is genuinely gone from this stream, and saying so is the
            // whole of `AGENTS.md` §5 here.
            Ok(_) | Err(_) => {
                missed = Some(
                    "the journal rotated more than once between two reads, so records \
                     written before it are not in this stream. They are on the card: \
                     `rivet get log all all` reads every file ADR-022 §8 still holds."
                        .to_string(),
                );
            }
        }

        let chunk = match open(ACTIVE_LOG, 0) {
            Ok(got) => got,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Chunk {
                text: String::new(),
                end: 0,
                id: 0,
            },
            Err(e) => return Err(refused(ACTIVE_LOG, &e)),
        };
        let (mut fresh, used) = complete_lines(&chunk.text);
        lines.append(&mut fresh);
        self.at = used;
        self.id = Some(chunk.id);

        Ok(Appended {
            lines,
            rotated: true,
            missed,
        })
    }
}

/// Split into complete lines, and say how many bytes those lines used.
///
/// The remainder — a line with no newline yet — is deliberately **not**
/// returned and not counted, so the next read starts at its first byte and sees
/// it whole.
fn complete_lines(text: &str) -> (Vec<String>, u64) {
    let mut lines = Vec::new();
    let mut used = 0usize;
    for line in text.split_inclusive('\n') {
        if !line.ends_with('\n') {
            break;
        }
        used += line.len();
        let line = line.trim_end_matches(['\n', '\r']);
        if !line.is_empty() {
            lines.push(line.to_string());
        }
    }
    (lines, used as u64)
}

/// The refusal a file that exists and will not open produces.
///
/// The same sentence [`read_with`] uses, because it is the same fact and a
/// second wording of it would be a second answer (`ADR-011`).
fn refused(path: &str, e: &std::io::Error) -> MachineError {
    MachineError::Refused(format!(
        "{path} is there and could not be read: {e}. That is not an empty journal — the \
         records are on the card and something is between them and this process."
    ))
}

/// Replace journal records with `ADR-022` §2's human view, for the text
/// reading only.
///
/// `rivet_core::operation::get` returns each record as the **JSONL line the
/// machine wrote**, which is the authoritative form (`ADR-022` §3) and is what
/// a pipeline wants. A person reading a terminal wants
/// `[     1.319] error rivet-timed: REFUSED …`, which is
/// `rivet_core::journal::Record::to_text` — §2's *view*.
///
/// **The value is not changed for `--output json`.** Both readings are of the
/// same structure (`ADR-006` §4), and the structure is the record; this is the
/// rendering step and nothing above it sees the difference.
///
/// # Why it matches on a field name
///
/// Because there is nothing else to match on: `rivet_core::value::Value` is a
/// document, not a schema, so "these strings are journal records" is not
/// something the value carries. The narrowing that makes it safe is that a line
/// is only rewritten **if it parses as a record** — so a `records` field that
/// held anything else is passed through untouched rather than mangled, and an
/// operation that grows a `records` field of its own does not silently acquire
/// this behaviour.
///
/// The alternative — a `Value` variant meaning "renderable as text some other
/// way" — is a change to `ADR-006` §7's shape, and this is one field of one
/// operation.
pub fn as_text_view(value: &rivet_core::Value) -> rivet_core::Value {
    use rivet_core::Value as V;
    use rivet_core::journal::Record;

    match value {
        V::Object(fields) => V::Object(
            fields
                .iter()
                .map(|(name, v)| {
                    if *name == RECORDS_FIELD {
                        (*name, as_text_view(v))
                    } else {
                        (*name, v.clone())
                    }
                })
                .collect(),
        ),
        V::List(items) => V::List(items.iter().map(as_text_view).collect()),
        V::Str(text) => match Record::from_jsonl(text) {
            Ok(record) => V::str(record.to_text()),
            // Not a record: left exactly as it was. A refusal here would make
            // the text reading fail on a value the JSON reading renders fine,
            // which `ADR-006` §4 forbids — both are readers of one structure.
            Err(_) => value.clone(),
        },
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// Filing a record — `ADR-022` §11
// ---------------------------------------------------------------------------

/// Send one record to the supervisor's journal.
///
/// **One connection, one record, close.** The line goes on the wire exactly as
/// `rivet_core::journal::Record::to_jsonl` renders it — which is the form
/// `ADR-022` §3 makes authoritative, so nothing is encoded twice and there is no
/// second format for the two sides to disagree about.
///
/// Returns why it could not, and **the caller must not treat that as the record
/// being lost**: [`send_or_say`] is what this binary actually calls, and its
/// floor is stderr.
///
/// # What it deliberately does not do
///
/// * **It does not retry.** A supervisor that is not listening at this instant
///   is one that is down or restarting (`ADR-017` §3.1 expects both), and a
///   login shell that blocked an operator's command retrying a socket would be
///   worse than one that printed the record.
/// * **It does not read a reply**, because there is none. The supervisor
///   stamping and writing the record is not something this process waits on:
///   waiting would make every `rivet apply edit` depend on the supervisor's
///   scheduling, and the thing worth knowing — *did the bytes leave* — is what
///   `write_all` already answers.
/// * **It sends nothing on a build with no board.** `crate::edit`'s interlock
///   has already refused there.
pub fn send(line: &str) -> Result<(), String> {
    send_to(rivet_core::journal::INGEST_SOCKET_PATH, line)
}

/// The same, to a named socket.
///
/// The path is a parameter for [`crate::edit::FileStore`]'s reason and
/// `AGENTS.md` §8.3's: a test binds one in a temporary directory and drives the
/// real syscalls, and nothing here writes to a path a development host might
/// actually have.
fn send_to(path: &str, line: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::io::Write as _;

        if line.len() > rivet_core::journal::INGEST_MAX_BYTES {
            // Refused here rather than sent and cut at the far end: the
            // supervisor would drop an over-long line whole (`ADR-022` §11.5),
            // and a sender that made it produce that refusal instead of putting
            // its own record on stderr would have lost the record to be tidy.
            return Err(format!(
                "the record is {} bytes and the ingest socket takes at most {}",
                line.len(),
                rivet_core::journal::INGEST_MAX_BYTES
            ));
        }
        let mut socket =
            std::os::unix::net::UnixStream::connect(path).map_err(|e| e.to_string())?;
        socket.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
        // The close is the frame terminator as much as the newline is, so it is
        // explicit rather than left to the end of the function.
        drop(socket);
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (path, line);
        Err("this platform has no AF_UNIX sockets, and no Rivet journal to reach".to_string())
    }
}

/// Send a record, or put it on stderr — and say which.
///
/// **This is `ADR-022` §11.5's floor and the reason it is stderr.** A machine
/// whose supervisor is not listening is already in trouble; the *record of that*
/// is exactly what must survive, and this process has one channel that is
/// certainly still there. It is `stderr` and not `stdout` because stdout is the
/// operator's structured answer (`ADR-006` §4) and a record written into it
/// would corrupt the value an agent is parsing.
///
/// **The line, not a summary.** What lands on stderr is the record itself, so an
/// operator can paste it into the journal later and an agent reading the session
/// has the same structure it would have got from `rivet get log`.
///
/// Returns the sentence the operation reports as `unrecorded`, or `None`.
pub fn send_or_say(line: &str) -> Option<String> {
    let detail = send(line).err()?;
    eprintln!("rivet: this record is NOT in the journal, and here it is instead:");
    eprint!("{line}");
    if !line.ends_with('\n') {
        eprintln!();
    }
    Some(format!(
        "the record could not be sent to {socket}, so it is on this session's stderr and NOT in \
         the journal — `rivet get log` will not have it. The usual cause is a supervisor that \
         is not running (ADR-022 §11). THE FILE WAS STILL CHANGED. Detail: {detail}",
        socket = rivet_core::journal::INGEST_SOCKET_PATH,
    ))
}

/// The field `rivet_core::operation::get` puts records in.
///
/// Named here rather than spelled inline so the two places that care — this
/// module and that operation's catalogue entry — are greppable as one fact.
const RECORDS_FIELD: &str = "records";

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use rivet_core::state::{ACTIVE_LOG, LOG_FILES_KEPT, rotated_log};
    use std::collections::BTreeMap;
    use std::io::{Error, ErrorKind};

    fn reader(
        files: BTreeMap<String, std::io::Result<String>>,
    ) -> impl FnMut(&str) -> std::io::Result<String> {
        move |path| match files.get(path) {
            Some(Ok(text)) => Ok(text.clone()),
            Some(Err(e)) => Err(Error::new(e.kind(), e.to_string())),
            None => Err(Error::new(ErrorKind::NotFound, "no such file")),
        }
    }

    fn present(pairs: &[(&str, &str)]) -> BTreeMap<String, std::io::Result<String>> {
        pairs
            .iter()
            .map(|(p, t)| ((*p).to_string(), Ok((*t).to_string())))
            .collect()
    }

    /// A card whose files a test can rewrite between two reads, in the shape
    /// [`ActiveFile::read_with`] wants: bytes from an offset, and the file's
    /// whole length.
    ///
    /// **The length comes from the same map entry the bytes do**, which is the
    /// property the real reader gets from reading and seeking one open file —
    /// a fake that could disagree with itself would prove nothing about the
    /// thing it stands for.
    #[derive(Clone, Default)]
    struct Card(std::rc::Rc<std::cell::RefCell<CardState>>);

    /// A file is its bytes **and its identity**, and rotation moves both under
    /// a new name — which is what `rename(2)` does and what the follower's
    /// rotation detection depends on.
    #[derive(Default)]
    struct CardState {
        files: BTreeMap<String, (String, u64)>,
        next_id: u64,
    }

    impl Card {
        fn holding(pairs: &[(&str, &str)]) -> Self {
            let card = Self::default();
            for (path, text) in pairs {
                card.write(path, text);
            }
            card
        }

        fn write(&self, path: &str, text: &str) {
            let mut state = self.0.borrow_mut();
            state.next_id += 1;
            let id = state.next_id;
            state.files.insert(path.to_string(), (text.to_string(), id));
        }

        fn append(&self, path: &str, text: &str) {
            let mut state = self.0.borrow_mut();
            state.next_id += 1;
            let id = state.next_id;
            state
                .files
                .entry(path.to_string())
                .or_insert_with(|| (String::new(), id))
                .0
                .push_str(text);
        }

        /// `ADR-022` §8's rotation, as `cmd/rivet-sysd` performs it: a
        /// rename, so the content keeps its identity and the new active file
        /// gets a fresh one.
        fn rotate(&self) {
            let mut state = self.0.borrow_mut();
            if let Some(active) = state.files.remove(ACTIVE_LOG) {
                state.files.insert(rotated_log(1), active);
            }
            state.next_id += 1;
            let id = state.next_id;
            state
                .files
                .insert(ACTIVE_LOG.to_string(), (String::new(), id));
        }

        fn open(&self) -> impl FnMut(&str, u64) -> std::io::Result<Chunk> + use<> {
            let state = std::rc::Rc::clone(&self.0);
            move |path, from| {
                let state = state.borrow();
                let Some((text, id)) = state.files.get(path) else {
                    return Err(Error::new(ErrorKind::NotFound, "no such file"));
                };
                let end = text.len() as u64;
                if end <= from {
                    return Ok(Chunk {
                        text: String::new(),
                        end,
                        id: *id,
                    });
                }
                Ok(Chunk {
                    text: text[from as usize..].to_string(),
                    end,
                    id: *id,
                })
            }
        }
    }

    #[test]
    fn the_first_read_is_the_whole_active_file_and_the_second_is_only_what_arrived() {
        let card = Card::holding(&[(ACTIVE_LOG, "one\ntwo\n")]);
        let mut tail = ActiveFile::new();

        let first = tail.read_with(card.open()).expect("a readable file");
        assert_eq!(first.lines, ["one", "two"]);
        assert!(!first.rotated);

        assert!(
            tail.read_with(card.open()).expect("still readable").lines.is_empty(),
            "a file nothing was appended to has nothing to hand over"
        );

        card.append(ACTIVE_LOG, "three\n");
        let third = tail.read_with(card.open()).expect("still readable");
        assert_eq!(third.lines, ["three"], "only what was appended");
    }

    /// **The half-written line.** The supervisor appends a record with one
    /// write, but a reader that took a line with no newline on it would
    /// manufacture exactly the unparsable lines `ADR-022` §9.4 counts.
    #[test]
    fn a_line_with_no_newline_yet_is_left_where_it_is_and_read_whole_next_time() {
        let card = Card::holding(&[(ACTIVE_LOG, "one\npart")]);
        let mut tail = ActiveFile::new();

        assert_eq!(tail.read_with(card.open()).expect("readable").lines, ["one"]);

        card.append(ACTIVE_LOG, "ial\n");
        assert_eq!(
            tail.read_with(card.open()).expect("readable").lines,
            ["partial"],
            "the line was held back and handed over whole"
        );
    }

    /// **Rotation does not lose the records a follower had not reached.** They
    /// are not gone — they are in `journal.1.jsonl` at the same offsets — so
    /// they are read out of the new name first and append order survives.
    #[test]
    fn a_rotation_carries_the_tail_of_the_old_file_before_the_new_one() {
        let card = Card::holding(&[(ACTIVE_LOG, "one\n")]);
        let mut tail = ActiveFile::new();
        assert_eq!(tail.read_with(card.open()).expect("readable").lines, ["one"]);

        // Two more records land, and then the file rotates before anything
        // read them.
        card.append(ACTIVE_LOG, "two\nthree\n");
        card.rotate();
        card.append(ACTIVE_LOG, "four\n");

        let got = tail.read_with(card.open()).expect("readable");
        assert!(got.rotated, "the rotation must be reported");
        assert_eq!(got.lines, ["two", "three", "four"]);
        assert_eq!(got.missed, None, "nothing was actually missed");

        card.append(ACTIVE_LOG, "five\n");
        assert_eq!(
            tail.read_with(card.open()).expect("readable").lines,
            ["five"],
            "the offset must have been reset to the NEW file"
        );
    }

    /// Two rotations inside one poll interval — a service writing two megabytes
    /// in half a second. The records are genuinely not in this stream, and
    /// saying so is the whole of `AGENTS.md` §5 here.
    #[test]
    fn records_a_double_rotation_carried_away_are_reported_as_missed() {
        let card = Card::holding(&[(ACTIVE_LOG, "one\n")]);
        let mut tail = ActiveFile::new();
        tail.read_with(card.open()).expect("readable");

        card.append(ACTIVE_LOG, "two\n");
        card.rotate();
        card.append(ACTIVE_LOG, "three\n");
        card.rotate();
        card.append(ACTIVE_LOG, "four\n");

        let got = tail.read_with(card.open()).expect("readable");
        assert!(got.rotated);
        assert_eq!(got.lines, ["four"]);
        let missed = got.missed.expect("a gap this size must not be silent");
        assert!(missed.contains("rotated more than once"), "{missed}");
    }

    /// A machine that has never written a record. The same answer `read_with`
    /// gives for the same reason — absent is not broken.
    #[test]
    fn an_absent_journal_is_an_empty_stream_rather_than_a_refusal() {
        let card = Card::default();
        let got = ActiveFile::new()
            .read_with(card.open())
            .expect("an absent journal is not a failure");
        assert_eq!(got, Appended::default());
    }

    #[test]
    fn a_journal_that_exists_and_will_not_open_is_a_refusal_naming_the_file() {
        let mut tail = ActiveFile::new();
        let got = tail.read_with(|_, _| Err(Error::new(ErrorKind::PermissionDenied, "denied")));
        let Err(e) = got else {
            panic!("an unreadable journal was reported as an empty one");
        };
        assert!(e.to_string().contains(ACTIVE_LOG), "{e}");
    }

    /// A machine that has just been provisioned. Not an error — and the answer
    /// names where it looked, so an operator can tell it from a wrong path.
    #[test]
    fn a_machine_with_no_journal_at_all_is_an_empty_read_that_names_the_directory() {
        let got = read_with(reader(BTreeMap::new())).expect("an absent journal is not a failure");
        assert!(got.lines.is_empty());
        assert!(got.files.is_empty());
        assert_eq!(got.source, LOG_DIR);
    }

    /// **The property `--boot previous` rests on.** Rotated files are older
    /// than the active one, so their lines must come first — `journal.1.jsonl`
    /// holds the boot before whatever is in `journal.jsonl`.
    #[test]
    fn rotated_files_are_read_before_the_active_one() {
        let got = read_with(reader(present(&[
            (&rotated_log(2), "oldest\n"),
            (&rotated_log(1), "middle\n"),
            (ACTIVE_LOG, "newest\n"),
        ])))
        .expect("three readable files");
        assert_eq!(got.lines, ["oldest", "middle", "newest"]);
        assert_eq!(
            got.files,
            [rotated_log(2), rotated_log(1), ACTIVE_LOG.to_string()]
        );
    }

    /// Rotation leaves gaps — `ADR-022` §8 drops the oldest and a machine that
    /// has rotated twice has three of the eight names. A gap must not stop the
    /// read at the gap.
    #[test]
    fn a_gap_in_the_rotation_does_not_truncate_the_read() {
        let got = read_with(reader(present(&[
            (&rotated_log(LOG_FILES_KEPT - 1), "very old\n"),
            (ACTIVE_LOG, "current\n"),
        ])))
        .expect("two readable files");
        assert_eq!(got.lines, ["very old", "current"]);
        assert_eq!(got.files.len(), 2);
    }

    /// **The distinction that matters.** A file that is there and will not open
    /// is a refusal carrying what the system said — never an empty journal,
    /// which would report a machine that had done nothing.
    #[test]
    fn a_file_that_exists_and_cannot_be_read_is_a_refusal_and_not_an_empty_log() {
        let mut files = present(&[(ACTIVE_LOG, "a line\n")]);
        files.insert(
            rotated_log(1),
            Err(Error::new(ErrorKind::PermissionDenied, "denied")),
        );
        let got = read_with(reader(files));
        match got {
            Err(MachineError::Refused(why)) => {
                assert!(why.contains(&rotated_log(1)), "{why}");
                assert!(why.contains("not an empty journal"), "{why}");
            }
            other => panic!("a permission error was reported as {other:?}"),
        }
    }

    /// Blank lines and the trailing newline every file ends with must not
    /// become records. The domain skips them (`parse_lines`), and this asserts
    /// the adapter does not manufacture extra ones on the way.
    #[test]
    fn a_trailing_newline_does_not_become_an_extra_line() {
        let got = read_with(reader(present(&[(ACTIVE_LOG, "one\ntwo\n")]))).expect("readable");
        assert_eq!(got.lines, ["one", "two"]);
    }

    /// Every name the reader looks at is one the writer can produce. A reader
    /// and a writer that disagreed about the file names would show an operator
    /// an empty log on a machine that had been writing all along — silently,
    /// which is why this is asserted rather than assumed.
    #[test]
    fn the_reader_looks_at_exactly_the_names_rotation_produces() {
        let looked: Vec<String> = log_files_oldest_first();
        let written: Vec<String> = (0..LOG_FILES_KEPT).map(rotated_log).collect();
        let mut a = looked.clone();
        let mut b = written;
        a.sort();
        b.sort();
        assert_eq!(a, b);
    }

    // -- the text view -------------------------------------------------------

    use rivet_core::Value as V;
    use rivet_core::journal::{Record, Severity, Value as FieldValue};

    fn record() -> Record {
        Record {
            ts: Some("2026-08-22T14:02:11.412Z".to_string()),
            boot: "boot-1".to_string(),
            mono: 1.319,
            sev: Severity::Error,
            svc: "rivet-timed".to_string(),
            corr: "c".to_string(),
            actor: "supervisor".to_string(),
            msg: "REFUSED this image cannot run the program this service names".to_string(),
            fields: vec![("errno".to_string(), FieldValue::Int(2))],
        }
    }

    fn answer(records: Vec<V>) -> V {
        V::object(vec![
            ("subject", V::str("log")),
            ("records", V::List(records)),
            ("returned", V::Integer(1)),
        ])
    }

    /// `ADR-022` §2's view, through the one function that makes it.
    #[test]
    fn a_record_becomes_the_line_a_person_reads() {
        let line = record().to_jsonl();
        let got = as_text_view(&answer(vec![V::str(line.trim_end())]));
        match got {
            V::Object(fields) => match fields.iter().find(|(k, _)| *k == "records") {
                Some((_, V::List(items))) => {
                    assert_eq!(items.first(), Some(&V::str(record().to_text())));
                    let rendered = format!("{:?}", items.first());
                    assert!(rendered.contains("error"), "{rendered}");
                    assert!(rendered.contains("rivet-timed"), "{rendered}");
                    assert!(rendered.contains("errno=2"), "{rendered}");
                    assert!(!rendered.contains(r#""sev":"#), "still JSON: {rendered}");
                }
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    /// Every other field is untouched, so the two readings agree about
    /// everything the record is not (`ADR-006` §4).
    #[test]
    fn nothing_but_the_records_field_is_rewritten() {
        let got = as_text_view(&answer(vec![]));
        match got {
            V::Object(fields) => {
                assert_eq!(fields.first(), Some(&("subject", V::str("log"))));
                assert_eq!(fields.get(2), Some(&("returned", V::Integer(1))));
            }
            other => panic!("{other:?}"),
        }
    }

    /// A `records` entry that is not a record passes through unchanged. The
    /// narrowing that makes matching on a field name safe — and the failure it
    /// prevents is a text rendering that errors on a value the JSON rendering
    /// prints fine, which `ADR-006` §4 forbids.
    #[test]
    fn a_string_that_is_not_a_record_is_left_exactly_as_it_was() {
        let got = as_text_view(&answer(vec![V::str("rivet-initd: starting as pid 1")]));
        match got {
            V::Object(fields) => match fields.iter().find(|(k, _)| *k == "records") {
                Some((_, V::List(items))) => assert_eq!(
                    items.first(),
                    Some(&V::str("rivet-initd: starting as pid 1"))
                ),
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
    }

    /// And a value with no `records` field at all is returned unchanged, so
    /// every other operation's text rendering is the same function it was.
    #[test]
    fn a_value_from_another_operation_is_untouched() {
        let other = V::object(vec![
            ("path", V::str("/etc/passwd")),
            ("writable", V::Bool(false)),
        ]);
        assert_eq!(as_text_view(&other), other);
    }

    // -- sending (`ADR-022` §11) --------------------------------------------

    /// **The sender, over a real socket, checked against the reader that will
    /// actually receive it.**
    ///
    /// `AGENTS.md` §4: *"a test that round-trips a reader against its own writer
    /// proves the two agree with each other and nothing about the format."* The
    /// listener here is a plain `UnixListener` and what it hands the bytes to is
    /// `rivet_core::journal::ingest` — the same function `cmd/rivet-sysd`
    /// calls, which is what makes this a test of the two ends agreeing rather
    /// than of this module agreeing with itself.
    ///
    /// It binds under the host's temporary directory and never at
    /// `INGEST_SOCKET_PATH` (`AGENTS.md` §9), which is why [`send_to`] takes the
    /// path.
    #[cfg(unix)]
    #[test]
    fn a_sent_record_arrives_whole_and_the_supervisors_own_reader_accepts_it() {
        use rivet_core::journal::{Peer, Record, Severity};
        use std::io::Read as _;

        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-send-{}", std::process::id()));
        let dir = dir.to_string_lossy().into_owned();
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temporary directory");
        let path = format!("{dir}/journal");

        let listener = std::os::unix::net::UnixListener::bind(&path).expect("a socket to bind");
        let record = Record {
            ts: None,
            boot: "b".to_string(),
            mono: 1.0,
            sev: Severity::Info,
            svc: SELF_NAME_FOR_TEST.to_string(),
            corr: "c".to_string(),
            actor: "operator".to_string(),
            msg: "replace /state/x: 7 bytes to 6".to_string(),
            fields: vec![(
                "path".to_string(),
                rivet_core::journal::Value::Str("/state/x".to_string()),
            )],
        };
        assert_eq!(send_to(&path, &record.to_jsonl()), Ok(()));

        let (mut accepted, _) = listener.accept().expect("the connection");
        let mut got = String::new();
        accepted.read_to_string(&mut got).expect("the record");
        assert_eq!(got, record.to_jsonl(), "the bytes on the wire are not the record");

        let composed = rivet_core::journal::ingest(
            got.trim_end(),
            &Peer {
                uid: 0,
                gid: 0,
                pid: 1,
            },
        );
        assert_eq!(composed.msg, record.msg);
        assert_eq!(composed.svc, SELF_NAME_FOR_TEST);
        assert_eq!(composed.actor, "uid:0");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The name the record carries, pinned here so this test does not have to
    /// know `crate::edit`'s constant twice.
    #[cfg(unix)]
    const SELF_NAME_FOR_TEST: &str = crate::edit::SELF_NAME;

    /// **A record larger than the socket takes is refused here rather than at
    /// the far end**, so the sender's own stderr gets it instead of the
    /// supervisor journalling a refusal about bytes nobody kept.
    #[test]
    fn a_record_bigger_than_the_socket_takes_is_refused_before_it_is_sent() {
        let huge = "x".repeat(rivet_core::journal::INGEST_MAX_BYTES + 1);
        match send_to("/nonexistent/rivet/journal", &huge) {
            Err(why) => assert!(why.contains("at most"), "{why}"),
            Ok(()) => panic!("an oversized record was sent"),
        }
    }

    /// **A socket that is not there is an error and never a success.** The
    /// ordinary state of a development host, and the state of a board whose
    /// supervisor has died — and reporting it as success would lose the record
    /// silently, which is the one thing `ADR-022` §11.5 is about.
    #[test]
    fn a_socket_that_is_not_there_is_reported_rather_than_reported_as_sent() {
        let mut absent = std::env::temp_dir();
        absent.push(format!("rivet-no-such-socket-{}", std::process::id()));
        let absent = absent.to_string_lossy().into_owned();
        assert!(!std::path::Path::new(&absent).exists());
        assert!(send_to(&absent, "{}\n").is_err());
    }
}
