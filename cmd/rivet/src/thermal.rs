//! Reading this machine's heat and clock — `ADR-035` phase 4, and
//! `docs/design-candidates/Admin-Surface-Survey.md` §7.
//!
//! **`/sys/class/thermal` and `/sys/devices/system/cpu/cpu0/cpufreq`, and
//! nothing else in this commit.** `crate::journal`'s note carries why that
//! belongs here rather than behind `BootMedium`: it needs no `mount(2)`, no
//! device tree and no pid 1, so a build that refuses every other machine
//! operation still answers this one truthfully about whatever Linux it is on.
//!
//! # Why these two and not the throttle flags
//!
//! §7.6 of the survey **measured the board** rather than reasoning about it, and
//! the measurement splits the work cleanly:
//!
//! | What | Where | On the Pi 4 |
//! | :-- | :-- | :-- |
//! | SoC temperature | `/sys/class/thermal/thermal_zone0/temp` | present |
//! | current vs maximum clock | `/sys/devices/system/cpu/cpu0/cpufreq/` | present and complete |
//! | under-voltage via an `rpi_volt` hwmon | `/sys/class/hwmon` | **absent** — this kernel has no voltage driver |
//! | `get_throttled` as a file | under `/sys/devices/platform/soc` | **absent** |
//! | the firmware mailbox | `/dev/vcio` | present |
//!
//! So the two facts here are plain file reads with no privilege and no
//! dependency, **and they are the two that port to x86 unchanged** through
//! `hwmon` and `cpufreq` — which is the shape §7.2 asked for. The throttle
//! flags need an `ioctl` on `/dev/vcio` and are a separate, Pi-specific
//! adapter; until that exists this module reports them as **unmeasured**, never
//! as `false`.
//!
//! # What a below-maximum frequency does and does not mean
//!
//! §7.1 says *"a capped frequency **is** the throttle, observed rather than
//! inferred"*, and that is right about the *ceiling* and wrong about the
//! *current* clock. A governor scales an idle machine down: a Pi sitting at
//! 600 MHz against a 1500 MHz maximum is behaving correctly, not throttling. So
//! this module reports four separate numbers — current, hardware maximum,
//! hardware minimum and the **policy ceiling** (`scaling_max_freq`) — and lets
//! the domain say what each one is worth, rather than collapsing them into a
//! boolean whose most common value would be a false alarm.

use rivet_core::operation::machine::{
    CpuFrequency, MachineError, ThermalRead, ThermalZone, Throttle, UnreadableZone,
};

/// Where the zones are. `/sys/class/thermal` is the generic interface, and it
/// is what an x86 `coretemp` appears under too.
const THERMAL: &str = "/sys/class/thermal";

/// The prefix a thermal zone directory carries. `/sys/class/thermal` also holds
/// `cooling_deviceN` entries, which are fans and are not zones.
const ZONE: &str = "thermal_zone";

/// **One CPU's policy, and the answer says so.** A big.LITTLE machine has more
/// than one; the Pi 4 has one for all four cores. Naming the directory in the
/// answer is what keeps this from implying the whole package.
const CPUFREQ: &str = "/sys/devices/system/cpu/cpu0/cpufreq";

/// **What every refusal to read the flags has to say, whatever caused it.**
///
/// A sentence rather than a shrug, and the sentence has one job: stop a reader
/// taking silence for a clean bill of health. Composed onto the specific reason
/// so that *"this is not a Pi"* and *"this process may not open the mailbox"*
/// stay distinguishable — they need different next actions.
fn nothing_was_measured(why: &str) -> String {
    format!(
        "{why} NOTHING HERE SAYS THIS BOARD HAS NOT BEEN THROTTLED OR UNDER-VOLTED — it says \
         nobody could look. Under-voltage is the commonest cause of a Pi behaving strangely and \
         a leading cause of card corruption, and the sticky since-boot flags are the only record \
         of one that has already passed."
    )
}

/// This machine's zones and clock.
///
/// # Errors
///
/// If **neither** interface could be read, which on a Linux host means this is
/// not one. A machine with zones and no `cpufreq`, or the reverse, is answered
/// with the half it has and a sentence about the half it does not.
pub fn read() -> Result<ThermalRead, MachineError> {
    let (zones, zones_unreadable, listed) = zones();
    let frequency = frequency();

    // **Both halves missing is a refusal; one half missing is an answer.**
    // Reporting an empty `ThermalRead` on a Windows host would say *this machine
    // has no thermal zones and no cpufreq*, which is a claim about the hardware
    // rather than about this build's reach (`AGENTS.md` §5).
    if !listed && frequency.is_err() {
        return Err(MachineError::Refused(format!(
            "neither {THERMAL} nor {CPUFREQ} could be read, so nothing about this machine's \
             temperature or clock was measured. On Linux that means this build is not looking at \
             a machine it can read; on any other platform those interfaces do not exist at all."
        )));
    }

    let (frequency, frequency_unmeasured) = match frequency {
        Ok(f) => (Some(f), None),
        Err(why) => (None, Some(why)),
    };

    // **Never `Some(Throttle::default())`.** See `nothing_was_measured`, and
    // `ThermalRead::throttle_unmeasured` for why the distinction is the most
    // load-bearing one in this subject.
    let (throttle, throttle_unmeasured) = match throttle() {
        Ok(t) => (Some(t), None),
        Err(why) => (None, Some(nothing_was_measured(&why))),
    };

    Ok(ThermalRead {
        zones,
        zones_unreadable,
        zone_source: THERMAL.to_string(),
        frequency,
        frequency_unmeasured,
        throttle,
        throttle_unmeasured,
    })
}

/// Every thermal zone, the ones that could not be read, and **whether the
/// directory was listable at all**.
///
/// The third value is the one that matters to the caller: an empty list from a
/// directory that was read is *"this machine has no zones"*, and an empty list
/// from a directory that was not is nothing at all. Collapsing them would be the
/// failure `StorageRead::mount_table` exists to prevent.
fn zones() -> (Vec<ThermalZone>, Vec<UnreadableZone>, bool) {
    let Ok(entries) = std::fs::read_dir(THERMAL) else {
        return (Vec::new(), Vec::new(), false);
    };

    let mut found: Vec<(u32, ThermalZone)> = Vec::new();
    let mut unreadable = Vec::new();
    for entry in entries.flatten() {
        let file = entry.file_name();
        let Some(name) = file.to_str() else { continue };
        // `cooling_deviceN` lives here too and is a fan, not a temperature.
        let Some(index) = name.strip_prefix(ZONE).and_then(|n| n.parse::<u32>().ok()) else {
            continue;
        };
        let dir = format!("{THERMAL}/{name}");
        match read_number::<i64>(&format!("{dir}/temp")) {
            Ok(millidegrees) => found.push((
                index,
                ThermalZone {
                    // **The zone's own name, and the directory only as a
                    // fallback.** `type` is what the driver calls itself —
                    // `cpu_thermal`, `coretemp` — and it is the useful half.
                    name: read_text(&format!("{dir}/type")).unwrap_or_else(|_| name.to_string()),
                    millidegrees,
                },
            )),
            // **Counted, not dropped** (`AGENTS.md` §5): an answer that omitted
            // this zone would say the machine does not have one.
            Err(why) => unreadable.push(UnreadableZone {
                zone: name.to_string(),
                why,
            }),
        }
    }

    // Ascending zone index, so two readings of an unchanged machine produce the
    // same bytes — `crate::process`'s rule, and for the same reason: a
    // directory's order is the filesystem's, which nobody chose.
    found.sort_by_key(|(index, _)| *index);
    unreadable.sort_by(|a, b| a.zone.cmp(&b.zone));
    (
        found.into_iter().map(|(_, zone)| zone).collect(),
        unreadable,
        true,
    )
}

/// This CPU's clock, in **hertz**, or a sentence saying what could not be read.
///
/// `cpuinfo_max_freq` is the one that must be there: without it there is no
/// ceiling to compare against, and a current frequency on its own answers
/// nothing at all.
fn frequency() -> Result<CpuFrequency, String> {
    let khz = |name: &str| read_number::<u64>(&format!("{CPUFREQ}/{name}"));
    let maximum_hz = khz("cpuinfo_max_freq")?.saturating_mul(1000);
    let current_hz = khz("scaling_cur_freq")?.saturating_mul(1000);
    Ok(CpuFrequency {
        current_hz,
        maximum_hz,
        minimum_hz: khz("cpuinfo_min_freq").ok().map(|k| k.saturating_mul(1000)),
        ceiling_hz: khz("scaling_max_freq").ok().map(|k| k.saturating_mul(1000)),
        governor: read_text(&format!("{CPUFREQ}/scaling_governor")).ok(),
        source: CPUFREQ.to_string(),
    })
}

/// The firmware's throttle flags, or a sentence saying why there are none.
fn throttle() -> Result<Throttle, String> {
    mailbox::read()
}

/// **The firmware's throttle flags — survey §7.1's most valuable fact.**
///
/// A module of its own because every word in it is Broadcom's rather than
/// Linux's, and because the seam is worth being able to see: the two functions
/// that decide anything — `mailbox::request_message` and `mailbox::of_bits`
/// — are pure and tested on every host, and the one that touches the device is
/// six lines.
///
/// **`allow(dead_code)` off Linux, and that is the honest arrangement rather
/// than a silenced warning.** On a Windows development host there is no ioctl
/// to make, so [`mailbox::read`] refuses and the layout constants have no
/// caller in the binary — but they still have callers in the tests, which is
/// where they earn their keep: `AGENTS.md` §4 says a suite that cannot be run
/// rots, and gating this module out entirely would leave the bit assignment
/// checked only on a machine nobody runs the suite on.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod mailbox {
    use rivet_core::operation::machine::Throttle;

    /// The Broadcom firmware mailbox. Survey §7.6 measured it as **present** on
    /// the pinned kernel, alongside `vcio_gencmd` and five siblings.
    pub const VCIO: &str = "/dev/vcio";

    /// `GET_THROTTLED`, the firmware's own tag number for the flags.
    const TAG_GET_THROTTLED: u32 = 0x0003_0046;


    /// The property interface's *"this is a request"* code, in word 1.
    const REQUEST: u32 = 0x0000_0000;
    /// The firmware's *"I handled it"* code, written back into word 1.
    const SUCCEEDED: u32 = 0x8000_0000;
    /// The end-of-tag-list marker.
    const END_TAG: u32 = 0x0000_0000;
    /// The response buffer this tag needs: one word.
    const VALUE_BYTES: u32 = 4;

    /// Which word of the message the answer comes back in.
    const VALUE_WORD: usize = 5;

    /// How many words the whole exchange is: size, request code, tag, value
    /// size, request code, value, end tag.
    const MESSAGE_WORDS: usize = 7;

    /// **The eight bits, as the firmware defines them.**
    ///
    /// Written out rather than computed, because a shift arithmetic error here
    /// would report under-voltage as a temperature limit and nothing would
    /// catch it. The `_SINCE_BOOT` four are the same four conditions sixteen
    /// bits up, and they are **sticky**: once the condition has occurred they
    /// stay set for the rest of the boot, which is the whole reason this
    /// exchange is worth performing.
    const UNDER_VOLTAGE_NOW: u32 = 1 << 0;
    const FREQUENCY_CAPPED_NOW: u32 = 1 << 1;
    const THROTTLED_NOW: u32 = 1 << 2;
    const SOFT_TEMPERATURE_LIMIT_NOW: u32 = 1 << 3;
    const UNDER_VOLTAGE_SINCE_BOOT: u32 = 1 << 16;
    const FREQUENCY_CAPPED_SINCE_BOOT: u32 = 1 << 17;
    const THROTTLED_SINCE_BOOT: u32 = 1 << 18;
    const SOFT_TEMPERATURE_LIMIT_SINCE_BOOT: u32 = 1 << 19;

    /// The firmware's throttle flags, or a sentence saying why there are none.
    ///
    /// **Roughly fifty lines of Pi-specific adapter, and no `vcgencmd`.** Survey
    /// §7.1 named the usual route and rejected it: `vcgencmd` is a Broadcom
    /// userland tool this image does not carry and should not. §7.6 then
    /// measured what is actually on the board — no `rpi_volt` hwmon device, no
    /// `get_throttled` file, and `/dev/vcio` **present** — which makes the
    /// mailbox the only route rather than the convenient one.
    ///
    /// It is here rather than in `rivet-core` because every word of it is
    /// board-specific: a tag number, a bitmask and a `/dev` node are exactly the
    /// vocabulary `AGENTS.md` §3.1 forbids the domain. The domain sees eight
    /// booleans and a raw word.
    #[cfg(target_os = "linux")]
    pub fn read() -> Result<Throttle, String> {
        use std::os::fd::AsRawFd as _;

        let device = std::fs::File::open(VCIO).map_err(|e| {
            format!(
                "{VCIO} could not be opened: {e}. On a machine that is not a Raspberry Pi there \
                 is no such device; on one that is, this process may not have the privilege to \
                 open it."
            )
        })?;

        let mut message = request_message();
        crate::syscall::vcio_property(device.as_raw_fd(), &mut message)
            .map_err(|errno| format!("the property ioctl on {VCIO} failed with errno {errno}."))?;
        answer(&message).map(of_bits)
    }

    /// **No mailbox off Linux, and it says so rather than answering.**
    ///
    /// The development host is Windows (`AGENTS.md` §10.5) and `crate::syscall`
    /// is `#[cfg(target_os = "linux")]`, so there is no ioctl to make here. What
    /// must not happen is this arm returning clear flags.
    #[cfg(not(target_os = "linux"))]
    pub fn read() -> Result<Throttle, String> {
        Err(format!(
            "this build is not for Linux, so it cannot perform the property ioctl on {VCIO} that \
             the firmware's throttle flags come from."
        ))
    }

    /// The `GET_THROTTLED` request, as the firmware wants it laid out.
    ///
    /// **Pure, and separated from the ioctl so that it can be tested at all.**
    /// No development host has a `/dev/vcio`, so the exchange itself is
    /// unexercised until a board runs it — and the layout is one of the two
    /// halves where a mistake would be silent, which is exactly the half worth
    /// pulling out.
    fn request_message() -> [u32; MESSAGE_WORDS] {
        let mut message = [
            0, // total size in BYTES, filled in below from the buffer's length
            REQUEST,
            TAG_GET_THROTTLED,
            VALUE_BYTES,
            REQUEST,
            0, // where the answer lands
            END_TAG,
        ];
        // From the buffer rather than written out: the kernel's vcio driver
        // bounds its copy by this word, so a literal that drifted from the
        // array's length would either truncate the message or over-read it.
        message[0] = u32::try_from(size_of_val(&message)).unwrap_or(0);
        message
    }

    /// The firmware's answer, or why the buffer that came back is not one.
    ///
    /// **The success code is checked, and that check is the point.** A buffer
    /// the firmware never touched still holds a zero in [`VALUE_WORD`], and
    /// reading it would report a board with every flag clear — the exact false
    /// clean bill of health this whole subject exists to prevent
    /// (`AGENTS.md` §5: refuse rather than guess).
    fn answer(message: &[u32; MESSAGE_WORDS]) -> Result<u32, String> {
        if message[1] == SUCCEEDED {
            return Ok(message[VALUE_WORD]);
        }
        Err(format!(
            "{VCIO} answered {:#010x} rather than {SUCCEEDED:#010x}, so the firmware did not \
             handle the GET_THROTTLED request and word {VALUE_WORD} of the buffer is not an \
             answer.",
            message[1]
        ))
    }

    /// The firmware's word, as eight booleans and itself.
    ///
    /// The **other** half worth pulling out of the ioctl: a shift arithmetic
    /// error here would report under-voltage as a temperature limit, and nothing
    /// else in the system would catch it.
    fn of_bits(raw: u32) -> Throttle {
        let set = |bit: u32| raw & bit != 0;
        Throttle {
            under_voltage_now: set(UNDER_VOLTAGE_NOW),
            frequency_capped_now: set(FREQUENCY_CAPPED_NOW),
            throttled_now: set(THROTTLED_NOW),
            soft_temperature_limit_now: set(SOFT_TEMPERATURE_LIMIT_NOW),
            under_voltage_since_boot: set(UNDER_VOLTAGE_SINCE_BOOT),
            frequency_capped_since_boot: set(FREQUENCY_CAPPED_SINCE_BOOT),
            throttled_since_boot: set(THROTTLED_SINCE_BOOT),
            soft_temperature_limit_since_boot: set(SOFT_TEMPERATURE_LIMIT_SINCE_BOOT),
            // **Kept whole** (`AGENTS.md` §5). A bit this build does not know
            // about is invisible in the eight booleans and visible here, which
            // is the difference between an answer that can be checked against
            // the vendor's documentation and one that cannot.
            raw,
            source: VCIO.to_string(),
        }
    }

    #[cfg(test)]
    mod tests {
        // A test that cannot fail loudly is not a test.
        #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

        use super::*;

        /// The eight flags in the order the eight bits are declared, so that a
        /// bit can be checked against *every* flag rather than only the one it
        /// was meant to set.
        fn flags_of(t: &Throttle) -> [bool; 8] {
            [
                t.under_voltage_now,
                t.frequency_capped_now,
                t.throttled_now,
                t.soft_temperature_limit_now,
                t.under_voltage_since_boot,
                t.frequency_capped_since_boot,
                t.throttled_since_boot,
                t.soft_temperature_limit_since_boot,
            ]
        }

        const BITS: [u32; 8] = [
            UNDER_VOLTAGE_NOW,
            FREQUENCY_CAPPED_NOW,
            THROTTLED_NOW,
            SOFT_TEMPERATURE_LIMIT_NOW,
            UNDER_VOLTAGE_SINCE_BOOT,
            FREQUENCY_CAPPED_SINCE_BOOT,
            THROTTLED_SINCE_BOOT,
            SOFT_TEMPERATURE_LIMIT_SINCE_BOOT,
        ];

        /// **Every one of the eight bits sets its own flag and no other.**
        ///
        /// A shift arithmetic error would report under-voltage as a temperature
        /// limit and nothing else in the system would catch it, so each bit is
        /// set alone and *all eight* flags are checked rather than the one it
        /// was meant to set.
        #[test]
        fn each_firmware_bit_sets_exactly_one_flag() {
            for (position, bit) in BITS.into_iter().enumerate() {
                let flags = flags_of(&of_bits(bit));
                for (other, set) in flags.into_iter().enumerate() {
                    assert_eq!(
                        set,
                        other == position,
                        "{bit:#x} should set flag {position} and only that one"
                    );
                }
            }
        }

        /// **The `_now` and `_since_boot` halves are independent**, which is the
        /// whole reason the sticky half is worth reading: a board that is fine
        /// now and was under-volted at 03:00 comes back with the first four
        /// clear and the sticky ones set.
        #[test]
        fn a_board_that_is_fine_now_still_reports_what_happened_earlier() {
            let flags = of_bits(UNDER_VOLTAGE_SINCE_BOOT | THROTTLED_SINCE_BOOT);
            assert!(!flags.under_voltage_now, "nothing is wrong right now");
            assert!(!flags.throttled_now);
            assert!(
                flags.under_voltage_since_boot,
                "and this is the fact nothing else on the machine recovers"
            );
            assert!(flags.throttled_since_boot);
        }

        /// A firmware word carrying a bit this build does not know about is
        /// still reported whole, so the answer can be checked against the
        /// vendor's documentation rather than only against this file.
        #[test]
        fn an_unknown_bit_survives_in_the_raw_word() {
            let flags = of_bits(1 << 30);
            assert_eq!(flags.raw, 1 << 30);
            assert!(!flags.under_voltage_now, "an unknown bit set a known flag");
            assert_eq!(flags.source, VCIO);
        }

        /// All-clear is all-clear, and it is a **different answer** from the
        /// flags being absent — which is what `ThermalRead::throttle_unmeasured`
        /// carries.
        #[test]
        fn a_clean_board_reports_eight_false_flags_and_a_zero_word() {
            assert_eq!(
                of_bits(0),
                Throttle {
                    raw: 0,
                    source: VCIO.to_string(),
                    ..Throttle::default()
                }
            );
        }

        /// **The request the firmware is handed, checked word by word.** A wrong
        /// tag number returns a different property and a wrong size word makes
        /// the driver truncate or over-read.
        #[test]
        fn the_request_asks_for_get_throttled_and_states_its_own_length() {
            let message = request_message();
            assert_eq!(
                message[0] as usize,
                MESSAGE_WORDS * size_of::<u32>(),
                "the size word must be the buffer's own byte length"
            );
            assert_eq!(message[1], REQUEST, "word 1 says this is a request");
            assert_eq!(message[2], TAG_GET_THROTTLED, "0x00030046 is GET_THROTTLED");
            assert_eq!(message[3], VALUE_BYTES, "the answer is one word wide");
            assert_eq!(message[VALUE_WORD], 0, "nothing is asserted before the call");
            assert_eq!(message[MESSAGE_WORDS - 1], END_TAG);
        }

        /// **A buffer the firmware never touched is refused, not read.** It
        /// still holds a zero where the answer goes, and reading that would
        /// report a board with every flag clear — the false clean bill of health
        /// this subject exists to prevent.
        #[test]
        fn an_untouched_buffer_is_refused_rather_than_read_as_a_clean_board() {
            let why = answer(&request_message())
                .expect_err("a buffer with no success code is not an answer");
            assert!(why.contains(VCIO), "{why}");
            assert!(why.contains("did not handle"), "{why}");
        }

        /// And a buffer the firmware did handle yields the word it wrote.
        #[test]
        fn a_handled_request_yields_the_word_the_firmware_wrote() {
            let mut message = request_message();
            message[1] = SUCCEEDED;
            message[VALUE_WORD] = UNDER_VOLTAGE_SINCE_BOOT;
            assert_eq!(answer(&message), Ok(UNDER_VOLTAGE_SINCE_BOOT));
        }
    }
}


/// One sysfs file, trimmed.
fn read_text(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map(|s| s.trim().to_string())
        .map_err(|e| format!("{path} could not be read: {e}"))
}

/// One sysfs file holding a single number.
///
/// **The refusal names the file and what was in it**, because the two failures
/// here are very different: a file that is not there is a machine without the
/// interface, and a file holding something unparseable is an interface this
/// build has misunderstood.
fn read_number<T: std::str::FromStr>(path: &str) -> Result<T, String> {
    let text = read_text(path)?;
    text.parse::<T>()
        .map_err(|_| format!("{path} holds {text:?}, which is not the number this expected"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use super::*;

    /// The unmeasured sentence has to say the thing that is easy to get wrong,
    /// because a reader who takes silence for *"not throttled"* has read this
    /// subject backwards. **It also has to keep the specific cause**, because
    /// *"this is not a Pi"* and *"this process may not open the mailbox"* need
    /// different next actions.
    #[test]
    fn the_missing_flags_sentence_says_nobody_could_look_and_keeps_the_reason() {
        let said = nothing_was_measured("errno 13 opening the mailbox.");
        assert!(said.contains("errno 13"), "the specific cause was lost: {said}");
        assert!(said.contains("nobody could look"), "{said}");
        assert!(
            said.contains("NOTHING HERE SAYS"),
            "the sentence must refuse the inference explicitly: {said}"
        );
    }

    /// A number that is not a number is refused **naming the file and the
    /// contents**, so that an interface this build has misread is
    /// distinguishable from one the machine does not have.
    #[test]
    fn an_unparseable_sysfs_value_is_refused_naming_what_was_there() {
        let why = read_number::<u64>("/nonexistent/rivet-thermal-test/temp")
            .expect_err("a file that is not there cannot be a number");
        assert!(why.contains("could not be read"), "{why}");
    }

    /// **The read either answers or refuses; it never invents flags.** This runs
    /// on whatever host the suite is on — Linux answers, Windows refuses — and
    /// the property asserted is the one that must hold on both.
    #[test]
    fn a_reading_on_this_host_never_carries_throttle_flags_this_build_did_not_measure() {
        match read() {
            Ok(reading) => {
                assert!(
                    reading.throttle.is_none(),
                    "flags were reported by a build that reads no mailbox"
                );
                assert!(
                    reading.throttle_unmeasured.is_some(),
                    "absent flags must carry a reason"
                );
                assert_eq!(reading.zone_source, THERMAL);
                assert_eq!(
                    reading.frequency.is_none(),
                    reading.frequency_unmeasured.is_some(),
                    "exactly one of the frequency and its absence-reason is present"
                );
            }
            Err(refusal) => {
                assert!(refusal.why().contains(THERMAL), "{}", refusal.why());
                assert!(refusal.why().contains(CPUFREQ), "{}", refusal.why());
            }
        }
    }
}
