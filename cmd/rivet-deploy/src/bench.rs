//! **The fixture generator for the end-to-end bench**, and nothing else.
//!
//! `AGENTS.md` §6: *"until it runs, it isn't working."* The unit tests in
//! [`crate::install`] drive a `BTreeMap`; this writes a **real, signed `.rvt`**
//! out of a **real card image's** partitions, so that the shipped binary can be
//! run against real block devices and a real FAT filesystem.
//!
//! It is `#[ignore]`d and every path is an environment variable — `AGENTS.md`
//! §8.3 forbids an absolute path in a tracked file, and `AGENTS.md` §9 says to
//! name the target explicitly rather than rely on an ambient check. The same
//! shape as `rivet-crypto/tests/loopback.rs`, which established it.
//!
//! # What it is *not*
//!
//! It is not a second writer of the format. It calls
//! `rivet_core::package::write::Builder` and `rivet-crypto`, exactly as
//! `cargo rivet package` will; what is bespoke here is only *which* files
//! become members, which is the update kind's shape (`ADR-029` §3) and is what
//! `cmd/rivet-image` will own once it grows an emitter (`ADR-029` §10).
//!
//! # Running it
//!
//! Under WSL (`AGENTS.md` §10.5), against a **copy** of a card image over a loop
//! device. `tools/qemu/` has no part in this: `ADR-026` §4 puts `/dev/mmcblk0`
//! in the column the emulator cannot reach at all.
//!
//! ```sh
//! cp "$CARD" "$W/card.img"
//! LOOP=$(losetup -fP --show "$W/card.img")
//! mkdir -p "$W/boot-a" "$W/boot-b"
//! mount -o ro "${LOOP}p1" "$W/boot-a"
//! mount -o ro "${LOOP}p2" "$W/boot-b"
//!
//! RIVET_UPDATE_BENCH_DIR="$W" \
//! RIVET_UPDATE_BENCH_SYSTEM="${LOOP}p3" \
//! RIVET_UPDATE_BENCH_BOOT_A="$W/boot-a" \
//! RIVET_UPDATE_BENCH_BOOT_B="$W/boot-b" \
//!   cargo test -p rivet-deploy -- --ignored --nocapture write_a_payload
//! ```
//!
//! It writes `$DIR/rivet-0.0.2-<arch>-rpi4-update.rvt` and `$DIR/root.pub`,
//! where `<arch>` is `RIVET_UPDATE_BENCH_ARCH` and defaults to `aarch64`.
//! The signing key is derived from a constant seed in this file and is
//! **not a secret** — it exists so that a bench run and its verification agree
//! about which key signed what, and nothing that ships ever sees it.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::fs::File;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::Instant;

use rivet_core::package::kind::Kind;
use rivet_core::package::port::{Digest, Sign as _};
use rivet_core::package::update::{SlotVariance, TargetClass};
use rivet_core::package::write::{Builder, NewMember};
use rivet_crypto::{Blake3, Ed25519Signer, SEED_BYTES};

use crate::kernel::CHUNK;

/// The boot-half files a card carries, in `ADR-029` §6's order.
///
/// **Any that is absent is skipped**, because what a card holds is
/// `cmd/rivet-image`'s to decide and a list hard-coded here would go stale
/// (`AGENTS.md` §7). The same list `rivet-crypto/tests/loopback.rs` carries,
/// and it is a second copy of it — both are bench fixtures, and neither is the
/// authority for what a card contains.
const BOOT_INVARIANT: &[&str] = &[
    "kernel8.img",
    "start4.elf",
    "fixup4.dat",
    "bcm2711-rpi-4-b.dtb",
    "config.txt",
    "tryboot.txt",
];

/// `ADR-029` §3's one slot-variant file, measured rather than assumed.
const BOOT_VARIANT: &str = "cmdline.txt";

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
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

/// The bench's signing key. Derived from a constant, in memory — `AGENTS.md`
/// §8.3, nothing secret is written into a tracked file.
fn signer() -> Ed25519Signer {
    let mut digest = Blake3::new();
    // A DOMAIN SEPARATOR, not a name — deliberately not renamed with the binary,
    // because the only thing required of it is that it never changes.
    let seed: [u8; SEED_BYTES] = digest.of(b"rivet-update bench seed, not a key");
    Ed25519Signer::from_seed(&seed)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Build a real `.rvt` and write it, with the public key beside it.
///
/// **Panics rather than skipping when the arguments are absent.** A bench that
/// silently does nothing is the kind `AGENTS.md` §4 says will rot; this test is
/// `#[ignore]`d, so the only way to reach it is to have asked for it.
#[test]
#[ignore = "writes a real payload from a card image; see this module's documentation"]
fn write_a_payload_from_a_real_card() {
    let (dir, system_src, boot_a, boot_b) = match (
        var("RIVET_UPDATE_BENCH_DIR"),
        var("RIVET_UPDATE_BENCH_SYSTEM"),
        var("RIVET_UPDATE_BENCH_BOOT_A"),
        var("RIVET_UPDATE_BENCH_BOOT_B"),
    ) {
        (Some(d), Some(s), Some(a), Some(b)) => (
            PathBuf::from(d),
            PathBuf::from(s),
            PathBuf::from(a),
            PathBuf::from(b),
        ),
        _ => panic!(
            "this bench needs RIVET_UPDATE_BENCH_DIR, _SYSTEM, _BOOT_A and _BOOT_B. \
             It refuses rather than passing quietly."
        ),
    };

    // **`ADR-029` F6 is not switched off here; it is aimed.**
    //
    // A payload's `arch` is compared against the token the machine's IMAGE
    // records at `rivet_core::package::arch::PATH` (`ADR-027` §3 — "will this
    // binary execute"; since 2026-09-12, and against the architecture the
    // installer was compiled for before that). A development host has no such
    // file unless the bench provides one, and whatever it says decides whether a
    // run exercises the refusal or the install past it.
    //
    // The variable is therefore an argument rather than a default, and what a
    // run means depends on which way it was set. Naming it in the fixture keeps
    // the difference visible in the output rather than in somebody's memory.
    let arch = var("RIVET_UPDATE_BENCH_ARCH").unwrap_or_else(|| "aarch64".to_string());
    println!("  arch: {arch} (ADR-029 F6 compares this against the installer's own)");

    let mut digest = Blake3::new();
    let mut builder = Builder::new(crate::PRODUCT, "0.0.2", &arch, Some("rpi4"), Kind::update())
        .expect("the header fields fit")
        // `ADR-029` §4's reserved names come from the port, not from this file.
        .reserving(reserved());

    // The system member, digested by streaming. 640 MiB never becomes resident.
    let started = Instant::now();
    let (system_digest, system_len) = measure(&system_src);
    println!(
        "  system member: {system_len} bytes from {system_src:?}, digested in {:?}",
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
        .expect("the system member");

    // The boot half. Invariant files come from slot A's partition — they are
    // byte-identical on both, which is the measurement `ADR-029` §3 rests on.
    let mut sources: Vec<(String, u8, PathBuf)> = Vec::new();
    for name in BOOT_INVARIANT {
        let path = boot_a.join(name);
        if path.is_file() {
            sources.push(((*name).to_string(), SlotVariance::Invariant.as_byte(), path));
        } else {
            println!("  (no {name} on this card; skipped)");
        }
    }
    for (dir, variance) in [
        (&boot_a, SlotVariance::SlotA),
        (&boot_b, SlotVariance::SlotB),
    ] {
        let path = dir.join(BOOT_VARIANT);
        assert!(
            path.is_file(),
            "{path:?} must exist — ADR-029 §3's slot-variant member"
        );
        sources.push((BOOT_VARIANT.to_string(), variance.as_byte(), path));
    }

    let mut boot_bytes: Vec<Vec<u8>> = Vec::new();
    for (name, variance, path) in &sources {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("cannot read {path:?}: {e}"));
        println!(
            "  boot member: {name} variant {variance}, {} bytes",
            bytes.len()
        );
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

    // ADR-027 §3's grammar, composed from the same fields the header carries.
    let payload = dir.join(format!("rivet-0.0.2-{arch}-rpi4-update.rvt"));
    let mut out =
        File::create(&payload).unwrap_or_else(|e| panic!("cannot create {payload:?}: {e}"));
    out.write_all(&head).expect("the header");

    let mut source = File::open(&system_src).expect("the system source");
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

    let on_disk = std::fs::metadata(&payload).expect("stat").len();
    assert_eq!(
        on_disk, total,
        "the writer's own length and the file on disk disagree"
    );

    // The public key, raw and 32 bytes — `ADR-029` §7.1, no envelope to strip.
    let key = dir.join("root.pub");
    std::fs::write(&key, signer.public_key().0).expect("the key");

    println!("  payload: {total} bytes at {payload:?}");
    println!("  key:     {:?}", key);
    println!("  system member digest: {}", hex(&system_digest));
}

/// The names `PiFirmware` reserves on a boot-directory target. `ADR-029` §4 —
/// the port's answer, not this file's.
fn reserved() -> &'static [&'static str] {
    use rivet_adapter_image::{BootSlots as _, PiFirmware, SlotId};
    let payload = PiFirmware
        .payload(SlotId(1))
        .expect("slot A is in ADR-018 §4's table");
    PiFirmware.reserved_names(&payload.kernel)
}
