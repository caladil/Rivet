//! The stored wall clock: `ADR-021` §3's persist-and-restore, as a decision.
//!
//! A Raspberry Pi has no real-time clock. A board that has been powered off does
//! not know what time it is and starts at the epoch, which is not a theoretical
//! complaint — **every journal record the first Rivet board ever wrote is
//! stamped `1970-01-01T00:00:03`** (the 1970-clock finding, 2026-08-23). `ADR-021` §3 rules the fix:
//!
//! > The clock is written to `/state` periodically and on clean shutdown, and
//! > restored at boot. […] It does not give correct time. It gives
//! > *approximately* correct time immediately, and — more importantly — a
//! > **floor**: the machine never believes it is earlier than when it last ran.
//!
//! Everything here is a decision, and every one is reachable from a test with no
//! `/state` and no board — see [`crate::port`] for the seam that makes that true.
//! **Nothing here sets a clock.** Setting one is a syscall, so it lives in the
//! adapter that owns syscalls; this module answers *what* the clock should be
//! and *what should be written down*, which are the two questions worth testing.
//!
//! # A floor, not a set — and the record contradicts itself about this
//!
//! `ADR-021` §3 says **floor**. §3.2, two paragraphs later, describes the same
//! mechanism as *"read a file and call `clock_settime`"*, which is an
//! unconditional set and a different mechanism: it would move a *correct* clock
//! **backwards** to a stale stored value.
//!
//! §3 is the ruling and §3.2 is a loose restatement of it while arguing about
//! something else entirely (which process does the work). [`Floor::decide`]
//! implements §3, and the discrepancy is reported rather than quietly resolved.
//!
//! It is not academic. `PLAN.md` §9.2 commits Rivet to **an x86 target**, and
//! an x86 machine has a battery-backed clock that can disagree
//! with the stored value — in the direction where an unconditional set is
//! actively harmful. The Pi is the platform where the two mechanisms happen to
//! be indistinguishable, because its clock is always 1970 and therefore always
//! lower.
//!
//! # What is on the card
//!
//! ```text
//! /state/clock       the stored wall clock  (ADR-021 §3)
//! /state/clock.new   a save in progress     (crate::atomic)
//! ```
//!
//! **Not `/state/config`.** `ADR-008` §3.1 makes that directory *"the one Rivet
//! file an operator can open and edit with effect"*, and a clock stamp is
//! machine-derived state rather than operator configuration — editing it would
//! be stating a desired time, which is not a thing Rivet offers. `/state` is
//! the right filesystem for the property that actually matters, which is
//! `ADR-008` §3's table: `/state` **survives an image update**, and an update
//! replaces `/system` wholesale. Machine-derived state lives beside
//! `/state/config` rather than inside it, and this follows that.
//!
//! # Why the file is not TOML
//!
//! `rivet-config` owns the project's one document schema and its canonical
//! form (`ADR-007` §3.1), and this is not that document — it is two machine
//! words that no operator authors. Parsing it with the document parser would put
//! the desired-state schema in the boot path of the clock; giving it a *second*
//! TOML dialect would be a second configuration format for something that is not
//! configuration. One line, two fields, refused if it is anything else.

use crate::atomic::{commit, in_progress};
use crate::port::{Errno, Filesystem};
use crate::store::{Operation, StoreError};

/// Where the stored clock lives, inside `rivet_core::state::STATE_MOUNT_POINT`.
///
/// The **file name**; the directory comes from `rivet-core`, which is the one
/// place `/state` is spelled (`ADR-011`).
pub const CLOCK_FILE: &str = "clock";

/// What the wall clock a stamp was taken from descends from.
///
/// **This is provenance, not accuracy**, and the distinction is the whole reason
/// it is recorded. Neither variant claims the time is *right*; they say whether
/// anything outside this machine ever had a say in it.
///
/// # Why it is in the format now, when nothing branches on it
///
/// The failure this exists for is the one `ADR-021` §4.2 wants to avoid — a
/// backward step while running. A floor can only produce one if the stored value
/// is *ahead* of real time, and the way a value gets ahead and stays ahead is by
/// being re-persisted every hour as though something had vouched for it. A file
/// that records only a number cannot tell a value a time server supplied from
/// one that has been laundered through fifty boots of `max(stored, now)`.
///
/// **The format is the expensive thing to change later; the policy is not.** A
/// bare integer that later needs provenance means either a second file or a
/// migration on the one filesystem an update never touches. So the field is
/// written and reported, and **nothing branches on it** — ~~what a restored-only
/// value entitles the machine to do is `ADR-021` §4.2's step-versus-slew
/// question, which is the maintainer's and is not answered here.~~
///
/// **That question was ruled on 2026-08-23, `ADR-021` §4.2.1: Rivet always
/// steps and never slews, so there is no step-versus-slew decision for
/// provenance to inform.** `cmd/rivet-timed` reads a time server and steps;
/// it does not consult this field and never will, because what a *stored* value
/// descends from says nothing about whether a *server's* answer should be
/// applied. So "nothing branches on it" is now a stronger statement than it was:
/// it is not waiting on a ruling, and the one plausible consumer has been ruled
/// out of needing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// An external time source had set this machine's clock on the boot that
    /// wrote this.
    ///
    /// **Nothing in Rivet produces this today**, and saying so is the point
    /// rather than an omission. It is parsed because a reader must accept what a
    /// later writer produces, and it is written only by this crate's own tests.
    ///
    /// # The reason changed on 2026-08-23, and the new one is more interesting
    ///
    /// ~~`ADR-021` §4's SNTP client is unbuilt, so no Rivet machine has ever
    /// had a synchronised clock.~~ **Struck** — `cmd/rivet-timed` was built
    /// that day on `rsntp`, and `ADR-021` §4.2.1 ruled how it corrects. The
    /// sentence is kept readable because `AGENTS.md` §7 is about a claim of
    /// absence outliving the absence, and this one did.
    ///
    /// **What replaces it is a boundary rather than a gap.** The process that
    /// synchronises the clock (`cmd/rivet-timed`) and the process that writes
    /// this file (`cmd/rivet-sysd`) are **different processes with no
    /// channel between them** — deliberately, because two writers of
    /// `/state/clock` would be two authorities over one fact and `ADR-011`
    /// exists to prevent that. So the supervisor writes what it can honestly
    /// assert, which is [`Origin::Unsynchronised`], on a machine whose clock may
    /// have been set by a time server minutes earlier.
    ///
    /// Under-claiming is the safe direction and the behaviour is therefore
    /// correct as it stands. What is **not** settled anywhere in the record is
    /// which side should change — whether `rivet-timed` should report to the
    /// supervisor, or whether provenance should stop being a property of this
    /// file at all. It is an open question, not a defect.
    Synchronised,
    /// Nothing outside this machine had a say. The clock descended from a
    /// previous stored value, or from whatever the kernel started with.
    ///
    /// **Every stamp a Rivet machine writes today is this one.** A hardware
    /// real-time clock — which `PLAN.md` §9.2's x86 machine has and a Pi does
    /// not — also lands here, because Rivet cannot tell an RTC from a lucky
    /// build-time default, and a claim of `Synchronised` that rested on that
    /// guess would be exactly the assertion §4.2 must not be built on.
    /// Under-claiming is the safe direction.
    Unsynchronised,
}

impl Origin {
    /// The word written to the file. Stable: it is an on-disk format.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Synchronised => "synchronised",
            Self::Unsynchronised => "unsynchronised",
        }
    }

    fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "synchronised" => Self::Synchronised,
            "unsynchronised" => Self::Unsynchronised,
            _ => return None,
        })
    }
}

impl core::fmt::Display for Origin {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One stored clock reading: when, and what it descends from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    /// Milliseconds since the Unix epoch.
    ///
    /// Milliseconds rather than seconds because that is the unit every clock in
    /// this workspace already speaks — the supervisor's `Kernel::wall_ms` and
    /// `ADR-022`'s record stamp both — and a unit conversion at the storage layer
    /// is a rounding rule nobody would think to test.
    pub unix_ms: u64,
    pub origin: Origin,
}

impl Stamp {
    /// The file's whole content: one line, two fields, a trailing newline.
    pub fn to_line(self) -> String {
        format!("{} {}\n", self.unix_ms, self.origin.as_str())
    }

    /// Parse one, or say which shape rule it broke.
    ///
    /// Refuses rather than guessing (`AGENTS.md` §5): a file with three fields is
    /// not a file with two and something after it, and a machine that read the
    /// first number out of an unrecognised format would set its clock from bytes
    /// something else wrote.
    ///
    /// **The reason never quotes the file.** `StoreError::Refused`'s own type
    /// documentation carries the argument; the clock file holds nothing secret,
    /// and following the rule everywhere is what keeps it followed.
    fn parse(text: &str) -> Result<Self, String> {
        let mut words = text.split_ascii_whitespace();
        let (Some(when), Some(origin)) = (words.next(), words.next()) else {
            return Err(
                "expected two fields, `<unix milliseconds> <synchronised|unsynchronised>`, and \
                 found fewer"
                    .to_owned(),
            );
        };
        if words.next().is_some() {
            return Err(
                "expected two fields, `<unix milliseconds> <synchronised|unsynchronised>`, and \
                 found more"
                    .to_owned(),
            );
        }
        let Ok(unix_ms) = when.parse::<u64>() else {
            return Err(format!(
                "the first field is {} characters and is not an unsigned decimal number of \
                 milliseconds",
                when.len()
            ));
        };
        let Some(origin) = Origin::parse(origin) else {
            return Err(
                "the second field is neither `synchronised` nor `unsynchronised`".to_owned(),
            );
        };
        Ok(Self { unix_ms, origin })
    }
}

/// What the restore decided. `ADR-021` §3's floor, as a value.
///
/// A value rather than an effect, so that the *decision* is testable without a
/// clock to set: the caller reads [`Floor::set_to`] and performs the one syscall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Floor {
    /// Nothing is stored, so there is no floor and the clock is left exactly as
    /// the kernel had it.
    ///
    /// **Every first boot**, and not a fault. On a Pi it means the machine goes
    /// on believing it is 1970, which is the honest outcome: there is nothing to
    /// restore from and inventing a time would be worse than having none.
    NothingStored { now_ms: u64 },
    /// The stored value is later than the running clock, so the clock is raised
    /// to it. **The normal case on a Pi**, on every boot after the first.
    Raised {
        from_ms: u64,
        to_ms: u64,
        stored_origin: Origin,
    },
    /// The running clock is already at or past the stored value, so **nothing is
    /// set**.
    ///
    /// This is the arm that makes it a floor rather than a set, and the one an
    /// unconditional `clock_settime` would get wrong. Unreachable on a Pi and
    /// routine on a machine with a battery-backed clock (`PLAN.md` §9.2).
    AlreadyAhead {
        stored_ms: u64,
        now_ms: u64,
        stored_origin: Origin,
    },
}

impl Floor {
    /// `ADR-021` §3, in one line: `max(stored, now)`, and never less than now.
    pub fn decide(stored: Option<Stamp>, now_ms: u64) -> Self {
        let Some(stamp) = stored else {
            return Self::NothingStored { now_ms };
        };
        if stamp.unix_ms > now_ms {
            Self::Raised {
                from_ms: now_ms,
                to_ms: stamp.unix_ms,
                stored_origin: stamp.origin,
            }
        } else {
            Self::AlreadyAhead {
                stored_ms: stamp.unix_ms,
                now_ms,
                stored_origin: stamp.origin,
            }
        }
    }

    /// What the clock must be set to, or `None` when it must be left alone.
    ///
    /// `None` for two of the three arms, which is the point: the floor performs
    /// **at most one** clock movement per boot and it is always forwards.
    pub const fn set_to(self) -> Option<u64> {
        match self {
            Self::Raised { to_ms, .. } => Some(to_ms),
            Self::NothingStored { .. } | Self::AlreadyAhead { .. } => None,
        }
    }

    /// Whether this boot learned something the card does not already know, and
    /// should therefore write immediately rather than waiting for the first
    /// periodic write.
    ///
    /// Two cases, and the second is the one that matters:
    ///
    /// * **Nothing is stored** — the file does not exist, and a machine that
    ///   waited an hour to create it loses everything if it is power-cycled
    ///   first, which is the ordinary way a Pi is switched off.
    /// * **The running clock was ahead of the stored value** — something on this
    ///   machine knows a later time than the card does, and that is new
    ///   information.
    ///
    /// [`Floor::Raised`] is deliberately **not** here: the clock was just set
    /// *from* the stored value, so writing it back would be a card write that
    /// changes nothing (`ADR-018` §7 concentrates all of a Rivet machine's wear
    /// on this one partition).
    pub const fn worth_persisting_now(self) -> bool {
        match self {
            Self::NothingStored { .. } => true,
            Self::AlreadyAhead {
                stored_ms, now_ms, ..
            } => now_ms > stored_ms,
            Self::Raised { .. } => false,
        }
    }

    /// The value a machine believes after this decision, whatever it decided.
    ///
    /// This is the floor the caller must not let a later write fall below — see
    /// the same rule stated as an invariant on [`ClockStore::save`].
    pub const fn resulting_ms(self) -> u64 {
        match self {
            Self::NothingStored { now_ms } | Self::AlreadyAhead { now_ms, .. } => now_ms,
            Self::Raised { to_ms, .. } => to_ms,
        }
    }
}

/// The stored wall clock, on a filesystem.
///
/// Holds a directory and nothing else — no handle, no cache, for the reason
/// [`crate::store::ConfigStore`] gives: a store that remembered the last value it
/// read would be a second answer to "what does the card say", and the stale one
/// would be the one that got used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockStore {
    root: String,
}

impl ClockStore {
    /// The store at `/state`, through `rivet_core::state` rather than a second
    /// spelling of the mount point.
    pub fn at_default() -> Self {
        Self::at(rivet_core::state::STATE_MOUNT_POINT)
    }

    /// A store rooted anywhere, which is what the tests use. **A path is not a
    /// policy** — the same note `ConfigStore::at` carries.
    pub fn at(root: &str) -> Self {
        Self {
            root: root.trim_end_matches('/').to_owned(),
        }
    }

    /// The directory this store writes into. **Never created by this crate**: it
    /// is a mount point that ships in the image, because the read-only root it
    /// would be created on cannot be written to (`ADR-018` §3).
    pub fn root(&self) -> &str {
        &self.root
    }

    /// Where the stored clock is.
    pub fn path(&self) -> String {
        format!("{}/{CLOCK_FILE}", self.root)
    }

    /// Where a save in progress is written before it is committed.
    pub fn in_progress_path(&self) -> String {
        in_progress(&self.path())
    }

    /// Read the stored clock.
    ///
    /// `Ok(None)` means **nothing is stored**, which is a complete answer and not
    /// a failure: it is every first boot, and [`Floor::decide`] handles it by
    /// leaving the clock alone. Anything else that went wrong is an error, for
    /// the reason `AGENTS.md` §5 gives — a card returning `EIO` reported as
    /// "no clock stored" is a machine that would then *overwrite* the one good
    /// value it had.
    ///
    /// **The in-progress path is never read.** A `clock.new` left by a power cut
    /// names bytes that were never committed. It is not read, not deleted, and
    /// overwritten by the next [`Self::save`].
    pub fn load(&self, fs: &mut dyn Filesystem) -> Result<Option<Stamp>, StoreError> {
        let path = self.path();
        let bytes = match fs.read(&path) {
            Ok(bytes) => bytes,
            Err(Errno::ENOENT) => return Ok(None),
            Err(errno) => {
                return Err(StoreError::Io {
                    path,
                    op: Operation::Read,
                    errno,
                });
            }
        };

        let text = match core::str::from_utf8(&bytes) {
            Ok(text) => text,
            Err(e) => {
                return Err(StoreError::NotText {
                    path,
                    at: e.valid_up_to(),
                });
            }
        };

        match Stamp::parse(text) {
            Ok(stamp) => Ok(Some(stamp)),
            Err(detail) => Err(StoreError::Refused { path, detail }),
        }
    }

    /// Write the clock, so that a power cut leaves either the old value or the
    /// new one and never a mixture.
    ///
    /// The sequence and everything it does and does not guarantee is
    /// [`crate::atomic::commit`]'s, shared with
    /// [`crate::store::ConfigStore::save`] rather than written twice.
    ///
    /// # The invariant this function cannot enforce, and the caller must
    ///
    /// **The stored value must never decrease.** Nothing here checks it, because
    /// nothing here can: the store is handed a stamp and has no way to know
    /// whether the clock it came from is the one the floor established or a clock
    /// something moved backwards afterwards. A store that refused a lower value
    /// would also refuse the *correction* ~~an eventual~~ `ADR-021` §4's time
    /// client must be able to make ~~, which is a decision `ADR-021` §4.2
    /// reserves~~.
    ///
    /// **That client exists since 2026-08-23** (`cmd/rivet-timed`) and
    /// `ADR-021` §4.2.1 rules that it steps in **both** directions, so the
    /// hypothetical this paragraph was written against is now the ordinary
    /// case: a machine whose stored floor was ahead of real time has its clock
    /// moved backwards on purpose, by a time server, and a store that refused
    /// low values would have made that impossible. The reasoning is unchanged
    /// and the tense is.
    ///
    /// So the rule lives with the caller, and the reason it matters is concrete:
    /// a machine whose `clock_settime` was refused still reports 1970 from its
    /// wall clock, and persisting that would lower the floor and lose every hour
    /// the card had recorded.
    ///
    /// # No directory is created
    ///
    /// Unlike [`crate::store::ConfigStore::save`], which creates `/state/config`
    /// because `ADR-017` §3.1.7 says the writer creates it. `/state` is a mount
    /// point in the image and a `create_dir_all` over it would either be a no-op
    /// or, on a machine where it did not mount, create a directory on the
    /// read-only root — which is the failure this store needs reported, not
    /// worked around.
    pub fn save(&self, fs: &mut dyn Filesystem, stamp: Stamp) -> Result<(), StoreError> {
        commit(
            fs,
            &self.root,
            &self.path(),
            &self.in_progress_path(),
            stamp.to_line().as_bytes(),
        )
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::testfs::{Call, FakeFs, Script};

    /// A plausible wall time, so that a test reading `1970` in an assertion means
    /// something. Hand-composed and arbitrary — `AGENTS.md` §8.3 keeps the dates
    /// of real events out of tracked files, so this is a round number and not an
    /// observation.
    const A_PLAUSIBLE_MS: u64 = 1_700_000_000_000;

    fn store() -> ClockStore {
        ClockStore::at("/state")
    }

    fn unsynchronised(unix_ms: u64) -> Stamp {
        Stamp {
            unix_ms,
            origin: Origin::Unsynchronised,
        }
    }

    // ---- where it lives ----------------------------------------------------

    /// `ADR-008` §3.1 makes `/state/config` the operator's editable file; a clock
    /// stamp is machine-derived state and must not be in it.
    #[test]
    fn the_stored_clock_is_on_state_and_not_inside_the_operators_configuration() {
        let s = ClockStore::at_default();
        assert_eq!(s.root(), rivet_core::state::STATE_MOUNT_POINT);
        assert_eq!(s.path(), "/state/clock");
        assert!(
            !s.path().starts_with(rivet_core::state::CONFIG_DIR),
            "the clock must not be in the one file an operator edits (ADR-008 §3.1)"
        );
        // And it is on the filesystem that survives an update, which is the
        // property the whole mechanism depends on (ADR-008 §3).
        assert!(s.path().starts_with(rivet_core::state::STATE_MOUNT_POINT));
    }

    #[test]
    fn the_in_progress_path_is_a_sibling_of_the_stored_one() {
        let s = store();
        let temp = s.in_progress_path();
        assert_eq!(temp, "/state/clock.new");
        assert_eq!(temp.rfind('/'), s.path().rfind('/'));
        assert_ne!(temp, s.path());
    }

    #[test]
    fn a_trailing_slash_on_the_root_does_not_double_up() {
        assert_eq!(ClockStore::at("/state/").path(), ClockStore::at("/state").path());
    }

    // ---- the format --------------------------------------------------------

    #[test]
    fn a_stamp_round_trips_through_the_line_for_both_origins() {
        for origin in [Origin::Synchronised, Origin::Unsynchronised] {
            let stamp = Stamp {
                unix_ms: A_PLAUSIBLE_MS,
                origin,
            };
            assert_eq!(Stamp::parse(&stamp.to_line()), Ok(stamp));
        }
    }

    #[test]
    fn the_line_is_one_line_with_two_fields_and_a_newline() {
        let line = unsynchronised(A_PLAUSIBLE_MS).to_line();
        assert_eq!(line, format!("{A_PLAUSIBLE_MS} unsynchronised\n"));
        assert_eq!(line.lines().count(), 1);
    }

    /// Refuse rather than guess. Every one of these would set a clock from bytes
    /// something else wrote if the first number were simply taken.
    #[test]
    fn anything_that_is_not_two_recognised_fields_is_refused() {
        for text in [
            "",
            "\n",
            "1700000000000",
            "1700000000000 unsynchronised extra",
            "1700000000000 approximately",
            "-1 unsynchronised",
            "1.5 unsynchronised",
            "99999999999999999999999999 unsynchronised",
            "unsynchronised 1700000000000",
        ] {
            assert!(
                Stamp::parse(text).is_err(),
                "{text:?} was accepted as a stored clock"
            );
        }
    }

    /// Whitespace a writer might reasonably differ on is not a refusal — a
    /// trailing newline, or none.
    #[test]
    fn surrounding_whitespace_is_tolerated_because_it_carries_no_meaning() {
        for text in [
            "1700000000000 unsynchronised",
            "1700000000000 unsynchronised\n",
            "  1700000000000   unsynchronised  \n",
        ] {
            assert_eq!(Stamp::parse(text), Ok(unsynchronised(A_PLAUSIBLE_MS)), "{text:?}");
        }
    }

    /// `StoreError::Refused` carries a reason built from shape and counts, never
    /// from the file. The clock file holds nothing secret; following the rule
    /// everywhere is what keeps it followed.
    #[test]
    fn a_refusal_says_what_shape_was_expected_and_never_quotes_the_file() {
        let mut fs = FakeFs::mounted();
        fs.given("/state/clock", b"1700000000000 hunter2-shaped-nonsense\n");
        let err = store().load(&mut fs).expect_err("not a recognised origin");
        let rendered = err.to_string();
        assert!(rendered.contains("synchronised"), "{rendered}");
        assert!(!rendered.contains("hunter2"), "{rendered}");
    }

    // ---- load --------------------------------------------------------------

    /// The branch every first boot takes.
    #[test]
    fn nothing_stored_is_a_complete_answer_and_not_a_failure() {
        let mut fs = FakeFs::mounted();
        assert_eq!(store().load(&mut fs), Ok(None));
    }

    #[test]
    fn a_stored_clock_loads_back_as_the_value_that_was_saved() {
        let mut fs = FakeFs::mounted();
        let stamp = unsynchronised(A_PLAUSIBLE_MS);
        assert_eq!(store().save(&mut fs, stamp), Ok(()));
        assert_eq!(store().load(&mut fs), Ok(Some(stamp)));
    }

    /// `AGENTS.md` §5 again, and here the cost of guessing is specific: an `EIO`
    /// read as "nothing stored" gives a machine that leaves its clock at 1970 and
    /// then **overwrites** the one good value it had at the next periodic write.
    #[test]
    fn an_unreadable_stored_clock_is_an_error_and_never_mistaken_for_an_absent_one() {
        let mut fs = FakeFs::mounted();
        fs.read_results = Some(Script::always(Err(Errno::EIO)));
        assert_eq!(
            store().load(&mut fs),
            Err(StoreError::Io {
                path: "/state/clock".to_owned(),
                op: Operation::Read,
                errno: Errno::EIO,
            })
        );
    }

    /// `ADR-018` §8's damaged `/state`, in the shape a half-erased flash block
    /// takes.
    #[test]
    fn bytes_that_are_not_utf8_are_refused_with_an_offset_and_not_with_the_bytes() {
        let mut fs = FakeFs::mounted();
        fs.given("/state/clock", b"17000\xff\xfe");
        assert_eq!(
            store().load(&mut fs),
            Err(StoreError::NotText {
                path: "/state/clock".to_owned(),
                at: 5,
            })
        );
    }

    /// A power cut between the write and the rename leaves a `.new` naming bytes
    /// nothing committed. The cheapest guarantee they are not adopted is never to
    /// look, which is what this asserts.
    #[test]
    fn a_temp_left_by_a_crash_is_never_read_and_never_becomes_the_clock() {
        let mut fs = FakeFs::mounted();
        fs.given("/state/clock.new", b"9999999999999 synchronised\n");
        assert_eq!(store().load(&mut fs), Ok(None));
        assert!(
            !fs.calls.iter().any(|c| matches!(c, Call::Read(p) if p.ends_with(".new"))),
            "{:?}",
            fs.calls
        );
    }

    // ---- save --------------------------------------------------------------

    /// `/state` is a mount point that ships in the image. A save that created it
    /// would, on a machine where it did not mount, put a directory on the
    /// read-only root instead of reporting the failure that matters.
    #[test]
    fn a_save_creates_no_directory_and_commits_through_the_shared_sequence() {
        let mut fs = FakeFs::mounted();
        let stamp = unsynchronised(A_PLAUSIBLE_MS);
        let bytes = stamp.to_line().len();
        assert_eq!(store().save(&mut fs, stamp), Ok(()));
        assert_eq!(
            fs.calls,
            vec![
                Call::Write("/state/clock.new".to_owned(), bytes),
                Call::SyncFile("/state/clock.new".to_owned()),
                Call::Rename("/state/clock.new".to_owned(), "/state/clock".to_owned()),
                Call::SyncDir("/state".to_owned()),
            ]
        );
    }

    #[test]
    fn saving_twice_leaves_one_file_and_the_later_value() {
        let mut fs = FakeFs::mounted();
        assert_eq!(store().save(&mut fs, unsynchronised(A_PLAUSIBLE_MS)), Ok(()));
        assert_eq!(
            store().save(&mut fs, unsynchronised(A_PLAUSIBLE_MS + 3_600_000)),
            Ok(())
        );
        assert_eq!(fs.paths(), vec!["/state/clock"]);
        assert_eq!(
            store().load(&mut fs),
            Ok(Some(unsynchronised(A_PLAUSIBLE_MS + 3_600_000)))
        );
    }

    /// An unmounted `/state` refuses the write, and the message has to name the
    /// cause or `EROFS` sends an operator looking at permissions.
    #[test]
    fn an_unmounted_state_refuses_the_save_and_the_message_names_the_cause() {
        let mut fs = FakeFs::unmounted();
        let err = store()
            .save(&mut fs, unsynchronised(A_PLAUSIBLE_MS))
            .expect_err("nothing can be written");
        assert!(
            matches!(&err, StoreError::Io { op: Operation::Write, errno: Errno::EROFS, .. }),
            "{err:?}"
        );
        assert!(err.to_string().contains("/state is not mounted"), "{err}");
    }

    /// Past the commit the value *is* in place, so a directory flush that failed
    /// must not be reported as though nothing happened.
    #[test]
    fn a_directory_flush_that_fails_says_the_clock_is_stored_anyway() {
        let mut fs = FakeFs::mounted();
        fs.sync_dir_results = Some(Script::always(Err(Errno::EIO)));
        let err = store()
            .save(&mut fs, unsynchronised(A_PLAUSIBLE_MS))
            .expect_err("the flush failed");
        assert!(matches!(&err, StoreError::NotDurable { .. }), "{err:?}");
        assert_eq!(
            fs.contents("/state/clock"),
            Some(unsynchronised(A_PLAUSIBLE_MS).to_line().as_bytes())
        );
    }

    // ---- the floor: ADR-021 §3 ---------------------------------------------

    /// The normal Pi boot: the card knows a later time than the kernel does, so
    /// the clock is raised to it.
    #[test]
    fn a_stored_value_later_than_the_running_clock_raises_it() {
        let floor = Floor::decide(Some(unsynchronised(A_PLAUSIBLE_MS)), 3_000);
        assert_eq!(
            floor,
            Floor::Raised {
                from_ms: 3_000,
                to_ms: A_PLAUSIBLE_MS,
                stored_origin: Origin::Unsynchronised,
            }
        );
        assert_eq!(floor.set_to(), Some(A_PLAUSIBLE_MS));
        assert_eq!(floor.resulting_ms(), A_PLAUSIBLE_MS);
    }

    /// **The arm an unconditional `clock_settime` gets wrong**, and the whole of
    /// why `ADR-021` §3 says *floor* while §3.2 says *set*. A machine with a
    /// battery-backed clock — `PLAN.md` §9.2's x86 target — boots knowing a later
    /// time than the card, and the restore must leave it alone.
    #[test]
    fn a_running_clock_ahead_of_the_stored_value_is_never_moved_backwards() {
        let floor = Floor::decide(Some(unsynchronised(A_PLAUSIBLE_MS)), A_PLAUSIBLE_MS + 86_400_000);
        assert_eq!(floor.set_to(), None, "the floor moved a good clock backwards");
        assert_eq!(floor.resulting_ms(), A_PLAUSIBLE_MS + 86_400_000);
        assert!(
            floor.worth_persisting_now(),
            "the machine knows a later time than the card and must say so"
        );
    }

    /// The property, over the whole space rather than at two points: whatever is
    /// stored and whatever the clock reads, the decision never lowers it.
    #[test]
    fn the_floor_never_lowers_the_clock_for_any_pair_of_values() {
        let values = [0, 1, 3_000, A_PLAUSIBLE_MS - 1, A_PLAUSIBLE_MS, u64::MAX / 2];
        for stored in values {
            for now in values {
                let floor = Floor::decide(Some(unsynchronised(stored)), now);
                assert!(
                    floor.resulting_ms() >= now,
                    "stored={stored} now={now} produced {floor:?}"
                );
                assert_eq!(
                    floor.resulting_ms(),
                    stored.max(now),
                    "the floor is max(stored, now) — ADR-021 §3"
                );
                if let Some(to) = floor.set_to() {
                    assert!(to > now, "a set must always move the clock forwards");
                }
            }
        }
    }

    /// Equal values are not a raise and are not new information either.
    #[test]
    fn a_stored_value_equal_to_the_running_clock_sets_nothing_and_writes_nothing() {
        let floor = Floor::decide(Some(unsynchronised(A_PLAUSIBLE_MS)), A_PLAUSIBLE_MS);
        assert_eq!(floor.set_to(), None);
        assert!(!floor.worth_persisting_now());
    }

    /// Every first boot. Nothing is restored and nothing is invented — but the
    /// file is created, so a power cycle an hour later is not a total loss.
    #[test]
    fn nothing_stored_leaves_the_clock_alone_and_writes_immediately() {
        let floor = Floor::decide(None, 3_000);
        assert_eq!(floor, Floor::NothingStored { now_ms: 3_000 });
        assert_eq!(floor.set_to(), None);
        assert_eq!(floor.resulting_ms(), 3_000);
        assert!(floor.worth_persisting_now());
    }

    /// A raise writes nothing at boot: the clock was just set *from* the stored
    /// value, so persisting it back is a card write that changes nothing, on the
    /// one partition carrying all of a Rivet machine's wear (`ADR-018` §7).
    #[test]
    fn a_raise_does_not_immediately_write_back_what_it_just_read() {
        let floor = Floor::decide(Some(unsynchronised(A_PLAUSIBLE_MS)), 3_000);
        assert!(!floor.worth_persisting_now());
    }

    /// Provenance survives the decision, so a caller can report what the value it
    /// restored from descended from. **Nothing branches on it** — `ADR-021` §4.2
    /// is the maintainer's.
    #[test]
    fn the_stored_origin_is_carried_through_the_decision_unchanged() {
        for origin in [Origin::Synchronised, Origin::Unsynchronised] {
            let stamp = Stamp {
                unix_ms: A_PLAUSIBLE_MS,
                origin,
            };
            assert_eq!(
                Floor::decide(Some(stamp), 3_000),
                Floor::Raised {
                    from_ms: 3_000,
                    to_ms: A_PLAUSIBLE_MS,
                    stored_origin: origin,
                }
            );
            // ...and the decision itself is identical either way, which is the
            // half that says no policy has been invented here.
            assert_eq!(
                Floor::decide(Some(stamp), 3_000).set_to(),
                Floor::decide(Some(unsynchronised(A_PLAUSIBLE_MS)), 3_000).set_to()
            );
        }
    }

    /// End to end, with no board: save, reload, and restore onto a machine that
    /// has just come up at the epoch. This is the whole mechanism.
    #[test]
    fn a_power_cycle_restores_the_hour_the_card_last_recorded() {
        let mut fs = FakeFs::mounted();
        // An hour into a boot that already knew the time.
        assert_eq!(store().save(&mut fs, unsynchronised(A_PLAUSIBLE_MS)), Ok(()));

        // The power goes out; the next boot comes up at three seconds past the
        // epoch, which is what a Raspberry Pi 4 actually did (the 1970-clock finding, 2026-08-23).
        let stored = store().load(&mut fs).expect("readable").expect("stored");
        let floor = Floor::decide(Some(stored), 3_000);
        assert_eq!(floor.set_to(), Some(A_PLAUSIBLE_MS));
    }
}
