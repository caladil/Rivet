//! `cargo rivet library` — emit a **library `.rvt`**. Shared Rivets, 2026-08-31.
//!
//! `docs/design-candidates/Shared-Libraries-and-the-Build-Target.md` §4.4: a
//! library is a payload in the same container, signed with the same key and
//! verified by the same verifier. This is the emitter, and it is deliberately
//! **not** a mode of [`crate::package`].
//!
//! # Why a separate verb rather than `package --library`
//!
//! Because a library is usually not a Cargo crate. `cargo rivet package` reads
//! `Cargo.toml`, runs `cargo build`, and takes its name, version and manifest
//! from `[package.metadata.rivet]` — every one of which presumes a Rust crate.
//! The libraries this mechanism exists for are C and C++ built by a blueprint
//! (§5), so this verb takes the finished objects and is told the name and the
//! version.
//!
//! A `--library` flag on `package` would have meant most of that machinery
//! being skipped by a conditional, which is the shape that ends up with a tool
//! whose behaviour nobody can state in one sentence.
//!
//! # What it refuses, and why each refusal is here rather than on the board
//!
//! | Refused | Because |
//! | :-- | :-- |
//! | an object with no `DT_SONAME` | the loader finds a library **by** its SONAME. One without is a file no `DT_NEEDED` can name, so it would install perfectly and never be found |
//! | a `DT_SONAME` that is not the file's own name | there would then be two names for one object — the one it installs under and the one the loader asks for — and they would silently disagree |
//! | an object that is not a shared object | a static archive or an executable in a library artifact is a build mistake, and it is cheaper to say so here than to have a loader say `ENOEXEC` on a board |
//!
//! Each is `ADR-015` §6.1's rule pointed at a build tool: the moment a
//! declaration cannot be honoured is the moment to say so, and the earliest
//! such moment is here.

use std::path::{Path, PathBuf};

use rivet_core::package::kind::Kind;
use rivet_core::package::library::{self, MANIFEST_NAME, Reference, TargetClass};
use rivet_crypto::{Blake3, Ed25519Signer, public_key_for};
use rivet_remote::name;

use crate::elf;
use crate::emit;
use crate::key;

#[derive(Debug)]
pub struct Args {
    pub name: String,
    pub version: String,
    pub objects: Vec<PathBuf>,
    pub needs: Vec<Reference>,
    /// **The class of addon this library provides** —
    /// `PLAN-Feature-Set-2026-09-12` §6.2, `ADR-053`. `None` is the ordinary
    /// answer: this library joins no application by class, which is every
    /// library emitted before `ADR-053`.
    pub provides_class: Option<String>,
    /// **Which of [`Self::objects`] is the addon** — `.rulings/` R-201. The
    /// SONAME, which for an object this verb accepts is also its file name.
    ///
    /// `None` is the ordinary answer and means one of two things the manifest
    /// tells apart: a library that provides no class at all, or one that
    /// carries a single object and so has nothing to choose between.
    pub addon_soname: Option<String>,
    pub signing_key: PathBuf,
    pub target: String,
    pub out_dir: Option<PathBuf>,
    pub public_key_out: Option<PathBuf>,
}

pub fn usage() -> String {
    "usage: cargo rivet library --name NAME --version VERSION --object PATH [options]\n\
     \n\
     Emits a LIBRARY .rvt — a signed, content-addressed shared library that an\n\
     application declares in its own closure. Shared Rivets.\n\
     \n\
     library options:\n\
     \x20 --name NAME           REQUIRED. What the library is called. Becomes the\n\
     \x20                       header's `product` and the first part of the\n\
     \x20                       reference an application declares\n\
     \x20 --version VERSION     REQUIRED. Never parsed and never compared — it is\n\
     \x20                       there so a person reading a directory can tell two\n\
     \x20                       installs apart. The DIGEST is the identity\n\
     \x20 --object PATH         REQUIRED, repeatable. A shared object built for the\n\
     \x20                       target. Its file name must equal its DT_SONAME\n\
     \x20 --needs REFERENCE     repeatable. A library reference this library itself\n\
     \x20                       needs, exactly as printed by a previous run\n\
     \x20 --provides-class CLASS  the class of addon this library provides\n\
     \x20                       (PLAN-Feature-Set-2026-09-12 §6.2, ADR-053).\n\
     \x20                       Optional: absent means this library joins no\n\
     \x20                       application by class. Lowercase letters, digits\n\
     \x20                       and hyphens, beginning with a letter\n\
     \x20 --addon-soname NAME   which object is the addon, by its SONAME\n\
     \x20                       (.rulings/ R-201). REQUIRED when this library\n\
     \x20                       provides a class and carries more than one\n\
     \x20                       --object; pointless and refused without a\n\
     \x20                       class. A library with a single object needs it\n\
     \x20                       not: there is nothing to choose between\n\
     \x20 --signing-key PATH    32 raw bytes, or 64 hex characters. Optional:\n\
     \x20                       without it, $RIVET_APP_SEED then\n\
     \x20                       ~/.rivet/keys/rivet-app.seed\n\
     \x20 --target TRIPLE       default aarch64-unknown-linux-musl. Names the\n\
     \x20                       <arch> token only; nothing is built here\n\
     \x20 --out-dir DIR         where the .rvt goes (default ./target/rivet)\n\
     \x20 --public-key-out PATH write the PUBLIC key here, 32 raw bytes\n\
     \n\
     THE REFERENCE IS THE OUTPUT THAT MATTERS. This verb prints one line that an\n\
     application's Cargo.toml pastes into `[package.metadata.rivet] library`. It is\n\
     the name, the version and a digest of this exact artifact, and it is also the\n\
     directory the library installs to on a board — one string, so there is nothing\n\
     to keep in step.\n\
     \n\
     NOTHING IS BUILT HERE. `cargo rivet package` runs cargo; this takes objects\n\
     that already exist, because the libraries this exists for are C and C++ built\n\
     by a blueprint and not by cargo.\n"
        .to_string()
}

pub fn parse(args: &[String]) -> Result<Args, String> {
    let mut name_arg: Option<String> = None;
    let mut version: Option<String> = None;
    let mut objects = Vec::new();
    let mut needs = Vec::new();
    let mut provides_class: Option<String> = None;
    let mut addon_soname: Option<String> = None;
    let mut signing_key: Option<PathBuf> = None;
    let mut target = crate::DEFAULT_TARGET.to_string();
    let mut out_dir = None;
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
            "--name" => name_arg = Some(value()?),
            "--version" => version = Some(value()?),
            "--object" => objects.push(PathBuf::from(value()?)),
            "--needs" => {
                let text = value()?;
                needs.push(Reference::parse(&text).map_err(|fault| {
                    format!(
                        "--needs {text:?} is not a library reference: {fault}.\n\
                         A reference is what `cargo rivet library` prints when it emits one — \
                         <name>-<version>-<64 hex>. It is copied, never composed by hand."
                    )
                })?);
            }
            "--provides-class" => provides_class = Some(value()?),
            // **A SONAME, not a path**, `.rulings/` R-201. `--object` takes a
            // path because it reads a file; this names one of the objects
            // that were read, and this verb has already refused any object
            // whose SONAME and file name differ — so there is exactly one
            // string that can be meant and a second spelling to be tolerant
            // of would be the very ambiguity that refusal exists to remove.
            "--addon-soname" => addon_soname = Some(value()?),
            "--signing-key" => signing_key = Some(PathBuf::from(value()?)),
            "--target" => target = value()?,
            "--out-dir" => out_dir = Some(PathBuf::from(value()?)),
            "--public-key-out" => public_key_out = Some(PathBuf::from(value()?)),
            other => return Err(format!("{other:?} is not an option.\n\n{}", usage())),
        }
        index += 2;
    }

    Ok(Args {
        name: name_arg.ok_or("--name is required")?,
        version: version.ok_or("--version is required")?,
        objects: match objects.is_empty() {
            true => {
                return Err("--object is required, and may be given more than once. A library \
                            artifact with no shared object in it is not a library."
                    .to_string());
            }
            false => objects,
        },
        needs,
        provides_class,
        addon_soname,
        signing_key: match signing_key {
            Some(named) => named,
            None => crate::resolve_signing_key()?,
        },
        target,
        out_dir,
        public_key_out,
    })
}

pub fn run(args: &Args) -> Result<String, String> {
    let arch = name::arch_for(&args.target).ok_or_else(|| {
        format!(
            "{} is not a target this tool knows an <arch> token for (ADR-027 §3.1)",
            args.target
        )
    })?;

    let mut report = String::new();
    let mut sonames = Vec::new();
    let mut parts_from_objects = Vec::new();

    for path in &args.objects {
        let bytes = std::fs::read(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let dynamic = elf::read(&bytes)
            .map_err(|fault| format!("{} is not a shared object this tool can read: {fault}", path.display()))?;

        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("{} has no usable file name", path.display()))?;

        let soname = dynamic.soname.as_deref().ok_or_else(|| {
            format!(
                "{} carries no DT_SONAME, so nothing can ever name it.\n\
                 \n\
                 A loader finds a library BY its SONAME: an application's DT_NEEDED holds a\n\
                 name, and the loader opens a file called that. An object with no SONAME\n\
                 would install correctly, sit in the right directory, and never be found —\n\
                 which reaches you as `library not found` about a file that is plainly there.\n\
                 \n\
                 Link it with -Wl,-soname,{file_name} and package it again.",
                path.display()
            )
        })?;

        if soname != file_name {
            return Err(format!(
                "{} calls itself {soname:?} and its file is named {file_name:?}.\n\
                 \n\
                 Those must be the same string. The file name is what this artifact installs\n\
                 under and the SONAME is what a loader asks for, so a difference is two names\n\
                 for one object that agree today and disagree the first time somebody renames\n\
                 the file. Rename it to {soname:?}, or link it with -Wl,-soname,{file_name}.",
                path.display()
            ));
        }

        // A shared object has no PT_INTERP — it is not something the kernel
        // starts. An executable given here is a build mistake, and one that
        // would otherwise be found by a loader on a board.
        if !dynamic.is_static() {
            return Err(format!(
                "{} names an interpreter ({}), which makes it an executable rather than a \
                 shared object. A library artifact carries objects the loader maps, not \
                 programs the kernel starts.",
                path.display(),
                dynamic.interpreter.unwrap_or_default()
            ));
        }

        report.push_str(&format!(
            "object   {} — soname {soname}, needs [{}]\n",
            path.display(),
            dynamic.needed.join(", ")
        ));
        sonames.push(soname.to_string());
        parts_from_objects.push((file_name.to_string(), path.clone()));
    }

    let manifest = library::Manifest::new(sonames.clone())
        .and_then(|m| m.with_needs(args.needs.clone()))
        .and_then(|m| {
            m.with_addon(args.provides_class.clone(), args.addon_soname.clone())
        })
        // **The refusals `.rulings/` R-201 added arrive here**, and they are
        // the reason this verb builds the manifest before it signs anything:
        // the alternative to saying which object is the addon at the moment
        // the artifact is emitted is an installer on a board saying it, about
        // bytes somebody has already published.
        .map_err(|fault| format!("this library's manifest is not one: {fault}"))?;
    let manifest_text = manifest.render();

    let seed = key::read_32(&args.signing_key, "signing key")?;
    let mut signer = Ed25519Signer::from_seed(&seed);
    let mut digest = Blake3::new();

    let variant = rivet_core::package::application::VARIANT_NONE;
    let mut parts = vec![emit::Part {
        name: MANIFEST_NAME.to_string(),
        class: TargetClass::Manifest.as_byte(),
        variant,
        source: emit::Source::Bytes(manifest_text.clone().into_bytes()),
    }];
    for (member_name, path) in parts_from_objects {
        parts.push(emit::Part {
            name: member_name,
            class: TargetClass::Object.as_byte(),
            variant,
            source: emit::Source::File(path),
        });
    }

    let out_dir = args
        .out_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("target").join("rivet"));
    let out = out_dir.join(name::artifact(
        &args.name,
        &args.version,
        arch,
        None,
        &Kind::library(),
    ));

    let written = emit::write(
        &out,
        &emit::Identity {
            product: &args.name,
            version: &args.version,
            arch,
            platform: None,
            kind: Kind::library(),
        },
        &parts,
        &[],
        &mut digest,
        &mut signer,
    )?;

    // **The reference is read back out of the file that was just written**,
    // rather than computed from the parts that went into it. The board will do
    // exactly this — hash the header it received — so computing it any other
    // way here would be a second derivation of one number, and the two could
    // differ without anything noticing until a closure failed to resolve.
    let reference = reference_of(&written.path, &args.name, &args.version)?;

    let public = public_key_for(&seed);
    report.push_str(&format!(
        "wrote    {}\n\
         product  {}\n\
         version  {}\n\
         arch     {}\n\
         kind     {}\n\
         size     {} bytes\n\
         soname   {}\n\
         needs    {}\n\
         class    {}\n\
         addon    {}\n\
         key      {}\n",
        written.path.display(),
        args.name,
        args.version,
        arch,
        Kind::library(),
        written.total_len,
        sonames.join(", "),
        match args.needs.is_empty() {
            true => "nothing".to_string(),
            false => args
                .needs
                .iter()
                .map(|r| r.to_string())
                .collect::<Vec<_>>()
                .join(", "),
        },
        args.provides_class.as_deref().unwrap_or("none — joins no application by class"),
        // **Resolved, not echoed.** What is printed is what a supervisor will
        // hand a host: the named object, or the single one a library with
        // nothing to choose between has. An author who gave no
        // `--addon-soname` sees which object they in fact shipped.
        manifest
            .addon_object()
            .unwrap_or("none — this library provides no addon class"),
        key::hex(&public.0),
    ));

    if let Some(path) = &args.public_key_out {
        std::fs::write(path, public.0)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        report.push_str(&format!("public   {} (32 raw bytes)\n", path.display()));
    }

    report.push_str(&format!(
        "\nreference\n  {reference}\n\
         \n\
         That string is three things at once and deliberately only written down once:\n\
         the closure element an application declares, the directory this installs to on a\n\
         board, and a digest of this exact artifact. Put it in the application's\n\
         Cargo.toml:\n\
         \n\
         \x20 [package.metadata.rivet]\n\
         \x20 library = [\"{reference}\"]\n\
         \n\
         Install this on a board BEFORE the application that declares it. The installer\n\
         refuses an application whose closure it cannot satisfy, and names what is\n\
         missing — a refusal at install rather than a loader error at start.\n"
    ));

    Ok(report)
}

/// The reference of an artifact on disk: its name, its version, and BLAKE3 over
/// the signed region of its header.
pub fn reference_of(path: &Path, product: &str, version: &str) -> Result<Reference, String> {
    use rivet_core::package::format::HEADER_BYTES;
    use std::io::Read;

    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let mut head = vec![0u8; HEADER_BYTES];
    file.read_exact(&mut head)
        .map_err(|e| format!("cannot read the header of {}: {e}", path.display()))?;

    let mut digest = Blake3::new();
    let identity = library::identity(&head, &mut digest)
        .ok_or_else(|| format!("{} has no readable header", path.display()))?;
    Reference::compose(product, version, &identity)
        .map_err(|fault| format!("this artifact cannot be referenced: {fault}"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_reference_given_to_needs_must_be_one() {
        let refused = parse(&[
            "--name".into(),
            "libgreet".into(),
            "--version".into(),
            "1.0.0".into(),
            "--object".into(),
            "libgreet.so.1".into(),
            "--needs".into(),
            "libz-1.3.1".into(),
        ]);
        let message = refused.expect_err("a bare name was accepted as a reference");
        assert!(message.contains("is not a library reference"), "{message}");
    }

    /// The verb cannot emit an artifact with nothing in it, and the refusal says
    /// what a library is rather than naming a missing flag.
    #[test]
    fn a_library_with_no_object_is_refused() {
        let refused = parse(&[
            "--name".into(),
            "libgreet".into(),
            "--version".into(),
            "1.0.0".into(),
        ]);
        let message = refused.expect_err("a library with no object was accepted");
        assert!(message.contains("--object is required"), "{message}");
    }

    // ---- `--provides-class` — PLAN-Feature-Set-2026-09-12 §6.2, ADR-053 ----

    /// **This is the write half of `ADR-053`**: `cargo rivet library` must be
    /// able to take the flag and carry it into [`Args`]. `--signing-key` is
    /// given an arbitrary value here because `parse` never opens it — that
    /// happens in [`run`], which this test does not reach.
    #[test]
    fn provides_class_is_parsed_into_args() {
        let args = parse(&[
            "--name".into(),
            "libtemp".into(),
            "--version".into(),
            "1.0.0".into(),
            "--object".into(),
            "libtemp.so.1".into(),
            "--provides-class".into(),
            "temperature-sensor".into(),
            "--signing-key".into(),
            "unread-by-parse".into(),
        ])
        .expect("valid args");
        assert_eq!(args.provides_class.as_deref(), Some("temperature-sensor"));
    }

    /// Absence is TAKES NO ADDON, which is what a library emitted before
    /// `ADR-053` means without this flag — `parse` must not require it.
    #[test]
    fn provides_class_is_optional_and_defaults_to_none() {
        let args = parse(&[
            "--name".into(),
            "libtemp".into(),
            "--version".into(),
            "1.0.0".into(),
            "--object".into(),
            "libtemp.so.1".into(),
            "--signing-key".into(),
            "unread-by-parse".into(),
        ])
        .expect("valid args");
        assert_eq!(args.provides_class, None);
    }

    // ---- `--addon-soname` — `.rulings/` R-201 ----------------------------

    /// The write half of R-201: the flag reaches [`Args`] so that [`run`] can
    /// put it in the manifest.
    #[test]
    fn an_addon_soname_is_parsed_into_args() {
        let args = parse(&[
            "--name".into(),
            "libpack".into(),
            "--version".into(),
            "1.0.0".into(),
            "--object".into(),
            "libpack-private.so.1".into(),
            "--object".into(),
            "libpack.so.1".into(),
            "--provides-class".into(),
            "aura-background-pack".into(),
            "--addon-soname".into(),
            "libpack.so.1".into(),
            "--signing-key".into(),
            "unread-by-parse".into(),
        ])
        .expect("valid args");
        assert_eq!(args.addon_soname.as_deref(), Some("libpack.so.1"));
    }

    /// **Absence is the ordinary case**, and it must stay so: every addon
    /// this project ships carries one object and names none.
    #[test]
    fn an_addon_soname_is_optional_and_defaults_to_none() {
        let args = parse(&[
            "--name".into(),
            "libtemp".into(),
            "--version".into(),
            "1.0.0".into(),
            "--object".into(),
            "libtemp.so.1".into(),
            "--provides-class".into(),
            "temperature-sensor".into(),
            "--signing-key".into(),
            "unread-by-parse".into(),
        ])
        .expect("valid args");
        assert_eq!(args.addon_soname, None);
    }
}
