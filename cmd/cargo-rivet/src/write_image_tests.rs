//! Tests for `cargo rivet provision`.
//!
//! **No real block device is ever touched here, and cannot be** — the
//! session that wrote this file is bound by the same rule it is proving:
//! never write to a real device, never run with `confirm` against one. What
//! IS tested, against a temporary FILE standing in for a device, is
//! [`write_bytes_and_verify`] itself — the actual open, write, hash, sync,
//! reopen, read-back and compare — plus every decision that gates it:
//! removability, the retyped-identifier confirmation, a short write, and a
//! digest that does not match.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|s| (*s).to_string()).collect()
}

/// **Without `confirm`, nothing is written.** The single most important
/// property in this file.
#[test]
fn confirm_is_absent_by_default() {
    let parsed = parse(&args(&["--device", "/dev/sdz"])).expect("parses");
    assert!(
        !parsed.confirm,
        "a device write must never be armed by naming a device"
    );
}

/// `confirm` is accepted wherever it appears, because refusing it on word
/// order teaches nothing and the person has already said the dangerous word.
#[test]
fn confirm_is_taken_in_any_position() {
    for words in [
        args(&["confirm", "--device", "/dev/sdz"]),
        args(&["--device", "/dev/sdz", "confirm"]),
    ] {
        assert!(parse(&words).expect("parses").confirm, "{words:?}");
    }
}

/// An option with no value is refused by name rather than silently ignored —
/// `--device` swallowing the next flag is how a wrong disk gets written.
#[test]
fn an_option_with_no_value_is_refused_by_name() {
    let why = parse(&args(&["--device"])).expect_err("must refuse");
    assert!(why.contains("--device"), "{why}");
}

/// An unknown word is refused with the usage line, not treated as a path.
#[test]
fn an_unknown_word_is_refused_and_shows_the_usage() {
    let why = parse(&args(&["--wipe"])).expect_err("must refuse");
    assert!(why.contains("--wipe"), "{why}");
    assert!(why.contains("usage:"), "{why}");
}

/// **A single candidate is taken without asking.** Asking a question with one
/// answer trains a person to press Return without reading, which is what makes
/// the confirmation prompt worthless later.
#[test]
fn one_candidate_is_chosen_without_a_prompt() {
    let only = vec!["out/rivet-0.1.0-card.img".to_string()];
    assert_eq!(
        choose("card image", &only, "Pass --image <path>.").expect("chooses"),
        Choice::Selected(0)
    );
}

/// An empty list is a refusal that names what was being looked for, because
/// the caller turns it into advice about which command produces one.
#[test]
fn no_candidates_is_a_refusal_naming_what_was_wanted() {
    let why =
        choose("card image", &[], "Pass --image <path>.").expect_err("must refuse");
    assert!(why.contains("card image"), "{why}");
}

// ---------------------------------------------------------------------------
// `x`/`X` leaves a `choose_with` menu cleanly. [`parse_choice`] is the
// parsing half, pure and tested directly with no stdin; the I/O half
// (`choose_with` itself) stays untested, like every other menu in this file.
// ---------------------------------------------------------------------------

#[test]
fn a_bare_number_is_selected_one_based() {
    assert_eq!(parse_choice("2", 3), Ok(Choice::Selected(1)));
}

#[test]
fn an_empty_line_takes_the_default_first_entry() {
    assert_eq!(parse_choice("", 3), Ok(Choice::Selected(0)));
    assert_eq!(parse_choice("   ", 3), Ok(Choice::Selected(0)));
}

#[test]
fn x_or_capital_x_asks_to_exit() {
    assert_eq!(parse_choice("x", 3), Ok(Choice::Exit));
    assert_eq!(parse_choice("X", 3), Ok(Choice::Exit));
    // Surrounding whitespace, exactly as a number or a blank line already
    // tolerates it — `read_line` keeps the trailing newline.
    assert_eq!(parse_choice(" x\n", 3), Ok(Choice::Exit));
}

/// `x` is checked before the `usize` parse, so it is never refused as "not
/// a number" — it is the menu's own exit, not an invalid answer.
#[test]
fn x_is_not_treated_as_a_parse_failure() {
    let result = parse_choice("x", 5);
    assert!(result.is_ok(), "x must not be refused as \"not a number\": {result:?}");
    assert_eq!(result, Ok(Choice::Exit));
}

#[test]
fn a_number_out_of_range_is_still_refused() {
    let why = parse_choice("9", 3).expect_err("out of range");
    assert!(why.contains('9'), "{why}");
    assert!(why.contains("1 and 3"), "{why}");
}

#[test]
fn text_that_is_neither_empty_x_nor_a_number_is_refused() {
    let why = parse_choice("banana", 3).expect_err("not a number, not x");
    assert!(why.contains("banana"), "{why}");
}

// -----------------------------------------------------------------------
// `PLAN-Feature-Set-2026-09-12` §7.2: no interactive prompt without a
// terminal. `refuse_unless_terminal` takes `is_terminal` as a parameter
// rather than reading this process's own stdin, precisely so these tests do
// not depend on whether `cargo test` itself happens to have a terminal —
// see that function's own documentation.
// -----------------------------------------------------------------------

#[test]
fn a_terminal_is_never_refused() {
    assert_eq!(refuse_unless_terminal(true, "writing the device", "Add confirm."), Ok(()));
}

/// **The property this whole mechanism exists for.** The refusal names both
/// what was being asked and the flag that reaches it without a prompt.
#[test]
fn no_terminal_is_refused_and_names_what_and_the_way_past_it() {
    let refused = refuse_unless_terminal(false, "writing the device", "Add `confirm`.")
        .expect_err("no terminal");
    assert!(refused.contains("writing the device"), "{refused}");
    assert!(refused.contains("Add `confirm`"), "{refused}");
    assert!(refused.contains("no terminal"), "{refused}");
}

/// **More than one candidate is the one case that would otherwise read
/// stdin.** Broken deliberately first (`AGENTS.md` §4): before
/// `refuse_unless_terminal` was wired into `choose_with`, a closed stdin's
/// EOF read as an empty line and silently took the FIRST candidate — the
/// bare-Return default, applied to a run that never asked for one.
#[test]
fn more_than_one_candidate_with_no_terminal_is_refused_and_names_the_flag() {
    let many = vec![
        "out/rivet-0.1.0-card.img".to_string(),
        "out/rivet-0.1.1-card.img".to_string(),
    ];
    let why = choose_with(false, "card image", &many, "Pass --image <path>.")
        .expect_err("no terminal");
    assert!(why.contains("no terminal"), "{why}");
    assert!(why.contains("--image"), "{why}");
}

/// A terminal still reaches the same refusal it always would have for
/// invalid input — the check adds a gate before the menu, and does not
/// replace anything after it. `is_char_boundary` reads the empty line "" as
/// invalid rather than the default here, because stdin cannot be simulated
/// as a terminal in this test and the read itself still runs; the point
/// under test is only that `is_terminal: true` does not short-circuit.
#[test]
fn a_terminal_does_not_skip_the_menu_it_only_skips_the_refusal() {
    // With one candidate, neither the terminal check nor a read ever runs —
    // this asserts the refusal introduced above does not fire when it
    // should not, for the branch that bypasses stdin entirely.
    let one = vec!["out/rivet-0.1.0-card.img".to_string()];
    assert_eq!(
        choose_with(false, "card image", &one, "Pass --image <path>.").expect("one"),
        Choice::Selected(0)
    );
    assert_eq!(
        choose_with(true, "card image", &one, "Pass --image <path>.").expect("one"),
        Choice::Selected(0)
    );
}

/// [`choose_device`]'s own wiring: no default to fall through to, so without
/// the no-terminal refusal a closed stdin would land on
/// `"" is not a number"` — a worse message for the same underlying problem.
#[test]
fn choose_device_with_no_terminal_is_refused_and_names_device() {
    let many = vec!["disk 0 (32 GB)".to_string(), "disk 1 (64 GB)".to_string()];
    let why = choose_device_with(false, &many).expect_err("no terminal");
    assert!(why.contains("--device"), "{why}");
}

/// An empty list is no longer a dead end — with `r`/`x` always offered it is
/// a pass the operator rescans or exits from — so the no-terminal refusal is
/// what fires when there is no terminal, whether or not any device was seen.
#[test]
fn choose_device_with_no_terminal_is_refused_even_when_the_list_is_empty() {
    let why = choose_device_with(false, &[]).expect_err("no terminal");
    assert!(why.contains("--device"), "{why}");
}

#[test]
fn parse_device_choice_reads_the_two_letter_escape_hatches_either_spelling() {
    for token in ["r", "R", "rescan", "  Rescan  "] {
        assert_eq!(parse_device_choice(token, 3), Ok(DeviceChoice::Rescan), "{token:?}");
    }
    for token in ["x", "X", "exit", "  Exit\n"] {
        assert_eq!(parse_device_choice(token, 3), Ok(DeviceChoice::Exit), "{token:?}");
    }
}

#[test]
fn parse_device_choice_reads_a_one_based_device_number() {
    assert_eq!(parse_device_choice("1", 3), Ok(DeviceChoice::Selected(0)));
    assert_eq!(parse_device_choice(" 3 ", 3), Ok(DeviceChoice::Selected(2)));
}

#[test]
fn parse_device_choice_refuses_a_number_out_of_range_and_zero() {
    assert!(parse_device_choice("4", 3).is_err());
    assert!(parse_device_choice("0", 3).is_err());
}

#[test]
fn parse_device_choice_has_no_default_so_a_bare_return_is_refused() {
    let why = parse_device_choice("", 3).expect_err("no default");
    assert!(why.contains("no default"), "{why}");
}

/// With no devices, a number cannot be valid — only `r`/`x` are — and the
/// refusal says so rather than offering a range of one.
#[test]
fn parse_device_choice_with_no_devices_takes_only_r_or_x() {
    assert_eq!(parse_device_choice("r", 0), Ok(DeviceChoice::Rescan));
    assert_eq!(parse_device_choice("x", 0), Ok(DeviceChoice::Exit));
    let why = parse_device_choice("1", 0).expect_err("no devices");
    assert!(why.contains("rescan") && why.contains("exit"), "{why}");
}

/// **The write path.** With no terminal and no `confirm`, nothing is
/// measured for elevation and nothing is written — the refusal fires before
/// `Write it? [y/N]` would ever print, which is provable because this device
/// is never actually written to.
#[test]
fn write_and_verify_with_no_terminal_and_no_confirm_is_refused_before_writing_anything() {
    let scratch = crate::scratch::absent("provision-test");
    std::fs::write(&scratch, b"not a real image, just some bytes").expect("writable");
    let disk = Disk {
        id: "\\\\.\\PHYSICALDRIVE99".to_string(),
        model: "test".to_string(),
        size_bytes: 0,
        removable: true,
        letters: Vec::new(),
    };
    let refused = write_and_verify_with(false, &scratch, &disk, false, b"doc bytes")
        .expect_err("no terminal and not confirmed");
    assert!(refused.contains("writing the device"), "{refused}");
    assert!(refused.contains("confirm"), "{refused}");
    let _ = std::fs::remove_file(&scratch);
}

/// **`cargo rivet provision`'s bare interactive run (2026-09-14).** With a
/// terminal, no `--device` and no `confirm`, the removable devices are offered
/// as a menu — it no longer stops at a prepared image. `--device` still wins,
/// `confirm` still offers the menu, and `write-image` on its own (no offer)
/// still prepares the image and writes nothing.
#[test]
fn the_device_step_offers_the_menu_to_an_interactive_provision_run() {
    assert_eq!(device_step(false, false, true), DeviceStep::Menu, "the bare interactive provision run");
    assert_eq!(device_step(false, true, false), DeviceStep::Menu, "confirm with no --device");
    assert_eq!(device_step(false, false, false), DeviceStep::NoDevice, "write-image alone, or no terminal");
    assert_eq!(device_step(true, false, true), DeviceStep::Named, "--device wins");
    assert_eq!(device_step(true, true, false), DeviceStep::Named, "--device wins");
}

/// **Without `confirm`, `Write it? [y/N]` is really asked**, and `N` writes
/// nothing, saying so.
#[test]
fn without_confirm_the_write_question_is_asked_and_no_writes_nothing() {
    static ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    fn answer_no() -> Result<bool, String> {
        ASKED.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(false)
    }
    let declined = confirm_write(true, false, "\\\\.\\PHYSICALDRIVE99", answer_no).expect("a terminal to ask at");
    assert!(ASKED.load(std::sync::atomic::Ordering::SeqCst), "the question must be asked");
    let report = declined.expect("N must decline the write");
    assert!(report.starts_with("nothing was written"), "{report}");
}

/// **`confirm` already answered the question**, so nothing is asked — even
/// with no terminal at all.
#[test]
fn confirm_skips_the_write_question() {
    fn must_not_ask() -> Result<bool, String> {
        panic!("confirm was given: Write it? [y/N] must not be asked")
    }
    assert_eq!(confirm_write(false, true, "\\\\.\\PHYSICALDRIVE99", must_not_ask), Ok(None));
    assert_eq!(confirm_write(true, true, "\\\\.\\PHYSICALDRIVE99", must_not_ask), Ok(None));
}

/// The usage text says the dangerous thing in the words a person needs, and
/// this asserts it because it is the only warning before an irreversible act.
#[test]
fn the_usage_says_that_writing_destroys_the_device() {
    let text = usage();
    assert!(text.contains("DESTROYS"), "{text}");
    assert!(text.contains("confirm"), "{text}");
}

/// **The boot label list is both slots.** A document on only one of them is a
/// machine that forgets its configuration at its first update.
#[test]
fn both_boot_partitions_are_written() {
    assert_eq!(BOOT_LABELS, ["boot-a", "boot-b"]);
}

/// The partition window reads and writes within its own extent and refuses to
/// run past the end — the property that stops a full filesystem spilling into
/// the next partition.
#[test]
fn the_partition_window_stays_inside_its_extent() {
    let path = crate::scratch::absent("win");
    std::fs::write(&path, vec![0xAAu8; 4096]).expect("a file");
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("open");

    {
        let mut view = Partition::new(&mut file, 1024, 512);
        // Writing more than the window holds is a short write, then refused.
        let wrote = view.write(&[0x11u8; 4096]).expect("short write");
        assert_eq!(wrote, 512, "the window must not write past its extent");
        assert!(view.write(&[0x11u8; 1]).is_err(), "full means full");
    }

    let all = std::fs::read(&path).expect("read back");
    let _ = std::fs::remove_file(&path);
    assert_eq!(all[1023], 0xAA, "wrote before the window");
    assert_eq!(all[1024], 0x11, "did not write at the window's start");
    assert_eq!(all[1535], 0x11, "did not write to the window's end");
    assert_eq!(all[1536], 0xAA, "wrote past the window");
}

// ---------------------------------------------------------------------------
// The two pre-checks, 2026-09-05
// ---------------------------------------------------------------------------

fn scratch(what: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    crate::scratch::absent(&format!(
        "prov-{what}-{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// **A cold host is refused before an image is even chosen.**
///
/// The `--image` and `--document` are deliberately absent: if the credential
/// check ran late, this call would block on `choose`'s prompt or fail naming a
/// missing image, and either would prove the check is in the wrong place.
#[test]
fn a_cold_host_is_refused_before_anything_is_chosen() {
    let at = scratch("cold");
    let why = run(&ProvisionArgs {
        image_path: None,
        document_path: None,
        device_path: None,
        confirm: false,
        not_mine: false,
        at: Some(at.clone()),
        verify_only: false,
        elevated_write_task: None,
        elevated_verify_task: None,
        elevated_task_digest: None,
        staging_path: None,
        identity_path: None,
        fleet_repo_root: None,
        offer_device_menu: false,
        image_from_repository: false,
    })
    .expect_err("a cold host must be refused");

    assert!(why.starts_with("NOTHING WAS WRITTEN"), "{why}");
    assert!(why.contains("cargo rivet setup"), "{why}");
    assert!(!at.exists(), "the refusal created {}", at.display());
}

/// **A GPT image with a real FAT `boot-a` carrying a `peers` file.**
///
/// Built rather than checked in, for `gpt_tests.rs`'s stated reason: a reader
/// can see which bytes the check is being asked to believe. The partition is
/// large enough for `fatfs` to format — a FAT12 volume below about a megabyte
/// is refused by the formatter, not by this project.
fn image_with_peers(tag: &str, peers: Option<&str>) -> PathBuf {
    use std::io::Write as _;

    const BOOT_SECTORS: u64 = 8192; // 4 MiB
    let path = scratch(tag).with_extension("img");
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");

    let first = 2048u64;
    let last = first + BOOT_SECTORS - 1;
    {
        let mut file = File::create(&path).expect("a temp image");
        let mut mbr = vec![0u8; gpt::SECTOR_BYTES as usize];
        mbr[450] = 0xEE;
        file.write_all(&mbr).expect("mbr");

        let mut header = vec![0u8; gpt::SECTOR_BYTES as usize];
        header[0..8].copy_from_slice(b"EFI PART");
        header[72..80].copy_from_slice(&2u64.to_le_bytes());
        header[80..84].copy_from_slice(&1u32.to_le_bytes());
        header[84..88].copy_from_slice(&128u32.to_le_bytes());
        file.write_all(&header).expect("header");

        let mut entry = vec![0u8; 128];
        entry[0] = 0x01;
        entry[32..40].copy_from_slice(&first.to_le_bytes());
        entry[40..48].copy_from_slice(&last.to_le_bytes());
        for (i, unit) in BOOT_LABELS[0].encode_utf16().enumerate() {
            let byte = 56 + i * 2;
            entry[byte..byte + 2].copy_from_slice(&unit.to_le_bytes());
        }
        file.write_all(&entry).expect("entry");
        file.set_len((last + 1) * gpt::SECTOR_BYTES).expect("size");
    }

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("open");
    {
        let mut view = Partition::new(&mut file, first * gpt::SECTOR_BYTES, BOOT_SECTORS * gpt::SECTOR_BYTES);
        fatfs::format_volume(&mut view, fatfs::FormatVolumeOptions::new()).expect("format");
    }
    if let Some(body) = peers {
        let mut view = Partition::new(&mut file, first * gpt::SECTOR_BYTES, BOOT_SECTORS * gpt::SECTOR_BYTES);
        let fs = fatfs::FileSystem::new(&mut view, fatfs::FsOptions::new()).expect("a filesystem");
        let mut out = fs
            .root_dir()
            .create_file(credentials::PEERS_FILE)
            .expect("create");
        out.write_all(body.as_bytes()).expect("write");
    }
    path
}

/// **A GPT image with REAL `boot-a` AND `boot-b` FAT partitions** —
/// `image_with_peers`'s single-partition shape, doubled, for `inject`'s own
/// two-partition contract (`both_boot_partitions_are_written`). Each
/// partition is `sectors_per_part` sectors; both start formatted, empty.
fn two_partition_image(tag: &str, sectors_per_part: u64) -> PathBuf {
    use std::io::Write as _;

    let path = scratch(tag).with_extension("img");
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");

    let first_a = 2048u64;
    let last_a = first_a + sectors_per_part - 1;
    let first_b = last_a + 1;
    let last_b = first_b + sectors_per_part - 1;
    {
        let mut file = File::create(&path).expect("a temp image");
        let mut mbr = vec![0u8; gpt::SECTOR_BYTES as usize];
        mbr[450] = 0xEE;
        file.write_all(&mbr).expect("mbr");

        let mut header = vec![0u8; gpt::SECTOR_BYTES as usize];
        header[0..8].copy_from_slice(b"EFI PART");
        header[72..80].copy_from_slice(&2u64.to_le_bytes()); // entry_lba
        header[80..84].copy_from_slice(&2u32.to_le_bytes()); // count
        header[84..88].copy_from_slice(&128u32.to_le_bytes()); // entry_size
        file.write_all(&header).expect("header");

        let mut entries = vec![0u8; 256];
        for (slot, (label, first, last)) in
            [(BOOT_LABELS[0], first_a, last_a), (BOOT_LABELS[1], first_b, last_b)].iter().enumerate()
        {
            let base = slot * 128;
            entries[base] = 0x01; // a non-zero type GUID: not an unused slot
            entries[base + 32..base + 40].copy_from_slice(&first.to_le_bytes());
            entries[base + 40..base + 48].copy_from_slice(&last.to_le_bytes());
            for (i, unit) in label.encode_utf16().enumerate() {
                let byte = base + 56 + i * 2;
                entries[byte..byte + 2].copy_from_slice(&unit.to_le_bytes());
            }
        }
        file.write_all(&entries).expect("entries");
        file.set_len((last_b + 1) * gpt::SECTOR_BYTES).expect("size");
    }

    let mut file = OpenOptions::new().read(true).write(true).open(&path).expect("open");
    for (first, last) in [(first_a, last_a), (first_b, last_b)] {
        let mut view = Partition::new(&mut file, first * gpt::SECTOR_BYTES, (last - first + 1) * gpt::SECTOR_BYTES);
        fatfs::format_volume(&mut view, fatfs::FormatVolumeOptions::new()).expect("format");
    }
    path
}

/// Mint a real operator identity in `at`, and answer its public key.
fn operator_in(at: &Path) -> String {
    crate::identity::run(&crate::identity::IdentityArgs {
        verb: crate::identity::Verb::New,
        directory: at.join("identity").join("operator"),
        name: "an-operator".to_owned(),
    })
    .expect("a fresh directory");
    Survey::take(at, &Overrides::default())
        .named(credentials::OPERATOR_CREDENTIAL)
        .expect("in the set")
        .public_key()
        .expect("just minted")
}

/// **The check that could only be made here**, in all three directions: the
/// image names you, the image names somebody else, the image names nobody.
#[test]
fn the_images_own_peers_file_is_read_and_compared() {
    let at = scratch("enroll");
    let mine = operator_in(&at);
    let survey = Survey::take(&at, &Overrides::default());

    let names = image_with_peers("names", Some(&format!("{mine}\n")));
    assert_eq!(enrollment_of(&names, &survey).expect("readable"), Enrollment::Names);

    let stranger = "0".repeat(64);
    let excluded = image_with_peers("excluded", Some(&format!("{stranger}\n# a comment\n\n")));
    assert_eq!(
        enrollment_of(&excluded, &survey).expect("readable"),
        Enrollment::Excluded(1),
        "a comment and a blank line are not keys"
    );

    let nobody = image_with_peers("nobody", None);
    assert_eq!(enrollment_of(&nobody, &survey).expect("readable"), Enrollment::Absent);
}

/// Hex is compared without regard to case, because a `peers` file is written by
/// hand as often as by a tool and neither half of a pair is wrong.
#[test]
fn a_peers_file_in_upper_case_hex_still_names_you() {
    let at = scratch("case");
    let mine = operator_in(&at);
    let survey = Survey::take(&at, &Overrides::default());
    let image = image_with_peers("case", Some(&format!("  {}  \n", mine.to_uppercase())));
    assert_eq!(enrollment_of(&image, &survey).expect("readable"), Enrollment::Names);
}

/// **The refusal says what the board will do, and how to fix it.** A refusal
/// that only says *no* sends a person back to a build they have no reason to
/// think was wrong.
#[test]
fn the_enrollment_refusal_names_the_repair() {
    let image = Path::new("out/a-card.img");
    let why = Enrollment::Excluded(2)
        .refusal(false, image)
        .expect("must refuse");
    assert!(why.starts_with("NOTHING WAS WRITTEN"), "{why}");
    assert!(why.contains("2 key(s)"), "{why}");
    assert!(why.contains("RIVET_IDENTITY"), "{why}");
    assert!(why.contains("cargo rivet setup"), "{why}");
    assert!(why.contains("--not-mine"), "{why}");

    let absent = Enrollment::Absent.refusal(false, image).expect("must refuse");
    assert!(absent.contains("rivet-mcpd will not start"), "{absent}");
}

/// `--not-mine` is the only way past it, and it is not implied by anything.
#[test]
fn not_mine_is_the_only_way_past_the_enrollment_refusal() {
    let image = Path::new("out/a-card.img");
    assert!(Enrollment::Absent.refusal(true, image).is_none());
    assert!(Enrollment::Names.refusal(false, image).is_none());
    assert!(!parse(&args(&["--device", "/dev/sdz"])).expect("parses").not_mine);
    assert!(parse(&args(&["--not-mine"])).expect("parses").not_mine);
}

/// **A check that could not be made says so and never passes silently.**
/// `AGENTS.md` §7: an error and a true negative must not look the same.
#[test]
fn an_unmakeable_check_reports_that_it_could_not_be_made() {
    let note = Enrollment::Unknown("no operator certificate".to_owned()).note();
    assert!(note.contains("could NOT be made"), "{note}");
    assert!(note.contains("no operator certificate"), "{note}");

    // With no operator identity on disk there is nothing to compare against,
    // and the answer is Unknown rather than Names.
    let at = scratch("unknown");
    let survey = Survey::take(&at, &Overrides::default());
    let image = image_with_peers("unknown", Some("0000\n"));
    assert!(matches!(
        enrollment_of(&image, &survey).expect("readable"),
        Enrollment::Unknown(_)
    ));
}

/// **A document that does not parse never reaches a boot partition.**
///
/// `SPEC-05`'s History, 2026-09-07. This read the file as bytes and wrote them to both boot
/// partitions with nothing looking at them, so a typo became a card that
/// refuses on first boot — discovered on a headless board, and repaired only by
/// writing the card again.
///
/// The refusal has to carry the schema's own words: *"it did not parse"* sends
/// somebody back to a file they have already read.
#[test]
fn a_document_the_schema_rejects_is_refused_before_anything_is_written() {
    let refused = parsed_note(Path::new("bad.toml"), b"version = 1\nhostname = 4\n")
        .expect_err("a hostname that is not a string must not pass");
    assert!(refused.contains("NOTHING WAS WRITTEN"), "{refused}");
    assert!(refused.contains("bad.toml"), "{refused}");

    // Bytes that are not text at all are the other half, and they arrive from
    // the same place: a `--document` pointed at an image, a key, or a `.rvt`.
    let refused = parsed_note(Path::new("bad.toml"), &[0xff, 0xfe, 0x00])
        .expect_err("a binary file is not a document");
    assert!(refused.contains("UTF-8"), "{refused}");
}

/// And a document that DOES parse is reported as four facts rather than a tick,
/// because a person about to destroy a card wants to see the hostname they
/// expected — `SPEC-05` §4's argument, at the other end of the journey.
#[test]
fn a_document_that_parses_is_reported_as_what_it_says() {
    let (_, note) = parsed_note(
        Path::new("arbor.toml"),
        b"version = 1\nhostname = \"arbor\"\n\n[control-plane]\nlisten = \"0.0.0.0:7777\"\n",
    )
    .expect("this document parses");
    assert!(note.contains("arbor"), "{note}");
    assert!(note.contains("0.0.0.0:7777"), "{note}");
    assert!(note.contains("the same parser the board refuses with"), "{note}");
}

/// **D22 (`.rulings/` R-210): a `cpu` cap the kernel will not keep never
/// reaches a boot partition.** The document parses cleanly — this is a
/// different refusal from `parsed_note`'s own — but cgroup v2 would refuse
/// the cap at boot with no cause named, on the headless hardware this
/// module's own history is about.
#[test]
fn a_cpu_cap_the_kernel_will_not_keep_is_refused_before_anything_is_written() {
    let (document, _) = parsed_note(
        Path::new("arbor.toml"),
        b"version = 1\nhostname = \"arbor\"\n\n[services.photo-index]\nenabled = true\ncpu = \"0.005\"\n",
    )
    .expect("this document parses");
    let why = refuse_unenforceable_cpu_cap(Path::new("arbor.toml"), &document)
        .expect_err("a cpu cap below the kernel minimum must be refused");
    assert!(why.contains("NOTHING WAS WRITTEN"), "{why}");
    assert!(why.contains("photo-index"), "{why}");
    assert!(why.contains("cpu"), "{why}");
}

/// The kernel's own minimum, and no cap at all, both still write.
#[test]
fn an_enforceable_cpu_cap_and_no_cap_at_all_both_still_write() {
    let (at_minimum, _) = parsed_note(
        Path::new("arbor.toml"),
        b"version = 1\nhostname = \"arbor\"\n\n[services.photo-index]\nenabled = true\ncpu = \"0.01\"\n",
    )
    .expect("this document parses");
    assert!(refuse_unenforceable_cpu_cap(Path::new("arbor.toml"), &at_minimum).is_ok());

    let (none, _) = parsed_note(
        Path::new("arbor.toml"),
        b"version = 1\nhostname = \"arbor\"\n\n[services.photo-index]\nenabled = true\n",
    )
    .expect("this document parses");
    assert!(refuse_unenforceable_cpu_cap(Path::new("arbor.toml"), &none).is_ok());
}

// ---------------------------------------------------------------------------
// Wave C3 (R-178 item 1): `inject` widened beyond `rivet.toml`.
// ---------------------------------------------------------------------------

/// **Every extra file lands on BOTH boot partitions, byte for byte.** The
/// core of item 1: `crate::carry::gather`'s output is not special-cased —
/// `inject` places it exactly the way it has always placed `rivet.toml`.
#[test]
fn inject_widens_both_boot_partitions_with_every_extra_file() {
    let image = two_partition_image("inject-widen", 8192); // 4 MiB per partition
    let doc = b"version = 1\nhostname = \"arbor\"\n".to_vec();
    let extras = vec![
        (rivet_core::staging::BUNDLE_FILE.to_owned(), b"version = 1\n".to_vec()),
        ("device.seed".to_owned(), vec![0xABu8; 32]),
        (rivet_core::carried::card_file("rivet-proxyd"), vec![0xCDu8; 4096]),
    ];

    inject(&image, &doc, &extras).expect("every file fits a 4 MiB partition");

    let mut file = OpenOptions::new().read(true).write(true).open(&image).expect("reopen");
    let parts = gpt::read(&mut file).expect("a readable GPT");
    for label in BOOT_LABELS {
        let part = gpt::by_label(&parts, label).expect("the label exists");
        let mut view = Partition::new(&mut file, part.offset_bytes(), part.len_bytes());
        let fs = fatfs::FileSystem::new(&mut view, fatfs::FsOptions::new()).expect("a filesystem");
        let root = fs.root_dir();

        let mut got = Vec::new();
        root.open_file(DOCUMENT_NAME).expect("rivet.toml").read_to_end(&mut got).expect("read");
        assert_eq!(got, doc, "{label}'s rivet.toml");

        for (name, bytes) in &extras {
            let mut got = Vec::new();
            root.open_file(name)
                .unwrap_or_else(|e| panic!("{label} is missing {name}: {e}"))
                .read_to_end(&mut got)
                .expect("read");
            assert_eq!(&got, bytes, "{label}'s {name}");
        }
    }
    let _ = std::fs::remove_file(&image);
}

// ---------------------------------------------------------------------------
// The gate, 2026-09-14: the enrollment check reads the peers file the CARD
// will carry, not only the image's.
// ---------------------------------------------------------------------------

/// A complete scratch `~/.rivet` — every credential `run`'s survey requires,
/// the board identity included — minted by `cargo rivet setup`'s own path.
fn complete_host(at: &Path) {
    crate::setup::run(&crate::setup::SetupArgs {
        at: Some(at.to_path_buf()),
        name: Some("gate-test-operator".to_owned()),
        check: false,
        over: Overrides::default(),
    })
    .expect("a cold setup run mints every required credential");
}

fn generic_run_args(at: &Path, image: &Path, document: &Path) -> ProvisionArgs {
    ProvisionArgs {
        image_path: Some(image.to_path_buf()),
        document_path: Some(document.to_path_buf()),
        device_path: None,
        confirm: false,
        not_mine: false,
        at: Some(at.to_path_buf()),
        verify_only: false,
        elevated_write_task: None,
        elevated_verify_task: None,
        elevated_task_digest: None,
        staging_path: None,
        identity_path: None,
        fleet_repo_root: None,
        offer_device_menu: false,
        image_from_repository: false,
    }
}

fn boot_file(image: &Path, label: &str, name: &str) -> Option<Vec<u8>> {
    let mut file = OpenOptions::new().read(true).write(true).open(image).expect("reopen");
    let parts = gpt::read(&mut file).expect("a readable GPT");
    let part = gpt::by_label(&parts, label).expect("the label exists");
    let mut view = Partition::new(&mut file, part.offset_bytes(), part.len_bytes());
    let fs = fatfs::FileSystem::new(&mut view, fatfs::FsOptions::new()).expect("a filesystem");
    let mut got = Vec::new();
    fs.root_dir().open_file(name).ok()?.read_to_end(&mut got).expect("read");
    Some(got)
}

/// **The generic image a repository publishes carries no `peers` file**
/// (`R-175`: an image is never built for one computer), and provisioning puts
/// this card's identity — `peers` included — onto both boot partitions at
/// write time. Found by the gate rehearsal of 2026-09-14: the check read only
/// the image, so every real `cargo rivet provision` of a published image was
/// refused with "carries NO peers file" after the interview, and only
/// `--not-mine` (which every earlier proof passed) got past it.
#[test]
fn a_generic_image_is_not_refused_when_the_card_carries_a_peers_file_naming_you() {
    let at = scratch("generic-peers");
    complete_host(&at);
    let image = two_partition_image("generic-peers-img", 8192);
    let document = at.join("example-standalone.provisioning.toml");
    std::fs::write(&document, "version = 1\nhostname = \"example-standalone\"\n").expect("a document");

    let report = run(&generic_run_args(&at, &image, &document))
        .unwrap_or_else(|e| panic!("a generic image with the identity injected must not be refused: {e}"));
    assert!(report.contains("the image is ready"), "{report}");

    let identity_peers = std::fs::read(at.join("identity").join("board").join(credentials::PEERS_FILE))
        .expect("setup wrote the board's peers");
    for label in BOOT_LABELS {
        assert_eq!(
            boot_file(&image, label, credentials::PEERS_FILE).as_deref(),
            Some(identity_peers.as_slice()),
            "{label} must carry the identity's peers file"
        );
    }
    let _ = std::fs::remove_file(&image);
    let _ = std::fs::remove_dir_all(&at);
}

/// **The other direction: the image names you, the card will not.** The
/// injected `peers` replaces the image's on both partitions, so an identity
/// whose `peers` omits the operator is a card nobody can reach — refused
/// before anything is injected, naming the file that is actually at fault.
#[test]
fn an_injected_peers_file_that_omits_you_is_refused_even_when_the_image_names_you() {
    let at = scratch("injected-excludes");
    let mine = operator_in(&at);
    complete_host(&at);
    let board_peers = at.join("identity").join("board").join(credentials::PEERS_FILE);
    std::fs::write(&board_peers, format!("{}\n", "0".repeat(64))).expect("a stranger's peers");

    let image = two_partition_image("injected-excludes-img", 8192);
    {
        let mut file = OpenOptions::new().read(true).write(true).open(&image).expect("reopen");
        let parts = gpt::read(&mut file).expect("gpt");
        let part = gpt::by_label(&parts, BOOT_LABELS[0]).expect("boot-a");
        let mut view = Partition::new(&mut file, part.offset_bytes(), part.len_bytes());
        let fs = fatfs::FileSystem::new(&mut view, fatfs::FsOptions::new()).expect("fs");
        let mut out = fs.root_dir().create_file(credentials::PEERS_FILE).expect("create");
        out.write_all(format!("{mine}\n").as_bytes()).expect("write");
    }
    let document = at.join("example-standalone.provisioning.toml");
    std::fs::write(&document, "version = 1\nhostname = \"example-standalone\"\n").expect("a document");

    let why = run(&generic_run_args(&at, &image, &document)).expect_err("the card would refuse you");
    assert!(why.starts_with("NOTHING WAS WRITTEN"), "{why}");
    assert!(why.contains("none of them is yours"), "{why}");
    assert!(why.contains(&board_peers.display().to_string()), "must name the peers file at fault: {why}");
    assert!(
        boot_file(&image, BOOT_LABELS[0], DOCUMENT_NAME).is_none(),
        "nothing may be injected before the refusal"
    );
    let _ = std::fs::remove_file(&image);
    let _ = std::fs::remove_dir_all(&at);
}

/// **Refused by name, before either partition is touched at all**, when a
/// file will not fit. Proven with EQUAL-sized partitions, so a real card's
/// own symmetry (`ADR-018` §4: both slots are the same size) means the first
/// partition's own check is the one that actually gates this — see
/// `inject`'s own doc comment for why per-partition is the strongest
/// guarantee available without a slower two-pass plan-then-write.
#[test]
fn inject_refuses_by_name_when_a_file_will_not_fit_and_writes_nothing_to_either_partition() {
    let image = two_partition_image("inject-too-big", 8192); // 4 MiB per partition
    let doc = b"version = 1\n".to_vec();
    let extras = vec![("app-huge.rvt".to_owned(), vec![0u8; 8 * 1024 * 1024])]; // bigger than the partition itself

    let err = inject(&image, &doc, &extras).expect_err("8 MiB does not fit a 4 MiB partition");
    assert!(err.contains("boot-a"), "{err}");
    assert!(err.contains("app-huge.rvt"), "{err}");
    assert!(err.contains("nothing was written"), "{err}");

    let mut file = OpenOptions::new().read(true).write(true).open(&image).expect("reopen");
    let parts = gpt::read(&mut file).expect("a readable GPT");
    let part = gpt::by_label(&parts, BOOT_LABELS[0]).expect("boot-a exists");
    let mut view = Partition::new(&mut file, part.offset_bytes(), part.len_bytes());
    let fs = fatfs::FileSystem::new(&mut view, fatfs::FsOptions::new()).expect("still a filesystem");
    assert!(fs.root_dir().open_file(DOCUMENT_NAME).is_err(), "rivet.toml must not have been written");
    let _ = std::fs::remove_file(&image);
}

/// Seeking before the start is refused rather than wrapping into the previous
/// partition, which is what an unchecked cast would do.
#[test]
fn seeking_before_the_window_is_refused() {
    let path = crate::scratch::absent("seek");
    std::fs::write(&path, vec![0u8; 2048]).expect("a file");
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("open");
    let mut view = Partition::new(&mut file, 1024, 512);
    let refused = view.seek(SeekFrom::Current(-1)).is_err();
    let _ = std::fs::remove_file(&path);
    assert!(refused, "a negative seek must not reach outside the window");
}

// ---------------------------------------------------------------------------
// SPEC-05 §4, 2026-09-09: which disk, and proving what was written
//
// **The absolute rule for this section:** every path below that actually
// opens something for writing does so against a file this test creates in
// `std::env::temp_dir()`, never against a path that could resolve to a real
// disk. `refuse_unless_removable` and `confirm_identifier` are pure — no
// path is ever opened when they are the thing under test.
// ---------------------------------------------------------------------------

fn scratch_disk(id: &str, removable: bool) -> Disk {
    Disk {
        id: id.to_owned(),
        model: "Test Fixture".to_owned(),
        size_bytes: 0,
        removable,
        letters: Vec::new(),
    }
}

/// **A device the enumeration did not report as removable is refused, with
/// no override.** `SPEC-05` §4's central rule for this step, and
/// `AGENTS.md` §10.4's reason for it: an escape hatch here is how a
/// development host's own disk gets destroyed.
#[test]
fn a_non_removable_device_is_refused_with_no_override() {
    let fixed = scratch_disk(r"\\.\PHYSICALDRIVE0", false);
    let why = refuse_unless_removable(&fixed).expect_err("a fixed disk must be refused");
    assert!(why.starts_with("NOTHING WAS WRITTEN"), "{why}");
    assert!(why.contains("NOT reported as removable"), "{why}");
    assert!(why.contains("no flag that overrides"), "{why}");
    assert!(why.contains("Unplug and replug"), "{why}");

    let removable = scratch_disk(r"\\.\PHYSICALDRIVE7", true);
    assert!(refuse_unless_removable(&removable).is_ok(), "a removable disk must pass");
}

/// **`Write it? [y/N]` defaults to no**, pure — `is_yes` is proven with no
/// stdin at all, the same posture `confirm_identifier` used to be tested
/// with before `SPEC-05` §4's 2026-09-09 ruling replaced the retyped
/// identifier with this prompt.
#[test]
fn write_it_defaults_to_no_and_only_y_or_yes_says_yes() {
    for no in ["", "\n", "n", "N", "no", "  \n", "anything else"] {
        assert!(!is_yes(no), "{no:?} must not be read as yes");
    }
    for yes in ["y", "Y", "yes", "YES", " y \n", "Yes"] {
        assert!(is_yes(yes), "{yes:?} must be read as yes");
    }
}

/// **A byte-count mismatch is an error, and says the card is NOT usable.**
/// `SPEC-05` §4's write step.
#[test]
fn a_byte_count_mismatch_is_an_error() {
    assert!(short_write_error("target", 100, 100).is_none(), "an exact match is not an error");
    let why = short_write_error("target", 40, 100).expect("a short write must be reported");
    assert!(why.contains("40 bytes of 100"), "{why}");
    assert!(why.contains("NOT usable"), "{why}");
}

/// **A digest mismatch is an error, and says the card is NOT usable.**
/// `SPEC-05` §4's verify step. The two digests are real BLAKE3 output over
/// different bytes, not placeholders, so this also proves the comparison is
/// a genuine byte-for-byte one and not, say, a length check that two
/// 32-byte arrays would always pass.
#[test]
fn a_digest_mismatch_is_an_error() {
    let mut a = Blake3::new();
    a.start();
    a.write(b"what was written to the card");
    let written = a.finish();

    let mut b = Blake3::new();
    b.start();
    b.write(b"what came back off the card");
    let read_back = b.finish();

    assert!(
        digest_mismatch_error("target", 29, &written, &written).is_none(),
        "identical digests are not an error"
    );
    let why = digest_mismatch_error("target", 29, &written, &read_back)
        .expect("different digests must be reported");
    assert!(why.contains("NOT usable"), "{why}");
    assert!(why.contains(&rivetos_tls::encode_hex(&written)), "{why}");
    assert!(why.contains(&rivetos_tls::encode_hex(&read_back)), "{why}");
}

/// **The real write-and-verify machinery, end to end, against a temporary
/// file.** This is the one test in this file that actually calls
/// [`write_bytes_and_verify`] — open, write, hash, `sync_all`, reopen, read
/// back, hash, compare — proving the mechanism works rather than only the
/// decisions around it. The "device" is a file this test creates and deletes
/// in `std::env::temp_dir()`.
#[test]
fn the_write_and_verify_machinery_round_trips_against_a_temp_file() {
    let dir = crate::scratch::dir(&format!("w-{}", line!()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let image_path = dir.join("image.bin");
    let device_path = dir.join("device.bin");

    // Multiple chunks' worth, so the CHUNK-sized read/write loop actually
    // loops rather than finishing in one call.
    let content: Vec<u8> = (0..CHUNK * 2 + 12345).map(|i| (i % 251) as u8).collect();
    std::fs::write(&image_path, &content).expect("write the image fixture");
    // **Pre-existing, and larger than the image** — standing in for a device
    // whose capacity exceeds what is being written, which is the ordinary
    // case. `write_bytes_and_verify` opens with `create(false)`, so the file
    // must already exist before the call, exactly as a device node does.
    std::fs::write(&device_path, vec![0xAAu8; content.len() + 4096]).expect("seed the device file");

    // A progress path too, since this is the one test that drives the real
    // copy-and-verify loops with more than one CHUNK to move — proving
    // `crate::elevate::append_progress` is actually reached from inside them,
    // not only that its own file format is right in isolation.
    let progress_path = dir.join("progress.txt");
    let device_id = device_path.to_string_lossy().into_owned();
    let report = write_bytes_and_verify(
        &image_path,
        &device_id,
        content.len() as u64,
        Some(&progress_path),
        None,
    )
    .expect("a same-length, pre-existing target must round-trip");
    assert!(report.contains("read every one back"), "{report}");
    assert!(report.contains("BLAKE3"), "{report}");

    let written_back = std::fs::read(&device_path).expect("read the device file back");
    assert_eq!(
        &written_back[..content.len()],
        content.as_slice(),
        "the bytes actually on the stand-in device must equal the image, exactly"
    );

    let progress = std::fs::read_to_string(&progress_path).expect("a progress file must exist");
    assert!(progress.contains("write\t"), "{progress}");
    assert!(progress.contains("verify\t"), "{progress}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// A target smaller than what the image claims is a short write, reported by
/// [`write_bytes_and_verify`] itself rather than by the pure helper alone —
/// proving the real function reaches that branch, not only that the branch's
/// wording is right.
#[test]
fn write_bytes_and_verify_reports_a_real_short_write() {
    let dir = crate::scratch::dir(&format!("short-{}", line!()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let image_path = dir.join("image.bin");
    let device_path = dir.join("device.bin");

    std::fs::write(&image_path, vec![7u8; 4096]).expect("write the image fixture");
    std::fs::write(&device_path, vec![0u8; 4096]).expect("seed the device file");

    let device_id = device_path.to_string_lossy().into_owned();
    // Claiming the image is larger than it is is exactly what a stale
    // `image_len` measured before a race would look like — the caller in
    // `write_and_verify` measures it once and passes it down, so this
    // exercises what happens if that number and the image ever disagree.
    let why = write_bytes_and_verify(&image_path, &device_id, 4096 + 1, None, None)
        .expect_err("copying fewer bytes than claimed must be reported");
    assert!(why.contains("NOT usable"), "{why}");

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// D19: the elevated write checks the image against a digest taken before the
// elevation prompt, not only the disk against what it just wrote.
// ---------------------------------------------------------------------------

/// [`digest_file`] streams rather than loads the whole file, but must still
/// answer exactly what a one-shot digest of the same bytes would.
#[test]
fn digest_file_matches_a_one_shot_digest_of_the_same_bytes() {
    let dir = crate::scratch::dir(&format!("digest-file-{}", line!()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("x.bin");
    let content: Vec<u8> = (0..CHUNK + 3).map(|i| (i % 251) as u8).collect();
    std::fs::write(&path, &content).expect("write fixture");

    let mut hasher = Blake3::new();
    let expected = rivetos_tls::encode_hex(&hasher.of(&content));
    assert_eq!(digest_file(&path).expect("a readable file digests"), expected);

    let _ = std::fs::remove_dir_all(&dir);
}

/// An unreadable path is reported by name rather than panicking or returning
/// an empty digest.
#[test]
fn digest_file_reports_a_path_it_cannot_read() {
    let dir = crate::scratch::dir(&format!("digest-file-missing-{}", line!()));
    let why = digest_file(&dir.join("nothing-here.bin")).expect_err("a missing file cannot digest");
    assert!(why.contains("cannot open"), "{why}");
}

/// **A mismatch is an error and says plainly what happened**: the disk now
/// holds bytes that are not the image named for this run, and must not be
/// used. `None` — nothing to compare, or the two already agree — is not an
/// error.
#[test]
fn image_identity_mismatch_error_compares_against_what_was_recorded() {
    let mut believed_hasher = Blake3::new();
    let believed = believed_hasher.of(b"what the parent digested before the elevation prompt");
    let believed_hex = rivetos_tls::encode_hex(&believed);

    let mut actual_hasher = Blake3::new();
    let actual = actual_hasher.of(b"what was actually streamed to the disk just now");

    assert!(
        image_identity_mismatch_error("target", None, &actual).is_none(),
        "nothing to compare against must not be an error"
    );
    assert!(
        image_identity_mismatch_error("target", Some(&believed_hex), &believed).is_none(),
        "the same bytes, digested twice, must agree"
    );
    let why = image_identity_mismatch_error("target", Some(&believed_hex), &actual)
        .expect("bytes that differ from what was recorded must be reported");
    assert!(why.contains("target"), "{why}");
    assert!(why.contains("NOT the image"), "{why}");
    assert!(why.contains("must NOT be used"), "{why}");
}

/// **The writer loop's own seam.** [`copy_and_digest`] takes anything that
/// implements [`Read`], not only a real file, specifically so a source whose
/// eventual bytes differ from what an earlier caller recorded can be proven
/// here with no real file for a same-user attacker to race, and no
/// background writer for a test to race against (`AGENTS.md` §4, §9).
///
/// The difference lands in a chunk AFTER the first, so this also proves the
/// whole stream is digested rather than only its start.
#[test]
fn a_source_that_differs_from_what_was_recorded_is_caught_by_its_digest() {
    let dir = crate::scratch::dir(&format!("swap-{}", line!()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let device_path = dir.join("device.bin");

    let shared_prefix: Vec<u8> = (0..CHUNK + 500).map(|i| (i % 251) as u8).collect();
    let mut believed = shared_prefix.clone();
    believed.extend(std::iter::repeat_n(0xAAu8, 500));
    let mut actual = shared_prefix;
    actual.extend(std::iter::repeat_n(0xBBu8, 500)); // the same-user swap

    std::fs::write(&device_path, vec![0u8; actual.len() + 4096]).expect("seed the device file");
    let mut out = OpenOptions::new()
        .write(true)
        .open(&device_path)
        .expect("open the stand-in device");

    let mut believed_hasher = Blake3::new();
    let expected_hex = rivetos_tls::encode_hex(&believed_hasher.of(&believed));

    let (copied, digest_written) =
        copy_and_digest(actual.as_slice(), &mut out, "device", actual.len() as u64, None)
            .expect("the copy writes whatever it is actually handed - it never refuses on its own");
    assert_eq!(copied, actual.len() as u64, "every byte offered must still be copied");

    let why = image_identity_mismatch_error("device", Some(&expected_hex), &digest_written)
        .expect("bytes that changed after the digest was taken must be refused");
    assert!(why.contains("NOT the image"), "{why}");
    assert!(why.contains("must NOT be used"), "{why}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// **The full path, wired together.** An unchanged image's digest, recorded
/// before elevation and handed to [`write_bytes_and_verify`] as
/// `expected_image_digest`, matches what streaming it actually writes, and
/// nothing is refused. A digest that does not match — standing in for the
/// image changing under this run, which
/// [`a_source_that_differs_from_what_was_recorded_is_caught_by_its_digest`]
/// proves at the lower seam — is refused, names the disk, and says the card
/// must not be used.
#[test]
fn write_bytes_and_verify_checks_the_image_digest_it_is_given() {
    let dir = crate::scratch::dir(&format!("image-digest-{}", line!()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let image_path = dir.join("image.bin");
    let device_path = dir.join("device.bin");

    let content: Vec<u8> = (0..CHUNK + 777).map(|i| (i % 251) as u8).collect();
    std::fs::write(&image_path, &content).expect("write the image fixture");
    std::fs::write(&device_path, vec![0xAAu8; content.len() + 4096]).expect("seed the device file");
    let device_id = device_path.to_string_lossy().into_owned();

    let correct_digest = digest_file(&image_path).expect("the fixture is readable");
    let report = write_bytes_and_verify(
        &image_path,
        &device_id,
        content.len() as u64,
        None,
        Some(&correct_digest),
    )
    .expect("the recorded digest of an untouched image must match what streaming it writes");
    assert!(report.contains("read every one back"), "{report}");

    // Re-seed: the call above already wrote the real content onto the
    // stand-in device. Prove the wrong-digest refusal on a fresh, otherwise
    // identical attempt.
    std::fs::write(&device_path, vec![0xAAu8; content.len() + 4096]).expect("reseed the device file");
    let wrong_digest = "0".repeat(64);
    let why = write_bytes_and_verify(&image_path, &device_id, content.len() as u64, None, Some(&wrong_digest))
        .expect_err("a digest that does not match what was actually streamed must be refused");
    assert!(why.contains("NOT the image"), "{why}");
    assert!(why.contains(&device_id), "{why}");
    assert!(why.contains("must NOT be used"), "{why}");

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// `--verify-only`, and the unelevated boot-partition note, 2026-09-09
// ---------------------------------------------------------------------------

/// **The whole-disk verify, standing alone, with nothing written first** —
/// the same round trip [`write_bytes_and_verify`]'s own test proves, minus
/// the write half.
#[test]
fn verify_device_against_image_matches_a_correct_device() {
    let dir = crate::scratch::dir(&format!("verify-{}", line!()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let image_path = dir.join("image.bin");
    let device_path = dir.join("device.bin");

    let content: Vec<u8> = (0..CHUNK + 4096).map(|i| (i % 251) as u8).collect();
    std::fs::write(&image_path, &content).expect("write the image fixture");
    // The device carries the SAME bytes plus trailing capacity beyond the
    // image's own length — an ordinary card is larger than the image written
    // to it, and this proves the trailing bytes are never touched.
    let mut device_bytes = content.clone();
    device_bytes.extend(vec![0xEEu8; 8192]);
    std::fs::write(&device_path, &device_bytes).expect("seed the device file");

    let device_id = device_path.to_string_lossy().into_owned();
    let report = verify_device_against_image(&image_path, &device_id, None)
        .expect("identical bytes must verify");
    assert!(report.contains("matches"), "{report}");
    assert!(report.contains("BLAKE3"), "{report}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// A device whose bytes disagree with the image is reported as **NOT
/// usable**, the same wording a failed write-then-verify uses.
#[test]
fn verify_device_against_image_reports_a_mismatch_as_not_usable() {
    let dir = crate::scratch::dir(&format!("verify-bad-{}", line!()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let image_path = dir.join("image.bin");
    let device_path = dir.join("device.bin");

    std::fs::write(&image_path, vec![7u8; 4096]).expect("write the image fixture");
    std::fs::write(&device_path, vec![9u8; 4096]).expect("a device with different bytes");

    let device_id = device_path.to_string_lossy().into_owned();
    let why = verify_device_against_image(&image_path, &device_id, None)
        .expect_err("different bytes must be reported");
    assert!(why.contains("NOT usable"), "{why}");
    assert!(why.contains("BLAKE3"), "{why}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// A device shorter than the image is a short read, reported the same way as
/// a short write.
#[test]
fn verify_device_against_image_reports_a_short_device_as_not_usable() {
    let dir = crate::scratch::dir(&format!("verify-short-{}", line!()));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let image_path = dir.join("image.bin");
    let device_path = dir.join("device.bin");

    std::fs::write(&image_path, vec![1u8; 4096]).expect("write the image fixture");
    std::fs::write(&device_path, vec![1u8; 2048]).expect("a device shorter than the image");

    let device_id = device_path.to_string_lossy().into_owned();
    let why = verify_device_against_image(&image_path, &device_id, None)
        .expect_err("a short device must be reported");
    assert!(why.contains("NOT usable"), "{why}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// [`evaluate_boot_partition`] and [`describe_boot_check`], pure — proven
/// against fabricated bytes rather than a real drive letter, since this
/// project's own safety rule (`AGENTS.md` §9) means a unit test can conjure
/// neither a real device nor a real mounted volume.
#[test]
fn evaluate_boot_partition_reports_a_byte_exact_match() {
    let expected = b"hostname = \"arbor\"\n";
    let matching = evaluate_boot_partition("E:", Some(expected), true, expected);
    assert!(matching.document_matches);
    assert!(matching.staged_present);

    let wrong_bytes = evaluate_boot_partition("F:", Some(b"something else"), false, expected);
    assert!(!wrong_bytes.document_matches);
    assert!(!wrong_bytes.staged_present);

    let missing = evaluate_boot_partition("G:", None, false, expected);
    assert!(!missing.document_matches);
}

/// **Fewer than two matching partitions is called out by name** — the
/// failure `SPEC-05` §4 exists to catch, restated in the report rather than
/// left for the reader to count.
#[test]
fn describe_boot_check_flags_fewer_than_two_matches() {
    let both = describe_boot_check(&[
        BootPartitionCheck { letter: "E:".to_owned(), document_matches: true, staged_present: true },
        BootPartitionCheck { letter: "F:".to_owned(), document_matches: true, staged_present: true },
    ]);
    assert!(both.contains("2 of 2"), "{both}");
    assert!(!both.contains("FEWER THAN TWO"), "{both}");

    let one = describe_boot_check(&[
        BootPartitionCheck { letter: "E:".to_owned(), document_matches: true, staged_present: true },
        BootPartitionCheck { letter: "F:".to_owned(), document_matches: false, staged_present: false },
    ]);
    assert!(one.contains("1 of 2"), "{one}");
    assert!(one.contains("FEWER THAN TWO"), "{one}");

    let none = describe_boot_check(&[]);
    assert!(none.contains("no boot partition could be confirmed"), "{none}");
}
