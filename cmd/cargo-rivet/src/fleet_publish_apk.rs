//! `cargo rivet fleet publish FILE.apk` — `ADR-061` §4.
//!
//! An APK becomes an `apk` artifact (`ADR-061` §3) and is then published
//! exactly as any other artifact is, through
//! [`publish_verified`]: the
//! object stored under its digest, the entry appended, the repository's
//! retention applied, and only `manifest.toml` committed.
//!
//! Before anything is written, the APK is held to its product's history in
//! this repository, which is what Android itself will hold it to on a phone:
//!
//! 1. its version code must be **higher** than the product's newest, or
//!    Android will not install it as an update;
//! 2. its signing certificate must be the product's last one, **or** linked to
//!    it by the APK's own v3 rotation lineage, or Android will refuse the
//!    update's signature;
//! 3. a product's **first** publish has nothing to compare against, so its
//!    certificate is accepted and printed as the one the product is now pinned
//!    to — what Android does on a first install.
//!
//! The product's newest version code and last signer are read from the
//! `apk.json` of every `apk` object this repository holds for the product
//! (retention keeps three), each verified against the application key before
//! it is believed.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use rivet_core::artifact::{VersionOrder, version_order};
use rivet_core::package::Kind;
use rivet_core::package::apk::{APK_NAME, METADATA_NAME, Metadata};
use rivet_core::package::application::VARIANT_NONE;
use rivet_core::package::format::PRODUCT_BYTES;
use rivet_core::package::port::{PublicKey, Sign as _};
use rivet_crypto::{Blake3, Ed25519Signer, Ed25519Verifier};

use crate::apk_read::{self, ApkInfo};

/// The header fields every `apk` artifact carries — `ADR-061` §3.
const PLATFORM: &str = "android";
const ARCH: &str = "any";

/// The four bytes every zip, and so every APK, opens with.
const ZIP_MAGIC: [u8; 4] = *b"PK\x03\x04";

/// What `fleet publish` was handed.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Handed {
    /// Named `.apk` and opening with the zip magic.
    Apk,
    /// Anything else — the `.rvt` path, which reports its own refusals.
    Artifact,
}

/// Decide from the file's name **and** its first bytes. The two must agree:
/// an `.apk` that is not a zip, or a zip not named `.apk`, is refused rather
/// than read as whichever the caller seems to have meant.
///
/// # Errors
///
/// The two disagree, or a file named `.apk` cannot be read.
pub(super) fn classify(file: &Path) -> Result<Handed, String> {
    let named_apk = file
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("apk"));
    let mut head = [0u8; 4];
    let opens_as_zip = match std::fs::File::open(file) {
        Ok(mut opened) => {
            std::io::Read::read_exact(&mut opened, &mut head).is_ok() && head == ZIP_MAGIC
        }
        Err(e) if named_apk => return Err(format!("cannot read {}: {e}", file.display())),
        // Not named as an APK: the `.rvt` path reads it and names what is wrong.
        Err(_) => false,
    };
    match (named_apk, opens_as_zip) {
        (true, true) => Ok(Handed::Apk),
        (false, false) => Ok(Handed::Artifact),
        (true, false) => Err(format!(
            "{} is named .apk and is not one: an APK is a zip and opens with the bytes PK\\3\\4, \
             and this file does not. Nothing was published.",
            file.display()
        )),
        (false, true) => Err(format!(
            "{} is a zip — it opens with the bytes an APK does — and is not named .apk. \
             `fleet publish` takes an APK only by its .apk name, so that the file's name and its \
             contents say the same thing; rename it if it is an APK. Nothing was published.",
            file.display()
        )),
    }
}

/// The product an APK publishes under — `ADR-061` §4: the application id's
/// last dotted segment, with a trailing `.dev` becoming `-dev`, so
/// `….gantry.dev` is `gantry-dev` and `….logbook` is `logbook`.
#[must_use]
pub(super) fn derive_product(application_id: &str) -> String {
    match application_id.strip_suffix(".dev") {
        Some(stem) => format!("{}-dev", stem.rsplit('.').next().unwrap_or(stem)),
        None => application_id.rsplit('.').next().unwrap_or(application_id).to_string(),
    }
}

/// Whether `name` can be a product in `ADR-027`'s grammar: at most
/// [`PRODUCT_BYTES`] bytes of lowercase ASCII letters, digits and single
/// hyphens, neither starting nor ending with one, and read back as itself out
/// of an artifact file name composed with it — which also refuses a name the
/// name reader would split at a version-shaped token inside it.
///
/// # Errors
///
/// Why it cannot, in words that finish "the product name … ".
pub(super) fn check_product(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("is empty".to_string());
    }
    if name.len() > PRODUCT_BYTES {
        return Err(format!("is {} bytes, and a product is at most {PRODUCT_BYTES}", name.len()));
    }
    if let Some(bad) = name.chars().find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')) {
        return Err(format!(
            "holds {bad:?}, and a product is lowercase letters, digits and hyphens (ADR-027)"
        ));
    }
    if name.starts_with('-') || name.ends_with('-') || name.contains("--") {
        return Err("has a hyphen at an end or two together, which ADR-027's `-` between fields \
                    cannot tell from a field boundary"
            .to_string());
    }
    // Composed in an application's shape: the name reader knows no `any`
    // architecture, and where a product ends is decided the same way for
    // every kind — at the first token that opens a version.
    let composed =
        rivet_remote::name::artifact(name, "0.0.0", "aarch64", None, &Kind::application());
    match rivet_core::artifact::parse_name(&composed) {
        Some(read) if read.product == name => Ok(()),
        _ => Err(format!(
            "does not read back as itself from the artifact name it makes ({composed}) — it \
             holds a token the name reader takes for an architecture or a version"
        )),
    }
}

/// The application signing seed a publish signs with — `--signing-key`, then
/// `$RIVET_APP_SEED`, then the default path: [`crate::credentials::app_seed`],
/// the lookup `cargo rivet package` and `install` use.
///
/// # Errors
///
/// No seed was named and none was found; the refusal names every place looked.
pub(super) fn app_seed(
    named: Option<&Path>,
    over: &crate::credentials::Overrides,
    rivet_root: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(path) = named {
        return Ok(path.to_path_buf());
    }
    crate::credentials::app_seed(over, rivet_root).map_err(|looked_at| {
        let default = looked_at.map_or_else(
            || "(no HOME or USERPROFILE, so no default path)".to_string(),
            |p| p.display().to_string(),
        );
        format!(
            "NOTHING WAS PUBLISHED. An APK is wrapped as an `apk` artifact and signed with the \
             APPLICATION SIGNING SEED (ADR-061 §4), and none was found. Three places were looked \
             at, in this order:\n\
             \x20 --signing-key   not given\n\
             \x20 ${env}   not set\n\
             \x20 {default}   not a file\n\
             \n\
             `cargo rivet package` looks in the same places.",
            env = crate::APP_SEED_ENV,
        )
    })
}

/// Where the product stands in this repository before this publish.
struct Newest {
    version_code: i64,
    version_name: String,
    signer_sha256: String,
    /// The held artifact's digest, for the idempotent add.
    digest: String,
    /// BLAKE3 of the held `app.apk`, which is stored uncompressed, so this is
    /// the member digest the artifact's header already carries.
    app_apk: [u8; 32],
}

/// `cargo rivet fleet publish FILE.apk [--product NAME] [--signing-key PATH]`.
///
/// `seed` is asked only after the APK has been read and its product named,
/// so a file that is not an APK is refused before any key is looked for.
///
/// # Errors
///
/// Every refusal of `ADR-061` §4 — an APK that does not read, a product name
/// that cannot be one, a version code not higher than the product's newest, a
/// certificate neither the product's last one nor linked to it — and
/// [`super::publish_verified`]'s own.
pub(super) fn publish(
    root: &Path,
    file: &Path,
    product: Option<&str>,
    seed: &mut dyn FnMut() -> Result<PathBuf, String>,
) -> Result<String, String> {
    super::require_repository(root)?;

    let bytes = std::fs::read(file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    let info = apk_read::read(&bytes).map_err(|e| {
        format!("{} cannot be read as an APK: {e}. Nothing was published.", file.display())
    })?;

    // **`ADR-061` §4: a `versionName` must be one this repository's version
    // ordering can read** — dot-separated numbers (`ADR-027` §3), optionally
    // followed by `-` and build metadata. Retention's three-newest rule orders
    // by `versionName`, so one it cannot read would let retention keep the
    // wrong three. Checked before anything else about the APK, so a first
    // publish is refused exactly as a later one is.
    if matches!(version_order(&info.version_name), VersionOrder::Unreadable) {
        return Err(format!(
            "{} has versionName {:?}, and this repository's version ordering cannot read it: a \
             version is dot-separated numbers, optionally followed by `-` and build metadata \
             (ADR-027 §3). Nothing was published.",
            file.display(),
            info.version_name
        ));
    }

    let (product, product_source) = match product {
        Some(named) => {
            check_product(named).map_err(|why| {
                format!("--product {named:?} cannot be a product: it {why}. Nothing was published.")
            })?;
            (named.to_string(), "from --product".to_string())
        }
        None => {
            let derived = derive_product(&info.application_id);
            check_product(&derived).map_err(|why| {
                format!(
                    "the product name {derived:?}, derived from the application id {}, {why}. \
                     Name the product with --product NAME. Nothing was published.",
                    info.application_id
                )
            })?;
            (derived, format!("derived from {}", info.application_id))
        }
    };

    let seed_path = seed()?;
    let seed_bytes = crate::key::read_32(&seed_path, "application signing seed")?;
    let mut signer = Ed25519Signer::from_seed(&seed_bytes);
    let public = signer.public_key();

    let mut report = describe(file, &info, &product, &product_source);
    let signer_sha256 = info.signer_sha256();
    match newest(root, &product, public)? {
        None => {
            report.push_str(&format!("\n{product} is now pinned to signer {signer_sha256}\n"));
        }
        Some(newest) => {
            // **The idempotent add** (`ADR-052` §7): the newest held APK's exact
            // bytes again are already published, not a lower version. A retry
            // of a `--sync` that failed after its publish is this same command.
            let (this_apk, _) = super::blake3_of(file)?;
            if this_apk == newest.app_apk {
                report.push_str(&format!(
                    "\nthis repository already holds this APK, byte for byte, as {product} {} \
                     ({}); nothing new was made\n",
                    newest.version_name, newest.digest,
                ));
                return Ok(report);
            }
            if info.version_code <= newest.version_code {
                return Err(format!(
                    "{} has version code {} ({}), and {product}'s newest in this repository is {} \
                     ({}). A publish must carry a HIGHER version code than the newest: Android \
                     will not install a lower one as an update, and a phone holding the newest \
                     has no update to take from an equal one. Raise versionCode and build again. \
                     Nothing was published.",
                    file.display(),
                    info.version_code,
                    info.version_name,
                    newest.version_code,
                    newest.version_name,
                ));
            }
            // **`ADR-061` §4: versionCode and versionName must agree.**
            // Android compares versionCode; this repository's retention orders
            // by versionName. A versionCode higher than the newest's, whose
            // versionName does not order strictly above the newest's, would
            // let the two diverge, so it is refused here rather than trusted.
            if version_order(&info.version_name) <= version_order(&newest.version_name) {
                return Err(format!(
                    "{} has versionName {} (version code {}), and {product}'s newest in this \
                     repository is {} (version code {}). A higher version code must carry a \
                     versionName that orders STRICTLY ABOVE the one it replaces: Android compares \
                     versionCode, and this repository's three-newest retention orders by \
                     versionName, so the two must agree. Nothing was published.",
                    file.display(),
                    info.version_name,
                    info.version_code,
                    newest.version_name,
                    newest.version_code,
                ));
            }
            if signer_sha256 == newest.signer_sha256 {
                report.push_str(&format!(
                    "\nsigner matches {product}'s pinned signer {signer_sha256}\n"
                ));
            } else if info.lineage_sha256().contains(&newest.signer_sha256) {
                report.push_str(&format!(
                    "\nsigner {signer_sha256} is linked to {product}'s pinned signer {} by the \
                     APK's rotation lineage; {product} is now pinned to {signer_sha256}\n",
                    newest.signer_sha256,
                ));
            } else {
                return Err(format!(
                    "{} is signed with certificate {signer_sha256}, and {product} is pinned to \
                     {}. {} Android refuses an update signed by another key, so a phone holding \
                     {product} could never take this one. Sign it with the pinned key, or rotate \
                     to the new one with `apksigner rotate` and a lineage. Nothing was published.",
                    file.display(),
                    newest.signer_sha256,
                    if info.lineage.is_empty() {
                        "The APK carries no rotation lineage linking the two."
                    } else {
                        "The APK's rotation lineage does not include the pinned certificate."
                    },
                ));
            }
        }
    }

    let metadata = apk_json(&info);
    Metadata::parse(metadata.as_bytes()).map_err(|fault| {
        format!(
            "the apk.json this APK makes would not read back ({fault}); nothing was published. \
             The value at fault was read from the APK's own manifest."
        )
    })?;

    let staging = Staging::new()?;
    let out = staging.0.join(rivet_remote::name::artifact(
        &product,
        &info.version_name,
        ARCH,
        Some(PLATFORM),
        &Kind::apk(),
    ));
    crate::emit::write(
        &out,
        &crate::emit::Identity {
            product: &product,
            version: &info.version_name,
            arch: ARCH,
            platform: Some(PLATFORM),
            kind: Kind::apk(),
        },
        &[
            crate::emit::Part {
                name: APK_NAME.to_string(),
                class: 0,
                variant: VARIANT_NONE,
                // The bytes that were read, not the file again: what was
                // checked above is exactly what is wrapped.
                source: crate::emit::Source::Bytes(bytes),
            },
            crate::emit::Part {
                name: METADATA_NAME.to_string(),
                class: 0,
                variant: VARIANT_NONE,
                source: crate::emit::Source::Bytes(metadata.into_bytes()),
            },
        ],
        &[],
        &mut Blake3::new(),
        &mut signer,
    )?;

    let published = super::publish_verified(root, &out, None, &mut || Ok(public))?;
    report.push('\n');
    report.push_str(&published.report);
    Ok(report)
}

/// What was read out of the APK, for the report.
fn describe(file: &Path, info: &ApkInfo, product: &str, product_source: &str) -> String {
    let row = |name: &str, value: &str| format!("  {name:<16}{value}\n");
    let mut out = format!("APK   {}\n", file.display());
    out.push_str(&row("application_id", &info.application_id));
    out.push_str(&row(
        "version",
        &format!("{} (version code {})", info.version_name, info.version_code),
    ));
    if let Some(major) = info.version_code_major {
        out.push_str(&row(
            "versionCodeMajor",
            &format!("{major}: the version code compared is {major} << 32 | versionCode"),
        ));
    }
    out.push_str(&row("label", &info.label));
    out.push_str(&row("sdk", &format!("min {}, target {}", info.min_sdk, info.target_sdk)));
    out.push_str(&row(
        "signer",
        &format!("{} ({} signature)", info.signer_sha256(), info.scheme),
    ));
    if !info.lineage.is_empty() {
        out.push_str(&row("lineage", &info.lineage_sha256().join(" -> ")));
    }
    out.push_str(&row("product", &format!("{product} ({product_source})")));
    out
}

/// **The signing-key pin `product` holds in this repository** — its newest
/// held version's name and signer, read as a publish reads them — or `None`
/// when it holds no `apk` of that name. `fleet retire` prints what it drops.
///
/// # Errors
///
/// [`newest`]'s.
pub(super) fn pinned_signer(root: &Path, product: &str, trusted: PublicKey) -> Result<Option<(String, String)>, String> {
    Ok(newest(root, product, trusted)?.map(|n| (n.version_name, n.signer_sha256)))
}

/// The newest `apk` of `product` this repository holds, by version code, read
/// from each stored object's own `apk.json` after verifying it against the
/// application key. `None` when the repository holds none.
fn newest(root: &Path, product: &str, trusted: PublicKey) -> Result<Option<Newest>, String> {
    let apk_kind = Kind::apk();
    let mut best: Option<Newest> = None;
    for entry in super::read_manifest(root)?
        .iter()
        .filter(|e| e.product == product && e.kind == apk_kind.as_str())
    {
        let object = root.join(super::OBJECTS_DIR).join(&entry.digest);
        if !object.is_file() {
            return Err(format!(
                "the manifest names {product} {} ({}), and objects/ does not hold it, so its \
                 version code and signer cannot be read to compare this APK against. `fleet \
                 objects restore` puts objects back. Nothing was published.",
                entry.version, entry.digest,
            ));
        }
        let inspected = crate::inspect::inspect(&object, trusted, &[], &mut Blake3::new(), &Ed25519Verifier::new())
            .map_err(|e| {
                format!(
                    "{product} {} ({}) in this repository does not verify against the application \
                     key this publish signs with: {e}. Its version code and signer cannot be \
                     believed, so nothing was published.",
                    entry.version, entry.digest,
                )
            })?;
        let crate::inspect::Details::Apk { metadata, .. } = inspected.details else {
            return Err(format!(
                "the manifest records {} as an apk and its header says otherwise. Nothing was \
                 published.",
                entry.digest
            ));
        };
        if best.as_ref().is_none_or(|b| metadata.version_code() > b.version_code) {
            let app_apk = *inspected
                .package
                .members()
                .iter()
                .find(|m| m.name() == APK_NAME)
                .ok_or_else(|| format!("{} verified as an apk and holds no {APK_NAME}", entry.digest))?
                .digest();
            best = Some(Newest {
                version_code: metadata.version_code(),
                version_name: metadata.version_name().to_string(),
                signer_sha256: metadata.signer_sha256().to_string(),
                digest: entry.digest.clone(),
                app_apk,
            });
        }
    }
    Ok(best)
}

/// `apk.json` for `info` — `ADR-061` §3's eight keys, every one read from the
/// APK, in the order the ADR lists them.
#[must_use]
pub(super) fn apk_json(info: &ApkInfo) -> String {
    format!(
        "{{\"application_id\":{},\"version_code\":{},\"version_name\":{},\"label\":{},\
         \"min_sdk\":{},\"target_sdk\":{},\"signer_sha256\":{},\"development\":{}}}\n",
        json_string(&info.application_id),
        info.version_code,
        json_string(&info.version_name),
        json_string(&info.label),
        info.min_sdk,
        info.target_sdk,
        json_string(&info.signer_sha256()),
        info.application_id.ends_with(".dev"),
    )
}

/// `text` as a JSON string literal.
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A directory the wrapped `.rvt` is written to before it is published, and
/// removed afterwards whatever happened.
struct Staging(PathBuf);

impl Staging {
    fn new() -> Result<Self, String> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("rivet-apk-publish-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        Ok(Self(dir))
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        // Best-effort: a stray temporary file is harmless, and failing here
        // would turn a publish that succeeded into one reported as failed.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
#[path = "fleet_publish_apk_tests.rs"]
mod tests;
