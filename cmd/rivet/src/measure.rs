//! `rivet measure hash <path>` — the one measurement `ADR-029` §7.2 is waiting
//! on.
//!
//! # Why this exists, and what exactly it settles
//!
//! `ADR-029` §7.2 rules BLAKE3 the update payload's digest **"pending one
//! measurement"**, and says which measurement:
//!
//! > **One measurement settles three things**, and it has not been taken: this
//! > choice; whether reading a written slot back and hashing it is affordable
//! > (§9 step 9); and `ADR-015` §4.2's open cost, which already notes *"software
//! > hashing on a slow core, paid at every boot for every service."* Hash 640 MiB
//! > on the board with both algorithms.
//!
//! The hardware fact that makes the question interesting is `ADR-020` §6.2's, and
//! it is a reading rather than an expectation: on a Pi 4 Model B Rev 1.1,
//! `/proc/cpuinfo` gives `Features : fp asimd evtstrm crc32 cpuid` and `CPU part
//! 0xd08`. **No `sha2` feature bit.** So SHA-256 on this board is software
//! SHA-256, and the number that makes BLAKE3's number mean anything is the one
//! measured beside it on the same bytes, on the same core, in the same run.
//!
//! # ~~It is a verb~~ It is a bare word, and deliberately not a verb
//!
//! `ADR-006` §6.4's verb set is **closed** — `get set list add remove apply plan
//! export explain` — and the ADR says how it grows:
//!
//! > The verb set is part of the interface contract (section 4.1) and is extended
//! > only by amending this ADR.
//!
//! Amending an ADR is the maintainer's (`AGENTS.md` §1), so `measure` is **not**
//! added to [`rivet_core::Verb`], and `Verb::parse("measure")` still refuses.
//! It sits beside `Request::Version` and `Request::Schema` in [`crate::cli`],
//! following the precedent `schema` already set there — see that module for the
//! reading of §6.4 and §7 that makes a bare word consistent with a closed verb
//! set.
//!
//! **The precedent is followed, not stretched, and the difference is worth
//! stating.** `schema` is §7's *surface describing itself*. This is neither that
//! nor an administrative command: it does nothing to the machine, reads no
//! configuration, and answers a question about the **hardware**. It is an
//! instrument, and the honest description of its scope is that it exists to close
//! `ADR-029` §7.2 and `ADR-015` §4.2. Three outcomes are all legitimate once the
//! number is in: it becomes a verb by amendment, it stays a bare word, or it is
//! deleted along with `sha2`. That is the maintainer's call and this file does not
//! pre-empt it.
//!
//! # Why it cannot be a separate binary
//!
//! `cmd/rivet` **is** the login shell, and
//! [`crate::invocation::self_invocation`] refuses any `-c` string naming a program
//! that is not this one. So asking the machine to run `/system/bin/anything-else` is refused by
//! design, there is no `scp` to deliver a benchmark with (`ADR-029` §7.2's own
//! note), and a measurement that cannot be reached remotely cannot be taken at
//! all. Reachability through `rivet` is not a convenience here; it is the only
//! route onto the board.
//!
//! # How it measures, and what each number does and does not include
//!
//! One pass over the device, a fixed [`BUFFER_BYTES`] buffer, and **three
//! separately accumulated durations**: the time in `read(2)`, the time in
//! BLAKE3's `update`, and the time in SHA-256's `update`.
//!
//! Timing each algorithm's own work rather than a whole pass each is the
//! difference between a measurement and a trap. Two sequential passes over 640 MiB
//! on a 4 GB board would put the second algorithm's pass partly in the page cache
//! and hand it a throughput the first could not have had — an artefact that looks
//! exactly like the answer this is being run to get. Interleaving over one read
//! removes it: both algorithms see the same bytes, in the same order, with the
//! same cache state, and neither is charged for the card.
//!
//! **So `throughput-mb-s` is a hashing rate, not an end-to-end rate.** The card is
//! reported separately as `read-elapsed-ms`, and the total as `elapsed-ms`, so an
//! end-to-end figure is recoverable and is never implied by a number that is not
//! one. `ADR-015` §4.2's cost — "paid at every boot for every service" — is the
//! hashing rate; `ADR-029` §9's read-back cost is the total.
//!
//! **MB means 10^6 bytes**, not 2^20, so the figure is comparable with every
//! published hash benchmark. `bytes` and `elapsed-us` are both reported, so
//! anyone who prefers MiB/s can compute it rather than believe this.
//!
//! # What has not been established here
//!
//! `AGENTS.md` §6. **No number this produces on a development host says anything
//! about a Cortex-A72**, and the reason is stronger than "different hardware":
//! it is not even the same implementation. Read out of `blake3`'s build script,
//! an x86-64 host selects `blake3_{sse2,sse41,avx2}_rust` — SSE2, SSE4.1 and AVX2
//! through `std::arch` — while `aarch64-unknown-linux-musl` selects **nothing at
//! all** and runs the portable code. A dev-host run proves this command works; it
//! is not a preview of the answer, and the ratio between the two algorithms need
//! not survive the move either.
//!
//! **The comparison is worse on a dev host than the algorithms are.** Read from
//! this development host's `/proc/cpuinfo` on 2026-08-23: an Intel Core i7-14700KF
//! reporting **both `avx2` and `sha_ni`**. So a run here pits AVX2 BLAKE3 against
//! *hardware* SHA-256, which is the exact configuration `ADR-020` §6.2 established
//! the board does **not** have. The ratio measured on this machine is therefore
//! not merely imprecise about the board — it is biased in SHA-256's favour, and
//! the board's ratio should be expected to be larger rather than similar.
//!
//! **And the board's BLAKE3 figure is the floor rather than the ceiling.**
//! `blake3`'s NEON backend is C (`c/blake3_neon.c`) and taking it would need the
//! cross-toolchain `ADR-001` §4.4 rules out — see this crate's `Cargo.toml`, where
//! the exact line of `build.rs` that decides it is quoted.

use std::io::Read;

use sha2::Digest as _;

use crate::refusal::{Code, Refusal};
use crate::value::Value;

/// The word this command is spelled with. Not a verb — see the module docs.
pub const MEASURE: &str = "measure";

/// **This module's entry in `ADR-032` §4.2's internal-handler table.**
///
/// Declared here rather than in `crate::handler`, and that is the same rule
/// `rivet_core::operation::CATALOGUE` follows: *"Each entry is the `const
/// OPERATION` declared by the module that **implements** that operation."* A
/// description kept beside the table instead of beside the code is the thing
/// `ADR-006` §7 says will be wrong within a month.
pub const HANDLER: crate::handler::Handler = crate::handler::Handler {
    name: MEASURE,
    summary: "Measure this machine's hashing throughput, for the choice ADR-029 §7.2 left open.",
    takes: "`hash <absolute path>` — the subject, then a file to read. There is one subject.",
    why_not_a_verb: "It asks the machine to do something to nothing: no configuration is read, \
                     no state is changed, and there is no noun in ADR-007's schema it could take \
                     (ADR-006 §6.4 pairs the closed verb set with nouns that ARE schema paths). \
                     Whether it belongs in §6.4's set, stays here, or is deleted once the number \
                     is known is the maintainer's, and this word appears nowhere in ADR-006.",
};

/// The one subject there is. `ADR-006` §6.4 pairs a verb with a noun; this is a
/// bare word with a subject, and the shape is deliberately the same one
/// `rivet apply trial-boot <slot>` uses, so an operator who has typed that can
/// predict this.
pub const HASH: &str = "hash";

/// The read size. **Fixed, and never the file's size.**
///
/// The operator will point this at a 640 MiB `/system` partition on a board with
/// 4 GB of RAM and no swap; reading it into memory would work, would be measuring
/// something else, and would not work on the first larger thing anyone tried. One
/// MiB is large enough that the per-call overhead of `read(2)` is not what is
/// being timed and small enough to stay in cache while both algorithms consume it.
pub const BUFFER_BYTES: usize = 1024 * 1024;

/// The algorithm `ADR-029` §7.2 rules, and the one this run exists to confirm.
pub const BLAKE3: &str = "blake3";

/// The comparison, and **the reason it is here is the reason it can leave.**
///
/// `ADR-020` §6.2 established on the board that this A72 has no `sha2` feature
/// bit, so SHA-256 here is software SHA-256. On its own that is a number about
/// nothing; beside BLAKE3's, on the same bytes in the same pass, it is the ratio
/// that makes BLAKE3's number a decision. Once the ratio is known this name and
/// the `sha2` dependency go together — see this crate's `Cargo.toml`.
pub const SHA256: &str = "sha256";

// ---------------------------------------------------------------------------
// The clock
// ---------------------------------------------------------------------------

/// A monotonic clock, as a bare nanosecond count.
///
/// A **seam of this crate's own**, in `AGENTS.md` §3.1's second sense: it exists
/// so the measurement can be driven from a test on a development host, not so the
/// domain can express something. `rivet-core` never sees it and could not name
/// it — the thing behind it is `clock_gettime(2)`.
pub trait Monotonic {
    /// Nanoseconds since an unspecified fixed point. Differences of this are the
    /// only meaningful quantity.
    fn now_nanos(&mut self) -> Result<u128, Refusal>;
}

/// `CLOCK_MONOTONIC`, via [`crate::syscall`]. `ADR-021` §5.
#[cfg(target_os = "linux")]
pub struct SyscallClock;

#[cfg(target_os = "linux")]
impl Monotonic for SyscallClock {
    fn now_nanos(&mut self) -> Result<u128, Refusal> {
        crate::syscall::monotonic_nanos().map_err(|errno| {
            Refusal::new(
                Code::MACHINE_REFUSED,
                format!(
                    "clock_gettime(CLOCK_MONOTONIC) failed with errno {errno}. Without a clock \
                     there is no measurement, and reporting an untimed digest would be a result \
                     that answers a different question (ADR-021 §5)."
                ),
            )
        })
    }
}

/// A clock this build does not have, which **refuses** rather than substituting
/// one that is not monotonic.
///
/// `AGENTS.md` §3: *"Stubs refuse; they never fake success."* The development host
/// is Windows (`AGENTS.md` §10.5) and `clock_gettime(2)` is Linux's. Falling back
/// to a wall clock here would produce a plausible-looking benchmark whose
/// durations `ADR-021` §5 says cannot be trusted — the failure mode `AGENTS.md` §5
/// names, where a wrong guess *succeeds*.
#[cfg(any(test, not(target_os = "linux")))]
pub struct NoClock {
    platform: &'static str,
}

#[cfg(any(test, not(target_os = "linux")))]
impl NoClock {
    pub const fn new(platform: &'static str) -> Self {
        Self { platform }
    }
}

#[cfg(any(test, not(target_os = "linux")))]
impl Monotonic for NoClock {
    fn now_nanos(&mut self) -> Result<u128, Refusal> {
        Err(Refusal::new(
            Code::MACHINE_REFUSED,
            format!(
                "this rivet was built for {}, which has no clock_gettime(CLOCK_MONOTONIC). \
                 ADR-021 §5 requires the monotonic clock for every duration, and this build has \
                 no way to read one. Run this on the board — which is where the number means \
                 anything anyway (ADR-029 §7.2).",
                self.platform
            ),
        ))
    }
}

/// The clock this build can reach, whatever that is.
///
/// The one place the platform is chosen, exactly as [`crate::boot::machine`] is
/// for the boot medium.
pub fn clock() -> Box<dyn Monotonic> {
    #[cfg(target_os = "linux")]
    {
        Box::new(SyscallClock)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Box::new(NoClock::new(std::env::consts::OS))
    }
}

// ---------------------------------------------------------------------------
// The command
// ---------------------------------------------------------------------------

/// Bind the words after `measure` and run it.
///
/// The composition root's half is two lines in `crate::run`; everything a test
/// needs to reach is below [`stream`], which takes its bytes and its clock.
pub fn perform(rest: &[String]) -> Result<Value, Refusal> {
    let (subject, target) = bind(rest)?;
    if subject != HASH {
        return Err(Refusal::about(
            Code::UNKNOWN_SUBJECT,
            subject,
            format!(
                "measure has one subject and it is `{HASH}`. \
                 Usage: rivet measure {HASH} <path>."
            ),
        ));
    }

    // A named block device or a file, opened the same way, because on Linux they
    // are the same call. `/dev/mmcblk0p3` is what ADR-029 §7.2 asks for and a
    // file is what makes the command testable anywhere.
    let mut file = std::fs::File::open(target).map_err(|error| {
        Refusal::about(
            Code::MACHINE_REFUSED,
            target,
            // What the system said, verbatim (`AGENTS.md` §5). The errno text is
            // the difference between "you typed the wrong path" and "you are not
            // root", and paraphrasing it would lose that.
            format!("cannot open it: {error}"),
        )
    })?;

    let mut clock = clock();
    stream(target, &mut file, clock.as_mut())
}

/// Split the words after `measure` into a subject and a target.
fn bind(rest: &[String]) -> Result<(&str, &str), Refusal> {
    match rest {
        [] => Err(Refusal::new(
            Code::ARGUMENTS,
            format!("measure needs a subject and a path. Usage: rivet measure {HASH} <path>."),
        )),
        [subject] => Err(Refusal::about(
            Code::ARGUMENTS,
            subject,
            format!("measure {HASH} needs a path — a block device or a file to hash."),
        )),
        [subject, target] => Ok((subject.as_str(), target.as_str())),
        // Extra words are refused rather than ignored. A path with a space in it
        // cannot arrive here anyway: `crate::invocation::words` refuses a quoted
        // `-c` string outright rather than tokenising it, so silently dropping
        // the tail would hide that refusal one layer down.
        [_, _, extra, ..] => Err(Refusal::about(
            Code::ARGUMENTS,
            extra,
            format!("measure takes exactly two words: `{HASH}` and one path."),
        )),
    }
}

/// One pass over `reader`, hashing with both algorithms and timing each.
///
/// Separated from [`perform`] so it can be driven over any `Read` with any clock,
/// which is how every test below runs on a development host.
pub fn stream<R: Read>(
    target: &str,
    reader: &mut R,
    clock: &mut dyn Monotonic,
) -> Result<Value, Refusal> {
    let mut blake = blake3::Hasher::new();
    let mut sha = sha2::Sha256::new();

    let mut buffer = vec![0u8; BUFFER_BYTES];
    let mut bytes: u64 = 0;
    let mut read_nanos: u128 = 0;
    let mut blake_nanos: u128 = 0;
    let mut sha_nanos: u128 = 0;

    let start = clock.now_nanos()?;
    loop {
        let before_read = clock.now_nanos()?;
        let read = match reader.read(&mut buffer) {
            Ok(read) => read,
            // A signal interrupted the read and nothing was lost. Retrying is
            // what every correct reader does; treating it as an error would fail
            // a measurement for a reason that has nothing to do with the card.
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(Refusal::about(
                    Code::MACHINE_REFUSED,
                    target,
                    format!("read failed after {bytes} bytes: {error}"),
                ));
            }
        };
        read_nanos = read_nanos.saturating_add(clock.now_nanos()? - before_read);
        if read == 0 {
            break;
        }
        let chunk = buffer.get(..read).unwrap_or(&[]);

        let before_blake = clock.now_nanos()?;
        blake.update(chunk);
        let after_blake = clock.now_nanos()?;
        sha.update(chunk);
        let after_sha = clock.now_nanos()?;

        blake_nanos = blake_nanos.saturating_add(after_blake - before_blake);
        sha_nanos = sha_nanos.saturating_add(after_sha - after_blake);
        bytes = bytes.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
    }
    let total_nanos = clock.now_nanos()? - start;

    Ok(Value::object(vec![
        ("target", Value::str(target)),
        ("bytes", integer(u128::from(bytes))),
        (
            "buffer-bytes",
            integer(u128::try_from(BUFFER_BYTES).unwrap_or(0)),
        ),
        ("read-elapsed-ms", integer(read_nanos / 1_000_000)),
        ("elapsed-ms", integer(total_nanos / 1_000_000)),
        (
            "algorithms",
            Value::List(vec![
                report(BLAKE3, bytes, blake_nanos, hex(blake.finalize().as_bytes())),
                report(SHA256, bytes, sha_nanos, hex(&sha.finalize())),
            ]),
        ),
    ]))
}

/// One algorithm's row.
fn report(algorithm: &'static str, bytes: u64, nanos: u128, digest: String) -> Value {
    Value::object(vec![
        ("algorithm", Value::str(algorithm)),
        ("elapsed-ms", integer(nanos / 1_000_000)),
        // The microsecond figure is not decoration. `Value` has no float
        // (`rivet_core::value`), so `throughput-mb-s` is rounded to a whole
        // number; `bytes` and this together let a reader recompute it exactly,
        // in whatever unit they prefer. Reporting a rounded rate as the only
        // evidence would be a measurement nobody can check.
        ("elapsed-us", integer(nanos / 1_000)),
        (
            "throughput-mb-s",
            integer(megabytes_per_second(bytes, nanos)),
        ),
        ("digest", Value::str(digest)),
    ])
}

/// Megabytes per second, where a megabyte is 10^6 bytes.
///
/// Integer arithmetic in `u128` throughout: `bytes * 10^9 / nanos` is the rate in
/// bytes per second and dividing by 10^6 gives MB/s, so the whole thing is
/// `bytes * 1000 / nanos` with a half added for rounding. 640 MiB times a thousand
/// is nowhere near `u128`'s ceiling, so nothing here can overflow.
///
/// **Zero elapsed gives zero**, which is visibly wrong rather than infinite. It
/// cannot happen against a real device and can against a fake clock, and a
/// division by zero in the login shell would be a panic (`AGENTS.md` §3).
fn megabytes_per_second(bytes: u64, nanos: u128) -> u128 {
    if nanos == 0 {
        return 0;
    }
    (u128::from(bytes) * 1_000 + nanos / 2) / nanos
}

/// A count as the only integer [`Value`] there is.
///
/// Saturating rather than wrapping: a number too large to render is reported as
/// the largest one there is, which is obviously an artefact. A wrapped one would
/// look like a plausible measurement, which is `AGENTS.md` §5's whole complaint
/// about guesses — they succeed.
fn integer(n: u128) -> Value {
    Value::Integer(i64::try_from(n).unwrap_or(i64::MAX))
}

/// Lowercase hex, written out rather than reached for.
///
/// Neither `blake3` nor `sha2` is asked to format its own digest: `blake3`'s
/// `to_hex` is behind the `std` feature this crate deliberately does not enable,
/// and two crates formatting two digests is two chances for them to disagree
/// about case or separator in one document.
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let hi = usize::from(byte >> 4);
        let lo = usize::from(byte & 0x0f);
        out.push(char::from(DIGITS.get(hi).copied().unwrap_or(b'?')));
        out.push(char::from(DIGITS.get(lo).copied().unwrap_or(b'?')));
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use rivet_core::Verb;

    /// A clock that advances a fixed amount on every read.
    ///
    /// Deterministic on purpose: a test that asserted a real duration would be
    /// asserting how fast the machine running it is.
    struct FakeClock {
        now: u128,
        /// The gap added by each successive reading, cycled.
        ///
        /// **A single fixed step is not enough and this test file proved it.**
        /// With one step every interval between two consecutive readings is the
        /// same length, so a defect that charged SHA-256's time to BLAKE3's
        /// accumulator produced *identical* numbers and
        /// `each_algorithm_is_timed_on_its_own_work…` passed. Injecting exactly
        /// that defect is how it was found (`AGENTS.md` §4 — a regression test
        /// that cannot fail proves nothing). A cycle whose length is coprime with
        /// nothing in particular gives every interval a distinguishable width.
        steps: Vec<u128>,
        reading: usize,
    }

    impl FakeClock {
        fn new(step: u128) -> Self {
            Self::cycling(&[step])
        }

        fn cycling(steps: &[u128]) -> Self {
            Self {
                now: 0,
                steps: steps.to_vec(),
                reading: 0,
            }
        }
    }

    impl Monotonic for FakeClock {
        fn now_nanos(&mut self) -> Result<u128, Refusal> {
            let step = self
                .steps
                .get(self.reading % self.steps.len().max(1))
                .copied()
                .unwrap_or(0);
            self.reading += 1;
            self.now += step;
            Ok(self.now)
        }
    }

    /// A clock that refuses, which is what a non-Linux build has.
    struct BrokenClock;

    impl Monotonic for BrokenClock {
        fn now_nanos(&mut self) -> Result<u128, Refusal> {
            Err(Refusal::new(Code::MACHINE_REFUSED, "no clock here"))
        }
    }

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|w| (*w).to_string()).collect()
    }

    fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
        match value {
            Value::Object(fields) => fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn algorithm<'a>(value: &'a Value, name: &str) -> &'a Value {
        let Some(Value::List(rows)) = field(value, "algorithms") else {
            panic!("no algorithms list: {value:?}");
        };
        let found = rows
            .iter()
            .find(|row| field(row, "algorithm") == Some(&Value::str(name)));
        match found {
            Some(row) => row,
            None => panic!("{name} is not in the report: {value:?}"),
        }
    }

    // -- the vocabulary --------------------------------------------------------

    /// **`ADR-006` §6.4's closed set is not quietly extended by this command.**
    ///
    /// The same assertion `cli`'s `schema_is_not_a_verb_and_the_core_still_
    /// refuses_it_as_one` makes, for the same reason: recognising a word in this
    /// binary does not put it in the verb set, which is extended only by amending
    /// the ADR — and that is the maintainer's, not an agent's.
    #[test]
    fn measure_is_not_a_verb_and_the_core_still_refuses_it_as_one() {
        assert!(Verb::parse(MEASURE).is_err());
        assert!(!Verb::ALL.iter().any(|v| v.as_str() == MEASURE));
        // And its subject is not one either — `hash` is a noun this command
        // takes, not a word the vocabulary knows.
        assert!(Verb::parse(HASH).is_err());
    }

    // -- binding ---------------------------------------------------------------

    #[test]
    fn the_two_words_are_a_subject_and_a_path() {
        assert_eq!(bind(&words(&["hash", "/dev/x"])), Ok(("hash", "/dev/x")));
    }

    #[test]
    fn a_missing_subject_or_path_is_an_argument_error_naming_what_is_missing() {
        for (given, expected) in [
            (vec![], Code::ARGUMENTS),
            (words(&["hash"]), Code::ARGUMENTS),
            (words(&["hash", "/dev/x", "extra"]), Code::ARGUMENTS),
        ] {
            let Err(refusal) = bind(&given) else {
                panic!("{given:?} was accepted");
            };
            assert_eq!(refusal.code, expected, "{given:?}");
            assert!(
                refusal.message.contains(HASH),
                "{given:?} did not say what to type: {}",
                refusal.message
            );
        }
    }

    /// A subject that is not `hash` is `E_UNKNOWN_SUBJECT`, not `E_ARGUMENTS`.
    ///
    /// `ADR-006` §4.1's distinction, and the same one `rivet apply config b`
    /// already makes: the request was well-formed and names something this
    /// command has nothing to say about. An agent that gets `E_ARGUMENTS` retries
    /// with different words; one that gets `E_UNKNOWN_SUBJECT` stops.
    #[test]
    fn a_subject_that_is_not_hash_is_refused_as_a_subject_rather_than_as_arguments() {
        let Err(refusal) = perform(&words(&["throughput", "/dev/x"])) else {
            panic!("an invented subject was accepted");
        };
        assert_eq!(refusal.code, Code::UNKNOWN_SUBJECT);
        assert_eq!(refusal.subject.as_deref(), Some("throughput"));
    }

    /// A path that cannot be opened refuses with the machine's own words.
    ///
    /// `E_MACHINE_REFUSED` is `rivet-core`'s existing code for *"the machine
    /// could not be read … The message is what the system said"*, which is
    /// exactly this failure. No new code was invented for it (`ADR-006` §4.1 —
    /// codes are part of the interface contract).
    #[test]
    fn a_path_that_cannot_be_opened_refuses_with_the_errno_the_system_gave() {
        let Err(refusal) = perform(&words(&[HASH, "/rivet-no-such-device"])) else {
            panic!("a path that does not exist was hashed");
        };
        assert_eq!(refusal.code, Code::MACHINE_REFUSED);
        assert_eq!(refusal.subject.as_deref(), Some("/rivet-no-such-device"));
        assert!(
            refusal.message.contains("cannot open it"),
            "{}",
            refusal.message
        );
        // Exit 1: the operation refused, rather than 2, which would say the
        // arguments were unusable. They were fine; the path is not there.
        assert_eq!(refusal.exit_code(), 1);
    }

    // -- the digests -----------------------------------------------------------

    /// **The vectors are the published ones, and their provenance is the point.**
    ///
    /// `AGENTS.md` §4: *"A test that round-trips a reader against its own writer
    /// proves the two agree with each other and nothing about the format."* A
    /// digest this code produced and this test then asserted would prove only
    /// that `blake3::Hasher` was called. These are the values every other
    /// implementation of these two algorithms produces for the empty input and
    /// for `abc` — SHA-256's from FIPS 180-4's own example, BLAKE3's from the
    /// reference implementation's test vectors.
    #[test]
    fn both_algorithms_agree_with_the_published_vectors_for_the_empty_input() {
        let mut clock = FakeClock::new(1);
        let report = stream("empty", &mut [].as_slice(), &mut clock).expect("a clock and no bytes");
        assert_eq!(field(&report, "bytes"), Some(&Value::Integer(0)));
        assert_eq!(
            field(algorithm(&report, "sha256"), "digest"),
            Some(&Value::str(
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
            ))
        );
        assert_eq!(
            field(algorithm(&report, "blake3"), "digest"),
            Some(&Value::str(
                "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
            ))
        );
    }

    #[test]
    fn both_algorithms_agree_with_the_published_vectors_for_abc() {
        let mut clock = FakeClock::new(1);
        let report = stream("abc", &mut b"abc".as_slice(), &mut clock).expect("a clock and bytes");
        assert_eq!(field(&report, "bytes"), Some(&Value::Integer(3)));
        assert_eq!(
            field(algorithm(&report, "sha256"), "digest"),
            Some(&Value::str(
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            ))
        );
        assert_eq!(
            field(algorithm(&report, "blake3"), "digest"),
            Some(&Value::str(
                "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
            ))
        );
    }

    /// **The buffer is a boundary and the digest must not know about it.**
    ///
    /// The whole point of streaming is that a 640 MiB device produces the digest
    /// of its 640 MiB, not the digest of its last chunk. This feeds more than one
    /// buffer's worth through and checks the answer against a single-shot hash of
    /// the same bytes — the one comparison that can catch a reset, a dropped
    /// chunk, or a hasher rebuilt inside the loop.
    #[test]
    fn a_stream_longer_than_the_buffer_hashes_the_whole_of_it_and_not_the_last_chunk() {
        let bytes: Vec<u8> = (0..(BUFFER_BYTES * 2 + 12345))
            .map(|i| u8::try_from(i % 251).unwrap_or(0))
            .collect();
        let mut clock = FakeClock::new(1);
        let report = stream("long", &mut bytes.as_slice(), &mut clock).expect("bytes and a clock");

        assert_eq!(
            field(&report, "bytes"),
            Some(&Value::Integer(i64::try_from(bytes.len()).unwrap()))
        );
        assert_eq!(
            field(algorithm(&report, "blake3"), "digest"),
            Some(&Value::str(hex(blake3::hash(&bytes).as_bytes())))
        );
        assert_eq!(
            field(algorithm(&report, "sha256"), "digest"),
            Some(&Value::str(hex(&sha2::Sha256::digest(&bytes))))
        );
    }

    /// A reader that hands back one byte at a time — which is what a short
    /// `read(2)` on a real device looks like — must produce the same digest as one
    /// that hands back everything at once.
    #[test]
    fn short_reads_do_not_change_the_answer() {
        struct OneByteAtATime<'a>(&'a [u8]);
        impl Read for OneByteAtATime<'_> {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                match (self.0.first(), out.first_mut()) {
                    (Some(byte), Some(slot)) => {
                        *slot = *byte;
                        self.0 = self.0.get(1..).unwrap_or(&[]);
                        Ok(1)
                    }
                    _ => Ok(0),
                }
            }
        }

        let bytes = b"the quick brown fox jumps over the lazy dog";
        let mut whole = FakeClock::new(1);
        let mut dribbled = FakeClock::new(1);
        let a = stream("whole", &mut bytes.as_slice(), &mut whole).expect("a report");
        let b = stream("dribbled", &mut OneByteAtATime(bytes), &mut dribbled).expect("a report");
        assert_eq!(
            field(algorithm(&a, "blake3"), "digest"),
            field(algorithm(&b, "blake3"), "digest")
        );
        assert_eq!(
            field(algorithm(&a, "sha256"), "digest"),
            field(algorithm(&b, "sha256"), "digest")
        );
        assert_eq!(field(&a, "bytes"), field(&b, "bytes"));
    }

    // -- the numbers -----------------------------------------------------------

    /// The report carries everything `ADR-029` §7.2 asked for, for **both**
    /// algorithms. A benchmark missing one of the two settles nothing: §7.2's
    /// argument is comparative.
    #[test]
    fn every_field_the_ruling_asked_for_is_present_for_both_algorithms() {
        let mut clock = FakeClock::new(1_000);
        let report = stream("/dev/x", &mut vec![7u8; 4096].as_slice(), &mut clock)
            .expect("bytes and a clock");
        for key in [
            "target",
            "bytes",
            "buffer-bytes",
            "read-elapsed-ms",
            "elapsed-ms",
        ] {
            assert!(
                field(&report, key).is_some(),
                "{key} is missing: {report:?}"
            );
        }
        for name in [BLAKE3, SHA256] {
            let row = algorithm(&report, name);
            for key in ["elapsed-ms", "elapsed-us", "throughput-mb-s", "digest"] {
                assert!(field(row, key).is_some(), "{name} has no {key}: {row:?}");
            }
        }
    }

    /// **The timings are separate accumulations, not one duration reported
    /// twice.** If the two algorithms ever share a counter, this is what fails —
    /// and a benchmark reporting identical times for two different algorithms
    /// would look like a finding rather than a defect.
    #[test]
    fn each_algorithm_is_timed_on_its_own_work_and_the_card_is_timed_separately() {
        // **The gaps are 1 ms, 2 ms, 3 ms, cycling**, so no two adjacent
        // intervals are the same width and every accumulator gets a value only
        // its own arithmetic can produce. See [`FakeClock::steps`] for the defect
        // a single fixed step failed to catch.
        //
        // Nine readings for sixteen bytes, in this order, with the running total
        // in milliseconds beside each:
        //
        //   1 start            +1 =  1
        //   2 before_read      +2 =  3
        //   3 after_read       +3 =  6      read: 3
        //   4 before_blake     +1 =  7
        //   5 after_blake      +2 =  9      blake: 2
        //   6 after_sha        +3 = 12      sha:   3
        //   7 before_read      +1 = 13
        //   8 after_read       +2 = 15      read: 2   (the zero-length read)
        //   9 end              +3 = 18      total: 17
        let mut clock = FakeClock::cycling(&[1_000_000, 2_000_000, 3_000_000]);
        let report = stream("/dev/x", &mut vec![0u8; 16].as_slice(), &mut clock).expect("a report");

        let blake = field(algorithm(&report, BLAKE3), "elapsed-us");
        let sha = field(algorithm(&report, SHA256), "elapsed-us");
        assert_eq!(blake, Some(&Value::Integer(2_000)), "{report:?}");
        assert_eq!(sha, Some(&Value::Integer(3_000)), "{report:?}");
        assert_ne!(
            blake, sha,
            "the two algorithms reported the same duration; a shared accumulator \
             would look exactly like this and would look like a finding"
        );
        // The card, over both reads including the final zero-length one.
        assert_eq!(field(&report, "read-elapsed-ms"), Some(&Value::Integer(5)));
        // And the total spans every one of them, so it is never smaller than its
        // parts: 3 + 2 + 3 + 5 = 13 accounted for, out of 17.
        assert_eq!(field(&report, "elapsed-ms"), Some(&Value::Integer(17)));
    }

    /// The throughput is the one derived number, and it is derived from the two
    /// reported ones. A reader who does the arithmetic must get the same answer,
    /// or the rounded figure is the only evidence and cannot be checked.
    #[test]
    fn the_throughput_is_what_the_reported_bytes_and_microseconds_give() {
        // 10^9 bytes in 10^9 nanoseconds is one gigabyte per second: 1000 MB/s.
        assert_eq!(megabytes_per_second(1_000_000_000, 1_000_000_000), 1_000);
        // 640 MiB in two seconds.
        assert_eq!(megabytes_per_second(671_088_640, 2_000_000_000), 336);
        // Rounds rather than truncates: 1.5 MB/s is 2, not 1.
        assert_eq!(megabytes_per_second(1_500_000, 1_000_000_000), 2);
    }

    /// A zero duration reports zero rather than dividing by it. This is the
    /// login shell and a panic here is a session that dies
    /// with no message.
    #[test]
    fn a_zero_duration_reports_zero_rather_than_panicking() {
        assert_eq!(megabytes_per_second(1_000_000, 0), 0);
        let mut clock = FakeClock::new(0);
        let report = stream("/dev/x", &mut b"abc".as_slice(), &mut clock).expect("a report");
        assert_eq!(
            field(algorithm(&report, "blake3"), "throughput-mb-s"),
            Some(&Value::Integer(0))
        );
    }

    // -- refusing rather than faking ------------------------------------------

    /// **A clock that will not answer produces no measurement at all.**
    ///
    /// `AGENTS.md` §3: an unsupported platform refuses; it never returns a
    /// result. The digests would still be computable without a clock, and
    /// reporting them with a zero duration would be a value that looks like a
    /// benchmark and is not one.
    #[test]
    fn without_a_monotonic_clock_there_is_no_report_at_all() {
        let Err(refusal) = stream("/dev/x", &mut b"abc".as_slice(), &mut BrokenClock) else {
            panic!("a measurement was produced with no clock");
        };
        assert_eq!(refusal.code, Code::MACHINE_REFUSED);
    }

    /// The non-Linux clock says which platform it is and where to run this
    /// instead, because the operator's next question is why their laptop cannot
    /// do it.
    #[test]
    fn the_platform_without_a_monotonic_clock_says_so_and_says_where_to_run_it() {
        let Err(refusal) = NoClock::new("windows").now_nanos() else {
            panic!("a build with no clock produced a time");
        };
        assert_eq!(refusal.code, Code::MACHINE_REFUSED);
        assert!(refusal.message.contains("windows"), "{}", refusal.message);
        assert!(refusal.message.contains("ADR-021"), "{}", refusal.message);
    }

    // -- hex -------------------------------------------------------------------

    #[test]
    fn hex_is_lowercase_and_two_characters_per_byte() {
        assert_eq!(hex(&[0x00, 0x0f, 0xf0, 0xff]), "000ff0ff");
        assert_eq!(hex(&[]), "");
        assert_eq!(hex(&[0xde, 0xad, 0xbe, 0xef]), "deadbeef");
    }

    /// Both digests are 32 bytes, so both render as 64 characters. A digest of
    /// another length would mean a different algorithm was configured.
    #[test]
    fn both_digests_are_sixty_four_hex_characters() {
        let mut clock = FakeClock::new(1);
        let report = stream("x", &mut b"x".as_slice(), &mut clock).expect("a report");
        for name in [BLAKE3, SHA256] {
            let Some(Value::Str(digest)) = field(algorithm(&report, name), "digest") else {
                panic!("{name} produced no digest");
            };
            assert_eq!(digest.len(), 64, "{name} digest is {digest}");
            assert!(
                digest
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
                "{name} digest is not lowercase hex: {digest}"
            );
        }
    }
}
