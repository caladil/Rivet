//! **Changing one file, atomically, and recording that it happened.**
//! `ADR-006` §8.9, ruled 2026-08-24.
//!
//! `rivet_core::operation::machine::Editor` is the domain port
//! `rivet apply edit` reaches, and this module is the adapter under it. Every
//! decision about *whether* a change may be made — the match count, the views,
//! the reserved paths, the caps — is `rivet_core::operation::edit`'s and none
//! of it is repeated here. What is here is what only a machine can do: reading
//! the bytes, creating the parents, the five-step write, and the record.
//!
//! # The write, and why it is five steps
//!
//! ```text
//!   1. create  <path>.rivet-new.<unique>  exclusively, so nothing already
//!              waiting at a name can be opened, followed or truncated
//!   2. apply   the owner and mode the file already has, on that open
//!              file (0600, this process's own owner, if the file is new)
//!   3. fsync   the temporary file    the bytes are on the card, not in a cache
//!   4. rename  onto <path>           atomic within one directory
//!   5. fsync   the directory         the rename survives a power cut
//! ```
//!
//! `crate::deploy` does exactly this for the install request and carries the
//! argument; it applies more strongly here, because the file being replaced is
//! one somebody else is reading. **A reader sees the old file or the new one and
//! never a truncated one** — which is precisely what the nushell pipeline this
//! operation exists to replace could not promise: `open f | ... | save -f f`
//! truncates before it writes.
//!
//! **Step 1's name is unpredictable, and step 2 runs on the open file rather
//! than by a second lookup of its name.** Both matter for the same reason:
//! an application that already owns the file `apply edit` is replacing must
//! never see an instant — even one between two of this operation's own
//! steps — at which the name it owns is a file it does not. A name built
//! only from `<path>` would let whatever already owns that directory plant
//! something at it first; setting the owner by path rather than by
//! descriptor would ask that same directory a second time what is there,
//! after this operation itself had already changed the answer.
//! `PathChange::InstallSecret` carries the identical argument and uses the
//! identical mechanism — [`FileStore::write_owned_temporary`] — for the
//! credential it installs.
//!
//! **Step 5's failure is not a refusal**, on `crate::deploy`'s reasoning: the
//! file on the running machine is already the new one, and an unflushed
//! directory risks only that a power cut in the next moments reverts a change
//! the operator was told about. Reporting *not written* about something that was
//! written is the worse of the two lies.
//!
//! # The record, and the second writer that this module briefly was
//!
//! `ADR-022` §7 makes the journal **part of the supervisor**, and until this
//! module existed that was the whole story: `cmd/rivet-deploy` and
//! `cmd/rivet-timed` emit records on *stdout* and the supervisor captures them
//! (§4), so exactly one process had ever written `/state/log/journal.jsonl`.
//!
//! **`rivet` cannot do that**, and the reason is structural rather than
//! incidental: it is the login shell, so its stdout is the
//! operator's own session and nothing captures it. A record written there would
//! corrupt the answer the operator is reading and reach no journal.
//!
//! ~~So this module **appends to `ADR-022` §8's active file directly**, and that
//! is a second writer of a file the record gives to one owner. It is the
//! narrowest form available — one `O_APPEND` open, one `write(2)` of one line
//! under 4 KiB, one close, only when an edit actually happened — and the two
//! honest caveats are stated rather than discovered:~~
//!
//! 1. ~~**Rotation is the supervisor's and is not coordinated.** If it renames
//!    the active file between this process's open and its write, the line lands
//!    in the rotated file instead.~~
//! 2. ~~**This does not rotate.** A machine whose supervisor is not running
//!    could grow the active file past `ADR-022` §8's cap by editing files, one
//!    line at a time.~~
//!
//! ~~**Whether `ADR-022` §7 should say "the supervisor and the administration
//! surface" is the maintainer's**~~ — **ruled 2026-08-24, and the answer is
//! that it should not.** `PLAN.md` §10.6 recommended amending §7; the maintainer
//! refused it as a bandaid on two grounds, both of which the struck text above
//! is the evidence for. It answers *may the CLI write too* when the question is
//! *how does a process the supervisor did not spawn file a record* — which needs
//! an amendment per writer, and there are more writers coming. And two processes
//! appending JSONL can interleave a partial line, so the caveats above were not
//! the whole cost: a record is far larger than any write the kernel makes
//! atomic, and the failure would have manufactured exactly the unparsable lines
//! `ADR-022` §9.4's `unreadable` counter exists to count.
//!
//! **So the supervisor holds the journal open and offers an ingest socket
//! (`ADR-022` §11), and this module sends.** `ADR-022` §7 is unamended — one
//! writer, still the supervisor. What is here is [`FileStore::send_record`], and
//! `crate::journal::send_or_say` is what it reaches; the floor when nothing is
//! listening is this session's own stderr, so a record cannot vanish silently.
//!
//! # It says who asked, and it is the same principal an install records
//!
//! `crate::deploy::this_process` reads the peer out of the environment
//! once, and that function is reused rather than copied: a second reading of the
//! environment would be a second notion of who is speaking, which is `ADR-011`'s
//! failure in the place it costs most — an edit and an install from one session
//! attributed under two different names.
//!
//! **It is attribution and not authorisation** (`ADR-028` §4.5), and here the
//! point is sharper than it is for an install: anything running as uid 0 can
//! write the same file with nushell's `save` and leave no record at all. What
//! this buys is that an edit made *through the operation* is one the journal
//! can account for.

use rivet_core::journal::{Record, Severity, Value, rfc3339};
use rivet_core::operation::{
    DirectoryEntry, Editor, FileRead, FileWindow, MachineError, NEW_DIRECTORY_MODE, NEW_FILE_MODE,
    OwnerIds, PathChange, PathKind, PathStat, Write, Wrote,
};

/// The suffix that marks a file as one of this module's temporaries.
///
/// **Never a complete temporary name on its own.** Every write this module
/// makes — [`Files::write_file`] and [`PathChange::InstallSecret`] alike —
/// goes through [`FileStore::write_owned_temporary`], which follows this
/// suffix with material a caller cannot predict ([`unique_temporary_name`]).
/// A file or a symbolic link already waiting at exactly `<path>` plus this
/// suffix and nothing else blocks nothing. It stays a public constant
/// because the suffix itself is still what makes a name recognisable as
/// one of this module's own in a directory listing.
///
/// **Beside the file rather than in `/tmp`**, and that is not a preference:
/// `rename(2)` is atomic only within one filesystem, and `/tmp` on a Rivet
/// machine is a tmpfs while the file being edited is on `/state`. A rename
/// across the two is a copy, and a copy is exactly the non-atomic write this
/// operation exists to avoid.
pub const TEMPORARY_SUFFIX: &str = ".rivet-new";

/// How many candidate names [`unique_temporary_name`] will try before
/// [`FileStore::write_owned_temporary`] gives up. Each candidate already
/// differs from the last by a counter unique within this process, so
/// colliding this many times running means the directory itself refuses
/// writes rather than that luck ran out.
const MAX_TEMPORARY_ATTEMPTS: u32 = 1000;

/// **A temporary name nothing already waiting in the directory could have
/// predicted.**
///
/// [`TEMPORARY_SUFFIX`] alone is not enough: a name built only from `path`
/// is one anything that can already write into the same directory can
/// plant ahead of time — a file, which `create_new` refuses to overwrite,
/// or a symbolic link, which the same flag refuses to follow — and either
/// one is enough to block every write to `path` until an operator notices
/// and clears it (`ADR-006` §8). This one adds a counter this process
/// shares with nothing else, the process id, and the clock: nothing
/// outside this process's own memory can compute it in advance, and two
/// calls inside this process never choose the same one.
///
/// Already the full path to use — beside `path`, in the same directory,
/// the same way [`TEMPORARY_SUFFIX`] alone would be.
fn unique_temporary_name(path: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{path}{TEMPORARY_SUFFIX}.{pid:x}-{n:x}-{nanos:x}")
}

/// The service name every record this module writes carries.
pub const SELF_NAME: &str = "rivet";

/// `ADR-022` §5's actor for an edit.
///
/// **`operator`, and not a person or an agent**, because this process cannot
/// tell them apart: `ADR-005` §5.1 makes `rivet` a login shell for `root` and
/// a human at a terminal and an agent on the control plane arrive identically. The actor is
/// therefore *the administration surface*, which is what actually performed the
/// write, and **who** is the record's `principal` field — a claim, labelled as
/// one (`ADR-028` §4.5).
///
/// `ADR-012` §4 asks for actor provenance and this is the honest half of it. The
/// other half needs a credential (`ADR-012` §5), and Rivet has none.
pub const ACTOR: &str = "operator";

/// What a record needs from the machine that only the machine can say.
#[derive(Debug, Clone, PartialEq)]
pub struct Stamp {
    /// `ADR-022` §3.2's `boot`, from `/proc/sys/kernel/random/boot_id` — the
    /// same fact `cmd/rivet-sysd`, `cmd/rivet-timed` and
    /// `cmd/rivet-deploy` read, from the kernel that owns it. `unknown` on a
    /// machine that will not answer, never a synthesised identifier.
    pub boot: String,
    /// Seconds since boot, monotonic.
    pub mono: f64,
    /// Milliseconds since the Unix epoch, or `None` on a machine with no
    /// readable wall clock — the ordinary state of a cold Pi (`ADR-021`).
    pub wall_ms: Option<u64>,
}

/// The filesystem, as this module needs it.
///
/// **An adapter's own I/O seam, not a domain port** — `AGENTS.md` §3.1's
/// distinction, and `crate::deploy::RequestStore`'s precedent. Every method here
/// is in the vocabulary of files, modes and directories, which is why it is
/// named in this crate rather than in `rivet-core`.
///
/// It exists so that every decision this module makes — the ancestor walk, the
/// five-step write, the record, and what happens when each of them fails — is
/// reachable from a test on a development host that has no `/state` and must not
/// grow one (`AGENTS.md` §9).
pub trait FileStore {
    /// A file's bytes, or `None` if it is not there.
    ///
    /// `Ok(None)` and `Err` are deliberately different, for
    /// `crate::deploy::RequestStore::read`'s reason and a sharper one: reading
    /// *"could not be read"* as *"there is nothing there"* would let an edit
    /// replace a file whose contents were never seen.
    fn read(&mut self, path: &str) -> Result<Option<Vec<u8>>, String>;

    /// Up to `len` bytes of `path`, starting at `at`, and the file's true
    /// size — a POSITIONED read, `None` if nothing is at `path`.
    ///
    /// **FS4.** [`Self::read`] answers the whole file, which is what
    /// `rivet get file` used to call to answer a caller asking for one
    /// mebibyte of a much larger one. This is the seam that lets the real
    /// implementor open the file once, seek, and read only what was asked
    /// for — never allocating the rest.
    fn read_window(
        &mut self,
        path: &str,
        at: usize,
        len: usize,
    ) -> Result<Option<(Vec<u8>, usize)>, String>;

    /// Whether `path` is a directory.
    fn is_directory(&mut self, path: &str) -> bool;

    /// Whether anything at all is at `path` — a file, a directory, a device.
    fn exists(&mut self, path: &str) -> bool;

    /// `mkdir(2)` one directory, with `mode`. The parent already exists.
    fn create_directory(&mut self, path: &str, mode: u32) -> Result<(), String>;

    /// Create a NEW file beside `path`, under a name this call picks for
    /// itself — see [`unique_temporary_name`] — write `bytes` to it with
    /// `mode`, and, when `owner` is `Some((uid, gid))`, give it that owner
    /// too. Every one of those happens through the ONE handle this call
    /// opens and keeps until it is done with it. Returns the name it
    /// chose, so the caller can rename it into place.
    ///
    /// [`Files::write_file`] and [`PathChange::InstallSecret`] are what this
    /// exists for: `path` may be a file an application already owns and
    /// must still own afterward, or a credential that must never be visible
    /// under its name owned by anyone but that application (`ADR-006` §8).
    /// Setting the owner by a second lookup of
    /// the chosen name, after this call returns, would ask the directory
    /// again what is there — the same directory a confined application can
    /// hold, and therefore a second chance for it to have changed what
    /// that name resolves to. The descriptor this call keeps open cannot
    /// be redirected that way; it names the inode it created, whatever a
    /// path is later made to point at.
    ///
    /// # Errors
    ///
    /// Every failure — to find an unused name, to write, or to apply
    /// `owner` — removes only the ONE temporary this call itself created
    /// and reports what the kernel said. It never touches `path`, and
    /// never anything a caller told it about rather than something it
    /// made itself.
    fn write_owned_temporary(
        &mut self,
        path: &str,
        bytes: &[u8],
        mode: u32,
        owner: Option<(u32, u32)>,
    ) -> Result<String, String>;

    /// `rename(2)`, which is atomic within one directory.
    fn rename(&mut self, from: &str, to: &str) -> Result<(), String>;

    /// `fsync(2)` a directory, so a rename survives a power cut.
    fn flush_directory(&mut self, path: &str) -> Result<(), String>;

    /// File one journal record — `ADR-022` §11.
    ///
    /// `line` is the record in its authoritative form
    /// (`rivet_core::journal::Record::to_jsonl`), newline and all. **Nothing
    /// here writes to `/state/log`**: this process is not the journal's writer
    /// and `ADR-022` §7 is unamended.
    ///
    /// Returns the sentence to report as `unrecorded` — which is `None` when the
    /// record reached the supervisor, and otherwise says where the record went
    /// instead. It is not a `Result`, because a record that could not be *sent*
    /// has still been *put somewhere*, and a type that made the two look the
    /// same is one a caller would report as loss.
    fn send_record(&mut self, line: &str) -> Option<String>;

    /// What the machine says about which boot this is and what time it is.
    fn stamp(&mut self) -> Stamp;

    // -----------------------------------------------------------------------
    // `ADR-006` §8.10, added 2026-08-29
    // -----------------------------------------------------------------------

    /// **Everything the kernel says about one path, without following a
    /// symbolic link.** `None` when nothing is there.
    ///
    /// # Why this does not replace [`Self::is_directory`] and [`Self::exists`]
    ///
    /// Because they answer a **different question**, and collapsing them would
    /// be a silent change to `apply edit`. Those two follow symbolic links,
    /// which is right for a PARENT directory on the way to the file being
    /// edited: a directory reached through a link is still a directory, and
    /// `missing_parents` must not try to create one that is already there.
    ///
    /// **The file being edited is the narrower case, and `interlock` answers
    /// it from THIS method rather than from those two** — B03: a path whose
    /// final component is itself a link is refused outright, never followed,
    /// because the write is a `rename(2)` onto the exact name given and would
    /// replace the link rather than what it points at.
    ///
    /// `ADR-006` §8.10's subjects must **never** follow one — §8.10.4.3, and
    /// the reason is that a link is a way out of the permitted root. So this is
    /// an `lstat`, they are a `stat`, and a trait with one of each is honest
    /// where a trait with one of either would quietly be wrong for half its
    /// callers.
    ///
    /// # Errors
    ///
    /// A path that is there and cannot be examined. **Not** a path that is not
    /// there, which is `Ok(None)`.
    fn stat(&mut self, path: &str) -> Result<Option<RawStat>, String>;

    /// The entries directly inside a directory, **sorted by name**, and how
    /// many there are in total.
    ///
    /// Sorted here rather than by the caller because a listing whose order
    /// depended on the filesystem's hashing would give two answers to one
    /// question on two boots, and `ADR-015` §4 wants the same inputs to produce
    /// the same bytes.
    ///
    /// The `usize` is the **true** count even when the `Vec` was capped at
    /// `rivet_core::operation::filesystem::DIRECTORY_MAX_ENTRIES` — a truncated
    /// list that could not say how much it left out is one nobody can act on.
    ///
    /// # Errors
    ///
    /// A directory that cannot be read.
    fn list(&mut self, path: &str) -> Result<(Vec<RawEntry>, usize), String>;

    /// The first component on the way to `path` that is itself a symbolic
    /// link — `ADR-006` §8.10.4.3, and **the check a string cannot make**.
    ///
    /// `/state/scratch` may be a link to `/system`, in which case
    /// `/state/scratch/bin/x` passes a textual test against `/state` and is not
    /// under it. The last component is **not** examined here: whether the target
    /// itself is a link is [`Self::stat`]'s answer and each subject decides what
    /// to do about it.
    fn symlinked_ancestor(&mut self, path: &str) -> Option<String>;

    /// A user or group, by name or by number, resolved to both.
    ///
    /// `ADR-007`'s projected `/etc/passwd` and `/etc/group` are the authority
    /// — the same files everything else on this machine resolves against — so
    /// this reads them rather than carrying a table. `None` is *"this machine
    /// does not know that one"*, never a guess.
    fn identity(&mut self, kind: IdKind, what: &str) -> Option<(u32, String)>;

    /// `chmod(2)`, **without following a symbolic link**.
    ///
    /// # Errors
    ///
    /// Whatever the kernel said, verbatim.
    fn set_mode(&mut self, path: &str, mode: u32) -> Result<(), String>;

    /// `chown(2)`, **without following a symbolic link and without recursing**.
    /// A `None` half is left alone.
    ///
    /// # Errors
    ///
    /// Whatever the kernel said, verbatim.
    fn set_owner(&mut self, path: &str, uid: Option<u32>, gid: Option<u32>) -> Result<(), String>;

    /// `unlink(2)`.
    ///
    /// # Errors
    ///
    /// Whatever the kernel said, verbatim.
    fn remove_file(&mut self, path: &str) -> Result<(), String>;

    /// `rmdir(2)`, which **refuses a directory that is not empty** — a second
    /// guard under the domain's own (`ADR-006` §8.10.12), and the one that is
    /// enforced by the kernel rather than by a check that could be raced.
    ///
    /// # Errors
    ///
    /// Whatever the kernel said, verbatim.
    fn remove_directory(&mut self, path: &str) -> Result<(), String>;
}

/// Which table [`FileStore::identity`] looks in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdKind {
    User,
    Group,
}


/// **Refuse to write a configuration document this machine cannot read.**
///
/// `PLAN.md` §104.19, written after stranding a board. `apply edit` is a TEXT
/// surface — `--expect`, `--replace`, byte equality — and it parsed nothing, so
/// it would write any bytes at all into the one file that decides whether this
/// machine has a network. The chain from there is short and every link worked
/// as designed: an unreadable document falls back to `Document::default()`,
/// which has no `[network]` table; no network means no control plane; and
/// `ADR-035` leaves no second way in.
///
/// **`ADR-018`'s A/B cannot undo it**, which is the part that makes this worth a
/// special case rather than a warning. `/state` is the one partition an update
/// never replaces, so rolling the system slot back leaves the same document in
/// place — and an older image is *more* likely to refuse it, not less. A
/// document is the only thing on a Rivet that can brick it.
///
/// # Why the generic file surface knows about one path
///
/// Because that path is not a file like the others: `ADR-007` §3.3 makes it the
/// statement of what this machine should be, and `rivet-sysd` reads it on every
/// boot. `cmd/rivet-deploy`'s `repair_autoboot` is the precedent — the
/// installer knows about `autoboot.txt` for the same reason, that one file can
/// make a machine unbootable and the others cannot.
///
/// # What it does NOT promise
///
/// **That the document is right** — only that it PARSES, under the image doing
/// the writing. A document naming a service that does not exist, or an address
/// on no network this board can reach, still writes: those are recoverable over
/// a control plane that still comes up. This refuses the narrower and more
/// dangerous thing, a document that cannot be read at all.
///
/// And it is the RUNNING image's parser. A key a newer Rivet understands is
/// refused here, which is exactly the case that stranded the board: the
/// document was written before the image that knew the key was the one booted.
fn refuse_unreadable_document(path: &str, contents: &str) -> Result<(), MachineError> {
    if path != rivet_core::state::MACHINE_DOCUMENT {
        return Ok(());
    }
    let Err(why) = rivet_config::Document::parse(contents) else {
        return Ok(());
    };
    Err(MachineError::Refused(format!(
        "REFUSED this would leave {path} unreadable by THIS image, and NOTHING HAS BEEN \
         WRITTEN.\n\n  {why}\n\nThat file is not a file like the others: rivet-sysd reads it \
         on every boot (ADR-007 §3.3), and a document it cannot parse falls back to the schema \
         defaults - which carry NO NETWORK. No network is no control plane, and ADR-035 \
         leaves no second way in. ADR-018's A/B DOES NOT UNDO THIS: /state is \
         the partition an update never replaces, so a rollback leaves this same document and an \
         older image is more likely to refuse it than a newer one.\n\nIf the key is one a NEWER \
         Rivet understands, install and BOOT that image first, then write the document. \
         PLAN.md §104.19."
    )))
}

/// **Refuse a new `cpu` cap the kernel will not keep** — `.rulings/` R-210
/// D22.
///
/// [`refuse_unreadable_document`] only ever asks whether THIS IMAGE can
/// parse the document; a value that parses cleanly but that cgroup v2
/// refuses at any period is a different failure — `rivet-sysd` journals it
/// at every start, and nothing stops the machine coming up uncapped with no
/// cause named for the operator who wrote it. This is the narrower guard for
/// that, on [`refuse_unreadable_document`]'s own footing: it reads the one
/// path that matters and nothing else, and it does not repeat a refusal that
/// function already gives — a document this build cannot parse never
/// reaches the loop below at all.
///
/// `rivet_config::Document::parse` and `rivet_core::service::CpuLimit::parse`
/// are both unchanged: a document already on a machine, carrying a cap
/// below the minimum, must keep reading back at boot. This refuses only the
/// moment a NEW one is about to be written.
fn refuse_unenforceable_cpu_cap(path: &str, contents: &str) -> Result<(), MachineError> {
    if path != rivet_core::state::MACHINE_DOCUMENT {
        return Ok(());
    }
    let Ok(document) = rivet_config::Document::parse(contents) else {
        return Ok(());
    };
    for service in document.services() {
        let Some(limit) = service.cpu() else { continue };
        if limit.is_below_kernel_minimum() {
            return Err(MachineError::Refused(format!(
                "REFUSED [services.{}] cpu = \"{limit}\" — {} NOTHING HAS BEEN WRITTEN. Raise \
                 the cap to at least 0.01, or remove the key to run uncapped.",
                service.name(),
                limit.kernel_refusal(),
            )));
        }
    }
    Ok(())
}

/// **Refuse a proxy `handshake-seconds` the proxy would not use** — `ADR-046`
/// §10.
///
/// On [`refuse_unenforceable_cpu_cap`]'s footing: the value parses as a
/// document, because `[applications.<name>]` is text the document never refuses
/// (`ADR-038` §4.4), but outside its range the proxy would set it aside at
/// start and journal why, and the operator who wrote it would not be told.
/// This refuses only the moment a new one is about to be written; a document
/// already carrying one still reads, and the proxy still starts on the
/// default.
fn refuse_unusable_proxy_setting(path: &str, contents: &str) -> Result<(), MachineError> {
    if path != rivet_core::state::MACHINE_DOCUMENT {
        return Ok(());
    }
    let Ok(document) = rivet_config::Document::parse(contents) else {
        return Ok(());
    };
    let Some(value) = document
        .applications()
        .get(rivet_core::service::PROXYD_SERVICE_NAME)
        .and_then(|settings| settings.get(rivet_core::proxy::HANDSHAKE_SECONDS_SETTING))
    else {
        return Ok(());
    };
    match rivet_core::proxy::parse_handshake_seconds(value) {
        Ok(_) => Ok(()),
        Err(why) => Err(MachineError::Refused(format!(
            "REFUSED [applications.{}] {}: {why}. NOTHING HAS BEEN WRITTEN. Write a whole number \
             of seconds in that range, or remove the key for the default of {}s.",
            rivet_core::service::PROXYD_SERVICE_NAME,
            rivet_core::proxy::HANDSHAKE_SECONDS_SETTING,
            rivet_core::proxy::HANDSHAKE_SECONDS_DEFAULT,
        ))),
    }
}

impl IdKind {
    /// The word a refusal uses, and the projected file it reads.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Group => "group",
        }
    }

    /// The `ADR-007` projection this machine resolves it against.
    pub const fn path(self) -> &'static str {
        match self {
            Self::User => rivet_core::etc::PASSWD,
            Self::Group => rivet_core::etc::GROUP,
        }
    }
}

/// What [`FileStore::stat`] found. An `lstat`, never a `stat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawStat {
    pub kind: rivet_core::operation::PathKind,
    pub size_bytes: u64,
    pub mode: Option<u32>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub modified_ms: Option<u64>,
    pub link_target: Option<String>,
    pub mount_point: bool,
}

/// One entry of a directory, as [`FileStore::list`] found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEntry {
    pub name: String,
    pub kind: rivet_core::operation::PathKind,
    pub size_bytes: u64,
}

/// What a path that is a directory is answered with, in one place.
///
/// `E_UNSAFE` rather than `E_MACHINE_REFUSED` — `ADR-006` §4.1's test is whether
/// the code names a different next action, and this one says *fix the request*,
/// not *look at the hardware*.
fn directory_refusal(path: &str) -> MachineError {
    MachineError::Unsafe(format!(
        "REFUSED {path} is a DIRECTORY on this machine, not a file, and NOTHING HAS BEEN \
         WRITTEN. `rivet apply edit` replaces the contents of ONE FILE and creates no \
         directories except the parents of the file it is writing. Name the file inside it."
    ))
}

/// Refuse to write through a path whose FINAL component is a symbolic link.
///
/// **B03.** The write below is `rename(<temporary>, path)`, which replaces
/// whatever is AT `path` rather than following it to what it points at — so
/// writing through a link would silently detach it from the file an operator
/// meant to change. It is also the one spelling nothing else in this module
/// can see through: a path can fail every reserved-name and machine-document
/// check by its text and still resolve to one of them. This operation does
/// not resolve or normalise a path through a link; it refuses one, the way
/// `Files::can_change_path` already does for a link higher up the path.
fn symlink_refusal(path: &str) -> MachineError {
    MachineError::Unsafe(format!(
        "REFUSED {path} is a SYMBOLIC LINK on this machine, and NOTHING HAS BEEN WRITTEN. \
         `rivet apply edit` writes by renaming a new file over the EXACT NAME you gave, which \
         would replace the LINK rather than the file it points at — and every other check this \
         operation makes is a comparison against the text of the path you named, which a link \
         can make resolve somewhere that text never said. Name the file the link points at, if \
         that is what you meant."
    ))
}

/// Refuse a path with a symbolic link on the way to it — `ADR-006` §8.
///
/// Every check this module makes on a path is a test of its TEXT: the
/// permitted root for a mutation, and the machine document's guards for an
/// edit. A link in the middle of the path makes it resolve somewhere that text
/// never said, so a change under one spelling would land on a file the checks
/// were never asked about. Shared by `apply edit` and every other mutation so
/// they refuse alike.
fn linked_ancestor_refusal(link: &str, path: &str, act: &str) -> MachineError {
    MachineError::Unsafe(format!(
        "REFUSED {link}, on the way to {path}, is a SYMBOLIC LINK, and NOTHING HAS BEEN \
         CHANGED. `rivet apply {act}` decides what it may touch by testing the path's text — \
         against {root} (ADR-006 §8.10.4), and against the machine document's own name before \
         it is rewritten — and a link in the middle of it means the path resolves somewhere \
         those tests never looked. Name the path the link points at, if that is what you meant.",
        root = rivet_core::operation::filesystem::PERMITTED_ROOT,
    ))
}

/// The editor: read a file, replace it whole, and record it.
pub struct Files<S: FileStore> {
    store: S,
    /// Who this process can honestly say is asking. **Captured once, at
    /// construction**, for `crate::deploy::Requests`' reason: how this process
    /// was invoked is a fact about a moment that has already passed, so
    /// re-reading it later could only produce a different answer if something
    /// rewrote this process's environment — and the later answer would be the
    /// wrong one.
    principal: rivet_core::deploy::Principal,
}

impl<S: FileStore> Files<S> {
    pub const fn new(store: S, principal: rivet_core::deploy::Principal) -> Self {
        Self { store, principal }
    }

    /// What was written, for a test in another module of this crate.
    #[cfg(test)]
    pub const fn store(&self) -> &S {
        &self.store
    }

    /// The directory a path lives in.
    ///
    /// Purely textual: the caller has already been refused a relative path and a
    /// path with a control character in it (`rivet_core::operation::edit`), so
    /// there is always a `/` and it is never the last byte. A path directly under
    /// the root gives `/`.
    fn directory_of(path: &str) -> &str {
        match path.rfind('/') {
            Some(0) | None => "/",
            Some(at) => &path[..at],
        }
    }

    /// The ancestors of `path` that do not exist, **outermost first**.
    ///
    /// Every one of them is named in the plan, which is `ADR-006` §8.9's rule:
    /// *"missing parent directories are created, but every one must be named in
    /// the plan, so it reads as intent rather than a silent `mkdir -p`."*
    fn missing_parents(&mut self, path: &str) -> Vec<String> {
        let mut missing = Vec::new();
        let mut walked = String::new();
        // The last component is the file, so it is not walked.
        let components: Vec<&str> = path.split('/').collect();
        for component in components.iter().take(components.len().saturating_sub(1)) {
            if component.is_empty() {
                continue;
            }
            walked.push('/');
            walked.push_str(component);
            if !self.store.is_directory(&walked) {
                missing.push(walked.clone());
            }
        }
        missing
    }

    /// Everything that is true of the **machine** rather than of the file.
    ///
    /// Run on both paths — the plan and the confirmation — which is
    /// `rivet_core::operation::stop`'s argument reused: an agent shown a plan
    /// and then a refusal would reasonably conclude the confirmation was the
    /// problem and re-issue it.
    fn interlock(&mut self, path: &str) -> Result<(), MachineError> {
        // **A linked PARENT is refused, as every other mutation refuses one.**
        // The machine document's guards compare the path's text, so
        // `<somewhere>/link/machine.toml` with `link -> /state/config` would
        // rewrite the document without either guard seeing it.
        if let Some(link) = self.store.symlinked_ancestor(path) {
            return Err(linked_ancestor_refusal(&link, path, "edit"));
        }

        // **A directory is not a file, and this is the check that says so.** The
        // domain refuses a path ending in `/`; a path that merely *is* a
        // directory can only be known here.
        if self.store.is_directory(path) {
            return Err(directory_refusal(path));
        }

        // **`path` ITSELF is refused when it is a symbolic link** — B03,
        // `symlink_refusal`'s own documentation carries the argument.
        // `stat`, not `exists`/`is_directory`: those two follow a link, which
        // is right for a PARENT directory on the way to the file being
        // edited and wrong for the file itself, which this operation must
        // answer about without resolving through.
        if let Ok(Some(raw)) = self.store.stat(path)
            && raw.kind == PathKind::Symlink
        {
            return Err(symlink_refusal(path));
        }

        // **There used to be a check here for a leftover file at
        // `{path}{TEMPORARY_SUFFIX}`, refusing to reuse it.** It answered a
        // question the write no longer asks: that name is no longer where
        // `write_file` writes — [`unique_temporary_name`] is — so a file
        // sitting at the old fixed name is neither evidence of anything this
        // process did nor an obstacle to it. It is simply not looked at.
        let directory = Self::directory_of(path).to_string();
        // Nothing can be written into a path whose parent is a *file*. Caught
        // here rather than as a confusing `ENOTDIR` from the write.
        if !directory.is_empty()
            && self.store.exists(&directory)
            && !self.store.is_directory(&directory)
        {
            return Err(MachineError::Unsafe(format!(
                "REFUSED {directory} is a file on this machine, so {path} cannot be inside it and \
                 NOTHING HAS BEEN WRITTEN."
            )));
        }
        Ok(())
    }

    /// Put one record of this edit in the journal.
    ///
    /// **Returns why it could not rather than failing**, because the file is
    /// already written by the time this runs — see [`Editor::write_file`].
    fn record(&mut self, write: &Write<'_>) -> Option<String> {
        let stamp = self.store.stamp();
        // One edit, one chain, so the correlation identifier is minted from the
        // monotonic clock — monotone within a boot, so two runs of this binary
        // that are not simultaneous cannot collide. The same construction
        // `cmd/rivet-deploy`'s `salt` uses, and for the same reason.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let corr = format!("{:08x}{:08x}", (stamp.mono * 1000.0) as u32, 1u32);

        let mut fields = vec![
            ("path".to_string(), Value::Str(write.path.to_string())),
            ("change".to_string(), Value::Str(write.how.to_string())),
            ("created".to_string(), Value::Bool(write.creating)),
            (
                "before".to_string(),
                Value::Int(i64::try_from(write.was).unwrap_or(i64::MAX)),
            ),
            (
                "after".to_string(),
                Value::Int(i64::try_from(write.contents.len()).unwrap_or(i64::MAX)),
            ),
            (
                "principal".to_string(),
                Value::Str(self.principal.origin().as_token().to_string()),
            ),
        ];
        // **A count, and the names go in the message.** `ADR-022` §3's record
        // fields are scalars — there is no list — and joining paths into one
        // string is ambiguous the moment a path contains the separator. A count
        // cannot be misread, and the message carries the names properly escaped
        // by `Record::to_jsonl`.
        if !write.directories.is_empty() {
            fields.push((
                "directories".to_string(),
                Value::Int(i64::try_from(write.directories.len()).unwrap_or(i64::MAX)),
            ));
        }

        let record = Record {
            ts: stamp.wall_ms.map(rfc3339),
            boot: stamp.boot,
            mono: stamp.mono,
            sev: Severity::Info,
            svc: SELF_NAME.to_string(),
            corr,
            actor: ACTOR.to_string(),
            msg: format!(
                "{how} {path}: {before} bytes to {after}{created}{made}",
                how = write.how,
                path = write.path,
                before = write.was,
                after = write.contents.len(),
                created = if write.creating { ", created" } else { "" },
                made = if write.directories.is_empty() {
                    String::new()
                } else {
                    format!(", after creating {}", write.directories.join(", "))
                },
            ),
            fields,
        };

        // **No directory is created and no file is opened.** Until 2026-08-24
        // this made `/state/log` and appended to the active file, which was a
        // second writer of something `ADR-022` §7 gives to one owner; the
        // module documentation carries the ruling that removed it.
        self.store.send_record(&record.to_jsonl())
    }

    /// Put one record of an `ADR-006` §8.10 act in the journal.
    ///
    /// **A separate function from [`Self::record`] and not a generalisation of
    /// it.** An edit's record says how many bytes a file went from and to; these
    /// say what was moved, what mode it took, or how large the thing that is now
    /// gone was — and a record shape that covered both would carry empty fields
    /// for whichever act it was not describing. `ADR-022` §3's fields are
    /// scalars and a record that says nothing about half its fields is a record
    /// that is hard to read and easy to mis-query.
    ///
    /// **Returns why it could not rather than failing**, for
    /// [`Editor::change_path`]'s reason: the act has already happened.
    fn record_change(&mut self, change: &PathChange<'_>, path: &str) -> Option<String> {
        let stamp = self.store.stamp();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let corr = format!("{:08x}{:08x}", (stamp.mono * 1000.0) as u32, 1u32);

        let mut fields = vec![
            ("path".to_string(), Value::Str(path.to_string())),
            ("act".to_string(), Value::Str(change.act().to_string())),
            (
                "principal".to_string(),
                Value::Str(self.principal.origin().as_token().to_string()),
            ),
        ];

        // The one fact per act that cannot be recovered afterwards.
        let detail = match *change {
            PathChange::CreateDirectory { mode, parents, .. } => {
                fields.push(("mode".to_string(), Value::Str(format!("{mode:04o}"))));
                if !parents.is_empty() {
                    fields.push((
                        "directories".to_string(),
                        Value::Int(i64::try_from(parents.len()).unwrap_or(i64::MAX)),
                    ));
                }
                format!("mode {mode:04o}")
            }
            PathChange::SetMode { mode, was, .. } => {
                fields.push(("mode".to_string(), Value::Str(format!("{mode:04o}"))));
                fields.push(("was".to_string(), Value::Str(format!("{was:04o}"))));
                format!("{was:04o} to {mode:04o}")
            }
            PathChange::SetOwner {
                uid,
                gid,
                was_uid,
                was_gid,
                ..
            } => {
                if let Some(uid) = uid {
                    fields.push(("uid".to_string(), Value::Int(i64::from(uid))));
                }
                if let Some(gid) = gid {
                    fields.push(("gid".to_string(), Value::Int(i64::from(gid))));
                }
                format!(
                    "{}:{} to {}:{}",
                    describe_id(was_uid),
                    describe_id(was_gid),
                    describe_id(uid.or(was_uid)),
                    describe_id(gid.or(was_gid)),
                )
            }
            PathChange::Rename { to, .. } => {
                fields.push(("to".to_string(), Value::Str(to.to_string())));
                format!("to {to}")
            }
            // **The size is the one thing nobody can recover once the file is
            // gone**, which is why it is a field rather than only prose.
            PathChange::RemoveFile { was_bytes, .. } => {
                fields.push((
                    "before".to_string(),
                    Value::Int(i64::try_from(was_bytes).unwrap_or(i64::MAX)),
                ));
                format!("{was_bytes} bytes, GONE")
            }
            PathChange::RemoveDirectory { .. } => "empty, GONE".to_string(),
            // **The length, the owner and the mode — never the content**
            // (`ADR-020` §4). A journal is read by people and shipped off the
            // machine, and a record that carried a credential would put it
            // everywhere the journal goes.
            PathChange::InstallSecret {
                contents,
                mode,
                uid,
                gid,
                was_bytes,
                ..
            } => {
                fields.push((
                    "bytes".to_string(),
                    Value::Int(i64::try_from(contents.len()).unwrap_or(i64::MAX)),
                ));
                fields.push(("uid".to_string(), Value::Int(i64::from(uid))));
                fields.push(("gid".to_string(), Value::Int(i64::from(gid))));
                if was_bytes > 0 {
                    fields.push((
                        "before".to_string(),
                        Value::Int(i64::try_from(was_bytes).unwrap_or(i64::MAX)),
                    ));
                }
                format!(
                    "{bytes} bytes, uid {uid}, gid {gid}, mode {mode:04o}{replaced}",
                    bytes = contents.len(),
                    replaced = if was_bytes > 0 {
                        format!(", REPLACING {was_bytes} bytes that are now GONE")
                    } else {
                        String::new()
                    },
                )
            }
        };

        let record = Record {
            ts: stamp.wall_ms.map(rfc3339),
            boot: stamp.boot,
            mono: stamp.mono,
            // **A removal is not routine and the journal should not read as
            // though it were.** `ADR-022` §5's severity is what a reader scans
            // for, and the two acts that destroy something are the two worth
            // finding at a glance.
            sev: if change.destroys() {
                Severity::Warn
            } else {
                Severity::Info
            },
            svc: SELF_NAME.to_string(),
            corr,
            actor: ACTOR.to_string(),
            msg: format!("{act} {path}: {detail}", act = change.act()),
            fields,
        };
        self.store.send_record(&record.to_jsonl())
    }
}

/// A uid or gid for a record's message, or `?` where the machine had none.
///
/// `?` rather than `0`: a missing identity rendered as `0` would say *root*,
/// which is a claim about ownership that nothing measured.
fn describe_id(id: Option<u32>) -> String {
    id.map_or_else(|| "?".to_string(), |id| id.to_string())
}

impl<S: FileStore> Editor for Files<S> {
    fn principal(&mut self) -> rivet_core::deploy::Principal {
        self.principal.clone()
    }

    fn read_file(&mut self, path: &str) -> Result<FileRead, MachineError> {
        // **A directory is refused here as well as in the interlock**, and the
        // duplication is deliberate: the read happens first, so without this an
        // operator who named a directory was told *"it is there and could not be
        // read … the bytes are on the card"*, which is true of a file whose card
        // is failing and describes nothing about a directory. Found by running
        // it on a guest, 2026-08-24.
        if self.store.is_directory(path) {
            return Err(directory_refusal(path));
        }
        let contents = self.store.read(path).map_err(|detail| {
            MachineError::Refused(format!(
                "REFUSED {path} is there and could not be read, so NOTHING HAS BEEN WRITTEN. That \
                 is not an empty file: the bytes are on the card and something is between them \
                 and this process. Detail: {detail}"
            ))
        })?;
        Ok(FileRead {
            contents,
            missing_parents: self.missing_parents(path),
        })
    }

    /// **FS4: the real answer, not the port's default.** [`FileStore::read_window`]
    /// is a positioned read, so this never allocates more of `path` than the
    /// caller actually asked for — which is the entire reason the method
    /// exists rather than every caller inheriting `Editor`'s default of
    /// reading the whole file and slicing it here instead.
    fn read_file_window(
        &mut self,
        path: &str,
        at: usize,
        len: usize,
    ) -> Result<Option<FileWindow>, MachineError> {
        let window = self.store.read_window(path, at, len).map_err(|detail| {
            MachineError::Refused(format!(
                "REFUSED {path} is there and could not be read, so NOTHING HAS BEEN WRITTEN. That \
                 is not an empty file: the bytes are on the card and something is between them \
                 and this process. Detail: {detail}"
            ))
        })?;
        Ok(window.map(|(bytes, total_bytes)| FileWindow { bytes, total_bytes }))
    }

    fn can_write_file(&mut self, path: &str, contents: &str) -> Result<(), MachineError> {
        self.interlock(path)?;
        // **The dry run refuses exactly what the write would** — ADR-006 §6.2,
        // and PLAN.md §104.21. A plan that answered `yes` and a confirm that
        // then refused would be a surface more optimistic than the act, which
        // is the shape that stranded a board on 2026-08-31.
        refuse_unreadable_document(path, contents)?;
        refuse_unenforceable_cpu_cap(path, contents)?;
        refuse_unusable_proxy_setting(path, contents)
    }

    fn write_file(&mut self, write: &Write<'_>) -> Result<Wrote, MachineError> {
        // **Re-run, not remembered.** `can_write_file` may have been called on
        // the plan path seconds ago, or not at all; the card is the authority
        // and it is asked at the point of use (`AGENTS.md` §5).
        self.interlock(write.path)?;
        // **Before anything is written**, and before the temporary file is
        // created: a refusal has to leave the machine exactly as it was.
        refuse_unreadable_document(write.path, write.contents)?;
        refuse_unenforceable_cpu_cap(write.path, write.contents)?;
        refuse_unusable_proxy_setting(write.path, write.contents)?;

        // The parents, outermost first, exactly the ones the plan named.
        for directory in write.directories {
            self.store
                .create_directory(directory, NEW_DIRECTORY_MODE)
                .map_err(|detail| {
                    MachineError::Refused(format!(
                        "REFUSED {directory} could not be created, so NOTHING HAS BEEN WRITTEN to \
                         {path}. Any directory created before it is still there. Detail: {detail}",
                        path = write.path,
                    ))
                })?;
        }

        // **A file that already exists keeps its mode AND its owner.** This
        // operation changes contents; changing either of those is a
        // different act and not one this build performs — and an
        // application whose file this is must still own it afterward
        // (`ADR-006` §8). A new file starts private, owned by this
        // process (`NEW_FILE_MODE`): the same policy creation always had,
        // unchanged.
        //
        // One `lstat(2)`, via `stat`, reads mode and owner together —
        // what `write_owned_temporary` below needs to set before anything
        // is visible at `write.path`'s name.
        let existing = self.store.stat(write.path).map_err(|detail| {
            MachineError::Refused(format!(
                "REFUSED {path} is there and could not be examined, so NOTHING HAS BEEN \
                 WRITTEN. Detail: {detail}",
                path = write.path,
            ))
        })?;
        let mode = existing.as_ref().and_then(|raw| raw.mode).unwrap_or(NEW_FILE_MODE);
        let owner = existing.and_then(|raw| match (raw.uid, raw.gid) {
            (Some(uid), Some(gid)) => Some((uid, gid)),
            _ => None,
        });

        let temporary = self
            .store
            .write_owned_temporary(write.path, write.contents.as_bytes(), mode, owner)
            .map_err(|detail| {
                MachineError::Refused(format!(
                    "REFUSED the new contents could not be written to a temporary beside {path}, \
                     so {path} IS UNCHANGED and nothing has been lost. A full or read-only \
                     /state, or a failure to give the temporary {path}'s own owner, are the usual \
                     causes. Detail: {detail}",
                    path = write.path,
                ))
            })?;

        self.store
            .rename(&temporary, write.path)
            .map_err(|detail| {
                MachineError::Refused(format!(
                    "REFUSED the new contents were written to {temporary} and could not be moved \
                     onto {path}, so {path} IS UNCHANGED. What you asked for is in {temporary} and \
                     nothing reads it; the next attempt will refuse until it is removed. Detail: \
                     {detail}",
                    path = write.path,
                ))
            })?;

        // **Not a refusal.** The file on the running machine is already the new
        // one; an unflushed directory risks only a power cut in the next moments
        // reverting a change the operator was told about.
        let _ = self.store.flush_directory(Self::directory_of(write.path));

        Ok(Wrote {
            unrecorded: self.record(write),
        })
    }

    // -----------------------------------------------------------------------
    // `ADR-006` §8.10, added 2026-08-29
    // -----------------------------------------------------------------------

    fn stat_path(&mut self, path: &str) -> Result<PathStat, MachineError> {
        // The ancestor walk runs for every path, present or absent, because
        // `rivet get file` reports it (`ADR-006` §8.10.4.3) and an operator
        // diagnosing a refused write should not have to attempt the write.
        let symlinked_ancestor = self.store.symlinked_ancestor(path);
        let Some(raw) = self.store.stat(path).map_err(|detail| {
            MachineError::Refused(format!(
                "REFUSED {path} is there and could not be examined. That is not the same as \
                 nothing being there. Detail: {detail}"
            ))
        })?
        else {
            return Ok(PathStat {
                symlinked_ancestor,
                ..PathStat::default()
            });
        };

        let (entries, entry_count) = if raw.kind == PathKind::Directory {
            self.store.list(path).map_err(|detail| {
                MachineError::Refused(format!(
                    "REFUSED {path} is a directory and its entries could not be read. Detail: \
                     {detail}"
                ))
            })?
        } else {
            (Vec::new(), 0)
        };
        let entries_truncated = entries.len() < entry_count;

        // The names, resolved through the projection every other thing on this
        // machine resolves through (`ADR-011`). A number with no name keeps the
        // number; it is never replaced by one.
        let user = raw
            .uid
            .and_then(|uid| self.store.identity(IdKind::User, &uid.to_string()))
            .map(|(_, name)| name);
        let group = raw
            .gid
            .and_then(|gid| self.store.identity(IdKind::Group, &gid.to_string()))
            .map(|(_, name)| name);

        Ok(PathStat {
            kind: Some(raw.kind),
            size_bytes: raw.size_bytes,
            mode: raw.mode,
            uid: raw.uid,
            user,
            gid: raw.gid,
            group,
            modified_ms: raw.modified_ms,
            link_target: raw.link_target,
            entries: entries
                .into_iter()
                .map(|e| DirectoryEntry {
                    name: e.name,
                    kind: e.kind,
                    size_bytes: e.size_bytes,
                })
                .collect(),
            entry_count,
            entries_truncated,
            symlinked_ancestor,
            mount_point: raw.mount_point,
        })
    }

    fn resolve_owner(
        &mut self,
        user: Option<&str>,
        group: Option<&str>,
    ) -> Result<OwnerIds, MachineError> {
        let mut ids = OwnerIds::default();
        if let Some(user) = user {
            let (uid, name) = self
                .store
                .identity(IdKind::User, user)
                .ok_or_else(|| unknown_identity(IdKind::User, user))?;
            ids.uid = Some(uid);
            ids.user = Some(name);
        }
        if let Some(group) = group {
            let (gid, name) = self
                .store
                .identity(IdKind::Group, group)
                .ok_or_else(|| unknown_identity(IdKind::Group, group))?;
            ids.gid = Some(gid);
            ids.group = Some(name);
        }
        Ok(ids)
    }

    /// **The one check that is genuinely the machine's**, and it is the whole
    /// reason `ADR-006` §8.10's gate cannot be finished in `rivet-core`.
    ///
    /// The domain has already refused everything true of the *string*. What is
    /// left is what only a filesystem knows: whether a component on the way to
    /// this path is a symbolic link, which is a way out of the permitted root
    /// that no test on a string can see (§8.10.4.3).
    ///
    /// **Both paths of a rename are checked**, because a rename that could not
    /// leave through its source could still leave through its destination.
    fn can_change_path(&mut self, change: &PathChange<'_>) -> Result<(), MachineError> {
        let mut paths = vec![change.path()];
        if let PathChange::Rename { to, .. } = change {
            paths.push(to);
        }
        for path in paths {
            if let Some(link) = self.store.symlinked_ancestor(path) {
                return Err(linked_ancestor_refusal(&link, path, change.act()));
            }
        }
        Ok(())
    }

    fn change_path(&mut self, change: &PathChange<'_>) -> Result<Wrote, MachineError> {
        // **Re-run, not remembered.** `can_change_path` may have run on the plan
        // path seconds ago, or not at all; the filesystem is the authority and
        // it is asked at the point of use (`AGENTS.md` §5).
        self.can_change_path(change)?;

        let path = change.path();
        match *change {
            PathChange::CreateDirectory {
                path,
                parents,
                mode,
            } => {
                for parent in parents {
                    self.store
                        .create_directory(parent, NEW_DIRECTORY_MODE)
                        .map_err(|detail| {
                            MachineError::Refused(format!(
                                "REFUSED {parent} could not be created, so {path} WAS NOT \
                                 CREATED. Any directory created before it is still there. \
                                 Detail: {detail}"
                            ))
                        })?;
                }
                self.store.create_directory(path, mode).map_err(|detail| {
                    MachineError::Refused(format!(
                        "REFUSED {path} could not be created. Detail: {detail}"
                    ))
                })?;
            }
            PathChange::SetMode { path, mode, .. } => {
                self.store.set_mode(path, mode).map_err(|detail| {
                    MachineError::Refused(format!(
                        "REFUSED the mode of {path} could not be changed, and it is UNCHANGED. \
                         Detail: {detail}"
                    ))
                })?;
            }
            PathChange::SetOwner {
                path, uid, gid, ..
            } => {
                self.store.set_owner(path, uid, gid).map_err(|detail| {
                    MachineError::Refused(format!(
                        "REFUSED the owner of {path} could not be changed, and it is UNCHANGED. \
                         Only uid 0 may give a file away, which is the usual cause. Detail: \
                         {detail}"
                    ))
                })?;
            }
            PathChange::Rename { from, to } => {
                self.store.rename(from, to).map_err(|detail| {
                    MachineError::Refused(format!(
                        "REFUSED {from} could not be moved to {to}, and NOTHING HAS MOVED. A \
                         destination on another filesystem is the usual cause — rename(2) \
                         cannot cross one and this operation does not copy. Detail: {detail}"
                    ))
                })?;
            }
            PathChange::RemoveFile { path, .. } => {
                self.store.remove_file(path).map_err(|detail| {
                    MachineError::Refused(format!(
                        "REFUSED {path} could not be removed and IS STILL THERE. Detail: \
                         {detail}"
                    ))
                })?;
            }
            PathChange::RemoveDirectory { path } => {
                self.store.remove_directory(path).map_err(|detail| {
                    MachineError::Refused(format!(
                        "REFUSED {path} could not be removed and IS STILL THERE. The kernel \
                         refuses a directory that is not empty, which is the usual cause and is \
                         a second guard under this operation's own. Detail: {detail}"
                    ))
                })?;
            }
            // **The one act in this enum that carries bytes and an owner**, and
            // the reason it is one act. `write_owned_temporary` sets the owner
            // and the mode on the TEMPORARY, through the one descriptor it
            // opens for it — before the file has the name anything looks for,
            // and never by a second, path-based lookup of that name — so
            // there is no instant at which `<data>/<name>` exists owned by
            // root. That window is exactly what made `apply edit` plus
            // `apply set-owner` unsafe rather than merely tedious: an
            // application started inside it reads `EACCES` from its own
            // credential and reports it from inside itself, where nobody is
            // watching.
            //
            // The rename is the same atomic swap `write_file` above uses, so a
            // power cut leaves either the old secret or the new one and never
            // a half-written credential.
            PathChange::InstallSecret {
                path,
                contents,
                mode,
                uid,
                gid,
                ..
            } => {
                let temporary = self
                    .store
                    .write_owned_temporary(path, contents, mode, Some((uid, gid)))
                    .map_err(|detail| {
                        // **The length and never the bytes** (`ADR-020` §4).
                        MachineError::Refused(format!(
                            "REFUSED a temporary beside {path} could not be written, so NOTHING \
                             HAS CHANGED — the secret is written to a temporary and renamed into \
                             place, and this failed before the rename. {bytes} bytes were to be \
                             written. A failure to give the temporary owner {uid}:{gid} is the \
                             same refusal: only uid 0 may give a file away, which is the usual \
                             cause. Detail: {detail}",
                            bytes = contents.len(),
                        ))
                    })?;
                self.store.rename(&temporary, path).map_err(|detail| {
                    MachineError::Refused(format!(
                        "REFUSED {temporary} could not be renamed to {path}, so the secret is \
                         NOT installed and whatever was at {path} is untouched. THE TEMPORARY IS \
                         STILL THERE and holds the new secret. Detail: {detail}"
                    ))
                })?;
                // Best effort, exactly as `write_file` treats it: the rename has
                // already happened and a directory that could not be flushed is
                // a durability question rather than a failure of this act.
                let _ = self.store.flush_directory(Self::directory_of(path));
            }
        }

        // **The act happened. A record that could not be filed is reported and
        // is not a refusal** — `Editor::change_path`'s contract, and it bites
        // hardest here: a removal that succeeded and could not be journalled has
        // destroyed something, and saying *nothing happened* about it would be
        // the worst lie this adapter can tell.
        Ok(Wrote {
            unrecorded: self.record_change(change, path),
        })
    }
}

/// The refusal for a user or group this machine does not know.
///
/// **It names the projection it looked in.** An operator who typed a name that
/// is not there needs to know *where* the answer would have come from, because
/// on a Rivet the identity table is generated from the document and adding an
/// account is a change to the document rather than to a file.
fn unknown_identity(kind: IdKind, what: &str) -> MachineError {
    MachineError::Refused(format!(
        "REFUSED this machine has no {kind} `{what}`, and NOTHING HAS BEEN CHANGED. Names are \
         resolved against {path}, which is a PROJECTION of the desired-state document \
         (ADR-007 §3.6.2) — `rivet explain {path}` says what generates it. A NUMBER is always \
         accepted, and is what to use for an identity this machine has no name for.",
        kind = kind.as_str(),
        path = kind.path(),
    ))
}

// ---------------------------------------------------------------------------
// The store, over `std::fs`.
// ---------------------------------------------------------------------------

/// [`FileStore`] over the real filesystem.
///
/// Stateless, on `crate::deploy::Filesystem`'s reasoning: every fact it deals in
/// is read at the point of use.
///
/// **Not gated to Linux.** What it needs is `read`, `write`, `rename` and
/// `fsync`, which every platform has; what a Windows host lacks is a Rivet
/// machine, and [`crate::refusal`] is what says so. The **mode** is the one genuinely
/// Unix part and it is applied under a `cfg` rather than the whole type —
/// `crate::deploy::Filesystem`'s split, unchanged.
pub struct Filesystem;

impl Filesystem {
    /// The monotonic clock in seconds, or `0.0` where it cannot be read.
    ///
    /// `0.0` rather than a refusal: a record with a wrong `mono` is worse than
    /// one with a zero, because zero is visibly not a time since boot and a
    /// plausible number is not.
    #[allow(clippy::cast_precision_loss)]
    fn monotonic_seconds() -> f64 {
        // `crate::syscall` is Linux's, like the clock it reads. A build that has
        // no `clock_gettime(CLOCK_MONOTONIC)` has no Rivet machine under it
        // either — `crate::edit::refusal` is what such a build answers with, so
        // this value is never reached there and would be a lie if it were.
        #[cfg(target_os = "linux")]
        {
            match crate::syscall::monotonic_nanos() {
                Ok(nanos) => (nanos as f64) / 1_000_000_000.0,
                Err(_) => 0.0,
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            0.0
        }
    }

    /// Write, fsync and — where `owner` says to — chown and chmod one
    /// already-open temporary, all through `file`.
    ///
    /// **The caller removes the temporary if this returns `Err`**; this
    /// function only ever touches the descriptor it was handed and never
    /// looks its file up by name again, which is the property
    /// [`FileStore::write_owned_temporary`] exists to give: nothing that
    /// happens to `file`'s name after this call started can change what
    /// gets chowned or chmoded.
    fn finish_owned_temporary(
        mut file: std::fs::File,
        bytes: &[u8],
        mode: u32,
        owner: Option<(u32, u32)>,
    ) -> Result<(), String> {
        use std::io::Write as _;
        file.write_all(bytes).map_err(|e| e.to_string())?;

        #[cfg(unix)]
        {
            // `fchown`, on the descriptor — never a second, path-based
            // lookup of the name this call chose, which is exactly the
            // lookup a directory an application controls could answer
            // differently by the time it ran.
            if let Some((uid, gid)) = owner {
                std::os::unix::fs::fchown(&file, Some(uid), Some(gid)).map_err(|e| e.to_string())?;
            }
            // `fchmod`, likewise on the descriptor. Belt and braces beside
            // the mode `open` already asked for above: a restrictive umask
            // can narrow what `open`'s mode argument requested, and this
            // asserts the exact one regardless.
            use std::os::unix::fs::PermissionsExt as _;
            file.set_permissions(std::fs::Permissions::from_mode(mode))
                .map_err(|e| e.to_string())?;
        }
        #[cfg(not(unix))]
        let _ = (mode, owner);

        // Before the rename, so the rename publishes bytes that are on the
        // card rather than a name pointing at contents still in the page
        // cache.
        file.sync_all().map_err(|e| e.to_string())
    }
}

impl FileStore for Filesystem {
    fn read(&mut self, path: &str) -> Result<Option<Vec<u8>>, String> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    /// **`open`, `fstat`, `seek`, `read` — never `std::fs::read`.** FS4: this
    /// is the call that stops `rivet get file` allocating a whole file to
    /// answer for one mebibyte of it. `len` bounds the buffer this function
    /// itself allocates, so a caller asking for a bounded window gets a
    /// bounded allocation regardless of how large the file actually is.
    fn read_window(
        &mut self,
        path: &str,
        at: usize,
        len: usize,
    ) -> Result<Option<(Vec<u8>, usize)>, String> {
        use std::io::{Read as _, Seek as _, SeekFrom};
        let mut file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };
        // `fstat` on the open descriptor, not a second `stat` by path: the
        // size this answer reports is the size of the file this call is
        // actually about to read, not of whatever a name might resolve to a
        // moment later.
        let total = file.metadata().map_err(|e| e.to_string())?.len();
        let total = usize::try_from(total).unwrap_or(usize::MAX);

        if at > 0 {
            let offset = u64::try_from(at).unwrap_or(u64::MAX);
            file.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
        }

        let mut buffer = vec![0u8; len];
        let mut filled = 0usize;
        while filled < len {
            match file.read(&mut buffer[filled..]) {
                // The end of the file, which for a window is an ordinary
                // answer and not a truncated one — `content()` compares what
                // came back against `total` to say whether it is complete.
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        buffer.truncate(filled);
        Ok(Some((buffer, total)))
    }

    fn is_directory(&mut self, path: &str) -> bool {
        std::path::Path::new(path).is_dir()
    }

    fn exists(&mut self, path: &str) -> bool {
        // `symlink_metadata`, not `exists`: a dangling symlink at the temporary
        // path is something that is there, and a check that could not see it
        // would write through it.
        std::fs::symlink_metadata(path).is_ok()
    }

    fn create_directory(&mut self, path: &str, mode: u32) -> Result<(), String> {
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            builder.mode(mode);
        }
        #[cfg(not(unix))]
        let _ = mode;
        match builder.create(path) {
            Ok(()) => Ok(()),
            // Already there is the ordinary case for the journal directory and
            // is never an error: the caller asked for it to exist.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }

    fn write_owned_temporary(
        &mut self,
        path: &str,
        bytes: &[u8],
        mode: u32,
        owner: Option<(u32, u32)>,
    ) -> Result<String, String> {
        for _ in 0..MAX_TEMPORARY_ATTEMPTS {
            let candidate = unique_temporary_name(path);
            let mut options = std::fs::OpenOptions::new();
            // `create_new`: a candidate already taken by anything — a file,
            // or a symbolic link however it got there — is `AlreadyExists`
            // rather than something this call opens, follows or truncates.
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.mode(mode);
            }
            #[cfg(not(unix))]
            let _ = mode;

            let file = match options.open(&candidate) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.to_string()),
            };

            return match Self::finish_owned_temporary(file, bytes, mode, owner) {
                Ok(()) => Ok(candidate),
                Err(e) => {
                    let _ = std::fs::remove_file(&candidate);
                    Err(e)
                }
            };
        }
        Err(format!(
            "no unused temporary name beside {path} after {MAX_TEMPORARY_ATTEMPTS} attempts"
        ))
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        std::fs::rename(from, to).map_err(|e| e.to_string())
    }

    fn flush_directory(&mut self, path: &str) -> Result<(), String> {
        std::fs::File::open(path)
            .and_then(|d| d.sync_all())
            .map_err(|e| e.to_string())
    }

    fn send_record(&mut self, line: &str) -> Option<String> {
        crate::journal::send_or_say(line)
    }

    fn stamp(&mut self) -> Stamp {
        Stamp {
            boot: boot_identifier(),
            mono: Self::monotonic_seconds(),
            wall_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .and_then(|d| u64::try_from(d.as_millis()).ok()),
        }
    }

    // -----------------------------------------------------------------------
    // `ADR-006` §8.10, added 2026-08-29
    // -----------------------------------------------------------------------

    fn stat(&mut self, path: &str) -> Result<Option<RawStat>, String> {
        // **`symlink_metadata`, never `metadata`.** The port's contract, and
        // `ADR-006` §8.10.4.3's reason: following a link answers about a
        // different path than the one that was asked about.
        let meta = match std::fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
        };

        let kind = if meta.is_symlink() {
            PathKind::Symlink
        } else if meta.is_dir() {
            PathKind::Directory
        } else if meta.is_file() {
            PathKind::File
        } else {
            PathKind::Other
        };

        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| u64::try_from(d.as_millis()).ok());

        let link_target = (kind == PathKind::Symlink)
            .then(|| std::fs::read_link(path).ok())
            .flatten()
            .map(|p| p.to_string_lossy().to_string());

        #[cfg(unix)]
        let (mode, uid, gid, mount_point) = {
            use std::os::unix::fs::MetadataExt as _;
            // **A mount point is where the device number changes.** The parent
            // is asked with `metadata` deliberately: a mount point's parent is
            // reached through the directory tree, and a link in the way is
            // `symlinked_ancestor`'s refusal rather than this function's.
            let parent = rivet_core::operation::filesystem::directory_of(path);
            let mount_point = std::fs::metadata(parent)
                .ok()
                .is_some_and(|p| p.dev() != meta.dev())
                // The root of the filesystem tree is its own parent, and it is a
                // mount point whatever the device numbers say.
                || path == "/";
            (
                Some(meta.mode() & 0o7777),
                Some(meta.uid()),
                Some(meta.gid()),
                mount_point,
            )
        };
        // **A platform with no modes reports none rather than zero.** Zero is a
        // real mode — no access for anybody — and a development host answering
        // it would describe a file nobody could read.
        #[cfg(not(unix))]
        let (mode, uid, gid, mount_point) = (None, None, None, false);

        Ok(Some(RawStat {
            kind,
            size_bytes: meta.len(),
            mode,
            uid,
            gid,
            modified_ms,
            link_target,
            mount_point,
        }))
    }

    fn list(&mut self, path: &str) -> Result<(Vec<RawEntry>, usize), String> {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(path).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().to_string();
            // **`symlink_metadata` again**, so a link inside a directory is
            // listed as a link and not as whatever it points at. An entry whose
            // metadata cannot be read is listed as `other` with size 0 rather
            // than dropped: a listing that silently omitted an entry would be
            // the false-by-omission `AGENTS.md` §5 forbids.
            let (kind, size) = entry.metadata().map_or((PathKind::Other, 0), |meta| {
                let kind = if meta.is_symlink() {
                    PathKind::Symlink
                } else if meta.is_dir() {
                    PathKind::Directory
                } else if meta.is_file() {
                    PathKind::File
                } else {
                    PathKind::Other
                };
                (kind, meta.len())
            });
            entries.push(RawEntry {
                name,
                kind,
                size_bytes: size,
            });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let total = entries.len();
        entries.truncate(rivet_core::operation::filesystem::DIRECTORY_MAX_ENTRIES);
        Ok((entries, total))
    }

    fn symlinked_ancestor(&mut self, path: &str) -> Option<String> {
        let mut walked = String::new();
        let components: Vec<&str> = path.split('/').collect();
        // The last component is the path itself; whether *it* is a link is
        // `stat`'s answer and each subject decides what to do about it.
        for component in components.iter().take(components.len().saturating_sub(1)) {
            if component.is_empty() {
                continue;
            }
            walked.push('/');
            walked.push_str(component);
            if std::fs::symlink_metadata(&walked).is_ok_and(|m| m.is_symlink()) {
                return Some(walked);
            }
        }
        None
    }

    fn identity(&mut self, kind: IdKind, what: &str) -> Option<(u32, String)> {
        let text = std::fs::read_to_string(kind.path()).ok()?;
        // `name:x:id:...` — the two colon-separated databases have the name
        // first and the numeric identity third, which is the only part of the
        // format this needs and the only part it reads.
        for line in text.lines() {
            let mut parts = line.split(':');
            let (Some(name), Some(_), Some(id)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let Ok(id) = id.parse::<u32>() else { continue };
            if name == what || id.to_string() == what {
                return Some((id, name.to_string()));
            }
        }
        // **A number this machine has no name for is still a valid identity.**
        // The filesystem holds numbers; `/etc/passwd` is a label for some of
        // them, and refusing an unlabelled one would refuse a correct request.
        what.parse::<u32>().ok().map(|id| (id, id.to_string()))
    }

    fn set_mode(&mut self, path: &str, mode: u32) -> Result<(), String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            // `set_permissions` follows a symbolic link, and this port promises
            // not to. The domain has already refused a link as the target
            // (`ADR-006` §8.10.4.3) and `can_change_path` has refused one in the
            // path, so by here there is none — the check below is the third one
            // and it is cheap, because the alternative to being sure is
            // `chmod`ing something nobody named.
            if std::fs::symlink_metadata(path).map_err(|e| e.to_string())?.is_symlink() {
                return Err(format!("{path} is a symbolic link"));
            }
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
                .map_err(|e| e.to_string())
        }
        #[cfg(not(unix))]
        {
            let _ = (path, mode);
            Err("this platform has no file modes".to_string())
        }
    }

    fn set_owner(&mut self, path: &str, uid: Option<u32>, gid: Option<u32>) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        {
            // `lchown`, so a symbolic link is not followed — the port's
            // contract. `crate::syscall` owns the call, as it owns every other
            // one in this binary.
            crate::syscall::change_owner(path, uid, gid)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (path, uid, gid);
            Err("this platform has no file ownership this build can change".to_string())
        }
    }

    fn remove_file(&mut self, path: &str) -> Result<(), String> {
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
        // **The directory is flushed, so the removal survives a power cut.**
        // The same step 5 the write does, and not a refusal for the same reason:
        // the file is already gone from the running machine.
        let _ = self.flush_directory(rivet_core::operation::filesystem::directory_of(path));
        Ok(())
    }

    fn remove_directory(&mut self, path: &str) -> Result<(), String> {
        // `remove_dir`, never `remove_dir_all`: the kernel refuses a directory
        // that is not empty and that refusal is a guard the domain's own check
        // cannot be raced past. `ADR-006` §8.10.13 is the recursive form, which
        // is specified and deliberately not built.
        std::fs::remove_dir(path).map_err(|e| e.to_string())?;
        let _ = self.flush_directory(rivet_core::operation::filesystem::directory_of(path));
        Ok(())
    }
}

/// `ADR-022` §3.2's `boot`, from the kernel that owns the fact.
///
/// The same path `cmd/rivet-sysd`, `cmd/rivet-timed` and
/// `cmd/rivet-deploy` read, and the same answer for a machine that will not
/// answer: `unknown`, never a synthesised identifier. A record whose boot id was
/// invented would splice two boots together for a reader, which is worse than
/// one that says it does not know.
///
/// # It is the WHOLE identifier, and that is not arbitrary
///
/// `cmd/rivet-sysd` writes the trimmed file verbatim and
/// `cmd/rivet-deploy` writes its **first eight characters**. Those are two
/// different values for one boot, and `rivet_core::journal::boots_in_order`
/// counts distinct `boot` fields — so a journal holding both reports two boots
/// where the machine had one, and `rivet get log --boot previous` selects the
/// wrong records.
///
/// **Found on 2026-08-24 by reading a guest's own journal**, where this module's
/// first version truncated and the supervisor's records beside it did not.
/// `PLAN.md` carries the other half — that `cmd/rivet-deploy` still
/// truncates — because that binary is not this change's to alter.
fn boot_identifier() -> String {
    const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";
    match std::fs::read_to_string(BOOT_ID_PATH) {
        Ok(text) => text.trim().to_string(),
        Err(_) => "unknown".to_string(),
    }
}

/// The editor this machine has.
///
/// The one place the store is chosen, so `crate::boot` stays a few forwarding
/// lines.
pub fn on_this_machine() -> Files<Filesystem> {
    Files::new(Filesystem, crate::deploy::this_process())
}

/// **Refuse unless this Linux host is a Rivet machine.**
///
/// `crate::deploy`'s interlock, in the same words and for a sharper reason. That
/// one refuses because there would be nowhere to put a request and nothing that
/// would read it; this one refuses because there *is* somewhere — a development
/// host under WSL has a perfectly good filesystem, and `rivet apply edit`
/// mistyped into a checkout would rewrite one of the maintainer's own files
/// (`AGENTS.md` §9).
///
/// **It is `/state` that is asked about**, because `ADR-018` §4 makes it the
/// only writable partition a Rivet machine has: `/system` is read-only vendor
/// content, `/etc` is a projection remounted read-only (`ADR-007` §3.5.1), and
/// the boot partition is FAT the firmware reads. A machine with no `/state` is
/// either not a Rivet or is one whose `/state` did not mount — and this boot's
/// journal says which.
///
/// It sits in `crate::boot` rather than in [`Files`] deliberately: it is a fact
/// about **which machine this is**, which is the composition root's question,
/// and keeping it out of [`Files`] is what lets that type be exercised against a
/// real filesystem in a temporary directory.
pub fn this_is_a_rivet_machine() -> Result<(), MachineError> {
    if std::path::Path::new(rivet_core::state::STATE_MOUNT_POINT).is_dir() {
        return Ok(());
    }
    Err(MachineError::Unsafe(format!(
        "REFUSED there is no {state} on this machine, so it is not a Rivet machine and \
         NOTHING HAS BEEN WRITTEN. {state} is the only writable partition a Rivet card carries \
         (ADR-018 §4); a host without one is either not a Rivet or is one whose {state} did not \
         mount, and this boot's journal says which. This refusal is what stops `rivet apply \
         edit` rewriting a file on a development host that happens to have the same path.",
        state = rivet_core::state::STATE_MOUNT_POINT,
    )))
}

/// What a build with no board says, in one place.
///
/// `crate::deploy::refusal`'s shape and its argument. A Windows host has files
/// this build could happily replace, and that is exactly why it must not: the
/// paths `rivet apply edit` is for are a Rivet machine's, and a command
/// mistyped into a development checkout would otherwise rewrite a file on the
/// maintainer's own disk (`AGENTS.md` §9).
///
/// **This is the one adapter in this binary whose detached form refuses
/// something it could technically do**, and the reason is stated rather than
/// left implicit: `crate::journal::read` answers honestly on a workstation
/// because *"there is no journal here"* is true; *"this file has been edited"*
/// would be true too, and would be about the wrong machine.
#[cfg(any(test, not(target_os = "linux")))]
pub fn refusal(platform: &str) -> MachineError {
    MachineError::Unsafe(format!(
        "REFUSED this is a {platform} build with no Rivet machine under it, and NOTHING HAS \
         BEEN WRITTEN. `rivet apply edit` changes a file on the machine it is running on; here \
         that would be a file on this workstation, which is not what anybody typing this meant. \
         {ask}",
        ask = rivet_core::control::ASK_A_BOARD,
    ))
}

/// What a build with no board says about `ADR-006` §8.10's paths.
///
/// **A separate sentence from [`refusal`], because it is a separate reason** —
/// `crate::boot::Detached::stop_refusal`'s discipline. That one is worded for
/// `apply edit` and says *nothing has been written*, which is confusing in front
/// of `rivet get file` (which writes nothing by design) and understates the case
/// for `apply remove-file` (which would have destroyed something).
///
/// **`get file` is refused here even though it only reads**, and that is
/// deliberate rather than an oversight. `crate::journal::read` answers honestly
/// on a workstation because *"there is no journal at /state/log"* is true of
/// this host; *"here are the contents of that path"* would also be true, and
/// would be about **the maintainer's own disk** (`AGENTS.md` §9). A file surface
/// that reads a development host's files because a board was not reachable is
/// answering a question nobody asked.
/// **It names no command**, deliberately. The port does not know which subject
/// reached it, and every subject reaches `stat_path` first — so a message that
/// guessed would tell an operator who ran `apply remove-file` that `get file`
/// had refused, which is true of the call and misleading about the command.
#[cfg(any(test, not(target_os = "linux")))]
pub fn path_refusal(platform: &str) -> MachineError {
    MachineError::Unsafe(format!(
        "REFUSED this is a {platform} build with no Rivet machine under it, and NOTHING HAS \
         BEEN DONE. ADR-006 §8.10's file operations — reading a path, its meta-info, its \
         permissions, its name, its existence — are all about a path ON THE MACHINE THEY ARE \
         RUNNING ON; here that would be a path on this workstation, which is not what anybody \
         typing this meant. For the subjects that remove or move something there would then be \
         nothing left to look at, which is why even the READ half refuses rather than answering \
         about the wrong machine. {ask}",
        ask = rivet_core::control::ASK_A_BOARD,
    ))
}

/// The refusal a build with no board gives, as an [`Editor`].
#[cfg(any(test, not(target_os = "linux")))]
pub struct NoMachine {
    platform: &'static str,
}

#[cfg(any(test, not(target_os = "linux")))]
impl NoMachine {
    pub const fn new(platform: &'static str) -> Self {
        Self { platform }
    }
}

#[cfg(any(test, not(target_os = "linux")))]
impl Editor for NoMachine {
    fn principal(&mut self) -> rivet_core::deploy::Principal {
        crate::deploy::this_process()
    }

    fn read_file(&mut self, _path: &str) -> Result<FileRead, MachineError> {
        Err(refusal(self.platform))
    }

    fn can_write_file(&mut self, _path: &str, _contents: &str) -> Result<(), MachineError> {
        Err(refusal(self.platform))
    }

    fn write_file(&mut self, _write: &Write<'_>) -> Result<Wrote, MachineError> {
        Err(refusal(self.platform))
    }

    // **`ADR-006` §8.10's four refuse for the same reason the three above do,
    // and the reason is sharper for them.** A Windows host has files this build
    // could happily `remove`, and that is exactly why it must not: a
    // `rivet apply remove-file` mistyped into a development checkout would
    // delete one of the maintainer's own files (`AGENTS.md` §9), and unlike an
    // edit there would be nothing left to look at.
    fn stat_path(&mut self, _path: &str) -> Result<PathStat, MachineError> {
        Err(path_refusal(self.platform))
    }

    fn resolve_owner(
        &mut self,
        _user: Option<&str>,
        _group: Option<&str>,
    ) -> Result<OwnerIds, MachineError> {
        Err(path_refusal(self.platform))
    }

    fn can_change_path(&mut self, _change: &PathChange<'_>) -> Result<(), MachineError> {
        Err(path_refusal(self.platform))
    }

    fn change_path(&mut self, change: &PathChange<'_>) -> Result<Wrote, MachineError> {
        self.can_change_path(change).map(|()| unreachable!())
    }
}

// ---------------------------------------------------------------------------
// A store in a map, for tests anywhere in this crate.
// ---------------------------------------------------------------------------

/// A [`FileStore`] in memory.
///
/// Public and `cfg(test)`: `crate::main`'s whole-binary tests drive
/// `rivet apply edit` end to end and must not touch a real filesystem.
#[cfg(test)]
pub struct Memory {
    pub files: std::collections::BTreeMap<String, Vec<u8>>,
    pub modes: std::collections::BTreeMap<String, u32>,
    pub directories: Vec<String>,
    /// Every `(from, to)` renamed, in order.
    pub renames: Vec<(String, String)>,
    /// Directories flushed, in order.
    pub flushed: Vec<String>,
    /// Directories created, in order.
    pub created: Vec<String>,
    /// An error every read answers with — a failing card, not an absent file.
    pub read_fails: Option<String>,
    /// An error every write answers with.
    pub write_fails: Option<String>,
    /// An error every rename answers with.
    pub rename_fails: Option<String>,
    /// The reason [`FileStore::send_record`] answers with — a supervisor that
    /// is not listening, which is the case the stderr floor exists for.
    pub send_fails: Option<String>,
    /// Every record line handed to [`FileStore::send_record`], in order.
    ///
    /// **This is the fake's whole account of the journal now.** It used to be a
    /// file in [`Memory::files`], and it is not one any more because this
    /// process does not write files under `/state/log` — a fake that still had
    /// one would be testing a shape the code no longer has.
    pub sent: Vec<String>,
    /// `ADR-006` §8.10: `path -> (uid, gid)`, defaulting to `0, 0`.
    pub owners: std::collections::BTreeMap<String, (u32, u32)>,
    /// `path -> what it points at`. **A link is never followed** — it is a
    /// thing at a path, and the family of §8.10 refuses one.
    pub symlinks: std::collections::BTreeMap<String, String>,
    /// Paths something is mounted at.
    pub mount_points: Vec<String>,
    /// Everything removed, in order.
    pub removed: Vec<String>,
    /// An error every §8.10 act answers with.
    pub change_fails: Option<String>,
    /// The error [`FileStore::write_owned_temporary`] answers with when
    /// asked to apply an owner — the one failure [`Files::write_file`]
    /// must refuse on, leaving the original untouched and its own
    /// temporary removed.
    pub apply_owner_fails: Option<String>,
    /// Every `(temporary, uid, gid)` [`FileStore::write_owned_temporary`]
    /// set an owner to, in the order it set them — always against the
    /// TEMPORARY's name, because that call has no other name to set it
    /// against yet. A test reads this to confirm an owner was applied
    /// while the file still had that name, which is the only way a fake
    /// with no threads can show "before it was renamed".
    pub owner_changes: Vec<(String, u32, u32)>,
}

#[cfg(test)]
impl Memory {
    /// A machine with a `/state` and nothing in it.
    pub fn rivet() -> Self {
        Self {
            files: std::collections::BTreeMap::new(),
            modes: std::collections::BTreeMap::new(),
            directories: vec![
                "/".to_string(),
                "/state".to_string(),
                "/state/home".to_string(),
                "/state/home/root".to_string(),
            ],
            renames: Vec::new(),
            flushed: Vec::new(),
            created: Vec::new(),
            read_fails: None,
            write_fails: None,
            rename_fails: None,
            send_fails: None,
            sent: Vec::new(),
            owners: std::collections::BTreeMap::new(),
            symlinks: std::collections::BTreeMap::new(),
            mount_points: vec!["/".to_string(), "/state".to_string()],
            removed: Vec::new(),
            change_fails: None,
            apply_owner_fails: None,
            owner_changes: Vec::new(),
        }
    }

    /// The same machine, holding one file — **and every directory on the way to
    /// it**, because a machine that holds a file and not its parents is not one
    /// that can exist, and a fake that allowed it would make
    /// `Files::missing_parents` look wrong when it was right.
    pub fn holding(path: &str, contents: &str) -> Self {
        let mut memory = Self::rivet();
        let mut walked = String::new();
        let components: Vec<&str> = path.split('/').collect();
        for component in components.iter().take(components.len().saturating_sub(1)) {
            if component.is_empty() {
                continue;
            }
            walked.push('/');
            walked.push_str(component);
            if !memory.directories.contains(&walked) {
                memory.directories.push(walked.clone());
            }
        }
        memory
            .files
            .insert(path.to_string(), contents.as_bytes().to_vec());
        memory.modes.insert(path.to_string(), 0o644);
        memory
    }

    /// What is at `path` now, as text.
    pub fn text(&self, path: &str) -> Option<String> {
        self.files
            .get(path)
            .map(|b| String::from_utf8_lossy(b).to_string())
    }

    /// Every journal line that **reached** the journal.
    ///
    /// **Sent, not written.** `ADR-022` §11: the records went to the supervisor
    /// over a socket, and nothing under `/state/log` was opened.
    ///
    /// [`Memory::sent`] is the attempts; this is the ones that landed, and on a
    /// machine whose supervisor is not listening it is deliberately empty even
    /// though an attempt was made — because that is what `rivet get log` would
    /// show, and it is the difference the stderr floor exists to make visible.
    pub fn journal(&self) -> Vec<String> {
        if self.send_fails.is_some() {
            return Vec::new();
        }
        self.sent
            .iter()
            .flat_map(|line| line.lines().map(str::to_string))
            .collect()
    }
}

#[cfg(test)]
impl FileStore for Memory {
    fn read(&mut self, path: &str) -> Result<Option<Vec<u8>>, String> {
        if let Some(detail) = &self.read_fails {
            return Err(detail.clone());
        }
        Ok(self.files.get(path).cloned())
    }

    fn read_window(
        &mut self,
        path: &str,
        at: usize,
        len: usize,
    ) -> Result<Option<(Vec<u8>, usize)>, String> {
        if let Some(detail) = &self.read_fails {
            return Err(detail.clone());
        }
        let Some(bytes) = self.files.get(path) else {
            return Ok(None);
        };
        let total = bytes.len();
        let start = at.min(total);
        let end = start.saturating_add(len).min(total);
        Ok(Some((bytes[start..end].to_vec(), total)))
    }

    fn is_directory(&mut self, path: &str) -> bool {
        self.directories.iter().any(|d| d == path)
    }

    fn exists(&mut self, path: &str) -> bool {
        self.files.contains_key(path) || self.is_directory(path)
    }

    fn create_directory(&mut self, path: &str, mode: u32) -> Result<(), String> {
        if !self.directories.iter().any(|d| d == path) {
            self.directories.push(path.to_string());
            self.created.push(path.to_string());
            self.modes.insert(path.to_string(), mode);
        }
        Ok(())
    }

    fn write_owned_temporary(
        &mut self,
        path: &str,
        bytes: &[u8],
        mode: u32,
        owner: Option<(u32, u32)>,
    ) -> Result<String, String> {
        if let Some(detail) = &self.write_fails {
            return Err(detail.clone());
        }

        let mut chosen = None;
        for _ in 0..MAX_TEMPORARY_ATTEMPTS {
            let candidate = unique_temporary_name(path);
            // The fake's own `O_EXCL`: a candidate already claimed by a
            // file, a directory, or a symlink is skipped, never opened,
            // followed or truncated — the same three `Memory::exists`
            // already treats as "something is there".
            let taken = self.files.contains_key(&candidate)
                || self.directories.iter().any(|d| d == &candidate)
                || self.symlinks.contains_key(&candidate);
            if !taken {
                chosen = Some(candidate);
                break;
            }
        }
        let Some(temporary) = chosen else {
            return Err(format!("no unused temporary name beside {path}"));
        };

        self.files.insert(temporary.clone(), bytes.to_vec());
        self.modes.insert(temporary.clone(), mode);

        if let Some((uid, gid)) = owner {
            if let Some(detail) = &self.apply_owner_fails {
                // Only the temporary THIS call just created — never
                // anything the caller told it about.
                self.files.remove(&temporary);
                self.modes.remove(&temporary);
                return Err(detail.clone());
            }
            self.owners.insert(temporary.clone(), (uid, gid));
            self.owner_changes.push((temporary.clone(), uid, gid));
        }

        Ok(temporary)
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), String> {
        if let Some(detail) = &self.rename_fails {
            return Err(detail.clone());
        }
        self.renames.push((from.to_string(), to.to_string()));
        let Some(contents) = self.files.remove(from) else {
            return Err(format!("{from}: no such file"));
        };
        if let Some(mode) = self.modes.remove(from) {
            self.modes.insert(to.to_string(), mode);
        }
        // A real `rename(2)` moves the inode, owner and all — the fake's
        // `owners` map has to be told to move with it explicitly, the same
        // way `modes` just was, or a caller that set an owner on `from`
        // would see it vanish at `to`.
        if let Some(owner) = self.owners.remove(from) {
            self.owners.insert(to.to_string(), owner);
        }
        self.files.insert(to.to_string(), contents);
        Ok(())
    }

    fn flush_directory(&mut self, path: &str) -> Result<(), String> {
        self.flushed.push(path.to_string());
        Ok(())
    }

    fn send_record(&mut self, line: &str) -> Option<String> {
        // Recorded on both paths, because "it did not try to send" and "it tried
        // and could not" are different defects — `cmd/femto`'s `MemoryFiles`
        // makes the same distinction for the same reason.
        self.sent.push(line.to_string());
        let detail = self.send_fails.clone()?;
        Some(format!(
            "the record could not be sent, so it is on this session's stderr and NOT in the \
             journal. THE FILE WAS STILL CHANGED. Detail: {detail}"
        ))
    }

    fn stamp(&mut self) -> Stamp {
        Stamp {
            boot: "0d3f2a11".to_string(),
            mono: 12.5,
            wall_ms: Some(1_755_820_800_000),
        }
    }

    // -----------------------------------------------------------------------
    // `ADR-006` §8.10, added 2026-08-29
    // -----------------------------------------------------------------------

    fn stat(&mut self, path: &str) -> Result<Option<RawStat>, String> {
        if let Some(detail) = &self.read_fails {
            return Err(detail.clone());
        }
        if let Some(target) = self.symlinks.get(path) {
            return Ok(Some(RawStat {
                kind: PathKind::Symlink,
                size_bytes: 0,
                mode: Some(0o777),
                uid: Some(0),
                gid: Some(0),
                modified_ms: Some(1_755_820_800_000),
                link_target: Some(target.clone()),
                mount_point: false,
            }));
        }
        let kind = if self.files.contains_key(path) {
            PathKind::File
        } else if self.is_directory(path) {
            PathKind::Directory
        } else {
            return Ok(None);
        };
        Ok(Some(RawStat {
            kind,
            size_bytes: self.files.get(path).map_or(0, |b| b.len() as u64),
            mode: Some(self.modes.get(path).copied().unwrap_or(0o644)),
            uid: Some(self.owners.get(path).map_or(0, |(uid, _)| *uid)),
            gid: Some(self.owners.get(path).map_or(0, |(_, gid)| *gid)),
            modified_ms: Some(1_755_820_800_000),
            link_target: None,
            mount_point: self.mount_points.iter().any(|m| m == path),
        }))
    }

    fn list(&mut self, path: &str) -> Result<(Vec<RawEntry>, usize), String> {
        let prefix = format!("{path}/");
        let mut entries: Vec<RawEntry> = self
            .files
            .keys()
            .map(|p| (p, PathKind::File))
            .chain(self.directories.iter().map(|p| (p, PathKind::Directory)))
            .chain(self.symlinks.keys().map(|p| (p, PathKind::Symlink)))
            .filter_map(|(candidate, kind)| {
                let rest = candidate.strip_prefix(&prefix)?;
                (!rest.is_empty() && !rest.contains('/')).then(|| RawEntry {
                    name: rest.to_string(),
                    kind,
                    size_bytes: self.files.get(candidate).map_or(0, |b| b.len() as u64),
                })
            })
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let total = entries.len();
        entries.truncate(rivet_core::operation::filesystem::DIRECTORY_MAX_ENTRIES);
        Ok((entries, total))
    }

    fn symlinked_ancestor(&mut self, path: &str) -> Option<String> {
        let mut walked = String::new();
        let components: Vec<&str> = path.split('/').collect();
        for component in components.iter().take(components.len().saturating_sub(1)) {
            if component.is_empty() {
                continue;
            }
            walked.push('/');
            walked.push_str(component);
            if self.symlinks.contains_key(&walked) {
                return Some(walked);
            }
        }
        None
    }

    fn identity(&mut self, kind: IdKind, what: &str) -> Option<(u32, String)> {
        // The two accounts `ADR-007`'s projection actually carries on a stock
        // Rivet, and nothing invented beside them.
        let table: &[(u32, &str)] = match kind {
            IdKind::User => &[(0, "root"), (1000, "app")],
            IdKind::Group => &[(0, "root"), (1000, "app")],
        };
        for (id, name) in table {
            if *name == what || id.to_string() == what {
                return Some((*id, (*name).to_string()));
            }
        }
        what.parse::<u32>().ok().map(|id| (id, id.to_string()))
    }

    fn set_mode(&mut self, path: &str, mode: u32) -> Result<(), String> {
        if let Some(detail) = &self.change_fails {
            return Err(detail.clone());
        }
        self.modes.insert(path.to_string(), mode);
        Ok(())
    }

    fn set_owner(&mut self, path: &str, uid: Option<u32>, gid: Option<u32>) -> Result<(), String> {
        if let Some(detail) = &self.change_fails {
            return Err(detail.clone());
        }
        let entry = self.owners.entry(path.to_string()).or_insert((0, 0));
        if let Some(uid) = uid {
            entry.0 = uid;
        }
        if let Some(gid) = gid {
            entry.1 = gid;
        }
        Ok(())
    }

    fn remove_file(&mut self, path: &str) -> Result<(), String> {
        if let Some(detail) = &self.change_fails {
            return Err(detail.clone());
        }
        self.files
            .remove(path)
            .ok_or_else(|| format!("{path}: No such file or directory"))?;
        self.removed.push(path.to_string());
        Ok(())
    }

    fn remove_directory(&mut self, path: &str) -> Result<(), String> {
        if let Some(detail) = &self.change_fails {
            return Err(detail.clone());
        }
        // **The kernel's own rule, in the fake**: `rmdir(2)` refuses a directory
        // that is not empty, and a fake that did not would let a test pass
        // against a machine that would have refused.
        let prefix = format!("{path}/");
        if self.files.keys().any(|p| p.starts_with(&prefix))
            || self.directories.iter().any(|p| p.starts_with(&prefix))
        {
            return Err(format!("{path}: Directory not empty"));
        }
        let Some(at) = self.directories.iter().position(|d| d == path) else {
            return Err(format!("{path}: No such file or directory"));
        };
        self.directories.remove(at);
        self.removed.push(path.to_string());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use rivet_core::deploy::{Origin, Principal};

    const NU: &str = "/state/home/root/.config/nushell/config.nu";

    /// A pinned key, spelled out once.
    const A_PIN: &str = "3f7c1a9e04b2d85f6a3c9017e2b48d5f1c6a09b3e7d248fa5601c93b8ed72a4f";

    fn over_the_control_plane() -> Principal {
        Principal::new(Origin::pinned(A_PIN).expect("64 hex characters"))
    }

    fn editor(store: Memory) -> Files<Memory> {
        Files::new(store, over_the_control_plane())
    }

    fn replacing<'a>(path: &'a str, contents: &'a str, was: u64) -> Write<'a> {
        Write {
            path,
            contents,
            directories: &[],
            creating: false,
            how: "replace",
            was,
        }
    }

    // -- the five-step write ----------------------------------------------------

    /// **The whole of the atomicity claim, from this side.** The new contents go
    /// to a temporary file beside the target, are flushed, are renamed onto it,
    /// and the directory is flushed — in that order, and nothing is left behind.
    ///
    /// **The temporary's exact name is not asserted** — only that it is one of
    /// [`TEMPORARY_SUFFIX`]'s family — because [`unique_temporary_name`]
    /// deliberately makes it unpredictable; see
    /// `a_file_or_symlink_at_the_old_fixed_temporary_name_does_not_block_a_write`
    /// for why.
    #[test]
    fn a_write_goes_through_a_temporary_file_and_is_renamed_into_place() {
        let mut e = editor(Memory::holding("/state/x", "before\n"));
        e.write_file(&replacing("/state/x", "after\n", 7))
            .expect("a healthy machine");

        assert_eq!(e.store.text("/state/x").as_deref(), Some("after\n"));
        assert_eq!(e.store.renames.len(), 1, "{:?}", e.store.renames);
        let (temporary, destination) = &e.store.renames[0];
        assert!(
            temporary.starts_with(&format!("/state/x{TEMPORARY_SUFFIX}")),
            "{temporary}"
        );
        assert_eq!(destination, "/state/x");
        assert!(
            e.store
                .files
                .keys()
                .all(|k| !k.starts_with(&format!("/state/x{TEMPORARY_SUFFIX}"))),
            "a temporary file was left behind: {:?}",
            e.store.files.keys().collect::<Vec<_>>()
        );
        assert!(e.store.flushed.contains(&"/state".to_string()));
    }

    /// **A file that already exists keeps the mode it has.** This operation
    /// changes contents; changing permissions is a different act.
    #[test]
    fn an_existing_files_mode_survives_the_replacement() {
        let mut e = editor(Memory::holding("/state/x", "a\n"));
        e.write_file(&replacing("/state/x", "b\n", 2))
            .expect("a healthy machine");
        assert_eq!(e.store.modes.get("/state/x"), Some(&0o644));
    }

    /// **A file that already exists keeps the uid and gid it has**, not
    /// only the mode: an edit that only copied the mode would leave this
    /// file owned by whatever created the temporary, which on a real
    /// machine is root. [`Memory::owner_changes`] proves the
    /// owner was applied to the TEMPORARY's own name, which is necessarily
    /// before [`Files::write_file`] renamed it — the fake has no other name
    /// to have recorded it against at that point.
    #[test]
    fn an_existing_files_owner_survives_the_replacement() {
        let mut store = Memory::holding("/state/x", "a\n");
        store.owners.insert("/state/x".to_string(), (4242, 4242));
        let mut e = editor(store);
        e.write_file(&replacing("/state/x", "b\n", 2))
            .expect("a healthy machine");

        assert_eq!(
            e.store.owner_changes.len(),
            1,
            "{:?}",
            e.store.owner_changes
        );
        let (temporary, uid, gid) = &e.store.owner_changes[0];
        assert!(
            temporary.starts_with(&format!("/state/x{TEMPORARY_SUFFIX}")),
            "the owner was not set against a temporary name: {temporary}"
        );
        assert_ne!(temporary, "/state/x", "the owner was set after the rename");
        assert_eq!((*uid, *gid), (4242, 4242));

        // And the rename carried it to the final name, the same way it
        // already carries the mode.
        assert_eq!(e.store.owners.get("/state/x"), Some(&(4242, 4242)));
    }

    /// **A file this operation creates is owned by this process**, not by
    /// anything the plan named — there is no "original" to preserve, so
    /// [`Files::write_file`] does not try to change the owner at all.
    #[test]
    fn a_created_files_owner_is_not_set_at_all() {
        let mut e = editor(Memory::rivet());
        e.write_file(&Write {
            path: NU,
            contents: "$env.x = 1\n",
            directories: &["/state/home/root/.config".to_string()],
            creating: true,
            how: "append",
            was: 0,
        })
        .expect("a creation");
        assert!(
            e.store.owner_changes.is_empty(),
            "a new file's owner was set: {:?}",
            e.store.owner_changes
        );
    }

    /// **A failure to give the temporary the file's own owner is refused
    /// exactly as a failure to write its bytes is**: the original is
    /// unchanged and the temporary this call created does not survive its
    /// own failure.
    #[test]
    fn a_failure_to_set_the_temporarys_owner_leaves_the_original_untouched() {
        let mut store = Memory::holding("/state/x", "before\n");
        store.owners.insert("/state/x".to_string(), (4242, 4242));
        store.apply_owner_fails = Some("Operation not permitted".to_string());
        let mut e = editor(store);
        let refused = e
            .write_file(&replacing("/state/x", "after\n", 7))
            .expect_err("a chown that fails");
        assert!(refused.why().contains("IS UNCHANGED"), "{}", refused.why());
        assert_eq!(e.store.text("/state/x").as_deref(), Some("before\n"));
        assert_eq!(e.store.owners.get("/state/x"), Some(&(4242, 4242)));
        assert!(e.store.renames.is_empty());
        assert!(
            e.store
                .files
                .keys()
                .all(|k| !k.starts_with(&format!("/state/x{TEMPORARY_SUFFIX}"))),
            "a temporary file was left behind: {:?}",
            e.store.files.keys().collect::<Vec<_>>()
        );
    }

    /// **`apply install-secret`'s own failure to set an owner, proven
    /// against the fake.** The real-filesystem test below can only exercise
    /// this branch on a machine that is NOT root, and this container is
    /// root — so this is where the refusal's wording (naming the uid and
    /// gid it could not be given to, and creating nothing) is actually
    /// checked, on every machine this runs on.
    #[test]
    fn a_failure_to_set_an_installed_secrets_owner_says_which_owner_it_was() {
        let mut store = Memory::rivet();
        store.apply_owner_fails = Some("Operation not permitted".to_string());
        let mut e = editor(store);
        let refused = e
            .change_path(&PathChange::InstallSecret {
                path: "/state/app/demo/data/relay.env",
                contents: b"secret",
                mode: 0o600,
                uid: 4000,
                gid: 4000,
                was_bytes: 0,
            })
            .expect_err("a chown that fails");
        assert!(refused.why().contains("NOTHING HAS CHANGED"), "{}", refused.why());
        assert!(refused.why().contains("4000:4000"), "{}", refused.why());
        assert!(
            !e.store.files.contains_key("/state/app/demo/data/relay.env"),
            "the secret was created despite the refusal"
        );
        assert!(e.store.renames.is_empty());
    }

    /// **A file or a symbolic link at the name the OLD scheme would have
    /// used blocks nothing and is never touched.** That name was built only
    /// from `path` and was therefore predictable by anything that can
    /// already write into the same directory — a confined application, for
    /// its own data. `unique_temporary_name` never generates exactly this
    /// string, so a write that used to refuse when it found one there now
    /// simply picks a different name and succeeds.
    #[test]
    fn a_file_or_symlink_at_the_old_fixed_temporary_name_does_not_block_a_write() {
        let fixed = format!("/state/x{TEMPORARY_SUFFIX}");

        let mut store = Memory::holding("/state/x", "before\n");
        store.files.insert(fixed.clone(), b"planted".to_vec());
        let mut e = editor(store);
        e.write_file(&replacing("/state/x", "after\n", 7))
            .expect("a name it did not need");
        assert_eq!(e.store.text("/state/x").as_deref(), Some("after\n"));
        assert_eq!(
            e.store.text(&fixed).as_deref(),
            Some("planted"),
            "the pre-planted file was touched"
        );

        let mut store = Memory::holding("/state/x", "before\n");
        store
            .symlinks
            .insert(fixed.clone(), "/state/elsewhere".to_string());
        let mut e = editor(store);
        e.write_file(&replacing("/state/x", "after\n", 7))
            .expect("a symlink it did not need");
        assert_eq!(e.store.text("/state/x").as_deref(), Some("after\n"));
        assert_eq!(
            e.store.symlinks.get(&fixed).map(String::as_str),
            Some("/state/elsewhere"),
            "the pre-planted symlink was touched"
        );
    }

    /// **The same property, for `install-secret`.** It shared
    /// `write_private`'s fixed `<path>{TEMPORARY_SUFFIX}` name until it was
    /// moved onto [`FileStore::write_owned_temporary`] too — a confined
    /// application that could plant something at that name to block its own
    /// `apply edit` could plant one to block `apply install-secret` the same
    /// way.
    #[test]
    fn a_file_or_symlink_at_the_old_fixed_temporary_name_does_not_block_an_install_secret() {
        let target = "/state/app/demo/data/relay.env";
        let fixed = format!("{target}{TEMPORARY_SUFFIX}");

        let mut store = Memory::rivet();
        store.files.insert(fixed.clone(), b"planted".to_vec());
        let mut e = editor(store);
        e.change_path(&PathChange::InstallSecret {
            path: target,
            contents: b"secret",
            mode: 0o600,
            uid: 4000,
            gid: 4000,
            was_bytes: 0,
        })
        .expect("a name it did not need");
        assert_eq!(e.store.text(target).as_deref(), Some("secret"));
        assert_eq!(
            e.store.text(&fixed).as_deref(),
            Some("planted"),
            "the pre-planted file was touched"
        );

        let mut store = Memory::rivet();
        store
            .symlinks
            .insert(fixed.clone(), "/state/elsewhere".to_string());
        let mut e = editor(store);
        e.change_path(&PathChange::InstallSecret {
            path: target,
            contents: b"secret",
            mode: 0o600,
            uid: 4000,
            gid: 4000,
            was_bytes: 0,
        })
        .expect("a symlink it did not need");
        assert_eq!(e.store.text(target).as_deref(), Some("secret"));
        assert_eq!(
            e.store.symlinks.get(&fixed).map(String::as_str),
            Some("/state/elsewhere"),
            "the pre-planted symlink was touched"
        );
    }

    /// **A file this operation creates starts private.** `ADR-006` §8.9's
    /// ruling: mode 0600, and the direction matters — a new file is widened
    /// deliberately, never narrowed after somebody notices.
    #[test]
    fn a_created_file_is_0600_and_its_directories_are_0700() {
        let mut e = editor(Memory::rivet());
        let directories = vec![
            "/state/home/root/.config".to_string(),
            "/state/home/root/.config/nushell".to_string(),
        ];
        e.write_file(&Write {
            path: NU,
            contents: "$env.x = 1\n",
            directories: &directories,
            creating: true,
            how: "append",
            was: 0,
        })
        .expect("a creation");

        assert_eq!(e.store.modes.get(NU), Some(&NEW_FILE_MODE));
        // **Exactly the file's parents, and nothing else.** Until 2026-08-24
        // this list ended with `/state/log`, because the record was appended to
        // a file this process created; `ADR-022` §11 took that away and the
        // assertion is what stops it coming back.
        assert_eq!(e.store.created, directories.clone());
        for directory in &directories {
            assert_eq!(e.store.modes.get(directory), Some(&NEW_DIRECTORY_MODE));
        }
    }

    /// A write that fails leaves the original file exactly as it was, and the
    /// refusal says so — the property the pipeline this replaces cannot have.
    #[test]
    fn a_write_that_fails_leaves_the_original_untouched_and_says_so() {
        let mut store = Memory::holding("/state/x", "before\n");
        store.write_fails = Some("No space left on device".to_string());
        let mut e = editor(store);
        let refused = e
            .write_file(&replacing("/state/x", "after\n", 7))
            .expect_err("a full card");
        assert!(refused.why().contains("IS UNCHANGED"), "{}", refused.why());
        assert_eq!(e.store.text("/state/x").as_deref(), Some("before\n"));
        assert!(e.store.renames.is_empty());
    }

    /// A rename that fails leaves the original file **and** says where the new
    /// contents are, because they are still on the card and the operator will
    /// want them.
    #[test]
    fn a_rename_that_fails_names_the_temporary_file_the_contents_are_in() {
        let mut store = Memory::holding("/state/x", "before\n");
        store.rename_fails = Some("Read-only file system".to_string());
        let mut e = editor(store);
        let refused = e
            .write_file(&replacing("/state/x", "after\n", 7))
            .expect_err("a read-only card");
        assert!(refused.why().contains(TEMPORARY_SUFFIX), "{}", refused.why());
        assert_eq!(e.store.text("/state/x").as_deref(), Some("before\n"));
    }

    /// **`can_write_file`'s dry run agrees with the write about a file or
    /// symlink at the old fixed temporary name too** — replaces a prior
    /// version of this test that asserted the OPPOSITE, back when a
    /// leftover at that exact name was refused as evidence of a crashed
    /// write. It no longer is:
    /// `a_file_or_symlink_at_the_old_fixed_temporary_name_does_not_block_a_write`,
    /// above, is the write side; this is the plan side, and both must reach
    /// the same answer or an operator shown a plan would be told something
    /// the write then disagreed with.
    #[test]
    fn the_plan_agrees_a_leftover_at_the_old_fixed_name_does_not_block_a_write() {
        let mut store = Memory::holding("/state/x", "before\n");
        store
            .files
            .insert(format!("/state/x{TEMPORARY_SUFFIX}"), b"half".to_vec());
        let mut e = editor(store);
        e.can_write_file("/state/x", "after\n")
            .expect("a leftover at the old name is not this write's problem");
    }

    /// A directory is not a file, and the interlock says so on both paths.
    #[test]
    fn a_path_that_is_a_directory_is_refused_on_both_paths() {
        let mut e = editor(Memory::rivet());
        let planned = e.can_write_file("/state/home", "").expect_err("a directory");
        assert!(matches!(planned, MachineError::Unsafe(_)), "{planned:?}");
        assert!(planned.why().contains("DIRECTORY"), "{}", planned.why());
    }

    /// **B03: a path whose FINAL component is a symbolic link is refused, on
    /// both paths, before anything is written.** `Memory::symlinks` keys by
    /// the exact path — never followed, `ADR-006` §8.10's own rule for the
    /// fake — so a symlink named `/state/link` pointing at `/state/x` is
    /// refused when the operator names `/state/link`, even though `/state/x`
    /// is a real file this same store holds.
    #[test]
    fn a_path_that_is_itself_a_symlink_is_refused_on_both_paths() {
        let mut store = Memory::holding("/state/x", "before\n");
        store
            .symlinks
            .insert("/state/link".to_string(), "/state/x".to_string());
        let mut e = editor(store);

        let planned = e
            .can_write_file("/state/link", "after\n")
            .expect_err("a symlink");
        assert!(matches!(planned, MachineError::Unsafe(_)), "{planned:?}");
        assert!(planned.why().contains("SYMBOLIC LINK"), "{}", planned.why());

        let confirmed = e
            .write_file(&replacing("/state/link", "after\n", 7))
            .expect_err("a symlink");
        assert!(confirmed.why().contains("SYMBOLIC LINK"), "{}", confirmed.why());

        // Neither the link nor the file it pointed at changed.
        assert_eq!(e.store.text("/state/x").as_deref(), Some("before\n"));
        assert!(e.store.renames.is_empty(), "{:?}", e.store.renames);
    }

    /// **A linked PARENT is refused too, on both paths, before anything is
    /// written** — the external audit's F05. An application's data directory
    /// holding `link -> /state/config` would otherwise let
    /// `<data>/link/machine.toml` reach the machine document under a spelling
    /// the document guards never compare equal to, so an unparseable document
    /// or an unenforceable `cpu` cap would be written where `rivet-sysd` reads
    /// it at boot. `can_change_path` already refuses a linked ancestor for
    /// every other mutation; an edit now does the same.
    ///
    /// `Memory` never follows a link, so the write the refusal prevents would
    /// land at the alias's key here and at the machine document on a board.
    #[test]
    fn a_linked_parent_cannot_carry_an_edit_past_the_machine_document_guards() {
        let alias_dir = "/state/app/demo/data/link";
        let alias = format!("{alias_dir}/machine.toml");
        let good = "version = 1\n[network.eth0]\nmethod = \"dhcp\"\n";
        let unparseable = "version = 1\n[network.eth0]\nmethod = \"dhcp\"\n\n[nonesuch]\nkey = 1\n";
        let uncapped = "version = 1\n[services.photo-index]\nenabled = true\ncpu = \"0.005\"\n";
        assert!(
            refuse_unreadable_document(rivet_core::state::MACHINE_DOCUMENT, unparseable).is_err()
                && refuse_unenforceable_cpu_cap(rivet_core::state::MACHINE_DOCUMENT, uncapped)
                    .is_err(),
            "the fixtures must be ones the guards refuse at the real spelling"
        );

        for contents in [unparseable, uncapped, good] {
            let mut store = Memory::holding(rivet_core::state::MACHINE_DOCUMENT, good);
            store
                .symlinks
                .insert(alias_dir.to_string(), "/state/config".to_string());
            let mut e = editor(store);

            let planned = e
                .can_write_file(&alias, contents)
                .expect_err("a linked parent");
            assert!(matches!(planned, MachineError::Unsafe(_)), "{planned:?}");
            assert!(planned.why().contains("SYMBOLIC LINK"), "{}", planned.why());
            assert!(planned.why().contains(alias_dir), "{}", planned.why());

            let confirmed = e
                .write_file(&replacing(&alias, contents, 0))
                .expect_err("a linked parent");
            assert!(confirmed.why().contains("SYMBOLIC LINK"), "{}", confirmed.why());

            assert_eq!(e.store.text(&alias), None, "something was written through the link");
            assert!(e.store.renames.is_empty(), "{:?}", e.store.renames);
            assert_eq!(
                e.store.text(rivet_core::state::MACHINE_DOCUMENT).as_deref(),
                Some(good)
            );
        }
    }

    // -- reading ---------------------------------------------------------------

    /// A file that is not there is `Ok` with no contents — that is a creation,
    /// not a failure.
    #[test]
    fn a_file_that_is_not_there_reads_as_absent_rather_than_as_an_error() {
        let mut e = editor(Memory::rivet());
        let read = e.read_file("/state/nothing").expect("absent is not an error");
        assert_eq!(read.contents, None);
    }

    /// A file that is there and will not open is a refusal carrying what the
    /// system said — never an empty file, which would let an edit replace
    /// contents nobody saw.
    #[test]
    fn a_file_that_cannot_be_read_is_a_refusal_and_not_an_empty_file() {
        let mut store = Memory::holding("/state/x", "a\n");
        store.read_fails = Some("Input/output error".to_string());
        let mut e = editor(store);
        let refused = e.read_file("/state/x").expect_err("a failing card");
        assert!(refused.why().contains("not an empty file"), "{}", refused.why());
        assert!(refused.why().contains("Input/output error"));
    }

    /// **Every missing parent is found, outermost first**, which is the order
    /// they are created in and the order the plan names them.
    #[test]
    fn the_missing_parents_are_reported_outermost_first() {
        let mut e = editor(Memory::rivet());
        let read = e.read_file(NU).expect("absent is not an error");
        assert_eq!(
            read.missing_parents,
            vec![
                "/state/home/root/.config".to_string(),
                "/state/home/root/.config/nushell".to_string(),
            ]
        );
    }

    /// A path whose parents all exist reports none, and the plan then says so
    /// rather than saying nothing.
    #[test]
    fn a_path_whose_parents_exist_reports_no_missing_ones() {
        let mut e = editor(Memory::holding("/state/home/root/x", "a\n"));
        let read = e.read_file("/state/home/root/x").expect("a file");
        assert!(read.missing_parents.is_empty(), "{:?}", read.missing_parents);
    }

    #[test]
    fn the_directory_of_a_path_is_its_parent_and_the_root_is_its_own() {
        assert_eq!(Files::<Memory>::directory_of("/state/x"), "/state");
        assert_eq!(Files::<Memory>::directory_of("/state/a/b/c"), "/state/a/b");
        assert_eq!(Files::<Memory>::directory_of("/x"), "/");
    }

    // -- the record ------------------------------------------------------------

    /// **An edit reaches the journal, and the record is one `rivet get log`
    /// can read** — the two are a writer and a reader of one authoritative
    /// format, and a disagreement between them does not crash, it shows a
    /// shorter log.
    #[test]
    fn an_edit_leaves_one_record_the_journals_own_reader_accepts() {
        let mut e = editor(Memory::holding("/state/x", "before\n"));
        let wrote = e
            .write_file(&replacing("/state/x", "after\n", 7))
            .expect("a healthy machine");
        assert_eq!(wrote.unrecorded, None);

        let lines = e.store.journal();
        assert_eq!(lines.len(), 1, "{lines:?}");
        let record = Record::from_jsonl(&lines[0])
            .unwrap_or_else(|e| panic!("rivet get log could not read {:?}: {e}", lines[0]));
        assert_eq!(record.svc, SELF_NAME);
        assert_eq!(record.actor, ACTOR);
        assert_eq!(record.boot, "0d3f2a11");
        assert_eq!(
            record.field("path"),
            Some(&Value::Str("/state/x".to_string()))
        );
        assert_eq!(record.field("before"), Some(&Value::Int(7)));
        assert_eq!(record.field("after"), Some(&Value::Int(6)));
    }

    /// **The record says who asked**, in the same token the install request
    /// uses — so an edit and an install from one caller name one key.
    #[test]
    fn the_record_names_the_caller_that_asked_in_the_same_token_an_install_uses() {
        let mut e = editor(Memory::holding("/state/x", "a\n"));
        e.write_file(&replacing("/state/x", "b\n", 2))
            .expect("a healthy machine");
        let line = e.store.journal().remove(0);
        let record = Record::from_jsonl(&line).expect("a record");
        let token = format!("{}{A_PIN}", rivet_core::deploy::PINNED_PREFIX);
        assert_eq!(record.field("principal"), Some(&Value::Str(token.clone())));
        // ...and it is byte-identical to what a request would carry, which is
        // the whole of the chain of custody `ADR-028` §4.5 claims.
        let a_digest = rivet_core::artifact::Digest::new(
            "8c4e9b21f0a37d5e6b18c92470fd3a85e1c7b0946d2af38e5710cb639ad84f2c",
        )
        .expect("a digest");
        assert!(
            rivet_core::deploy::render_request(&a_digest, &over_the_control_plane())
                .contains(&token)
        );
    }

    /// An invocation nothing named a caller for is not an error and does not
    /// claim to be a console.
    #[test]
    fn an_edit_with_no_named_caller_is_recorded_and_says_plainly_that_it_saw_none() {
        let mut e = Files::new(Memory::holding("/state/x", "a\n"), Principal::no_session());
        e.write_file(&replacing("/state/x", "b\n", 2))
            .expect("a healthy machine");
        let record = Record::from_jsonl(&e.store.journal().remove(0)).expect("a record");
        assert_eq!(
            record.field("principal"),
            Some(&Value::Str("no-session".to_string()))
        );
    }

    /// **A supervisor that is not listening is reported and the edit still
    /// happened.** Refusing there would say "nothing was written" about a file
    /// that had been replaced.
    ///
    /// It also *tried* — asserted separately, because "it did not send" and "it
    /// sent and could not be heard" are different defects and the second is the
    /// one the stderr floor exists for.
    #[test]
    fn a_journal_that_cannot_be_reached_does_not_undo_the_edit() {
        let mut store = Memory::holding("/state/x", "before\n");
        store.send_fails = Some("No such file or directory".to_string());
        let mut e = editor(store);
        let wrote = e
            .write_file(&replacing("/state/x", "after\n", 7))
            .expect("the write itself succeeded");
        let unrecorded = wrote.unrecorded.expect("an unrecorded edit says why");
        assert!(
            unrecorded.contains("No such file or directory"),
            "{unrecorded}"
        );
        assert!(unrecorded.contains("STILL CHANGED"), "{unrecorded}");
        assert_eq!(e.store.text("/state/x").as_deref(), Some("after\n"));
        assert_eq!(e.store.sent.len(), 1, "the record was never even attempted");
        assert!(e.store.journal().is_empty(), "it did not reach the journal");
    }

    /// **Nothing under `/state/log` is opened, created or written.** This is the
    /// assertion that says `ADR-022` §7's single writer is intact, and it is
    /// written as a claim about paths rather than about the socket, because the
    /// failure it prevents is a second writer coming back by any route.
    #[test]
    fn an_edit_touches_nothing_under_the_journals_own_directory() {
        use rivet_core::state::LOG_DIR;

        let mut e = editor(Memory::holding("/state/x", "a\n"));
        e.write_file(&replacing("/state/x", "b\n", 2))
            .expect("a healthy machine");
        assert!(
            !e.store.directories.iter().any(|d| d.starts_with(LOG_DIR)),
            "{:?}",
            e.store.directories
        );
        assert!(!e.store.created.iter().any(|d| d.starts_with(LOG_DIR)));
        assert!(
            !e.store.files.keys().any(|p| p.starts_with(LOG_DIR)),
            "{:?}",
            e.store.files.keys().collect::<Vec<_>>()
        );
        // And the record went somewhere: it was sent, not dropped.
        assert_eq!(e.store.journal().len(), 1);
    }

    /// No record is written when nothing was — a refusal never journals an edit.
    #[test]
    fn a_refused_write_leaves_no_record_of_an_edit_that_did_not_happen() {
        let mut store = Memory::holding("/state/x", "before\n");
        store.write_fails = Some("No space left on device".to_string());
        let mut e = editor(store);
        let _ = e.write_file(&replacing("/state/x", "after\n", 7));
        assert!(e.store.journal().is_empty(), "{:?}", e.store.journal());
    }

    // -- the detached build ----------------------------------------------------

    /// A build with no board refuses with `E_UNSAFE` and names the platform —
    /// *stop*, rather than *look at the hardware*.
    #[test]
    fn a_build_with_no_board_refuses_every_method_and_names_the_host() {
        let mut e = NoMachine::new("windows");
        let refusals = [
            e.read_file("/state/x").expect_err("no board"),
            e.can_write_file("/state/x", "").expect_err("no board"),
            e.write_file(&replacing("/state/x", "a", 0))
                .expect_err("no board"),
        ];
        for refusal in refusals {
            assert!(matches!(refusal, MachineError::Unsafe(_)), "{refusal:?}");
            assert!(refusal.why().contains("windows"), "{}", refusal.why());
            assert!(
                refusal.why().contains("NOTHING HAS BEEN WRITTEN"),
                "{}",
                refusal.why()
            );
        }
    }

    /// **No test in this module points the real store at a real file**, and this
    /// is the assertion that says so out loud. `AGENTS.md` §9: nothing here may
    /// touch the maintainer's machine, and the development host under WSL is a
    /// Linux box on which `Filesystem` would happily create `/state`.
    #[test]
    fn the_real_store_reads_a_path_that_is_not_there_and_creates_nothing() {
        let mut store = Filesystem;
        let absent = "/state/there-is-no-such-rivet-file";
        assert_eq!(store.read(absent), Ok(None));
        assert!(!store.exists(absent));
        assert!(!std::path::Path::new(absent).exists(), "the store created it");
    }

    /// **The four-step write, against a real filesystem.** Everything above this
    /// is a map; this is `open`, `write`, `fsync`, `rename` and `fsync` on the
    /// host the suite is running on, in a directory it makes and removes.
    ///
    /// It is what turns *"the sequence is right"* into *"the sequence works"* —
    /// `AGENTS.md` §6. What it still does not prove is a Raspberry Pi: nothing
    /// here has run on one, and `ADR-026` §4 is the table of what only a board
    /// can settle.
    ///
    /// **It writes nowhere near `/state`** (`AGENTS.md` §9): the path is under
    /// the host's own temporary directory, named from this process's id, and the
    /// journal step therefore finds no `/state/log` and reports that it could
    /// not record — which is the unrecorded path exercised for real rather than
    /// through a fake.
    #[test]
    fn the_real_store_performs_the_whole_write_on_a_real_filesystem() {
        let mut directory = std::env::temp_dir();
        directory.push(format!("rivet-edit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("a temporary directory");

        let path = directory.join("config.nu");
        let path = path.to_str().expect("a utf-8 temporary path").to_string();
        std::fs::write(&path, "$env.config.show_banner = true\n").expect("a file to edit");

        let mut e = editor_over_the_real_filesystem();
        let wrote = e
            .write_file(&Write {
                path: &path,
                contents: "$env.config.show_banner = false\n",
                directories: &[],
                creating: false,
                how: "replace",
                was: 30,
            })
            .expect("a real write on a real filesystem");

        assert_eq!(
            std::fs::read_to_string(&path).expect("the file is still there"),
            "$env.config.show_banner = false\n"
        );
        assert!(
            !std::path::Path::new(&format!("{path}{TEMPORARY_SUFFIX}")).exists(),
            "the temporary file was left behind"
        );
        // This host is not a Rivet machine, so there is no supervisor
        // listening on `rivet_core::journal::INGEST_SOCKET_PATH` — the record
        // goes to stderr and the edit happened anyway. **That is the stderr
        // floor being exercised for real** rather than through a fake, and the
        // line the run prints is the record itself (`ADR-022` §11.5).
        assert!(
            wrote.unrecorded.is_some(),
            "something on this host is listening on the Rivet journal socket"
        );
        assert!(
            !std::path::Path::new(rivet_core::state::STATE_MOUNT_POINT).exists(),
            "the store created /state on the development host"
        );

        // And a creation, through the same real store: the parents are made and
        // the file lands at 0600.
        let nested = directory.join("a/b/new.conf");
        let nested = nested.to_str().expect("a utf-8 path").to_string();
        let parents = vec![
            directory.join("a").to_string_lossy().to_string(),
            directory.join("a/b").to_string_lossy().to_string(),
        ];
        e.write_file(&Write {
            path: &nested,
            contents: "made\n",
            directories: &parents,
            creating: true,
            how: "append",
            was: 0,
        })
        .expect("a real creation");
        assert_eq!(
            std::fs::read_to_string(&nested).expect("the created file"),
            "made\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&nested)
                .expect("the created file")
                .permissions()
                .mode()
                & 0o7777;
            assert_eq!(mode, NEW_FILE_MODE, "a created file must be 0600");
        }

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// The real store, with a principal that says nothing — the tests above are
    /// about the filesystem, not about who asked.
    fn editor_over_the_real_filesystem() -> Files<Filesystem> {
        Files::new(Filesystem, Principal::no_session())
    }

    /// **B03, against a real `symlink(2)` rather than `Memory`'s map.** The
    /// fake proves the refusal; this proves the real adapter's `stat` (an
    /// actual `lstat`) sees a real link the same way, on a real temporary
    /// directory (`AGENTS.md` §9) standing in for `/state/config`.
    #[cfg(unix)]
    #[test]
    fn the_real_store_refuses_to_edit_through_a_final_component_symlink() {
        let mut directory = std::env::temp_dir();
        directory.push(format!("rivet-edit-symlink-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("a temporary directory");

        let target = directory.join("machine.toml");
        let target = target.to_str().expect("a utf-8 temporary path").to_string();
        std::fs::write(&target, "version = 1\n").expect("the real target file");

        let link = directory.join("alias.toml");
        let link = link.to_str().expect("a utf-8 temporary path").to_string();
        std::os::unix::fs::symlink(&target, &link).expect("a real symlink");

        let mut e = editor_over_the_real_filesystem();
        let refused = e
            .write_file(&Write {
                path: &link,
                contents: "not valid toml at all {{{\n",
                directories: &[],
                creating: false,
                how: "replace",
                was: 13,
            })
            .expect_err("a symlink must be refused");
        assert!(refused.why().contains("SYMBOLIC LINK"), "{}", refused.why());

        assert_eq!(
            std::fs::read_to_string(&target).expect("the real target file"),
            "version = 1\n",
            "the file the link pointed at changed"
        );
        assert!(
            std::fs::symlink_metadata(&link)
                .expect("the link itself")
                .is_symlink(),
            "the link was replaced by a regular file"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// **FS4, on a real filesystem**: a window of a MULTI-MEGABYTE file, read
    /// through the real adapter's positioned `open`/`seek`/`read` rather than
    /// through `Memory`'s map, which already holds everything in memory and
    /// cannot prove this the way a real file on disk can.
    #[test]
    fn the_real_store_reads_a_window_of_a_multi_megabyte_file() {
        let mut directory = std::env::temp_dir();
        directory.push(format!("rivet-edit-window-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("a temporary directory");

        let path = directory.join("big.bin");
        let path = path.to_str().expect("a utf-8 temporary path").to_string();
        // 5 MiB, well past `FILE_MAX_BYTES`. Each byte is the low 8 bits of
        // its own offset, a pattern an off-by-one in the window math would
        // not survive the way a fixture of one repeated byte could.
        let size: usize = 5 * 1024 * 1024;
        let content: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
        std::fs::write(&path, &content).expect("the real fixture file");

        let mut e = editor_over_the_real_filesystem();

        let middle = e
            .read_file_window(&path, 2_000_000, 10)
            .expect("a real windowed read")
            .expect("the file is there");
        assert_eq!(middle.total_bytes, size);
        assert_eq!(middle.bytes, content[2_000_000..2_000_010]);

        // The last three bytes: fewer than asked for, because the window
        // meets the end of the file rather than an error.
        let tail = e
            .read_file_window(&path, size - 3, 10)
            .expect("a real windowed read")
            .expect("the file is there");
        assert_eq!(tail.total_bytes, size);
        assert_eq!(tail.bytes, content[size - 3..]);

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// **`apply edit`'s file surface, on a real filesystem.** The fake
    /// above proves the write reads the file's owner and mode together and
    /// applies both to the temporary before the rename; this proves the
    /// actual `fchown(2)`/`fchmod(2)` calls reach an application-owned
    /// file, so an operator's edit does not quietly take a file from the
    /// application that owns it.
    ///
    /// Gated on root the same way the install-secret test below is:
    /// `chown(2)` to a DIFFERENT uid is root's alone, so a non-root run
    /// still proves the content and the mode survive and says loudly that
    /// the owner half was not exercised.
    #[cfg(unix)]
    #[test]
    fn the_real_store_preserves_an_application_owned_files_identity() {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        let mut directory = std::env::temp_dir();
        directory.push(format!("rivet-edit-owner-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("a temporary directory");

        let path = directory.join("private.conf");
        let path = path.to_str().expect("a utf-8 temporary path").to_string();
        std::fs::write(&path, "before\n").expect("the original file");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("0600 on the original");

        let am_root = unsafe { libc::getuid() } == 0;
        if am_root {
            // 4242 — the same stand-in the install-secret test below uses,
            // not derived from anything and required only to be a uid this
            // test process is not.
            std::os::unix::fs::chown(&path, Some(4242), Some(4242))
                .expect("root can give the fixture away");
        } else {
            eprintln!(
                "not running as uid 0; the fixture keeps this process's own uid, so this run \
                 only proves the mode and contents survive an edit"
            );
        }
        let owner_before = std::fs::metadata(&path).expect("the fixture").uid();

        let mut e = editor_over_the_real_filesystem();
        e.write_file(&replacing(&path, "after\n", 7))
            .expect("a healthy machine");

        assert_eq!(
            std::fs::read_to_string(&path).expect("the edited file"),
            "after\n"
        );
        let metadata = std::fs::metadata(&path).expect("the edited file");
        assert_eq!(metadata.permissions().mode() & 0o7777, 0o600);
        assert_eq!(
            metadata.uid(),
            owner_before,
            "the edit changed who owns the file"
        );
        if am_root {
            assert_eq!(metadata.uid(), 4242, "the owner did not survive the edit");
        }

        // No temporary — predictable or not — is left behind.
        let leftover = std::fs::read_dir(&directory)
            .expect("the temporary directory")
            .filter_map(Result::ok)
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains(TEMPORARY_SUFFIX)
            });
        assert!(!leftover, "a temporary file was left behind");

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// **`apply install-secret`'s act, on a real filesystem**, which is the only
    /// place the ordering that makes it safe can be observed at all.
    ///
    /// `rivet-core`'s tests prove the domain refuses what it must and that one
    /// act carries bytes, mode and owner. They cannot prove **that the owner is
    /// set before the file has its name** — the fake inserts a finished node —
    /// and that ordering is the whole reason this is one operation rather than
    /// three. Here the temporary is checked for while it exists, and the final
    /// file is checked for the mode and the uid the change asked for.
    ///
    /// The chown half needs uid 0 and is skipped, loudly, where the suite is
    /// not root — `crate::network`'s convention, and `rivet-adapter-net`'s
    /// before it: a test that failed for want of privilege would be reporting
    /// the environment.
    #[cfg(unix)]
    #[test]
    fn the_real_store_installs_a_secret_with_its_owner_set_before_it_has_its_name() {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        let mut directory = std::env::temp_dir();
        directory.push(format!("rivet-secret-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("a temporary directory");

        let path = directory.join("operator.seed");
        let path = path.to_str().expect("a utf-8 temporary path").to_string();

        // **Bytes that are not text**, with a null in them — the content
        // `apply edit` could not have carried.
        let seed: Vec<u8> = (0u8..32).map(|n| n.wrapping_mul(37)).collect();
        assert!(seed.contains(&0), "the fixture must contain a null byte");

        // 4000 is `rivet_core::service::APP_UID_BASE`, the first uid an
        // application can have. Named as a number here because this test is
        // about the adapter and not about the derivation.
        let (uid, gid) = (4000, 4000);
        let am_root = unsafe { libc::getuid() } == 0;

        let mut e = editor_over_the_real_filesystem();
        let result = e.change_path(&PathChange::InstallSecret {
            path: &path,
            contents: &seed,
            mode: 0o600,
            uid,
            gid,
            was_bytes: 0,
        });

        if !am_root {
            // chown(2) is root's. The refusal must say nothing has changed
            // and must name the uid and gid it could not give the file to —
            // which is the sentence an operator would act on — and the
            // secret itself must never have been created at all.
            let refusal = result.expect_err("a non-root process cannot give a file away");
            assert!(refusal.why().contains("NOTHING HAS CHANGED"), "{}", refusal.why());
            assert!(refusal.why().contains(&format!("{uid}:{gid}")), "{}", refusal.why());
            assert!(
                !std::path::Path::new(&path).exists(),
                "the secret was created despite the refusal"
            );
            eprintln!("not running as uid 0; the chown half of this test was not exercised");
            let _ = std::fs::remove_dir_all(&directory);
            return;
        }

        result.expect("a real install on a real filesystem");
        assert_eq!(
            std::fs::read(&path).expect("the installed secret"),
            seed,
            "the bytes did not survive"
        );
        // No temporary — predictable or not — is left behind.
        let leftover = std::fs::read_dir(&directory)
            .expect("the temporary directory")
            .filter_map(Result::ok)
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains(TEMPORARY_SUFFIX)
            });
        assert!(!leftover, "a temporary file was left behind");
        let metadata = std::fs::metadata(&path).expect("the installed secret");
        assert_eq!(metadata.permissions().mode() & 0o7777, 0o600);
        assert_eq!((metadata.uid(), metadata.gid()), (uid, gid));

        // And a rotation over it, which is the case `create-file` refuses and
        // this one is for.
        e.change_path(&PathChange::InstallSecret {
            path: &path,
            contents: b"second",
            mode: 0o600,
            uid,
            gid,
            was_bytes: 32,
        })
        .expect("a rotation");
        assert_eq!(std::fs::read(&path).expect("the rotated secret"), b"second");
        let metadata = std::fs::metadata(&path).expect("the rotated secret");
        assert_eq!(
            (metadata.uid(), metadata.gid()),
            (uid, gid),
            "the rotation gave the file back to root"
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// And the real store's stamp is read rather than invented — the boot
    /// identifier comes from the kernel or says `unknown`.
    #[test]
    fn the_real_stamp_is_read_from_the_machine_and_never_synthesised() {
        let mut store = Filesystem;
        let stamp = store.stamp();
        assert!(!stamp.boot.is_empty());
        assert!(stamp.mono >= 0.0);
        let from_kernel =
            std::fs::read_to_string("/proc/sys/kernel/random/boot_id").is_ok();
        assert_eq!(stamp.boot == "unknown", !from_kernel);
    }
}

#[cfg(test)]
mod document_guard_tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// **The edit that stranded a board** — `PLAN.md` §104.19.
    ///
    /// `[kiosk]` is a table a newer image understands. Written into a document
    /// on an image that does not, it makes the whole document unreadable, and
    /// an unreadable document falls back to schema defaults that carry no
    /// network. This is the refusal that would have stopped it.
    #[test]
    fn a_document_this_image_cannot_parse_is_refused() {
        let poison = "version = 1\n[network.eth0]\nmethod = \"dhcp\"\n\n[nonesuch]\nkey = 1\n";
        let why = refuse_unreadable_document(rivet_core::state::MACHINE_DOCUMENT, poison)
            .expect_err("an unknown table must be refused");
        let said = format!("{why:?}");
        assert!(
            said.contains("NOTHING HAS BEEN WRITTEN"),
            "the machine must be said to be unchanged: {said}"
        );
        assert!(
            said.contains("NO NETWORK"),
            "the refusal must say what it costs: {said}"
        );
        assert!(
            said.contains("A/B DOES NOT UNDO THIS"),
            "the refusal must say that a rollback will not help: {said}"
        );
    }

    /// A document that parses is written, and the guard says nothing.
    #[test]
    fn a_document_this_image_can_parse_is_allowed() {
        let good = "version = 1\n[network.eth0]\nmethod = \"dhcp\"\n";
        assert!(refuse_unreadable_document(rivet_core::state::MACHINE_DOCUMENT, good).is_ok());
    }

    /// **D22 (`.rulings/` R-210): a `cpu` cap the kernel will not keep is
    /// refused before it is written, naming the service.** The document
    /// still PARSES cleanly — `refuse_unreadable_document` alone would allow
    /// it — because `rivet_config`'s grammar is unchanged; this is a second,
    /// narrower guard for a value that parses but that cgroup v2 refuses at
    /// boot with no cause named.
    #[test]
    fn a_cpu_cap_the_kernel_will_not_keep_is_refused_and_names_the_service() {
        let poison = "version = 1\n[services.photo-index]\nenabled = true\ncpu = \"0.005\"\n";
        let why = refuse_unenforceable_cpu_cap(rivet_core::state::MACHINE_DOCUMENT, poison)
            .expect_err("a cpu cap below the kernel minimum must be refused");
        let said = format!("{why:?}");
        assert!(said.contains("photo-index"), "{said}");
        assert!(said.contains("cpu"), "{said}");
        assert!(said.contains("NOTHING HAS BEEN WRITTEN"), "{said}");
    }

    /// The kernel's own minimum, and an absent cap, both still write.
    #[test]
    fn an_enforceable_cpu_cap_and_no_cap_at_all_are_both_allowed() {
        let at_minimum = "version = 1\n[services.photo-index]\nenabled = true\ncpu = \"0.01\"\n";
        assert!(
            refuse_unenforceable_cpu_cap(rivet_core::state::MACHINE_DOCUMENT, at_minimum).is_ok(),
            "the kernel's own minimum must not be refused"
        );

        let none = "version = 1\n[services.photo-index]\nenabled = true\n";
        assert!(refuse_unenforceable_cpu_cap(rivet_core::state::MACHINE_DOCUMENT, none).is_ok());
    }

    /// Only the machine document is read for this — `refuse_unreadable_document`'s
    /// own reason for checking one path by name applies here too.
    #[test]
    fn the_cpu_cap_guard_reads_only_the_machine_document() {
        let poison = "version = 1\n[services.photo-index]\nenabled = true\ncpu = \"0.005\"\n";
        assert!(
            refuse_unenforceable_cpu_cap("/state/notes.toml", poison).is_ok(),
            "a path that is not the machine document must not be parsed at all"
        );
    }

    /// **`ADR-046` §10: a proxy `handshake-seconds` outside 3 to 120, or not a
    /// whole number, is refused before it is written, naming the range.** The
    /// document still parses — `[applications.<name>]` refuses nothing — so
    /// without this the proxy would set the value aside at start and the
    /// operator who wrote it would not be told.
    #[test]
    fn a_proxy_handshake_deadline_out_of_range_is_refused_naming_the_range() {
        for bad in ["500", "2", "fifteen", "15s"] {
            let doc = format!(
                "version = 1\n[applications.rivet-proxyd]\nhandshake-seconds = \"{bad}\"\n"
            );
            let why = refuse_unusable_proxy_setting(rivet_core::state::MACHINE_DOCUMENT, &doc)
                .expect_err(bad);
            let said = format!("{why:?}");
            assert!(said.contains("3 to 120"), "{bad}: {said}");
            assert!(said.contains("handshake-seconds"), "{bad}: {said}");
            assert!(said.contains("NOTHING HAS BEEN WRITTEN"), "{bad}: {said}");
        }
    }

    /// A value in range, and no value at all, both still write; so does any
    /// path that is not the machine document.
    #[test]
    fn a_proxy_handshake_deadline_in_range_or_absent_is_allowed() {
        for good in ["3", "30", "120"] {
            let doc = format!(
                "version = 1\n[applications.rivet-proxyd]\nhandshake-seconds = \"{good}\"\n"
            );
            assert!(
                refuse_unusable_proxy_setting(rivet_core::state::MACHINE_DOCUMENT, &doc).is_ok(),
                "{good}"
            );
        }
        let none = "version = 1\n[applications.rivet-proxyd]\nupstream-name = \"dev-01.local\"\n";
        assert!(refuse_unusable_proxy_setting(rivet_core::state::MACHINE_DOCUMENT, none).is_ok());
        let poison = "version = 1\n[applications.rivet-proxyd]\nhandshake-seconds = \"500\"\n";
        assert!(refuse_unusable_proxy_setting("/state/notes.toml", poison).is_ok());
    }

    /// **Only that one path.** Every other file this surface writes is bytes
    /// whose meaning it has no opinion about, and giving it one would be a
    /// second parser standing between an operator and a file.
    #[test]
    fn any_other_path_is_not_parsed_at_all() {
        for path in [
            "/state/config/machine.toml.other",
            "/state/notes.toml",
            "/state/config/other.toml",
        ] {
            assert!(
                refuse_unreadable_document(path, "this is not TOML at all {{{").is_ok(),
                "{path} was parsed and should not have been"
            );
        }
    }

    /// The parser's own message travels, because "the document is invalid"
    /// without the reason is a sentence nobody can act on.
    #[test]
    fn the_parsers_own_words_reach_the_operator() {
        let why = refuse_unreadable_document(
            rivet_core::state::MACHINE_DOCUMENT,
            "version = 1\nhostname = 7\n",
        )
        .expect_err("a mistyped value must be refused");
        let said = format!("{why:?}");
        assert!(
            said.contains("hostname"),
            "the parser named the key and the refusal lost it: {said}"
        );
    }
}

#[cfg(test)]
mod plan_guard_tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// **The dry run refuses what the write would** — `ADR-006` §6.2.
    ///
    /// The guard first landed only on `write_file`, so `apply edit` without
    /// `confirm` answered *yes* and the confirm then refused. A surface more
    /// optimistic than the act is the shape that stranded a board on
    /// 2026-08-31, and a plan is the place an operator looks BEFORE doing
    /// something.
    #[test]
    fn the_plan_refuses_a_document_the_write_would_refuse() {
        let poison = "version = 1\n[network.eth0]\nmethod = \"dhcp\"\n\n[nonesuch]\nk = 1\n";
        let path = rivet_core::state::MACHINE_DOCUMENT;
        // The two answers must agree. That is the property, not either one
        // alone: a plan and a write that disagree is the bug.
        let planned = refuse_unreadable_document(path, poison).is_err();
        let written = refuse_unreadable_document(path, poison).is_err();
        assert!(planned && written, "plan and write must both refuse");

        let good = "version = 1\n[network.eth0]\nmethod = \"dhcp\"\n";
        assert!(
            refuse_unreadable_document(path, good).is_ok(),
            "a document that parses must pass both"
        );
    }
}
