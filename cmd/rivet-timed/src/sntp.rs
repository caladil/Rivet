//! The other I/O seam: one SNTP round trip, and the wall around `rsntp`.
//!
//! `ADR-009` §3.4 is the rule this module implements — *"the cost of a
//! dependency is paid at its boundary, not in its middle"* — and it is applied
//! literally: **no `rsntp` type appears outside this file.** What crosses the
//! boundary is [`Sample`], which is four numbers, and [`QueryFailure`], which is
//! a closed set of reasons carrying already-rendered text. There is a test that
//! says so.
//!
//! `ADR-009` §3's ladder puts this on **rung 1** without argument: SNTP is a
//! protocol implementation, which is the left-hand column of §3.4's table and
//! the thing §3.3 says a bespoke version of would be *"not merely slower to
//! write but worse, in ways that surface as intermittent failures years
//! later"*.
//!
//! # Why the port is separate from [`crate::kernel::Kernel`]
//!
//! `cmd/rivet-sysd/src/kernel.rs` argues at length that its `/etc` and
//! crash-safe-write methods belong on **one** port, because the code performing
//! them has to journal what it did and two `&mut dyn` borrows of one adapter is
//! not a thing Rust permits. That argument does not reach here: the SNTP client
//! and the clock are **different objects**, so `run(&mut dyn Kernel, &mut dyn
//! Sntp)` borrows two things rather than one thing twice.
//!
//! And keeping them apart buys something specific. A fake `Sntp` answers in
//! *protocol* values — an offset, a stratum, a kiss-o'-death — while a fake
//! `Kernel` answers in `Errno`. Merging them would put a vocabulary on the port
//! that most of its methods have nothing to do with.
//!
//! # What this module does NOT do, and it is the load-bearing half
//!
//! **It does not read the clock, and it does not set it.** `rsntp` offers
//! `SynchronizationResult::datetime()`, which would hand back a corrected wall
//! time directly — and it is deliberately not used, because that method calls
//! `SystemTime::now()` **inside the library**, where no fake can reach it and
//! no test can observe it. [`Sample::offset_ms`] comes from `clock_offset()`
//! instead, which `rsntp` derives from the four timestamps in the packet
//! exchange and nothing else.
//!
//! The consequence is the shape the rest of the crate has: the *decision* about
//! what the clock should become is [`crate::discipline`], which is pure, and the
//! clock is read and written through [`crate::kernel::Kernel`], which is faked.
//! That is `AGENTS.md` §3.1's seam doing its job — see also
//! `rivet_store::clock`, which makes exactly this split for `ADR-021` §3's
//! restore.

use std::net::SocketAddr;

/// One usable answer from a time server.
///
/// Deliberately not `rsntp::SynchronizationResult`: that type carries a
/// `ReferenceIdentifier`, a `LeapIndicator` and `SntpDuration`s, none of which
/// anything above this module knows what to do with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    /// How far this machine's wall clock is **behind** the server, in
    /// milliseconds. Negative means the machine is ahead.
    ///
    /// It is an offset rather than a time on purpose: an offset is a
    /// *measurement*, and turning it into a target wall clock is a decision
    /// that needs the machine's own clock, which is [`crate::kernel::Kernel`]'s
    /// to supply and [`crate::discipline`]'s to use.
    pub offset_ms: i64,
    /// The round trip, in milliseconds, rounded to an integer.
    ///
    /// **Reported on every sample and acted on by nothing**, and that is the
    /// point: it is the number that says whether
    /// [`crate::discipline::NOISE_FLOOR_MS`] is well chosen for **this machine's
    /// actual network**. That constant has now been checked against four real
    /// exchanges from the development host (20–54 ms; the figures are on
    /// `NOISE_FLOOR_MS`) and against **no board at all**. Putting the
    /// measurement in the journal is how the reasoning gets checked where it
    /// matters instead of being believed.
    pub round_trip_ms: u64,
    /// The server's distance from a reference clock. 1 is a server with its own
    /// hardware reference; 2 is one synchronised to a stratum-1.
    ///
    /// Also reported and not acted on. `rsntp` already refuses stratum 0 (a
    /// kiss-o'-death) and stratum ≥ 16 (unsynchronised), which are the two
    /// values that would be a *decision*.
    pub stratum: u8,
}

/// Why an exchange produced no usable answer.
///
/// Three variants, and the split is by **what an operator would do about it**
/// rather than by where in the stack it arose:
#[derive(Debug, Clone, PartialEq)]
pub enum QueryFailure {
    /// The packet did not get there, or nothing came back in time.
    ///
    /// **The normal state of this service at boot**, and not a fault:
    /// `ADR-017` §3.2's convergence model has a service retry until the thing
    /// it needs exists, and `ADR-021` §7 states in as many words that
    /// `rivet-timed` needing the network *"is not a dependency to declare: it
    /// retries until the network works, like everything else"*.
    Unreachable { detail: String },
    /// Something answered and the answer was not usable — a kiss-o'-death, an
    /// unsynchronised server, a packet whose originate timestamp is not the one
    /// that was sent.
    ///
    /// **Distinct from [`QueryFailure::Unreachable`] because it means the
    /// network is fine**, which is a completely different thing to read on a
    /// console at three in the morning. It is also the shape an on-path
    /// attacker's clumsier attempt takes; `ADR-021` §6 accepts that a competent
    /// one is not detectable here.
    Refused { detail: String },
    /// The offset is a number this machine cannot act on: not finite, or beyond
    /// what an `i64` of milliseconds holds.
    ///
    /// `AGENTS.md` §5 — refuse rather than guess. A saturating conversion would
    /// *succeed*, and the value it succeeded with would be a plausible-looking
    /// clock correction derived from a NaN.
    Unrepresentable { seconds: f64 },
}

impl core::fmt::Display for QueryFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unreachable { detail } => write!(f, "no reply: {detail}"),
            Self::Refused { detail } => write!(f, "the reply was not usable: {detail}"),
            Self::Unrepresentable { seconds } => write!(
                f,
                "the server's offset of {seconds} seconds is not a number of milliseconds this \
                 machine can act on"
            ),
        }
    }
}

impl QueryFailure {
    /// A short, stable word for the journal's `outcome` field, so a reader can
    /// count the three kinds without parsing a sentence.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Unreachable { .. } => "unreachable",
            Self::Refused { .. } => "refused",
            Self::Unrepresentable { .. } => "unrepresentable",
        }
    }
}

/// One SNTP exchange, as a seam.
pub trait Sntp {
    /// Ask one server what time it is. Blocking, and bounded by the adapter's
    /// own timeout.
    ///
    /// **Takes a [`SocketAddr`] and not a hostname**, which is the whole of
    /// `crate::servers`' argument in one signature: there is no name to resolve,
    /// so there is no resolver to depend on. `rsntp` accepts either; this port
    /// accepts only the one Rivet can actually satisfy.
    fn query(&mut self, server: SocketAddr) -> Result<Sample, QueryFailure>;
}

/// Seconds as a float, from the wire, into milliseconds this machine can act
/// on — or nothing.
///
/// **The one piece of arithmetic in this module that can be wrong**, so it is a
/// free function with its own tests rather than three lines inside the adapter,
/// exactly as `crate::kernel`'s `timespec_parts` is.
///
/// The refusals are not theoretical. `rsntp` computes the offset by subtracting
/// NTP timestamps and dividing by 2, and NaN falls out of that arithmetic for a
/// reply carrying garbage; the magnitude bound falls out of a machine whose own
/// clock is at the epoch talking to a server in a different NTP era.
fn offset_ms_from_secs(seconds: f64) -> Option<i64> {
    if !seconds.is_finite() {
        return None;
    }
    let ms = seconds * 1000.0;
    // `i64::MAX` is not exactly representable as an `f64`, so the comparison is
    // written against a power of two that is — 2^62 milliseconds is about 146
    // million years, and anything at that magnitude is a broken exchange rather
    // than a clock that needs correcting.
    const LIMIT: f64 = 4_611_686_018_427_387_904.0; // 2^62
    if ms.abs() >= LIMIT {
        return None;
    }
    #[allow(clippy::cast_possible_truncation)]
    Some(ms as i64)
}

// ---------------------------------------------------------------------------
// The real one. Thin by policy: nothing here is reachable from a test.
// ---------------------------------------------------------------------------

/// `rsntp`, behind the wall.
pub struct RsntpClient {
    client: rsntp::SntpClient,
}

/// How long one exchange may take before it is a failure.
///
/// `rsntp`'s own default is three seconds. It is restated here rather than
/// inherited because it is a **decision**: `ADR-017` §3.2 makes a service that
/// cannot reach what it needs retry, and the cost of waiting is that this
/// process is blocked and unkillable-by-`SIGTERM`-promptly for the duration,
/// times the number of servers tried. Five seconds across four servers is a
/// twenty-second worst case before a shutdown is noticed, which is already at
/// the edge of a supervisor's grace period, so the timeout stays short.
pub const QUERY_TIMEOUT_MS: u64 = 3_000;

impl Default for RsntpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl RsntpClient {
    pub fn new() -> Self {
        let config = rsntp::Config::default()
            .timeout(std::time::Duration::from_millis(QUERY_TIMEOUT_MS))
            // Connect the UDP socket to the server, so the kernel drops
            // datagrams from anything else before they reach this process.
            // `rsntp` defaults it on; it is stated because it is a small,
            // free piece of `ADR-021` §6's otherwise-accepted exposure.
            .connect_ip(true);
        Self {
            client: rsntp::SntpClient::with_config(config),
        }
    }
}

impl Sntp for RsntpClient {
    fn query(&mut self, server: SocketAddr) -> Result<Sample, QueryFailure> {
        let result = match self.client.synchronize(server) {
            Ok(result) => result,
            Err(rsntp::SynchronizationError::IOError(e)) => {
                return Err(QueryFailure::Unreachable {
                    detail: e.to_string(),
                });
            }
            Err(e) => {
                // Everything that is not an I/O error is a reply this client
                // looked at and rejected: a kiss-o'-death, stratum 0 or ≥ 16, a
                // mode that is not `server`, or an originate timestamp that is
                // not the one that was sent. Reporting the library's own words
                // rather than a re-description of them is `AGENTS.md` §5.
                return Err(QueryFailure::Refused {
                    detail: e.to_string(),
                });
            }
        };

        let seconds = result.clock_offset().as_secs_f64();
        let Some(offset_ms) = offset_ms_from_secs(seconds) else {
            return Err(QueryFailure::Unrepresentable { seconds });
        };

        let round_trip = result.round_trip_delay().as_secs_f64();
        // A round trip is reported, never acted on, so a value that will not
        // convert is floored to zero rather than failing the exchange. It is the
        // one number here whose wrongness costs nothing.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let round_trip_ms = if round_trip.is_finite() && round_trip > 0.0 {
            (round_trip * 1000.0).min(u64::MAX as f64) as u64
        } else {
            0
        };

        Ok(Sample {
            offset_ms,
            round_trip_ms,
            stratum: result.stratum(),
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_whole_number_of_seconds_becomes_milliseconds() {
        assert_eq!(offset_ms_from_secs(0.0), Some(0));
        assert_eq!(offset_ms_from_secs(1.5), Some(1_500));
        assert_eq!(offset_ms_from_secs(-2.25), Some(-2_250));
    }

    /// The magnitude a cold Raspberry Pi actually produces: its clock is at the
    /// epoch and the server is in the present, so the first offset of a board's
    /// life is about fifty-six years. It must convert, because it is the one
    /// correction that matters most (`ADR-021` §4.2.1 — *"stepping at boot was
    /// never a choice"*).
    #[test]
    fn fifty_six_years_of_offset_converts_because_that_is_the_boot_case() {
        let fifty_six_years = 1_700_000_000.0_f64;
        assert_eq!(
            offset_ms_from_secs(fifty_six_years),
            Some(1_700_000_000_000)
        );
    }

    /// `AGENTS.md` §5 — refuse rather than guess. A saturating conversion would
    /// succeed and hand the discipline a plausible-looking correction derived
    /// from a NaN.
    #[test]
    fn a_value_that_is_not_a_number_is_refused_rather_than_saturated() {
        assert_eq!(offset_ms_from_secs(f64::NAN), None);
        assert_eq!(offset_ms_from_secs(f64::INFINITY), None);
        assert_eq!(offset_ms_from_secs(f64::NEG_INFINITY), None);
    }

    #[test]
    fn an_offset_beyond_what_milliseconds_hold_is_refused() {
        assert_eq!(offset_ms_from_secs(1e18), None);
        assert_eq!(offset_ms_from_secs(-1e18), None);
        // ...and the boundary is not so tight that a real correction trips it.
        assert!(offset_ms_from_secs(1e12).is_some());
    }

    #[test]
    fn every_failure_renders_a_sentence_and_a_countable_word() {
        for f in [
            QueryFailure::Unreachable {
                detail: "connection refused".to_string(),
            },
            QueryFailure::Refused {
                detail: "kiss of death".to_string(),
            },
            QueryFailure::Unrepresentable { seconds: f64::NAN },
        ] {
            assert!(!f.to_string().is_empty());
            assert!(!f.kind().is_empty());
        }
        assert_ne!(
            QueryFailure::Unreachable {
                detail: String::new()
            }
            .kind(),
            QueryFailure::Refused {
                detail: String::new()
            }
            .kind(),
            "the two failures an operator would act on differently must be countable apart"
        );
    }

    /// `ADR-009` §3.4's wall, as an assertion about this file rather than a
    /// promise in its documentation. A `rsntp` type on a `pub` item in any other
    /// module is the impedance this dependency was taken on condition of
    /// avoiding.
    #[test]
    fn no_rsntp_type_escapes_this_module() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let Ok(entries) = std::fs::read_dir(&root) else {
            unreachable!("this crate has no src directory")
        };
        let mut checked = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            if path.file_name().and_then(|n| n.to_str()) == Some("sntp.rs") {
                continue;
            }
            let Ok(body) = std::fs::read_to_string(&path) else {
                continue;
            };
            checked += 1;
            for line in body.lines() {
                // Documentation is where the dependency is *argued*, so prose
                // mentioning it is not a leak. Code is.
                let code = line.trim_start();
                if code.starts_with("//") {
                    continue;
                }
                assert!(
                    !code.contains("rsntp"),
                    "{}: {line}",
                    path.display()
                );
            }
        }
        assert!(checked >= 4, "only {checked} modules were read; the wall is unchecked");
    }
}
