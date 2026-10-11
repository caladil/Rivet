//! `cargo rivet key backup <backup-dir>` / `cargo rivet key restore
//! <backup-dir> [--replace]` — **the two signing seeds, passphrase-encrypted.**
//! `.rulings/` R-183 D3.
//!
//! # Why this is not `rivet_filecopy::copy_all`
//!
//! `rivet-provision ca backup` and `cargo rivet fleet objects backup`
//! (`crate::fleet`) both copy plaintext — the CA is recoverable if stolen
//! (`SPEC-05`'s twelve-step re-mint), and an object-store artifact is public
//! by the time it is published. **A signing seed is neither.** `ADR-028`
//! §5.5.4: revoking a compromised signing key is an image update shipped to
//! every board, and until that ships, whoever holds the seed can sign
//! anything the fleet will run. R-183 D3 rules the seed backup
//! passphrase-encrypted for exactly this reason, so this module encrypts the
//! bytes itself, in memory, before anything reaches a file — `rivet_filecopy`
//! never sees a seed, encrypted or not.
//!
//! # The file: one envelope per seed
//!
//! Each seed is sealed in the passphrase envelope `crate::envelope` defines —
//! Argon2id and XChaCha20-Poly1305, every parameter in the file and
//! authenticated — under the magic `RVTSEED`. That module holds the byte
//! layout, the prompt, and why a wrong passphrase and a tampered file are
//! refused with the same sentence; `cargo rivet backup-secrets` seals a
//! board's secrets in the same envelope (R-185 D1).
//!
//! One file per seed, `<seed-file-name>.enc` (e.g. `rivet-system.seed.enc`)
//! — **never bare `.seed`**, deliberately:
//! `cargo rivet fleet`'s own `find_seed` refuses to initialise or commit a
//! fleet repository tree holding any `.seed` file (`ADR-052` §4), and an
//! encrypted backup is not the secret that check exists to keep out of git —
//! it is ciphertext nobody can sign with. Giving it the same extension would
//! make it collide with that refusal by accident the day somebody's backup
//! directory ends up inside a fleet repository's tree.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use rivet_filecopy::{age_text, Existing};

use crate::credentials::{self, APP_SEED_FILE, SYSTEM_SEED_FILE};
use crate::envelope::{self, Kind, OpenError};

/// The envelope a seed backup is: `RVTSEED`, the magic every seed backup ever
/// written carries.
const SEED_ENVELOPE: Kind = Kind { magic: b"RVTSEED", noun: "a Rivet seed backup file" };

const SEED_LEN: usize = 32;

/// What the prompt calls this backup.
const WHAT: &str = "seed backup";

/// The backup file's own extension — see the module doc for why it is never
/// `.seed`.
const BACKUP_SUFFIX: &str = ".enc";

// ---------------------------------------------------------------------------
// The envelope, for one seed
// ---------------------------------------------------------------------------

/// Seal one seed. See `crate::envelope::seal` for the salt, nonce and costs.
fn encrypt(seed: &[u8; SEED_LEN], passphrase: &str) -> Result<Vec<u8>, String> {
    envelope::seal(SEED_ENVELOPE, seed, passphrase)
}

/// Open one seed backup, and refuse a plaintext that is not a seed.
///
/// # Errors
///
/// [`OpenError::Format`] for a file that is not a seed backup at all, or one
/// that opens to something other than 32 bytes;
/// [`OpenError::WrongPassphraseOrTampered`] when the AEAD will not open.
fn decrypt(file: &[u8], passphrase: &str) -> Result<[u8; SEED_LEN], OpenError> {
    let opened = envelope::open(SEED_ENVELOPE, file, passphrase)?;
    <[u8; SEED_LEN]>::try_from(opened.as_slice())
        .map_err(|_| OpenError::Format("the decrypted seed is not 32 bytes".to_owned()))
}

/// Read a new passphrase twice — `crate::envelope::read_new`, named for this verb.
fn read_new_passphrase() -> Result<String, String> {
    envelope::read_new(WHAT, "cargo rivet key backup")
}

/// Read the passphrase once, for a restore.
fn read_existing_passphrase() -> Result<String, String> {
    envelope::read_existing(WHAT, "cargo rivet key restore")
}

// ---------------------------------------------------------------------------
// Where the live seeds are — a fixed, conventional location
// ---------------------------------------------------------------------------

/// `~/.rivet/keys`. **Deliberately not `credentials::Survey`** — that
/// machinery finds ONE seed to sign with, honouring `$RIVET_APP_SEED`/
/// `$RIVET_SYSTEM_SEED` overrides that can point anywhere on disk; a backup
/// is a physical copy of the conventional keys directory, matching
/// `rivet-provision ca backup`'s own fixed-location precedent
/// (`crate::secrets`'s default authority, `~/.rivet/ca`). A seed an operator
/// keeps at a path named only by an environment variable is not backed up by
/// this command, and its own output never claims otherwise.
fn keys_dir() -> Option<PathBuf> {
    credentials::root().map(|root| root.join("keys"))
}

/// The two names this project ever mints, in the order [`backup_usage`] lists them.
/// Not a hard-coded pair copied elsewhere — both constants come from
/// `crate::credentials`, the one place that already owns these file names.
const SEED_NAMES: [&str; 2] = [SYSTEM_SEED_FILE, APP_SEED_FILE];

// ---------------------------------------------------------------------------
// backup
// ---------------------------------------------------------------------------

pub fn backup_usage() -> String {
    format!(
        "usage: cargo rivet key backup <directory>\n\
         \n\
         Encrypts whatever of ~/.rivet/keys/{SYSTEM_SEED_FILE} and {APP_SEED_FILE} actually\n\
         exists — never a hard-coded pair, so a workstation holding only one is backed up\n\
         correctly rather than refused — and writes one <name>.seed.enc file per seed into\n\
         <directory>. <directory> is REQUIRED: there is no default location for a fleet's\n\
         signing seeds, and SPEC-05 rules that out on purpose.\n\
         \n\
         Asks for a passphrase twice, with no echo, from a real terminal. There is no\n\
         --passphrase flag and no environment variable.\n\
         \n\
         Refuses to overwrite a backup already at <directory> — naming it and its age —\n\
         unless --replace is given.\n\
         \n\
         A FORGOTTEN PASSPHRASE MEANS THIS BACKUP CANNOT BE RESTORED. There is no recovery\n\
         route around it, by design (ADR-028: a compromised seed's blast radius is every\n\
         board that trusts its public half).\n\
         \n\
         e.g. cargo rivet key backup D:\\Backups\\rivet-keys\n"
    )
}

pub fn backup(args: &[String]) -> Result<String, String> {
    let (dest, replace) = parse_one_dir(args, "backup")?;
    let source = keys_dir().ok_or(
        "neither HOME nor USERPROFILE is set, so there is no ~/.rivet/keys to back up",
    )?;
    backup_at(&source, &dest, replace)
}

/// [`backup`]'s whole implementation, taking `source` and `dest` explicitly
/// rather than resolving `~/.rivet/keys` itself — the split that lets a test
/// exercise every refusal and the real encrypt-and-write path against
/// scratch directories, and never through the real `HOME`/`USERPROFILE` this
/// session is forbidden from touching.
fn backup_at(source: &Path, dest: &Path, replace: bool) -> Result<String, String> {
    let mut found: Vec<(&str, PathBuf, [u8; SEED_LEN])> = Vec::new();
    for name in SEED_NAMES {
        let path = source.join(name);
        if !path.is_file() {
            continue;
        }
        let seed = crate::key::read_32(&path, "signing seed")?;
        found.push((name, path, seed));
    }
    if found.is_empty() {
        return Err(format!(
            "{} holds neither {SYSTEM_SEED_FILE} nor {APP_SEED_FILE} — nothing to back up",
            source.display()
        ));
    }

    // **Checked before a passphrase is even asked for** — refusing after
    // asking somebody to type and confirm a passphrase would be a wasted
    // ceremony for a backup that was never going anywhere.
    let mut existing: Vec<Existing> = Vec::new();
    for (name, _, _) in &found {
        let target = dest.join(format!("{name}{BACKUP_SUFFIX}"));
        if let Ok(meta) = std::fs::metadata(&target) {
            existing.push(Existing {
                name: format!("{name}{BACKUP_SUFFIX}"),
                bytes: meta.len(),
                age: meta.modified().ok().and_then(|m| {
                    std::time::SystemTime::now().duration_since(m).ok()
                }),
            });
        }
    }
    if !existing.is_empty() && !replace {
        let mut report = format!("{} already holds a backup. NOT overwritten.\n", dest.display());
        for e in &existing {
            report.push_str(&format!(
                "      {:<28} {:>10} bytes  {}\n",
                e.name,
                e.bytes,
                age_text(e.age)
            ));
        }
        report.push_str("  pass --replace to replace it, or choose a different directory.\n");
        return Err(report);
    }

    let passphrase = read_new_passphrase()?;
    backup_with_passphrase(&found, dest, &passphrase)
}

/// The part of [`backup_at`] that actually encrypts and writes, taking the
/// passphrase as a parameter rather than reading a terminal — what a test
/// calls directly to prove the real encrypt-and-write path (and the read
/// path in [`restore_with_passphrase`]) with no terminal involved at all.
fn backup_with_passphrase(
    found: &[(&str, PathBuf, [u8; SEED_LEN])],
    dest: &Path,
    passphrase: &str,
) -> Result<String, String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    let mut report = format!("  encrypted {} seed(s) to {}\n", found.len(), dest.display());
    for (name, _, seed) in found {
        let envelope = encrypt(seed, passphrase)?;
        let target = dest.join(format!("{name}{BACKUP_SUFFIX}"));
        std::fs::write(&target, &envelope)
            .map_err(|e| format!("cannot write {}: {e}", target.display()))?;
        report.push_str(&format!(
            "      {:<28} {:>10} bytes\n",
            format!("{name}{BACKUP_SUFFIX}"),
            envelope.len()
        ));
    }
    report.push_str(&format!(
        "\n  {} file(s) written, each encrypted under the passphrase you just entered.\n",
        found.len()
    ));
    report.push_str(&format!("  {}\n", envelope::FORGOTTEN));
    Ok(report)
}

// ---------------------------------------------------------------------------
// restore
// ---------------------------------------------------------------------------

pub fn restore_usage() -> String {
    format!(
        "usage: cargo rivet key restore <directory> [--replace]\n\
         \n\
         Reads whichever <name>.seed.enc files <directory> holds (from `cargo rivet key\n\
         backup`), asks for the passphrase once, and decrypts each back to\n\
         ~/.rivet/keys/<name> — {SYSTEM_SEED_FILE} and/or {APP_SEED_FILE}.\n\
         \n\
         Refuses a wrong passphrase, or a file altered since it was written, by name and\n\
         WITHOUT WRITING ANYTHING — an AEAD cannot and does not try to tell those two\n\
         apart (see `cargo rivet key backup --help` for why that is the security property,\n\
         not a limitation).\n\
         \n\
         Refuses to overwrite an existing seed at the destination unless --replace is\n\
         given — the same reason `cargo rivet key new` already refuses: there is no way to\n\
         recover a seed that is lost, since every board that trusts its public half would\n\
         need to be re-trusted.\n\
         \n\
         e.g. cargo rivet key restore D:\\Backups\\rivet-keys\n"
    )
}

pub fn restore(args: &[String]) -> Result<String, String> {
    let (source, replace) = parse_one_dir(args, "restore")?;
    let dest = keys_dir().ok_or(
        "neither HOME nor USERPROFILE is set, so there is nowhere to restore a seed to",
    )?;
    restore_at(&source, &dest, replace)
}

/// [`restore`]'s whole implementation — see [`backup_at`]'s doc for why the
/// split exists.
fn restore_at(source: &Path, dest: &Path, replace: bool) -> Result<String, String> {
    let mut found: Vec<(&str, PathBuf)> = Vec::new();
    for name in SEED_NAMES {
        let path = source.join(format!("{name}{BACKUP_SUFFIX}"));
        if path.is_file() {
            found.push((name, path));
        }
    }
    if found.is_empty() {
        return Err(format!(
            "{} holds neither {SYSTEM_SEED_FILE}{BACKUP_SUFFIX} nor {APP_SEED_FILE}{BACKUP_SUFFIX} \
             — nothing to restore",
            source.display()
        ));
    }

    // Same ordering as `backup`: refuse an overwrite before asking for a
    // passphrase, so a doomed restore never makes somebody type one first.
    let mut existing: Vec<Existing> = Vec::new();
    for (name, _) in &found {
        let target = dest.join(name);
        if let Ok(meta) = std::fs::metadata(&target) {
            existing.push(Existing {
                name: (*name).to_owned(),
                bytes: meta.len(),
                age: meta.modified().ok().and_then(|m| {
                    std::time::SystemTime::now().duration_since(m).ok()
                }),
            });
        }
    }
    if !existing.is_empty() && !replace {
        let mut report = format!(
            "{} already holds a seed. NOT restored — there is no way to recover a seed that is \
             lost, since every board that trusts its public half would need to be re-trusted.\n",
            dest.display()
        );
        for e in &existing {
            report.push_str(&format!(
                "      {:<24} {:>10} bytes  {}\n",
                e.name,
                e.bytes,
                age_text(e.age)
            ));
        }
        report.push_str("  pass --replace to restore over it anyway.\n");
        return Err(report);
    }

    let passphrase = read_existing_passphrase()?;
    restore_with_passphrase(&found, dest, &passphrase)
}

/// The part of [`restore_at`] that actually decrypts and writes, taking the
/// passphrase as a parameter — see [`backup_with_passphrase`]'s doc, the
/// identical split for the other direction.
fn restore_with_passphrase(
    found: &[(&str, PathBuf)],
    dest: &Path,
    passphrase: &str,
) -> Result<String, String> {
    // **Every file is opened before anything is written.** A restore that
    // decrypted and wrote its first seed, then refused on its second file's
    // bad passphrase, would leave a half-restored pair — exactly the
    // "half a backup is worse than none" failure `rivet_filecopy`'s own
    // refusal ladder exists to prevent, applied here by hand because this
    // path never calls that crate.
    let mut opened: Vec<(&str, [u8; SEED_LEN])> = Vec::new();
    for (name, path) in found {
        let bytes = read_whole_file(path)?;
        let seed = decrypt(&bytes, passphrase)
            .map_err(|e| format!("{}: {e} Nothing was written.", path.display()))?;
        opened.push((name, seed));
    }

    std::fs::create_dir_all(dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    let mut report = format!("  restored {} seed(s) to {}\n", opened.len(), dest.display());
    for (name, seed) in &opened {
        let target = dest.join(name);
        write_seed_file(&target, seed)?;
        let bytes = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
        report.push_str(&format!("      {name:<24} {bytes:>10} bytes\n"));
    }
    Ok(report)
}

fn read_whole_file(path: &Path) -> Result<Vec<u8>, String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(buffer)
}

/// Write a restored seed with the same platform posture `crate::key::new_seed`
/// already takes — mode 0600 set in the `open` on Unix, the directory's own
/// ACL on Windows (stated, not pretended). Not a second implementation of
/// that logic: `crate::key`'s `write_seed` is private to its module, and a
/// restored seed is 32 raw bytes exactly as a minted one is, so this is the
/// same two platform bodies restated here rather than exported for one
/// caller — `ADR-011`'s cost either way is small; making a private helper
/// `pub(crate)` for a single second caller is the smaller one.
#[cfg(unix)]
fn write_seed_file(path: &Path, seed: &[u8; SEED_LEN]) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    file.write_all(seed).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    file.sync_all().map_err(|e| format!("cannot flush {}: {e}", path.display()))
}

#[cfg(not(unix))]
fn write_seed_file(path: &Path, seed: &[u8; SEED_LEN]) -> Result<(), String> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    file.write_all(seed).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    file.sync_all().map_err(|e| format!("cannot flush {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// shared arg parsing
// ---------------------------------------------------------------------------

fn parse_one_dir(args: &[String], verb: &str) -> Result<(PathBuf, bool), String> {
    let mut replace = false;
    let mut positional: Vec<&String> = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--replace" => replace = true,
            other if other.starts_with("--") => {
                return Err(format!(
                    "{other:?} is not an option of `key {verb}`. There is no --passphrase flag \
                     and no environment variable — see `cargo rivet key {verb} --help`.",
                ));
            }
            _ => positional.push(arg),
        }
    }
    let [dir] = positional.as_slice() else {
        return Err(format!(
            "usage: cargo rivet key {verb} <directory>{}\n<directory> is REQUIRED — there is no \
             default location.",
            if verb == "restore" { " [--replace]" } else { "" }
        ));
    };
    Ok((PathBuf::from(dir), replace))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        crate::scratch::dir(&format!(
            "key-backup-{name}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ))
    }

    const A_SEED: [u8; SEED_LEN] = [7u8; SEED_LEN];
    const A_PASSPHRASE: &str = "correct horse battery staple, fictional and fixed";
    const A_DIFFERENT_PASSPHRASE: &str = "a different fictional passphrase entirely";

    // -- the envelope: encrypt/decrypt, pure, no filesystem -----------------

    /// **An envelope that opens to anything but 32 bytes is not a seed** —
    /// refused as a format problem, never truncated or padded into one.
    #[test]
    fn an_envelope_that_is_not_32_bytes_is_not_a_seed() {
        let envelope = envelope::seal(SEED_ENVELOPE, &[7u8; 31], A_PASSPHRASE).expect("seals");
        match decrypt(&envelope, A_PASSPHRASE) {
            Err(OpenError::Format(why)) => assert!(why.contains("not 32 bytes"), "{why}"),
            other => panic!("expected Format, got {other:?}"),
        }
    }

    /// **A board's secrets backup handed to `key restore` is the wrong kind of
    /// file**, refused by its magic — the two share an envelope, not a meaning.
    #[test]
    fn a_secrets_backup_envelope_is_not_a_seed_backup() {
        let secrets = envelope::Kind { magic: b"RVTSECR", noun: "a secrets backup" };
        let file = envelope::seal(secrets, &[7u8; SEED_LEN], A_PASSPHRASE).expect("seals");
        match decrypt(&file, A_PASSPHRASE) {
            Err(OpenError::Format(why)) => assert!(why.contains("not a Rivet seed backup"), "{why}"),
            other => panic!("expected Format, got {other:?}"),
        }
    }

    /// **The round trip.** What `cargo rivet key backup` then `key restore`
    /// do to one seed, with no terminal or filesystem involved.
    #[test]
    fn a_seed_round_trips_through_encrypt_and_decrypt() {
        let envelope = encrypt(&A_SEED, A_PASSPHRASE).expect("encrypts");
        let opened = decrypt(&envelope, A_PASSPHRASE).expect("decrypts under the same passphrase");
        assert_eq!(opened, A_SEED);
    }

    /// **Two encryptions of the same seed are not the same bytes** — the
    /// fresh random salt and nonce this module's own doc promises, checked
    /// rather than assumed.
    #[test]
    fn two_encryptions_of_the_same_seed_differ() {
        let a = encrypt(&A_SEED, A_PASSPHRASE).expect("encrypts");
        let b = encrypt(&A_SEED, A_PASSPHRASE).expect("encrypts");
        assert_ne!(a, b, "salt and nonce must be fresh every time");
    }

    /// **A wrong passphrase is refused, and the error names what it is** —
    /// not a format problem, the AEAD's own authentication failure.
    #[test]
    fn a_wrong_passphrase_is_refused() {
        let envelope = encrypt(&A_SEED, A_PASSPHRASE).expect("encrypts");
        match decrypt(&envelope, A_DIFFERENT_PASSPHRASE) {
            Err(OpenError::WrongPassphraseOrTampered) => {}
            other => panic!(
                "expected WrongPassphraseOrTampered, got {}",
                other.map(|_| "Ok").unwrap_or("a different Err")
            ),
        }
    }

    /// **A single flipped ciphertext byte is refused the identical way a
    /// wrong passphrase is** — the module doc's claim, checked: an AEAD does
    /// not and must not distinguish the two.
    #[test]
    fn a_tampered_ciphertext_is_refused_identically_to_a_wrong_passphrase() {
        let mut envelope = encrypt(&A_SEED, A_PASSPHRASE).expect("encrypts");
        let last = envelope.len() - 1;
        envelope[last] ^= 0xFF;
        match decrypt(&envelope, A_PASSPHRASE) {
            Err(OpenError::WrongPassphraseOrTampered) => {}
            other => panic!(
                "expected WrongPassphraseOrTampered, got {}",
                other.map(|_| "Ok").unwrap_or("a different Err")
            ),
        }
    }

    /// **A tampered PARAMETER (not the ciphertext) is caught too** —
    /// flipping a byte of the salt, which is authenticated but not secret,
    /// must not silently derive a different key and appear to work; the
    /// AEAD's AAD binding is what makes this a refusal rather than a
    /// different-but-plausible decryption.
    #[test]
    fn a_tampered_salt_byte_is_refused() {
        let mut envelope = encrypt(&A_SEED, A_PASSPHRASE).expect("encrypts");
        // Byte 22 is the first salt byte (offset 7+1+1+4+4+4+1 = 22).
        envelope[22] ^= 0xFF;
        assert!(matches!(
            decrypt(&envelope, A_PASSPHRASE),
            Err(OpenError::WrongPassphraseOrTampered)
        ));
    }

    /// **Bad magic is a format refusal, not a passphrase refusal** — the two
    /// error variants say different things, and a corrupted or foreign file
    /// should say so rather than implying the passphrase was wrong.
    #[test]
    fn bad_magic_is_a_format_error_not_a_passphrase_error() {
        let mut envelope = encrypt(&A_SEED, A_PASSPHRASE).expect("encrypts");
        envelope[0] = b'X';
        match decrypt(&envelope, A_PASSPHRASE) {
            Err(OpenError::Format(why)) => assert!(why.contains("not a Rivet seed backup")),
            other => panic!("expected Format, got {}", other.map(|_| "Ok").unwrap_or("WrongPassphraseOrTampered")),
        }
    }

    /// **A truncated file is a format refusal**, at every truncation point —
    /// nothing here should panic or read out of bounds.
    #[test]
    fn a_truncated_file_is_a_format_error_at_every_length() {
        let envelope = encrypt(&A_SEED, A_PASSPHRASE).expect("encrypts");
        for len in 0..envelope.len() {
            match decrypt(&envelope[..len], A_PASSPHRASE) {
                Err(OpenError::Format(_)) => {}
                Err(OpenError::WrongPassphraseOrTampered) => {
                    // Only reachable once the full ciphertext length is
                    // present but the buffer is otherwise complete, which
                    // this loop's range (< envelope.len()) never produces —
                    // treated as a hard failure rather than silently passed.
                    panic!("a truncated buffer produced an AEAD failure instead of a format one at len {len}");
                }
                Ok(_) => panic!("a truncated buffer at len {len} decrypted successfully"),
            }
        }
    }

    /// **An unsupported format version is refused by name.**
    #[test]
    fn an_unsupported_version_is_refused_by_name() {
        let mut envelope = encrypt(&A_SEED, A_PASSPHRASE).expect("encrypts");
        envelope[envelope::MAGIC_LEN] = 99;
        match decrypt(&envelope, A_PASSPHRASE) {
            Err(OpenError::Format(why)) => assert!(why.contains("version 99"), "{why}"),
            other => panic!("expected Format, got {}", other.map(|_| "Ok").unwrap_or("WrongPassphraseOrTampered")),
        }
    }

    // -- backup_with_passphrase / restore_with_passphrase: real files -------

    /// **The full round trip through the real CLI logic**, passphrase
    /// supplied directly rather than through a terminal (that half is
    /// covered separately, in `tests/pty_key_backup.rs`, which this session
    /// did not run — see that file's own module doc). Both seeds present,
    /// both survive.
    #[test]
    fn backup_then_restore_round_trips_both_seeds_through_real_files() {
        let base = scratch("round-trip");
        let keys = base.join("keys");
        std::fs::create_dir_all(&keys).expect("a keys directory");
        std::fs::write(keys.join(SYSTEM_SEED_FILE), [1u8; SEED_LEN]).expect("write");
        std::fs::write(keys.join(APP_SEED_FILE), [2u8; SEED_LEN]).expect("write");

        let backup_dir = base.join("backup");
        let found: Vec<(&str, PathBuf, [u8; SEED_LEN])> = SEED_NAMES
            .iter()
            .map(|name| {
                let path = keys.join(name);
                let seed = crate::key::read_32(&path, "signing seed").expect("reads");
                (*name, path, seed)
            })
            .collect();
        let report =
            backup_with_passphrase(&found, &backup_dir, A_PASSPHRASE).expect("backs up");
        assert!(report.contains("2 file(s) written"), "{report}");
        assert!(backup_dir.join(format!("{SYSTEM_SEED_FILE}{BACKUP_SUFFIX}")).is_file());
        assert!(backup_dir.join(format!("{APP_SEED_FILE}{BACKUP_SUFFIX}")).is_file());

        // Restore into a FRESH destination, never the live `keys` above.
        let restored_keys = base.join("restored-keys");
        let restore_found: Vec<(&str, PathBuf)> = SEED_NAMES
            .iter()
            .map(|name| (*name, backup_dir.join(format!("{name}{BACKUP_SUFFIX}"))))
            .collect();
        let report = restore_with_passphrase(&restore_found, &restored_keys, A_PASSPHRASE)
            .expect("restores");
        assert!(report.contains("2 seed(s)"), "{report}");
        assert_eq!(
            std::fs::read(restored_keys.join(SYSTEM_SEED_FILE)).expect("read back"),
            vec![1u8; SEED_LEN]
        );
        assert_eq!(
            std::fs::read(restored_keys.join(APP_SEED_FILE)).expect("read back"),
            vec![2u8; SEED_LEN]
        );
    }

    /// **A wrong passphrase at restore time writes nothing** — checked by
    /// asserting the destination directory was never even created.
    #[test]
    fn restore_with_the_wrong_passphrase_writes_nothing() {
        let base = scratch("wrong-passphrase-writes-nothing");
        let backup_dir = base.join("backup");
        let found: Vec<(&str, PathBuf, [u8; SEED_LEN])> =
            vec![(SYSTEM_SEED_FILE, PathBuf::new(), [3u8; SEED_LEN])];
        backup_with_passphrase(&found, &backup_dir, A_PASSPHRASE).expect("backs up");

        let restored_keys = base.join("never-created");
        let restore_found = vec![(SYSTEM_SEED_FILE, backup_dir.join(format!("{SYSTEM_SEED_FILE}{BACKUP_SUFFIX}")))];
        let err = restore_with_passphrase(&restore_found, &restored_keys, A_DIFFERENT_PASSPHRASE)
            .expect_err("a wrong passphrase must refuse");
        assert!(err.contains("wrong passphrase"), "{err}");
        assert!(!restored_keys.exists(), "nothing should have been written");
    }

    // -- backup_at / restore_at: the overwrite refusal and --replace --------

    /// **`backup_at` refuses an existing backup before ever reading a
    /// terminal** — provable because the refusal error is the overwrite
    /// sentence, never the "no terminal" one, even though this test process
    /// has no terminal on stdin either.
    #[test]
    fn backup_at_refuses_an_existing_backup_before_asking_for_a_passphrase() {
        let base = scratch("backup-overwrite");
        let keys = base.join("keys");
        std::fs::create_dir_all(&keys).expect("a keys directory");
        std::fs::write(keys.join(SYSTEM_SEED_FILE), [4u8; SEED_LEN]).expect("write");

        let dest = base.join("dest");
        std::fs::create_dir_all(&dest).expect("a destination directory");
        std::fs::write(dest.join(format!("{SYSTEM_SEED_FILE}{BACKUP_SUFFIX}")), b"already here")
            .expect("write");

        let err = backup_at(&keys, &dest, false).expect_err("an existing backup must refuse");
        assert!(err.contains("already holds a backup"), "{err}");
        assert!(!err.contains("terminal"), "must refuse on the overwrite, not the terminal: {err}");
        assert_eq!(
            std::fs::read(dest.join(format!("{SYSTEM_SEED_FILE}{BACKUP_SUFFIX}"))).expect("unchanged"),
            b"already here"
        );
    }

    /// **`--replace` gets past the overwrite refusal** — proven by the error
    /// changing from "already holds a backup" to the terminal refusal (this
    /// test process has none on stdin), which is only reachable once the
    /// overwrite check has been satisfied.
    #[test]
    fn replace_gets_backup_at_past_the_overwrite_refusal() {
        let base = scratch("backup-replace");
        let keys = base.join("keys");
        std::fs::create_dir_all(&keys).expect("a keys directory");
        std::fs::write(keys.join(SYSTEM_SEED_FILE), [5u8; SEED_LEN]).expect("write");

        let dest = base.join("dest");
        std::fs::create_dir_all(&dest).expect("a destination directory");
        std::fs::write(dest.join(format!("{SYSTEM_SEED_FILE}{BACKUP_SUFFIX}")), b"already here")
            .expect("write");

        let err = backup_at(&keys, &dest, true).expect_err("no terminal in a test process");
        assert!(err.contains("terminal"), "expected the overwrite check to have been passed: {err}");
    }

    /// **`restore_at` refuses an existing live seed before asking for a
    /// passphrase**, the identical shape as `backup_at`'s own check.
    #[test]
    fn restore_at_refuses_an_existing_seed_before_asking_for_a_passphrase() {
        let base = scratch("restore-overwrite");
        let backup_dir = base.join("backup");
        std::fs::create_dir_all(&backup_dir).expect("a backup directory");
        std::fs::write(backup_dir.join(format!("{SYSTEM_SEED_FILE}{BACKUP_SUFFIX}")), b"envelope")
            .expect("write");

        let dest = base.join("keys");
        std::fs::create_dir_all(&dest).expect("a destination directory");
        std::fs::write(dest.join(SYSTEM_SEED_FILE), [6u8; SEED_LEN]).expect("an existing live seed");

        let err = restore_at(&backup_dir, &dest, false).expect_err("an existing seed must refuse");
        assert!(err.contains("already holds a seed"), "{err}");
        assert!(!err.contains("terminal"), "must refuse on the overwrite, not the terminal: {err}");
        assert_eq!(std::fs::read(dest.join(SYSTEM_SEED_FILE)).expect("unchanged"), vec![6u8; SEED_LEN]);
    }

    /// **No source at all — neither seed's backup file present — is refused
    /// by name**, before any terminal is touched.
    #[test]
    fn restore_at_with_no_backup_files_present_is_refused() {
        let base = scratch("restore-empty");
        let backup_dir = base.join("backup");
        std::fs::create_dir_all(&backup_dir).expect("an empty backup directory");
        let dest = base.join("keys");
        let err = restore_at(&backup_dir, &dest, false).expect_err("nothing to restore");
        assert!(err.contains("nothing to restore"), "{err}");
    }

    // -- CLI argument parsing ------------------------------------------------

    #[test]
    fn parse_one_dir_requires_exactly_one_directory() {
        let err = parse_one_dir(&[], "backup").expect_err("no directory given");
        assert!(err.contains("REQUIRED"), "{err}");
        let err = parse_one_dir(
            &["a".to_owned(), "b".to_owned()],
            "backup",
        )
        .expect_err("two directories given");
        assert!(err.contains("REQUIRED"), "{err}");
    }

    #[test]
    fn parse_one_dir_reads_replace() {
        let (dir, replace) =
            parse_one_dir(&["D:\\x".to_owned(), "--replace".to_owned()], "restore").expect("parses");
        assert_eq!(dir, PathBuf::from("D:\\x"));
        assert!(replace);
    }

    #[test]
    fn parse_one_dir_refuses_an_unknown_flag() {
        let err = parse_one_dir(&["D:\\x".to_owned(), "--passphrase".to_owned(), "hunter2".to_owned()], "backup")
            .expect_err("no --passphrase flag exists");
        assert!(err.contains("no --passphrase flag"), "{err}");
    }
}
