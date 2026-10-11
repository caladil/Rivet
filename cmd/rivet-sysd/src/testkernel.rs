//! A [`Kernel`] that answers however a test needs it to.
//!
//! Compiled only under `cfg(test)`. It exists so the supervisor's decisions can
//! be driven through states a developer's machine does not reach on demand: a
//! service that will not die, a cgroup hierarchy that is not mounted, a
//! `signalfd(2)` that was refused, and — the reason this fake matters more than
//! PID 1's — **a clock that moves when the test says so.** A thirty-second
//! backoff and a ten-second stop grace are tested here in microseconds.
//!
//! Every answer that can vary over a call sequence is a [`Script`], because the
//! interesting boundary is rarely one call's result: it is "the cgroup holds
//! processes, then it is empty", which is what a supervisor restart followed by
//! the service exiting actually looks like.

use std::collections::{BTreeMap, VecDeque};

use crate::journal::Record;
use crate::kernel::{
    CecTransaction, CecWorker, Completed, Errno, Event, ExitStatus, Hierarchy, Kernel, MountFlags,
    Observation, Program, Sig, Stream,
};

/// One `mount(2)` the fake was asked to perform, kept whole.
///
/// Whole rather than reduced to a boolean, because the projection's interesting
/// assertions are about *which* call it was: the first mount carries the tmpfs
/// options, the last carries `MS_REMOUNT|MS_RDONLY`, and a projection that
/// skipped the last one would look identical to one that did not if only the
/// count were recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountCall {
    pub source: String,
    pub target: String,
    pub fstype: String,
    pub flags: MountFlags,
    pub data: Option<String>,
}

/// One file the fake was asked to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteCall {
    pub path: String,
    pub contents: Vec<u8>,
    pub mode: u32,
}

/// A queue of answers ending in one that repeats for ever.
///
/// The same shape as `cmd/rivet-initd`'s. See the note at the top of
/// `kernel.rs`: this is the third type the two binaries want to share, and it
/// is evidence for `ADR-013` §7 rather than something to fix by having one
/// binary depend on the other.
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

/// The pid the fake hands out first when a test has not said otherwise.
pub const FIRST_FAKE_PID: i32 = 1_000;

/// The fake operating system.
///
/// Defaults describe **the machine Rivet actually is today**, not a
/// convenient one.
///
/// ~~no cgroup hierarchy is mounted, so [`Kernel::observe`] answers
/// `Unavailable`.~~ **Corrected 2026-08-23.** That was true until
/// `rivet_core::mount::stage0_plan` grew cgroup2 on 2026-08-22, and it stayed
/// written here afterwards — which is `AGENTS.md` §7 in miniature, an absence
/// claim that outlived the thing it described. A real Rivet *does* mount
/// cgroup2, and a Raspberry Pi 4 was observed doing so (`ADR-024` §8.1).
///
/// So the defaults are now **the first boot that found this defect**:
/// [`Kernel::cgroup_hierarchy`] answers [`Hierarchy::Present`] and
/// [`Kernel::observe`] answers `Unavailable(ENOENT)`, which is a healthy
/// machine on which this service has simply never run. A test that wants the
/// blind machine — no cgroup2 at all — sets [`FakeKernel::hierarchy_results`],
/// and a test that wants the adoption path scripts an observation.
pub struct FakeKernel {
    /// The monotonic clock, in milliseconds. Assign to it to move time.
    pub mono_ms: u64,
    /// The wall clock, or `None` for a machine with no readable one.
    ///
    /// A successful [`Kernel::set_wall_ms`] moves it, which is what a real
    /// kernel does and what `crate::clock`'s tests depend on: the whole claim
    /// being made is that records written *after* the restore carry the restored
    /// time.
    pub wall_ms: Option<u64>,
    /// Every wall clock this was asked to set, in order.
    pub wall_sets: Vec<u64>,
    pub set_wall_results: Script<Result<(), Errno>>,

    /// What this machine ended up called, or `None` if nothing set a name.
    ///
    /// `None` rather than `(none)`: the fake models *nothing was ever set*, and
    /// a test asserting the kernel's literal default would be asserting Linux's
    /// behaviour rather than this crate's.
    pub hostname: Option<String>,
    /// What `set_hostname` answers. `Ok(())` unless a test wants the refusal.
    pub set_hostname_result: Result<(), Errno>,

    /// Every record that reached the sink, in order.
    pub records: Vec<Record>,
    /// Every `(service, program)` that was spawned, in order.
    pub spawned: Vec<(String, Program)>,
    /// Every `(pid, signal)` that was sent, in order.
    pub signals: Vec<(i32, Sig)>,
    /// The path `open_query` was given, if it was called — `ADR-032` §3.2.
    pub query_socket: Option<String>,

    /// What [`Kernel::landlock_abi`] answers — `ADR-042` §5.
    ///
    /// **Defaults to a kernel that HAS Landlock**, because that is the machine
    /// Rivet ships: `tools/buildroot/linux/rivet.config` sets
    /// `CONFIG_SECURITY_LANDLOCK=y` and puts `landlock` in `CONFIG_LSM`, and
    /// the board's own log says `landlock: Up and running.` A fake defaulting
    /// to zero would make every application in every test refuse to start, and
    /// the suite would then be measuring the refusal rather than the feature.
    ///
    /// Set it to `(0, ENOSYS)` for the other direction — see
    /// `crate::supervise`'s test that an application is NOT started on a kernel
    /// without it.
    pub landlock: (u32, i32),

    /// Where `rivet_core::kiosk`'s socket was bound and who it was given to.
    pub kiosk_socket: Option<(String, Option<rivet_core::service::AppIdentity>)>,

    /// Every answer written to the kiosk socket, in order.
    pub kiosk_answered: Vec<String>,

    /// Where `rivet_core::facts`'s socket was bound — `ADR-057`.
    ///
    /// **No owner beside it**, unlike `kiosk_socket`, and the absence is the
    /// property: that socket is chowned to one consumer and this one is
    /// `0666`, so a test that asserted an owner here would be asserting the
    /// wrong design.
    pub facts_socket: Option<String>,

    /// Every answer written to the facts socket, in order.
    pub facts_answered: Vec<String>,
    /// Everything `answer_query` was handed, in order.
    pub answered: Vec<String>,
    /// What `open_query` should answer.
    pub query_open: Script<Result<(), Errno>>,
    /// Every `(service, pid)` placed in a cgroup.
    pub placed: Vec<(String, i32)>,
    /// Every service [`Kernel::cgroup_kill`] was asked to kill, in order and
    /// with repeats — `S-1`'s escalation can ask more than once.
    pub cgroup_kills: Vec<String>,
    /// What [`Kernel::cgroup_kill`] answers. `Ok(())` by default — the
    /// machine Rivet ships has `cgroup.kill` — and a test that wants the
    /// fallback (a kernel older than 5.14) sets `Err`.
    pub cgroup_kill_results: Script<Result<(), Errno>>,
    /// Every `(service, bytes)` capped — the operator's half of the 2026-08-24
    /// split, as the kernel saw it.
    ///
    /// **Successful caps only.** A refused one enforced nothing, and a test
    /// that could not tell the two apart is exactly the test that would pass
    /// while a board ran uncapped (`ADR-024` §8.1 makes that a real machine
    /// rather than a hypothetical one).
    pub capped: Vec<(String, u64)>,
    /// What [`Kernel::apply_memory_limit`] should answer.
    pub memory_results: Script<Result<(), Errno>>,
    /// Every `(service, quota_us)` capped — [`Self::capped`]'s footing, for
    /// [`Kernel::apply_cpu_limit`]. **Successful caps only**, for
    /// [`Self::capped`]'s reason.
    pub capped_cpu: Vec<(String, u64)>,
    /// What [`Kernel::apply_cpu_limit`] should answer.
    pub cpu_results: Script<Result<(), Errno>>,
    /// Binary files, for [`Kernel::read_file_bytes`] — a signed header, a
    /// trust store, a program to be hashed. Separate from the text files
    /// because those are documents and these are not, and because a test that
    /// puts a 4 KiB header in a `String` map is a test nobody can read.
    pub blobs: BTreeMap<String, Vec<u8>>,
    /// What [`Kernel::local_time`] should answer -- `ADR-021` §9.
    ///
    /// `None` by default: see the method. A test that wants a reading sets one
    /// and is visible doing it, because the value is a CLAIM about what a C
    /// library would say and no test off a board can make that claim.
    pub local_reading: Option<crate::kernel::LocalReading>,
    /// What [`Kernel::grow_state`] should answer.
    ///
    /// **The default is the answer a machine with no card gives**, which is
    /// every guest under QEMU (`ADR-026` §4) and every host running these
    /// tests. A default of "it grew" would make the ordinary case the exotic
    /// one and would let a test assert a grow that no medium performed.
    pub grew: rivet_adapter_image::Grew,
    /// How many times [`Kernel::grow_state`] was called.
    ///
    /// **Once per boot is the contract**, not "at least once": `ADR-018` §10.3
    /// writes a partition table, and a second call in one boot would mean the
    /// call site had grown a retry that nothing rules.
    pub grew_called: usize,
    /// Services with an open notification socket, in name order.
    pub notify_open: Vec<String>,
    /// Who each notification socket was chowned to — `None` for a root
    /// service. `ADR-017` §3.4 plus `ADR-015` §5.1: an application that cannot
    /// WRITE its own socket can never declare readiness, which is what a board
    /// answered before 2026-08-25.
    pub notify_owners: BTreeMap<String, Option<(u32, u32)>>,
    /// Service to (LISTEN_FDNAMES tokens, the owning uid) — `ADR-015` §5.2.
    pub listeners_open: BTreeMap<String, (Vec<String>, Option<u32>)>,
    /// Service to the `address:port` of each tcp and udp socket it was last
    /// opened with, in the manifest's order — `ADR-046` §7, `.rulings/` R-188.
    /// Kept after a close, so a test can read where a stopped service listened.
    pub listeners_bound: BTreeMap<String, Vec<String>>,
    /// Every service `close_listeners` was called for, in order and with
    /// repeats: it is idempotent, so a test asserting it was called exactly
    /// once would be asserting something the contract does not promise.
    pub listeners_closed: Vec<String>,
    /// Every service `close_display` was called for, in order.
    ///
    /// **The trait defaults every display method to a no-op** so a test double
    /// is not obliged to model a socketpair it never reads, and the cost of
    /// that was found by the audit of 2026-09-03: `Kernel::close_display` had
    /// no caller anywhere in the tree, and nothing could have noticed, because
    /// nothing recorded whether it was called. This is the recording.
    pub displays_closed: Vec<String>,
    /// When set, every `open_listeners` refuses with it — the "port already in
    /// use" case, which is the one an operator actually meets.
    pub listener_error: Option<Errno>,
    /// Which services hold an answering channel — `ADR-045` §3, keyed by
    /// name with the `LISTEN_FDNAMES` token it was handed under.
    pub health_open: BTreeMap<String, String>,
    /// When set, every `open_health` refuses with it.
    pub health_error: Option<Errno>,
    /// Every service `ask_health` was called for, in order and with repeats —
    /// so a test can assert the CADENCE, which is the half of `ADR-045` §3
    /// that the supervisor now owns.
    pub health_asked: Vec<String>,
    /// When set, every `ask_health` refuses with it — the socket that is not
    /// there, which the supervisor reports as *it did not answer* rather than
    /// retrying.
    pub ask_health_error: Option<Errno>,
    /// Every timeout `wait` was called with.
    pub waits: Vec<Option<u64>>,
    pub arm_calls: usize,

    /// Every `mkdir -p` asked for, in order.
    pub mkdir_calls: Vec<String>,
    /// Every `(path, mode)` [`Kernel::mkdir_mode`] was asked for, in order.
    ///
    /// The mode is kept because it is half the point: a caller that created
    /// the right paths with the wrong modes would look identical here if only
    /// the paths were recorded, and a mode is what decides whether a program
    /// that reads the directory later refuses it.
    pub mkdir_mode_calls: Vec<(String, u32)>,
    /// Every program [`Kernel::run_to_completion`] was asked to run, in order.
    pub ran: Vec<Program>,
    /// Paths that `list_dir` must NOT report, because they are not
    /// directories — `ADR-015` §7's `current` is the one that matters. The
    /// real kernel skips them and this fake did not, which is how the first
    /// uninstall on a board left a dangling link (2026-08-25).
    pub not_a_directory: std::collections::BTreeSet<String>,
    /// Paths [`Kernel::is_symlink`] answers `Ok(true)` for. A path not in here
    /// answers `Ok(false)` — not a link, whether or not it exists — which is
    /// every path a test has not deliberately planted one at.
    pub symlinks: std::collections::BTreeSet<String>,
    /// Every `Kernel::reclaim_in_child` asked for, as `(service, dir,
    /// prefix)`, in order. Nothing is forked: the matching directories are
    /// removed from the fake at once, which is what the helper would leave.
    pub reclaims: Vec<(String, String, String)>,
    /// Every [`Kernel::issue_in_child`] asked for, in order. Nothing is forked
    /// and nothing is signed: a successful answer puts placeholder text at
    /// every file the order names, so what reads those files next finds them.
    pub issues: Vec<crate::kernel::IssueOrder>,
    /// What [`Kernel::issue_in_child`] answers.
    pub issue_results: Script<Result<Result<String, String>, Errno>>,
    /// Every value [`Kernel::set_own_oom_score_adj`] was asked for, in order.
    pub own_oom_score_adj: Vec<i32>,
    /// What it answers.
    pub own_oom_score_adj_result: Result<(), Errno>,
    /// Every policy [`Kernel::apply_firewall`] was asked to apply, in order.
    pub firewalls: Vec<rivet_adapter_net::firewall::Policy>,
    /// What it answers. `None`: an apply the kernel accepted, read back as
    /// exactly the policy asked for.
    pub firewall_result: Option<Result<rivet_adapter_net::firewall::Applied, String>>,
    /// The deadline every `Kernel::run_within` was given, in call order.
    pub run_limits: Vec<u64>,
    /// Every path [`Kernel::read_file`] was asked for, in order.
    ///
    /// Recorded so that a test can assert a component **did not read** a file,
    /// which is how a component proves it cannot leak the contents of a file
    /// rather than merely observing that it did not on one run.
    pub reads: Vec<String>,
    /// Every `mount(2)` asked for, in order.
    pub mount_calls: Vec<MountCall>,
    /// Every file written, in order. **Successful writes only** — a refused
    /// write left nothing on the card and must not look like one that did.
    pub writes: Vec<WriteCall>,
    /// How many times a write was *attempted*, refused or not.
    ///
    /// The other half of [`FakeKernel::writes`], and it exists for one
    /// assertion: `crate::clock` must not retry a failed save on every wakeup,
    /// and a test counting only successes could not tell "backed off" from
    /// "tried a hundred times and failed a hundred times".
    pub attempted_writes: usize,
    /// Every path [`Kernel::sync_file`] was asked to flush, in order.
    pub syncs: Vec<String>,
    /// Every `(from, to)` [`Kernel::rename`] was asked for, in order.
    pub renames: Vec<(String, String)>,
    /// Every directory [`Kernel::sync_dir`] was asked to flush, in order.
    pub dir_syncs: Vec<String>,
    /// Every path [`Kernel::remove_file`] was asked to remove, in order.
    pub removed: Vec<String>,
    /// Every `(path, bytes)` [`Kernel::append_file`] was asked for, in order.
    /// **Successful appends only**, on [`FakeKernel::writes`]'s reasoning: a
    /// refused append put nothing on the card and must not look like one that
    /// did.
    pub appends: Vec<(String, Vec<u8>)>,
    pub append_results: Script<Result<(), Errno>>,
    /// Every path [`Kernel::write_file`] replaced the contents of.
    ///
    /// Recorded separately from [`FakeKernel::writes`] for one assertion: the
    /// journal must **append** to the previous boot's records rather than
    /// replace them, and a test that only checked the bytes could not tell a
    /// journal that appended from one that truncated and wrote the same bytes.
    pub truncations: Vec<String>,
    /// Paths written through [`Kernel::write_file_nofollow`] rather than
    /// [`Kernel::write_file`] — the guarded write, for a path below a directory
    /// an unprivileged uid owns. A test asserts a path is in here; the guards
    /// themselves need a real filesystem and are tested against one.
    pub nofollow_writes: Vec<String>,
    /// Sizes [`Kernel::file_size`] reports. A path that is not here answers
    /// `ENOENT`, which is "nothing has been written yet".
    sizes: BTreeMap<String, u64>,
    /// Modification times [`Kernel::modified_secs`] reports, in seconds. A path
    /// that is not here answers `None` — which retention reads as "this
    /// directory's age is unknown", not as "it is not there".
    modified: BTreeMap<String, u64>,
    /// Paths whose size cannot be read at all.
    ///
    /// Distinct from having no entry in [`FakeKernel::sizes`], and the
    /// distinction is `AGENTS.md` §7's: absent and unreadable are different
    /// facts, a fake that could only express the first makes the second's branch
    /// untestable, and the second is the one where a component has to decide
    /// what to do about a card it cannot ask.
    size_errors: BTreeMap<String, Errno>,
    /// Which paths currently exist, as far as the file operations are
    /// concerned. Maintained by append, write, rename and remove so that
    /// `live_files` can answer the question rotation is actually about: how
    /// many files are on the card.
    live: std::collections::BTreeSet<String>,
    pub sync_file_results: Script<Result<(), Errno>>,
    pub rename_results: Script<Result<(), Errno>>,
    pub sync_dir_results: Script<Result<(), Errno>>,
    pub remove_file_results: Script<Result<(), Errno>>,
    /// When true, a successful [`Kernel::write_file`] also makes the file
    /// readable back through [`Kernel::read_file`] — which is what a real
    /// filesystem does, and what the projection's read-back evidence needs in
    /// order to be tested at all.
    pub writes_are_readable: bool,

    /// Children waiting to be reaped. Push to it, then call `reap`.
    pub exits: Vec<(i32, ExitStatus)>,
    /// When true, a `SIGTERM` queues the child's exit, which is what a
    /// well-behaved service does. Off by default so that the tests which drive
    /// exits by hand see exactly what they queued and nothing else.
    pub obey_sigterm: bool,

    /// `Ok(0)` means "allocate the next fake pid", which is what a test that
    /// does not care about pid values wants.
    pub spawn_results: Script<Result<i32, Errno>>,
    pub signal_results: Script<Result<(), Errno>>,
    pub place_results: Script<Result<(), Errno>>,
    pub notify_results: Script<Result<String, Errno>>,
    pub observations: Script<Observation>,
    /// What [`Kernel::cgroup_hierarchy`] answers.
    ///
    /// A [`Script`] rather than a single value, because "the hierarchy was
    /// there and then was not" is a sequence a supervisor has to survive, and
    /// because a scripted sequence is how a test proves the answer is **re-read
    /// rather than remembered**.
    pub hierarchy_results: Script<Hierarchy>,
    /// How many times [`Kernel::cgroup_hierarchy`] was asked.
    ///
    /// Counted so a test can pin that the supervisor asks the kernel again on
    /// each pass instead of caching a verdict (`AGENTS.md` §5), and — the other
    /// direction — that it does **not** ask on the paths where the answer could
    /// not change anything.
    pub hierarchy_calls: usize,
    pub wait_results: Script<Result<Vec<Event>, Errno>>,
    pub arm_result: Result<(), Errno>,
    /// How many times [`Kernel::open_kmsg`] was asked. Counted so a test can
    /// pin that it is asked ONCE — reopening /dev/kmsg would restart at the
    /// oldest surviving record and journal the whole boot again.
    pub kmsg_calls: usize,
    /// **`Err(ENOENT)` by default, which is what a machine with no devtmpfs
    /// answers** — `tools/qemu/run.sh`'s initramfs, and therefore the state
    /// every emulator run is in. A fake that opened it by default would make
    /// the refusal path the untested one.
    pub kmsg_result: Result<(), Errno>,
    /// Every path [`Kernel::open_journal_ingest`] was asked to bind, in order.
    /// A list rather than a count, because binding it twice would leave a
    /// listener nothing is polling — and because the *path* is the fact three
    /// binaries have to agree about (`ADR-022` §11).
    pub ingest_bound: Vec<String>,
    /// **`Ok(())` by default**, unlike `kmsg_result`. `/run` is a tmpfs PID 1
    /// mounts (`rivet_core::mount`) and exists on every machine this runs on,
    /// including the emulator — so the ordinary state is a socket that binds,
    /// and a fake that refused by default would make the working path the
    /// untested one.
    pub ingest_result: Result<(), Errno>,
    pub mkdir_results: Script<Result<(), Errno>>,
    /// `Ok(true)` is "it was not there and now is", which is what a first boot
    /// does with every directory.
    pub mkdir_mode_results: Script<Result<bool, Errno>>,
    pub mount_results: Script<Result<(), Errno>>,
    pub write_results: Script<Result<(), Errno>>,
    pub run_results: Script<Result<ExitStatus, Errno>>,
    /// What a generator writes on its streams — `ADR-022` §4, applied to a
    /// program that runs to completion. Empty by default, which is a silent
    /// generator and is what most tests here are about.
    pub run_says: Vec<(Stream, String)>,
    /// Paths a **successful** [`Kernel::run_to_completion`] makes exist, as a
    /// real generator's output does.
    ///
    /// Empty by default, and that default is the honest one rather than the
    /// convenient one: a program that exits 0 and writes nothing is a state
    /// a caller has to be able to catch, so it has to be the state a fake
    /// that was told nothing is in. A test whose subject is the *working*
    /// generator says which file it writes.
    pub run_creates: Vec<String>,

    /// Per-path file answers. A path with no entry reads `ENOENT`, which is
    /// what an absent file actually does.
    files: BTreeMap<String, Script<Result<String, Errno>>>,
    /// Per-path answers to [`Kernel::file_exists`]. A path with no entry is
    /// `Ok(false)` — not there, definitively.
    exists: BTreeMap<String, Script<Result<bool, Errno>>>,
    /// Every path [`Kernel::file_exists`] was actually asked about, in order.
    /// `crate::material`'s tests use it to prove a service that declares no
    /// material is never asked about any path at all, rather than merely
    /// answered `Ok(false)` for one it happened not to name.
    pub exists_asked: Vec<String>,
    /// What is in each directory. A path with no entry is `ENOENT`.
    dirs: BTreeMap<String, Vec<String>>,
    /// Directories that answer with an error instead of a listing — the branch
    /// where a card is failing rather than empty.
    dir_errors: BTreeMap<String, Errno>,
    /// Directories [`Kernel::mkdir_mode`] has actually created, so that a
    /// second pass over the same plan reports creating nothing.
    made_dirs: std::collections::BTreeSet<String>,
    /// Every [`Kernel::chown`], in order: path, uid, gid. Added 2026-08-24 with
    /// `ADR-015` §5.1's identity, and public because *whether the application's
    /// own directory was given to the application* is the question the tests
    /// around it ask.
    pub chowns: Vec<(String, u32, u32)>,
    /// Directories as `list_dir_entries` sees them — every entry, whatever
    /// type. `/dev/input` holds character devices and no directory at all, so
    /// this cannot be `dirs` with a different reader.
    pub dir_entries: BTreeMap<String, Vec<String>>,
    /// Every `(path, mode)` `chmod` was given, in order.
    pub chmods: Vec<(String, u32)>,
    /// Every `(path, gid)` `share_tree` was given, in order.
    ///
    /// **The root of each walk, not every path it would touch.** The fake does
    /// not model a filesystem — `crate::repos`' `on_disk` helper says why — so
    /// what a test can assert here is that Rivet asked for the right tree to be
    /// shared with the right group. That the walk reaches every directory
    /// underneath is the real kernel's claim, and `share_tree_reaches_every_directory`
    /// in `crate::kernel` is where it is made against a real one.
    pub share_trees: Vec<(String, u32)>,
    pub chmod_results: Script<Result<(), Errno>>,
    pub share_tree_results: Script<Result<(), Errno>>,
    /// What [`Kernel::chown`] answers. `Ok(())` unless a test says otherwise —
    /// the refusal branch reports and does not stop the service, so it has to
    /// be reachable.
    pub chown_results: Script<Result<(), Errno>>,
    next_pid: i32,
}

impl FakeKernel {
    pub fn new() -> Self {
        Self {
            mono_ms: 0,
            wall_ms: Some(0),
            wall_sets: Vec::new(),
            set_wall_results: Script::always(Ok(())),
            hostname: None,
            set_hostname_result: Ok(()),
            records: Vec::new(),
            spawned: Vec::new(),
            signals: Vec::new(),
            query_socket: None,
            // The machine Rivet ships. See the field.
            landlock: (crate::landlock::MINIMUM_ABI, 0),
            kiosk_socket: None,
            kiosk_answered: Vec::new(),
            facts_socket: None,
            facts_answered: Vec::new(),
            answered: Vec::new(),
            query_open: Script::always(Ok(())),
            placed: Vec::new(),
            cgroup_kills: Vec::new(),
            cgroup_kill_results: Script::always(Ok(())),
            notify_open: Vec::new(),
            notify_owners: BTreeMap::new(),
            listeners_open: BTreeMap::new(),
            listeners_bound: BTreeMap::new(),
            listeners_closed: Vec::new(),
            health_open: BTreeMap::new(),
            health_error: None,
            health_asked: Vec::new(),
            ask_health_error: None,
            displays_closed: Vec::new(),
            listener_error: None,
            waits: Vec::new(),
            arm_calls: 0,
            mkdir_calls: Vec::new(),
            mkdir_mode_calls: Vec::new(),
            ran: Vec::new(),
            reads: Vec::new(),
            reclaims: Vec::new(),
            issues: Vec::new(),
            issue_results: Script::always(Ok(Ok("SHA-256 issued-by-the-fake".to_string()))),
            own_oom_score_adj: Vec::new(),
            own_oom_score_adj_result: Ok(()),
            firewalls: Vec::new(),
            firewall_result: None,
            run_limits: Vec::new(),
            not_a_directory: std::collections::BTreeSet::new(),
            symlinks: std::collections::BTreeSet::new(),
            mount_calls: Vec::new(),
            writes: Vec::new(),
            attempted_writes: 0,
            syncs: Vec::new(),
            renames: Vec::new(),
            dir_syncs: Vec::new(),
            removed: Vec::new(),
            appends: Vec::new(),
            append_results: Script::always(Ok(())),
            truncations: Vec::new(),
            nofollow_writes: Vec::new(),
            sizes: BTreeMap::new(),
            modified: BTreeMap::new(),
            size_errors: BTreeMap::new(),
            live: std::collections::BTreeSet::new(),
            sync_file_results: Script::always(Ok(())),
            rename_results: Script::always(Ok(())),
            sync_dir_results: Script::always(Ok(())),
            remove_file_results: Script::always(Ok(())),
            writes_are_readable: true,
            exits: Vec::new(),
            obey_sigterm: false,
            spawn_results: Script::always(Ok(0)),
            signal_results: Script::always(Ok(())),
            place_results: Script::always(Ok(())),
            notify_results: Script::always(Ok(String::new())),
            observations: Script::always(Observation::Unavailable(Errno::ENOENT)),
            hierarchy_results: Script::always(Hierarchy::Present),
            hierarchy_calls: 0,
            wait_results: Script::always(Ok(vec![Event::Timeout])),
            arm_result: Ok(()),
            kmsg_calls: 0,
            kmsg_result: Err(Errno::ENOENT),
            ingest_bound: Vec::new(),
            ingest_result: Ok(()),
            mkdir_results: Script::always(Ok(())),
            mkdir_mode_results: Script::always(Ok(true)),
            mount_results: Script::always(Ok(())),
            write_results: Script::always(Ok(())),
            run_results: Script::always(Ok(ExitStatus::Exited(0))),
            run_says: Vec::new(),
            run_creates: Vec::new(),
            capped: Vec::new(),
            memory_results: Script::always(Ok(())),
            capped_cpu: Vec::new(),
            cpu_results: Script::always(Ok(())),
            blobs: BTreeMap::new(),
            local_reading: None,
            grew: rivet_adapter_image::Grew::Failed {
                at: rivet_adapter_image::GrowStep::ReadDeviceSize,
                errno: rivet_adapter_image::Errno::ENOENT,
            },
            grew_called: 0,
            files: BTreeMap::new(),
            exists: BTreeMap::new(),
            exists_asked: Vec::new(),
            dirs: BTreeMap::new(),
            dir_errors: BTreeMap::new(),
            made_dirs: std::collections::BTreeSet::new(),
            chowns: Vec::new(),
            dir_entries: BTreeMap::new(),
            chmods: Vec::new(),
            share_trees: Vec::new(),
            chmod_results: Script::always(Ok(())),
            share_tree_results: Script::always(Ok(())),
            chown_results: Script::always(Ok(())),
            next_pid: FIRST_FAKE_PID,
        }
    }

    pub fn files_mut(&mut self) -> &mut BTreeMap<String, Script<Result<String, Errno>>> {
        &mut self.files
    }

    pub fn exists_mut(&mut self) -> &mut BTreeMap<String, Script<Result<bool, Errno>>> {
        &mut self.exists
    }

    /// What each directory holds. Only directories are listed by the real
    /// kernel, so every name put here is one.
    pub fn dirs_mut(&mut self) -> &mut BTreeMap<String, Vec<String>> {
        &mut self.dirs
    }

    /// **Every directory removed**, in order: each one renamed aside into a
    /// directory a helper was then sent to reclaim (`crate::reclaim`). The
    /// port has no call that deletes a tree in the supervisor's loop, so a
    /// directory goes this way or not at all.
    pub fn removed_paths(&self) -> Vec<String> {
        let reclaimed = |to: &str| {
            self.reclaims
                .iter()
                .any(|(_, dir, prefix)| to.starts_with(&format!("{dir}/{prefix}")))
        };
        self.renames
            .iter()
            .filter(|(_, to)| reclaimed(to))
            .map(|(from, _)| from.clone())
            .collect()
    }

    /// Directories that fail rather than answer.
    pub fn dir_errors_mut(&mut self) -> &mut BTreeMap<String, Errno> {
        &mut self.dir_errors
    }

    /// What [`Kernel::file_size`] should report. Setting one also makes the
    /// path live, because a file with a size is a file that is there.
    pub fn sizes_mut(&mut self) -> &mut BTreeMap<String, u64> {
        &mut self.sizes
    }

    /// Paths whose size answers with an error instead of a number — a card that
    /// is failing rather than a file that is not there.
    pub fn size_errors_mut(&mut self) -> &mut BTreeMap<String, Errno> {
        &mut self.size_errors
    }

    /// What [`Kernel::modified_secs`] should report for a path. Absent means
    /// unknown, which is a legitimate answer and one retention has to keep
    /// working under.
    pub fn modified_mut(&mut self) -> &mut BTreeMap<String, u64> {
        &mut self.modified
    }

    /// Which paths the file operations have left on the card, in path order.
    ///
    /// The question `ADR-022` §8's cap is actually about: not "was rotate
    /// called" but "how many files are there". A rotation that renamed without
    /// removing would pass every call-sequence assertion and fill the card.
    pub fn live_files(&self) -> Vec<String> {
        let mut out: Vec<String> = self.live.iter().cloned().collect();
        out.extend(self.sizes.keys().cloned());
        out.sort();
        out.dedup();
        out
    }

    /// What is at a path now, without recording a read.
    ///
    /// A test's *inspection* is not part of the call sequence it asserts on —
    /// a test proves a component did not read a file by
    /// looking at [`FakeKernel::reads`], and a helper that appended to it would
    /// break them. The script is cloned rather than advanced, for the same
    /// reason.
    pub fn contents(&self, path: &str) -> Option<String> {
        self.files
            .get(path)
            .cloned()
            .and_then(|mut script| script.next().ok())
    }
}

/// The last component of a path, for a fake that has to keep a directory
/// listing honest across a rename.
fn leaf(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

impl Kernel for FakeKernel {
    fn now_ms(&mut self) -> u64 {
        self.mono_ms
    }

    fn wall_ms(&mut self) -> Option<u64> {
        self.wall_ms
    }

    /// **`None` by default, and the default is the honest one.**
    ///
    /// This fake has no C library and no `/etc/localtime`, so it cannot answer
    /// what musl would make of one — and a fake that invented an offset would
    /// let a test claim a zone had taken effect on a machine where nothing had
    /// happened. What a test off a board can prove about `Kernel::local_time`
    /// is that the supervisor *asks* and *reports what it is told*; that the
    /// answer is right is the board's to settle (`ADR-026` §4).
    fn local_time(&mut self) -> Option<crate::kernel::LocalReading> {
        self.local_reading.clone()
    }

    fn set_hostname(&mut self, name: &str) -> Result<(), Errno> {
        // Recorded only on success, unlike `wall_sets`: a refused `sethostname`
        // leaves the kernel holding whatever it held, and a fake that recorded
        // the attempt as a state would let a test pass against a machine whose
        // name never changed.
        self.set_hostname_result?;
        self.hostname = Some(name.to_owned());
        Ok(())
    }

    fn set_wall_ms(&mut self, unix_ms: u64) -> Result<(), Errno> {
        // Recorded whether or not it succeeded: a set that was attempted and
        // refused is a different state from one that was never attempted, and
        // `crate::clock` behaves differently in each.
        self.wall_sets.push(unix_ms);
        let answer = self.set_wall_results.next();
        if answer.is_ok() {
            // A real kernel's clock reads back what was set, and the whole claim
            // `crate::clock` makes is about what records written *afterwards*
            // are stamped with. A fake that did not move would make that
            // untestable.
            self.wall_ms = Some(unix_ms);
        }
        answer
    }

    fn sync_file(&mut self, path: &str) -> Result<(), Errno> {
        self.syncs.push(path.to_string());
        self.sync_file_results.next()
    }

    fn append_file(&mut self, path: &str, contents: &[u8], _mode: u32) -> Result<(), Errno> {
        let answer = self.append_results.next();
        if answer.is_ok() {
            self.appends.push((path.to_string(), contents.to_vec()));
            self.live.insert(path.to_string());
            *self.sizes.entry(path.to_string()).or_insert(0) += contents.len() as u64;
        }
        answer
    }

    fn file_size(&mut self, path: &str) -> Result<u64, Errno> {
        if let Some(e) = self.size_errors.get(path) {
            return Err(*e);
        }
        self.sizes.get(path).copied().ok_or(Errno::ENOENT)
    }

    fn modified_secs(&mut self, path: &str) -> Option<u64> {
        self.modified.get(path).copied()
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), Errno> {
        self.renames.push((from.to_string(), to.to_string()));

        // **A directory counts as being there, and moves with everything under
        // it.** Added 2026-08-26 with `crate::repos`, which renames a bare
        // repository — a directory with an identifier file inside it. The fake
        // modelled files only, so it answered `ENOENT` to a rename a real
        // kernel performs, and the projection's rename path was untestable.
        //
        // This is the same class of gap as `list_dir`'s below, in the direction
        // that costs less: a fake *stricter* than the kernel fails a test that
        // should pass, which is noticed immediately. The other direction hid a
        // real defect for a day.
        let prefix = format!("{from}/");
        let under: Vec<String> = self
            .files
            .keys()
            .chain(self.live.iter())
            .filter(|p| p.starts_with(&prefix))
            .cloned()
            .collect();
        let is_a_directory = self.dirs.contains_key(from) || !under.is_empty();

        // A rename of something that is not there is `ENOENT`, as it is on a
        // real filesystem — and rotation depends on the difference: it renames
        // files that may never have existed, and treats that as nothing to do.
        // A fake that succeeded regardless would let a rotation which moved
        // phantom files pass.
        if !is_a_directory
            && !self.live.contains(from)
            && !self.sizes.contains_key(from)
            && !self.files.contains_key(from)
        {
            return Err(Errno::ENOENT);
        }
        let answer = self.rename_results.next();
        if answer.is_ok() {
            self.live.remove(from);
            self.live.insert(to.to_string());
            if let Some(size) = self.sizes.remove(from) {
                self.sizes.insert(to.to_string(), size);
            }
            // Moved rather than copied, as a real filesystem does — the temp has
            // to stop existing, or `a save leaves one file` could never be
            // asserted.
            if let Some(script) = self.files.remove(from) {
                self.files.insert(to.to_string(), script);
            }
            if let Some(script) = self.exists.remove(from) {
                self.exists.insert(to.to_string(), script);
            }
            // Everything under the directory comes with it, which is what makes
            // this a move rather than an unlink of the top name.
            for path in under {
                let moved = format!("{to}/{}", &path[prefix.len()..]);
                if let Some(script) = self.files.remove(&path) {
                    self.files.insert(moved.clone(), script);
                }
                if let Some(script) = self.exists.remove(&path) {
                    self.exists.insert(moved.clone(), script);
                }
                if let Some(size) = self.sizes.remove(&path) {
                    self.sizes.insert(moved.clone(), size);
                }
                if self.live.remove(&path) {
                    self.live.insert(moved);
                }
            }
            // And the listing that named it. A directory that answered under
            // its old name after a move would make a convergence test pass
            // against a machine that had renamed nothing.
            if let Some(entries) = self.dirs.remove(from) {
                self.dirs.insert(to.to_string(), entries);
            }
            // Every directory beneath it moves too, as on a real filesystem, so
            // a test can tell a tree that was renamed aside whole from one that
            // left its `data` behind.
            let nested: Vec<String> = self
                .dirs
                .keys()
                .filter(|known| known.starts_with(&prefix))
                .cloned()
                .collect();
            for known in nested {
                if let Some(entries) = self.dirs.remove(&known) {
                    self.dirs
                        .insert(format!("{to}/{}", &known[prefix.len()..]), entries);
                }
            }
            for entries in self.dirs.values_mut() {
                let (old_leaf, new_leaf) = (leaf(from), leaf(to));
                for entry in entries.iter_mut() {
                    if entry == old_leaf {
                        *entry = new_leaf.to_string();
                    }
                }
            }
        }
        answer
    }

    fn sync_dir(&mut self, path: &str) -> Result<(), Errno> {
        self.dir_syncs.push(path.to_string());
        self.sync_dir_results.next()
    }

    fn remove_file(&mut self, path: &str) -> Result<(), Errno> {
        self.removed.push(path.to_string());
        // Same reasoning as `rename`: rotation removes a file that may never
        // have existed and must read `ENOENT` as nothing to do.
        if !self.live.contains(path)
            && !self.sizes.contains_key(path)
            && !self.files.contains_key(path)
        {
            return Err(Errno::ENOENT);
        }
        let answer = self.remove_file_results.next();
        if answer.is_ok() {
            self.files.remove(path);
            self.exists.remove(path);
            self.live.remove(path);
            self.sizes.remove(path);
        }
        answer
    }

    fn emit(&mut self, record: &Record) {
        self.records.push(record.clone());
    }

    fn read_file(&mut self, path: &str) -> Result<String, Errno> {
        self.reads.push(path.to_string());
        match self.files.get_mut(path) {
            Some(script) => script.next(),
            None => Err(Errno::ENOENT),
        }
    }

    fn mkdir_p(&mut self, path: &str) -> Result<(), Errno> {
        self.mkdir_calls.push(path.to_string());
        self.mkdir_results.next()
    }

    fn chown(&mut self, path: &str, uid: u32, gid: u32) -> Result<(), Errno> {
        // Recorded before the answer, on `mkdir_mode`'s reasoning: a chown that
        // was never attempted is a different defect from one that was refused,
        // and only the record can tell them apart.
        self.chowns.push((path.to_string(), uid, gid));
        self.chown_results.next()
    }

    fn chmod(&mut self, path: &str, mode: u32) -> Result<(), Errno> {
        self.chmods.push((path.to_string(), mode));
        self.chmod_results.next()
    }

    fn share_tree(&mut self, path: &str, gid: u32) -> Result<(), Errno> {
        self.share_trees.push((path.to_string(), gid));
        self.share_tree_results.next()
    }

    fn mkdir_mode(&mut self, path: &str, mode: u32) -> Result<bool, Errno> {
        // Recorded whether or not it succeeded, for the reason `mount` gives:
        // "which call was attempted" is the question most of these tests ask,
        // and a directory that was never attempted is a different defect from
        // one that was attempted and refused.
        self.mkdir_mode_calls.push((path.to_string(), mode));
        // Idempotence, as a real filesystem provides it rather than as a script
        // remembers to. A second pass over the same plan — which is what a
        // service restart makes — must report that it created nothing, and a
        // fake that answered `Ok(true)` for ever would make that untestable.
        // The script is not consulted here, so a scripted sequence counts real
        // attempts rather than repeats.
        if self.made_dirs.contains(path) {
            return Ok(false);
        }
        let answer = self.mkdir_mode_results.next();
        if matches!(answer, Ok(true)) {
            self.made_dirs.insert(path.to_string());
            // A directory that was created is a directory that now exists, so
            // a later `file_exists` on it must say so.
            self.exists
                .insert(path.to_string(), Script::always(Ok(true)));
        }
        answer
    }

    fn run_to_completion(&mut self, program: &Program) -> Result<Completed, Errno> {
        let answer = self.run_results.next();
        if let Ok(status) = answer {
            self.ran.push(program.clone());
            if status.is_success() {
                for path in self.run_creates.clone() {
                    self.exists.insert(path, Script::always(Ok(true)));
                }
            }
        }
        // `run_results` stays a script of *statuses*, because that is what
        // almost every test about a one-shot program is choosing. What such a program
        // SAYS is a separate axis and a separate field, so a test that does not
        // care sets nothing — and a fake that made every generator silent by
        // construction would have hidden `ADR-022` §4's capture here entirely.
        answer.map(|status| Completed {
            status,
            output: self.run_says.clone(),
        })
    }

    fn run_within(&mut self, program: &Program, limit_ms: u64) -> Result<Completed, Errno> {
        // `run_results` decides the answer exactly as it does for
        // `run_to_completion`, so a test makes a program time out by scripting
        // `Err(ETIMEDOUT)`. The limit is recorded for the test to assert on.
        self.run_limits.push(limit_ms);
        self.run_to_completion(program)
    }

    fn mount(
        &mut self,
        source: &str,
        target: &str,
        fstype: &str,
        flags: MountFlags,
        data: Option<&str>,
    ) -> Result<(), Errno> {
        let answer = self.mount_results.next();
        // Recorded whether or not it succeeded: "which call was attempted" is
        // the question most of the projection's tests ask, and a refused
        // remount that was never attempted is a different defect from one that
        // was attempted and refused.
        self.mount_calls.push(MountCall {
            source: source.to_string(),
            target: target.to_string(),
            fstype: fstype.to_string(),
            flags,
            data: data.map(str::to_string),
        });
        answer
    }

    fn file_exists(&mut self, path: &str) -> Result<bool, Errno> {
        self.exists_asked.push(path.to_string());
        match self.exists.get_mut(path) {
            Some(script) => script.next(),
            // The machine Rivet actually is: `cmd/rivet` is not built, so
            // nothing is at `LOGIN_SHELL_PATH`. A fake that answered `true` by
            // default would make every test agree with a world that does not
            // exist.
            None => Ok(false),
        }
    }

    fn is_symlink(&mut self, path: &str) -> Result<bool, Errno> {
        Ok(self.symlinks.contains(path))
    }

    fn write_file(&mut self, path: &str, contents: &[u8], mode: u32) -> Result<(), Errno> {
        self.attempted_writes += 1;
        let answer = self.write_results.next();
        if answer.is_ok() {
            self.truncations.push(path.to_string());
            self.live.insert(path.to_string());
            self.sizes.insert(path.to_string(), contents.len() as u64);
            self.writes.push(WriteCall {
                path: path.to_string(),
                contents: contents.to_vec(),
                mode,
            });
            if self.writes_are_readable {
                self.files.insert(
                    path.to_string(),
                    Script::always(Ok(String::from_utf8_lossy(contents).into_owned())),
                );
                // A file that was written is a file that now exists. Added
                // 2026-08-23 with the first caller that converges a file, which asks
                // `file_exists` before reading — without this the fake would say
                // a file it had just written was absent, and the
                // no-op-on-identical-bytes property would be untestable.
                //
                // Guarded by the same flag, because a test that turns
                // `writes_are_readable` off is describing a write that did not
                // take, and such a write must not make the file appear either.
                self.exists
                    .insert(path.to_string(), Script::always(Ok(true)));
            }
        }
        answer
    }

    /// **The same recording, plus a note of which method was used.**
    ///
    /// A fake cannot follow a symlink, so it cannot demonstrate the guards;
    /// what it CAN prove is that the settings write asks for them, which is the
    /// half a unit test is able to hold. `nofollow_writes` is what a test
    /// asserts against — see `runtime`'s.
    fn write_file_nofollow(
        &mut self,
        path: &str,
        contents: &[u8],
        mode: u32,
    ) -> Result<(), Errno> {
        self.nofollow_writes.push(path.to_string());
        self.write_file(path, contents, mode)
    }

    fn arm_signals(&mut self) -> Result<(), Errno> {
        self.arm_calls += 1;
        self.arm_result
    }

    fn open_kmsg(&mut self) -> Result<(), Errno> {
        self.kmsg_calls += 1;
        self.kmsg_result
    }

    /// `ADR-032` §3.2's query socket. Records the path so a test can assert it
    /// was bound, and answers nothing — this fake has no socket.
    fn open_query(&mut self, path: &str) -> Result<(), Errno> {
        self.query_socket = Some(path.to_string());
        self.query_open.next()
    }

    /// What the supervisor said, kept so a test can read it. There is no
    /// connection to write to, so this is the whole of the far end.
    fn answer_query(&mut self, answer: &str) {
        self.answered.push(answer.to_string());
    }

    /// `rivet_core::kiosk`'s socket. The **owner is recorded**, because the
    /// grant is the whole security property: a test that only checked the path
    /// would pass on a build that bound it root-owned and never handed it over.
    fn open_kiosk(
        &mut self,
        path: &str,
        owner: Option<rivet_core::service::AppIdentity>,
    ) -> Result<(), Errno> {
        self.kiosk_socket = Some((path.to_string(), owner));
        Ok(())
    }

    /// `rivet_core::facts`'s socket. Records the path so a test can assert it
    /// was bound; there is no socket in this fake and nothing to receive on.
    fn open_facts(&mut self, path: &str) -> Result<(), Errno> {
        self.facts_socket = Some(path.to_string());
        Ok(())
    }

    /// What the supervisor answered an application that asked for the facts.
    fn answer_facts(&mut self, answer: &str) {
        self.facts_answered.push(answer.to_string());
    }

    /// Kept apart from `answered`, so a test cannot mistake an answer to the
    /// operator for an answer to the launcher.
    fn answer_kiosk(&mut self, answer: &str) {
        self.kiosk_answered.push(answer.to_string());
    }

    fn open_journal_ingest(&mut self, path: &str) -> Result<(), Errno> {
        self.ingest_bound.push(path.to_string());
        self.ingest_result
    }

    fn wait(&mut self, timeout_ms: Option<u64>) -> Result<Vec<Event>, Errno> {
        self.waits.push(timeout_ms);
        self.wait_results.next()
    }

    fn open_notify(
        &mut self,
        service: &str,
        owner: Option<rivet_core::service::AppIdentity>,
    ) -> Result<String, Errno> {
        match self.notify_results.next() {
            Ok(path) => {
                // Recorded so a test can assert the socket is owned by the
                // identity that will write to it — the whole of the 2026-08-25
                // fix, and a thing that is invisible in the returned path.
                self.notify_owners
                    .insert(service.to_string(), owner.map(|id| (id.uid, id.gid)));
                if !self.notify_open.iter().any(|s| s == service) {
                    self.notify_open.push(service.to_string());
                    self.notify_open.sort();
                }
                Ok(if path.is_empty() {
                    format!("/run/rivet/notify/{service}")
                } else {
                    path
                })
            }
            Err(e) => Err(e),
        }
    }

    fn open_listeners(
        &mut self,
        service: &str,
        sockets: &[rivet_core::package::application::Socket],
        proxied: bool,
        owner: Option<rivet_core::service::AppIdentity>,
    ) -> Result<Vec<String>, Errno> {
        use rivet_core::package::application::Socket;
        if let Some(e) = self.listener_error {
            return Err(e);
        }
        // Recorded rather than merely counted: a test asserting the ORDER is
        // asserting the thing that matters, because `LISTEN_FDS` hands the
        // descriptors over positionally and a reordering would give an
        // application the wrong socket without any error anywhere.
        let names: Vec<String> = sockets
            .iter()
            .map(|socket| match socket {
                Socket::Tcp(port) | Socket::TcpDirect(port) => format!("tcp-{port}"),
                Socket::Unix(name) => name.clone(),
                Socket::Udp(port) | Socket::UdpDirect(port) => format!("udp-{port}"),
            })
            .collect();
        // **The address each inet socket would be bound on**, by the function
        // `LibcKernel` binds with — `ADR-046` §7. A unix socket has none.
        let bound: Vec<String> = sockets
            .iter()
            .filter_map(|socket| Some(format!("{}:{}", socket.bind_address(proxied)?, socket.port()?)))
            .collect();
        self.listeners_bound.insert(service.to_string(), bound);
        self.listeners_open
            .insert(service.to_string(), (names.clone(), owner.map(|id| id.uid)));
        Ok(names)
    }

    fn close_listeners(&mut self, service: &str) {
        self.listeners_open.remove(service);
        self.listeners_closed.push(service.to_string());
    }

    /// **`ADR-045` §3's reserved entry, appended where the real one appends
    /// it.** Recorded rather than defaulted away, because the thing a test
    /// needs to be able to assert is that the token lands AFTER the manifest's
    /// own — a prepended one would move every descriptor an application was
    /// promised.
    fn open_health(&mut self, service: &str) -> Result<Option<String>, Errno> {
        if let Some(e) = self.health_error {
            return Err(e);
        }
        let token = rivet_core::health::CHANNEL_NAME.to_string();
        self.health_open
            .entry(service.to_string())
            .or_insert_with(|| token.clone());
        if let Some((names, _)) = self.listeners_open.get_mut(service) {
            names.push(token.clone());
        } else {
            self.listeners_open
                .insert(service.to_string(), (vec![token.clone()], None));
        }
        Ok(Some(token))
    }

    /// The question, recorded. **What comes back is `health_answers`**, and a
    /// service with no entry there answers nothing — which is the wedged case
    /// and is the default on purpose.
    fn ask_health(&mut self, service: &str) -> Result<(), Errno> {
        if let Some(e) = self.ask_health_error {
            return Err(e);
        }
        self.health_asked.push(service.to_string());
        Ok(())
    }

    fn close_health(&mut self, service: &str) {
        self.health_open.remove(service);
    }

    fn close_notify(&mut self, service: &str) {
        self.notify_open.retain(|s| s != service);
    }

    /// Recorded rather than defaulted away — see [`FakeKernel::displays_closed`].
    fn close_display(&mut self, service: &str) {
        self.displays_closed.push(service.to_string());
    }

    fn landlock_abi(&mut self) -> (u32, i32) {
        self.landlock
    }

    fn apply_firewall(
        &mut self,
        policy: &rivet_adapter_net::firewall::Policy,
    ) -> Result<rivet_adapter_net::firewall::Applied, String> {
        self.firewalls.push(policy.clone());
        self.firewall_result.clone().unwrap_or_else(|| {
            Ok(rivet_adapter_net::firewall::Applied {
                found: policy.guards(),
                unrecognised: 0,
                raw: Vec::new(),
            })
        })
    }

    fn set_own_oom_score_adj(&mut self, value: i32) -> Result<(), Errno> {
        self.own_oom_score_adj.push(value);
        self.own_oom_score_adj_result
    }

    fn spawn(&mut self, service: &str, program: &Program) -> Result<i32, Errno> {
        let answer = self.spawn_results.next();
        match answer {
            Ok(0) => {
                let pid = self.next_pid;
                self.next_pid += 1;
                self.spawned.push((service.to_string(), program.clone()));
                Ok(pid)
            }
            Ok(pid) => {
                self.spawned.push((service.to_string(), program.clone()));
                Ok(pid)
            }
            Err(e) => Err(e),
        }
    }

    fn cec_send(&mut self, lit: bool, transaction: CecTransaction) -> Result<CecWorker, Errno> {
        // **No fork here.** `cargo test` runs many tests on many threads
        // inside one process, so forking from one would carry exactly the
        // hazard `CecWorker::Child` exists to avoid — see `CecWorker`'s own
        // doc. `transaction` is called in this thread instead, and the
        // answer is handed back already resolved so `Kernel::cec_poll`'s
        // ordinary non-blocking shape still applies to it.
        Ok(CecWorker::Resolved(Some(transaction(lit))))
    }

    fn reclaim_in_child(&mut self, service: &str, dir: &str, prefix: &str) -> Result<(), Errno> {
        // **No fork here**, for `cec_send`'s reason above. The helper's work
        // is done in place instead, so a test can assert on what is LEFT.
        self.reclaims
            .push((service.to_string(), dir.to_string(), prefix.to_string()));
        let start = format!("{dir}/{prefix}");
        self.dirs.retain(|known, _| !known.starts_with(&start));
        if let Some(names) = self.dirs.get_mut(dir) {
            names.retain(|n| !n.starts_with(prefix));
        }
        Ok(())
    }

    fn issue_in_child(
        &mut self,
        order: &crate::kernel::IssueOrder,
    ) -> Result<Result<String, String>, Errno> {
        // **No fork here**, for `cec_send`'s reason above.
        self.issues.push(order.clone());
        let answer = self.issue_results.next();
        if matches!(answer, Ok(Ok(_))) {
            for target in &order.targets {
                let names = target
                    .root
                    .iter()
                    .chain([&target.key, &target.certificate]);
                for name in names {
                    let path = format!("{}/{name}", target.dir);
                    self.files
                        .insert(path.clone(), Script::always(Ok(format!("issued {name}"))));
                    self.exists.insert(path, Script::always(Ok(true)));
                }
            }
        }
        answer
    }

    fn reap_ready(&mut self) -> Vec<(i32, ExitStatus)> {
        std::mem::take(&mut self.exits)
    }

    fn signal_process(&mut self, pid: i32, sig: Sig) -> Result<(), Errno> {
        self.signals.push((pid, sig));
        let answer = self.signal_results.next();
        if self.obey_sigterm && answer.is_ok() && sig == Sig::Term {
            self.exits.push((pid, ExitStatus::Signalled(15)));
        }
        answer
    }

    fn observe(&mut self, _service: &str) -> Observation {
        self.observations.next()
    }

    fn cgroup_hierarchy(&mut self) -> Hierarchy {
        self.hierarchy_calls += 1;
        self.hierarchy_results.next()
    }

    fn place_in_cgroup(&mut self, service: &str, pid: i32) -> Result<(), Errno> {
        let answer = self.place_results.next();
        if answer.is_ok() {
            self.placed.push((service.to_string(), pid));
        }
        answer
    }

    fn cgroup_kill(&mut self, service: &str) -> Result<(), Errno> {
        // Recorded before the answer, on `chown`'s reasoning: an attempt that
        // was refused is a different fact from one that was never made, and
        // only the record can tell them apart.
        self.cgroup_kills.push(service.to_string());
        self.cgroup_kill_results.next()
    }

    /// Every entry, whatever type it is — the `/dev` case. Backed by its own
    /// map, because the point of the method is that it answers about things
    /// `dirs` deliberately does not hold.
    fn list_dir_entries(&mut self, path: &str) -> Result<Vec<String>, Errno> {
        if let Some(e) = self.dir_errors.get(path) {
            return Err(*e);
        }
        let mut names = self.dir_entries.get(path).cloned().ok_or(Errno::ENOENT)?;
        names.sort();
        Ok(names)
    }

    fn list_dir(&mut self, path: &str) -> Result<Vec<String>, Errno> {
        if let Some(e) = self.dir_errors.get(path) {
            return Err(*e);
        }
        // An absent directory is `ENOENT`, which is what a real one does and
        // what `crate::applications` reads as "nothing is installed". A fake
        // that answered `Ok(vec![])` would make the interesting branch — a
        // machine whose `/state/app` cannot be read — unreachable from a test.
        let names = self.dirs.get(path).cloned().ok_or(Errno::ENOENT)?;
        // **Directories only, because the real one answers directories only.**
        // `LibcKernel::list_dir` skips anything that is not a directory — which
        // is deliberate, so a symlink into `/system` cannot present itself as an
        // installed application — and this fake did not.
        //
        // **That let a real defect through on 2026-08-25**: the first uninstall
        // on a board enumerated an application's directory expecting to see
        // `current`, removed everything it was shown, reported success, and left
        // the link behind. The test passed because the fake was more generous
        // than the kernel, which is the one way a fake can be worse than no test
        // at all.
        //
        // An entry is a directory here **unless a test says it is not** —
        // [`FakeKernel::not_a_directory`]. That way round because every existing
        // fixture means "a directory" and the exception is the interesting case.
        Ok(names
            .into_iter()
            .filter(|name| !self.not_a_directory.contains(&format!("{path}/{name}")))
            .collect())
    }

    fn apply_memory_limit(&mut self, service: &str, bytes: u64) -> Result<(), Errno> {
        let answer = self.memory_results.next();
        if answer.is_ok() {
            self.capped.push((service.to_string(), bytes));
        }
        answer
    }

    fn apply_cpu_limit(&mut self, service: &str, quota_us: u64) -> Result<(), Errno> {
        let answer = self.cpu_results.next();
        if answer.is_ok() {
            self.capped_cpu.push((service.to_string(), quota_us));
        }
        answer
    }

    fn read_file_bytes(&mut self, path: &str, max: usize) -> Result<Vec<u8>, Errno> {
        // Recorded like `read_file`, because "was this read at all" is a
        // real assertion: ADR-015 §4.2 reads the trust store on EVERY launch
        // and a cached one would defeat revocation.
        self.reads.push(path.to_string());
        let Some(bytes) = self.blobs.get(path) else {
            // Falls back to the text files, so a test that put a document
            // somewhere with `files_mut` does not also have to put it here.
            let text = self.read_file(path)?;
            let bytes = text.into_bytes();
            return if bytes.len() > max {
                Err(Errno::EFBIG)
            } else {
                Ok(bytes)
            };
        };
        if bytes.len() > max {
            return Err(Errno::EFBIG);
        }
        Ok(bytes.clone())
    }

    fn grow_state(&mut self) -> rivet_adapter_image::Grew {
        self.grew_called = self.grew_called.saturating_add(1);
        self.grew.clone()
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

    #[test]
    fn an_unconfigured_path_reads_as_absent() {
        let mut k = FakeKernel::new();
        assert_eq!(k.read_file("/proc/anything"), Err(Errno::ENOENT));
    }

    /// The default must describe the machine Rivet actually is.
    ///
    /// ~~nothing mounts a cgroup2 hierarchy, so the question cannot be
    /// asked.~~ **Corrected 2026-08-23**, along with the doc comment on
    /// [`FakeKernel`] that said the same thing: `stage0_plan` has mounted
    /// cgroup2 since 2026-08-22 and a board was observed doing it.
    ///
    /// The pair below is the state a real first boot is in, and it is the state
    /// that produced the defect `ADR-017` §3.1.3.1 fixes: the hierarchy is
    /// there, this service's cgroup is not, and nothing about that is a hazard.
    /// A fake that answered `Empty` by default would still be wrong — an empty
    /// cgroup is one that *exists*, which is a different fact again.
    #[test]
    fn the_fake_defaults_to_a_first_boot_on_a_healthy_machine() {
        let mut k = FakeKernel::new();
        assert_eq!(k.cgroup_hierarchy(), Hierarchy::Present);
        assert_eq!(
            k.observe("anything"),
            Observation::Unavailable(Errno::ENOENT)
        );
    }

    /// The blind machine, which a test has to ask for.
    #[test]
    fn a_scripted_hierarchy_is_what_makes_the_dangerous_case_reachable() {
        let mut k = FakeKernel::new();
        k.hierarchy_results = Script::always(Hierarchy::Unreachable(Errno::ENOENT));
        assert_eq!(k.cgroup_hierarchy(), Hierarchy::Unreachable(Errno::ENOENT));
        assert_eq!(k.hierarchy_calls, 1);
    }

    #[test]
    fn spawning_hands_out_distinct_pids_when_a_test_does_not_care() {
        let mut k = FakeKernel::new();
        let p = Program {
            path: "/bin/x".to_string(),
            args: Vec::new(),
            env: Vec::new(),
            identity: None,
            confinement: None,
            groups: Vec::new(),
            capabilities: Vec::new(),
        };
        assert_eq!(k.spawn("a", &p), Ok(FIRST_FAKE_PID));
        assert_eq!(k.spawn("b", &p), Ok(FIRST_FAKE_PID + 1));
        assert_eq!(k.spawned.len(), 2);
    }

    #[test]
    fn a_scripted_spawn_failure_records_no_spawn() {
        let mut k = FakeKernel::new();
        k.spawn_results = Script::always(Err(Errno::ENOENT));
        let p = Program {
            path: "/bin/x".to_string(),
            args: Vec::new(),
            env: Vec::new(),
            identity: None,
            confinement: None,
            groups: Vec::new(),
            capabilities: Vec::new(),
        };
        assert_eq!(k.spawn("a", &p), Err(Errno::ENOENT));
        assert!(k.spawned.is_empty());
    }

    #[test]
    fn reaping_drains_the_pending_exits_exactly_once() {
        let mut k = FakeKernel::new();
        k.exits.push((7, ExitStatus::Exited(0)));
        assert_eq!(k.reap_ready(), vec![(7, ExitStatus::Exited(0))]);
        assert!(k.reap_ready().is_empty());
    }

    #[test]
    fn a_notify_socket_opens_and_closes() {
        let mut k = FakeKernel::new();
        assert_eq!(k.open_notify("a", None), Ok("/run/rivet/notify/a".to_string()));
        assert_eq!(k.notify_open, vec!["a".to_string()]);
        k.close_notify("a");
        assert!(k.notify_open.is_empty());
        // Closing something that was never opened is not an error.
        k.close_notify("a");
    }

    #[test]
    fn a_failed_placement_is_not_recorded_as_a_placement() {
        let mut k = FakeKernel::new();
        k.place_results = Script::always(Err(Errno::EPERM));
        assert_eq!(k.place_in_cgroup("a", 1), Err(Errno::EPERM));
        assert!(k.placed.is_empty());
    }

    /// The fake's `mkdir_mode` has to be idempotent the way a filesystem is,
    /// or a caller cannot be tested for what it does on a second pass at all:
    /// every restart would look like a first boot.
    #[test]
    fn a_directory_this_fake_created_is_already_there_the_second_time() {
        let mut k = FakeKernel::new();
        assert_eq!(k.mkdir_mode("/state/git/photos.git", 0o700), Ok(true));
        assert_eq!(k.mkdir_mode("/state/git/photos.git", 0o700), Ok(false));
        assert_eq!(
            k.mkdir_mode_calls.len(),
            2,
            "both attempts must be recorded"
        );
        // And it exists afterwards, which is what a real one does.
        assert_eq!(k.file_exists("/state/git/photos.git"), Ok(true));
    }

    #[test]
    fn a_refused_mkdir_creates_nothing_and_is_still_recorded() {
        let mut k = FakeKernel::new();
        k.mkdir_mode_results = Script::always(Err(Errno::EROFS));
        assert_eq!(k.mkdir_mode("/state/git/photos.git", 0o700), Err(Errno::EROFS));
        assert_eq!(k.mkdir_mode_calls, vec![("/state/git/photos.git".to_string(), 0o700)]);
        assert_eq!(k.file_exists("/state/git/photos.git"), Ok(false));
    }

    /// The default is *no* output, because a program that exits 0 and writes
    /// nothing is a real state and the one a fake told nothing should be in.
    #[test]
    fn a_run_produces_no_file_unless_the_test_says_which() {
        let program = Program {
            path: "/system/bin/git".to_string(),
            identity: None,
            confinement: None,
            groups: Vec::new(),
            capabilities: Vec::new(),
            args: vec!["init".to_string()],
            env: Vec::new(),
        };
        let mut k = FakeKernel::new();
        assert_eq!(
            k.run_to_completion(&program).map(|c| c.status),
            Ok(ExitStatus::Exited(0))
        );
        assert_eq!(k.ran.len(), 1);
        assert_eq!(k.file_exists("/state/git/photos.git/rivet-repo-id"), Ok(false));

        let mut k = FakeKernel::new();
        k.run_creates = vec!["/state/git/photos.git/rivet-repo-id".to_string()];
        assert_eq!(
            k.run_to_completion(&program).map(|c| c.status),
            Ok(ExitStatus::Exited(0))
        );
        assert_eq!(k.file_exists("/state/git/photos.git/rivet-repo-id"), Ok(true));
    }

    #[test]
    fn a_run_that_failed_produces_nothing_however_run_creates_is_set() {
        let program = Program {
            path: "/x".to_string(),
            args: Vec::new(),
            env: Vec::new(),
            identity: None,
            confinement: None,
            groups: Vec::new(),
            capabilities: Vec::new(),
        };
        let mut k = FakeKernel::new();
        k.run_creates = vec!["/state/git/photos.git/rivet-repo-id".to_string()];
        k.run_results = Script::always(Ok(ExitStatus::Exited(1)));
        assert_eq!(
            k.run_to_completion(&program).map(|c| c.status),
            Ok(ExitStatus::Exited(1))
        );
        assert_eq!(k.file_exists("/state/git/photos.git/rivet-repo-id"), Ok(false));

        let mut k = FakeKernel::new();
        k.run_creates = vec!["/state/git/photos.git/rivet-repo-id".to_string()];
        k.run_results = Script::always(Err(Errno::ENOENT));
        assert_eq!(k.run_to_completion(&program), Err(Errno::ENOENT));
        assert!(k.ran.is_empty(), "a run that never started is not a run");
    }

    #[test]
    fn every_read_is_recorded_so_a_test_can_assert_a_file_was_not_read() {
        let mut k = FakeKernel::new();
        let _ = k.read_file("/state/git/photos.git/rivet-repo-id");
        assert_eq!(k.reads, vec!["/state/git/photos.git/rivet-repo-id".to_string()]);
    }

    #[test]
    fn time_only_moves_when_a_test_moves_it() {
        let mut k = FakeKernel::new();
        assert_eq!(k.now_ms(), 0);
        assert_eq!(k.now_ms(), 0);
        k.mono_ms = 5_000;
        assert_eq!(k.now_ms(), 5_000);
    }
}
