//! The update payload's round trip, driven through the built binary.
//!
//! # What this suite proves, and what it cannot
//!
//! `AGENTS.md` §6. It proves that a payload **`cargo rivet update` writes** is
//! one that:
//!
//! * `rivet_core::package::read::open` accepts under **real Ed25519 and
//!   BLAKE3** — the same verifier a board runs, not a second one written here;
//! * `rivet_core::package::update::Payload::open` accepts as an *update*, with
//!   `ADR-029` §4's reserved names taken from `PiFirmware` rather than typed;
//! * `rivet_adapter_image::install::InstallPlan` turns into the ordered set of
//!   writes `cmd/rivet-deploy` performs — **the shipped install path's
//!   decision half**, against a payload this tool produced rather than a fixture
//!   assembled by the test.
//!
//! It proves **nothing about a Rivet**, and the gap is larger than usual here.
//! `InstallPlan` is the *plan*; the I/O half is `cmd/rivet-deploy`'s and needs
//! block devices, mounts and root. `ADR-029` §14.6 is the account of that binary
//! running against a loop device, and §13.5 and §14.6 both say what a loop
//! device cannot be: an SD card. Nothing in this file writes a byte to any
//! device.
//!
//! **Provenance:** every fixture here is hand-composed. The file *names* are the
//! ones `cmd/rivet-image`'s `boot::contents` writes and the `cmdline.txt`
//! contents are `ADR-029` §3's measured difference; the bytes are not a card's.
//! `AGENTS.md` §4 — a vector captured from real hardware is worth more, and
//! `rivet-crypto/tests/loopback.rs` is where one is used.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rivet_adapter_image::{BootFileAction, BootSlots, InstallPlan, PiFirmware, SlotId, action_for};
use rivet_core::package::format::HEADER_BYTES;
use rivet_core::package::kind::Kind;
use rivet_core::package::port::{MaySign, PublicKey, TrustSet};
use rivet_core::package::read::{Package, open};
use rivet_core::package::update::{Payload, SlotVariance, TargetClass};
use rivet_core::state::Slot;
use rivet_crypto::{Blake3, Ed25519Verifier};

const EXE: &str = env!("CARGO_BIN_EXE_cargo-rivet");

/// The one `/system` member, and it is deliberately not a round megabyte: a
/// length that is not a multiple of the 1 MiB streaming chunk is the case a
/// short final read gets wrong.
const SYSTEM_BYTES: usize = 3 * 1024 * 1024 + 1777;

fn scratch(what: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    scratch::dir(&format!("update-{what}-{stamp}"))
}

/// The crate's own scratch root, compiled into this test binary as well — the
/// same `#[path]` idiom `main.rs` uses for the SDK's QOI encoder. An
/// integration test is its own crate and cannot reach `crate::scratch`, and
/// without this it leaks every fixture directory into `%TEMP%` for ever.
#[path = "../src/scratch.rs"]
#[allow(dead_code)]
mod scratch;

/// 32 bytes that are not in any tracked file and are different every run —
/// `AGENTS.md` §8.3, the same shape `tests/roundtrip.rs` established.
fn mint_seed(at: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(1);
    let mut seed = [0u8; 32];
    let mut state = stamp ^ (u128::from(std::process::id()) << 64) ^ 0x9e37_79b9_7f4a_7c15;
    for byte in &mut seed {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        *byte = (state >> 33) as u8;
    }
    assert!(seed.iter().any(|b| *b != 0));
    let path = at.join("system.seed");
    std::fs::write(&path, seed).unwrap();
    path
}

fn run(args: &[&str]) -> Output {
    Command::new(EXE)
        .args(args)
        .output()
        .expect("cargo-rivet did not run")
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

struct Card {
    dir: PathBuf,
    system: PathBuf,
    boot_a: PathBuf,
    boot_b: PathBuf,
    seed: PathBuf,
    public: PathBuf,
}

/// A card's two boot partitions and one `/system` filesystem, as directories
/// and a file.
///
/// The file set is `cmd/rivet-image`'s `boot::contents` — the firmware blobs,
/// the kernel, the two `.txt` settings files, `cmdline.txt`, and the two names
/// `ADR-029` §4 reserves. `autoboot.txt` is written **differently on the two
/// sides** on purpose: on a real card it is, and it is therefore the case that
/// tells a reserved-name filter running before the diff apart from one running
/// after it.
fn card(what: &str) -> Card {
    let dir = scratch(what);
    let boot_a = dir.join("boot-a");
    let boot_b = dir.join("boot-b");
    std::fs::create_dir_all(&boot_a).unwrap();
    std::fs::create_dir_all(&boot_b).unwrap();

    let kernel: Vec<u8> = (0u32..1_048_576).map(|n| (n % 251) as u8).collect();
    let firmware: Vec<u8> = (0u32..40_000).map(|n| (n % 233) as u8).collect();
    for at in [&boot_a, &boot_b] {
        std::fs::write(at.join("kernel8.img"), &kernel).unwrap();
        std::fs::write(at.join("start4.elf"), &firmware).unwrap();
        std::fs::write(at.join("bcm2711-rpi-4-b.dtb"), &firmware).unwrap();
        std::fs::write(at.join("config.txt"), b"arm_64bit=1\ntryboot_a_b=1\n").unwrap();
        std::fs::write(at.join("tryboot.txt"), b"[all]\n").unwrap();
        std::fs::write(at.join("rivet.toml"), b"version = 1\n").unwrap();
    }
    // ADR-029 §3's measured difference, and the only one.
    std::fs::write(
        boot_a.join("cmdline.txt"),
        b"root=PARTLABEL=system-a rootwait\n",
    )
    .unwrap();
    std::fs::write(
        boot_b.join("cmdline.txt"),
        b"root=PARTLABEL=system-b rootwait\n",
    )
    .unwrap();
    // Reserved, and different on the two sides — as on a real card.
    std::fs::write(boot_a.join("autoboot.txt"), b"[all]\nboot_partition=1\n").unwrap();
    std::fs::write(boot_b.join("autoboot.txt"), b"[all]\nboot_partition=2\n").unwrap();

    let system: Vec<u8> = (0..SYSTEM_BYTES).map(|n| (n % 241) as u8).collect();
    let system_path = dir.join("system.img");
    std::fs::write(&system_path, &system).unwrap();

    let seed = mint_seed(&dir);
    let public = dir.join("system.pub");
    Card {
        dir,
        system: system_path,
        boot_a,
        boot_b,
        seed,
        public,
    }
}

fn build(card: &Card, version: &str) -> Output {
    run(&[
        "rivet",
        "update",
        "--system",
        card.system.to_str().unwrap(),
        "--boot-a",
        card.boot_a.to_str().unwrap(),
        "--boot-b",
        card.boot_b.to_str().unwrap(),
        "--signing-key",
        card.seed.to_str().unwrap(),
        "--version",
        version,
        "--out-dir",
        card.dir.to_str().unwrap(),
        "--public-key-out",
        card.public.to_str().unwrap(),
    ])
}

fn built_payload(card: &Card) -> PathBuf {
    std::fs::read_dir(&card.dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "rvt"))
        .expect("no .rvt was written")
}

/// `ADR-029` §5 steps 1–7 over the file, through the real verifier.
fn open_payload(path: &Path, public: &Path) -> Package {
    let len = std::fs::metadata(path).expect("stat").len();
    let mut head = [0u8; HEADER_BYTES];
    use std::io::Read as _;
    std::fs::File::open(path)
        .unwrap()
        .read_exact(&mut head)
        .expect("the header");

    let raw: [u8; 32] = std::fs::read(public).unwrap().try_into().unwrap();
    let mut digest = Blake3::new();
    let mut trust = TrustSet::new();
    trust.trust(
        &mut digest,
        PublicKey(raw),
        "test-root",
        MaySign::these(&[Kind::update()]),
    );
    open(&head, len, &mut digest, &Ed25519Verifier::new(), &trust).expect("a verified payload")
}

/// Read one member out of the file and hand back its bytes **only if the digest
/// matched** — `ADR-029` §5 step 8, through `Package::member_stream`.
fn verified_bytes(package: &Package, path: &Path, index: usize) -> Vec<u8> {
    use std::io::{Read as _, Seek as _, SeekFrom};
    let member = &package.members()[index];
    let mut file = std::fs::File::open(path).unwrap();
    file.seek(SeekFrom::Start(member.offset())).unwrap();
    let mut digest = Blake3::new();
    let mut stream = package.member_stream(index, &mut digest).unwrap();
    let mut kept = Vec::new();
    let mut left = member.stored_len();
    let mut buffer = vec![0u8; 64 * 1024];
    while left > 0 {
        let want = buffer.len().min(left as usize);
        file.read_exact(&mut buffer[..want]).unwrap();
        stream.write(&buffer[..want]).unwrap();
        kept.extend_from_slice(&buffer[..want]);
        left -= want as u64;
    }
    stream.finish().expect("the member's digest matched");
    kept
}

/// `ADR-029` §4's reserved names, from the port rather than from this file.
fn reserved() -> &'static [&'static str] {
    let target = PiFirmware
        .payload(SlotId(1))
        .expect("slot A is in ADR-018 §4's table");
    PiFirmware.reserved_names(&target.kernel)
}

// ---------------------------------------------------------------------------

#[test]
fn a_built_payload_verifies_and_is_a_complete_update_for_both_slots() {
    let card = card("roundtrip");
    let out = build(&card, "0.0.1-gdeadbee");
    assert!(out.status.success(), "{}", text(&out));
    let report = text(&out);

    // ADR-027 §3's grammar for an update, which DOES carry <platform>. The
    // presence of `rpi4` is the ruling, so it is asserted rather than assumed.
    let payload = built_payload(&card);
    let name = payload.file_name().unwrap().to_string_lossy().to_string();
    assert_eq!(name, "rivet-0.0.1-gdeadbee-aarch64-rpi4-update.rvt", "{name}");

    // ADR-029 §4: skipping is not allowed to be silent.
    assert!(report.contains("autoboot.txt was NOT packaged"), "{report}");
    assert!(report.contains("rivet.toml was NOT packaged"), "{report}");
    // The measurement, re-taken rather than quoted — and it agreed, so the
    // staleness warning must NOT be there.
    assert!(report.contains("cmdline.txt"), "{report}");
    assert!(!report.contains("stale"), "{report}");

    // --- the same verifier a board runs -----------------------------------
    let package = open_payload(&payload, &card.public);
    let header = package.header();
    assert_eq!(header.product(), "rivet");
    assert_eq!(header.kind().as_str(), "update");
    assert_eq!(header.arch(), "aarch64");
    assert_eq!(header.platform(), Some(PiFirmware.platform_token()));

    let update = Payload::open(&package, reserved()).expect("a complete update payload");
    update
        .accepts("rivet", "aarch64", PiFirmware.platform_token())
        .expect("this machine's own product, arch and platform");
    // ADR-029 F6, the refusal that writes nothing.
    assert!(update.accepts("rivet", "x86_64", "rpi4").is_err());
    assert!(update.accepts("rivet", "aarch64", "uefi").is_err());
    assert!(update.accepts("something-else", "aarch64", "rpi4").is_err());

    // --- the system member is the bytes that went in ----------------------
    let system = update.system_member();
    assert_eq!(system.name(), "system.img");
    assert_eq!(system.stored_len(), SYSTEM_BYTES as u64);
    assert_eq!(system.target_class(), TargetClass::WholePartition.as_byte());
    assert_eq!(system.variant(), SlotVariance::Invariant.as_byte());
    assert_eq!(
        verified_bytes(&package, &payload, system.index()),
        std::fs::read(&card.system).unwrap()
    );

    // --- the boot half, per slot ------------------------------------------
    let names = |slot: Slot| -> Vec<String> {
        update
            .boot_members_for(slot)
            .map(|m| m.name().to_string())
            .collect()
    };
    let expected = vec![
        "bcm2711-rpi-4-b.dtb".to_string(),
        "cmdline.txt".to_string(),
        "config.txt".to_string(),
        "kernel8.img".to_string(),
        "start4.elf".to_string(),
        "tryboot.txt".to_string(),
    ];
    assert_eq!(names(Slot::A), expected);
    assert_eq!(names(Slot::B), expected);

    // The one file that differs carries the right side into the right slot.
    for (slot, want) in [(Slot::A, &b"system-a"[..]), (Slot::B, &b"system-b"[..])] {
        let member = update
            .boot_members_for(slot)
            .find(|m| m.name() == "cmdline.txt")
            .expect("a cmdline for each slot");
        let bytes = verified_bytes(&package, &payload, member.index());
        assert!(
            bytes.windows(want.len()).any(|w| w == want),
            "{slot:?} got {:?}",
            String::from_utf8_lossy(&bytes)
        );
    }

    // Nothing reserved got in, checked over the member table rather than over
    // the report — ADR-029 §4's refusal is about the bytes.
    for member in package.members() {
        assert!(
            !reserved().contains(&member.name()),
            "{} is reserved",
            member.name()
        );
    }
    assert!(card.dir.exists());
}

/// The verb this exists to feed: `rivet_adapter_image::install` is the
/// decision half of `cmd/rivet-deploy`, and it is driven here over a payload
/// `cargo rivet update` produced rather than over a fixture.
#[test]
fn the_installers_own_plan_accepts_the_payload_and_orders_it_the_safe_way() {
    let card = card("plan");
    let out = build(&card, "0.0.1");
    assert!(out.status.success(), "{}", text(&out));
    let payload = built_payload(&card);
    let package = open_payload(&payload, &card.public);
    let update = Payload::open(&package, reserved()).expect("a complete update payload");

    // `ADR-018` §4's slots: SlotId is the BOOT partition number, 1 for A and 2
    // for B, and no code above the port does arithmetic on it.
    for (slot_id, slot) in [(SlotId(1), Slot::A), (SlotId(2), Slot::B)] {
        let target = PiFirmware.payload(slot_id).expect("a slot in the table");
        let plan = InstallPlan::for_slot(&update, slot_id, &target, 640 * 1024 * 1024)
            .expect("a plan for this slot");

        // The system member is a separate field from the boot half, which is
        // ADR-029 §9's second bound on F5 made structural.
        assert_eq!(plan.system().bytes, SYSTEM_BYTES as u64);
        assert_eq!(plan.system().member_index, update.system_member().index());
        assert_eq!(plan.member_indices().first(), Some(&plan.system().member_index));
        assert_eq!(plan.boot().files.len(), 6);
        assert_eq!(
            plan.boot()
                .files
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>(),
            update
                .boot_members_for(slot)
                .map(|m| m.name())
                .collect::<Vec<_>>()
        );

        // ADR-029 §9.3's F5, reported at the moment it applies. Slot A is the
        // partition the firmware falls back to; slot B is not.
        assert_eq!(
            plan.targets_the_fallback_partition(),
            slot == Slot::A,
            "{slot:?}"
        );

        // ADR-029 §6's skip, which §9 makes a SAFETY property: a boot file whose
        // digest already matches the target is not written at all.
        let kernel = plan
            .boot()
            .files
            .iter()
            .find(|f| f.name == "kernel8.img")
            .expect("a kernel");
        assert_eq!(
            action_for(&kernel.digest, Some(&kernel.digest)),
            BootFileAction::Skip
        );
        assert_eq!(action_for(&kernel.digest, None), BootFileAction::Write);
    }

    // ADR-029 F8, refused before anything is written.
    let target = PiFirmware.payload(SlotId(2)).unwrap();
    let refused = InstallPlan::for_slot(&update, SlotId(2), &target, 1024)
        .expect_err("a 3 MiB member does not fit a 1 KiB partition");
    assert!(format!("{refused}").contains("1024"), "{refused}");
}

/// `cargo rivet inspect` used to call `Application::open` unconditionally, so
/// a valid update payload was refused by the *reporting* layer after the
/// verifier had accepted it. This is that defect's regression test.
#[test]
fn inspect_reads_an_update_payload_back_and_says_which_files_go_to_which_slot() {
    let card = card("inspect");
    let out = build(&card, "0.0.1");
    assert!(out.status.success(), "{}", text(&out));
    let payload = built_payload(&card);

    let out = run(&[
        "rivet",
        "inspect",
        payload.to_str().unwrap(),
        "--trusted-key",
        card.public.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", text(&out));
    let report = text(&out);
    assert!(report.contains("kind     update"), "{report}");
    assert!(report.contains("platform rpi4"), "{report}");
    assert!(report.contains("installed into slot A"), "{report}");
    assert!(report.contains("installed into slot B"), "{report}");
    assert!(report.contains("SYSTEM UPDATE"), "{report}");
    // The per-member slot column, which is the thing a person checks.
    assert!(report.contains("slot A"), "{report}");
    assert!(report.contains("slot B"), "{report}");

    // An application's flag on an update is an error rather than a silent
    // no-op: ADR-029 §11 is one container and three interpretations.
    let out = run(&[
        "rivet",
        "inspect",
        payload.to_str().unwrap(),
        "--trusted-key",
        card.public.to_str().unwrap(),
        "--extract-program",
        card.dir.join("nope").to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert!(text(&out).contains("no program member"), "{}", text(&out));
}

/// Each corruption is checked against a baseline that succeeds, in the same
/// test, so none of them can pass vacuously.
#[test]
fn every_corruption_of_a_payload_is_refused_and_the_untouched_one_is_not() {
    let card = card("corruption");
    let out = build(&card, "0.0.1");
    assert!(out.status.success(), "{}", text(&out));
    let payload = built_payload(&card);
    let good = std::fs::read(&payload).unwrap();

    let verify = |bytes: &[u8], label: &str| -> Output {
        let path = card.dir.join(format!("{label}.rvt"));
        std::fs::write(&path, bytes).unwrap();
        run(&[
            "rivet",
            "inspect",
            path.to_str().unwrap(),
            "--trusted-key",
            card.public.to_str().unwrap(),
        ])
    };

    let baseline = verify(&good, "baseline");
    assert!(baseline.status.success(), "{}", text(&baseline));

    // One bit inside the 3 MiB system member. Past the signed header, so only
    // the member digest can catch it — ADR-029 §5 step 8.
    let mut flipped = good.clone();
    flipped[HEADER_BYTES + 1_000_000] ^= 0x01;
    let out = verify(&flipped, "system-bit");
    assert!(!out.status.success());
    assert!(text(&out).contains("digest"), "{}", text(&out));

    // One bit inside the signed header — the platform field, which is exactly
    // the thing ADR-029 F6 refuses on and therefore the thing worth forging.
    let mut forged = good.clone();
    forged[144] ^= 0x20;
    let out = verify(&forged, "header-bit");
    assert!(!out.status.success());
    assert!(text(&out).contains("signature"), "{}", text(&out));

    // A truncated transfer — ADR-029 F1.
    let out = verify(&good[..good.len() - 1], "truncated");
    assert!(!out.status.success());
    assert!(text(&out).contains("truncated"), "{}", text(&out));

    let baseline = verify(&good, "baseline-again");
    assert!(baseline.status.success(), "{}", text(&baseline));
}

/// A file on one boot partition and not the other. The board refuses it too;
/// refusing here names the file while it is still on this host's disk.
#[test]
fn a_boot_file_on_one_slot_only_is_refused_before_anything_is_written() {
    let card = card("one-sided");
    std::fs::write(card.boot_a.join("extra.dtb"), b"only on a").unwrap();
    let out = build(&card, "0.0.1");
    assert!(!out.status.success());
    let report = text(&out);
    assert!(report.contains("extra.dtb"), "{report}");
    assert!(report.contains("slot B"), "{report}");
    assert!(built_payload_absent(&card), "a payload was written anyway");
}

/// A file whose bytes agree with `kernel8.img`'s but whose name differs only
/// in ASCII case becomes a second `Invariant` member under a distinct exact
/// name — this container's own duplicate check does not see it, but the
/// target's boot filesystem does, so the build refuses it the same way
/// `Payload::open` would on a board.
///
/// **Linux only.** The fixture needs two directory entries that differ only
/// in case, and the development host's filesystem is case-insensitive: a
/// second write under the other spelling lands on the first entry rather
/// than creating one, which is a fact about NTFS and default APFS, not about this refusal.
#[cfg(target_os = "linux")]
#[test]
fn a_boot_file_whose_name_collides_with_another_only_once_case_is_folded_is_refused() {
    let card = card("case-collision");
    let kernel = std::fs::read(card.boot_a.join("kernel8.img")).unwrap();
    for at in [&card.boot_a, &card.boot_b] {
        std::fs::write(at.join("KERNEL8.IMG"), &kernel).unwrap();
    }
    let out = build(&card, "0.0.1");
    assert!(!out.status.success());
    let report = text(&out);
    assert!(report.contains("kernel8.img"), "{report}");
    assert!(report.contains("KERNEL8.IMG"), "{report}");
    assert!(built_payload_absent(&card), "a payload was written anyway");
}

/// A member named another member's install-time staging name: the second
/// member's write would land on the first member's already-installed file.
#[test]
fn a_boot_file_named_another_files_staging_name_is_refused() {
    let card = card("temp-collision");
    let kernel = std::fs::read(card.boot_a.join("kernel8.img")).unwrap();
    for at in [&card.boot_a, &card.boot_b] {
        std::fs::write(at.join("kernel8.img.new"), &kernel).unwrap();
    }
    let out = build(&card, "0.0.1");
    assert!(!out.status.success());
    let report = text(&out);
    assert!(report.contains("kernel8.img"), "{report}");
    assert!(built_payload_absent(&card), "a payload was written anyway");
}

/// `RIVET.TOML` survives `boot_half`'s own case-sensitive reserved-name
/// filter — the file on disk is spelled differently from the lowercase name
/// that filter compares against — so this proves the backstop: the generic
/// writer refuses a reserved name under any ASCII case at the point the
/// member is added, the same as it already refused an exact-case one.
///
/// **Linux only** — same reason as the collision test above: the fixture
/// needs `rivet.toml` and `RIVET.TOML` as two distinct directory entries,
/// which a case-insensitive filesystem will not hold.
#[cfg(target_os = "linux")]
#[test]
fn a_boot_file_whose_name_is_reserved_only_once_case_is_folded_is_refused() {
    let card = card("reserved-case");
    for at in [&card.boot_a, &card.boot_b] {
        std::fs::write(at.join("RIVET.TOML"), b"version = 1\n").unwrap();
    }
    let out = build(&card, "0.0.1");
    assert!(!out.status.success());
    let report = text(&out);
    assert!(report.contains("RIVET.TOML"), "{report}");
    assert!(built_payload_absent(&card), "a payload was written anyway");
}

fn built_payload_absent(card: &Card) -> bool {
    std::fs::read_dir(&card.dir)
        .unwrap()
        .filter_map(Result::ok)
        .all(|e| e.path().extension().is_none_or(|x| x != "rvt"))
}

/// Three flags with no defensible default. Each refusal names the decision it
/// is declining to make (`AGENTS.md` §1).
#[test]
fn the_flags_with_no_defensible_default_are_refused_rather_than_guessed() {
    let card = card("required");
    let base = [
        "rivet",
        "update",
        "--system",
        card.system.to_str().unwrap(),
        "--boot-a",
        card.boot_a.to_str().unwrap(),
        "--boot-b",
        card.boot_b.to_str().unwrap(),
    ];

    let mut without_key: Vec<&str> = base.to_vec();
    without_key.extend(["--version", "0.0.1", "--out-dir", card.dir.to_str().unwrap()]);
    let out = run(&without_key);
    assert!(!out.status.success());
    assert!(text(&out).contains("--signing-key"), "{}", text(&out));

    let mut without_version: Vec<&str> = base.to_vec();
    without_version.extend([
        "--signing-key",
        card.seed.to_str().unwrap(),
        "--out-dir",
        card.dir.to_str().unwrap(),
    ]);
    let out = run(&without_version);
    assert!(!out.status.success());
    let report = text(&out);
    assert!(report.contains("--version"), "{report}");
    // The reason, not merely the flag: this tool did not build the image.
    assert!(report.contains("did not build"), "{report}");
}

/// `<platform>` is the boot contract (`ADR-027` §3.1) and the reserved names
/// come from the same implementor, so a token with no implementor cannot be
/// packaged for — and the refusal says which one exists.
#[test]
fn a_platform_with_no_implementor_is_refused_and_names_the_one_that_exists() {
    let card = card("platform");
    let out = run(&[
        "rivet",
        "update",
        "--system",
        card.system.to_str().unwrap(),
        "--boot-a",
        card.boot_a.to_str().unwrap(),
        "--boot-b",
        card.boot_b.to_str().unwrap(),
        "--signing-key",
        card.seed.to_str().unwrap(),
        "--version",
        "0.0.1",
        "--out-dir",
        card.dir.to_str().unwrap(),
        "--platform",
        "uefi",
    ]);
    assert!(!out.status.success());
    let report = text(&out);
    assert!(report.contains("uefi"), "{report}");
    assert!(report.contains("rpi4"), "{report}");
    assert!(built_payload_absent(&card));
}

/// The wrong trust set. A key that signed nothing must not verify a payload,
/// and the refusal has to name the trust set rather than the signature — the
/// two are different failures and `ADR-028` §5.2 is about the first.
#[test]
fn a_payload_signed_by_another_key_is_refused_by_the_trust_set() {
    let card = card("trust");
    let out = build(&card, "0.0.1");
    assert!(out.status.success(), "{}", text(&out));
    let payload = built_payload(&card);

    let other = card.dir.join("other.pub");
    std::fs::write(&other, [0x11u8; 32]).unwrap();
    let out = run(&[
        "rivet",
        "inspect",
        payload.to_str().unwrap(),
        "--trusted-key",
        other.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert!(text(&out).contains("trust set"), "{}", text(&out));
}

/// The staleness warning, which is the half of `ADR-029` §3's measurement that
/// only fires when the record is wrong. Two files differ here, so the ADR's
/// *"`cmdline.txt` is the only file that differs"* is no longer true of this
/// input — and the tool must say so rather than quietly shipping a correct
/// payload against a stale document.
#[test]
fn a_second_slot_variant_file_produces_a_correct_payload_and_a_warning() {
    let card = card("second-variant");
    std::fs::write(card.boot_a.join("config.txt"), b"arm_64bit=1\n# a\n").unwrap();
    std::fs::write(card.boot_b.join("config.txt"), b"arm_64bit=1\n# b\n").unwrap();

    let out = build(&card, "0.0.1");
    assert!(out.status.success(), "{}", text(&out));
    let report = text(&out);
    assert!(report.contains("stale"), "{report}");

    // And the payload is right rather than merely warned about: config.txt is
    // carried twice, one for each slot.
    let payload = built_payload(&card);
    let package = open_payload(&payload, &card.public);
    let update = Payload::open(&package, reserved()).expect("still a complete payload");
    let a = update
        .boot_members_for(Slot::A)
        .find(|m| m.name() == "config.txt")
        .expect("slot A's config.txt");
    let b = update
        .boot_members_for(Slot::B)
        .find(|m| m.name() == "config.txt")
        .expect("slot B's config.txt");
    assert_ne!(a.index(), b.index());
    assert_eq!(a.variant(), SlotVariance::SlotA.as_byte());
    assert_eq!(b.variant(), SlotVariance::SlotB.as_byte());
}
