//! `rivet get storage` — **how full this machine's filesystems are**, and the
//! answer to a question a Rivet board could not previously be asked.
//!
//! `ADR-005` §4.1.3 ruled this on 2026-08-24, and the ruling is worth quoting
//! because it decides the *shape* rather than merely commissioning the feature:
//!
//! > **`df`** → **a Rivet operation** — a `get` subject beside `log`.
//! >
//! > free space is a **fact**, and the catalogue is already the structured
//! > surface: one implementation serves the nu table, the network API and a
//! > dashboard. A `df` binary needs a parser per consumer. **It matters now**:
//! > `/state` does not grow to fill its partition (`ADR-018` §10, unbuilt) and
//! > `ADR-022` §8 caps the journal at 8 MiB against a card whose free space
//! > nothing on the machine can measure.
//!
//! The survey behind it (`ADR-005` §4.1.2) was taken from a running machine
//! rather than from a package list: `nu` has no `df`, and **`sys disks` returned
//! `[]`**, so there was no second route to a filesystem's free space either.
//!
//! # Derived from `uutils`, and what was taken
//!
//! `ADR-005` §4.1.4 names `uucore::fsext` as worth reading and §4.1.5 rules that
//! it may not be depended on — the most minimal ask available pulls **45 crates
//! including the Mozilla Fluent localization engine**, non-optionally. §4.1.6
//! rules the remedy: *"take the algorithms, with attribution … where Rivet's
//! implementation is derived from `uutils`, the source file says so and names
//! the module it came from."*
//!
//! **Derived from `uutils`' `uucore::fsext` (MIT), specifically its `FsUsage`
//! fields and the `df` arithmetic over them.** What was taken is one paragraph
//! of accumulated knowledge, and it is the paragraph everybody gets wrong:
//!
//! > `used` is `blocks - blocks_free`, and `available` is `blocks_available`.
//! > **They do not add up to `total`.** A filesystem reserves blocks that are
//! > neither in use nor available to an ordinary caller — 5% of the partition
//! > on a default `ext4` — so computing `used` as `total - available` reports a
//! > freshly made filesystem as 5% full and drifts by that much for ever after.
//! > The percentage a reader wants is likewise `used / (used + available)`,
//! > **not** `used / total`, so that a filesystem with nothing available reads
//! > as 100% whatever the reserve is doing.
//!
//! What was **not** taken: `pretty_fstype`'s magic-number table, because this
//! implementation reads the mount table and is handed the type as a word;
//! `read_fs_list`'s own structure, which is `rivet-adapter-system`'s; and any
//! filtering of "dummy" filesystems — see below.
//!
//! # Nothing is filtered, and that is a decision
//!
//! `df` hides pseudo-filesystems by default, from a list of type names compiled
//! into it. This does not, and the reason is `AGENTS.md` §5: a hidden filter is
//! how *"is `/state` full?"* becomes unanswerable on the one machine where the
//! filter is wrong. A Rivet machine has under a dozen mounts (`ADR-005` §2.2's
//! pseudo-filesystems, `/system` as the root, `/state`, and the tmpfs `ADR-008`
//! §3 puts at `/run`), so the whole table is a readable answer — and two of the
//! ones `df` would hide, `/run` and `/tmp`, are tmpfs that a runaway service
//! genuinely fills.
//!
//! Selecting is the reader's, which is what `ADR-005` §4.1.1 ruled structured
//! output *for*:
//!
//! ```text
//! rivet get storage | from json | get filesystems | where mount-point == /state
//! rivet get storage | from json | get filesystems | each { |f| $f | update total-bytes ($f.total-bytes | into filesize) }
//! ```
//!
//! **A `?` is needed for the optional fields, and that was measured rather than
//! predicted.** Run against the emulator on 2026-08-24, `select … capacity-
//! percent` failed with `nu::shell::column_not_found` — because `/proc`, `/sys`,
//! `/sys/fs/cgroup` and `/dev/pts` all report no blocks and therefore carry no
//! capacity, and nushell's `select` refuses a column that is missing from any
//! row. `select capacity-percent?` is the form that works, and the catalogue
//! entry carries it so an agent does not have to discover it the same way:
//!
//! ```text
//! rivet get storage | from json | get filesystems | select mount-point fstype capacity-percent? holds?
//! ```
//!
//! That is the price of omitting a field rather than reporting a false `0`, and
//! it is the right way round: a pipeline that errors is one the operator fixes,
//! and a `0%` on a filesystem with no capacity is one they believe.
//!
//! # `holds`, and why the answer is not simply a row per mount
//!
//! **`/system` is not a mount point.** `ADR-008` §3's table reads as though it
//! were, and `cmd/rivet-image`'s own tree module records what is actually
//! built: the slot's `erofs` is mounted at `/` by `root=PARTLABEL=system-a`, and
//! `/system` is a **directory inside it**. So a reader looking for a `/system`
//! row would find none and conclude the operation had missed it.
//!
//! `holds` answers it instead: each filesystem carries which of Rivet's own
//! named locations it is the one holding, by longest mount point. On a board the
//! `/` row holds `/system` and the `/state` row holds the rest. Under QEMU
//! there is no `/dev/mmcblk0`, so PID 1's `/state` mount refuses and `/state` is
//! a directory on the initramfs — and the answer says exactly that, by putting
//! all four locations on the initramfs row. **That is a real answer about a real
//! filesystem and it is not a card** (`ADR-026` §4).
//!
//! # What this operation deliberately does not do
//!
//! **Take a path.** `df /some/path` is the obvious next thing and it is not
//! here, because `ADR-006` §6.4's inputs are *positional* and shared across a
//! verb's subjects: a `path` would have to occupy the position `log`'s `boot`
//! already has, which is `ADR-006` §8.5's wart paid a second time for something
//! [`holds`](OUTPUTS) already answers. Keying the catalogue by verb *and* noun
//! is what makes it free, and §6.4 says that is done by amending the ADR.
//!
//! **Report a human-readable size.** The bytes are the value; a `4.9 GiB` beside
//! them would be a second representation of one fact, which is `ADR-011`. The
//! human rendering is the *reader's* — `into filesize` in a nu pipeline, and
//! `--output text` prints the same integers the JSON carries, because
//! `ADR-006` §4 forbids the two readings disagreeing.

use super::machine::{MountedFilesystem, Observed, StorageRead, UnreadableFilesystem};
use super::{ErrorCode, Field, OperationError};
use crate::value::Value;

/// The subject. `ADR-006` §6.4's nouns are values, not catalogue keys —
/// `ADR-006` §8.5.1.
pub const SUBJECT: &str = "storage";

/// `ADR-008` §3's read-only half of the layout.
///
/// **A directory in the root filesystem, not a mount point** — see this module's
/// documentation. It is spelled here rather than read from a constant because
/// `rivet-core` has none: `crate::state` names `/state` and its children, and
/// nothing in this crate names `/system` on its own. What keeps the literal from
/// drifting is `the_system_directory_is_the_one_the_installed_binaries_are_in`,
/// which checks it against [`crate::etc::CLI_PATH`] — the path the `/etc`
/// projection puts in `/etc/shells`, and one that cannot be wrong without no
/// card being able to accept a login.
const SYSTEM_DIR: &str = "/system";

/// The locations Rivet's own design names, so an answer can say which
/// filesystem is holding each.
///
/// **Every one of these is read from the constant that owns it** (`ADR-011`),
/// except `/system` — see [`SYSTEM_DIR`]. Ordered outermost first, which is the
/// order they read in.
fn named_locations() -> [&'static str; 4] {
    [
        SYSTEM_DIR,
        crate::state::STATE_MOUNT_POINT,
        crate::state::CONFIG_DIR,
        crate::state::LOG_DIR,
    ]
}

/// How many unmeasurable entries the result quotes.
///
/// `super::log`'s number and `super::log`'s reason: a count is not actionable,
/// and five is enough to see whether they are all the same kind of wrong.
const REFUSALS_QUOTED: usize = 5;

/// What this subject's result carries.
///
/// **This is the authority.** [`super::get::subject_table`] names this list and
/// the published union is generated from it — see [`super::log::OUTPUTS`], which
/// carries the same note and the date.
pub(super) const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was read. Always `storage` in this build.",
    },
    Field {
        name: "filesystems",
        description: "One record per mounted filesystem, in mount order. NOTHING IS FILTERED — \
                      `df` hides pseudo-filesystems from a compiled-in list and this does not, \
                      because a hidden filter is how `is /state full` becomes unanswerable on \
                      the one machine where the filter is wrong. Each record carries `source`, \
                      `mount-point`, `fstype`, `read-only`, `total-bytes`, `used-bytes`, \
                      `available-bytes`, `capacity-percent`, `inodes`, `inodes-used`, \
                      `inodes-free`, and `holds` — the last being which of Rivet's own \
                      locations this filesystem is the one holding. `capacity-percent` and the \
                      inode fields are ABSENT on a filesystem that has no such number, rather \
                      than reported as 0; `inodes-used` is additionally absent where the \
                      machine's own two inode counts do not support subtracting them, which \
                      real filesystems do report. In nushell: \
                      `rivet get storage | from json | get filesystems | where mount-point == /state`. \
                      SELECT THE OPTIONAL FIELDS WITH A `?` — `select mount-point capacity-percent?` \
                      — because `select` on a column that is absent from any row is an error in \
                      nushell, and `/proc` is such a row on every machine.",
    },
    Field {
        name: "count",
        description: "How many filesystems are in `filesystems`.",
    },
    Field {
        name: "unreadable",
        description: "Entries of the mount table whose usage could not be measured. NOT dropped \
                      silently: an answer that omitted one would say THERE IS NO SUCH \
                      FILESYSTEM, which is the one thing this must not say falsely \
                      (AGENTS.md §5).",
    },
    Field {
        name: "unreadable-detail",
        description: "Up to five of those, by mount point and what the system said. Absent when \
                      `unreadable` is 0.",
    },
    Field {
        name: "mount-table",
        description: "Where this machine's mount table was read from. Reported even when it was \
                      empty, so an empty answer names a place rather than being a shrug.",
    },
];

/// The fields of one filesystem's record, and the meaning of each.
///
/// **Not a `Field` list**, because `super::Operation` describes an operation's
/// top-level result and `ADR-006` §7 gives it no way to describe a nested one.
/// That is a real limitation rather than a shortcut: an agent reading the
/// catalogue learns that `filesystems` exists and learns what is in it only from
/// the prose of that entry's description, which is why the description above
/// lists the field names. Widening [`Field`] to nest is a change to §7's shape
/// and therefore the maintainer's.
///
/// The values here are the record keys, and they are what
/// `every_record_key_is_named_in_the_catalogue_description` checks the
/// description against — so the two cannot drift.
pub(super) const RECORD_KEYS: &[&str] = &[
    "source",
    "mount-point",
    "fstype",
    "read-only",
    "total-bytes",
    "used-bytes",
    "available-bytes",
    "capacity-percent",
    "inodes",
    "inodes-used",
    "inodes-free",
    "holds",
];

/// Perform `get storage`. The subject has already matched — [`super::get::
/// perform`] dispatches.
pub(super) fn perform(
    arguments: &[&str],
    observed: &mut dyn Observed,
) -> Result<Value, OperationError> {
    // `get`'s second and third positions are `log`'s `boot` and `limit`
    // (`ADR-006` §8.5's wart, paid once). This subject takes neither, and an
    // extra word is refused rather than ignored — an operator who typed
    // `rivet get storage /state` expecting `df /state` must be told that it
    // was not read, not shown the whole machine as though it had been.
    if let Some(extra) = arguments.get(1) {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            *extra,
            format!(
                "`{SUBJECT}` takes no further words. `boot` and `limit` are `{}`'s. This \
                 operation reports EVERY mounted filesystem and each record says which of \
                 Rivet's own locations it holds, so select in the shell: \
                 `rivet get storage | from json | get filesystems | where mount-point == \
                 {extra}`.",
                super::log::SUBJECT,
            ),
        ));
    }

    let read: StorageRead = observed
        .read_filesystems()
        .map_err(|e| OperationError::about(ErrorCode::MachineRefused, SUBJECT, e.to_string()))?;

    let holdings = holdings(&read.filesystems, &read.unreadable);

    let mut fields = vec![
        ("subject", Value::str(SUBJECT)),
        (
            "filesystems",
            Value::List(
                read.filesystems
                    .iter()
                    .zip(&holdings)
                    .map(|(fs, holds)| record(fs, holds))
                    .collect(),
            ),
        ),
        ("count", count(read.filesystems.len())),
        ("unreadable", count(read.unreadable.len())),
    ];
    if !read.unreadable.is_empty() {
        fields.push((
            "unreadable-detail",
            Value::strings(
                read.unreadable
                    .iter()
                    .take(REFUSALS_QUOTED)
                    .map(|u| format!("{}: {}", u.mount_point, u.why)),
            ),
        ));
    }
    fields.push(("mount-table", Value::str(read.mount_table)));
    debug_assert!(
        super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)),
        "a storage result's keys must be a subsequence of OUTPUTS, in the declared order"
    );
    Ok(Value::object(fields))
}

/// One filesystem, as the record `RECORD_KEYS` describes.
fn record(fs: &MountedFilesystem, holds: &[&'static str]) -> Value {
    let mut fields = vec![
        ("source", Value::str(fs.source.clone())),
        ("mount-point", Value::str(fs.mount_point.clone())),
        ("fstype", Value::str(fs.fstype.clone())),
        ("read-only", Value::Bool(fs.read_only)),
        ("total-bytes", count(total_bytes(fs))),
        ("used-bytes", count(used_bytes(fs))),
        ("available-bytes", count(available_bytes(fs))),
    ];
    // Omitted rather than reported as 0 when there is nothing to be a percentage
    // OF — a `/proc` reports no blocks at all, and "0% used" would be an answer
    // about a filesystem that has no capacity to use. The absent-key convention
    // is `crate::value`'s: there is no null, so an absent field is how a value
    // says it does not have one.
    if let Some(percent) = capacity_percent(fs) {
        fields.push(("capacity-percent", count(percent)));
    }
    // The same, for the filesystems that have no fixed number of inodes. Zero
    // there means NOT APPLICABLE and not NONE LEFT, and emitting `inodes: 0`
    // beside `inodes-free: 0` would read as the second.
    if fs.inodes > 0 {
        fields.push(("inodes", count(fs.inodes)));
        // **`inodes-used` is omitted when the machine's own two numbers do not
        // support it**, which is not hypothetical: a 9p filesystem read on
        // 2026-08-24 reported 999 inodes and 1 000 000 free. Saturating the
        // subtraction would print `inodes-used: 0` beside those two and look
        // like an answer; refusing to derive it, while still reporting both
        // numbers the machine gave, is `AGENTS.md` §5 — report what the system
        // said, and do not manufacture what it did not.
        if fs.inodes_free <= fs.inodes {
            fields.push(("inodes-used", count(fs.inodes - fs.inodes_free)));
        }
        fields.push(("inodes-free", count(fs.inodes_free)));
    }
    if !holds.is_empty() {
        fields.push(("holds", Value::strings(holds.iter().copied())));
    }
    debug_assert!(
        super::get::in_declared_order(&fields, RECORD_KEYS.iter().copied()),
        "a filesystem record's keys must be a subsequence of RECORD_KEYS, in the declared order"
    );
    Value::object(fields)
}

/// A count, as the only integer [`Value`] has.
///
/// `Value::Integer` is `i64` and every number here is a `u64` (`crate::value`
/// explains why the enum has one integer and it is signed). A filesystem large
/// enough to overflow an `i64` of bytes is eight exabytes and not a Raspberry
/// Pi's card; saturating rather than wrapping is what makes the failure
/// direction "implausibly large" rather than "negative".
fn count(n: impl TryInto<i64>) -> Value {
    Value::Integer(n.try_into().unwrap_or(i64::MAX))
}

/// The whole filesystem, in bytes.
fn total_bytes(fs: &MountedFilesystem) -> u64 {
    fs.blocks.saturating_mul(fs.block_bytes)
}

/// What is in use, in bytes — **`blocks - blocks_free`**, not
/// `total - available`. See this module's documentation for what the difference
/// is and why the naive form is wrong.
fn used_bytes(fs: &MountedFilesystem) -> u64 {
    fs.blocks
        .saturating_sub(fs.blocks_free)
        .saturating_mul(fs.block_bytes)
}

/// What an ordinary caller may still use, in bytes.
fn available_bytes(fs: &MountedFilesystem) -> u64 {
    fs.blocks_available.saturating_mul(fs.block_bytes)
}

/// How full it is, as a percentage **rounded up**, or `None` when there is
/// nothing to be a percentage of.
///
/// `used / (used + available)`, which is `df`'s and not `used / total`. Rounded
/// up so that a filesystem with any space in use never reads as `0`, which is
/// the rounding a capacity figure wants: the direction that misleads is the one
/// that says a full filesystem has room.
fn capacity_percent(fs: &MountedFilesystem) -> Option<u64> {
    let used = u128::from(fs.blocks.saturating_sub(fs.blocks_free));
    let available = u128::from(fs.blocks_available);
    let denominator = used + available;
    if denominator == 0 {
        return None;
    }
    let percent = (used * 100).div_ceil(denominator);
    u64::try_from(percent).ok()
}

/// Which of Rivet's named locations each filesystem holds, in the same order
/// as the filesystems.
fn holdings(
    filesystems: &[MountedFilesystem],
    unreadable: &[UnreadableFilesystem],
) -> Vec<Vec<&'static str>> {
    let mut holdings: Vec<Vec<&'static str>> = vec![Vec::new(); filesystems.len()];
    for location in named_locations() {
        if let Some(index) = holder(filesystems, unreadable, location) {
            holdings[index].push(location);
        }
    }
    holdings
}

/// Which filesystem a path is actually on: the one with the **longest** mount
/// point that contains it, and the **last** such one when two share it —
/// **among filesystems this reading could actually measure.**
///
/// All three halves are load-bearing.
///
/// *Longest* is what makes `/state/log` land on `/state` rather than on `/`, and
/// a shortest-match or first-match rule would report the root filesystem's free
/// space as the journal's — which on a board is a different partition and a
/// different order of magnitude.
///
/// *Last* is because a mount table is a history: mounting a filesystem over an
/// existing mount point does not remove the old entry, it shadows it. A path
/// reaches the most recent one, so an implementation taking the first match
/// would name a filesystem nothing can currently write to. `StorageRead::
/// filesystems` documents that its order is the table's for exactly this reason.
///
/// *Among the readable ones* because `unreadable` is checked too, and a longest
/// match found there means no readable filesystem holds the path at all —
/// `None`, not the next-longest readable ancestor. `/state` failing to read
/// must not make `/` claim `/state/log`'s capacity and read-only state as its
/// own; that is a different filesystem's numbers attributed to the wrong mount
/// (`AGENTS.md` §5). An unreadable mount tying a readable one's length is the
/// same path reported once each way, so the tie also shadows: `>=`, not `>`.
fn holder(
    filesystems: &[MountedFilesystem],
    unreadable: &[UnreadableFilesystem],
    path: &str,
) -> Option<usize> {
    let mut best: Option<(usize, usize)> = None;
    for (index, fs) in filesystems.iter().enumerate() {
        if !contains(&fs.mount_point, path) {
            continue;
        }
        let length = fs.mount_point.len();
        if best.is_none_or(|(_, longest)| length >= longest) {
            best = Some((index, length));
        }
    }
    let longest_unreadable = unreadable
        .iter()
        .filter(|u| contains(&u.mount_point, path))
        .map(|u| u.mount_point.len())
        .max();
    if let Some(longest_unreadable) = longest_unreadable
        && best.is_none_or(|(_, longest_readable)| longest_unreadable >= longest_readable)
    {
        return None;
    }
    best.map(|(index, _)| index)
}

/// Whether a mount point contains a path, **component-wise**.
///
/// `/state` contains `/state/log` and does not contain `/stateful`, which a
/// plain `starts_with` would get wrong. `/` contains everything absolute, which
/// is the case that has to be special because it is the only mount point ending
/// in a separator.
///
/// The same rule `rivet_core::mount`'s
/// `stage0_plan_mounts_a_parent_before_anything_inside_it` applies, which called
/// out this exact hazard: *"`/sys` contains `/sys/fs/cgroup` but not `/system`,
/// which is a different filesystem entirely"*.
fn contains(mount_point: &str, path: &str) -> bool {
    if mount_point == path {
        return true;
    }
    if mount_point == "/" {
        return path.starts_with('/');
    }
    path.starts_with(&format!("{mount_point}/"))
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::operation::machine::{MachineError, MemoryRead, UnreadableFilesystem};

    /// A machine whose mount table is whatever the test says it is.
    struct FakeStorage {
        read: Result<StorageRead, MachineError>,
        reads: usize,
    }

    impl FakeStorage {
        fn holding(read: StorageRead) -> Self {
            Self {
                read: Ok(read),
                reads: 0,
            }
        }

        fn refusing(why: &str) -> Self {
            Self {
                read: Err(MachineError::Refused(why.to_string())),
                reads: 0,
            }
        }
    }

    /// The boot-slot and shutdown stubs this used to carry are gone: since
    /// 2026-08-24 `get storage` takes `&mut dyn Observed` and cannot reach
    /// them, so there is nothing left to stub.
    impl Observed for FakeStorage {
        /// **Refuses rather than answering that this machine trusts nobody.**
        /// An empty trust store is a REAL reading -- it is what `ADR-028` §5.5.5's
        /// `--no-trust` image produces -- and this fake has not taken it. The
        /// direction is the dangerous one: *"trusts nobody"* is the answer
        /// somebody acts on by re-signing an artifact.
        fn read_trust(&mut self) -> Result<crate::operation::machine::TrustRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering an idle machine.** Zeroed counters
        /// are what a machine that has run nothing since it booted looks like,
        /// and a load average of zero is what an idle one looks like — both are
        /// readings, and this fake has taken neither (`AGENTS.md` §3, §5).
        fn read_cpu(&mut self) -> Result<crate::operation::machine::CpuRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering that this machine has no block
        /// devices**, which is not a state a machine that booted off one can
        /// be in.
        fn read_devices(&mut self) -> Result<crate::operation::machine::DeviceRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering an empty network.** No interfaces at
        /// all reads as a machine with none, which is not a state a running
        /// Linux is in.
        fn read_network(&mut self) -> Result<crate::operation::machine::NetworkRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than naming a slot.** Naming the wrong one is how
        /// an update overwrites the running system, so a fake that observes
        /// nothing names none.
        fn read_slots(&mut self) -> Result<crate::operation::machine::SlotRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering an uptime of zero.** `0` would say
        /// this machine booted a moment ago, which is a reading nobody took.
        fn read_boot(&mut self) -> Result<crate::operation::machine::BootRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering with unset flags.** Reporting a
        /// `Throttle` of all-`false` would assert that this machine has never
        /// been under-volted since it booted — the one claim this subject must
        /// never make without a measurement (`AGENTS.md` §3, §5).
        fn read_thermal(
            &mut self,
        ) -> Result<crate::operation::machine::ThermalRead, MachineError> {
            Err(MachineError::Refused(
                    "not this fake's port; unread throttle flags are never reported as false"
                        .to_string(),
            ))
        }
    fn read_processes(
        &mut self,
    ) -> Result<crate::operation::machine::ProcessRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_memory(&mut self) -> Result<MemoryRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_desired_state(&mut self) -> Result<Option<String>, MachineError> {
        Ok(None)
    }
        /// `ADR-032` §3.2's query socket, added 2026-08-26. This fake asks no
        /// supervisor and refuses rather than pretending (`AGENTS.md` §3).
        fn supervisor_status(
            &mut self,
        ) -> Result<crate::operation::status::Status, MachineError> {
            Err(MachineError::Refused(
                "this fake answers no supervisor; an operation that reached this port has changed"
                    .to_string(),
            ))
        }

        /// The fake supplies no schema: `rivet-core` cannot name one
        /// (`ADR-014` §3.2), so a test that needs the real document belongs in
        /// the adapter that has both crates. Loud rather than empty, so an
        /// assertion against real schema content fails instead of passing.
        fn configuration_schema(&mut self) -> Value {
            Value::str("<no schema: this is a test fake>")
        }

        fn read_filesystems(&mut self) -> Result<StorageRead, MachineError> {
            self.reads += 1;
            self.read.clone()
        }

        /// get storage must not read the journal
        fn read_journal(&mut self) -> Result<crate::operation::JournalRead, MachineError> {
            panic!("get storage must not read the journal")
        }
    }

    fn mounted(source: &str, at: &str, fstype: &str) -> MountedFilesystem {
        MountedFilesystem {
            source: source.to_string(),
            mount_point: at.to_string(),
            fstype: fstype.to_string(),
            ..MountedFilesystem::default()
        }
    }

    /// One 4 KiB-block filesystem, sized in blocks so the arithmetic is legible.
    fn sized(
        mut base: MountedFilesystem,
        blocks: u64,
        free: u64,
        available: u64,
    ) -> MountedFilesystem {
        base.block_bytes = 4096;
        base.blocks = blocks;
        base.blocks_free = free;
        base.blocks_available = available;
        base
    }

    /// **A card, as `ADR-018` §4 lays one out**: the slot's `erofs` at `/` with
    /// `/system` inside it, and `/state` as its own `ext4` partition.
    fn a_card() -> StorageRead {
        StorageRead {
            filesystems: vec![
                sized(mounted("/dev/mmcblk0p3", "/", "erofs"), 1000, 0, 0),
                sized(mounted("devtmpfs", "/dev", "devtmpfs"), 0, 0, 0),
                // 5% reserved, which is the case the arithmetic is about:
                // 1000 blocks, 900 free, 850 available.
                sized(mounted("/dev/mmcblk0p5", "/state", "ext4"), 1000, 900, 850),
                sized(mounted("tmpfs", "/run", "tmpfs"), 100, 100, 100),
            ],
            unreadable: Vec::new(),
            mount_table: "/proc/self/mounts".to_string(),
        }
    }

    fn run(read: StorageRead) -> Value {
        let mut m = FakeStorage::holding(read);
        perform(&[SUBJECT], &mut m).unwrap_or_else(|e| panic!("get storage failed: {e}"))
    }

    fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
        match v {
            Value::Object(fields) => fields
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v)
                .unwrap_or_else(|| panic!("no {key} in {v:?}")),
            other => panic!("not an object: {other:?}"),
        }
    }

    fn maybe<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
        match v {
            Value::Object(fields) => fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v),
            other => panic!("not an object: {other:?}"),
        }
    }

    fn rows(v: &Value) -> &[Value] {
        match field(v, "filesystems") {
            Value::List(items) => items,
            other => panic!("filesystems is not a list: {other:?}"),
        }
    }

    fn row<'a>(v: &'a Value, mount_point: &str) -> &'a Value {
        rows(v)
            .iter()
            .find(|r| field(r, "mount-point") == &Value::str(mount_point))
            .unwrap_or_else(|| panic!("no row for {mount_point} in {v:?}"))
    }

    // -- the arithmetic, which is the part that is worth getting right --------

    /// **The `uutils`-derived rule, asserted as a number.** 1000 blocks of
    /// 4 KiB, 900 free, 850 available to an ordinary caller: 100 blocks are in
    /// use and 50 are the filesystem's reserve.
    ///
    /// The naive `total - available` would report 150 blocks used — 600 KiB
    /// that nothing wrote — and would do so on a freshly made `ext4` for ever.
    #[test]
    fn used_is_blocks_minus_free_and_not_total_minus_available() {
        let fs = sized(mounted("/dev/mmcblk0p5", "/state", "ext4"), 1000, 900, 850);
        assert_eq!(total_bytes(&fs), 1000 * 4096);
        assert_eq!(used_bytes(&fs), 100 * 4096);
        assert_eq!(available_bytes(&fs), 850 * 4096);
        // And the three do not add up, which is the whole point.
        assert_ne!(
            used_bytes(&fs) + available_bytes(&fs),
            total_bytes(&fs),
            "the reserve has gone missing; this is the assertion the naive form fails"
        );
    }

    /// The percentage is of what a caller can reach, not of the partition —
    /// again `df`'s rule. 100 used against 950 usable is 11%, where
    /// `used / total` would say 10%.
    #[test]
    fn capacity_is_of_used_plus_available_rather_than_of_the_whole_partition() {
        let fs = sized(mounted("/dev/mmcblk0p5", "/state", "ext4"), 1000, 900, 850);
        assert_eq!(capacity_percent(&fs), Some(11));
    }

    /// Rounded **up**, so that a filesystem with something in it never reads as
    /// empty. The direction that misleads is the one saying a full filesystem
    /// has room.
    #[test]
    fn a_single_used_block_in_a_large_filesystem_reads_as_one_percent_and_not_zero() {
        let fs = sized(
            mounted("/dev/mmcblk0p5", "/state", "ext4"),
            1_000_000,
            999_999,
            999_999,
        );
        assert_eq!(capacity_percent(&fs), Some(1));
    }

    /// And a filesystem with nothing available is 100% however large the
    /// reserve is — the answer an operator needs, since nothing more can be
    /// written to it.
    #[test]
    fn a_filesystem_with_nothing_available_is_a_hundred_percent() {
        let fs = sized(mounted("/dev/mmcblk0p5", "/state", "ext4"), 1000, 50, 0);
        assert_eq!(capacity_percent(&fs), Some(100));
    }

    /// A filesystem with no blocks at all — `/proc`, `/sys` — has no capacity
    /// to be a percentage of, and the field is **absent** rather than `0`.
    /// `crate::value` has no null, so absence is how a value says it has none.
    #[test]
    fn a_filesystem_with_no_blocks_omits_the_capacity_rather_than_reporting_zero_percent() {
        let answer = run(a_card());
        let dev = row(&answer, "/dev");
        assert_eq!(field(dev, "total-bytes"), &Value::Integer(0));
        assert_eq!(maybe(dev, "capacity-percent"), None);
        // ...and one that HAS blocks carries it.
        assert!(maybe(row(&answer, "/state"), "capacity-percent").is_some());
    }

    /// Zero inodes means *this filesystem does not have a fixed number of
    /// them*, not *none are left*. Reported as absence for the same reason.
    #[test]
    fn a_filesystem_with_no_inode_limit_omits_the_inode_fields_entirely() {
        let mut read = a_card();
        read.filesystems[2].inodes = 64;
        read.filesystems[2].inodes_free = 60;
        let answer = run(read);

        let state = row(&answer, "/state");
        assert_eq!(field(state, "inodes"), &Value::Integer(64));
        assert_eq!(field(state, "inodes-used"), &Value::Integer(4));
        assert_eq!(field(state, "inodes-free"), &Value::Integer(60));

        let run_row = row(&answer, "/run");
        assert_eq!(maybe(run_row, "inodes"), None);
        assert_eq!(maybe(run_row, "inodes-used"), None);
    }

    /// **A real filesystem reports two inode counts that cannot both be true**,
    /// and this is the vector from the machine that produced it: a `9p`
    /// filesystem, read on 2026-08-24 while verifying this operation under WSL,
    /// answered `f_files = 999` and `f_ffree = 1000000`.
    ///
    /// It is labelled by provenance (`AGENTS.md` §4) because that is what makes
    /// it worth keeping: nobody would compose it, and the first version of this
    /// operation saturated the subtraction and printed `inodes-used: 0` beside
    /// the two numbers — an answer, and a false one. What is reported now is
    /// both numbers the machine gave and **no derived third**.
    #[test]
    fn a_filesystem_reporting_more_free_inodes_than_inodes_gets_no_derived_used_count() {
        let mut read = a_card();
        read.filesystems[3].inodes = 999;
        read.filesystems[3].inodes_free = 1_000_000;
        let answer = run(read);

        let odd = row(&answer, "/run");
        assert_eq!(field(odd, "inodes"), &Value::Integer(999));
        assert_eq!(field(odd, "inodes-free"), &Value::Integer(1_000_000));
        assert_eq!(
            maybe(odd, "inodes-used"),
            None,
            "a used count was derived from two numbers that cannot both be true"
        );
    }

    // -- which filesystem holds what ------------------------------------------

    /// **The question this operation exists to answer**, on the layout a card
    /// actually has: `/system` is a directory in the root filesystem and
    /// `/state` is its own partition.
    #[test]
    fn on_a_card_the_root_holds_system_and_the_state_partition_holds_the_rest() {
        let answer = run(a_card());
        assert_eq!(
            field(row(&answer, "/"), "holds"),
            &Value::strings(["/system"])
        );
        assert_eq!(
            field(row(&answer, "/state"), "holds"),
            &Value::strings(["/state", "/state/config", "/state/log"])
        );
        // A filesystem holding none of them says nothing rather than carrying
        // an empty list.
        assert_eq!(maybe(row(&answer, "/run"), "holds"), None);
    }

    /// **`ADR-026` §4's trap, asserted rather than described.** Under QEMU there
    /// is no `/dev/mmcblk0`, so `/state` never mounts and is a directory on the
    /// initramfs — and the honest answer is that the root filesystem holds all
    /// four locations. A reader who sees this knows the numbers are RAM.
    #[test]
    fn with_no_state_partition_mounted_the_root_filesystem_holds_every_location() {
        let read = StorageRead {
            filesystems: vec![sized(mounted("rootfs", "/", "rootfs"), 1000, 500, 500)],
            unreadable: Vec::new(),
            mount_table: "/proc/self/mounts".to_string(),
        };
        let answer = run(read);
        assert_eq!(
            field(row(&answer, "/"), "holds"),
            &Value::strings(["/system", "/state", "/state/config", "/state/log"])
        );
    }

    /// Longest match, not first: `/state/log` is on `/state` even though `/`
    /// contains it too. A shortest-match rule would report the root's free
    /// space as the journal's, which on a board is a different partition.
    #[test]
    fn a_location_lands_on_the_longest_mount_point_that_contains_it() {
        let filesystems = a_card().filesystems;
        assert_eq!(
            holder(&filesystems, &[], "/state/log").map(|i| filesystems[i].mount_point.clone()),
            Some("/state".to_string())
        );
        assert_eq!(
            holder(&filesystems, &[], "/system/bin").map(|i| filesystems[i].mount_point.clone()),
            Some("/".to_string())
        );
    }

    /// **Component-wise containment.** `/state` does not contain `/stateful`,
    /// and a plain `starts_with` says it does — the hazard
    /// `rivet_core::mount`'s own ordering test names.
    #[test]
    fn a_mount_point_does_not_contain_a_path_that_merely_starts_with_its_name() {
        assert!(contains("/state", "/state"));
        assert!(contains("/state", "/state/log"));
        assert!(!contains("/state", "/stateful"));
        assert!(!contains("/sys", "/system"));
        assert!(contains("/", "/anything"));
        assert!(contains("/", "/"));
    }

    /// A mount over an existing mount point shadows it, and a path reaches the
    /// later one. Taking the first match would name a filesystem nothing can
    /// currently write to.
    #[test]
    fn when_two_filesystems_share_a_mount_point_the_later_one_is_the_one_a_path_reaches() {
        let filesystems = vec![
            sized(mounted("first", "/state", "ext4"), 1000, 900, 850),
            sized(mounted("second", "/state", "tmpfs"), 10, 10, 10),
        ];
        assert_eq!(
            holder(&filesystems, &[], "/state/log").map(|i| filesystems[i].source.clone()),
            Some("second".to_string())
        );
    }

    /// A machine whose table contains nothing containing a location — no root
    /// mount at all — attributes it to nothing rather than to the first entry.
    #[test]
    fn a_location_no_filesystem_contains_is_held_by_none_of_them() {
        let filesystems = vec![sized(mounted("tmpfs", "/run", "tmpfs"), 100, 100, 100)];
        assert_eq!(holder(&filesystems, &[], "/state"), None);
        assert_eq!(holdings(&filesystems, &[]), vec![Vec::<&str>::new()]);
    }

    // -- the answer's shape ---------------------------------------------------

    #[test]
    fn every_mounted_filesystem_is_reported_in_the_order_the_table_listed_them() {
        let answer = run(a_card());
        let points: Vec<&Value> = rows(&answer)
            .iter()
            .map(|r| field(r, "mount-point"))
            .collect();
        assert_eq!(
            points,
            vec![
                &Value::str("/"),
                &Value::str("/dev"),
                &Value::str("/state"),
                &Value::str("/run"),
            ]
        );
        assert_eq!(field(&answer, "count"), &Value::Integer(4));
        assert_eq!(field(&answer, "subject"), &Value::str(SUBJECT));
        assert_eq!(
            field(&answer, "mount-table"),
            &Value::str("/proc/self/mounts")
        );
    }

    /// **Nothing is filtered.** `df` would hide `devtmpfs` and this must not,
    /// because a hidden filter is how the one filesystem an operator needs
    /// disappears.
    #[test]
    fn a_pseudo_filesystem_df_would_hide_is_still_in_the_answer() {
        let answer = run(a_card());
        assert_eq!(
            field(row(&answer, "/dev"), "fstype"),
            &Value::str("devtmpfs")
        );
    }

    /// Every key the record carries is one `RECORD_KEYS` declares, and in that
    /// order — so the catalogue's prose and the value cannot drift apart.
    #[test]
    fn a_record_carries_only_declared_keys_and_in_the_declared_order() {
        let mut read = a_card();
        read.filesystems[2].inodes = 64;
        read.filesystems[2].inodes_free = 60;
        let answer = run(read);
        for row in rows(&answer) {
            let Value::Object(fields) = row else {
                panic!("a filesystem is not an object: {row:?}");
            };
            let mut at = 0usize;
            for (key, _) in fields {
                let found = RECORD_KEYS
                    .iter()
                    .skip(at)
                    .position(|k| k == key)
                    .unwrap_or_else(|| {
                        panic!("`{key}` is not a declared record key after position {at}")
                    });
                at += found + 1;
            }
        }
    }

    /// The catalogue is what an agent reads instead of documentation (`ADR-006`
    /// §7), and `filesystems` is a nested value §7 cannot describe structurally
    /// — so its description carries the field names, and this is what keeps
    /// them right.
    #[test]
    fn every_record_key_is_named_in_the_catalogue_description() {
        let described = OUTPUTS
            .iter()
            .find(|f| f.name == "filesystems")
            .expect("filesystems is a declared output")
            .description;
        for key in RECORD_KEYS {
            assert!(
                described.contains(key),
                "`{key}` is not described: {described}"
            );
        }
        // And the pipeline that makes the answer usable, for `log`'s reason.
        assert!(described.contains("from json"), "{described}");
    }

    // -- honesty --------------------------------------------------------------

    /// **A filesystem that could not be measured is counted and quoted, never
    /// dropped.** An answer that omitted one would say *there is no such
    /// filesystem*, which is the failure `AGENTS.md` §5 is about.
    #[test]
    fn a_filesystem_that_could_not_be_measured_is_reported_rather_than_omitted() {
        let mut read = a_card();
        read.unreadable.push(UnreadableFilesystem {
            mount_point: "/proc/sys/fs/binfmt_misc".to_string(),
            why: "errno 2 (No such file or directory)".to_string(),
        });
        let answer = run(read);
        assert_eq!(field(&answer, "unreadable"), &Value::Integer(1));
        let detail = format!("{:?}", maybe(&answer, "unreadable-detail"));
        assert!(detail.contains("binfmt_misc"), "{detail}");
        assert!(detail.contains("errno 2"), "{detail}");
    }

    /// **An unreadable `/state` must not make `/` claim it, or anything
    /// under it, as its own.** Before this, `holder` looked only at
    /// `StorageRead::filesystems`, so the longest READABLE mount point won
    /// regardless of whether a longer, unreadable one existed — and
    /// `/state`'s capacity and read-only state would have been reported as
    /// `/`'s.
    #[test]
    fn an_unreadable_state_mount_is_held_by_no_readable_filesystem() {
        let read = StorageRead {
            filesystems: vec![sized(mounted("rootfs", "/", "rootfs"), 1000, 500, 500)],
            unreadable: vec![UnreadableFilesystem {
                mount_point: "/state".to_string(),
                why: "errno 5 (Input/output error)".to_string(),
            }],
            mount_table: "/proc/self/mounts".to_string(),
        };
        let answer = run(read);
        // Only /system is left for the root filesystem to hold: /state and
        // everything under it belongs to the mount that could not be measured.
        assert_eq!(field(row(&answer, "/"), "holds"), &Value::strings(["/system"]));
        assert_eq!(field(&answer, "unreadable"), &Value::Integer(1));
        let detail = format!("{:?}", maybe(&answer, "unreadable-detail"));
        assert!(detail.contains("/state"), "{detail}");
    }

    /// **The shadow is as narrow as the unreadable mount, not its whole
    /// ancestor.** An unreadable `/state/log` must not swallow
    /// `/state/config` too — only the location the unreadable mount actually
    /// contains is taken away from its readable parent.
    #[test]
    fn an_unreadable_child_mount_does_not_shadow_its_readable_parents_other_children() {
        let mut read = a_card();
        read.unreadable.push(UnreadableFilesystem {
            mount_point: "/state/log".to_string(),
            why: "errno 5 (Input/output error)".to_string(),
        });
        let answer = run(read);
        assert_eq!(
            field(row(&answer, "/state"), "holds"),
            &Value::strings(["/state", "/state/config"]),
            "/state/log must not be held by /state, and /state/config must still be"
        );
        assert_eq!(field(&answer, "unreadable"), &Value::Integer(1));
    }

    #[test]
    fn a_machine_with_nothing_wrong_omits_the_detail_field_rather_than_emitting_an_empty_list() {
        let answer = run(a_card());
        assert_eq!(field(&answer, "unreadable"), &Value::Integer(0));
        assert_eq!(maybe(&answer, "unreadable-detail"), None);
    }

    /// A machine that cannot be read is an `E_MACHINE_REFUSED` carrying what it
    /// said — never an empty list of filesystems, which would report a machine
    /// with nothing mounted.
    #[test]
    fn a_machine_that_refuses_is_an_error_rather_than_a_machine_with_no_filesystems() {
        let mut m = FakeStorage::refusing("/proc is not mounted");
        match perform(&[SUBJECT], &mut m) {
            Err(e) => {
                assert_eq!(e.code(), ErrorCode::MachineRefused);
                assert!(e.to_string().contains("/proc is not mounted"), "{e}");
            }
            Ok(v) => panic!("a refusal was reported as an answer: {v:?}"),
        }
    }

    /// An empty table is an answer, and it names where it looked — so an
    /// operator can tell "nothing is mounted" from "I read the wrong file".
    #[test]
    fn an_empty_mount_table_is_an_answer_and_it_says_where_it_looked() {
        let answer = run(StorageRead {
            filesystems: Vec::new(),
            unreadable: Vec::new(),
            mount_table: "/proc/self/mounts".to_string(),
        });
        assert_eq!(field(&answer, "count"), &Value::Integer(0));
        assert_eq!(
            field(&answer, "mount-table"),
            &Value::str("/proc/self/mounts")
        );
    }

    /// The table is read **once** per invocation. Not efficiency: a second read
    /// could return a different set of mounts, and `count` would then describe a
    /// table the records did not come from.
    #[test]
    fn the_mount_table_is_read_once() {
        let mut m = FakeStorage::holding(a_card());
        let _ = perform(&[SUBJECT], &mut m);
        assert_eq!(m.reads, 1);
    }

    /// A second word is refused rather than ignored, and the refusal says what
    /// to type instead. An operator who typed `rivet get storage /state`
    /// expecting `df /state` must not be shown the whole machine as though
    /// their argument had been read.
    #[test]
    fn a_path_is_refused_by_name_and_the_refusal_says_how_to_select_instead() {
        let mut m = FakeStorage::holding(a_card());
        match perform(&[SUBJECT, "/state"], &mut m) {
            Err(e) => {
                assert_eq!(e.code(), ErrorCode::Arguments);
                assert_eq!(e.subject(), Some("/state"));
                assert!(e.message().contains("where mount-point"), "{}", e.message());
            }
            Ok(v) => panic!("an argument this operation does not take was ignored: {v:?}"),
        }
        assert_eq!(m.reads, 0, "a refused request read the machine anyway");
    }

    // -- the literal that has to stay true ------------------------------------

    /// `SYSTEM_DIR` is spelled out because `rivet-core` has no constant for
    /// it, so this is the guard: the installed CLI's path is `/system/bin/…`,
    /// and it is a path that cannot be wrong without no card accepting a login.
    #[test]
    fn the_system_directory_is_the_one_the_installed_binaries_are_in() {
        assert_eq!(SYSTEM_DIR, "/system");
        assert!(crate::etc::CLI_PATH.starts_with(&format!("{SYSTEM_DIR}/")));
        assert!(crate::etc::NUSHELL_PATH.starts_with(&format!("{SYSTEM_DIR}/")));
    }

    /// And every named location is read from the constant that owns it, so a
    /// move of `/state` moves this with it rather than leaving it behind.
    #[test]
    fn the_named_locations_are_the_ones_the_record_owns() {
        let named = named_locations();
        assert!(named.contains(&crate::state::STATE_MOUNT_POINT));
        assert!(named.contains(&crate::state::CONFIG_DIR));
        assert!(named.contains(&crate::state::LOG_DIR));
        // Outermost first, and no duplicates: a location listed twice would be
        // attributed to a filesystem twice.
        let mut sorted = named.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), named.len());
        for location in named {
            assert!(location.starts_with('/'), "{location} is not absolute");
        }
    }
}
