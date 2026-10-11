//! **Loading an addon is a trial boot, one level down.** `.rulings/` R-198 D3.
//!
//! A loaded addon shares the host's address space, so an addon that writes
//! through a wild pointer takes the host down and no rule in this crate can
//! stop it. What this module stops is the SECOND time: a host that crashed
//! while loading an addon finds the note it wrote, skips that addon, says so
//! in its journal, and comes up without it.
//!
//! The cost of a memory-unsafe addon is then one restart and a card that is
//! not there, rather than a crash loop that takes the machine's screen with
//! it. `ADR-018` makes exactly this trade for a system image — a trial
//! boot that is not committed rolls back with no userspace involvement — and
//! the shape is borrowed deliberately, including the part that matters most:
//! **withholding the confirmation is the rollback.** Nothing has to be done to
//! cause it, which is what makes it work when the thing that went wrong was a
//! process dying without warning.
//!
//! # Where the note lives, and why not in the document
//!
//! `RIVET_APP_DATA/addons/<class>.loading` — the host's own private data
//! directory, `0700` and owned by the host's uid. It is deliberately not the
//! machine's document: a document is configuration the operator owns and a
//! converging supervisor rewrites, while this is one process's memory of what
//! it was doing a moment before it died.
//!
//! # The note names the object, so a reinstall is what clears it
//!
//! A note used to carry nothing but its own existence, and the documented
//! recovery — "reinstalling the addon clears that" — had no code behind it:
//! [`forgive`] was reachable from nowhere in the tree, so a crash mid-load
//! retired an addon for as long as the note survived, reinstall or not. The
//! note now carries [`begin`]'s `identity` argument, and [`left_behind`]
//! compares it against the object about to be loaded: the SAME object is the
//! crash-loop protection this module exists for and stays skipped, a
//! DIFFERENT one means something changed since the crash — an upgrade, a
//! reinstall — and is forgiven automatically, with no operator action beyond
//! the reinstall itself.
//!
//! A note written before this change carries no identity at all, and
//! [`left_behind`] reads that case as [`LeftBehind::Unidentified`] rather
//! than guessing: there is nothing recorded to compare the current object
//! against, so treating it as "different" would be inventing an answer and
//! treating it as "same" would be assuming the very crash loop this exists to
//! catch. The caller skips the addon exactly as it always did, and the note
//! is left for an operator to clear by hand until a load actually happens
//! under this version and gives it something to compare against.

use std::path::{Path, PathBuf};

/// The note for one class, under `data_dir`.
#[must_use]
pub fn note_path(data_dir: &Path, class: &str) -> PathBuf {
    data_dir.join("addons").join(format!("{class}.loading"))
}

/// **A note written before this module recorded identity.** Never written by
/// [`begin`] any more; read back so a pre-existing note is told apart from
/// one this version wrote.
const UNIDENTIFIED: &str = "loading";

/// What a note left behind says, compared against the object about to be
/// loaded now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftBehind {
    /// No note. The previous load, if there was one, reached [`settled`].
    Clear,
    /// A note naming the SAME object now being loaded — the crash-loop
    /// protection this module exists for. The caller skips the addon and
    /// journals it by name.
    Same,
    /// A note naming a DIFFERENT object than the one now being loaded. The
    /// object changed since the crash — an upgrade or a reinstall — so the
    /// caller forgives the note and loads the new one.
    Different,
    /// A note written before this module recorded identity. There is nothing
    /// to compare the current object against, so the caller treats it the
    /// way an unnamed note always was: skip once more, conservatively, rather
    /// than guess.
    Unidentified,
}

/// **Was this addon being loaded when the host last stopped, and does the
/// note still name the object about to be loaded?**
///
/// `identity` is the object the caller is about to load — see
/// [`crate::host::Installed::identity`] for where that comes from.
///
/// **Never fails.** A note that cannot be read — removed since, or a
/// permissions problem — reads as [`LeftBehind::Clear`] rather than blocking
/// a load over a file this call cannot even see the contents of, the same
/// direction [`settled`] already fails safe in.
#[must_use]
pub fn left_behind(data_dir: &Path, class: &str, identity: &str) -> LeftBehind {
    let Ok(recorded) = std::fs::read_to_string(note_path(data_dir, class)) else {
        return LeftBehind::Clear;
    };
    match recorded.trim() {
        UNIDENTIFIED => LeftBehind::Unidentified,
        same if same == identity => LeftBehind::Same,
        _ => LeftBehind::Different,
    }
}

/// Write the note, immediately before opening the object.
///
/// `identity` is recorded verbatim, and is what a later [`left_behind`]
/// compares against — see the module header for why it must not be the
/// bare word `loading`, which is how an identity-less note from before
/// identities were recorded reads. [`crate::host::Installed::identity`] never
/// produces that string: a reference directory or an absolute path, neither
/// of which is ever the bare word `loading`.
///
/// # Errors
/// When the note cannot be written. **A host that cannot write the note must
/// not load the addon**: an unrecorded load is one that cannot be rolled back,
/// which is the whole mechanism gone.
pub fn begin(data_dir: &Path, class: &str, identity: &str) -> std::io::Result<()> {
    let path = note_path(data_dir, class);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, format!("{identity}\n"))
}

/// Clear the note, once the addon has answered.
///
/// **Called after the first successful exchange, not after the `dlopen`.** An
/// object that loads and then faults on its first call has not settled, and
/// the note is what makes the next start skip it.
///
/// # Errors
/// When the note exists and cannot be removed — which leaves the addon
/// skipped next time, the safe direction to fail in.
pub fn settled(data_dir: &Path, class: &str) -> std::io::Result<()> {
    let path = note_path(data_dir, class);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Forget that an addon was skipped, so the next start tries it again.
///
/// The operator's escape hatch: reinstalling an addon, or asking for it
/// explicitly, clears the note. Without this a single crash would retire an
/// addon for ever, which is a worse failure than the one it prevents.
///
/// # Errors
/// When the note exists and cannot be removed.
pub fn forgive(data_dir: &Path, class: &str) -> std::io::Result<()> {
    settled(data_dir, class)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rivet-addon-trial-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    #[test]
    fn a_load_that_settles_leaves_nothing_behind() {
        let dir = scratch("settles");
        assert_eq!(left_behind(&dir, "test-source", "obj-a"), LeftBehind::Clear);
        begin(&dir, "test-source", "obj-a").expect("note written");
        assert_eq!(left_behind(&dir, "test-source", "obj-a"), LeftBehind::Same);
        settled(&dir, "test-source").expect("note cleared");
        assert_eq!(left_behind(&dir, "test-source", "obj-a"), LeftBehind::Clear);
    }

    /// The property the whole mechanism exists for: nothing runs after the
    /// host dies, so the note has to be what survives rather than anything
    /// the host does on the way down.
    #[test]
    fn a_host_that_dies_mid_load_is_told_so_on_the_next_start() {
        let dir = scratch("dies");
        begin(&dir, "reading-source", "obj-a").expect("note written");
        // No `settled` — this is the crash.
        assert_eq!(
            left_behind(&dir, "reading-source", "obj-a"),
            LeftBehind::Same,
            "the next start must find the note and read it as the same object"
        );
        forgive(&dir, "reading-source").expect("forgiven");
        assert_eq!(left_behind(&dir, "reading-source", "obj-a"), LeftBehind::Clear);
    }

    /// **The crash-loop protection.** A note recorded against object A is
    /// still skipped when the object about to be loaded is still A — the
    /// whole reason this mechanism exists, and the case that must never be
    /// silently forgiven.
    #[test]
    fn a_note_against_the_same_object_is_still_skipped() {
        let dir = scratch("same-object");
        begin(&dir, "reading-source", "aura-reading-source-1.0.0-aaaa").expect("note written");
        assert_eq!(
            left_behind(&dir, "reading-source", "aura-reading-source-1.0.0-aaaa"),
            LeftBehind::Same
        );
    }

    /// **The escape hatch, finally with a caller.** A note recorded against
    /// object A is forgiven — read as [`LeftBehind::Different`] — when the
    /// object now installed is B, which is what an upgrade or a reinstall
    /// changes automatically, with no separate command.
    #[test]
    fn a_note_against_a_different_object_is_forgiven() {
        let dir = scratch("different-object");
        begin(&dir, "reading-source", "aura-reading-source-1.0.0-aaaa").expect("note written");
        assert_eq!(
            left_behind(&dir, "reading-source", "aura-reading-source-1.0.1-bbbb"),
            LeftBehind::Different
        );
    }

    /// **A note from before this module recorded identity.** There is
    /// nothing to compare, so it reads as [`LeftBehind::Unidentified`] rather
    /// than being guessed at as either same or different — for any object
    /// now being asked about, including one that happens to share bytes with
    /// what crashed.
    #[test]
    fn an_identity_less_note_is_unidentified_regardless_of_what_is_asked() {
        let dir = scratch("unidentified");
        // Simulates a note this module wrote before it recorded identity —
        // `begin` itself can no longer produce this content.
        let path = note_path(&dir, "reading-source");
        std::fs::create_dir_all(path.parent().expect("has a parent")).expect("dir");
        std::fs::write(&path, b"loading\n").expect("legacy note written");

        assert_eq!(
            left_behind(&dir, "reading-source", "aura-reading-source-1.0.0-aaaa"),
            LeftBehind::Unidentified
        );
        assert_eq!(
            left_behind(&dir, "reading-source", "anything-else-entirely"),
            LeftBehind::Unidentified
        );
    }

    #[test]
    fn one_addons_note_says_nothing_about_another() {
        let dir = scratch("separate");
        begin(&dir, "reading-source", "obj-a").expect("note written");
        assert_eq!(
            left_behind(&dir, "temperature-source", "obj-a"),
            LeftBehind::Clear
        );
    }

    #[test]
    fn clearing_a_note_that_is_not_there_is_not_an_error() {
        let dir = scratch("absent");
        settled(&dir, "never-loaded").expect("not an error");
    }
}
