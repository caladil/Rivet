//! The fakes behind this crate's two ports.
//!
//! `#[cfg(test)]` only. Everything the board does through [`crate::kernel`] and
//! [`crate::sntp`] is scriptable here, which is the whole reason those two
//! traits exist: **`clock_settime` has never executed anywhere in this project**
//! — deliberately, because a test that succeeded would set the clock of the
//! machine running the suite — and a seventeen-minute poll interval cannot be
//! waited for.
//!
//! The shape is `cmd/rivet-sysd/src/testkernel.rs`'s, deliberately, and
//! it is smaller because this port is smaller.

use std::net::SocketAddr;

use crate::journal::Record;
use crate::kernel::{Errno, Kernel};
use crate::sntp::{QueryFailure, Sample, Sntp};

/// A machine, as this crate can see one.
pub struct FakeKernel {
    /// The monotonic clock, in milliseconds. Advanced by assignment.
    pub mono_ms: u64,
    /// The wall clock. `None` is a machine with no readable one.
    pub wall_ms: Option<u64>,
    /// What `set_wall_ms` answers. `None` means it succeeds.
    pub set_wall_results: Option<Result<(), Errno>>,
    /// Every value `set_wall_ms` was asked for, in order — **including the ones
    /// it then refused**, so a test can tell "did not try" from "tried and was
    /// refused".
    pub attempted_wall_sets: Vec<u64>,
    /// Every value the clock was actually moved to.
    pub wall_sets: Vec<u64>,
    pub records: Vec<Record>,
    pub reads: Vec<String>,
    pub sleeps: Vec<u64>,
    /// What `read_file` answers for any path.
    pub read_result: Result<String, Errno>,
}

impl Default for FakeKernel {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeKernel {
    pub fn new() -> Self {
        Self {
            mono_ms: 0,
            wall_ms: Some(0),
            set_wall_results: None,
            attempted_wall_sets: Vec::new(),
            wall_sets: Vec::new(),
            records: Vec::new(),
            reads: Vec::new(),
            sleeps: Vec::new(),
            read_result: Err(Errno::ENOENT),
        }
    }
}

impl Kernel for FakeKernel {
    fn now_ms(&mut self) -> u64 {
        self.mono_ms
    }

    fn wall_ms(&mut self) -> Option<u64> {
        self.wall_ms
    }

    fn set_wall_ms(&mut self, unix_ms: u64) -> Result<(), Errno> {
        self.attempted_wall_sets.push(unix_ms);
        if let Some(Err(e)) = self.set_wall_results {
            return Err(e);
        }
        self.wall_sets.push(unix_ms);
        self.wall_ms = Some(unix_ms);
        Ok(())
    }

    fn emit(&mut self, record: &Record) {
        self.records.push(record.clone());
    }

    fn read_file(&mut self, path: &str) -> Result<String, Errno> {
        self.reads.push(path.to_string());
        self.read_result.clone()
    }

    fn sleep_ms(&mut self, ms: u64) {
        self.sleeps.push(ms);
        // A sleep advances the monotonic clock, because on a board it does.
        // Without this the rate limit and the poll interval would be testable
        // only by moving `mono_ms` by hand, and a test that forgot to would
        // silently exercise a machine where no time passes.
        self.mono_ms = self.mono_ms.saturating_add(ms);
    }
}

/// A time server, as this crate can see one.
pub struct FakeSntp {
    /// Answers, in order. The last one repeats for ever once the script runs
    /// out, so a test that polls four times does not have to write four
    /// identical entries.
    answers: Vec<Result<Sample, QueryFailure>>,
    next: usize,
    /// Every server that was asked, in order. This is what proves the rotation
    /// and the try-until-one-answers behaviour.
    pub asked: Vec<SocketAddr>,
}

impl FakeSntp {
    pub fn always(answer: Result<Sample, QueryFailure>) -> Self {
        Self {
            answers: vec![answer],
            next: 0,
            asked: Vec::new(),
        }
    }

    pub fn scripted(answers: Vec<Result<Sample, QueryFailure>>) -> Self {
        Self {
            answers,
            next: 0,
            asked: Vec::new(),
        }
    }
}

impl Sntp for FakeSntp {
    fn query(&mut self, server: SocketAddr) -> Result<Sample, QueryFailure> {
        self.asked.push(server);
        let answer = self
            .answers
            .get(self.next)
            .or_else(|| self.answers.last())
            .cloned();
        self.next += 1;
        answer.unwrap_or(Err(QueryFailure::Unreachable {
            detail: "the fake was scripted with no answers at all".to_string(),
        }))
    }
}
