//! The default-slot record's lock, and the rule that a lock-holder reclaims a
//! mount it finds rather than using it.
//!
//! Two kinds of fake. [`FakeMedium`] answers one call at a time and records
//! the order of lock, mount, unmount and unlock. [`Process`] is several
//! processes sharing one card and one kernel — one mount table entry and one
//! lock — so an interleaving between them can be written down and replayed:
//! the second process runs at a named point in the first one's sequence.
//! Both are hand-composed; nothing here was captured from a board.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use super::*;
use crate::firmware::{AUTOBOOT_LOCK, DT_BOOT_PARTITION};
use crate::port::{Errno, Mounted};
use crate::testio::{FakeMedium, Script};

const AUTOBOOT: &str = "[all]\ntryboot_a_b=1\nboot_partition=1\n\n[tryboot]\nboot_partition=2\n";

fn card(running: u32) -> FakeMedium {
    let mut m = FakeMedium::new();
    m.properties_mut().insert(
        DT_BOOT_PARTITION.to_string(),
        Script::always(Ok(running.to_string().into_bytes())),
    );
    m.files_mut().insert(
        format!("{AUTOBOOT_PARTITION_MOUNT}/{AUTOBOOT_FILE}"),
        Script::always(Ok(AUTOBOOT.to_string())),
    );
    m
}

fn order(what: &[&str]) -> Vec<String> {
    what.iter()
        .map(|w| match *w {
            "lock" => format!("lock {AUTOBOOT_LOCK}"),
            "unlock" => format!("unlock {AUTOBOOT_LOCK}"),
            "mount" => format!("mount {AUTOBOOT_PARTITION_MOUNT}"),
            "unmount" => format!("unmount {AUTOBOOT_PARTITION_MOUNT}"),
            other => other.to_string(),
        })
        .collect()
}

// -- one process at a time --------------------------------------------------

/// Every access — the commit, the read of the default, the listing — takes the
/// lock before it mounts and releases it after it unmounts.
#[test]
fn the_lock_brackets_every_mount_of_the_record_partition() {
    let mut commit = card(2);
    assert_eq!(PiFirmware.commit(&mut commit, SlotId(2)), Ok(Committed::Changed));
    assert_eq!(commit.order, order(&["lock", "mount", "unmount", "unlock"]));

    let mut read = card(2);
    assert_eq!(PiFirmware.default_slot(&mut read), Ok(SlotId(1)));
    assert_eq!(read.order, order(&["lock", "mount", "unmount", "unlock"]));

    let mut listing = card(2);
    listing.list_results = Script::always(Ok(Vec::new()));
    assert!(PiFirmware.record_medium_listing(&mut listing, 8).is_ok());
    assert_eq!(listing.order, order(&["lock", "mount", "unmount", "unlock"]));
}

/// **A lock another process holds is a medium refusal, and nothing is
/// mounted.** For PID 1 that is an attempt its retry schedules again; for a
/// reader it is the answer *busy* rather than a hang. The lock that was never
/// taken is never released.
#[test]
fn a_held_lock_refuses_as_the_medium_and_touches_no_mount() {
    let mut commit = card(2);
    commit.lock_results = Script::always(Err(Errno::EAGAIN));
    let got = PiFirmware.commit(&mut commit, SlotId(2));
    assert!(
        matches!(&got, Err(ArmRefusal::Medium(why))
            if why.contains(AUTOBOOT_LOCK) && why.contains("EAGAIN")),
        "{got:?}"
    );
    assert_eq!(commit.order, order(&["lock"]));
    assert!(commit.write_calls.is_empty());

    let mut read = card(2);
    read.lock_results = Script::always(Err(Errno::EAGAIN));
    let got = PiFirmware.default_slot(&mut read);
    assert!(
        got.as_ref().is_err_and(|why| why.contains("another process holds it")),
        "{got:?}"
    );
    assert_eq!(read.order, order(&["lock"]));
}

/// **A mount found in place under the lock is an orphan, and is reclaimed**
/// — unmounted and mounted afresh, inside the lock — for the commit and the
/// reader alike; and one that will not go is a medium refusal that writes
/// nothing.
#[test]
fn a_mount_found_under_the_lock_is_reclaimed_rather_than_used() {
    let reclaimed = order(&["lock", "mount", "unmount", "mount", "unmount", "unlock"]);

    let mut commit = card(2);
    commit.mount_results = Script::sequence(vec![Ok(Mounted::Adopted)], Ok(Mounted::Made));
    assert_eq!(PiFirmware.commit(&mut commit, SlotId(2)), Ok(Committed::Changed));
    assert_eq!(commit.order, reclaimed);
    assert_eq!(commit.reclaimed, vec![AUTOBOOT_PARTITION_MOUNT.to_string()]);

    let mut read = card(2);
    read.mount_results = Script::sequence(vec![Ok(Mounted::Adopted)], Ok(Mounted::Made));
    assert_eq!(PiFirmware.default_slot(&mut read), Ok(SlotId(1)));
    assert_eq!(read.order, reclaimed);

    let mut stuck = card(2);
    stuck.mount_results = Script::always(Ok(Mounted::Adopted));
    assert!(matches!(
        PiFirmware.commit(&mut stuck, SlotId(2)),
        Err(ArmRefusal::Medium(_))
    ));
    assert!(stuck.write_calls.is_empty());
    assert_eq!(
        stuck.order,
        order(&["lock", "mount", "unmount", "mount", "unlock"])
    );
}

/// A mount that fails releases the lock and unmounts nothing.
#[test]
fn a_mount_that_fails_releases_the_lock_and_unmounts_nothing() {
    let mut m = card(2);
    m.mount_results = Script::always(Err(Errno::ENODEV));
    assert!(matches!(
        PiFirmware.commit(&mut m, SlotId(2)),
        Err(ArmRefusal::Medium(_))
    ));
    assert_eq!(m.order, order(&["lock", "mount", "unlock"]));
}

// -- several processes, one card --------------------------------------------

/// The card and the kernel as every process on the machine sees them.
struct World {
    /// Whether the record's partition is mounted at the mount point, and if so
    /// whether writable.
    mounted: Option<bool>,
    /// The partition's root directory, by name.
    partition: BTreeMap<String, String>,
    /// The bare mount-point directory on `/run`, which is what a path under the
    /// mount point reaches when nothing is mounted there.
    bare: BTreeMap<String, String>,
    /// Which process holds [`AUTOBOOT_LOCK`].
    lock: Option<&'static str>,
}

impl World {
    fn card() -> Rc<RefCell<Self>> {
        let mut partition = BTreeMap::new();
        partition.insert(AUTOBOOT_FILE.to_string(), AUTOBOOT.to_string());
        Rc::new(RefCell::new(Self {
            mounted: None,
            partition,
            bare: BTreeMap::new(),
            lock: None,
        }))
    }
}

/// A point in one process's sequence at which another process runs.
type Hook = Box<dyn FnMut()>;

/// One process's [`BootMedium`] over the shared [`World`].
///
/// Its mount answers the way both real implementors do: a mount point already
/// holding the partition is adopted rather than refused, whatever the flags.
/// Its lock answers the way a reader's does once its bounded wait has run
/// out — the holder in these interleavings does not let go while the other
/// process is running, so the wait is not modelled.
struct Process {
    name: &'static str,
    world: Rc<RefCell<World>>,
    /// Runs once, after this process writes [`AUTOBOOT_TEMP`].
    after_temp_written: Option<Hook>,
    /// Runs once, after this process reads [`AUTOBOOT_FILE`].
    after_record_read: Option<Hook>,
    /// Unmounts that found nothing mounted — the kernel's `EINVAL`.
    unmounts_of_nothing: usize,
    unmounts: usize,
    /// Orphaned mounts this process reported reclaiming.
    reclaims: usize,
}

impl Process {
    fn new(name: &'static str, world: &Rc<RefCell<World>>) -> Self {
        Self {
            name,
            world: Rc::clone(world),
            after_temp_written: None,
            after_record_read: None,
            unmounts_of_nothing: 0,
            unmounts: 0,
            reclaims: 0,
        }
    }

    fn name_in_mount(path: &str) -> Option<String> {
        path.strip_prefix(AUTOBOOT_PARTITION_MOUNT)
            .and_then(|rest| rest.strip_prefix('/'))
            .map(str::to_string)
    }
}

impl BootMedium for Process {
    fn read_property(&mut self, path: &str) -> Result<Vec<u8>, Errno> {
        match path {
            p if p == DT_BOOT_PARTITION => Ok(b"2".to_vec()),
            p if p == crate::firmware::DT_TRYBOOT => Ok(b"1".to_vec()),
            _ => Err(Errno::ENOENT),
        }
    }

    fn read_file(&mut self, path: &str) -> Result<String, Errno> {
        let name = Self::name_in_mount(path).ok_or(Errno::ENOENT)?;
        let text = {
            let world = self.world.borrow();
            let dir = if world.mounted.is_some() {
                &world.partition
            } else {
                &world.bare
            };
            dir.get(&name).cloned().ok_or(Errno::ENOENT)
        };
        if name == AUTOBOOT_FILE
            && let Some(mut hook) = self.after_record_read.take()
        {
            hook();
        }
        text
    }

    fn write_file(&mut self, path: &str, contents: &[u8]) -> Result<(), Errno> {
        let name = Self::name_in_mount(path).ok_or(Errno::ENOENT)?;
        {
            let mut world = self.world.borrow_mut();
            let text = String::from_utf8_lossy(contents).into_owned();
            match world.mounted {
                Some(false) => return Err(Errno::EROFS),
                Some(true) => world.partition.insert(name.clone(), text),
                None => world.bare.insert(name.clone(), text),
            };
        }
        if name == AUTOBOOT_TEMP
            && let Some(mut hook) = self.after_temp_written.take()
        {
            hook();
        }
        Ok(())
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), Errno> {
        let (Some(from), Some(to)) = (Self::name_in_mount(from), Self::name_in_mount(to)) else {
            return Err(Errno::ENOENT);
        };
        let mut world = self.world.borrow_mut();
        let dir = if world.mounted.is_some() {
            &mut world.partition
        } else {
            &mut world.bare
        };
        let text = dir.remove(&from).ok_or(Errno::ENOENT)?;
        dir.insert(to, text);
        Ok(())
    }

    fn sync(&mut self) {}

    fn mount_boot_filesystem(
        &mut self,
        _device: &str,
        at: &str,
        _fstype: &str,
        writable: bool,
    ) -> Result<Mounted, Errno> {
        assert_eq!(at, AUTOBOOT_PARTITION_MOUNT);
        let mut world = self.world.borrow_mut();
        if world.mounted.is_some() {
            return Ok(Mounted::Adopted);
        }
        world.mounted = Some(writable);
        Ok(Mounted::Made)
    }

    fn unmount(&mut self, _at: &str) {
        self.unmounts += 1;
        let mut world = self.world.borrow_mut();
        if world.mounted.take().is_none() {
            self.unmounts_of_nothing += 1;
        }
    }

    fn reclaimed_orphan(&mut self, _at: &str) {
        self.reclaims += 1;
    }

    fn lock(&mut self, path: &str) -> Result<(), Errno> {
        assert_eq!(path, AUTOBOOT_LOCK);
        let mut world = self.world.borrow_mut();
        match world.lock {
            Some(holder) if holder != self.name => Err(Errno::EAGAIN),
            _ => {
                world.lock = Some(self.name);
                Ok(())
            }
        }
    }

    fn unlock(&mut self, _path: &str) {
        let mut world = self.world.borrow_mut();
        if world.lock == Some(self.name) {
            world.lock = None;
        }
    }
}

/// What the second process got, and how many unmounts it made.
type Seen = Rc<RefCell<Option<(Result<SlotId, String>, usize)>>>;

/// A reader of the default slot, run at a hook point, recording what it got.
fn reader_at(world: &Rc<RefCell<World>>, seen: &Seen) -> Hook {
    let world = Rc::clone(world);
    let seen = Rc::clone(seen);
    Box::new(move || {
        let mut reader = Process::new("rivet get slot", &world);
        let got = PiFirmware.default_slot(&mut reader);
        *seen.borrow_mut() = Some((got, reader.unmounts));
    })
}

/// **The trial boot of dev-01, replayed.** PID 1 commits the moment the
/// control plane is reached, which is the handshake of the connection that
/// then runs `rivet get slot`. The reader arrives between PID 1's write of
/// `autoboot.txt.new` and its rename.
///
/// Without the lock and the ownership rule the reader adopted PID 1's
/// writable mount, read, and unmounted it; PID 1's rename then ran in the
/// bare mount-point directory and failed with `ENOENT`, and its own unmount
/// found nothing. Now the reader is refused as busy without touching the
/// mount, and the commit lands.
#[test]
fn a_reader_arriving_between_the_commits_write_and_rename_no_longer_breaks_it() {
    let world = World::card();
    let seen: Seen = Rc::new(RefCell::new(None));
    let mut pid1 = Process::new("pid 1", &world);
    pid1.after_temp_written = Some(reader_at(&world, &seen));

    let committed = PiFirmware.commit(&mut pid1, SlotId(2));

    assert_eq!(committed, Ok(Committed::Changed));
    let record = world.borrow().partition.get(AUTOBOOT_FILE).cloned();
    assert_eq!(
        record.as_deref().map(default_boot_partition),
        Some(Ok(2)),
        "the record does not name the committed slot: {record:?}"
    );
    assert_eq!(pid1.unmounts, 1);
    assert_eq!(pid1.unmounts_of_nothing, 0, "PID 1's unmount found nothing");

    let seen = seen.borrow();
    assert!(
        matches!(&*seen, Some((Err(why), 0)) if why.contains(AUTOBOOT_LOCK)),
        "the reader was not refused as busy, or it unmounted: {seen:?}"
    );
    assert_eq!(world.borrow().mounted, None, "a mount was leaked");
    assert_eq!(world.borrow().lock, None, "the lock was leaked");
}

/// **Two readers, one mount point.** The first made the mount; the second
/// arrives after the first has read and before it unmounts. It is refused
/// rather than adopting a mount about to disappear under it — which is why the
/// lock is exclusive for readers too.
#[test]
fn a_second_reader_waits_on_the_first_rather_than_sharing_its_mount() {
    let world = World::card();
    let seen: Seen = Rc::new(RefCell::new(None));
    let mut first = Process::new("first reader", &world);
    first.after_record_read = Some(reader_at(&world, &seen));

    assert_eq!(PiFirmware.default_slot(&mut first), Ok(SlotId(1)));
    assert_eq!(first.unmounts_of_nothing, 0);
    assert_eq!(first.reclaims, 0);
    let seen = seen.borrow();
    assert!(
        matches!(&*seen, Some((Err(why), 0)) if why.contains(AUTOBOOT_LOCK)),
        "{seen:?}"
    );
    assert_eq!(world.borrow().mounted, None);
}

/// **A leaked read-only mount no longer costs the trial boot.** A reader died
/// between its mount and its unmount — its lock went with it, its mount did
/// not. Adopting that mount would make every commit attempt fail with `EROFS`
/// until the next boot rolled the trial back; the commit reclaims it instead,
/// mounts writable, and lands on the first attempt.
#[test]
fn a_leaked_read_only_mount_is_reclaimed_and_the_commit_lands() {
    let world = World::card();
    world.borrow_mut().mounted = Some(false);
    let mut pid1 = Process::new("pid 1", &world);

    let committed = PiFirmware.commit(&mut pid1, SlotId(2));

    assert_eq!(committed, Ok(Committed::Changed));
    assert_eq!(pid1.reclaims, 1);
    let record = world.borrow().partition.get(AUTOBOOT_FILE).cloned();
    assert_eq!(record.as_deref().map(default_boot_partition), Some(Ok(2)));
    assert_eq!(world.borrow().mounted, None, "the fresh mount was not released");
    assert_eq!(world.borrow().lock, None);
}
