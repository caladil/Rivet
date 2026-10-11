//! Stage announcements for `cargo rivet provision` — the maintainer ruled them
//! a preference and not a condition of acceptance, as long as the run itself
//! works (.rulings/ R-174.1). Nothing in this module is on the critical path; a
//! caller with no [`crate::progress::Stages`] at all still runs correctly,
//! just quietly.
//!
//! # Why there is no spinner
//!
//! Every LONG stage this drives — a cross build, a container build — already
//! inherits its own child process's stdout, which streams live (cargo's own
//! build output, Docker's own layer progress). A spinner or a
//! carriage-return-redrawn elapsed counter would be printing to the same
//! terminal at the same time as that inherited stream, and the two would
//! interleave into garbage rather than either one reading cleanly. So this
//! prints one line when a stage starts and one when it ends, with no attempt
//! to draw over what runs in between — cheap, safe, and exactly as useful for
//! the failure case, which is the one that matters:
//! [`crate::progress::Stage::failed`] names the stage before the underlying
//! refusal is shown.
//!
//! # Why there is nothing to disable when stdout is not a terminal
//!
//! There is no redraw and no ANSI code to begin with, so an agent, a CI log,
//! or the image-file end-to-end run all see the identical plain lines a
//! person watching a real terminal does —
//! `PLAN-Feature-Set-2026-09-12` §7.2's rule against a run behaving
//! differently depending on who is reading it, applied to output as it
//! already is to input.

use std::cell::Cell;
use std::time::Instant;

/// A numbered sequence of stages, printed as `[n/total] label`.
pub struct Stages {
    total: usize,
    next: Cell<usize>,
}

impl Stages {
    #[must_use]
    pub fn new(total: usize) -> Self {
        Self { total, next: Cell::new(1) }
    }

    /// Announce the next stage starting, and hand back what closes it.
    pub fn start(&self, label: &str) -> Stage {
        let n = self.next.get();
        self.next.set(n + 1);
        println!("[{n}/{}] {label}", self.total);
        Stage { n, total: self.total, label: label.to_owned(), started: Instant::now() }
    }
}

/// One announced stage, in progress.
pub struct Stage {
    n: usize,
    total: usize,
    label: String,
    started: Instant,
}

impl Stage {
    /// The stage succeeded — prints how long it took.
    pub fn done(self) {
        println!("[{}/{}] done ({}s)", self.n, self.total, self.started.elapsed().as_secs());
    }

    /// The stage failed — names which one, before the caller prints (or
    /// returns) the refusal itself. Takes `&self` rather than consuming,
    /// because a caller under a `map_err` closure only borrows the stage it
    /// is about to report on.
    pub fn failed(&self, why: &str) {
        eprintln!("[{}/{}] FAILED: {} -- {why}", self.n, self.total, self.label);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// The numbering is sequential across calls, and independent of how long
    /// each stage actually takes.
    #[test]
    fn stages_number_themselves_in_order() {
        let stages = Stages::new(3);
        let a = stages.start("first");
        assert_eq!(a.n, 1);
        assert_eq!(a.total, 3);
        let b = stages.start("second");
        assert_eq!(b.n, 2);
        a.done();
        let c = stages.start("third");
        assert_eq!(c.n, 3);
        b.done();
        c.done();
    }
}
