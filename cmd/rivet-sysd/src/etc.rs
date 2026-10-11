//! The runtime `/etc` projection — **the one thing that generates `/etc`.**
//!
//! `ADR-007` §3.5: `/etc` "exists, is generated, and lives on a tmpfs. It is a
//! build artifact of applying the configuration, not an input to anything."
//! `ADR-008` §5: it "is not part of Rivet's design at all", it exists because
//! musl has nine paths compiled in as string literals, and `ADR-008` §5.3.1
//! decides which of the nine are rendered and which are deliberately absent.
//!
//! `rivet_core::etc` holds all of that as data. This module is the half that
//! touches the world: mount a tmpfs at `/etc`, write the rendered files onto it,
//! remount it read-only, and read the result back off the kernel rather than
//! asserting it.
//!
//! # Why the supervisor and not PID 1 — ruled 2026-08-23, `ADR-017` §3.1.8
//!
//! This was the substantive decision in writing this module, and the record
//! genuinely pointed both ways. It is argued at length in `ADR-017` §3.1.8; the
//! short form is three points.
//!
//! **The case for PID 1** is that a supervisor that fails before writing `/etc`
//! leaves a machine nobody can log into, and PID 1 is the
//! process that is not allowed to fail.
//!
//! **The case against it is that the failure it describes cannot happen.**
//! Everything that reads `/etc` is a *service*, so it is started by the
//! supervisor. A supervisor that dies before rendering `/etc` also dies before
//! starting anything that reads `/etc` — and `ADR-017` §3.1.1's re-adoption
//! case is the same the other way round: a service that is *already* running
//! survived a supervisor that had already rendered `/etc`, because rendering
//! happens before the first `converge`. There is no ordering in which a process
//! reads `/etc` on a machine where this module has not run.
//!
//! **And `/state`'s precedent does not carry.** `ADR-017` §3.1.7 grew PID 1's
//! job list by one filesystem the same day, and its deciding argument was
//! specific rather than general: "the failure to mount `/state` is the single
//! event *least* able to be journalled, because journalling it is what has just
//! failed". `/etc` has no such property. The journal is the supervisor's own
//! (`ADR-017` §8 item 2), it is running before this module is called, and a
//! failure here is journalled by the thing that is already up — which is what
//! every record this module emits demonstrates.
//!
//! # The three honesty mechanisms `ADR-007` §3.6.1 requires, and where they are
//!
//! | | Mechanism | Here |
//! | :-- | :-- | :-- |
//! | 1 | the mount is genuinely read-only | [`remount_read_only`] — and a **failure to remount is reported loudly**, because a writable `/etc` is §3.6's silent-data-loss row |
//! | 2 | a provenance header on every generated file | `rivet_core::etc::header`, on the two formats that can carry a comment (`ADR-008` §5.3.2) |
//! | 3 | a reverse lookup — `rivet explain /etc/passwd` | **not built**, and cannot be: `cmd/rivet` does not exist |
//!
//! # What is unproven
//!
//! **Nothing here has run on a board.** It is exercised under QEMU, where the
//! root filesystem is an initramfs rather than the read-only `erofs` of
//! `ADR-018` §3 — so the `mkdir` for the mount point succeeds there and on
//! hardware it will find the directory the image already ships
//! (`cmd/rivet-image`'s `system::ETC`). `ADR-026` §4's right-hand column
//! applies in full.

use rivet_core::etc::{
    ApplicationAccount, ETC_FLAGS, ETC_FSTYPE, ETC_MOUNT_POINT, ETC_TMPFS_DATA, EtcFile, EtcInputs,
    LOGIN_SHELL_PATHS, SHELLS, etc_projection,
};
use rivet_core::service::ServiceName;

use crate::journal::{Journal, Severity, Value};
use crate::kernel::{Errno, Kernel, MountFlags};
use crate::supervise::SELF_NAME;

/// What happened to the filesystem `/etc` is rendered onto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountOutcome {
    /// The kernel mounted the tmpfs on this call.
    Mounted,
    /// It was already there — a previous run of this supervisor
    /// (`ADR-017` §3.1). Re-running converges; it does not fail.
    Converged,
    /// Reported and survived. `/etc` is whatever it was before.
    Refused { errno: Errno, detail: String },
}

/// What the projection left behind, as a value the caller can act on.
///
/// **Never an `Err`.** `ADR-017` §3.1 lets this process fail, but not over
/// this: a supervisor that exited because `/etc` would not mount would be
/// restarted by PID 1 into the same failure for ever, and the services it could
/// still have run would never start. So the projection reports and carries on,
/// exactly as PID 1's mount phase does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Projection {
    /// Every file `rivet_core::etc` defines is on the tmpfs.
    Rendered {
        files: usize,
        /// Whether `ADR-007` §3.6.1 item 1 actually holds on this machine.
        read_only: bool,
        /// Whether `/etc/shells` names anything at all — see
        /// [`Projection::permits_a_login`].
        names_a_login_shell: bool,
    },
    /// The tmpfs is there and at least one file is not. Named separately from
    /// [`Rendered`](Self::Rendered) because a *partial* `/etc` is the state
    /// most likely to be misread as working: `getpwnam` answers and
    /// `getusershell` does not, or the other way round.
    Partial {
        written: usize,
        failed: usize,
        read_only: bool,
        names_a_login_shell: bool,
    },
    /// There is no tmpfs at `/etc`, so `/etc` is whatever the image left there
    /// — which is an empty directory (`ADR-007` §3.5). Every login is rejected
    /// before authentication.
    Refused { detail: String },
}

impl Projection {
    /// Whether anything can log in to this machine on the strength of `/etc`.
    ///
    /// Deliberately *not* the same question as "did the projection succeed".
    /// With no `cmd/rivet` on the machine, `/etc/shells` names no shell, so
    /// this is `false` even after a perfect render — and saying so is the whole
    /// point (`AGENTS.md` §6).
    pub fn permits_a_login(&self) -> bool {
        match self {
            Self::Rendered {
                names_a_login_shell,
                ..
            }
            | Self::Partial {
                names_a_login_shell,
                ..
            } => *names_a_login_shell,
            Self::Refused { .. } => false,
        }
    }
}

/// Where the login shells would be, and which of them are there.
///
/// This is the only fact the projection reads off the machine rather than out
/// of a constant, and it is read rather than assumed for the reason
/// `rivet_core::etc::EtcInputs` gives: naming a binary that is not there
/// produces an `/etc/shells` pointing at nothing, which reads as working and is
/// not.
///
/// A probe that *fails* — as distinct from one that answers "not there" —
/// drops that entry too, and says so at `Warn`. That is the safe direction: an
/// unlisted shell rejects a login before authentication with a message naming
/// the shell, while a listed-but-absent one fails at `exec` after
/// authentication with a message naming nothing.
///
/// **It probes every candidate, and the ruling that makes `nu` optional is the reason it
/// must keep doing so.** The ruling that made `nu` `root`'s login shell is
/// cheap precisely because nothing here is told what the image contains: a card
/// built without `nu` finds one entry, projects a working `/etc/passwd` naming
/// `cmd/rivet`, and `permits_a_login` stays true. Replacing this with a
/// compile-time constant would trade that away, and the loss would only be
/// visible on a card nobody built yet.
///
/// The returned order is [`LOGIN_SHELL_PATHS`]' order, filtered — which matters
/// because the first entry becomes `/etc/passwd`'s seventh field.
/// Read the trust roots this image ships, or `None` if it ships none.
///
/// **Derived by looking**, on `login_shells`' rule and for its reason: an image
/// without a bundle must project no file rather than an empty one, and only the
/// filesystem can say which this image is.
///
/// A read error is reported and treated as absent. That is the safe direction:
/// no `/etc/ssl/certs/ca-certificates.crt` fails every TLS verification loudly,
/// where a truncated one would fail some and not others depending on which
/// authority signed what.
fn ca_bundle(k: &mut dyn Kernel, journal: &mut Journal) -> Option<String> {
    match k.read_file(rivet_core::etc::CA_BUNDLE_SOURCE) {
        // `read_file` already answers text — the Kernel port decodes, so a
        // bundle that is not UTF-8 fails there rather than here.
        Ok(text) => Some(text),
        Err(errno) => {
                note(
                    k,
                    journal,
                    Severity::Error,
                    "the trust roots on this image are not text, so no CA bundle was projected \
                     and every TLS verification on this machine will fail",
                    vec![(
                        "path".to_string(),
                        Value::Str(rivet_core::etc::CA_BUNDLE_SOURCE.to_string()),
                    )],
                );
            note(
                k,
                journal,
                Severity::Warn,
                "this image ships no CA bundle, so nothing on this machine can verify a TLS \
                 certificate. An HTTPS client will report `certificate signed by unknown \
                 authority` and the cause is here",
                vec![
                    (
                        "path".to_string(),
                        Value::Str(rivet_core::etc::CA_BUNDLE_SOURCE.to_string()),
                    ),
                    ("errno".to_string(), Value::Str(errno.to_string())),
                ],
            );
            None
        }
    }
}

/// The whole of the `/etc` tmpfs, as a read-back ceiling.
///
/// `rivet_core::etc::ETC_TMPFS_DATA` mounts it `size=1m`, so nothing this
/// projection wrote to `/etc` can be bigger than this — a read that hit the cap
/// would mean the file grew after it was written, which is a different failure
/// from the one the read-back is looking for and is worth not conflating.
const ETC_TMPFS_CAP_BYTES: usize = 1024 * 1024;

/// The largest tzfile this will read.
///
/// **Measured, not guessed**: the whole of tzdata 2026c — 599 zones, read off
/// the build container on 2026-09-02 — is 700 KB, and the largest single file
/// in it is under 4 KB. 64 KB is two orders of magnitude of headroom against a
/// future database and still a cap, which is what `read_file_bytes` wants: a
/// projection that read an unbounded file into a `/etc` tmpfs with a 1 MB cap
/// would fail at the write instead of at the read, and the diagnostic would
/// name the wrong thing.
const MAX_TZFILE_BYTES: usize = 64 * 1024;

/// Read the tzfile for the zone the document names — `ADR-021` §9.
///
/// **Three distinct failures, three records, and every one of them ends in the
/// same place: this machine keeps UTC.** `ADR-034` §5.3 is why they are not
/// collapsed into one: a document that says `America/Chicago` on a machine
/// whose clock is UTC is a setting reported as applied when it is not, and the
/// only thing standing between that and an operator is what this function says
/// in the journal.
///
/// * **The document named nothing** — not a failure, and not journalled. UTC is
///   what the operator asked for by saying nothing.
/// * **The document named a zone this image has no file for.** The commonest
///   real mistake, because the schema deliberately cannot refuse it: which
///   zones exist is a property of the image (`rivet_core::timezone`).
/// * **The file is there and is not a tzfile.** musl would map it, compare the
///   magic, unmap it and keep UTC **silently** — so this is the one case where
///   copying the bytes would look like success at every step and produce a
///   wrong clock. It is refused here instead, with the same check musl makes.
fn local_time<'a>(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    zone: Option<&'a rivet_core::timezone::Timezone>,
    bytes: &'a mut Vec<u8>,
) -> Option<rivet_core::etc::LocalTime<'a>> {
    let zone = zone?;
    let path = zone.tzfile_under(rivet_core::etc::ZONEINFO_DIR);
    match k.read_file_bytes(&path, MAX_TZFILE_BYTES) {
        Ok(read) => {
            if !rivet_core::timezone::is_tzfile(&read) {
                note(
                    k,
                    journal,
                    Severity::Error,
                    "the zone file for the timezone this document names is NOT a tzfile, so no \
                     /etc/localtime was projected and this machine keeps UTC. musl would have \
                     mapped it, seen the wrong magic and kept UTC without saying anything, \
                     which is the silent wrong clock this refusal exists to prevent",
                    vec![
                        ("timezone".to_string(), Value::Str(zone.to_string())),
                        ("path".to_string(), Value::Str(path)),
                        (
                            "bytes".to_string(),
                            Value::Int(i64::try_from(read.len()).unwrap_or(i64::MAX)),
                        ),
                    ],
                );
                return None;
            }
            *bytes = read;
            Some(rivet_core::etc::LocalTime {
                zone: zone.as_str(),
                tzfile: bytes,
            })
        }
        Err(errno) => {
            note(
                k,
                journal,
                Severity::Error,
                "this image has no zone file for the timezone this document names, so no \
                 /etc/localtime was projected and this machine keeps UTC. WHICH ZONES EXIST is \
                 a property of the image and not of the document, which is why the document \
                 was accepted and this is where it is reported",
                vec![
                    ("timezone".to_string(), Value::Str(zone.to_string())),
                    ("path".to_string(), Value::Str(path)),
                    ("errno".to_string(), Value::Str(errno.to_string())),
                ],
            );
            None
        }
    }
}

fn login_shells(k: &mut dyn Kernel, journal: &mut Journal) -> Vec<&'static str> {
    let mut found = Vec::new();
    for path in LOGIN_SHELL_PATHS {
        match k.file_exists(path) {
            Ok(true) => found.push(*path),
            Ok(false) => {}
            Err(e) => {
                note(
                    k,
                    journal,
                    Severity::Warn,
                    "could not tell whether a login shell is present, so /etc/shells will not \
                     name it. Listing one that is absent fails at exec after authentication, \
                     which is the worse of the two failures",
                    vec![
                        ("path".to_string(), Value::Str((*path).to_string())),
                        ("detail".to_string(), Value::Str(e.to_string())),
                    ],
                );
            }
        }
    }
    found
}

/// What the configuration document contributes to `/etc`.
///
/// **`hostname` and `timezone` reach it.** `EtcInputs::hostname` and
/// `EtcInputs::local_time` are the answers — ruled 2026-08-26 and 2026-09-02
/// (`ADR-021` §9) respectively. The guard test below is what asks the
/// question every time the schema grows a key, which is what it is for.
///
/// `ADR-007` §7 lists the rest of the schema key by key and not one of them
/// is a fact any file `rivet_core::etc` renders carries. **What is still open
/// is the human half of `ADR-008` §7.3**, and it is not waiting on a schema
/// key — it is waiting on an authorization layer for a human user to attach
/// to.
///
/// Inventing a `hostname` key to make this parameter earn its place would be
/// exactly the decision `AGENTS.md` §1 reserves to the maintainer, so it is not
/// invented. What is here instead is
/// `the_document_contributes_nothing_and_a_new_key_must_decide_whether_it_does`,
/// which fails the moment `Document::DECLARED_KEYS` changes — so the question
/// is *asked* when the schema grows rather than quietly missed.
///
/// **The shells arrive as an argument rather than being probed here**, because
/// `EtcInputs` borrows the list (the dual-login-shell ruling made it a slice) and the
/// owner has to outlive the projection. The probe is [`login_shells`]; the
/// caller holds its result.
/// **The applications arrive the same way and for the same reason**, added
/// 2026-08-24: `EtcInputs` borrows the list, so the owner has to outlive the
/// projection and the caller holds it. They are *not* the document's — an
/// application account exists because a directory under
/// `rivet_core::service::APP_ROOT` exists, which is what `ADR-008` §7.3 means
/// by *"service accounts arrive with the services that need them"*.
fn inputs_from<'a>(
    shells: &'a [&'a str],
    applications: &'a [ApplicationAccount<'a>],
    naming: &'a crate::naming::Naming,
    nameservers: &'a [core::net::Ipv4Addr],
    search: &'a [&'a str],
    ca_bundle: Option<&'a str>,
    local_time: Option<rivet_core::etc::LocalTime<'a>>,
) -> EtcInputs<'a> {
    EtcInputs {
        login_shells: shells,
        applications,
        hostname: naming.hostname(),
        // The resolver: `[network.resolver]` when the document sets one, else
        // the DHCP lease — computed by the caller (`project`) so each source
        // owns the slice it lends. `rivet_core::etc::RESOLV_CONF` carries why
        // the file exists and why it is absent when this is empty.
        nameservers,
        // Borrowed like the shells and for the same reason: `EtcInputs` holds a
        // slice, so the owner has to outlive the projection and the caller
        // holds it.
        search,
        // Read off /system by the caller, same ownership rule again.
        ca_bundle,        // **The document's, resolved against the image.** Two things had to be
        // true at once and neither implies the other — the document had to name
        // a zone, and this image had to carry its tzfile — which is exactly the
        // shape `login_shells` has and is why `local_time` reports rather than
        // asserts.
        local_time,
    }
}

/// The version this build is, as `ADR-027` §10 spells it and as
/// `rivet --version` prints it.
///
/// Read from this crate's own `CARGO_PKG_VERSION` — every crate in the
/// workspace takes `version.workspace = true`, so there is one number and
/// nothing in this build can disagree with itself about which one it is.
///
/// **`pub(crate)`, not private, since `R33`** (`.rulings/` R-163):
/// `crate::supervise::Supervisor::status` reads the same constant rather than
/// reading its own `CARGO_PKG_VERSION`, so `get status` cannot report a
/// different build than the one that is actually running.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Which of these definitions is an installed application, by name.
///
/// The first half of getting `ADR-015` §5.1's accounts into `/etc`, and it is
/// two halves because [`ApplicationAccount`] borrows its name: somebody has to
/// own the `ServiceName`s, and the caller is the only thing that outlives both
/// this and the projection.
///
/// **It reads the definitions rather than the filesystem**, which is the point.
/// `crate::applications` has already refused a name that is not a name, a
/// manifest that will not parse, a collision with a system service and a
/// collision with another application's uid; a second walk of `/state/app`
/// here would be a second answer to *which applications are on this machine*
/// and would put back into `/etc/passwd` exactly the rows that were refused.
///
/// **Each name comes back with the volume it is installed on** (`ADR-031` §8
/// item 4), because `passwd`'s home field is the application's data directory
/// and that is no longer a function of the name alone. A definition with an
/// identity and no volume cannot happen — `crate::applications::definition`
/// fills both — and if one ever did, it is dropped here rather than given an
/// account whose home directory would be a guess.
pub fn application_names(defs: &[crate::service::ServiceDef]) -> Vec<(ServiceName, String)> {
    defs.iter()
        .filter(|d| d.program.identity.is_some())
        .filter_map(|d| {
            let name = ServiceName::new(&d.name)?;
            let volume = d.volume.clone()?;
            Some((name, volume))
        })
        .collect()
}

/// The accounts, borrowing names the caller owns. See [`application_names`].
///
/// The identity is derived here from the same function that derived it at
/// discovery rather than carried across, because there is one rule and
/// re-applying it is how a second copy of the rule is prevented from existing
/// at all (`ADR-011`).
pub fn accounts(names: &[(ServiceName, String)]) -> Vec<ApplicationAccount<'_>> {
    names
        .iter()
        .map(|(name, volume)| ApplicationAccount {
            name,
            identity: rivet_core::service::app_identity(name),
            volume,
        })
        .collect()
}

/// Say something in the supervisor's own voice, before there is a `Supervisor`
/// to say it through.
fn note(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    sev: Severity,
    msg: &str,
    fields: Vec<(String, Value)>,
) {
    let corr = journal.next_correlation();
    journal.emit(k, sev, SELF_NAME, &corr, msg, fields);
}

/// What the kernel says is mounted at `target` — its word, not ours.
///
/// The **last** matching line wins: a stacked mount shadows the ones beneath
/// it, and the top one is what a path lookup actually reaches. Field 2 is the
/// mount point, field 3 the type, field 4 the options.
fn mount_table_entry<'t>(table: &'t str, target: &str) -> Option<&'t str> {
    table
        .lines()
        .rfind(|line| line.split(' ').nth(1) == Some(target))
}

fn field(line: &str, index: usize) -> Option<&str> {
    line.split(' ').nth(index)
}

/// Classify what `mount(2)` said, in words that name the repair.
///
/// Read by somebody at a serial console with no other diagnostic channel, which
/// is why each arm names a state the machine can actually be in rather than
/// restating the syscall.
fn mount_detail(errno: Errno) -> String {
    match errno {
        Errno::ENOENT => format!(
            "mount({ETC_MOUNT_POINT}): {errno} — the mount point does not exist. The image \
             ships it as an empty directory (ADR-018 §3 makes the root read-only, so nothing \
             can create it at runtime); a card written by something else would not have it"
        ),
        Errno::ENODEV => format!(
            "mount({ETC_MOUNT_POINT}): {errno} — the kernel has no {ETC_FSTYPE}; check \
             CONFIG_TMPFS (ADR-024)"
        ),
        Errno::EACCES | Errno::EPERM => format!(
            "mount({ETC_MOUNT_POINT}): {errno} — no permission. ADR-017 §3.1 runs this \
             process as root, so this is a namespace or LSM problem rather than a Rivet one"
        ),
        Errno::EINVAL => format!(
            "mount({ETC_MOUNT_POINT}): {errno} — the kernel refused the options \
             {:?}. That is rivet_core::etc::ETC_TMPFS_DATA, not anything an operator wrote",
            ETC_TMPFS_DATA.unwrap_or("")
        ),
        _ => format!("mount({ETC_MOUNT_POINT}): {errno}"),
    }
}

/// Classify what a write said.
fn write_detail(path: &str, errno: Errno) -> String {
    match errno {
        Errno::EROFS => format!(
            "{path}: {errno} — /etc is mounted read-only. A previous run of this projection \
             remounted it and this one did not remount it writable first"
        ),
        Errno::ENOSPC => format!(
            "{path}: {errno} — the /etc tmpfs is full. Its cap is \
             rivet_core::etc::ETC_TMPFS_DATA and the projection is a few hundred bytes, so \
             something is writing to /etc that is not this"
        ),
        Errno::ENOENT => format!("{path}: {errno} — the directory it goes in is not there"),
        _ => format!("{path}: {errno}"),
    }
}

/// Mount the tmpfs, converging rather than failing if it is already there.
///
/// **Convergence is asked of the kernel, not inferred**, and for a sharper
/// reason here than for a block device: tmpfs mints a new superblock on every
/// mount, so the kernel does *not* refuse a second one. It stacks it, the
/// files this run writes land on the new one, and every file a previous run
/// wrote is hidden underneath — an `/etc` that looks right, is right, and grows
/// a layer on every supervisor restart (`ADR-017` §3.1 expects several).
fn mount_tmpfs(k: &mut dyn Kernel, journal: &mut Journal) -> MountOutcome {
    // The mount point. On a board this is already in the image and this is a
    // no-op; under QEMU the initramfs holds only /init and /dev, so it is not.
    // A failure here is not the refusal that matters — mount(2)'s is — but it
    // is worth saying, because ENOENT from mount(2) afterwards is otherwise a
    // second report of the same cause.
    if let Err(e) = k.mkdir_p(ETC_MOUNT_POINT) {
        note(
            k,
            journal,
            Severity::Warn,
            "could not create the /etc mount point; the mount below is what decides",
            vec![
                ("path".to_string(), Value::Str(ETC_MOUNT_POINT.to_string())),
                ("detail".to_string(), Value::Str(e.to_string())),
            ],
        );
    }

    if let Ok(table) = k.read_mounts()
        && let Some(line) = mount_table_entry(&table, ETC_MOUNT_POINT)
    {
        let fstype = field(line, 2).unwrap_or("?").to_string();
        note(
            k,
            journal,
            Severity::Info,
            "/etc is already mounted; converging onto it rather than stacking a second tmpfs",
            vec![("fstype".to_string(), Value::Str(fstype))],
        );
        return MountOutcome::Converged;
    }

    match k.mount(
        ETC_FSTYPE,
        ETC_MOUNT_POINT,
        ETC_FSTYPE,
        MountFlags::from_core(ETC_FLAGS),
        ETC_TMPFS_DATA,
    ) {
        Ok(()) => MountOutcome::Mounted,
        // The backstop for the case above when /proc is unreadable. tmpfs does
        // not normally answer EBUSY, but PID 1's own convergence does not rely
        // on a filesystem type's behaviour for its correctness and neither does
        // this.
        Err(Errno::EBUSY) => MountOutcome::Converged,
        Err(errno) => MountOutcome::Refused {
            errno,
            detail: mount_detail(errno),
        },
    }
}

/// Make an already-mounted `/etc` writable again.
///
/// Only reached on a re-render, because the previous run left it read-only. A
/// refusal here is not fatal: the writes below will answer `EROFS` and say so
/// per file, which is a more useful console than one line here would be.
fn remount_writable(k: &mut dyn Kernel) -> Result<(), Errno> {
    k.mount(
        ETC_FSTYPE,
        ETC_MOUNT_POINT,
        ETC_FSTYPE,
        MountFlags::from_core(ETC_FLAGS).union(MountFlags::REMOUNT),
        ETC_TMPFS_DATA,
    )
}

/// `ADR-007` §3.6.1 item 1, which that section calls **non-negotiable**:
/// "the mount is genuinely read-only, so the write fails rather than being
/// accepted and discarded. Everything else is decoration without it."
///
/// Without this the middle row of §3.6's table is what a Rivet has: an edit
/// to `/etc/shells` succeeds, the operator believes it, and the next boot
/// discards it silently. That is the worst outcome the section enumerates, and
/// it is the default state of a tmpfs.
fn remount_read_only(k: &mut dyn Kernel) -> Result<(), Errno> {
    k.mount(
        ETC_FSTYPE,
        ETC_MOUNT_POINT,
        ETC_FSTYPE,
        MountFlags::from_core(ETC_FLAGS)
            .union(MountFlags::REMOUNT)
            .union(MountFlags::RDONLY),
        ETC_TMPFS_DATA,
    )
}

/// Write one file and read it straight back off the kernel.
///
/// The read-back is not decoration (`AGENTS.md` §5). `write(2)` returning zero
/// is *our* claim that `/etc/shells` says what it should; the file is the
/// kernel's, and a tmpfs that silently truncated — because the cap in
/// [`ETC_TMPFS_DATA`] was reached mid-write — would answer `Ok` for every write
/// and hold something else.
fn write_and_verify(k: &mut dyn Kernel, journal: &mut Journal, file: &EtcFile) -> bool {
    // **A projected path may be nested**, since 2026-09-01 and
    // `rivet_core::etc::CA_BUNDLE`. Every file before it sat directly in /etc,
    // so only the mount point was ever created; a nested one fails `write(2)`
    // with ENOENT and the diagnostic names the file rather than the missing
    // directory, which sends its reader to the wrong question.
    //
    // The mount point itself is created by the caller and is not re-created
    // here: this only runs for a path with a directory component below /etc.
    if let Some((parent, _)) = file.path.rsplit_once('/')
        && parent != ETC_MOUNT_POINT
        && let Err(errno) = k.mkdir_p(parent)
    {
        note(
            k,
            journal,
            Severity::Error,
            "a directory of /etc could not be created, so the file inside it was not written",
            vec![
                ("path".to_string(), Value::Str(parent.to_string())),
                ("errno".to_string(), Value::Str(errno.to_string())),
            ],
        );
        return false;
    }
    if let Err(errno) = k.write_file(file.path, &file.contents, file.mode) {
        note(
            k,
            journal,
            Severity::Error,
            "a file of /etc was not written",
            vec![
                ("path".to_string(), Value::Str(file.path.to_string())),
                (
                    "detail".to_string(),
                    Value::Str(write_detail(file.path, errno)),
                ),
                ("why".to_string(), Value::Str(file.reason.to_string())),
            ],
        );
        return false;
    }

    // **`read_file_bytes`, not `read_file`, since 2026-09-02.** `/etc/localtime`
    // is a tzfile, so a read that decoded UTF-8 would fail on the one file whose
    // contents this check matters most for — musl keeps UTC in silence when the
    // bytes are wrong, and this read-back is the only thing that would say so.
    // The cap is the tmpfs's own (`ETC_TMPFS_DATA`): nothing this projection
    // wrote can be larger than the filesystem it wrote it to.
    let read_back = k.read_file_bytes(file.path, ETC_TMPFS_CAP_BYTES);
    let verified = read_back.as_deref() == Ok(file.contents.as_slice());
    note(
        k,
        journal,
        if verified {
            Severity::Info
        } else {
            Severity::Error
        },
        if verified {
            "rendered"
        } else {
            "rendered, and the file read back is NOT what was written"
        },
        vec![
            ("path".to_string(), Value::Str(file.path.to_string())),
            (
                "mode".to_string(),
                Value::Str(format!("{:04o}", file.mode)),
            ),
            (
                "bytes".to_string(),
                Value::Int(i64::try_from(file.contents.len()).unwrap_or(i64::MAX)),
            ),
            (
                "read_back_bytes".to_string(),
                Value::Int(
                    read_back
                        .as_ref()
                        .map_or(-1, |s| i64::try_from(s.len()).unwrap_or(i64::MAX)),
                ),
            ),
            ("why".to_string(), Value::Str(file.reason.to_string())),
        ],
    );
    verified
}

/// The directory `cmd/rivet-dhcp-hook` writes a lease into, one file per
/// interface — `rivet_adapter_net::lease::LEASE_DIR`.
///
/// **The path and the two field names are contracted here rather than
/// depended on.** The canonical writer is `rivet_adapter_net::lease`
/// (`Lease::to_pairs_text`), but that crate is Linux-only and this one builds
/// on the host for its tests, so it cannot be a dependency. What is reused
/// instead is the *format*: `key=value` lines, with `dns=<space-separated
/// IPv4>` and `domain=<name>`. `ADR-011` §3's failure is two readers of one
/// **address** disagreeing; this reads only the resolver and never the address,
/// so the fact that could disagree is not read here at all.
pub(crate) const LEASE_DIR: &str = "/run/rivet/dhcp";

/// The nameservers and search list a DHCP lease supplies.
///
/// Read only when the document's `[network.resolver]` is silent, so a resolver
/// the operator wrote always wins — `rivet_config::resolver` is the static half
/// and this is the lease half (`ADR-023`), the one authority (`ADR-011`) being
/// the document whenever it speaks.
struct LeasedResolver {
    nameservers: Vec<core::net::Ipv4Addr>,
    search: Vec<String>,
}

/// Read the DHCP lease(s) and return the resolver they offer, or `None`.
///
/// **Only the resolver, and only when asked**: `project` calls this solely when
/// the document said nothing. Nameservers are taken in the order the leases
/// list them, deduplicated, and capped at `Resolver::MAX` because musl reads no
/// more. A lease with no `dns=` contributes nothing, and if none offers a
/// nameserver the whole thing is `None` — which projects **no**
/// `/etc/resolv.conf`, musl's clean `127.0.0.1` fail-fast, exactly as a silent
/// document does. An absent lease directory is the ordinary case on a
/// statically-addressed machine and on any board before DHCP has completed, so
/// it is not a fault and not journalled.
fn leased_resolver(k: &mut dyn Kernel, journal: &mut Journal) -> Option<LeasedResolver> {
    // `list_dir_entries` and not `list_dir`: a lease is a FILE, and `list_dir`
    // answers directories only (`Kernel::list_dir`'s own contract).
    let entries = k.list_dir_entries(LEASE_DIR).ok()?;
    let mut nameservers: Vec<core::net::Ipv4Addr> = Vec::new();
    let mut search: Vec<String> = Vec::new();
    for name in entries {
        if !name.ends_with(".lease") {
            continue;
        }
        let Ok(text) = k.read_file(&format!("{LEASE_DIR}/{name}")) else {
            continue;
        };
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "dns" => {
                    for field in value.split_whitespace() {
                        if let Ok(ip) = field.parse::<core::net::Ipv4Addr>() {
                            if !nameservers.contains(&ip) {
                                nameservers.push(ip);
                            }
                        }
                    }
                }
                "domain" => {
                    let domain = value.trim();
                    if !domain.is_empty() && !search.iter().any(|d| d == domain) {
                        search.push(domain.to_string());
                    }
                }
                _ => {}
            }
        }
    }
    // musl reads at most `MAXNS`, which `Resolver::MAX` mirrors; a fourth would
    // be written and never asked, so it is dropped rather than rendered.
    nameservers.truncate(rivet_config::resolver::Resolver::MAX);
    if nameservers.is_empty() {
        return None;
    }
    note(
        k,
        journal,
        Severity::Info,
        "no [network.resolver] in the document, so /etc/resolv.conf is filled from the DHCP \
         lease (ADR-023). A resolver named in the document would win over this",
        vec![(
            "nameservers".to_string(),
            Value::Str(
                nameservers
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
        )],
    );
    Some(LeasedResolver { nameservers, search })
}

/// Render the whole of `/etc`, onto a tmpfs, before anything that reads it
/// starts.
///
/// The order is the contract and it is stated here because nothing else states
/// it:
///
/// 1. the document is already in hand — the caller supplies it, because
///    `rivet-store` does not exist and `cmd/rivet-sysd` has nowhere to
///    read one from (see `run` in `main.rs`);
/// 2. the tmpfs is mounted;
/// 3. every file is written and read back;
/// 4. the tmpfs is remounted read-only;
/// 5. **only then** does the caller converge any service.
///
/// Step 5 is the caller's and is the reason this returns a value rather than
/// spawning anything: `ADR-017` §3.2 has all services start together and retry,
/// but "retry" is about a socket that is not listening yet, not about a
/// `getpwnam` that answered wrongly and was believed.
pub fn project(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    document: &rivet_config::Document,
    applications: &[ApplicationAccount<'_>],
    naming: &crate::naming::Naming,
) -> Projection {
    let shells = login_shells(k, journal);

    // **The resolver: the document wins; a silent `[network.resolver]` lets the
    // DHCP lease fill it.** `rivet_config::resolver` is the static half and
    // `leased_resolver` is the lease half (`ADR-023`); the one authority
    // (`ADR-011`) is the document whenever it names a resolver at all — silent
    // meaning `is_silent`, no nameservers and no search. Both nameservers and
    // search are owned here so the slices `EtcInputs` borrows outlive the
    // projection, on `login_shells`' rule.
    let document_silent = document.resolver().map_or(true, |r| r.is_silent());
    let leased = document_silent.then(|| leased_resolver(k, journal)).flatten();
    let (nameservers, search): (&[core::net::Ipv4Addr], Vec<&str>) = match &leased {
        Some(l) => (&l.nameservers, l.search.iter().map(String::as_str).collect()),
        None => (
            document.nameservers(),
            document
                .resolver()
                .map(|r| r.search().iter().map(|d| d.as_str()).collect())
                .unwrap_or_default(),
        ),
    };

    let roots = ca_bundle(k, journal);
    // Owned here, on `search`'s terms exactly: `EtcInputs` borrows the tzfile,
    // so the bytes have to outlive the projection and the caller is the only
    // thing that does.
    let mut tzfile = Vec::new();
    let zone = local_time(k, journal, document.timezone(), &mut tzfile);
    let inputs = inputs_from(
        &shells,
        applications,
        naming,
        nameservers,
        &search,
        roots.as_deref(),
        zone,
    );

    let mount = mount_tmpfs(k, journal);
    match &mount {
        MountOutcome::Mounted => note(
            k,
            journal,
            Severity::Info,
            "mounted a tmpfs over /etc (ADR-007 §3.5): /etc is generated output, never a source",
            vec![
                ("fstype".to_string(), Value::Str(ETC_FSTYPE.to_string())),
                (
                    "options".to_string(),
                    Value::Str(ETC_TMPFS_DATA.unwrap_or("").to_string()),
                ),
            ],
        ),
        MountOutcome::Converged => {
            if let Err(e) = remount_writable(k) {
                note(
                    k,
                    journal,
                    Severity::Warn,
                    "could not remount /etc writable to re-render it; the writes below will say \
                     what that cost",
                    vec![("detail".to_string(), Value::Str(e.to_string()))],
                );
            }
        }
        MountOutcome::Refused { detail, .. } => {
            note(
                k,
                journal,
                Severity::Error,
                "REFUSED /etc could not be mounted, so nothing is rendered. musl reads nine \
                 paths under /etc (ADR-008 §5.1) and will find none of them: getpwnam(\"root\") \
                 answers nothing, localhost is resolved over DNS, and getusershell() claims \
                 /bin/sh and /bin/csh are this machine's login shells",
                vec![("detail".to_string(), Value::Str(detail.clone()))],
            );
            return Projection::Refused {
                detail: detail.clone(),
            };
        }
    }

    // Every file is attempted. One refusal does not stop the rest, on PID 1's
    // reasoning: a failure on /etc/hosts must not cost /etc/shells, and a more
    // complete /etc is worth more than a tidy abort.
    let files = etc_projection(&inputs);
    let total = files.len();
    let mut written = 0usize;
    // Tracked by name rather than inferred from the count, because the question
    // the reading below answers is about ONE file: did the tzfile actually land.
    let mut localtime_written = false;
    for file in &files {
        if write_and_verify(k, journal, file) {
            written += 1;
            if file.path == rivet_core::etc::LOCALTIME {
                localtime_written = true;
            }
        }
    }

    // **Ask the C library what it now believes** — `ADR-021` §9, and this is
    // the only thing in the whole chain that is evidence rather than intent.
    //
    // Everything above it is Rivet talking to itself: the document parsed, the
    // tzfile was found, the bytes were written and read back. musl checks the
    // magic itself and falls back to UTC **without a word**, so all of that can
    // succeed on a machine whose clock is UTC. `Kernel::local_time` is
    // `localtime_r(3)` — the library, reading the file this projection just
    // wrote, saying what it made of it.
    //
    // Placed here, before the read-only remount, only because that is where the
    // write finished; the remount does not affect a read either way.
    if let Some(zone) = document.timezone().filter(|_| localtime_written) {
        match k.local_time() {
            Some(reading) => note(
                k,
                journal,
                Severity::Info,
                if reading.offset_s == 0 {
                    "the C library still answers UTC for local time. That is CORRECT for a zone \
                     that is genuinely UTC-offset today, and is what a failed projection also \
                     looks like — compare the abbreviation against the zone this document names"
                } else {
                    "the C library now answers a LOCAL time for this machine, read from the \
                     /etc/localtime this projection wrote (ADR-021 §9). This is the reading, \
                     not the intention"
                },
                vec![
                    ("timezone".to_string(), Value::Str(zone.to_string())),
                    ("local".to_string(), Value::Str(reading.local.clone())),
                    (
                        "abbreviation".to_string(),
                        Value::Str(reading.abbreviation.clone()),
                    ),
                    (
                        "utc_offset_s".to_string(),
                        Value::Int(i64::from(reading.offset_s)),
                    ),
                ],
            ),
            None => note(
                k,
                journal,
                Severity::Warn,
                "/etc/localtime was written and this machine has no readable wall clock to ask \
                 the C library about, so whether the zone took effect is UNPROVEN on this boot. \
                 A Pi has no RTC (ADR-021 §2), so this is the ordinary state before the clock is \
                 restored rather than a fault",
                vec![("timezone".to_string(), Value::Str(zone.to_string()))],
            ),
        }
    }

    let read_only = match remount_read_only(k) {
        Ok(()) => true,
        Err(e) => {
            note(
                k,
                journal,
                Severity::Error,
                "DEGRADED /etc is mounted WRITABLE. ADR-007 §3.6.1 item 1 calls a genuinely \
                 read-only mount non-negotiable, and this machine does not have one: an edit \
                 to a file under /etc will now succeed, have no effect on anything, and be \
                 discarded at the next boot — the silent-data-loss row of §3.6's table",
                vec![("detail".to_string(), Value::Str(e.to_string()))],
            );
            false
        }
    };

    report_evidence(k, journal, &inputs, read_only);

    let names_a_login_shell = !inputs.login_shells.is_empty();
    if written == total {
        Projection::Rendered {
            files: total,
            read_only,
            names_a_login_shell,
        }
    } else {
        Projection::Partial {
            written,
            failed: total - written,
            read_only,
            names_a_login_shell,
        }
    }
}

/// Say what the kernel thinks is at `/etc`, and what `/etc/shells` actually
/// permits.
///
/// Both are read back rather than restated. The mount-table line is the only
/// evidence that the read-only remount took — `mount(2)` returning zero is our
/// claim about it — and `/etc/shells` is the file hardware measurement made
/// mandatory, so what it *contains* is the difference between a board that can
/// be logged into and one that cannot.
fn report_evidence(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    inputs: &EtcInputs<'_>,
    read_only: bool,
) {
    match k.read_mounts() {
        Ok(table) => match mount_table_entry(&table, ETC_MOUNT_POINT) {
            Some(line) => {
                let options = field(line, 3).unwrap_or("");
                let kernel_says_ro = options.split(',').any(|o| o == "ro");
                note(
                    k,
                    journal,
                    if kernel_says_ro == read_only {
                        Severity::Info
                    } else {
                        Severity::Warn
                    },
                    "the kernel's own mount table entry for /etc",
                    vec![
                        ("line".to_string(), Value::Str(line.to_string())),
                        ("kernel_says_read_only".to_string(), Value::Bool(kernel_says_ro)),
                        ("we_think_read_only".to_string(), Value::Bool(read_only)),
                    ],
                );
            }
            None => note(
                k,
                journal,
                Severity::Warn,
                "mount(2) succeeded but the kernel's table shows nothing at /etc",
                vec![],
            ),
        },
        Err(e) => note(
            k,
            journal,
            Severity::Warn,
            "cannot read the mount table to confirm what is at /etc",
            vec![("detail".to_string(), Value::Str(e.to_string()))],
        ),
    }

    // Two facts, not one, and separating them is the point: which shells the
    // machine will *accept* (`/etc/shells`, a membership test) and which one
    // `root` actually *gets* (`/etc/passwd`, one field). Before the
    // dual-login-shell ruling they were the same string and one record could carry both; now a
    // reader who sees only "names a login shell" would not know which shell a
    // session on this machine actually lands in.
    match inputs.login_shells.split_first() {
        Some((first, rest)) => note(
            k,
            journal,
            Severity::Info,
            "/etc/shells names a login shell, so an account will not be rejected for having \
             it",
            vec![
                ("shell".to_string(), Value::Str((*first).to_string())),
                (
                    "also_permitted".to_string(),
                    Value::Str(rest.join(" ")),
                ),
                (
                    "listed".to_string(),
                    Value::Int(i64::try_from(inputs.login_shells.len()).unwrap_or(-1)),
                ),
            ],
        ),
        None => note(
            k,
            journal,
            Severity::Warn,
            "/etc/shells names NO login shell, so every login is rejected before \
             authentication. That is correct and not a defect of this projection: none of the \
             candidate shells is on this machine. It is still the stronger statement than the \
             file being absent, which would assert that /bin/sh and /bin/csh are this \
             machine's login shells (ADR-008 §5.1.1)",
            vec![
                ("path".to_string(), Value::Str(SHELLS.to_string())),
                (
                    "would_be".to_string(),
                    Value::Str(LOGIN_SHELL_PATHS.join(" ")),
                ),
            ],
        ),
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::kernel::MOUNTS_PATH;
    use crate::testkernel::{FakeKernel, Script};
    use rivet_config::Document;
    use rivet_core::etc::{CLI_PATH, ETC_FILES, GROUP, HOSTS, NUSHELL_PATH, PASSWD, SHADOW};

    fn journal() -> Journal {
        Journal::new("test-boot".to_string(), 0, SELF_NAME.to_string())
    }

    /// A machine on which nothing is mounted yet — a first boot.
    fn fresh() -> FakeKernel {
        let mut k = FakeKernel::new();
        k.files_mut()
            .insert(MOUNTS_PATH.to_string(), Script::always(Err(Errno::ENOENT)));
        k
    }

    /// **What a DEFAULT document on a fixture machine projects**, which is
    /// ~~two~~ **three** fewer than the projection defines.
    ///
    /// `/etc/resolv.conf` is conditional on `[network.resolver]` naming a
    /// nameserver (`rivet_core::etc::RESOLV_CONF`, 2026-09-01), and
    /// `Document::default()` names none. Absent is the correct output for that
    /// machine: musl then fails a lookup fast against its own `127.0.0.1`
    /// fallback instead of answering from a file nobody configured.
    ///
    /// `/etc/ssl/certs/ca-certificates.crt` is the second, conditional on the
    /// IMAGE carrying a bundle at `CA_BUNDLE_SOURCE` — and this fixture's fake
    /// kernel has no such file, exactly as a board with no bundle would not.
    ///
    /// `/etc/localtime` is the third, added 2026-09-02 (`ADR-021` §9),
    /// conditional on the document naming a zone **and** this image carrying
    /// its tzfile. `Document::default()` names none, so the machine keeps UTC
    /// — which is what every Rivet did before the key existed.
    ///    ///
    /// Written as an expression rather than a literal so the next ruling moves
    /// it once — the reason `ETC_FILES.len()` was used here in the first place.
    /// It did move once, on the day this comment predicted it would.
    const PROJECTED_BY_DEFAULT: usize = ETC_FILES.len() - 3;

    fn run(k: &mut FakeKernel) -> Projection {
        let mut j = journal();
        // A machine with nothing installed. `project_names_every_installed_application`
        // is the run that has some.
        // Nameless: this fixture's machine has no `hostname` key and the
        // fake reads no serial, which is the state a board with neither is in.
        // `a_named_machine_...` below is the run that has one.
        project(k, &mut j, &Document::default(), &[], &crate::naming::Naming::Nameless)
    }

    fn console(k: &FakeKernel) -> String {
        k.records
            .iter()
            .map(|r| format!("{} {} {:?}", r.sev, r.msg, r.fields))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// What the projection actually put on the tmpfs at `path`, as text.
    fn written(k: &FakeKernel, path: &str) -> String {
        let w = k.writes.iter().find(|w| w.path == path);
        String::from_utf8_lossy(&w.map(|w| w.contents.clone()).unwrap_or_default()).into_owned()
    }

    /// One field of the first record whose message contains `msg`.
    ///
    /// Reads the record rather than a rendering of it, because `ADR-022` §2
    /// makes the record the primary thing — a test that greps the console text
    /// is testing the view.
    fn record_field(k: &FakeKernel, msg: &str, key: &str) -> Option<Value> {
        k.records
            .iter()
            .find(|r| r.msg.contains(msg))?
            .fields
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, v)| v.clone())
    }

    // -- the projected files -------------------------------------------------

    /// The point of the whole module, as one assertion: every path the
    /// projection defines exists on a machine where nothing had rendered them.
    ///
    /// All five paths are musl's. The count is `ETC_FILES.len()` rather than a
    /// literal so that the next ruling moves it once.
    #[test]
    fn a_first_boot_renders_every_file_the_projection_defines() {
        let mut k = fresh();
        let got = run(&mut k);
        assert_eq!(
            got,
            Projection::Rendered {
                files: PROJECTED_BY_DEFAULT,
                read_only: true,
                names_a_login_shell: false,
            }
        );
        let paths: Vec<&str> = k.writes.iter().map(|w| w.path.as_str()).collect();
        assert_eq!(paths, [GROUP, HOSTS, PASSWD, SHADOW, SHELLS]);
    }

    /// The bytes are `rivet_core`'s, not this module's. A projection that
    /// re-rendered the files here would be the second generator `ADR-007` §3.5
    /// exists to prevent.
    #[test]
    fn the_bytes_written_are_exactly_what_rivet_core_defines() {
        let mut k = fresh();
        let _ = run(&mut k);
        for file in etc_projection(&EtcInputs::default()) {
            let written = k.writes.iter().find(|w| w.path == file.path);
            assert!(written.is_some(), "{} was not written", file.path);
            assert_eq!(
                written.map(|w| w.contents.clone()),
                Some(file.contents.clone()),
                "{} does not carry rivet_core::etc's bytes",
                file.path
            );
        }
    }

    /// `/etc/shadow` is `0600` and everything else is `0644`. A `shadow`
    /// written world-readable is a defect no test of the *contents* catches.
    #[test]
    fn shadow_is_written_root_only_and_the_rest_world_readable() {
        let mut k = fresh();
        let _ = run(&mut k);
        for w in &k.writes {
            if w.path == SHADOW {
                assert_eq!(w.mode, 0o600, "{} is readable by more than root", w.path);
            } else {
                assert_eq!(w.mode, 0o644, "{} is not readable by a lookup", w.path);
            }
        }
    }

    // -- the mount -----------------------------------------------------------

    #[test]
    fn the_first_mount_is_a_tmpfs_with_the_flags_and_options_core_names() {
        let mut k = fresh();
        let _ = run(&mut k);
        let first = k.mount_calls.first();
        assert!(first.is_some(), "nothing was mounted");
        assert!(first.is_some_and(|m| m.target == ETC_MOUNT_POINT
            && m.fstype == ETC_FSTYPE
            && m.data.as_deref() == ETC_TMPFS_DATA));
        assert!(first.is_some_and(|m| m.flags.contains(MountFlags::NOEXEC)));
        assert!(first.is_some_and(|m| m.flags.contains(MountFlags::NOSUID)));
        assert!(first.is_some_and(|m| m.flags.contains(MountFlags::NODEV)));
        assert!(
            first.is_some_and(|m| !m.flags.contains(MountFlags::RDONLY)),
            "the first mount must be writable or nothing can be rendered onto it"
        );
    }

    /// `ADR-007` §3.6.1 item 1, and the ordering that makes it work: read-only
    /// **after** the writes, never before.
    #[test]
    fn the_tmpfs_is_remounted_read_only_after_everything_is_written() {
        let mut k = fresh();
        let got = run(&mut k);
        assert!(matches!(got, Projection::Rendered { read_only: true, .. }));
        let last = k.mount_calls.last();
        assert!(last.is_some_and(|m| m.flags.contains(MountFlags::RDONLY)));
        assert!(
            last.is_some_and(|m| m.flags.contains(MountFlags::REMOUNT)),
            "without MS_REMOUNT this stacks a second, empty, read-only tmpfs over the files"
        );
        // Every write happened before the remount; the fake records both in
        // order, so this is the actual ordering rather than an intention.
        assert_eq!(k.writes.len(), PROJECTED_BY_DEFAULT);
        assert_eq!(k.mount_calls.len(), 2, "{:?}", k.mount_calls);
    }

    /// The row of `ADR-007` §3.6's table that this whole mechanism exists to
    /// avoid: a writable `/etc` accepts an edit and discards it at reboot. If
    /// the remount fails, the machine is in that state and must say so.
    #[test]
    fn a_refused_read_only_remount_is_an_error_that_names_the_silent_data_loss() {
        let mut k = fresh();
        k.mount_results = Script::sequence(vec![Ok(())], Err(Errno::EPERM));
        let got = run(&mut k);
        assert!(matches!(
            got,
            Projection::Rendered {
                read_only: false,
                ..
            }
        ));
        let out = console(&k);
        assert!(out.contains("DEGRADED /etc is mounted WRITABLE"), "{out}");
        assert!(out.contains("discarded at the next boot"), "{out}");
    }

    /// tmpfs does not refuse a second mount on the same point — it stacks one,
    /// and everything the previous run wrote is hidden underneath. So a
    /// restarted supervisor must ask the kernel first.
    #[test]
    fn a_supervisor_restart_converges_onto_the_existing_tmpfs_instead_of_stacking_one() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            MOUNTS_PATH.to_string(),
            Script::always(Ok("tmpfs /etc tmpfs ro,noexec,nosuid,nodev 0 0\n".to_string())),
        );
        let got = run(&mut k);
        assert!(matches!(got, Projection::Rendered { .. }));
        // Remount-writable, then remount-read-only. Neither is a fresh mount.
        assert_eq!(k.mount_calls.len(), 2, "{:?}", k.mount_calls);
        for call in &k.mount_calls {
            assert!(
                call.flags.contains(MountFlags::REMOUNT),
                "a restart mounted a second tmpfs over /etc: {call:?}"
            );
        }
        assert!(console(&k).contains("already mounted"), "{}", console(&k));
    }

    /// The convergence path must leave the same bytes as the first-boot path,
    /// or a restart changes `/etc` underneath whatever is reading it.
    #[test]
    fn a_re_render_writes_byte_identical_files() {
        let mut first = fresh();
        let _ = run(&mut first);
        let mut again = FakeKernel::new();
        again.files_mut().insert(
            MOUNTS_PATH.to_string(),
            Script::always(Ok("tmpfs /etc tmpfs ro 0 0\n".to_string())),
        );
        let _ = run(&mut again);
        assert_eq!(first.writes, again.writes);
    }

    #[test]
    fn ebusy_from_the_kernel_is_convergence_rather_than_a_refusal() {
        let mut k = fresh();
        k.mount_results = Script::sequence(vec![Err(Errno::EBUSY)], Ok(()));
        let got = run(&mut k);
        assert!(matches!(got, Projection::Rendered { .. }), "{got:?}");
    }

    // -- the ways it can fail ------------------------------------------------

    /// Every errno a real machine can produce here, and the requirement that
    /// the console names the repair rather than the syscall.
    #[test]
    fn every_refused_mount_says_what_is_actually_wrong() {
        for (errno, must_contain) in [
            (Errno::ENOENT, "mount point does not exist"),
            (Errno::ENODEV, "CONFIG_TMPFS"),
            (Errno::EPERM, "namespace or LSM"),
            (Errno::EINVAL, "refused the options"),
        ] {
            let mut k = fresh();
            k.mount_results = Script::always(Err(errno));
            let got = run(&mut k);
            assert!(
                matches!(&got, Projection::Refused { detail } if detail.contains(must_contain)),
                "{errno} must produce a message containing {must_contain:?}, got {got:?}"
            );
            assert!(k.writes.is_empty(), "it wrote onto the read-only root");
        }
    }

    /// A refused mount is the state in which musl's fallbacks are what the
    /// machine runs on, and the console has to say what each of them costs —
    /// `ADR-008` §5.1.1's table is not something an operator has to hand.
    #[test]
    fn a_refused_mount_spells_out_what_musls_fallbacks_will_do_instead() {
        let mut k = fresh();
        k.mount_results = Script::always(Err(Errno::ENODEV));
        let _ = run(&mut k);
        let out = console(&k);
        for expected in ["getpwnam", "over DNS", "/bin/sh", "getusershell"] {
            assert!(out.contains(expected), "missing {expected:?} from: {out}");
        }
    }

    /// One file failing must not cost the others — the same rule PID 1's
    /// mount phase follows.
    #[test]
    fn one_unwritable_file_does_not_stop_the_others() {
        let mut k = fresh();
        k.write_results = Script::sequence(vec![Err(Errno::ENOSPC)], Ok(()));
        let got = run(&mut k);
        assert_eq!(
            got,
            Projection::Partial {
                written: PROJECTED_BY_DEFAULT - 1,
                failed: 1,
                read_only: true,
                names_a_login_shell: false,
            }
        );
        assert_eq!(k.writes.len(), PROJECTED_BY_DEFAULT - 1);
        assert!(console(&k).contains("tmpfs is full"), "{}", console(&k));
    }

    /// `write(2)` returning zero is our claim; the file is the kernel's. A
    /// tmpfs that truncated mid-write answers `Ok` and holds something else.
    #[test]
    fn a_file_that_reads_back_differently_is_reported_and_not_counted() {
        let mut k = fresh();
        k.writes_are_readable = false;
        k.files_mut()
            .insert(SHELLS.to_string(), Script::always(Ok("truncated".to_string())));
        let got = run(&mut k);
        assert!(
            matches!(got, Projection::Partial { written: 0, failed, .. } if failed == PROJECTED_BY_DEFAULT),
            "{got:?}"
        );
        assert!(
            console(&k).contains("read back is NOT what was written"),
            "{}",
            console(&k)
        );
    }

    // -- the login shell -----------------------------------------------------

    /// **The honest Stage 1 answer.** Nothing is at any of
    /// `LOGIN_SHELL_PATHS`, so the file names no shell — and the console says
    /// that in as many words rather than leaving a reader to notice an empty
    /// file.
    #[test]
    fn with_no_cmd_rivet_on_the_machine_shells_names_nothing_and_says_so() {
        let mut k = fresh();
        let _ = run(&mut k);
        let shells = k.writes.iter().find(|w| w.path == SHELLS);
        assert!(shells.is_some());
        let text = String::from_utf8_lossy(&shells.map(|w| w.contents.clone()).unwrap_or_default())
            .into_owned();
        let records: Vec<&str> = text
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();
        assert!(records.is_empty(), "a shell was named: {text}");
        assert!(
            console(&k).contains("names NO login shell"),
            "{}",
            console(&k)
        );
    }

    /// And the day the binary ships, the projection finds it by *looking* —
    /// nothing here has to be edited.
    ///
    /// Written against `CLI_PATH` alone rather than against both, because that
    /// is the card the ruling that makes `nu` optional promises keeps working: `nu` absent,
    /// the CLI present, and a machine that still permits a login.
    #[test]
    fn a_login_shell_that_is_present_is_found_by_probing_rather_than_by_assertion() {
        let mut k = fresh();
        k.exists_mut()
            .insert(CLI_PATH.to_string(), Script::always(Ok(true)));
        let got = run(&mut k);
        let shells = k.writes.iter().find(|w| w.path == SHELLS);
        let text = String::from_utf8_lossy(&shells.map(|w| w.contents.clone()).unwrap_or_default())
            .into_owned();
        assert!(text.contains(CLI_PATH), "{text}");
        assert!(got.permits_a_login());
        // And root's shell field follows it, or the account is unusable anyway.
        let passwd = k.writes.iter().find(|w| w.path == PASSWD);
        let text = String::from_utf8_lossy(&passwd.map(|w| w.contents.clone()).unwrap_or_default())
            .into_owned();
        assert!(text.contains(CLI_PATH), "{text}");
    }

    /// The ordering and listing rules, through the probe rather than through
    /// the renderer: with **both** binaries on the machine, `/etc/shells` names
    /// both and `/etc/passwd` names `nu`.
    ///
    /// This is the joint test `AGENTS.md` §4 asks for — the renderer and the
    /// probe are individually correct in `rivet_core::etc`'s suite, and this
    /// is the only place their agreement is checked.
    #[test]
    fn with_nu_on_the_machine_root_gets_nu_and_shells_names_both() {
        let mut k = fresh();
        k.exists_mut()
            .insert(CLI_PATH.to_string(), Script::always(Ok(true)));
        k.exists_mut()
            .insert(NUSHELL_PATH.to_string(), Script::always(Ok(true)));
        let got = run(&mut k);
        assert!(got.permits_a_login());

        let text = written(&k, SHELLS);
        assert!(text.contains(NUSHELL_PATH), "{text}");
        assert!(text.contains(CLI_PATH), "{text}");

        let text = written(&k, PASSWD);
        assert!(text.contains(&format!(":{NUSHELL_PATH}")), "{text}");
        assert!(!text.contains(CLI_PATH), "{text}");

        // And the record says which one root actually got, because "names a
        // login shell" no longer answers that on its own.
        assert_eq!(
            record_field(&k, "names a login shell", "shell"),
            Some(Value::Str(NUSHELL_PATH.to_string())),
            "{}",
            console(&k)
        );
        assert_eq!(
            record_field(&k, "names a login shell", "listed"),
            Some(Value::Int(2)),
            "{}",
            console(&k)
        );
    }

    /// A probe that could not be answered must not become a "yes". Listing a
    /// shell that is absent fails at `exec` *after* authentication, with a
    /// message that names neither the file nor the binary.
    #[test]
    fn a_probe_that_fails_names_no_shell_and_says_it_could_not_tell() {
        let mut k = fresh();
        k.exists_mut()
            .insert(CLI_PATH.to_string(), Script::always(Err(Errno::EIO)));
        let _ = run(&mut k);
        let text = written(&k, SHELLS);
        assert!(!text.contains(CLI_PATH), "{text}");
        assert!(console(&k).contains("could not tell"), "{}", console(&k));
    }

    /// One unanswerable probe must not cost the shells that *did* answer.
    ///
    /// The failure this prevents is specific: a `nu` whose probe errors while
    /// `cmd/rivet` is plainly present would, under a short-circuiting loop,
    /// produce a machine with no login shell at all — a total loss of access
    /// caused by the *optional* half of `LOGIN_SHELL_PATHS`.
    #[test]
    fn a_failed_probe_costs_only_its_own_entry() {
        let mut k = fresh();
        k.exists_mut()
            .insert(NUSHELL_PATH.to_string(), Script::always(Err(Errno::EIO)));
        k.exists_mut()
            .insert(CLI_PATH.to_string(), Script::always(Ok(true)));
        let got = run(&mut k);
        assert!(got.permits_a_login());
        let text = written(&k, PASSWD);
        assert!(text.contains(&format!(":{CLI_PATH}")), "{text}");
    }

    // -- the document --------------------------------------------------------

    /// **The guard that makes the unused `Document` honest.**
    ///
    /// `project` reads the document to build `EtcInputs`, passing what reaches
    /// `/etc` as explicit inputs (`inputs_from` no longer takes the document
    /// itself). This guard fires the moment `Document::DECLARED_KEYS` grows, so
    /// whoever adds a key has to decide whether it projects rather than
    /// discovering months later that it does not — several keys already do, and
    /// each verdict is recorded beside the key below.
    ///
    /// Injecting a fourth key into `Document::DECLARED_KEYS` breaks it; that
    /// was checked before this was committed.
    ///
    /// **It has fired repeatedly, and each verdict is recorded beside the key
    /// it is about in the list below.** The two outcomes it forces are: widen
    /// the list with a reason, or give `EtcInputs` a field and have
    /// `inputs_from` pass it on. Nothing may pass silently.
    ///
    /// **It fired on 2026-08-25** for `volumes` (`ADR-031` §7.1), and the
    /// verdict is **no**. The obvious `/etc` file for a volume is `/etc/fstab`,
    /// and Rivet has none: `ADR-005` §4 makes PID 1 the thing that mounts, from
    /// the document, with no intermediate file for a text editor to disagree
    /// with. A projected `fstab` would be a second statement of the same intent
    /// that nothing reads — the "trusted and wrong" document `ADR-006` §7
    /// refuses.
    ///
    /// **It fired again, on 2026-08-26**, for `repos` (`crate::repos`), and the
    /// verdict is **no**. There is no `/etc` file that says what a git
    /// repository is: a repository is a directory of data under `/state/git`,
    /// and `crate::repos` is the projection that makes it exist.
    #[test]
    fn the_document_contributes_nothing_and_a_new_key_must_decide_whether_it_does() {
        assert_eq!(
            Document::DECLARED_KEYS,
            [
                "version", "hostname", "network",
                // `mode` and `fleet-role` were both answered 2026-09-13 (plan
                // §5.2 item 2, `.rulings/` R-162) and the answer is NO, for
                // two different reasons. `mode` composes nothing at all in
                // `rivet-config` — see `crate::machine_mode`'s module
                // documentation — so there is nothing here for a projection
                // to act on; the actual screen-taking behaviour is still
                // `kiosk`'s, decided NO below. `fleet-role` composes
                // `[control-plane]` and `[services.<name>]` as DEFAULTS
                // (`ADR-050` §5), and both of those are themselves already NO
                // — `control-plane` for the reason given below, and a
                // service's `enabled` for the reason `ADR-017` §4 gives. A
                // key whose only effect is on two keys that do not reach
                // `/etc` reaches `/etc` itself not at all.
                "mode", "fleet-role",
                // `timezone` was answered on 2026-09-02 and the answer is YES:
                // the /etc/localtime row of `rivet_core::etc`'s own table had
                // been waiting on that key since 2026-08-23, and
                // `EtcInputs::local_time` is the answer. It is the LAST row of
                // that table a schema key could close — what is left is the
                // human half of ADR-008 §7.3, which is blocked on an
                // authorization layer rather than on a key.
                "timezone",
                // `kiosk` was answered on 2026-08-31 and the answer is NO: it
                // names an application, and no /etc file describes which
                // program holds a screen.
                "kiosk", "services", "volumes", "repos",
                // `control-plane` was answered on 2026-08-28 and the answer is
                // NO: it is a listen address, which is neither a name to
                // resolve nor an identity to project. `cmd/rivet-mcpd` reads it
                // out of the document itself, exactly as netd and mdnsd read
                // theirs, so nothing in /etc needs to know it exists.
                // `splash` was answered on 2026-08-30 and the answer is NO:
                // the boot screen is drawn by a program pid 1 starts, told what
                // to draw through flags pid 1 composes from the document. There
                // is no /etc file that describes a screen, and nothing reads one.
                "control-plane", "splash",
                // `display` was answered on 2026-09-02 and the answer is NO
                // - after a first build that said yes and was wrong.
                //
                // **The rule it broke is worth stating, because nothing had
                // stated it**: /etc on a Rivet exists for C runtimes that
                // demand an exact path. musl reads /etc/localtime and nowhere
                // else; getpwnam reads /etc/passwd; getusershell reads
                // /etc/shells. It is a COMPATIBILITY surface, which is why
                // ADR-007 §3.6.2 calls a file there a view and never an
                // interface.
                //
                // No C library looks for a display schedule. Its only reader is
                // `rivet_sdk::schedule` - Rivet's own code inside somebody
                // else's program - so it is published to /run/rivet by
                // `crate::runtime` and appears in `generated::GENERATED_PATHS`
                // rather than here. ADR-039 §3.1.
                //
                // The test for this table is therefore: NAME THE C RUNTIME
                // THAT WILL NOT LOOK ANYWHERE ELSE. If there is not one, the
                // answer is no.
                "display",
                // `applications` was answered on 2026-09-02 and the answer is
                // NO - `ADR-038` §6 and §8.
                //
                // A setting is delivered to ONE application, as a file inside
                // that application's own directory, by the supervisor. `/etc`
                // is a machine-wide namespace: putting one application's
                // configuration there would make it readable by every other,
                // and would need a name nobody has a rule for.
                //
                // And the test this table actually applies (`crate::runtime`):
                // NAME THE C RUNTIME THAT WILL NOT LOOK ANYWHERE ELSE. None
                // does. No C library has ever asked what brightness an Aura
                // renderer is set to.
                "applications",
                // `carry` was answered on 2026-09-13 (`.rulings/` R-159 R20)
                // and the answer is NO. It pins which published version of a
                // product `cmd/rivet-image --fleet-repository` must resolve,
                // and that resolution happens once, on a workstation, before
                // this machine's card is written — nothing on a BOOTED Rivet
                // ever opens a fleet repository or reads this key. There is no
                // C runtime waiting on it, and no `/etc` file would have
                // anywhere to route it to: `rivet-config::carry`'s own header
                // is the argument in full.
                "carry",
                // `dns` is NO. musl does read `/etc/hosts`, so a C runtime
                // exists that would take the table from there — and that is
                // the reason not to: the table's reader is `rivet-dnsd`, which
                // answers every device that asks it, and a copy in this
                // machine's `/etc/hosts` would be a second answer to the same
                // name, reached only by programs on this one machine and
                // stale the moment the service's view changed. A machine that
                // wants the fleet's names points `[network.resolver]` at the
                // name service (`ADR-047`).
                "dns",
            ],
            "the schema grew a key. Does it belong in /etc? hostname -> /etc/hosts \
             (ADR-025), users -> passwd/group/shadow (ADR-008 §7.3), timezone -> \
             /etc/localtime (ADR-021). If it does, EtcInputs gains a field and \
             inputs_from passes it on; if it does not, widen this list. `mode` and \
             `fleet-role` were both decided 2026-09-13 and both do NOT: `mode` composes \
             nothing at all in rivet-config, and `fleet-role` only composes \
             `control-plane` and a service's `enabled`, which are themselves both NO \
             below - see rivet-config::machine_mode and rivet-config::fleet_role. \
             `volumes` was \
             decided on 2026-08-25: it does NOT - PID 1 mounts from the document and \
             Rivet has no fstab (ADR-005 §4). `hostname` was decided on 2026-08-26: it \
             DOES - the /etc/hosts row above had been waiting on that key since \
             2026-08-23, and EtcInputs::hostname is the answer. `repos` was decided on \
             2026-08-26 too and it does NOT: a repository \
             is a directory of data under /state/git, and no /etc file says what one is. \
             `splash` was decided on 2026-08-30 and it does NOT: the boot screen is drawn \
             by a program pid 1 starts, told what to draw through flags composed from the \
             document, and no /etc file describes a screen. `kiosk` was decided on \
             2026-08-31 and it does NOT: it names which INSTALLED APPLICATION holds the \
             display, which the supervisor answers by starting one service and not \
             another. There is no /etc file that says what is on a screen, and a \
             projection of it would be a second place to look for an answer the document \
             already gives. `carry` was decided on 2026-09-13 (.rulings/ R-159 R20) and it \
             does NOT: it pins a version for cmd/rivet-image --fleet-repository to resolve, \
             once, on a workstation, before this machine's card exists - nothing on a \
             BOOTED Rivet ever reads it."
        );

        // Concretely, in both directions.
        //
        // First: two documents differing only in keys that do NOT reach /etc
        // still produce identical bytes. That is the original assertion and it
        // is unchanged — `network` is still nothing to /etc.
        let mut a = fresh();
        let mut ja = journal();
        let doc = Document::parse(
            "version = 1\n\n[network.eth0]\nmethod = \"static\"\naddress = \"10.0.0.2/24\"\n",
        )
        .expect("a valid fixture document");
        let _ = project(&mut a, &mut ja, &doc, &[], &crate::naming::Naming::Nameless);
        let mut b = fresh();
        let _ = run(&mut b);
        assert_eq!(
            a.writes, b.writes,
            "a document key reached /etc without this test being told about it"
        );

        // Second: a key that DOES reach /etc actually reaches it. A guard
        // that only ever asserts "nothing changed" would pass just as well
        // against an `inputs_from` that dropped a real field on the floor —
        // which is the failure `AGENTS.md` §6 is about. `hostname` reaching
        // `/etc/hosts` is proven concretely in `rivet_core::etc`'s own tests
        // (`a_named_machine_resolves_its_own_name_without_a_network` and
        // neighbours), because that is where the rendering lives; `timezone`
        // is proven concretely in the section beneath this one, through this
        // module's own `project` wrapper.
    }

    // -- the resolver: the document, then the DHCP lease (ADR-023) -----------

    /// The `dns=` and `domain=` of a real `bound` lease, as
    /// `cmd/rivet-dhcp-hook` writes one.
    fn lease_text(dns: &str, domain: Option<&str>) -> String {
        let mut out = String::from(
            "interface=eth0\nip=192.168.50.66\nmask=22\nrouter=192.168.50.1\nlease=7200\n",
        );
        out.push_str(&format!("dns={dns}\n"));
        if let Some(d) = domain {
            out.push_str(&format!("domain={d}\n"));
        }
        out
    }

    /// **A DHCP lease fills `/etc/resolv.conf` when the document is silent** —
    /// the lease half of `ADR-023`, and the exact case `get network` reported as
    /// `nameservers:[]` on dev-01. Without it, musl falls back to `127.0.0.1`
    /// and every external lookup fails `EAI_AGAIN`.
    #[test]
    fn a_dhcp_lease_fills_resolv_conf_when_the_document_names_no_resolver() {
        let mut k = fresh();
        k.dir_entries
            .insert(LEASE_DIR.to_string(), vec!["eth0.lease".to_string()]);
        k.files_mut().insert(
            format!("{LEASE_DIR}/eth0.lease"),
            Script::always(Ok(lease_text("192.168.50.1", Some("lan")))),
        );
        let mut j = journal();
        let _ = project(
            &mut k,
            &mut j,
            &Document::default(),
            &[],
            &crate::naming::Naming::Nameless,
        );
        let written = written(&k, rivet_core::etc::RESOLV_CONF);
        assert!(
            written.contains("nameserver 192.168.50.1"),
            "the lease's nameserver did not reach resolv.conf: {written:?}"
        );
        assert!(
            written.contains("search lan"),
            "the lease's domain did not reach resolv.conf as a search: {written:?}"
        );
    }

    /// **A resolver in the document wins over the lease, and the lease is not
    /// even read** — `ADR-011`, one authority, the document whenever it speaks.
    #[test]
    fn a_document_resolver_wins_over_the_dhcp_lease() {
        let mut k = fresh();
        k.dir_entries
            .insert(LEASE_DIR.to_string(), vec!["eth0.lease".to_string()]);
        k.files_mut().insert(
            format!("{LEASE_DIR}/eth0.lease"),
            Script::always(Ok(lease_text("192.168.50.1", None))),
        );
        let doc = Document::parse("version = 1\n\n[network.resolver]\nnameservers = [\"1.1.1.1\"]\n")
            .expect("a valid fixture document");
        let mut j = journal();
        let _ = project(&mut k, &mut j, &doc, &[], &crate::naming::Naming::Nameless);
        let written = written(&k, rivet_core::etc::RESOLV_CONF);
        assert!(
            written.contains("nameserver 1.1.1.1"),
            "the document's nameserver was not used: {written:?}"
        );
        assert!(
            !written.contains("192.168.50.1"),
            "the lease was consulted although the document named a resolver: {written:?}"
        );
        assert!(
            !k.reads.iter().any(|p| p.starts_with(LEASE_DIR)),
            "the lease directory was read although the document named a resolver: {:?}",
            k.reads
        );
    }

    /// **No document resolver and no lease projects no file at all** — musl's
    /// clean `127.0.0.1` fail-fast, exactly as before the lease half existed.
    #[test]
    fn no_resolver_and_no_lease_projects_no_resolv_conf() {
        let mut k = fresh();
        let _ = run(&mut k);
        assert!(
            !k.writes
                .iter()
                .any(|w| w.path == rivet_core::etc::RESOLV_CONF),
            "a resolv.conf was written with neither a document resolver nor a lease"
        );
    }

    /// **musl reads at most three nameservers, so a lease offering more is
    /// capped** — `rivet_config::resolver::Resolver::MAX`. A fourth written and
    /// never read is `ADR-007` §3's input-that-is-discarded.
    #[test]
    fn a_lease_offering_more_than_three_nameservers_is_capped() {
        let mut k = fresh();
        k.dir_entries
            .insert(LEASE_DIR.to_string(), vec!["eth0.lease".to_string()]);
        k.files_mut().insert(
            format!("{LEASE_DIR}/eth0.lease"),
            Script::always(Ok(lease_text("1.1.1.1 8.8.8.8 9.9.9.9 4.4.4.4", None))),
        );
        let mut j = journal();
        let _ = project(
            &mut k,
            &mut j,
            &Document::default(),
            &[],
            &crate::naming::Naming::Nameless,
        );
        let written = written(&k, rivet_core::etc::RESOLV_CONF);
        let count = written
            .lines()
            .filter(|l| l.starts_with("nameserver "))
            .count();
        assert_eq!(
            count, 3,
            "musl reads three; the fourth must be dropped: {written:?}"
        );
        assert!(
            !written.contains("4.4.4.4"),
            "the fourth nameserver was rendered: {written:?}"
        );
    }

    // -- the timezone, ADR-021 §9, ruled 2026-09-02 --------------------------

    /// A tzfile as a card carries one.
    ///
    /// **Hand-composed and labelled as such** (`AGENTS.md` §4): a real TZif
    /// header and a body of zeroes. It proves the bytes travel; it does not
    /// prove musl resolves a time from them, and nothing off a board can.
    fn a_tzfile() -> Vec<u8> {
        let mut bytes = Vec::from(*rivet_core::timezone::TZIF_MAGIC);
        bytes.extend(std::iter::repeat_n(0u8, 60));
        bytes
    }

    fn with_zone(zone: &str) -> Document {
        Document::parse(&format!("version = 1\ntimezone = \"{zone}\"\n"))
            .expect("a valid fixture document")
    }

    /// **The bytes on the card reach `/etc` unaltered.** The whole of what the
    /// projection is for: musl maps this file and nothing else, so a byte that
    /// changed on the way is a machine on a zone nobody published.
    #[test]
    fn a_document_that_names_a_zone_projects_the_tzfile_the_image_carries() {
        let mut k = fresh();
        let tz = a_tzfile();
        k.blobs.insert(
            "/system/share/zoneinfo/America/Chicago".to_string(),
            tz.clone(),
        );
        let mut j = journal();
        let _ = project(
            &mut k,
            &mut j,
            &with_zone("America/Chicago"),
            &[],
            &crate::naming::Naming::Nameless,
        );
        let write = k
            .writes
            .iter()
            .find(|w| w.path == rivet_core::etc::LOCALTIME)
            .expect("a document with a zone projects /etc/localtime");
        assert_eq!(write.contents, tz);
        assert_eq!(write.mode, rivet_core::etc::ETC_FILE_MODE);
    }

    /// **The commonest real mistake, and the schema deliberately cannot catch
    /// it.** Which zones exist is a property of the image, so a well-formed
    /// name for a zone this card has no file for is accepted by the parser and
    /// reported here — loudly, because the alternative is a UTC clock under a
    /// document that says otherwise (`ADR-034` §5.3).
    #[test]
    fn a_zone_this_image_does_not_carry_is_reported_and_the_machine_keeps_utc() {
        let mut k = fresh();
        let mut j = journal();
        let _ = project(
            &mut k,
            &mut j,
            &with_zone("Mars/Olympus_Mons"),
            &[],
            &crate::naming::Naming::Nameless,
        );
        assert!(
            !k.writes
                .iter()
                .any(|w| w.path == rivet_core::etc::LOCALTIME),
            "a zone with no file projected one anyway: {:?}",
            k.writes
        );
        let said = console(&k);
        assert!(said.contains("Mars/Olympus_Mons"), "{said}");
        assert!(said.contains("keeps UTC"), "{said}");
    }

    /// **The silent case, refused.** A file that is present and is not a tzfile
    /// would be copied, mapped by musl, rejected on its magic and replaced by
    /// UTC **without a word anywhere**. This is the one failure that looks like
    /// success at every step, so it is checked with musl's own test before the
    /// bytes go anywhere.
    #[test]
    fn a_zone_file_that_is_not_a_tzfile_is_refused_rather_than_copied() {
        let mut k = fresh();
        k.blobs.insert(
            "/system/share/zoneinfo/America/Chicago".to_string(),
            b"this is not a tzfile, it is a note somebody left".to_vec(),
        );
        let mut j = journal();
        let _ = project(
            &mut k,
            &mut j,
            &with_zone("America/Chicago"),
            &[],
            &crate::naming::Naming::Nameless,
        );
        assert!(
            !k.writes
                .iter()
                .any(|w| w.path == rivet_core::etc::LOCALTIME),
            "a file that is not a tzfile was copied into /etc: {:?}",
            k.writes
        );
        let said = console(&k);
        assert!(said.contains("NOT a tzfile"), "{said}");
    }

    /// **The supervisor asks the C library and reports what it is told.**
    ///
    /// What this test can prove and what it cannot are worth separating,
    /// because the gap is the whole of `ADR-026` §4. It proves the supervisor
    /// *asks* after writing the file and *puts the answer in the journal
    /// verbatim*. It proves nothing about whether musl would have produced that
    /// answer — the fake has no C library and no `/etc/localtime`, and a fake
    /// that invented an offset would be the "reads as working" failure this
    /// record exists to prevent. **The board settles that half.**
    #[test]
    fn the_c_librarys_own_answer_is_asked_for_and_reported_verbatim() {
        let mut k = fresh();
        k.blobs.insert(
            "/system/share/zoneinfo/America/Chicago".to_string(),
            a_tzfile(),
        );
        k.local_reading = Some(crate::kernel::LocalReading {
            offset_s: -18_000,
            abbreviation: "CST".to_string(),
            local: "2026-09-02 14:05:00".to_string(),
        });
        let mut j = journal();
        let _ = project(
            &mut k,
            &mut j,
            &with_zone("America/Chicago"),
            &[],
            &crate::naming::Naming::Nameless,
        );
        let said = console(&k);
        assert!(said.contains("CST"), "{said}");
        assert!(said.contains("-18000"), "{said}");
        assert!(said.contains("2026-09-02 14:05:00"), "{said}");
        assert!(
            said.contains("This is the reading, not the intention"),
            "{said}"
        );
    }

    /// **A zero offset is reported as ambiguous rather than as success**, which
    /// is the case the record would otherwise be a lie about: a machine on
    /// `Europe/London` in winter and a machine whose tzfile musl silently
    /// rejected produce the same number.
    #[test]
    fn a_zero_offset_says_it_looks_the_same_as_a_failure() {
        let mut k = fresh();
        k.blobs.insert(
            "/system/share/zoneinfo/Europe/London".to_string(),
            a_tzfile(),
        );
        k.local_reading = Some(crate::kernel::LocalReading {
            offset_s: 0,
            abbreviation: "GMT".to_string(),
            local: "2026-01-02 03:04:05".to_string(),
        });
        let mut j = journal();
        let _ = project(
            &mut k,
            &mut j,
            &with_zone("Europe/London"),
            &[],
            &crate::naming::Naming::Nameless,
        );
        let said = console(&k);
        assert!(said.contains("still answers UTC"), "{said}");
        assert!(said.contains("GMT"), "{said}");
    }

    /// A machine with no zone never asks, so the record cannot appear on one —
    /// which is what keeps the journal's presence meaningful.
    #[test]
    fn a_machine_with_no_zone_never_asks_the_library() {
        let mut k = fresh();
        k.local_reading = Some(crate::kernel::LocalReading {
            offset_s: -18_000,
            abbreviation: "CST".to_string(),
            local: "2026-09-02 14:05:00".to_string(),
        });
        let mut j = journal();
        let _ = project(
            &mut k,
            &mut j,
            &Document::default(),
            &[],
            &crate::naming::Naming::Nameless,
        );
        let said = console(&k);
        assert!(!said.contains("CST"), "{said}");
    }

    /// **Saying nothing is UTC and is not an error.** No file, and nothing in
    /// the journal — a machine with no opinion about time is not a machine with
    /// a problem.
    #[test]
    fn a_document_that_names_no_zone_projects_nothing_and_says_nothing() {
        let mut k = fresh();
        let mut j = journal();
        let _ = project(
            &mut k,
            &mut j,
            &Document::default(),
            &[],
            &crate::naming::Naming::Nameless,
        );
        assert!(
            !k.writes
                .iter()
                .any(|w| w.path == rivet_core::etc::LOCALTIME)
        );
        assert!(
            !k.reads
                .iter()
                .any(|r| r.starts_with(rivet_core::etc::ZONEINFO_DIR)),
            "a document with no zone read the database anyway: {:?}",
            k.reads
        );
        let said = console(&k);
        assert!(!said.contains("keeps UTC"), "{said}");
    }

    // -- the evidence --------------------------------------------------------

    /// The read-only remount is claimed by `mount(2)` and confirmed by the
    /// kernel's table. When the two disagree, the table wins and it is said.
    #[test]
    fn the_mount_table_is_read_back_and_a_disagreement_is_a_warning() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            MOUNTS_PATH.to_string(),
            // Absent for the convergence check, then present and *writable*
            // even though the remount claimed success.
            Script::sequence(
                vec![Err(Errno::ENOENT)],
                Ok("tmpfs /etc tmpfs rw,noexec,nosuid,nodev 0 0\n".to_string()),
            ),
        );
        let _ = run(&mut k);
        let out = console(&k);
        assert!(out.contains("kernel_says_read_only"), "{out}");
        assert!(out.contains("Bool(false)"), "{out}");
    }

    #[test]
    fn a_mount_table_with_nothing_at_etc_after_a_successful_mount_is_reported() {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            MOUNTS_PATH.to_string(),
            Script::sequence(
                vec![Err(Errno::ENOENT)],
                Ok("proc /proc proc rw 0 0\n".to_string()),
            ),
        );
        let _ = run(&mut k);
        assert!(
            console(&k).contains("shows nothing at /etc"),
            "{}",
            console(&k)
        );
    }

    /// The last matching line wins, because a stacked mount shadows the ones
    /// beneath it and the top one is what a path lookup reaches.
    #[test]
    fn the_mount_table_reader_takes_the_topmost_entry_and_matches_whole_fields() {
        let table = "tmpfs /etc tmpfs rw 0 0\ntmpfs /etc tmpfs ro 0 0\ntmpfs /etcetera tmpfs rw 0 0\n";
        let line = mount_table_entry(table, "/etc");
        assert_eq!(line, Some("tmpfs /etc tmpfs ro 0 0"));
        assert_eq!(mount_table_entry(table, "/et"), None);
        assert_eq!(field(line.unwrap_or(""), 3), Some("ro"));
    }

    /// A projection that rendered perfectly still does not permit a login,
    /// because there is no shell to name. "Wired up" is not "works".
    #[test]
    fn a_perfect_render_does_not_claim_a_machine_can_be_logged_into() {
        let mut k = fresh();
        let got = run(&mut k);
        assert!(matches!(got, Projection::Rendered { files, .. } if files == PROJECTED_BY_DEFAULT));
        assert!(
            !got.permits_a_login(),
            "there is no cmd/rivet on this machine; a perfect /etc does not change that"
        );
    }

    // -- the accounts an installed application brings, 2026-08-24 ------------

    /// **The end of the chain, inside this process.** A uid that no name
    /// resolves to makes `ls -l` print a number and hands a listening service a
    /// `SO_PEERCRED` caller it cannot name (`ADR-015` §5.1, §5.3). This is the
    /// test that the projection actually writes those names onto the tmpfs.
    #[test]
    fn an_installed_applications_account_reaches_the_passwd_that_is_written() {
        let mut k = fresh();
        let mut j = journal();
        let volume = rivet_core::service::BUILT_IN_VOLUME.to_string();
        let names = vec![
            (ServiceName::new("photo-index").expect("valid"), volume.clone()),
            (ServiceName::new("metrics").expect("valid"), volume),
        ];
        let got = project(
            &mut k,
            &mut j,
            &Document::default(),
            &accounts(&names),
            &crate::naming::Naming::Nameless,
        );
        assert!(matches!(got, Projection::Rendered { files, .. } if files == PROJECTED_BY_DEFAULT));

        let passwd = written(&k, rivet_core::etc::PASSWD);
        for (name, _) in &names {
            let id = rivet_core::service::app_identity(name);
            let expected = format!("{name}:x:{}:{}:", id.uid, id.gid);
            assert!(
                passwd.contains(&expected),
                "{name} is not in the /etc/passwd this machine wrote:\n{passwd}"
            );
        }
        // And the group file too, since `SO_PEERCRED` gives a gid as well.
        let group = written(&k, rivet_core::etc::GROUP);
        for (name, _) in &names {
            let id = rivet_core::service::app_identity(name);
            assert!(group.contains(&format!("{name}:x:{}:", id.gid)), "{group}");
        }
    }

    /// Only definitions with an identity become accounts. A `rivet-netd` line in
    /// `/etc/passwd` would be an account for a process that runs as root, which
    /// is a claim about the machine that is not true. **An image service with a
    /// home is not root and does get one** (`ADR-060`), with the home it runs
    /// from.
    #[test]
    fn only_definitions_with_an_identity_become_accounts() {
        let mut defs = crate::definitions::system_services();
        let before = defs.len();
        assert!(before > 0);
        // As sets: the definitions are in start order and the table in the
        // order each service moved out of root, and neither order is the claim.
        let mut homed: Vec<String> = rivet_core::service::IMAGE_SERVICE_HOMES
            .iter()
            .filter_map(|home| ServiceName::new(home.name))
            .map(|name| name.to_string())
            .collect();
        homed.sort();
        let mut accounts: Vec<String> = application_names(&defs)
            .into_iter()
            .map(|(name, _)| name.to_string())
            .collect();
        accounts.sort();
        assert_eq!(
            accounts,
            homed,
            "a system service that runs as root was given an account, or an image service \
             that runs as its own uid was not"
        );

        let app = ServiceName::new("photo-index").expect("valid");
        defs.push(crate::applications::definition(
            rivet_core::service::BUILT_IN_VOLUME,
            &app,
            &rivet_core::package::application::Manifest::new(
                rivet_core::package::application::ProgramKind::Daemon,
            ),
            &[],
        ));
        let names = application_names(&defs);
        assert_eq!(names.len(), homed.len() + 1);
        let last = names.last().expect("the application");
        assert_eq!(last.0, app);
        assert_eq!(last.1, rivet_core::service::BUILT_IN_VOLUME);
    }
}
