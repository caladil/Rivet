//! **What an application may reach, derived from what it declared.**
//! `ADR-042`, ruled 2026-09-04.
//!
//! `ADR-015` §5.1's *"Filesystem restriction — Landlock"* row had no kernel to
//! run on until Rivet built its own, and this module is the half of that row
//! which does not name a kernel at all: a **policy**, in Rivet's vocabulary,
//! computed from a manifest and two path functions this crate already owns.
//! `cmd/rivet-sysd` turns it into a Landlock ruleset and applies it between
//! `fork` and `exec`.
//!
//! # Why the policy lives here and the syscall does not
//!
//! `AGENTS.md` §3: `rivet-core` names no platform type, and a Landlock access
//! bit is one. So [`Access`] is Rivet's own set of verbs, the adapter maps it,
//! and the mapping is a table somebody can read rather than a constant folded
//! into a syscall site. The property that buys: **this module is testable with
//! no kernel**, and the answers below are asserted against the same
//! `crate::service` functions that compose `RIVET_APP_DIR`, `RIVET_APP_DATA`
//! and `LD_LIBRARY_PATH`, so a path that moves moves in one place.
//!
//! # The rule the table obeys
//!
//! **An application is handed what it needs and does not go and take one**
//! (`SPEC-03` §2). Every row of [`policy`] is therefore either a path the
//! machine *told* the application about — `RIVET_APP_DIR`, `RIVET_APP_DATA`,
//! the declared closure, the sockets under `/run/rivet` — or one it cannot run
//! without and no manifest can describe: the ELF interpreter, the `/etc`
//! projection, and the boot id the SDK stamps on a journal record.
//!
//! Those last three are marked in the table as *implicit* and they are the
//! rows to argue about, because they are the ones no author asked for.
//!
//! **The rule has a second half, and leaving it unwritten cost a display.**
//! *Handed* is not a figure of speech here: it is a path the supervisor puts in
//! the child's environment or writes into `/run/rivet` for it. So **every
//! variable `crate::service` composes must have a row**, and on 2026-09-04 one
//! did not — [`crate::settings::path`] moved out of `RIVET_APP_DATA` the same
//! night this module was written, and `RIVET_APP_SETTINGS` named a file no
//! policy granted. `handed_paths_are_granted_paths` in the tests below is that
//! half of the rule, asserted rather than remembered.
//!
//! # What is deliberately NOT here
//!
//! * ~~**`/system`.** Read-only vendor content, and nothing an application is
//!   handed lives in it.~~ **Amended 2026-09-05, `ADR-042` §8**: the second
//!   clause was false when it was written. [`crate::etc::SYSTEM_FONT`] has
//!   lived in `/system/share/fonts` since 2026-09-01 precisely so that several
//!   programs could share it, `rivet-sdk`'s `text` module publishes the path to
//!   every application, and the first confined boot refused all three faces —
//!   *"NO SYSTEM FONT — /system/share/fonts/Geist.ttf: Permission denied"* on a
//!   board, with nothing on the screen. So **[`FONT_DIR`] is granted and the
//!   rest of `/system` is not.**
//!
//!   The original sentence's first clause still stands and is why the grant is
//!   a leaf and not the tree: an application that can read
//!   `/system/bin/rivet-sysd` is reading the supervisor's own program, before
//!   this policy it could, and `SPEC-01` §5.2 already says an application has
//!   no business there. `/system/bin`, `/system/ssl` and `/system/trust` are
//!   refused, and the trust roots reach a C program through the `/etc`
//!   projection at [`crate::etc::CA_BUNDLE`] rather than through their source.
//! * **`/tmp` and `/dev/shm`.** World-writable by mode, so Unix permissions
//!   never separated two applications there at all. `RIVET_APP_DATA` is *the*
//!   writable directory (`SPEC-03` §3.3) and this makes that sentence true
//!   rather than advisory.
//! * **`/proc` and `/sys`, beyond one file.** See `BOOT_ID_PATH`.
//! * **`network`, still.** `ADR-015` §5.1's `network` key promises a network
//!   namespace — an application with `network = "none"` has no interfaces at
//!   all — and Landlock's network rights are not that; nothing here computes
//!   a grant for either the key or a host or a port, and the namespace itself
//!   is still unbuilt (`ADR-042` §7).
//! * **`LANDLOCK_ACCESS_NET_BIND_TCP`, as of `ADR-042` §7's amendment, is NOT
//!   here for a reason worth stating rather than assuming.** It is handled
//!   and granted to nothing by `cmd/rivet-sysd/src/landlock.rs::build`
//!   directly, unconditionally, on every confined spawn — not computed from a
//!   manifest, so it needs no [`Access`] bit and no [`Policy`] row: no
//!   application has a lawful reason to bind a TCP port at all, since the
//!   supervisor pre-binds every socket a manifest declares and hands the
//!   descriptor over — `SPEC-03` §3's listening-socket contract. A verb that
//!   is denied to *everything* regardless of what the manifest says is not a
//!   policy decision this module has anything to compute, which is why it
//!   lives in the adapter and not here. `LANDLOCK_ACCESS_NET_CONNECT_TCP`
//!   remains unhandled, for the reason directly above: deciding which hosts
//!   and ports an application may reach needs a vocabulary this manifest
//!   does not have.

use crate::package::application::Manifest;
use crate::service::ServiceName;

/// What may be done with a path — Rivet's verbs, not the kernel's.
///
/// A bitset rather than an enum because a grant is a *set*: the data directory
/// is read and write and create and delete, and naming that as one value is
/// what lets [`Grant`] be one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Access(u32);

impl Access {
    /// Nothing. The value a grant may never carry — see [`Policy::grants`].
    pub const NONE: Self = Self(0);
    /// Open a file for reading.
    pub const READ: Self = Self(1 << 0);
    /// Read a directory's entries.
    pub const LIST: Self = Self(1 << 1);
    /// Execute a program, or map a shared object executable.
    pub const EXECUTE: Self = Self(1 << 2);
    /// Open a file for writing.
    pub const WRITE: Self = Self(1 << 3);
    /// Create a regular file, a directory, a unix socket, a FIFO or a symlink.
    ///
    /// **Never a device node.** A character or block special file is how a
    /// process reaches hardware the manifest did not declare, and an
    /// application has no use for one it made itself.
    pub const CREATE: Self = Self(1 << 4);
    /// Remove a file or a directory.
    pub const DELETE: Self = Self(1 << 5);
    /// Shorten a file that is already open.
    ///
    /// Separate from [`Access::WRITE`] because the kernel separates them, and
    /// on a kernel too old to know the difference it is the one right whose
    /// absence makes the grant **wider** rather than narrower.
    pub const TRUNCATE: Self = Self(1 << 6);
    /// Link or rename a file into a different directory of the same grant.
    ///
    /// Both ends must carry it, so a grant that has it can still only move a
    /// file **within itself**.
    pub const REPARENT: Self = Self(1 << 7);
    /// `ioctl` on a device node.
    pub const DEVICE_CONTROL: Self = Self(1 << 8);

    /// The union of two sets.
    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether every verb in `other` is in this set.
    #[must_use]
    pub const fn holds(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The raw bits, for an adapter that must map them.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether this set is empty.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Every verb, in a stable order, with the word Rivet uses for it.
    ///
    /// For the journal: an operator reading *"what was this application
    /// given"* needs the answer in words, and a hexadecimal mask is not one.
    #[must_use]
    pub fn words(self) -> Vec<&'static str> {
        const TABLE: &[(Access, &str)] = &[
            (Access::READ, "read"),
            (Access::LIST, "list"),
            (Access::EXECUTE, "execute"),
            (Access::WRITE, "write"),
            (Access::CREATE, "create"),
            (Access::DELETE, "delete"),
            (Access::TRUNCATE, "truncate"),
            (Access::REPARENT, "reparent"),
            (Access::DEVICE_CONTROL, "device-control"),
        ];
        TABLE
            .iter()
            .filter(|(bit, _)| self.holds(*bit))
            .map(|(_, word)| *word)
            .collect()
    }
}

/// Whether a path that is not there is a fault or an ordinary machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// **The application cannot run without it and the spawn fails.** Its own
    /// installed files, its own data directory, and every library its manifest
    /// declared: each of those is something the machine promised, so a missing
    /// one is a broken install and not a configuration.
    Required,
    /// **Absent is an answer.** A device node for hardware that is not fitted,
    /// `/lib` on an image with nothing dynamic in it, `/dev/null` under an
    /// emulator with no devtmpfs. The grant is dropped, which leaves the
    /// application *more* confined rather than less, so the failure direction
    /// is the safe one.
    Optional,
}

/// One row: a directory or a file, and what may be done inside or with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    /// An absolute path. A directory grant reaches everything beneath it.
    pub path: String,
    /// What is permitted there.
    pub access: Access,
    /// What a missing path means.
    pub presence: Presence,
    /// Why this row is here, in one clause, for the journal.
    ///
    /// **Carried rather than derived** so that the sentence an operator reads
    /// is written beside the row it describes; a lookup table somewhere else
    /// is the second representation `ADR-011` is about.
    pub because: &'static str,
}

/// Everything one application may reach.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Policy {
    /// In the order [`policy`] writes them, which is the order the journal
    /// reports and the order the rules are added in. **No row is ever
    /// [`Access::NONE`]** — a row granting nothing is a row that would read as
    /// a grant and confine nothing.
    pub grants: Vec<Grant>,
}

impl Policy {
    /// Add a row, dropping it if it grants nothing.
    pub fn allow(&mut self, path: String, access: Access, presence: Presence, because: &'static str) {
        if access.is_empty() {
            return;
        }
        self.grants.push(Grant {
            path,
            access,
            presence,
            because,
        });
    }
}

/// The boot id the SDK stamps on every journal record — `SPEC-03` §3.6.
///
/// **The one file under `/proc` any policy grants, and it is granted because
/// the platform's own client library reads it.** `rivet-sdk`'s
/// `journal::boot_id` and `template-go-service`'s `rivet.go` both open this
/// exact path; a record whose `boot` field is missing becomes a `warn` record
/// carrying the raw bytes (`SPEC-03` §3.6), so denying it would turn every
/// application's log line into a complaint about itself while the log still
/// looked busy.
///
/// Granting the **file** rather than `/proc/sys` or `/proc` is the whole point:
/// a directory grant here would hand over every other process's command line.
pub const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";

/// **The programs that may look at the supervisor's own `/proc` directory,
/// named here rather than asked for** — `ADR-056` §3, widened by `ADR-057`.
///
/// `ADR-056` §3 left the maintainer two ways to select it: a new manifest key,
/// or a name in the platform. **The name is taken**, and the reason is the whole
/// purpose of the instrument. A manifest key would make this a capability any
/// application could request, and `ADR-056` §3 exists to stop the page becoming
/// something anything else reaches for; a capability that can be asked for is one
/// that will be. This way the answer to *who may watch the supervisor* is a
/// constant in the platform, greppable, and changed only by changing this list.
///
/// **A list rather than one name since 2026-09-19, and the shape of the grant
/// did not change with it.** `ADR-057` gives the same two leaves to a screen for
/// the same reason the portal has them: an instrument may not depend on the
/// thing it measures, so the one question the supervisor cannot be trusted to
/// answer about itself is read from `/proc` by whatever is drawing it. What a
/// second entry does **not** do is widen anybody's window — each named program
/// gets exactly `supervisor_proc_dir` and [`UPTIME_PATH`], and every other
/// application on the machine still gets neither.
///
/// It is ugly on purpose. A general mechanism here would be worse, and a list
/// that has to be edited by hand is a list somebody has to think about.
pub const SUPERVISOR_WINDOWS: &[&str] = &[
    // `ADR-056` §3: the portal's button, which is what a person has when the
    // control plane has already gone quiet.
    "rivet-fleetd",
    // `ADR-057`: the kiosk dashboard, which is the same instrument on glass in
    // a room somebody walks through.
    "rivet-dashboard",
    // `ADR-057` §7: the first article. A platform surface nothing confined has
    // ever read is a surface nobody has tested — `ADR-053` shipped unit-tested
    // and the first real machine refused it — so the probe that reads this one
    // on a board is named here beside the programs it is proving it for.
    "facts-probe",
];

/// Whether this program is one of [`SUPERVISOR_WINDOWS`].
///
/// A function rather than callers writing the `contains`, so that *who may
/// watch the supervisor* has one answer and adding a name never means finding
/// every place that asked.
#[must_use]
pub fn watches_the_supervisor(name: &ServiceName) -> bool {
    SUPERVISOR_WINDOWS.contains(&name.as_str())
}

/// Seconds since boot, and the interval every cumulative counter is read
/// against — `ADR-056` §3.
///
/// **A file grant, not a directory**, for `BOOT_ID_PATH`'s reason exactly. It
/// carries nothing about any process, which is why it can be granted without
/// being an argument about one.
pub const UPTIME_PATH: &str = "/proc/uptime";

/// The supervisor's own subtree of `/proc`, for [`SUPERVISOR_WINDOWS`].
///
/// **One process's directory, never `/proc` itself.** The directory above this
/// is every other process's command line, environment and descriptors; this one
/// is the supervisor's, which is the process whose state the instrument exists
/// to read. The grant carries [`Access::READ`] and **not** [`Access::LIST`]:
/// `ADR-056` §3 names every file it opens, so entries never need enumerating,
/// and a `LIST` here would hand over the supervisor's open descriptors and
/// threads as a browsable tree.
#[must_use]
pub fn supervisor_proc_dir(pid: u32) -> String {
    format!("/proc/{pid}")
}

/// Where the ELF interpreter and the image's shared objects are.
///
/// **Implicit, and no manifest key describes it.** `SPEC-02`'s declared closure
/// is content-addressed and lives under `<volume>/lib`; this is the other
/// directory, the one a `NEEDED` entry resolves against with no help from
/// anybody — `ld-musl-aarch64.so.1` and the Mesa and DRM objects a kiosk
/// application links. A dynamically linked program denied this does not fail
/// at the library it wanted; it fails before its first instruction, with the
/// loader unable to map itself.
pub const LOADER_DIR: &str = "/lib";

/// The read-only `/etc` projection — `ADR-007` §3.5.
///
/// **§3.5 and not §3.5.1**, which twenty other citations in this tree get
/// wrong: `docs/index.json` has no such subsection, so `claims.py` cannot
/// resolve any of them. Corrected here rather than joining them.
///
/// Implicit for the same reason as [`LOADER_DIR`]: musl resolves `/etc/hosts`
/// and `/etc/resolv.conf`, reads `/etc/localtime` for the wall clock
/// (`ADR-021` §9, which is precisely why `TZ` is left unset), and looks up an
/// account in `/etc/passwd`. It is a projection remounted read-only, so a read
/// grant on it is the whole of what any program could do with it anyway.
pub const ETC_DIR: &str = "/etc";

/// Where the machine puts what it hands an application — `SPEC-03` §3.4, §3.6,
/// §3.9.
///
/// The journal's ingest socket, the `sd_notify` socket, the granted peer
/// sockets and `display-hours` are all under here, and every one of them is a
/// path the supervisor **told** the application about. Read and write, and
/// deliberately no create and no delete: an application may talk on what it
/// was given and may not leave a socket of its own where another one is
/// looking for a peer.
pub const RUNTIME_DIR: &str = "/run/rivet";

/// **The image's shared typefaces** — [`crate::etc::SYSTEM_FONT_DIR`], and the
/// only part of `/system` any policy grants.
///
/// Added 2026-09-05 after the first confined boot took the maintainer's wall
/// display down. `ADR-042` §8 is the argument and it is short: a font is not
/// something an application goes and takes, it is **image content the machine
/// provides**, on exactly the terms [`ETC_DIR`] and [`LOADER_DIR`] are provided
/// on. `rivet-sdk`'s `text::SYSTEM_FONT` is a published constant of this
/// platform's own client library — the SDK *hands* it over, and a policy that
/// then refuses it is the platform contradicting itself rather than an
/// application misbehaving.
///
/// **Read and list, never execute and never write.** The slot is mounted
/// read-only, so the write half is already true; saying it here is what makes
/// it true of the *policy* rather than of this machine's mount options.
///
/// [`Presence::Optional`]: a card can be built without faces, and an image with
/// no type is a machine that draws no text rather than one that starts nothing.
/// The alternative — [`Presence::Required`] — would make every application on
/// such a card refuse to launch, which is a worse answer to a cosmetic fault.
///
/// # Why the directory and not the three files
///
/// A file grant per face is narrower by exactly nothing: all three faces are in
/// this directory and there is nothing else in it. What it would cost is a
/// fourth face becoming a silent `EACCES` on a board — the failure this whole
/// row exists to repair, reintroduced one file along.
/// `crate::etc`'s `the_font_directory_holds_every_system_face` is the guard on
/// the other side of that trade.
pub const FONT_DIR: &str = crate::etc::SYSTEM_FONT_DIR;

/// **Mesa's GBM backend**, which `libgbm.so.1` `dlopen`s and which no closure
/// walker can see.
///
/// Added 2026-09-20 after `examples/probes/gpu-flip` — the first application to
/// ask this platform for the GPU's own scanout buffer — was refused on a board
/// with:
///
/// ```text
/// MESA-LOADER: failed to open dri: Error loading shared library
///   /usr/lib/gbm/dri_gbm.so: Permission denied (search paths /usr/lib/gbm)
/// gpu-flip: gbm_create_device returned NULL
/// ```
///
/// **`Permission denied`, not `No such file`**, which is the whole diagnosis:
/// `tools/card/build.sh`'s `collect_gbm_backends` already stages this object,
/// having previously produced a card whose eight libraries were all present
/// and whose `gbm_create_device` returned NULL for exactly the reason a dlopen
/// is not a `NEEDED` entry. The file was there. The policy refused it.
///
/// # The argument is [`FONT_DIR`]'s, one library along
///
/// `libgbm.so.1` is in [`crate::package::library::PUBLIC_LIBRARIES`] — **the
/// platform promises it to every application**, exactly as it promises
/// `libdrm.so.2` and the typefaces. But libgbm resolves its backend from a
/// path compiled into it and looks nowhere else (`strings libgbm.so.1` is
/// `build.sh`'s own evidence), so a promise of the library without its backend
/// is a promise of a library that returns NULL. That is the platform
/// contradicting itself rather than an application misbehaving, which is the
/// sentence `FONT_DIR` was added under.
///
/// **Read, list AND execute**, unlike every other implicit grant here. A
/// shared object is loaded, not read, and Landlock distinguishes the two; a
/// read-only grant would produce the identical `EACCES` and look like this
/// repair having failed.
///
/// [`Presence::Optional`]: a card can be built without Mesa, and one that is
/// should start every application that does not want a GPU rather than none of
/// them.
///
/// # Why the directory and not the file
///
/// `FONT_DIR`'s trade, unchanged: the directory holds one object today and a
/// file grant would be narrower by nothing, while a second backend — a
/// different Gallium driver on different silicon — would become a silent
/// `EACCES` on a board. That is the failure this row exists to end.
pub const GBM_BACKEND_DIR: &str = "/usr/lib/gbm";

/// **Where a DRM descriptor is turned back into a device**, and the second
/// thing Mesa needs that no manifest names.
///
/// With [`GBM_BACKEND_DIR`] granted, `gpu-flip` got past `Permission denied`
/// on 2026-09-20 and straight into:
///
/// ```text
/// MESA-LOADER: failed to retrieve device information
/// gpu-flip: gbm_create_device returned NULL
/// ```
///
/// Mesa's loader identifies a device by `fstat`ing the descriptor for its
/// major and minor and then reading **sysfs** — `/sys/dev/char/<major>:<minor>`
/// is a symlink into `/sys/devices`, and libdrm's `drmGetDevice2` walks it to
/// learn what the hardware is. This module's own header refuses `/sys` *"beyond
/// one file"*, so the walk ended in `EACCES` and Mesa reported the consequence
/// rather than the cause.
///
/// **Both halves are needed and one is not enough.** Landlock resolves a
/// symlink to its target, so granting the `char` directory without the
/// `devices` tree grants the pointer and not the thing pointed at.
///
/// # Granted only to an application that declared `display` or `gpu`
///
/// Unlike every other constant in this file these are **conditional**, and
/// that is `.rulings/` R-204's rule applied the first time it came up: the
/// platform keeps to the application's contract and hands over only what was
/// asked for. An application that asked for neither a screen nor the GPU has no
/// business reading the hardware tree, and handing it one unasked is the shape
/// that ruling rejected.
///
/// **`gpu` was added to the condition on 2026-09-20** (`.rulings/` R-206 D4).
/// These are **Mesa's** grants and never the screen's — they are what
/// `gbm_create_device` needs to turn a DRM descriptor back into a device — and
/// they were keyed on `display` alone only because `display` used to be the
/// sole route to a render node. `ADR-058` separated those, so the condition now
/// names the thing that actually needs them rather than the thing that used to
/// imply it.
///
/// **Read and list, never write and never execute.** Nothing here is loaded and
/// nothing here is a secret; it is the kernel's description of the hardware
/// this machine already is.
pub const DRM_SYSFS_CHAR_DIR: &str = "/sys/dev/char";
/// The tree [`DRM_SYSFS_CHAR_DIR`]'s symlinks resolve into — see that constant.
pub const DRM_SYSFS_DEVICES_DIR: &str = "/sys/devices";
/// The other directory Mesa enumerates DRM devices through, on the same terms
/// as [`DRM_SYSFS_CHAR_DIR`] and into the same tree.
pub const DRM_SYSFS_CLASS_DIR: &str = "/sys/class/drm";

/// **The DRM node directory — LIST, and never READ.**
///
/// Mesa enumerates `/dev/dri` to find out what this machine has. With the
/// directory unlistable, `eglInitialize` answered `EGL_NOT_INITIALIZED`
/// (`0x3001`), which carries no cause at all — `gpu-flip`'s reachability
/// report is what turned that into a path.
///
/// **The distinction between LIST and READ is doing real work here**, and it
/// is why this grant is safe. `cmd/rivet-sysd`'s `landlock::bits_of` maps
/// [`Access::LIST`] to `FS_READ_DIR` and [`Access::READ`] to `FS_READ_FILE`:
/// listing a directory is not permission to open what is inside it. So an
/// application may learn that `card0` exists and still be refused when it
/// tries to open it — which keeps `ADR-015` §6's arbitration exactly where
/// it was. **A `READ` here would hand every kiosk application the arbiter's
/// card node**, and that is the one mistake this row must not make.
pub const DRM_NODE_DIR: &str = "/dev/dri";

/// The character devices a program may reach without declaring anything.
///
/// **Not a device class** — `ADR-015` §6.2's classes are the manifest's, and
/// these are not in it. They are here because a language runtime opens them
/// with no manifest to consult: `/dev/null` is where a spawned process's
/// output goes and what `Stdio::null()` opens, `/dev/zero` backs an anonymous
/// mapping in some allocators, and `/dev/urandom` is `getrandom(2)`'s fallback
/// on a kernel that refuses the syscall.
///
/// Each is [`Presence::Optional`], and the reason is measured rather than
/// theoretical: under `tools/qemu/run.sh` the root is an initramfs with no
/// devtmpfs and **there is no `/dev/null` at all** (`AGENTS.md` §10.5, three
/// defects). A policy that required one would refuse to start every
/// application in the emulator.
pub const PLAIN_DEVICES: &[&str] = &["/dev/null", "/dev/zero", "/dev/urandom", "/dev/random"];

/// **The policy for one application.** `ADR-042` §4 is this table in prose.
///
/// Pure: it reads the manifest and the two path functions, touches nothing,
/// and returns the same answer on any machine. Whether each path *exists* is
/// the adapter's question, and [`Presence`] is how this says what the answer
/// means.
///
/// # What it does not include
///
/// The nodes a `devices` class expands to. Those come from
/// `crate::service::device_globs` and a **glob against the machine**, so they
/// are the supervisor's to append — `cmd/rivet-sysd`'s `crate::devices` walks
/// them once, for the chown, and the same list is added to this policy rather
/// than expanded a second time (`ADR-011`).
///
/// # `supervisor`
///
/// The pid of the process composing this policy, when that process **is** the
/// supervisor. It is the one input that does not come from the manifest, and it
/// is a parameter rather than a `std::process::id()` call inside so that this
/// function stays pure and every test can state the pid it means.
///
/// `None` says *composed by something that is not a supervisor* — a tool, a
/// test, `cargo rivet` reading a package — and the only thing it changes is that
/// a [`SUPERVISOR_WINDOWS`] program gets no window. Every other row is
/// identical, which is what keeps a policy printed by a tool an honest preview
/// of the one the machine will build.
#[must_use]
pub fn policy(
    volume: &str,
    name: &ServiceName,
    manifest: &Manifest,
    supervisor: Option<u32>,
    addons: &[crate::service::InstalledLibrary],
) -> Policy {
    let mut out = Policy::default();

    // Its own installed files, and the program itself: `.../current/bin/<name>`
    // is inside this directory, so EXECUTE here is what lets the `execve` that
    // follows the ruleset succeed at all. Read-only, because `ADR-015` §7 makes
    // an install an unpack-and-swap and a program that could rewrite its own
    // version directory would be a program that installs itself.
    out.allow(
        crate::service::current_dir(volume, name),
        Access::READ.with(Access::LIST).with(Access::EXECUTE),
        Presence::Required,
        "RIVET_APP_DIR: its own installed files, and the program the supervisor is about to exec",
    );

    home_rows(&mut out, volume, name);

    // The declared closure — the same list `LD_LIBRARY_PATH` is composed from,
    // through the same function, so the loader's search path and the
    // confinement cannot disagree.
    for reference in manifest.library() {
        out.allow(
            crate::package::install::library_object_dir(volume, reference),
            Access::READ.with(Access::LIST).with(Access::EXECUTE),
            Presence::Required,
            "a library this manifest declared: the same directory LD_LIBRARY_PATH names",
        );
    }

    // **And the class-matched ones, which is the other half of that sentence.**
    //
    // `ADR-053` puts every installed library whose class this manifest accepts
    // onto `LD_LIBRARY_PATH`, and until 2026-09-19 it put nothing into this
    // ruleset — so the loader searched a directory the kernel then refused.
    // `examples/probes/addon-probe-host` found it on `rivet-06f5c` the first
    // time an addon was ever discovered on a real machine: a correct path, a
    // real object, and `dlopen` reporting it was not there.
    //
    // **Optional rather than Required**, which is where this differs from the
    // declared closure above. A declared reference is part of what the
    // application IS and its absence is a refusal at install; a class-matched
    // addon is something that happened to be installed beside it, and one that
    // is removed between a discovery pass and the next spawn must not stop the
    // host starting. `ADR-053` §2's *nothing joins by being present* has a
    // mirror: nothing fails by being absent.
    for dir in crate::service::addon_library_dirs(volume, manifest, addons) {
        out.allow(
            dir,
            Access::READ.with(Access::LIST).with(Access::EXECUTE),
            Presence::Optional,
            "an addon this manifest accepts by class: the same directory LD_LIBRARY_PATH names",
        );
    }

    platform_rows(&mut out, name, supervisor, talks_to_mesa(manifest.devices()));

    out
}

/// **Whether a program gets Mesa's device lookup** — see
/// [`DRM_SYSFS_CHAR_DIR`]. One answer for an application's manifest and an
/// image service's definition, so the two kinds of confined program cannot
/// come to disagree about which declaration earns the hardware tree.
fn talks_to_mesa(devices: &[crate::package::application::Device]) -> bool {
    devices.iter().any(|d| {
        matches!(
            d,
            crate::package::application::Device::Display
                | crate::package::application::Device::Gpu
        )
    })
}

/// **The policy for a service the IMAGE defines and runs under its own
/// identity** — [`crate::service::IMAGE_SERVICE_HOMES`], `ADR-060`.
///
/// An image service has no manifest, so [`policy`] cannot be asked. What it
/// does have is the same home an application has — `data`, `tmp`, the
/// settings file and the routing table under `<volume>/app/<name>` — and the
/// same platform the machine hands every confined program. **Both of those are
/// the same functions [`policy`] calls**, so a row added for applications is a
/// row added here, and the two cannot drift into two answers to *what may a
/// confined program on this machine reach*.
///
/// What differs is one row. An application's program is inside its installed
/// version directory, so [`policy`] grants `current`; an image service's program
/// is one file on the read-only system slot, so this grants **that file** —
/// read and execute, and nothing else under `/system/bin`, which is why the
/// grant is the file and not its directory. Required: the supervisor is about
/// to `execve` it, and a ruleset without it would refuse the program it exists
/// to confine.
///
/// It takes no manifest and so declares no library and accepts no addon.
///
/// # `devices`
///
/// The device classes the service's definition declares — the same field an
/// application's manifest carries, read the same way. A service declaring
/// `display` or `gpu` gets Mesa's device lookup exactly as an application
/// declaring it would ([`DRM_SYSFS_CHAR_DIR`]); the compositor is the one that
/// does, because it renders. The device NODES are not here for either kind of
/// program: the supervisor expands the classes against the machine for the
/// chown and appends the same list (see [`policy`]).
#[must_use]
pub fn image_service_policy(
    program: &str,
    volume: &str,
    name: &ServiceName,
    supervisor: Option<u32>,
    devices: &[crate::package::application::Device],
) -> Policy {
    let mut out = Policy::default();
    out.allow(
        program.to_string(),
        Access::READ.with(Access::EXECUTE),
        Presence::Required,
        "the program the supervisor is about to exec, a single file on the read-only system slot",
    );
    home_rows(&mut out, volume, name);
    platform_rows(&mut out, name, supervisor, talks_to_mesa(devices));
    out
}

/// The rows for the four paths under `<volume>/app/<name>` the machine makes
/// and then tells the program about — shared by [`policy`] and
/// [`image_service_policy`], so both kinds of confined program get the same
/// answer for the same home.
fn home_rows(out: &mut Policy, volume: &str, name: &ServiceName) {
    // The one writable directory (`SPEC-03` §3.3). Everything except EXECUTE,
    // and the exception is the point: application volumes are mounted `exec`
    // (`ADR-031` §5), so before this a program could write a binary here and
    // run it. `PR_SET_NO_NEW_PRIVS` stopped that binary gaining privilege; this
    // stops it being run.
    out.allow(
        crate::service::data_dir(volume, name),
        Access::READ
            .with(Access::LIST)
            .with(Access::WRITE)
            .with(Access::CREATE)
            .with(Access::DELETE)
            .with(Access::TRUNCATE)
            .with(Access::REPARENT),
        Presence::Required,
        "RIVET_APP_DATA: the one directory it may write, and NOT one it may execute from",
    );

    // **The scratch directory** — `RIVET_APP_TMP` and `TMPDIR`,
    // `crate::service::tmp_dir`, ruled 2026-09-07.
    //
    // Composed here rather than left to the application for the reason the two
    // rows above exist: the machine MAKES this directory and then TELLS the
    // application about it, so the grant and the environment come from one set
    // of functions (`ADR-011`). Before it, a confined application had exactly
    // one writable path and every convention for a scratch file pointed
    // somewhere the ruleset refuses — which surfaced as `SQLITE_CANTOPEN` from
    // a restore and named no path at all.
    //
    // **The same access as `data`, and NOT EXECUTE, for the same reason**:
    // application volumes are mounted `exec` (`ADR-031` §5), and a writable
    // directory a program could also run from is a way back in.
    out.allow(
        crate::service::tmp_dir(volume, name),
        Access::READ
            .with(Access::LIST)
            .with(Access::WRITE)
            .with(Access::CREATE)
            .with(Access::DELETE)
            .with(Access::TRUNCATE)
            .with(Access::REPARENT),
        Presence::Required,
        "RIVET_APP_TMP: where it may put a scratch file, emptied by the supervisor at every \
         spawn and every exit",
    );

    // **The settings file the supervisor hands it** — `RIVET_APP_SETTINGS`,
    // `crate::settings::path`, `ADR-038` §8.1.
    //
    // A row on the same footing as the two above and composed by the same
    // module, because it is the same kind of thing: a path the machine WROTE
    // and then TOLD the application about. It is a sibling of `current` and
    // `data` rather than inside either (`crate::settings::path` has why), so
    // neither grant above reaches it — which is how it came to be missing.
    //
    // **Read, and nothing else.** `ADR-007` §3.6.2: a view that can be written
    // and have effect is not a view. The file is `0644 root:root` so the uid
    // could not write it anyway; the policy says so as well, and the policy is
    // the half that is still true if the mode is ever got wrong.
    //
    // Optional: an application whose document names no `[applications.<name>]`
    // table is handed no file, and that is an ordinary machine.
    out.allow(
        crate::settings::path(volume, name),
        Access::READ,
        Presence::Optional,
        "RIVET_APP_SETTINGS: the settings file the supervisor wrote for it and named in its \
         environment",
    );

    // **The routing table** — `crate::proxy::path`, `ADR-046` §5.
    //
    // **The same defect as the row above, found the same way and one file
    // later.** `crate::proxy::path` documents itself as *root-owned,
    // application-readable* and is written `0644`, so the mode was never the
    // problem; it is a sibling of `current` and `data`, so neither grant
    // reaches it, and `rivet-proxyd` failed five times on a board with
    // *"the routing table … could not be read: Permission denied"*. The
    // comment above says a sibling path is *"how it came to be missing"* — it
    // said so about `settings` while `routes` was missing beside it.
    //
    // **Read, and nothing else**, which is the whole reason the table is here
    // rather than in `data/`: a proxy that could rewrite its own routes could
    // route itself somewhere it was not sent. The mode says so and the policy
    // says so, and the policy is the half still true if the mode is got wrong.
    //
    // **Optional, and for every application rather than the proxy alone.** The
    // supervisor writes exactly one table, into the proxy's directory, so every
    // other application simply has no such file — and a policy that named one
    // service would be confinement special-casing a program by name, which is
    // the thing `ADR-015` §5.1 refuses for identities and refuses here too.
    out.allow(
        crate::proxy::path(volume, name),
        Access::READ,
        Presence::Optional,
        "the routing table, for the application that serves it",
    );

    // **The name service's projection of the document** —
    // `crate::name_service::path`, `ADR-047` §6.
    //
    // The routing table's footing exactly: a root-owned file the supervisor
    // writes beside `settings`, read-only so the service cannot rewrite the
    // zone it serves, and Optional and for every home rather than for one
    // service named here. Only `rivet-dnsd`'s home ever holds one.
    out.allow(
        crate::name_service::path(volume, name),
        Access::READ,
        Presence::Optional,
        "the name service's projection of the document: version, hostname and [dns], for the \
         service that answers for them",
    );

    // **The mDNS responder's projection** — `crate::mdns::path`, `ADR-017`
    // §3. The name service's footing exactly, and for its reasons: read
    // only, so the responder cannot rewrite what it announces, and Optional
    // for every home. Only `rivet-mdnsd`'s home ever holds one.
    out.allow(
        crate::mdns::path(volume, name),
        Access::READ,
        Presence::Optional,
        "the mDNS responder's projection: the machine's name and its interfaces' addresses, for \
         the service that announces them",
    );
}

/// The rows every confined program gets from the platform rather than from a
/// declaration — shared by [`policy`] and [`image_service_policy`].
///
/// `mesa` is the one condition in it: whether the program declared `display`
/// or `gpu` — [`talks_to_mesa`].
fn platform_rows(out: &mut Policy, name: &ServiceName, supervisor: Option<u32>, mesa: bool) {
    out.allow(
        LOADER_DIR.to_string(),
        Access::READ.with(Access::LIST).with(Access::EXECUTE),
        Presence::Optional,
        "implicit: the ELF interpreter and the image's shared objects, which no manifest key names",
    );
    out.allow(
        ETC_DIR.to_string(),
        Access::READ.with(Access::LIST),
        Presence::Optional,
        "implicit: the read-only /etc projection every libc resolves against",
    );
    out.allow(
        FONT_DIR.to_string(),
        Access::READ.with(Access::LIST),
        Presence::Optional,
        "implicit: the image's shared typefaces, which rivet-sdk's text module hands to every \
         application that draws",
    );
    out.allow(
        GBM_BACKEND_DIR.to_string(),
        // **EXECUTE, unlike its neighbours.** A backend is dlopened, and
        // Landlock treats loading an object as execution rather than reading.
        Access::READ.with(Access::LIST).with(Access::EXECUTE),
        Presence::Optional,
        "implicit: the backend libgbm.so.1 dlopens from a path compiled into it, without which \
         the libgbm this platform promises returns NULL",
    );
    // **Conditional, and the only conditional grant in this function** — see
    // `DRM_SYSFS_CHAR_DIR`. An application that asked for neither a screen nor
    // the GPU does not get the hardware tree.
    //
    // **`gpu` joins `display` here, 2026-09-20** (`.rulings/` R-206 D4). These
    // five grants are Mesa's, not the screen's: they are what `gbm_create_
    // device` needs to turn a DRM descriptor back into a device, and they were
    // keyed on `display` only because `display` used to be the one way to get a
    // render node. `ADR-058` split those apart — an application can now want
    // the V3D and not the panel — so the condition follows the thing that
    // actually needs them.
    if mesa {
        for path in [DRM_SYSFS_CHAR_DIR, DRM_SYSFS_DEVICES_DIR, DRM_SYSFS_CLASS_DIR] {
            out.allow(
                path.to_string(),
                Access::READ.with(Access::LIST),
                Presence::Optional,
                "declared `display` or `gpu`: where Mesa turns a DRM descriptor back into a \
                 device, without which gbm_create_device returns NULL",
            );
        }
        // **LIST and deliberately NOT READ** — see `DRM_NODE_DIR`. Enumerating
        // this directory must not become permission to open what is in it.
        out.allow(
            DRM_NODE_DIR.to_string(),
            Access::LIST,
            Presence::Optional,
            "declared `display` or `gpu`: Mesa enumerates the DRM nodes. LIST only — which node \
             this application may OPEN is device_globs' answer and stays its answer",
        );
    }
    out.allow(
        RUNTIME_DIR.to_string(),
        Access::READ.with(Access::LIST).with(Access::WRITE),
        Presence::Optional,
        "the sockets and files the supervisor hands it: the journal, sd_notify, its granted peers",
    );
    out.allow(
        BOOT_ID_PATH.to_string(),
        Access::READ,
        Presence::Optional,
        "implicit: the boot id the SDK stamps on every journal record",
    );
    // **The supervisor's own `/proc`, for the programs that watch it** —
    // `ADR-056` §3, and [`SUPERVISOR_WINDOWS`] says why they are names here
    // and not a manifest key.
    //
    // `Presence::Optional` on both rows, and the direction of that choice is the
    // safe one: on a machine with no `/proc` mounted, or composed by something
    // that is not a supervisor, the rows are dropped and `rivet-fleetd` is left
    // *more* confined. The page then reports that it cannot read the supervisor,
    // which `ADR-056` §3 already requires it to be able to say.
    if watches_the_supervisor(name) {
        if let Some(pid) = supervisor {
            out.allow(
                supervisor_proc_dir(pid),
                Access::READ,
                Presence::Optional,
                "ADR-056: the supervisor's own /proc, read-only and not listable, so this \
                 program can say whether the supervisor has stopped",
            );
            out.allow(
                UPTIME_PATH.to_string(),
                Access::READ,
                Presence::Optional,
                "ADR-056: seconds since boot, the interval the supervisor's cumulative \
                 counters are worthless without",
            );
        }
    }
    for node in PLAIN_DEVICES {
        out.allow(
            (*node).to_string(),
            Access::READ.with(Access::WRITE),
            Presence::Optional,
            "implicit: a plain character device a language runtime opens with no manifest to ask",
        );
    }
}

/// What a device node granted by `devices` may have done to it.
///
/// Kept beside [`policy`] rather than in the supervisor so that the whole
/// answer to *"what may this application reach"* is one module. `ioctl` is in
/// it because a DRM render node is driven by nothing else.
#[must_use]
pub const fn device_access() -> Access {
    Access::READ
        .with(Access::WRITE)
        .with(Access::DEVICE_CONTROL)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;
    use crate::package::application::{Manifest, ProgramKind};

    fn name(text: &str) -> ServiceName {
        ServiceName::new(text).expect("a service name")
    }

    fn plain() -> Manifest {
        Manifest::new(ProgramKind::Daemon)
    }

    /// **Enumerating `/dev/dri` must never become permission to open what is
    /// in it**, or every kiosk application would hold the arbiter's card node
    /// and `ADR-015` §6 would be arbitrating nothing.
    ///
    /// The grant exists because Mesa scans that directory and `eglInitialize`
    /// answered `EGL_NOT_INITIALIZED` without it. The distinction it rests on
    /// is `landlock::bits_of`'s: `LIST` is `FS_READ_DIR`, `READ` is
    /// `FS_READ_FILE`. This test is the guard on that sentence staying true.
    #[test]
    fn a_display_application_may_list_the_drm_nodes_and_never_read_them() {
        let manifest = plain().with_devices(vec![crate::package::application::Device::Display]);
        let p = policy("/state", &name("photo-index"), &manifest, None, &[]);
        let grant = p
            .grants
            .iter()
            .find(|g| g.path == DRM_NODE_DIR)
            .expect("a display application is granted the DRM node directory");
        assert!(grant.access.holds(Access::LIST), "it must be listable");
        assert!(
            !grant.access.holds(Access::READ),
            "READ on {DRM_NODE_DIR} would hand this application /dev/dri/card0, which is the \
             arbiter's and the one thing this grant must not do"
        );
        assert!(!grant.access.holds(Access::WRITE));
        assert!(!grant.access.holds(Access::EXECUTE));
    }

    /// The hardware tree is for applications that asked for a screen. One that
    /// did not gets none of it — `.rulings/` R-204: the platform keeps to the
    /// application's contract.
    #[test]
    fn an_application_that_declared_no_display_gets_no_hardware_tree() {
        let p = policy("/state", &name("photo-index"), &plain(), None, &[]);
        for path in [
            DRM_NODE_DIR,
            DRM_SYSFS_CHAR_DIR,
            DRM_SYSFS_DEVICES_DIR,
            DRM_SYSFS_CLASS_DIR,
        ] {
            assert!(
                !p.grants.iter().any(|g| g.path == path),
                "{path} was granted to an application that never asked for a screen"
            );
        }
    }

    #[test]
    fn the_two_directories_the_machine_composes_are_always_granted() {
        let p = policy("/state", &name("photo-index"), &plain(), None, &[]);
        let dir = p
            .grants
            .iter()
            .find(|g| g.path == "/state/app/photo-index/current")
            .expect("RIVET_APP_DIR is granted");
        let data = p
            .grants
            .iter()
            .find(|g| g.path == "/state/app/photo-index/data")
            .expect("RIVET_APP_DATA is granted");
        assert_eq!(dir.presence, Presence::Required);
        assert_eq!(data.presence, Presence::Required);
    }

    /// The property the whole change is for, stated as a test: an application
    /// may not run a program out of the directory it can write.
    #[test]
    fn the_writable_directory_is_never_executable() {
        let p = policy("/state", &name("photo-index"), &plain(), None, &[]);
        let data = p
            .grants
            .iter()
            .find(|g| g.path == "/state/app/photo-index/data")
            .expect("RIVET_APP_DATA is granted");
        assert!(data.access.holds(Access::WRITE));
        assert!(
            !data.access.holds(Access::EXECUTE),
            "RIVET_APP_DATA is mounted exec (ADR-031 §5); granting EXECUTE here would put back \
             exactly what no_new_privs was set to stop"
        );
    }

    /// And the mirror of it, because a policy that granted write nowhere would
    /// pass the test above and break every application.
    #[test]
    fn the_installed_directory_is_never_writable() {
        let p = policy("/state", &name("photo-index"), &plain(), None, &[]);
        let dir = p
            .grants
            .iter()
            .find(|g| g.path == "/state/app/photo-index/current")
            .expect("RIVET_APP_DIR is granted");
        assert!(dir.access.holds(Access::EXECUTE));
        assert!(!dir.access.holds(Access::WRITE));
        assert!(!dir.access.holds(Access::CREATE));
    }

    /// `ADR-011`: the confinement and `LD_LIBRARY_PATH` must be the same list,
    /// not two lists that agree today.
    #[test]
    fn every_declared_library_is_granted_at_the_path_ld_library_path_names() {
        // The reference syntax is `SPEC-02`'s and this test is about the
        // agreement, not about parsing; a change there should fail loudly here
        // rather than silently skip.
        let reference = crate::package::library::Reference::parse(
            "libgreet-1.0.0-b3c8f1a2d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1",
        )
        .expect("a reference SPEC-02 accepts");
        let manifest = plain()
            .with_library(vec![reference])
            .expect("a one-entry closure");
        let p = policy("/state", &name("greeter"), &manifest, None, &[]);
        let expected =
            crate::service::library_path("/state", &manifest).expect("a library path is composed");
        for dir in expected.split(':') {
            let grant = p
                .grants
                .iter()
                .find(|g| g.path == dir)
                .unwrap_or_else(|| panic!("{dir} is on LD_LIBRARY_PATH and is not granted"));
            assert!(grant.access.holds(Access::EXECUTE));
            assert_eq!(grant.presence, Presence::Required);
        }
    }

    /// **The same agreement, for an addon** — and this one is a regression
    /// test with a board behind it.
    ///
    /// `ADR-053` shipped unit-tested and no library had ever been discovered
    /// on a real machine. The first time one was, on `rivet-06f5c` on
    /// 2026-09-19, the supervisor put the addon's directory on
    /// `LD_LIBRARY_PATH` and into no ruleset, and `dlopen` reported the object
    /// was not there — a correct path, a real file, and a kernel refusing it.
    #[test]
    fn a_class_matched_addon_is_granted_at_the_path_ld_library_path_names() {
        let reference = crate::package::library::Reference::parse(
            "probe-source-1.0.0-833f428e06005c6ce091386028a99967b68b9c70268a787db7fae26fea4ce849",
        )
        .expect("a reference SPEC-02 accepts");
        let installed = [crate::service::InstalledLibrary {
            reference,
            class: Some("probe-source".to_string()),
            soname: Some("libprobe-source.so.1".to_string()),
        }];
        let manifest = plain()
            .with_accepts_classes(vec!["probe-source".to_string()])
            .expect("a one-entry accepted list");

        let expected = crate::service::addon_library_path("/state", &manifest, &installed)
            .expect("the addon path is composed");
        let p = policy("/state", &name("addon-probe-host"), &manifest, None, &installed);
        for dir in expected.split(':') {
            let grant = p
                .grants
                .iter()
                .find(|g| g.path == dir)
                .unwrap_or_else(|| panic!("{dir} is on LD_LIBRARY_PATH and is not granted"));
            assert!(
                grant.access.holds(Access::EXECUTE),
                "a shared object is mapped executable"
            );
            assert!(grant.access.holds(Access::READ));
            assert_eq!(
                grant.presence,
                Presence::Optional,
                "an addon removed between discovery and spawn must not stop the host starting"
            );
        }
    }

    /// The mirror of `ADR-053` §2's *nothing joins by being present*: an
    /// installed library whose class this manifest does not accept is on
    /// neither list.
    #[test]
    fn a_library_of_another_class_is_granted_nothing() {
        let reference = crate::package::library::Reference::parse(
            "other-source-1.0.0-b3c8f1a2d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1",
        )
        .expect("a reference SPEC-02 accepts");
        let installed = [crate::service::InstalledLibrary {
            reference: reference.clone(),
            class: Some("other-source".to_string()),
            soname: Some("libother-source.so.1".to_string()),
        }];
        let manifest = plain()
            .with_accepts_classes(vec!["probe-source".to_string()])
            .expect("a one-entry accepted list");

        assert!(crate::service::addon_library_path("/state", &manifest, &installed).is_none());
        let unwanted = crate::package::install::library_object_dir("/state", &reference);
        let p = policy("/state", &name("addon-probe-host"), &manifest, None, &installed);
        assert!(
            !p.grants.iter().any(|g| g.path == unwanted),
            "a library this manifest never accepted is not in its window"
        );
    }

    /// **Every path the machine puts in the child's environment has a row.**
    ///
    /// `ADR-042` §8.1. This is the second half of the module's own rule —
    /// *handed what it needs* — and it is a test rather than a sentence because
    /// the sentence was already there on 2026-09-04 and `RIVET_APP_SETTINGS`
    /// still shipped ungranted. A variable added to `crate::service`'s
    /// environment with no row here fails this, in `rivet-core`, with no kernel
    /// and no board.
    ///
    /// `RIVET_APP_DIR` and `RIVET_APP_DATA` are checked by name above; this
    /// walks the whole set so that a *fourth* variable cannot be added without
    /// somebody answering the question.
    #[test]
    fn handed_paths_are_granted_paths() {
        let p = policy("/state", &name("photo-index"), &plain(), None, &[]);
        let handed = [
            crate::service::current_dir("/state", &name("photo-index")),
            crate::service::data_dir("/state", &name("photo-index")),
            crate::settings::path("/state", &name("photo-index")),
            // `RIVET_APP_TMP`, and `TMPDIR` set to the same value. Added
            // 2026-09-07 — the variable this tripwire was written for arrived,
            // and it arrived because the ABSENCE of one cost a restore on a
            // board and an hour of diagnosis (`service::APP_TMP_ENV`).
            crate::service::tmp_dir("/state", &name("photo-index")),
        ];
        for path in handed {
            let grant = p
                .grants
                .iter()
                .find(|g| g.path == path)
                .unwrap_or_else(|| {
                    panic!(
                        "{path} is handed to the application and is not in its policy, so the \
                         program will be told where its file is and refused it with EACCES: \
                         {:?}",
                        p.grants.iter().map(|g| &g.path).collect::<Vec<_>>()
                    )
                });
            assert!(
                grant.access.holds(Access::READ),
                "{path} is granted and cannot be read, which grants nothing"
            );
        }
    }

    /// **The scratch directory is writable and NOT executable**, and it is a
    /// sibling of `data` rather than a child of it.
    ///
    /// The executable half is `data`'s reason: application volumes are mounted
    /// `exec` (`ADR-031` §5), so a writable directory a program could also run
    /// from is a way back in. The sibling half is what makes it emptiable —
    /// `crate::service::tmp_dir` has that argument, and this asserts the
    /// layout the argument depends on rather than trusting the prose.
    #[test]
    fn the_scratch_directory_is_writable_and_not_executable() {
        let p = policy("/state", &name("photo-index"), &plain(), None, &[]);
        let path = crate::service::tmp_dir("/state", &name("photo-index"));
        let grant = p
            .grants
            .iter()
            .find(|g| g.path == path)
            .expect("the scratch directory is granted");

        for word in [
            Access::READ,
            Access::LIST,
            Access::WRITE,
            Access::CREATE,
            Access::DELETE,
        ] {
            assert!(
                grant.access.holds(word),
                "a scratch directory missing {word:?} is not one"
            );
        }
        assert!(
            !grant.access.holds(Access::EXECUTE),
            "TMPDIR is where untrusted bytes land; a program that could run them has a way back in"
        );

        // A CHILD of `data` could never be emptied without risking the history
        // beside it, which is the whole reason the sibling was chosen.
        let data = crate::service::data_dir("/state", &name("photo-index"));
        assert!(
            !path.starts_with(&data),
            "the scratch directory is inside the data directory: emptying it would put the \
             application's history one bug away from deletion"
        );
    }

    /// **The settings file is readable and NOT writable**, because it is a view
    /// (`ADR-007` §3.6.2) and the document is the authority.
    #[test]
    fn the_settings_file_is_read_only() {
        let p = policy("/state", &name("photo-index"), &plain(), None, &[]);
        let settings = p
            .grants
            .iter()
            .find(|g| g.path == "/state/app/photo-index/settings")
            .expect("RIVET_APP_SETTINGS is granted");
        assert!(settings.access.holds(Access::READ));
        assert!(!settings.access.holds(Access::WRITE));
        assert!(!settings.access.holds(Access::TRUNCATE));
        assert!(!settings.access.holds(Access::EXECUTE));
        assert_eq!(
            settings.presence,
            Presence::Optional,
            "an application whose document names no settings table is handed no file, and that \
             is an ordinary machine rather than a broken install"
        );
    }

    /// **The routing table is readable and NOT writable**, and it is granted at
    /// all — which it was not until a board said so.
    ///
    /// `rivet-proxyd` failed five times on `0.1.72` with *"the routing table
    /// /state/app/rivet-proxyd/routes could not be read: Permission denied"*.
    /// The file was `0644 root:root` and `crate::proxy::path` called itself
    /// *application-readable*; what was missing was this grant. **The mode being
    /// right is what made it invisible** — every check that looked at the file
    /// said yes, and Landlock said no.
    #[test]
    fn the_routing_table_is_readable_and_not_writable() {
        let p = policy("/state", &name("photo-index"), &plain(), None, &[]);
        let routes = p
            .grants
            .iter()
            .find(|g| g.path == "/state/app/photo-index/routes")
            .expect(
                "the routing table is not granted at all — this is the defect that failed \
                 rivet-proxyd five times on a board, and it is invisible to anything that \
                 inspects the file rather than the policy",
            );
        assert!(routes.access.holds(Access::READ));
        assert!(
            !routes.access.holds(Access::WRITE),
            "a proxy that can rewrite its own routing table can route itself somewhere it was \
             not sent, which is the whole reason the table is not in `data/`"
        );
        assert!(!routes.access.holds(Access::TRUNCATE));
        assert!(!routes.access.holds(Access::EXECUTE));
        assert_eq!(
            routes.presence,
            Presence::Optional,
            "the supervisor writes one table, into one application's directory; every other \
             application has no such file and that is not a broken install"
        );
    }


    /// **The named programs, and nothing else on the machine** — `ADR-056`
    /// §3.4, `ADR-057` §5.
    ///
    /// The instrument exists so a person can see a stopped supervisor; the whole
    /// design of `ADR-056` §3 is that nothing else should reach for it. This
    /// asserts the grant is keyed on the name and not on anything an author can
    /// write, so an application cannot acquire the window by declaring
    /// something.
    #[test]
    fn only_the_named_programs_get_a_window_on_the_supervisor() {
        let window = supervisor_proc_dir(147);

        for watching in SUPERVISOR_WINDOWS {
            let watcher = policy("/state", &name(watching), &plain(), Some(147), &[]);
            assert!(
                watcher.grants.iter().any(|g| g.path == window),
                "{watching} is one of the programs ADR-056 §3 names, and it is granted \
                 the supervisor's own /proc directory"
            );
            assert!(
                watcher.grants.iter().any(|g| g.path == UPTIME_PATH),
                "cumulative counters with no interval to read them against say nothing, so \
                 the window is worthless without /proc/uptime and comes with it"
            );
        }

        let ordinary = policy("/state", &name("photo-index"), &plain(), Some(147), &[]);
        assert!(
            !ordinary.grants.iter().any(|g| g.path == window),
            "an application that is not in SUPERVISOR_WINDOWS sees nothing of the supervisor, \
             and there is no manifest key that would change that"
        );
        assert!(
            !ordinary.grants.iter().any(|g| g.path == UPTIME_PATH),
            "/proc/uptime rides with the window and is not granted on its own"
        );
    }

    /// **Read the files it names; do not browse the directory** — `ADR-056` §3
    /// sets the grant as a leaf and names every file the page reads, so
    /// nothing needs listing.
    ///
    /// `LIST` here would hand over the supervisor's open descriptors and its
    /// threads as a browsable tree, which is a different and much larger grant
    /// wearing the same path.
    #[test]
    fn the_window_on_the_supervisor_is_read_only_and_not_listable() {
        let p = policy("/state", &name(SUPERVISOR_WINDOWS[0]), &plain(), Some(147), &[]);
        let window = p
            .grants
            .iter()
            .find(|g| g.path == supervisor_proc_dir(147))
            .expect("the window is granted");
        assert!(window.access.holds(Access::READ));
        assert!(
            !window.access.holds(Access::LIST),
            "a LIST on the supervisor's /proc directory enumerates its descriptors and \
             threads; ADR-056 3.5 names every file it opens, so it never needs to"
        );
        for refused in [
            Access::WRITE,
            Access::EXECUTE,
            Access::CREATE,
            Access::DELETE,
            Access::TRUNCATE,
        ] {
            assert!(
                !window.access.holds(refused),
                "the instrument reads and changes nothing: ADR-056 5 makes that a \
                 perturbation rule before it is a security one"
            );
        }
    }

    /// **`/proc` itself is never the grant, at any pid.**
    ///
    /// The directory above the window is every other process's command line,
    /// environment and descriptors. `BOOT_ID_PATH`'s own note says why that is
    /// the line, and this asserts the new rows did not cross it.
    #[test]
    fn the_window_is_one_process_and_never_the_directory_above_it() {
        let p = policy("/state", &name(SUPERVISOR_WINDOWS[0]), &plain(), Some(147), &[]);
        for grant in &p.grants {
            assert_ne!(grant.path, "/proc", "the whole of /proc is never granted");
            assert_ne!(grant.path, "/proc/", "the whole of /proc is never granted");
        }
        assert_eq!(supervisor_proc_dir(147), "/proc/147");
    }

    /// **No supervisor, no window** — and the failure direction is the safe one.
    ///
    /// A tool composing a policy for a preview is not a supervisor and has no
    /// pid to offer. The row is dropped, which leaves the application *more*
    /// confined, and `ADR-056` §3 already requires the page to be able to say
    /// it cannot identify a supervisor.
    #[test]
    fn a_policy_composed_by_something_that_is_not_a_supervisor_grants_no_window() {
        let p = policy("/state", &name(SUPERVISOR_WINDOWS[0]), &plain(), None, &[]);
        assert!(
            !p.grants.iter().any(|g| g.path.starts_with("/proc/1")),
            "with no supervisor pid there is no window, and every other row is unchanged"
        );
        let with = policy("/state", &name(SUPERVISOR_WINDOWS[0]), &plain(), Some(147), &[]);
        assert_eq!(
            p.grants.len() + 2,
            with.grants.len(),
            "the pid adds exactly the two rows ADR-056 3.4 names and changes nothing else, \
             which is what makes a tool's preview an honest one"
        );
    }
    /// **The fonts, and only the fonts** — `ADR-042` §8.
    ///
    /// The regression of 2026-09-05: three faces the image ships and the SDK
    /// publishes, refused with `EACCES` on a board, and a launcher with no text
    /// on the screen. The grant is a leaf of `/system` and the test asserts
    /// both halves — that it reaches the faces, and that it reaches nothing
    /// else under that slot.
    #[test]
    fn the_system_typefaces_are_granted_and_the_rest_of_system_is_not() {
        let p = policy("/state", &name("photo-index"), &plain(), None, &[]);
        let fonts = p
            .grants
            .iter()
            .find(|g| g.path == crate::etc::SYSTEM_FONT_DIR)
            .expect("the image's typefaces are granted");
        assert!(fonts.access.holds(Access::READ));
        assert!(fonts.access.holds(Access::LIST));
        assert!(
            !fonts.access.holds(Access::WRITE) && !fonts.access.holds(Access::EXECUTE),
            "the system slot is read-only and the policy says so too, so a mount that came up \
             writable would still not let an application replace this machine's typeface"
        );
        for face in [
            crate::etc::SYSTEM_FONT,
            crate::etc::SYSTEM_MONO,
            crate::etc::ICON_FONT,
        ] {
            assert!(
                face.starts_with(&format!("{}/", crate::etc::SYSTEM_FONT_DIR)),
                "{face} is outside the granted directory, so a confined application is refused it"
            );
        }
    }

    /// Absence claims, as a test. Each of these was reachable before the
    /// ruleset and the change is worth nothing if one comes back.
    ///
    /// **`/system/share` is on the list and `/system/share/fonts` is not**, and
    /// the distinction is the whole of `ADR-042` §8: the typefaces are granted
    /// and the slot they sit on is not. `/system/share/zoneinfo` is beneath the
    /// first and is therefore refused — correctly, because `TZ` is unset and
    /// every runtime on a Rivet reads the `/etc/localtime` projection instead
    /// (`crate::etc::ZONEINFO_DIR`).
    #[test]
    fn nothing_outside_the_declaration_is_granted() {
        let p = policy("/state", &name("photo-index"), &plain(), None, &[]);
        for forbidden in [
            "/",
            "/tmp",
            "/system",
            "/system/bin",
            "/system/trust",
            "/system/ssl",
            "/system/share",
            "/system/share/zoneinfo",
            "/state",
            "/state/app",
            "/state/config",
            "/state/config/machine.toml",
            "/proc",
            "/sys",
            "/dev",
            "/dev/shm",
            "/state/app/other-app/data",
            "/state/app/other-app/settings",
        ] {
            assert!(
                !p.grants.iter().any(|g| g.path == forbidden),
                "{forbidden} is granted, and nothing in the manifest asked for it"
            );
        }
    }

    #[test]
    fn a_grant_is_never_empty() {
        let p = policy("/state", &name("photo-index"), &plain(), None, &[]);
        assert!(!p.grants.is_empty());
        for grant in &p.grants {
            assert!(
                !grant.access.is_empty(),
                "{} grants nothing, which reads as a grant and confines nothing",
                grant.path
            );
            assert!(grant.path.starts_with('/'), "{} is not absolute", grant.path);
        }
    }

    /// `rivet-proxyd`'s manifest as the carried application declared it —
    /// `cmd/rivet-proxyd/Cargo.toml`'s `[package.metadata.rivet]`, in the
    /// canonical form an installer writes. **Hand-composed from that file**, not
    /// captured from a machine.
    const CARRIED_PROXY_MANIFEST: &str = "kind = \"daemon\"\nrestart = \"on-failure\"\n\
         ready = \"listening\"\nnetwork = \"none\"\nstate = \"private\"\n\
         listen = [\"tcp:443\"]\nprovides = []\nconnects = []\nlibrary = []\n\
         devices = []\nsettings = [\"upstream-name\"]\n";

    /// **The proxy built into the image reaches exactly what it reached as an
    /// application**, except that its program is one file on the system slot
    /// rather than an installed version directory — `ADR-060`.
    ///
    /// Compared row for row, in order, against [`policy`] over the manifest the
    /// proxy shipped with. The two share `home_rows` and `platform_rows`, and a
    /// row one of them grew on its own would be two answers to what a confined
    /// program on this machine may reach.
    #[test]
    fn the_image_proxy_reaches_what_the_carried_proxy_reached() {
        let manifest =
            Manifest::parse(CARRIED_PROXY_MANIFEST.as_bytes()).expect("the carried manifest parses");
        let proxy = name(crate::service::PROXYD_SERVICE_NAME);

        let mut expected = policy("/state", &proxy, &manifest, Some(147), &[]);
        let current = crate::service::current_dir("/state", &proxy);
        assert_eq!(expected.grants[0].path, current, "an application's first row is `current`");
        expected.grants.remove(0);
        expected.grants.insert(
            0,
            Grant {
                path: crate::service::PROXYD_PATH.to_string(),
                access: Access::READ.with(Access::EXECUTE),
                presence: Presence::Required,
                because: "the program the supervisor is about to exec, a single file on the \
                          read-only system slot",
            },
        );

        let image = image_service_policy(crate::service::PROXYD_PATH, "/state", &proxy, Some(147), &[]);
        assert_eq!(
            image.grants.iter().map(|g| (&g.path, g.access, g.presence)).collect::<Vec<_>>(),
            expected.grants.iter().map(|g| (&g.path, g.access, g.presence)).collect::<Vec<_>>(),
        );
    }

    /// **The program row is the file, read and execute, and Required.**
    ///
    /// Not its directory: `/system/bin` holds the supervisor and every other
    /// image program, and a directory grant would hand all of them to a
    /// program running as its own uid. Not writable: the slot is read-only and
    /// the policy says so too.
    #[test]
    fn an_image_services_program_is_granted_as_one_file_it_may_execute() {
        let proxy = name(crate::service::PROXYD_SERVICE_NAME);
        let p = image_service_policy(crate::service::PROXYD_PATH, "/state", &proxy, None, &[]);
        let program = p
            .grants
            .iter()
            .find(|g| g.path == crate::service::PROXYD_PATH)
            .expect("the program is granted");
        assert_eq!(program.access, Access::READ.with(Access::EXECUTE));
        assert_eq!(program.presence, Presence::Required);
        assert!(
            !p.grants.iter().any(|g| g.path == "/system/bin" || g.path == "/system"),
            "the directory around the program is never granted"
        );
        assert!(
            !p.grants.iter().any(|g| g.path == crate::service::current_dir("/state", &proxy)),
            "an image service has no installed version directory to grant"
        );
    }

    /// Every path the supervisor hands an image service is granted — the same
    /// tripwire as `handed_paths_are_granted_paths`, pointed at the other kind
    /// of confined program — and no row grants nothing.
    #[test]
    fn an_image_service_is_granted_every_path_it_is_handed() {
        let proxy = name(crate::service::PROXYD_SERVICE_NAME);
        let p = image_service_policy(crate::service::PROXYD_PATH, "/state", &proxy, None, &[]);
        for path in [
            crate::service::data_dir("/state", &proxy),
            crate::service::tmp_dir("/state", &proxy),
            crate::settings::path("/state", &proxy),
            crate::proxy::path("/state", &proxy),
        ] {
            assert!(
                p.grants.iter().any(|g| g.path == path && g.access.holds(Access::READ)),
                "{path} is handed to the image service and not granted"
            );
        }
        for grant in &p.grants {
            assert!(!grant.access.is_empty(), "{} grants nothing", grant.path);
        }
    }

    /// **The name service reads its projection and not the machine document**
    /// — `ADR-047` §6. The projection is granted read and nothing else, so the
    /// service cannot rewrite the zone it serves; the document, the directory
    /// that holds it and the kernel's hostname file are not granted at all.
    #[test]
    fn the_name_service_reads_its_projection_and_nothing_of_the_document() {
        let dnsd = name(crate::service::DNSD_SERVICE_NAME);
        let p = image_service_policy(crate::service::DNSD_PATH, "/state", &dnsd, Some(147), &[]);
        let projection = crate::name_service::path("/state", &dnsd);
        let row = p
            .grants
            .iter()
            .find(|g| g.path == projection)
            .expect("the projection is granted");
        assert_eq!(row.access, Access::READ, "read, and never write");
        assert_eq!(row.presence, Presence::Optional);
        for forbidden in [
            "/state/config",
            "/state/config/machine.toml",
            "/proc",
            "/proc/sys/kernel/hostname",
            "/state",
            "/state/app",
        ] {
            assert!(
                !p.grants.iter().any(|g| g.path == forbidden),
                "{forbidden} is granted to the name service"
            );
        }
        // Writing is its own home's data and scratch directories, and nowhere
        // else.
        let writable: Vec<&str> = p
            .grants
            .iter()
            .filter(|g| g.access.holds(Access::WRITE) && !g.path.starts_with("/dev/"))
            .map(|g| g.path.as_str())
            .collect();
        for path in &writable {
            assert!(
                path.starts_with("/state/app/rivet-dnsd/") || *path == RUNTIME_DIR,
                "{path} is writable by the name service"
            );
        }
    }

    /// **The mDNS responder reads its projection and nothing it announces
    /// from** — `ADR-017` §3. Not the document, not the DHCP leases, and not
    /// the kernel's hostname file: it takes the name by `uname(2)`, which is
    /// no path at all. The lease directory sits under [`RUNTIME_DIR`], which
    /// every confined program is granted; it is the directory's own mode,
    /// `0700` and root's, that keeps another uid out of it, and that is why the
    /// addresses are projected rather than read.
    #[test]
    fn the_mdns_responder_reads_its_projection_and_not_the_leases_or_the_document() {
        let mdnsd = name(crate::service::MDNSD_SERVICE_NAME);
        let p = image_service_policy(crate::service::MDNSD_PATH, "/state", &mdnsd, Some(147), &[]);
        let projection = crate::mdns::path("/state", &mdnsd);
        let row = p
            .grants
            .iter()
            .find(|g| g.path == projection)
            .expect("the projection is granted");
        assert_eq!(row.access, Access::READ, "read, and never write");
        assert_eq!(row.presence, Presence::Optional);
        for forbidden in [
            "/state/config",
            "/state/config/machine.toml",
            "/run/rivet/dhcp",
            "/run/rivet/dhcp/eth0.lease",
            "/proc",
            "/proc/sys/kernel/hostname",
            "/state",
            "/state/app",
        ] {
            assert!(
                !p.grants.iter().any(|g| g.path == forbidden),
                "{forbidden} is granted to the mDNS responder"
            );
        }
        let writable: Vec<&str> = p
            .grants
            .iter()
            .filter(|g| g.access.holds(Access::WRITE) && !g.path.starts_with("/dev/"))
            .map(|g| g.path.as_str())
            .collect();
        for path in &writable {
            assert!(
                path.starts_with("/state/app/rivet-mdnsd/") || *path == RUNTIME_DIR,
                "{path} is writable by the mDNS responder"
            );
        }
    }

    /// **The compositor gets Mesa's device lookup because it declares
    /// `display`, and an image service that declares nothing does not** —
    /// `ADR-058` §4. The rows are the ones an application declaring `display`
    /// is given, by the same function; `/dev/dri` is listable and never
    /// readable, because which card node it may OPEN is the device grant's
    /// answer and is appended by the supervisor, not composed here.
    #[test]
    fn the_compositor_reaches_mesa_and_no_other_image_service_does() {
        let displayd = name(crate::compositor::PROVIDER);
        let display = [crate::package::application::Device::Display];
        let p = image_service_policy(crate::service::DISPLAYD_PATH, "/state", &displayd, None, &display);
        for path in [DRM_SYSFS_CHAR_DIR, DRM_SYSFS_DEVICES_DIR, DRM_SYSFS_CLASS_DIR] {
            let row = p.grants.iter().find(|g| g.path == path).expect("the sysfs tree is granted");
            assert_eq!(row.access, Access::READ.with(Access::LIST), "{path}");
        }
        let nodes = p.grants.iter().find(|g| g.path == DRM_NODE_DIR).expect("/dev/dri is listed");
        assert_eq!(nodes.access, Access::LIST, "listing the node directory is not opening a node");
        assert!(
            !p.grants.iter().any(|g| g.path.starts_with("/dev/dri/") || g.path == "/dev/fb0"),
            "a device node is the supervisor's grant, expanded against the machine"
        );
        let gbm = p.grants.iter().find(|g| g.path == GBM_BACKEND_DIR).expect("the GBM backend");
        assert!(gbm.access.holds(Access::EXECUTE), "a backend is loaded, not read");

        // The application form of the same declaration gets the same rows.
        let manifest = plain().with_devices(display.to_vec());
        let app = policy("/state", &displayd, &manifest, None, &[]);
        let mesa = |p: &Policy| -> Vec<(String, Access)> {
            p.grants
                .iter()
                .filter(|g| g.path.starts_with("/sys/") || g.path == DRM_NODE_DIR)
                .map(|g| (g.path.clone(), g.access))
                .collect()
        };
        assert_eq!(mesa(&p), mesa(&app));

        let dnsd = name(crate::service::DNSD_SERVICE_NAME);
        let none = image_service_policy(crate::service::DNSD_PATH, "/state", &dnsd, None, &[]);
        assert!(mesa(&none).is_empty(), "a service that declared no display reads no hardware tree");
    }

    #[test]
    fn the_words_are_the_bits() {
        assert_eq!(Access::NONE.words(), Vec::<&str>::new());
        assert_eq!(
            Access::READ.with(Access::EXECUTE).words(),
            vec!["read", "execute"]
        );
        assert!(device_access().holds(Access::DEVICE_CONTROL));
    }
}
