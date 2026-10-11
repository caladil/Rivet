//! `ADR-021` §3's persist-and-restore, wired into the supervisor's start-up and
//! its wait loop.
//!
//! **Every journal record the first Rivet board ever wrote is stamped
//! `1970-01-01T00:00:03`** (the 1970-clock finding, 2026-08-23). A Raspberry Pi has no real-time
//! clock, so a board that has been powered off does not know what time it is, and
//! a journal whose timestamps are all 1970 cannot be correlated across boots or
//! against another machine — which sits directly underneath the thing an operator
//! would use to debug a remote board. This module is what stops that.
//!
//! # The division of labour
//!
//! | Where | What |
//! | :-- | :-- |
//! | `rivet_store::clock` | **the decisions** — the floor, the file's format, the crash-safe save |
//! | `crate::kernel` | **the syscalls** — `clock_settime`, and the four calls the atomic write needs |
//! | here | the wiring: when it happens, what is said about it, and the one invariant neither of the others can enforce |
//!
//! Nothing in this file is a decision the store could have made, and that is on
//! purpose: `rivet-store`'s tests reach the states that matter — a `/state`
//! that did not mount, a card that is full, `ENOSPC` arriving at `fsync` rather
//! than at `write` — against a fake filesystem, on a Windows development host.
//!
//! # It is a floor, and the record contradicts itself about that
//!
//! `ADR-021` §3 rules a **floor** — *"the machine never believes it is earlier
//! than when it last ran"* — while §3.2, arguing about which process does the
//! work, restates it as *"read a file and call `clock_settime`"*. Those are
//! different mechanisms: the second moves a *correct* clock backwards onto a
//! stale stored value. §3 is the ruling; `rivet_store::clock::Floor` implements
//! it, and the discrepancy is reported rather than quietly resolved. It matters
//! for `PLAN.md` §9.2's x86 machine, which has a battery-backed clock to
//! disagree with.
//!
//! # What is NOT here, and is not an omission
//!
//! * ~~**No clock correction and no time protocol.** `ADR-021` §4 rules a small
//!   SNTP crate and `ADR-021` §8 item 1 leaves *which one* open; §4.2's
//!   step-versus-slew threshold is unruled. So this restores a floor and nothing
//!   more, and **no Rivet machine has ever had a correct clock** — only one
//!   that does not go backwards.~~
//!
//!   **Superseded 2026-08-23**, and struck rather than deleted because
//!   `AGENTS.md` §7 is about exactly this. Both open questions were closed the
//!   same day: `ADR-021` §8 item 1 took `rsntp`, and §4.2.1 ruled that Rivet
//!   **always steps and never slews**, so there is no threshold. `cmd/rivet-timed`
//!   is the client and `cmd/rivet-sysd/src/definitions.rs` is its
//!   service definition.
//!
//!   **What is still true, stated narrowly rather than deleted with the rest:**
//!   *this module* corrects nothing and never will — it restores a floor, which
//!   is `ADR-021` §3's separate problem. And **no board has ever run
//!   `rivet-timed`**: `cmd/rivet-image` does not install it, so the
//!   definition ships `enabled: false` and no Rivet machine has yet had a
//!   correct clock. That is a fact about the image, not about this file.
//!
//! * **No knowledge of whether anything corrected the clock afterwards.**
//!   `rivet-timed` is a separate process this supervisor starts, and there is
//!   no channel back — so [`Clock::tick`] goes on writing
//!   `Origin::Unsynchronised` every hour even on a machine whose clock a time
//!   server set five minutes ago. Under-claiming is the safe direction
//!   (`rivet_store::clock::Origin` carries that argument), and the alternative
//!   — letting `rivet-timed` write `/state/clock` too — would be two writers of
//!   one file, which is what `ADR-011` exists to prevent. It is recorded as an
//!   open question rather than resolved here.
//! * ~~**No write on clean shutdown.** `ADR-021` §3 says the clock is written
//!   *"periodically and on clean shutdown"* and **never says by whom**.
//!   `ADR-017` §3.1.5 item 6 gives `SIGTERM` handling to PID 1, and §3.2's whole
//!   argument for keeping PID 1's list short cuts the other way. The periodic
//!   half needs no ruling and is built; the shutdown half is an open question and
//!   is left as one rather than guessed at.~~
//!
//!   **Ruled and built 2026-08-23 — `ADR-021` §3.3: this process writes it, on
//!   its own `SIGTERM`.** See [`Clock::persist_on_shutdown`]. Two reasons, and
//!   the second is why the question could not be settled earlier: this process
//!   is already the **sole writer** of `/state/clock` and a second one is what
//!   `ADR-011` exists to prevent; and until `ADR-017` §3.1.10 gave PID 1 a third
//!   signal there was **no clean shutdown to hang it on** — a Rivet board
//!   could only be stopped by removing power, so every shutdown this project has
//!   ever performed was unclean.

use rivet_store::clock::{ClockStore, Floor, Origin, Stamp};
use rivet_store::{Errno as StoreErrno, Filesystem, StoreError};

use crate::journal::{Journal, Severity, Value, rfc3339};
use crate::kernel::{Errno, Kernel};
use crate::supervise::SELF_NAME;

/// How often the running clock is written to the card.
///
/// **`ADR-021` §3.1 names hourly and the arithmetic supports it** against
/// `ADR-018` §7's card-wear concern, which is not a general worry but a specific
/// one: `ADR-018` §2 establishes that *all* of a Rivet machine's write load is
/// concentrated on this one partition, because `/system` is read-only and
/// everything else is a tmpfs.
///
/// | Interval | Writes a year | At a pessimistic 128 KiB of erase block per save |
/// | :-- | --: | --: |
/// | every minute | 525 600 | ~64 GiB |
/// | **hourly** | **8 760** | **~1.07 GiB** |
///
/// §3.1's own phrasing — *"every minute is over half a million small writes a
/// year"* — is the top row, and 525 600 is exactly that. The bottom row is 60×
/// less for a mechanism whose stated purpose is *approximately* right, on a
/// partition that also carries `ADR-022`'s journal, which is the write load that
/// actually matters.
///
/// **The cost is stated rather than buried:** an unclean shutdown loses up to
/// one hour of wall time. `ADR-021` §3.1 accepts exactly that — *"the point of
/// the mechanism is approximately right, and problem two corrects it"*.
///
/// **What is assumed and not measured:** the 128 KiB figure is a pessimistic
/// stand-in for `ext4` journaling plus the card's own erase-block write
/// amplification. Nothing has measured either on the card Rivet runs on, and
/// the error direction is deliberate — a real save is a ~26-byte file and almost
/// certainly costs far less.
pub const WRITE_INTERVAL_MS: u64 = 60 * 60 * 1000;

/// The mode the stored clock is written with.
///
/// `0644`: it is not a secret and an operator reading it is diagnosing a
/// machine. `ADR-018` §4's "root-owned" comes from PID 1 running as root and
/// from the process umask, exactly as it does for `/state/config`.
pub const STORED_CLOCK_MODE: u32 = 0o644;

/// `rivet-store`'s `Filesystem` port over this crate's [`Kernel`].
///
/// The read-only twin of `crate::document`'s `ReadOnlyStore`, and this one
/// writes — which is the difference worth stating, because that module argues at
/// length that *the supervisor is not an author of desired state*. It is not.
/// The stored clock is not desired state: nothing authors it, no operator edits
/// it (`ADR-008` §3.1 is why it is not in `/state/config`), and the supervisor
/// is the only process on the machine with both a wall clock and a timer loop.
///
/// Going through the port rather than through `rivet_store::StdFilesystem`
/// keeps every branch reachable from a test — the same argument
/// `crate::document` makes, and it bites harder here, because the states this
/// code must be right about are a `/state` that did not mount and a card that is
/// full.
struct StateFs<'k> {
    k: &'k mut dyn Kernel,
}

/// The two error vocabularies are separate types on purpose; on the target they
/// are the same numbers, so the conversion is the number and nothing else.
const fn to_store_errno(e: Errno) -> StoreErrno {
    StoreErrno(e.0)
}

impl Filesystem for StateFs<'_> {
    fn create_dir_all(&mut self, path: &str) -> Result<(), StoreErrno> {
        // Not reached by the clock store, which never creates a directory —
        // `/state` is a mount point that ships in the image. Implemented rather
        // than refused because a port implementation that lies about one method
        // is a trap for the next caller.
        self.k.mkdir_p(path).map_err(to_store_errno)
    }

    fn read(&mut self, path: &str) -> Result<Vec<u8>, StoreErrno> {
        self.k
            .read_file(path)
            .map(String::into_bytes)
            .map_err(to_store_errno)
    }

    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), StoreErrno> {
        self.k
            .write_file(path, bytes, STORED_CLOCK_MODE)
            .map_err(to_store_errno)
    }

    fn sync_file(&mut self, path: &str) -> Result<(), StoreErrno> {
        self.k.sync_file(path).map_err(to_store_errno)
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), StoreErrno> {
        self.k.rename(from, to).map_err(to_store_errno)
    }

    fn sync_dir(&mut self, path: &str) -> Result<(), StoreErrno> {
        self.k.sync_dir(path).map_err(to_store_errno)
    }

    fn remove_file(&mut self, path: &str) -> Result<(), StoreErrno> {
        self.k.remove_file(path).map_err(to_store_errno)
    }
}

/// The stored clock, as the supervisor holds it across a run.
///
/// Two pieces of state, and each exists to stop a specific failure:
///
/// * `due_mono_ms` is a **monotonic** deadline. `ADR-021` §5 is the rule a time
///   ADR exists to state — *wall-clock time is for timestamps,
///   `CLOCK_MONOTONIC` is for durations* — and this is the one place in this
///   crate where getting it wrong would be self-inflicted: the restore itself
///   moves the wall clock, so an hourly deadline expressed in wall time would
///   fire immediately or in fifty-five years depending on which side of the jump
///   it was computed on.
/// * `floor_ms` is the highest wall time this machine has established, and it is
///   what stops a write **lowering** the stored value. See [`Clock::tick`].
pub struct Clock {
    store: ClockStore,
    due_mono_ms: Option<u64>,
    floor_ms: Option<u64>,
    /// Whether [`Clock::persist_on_shutdown`] has already run this process.
    ///
    /// A second `SIGTERM` is ordinary — an impatient operator, or PID 1 escalating
    /// — and it must not become a second write to the card at the exact moment
    /// the machine is going down.
    wrote_on_shutdown: bool,
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock {
    /// The clock at `/state/clock` — `rivet_store::ClockStore`'s own default,
    /// so the path is spelled in one place.
    pub fn new() -> Self {
        Self {
            store: ClockStore::at_default(),
            due_mono_ms: None,
            floor_ms: None,
            wrote_on_shutdown: false,
        }
    }

    /// `ADR-021` §3's restore, as the supervisor's start-up performs it.
    ///
    /// # Where this sits in the start-up order, which is a contract
    ///
    /// `crate::run`'s table has it second, and both neighbours are load-bearing:
    ///
    /// * **After the journal exists**, so a failure to restore can be reported.
    ///   `ADR-017` §8 item 2 is the general form of that rule — a thing that can
    ///   fail must not be the thing the record of its failure depends on.
    /// * **Before the first record is stamped.** Every record carries a wall
    ///   clock (`ADR-022` §3.2), so a restore that happened one line later would
    ///   leave the supervisor's own start-up record stamped 1970 for ever. This
    ///   is the whole point of the change, seen from the console.
    ///
    /// # It carries on through every failure
    ///
    /// There is no arm of this that stops the boot, and the argument is
    /// `ADR-018` §8's, which `crate::document` already applies to an unreadable
    /// desired-state document: **a machine that is reachable is one somebody can
    /// repair.** A wrong clock is not worth refusing to start the control plane
    /// over, and the fault is on `/state`, which no update and no rollback
    /// touches — so there
    /// is nothing an escalation could reach that would fix it.
    pub fn restore(&mut self, k: &mut dyn Kernel, journal: &mut Journal) {
        let path = self.store.path();

        // Reborrowed rather than moved, because `k` is wanted again to journal
        // what happened. The adapter lives only for the read.
        let stored = {
            let mut fs = StateFs { k: &mut *k };
            self.store.load(&mut fs)
        };

        let stored = match stored {
            Ok(stored) => stored,
            Err(why) => {
                note(
                    k,
                    journal,
                    Severity::Error,
                    "DEGRADED the stored clock could not be read, so there is no floor to restore \
                     and this machine's wall clock is whatever the kernel started with — on a Pi, \
                     1970 (ADR-021 §1). Every record this boot writes is stamped from it. Carrying \
                     on rather than stopping: the fault is on /state, which no update or rollback \
                     touches (ADR-018 §8), and a machine that can be reached is one somebody can \
                     repair",
                    vec![
                        ("path".to_string(), Value::Str(path.clone())),
                        ("detail".to_string(), Value::Str(why.to_string())),
                    ],
                );
                // Deliberately treated as "no floor" rather than as "write over
                // it": overwriting a stored clock this process could not read is
                // the same class of decision `rivet_store::ConfigStore::seed`
                // refuses to take about a damaged document, and `ADR-018` §9
                // item 3 carries it as open. So nothing is scheduled — see the
                // early return below.
                self.due_mono_ms = None;
                return;
            }
        };

        let Some(now_ms) = k.wall_ms() else {
            note(
                k,
                journal,
                Severity::Error,
                "DEGRADED there is no readable wall clock on this machine, so ADR-021 §3's floor \
                 cannot be applied and nothing can be persisted. Every journal record this boot \
                 writes will have no ts field at all (ADR-022 §3.2)",
                vec![("path".to_string(), Value::Str(path))],
            );
            self.due_mono_ms = None;
            return;
        };

        let floor = Floor::decide(stored, now_ms);

        match floor {
            Floor::NothingStored { .. } => note(
                k,
                journal,
                Severity::Warn,
                "no clock is stored, so nothing is restored and this machine believes whatever the \
                 kernel started it with. Normal on a first boot; on a Pi it means 1970 until \
                 something sets the clock (ADR-021 §1, §4). The current reading is written to the \
                 card now, so a power cycle is not a total loss",
                vec![
                    ("path".to_string(), Value::Str(path.clone())),
                    ("wall_clock".to_string(), Value::Str(rfc3339(now_ms))),
                ],
            ),
            Floor::Raised {
                from_ms,
                to_ms,
                stored_origin,
            } => match k.set_wall_ms(to_ms) {
                Ok(()) => note(
                    k,
                    journal,
                    Severity::Info,
                    "the wall clock is restored to the floor the card recorded (ADR-021 §3). It is \
                     NOT correct time — it is the last hour this machine is known to have been \
                     running. Correct time is rivet-timed's (ADR-021 §4), which runs later and \
                     only if the image carries it and the document enables it",
                    vec![
                        ("path".to_string(), Value::Str(path.clone())),
                        ("was".to_string(), Value::Str(rfc3339(from_ms))),
                        ("now".to_string(), Value::Str(rfc3339(to_ms))),
                        ("moved_forward_ms".to_string(), int(to_ms - from_ms)),
                        (
                            "stored_origin".to_string(),
                            Value::Str(stored_origin.to_string()),
                        ),
                    ],
                ),
                Err(e) => {
                    note(
                        k,
                        journal,
                        Severity::Error,
                        "DEGRADED the wall clock could not be set, so this machine keeps whatever \
                         the kernel started it with and every record this boot writes is stamped \
                         from it. Setting the clock needs CAP_SYS_TIME, so EPERM here means this \
                         process is not running with it",
                        vec![
                            ("path".to_string(), Value::Str(path.clone())),
                            ("wanted".to_string(), Value::Str(rfc3339(to_ms))),
                            ("errno".to_string(), Value::Int(i64::from(e.0))),
                            ("detail".to_string(), Value::Str(e.to_string())),
                        ],
                    );
                    // **Nothing is scheduled.** The wall clock still reads
                    // `from_ms`, which is BELOW the stored value; persisting it
                    // would lower the floor and throw away every hour the card
                    // had recorded. `Clock::tick`'s guard would catch it anyway;
                    // this makes it impossible rather than caught.
                    self.due_mono_ms = None;
                    return;
                }
            },
            Floor::AlreadyAhead {
                stored_ms,
                now_ms: running_ms,
                stored_origin,
            } => note(
                k,
                journal,
                Severity::Info,
                "the wall clock is already at or past the stored floor, so it is NOT set — \
                 ADR-021 §3 rules a floor, and a machine that knows a later time than its card \
                 must not be moved backwards onto it",
                vec![
                    ("path".to_string(), Value::Str(path.clone())),
                    ("stored".to_string(), Value::Str(rfc3339(stored_ms))),
                    ("running".to_string(), Value::Str(rfc3339(running_ms))),
                    (
                        "stored_origin".to_string(),
                        Value::Str(stored_origin.to_string()),
                    ),
                ],
            ),
        }

        self.floor_ms = Some(floor.resulting_ms());

        // Said once, at boot, because it is the reason a certificate check will
        // still fail on a board whose journal has just started looking
        // plausible — `AGENTS.md` §6, "wired up" is never reported as "works".
        //
        // ~~"ADR-021 §4's SNTP client is not built and no crate for it has been
        // taken (ADR-021 §8 item 1)"~~ — that clause was true when it was
        // written and stopped being true on 2026-08-23, when `cmd/rivet-timed`
        // was built on `rsntp`. The *record* below is corrected rather than
        // deleted, because what it warns about is unchanged at the moment it is
        // written: nothing has corrected this clock **yet**, and this process
        // will never learn whether anything does.
        note(
            k,
            journal,
            Severity::Warn,
            "this machine's clock has NEVER been synchronised against an external source at this \
             point in the boot, and this process cannot learn otherwise later: ADR-021 §4's client \
             is rivet-timed, a separate process, with no channel back. Until it has run and said \
             so on this console, certificate expiry and signature freshness cannot be trusted, and \
             the restored time is as wrong as the last boot's was plus however long the machine \
             was off. Every hourly write below will say `unsynchronised` whatever rivet-timed \
             does, because under-claiming is the safe direction",
            vec![(
                "origin".to_string(),
                Value::Str(Origin::Unsynchronised.to_string()),
            )],
        );

        let now_mono = k.now_ms();
        self.due_mono_ms = Some(if floor.worth_persisting_now() {
            // This boot knows something the card does not: either nothing was
            // stored at all, or the running clock was ahead of what was. Both
            // are worth a write now rather than in an hour, because the ordinary
            // way a Pi is switched off is a power cut.
            now_mono
        } else {
            // A raise writes nothing back: the clock was just set FROM the card,
            // so persisting it would be a write that changes nothing on the one
            // partition carrying all of this machine's wear (`ADR-018` §7).
            now_mono.saturating_add(WRITE_INTERVAL_MS)
        });
    }

    /// `ADR-021` §3's *"and on clean shutdown"* — ruled 2026-08-23, `ADR-021`
    /// §3.3.
    ///
    /// # Why this process rather than PID 1
    ///
    /// `ADR-021` §3 says the clock is written *"periodically and on clean
    /// shutdown"* and never says by whom. It is this process, for two reasons
    /// that point the same way:
    ///
    /// 1. **It is already the sole writer of `/state/clock`**, and two writers
    ///    of one file is exactly what `ADR-011` exists to prevent. PID 1 writing
    ///    it would need this module's floor, this module's `Origin` reasoning
    ///    and this module's crash-safe sequence, all restated in the process
    ///    whose failure panics the kernel.
    /// 2. **`ADR-017` §3.2's argument for keeping PID 1's job list short still
    ///    stands**, and §3.1.10 has just grown it by a signal.
    ///
    /// # Why it runs when the shutdown *starts* rather than when it finishes
    ///
    /// PID 1 stops this process first and only then syncs and reboots
    /// (`cmd/rivet-initd`'s `Pid1::shut_down`), so there **is** a window — but
    /// it is bounded by that crate's `STOP_GRACE_MS`, which is 5s, and this
    /// process may take `stop_grace_ms` + `kill_grace_ms` (12s by default) to
    /// bring down a service that ignores `SIGTERM`. Writing at the end of the
    /// shutdown would therefore be a write that a badly-behaved service can
    /// cancel. Writing at the start costs the few seconds the shutdown takes,
    /// against a mechanism whose stated granularity is an hour.
    ///
    /// # It writes nothing when this boot never established a floor
    ///
    /// `due_mono_ms` is `None` in exactly the three states [`Clock::restore`]
    /// refuses to schedule from — an unreadable stored clock, no wall clock at
    /// all, and a `clock_settime` the kernel refused — and every one of them is
    /// a state where writing would *lose* information rather than save it. A
    /// shutdown is not a reason to relax that, so the check is a plain early
    /// return and the reason is said on the console.
    pub fn persist_on_shutdown(&mut self, k: &mut dyn Kernel, journal: &mut Journal) {
        if self.wrote_on_shutdown {
            return;
        }
        self.wrote_on_shutdown = true;

        if self.due_mono_ms.is_none() {
            note(
                k,
                journal,
                Severity::Warn,
                "nothing is written to the stored clock on this shutdown, because this boot never \
                 established a floor to write from — see the DEGRADED record earlier in this boot. \
                 The next boot restores from whatever was stored before this one, or from nothing \
                 (ADR-021 §3.3)",
                vec![("path".to_string(), Value::Str(self.store.path()))],
            );
            return;
        }

        note(
            k,
            journal,
            Severity::Info,
            "shutdown requested; writing the running clock to the card now rather than at the hour \
             (ADR-021 §3, §3.3). This is the half of the mechanism an unclean shutdown loses",
            vec![("path".to_string(), Value::Str(self.store.path()))],
        );

        // Through `tick` rather than around it: the floor guard, the `Origin`
        // decision and the crash-safe sequence are all obligations of a write,
        // not of a schedule, and a shutdown path with its own copy of them is a
        // second implementation that only ever runs when nobody is watching.
        self.due_mono_ms = Some(k.now_ms());
        self.tick(k, journal);
    }

    /// **`apply clock-floor-reset`: make the running clock the floor** —
    /// `rivet_core::operation::clock_floor`, `ADR-021` §3. `apply` is `false`
    /// for the plan, which changes nothing and answers what would happen.
    ///
    /// The floor only ever rises on its own, so a clock once stepped to a
    /// wrong future is restored to it at every boot after. This is the one way
    /// it comes down: the operator asks. The new floor is written to the card
    /// first and taken in memory only once it is there — or committed and not
    /// yet flushed, which [`Self::tick`] treats the same way — so a refusal
    /// leaves the floor this process held, in memory and on the card.
    ///
    /// It also re-arms the hourly write on a boot that had stopped it because
    /// the stored clock could not be read: an operator writing a new floor is
    /// the decision that boot was declining to take on its own.
    pub fn reset_floor(
        &mut self,
        k: &mut dyn Kernel,
        journal: &mut Journal,
        apply: bool,
    ) -> Result<
        rivet_core::operation::clock_floor::Floor,
        rivet_core::operation::clock_floor::Refusal,
    > {
        use rivet_core::operation::clock_floor::{Floor as Reset, Refusal};

        let Some(now_ms) = k.wall_ms() else {
            return Err(Refusal::NoWallClock);
        };
        let answer = Reset {
            was_ms: self.floor_ms,
            now_ms,
        };
        if !apply {
            return Ok(answer);
        }

        let path = self.store.path();
        let stamp = Stamp {
            unix_ms: now_ms,
            origin: Origin::Unsynchronised,
        };
        let outcome = {
            let mut fs = StateFs { k: &mut *k };
            self.store.save(&mut fs, stamp)
        };
        match outcome {
            Ok(()) | Err(StoreError::NotDurable { .. }) => {}
            Err(why) => {
                return Err(Refusal::NotWritten {
                    detail: why.to_string(),
                });
            }
        }
        self.floor_ms = Some(now_ms);
        self.due_mono_ms = Some(k.now_ms().saturating_add(WRITE_INTERVAL_MS));
        note(
            k,
            journal,
            if answer.lowers() {
                Severity::Warn
            } else {
                Severity::Info
            },
            "the clock floor was reset to the running clock by an operator (ADR-021 §3). The next \
             boot restores the clock to no earlier than this, and the floor rises from here, an \
             hour at a time",
            vec![
                ("path".to_string(), Value::Str(path)),
                (
                    "was".to_string(),
                    Value::Str(answer.was_ms.map_or_else(|| "none".to_string(), rfc3339)),
                ),
                ("now".to_string(), Value::Str(rfc3339(now_ms))),
                ("lowered".to_string(), Value::Bool(answer.lowers())),
            ],
        );
        Ok(answer)
    }

    /// How long until this module has work, in milliseconds, or `None` when it
    /// has none.
    ///
    /// The same shape as `Supervisor::next_timeout_ms`, so `crate::run` can take
    /// the sooner of the two and keep one `poll(2)`.
    pub fn next_timeout_ms(&self, now_mono_ms: u64) -> Option<u64> {
        self.due_mono_ms
            .map(|due| due.saturating_sub(now_mono_ms))
    }

    /// Write the running clock to the card, if an hour has passed.
    ///
    /// # The one invariant this module owns
    ///
    /// **The stored value must never decrease.** `rivet_store::ClockStore`
    /// cannot enforce it — it is handed a stamp and has no way to know where the
    /// clock came from, and a store that refused a lower value would also refuse
    /// the correction `ADR-021` §4.2 reserves for the maintainer. So the guard
    /// is here, against the floor this boot established, and what it protects
    /// against is concrete: a machine whose `clock_settime` was refused reads
    /// 1970 from its wall clock, and persisting *that* would throw away every
    /// hour the card had recorded.
    pub fn tick(&mut self, k: &mut dyn Kernel, journal: &mut Journal) {
        let Some(due) = self.due_mono_ms else { return };
        if k.now_ms() < due {
            return;
        }
        // Rescheduled before anything can fail, so that no failure path can leave
        // the machine writing on every wakeup — which on `/state` is the
        // restart-loop hazard `ADR-017` §3.5 exists for, pointed at a card.
        self.due_mono_ms = Some(k.now_ms().saturating_add(WRITE_INTERVAL_MS));

        let path = self.store.path();

        let Some(now_ms) = k.wall_ms() else {
            note(
                k,
                journal,
                Severity::Warn,
                "the wall clock could not be read, so nothing was written to the stored clock this \
                 hour",
                vec![("path".to_string(), Value::Str(path))],
            );
            return;
        };

        if let Some(floor) = self.floor_ms
            && now_ms < floor
        {
            note(
                k,
                journal,
                Severity::Warn,
                "the wall clock is BEHIND the floor this boot established, so nothing is written: \
                 persisting it would lower the stored clock and throw away time the card already \
                 knows about (ADR-021 §3 — the machine never believes it is earlier than when it \
                 last ran). Something moved the clock backwards while this process was running, \
                 and since 2026-08-23 there is a first thing to suspect: rivet-timed steps in \
                 BOTH directions (ADR-021 §4.2.1), so a machine whose stored floor was ahead of \
                 real time will produce this record exactly once and then settle",
                vec![
                    ("path".to_string(), Value::Str(path)),
                    ("floor".to_string(), Value::Str(rfc3339(floor))),
                    ("running".to_string(), Value::Str(rfc3339(now_ms))),
                ],
            );
            return;
        }

        let stamp = Stamp {
            unix_ms: now_ms,
            // **Never `Synchronised`.** ~~Nothing in Rivet can synchronise a
            // clock, so nothing it writes may claim it did~~ — **the premise
            // changed on 2026-08-23 and the conclusion did not.**
            // `cmd/rivet-timed` can now synchronise a clock; this process
            // still cannot *observe* that it did, because it is a different
            // process with no channel back. So the honest stamp is unchanged
            // and the reason is different, which is worth writing down rather
            // than leaving a sentence that is no longer true.
            //
            // The specific hazard is compounding: a value laundered through
            // many boots of `max(stored, now)` must not become
            // indistinguishable from one a time server supplied.
            // `rivet_store::clock::Origin` carries the argument.
            origin: Origin::Unsynchronised,
        };

        let outcome = {
            let mut fs = StateFs { k: &mut *k };
            self.store.save(&mut fs, stamp)
        };

        match outcome {
            Ok(()) => {
                self.floor_ms = Some(now_ms);
                note(
                    k,
                    journal,
                    Severity::Info,
                    "the running clock is written to the card, so the next boot has a floor to \
                     restore from (ADR-021 §3)",
                    vec![
                        ("path".to_string(), Value::Str(path)),
                        ("wall_clock".to_string(), Value::Str(rfc3339(now_ms))),
                        (
                            "origin".to_string(),
                            Value::Str(stamp.origin.to_string()),
                        ),
                        ("next_write_ms".to_string(), int(WRITE_INTERVAL_MS)),
                    ],
                );
            }
            Err(StoreError::NotDurable { errno, .. }) => {
                // Past the commit: the value IS on the card and may not survive a
                // power cut in the next few seconds. Reporting that as a failure
                // would say nothing happened, which is false — so the floor is
                // advanced exactly as it is on success.
                self.floor_ms = Some(now_ms);
                note(
                    k,
                    journal,
                    Severity::Warn,
                    "the clock was written and committed and the directory entry could not be \
                     flushed to the card. A power cut in the next few seconds may lose it; the \
                     previous stored value would then stand",
                    vec![
                        ("path".to_string(), Value::Str(path)),
                        ("wall_clock".to_string(), Value::Str(rfc3339(now_ms))),
                        ("errno".to_string(), Value::Int(i64::from(errno.0))),
                    ],
                );
            }
            Err(why) => note(
                k,
                journal,
                Severity::Error,
                "DEGRADED the clock could not be written, so the next boot restores from whatever \
                 was stored before this one — or from nothing. The machine keeps running; a wrong \
                 clock is not worth losing a board over (ADR-018 §8)",
                vec![
                    ("path".to_string(), Value::Str(path)),
                    ("detail".to_string(), Value::Str(why.to_string())),
                ],
            ),
        }
    }
}

/// Say something in the supervisor's own voice. The same helper
/// `crate::document` and `crate::etc` keep, for the same reason: these records
/// are emitted before there is a `Supervisor` to speak through, and in `tick`'s
/// case from a place that holds the journal directly.
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

fn int(n: u64) -> Value {
    Value::Int(i64::try_from(n).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::supervise::ACTOR;
    use crate::testkernel::{FakeKernel, Script};

    /// A plausible wall time, hand-composed and arbitrary. `AGENTS.md` §8.3
    /// keeps the dates of real events out of tracked files, so this is a round
    /// number and not an observation.
    const A_PLAUSIBLE_MS: u64 = 1_700_000_000_000;

    /// What a Raspberry Pi 4 actually came up at — the 1970-clock finding, 2026-08-23 records the
    /// stamp `1970-01-01T00:00:03` on every record the first board wrote.
    const WHAT_A_COLD_PI_BELIEVES_MS: u64 = 3_000;

    const STORED_PATH: &str = "/state/clock";

    fn journal() -> Journal {
        Journal::new("b".to_string(), 1, ACTOR.to_string())
    }

    /// A machine with a stored clock and a wall clock at the epoch: the ordinary
    /// second boot of a Pi.
    fn a_pi_with_a_stored_clock(stored_ms: u64) -> FakeKernel {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(WHAT_A_COLD_PI_BELIEVES_MS);
        k.files_mut().insert(
            STORED_PATH.to_string(),
            Script::always(Ok(format!("{stored_ms} unsynchronised\n"))),
        );
        k
    }

    fn said(k: &FakeKernel, needle: &str) -> bool {
        k.records.iter().any(|r| r.msg.contains(needle))
    }

    // -- the restore ---------------------------------------------------------

    /// **The whole change, from the outside.** A board with a stored clock comes
    /// up believing it is 1970 and, before it writes anything else, believes the
    /// hour the card recorded instead.
    #[test]
    fn a_boot_with_a_stored_clock_leaves_the_machine_believing_the_stored_hour() {
        let mut k = a_pi_with_a_stored_clock(A_PLAUSIBLE_MS);
        let mut j = journal();
        Clock::new().restore(&mut k, &mut j);
        assert_eq!(k.wall_sets, vec![A_PLAUSIBLE_MS]);
        assert_eq!(k.wall_ms, Some(A_PLAUSIBLE_MS));
    }

    /// ...and the records it writes carry that time rather than 1970, which is
    /// the thing the 1970-clock finding, 2026-08-23 is actually complaining about.
    #[test]
    fn every_record_after_the_restore_is_stamped_from_the_restored_clock() {
        let mut k = a_pi_with_a_stored_clock(A_PLAUSIBLE_MS);
        let mut j = journal();
        Clock::new().restore(&mut k, &mut j);
        let corr = j.next_correlation();
        j.emit(&mut k, Severity::Info, "svc", &corr, "anything", vec![]);
        let Some(last) = k.records.last() else {
            unreachable!("nothing was recorded")
        };
        let Some(ts) = &last.ts else {
            unreachable!("a record with no timestamp")
        };
        assert!(!ts.starts_with("1970"), "{ts}");
        assert_eq!(ts, &rfc3339(A_PLAUSIBLE_MS));
    }

    /// **The arm an unconditional `clock_settime` gets wrong.** `ADR-021` §3
    /// rules a floor; §3.2 restates it as a set. A machine with a battery-backed
    /// clock — `PLAN.md` §9.2's x86 target — must not be moved backwards onto a
    /// stale card value.
    #[test]
    fn a_machine_whose_clock_is_ahead_of_the_card_is_not_moved_backwards() {
        let mut k = a_pi_with_a_stored_clock(A_PLAUSIBLE_MS);
        k.wall_ms = Some(A_PLAUSIBLE_MS + 86_400_000);
        let mut j = journal();
        Clock::new().restore(&mut k, &mut j);
        assert!(k.wall_sets.is_empty(), "a good clock was moved backwards");
        assert_eq!(k.wall_ms, Some(A_PLAUSIBLE_MS + 86_400_000));
        assert!(said(&k, "already at or past the stored floor"), "{:?}", k.records);
    }

    /// ...and that machine writes what it knows immediately, because it knows a
    /// later time than the card does.
    #[test]
    fn a_machine_ahead_of_the_card_writes_what_it_knows_at_once() {
        let mut k = a_pi_with_a_stored_clock(A_PLAUSIBLE_MS);
        k.wall_ms = Some(A_PLAUSIBLE_MS + 86_400_000);
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);
        assert_eq!(clock.next_timeout_ms(k.mono_ms), Some(0));
        clock.tick(&mut k, &mut j);
        assert_eq!(
            k.contents(STORED_PATH),
            Some(format!("{} unsynchronised\n", A_PLAUSIBLE_MS + 86_400_000))
        );
    }

    /// A first boot: nothing is stored, nothing is restored, and no time is
    /// invented. The file is created at once so a power cycle an hour later is
    /// not a total loss.
    #[test]
    fn a_first_boot_restores_nothing_and_creates_the_file_immediately() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(WHAT_A_COLD_PI_BELIEVES_MS);
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);
        assert!(k.wall_sets.is_empty(), "a first boot invented a time");
        assert!(said(&k, "no clock is stored"), "{:?}", k.records);
        assert_eq!(clock.next_timeout_ms(k.mono_ms), Some(0));
        clock.tick(&mut k, &mut j);
        assert_eq!(
            k.contents(STORED_PATH),
            Some(format!("{WHAT_A_COLD_PI_BELIEVES_MS} unsynchronised\n"))
        );
    }

    /// A raise writes nothing back: the clock was just set from the card, so a
    /// boot-time write would change nothing and cost a write on the one
    /// partition carrying all of this machine's wear (`ADR-018` §7).
    #[test]
    fn an_ordinary_pi_boot_writes_nothing_until_the_hour() {
        let mut k = a_pi_with_a_stored_clock(A_PLAUSIBLE_MS);
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);
        assert_eq!(clock.next_timeout_ms(k.mono_ms), Some(WRITE_INTERVAL_MS));
        clock.tick(&mut k, &mut j);
        assert!(k.writes.is_empty(), "{:?}", k.writes);
    }

    /// A clock that could not be set leaves the machine below the stored floor,
    /// and **nothing may be written** — persisting 1970 would throw away every
    /// hour the card had recorded.
    #[test]
    fn a_refused_clock_set_degrades_loudly_and_never_lowers_the_stored_value() {
        let mut k = a_pi_with_a_stored_clock(A_PLAUSIBLE_MS);
        k.set_wall_results = Script::always(Err(Errno::EPERM));
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);

        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Error && r.msg.starts_with("DEGRADED")),
            "{:?}",
            k.records
        );
        assert_eq!(k.wall_ms, Some(WHAT_A_COLD_PI_BELIEVES_MS));
        assert_eq!(clock.next_timeout_ms(k.mono_ms), None, "a write was scheduled anyway");
        k.mono_ms = WRITE_INTERVAL_MS * 10;
        clock.tick(&mut k, &mut j);
        assert!(k.writes.is_empty(), "1970 was persisted over the stored hour");
    }

    /// A card returning `EIO` is not "nothing stored". Reading it that way would
    /// give a machine that leaves its clock at 1970 and then overwrites the one
    /// good value it had.
    #[test]
    fn an_unreadable_stored_clock_degrades_and_never_overwrites_itself() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(WHAT_A_COLD_PI_BELIEVES_MS);
        k.files_mut()
            .insert(STORED_PATH.to_string(), Script::always(Err(Errno::EIO)));
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);
        assert!(said(&k, "stored clock could not be read"), "{:?}", k.records);
        assert!(k.wall_sets.is_empty());
        assert_eq!(clock.next_timeout_ms(k.mono_ms), None);
        k.mono_ms = WRITE_INTERVAL_MS * 10;
        clock.tick(&mut k, &mut j);
        assert!(k.writes.is_empty(), "a clock nothing could read was overwritten");
    }

    /// A stored clock in a shape this build does not recognise is refused, not
    /// half-read. Something else wrote it, or a flash block is half erased
    /// (`ADR-018` §8).
    #[test]
    fn a_stored_clock_that_will_not_parse_is_refused_rather_than_partly_believed() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(WHAT_A_COLD_PI_BELIEVES_MS);
        k.files_mut().insert(
            STORED_PATH.to_string(),
            Script::always(Ok("1700000000000\n".to_string())),
        );
        let mut j = journal();
        Clock::new().restore(&mut k, &mut j);
        assert!(k.wall_sets.is_empty(), "a malformed file set the clock");
        assert!(said(&k, "stored clock could not be read"), "{:?}", k.records);
    }

    /// A machine with no readable wall clock at all: nothing to floor, nothing to
    /// store, and it says so rather than failing.
    #[test]
    fn a_machine_with_no_wall_clock_says_so_and_carries_on() {
        let mut k = a_pi_with_a_stored_clock(A_PLAUSIBLE_MS);
        k.wall_ms = None;
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);
        assert!(said(&k, "no readable wall clock"), "{:?}", k.records);
        assert_eq!(clock.next_timeout_ms(0), None);
    }

    /// Said once, at boot: nothing has ever synchronised this clock. It is the
    /// reason a certificate check still fails on a board whose journal has just
    /// started looking plausible.
    #[test]
    fn the_restore_says_out_loud_that_nothing_has_ever_synchronised_this_clock() {
        let mut k = a_pi_with_a_stored_clock(A_PLAUSIBLE_MS);
        let mut j = journal();
        Clock::new().restore(&mut k, &mut j);
        let saying: Vec<_> = k
            .records
            .iter()
            .filter(|r| r.msg.contains("NEVER been synchronised"))
            .collect();
        assert_eq!(saying.len(), 1, "{:?}", k.records);
        assert_eq!(saying[0].sev, Severity::Warn);
    }

    // -- the periodic write --------------------------------------------------

    /// Hourly, and not a minute before. `ADR-021` §3.1 and `ADR-018` §7 — this
    /// is the interval decision, as an assertion.
    #[test]
    fn the_write_happens_on_the_hour_and_not_before_it() {
        let mut k = a_pi_with_a_stored_clock(A_PLAUSIBLE_MS);
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);

        k.mono_ms = WRITE_INTERVAL_MS - 1;
        clock.tick(&mut k, &mut j);
        assert!(k.writes.is_empty(), "it wrote a millisecond early");

        k.mono_ms = WRITE_INTERVAL_MS;
        k.wall_ms = Some(A_PLAUSIBLE_MS + WRITE_INTERVAL_MS);
        clock.tick(&mut k, &mut j);
        assert_eq!(
            k.contents(STORED_PATH),
            Some(format!(
                "{} unsynchronised\n",
                A_PLAUSIBLE_MS + WRITE_INTERVAL_MS
            ))
        );
        assert_eq!(clock.next_timeout_ms(k.mono_ms), Some(WRITE_INTERVAL_MS));
    }

    /// Three hours of uptime is three writes, not three hundred. The deadline is
    /// monotonic, so it is not disturbed by the restore having moved the wall
    /// clock (`ADR-021` §5).
    #[test]
    fn a_long_uptime_writes_once_an_hour_and_the_deadline_is_monotonic() {
        let mut k = a_pi_with_a_stored_clock(A_PLAUSIBLE_MS);
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);

        for hour in 1..=3 {
            // The wakeups a real supervisor makes: several per hour, for reasons
            // that have nothing to do with the clock.
            for step in 0..4 {
                k.mono_ms = (hour - 1) * WRITE_INTERVAL_MS + step * (WRITE_INTERVAL_MS / 4);
                k.wall_ms = Some(A_PLAUSIBLE_MS + k.mono_ms);
                clock.tick(&mut k, &mut j);
            }
            k.mono_ms = hour * WRITE_INTERVAL_MS;
            k.wall_ms = Some(A_PLAUSIBLE_MS + k.mono_ms);
            clock.tick(&mut k, &mut j);
        }
        assert_eq!(k.writes.len(), 3, "{:?}", k.writes);
    }

    /// The invariant this module owns: a clock that went backwards while running
    /// is **not** persisted, because doing so would lower the floor.
    #[test]
    fn a_clock_that_went_backwards_while_running_is_not_written() {
        let mut k = a_pi_with_a_stored_clock(A_PLAUSIBLE_MS);
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);

        k.mono_ms = WRITE_INTERVAL_MS;
        k.wall_ms = Some(A_PLAUSIBLE_MS - 1);
        clock.tick(&mut k, &mut j);
        assert!(k.writes.is_empty(), "{:?}", k.writes);
        assert!(said(&k, "BEHIND the floor"), "{:?}", k.records);
        // ...and the next hour, with the clock recovered, writes normally.
        k.mono_ms = WRITE_INTERVAL_MS * 2;
        k.wall_ms = Some(A_PLAUSIBLE_MS + WRITE_INTERVAL_MS);
        clock.tick(&mut k, &mut j);
        assert_eq!(k.writes.len(), 1);
    }

    // -- D5: the operator lowers the floor -----------------------------------

    /// 2200-01-01T00:00:00Z: a floor one lying server put on the card.
    const THE_YEAR_2200_MS: u64 = 7_258_118_400_000;

    /// **The plan changes nothing**, and says the floor and the running clock.
    #[test]
    fn a_floor_reset_plan_writes_nothing_and_names_both_times() {
        let mut k = a_pi_with_a_stored_clock(THE_YEAR_2200_MS);
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);
        let writes = k.writes.len();
        k.wall_ms = Some(A_PLAUSIBLE_MS);

        let plan = clock.reset_floor(&mut k, &mut j, false).expect("a plan");
        assert_eq!(plan.was_ms, Some(THE_YEAR_2200_MS));
        assert_eq!(plan.now_ms, A_PLAUSIBLE_MS);
        assert!(plan.lowers());
        assert_eq!(k.writes.len(), writes, "a plan wrote: {:?}", k.writes);
    }

    /// **A confirmed reset lowers the floor, on the card and in memory**, so
    /// the next hour's write goes through instead of being refused as behind
    /// the floor, and the next boot restores the corrected time.
    #[test]
    fn a_confirmed_floor_reset_lowers_the_floor_and_the_hourly_write_resumes() {
        let mut k = a_pi_with_a_stored_clock(THE_YEAR_2200_MS);
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);
        // Something corrected the running clock back to the present.
        k.wall_ms = Some(A_PLAUSIBLE_MS);

        let done = clock.reset_floor(&mut k, &mut j, true).expect("reset");
        assert!(done.lowers());
        let written = k
            .writes
            .iter()
            .rev()
            .find(|w| w.path.starts_with(STORED_PATH))
            .map(|w| String::from_utf8_lossy(&w.contents).into_owned())
            .expect("the new floor was written");
        assert!(written.starts_with(&A_PLAUSIBLE_MS.to_string()), "{written:?}");
        assert!(said(&k, "reset to the running clock by an operator"), "{:?}", k.records);

        let writes = k.writes.len();
        k.mono_ms = WRITE_INTERVAL_MS + 1;
        k.wall_ms = Some(A_PLAUSIBLE_MS + WRITE_INTERVAL_MS);
        clock.tick(&mut k, &mut j);
        assert!(k.writes.len() > writes, "the hourly write was still refused");
        assert!(!said(&k, "BEHIND the floor"), "{:?}", k.records);
    }

    /// **A write that fails changes nothing**: the floor stays where it was.
    #[test]
    fn a_floor_reset_that_cannot_be_written_keeps_the_old_floor() {
        let mut k = a_pi_with_a_stored_clock(THE_YEAR_2200_MS);
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);
        k.wall_ms = Some(A_PLAUSIBLE_MS);
        k.write_results = Script::always(Err(Errno::ENOSPC));

        let refused = clock.reset_floor(&mut k, &mut j, true).expect_err("refused");
        assert_eq!(refused.code(), "not-written");
        let again = clock.reset_floor(&mut k, &mut j, false).expect("a plan");
        assert_eq!(again.was_ms, Some(THE_YEAR_2200_MS));
    }

    /// No wall clock, nothing to reset to.
    #[test]
    fn a_floor_reset_with_no_wall_clock_is_refused() {
        let mut k = FakeKernel::new();
        k.wall_ms = None;
        let mut j = journal();
        let refused = Clock::new()
            .reset_floor(&mut k, &mut j, true)
            .expect_err("refused");
        assert_eq!(refused.code(), "no-wall-clock");
        assert!(k.writes.is_empty());
    }

    /// A write that fails does not make the machine try again on every wakeup —
    /// which on `/state` is `ADR-017` §3.5's restart-loop hazard pointed at a
    /// card.
    #[test]
    fn a_failed_write_backs_off_to_the_next_hour_rather_than_retrying_every_wakeup() {
        let mut k = a_pi_with_a_stored_clock(A_PLAUSIBLE_MS);
        k.write_results = Script::always(Err(Errno::ENOSPC));
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);

        k.mono_ms = WRITE_INTERVAL_MS;
        k.wall_ms = Some(A_PLAUSIBLE_MS + WRITE_INTERVAL_MS);
        clock.tick(&mut k, &mut j);
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Error && r.msg.contains("clock could not be written")),
            "{:?}",
            k.records
        );
        assert_eq!(clock.next_timeout_ms(k.mono_ms), Some(WRITE_INTERVAL_MS));

        // A hundred more wakeups within the hour attempt nothing.
        let before = k.attempted_writes;
        for step in 1..=100 {
            k.mono_ms = WRITE_INTERVAL_MS + step;
            clock.tick(&mut k, &mut j);
        }
        assert_eq!(k.attempted_writes, before);
    }

    /// Past the commit the value IS on the card, so a failed directory flush must
    /// not be reported as though nothing happened — and the floor advances,
    /// because it did.
    #[test]
    fn a_directory_flush_that_fails_says_the_clock_is_stored_anyway() {
        let mut k = a_pi_with_a_stored_clock(A_PLAUSIBLE_MS);
        k.sync_dir_results = Script::always(Err(Errno::EIO));
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);

        k.mono_ms = WRITE_INTERVAL_MS;
        k.wall_ms = Some(A_PLAUSIBLE_MS + WRITE_INTERVAL_MS);
        clock.tick(&mut k, &mut j);
        assert!(said(&k, "could not be flushed to the card"), "{:?}", k.records);
        assert_eq!(
            k.contents(STORED_PATH),
            Some(format!(
                "{} unsynchronised\n",
                A_PLAUSIBLE_MS + WRITE_INTERVAL_MS
            ))
        );
    }

    /// The crash-safe sequence really is `rivet-store`'s, reached through this
    /// crate's port: write a temp, flush it, rename it over the target, flush the
    /// directory. This is what stops a power cut leaving a half-written clock.
    #[test]
    fn the_write_goes_through_the_stores_crash_safe_sequence() {
        let mut k = a_pi_with_a_stored_clock(A_PLAUSIBLE_MS);
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);
        k.mono_ms = WRITE_INTERVAL_MS;
        k.wall_ms = Some(A_PLAUSIBLE_MS + WRITE_INTERVAL_MS);
        clock.tick(&mut k, &mut j);

        assert_eq!(
            k.writes.iter().map(|w| w.path.as_str()).collect::<Vec<_>>(),
            vec!["/state/clock.new"],
            "the target was written directly rather than through a temp"
        );
        assert_eq!(k.syncs, vec!["/state/clock.new".to_string()]);
        assert_eq!(
            k.renames,
            vec![("/state/clock.new".to_string(), "/state/clock".to_string())]
        );
        assert_eq!(k.dir_syncs, vec!["/state".to_string()]);
    }

    /// **Nothing may ever write `synchronised`.** Nothing in Rivet can
    /// synchronise a clock, and a value laundered through many boots of
    /// `max(stored, now)` must not become indistinguishable from one a time
    /// server supplied.
    #[test]
    fn what_this_machine_writes_never_claims_to_have_been_synchronised() {
        let mut k = a_pi_with_a_stored_clock(A_PLAUSIBLE_MS);
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);
        k.mono_ms = WRITE_INTERVAL_MS;
        k.wall_ms = Some(A_PLAUSIBLE_MS + WRITE_INTERVAL_MS);
        clock.tick(&mut k, &mut j);

        for write in &k.writes {
            let text = String::from_utf8_lossy(&write.contents);
            assert!(text.contains("unsynchronised"), "{text}");
        }
        // ...even when the card said the previous boot WAS synchronised, which is
        // a state no Rivet writer can produce today and a reader must accept.
        let mut k = FakeKernel::new();
        k.wall_ms = Some(WHAT_A_COLD_PI_BELIEVES_MS);
        k.files_mut().insert(
            STORED_PATH.to_string(),
            Script::always(Ok(format!("{A_PLAUSIBLE_MS} synchronised\n"))),
        );
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);
        assert_eq!(k.wall_sets, vec![A_PLAUSIBLE_MS], "a synchronised stamp did not restore");
        k.mono_ms = WRITE_INTERVAL_MS;
        k.wall_ms = Some(A_PLAUSIBLE_MS + WRITE_INTERVAL_MS);
        clock.tick(&mut k, &mut j);
        for write in &k.writes {
            let text = String::from_utf8_lossy(&write.contents);
            assert!(
                !text.contains(" synchronised"),
                "a restored value was re-persisted as authoritative: {text}"
            );
        }
    }

    /// **The mechanism, end to end, across a power cycle.** Two supervisor runs
    /// over one card: the first records an hour, the power goes out, and the
    /// second comes up at the epoch and believes that hour instead.
    #[test]
    fn a_power_cycle_restores_the_hour_the_previous_boot_recorded() {
        // Boot one: nothing stored, then an hour of uptime.
        let mut k = FakeKernel::new();
        k.wall_ms = Some(A_PLAUSIBLE_MS);
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);
        clock.tick(&mut k, &mut j);
        k.mono_ms = WRITE_INTERVAL_MS;
        k.wall_ms = Some(A_PLAUSIBLE_MS + WRITE_INTERVAL_MS);
        clock.tick(&mut k, &mut j);
        let Some(on_the_card) = k.contents(STORED_PATH) else {
            unreachable!("boot one stored nothing")
        };

        // The power goes out. Boot two comes up at three seconds past the epoch,
        // which is what a Raspberry Pi 4 actually did.
        let mut k = FakeKernel::new();
        k.wall_ms = Some(WHAT_A_COLD_PI_BELIEVES_MS);
        k.files_mut()
            .insert(STORED_PATH.to_string(), Script::always(Ok(on_the_card)));
        let mut j = journal();
        Clock::new().restore(&mut k, &mut j);
        assert_eq!(k.wall_ms, Some(A_PLAUSIBLE_MS + WRITE_INTERVAL_MS));
    }

    // -- the write on clean shutdown (`ADR-021` §3.3) ------------------------
    //
    // ~~`nothing_writes_the_clock_on_shutdown_because_the_record_does_not_say_
    // who_should`~~ stood here until 2026-08-23 and asserted the absence out
    // loud, with a note saying it was the test to delete when the question was
    // ruled. It was ruled — `ADR-021` §3.3 — and these say the true thing
    // instead. It was hard to settle partly because there was **no clean
    // shutdown to hang it on**: nothing could ask PID 1 to stop a board until
    // `ADR-017` §3.1.10 gave it a third signal.

    /// **The half-hour that used to be lost.** A machine shut down thirty
    /// minutes into an hour writes what it knows, instead of the next boot
    /// restoring to the top of the hour.
    #[test]
    fn a_clean_shutdown_writes_the_running_clock_rather_than_losing_the_part_hour() {
        let mut k = a_pi_with_a_stored_clock(A_PLAUSIBLE_MS);
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);
        assert!(k.writes.is_empty(), "an ordinary Pi boot writes nothing yet");

        // Half an hour in, a SIGTERM arrives.
        k.mono_ms = WRITE_INTERVAL_MS / 2;
        k.wall_ms = Some(A_PLAUSIBLE_MS + WRITE_INTERVAL_MS / 2);
        clock.persist_on_shutdown(&mut k, &mut j);

        assert_eq!(
            k.contents(STORED_PATH),
            Some(format!(
                "{} unsynchronised\n",
                A_PLAUSIBLE_MS + WRITE_INTERVAL_MS / 2
            )),
            "the half hour before the shutdown was lost"
        );
        assert!(said(&k, "shutdown requested; writing the running clock"), "{:?}", k.records);
    }

    /// It goes through the same crash-safe sequence as the hourly write, because
    /// a power cut during a shutdown is the *likeliest* moment for one.
    #[test]
    fn the_shutdown_write_uses_the_same_crash_safe_sequence_as_the_hourly_one() {
        let mut k = a_pi_with_a_stored_clock(A_PLAUSIBLE_MS);
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);
        k.mono_ms = 60_000;
        k.wall_ms = Some(A_PLAUSIBLE_MS + 60_000);
        clock.persist_on_shutdown(&mut k, &mut j);

        assert_eq!(
            k.writes.iter().map(|w| w.path.as_str()).collect::<Vec<_>>(),
            vec!["/state/clock.new"]
        );
        assert_eq!(k.syncs, vec!["/state/clock.new".to_string()]);
        assert_eq!(
            k.renames,
            vec![("/state/clock.new".to_string(), "/state/clock".to_string())]
        );
        assert_eq!(k.dir_syncs, vec!["/state".to_string()]);
    }

    /// A second `SIGTERM` is ordinary — an impatient operator, or PID 1
    /// escalating — and must not be a second write to a card that is going away.
    #[test]
    fn a_second_shutdown_request_does_not_write_twice() {
        let mut k = a_pi_with_a_stored_clock(A_PLAUSIBLE_MS);
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);
        k.mono_ms = 60_000;
        k.wall_ms = Some(A_PLAUSIBLE_MS + 60_000);

        clock.persist_on_shutdown(&mut k, &mut j);
        let after_one = k.attempted_writes;
        clock.persist_on_shutdown(&mut k, &mut j);
        clock.persist_on_shutdown(&mut k, &mut j);
        assert_eq!(k.attempted_writes, after_one, "the card was written twice");
    }

    /// **The guard a shutdown must not relax.** A machine whose `clock_settime`
    /// was refused is sitting at 1970; persisting that on the way out would
    /// throw away every hour the card had recorded — which is worse than losing
    /// the part-hour this feature exists to save.
    #[test]
    fn a_boot_that_never_established_a_floor_writes_nothing_on_shutdown_and_says_why() {
        let mut k = a_pi_with_a_stored_clock(A_PLAUSIBLE_MS);
        k.set_wall_results = Script::always(Err(Errno::EPERM));
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);

        k.mono_ms = 60_000;
        clock.persist_on_shutdown(&mut k, &mut j);
        assert!(k.writes.is_empty(), "1970 was persisted over the stored hour");
        assert!(said(&k, "never established a floor"), "{:?}", k.records);
    }

    /// ...and the same for a card that could not be read at all, which is the
    /// other way `restore` refuses to schedule.
    #[test]
    fn an_unreadable_stored_clock_is_not_overwritten_by_a_shutdown_either() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(WHAT_A_COLD_PI_BELIEVES_MS);
        k.files_mut()
            .insert(STORED_PATH.to_string(), Script::always(Err(Errno::EIO)));
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);
        clock.persist_on_shutdown(&mut k, &mut j);
        assert!(k.writes.is_empty(), "a clock nothing could read was overwritten");
    }

    /// A clock that went backwards while running is still refused on the way
    /// out: `tick`'s floor guard is reached, not bypassed.
    #[test]
    fn a_shutdown_does_not_lower_the_stored_clock() {
        let mut k = a_pi_with_a_stored_clock(A_PLAUSIBLE_MS);
        let mut j = journal();
        let mut clock = Clock::new();
        clock.restore(&mut k, &mut j);

        k.mono_ms = 60_000;
        k.wall_ms = Some(A_PLAUSIBLE_MS - 1);
        clock.persist_on_shutdown(&mut k, &mut j);
        assert!(k.writes.is_empty(), "{:?}", k.writes);
        assert!(said(&k, "BEHIND the floor"), "{:?}", k.records);
    }

    /// The interval, and the arithmetic behind it, pinned so that changing it has
    /// to change a test that states the cost.
    #[test]
    fn the_write_interval_is_hourly_which_is_eight_thousand_writes_a_year() {
        assert_eq!(WRITE_INTERVAL_MS, 3_600_000);
        let a_year = 365 * 24 * 60 * 60 * 1000_u64;
        assert_eq!(a_year / WRITE_INTERVAL_MS, 8_760);
        // ADR-021 §3.1's own comparison: "every minute is over half a million
        // small writes a year".
        assert_eq!(a_year / 60_000, 525_600);
    }
}
