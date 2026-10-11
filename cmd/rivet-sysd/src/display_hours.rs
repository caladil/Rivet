//! **Acting on the display window** — `ADR-039` §3.1 and §8.
//!
//! `crate::runtime` publishes the window for whoever holds the screen to read.
//! This is the other half: **the machine doing what it can itself**, which is
//! telling the television.
//!
//! # Why the machine can do this and cannot blank the panel
//!
//! A kiosk application is leased the CRTC and its connector, and a lease is
//! exclusive — so while an application is drawing, the supervisor cannot touch
//! the panel (`ADR-039` §8.1). **CEC is a different device.** `/dev/cecN` is
//! not in the lease and is not affected by who holds the display, which is
//! exactly why §8.2 makes it the machine's transport.
//!
//! So the division is:
//!
//! | | who does it |
//! | :-- | :-- |
//! | decide the hours | the document, `[display]` |
//! | stop drawing | the application, reading `crate::runtime`'s file |
//! | **switch the television off** | **here** |
//!
//! # It acts on CHANGES, not on state
//!
//! `Standby` every convergence pass would be a television that cannot be turned
//! on by hand for the whole night, and a bus carrying a message every second
//! for no reason. This sends only when the answer changes — and on the first
//! pass, because a machine that boots inside the dark window has a television
//! somebody left on.
//!
//! # Every failure here is survivable, and none of them stops a convergence
//!
//! A missing adapter, no cable, a television with CEC switched off in a menu:
//! all ordinary, all reported once, none fatal. `ADR-039` §8.2's point is that
//! CEC saves the power; the *picture* is still handled by the application, so a
//! machine with no CEC at all still goes dark on schedule — it just keeps its
//! backlight lit.

use rivet_core::display::Hours;
use rivet_core::journal::{Severity, Value};

use crate::journal::Journal;
use crate::kernel::Kernel;

const SELF_NAME: &str = "rivet-sysd";

fn note(k: &mut dyn Kernel, journal: &mut Journal, sev: Severity, msg: &str, fields: Vec<(String, Value)>) {
    let corr = journal.next_correlation();
    journal.emit(k, sev, SELF_NAME, &corr, msg, fields);
}

/// **The clock face out of `Kernel::local_time`'s reading.**
///
/// `LocalReading::local` is `YYYY-MM-DD HH:MM:SS`, composed by
/// `crate::kernel` from `struct tm`'s own fields — so the `HH:MM` at offset 11
/// is exactly the spelling `rivet_core::display::TimeOfDay::parse` takes, and
/// this is a slice rather than a second parser.
///
/// **The same call, and therefore the same answer, as the clock on the
/// screen.** `Kernel::local_time` is `localtime_r(3)` against the tzfile
/// `crate::etc` projects; `aura-renderer` calls the same function through
/// `rivet_sdk::schedule::local_now`. A television switching off a minute before
/// the picture did would be two clocks, and `ADR-011` is why there is one.
///
/// `None` when the string is not that shape, which is a defect in
/// `crate::kernel` rather than a state — and returning `None` makes it a
/// television that is not told, not a television told the wrong thing.
#[must_use]
pub fn clock_face(reading: &crate::kernel::LocalReading) -> Option<rivet_core::display::TimeOfDay> {
    reading
        .local
        .get(11..16)
        .and_then(rivet_core::display::TimeOfDay::parse)
}

/// The last thing this machine told the television, so it is told only on a
/// change.
#[derive(Debug, Default)]
pub struct Television {
    /// `None` until the first evaluation — which is why a machine that boots
    /// inside the dark window still sends `Standby`.
    lit: Option<bool>,
    /// Set once a failure has been reported, so a board with no HDMI cable does
    /// not fill its journal at one line per pass.
    complained: bool,
    /// A send that is on the wire, waiting to be reported.
    ///
    /// **See [`Television::converge`] for why this exists.** In one sentence:
    /// claiming a CEC logical address is the kernel talking to a television
    /// over a wire, it takes as long as it takes, and it was happening on the
    /// supervisor's convergence pass.
    in_flight: Option<InFlight>,
}

/// One `send` running in its own process, and what it will need to be
/// journalled when it finishes.
///
/// **The window is captured here rather than read at collection time.** The
/// record says which window this send was for, and by the time it lands the
/// document may name a different one — reading it then would attach the
/// outcome to a window that had nothing to do with it.
#[derive(Debug)]
struct InFlight {
    lit: bool,
    window: String,
    worker: crate::kernel::CecWorker,
}

impl Television {
    /// Evaluate the window and tell the television if the answer changed.
    ///
    /// `hours` is the document's; `local_now` is the machine's local time in
    /// minutes since midnight, or `None` when the clock cannot be read — in
    /// which case **nothing is sent**, because a schedule evaluated against a
    /// clock that is wrong is worse than no schedule at all. `ADR-021` §4.2
    /// steps the wall clock at boot, so this is a real state and not a
    /// theoretical one.
    pub fn converge(
        &mut self,
        k: &mut dyn Kernel,
        journal: &mut Journal,
        hours: Option<&Hours>,
        local_now: Option<rivet_core::display::TimeOfDay>,
    ) {
        // **Whatever the last pass started, reported now if it has finished.**
        // Non-blocking; a send still on the wire simply is not here yet.
        self.collect(k, journal);

        let Some(hours) = hours else {
            // No schedule. Nothing is sent and nothing is remembered: a
            // document that gains a window later must be acted on from a clean
            // slate rather than compared against a state from before it existed.
            self.lit = None;
            self.complained = false;
            return;
        };
        let Some(now) = local_now else {
            return;
        };

        let lit = hours.is_on(now);
        if self.lit == Some(lit) {
            return;
        }
        // **One send on the bus at a time.** The answer changes twice a day, so
        // reaching here with one still outstanding means the previous send is
        // taking a very long time — which is exactly the case this whole change
        // is about. Two threads driving one CEC adapter would be a second
        // logical-address claim against the first, so the decision waits for
        // the next pass and `self.lit` is deliberately NOT moved: leaving it
        // means the next pass sees the same difference and tries again.
        if self.in_flight.is_some() {
            return;
        }
        // Recorded BEFORE the attempt, so a television that cannot be reached
        // is not retried every pass for the rest of the window.
        self.lit = Some(lit);
        self.complained = false;

        // **Off the convergence pass, in a forked child rather than a spawned
        // one** — `ADR-039` §8.
        //
        // `send` opens the adapter and calls `configure`, which asks the
        // kernel to claim a CEC logical address: a negotiation over a
        // physical wire with a television that may be off, unplugged or
        // slow, taking as long as it takes. Running it inside `converge`
        // would hold the supervisor's own pass for that whole duration — no
        // service started, no stop noticed, no socket accepted — and the
        // failure is unbounded even though the trigger (an HDMI cable) is
        // ordinary.
        //
        // A fork rather than a poll registration, because `rivet_adapter_cec`
        // is a blocking ioctl API and wrapping it in a state machine would
        // mean owning the kernel's own retry loop. And a fork rather than a
        // thread: this process forks elsewhere too, to start a service, and a
        // second thread sharing its allocator would make that unsafe at any
        // moment this one happened to be allocating inside `send`.
        // `Kernel::cec_send`'s doc has the full argument.
        //
        // The child's result is picked up by `collect` on a later pass,
        // non-blocking either way; if the negotiation never finishes this
        // machine leaks one process blocked in an ioctl rather than one
        // thread — no worse than before, and nothing else here ever shares
        // memory with it again.
        let window = hours.to_string();
        match k.cec_send(lit, send) {
            Ok(worker) => self.in_flight = Some(InFlight { lit, window, worker }),
            // The transaction never ran at all — `pipe(2)` or `fork(2)`
            // refused. Reported exactly as a negotiation that ran and failed
            // would be: either way the television was not told.
            Err(e) => self.report_unreachable(k, journal, e.to_string(), window),
        }
    }

    /// Report a finished send, if one has finished. Never blocks.
    fn collect(&mut self, k: &mut dyn Kernel, journal: &mut Journal) {
        self.drain(k, journal);
    }

    /// The single non-blocking check for an outstanding send, shared by
    /// [`Television::collect`] and, in a bounded retry loop, by the test-only
    /// `Television::settle`.
    ///
    /// **One reporting path, taken by both callers.** There is exactly one
    /// place that decides a send has finished and exactly one place that
    /// clears [`Television::in_flight`] — a second copy of either question is
    /// how a test-only "wait, then look" caller and the ordinary "look once
    /// and take whatever is there" caller end up disagreeing about which one
    /// saw the real outcome, which is the same species of mistake as two
    /// readers of one fact (`ADR-011`).
    fn drain(&mut self, k: &mut dyn Kernel, journal: &mut Journal) {
        let Some(flight) = self.in_flight.as_mut() else {
            return;
        };
        let Some(outcome) = k.cec_poll(&mut flight.worker) else {
            // Still on the wire. The ordinary case for a pass or two.
            return;
        };
        let (lit, window) = (flight.lit, flight.window.clone());
        self.in_flight = None;

        match outcome {
            Ok(path) => note(
                k,
                journal,
                Severity::Info,
                if lit {
                    "the display window opened: the television was told to wake"
                } else {
                    "the display window closed: the television was told to sleep"
                },
                vec![
                    ("adapter".to_string(), Value::Str(path)),
                    ("window".to_string(), Value::Str(window)),
                ],
            ),
            Err(why) => self.report_unreachable(k, journal, why, window),
        }
    }

    /// Report that the television could not be reached — a transaction that
    /// never started, or one that ran and failed — at most once per change.
    /// See [`Television::complained`].
    fn report_unreachable(
        &mut self,
        k: &mut dyn Kernel,
        journal: &mut Journal,
        why: String,
        window: String,
    ) {
        if self.complained {
            return;
        }
        self.complained = true;
        note(
            k,
            journal,
            Severity::Warn,
            "the television could not be told, so it keeps its own power state. \
             The PICTURE still follows the window - whatever holds the screen reads \
             it from /run/rivet/display-hours - so this costs the backlight and not \
             the schedule (ADR-039 §8.2)",
            vec![
                ("reason".to_string(), Value::Str(why)),
                ("window".to_string(), Value::Str(window)),
            ],
        );
    }

    /// **Wait for an outstanding send and report it.** Tests only.
    ///
    /// `converge` is deliberately non-blocking and so is [`Kernel::cec_poll`],
    /// so a test that called `converge` once and looked at the journal would
    /// be reading it before the answer arrived — and would pass or fail on
    /// the scheduler. This polls in a bounded loop, so the assertions below
    /// are about the code rather than about timing.
    ///
    /// It is `#[cfg(test)]` because a supervisor that waited here would be the
    /// defect this change removed.
    #[cfg(test)]
    fn settle(&mut self, k: &mut dyn Kernel, journal: &mut Journal) {
        // Long enough that a failing `open_first_usable` on a machine with no
        // `/dev/cec0` or `/dev/cec1` is never mistaken for a hang; short
        // enough that a genuine hang fails the test rather than the suite.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            self.drain(k, journal);
            if self.in_flight.is_none() {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the television was never told: settle() polled for 10s with a send still \
                 outstanding"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

/// Open, configure and send — the whole transaction, every time.
///
/// **Not held open between calls, deliberately.** A held descriptor claims a
/// logical address on the bus for the life of the process, which is a thing
/// this machine would be doing all day to send two messages. Twice a day is
/// cheap; a permanent claim on somebody's living-room bus is rude.
///
/// `open_first_usable` opens and configures each of `rivet_adapter_cec`'s
/// candidate ports in turn and hands back the first that is actually usable
/// — a television on HDMI1 is reached even when HDMI0 has no cable — so
/// exactly one adapter comes back and exactly one command is sent on it.
fn send(lit: bool) -> Result<String, String> {
    let adapter =
        rivet_adapter_cec::Adapter::open_first_usable().map_err(|e| e.to_string())?;
    if lit {
        adapter.image_view_on().map_err(|e| e.to_string())?;
    } else {
        adapter.standby().map_err(|e| e.to_string())?;
    }
    Ok(adapter.path().to_string())
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;
    use crate::testkernel::FakeKernel;
    use rivet_core::display::TimeOfDay;

    fn journal() -> Journal {
        Journal::new("boot".to_string(), 0, SELF_NAME.to_string())
    }

    fn window() -> Hours {
        Hours::new(
            TimeOfDay::new(19, 0).expect("19:00"),
            TimeOfDay::new(7, 0).expect("07:00"),
        )
        .expect("a valid window")
    }

    fn at(h: u8, m: u8) -> Option<TimeOfDay> {
        TimeOfDay::new(h, m)
    }

    /// The clock face is sliced out of the supervisor's own reading, so there
    /// is one answer to what time it is locally.
    #[test]
    fn the_clock_face_is_read_out_of_the_supervisors_own_reading() {
        let reading = |local: &str| crate::kernel::LocalReading {
            offset_s: -18000,
            abbreviation: "CDT".to_string(),
            local: local.to_string(),
        };
        assert_eq!(
            clock_face(&reading("2026-09-02 19:00:00")),
            TimeOfDay::new(19, 0)
        );
        assert_eq!(
            clock_face(&reading("2026-01-01 00:00:00")),
            TimeOfDay::new(0, 0)
        );
        assert_eq!(
            clock_face(&reading("2026-12-31 23:59:59")),
            TimeOfDay::new(23, 59)
        );
        // A string that is not that shape is a defect in `crate::kernel`, and
        // it must produce a television that is NOT TOLD rather than one told
        // the wrong thing.
        for broken in ["", "2026-09-02", "not a time at all", "2026-09-02 7:0:0"] {
            assert_eq!(clock_face(&reading(broken)), None, "{broken:?}");
        }
    }

    /// **Nothing is sent while the answer is unchanged.** A `Standby` every
    /// pass is a television nobody can switch on by hand until morning.
    #[test]
    fn the_television_is_told_only_when_the_answer_changes() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        let mut tv = Television::default();

        // **`settle` after every pass that decides something**, because the
        // send runs in its own process now and `converge` returns before it
        // has an answer. A test that read the journal here without settling
        // would be asserting against the scheduler.
        tv.converge(&mut k, &mut j, Some(&window()), at(20, 0));
        tv.settle(&mut k, &mut j);
        assert_eq!(tv.lit, Some(false), "20:00 is inside the dark window");
        let after_first = j.pending().len();

        // Three more passes inside the same window say nothing further.
        for minute in [1, 2, 3] {
            tv.converge(&mut k, &mut j, Some(&window()), at(20, minute));
            tv.settle(&mut k, &mut j);
        }
        assert_eq!(
            j.pending().len(),
            after_first,
            "the television was told again while nothing had changed"
        );

        // And the window opening is a change.
        tv.converge(&mut k, &mut j, Some(&window()), at(7, 0));
        tv.settle(&mut k, &mut j);
        assert_eq!(tv.lit, Some(true));
        assert!(j.pending().len() > after_first, "the window opening said nothing");
    }

    /// **The send does not happen on the convergence pass.**
    ///
    /// `send` opens the CEC adapter and asks the kernel to claim a logical
    /// address — a negotiation with a television that may be off or
    /// unplugged, taking as long as it takes — and running it inside
    /// `converge` would hold the supervisor's own pass for that whole
    /// duration: no service started, no exit noticed.
    ///
    /// Asserted as *the journal is still empty when `converge` returns*, which
    /// is the observable form of "it did not wait". `settle` then shows the
    /// answer does arrive, so this cannot pass by the send having been dropped.
    #[test]
    fn converge_does_not_wait_for_the_television() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        let mut tv = Television::default();

        tv.converge(&mut k, &mut j, Some(&window()), at(20, 0));
        assert!(
            tv.in_flight.is_some(),
            "the send was not dispatched at all, so this test proves nothing about waiting"
        );
        assert!(
            j.pending().is_empty(),
            "converge journalled an outcome, which means it waited for one"
        );

        tv.settle(&mut k, &mut j);
        assert!(tv.in_flight.is_none(), "the outcome was never collected");
        assert!(
            !j.pending().is_empty(),
            "the send's outcome never reached the journal, so it was lost rather than deferred"
        );
    }

    /// **A machine that boots inside the dark window still sends.** `lit`
    /// starts as `None`, not as `true`, so the first evaluation is a change —
    /// which is what catches a television somebody left on.
    #[test]
    fn a_machine_that_boots_in_the_dark_window_tells_the_television() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        let mut tv = Television::default();
        assert_eq!(tv.lit, None, "nothing is assumed about a television at boot");
        tv.converge(&mut k, &mut j, Some(&window()), at(23, 30));
        tv.settle(&mut k, &mut j);
        assert_eq!(tv.lit, Some(false));
        assert!(!j.pending().is_empty(), "the first pass said nothing");
    }

    /// **A clock that cannot be read sends nothing.** `ADR-021` §4.2 steps the
    /// wall clock at boot, so there is a real window in which the time is
    /// years wrong — and a schedule evaluated then would switch a television
    /// off at what it thinks is midnight in 1970.
    #[test]
    fn an_unreadable_clock_sends_nothing_rather_than_guessing() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        let mut tv = Television::default();
        tv.converge(&mut k, &mut j, Some(&window()), None);
        tv.settle(&mut k, &mut j);
        assert_eq!(tv.lit, None, "nothing was decided");
        assert!(j.pending().is_empty(), "a machine with no clock said something");
    }

    /// A document with no `[display]` table sends nothing and **forgets**, so
    /// that adding a window later is acted on from a clean slate rather than
    /// compared against a state from before it existed.
    #[test]
    fn removing_the_window_forgets_what_the_television_was_told() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        let mut tv = Television::default();
        tv.converge(&mut k, &mut j, Some(&window()), at(20, 0));
        tv.settle(&mut k, &mut j);
        assert_eq!(tv.lit, Some(false));

        tv.converge(&mut k, &mut j, None, at(20, 1));
        assert_eq!(tv.lit, None, "the state must not survive the window's removal");

        // And the window coming back is a change again, even to the same answer.
        let before = j.pending().len();
        tv.converge(&mut k, &mut j, Some(&window()), at(20, 2));
        tv.settle(&mut k, &mut j);
        assert_eq!(tv.lit, Some(false));
        assert!(j.pending().len() > before, "the returning window said nothing");
    }

    /// **A board with no CEC complains once, not once per pass.** The failure
    /// path is the ordinary one on any machine with no HDMI cable, and a
    /// journal line per second would bury everything else on it.
    #[test]
    fn an_unreachable_television_is_reported_once_per_change() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        let mut tv = Television::default();

        // This host has no /dev/cec0, so `send` fails - which is the case
        // under test rather than a limitation of it.
        tv.converge(&mut k, &mut j, Some(&window()), at(20, 0));
        tv.settle(&mut k, &mut j);
        let after_first = j.pending().len();
        assert!(after_first > 0, "a failure must be reported at least once");

        for minute in [1, 2, 3] {
            tv.converge(&mut k, &mut j, Some(&window()), at(20, minute));
            tv.settle(&mut k, &mut j);
        }
        assert_eq!(
            j.pending().len(),
            after_first,
            "an unreachable television filled the journal"
        );
    }
}
