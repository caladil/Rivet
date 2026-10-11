//! **What is installed under `<volume>/lib`, for addon classes.**
//! `PLAN-Feature-Set-2026-09-12` §6.2, `ADR-053`.
//!
//! This is the one place `rivet-sysd` lists `/state/lib` at all. Every other
//! reader of a library — `rivet_core::service::library_path`, an
//! application's own `env` — resolves a *declared* reference by `stat`,
//! never by listing (`SPEC-02` §5): a reference *is* a directory name, so
//! satisfying an explicit closure needs no search. An addon class is the
//! opposite question — *which installed libraries, whatever their name,
//! declared this class* — and that can only be answered by listing.
//!
//! # What this does NOT decide
//!
//! Whether a library's class **matches** an application's is
//! `rivet_core::service::addon_library_path`'s question, answered with no
//! I/O at all. This module only reports what is on the filesystem; the core
//! decides what it means, which is `AGENTS.md` §3's ports-and-adapters split
//! applied to a directory listing rather than a socket.
//!
//! # Why a fault here never stops the boot
//!
//! The same argument `crate::applications` makes about `/state/app`: a
//! machine that is reachable is one somebody can repair. An unreadable
//! `/state/lib` or one unreadable library manifest means fewer addons join,
//! never that discovery stops or that an application already running is
//! touched.

use rivet_core::package::install::lib_root;
use rivet_core::package::library::{self, Reference};
use rivet_core::service::InstalledLibrary;

use crate::journal::{Journal, Severity, Value};
use crate::kernel::{Errno, Kernel};
use crate::supervise::SELF_NAME;

/// Every library installed on `volume`, with the class its own manifest
/// provides, if it provides one.
///
/// **Nothing here decides whether a class is useful to anybody** — see this
/// module's own documentation. A library whose manifest cannot be read is
/// simply absent from the result, exactly as an application with an
/// unreadable manifest is absent from `crate::applications::discover`'s.
pub fn discover_installed(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    volume: &str,
) -> Vec<InstalledLibrary> {
    let root = lib_root(volume);

    let entries = match k.list_dir(&root) {
        Ok(entries) => entries,
        Err(Errno::ENOENT) => {
            // **Not a fault, and the commonest case today.** No machine has
            // ever had a library installed by class before `ADR-053`, and
            // most never will. Said at `Info` for `crate::applications::
            // discover`'s reason: nothing is wrong and nothing is degraded.
            note(
                k,
                journal,
                Severity::Info,
                "no libraries are installed on this machine, so no application declaring an \
                 accepted class can find one to join (PLAN-Feature-Set-2026-09-12 §6.2)",
                vec![("path".to_string(), Value::Str(root))],
            );
            return Vec::new();
        }
        Err(e) => {
            note(
                k,
                journal,
                Severity::Error,
                "DEGRADED the library directory could not be read, so no installed library can \
                 be found for an application declaring an accepted class. Every application's \
                 explicit `library` closure is unaffected — that is resolved by stat, not by \
                 this listing (SPEC-02 §5). Carrying on rather than stopping: the fault is on \
                 /state, which no update or rollback touches (ADR-018 §8)",
                vec![
                    ("path".to_string(), Value::Str(root)),
                    ("detail".to_string(), Value::Str(e.to_string())),
                ],
            );
            return Vec::new();
        }
    };

    // Sorted, for the reason `crate::applications::discover` sorts its own
    // listing: a result that depended on readdir order would be a machine
    // whose addon joins could differ between two boots of the same state.
    let mut entries = entries;
    entries.sort_unstable();

    let mut out = Vec::new();
    for entry in entries {
        // **A directory that is not a reference is not this build's
        // problem.** Every directory this project ever writes under
        // `<volume>/lib` is named by `Reference::compose` (`SPEC-02` §3), so
        // one that will not parse was put there by something else, and it is
        // simply not a candidate — the same silence `crate::applications::
        // discover` gives an application directory whose name is not a
        // `ServiceName`, restated for a reference instead.
        let Ok(reference) = Reference::parse(&entry) else {
            continue;
        };

        // **An interrupted install is not a candidate, quietly, same as one
        // with no manifest at all.** `rivet_core::package::install`'s plan
        // writes the retained header LAST, after the manifest and every
        // object, so a directory without it is a library that has not
        // finished installing — its manifest may parse, its objects may be
        // there or not, and none of that is settled yet. Checked before the
        // manifest is even read, for the same reason a missing manifest is
        // silent: this is the ordinary shape of a retry in progress, not a
        // fault worth a journal line on every discovery pass.
        //
        // **Asked as existence, never as a read.** The header is the signed
        // package header, which is binary: `Kernel::read_file` returns a
        // `String` and refuses every real one, which would skip every
        // installed library and join none to any application. Existence is
        // also the question the installer itself asks of this path.
        if !matches!(
            k.file_exists(&rivet_core::package::install::library_header_path(
                volume, &reference,
            )),
            Ok(true)
        ) {
            continue;
        }

        let manifest_path = format!(
            "{}/{}",
            rivet_core::package::install::library_dir(volume, &reference),
            library::MANIFEST_NAME
        );
        let text = match k.read_file(&manifest_path) {
            Ok(text) => text,
            // No manifest, or one this process cannot read: this library
            // simply declares no class as far as this boot can tell. Not
            // logged at every discovery pass — see this function's own
            // ENOENT case above for why an ordinary absence stays quiet;
            // an application's explicit closure still resolves this library
            // by `stat` regardless of whether its manifest can be read here.
            Err(_) => continue,
        };

        let manifest = match library::Manifest::parse(text.as_bytes()) {
            Ok(manifest) => manifest,
            Err(fault) => {
                note(
                    k,
                    journal,
                    Severity::Warn,
                    "an installed library's manifest could not be read, so it cannot be matched \
                     to any application by class. Any application naming it in an explicit \
                     `library` closure is unaffected, because that is resolved by stat and never \
                     by reading this file (SPEC-02 §5)",
                    vec![
                        ("reference".to_string(), Value::Str(reference.to_string())),
                        ("path".to_string(), Value::Str(manifest_path)),
                        ("detail".to_string(), Value::Str(fault.to_string())),
                    ],
                );
                continue;
            }
        };

        // **The object, taken in the same pass as the class** —
        // `PLAN-Addon-Feature-Set-2026-09-19` T2. Until 2026-09-19 a host
        // derived the filename from the class, so the reference alone was
        // enough and this key was never read here; the enumeration channel
        // names a file, so it is read now, out of the manifest already parsed
        // rather than by a second look at the same artifact.
        //
        // **The object the library itself names**, `.rulings/` R-201 and
        // `SPEC-02` §11 — asked of the manifest rather than picked out of
        // its `soname` list here, because the rule has three parts and this
        // is not the place any of them should be re-derived. A library that
        // carries one object names none and answers with it anyway; one that
        // carries several and named none never parsed.
        let soname = manifest.addon_object().map(str::to_string);
        if soname.is_none() && manifest.provides_class().is_some() {
            // **Info, not a fault, and it costs this library nothing but the
            // naming.** Its directory still joins `LD_LIBRARY_PATH` and the
            // ruleset, because those are about a directory; only the
            // enumeration needs a filename. Said once per pass so that an
            // operator whose host reports no addon has the sentence that
            // explains it rather than a silence.
            //
            // **Unreachable through this build's own grammar**, and written
            // anyway: `Manifest::parse` refuses a library that provides a
            // class without an addon object, so an artifact that reaches this
            // branch came from a grammar wider than the one this binary
            // holds. The alternative to a sentence is a panic in the
            // supervisor on the day that happens — `AGENTS.md` §5's *refuse
            // rather than guess*, in its quieter form.
            note(
                k,
                journal,
                Severity::Info,
                "an installed library declares a class and no addon object, so it is not named \
                 to any host that accepts its class. Its directory still joins LD_LIBRARY_PATH \
                 and the Landlock ruleset — only the enumeration needs a filename \
                 (.rulings/ R-201, SPEC-02 §11)",
                vec![("reference".to_string(), Value::Str(reference.to_string()))],
            );
        }

        out.push(InstalledLibrary {
            reference,
            class: manifest.provides_class().map(str::to_string),
            soname,
        });
    }
    out
}

/// Say something in the supervisor's own voice — `crate::applications`'
/// own helper, restated here for the same reason `crate::document` and
/// `crate::etc` keep their own copy.
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
    use rivet_core::service::BUILT_IN_VOLUME;

    fn journal() -> Journal {
        Journal::new("b".to_string(), 1, crate::supervise::ACTOR.to_string())
    }

    fn reference(name: &str, digest_byte: u8) -> Reference {
        Reference::parse(&format!(
            "{name}-1.0.0-{}",
            format!("{digest_byte:02x}").repeat(32)
        ))
        .expect("a valid fixture reference")
    }

    fn install_library(k: &mut FakeKernel, name: &str, digest_byte: u8, class: Option<&str>) {
        let reference = reference(name, digest_byte);
        let root = lib_root(BUILT_IN_VOLUME);
        k.dirs_mut()
            .entry(root)
            .or_default()
            .push(reference.to_string());

        let manifest = library::Manifest::new(vec![format!("{name}.so.1")])
            .expect("valid soname")
            .with_addon(class.map(ToString::to_string), None)
            .expect("valid class");
        k.files_mut().insert(
            format!(
                "{}/{}",
                rivet_core::package::install::library_dir(BUILT_IN_VOLUME, &reference),
                library::MANIFEST_NAME
            ),
            Script::always(Ok(manifest.render())),
        );
        mark_finished(k, &reference);
    }

    /// Put the retained header at `reference`'s directory, as the installer
    /// leaves it: [`real_header`]'s bytes, which are binary. Every fixture in
    /// this module that means to describe a FINISHED install calls this; one
    /// that means to describe an install still in progress does not.
    fn mark_finished(k: &mut FakeKernel, reference: &Reference) {
        let path = rivet_core::package::install::library_header_path(BUILT_IN_VOLUME, reference);
        k.blobs.insert(path.clone(), real_header());
        k.exists_mut().insert(path, Script::always(Ok(true)));
    }

    /// A retained header as the installer writes one: the first
    /// `HEADER_BYTES` of a package built and signed by `rivet_core`'s own
    /// writer, through `crate::verify::fixture`. It is an application's
    /// header rather than a library's, which discovery cannot tell apart and
    /// does not need to — what this fixture must carry is that a real header
    /// is binary, never text.
    fn real_header() -> Vec<u8> {
        let (whole, _) = crate::verify::fixture::signed_application(
            "fixture-library",
            crate::verify::fixture::seed(7),
        );
        whole[..rivet_core::package::format::HEADER_BYTES].to_vec()
    }

    /// No `/state/lib` at all is the ordinary case, not a fault.
    #[test]
    fn no_library_directory_is_not_a_fault_and_yields_nothing() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        let found = discover_installed(&mut k, &mut j, BUILT_IN_VOLUME);
        assert!(found.is_empty());
    }

    /// **The whole of what this module reports.** A library's reference and
    /// the class its own manifest provides, read back exactly.
    #[test]
    fn an_installed_librarys_class_is_read_from_its_own_manifest() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install_library(&mut k, "libtemp", 0x11, Some("temperature-sensor"));

        let found = discover_installed(&mut k, &mut j, BUILT_IN_VOLUME);
        assert_eq!(found.len(), 1, "{:?}", k.records);
        assert_eq!(found[0].reference, reference("libtemp", 0x11));
        assert_eq!(found[0].class.as_deref(), Some("temperature-sensor"));
    }

    /// **The object's name comes back with the class, from the same pass.**
    /// `PLAN-Addon-Feature-Set-2026-09-19` T2: the enumeration channel names a
    /// file, and the filename is in the manifest this function has already
    /// parsed for the class — so taking it anywhere else would mean reading
    /// one artifact twice to learn two things about it.
    #[test]
    fn an_installed_librarys_object_is_read_from_the_same_manifest_as_its_class() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install_library(&mut k, "libtemp", 0x11, Some("temperature-sensor"));

        let found = discover_installed(&mut k, &mut j, BUILT_IN_VOLUME);
        assert_eq!(found.len(), 1, "{:?}", k.records);
        assert_eq!(found[0].soname.as_deref(), Some("libtemp.so.1"));
    }

    /// A library that carries several objects is joined by **the one it
    /// names**, `.rulings/` R-201 — and the fixture names the second of them
    /// on purpose, because a reader taking the first would pass a fixture
    /// that named the first and prove nothing.
    #[test]
    fn a_library_of_several_objects_is_named_by_the_one_it_declares() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        let reference = reference("libpack", 0x44);
        k.dirs_mut()
            .entry(lib_root(BUILT_IN_VOLUME))
            .or_default()
            .push(reference.to_string());
        let manifest = library::Manifest::new(vec![
            "libpack-private.so.1".to_string(),
            "libpack.so.1".to_string(),
        ])
        .expect("valid sonames")
        .with_addon(
            Some("aura-background-pack".to_string()),
            Some("libpack.so.1".to_string()),
        )
        .expect("valid class and addon object");
        k.files_mut().insert(
            format!(
                "{}/{}",
                rivet_core::package::install::library_dir(BUILT_IN_VOLUME, &reference),
                library::MANIFEST_NAME
            ),
            Script::always(Ok(manifest.render())),
        );
        mark_finished(&mut k, &reference);

        let found = discover_installed(&mut k, &mut j, BUILT_IN_VOLUME);
        assert_eq!(found.len(), 1, "{:?}", k.records);
        assert_eq!(found[0].soname.as_deref(), Some("libpack.so.1"));
    }

    /// **A library that carries several objects and names none never reaches
    /// a board**, `.rulings/` R-201 — but if one did, this is the supervisor
    /// half of the same rule: the manifest does not parse, so the library is
    /// reported with the warning every unreadable manifest gets rather than
    /// joined to a host by a guess. The text is hand-written here because
    /// `Manifest` will not build one.
    #[test]
    fn a_library_of_several_objects_naming_none_does_not_parse_and_so_does_not_join() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        let reference = reference("libpack", 0x55);
        k.dirs_mut()
            .entry(lib_root(BUILT_IN_VOLUME))
            .or_default()
            .push(reference.to_string());
        k.files_mut().insert(
            format!(
                "{}/{}",
                rivet_core::package::install::library_dir(BUILT_IN_VOLUME, &reference),
                library::MANIFEST_NAME
            ),
            Script::always(Ok("kind = \"library\"\n\
                 soname = [\"libpack.so.1\", \"libpack-private.so.1\"]\n\
                 needs = []\n\
                 provides-class = \"aura-background-pack\"\n"
                .to_string())),
        );
        // A finished install with a manifest that will not parse — this
        // test is about THAT failure, not about an install still in
        // progress, so the header is present exactly as a real one would be.
        mark_finished(&mut k, &reference);

        let found = discover_installed(&mut k, &mut j, BUILT_IN_VOLUME);
        assert!(found.is_empty(), "{found:?}");
        assert!(
            k.records
                .iter()
                .any(|r| r.msg.contains("manifest could not be read")),
            "{:?}",
            k.records
        );
    }

    /// A library that declares no class is still reported — WITH no class —
    /// so that `rivet_core::service::addon_library_path` is where "a library
    /// with no class is not joined" is decided, once, rather than twice.
    #[test]
    fn a_library_with_no_declared_class_is_reported_with_none() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install_library(&mut k, "libplain", 0x22, None);

        let found = discover_installed(&mut k, &mut j, BUILT_IN_VOLUME);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].class, None);
    }

    /// A directory under `/state/lib` that is not a reference at all is
    /// skipped rather than refused — it was not put there by this build.
    #[test]
    fn an_entry_that_is_not_a_reference_is_skipped() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        k.dirs_mut()
            .entry(lib_root(BUILT_IN_VOLUME))
            .or_default()
            .push("lost+found".to_string());

        let found = discover_installed(&mut k, &mut j, BUILT_IN_VOLUME);
        assert!(found.is_empty());
    }

    /// **The completeness rule matches the installer's own.** A directory
    /// whose manifest reads back cleanly but whose retained header was never
    /// written is an install still in progress, and is skipped exactly as a
    /// directory with no manifest at all is — quietly, without a journal
    /// line, because it is the ordinary shape of a retry in flight rather
    /// than a fault. A complete library beside it is unaffected: the check
    /// is per directory, not a reason for the whole pass to stop early.
    #[test]
    fn an_interrupted_library_is_skipped_and_a_complete_one_beside_it_is_not() {
        let mut k = FakeKernel::new();
        let mut j = journal();

        // Interrupted: the directory and its manifest are there, the
        // retained header is not — what an install stopped between the
        // manifest and the header, or between two objects, leaves behind.
        let interrupted = reference("libhalf", 0x66);
        k.dirs_mut()
            .entry(lib_root(BUILT_IN_VOLUME))
            .or_default()
            .push(interrupted.to_string());
        let manifest = library::Manifest::new(vec!["libhalf.so.1".to_string()])
            .expect("valid soname")
            .with_addon(Some("temperature-sensor".to_string()), None)
            .expect("valid class");
        k.files_mut().insert(
            format!(
                "{}/{}",
                rivet_core::package::install::library_dir(BUILT_IN_VOLUME, &interrupted),
                library::MANIFEST_NAME
            ),
            Script::always(Ok(manifest.render())),
        );

        // Finished, beside it.
        install_library(&mut k, "libwhole", 0x77, Some("temperature-sensor"));

        let found = discover_installed(&mut k, &mut j, BUILT_IN_VOLUME);
        assert_eq!(found.len(), 1, "{:?}", found);
        assert_eq!(found[0].reference, reference("libwhole", 0x77));
        assert!(
            k.records.is_empty(),
            "an interrupted install logged something on an ordinary pass: {:?}",
            k.records
        );
    }

    /// **A real retained header is binary, and discovery still offers the
    /// library.** The header is the signed package header the installer
    /// keeps; a check that asks whether it reads as text refuses every real
    /// one, so no library joins any application at all. This also pins the
    /// fixture's premise: if [`real_header`] ever read as text, the other
    /// tests in this module would stop describing a real install.
    #[test]
    fn a_finished_librarys_binary_header_is_enough_to_offer_it() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install_library(&mut k, "libbin", 0x55, Some("temperature-sensor"));

        assert!(
            std::str::from_utf8(&real_header()).is_err(),
            "the fixture's header reads as text, so it no longer models a real one"
        );

        let found = discover_installed(&mut k, &mut j, BUILT_IN_VOLUME);
        assert_eq!(found.len(), 1, "{:?}", k.records);
        assert_eq!(found[0].reference, reference("libbin", 0x55));
    }
}
