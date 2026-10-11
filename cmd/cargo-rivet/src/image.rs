//! `cargo rivet image` — wrap an already-built card image as a signed
//! **`card`** `.rvt`, publish it into the fleet repository, and leave the
//! signed file beside the image. `R-177` D1, `R-177.1`.
//!
//! # What this closes
//!
//! `ADR-052` §7.2.3.5 used to say the fleet repository *"holds no card
//! images … recorded here as intent"*. `R-176`'s whole finding was that
//! provisioning cannot be a consumer of a repository nothing ever produces
//! into — *"the missing piece is a verb, not a concept"* — and this is that
//! verb.
//!
//! # It never builds — `R-177` D2
//!
//! `tools/container/build-card.sh` (and `cmd/rivet-image` beneath it) is the
//! only builder of a card image, exactly as `cargo rivet update` never builds
//! a system image (`ADR-027` §9.4 item 3, R-173's "the orchestrator stays a
//! writer"). Given no `--img`, this looks in `out/` for the newest
//! `*-card.img` — the same convention `cmd/rivet-image` and
//! `write_image::candidates` already use — and refuses by name, pointing at
//! the build script, when there is none.
//!
//! # What gets wrapped, and how it is named
//!
//! One opaque member holding the whole `.img`, zstd-compressed
//! (`rivet_core::package::read::COMPRESSION_ZSTD`, R-177.1). There is no
//! manifest and no `rivet_core::package::card` module — a card carries
//! nothing this container's own generic fields do not already say
//! (`cmd/cargo-rivet/src/inspect.rs`'s `Details::Card`).
//!
//! `<product>-<version>-<arch>-<platform>-card.rvt`, composed by
//! `rivet_remote::name::artifact` exactly as an update payload's name is —
//! `ADR-027` §7: the tool composes the name, and there is no override.
//!
//! # Platform and schema version are READ, never typed — `R-177` items 2 and 3
//!
//! `--platform` defaults to [`rivet_adapter_image::PiFirmware::platform_token`]
//! and is refused if it names a token with no `BootSlots` implementor — the
//! same posture `cargo rivet update` already takes (`update::slots_for`,
//! reused rather than re-derived), so a typo cannot mint a platform this
//! build does not implement.
//!
//! The schema version is read out of the image's own template `rivet.toml`,
//! on its boot partition (`write_image::read_boot_toml`) — never the value
//! this build of `cargo-rivet` happens to be compiled against, because this
//! tool did not compose those bytes and asserting a fact about somebody
//! else's document is exactly `ADR-027` §9.4 item 3's rule about an update's
//! version, applied one field over.
//!
//! # Compression streams — Wave B item 5
//!
//! The `.img` is read once, a chunk at a time, through a zstd encoder writing
//! to a temporary file beside the destination; the running BLAKE3 digest is
//! taken over the bytes actually written (the **stored**, compressed bytes —
//! `ADR-029` §5.1). Peak memory for this pass is one encoder buffer, not the
//! 2 GiB input or its compressed copy. The temporary file is then streamed
//! into the final `.rvt`, after the signed header, and removed.
//!
//! # Entitlement — `R-177` item 6
//!
//! Signed with the **system** seed (`crate::credentials::system_seed`,
//! sibling to `app_seed`), never the application one — `ADR-028` §5.5.2's
//! mask puts `card` on `rivet-system` for the same reason `update` and
//! `extension` are there: the key that ships Rivet's own images must be a
//! different key from the one that ships somebody's application.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use rivet_adapter_image::{BootSlots, PiFirmware};
use rivet_core::package::format::DIGEST_BYTES;
use rivet_core::package::kind::Kind;
use rivet_core::package::port::{Digest, KeyId, Sign};
use rivet_core::package::read::COMPRESSION_ZSTD;
use rivet_core::package::update::PRODUCT;
use rivet_core::package::write::{Builder, NewMember};
use rivet_crypto::{Blake3, Ed25519Signer};

use crate::credentials::{self, Overrides};
use crate::key;
use crate::progress::Stages;

/// The one member every `card` payload has.
const IMAGE_MEMBER_NAME: &str = "image";

/// Bytes moved per read/write — matched to `emit.rs`, `inspect.rs` and
/// `write_image.rs`'s own constant so a multi-gigabyte image is never held in
/// memory at once.
const CHUNK: usize = 1024 * 1024;

/// zstd's own default level. **Chosen for speed, not ratio** — R-177.1's
/// whole argument for zstd over xz is *"much faster to compress and
/// decompress 2 GiB, within a few percent of xz's size"*, and a higher level
/// spends exactly the time that argument was made to save. Measured, not
/// assumed — see the end-to-end report this change ships with.
const ZSTD_LEVEL: i32 = 3;

pub struct ImageArgs {
    pub img: Option<PathBuf>,
    pub version: Option<String>,
    pub arch: Option<String>,
    pub platform: Option<String>,
    pub product: String,
    pub signing_key: Option<PathBuf>,
    pub out_dir: Option<PathBuf>,
    pub at: Option<PathBuf>,
    pub public_key_out: Option<PathBuf>,
}

pub fn usage() -> String {
    format!(
        "usage: cargo rivet image [--img PATH] --version V --arch TOKEN\n\
         \x20                          [--platform TOKEN] [--signing-key PATH]\n\
         \x20                          [--out-dir DIR] [--at DIR] [--product NAME]\n\
         \n\
         `R-177`: wraps an ALREADY-BUILT card image as a signed `card` .rvt,\n\
         publishes it into the fleet repository (ADR-052 §7), and leaves the\n\
         signed file beside the image. It never builds one — that is\n\
         tools/container/build-card.sh, exactly as `cargo rivet update` never\n\
         builds a system image.\n\
         \n\
         image options:\n\
         \x20 --img PATH          the card image tools/container/build-card.sh\n\
         \x20                     wrote. Default: the newest `*-card.img` in\n\
         \x20                     `out/`, same convention `cargo rivet\n\
         \x20                     write-image` uses\n\
         \x20 --version VERSION   REQUIRED. This tool did not build these bytes,\n\
         \x20                     so it cannot compose a version that claims a\n\
         \x20                     commit (ADR-027 §3.2.1). Pass what\n\
         \x20                     tools/card/build.sh was given\n\
         \x20 --arch TOKEN        REQUIRED — ADR-027 §3.1's token for the target\n\
         \x20                     every binary in the image was built for\n\
         \x20 --platform TOKEN    default {}, and it is the only token with a\n\
         \x20                     BootSlots implementor. ADR-027 §3.1 also lists\n\
         \x20                     rpizw and uefi; neither has one yet\n\
         \x20 --product NAME      default {PRODUCT}\n\
         \x20 --signing-key PATH  the SYSTEM seed. Optional: without it,\n\
         \x20                     ${} then ~/.rivet/keys/{}\n\
         \x20                     — deliberately NOT the application seed\n\
         \x20                     (ADR-028 §5.5.2): rivet-system is the only key\n\
         \x20                     entitled to sign a `card`\n\
         \x20 --out-dir DIR       where the .rvt goes. Default: beside --img\n\
         \x20 --at DIR            the fleet repository to publish into. Default:\n\
         \x20                     ~/.rivet/fleet, same as `fleet` and `deploy`\n\
         \x20 --public-key-out PATH write the PUBLIC key here, 32 raw bytes\n\
         \n\
         The name is composed and there is no override — ADR-027 §7. Compressed\n\
         with zstd (R-177.1) — the one workstation-only member this format's\n\
         codec byte legally carries; every other kind stays uncompressed on\n\
         purpose (ADR-029 §13/R-107).\n",
        PiFirmware.platform_token(),
        credentials::SYSTEM_SEED_ENV,
        credentials::SYSTEM_SEED_FILE,
    )
}

pub fn parse(args: &[String]) -> Result<ImageArgs, String> {
    let mut img: Option<PathBuf> = None;
    let mut version: Option<String> = None;
    let mut arch: Option<String> = None;
    let mut platform: Option<String> = None;
    let mut product = PRODUCT.to_string();
    let mut signing_key: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut at: Option<PathBuf> = None;
    let mut public_key_out: Option<PathBuf> = None;

    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = || {
            args.get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag {
            "--img" => img = Some(PathBuf::from(value()?)),
            "--version" => version = Some(value()?),
            "--arch" => arch = Some(value()?),
            "--platform" => platform = Some(value()?),
            "--product" => product = value()?,
            "--signing-key" => signing_key = Some(PathBuf::from(value()?)),
            "--out-dir" => out_dir = Some(PathBuf::from(value()?)),
            "--at" => at = Some(PathBuf::from(value()?)),
            "--public-key-out" => public_key_out = Some(PathBuf::from(value()?)),
            other => return Err(format!("{other:?} is not an option.\n\n{}", usage())),
        }
        index += 2;
    }

    Ok(ImageArgs {
        img,
        version,
        arch,
        platform,
        product,
        signing_key,
        out_dir,
        at,
        public_key_out,
    })
}

/// The `BootSlots` implementor a `<platform>` token names — `update.rs`'s own
/// function, reused rather than re-derived (`ADR-011`).
fn slots_for(token: &str) -> Result<&'static dyn BootSlots, String> {
    crate::update::slots_for(token)
}

/// The newest `*-card.img` in `out/`, or the refusal naming the build script.
fn discover_image() -> Result<PathBuf, String> {
    let found = crate::write_image::candidates("out", "-card.img");
    found.into_iter().next().ok_or_else(|| {
        "no --img was given and no `*-card.img` was found in `out/`. `cargo rivet image` NEVER \
         BUILDS one (R-177 D2) — build a card first with tools/container/build-card.sh, then \
         pass --img, or run this from a tree where that script's output is in `out/`."
            .to_string()
    })
}

/// The image's own declared schema version, read out of its boot partition's
/// template `rivet.toml` — never this build's compiled-in one (`R-177` item
/// 3).
///
/// # Errors
///
/// The boot partition could not be read at all, it carries no document, or
/// the document has no readable integer `version` key.
fn schema_version_of(image: &Path) -> Result<u32, String> {
    let bytes = crate::write_image::read_boot_toml(image)?.ok_or_else(|| {
        format!(
            "{} carries no rivet.toml on its boot partition, so this tool cannot read the \
             schema version it was built against. Every card `cmd/rivet-image` builds carries \
             at least a template declaring one (COMMANDS.md); an image missing even that is not \
             one this build recognises.",
            image.display()
        )
    })?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|e| format!("{}'s rivet.toml is not UTF-8: {e}", image.display()))?;
    let doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|e| format!("{}'s rivet.toml does not parse: {e}", image.display()))?;
    let raw = doc
        .get("version")
        .and_then(toml_edit::Item::as_integer)
        .ok_or_else(|| {
            format!(
                "{}'s rivet.toml has no integer `version` key — every schema-versioned document \
                 must have one (rivet_config::SCHEMA_VERSION).",
                image.display()
            )
        })?;
    u32::try_from(raw).map_err(|_| {
        format!("{}'s rivet.toml declares version {raw}, which is not a schema version (versions count up from 1)", image.display())
    })
}

/// **Where the system signing seed is**, refused naming both places looked —
/// `credentials::system_seed`'s own two-place lookup, worded for this verb the
/// way `fleet::deploy_app_seed` words it for `deploy`.
fn system_seed_path(named: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = named {
        return Ok(path.to_path_buf());
    }
    let over = Overrides::from_environment();
    let rivet_root = credentials::root();
    credentials::system_seed(&over, rivet_root.as_deref()).map_err(|looked_at| {
        let default = looked_at.map_or_else(
            || "(no HOME or USERPROFILE, so no default path)".to_string(),
            |p| p.display().to_string(),
        );
        format!(
            "NOTHING WAS SIGNED. `cargo rivet image` signs a `card` with the SYSTEM signing \
             seed (ADR-028 §5.5.2) — never the application one — and none was found. Two places \
             were looked at, in this order:\n\
             \x20 ${env}   not set\n\
             \x20 {default}   not a file\n\
             \n\
             Set ${env} to the seed that signs Rivet's own images, or put it at the second \
             path. `cargo rivet key new --out PATH` mints one.",
            env = credentials::SYSTEM_SEED_ENV,
        )
    })
}

/// A [`Write`] that both forwards to `inner` and accumulates a running BLAKE3
/// digest and length over exactly what was written — the **stored** bytes, so
/// this is the digest `ADR-029` §5.1 wants regardless of what codec produced
/// them.
struct Digesting<'d, W> {
    inner: W,
    digest: &'d mut dyn Digest,
    len: u64,
}

impl<W: Write> Write for Digesting<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.digest.write(&buf[..n]);
        self.len += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Compress `img` to `dest` with zstd, streaming — never holding the plain
/// image or its compressed copy resident. Returns the compressed length and
/// the BLAKE3 digest of the compressed bytes.
///
/// # Errors
///
/// Any I/O failure reading `img` or writing `dest`.
fn compress_streaming(
    img: &Path,
    dest: &Path,
    digest: &mut dyn Digest,
) -> Result<(u64, [u8; DIGEST_BYTES]), String> {
    digest.start();
    let input = File::open(img).map_err(|e| format!("cannot read {}: {e}", img.display()))?;
    let mut reader = BufReader::with_capacity(CHUNK, input);
    let output = File::create(dest).map_err(|e| format!("cannot write {}: {e}", dest.display()))?;
    let counting = Digesting {
        inner: BufWriter::with_capacity(CHUNK, output),
        digest,
        len: 0,
    };

    let mut encoder = zstd::stream::write::Encoder::new(counting, ZSTD_LEVEL)
        .map_err(|e| format!("cannot start the zstd encoder: {e}"))?;
    std::io::copy(&mut reader, &mut encoder)
        .map_err(|e| format!("cannot compress {}: {e}", img.display()))?;
    let mut counting = encoder
        .finish()
        .map_err(|e| format!("cannot finish compressing {}: {e}", img.display()))?;
    counting
        .flush()
        .map_err(|e| format!("cannot flush {}: {e}", dest.display()))?;
    let stored_len = counting.len;

    Ok((stored_len, digest.finish()))
}

pub fn run(args: &[String]) -> Result<String, String> {
    let args = parse(args)?;
    let version = args.version.clone().ok_or(
        "--version is required. This tool did not build the image it is handed, so composing a \
         version here would make ADR-027 §3.2.1's claim about a commit on somebody else's \
         behalf. Pass what tools/card/build.sh was given.",
    )?;
    let arch = args.arch.clone().ok_or(
        "--arch is required: ADR-027 §3.1's token for the target the image's binaries were \
         built for.",
    )?;

    let stages = Stages::new(5);

    let stage = stages.start("locating the card image");
    let image = match &args.img {
        Some(p) => p.clone(),
        None => match discover_image() {
            Ok(p) => p,
            Err(e) => {
                stage.failed(&e);
                return Err(e);
            }
        },
    };
    if !image.is_file() {
        let e = format!("{} is not a file", image.display());
        stage.failed(&e);
        return Err(e);
    }
    let plain_len = std::fs::metadata(&image)
        .map_err(|e| format!("cannot stat {}: {e}", image.display()))?
        .len();
    stage.done();

    let stage = stages.start("reading the image's platform and schema version");
    let platform_token = args
        .platform
        .clone()
        .unwrap_or_else(|| PiFirmware.platform_token().to_string());
    let slots = match slots_for(&platform_token) {
        Ok(s) => s,
        Err(e) => {
            stage.failed(&e);
            return Err(e);
        }
    };
    let schema_version = match schema_version_of(&image) {
        Ok(v) => v,
        Err(e) => {
            stage.failed(&e);
            return Err(e);
        }
    };
    stage.done();

    let seed_path = system_seed_path(args.signing_key.as_deref())?;
    let seed = key::read_32(&seed_path, "system signing seed")?;
    let mut signer = Ed25519Signer::from_seed(&seed);
    let mut digest = Blake3::new();

    let out_dir = args
        .out_dir
        .clone()
        .or_else(|| image.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| format!("cannot create {}: {e}", out_dir.display()))?;
    let out_name = rivet_remote::name::artifact(
        &args.product,
        &version,
        &arch,
        Some(slots.platform_token()),
        &Kind::card(),
    );
    let out_path = out_dir.join(&out_name);
    let temp_path = out_dir.join(format!(".{out_name}.zstd-tmp"));

    let stage = stages.start(&format!("compressing {} with zstd", image.display()));
    let (stored_len, member_digest) = match compress_streaming(&image, &temp_path, &mut digest) {
        Ok(pair) => pair,
        Err(e) => {
            let _ = std::fs::remove_file(&temp_path);
            stage.failed(&e);
            return Err(e);
        }
    };
    stage.done();

    let stage = stages.start("signing and sealing the header");
    let mut builder = match Builder::new(
        &args.product,
        &version,
        &arch,
        Some(slots.platform_token()),
        Kind::card(),
    ) {
        Ok(b) => b,
        Err(e) => {
            let _ = std::fs::remove_file(&temp_path);
            let e = format!("this artifact cannot be named: {e}");
            stage.failed(&e);
            return Err(e);
        }
    };
    let member = match NewMember::streamed(
        IMAGE_MEMBER_NAME,
        0,
        0,
        COMPRESSION_ZSTD,
        member_digest,
        stored_len,
        plain_len,
    ) {
        Ok(m) => m,
        Err(e) => {
            let _ = std::fs::remove_file(&temp_path);
            let e = format!("the image cannot be a member: {e}");
            stage.failed(&e);
            return Err(e);
        }
    };
    if let Err(e) = builder.add(member) {
        let _ = std::fs::remove_file(&temp_path);
        let e = format!("the image cannot be added: {e}");
        stage.failed(&e);
        return Err(e);
    }
    let header = match builder.seal(&mut digest, &mut signer) {
        Ok(h) => h,
        Err(e) => {
            let _ = std::fs::remove_file(&temp_path);
            let e = format!("the header cannot be sealed: {e}");
            stage.failed(&e);
            return Err(e);
        }
    };
    stage.done();

    let stage = stages.start(&format!("writing {}", out_path.display()));
    if let Err(e) = write_final(&out_path, &header, &temp_path) {
        let _ = std::fs::remove_file(&temp_path);
        stage.failed(&e);
        return Err(e);
    }
    let _ = std::fs::remove_file(&temp_path);
    stage.done();

    let public = signer.public_key();
    let key_id = KeyId(digest.of(&public.0));
    let mut report = String::new();
    if let Some(path) = &args.public_key_out {
        std::fs::write(path, public.0)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        report.push_str(&format!("public   {} (32 raw bytes)\n", path.display()));
    }

    let ratio = if plain_len == 0 {
        0.0
    } else {
        stored_len as f64 / plain_len as f64 * 100.0
    };
    report.push_str(&format!(
        "wrote    {}\n\
         product  {}\n\
         version  {}  (taken as given: this tool did not build the image)\n\
         arch     {}\n\
         platform {}  ({} — from the boot port, ADR-027 §3.1)\n\
         kind     {}\n\
         schema   {schema_version}  (read from the image's own rivet.toml, R-177 item 3)\n\
         plain    {plain_len} bytes\n\
         stored   {stored_len} bytes  (zstd, {ratio:.1}% of plain — R-177.1)\n\
         key      {} (id {key_id})\n",
        out_path.display(),
        args.product,
        version,
        arch,
        slots.platform_token(),
        slots.platform(),
        Kind::card(),
        key::hex(&public.0),
    ));

    let stage = stages.start("publishing into the fleet repository");
    let root = match crate::fleet::deploy_root(args.at.as_deref()) {
        Ok(r) => r,
        Err(e) => {
            stage.failed(&e);
            return Err(e);
        }
    };
    if !root.join(".git").is_dir() {
        if let Err(e) = crate::fleet::init(&root) {
            let e = format!(
                "the fleet repository at {} could not be made: {e}",
                root.display()
            );
            stage.failed(&e);
            return Err(e);
        }
        report.push_str(&format!(
            "fleet    CREATED the fleet repository at {}\n",
            root.display()
        ));
    }
    let published =
        crate::fleet::publish_verified(&root, &out_path, Some(schema_version), &mut || Ok(public));
    let published = match published {
        Ok(p) => p,
        Err(e) => {
            stage.failed(&e);
            return Err(format!(
                "{report}\nTHE SIGNED .rvt IS AT {} — it was NOT published. {e}",
                out_path.display()
            ));
        }
    };
    stage.done();
    report.push_str(&published.report);

    report.push_str(&format!(
        "\n`cargo rivet fleet list` shows it; `cargo rivet fleet publish {} --trusted-key PATH` \
         accepts this exact file by hand, too.\n",
        out_path.display()
    ));

    Ok(report)
}

/// Header, then the compressed temp file's bytes, streamed — never resident.
fn write_final(out: &Path, header: &[u8], temp: &Path) -> Result<(), String> {
    let file = File::create(out).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    let mut sink = BufWriter::with_capacity(CHUNK, file);
    sink.write_all(header)
        .map_err(|e| format!("cannot write the header: {e}"))?;

    let mut source =
        File::open(temp).map_err(|e| format!("cannot reopen {}: {e}", temp.display()))?;
    source
        .seek(SeekFrom::Start(0))
        .map_err(|e| format!("cannot rewind {}: {e}", temp.display()))?;
    let mut buffer = vec![0u8; CHUNK];
    loop {
        let read = source
            .read(&mut buffer)
            .map_err(|e| format!("cannot read {}: {e}", temp.display()))?;
        if read == 0 {
            break;
        }
        sink.write_all(&buffer[..read])
            .map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    }
    sink.flush()
        .map_err(|e| format!("cannot flush {}: {e}", out.display()))?;
    let file = sink
        .into_inner()
        .map_err(|e| format!("cannot finish {}: {e}", out.display()))?;
    file.sync_all()
        .map_err(|e| format!("cannot sync {}: {e}", out.display()))?;
    Ok(())
}

#[cfg(test)]
#[path = "image_tests.rs"]
pub(crate) mod tests;
