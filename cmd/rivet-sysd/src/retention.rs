//! **One build of a library per product, so that replacing an addon does not
//! darken a screen.** `.rulings/` R-202, `SPEC-02` §12.
//!
//! # The hole this closes
//!
//! A library artifact's identity is the digest of its bytes (`SPEC-02` §3) and
//! **Rust builds are not byte-reproducible across rebuilds**, so the same
//! source built twice emits two references and installs as two directories
//! under `<volume>/lib`. Both declare the same `provides-class`, and two
//! libraries of one class is exactly what `.rulings/` R-200 makes
//! `rivetos_addon::host::Loaded::open(class)` refuse — correctly, because
//! choosing between them is not a decision a host can make.
//!
//! The consequence was that **shipping a new version of an addon put out the
//! card that addon feeds**, and nothing removed a library but
//! `apply remove-directory` against `/state/lib` by hand. There was no
//! supported form of the operation at all.
//!
//! # Where this runs, and why it is an edge rather than a clock
//!
//! [`crate::installed::Watch::look`], at the install edge, **before**
//! `crate::applications::discover` — so the discovery that composes
//! `RIVET_ADDONS` and `LD_LIBRARY_PATH` sees the retained set rather than the
//! one this is about to change.
//!
//! An install is an **operator-initiated act**, which is the whole reason a
//! sweep is allowed to delete anything: `crate::installed`'s own header makes
//! the same argument for why it re-runs discovery there and nowhere else. This
//! deliberately does **not** run at boot. A machine already holding two builds
//! of one library is in a state an operator can see and repair, and a
//! supervisor that pruned `/state` on its way up would be deleting an
//! operator's artifacts in the one pass nobody is watching.
//!
//! # Why a fault here never stops the boot, and never evicts either
//!
//! `crate::applications` and `crate::libraries` both argue that a fault under
//! `/state` degrades rather than stops. This module takes that one step
//! further, because this module **removes** things and neither of those two
//! does: every failure to learn something ends the sweep with nothing evicted.
//!
//! An application whose manifest cannot be read is an application whose
//! declared closure is unknown, and a guard computed from an unknown closure is
//! a guard with a hole in it. So the answer to *"I could not read it"* is to
//! keep every library on the machine and say so — a board that needs a
//! gigabyte tidied is a smaller problem than one whose application will not
//! start after the next reboot.

use rivet_core::package::closure;
use rivet_core::package::install::{HeldLibrary, KEEP_LIBRARY, lib_root, library_dir};
use rivet_core::package::library::Reference;

use crate::journal::{Journal, Severity, Value};
use crate::kernel::{Errno, Kernel};
use crate::supervise::SELF_NAME;

/// Retain one build per library product on `volume`, and remove the rest.
///
/// Reports every eviction **by name** — `.rulings/` R-202 D1 asks for that in
/// the same breath as the rule, and `ADR-052` §7 already learned why: a
/// store that pruned silently left an operator with a missing artifact and no
/// record of anything having decided to remove it.
pub fn sweep(k: &mut dyn Kernel, journal: &mut Journal, volume: &str) {
    let Some(declared) = declared_closures(k, journal, volume) else {
        return;
    };
    let Some(held) = held_libraries(k, journal, volume) else {
        return;
    };

    // `arriving` is `None` here, and that is the one place this differs from
    // the artifact store's caller. The store transfers several payloads inside
    // one clock tick, so it has to name the object it just wrote or the
    // tie-break can delete exactly those bytes (`ADR-052` §7). This runs
    // once per finished install, over directories whose ages are minutes or
    // days apart, and it is reached through `crate::installed` — which refuses
    // on principle to take the installer's account of what it wrote as an
    // authority (that module's header). What separates two builds of one
    // version here is `HeldLibrary::added`, the directory's own modification
    // time, and the machine that wrote it is the machine reading it.
    let dropped = rivet_core::package::install::library_evictions(
        &held,
        KEEP_LIBRARY,
        None,
        &declared,
    );
    if dropped.is_empty() {
        return;
    }

    for entry in dropped {
        let path = library_dir(volume, &entry.reference);
        // **A removal of a path built from a directory name read off the
        // card.** What makes it safe is the same thing that makes the
        // uninstall path safe: the name went through `Reference::parse`, which
        // refuses anything but `[A-Za-z0-9._-]`, refuses a leading `.` or `-`,
        // refuses `..`, and requires a trailing `-<64 lowercase hex>`. A string
        // that is not one path component below `<volume>/lib` cannot reach
        // here.
        //
        // **Renamed aside and deleted by a helper** (`ADR-017` §3): the build
        // is gone from the library root when this reports it, which is all the
        // discovery after an install needs, and the time a deletion takes is
        // not spent in the loop.
        match crate::reclaim::set_aside(k, SELF_NAME, &path) {
            Ok(done) => {
                let mut fields = vec![
                    ("reference".to_string(), Value::Str(entry.reference.to_string())),
                    ("path".to_string(), Value::Str(path)),
                    ("keep".to_string(), Value::Int(KEEP_LIBRARY as i64)),
                ];
                if let Some(e) = done.unforked {
                    fields.push((
                        "deletion".to_string(),
                        Value::Str(format!(
                            "the helper to delete what was set aside could not start ({e}); the \
                             next start deletes it"
                        )),
                    ));
                }
                note(
                    k,
                    journal,
                    Severity::Info,
                    "an older build of an installed library is superseded and has been removed, \
                     so one build of this product is installed and a host accepting its class \
                     has one library to join (.rulings/ R-202, SPEC-02 §12)",
                    fields,
                );
            }
            Err(e) => note(
                k,
                journal,
                Severity::Warn,
                "DEGRADED an older build of an installed library could not be removed, so this \
                 machine still holds two builds of one product. If they declare a class, a host \
                 accepting it will refuse to choose between them and the card it draws stays \
                 dark until one is removed by hand (.rulings/ R-200, R-202)",
                vec![
                    ("reference".to_string(), Value::Str(entry.reference.to_string())),
                    ("path".to_string(), Value::Str(path)),
                    ("detail".to_string(), Value::Str(e.to_string())),
                ],
            ),
        }
    }
}

/// Every reference every installed application declares, or nothing.
///
/// **`None` means "do not evict".** See this module's header: a closure this
/// function could not read is a guard with a hole in it, and the hole is an
/// application that will not start after the next reboot.
///
/// The reading is `rivet_core::package::closure::declared`'s, which
/// `apply uninstall-library` reads through as well — one reader of which
/// manifests count, so retention and a removal on purpose cannot disagree. It
/// passes over an image service's home and an application holding only its
/// data, both of which have no manifest by their ordinary shape: treating
/// either as unreadable aborted the whole sweep on every machine that had one.
fn declared_closures(k: &mut dyn Kernel, journal: &mut Journal, volume: &str) -> Option<Vec<Reference>> {
    let unknown = match closure::declared(&mut Card(&mut *k), volume) {
        Ok(applications) => {
            return Some(
                applications
                    .into_iter()
                    .flat_map(|application| application.library)
                    .collect(),
            );
        }
        Err(unknown) => unknown,
    };

    let (msg, mut fields) = match &unknown {
        closure::Unknown::Unlisted { .. } => (
            "the application directory could not be read, so what each installed application \
             declares is unknown and NO library was removed. Retention keeps one build of a \
             library per product and never evicts one an application names, and it cannot \
             tell which those are without this listing (SPEC-02 §12)",
            Vec::new(),
        ),
        closure::Unknown::Unread { application, .. } => (
            "an installed application has no readable manifest, so what libraries it \
             declares is unknown and NO library was removed on this machine. The \
             application is already not started for the same reason (ADR-015 §3); this \
             is the further consequence, that retention refuses to guess which libraries \
             are load-bearing (SPEC-02 §12)",
            vec![("svc_name".to_string(), Value::Str(application.clone()))],
        ),
        closure::Unknown::Unparsed { application, .. } => (
            "an installed application's manifest could not be read, so what libraries it \
             declares is unknown and NO library was removed on this machine (SPEC-02 §12)",
            vec![("svc_name".to_string(), Value::Str(application.clone()))],
        ),
    };
    fields.push(("path".to_string(), Value::Str(unknown.path().to_string())));
    // For a manifest that did not parse, the fault names a key, never a value.
    fields.push(("detail".to_string(), Value::Str(unknown.detail().to_string())));
    note(k, journal, Severity::Warn, msg, fields);
    None
}

/// **This machine's tree, as `rivet_core::package::closure` reads it** — the
/// supervisor's kernel answering in that port's terms. Directories only,
/// because [`Kernel::list_dir`] answers directories only, which is what makes
/// it the same view discovery has.
///
/// Shared by retention and `crate::uninstall_library_request`, the two places
/// this process removes a library.
pub(crate) struct Card<'k>(pub &'k mut dyn Kernel);

impl closure::Tree for Card<'_> {
    fn directories(&mut self, path: &str) -> Result<Option<Vec<String>>, String> {
        match self.0.list_dir(path) {
            Ok(entries) => Ok(Some(entries)),
            Err(Errno::ENOENT) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn exists(&mut self, path: &str) -> Result<bool, String> {
        self.0.file_exists(path).map_err(|e| e.to_string())
    }

    fn read(&mut self, path: &str) -> Result<String, String> {
        self.0.read_file(path).map_err(|e| e.to_string())
    }
}

/// Every library directory on `volume`, with its age, or nothing.
///
/// `None` for a listing that failed, for [`declared_closures`]' reason in the
/// other direction: a partial view of what is installed would make retention
/// count a product's builds wrongly.
fn held_libraries(k: &mut dyn Kernel, journal: &mut Journal, volume: &str) -> Option<Vec<HeldLibrary>> {
    let root = lib_root(volume);
    let entries = match k.list_dir(&root) {
        Ok(entries) => entries,
        Err(Errno::ENOENT) => return Some(Vec::new()),
        Err(e) => {
            note(
                k,
                journal,
                Severity::Warn,
                "the library directory could not be read, so nothing was removed from it. Every \
                 application's declared closure is unaffected — that is resolved by stat, not by \
                 this listing (SPEC-02 §5)",
                vec![
                    ("path".to_string(), Value::Str(root)),
                    ("detail".to_string(), Value::Str(e.to_string())),
                ],
            );
            return None;
        }
    };

    let mut held = Vec::new();
    for entry in entries {
        // A directory that is not a reference was put there by something other
        // than this project, so it is not a candidate and — more to the point
        // — it is not something this module will remove. The same silence
        // `crate::libraries::discover_installed` gives it.
        let Ok(reference) = Reference::parse(&entry) else {
            continue;
        };
        let added = k.modified_secs(&library_dir(volume, &reference)).unwrap_or(0);
        held.push(HeldLibrary { reference, added });
    }
    Some(held)
}

/// Say something in the supervisor's own voice — `crate::libraries`' helper,
/// restated here for the reason that module restates it.
fn note(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    sev: Severity,
    msg: &str,
    fields: Vec<(String, Value)>,
) {
    let corr = journal.next_correlation();
    journal.emit(k, sev, SELF_NAME, &corr, msg, fields);
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::testkernel::{FakeKernel, Script};
    use rivet_core::service::{self, BUILT_IN_VOLUME, ServiceName};

    fn journal() -> Journal {
        Journal::new("b".to_string(), 1, crate::supervise::ACTOR.to_string())
    }

    fn reference(product: &str, version: &str, digest_byte: u8) -> Reference {
        Reference::compose(product, version, &[digest_byte; 32]).expect("a reference")
    }

    /// A library directory on the fake card, installed at `added`.
    fn install_library(k: &mut FakeKernel, reference: &Reference, added: u64) {
        let dir = library_dir(BUILT_IN_VOLUME, reference);
        k.dirs_mut()
            .entry(lib_root(BUILT_IN_VOLUME))
            .or_default()
            .push(reference.to_string());
        k.dirs_mut().insert(dir.clone(), vec!["lib".to_string()]);
        k.modified_mut().insert(dir, added);
    }

    /// An application on the fake card, declaring `closure`.
    ///
    /// **Rendered by the manifest's own writer rather than hand-written TOML**
    /// — `AGENTS.md` §4, and `crate::applications::tests` says the same thing
    /// at more length: a fixture composed by the reader's own author is the
    /// only kind that proves the reader agrees with itself.
    fn install_application(k: &mut FakeKernel, name: &str, closure: &[&Reference]) {
        k.dirs_mut()
            .entry(service::app_root(BUILT_IN_VOLUME))
            .or_default()
            .push(name.to_string());
        let manifest = rivet_core::package::application::Manifest::new(
            rivet_core::package::application::ProgramKind::Daemon,
        )
        .with_library(closure.iter().map(|r| (*r).clone()).collect())
        .expect("a valid closure");
        let path = service::manifest_path(
            BUILT_IN_VOLUME,
            &ServiceName::new(name).expect("a service name"),
        );
        k.files_mut()
            .insert(path, Script::always(Ok(manifest.render())));
    }

    fn removed(k: &FakeKernel) -> Vec<String> {
        k.removed_paths()
    }

    /// **An eviction answers at the rename, and a helper deletes** —
    /// `ADR-017` §3. The older build is gone from the library root when the
    /// eviction is reported, so the discovery that follows sees one build, and
    /// the loop deleted nothing itself.
    #[test]
    fn an_evicted_build_is_renamed_aside_and_deleted_by_a_helper() {
        let old = reference("libaura-reading-source", "0.1.0", 0xaa);
        let new = reference("libaura-reading-source", "0.1.0", 0xbb);
        let mut k = FakeKernel::new();
        install_library(&mut k, &old, 1_000);
        install_library(&mut k, &new, 2_000);

        sweep(&mut k, &mut journal(), BUILT_IN_VOLUME);

        assert_eq!(
            k.reclaims,
            [(
                SELF_NAME.to_string(),
                lib_root(BUILT_IN_VOLUME),
                rivet_core::service::RECLAIM_MARK.to_string()
            )]
        );
        assert_eq!(
            k.list_dir(&lib_root(BUILT_IN_VOLUME)),
            Ok(vec![new.to_string()]),
            "the evicted build is still listed"
        );
    }

    /// **The case that could not ship**: the same product, built twice.
    #[test]
    fn a_rebuilt_library_supersedes_its_own_products_older_build() {
        let old = reference("libaura-reading-source", "0.1.0", 0xaa);
        let new = reference("libaura-reading-source", "0.1.0", 0xbb);
        let mut k = FakeKernel::new();
        install_library(&mut k, &old, 1_000);
        install_library(&mut k, &new, 2_000);

        sweep(&mut k, &mut journal(), BUILT_IN_VOLUME);

        assert_eq!(
            removed(&k),
            vec![library_dir(BUILT_IN_VOLUME, &old)],
            "the older build goes and the one just installed stays"
        );
    }

    /// **Two products of one class are not an upgrade**, and retention must
    /// leave them exactly where `.rulings/` R-200 wants them: both installed,
    /// with the host refusing to choose. Two `probe-source` libraries stand on
    /// `rivet-06f5c` as the evidence of that refusal, and a retention rule that
    /// tidied one of them away would delete the evidence.
    #[test]
    fn two_different_products_of_one_class_are_both_kept() {
        let one = reference("libprobe-source-one", "0.1.0", 0x11);
        let two = reference("libprobe-source-two", "0.1.0", 0x22);
        let mut k = FakeKernel::new();
        install_library(&mut k, &one, 1_000);
        install_library(&mut k, &two, 2_000);

        sweep(&mut k, &mut journal(), BUILT_IN_VOLUME);

        assert!(
            removed(&k).is_empty(),
            "one build each of two products is the retained state: {:?}",
            removed(&k)
        );
    }

    /// **The guard.** `SPEC-02` §5's closure is a signed statement that an
    /// application needs this exact reference, and retention that evicted it
    /// would break that application at its next start.
    #[test]
    fn a_library_named_in_a_declared_closure_survives_a_newer_build() {
        let declared = reference("libgreet", "1.0.0", 0xcc);
        let newer = reference("libgreet", "2.0.0", 0xdd);
        let mut k = FakeKernel::new();
        install_library(&mut k, &declared, 1_000);
        install_library(&mut k, &newer, 2_000);
        install_application(&mut k, "greeter", &[&declared]);

        sweep(&mut k, &mut journal(), BUILT_IN_VOLUME);

        assert!(
            removed(&k).is_empty(),
            "the declared one is off the table and costs the product's quota nothing, so the \
             newer one keeps its place too: {:?}",
            removed(&k)
        );
    }

    /// **A declared library costs its product's quota nothing**, and the
    /// workflow this protects is `SPEC-02` §5's: a library is installed
    /// *before* the application that declares it, so at the moment it lands
    /// nothing on the machine names it yet.
    ///
    /// A guard that merely held the declared reference back inside the
    /// ordering would let it occupy the product's one place, evict the version
    /// that had just been installed for the application about to arrive, and
    /// then refuse that application for an unsatisfied closure — retention
    /// producing the exact refusal §5.1 exists to raise, against itself.
    #[test]
    fn a_declared_library_does_not_evict_a_second_version_of_its_own_product() {
        let declared = reference("libgreet", "2.0.0", 0xcc);
        let arriving = reference("libgreet", "1.5.0", 0xdd);
        let mut k = FakeKernel::new();
        install_library(&mut k, &declared, 1_000);
        install_library(&mut k, &arriving, 2_000);
        install_application(&mut k, "greeter", &[&declared]);

        sweep(&mut k, &mut journal(), BUILT_IN_VOLUME);

        assert!(
            removed(&k).is_empty(),
            "the declared 2.0.0 is off the table, leaving 1.5.0 as the single candidate against \
             a quota of one: {:?}",
            removed(&k)
        );
    }

    /// Two undeclared builds of a product an application *also* declares: the
    /// guard covers the reference named in the closure and nothing else, so
    /// the two that are merely installed are still retained down to one.
    #[test]
    fn the_guard_covers_the_named_reference_and_not_its_whole_product() {
        let declared = reference("libgreet", "3.0.0", 0xaa);
        let newer = reference("libgreet", "2.0.0", 0xbb);
        let older = reference("libgreet", "1.0.0", 0xcc);
        let mut k = FakeKernel::new();
        install_library(&mut k, &declared, 3_000);
        install_library(&mut k, &newer, 2_000);
        install_library(&mut k, &older, 1_000);
        install_application(&mut k, "greeter", &[&declared]);

        sweep(&mut k, &mut journal(), BUILT_IN_VOLUME);

        assert_eq!(
            removed(&k),
            vec![library_dir(BUILT_IN_VOLUME, &older)],
            "two candidates, the quota is one, and the declared build is neither"
        );
    }

    /// **An application whose manifest cannot be read stops the sweep dead.**
    /// This module's header is the argument, and it is the direction that costs
    /// disk rather than a start.
    #[test]
    fn an_unreadable_application_manifest_evicts_nothing_at_all() {
        let old = reference("libaura-reading-source", "0.1.0", 0xaa);
        let new = reference("libaura-reading-source", "0.1.0", 0xbb);
        let mut k = FakeKernel::new();
        install_library(&mut k, &old, 1_000);
        install_library(&mut k, &new, 2_000);
        // A directory under the application root with no manifest behind it.
        k.dirs_mut()
            .entry(service::app_root(BUILT_IN_VOLUME))
            .or_default()
            .push("half-installed".to_string());

        sweep(&mut k, &mut journal(), BUILT_IN_VOLUME);

        assert!(
            removed(&k).is_empty(),
            "an unknown closure is a guard with a hole in it: {:?}",
            removed(&k)
        );
    }

    /// **A-5's fix.** An image-service home — `rivet-proxyd`'s directory on a
    /// fleet machine, `ADR-060` — has no `current/manifest.toml` by its
    /// ordinary shape, and must not be mistaken for an application whose
    /// manifest failed to read: that mistake aborted the whole sweep, so two
    /// builds of one addon class accumulated on every machine that runs it.
    #[test]
    fn an_image_service_home_with_no_manifest_does_not_block_retention() {
        let old = reference("libaura-reading-source", "0.1.0", 0xaa);
        let new = reference("libaura-reading-source", "0.1.0", 0xbb);
        let mut k = FakeKernel::new();
        install_library(&mut k, &old, 1_000);
        install_library(&mut k, &new, 2_000);
        // `rivet-proxyd`'s home: a directory under the application root,
        // named for a service `service::image_service_home` knows, with no
        // manifest behind it — never a broken install.
        k.dirs_mut()
            .entry(service::app_root(BUILT_IN_VOLUME))
            .or_default()
            .push(service::PROXYD_SERVICE_NAME.to_string());

        sweep(&mut k, &mut journal(), BUILT_IN_VOLUME);

        assert_eq!(
            removed(&k),
            vec![library_dir(BUILT_IN_VOLUME, &old)],
            "an image-service home with no manifest stopped retention from running at all: {:?}",
            removed(&k)
        );
    }

    /// **The same for an image service with no home**: the compositor's
    /// capture directory under `<volume>/app` has no manifest either, and must
    /// not stop the sweep on every machine with a screen.
    #[test]
    fn the_compositors_capture_directory_does_not_block_retention() {
        let old = reference("libaura-reading-source", "0.1.0", 0xaa);
        let new = reference("libaura-reading-source", "0.1.0", 0xbb);
        let mut k = FakeKernel::new();
        install_library(&mut k, &old, 1_000);
        install_library(&mut k, &new, 2_000);
        k.dirs_mut()
            .entry(service::app_root(BUILT_IN_VOLUME))
            .or_default()
            .push(rivet_core::compositor::PROVIDER.to_string());

        sweep(&mut k, &mut journal(), BUILT_IN_VOLUME);

        assert_eq!(
            removed(&k),
            vec![library_dir(BUILT_IN_VOLUME, &old)],
            "the compositor's capture directory stopped retention from running at all: {:?}",
            removed(&k)
        );
    }

    /// **An application uninstalled with its data kept** leaves
    /// `<vol>/app/<name>` holding `data` alone, and perhaps the set-aside
    /// versions its removal set aside for a helper to delete. That is an application with no
    /// versions, which declares nothing, and it must not stop the sweep.
    #[test]
    fn an_application_uninstalled_with_its_data_kept_does_not_block_retention() {
        for leftover in [
            vec!["data".to_string()],
            vec![
                "data".to_string(),
                format!("0.1.0{}1700-1", rivet_core::service::RECLAIM_MARK),
            ],
        ] {
            let old = reference("libaura-reading-source", "0.1.0", 0xaa);
            let new = reference("libaura-reading-source", "0.1.0", 0xbb);
            let mut k = FakeKernel::new();
            install_library(&mut k, &old, 1_000);
            install_library(&mut k, &new, 2_000);
            k.dirs_mut()
                .entry(service::app_root(BUILT_IN_VOLUME))
                .or_default()
                .push("photo-index".to_string());
            k.dirs_mut().insert(
                service::app_dir(BUILT_IN_VOLUME, &ServiceName::new("photo-index").expect("a name")),
                leftover.clone(),
            );

            sweep(&mut k, &mut journal(), BUILT_IN_VOLUME);

            assert_eq!(
                removed(&k),
                vec![library_dir(BUILT_IN_VOLUME, &old)],
                "a data-only directory {leftover:?} stopped retention: {:?}",
                k.records
            );
        }
    }

    /// The exemption is for a directory with no versions in it. One holding a
    /// version but no readable manifest is still an unknown closure.
    #[test]
    fn a_version_directory_without_a_manifest_still_blocks_retention() {
        let old = reference("libaura-reading-source", "0.1.0", 0xaa);
        let new = reference("libaura-reading-source", "0.1.0", 0xbb);
        let mut k = FakeKernel::new();
        install_library(&mut k, &old, 1_000);
        install_library(&mut k, &new, 2_000);
        k.dirs_mut()
            .entry(service::app_root(BUILT_IN_VOLUME))
            .or_default()
            .push("photo-index".to_string());
        k.dirs_mut().insert(
            service::app_dir(BUILT_IN_VOLUME, &ServiceName::new("photo-index").expect("a name")),
            vec!["data".to_string(), "0.1.0".to_string()],
        );

        sweep(&mut k, &mut journal(), BUILT_IN_VOLUME);

        assert!(removed(&k).is_empty(), "{:?}", removed(&k));
    }

    /// A machine with no libraries at all is the ordinary case and not a fault.
    #[test]
    fn no_library_directory_is_not_a_fault_and_removes_nothing() {
        let mut k = FakeKernel::new();
        sweep(&mut k, &mut journal(), BUILT_IN_VOLUME);
        assert!(removed(&k).is_empty());
    }
}
