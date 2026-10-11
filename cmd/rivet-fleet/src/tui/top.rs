//! **The monitor** — a per-device `top`, and the pane the maintainer asked for
//! on 2026-08-29: a `top`-like screen showing the current status of the
//! machine — cpu, memory, disk, thermals and running processes, with the
//! kernel noise filtered out by default (.rulings/ R-328).
//!
//! `crate::telemetry` fetches; this renders. The split is not tidiness: the
//! fetching has a real constraint in it — `ADR-022` §8's 8 MiB journal against a
//! poll that costs a record — and the rendering has a different one, which is
//! this file's subject.
//!
//! # The failure this pane is designed against
//!
//! **A stale number rendered as a live one.** Every other defect in a monitoring
//! screen announces itself; this one looks exactly like the screen working. A
//! person reading 612 MiB available off a board that stopped answering four
//! minutes ago will act on it.
//!
//! So the rule here is absolute: **no figure is drawn without its age beside
//! it.** Each section's heading carries how old its reading is, and when the
//! last attempt failed the heading says so *in the error style* while the
//! numbers below it stay visibly attributed to a moment in the past. `AGENTS.md`
//! §5 — never false by omission — and the omission a monitor makes is the
//! timestamp.
//!
//! # ~~What this build cannot show~~ — what a percentage costs, since 2026-08-29
//!
//! The section this header used to hold is **struck rather than deleted**
//! (`AGENTS.md` §2, §7), because it was right and because what replaced it is
//! the same argument one step further on. It read:
//!
//! > **CPU.** The maintainer asked for it and there is nothing to read: there is
//! > no `get cpu` and no load average in the catalogue, and **`get process`
//! > carries no CPU time at all** — so even a per-process percentage cannot be
//! > computed from two readings. A screen that filled that space with something
//! > plausible would be `AGENTS.md` §5's guess that *succeeds*.
//!
//! `get cpu` exists now and `get process` carries `user_millis` and
//! `system_millis`. **What has not changed is that a percentage needs two
//! readings**, and the subject refuses to invent the second one — so this pane
//! is where the second reading lives, and the honesty moved rather than went
//! away:
//!
//! * **the first reading has no percentage to give**, and [`cpu`] says so in a
//!   sentence instead of drawing `0%`. Zero percent is a measurement of an idle
//!   machine and it is the one number a monitor must never guess;
//! * **the interval is on screen beside the figure**, because a rate over an
//!   unstated interval is not a measurement;
//! * **per-CPU bars, not only an aggregate.** One core pinned while three idle
//!   is the failure worth finding on a small board and an average divides it by
//!   four until it stops looking like anything;
//! * **`iowait` is drawn apart from busy.** A board at 90% iowait is not a busy
//!   board, it is a board waiting on a card that is failing — `ADR-018` §7 — and
//!   the `D`-state rows in the table below are the same fact one process at a
//!   time;
//! * **the load average is labelled as not a utilisation.** On Linux it counts
//!   uninterruptible sleep, which every other Unix does not, and a reader who
//!   learned the number elsewhere will read a stuck machine as a busy one.
//!
//! # Thermals: the throttle is what is read, and the temperature is the least of
//! it
//!
//! `docs/design-candidates/Admin-Surface-Survey.md` §7.1 settled the order
//! before this renderer existed:
//!
//! > a capped frequency **is** the throttle, observed rather than inferred, and
//! > under-voltage is the Pi's actual killer.
//!
//! So [`thermal`] leads with **current against maximum frequency** and with the
//! since-boot flags — the only fact of the four that cannot be recovered any
//! other way — and puts the temperature last. And the comparison it draws as a
//! *throttle* is `ceiling` against `maximum`, never `current` against `maximum`:
//! a governor scales an idle machine down, so a Pi at 600 MHz of 1500 is
//! behaving correctly, and a screen that called that a throttle would cry wolf
//! all day until nobody read it.
//!
//! **The degradation path is kept and is not a leftover.** `Admin-Surface-
//! Survey.md` §7.6 measured that the throttle flags need `/dev/vcio`, and a
//! board where that is absent or refuses answers with `throttle_unmeasured`
//! rather than with eight falses. That renders as **not measured** — never as
//! *no throttling*, which is the one claim this pane could make that would be
//! both wrong and reassuring.
//!
//! # The supervisor and the kernel are two different questions
//!
//! `get status` is what the supervisor **believes** it is running; `get process`
//! is what the kernel **says** is executing. `rivet_core::operation::process`
//! makes the point that both are correct and the interesting cases are where
//! they disagree, so [`agreement`] puts them side by side and names the three
//! disagreements that matter: a service running with no process behind it, a
//! userland process no service claims, and a zombie.
//!
//! **The two readings are not simultaneous** — `status` is on the medium tier
//! and `process` on the fast one — so the section says how old the supervisor's
//! half is. A cross-check between readings taken at different moments is worth
//! having and is not worth trusting silently.

use std::time::{Duration, Instant};

use ratatui::text::{Line, Span};

use crate::telemetry::{Liveness, Outcome, Poller, Reading, Slot, Subject};
use crate::tui::theme::{Glyphs, Palette};

/// How wide a bar is drawn. **Fixed rather than proportional to the pane**, so
/// two rows are comparable by eye at any terminal size.
const BAR: usize = 22;

/// The furthest the pane will scroll.
///
/// [`draw`] clamps to the real end, which it can do because it knows the height;
/// the key handler cannot, so it caps here instead. **A machine with more than
/// this many processes exists** — and the number is far past a Rivet's, whose
/// userland list is supposed to be short enough to account for by hand.
pub const SCROLL_LIMIT: usize = 4096;

/// How the process table is ordered.
///
/// ~~**Resident size first**, because it is the only quantity the table carries
/// that says *this one is using the machine* — there is no CPU time to sort
/// by.~~ **CPU first, since 2026-08-29**, and the sentence is struck rather than
/// edited because its premise is what changed: `get process` now carries
/// `user_millis` and `system_millis`, so there *is* a CPU figure to sort by, and
/// it is the one `top` opens on because it is the one that answers *what is
/// using this machine right now*.
///
/// Resident size stays second. It answers a different and slower question —
/// *what is using this machine's memory* — and on a board with 926 MiB that is
/// worth one keypress rather than none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    /// **Needs two readings to mean anything**, and before the second one lands
    /// every process sorts equal — which is why the header says so rather than
    /// presenting an arbitrary order as a ranking.
    Cpu,
    Resident,
    Pid,
    Name,
}

impl Order {
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Resident => "resident",
            Self::Pid => "pid",
            Self::Name => "name",
        }
    }

    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Cpu => Self::Resident,
            Self::Resident => Self::Pid,
            Self::Pid => Self::Name,
            Self::Name => Self::Cpu,
        }
    }
}

/// What the monitor pane is showing, as distinct from what it has fetched.
#[derive(Debug, Clone)]
pub struct Monitor {
    /// **Kernel threads are hidden by default**, which is what was asked for:
    /// *"default to filtering out the kernel noise"*. A Rivet's userland list is
    /// short and should be wholly accounted for by the document, and that is the
    /// cheapest anomaly check this surface offers — it only works if the kernel
    /// threads are out of the way.
    ///
    /// The filter is not a secret: the count of what is hidden is on screen and
    /// the key that brings them back is in the footer, because a hidden filter
    /// is how a question becomes unanswerable on the one machine where the
    /// filter is wrong (`rivet_core::operation::storage`'s own argument).
    pub kernel_threads: bool,
    pub order: Order,
    /// How far down the process table.
    pub scroll: usize,
}

impl Default for Monitor {
    fn default() -> Self {
        Self::new()
    }
}

impl Monitor {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            kernel_threads: false,
            order: Order::Cpu,
            scroll: 0,
        }
    }
}

/// The whole pane.
///
/// `poller` is `None` before the pane has ever been opened for this device —
/// which is a different state from *asked and got nothing*, and is said
/// differently.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn draw(
    view: &Monitor,
    poller: Option<&Poller>,
    // **Why there is no reader**, when there is none. `crate::operator`'s own
    // sentence — usually *no `--identity` was given*, which is a thing a person
    // must be told rather than left to infer from a blank pane. `None` means the
    // pane has simply not opened one yet.
    no_reader: Option<&str>,
    now: Instant,
    glyphs: Glyphs,
    palette: Palette,
    width: u16,
    height: u16,
) -> Vec<Line<'static>> {
    let Some(poller) = poller else {
        // **The fourth state, said in words and not animated** — `ADR-034`
        // §11.5.2. A clock face here would be claiming a poll is coming.
        let mut lines = vec![
            Line::from(""),
            Line::from(indicator(
                Liveness::Stopped(crate::telemetry::NO_TRANSPORT),
                glyphs,
                palette,
            )),
        ];
        match no_reader {
            Some(why) => {
                for text in wrap(why, width) {
                    lines.push(Line::from(Span::styled(text, palette.error())));
                }
            }
            None => lines.push(note("Opening a reader for this machine…", palette)),
        }
        return fit(lines, width);
    };

    // **One scroll for one pane, and the banner is pinned.**
    //
    // The first version gave the process table whatever height was left over
    // and let the sections above it keep their full size. Driven in a pty at
    // 44 rows, every process was off the bottom of the screen — the pane the
    // maintainer asked for is *the process list*, and it was the only thing not
    // on it. Windowing the table alone could not fix that, because the problem
    // was the sections above rather than the table.
    //
    // So the whole body scrolls and nothing is windowed twice. The banner stays
    // because it is the line that says whether any of this is live.
    let mut body = Vec::new();
    body.push(Line::from(""));
    body.extend(cpu(poller, now, palette, width));
    body.push(Line::from(""));
    body.extend(memory(poller, now, palette, width));
    body.push(Line::from(""));
    body.extend(thermal(poller, now, palette, width));
    body.push(Line::from(""));
    body.extend(storage(poller, now, palette, width));
    body.push(Line::from(""));
    body.extend(agreement(poller, now, glyphs, palette, width));
    body.push(Line::from(""));
    // **Last of the sections above the table**, because it is the one nobody
    // watches: which slot booted and what devices exist do not change while a
    // person looks, which is the whole reason they are on the `once` tier.
    body.extend(machine(poller, now, palette, width));
    body.push(Line::from(""));
    body.extend(processes(poller, view, now, glyphs, palette, width));

    let total = body.len();
    // One row for the banner, one for the indicator that says there is more.
    let room = usize::from(height.saturating_sub(2)).max(1);
    let at = view.scroll.min(total.saturating_sub(room));

    let mut lines = vec![banner(poller, view, now, glyphs, palette)];
    lines.extend(body.into_iter().skip(at).take(room));
    if total > room {
        lines.push(note(
            &format!(
                "lines {}–{} of {} — [↑↓] scrolls, [Home] returns to the top",
                at + 1,
                (at + room).min(total),
                total
            ),
            palette,
        ));
    }
    fit(lines, width)
}

/// Cut every line to the space the pane actually has.
///
/// **A last pass over everything, rather than a rule each renderer remembers.**
/// A table's columns are fixed-width and a narrow terminal is not a case any of
/// them think about; `ratatui` would clip at the frame anyway, but a pane whose
/// own output is longer than its area is a pane whose tests cannot assert
/// anything about what a person sees. `no_line_is_wider_than_the_pane` is that
/// assertion and this is what makes it true at 40 columns as well as at 200.
///
/// Two columns are taken off for the border the block draws — the same
/// arithmetic `crate::tui::device::wrapped` does for the prose it lays out.
fn fit(lines: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
    let room = usize::from(width.saturating_sub(2));
    lines
        .into_iter()
        .map(|line| {
            let mut left = room;
            let mut spans: Vec<Span<'static>> = Vec::with_capacity(line.spans.len());
            for span in line.spans {
                if left == 0 {
                    break;
                }
                let count = span.content.chars().count();
                if count <= left {
                    left -= count;
                    spans.push(span);
                } else {
                    let cut: String = span.content.chars().take(left).collect();
                    left = 0;
                    spans.push(Span::styled(cut, span.style));
                }
            }
            Line::from(spans)
        })
        .collect()
}

/// The line that says whether what is below it is live.
///
/// **The indicator is the first thing on it**, because it is the one claim the
/// rest of the pane depends on — `ADR-034` §11.5.2.
fn banner(
    poller: &Poller,
    view: &Monitor,
    now: Instant,
    glyphs: Glyphs,
    palette: Palette,
) -> Line<'static> {
    let mut spans = vec![Span::styled("  monitor  ", palette.faint())];
    spans.extend(indicator(poller.liveness(now), glyphs, palette));
    spans.push(Span::styled(
        format!("  fast every {}  ", seconds(poller.fast_interval())),
        palette.plain(),
    ));
    spans.push(Span::styled(
        format!("order by {}  ", view.order.word()),
        palette.faint(),
    ));
    // **Skipped ticks are on screen.** A monitor that is quietly falling behind
    // a slow link should say so rather than looking merely slow.
    if poller.skipped > 0 {
        spans.push(Span::styled(
            format!("{} refresh(es) skipped while busy", poller.skipped),
            palette.faint(),
        ));
    }
    Line::from(spans)
}

/// **The liveness indicator** — a glyph and the words beside it.
///
/// # This function draws a state; it never decides one
///
/// `ADR-034` §11.5.2's ruling is that the indicator is derived from the
/// poller's state and never from a wall clock, and the shape of this function is
/// how that is enforced rather than remembered: it takes a
/// [`crate::telemetry::Liveness`] and **has no access to a clock at all**. Every
/// duration it renders arrived inside that value, measured from something the
/// poller did.
///
/// A renderer that read `Instant::now()` here could animate a spinner over a
/// transport that had been dead for an hour, which is the defect the ruling
/// names: *an animation that says alive about something that is not*, worse than
/// no indicator.
///
/// # Every glyph has its word beside it
///
/// `crate::tui::theme`'s standing rule — no glyph carries meaning on its own —
/// and it matters more here than anywhere else in the interface, because this is
/// the state a person checks before believing a number.
fn indicator(state: Liveness, glyphs: Glyphs, palette: Palette) -> Vec<Span<'static>> {
    match state {
        Liveness::Stopped(why) => vec![
            Span::styled(format!("{} ", glyphs.stopped), palette.warn()),
            Span::styled(why.to_owned(), palette.warn()),
        ],
        Liveness::Waiting { elapsed, interval } => {
            let face = clock_face(elapsed, interval, glyphs);
            let left = interval.saturating_sub(elapsed);
            vec![
                Span::styled(format!("{face} "), palette.up()),
                Span::styled(format!("next read in {}", seconds(left)), palette.faint()),
            ]
        }
        Liveness::InFlight { since } => {
            // **The frame turns with the request, not with the clock.** `since`
            // is measured from the moment the cycle was sent, so a stalled
            // request keeps spinning — which is true, it is still out — and
            // nothing spins when nothing is out.
            let frames = glyphs.spinning;
            let at = (since.as_millis() / SPIN_EVERY_MS) as usize % frames.len();
            let frame = frames.get(at).copied().unwrap_or(glyphs.stopped);
            vec![
                Span::styled(format!("{frame} "), palette.up()),
                // **Never a countdown.** The remaining time is unknown and the
                // honest statement is that this end is waiting on the board.
                Span::styled("waiting on the board".to_owned(), palette.faint()),
            ]
        }
        Liveness::BackingOff { at } => vec![
            Span::styled(format!("{} ", glyphs.warn), palette.error()),
            Span::styled(
                format!("NOT ANSWERING — retrying every {}", seconds(at)),
                palette.error(),
            ),
        ],
    }
}

/// How often the in-flight spinner advances a frame, in milliseconds.
///
/// **120 ms**: fast enough to read as motion, slow enough that a 250 ms draw
/// loop (`crate::tui`'s `TICK`) does not skip frames unevenly enough to look
/// broken.
const SPIN_EVERY_MS: u128 = 120;

/// Which clock face a fraction of an interval lands on.
///
/// **Rounded down and clamped**, so the last face appears only when the interval
/// is actually up: a countdown that showed *full* early would be announcing a
/// poll that has not happened.
fn clock_face(elapsed: Duration, interval: Duration, glyphs: Glyphs) -> char {
    let faces = glyphs.waiting;
    let last = faces.len() - 1;
    let at = if interval.is_zero() {
        last
    } else {
        let millis = interval.as_millis().max(1);
        let through = elapsed.as_millis().min(millis);
        // `usize` cannot overflow here: the product is at most `last * millis`
        // and the division brings it back under `last`.
        ((through * last as u128) / millis) as usize
    };
    faces.get(at.min(last)).copied().unwrap_or(glyphs.stopped)
}

/// A section heading that always says how old what follows is.
///
/// **This is the whole honesty mechanism of the pane**, so it is one function
/// and every section goes through it.
fn heading(
    name: &str,
    poller: &Poller,
    subject: Subject,
    now: Instant,
    palette: Palette,
    width: u16,
) -> Vec<Line<'static>> {
    let Some(slot) = poller.slot(subject) else {
        return vec![Line::from(Span::styled(
            format!("  {name}"),
            palette.plain(),
        ))];
    };
    let interval = poller.interval(subject);
    let (state, detail, style) = freshness(slot, interval, now, palette);
    let mut lines = vec![Line::from(vec![
        Span::styled(format!("  {name:<11}"), palette.plain()),
        Span::styled(state, style),
    ])];
    // **The reason goes on its own lines, wrapped.** It was appended to the
    // heading until a test cut it at *"failed on"* — which is `PLAN.md` §55's
    // defect exactly: the useful half of a refusal is at the end of it, and a
    // pane that clips stops before the part that says what to do.
    if let Some(detail) = detail {
        for text in wrap(&detail, width) {
            lines.push(Line::from(Span::styled(text, style)));
        }
    }
    lines
}

/// What to say about a reading's currency, and in what style.
///
/// Every branch is a different claim and none of them is *"here is a number"*
/// without a qualifier:
///
/// * **not asked yet** — distinct from unreadable, for `crate::tui::reach`'s
///   reason: a screen that says *unreadable* before it has looked is stating
///   something it does not know;
/// * **updated N ago** — and `STALE` past two and a half intervals;
/// * **the last refresh failed** — said in the error style, with the age of what
///   is still on screen, so nothing below reads as current;
/// * **not measured** — this machine's build has no such subject, which is
///   neither a value nor a failure to retry.
fn freshness(
    slot: &Slot,
    interval: Duration,
    now: Instant,
    palette: Palette,
) -> (String, Option<String>, ratatui::style::Style) {
    match &slot.outcome {
        Outcome::Never => ("not asked yet".to_string(), None, palette.unknown()),
        Outcome::Unsupported(_) => (
            format!(
                "NOT MEASURED — this machine's build has no `rivet get {}`",
                slot.subject.word()
            ),
            None,
            palette.warn(),
        ),
        Outcome::Answered => match slot.age(now) {
            None => (
                "answered, with nothing in it".to_string(),
                None,
                palette.warn(),
            ),
            Some(age) => {
                let text = format!("updated {} ago · every {}", ago(age), seconds(interval));
                if slot.is_stale(now, interval) {
                    (format!("{text} · STALE"), None, palette.warn())
                } else {
                    (text, None, palette.faint())
                }
            }
        },
        Outcome::Failed(why) => {
            // The whole refusal, on its own lines. A machine's own words are
            // what `AGENTS.md` §5 wants reported, and they do not fit on a
            // heading.
            let detail = Some(why.split_whitespace().collect::<Vec<_>>().join(" "));
            match slot.age(now) {
                // **The dangerous case, spelled out.** There are numbers below
                // and they are not current.
                Some(age) => (
                    format!("LAST REFRESH FAILED — the figures below are {} old", ago(age)),
                    detail,
                    palette.error(),
                ),
                None => ("could not be read".to_string(), detail, palette.error()),
            }
        }
    }
}

/// **The section the maintainer asked for first**, and the one that spent a
/// commit explaining why it was empty.
///
/// See the module header for what changed and what did not. The rule it is built
/// on: a utilisation is a rate, the machine answers counters, and **this pane
/// holds the second reading** — so before that second reading lands there is no
/// percentage and the section says so rather than drawing zero.
fn cpu(poller: &Poller, now: Instant, palette: Palette, width: u16) -> Vec<Line<'static>> {
    let mut lines = heading("CPU", poller, Subject::Cpu, now, palette, width);
    let Some(Reading::Cpu(reading)) = poller.slot(Subject::Cpu).and_then(Slot::value) else {
        return lines;
    };

    match poller.cpu_rates() {
        // **The first reading, and the honest thing to say about it.** A counter
        // read once says how the machine has spent its whole life; the
        // percentage a person wants is a difference, and there is nothing yet to
        // difference against.
        None => {
            for text in wrap(
                "no utilisation yet — a percentage is a RATE and needs two readings. The counters \
                 have arrived; the next refresh gives the figure. Nothing here is drawn as zero \
                 per cent, which would be a measurement of an idle machine.",
                width,
            ) {
                lines.push(Line::from(Span::styled(text, palette.faint())));
            }
        }
        Some(rates) => {
            lines.push(gauge(
                "busy",
                rates.total,
                100,
                &format!(
                    "{}% over {}  ·  {} processor(s)",
                    rates.total,
                    ago(rates.over),
                    reading.processors
                ),
                palette,
                true,
            ));
            // **Drawn apart from busy, and never added to it.** A board at 90%
            // iowait is not working, it is waiting — very often on a card that
            // is failing (`ADR-018` §7), which the `D` rows in the table below
            // say one process at a time.
            if rates.iowait > 0 {
                lines.push(gauge(
                    "iowait",
                    rates.iowait,
                    100,
                    &format!(
                        "{}% waiting on I/O — NOT work; cross-check the D states below",
                        rates.iowait
                    ),
                    palette,
                    true,
                ));
            }
            // **Per core.** One pinned while three idle is the failure worth
            // finding on a four-core board, and the aggregate above divides it
            // by four until it stops looking like anything.
            for (name, percent) in &rates.cpus {
                lines.push(gauge(
                    name,
                    *percent,
                    100,
                    &format!("{percent}%"),
                    palette,
                    true,
                ));
            }
        }
    }

    match reading.load {
        // **Absent, never three zeroes.** A load average of zero is a real
        // reading of an idle machine and it is the figure a person is likeliest
        // to act on without checking.
        None => {
            for text in wrap(
                "load         NOT MEASURED — this machine states no load average. It has not \
                 measured a load of zero; it has measured nothing.",
                width,
            ) {
                lines.push(Line::from(Span::styled(text, palette.warn())));
            }
        }
        Some(load) => {
            let stuck = reading.processors > 0
                && load.avg1 > reading.processors.saturating_mul(100);
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<14}", "load"), palette.faint()),
                Span::styled(
                    format!(
                        "{} / {} / {}   (1m / 5m / 15m)   {} runnable of {}",
                        two_places(load.avg1),
                        two_places(load.avg5),
                        two_places(load.avg15),
                        load.runnable,
                        load.existing
                    ),
                    if stuck { palette.warn() } else { palette.plain() },
                ),
            ]));
            // **The caveat travels with the number, and only when it bites.** A
            // load above the core count on Linux is either work queued or tasks
            // stuck in uninterruptible sleep, and the two look identical here —
            // which is exactly why the reader is sent to the process table
            // rather than told which it is.
            if stuck {
                for text in wrap(
                    "load is above this machine's processor count. ON LINUX THAT IS NOT \
                     NECESSARILY BUSY: the figure counts tasks in uninterruptible sleep as well \
                     as runnable ones, so on a Pi it is very often a failing card rather than \
                     work. Look for D states in the table below.",
                    width,
                ) {
                    lines.push(Line::from(Span::styled(text, palette.warn())));
                }
            }
        }
    }
    lines
}

/// Hundredths as `1.23`. [`hundredths`] with no percent sign, because a load
/// average is a count of tasks and is not a percentage of anything.
#[must_use]
pub fn two_places(value: i64) -> String {
    format!("{}.{:02}", value / 100, (value % 100).abs())
}

fn memory(poller: &Poller, now: Instant, palette: Palette, width: u16) -> Vec<Line<'static>> {
    let mut lines = heading("Memory", poller, Subject::Memory, now, palette, width);
    let Some(Reading::Memory(memory)) = poller.slot(Subject::Memory).and_then(Slot::value) else {
        return lines;
    };

    // **`available` against `total`, and it is the bar.** A healthy Linux has
    // almost no free memory because everything spare is page cache, and reading
    // `free` as headroom diagnoses exhaustion on a machine that is working
    // perfectly — `rivet_core::operation::memory` calls it a trap and this
    // screen refuses to draw the trap large.
    let used = memory.total_bytes.saturating_sub(memory.available_bytes);
    lines.push(gauge(
        "available",
        used,
        memory.total_bytes,
        &format!(
            "{} of {} available",
            human(memory.available_bytes),
            human(memory.total_bytes)
        ),
        palette,
        true,
    ));
    lines.push(field(
        "free",
        &format!(
            "{}  — NOT headroom; {} of the difference is page cache",
            human(memory.free_bytes),
            human(memory.cached_bytes)
        ),
        palette,
    ));
    lines.push(field(
        "swap",
        &if memory.swap_total_bytes == 0 {
            "none configured".to_string()
        } else {
            format!(
                "{} free of {}",
                human(memory.swap_free_bytes),
                human(memory.swap_total_bytes)
            )
        },
        palette,
    ));

    match memory.pressure {
        // **Absent is not zero.** A kernel without CONFIG_PSI has measured
        // nothing, and four zeroes here would say *nothing is stalling*.
        None => {
            for text in wrap(
                "pressure     NOT MEASURED — this kernel has no CONFIG_PSI. It has not measured \
                 zero stalling; it has measured nothing, and this is the one number that answers \
                 whether the box is struggling rather than merely full.",
                width,
            ) {
                lines.push(Line::from(Span::styled(text, palette.warn())));
            }
        }
        Some(p) => {
            // The 60-second average is the one that catches a machine that is
            // fine NOW and was thrashing a minute ago; the 10-second one will
            // already have recovered.
            let bad = p.some_avg60 > 0 || p.full_avg60 > 0;
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<14}", "pressure"), palette.faint()),
                Span::styled(
                    format!(
                        "some {} / {}   full {} / {}   (10s / 60s)",
                        hundredths(p.some_avg10),
                        hundredths(p.some_avg60),
                        hundredths(p.full_avg10),
                        hundredths(p.full_avg60)
                    ),
                    if bad { palette.warn() } else { palette.plain() },
                ),
            ]));
            if p.full_avg60 > 0 {
                lines.push(note(
                    "`full` above zero means EVERY runnable task was waiting on memory — the \
                     machine did no work for that fraction of the last minute.",
                    palette,
                ));
            }
        }
    }
    lines
}

/// **Thermals, led by the throttle rather than by the temperature.**
///
/// The survey's §7.1 order, and see the module header. The lines come out
/// strongest-first — has this board *ever* been held back, is its clock capped
/// now, and only then how warm it is — because a person who reads one line reads
/// the first one.
fn thermal(poller: &Poller, now: Instant, palette: Palette, width: u16) -> Vec<Line<'static>> {
    let mut lines = heading("Thermal", poller, Subject::Thermal, now, palette, width);
    let value = poller.slot(Subject::Thermal).and_then(Slot::value);
    match value {
        // The heading has already said what happened; nothing follows it.
        None => return lines,
        Some(Reading::Thermal(_)) => {}
        Some(_) => {
            lines.push(note("an unexpected reading for this subject", palette));
            return lines;
        }
    }
    let Some(Reading::Thermal(thermal)) = value else {
        return lines;
    };

    // ---- 1. The sticky flags: the only fact here that cannot be recovered
    // any other way. A board that is fine now and was under-volted at 03:00
    // looks identical to one that never was, except here.
    match (&thermal.throttle, thermal.ever_held_back()) {
        (Some(t), Some(true)) => {
            let mut held = Vec::new();
            if t.under_voltage_since_boot {
                held.push("UNDER-VOLTED");
            }
            if t.frequency_capped_since_boot {
                held.push("frequency capped");
            }
            if t.throttled_since_boot {
                held.push("throttled");
            }
            if t.soft_temperature_limit_since_boot {
                held.push("past its soft temperature limit");
            }
            let now_too: Vec<&str> = [
                (t.under_voltage_now, "under-voltage"),
                (t.frequency_capped_now, "a frequency cap"),
                (t.throttled_now, "throttling"),
                (t.soft_temperature_limit_now, "a soft temperature limit"),
            ]
            .into_iter()
            .filter_map(|(set, word)| set.then_some(word))
            .collect();
            for text in wrap(
                &format!(
                    "SINCE BOOT this board has been {}. {}. Under-voltage is the commonest cause \
                     of a Pi behaving strangely and a leading cause of card corruption — suspect \
                     the supply or the cable before the software.",
                    held.join(", "),
                    if now_too.is_empty() {
                        "It is not doing so right now".to_string()
                    } else {
                        format!("RIGHT NOW it is showing {}", now_too.join(", "))
                    }
                ),
                width,
            ) {
                lines.push(Line::from(Span::styled(text, palette.error())));
            }
        }
        (Some(_), Some(false)) => lines.push(field(
            "since boot",
            "never throttled or under-volted — the firmware's own flags, read",
            palette,
        )),
        // **NOT MEASURED, and never `no throttling`.** `Admin-Surface-Survey.md`
        // §7.6 measured that these flags need `/dev/vcio`; absent flags are not
        // clear flags, and eight falses would read as a clean bill of health for
        // the last several hours of this board's life.
        _ => {
            for text in wrap(
                &format!(
                    "since boot  NOT MEASURED — the firmware's throttle flags could not be read, \
                     so NOTHING here says whether this board has been under-volted, including at \
                     03:00 when nobody was looking. {}",
                    thermal
                        .throttle_unmeasured
                        .as_deref()
                        .unwrap_or("The machine gave no reason.")
                ),
                width,
            ) {
                lines.push(Line::from(Span::styled(text, palette.warn())));
            }
        }
    }

    // ---- 2. The clock against its ceiling: the throttle OBSERVED.
    match &thermal.clock {
        None => {
            for text in wrap(
                &format!(
                    "clock        NOT MEASURED — {}",
                    thermal
                        .clock_unmeasured
                        .as_deref()
                        .unwrap_or("no cpufreq policy was read.")
                ),
                width,
            ) {
                lines.push(Line::from(Span::styled(text, palette.warn())));
            }
        }
        Some(clock) => {
            // **`current` against `maximum` is the bar and NOT the alarm.** A
            // governor scales an idle machine down; a Pi at 600 MHz of 1500 is
            // behaving correctly, so this bar is never coloured as trouble.
            lines.push(gauge(
                "clock",
                clock.current_hz,
                clock.maximum_hz,
                &format!(
                    "{} of {} max{}{}",
                    megahertz(clock.current_hz),
                    megahertz(clock.maximum_hz),
                    clock
                        .governor
                        .as_ref()
                        .map_or_else(String::new, |g| format!("  governor {g}")),
                    if clock.at_maximum { "  at maximum" } else { "" }
                ),
                palette,
                // A clock below its maximum is the ordinary state and colouring
                // it as trouble teaches a person to ignore the colour.
                false,
            ));
            // **THIS is the throttle**: the ceiling itself moving, which is a
            // different comparison from the one above and the only one that
            // means anything.
            match (clock.ceiling_hz, clock.ceiling_below_maximum) {
                (Some(ceiling), Some(true)) => {
                    for text in wrap(
                        &format!(
                            "CEILING LOWERED to {} from a maximum of {} — this is a throttle \
                             OBSERVED rather than inferred: policy is refusing to let this \
                             machine reach its own top speed.",
                            megahertz(ceiling),
                            megahertz(clock.maximum_hz)
                        ),
                        width,
                    ) {
                        lines.push(Line::from(Span::styled(text, palette.error())));
                    }
                }
                (Some(ceiling), _) => lines.push(field(
                    "ceiling",
                    &format!(
                        "{} — the full maximum, so nothing is capping this clock",
                        megahertz(ceiling)
                    ),
                    palette,
                )),
                // No ceiling to compare. Said, rather than left as a gap that
                // reads like an all-clear.
                _ => lines.push(note(
                    "no policy ceiling was reported, so nothing here says whether this clock is \
                     capped",
                    palette,
                )),
            }
            if let Some(minimum) = clock.minimum_hz {
                lines.push(field(
                    "floor",
                    &format!("{} — where the governor idles it", megahertz(minimum)),
                    palette,
                ));
            }
        }
    }

    // ---- 3. The temperature, last, because it is the weakest of the four and
    // a board that throttled an hour ago reads cool by the time anyone looks.
    match thermal.hottest() {
        None => lines.push(note(
            "no thermal zone was found, so no temperature was measured — a fact about this \
             machine's interfaces rather than a temperature of zero",
            palette,
        )),
        Some(zone) => {
            let style = if zone.millidegrees >= 80_000 {
                palette.error()
            } else if zone.millidegrees >= 70_000 {
                palette.warn()
            } else {
                palette.plain()
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<14}", "temperature"), palette.faint()),
                Span::styled(
                    format!(
                        "{} ({}){}",
                        celsius(zone.millidegrees),
                        zone.name,
                        if thermal.zones.len() > 1 {
                            format!("  — hottest of {}", thermal.zones.len())
                        } else {
                            String::new()
                        }
                    ),
                    style,
                ),
                Span::styled("  a SPOT reading", palette.faint()),
            ]));
        }
    }
    if !thermal.zones_unreadable.is_empty() {
        for text in wrap(
            &format!(
                "{} zone(s) could not be read: {}. Named rather than dropped — an answer that \
                 omitted one would say this machine has no such zone.",
                thermal.zones_unreadable.len(),
                thermal.zones_unreadable.join("; ")
            ),
            width,
        ) {
            lines.push(Line::from(Span::styled(text, palette.warn())));
        }
    }
    lines
}

/// **What this machine is, as opposed to what it is doing** — the `once` tier.
///
/// `slot`, `boot` and `device` are read once and not re-asked, because none of
/// them can change without a reboot and a reboot ends the session watching. The
/// uptime is the exception and is handled by arithmetic rather than by polling:
/// the reading's age is added to it, so the figure on screen is current without
/// a request having been made for it.
fn machine(poller: &Poller, now: Instant, palette: Palette, width: u16) -> Vec<Line<'static>> {
    let mut lines = heading("Machine", poller, Subject::Slot, now, palette, width);

    if let Some(Reading::Slots(slots)) = poller.slot(Subject::Slot).and_then(Slot::value) {
        match (&slots.running, slots.agree) {
            // **The finding this subject exists for.** The firmware read one
            // slot's boot partition and the kernel mounted the other's
            // filesystem: `apply trial-boot` would arm the wrong partition.
            (None, Some(false)) => {
                for text in wrap(
                    &format!(
                        "SLOT: THE TWO AUTHORITIES DISAGREE — the firmware booted {} and the \
                         kernel mounted {} as /. No running slot is named because there is not \
                         one, and NOTHING SHOULD BE ARMED on this machine until somebody \
                         understands why.",
                        slots.booted.as_deref().unwrap_or("?"),
                        slots.root.as_deref().unwrap_or("?")
                    ),
                    width,
                ) {
                    lines.push(Line::from(Span::styled(text, palette.error())));
                }
            }
            (Some(running), _) => lines.push(field(
                "slot",
                &format!(
                    "running {running}{}{}",
                    slots
                        .inactive
                        .as_ref()
                        .map_or_else(String::new, |i| format!("  ·  an update writes {i}")),
                    if slots.agree == Some(true) {
                        "  ·  firmware and kernel agree"
                    } else {
                        "  ·  one authority only"
                    }
                ),
                palette,
            )),
            (None, _) => lines.push(note(
                "no A/B slot was named — this machine was not booted from an ADR-018 §4 layout, \
                 which is a true answer rather than a failure",
                palette,
            )),
        }
        match slots.in_trial_boot {
            Some(true) => {
                for text in wrap(
                    "THIS BOOT IS A ONE-SHOT TRIAL. An ordinary restart goes back to the other \
                     slot — ADR-018 §5.1 rolls a trial back unless it is committed, and doing \
                     nothing is what rolls it back.",
                    width,
                ) {
                    lines.push(Line::from(Span::styled(text, palette.warn())));
                }
            }
            Some(false) => {}
            // **Absent, never `false`.** `false` asserts that doing nothing is
            // safe, which is the one claim an unread flag must not make.
            None => {
                for text in wrap(
                    &format!(
                        "trial        NOT MEASURED — this is not the same as its being a \
                         committed boot. {}",
                        slots
                            .trial_unmeasured
                            .as_deref()
                            .unwrap_or("The machine gave no reason.")
                    ),
                    width,
                ) {
                    lines.push(Line::from(Span::styled(text, palette.warn())));
                }
            }
        }
    }

    if let Some((slot, Reading::Boot(boot))) = poller
        .slot(Subject::Boot)
        .and_then(|slot| Some((slot, slot.value()?)))
    {
        // **The reading's age is added to the uptime**, which is what makes a
        // once-only reading current: a clock that ticks needs no request.
        let since = slot.age(now).unwrap_or(Duration::ZERO).as_secs();
        let mut text = format!(
            "{} — off the monotonic counter",
            uptime(boot.uptime_seconds.saturating_add(since))
        );
        if let Some(boots) = boot.boots {
            text.push_str(&format!("  ·  {boots} boot(s) in the journal"));
        }
        lines.push(field("uptime", &text, palette));
        if boot.recorded == Some(false) {
            for wrapped in wrap(
                "THIS BOOT IS NOT IN THE JOURNAL, which means nothing has been written to it \
                 during this boot — on a booted Rivet that means /state did not mount.",
                width,
            ) {
                lines.push(Line::from(Span::styled(wrapped, palette.error())));
            }
        }
    }

    if let Some(Reading::Devices(devices)) = poller.slot(Subject::Device).and_then(Slot::value) {
        let total: u64 = devices
            .devices
            .iter()
            .filter(|d| d.kind.as_deref() == Some("disk"))
            .filter_map(|d| d.size_bytes)
            .sum();
        let written: u64 = devices
            .devices
            .iter()
            .filter(|d| d.kind.as_deref() == Some("disk"))
            .filter_map(|d| d.written_bytes)
            .sum();
        let mut text = format!(
            "{} disk(s), {} device(s) in all{}",
            devices.disks,
            devices.devices.len(),
            if total > 0 {
                format!("  ·  {}", human(total))
            } else {
                String::new()
            }
        );
        if written > 0 {
            // `ADR-018` §7 treats SD wear as a design constraint and nothing has
            // ever measured any of it. This is the first figure that does.
            text.push_str(&format!("  ·  {} written since boot", human(written)));
        }
        lines.push(field("devices", &text, palette));
        // **Endurance is verbatim or absent.** A consumer SD card states none,
        // and a percentage computed for one would be invented.
        for device in &devices.devices {
            if let Some(life) = &device.life_time {
                lines.push(field(
                    &device.name,
                    &format!(
                        "life_time {life}{}  — the device's own vendor code, not a percentage",
                        device
                            .pre_eol_info
                            .as_ref()
                            .map_or_else(String::new, |e| format!("  pre_eol {e}"))
                    ),
                    palette,
                ));
            }
        }
        if devices.unreadable > 0 {
            lines.push(note(
                &format!(
                    "{} device(s) could not be read and are named in the answer rather than \
                     dropped",
                    devices.unreadable
                ),
                palette,
            ));
        }
    }
    lines
}

fn storage(poller: &Poller, now: Instant, palette: Palette, width: u16) -> Vec<Line<'static>> {
    let mut lines = heading("Storage", poller, Subject::Storage, now, palette, width);
    let Some(Reading::Storage(storage)) = poller.slot(Subject::Storage).and_then(Slot::value)
    else {
        return lines;
    };
    for filesystem in &storage.filesystems {
        // A pseudo-filesystem has no size at all. Nothing is filtered out —
        // `get storage`'s own ruling, and hiding a row is how *is /state full?*
        // becomes unanswerable on the one machine where the filter is wrong —
        // but a bar drawn over a zero total would be a bar that means nothing.
        if filesystem.total_bytes == 0 {
            lines.push(field(
                &filesystem.mount_point,
                &format!("{} — no size to report", filesystem.fstype),
                palette,
            ));
            continue;
        }
        let label = format!(
            "{} used of {}{}",
            human(filesystem.used_bytes),
            human(filesystem.total_bytes),
            if filesystem.read_only { "  read-only" } else { "" }
        );
        lines.push(gauge(
            &filesystem.mount_point,
            filesystem.used_bytes,
            filesystem
                .used_bytes
                .saturating_add(filesystem.available_bytes),
            &label,
            palette,
            // **A full read-only filesystem is not a problem.** `/system` is a
            // squashfs and is 100% full by construction — `ADR-018` §4 — and
            // colouring it as trouble teaches a person to ignore the colour,
            // which is the whole cost of a false alarm. Seen on the first run in
            // a terminal.
            !filesystem.read_only,
        ));
    }
    if storage.unreadable > 0 {
        for text in wrap(
            &format!(
                "{} mount(s) could not be measured: {}. They are named rather than dropped — an \
                 answer that omitted one would say there is no such filesystem.",
                storage.unreadable,
                storage.unreadable_detail.join("; ")
            ),
            width,
        ) {
            lines.push(Line::from(Span::styled(text, palette.warn())));
        }
    }
    lines
}

/// **What the supervisor believes, against what the kernel says.**
///
/// See the module header. The section exists for the disagreements, so it leads
/// with them and only then lists what agrees.
fn agreement(
    poller: &Poller,
    now: Instant,
    glyphs: Glyphs,
    palette: Palette,
    width: u16,
) -> Vec<Line<'static>> {
    let mut lines = heading("Services", poller, Subject::Status, now, palette, width);
    let Some(Reading::Supervisor(supervisor)) = poller.slot(Subject::Status).and_then(Slot::value)
    else {
        return lines;
    };
    let Some(Reading::Processes(processes)) =
        poller.slot(Subject::Process).and_then(Slot::value)
    else {
        lines.push(note(
            "the kernel's process table has not arrived, so nothing is cross-checked yet",
            palette,
        ));
        return lines;
    };

    // **The two halves are not simultaneous**, and that is said rather than
    // hoped away: `status` is on the medium tier and `process` on the fast one.
    if let (Some(status_age), Some(process_age)) = (
        poller.slot(Subject::Status).and_then(|s| s.age(now)),
        poller.slot(Subject::Process).and_then(|s| s.age(now)),
    ) {
        let apart = status_age.abs_diff(process_age);
        lines.push(note(
            &format!(
                "compared across readings {} apart — the supervisor's is on a slower tier",
                ago(apart)
            ),
            palette,
        ));
    }

    lines.push(field("phase", &supervisor.phase, palette));
    for service in &supervisor.services {
        let running = processes
            .rows
            .iter()
            .find(|p| Some(p.pid) == service.pid && !p.kernel);
        let (glyph, word, style) = match (service.pid, running) {
            // The supervisor holds a pid and the kernel has that process. The
            // ordinary case.
            (Some(_), Some(_)) => (glyphs.up, "running".to_string(), palette.up()),
            // **The disagreement worth finding.** The supervisor believes it is
            // running something that is not there.
            (Some(pid), None) => (
                glyphs.warn,
                format!("claims pid {pid} — NO SUCH PROCESS"),
                palette.error(),
            ),
            // No pid: the supervisor is not running it and says so.
            (None, _) => (
                glyphs.off,
                format!("{} (supervisor holds no pid)", service.state),
                palette.faint(),
            ),
        };
        let mut spans = vec![
            Span::styled(format!("  {glyph} "), style),
            Span::styled(format!("{:<16}", service.name), palette.plain()),
            Span::styled(word, style),
        ];
        if service.restarts > 0 {
            spans.push(Span::styled(
                format!("  {} restart(s)", service.restarts),
                palette.warn(),
            ));
        }
        lines.push(Line::from(spans));
    }

    // **A userland process no service claims.** On a machine with no shell this
    // should be impossible, which is what makes it worth a line — and the
    // process answering this very question is one of them, so it is expected to
    // be non-empty rather than empty.
    let claimed: Vec<i64> = supervisor.services.iter().filter_map(|s| s.pid).collect();
    let unclaimed: Vec<String> = processes
        .rows
        .iter()
        .filter(|p| !p.kernel && p.pid != 1 && !claimed.contains(&p.pid))
        .map(|p| format!("{} ({})", p.name, p.pid))
        .collect();
    if !unclaimed.is_empty() {
        for text in wrap(
            &format!(
                "userland the supervisor does not claim: {}. Some of these are this very \
                 question being answered; anything else on a machine with no shell is a finding.",
                unclaimed.join(", ")
            ),
            width,
        ) {
            lines.push(Line::from(Span::styled(text, palette.faint())));
        }
    }
    lines
}

fn processes(
    poller: &Poller,
    view: &Monitor,
    now: Instant,
    glyphs: Glyphs,
    palette: Palette,
    width: u16,
) -> Vec<Line<'static>> {
    let mut lines = heading("Processes", poller, Subject::Process, now, palette, width);
    let Some(Reading::Processes(processes)) =
        poller.slot(Subject::Process).and_then(Slot::value)
    else {
        return lines;
    };

    let hidden = processes.rows.iter().filter(|p| p.kernel).count();
    let mut rows: Vec<&crate::telemetry::Process> = processes
        .rows
        .iter()
        .filter(|p| view.kernel_threads || !p.kernel)
        .collect();
    let rates = poller.process_rates();
    match view.order {
        // Descending, because the question is *what is using this machine*. A
        // process with no measured rate sorts below every one that has one —
        // `None` is *not known*, not *zero*, and putting it at the bottom is the
        // one placement that does not assert either.
        Order::Cpu => rows.sort_by(|a, b| {
            let of = |p: &crate::telemetry::Process| rates.and_then(|r| r.of(p.pid));
            of(b).cmp(&of(a)).then_with(|| a.pid.cmp(&b.pid))
        }),
        Order::Resident => rows.sort_by(|a, b| {
            b.resident_bytes
                .cmp(&a.resident_bytes)
                .then_with(|| a.pid.cmp(&b.pid))
        }),
        Order::Pid => rows.sort_by_key(|p| p.pid),
        Order::Name => rows.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.pid.cmp(&b.pid))),
    }

    let mut summary = format!(
        "{} userland · {} kernel thread(s) {}",
        processes.userland,
        hidden,
        if view.kernel_threads { "shown" } else { "hidden" }
    );
    if processes.vanished > 0 {
        // Not an error: /proc changes while it is read. On screen because a
        // count that silently shrinks is one nobody can reason about, and a
        // large number on an idle machine is itself a finding.
        summary.push_str(&format!(" · {} vanished mid-read", processes.vanished));
    }
    lines.push(note(&summary, palette));

    // **The first reading has no CPU column to fill**, and the header says so
    // rather than printing `CPU%` over a column of dashes a person would read as
    // idle processes.
    match rates {
        Some(rates) => lines.push(note(
            &format!(
                "CPU% is a share of ONE processor measured over {} — a four-threaded process on \
                 four cores reads 400%",
                ago(rates.over)
            ),
            palette,
        )),
        None => lines.push(note(
            "CPU% needs two readings and only one has arrived — the next refresh fills the \
             column. A dash there is NOT MEASURED, never idle.",
            palette,
        )),
    }

    lines.push(Line::from(Span::styled(
        format!(
            "  {:>6} {:>6} {:<2} {:>4} {:>7} {:>10}  {}",
            "PID", "PPID", "S", "THR", "CPU%", "RESIDENT", "NAME"
        ),
        palette.faint(),
    )));

    // **Every row, and the pane scrolls.** The table used to window itself, and
    // on a 44-row terminal that meant the sections above it consumed the screen
    // and no process was on it at all. `draw` scrolls the whole body instead.
    for process in rows {
        let style = if process.is_trouble() {
            palette.warn()
        } else {
            palette.plain()
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!(
                    "  {:>6} {:>6} {:<2} {:>4} {:>7} {:>10}  ",
                    process.pid,
                    process.parent_pid,
                    process.state,
                    process.threads,
                    // **A dash is NOT MEASURED and never idle**, and there are
                    // three ways to reach it: no second reading yet, a process
                    // that did not exist at the first one, and a machine whose
                    // /proc/<pid>/stat could not be read. All three are *this is
                    // not known*, and `0.00` would be a measurement.
                    rates
                        .and_then(|r| r.of(process.pid))
                        .map_or_else(|| "—".to_string(), |p| format!("{}.{:02}", p / 100, p % 100)),
                    // **Absent, never zero.** A kernel thread has no user
                    // address space; `0` would claim it occupies none.
                    process
                        .resident_bytes
                        .map_or_else(|| "—".to_string(), human)
                ),
                style,
            ),
            Span::styled(clip(&process.name, width), style),
            if process.is_trouble() {
                Span::styled(format!("  {} {}", glyphs.warn, trouble(&process.state)), palette.warn())
            } else {
                Span::styled(String::new(), palette.plain())
            },
        ]));
    }
    lines
}

/// What a process state that is worth reading twice means.
///
/// `rivet_core::operation::process`: uninterruptible sleep on a Pi is very often
/// I/O against a failing SD card, which is `ADR-018` §7's whole concern, and it
/// will not appear as a service failure.
fn trouble(state: &str) -> &'static str {
    if state.starts_with('D') {
        "uninterruptible — often a failing card"
    } else {
        "zombie — its parent is not reaping"
    }
}

// ---------------------------------------------------------------------------
// Rendering pieces
// ---------------------------------------------------------------------------

/// A `name  value` line, matching `crate::tui::device::field`'s shape.
fn field(name: &str, value: &str, palette: Palette) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {name:<14}"), palette.faint()),
        Span::styled(value.to_owned(), palette.plain()),
    ])
}

fn note(text: &str, palette: Palette) -> Line<'static> {
    Line::from(Span::styled(format!("  {text}"), palette.faint()))
}

/// A labelled bar.
///
/// **The label carries the numbers and the bar carries the proportion**, so a
/// terminal that renders the bar badly loses nothing: `crate::tui::theme`'s rule
/// that no glyph is load-bearing, applied to a shape rather than to an icon.
fn gauge(
    name: &str,
    part: u64,
    whole: u64,
    label: &str,
    palette: Palette,
    // Whether being full is worth saying something about. It is not, on a
    // read-only filesystem.
    fullness_matters: bool,
) -> Line<'static> {
    let percent = percent(part, whole);
    let filled = percent.map_or(0, |p| (usize::try_from(p).unwrap_or(0) * BAR) / 100);
    let style = match percent {
        Some(p) if p >= 90 && fullness_matters => palette.error(),
        Some(p) if p >= 75 && fullness_matters => palette.warn(),
        _ => palette.plain(),
    };
    Line::from(vec![
        Span::styled(format!("  {name:<14}"), palette.faint()),
        Span::styled(
            format!(
                "[{}{}] ",
                "#".repeat(filled.min(BAR)),
                ".".repeat(BAR.saturating_sub(filled))
            ),
            style,
        ),
        Span::styled(
            // **A proportion of nothing is not zero percent.** A filesystem with
            // no total says so rather than reading as empty.
            percent.map_or_else(|| "  — ".to_string(), |p| format!("{p:>3}%")),
            style,
        ),
        Span::styled(format!("  {label}"), palette.plain()),
    ])
}

/// `part` as a whole percent of `whole`, or `None` when there is no such
/// proportion.
#[must_use]
pub fn percent(part: u64, whole: u64) -> Option<u64> {
    if whole == 0 {
        return None;
    }
    Some((part.saturating_mul(100) / whole).min(100))
}

/// Bytes, as a person reads them.
///
/// **Binary units, named as binary units.** A board with 971,505,664 bytes of
/// memory has 926 MiB, and calling that 971 MB — or worse, 926 MB — is how two
/// readings of the same machine disagree.
#[must_use]
pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    let name = UNITS.get(unit).copied().unwrap_or("B");
    if value >= 100.0 {
        format!("{value:.0} {name}")
    } else {
        format!("{value:.1} {name}")
    }
}

/// Hundredths of a percent, as a percentage.
///
/// The unit is `rivet_core::operation::memory`'s: the structured surface has no
/// float, and whole percent would collapse the difference between a machine
/// stalling 0.4% of the time and one that is not stalling at all. **So the two
/// decimal places are the measurement and not a flourish.**
#[must_use]
pub fn hundredths(value: i64) -> String {
    format!("{}.{:02}%", value / 100, (value % 100).abs())
}

/// A duration, as short as it can be said without becoming vague.
#[must_use]
pub fn ago(duration: Duration) -> String {
    let millis = duration.as_millis();
    if millis < 10_000 {
        return format!("{}.{}s", millis / 1000, (millis % 1000) / 100);
    }
    let secs = duration.as_secs();
    if secs < 120 {
        return format!("{secs}s");
    }
    if secs < 7200 {
        return format!("{}m{}s", secs / 60, secs % 60);
    }
    format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
}

/// An interval, for the header. Whole seconds, because every one of them is.
#[must_use]
pub fn seconds(interval: Duration) -> String {
    format!("{}s", interval.as_secs())
}

/// Hertz as megahertz, which is how a clock is spoken about.
///
/// **Whole MHz.** A Pi's `cpufreq` steps in 100 MHz and the decimals would be
/// noise; a machine that reported an odd figure still shows it, rounded, rather
/// than being refused.
#[must_use]
pub fn megahertz(hz: u64) -> String {
    format!("{} MHz", hz / 1_000_000)
}

/// Thousandths of a degree as degrees, to one place.
///
/// The unit is Linux's and the conversion is here rather than in the reader for
/// [`human`]'s reason: what the machine stated travels unchanged and this is a
/// rendering decision.
#[must_use]
pub fn celsius(millidegrees: i64) -> String {
    format!("{}.{}°C", millidegrees / 1000, (millidegrees % 1000).abs() / 100)
}

/// Seconds since boot, as a person says them.
///
/// **Days, because a Rivet is meant to stay up.** `27d4h` is the answer to *has
/// this board rebooted*, and `2352847s` is not.
#[must_use]
pub fn uptime(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    if days > 0 {
        return format!("{days}d{hours}h{minutes}m");
    }
    if hours > 0 {
        return format!("{hours}h{minutes}m");
    }
    format!("{minutes}m{}s", seconds % 60)
}

/// Cut a name that would run past the frame.
///
/// A line that overflows corrupts the box around it and not just itself —
/// `crate::tui::device::wrapped` found that the hard way.
fn clip(text: &str, width: u16) -> String {
    let room = usize::from(width).saturating_sub(40).max(8);
    if text.chars().count() <= room {
        return text.to_owned();
    }
    text.chars().take(room).collect()
}

/// Break a sentence into lines that fit, with this pane's two-column margin.
///
/// `crate::tui::device::wrapped` is the same function and is `pub(super)` for
/// this caller — one wrap, not two, so a message that is readable in one pane is
/// readable in the other.
fn wrap(text: &str, width: u16) -> Vec<String> {
    crate::tui::device::wrapped(text, width)
}

#[cfg(test)]
#[path = "top_tests.rs"]
mod tests;
