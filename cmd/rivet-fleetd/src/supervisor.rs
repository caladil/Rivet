//! **Is the supervisor alive?** — `ADR-056` §3, and `.rulings/` R-195 D2.
//!
//! A page a person opens when a machine has stopped answering, answering one
//! question and refusing every other. It reads `/proc` in **this** process and
//! makes no connection at all — `ADR-056` §3, because `rivet-mcpd` is the
//! first thing to go silent and every other page in this server reaches the
//! machine through it.
//!
//! # What the incident of 2026-09-16 established, and why this shape
//!
//! `docs/design-candidates/Supervisor-Blocking-Audit-2026-09-16.md` §10.4 took
//! the eight signals Fleet Monitor was capturing and split them by whether the
//! answer needs the supervisor. `get thermal`, `get boot`, `get memory` and the
//! journal read all answered in tens of milliseconds for **eleven minutes**
//! after the supervisor stopped; `get status`, which asks it, timed out every
//! minute forever. So the failure is confined to `rivet-sysd` and it is total —
//! journal ingest, health polling, supervision and answering stopped in one
//! instant on a loop with one thread.
//!
//! Two consequences shape everything below:
//!
//! 1. **Nothing here may ask the supervisor anything**, including indirectly.
//!    The identification of §3.3 is `getppid`, which is a syscall and not a
//!    question.
//! 2. **A single sample is worthless.** Every field is a state or a cumulative
//!    counter, and the question — *is it turning* — is a difference. So the
//!    page takes two samples about a second apart inside one request and shows
//!    both the values and what moved. §3.5 is emphatic about this, and the
//!    reason is the reader at three in the morning who would otherwise have to
//!    hold a number in their head and press refresh.
//!
//! # What it cannot say
//!
//! **Where** the supervisor is stuck. `wchan`, `syscall` and `stack` name the
//! blocking function in one word, and this program runs as an application uid;
//! two of the three are mode `0400` root. The audit §10.6 measured that on the
//! board rather than assuming it. Widening this server to reach them would break
//! `ADR-015` §5 for every application, so the page says *it has stopped* and
//! says plainly that it cannot say where.
//!
//! # Why it is deliberately awkward to parse
//!
//! `ADR-056` §3. The maintainer's constraint is that this must not become
//! something an agent reaches for instead of `rivet-mcpd`, and access control
//! cannot deliver that — anyone who can open this page holds the same operator
//! certificate the control plane takes. So the page is made **useless as a
//! machine interface**: HTML only, no JSON anywhere on the path, numbers inside
//! English sentences, no inputs to vary, and **no stable contract** — every
//! name, number, phrase and ordering here may change in any release with no
//! notice. Nothing is permitted to depend on it.

/// `USER_HZ`, the unit `utime` and `stime` are counted in.
///
/// Fixed at 100 by the kernel ABI for every architecture Rivet targets, and
/// `sysconf(_SC_CLK_TCK)` would be the general answer at the cost of a libc
/// dependency this crate does not otherwise carry. The page reports seconds
/// derived from it and also the raw ticks, so a reader is never relying on this
/// constant alone to know whether a counter moved.
const USER_HZ: u64 = 100;

/// How far apart the two samples of §3.5 are taken.
///
/// **Not configurable, and `ADR-056` §3 is why**: a page with an input is a
/// page that can be tuned into something cheap to poll. It is also the reason
/// the whole request costs a second — that cost is the design working.
const BETWEEN_SAMPLES: std::time::Duration = std::time::Duration::from_millis(1000);

/// Who this page decided the supervisor is — `ADR-056` §3.
///
/// The supervisor spawns every service and every application, so an
/// application's parent **is** the supervisor. There is no scan and no name
/// lookup: a scan would need a directory grant over `/proc` that
/// `rivet_core::confinement` refuses on the record, and it would be wrong on its
/// own terms, because two processes can carry the same `comm` and the one this
/// program's fate is tied to is its parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Identified {
    /// The same pid at startup and at render, and `comm` agrees.
    Supervisor { pid: u32, comm: String },
    /// The parent at render is pid 1: the supervisor that started this
    /// application is gone and this process was reparented to `rivet-initd`.
    /// **That is an answer, not a failure of the page.**
    Reparented { was: u32 },
    /// The two pids differ and neither is 1 — the supervisor was replaced while
    /// this application kept running. Worth naming: it means a restart nobody
    /// asked about.
    Replaced { was: u32, now: u32 },
    /// The parent at startup was already 1, so this process was not started by
    /// a supervisor and the page cannot identify one. It says so rather than
    /// guessing.
    NoSupervisor,
    /// The parent is identified but its `/proc` cannot be read — the confinement
    /// of `ADR-056` §3 is absent, or `/proc` is not mounted.
    Unreadable { pid: u32, why: String },
}

/// One reading of the supervisor — `ADR-056` §3.
///
/// Every field is a **state** or a **cumulative counter**. None of them is a
/// rate, and none of them means anything on its own; [`Moved`] is where they
/// become an answer.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sample {
    /// `R`, `S`, `D`, `Z`, `T` — running, sleeping, uninterruptible, zombie,
    /// stopped. The audit found every candidate wait lands in `S`, which is
    /// exactly why this letter is not the answer by itself.
    pub state: String,
    /// Whether the supervisor is still the single-threaded program its own
    /// safety argument depends on — `ADR-056` §7.
    pub threads: u64,
    /// How many times the loop parked itself. A loop that is turning moves this
    /// constantly.
    pub voluntary: u64,
    /// Preemption, which separates *starved* from *parked*.
    pub nonvoluntary: u64,
    /// A signal that is blocked and pending is one the `signalfd` has not been
    /// read for — a direct witness that the loop is not draining it.
    pub pending: String,
    /// What the supervisor blocks, which is what makes `pending` legible.
    pub blocked: String,
    /// Cumulative user and system time, in `USER_HZ` ticks.
    pub utime: u64,
    /// Cumulative system time, in `USER_HZ` ticks.
    pub stime: u64,
    /// The supervisor's own age in ticks since boot, which says whether it has
    /// been restarted since this machine started.
    pub starttime: u64,
    /// Seconds since boot — the interval the counters above are read against,
    /// and the number that makes *"the supervisor has done nothing for five
    /// hours"* sayable at all.
    pub uptime: f64,
}

/// What moved between two samples — the whole question, as a difference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moved {
    pub voluntary: u64,
    pub nonvoluntary: u64,
    pub ticks: u64,
}

impl Moved {
    /// **Did the loop turn at all?**
    ///
    /// Any one of the three moving is enough. A supervisor that burned CPU
    /// without parking is alive and busy; one that parked is alive and idle;
    /// one that did neither, over a whole second, did nothing.
    #[must_use]
    pub fn turned(self) -> bool {
        self.voluntary > 0 || self.nonvoluntary > 0 || self.ticks > 0
    }
}

/// What one request found: who the supervisor is, its state at the end of the
/// second, and what moved during it.
///
/// **The first sample is not kept.** It exists only to make [`Moved`] a
/// difference, and a page carrying both readings would be inviting a reader to
/// subtract two numbers that have already been subtracted for them — which is
/// the thing §3.5 says nobody does correctly at three in the morning.
#[derive(Debug, Clone)]
pub struct Reading {
    pub who: Identified,
    pub second: Sample,
    pub moved: Moved,
}

/// The parent pid this process was started with, recorded once.
///
/// `ADR-056` §3 compares this against the parent at render, and the
/// comparison is itself diagnostic — see [`Identified`].
static STARTED_UNDER: std::sync::OnceLock<u32> = std::sync::OnceLock::new();

/// Record the parent pid at startup. Call once, from `main`, before anything
/// can have reparented this process.
pub fn remember_parent() {
    let _ = STARTED_UNDER.set(parent_now());
}

/// This process's parent, right now.
///
/// A syscall, not a question put to anybody — which is the property that makes
/// it usable when the supervisor has stopped.
fn parent_now() -> u32 {
    std::os::unix::process::parent_id()
}

/// Decide who the supervisor is, per `ADR-056` §3's table.
fn identify(started_under: u32, now: u32, comm: Result<String, String>) -> Identified {
    if started_under == 1 {
        return Identified::NoSupervisor;
    }
    if now == 1 {
        return Identified::Reparented { was: started_under };
    }
    if now != started_under {
        return Identified::Replaced {
            was: started_under,
            now,
        };
    }
    match comm {
        Ok(comm) => Identified::Supervisor {
            pid: now,
            comm: comm.trim().to_string(),
        },
        Err(why) => Identified::Unreadable { pid: now, why },
    }
}

/// `State`, `Threads`, `voluntary_ctxt_switches` and the rest, out of
/// `/proc/<pid>/status`.
///
/// Tolerant by construction: a field this kernel does not carry leaves its
/// value at the default rather than failing the whole read, because a page that
/// refused to render over one missing line would be a page that says nothing on
/// the day it is needed.
fn parse_status(text: &str, into: &mut Sample) {
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key {
            "State" => into.state = value.to_string(),
            "Threads" => into.threads = value.parse().unwrap_or_default(),
            "voluntary_ctxt_switches" => into.voluntary = value.parse().unwrap_or_default(),
            "nonvoluntary_ctxt_switches" => into.nonvoluntary = value.parse().unwrap_or_default(),
            "ShdPnd" => into.pending = value.to_string(),
            "SigBlk" => into.blocked = value.to_string(),
            _ => {}
        }
    }
}

/// `utime`, `stime` and `starttime` out of `/proc/<pid>/stat`.
///
/// **The comm field is split off by the last `)` and not by whitespace.** A
/// process name may contain spaces and parentheses, so a whitespace split of
/// this file is a parser that works until somebody names a program badly. The
/// fields after that closing bracket are 1-indexed from `state` as field 3,
/// which puts `utime` at 14, `stime` at 15 and `starttime` at 22.
fn parse_stat(text: &str, into: &mut Sample) {
    let Some(close) = text.rfind(')') else {
        return;
    };
    let rest: Vec<&str> = text[close + 1..].split_whitespace().collect();
    // `rest[0]` is field 3, so field N is `rest[N - 3]`.
    let field = |n: usize| -> u64 {
        rest.get(n.wrapping_sub(3))
            .and_then(|v| v.parse().ok())
            .unwrap_or_default()
    };
    into.utime = field(14);
    into.stime = field(15);
    into.starttime = field(22);
}

/// Seconds since boot, out of `/proc/uptime`'s first number.
fn parse_uptime(text: &str) -> f64 {
    text.split_whitespace()
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or_default()
}

/// Read one sample of a pid.
fn sample(pid: u32) -> Sample {
    let dir = rivet_core::confinement::supervisor_proc_dir(pid);
    let mut out = Sample::default();
    if let Ok(text) = std::fs::read_to_string(format!("{dir}/status")) {
        parse_status(&text, &mut out);
    }
    if let Ok(text) = std::fs::read_to_string(format!("{dir}/stat")) {
        parse_stat(&text, &mut out);
    }
    if let Ok(text) = std::fs::read_to_string(rivet_core::confinement::UPTIME_PATH) {
        out.uptime = parse_uptime(&text);
    }
    out
}

/// Take the whole reading: identify, sample, wait, sample again.
///
/// **This blocks for [`BETWEEN_SAMPLES`]**, which is the design and not an
/// oversight — see §3.5 and §3.7.
#[must_use]
pub fn read() -> Reading {
    let started_under = *STARTED_UNDER.get_or_init(parent_now);
    let now = parent_now();
    let comm = std::fs::read_to_string(format!(
        "{}/comm",
        rivet_core::confinement::supervisor_proc_dir(now)
    ))
    .map_err(|e| e.to_string());
    let who = identify(started_under, now, comm);

    let pid = match &who {
        Identified::Supervisor { pid, .. } | Identified::Unreadable { pid, .. } => Some(*pid),
        Identified::Replaced { now, .. } => Some(*now),
        Identified::Reparented { .. } | Identified::NoSupervisor => None,
    };

    let (first, second) = match pid {
        Some(pid) => {
            let first = sample(pid);
            std::thread::sleep(BETWEEN_SAMPLES);
            (first, sample(pid))
        }
        None => (Sample::default(), Sample::default()),
    };

    let moved = Moved {
        voluntary: second.voluntary.saturating_sub(first.voluntary),
        nonvoluntary: second.nonvoluntary.saturating_sub(first.nonvoluntary),
        ticks: (second.utime + second.stime).saturating_sub(first.utime + first.stime),
    };

    Reading {
        who,

        second,
        moved,
    }
}

/// Escape text for HTML. Small, because everything on this page is a number or
/// a kernel field and the escape is belt and braces.
fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// How long the supervisor has been idle, as a sentence a person reads.
fn since(seconds: f64) -> String {
    let seconds = seconds.max(0.0);
    if seconds < 90.0 {
        return format!("{seconds:.0} seconds");
    }
    let minutes = seconds / 60.0;
    if minutes < 90.0 {
        return format!("{minutes:.0} minutes");
    }
    format!("{:.1} hours", minutes / 60.0)
}

/// **The page** — prose, not fields, per `ADR-056` §3.
#[must_use]
pub fn html(reading: &Reading) -> String {
    let mut body = String::new();
    body.push_str(
        "<!doctype html><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>Supervisor</title>\
         <style>\
         :root{color-scheme:light dark}\
         body{margin:0;padding:2rem 1rem;font:16px/1.6 system-ui,sans-serif;max-width:40rem;margin-inline:auto}\
         h1{font-size:1.4rem;margin:0 0 1.5rem}\
         p{margin:0 0 1rem}\
         .verdict{font-size:1.2rem;font-weight:600;padding:1rem;border-radius:.5rem;border:2px solid}\
         .stopped{color:#a00;border-color:#a00}\
         .turning{color:#060;border-color:#060}\
         .unknown{color:#840;border-color:#840}\
         .quiet{opacity:.75;font-size:.9rem}\
         </style>",
    );
    body.push_str("<h1>Supervisor</h1>");

    match &reading.who {
        Identified::NoSupervisor => {
            body.push_str(
                "<p class=\"verdict unknown\">This page cannot identify a supervisor.</p>\
                 <p>The process serving this page was started with pid 1 as its parent, so it \
                 was not started by a supervisor at all. There is nothing here to report on, \
                 and guessing which process is the supervisor by searching for one with the \
                 right name would be a different answer to a different question.</p>",
            );
            return body;
        }
        Identified::Reparented { was } => {
            body.push_str(&format!(
                "<p class=\"verdict stopped\">The supervisor is gone.</p>\
                 <p>This program was started by pid {was}, and its parent is now pid 1 — \
                 <code>rivet-initd</code>. A process is reparented to pid 1 when its parent \
                 exits, so the supervisor that started this one has ended. That is an answer \
                 rather than a failure of this page.</p>\
                 <p>Nothing restarted it, which is the thing to carry away: the supervisor is \
                 what restarts everything else.</p>"
            ));
            return body;
        }
        Identified::Replaced { was, now } => {
            body.push_str(&format!(
                "<p class=\"verdict unknown\">The supervisor was replaced.</p>\
                 <p>This program was started by pid {was} and its parent is now pid {now}, \
                 and neither is pid 1. Something restarted the supervisor while this program \
                 kept running — a restart nobody asked this page about, and worth knowing \
                 before reading anything below.</p>"
            ));
        }
        Identified::Unreadable { pid, why } => {
            body.push_str(&format!(
                "<p class=\"verdict unknown\">The supervisor is pid {pid}, and this page \
                 cannot read it.</p>\
                 <p>Opening its <code>/proc</code> directory failed: {}. This program is \
                 confined to exactly two paths for this purpose — the supervisor's own \
                 <code>/proc</code> directory and <code>/proc/uptime</code> — so the likely \
                 cause is that the confinement was not granted on this build, rather than \
                 anything about the supervisor itself.</p>",
                escaped(why)
            ));
            return body;
        }
        Identified::Supervisor { pid, comm } => {
            body.push_str(&format!(
                "<p class=\"quiet\">Pid {pid}, which calls itself <code>{}</code>, and which \
                 is this program's own parent. That is how it was identified: the supervisor \
                 starts every service, so the parent is the supervisor, and nothing was \
                 searched for or asked.</p>",
                escaped(comm)
            ));
        }
    }

    let m = reading.moved;
    let s = &reading.second;
    let idle = reading.second.uptime - (reading.second.starttime as f64 / USER_HZ as f64);

    if m.turned() {
        body.push_str(&format!(
            "<p class=\"verdict turning\">The supervisor is turning.</p>\
             <p>In the second this page took to draw, it parked itself {}, was \
             preempted {}, and used {}. A loop that is moving is a loop that is \
             answering.</p>",
            times(m.voluntary),
            times(m.nonvoluntary),
            ticks(m.ticks),
        ));
    } else {
        body.push_str(&format!(
            "<p class=\"verdict stopped\">The supervisor has stopped.</p>\
             <p>Over the second this page took to draw it did not park itself once, was not \
             preempted once, and used no processor time at all. It has been running for {}, \
             and in all that time has used {} — a supervisor that is working moves every one \
             of those numbers constantly.</p>\
             <p>Everything it started is probably still running, and the machine may well \
             look fine from the outside. What has stopped is the thing that would notice if \
             it were not.</p>",
            since(idle),
            ticks(s.utime + s.stime),
        ));
    }

    body.push_str(&format!(
        "<p>The kernel reports it as <code>{}</code>, with {} thread{}. Its blocked-and-\
         pending signal mask is <code>{}</code> against a blocked mask of <code>{}</code>; a \
         signal that is blocked and pending is one the supervisor's <code>signalfd</code> \
         has not been read for, which is a direct witness that the loop is not draining it.</p>",
        escaped(&s.state),
        s.threads,
        if s.threads == 1 { "" } else { "s" },
        escaped(&s.pending),
        escaped(&s.blocked),
    ));

    if s.threads != 1 {
        body.push_str(
            "<p><strong>That thread count is itself a finding.</strong> The supervisor's own \
             safety argument depends on it being a single-threaded program, and a second \
             thread means something in it disagrees.</p>",
        );
    }

    body.push_str(
        "<p class=\"quiet\">This page cannot say <em>where</em> the supervisor is stopped. \
         The files that name the blocking function in one word are readable only by root, \
         and this program runs as an ordinary application so that nothing else it serves \
         can reach further than a page should. Ask the control plane for that while the \
         control plane is still answering.</p>\
         <p class=\"quiet\">Every name, number and phrase on this page may change in any \
         release. Nothing should be built to read it.</p>",
    );

    body
}

/// Ticks as a sentence, keeping the raw count visible.
///
/// **A whole clause, not a number to drop into one.** The zero case reads
/// *"no processor time"*, and an earlier version of this returned that into a
/// sentence which then appended *"of processor time"* — which is how the first
/// live render of this page said *"used no processor time of processor time"*.
/// Every branch here ends the clause it is in.
fn ticks(count: u64) -> String {
    if count == 0 {
        return "no processor time".to_string();
    }
    format!(
        "{:.2} seconds of processor time ({count} ticks)",
        count as f64 / USER_HZ as f64
    )
}

/// A count of occurrences, as a person says it.
///
/// `0` is *"not once"* rather than *"0 times"*, because the zero is the whole
/// finding on this page and a reader should meet it as a sentence.
fn times(count: u64) -> String {
    match count {
        0 => "not once".to_string(),
        1 => "once".to_string(),
        n => format!("{n} times"),
    }
}

#[cfg(test)]
#[path = "supervisor_tests.rs"]
mod tests;
