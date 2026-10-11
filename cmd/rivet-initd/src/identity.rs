//! **`ADR-004` §6 step 3, at last — and the first secret this project has ever
//! taken off the boot partition.**
//!
//! `crate::seed`'s *What is not here* has recorded step 3 as outstanding since
//! 2026-08-23, with an honest reason: *"it is not merely unimplemented: it has
//! nothing to remove"*, because `ADR-007` §7.3 keeps the wireless passphrase out
//! of the configuration document entirely, so no key existed that a secret could
//! be written under.
//!
//! **A device TLS identity is the first thing that changes that** (`ADR-025` §2,
//! `ADR-035` §7.2). It is a real secret, it must reach the card from the
//! operator's own tooling (`ADR-019` §2), and `ADR-019` §3.1 is explicit about
//! what happens next:
//!
//! > PID 1 reads the provisioning file, derives what it needs, writes the result
//! > into `/state/config`, and **blanks the credentials from the provisioning
//! > file.** … A card recovered from a running device does not carry the
//! > wireless passphrase or the enrollment identity.
//!
//! # Why these are files and not keys in the document
//!
//! `crate::seed` writes the provisioning *document* to `/state/config`, and the
//! obvious design puts the seed in that document as a key. **It would put an
//! unredactable secret into the configuration schema**, and `ADR-020` §4 exists
//! precisely to stop that:
//!
//! > **The failure this prevents** is the realistic one:
//! > `rivet export > machine.toml` followed by `git add`.
//!
//! §4's `Secret<T>` — whose `Debug` and `Display` redact — **is not built**. So
//! a seed in the schema today would render in `export`, in `--explain`, in
//! `get schema`, and in any error that quoted a value. Three files beside the
//! document avoid the question entirely: nothing in `rivet-config` ever sees
//! them, so nothing in `rivet-config` can print them.
//!
//! It also makes the operator's job the one `cmd/cargo-rivet` already produces.
//! `cargo rivet identity new` writes `device.seed` and `device.crt`; those two
//! files are copied onto the boot partition as they are, with no encoding step
//! that could go wrong in one direction and be undetectable in the other.
//!
//! # The order, which is the whole of the safety
//!
//! 1. copy all three to `/state/secrets`,
//! 2. `sync`,
//! 3. **only then** overwrite `device.seed` on the boot partition with zeros,
//! 4. `sync`,
//! 5. unlink it, and `sync` again.
//!
//! Steps 3 and 5 are two things rather than one, and [`blank_then_remove`]
//! carries the measurement that made them two: **unlinking a file on FAT does
//! not erase its bytes**, and a board that had provisioned itself was found
//! still carrying its own seed in the boot partition's free space.
//!
//! Reversed, a power cut between the delete and the copy destroys the only
//! copy of a key that cannot be regenerated — `ADR-025` §3 has the fleet
//! generate it and retain only the public half, so a lost seed is a card that
//! must be re-provisioned by hand. The cost of this order is the opposite and
//! much cheaper failure: a crash after step 2 leaves the seed on the boot
//! partition and the next boot removes it.
//!
//! # What it does not do
//!
//! **It never generates a key.** `ADR-025` §3's circularity argument puts
//! generation in the fleet tool, and a board that mints its own identity has to
//! tell the fleet about it over a network that does not exist yet.
//!
//! **It never overwrites `/state`'s copy.** See [`provision_identity`]. That
//! includes the peers file whenever the seed already matches, because the
//! peers file changes after the card is written and the card's is the stale
//! one.
//!
//! # Both boot partitions
//!
//! `cmd/rivet-image` writes the identity onto both boot partitions. The one
//! the firmware booted is handled above; the other one's seed is blanked by
//! `tidy_unbooted_seed` once `/state` provably holds the same key, through
//! `crate::unbooted`.

use crate::kernel::{Errno, Kernel};
use crate::provision::{BOOT_PARTITION_MOUNT, mount_boot_partition, umount_boot_partition};
use crate::unbooted::{Tidied, Verdict};

/// Where a provisioned identity comes to rest — `ADR-025` §2.3.
pub const SECRETS_DIR: &str = rivet_core::state::SECRETS;

/// The Ed25519 seed. Thirty-two raw bytes, mode `0600` once it is here.
pub const SEED_FILE: &str = "device.seed";

/// The self-signed certificate carrying the seed's public half. DER, and not a
/// secret — it is what a peer pins.
pub const CERTIFICATE_FILE: &str = "device.crt";

/// The Ed25519 public keys allowed to connect, one hex key per line.
///
/// Not a secret either: `ADR-025` §2.3 calls the fleet's public key *"ordinary
/// configuration — public by definition, and what matters for it is integrity,
/// not confidentiality"*.
pub const PEERS_FILE: &str = "peers";

/// What happened, in the words the console gets.
#[derive(Debug, PartialEq, Eq)]
pub enum Provisioned {
    /// No `device.seed` on the boot partition. **The ordinary case** on every
    /// boot after the first, and on a card that was never given an identity.
    Nothing,
    /// Copied to `/state/secrets` and removed from the boot partition.
    Installed,
    /// `/state` already had one, so the boot partition's copy was removed
    /// without being used.
    AlreadyHeld,
    /// `/state` holds a complete identity that differs from the boot
    /// partition's, so `/state`'s is kept and **the boot partition's seed is
    /// left in place** — it is not this machine's key.
    KeptDifferent,
    /// Something was there and could not be completed. **The seed is left on
    /// the boot partition** whenever this happens.
    Refused(String),
}

impl Provisioned {
    /// One line for the console.
    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::Nothing => {
                format!("no {SEED_FILE} on the boot partition; nothing to provision")
            }
            Self::Installed => format!(
                "the device identity is in {SECRETS_DIR}; {SEED_FILE} has been OVERWRITTEN \
                 WITH ZEROS and unlinked on the boot partition (ADR-019 3.1). Unlinking \
                 alone leaves the bytes readable in FAT free space, which was measured \
                 on a board and is why this is two steps"
            ),
            Self::AlreadyHeld => format!(
                "{SECRETS_DIR}/{SEED_FILE} already exists, so the boot partition's copy was \
                 removed WITHOUT being used. Rotation is a configuration change and not a \
                 reflash (ADR-025 §4), so a seed appearing here on a provisioned machine is a \
                 leftover rather than a new key — and a private key left on a FAT partition is \
                 the thing ADR-019 §3.1 exists to prevent"
            ),
            Self::KeptDifferent => format!(
                "{SECRETS_DIR}/{SEED_FILE} holds a complete identity that does NOT match the boot \
                 partition's {SEED_FILE}. /state's is KEPT as this machine's own, and the boot \
                 partition's seed is LEFT IN PLACE, because it is not this machine's key and \
                 destroying it would be destroying a key nobody here holds. A private key is on \
                 the boot partition until it is removed by hand or the card is rewritten"
            ),
            Self::Refused(why) => format!("the device identity was NOT provisioned: {why}"),
        }
    }
}

/// How the bytes already in `/state` relate to what the card holds for the
/// same file right now.
///
/// **Merely readable is not merely enough** — a partial write an interrupted
/// boot left behind is readable too, and treating the two alike is what let a
/// truncated seed pass for a complete one. Byte equality to the card's current
/// copy is the proof this module prefers; when the content genuinely differs,
/// matching LENGTH is the fallback, because a length that agrees with an
/// independent source is the best evidence available, short of a checksum
/// this format does not carry, that what is there is a complete file of its
/// own rather than a fragment of this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Existing {
    /// Nothing readable at the destination. Not held.
    Absent,
    /// Byte-for-byte the same as the card's current copy.
    Matches,
    /// Readable, and the same length as the card's current copy, but the
    /// content differs. Treated as a genuinely different, complete file and
    /// left alone.
    DifferentComplete,
    /// Readable, but a different length than the card's current copy — the
    /// shape an interrupted write leaves. Not trusted.
    Incomplete,
}

/// Compare what is already at `dest` in `/state` to `source`, the card's
/// current bytes for the same file.
fn compare_to_existing(k: &mut dyn Kernel, dest: &str, source: &[u8]) -> Existing {
    match k.read_bytes(dest) {
        Err(_) => Existing::Absent,
        Ok(existing) if existing == source => Existing::Matches,
        Ok(existing) if existing.len() == source.len() => Existing::DifferentComplete,
        Ok(_) => Existing::Incomplete,
    }
}

/// Copy a device identity off the boot partition and blank the secret half.
///
/// # `/state`'s copy always wins when it is a different, complete identity
///
/// If `/state/secrets/device.seed` is readable and does not match the boot
/// partition's current seed but is the same length, `/state`'s is kept and
/// nothing from the card is installed: `ADR-025` §4 rules that *"rotation is a
/// configuration change, not a reflash"*, so a seed on the boot partition of an
/// already-provisioned machine is never an instruction to replace anything.
///
/// **And the boot partition's seed is left in place, with the mismatch said.**
/// It is not this machine's key, and nothing here can tell a leftover from the
/// flash from the only copy of a key an operator still needs; destroying a key
/// nobody here holds on the strength of a comparison that failed is the one
/// outcome that cannot be undone. The cost is a private key on a FAT partition
/// until somebody removes it, which `ADR-019` §3.1 would rather not have — and
/// which an operator can see, because every boot says so.
///
/// # A seed that merely reads is not the same as an identity that is complete
///
/// **`/state` holding a correct seed does not by itself mean the card's copy
/// may go.** The certificate and the peers file are copied too, and only once
/// the whole set is proven complete — matching the card byte for byte, or
/// already held as a different, complete identity — is the boot partition's
/// seed blanked. A seed that reached `/state` correctly and was then
/// interrupted before its certificate arrived is exactly the state that must
/// NOT be read as "already held": the certificate still has to come from the
/// card, which is why this boot needs it too.
pub fn provision_identity(k: &mut dyn Kernel) -> Provisioned {
    k.say("rivet-initd: looking for a device identity on the boot partition");

    // **Writable**, because step 3 deletes. `read_provisioning` mounts it
    // read-only and unmounts before this runs, so there is no second mount
    // live at the same time.
    if let Err(why) = mount_boot_partition(k, true) {
        // Not a refusal: an emulator has no boot partition at all (`ADR-026`
        // §4), and `read_provisioning` has already said so once.
        let outcome = Provisioned::Nothing;
        k.say(&format!("  no boot partition to read ({why})"));
        k.say(&format!("  {}", outcome.summary()));
        return outcome;
    }

    let outcome = provision_from_mounted(k);
    umount_boot_partition(k);
    k.say(&format!("  {}", outcome.summary()));
    tidy_unbooted_seed(k);
    outcome
}

/// **The other boot partition's copy of the seed** — `cmd/rivet-image` writes
/// the identity onto both, and `ADR-019` §3's promise is about the card, not
/// one partition of it.
///
/// Removed, by [`blank_then_remove`], only when `/state` holds the SAME seed
/// and a certificate beside it: that is the copy proven to be durably on
/// `/state`. Run on every boot rather than only the first, so a card whose
/// second copy outlived an earlier boot is finished by a later one.
///
/// **A seed there that differs from `/state`'s is left in place and said**:
/// `/state` keeps its own complete identity, the mismatch is reported, and a
/// key this machine does not hold is never destroyed on the strength of a
/// comparison that failed. A seed `/state` does not hold at
/// all may be the last copy of that key, and is left for the same reason.
fn tidy_unbooted_seed(k: &mut dyn Kernel) {
    let seed_dest = format!("{SECRETS_DIR}/{SEED_FILE}");
    let cert_dest = format!("{SECRETS_DIR}/{CERTIFICATE_FILE}");
    let outcome = crate::unbooted::tidy(
        k,
        SEED_FILE,
        |k, card| match k.read_bytes(&seed_dest) {
            Ok(held) if held == card && k.read_bytes(&cert_dest).is_ok() => Verdict::Remove,
            Ok(held) if held == card => Verdict::Keep(format!(
                "{seed_dest} matches it but {cert_dest} is not there yet, so the identity on \
                 /state is not complete and this copy may still be needed"
            )),
            Ok(_) => Verdict::Keep(format!(
                "it does not match {seed_dest}. /state keeps its own identity and this seed is \
                 LEFT IN PLACE, because it is not this machine's key and destroying it would be \
                 destroying a key nobody here holds"
            )),
            Err(_) => Verdict::Keep(format!(
                "{seed_dest} is not there, so this may be the only copy of the key"
            )),
        },
        blank_then_remove,
    );
    match outcome {
        // An emulator, or a card whose other partition is not there: nothing
        // to say beyond what the booted partition's pass already said.
        Tidied::NoPartition(_) | Tidied::Absent => {}
        Tidied::Removed => k.say(&format!(
            "  the OTHER boot partition's copy of {SEED_FILE} has been OVERWRITTEN WITH ZEROS and \
             unlinked: {SECRETS_DIR} holds the same key, and a card carries no private key once \
             the machine has it (ADR-019 3.1)"
        )),
        Tidied::Kept(why) => k.say(&format!(
            "  the OTHER boot partition still carries {SEED_FILE}: {why}"
        )),
        Tidied::Refused(why) => k.say(&format!(
            "  the OTHER boot partition still carries {SEED_FILE}, which is a PRIVATE KEY on a FAT \
             partition any card reader can read (ADR-019 3.1); the next boot will try again: {why}"
        )),
    }
}

/// The part that assumes the boot partition is mounted and writable.
fn provision_from_mounted(k: &mut dyn Kernel) -> Provisioned {
    let seed_on_card = format!("{BOOT_PARTITION_MOUNT}/{SEED_FILE}");
    let seed = match k.read_bytes(&seed_on_card) {
        Err(Errno::ENOENT) => return Provisioned::Nothing,
        Err(errno) => {
            return Provisioned::Refused(format!("{seed_on_card} could not be read: {errno}"));
        }
        Ok(bytes) => bytes,
    };

    let seed_dest = format!("{SECRETS_DIR}/{SEED_FILE}");
    let seed_state = compare_to_existing(k, &seed_dest, &seed);

    // **A different, complete identity already in `/state` is the machine's
    // own and is kept, and the card's seed is LEFT IN PLACE** — the rule
    // `tidy_unbooted_seed` follows for the other partition. Nothing here can
    // tell a leftover from the flash from the last copy of a key somebody
    // still needs, and a comparison that failed is no grounds for destroying
    // a key this machine does not hold. The mismatch is said, on every boot
    // it persists.
    if seed_state == Existing::DifferentComplete {
        k.say(&format!(
            "  {seed_dest} already holds a seed that does not match the one on the boot \
             partition, and is the same length. /state keeps its own identity and the boot \
             partition's seed is LEFT IN PLACE"
        ));
        return Provisioned::KeptDifferent;
    }

    // Absent or Incomplete: the seed itself is not held and must be (re)copied
    // below. Matches: the seed is held, but the REST of the identity set still
    // has to be proven complete before the card's copy may go — a seed that
    // reached `/state` correctly and was then interrupted before its
    // certificate arrived must not be mistaken for a finished job.

    // **Both halves or neither.** A seed with no certificate produces a machine
    // that cannot serve TLS and whose failure appears later, in `rivet-mcpd`,
    // as a missing file — a long way from the card that was written wrong.
    let certificate_on_card = format!("{BOOT_PARTITION_MOUNT}/{CERTIFICATE_FILE}");
    let certificate = match k.read_bytes(&certificate_on_card) {
        Ok(bytes) => bytes,
        Err(errno) => {
            return Provisioned::Refused(format!(
                "{SEED_FILE} is on the boot partition and {CERTIFICATE_FILE} is not ({errno}). \
                 `cargo rivet identity new` writes both and they must be copied together — a \
                 certificate from a different keypair is refused by rustls with `KeyMismatch`, \
                 and no certificate at all is a machine that cannot serve TLS. The seed has been \
                 LEFT on the boot partition, so fixing the card and rebooting completes this"
            ));
        }
    };

    // The pin file is optional here and refused later by `rivet-mcpd` if it is
    // absent, which is the right place: this module is not the authority on
    // what a control plane needs, and a card carrying an identity but no peers
    // is a card whose operator has not finished — not one PID 1 should judge.
    let peers = k
        .read_bytes(&format!("{BOOT_PARTITION_MOUNT}/{PEERS_FILE}"))
        .ok();

    if let Err(errno) = k.mkdir_private(SECRETS_DIR) {
        return Provisioned::Refused(format!("cannot create {SECRETS_DIR}: {errno}"));
    }

    let mut wrote_anything = false;

    // The seed only needs (re)writing when it is not already proven to match
    // the card — `write_secret` is the atomic, private, durable publish, so
    // an interrupted attempt here can never leave a partial file at
    // `seed_dest` for a later boot to trust.
    if seed_state != Existing::Matches {
        if seed_state == Existing::Incomplete {
            k.say(&format!(
                "  {seed_dest} exists but is a different length than the boot partition's \
                 current seed -- the shape an interrupted copy leaves. Overwriting it from the \
                 boot partition rather than trusting it"
            ));
        }
        if let Err(errno) = k.write_secret(&seed_dest, &seed) {
            return Provisioned::Refused(format!("cannot write {seed_dest}: {errno}"));
        }
        wrote_anything = true;
    }

    // **The certificate is a function of the seed**, so for the seed this boot
    // trusts, the card's certificate is the right one and an existing file
    // that differs from it is brought up to date: a certificate that does not
    // match its key is refused by rustls, and a partial one an interrupted
    // boot left behind is repaired here.
    let cert_dest = format!("{SECRETS_DIR}/{CERTIFICATE_FILE}");
    if compare_to_existing(k, &cert_dest, &certificate) != Existing::Matches {
        if let Err(errno) = k.write_file(&cert_dest, &certificate) {
            return Provisioned::Refused(format!("cannot write {cert_dest}: {errno}"));
        }
        wrote_anything = true;
    }

    // **The peers file is NOT a function of the seed: it is live state.**
    // Enrollments and revocations change it after the card is written, so the
    // card's copy is installed only alongside the card's seed. When the seed
    // already matches, `/state`'s peers file is the machine's own and wins —
    // otherwise a later first boot of the other slot, whose partition still
    // carries the card's original, would restore revoked peers and drop new
    // ones. An ABSENT one is still filled, because that is a first boot that
    // was interrupted before its peers file arrived.
    if let Some(peers) = &peers {
        let peers_dest = format!("{SECRETS_DIR}/{PEERS_FILE}");
        let held = compare_to_existing(k, &peers_dest, peers);
        let install = match held {
            Existing::Matches => false,
            Existing::Absent => true,
            Existing::DifferentComplete | Existing::Incomplete => seed_state != Existing::Matches,
        };
        if install {
            if let Err(errno) = k.write_file(&peers_dest, peers) {
                return Provisioned::Refused(format!("cannot write {peers_dest}: {errno}"));
            }
            wrote_anything = true;
        } else if held != Existing::Matches {
            k.say(&format!(
                "  {peers_dest} differs from the boot partition's copy and the seed already \
                 matches, so /state's is KEPT: it carries the enrollments and revocations made \
                 since the card was written"
            ));
        }
    }

    // **The order is the safety.** Everything is on `/state` and flushed before
    // the only other copy is destroyed — see this module's header.
    k.sync();

    match blank_then_remove(k, &seed_on_card, seed.len()) {
        Ok(()) => {
            if wrote_anything {
                Provisioned::Installed
            } else {
                // The seed already matched and so did the certificate and any
                // peers file: nothing on this boot came from the card except
                // the read that proved it. Same word the pre-B12 code used for
                // "an identity was already here", because that is still true.
                Provisioned::AlreadyHeld
            }
        }
        // The copy succeeded, so nothing is lost — but the secret is still on a
        // FAT partition and saying otherwise would be the lie ADR-019 §3.1 is
        // about.
        Err(errno) => Provisioned::Refused(format!(
            "the identity was copied to {SECRETS_DIR} but {seed_on_card} could not be removed: \
             {errno}. THE PRIVATE KEY IS STILL ON THE BOOT PARTITION, which is readable by any \
             machine with a card reader (ADR-019 §3.1). The next boot will try again"
        )),
    }
}

/// **Overwrite the seed's bytes before unlinking it, because unlinking does not
/// erase them.**
///
/// # The measurement this exists for — 2026-08-28
///
/// The first version of this module called `remove_file` and stopped, and
/// `ADR-019` §3.1's promise was read as satisfied:
///
/// > A card recovered from a running device does not carry the wireless
/// > passphrase or the enrollment identity.
///
/// **It was not.** The card was taken out of a board that had provisioned
/// itself, read on a workstation and searched, with the certificate — a file
/// nothing deletes — as the positive control:
///
/// ```text
/// POSITIVE CONTROL - certificate (never deleted): FOUND at 13619200
/// THE QUESTION      - seed bytes: STILL RECOVERABLE at 13621248
/// ```
///
/// FAT deletion clears a directory entry and returns clusters to the free
/// list; it does not touch the data in them. So the seed survived exactly as
/// long as nothing else happened to allocate that cluster, which on a boot
/// partition that is written once per update is **indefinitely**.
///
/// Writing zeros over the file first puts the erase where the filesystem will
/// actually perform it, and `sync` between the two makes the order observable
/// rather than hopeful.
///
/// # And the zeros must go in WITHOUT truncating, which took a second board run
///
/// The first fix used `write_file`, which is `std::fs::write`, which truncates.
/// **On FAT that frees the cluster chain**, and the 32 bytes that follow are
/// free to land somewhere else — so the board came back with the zeros in one
/// place and its seed still readable at offset 13647872 in another.
/// `Kernel::overwrite_in_place` opens without truncation, which keeps the
/// chain and puts the zeros on the bytes that held the key.
///
/// # What this still does not promise, and it must not be read as more
///
/// **The flash may keep the old page.** An SD card is a wear-levelled device
/// with its own controller: a write to a logical block is free to land on a
/// fresh physical page and leave the previous one intact and readable by
/// anything that can talk to the raw NAND. Nothing this program can do reaches
/// that, and `ADR-020` §3's *"someone takes only the SD card"* row is therefore
/// **still a no** for an attacker willing to go past the block layer.
///
/// What this does buy is the whole of the ordinary case: the card is no longer
/// carrying the key at the level any filesystem tool, forensic recovery
/// utility or `grep` over the block device can see.
fn blank_then_remove(k: &mut dyn Kernel, path: &str, length: usize) -> Result<(), Errno> {
    // Same length, so the same clusters are rewritten rather than a shorter
    // file leaving the tail of the old one allocated and untouched.
    k.overwrite_in_place(path, &vec![0u8; length])?;
    k.sync();
    k.remove_file(path)?;
    k.sync();
    Ok(())
}
