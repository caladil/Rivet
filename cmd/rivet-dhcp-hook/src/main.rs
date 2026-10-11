//! **The program `udhcpc` execs when something happens to a lease.**
//!
//! `ADR-004` §5 rules that a mechanism's own configuration file does not exist
//! on a Rivet system and that the mechanism is *driven at runtime*. `udhcpc` is
//! unusually well suited to that, and it is why it was chosen over `dhcpcd`:
//! **it configures nothing itself**. It speaks DHCP, and when it has an answer
//! it execs a program with the lease in that program's environment. Every
//! decision about the answer belongs to the program it execs, which is this
//! one.
//!
//! # Why this is its own binary rather than part of `rivet-netd`
//!
//! Because `udhcpc` execs its script **once per event**, and every event would
//! then be a `rivet-netd` of its own, racing the supervisor's run of it to
//! configure the same interface.
//!
//! So this program does the smallest possible thing: parse the environment into
//! a lease, write it where netd will look, and nudge the supervisor, which runs
//! netd again when the lease is new or different. **It performs no netlink
//! operation at all** — `ADR-011`'s one authority per fact, where the fact is
//! *what address does this interface have*, and `rivet-netd` stays the only
//! thing on the machine that answers it.
//!
//! # There is no shell, and it does not matter
//!
//! Every `udhcpc` tutorial hands it a `/bin/sh` script. `ADR-008` §3 rejects the
//! FHS and there is no shell in a Rivet image — the applet is deliberately
//! absent from `tools/buildroot/busybox/busybox.config`. `udhcpc` does not
//! care: it `exec`s whatever path it was given, and a static Rust binary is a
//! better answer for the same reasons everything else here is one.
//!
//! # The judgement is in [`fn@decide`], and it touches nothing
//!
//! This file is the edge: arguments in, filesystem out. Everything that decides
//! *what* should happen is a pure function a test can drive.

mod decide;

use std::io::Write as _;

use decide::{Action, decide};
use rivet_adapter_net::lease::LEASE_DIR;

/// Mode `0700` on the directory.
///
/// A lease is not a secret and this is not about secrecy: `/run` is a tmpfs
/// every process can read, and an address written there by something that is
/// not this program would be believed by `rivet-netd`. Root-only is the
/// cheapest way to say that only root writes here.
const DIR_MODE: libc::mode_t = 0o700;

fn main() -> std::process::ExitCode {
    let word = std::env::args().nth(1).unwrap_or_default();
    match decide(&word, &|key| std::env::var(key).ok()).and_then(perform) {
        Ok(said) => {
            report(&said);
            std::process::ExitCode::SUCCESS
        }
        // **Non-zero, deliberately.** `udhcpc` does not act on the exit status,
        // but it logs a script that failed — and that log is the only trace a
        // machine which silently never got an address would otherwise leave.
        Err(why) => {
            report(&format!("FAILED: {why}"));
            std::process::ExitCode::FAILURE
        }
    }
}

/// Say something where the supervisor will keep it.
///
/// **stderr, and it is not a fallback.** `udhcpc` is a supervised service and
/// this program inherits its descriptors, so this reaches the journal under
/// the client's service name the way every other program's output does
/// (`ADR-022`). It is also why
/// `CONFIG_FEATURE_SYSLOG` is off in the BusyBox configuration: with syslog
/// compiled in, `udhcpc`'s own messages would go somewhere nothing here reads.
fn report(what: &str) {
    let mut err = std::io::stderr();
    let _ = writeln!(err, "rivet-dhcp-hook: {what}");
    let _ = err.flush();
}

/// Carry out what [`fn@decide`] decided.
fn perform(action: Action) -> Result<String, String> {
    match action {
        Action::Write { path, text } => {
            make_directory()?;
            write_atomically(&path, &text)?;
            let summary: String = text
                .lines()
                .filter(|l| l.starts_with("ip=") || l.starts_with("mask=") || l.starts_with("router="))
                .collect::<Vec<_>>()
                .join(" ");
            // **Ask the supervisor to reload now that a lease exists.** The
            // reload is what runs `rivet-netd` to put the lease's address on
            // the interface, and what re-projects `/etc/resolv.conf` with the
            // lease's `dns=`. Best effort: a nudge that cannot be delivered is
            // reported and shrugged off, because the lease is written
            // regardless and the next reload picks it up.
            let nudge = nudge_supervisor();
            Ok(format!("wrote {path} — {summary}; {nudge}"))
        }
        Action::Remove { path } => match std::fs::remove_file(&path) {
            // **And the supervisor is asked to reload**, which runs
            // `rivet-netd` again to take the withdrawn lease's address off the
            // interface (`ADR-004` §8).
            Ok(()) => {
                let nudge = nudge_supervisor();
                Ok(format!(
                    "{path} removed — this interface has no address from DHCP; {nudge}"
                ))
            }
            // Already absent is the ORDINARY case, not a fault: `deconfig` is
            // the first thing udhcpc says, before any lease has ever existed.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(format!("no lease was held ({path})"))
            }
            Err(e) => Err(format!("cannot remove {path}: {e}")),
        },
    }
}

/// **Nudge `rivet-sysd` to reload — best effort, never fatal.**
///
/// `SIGHUP` is what asks the supervisor to re-read — the same signal `apply
/// reload` sends (`ADR-032` §3). Its reload reads the lease this hook just
/// wrote and runs `rivet-netd` again when it is new or different, so that the
/// address reaches the interface; and it re-projects `/etc`, where
/// `leased_resolver` takes the lease's `dns=` into `/etc/resolv.conf`, which is
/// otherwise projected once, at boot, before DHCP has an answer
/// (`cmd/rivet-sysd/src/etc.rs`).
///
/// **It finds the supervisor by name rather than by `getppid`.** The parent
/// here is `udhcpc`, the service, and the supervisor is its parent in turn; a
/// name is the identification that does not depend on how many processes sit
/// between. It looks for the one process whose `/proc/<pid>/comm`
/// is `rivet-sysd` and whose parent is pid 1 — the supervisor (`ADR-017` §3
/// says there is exactly one), the same identification `cmd/rivet`'s reload
/// adapter makes. Zero, several, or an unreadable `/proc` are each reported and
/// shrugged off: the lease was written, and the next reload picks the resolver
/// up.
fn nudge_supervisor() -> String {
    let entries = match std::fs::read_dir("/proc") {
        Ok(entries) => entries,
        Err(e) => return format!("supervisor not nudged (/proc unreadable: {e})"),
    };
    let mut found: Vec<i32> = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|s| s.parse::<i32>().ok())
        else {
            continue;
        };
        // A process that vanished between the listing and this read is simply
        // not a match, which is the common case on a busy machine.
        let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm")) else {
            continue;
        };
        // `comm` is `TASK_COMM_LEN` and truncates at 15, which is why the
        // supervisor's name is short enough to compare whole.
        if comm.trim_end() == "rivet-sysd" && parent_pid(pid) == Some(1) {
            found.push(pid);
        }
    }
    match found.as_slice() {
        [pid] => {
            // SAFETY: `kill(2)` takes a pid and a signal number and touches no
            // memory. `ESRCH` — the supervisor exited between the scan and here
            // — is not worth failing a lease write over.
            let rc = unsafe { libc::kill(*pid, libc::SIGHUP) };
            if rc == 0 {
                format!("nudged the supervisor (SIGHUP to pid {pid}) to reload and apply it")
            } else {
                format!(
                    "supervisor pid {pid} not nudged: {}",
                    std::io::Error::last_os_error()
                )
            }
        }
        [] => "supervisor not nudged (no rivet-sysd child of pid 1 in /proc)".to_string(),
        many => format!(
            "supervisor not nudged ({} processes named rivet-sysd, refusing to choose)",
            many.len()
        ),
    }
}

/// The parent pid from `/proc/<pid>/stat`.
///
/// **Read past the `comm` field on the last `)`.** `comm` is parenthesised and
/// may itself contain spaces or a `)` — `(rivet-sysd)`, but also anything a
/// program set — so splitting on whitespace from the front miscounts. The last
/// `)` ends `comm`; the space-separated fields after it are `state` then `ppid`.
fn parent_pid(pid: i32) -> Option<i32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after = stat.get(stat.rfind(')')? + 1..)?;
    after.split_whitespace().nth(1)?.parse().ok()
}

/// Create `/run/rivet/dhcp`, root-only, if it is not already there.
fn make_directory() -> Result<(), String> {
    // Parents first. `/run` is a tmpfs `rivet_core::mount` established, and
    // `/run/rivet` may or may not exist depending on what has started.
    for parent in ["/run/rivet", LEASE_DIR] {
        let c = std::ffi::CString::new(parent).map_err(|_| format!("{parent} cannot be a path"))?;
        // SAFETY: `c` is a valid NUL-terminated string for the length of the
        // call. `mkdir(2)` rather than `std::fs::create_dir` followed by
        // `set_permissions`, because the gap between those two is a window in
        // which the directory has whatever the umask allowed.
        let rc = unsafe { libc::mkdir(c.as_ptr(), DIR_MODE) };
        if rc != 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(format!("cannot create {parent}: {e}"));
            }
        }
    }
    Ok(())
}

/// Write, then rename.
///
/// **`rivet-netd` may read at any moment**, and half a lease parses as a damaged
/// one — which the parser correctly refuses, turning a working machine into an
/// unaddressed one for no reason but timing. A rename within one filesystem is
/// atomic, so a reader sees either the old lease or the new one.
fn write_atomically(path: &str, text: &str) -> Result<(), String> {
    let temporary = format!("{path}.new");
    {
        let mut file = std::fs::File::create(&temporary)
            .map_err(|e| format!("cannot create {temporary}: {e}"))?;
        file.write_all(text.as_bytes())
            .map_err(|e| format!("cannot write {temporary}: {e}"))?;
        // **Flushed before the rename, not after.** A rename that beats its own
        // data leaves a file that exists and is empty, and an empty lease is
        // refused — so the machine would have no address and a file saying it
        // should have one.
        file.sync_all()
            .map_err(|e| format!("cannot flush {temporary}: {e}"))?;
    }
    std::fs::rename(&temporary, path).map_err(|e| format!("cannot rename onto {path}: {e}"))
}
