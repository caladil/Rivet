//! The round trip, driven through the built binary.
//!
//! # What this suite proves, and what it cannot
//!
//! `AGENTS.md` §6. It proves that an artifact **this tool writes** is one
//! `rivet_core::package::read` accepts under **real Ed25519 and BLAKE3**, that
//! the program comes back out byte-identical, and that each of five specific
//! corruptions is refused. Every refusal below is checked against a *baseline
//! that succeeds*, in the same test, so none of them can pass vacuously — the
//! failure `PLAN.md` §0.7 records against `a_small_order_public_key_is_refused`.
//!
//! It proves **nothing about a Rivet**. Nothing on a machine verifies or
//! installs one of these (`PLAN.md` §0.8, links 6 and 7), and the program packed
//! here is not even an aarch64 binary — the suite passes `--binary` so it does
//! not need a cross toolchain. A real cross build was run by hand and is
//! reported in `PLAN.md` §0.9; this file cannot stand in for that.
//!
//! **Provenance:** every fixture here is hand-composed except the crate
//! metadata, which is `ADR-015` §3.2's own example table.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const EXE: &str = env!("CARGO_BIN_EXE_cargo-rivet");

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("hello-app")
        .join("Cargo.toml")
}

/// A scratch directory nothing else is using.
///
/// `AGENTS.md` §8.3: *"the test suite mints its own secrets into a temp
/// directory, precisely so nothing has to be committed."* That is why there is
/// no key in this file.
fn scratch(what: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    scratch::dir(&format!("{what}-{stamp}"))
}

/// The crate's own scratch root — see the note in `tests/update.rs`.
#[path = "../src/scratch.rs"]
#[allow(dead_code)]
mod scratch;

/// 32 bytes that are not in any tracked file and are different every run.
fn mint_seed(at: &Path) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(1);
    let pid = u128::from(std::process::id());
    let mut seed = [0u8; 32];
    // Not cryptographic and does not need to be: the property wanted is that
    // two runs of this suite do not share a key and that no key is committed.
    let mut state = stamp ^ (pid << 64) ^ 0x9e37_79b9_7f4a_7c15;
    for byte in &mut seed {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        *byte = (state >> 33) as u8;
    }
    // An all-zero seed is a legal Ed25519 seed but would make two runs share a
    // key if the mixing above ever degenerated. Cheap to notice.
    assert!(seed.iter().any(|b| *b != 0));
    let path = at.join("app.key");
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

/// A fake program: bytes that are not an ELF and do not need to be, because the
/// container has no opinion about a member's contents.
fn fake_program(at: &Path) -> PathBuf {
    let path = at.join("hello-app");
    let bytes: Vec<u8> = (0u32..300_000).map(|n| (n % 251) as u8).collect();
    std::fs::write(&path, bytes).unwrap();
    path
}

/// A **readable** aarch64 ELF: the 64-byte header, correct magic, class,
/// endianness and `e_machine`, and no program headers — which `crate::elf`
/// reads as a static binary needing nothing.
///
/// [`fake_program`] above is deliberately not an ELF, because the container has
/// no opinion about a member's contents. This one has to be one, because the
/// thing under test is what `linkage_report` says about a program it could
/// read.
///
/// **Provenance: hand-composed**, from the field offsets in `elf.rs`.
fn fake_aarch64_program(at: &Path, name: &str, carrying_dwarf: bool) -> PathBuf {
    let bytes = if carrying_dwarf {
        elf_with_sections(&[".text", ".debug_info", ".debug_abbrev"])
    } else {
        // ~~`\0.text\0.debug_info\0.debug_abbrev\0` appended to a bare header.~~
        // **Rewritten 2026-09-04.** That produced a file with the section NAMES
        // in it and no section header table, which was enough while
        // `debug_note` searched the bytes and is exactly the shape that turned
        // out to be a stripped Rust binary. A fixture that stands in for a real
        // program has to have a real table.
        elf_with_sections(&[".text", ".rodata"])
    };
    let path = at.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

/// A 64-bit little-endian aarch64 ELF with a **real section header table**
/// carrying the named sections, plus the null section and `.shstrtab`.
///
/// **Provenance: hand-composed**, from the field offsets in `cargo-rivet`'s own
/// `elf.rs`. It is the smallest file `elf::debug_sections` can read honestly —
/// no program headers, so `crate::elf` reads it as a static binary needing
/// nothing, which is what the linkage half of the report expects.
fn elf_with_sections(named: &[&str]) -> Vec<u8> {
    const EM_AARCH64: u16 = 183;
    const SHDR: usize = 64;

    // The section name string table: a leading NUL, then every name.
    let mut shstrtab = vec![0u8];
    let mut offsets = Vec::new();
    for name in named.iter().copied().chain(std::iter::once(".shstrtab")) {
        offsets.push(u32::try_from(shstrtab.len()).unwrap());
        shstrtab.extend_from_slice(name.as_bytes());
        shstrtab.push(0);
    }

    let strtab_off = 64usize;
    let shoff = strtab_off + shstrtab.len();
    // The null section, one per name, and `.shstrtab` itself.
    let count = named.len() + 2;
    let mut bytes = vec![0u8; shoff + count * SHDR];

    bytes[0..4].copy_from_slice(b"\x7fELF");
    bytes[4] = 2; // ELFCLASS64
    bytes[5] = 1; // ELFDATA2LSB
    bytes[18..20].copy_from_slice(&EM_AARCH64.to_le_bytes());
    bytes[40..48].copy_from_slice(&(shoff as u64).to_le_bytes());
    bytes[58..60].copy_from_slice(&u16::try_from(SHDR).unwrap().to_le_bytes());
    bytes[60..62].copy_from_slice(&u16::try_from(count).unwrap().to_le_bytes());
    bytes[62..64].copy_from_slice(&u16::try_from(count - 1).unwrap().to_le_bytes());
    bytes[strtab_off..shoff].copy_from_slice(&shstrtab);

    for (index, offset) in offsets.iter().enumerate() {
        let at = shoff + (index + 1) * SHDR;
        bytes[at..at + 4].copy_from_slice(&offset.to_le_bytes());
        let size = if index < named.len() { 4096 } else { shstrtab.len() as u64 };
        bytes[at + 32..at + 40].copy_from_slice(&size.to_le_bytes());
        if index == named.len() {
            // `.shstrtab`'s own `sh_offset`, which is how the parser finds it.
            bytes[at + 24..at + 32].copy_from_slice(&(strtab_off as u64).to_le_bytes());
        }
    }
    bytes
}

struct Built {
    dir: PathBuf,
    rvt: PathBuf,
    public: PathBuf,
    program: PathBuf,
}

fn build_one(what: &str) -> Built {
    build_with(what, &[])
}

/// [`build_one`], plus whatever extra arguments the caller wants on the
/// `package` command line.
fn build_with(what: &str, extra: &[&str]) -> Built {
    let dir = scratch(what);
    let seed = mint_seed(&dir);
    let program = fake_program(&dir);
    let public = dir.join("app.pub");
    // Bound rather than written inline: a `&str` taken from a `PathBuf`
    // temporary inside a `vec![]` would not outlive the statement that builds
    // it, where inside the `run(&[..])` call it did.
    let manifest = fixture();
    let mut args = vec![
        "rivet",
        "package",
        "--manifest-path",
        manifest.to_str().unwrap(),
        "--binary",
        program.to_str().unwrap(),
        "--signing-key",
        seed.to_str().unwrap(),
        "--out-dir",
        dir.to_str().unwrap(),
        "--public-key-out",
        public.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    let out = run(&args);
    assert!(out.status.success(), "{}", text(&out));

    let rvt = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "rvt"))
        .expect("no .rvt was written");
    Built {
        dir,
        rvt,
        public,
        program,
    }
}

#[test]
fn a_packaged_application_verifies_and_the_program_comes_back_unchanged() {
    let built = build_one("roundtrip");

    // ADR-027's grammar, composed by the tool. The version field carries a SHA
    // when this tree is not a clean tag, so the middle is not asserted — what is
    // asserted is that there is NO platform component, which is the ruling.
    let name = built.rvt.file_name().unwrap().to_string_lossy().to_string();
    assert!(name.starts_with("hello-app-0.1.0"), "{name}");
    assert!(name.ends_with("-aarch64-application.rvt"), "{name}");
    assert!(!name.contains("rpi4"), "{name}");

    let extracted = built.dir.join("extracted");
    let out = run(&[
        "rivet",
        "inspect",
        built.rvt.to_str().unwrap(),
        "--trusted-key",
        built.public.to_str().unwrap(),
        "--extract-program",
        extracted.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", text(&out));
    let report = text(&out);
    assert!(report.contains("verified"), "{report}");
    assert!(report.contains("manifest.toml"), "{report}");
    assert!(report.contains("bind tcp:8080"), "{report}");
    assert!(report.contains("network group:media"), "{report}");
    assert!(report.contains("connect photo-index:api"), "{report}");
    assert!(report.contains("NOT ENFORCED"), "{report}");
    // ADR-027 §9 item 3, visible in the report rather than merely true.
    assert!(report.contains("carries none"), "{report}");

    assert_eq!(
        std::fs::read(&extracted).unwrap(),
        std::fs::read(&built.program).unwrap()
    );
}

/// **`inspect` says who the application will be.** `ADR-015` §5.1.1, Pile B
/// item 2, added 2026-08-29.
///
/// `PLAN.md` §102.6's cold agent needed the uid, could not get it from this
/// tool, and inferred it — and separately lost a deploy to `ADR-015` §3.2's
/// wrong writable directory, which §3.2A corrected 400 lines away from where
/// the wrong sentence was. Both are answered by printing the two values,
/// **computed by the same functions the machine uses**.
///
/// The expected uid is taken from `rivet_core::service::app_identity` rather
/// than written out as a number. A literal here would be a second authority for
/// a value `service.rs` calls part of the on-disk contract — and a test that
/// agreed with a literal would keep agreeing with it after the function changed,
/// which is the drift `ADR-011` is about.
#[test]
fn inspect_reports_the_uid_the_application_will_run_as_and_the_one_path_it_can_write() {
    let built = build_one("identity");
    let out = run(&[
        "rivet",
        "inspect",
        built.rvt.to_str().unwrap(),
        "--trusted-key",
        built.public.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", text(&out));
    let report = text(&out);

    let name = rivet_core::service::ServiceName::new("hello-app").expect("a service name");
    let id = rivet_core::service::app_identity(&name);
    let data = rivet_core::service::data_dir(rivet_core::service::BUILT_IN_VOLUME, &name);

    assert!(report.contains("runs as"), "{report}");
    assert!(
        report.contains(&format!("uid {}, gid {}", id.uid, id.gid)),
        "the report does not carry the identity the machine will derive: {report}"
    );
    // The uid is inside the range applications are given, so a report of `0`
    // could never pass this test — which is the failure worth catching.
    assert!(
        (rivet_core::service::APP_UID_BASE..=rivet_core::service::APP_UID_LAST).contains(&id.uid),
        "{}",
        id.uid
    );
    assert!(
        report.contains(&data),
        "the report does not name the only directory this application can write \
         (ADR-015 §3.2A): {report}"
    );
    // The path is the one §3.2A corrected TO, not the one §3.2 got wrong.
    assert!(report.contains("/state/app/hello-app/data"), "{report}");
    assert!(report.contains("ADR-015 §5.1.1"), "{report}");
    assert!(report.contains("ADR-015 §3.2A"), "{report}");
}

/// Five corruptions, each against the same artifact that has just been shown to
/// verify. The baseline is re-run inside the test so no refusal can pass because
/// the fixture was broken to begin with.
#[test]
fn every_corruption_is_refused_and_the_untouched_artifact_is_not() {
    let built = build_one("corruption");
    let good = std::fs::read(&built.rvt).unwrap();

    let verify = |bytes: &[u8], label: &str| -> Output {
        let path = built.dir.join(format!("{label}.rvt"));
        std::fs::write(&path, bytes).unwrap();
        run(&[
            "rivet",
            "inspect",
            path.to_str().unwrap(),
            "--trusted-key",
            built.public.to_str().unwrap(),
        ])
    };

    // 0. The baseline. If this fails, nothing below means anything.
    let baseline = verify(&good, "baseline");
    assert!(baseline.status.success(), "{}", text(&baseline));

    // 1. One bit in a member's content — past the 4096-byte header, so the
    //    signature still checks out and only the member digest can catch it.
    let mut flipped = good.clone();
    let at = 4096 + flipped.len() / 2 - 2048;
    flipped[at] ^= 0x01;
    let out = verify(&flipped, "member-bit");
    assert!(!out.status.success());
    assert!(text(&out).contains("digest"), "{}", text(&out));

    // 2. One bit inside the signed header — the `version` field, which is a
    //    thing a person might actually want to change after the fact.
    let mut forged = good.clone();
    forged[80] ^= 0x20;
    let out = verify(&forged, "header-bit");
    assert!(!out.status.success());
    assert!(text(&out).contains("signature"), "{}", text(&out));

    // 3. A truncated transfer — ADR-029 F1.
    let out = verify(&good[..good.len() - 1], "truncated");
    assert!(!out.status.success());
    assert!(text(&out).contains("truncated"), "{}", text(&out));

    // 4. A file shorter than a header at all, which is a different refusal.
    let out = verify(&good[..100], "stub");
    assert!(!out.status.success());
    assert!(text(&out).contains("nothing to verify"), "{}", text(&out));

    // 5. The right artifact, the wrong trust set. A key that signed nothing.
    let other = built.dir.join("other.pub");
    std::fs::write(&other, [0x11u8; 32]).unwrap();
    let out = run(&[
        "rivet",
        "inspect",
        built.rvt.to_str().unwrap(),
        "--trusted-key",
        other.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert!(text(&out).contains("trust set"), "{}", text(&out));

    // And the baseline again, last, so a mutation that escaped into the shared
    // artifact would be caught rather than reported as five clean refusals.
    let baseline = verify(&good, "baseline-again");
    assert!(baseline.status.success(), "{}", text(&baseline));
}

/// The verifier is where a trust decision is made, so a tool that could be asked
/// to skip it would be a tool with a flag for accepting anything.
#[test]
fn inspecting_without_a_trusted_key_is_refused() {
    let built = build_one("no-key");
    let out = run(&["rivet", "inspect", built.rvt.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(text(&out).contains("--trusted-key"), "{}", text(&out));
}

/// **The comment above this test was still true and the test below it was
/// not.** *"`--signing-key` has no default, because where a signing key lives
/// is not ruled (`PLAN.md` §0.4.2)"* described the tool until 2026-08-30, when
/// `resolve_signing_key` gave it one — an environment variable, then
/// `~/.rivet/keys/rivet-app.seed` — to close a blocker a cold agent hit. The
/// ruling that a *repository* must not publish the path was kept; what changed
/// is that the tool may look.
///
/// From then on this asserted a refusal that only happens on a machine with no
/// key, so it passed on CI and had been failing on the maintainer's workstation
/// (found 2026-09-01). A test whose verdict depends on the contents of a home
/// directory is not testing the tool.
///
/// So it now pins the part that holds either way, and it is the part that
/// mattered to the agent: **not being told merely "no".**
#[test]
fn packaging_without_a_signing_key_never_just_says_no() {
    let out = run(&[
        "rivet",
        "package",
        "--manifest-path",
        fixture().to_str().unwrap(),
    ]);
    if out.status.success() {
        return; // A key was resolved, which is the 2026-08-30 behaviour.
    }
    let said = text(&out);
    assert!(
        said.contains("RIVET") || said.contains(".rivet") || said.contains("--signing-key"),
        "a refusal must name a place to look or a flag to pass: {said}"
    );
}

#[test]
fn a_key_that_is_not_thirty_two_bytes_is_refused_rather_than_padded() {
    let dir = scratch("short-key");
    let short = dir.join("short.key");
    std::fs::write(&short, [0u8; 31]).unwrap();
    let out = run(&[
        "rivet",
        "package",
        "--manifest-path",
        fixture().to_str().unwrap(),
        "--binary",
        fake_program(&dir).to_str().unwrap(),
        "--signing-key",
        short.to_str().unwrap(),
        "--out-dir",
        dir.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert!(text(&out).contains("exactly 32"), "{}", text(&out));
}

/// Hex is the other way a person hands over 32 bytes, and it must produce the
/// same key as the raw form rather than a second one.
#[test]
fn a_hex_key_and_the_raw_bytes_it_spells_are_the_same_key() {
    let dir = scratch("hex-key");
    let program = fake_program(&dir);
    let raw_seed = mint_seed(&dir);
    let bytes = std::fs::read(&raw_seed).unwrap();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let hex_seed = dir.join("app.hex");
    std::fs::write(&hex_seed, format!("{hex}\n")).unwrap();

    let mut ids = Vec::new();
    for (label, seed) in [("raw", &raw_seed), ("hex", &hex_seed)] {
        let out_dir = dir.join(label);
        let out = run(&[
            "rivet",
            "package",
            "--manifest-path",
            fixture().to_str().unwrap(),
            "--binary",
            program.to_str().unwrap(),
            "--signing-key",
            seed.to_str().unwrap(),
            "--out-dir",
            out_dir.to_str().unwrap(),
        ]);
        assert!(out.status.success(), "{}", text(&out));
        let report = text(&out);
        let line = report
            .lines()
            .find(|l| l.starts_with("key "))
            .expect("no key line")
            .to_string();
        ids.push(line);
    }
    assert_eq!(ids[0], ids[1]);
}

/// A `Cargo.toml` written for this test rather than by the emitter — the
/// metadata refusals are the ones an author actually hits.
fn crate_with(metadata: &str, at: &Path) -> PathBuf {
    let dir = at.join("crate");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src").join("main.rs"), "fn main() {}\n").unwrap();
    let manifest = dir.join("Cargo.toml");
    std::fs::write(
        &manifest,
        format!(
            "[workspace]\n\n[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \
             \"2024\"\npublish = false\n\n{metadata}"
        ),
    )
    .unwrap();
    manifest
}

fn package_metadata(metadata: &str, label: &str) -> Output {
    let dir = scratch(label);
    let manifest = crate_with(metadata, &dir);
    let seed = mint_seed(&dir);
    let program = fake_program(&dir);
    run(&[
        "rivet",
        "package",
        "--manifest-path",
        manifest.to_str().unwrap(),
        "--binary",
        program.to_str().unwrap(),
        "--signing-key",
        seed.to_str().unwrap(),
        "--out-dir",
        dir.join("out").to_str().unwrap(),
    ])
}

#[test]
fn a_manifest_that_asks_for_something_this_build_does_not_define_is_refused() {
    // The baseline: the smallest legal table.
    let ok = package_metadata(
        "[package.metadata.rivet]\nkind = \"oneshot\"\n",
        "meta-ok",
    );
    assert!(ok.status.success(), "{}", text(&ok));

    // No table at all — silence is not a default, because the manifest IS the
    // permission request.
    let out = package_metadata("", "meta-absent");
    assert!(!out.status.success());
    assert!(
        text(&out).contains("package.metadata.rivet"),
        "{}",
        text(&out)
    );

    // `exec`, which ADR-015 §3.2 forbids by name and for a reason worth
    // repeating to whoever typed it.
    let out = package_metadata(
        "[package.metadata.rivet]\nkind = \"daemon\"\nexec = \"/bin/sh\"\n",
        "meta-exec",
    );
    assert!(!out.status.success());
    assert!(
        text(&out).contains("signature never covered"),
        "{}",
        text(&out)
    );

    // A typo, which must be reported as the typo rather than as a missing key.
    let out = package_metadata(
        "[package.metadata.rivet]\nkind = \"daemon\"\nnetwrok = \"none\"\n",
        "meta-typo",
    );
    assert!(!out.status.success());
    assert!(text(&out).contains("netwrok"), "{}", text(&out));

    // A value outside the enumeration.
    let out = package_metadata(
        "[package.metadata.rivet]\nkind = \"service\"\n",
        "meta-kind",
    );
    assert!(!out.status.success());
    assert!(text(&out).contains("daemon, oneshot"), "{}", text(&out));

    // A port that cannot be passed as a descriptor.
    let out = package_metadata(
        "[package.metadata.rivet]\nkind = \"daemon\"\nlisten = [\"tcp:0\"]\n",
        "meta-port",
    );
    assert!(!out.status.success());
    assert!(text(&out).contains("tcp:<port>"), "{}", text(&out));
}

/// **A `cpu` request below the kernel's representable minimum is refused at
/// packaging** — `.rulings/` R-210 D22. The grammar `CpuLimit::parse` shares
/// with the desired-state document allows a cap as fine as one microsecond,
/// and cgroup v2 refuses any quota under one millisecond. Signing and
/// shipping this would mean the author learns about it from a board's own
/// journal, on every boot, with no cause named; this is the point in the
/// chain where the number is still in front of them, and it now stops here
/// rather than only noting it and packaging anyway.
#[test]
fn a_cpu_request_below_the_kernels_minimum_is_refused_at_packaging() {
    let refused = package_metadata(
        "[package.metadata.rivet]\nkind = \"daemon\"\ncpu = \"0.005\"\n",
        "cpu-refused",
    );
    assert!(!refused.status.success(), "{}", text(&refused));
    let report = text(&refused);
    assert!(report.contains("cpu"), "{report}");
    assert!(report.contains("kernel"), "{report}");

    // An ordinary request the kernel can keep still packages.
    let quiet = package_metadata(
        "[package.metadata.rivet]\nkind = \"daemon\"\ncpu = \"0.5\"\n",
        "cpu-quiet",
    );
    assert!(quiet.status.success(), "{}", text(&quiet));

    // No `cpu` at all is the ordinary case and still packages.
    let none = package_metadata("[package.metadata.rivet]\nkind = \"daemon\"\n", "cpu-none");
    assert!(none.status.success(), "{}", text(&none));
}

/// `ADR-015` §5.4's own worked example packages, and an over-long name is
/// still refused with the limit named.
///
/// **This assertion used to run the other way.** `product` was sixteen bytes
/// and `metrics-collector` is seventeen characters, so the ADR's own example
/// could not be packaged. The maintainer ruled the field grown to 32 on
/// 2026-08-24, while it could still be done (.rulings/ R-046), because nothing
/// had ever verified an `.rvt` on a Rivet, so there were zero deployed
/// artifacts and the change was free exactly once. `ADR-029` §3.1 carries it.
///
/// The refusal still has to exist. A truncated name is an artifact that
/// verifies and is not the one the author thinks they built.
#[test]
fn the_adrs_own_example_packages_and_an_over_long_name_is_still_refused() {
    let dir = scratch("long-name");

    // Seventeen characters. This is the case that used to fail.
    let ok = package_named(&dir, "metrics-collector");
    assert!(
        ok.status.success(),
        "ADR-015 §5.4's example must package now: {}",
        text(&ok)
    );

    // Thirty-three, which still does not fit in thirty-two.
    let refused = package_named(&dir, "product-name-with-thirty-three-ch");
    assert!(!refused.status.success());
    let report = text(&refused);
    assert!(report.contains("product"), "{report}");
    assert!(report.contains("32"), "{report}");
    assert!(report.contains("33"), "{report}");
}

/// Build a throwaway crate with `name` and try to package it.
fn package_named(dir: &std::path::Path, name: &str) -> std::process::Output {
    let crate_dir = dir.join(name);
    std::fs::create_dir_all(crate_dir.join("src")).unwrap();
    std::fs::write(
        crate_dir.join("src").join("main.rs"),
        "fn main() {}
",
    )
    .unwrap();
    let manifest = crate_dir.join("Cargo.toml");
    std::fs::write(
        &manifest,
        format!(
            "[workspace]

[package]
name = \"{name}\"
version = \"0.3.1\"
edition =              \"2024\"
publish = false

[package.metadata.rivet]
kind = \"daemon\"
"
        ),
    )
    .unwrap();
    run(&[
        "rivet",
        "package",
        "--manifest-path",
        manifest.to_str().unwrap(),
        "--binary",
        fake_program(dir).to_str().unwrap(),
        "--signing-key",
        mint_seed(dir).to_str().unwrap(),
        "--out-dir",
        dir.join("out").to_str().unwrap(),
    ])
}

/// A target this tool has no `<arch>` token for is refused before anything is
/// built, rather than named by splitting the triple at the first dash.
#[test]
fn an_unknown_target_is_refused_before_anything_is_built() {
    let dir = scratch("target");
    let out = run(&[
        "rivet",
        "package",
        "--manifest-path",
        fixture().to_str().unwrap(),
        "--binary",
        fake_program(&dir).to_str().unwrap(),
        "--signing-key",
        mint_seed(&dir).to_str().unwrap(),
        "--target",
        "aarch64-unknown-linux-gnu",
        "--out-dir",
        dir.to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    assert!(text(&out).contains("ADR-027"), "{}", text(&out));
}

/// **`assets/` arrives in the artifact with no `--asset` argument anywhere** —
/// register `R-26`, driven through the built binary and read back through the
/// same verifier a board would use.
///
/// The unit tests around `assets_in` prove the walk; this proves the whole
/// chain, which is the half that actually failed. `PLAN.md` §119.3:
/// `aura-renderer` was deployed without its four typefaces because `--asset`
/// was the only way in, exited on `SourceCodePro-Bold.ttf: No such file or
/// directory`, and left a board's screen black for hours with the display
/// hand-over working perfectly. Nothing refused, because nothing had been asked
/// for — so the test that matters is one where the command line says nothing
/// about assets at all, exactly as that deploy's did.
///
/// The fixture's two files are named so the order is visible: `Aardvark-Bold
/// .ttf` sorts before `greeting.txt` and `read_dir` does not promise either.
#[test]
fn the_assets_directory_is_packaged_without_being_asked_for() {
    let built = build_one("assets");
    let out = run(&[
        "rivet",
        "inspect",
        built.rvt.to_str().unwrap(),
        "--trusted-key",
        built.public.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", text(&out));
    let report = text(&out);

    // Two assets, each named by its own filename, each described as an asset.
    for line in ["Aardvark-Bold.ttf", "greeting.txt"] {
        assert!(
            report.lines().any(|l| l.contains(line) && l.contains("asset")),
            "{line} is not in the artifact as an asset:\n{report}"
        );
    }
    // Four members: manifest, program, and the two above. A fifth would mean
    // something was picked up that nobody put in `assets/`.
    assert!(report.contains("members  4"), "{report}");

    // And the order the walker fixed, so an artifact is a function of the tree
    // rather than of the filesystem's enumeration order.
    let at = |needle: &str| report.find(needle).expect(needle);
    assert!(
        at("Aardvark-Bold.ttf") < at("greeting.txt"),
        "the members are not in sorted order:\n{report}"
    );
}

/// **`--no-assets` is the way out, and it is a thing the author typed.**
///
/// Everything in `assets/` ships by default because a filter is a rule about
/// which files silently do not arrive — which is the defect the walk closes. A
/// crate whose `assets/` holds source material rather than shipped files needs
/// a way to say so, and this is it.
#[test]
fn no_assets_packages_none_of_them() {
    let built = build_with("noassets", &["--no-assets"]);
    let out = run(&[
        "rivet",
        "inspect",
        built.rvt.to_str().unwrap(),
        "--trusted-key",
        built.public.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", text(&out));
    let report = text(&out);
    assert!(!report.contains("greeting.txt"), "{report}");
    assert!(!report.contains("Aardvark-Bold.ttf"), "{report}");
    assert!(report.contains("members  2"), "manifest and program only:\n{report}");
}

/// **A program carrying DWARF gets a line about it, in the report the operator
/// reads** — `PLAN.md` §123 Wave 4.4, register `R-25`.
///
/// `debug_note` has a unit test over the byte search. This is the other half
/// and it is the half that was missing: that the sentence survives
/// `linkage_report`, reaches the operator's own output, and is absent for a
/// stripped program. `size_check` refuses only past 16 MiB, so everything under
/// it packaged with nothing said — `out/rivet-launcher-0.1.61` at 10.6 MB, ten
/// times its stripped size, hashed whole at every launch (`ADR-015` §4.2).
///
/// It is deliberately **not** a refusal. `ADR-005` §7.5 records what stripping
/// the only copy of a released binary cost a debugger, so shipping symbols is a
/// trade the author is allowed to make; this is the sentence that tells them
/// they are making it.
#[test]
fn a_packaged_program_carrying_dwarf_says_so_in_the_report() {
    let dir = scratch("dwarf");
    let seed = mint_seed(&dir);
    let with = fake_aarch64_program(&dir, "hello-app", true);
    let out = run(&[
        "rivet",
        "package",
        "--manifest-path",
        fixture().to_str().unwrap(),
        "--binary",
        with.to_str().unwrap(),
        "--signing-key",
        seed.to_str().unwrap(),
        "--out-dir",
        dir.join("with").to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", text(&out));
    let report = text(&out);
    assert!(report.contains("DWARF"), "the operator is not told: {report}");
    assert!(report.contains("RIVET_STRIP"), "it must name the tool: {report}");
    assert!(
        report.contains("ADR-005"),
        "it must not advise throwing the symbols away: {report}"
    );
    // A warning, not a refusal — the artifact is still written.
    assert!(report.contains("wrote "), "{report}");

    // The same program without the section name says nothing at all. A note on
    // every package would be a note nobody reads.
    let stripped = fake_aarch64_program(&dir, "stripped", false);
    let out = run(&[
        "rivet",
        "package",
        "--manifest-path",
        fixture().to_str().unwrap(),
        "--binary",
        stripped.to_str().unwrap(),
        "--signing-key",
        seed.to_str().unwrap(),
        "--out-dir",
        dir.join("without").to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", text(&out));
    let report = text(&out);
    assert!(!report.contains("DWARF"), "a stripped program must be silent: {report}");
    assert!(report.contains("linkage  static"), "it was read as an ELF: {report}");
}

/// A `--asset` naming a member `assets/` already provides is refused rather
/// than silently overriding it — both would install to one path.
#[test]
fn an_asset_flag_that_repeats_a_walked_name_is_refused() {
    let dir = scratch("collide");
    let out = run(&[
        "rivet",
        "package",
        "--manifest-path",
        fixture().to_str().unwrap(),
        "--binary",
        fake_program(&dir).to_str().unwrap(),
        "--signing-key",
        mint_seed(&dir).to_str().unwrap(),
        "--out-dir",
        dir.to_str().unwrap(),
        "--asset",
        "greeting.txt=Cargo.toml",
    ]);
    assert!(!out.status.success(), "{}", text(&out));
    let why = text(&out);
    assert!(why.contains("greeting.txt"), "{why}");
    assert!(why.contains("--no-assets"), "it says what to do instead: {why}");
}
