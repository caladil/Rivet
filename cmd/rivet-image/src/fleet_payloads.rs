//! **Resolving a role's required payloads out of the fleet repository, by
//! digest** — `PLAN-Feature-Set-2026-09-12` §4, `ADR-050` §5.1, `ADR-052` §2.
//!
//! `ADR-050` §5.1 already decides *which* products a role needs carried — a
//! fleet server's document enables `rivet-fleetd` and `rivet-proxyd`, a
//! kiosk's enables its auto-run application — and
//! `rivet_config::Document::carried_services` reads that decision back off
//! the composed document rather than re-deriving a role this document has no
//! field for (see that method's own documentation). What was missing is this:
//! turning a required PRODUCT NAME into a PAYLOAD, so `--fleet-repository`
//! can do automatically what `--application PATH` has always made the
//! operator spell out by hand.
//!
//! # One entry or a refusal — never a guess
//!
//! **This module never picks a "latest" version on its own.** `AGENTS.md` §5
//! asks that a response which cannot be answered correctly refuse rather than
//! guess at one that looks plausible. A repository that has published two
//! versions of `rivet-fleetd` has recorded two decisions about what should
//! run, and nothing here is positioned to pick between them unless the
//! machine document (or the register it is composed from) says which —
//! `crate::carry`'s pin is `.rulings/` R-159 R20's answer to exactly this.
//!
//! So [`resolve`] takes an optional pin and:
//!
//! * **Pinned:** succeeds only when the manifest names an `application`
//!   artifact for the product AT THAT VERSION, and otherwise refuses by name
//!   — the pin, and what the repository actually holds for the product, if
//!   anything.
//! * **Unpinned, one candidate:** succeeds, exactly as before R20.
//! * **Unpinned, several candidates:** refuses, naming every one — and MAY
//!   suggest the newest as a sentence naming the key to set, which is never
//!   chosen silently (`ADR-050` §5, R-159 R20).
//! * **No candidate at all:** refuses by name, pinned or not.
//!
//! `cmd/cargo-rivet/src/fleet.rs::resolve`'s own digest-prefix ambiguity
//! refusal is the precedent this follows.
//!
//! # A second reader of `manifest.toml`, not a shared one
//!
//! `cmd/cargo-rivet/src/fleet.rs` already reads this exact file, and this
//! crate cannot depend on it — that crate has no `[lib]` target, on its own
//! module's stated reasoning (see `rivet-fleet-repo`'s header, which took the
//! identical trade for the same file's text half). So this reads the same
//! three fields a second time with the same `toml_edit` version and feature
//! set, rather than inventing a third format or forcing a `[lib]` target onto
//! a crate whose shape is not this change's to alter.

use std::path::{Path, PathBuf};

use rivet_core::artifact::{BuildKey, newest_cmp};
use toml_edit::{DocumentMut, Item};

/// `<root>/manifest.toml` — `cmd/cargo-rivet/src/fleet.rs::MANIFEST_FILE`.
const MANIFEST_FILE: &str = "manifest.toml";

/// `<root>/objects/` — `cmd/cargo-rivet/src/fleet.rs::OBJECTS_DIR`.
const OBJECTS_DIR: &str = "objects";

/// The fields resolution needs. Everything else in an `[[artifact]]`
/// table — `arch`, `platform`, `size` — is the manifest's business and not
/// this module's; `version` and `added` are read too, but only so an
/// ambiguity refusal can name what it found, and order its suggestion, in
/// words a person reads (`newest_cmp`, `.rulings/` R-213).
struct Entry {
    digest: String,
    product: String,
    version: String,
    kind: String,
    added: String,
}

/// Read every entry `<root>/manifest.toml` names, or an empty list for a
/// repository with nothing published yet — `cmd/cargo-rivet/src/fleet.rs::
/// read_manifest`'s own posture, matched rather than tightened, because a
/// fresh `cargo rivet fleet init` and a repository nobody has run that
/// against are the same "nothing published" state to a reader.
///
/// # Errors
///
/// The file exists and is not valid TOML, or an `[[artifact]]` is missing one
/// of the fields this module reads.
fn read_manifest(root: &Path) -> Result<Vec<Entry>, String> {
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

    let mut out = Vec::new();
    for table in array.iter() {
        let string = |key: &'static str| -> Result<String, String> {
            table
                .get(key)
                .and_then(Item::as_str)
                .map(ToString::to_string)
                .ok_or_else(|| format!("{} has an [[artifact]] with no string {key}", path.display()))
        };
        out.push(Entry {
            digest: string("digest")?,
            product: string("product")?,
            version: string("version")?,
            kind: string("kind")?,
            added: string("added")?,
        });
    }
    Ok(out)
}

/// Resolve `product` against `root`'s manifest to the path of its bytes in
/// the object store.
///
/// **Only `kind = "application"` is considered** — `ADR-050` §4.5 refuses to
/// carry a `library` at all (a content-addressed install the machine cannot
/// yet ask the filesystem "did this land"), and an `update` is `ADR-018` §5's
/// A/B mechanism and does not arrive this way either. A repository that holds
/// only a `library` or `update` entry for the name reads as **not found**,
/// with the reason said out loud, rather than a confusing "found it" that
/// then fails to carry.
///
/// `pinned` is the machine document's (or the register's) answer to *which*
/// version, read off `rivet_config::Document::carried_version` —
/// `crate::carry`, `.rulings/` R-159 R20. `None` means the document named no
/// pin for this product, which is not the same as *no version has been
/// published*: see this module's own documentation for what each combination
/// does.
///
/// # Errors
///
/// * **Pinned, and no `application` artifact matches BOTH the product and the
///   pinned version** — refused by name, naming the pin and, if the product
///   was published at all, every version this repository actually holds for
///   it.
/// * **Unpinned, and no `application` artifact names `product`** — the fleet
///   server (or whatever the role required) has never been published to this
///   repository, or was published as a different kind. Names the product and
///   what `cargo rivet fleet publish` is for.
/// * **Unpinned, and more than one does** — refuses rather than guessing
///   which the operator meant (see this module's own documentation), naming
///   every candidate's digest and version, and suggesting the newest as the
///   key to set — never chosen silently.
/// * **The manifest names bytes `objects/` does not have** — the repository
///   is inconsistent, `cmd/cargo-rivet/src/fleet.rs::materialise_for_deploy`'s
///   own refusal for the identical inconsistency.
pub fn resolve(root: &Path, product: &str, pinned: Option<&str>) -> Result<PathBuf, String> {
    let entries = read_manifest(root)?;
    let candidates: Vec<&Entry> =
        entries.iter().filter(|e| e.product == product && e.kind == "application").collect();

    let selected: &Entry = if let Some(version) = pinned {
        let matches: Vec<&Entry> = candidates.iter().copied().filter(|e| e.version == version).collect();
        match matches.as_slice() {
            [] if candidates.is_empty() => {
                return Err(format!(
                    "the machine document pins {product:?} to version {version:?} and the fleet \
                     repository at {} has no `application` artifact of that name at all. `cargo \
                     rivet fleet publish` (or `cargo rivet fleet list` to see what it already \
                     holds) is how one gets there.",
                    root.display()
                ));
            }
            [] => {
                return Err(format!(
                    "the machine document pins {product:?} to version {version:?} and the fleet \
                     repository at {} holds no `application` artifact of that product at that \
                     version. Published: {}.",
                    root.display(),
                    candidates.iter().map(|e| e.version.as_str()).collect::<Vec<_>>().join(", "),
                ));
            }
            [one] => one,
            many => {
                return Err(format!(
                    "the machine document pins {product:?} to version {version:?} and the fleet \
                     repository at {} holds {} `application` artifacts of that product at that \
                     version, and nothing here is positioned to pick between them (AGENTS.md §5 \
                     — refuse rather than guess):\n{}",
                    root.display(),
                    many.len(),
                    many.iter().map(|e| format!("  {} {} {}\n", e.digest, e.product, e.version)).collect::<String>(),
                ));
            }
        }
    } else {
        match candidates.as_slice() {
            [] => {
                return Err(format!(
                    "the role requires {product:?} and the fleet repository at {} has no \
                     `application` artifact of that name. `cargo rivet fleet publish` (or \
                     `cargo rivet fleet list` to see what it already holds) is how one gets \
                     there.",
                    root.display()
                ));
            }
            [one] => one,
            many => {
                // `.rulings/` R-213: ordered by version then by which was
                // added to the repository more recently — never by
                // `version_order` alone, which ties every build sharing a
                // version's numeric core and would leave the suggestion
                // depending on the manifest's own listing order.
                let newest = many.iter().max_by(|a, b| {
                    newest_cmp(
                        BuildKey { version: &a.version, added: &a.added, identity: &a.digest },
                        BuildKey { version: &b.version, added: &b.added, identity: &b.digest },
                    )
                });
                let suggestion = newest.map_or_else(String::new, |e| {
                    format!(
                        "\nThe newest published is {} {}; pin it by writing `version = {:?}` \
                         under `[carry.{product}]` in the machine document, or pin a different \
                         one. A pin is never chosen silently.",
                        e.product, e.version, e.version
                    )
                });
                return Err(format!(
                    "the role requires {product:?} and the fleet repository at {} holds {} \
                     `application` artifacts of that name, and nothing here is positioned to \
                     pick between them (AGENTS.md §5 — refuse rather than guess):\n{}Publish \
                     only one, pin the version the machine document should carry, or carry it \
                     explicitly with --application PATH instead of --fleet-repository.{suggestion}",
                    root.display(),
                    many.len(),
                    many.iter().map(|e| format!("  {} {} {}\n", e.digest, e.product, e.version)).collect::<String>(),
                ));
            }
        }
    };

    let object = root.join(OBJECTS_DIR).join(&selected.digest);
    if !object.is_file() {
        return Err(format!(
            "the fleet repository's manifest names {} ({} {}) and its bytes are not at {} — the \
             repository is inconsistent. Nothing was carried.",
            selected.digest,
            selected.product,
            selected.version,
            object.display()
        ));
    }
    Ok(object)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    /// A scratch fleet repository — `manifest.toml` beside `objects/`, the
    /// same shape `cargo rivet fleet init`/`publish` leave, built by hand so
    /// this module's tests do not need `cmd/cargo-rivet` to run.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("rivet-image-fleet-payloads-{label}-{n}"));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(path.join(OBJECTS_DIR)).expect("a scratch object store");
            Self(path)
        }

        /// Add one artifact: write its bytes under `objects/<digest>` and
        /// append its entry to `manifest.toml`, in exactly the shape
        /// `cmd/cargo-rivet/src/fleet.rs::render` writes.
        fn artifact(&self, digest: &str, product: &str, version: &str, kind: &str) {
            self.artifact_added(digest, product, version, kind, "2026-09-13T00:00:00Z");
        }

        /// [`Self::artifact`], with `added` also given rather than fixed —
        /// needed to tell two builds of the SAME version apart, which
        /// `artifact` (one fixed `added` for every call) cannot express.
        fn artifact_added(&self, digest: &str, product: &str, version: &str, kind: &str, added: &str) {
            fs::write(self.0.join(OBJECTS_DIR).join(digest), b"not a real payload, just bytes")
                .expect("write the object");
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

    /// **The primary case**: a role's product resolves to its bytes' path by
    /// digest, from a scratch fleet repository.
    #[test]
    fn a_unique_application_resolves_to_its_object_by_digest() {
        let repo = Scratch::new("unique");
        repo.artifact("d".repeat(64).as_str(), "rivet-fleetd", "0.1.0", "application");
        let resolved = resolve(&repo.0, "rivet-fleetd", None).expect("exactly one candidate");
        assert_eq!(resolved, repo.0.join(OBJECTS_DIR).join("d".repeat(64)));
    }

    /// **A missing payload is refused by name.** An empty repository, or one
    /// that never published this product, names the product it could not
    /// find rather than silently carrying nothing.
    #[test]
    fn a_product_the_repository_never_published_is_refused_by_name() {
        let repo = Scratch::new("missing");
        let err = resolve(&repo.0, "rivet-fleetd", None).expect_err("nothing published");
        assert!(err.contains("rivet-fleetd"), "{err}");
        assert!(err.contains("no `application` artifact"), "{err}");
    }

    /// A repository that has never been published to at all — no
    /// `manifest.toml` on disk — is the same "nothing published" refusal, not
    /// an I/O error.
    #[test]
    fn an_uninitialised_repository_refuses_the_same_way_as_an_empty_one() {
        let repo = Scratch::new("uninitialised");
        fs::remove_file(repo.0.join(MANIFEST_FILE)).ok();
        let err = resolve(&repo.0, "rivet-fleetd", None).expect_err("no manifest at all");
        assert!(err.contains("rivet-fleetd"), "{err}");
    }

    /// **Refuse rather than guess, unpinned.** Two published versions of one
    /// product is two candidates, and this module picks neither — it names
    /// both, says why, and suggests the newest as the key to set, which is
    /// never chosen silently (`.rulings/` R-159 R20).
    #[test]
    fn two_published_versions_of_one_product_are_refused_not_guessed() {
        let repo = Scratch::new("ambiguous");
        repo.artifact("a".repeat(64).as_str(), "rivet-fleetd", "0.1.0", "application");
        repo.artifact("b".repeat(64).as_str(), "rivet-fleetd", "0.2.0", "application");
        let err = resolve(&repo.0, "rivet-fleetd", None).expect_err("two candidates, not one");
        assert!(err.contains("2 `application`"), "{err}");
        assert!(err.contains("0.1.0") && err.contains("0.2.0"), "{err}");
        // The suggestion: the newest, named, with the key to set — and it is a
        // suggestion, not a silent choice, so the function still refused.
        assert!(err.contains("newest published is rivet-fleetd 0.2.0"), "{err}");
        assert!(err.contains("[carry.rivet-fleetd]"), "{err}");
        assert!(err.contains("version = \"0.2.0\""), "{err}");
    }

    /// **`.rulings/` R-213.** Two builds share one version's numeric core —
    /// differing only in the build metadata `version_order` discards
    /// (`-g<hash>`) — so the suggestion must name the one added to the
    /// repository more recently, REGARDLESS of which order the manifest
    /// lists them in. Proven both ways: a manifest this tool writes always
    /// lists them oldest-first, but nothing enforces that of a hand-edited
    /// one, and the suggestion is a fact about the builds, not about how
    /// they were read.
    #[test]
    fn a_version_tie_suggests_the_most_recently_added_build_regardless_of_manifest_order() {
        for reversed in [false, true] {
            let repo = Scratch::new(&format!("version-tie-{reversed}"));
            let mut writes = [
                ("a".repeat(64), "0.1.114-gaaaaaaa", "2026-09-24T10:00:00Z"),
                ("b".repeat(64), "0.1.114-gbbbbbbb", "2026-09-24T11:00:00Z"),
            ];
            if reversed {
                writes.reverse();
            }
            for (digest, version, added) in &writes {
                repo.artifact_added(digest, "tempmon-rivet", version, "application", added);
            }
            let err = resolve(&repo.0, "tempmon-rivet", None).expect_err("two tied candidates");
            assert!(
                err.contains("newest published is tempmon-rivet 0.1.114-gbbbbbbb"),
                "reversed={reversed}: {err}"
            );
        }
    }

    /// **R-159 R20's pinned case: present.** A document that pins the version
    /// gets exactly that one, even with another version also published —
    /// the pin decides, unambiguously, what the unpinned case cannot.
    #[test]
    fn a_pinned_version_that_is_published_resolves_to_it() {
        let repo = Scratch::new("pinned-present");
        repo.artifact("a".repeat(64).as_str(), "rivet-fleetd", "0.1.0", "application");
        repo.artifact("b".repeat(64).as_str(), "rivet-fleetd", "0.2.0", "application");
        let resolved =
            resolve(&repo.0, "rivet-fleetd", Some("0.1.0")).expect("the pinned version exists");
        assert_eq!(resolved, repo.0.join(OBJECTS_DIR).join("a".repeat(64)));
    }

    /// **R-159 R20's pinned case: absent.** A pin naming a version this
    /// repository never published is refused by name, whether or not OTHER
    /// versions of the product exist — never silently substituted.
    #[test]
    fn a_pinned_version_that_is_not_published_is_refused_by_name() {
        let repo = Scratch::new("pinned-absent");
        repo.artifact("a".repeat(64).as_str(), "rivet-fleetd", "0.1.0", "application");
        let err = resolve(&repo.0, "rivet-fleetd", Some("9.9.9"))
            .expect_err("9.9.9 was never published");
        assert!(err.contains("rivet-fleetd"), "{err}");
        assert!(err.contains("9.9.9"), "{err}");
        // What IS published is named, so the operator can fix the pin.
        assert!(err.contains("0.1.0"), "{err}");

        // And pinning a product never published at all is the same refusal
        // in shape, naming the pin rather than claiming a version exists to
        // compare against.
        let err = resolve(&repo.0, "rivet-proxyd", Some("0.1.0"))
            .expect_err("rivet-proxyd was never published");
        assert!(err.contains("rivet-proxyd"), "{err}");
        assert!(err.contains("0.1.0"), "{err}");
    }

    /// **Only `application` is carried this way** — `ADR-050` §4.5. A
    /// `library` published under the same product name is not a candidate,
    /// so this reads as "not found" rather than "found the wrong kind and
    /// carried it anyway".
    #[test]
    fn a_library_of_the_same_name_is_not_a_candidate() {
        let repo = Scratch::new("wrong-kind");
        repo.artifact("c".repeat(64).as_str(), "libgreet", "1.0.0", "library");
        let err = resolve(&repo.0, "libgreet", None).expect_err("a library is not an application");
        assert!(err.contains("libgreet"), "{err}");
    }

    /// **`ADR-052` §7: a hidden product is still carried.** Hiding withholds a
    /// product from every fleet and keeps it in the repository, so a card
    /// build that carries it still finds it. The manifest's `[[hidden]]`
    /// tables are read by no one here, and do not stop the file reading.
    #[test]
    fn a_hidden_product_still_resolves_and_the_hidden_set_does_not_stop_the_read() {
        let repo = Scratch::new("hidden");
        repo.artifact("d".repeat(64).as_str(), "rivet-fleetd", "0.1.0", "application");
        let manifest = repo.0.join(MANIFEST_FILE);
        let mut text = fs::read_to_string(&manifest).expect("the manifest");
        // Hand-composed, in the shape `cmd/cargo-rivet`'s `render_manifest` writes.
        text.push_str("\n[[hidden]]\nproduct = \"rivet-fleetd\"\nkind    = \"application\"\n");
        fs::write(&manifest, text).expect("the manifest");
        let resolved = resolve(&repo.0, "rivet-fleetd", Some("0.1.0")).expect("a hidden product resolves");
        assert_eq!(resolved, repo.0.join(OBJECTS_DIR).join("d".repeat(64)));
    }

    /// The repository's manifest can go inconsistent with its own object
    /// store; this must be refused rather than carrying a path with nothing
    /// behind it onto the boot partition.
    #[test]
    fn a_manifest_naming_bytes_the_store_does_not_have_is_refused() {
        let repo = Scratch::new("inconsistent");
        let manifest = repo.0.join(MANIFEST_FILE);
        // Written directly rather than through `artifact()`, which always
        // writes the object too — the point here is that it must NOT exist.
        fs::write(
            &manifest,
            format!(
                "\n[[artifact]]\ndigest = \"{}\"\nproduct = \"rivet-fleetd\"\nversion = \"0.1.0\"\n\
                 arch = \"aarch64\"\nkind = \"application\"\nsize = 30\nadded = \"2026-09-13\"\n",
                "e".repeat(64)
            ),
        )
        .expect("write the manifest");
        let err = resolve(&repo.0, "rivet-fleetd", None).expect_err("the object is not on disk");
        assert!(err.contains("inconsistent"), "{err}");
    }
}
