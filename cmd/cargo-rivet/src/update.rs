//! `cargo rivet update` — the verb that produces a signed **system update**
//! `.rvt`.
//!
//! # The hole this fills, stated plainly
//!
//! `ADR-029` designed the payload; `rivet-core` reads and verifies one;
//! `rivet-adapter-image` plans an install and `cmd/rivet-deploy` performs
//! one. **Nothing produced one.** Both ends of the chain were exercised against
//! fixtures — `rivet-crypto/tests/loopback.rs` and `cmd/rivet-deploy`'s own
//! `bench` module, each an `#[ignore]`d test that assembles a payload by hand —
//! so the installer's whole path existed only underneath a test harness. A
//! format one side can install and no side can *build* is not a deploy chain.
//!
//! # Why this verb is here rather than in `cmd/rivet-image`
//!
//! `ADR-029` §10's table says `cmd/rivet-image` *"gains a payload emitter"*,
//! and that row is **superseded** — `ADR-029` §15 carries the reasoning and this
//! is the summary. **`cmd/cargo-rivet` is the only crate in this project that
//! holds a signing key**, and that is its defining property (`ADR-015` §4's
//! first bullet: the private key never leaves the build machine). An emitter in
//! `cmd/rivet-image` would either sign — putting a second signing key holder
//! in the workspace — or emit something unsigned that a second tool then signs,
//! which is a two-step pipeline with an unsigned artifact in the middle.
//!
//! The row was written on 2026-08-23, the day before `cmd/cargo-rivet`
//! existed. It named the only crate that could have done it at the time.
//!
//! # What the input is, and why it is the pieces rather than a card image
//!
//! **A payload is exactly what one slot needs** — `ADR-018` §5.2 and `ADR-029`
//! §9.2 step 7 and 8: one whole `/system` filesystem, and the file set of one
//! boot partition. So the inputs are:
//!
//! | Flag | Is |
//! | :-- | :-- |
//! | `--system PATH` | the `/system` filesystem image — a file, or a partition device. Streamed; never resident |
//! | `--boot-a DIR` | the boot partition of slot A, as files |
//! | `--boot-b DIR` | the boot partition of slot B, as files |
//!
//! **It does not take a card image**, and the omission is deliberate. Slicing
//! one would mean a GPT parser and a FAT reader inside a signing tool — the two
//! things `ADR-029` §5 spent its whole design keeping *out* of the path that
//! reaches a verifier — to save a `losetup` the operator runs anyway. What a
//! card image is, is `cmd/rivet-image`'s; what a payload is, is this.
//!
//! # The slot variance is measured, not declared
//!
//! `ADR-029` §3 rests on a measurement: *"extracting `p1` and `p2` from a built
//! image and diffing them, **`cmdline.txt` is the only file that differs**"*.
//! Both existing fixtures encode the **conclusion** — a hard-coded list of
//! invariant names plus `cmdline.txt` — and each says in its own comments that
//! the list will go stale, because what a card carries is `cmd/rivet-image`'s
//! to decide (`AGENTS.md` §7).
//!
//! **This takes the two directories and does the diff.** A name whose bytes
//! agree across the slots becomes one `Invariant` member; a name whose bytes
//! differ becomes two, `SlotA` and `SlotB`. Nothing here knows that
//! `cmdline.txt` is special, so a release in which a second file becomes
//! slot-variant produces a correct payload instead of a silently wrong one — and
//! the report says which files were which, so the measurement is re-taken on
//! every build rather than quoted from an ADR.
//!
//! A name present on **one** side only is refused. `ADR-029` §6.2's property is
//! *"this artifact and nothing else is what slot B becomes"*, and a payload
//! complete for A and short for B does not have it. `Payload::open` would refuse
//! it on the board anyway (`SlotVarianceIncomplete`); refusing here names the
//! file on the build host's disk instead.
//!
//! # Reserved names are skipped, and every skip is printed
//!
//! `ADR-029` §4 refuses a *payload* containing `autoboot.txt` or
//! `rivet.toml`, and a real boot partition has both. So they are dropped
//! before the builder sees them — and **named in the report**, because §4's own
//! argument against silence is that *"silently skipping would let a future
//! builder believe it had shipped something it had not"*. The list comes from
//! `BootSlots::reserved_names`, never from a constant here: on the Pi it is a
//! pair of filenames and on a UEFI machine the equivalent object is an EFI
//! variable, which is why it is the port's answer.
//!
//! `Builder::reserving` is given the same list as a backstop, so a name that
//! escaped the filter is refused rather than shipped.

use std::path::{Path, PathBuf};

use rivet_adapter_image::{BootSlots, PiFirmware, SlotId};
use rivet_core::package::kind::Kind;
use rivet_core::package::port::{Digest, KeyId, Sign};
use rivet_core::package::update::{
    BootMemberIdentity, PRODUCT, SYSTEM_MEMBER_NAME, SlotVariance, TargetClass,
    check_boot_member_names,
};
use rivet_crypto::{Blake3, Ed25519Signer};

use crate::emit;
use crate::key;
use crate::name;

/// What `cargo rivet update` was asked to do.
pub struct UpdateArgs {
    pub system: PathBuf,
    pub boot_a: PathBuf,
    pub boot_b: PathBuf,
    pub signing_key: PathBuf,
    pub version: String,
    pub product: String,
    pub target: String,
    pub platform: String,
    pub out_dir: PathBuf,
    pub public_key_out: Option<PathBuf>,
}

pub fn usage() -> String {
    format!(
        "usage: cargo rivet update --system PATH --boot-a DIR --boot-b DIR\n\
         \x20                          --signing-key PATH --version VERSION --out-dir DIR\n\
         \n\
         update options:\n\
         \x20 --system PATH       the /system filesystem for the slot an update writes.\n\
         \x20                     A file, or a partition device on a host that has one.\n\
         \x20                     Streamed past the digest; never held in memory\n\
         \x20 --boot-a DIR        slot A's boot partition, as files\n\
         \x20 --boot-b DIR        slot B's boot partition, as files\n\
         \x20                     Both are read and DIFFED: a name whose bytes agree\n\
         \x20                     becomes one invariant member, a name whose bytes\n\
         \x20                     differ becomes two (ADR-029 §3), and a name present\n\
         \x20                     on one side only is refused\n\
         \x20 --signing-key PATH  32 raw bytes, or 64 hex characters. REQUIRED, and\n\
         \x20                     there is deliberately no default location\n\
         \x20                     (PLAN.md §0.4.2)\n\
         \x20 --version VERSION   REQUIRED. This tool did not BUILD the system image\n\
         \x20                     it is handed, so it cannot compose a version that\n\
         \x20                     claims a commit (ADR-027 §3.2.1). Pass what\n\
         \x20                     tools/card/build.sh was given:\n\
         \x20                       git describe --tags --always --dirty\n\
         \x20                     asked of WINDOWS git, not WSL's (AGENTS.md §10.5)\n\
         \x20 --out-dir DIR       REQUIRED. ADR-027 §7: the caller supplies a\n\
         \x20                     directory and the tool composes the name\n\
         \x20 --product NAME      default {PRODUCT}\n\
         \x20 --target TRIPLE     default {}\n\
         \x20 --platform TOKEN    default {}, and it is the only token with a\n\
         \x20                     BootSlots implementor. ADR-027 §3.1 also lists\n\
         \x20                     rpizw and uefi; neither has one yet\n\
         \x20 --public-key-out PATH write the PUBLIC key here, 32 raw bytes\n\
         \n\
         The name is composed and there is no override — ADR-027 §7. An update\n\
         DOES carry <platform>, unlike an application: a system image is\n\
         firmware-shaped (ADR-027 §3.1).\n",
        crate::DEFAULT_TARGET,
        PiFirmware.platform_token(),
    )
}

pub fn parse(args: &[String]) -> Result<UpdateArgs, String> {
    let mut system: Option<PathBuf> = None;
    let mut boot_a: Option<PathBuf> = None;
    let mut boot_b: Option<PathBuf> = None;
    let mut signing_key: Option<PathBuf> = None;
    let mut version: Option<String> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut product = PRODUCT.to_string();
    let mut target = crate::DEFAULT_TARGET.to_string();
    let mut platform = PiFirmware.platform_token().to_string();
    let mut public_key_out = None;

    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = || {
            args.get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag {
            "--system" => system = Some(PathBuf::from(value()?)),
            "--boot-a" => boot_a = Some(PathBuf::from(value()?)),
            "--boot-b" => boot_b = Some(PathBuf::from(value()?)),
            "--signing-key" => signing_key = Some(PathBuf::from(value()?)),
            "--version" => version = Some(value()?),
            "--out-dir" => out_dir = Some(PathBuf::from(value()?)),
            "--product" => product = value()?,
            "--target" => target = value()?,
            "--platform" => platform = value()?,
            "--public-key-out" => public_key_out = Some(PathBuf::from(value()?)),
            other => return Err(format!("{other:?} is not an option.\n\n{}", usage())),
        }
        index += 2;
    }

    Ok(UpdateArgs {
        system: system.ok_or("--system is required: a payload with no /system member is one the board refuses (ADR-029 §9.2 step 5)")?,
        boot_a: boot_a.ok_or("--boot-a is required")?,
        boot_b: boot_b.ok_or("--boot-b is required: the slot-variant half of the payload cannot be derived from one side (ADR-029 §3)")?,
        signing_key: signing_key
            .ok_or("--signing-key is required; there is no default location for one")?,
        version: version.ok_or(
            "--version is required. This tool is handed a system image it did not build, so \
             composing a version here would make ADR-027 §3.2.1's claim about a commit on \
             somebody else's behalf. Pass what tools/card/build.sh was given.",
        )?,
        out_dir: out_dir
            .ok_or("--out-dir is required: ADR-027 §7 gives the tool the name and the caller the directory")?,
        product,
        target,
        platform,
        public_key_out,
    })
}

/// The `BootSlots` implementor a `<platform>` token names.
///
/// **The accepted token is read off the port rather than spelled here**, so
/// this cannot drift from `PiFirmware::platform_token` — which is the thing
/// `ADR-029` §9.2 step 2 compares a signed header against on the machine. Two
/// spellings of `rpi4` is exactly the shape `ADR-011` exists to remove, and the
/// failure it produces is a payload that names a platform no board accepts.
pub(crate) fn slots_for(token: &str) -> Result<&'static dyn BootSlots, String> {
    if token == PiFirmware.platform_token() {
        return Ok(&PiFirmware);
    }
    Err(format!(
        "{token:?} is not a <platform> this build can produce a payload for. The only \
         BootSlots implementor is {:?} ({}). ADR-027 §3.1 also lists `rpizw` and `uefi`; \
         neither has an implementor, and the reserved names and slot layout a payload needs \
         are the port's answer rather than a constant this tool could guess (ADR-029 §4).",
        PiFirmware.platform_token(),
        PiFirmware.platform(),
    ))
}

/// One boot-half member: which file, and which slot it is for.
#[derive(Debug)]
struct BootSource {
    name: String,
    variant: SlotVariance,
    path: PathBuf,
}

/// What the diff of the two boot directories found, for the report.
#[derive(Debug)]
struct BootHalf {
    sources: Vec<BootSource>,
    /// Names dropped because the platform reserves them — `ADR-029` §4.
    reserved: Vec<String>,
    /// Names whose bytes agree across the two slots.
    invariant: Vec<String>,
    /// Names whose bytes differ, and therefore appear twice.
    variant: Vec<String>,
}

/// The regular files directly in `dir`, by name.
///
/// **A subdirectory is refused rather than skipped.** A member name is a flat
/// 32-byte field and `rivet_core::package::format::read_name` refuses a path
/// separator, so a directory is a thing the format cannot carry — and
/// `cmd/rivet-image`'s `boot::contents` writes a flat partition, so one being
/// there at all means something this tool does not understand has appeared.
/// Skipping it would ship a payload missing files nobody was told about.
fn boot_files(dir: &Path) -> Result<std::collections::BTreeMap<String, PathBuf>, String> {
    let mut found = std::collections::BTreeMap::new();
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read the boot directory {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
        let path = entry.path();
        // `metadata` rather than `file_type`: it follows a symlink, and what
        // matters is what the bytes are, not how the host spelled the way there.
        let meta = std::fs::metadata(&path)
            .map_err(|e| format!("cannot stat {}: {e}", path.display()))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if meta.is_dir() {
            return Err(format!(
                "{} is a directory. A Rivet boot partition is flat (cmd/rivet-image's \
                 boot::contents) and a member name cannot contain a path separator, so this \
                 is refused rather than skipped — a payload silently missing whatever is \
                 inside it would install and report success.",
                path.display()
            ));
        }
        found.insert(name, path);
    }
    Ok(found)
}

/// Diff the two boot partitions and turn them into members.
///
/// The comparison is by BLAKE3 digest rather than by holding both files: the
/// largest is `kernel8.img` at about 10 MB and holding two of everything would
/// work today and stop working on the first card that carries more.
fn boot_half(
    boot_a: &Path,
    boot_b: &Path,
    reserved_names: &[&str],
    digest: &mut dyn Digest,
) -> Result<BootHalf, String> {
    let a = boot_files(boot_a)?;
    let b = boot_files(boot_b)?;

    let mut half = BootHalf {
        sources: Vec::new(),
        reserved: Vec::new(),
        invariant: Vec::new(),
        variant: Vec::new(),
    };

    let mut names: Vec<&String> = a.keys().chain(b.keys()).collect();
    names.sort_unstable();
    names.dedup();

    for name in names {
        if reserved_names.iter().any(|r| r == name) {
            half.reserved.push(name.clone());
            continue;
        }
        match (a.get(name), b.get(name)) {
            (Some(in_a), Some(in_b)) => {
                let (digest_a, _) = stream(in_a, digest)?;
                let (digest_b, _) = stream(in_b, digest)?;
                if digest_a == digest_b {
                    half.invariant.push(name.clone());
                    half.sources.push(BootSource {
                        name: name.clone(),
                        variant: SlotVariance::Invariant,
                        path: in_a.clone(),
                    });
                } else {
                    half.variant.push(name.clone());
                    half.sources.push(BootSource {
                        name: name.clone(),
                        variant: SlotVariance::SlotA,
                        path: in_a.clone(),
                    });
                    half.sources.push(BootSource {
                        name: name.clone(),
                        variant: SlotVariance::SlotB,
                        path: in_b.clone(),
                    });
                }
            }
            (present, _) => {
                let (has, missing) = if present.is_some() {
                    ("slot A", "slot B")
                } else {
                    ("slot B", "slot A")
                };
                return Err(format!(
                    "{name:?} is on {has} and not on {missing}. A payload must fully define \
                     either slot — ADR-029 §6.2's \"this artifact and nothing else is what \
                     slot B becomes\" — and one that is complete for {has} would, installed \
                     into {missing}, leave whatever was already there. The board refuses it \
                     too (SlotVarianceIncomplete); refusing here names the file while it is \
                     still on this disk."
                ));
            }
        }
    }

    if half.sources.is_empty() {
        return Err(format!(
            "there is nothing to put in the boot half. {} and {} hold no file that is not \
             reserved (ADR-029 §4), and a payload that replaced /system without its kernel \
             would boot the old one against the new root.",
            boot_a.display(),
            boot_b.display()
        ));
    }
    Ok(half)
}

/// Digest and length of a file or a block device, without holding it.
fn stream(path: &Path, digest: &mut dyn Digest) -> Result<([u8; 32], u64), String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut total = 0u64;
    digest.start();
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        digest.write(&buffer[..read]);
        total += read as u64;
    }
    Ok((digest.finish(), total))
}

pub fn run(args: &[String]) -> Result<String, String> {
    let args = parse(args)?;

    let slots = slots_for(&args.platform)?;
    let arch = name::arch_for(&args.target).ok_or_else(|| {
        format!(
            "{} is not a target this tool knows an <arch> token for (ADR-027 §3.1). Known: \
             aarch64-unknown-linux-musl, x86_64-unknown-linux-musl, arm-unknown-linux-musleabihf",
            args.target
        )
    })?;

    // `ADR-029` §4's reserved names, from the port. `reserved_names` for a
    // boot-directory target is deliberately not conditioned on which slot the
    // target belongs to, so slot A's is the whole answer.
    let target = slots
        .payload(SlotId(1))
        .map_err(|e| format!("this platform cannot describe slot A's targets: {e}"))?;
    let reserved = slots.reserved_names(&target.kernel);

    // The system image. Refused here rather than at the first read, so the
    // 640 MiB digest is not started against a directory.
    let meta = std::fs::metadata(&args.system)
        .map_err(|e| format!("cannot stat {}: {e}", args.system.display()))?;
    if meta.is_dir() {
        return Err(format!(
            "{} is a directory. --system takes the /system FILESYSTEM — the bytes that \
             replace the inactive slot's partition whole (ADR-018 §3, ADR-029 §9.2 step 7) \
             — not the tree they were built from.",
            args.system.display()
        ));
    }

    let seed = key::read_32(&args.signing_key, "signing key")?;
    let mut signer = Ed25519Signer::from_seed(&seed);
    let mut digest = Blake3::new();

    let half = boot_half(&args.boot_a, &args.boot_b, reserved, &mut digest)?;

    // `ADR-029` §9 second bound: the system member first, the boot half last.
    // The install order is `InstallPlan`'s to enforce and does not follow from
    // member order — but a payload whose members are in the order they are
    // installed is one a person reading `inspect` can check against §9.2.
    let mut parts = vec![emit::Part {
        name: SYSTEM_MEMBER_NAME.to_string(),
        class: TargetClass::WholePartition.as_byte(),
        variant: SlotVariance::Invariant.as_byte(),
        source: emit::Source::File(args.system.clone()),
    }];
    for source in &half.sources {
        parts.push(emit::Part {
            name: source.name.clone(),
            class: TargetClass::BootDirectory.as_byte(),
            variant: source.variant.as_byte(),
            source: emit::Source::File(source.path.clone()),
        });
    }

    // The same refusal `Payload::open` applies when a board installs this
    // payload, run here so a name collision that only shows up once ASCII
    // case is folded — or against another member's install-time staging name
    // — is refused before anything is signed, rather than shipped and caught
    // later on a board.
    let identities: Vec<BootMemberIdentity<'_>> = parts
        .iter()
        .enumerate()
        .map(|(index, part)| BootMemberIdentity {
            index,
            name: &part.name,
            target_class: part.class,
            variant: part.variant,
        })
        .collect();
    check_boot_member_names(&identities)
        .map_err(|e| format!("this payload cannot be built: {e}"))?;

    let out = args.out_dir.join(name::artifact(
        &args.product,
        &args.version,
        arch,
        Some(slots.platform_token()),
        &Kind::update(),
    ));
    let written = emit::write(
        &out,
        &emit::Identity {
            product: &args.product,
            version: &args.version,
            arch,
            platform: Some(slots.platform_token()),
            kind: Kind::update(),
        },
        &parts,
        reserved,
        &mut digest,
        &mut signer,
    )?;

    let public = signer.public_key();
    let key_id = KeyId(digest.of(&public.0));
    let mut report = String::new();
    if let Some(path) = &args.public_key_out {
        std::fs::write(path, public.0)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        report.push_str(&format!("public   {} (32 raw bytes)\n", path.display()));
    }

    report.push_str(&format!(
        "wrote    {}\n\
         product  {}\n\
         version  {}  (taken as given: this tool did not build the system image)\n\
         arch     {}\n\
         platform {}  ({} — ADR-027 §3.1, from the boot port)\n\
         kind     {}\n\
         size     {} bytes\n\
         blake3   {}\n\
         key      {} (id {})\n",
        written.path.display(),
        args.product,
        args.version,
        arch,
        slots.platform_token(),
        slots.platform(),
        Kind::update(),
        written.total_len,
        key::hex(&written.digest),
        key::hex(&public.0),
        key_id,
    ));

    report.push_str("members\n");
    for (index, (member_name, length)) in written.members.iter().enumerate() {
        let described = if index == 0 {
            "/system, whole partition".to_string()
        } else {
            match half.sources.get(index - 1).map(|s| s.variant) {
                Some(SlotVariance::Invariant) => "boot, either slot".to_string(),
                Some(SlotVariance::SlotA) => "boot, slot A only".to_string(),
                Some(SlotVariance::SlotB) => "boot, slot B only".to_string(),
                None => "boot".to_string(),
            }
        };
        report.push_str(&format!(
            "  {member_name:<24} {length:>12} bytes  {described}\n"
        ));
    }

    // `ADR-029` §3's measurement, re-taken on this build rather than quoted.
    report.push_str(&format!(
        "\nthe two boot partitions were diffed: {} file(s) identical, {} differing ({})\n",
        half.invariant.len(),
        half.variant.len(),
        if half.variant.is_empty() {
            "none".to_string()
        } else {
            half.variant.join(", ")
        }
    ));
    if half.variant.len() != 1 || half.variant.first().map(String::as_str) != Some("cmdline.txt") {
        report.push_str(
            "  ⚠ ADR-029 §3's measurement is that `cmdline.txt` is the ONLY file that differs\n\
             \x20   between the slots. This build found otherwise. The payload is correct — the\n\
             \x20   variance was measured here, not assumed — but the ADR's number is stale.\n",
        );
    }

    // `ADR-029` §4: skipping is not allowed to be silent.
    for name in &half.reserved {
        report.push_str(&format!(
            "  ⚠ {name} was NOT packaged: {} reserves it on a payload target (ADR-029 §4).\n\
             \x20   Carrying it would let an update overwrite the record of which slot the\n\
             \x20   firmware boots, or the provisioning document PID 1 mutates.\n",
            slots.platform(),
        ));
    }

    report.push_str(&format!(
        "\nNothing here has been installed. `cargo rivet inspect {} --trusted-key PATH`\n\
         reads it back through the same verifier a board runs; on a Rivet,\n\
         `rivet apply install <digest> confirm` hands it to rivet-deploy, which\n\
         verifies it against the card's own trust store (ADR-028 §5.8).\n\
         THE ARTIFACT HAS TO BE IN THE MACHINE'S STORE FIRST — an install names a\n\
         digest and resolves it under /state/artifacts, and there is no form of it\n\
         that names a path (ADR-052 §7). `cargo rivet install --rvt` is what puts\n\
         it there and then asks. /state on a card built at the layout's minimum is\n\
         232 MiB — see PLAN.md §13.4 before assuming it fits, and note the store\n\
         keeps the three newest of each product.\n",
        written.path.display()
    ));
    Ok(report)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn scratch(what: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        crate::scratch::dir(&format!("unit-{what}-{stamp}"))
    }

    fn minimal(args: &[&str]) -> Vec<String> {
        let mut out: Vec<String> = vec![
            "--system".into(),
            "s".into(),
            "--boot-a".into(),
            "a".into(),
            "--boot-b".into(),
            "b".into(),
            "--signing-key".into(),
            "k".into(),
            "--version".into(),
            "0.0.1".into(),
            "--out-dir".into(),
            "o".into(),
        ];
        out.extend(args.iter().map(|a| (*a).to_string()));
        out
    }

    #[test]
    fn the_defaults_are_the_ones_the_project_builds_for() {
        let parsed = parse(&minimal(&[])).unwrap();
        assert_eq!(parsed.product, "rivet");
        assert_eq!(parsed.target, crate::DEFAULT_TARGET);
        // Read off the port rather than spelled, in the test as in the code.
        assert_eq!(parsed.platform, PiFirmware.platform_token());
    }

    /// Each of the five is a decision this tool refuses to make on the
    /// maintainer's behalf, and each refusal names why.
    #[test]
    fn every_required_flag_is_refused_rather_than_defaulted() {
        for (flag, expected) in [
            ("--system", "--system is required"),
            ("--boot-b", "--boot-b is required"),
            ("--signing-key", "--signing-key is required"),
            ("--version", "--version is required"),
            ("--out-dir", "--out-dir is required"),
        ] {
            let full = minimal(&[]);
            let mut without = Vec::new();
            let mut index = 0;
            while index < full.len() {
                if full[index] == flag {
                    index += 2;
                    continue;
                }
                without.push(full[index].clone());
                without.push(full[index + 1].clone());
                index += 2;
            }
            let refused = parse(&without);
            assert!(
                refused.is_err_and(|e| e.contains(expected)),
                "{flag} was not refused"
            );
        }
    }

    #[test]
    fn a_platform_with_no_boot_slots_implementor_is_refused_and_says_which_exist() {
        assert!(slots_for(PiFirmware.platform_token()).is_ok());
        let refused = slots_for("uefi")
            .err()
            .expect("there is no UEFI implementor");
        assert!(refused.contains("rpi4"), "{refused}");
        assert!(refused.contains("uefi"), "{refused}");
    }

    #[test]
    fn a_file_on_one_slot_only_is_refused_naming_the_slot_that_lacks_it() {
        let dir = scratch("one-sided");
        let a = dir.join("a");
        let b = dir.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("kernel8.img"), b"kernel").unwrap();
        std::fs::write(b.join("kernel8.img"), b"kernel").unwrap();
        std::fs::write(a.join("only-on-a"), b"x").unwrap();

        let mut digest = Blake3::new();
        let refused = boot_half(&a, &b, &[], &mut digest).expect_err("one-sided");
        assert!(refused.contains("only-on-a"), "{refused}");
        assert!(refused.contains("slot B"), "{refused}");
    }

    /// The measurement, done rather than declared — and the case that proves it
    /// is a measurement is a file that differs but is **not** `cmdline.txt`.
    #[test]
    fn variance_is_derived_from_the_bytes_and_not_from_a_list_of_names() {
        let dir = scratch("variance");
        let a = dir.join("a");
        let b = dir.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("config.txt"), b"same").unwrap();
        std::fs::write(b.join("config.txt"), b"same").unwrap();
        // Not cmdline.txt. Nothing in this module knows that name.
        std::fs::write(a.join("weird.dtb"), b"for-a").unwrap();
        std::fs::write(b.join("weird.dtb"), b"for-b").unwrap();

        let mut digest = Blake3::new();
        let half = boot_half(&a, &b, &[], &mut digest).unwrap();
        assert_eq!(half.invariant, vec!["config.txt".to_string()]);
        assert_eq!(half.variant, vec!["weird.dtb".to_string()]);
        assert_eq!(half.sources.len(), 3);
        let variants: Vec<SlotVariance> = half.sources.iter().map(|s| s.variant).collect();
        assert_eq!(
            variants,
            vec![
                SlotVariance::Invariant,
                SlotVariance::SlotA,
                SlotVariance::SlotB
            ]
        );
    }

    #[test]
    fn a_reserved_name_is_dropped_and_reported_rather_than_packaged() {
        let dir = scratch("reserved");
        let a = dir.join("a");
        let b = dir.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        for at in [&a, &b] {
            std::fs::write(at.join("kernel8.img"), b"kernel").unwrap();
            std::fs::write(at.join("autoboot.txt"), b"[all]\nboot_partition=1\n").unwrap();
            std::fs::write(at.join("rivet.toml"), b"version = 1\n").unwrap();
        }

        let mut digest = Blake3::new();
        let half = boot_half(&a, &b, &["autoboot.txt", "rivet.toml"], &mut digest).unwrap();
        assert_eq!(
            half.sources.iter().map(|s| &s.name).collect::<Vec<_>>(),
            vec!["kernel8.img"]
        );
        assert_eq!(half.reserved, vec!["autoboot.txt", "rivet.toml"]);
    }

    /// `autoboot.txt` differs between the slots on a real card, so it would be
    /// a slot-variant member if it were not reserved. The reserved check has to
    /// happen **first**, or the diff refuses a payload the platform would
    /// simply have dropped.
    #[test]
    fn a_reserved_name_is_dropped_before_the_diff_can_object_to_it() {
        let dir = scratch("reserved-first");
        let a = dir.join("a");
        let b = dir.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("kernel8.img"), b"kernel").unwrap();
        std::fs::write(b.join("kernel8.img"), b"kernel").unwrap();
        // On one side only, which is the refusal the previous test asserts.
        std::fs::write(a.join("rivet.toml"), b"version = 1\n").unwrap();

        let mut digest = Blake3::new();
        let half = boot_half(&a, &b, &["autoboot.txt", "rivet.toml"], &mut digest).unwrap();
        assert_eq!(half.reserved, vec!["rivet.toml"]);
    }

    #[test]
    fn a_subdirectory_is_refused_rather_than_skipped() {
        let dir = scratch("subdir");
        let a = dir.join("a");
        std::fs::create_dir_all(a.join("overlays")).unwrap();
        let refused = boot_files(&a).expect_err("a directory is not a member");
        assert!(refused.contains("overlays"), "{refused}");
        assert!(refused.contains("flat"), "{refused}");
    }

    #[test]
    fn two_empty_boot_directories_are_refused_rather_than_producing_a_kernel_less_payload() {
        let dir = scratch("empty");
        let a = dir.join("a");
        let b = dir.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let mut digest = Blake3::new();
        let refused = boot_half(&a, &b, &[], &mut digest).expect_err("nothing to ship");
        assert!(refused.contains("nothing to put in the boot half"), "{refused}");
    }
}
