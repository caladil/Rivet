//! `ADR-018` §5.2 step 1 — writing the inactive slot — against a **loop device**,
//! which is as close as this bench gets.
//!
//! # What this is a substitute for, said before anything else
//!
//! `ADR-026` §4 puts `/dev/mmcblk0` in the column QEMU cannot reach. A loopback
//! file over one of `out/`'s 2 GiB card images has the **real partition table**
//! `cmd/rivet-image` wrote — five partitions, `boot-a`/`boot-b` at 256 MiB and
//! `system-a`/`system-b` at 640 MiB — and presents each partition as a block
//! device with the same `write`/`fsync`/`read` surface a card does.
//!
//! **It is not a card, and a green run here says nothing about one.** What it
//! cannot exercise: the SD controller, a card that accepts a write and stores
//! something else (`ADR-029` §9.2 step 9's whole reason for existing), a card
//! that has gone read-only mid-write, erase-block behaviour, and every timing
//! property that follows from those. `AGENTS.md` §6 — this proves the code path
//! *functions against a block device*, not that an update works on hardware.
//!
//! # It is `#[ignore]`d, and it refuses anything that is not a loop device
//!
//! `AGENTS.md` §10.4 names writing to a block device as dangerous here, and §9
//! says to name the target explicitly rather than rely on an ambient check. Both
//! are honoured structurally: the destination arrives in an environment variable
//! (naming it), **and** [`refuse_unless_loopback`] refuses any path that is not
//! under `/dev/loop` (so a typo cannot reach the development host's own disk).
//! Nothing here writes to a card and nothing here can.
//!
//! # Running it
//!
//! Under WSL (`AGENTS.md` §10.5 — `losetup` is Linux's), as root, against a
//! **copy** of a card image. Every path is an argument; `AGENTS.md` §8.3 forbids
//! one in a tracked file, so `$CARD`, `$WORK` and `$REPO` are yours to set.
//!
//! ```sh
//! cp "$CARD" "$WORK/card.img"          # out/ is read, never written
//! LOOP=$(losetup -fP --show "$WORK/card.img")
//! mkdir -p "$WORK/boot-a" "$WORK/boot-b"
//! mount -o ro "${LOOP}p1" "$WORK/boot-a"
//! mount -o ro "${LOOP}p2" "$WORK/boot-b"
//!
//! # Built as your user, run as root: the test opens a block device.
//! #   cargo test -p rivet-crypto --test loopback --no-run
//! RIVET_FPK_WORK="$WORK" \
//! RIVET_FPK_SYSTEM_SRC="${LOOP}p3" \
//! RIVET_FPK_SYSTEM_DST="${LOOP}p4" \
//! RIVET_FPK_BOOT_A="$WORK/boot-a" \
//! RIVET_FPK_BOOT_B="$WORK/boot-b" \
//!   "$TEST_BINARY" --ignored --nocapture --test-threads=1
//!
//! umount "$WORK/boot-a" "$WORK/boot-b"; losetup -d "$LOOP"
//! ```
//!
//! `--test-threads=1` because all three tests share one loop device, and two of
//! them write to it.
//!
//! # What a run looked like — 2026-08-24, development host under WSL2
//!
//! Against `out/`'s newest card image, a **debug** build:
//!
//! | Step | 640 MiB `system-a` |
//! | :-- | --: |
//! | digest the source partition | ~24 s |
//! | verify every member of the payload | ~25 s |
//! | write the member to `/dev/loopNp4` and `fsync` | **~0.5 s** |
//! | read the slot back and hash it (`ADR-029` §9.2 step 9) | ~24 s |
//!
//! **These are not `ADR-029` §7.2's measurement and must not be read as one.**
//! Three reasons, all of which make the number meaningless for the board: it is
//! an unoptimised build, it is an x86-64 host with AVX2 where the A72 has no SIMD
//! backend at all, and a loop device over `ext4` is not an SD card. What the
//! shape does say is that **hashing dominates and the write does not** — the
//! read-back §9.2 step 9 recommends costs about as much as the verification
//! already does, which is the trade that measurement has to settle.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use rivet_adapter_image::{BootSlots, PiFirmware, SlotId};
use rivet_core::package::update::PRODUCT;
use rivet_core::package::error::PackageError;
use rivet_core::package::format::HEADER_BYTES;
use rivet_core::package::kind::Kind;
use rivet_core::package::port::MaySign;
use rivet_core::package::port::{Digest, Sign, TrustSet};
use rivet_core::package::read::{Package, VerifiedMember, open};
use rivet_core::package::update::{Payload, SlotVariance, TargetClass};
use rivet_core::package::write::{Builder, NewMember};
use rivet_core::state::Slot;
use rivet_crypto::{Blake3, Ed25519Signer, Ed25519Verifier, SEED_BYTES};

/// One MiB, the read size `cmd/rivet`'s `measure hash` already uses and for the
/// same reason: large enough that `read(2)`'s per-call cost is not what is being
/// measured, small enough to stay in cache.
const CHUNK: usize = 1024 * 1024;

/// The boot-half files a card carries, in the order `ADR-029` §6's table lists
/// them. Read from the mounted partition; any that is absent is skipped, because
/// what a card holds is `cmd/rivet-image`'s to decide and this test must not
/// hard-code a list that goes stale (`AGENTS.md` §7).
const BOOT_INVARIANT: &[&str] = &[
    "kernel8.img",
    "start4.elf",
    "fixup4.dat",
    "bcm2711-rpi-4-b.dtb",
    "config.txt",
    "tryboot.txt",
];

/// `cmdline.txt` is the one file that differs between slots — `ADR-029` §3,
/// measured rather than assumed.
const BOOT_VARIANT: &str = "cmdline.txt";

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// `AGENTS.md` §10.4 and §9, as code rather than as a warning in a comment.
///
/// Naming the device in an environment variable is the guard; this is the second
/// one, and it is the structural half — a mistyped variable cannot reach the
/// development host's own disk, because nothing outside `/dev/loop` is accepted.
fn refuse_unless_loopback(path: &str) {
    assert!(
        path.starts_with("/dev/loop"),
        "this test writes to a block device and will only ever write to a loopback one. \
         {path:?} is not under /dev/loop — refusing. AGENTS.md §10.4."
    );
}

/// The signing key. Minted here, in memory, never written down — `AGENTS.md`
/// §8.3.
fn signer() -> Ed25519Signer {
    let mut digest = Blake3::new();
    let seed: [u8; SEED_BYTES] = digest.of(b"rivet loopback bench seed, not a key");
    Ed25519Signer::from_seed(&seed)
}

/// Digest and length of a file or block device, without holding it.
fn measure(path: &Path) -> ([u8; 32], u64) {
    let mut file = File::open(path).unwrap_or_else(|e| panic!("cannot read {path:?}: {e}"));
    let mut digest = Blake3::new();
    digest.start();
    let mut buffer = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        let n = file.read(&mut buffer).expect("read");
        if n == 0 {
            break;
        }
        digest.write(&buffer[..n]);
        total += n as u64;
    }
    (digest.finish(), total)
}

/// The names `PiFirmware` reserves on the boot half of the slot an update would
/// write. `ADR-029` §4 — the port's answer, not this test's.
fn reserved() -> &'static [&'static str] {
    let payload = PiFirmware
        .payload(SlotId(1))
        .expect("slot A is in ADR-018 §4's table");
    PiFirmware.reserved_names(&payload.kernel)
}

/// Build a real `.rvt` on disk, streaming the system member rather than holding
/// it. Returns the path and the total length.
fn build_payload(work: &Path, system_src: &Path, boot_a: &Path, boot_b: &Path) -> (PathBuf, u64) {
    let mut digest = Blake3::new();
    let mut builder = Builder::new(PRODUCT, "0.0.1", "aarch64", Some("rpi4"), Kind::update())
        .expect("the header fields fit")
        .reserving(reserved());

    // The system member, measured by streaming. 640 MiB never becomes resident.
    let started = Instant::now();
    let (system_digest, system_len) = measure(system_src);
    println!(
        "  system member: {system_len} bytes digested in {:?}",
        started.elapsed()
    );
    builder
        .add(
            NewMember::streamed(
                "system.img",
                TargetClass::WholePartition.as_byte(),
                SlotVariance::Invariant.as_byte(),
                0,
                system_digest,
                system_len,
                system_len,
            )
            .expect("a well-formed member"),
        )
        .expect("the first member");

    // The boot half. Invariant files come from slot A's partition — they are
    // byte-identical on both, which is the measurement ADR-029 §3 rests on and
    // which this test re-checks below.
    let mut boot_sources: Vec<(String, u8, PathBuf)> = Vec::new();
    for name in BOOT_INVARIANT {
        let path = boot_a.join(name);
        if path.is_file() {
            boot_sources.push(((*name).to_string(), SlotVariance::Invariant.as_byte(), path));
        }
    }
    for (dir, variance) in [(boot_a, SlotVariance::SlotA), (boot_b, SlotVariance::SlotB)] {
        let path = dir.join(BOOT_VARIANT);
        assert!(
            path.is_file(),
            "{path:?} must exist — ADR-029 §3's slot-variant member"
        );
        boot_sources.push((BOOT_VARIANT.to_string(), variance.as_byte(), path));
    }

    let mut boot_bytes: Vec<Vec<u8>> = Vec::new();
    for (name, variance, path) in &boot_sources {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("cannot read {path:?}: {e}"));
        builder
            .add(
                NewMember::stored(
                    name,
                    TargetClass::BootDirectory.as_byte(),
                    *variance,
                    &bytes,
                    &mut digest,
                )
                .unwrap_or_else(|e| panic!("{name} is not a usable member: {e}")),
            )
            .unwrap_or_else(|e| panic!("{name} was refused: {e}"));
        boot_bytes.push(bytes);
    }

    let total = builder.total_len();
    let mut signer = signer();
    let head = builder
        .seal(&mut digest, &mut signer)
        .expect("a sealed header");

    let path = work.join("rivet-0.0.1-aarch64-rpi4-update.rvt");
    let mut out = File::create(&path).unwrap_or_else(|e| panic!("cannot create {path:?}: {e}"));
    out.write_all(&head).expect("the header");

    // The system member, streamed straight from the source into the payload.
    let mut source = File::open(system_src).expect("the system source");
    let mut buffer = vec![0u8; CHUNK];
    let mut written = 0u64;
    while written < system_len {
        let want = CHUNK.min((system_len - written) as usize);
        source.read_exact(&mut buffer[..want]).expect("read");
        out.write_all(&buffer[..want]).expect("write");
        written += want as u64;
    }
    for bytes in &boot_bytes {
        out.write_all(bytes).expect("write");
    }
    out.sync_all().expect("fsync");

    let on_disk = std::fs::metadata(&path).expect("stat").len();
    assert_eq!(
        on_disk, total,
        "the writer's own length and the file disagree"
    );
    (path, total)
}

/// `ADR-029` §5 steps 1–7 over a real file.
fn open_payload(path: &Path) -> Result<Package, PackageError> {
    let len = std::fs::metadata(path).expect("stat").len();
    let mut file = File::open(path).expect("open");
    let mut head = [0u8; HEADER_BYTES];
    file.read_exact(&mut head).expect("the header");

    let mut digest = Blake3::new();
    let mut trust = TrustSet::new();
    trust.trust(
        &mut digest,
        signer().public_key(),
        "test-update",
        MaySign::these(&[Kind::update()]),
    );
    open(&head, len, &mut digest, &Ed25519Verifier::new(), &trust)
}

/// `ADR-029` §5 step 8 for one member, reading it out of the payload file.
fn verify_member<'a>(
    package: &'a Package,
    path: &Path,
    index: usize,
) -> Result<VerifiedMember<'a>, PackageError> {
    let member = &package.members()[index];
    let mut file = File::open(path).expect("open");
    file.seek(SeekFrom::Start(member.offset())).expect("seek");
    let mut digest = Blake3::new();
    let mut stream = package.member_stream(index, &mut digest)?;
    let mut buffer = vec![0u8; CHUNK];
    let mut left = member.stored_len();
    while left > 0 {
        let want = CHUNK.min(left as usize);
        file.read_exact(&mut buffer[..want]).expect("read");
        stream.write(&buffer[..want])?;
        left -= want as u64;
    }
    stream.finish()
}

/// Copy a verified member's stored bytes onto a block device.
///
/// **Takes a [`VerifiedMember`], and that is the point.** The token cannot be
/// constructed by a caller, so this function is unreachable with bytes whose
/// digest has not matched the signed header — `ADR-029` §5 step 8's *"refuse on
/// mismatch, having written nothing further"* expressed in the type system
/// rather than in a comment.
fn write_verified_member(
    verified: VerifiedMember<'_>,
    payload: &Path,
    destination: &str,
) -> std::time::Duration {
    refuse_unless_loopback(destination);
    let member = verified.member();

    let mut source = File::open(payload).expect("open");
    source.seek(SeekFrom::Start(member.offset())).expect("seek");
    let mut target = OpenOptions::new()
        .write(true)
        .open(destination)
        .unwrap_or_else(|e| panic!("cannot open {destination} for writing: {e}"));

    let started = Instant::now();
    let mut buffer = vec![0u8; CHUNK];
    let mut left = member.stored_len();
    while left > 0 {
        let want = CHUNK.min(left as usize);
        source.read_exact(&mut buffer[..want]).expect("read");
        target.write_all(&buffer[..want]).expect("write");
        left -= want as u64;
    }
    // ADR-029 §9.2 step 7: fsync. On a card this is what stands between a
    // completed write and a power cut.
    target.sync_all().expect("fsync");
    started.elapsed()
}

// ---------------------------------------------------------------------------

/// The bench's five arguments.
///
/// **Panics rather than skipping when they are absent.** A bench test that
/// silently does nothing is the kind `AGENTS.md` §4 says will rot; these tests
/// are `#[ignore]`d, so the only way to reach this code is to have asked for it.
struct Bench {
    work: PathBuf,
    system_src: PathBuf,
    destination: String,
    boot_a: PathBuf,
    boot_b: PathBuf,
}

impl Bench {
    fn from_environment() -> Self {
        let (work, system_src, destination, boot_a, boot_b) = match (
            var("RIVET_FPK_WORK"),
            var("RIVET_FPK_SYSTEM_SRC"),
            var("RIVET_FPK_SYSTEM_DST"),
            var("RIVET_FPK_BOOT_A"),
            var("RIVET_FPK_BOOT_B"),
        ) {
            (Some(w), Some(s), Some(d), Some(a), Some(b)) => (w, s, d, a, b),
            _ => panic!(
                "this test needs RIVET_FPK_WORK, _SYSTEM_SRC, _SYSTEM_DST, _BOOT_A and \
                 _BOOT_B. It refuses rather than passing quietly."
            ),
        };
        refuse_unless_loopback(&destination);
        Self {
            work: PathBuf::from(work),
            system_src: PathBuf::from(system_src),
            destination,
            boot_a: PathBuf::from(boot_a),
            boot_b: PathBuf::from(boot_b),
        }
    }

    fn payload_path(&self) -> PathBuf {
        self.work.join("rivet-0.0.1-aarch64-rpi4-update.rvt")
    }

    /// Build the payload if it is not already there.
    ///
    /// **Each test stands on its own**, rather than one depending on another
    /// having run first — which is how the first version of this file failed:
    /// the harness runs tests in name order, and the corruption test sorts
    /// before the one that built its fixture.
    fn ensure_payload(&self) -> PathBuf {
        let path = self.payload_path();
        if !path.is_file() {
            println!("building a payload from {:?}", self.system_src);
            let (built, total) =
                build_payload(&self.work, &self.system_src, &self.boot_a, &self.boot_b);
            println!("  payload: {total} bytes at {built:?}");
        }
        path
    }
}

/// The whole of `ADR-029` §9.2 that does not need a firmware: build, verify,
/// write the inactive slot, read it back.
#[test]
#[ignore = "needs a loop device and root; see this file's documentation"]
fn a_signed_payload_is_verified_and_then_written_to_a_loopback_inactive_slot() {
    let bench = Bench::from_environment();
    let Bench {
        work: _,
        system_src,
        destination,
        boot_a: _,
        boot_b: _,
    } = &bench;
    let destination = destination.clone();

    println!("building a payload from {system_src:?}");
    let payload_path = bench.ensure_payload();

    // --- ADR-029 §5 steps 1-7 ---------------------------------------------
    let started = Instant::now();
    let package = open_payload(&payload_path).expect("the payload verifies");
    println!("  header verified in {:?}", started.elapsed());
    assert_eq!(package.header().product(), "rivet");
    assert_eq!(package.header().kind(), &Kind::update());

    let update = Payload::open(&package, reserved()).expect("a well-formed update payload");
    assert_eq!(update.system_member().name(), "system.img");

    // The target is slot B here — the destination the operator named. What a
    // running board would do instead is `running_slot -> other_slot`, which
    // `tests/port_agreement.rs` drives; a loop device has no firmware to ask.
    let boot_for_b: Vec<&str> = update.boot_members_for(Slot::B).map(|m| m.name()).collect();
    assert!(boot_for_b.contains(&BOOT_VARIANT));
    let boot_for_a: Vec<&str> = update.boot_members_for(Slot::A).map(|m| m.name()).collect();
    assert_eq!(boot_for_a.len(), boot_for_b.len());
    println!("  boot half for slot B: {boot_for_b:?}");

    // --- ADR-029 §9.2 step 6: verify EVERY member before writing any ------
    let started = Instant::now();
    let mut system_index = None;
    for index in 0..package.members().len() {
        let verified = verify_member(&package, &payload_path, index)
            .unwrap_or_else(|e| panic!("member {index} did not verify: {e}"));
        if verified.member().name() == "system.img" {
            system_index = Some(index);
        }
    }
    println!("  every member verified in {:?}", started.elapsed());
    let system_index = system_index.expect("the system member");

    // --- ADR-029 §9.2 step 7: write the system member, fsync --------------
    let verified = verify_member(&package, &payload_path, system_index).expect("re-verified");
    let member_len = verified.member().stored_len();
    let member_digest = *verified.member().digest();
    let elapsed = write_verified_member(verified, &payload_path, &destination);
    println!("  wrote {member_len} bytes to {destination} in {elapsed:?}");

    // --- ADR-029 §9.2 step 9: read it back and hash it --------------------
    // "SD cards do accept writes and store something else, and this is the only
    // defence against that." A loop device does not do that, so what this
    // demonstrates is the *mechanism*, not the hazard it defends against.
    let started = Instant::now();
    let mut target = File::open(&destination).expect("open");
    let mut digest = Blake3::new();
    digest.start();
    let mut buffer = vec![0u8; CHUNK];
    let mut left = member_len;
    while left > 0 {
        let want = CHUNK.min(left as usize);
        target.read_exact(&mut buffer[..want]).expect("read");
        digest.write(&buffer[..want]);
        left -= want as u64;
    }
    let read_back = digest.finish();
    println!(
        "  read {member_len} bytes back and hashed them in {:?}",
        started.elapsed()
    );
    assert_eq!(
        read_back, member_digest,
        "the inactive slot does not hold what the signed header says it should"
    );
}

/// **The one that matters more than the happy path.** A corrupted payload must
/// not reach the slot — and after it is refused, the slot must still hold
/// exactly what the previous, good install put there.
///
/// `ADR-029` F3: *"The slot is partially written and **inert**. No selector
/// moved. Report and leave it."* Nothing here moves a selector, and nothing here
/// wipes anything.
#[test]
#[ignore = "needs a loop device and root; run after the test above, which fills the slot"]
fn a_corrupted_payload_does_not_reach_the_slot() {
    let bench = Bench::from_environment();
    let work = bench.work.clone();
    let destination = bench.destination.clone();
    let good = bench.ensure_payload();

    let package = open_payload(&good).expect("the good payload still verifies");
    let system_index = package
        .members()
        .iter()
        .position(|m| m.name() == "system.img")
        .expect("the system member");
    let member_digest = *package.members()[system_index].digest();

    // Put the good payload in the slot first, so "the slot is unchanged" below
    // is a comparison against something rather than against whatever was there.
    // Each test stands on its own; running order must not matter.
    let (already, _) = measure(Path::new(&destination));
    if already != member_digest {
        let verified = verify_member(&package, &good, system_index).expect("the good member");
        let elapsed = write_verified_member(verified, &good, &destination);
        println!("  installed the good payload first, in {elapsed:?}");
    }

    let (before, before_len) = measure(Path::new(&destination));
    assert_eq!(
        before, member_digest,
        "the slot does not hold the good payload"
    );
    let member = &package.members()[system_index];

    // --- 1. A single flipped bit in the middle of the system member --------
    let bent = work.join("bent.rvt");
    std::fs::copy(&good, &bent).expect("copy");
    let at = member.offset() + member.stored_len() / 2;
    {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&bent)
            .expect("open");
        file.seek(SeekFrom::Start(at)).expect("seek");
        let mut byte = [0u8; 1];
        file.read_exact(&mut byte).expect("read");
        byte[0] ^= 0x01;
        file.seek(SeekFrom::Start(at)).expect("seek");
        file.write_all(&byte).expect("write");
        file.sync_all().expect("fsync");
    }
    // The header still verifies — the signature covers [0..4032) and the member
    // content is covered transitively, by its digest.
    let bent_package = open_payload(&bent).expect("the header is untouched and still verifies");
    let outcome = verify_member(&bent_package, &bent, system_index);
    assert!(
        matches!(outcome, Err(PackageError::MemberDigestMismatch { .. })),
        "a flipped bit in the system member was not caught: {outcome:?}"
    );

    // --- 2. A flipped bit in the signed header ----------------------------
    let forged = work.join("forged.rvt");
    std::fs::copy(&good, &forged).expect("copy");
    {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&forged)
            .expect("open");
        // Byte 200 is inside the member table, which the signature covers.
        file.seek(SeekFrom::Start(200)).expect("seek");
        let mut byte = [0u8; 1];
        file.read_exact(&mut byte).expect("read");
        byte[0] ^= 0x01;
        file.seek(SeekFrom::Start(200)).expect("seek");
        file.write_all(&byte).expect("write");
        file.sync_all().expect("fsync");
    }
    assert!(
        matches!(
            open_payload(&forged),
            Err(PackageError::BadSignature { .. })
        ),
        "a flipped bit in the signed header was not caught"
    );

    // --- 3. A truncated transfer ------------------------------------------
    let short = work.join("short.rvt");
    std::fs::copy(&good, &short).expect("copy");
    {
        let file = OpenOptions::new().write(true).open(&short).expect("open");
        let len = std::fs::metadata(&short).expect("stat").len();
        file.set_len(len - 4096).expect("truncate");
    }
    assert!(
        matches!(
            open_payload(&short),
            Err(PackageError::MemberOverflowsFile { .. })
        ),
        "a truncated payload was not caught"
    );

    // --- And the slot is untouched by all three ---------------------------
    let (after, after_len) = measure(Path::new(&destination));
    assert_eq!(after_len, before_len);
    assert_eq!(
        after, before,
        "something reached the slot while three refused payloads were being read"
    );
    println!("  the slot is byte-identical after three refusals");

    for scratch in [&bent, &forged, &short] {
        let _ = std::fs::remove_file(scratch);
    }
}

/// `ADR-029` §3's measurement, re-checked against the card this bench is using
/// rather than believed: **`cmdline.txt` is the only file that differs between
/// the two boot partitions.**
///
/// If a second file ever differs, the payload's slot-variance handling has to
/// grow and `ADR-029` §3's "136 bytes of duplication" stops being the whole cost.
#[test]
#[ignore = "needs both boot partitions mounted"]
fn cmdline_is_still_the_only_file_that_differs_between_the_boot_partitions() {
    let (boot_a, boot_b) = match (var("RIVET_FPK_BOOT_A"), var("RIVET_FPK_BOOT_B")) {
        (Some(a), Some(b)) => (PathBuf::from(a), PathBuf::from(b)),
        _ => panic!("this test needs RIVET_FPK_BOOT_A and RIVET_FPK_BOOT_B"),
    };

    let listing = |dir: &Path| -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("cannot list {dir:?}: {e}"))
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().is_file())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
    };

    let a = listing(&boot_a);
    let b = listing(&boot_b);
    assert_eq!(
        a, b,
        "the two boot partitions do not hold the same file names"
    );

    let mut differ: Vec<&str> = Vec::new();
    for name in &a {
        let (da, _) = measure(&boot_a.join(name));
        let (db, _) = measure(&boot_b.join(name));
        if da != db {
            differ.push(name);
        }
    }
    differ.sort_unstable();
    println!("  files present on both boot partitions: {}", a.len());
    println!("  files that differ: {differ:?}");
    assert_eq!(
        differ,
        vec![BOOT_VARIANT],
        "ADR-029 §3 measured cmdline.txt as the only file that differs between slots; \
         this card disagrees, and the payload's slot-variance handling must grow"
    );
}
