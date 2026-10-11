//! **Read an installed secret by name** — `ADR-038` §5, `.rulings/` R-169
//! and R-169.1.
//!
//! # Where `install-secret` writes, and why nothing new had to be handed over
//!
//! `rivet apply install-secret <application> <name> --hex <hex>`
//! (`rivet_core::operation::install_secret`) writes directly into the
//! application's own writable directory — `<data directory>/<name>`, and that
//! data directory is **exactly** [`crate::app::data`]: the supervisor sets
//! `RIVET_APP_DATA` to `rivet_core::service::data_dir(volume, name)`, the same
//! path `install-secret` composes its own write from. So an application
//! already had everything it needed to find a secret by name — the one thing
//! missing was a reader that joins the name on and treats absence as
//! ordinary, which is the whole of what this module adds.
//!
//! This is the opposite of `crate::settings`'s history: that file *left*
//! `RIVET_APP_DATA` in 2026-09-04 because writing root-owned bytes by path
//! into a directory an unprivileged uid controls is a symlink hazard
//! (`ADR-038` §8.1). `ADR-038` §8.1.3 kept `install-secret` where it was, on
//! purpose, for a reason that applies here too: **the application must be
//! able to rewrite its own credential file**, and `install-secret`'s own
//! `SECRET_MODE` is `0600` rather than `0400` for exactly that. A root-owned,
//! settings-style relocation would take that back. So no `RIVET_APP_SECRETS`
//! variable exists, and this module reads through [`crate::app::data`] rather
//! than composing a second one.
//!
//! # Absent and unreadable are the same answer
//!
//! `crate::settings`'s rule, restated for the second channel `ADR-038` §5
//! names: a machine nobody has installed a secret for is the ordinary
//! machine, not a fault. [`get`] and [`get_bytes`] give `None` for a file that
//! is missing, unreadable, or — for [`get`] only — not valid UTF-8. Nothing
//! here returns an error, and a caller that wants to tell those apart already
//! has [`path`] to look at the filesystem itself.
//!
//! # This never logs or prints the value, and neither should a caller
//!
//! `rivet_core::secret::Secret` redacts a value it holds by construction;
//! this module hands back a plain `String`/`Vec<u8>` because a caller that
//! must use a credential cannot avoid holding it in the clear at some point —
//! the platform's job stops at not being the one to leak it, exactly as
//! `apply install-secret`'s own plan never echoes the value it is installing
//! (`ADR-020` §4). **Nothing in this module's own code paths, including its
//! tests, ever logs or prints a value it reads.**
//!
//! # Unified with TempMon and Aura — `.rulings/` R-169.1
//!
//! Before this call existed, `aura-controller` read its relay account's
//! credential with its own `credential::read`, composing the path from its
//! config rather than from a name handed to one SDK call, and TempMon — a
//! separate repository — had no equivalent at all and kept its write-token in
//! the machine document as an ordinary setting, which `ADR-038` §4.2's
//! application region has no schema to refuse that with. This call is the
//! "unified way" the ruling asks for: every application that wants a
//! credential asks for it **by name**, the same way, whether that name is
//! `relay.env` or `write-token`.

use std::path::PathBuf;

/// The longest name [`path`] accepts — `rivet_core::operation::SECRET_NAME_MAX_LEN`,
/// restated. Re-declared rather than imported for `ADR-036` §8.3's reason: an
/// application must not link `rivet-core`, so `tests/agreement.rs` is what
/// buys the right to restate this number rather than import it.
pub const NAME_MAX_LEN: usize = 64;

/// One installed secret's raw bytes.
///
/// `None` when this process was not handed a data directory
/// ([`crate::app::data`] failed), `name` is not a name `install-secret` could
/// have used, or the file is not there or cannot be read. All three are the
/// same answer to a caller: nothing was installed under that name.
///
/// # This is the value. Do not format it.
///
/// Wrap the result yourself if you need to hold it — nothing here can stop a
/// caller from putting it in a `format!`, which is exactly the accident
/// `rivet_core::secret::Secret`'s missing `Display` exists to catch on the
/// platform's own side.
#[must_use]
pub fn get_bytes(name: &str) -> Option<Vec<u8>> {
    std::fs::read(path(name)?).ok()
}

/// The same secret, decoded as UTF-8 text — the common case, since
/// `install-secret`'s own examples (an Ed25519 seed aside) are credentials
/// somebody typed.
///
/// `None` for bytes that are not valid UTF-8, on the same footing as a
/// missing file: this module cannot report *which* failure it was without
/// risking the bytes on the way past (a `Utf8Error` carries the byte that
/// broke it), and a caller that must tell the two apart has [`get_bytes`].
#[must_use]
pub fn get(name: &str) -> Option<String> {
    String::from_utf8(get_bytes(name)?).ok()
}

/// Where a secret named `name` would be, for this application.
///
/// `None` when [`crate::app::data`] was not handed — this process was not
/// started by a Rivet supervisor — or `name` is not [`is_secret_name`]. A
/// name is never guessed at or sanitised: a `..` or a path separator composed
/// onto a directory this application owns is exactly the traversal
/// `rivet_core::operation::install_secret::check_name` refuses on the write
/// side, so this side refuses to compose a path from one at all rather than
/// normalising it into something that happens to still be safe today.
#[must_use]
pub fn path(name: &str) -> Option<PathBuf> {
    if !is_secret_name(name) {
        return None;
    }
    crate::app::data().ok().map(|dir| dir.join(name))
}

/// Whether `text` is a name `apply install-secret` could have used —
/// `rivet_core::operation::is_secret_name`, restated. `tests/agreement.rs`
/// is what buys the restatement: a name this side accepts and the platform's
/// write side would have refused is not a failure that shows itself, because
/// the file this reads just never exists.
#[must_use]
pub fn is_secret_name(text: &str) -> bool {
    if text.is_empty() || text.len() > NAME_MAX_LEN {
        return false;
    }
    let mut characters = text.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    characters.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

/// The mode [`install`] gives a secret file — `0o600`, the same number
/// `rivet_core::operation::install_secret::SECRET_MODE` writes. Re-declared
/// rather than imported (`ADR-036` §8: an application must not link
/// `rivet-core`), and `tests/agreement.rs` is what keeps the two equal.
///
/// `0o600` and not `0o400` for the reason the platform's own write is: **the
/// application must be able to rewrite its own credential file.**
pub const MODE: u32 = 0o600;

/// The most bytes [`install`] accepts — the same cap
/// `rivet_core::operation::install_secret::SECRET_MAX_BYTES` puts on the
/// operator's channel, so a value the dialog stores is one the platform would
/// have. Re-declared for the same reason as [`MODE`], guarded the same way.
pub const MAX_BYTES: usize = 64 * 1024;

/// **Why [`install`] could not write the secret.** Each variant is a distinct
/// mistake; none of them ever carries the value.
#[derive(Debug)]
pub enum InstallError {
    /// This process was not started by a Rivet supervisor, so there is no
    /// application data directory to write into — the same cause
    /// [`crate::app::data`] reports as `NotHanded`.
    NotHanded,
    /// `name` is not a name `install-secret` could have used — see
    /// [`is_secret_name`]. Refused rather than sanitised, exactly as [`path`]
    /// refuses to compose a path from one.
    BadName,
    /// The value is larger than [`MAX_BYTES`], the cap the platform's own
    /// channel enforces.
    TooLarge {
        /// How many bytes were offered.
        len: usize,
        /// The most that would be accepted — [`MAX_BYTES`].
        max: usize,
    },
    /// The write itself failed. The `io::Error` names the syscall and the
    /// path, never the bytes.
    Io(std::io::Error),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotHanded => write!(
                f,
                "this process was not handed an application data directory, so there is nowhere \
                 to write a secret"
            ),
            Self::BadName => write!(
                f,
                "that is not a name `install-secret` could have used, so no file is composed for it"
            ),
            Self::TooLarge { len, max } => {
                write!(f, "a secret of {len} bytes is larger than the {max}-byte limit")
            }
            Self::Io(e) => write!(f, "writing the secret failed: {e}"),
        }
    }
}

impl std::error::Error for InstallError {}

/// **Write the secret named `name` for this application** — the same file
/// `rivet apply install-secret <this application> <name>` writes, from inside
/// the application rather than from the operator's control plane.
///
/// # It is one file with two writers, not two stores
///
/// [`get_bytes`] reads `RIVET_APP_DATA/<name>`; `install-secret` writes exactly
/// there, at exactly [`MODE`], owned by this application's own uid — the SDK
/// module note above is the account. This call is the application's own hand on
/// that same file. It is not a second, weaker place a credential can live: it
/// is the place the platform already defined, written through the same name
/// rule and the same mode, and read back by [`get_bytes`] with nothing
/// downstream told that the operator's verb was not the one that wrote it.
///
/// It deliberately does **not** reach the control-plane `install-secret`
/// operation: that channel authenticates the operator's pinned key, which an
/// application has no way to present to itself and does not need to — the file
/// is the mechanism, and the verb is the operator's way of writing it.
///
/// # The write is atomic and never widens the file
///
/// The bytes go to a temporary this call creates EXCLUSIVELY, under a name
/// only this one call chose — never a name built from `name` and the process
/// id alone, which let two installs of the same secret, run concurrently in
/// one process, collide on a single file. It is created at [`MODE`] so it is
/// never briefly world-readable, and that temporary is renamed over `name`.
/// A reader therefore sees either the whole old secret or the whole new one,
/// never a half-written credential and never a mix of two installs' bytes.
///
/// The directory is flushed after the rename too, best-effort — the same
/// footing `apply install-secret`'s own directory fsync stands on: the
/// rename has already happened by then, so a directory that could not be
/// flushed is a question of surviving a power cut and not a reason to
/// report the secret unwritten.
///
/// # It never logs the value, and neither should a caller
///
/// Nothing here formats `bytes`. The same discipline `get_bytes`'s note asks of
/// a reader is asked of a writer: the platform's job stops at not being the one
/// to leak it.
pub fn install(name: &str, bytes: &[u8]) -> Result<(), InstallError> {
    if !is_secret_name(name) {
        return Err(InstallError::BadName);
    }
    if bytes.len() > MAX_BYTES {
        return Err(InstallError::TooLarge {
            len: bytes.len(),
            max: MAX_BYTES,
        });
    }
    let dir = crate::app::data().map_err(|_| InstallError::NotHanded)?;
    let final_path = dir.join(name);
    let tmp = write_new_private(&dir, name, bytes).map_err(InstallError::Io)?;
    if let Err(e) = std::fs::rename(&tmp, &final_path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(InstallError::Io(e));
    }
    // Best effort; see the doc comment above for why this is not a refusal.
    if let Ok(parent) = std::fs::File::open(&dir) {
        let _ = parent.sync_all();
    }
    Ok(())
}

/// How many candidate temporary names one call will try before giving up.
/// Each candidate already differs from the last by a counter unique within
/// this process, so colliding this many times running means the directory
/// itself refuses writes rather than that luck ran out.
const MAX_TEMPORARY_ATTEMPTS: u32 = 1000;

/// Create a NEW, exclusively-created private file beside `name` inside
/// `dir`, under a name this call picks for itself, write `bytes` to it, and
/// set its mode through the same open file — returning the path it chose so
/// the caller can rename it into place.
///
/// **Two installs of the same secret no longer share one name.** The
/// previous scheme built the name from `name` and this process's pid
/// alone — identical for every thread of this process installing the same
/// secret at once — and opened it with `create(true).truncate(true)` rather
/// than exclusively, so one call could truncate another's bytes, publish
/// the other's bytes as its own, or remove the other's temporary from its
/// own error-cleanup path. This name also carries a counter unique to the
/// call and the clock, so two calls in one process never choose the same
/// one, and it is opened with `create_new` so a name that IS somehow
/// already taken — including by a symbolic link, however it got there — is
/// never opened, followed or truncated; a taken name is simply skipped for
/// another.
///
/// On unix the file is created at [`MODE`] and its permissions are then set
/// again, explicitly, on the same open descriptor — never by a second,
/// path-based lookup of the name — so a restrictive umask cannot leave it
/// wider *or* narrower than the platform's own secret. A narrower one would
/// make [`get_bytes`] fail to read back what was just written.
///
/// # Errors
///
/// Every failure — to find an unused name, to write, or to set the mode —
/// removes only the ONE temporary this call itself created and reports what
/// the kernel said.
fn write_new_private(
    dir: &std::path::Path,
    name: &str,
    bytes: &[u8],
) -> std::io::Result<std::path::PathBuf> {
    use std::io::Write as _;

    for _ in 0..MAX_TEMPORARY_ATTEMPTS {
        let candidate = dir.join(unique_temporary_name(name));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(MODE);
        }

        let mut file = match options.open(&candidate) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };

        let sealed = (|| -> std::io::Result<()> {
            file.write_all(bytes)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                file.set_permissions(std::fs::Permissions::from_mode(MODE))?;
            }
            file.sync_all()
        })();

        return match sealed {
            Ok(()) => Ok(candidate),
            Err(e) => {
                drop(file);
                let _ = std::fs::remove_file(&candidate);
                Err(e)
            }
        };
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!(
            "no unused temporary name for {name:?} in {} after {MAX_TEMPORARY_ATTEMPTS} attempts",
            dir.display()
        ),
    ))
}

/// **A temporary name nothing already waiting in `dir` could have
/// predicted.** See [`write_new_private`]'s doc comment for why that matters.
fn unique_temporary_name(name: &str) -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(".{name}.{}.{n:x}-{nanos:x}.tmp", std::process::id())
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_name_with_no_handed_directory_finds_nothing() {
        // No `RIVET_APP_DATA` in this test process — the ordinary state of a
        // program not started by a Rivet supervisor.
        assert!(get("relay.env").is_none());
        assert!(get_bytes("write-token").is_none());
        assert!(path("relay.env").is_none());
    }

    /// Mirrors `rivet_core::operation::install_secret`'s `NAME_RULE` cases,
    /// restated so this module's own predicate is exercised directly rather
    /// than only through the agreement test.
    #[test]
    fn the_naming_rule_matches_what_install_secret_would_accept() {
        for good in ["relay.env", "write-token", "a", "A1", "cert.pem", "x".repeat(64).as_str()] {
            assert!(is_secret_name(good), "{good:?} should be a valid name");
        }
        for bad in [
            "",
            ".",
            "..",
            "-leading",
            "has/slash",
            "has\\backslash",
            "has space",
            &"x".repeat(65),
        ] {
            assert!(!is_secret_name(bad), "{bad:?} should not be a valid name");
        }
    }

    /// `path` refuses to compose anything for a name it does not accept,
    /// rather than sanitising it — the traversal case must be unrepresentable,
    /// not merely caught.
    #[test]
    fn path_refuses_a_bad_name_even_with_no_directory_handed() {
        for bad in ["..", "a/b", ""] {
            assert!(path(bad).is_none(), "{bad:?}");
        }
    }

    /// A unique scratch directory for a test that must touch the filesystem.
    /// No `tempfile` dependency reaches this crate — an application links it —
    /// so this composes one under the system temp dir and removes it after.
    fn scratch(tag: &str) -> std::path::PathBuf {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "rivet-sdk-secrets-{tag}-{}-{now}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("make scratch dir");
        dir
    }

    /// The writer puts the exact bytes down and, on unix, at exactly [`MODE`] —
    /// the guarantee [`get_bytes`] depends on to read back what was written.
    #[test]
    fn write_new_private_writes_the_bytes_and_the_mode() {
        let dir = scratch("write");
        let chosen = write_new_private(&dir, "relay.env", b"secret-bytes").expect("write");
        assert_eq!(std::fs::read(&chosen).expect("read back"), b"secret-bytes");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&chosen).expect("stat").permissions().mode();
            assert_eq!(mode & 0o777, MODE, "the secret must be owner-only");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Two installs of the same secret, interleaved, never share a
    /// temporary.** The seam under test is [`write_new_private`] directly
    /// rather than [`install`], whose data directory comes from an
    /// environment variable this test must not touch — every other test in
    /// this binary can run concurrently with it and some depend on that
    /// variable being absent. Both threads are handed one scratch directory
    /// explicitly instead, and a `Barrier` forces the overlap the old
    /// shared, non-exclusive temporary used to allow silently.
    #[test]
    fn two_concurrent_installs_of_the_same_secret_never_share_a_temporary() {
        let dir = scratch("concurrent");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));

        let first = vec![b'A'; 4096];
        let second = vec![b'B'; 4096];

        let spawn = |payload: Vec<u8>, dir: std::path::PathBuf, barrier: std::sync::Arc<std::sync::Barrier>| {
            std::thread::spawn(move || {
                barrier.wait();
                write_new_private(&dir, "shared", &payload)
            })
        };
        let a = spawn(first.clone(), dir.clone(), std::sync::Arc::clone(&barrier));
        let b = spawn(second.clone(), dir.clone(), barrier);

        let path_a = a.join().expect("thread a did not panic").expect("install a");
        let path_b = b.join().expect("thread b did not panic").expect("install b");

        // **Never the same temporary.** The old scheme named it from the
        // secret and the pid alone — identical for both threads of one
        // process — which is the whole defect this replaces.
        assert_ne!(path_a, path_b, "two installs shared one temporary");

        // Each temporary holds exactly one COMPLETE offered value: never a
        // mix of the two, and never truncated by the other's write.
        let bytes_a = std::fs::read(&path_a).expect("read a's temporary");
        let bytes_b = std::fs::read(&path_b).expect("read b's temporary");
        assert!(
            bytes_a == first || bytes_a == second,
            "a's temporary does not hold one complete offered value"
        );
        assert!(
            bytes_b == first || bytes_b == second,
            "b's temporary does not hold one complete offered value"
        );
        assert_ne!(
            bytes_a, bytes_b,
            "both temporaries ended up holding the same payload"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The full flow, concurrent, racing onto one shared final name.**
    /// Two threads install the SAME secret with different payloads at
    /// once; whichever rename wins, the published file holds one complete
    /// value — never a mix — and nothing but that one file is left in the
    /// directory afterward, which is the "failed cleanup cannot remove
    /// another operation's file" property: neither install's cleanup path
    /// runs at all here, because with unique temporaries neither write can
    /// fail on account of the other.
    #[test]
    fn two_concurrent_installs_of_the_same_secret_each_publish_one_complete_value() {
        let dir = scratch("publish");
        let final_path = dir.join("shared-secret");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));

        let attempt = |payload: Vec<u8>,
                       dir: std::path::PathBuf,
                       final_path: std::path::PathBuf,
                       barrier: std::sync::Arc<std::sync::Barrier>| {
            std::thread::spawn(move || -> std::io::Result<()> {
                barrier.wait();
                let tmp = write_new_private(&dir, "shared-secret", &payload)?;
                if let Err(e) = std::fs::rename(&tmp, &final_path) {
                    let _ = std::fs::remove_file(&tmp);
                    return Err(e);
                }
                Ok(())
            })
        };

        let first = vec![b'A'; 8192];
        let second = vec![b'B'; 8192];
        let a = attempt(
            first.clone(),
            dir.clone(),
            final_path.clone(),
            std::sync::Arc::clone(&barrier),
        );
        let b = attempt(second.clone(), dir.clone(), final_path.clone(), barrier);

        a.join().expect("thread a did not panic").expect("install a");
        b.join().expect("thread b did not panic").expect("install b");

        let published = std::fs::read(&final_path).expect("the published secret");
        assert!(
            published == first || published == second,
            "the published secret is neither offered value complete"
        );

        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .expect("scratch dir")
            .filter_map(Result::ok)
            .map(|e| e.file_name())
            .filter(|name| name != "shared-secret")
            .collect();
        assert!(leftovers.is_empty(), "stray files left behind: {leftovers:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A stale temporary at the name the OLD scheme would have used
    /// blocks nothing and is never touched.** That name was built only from
    /// the secret name and the pid — guessable by anything that can
    /// already write into this application's own data directory — so this
    /// is the realistic version of "a candidate name already taken":
    /// the one name an attacker could actually have predicted, rather than
    /// one carrying a counter and a timestamp it could not.
    #[test]
    fn a_stale_temporary_at_the_old_predictable_name_is_never_touched() {
        let dir = scratch("stale");
        let predictable = dir.join(format!(".shared.{}.tmp", std::process::id()));
        std::fs::write(&predictable, b"stale, from a process that died")
            .expect("plant a stale temporary");

        let chosen =
            write_new_private(&dir, "shared", b"fresh bytes").expect("a name it did not need");
        assert_ne!(chosen, predictable, "the write reused the predictable name");
        assert_eq!(
            std::fs::read(&predictable).expect("the stale temporary"),
            b"stale, from a process that died",
            "the stale temporary was touched"
        );
        assert_eq!(std::fs::read(&chosen).expect("the fresh temporary"), b"fresh bytes");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A symbolic link at that same predictable name is neither followed
    /// nor removed.** `create_new` refuses a name a symlink already
    /// occupies without ever resolving it, so the link and whatever it
    /// points at are exactly as this call found them.
    #[cfg(unix)]
    #[test]
    fn a_symlink_at_the_old_predictable_name_is_never_followed_or_removed() {
        let dir = scratch("stale-symlink");
        let predictable = dir.join(format!(".shared.{}.tmp", std::process::id()));
        let target = dir.join("do-not-touch");
        std::fs::write(&target, b"not this file").expect("the symlink's target");
        std::os::unix::fs::symlink(&target, &predictable).expect("plant a symlink");

        let chosen =
            write_new_private(&dir, "shared", b"fresh bytes").expect("a name it did not need");
        assert_ne!(chosen, predictable);
        assert_eq!(
            std::fs::read_link(&predictable).expect("the symlink"),
            target,
            "the symlink was replaced or removed"
        );
        assert_eq!(
            std::fs::read(&target).expect("the symlink's target"),
            b"not this file",
            "the symlink was followed and its target written through"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The size cap and the name rule are both checked before the data
    /// directory is even consulted, so both refuse the same way whether or not
    /// this process was handed one.
    #[test]
    fn install_refuses_a_bad_name_and_an_oversized_value() {
        assert!(
            matches!(install("has/slash", b"x"), Err(InstallError::BadName)),
            "a name with a separator is refused, not sanitised"
        );
        let too_big = vec![0u8; MAX_BYTES + 1];
        assert!(
            matches!(install("relay.env", &too_big), Err(InstallError::TooLarge { .. })),
            "a value past the cap is refused before any write"
        );
    }
}
