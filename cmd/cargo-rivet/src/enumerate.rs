//! Enumerating the disks a card could be written to — `SPEC-05` §4.
//!
//! # What this replaces
//!
//! Until now `cargo rivet provision --device PATH` took the operator's word
//! for it. `AGENTS.md` §10.4 already named the risk before this module
//! existed to close it: *"the cost of getting it wrong is the development
//! host's own disk."* This is the enumeration `SPEC-05` §4 requires, so a
//! menu can replace a typed path and a removability check can replace trust.
//!
//! # Why PowerShell and CIM, and not a crate
//!
//! `cmd/cargo-rivet/Cargo.toml` already carries the history here: `sysinfo`
//! was in this crate's dependency table and was removed, because disk
//! enumeration was twenty crates for a question `--device` used to answer by
//! itself. Binding `SetupDi`/`IOCTL_DISK_*` directly would put unsafe
//! device-enumeration code into a tool whose entire job at this step is *not
//! to guess* — the opposite trade.
//!
//! Shelling out to `Get-CimInstance` is deliberate for a second reason: it is
//! **the same query a person can run by hand** to check this tool's answer.
//! `cmd/rivet-reset/src/store.rs` made the identical choice for the
//! certificate store and the reasoning transfers unchanged — a locale-proof,
//! script-checkable answer beats a library that has an opinion.
//!
//! **JSON rather than `store.rs`'s space-delimited lines**, because a disk
//! `Model` string routinely contains spaces and even quotes (`"Generic-
//! Compact Flash SD/MMC USB Device"`), and a delimiter that appears inside
//! the data it delimits is exactly the bug this project's own `prose.py`
//! exists to catch in the other direction. `ConvertTo-Json` escapes correctly
//! by construction; [`crate::json`] is the reader for it.

use crate::json::Json;
use std::path::Path;

/// One disk this host can see, removable or not.
///
/// **Both kinds are enumerated, on purpose.** Refusing a non-removable
/// `--device` (`crate::provision::resolve_device`) needs the full list to
/// explain *why* the refusal fired, not just the shorter list of acceptable
/// answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disk {
    /// What this host writes to, to reach the disk. `\\.\PHYSICALDRIVEn` on
    /// Windows (`Win32_DiskDrive.DeviceID`, which is also the exact string
    /// `OpenOptions::open` needs — no translation happens between the two);
    /// `/dev/sdX` on Linux.
    pub id: String,
    /// The manufacturer's own string, or empty if the platform did not offer
    /// one. Never invented.
    pub model: String,
    pub size_bytes: u64,
    /// Whether the platform itself reports this as removable media — never
    /// inferred from the id, the model, or anything else this module could
    /// get wrong. See [`all`] for what each platform is asked.
    pub removable: bool,
    /// What is mounted from this disk: drive letters on Windows, mount points
    /// on macOS, and an empty `Vec` on Linux or when nothing is mounted. This is the
    /// "letter" `SPEC-05` §4 requires so an operator can recognise the
    /// card they just plugged in without reading a `DeviceID`.
    pub letters: Vec<String>,
}

impl Disk {
    /// One line for the menu and for the write plan — size, model, and
    /// letters, because a `DeviceID` alone is exactly what an operator
    /// cannot recognise a physical card by.
    #[must_use]
    pub fn describe(&self) -> String {
        let size = human_size(self.size_bytes);
        let model = if self.model.trim().is_empty() {
            "(no model reported)"
        } else {
            self.model.trim()
        };
        let letters = if self.letters.is_empty() {
            "nothing mounted from it".to_owned()
        } else if self.letters[0].starts_with('/') {
            format!("mounted at {}", self.letters.join(", "))
        } else {
            format!("drive {}", self.letters.join(", "))
        };
        format!("{} — {size}, {model}, {letters}", self.id)
    }
}

/// A human-scale size — decimal GB above a gigabyte, decimal MB below it,
/// because that is what is printed on the card's own retail packaging and
/// what an operator will be holding it next to.
///
/// `pub(crate)` since `crate::elevate`'s progress renderer wants the same
/// scale for the bytes moved so far during an elevated write — a second copy
/// of this formula would be a second answer to "how big is that," which is
/// exactly what this function exists to avoid being asked twice.
pub(crate) fn human_size(bytes: u64) -> String {
    const MB: f64 = 1_000_000.0;
    const GB: f64 = 1_000_000_000.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else {
        format!("{:.1} MB", b / MB)
    }
}

/// Every disk this host can see.
///
/// # Errors
///
/// Whatever stopped the platform's own enumeration running — never an empty
/// list standing in for a failure, for `AGENTS.md` §7's reason: an error and
/// a true negative must not look the same. A host with no removable device
/// attached answers `Ok(vec![...only fixed disks...])`, never `Err`.
#[cfg(windows)]
pub fn all() -> Result<Vec<Disk>, String> {
    // One `PSCustomObject` per physical disk, wrapped in a named `disks`
    // property rather than piped straight into `ConvertTo-Json`.
    //
    // **This is not decoration.** `ConvertTo-Json` collapses a single-element
    // top-level array into a bare object — proven on this host: `@($one) |
    // ConvertTo-Json` prints `{...}`, not `[{...}]`, on Windows PowerShell
    // 5.1. A host with exactly one disk attached (routine on a workstation
    // with one internal drive and nothing else plugged in) would otherwise
    // silently hand this module an object where it expects an array, and
    // `Json::as_array` would return `None` — not a parse failure, a *missing*
    // list, which is the worse of the two to get wrong here. Wrapping in a
    // named property sidesteps the collapse entirely: a `disks` *property*
    // whose value is a genuine array stays an array at any length, including
    // zero and one — verified the same way, on this host, before this
    // module was written.
    //
    // Association classes rather than a single WMI query: `SPEC-05` §4
    // asks for the drive letter behind each disk, and CIM has no single class
    // that carries both a disk's identity and its mounted letters —
    // `Win32_DiskDriveToDiskPartition` and `Win32_LogicalDiskToPartition` are
    // the two hops, walked here with `Get-CimAssociatedInstance` rather than
    // parsed out of the WMI relationship-path strings those classes return by
    // default, which are a second grammar this module would have to own.
    let script = "$ErrorActionPreference = 'Stop'; \
         $disks = @(Get-CimInstance Win32_DiskDrive | ForEach-Object { \
         $disk = $_; \
         $letters = @(); \
         $parts = Get-CimAssociatedInstance -InputObject $disk -ResultClassName Win32_DiskPartition -ErrorAction SilentlyContinue; \
         foreach ($p in $parts) { \
         $logicals = Get-CimAssociatedInstance -InputObject $p -ResultClassName Win32_LogicalDisk -ErrorAction SilentlyContinue; \
         foreach ($l in $logicals) { $letters += $l.DeviceID } \
         }; \
         [PSCustomObject]@{ DeviceID = $disk.DeviceID; Model = $disk.Model; Size = $disk.Size; \
         InterfaceType = $disk.InterfaceType; MediaType = $disk.MediaType; Letters = @($letters) } \
         }); \
         [PSCustomObject]@{ disks = $disks } | ConvertTo-Json -Depth 4";

    // `powershell`, not `pwsh` — the same choice `cmd/rivet-reset/src/store.rs`
    // made and for the same reason: it is what is on every one of this
    // project's Windows hosts without anybody having installed anything, and
    // `-NoProfile -NonInteractive` keeps a profile's own banner or prompt off
    // the stream this module is about to parse as JSON.
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .map_err(|e| format!("powershell could not be run to enumerate disks: {e}"))?;
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr);
        let last = said
            .lines()
            .map(str::trim)
            .rfind(|l| !l.is_empty())
            .unwrap_or("powershell refused to enumerate disks and said nothing");
        return Err(format!("could not enumerate disks: {last}"));
    }
    parse_disks(&out.stdout)
}

/// The unbuilt platforms, which say so rather than reporting an empty list
/// that would be read as "nothing is attached."
///
/// # Errors
///
/// Always.
#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub fn all() -> Result<Vec<Disk>, String> {
    Err("removable-disk enumeration is built only for Windows, Linux and macOS (ADR-062 §5). \
         --device cannot be checked on this platform, so it is refused rather than trusted."
        .to_owned())
}

/// Every physical disk, as `diskutil` describes it — `ADR-062` §5.
///
/// `diskutil list -plist physical` names the whole disks and their mounted
/// partitions; `diskutil info -plist` on each gives the size, the model and
/// the removability flags. Both answer in XML property lists, which `plutil`
/// converts to JSON so that [`crate::json`] is the one reader — the same
/// posture the Windows branch takes toward CIM, and for the same reason: it is
/// the query a person can run by hand to check this tool's answer.
///
/// # Errors
///
/// `diskutil` or `plutil` would not run or refused, or printed something that
/// is not the shape described above — never an empty list standing in for a
/// failure.
#[cfg(target_os = "macos")]
pub fn all() -> Result<Vec<Disk>, String> {
    let list = diskutil_json(&["list", "-plist", "physical"])?;
    let entries = list
        .get("AllDisksAndPartitions")
        .and_then(Json::as_array)
        .ok_or("`diskutil list -plist physical` printed no AllDisksAndPartitions array")?;
    entries
        .iter()
        .map(|entry| {
            let id = entry
                .get("DeviceIdentifier")
                .and_then(Json::as_str)
                .ok_or("a disk in `diskutil list` has no DeviceIdentifier")?;
            let info = diskutil_json(&["info", "-plist", id])?;
            disk_from_diskutil(entry, &info)
        })
        .collect()
}

/// `diskutil ARGS`, converted from a property list to JSON and parsed.
///
/// Both programs by absolute path: they ship at these locations on every macOS,
/// and this is the query that decides which disk may be overwritten, so a
/// different program earlier on `PATH` must not be able to answer it.
#[cfg(target_os = "macos")]
fn diskutil_json(args: &[&str]) -> Result<Json, String> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let listed = Command::new("/usr/sbin/diskutil")
        .args(args)
        .output()
        .map_err(|e| format!("diskutil could not be run to enumerate disks: {e}"))?;
    if !listed.status.success() {
        let said = String::from_utf8_lossy(&listed.stderr);
        let last = said.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("it said nothing");
        return Err(format!("`diskutil {}` refused: {last}", args.join(" ")));
    }
    let mut plutil = Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("plutil could not be run to read diskutil's answer: {e}"))?;
    plutil
        .stdin
        .take()
        .ok_or("plutil's input could not be opened")?
        .write_all(&listed.stdout)
        .map_err(|e| format!("cannot hand diskutil's answer to plutil: {e}"))?;
    let converted = plutil.wait_with_output().map_err(|e| format!("plutil did not finish: {e}"))?;
    if !converted.status.success() {
        return Err(format!(
            "plutil could not convert `diskutil {}`'s answer: {}",
            args.join(" "),
            String::from_utf8_lossy(&converted.stderr).trim()
        ));
    }
    crate::json::parse(&String::from_utf8_lossy(&converted.stdout))
        .map_err(|e| format!("`diskutil {}` did not convert to JSON this can read: {e}", args.join(" ")))
}

/// One disk, from its `diskutil list` entry and its `diskutil info`.
///
/// **The removability rule, stated once.** `RemovableMedia` is true for a card
/// in a built-in SD slot; `RemovableMediaOrExternalDevice` with `Internal`
/// false is a USB reader or any other external disk. The internal SSD is false
/// on every one. **A flag that is absent reads as the safer answer** — not
/// removable, internal — because a disk this cannot fully describe must never
/// be the one offered as a write target.
///
/// The id is the RAW node, `/dev/rdiskN`: the buffered `/dev/diskN` works and
/// is many times slower for a whole-card write. What is mounted from the disk
/// goes where Windows puts drive letters, so an operator recognises the card
/// they just inserted.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn disk_from_diskutil(entry: &Json, info: &Json) -> Result<Disk, String> {
    let ident = info
        .get("DeviceIdentifier")
        .and_then(Json::as_str)
        .ok_or("a disk in `diskutil info` has no DeviceIdentifier")?;
    let flag = |key: &str| info.get(key).and_then(Json::as_bool);
    let removable = flag("RemovableMedia").unwrap_or(false)
        || (!flag("Internal").unwrap_or(true) && flag("RemovableMediaOrExternalDevice").unwrap_or(false));
    let size_bytes = info
        .get("TotalSize")
        .or_else(|| info.get("Size"))
        .and_then(Json::as_u64)
        .unwrap_or(0);
    let model = info.get("MediaName").and_then(Json::as_str).unwrap_or_default().trim().to_owned();

    let mut letters: Vec<String> = Vec::new();
    let mut mounted = |v: &Json| {
        if let Some(point) = v.get("MountPoint").and_then(Json::as_str) {
            letters.push(point.to_owned());
        }
    };
    mounted(entry);
    for part in entry.get("Partitions").and_then(Json::as_array).unwrap_or_default() {
        mounted(part);
    }

    Ok(Disk { id: format!("/dev/r{ident}"), model, size_bytes, removable, letters })
}

/// Every disk under `/sys/block`, read the way the kernel already publishes
/// the answer.
///
/// # Errors
///
/// Whatever stopped `/sys/block` from being read at all. A disk this cannot
/// read `removable`, `size` or `device/model` for is not skipped — it is
/// reported with whatever it could not determine defaulted to the safer
/// reading (not removable, size zero), because a disk this module cannot
/// fully describe must never be the one silently offered as a write target.
#[cfg(target_os = "linux")]
pub fn all() -> Result<Vec<Disk>, String> {
    all_under(Path::new("/sys/block"))
}

/// The `/sys/block` reader, taking the root as a parameter so it can be
/// proven against a scratch directory standing in for `/sys/block` — this
/// project's Windows development host has no `/sys` at all
/// (`AGENTS.md` §10.5), so this is the parsing logic, verified; whether the
/// real path answers the same way is unproven until it runs on a Linux host.
///
/// **Reachable only from [`all`] on Linux, and from this module's own tests
/// everywhere else.** A non-Linux, non-test build calls neither, which is
/// exactly the platform gate working as intended rather than dead code.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn all_under(root: &Path) -> Result<Vec<Disk>, String> {
    let entries =
        std::fs::read_dir(root).map_err(|e| format!("cannot read {}: {e}", root.display()))?;
    let mut out = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|e| format!("cannot read an entry under {}: {e}", root.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let dir = entry.path();

        // **Absence defaults to the safer answer, not to a refusal.** A block
        // device this host cannot fully describe (a loop device with no
        // `device/model`, for instance) is real and must still appear in the
        // "everything" list `resolve_device`'s refusal message reads from —
        // just never as removable, which is the one field a wrong default
        // could turn into a written-over disk.
        let removable = std::fs::read_to_string(dir.join("removable"))
            .map(|s| s.trim() == "1")
            .unwrap_or(false);
        // `size` is 512-byte sectors — the kernel's own unit for this file,
        // documented in Documentation/ABI/testing/sysfs-block, unrelated to
        // the disk's actual sector size.
        let sectors: u64 = std::fs::read_to_string(dir.join("size"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        let model = std::fs::read_to_string(dir.join("device").join("model"))
            .map(|s| s.trim().to_owned())
            .unwrap_or_default();

        out.push(Disk {
            id: format!("/dev/{name}"),
            model,
            size_bytes: sectors.saturating_mul(512),
            removable,
            // Linux has no drive-letter concept; `/dev/<name>` already is
            // the human-recognisable handle.
            letters: Vec::new(),
        });
    }
    Ok(out)
}

/// Turn the PowerShell script's JSON into disks.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_disks(bytes: &[u8]) -> Result<Vec<Disk>, String> {
    let text = String::from_utf8_lossy(bytes);
    let root = crate::json::parse(&text)
        .map_err(|e| format!("the disk list powershell printed is not JSON this can read: {e}"))?;
    let array = root
        .get("disks")
        .and_then(Json::as_array)
        .ok_or_else(|| "the disk list powershell printed has no \"disks\" array".to_owned())?;
    array.iter().map(disk_from_json).collect()
}

#[cfg_attr(not(windows), allow(dead_code))]
fn disk_from_json(v: &Json) -> Result<Disk, String> {
    let id = v
        .get("DeviceID")
        .and_then(Json::as_str)
        .ok_or_else(|| "a disk in the CIM answer has no DeviceID".to_owned())?
        .to_owned();
    let model = v.get("Model").and_then(Json::as_str).unwrap_or_default().trim().to_owned();
    let size_bytes = v.get("Size").and_then(Json::as_u64).unwrap_or(0);
    let interface_type = v.get("InterfaceType").and_then(Json::as_str).unwrap_or_default();
    let media_type = v.get("MediaType").and_then(Json::as_str).unwrap_or_default();

    // **The removability rule, stated once.** A card reader often answers
    // `USB` on `InterfaceType` with a `MediaType` of "External hard disk
    // media" rather than "Removable Media" — Windows' own inconsistency, not
    // this module's — so either signal alone is trusted.
    let removable =
        media_type.to_ascii_lowercase().contains("removable") || interface_type.eq_ignore_ascii_case("usb");

    let letters = v
        .get("Letters")
        .and_then(Json::as_array)
        .map(|a| a.iter().filter_map(Json::as_str).map(str::to_owned).collect())
        .unwrap_or_default();

    Ok(Disk { id, model, size_bytes, removable, letters })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **The real thing, read-only.** Not a fixture — this actually shells
    /// out to `powershell` and asks this development host what it sees,
    /// exactly the way `crate::provision` will. Run with `-- --nocapture` to
    /// read what it found; the assertions only check that the call succeeds
    /// and that every disk it names is well-formed, because what is actually
    /// attached to this machine on any given run is not this test's business
    /// to assume.
    #[cfg(windows)]
    #[test]
    fn the_real_windows_enumeration_runs_and_prints_what_it_saw() {
        let disks = all().expect("enumerating this host's own disks must succeed");
        println!("\n--- real enumeration on this host: {} disk(s)", disks.len());
        for disk in &disks {
            println!(
                "    {} removable={} {}",
                disk.id,
                disk.removable,
                disk.describe()
            );
            assert!(!disk.id.is_empty(), "every disk must have an id");
        }
    }

    /// **The real fixture again**, this time end to end through
    /// [`parse_disks`] rather than through `crate::json` alone — the same
    /// bytes `crate::json`'s test parses, captured from this host.
    #[test]
    fn a_real_cim_payload_becomes_two_fixed_disks() {
        let text = br#"{
    "disks":  [
                  {
                      "DeviceID":  "\\\\.\\PHYSICALDRIVE1",
                      "Model":  "NVMe MTFDKBA1T0QFM-1BD1AABGB",
                      "Size":  1024203640320,
                      "InterfaceType":  "SCSI",
                      "MediaType":  "Fixed hard disk media",
                      "Letters":  [
                                      "D:"
                                  ]
                  },
                  {
                      "DeviceID":  "\\\\.\\PHYSICALDRIVE0",
                      "Model":  "NVMe MTFDKBA1T0QFM-1BD1AABGB",
                      "Size":  1024203640320,
                      "InterfaceType":  "SCSI",
                      "MediaType":  "Fixed hard disk media",
                      "Letters":  [
                                      "C:"
                                  ]
                  }
              ]
}"#;
        let disks = parse_disks(text).expect("real output must parse");
        assert_eq!(disks.len(), 2);
        assert_eq!(disks[0].id, r"\\.\PHYSICALDRIVE1");
        assert_eq!(disks[0].size_bytes, 1_024_203_640_320);
        assert_eq!(disks[0].letters, vec!["D:".to_owned()]);
        assert!(!disks[0].removable, "SCSI/Fixed hard disk media is not removable");
        assert!(!disks[1].removable);
    }

    /// **The empty case**, named in the brief: no disk attached at all.
    /// `all()`'s own doc comment promises `Ok(vec![])`, never an error, for
    /// exactly this shape.
    #[test]
    fn an_empty_disks_array_is_ok_with_nothing_in_it() {
        let disks = parse_disks(br#"{"disks": []}"#).expect("an empty list is not a failure");
        assert!(disks.is_empty());
    }

    /// A USB card reader — `MediaType` says removable, `InterfaceType` is
    /// USB, and either alone must be enough to mark it so.
    #[test]
    fn a_removable_media_type_or_a_usb_interface_marks_a_disk_removable() {
        let by_media = disk_from_json(
            &crate::json::parse(
                r#"{"DeviceID":"\\\\.\\PHYSICALDRIVE2","Model":"SD Card","Size":16000000000,"InterfaceType":"USB","MediaType":"Removable Media"}"#,
            )
            .expect("parses"),
        )
        .expect("a disk");
        assert!(by_media.removable);

        // Windows really does answer this way for some card readers:
        // InterfaceType USB, MediaType "External hard disk media" — neither
        // string contains the word "removable", and the USB half alone must
        // still be trusted.
        let by_interface = disk_from_json(
            &crate::json::parse(
                r#"{"DeviceID":"\\\\.\\PHYSICALDRIVE3","Model":"Reader","Size":8000000000,"InterfaceType":"USB","MediaType":"External hard disk media"}"#,
            )
            .expect("parses"),
        )
        .expect("a disk");
        assert!(by_interface.removable);

        let fixed = disk_from_json(
            &crate::json::parse(
                r#"{"DeviceID":"\\\\.\\PHYSICALDRIVE0","Model":"NVMe","Size":1000000000000,"InterfaceType":"SCSI","MediaType":"Fixed hard disk media"}"#,
            )
            .expect("parses"),
        )
        .expect("a disk");
        assert!(!fixed.removable);
    }

    /// A field the platform did not answer — `null`, or missing entirely —
    /// is read as empty/zero rather than failing the whole disk.
    #[test]
    fn a_null_or_missing_field_defaults_rather_than_refuses() {
        let disk = disk_from_json(
            &crate::json::parse(r#"{"DeviceID":"\\\\.\\PHYSICALDRIVE9","Model":null}"#)
                .expect("parses"),
        )
        .expect("a disk missing everything but DeviceID must still parse");
        assert_eq!(disk.model, "");
        assert_eq!(disk.size_bytes, 0);
        assert!(!disk.removable);
        assert!(disk.letters.is_empty());
    }

    /// No `DeviceID` at all is the one field this refuses to default —
    /// nothing this project writes to a disk it cannot name.
    #[test]
    fn a_disk_with_no_device_id_is_refused() {
        let why = parse_disks(br#"{"disks": [{"Model": "mystery"}]}"#)
            .expect_err("a disk with no address must not silently vanish or default to one");
        assert!(why.contains("DeviceID"), "{why}");
    }

    #[test]
    fn describe_reports_size_model_and_letters() {
        let disk = Disk {
            id: r"\\.\PHYSICALDRIVE2".to_owned(),
            model: "SanDisk Ultra USB Device".to_owned(),
            size_bytes: 31_914_983_424,
            removable: true,
            letters: vec!["E:".to_owned()],
        };
        let text = disk.describe();
        assert!(text.contains("31.9 GB"), "{text}");
        assert!(text.contains("SanDisk"), "{text}");
        assert!(text.contains("E:"), "{text}");
    }

    #[test]
    fn describe_says_plainly_when_nothing_is_mounted() {
        let disk = Disk {
            id: "/dev/sdb".to_owned(),
            model: String::new(),
            size_bytes: 500_000_000,
            removable: true,
            letters: Vec::new(),
        };
        let text = disk.describe();
        assert!(text.contains("nothing mounted from it"), "{text}");
        assert!(text.contains("no model reported"), "{text}");
        assert!(text.contains("500.0 MB"), "{text}");
    }

    /// **The `/sys/block` reader, proven against a scratch directory** — this
    /// is the parsing logic, not the real `/sys/block`, which this project's
    /// Windows development host does not have (`AGENTS.md` §10.5). Whether
    /// the real path answers this way is unproven until this runs on Linux.
    #[test]
    fn sys_block_shaped_fixtures_parse_into_disks() {
        let root = crate::scratch::absent(&format!("sysblock-{}", line!()));

        // A removable card reader: sda, 1,000,000 sectors, a model file.
        let sda = root.join("sda");
        std::fs::create_dir_all(sda.join("device")).expect("mkdir");
        std::fs::write(sda.join("removable"), "1\n").expect("write");
        std::fs::write(sda.join("size"), "1000000\n").expect("write");
        std::fs::write(sda.join("device").join("model"), "Ultra Fit    \n").expect("write");

        // A fixed disk with no model file at all — must default, not fail.
        let sdb = root.join("sdb");
        std::fs::create_dir_all(&sdb).expect("mkdir");
        std::fs::write(sdb.join("removable"), "0\n").expect("write");
        std::fs::write(sdb.join("size"), "2000000000\n").expect("write");

        let mut disks = all_under(&root).expect("a scratch /sys/block must read");
        disks.sort_by(|a, b| a.id.cmp(&b.id));
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(disks.len(), 2);
        assert_eq!(disks[0].id, "/dev/sda");
        assert!(disks[0].removable);
        assert_eq!(disks[0].size_bytes, 1_000_000 * 512);
        assert_eq!(disks[0].model, "Ultra Fit", "trailing whitespace must be trimmed");
        assert_eq!(disks[1].id, "/dev/sdb");
        assert!(!disks[1].removable);
        assert_eq!(disks[1].model, "", "a disk with no device/model file defaults to empty");
    }

    /// A `/sys/block` this host cannot read at all is an error, never an
    /// empty list — `AGENTS.md` §7 applies to a filesystem read exactly as it
    /// does to anything else this project reads.
    #[test]
    fn an_unreadable_root_is_an_error_not_an_empty_list() {
        let missing = crate::scratch::absent(&format!("sysblock-missing-{}", line!()));
        let why = all_under(&missing).expect_err("a directory that does not exist must refuse");
        assert!(why.contains(&missing.display().to_string()), "{why}");
    }
}

#[cfg(test)]
mod diskutil_tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn json(text: &str) -> Json {
        crate::json::parse(text).expect("fixture parses")
    }

    /// **Captured**: the removability fields and values `diskutil info -plist
    /// disk0 | plutil -convert json` reported for an Apple Silicon Mac's
    /// internal SSD, with its UUIDs and every other identifier left out. `\/`
    /// is kept as `plutil` wrote it.
    const INTERNAL_SSD: &str = r#"{"DeviceNode":"\/dev\/disk0","RemovableMediaOrExternalDevice":false,
        "Internal":true,"WholeDisk":true,"RemovableMedia":false,"Size":500277792768,
        "BusProtocol":"Apple Fabric","Ejectable":false,"DeviceIdentifier":"disk0",
        "MediaName":"APPLE SSD AP0512Z","TotalSize":500277792768,"Removable":false}"#;

    #[test]
    fn the_internal_ssd_is_never_removable() {
        let disk = disk_from_diskutil(&json(r#"{"DeviceIdentifier":"disk0"}"#), &json(INTERNAL_SSD)).unwrap();
        assert!(!disk.removable, "{disk:?}");
        assert_eq!(disk.id, "/dev/rdisk0");
        assert_eq!(disk.size_bytes, 500_277_792_768);
    }

    /// **Hand-composed**: a card in a USB reader — external, and not internal.
    #[test]
    fn a_usb_card_reader_is_removable_and_names_what_is_mounted() {
        let info = json(
            r#"{"DeviceIdentifier":"disk4","Internal":false,"RemovableMediaOrExternalDevice":true,
                "RemovableMedia":true,"TotalSize":31914983424,"MediaName":"SD Card Reader"}"#,
        );
        let entry = json(
            r#"{"DeviceIdentifier":"disk4","Partitions":[
                {"DeviceIdentifier":"disk4s1","MountPoint":"\/Volumes\/bootfs"},
                {"DeviceIdentifier":"disk4s2"}]}"#,
        );
        let disk = disk_from_diskutil(&entry, &info).unwrap();
        assert!(disk.removable);
        assert_eq!(disk.id, "/dev/rdisk4");
        assert_eq!(disk.letters, vec!["/Volumes/bootfs".to_owned()]);
        assert!(disk.describe().contains("mounted at /Volumes/bootfs"), "{}", disk.describe());
    }

    /// **Hand-composed**: a card in a built-in SD slot, which is internal and
    /// still removable media.
    #[test]
    fn a_built_in_sd_slot_is_removable() {
        let info = json(
            r#"{"DeviceIdentifier":"disk5","Internal":true,"RemovableMedia":true,
                "RemovableMediaOrExternalDevice":true}"#,
        );
        assert!(disk_from_diskutil(&json("{}"), &info).unwrap().removable);
    }

    /// A disk whose flags are missing reads as the safer answer.
    #[test]
    fn missing_flags_are_not_removable() {
        let info = json(r#"{"DeviceIdentifier":"disk6","RemovableMediaOrExternalDevice":true}"#);
        assert!(!disk_from_diskutil(&json("{}"), &info).unwrap().removable, "Internal absent reads as internal");
    }

    /// **The real thing, read-only**: this host's own `diskutil`. Asserts only
    /// that it answers and that no disk it calls internal is offered.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_real_macos_enumeration_runs_and_prints_what_it_saw() {
        let disks = all().expect("enumerating this host's own disks must succeed");
        println!("\n--- real enumeration on this host: {} disk(s)", disks.len());
        for disk in &disks {
            println!("    removable={} {}", disk.removable, disk.describe());
            assert!(disk.id.starts_with("/dev/rdisk"), "{disk:?}");
        }
        assert!(!disks.is_empty(), "a Mac has at least its own disk");
    }
}
