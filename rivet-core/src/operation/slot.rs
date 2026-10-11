//! `rivet get slot` — **which A/B slot this machine is running, and whether
//! this boot is on trial.**
//!
//! `ADR-035` phase 4, `PLAN.md` §92.3, and
//! `docs/design-candidates/Admin-Surface-Survey.md` §4, which says exactly why
//! it earns a subject: *"which A/B slot is running, what is armed, whether a
//! trial is pending. Reconstructed by hand from `/proc/cmdline` twice this
//! week."*
//!
//! # The one thing this must never do is hash a binary
//!
//! *"Which slot am I running"* has an answer that looks easy and is wrong:
//! compare a hash of the running binary against each slot's copy. **It fails
//! silently.** Two slots holding the same build produce the same hash, so the
//! comparison is a coin toss that reads like a measurement — and what it names
//! is the partition an update writes into. A wrong answer overwrites the running
//! system.
//!
//! The kernel's own `root=` argument is a statement, not an inference. So is the
//! firmware's boot-partition property. Both are read; neither is derived.
//!
//! # Two authorities, and the disagreement is the finding
//!
//! | Reading | Answers | From |
//! | :-- | :-- | :-- |
//! | `booted` | which **boot** partition the firmware read | the firmware's own device-tree property |
//! | `root` | which **system** partition the kernel mounted as `/` | `root=PARTLABEL=system-a` on the kernel command line |
//!
//! They should agree, because `boot-a`'s `cmdline.txt` names `system-a`
//! (`ADR-018` §4) — and `ADR-029` §3 measured that `cmdline.txt` is the *only*
//! file that differs between the two slots' boot partitions, which is what makes
//! it identify one.
//!
//! **When they disagree, this subject names no running slot at all.** That is
//! the whole reason both are read. A machine that booted one slot's firmware
//! configuration into the other slot's filesystem is a machine where
//! `apply trial-boot` would arm the wrong partition, and *"the answer is
//! whichever one I looked at first"* is precisely the silent wrongness
//! `AGENTS.md` §5 refuses. It is [`super::process`]'s arrangement — `ADR-011`
//! §3 usually means deleting one of two authorities, and here the interesting
//! case is exactly where they differ.
//!
//! # A trial that was not measured is not a committed boot
//!
//! `in_trial_boot` is present or absent, never `false` for a flag nobody read.
//! `ADR-018` §5.1 makes a trial roll back unless it is committed, so `false`
//! is a claim that doing nothing is safe — and that is the one claim a build
//! which could not read the flag has no business making.

use super::machine::Observed;
use super::{ErrorCode, Field, OperationError};
use crate::state::Slot;
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "slot";

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was read. Always `slot` in this build.",
    },
    Field {
        name: "running",
        description: "THE ANSWER, when there is one: `a` or `b`. Present when both authorities \
                      below agree, or when exactly one of them could be read. ABSENT WHEN THEY \
                      DISAGREE, and that is deliberate — a machine that booted one slot's \
                      firmware configuration into the other slot's filesystem has no single \
                      running slot, and naming one would send the next update into the wrong \
                      partition.",
    },
    Field {
        name: "inactive",
        description: "The other slot — the one an update writes into, since an update never \
                      writes the slot it is running from (ADR-018 §5). Present exactly when \
                      `running` is.",
    },
    Field {
        name: "booted",
        description: "Which BOOT partition the firmware read, as the firmware itself states it \
                      in the device tree. This is not the same question as `root` and the two \
                      are reported separately on purpose.",
    },
    Field {
        name: "booted_unmeasured",
        description: "Present exactly when `booted` is absent — a development host has no device \
                      tree, and saying so is a different statement from naming a slot.",
    },
    Field {
        name: "booted_from",
        description: "The device-tree property the firmware's answer was read from.",
    },
    Field {
        name: "root",
        description: "Which SYSTEM partition the kernel mounted as `/`, resolved from \
                      `root_argument`: `PARTLABEL=system-a` is slot `a`. Absent when the \
                      argument names something this layout does not describe, which is not a \
                      failure — a machine booted by `root=UUID=…` is a real machine and it is \
                      not running an ADR-018 §4 slot.",
    },
    Field {
        name: "root_argument",
        description: "The `root=` argument this kernel was given, VERBATIM. Reported even when \
                      it names no slot, because what the kernel was actually told is the fact; \
                      what this build makes of it is an interpretation.",
    },
    Field {
        name: "root_unmeasured",
        description: "Present exactly when `root_argument` is absent, and says why.",
    },
    Field {
        name: "agree",
        description: "Whether the firmware's boot partition and the kernel's root partition name \
                      the SAME slot. Present only when both were measured. FALSE IS SERIOUS: it \
                      means this machine booted one slot's firmware configuration into the \
                      other's filesystem, `running` is withheld, and nothing should be armed \
                      until it is understood.",
    },
    Field {
        name: "in_trial_boot",
        description: "HOW THIS BOOT WAS ENTERED, and nothing else: whether the firmware took \
                      ADR-018 §5's one-shot tryboot path. It is set before userspace runs and \
                      never changes afterwards, so it stays TRUE for the rest of the life of a \
                      trial that has since been CONFIRMED. **It is not the answer to \"will a \
                      restart come back here\" — `committed` is.** Absent — never `false` — \
                      where the flag could not be read.",
    },
    Field {
        name: "trial_unmeasured",
        description: "Present exactly when `in_trial_boot` is absent, and says why.",
    },
    Field {
        name: "default_slot",
        description: "The slot an ORDINARY restart boots, read from the platform's default-slot \
                      record. Absent when it could not be read.",
    },
    Field {
        name: "default_unmeasured",
        description: "Present exactly when `default_slot` is absent, and says why.",
    },
    Field {
        name: "committed",
        description: "WHETHER A RESTART COMES BACK HERE — the default slot and the running slot \
                      are the same. TRUE means this boot is the machine's settled state however \
                      it was entered; FALSE means an uncommitted trial, and doing nothing rolls \
                      it back. Present only when both slots were measured. Added 2026-09-05 \
                      after `in_trial_boot` misled three separate readers.",
    },
    Field {
        name: "read_from",
        description: "Where the kernel command line was read from.",
    },
    Field {
        name: "means",
        description: "What this reading asserts, in prose — including, when the two authorities \
                      disagree, that nothing should be armed on this machine until somebody \
                      understands why.",
    },
];

/// Perform it.
///
/// # Errors
///
/// If a second word was given, or nothing at all about this machine's slots
/// could be read.
pub(super) fn perform(
    arguments: &[&str],
    observed: &mut dyn Observed,
) -> Result<Value, OperationError> {
    if let Some(extra) = arguments.get(1) {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "`{SUBJECT}` takes no second word and {extra:?} is one. It reads which slot is \
                 running; naming a slot to ARM one is `apply trial-boot`, which is a different \
                 verb because it changes the machine."
            ),
        ));
    }

    let read = observed.read_slots().map_err(|e| {
        OperationError::about(ErrorCode::MachineRefused, SUBJECT, e.why().to_string())
    })?;

    // **The one interpretation this subject makes**, and it is the domain's
    // because `Slot` is: `PARTLABEL=system-a` names slot A. The adapter passed
    // the argument through verbatim precisely so that this spelling lives in
    // one place (`crate::state::Slot::system_partlabel`).
    let root = read.root_argument.as_deref().and_then(slot_of_root);

    // **Withheld when the two disagree.** See this module's documentation: a
    // machine with two answers has no running slot, and picking one would name
    // the partition an update overwrites.
    let running = match (read.booted, root) {
        (Some(a), Some(b)) if a == b => Some(a),
        (Some(_), Some(_)) => None,
        (Some(only), None) | (None, Some(only)) => Some(only),
        (None, None) => None,
    };

    let mut fields = vec![("subject", Value::str(SUBJECT))];
    if let Some(slot) = running {
        fields.push(("running", Value::str(slot.as_str())));
        fields.push(("inactive", Value::str(slot.other().as_str())));
    }
    if let Some(slot) = read.booted {
        fields.push(("booted", Value::str(slot.as_str())));
    } else if let Some(why) = &read.booted_unmeasured {
        fields.push(("booted_unmeasured", Value::str(why.clone())));
    }
    fields.push(("booted_from", Value::str(read.booted_from.clone())));
    if let Some(slot) = root {
        fields.push(("root", Value::str(slot.as_str())));
    }
    if let Some(argument) = &read.root_argument {
        fields.push(("root_argument", Value::str(argument.clone())));
    } else if let Some(why) = &read.root_unmeasured {
        fields.push(("root_unmeasured", Value::str(why.clone())));
    }
    // Present only when there were two answers to compare. `true` on a machine
    // where one of them is missing would be an agreement with nothing.
    if let (Some(a), Some(b)) = (read.booted, root) {
        fields.push(("agree", Value::Bool(a == b)));
    }
    // **Present or absent, never `false` for an unread flag.**
    if let Some(trial) = read.in_trial_boot {
        fields.push(("in_trial_boot", Value::Bool(trial)));
    } else if let Some(why) = &read.trial_unmeasured {
        fields.push(("trial_unmeasured", Value::str(why.clone())));
    }
    if let Some(default) = read.default_slot {
        fields.push(("default_slot", Value::str(default.as_str().to_string())));
    } else if let Some(why) = &read.default_unmeasured {
        fields.push(("default_unmeasured", Value::str(why.clone())));
    }
    // Same rule as every other derived field here: present only when both
    // halves were measured, because a `false` composed from a missing reading
    // would say *this boot rolls back* about a machine nobody asked.
    if let Some(committed) = read.committed() {
        fields.push(("committed", Value::Bool(committed)));
    }
    fields.push(("read_from", Value::str(read.root_from.clone())));
    fields.push(("means", Value::str(means(&read, root, running))));

    debug_assert!(
        super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)),
        "a get slot result's keys must be a subsequence of OUTPUTS, in order"
    );
    Ok(Value::object(fields))
}

/// Which slot a `root=` argument names, or `None`.
///
/// **`None` rather than a default**, for `crate::state::Slot::
/// from_boot_partition`'s reason and in its words: the guess would name a
/// partition an update would then overwrite. A machine booted by
/// `root=UUID=…` is a real machine that is not running an `ADR-018` §4 slot,
/// and that is a true answer rather than a failure.
fn slot_of_root(argument: &str) -> Option<Slot> {
    let label = argument.strip_prefix("PARTLABEL=")?;
    [Slot::A, Slot::B]
        .into_iter()
        .find(|slot| slot.system_partlabel() == label)
}

/// **What this reading asserts**, composed from what was actually measured.
///
/// The disagreement case gets its own paragraph and its own instruction,
/// because it is the one where a reader who skims and acts does damage.
fn means(read: &super::machine::SlotRead, root: Option<Slot>, running: Option<Slot>) -> String {
    let mut said = String::new();

    match (read.booted, root) {
        (Some(a), Some(b)) if a != b => said.push_str(
            "THE TWO AUTHORITIES DISAGREE. The firmware says it read one slot's boot partition \
             and the kernel says it mounted the other slot's filesystem as /. No running slot is \
             named here because there is not one, and NOTHING SHOULD BE ARMED on this machine \
             until somebody understands why: `apply trial-boot` would write into a partition \
             chosen from an answer this machine cannot give. ",
        ),
        (Some(_), Some(_)) => said.push_str(
            "Both the firmware and the kernel were asked, independently, and they agree. This is \
             read from what each of them STATES — the device tree and the kernel command line — \
             and never from hashing a binary, which two slots holding the same build would make \
             a coin toss. ",
        ),
        (None, None) => said.push_str(
            "Neither the firmware's boot partition nor the kernel's root argument named a slot, \
             so nothing here says which one is running. ",
        ),
        _ => said.push_str(
            "Only one of the two authorities answered, so this is a single reading rather than a \
             corroborated one — see `booted_unmeasured` or `root_unmeasured` for which was \
             missing and why. ",
        ),
    }

    if running.is_some() {
        said.push_str(
            "`inactive` is the slot an update writes into, because an update never writes the \
             slot it is running from (ADR-018 §5). ",
        );
    }

    // **`committed` leads, because it is the question being asked.** Whether
    // the firmware took the tryboot path is history; whether a restart comes
    // back here is the thing a reader is deciding on, and until 2026-09-05 this
    // paragraph answered the first while appearing to answer the second.
    match (read.in_trial_boot, read.committed()) {
        (Some(true), Some(true)) => said.push_str(
            "This boot was ENTERED as a one-shot trial and has since been CONFIRMED: the default \
             slot is this one, so an ordinary restart comes back here and nothing rolls back. \
             `in_trial_boot` stays true for the life of the boot because it records how the \
             firmware started it, not what happened afterwards.",
        ),
        (Some(true), Some(false)) => said.push_str(
            "THIS BOOT IS AN UNCOMMITTED ONE-SHOT TRIAL. An ordinary restart goes back to the \
             default slot: ADR-018 §5.1 rolls a trial back unless it is committed, and doing \
             nothing is what rolls it back.",
        ),
        (Some(true), None) => said.push_str(
            "This boot was entered as a one-shot trial, and WHETHER IT HAS BEEN COMMITTED WAS \
             NOT MEASURED — see `default_unmeasured`. Resolve that before restarting anything: \
             an uncommitted trial rolls back and a committed one does not, and this answer \
             cannot tell them apart.",
        ),
        (Some(false), _) => said.push_str(
            "This is not a trial boot, so a restart comes back to the same slot.",
        ),
        (None, _) => said.push_str(
            "WHETHER THIS IS A TRIAL BOOT WAS NOT MEASURED, which is not the same as its being a \
             committed one — see `trial_unmeasured`. A trial that is not committed rolls back at \
             the next restart, so this is worth resolving before restarting anything.",
        ),
    }

    said
}

#[cfg(test)]
#[path = "slot_tests.rs"]
mod tests;
