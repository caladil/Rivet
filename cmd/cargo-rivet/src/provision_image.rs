//! Selecting a system image from the fleet repository for
//! `cargo rivet provision` — `.rulings/` R-176 Q1/Q2/Q6/Q8, R-178 (platform
//! note, P2, P3), R-180. **Wave C2 of the provisioning rebuild.**
//!
//! # What lives here, and what stays in `provision.rs`
//!
//! This module holds the pure decisions (platform, schema, refusal text) and
//! the two pieces of I/O that are specific to selecting an image: the
//! `create a fleet repository?` prompt (item 1) and materialising a chosen
//! entry (item 6, `fleet::materialise_image` with the workstation's own
//! `keys/rivet-system.pub`). `provision.rs` owns the `ImageSelector` trait and
//! its real/fake implementors, on the same footing as `Preflight`,
//! `Interview` and every other seam that module's own documentation explains
//! — this module is the thing the real one calls, not a second orchestrator.
//!
//! # R-178 — platform is asked, never inferred
//!
//! When the platform is not indicated and more than one is available, the
//! maintainer expects to be prompted for the platform he is provisioning for
//! before an image is selected.
//!
//! The design draft this wave started from (`provision-design/DRAFT.md` §3)
//! proposed deriving the platform from this workstation's own cross-build
//! target (`rivet_adapter_image::PiFirmware::platform_token`). R-178
//! overrides that: the platform comes from what the fleet repository's own
//! manifest actually holds ([`distinct_platforms`]) — one platform is used
//! silently and named on screen; more than one is asked before a single
//! image is listed ([`decide_platform`]); a named `--platform` token with no
//! image is refused by name, listing the platforms that do have one. The
//! tool's own compiled target decides nothing here.
//!
//! # R-178 P2 — a schema mismatch is listed and refused, never a note
//!
//! The design draft (§3) recommended a printed *note* for an image built
//! against a different `rivet_config::SCHEMA_VERSION` than this build
//! speaks, reasoning that a one-version skew might not even matter. R-178
//! overrules that: *"a board refuses a document on any other schema
//! version"* (`rivet-config`'s own `parse.rs`), so an image this build could
//! not compose a working document for is not a real option — it is listed
//! (so an operator can see it exists and why it is unusable), marked with the
//! reason ([`mark_schema`]), and cannot be selected. If nothing for the
//! chosen platform is selectable, that is a refusal by name
//! ([`none_selectable_refusal`]), not a silent fallback to an older schema.
//!
//! # R-178 P3 — selection happens before the interview
//!
//! Every function here is called from `provision.rs`'s `RealImageSelector`,
//! itself called before `Interview::run` — a refusal here (no fleet
//! repository, no image, an unknown platform, nothing selectable) costs the
//! operator no typing at all.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use rivet_core::package::port::PublicKey;

use crate::fleet::{self, Entry};

// ---------------------------------------------------------------------------
// Item 1 — offering to create a missing fleet repository, the way
// `cmd/rivet-provision`'s own `ensure_authority` offers a CA.
// ---------------------------------------------------------------------------

/// Whether a missing fleet repository should be created, given the raw
/// answer text a terminal read (or would read) to `create one? [Y/n]`.
///
/// **Pure**, so every answer shape — an empty line (bare Return), `y`, `yes`,
/// `n`, `no`, garbage — is provable with no real prompt at all.
/// [`ensure_fleet_repository`] is the only caller that ever reads a real one.
#[must_use]
pub(crate) fn wants_fleet_init(answer: &str) -> bool {
    let normalized = answer.trim().to_ascii_lowercase();
    normalized.is_empty() || normalized == "y" || normalized == "yes"
}

/// Whether `root` already looks like a fleet repository — `manifest.toml`
/// (a fresh `fleet init` always writes one, even empty) or `.git` (a
/// repository from before this field existed). Reused by
/// [`ensure_fleet_repository`] so its own "already there" branch and this
/// probe can never drift apart.
#[must_use]
fn fleet_repository_exists(root: &Path) -> bool {
    root.join("manifest.toml").is_file() || root.join(".git").is_dir()
}

/// **Item 1.** `no fleet repository yet at <path> -- create one? [Y/n]`, the
/// same shape `cmd/rivet-provision`'s own `ensure_authority` already uses for
/// a missing CA (`provision-design/DRAFT.md` §2).
///
/// * **Present** (`manifest.toml` or `.git`): returns, silently — the
///   ordinary case on every run after the first.
/// * **Absent, with a terminal:** asks; `yes` (or a bare Return) calls
///   [`fleet::init`] — this crate's own function, no subprocess; anything
///   else (`no`, or nothing at all) declines and returns anyway. Nothing is
///   created silently either way.
/// * **Absent, no terminal:** declines without asking — there is nothing
///   here to ask an agent. Either way, an empty (or still-missing) repository
///   reaches the listing step next, which is where `.rulings/` R-176 Q8's own
///   refusal belongs ("No images available for provisioning…") — declining
///   the offer and finding no images are the same honest state to a caller.
///
/// # Errors
///
/// This run's own stdin could not be read, or [`fleet::init`] itself refused
/// (`ADR-052` §4's `.seed` check, or `git` failing).
pub(crate) fn ensure_fleet_repository(root: &Path, is_terminal: bool) -> Result<(), String> {
    if fleet_repository_exists(root) {
        return Ok(());
    }
    if !is_terminal {
        return Ok(());
    }
    print!("no fleet repository yet at {} -- create one? [Y/n] ", root.display());
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(|e| format!("cannot read a choice: {e}"))?;
    if wants_fleet_init(&line) {
        fleet::init(root)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Items 2/3 — platform.
// ---------------------------------------------------------------------------

/// `.rulings/` R-176 Q8's own refusal, verbatim — an empty repository, or one
/// with no `card` images at all for any platform.
pub(crate) const NO_IMAGES_REFUSAL: &str =
    "No images available for provisioning. Deploy a system image to your artifact repository \
     prior to provisioning.";

/// Every distinct `platform` a `card` entry names, sorted for a
/// deterministic menu order.
///
/// `Entry::platform` is `None` only for a kind other than `card`
/// (`cargo rivet image` always names one for a card, `ADR-027` §3) — since
/// `images` is already `card`-only ([`fleet::list_images`]'s own contract),
/// this filters out `None` defensively rather than assuming it never occurs.
#[must_use]
pub(crate) fn distinct_platforms(images: &[Entry]) -> Vec<String> {
    let mut platforms: Vec<String> = images.iter().filter_map(|e| e.platform.clone()).collect();
    platforms.sort();
    platforms.dedup();
    platforms
}

/// **R-178: the platform is asked, never inferred from the tool's own build
/// target — see the module documentation.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PlatformDecision {
    /// Used without asking — exactly one platform has images, or
    /// `--platform` named one that does. Named on screen either way.
    Resolved(String),
    /// More than one platform has images and `--platform` named none — ask,
    /// from this exact list (already sorted, per [`distinct_platforms`]).
    Ask(Vec<String>),
}

/// # Errors
///
/// No platform has any image at all ([`NO_IMAGES_REFUSAL`], verbatim), or
/// `requested` names a platform with none — listing, by name, the platforms
/// that do have images.
pub(crate) fn decide_platform(
    all_platforms: &[String],
    requested: Option<&str>,
) -> Result<PlatformDecision, String> {
    if all_platforms.is_empty() {
        return Err(NO_IMAGES_REFUSAL.to_owned());
    }
    if let Some(token) = requested {
        return if all_platforms.iter().any(|p| p == token) {
            Ok(PlatformDecision::Resolved(token.to_owned()))
        } else {
            Err(format!(
                "no images in the fleet repository are for platform {token:?}. Platforms with \
                 images: {}",
                all_platforms.join(", "),
            ))
        };
    }
    match all_platforms {
        [one] => Ok(PlatformDecision::Resolved(one.clone())),
        many => Ok(PlatformDecision::Ask(many.to_vec())),
    }
}

// ---------------------------------------------------------------------------
// Item 4 — schema mismatch: listed, marked, never silently skipped.
// ---------------------------------------------------------------------------

/// **R-178 P2.** Why `entry` cannot be selected by THIS build, or `None` when
/// it can — a board refuses a document on any schema but its own
/// (`rivet-config`'s `parse.rs`), so a mismatch here is named rather than
/// left to fail deep inside the interview or, worse, on the board itself.
#[must_use]
pub(crate) fn schema_block_reason(entry: &Entry, tool_schema: u32) -> Option<String> {
    match entry.schema_version {
        Some(v) if v == tool_schema => None,
        Some(v) => Some(format!("needs schema {v}, this tool speaks {tool_schema}")),
        None => Some(format!("carries no recorded schema version, this tool speaks {tool_schema}")),
    }
}

/// One `card` image, together with whether [`schema_block_reason`] lets it be
/// selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub entry: Entry,
    /// `None`: selectable. `Some(reason)`: R-178 P2's own reason, shown on
    /// screen and in [`none_selectable_refusal`].
    pub blocked: Option<String>,
}

/// Mark every image with [`schema_block_reason`] — order preserved
/// ([`fleet::list_images`]'s own newest-first order).
#[must_use]
pub(crate) fn mark_schema(images: Vec<Entry>, tool_schema: u32) -> Vec<Candidate> {
    images
        .into_iter()
        .map(|entry| {
            let blocked = schema_block_reason(&entry, tool_schema);
            Candidate { entry, blocked }
        })
        .collect()
}

/// **R-178 P2, "if no image is selectable, refuse by name before the
/// interview."** Named against every blocked candidate's own reason, so the
/// refusal explains itself rather than just saying "none".
#[must_use]
pub(crate) fn none_selectable_refusal(platform: &str, blocked: &[Candidate], tool_schema: u32) -> String {
    let mut out = format!(
        "No image for platform {platform:?} can be provisioned by this build (schema version \
         {tool_schema}) -- every image the fleet repository holds for it needs a different \
         schema:\n",
    );
    for candidate in blocked {
        out.push_str(&format!(
            "  {} {} {} -- {}\n",
            &candidate.entry.digest[..candidate.entry.digest.len().min(12)],
            candidate.entry.product,
            candidate.entry.version,
            candidate.blocked.as_deref().unwrap_or(""),
        ));
    }
    out.push_str(
        "Deploy an image built against this schema version, or use a build of cargo-rivet that \
         speaks the image's own schema version.",
    );
    out
}

/// One line per candidate, for the selection menu — digest prefix, product,
/// version, schema, size and when it was added, the same fields
/// `fleet::list`'s own report already shows a person.
#[must_use]
pub(crate) fn describe_entry(entry: &Entry) -> String {
    format!(
        "{} {} (schema {}, {} bytes, added {}) {}",
        entry.product,
        entry.version,
        entry.schema_version.map_or_else(|| "?".to_owned(), |v| v.to_string()),
        entry.size,
        entry.added,
        &entry.digest[..entry.digest.len().min(12)],
    )
}

// ---------------------------------------------------------------------------
// Item 6 — verifying and decompressing the selected image.
// ---------------------------------------------------------------------------

/// A fresh scratch directory this run may decompress an image into —
/// `<at>/provision/image-work-<pid>`, beside `<at>/provision`'s own documents
/// and staging bundles. **Never `~/.rivet/fleet` itself**, which stays
/// read-only object storage (`ADR-052` §3) — this is always a second,
/// disposable copy.
#[must_use]
pub(crate) fn scratch_dir(at: &Path) -> PathBuf {
    at.join("provision").join(format!("image-work-{}", std::process::id()))
}

/// Remove this run's scratch working copy `image`, then the
/// [`scratch_dir`] directory that holds it.
///
/// **The directory used to be left behind.** Only the file was removed, so
/// every run left an empty `image-work-<pid>` under `<at>/provision` — four
/// of them after the maintainer's first real runs, 2026-09-14.
///
/// **Narrow on purpose.** The directory is removed only when it is named
/// `image-work-<this process id>`, which is the one this run's own
/// [`scratch_dir`] names, and only when it is empty once `image` is gone:
/// `std::fs::remove_dir` refuses a directory holding anything else, so a file
/// this run did not write keeps its directory. A working copy anywhere else,
/// such as a test's fixture, loses only the file.
pub(crate) fn remove_scratch(image: &Path) {
    let _ = std::fs::remove_file(image);
    let Some(dir) = image.parent() else {
        return;
    };
    let ours = format!("image-work-{}", std::process::id());
    if dir.file_name().is_some_and(|name| name == ours.as_str()) {
        let _ = std::fs::remove_dir(dir);
    }
}

/// How old another run's scratch must be before a later run removes it.
///
/// A run lasts minutes, or as long as an operator leaves `Write it? [y/N]`
/// unanswered; twelve hours is far beyond both, so nothing a live run is
/// still using is old enough.
pub(crate) const STALE_SCRATCH_AFTER: std::time::Duration = std::time::Duration::from_secs(12 * 60 * 60);

/// **Remove scratch that earlier runs left behind**, and say what went.
///
/// [`remove_scratch`] runs when a run ends, and a run that is killed, one whose
/// administrator prompt is cancelled by closing its window, or one interrupted
/// at `Write it? [y/N]`, never reaches it. What it leaves is a whole card image
/// with a machine's identity injected into it, private key included. No code
/// runs in a process that has been killed, so the removal belongs to the next
/// run instead — `crate::scratch`'s arrangement for this crate's tests.
///
/// **Narrow, like [`remove_scratch`].** Only a directory under
/// `<at>/provision` named `image-work-<digits>`, not this process's own, and
/// last modified more than [`STALE_SCRATCH_AFTER`] before `now`. Best-effort
/// throughout: a directory that cannot be read or removed is left for next
/// time, and this never fails the run it precedes. Returns one line per
/// directory removed.
pub(crate) fn sweep_stale_scratch(at: &Path, now: std::time::SystemTime) -> Vec<String> {
    let ours = format!("image-work-{}", std::process::id());
    let Ok(entries) = std::fs::read_dir(at.join("provision")) else {
        return Vec::new();
    };
    let mut removed = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(pid) = name.strip_prefix("image-work-") else {
            continue;
        };
        if name == ours || pid.is_empty() || !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let stale = meta
            .modified()
            .ok()
            .and_then(|at| now.duration_since(at).ok())
            .is_some_and(|age| age > STALE_SCRATCH_AFTER);
        if !meta.is_dir() || !stale {
            continue;
        }
        let bytes: u64 = std::fs::read_dir(entry.path())
            .map(|inside| inside.flatten().filter_map(|f| f.metadata().ok()).map(|m| m.len()).sum())
            .unwrap_or(0);
        if std::fs::remove_dir_all(entry.path()).is_ok() {
            removed.push(format!(
                "removed {} ({bytes} bytes), left by an earlier run that did not finish",
                entry.path().display()
            ));
        }
    }
    removed
}

/// Verify `entry` and decompress it — [`fleet::materialise_image`], with the
/// workstation's own `keys/rivet-system.pub` as the trust anchor: the
/// established location `cargo rivet setup` writes
/// (`cmd/cargo-rivet/src/setup.rs`) and `write_image.rs`'s own `trust_note`
/// already reads — never the seed.
///
/// # Errors
///
/// The public key file cannot be read as 32 bytes, or
/// [`fleet::materialise_image`]'s own errors (the object missing, the header
/// not verifying, a digest or length mismatch).
pub(crate) fn materialise(
    root: &Path,
    entry: &Entry,
    rivet_root: &Path,
    out_dir: &Path,
) -> Result<PathBuf, String> {
    let key_path = rivet_root.join("keys").join("rivet-system.pub");
    let trusted = crate::key::read_32(&key_path, "system signing public key")?;
    // A failure after `materialise_image` created `out_dir` (a decompression
    // error, say) would leave it, and possibly a partial image, behind. Only
    // a directory this call created is removed, so everything in it is this
    // call's own.
    let existed = out_dir.exists();
    let result = fleet::materialise_image(root, entry, PublicKey(trusted), out_dir);
    if result.is_err() && !existed {
        let _ = std::fs::remove_dir_all(out_dir);
    }
    result
}

#[cfg(test)]
#[path = "provision_image_tests.rs"]
mod tests;
