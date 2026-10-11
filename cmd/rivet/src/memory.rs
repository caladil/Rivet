//! Reading this machine's memory — `ADR-035` phase 4.
//!
//! **`/proc/meminfo`, `/proc/pressure/memory`, `/proc/slabinfo` and the
//! `/sys/kernel/slab` directory, and nothing else.**
//! `crate::journal`'s note carries why that belongs here rather than behind
//! `BootMedium`: it needs no `mount(2)`, no device tree and no pid 1, so a
//! build that refuses every other machine operation still answers this one
//! truthfully — the numbers are about whatever Linux is running, Rivet or not.

use std::collections::{BTreeMap, BTreeSet};

use rivet_core::operation::machine::{
    KernelMemory, MachineError, MemoryPressure, MemoryRead, SlabCache, SlabCaches,
};

const MEMINFO: &str = "/proc/meminfo";
const PRESSURE: &str = "/proc/pressure/memory";
const SLABINFO: &str = "/proc/slabinfo";
const SLAB_DIRECTORY: &str = "/sys/kernel/slab";

/// The only `/proc/slabinfo` format this reads. Version 2.1 has been the
/// format since Linux 2.6; a different header is refused rather than read as
/// though its columns meant the same things.
const SLABINFO_HEADER: &str = "slabinfo - version: 2.1";

/// This machine's memory.
///
/// # Errors
///
/// If `/proc/meminfo` cannot be read or does not carry `MemTotal`. **A missing
/// `/proc/pressure/memory` is not an error** — see [`MemoryRead::pressure`].
pub fn read() -> Result<MemoryRead, MachineError> {
    let text = std::fs::read_to_string(MEMINFO)
        .map_err(|e| MachineError::Refused(format!("{MEMINFO} could not be read: {e}")))?;
    let get = |key: &str| field(&text, key);

    // **`MemTotal` is the one that must be there.** A `/proc/meminfo` without it
    // is not a meminfo, and answering zero would be a claim that this machine
    // has no memory rather than that the file was not what was expected.
    let total_bytes = get("MemTotal").ok_or_else(|| {
        MachineError::Refused(format!(
            "{MEMINFO} carries no MemTotal, so it is not the file this expected"
        ))
    })?;

    Ok(MemoryRead {
        total_bytes,
        // `MemAvailable` predates every kernel this runs on, but falling back to
        // `MemFree` is stated rather than silent: the two mean different things
        // and a caller reading the fallback as the real thing would badly
        // misjudge a healthy machine.
        available_bytes: get("MemAvailable").or_else(|| get("MemFree")).unwrap_or(0),
        free_bytes: get("MemFree").unwrap_or(0),
        cached_bytes: get("Cached").unwrap_or(0),
        swap_total_bytes: get("SwapTotal").unwrap_or(0),
        swap_free_bytes: get("SwapFree").unwrap_or(0),
        pressure: pressure(),
        kernel: kernel(&text),
        slab_caches: slab_caches(),
    })
}

/// Every slab cache, with the names merged into each — or `None` when
/// `/proc/slabinfo` cannot be read, which for anyone but root it cannot.
///
/// **Why `/proc/slabinfo` and not `/sys/kernel/slab/<cache>/`:** one read of one
/// file gives every cache's counts at a single moment, where the directory
/// needs several reads per cache and its per-cache `slabs` and `objects` files
/// exist only with `CONFIG_SLUB_DEBUG`, which is also what provides slabinfo.
/// The directory is still read, once, for the one thing slabinfo cannot say:
/// which caches the kernel merged into which.
fn slab_caches() -> Option<SlabCaches> {
    let text = std::fs::read_to_string(SLABINFO).ok()?;
    Some(match parse_slabinfo(&text) {
        Err(why) => SlabCaches::Refused(why),
        Ok(mut caches) => {
            if let Some(entries) = slab_directory() {
                attach_merged(&mut caches, &entries);
            }
            SlabCaches::Read(caches)
        }
    })
}

/// `/proc/slabinfo` version 2.1, every cache line, or a refusal naming the
/// line that did not parse.
///
/// **Refused whole, never skipped.** A line this cannot read is a line whose
/// cache would be missing from the answer, and the missing one is exactly as
/// likely as any other to be the cache that is growing.
fn parse_slabinfo(text: &str) -> Result<Vec<SlabCache>, String> {
    let mut lines = text.lines();
    let header = lines.next().unwrap_or_default();
    if header.trim_end() != SLABINFO_HEADER {
        return Err(format!(
            "{SLABINFO} opens with {header:?} and this build reads only {SLABINFO_HEADER:?}, so \
             its columns are not assumed to mean the same things"
        ));
    }
    let mut caches = Vec::new();
    for (at, line) in lines.enumerate() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        caches.push(slabinfo_line(line).ok_or_else(|| {
            format!(
                "{SLABINFO} line {} is not a version 2.1 cache line, so the listing is refused \
                 whole rather than reported without it: {line:?}",
                at + 2
            )
        })?);
    }
    if caches.is_empty() {
        return Err(format!("{SLABINFO} lists no cache at all, which no running kernel does"));
    }
    Ok(caches)
}

/// `name active num objsize objperslab pagesperslab : tunables l b s : slabdata a n s`
/// — sixteen fields, the separators where 2.1 puts them, every number a number.
fn slabinfo_line(line: &str) -> Option<SlabCache> {
    let f: Vec<&str> = line.split_whitespace().collect();
    if f.len() != 16 || f[6] != ":" || f[7] != "tunables" || f[11] != ":" || f[12] != "slabdata" {
        return None;
    }
    let number = |i: usize| f[i].parse::<u64>().ok();
    for i in [1, 2, 3, 4, 5, 8, 9, 10, 13, 14, 15] {
        number(i)?;
    }
    Some(SlabCache {
        name: f[0].to_string(),
        active_objects: number(1)?,
        objects: number(2)?,
        object_size: number(3)?,
        merged: None,
    })
}

/// One entry of `/sys/kernel/slab`: its name, and where it links if it is a
/// link. SLUB gives a mergeable cache a directory named by a unique id such as
/// `:0000192` and makes EVERY name merged into it a link there, so names that
/// link to the same place are one cache; an unmergeable cache is a directory
/// under its own name.
type SlabEntry = (String, Option<String>);

fn slab_directory() -> Option<Vec<SlabEntry>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(SLAB_DIRECTORY).ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name().into_string().ok()?;
        let target = if entry.file_type().ok()?.is_symlink() {
            Some(std::fs::read_link(entry.path()).ok()?.to_str()?.to_string())
        } else {
            None
        };
        entries.push((name, target));
    }
    Some(entries)
}

/// Give each cache the OTHER names linked to the same place, sorted; an empty
/// list for an unmerged cache; and `None` for a cache the directory does not
/// name, since nothing was read about it.
fn attach_merged(caches: &mut [SlabCache], entries: &[SlabEntry]) {
    let mut groups: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut link: BTreeMap<&str, &str> = BTreeMap::new();
    let mut directories: BTreeSet<&str> = BTreeSet::new();
    for (name, target) in entries {
        match target {
            Some(t) => {
                // `:0000192` or `../:0000192`: the place is its last component.
                let place = t.rsplit('/').next().unwrap_or(t);
                groups.entry(place).or_default().insert(name);
                link.insert(name, place);
            }
            None => {
                directories.insert(name);
            }
        }
    }
    for cache in caches {
        cache.merged = if let Some(place) = link.get(cache.name.as_str()) {
            groups.get(place).map(|names| {
                names.iter().filter(|n| **n != cache.name).map(|n| (*n).to_string()).collect()
            })
        } else if directories.contains(cache.name.as_str()) {
            Some(Vec::new())
        } else {
            None
        };
    }
}

/// The kernel's own holdings. **Each line independently**: a kernel without
/// `CONFIG_CMA` writes no `CmaTotal`, and that is `None` rather than a zero
/// the kernel never stated.
fn kernel(text: &str) -> KernelMemory {
    let get = |key: &str| field(text, key);
    KernelMemory {
        slab_bytes: get("Slab"),
        slab_reclaimable_bytes: get("SReclaimable"),
        slab_unreclaimable_bytes: get("SUnreclaim"),
        shmem_bytes: get("Shmem"),
        page_tables_bytes: get("PageTables"),
        kernel_stack_bytes: get("KernelStack"),
        vmalloc_used_bytes: get("VmallocUsed"),
        cma_total_bytes: get("CmaTotal"),
        cma_free_bytes: get("CmaFree"),
    }
}

/// One `key:  1234 kB` line, in **bytes**.
///
/// `/proc/meminfo` is in kibibytes for every field that has a unit, and the
/// conversion happens here so that nothing downstream carries the unit around.
fn field(text: &str, key: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix(':'))
        .and_then(|rest| rest.split_whitespace().next()?.parse::<u64>().ok())
        .map(|kib| kib.saturating_mul(1024))
}

/// Pressure stall information, or `None` on a kernel that has none.
fn pressure() -> Option<MemoryPressure> {
    let text = std::fs::read_to_string(PRESSURE).ok()?;
    let line = |prefix: &str| -> Option<(u32, u32)> {
        let row = text.lines().find(|l| l.starts_with(prefix))?;
        Some((centipercent(row, "avg10=")?, centipercent(row, "avg60=")?))
    };
    let (some_avg10, some_avg60) = line("some")?;
    // **`full` is absent for cpu and may be absent here on older kernels**, so
    // its absence is zero rather than a reason to discard the `some` numbers,
    // which are the more useful half anyway.
    let (full_avg10, full_avg60) = line("full").unwrap_or((0, 0));
    Some(MemoryPressure { some_avg10, some_avg60, full_avg10, full_avg60 })
}

/// `avg10=12.34` as `1234` — hundredths of a percent, and see
/// [`MemoryPressure`] for why the value model needs them that way.
fn centipercent(row: &str, key: &str) -> Option<u32> {
    let raw = row.split_whitespace().find_map(|f| f.strip_prefix(key))?;
    let (whole, frac) = raw.split_once('.').unwrap_or((raw, "0"));
    let whole: u32 = whole.parse().ok()?;
    let frac: u32 = format!("{frac:0<2}").get(..2)?.parse().ok()?;
    Some(whole.saturating_mul(100).saturating_add(frac))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use super::*;

    /// kB in, bytes out — the conversion this module exists to do once.
    #[test]
    fn a_meminfo_field_is_returned_in_bytes() {
        let text = "MemTotal:        8039332 kB\nMemFree:          123 kB\n";
        assert_eq!(field(text, "MemTotal"), Some(8_039_332 * 1024));
        assert_eq!(field(text, "MemFree"), Some(123 * 1024));
        assert_eq!(field(text, "Nope"), None);
    }

    /// **A prefix is not a field.** `MemFree` must not answer for `MemFree2`,
    /// and `Mem` must not answer for `MemTotal` — the colon is what makes a key
    /// a key.
    #[test]
    fn a_key_matches_only_a_whole_field_name() {
        let text = "MemTotal:  100 kB\nHugePages_Total:  0\n";
        assert_eq!(field(text, "Mem"), None, "a prefix matched a longer name");
        assert_eq!(field(text, "MemTotal"), Some(100 * 1024));
    }

    /// **Captured, not composed**: the text a Raspberry Pi 4 running Rivet
    /// `0.1.121` returned for `get file /proc/meminfo`, byte for byte. The
    /// board's read stopped at `CmaFree`, so this is a prefix of the kernel's
    /// file rather than all of it — which is every line this module reads.
    const PI4_MEMINFO: &str = "MemTotal:         922976 kB\nMemFree:          772288 kB\nMemAvailable:     813040 kB\nBuffers:            7596 kB\nCached:            66796 kB\nSwapCached:            0 kB\nActive:            60148 kB\nInactive:          19016 kB\nActive(anon):      31036 kB\nInactive(anon):        0 kB\nActive(file):      29112 kB\nInactive(file):    19016 kB\nUnevictable:        9844 kB\nMlocked:               0 kB\nSwapTotal:             0 kB\nSwapFree:              0 kB\nZswap:                 0 kB\nZswapped:              0 kB\nDirty:                 4 kB\nWriteback:             0 kB\nAnonPages:         14648 kB\nMapped:            40412 kB\nShmem:             26264 kB\nKReclaimable:       6580 kB\nSlab:              20656 kB\nSReclaimable:       6580 kB\nSUnreclaim:        14076 kB\nKernelStack:        2588 kB\nPageTables:          488 kB\nSecPageTables:         0 kB\nNFS_Unstable:          0 kB\nBounce:                0 kB\nWritebackTmp:          0 kB\nCommitLimit:      461488 kB\nCommitted_AS:      68692 kB\nVmallocTotal:   261087232 kB\nVmallocUsed:        5108 kB\nVmallocChunk:          0 kB\nPercpu:              624 kB\nCmaTotal:         524288 kB\nCmaFree:          498024 kB\n";

    /// Every kernel line out of a real board's file, in bytes, each from its
    /// own line — and `SecPageTables` must not answer for `PageTables`, nor
    /// `VmallocTotal` for `VmallocUsed`.
    #[test]
    fn a_real_pi4_meminfo_yields_every_kernel_holding_in_bytes() {
        let k = kernel(PI4_MEMINFO);
        assert_eq!(k.slab_bytes, Some(20_656 * 1024));
        assert_eq!(k.slab_reclaimable_bytes, Some(6_580 * 1024));
        assert_eq!(k.slab_unreclaimable_bytes, Some(14_076 * 1024));
        assert_eq!(k.shmem_bytes, Some(26_264 * 1024));
        assert_eq!(k.page_tables_bytes, Some(488 * 1024));
        assert_eq!(k.kernel_stack_bytes, Some(2_588 * 1024));
        assert_eq!(k.vmalloc_used_bytes, Some(5_108 * 1024));
        assert_eq!(k.cma_total_bytes, Some(524_288 * 1024));
        assert_eq!(k.cma_free_bytes, Some(498_024 * 1024));
    }

    /// The same capture still answers the fields that were there before.
    #[test]
    fn a_real_pi4_meminfo_still_answers_the_original_fields() {
        assert_eq!(field(PI4_MEMINFO, "MemTotal"), Some(922_976 * 1024));
        assert_eq!(field(PI4_MEMINFO, "MemAvailable"), Some(813_040 * 1024));
        assert_eq!(field(PI4_MEMINFO, "Cached"), Some(66_796 * 1024));
    }

    /// **A kernel without CONFIG_CMA writes no `Cma*` lines**, and they come
    /// back `None` — not zero — while every other line is still read.
    /// Composed from the capture above by deleting those two lines.
    #[test]
    fn a_line_the_kernel_did_not_write_is_none_and_the_rest_still_read() {
        let without_cma: String = PI4_MEMINFO
            .lines()
            .filter(|l| !l.starts_with("Cma"))
            .map(|l| format!("{l}\n"))
            .collect();
        let k = kernel(&without_cma);
        assert_eq!(k.cma_total_bytes, None);
        assert_eq!(k.cma_free_bytes, None);
        assert_eq!(k.slab_unreclaimable_bytes, Some(14_076 * 1024));
        assert_eq!(k.shmem_bytes, Some(26_264 * 1024));
    }

    /// Hundredths, including the case the whole-number form would lose.
    #[test]
    fn a_pressure_average_becomes_hundredths_of_a_percent() {
        let row = "some avg10=0.42 avg60=12.30 avg300=0.00 total=123";
        assert_eq!(centipercent(row, "avg10="), Some(42));
        assert_eq!(centipercent(row, "avg60="), Some(1230));
        assert_eq!(centipercent(row, "avg300="), Some(0));
    }

    /// **Captured, not composed**: what a Raspberry Pi 4 running Rivet `0.1.128`
    /// returned for `get file /proc/slabinfo`, byte for byte. It ends at
    /// `kmem_cache`, which the kernel lists last.
    const PI4_SLABINFO: &str = include_str!("../tests/fixtures/slabinfo-pi4-0128.txt");

    fn named<'a>(caches: &'a [SlabCache], name: &str) -> &'a SlabCache {
        caches.iter().find(|c| c.name == name).expect(name)
    }

    /// Every cache line of a real board's listing is read, with its counts in
    /// the columns 2.1 puts them.
    #[test]
    fn a_real_pi4_slabinfo_yields_every_cache_with_its_counts() {
        let caches = parse_slabinfo(PI4_SLABINFO).expect("the capture parses");
        let lines = PI4_SLABINFO.lines().skip(1).filter(|l| !l.starts_with('#')).count();
        assert_eq!(caches.len(), lines, "a cache line was dropped");
        let kernfs = named(&caches, "kernfs_node_cache");
        assert_eq!((kernfs.active_objects, kernfs.objects, kernfs.object_size), (18_480, 18_480, 136));
        assert_eq!(kernfs.bytes(), 18_480 * 136);
        let buffer_head = named(&caches, "buffer_head");
        assert_eq!((buffer_head.active_objects, buffer_head.objects), (700, 975));
        let task = named(&caches, "task_struct");
        assert_eq!(task.object_size, 8448);
        assert!(caches.iter().all(|c| c.merged.is_none()), "merging is not slabinfo's to say");
    }

    /// **One line that does not parse refuses the whole listing, and the refusal
    /// carries that line.** Composed from the capture by corrupting one line.
    #[test]
    fn one_unparseable_line_refuses_the_listing_and_names_the_line() {
        let broken = with_cache_line(PI4_SLABINFO, "task_struct", |f| f[2] = "lots");
        let why = parse_slabinfo(&broken).expect_err("a line did not parse");
        assert!(why.contains("task_struct 168 lots 8448"), "{why}");
    }

    /// A line with a missing column is refused too, not read with the columns
    /// shifted one place.
    #[test]
    fn a_line_with_a_column_missing_is_refused_rather_than_shifted() {
        let broken = with_cache_line(PI4_SLABINFO, "task_struct", |f| {
            f.remove(8);
        });
        assert!(parse_slabinfo(&broken).is_err());
    }

    /// The capture with one cache's line edited field by field, re-joined with
    /// single spaces; panics if that cache is not in the capture.
    fn with_cache_line(text: &str, name: &str, edit: impl Fn(&mut Vec<&str>)) -> String {
        let mut landed = false;
        let out: String = text
            .lines()
            .map(|line| {
                let mut f: Vec<&str> = line.split_whitespace().collect();
                if f.first() == Some(&name) {
                    landed = true;
                    edit(&mut f);
                    format!("{}\n", f.join(" "))
                } else {
                    format!("{line}\n")
                }
            })
            .collect();
        assert!(landed, "{name} is not in the capture");
        out
    }

    /// Another format version is refused rather than read as 2.1's columns.
    #[test]
    fn a_different_slabinfo_version_is_refused() {
        let other = PI4_SLABINFO.replacen("version: 2.1", "version: 3.0", 1);
        let why = parse_slabinfo(&other).expect_err("not 2.1");
        assert!(why.contains("3.0"), "{why}");
    }

    /// A header with no caches is refused: no running kernel lists none.
    #[test]
    fn a_listing_with_no_caches_is_refused() {
        assert!(parse_slabinfo("slabinfo - version: 2.1\n# name <active_objs>\n").is_err());
    }

    /// **Names linked to one place are one cache.** The link targets are the
    /// form SLUB writes (`:0000192`); `drm_sched_fence` → `:0000192` was read
    /// from dev-01 with `get file /sys/kernel/slab/drm_sched_fence`, and the
    /// rest of this directory is composed around it.
    #[test]
    fn caches_linked_to_one_place_name_each_other_as_merged() {
        let mut caches = parse_slabinfo(PI4_SLABINFO).expect("the capture parses");
        let entries: Vec<SlabEntry> = vec![
            ("filp".into(), Some(":0000192".into())),
            ("drm_sched_fence".into(), Some(":0000192".into())),
            ("cred".into(), Some("../:0000192".into())),
            (":0000192".into(), None),
            ("vmap_area".into(), Some(":0000072".into())),
            ("kmalloc-192".into(), None),
        ];
        attach_merged(&mut caches, &entries);
        assert_eq!(
            named(&caches, "filp").merged.as_deref(),
            Some(&["cred".to_string(), "drm_sched_fence".to_string()][..])
        );
        assert_eq!(named(&caches, "vmap_area").merged.as_deref(), Some(&[][..]));
        assert_eq!(named(&caches, "kmalloc-192").merged.as_deref(), Some(&[][..]));
        assert_eq!(named(&caches, "dentry").merged, None, "the directory did not name it");
    }

    /// A single-digit fraction is tenths, not hundredths: `0.4` is 40, not 4.
    #[test]
    fn a_one_digit_fraction_is_not_read_as_hundredths() {
        assert_eq!(centipercent("some avg10=0.4", "avg10="), Some(40));
        assert_eq!(centipercent("some avg10=3", "avg10="), Some(300));
    }
}
