//! **An addon that counts**, so the crossing can be proven without a sensor.
//!
//! What this probe is for, in the order the proof needs it:
//!
//! 1. a Rust application can `dlopen` a class-matched object at all;
//! 2. the version handshake happens before any bytes are exchanged;
//! 3. state survives between calls — the count goes up;
//! 4. an addon's own cadence reaches the host — this one asks for a short
//!    retry until it is configured, then a long one, which is exactly the
//!    shape a networked reading source's boot race needs (`.rulings/` R-197 D4);
//! 5. a panicking addon returns a fault instead of killing the host.
//!
//! Point 5 is why [`ProbeAddon::poll`] panics on demand: `panic-now`, a
//! setting nothing real would have, is how rule 3 of the ABI gets exercised
//! from outside the process — nothing else can trigger a panic across the
//! boundary on command.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use rivetos_addon::abi::Addon;
use rivetos_addon::{Description, Field, FieldKind, Polled, Reading, Status, Trend};

/// How long to wait when nothing has configured this addon yet. Short on
/// purpose: an addon that is not ready should ask to be called again soon,
/// not sulk for a whole interval.
const UNCONFIGURED_RETRY_MS: u64 = 2_000;

#[derive(Default)]
pub struct ProbeAddon {
    polls: u64,
    step: f64,
    configured: bool,
    panic_now: bool,
}

impl Addon for ProbeAddon {
    fn describe(&mut self) -> Description {
        Description {
            name: "addon-probe".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            class: "probe-source".to_string(),
            material: Vec::new(),
            // **This probe measures; it does not draw.** A catalogue is a
            // drawing addon's, and `examples/probes/frame-probe` is the one
            // that has one.
            backgrounds: Vec::new(),
            fields: vec![
                Field {
                    key: "step".to_string(),
                    label: "Step".to_string(),
                    help: "How much the reading climbs per poll.".to_string(),
                    kind: FieldKind::Number {
                        default: 1.0,
                        min: 0.0,
                        max: 100.0,
                        unit: "units".to_string(),
                    },
                },
                Field {
                    key: "panic-now".to_string(),
                    label: "Panic on the next poll".to_string(),
                    help: "Proves the boundary turns a panic into an answer.".to_string(),
                    kind: FieldKind::Bool { default: false },
                },
            ],
        }
    }

    fn configure(
        &mut self,
        settings: &serde_json::Map<String, serde_json::Value>,
        _material: &BTreeMap<String, String>,
    ) -> Result<(), String> {
        self.step = settings
            .get("step")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(1.0);
        if self.step < 0.0 {
            return Err("step cannot be negative".to_string());
        }
        self.panic_now = settings
            .get("panic-now")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        self.configured = true;
        Ok(())
    }

    fn poll(&mut self, now_ms: i64) -> Polled {
        assert!(
            !self.panic_now,
            "asked to panic, which the boundary must turn into an answer"
        );
        if !self.configured {
            return Polled {
                reading: None,
                status: Status::Unavailable,
                call_again_in_ms: UNCONFIGURED_RETRY_MS,
                complaint: "nothing has configured this addon yet".to_string(),
                also: Vec::new(),
            };
        }
        self.polls += 1;
        #[allow(clippy::cast_precision_loss)]
        let value = self.polls as f64 * self.step;
        Polled {
            reading: Some(Reading {
                value,
                unit: "units".to_string(),
                taken_at_ms: now_ms,
                trend: Some(Trend::Rising),
                // Above 50 units this probe says so, which is the only use it
                // has for the field: a host marking a reading without knowing
                // what makes one worth marking.
                attention: value > 50.0,
            }),
            status: Status::Ok,
            call_again_in_ms: 60_000,
            complaint: String::new(),
            // This probe measures one thing. `Polled::also` exists for a
            // source that measured several at once — TempMon's indoor sensor
            // reporting humidity beside its temperature — and a probe that
            // filled it would be proving a shape no host asked it for.
            also: Vec::new(),
        }
    }
}

rivetos_addon::export_addon!(ProbeAddon);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_will_not_answer_with_a_reading_before_it_is_configured() {
        let mut a = ProbeAddon::default();
        let p = a.poll(0);
        assert!(p.reading.is_none());
        assert_eq!(p.status, Status::Unavailable);
        assert_eq!(
            p.call_again_in_ms, UNCONFIGURED_RETRY_MS,
            "an addon that is not ready asks to be called again SOON"
        );
    }

    #[test]
    fn the_count_survives_between_calls() {
        let mut a = ProbeAddon::default();
        a.configure(&serde_json::Map::new(), &BTreeMap::new())
            .expect("configures");
        assert_eq!(a.poll(0).reading.expect("a reading").value, 1.0);
        assert_eq!(a.poll(0).reading.expect("a reading").value, 2.0);
    }
}
