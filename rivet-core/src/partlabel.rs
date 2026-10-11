//! Finding a partition by the **label `cmd/rivet-image` wrote on it**, using
//! the kernel's own account of the disk rather than a name for the medium.
//!
//! # The question this answers, and why it was open
//!
//! `ADR-018` §4's five partitions are identified two different ways in this
//! repository, and until 2026-08-28 the two disagreed with each other.
//!
//! `cmd/rivet-image`'s `boot::root_spec` writes `root=PARTLABEL=system-a` into
//! `cmdline.txt`, and its comment gives the argument in one line: a device path
//! *"names the medium"*, and *"the same card in a USB enclosure is
//! `/dev/sda3`"*. So `/system` — the filesystem the kernel has to find before
//! anything of Rivet's runs at all — is named by **label**.
//!
//! `/state` was named by **device path**: [`crate::state::state_device`]
//! composed `/dev/mmcblk0` with a partition number, and
//! [`crate::state::STATE_PARTITION`] carried the reason a label was not used:
//!
//! > `/dev/disk/by-partlabel/` is **udev's**, and `ADR-005` §2.1 rules that
//! > Rivet has no udev. Nothing on a Rivet board creates those symlinks.
//! > Resolving a `PARTLABEL` without udev means parsing the GPT out of the raw
//! > disk, which is real machinery in the process that must not fail.
//!
//! Both halves of that are true and the conclusion does not follow, because
//! there is a third way to resolve a label that is neither udev nor a GPT
//! parser: **ask the kernel, which has already parsed it.** Read out of the
//! tree `tools/buildroot/build.sh` builds (Linux 6.12.61):
//!
//! | Source | What it does |
//! | :-- | :-- |
//! | `block/partitions/efi.c:750` | `utf16_le_to_7bit(ptes[i].partition_name, …, info->volname)` — the GPT parser keeps the label |
//! | `block/partitions/core.c:353` | that `partition_meta_info` is copied onto the `block_device` as `bd_meta_info` |
//! | `block/partitions/core.c:254` | `add_uevent_var(env, "PARTNAME=%s", …)` — and the label is in the partition's `uevent` |
//! | `block/early-lookup.c:107` | the same field is what resolves `root=PARTLABEL=` before any userspace exists |
//!
//! The last row is the one that settles it: the kernel resolves a `PARTLABEL`
//! for the root filesystem **with no userspace running at all**, so a design
//! that treats labels as needing udev has already been contradicted by the
//! `cmdline.txt` this project writes.
//!
//! A sysfs `uevent` file is *generated when it is read* — it is not a record
//! left behind by a hotplug event and it does not require anything to have
//! processed one. That is what makes this available to PID 1 on a machine with
//! no udev, no `mdev`, and nothing else populating `/dev` but devtmpfs.
//!
//! # What this module is, and what it deliberately is not
//!
//! It is the **decision** and the **parsing**, with no I/O, for
//! `crate::mount`'s reason: the rules are then testable without root, without a
//! card and without a kernel. Reading the directory and the files is the
//! caller's — `cmd/rivet-initd`'s `state` module does it through its `Kernel`
//! port.
//!
//! It is **not** a general block-device library. There is one question here —
//! *which partition carries this label* — and three answers, one of which is
//! "more than one, and I will not choose" (`AGENTS.md` §5).

/// Where the kernel lists every block device and every partition of one.
///
/// `/sys/class/block` rather than `/sys/block`, because the latter lists whole
/// disks and reaches partitions only as children — so finding a partition means
/// walking two levels and knowing which entries are disks. This directory has
/// every one of them at the top level, disks and partitions alike, and the
/// partitions are exactly the entries whose `uevent` carries a `PARTN`.
pub const SYS_BLOCK_DIR: &str = "/sys/class/block";

/// The file inside each of those entries that the kernel writes on demand.
pub const UEVENT_FILE: &str = "uevent";

/// Where devtmpfs puts the node for a name the kernel reports as `DEVNAME`.
const DEV_DIR: &str = "/dev";

/// The `uevent` key carrying a GPT partition's label. `block/partitions/core.c`.
pub const PARTNAME: &str = "PARTNAME";

/// The `uevent` key carrying a partition's 1-based number.
pub const PARTN: &str = "PARTN";

/// The `uevent` key carrying the name devtmpfs makes the node under.
pub const DEVNAME: &str = "DEVNAME";

/// The `uevent` file of one entry of [`SYS_BLOCK_DIR`].
pub fn uevent_path(entry: &str) -> String {
    format!("{SYS_BLOCK_DIR}/{entry}/{UEVENT_FILE}")
}

/// The `uevent` of the **whole disk** an entry is a partition of.
///
/// `/sys/class/block/vda5` is a symlink into `/sys/devices/…/vda/vda5`, and a
/// path resolves the symlink before it applies `..` — so `..` from there is the
/// disk's own directory, not `/sys/class/block`. That is what makes this one
/// string instead of a name-mangling rule: `vda5` → `vda` and `mmcblk0p5` →
/// `mmcblk0` are different rules, and the kernel already knows the answer to
/// both.
///
/// **Added 2026-08-28 for `PLAN.md` §62.7.** `cmd/rivet-sysd` rewrites a GPT,
/// which means it needs the disk rather than the partition, and the disk it
/// must never get wrong is the one `/state` is on.
///
/// Read [`DEVNAME`] out of the result to get the disk's node.
pub fn containing_disk_uevent_path(entry: &str) -> String {
    format!("{SYS_BLOCK_DIR}/{entry}/../{UEVENT_FILE}")
}

/// The sysfs entry name for a device node — `/dev/vda5` → `vda5`.
///
/// **Refuses anything that is not a direct child of `/dev`.** A device-mapper
/// node is `/dev/mapper/x`, and composing `/sys/class/block/mapper/x/uevent`
/// from it would silently read nothing; `None` sends the caller to report that
/// it could not identify the device, which is the honest answer (`AGENTS.md`
/// §5).
pub fn entry_of_device(device: &str) -> Option<&str> {
    let rest = device.strip_prefix(DEV_DIR)?.strip_prefix('/')?;
    (!rest.is_empty() && !rest.contains('/')).then_some(rest)
}

/// The device node for a name the kernel reported as [`DEVNAME`].
///
/// **Not composed from the sysfs entry name**, even though the two are equal
/// for every device this system has. `DEVNAME` is what devtmpfs itself uses to
/// make the node, so it is the kernel's answer to *"what is this called under
/// `/dev`"*; the directory entry is the kernel's answer to a different question
/// and happens to agree. `AGENTS.md` §5 — the two are only the same until they
/// are not, and a device-mapper name is the standing counter-example.
pub fn device_node(devname: &str) -> String {
    format!("{DEV_DIR}/{devname}")
}

/// One `key=value` line of a `uevent`, or `None`.
///
/// The format is one `KEY=value` per line with no quoting, no continuations and
/// no comments — it is built by `add_uevent_var` and read back verbatim. A
/// value may contain `=`, so the split is on the **first** one only.
///
/// Matching is on the whole key, not a prefix: `PARTN` and `PARTNAME` differ by
/// a suffix, and a prefix match for the shorter would answer the longer one's
/// question with a partition label where a number was wanted.
pub fn field<'u>(uevent: &'u str, key: &str) -> Option<&'u str> {
    uevent.lines().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        (k == key).then_some(v)
    })
}

/// What a search of [`SYS_BLOCK_DIR`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Located {
    /// Exactly one partition carries the label.
    At(Partition),
    /// Nothing carries it. Not an error here — the caller decides what an
    /// absent label is worth, and for `/state` it is a fall back to the
    /// composed device path rather than a failure.
    Nowhere,
    /// **More than one partition carries it, and this refuses to choose.**
    ///
    /// Two cards written by `cmd/rivet-image` attached to one machine is not a
    /// strange situation — it is a card and its replacement, or a board with
    /// the card it boots from and a second one being written. Picking either
    /// one would mount somebody's data as this machine's `/state`, and the
    /// wrong choice is silent and permanent. `AGENTS.md` §5.
    ///
    /// The device nodes, sorted, so a console message is stable between boots
    /// (directory order is not).
    Ambiguous(Vec<String>),
}

/// A partition the kernel is reporting, as much of it as the `uevent` said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    /// The sysfs entry it was found under, e.g. `vda5`.
    pub entry: String,
    /// The device node, from [`DEVNAME`] — e.g. `/dev/vda5`.
    pub device: String,
    /// Its 1-based number, from [`PARTN`], if the kernel gave one.
    ///
    /// `Option` because the caller must not be able to confuse *"the kernel did
    /// not say"* with a number. Nothing here needs it to work; it exists so a
    /// console can report a partition found at a number other than the one
    /// `ADR-018` §4 puts the label on, which is a card worth looking at.
    pub number: Option<u32>,
}

/// Which of these entries carries `label`.
///
/// Takes what was read rather than reading it, so that every branch below —
/// including the one that refuses — is reachable in a test with no card.
/// `entries` is `(sysfs entry name, contents of its uevent)`; an entry whose
/// `uevent` could not be read is simply not passed in, because "unreadable" and
/// "not this one" are the same fact to this decision and the caller is where
/// the difference can be reported.
///
/// An entry that matches the label but reports no [`DEVNAME`] is **skipped**
/// rather than guessed at. That cannot happen on a partition — `dev_uevent`
/// writes `DEVNAME` for everything devtmpfs makes a node for — and if it ever
/// does, composing a path from the sysfs entry name would be inventing the one
/// value this whole module exists to avoid inventing.
pub fn find(label: &str, entries: &[(String, String)]) -> Located {
    let mut hits: Vec<Partition> = entries
        .iter()
        .filter(|(_, uevent)| field(uevent, PARTNAME) == Some(label))
        .filter_map(|(entry, uevent)| {
            let devname = field(uevent, DEVNAME)?;
            Some(Partition {
                entry: entry.clone(),
                device: device_node(devname),
                number: field(uevent, PARTN).and_then(|n| n.parse().ok()),
            })
        })
        .collect();

    match hits.len() {
        0 => Located::Nowhere,
        1 => Located::At(hits.remove(0)),
        _ => {
            let mut devices: Vec<String> = hits.into_iter().map(|p| p.device).collect();
            devices.sort();
            Located::Ambiguous(devices)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real `uevent`, byte for byte, from a partition of a GPT-labelled disk.
    fn uevent(devname: &str, partn: u32, partname: &str) -> String {
        format!(
            "MAJOR=254\nMINOR={partn}\nDEVNAME={devname}\nDEVTYPE=partition\n\
             DISKSEQ=1\nPARTN={partn}\nPARTNAME={partname}\n"
        )
    }

    /// A whole disk's, which has neither of the two keys that matter.
    fn disk_uevent(devname: &str) -> String {
        format!("MAJOR=254\nMINOR=0\nDEVNAME={devname}\nDEVTYPE=disk\nDISKSEQ=1\n")
    }

    fn entries(v: &[(&str, String)]) -> Vec<(String, String)> {
        v.iter().map(|(n, u)| ((*n).to_string(), u.clone())).collect()
    }

    // -- the paths ---------------------------------------------------------

    #[test]
    fn the_uevent_path_is_the_entrys_own_file_under_sys_class_block() {
        assert_eq!(uevent_path("vda5"), "/sys/class/block/vda5/uevent");
        assert_eq!(uevent_path("mmcblk0p5"), "/sys/class/block/mmcblk0p5/uevent");
    }

    #[test]
    fn a_device_node_is_devname_under_dev_and_nothing_is_composed() {
        assert_eq!(device_node("vda5"), "/dev/vda5");
        assert_eq!(device_node("mmcblk0p5"), "/dev/mmcblk0p5");
    }

    // -- the disk a partition belongs to -----------------------------------

    /// The `..` is doing the work and it is worth an assertion of its own: it
    /// is applied AFTER the symlink resolves, so it lands on the disk rather
    /// than back in `/sys/class/block`.
    #[test]
    fn a_partitions_disk_is_reached_through_the_entry_rather_than_by_name_mangling() {
        assert_eq!(
            containing_disk_uevent_path("vda5"),
            "/sys/class/block/vda5/../uevent"
        );
        assert_eq!(
            containing_disk_uevent_path("mmcblk0p5"),
            "/sys/class/block/mmcblk0p5/../uevent"
        );
    }

    /// The reason it is a path and not a rule: the two names this system
    /// actually meets need two different manglings, and this needs neither.
    #[test]
    fn the_two_naming_conventions_need_no_special_case_here() {
        let virtio = containing_disk_uevent_path("vda5");
        let card = containing_disk_uevent_path("mmcblk0p5");
        assert!(!virtio.contains("vda/"), "{virtio}");
        assert!(!card.contains("mmcblk0/"), "{card}");
    }

    // -- a device node back to a sysfs entry -------------------------------

    #[test]
    fn a_device_node_maps_back_to_the_entry_it_was_made_from() {
        assert_eq!(entry_of_device("/dev/vda5"), Some("vda5"));
        assert_eq!(entry_of_device("/dev/mmcblk0p5"), Some("mmcblk0p5"));
        assert_eq!(entry_of_device("/dev/sda"), Some("sda"));
    }

    /// Refused rather than mangled. `/sys/class/block/mapper/rivet/uevent` does
    /// not exist, so composing it would read nothing and the caller would have
    /// to guess why.
    #[test]
    fn anything_that_is_not_a_direct_child_of_dev_is_refused() {
        for bad in [
            "/dev/mapper/rivet",
            "/dev/",
            "/dev",
            "vda5",
            "/devices/vda5",
            "",
            "tmpfs",
        ] {
            assert_eq!(entry_of_device(bad), None, "{bad} should be refused");
        }
    }

    /// The round trip both directions, which is what the grow relies on.
    #[test]
    fn a_node_and_its_entry_round_trip() {
        for entry in ["vda5", "mmcblk0p5", "sda1"] {
            assert_eq!(entry_of_device(&device_node(entry)), Some(entry));
        }
    }

    // -- parsing one line --------------------------------------------------

    #[test]
    fn a_field_is_read_by_its_whole_key() {
        let u = uevent("vda5", 5, "state");
        assert_eq!(field(&u, PARTNAME), Some("state"));
        assert_eq!(field(&u, PARTN), Some("5"));
        assert_eq!(field(&u, DEVNAME), Some("vda5"));
        assert_eq!(field(&u, "NOTHING"), None);
    }

    /// The reason the match is on the whole key. `PARTN` is a prefix of
    /// `PARTNAME`, and a prefix match would answer *"what number"* with
    /// *"state"* on every GPT partition in existence.
    #[test]
    fn partn_does_not_match_partname_although_it_is_a_prefix_of_it() {
        // PARTNAME first, so a prefix search would reach it before PARTN.
        let u = "PARTNAME=state\nPARTN=5\n";
        assert_eq!(field(u, PARTN), Some("5"));
        assert_eq!(field(u, PARTNAME), Some("state"));
    }

    /// A GPT label may contain `=`; the split is on the first one only.
    #[test]
    fn a_value_containing_an_equals_sign_survives_intact() {
        assert_eq!(field("PARTNAME=a=b\n", PARTNAME), Some("a=b"));
    }

    #[test]
    fn a_uevent_with_no_trailing_newline_still_parses() {
        assert_eq!(field("DEVNAME=vda5", DEVNAME), Some("vda5"));
    }

    // -- the search --------------------------------------------------------

    #[test]
    fn one_partition_with_the_label_is_found_with_its_node_and_number() {
        let e = entries(&[
            ("vda", disk_uevent("vda")),
            ("vda1", uevent("vda1", 1, "boot-a")),
            ("vda5", uevent("vda5", 5, "state")),
        ]);
        assert_eq!(
            find("state", &e),
            Located::At(Partition {
                entry: "vda5".to_string(),
                device: "/dev/vda5".to_string(),
                number: Some(5),
            })
        );
    }

    /// The board's names, which differ from the emulator's in exactly the way
    /// this module exists for: same label, same search, different node.
    #[test]
    fn the_same_search_finds_the_cards_partition_under_its_own_name() {
        let e = entries(&[
            ("mmcblk0", disk_uevent("mmcblk0")),
            ("mmcblk0p5", uevent("mmcblk0p5", 5, "state")),
        ]);
        assert_eq!(
            find("state", &e),
            Located::At(Partition {
                entry: "mmcblk0p5".to_string(),
                device: "/dev/mmcblk0p5".to_string(),
                number: Some(5),
            })
        );
    }

    #[test]
    fn a_label_no_partition_carries_is_nowhere_rather_than_an_error() {
        let e = entries(&[
            ("vda", disk_uevent("vda")),
            ("vda1", uevent("vda1", 1, "boot-a")),
        ]);
        assert_eq!(find("state", &e), Located::Nowhere);
    }

    #[test]
    fn a_machine_with_no_block_devices_at_all_is_nowhere() {
        assert_eq!(find("state", &[]), Located::Nowhere);
    }

    /// Two cards, and the whole point of the enum's third arm. Choosing either
    /// mounts somebody else's data as this machine's `/state`.
    #[test]
    fn two_partitions_with_the_label_refuse_to_be_chosen_between() {
        let e = entries(&[
            ("vdb5", uevent("vdb5", 5, "state")),
            ("vda5", uevent("vda5", 5, "state")),
        ]);
        assert_eq!(
            find("state", &e),
            Located::Ambiguous(vec!["/dev/vda5".to_string(), "/dev/vdb5".to_string()])
        );
    }

    /// The list is sorted, because `read_dir` order is not stable and a console
    /// message that reorders itself between boots reads as two different faults.
    #[test]
    fn an_ambiguous_answer_is_sorted_whatever_order_it_was_read_in() {
        let forwards = entries(&[
            ("vda5", uevent("vda5", 5, "state")),
            ("vdb5", uevent("vdb5", 5, "state")),
        ]);
        let backwards = entries(&[
            ("vdb5", uevent("vdb5", 5, "state")),
            ("vda5", uevent("vda5", 5, "state")),
        ]);
        assert_eq!(find("state", &forwards), find("state", &backwards));
    }

    /// A whole disk has no `PARTNAME`, so it can never be selected — including
    /// the case where a disk's `DEVNAME` happens to equal the label sought.
    #[test]
    fn a_whole_disk_is_never_selected_however_it_is_named() {
        let e = entries(&[("state", disk_uevent("state"))]);
        assert_eq!(find("state", &e), Located::Nowhere);
    }

    /// The label is matched exactly. `ADR-018` §4 has `state`, `system-a` and
    /// `system-b` on one card, and a prefix or substring match would make
    /// `system-a` answer a search for `system`.
    #[test]
    fn the_label_is_matched_exactly_and_not_as_a_prefix() {
        let e = entries(&[
            ("vda3", uevent("vda3", 3, "system-a")),
            ("vda4", uevent("vda4", 4, "system-b")),
        ]);
        assert_eq!(find("system", &e), Located::Nowhere);
        assert!(matches!(find("system-a", &e), Located::At(_)));
    }

    /// Skipped rather than guessed at — see [`find`]'s documentation.
    #[test]
    fn a_match_that_reports_no_devname_is_skipped_rather_than_composed() {
        let e = entries(&[("vda5", "PARTN=5\nPARTNAME=state\n".to_string())]);
        assert_eq!(find("state", &e), Located::Nowhere);
    }

    /// The number is reported when the kernel gives one and is `None` when it
    /// does not, so nothing downstream can read an absence as a partition 0.
    #[test]
    fn a_missing_or_unparseable_number_is_none_and_not_a_default() {
        let e = entries(&[("vda5", "DEVNAME=vda5\nPARTNAME=state\n".to_string())]);
        assert!(matches!(find("state", &e), Located::At(p) if p.number.is_none()));

        let e = entries(&[(
            "vda5",
            "DEVNAME=vda5\nPARTN=five\nPARTNAME=state\n".to_string(),
        )]);
        assert!(matches!(find("state", &e), Located::At(p) if p.number.is_none()));
    }
}
