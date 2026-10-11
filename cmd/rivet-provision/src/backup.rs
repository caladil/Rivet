//! **Copying a fleet's authority out, and back in.**
//!
//! # The gap this closed
//!
//! Nothing in this crate copied the authority anywhere, and `cmd/rivet-reset`
//! deletes it file by file — `remove_file` on each, then `remove_dir` — with
//! no check that a copy exists first. `root.key` is the sole copy of the
//! fleet's whole trust; losing it orphans every certificate the fleet's
//! machines carry, with no way to issue another. `SPEC-05` §4.
//!
//! # This module is now a thin wrapper — 2026-09-14
//!
//! The engine that used to live here (`copy_all`, refuse-or-`--replace`, a
//! named file list with sizes and ages, byte-for-byte round trip) moved to
//! `rivet-filecopy` at its third consumer: `cargo rivet key backup`/`restore`
//! is the second and `cargo rivet fleet objects backup`/`restore` is the
//! third, and neither of those lives in this crate — `rivet-filecopy/
//! Cargo.toml` has the full reasoning. **`ca backup`/`restore`'s own
//! behaviour and output are unchanged**: this module re-exports the same
//! names it used to define, so `main.rs`'s `ca_backup`/`ca_restore` and this
//! module's own test suite (below) call the identical function they always
//! did, byte for byte, word for word.

pub use rivet_filecopy::{age_text, copy_all, Copied, Error, Existing};

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use std::path::PathBuf;

    /// One scratch directory per test, so parallel tests do not collide —
    /// `AGENTS.md` §10.5's shared-cache warning is about the container, and
    /// the same discipline applies to any test that touches real paths.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rivet-provision-backup-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    /// **A missing source directory is refused, not read as empty.** `Empty`
    /// and an absent directory are different facts, and the first silently
    /// including the second would tell an operator a backup was legitimately
    /// made of nothing.
    #[test]
    fn a_missing_source_is_an_io_error_not_an_empty_backup() {
        let base = scratch("missing-source");
        let source = base.join("does-not-exist");
        let dest = base.join("dest");
        match copy_all(&source, &dest, false) {
            Err(Error::Io(why)) => assert!(why.contains("does-not-exist"), "{why}"),
            other => panic!("expected Io, got a different outcome: {}", other.is_ok()),
        }
    }

    /// **An empty source directory is refused by name.** Writing zero files
    /// and calling that success is the shape of failure this exists to
    /// prevent — a backup command that ran and reported nothing wrong.
    #[test]
    fn an_empty_source_directory_is_refused() {
        let base = scratch("empty-source");
        let source = base.join("empty");
        std::fs::create_dir_all(&source).expect("an empty directory");
        let dest = base.join("dest");
        assert!(matches!(copy_all(&source, &dest, false), Err(Error::Empty(_))));
    }

    /// **The report names every file copied, with its size.** This is the
    /// maintainer's requirement (.rulings/ R-337): the report says exactly what
    /// was copied, file by file, with sizes — the evidence, not somebody's
    /// memory of having run the command.
    #[test]
    fn every_file_is_named_and_sized() {
        let base = scratch("names-and-sizes");
        let source = base.join("source");
        std::fs::create_dir_all(&source).expect("a source directory");
        std::fs::write(source.join("root.crt"), b"the root certificate").expect("a file");
        std::fs::write(source.join("root.key"), b"the key").expect("a file");
        let dest = base.join("dest");

        let copied = copy_all(&source, &dest, false).expect("copies");
        assert_eq!(copied.len(), 2);
        let root_crt = copied.iter().find(|c| c.name == "root.crt").expect("root.crt");
        assert_eq!(root_crt.bytes, "the root certificate".len() as u64);
        let root_key = copied.iter().find(|c| c.name == "root.key").expect("root.key");
        assert_eq!(root_key.bytes, "the key".len() as u64);
        assert_eq!(
            std::fs::read(dest.join("root.crt")).expect("readable"),
            b"the root certificate"
        );
    }

    /// **A subdirectory in the source is left alone.** The authority
    /// directory is flat by construction; this asserts the module does not
    /// guess at what a subdirectory would mean.
    #[test]
    fn a_subdirectory_in_the_source_is_not_copied() {
        let base = scratch("subdirectory");
        let source = base.join("source");
        std::fs::create_dir_all(source.join("nested")).expect("a nested directory");
        std::fs::write(source.join("root.crt"), b"x").expect("a file");
        std::fs::write(source.join("nested").join("leftover"), b"y").expect("a nested file");
        let dest = base.join("dest");

        let copied = copy_all(&source, &dest, false).expect("copies");
        assert_eq!(copied.len(), 1, "only the flat file should be reported");
        assert!(!dest.join("nested").exists(), "the subdirectory was not copied");
    }

    /// **The refusal to overwrite an existing backup, and what it names.**
    /// This is the requirement in the maintainer's own words: a backup that
    /// quietly replaced last week's good copy with today's broken one is the
    /// failure the verb exists to prevent, so it must refuse and say what is
    /// already there.
    #[test]
    fn an_existing_file_at_the_destination_refuses_the_whole_copy() {
        let base = scratch("existing-backup");
        let source = base.join("source");
        std::fs::create_dir_all(&source).expect("a source directory");
        std::fs::write(source.join("root.crt"), b"new").expect("a file");
        let dest = base.join("dest");
        std::fs::create_dir_all(&dest).expect("a destination directory");
        std::fs::write(dest.join("root.crt"), b"old, and already here").expect("a file");

        match copy_all(&source, &dest, false) {
            Err(Error::WouldOverwrite(existing)) => {
                assert_eq!(existing.len(), 1);
                assert_eq!(existing[0].name, "root.crt");
                assert_eq!(existing[0].bytes, "old, and already here".len() as u64);
            }
            other => panic!("expected WouldOverwrite, got a different outcome: {}", other.is_ok()),
        }
        // **Nothing was copied.** A refused backup must not have partially
        // replaced what was already at the destination.
        assert_eq!(
            std::fs::read(dest.join("root.crt")).expect("still readable"),
            b"old, and already here"
        );
    }

    /// **`replace` is the explicit word that overrides the refusal**, and
    /// only that flag does — there is no silent path around it.
    #[test]
    fn replace_overwrites_what_was_there() {
        let base = scratch("replace");
        let source = base.join("source");
        std::fs::create_dir_all(&source).expect("a source directory");
        std::fs::write(source.join("root.crt"), b"new").expect("a file");
        let dest = base.join("dest");
        std::fs::create_dir_all(&dest).expect("a destination directory");
        std::fs::write(dest.join("root.crt"), b"old").expect("a file");

        let copied = copy_all(&source, &dest, true).expect("replace overrides the refusal");
        assert_eq!(copied.len(), 1);
        assert_eq!(std::fs::read(dest.join("root.crt")).expect("readable"), b"new");
    }

    /// **The round trip.** Mint what a throwaway authority would leave on
    /// disk, back it up, delete the "live" copy the way `rivet-reset` would,
    /// restore it, and assert every byte survived. This is `copy_all` used
    /// in both directions — `main.rs`'s `ca_backup` and `ca_restore` are thin
    /// wrappers around exactly this function, so proving it here proves the
    /// file half of both verbs without going near a certificate store.
    #[test]
    fn a_backup_and_a_restore_round_trip_every_byte() {
        let base = scratch("round-trip");
        let authority = base.join("authority");
        std::fs::create_dir_all(&authority).expect("an authority directory");
        let files: &[(&str, &[u8])] = &[
            ("root.crt", b"root certificate bytes"),
            ("root.key", b"root key bytes -- the fleet's whole trust"),
            ("browser.crt", b"browser certificate bytes"),
            ("browser.key", b"browser key bytes"),
            ("suffix", b"local\nhome-fleet.home.arpa"),
            ("server-arbor.local.crt", b"a fleet server's certificate"),
            ("server-arbor.local.key", b"a fleet server's key"),
        ];
        for (name, body) in files {
            std::fs::write(authority.join(name), body).expect("a file");
        }

        // Back it up.
        let backup = base.join("backup");
        let copied = copy_all(&authority, &backup, false).expect("backs up");
        assert_eq!(copied.len(), files.len());

        // Destroy the "live" authority — what rivet-reset does.
        for (name, _) in files {
            std::fs::remove_file(authority.join(name)).expect("remove");
        }
        std::fs::remove_dir(&authority).expect("remove the now-empty directory");
        assert!(!authority.exists());

        // Restore it.
        let restored = copy_all(&backup, &authority, false).expect("restores");
        assert_eq!(restored.len(), files.len());

        // Every byte survived the round trip.
        for (name, body) in files {
            assert_eq!(
                std::fs::read(authority.join(name)).unwrap_or_else(|e| panic!("{name}: {e}")),
                *body,
                "{name} did not survive the round trip"
            );
        }
    }

    #[test]
    fn age_is_worded_for_a_person() {
        assert_eq!(age_text(None), "age unknown");
        assert_eq!(age_text(Some(std::time::Duration::from_secs(3600))), "less than a day old");
        assert_eq!(age_text(Some(std::time::Duration::from_secs(90_000))), "1 day old");
        assert_eq!(age_text(Some(std::time::Duration::from_secs(5 * 86_400))), "5 days old");
    }
}
