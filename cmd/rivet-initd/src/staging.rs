//! **The second thing PID 1 carries off the boot partition** — `ADR-049`, and
//! it is [`crate::identity`]'s journey with one hop added on the far end.
//!
//! # Why there is a second courier at all
//!
//! `crate::identity` moves the MACHINE's credential: a device seed, its
//! certificate and the peers file, into `/state/secrets`, root-owned, read by
//! root-owned programs. That is the whole of what a machine needs to be
//! reached.
//!
//! It is not the whole of what a machine needs to be *useful*. A fleet server's
//! `rivet-fleetd` reads a server certificate, a key, an authority, its own
//! operator credential and a register — five files under three naming schemes —
//! and every one of them is per-machine material that cannot be in a signed
//! payload every machine gets a copy of. On 2026-09-07 all of it was typed in
//! by hand after the card had booted, and `SPEC-05` §1 is the maintainer ruling
//! that out.
//!
//! # What this module does, and the one thing it deliberately does NOT
//!
//! It reads the bundle, checks that it reads, splits it into one bundle per
//! application under `rivet_core::state::STAGING`, and then blanks and unlinks
//! the card's copy exactly as the seed is blanked and unlinked.
//!
//! **Split rather than unpacked**, and `rivet_core::staging::staged_bundle`
//! carries the argument: the supervisor's `Kernel` port has no recursive
//! directory walker in it and is documented as not wanting one, so the reader
//! on the far end wants one file per application rather than a tree to walk.
//!
//! **It does not deliver anything to an application.** It cannot: an
//! application's data directory is mode `0700` owned by that application's own
//! uid (`ADR-015` §5), the uid is derived by the supervisor at discovery, and
//! at this moment in the boot no application has been discovered and most of
//! them are not installed. `cmd/rivet-sysd` owns that hop. **Guessing it here
//! is the exact failure this mechanism exists to end** — files placed as root
//! in an application's directory are invisible to the application, and
//! `rivetos_register::store::load` reports an unreadable register and an absent
//! one identically, which is correct for a first run and silent for this.
//!
//! # The order, which is the whole of the safety
//!
//! [`crate::identity`]'s, unchanged and for its reasons:
//!
//! 1. write every staged file to `/state`,
//! 2. `sync`,
//! 3. **only then** overwrite the bundle on the boot partition with zeros,
//! 4. `sync`,
//! 5. unlink it, and `sync` again.
//!
//! Steps 3 and 5 are two things because **unlinking a file on FAT does not
//! erase its bytes** — measured on a real board, and the measurement is written
//! up in [`crate::identity`]. The bundle carries `server.key` and an operator
//! seed, so the same discipline applies to it and the maintainer ruled that it
//! must.
//!
//! # An existing COMPLETE file is never overwritten
//!
//! `/state` wins over the card, for [`crate::identity::provision_identity`]'s
//! reason: material at rest on a machine is the machine's, and a card left in a
//! slot is a leftover from the flash rather than an instruction to replace
//! anything. A staged file already present is skipped and the bundle is removed
//! anyway, because a private key on a FAT partition is what this is here to
//! prevent.
//!
//! **"Present" means proven complete, not merely readable.** A bundle an
//! interrupted boot left half-written is readable too, and destroying the
//! card's copy on the strength of that reading would destroy the only
//! complete source. What counts as present is decided by comparing to what
//! this boot would render for the same application right now — see
//! `compare_to_existing`.

use crate::kernel::{Errno, Kernel};
use crate::provision::{BOOT_PARTITION_MOUNT, mount_boot_partition, umount_boot_partition};

use rivet_core::staging;

/// How the bytes already in `/state/staging` for one application relate to
/// what this boot would render for it right now.
///
/// The same distinction `crate::identity` draws, for the same reason: a
/// bundle an interrupted boot half-wrote is readable too, and this module
/// used to accept any readable bundle as "already staged" before destroying
/// the card's complete copy. Byte equality to the freshly rendered bundle is
/// the proof; matching LENGTH is the fallback that keeps `/state`'s own
/// deliberate policy — a different bundle already waiting is presumed to be a
/// still-undelivered share from an earlier, different provisioning of this
/// same card, and is left for the supervisor rather than judged here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Existing {
    /// Nothing readable at the destination. Not held.
    Absent,
    /// Byte-for-byte what this boot would render.
    Matches,
    /// Readable, and the same length as this boot's render, but the content
    /// differs. Treated as a genuinely different, complete bundle — `/state`
    /// wins, unchanged from before this module compared bytes at all.
    DifferentComplete,
    /// Readable, but a different length than this boot's render — the shape
    /// an interrupted write leaves. Not trusted.
    Incomplete,
}

fn compare_to_existing(k: &mut dyn Kernel, dest: &str, source: &[u8]) -> Existing {
    match k.read_bytes(dest) {
        Err(_) => Existing::Absent,
        Ok(existing) if existing == source => Existing::Matches,
        Ok(existing) if existing.len() == source.len() => Existing::DifferentComplete,
        Ok(_) => Existing::Incomplete,
    }
}

/// Where staged material comes to rest.
pub const STAGING_DIR: &str = staging::STAGING_DIR;

/// The bundle's name on the boot partition.
pub const BUNDLE_FILE: &str = staging::BUNDLE_FILE;

/// What happened, in the words the console gets.
#[derive(Debug, PartialEq, Eq)]
pub enum Staged {
    /// No bundle on the boot partition. **The ordinary case** on every boot
    /// after the first, and on every card that stages nothing.
    Nothing,
    /// Written to `/state/staging` and removed from the boot partition. The
    /// counts are applications given a bundle and applications whose bundle was
    /// already there and was left alone.
    Installed { written: usize, kept: usize },
    /// Something was there and could not be completed. **The bundle is left on
    /// the boot partition** whenever this happens, so a corrected card — or
    /// simply the next boot — can finish the job.
    Refused(String),
}

impl Staged {
    /// One line for the console.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::Nothing => {
                format!("no {BUNDLE_FILE} on the boot partition; nothing staged for any application")
            }
            Self::Installed { written, kept } => format!(
                "{written} application(s) have material waiting in {STAGING_DIR} and {kept} \
                 already did; \
                 {BUNDLE_FILE} has been OVERWRITTEN WITH ZEROS and unlinked on the boot partition \
                 (ADR-019 §3), because it carries private keys. The SUPERVISOR delivers each \
                 application's share into the directory that application owns — nothing here has \
                 given anything to an application yet"
            ),
            Self::Refused(why) => format!("nothing was staged: {why}"),
        }
    }
}

/// Copy staged material off the boot partition and destroy the card's copy.
pub fn provision_staging(k: &mut dyn Kernel) -> Staged {
    k.say("rivet-initd: looking for staged application material on the boot partition");

    // **Writable**, because the last step deletes. `crate::identity` has already
    // unmounted by the time this runs; two live mounts of one partition is not
    // a state this program creates.
    if let Err(why) = mount_boot_partition(k, true) {
        // Not a refusal: an emulator has no boot partition at all (`ADR-026`
        // §4), and two callers before this one have already said so.
        let outcome = Staged::Nothing;
        k.say(&format!("  no boot partition to read ({why})"));
        k.say(&format!("  {}", outcome.summary()));
        return outcome;
    }

    let mut staged = None;
    let outcome = stage_from_mounted(k, &mut staged);
    umount_boot_partition(k);
    k.say(&format!("  {}", outcome.summary()));
    if let Some(raw) = staged {
        tidy_unbooted_bundle(k, &raw);
    }
    outcome
}

/// **The other boot partition's copy of the bundle** — `cmd/rivet-image`
/// writes it onto both, and it carries private keys.
///
/// Removed only when it is byte-identical to the bundle this boot has just
/// staged and flushed, which is the one proof available that its contents are
/// on `/state`. On a later boot the shares may already be delivered and
/// removed from `/state/staging`, so nothing can prove it there, and a bundle
/// that differs may be one an operator put on that partition for that slot's
/// own first boot: both are left, and said.
fn tidy_unbooted_bundle(k: &mut dyn Kernel, staged: &[u8]) {
    let outcome = crate::unbooted::tidy(
        k,
        BUNDLE_FILE,
        |_, card| {
            if card == staged {
                crate::unbooted::Verdict::Remove
            } else {
                crate::unbooted::Verdict::Keep(
                    "it differs from the bundle this boot staged, so nothing proves its \
                     contents are on /state and it is left for that slot's own first boot"
                        .to_string(),
                )
            }
        },
        blank_then_remove,
    );
    match outcome {
        crate::unbooted::Tidied::NoPartition(_) | crate::unbooted::Tidied::Absent => {}
        crate::unbooted::Tidied::Removed => k.say(&format!(
            "  the OTHER boot partition's copy of {BUNDLE_FILE} has been OVERWRITTEN WITH ZEROS \
             and unlinked: it is the bundle this boot staged (ADR-019 §3)"
        )),
        crate::unbooted::Tidied::Kept(why) => k.say(&format!(
            "  the OTHER boot partition still carries {BUNDLE_FILE}: {why}"
        )),
        crate::unbooted::Tidied::Refused(why) => k.say(&format!(
            "  the OTHER boot partition still carries {BUNDLE_FILE}, which holds PRIVATE KEYS on a \
             FAT partition any card reader can read (ADR-019 §3): {why}"
        )),
    }
}

/// The part that assumes the boot partition is mounted and writable. `staged`
/// is given the bundle's bytes when, and only when, they were staged and the
/// card's copy destroyed.
fn stage_from_mounted(k: &mut dyn Kernel, staged: &mut Option<Vec<u8>>) -> Staged {
    let bundle_on_card = format!("{BOOT_PARTITION_MOUNT}/{BUNDLE_FILE}");

    // `stat` before `read` — R-2. An oversized or hostile card must not make
    // PID 1 allocate the whole of it first and reject the allocation second;
    // `staging::parse`'s own cap is checked only after such a read. A `stat`
    // failure of any kind (including a plain `ENOENT`) falls through to the
    // read below, which already reports every one of those cases correctly.
    if let Ok(size) = k.file_size(&bundle_on_card)
        && size as usize > staging::MAX_BUNDLE_BYTES
    {
        let why = staging::Error::TooLarge {
            bytes: size as usize,
        };
        return Staged::Refused(format!(
            "{bundle_on_card} will not read: {why}. NOTHING was written and the file has been \
             LEFT on the boot partition"
        ));
    }

    let raw = match k.read_bytes(&bundle_on_card) {
        Err(Errno::ENOENT) => return Staged::Nothing,
        Err(errno) => {
            return Staged::Refused(format!("{bundle_on_card} could not be read: {errno}"));
        }
        Ok(bytes) => bytes,
    };

    let Ok(text) = core::str::from_utf8(&raw) else {
        return Staged::Refused(format!(
            "{bundle_on_card} is not UTF-8 text. It is a document `rivet-provision` writes; a \
             card whose copy is not text was not written by it, or was written to a card that is \
             failing. THE FILE HAS BEEN LEFT IN PLACE"
        ));
    };

    // **Refused whole, never in part.** A half-applied staging is a machine in
    // a state nobody designed and nothing on it would say which half arrived.
    // The card still holds everything, so a corrected card is the repair.
    let items = match staging::parse(text) {
        Ok(items) => items,
        Err(why) => {
            return Staged::Refused(format!(
                "{bundle_on_card} will not read: {why}. NOTHING was written and the file has been \
                 LEFT on the boot partition"
            ));
        }
    };
    // **A fleet-root mode fleet server's request is PID 1's, taken out here**
    // — `ADR-063` §4, and `crate::fleet_root` carries why minting happens at
    // this point: the root is minted, or found resting, and the server's
    // material joins the bundle for the applications that open it. Refused
    // whole, like a bundle that will not read, and the card keeps its copy.
    let items = match crate::fleet_root::take(k, items) {
        Ok((items, outcome)) => {
            if let Some(line) = outcome.summary() {
                k.say(&format!("  {line}"));
            }
            items
        }
        Err(why) => {
            return Staged::Refused(format!(
                "{why}. NOTHING was staged and {bundle_on_card} has been LEFT on the boot \
                 partition, so the next boot tries again"
            ));
        }
    };
    if items.is_empty() {
        return Staged::Refused(format!(
            "{bundle_on_card} stages no files. It is left in place rather than removed, because a \
             bundle that reached a card and asks for nothing is a provisioning run that went \
             wrong on the workstation and the evidence is the file"
        ));
    }

    if let Err(errno) = k.mkdir_private(STAGING_DIR) {
        return Staged::Refused(format!("cannot create {STAGING_DIR}: {errno}"));
    }

    let mut written = 0usize;
    let mut kept = 0usize;
    for (application, mine) in staging::by_application(&items) {
        let at = staging::staged_bundle(&application);
        let rendered = staging::render(&mine);
        match compare_to_existing(k, &at, rendered.as_bytes()) {
            // **`/state` wins.** A bundle already waiting was put there by an
            // earlier boot of this card and may already be half delivered. How
            // far it got is the supervisor's to know — it is what removes the
            // bundle when delivery finishes — so replacing it here would hand
            // that application its credentials twice. A `DifferentComplete`
            // bundle is presumed to be exactly that: a complete share from an
            // earlier, different provisioning of this same card, still
            // waiting.
            Existing::Matches | Existing::DifferentComplete => {
                kept = kept.saturating_add(1);
                continue;
            }
            Existing::Absent => {}
            // The shape an interrupted write leaves. Not the supervisor's
            // half-delivered file — a file it is mid-delivery on was written
            // whole in the first place — so it is overwritten rather than kept.
            Existing::Incomplete => {
                k.say(&format!(
                    "  {at} exists but is a different length than this boot would render for \
                     {application} -- the shape an interrupted copy leaves. Overwriting it"
                ));
            }
        }
        // `write_secret` and not `write_file`: everything in a bundle is
        // somebody's credential until the supervisor has said otherwise, and
        // `ADR-020` §5 keeps values root-owned until they are delivered. It is
        // also the atomic, private, durable publish, so an interrupted attempt
        // here can never leave a partial file at `at` for a later boot to
        // trust.
        if let Err(errno) = k.write_secret(&at, rendered.as_bytes()) {
            return Staged::Refused(format!(
                "cannot write {at}: {errno}. Another application's bundle may already be in \
                 {STAGING_DIR}; the card's copy has been LEFT in place, so the next boot \
                 completes this"
            ));
        }
        written = written.saturating_add(1);
    }

    // **The order is the safety.** Everything is on `/state` and flushed before
    // the only other copy is destroyed — see this module's header and
    // `crate::identity`'s, which carries the board measurement behind it.
    k.sync();

    match blank_then_remove(k, &bundle_on_card, raw.len()) {
        Ok(()) => {
            *staged = Some(raw);
            Staged::Installed { written, kept }
        }
        Err(errno) => Staged::Refused(format!(
            "the staged material is in {STAGING_DIR} but {bundle_on_card} could not be removed: \
             {errno}. THE PRIVATE KEYS ARE STILL ON THE BOOT PARTITION, which is readable by any \
             machine with a card reader (ADR-019 §3). The next boot will try again"
        )),
    }
}

/// Overwrite the bundle's bytes before unlinking it.
///
/// **The same two steps, for the same measured reason.**
/// `crate::identity::blank_then_remove` carries the board run that proved
/// unlinking on FAT leaves the bytes recoverable, and the second board run that
/// proved a truncating write leaves them recoverable somewhere else. This is
/// that procedure applied to the other file on the partition that holds a
/// private key, and it is a second copy of five lines rather than a shared one
/// because the two modules are independently removable: a card may carry either
/// file without the other, and neither module may become a precondition of the
/// other's correctness.
fn blank_then_remove(k: &mut dyn Kernel, path: &str, length: usize) -> Result<(), Errno> {
    k.overwrite_in_place(path, &vec![0u8; length])?;
    k.sync();
    k.remove_file(path)?;
    k.sync();
    Ok(())
}
