//! **Every capability this daemon holds, given up once the controller is
//! bound** — `ADR-041` §6, `ADR-017` §3.
//!
//! The supervisor starts this program under its own uid keeping
//! `CAP_NET_ADMIN`, which binding `HCI_CHANNEL_USER` checks, and `CAP_SETPCAP`,
//! which is what lets a process take a capability out of its own bounding set.
//! Neither is needed once the user channel is bound: reading and writing a
//! bound HCI socket checks nothing. So [`drop_all`] empties every set the
//! kernel keeps, in the one order that works, and then asks the kernel what it
//! holds rather than trusting the return codes.
//!
//! # The order
//!
//! 1. **The bounding set**, one capability at a time, walked upward until the
//!    kernel says the number does not exist. `PR_CAPBSET_DROP` needs
//!    `CAP_SETPCAP` in the effective set, so this is first.
//! 2. **The ambient set**, cleared. Nothing is needed for that.
//! 3. **`capset`** with permitted, effective and inheritable all zero. Lowering
//!    is always allowed, and after it nothing can be raised again.
//! 4. **`PR_SET_NO_NEW_PRIVS`**, which the supervisor already set; set again so
//!    the claim does not rest on how this process was started.
//!
//! Then [`held`] reads each set back. A drop the kernel did not honour is an
//! error, and the daemon refuses to scan rather than scan holding what it said
//! it gave up.
//!
//! # Why the bounding set and not only the effective one
//!
//! With its own uid and `NoNewPrivs` set, a process holding nothing in its
//! permitted set could not raise a capability whatever its bounding set said.
//! Emptying it as well makes the record read the way the process is: every
//! line of `/proc/<pid>/status` that names a capability set reads zero.

/// What the kernel says this process holds, as the masks `/proc/<pid>/status`
/// prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Held {
    pub inheritable: u64,
    pub permitted: u64,
    pub effective: u64,
    pub bounding: u64,
    pub ambient: u64,
    pub no_new_privs: bool,
}

impl Held {
    /// Whether every set is empty and `NoNewPrivs` is set.
    #[must_use]
    pub fn is_nothing(&self) -> bool {
        self.inheritable == 0
            && self.permitted == 0
            && self.effective == 0
            && self.bounding == 0
            && self.ambient == 0
            && self.no_new_privs
    }
}

/// `struct __user_cap_header_struct`, `linux/capability.h`.
#[repr(C)]
struct Header {
    version: u32,
    pid: libc::c_int,
}

/// `struct __user_cap_data_struct`: one 32-bit word of each set. Version 3
/// takes two, low word first.
#[repr(C)]
#[derive(Clone, Copy)]
struct Data {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

/// `_LINUX_CAPABILITY_VERSION_3`, the 64-bit form.
const VERSION_3: u32 = 0x2008_0522;

/// The highest capability number walked. `CAP_LAST_CAP` is 40 on the kernels
/// Rivet runs; the walk stops at the first number the kernel refuses, and this
/// only bounds a kernel that never refuses.
const WALK_LIMIT: libc::c_int = 64;

/// Give up every capability, and read back that it is gone.
///
/// # Errors
///
/// The syscall that refused, or `EPERM` when every call succeeded and the
/// kernel still reports something held — the same reading `cmd/rivet-sysd`
/// takes of its own spawn.
pub fn drop_all() -> std::io::Result<Held> {
    // SAFETY: `prctl` with integer arguments only. `PR_CAPBSET_READ` and
    // `PR_CAPBSET_DROP` read and change this process's own bounding set and
    // take no pointer.
    unsafe {
        let mut cap: libc::c_int = 0;
        while cap < WALK_LIMIT && libc::prctl(libc::PR_CAPBSET_READ, cap, 0, 0, 0) >= 0 {
            if libc::prctl(libc::PR_CAPBSET_DROP, cap, 0, 0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            cap += 1;
        }
    }
    // SAFETY: integer arguments only; clears this process's ambient set.
    let rc = unsafe {
        libc::prctl(
            libc::PR_CAP_AMBIENT,
            libc::PR_CAP_AMBIENT_CLEAR_ALL as libc::c_ulong,
            0,
            0,
            0,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut header = Header {
        version: VERSION_3,
        pid: 0,
    };
    let empty = [Data {
        effective: 0,
        permitted: 0,
        inheritable: 0,
    }; 2];
    // SAFETY: `header` and `empty` are live, correctly laid out structures on
    // this frame for the duration of the call; `capset` reads them and keeps
    // no pointer. Version 3 reads exactly two `Data` words.
    let rc = unsafe { libc::syscall(libc::SYS_capset, &raw mut header, empty.as_ptr()) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: integer arguments only.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let now = held()?;
    if !now.is_nothing() {
        return Err(std::io::Error::from_raw_os_error(libc::EPERM));
    }
    Ok(now)
}

/// What this process holds, in the kernel's words.
///
/// From syscalls rather than `/proc/self/status`, because the ruleset this
/// daemon runs under does not grant `/proc/self`.
///
/// # Errors
///
/// `capget`'s errno.
pub fn held() -> std::io::Result<Held> {
    let mut header = Header {
        version: VERSION_3,
        pid: 0,
    };
    let mut data = [Data {
        effective: 0,
        permitted: 0,
        inheritable: 0,
    }; 2];
    // SAFETY: `header` and `data` are live and exclusively borrowed for the
    // call; version 3 writes exactly two `Data` words into `data`.
    let rc = unsafe { libc::syscall(libc::SYS_capget, &raw mut header, data.as_mut_ptr()) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let word = |f: fn(&Data) -> u32| u64::from(f(&data[0])) | (u64::from(f(&data[1])) << 32);
    let mut bounding = 0u64;
    let mut ambient = 0u64;
    for cap in 0..WALK_LIMIT {
        // SAFETY: integer arguments only; each reads one bit of this
        // process's own state.
        let (in_bounding, in_ambient) = unsafe {
            (
                libc::prctl(libc::PR_CAPBSET_READ, cap, 0, 0, 0),
                libc::prctl(
                    libc::PR_CAP_AMBIENT,
                    libc::PR_CAP_AMBIENT_IS_SET as libc::c_ulong,
                    // `cap` is below `WALK_LIMIT`, so the cast is exact.
                    cap as libc::c_ulong,
                    0,
                    0,
                ),
            )
        };
        if in_bounding < 0 {
            break;
        }
        if in_bounding == 1 {
            bounding |= 1 << cap;
        }
        if in_ambient == 1 {
            ambient |= 1 << cap;
        }
    }
    // SAFETY: integer arguments only.
    let no_new_privs = unsafe { libc::prctl(libc::PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) } == 1;
    Ok(Held {
        inheritable: word(|d| d.inheritable),
        permitted: word(|d| d.permitted),
        effective: word(|d| d.effective),
        bounding,
        ambient,
        no_new_privs,
    })
}

/// A mask as `/proc/<pid>/status` prints it: sixteen hex digits.
#[must_use]
pub fn mask(bits: u64) -> String {
    format!("{bits:016x}")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_mask_reads_the_way_proc_prints_it() {
        assert_eq!(mask(0), "0000000000000000");
        assert_eq!(mask(0x1100), "0000000000001100");
    }

    #[test]
    fn nothing_held_means_every_set_empty_and_no_new_privs_set() {
        let nothing = Held {
            inheritable: 0,
            permitted: 0,
            effective: 0,
            bounding: 0,
            ambient: 0,
            no_new_privs: true,
        };
        assert!(nothing.is_nothing());
        assert!(!Held { no_new_privs: false, ..nothing }.is_nothing());
        assert!(!Held { bounding: 1 << 12, ..nothing }.is_nothing());
        assert!(!Held { ambient: 1 << 12, ..nothing }.is_nothing());
        assert!(!Held { effective: 1 << 8, ..nothing }.is_nothing());
    }

    /// **The drop, run for real, in a child process.** Forked so the test
    /// process keeps what it had; the child reports what the kernel says it
    /// holds afterwards through its exit status. Whatever the suite runs as —
    /// root in the container, or an ordinary user — the drop must leave
    /// nothing: root has `CAP_SETPCAP` for the bounding set, and an ordinary
    /// user's bounding set is the one thing it cannot empty, which this
    /// reports rather than hides.
    #[test]
    fn dropping_everything_leaves_every_set_empty() {
        let before = held().expect("capget");
        let can_empty_bounding = before.effective & (1 << 8) != 0 || before.bounding == 0;
        if !can_empty_bounding {
            eprintln!("this process cannot empty a bounding set, so the drop is not exercised");
            return;
        }
        // SAFETY: `fork` in a test; the child calls only `drop_all`, `held`
        // and `_exit`, none of which touches a lock another thread could hold
        // mid-operation in a way that matters to a process about to exit.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed");
        if pid == 0 {
            let code = match drop_all() {
                Ok(h) if h.is_nothing() => 0,
                Ok(_) => 2,
                Err(_) => 3,
            };
            // SAFETY: ends the child without running the parent's test
            // harness teardown.
            unsafe { libc::_exit(code) };
        }
        let mut status: libc::c_int = 0;
        // SAFETY: waits for the child forked above, writing only `status`.
        let rc = unsafe { libc::waitpid(pid, &raw mut status, 0) };
        assert_eq!(rc, pid);
        assert!(libc::WIFEXITED(status), "the child did not exit: {status}");
        assert_eq!(
            libc::WEXITSTATUS(status),
            0,
            "2 is a drop the kernel did not honour; 3 is a syscall that refused"
        );
    }
}
