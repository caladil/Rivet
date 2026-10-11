//! Getting administrator for the one step of `cargo rivet write-image` that
//! needs it, and no further — `SPEC-05` §4.
//!
//! **`write-image` is `provision`'s old name for the same module** — renamed
//! 2026-09-13 (`docs/PLAN-Feature-Set-2026-09-12.md` §3.0, §5.2 item B;
//! `.rulings/` R-162, Wave 3b) when `provision` itself became the one-command
//! orchestrator that runs the interview and then calls this verb. Nothing
//! this module does changed.
//!
//! # Why this exists
//!
//! Raw whole-disk access on Windows requires administrator — measured
//! 2026-09-09, where an unelevated shell was refused even a *read* of
//! `\\.\PHYSICALDRIVE2` while volume handles like `\\.\E:` opened for read
//! and write without complaint. `cargo rivet write-image` itself runs
//! unelevated, on purpose: elevation is a property of a process token at
//! creation, a process cannot raise its own, and the interview — the menu,
//! the plan, the `[y/N]` — belongs unelevated, where the person is.
//!
//! So the interview spawns one short-lived elevated child that does exactly
//! the write and the verify, and nothing else. The child is `cargo-rivet`
//! itself, under `write-image --elevated-write`, launched from
//! [`std::env::current_exe`] — one binary to trust, and no second artefact
//! to keep in step or fall out of date with this one.
//!
//! # Handles do not cross the elevation boundary
//!
//! `ShellExecuteEx`'s `runas` verb — which is what PowerShell's
//! `Start-Process -Verb RunAs` shells out to — cannot hand a child the
//! parent's stdout or stdin. The child gets a fresh console, or none at all.
//! So progress and the result are **files**, not a pipe:
//! [`ElevatedTask`](crate::elevate::ElevatedTask)
//! names a progress file the parent polls and a result file the child writes
//! before it exits, both under the system temporary directory with
//! unpredictable names, and the parent removes all three when it is done.
//!
//! # This crate still has no `windows-sys` dependency
//!
//! Every Windows-specific operation here — launching the elevated child,
//! taking the disk offline and restoring it — is a PowerShell script built as
//! a string and shelled out to, the same posture [`crate::enumerate`] already
//! takes for CIM and for the identical reason: it keeps unsafe Win32 code out
//! of a tool whose whole job at this step is not to guess, and it means a
//! person can run the same command by hand to check this tool's answer.
//!
//! # The guard that cannot be clicked through
//!
//! **The elevated child does not trust its arguments.** `run_elevated_child`
//! re-enumerates in its own context with [`crate::enumerate::all`] and refuses
//! any device that is not removable, whatever the unelevated parent believed.
//! The parent could be wrong about which disk is which — a stale enumeration,
//! a card unplugged and a different one plugged in during the `[y/N]` pause —
//! and no amount of typing or clicking in the parent guards against that.
//! `AGENTS.md` §10.4's cost, stated plainly: the wrong path here is the
//! development host's own disk.
//!
//! # The task file cannot be rewritten out from under the run that wrote it
//!
//! `Start-Process -Verb RunAs -Wait` waits on an interactive UAC prompt —
//! seconds of human reaction time — before the elevated child ever opens the
//! task file. Any other process running as the same Windows user can rewrite
//! that file during the wait, with no elevation of its own needed, and
//! redirect the write to a different device or a different image.
//!
//! So the parent computes a digest of the task file's exact bytes
//! (`task_digest`) right after writing them, and hands that digest to the
//! elevated child on the SAME command line PowerShell was invoked with,
//! never inside the file itself. `read_task_verified` is the child's half:
//! it refuses before parsing a single field if what is on disk right now does
//! not hash to the digest it was given. A same-user attacker can still
//! rewrite the file, but by the time `Start-Process` has been invoked the
//! digest is fixed — rewriting the file can only make it fail this check, not
//! pass it with different content.
//!
//! **What `task_digest` binds, and what it does not**: only the task file.
//! The image file `ElevatedTask`'s own `image` field names is read fresh by
//! the elevated child and could itself be swapped during the same window —
//! so `compose_task` takes a second digest, of the image itself, BEFORE this
//! module ever raises the prompt, and carries it the same way: inside the
//! (already digest-bound) task rather than as a further command-line
//! argument. `crate::write_image::write_bytes_and_verify` digests the image
//! again as it streams it to the disk and refuses, after the write, if the
//! two disagree — the card is left holding whatever was actually swapped in,
//! but is reported as unusable rather than as a success.
//!
//! **Reached only where a card device can be written** — Windows and Linux,
//! the two platforms [`crate::enumerate`] can enumerate. Elsewhere the whole
//! module is compiled and never called, which is the platform gate working
//! (`ADR-062` §5), so its dead-code lints are silenced there and only there.
#![cfg_attr(not(any(windows, target_os = "linux", target_os = "macos")), allow(dead_code, unused_imports))]

use std::path::{Path, PathBuf};

use rivet_core::package::port::Digest;
use rivet_crypto::Blake3;

use crate::enumerate::{self, Disk};

// ---------------------------------------------------------------------------
// The task file — what the parent tells the child, and nothing it is trusted
// to believe without re-checking (see the module header).
// ---------------------------------------------------------------------------

/// Everything the elevated child needs, and the whole of what it is told.
///
/// **Four lines, in a fixed order, nothing more.** Not JSON: this never
/// leaves the machine and has exactly one reader, which is this module's own
/// [`ElevatedTask::from_text`] — a hand-rolled parser earns its keep only
/// when a format crosses a boundary this project does not own, and this one
/// crosses nothing but a `RunAs` verb between two invocations of the same
/// binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElevatedTask {
    /// The card image, already carrying the injected document on both boot
    /// partitions — nothing about elevation touches that step.
    pub image: PathBuf,
    /// The BLAKE3 digest of `image`'s own bytes, hex-encoded, taken by the
    /// unelevated parent BEFORE the elevation prompt ([`compose_task`]) —
    /// the same way [`task_digest`] binds this whole task file, but carried
    /// INSIDE it rather than as a second command-line argument, since it is
    /// only ever needed once this task is already trusted.
    ///
    /// [`crate::write_image::write_bytes_and_verify`] digests the image again
    /// as it streams it to the disk and refuses if the two disagree. A
    /// same-user process that cannot rewrite the digest-bound task file
    /// during the UAC wait could still rewrite the image it names, and this is
    /// what catches that.
    pub image_digest: String,
    /// The disk to write, as this host's own enumeration named it. The child
    /// looks this up again in its own enumeration rather than trusting it —
    /// see the module header.
    pub device_id: String,
    /// Where the child appends `PHASE\tDONE\tTOTAL\n` lines as it works.
    pub progress_path: PathBuf,
    /// Where the child writes its [`Outcome`] before it exits, whatever that
    /// outcome is.
    pub result_path: PathBuf,
}

impl ElevatedTask {
    /// The exact bytes [`Self::write`] persists — shared with it and with
    /// [`task_digest`], so the parent's digest is provably of what actually
    /// landed on disk rather than a second rendering that could drift from
    /// it.
    fn to_bytes(&self) -> Vec<u8> {
        format!(
            "{}\n{}\n{}\n{}\n{}\n",
            self.image.display(),
            self.image_digest,
            self.device_id,
            self.progress_path.display(),
            self.result_path.display(),
        )
        .into_bytes()
    }

    /// # Errors
    ///
    /// The file cannot be created or written.
    pub fn write(&self, path: &Path) -> Result<(), String> {
        std::fs::write(path, self.to_bytes())
            .map_err(|e| format!("cannot write the elevation task file {}: {e}", path.display()))
    }

    /// The one parser for the five-line format — [`read_task_verified`]'s
    /// last step, once it already has the file's digest-checked text in
    /// hand, and this module's tests' own way of proving the format round
    /// trips with no digest involved at all.
    ///
    /// # Errors
    ///
    /// `text` has fewer than its five lines — never silently defaults a
    /// missing field, `AGENTS.md` §5 — or the image-digest line is not
    /// [`looks_like_digest_hex`]. Either way this returns before a single
    /// byte of any device is touched: [`read_task_verified`] calls this
    /// through `?`, and its Windows-only caller does the same before it
    /// ever reaches a write.
    fn from_text(text: &str, path: &Path) -> Result<Self, String> {
        let mut lines = text.lines();
        let missing = || format!("{} is missing a line this format requires", path.display());
        let image = lines.next().ok_or_else(missing)?;
        let image_digest = lines.next().ok_or_else(missing)?;
        let device_id = lines.next().ok_or_else(missing)?;
        let progress_path = lines.next().ok_or_else(missing)?;
        let result_path = lines.next().ok_or_else(missing)?;
        if !looks_like_digest_hex(image_digest) {
            return Err(format!(
                "{} names {image_digest:?} as the image's own digest, which is not 64 lowercase \
                 hex characters, so the image cannot be bound to the run that named it. NOTHING \
                 WAS DONE.",
                path.display(),
            ));
        }
        Ok(Self {
            image: PathBuf::from(image),
            image_digest: image_digest.to_owned(),
            device_id: device_id.to_owned(),
            progress_path: PathBuf::from(progress_path),
            result_path: PathBuf::from(result_path),
        })
    }
}

/// Whether `hex` is shaped like a BLAKE3 digest [`task_digest`] or
/// [`crate::write_image::digest_file`] could have produced: 64 lowercase hex
/// characters, nothing more or less.
///
/// **Structural only.** This says a value is SHAPED like a digest, not that
/// it is the RIGHT one — [`crate::write_image::write_bytes_and_verify`]'s own
/// comparison, once the image has actually been streamed, is what decides
/// that.
fn looks_like_digest_hex(hex: &str) -> bool {
    hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The BLAKE3 digest of `bytes`, hex-encoded the same way this binary
/// already prints a card's own digest (`rivetos_tls::encode_hex`) — one
/// hashing primitive and one hex alphabet in this binary, not a second pair
/// invented for this check.
#[cfg_attr(not(windows), allow(dead_code))]
fn task_digest(bytes: &[u8]) -> String {
    rivetos_tls::encode_hex(&Blake3::new().of(bytes))
}

/// Read and parse `path` as an [`ElevatedTask`], refusing FIRST if its bytes
/// right now do not hash to `expected_digest_hex` — the child's half of the
/// binding `run_elevated` (Windows-only, so named rather than linked from
/// this cross-platform function) establishes when it computes
/// [`task_digest`] of the bytes it just wrote and carries that digest on the
/// elevated command line rather than inside the file a same-user attacker
/// can rewrite during the UAC prompt. See the module header.
///
/// # Errors
///
/// `expected_digest_hex` is empty; `path` cannot be read; the digest of
/// what `path` holds right now does not equal `expected_digest_hex` —
/// whether because the file was rewritten after the digest was computed, or
/// because the value given on the command line is simply wrong, this reads
/// the same either way, and refuses before a single field of the file is
/// parsed, `AGENTS.md` §5; or [`ElevatedTask::from_text`] refuses the
/// (now digest-verified) bytes.
///
/// Cross-platform on purpose, like [`task_digest`]: its only PRODUCTION
/// caller is Windows-only, but proving the digest binding itself needs
/// neither Windows nor a real elevated child, so its tests run everywhere —
/// the same reasoning `disk_number` and `diskpart_clean_script` already
/// apply below.
#[cfg_attr(not(windows), allow(dead_code))]
fn read_task_verified(path: &Path, expected_digest_hex: &str) -> Result<ElevatedTask, String> {
    if expected_digest_hex.is_empty() {
        return Err(format!(
            "no digest was given alongside the elevation task file {}, so its contents cannot \
             be bound to the run that named it. NOTHING WAS DONE.",
            path.display(),
        ));
    }
    let bytes = std::fs::read(path)
        .map_err(|e| format!("cannot read the elevation task file {}: {e}", path.display()))?;
    let actual_digest = task_digest(&bytes);
    if actual_digest != expected_digest_hex {
        return Err(format!(
            "the elevation task file {} does not match the digest this run started with, so it \
             is refused rather than trusted. NOTHING WAS DONE. Either the file was rewritten \
             after the digest was computed, or the digest given on the command line is wrong — \
             `SPEC-05` §4's guard against a task file changing underneath an elevated write.",
            path.display(),
        ));
    }
    let text = String::from_utf8(bytes)
        .map_err(|e| format!("the elevation task file {} is not valid UTF-8: {e}", path.display()))?;
    ElevatedTask::from_text(&text, path)
}

// ---------------------------------------------------------------------------
// Progress — appended by the child, polled and rendered by the parent.
// ---------------------------------------------------------------------------

/// One reading off the progress file: which phase, how far, out of how much.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressLine {
    pub phase: String,
    pub done: u64,
    pub total: u64,
}

/// Add one reading to the progress file, creating it if this is the first.
///
/// **Appended, never truncated and rewritten.** A rewrite is a window where
/// the file is briefly empty and the parent's poll would read nothing at all;
/// an append's only failure mode is the *next* poll racing the write of the
/// newest line, which [`parse_progress`] is built to tolerate by falling back
/// to the last complete one.
///
/// # Errors
///
/// The file cannot be opened for appending, or the line cannot be written.
pub fn append_progress(path: &Path, phase: &str, done: u64, total: u64) -> Result<(), String> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("cannot open the progress file {}: {e}", path.display()))?;
    writeln!(file, "{phase}\t{done}\t{total}")
        .map_err(|e| format!("cannot append progress to {}: {e}", path.display()))
}

/// Read whatever the progress file currently holds, or `None` if there is
/// nothing readable yet — never an error. **Best-effort by design**: this is
/// polled every few hundred milliseconds while a card is written, and a
/// momentary read failure (the file not created yet, a locked handle) is not
/// news; it is answered by trying again on the next poll.
#[must_use]
pub fn read_progress(path: &Path) -> Option<ProgressLine> {
    let text = std::fs::read_to_string(path).ok()?;
    parse_progress(&text)
}

/// The last COMPLETE line in `text`, parsed.
///
/// **The reason this is not simply "the last line"**: the parent can poll
/// mid-`write_all` on the child's side, and Windows gives no cross-process
/// guarantee that an append lands atomically once it is more than a few dozen
/// bytes. A line with no trailing `\n` yet might be half of `"write\t512"`
/// that will become `"write\t5242880\t..."` on the next poll — reading it now
/// would report a number that never actually happened. So a line is trusted
/// only once its own newline has arrived; until then, the previous complete
/// line answers instead, and if none has arrived yet, `None` does.
#[must_use]
pub fn parse_progress(text: &str) -> Option<ProgressLine> {
    let complete = if text.ends_with('\n') {
        text
    } else {
        &text[..=text.rfind('\n')?]
    };
    complete.lines().rev().find_map(parse_progress_line)
}

fn parse_progress_line(line: &str) -> Option<ProgressLine> {
    let mut parts = line.splitn(3, '\t');
    let phase = parts.next()?.to_owned();
    let done: u64 = parts.next()?.parse().ok()?;
    let total: u64 = parts.next()?.parse().ok()?;
    Some(ProgressLine { phase, done, total })
}

// ---------------------------------------------------------------------------
// The result — the one thing the child is trusted to have written, since it
// is what the parent reads AFTER deciding the child has finished.
// ---------------------------------------------------------------------------

/// What the elevated child (or, for [`Outcome::Declined`], the unelevated
/// launcher itself) reports before anything further happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The write and the read-back both succeeded. Carries the same report
    /// string [`crate::write_image::write_bytes_and_verify`] already returns —
    /// not re-derived here, so the two cannot disagree about what happened.
    Success(String),
    /// The child's OWN re-enumeration would not vouch for the device, or it
    /// did not fit. `SPEC-05` §4's guard that cannot be clicked through —
    /// see the module header.
    Refused(String),
    /// Something else stopped the write or the verify.
    Failed(String),
    /// **Written by the unelevated launcher, not the child** — the child
    /// never started at all. `Start-Process -Verb RunAs` raises when the UAC
    /// prompt is cancelled, and that is an ordinary outcome: somebody who
    /// read the plan and changed their mind behaved correctly, and this
    /// carries the .NET exception's own words rather than inventing new ones.
    Declined(String),
}

impl Outcome {
    fn tag(&self) -> &'static str {
        match self {
            Self::Success(_) => "OK",
            Self::Refused(_) => "REFUSED",
            Self::Failed(_) => "FAILED",
            Self::Declined(_) => "DECLINED",
        }
    }

    fn detail(&self) -> &str {
        match self {
            Self::Success(s) | Self::Refused(s) | Self::Failed(s) | Self::Declined(s) => s,
        }
    }

    /// # Errors
    ///
    /// The file cannot be written.
    pub fn write(&self, path: &Path) -> Result<(), String> {
        std::fs::write(path, format!("{}\n{}", self.tag(), self.detail()))
            .map_err(|e| format!("cannot write the elevation result file {}: {e}", path.display()))
    }

    /// `Ok(None)` when the file is absent — a live possibility, not
    /// an error, while the elevated child is still running or the UAC prompt
    /// is still on screen.
    ///
    /// # Errors
    ///
    /// The file exists and cannot be read, or its first line is not one of
    /// the four tags this build knows — **never silently read as a pass**,
    /// `AGENTS.md` §7: an unreadable result and a true failure must not look
    /// the same to whoever is about to trust a card.
    pub fn read(path: &Path) -> Result<Option<Self>, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("cannot read the elevation result file {}: {e}", path.display())),
        };
        // **A BOM, and it is not a theoretical one.** Windows PowerShell 5.1
        // writes UTF-8 *with* a byte-order mark for `-Encoding utf8`, so the
        // file this parent reads begins `\u{feff}DECLINED` and every tag below
        // misses. Measured 2026-09-09 on the first cancelled prompt: the parent
        // reported *"names an outcome this build does not recognise"* about a
        // child that had said exactly the right word.
        //
        // Stripped here rather than at the writer, because the writer is a
        // PowerShell command line and which PowerShell runs it is the operator's
        // machine's business — `pwsh` 7 writes no mark and 5.1 does, and a
        // reader that copes with both needs no agreement about which is
        // installed.
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let (tag, rest) = text.split_once('\n').unwrap_or((text.trim_end(), ""));
        let detail = rest.trim_start_matches('\u{feff}').to_owned();
        match tag {
            "OK" => Ok(Some(Self::Success(detail))),
            "REFUSED" => Ok(Some(Self::Refused(detail))),
            "FAILED" => Ok(Some(Self::Failed(detail))),
            "DECLINED" => Ok(Some(Self::Declined(detail))),
            other => Err(format!(
                "{} names an outcome this build does not recognise: {other:?}. This is either a \
                 version mismatch between the elevated child and this parent, or a corrupted \
                 file — either way, nothing here can say what actually happened to the device.",
                path.display(),
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Windows: launching the elevated child and waiting on it from a distance.
// ---------------------------------------------------------------------------

/// Run `cargo rivet write-image`'s write-and-verify step in a short-lived
/// elevated child, polling `disk`'s own progress file while it works.
///
/// **This function itself never touches a device.** It writes a task file,
/// asks PowerShell to relaunch this same binary elevated against it, renders
/// whatever the child reports while waiting, and turns the child's
/// [`Outcome`] into the `Result` `crate::write_image::write_and_verify`
/// already returns for every other platform — the caller does not need to
/// know two processes were involved.
///
/// # Errors
///
/// This host's own path cannot be found (`current_exe` failing); PowerShell
/// could not be started at all; the elevated helper exited leaving no result
/// file, which is reported rather than read as either success or failure —
/// `AGENTS.md` §7 applies to a missing file exactly as it does to a claim in
/// prose; or the child's own [`Outcome::Refused`] or [`Outcome::Failed`].
#[cfg(any(windows, target_os = "macos"))]
pub fn write_via_elevated_child(image: &Path, disk: &Disk) -> Result<String, String> {
    run_elevated(image, disk, "--elevated-write")
}

/// **`cargo rivet write-image --verify-only`, standing alone.** The whole-disk
/// read `SPEC-05` §4 requires needs administrator exactly as much as the
/// write does — measured 2026-09-09, an unelevated shell was refused even a
/// *read* of `\\.\PHYSICALDRIVEn` — so this takes the same elevated-child
/// path as [`write_via_elevated_child`], differing only in which verb the
/// child is launched under and in what the child actually does once it gets
/// there: [`attempt_elevated_verify`] never dismounts, never takes the disk
/// offline, and never opens it for writing at all.
///
/// # Errors
///
/// The same shape as [`write_via_elevated_child`], read as a verify failure
/// rather than a write one.
#[cfg(any(windows, target_os = "macos"))]
pub fn verify_via_elevated_child(image: &Path, disk: &Disk) -> Result<String, String> {
    run_elevated(image, disk, "--elevated-verify")
}

/// Compose the task the elevated child will read, digesting `image` right
/// now — before the elevation prompt this raises on Windows — alongside
/// everything else it already knows.
///
/// **This is the digest [`crate::write_image::write_bytes_and_verify`]
/// compares its own write-time digest against.** A same-user process cannot
/// rewrite the task file itself during the UAC wait without failing
/// [`task_digest`]'s check (the module header), but nothing stopped it
/// rewriting the IMAGE the task merely names — until the elevated writer
/// re-digests the image as it streams it and refuses on a mismatch.
///
/// Cross-platform on purpose, like [`task_digest`] and [`read_task_verified`]:
/// composing the task needs neither Windows nor a real elevated child, which
/// is what lets it be tested here; the Windows-only `run_elevated` is its
/// production caller.
///
/// # Errors
///
/// `image` cannot be opened or read, to be digested.
#[cfg_attr(not(windows), allow(dead_code))]
fn compose_task(
    image: &Path,
    device_id: &str,
    progress_path: PathBuf,
    result_path: PathBuf,
) -> Result<ElevatedTask, String> {
    let image_digest = crate::write_image::digest_file(image)?;
    Ok(ElevatedTask {
        image: image.to_path_buf(),
        image_digest,
        device_id: device_id.to_owned(),
        progress_path,
        result_path,
    })
}

/// The orchestration shared by a write and a verify: write the task, launch
/// the elevated child under `verb`, poll and render its progress, and turn
/// its [`Outcome`] into a `Result`. What differs between the two is entirely
/// on the CHILD's side of the boundary — see [`attempt_elevated_write`] and
/// [`attempt_elevated_verify`].
#[cfg(any(windows, target_os = "macos"))]
fn run_elevated(image: &Path, disk: &Disk, verb: &str) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| {
        format!("cannot find this build's own path, so there is nothing to relaunch elevated: {e}")
    })?;

    let tag = temp_tag();
    let dir = std::env::temp_dir();
    let task_path = dir.join(format!("cargo-rivet-provision-{tag}.task"));
    let progress_path = dir.join(format!("cargo-rivet-provision-{tag}.progress"));
    let result_path = dir.join(format!("cargo-rivet-provision-{tag}.result"));

    let task = compose_task(image, &disk.id, progress_path.clone(), result_path.clone())?;
    task.write(&task_path)?;
    // Computed from the bytes just handed to `write`, not re-read from disk —
    // see the module header. Carried on the elevated command line below,
    // never inside the file itself.
    let digest = task_digest(&task.to_bytes());

    let cleanup = || {
        let _ = std::fs::remove_file(&task_path);
        let _ = std::fs::remove_file(&progress_path);
        let _ = std::fs::remove_file(&result_path);
    };

    // `None` is a launcher that already recorded the refusal in the result
    // file — macOS's `sudo -v` declined before any child existed.
    let child = match spawn_launcher(&exe, verb, &task_path, &digest, &result_path) {
        Ok(c) => c,
        Err(e) => {
            cleanup();
            return Err(e);
        }
    };

    if let Some(mut child) = child
        && let Err(e) = wait_and_render(&mut child, &progress_path)
    {
        cleanup();
        return Err(e);
    }

    let outcome = Outcome::read(&result_path);
    cleanup();

    interpret_outcome(outcome?, verb)
}

/// Turn whatever the result file held into the `Result` the caller of
/// [`write_via_elevated_child`] or [`verify_via_elevated_child`] sees —
/// pulled out of [`run_elevated`] so **the declined-UAC path can be proven
/// with no PowerShell, no process, and no file at all**: an interactive UAC
/// prompt cannot be driven from a test, but the interpretation of what its
/// cancellation means can be, in full.
#[cfg(any(windows, target_os = "macos"))]
fn interpret_outcome(outcome: Option<Outcome>, verb: &str) -> Result<String, String> {
    match outcome {
        None => Err(format!(
            "the elevated helper exited and left no result file at all, so nothing here can say \
             whether anything happened to the device. That is not a pass — SPEC-05 §4 treats a \
             missing result the same as a failed one. If this repeats, run the same {verb} \
             command again and read anything the helper printed; it may have crashed before it \
             could report."
        )),
        Some(Outcome::Declined(msg)) => {
            Ok(format!("nothing was written. The administrator prompt was declined: {msg}\n"))
        }
        Some(Outcome::Refused(msg) | Outcome::Failed(msg)) => Err(msg),
        Some(Outcome::Success(report)) => Ok(report),
    }
}

/// Poll `child` until it exits, printing whatever [`read_progress`] finds on
/// each pass — a carriage-returned line so a two-gigabyte write does not
/// scroll a screen full of numbers.
#[cfg(any(windows, target_os = "macos"))]
fn wait_and_render(child: &mut std::process::Child, progress_path: &Path) -> Result<(), String> {
    use std::io::Write as _;

    const POLL: std::time::Duration = std::time::Duration::from_millis(300);
    let mut last_width = 0usize;
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {}
            Err(e) => return Err(format!("cannot poll the elevated helper: {e}")),
        }
        if let Some(line) = read_progress(progress_path) {
            let rendered = format!(
                "\r  {:<9} {} of {}",
                line.phase,
                crate::enumerate::human_size(line.done),
                crate::enumerate::human_size(line.total),
            );
            print!("{rendered}{}", " ".repeat(last_width.saturating_sub(rendered.len())));
            last_width = rendered.len();
            let _ = std::io::stdout().flush();
        }
        std::thread::sleep(POLL);
    }
    println!();
    Ok(())
}

/// An identifier for this run's three temporary files — unpredictable enough
/// that a second `cargo rivet write-image` running at the same moment cannot
/// collide with it, without needing a real random-number source for what is
/// only ever a filename.
#[cfg(any(windows, target_os = "macos"))]
fn temp_tag() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{nanos}", std::process::id())
}

/// Ask PowerShell to relaunch `exe` elevated against `task_path`, and catch
/// the one error that means "the person said no."
///
/// **Why this writes [`Outcome::Declined`] itself, unelevated, rather than
/// leaving the parent to infer it from an exit code.** `Start-Process -Verb
/// RunAs` raises a terminating `System.ComponentModel.Win32Exception` the
/// moment the UAC prompt is cancelled — before `-Wait` ever begins waiting,
/// because the process never started — and that exception's own message
/// (*"The operation was canceled by the user"*) is the most honest text
/// available for what happened. Catching it here and writing it to the SAME
/// result file the elevated child would have written keeps the parent's
/// [`Outcome::read`] the one and only place that interprets what occurred,
/// instead of a second code path for "the launcher itself failed."
#[cfg(windows)]
fn spawn_launcher(
    exe: &Path,
    verb: &str,
    task_path: &Path,
    digest: &str,
    result_path: &Path,
) -> Result<Option<std::process::Child>, String> {
    // **Built as separate elements, each correctly quoted, rather than one
    // hand-wrapped string.** The old `format!("\"write-image\" \"{verb}\"
    // \"{}\"", task_path.display())` wrapped each token in literal `"..."`
    // with no escaping, so a `"` inside `task_path` would close the argument
    // early. `quote_windows_argument` is the escaping every process on this
    // platform uses to recover its own argv from this same single string,
    // matching what the elevated child's `std::env::args` does on the other
    // end.
    let argument_line = elevated_argv(verb, task_path, digest)
        .iter()
        .map(|a| quote_windows_argument(a))
        .collect::<Vec<_>>()
        .join(" ");
    // **`-WindowStyle Hidden`: the elevated child gets no visible console.**
    // `-Verb RunAs` on a console binary otherwise spawns a fresh, blank
    // `Administrator: cargo-rivet.exe` window that shows nothing and does
    // nothing an operator reads — the child reports through `result_path`,
    // which the UNELEVATED parent renders in the console the operator is
    // already watching. Hiding it removes a window that only ever looked like
    // it might be waiting for something. The UAC consent prompt is Windows'
    // own and is unaffected.
    let script = format!(
        "$ErrorActionPreference = 'Stop'; \
         try {{ \
             Start-Process -FilePath {exe} -ArgumentList {args} -Verb RunAs -WindowStyle Hidden -Wait; \
         }} catch {{ \
             $msg = $_.Exception.Message; \
             Set-Content -LiteralPath {result} -Value (\"DECLINED`n\" + $msg) -Encoding utf8 -NoNewline; \
         }}",
        exe = powershell_quote(&exe.display().to_string()),
        args = powershell_quote(&argument_line),
        result = powershell_quote(&result_path.display().to_string()),
    );
    std::process::Command::new(powershell_exe())
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .spawn()
        .map(Some)
        .map_err(|e| format!("powershell could not be started to request administrator: {e}"))
}

/// macOS: ask for the password with `sudo -v` in this terminal, then start
/// the child with `sudo -n` on the credential that just granted.
///
/// **Two steps, not one**, because the parent renders progress on this same
/// terminal while the child works, and a password prompt from a `sudo` that is
/// also the child would be overwritten by it. `-n` makes the second `sudo`
/// refuse rather than prompt, so a credential that has lapsed fails loudly
/// instead of waiting on a prompt nobody can see.
///
/// A declined or failed `sudo -v` writes [`Outcome::Declined`] to the result
/// file itself — the one place the parent reads — and starts no child.
///
/// `sudo` is named by absolute path for `powershell_exe`'s reason: this is the
/// process trusted to decide what runs as root.
#[cfg(target_os = "macos")]
fn spawn_launcher(
    exe: &Path,
    verb: &str,
    task_path: &Path,
    digest: &str,
    result_path: &Path,
) -> Result<Option<std::process::Child>, String> {
    let granted = std::process::Command::new("/usr/bin/sudo")
        .args(["-v", "-p", "Password for %u (to write the card as root): "])
        .status()
        .map_err(|e| format!("sudo could not be started to request administrator: {e}"))?;
    if !granted.success() {
        Outcome::Declined(format!("sudo did not grant administrator ({granted})"))
            .write(result_path)?;
        return Ok(None);
    }
    std::process::Command::new("/usr/bin/sudo")
        .arg("-n")
        .arg("--")
        .arg(exe)
        .args(["write-image", verb])
        .arg(task_path)
        .arg(digest)
        .stdin(std::process::Stdio::null())
        .spawn()
        .map(Some)
        .map_err(|e| format!("sudo could not start the write as root: {e}"))
}

/// The elevated child's own argv, as separate elements — `write-image`
/// (`main`'s own dispatch word), which verb, the task file, and the digest
/// that binds it to this run ([`task_digest`]). [`spawn_launcher`] quotes
/// each one for the Windows command line with [`quote_windows_argument`]
/// rather than hand-wrapping the whole line.
#[cfg(windows)]
fn elevated_argv(verb: &str, task_path: &Path, digest: &str) -> Vec<String> {
    vec![
        "write-image".to_owned(),
        verb.to_owned(),
        task_path.display().to_string(),
        digest.to_owned(),
    ]
}

/// Quote one argument for a Windows command line — the escaping every
/// process on this platform uses to recover its own `argv` from the single
/// string `CreateProcess`/`ShellExecuteEx` hands it, which is how the
/// elevated child reads back what [`spawn_launcher`] sends it
/// (`std::env::args`, the same algorithm, in reverse).
///
/// **Always wraps in `"..."`**, even when nothing inside needs escaping —
/// simpler than deciding per argument, and no less correct for it. What the
/// naive `format!("\"{value}\"")` this replaces did NOT handle: a literal
/// `"` inside `value`, and a run of `\` immediately before the closing
/// quote, either of which would otherwise end the argument early or merge
/// it with the next one.
#[cfg(windows)]
fn quote_windows_argument(value: &str) -> String {
    let mut out = String::from("\"");
    let mut backslashes: usize = 0;
    for c in value.chars() {
        if c == '\\' {
            backslashes += 1;
        } else {
            if c == '"' {
                // The backslashes already pushed below, plus one more, so
                // the quote about to be pushed is escaped data rather than
                // this argument's closing quote.
                for _ in 0..=backslashes {
                    out.push('\\');
                }
            }
            backslashes = 0;
        }
        out.push(c);
    }
    // Any backslashes immediately before the closing quote must be doubled,
    // or they would escape THAT quote instead of standing for themselves.
    for _ in 0..backslashes {
        out.push('\\');
    }
    out.push('"');
    out
}

/// The fully-qualified path to `relative` under `system_root` — so a spawn
/// resolves a known location rather than searching `PATH`, where a writable
/// directory earlier than `System32` could substitute a different binary.
/// Pure: `system_root` is passed in rather than read here, so both the
/// present-`SystemRoot` and absent-`SystemRoot` cases are provable with no
/// environment mutation.
///
/// # Errors
///
/// Never — an absent `system_root` falls back to `C:\Windows`, which is
/// where `SystemRoot` points on every Windows installation this project has
/// seen. If that fallback is ever wrong, [`std::process::Command::spawn`]
/// reports a file-not-found; this never guesses harder than that.
#[cfg(windows)]
fn resolve_system_binary(system_root: Option<&str>, relative: &str) -> PathBuf {
    Path::new(system_root.unwrap_or(r"C:\Windows")).join(relative)
}

/// The real `powershell.exe`, resolved from `%SystemRoot%` rather than
/// found by searching `PATH` — this is the process trusted to build the
/// `Start-Process -Verb RunAs` call that names what gets elevated, so a
/// substituted one is worse than an ordinary `PATH` hijack.
#[cfg(windows)]
fn powershell_exe() -> PathBuf {
    resolve_system_binary(
        std::env::var("SystemRoot").ok().as_deref(),
        r"System32\WindowsPowerShell\v1.0\powershell.exe",
    )
}

/// [`powershell_exe`]'s sibling for `diskpart.exe`.
#[cfg(windows)]
fn diskpart_exe() -> PathBuf {
    resolve_system_binary(std::env::var("SystemRoot").ok().as_deref(), r"System32\diskpart.exe")
}

/// A single-quoted PowerShell string literal holding exactly `value`.
///
/// **Single-quoted, not double**, because a double-quoted PowerShell string
/// interpolates `$variables` and this project has no need of that and every
/// need to avoid it — a path containing a literal `$` is otherwise-ordinary
/// and must not be read as a reference to one. The one character that still
/// needs escaping inside a single-quoted literal is the quote itself,
/// doubled, which is PowerShell's own rule for it.
#[cfg(windows)]
fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

// ---------------------------------------------------------------------------
// Windows: the elevated child's own side of the conversation.
// ---------------------------------------------------------------------------

/// **The entry point `cargo-rivet.exe write-image --elevated-write <task>
/// <digest>` reaches**, running under whatever elevation `Start-Process -Verb
/// RunAs` obtained. Verifies the task file against `digest`, re-enumerates,
/// writes, verifies, and reports — see the module header for why every one
/// of those is re-derived rather than trusted from the task file.
///
/// # Errors
///
/// Whatever [`read_task_verified`] failed on — including a task file that
/// does not match `digest` — before that point there is no `result_path` to
/// write a report to, so a refusal here reaches the parent only as "no
/// result file," which [`write_via_elevated_child`] already treats as a
/// failure rather than a pass. After that point, this always writes an
/// [`Outcome`] before returning, whatever the outcome was.
#[cfg(any(windows, target_os = "macos"))]
pub fn run_elevated_child(task_path: &Path, digest: &str) -> Result<(), String> {
    let task = read_task_verified(task_path, digest)?;
    let outcome = attempt_elevated_write(&task);
    outcome.write(&task.result_path)?;
    match outcome {
        Outcome::Success(_) | Outcome::Declined(_) => Ok(()),
        Outcome::Refused(m) | Outcome::Failed(m) => Err(m),
    }
}

/// **The entry point `cargo-rivet.exe write-image --elevated-verify <task>
/// <digest>` reaches** — [`run_elevated_child`]'s sibling for a read-only
/// check.
///
/// # Errors
///
/// The same shape as [`run_elevated_child`].
#[cfg(any(windows, target_os = "macos"))]
pub fn run_elevated_verify_child(task_path: &Path, digest: &str) -> Result<(), String> {
    let task = read_task_verified(task_path, digest)?;
    let outcome = attempt_elevated_verify(&task);
    outcome.write(&task.result_path)?;
    match outcome {
        Outcome::Success(_) | Outcome::Declined(_) => Ok(()),
        Outcome::Refused(m) | Outcome::Failed(m) => Err(m),
    }
}

/// Look `task.device_id` up in THIS context's own enumeration and refuse
/// anything it will not vouch for, then measure the image — the half of the
/// work identical between a write and a verify. Returns the disk and the
/// image's length on success, or the [`Outcome`] to report immediately.
#[cfg(any(windows, target_os = "macos"))]
fn locate_and_check(task: &ElevatedTask) -> Result<(Disk, u64), Outcome> {
    let disks = enumerate::all().map_err(|e| {
        Outcome::Failed(format!(
            "this elevated context could not enumerate disks on its own account: {e}. \
             SPEC-05 §4 gives the removability check no bypass, so a device it cannot see \
             for itself is refused rather than trusted from the unelevated half."
        ))
    })?;
    let Some(disk) = disks.iter().find(|d| d.id == task.device_id) else {
        return Err(Outcome::Refused(format!(
            "NOTHING WAS WRITTEN. {} is not a disk THIS elevated context's own enumeration can \
             see. The unelevated half could be wrong about which disk is which — a stale \
             enumeration, a card swapped while the [y/N] prompt was on screen — and this check \
             exists precisely so no amount of typing or clicking in that half can get past it.",
            task.device_id,
        )));
    };
    crate::write_image::refuse_unless_removable(disk).map_err(Outcome::Refused)?;

    let image_len = std::fs::metadata(&task.image)
        .map_err(|e| Outcome::Failed(format!("cannot measure {}: {e}", task.image.display())))?
        .len();
    if disk.size_bytes != 0 && disk.size_bytes < image_len {
        return Err(Outcome::Refused(format!(
            "NOTHING WAS WRITTEN. {} is {} bytes and the image is {image_len} bytes — it does \
             not fit.",
            disk.id, disk.size_bytes,
        )));
    }
    Ok((disk.clone(), image_len))
}

/// The actual write, from inside the elevated context — every step
/// re-derived from [`crate::enumerate::all`] rather than taken on the parent's
/// word, per the module header.
#[cfg(any(windows, target_os = "macos"))]
fn attempt_elevated_write(task: &ElevatedTask) -> Outcome {
    let (disk, image_len) = match locate_and_check(task) {
        Ok(pair) => pair,
        Err(outcome) => return outcome,
    };

    let _ = append_progress(&task.progress_path, PREPARE_PHASE, 0, image_len);
    let prepared = match prepare_disk(&disk.id) {
        Ok(prepared) => prepared,
        Err(why) => {
            return Outcome::Failed(format!(
                "could not free {} of its mounted volumes before writing: {why}. A raw write to \
                 a disk with mounted volumes is refused or, worse, reported successful while a \
                 filesystem driver writes behind it — so this refuses to proceed rather than try \
                 anyway.",
                disk.id,
            ));
        }
    };

    let write_result = crate::write_image::write_bytes_and_verify(
        &task.image,
        &disk.id,
        image_len,
        Some(&task.progress_path),
        Some(&task.image_digest),
    );

    let _ = append_progress(&task.progress_path, "restore", image_len, image_len);
    let restore_result = restore_disk(&disk.id, prepared);

    match (write_result, restore_result) {
        (Ok(report), Ok(())) => Outcome::Success(report),
        (Ok(report), Err(why)) => Outcome::Failed(format!(
            "{report}\nBUT {} could not be brought back online afterwards: {why}. The card was \
             written and verified — this is a housekeeping failure, not a data one — but the \
             drive letters will not reappear on their own. `Set-Disk -Number {} -IsOffline \
             $false` (or Disk Management) brings it back.",
            disk.id,
            disk_number(&disk.id).map_or_else(|| "N".to_owned(), |n| n.to_string()),
        )),
        (Err(why), Ok(())) => Outcome::Failed(why),
        (Err(why), Err(restore_why)) => Outcome::Failed(format!(
            "{why}\nAND {} could not be brought back online afterwards either: {restore_why}.",
            disk.id,
        )),
    }
}

/// The read-only check, from inside the elevated context. **Never dismounts,
/// never takes the disk offline, never opens it for writing** — a verify
/// reads exactly the bytes already there, so none of a write's
/// housekeeping applies, and `crate::write_image::verify_device_against_image`
/// is the same streamed hash-and-compare [`crate::write_image::write_bytes_and_verify`]'s
/// own read-back half performs, called here with nothing written first.
#[cfg(any(windows, target_os = "macos"))]
fn attempt_elevated_verify(task: &ElevatedTask) -> Outcome {
    let (disk, _image_len) = match locate_and_check(task) {
        Ok(pair) => pair,
        Err(outcome) => return outcome,
    };
    match crate::write_image::verify_device_against_image(&task.image, &disk.id, Some(&task.progress_path)) {
        Ok(report) => Outcome::Success(report),
        Err(why) => Outcome::Failed(why),
    }
}

/// `Set-Disk -Number N -IsOffline $true` before the write, `$false` to
/// restore it — `SPEC-05` §4's dismount step. **One mechanism handles both
/// named alternatives at once**: taking a disk offline dismounts every volume
/// on it in the same call, which is the standard technique whole-disk imaging
/// tools use on Windows for exactly the reason `SPEC-05` names — a raw write
/// underneath a still-mounted filesystem driver is refused or, worse,
/// silently overwritten by it.
///
/// # Errors
///
/// `disk_id` is not a `\\.\PHYSICALDRIVEn` identifier (so its `Get-Disk`
/// number cannot be derived), PowerShell could not be run, or `Set-Disk`
/// itself refused — carrying its own last non-empty line, the same posture
/// [`crate::enumerate::all`] takes toward CIM's own errors.
#[cfg(windows)]
fn set_disk_offline(disk_id: &str, offline: bool) -> Result<(), String> {
    let Some(number) = disk_number(disk_id) else {
        return Err(format!(
            "{disk_id} is not a \\\\.\\PHYSICALDRIVEn identifier, so its Get-Disk NUMBER cannot \
             be derived to take it offline or restore it."
        ));
    };
    let flag = if offline { "$true" } else { "$false" };
    let script = format!("$ErrorActionPreference = 'Stop'; Set-Disk -Number {number} -IsOffline {flag}");
    let out = std::process::Command::new(powershell_exe())
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|e| format!("powershell could not be run: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&out.stderr);
    let last = said
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("Set-Disk refused and said nothing");
    Err(format!("Set-Disk -Number {number} -IsOffline {flag} failed: {last}"))
}

/// How the disk was made safe to write raw, so the restore step undoes only
/// what was done.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prepared {
    /// `Set-Disk -IsOffline $true` succeeded; `$false` restores it.
    Offline,
    /// `diskpart clean` removed every partition, so no volume is mounted.
    Cleaned,
}

/// Take the disk offline, or, where Windows refuses that, clean it.
///
/// **`Set-Disk -IsOffline` is "Not Supported" on removable media**
/// (`StorageWMI 1`, measured 2026-09-14 on a USB SD card reader, twice, card
/// re-seated between): Windows does not take a removable disk offline. What
/// a whole-disk write needs is only that no volume is mounted over it, and
/// `diskpart clean` gives exactly that on removable media by removing the
/// partition table the volumes came from. The disk was already confirmed
/// removable in this elevated context ([`locate_and_check`]), and the plan the
/// operator confirmed already says the write erases everything on it. A
/// filesystem driver that did write behind the image is caught by the
/// read-back digest either way.
#[cfg(windows)]
fn prepare_disk(disk_id: &str) -> Result<Prepared, String> {
    match set_disk_offline(disk_id, true) {
        Ok(()) => Ok(Prepared::Offline),
        Err(offline_why) => clean_disk(disk_id).map(|()| Prepared::Cleaned).map_err(|clean_why| {
            format!("{offline_why}; and cleaning it with diskpart instead also failed: {clean_why}")
        }),
    }
}

/// The progress label for the step before the write.
#[cfg(windows)]
const PREPARE_PHASE: &str = "offline";

/// Undo only what [`prepare_disk`] did. **A cleaned disk has nothing to bring
/// back online**: `diskpart clean` never took it offline, and the partitions
/// the image just wrote appear when the card is next inserted.
#[cfg(windows)]
fn restore_disk(disk_id: &str, prepared: Prepared) -> Result<(), String> {
    match prepared {
        Prepared::Offline => set_disk_offline(disk_id, false),
        Prepared::Cleaned => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// macOS: the child is the same; freeing the disk is `diskutil`'s.
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
const PREPARE_PHASE: &str = "unmount";

/// How the disk was freed. macOS has one way, and nothing to undo: the
/// volumes the new image carries are mounted when the card is next inserted.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prepared {
    Unmounted,
}

/// `diskutil unmountDisk` on the whole disk — every volume on it, in one call,
/// which is what a raw write needs and what Raspberry Pi Imager does.
#[cfg(target_os = "macos")]
fn prepare_disk(disk_id: &str) -> Result<Prepared, String> {
    let whole = macos_whole_disk(disk_id)
        .ok_or_else(|| format!("{disk_id} is not a /dev/rdiskN node, so it cannot be unmounted"))?;
    let out = std::process::Command::new("/usr/sbin/diskutil")
        .args(["unmountDisk", &whole])
        .output()
        .map_err(|e| format!("diskutil could not be run: {e}"))?;
    if out.status.success() {
        return Ok(Prepared::Unmounted);
    }
    let said = String::from_utf8_lossy(&out.stderr);
    let last = said.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("it said nothing");
    Err(format!("`diskutil unmountDisk {whole}` refused: {last}"))
}

#[cfg(target_os = "macos")]
fn restore_disk(_disk_id: &str, _prepared: Prepared) -> Result<(), String> {
    Ok(())
}

/// `/dev/rdiskN` → `/dev/diskN`, the node `diskutil` names a whole disk by.
/// Pure, and strict: anything but a whole raw disk is `None`, so a partition
/// or a path somebody typed is never unmounted by accident.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn macos_whole_disk(raw: &str) -> Option<String> {
    let number = raw.strip_prefix("/dev/rdisk")?;
    (!number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())).then(|| format!("/dev/disk{number}"))
}

/// `diskpart /s` with [`diskpart_clean_script`], from a script file under the
/// system temporary directory that is removed afterwards.
#[cfg(windows)]
fn clean_disk(disk_id: &str) -> Result<(), String> {
    let Some(number) = disk_number(disk_id) else {
        return Err(format!(
            "{disk_id} is not a \\\\.\\PHYSICALDRIVEn identifier, so its diskpart disk number \
             cannot be derived."
        ));
    };
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let script_path = std::env::temp_dir()
        .join(format!("cargo-rivet-clean-{}-{nonce}.txt", std::process::id()));
    std::fs::write(&script_path, diskpart_clean_script(number))
        .map_err(|e| format!("cannot write the diskpart script: {e}"))?;
    let out = std::process::Command::new(diskpart_exe()).arg("/s").arg(&script_path).output();
    let _ = std::fs::remove_file(&script_path);
    let out = out.map_err(|e| format!("diskpart could not be run: {e}"))?;
    let said = String::from_utf8_lossy(&out.stdout);
    // diskpart run with `/s` stops at a failing command and exits non-zero;
    // its success sentence is localised, so the status is what is trusted.
    if out.status.success() {
        return Ok(());
    }
    let last = said
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("diskpart refused and said nothing");
    Err(format!("diskpart clean of disk {number} failed: {last}"))
}

/// The two diskpart lines that remove every partition from disk `number` —
/// pure, so the exact script is provable with no disk at all.
#[cfg_attr(not(windows), allow(dead_code))]
fn diskpart_clean_script(number: u32) -> String {
    format!("select disk {number}\r\nclean\r\n")
}

/// The `Get-Disk`/`Set-Disk` NUMBER for a `\\.\PHYSICALDRIVEn` identifier —
/// pure, so it is provable with no PowerShell and no disk at all.
///
/// **Assumes the trailing digits of `Win32_DiskDrive.DeviceID` equal
/// `Get-Disk`'s `Number` property.** This holds on every host this project
/// has been run against; it is a Windows convention rather than a documented
/// contract, which is why `set_disk_offline`'s refusal names the identifier
/// it could not parse rather than silently guessing a number.
#[cfg_attr(not(windows), allow(dead_code))]
fn disk_number(disk_id: &str) -> Option<u32> {
    disk_id.strip_prefix(r"\\.\PHYSICALDRIVE")?.parse().ok()
}

// ---------------------------------------------------------------------------
// Linux: a different problem, not the same one — SPEC-05 §4's own words.
// ---------------------------------------------------------------------------

/// Whether this process is already running as root.
///
/// **There is no elevation dance on Linux.** Windows hands off mid-run
/// because a process cannot raise its own token — `ShellExecuteEx`'s `runas`
/// verb starts a NEW process with a new token. Linux has no equivalent
/// UAC-style hand-off for an already-running process to request, so this
/// tool does not attempt to relaunch itself with `sudo`: it checks whether it
/// is already root, and if it is not, it says so and asks the operator to
/// re-run the command themselves rather than pretending the two platforms
/// share a mechanism.
#[cfg(target_os = "linux")]
#[must_use]
pub fn running_as_root() -> bool {
    // AGENTS.md §7: an unreadable /proc/self/status must read as "not root,"
    // never as "assume yes" — refuse rather than guess, pointed at a
    // capability check instead of a piece of system state.
    let Ok(text) = std::fs::read_to_string("/proc/self/status") else {
        return false;
    };
    parse_effective_uid(&text) == Some(0)
}

/// The effective UID out of `/proc/self/status`'s own `Uid:` line — pure, so
/// it is provable against a fabricated string on this project's Windows
/// development host, which has no `/proc` at all (`AGENTS.md` §10.5).
///
/// The kernel's own layout (`Documentation/filesystems/proc.rst`) is
/// `Uid:\treal\teffective\tsaved\tfilesystem` — the second field is the one
/// that decides whether `Set-Disk`'s Linux equivalent, a raw write to
/// `/dev/sdX`, would even be attempted by the kernel.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_effective_uid(text: &str) -> Option<u32> {
    let line = text.lines().find(|l| l.starts_with("Uid:"))?;
    line.split_whitespace().nth(2)?.parse().ok()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn scratch(what: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        crate::scratch::absent(&format!(
            "elevate-{what}-{}",
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    /// Read and parse `path` with NO digest check — production code has no
    /// caller like this any more (that is the point of this item), but the
    /// four-line format itself still deserves a test with no digest in the
    /// way of what it is proving.
    fn read_unverified(path: &Path) -> Result<ElevatedTask, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read the elevation task file {}: {e}", path.display()))?;
        ElevatedTask::from_text(&text, path)
    }

    /// **The argument round trip between parent and child.** `write` then
    /// `read` must reproduce the exact struct — this is the whole of what the
    /// two halves of the elevation dance agree on, since no pipe and no
    /// shared memory crosses the `RunAs` boundary.
    #[test]
    fn a_task_round_trips_through_its_own_file() {
        let path = scratch("task");
        let task = ElevatedTask {
            image: PathBuf::from(r"C:\cards\rivet-0.1.80-card.img"),
            image_digest: "a".repeat(64),
            device_id: r"\\.\PHYSICALDRIVE2".to_owned(),
            progress_path: std::env::temp_dir().join("p.progress"),
            result_path: std::env::temp_dir().join("p.result"),
        };
        task.write(&path).expect("a scratch directory is always writable");
        let back = read_unverified(&path).expect("what was just written must read back");
        let _ = std::fs::remove_file(&path);
        assert_eq!(task, back);
    }

    /// A task file missing a line is refused by name rather than defaulting
    /// an empty path in, which would be a device write against `""`.
    #[test]
    fn a_short_task_file_is_refused() {
        let path = scratch("short");
        std::fs::write(&path, "only-one-line\n").expect("a scratch directory is always writable");
        let why = read_unverified(&path).expect_err("three missing lines must be refused");
        let _ = std::fs::remove_file(&path);
        assert!(why.contains("missing a line"), "{why}");
    }

    // -----------------------------------------------------------------------
    // HT1-1: the task file is bound to the digest the parent computed for it.
    // -----------------------------------------------------------------------

    /// **The digest accepts a task file it matches.** `write` then
    /// [`task_digest`] then [`read_task_verified`] reproduce the exact
    /// struct — [`a_task_round_trips_through_its_own_file`] proves the same
    /// thing for the unverified `read_unverified`; this proves it for the
    /// verified path the elevated child actually uses.
    #[test]
    fn an_intact_task_file_is_accepted_by_its_own_digest() {
        let path = scratch("verified-intact");
        let task = ElevatedTask {
            image: PathBuf::from(r"C:\cards\rivet-0.1.80-card.img"),
            image_digest: "a".repeat(64),
            device_id: r"\\.\PHYSICALDRIVE2".to_owned(),
            progress_path: std::env::temp_dir().join("p.progress"),
            result_path: std::env::temp_dir().join("p.result"),
        };
        task.write(&path).expect("a scratch directory is always writable");
        let digest = task_digest(&task.to_bytes());
        let back = read_task_verified(&path, &digest).expect("an intact file must be accepted");
        let _ = std::fs::remove_file(&path);
        assert_eq!(task, back);
    }

    /// **The regression HT1-1 names.** A same-user process can rewrite the
    /// task file during the seconds the UAC prompt is on screen, after the
    /// digest for THIS run was already computed and handed to the
    /// elevated child on its own command line. The rewritten file must be
    /// refused, even though it is a perfectly well-formed task on its own —
    /// a different, otherwise-valid device is exactly what a same-user
    /// attacker would substitute.
    #[test]
    fn a_task_file_rewritten_after_the_digest_was_computed_is_refused() {
        let path = scratch("verified-rewrite");
        let original = ElevatedTask {
            image: PathBuf::from(r"C:\cards\rivet-0.1.80-card.img"),
            image_digest: "a".repeat(64),
            device_id: r"\\.\PHYSICALDRIVE2".to_owned(),
            progress_path: std::env::temp_dir().join("p.progress"),
            result_path: std::env::temp_dir().join("p.result"),
        };
        original.write(&path).expect("a scratch directory is always writable");
        let digest = task_digest(&original.to_bytes());

        // The rewrite: a different, well-formed task file at the SAME path,
        // written after the digest above was already computed.
        let tampered = ElevatedTask { device_id: r"\\.\PHYSICALDRIVE0".to_owned(), ..original };
        tampered.write(&path).expect("a scratch directory is always writable");

        let why = read_task_verified(&path, &digest)
            .expect_err("a task file rewritten after its digest was computed must be refused");
        let _ = std::fs::remove_file(&path);
        assert!(why.contains("does not match"), "{why}");
    }

    /// An absent digest is refused by name, never read as "verification not
    /// requested."
    #[test]
    fn a_missing_digest_is_refused() {
        let path = scratch("verified-missing-digest");
        let task = ElevatedTask {
            image: PathBuf::from("image.img"),
            image_digest: "a".repeat(64),
            device_id: r"\\.\PHYSICALDRIVE2".to_owned(),
            progress_path: std::env::temp_dir().join("p.progress"),
            result_path: std::env::temp_dir().join("p.result"),
        };
        task.write(&path).expect("a scratch directory is always writable");
        let why = read_task_verified(&path, "").expect_err("an empty digest must be refused");
        let _ = std::fs::remove_file(&path);
        assert!(why.contains("no digest"), "{why}");
    }

    /// A digest that is simply wrong — not a rewritten file, just a bad
    /// value — needs no special case: it is refused the same way a mismatch
    /// is.
    #[test]
    fn a_malformed_digest_is_refused() {
        let path = scratch("verified-malformed-digest");
        let task = ElevatedTask {
            image: PathBuf::from("image.img"),
            image_digest: "a".repeat(64),
            device_id: r"\\.\PHYSICALDRIVE2".to_owned(),
            progress_path: std::env::temp_dir().join("p.progress"),
            result_path: std::env::temp_dir().join("p.result"),
        };
        task.write(&path).expect("a scratch directory is always writable");
        let why = read_task_verified(&path, "not-a-real-digest")
            .expect_err("a malformed digest must be refused");
        let _ = std::fs::remove_file(&path);
        assert!(why.contains("does not match"), "{why}");
    }

    // -----------------------------------------------------------------------
    // D19: the image itself is bound to a digest taken before the prompt too.
    // -----------------------------------------------------------------------

    /// **A task file with no image-digest line at all is refused as any
    /// short file is** — `AGENTS.md` §5, the same rule [`a_short_task_file_is_refused`]
    /// proves for the format in general, pinned here to the specific field.
    #[test]
    fn a_task_file_with_no_image_digest_line_is_refused() {
        let path = scratch("image-digest-missing-line");
        // Four lines only: image, device, progress, result - no digest line.
        let body = format!("image.img\n{}\np.progress\np.result\n", r"\\.\PHYSICALDRIVE2");
        std::fs::write(&path, body).expect("a scratch directory is always writable");
        let why = read_unverified(&path).expect_err("a task with no image-digest line must be refused");
        let _ = std::fs::remove_file(&path);
        assert!(why.contains("missing a line"), "{why}");
    }

    /// **A present but malformed image digest is refused before anything is
    /// written**, not defaulted or silently skipped — the same shape
    /// [`a_malformed_digest_is_refused`] proves for the task's own digest,
    /// pinned here to the image's.
    #[test]
    fn a_malformed_image_digest_is_refused() {
        let path = scratch("image-digest-malformed");
        let task = ElevatedTask {
            image: PathBuf::from("image.img"),
            image_digest: "not-a-real-digest".to_owned(),
            device_id: r"\\.\PHYSICALDRIVE2".to_owned(),
            progress_path: std::env::temp_dir().join("p.progress"),
            result_path: std::env::temp_dir().join("p.result"),
        };
        task.write(&path).expect("a scratch directory is always writable");
        let why = read_unverified(&path).expect_err("a malformed image digest must be refused");
        let _ = std::fs::remove_file(&path);
        assert!(why.contains("image"), "{why}");
        assert!(why.contains("digest"), "{why}");
    }

    /// An empty image-digest line is exactly as malformed as a garbled one —
    /// `looks_like_digest_hex` has no special case for it, and neither does
    /// this test need one.
    #[test]
    fn an_empty_image_digest_is_refused() {
        assert!(!looks_like_digest_hex(""));
        assert!(!looks_like_digest_hex("abc"));
        assert!(!looks_like_digest_hex(&"a".repeat(63)));
        assert!(!looks_like_digest_hex(&"A".repeat(64)), "uppercase is not this project's hex");
        assert!(looks_like_digest_hex(&"a".repeat(64)));
    }

    /// **The parent records the digest of the file it names.** [`compose_task`]
    /// is [`run_elevated`]'s only way to build a task, and this is what makes
    /// [`crate::write_image::write_bytes_and_verify`]'s own check meaningful:
    /// without an honest digest recorded here, there is nothing to compare a
    /// write-time digest against.
    #[test]
    fn compose_task_records_the_digest_of_the_image_it_names() {
        let image_path = scratch("compose-task-image");
        let content = b"pretend card image bytes, more than a handful of them";
        std::fs::write(&image_path, content).expect("write the image fixture");

        let task = compose_task(
            &image_path,
            r"\\.\PHYSICALDRIVE2",
            std::env::temp_dir().join("p.progress"),
            std::env::temp_dir().join("p.result"),
        )
        .expect("a readable image composes");

        let mut hasher = Blake3::new();
        let expected = rivetos_tls::encode_hex(&hasher.of(content));
        let _ = std::fs::remove_file(&image_path);

        assert_eq!(task.image_digest, expected);
        assert_eq!(task.image, image_path);
        assert!(looks_like_digest_hex(&task.image_digest));
    }

    /// An image that cannot be read is refused when the task is composed,
    /// not silently given an empty or placeholder digest.
    #[test]
    fn compose_task_refuses_an_image_it_cannot_read() {
        let missing = scratch("compose-task-missing-image");
        let why = compose_task(
            &missing,
            r"\\.\PHYSICALDRIVE2",
            std::env::temp_dir().join("p.progress"),
            std::env::temp_dir().join("p.result"),
        )
        .expect_err("an image that does not exist cannot be digested");
        assert!(why.contains("cannot open"), "{why}");
    }

    /// [`task_digest`] is deterministic and sensitive to every byte — the
    /// property the whole binding rests on.
    #[test]
    fn task_digest_changes_with_a_single_byte() {
        let a = task_digest(b"hello");
        let b = task_digest(b"hellp");
        assert_ne!(a, b, "a single changed byte must change the digest");
        assert_eq!(a, task_digest(b"hello"), "the same bytes must always digest the same way");
    }

    /// **The byte-order mark Windows PowerShell 5.1 writes**, which broke the
    /// first cancelled prompt this was ever run against.
    ///
    /// `-Encoding utf8` means *UTF-8 with a BOM* in 5.1 and *without* one in
    /// `pwsh` 7, so a reader that does not strip it works on one operator's
    /// machine and not another's. The symptom is the worst kind: the parent
    /// reporting that the child *"names an outcome this build does not
    /// recognise"* about a child that said exactly the right word.
    #[test]
    fn a_result_file_with_a_byte_order_mark_reads_as_the_tag_it_carries() {
        for (bytes, expected) in [
            (
                "\u{feff}DECLINED\nThe operation was canceled by the user",
                Outcome::Declined("The operation was canceled by the user".to_owned()),
            ),
            (
                "\u{feff}OK\nwrote 100 bytes and read every one back",
                Outcome::Success("wrote 100 bytes and read every one back".to_owned()),
            ),
        ] {
            let path = scratch("bom");
            std::fs::write(&path, bytes).expect("a scratch directory is always writable");
            let back = Outcome::read(&path)
                .expect("a mark is not a corrupt file")
                .expect("present");
            let _ = std::fs::remove_file(&path);
            assert_eq!(back, expected);
        }
    }

    /// **Every outcome the result file can carry**, round-tripped.
    #[test]
    fn every_outcome_round_trips_through_the_result_file() {
        for outcome in [
            Outcome::Success("wrote 100 bytes and read every one back\n".to_owned()),
            Outcome::Refused("NOTHING WAS WRITTEN. not removable.".to_owned()),
            Outcome::Failed("the card is NOT usable".to_owned()),
            Outcome::Declined("The operation was canceled by the user".to_owned()),
        ] {
            let path = scratch("outcome");
            outcome.write(&path).expect("a scratch directory is always writable");
            let back = Outcome::read(&path).expect("what was just written must read back").expect("present");
            let _ = std::fs::remove_file(&path);
            assert_eq!(outcome, back);
        }
    }

    /// An absent result file is `Ok(None)`, not an error — the
    /// live case while the elevated child (or the UAC prompt) is still
    /// running.
    #[test]
    fn a_missing_result_file_is_ok_none_not_an_error() {
        let path = scratch("missing");
        assert_eq!(Outcome::read(&path).expect("absence is not a failure"), None);
    }

    /// An outcome tag this build does not recognise is refused rather than
    /// silently read as a pass — `AGENTS.md` §7.
    #[test]
    fn an_unrecognised_tag_is_refused_not_read_as_a_pass() {
        let path = scratch("badtag");
        std::fs::write(&path, "MAYBE\nwho knows").expect("a scratch directory is always writable");
        let why = Outcome::read(&path).expect_err("an unknown tag must not silently pass");
        let _ = std::fs::remove_file(&path);
        assert!(why.contains("MAYBE"), "{why}");
    }

    /// A complete line parses.
    #[test]
    fn a_complete_progress_line_parses() {
        let line = parse_progress("write\t1048576\t2147483648\n").expect("must parse");
        assert_eq!(line.phase, "write");
        assert_eq!(line.done, 1_048_576);
        assert_eq!(line.total, 2_147_483_648);
    }

    /// **The truncated final line.** A second, torn line at the end of the
    /// text must not be trusted — the last COMPLETE line answers instead.
    #[test]
    fn a_truncated_final_line_falls_back_to_the_last_complete_one() {
        let text = "write\t1048576\t2147483648\nwrite\t20971"; // no trailing \n: torn
        let line = parse_progress(text).expect("the complete first line must still answer");
        assert_eq!(line.done, 1_048_576, "the torn second line must not be trusted");
    }

    /// A file holding only a torn line, with no complete line ever written
    /// yet, has nothing to report.
    #[test]
    fn a_file_with_no_complete_line_yet_reports_nothing() {
        assert_eq!(parse_progress("write\t512"), None);
        assert_eq!(parse_progress(""), None);
    }

    /// Multiple complete lines: the LAST one is current, not the first.
    #[test]
    fn the_most_recent_complete_line_wins() {
        let text = "write\t100\t1000\nwrite\t500\t1000\nverify\t200\t1000\n";
        let line = parse_progress(text).expect("must parse");
        assert_eq!(line.phase, "verify");
        assert_eq!(line.done, 200);
    }

    /// Reading a progress file through [`read_progress`] end to end,
    /// including a torn final append — the shape [`append_progress`]'s own
    /// caller relies on: read whatever is safely readable while a write to
    /// the same file may still be in flight.
    #[test]
    fn append_and_read_progress_round_trip() {
        let path = scratch("progress");
        append_progress(&path, "write", 100, 1000).expect("append");
        append_progress(&path, "write", 500, 1000).expect("append");
        let line = read_progress(&path).expect("must read something back");
        let _ = std::fs::remove_file(&path);
        assert_eq!(line.done, 500);
    }

    /// An absent progress file reads as `None`, not an error
    /// — the parent's very first poll, before the child has written anything.
    #[test]
    fn reading_a_progress_file_that_does_not_exist_yet_is_none() {
        let path = scratch("never-written");
        assert_eq!(read_progress(&path), None);
    }

    /// [`disk_number`], pure: the `Get-Disk`/`Set-Disk` number this project
    /// assumes matches the trailing digits of a `PHYSICALDRIVEn` identifier.
    #[test]
    fn disk_number_reads_the_trailing_digits() {
        assert_eq!(disk_number(r"\\.\PHYSICALDRIVE2"), Some(2));
        assert_eq!(disk_number(r"\\.\PHYSICALDRIVE17"), Some(17));
        assert_eq!(disk_number("/dev/sdb"), None, "a Linux id has no PHYSICALDRIVE number");
        assert_eq!(disk_number(r"\\.\PHYSICALDRIVE"), None, "no digits at all must not default to 0");
    }

    /// [`diskpart_clean_script`], pure: exactly the disk named, and nothing
    /// but `select` and `clean` — no `clean all`, no `format`, no second disk.
    #[test]
    fn the_diskpart_clean_script_selects_only_the_named_disk() {
        assert_eq!(diskpart_clean_script(2), "select disk 2\r\nclean\r\n");
        assert_eq!(diskpart_clean_script(17).lines().count(), 2);
    }

    /// [`parse_effective_uid`], pure — proven against a fabricated
    /// `/proc/self/status`-shaped string, since this project's Windows
    /// development host has no real `/proc` (`AGENTS.md` §10.5). Whether the
    /// real path answers this way is unproven until this runs on Linux.
    #[test]
    fn parse_effective_uid_reads_the_second_field() {
        let root = "Name:\tcargo-rivet\nUmask:\t0022\nState:\tR (running)\n\
                     Uid:\t1000\t0\t1000\t1000\nGid:\t1000\t1000\t1000\t1000\n";
        assert_eq!(parse_effective_uid(root), Some(0), "effective uid is the SECOND field");

        let unprivileged = "Uid:\t1000\t1000\t1000\t1000\n";
        assert_eq!(parse_effective_uid(unprivileged), Some(1000));

        assert_eq!(parse_effective_uid("Name:\tcargo-rivet\n"), None, "no Uid line at all");
    }

    /// [`powershell_quote`] escapes the one character that matters inside a
    /// single-quoted PowerShell literal, and leaves everything else alone —
    /// including a `$`, which must NOT be escaped, because the whole point of
    /// single-quoting is that PowerShell never interpolates inside it.
    #[cfg(windows)]
    #[test]
    fn powershell_quote_escapes_only_the_single_quote() {
        assert_eq!(powershell_quote("plain"), "'plain'");
        assert_eq!(powershell_quote("O'Brien"), "'O''Brien'");
        assert_eq!(
            powershell_quote(r"C:\Users\someone\AppData\Local\Temp\$weird.img"),
            r"'C:\Users\someone\AppData\Local\Temp\$weird.img'",
            "a literal $ must survive unescaped inside single quotes"
        );
    }

    // -----------------------------------------------------------------------
    // powershell.exe and diskpart.exe are resolved, never searched for on
    // PATH.
    // -----------------------------------------------------------------------

    /// [`resolve_system_binary`], pure: the full path under a given
    /// `SystemRoot`, and the `C:\Windows` fallback when it is absent —
    /// proven with no environment mutation, since `system_root` is a
    /// parameter rather than something this function reads for itself.
    #[cfg(windows)]
    #[test]
    fn resolve_system_binary_uses_the_given_system_root_or_falls_back() {
        assert_eq!(
            resolve_system_binary(Some(r"C:\Windows"), r"System32\WindowsPowerShell\v1.0\powershell.exe"),
            PathBuf::from(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"),
        );
        assert_eq!(
            resolve_system_binary(Some(r"D:\OtherRoot"), r"System32\diskpart.exe"),
            PathBuf::from(r"D:\OtherRoot\System32\diskpart.exe"),
            "a non-default SystemRoot must be honoured, not just C:\\Windows"
        );
        assert_eq!(
            resolve_system_binary(None, r"System32\diskpart.exe"),
            PathBuf::from(r"C:\Windows\System32\diskpart.exe"),
            "an absent SystemRoot must fall back to C:\\Windows"
        );
    }

    // -----------------------------------------------------------------------
    // The elevated child's argv survives shell metacharacters.
    // -----------------------------------------------------------------------

    /// [`elevated_argv`], pure: `write-image`, the verb, the task path, and
    /// the digest — in that order, and nothing else.
    #[cfg(windows)]
    #[test]
    fn elevated_argv_is_write_image_verb_task_and_digest_in_order() {
        let args = elevated_argv("--elevated-write", Path::new(r"C:\temp\a.task"), "abcd1234");
        assert_eq!(
            args,
            vec![
                "write-image".to_owned(),
                "--elevated-write".to_owned(),
                r"C:\temp\a.task".to_owned(),
                "abcd1234".to_owned(),
            ],
        );
    }

    /// The inverse of [`quote_windows_argument`], implementing the same
    /// argv-splitting rule the platform's own process start-up uses (and
    /// which `std::env::args` follows on Windows) — an ORACLE for the
    /// round-trip test below, not something production code needs: the
    /// elevated child recovers its own argv from the OS, it never parses a
    /// command line by hand. This exists only to prove
    /// [`quote_windows_argument`] agrees with what will actually read it
    /// back.
    #[cfg(windows)]
    fn parse_windows_command_line(line: &str) -> Vec<String> {
        let chars: Vec<char> = line.chars().collect();
        let n = chars.len();
        let is_space = |c: char| c == ' ' || c == '\t';
        let mut i = 0;
        while i < n && is_space(chars[i]) {
            i += 1;
        }
        let mut args = Vec::new();
        while i < n {
            let mut current = String::new();
            let mut in_quotes = false;
            while i < n {
                if chars[i] == '\\' {
                    let start = i;
                    while i < n && chars[i] == '\\' {
                        i += 1;
                    }
                    let count = i - start;
                    if i < n && chars[i] == '"' {
                        current.push_str(&"\\".repeat(count / 2));
                        if count % 2 == 1 {
                            current.push('"');
                        } else {
                            in_quotes = !in_quotes;
                        }
                        i += 1;
                    } else {
                        current.push_str(&"\\".repeat(count));
                    }
                    continue;
                }
                if chars[i] == '"' {
                    in_quotes = !in_quotes;
                    i += 1;
                    continue;
                }
                if !in_quotes && is_space(chars[i]) {
                    break;
                }
                current.push(chars[i]);
                i += 1;
            }
            args.push(current);
            while i < n && is_space(chars[i]) {
                i += 1;
            }
        }
        args
    }

    /// **The actual defect this repairs.** The old code hand-wrapped `verb`
    /// and `task_path` in literal `"..."` with no escaping, so a `"` inside a
    /// value closed the argument early and split it into pieces the
    /// elevated child would misparse. [`quote_windows_argument`] and
    /// [`parse_windows_command_line`] implement the same escaping the
    /// child's own `std::env::args` uses, and this proves they agree: a
    /// task path carrying a double quote, a space, a single quote and a
    /// backtick all at once still comes back out exactly as it went in.
    #[cfg(windows)]
    #[test]
    fn a_task_path_with_shell_metacharacters_round_trips_through_the_argument_line() {
        let nasty = Path::new(r#"C:\Temp\weird "task" it's a `card`.txt"#);
        let elements = elevated_argv("--elevated-write", nasty, "ab12cd34");
        let line = elements.iter().map(|a| quote_windows_argument(a)).collect::<Vec<_>>().join(" ");
        let parsed = parse_windows_command_line(&line);
        assert_eq!(
            parsed, elements,
            "the composite command line must parse back to exactly the elements that built it"
        );
    }

    // -----------------------------------------------------------------------
    // The declined-UAC path, and every other outcome — pure, no PowerShell,
    // no process, no interactive prompt. What an interactive UAC dialog MEANS
    // once cancelled can be proven in full even though driving the dialog
    // itself cannot be.
    // -----------------------------------------------------------------------

    /// **Cancelling the administrator prompt is reported as nothing written,
    /// not as an error someone has to interpret.** This is the whole of what
    /// `SPEC-05` §4 requires of that path: exit clean, no stack trace, and
    /// the operator's own reason for cancelling carried through verbatim.
    #[cfg(windows)]
    #[test]
    fn a_declined_uac_prompt_is_reported_as_nothing_written_not_an_error() {
        let outcome = Some(Outcome::Declined("The operation was canceled by the user".to_owned()));
        let report = interpret_outcome(outcome, "--elevated-write")
            .expect("a decline must be Ok, never Err — it is an ordinary outcome");
        assert!(report.starts_with("nothing was written"), "{report}");
        assert!(report.contains("canceled by the user"), "{report}");
    }

    /// The child's own refusal (not removable, or not fit) surfaces as an
    /// `Err`, carrying the child's exact wording rather than a generic one.
    #[cfg(windows)]
    #[test]
    fn a_refused_outcome_surfaces_as_an_error_with_the_childs_own_words() {
        let outcome = Some(Outcome::Refused("NOTHING WAS WRITTEN. not removable.".to_owned()));
        let why = interpret_outcome(outcome, "--elevated-write").expect_err("must be Err");
        assert!(why.contains("not removable"), "{why}");
    }

    /// A short write or a digest mismatch inside the child is a [`Failed`],
    /// and also surfaces as `Err`.
    ///
    /// [`Failed`]: Outcome::Failed
    #[cfg(windows)]
    #[test]
    fn a_failed_outcome_surfaces_as_an_error() {
        let outcome = Some(Outcome::Failed("the card is NOT usable".to_owned()));
        let why = interpret_outcome(outcome, "--elevated-write").expect_err("must be Err");
        assert!(why.contains("NOT usable"), "{why}");
    }

    /// A success carries the child's own write-and-verify report through
    /// unchanged — no re-wording, so the two cannot disagree.
    #[cfg(windows)]
    #[test]
    fn a_success_outcome_carries_the_report_through_unchanged() {
        let report = "wrote 100 bytes to X and read every one back — BLAKE3 abcd on both sides\n";
        let outcome = Some(Outcome::Success(report.to_owned()));
        assert_eq!(interpret_outcome(outcome, "--elevated-write").expect("must be Ok"), report);
    }

    /// **A missing result is treated as a failure, never as a silent pass** —
    /// `AGENTS.md` §7: an unreadable outcome and a true failure must look the
    /// same, both `Err`, so nobody trusts a card on the strength of silence.
    #[cfg(windows)]
    #[test]
    fn a_missing_result_is_an_error_not_a_silent_pass() {
        let why = interpret_outcome(None, "--elevated-write").expect_err("absence must be Err");
        assert!(why.contains("left no result file"), "{why}");
    }

    /// **The elevated child refuses a non-removable device it was told to
    /// write**, proven against this development host's own enumeration
    /// rather than a fixture — `AGENTS.md` §9 permits a READ-ONLY
    /// enumeration, and `attempt_elevated_write` never opens a disk at all
    /// once `refuse_unless_removable` has already said no: the image is
    /// never measured, `Set-Disk` is never called, nothing is written.
    /// `\\.\PHYSICALDRIVE0` is this machine's own fixed system disk — see
    /// this crate's safety rules — so either this host's enumeration finds it
    /// and refuses it as fixed, or (if CIM cannot see it under whatever
    /// account this test runs as) the elevated context's own enumeration does
    /// not see it at all and refuses it for THAT reason instead. Both are the
    /// same guard from two different doors, and both must refuse.
    #[cfg(windows)]
    #[test]
    fn the_elevated_child_refuses_a_real_non_removable_disk() {
        let task = ElevatedTask {
            image: PathBuf::from("this-path-need-not-exist.img"),
            image_digest: "a".repeat(64),
            device_id: r"\\.\PHYSICALDRIVE0".to_owned(),
            progress_path: scratch("na-progress"),
            result_path: scratch("na-result"),
        };
        match attempt_elevated_write(&task) {
            Outcome::Refused(_) => {}
            other => panic!(
                "PHYSICALDRIVE0 is this host's own fixed system disk (never removable) and must \
                 be refused by the elevated child's own re-check, got {other:?}"
            ),
        }
    }
}

#[cfg(test)]
mod macos_tests {
    use super::macos_whole_disk;

    /// Only a whole raw disk maps; a partition, a buffered node, or anything
    /// typed is refused rather than unmounted by accident.
    #[test]
    fn only_a_whole_raw_disk_names_a_disk_to_unmount() {
        assert_eq!(macos_whole_disk("/dev/rdisk4").as_deref(), Some("/dev/disk4"));
        assert_eq!(macos_whole_disk("/dev/rdisk12").as_deref(), Some("/dev/disk12"));
        for refused in ["/dev/rdisk4s1", "/dev/disk4", "/dev/rdisk", "rdisk4", "/dev/rdisk4 ", r"\\.\PHYSICALDRIVE2"] {
            assert_eq!(macos_whole_disk(refused), None, "{refused:?}");
        }
    }
}
