//! **Listing published application versions, for the interview's own version
//! question** — `.rulings/` R-178 P3, R-176 Q6 ("the interview lists what is
//! published, with the newest preselected").
//!
//! # A third reader of `manifest.toml`, not a shared one
//!
//! `cmd/cargo-rivet/src/fleet.rs` and `cmd/rivet-image/src/fleet_payloads.rs`
//! both already read this exact file, and neither crate has a `[lib]` target
//! this one could depend on instead — `rivet-fleet-repo`'s own module header
//! takes the identical trade for the same file's text half, for the same
//! reason. So this reads the same fields (`kind`, `product`, `version`,
//! `added`, `digest`) a third time with the same `toml_edit` version and
//! feature set, rather than inventing a fourth format or forcing a `[lib]`
//! target onto a crate whose shape is not this change's to alter. **The
//! ORDERING rule is not re-derived, only the field reading is**: this crate
//! already depends on `rivet_core` for
//! [`rivet_core::artifact::version_order`], so [`list`] calls
//! [`rivet_core::artifact::newest_cmp`] the same way the other two readers do
//! (`.rulings/` R-213) — one rule, three field-readers.
//!
//! # What this does NOT do
//!
//! **It never picks one.** [`list`] only orders what is published, newest
//! first; the interview itself is where a bare Return takes the newest and a
//! number picks another (`.rulings/` R-176 Q6) — the same "never chosen
//! silently" rule `cmd/rivet-image/src/fleet_payloads.rs::resolve` already
//! follows for the identical file.

use std::path::Path;

use rivet_core::artifact::{BuildKey, newest_cmp};
use toml_edit::{DocumentMut, Item};

/// `<root>/manifest.toml` — `cmd/cargo-rivet/src/fleet.rs::MANIFEST_FILE`.
const MANIFEST_FILE: &str = "manifest.toml";

/// One published build, as much of it as [`list`]'s ordering needs — the
/// version string it hands back, plus the two fields [`newest_cmp`] breaks a
/// tie with. Kept internal: a caller wants versions, not a manifest entry.
struct Build {
    version: String,
    added: String,
    digest: String,
}

/// Every version of `product` published as an `application` artifact in the
/// fleet repository at `root`, **newest first** —
/// [`rivet_core::artifact::newest_cmp`]'s own ordering: version first, then
/// which was added to the repository more recently, then the digest as a
/// last, deterministic tie-break. The same rule the repository's own
/// retention and `fleet_payloads::resolve`'s "newest published" suggestion
/// already use, and needed for the identical reason (`.rulings/` R-213):
/// several builds commonly share one version's numeric core, differing only
/// in the build metadata `version_order` discards (`-g<hash>`, `-dirty`), and
/// ordering by version alone leaves those tied — a stable sort then keeps
/// whichever this manifest happens to list first, which is the OLDEST of
/// them, because publish only ever appends.
///
/// An absent `manifest.toml`, or one naming nothing for `product`, is an
/// empty list rather than an error — `cmd/rivet-image/src/fleet_payloads.rs`'s
/// own posture for "nothing published yet": a fresh `cargo rivet fleet init`
/// and a repository nobody has run that against are the same state to a
/// reader.
///
/// # Errors
///
/// `manifest.toml` exists and is not valid TOML, or an `[[artifact]]` is
/// missing one of the fields this reads.
pub fn list(root: &Path, product: &str) -> Result<Vec<String>, String> {
    let path = root.join(MANIFEST_FILE);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let doc: DocumentMut =
        text.parse().map_err(|e| format!("{} is not valid TOML: {e}", path.display()))?;
    let Some(array) = doc.get("artifact").and_then(Item::as_array_of_tables) else {
        return Ok(Vec::new());
    };

    let mut builds = Vec::new();
    for table in array.iter() {
        let string = |key: &'static str| -> Result<&str, String> {
            table.get(key).and_then(Item::as_str).ok_or_else(|| {
                format!("{} has an [[artifact]] with no string {key}", path.display())
            })
        };
        let kind = string("kind")?;
        let this_product = string("product")?;
        if kind == "application" && this_product == product {
            builds.push(Build {
                version: string("version")?.to_owned(),
                added: string("added")?.to_owned(),
                digest: string("digest")?.to_owned(),
            });
        }
    }

    builds.sort_by(|a, b| {
        newest_cmp(
            BuildKey { version: &a.version, added: &a.added, identity: &a.digest },
            BuildKey { version: &b.version, added: &b.added, identity: &b.digest },
        )
        .reverse()
    });
    Ok(builds.into_iter().map(|b| b.version).collect())
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    /// A scratch fleet repository — `manifest.toml` alone, the one file this
    /// module reads. `objects/` is not needed: `list` never resolves a
    /// digest to bytes.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("rivet-provision-versions-{label}-{n}"));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("a scratch directory");
            Self(path)
        }

        /// Append one `[[artifact]]` entry, in the shape
        /// `cmd/cargo-rivet/src/fleet.rs::render` writes.
        fn artifact(&self, product: &str, version: &str, kind: &str) {
            self.artifact_added(product, version, kind, "2026-09-13T00:00:00Z", &"a".repeat(64));
        }

        /// [`Self::artifact`], with `added` and `digest` also given rather
        /// than fixed — needed to tell two builds of the SAME version apart,
        /// which `artifact` (one fixed `added`, one fixed `digest` for every
        /// call) cannot express.
        fn artifact_added(&self, product: &str, version: &str, kind: &str, added: &str, digest: &str) {
            let manifest = self.0.join(MANIFEST_FILE);
            let mut text = fs::read_to_string(&manifest).unwrap_or_default();
            text.push_str(&format!(
                "\n[[artifact]]\ndigest = \"{digest}\"\nproduct = \"{product}\"\nversion = \
                 \"{version}\"\narch = \"aarch64\"\nkind = \"{kind}\"\nsize = 30\nadded = \
                 \"{added}\"\n"
            ));
            fs::write(&manifest, text).expect("write the manifest");
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// **The primary case, and Q6's ordering.** Several published versions
    /// come back newest first.
    #[test]
    fn published_versions_come_back_newest_first() {
        let repo = Scratch::new("newest-first");
        repo.artifact("rivet-fleetd", "0.1.0", "application");
        repo.artifact("rivet-fleetd", "0.1.10", "application");
        repo.artifact("rivet-fleetd", "0.2.0", "application");
        let versions = list(&repo.0, "rivet-fleetd").expect("a readable manifest");
        assert_eq!(versions, ["0.2.0", "0.1.10", "0.1.0"]);
    }

    /// **`.rulings/` R-213's own case.** Two builds share one version's
    /// numeric core — the ordinary shape of one workspace version published
    /// as more than one build — and the one added to the repository more
    /// recently must sort first and be marked "(newest)" by the interview's
    /// own menu (`main.rs`'s `choose_carried_version`, which trusts index 0).
    /// Written in manifest (append) order — oldest first, exactly as
    /// `cargo rivet fleet publish` always writes it.
    #[test]
    fn a_version_tie_is_broken_by_the_later_added_stamp() {
        let repo = Scratch::new("version-tie");
        repo.artifact_added(
            "rivet-fleetd",
            "0.1.114-gaaaaaaa",
            "application",
            "2026-09-24T10:00:00Z",
            &"a".repeat(64),
        );
        repo.artifact_added(
            "rivet-fleetd",
            "0.1.114-gbbbbbbb",
            "application",
            "2026-09-24T11:00:00Z",
            &"b".repeat(64),
        );
        let versions = list(&repo.0, "rivet-fleetd").expect("a readable manifest");
        assert_eq!(
            versions,
            ["0.1.114-gbbbbbbb", "0.1.114-gaaaaaaa"],
            "the later-added build of a tied version must be listed first"
        );
    }

    /// A product the repository never published is an empty list, not an
    /// error — the interview's own caller is what turns emptiness into a
    /// refusal, naming `cargo rivet fleet publish`.
    #[test]
    fn a_product_never_published_is_an_empty_list() {
        let repo = Scratch::new("missing");
        repo.artifact("rivet-fleetd", "0.1.0", "application");
        assert_eq!(list(&repo.0, "rivet-proxyd").expect("a readable manifest"), Vec::<String>::new());
    }

    /// No `manifest.toml` at all is the same "nothing published" state as an
    /// empty one — a fresh `cargo rivet fleet init`.
    #[test]
    fn an_uninitialised_repository_is_an_empty_list() {
        let repo = Scratch::new("uninitialised");
        assert_eq!(list(&repo.0, "rivet-fleetd").expect("no manifest at all"), Vec::<String>::new());
    }

    /// **Only `application` is offered** — `ADR-050` §4's rule,
    /// `fleet_payloads::resolve`'s own precedent: a `library` or `update`
    /// published under the same product name is not a candidate to carry.
    #[test]
    fn a_library_of_the_same_name_is_not_offered() {
        let repo = Scratch::new("wrong-kind");
        repo.artifact("libgreet", "1.0.0", "library");
        assert_eq!(list(&repo.0, "libgreet").expect("a readable manifest"), Vec::<String>::new());
    }
}
