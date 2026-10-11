//! `cargo rivet write-image` — put a built image, carrying a configuration,
//! onto a card.
//!
//! **Renamed from `provision` to `write-image` 2026-09-13**
//! (`docs/PLAN-Feature-Set-2026-09-12.md` §3.0, §5.2 item B; `.rulings/`
//! R-162, Wave 3b). `provision` now names `cmd/cargo-rivet/src/provision.rs`,
//! the one-command orchestrator that runs the generated interview, composes
//! the document, selects or builds an image, and calls [`run()`] here to
//! write it — everything this module does is unchanged, and it is still exactly
//! what a person or a script can call directly with `--document PATH`
//! already in hand, skipping the interview entirely.
//!
//! # The gap this closes
//!
//! Until now, getting a document onto a card meant `sgdisk -p` to read the
//! offsets, `dd` to find the boot partition and `mcopy` to write into it —
//! `tools/update/build.sh`'s header lists the same incantations for the other
//! direction and exists for the same reason. This is that end being made right
//! for a person, which is `cmd/rivet-provision`'s argument one step later in
//! the story.
//!
//! # Four things this refuses to do, and each is deliberate
//!
//! **It never writes a device without being told twice.** Everything else in
//! this project that destroys something prints a plan and refuses without
//! `confirm` — `apply restart`, `apply install`, `cargo rivet install`. Writing
//! a whole-disk image over a block device is the most destructive thing any
//! tool here does, and it is the one that was reaching for `io::copy` with no
//! guard at all.
//!
//! **`confirm` alone is no longer the whole guard.** `AGENTS.md` §10.4:
//! *"naming the device is the guard."* So the plan names one, specific,
//! already-enumerated disk, and the operator is asked `Write it? [y/N]` —
//! default no, a bare Enter writes nothing — before Windows is ever told to
//! ask for administrator. `confirm` as the last word on the command line
//! skips that prompt and nothing else; the plan and the elevation line still
//! print, because the second is an explanation of the mechanism rather than a
//! second confirmation of the decision.
//!
//! **The guard that used to live in a retyped identifier now lives in the
//! elevated child instead**, and that is a strengthening rather than a
//! removal. Ruled by the maintainer 2026-09-09: the unelevated parent could
//! be wrong about which disk is which — a stale enumeration, a card swapped
//! for another during the confirmation pause — and no amount of typing or
//! clicking in that half guards against that. So the elevated child
//! `crate::elevate::run_elevated_child` re-enumerates on its own account and
//! refuses any device that is not removable, whatever the parent believed.
//! This is the check that cannot be clicked through, because it is not a
//! prompt at all — see [`crate::elevate`]'s module documentation for why the
//! write happens in a second, short-lived, elevated process rather than here.
//!
//! **It never writes a device the platform itself has not reported as
//! removable, or one smaller than the image.** [`crate::enumerate`] asks
//! Windows and Linux what each disk actually is — this module never inspects
//! a path and guesses, which is what made this claim false for as long as
//! [`crate::enumerate`] did not exist. **There is no flag that overrides the
//! removability refusal.** `SPEC-05` §4, `AGENTS.md` §10.4: an escape hatch
//! here is how a development host's own disk gets destroyed.
//!
//! **It never invents a partition layout.** `ADR-018` §4 fixes the five
//! partitions and `crate::gpt` reads them **by label** rather than by index,
//! for `tools/update/build.sh`'s stated reason: an image built by a future
//! layout would otherwise be silently mis-read. The first version of this file
//! read an MBR and indexed partitions 1 and 2, which on a GPT image is the
//! protective entry and an empty slot.
//!
//! # And the fifth, added 2026-09-05: it refuses a card nobody can reach
//!
//! **Two checks run before anything is written**, and the second is the one
//! that could only ever have been made here.
//!
//! The first is [`crate::credentials`]: the set of keys a card is built
//! *against* has to be on this host, and the refusal names each missing piece
//! and the command that makes it.
//!
//! The second reads the **image itself**. `cmd/rivet-image` writes an operator's
//! `peers` file onto the boot partition — the machine's whole answer to who may
//! reach its control plane — and this tool already opens that partition to put
//! a document in it. So it asks the only question that actually matters: *is
//! your operator key in the image you are about to write?* A card that answers
//! no is a board that boots, addresses itself, brings up the control plane, and
//! refuses you — headless, with no shell and no serial console, recoverable
//! only by writing the card again.
//!
//! **Neither check is a warning.** A warning is read after the card is written.
//!
//! # And the sixth, added 2026-09-09: it never trusts its own write
//!
//! `SPEC-05` §4: a write that returns `Ok` and produces an unbootable card
//! is the failure this exists to catch, because it costs a person a boot cycle
//! and a screen with nothing on it to discover otherwise. So the image is
//! hashed with BLAKE3 as it is written, the device is hashed the same way as
//! it is read back — streamed, `CHUNK` bytes at a time, so a multi-gigabyte
//! image is never held in memory twice — and the two digests are compared. A
//! mismatch is reported as *"the card is NOT usable,"* in those words, because
//! that is the one fact an operator staring at a black screen needs restated.
//!
//! # And the seventh, added 2026-09-09: it asks for administrator itself
//!
//! `Write it? [y/N]` runs in this, unelevated, process — where the operator
//! is. Raw whole-disk access does not: an unelevated shell is refused even a
//! *read* of `\\.\PHYSICALDRIVEn`, measured on the maintainer's own machine.
//! So once the operator has answered yes, this process spawns one
//! short-lived elevated child — `cargo-rivet` itself, under `write-image
//! --elevated-write`, launched from `current_exe()` — that does exactly the
//! write, the verify, and nothing else, then exits. [`crate::elevate`] is the
//! whole mechanism: the task handed to the child, the progress and result
//! files that stand in for the pipe `ShellExecuteEx`'s `runas` verb cannot
//! give it, and the two Windows-only steps — taking the disk offline before
//! writing so no filesystem driver writes behind this tool, and bringing it
//! back afterwards — that an operator cannot reasonably be expected to get
//! right by hand.
//!
//! **Cancelling the administrator prompt is an ordinary outcome, not an
//! error.** Somebody who read the plan and changed their mind at the last
//! click behaved correctly, and this reports *"nothing was written"* with no
//! stack trace, exactly as if they had answered `N` a moment earlier.
//!
//! **Linux is a different problem, not the same one.** There the answer is
//! already being root — this tool checks and refuses plainly if it is not,
//! rather than pretending Windows's mid-run hand-off has a Linux equivalent.

use crate::credentials::{self, Overrides, Survey};
use crate::enumerate::{self, Disk};
use crate::gpt;
use rivet_core::package::port::Digest;
use rivet_crypto::Blake3;
use std::fs::{File, OpenOptions};
use std::io::{IsTerminal, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// The label of the boot partition each slot's firmware reads — `ADR-018` §4.
///
/// **Both are written.** A card is provisioned once and may boot either slot
/// for the rest of its life; a document on only one of them is a machine that
/// forgets its configuration the first time it takes an update.
pub(crate) const BOOT_LABELS: [&str; 2] = ["boot-a", "boot-b"];

/// Bytes moved per read/write while writing and verifying a device —
/// `emit.rs` and `inspect.rs`'s own constant, matched here so a multi-
/// gigabyte image is never held in memory at once (`SPEC-05` §4).
const CHUNK: usize = 1024 * 1024;

/// What the firmware and `rivet-initd` look for on the boot partition.
pub(crate) const DOCUMENT_NAME: &str = "rivet.toml";

/// The two-line explanation printed after `Write it? [y/N] y` and before
/// Windows is asked for administrator — `SPEC-05` §4's ruled screen, verbatim.
/// Extracted to a constant so the wording used in the interview and the
/// wording asserted in this module's own tests cannot drift apart, and so a
/// change to it is a one-line diff rather than a hunt through `println!`s.
#[cfg(windows)]
const ELEVATION_EXPLAINER: &str = "\
Windows will now ask for administrator, because writing a whole disk needs it.\n\
Confirming that prompt starts the write; cancelling it writes nothing.\n";

/// The same explanation for `--verify-only`, and it exists because the one
/// above was **wrong** there.
///
/// A verification writes nothing, and printing *"confirming that prompt starts
/// the write"* before a read-only check is a sentence that would make somebody
/// cancel a safe operation — measured 2026-09-09, running `--verify-only`
/// unelevated and reading what it said. Raw whole-disk access needs
/// administrator for **reading** as well, which is the fact this has to
/// convey; on the same machine an unelevated shell was refused a read of
/// `\\.\PHYSICALDRIVE2` while volume handles opened for read and write.
#[cfg(windows)]
const ELEVATION_EXPLAINER_VERIFY: &str = "\
Windows will now ask for administrator, because reading a whole disk needs it.\n\
Nothing is written either way; cancelling the prompt just checks nothing.\n";

/// macOS's version of [`ELEVATION_EXPLAINER`]: the raw disk belongs to root,
/// and `sudo` asks in this terminal rather than in a dialog.
#[cfg(target_os = "macos")]
const ELEVATION_EXPLAINER: &str = "\
macOS needs administrator to write a whole disk, so sudo will ask for your password.\n\
Only the write runs as root; entering nothing, or a wrong password, writes nothing.\n";

#[cfg(target_os = "macos")]
const ELEVATION_EXPLAINER_VERIFY: &str = "\
macOS needs administrator to read a whole disk, so sudo will ask for your password.\n\
Nothing is written either way.\n";

#[derive(Debug, Clone)]
pub struct ProvisionArgs {
    pub image_path: Option<PathBuf>,
    pub document_path: Option<PathBuf>,
    pub device_path: Option<PathBuf>,
    /// The last word. Without it this prints a plan and writes nothing.
    pub confirm: bool,
    /// Write a card whose control plane will not answer you.
    ///
    /// **The escape hatch, and it is narrow on purpose.** A card built for
    /// somebody else's `peers` file is a real thing to want; a card built for
    /// nobody's is a mistake, and the two are indistinguishable without being
    /// told. `tools/card/build.sh`'s `RIVET_NO_TRUST` is the same shape and the
    /// same argument.
    pub not_mine: bool,
    /// Where `~/.rivet` is, for a cold run against a scratch directory.
    pub at: Option<PathBuf>,
    /// Skip the image and document steps entirely and only read back and hash
    /// `--device`, comparing it against `--image` — `SPEC-05` §4's verify
    /// step, standing alone. For the maintainer who wrote a card earlier and
    /// wants to check it again without writing it a second time.
    pub verify_only: bool,
    /// **Internal — do not type this.** How the elevated child
    /// `crate::elevate::run_elevated_child` is invoked: `Start-Process
    /// -Verb RunAs` relaunches this same binary under this flag, naming the
    /// task file the two halves communicate through. A person who types this
    /// by hand runs the write UNELEVATED and it will fail for exactly the
    /// reason `crate::elevate`'s module documentation opens with.
    pub elevated_write_task: Option<PathBuf>,
    /// [`Self::elevated_write_task`]'s sibling for `--verify-only`'s elevated
    /// read — same posture, same warning.
    pub elevated_verify_task: Option<PathBuf>,
    /// **Internal — do not type this.** The hex digest `crate::elevate`
    /// computed over the task file named above, at the moment it wrote it,
    /// and handed to PowerShell on the SAME command line as the task path —
    /// never inside the file, which a same-user process can rewrite during
    /// the seconds the UAC prompt is on screen. `crate::elevate::run_elevated_child`/
    /// `run_elevated_verify_child` refuse a task file that does not hash to
    /// this, `None` and an empty digest included: nothing about an elevated
    /// write is trusted unverified. `SPEC-05` §4.
    pub elevated_task_digest: Option<String>,
    /// **Wave C3 (`.rulings/` R-178 item 1).** The `ADR-049` staging bundle to
    /// widen `inject` with, beside the document's `rivet.toml` — `--staging
    /// PATH` when this is called directly, or `crate::provision::require_staging`'s
    /// own resolved path when `cargo rivet provision` is the caller. `None`
    /// for a stand-alone document, which carries no application service and
    /// so needs none.
    pub staging_path: Option<PathBuf>,
    /// **Wave C3.** This card's own identity directory — `device.seed`,
    /// `device.crt` and `peers`, `cargo rivet setup`'s own output — to widen
    /// `inject` with. `--identity PATH` when this is called directly, or
    /// `cmd/rivet-provision`'s own "Identity" answer, read off the
    /// interview's breadcrumb, when `cargo rivet provision` is the caller.
    /// `None` falls back to `credentials::Survey`'s own `BOARD_CREDENTIAL`
    /// default (`~/.rivet/identity/board`).
    pub identity_path: Option<PathBuf>,
    /// **Wave C3.** The fleet repository `[carry.<product>]` pins resolve
    /// against, and — for a fleet server's document — the repository P1
    /// carries onto the card. `--fleet-repository PATH` when this is called
    /// directly, or `cargo rivet provision`'s own already-resolved root.
    /// `None` falls back to `rivetos_fleet_repo::default_root()`.
    pub fleet_repo_root: Option<PathBuf>,
    /// **Not a flag.** Set by `cargo rivet provision` on a run that has a
    /// terminal (`.rulings/` R-174: the bare command ends in a written card).
    /// With no `--device` and no `confirm`, the removable devices are offered
    /// as a menu and `Write it? [y/N]` is asked, instead of stopping at a
    /// prepared image. `cargo rivet write-image` never sets it.
    pub offer_device_menu: bool,
    /// **Not a flag.** Set by `cargo rivet provision` when `image_path` is a
    /// working copy of an image selected from the fleet repository
    /// (`ADR-054`), which this checkout did not build. [`trust_note`]
    /// compares this checkout's `keys/*.pub` and says what a build from here
    /// would trust, which says nothing about a published image, so it is not
    /// printed then. `cargo rivet write-image` never sets it.
    pub image_from_repository: bool,
}

/// What [`run_outcome`] reports: its words, and whether a device was actually
/// written and verified. `cargo rivet provision` records a card only when
/// `written` is true — a declined `[y/N]`, a cancelled administrator prompt,
/// no removable device, or a prepared image alone all leave it false.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOutcome {
    pub report: String,
    pub written: bool,
}

impl WriteOutcome {
    fn not_written(report: String) -> Self {
        Self { report, written: false }
    }
}

/// Where a run's target device comes from — [`device_step`]'s answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeviceStep {
    /// `--device PATH` named one.
    Named,
    /// The removable devices are offered as a menu.
    Menu,
    /// No device: the image is prepared and nothing is written.
    NoDevice,
}

/// **The decision, pure.** `--device` wins; otherwise `confirm` or
/// [`ProvisionArgs::offer_device_menu`] offers the menu; otherwise nothing is
/// written. The `[y/N]` question is decided separately, by `confirm` alone.
pub(crate) fn device_step(device_named: bool, confirm: bool, offer_device_menu: bool) -> DeviceStep {
    if device_named {
        DeviceStep::Named
    } else if confirm || offer_device_menu {
        DeviceStep::Menu
    } else {
        DeviceStep::NoDevice
    }
}

pub fn usage() -> String {
    "usage: cargo rivet write-image [--image PATH] [--document PATH] [--device PATH]\n\
     \x20                              [--at DIR] [--not-mine] [confirm]\n\
     \x20                              [--verify-only]\n\
     \n\
     Injects a desired-state document into a built card image's boot partitions\n\
     and writes the result to a device. `cargo rivet provision` is the one-\n\
     command orchestrator that runs the interview, composes the document and\n\
     calls this verb; call this one directly when a document already exists.\n\
     \n\
     \x20 --image PATH      a card image from tools/card/build.sh. Omitted, the\n\
     \x20                   images in out/ are offered as a numbered menu\n\
     \x20 --document PATH   the document to inject. Omitted, the .toml files\n\
     \x20                   beside you are offered. rivet-provision writes one\n\
     \x20 --device PATH     where to write, as this host's own enumeration\n\
     \x20                   names it (\\\\.\\PHYSICALDRIVEn on Windows, /dev/sdX\n\
     \x20                   on Linux). Omitted with `confirm`, the removable\n\
     \x20                   devices this host can see are offered as a menu.\n\
     \x20                   Omitted without `confirm`, nothing is written and\n\
     \x20                   the image is left with the document in it\n\
     \x20 --at DIR          where ~/.rivet is, for the credential check below\n\
     \x20 --not-mine        proceed for a board SOMEBODY ELSE operates, whose\n\
     \x20                   peers file is theirs. You will not be able to reach\n\
     \x20                   the card this writes\n\
     \x20 confirm           Skips the `Write it? [y/N]` prompt below and answers\n\
     \x20                   it yes. The plan and the administrator line still\n\
     \x20                   print — this is not a way to write silently\n\
     \x20 --verify-only     Read --device back and compare it against --image.\n\
     \x20                   Nothing is written, no document is injected, and no\n\
     \x20                   credential or peers check runs — for checking a card\n\
     \x20                   an earlier run already wrote. --device omitted, the\n\
     \x20                   removable devices this host can see are offered as\n\
     \x20                   a menu, exactly as for a write\n\
     \x20 --staging PATH    an ADR-049 staging bundle to widen the injection\n\
     \x20                   with (`cargo rivet provision` supplies this itself)\n\
     \x20 --identity PATH   this card's own identity directory (device.seed,\n\
     \x20                   device.crt, peers — `cargo rivet setup`'s output).\n\
     \x20                   Omitted, credentials::Survey's own default is used\n\
     \x20 --fleet-repository PATH\n\
     \x20                   where [carry.<product>] pins resolve against.\n\
     \x20                   Omitted, RIVET_FLEET_REPOSITORY or ~/.rivet/fleet\n\
     \n\
     BEFORE ANYTHING IS WRITTEN, THREE THINGS ARE CHECKED and any one refuses:\n\
     \n\
     \x20 1  the credentials a card is built AGAINST are on this host.\n\
     \x20    `cargo rivet setup` makes them; `setup --check` lists them\n\
     \x20 2  the IMAGE's own peers file names your operator key, which is the\n\
     \x20    machine's whole answer to who may reach its control plane\n\
     \x20 3  the DOCUMENT parses, by the same parser the board refuses with.\n\
     \x20    A typo here is a card that boots and stops, on hardware with no\n\
     \x20    shell, and the only repair is to write the card again\n\
     \n\
     The document REPLACES whatever rivet.toml the image already carried.\n\
     \n\
     WRITING A DEVICE DESTROYS EVERYTHING ON IT. There is no undo and no trash.\n\
     A device this host's own enumeration does not report as removable is\n\
     refused, with no override — unplug and replug the card, or write it with\n\
     an external imager deliberately. The plan is printed and `Write it? [y/N]`\n\
     is asked (skipped by `confirm`); ONLY THEN does this ask Windows for\n\
     administrator — writing a whole disk needs it, and an unelevated shell is\n\
     refused even a READ of the raw device. The write itself happens in a\n\
     short-lived elevated helper that re-checks removability on its own\n\
     account, whatever this process believed; cancelling the administrator\n\
     prompt is an ordinary outcome and is reported as nothing written. Linux\n\
     needs no second process — it needs this one to already be root, and\n\
     refuses plainly if it is not.\n\
     \n\
     AFTER WRITING, THE DEVICE IS READ BACK AND HASHED. A digest that does not\n\
     match what was written is reported as the card being NOT usable. Where\n\
     Windows can, an UNELEVATED second check also confirms the document landed\n\
     on both boot partitions by their own drive letters.\n"
        .to_string()
}

pub fn parse(args: &[String]) -> Result<ProvisionArgs, String> {
    let mut out = ProvisionArgs {
        image_path: None,
        document_path: None,
        device_path: None,
        confirm: false,
        not_mine: false,
        at: None,
        verify_only: false,
        elevated_write_task: None,
        elevated_verify_task: None,
        elevated_task_digest: None,
        staging_path: None,
        identity_path: None,
        fleet_repo_root: None,
        offer_device_menu: false,
        image_from_repository: false,
    };
    let mut i = 0;
    while i < args.len() {
        let need = |i: usize, what: &str| -> Result<PathBuf, String> {
            args.get(i + 1)
                .map(PathBuf::from)
                .ok_or_else(|| format!("{what} needs a path after it"))
        };
        // [`ProvisionArgs::elevated_task_digest`]'s own parser — the digest
        // sits one further along than the task path `need` above reads.
        let need_digest = |i: usize, what: &str| -> Result<String, String> {
            args.get(i + 2)
                .cloned()
                .ok_or_else(|| format!("{what} needs a digest after its task path"))
        };
        match args[i].as_str() {
            "--image" => {
                out.image_path = Some(need(i, "--image")?);
                i += 2;
            }
            "--document" => {
                out.document_path = Some(need(i, "--document")?);
                i += 2;
            }
            "--device" => {
                out.device_path = Some(need(i, "--device")?);
                i += 2;
            }
            "--at" => {
                out.at = Some(need(i, "--at")?);
                i += 2;
            }
            "--not-mine" => {
                out.not_mine = true;
                i += 1;
            }
            "--verify-only" => {
                out.verify_only = true;
                i += 1;
            }
            // **Wave C3.** Recognised directly so `cargo rivet write-image`
            // works standalone, on the SAME footing `--document`/`--image`
            // already do — `cargo rivet provision` strips these three before
            // this parser ever sees them (`crate::provision`'s own
            // `extract_staging`/`extract_identity_and_pin`) and sets the
            // resolved values on `ProvisionArgs` directly instead.
            "--staging" => {
                out.staging_path = Some(need(i, "--staging")?);
                i += 2;
            }
            "--identity" => {
                out.identity_path = Some(need(i, "--identity")?);
                i += 2;
            }
            "--fleet-repository" => {
                out.fleet_repo_root = Some(need(i, "--fleet-repository")?);
                i += 2;
            }
            // **Internal.** See `ProvisionArgs::elevated_write_task`'s own
            // documentation for who invokes these (PowerShell, not a person).
            // Each takes TWO arguments — the task path, then the digest that
            // binds it (`ProvisionArgs::elevated_task_digest`) — not because
            // a person types either, but so a malformed or missing digest is
            // a parse refusal rather than a silent `None` that
            // `crate::elevate` would have to treat as "not verified."
            "--elevated-write" => {
                out.elevated_write_task = Some(need(i, "--elevated-write")?);
                out.elevated_task_digest = Some(need_digest(i, "--elevated-write")?);
                i += 3;
            }
            "--elevated-verify" => {
                out.elevated_verify_task = Some(need(i, "--elevated-verify")?);
                out.elevated_task_digest = Some(need_digest(i, "--elevated-verify")?);
                i += 3;
            }
            // **The last word, as everywhere else in this tool.** Accepted
            // anywhere so that a person who types it first is not refused for
            // word order, which would teach nothing.
            "confirm" => {
                out.confirm = true;
                i += 1;
            }
            other => {
                return Err(format!(
                    "{other:?} is not an option. {}",
                    usage().lines().next().unwrap_or_default()
                ));
            }
        }
    }
    Ok(out)
}

/// **`PLAN-Feature-Set-2026-09-12` §7.2's fourth agent-experience principle:
/// no interactive prompt without a terminal.**
///
/// Checked before a single byte is written to stdout or read from stdin, so a
/// run with no terminal — an agent, a script, anything piped — gets a
/// refusal naming its way past this exact point, rather than a menu it
/// cannot answer or, worse, a closed stdin silently read as an empty line and
/// misread as a choice (`choose`'s bare-Return default would otherwise take
/// the FIRST candidate for a run that never asked for it at all).
///
/// **The decision is a pure function of `is_terminal`**, taken as a parameter
/// rather than read here, so it is testable without this process's own
/// stdin deciding what a test asserts — `cargo test` itself may or may not
/// have a terminal, depending on who is running it and how, and a test whose
/// answer depends on that is not a test. [`choose`], [`choose_device`] and
/// [`write_and_verify`] are the callers that read the real value, each
/// through its own `_with` variant that takes it as a parameter instead.
///
/// # Errors
///
/// When `is_terminal` is false. `instead` is the flag that reaches this same
/// point without a prompt — the sentence names it so the refusal is a fix,
/// not a dead end.
fn refuse_unless_terminal(is_terminal: bool, what: &str, instead: &str) -> Result<(), String> {
    if is_terminal {
        return Ok(());
    }
    Err(format!(
        "this run has no terminal to ask about {what} at, so nothing was written and nothing \
         was read from stdin. {instead}"
    ))
}

/// What the operator did at a [`choose_with`] menu.
///
/// **`x`/`X` is offered on every menu this backs, and means the same thing on
/// every one of them: leave without choosing anything here.** What ending the
/// run that way actually costs — a card, a document, a Fleet record, nothing
/// at all — is the caller's to say, in the one line it prints once this is
/// seen; this type carries no wording of its own, on purpose, because it
/// backs menus whose stakes differ (choosing a document is not choosing a
/// removable device to overwrite).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Choice {
    /// The zero-based index of the chosen option.
    Selected(usize),
    /// `x` or `X`.
    Exit,
}

/// Offer a numbered menu and read a choice, or take the only candidate.
///
/// **One-based, and the default is the first entry**, which is the maintainer's
/// standing preference for a menu — `cmd/rivet-provision` is the reference. A
/// bare Return takes the default so the common case is one keypress.
///
/// `instead` names the flag that supplies `what` without a prompt.
///
/// # Errors
///
/// An empty list, input that is neither a number in range, empty nor `x`/`X`,
/// or no terminal to ask at (only when there is more than one candidate — see
/// below).
fn choose(what: &str, options: &[String], instead: &str) -> Result<Choice, String> {
    choose_with(std::io::stdin().is_terminal(), what, options, instead)
}

/// [`choose`], with whether this run has a terminal taken as a parameter —
/// see [`refuse_unless_terminal`] for why.
///
/// # Errors
///
/// As [`choose`].
pub(crate) fn choose_with(
    is_terminal: bool,
    what: &str,
    options: &[String],
    instead: &str,
) -> Result<Choice, String> {
    match options {
        [] => Err(format!("there are no {what} to choose from")),
        // **One candidate is not a question.** Asking it would train a person
        // to press Return without reading, which is the habit that makes the
        // confirmation in `write_device` worthless. Nothing is read from
        // stdin here, so this needs no terminal either — and no `x) exit`
        // line, because nothing was asked for it to decline.
        [only] => {
            println!("  {what}: {only}");
            Ok(Choice::Selected(0))
        }
        many => {
            refuse_unless_terminal(is_terminal, what, instead)?;
            println!("\n  {what}:");
            for (n, option) in many.iter().enumerate() {
                let marker = if n == 0 { " (default)" } else { "" };
                println!("    {}) {option}{marker}", n + 1);
            }
            println!("    x) exit — leave without choosing");
            print!("\n  choose 1-{} [1], or x: ", many.len());
            std::io::stdout().flush().ok();

            let mut line = String::new();
            std::io::stdin()
                .read_line(&mut line)
                .map_err(|e| format!("cannot read a choice: {e}"))?;
            parse_choice(&line, many.len())
        }
    }
}

/// [`choose_with`]'s parsing half, pure and unit-tested directly: a bare
/// Return takes the default (index 0), `x`/`X` asks to leave without
/// choosing, a number picks by position, and anything else is refused.
///
/// # Errors
///
/// Input that is neither empty, `x`/`X` (case-insensitively), nor a number
/// between 1 and `count`.
fn parse_choice(input: &str, count: usize) -> Result<Choice, String> {
    let typed = input.trim();
    if typed.is_empty() {
        return Ok(Choice::Selected(0));
    }
    if typed.eq_ignore_ascii_case("x") {
        return Ok(Choice::Exit);
    }
    let n: usize = typed
        .parse()
        .map_err(|_| format!("{typed:?} is not a number between 1 and {count} (or x to exit)"))?;
    if n == 0 || n > count {
        return Err(format!("{n} is not between 1 and {count}"));
    }
    Ok(Choice::Selected(n - 1))
}

/// Files under `dir` whose name ends in `suffix`, newest first.
///
/// Newest first because the card a person wants is almost always the one they
/// just built, and it makes the default right without asking them.
pub(crate) fn candidates(dir: &str, suffix: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.to_string_lossy().ends_with(suffix))
        .filter_map(|p| {
            let when = p.metadata().and_then(|m| m.modified()).ok()?;
            Some((when, p))
        })
        .collect();
    // Newest first, so `sort_by_key` on the time and then reverse rather than
    // a comparator — clippy is right that the comparator form hides which key
    // is being sorted on.
    found.sort_by_key(|(when, _)| *when);
    found.reverse();
    found.into_iter().map(|(_, p)| p).collect()
}

pub fn run(args: &ProvisionArgs) -> Result<String, String> {
    run_outcome(args).map(|outcome| outcome.report)
}

/// [`run`], also saying whether a device was actually written — see
/// [`WriteOutcome`].
///
/// # Errors
///
/// As [`run`].
pub fn run_outcome(args: &ProvisionArgs) -> Result<WriteOutcome, String> {
    // **The two internal entry points, checked first and unconditionally.**
    // These are how `Start-Process -Verb RunAs` re-invokes this same binary
    // elevated (`crate::elevate`'s module documentation) — nobody is expected
    // to type either flag, and reaching this branch means PowerShell already
    // did. Everything below this block is the ordinary, unelevated interview.
    if let Some(task_path) = &args.elevated_write_task {
        let digest = args.elevated_task_digest.as_deref().unwrap_or("");
        return run_elevated_write_child(task_path, digest).map(WriteOutcome::not_written);
    }
    if let Some(task_path) = &args.elevated_verify_task {
        let digest = args.elevated_task_digest.as_deref().unwrap_or("");
        return run_elevated_verify_child(task_path, digest).map(WriteOutcome::not_written);
    }

    // **`--verify-only` skips the whole interview.** No image is chosen from
    // a menu, no document is injected, no credential or enrollment check
    // runs — none of that applies to reading a card that was already
    // written. See [`run_verify_only`].
    if args.verify_only {
        return run_verify_only(args).map(WriteOutcome::not_written);
    }

    // **First, and before a menu is even offered.** A person who has chosen an
    // image and a document and is then told their keys are missing has spent
    // attention on the wrong thing; a refusal is worth most when it arrives
    // before the work does.
    let at = resolve_at(args)?;
    let survey = survey_for(args, &at)?;
    if let Some(why) = survey.refusal() {
        return Err(why);
    }

    let image = match &args.image_path {
        Some(p) => p.clone(),
        None => {
            let found = candidates("out", "-card.img");
            let names: Vec<String> = found.iter().map(|p| p.display().to_string()).collect();
            let n = match choose("card image", &names, "Pass --image <path>.") {
                Ok(Choice::Selected(n)) => n,
                Ok(Choice::Exit) => {
                    return Ok(WriteOutcome::not_written(
                        "exited — no card image was chosen, and nothing was written.\n".to_string(),
                    ));
                }
                Err(e) => {
                    return Err(format!(
                        "{e}. Build one with tools/container/build-card.sh, or pass --image"
                    ));
                }
            };
            found[n].clone()
        }
    };
    if !image.is_file() {
        return Err(format!("{} is not a file", image.display()));
    }

    let document = match &args.document_path {
        Some(p) => p.clone(),
        None => {
            let found = candidates(".", ".toml");
            let names: Vec<String> = found.iter().map(|p| p.display().to_string()).collect();
            let n = match choose("document", &names, "Pass --document <path>.") {
                Ok(Choice::Selected(n)) => n,
                Ok(Choice::Exit) => {
                    return Ok(WriteOutcome::not_written(
                        "exited — no document was chosen, and nothing was written.\n".to_string(),
                    ));
                }
                Err(e) => {
                    return Err(format!("{e}. cmd/rivet-provision writes one, or pass --document"));
                }
            };
            found[n].clone()
        }
    };
    let doc_bytes = std::fs::read(&document)
        .map_err(|e| format!("cannot read {}: {e}", document.display()))?;

    // **The third check, and the cheapest of the three.** `SPEC-05`'s History,
    // 2026-09-07: this read the file as bytes and wrote them to both boot
    // partitions with nothing looking at them, so a document with a typo in it
    // became a card that refuses on first boot — on hardware with no shell and
    // no serial console, where the only repair is to write the card again.
    let (parsed_document, note) = parsed_note(&document, &doc_bytes)?;
    refuse_unenforceable_cpu_cap(&document, &parsed_document)?;
    print!("{note}");

    // **A server or member document with no staging bundle has one built**
    // (`crate::provision::ensure_staging`) before anything below reads it.
    // `cargo rivet provision` has already resolved one and set
    // `staging_path`, so this fires for `write-image --document` alone.
    let widened;
    let args = if args.staging_path.is_none() && crate::provision::role_of(&parsed_document).is_some() {
        let identity =
            args.identity_path.clone().or_else(|| survey.named(credentials::BOARD_CREDENTIAL).map(|b| b.path.clone()));
        let (staging_path, built) = crate::provision::ensure_staging(
            &document,
            &parsed_document,
            None,
            identity.as_deref(),
            &at,
            &crate::provision::RealStagingBuilder,
        )?;
        print!("{built}");
        widened = ProvisionArgs { staging_path, ..args.clone() };
        &widened
    } else {
        args
    };

    // **Wave C3 (R-178 item 1): everything beyond rivet.toml this run
    // places** — the staging bundle, this card's own identity, and every
    // carried `app-<product>.rvt`, gathered BEFORE anything is written so a
    // refusal here (a missing pin, an unverifiable signature, a repository
    // too large for the staging cap) costs nothing already written.
    let extras = crate::carry::gather(&document, &parsed_document, &survey, args, &at)?;

    // **The check that could only be made here**, and it answers the one
    // question that decides whether this card is any use to the person
    // writing it: does the `peers` file THE CARD WILL CARRY name you?
    //
    // **Fixed 2026-09-14 (the gate rehearsal).** This read the IMAGE's own
    // `peers` and ran before `gather`. Since Wave C3 the card carries this
    // run's identity — `peers` included — on both boot partitions, replacing
    // whatever the image was built with, and a repository image is generic
    // (`R-175`) and carries none. So every real provision of a published image
    // was refused "carries NO peers file" after the interview, and a card whose
    // injected `peers` omitted the operator was written anyway. The image's
    // own file is read only when nothing is injected over it.
    let (enrollment, subject) = match extras.files.iter().find(|(name, _)| name == credentials::PEERS_FILE) {
        Some((_, bytes)) => {
            let from = survey
                .named(credentials::BOARD_CREDENTIAL)
                .map_or_else(|| PathBuf::from(credentials::PEERS_FILE), |b| b.path.join(credentials::PEERS_FILE));
            (enrollment_in(&String::from_utf8_lossy(bytes), &survey), EnrollmentSubject::Injected(from))
        }
        None => (enrollment_of(&image, &survey)?, EnrollmentSubject::Image(image.clone())),
    };
    if let Some(why) = enrollment.refusal_about(args.not_mine, &subject) {
        return Err(why);
    }

    print!("{}", enrollment.note_about(&subject));
    if !args.image_from_repository {
        print!("{}", trust_note(&survey));
    }
    print!("{}", extras.report);

    // **The document goes into the image, and the image is the artifact.**
    // Writing a device is a separate act below, and skipping it is a supported
    // outcome: an image with a document in it can be written by anything.
    inject(&image, &doc_bytes, &extras.files)?;
    // **"replacing" rather than "carries".** The second half of the same
    // finding: this overwrites whatever document the image was built with and
    // used to say nothing about it, so an operator injecting into an image that
    // already had one had no way to know which of the two the card would boot.
    println!(
        "--- {} now carries {} and {} more file(s), replacing any the image was built with, on \
         both boot partitions",
        image.display(),
        DOCUMENT_NAME,
        extras.files.len(),
    );

    // **`SPEC-05` §4's device-menu step.** `--device` names a choice already
    // made; omitting it with `confirm` — or on `cargo rivet provision`'s
    // interactive run (`offer_device_menu`) — is the same choice made from a
    // menu instead. Omitting it otherwise is the "just prepare the image"
    // mode. When this tool IS asked to write, it enumerates and asks rather
    // than trusting a typed path.
    let disk = match (device_step(args.device_path.is_some(), args.confirm, args.offer_device_menu), &args.device_path) {
        (DeviceStep::Named, Some(device)) => resolve_named_device(device)?,
        (DeviceStep::Menu, _) | (DeviceStep::Named, None) => match choose_removable_device()? {
            Some(disk) => disk,
            None => {
                return Ok(WriteOutcome::not_written(
                    "nothing was written: you exited the device menu. The image is ready to write \
                     later with --device, or by choosing a device from the menu next time.\n"
                        .to_owned(),
                ));
            }
        },
        (DeviceStep::NoDevice, _) => {
            return Ok(WriteOutcome::not_written(format!(
                "the image is ready at {}. Nothing was written to a device — pass --device, or \
                 confirm with no --device to choose a removable device from a menu, or write it \
                 with any imaging tool\n",
                image.display()
            )));
        }
    };

    write_and_verify(&image, &disk, args.confirm, &doc_bytes)
}

/// The internal entry point `--elevated-write TASK DIGEST` reaches — see
/// [`ProvisionArgs::elevated_write_task`]'s own documentation for who is
/// meant to type this (nobody) and who actually does (PowerShell).
///
/// **Windows-only in practice, and this compiles that as a refusal rather
/// than as dead code on every other platform**, so a person who somehow
/// reaches this flag on Linux gets a sentence instead of a build error naming
/// a function absent from that build.
fn run_elevated_write_child(task_path: &Path, digest: &str) -> Result<String, String> {
    #[cfg(any(windows, target_os = "macos"))]
    {
        crate::elevate::run_elevated_child(task_path, digest)?;
        Ok(String::new())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (task_path, digest);
        Err("--elevated-write is how Windows's administrator prompt and macOS's sudo relaunch \
             this binary; this build is neither, so it never sends itself here. SPEC-05 §4: \
             Linux is a different problem, not the same one."
            .to_owned())
    }
}

/// [`run_elevated_write_child`]'s sibling for `--elevated-verify TASK DIGEST`.
fn run_elevated_verify_child(task_path: &Path, digest: &str) -> Result<String, String> {
    #[cfg(any(windows, target_os = "macos"))]
    {
        crate::elevate::run_elevated_verify_child(task_path, digest)?;
        Ok(String::new())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = (task_path, digest);
        Err("--elevated-verify is how Windows's administrator prompt and macOS's sudo relaunch \
             this binary; this build is neither, so it never sends itself here."
            .to_owned())
    }
}

/// `cargo rivet write-image --device <id> --image <path> --verify-only` —
/// `SPEC-05` §4, standing alone. For the maintainer who wrote a card in an
/// earlier run and wants to check it again without writing it a second time.
///
/// # Errors
///
/// `--image` is missing or is not a file; `--device` names something this
/// host's own enumeration will not vouch for (or, with `--device` omitted, no
/// removable device is attached at all); or [`verify_elevated`]'s own errors —
/// ultimately [`verify_device_against_image`]'s short-read or digest-mismatch
/// refusals, every one of which says the card is **NOT usable**.
fn run_verify_only(args: &ProvisionArgs) -> Result<String, String> {
    let image = args
        .image_path
        .clone()
        .ok_or_else(|| "--verify-only needs --image PATH naming the image that was written, so \
                         there is something to compare the device against."
            .to_owned())?;
    if !image.is_file() {
        return Err(format!("{} is not a file", image.display()));
    }

    let disk = match &args.device_path {
        Some(device) => resolve_named_device(device)?,
        // Exiting the menu (or a rescan that still finds nothing, then exit)
        // is the "no device" case here — verify has nothing to check against.
        None => choose_removable_device()?
            .ok_or_else(|| "no removable device was chosen, so there is nothing to verify.".to_owned())?,
    };

    verify_elevated(&image, &disk)
}

// ---------------------------------------------------------------------------
// The two checks, 2026-09-05
// ---------------------------------------------------------------------------

/// **Wave C3**: `--at DIR`, or this host's own default — the one resolution
/// `survey_for` and `crate::carry::gather` (which needs it again, for
/// `<rivet root>/keys/rivet-app.pub`, R-178 P5) must never disagree about.
///
/// # Errors
///
/// A host with neither `HOME` nor `USERPROFILE` and no `--at`.
fn resolve_at(args: &ProvisionArgs) -> Result<PathBuf, String> {
    match &args.at {
        Some(p) => Ok(p.clone()),
        None => credentials::root().ok_or_else(|| {
            "this host sets neither HOME nor USERPROFILE, so there is nowhere for a Rivet \
             credential to be. Pass --at DIR, or set one."
                .to_owned()
        }),
    }
}

/// The credential survey for this run, honouring `--at` and the environment.
///
/// **Wave C3**: `args.identity_path` — `--identity PATH`, or `cargo rivet
/// provision`'s own already-resolved directory — wins over `RIVET_IDENTITY`
/// and the default, exactly the precedence `Overrides` already documents for
/// every other field: a caller who named a value on purpose is never told it
/// was ignored.
fn survey_for(args: &ProvisionArgs, at: &Path) -> Result<Survey, String> {
    let mut over = Overrides::from_environment();
    if args.identity_path.is_some() {
        over.board_identity = args.identity_path.clone();
    }
    Ok(Survey::take(at, &over))
}

/// **Hand the document to the parser a board refuses with, and refuse here
/// instead.**
///
/// `SPEC-05` §4, and its History for the defect this closes.
/// `cmd/rivet-provision`'s `w` key already reads back
/// what it wrote and parses it; this is the same check at the other end of the
/// journey, because the file that reaches a card need not be the file that tool
/// wrote — it can be hand-edited, copied off another machine, or picked from
/// the menu two directories away.
///
/// **It is `rivet_config::Document::parse` and not a look at the text**, which
/// is the whole point: only the schema knows, and a board
/// refusing at first boot is the same refusal arriving where nobody can read it.
///
/// # Errors
///
/// A document that is not UTF-8, or that the schema rejects — carrying the
/// schema's own words, and **before anything is written to the image**.
///
/// **Wave C3**: also returns the PARSED document, so `run` can hand it to
/// `crate::carry::gather` (item 1) rather than parsing it a second time —
/// one parse, two readers of its result, `ADR-011`.
fn parsed_note(path: &Path, bytes: &[u8]) -> Result<(rivet_config::Document, String), String> {
    let refusal = |why: String| {
        format!(
            "NOTHING WAS WRITTEN. {} is not a document this build can read: {why}\n\
             \n\
             This is the same parser the board refuses with (ADR-004), so a card built from \
             this file would boot, fail to read its own configuration, and stop — on hardware \
             with no shell and no serial console, where the only repair is to write the card \
             again.\n\
             \n\
             cmd/rivet-provision composes and checks one; `rivet-fleet export provisioning` \
             composes one from the register.",
            path.display(),
        )
    };

    let text = std::str::from_utf8(bytes)
        .map_err(|e| refusal(format!("it is not UTF-8 text ({e})")))?;
    let parsed = rivet_config::Document::parse(text).map_err(|why| refusal(format!("{why:?}")))?;

    // **Four facts rather than a tick**, as `SPEC-05` §4 requires of the other
    // end: a person about to destroy a card wants to see the hostname they
    // expected and a control-plane address they can reach, not the word `ok`.
    let note = format!(
        "--- {} parsed by rivet-config, the same parser the board refuses with\n\
         \x20      hostname       {}\n\
         \x20      interfaces     {}\n\
         \x20      services       {}\n\
         \x20      control plane  {}\n",
        path.display(),
        parsed
            .hostname()
            .map_or_else(|| "(derived by the machine)".to_owned(), ToString::to_string),
        parsed.network().len(),
        parsed.services().len(),
        parsed.effective_control_plane().listen(),
    );
    Ok((parsed, note))
}

/// **A `cpu` cap the kernel will not keep is refused before anything is
/// written** — `.rulings/` R-210 D22.
///
/// Unlike [`parsed_note`]'s own refusal, the document DOES parse here and a
/// board WOULD accept it — `rivet_config::Document::parse` and
/// `rivet_core::service::CpuLimit::parse` are both unchanged. cgroup v2 alone
/// refuses the cap, silently, and `rivet-sysd` journals that refusal on every
/// start — on the same hardware `parsed_note`'s own documentation describes,
/// with no shell and no serial console, where the only way to learn why a
/// service never got its share is to already know to go looking.
fn refuse_unenforceable_cpu_cap(path: &Path, document: &rivet_config::Document) -> Result<(), String> {
    for service in document.services() {
        let Some(limit) = service.cpu() else { continue };
        if limit.is_below_kernel_minimum() {
            return Err(format!(
                "NOTHING WAS WRITTEN. {}: [services.{}] cpu = \"{limit}\" — {}",
                path.display(),
                service.name(),
                limit.kernel_refusal(),
            ));
        }
    }
    Ok(())
}

/// **What the image's own `peers` file says about you.**
///
/// `cmd/rivet-image` writes three files onto the boot partition when it is
/// given `--identity`: `device.seed`, `device.crt` and `peers`. The last is the
/// machine's whole answer to who may reach its control plane (`ADR-025` §2.3),
/// and it is the only one of the three that has to name *this* workstation.
#[derive(Debug, PartialEq, Eq)]
pub enum Enrollment {
    /// The image carries a `peers` file and your operator key is in it.
    Names,
    /// It carries one, and your key is not among the `usize` keys in it.
    Excluded(usize),
    /// The image carries no `peers` file at all, so no workstation can reach
    /// it. `rivet-mcpd` will not start (`tools/card/build.sh`'s own note).
    Absent,
    /// The check could not be made, and says why rather than passing.
    ///
    /// **Not silently a pass.** `AGENTS.md` §7: a probe that cannot tell an
    /// error from a true negative must report which it had.
    Unknown(String),
}

/// **Whose `peers` file [`Enrollment`] was read from** — the gate's fix of
/// 2026-09-14. The card carries exactly one of the two, and the refusal has to
/// name the one that is actually at fault and the repair that applies to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnrollmentSubject {
    /// The image's own `boot-a/peers` — read only when this run injects no
    /// identity over it.
    Image(PathBuf),
    /// The identity directory's `peers`, which this run places on both boot
    /// partitions, replacing the image's.
    Injected(PathBuf),
}

impl Enrollment {
    /// The refusal, unless the operator said they were building for somebody
    /// else — about the image's own `peers` file.
    #[cfg(test)]
    fn refusal(&self, not_mine: bool, image: &Path) -> Option<String> {
        self.refusal_about(not_mine, &EnrollmentSubject::Image(image.to_path_buf()))
    }

    /// The refusal, unless the operator said they were building for somebody
    /// else, naming whichever `peers` file the card will actually carry.
    fn refusal_about(&self, not_mine: bool, subject: &EnrollmentSubject) -> Option<String> {
        let detail = match self {
            Self::Names | Self::Unknown(_) => return None,
            Self::Absent => "carries NO peers file. rivet-mcpd will not start on it at all, so \
                             the card has no control plane and there is no way in"
                .to_owned(),
            Self::Excluded(count) => format!(
                "carries a peers file naming {count} key(s), and none of them is yours"
            ),
        };
        if not_mine {
            return None;
        }
        let (what, repair) = match subject {
            EnrollmentSubject::Image(image) => (
                format!("{}", image.display()),
                "Build the image with RIVET_IDENTITY pointing at a directory whose peers file \
                 names your operator key. `cargo rivet setup` writes both, and `cargo rivet \
                 setup --check` says where they are."
                    .to_owned(),
            ),
            EnrollmentSubject::Injected(peers) => (
                format!(
                    "The card this run would write — its peers file comes from {}, which",
                    peers.display()
                ),
                "Add your operator key to that peers file — `cargo rivet identity show \
                 --directory <rivet root>/identity/operator` prints the line that belongs in \
                 it — or name a different identity directory (the interview's Identity field, \
                 or --identity PATH). `cargo rivet setup` writes a board identity whose peers \
                 file names you."
                    .to_owned(),
            ),
        };
        Some(format!(
            "NOTHING WAS WRITTEN. {what} {detail}.\n\
             \n\
             The board this card makes will boot, take its address and bring up its control \
             plane, and then refuse you — on hardware with no shell and no serial console. The \
             only repair is to write the card again, so this refuses now instead.\n\
             \n\
             {repair}\n\
             \n\
             Pass --not-mine if this card is genuinely for a board somebody else operates.",
        ))
    }

    /// The line printed when the check passed, or could not be made — about
    /// the image's own `peers` file.
    #[cfg(test)]
    fn note(&self) -> String {
        self.note_about(&EnrollmentSubject::Image(PathBuf::new()))
    }

    /// The line printed when the check passed, or could not be made, naming
    /// whichever `peers` file the card will carry.
    fn note_about(&self, subject: &EnrollmentSubject) -> String {
        let whose = match subject {
            EnrollmentSubject::Image(_) => "the image's peers file".to_owned(),
            EnrollmentSubject::Injected(peers) => {
                format!("the peers file this card will carry ({})", peers.display())
            }
        };
        match self {
            Self::Names => format!("--- {whose} names your operator key\n"),
            Self::Absent => format!("--- {whose} is ABSENT: nobody can reach this card\n"),
            Self::Excluded(count) => format!(
                "--- {whose} names {count} key(s) and none is yours: you will not be able to \
                 reach this card\n"
            ),
            Self::Unknown(why) => format!(
                "--- the peers check could NOT be made, so nothing here says this card is \
                 reachable: {why}\n"
            ),
        }
    }
}

/// Read `peers` off the image's first boot partition and look for the operator.
///
/// **`boot-a` only.** `cmd/rivet-image` writes the identity to both, and a
/// disagreement between them is not a thing this project can produce; reading
/// one is the check, and reading two would be a claim about media integrity
/// this tool is in no position to make — `crate::gpt`'s own argument about CRCs.
///
/// # Errors
///
/// Only what stops the image being opened at all. Everything narrower is an
/// [`Enrollment`], because *"there is no peers file"* is an answer and not a
/// failure.
fn enrollment_of(image: &Path, survey: &Survey) -> Result<Enrollment, String> {
    if let Err(unknown) = operator_key(survey) {
        return Ok(unknown);
    }

    let mut file = File::open(image).map_err(|e| format!("cannot open {}: {e}", image.display()))?;
    let parts = gpt::read(&mut file)?;
    let part = gpt::by_label(&parts, BOOT_LABELS[0])?;
    let mut view = Partition::new(&mut file, part.offset_bytes(), part.len_bytes());
    let fs = fatfs::FileSystem::new(&mut view, fatfs::FsOptions::new())
        .map_err(|e| format!("{} is not a readable FAT filesystem: {e}", BOOT_LABELS[0]))?;

    let mut text = String::new();
    match fs.root_dir().open_file(credentials::PEERS_FILE) {
        Err(_) => return Ok(Enrollment::Absent),
        Ok(mut peers) => {
            if let Err(why) = peers.read_to_string(&mut text) {
                return Ok(Enrollment::Unknown(format!(
                    "the image's peers file could not be read: {why}"
                )));
            }
        }
    }

    Ok(enrollment_in(&text, survey))
}

/// The operator's own public key, or the [`Enrollment::Unknown`] that says why
/// there is none to compare against.
fn operator_key(survey: &Survey) -> Result<String, Enrollment> {
    let Some(operator) = survey.named(credentials::OPERATOR_CREDENTIAL) else {
        return Err(Enrollment::Unknown(
            "this build has no operator credential in its own survey".to_owned(),
        ));
    };
    operator.public_key().map_err(Enrollment::Unknown)
}

/// **Does this `peers` text name the operator?** — the comparison itself,
/// whichever file it came from: the image's own (`enrollment_of`), or the one
/// this run injects over it (`run`, since the gate's fix of 2026-09-14).
fn enrollment_in(text: &str, survey: &Survey) -> Enrollment {
    let mine = match operator_key(survey) {
        Ok(key) => key,
        Err(unknown) => return unknown,
    };
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    if lines.iter().any(|l| l.eq_ignore_ascii_case(&mine)) {
        Enrollment::Names
    } else {
        Enrollment::Excluded(lines.len())
    }
}

/// Read `rivet.toml` off the image's first boot partition, as bytes.
///
/// **`boot-a` only** — [`enrollment_of`]'s same reasoning: `cmd/rivet-image`
/// writes the document to both, and reading one is the check.
///
/// `Ok(None)` when the boot partition has no document at all, which
/// `COMMANDS.md` records as a real state: *"a card built with no document
/// carries a template declaring a schema version and nothing else."* That is
/// this function's caller's problem (`cargo rivet image` refuses to publish an
/// image with nothing to read a schema version off), not this one's — reading
/// the GPT and the FAT filesystem is the part that belongs here, exactly as it
/// does in [`enrollment_of`].
///
/// # Errors
///
/// Only what stops the image being opened or its boot partition being read as
/// a filesystem at all.
pub(crate) fn read_boot_toml(image: &Path) -> Result<Option<Vec<u8>>, String> {
    let mut file = File::open(image).map_err(|e| format!("cannot open {}: {e}", image.display()))?;
    let parts = gpt::read(&mut file)?;
    let part = gpt::by_label(&parts, BOOT_LABELS[0])?;
    let mut view = Partition::new(&mut file, part.offset_bytes(), part.len_bytes());
    let fs = fatfs::FileSystem::new(&mut view, fatfs::FsOptions::new())
        .map_err(|e| format!("{} is not a readable FAT filesystem: {e}", BOOT_LABELS[0]))?;
    match fs.root_dir().open_file(DOCUMENT_NAME) {
        Err(_) => Ok(None),
        Ok(mut document) => {
            let mut bytes = Vec::new();
            document
                .read_to_end(&mut bytes)
                .map_err(|e| format!("cannot read {DOCUMENT_NAME} off {}: {e}", BOOT_LABELS[0]))?;
            Ok(Some(bytes))
        }
    }
}

/// **The note about the trust store, which cannot be a refusal.**
///
/// A card's trust store lives on the read-only `/system` slot (`ADR-028`
/// §5.5.4) and is ext4, so nothing in this tool can read it — `crate::gpt` and
/// `fatfs` reach the FAT boot partitions and no further. What *can* be compared
/// is this checkout's committed `keys/*.pub`, which is what `tools/card/build.sh`
/// composes the store from unless it was told otherwise, against the seeds this
/// host holds.
///
/// So it says what it compared and stays a note. Refusing on a guess about a
/// build that has already happened would be `AGENTS.md` §5's *"report what the
/// system said, not what it was expected to say"* pointed the wrong way.
fn trust_note(survey: &Survey) -> String {
    let mut out = String::new();
    for (credential, published, variable) in [
        ("system signing seed", "keys/rivet-system.pub", "RIVET_TRUSTED_KEY"),
        ("application signing seed", "keys/rivet-app.pub", "RIVET_APP_KEY"),
    ] {
        let Some(seed) = survey.named(credential) else {
            continue;
        };
        let published = Path::new(published);
        if !seed.present || !published.is_file() {
            continue;
        }
        match credentials::agrees(&seed.path, published) {
            Ok(true) | Err(_) => {}
            Ok(false) => out.push_str(&format!(
                "--- NOTE: {published} is not your {credential}'s public half. A card built \
                 from this checkout without {variable} set trusts that key and not yours, and \
                 will refuse everything you sign. This tool cannot read the store off the \
                 image to be sure — /system is ext4 and only the FAT partitions are reachable \
                 from here.\n",
                published = published.display(),
            )),
        }
    }
    out
}

/// Put the document on every boot partition the image has.
///
/// **Wave C3 widens this to more than `rivet.toml`.** `crate::carry::gather`
/// collects everything else — the `ADR-049` staging bundle, this card's own
/// identity, and every carried `app-<product>.rvt` — under the exact names
/// `cmd/rivet-image` uses for the same inputs, and `extras` is that list.
/// `inject_all` places all of it, `doc` included, so first boot needs no
/// change (`.rulings/` R-178 item 1).
///
/// **Refuses by name, before writing anything, if a file will not fit.**
/// Checked against EACH partition's own free space — `fatfs::FileSystem::stats`,
/// read before the first `create_file` — because a short write mid-injection
/// would leave a partition with some of these files and not others, which is
/// worse than refusing before any of them land.
///
/// # Errors
///
/// The image cannot be opened or its boot partitions read as GPT/FAT; a name
/// this FAT filesystem cannot hold (`ADR-018` §4's flat-partition constraint,
/// `rivet_core::carried`/`staging`'s own names all satisfy it, so this is
/// defensive); or the files, summed, will not fit the partition's own free
/// space — named with the byte counts, before either partition is touched.
fn inject(image: &PathBuf, doc: &[u8], extras: &[(String, Vec<u8>)]) -> Result<(), String> {
    let mut files: Vec<(&str, &[u8])> = vec![(DOCUMENT_NAME, doc)];
    files.extend(extras.iter().map(|(name, bytes)| (name.as_str(), bytes.as_slice())));

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(image)
        .map_err(|e| format!("cannot open {}: {e}", image.display()))?;

    let parts = gpt::read(&mut file)?;
    for label in BOOT_LABELS {
        let part = gpt::by_label(&parts, label)?;
        let mut view = Partition::new(&mut file, part.offset_bytes(), part.len_bytes());
        let fs = fatfs::FileSystem::new(&mut view, fatfs::FsOptions::new())
            .map_err(|e| format!("{label} is not a readable FAT filesystem: {e}"))?;

        // **Refuse by name before writing a single byte to THIS partition.**
        // `stats()` reads the FAT's own free-cluster count — the filesystem's
        // own answer, not an estimate from the partition's raw byte length,
        // which would ignore FAT's directory and cluster-rounding overhead.
        let stats = fs
            .stats()
            .map_err(|e| format!("cannot read {label}'s own free space: {e}"))?;
        let available = u64::from(stats.free_clusters()) * u64::from(stats.cluster_size());
        let needed: u64 = files.iter().map(|(_, bytes)| bytes.len() as u64).sum();
        if needed > available {
            return Err(format!(
                "{label} has {available} bytes free and this run would write {needed} bytes to \
                 it ({} file(s): {}) — nothing was written to either boot partition. A fresh \
                 card build carries the same files from the start; this image's boot partition \
                 is smaller than usual, or is carrying more applications than it has room for.",
                files.len(),
                files.iter().map(|(name, bytes)| format!("{name} ({} bytes)", bytes.len())).collect::<Vec<_>>().join(", "),
            ));
        }

        let root = fs.root_dir();
        for (name, bytes) in &files {
            let mut out =
                root.create_file(name).map_err(|e| format!("cannot create {name} on {label}: {e}"))?;
            out.truncate().map_err(|e| format!("cannot replace {name} on {label}: {e}"))?;
            out.write_all(bytes).map_err(|e| format!("cannot write {name} on {label}: {e}"))?;
        }
    }
    Ok(())
}

/// **A window onto one partition, so `fatfs` sees a filesystem and not a disk.**
///
/// This is not a convenience. `fatfs` seeks from the stream's own origin, so
/// handing it the whole image after seeking to a partition makes it parse the
/// GPT header as a FAT boot sector — which is the second defect the first
/// version of this file had, independent of reading the wrong table.
pub(crate) struct Partition<'a> {
    file: &'a mut File,
    start: u64,
    len: u64,
    at: u64,
}

impl<'a> Partition<'a> {
    pub(crate) fn new(file: &'a mut File, start: u64, len: u64) -> Self {
        Self {
            file,
            start,
            len,
            at: 0,
        }
    }
}

impl Read for Partition<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.len.saturating_sub(self.at);
        if left == 0 {
            return Ok(0);
        }
        let want = buf.len().min(left as usize);
        self.file.seek(SeekFrom::Start(self.start + self.at))?;
        let n = self.file.read(&mut buf[..want])?;
        self.at += n as u64;
        Ok(n)
    }
}

impl Write for Partition<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let left = self.len.saturating_sub(self.at);
        if left == 0 {
            // **Short-write rather than growing.** A filesystem that ran out of
            // partition must not spill into the next one.
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "the partition is full",
            ));
        }
        let want = buf.len().min(left as usize);
        self.file.seek(SeekFrom::Start(self.start + self.at))?;
        let n = self.file.write(&buf[..want])?;
        self.at += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

impl Seek for Partition<'_> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let to = match pos {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::Current(d) => self.at as i64 + d,
            SeekFrom::End(d) => self.len as i64 + d,
        };
        if to < 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "seek before the start of the partition",
            ));
        }
        self.at = to as u64;
        Ok(self.at)
    }
}

// ---------------------------------------------------------------------------
// SPEC-05 §4, 2026-09-09: which disk, and proving what was written
// ---------------------------------------------------------------------------

/// `--device PATH` was given: find it in this host's own enumeration, and
/// refuse anything the enumeration cannot vouch for.
///
/// **This is the whole of requirement #2 from the module header.** A path
/// that used to be trusted outright is now looked up, and there are exactly
/// two ways to fail the lookup: the enumeration cannot see it at all, or it
/// can see it and it is not removable. Neither has an override.
///
/// # Errors
///
/// `crate::enumerate::all` failing outright; the device not being in the
/// list this host can see; the device being in the list and not removable.
fn resolve_named_device(device: &Path) -> Result<Disk, String> {
    let disks = enumerate::all()?;
    let typed = device.to_string_lossy();
    let Some(found) = disks.iter().find(|d| d.id == typed) else {
        let seen: Vec<&str> = disks.iter().map(|d| d.id.as_str()).collect();
        return Err(format!(
            "NOTHING WAS WRITTEN. {typed} is not a disk this host's enumeration can see, so its \
             removability cannot be confirmed — SPEC-05 §4 gives that check no bypass. \
             Disks seen: {}",
            if seen.is_empty() { "(none)".to_owned() } else { seen.join(", ") },
        ));
    };
    refuse_unless_removable(found)?;
    Ok(found.clone())
}

/// `--device` was omitted and the menu is offered ([`device_step`]): ask
/// which removable disk, exactly as `SPEC-05` §4 requires when this tool is
/// the one choosing.
///
/// `Ok(None)` when no removable disk is attached and `confirm` was NOT given
/// — `cargo rivet provision`'s interactive run, which reports that nothing was
/// written and names the command that writes the card later.
///
/// # Errors
///
/// Enumeration failing outright, or no removable disk being attached when
/// `confirm` was given — named rather than silently falling through to
/// "nothing was written", because an operator who typed `confirm` meant to
/// write something.
fn choose_removable_device() -> Result<Option<Disk>, String> {
    // **Enumeration is inside the loop, because that is what `Rescan` is.** A
    // fresh `enumerate::all()` every pass is how a card plugged in after the
    // menu appeared becomes visible without restarting the whole interview —
    // and an empty list is no longer a dead end, only a pass where the operator
    // rescans or exits. `Ok(None)` is an operator who chose `x`.
    loop {
        let disks = enumerate::all()?;
        let removable: Vec<Disk> = disks.into_iter().filter(|d| d.removable).collect();
        let names: Vec<String> = removable.iter().map(|d| d.describe()).collect();
        match choose_device(&names)? {
            DeviceChoice::Selected(n) => return Ok(Some(removable[n].clone())),
            DeviceChoice::Rescan => continue,
            DeviceChoice::Exit => return Ok(None),
        }
    }
}

/// A device the enumeration did not report as removable is refused, naming
/// the repair — `SPEC-05` §4: *"there is no flag that overrides it."*
///
/// `pub(crate)` so `crate::elevate::attempt_elevated_write` can run the
/// identical check inside the elevated child — see that module's header for
/// why this check has to run TWICE rather than once in the unelevated half.
pub(crate) fn refuse_unless_removable(disk: &Disk) -> Result<(), String> {
    if disk.removable {
        return Ok(());
    }
    Err(format!(
        "NOTHING WAS WRITTEN. {} was NOT reported as removable by this host's enumeration \
         ({}). Writing a disk this host thinks is fixed is how a development host's own disk \
         gets destroyed (AGENTS.md §10.4), and there is no flag that overrides this. Unplug \
         and replug the card and run this again — enumeration is read fresh every run — or \
         write it with an external imaging tool deliberately.",
        disk.id,
        disk.describe(),
    ))
}

/// The device menu, `SPEC-05` §4.
///
/// **Deliberately not [`choose`].** Every other menu in this file takes a
/// bare Return as "the first one" — right for an image or a document, where
/// picking the wrong one costs a re-run. Here it is a block device, and
/// `SPEC-05` says the tool "MUST NOT ... default to one." An operator
/// picks a number, on purpose, every time; there is no default to fall
/// through to.
///
/// # Errors
///
/// An empty list, naming what was wanted; input that is not a number in
/// range — including an empty line, which elsewhere in this file takes the
/// default and here has none to take.
fn choose_device(options: &[String]) -> Result<DeviceChoice, String> {
    choose_device_with(std::io::stdin().is_terminal(), options)
}

/// What the operator did at the device menu: picked a device, asked to
/// [`Rescan`](DeviceChoice::Rescan) after plugging one in, or [`Exit`](DeviceChoice::Exit).
///
/// **`Rescan` and `Exit` are letters, not numbers**, so they can never be
/// confused with a device index — `SPEC-05` §4's "pick a number, on purpose,
/// no default" still holds for the *devices*, and the two escape hatches sit
/// beside them as `r` and `x`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeviceChoice {
    /// The zero-based index of the chosen device in the list as shown.
    Selected(usize),
    /// Re-enumerate: the operator plugged a card in (or pulled the wrong one)
    /// and wants a fresh list.
    Rescan,
    /// Leave the menu without writing anything.
    Exit,
}

/// [`choose_device`], with whether this run has a terminal taken as a
/// parameter — see [`refuse_unless_terminal`] for why. The list may be empty:
/// with `r` and `x` always offered, "no removable device yet" is a state the
/// operator escapes by plugging one in and rescanning, not a dead end.
///
/// # Errors
///
/// No terminal to ask at, or an answer [`parse_device_choice`] cannot read.
fn choose_device_with(is_terminal: bool, options: &[String]) -> Result<DeviceChoice, String> {
    refuse_unless_terminal(is_terminal, "which removable device", "Pass --device <id>.")?;
    if options.is_empty() {
        println!("\n  removable devices: none this host can see yet.");
    } else {
        println!("\n  removable devices:");
        for (n, option) in options.iter().enumerate() {
            println!("    {}) {option}", n + 1);
        }
    }
    println!("    r) rescan — plug a card in, then choose this");
    println!("    x) exit — write nothing and leave");
    match options.len() {
        0 => print!("\n  choose r or x: "),
        n => print!("\n  choose 1-{n}, or r/x: "),
    }
    std::io::stdout().flush().ok();

    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| format!("cannot read a choice: {e}"))?;
    parse_device_choice(&line, options.len())
}

/// [`choose_device_with`]'s parsing half, pure and unit-tested directly.
///
/// `r`/`rescan` and `x`/`exit` (case-insensitive) are the two escape hatches;
/// a number in `1..=count` picks that device. A bare Return has no default —
/// `SPEC-05` §4 — so it, and anything else, is refused by name rather than
/// silently taking a device.
///
/// # Errors
///
/// An empty line (no default here), or a token that is neither `r`/`x` nor a
/// number in range — the refusal names what was valid.
fn parse_device_choice(input: &str, count: usize) -> Result<DeviceChoice, String> {
    let typed = input.trim();
    match typed.to_ascii_lowercase().as_str() {
        "r" | "rescan" => return Ok(DeviceChoice::Rescan),
        "x" | "exit" => return Ok(DeviceChoice::Exit),
        _ => {}
    }
    let valid = if count == 0 {
        "r (rescan) or x (exit)".to_owned()
    } else {
        format!("a number 1-{count}, r (rescan) or x (exit)")
    };
    if typed.is_empty() {
        return Err(format!("nothing was typed — choose {valid}; there is no default here"));
    }
    let n: usize = typed.parse().map_err(|_| format!("{typed:?} is not {valid}"))?;
    if n == 0 || n > count {
        return Err(format!("{n} is not {valid}"));
    }
    Ok(DeviceChoice::Selected(n - 1))
}

/// Write the image to `disk`, once the operator has said yes, then read
/// every byte back to prove the card is usable — `SPEC-05` §4's ruled
/// sequence: the plan, `[y/N]`, the line explaining that Windows is about to
/// ask for administrator, the elevated write and verify, and the result.
///
/// # The decision and the mechanism are two different steps
///
/// `Write it? [y/N]` is answered HERE, in this unelevated process, where the
/// operator is reading — unless `confirmed` (`confirm`, the last word on the
/// command line) already answered it, in which case this prompt is skipped
/// and nothing else is. **Only after that** does [`write_elevated`] print the
/// line explaining that administrator is about to be requested and actually
/// request it. The two are kept in that order on purpose: the decision is
/// taken while the operator is reading the plan, and the administrator
/// prompt arrives already explained rather than being the first place a
/// person is asked anything — which is what stops it being a dialog somebody
/// clicks through on reflex. See [`crate::elevate`] for why a second,
/// short-lived, elevated process performs the write at all.
///
/// # Errors
///
/// The device being smaller than the image; `write_elevated`'s own errors —
/// the platform-specific ones documented on it, and ultimately whatever
/// [`write_bytes_and_verify`] reports for a short write, a short read-back,
/// or a digest mismatch, every one of which says plainly that **the card is
/// NOT usable**.
fn write_and_verify(image: &Path, disk: &Disk, confirmed: bool, doc_bytes: &[u8]) -> Result<WriteOutcome, String> {
    write_and_verify_with(std::io::stdin().is_terminal(), image, disk, confirmed, doc_bytes)
}

/// [`write_and_verify`], with whether this run has a terminal taken as a
/// parameter — see [`refuse_unless_terminal`] for why.
///
/// # Errors
///
/// As [`write_and_verify`].
fn write_and_verify_with(
    is_terminal: bool,
    image: &Path,
    disk: &Disk,
    confirmed: bool,
    doc_bytes: &[u8],
) -> Result<WriteOutcome, String> {
    let image_len = std::fs::metadata(image)
        .map_err(|e| format!("cannot measure {}: {e}", image.display()))?
        .len();

    println!();
    println!("PLAN — nothing has been written.");
    println!();
    println!("  image    {} ({} bytes)", image.display(), image_len);
    println!("  device   {}", disk.describe());
    println!();
    println!("  WRITING THIS ERASES EVERYTHING ON {}.", disk.id);
    println!("  There is no undo and no trash.");
    println!();

    // **Checked here, not just left to `io::copy` to discover.** An image
    // that does not fit is refused by name rather than as a write error two
    // gigabytes into a card that is now half-overwritten and still useless.
    if disk.size_bytes != 0 && disk.size_bytes < image_len {
        return Err(format!(
            "NOTHING WAS WRITTEN. {} is {} bytes and the image is {image_len} bytes — it does \
             not fit. Choose a larger device.",
            disk.id, disk.size_bytes
        ));
    }

    if let Some(declined) = confirm_write(is_terminal, confirmed, &disk.id, ask_write_it)? {
        return Ok(WriteOutcome::not_written(declined));
    }
    println!();

    let report = write_elevated(image, disk, image_len, doc_bytes)?;
    // **A cancelled administrator prompt is `Ok`, and wrote nothing** —
    // `crate::elevate::interpret_outcome` reports it starting "nothing was
    // written", and that module's own declined-prompt test pins the prefix.
    let written = !report.starts_with("nothing was written");
    Ok(WriteOutcome { report, written })
}

/// **The decision to write, once the plan is on screen.** `confirmed`
/// (`confirm` on the command line) already answered it and nothing is asked;
/// otherwise `ask` puts `Write it? [y/N]` to the operator. `Ok(None)` means
/// write; `Ok(Some(report))` means the answer was no, and nothing was written.
///
/// # Errors
///
/// Not confirmed and no terminal to ask at, or `ask` itself failing.
fn confirm_write(
    is_terminal: bool,
    confirmed: bool,
    disk_id: &str,
    ask: fn() -> Result<bool, String>,
) -> Result<Option<String>, String> {
    if confirmed {
        return Ok(None);
    }
    // `PLAN-Feature-Set-2026-09-12` §7.2: a run with no terminal cannot
    // answer `Write it? [y/N]`, and a closed stdin's EOF reads as an
    // empty line — `is_yes`'s default — which would silently DECLINE a
    // write an agent may have every intention of confirming. Refusing by
    // name, before the prompt prints at all, is the only honest answer.
    refuse_unless_terminal(is_terminal, "writing the device", "Add `confirm` as the last word.")?;
    if ask()? {
        return Ok(None);
    }
    Ok(Some(format!(
        "nothing was written. Answer y at the prompt, or add `confirm` as the last word, to write \
         {disk_id}\n"
    )))
}

/// `Write it? [y/N]` — `SPEC-05` §4's step 3, default no. A bare Enter takes
/// the default, unlike every OTHER menu in this file: those default to the
/// first, least-costly-to-re-run option, and here the least-costly option is
/// declining.
///
/// # Errors
///
/// Stdin cannot be read at all.
fn ask_write_it() -> Result<bool, String> {
    print!("Write it? [y/N] ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| format!("cannot read a choice: {e}"))?;
    Ok(is_yes(&line))
}

/// The yes/no reading, pulled out of [`ask_write_it`] so it can be proven
/// with no stdin at all.
fn is_yes(typed: &str) -> bool {
    matches!(typed.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

// ---------------------------------------------------------------------------
// The mechanism, per platform — `SPEC-05` §4's step 4 onward. Windows hands
// off to a second, elevated process (`crate::elevate`); Linux does not,
// because it is a different problem and not the same one asked twice.
// ---------------------------------------------------------------------------

/// Print the line explaining what is about to happen, then perform it —
/// `SPEC-05` §4's step 4 (the explanation) and steps 5–6 (the mechanism and
/// the result), kept in one function per platform because the two are not
/// separable: Windows explains an administrator prompt it is about to raise;
/// Linux either explains that it is already root, or refuses before raising
/// anything at all, because there is nothing here for it to raise.
#[cfg(windows)]
fn write_elevated(image: &Path, disk: &Disk, _image_len: u64, doc_bytes: &[u8]) -> Result<String, String> {
    print!("{ELEVATION_EXPLAINER}");
    std::io::stdout().flush().ok();
    let report = crate::elevate::write_via_elevated_child(image, disk)?;
    Ok(format!("{report}{}", boot_partition_note(&disk.id, doc_bytes)))
}

#[cfg(target_os = "linux")]
fn write_elevated(image: &Path, disk: &Disk, image_len: u64, _doc_bytes: &[u8]) -> Result<String, String> {
    if !crate::elevate::running_as_root() {
        return Err(linux_root_required("write", &disk.id));
    }
    println!(
        "already running as root: there is no second process to hand this off to, and none is \
         needed. The write begins now, in this process."
    );
    write_bytes_and_verify(image, &disk.id, image_len, None, None)
}

/// macOS takes Windows's shape rather than Linux's: the interview, the plan and
/// every file it writes stay the operator's, and only the write runs as root, in
/// a child `sudo` starts — so nothing under `~/.rivet` or the output directory
/// comes to be owned by root. `ADR-062` §5.
#[cfg(target_os = "macos")]
fn write_elevated(image: &Path, disk: &Disk, _image_len: u64, _doc_bytes: &[u8]) -> Result<String, String> {
    print!("{ELEVATION_EXPLAINER}");
    std::io::stdout().flush().ok();
    crate::elevate::write_via_elevated_child(image, disk)
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn write_elevated(_image: &Path, _disk: &Disk, _image_len: u64, _doc_bytes: &[u8]) -> Result<String, String> {
    Err("writing a device is built only for Windows, Linux and macOS (ADR-062 §5).".to_owned())
}

/// **Linux is a different problem, not the same one** — `SPEC-05` §4. Windows
/// hands elevation off mid-run because `ShellExecuteEx`'s `runas` verb starts
/// a NEW process with a new token; a running process cannot raise its own.
/// Linux has nothing comparable for an already-running process to request, so
/// this does not attempt to relaunch itself with `sudo` — it checks whether
/// it already IS root, and if not, says so and asks the operator to re-run
/// the command themselves.
#[cfg(target_os = "linux")]
fn linux_root_required(what: &str, disk_id: &str) -> String {
    format!(
        "NOTHING WAS WRITTEN. {what} {disk_id} needs root on Linux, and this process is not \
         root. There is no UAC-style hand-off here for an already-running process to request — \
         SPEC-05 §4 — so re-run the exact same command with sudo in front of it."
    )
}

/// [`write_elevated`]'s read-only sibling for `--verify-only` — reads instead
/// of writing, but needs exactly the same administrator access on Windows
/// (measured 2026-09-09: even a *read* of `\\.\PHYSICALDRIVEn` was refused
/// unelevated), so it takes the identical per-platform path.
#[cfg(windows)]
fn verify_elevated(image: &Path, disk: &Disk) -> Result<String, String> {
    print!("{ELEVATION_EXPLAINER_VERIFY}");
    std::io::stdout().flush().ok();
    crate::elevate::verify_via_elevated_child(image, disk)
}

#[cfg(target_os = "linux")]
fn verify_elevated(image: &Path, disk: &Disk) -> Result<String, String> {
    if !crate::elevate::running_as_root() {
        return Err(linux_root_required("verifying", &disk.id));
    }
    verify_device_against_image(image, &disk.id, None)
}

#[cfg(target_os = "macos")]
fn verify_elevated(image: &Path, disk: &Disk) -> Result<String, String> {
    print!("{ELEVATION_EXPLAINER_VERIFY}");
    std::io::stdout().flush().ok();
    crate::elevate::verify_via_elevated_child(image, disk)
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn verify_elevated(_image: &Path, _disk: &Disk) -> Result<String, String> {
    Err("verifying a device is built only for Windows, Linux and macOS (ADR-062 §5).".to_owned())
}

/// **Confirms, without elevation, that the document reached both boot
/// partitions** — `SPEC-05` §4: the check that answers *"will this card boot
/// configured,"* which is the failure that cost a whole bring-up on
/// 2026-09-07 when a card was provisioned onto only one of the two.
///
/// Volume handles open unelevated — measured 2026-09-09 alongside the
/// `\\.\PHYSICALDRIVEn` refusal that is the whole reason [`crate::elevate`]
/// exists — so after the elevated child restores the disk online, THIS
/// process (not the child) can read `<letter>:\rivet.toml` off whatever
/// drive letters Windows assigns the boot partitions, the same way any USB
/// drive is read every day. It is a NOTE appended to a success report, never
/// a reason to fail the run: the whole-disk digest [`write_bytes_and_verify`]
/// already compared covers every byte on the device, including these two
/// partitions, so this is corroboration rather than the primary evidence —
/// see [`crate::elevate`]'s module documentation for why the primary
/// evidence has to come from the elevated side at all.
#[cfg(windows)]
fn boot_partition_note(disk_id: &str, expected: &[u8]) -> String {
    const ATTEMPTS: u32 = 10;
    const DELAY: std::time::Duration = std::time::Duration::from_millis(500);

    for attempt in 0..ATTEMPTS {
        let Ok(disks) = enumerate::all() else { continue };
        let Some(disk) = disks.iter().find(|d| d.id == disk_id) else { continue };
        if !disk.letters.is_empty() {
            let checks: Vec<BootPartitionCheck> = disk
                .letters
                .iter()
                .map(|letter| {
                    evaluate_boot_partition(
                        letter,
                        read_boot_document(letter).as_deref(),
                        staged_bundle_present(letter),
                        expected,
                    )
                })
                .collect();
            return describe_boot_check(&checks);
        }
        if attempt + 1 < ATTEMPTS {
            std::thread::sleep(DELAY);
        }
    }
    "--- could not confirm the boot partitions unelevated: no drive letter appeared for this \
     disk within five seconds of it coming back online. This does not mean the write failed — \
     only that this particular check could not run; the whole-disk digest above already covers \
     these bytes.\n"
        .to_owned()
}

#[cfg(windows)]
fn read_boot_document(letter: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("{letter}\\{DOCUMENT_NAME}")).ok()
}

#[cfg(windows)]
fn staged_bundle_present(letter: &str) -> bool {
    Path::new(&format!("{letter}\\{}", rivet_core::staging::BUNDLE_FILE)).is_file()
}

/// Whether a failed flush of `disk_id` is macOS's raw disk saying it has no
/// cache to flush, rather than a failure.
///
/// On macOS `sync_all` is `F_FULLFSYNC`, and a raw `/dev/rdiskN` node refuses
/// it with `ENOTTY` (25) — measured on the first card a Mac wrote, after every
/// byte had gone out. The raw node is uncached by definition: writes go to the
/// device and the read-back reads from it, which is the property the flush
/// exists to guarantee. Any other error, any other node, and any other
/// platform still fail.
fn flush_refusal_means_uncached(disk_id: &str, errno: Option<i32>) -> bool {
    const ENOTTY: i32 = 25;
    cfg!(target_os = "macos") && errno == Some(ENOTTY) && disk_id.starts_with("/dev/rdisk")
}

/// What was found on one mounted boot partition.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
struct BootPartitionCheck {
    letter: String,
    document_matches: bool,
    staged_present: bool,
}

/// Pure, so it is provable against fabricated file contents rather than a
/// real drive letter — this project's own safety rule (`AGENTS.md` §9) means
/// no agent may write to a real device, and a unit test cannot conjure a real
/// mounted volume either.
#[cfg_attr(not(windows), allow(dead_code))]
fn evaluate_boot_partition(
    letter: &str,
    document: Option<&[u8]>,
    staged_present: bool,
    expected: &[u8],
) -> BootPartitionCheck {
    BootPartitionCheck {
        letter: letter.to_owned(),
        document_matches: document == Some(expected),
        staged_present,
    }
}

/// Render what `boot_partition_note` found — pure, and the other half of
/// [`evaluate_boot_partition`]'s testability.
#[cfg_attr(not(windows), allow(dead_code))]
fn describe_boot_check(checks: &[BootPartitionCheck]) -> String {
    if checks.is_empty() {
        return "--- no boot partition could be confirmed unelevated: nothing is mounted with a \
                drive letter for this disk\n"
            .to_owned();
    }
    let carrying = checks.iter().filter(|c| c.document_matches).count();
    let mut out = format!(
        "--- unelevated check: {DOCUMENT_NAME} confirmed byte-for-byte on {carrying} of {} \
         mounted volume(s) for this disk\n",
        checks.len(),
    );
    for c in checks {
        out.push_str(&format!(
            "      {}   {DOCUMENT_NAME} {}, {} {}\n",
            c.letter,
            if c.document_matches { "matches" } else { "MISSING or does not match" },
            rivet_core::staging::BUNDLE_FILE,
            if c.staged_present { "present" } else { "absent" },
        ));
    }
    if carrying < 2 {
        out.push_str(
            "      FEWER THAN TWO boot partitions carry the document unelevated. A card that \
             boots the other slot at its first update will forget this configuration — the \
             failure that cost a whole bring-up on 2026-09-07. (The whole-disk digest above is \
             the stronger evidence; if it passed, this is almost certainly a drive-letter timing \
             read rather than a real gap — re-run the standalone verify if in doubt.)\n",
        );
    }
    out
}

/// How many chunks pass between progress-file updates during a write or a
/// verify. **Not every chunk**: at `CHUNK` = 1 MiB, writing one progress line
/// per chunk on a multi-gigabyte image is thousands of small file writes for
/// no benefit an operator watching a percentage would notice — this still
/// updates several times a second on ordinary card-writer throughput.
#[cfg_attr(not(any(windows, target_os = "linux", target_os = "macos")), allow(dead_code))]
const PROGRESS_EVERY: u32 = 8;

/// Stream `src` into `out` a chunk at a time, digesting exactly the bytes
/// written — the copy half of [`write_bytes_and_verify`]'s write phase,
/// pulled out and generalised over the source so a test can hand it bytes
/// that do not match what an earlier caller recorded, with no real file for
/// an attacker to race and no background writer for a test to race against
/// (`AGENTS.md` §4 and §9).
///
/// Returns how many bytes were copied and the BLAKE3 digest of exactly those
/// bytes, computed in this one pass — never a second read of `src`.
///
/// # Errors
///
/// `src` cannot be read, or `out` cannot be written to.
#[cfg_attr(not(any(windows, target_os = "linux", target_os = "macos")), allow(dead_code))]
fn copy_and_digest(
    mut src: impl Read,
    out: &mut File,
    disk_id: &str,
    image_len: u64,
    progress: Option<&Path>,
) -> Result<(u64, [u8; 32]), String> {
    let mut written = Blake3::new();
    written.start();
    let mut buffer = vec![0u8; CHUNK];
    let mut copied: u64 = 0;
    let mut since_report: u32 = 0;
    loop {
        let read = src.read(&mut buffer).map_err(|e| format!("while reading the image: {e}"))?;
        if read == 0 {
            break;
        }
        out.write_all(&buffer[..read])
            .map_err(|e| format!("while writing {disk_id}: {e}"))?;
        written.write(&buffer[..read]);
        copied += read as u64;
        since_report += 1;
        if let Some(path) = progress
            && since_report >= PROGRESS_EVERY
        {
            let _ = crate::elevate::append_progress(path, "write", copied, image_len);
            since_report = 0;
        }
    }
    if let Some(path) = progress {
        let _ = crate::elevate::append_progress(path, "write", copied, image_len);
    }
    Ok((copied, written.finish()))
}

/// Whether the bytes [`copy_and_digest`] just streamed are the image this run
/// was told to write, when the caller has an `expected` digest to compare
/// against.
///
/// **Only the Windows elevated write carries one** — taken by the unelevated
/// parent from the image file before its own elevation prompt
/// (`crate::elevate::compose_task`), so a same-user rewrite of the image
/// during the wait for that prompt is caught here rather than written to the
/// card silently. `None` (nothing to compare) for every other caller, and
/// when the two digests agree.
#[cfg_attr(not(any(windows, target_os = "linux", target_os = "macos")), allow(dead_code))]
fn image_identity_mismatch_error(disk_id: &str, expected: Option<&str>, written: &[u8; 32]) -> Option<String> {
    let expected = expected?;
    let written_hex = rivetos_tls::encode_hex(written);
    if written_hex == expected {
        return None;
    }
    Some(format!(
        "{disk_id} now holds bytes that are NOT the image named for this run: its BLAKE3 was \
         {expected} when this run began, before the elevation prompt, and {written_hex} when \
         this pass actually streamed it to the disk. {disk_id} must NOT be used — write the \
         card again.",
    ))
}

/// The BLAKE3 digest of the whole file at `path`, hex-encoded, streamed a
/// chunk at a time rather than held in memory at once — the same posture
/// [`write_bytes_and_verify`] already takes toward a multi-gigabyte image.
///
/// `pub(crate)` for `crate::elevate::compose_task`: the unelevated parent
/// calls this on the image before the elevation prompt, so
/// [`write_bytes_and_verify`] has something honest to compare its own
/// write-time digest against.
///
/// # Errors
///
/// `path` cannot be opened or read.
pub(crate) fn digest_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let mut hasher = Blake3::new();
    hasher.start();
    let mut buffer = vec![0u8; CHUNK];
    loop {
        let read = file.read(&mut buffer).map_err(|e| format!("while reading {}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.write(&buffer[..read]);
    }
    Ok(rivetos_tls::encode_hex(&hasher.finish()))
}

/// The write and the read-back, once every confirmation has already passed.
///
/// **Takes `disk_id` as a bare string rather than a [`Disk`]**, and
/// `image_len` rather than re-measuring it, specifically so this can be
/// called directly against a temporary file standing in for a device — which
/// is how this project's own safety rule requires the writing path to be
/// tested (`AGENTS.md` §9: never a real block device from an agent's hands).
/// `crate::elevate::attempt_elevated_write` is the real caller on Windows,
/// passing the elevated child's own progress file and the image digest its
/// task file carries; [`write_elevated`] calls it directly, with no progress
/// file and no digest to compare against, on Linux, where the whole run is
/// one process and there is nobody else to poll one and no elevation wait for
/// a same-user attacker to use.
///
/// `pub(crate)` for [`crate::elevate`] alone — see the module header there
/// for why the write has to happen from inside an elevated child on Windows
/// rather than here.
///
/// # Errors
///
/// A short write, a short read-back, a digest mismatch between what was
/// written and what was read back, or — when `expected_image_digest` is
/// `Some` — a mismatch between what was actually streamed and the digest
/// taken before the elevation prompt. Every one of these says plainly that
/// **the card is NOT usable**.
#[cfg_attr(not(any(windows, target_os = "linux", target_os = "macos")), allow(dead_code))]
pub(crate) fn write_bytes_and_verify(
    image: &Path,
    disk_id: &str,
    image_len: u64,
    progress: Option<&Path>,
    expected_image_digest: Option<&str>,
) -> Result<String, String> {
    // **Opened without `create`.** A path that is not already there is a typo,
    // and the alternative is quietly filling a home directory with a multi-
    // gigabyte file named after a device that does not exist.
    let mut out = OpenOptions::new().write(true).create(false).open(disk_id).map_err(|e| {
        format!(
            "cannot open {disk_id} for writing: {e}. It must already exist — this refuses to \
             create a file where a device was meant",
        )
    })?;
    let src = File::open(image).map_err(|e| format!("cannot reopen the image: {e}"))?;

    let (copied, digest_written) = copy_and_digest(src, &mut out, disk_id, image_len, progress)?;

    // **Flushed to the media before anything reads it back.** The read-back
    // below is worthless if it can be answered from the same cache the write
    // above just populated — `AGENTS.md` §5's rule about the kernel's
    // verdicts, pointed at a disk instead of a network interface.
    if let Err(e) = out.sync_all()
        && !flush_refusal_means_uncached(disk_id, e.raw_os_error())
    {
        return Err(format!("while flushing {disk_id}: {e}"));
    }

    if let Some(why) = image_identity_mismatch_error(disk_id, expected_image_digest, &digest_written) {
        return Err(why);
    }
    if let Some(why) = short_write_error(disk_id, copied, image_len) {
        return Err(why);
    }
    drop(out);

    // **A fresh handle, opened after the flush above.** Re-reading through
    // `out`'s own handle would risk answering from whatever buffering that
    // handle still holds; a new `File::open` has nothing to answer from but
    // the device itself.
    let mut check = File::open(disk_id)
        .map_err(|e| format!("cannot reopen {disk_id} to verify it: {e}"))?;
    let mut buffer = vec![0u8; CHUNK];
    let mut read_back_hash = Blake3::new();
    read_back_hash.start();
    let mut read_back: u64 = 0;
    let mut since_report: u32 = 0;
    while read_back < copied {
        let want = buffer.len().min((copied - read_back) as usize);
        let got = check
            .read(&mut buffer[..want])
            .map_err(|e| format!("while reading {disk_id} back to verify it: {e}"))?;
        if got == 0 {
            return Err(format!(
                "read only {read_back} of {copied} bytes back from {disk_id}. The card is NOT \
                 usable"
            ));
        }
        read_back_hash.write(&buffer[..got]);
        read_back += got as u64;
        since_report += 1;
        if let Some(path) = progress
            && since_report >= PROGRESS_EVERY
        {
            let _ = crate::elevate::append_progress(path, "verify", read_back, copied);
            since_report = 0;
        }
    }
    if let Some(path) = progress {
        let _ = crate::elevate::append_progress(path, "verify", read_back, copied);
    }
    let digest_read_back = read_back_hash.finish();

    if let Some(why) = digest_mismatch_error(disk_id, copied, &digest_written, &digest_read_back) {
        return Err(why);
    }

    Ok(format!(
        "wrote {copied} bytes to {disk_id} and read every one back — BLAKE3 {} on both sides. \
         Eject it before removing the card\n",
        rivetos_tls::encode_hex(&digest_written),
    ))
}

/// **`cargo rivet write-image --verify-only`, standing alone** — `SPEC-05`
/// §4 with nothing written first. Hashes `image` and `disk_id`
/// independently, streamed, and compares — the same read-back half
/// [`write_bytes_and_verify`] performs after a write, run here for a card
/// that was already written in an earlier session.
///
/// **Reads at most `image`'s own length off the device**, not the whole
/// disk: a card is typically larger than the image written to it (`SPEC-05`
/// §4's fit check only ever refuses the opposite), and hashing the trailing,
/// untouched capacity would compare bytes this tool never claimed anything
/// about.
///
/// `pub(crate)` because [`crate::elevate`] is what calls this on
/// Windows — reading a whole `\\.\PHYSICALDRIVEn` needs administrator exactly
/// as much as writing one does (measured 2026-09-09), so this runs inside the
/// same elevated child a write would, just without the offline/online dance
/// or the write itself.
///
/// # Errors
///
/// The image cannot be opened or measured, the device cannot be opened or
/// read, a short read, or the two digests disagreeing — reported the same
/// way a failed write-then-verify is: **the card is NOT usable.**
#[cfg_attr(not(any(windows, target_os = "linux", target_os = "macos")), allow(dead_code))]
pub(crate) fn verify_device_against_image(
    image: &Path,
    disk_id: &str,
    progress: Option<&Path>,
) -> Result<String, String> {
    let mut src = File::open(image).map_err(|e| format!("cannot open {}: {e}", image.display()))?;
    let image_len = src
        .metadata()
        .map_err(|e| format!("cannot measure {}: {e}", image.display()))?
        .len();

    let mut buffer = vec![0u8; CHUNK];
    let mut image_hash = Blake3::new();
    image_hash.start();
    loop {
        let n = src
            .read(&mut buffer)
            .map_err(|e| format!("while reading {}: {e}", image.display()))?;
        if n == 0 {
            break;
        }
        image_hash.write(&buffer[..n]);
    }
    let image_digest = image_hash.finish();

    let mut device = File::open(disk_id).map_err(|e| format!("cannot open {disk_id}: {e}"))?;
    let mut device_hash = Blake3::new();
    device_hash.start();
    let mut checked: u64 = 0;
    let mut since_report: u32 = 0;
    while checked < image_len {
        let want = buffer.len().min((image_len - checked) as usize);
        let got = device
            .read(&mut buffer[..want])
            .map_err(|e| format!("while reading {disk_id}: {e}"))?;
        if got == 0 {
            return Err(format!(
                "read only {checked} of {image_len} bytes from {disk_id}. The card is NOT usable"
            ));
        }
        device_hash.write(&buffer[..got]);
        checked += got as u64;
        since_report += 1;
        if let Some(path) = progress
            && since_report >= PROGRESS_EVERY
        {
            let _ = crate::elevate::append_progress(path, "verify", checked, image_len);
            since_report = 0;
        }
    }
    if let Some(path) = progress {
        let _ = crate::elevate::append_progress(path, "verify", checked, image_len);
    }
    let device_digest = device_hash.finish();

    if image_digest != device_digest {
        return Err(format!(
            "{disk_id} does not match {} — BLAKE3 {} for the image, {} for the device. The card \
             is NOT usable.",
            image.display(),
            rivetos_tls::encode_hex(&image_digest),
            rivetos_tls::encode_hex(&device_digest),
        ));
    }
    Ok(format!(
        "{disk_id} matches {} — BLAKE3 {} on both sides, verified with nothing written\n",
        image.display(),
        rivetos_tls::encode_hex(&image_digest),
    ))
}

/// `SPEC-05` §4's short-write check, pulled out so its exact wording is
/// provable without driving a real short write.
#[cfg_attr(not(any(windows, target_os = "linux", target_os = "macos")), allow(dead_code))]
fn short_write_error(disk_id: &str, copied: u64, image_len: u64) -> Option<String> {
    if copied == image_len {
        return None;
    }
    Some(format!(
        "wrote {copied} bytes of {image_len} to {disk_id}. The card is NOT usable"
    ))
}

/// `SPEC-05` §4's read-back comparison, pulled out for the same reason as
/// [`short_write_error`].
#[cfg_attr(not(any(windows, target_os = "linux", target_os = "macos")), allow(dead_code))]
fn digest_mismatch_error(
    disk_id: &str,
    copied: u64,
    written: &[u8; 32],
    read_back: &[u8; 32],
) -> Option<String> {
    if written == read_back {
        return None;
    }
    Some(format!(
        "wrote {copied} bytes to {disk_id} but the BLAKE3 digest read back does not match what \
         was written ({} written, {} read back). The card is NOT usable.",
        rivetos_tls::encode_hex(written),
        rivetos_tls::encode_hex(read_back),
    ))
}

#[cfg(test)]
#[path = "write_image_tests.rs"]
mod tests;

#[cfg(test)]
mod flush_tests {
    use super::flush_refusal_means_uncached;

    /// The macOS raw node's ENOTTY is tolerated; nothing else is, anywhere.
    #[test]
    fn only_a_macos_raw_disk_refusing_the_flush_is_forgiven() {
        assert_eq!(flush_refusal_means_uncached("/dev/rdisk4", Some(25)), cfg!(target_os = "macos"));
        assert!(!flush_refusal_means_uncached("/dev/disk4", Some(25)), "the buffered node has a cache");
        assert!(!flush_refusal_means_uncached("/dev/rdisk4", Some(5)), "EIO is a real failure");
        assert!(!flush_refusal_means_uncached("/dev/rdisk4", None));
        assert!(!flush_refusal_means_uncached("/dev/sdb", Some(25)));
    }
}
