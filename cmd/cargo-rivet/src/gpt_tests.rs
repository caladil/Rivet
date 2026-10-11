//! Tests for the GPT reader.
//!
//! **The fixture is built here rather than checked in**, so a reader can see
//! exactly which bytes the parser is being asked to believe — and so the
//! "this is not a GPT" case can be produced by changing eight of them.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use std::io::Write;

/// A unique scratch path.
///
/// This crate has no `tempfile` dev-dependency, and adding one for a test is
/// the third row of `AGENTS.md` §1.1. [`crate::scratch`] is the crate's own
/// answer: a path under this process's scratch root, which the next run sweeps.
fn scratch(tag: &str) -> std::path::PathBuf {
    crate::scratch::absent(&format!("gpt-{tag}"))
}

/// A minimal but structurally real GPT: a protective MBR sector, a header at
/// LBA 1 naming a table at LBA 2, and one 128-byte entry per label.
fn image_with(tag: &str, labels: &[&str]) -> (std::path::PathBuf, Vec<(u64, u64)>) {
    let path = scratch(tag);
    let mut f = std::fs::File::create(&path).expect("a temp file");
    let mut extents = Vec::new();

    // LBA 0 — the protective MBR. Deliberately non-zero, because reading THIS
    // as the partition table is the defect the parser exists not to repeat.
    let mut mbr = vec![0u8; SECTOR_BYTES as usize];
    mbr[450] = 0xEE;
    f.write_all(&mbr).expect("mbr");

    // LBA 1 — the header.
    let mut header = vec![0u8; SECTOR_BYTES as usize];
    header[0..8].copy_from_slice(SIGNATURE);
    header[72..80].copy_from_slice(&2u64.to_le_bytes());
    header[80..84].copy_from_slice(&(labels.len() as u32).to_le_bytes());
    header[84..88].copy_from_slice(&128u32.to_le_bytes());
    f.write_all(&header).expect("header");

    // LBA 2 onwards — the entries.
    let mut first = 2048u64;
    for label in labels {
        let mut entry = vec![0u8; 128];
        entry[0] = 0x01; // a non-zero type GUID: the slot is used
        let last = first + 2047;
        entry[32..40].copy_from_slice(&first.to_le_bytes());
        entry[40..48].copy_from_slice(&last.to_le_bytes());
        for (i, unit) in label.encode_utf16().enumerate() {
            let at = 56 + i * 2;
            entry[at..at + 2].copy_from_slice(&unit.to_le_bytes());
        }
        f.write_all(&entry).expect("entry");
        extents.push((first, last));
        first = last + 1;
    }

    f.flush().expect("flush");
    (path, extents)
}

/// **The labels and the extents both come back**, which is the whole job.
#[test]
fn the_partitions_are_read_by_label_and_extent() {
    let (path, extents) = image_with("extents", &["boot-a", "boot-b", "system-a"]);
    let mut handle = std::fs::File::open(&path).expect("open");
    let parts = read(&mut handle).expect("a table");
    let _ = std::fs::remove_file(&path);

    assert_eq!(parts.len(), 3, "{parts:?}");
    assert_eq!(parts[0].label, "boot-a");
    assert_eq!(parts[2].label, "system-a");
    assert_eq!((parts[1].first_lba, parts[1].last_lba), extents[1]);
    assert_eq!(parts[0].offset_bytes(), 2048 * SECTOR_BYTES);
    assert_eq!(parts[0].len_bytes(), 2048 * SECTOR_BYTES);
}

/// **The protective MBR is not the table** — the defect this module replaces.
///
/// The fixture's LBA 0 carries `0xEE` where an MBR entry's type byte would be.
/// A parser reading sector 0 would find one partition covering the disk; this
/// must find three, none of them starting below the first usable LBA.
#[test]
fn the_protective_mbr_at_lba_zero_is_not_mistaken_for_the_table() {
    let (path, _) = image_with("protective", &["boot-a", "boot-b", "system-a"]);
    let mut handle = std::fs::File::open(&path).expect("open");
    let parts = read(&mut handle).expect("a table");
    let _ = std::fs::remove_file(&path);

    assert_eq!(parts.len(), 3);
    assert!(
        parts.iter().all(|p| p.first_lba >= 2048),
        "a partition below the first usable LBA means sector 0 was read: {parts:?}"
    );
}

/// A file with no GPT is refused, and the refusal names the three things it is
/// most likely to actually be.
#[test]
fn a_file_that_is_not_a_gpt_image_is_refused_by_name() {
    let path = scratch("notgpt");
    std::fs::write(&path, vec![0u8; 4096]).expect("zeroes");
    let mut handle = std::fs::File::open(&path).expect("open");
    let why = read(&mut handle).expect_err("must refuse");
    let _ = std::fs::remove_file(&path);

    assert!(why.contains("no GPT header"), "{why}");
    assert!(why.contains("MBR"), "{why}");
}

/// An entry with a type GUID but no name is a real partition with no label,
/// which is a different thing from an unused slot. The parser must not
/// conflate them.
#[test]
fn a_used_entry_with_no_name_is_still_a_partition() {
    let (path, _) = image_with("noname", &["boot-a", "", "system-a"]);
    let mut handle = std::fs::File::open(&path).expect("open");
    let parts = read(&mut handle).expect("a table");
    let _ = std::fs::remove_file(&path);

    assert_eq!(parts.len(), 3);
    assert_eq!(parts[1].label, "");
}

/// The lookup names what it found, because being handed the wrong file is the
/// failure it guards.
#[test]
fn a_missing_label_is_refused_with_the_labels_that_are_there() {
    let (path, _) = image_with("missing", &["boot-a", "system-a"]);
    let mut handle = std::fs::File::open(&path).expect("open");
    let parts = read(&mut handle).expect("a table");
    let _ = std::fs::remove_file(&path);

    let why = by_label(&parts, "boot-b").expect_err("must refuse");
    assert!(why.contains("boot-b"), "{why}");
    assert!(why.contains("boot-a"), "names what it found: {why}");
    assert!(why.contains("system-a"), "names what it found: {why}");
}

/// **A hostile entry size is refused before it is allocated.** These numbers
/// come out of a file.
#[test]
fn an_absurd_entry_size_is_refused_rather_than_allocated() {
    let path = scratch("absurd");
    let mut f = std::fs::File::create(&path).expect("a temp file");
    f.write_all(&vec![0u8; SECTOR_BYTES as usize]).expect("mbr");
    let mut header = vec![0u8; SECTOR_BYTES as usize];
    header[0..8].copy_from_slice(SIGNATURE);
    header[72..80].copy_from_slice(&2u64.to_le_bytes());
    header[80..84].copy_from_slice(&1u32.to_le_bytes());
    header[84..88].copy_from_slice(&u32::MAX.to_le_bytes());
    f.write_all(&header).expect("header");
    f.flush().expect("flush");
    drop(f);

    let mut handle = std::fs::File::open(&path).expect("open");
    let why = read(&mut handle).expect_err("must refuse");
    let _ = std::fs::remove_file(&path);
    assert!(why.contains("4096"), "{why}");
}
