//! **The kiosk's display selection, kept across a reboot.**
//!
//! `Supervisor::display_selection` (`crate::supervise`) is which application
//! holds the screen, and it is *runtime* state: it starts as `None` at
//! construction and is set only by a live `select_display`. That is right while
//! the machine runs, and wrong the instant it reboots — nothing wrote the
//! selection down, so `apply_display_gate` comes up with `None` and disables
//! every offered display application. A compositor-drawing app such as
//! `aura-renderer` is therefore silently held off after any restart, its
//! `restarts` count stuck at `0`, until an operator selects it again.
//!
//! This module is the missing durability: **persist on select, clear on return,
//! restore on boot.**
//!
//! # It is not the document, and that is the whole reason it is here
//!
//! `crate::document` is the OFFERED set — what `enabled` says a machine may put
//! on its screen — and it is the operator's. The *selection* is which of those
//! is running, and writing it into the document would make every button press on
//! a remote control a configuration change (`crate::supervise`'s
//! `display_selection` says the same). So the selection gets its own small state
//! file, next to `crate::clock`'s and `crate::repos`', written the crash-safe
//! way through `crate::durable`.
//!
//! # It never stops a boot
//!
//! Every failure here is reported and the machine carries on — `crate::clock`'s
//! reading exactly. A selection that could not be restored costs a blank screen
//! until the operator selects again; a boot that refused to start the control
//! plane over it would cost the machine. `ADR-018` §8: a machine that can be
//! reached is one somebody can repair.

use crate::durable::{self, Durability};
use crate::journal::{Severity, Value};
use crate::kernel::{Errno, Kernel};
use crate::supervise::Supervisor;

/// The file's name inside `rivet_core::state::STATE_MOUNT_POINT`.
///
/// The **file name** only; the directory comes from `rivet-core`, which is the
/// one place `/state` is spelled (`ADR-011`) — the same division
/// `rivet_store::clock::CLOCK_FILE` keeps.
pub const KIOSK_SELECTION_FILE: &str = "kiosk-selection";

/// The mode the selection is written with — `0600`.
///
/// It is written and read only by this process, which runs as root, so nothing
/// else needs to reach it. `crate::clock` writes its stored clock `0644` because
/// an operator reads it while diagnosing; nobody reads this one by hand, so it
/// stays as tight as the task asks.
pub const KIOSK_SELECTION_MODE: u32 = 0o600;

/// Where the selection is stored.
///
/// Composed from `rivet_core::state::STATE_MOUNT_POINT` rather than spelling
/// `/state` a second time — `crate::clock` reaches the same directory the same
/// way.
#[must_use]
pub fn path() -> String {
    format!(
        "{}/{KIOSK_SELECTION_FILE}",
        rivet_core::state::STATE_MOUNT_POINT
    )
}

/// **Write the selected application's name down**, crash-safe.
///
/// Called from `crate::main`'s `Request::Select` handler after
/// `Supervisor::select_display` has returned `Ok` — the selection is already
/// live in memory, so a failure here is logged by the caller and the machine
/// carries on. It only means the next boot comes up with nothing selected,
/// which is exactly the state this whole module exists to avoid, but no worse
/// than before it existed.
///
/// Through `crate::durable` for the reason its own header gives: a
/// truncate-in-place would leave a window in which the file is empty, and an
/// empty file reads as *nothing selected* — a power cut in that window would
/// hold the screen off on the next boot without a word.
pub fn persist(k: &mut dyn Kernel, name: &str) -> Result<Durability, Errno> {
    durable::commit(
        k,
        &path(),
        format!("{name}\n").as_bytes(),
        KIOSK_SELECTION_MODE,
    )
}

/// **Forget the selection**, because the launcher took the screen back.
///
/// Called from `crate::main`'s `Request::Return` handler after
/// `Supervisor::return_display`. The file is removed rather than emptied: absent
/// and empty both read as *nothing selected* to [`restore`], and removing it
/// leaves nothing behind to reason about.
///
/// **A file that is already gone is not a failure.** Return can be asked when
/// nothing was ever selected, or twice, so `ENOENT` is the ordinary answer and
/// is folded into success — the same reading `crate::clock`'s rotation gives a
/// remove of a file that may never have existed.
pub fn clear(k: &mut dyn Kernel) -> Result<(), Errno> {
    match k.remove_file(&path()) {
        Ok(()) | Err(Errno::ENOENT) => Ok(()),
        Err(e) => Err(e),
    }
}

/// **Restore the selection this boot inherited**, if any, so the kiosk resumes
/// the application it was showing before the reboot.
///
/// # Where this sits, and why it is a contract
///
/// `crate::main` calls it once, right after `Supervisor::new` has computed the
/// offered set and `set_kiosk_mode` has run. The offered set has to exist first,
/// because `Supervisor::select_display` validates against it — and it does, the
/// constructor reads it before the gate narrows it.
///
/// # Every arm carries on
///
/// * **No file, or an empty one** — the ordinary first boot, and a machine whose
///   launcher last held the screen itself. Nothing is restored and nothing is
///   said: a line per boot about the absence of a selection is noise.
/// * **A name that is no longer offered** — the application was uninstalled, or
///   the document stopped enabling it, between the write and this boot.
///   `select_display` refuses it, and that refusal is the signal to drop the
///   stale file rather than carry a selection that can never take. Logged at
///   info, not treated as an error.
/// * **A file that will not read** — `/state` answering `EIO`. Reported and left
///   in place: it is not this module's to delete a file it could not read, the
///   same call `crate::clock` declines to make about a stored clock it could not
///   parse.
pub fn restore(k: &mut dyn Kernel, sup: &mut Supervisor) {
    let path = path();

    let stored = match k.read_file(&path) {
        Ok(text) => text,
        // No selection stored is the ordinary case and says nothing.
        Err(Errno::ENOENT) => return,
        Err(e) => {
            sup.note(
                k,
                Severity::Warn,
                "the kiosk's stored display selection could not be read, so this machine comes up \
                 with nothing on the screen until an application is selected again. Carrying on: \
                 the fault is on /state, which no update or rollback touches (ADR-018 §8)",
                vec![
                    ("path".to_string(), Value::Str(path)),
                    ("detail".to_string(), Value::Str(e.to_string())),
                ],
            );
            return;
        }
    };

    let name = stored.trim();
    // Absent and empty are the same fact: nothing was selected.
    if name.is_empty() {
        return;
    }

    match sup.select_display(name) {
        Ok(_) => sup.note(
            k,
            Severity::Info,
            "the kiosk resumed the application it was showing before this boot. The selection \
             survived the reboot in /state; the convergence at the bottom of \
             the loop starts it, by the same rules every service is subject to",
            vec![("svc_name".to_string(), Value::Str(name.to_string()))],
        ),
        Err(why) => {
            // The stored application is no longer offered — uninstalled, or the
            // document stopped enabling it. The selection cannot take, so the
            // stale file is dropped rather than carried.
            let cleared = clear(k);
            let mut fields = vec![
                ("svc_name".to_string(), Value::Str(name.to_string())),
                ("detail".to_string(), Value::Str(why)),
            ];
            if let Err(e) = cleared {
                fields.push(("clear_errno".to_string(), Value::Str(e.to_string())));
            }
            sup.note(
                k,
                Severity::Info,
                "the application this kiosk had selected before the reboot is no longer offered a \
                 screen on this machine, so the stored selection is stale and has been cleared. \
                 The machine comes up with nothing selected rather than refusing to boot",
                fields,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::testkernel::{FakeKernel, Script};

    /// **(a) Persisting a selection writes the name, crash-safe.** The bytes
    /// land on a sibling temp and reach the target by `rename(2)`, at `0600` —
    /// the same sequence `crate::durable` holds and `crate::repos` uses for a
    /// repository's identifier.
    #[test]
    fn persisting_a_selection_writes_the_name_through_the_crash_safe_sequence() {
        let mut k = FakeKernel::new();
        assert_eq!(persist(&mut k, "aura-renderer"), Ok(Durability::Flushed));

        assert_eq!(k.contents(&path()).as_deref(), Some("aura-renderer\n"));

        let temp = format!("{}.new", path());
        let write = k
            .writes
            .iter()
            .find(|w| w.path == temp)
            .expect("the selection was written to the sibling temp");
        assert_eq!(write.mode, KIOSK_SELECTION_MODE);
        assert!(
            !k.truncations.contains(&path()),
            "the target was truncated in place: {:?}",
            k.truncations
        );
        assert!(
            k.renames.contains(&(temp, path())),
            "the name was not committed by rename: {:?}",
            k.renames
        );
    }

    /// **(d) Returning the screen clears the file.** Nothing is left on `/state`
    /// for the next boot to restore.
    #[test]
    fn clearing_the_selection_removes_the_file() {
        let mut k = FakeKernel::new();
        k.files_mut()
            .insert(path(), Script::always(Ok("aura-renderer\n".to_string())));

        assert_eq!(clear(&mut k), Ok(()));
        assert!(k.removed.contains(&path()), "the file was not removed: {:?}", k.removed);
        assert_eq!(k.contents(&path()), None, "the selection outlived the return");
    }

    /// ...and clearing a selection that was never stored is not a failure — a
    /// return can be asked when nothing held the screen, or asked twice.
    #[test]
    fn clearing_when_nothing_is_stored_is_not_an_error() {
        let mut k = FakeKernel::new();
        assert_eq!(clear(&mut k), Ok(()), "an absent selection made return fail");
    }

    /// A remove that fails for a reason other than absence is surfaced, not
    /// swallowed: `ENOENT` is the only errno `clear` folds into success, and a
    /// card answering `EIO` is a fault the caller has to be told about.
    #[test]
    fn a_remove_that_fails_for_any_reason_but_absence_is_surfaced() {
        let mut k = FakeKernel::new();
        // Present, so the remove is attempted rather than short-circuited as
        // "nothing to do", and then made to fail.
        k.files_mut()
            .insert(path(), Script::always(Ok("aura-renderer\n".to_string())));
        k.remove_file_results = Script::always(Err(Errno::EIO));
        assert_eq!(clear(&mut k), Err(Errno::EIO), "an EIO was swallowed as success");
    }

    // The restore tests, which need a full `Supervisor` and an offered display
    // application, live in `crate::supervise`'s test module — that is where the
    // offered-display fixtures (`kiosk_def`, `showing`) already are.
}
