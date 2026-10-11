//! Reading which system partition the kernel actually mounted — `ADR-035`
//! phase 4.
//!
//! **`/proc/cmdline`, and nothing else here.** The *other* half of `get slot` —
//! which boot partition the firmware read, and whether this is a one-shot trial
//! — comes off `rivet_adapter_image::PiFirmware` through a `BootMedium`, and
//! `crate::boot` is where the two are put together. They are separated because
//! they are separate seams: this one is a `read_to_string` with no mount and no
//! device tree behind it, so it answers on any Linux.
//!
//! # Why the command line and never a hash
//!
//! The question *"which slot am I running"* has an answer that looks easy and
//! is wrong: hash the running binary and compare it against each slot's copy.
//! **It fails silently.** Two slots holding the same build produce the same
//! hash, so the comparison is a coin toss that reads like a measurement — and
//! the cost of getting it wrong is an update written over the running system.
//!
//! The kernel command line is the kernel's own statement of what it mounted as
//! `/`. `cmd/rivet-image` writes `root=PARTLABEL=system-a` into each slot's
//! `cmdline.txt`, and `ADR-029` §3's measurement is that `cmdline.txt` is the
//! **only** file that differs between the two slots' boot partitions — so it is
//! also the one file whose contents identify a slot.
//!
//! # What this module does NOT decide
//!
//! It does not turn `PARTLABEL=system-a` into a slot. That mapping is
//! `rivet_core::state::Slot::system_partlabel`'s, in the crate that owns the
//! type, and resolving it here would put the label in a second place. What
//! leaves this module is the `root=` argument **verbatim** (`AGENTS.md` §5), so
//! a machine booted by `root=/dev/mmcblk0p3` or `root=UUID=…` reports what it
//! was actually told rather than nothing at all.

/// The kernel's own record of how it was started.
pub const CMDLINE: &str = "/proc/cmdline";

/// The `root=` argument this kernel was given, verbatim, or why there is none.
///
/// # Errors
///
/// A string saying what happened — the file could not be read, or it carries no
/// `root=` at all. **Both are answers rather than faults**: a kernel started by
/// an initramfs may legitimately carry none, and neither may be reported as a
/// slot.
pub fn root_argument() -> Result<String, String> {
    let text = std::fs::read_to_string(CMDLINE)
        .map_err(|e| format!("{CMDLINE} could not be read: {e}"))?;
    root_of(&text).ok_or_else(|| {
        format!(
            "{CMDLINE} carries no `root=` argument, so the kernel's own statement of what it \
             mounted as / is not there to read"
        )
    })
}

/// The value of the **first** `root=` on a kernel command line.
///
/// **First and not last**, which is a real choice: the kernel itself takes the
/// last occurrence of a repeated parameter, and this build cannot verify that
/// against a kernel it did not read. Taking the first and reporting it verbatim
/// makes a duplicated `root=` visible as a wrong answer rather than invisible
/// as a right one — and `cmd/rivet-image` writes exactly one, so a second is
/// already a card nobody built.
fn root_of(cmdline: &str) -> Option<String> {
    cmdline
        .split_whitespace()
        .find_map(|word| word.strip_prefix("root="))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;
    use rivet_core::state::Slot;

    /// The line `cmd/rivet-image` actually writes, for both slots.
    const A: &str = "console=serial0,115200 console=tty1 root=PARTLABEL=system-a \
                     rootfstype=erofs ro rootwait";
    const B: &str = "console=serial0,115200 console=tty1 root=PARTLABEL=system-b \
                     rootfstype=erofs ro rootwait";

    /// **The argument comes out verbatim and it names the slot the card
    /// claims.** The resolution itself is the domain's, so this test asserts
    /// the two halves meet — which is the join a unit test on either side alone
    /// cannot check.
    #[test]
    fn each_slots_own_command_line_yields_that_slots_partlabel() {
        for (line, slot) in [(A, Slot::A), (B, Slot::B)] {
            let argument = root_of(line).expect("a root= argument");
            assert_eq!(argument, format!("PARTLABEL={}", slot.system_partlabel()));
            assert!(
                !argument.contains(slot.other().system_partlabel()),
                "{argument} also matches the other slot"
            );
        }
    }

    /// **A `root=` this build does not understand still comes out whole**, so an
    /// operator sees what the kernel was told instead of an empty answer.
    #[test]
    fn a_root_argument_in_another_form_is_carried_through_unchanged() {
        for form in ["/dev/mmcblk0p3", "UUID=1234-5678", "PARTUUID=abc-03", "/dev/nfs"] {
            assert_eq!(
                root_of(&format!("quiet root={form} ro")).as_deref(),
                Some(form)
            );
        }
    }

    /// **A command line with no `root=` yields nothing**, so the caller reports
    /// it as unmeasured rather than naming a slot it did not read.
    #[test]
    fn a_command_line_with_no_root_names_no_slot() {
        for line in ["", "quiet ro", "rootfstype=erofs rootwait", "notroot=x"] {
            assert_eq!(root_of(line), None, "{line:?} was read as a root argument");
        }
    }

    /// `rootfstype=` starts with `root` and is not `root=`. The `=` is what
    /// makes the key a key — `crate::memory`'s colon, in another spelling.
    #[test]
    fn a_longer_parameter_beginning_with_root_is_not_a_root_argument() {
        assert_eq!(root_of("rootwait rootfstype=erofs"), None);
        assert_eq!(
            root_of("rootwait root=PARTLABEL=system-a rootfstype=erofs").as_deref(),
            Some("PARTLABEL=system-a")
        );
    }
}
