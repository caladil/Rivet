//! A [`Kernel`] that answers however a test needs it to.
//!
//! Compiled only under `cfg(test)`. It exists so PID 1's decisions can be
//! driven through states the board reaches and a developer's machine does not:
//! a `/proc` that is not mounted yet, a read-only `/system` (`ADR-018` §3),
//! `signalfd(2)` refused, `EINTR` in the middle of a drain, a `tryboot` flag
//! that `ADR-026` §4 says QEMU cannot produce at all.
//!
//! Every answer is a [`Script`], so a *sequence* of answers can be expressed —
//! which is the point. The interesting boundary is rarely one call's result; it
//! is "`read_mounts` fails, then `mount` succeeds, then `read_mounts`
//! succeeds", which is what every first boot actually does.
//!
//! **The clock advances only when the code under test waits.** `sleep_ms` and a
//! timed-out `await_signal` move it forward, and nothing else does, so backoff
//! and settle times are exact rather than flaky.

use std::collections::{BTreeMap, VecDeque};

use crate::kernel::{Errno, Kernel, MountFlags, Reaped, RebootHow, Sig};

/// A queue of answers ending in one that repeats for ever.
///
/// `Script::sequence(vec![a, b], z)` answers `a`, then `b`, then `z` on every
/// call after. `Script::always(z)` answers `z` from the start.
#[derive(Debug, Clone)]
pub struct Script<T: Clone> {
    queue: VecDeque<T>,
    forever: T,
}

impl<T: Clone> Script<T> {
    pub fn always(forever: T) -> Self {
        Self {
            queue: VecDeque::new(),
            forever,
        }
    }

    pub fn sequence(steps: Vec<T>, forever: T) -> Self {
        Self {
            queue: steps.into(),
            forever,
        }
    }

    pub fn next(&mut self) -> T {
        self.queue
            .pop_front()
            .unwrap_or_else(|| self.forever.clone())
    }
}

/// One recorded `mount(2)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountCall {
    pub source: String,
    pub target: String,
    pub fstype: String,
    pub flags: MountFlags,
    pub data: Option<String>,
}

/// The fake kernel.
///
/// Defaults are the boring happy path — every syscall succeeds, `/proc` reads
/// as empty, there are no children — so a test only has to say what is
/// *unusual* about the world it is describing.
pub struct FakeKernel {
    pub pid: u32,
    /// Everything [`Kernel::say`] was given, in order.
    pub lines: Vec<String>,
    /// Bytes waiting on the console — `ADR-035` §8A.1.2's break-glass input.
    /// Drained by `read_console`, so a test can deliver a line one byte at a
    /// time and watch the reader assemble it.
    pub console_in: Vec<u8>,
    /// Make the console unreadable, for the case that must not stop pid 1.
    pub console_fails: bool,
    /// The monotonic clock, in milliseconds.
    pub clock_ms: u64,

    pub mkdir_calls: Vec<String>,
    pub mount_calls: Vec<MountCall>,
    pub umount_calls: Vec<String>,
    pub write_calls: Vec<(String, Vec<u8>)>,
    /// **Every call that changes something, in the order it happened.**
    ///
    /// Added 2026-08-28 for `crate::identity`, whose central guarantee is an
    /// ORDER rather than a set: the seed must reach /state and be flushed
    /// before the card's copy is destroyed, because the reverse loses a key
    /// nobody kept a second copy of (ADR-025 §3). Separate `*_calls` lists can
    /// say what happened and cannot say when.
    pub order: Vec<String>,
    /// Paths written through `write_secret`, which sets mode 0600 in the open.
    pub secret_writes: Vec<(String, Vec<u8>)>,
    /// Directories created through `mkdir_private`, which sets mode 0700.
    pub private_dirs: Vec<String>,
    pub rename_calls: Vec<(String, String)>,
    pub remove_calls: Vec<String>,
    pub spawn_calls: Vec<String>,
    /// What each `spawn_with` was told, so a test can ask whether a program was
    /// given the right arguments rather than only whether it was started.
    pub spawn_args: Vec<(String, Vec<String>)>,
    /// How many times the console was taken for the splash, and it must be
    /// none on a trial boot.
    pub quiet_console_calls: usize,
    pub kill_calls: Vec<(i32, Sig)>,
    pub reboot_calls: Vec<RebootHow>,
    pub sync_calls: usize,
    pub slept_ms: Vec<u64>,
    pub awaited_timeouts_ms: Vec<u64>,
    pub await_calls: usize,
    pub arm_calls: usize,

    pub mkdir_results: Script<Result<(), Errno>>,
    pub mount_results: Script<Result<(), Errno>>,
    pub umount_results: Script<Result<(), Errno>>,
    /// Every `try_lock` and `unlock`, in order, as `lock <path>` / `unlock
    /// <path>`.
    pub lock_calls: Vec<String>,
    pub lock_results: Script<Result<(), Errno>>,
    pub write_results: Script<Result<(), Errno>>,
    pub rename_results: Script<Result<(), Errno>>,
    pub remove_results: Script<Result<(), Errno>>,
    pub arm_result: Result<(), Errno>,
    pub await_results: Script<Result<Option<Sig>, Errno>>,
    pub wait_results: Script<Result<Reaped, Errno>>,
    pub fork_results: Script<Result<i32, Errno>>,
    pub spawn_results: Script<Result<i32, Errno>>,
    pub kill_results: Script<Result<(), Errno>>,
    pub reboot_results: Script<Result<(), Errno>>,

    /// Per-path text answers. A path with no entry reads `ENOENT`, which is
    /// what an absent file actually does.
    pub files: BTreeMap<String, Script<Result<String, Errno>>>,
    /// Per-path byte answers, for `/proc/device-tree` properties and anything
    /// else that is not text.
    blobs: BTreeMap<String, Script<Result<Vec<u8>, Errno>>>,
    dirs: BTreeMap<String, Script<Result<Vec<String>, Errno>>>,
    /// Per-path `stat` answers, kept apart from `blobs`/`files` rather than
    /// derived from them: `Script::next()` is consuming, and deriving a size
    /// from a `Script::sequence` a test wrote for `read_bytes` would silently
    /// steal one of its answers. A path with no entry reads `ENOENT`, matching
    /// `read_bytes`'s own default for the same reason.
    file_sizes: BTreeMap<String, Script<Result<u64, Errno>>>,
    /// **Every path `read_bytes` was asked for, in order.** Added for `R-2`:
    /// the property a size cap must prove is that an oversized file is
    /// refused *without* this call ever happening, and a Script's returned
    /// value cannot show that on its own.
    pub read_bytes_calls: Vec<String>,
    /// **Whether this machine has a random source.** `false` by default, so
    /// the trait's refusing default is what a test gets unless it says
    /// otherwise; `true` answers distinct bytes from a counter. Self-generated,
    /// and never mistaken for a board's `getrandom(2)`.
    pub random_works: bool,
    random_next: u8,
    /// How many random bytes were drawn, so a test can say none were.
    pub random_drawn: usize,
}

impl FakeKernel {
    pub fn new() -> Self {
        Self {
            pid: 1,
            lines: Vec::new(),
            console_in: Vec::new(),
            console_fails: false,
            clock_ms: 0,
            mkdir_calls: Vec::new(),
            mount_calls: Vec::new(),
            umount_calls: Vec::new(),
            write_calls: Vec::new(),
            order: Vec::new(),
            secret_writes: Vec::new(),
            private_dirs: Vec::new(),
            rename_calls: Vec::new(),
            remove_calls: Vec::new(),
            spawn_calls: Vec::new(),
            spawn_args: Vec::new(),
            quiet_console_calls: 0,
            kill_calls: Vec::new(),
            reboot_calls: Vec::new(),
            sync_calls: 0,
            slept_ms: Vec::new(),
            awaited_timeouts_ms: Vec::new(),
            await_calls: 0,
            arm_calls: 0,
            mkdir_results: Script::always(Ok(())),
            mount_results: Script::always(Ok(())),
            umount_results: Script::always(Ok(())),
            lock_calls: Vec::new(),
            lock_results: Script::always(Ok(())),
            write_results: Script::always(Ok(())),
            rename_results: Script::always(Ok(())),
            remove_results: Script::always(Ok(())),
            arm_result: Ok(()),
            await_results: Script::always(Ok(None)),
            wait_results: Script::always(Err(Errno::ECHILD)),
            // Two pids by default, so a self-test can be spawned without the
            // test having to say so.
            fork_results: Script::sequence(vec![Ok(53), Ok(54)], Ok(60)),
            spawn_results: Script::always(Ok(200)),
            kill_results: Script::always(Ok(())),
            reboot_results: Script::always(Ok(())),
            files: {
                // **The reachability marker is present by default** —
                // PLAN.md §104.20. Every test written before it exists is
                // about something else, and a fake that answered ENOENT here
                // would make all of them assert WaitingToBeReached instead of
                // what they were written for. A test ABOUT reachability
                // removes it, which is the direction that reads correctly:
                // the interesting case is the machine nobody reached.
                let mut files = BTreeMap::new();
                files.insert(
                    rivet_core::operation::status::REACHED_MARKER_PATH.to_string(),
                    Script::always(Ok(String::new())),
                );
                files
            },
            blobs: BTreeMap::new(),
            dirs: BTreeMap::new(),
            file_sizes: BTreeMap::new(),
            read_bytes_calls: Vec::new(),
            random_works: false,
            random_next: 1,
            random_drawn: 0,
        }
    }

    pub fn files_mut(&mut self) -> &mut BTreeMap<String, Script<Result<String, Errno>>> {
        &mut self.files
    }

    pub fn blobs_mut(&mut self) -> &mut BTreeMap<String, Script<Result<Vec<u8>, Errno>>> {
        &mut self.blobs
    }

    pub fn dirs_mut(&mut self) -> &mut BTreeMap<String, Script<Result<Vec<String>, Errno>>> {
        &mut self.dirs
    }

    pub fn file_sizes_mut(&mut self) -> &mut BTreeMap<String, Script<Result<u64, Errno>>> {
        &mut self.file_sizes
    }
}

impl Kernel for FakeKernel {
    fn self_pid(&self) -> u32 {
        self.pid
    }

    fn random_bytes(&mut self, len: usize) -> Result<Vec<u8>, Errno> {
        if !self.random_works {
            return Err(Errno(libc::ENOSYS));
        }
        self.random_drawn += len;
        Ok((0..len)
            .map(|_| {
                self.random_next = self.random_next.wrapping_add(37);
                self.random_next
            })
            .collect())
    }

    fn read_console(&mut self, into: &mut [u8]) -> Result<usize, Errno> {
        if self.console_fails {
            return Err(Errno::EIO);
        }
        let n = self.console_in.len().min(into.len());
        into[..n].copy_from_slice(&self.console_in[..n]);
        self.console_in.drain(..n);
        Ok(n)
    }

    fn say(&mut self, line: &str) {
        self.lines.push(line.to_string());
    }

    fn now_ms(&mut self) -> u64 {
        self.clock_ms
    }

    fn mkdir_p(&mut self, path: &str) -> Result<(), Errno> {
        self.mkdir_calls.push(path.to_string());
        self.mkdir_results.next()
    }

    fn mount(
        &mut self,
        source: &str,
        target: &str,
        fstype: &str,
        flags: MountFlags,
        data: Option<&str>,
    ) -> Result<(), Errno> {
        self.mount_calls.push(MountCall {
            source: source.to_string(),
            target: target.to_string(),
            fstype: fstype.to_string(),
            flags,
            data: data.map(str::to_string),
        });
        self.mount_results.next()
    }

    fn umount(&mut self, target: &str) -> Result<(), Errno> {
        self.umount_calls.push(target.to_string());
        self.umount_results.next()
    }

    /// Takes no time, whatever it answers: the real one never waits.
    fn try_lock(&mut self, path: &str) -> Result<(), Errno> {
        self.lock_calls.push(format!("lock {path}"));
        self.lock_results.next()
    }

    fn unlock(&mut self, path: &str) {
        self.lock_calls.push(format!("unlock {path}"));
    }

    fn read_file(&mut self, path: &str) -> Result<String, Errno> {
        match self.files.get_mut(path) {
            Some(script) => script.next(),
            None => Err(Errno::ENOENT),
        }
    }

    fn read_bytes(&mut self, path: &str) -> Result<Vec<u8>, Errno> {
        self.read_bytes_calls.push(path.to_string());
        match self.blobs.get_mut(path) {
            Some(script) => script.next(),
            // Falling back to the text map keeps a test from having to say the
            // same thing twice when a file is read both ways.
            None => self.read_file(path).map(String::into_bytes),
        }
    }

    fn file_size(&mut self, path: &str) -> Result<u64, Errno> {
        match self.file_sizes.get_mut(path) {
            Some(script) => script.next(),
            None => Err(Errno::ENOENT),
        }
    }

    fn write_file(&mut self, path: &str, contents: &[u8]) -> Result<(), Errno> {
        self.order.push(format!("write_file {path}"));
        self.write_calls.push((path.to_string(), contents.to_vec()));
        self.write_results.next()
    }

    fn mkdir_private(&mut self, path: &str) -> Result<(), Errno> {
        self.private_dirs.push(path.to_string());
        self.mkdir_results.next()
    }

    fn overwrite_in_place(&mut self, path: &str, contents: &[u8]) -> Result<(), Errno> {
        self.order.push(format!("overwrite_in_place {path}"));
        self.write_calls.push((path.to_string(), contents.to_vec()));
        self.write_results.next()
    }

    /// **Recorded separately from `write_file`**, so a test can assert that a
    /// seed went through the writer that sets `0600` and not the one that does
    /// not. Two lists rather than one flag, because the question a test asks is
    /// *which paths were written privately* and a flag answers a different one.
    fn write_secret(&mut self, path: &str, contents: &[u8]) -> Result<(), Errno> {
        self.order.push(format!("write_secret {path}"));
        self.secret_writes
            .push((path.to_string(), contents.to_vec()));
        self.write_results.next()
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), Errno> {
        self.rename_calls.push((from.to_string(), to.to_string()));
        self.rename_results.next()
    }

    fn remove_file(&mut self, path: &str) -> Result<(), Errno> {
        self.order.push(format!("remove_file {path}"));
        self.remove_calls.push(path.to_string());
        self.remove_results.next()
    }

    fn read_dir(&mut self, path: &str) -> Result<Vec<String>, Errno> {
        match self.dirs.get_mut(path) {
            Some(script) => script.next(),
            None => Err(Errno::ENOENT),
        }
    }

    fn arm_signals(&mut self) -> Result<(), Errno> {
        self.arm_calls += 1;
        self.arm_result
    }

    fn await_signal(&mut self, timeout_ms: u64) -> Result<Option<Sig>, Errno> {
        self.await_calls += 1;
        self.awaited_timeouts_ms.push(timeout_ms);
        let answer = self.await_results.next();
        // A wait that delivered nothing waited the whole timeout; one that
        // delivered a signal was cut short. Modelling that is what makes
        // backoff and settle times exact in tests rather than approximate.
        self.clock_ms = self.clock_ms.saturating_add(match answer {
            Ok(None) => timeout_ms,
            _ => 1,
        });
        answer
    }

    fn sleep_ms(&mut self, ms: u64) {
        self.slept_ms.push(ms);
        self.clock_ms = self.clock_ms.saturating_add(ms);
    }

    fn waitpid_any(&mut self) -> Result<Reaped, Errno> {
        self.wait_results.next()
    }

    fn quiet_console(&mut self) -> crate::kernel::ConsoleQuiet {
        self.quiet_console_calls += 1;
        // Recorded in `order` as well, because WHEN it happened is the property
        // that matters: a console taken after the splash was spawned is a race
        // the picture loses.
        self.order.push("quiet_console".to_string());
        crate::kernel::ConsoleQuiet {
            fbcon_unbound: Some("vtcon1".to_string()),
            printk_lowered: true,
            output_discarded: true,
        }
    }

    fn spawn_with(&mut self, path: &str, args: &[String]) -> Result<i32, Errno> {
        self.spawn_args.push((path.to_string(), args.to_vec()));
        self.spawn(path)
    }

    fn spawn(&mut self, path: &str) -> Result<i32, Errno> {
        self.spawn_calls.push(path.to_string());
        // In `order` too, because starting a process is a change to the machine
        // and one caller cares WHEN it happened relative to another call: the
        // splash must be started after the console is taken, never before.
        self.order.push(format!("spawn {path}"));
        self.spawn_results.next()
    }

    fn kill(&mut self, pid: i32, signal: Sig) -> Result<(), Errno> {
        self.kill_calls.push((pid, signal));
        self.kill_results.next()
    }

    fn sync(&mut self) {
        self.order.push("sync".to_string());
        self.sync_calls += 1;
    }

    fn reboot(&mut self, how: RebootHow) -> Result<(), Errno> {
        self.reboot_calls.push(how);
        self.reboot_results.next()
    }

    fn fork_child_exiting_with(&mut self, _code: i32) -> Result<i32, Errno> {
        self.fork_results.next()
    }

    fn fork_orphan_maker(&mut self, _delay_ms: u64, _code: i32) -> Result<i32, Errno> {
        self.fork_results.next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_repeats_its_last_answer_for_ever() {
        let mut s = Script::sequence(vec![1, 2], 9);
        assert_eq!([s.next(), s.next(), s.next(), s.next()], [1, 2, 9, 9]);
    }

    #[test]
    fn an_always_script_never_changes_its_answer() {
        let mut s = Script::always(7);
        assert_eq!([s.next(), s.next()], [7, 7]);
    }

    /// The fake must model an absent file as `ENOENT` rather than as a panic or
    /// an empty string, because "absent" is the state under test in half of
    /// these suites.
    #[test]
    fn an_unconfigured_path_reads_as_absent() {
        let mut k = FakeKernel::new();
        assert_eq!(k.read_file("/proc/anything"), Err(Errno::ENOENT));
        assert_eq!(k.read_bytes("/proc/anything"), Err(Errno::ENOENT));
        assert_eq!(k.read_dir("/proc"), Err(Errno::ENOENT));
    }

    #[test]
    fn read_mounts_goes_through_read_file_so_the_default_method_is_covered() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            crate::kernel::MOUNTS_PATH.to_string(),
            Script::always(Ok("proc /proc proc rw 0 0\n".to_string())),
        );
        assert_eq!(k.read_mounts(), Ok("proc /proc proc rw 0 0\n".to_string()));
    }

    #[test]
    fn a_text_file_can_also_be_read_as_bytes() {
        let mut k = FakeKernel::new();
        k.files_mut()
            .insert("/f".to_string(), Script::always(Ok("hi".to_string())));
        assert_eq!(k.read_bytes("/f"), Ok(b"hi".to_vec()));
    }

    #[test]
    fn the_fake_defaults_to_being_pid_one() {
        assert_eq!(FakeKernel::new().self_pid(), 1);
    }

    /// The clock is what makes backoff testable, so its rules are themselves
    /// tested: it moves when the code waits and at no other time.
    #[test]
    fn the_clock_advances_only_when_the_code_under_test_waits() {
        let mut k = FakeKernel::new();
        assert_eq!(k.now_ms(), 0);
        let _ = k.spawn("/x");
        let _ = k.waitpid_any();
        assert_eq!(k.now_ms(), 0, "syscalls that do not wait take no time");
        k.sleep_ms(250);
        assert_eq!(k.now_ms(), 250);
    }

    #[test]
    fn a_timed_out_wait_consumes_its_whole_timeout() {
        let mut k = FakeKernel::new();
        let got = k.await_signal(5_000);
        assert_eq!(got, Ok(None));
        assert_eq!(k.now_ms(), 5_000);
    }

    #[test]
    fn a_wait_that_delivers_a_signal_is_cut_short() {
        let mut k = FakeKernel::new();
        k.await_results = Script::always(Ok(Some(Sig::Term)));
        let got = k.await_signal(5_000);
        assert_eq!(got, Ok(Some(Sig::Term)));
        assert_eq!(k.now_ms(), 1, "a delivered signal did not wait the timeout");
    }
}
