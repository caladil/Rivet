//! `rivet get thermal` — **what this machine's silicon is doing to itself, and
//! what it has already done.**
//!
//! `ADR-035` phase 4, the third of the five the maintainer named on 2026-08-29,
//! and the one `docs/design-candidates/Admin-Surface-Survey.md` §7 was written
//! about. Nothing answered any part of it before: §7's own verification line is
//! *"there is no `thermal`, `hwmon` or throttle code anywhere in this
//! repository."*
//!
//! # It is not `get temperature`, and the rename is the whole design
//!
//! §7.1 corrected the framing before a line was written. The subject was
//! `get temperature` in §4's table; thinking it through produced the opposite
//! conclusion — **heat is the weakest of the four facts**:
//!
//! | Fact | Worth |
//! | :-- | :-- |
//! | SoC temperature | the number everyone asks for. 70 °C is fine, and a board that throttled an hour ago reads cool by the time anyone looks |
//! | clock against its ceiling | the throttle **observed** rather than inferred — where the ceiling is what moved |
//! | under-voltage now | the Pi's actual killer. A marginal PSU or a cheap cable, and a leading cause of card corruption |
//! | **…since boot** | the only one of the four that cannot be recovered any other way |
//!
//! That last row is why this is worth building carefully rather than quickly. A
//! board that is fine at 09:00 and spent 03:00 under-volted looks **identical**
//! to one that never did — every other reading here is a spot value.
//!
//! # A current frequency below the maximum is NOT a throttle
//!
//! It is the one misreading this subject would otherwise invite, and it would
//! fire constantly: a governor scales an idle machine down, so a Pi at 600 MHz
//! against a 1500 MHz maximum is behaving correctly. So four numbers are
//! reported rather than a boolean, and [`OUTPUTS`] says what each is worth. The
//! comparison that would show a lowered **ceiling** is `ceiling_hz` against
//! `maximum_hz`; whether this platform's firmware throttle shows up there at all
//! is not something this build has measured, and the answer says so rather than
//! implying it.
//!
//! # What was not measured is reported as unmeasured
//!
//! [`super::memory`]'s rule for PSI, and here it is load-bearing rather than
//! tidy. `throttled_since_boot: false` is a **claim about the last several
//! hours of this board's life**, and a build that could not reach the firmware
//! has established nothing of the kind. So the flags are present or absent, an
//! absent set carries the reason it is absent, and there is a test that a
//! machine which cannot answer produces no flags rather than clear ones.

use super::machine::Observed;
use super::{ErrorCode, Field, OperationError};
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
///
/// **`thermal`, not `temperature`.** The survey's §4 proposed the latter and
/// §7.1 overturned it in the same document: the subject is four facts and the
/// temperature is the least useful of them, so a name that promised only the
/// temperature would describe the least of what this answers.
pub const SUBJECT: &str = "thermal";

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was read. Always `thermal` in this build.",
    },
    Field {
        name: "zones",
        description: "Every thermal zone, in ascending zone order so that two readings of an \
                      unchanged machine produce the same bytes. Each carries `name` — the \
                      driver's own word for itself, `cpu_thermal` on a Pi 4 and `coretemp` on \
                      x86 — and `millidegrees`, which is THOUSANDTHS OF A DEGREE CELSIUS \
                      because that is the unit Linux states and this surface has no \
                      floating-point value to convert it into. An empty list on a machine whose \
                      zone directory WAS read is a true answer: some hardware has none.",
    },
    Field {
        name: "zones_unreadable",
        description: "Zones that were listed and could not be read, each with the reason the \
                      system gave. NEVER DROPPED: an answer that omitted one would say this \
                      machine has no such zone, which is a different statement and a false one.",
    },
    Field {
        name: "frequency",
        description: "This CPU's clock, in HERTZ — `current_hz` now, `maximum_hz` what the \
                      hardware can do, `minimum_hz` its floor, `ceiling_hz` the ceiling policy \
                      currently allows, and `governor`. READ THESE CAREFULLY: a `current_hz` \
                      below `maximum_hz` IS NOT A THROTTLE — a governor scales an idle machine \
                      down, and on a Pi that is the ordinary state. What would show a throttle \
                      is `ceiling_hz` below `maximum_hz`, which is the ceiling itself moving; \
                      `at_maximum` and `ceiling_below_maximum` are those two comparisons made \
                      once here rather than by every caller. AND `at_maximum` TRUE IS NOT A \
                      THERMAL SIGNAL EITHER: under a steady load a governor sits at maximum \
                      indefinitely, so the flag says this CPU is BUSY, not that it is hot. An \
                      eight-hour soak on 2026-09-03 read it true at every sample — including \
                      while the die was COOLING — and an operator rule keyed on it would have \
                      powered a healthy board off. For whether this machine has actually been \
                      held back, read `throttle`, whose `_since_boot` half answers for the \
                      whole run rather than for the instant you asked. Whether this platform's \
                      FIRMWARE throttle appears in `ceiling_hz` at all has not been measured by \
                      this build — the flags are the authority for that, not this. `source` \
                      names which CPU's policy was read, because a big.LITTLE machine has more \
                      than one and this is one of them.",
    },
    Field {
        name: "frequency_unmeasured",
        description: "Present exactly when `frequency` is absent, and says why in the system's \
                      own words. A machine with no `cpufreq` is an ordinary machine, not a \
                      fault.",
    },
    Field {
        name: "throttle",
        description: "The firmware's own account of whether it has held this machine back: \
                      `under_voltage_*`, `frequency_capped_*`, `throttled_*` and \
                      `soft_temperature_limit_*`, each in a `_now` and a `_since_boot` form, \
                      plus `raw` — the word the firmware returned, kept whole so a bit this \
                      build does not know about is still visible — and `source`. THE \
                      `_since_boot` HALF IS THE POINT: a board that is fine now and was \
                      under-volted at 03:00 is indistinguishable from one that never was, and \
                      nothing else on this machine recovers that.",
    },
    Field {
        name: "throttle_unmeasured",
        description: "Present exactly when `throttle` is absent, and it is the most important \
                      field in this subject. ABSENT FLAGS ARE NOT CLEAR FLAGS. A build that \
                      could not reach the firmware has not established that this board has never \
                      been under-volted — it has established nothing — so the flags are omitted \
                      and this says why, rather than reporting eight falses that would read as a \
                      clean bill of health. Same rule `get memory` follows for pressure.",
    },
    Field {
        name: "read_from",
        description: "Where the zones were looked for, whether or not any were found, so that \
                      an empty list names a place rather than shrugging.",
    },
    Field {
        name: "means",
        description: "What this reading does and does not assert, in prose — in particular \
                      which of the four facts of ADR-035 phase 4's thermal surface were \
                      actually measured on this machine.",
    },
];

/// Perform it.
///
/// # Errors
///
/// If a second word was given, or neither of the machine's two thermal
/// interfaces could be read.
pub(super) fn perform(
    arguments: &[&str],
    observed: &mut dyn Observed,
) -> Result<Value, OperationError> {
    if let Some(extra) = arguments.get(1) {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "`{SUBJECT}` takes no second word and {extra:?} is one. The whole reading is the \
                 answer; select a zone or a frequency from it with a JSON tool rather than with \
                 an argument this build does not have."
            ),
        ));
    }

    let read = observed.read_thermal().map_err(|e| {
        OperationError::about(ErrorCode::MachineRefused, SUBJECT, e.why().to_string())
    })?;

    let mut fields = vec![
        ("subject", Value::str(SUBJECT)),
        (
            "zones",
            Value::List(
                read.zones
                    .iter()
                    .map(|z| {
                        Value::object(vec![
                            ("name", Value::str(z.name.clone())),
                            ("millidegrees", Value::Integer(z.millidegrees)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "zones_unreadable",
            Value::List(
                read.zones_unreadable
                    .iter()
                    .map(|u| {
                        Value::object(vec![
                            ("zone", Value::str(u.zone.clone())),
                            ("why", Value::str(u.why.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
    ];

    // **Present or absent, never a zero clock.** A machine with no `cpufreq`
    // has not been measured at 0 Hz.
    if let Some(f) = &read.frequency {
        let hz = |n: u64| Value::Integer(i64::try_from(n).unwrap_or(i64::MAX));
        let mut clock = vec![
            ("current_hz", hz(f.current_hz)),
            ("maximum_hz", hz(f.maximum_hz)),
        ];
        if let Some(minimum) = f.minimum_hz {
            clock.push(("minimum_hz", hz(minimum)));
        }
        if let Some(ceiling) = f.ceiling_hz {
            clock.push(("ceiling_hz", hz(ceiling)));
            // **The comparison that would actually show a throttle**, made once
            // here rather than by every caller — `get process`'s `userland`
            // argument, and the same reason: a check nobody makes is a check
            // that does not exist.
            clock.push((
                "ceiling_below_maximum",
                Value::Bool(ceiling < f.maximum_hz),
            ));
        }
        if let Some(governor) = &f.governor {
            clock.push(("governor", Value::str(governor.clone())));
        }
        // **NOT `throttled`.** An idle machine is below its maximum almost all
        // the time, and a field named for the inference would be wrong far more
        // often than it was right.
        clock.push(("at_maximum", Value::Bool(f.current_hz >= f.maximum_hz)));
        clock.push(("source", Value::str(f.source.clone())));
        fields.push(("frequency", Value::object(clock)));
    } else if let Some(why) = &read.frequency_unmeasured {
        fields.push(("frequency_unmeasured", Value::str(why.clone())));
    }

    // **The one that must never be invented.** Eight falses would read as a
    // clean bill of health for the last several hours of this board's life.
    if let Some(t) = &read.throttle {
        fields.push((
            "throttle",
            Value::object(vec![
                ("under_voltage_now", Value::Bool(t.under_voltage_now)),
                ("frequency_capped_now", Value::Bool(t.frequency_capped_now)),
                ("throttled_now", Value::Bool(t.throttled_now)),
                (
                    "soft_temperature_limit_now",
                    Value::Bool(t.soft_temperature_limit_now),
                ),
                (
                    "under_voltage_since_boot",
                    Value::Bool(t.under_voltage_since_boot),
                ),
                (
                    "frequency_capped_since_boot",
                    Value::Bool(t.frequency_capped_since_boot),
                ),
                ("throttled_since_boot", Value::Bool(t.throttled_since_boot)),
                (
                    "soft_temperature_limit_since_boot",
                    Value::Bool(t.soft_temperature_limit_since_boot),
                ),
                ("raw", Value::Integer(i64::from(t.raw))),
                ("source", Value::str(t.source.clone())),
            ]),
        ));
    } else if let Some(why) = &read.throttle_unmeasured {
        fields.push(("throttle_unmeasured", Value::str(why.clone())));
    }

    fields.push(("read_from", Value::str(read.zone_source.clone())));
    fields.push(("means", Value::str(means(&read))));

    debug_assert!(
        super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)),
        "a get thermal result's keys must be a subsequence of OUTPUTS, in order"
    );
    Ok(Value::object(fields))
}

/// **What this reading asserts, and what it deliberately does not.**
///
/// It is composed from what was actually measured rather than written once,
/// because the sentence that matters changes with the answer: a machine whose
/// flags were read has a different caveat from one whose were not, and a reader
/// handed the wrong one is worse off than a reader handed none.
fn means(read: &super::machine::ThermalRead) -> String {
    let mut said = String::new();

    if read.zones.is_empty() && read.zones_unreadable.is_empty() {
        said.push_str(
            "No thermal zone was found, so no temperature was measured — which is a fact about \
             this machine's interfaces, not a temperature of zero. ",
        );
    } else {
        said.push_str(
            "Temperatures are a SPOT READING. A board that throttled an hour ago reads cool now, \
             so a comfortable number here rules nothing out. ",
        );
    }

    match &read.frequency {
        Some(f) if f.current_hz < f.maximum_hz => said.push_str(
            "This CPU is below its maximum clock, WHICH IS NOT BY ITSELF A THROTTLE: a governor \
             scales an idle machine down and that is the ordinary state. What would show a \
             throttle is the ceiling moving, not the current clock. ",
        ),
        Some(_) => said.push_str("This CPU is at its maximum clock. "),
        None => said.push_str("No cpufreq policy was read, so nothing here is about the clock. "),
    }

    if read.throttle.is_some() {
        said.push_str(
            "The firmware's throttle flags WERE read, so the since-boot half of them is the \
             authority on whether this board has been under-volted or held back at any point \
             during this boot.",
        );
    } else {
        said.push_str(
            "THE FIRMWARE'S THROTTLE FLAGS WERE NOT READ. Nothing in this answer says whether \
             this board has been under-volted or throttled — including at 03:00, when nobody was \
             looking, which is precisely the case those flags exist to recover. Under-voltage is \
             the commonest cause of a Pi behaving strangely and a leading cause of card \
             corruption; a clean-looking reading here is not evidence against it.",
        );
    }

    said
}

#[cfg(test)]
#[path = "thermal_tests.rs"]
mod tests;
