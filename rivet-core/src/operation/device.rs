//! `rivet get device` — **what hardware this machine actually has under its
//! filesystems.**
//!
//! `ADR-035` phase 4, the fifth of the five the maintainer named on 2026-08-29,
//! and `docs/design-candidates/Admin-Surface-Survey.md` §7.3.
//!
//! # It is not `get storage`, and the difference is which question
//!
//! | | |
//! | :-- | :-- |
//! | [`super::storage`] | how full each **mounted filesystem** is — `ADR-005` §4.1.3 |
//! | this | what **block devices** are there, whether or not anything is mounted on them |
//!
//! A partition that failed to mount is **invisible** to the first and present
//! here, which is the case that matters on this hardware: on an A/B machine the
//! inactive slot's `/system` is a real partition with no filesystem view, and
//! *"is the other slot even there"* was not a question this system could
//! answer.
//!
//! `ADR-035` §9's original scope named `/sys/class/block/*/uevent` as one of
//! *"the reads a debugging session actually needed on 2026-08-28"*, and it was
//! reached with a shell.
//!
//! # The number this refuses to invent
//!
//! Survey §7.3 was asked for *"wear-levelling metrics and remaining
//! endurance"*, and split the answer rather than producing one:
//!
//! * **A JEDEC 5.0+ eMMC genuinely reports it** — `life_time` and
//!   `pre_eol_info`. Real, and worth reading where present.
//! * **A consumer SD card generally does not.** A subject that reported
//!   *"remaining endurance"* for one would be **inventing a number**, which is
//!   the exact failure this codebase refuses everywhere else.
//!
//! So both figures travel **verbatim where the device states them and absent
//! where it does not**, and nothing here turns a vendor code into a percentage.
//! What is always there is `written_bytes`, and §7.3 is right that it is the
//! more useful number anyway: `ADR-018` §7 already treats SD wear as a design
//! constraint — the journal-on-disk decisions were all made for it — so
//! measuring actual write volume **closes the loop on decisions that were taken
//! blind**.

use super::machine::Observed;
use super::{ErrorCode, Field, OperationError};
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "device";

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was read. Always `device` in this build.",
    },
    Field {
        name: "devices",
        description: "Every block device, IN NAME ORDER so that two readings of an unchanged \
                      machine produce the same bytes. Each carries `name`, `kind` (`disk` or \
                      `partition`, the kernel's own word), `node`, `partition_label`, \
                      `partition_number`, `parent`, `size_bytes`, `read_only`, `removable`, \
                      `model`, `io`, and — WHERE THE DEVICE ITSELF STATES THEM — `life_time` and \
                      `pre_eol_info`. `partition_label` is the GPT label: it is what \
                      `root=PARTLABEL=` names and what a running machine finds /state by, which \
                      makes it the most useful single string on a partition. `size_bytes` is \
                      BYTES: sysfs states sectors and always in 512-byte units whatever the \
                      device's own block size is, and the conversion happens once in the adapter.",
    },
    Field {
        name: "devices_unreadable",
        description: "Devices that were listed and could not be read, each with what the system \
                      said. NEVER DROPPED: on an A/B machine the device that went missing from \
                      an answer is the one somebody is looking for.",
    },
    Field {
        name: "disks",
        description: "How many of them are whole disks rather than partitions. Computed here so \
                      that the count does not depend on every caller knowing which `kind` word \
                      to filter on.",
    },
    Field {
        name: "read_from",
        description: "Where the devices were looked for, so an empty list names a place.",
    },
    Field {
        name: "means",
        description: "What this reading does and does not assert, in prose — in particular that \
                      NO REMAINING ENDURANCE IS REPORTED for a device that does not state one, \
                      and that this subject sees partitions `get storage` cannot because nothing \
                      is mounted on them.",
    },
];

/// Perform it.
///
/// # Errors
///
/// If a second word was given, or the machine's block-device directory could
/// not be listed.
pub(super) fn perform(
    arguments: &[&str],
    observed: &mut dyn Observed,
) -> Result<Value, OperationError> {
    if let Some(extra) = arguments.get(1) {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "`{SUBJECT}` takes no second word and {extra:?} is one. The whole table is the \
                 answer; select one device from it with a JSON tool rather than with an argument \
                 this build does not have. If you wanted how full a MOUNTED filesystem is, that \
                 is `{}` — a different question about the same hardware.",
                super::storage::SUBJECT
            ),
        ));
    }

    let read = observed.read_devices().map_err(|e| {
        OperationError::about(ErrorCode::MachineRefused, SUBJECT, e.why().to_string())
    })?;

    let number = |n: u64| Value::Integer(i64::try_from(n).unwrap_or(i64::MAX));
    let disks = read
        .devices
        .iter()
        .filter(|d| d.kind.as_deref() == Some(DISK))
        .count();

    let devices: Vec<Value> = read
        .devices
        .iter()
        .map(|d| {
            let mut fields = vec![("name", Value::str(d.name.clone()))];
            // Every one of these is absent rather than defaulted: a `loop`
            // device has no model, a whole disk has no partition label, and a
            // zero or an empty string in either place would be a statement.
            if let Some(kind) = &d.kind {
                fields.push(("kind", Value::str(kind.clone())));
            }
            if let Some(node) = &d.node {
                fields.push(("node", Value::str(node.clone())));
            }
            if let Some(label) = &d.partition_label {
                fields.push(("partition_label", Value::str(label.clone())));
            }
            if let Some(n) = d.partition_number {
                fields.push(("partition_number", Value::Integer(i64::from(n))));
            }
            if let Some(parent) = &d.parent {
                fields.push(("parent", Value::str(parent.clone())));
            }
            if let Some(size) = d.size_bytes {
                fields.push(("size_bytes", number(size)));
            }
            if let Some(read_only) = d.read_only {
                fields.push(("read_only", Value::Bool(read_only)));
            }
            if let Some(removable) = d.removable {
                fields.push(("removable", Value::Bool(removable)));
            }
            if let Some(model) = &d.model {
                fields.push(("model", Value::str(model.clone())));
            }
            if let Some(io) = &d.io {
                fields.push((
                    "io",
                    Value::object(vec![
                        ("read_bytes", number(io.read_bytes)),
                        ("written_bytes", number(io.written_bytes)),
                        ("reads_completed", number(io.reads_completed)),
                        ("writes_completed", number(io.writes_completed)),
                    ]),
                ));
            }
            // **Verbatim or absent.** Survey §7.3: a percentage computed here
            // would be the invented endurance figure that section refuses.
            if let Some(life) = &d.life_time {
                fields.push(("life_time", Value::str(life.clone())));
            }
            if let Some(eol) = &d.pre_eol_info {
                fields.push(("pre_eol_info", Value::str(eol.clone())));
            }
            Value::object(fields)
        })
        .collect();

    let fields = vec![
        ("subject", Value::str(SUBJECT)),
        ("devices", Value::List(devices)),
        (
            "devices_unreadable",
            Value::List(
                read.devices_unreadable
                    .iter()
                    .map(|u| {
                        Value::object(vec![
                            ("what", Value::str(u.what.clone())),
                            ("why", Value::str(u.why.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "disks",
            Value::Integer(i64::try_from(disks).unwrap_or(i64::MAX)),
        ),
        ("read_from", Value::str(read.source.clone())),
        ("means", Value::str(means(&read))),
    ];

    debug_assert!(
        super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)),
        "a get device result's keys must be a subsequence of OUTPUTS, in order"
    );
    Ok(Value::object(fields))
}

/// The kernel's word for a whole disk, as `uevent`'s `DEVTYPE` spells it.
const DISK: &str = "disk";

/// **What this reading asserts**, composed from what was measured.
fn means(read: &super::machine::DeviceRead) -> String {
    let mut said = String::from(
        "These are BLOCK DEVICES, not filesystems: a partition appears here whether or not \
         anything is mounted on it, which is how the inactive A/B slot's /system becomes \
         visible at all. `get storage` answers the other question and cannot see it. ",
    );

    let states_endurance = read
        .devices
        .iter()
        .any(|d| d.life_time.is_some() || d.pre_eol_info.is_some());
    if states_endurance {
        said.push_str(
            "At least one device states its own `life_time` or `pre_eol_info`, and those are \
             carried VERBATIM as the device's vendor codes — nothing here converts one into a \
             percentage or a remaining life, because that mapping is the vendor's. ",
        );
    } else {
        said.push_str(
            "NO DEVICE HERE STATES A REMAINING ENDURANCE, and none is reported. A JEDEC 5.0+ \
             eMMC reports `life_time` and `pre_eol_info`; a consumer SD card generally does not, \
             and a figure produced for one would be invented rather than measured. What IS real \
             is `written_bytes` — ADR-018 §7 treats SD wear as a design constraint and this is \
             the first thing that has ever measured any of it. ",
        );
    }

    if read.devices_unreadable.is_empty() {
        said.push_str("Every device that was listed was read.");
    } else {
        said.push_str(
            "SOME DEVICES COULD NOT BE READ and are named in `devices_unreadable` rather than \
             dropped — an answer that omitted one would say this machine does not have it.",
        );
    }

    said
}

#[cfg(test)]
#[path = "device_tests.rs"]
mod tests;
