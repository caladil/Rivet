// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use super::*;

/// A fresh, empty directory nothing else is using — `fleet_tests.rs`'s own
/// pattern, copied rather than shared: two crates' worth of tests naming one
/// helper would be `ADR-011` with extra steps for thirty lines.
fn scratch(label: &str) -> PathBuf {
    crate::scratch::dir(&format!("fleet-drift-test-{label}"))
}

/// A fleet repository's manifest naming one published `application` artifact
/// — hand-written rather than composed through `fleet::publish`, because
/// these tests are about the COMPARISON and `fleet_tests.rs` already covers
/// how an artifact gets into the repository.
fn manifest_naming(root: &Path, product: &str, version: &str) {
    std::fs::write(
        root.join("manifest.toml"),
        format!(
            "[[artifact]]\n\
             digest  = \"{}\"\n\
             product = \"{product}\"\n\
             version = \"{version}\"\n\
             arch    = \"aarch64\"\n\
             kind    = \"application\"\n\
             size    = 1\n\
             added   = \"2026-09-09T00:00:00Z\"\n",
            "0".repeat(64),
        ),
    )
    .expect("write a manifest");
}

/// A `get status` answer naming exactly `names`, and nothing else this module
/// reads — every other field `Status::value` would carry is left out, because
/// [`service_names`] reads only `"name"` and a fixture that pretended to be
/// complete would be a second, unmaintained copy of `status.rs`'s own shape.
fn status_answer(names: &[&str]) -> String {
    let services: String = names
        .iter()
        .map(|n| {
            format!(
                r#"{{"name":"{n}","enabled":true,"state":"started","pid":100,"restarts":0,"ready":true,"health":"unknown","critical":false}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"subject":"status","phase":"converging","health":"healthy","mode":"headless","services":[{services}]}}"#
    )
}

/// A `get file` answer for an activation symlink — `path` present, `type`
/// `symlink`, and `link_target` the version it names.
fn symlink_answer(path: &str, target: &str) -> String {
    format!(
        r#"{{"subject":"file","path":"{path}","present":true,"type":"symlink","link_target":"{target}","mount_point":false}}"#
    )
}

/// A `get file` answer for a path with nothing at it — `ADR-006` §8.10.4's
/// own honest reading, and every image-defined service's `/state/app/<name>`.
fn absent_answer(path: &str) -> String {
    format!(r#"{{"subject":"file","path":"{path}","present":false}}"#)
}

/// **A fake board, at the [`CallsTools`] boundary** — `rivet_remote::tls`'s
/// own `FakeMcp` pattern, one layer higher: this fakes the ANSWER a real
/// `Tls` would already have decoded out of the wire, because that decoding is
/// what `tls_tests.rs` exists to prove and this module's own tests are about
/// what happens to the answer once it is text.
struct FakeMachine {
    /// What `get status` answers. `Err` is what an unreachable machine or a
    /// dropped connection looks like to a caller of [`CallsTools`].
    status: Result<String, String>,
    /// What `get file <path>` answers, keyed by the exact path asked for.
    /// A path not in here is a test author's mistake, not a machine's honest
    /// `present: false` — [`Self::call_tool`] says so rather than defaulting.
    files: HashMap<String, Result<String, String>>,
    /// Every call this was asked, in order — read by
    /// `every_service_is_asked_about_its_own_directory_and_nothing_else`.
    calls: RefCell<Vec<String>>,
}

impl FakeMachine {
    /// A machine with no `/state/lib` — the honest state of one that holds no
    /// shared rivet. A test about libraries replaces it with
    /// [`Self::with_file`]; every test about services is then unchanged by the
    /// libraries half of the comparison existing.
    fn new(status: Result<String, String>) -> Self {
        let mut files = HashMap::new();
        files.insert(LIBRARY_ROOT.to_string(), Ok(absent_answer(LIBRARY_ROOT)));
        Self {
            status,
            files,
            calls: RefCell::new(Vec::new()),
        }
    }

    fn with_file(mut self, path: &str, answer: Result<String, String>) -> Self {
        self.files.insert(path.to_string(), answer);
        self
    }
}

impl CallsTools for FakeMachine {
    fn call_tool(&self, _tool: &str, arguments: &str) -> Result<String, String> {
        self.calls.borrow_mut().push(arguments.to_string());
        if arguments.contains(r#""subject":"status""#) {
            return self.status.clone();
        }
        // **The path is found by CONTENT, not by the request's JSON key.**
        // `rivet_core::operation::get::OPERATION` names `file`'s path
        // position `"boot"` — `get.rs`'s own doc comment calls it "§8.5.1's
        // polymorphic positional input, paid again", reused across `log`,
        // `file` and `diff` rather than given a fresh name per subject. This
        // fake reads the path the same way `crate::fleet_drift` reads a
        // machine's ANSWER, by what the value actually says, so it does not
        // have to know that wire detail to pretend to be one.
        let path = self
            .files
            .keys()
            .find(|known| arguments.contains(known.as_str()))
            .cloned();
        match path {
            Some(path) => self.files.get(&path).cloned().unwrap_or_else(|| {
                Err(format!("FakeMachine was never told what to answer for {path}"))
            }),
            None => Err(format!("FakeMachine does not recognise this call: {arguments}")),
        }
    }
}

// ---------------------------------------------------------------------------
// matches — the honest positive case
// ---------------------------------------------------------------------------

#[test]
fn a_machine_running_what_the_repository_names_reports_no_drift() {
    let root = scratch("matches");
    manifest_naming(&root, "tempmon-rivet", "0.1.0");
    let machine = FakeMachine::new(Ok(status_answer(&["tempmon-rivet"]))).with_file(
        "/state/app/tempmon-rivet/current",
        Ok(symlink_answer("/state/app/tempmon-rivet/current", "0.1.0")),
    );

    let report = compare(&root, &machine).expect("a real answer");
    assert!(report.contains("\nMATCHES 1"), "{report}");
    assert!(report.contains("tempmon-rivet 0.1.0"), "{report}");
    assert!(report.contains("\nNOT IN THE FLEET REPOSITORY 0"), "{report}");
    assert!(report.contains("\nDIFFERS 0"), "{report}");
    assert!(report.contains("\nNOT COMPARABLE 0"), "{report}");
    std::fs::remove_dir_all(&root).ok();
}

// ---------------------------------------------------------------------------
// each kind of difference
// ---------------------------------------------------------------------------

/// `ADR-052` §1's own motivating symptom, read back off a fake machine: an
/// artifact is running and the repository carries no entry for the product.
#[test]
fn an_artifact_the_repository_has_never_published_is_named_by_the_adrs_own_symptom() {
    let root = scratch("not-in-repo");
    // No `manifest_naming` call: an empty repository, nothing ever published.
    let machine = FakeMachine::new(Ok(status_answer(&["reliquary-rivet"]))).with_file(
        "/state/app/reliquary-rivet/current",
        Ok(symlink_answer(
            "/state/app/reliquary-rivet/current",
            "0.1.1-g598efb9-dirty-c563f94f",
        )),
    );

    let report = compare(&root, &machine).expect("a real answer");
    assert!(report.contains("\nNOT IN THE FLEET REPOSITORY 1"), "{report}");
    assert!(
        report.contains("reliquary-rivet 0.1.1-g598efb9-dirty-c563f94f"),
        "{report}"
    );
    assert!(report.contains("\nMATCHES 0"), "{report}");
    std::fs::remove_dir_all(&root).ok();
}

/// The repository knows the product and not this exact version — a redeploy
/// the repository was never told about, or a hand-edit on the board.
#[test]
fn a_version_the_repository_does_not_hold_for_a_known_product_differs() {
    let root = scratch("differs");
    manifest_naming(&root, "tempmon-rivet", "0.1.0");
    let machine = FakeMachine::new(Ok(status_answer(&["tempmon-rivet"]))).with_file(
        "/state/app/tempmon-rivet/current",
        Ok(symlink_answer("/state/app/tempmon-rivet/current", "0.2.0")),
    );

    let report = compare(&root, &machine).expect("a real answer");
    assert!(report.contains("\nDIFFERS 1"), "{report}");
    assert!(
        report.contains("tempmon-rivet runs 0.2.0; the repository holds 0.1.0"),
        "{report}"
    );
    assert!(report.contains("\nMATCHES 0"), "{report}");
    assert!(report.contains("\nNOT IN THE FLEET REPOSITORY 0"), "{report}");
    std::fs::remove_dir_all(&root).ok();
}

/// **The thing that cannot be compared says so, rather than reporting a
/// difference.** An image-defined service has no `/state/app/<name>` at all,
/// so this repository holds no opinion about its version — and that must
/// read differently from "installed and wrong".
#[test]
fn an_image_defined_service_with_no_application_directory_is_not_comparable() {
    let root = scratch("not-comparable");
    let machine = FakeMachine::new(Ok(status_answer(&["rivet-netd"]))).with_file(
        "/state/app/rivet-netd/current",
        Ok(absent_answer("/state/app/rivet-netd/current")),
    );

    let report = compare(&root, &machine).expect("a real answer");
    assert!(report.contains("\nNOT COMPARABLE 1"), "{report}");
    assert!(report.contains("  rivet-netd\n"), "{report}");
    assert!(report.contains("\nMATCHES 0"), "{report}");
    assert!(report.contains("\nDIFFERS 0"), "{report}");
    assert!(report.contains("\nNOT IN THE FLEET REPOSITORY 0"), "{report}");
    std::fs::remove_dir_all(&root).ok();
}

/// **`present: true` but not a symlink** reads the same way as absent — there
/// is no version to read, and this module does not guess one out of a shape
/// `ADR-015` §7's installer never produces.
#[test]
fn a_present_path_that_is_not_a_symlink_is_not_comparable_either() {
    let root = scratch("not-a-symlink");
    let machine = FakeMachine::new(Ok(status_answer(&["odd-service"]))).with_file(
        "/state/app/odd-service/current",
        Ok(r#"{"subject":"file","path":"/state/app/odd-service/current","present":true,"type":"directory","entry_count":0,"entries":[],"entries_truncated":false}"#.to_string()),
    );

    let report = compare(&root, &machine).expect("a real answer");
    assert!(report.contains("\nNOT COMPARABLE 1"), "{report}");
    std::fs::remove_dir_all(&root).ok();
}

// ---------------------------------------------------------------------------
// a machine that does not answer is not drift
// ---------------------------------------------------------------------------

/// **The distinction this module exists to keep straight.** A refusal to
/// connect must not read like any of the successful reports above — no
/// section headers, because nothing was compared at all.
#[test]
fn a_machine_that_does_not_answer_reads_differently_from_one_that_drifted() {
    let root = scratch("unreachable");
    let machine = FakeMachine::new(Err("connection refused".to_string()));

    let refused = compare(&root, &machine).expect_err("no status, no comparison");
    assert!(refused.contains("did not answer"), "{refused}");
    assert!(refused.contains("NOT DRIFT"), "{refused}");
    assert!(!refused.contains("MATCHES"), "{refused}");
    assert!(!refused.contains("DIFFERS"), "{refused}");
    std::fs::remove_dir_all(&root).ok();
}

/// **A narrower failure than the machine being unreachable**: `get status`
/// answered and one service's `get file` did not. That is reported against
/// the one service, and the rest of the comparison — which DID get an honest
/// answer — is not thrown away because of it.
#[test]
fn a_get_file_failure_for_one_service_does_not_sink_the_rest_of_the_report() {
    let root = scratch("partial-failure");
    manifest_naming(&root, "tempmon-rivet", "0.1.0");
    let machine = FakeMachine::new(Ok(status_answer(&["tempmon-rivet", "notes-rivet"])))
        .with_file(
            "/state/app/tempmon-rivet/current",
            Ok(symlink_answer("/state/app/tempmon-rivet/current", "0.1.0")),
        )
        .with_file(
            "/state/app/notes-rivet/current",
            Err("connection dropped mid-read".to_string()),
        );

    let report = compare(&root, &machine).expect("the rest still answers");
    assert!(report.contains("\nMATCHES 1"), "{report}");
    assert!(report.contains("\nCOULD NOT BE READ 1"), "{report}");
    assert!(
        report.contains(
            "notes-rivet: `get file /state/app/notes-rivet/current` did not answer"
        ),
        "{report}"
    );
    std::fs::remove_dir_all(&root).ok();
}

// ---------------------------------------------------------------------------
// the edges
// ---------------------------------------------------------------------------

/// A machine with no services still answers cleanly — `status.rs`'s own
/// `a_supervisor_with_no_services_still_reports_its_phase`, at this layer.
#[test]
fn a_machine_with_no_services_still_answers_cleanly() {
    let root = scratch("no-services");
    let machine = FakeMachine::new(Ok(status_answer(&[])));
    let report = compare(&root, &machine).expect("an empty machine still answers");
    assert!(report.contains("0 service(s) read"), "{report}");
    assert!(report.contains("\nMATCHES 0"), "{report}");
    std::fs::remove_dir_all(&root).ok();
}

/// **Every service is asked about its OWN directory, and only that one** — a
/// service's name goes straight into the path this module asks `get file`
/// about, so a mix-up would ask the wrong question about the right service or
/// the right question about the wrong one.
#[test]
fn every_service_is_asked_about_its_own_directory_and_nothing_else() {
    let root = scratch("own-directory");
    let machine = FakeMachine::new(Ok(status_answer(&["tempmon-rivet", "notes-rivet"])))
        .with_file(
            "/state/app/tempmon-rivet/current",
            Ok(absent_answer("/state/app/tempmon-rivet/current")),
        )
        .with_file(
            "/state/app/notes-rivet/current",
            Ok(absent_answer("/state/app/notes-rivet/current")),
        );

    let _ = compare(&root, &machine).expect("a real answer");
    let calls = machine.calls.borrow();
    assert!(
        calls.iter().any(|c| c.contains("/state/app/tempmon-rivet/current")),
        "{calls:?}"
    );
    assert!(
        calls.iter().any(|c| c.contains("/state/app/notes-rivet/current")),
        "{calls:?}"
    );
    std::fs::remove_dir_all(&root).ok();
}

// ---------------------------------------------------------------------------
// shared rivets — PLAN-Feature-Set-2026-09-12 §2 decision 7
// ---------------------------------------------------------------------------

/// A `get file` answer for a directory holding `names` — the shape
/// `rivet_core::operation::file` writes, with only the fields this module
/// reads plus the per-entry `type` and `size_bytes` that make a repeated
/// `"type"` key real rather than hypothetical.
fn directory_answer(path: &str, names: &[&str], truncated: bool) -> String {
    let entries: String = names
        .iter()
        .map(|n| format!(r#"{{"name":"{n}","type":"directory","size_bytes":4096}}"#))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"subject":"file","path":"{path}","present":true,"type":"directory","size_bytes":4096,"entry_count":{},"entries":[{entries}],"entries_truncated":{truncated},"mount_point":false}}"#,
        names.len()
    )
}

/// **The reference a machine would name this artifact's directory**, computed
/// here from the file's first `SIGNED_BYTES` with BLAKE3 and formatted by hand
/// — not through `crate::library::reference_of`, which is what the code under
/// test uses, so a defect in that path cannot agree with itself here.
fn machine_reference(artifact: &Path, product: &str, version: &str) -> String {
    use rivet_core::package::port::Digest as _;
    let bytes = std::fs::read(artifact).expect("read the artifact");
    let mut digest = rivet_crypto::Blake3::new();
    digest.start();
    digest.write(&bytes[..rivet_core::package::format::SIGNED_BYTES]);
    let hex: String = digest.finish().iter().map(|b| format!("{b:02x}")).collect();
    format!("{product}-{version}-{hex}")
}

/// A fleet repository with one library really published into it through
/// `fleet::publish`, and the reference a machine holding it would list.
fn repository_with_library(label: &str, product: &str, version: &str, object: &[u8]) -> (PathBuf, String) {
    let root = scratch(label);
    super::super::init(&root).expect("init");
    let (artifact, key) = super::super::tests::library_fixture(
        &scratch(&format!("{label}-fixture")),
        product,
        version,
        object,
        77,
    );
    super::super::publish(&root, &artifact, &key).expect("publish the library");
    (root, machine_reference(&artifact, product, version))
}

/// **The acceptance case: a published library shows up in drift output.**
/// Published for real, listed by a fake machine under the reference its own
/// header yields, and reported as a match — the whole reference, digest and
/// all.
#[test]
fn a_published_library_the_machine_holds_is_a_library_match() {
    let (root, reference) =
        repository_with_library("library-matches", "libgreet", "1.0.0", b"libgreet 1.0.0 object");
    let machine = FakeMachine::new(Ok(status_answer(&[])))
        .with_file(LIBRARY_ROOT, Ok(directory_answer(LIBRARY_ROOT, &[&reference], false)));

    let report = compare(&root, &machine).expect("a real answer");
    assert!(report.contains("\nLIBRARY MATCHES 1"), "{report}");
    assert!(report.contains(&format!("  {reference}\n")), "{report}");
    assert!(report.contains("\nLIBRARY NOT MATCHED 0"), "{report}");
    assert!(report.contains("1 library reference(s) read"), "{report}");
    // The application headings are untouched by a library matching.
    assert!(report.contains("\nMATCHES 0"), "{report}");
    std::fs::remove_dir_all(&root).ok();
}

/// A library nothing ever published — the `ADR-052` §1 symptom, for a shared
/// rivet. An empty repository, so there is no product to call it a version of,
/// and `.rulings/` R-157 R17's hint is a guess only, never printed with
/// nothing to guess from.
#[test]
fn a_library_the_repository_has_never_published_is_named() {
    let root = scratch("library-not-in-repo");
    let reference = format!("libz-1.3.1-{}", "ab".repeat(32));
    let machine = FakeMachine::new(Ok(status_answer(&[])))
        .with_file(LIBRARY_ROOT, Ok(directory_answer(LIBRARY_ROOT, &[&reference], false)));

    let report = compare(&root, &machine).expect("a real answer");
    assert!(report.contains("\nLIBRARY NOT MATCHED 1"), "{report}");
    assert!(report.contains(&format!("  {reference}\n")), "{report}");
    assert!(!report.contains("closest published name"), "{report}");
    assert!(report.contains("\nLIBRARY MATCHES 0"), "{report}");
    std::fs::remove_dir_all(&root).ok();
}

/// **Same name, same version, different bytes** — the dirty-rebuild case, and
/// the one a version comparison alone would call a match. `.rulings/` R-157
/// R17's hint names the closest published product, labelled as a guess, with
/// the repository's own reference beside it so the two digests can be read.
#[test]
fn a_different_build_of_a_published_library_is_not_matched_and_carries_a_hint() {
    let (root, published) =
        repository_with_library("library-differs", "libgreet", "1.0.0", b"the published build");
    let installed = format!("libgreet-1.0.0-{}", "cd".repeat(32));
    assert_ne!(installed, published, "the fixture must be a different build");
    let machine = FakeMachine::new(Ok(status_answer(&[])))
        .with_file(LIBRARY_ROOT, Ok(directory_answer(LIBRARY_ROOT, &[&installed], false)));

    let report = compare(&root, &machine).expect("a real answer");
    assert!(report.contains("\nLIBRARY NOT MATCHED 1"), "{report}");
    assert!(
        report.contains(&format!(
            "  {installed} — closest published name: libgreet (a guess from the name); \
             repository holds {published}\n"
        )),
        "{report}"
    );
    assert!(report.contains("\nLIBRARY MATCHES 0"), "{report}");
    std::fs::remove_dir_all(&root).ok();
}

/// **The prefix rule's known cost, pinned so a change to it is a visible one.**
/// A reference is `<name>-<version>-<digest>` and a library's version is free
/// text (`cargo rivet library --version` never parses it), so nothing in the
/// string says where the name ends. `libgreet-extra-2.0.0-…` beside a
/// repository holding only `libgreet` therefore reads as a `libgreet` build and
/// carries that hint. Either way it is NOT MATCHED, which is the one verdict
/// the hint never decides (`.rulings/` R-157 R17).
#[test]
fn the_prefix_rule_reads_a_hyphenated_name_as_the_longest_published_product() {
    let (root, published) =
        repository_with_library("library-prefix", "libgreet", "1.0.0", b"libgreet object");
    let installed = format!("libgreet-extra-2.0.0-{}", "ef".repeat(32));
    let machine = FakeMachine::new(Ok(status_answer(&[])))
        .with_file(LIBRARY_ROOT, Ok(directory_answer(LIBRARY_ROOT, &[&installed], false)));

    let report = compare(&root, &machine).expect("a real answer");
    assert!(report.contains("\nLIBRARY NOT MATCHED 1"), "{report}");
    assert!(report.contains("closest published name: libgreet (a guess from the name)"), "{report}");
    assert!(report.contains(&format!("repository holds {published}")), "{report}");
    std::fs::remove_dir_all(&root).ok();
}

/// An entry in `/state/lib` that is not a reference names no artifact: it is
/// listed so an operator can see it, and counted as nothing else.
#[test]
fn an_entry_that_is_not_a_reference_is_named_and_not_compared() {
    let root = scratch("library-not-a-reference");
    let machine = FakeMachine::new(Ok(status_answer(&[])))
        .with_file(LIBRARY_ROOT, Ok(directory_answer(LIBRARY_ROOT, &["lost+found"], false)));

    let report = compare(&root, &machine).expect("a real answer");
    assert!(report.contains("\nLIBRARY DIRECTORY ENTRIES THAT ARE NOT REFERENCES 1"), "{report}");
    assert!(report.contains("  lost+found\n"), "{report}");
    assert!(report.contains("\nLIBRARY NOT MATCHED 0"), "{report}");
    assert!(report.contains("0 library reference(s) read"), "{report}");
    std::fs::remove_dir_all(&root).ok();
}

/// A listing the machine cut says so, rather than letting what it did not name
/// read as absent.
#[test]
fn a_listing_the_machine_cut_says_so() {
    let root = scratch("library-truncated");
    let reference = format!("libz-1.3.1-{}", "01".repeat(32));
    let machine = FakeMachine::new(Ok(status_answer(&[])))
        .with_file(LIBRARY_ROOT, Ok(directory_answer(LIBRARY_ROOT, &[&reference], true)));

    let report = compare(&root, &machine).expect("a real answer");
    assert!(report.contains("\nLIBRARY LISTING CUT BY THE MACHINE"), "{report}");
    std::fs::remove_dir_all(&root).ok();
}

/// A machine with no `/state/lib` holds no library, and that is zero in each
/// heading rather than a failure.
#[test]
fn a_machine_with_no_library_directory_reports_zero_libraries() {
    let root = scratch("library-none");
    let machine = FakeMachine::new(Ok(status_answer(&[])));

    let report = compare(&root, &machine).expect("a real answer");
    assert!(report.contains("\nLIBRARY MATCHES 0"), "{report}");
    assert!(report.contains("0 library reference(s) read"), "{report}");
    assert!(!report.contains("LIBRARIES COULD NOT BE READ"), "{report}");
    std::fs::remove_dir_all(&root).ok();
}

/// The listing failing is reported against the libraries alone; the services
/// beside it, which did answer, are still reported.
#[test]
fn a_library_listing_that_fails_does_not_sink_the_services() {
    let root = scratch("library-unreadable");
    manifest_naming(&root, "tempmon-rivet", "0.1.0");
    let machine = FakeMachine::new(Ok(status_answer(&["tempmon-rivet"])))
        .with_file(
            "/state/app/tempmon-rivet/current",
            Ok(symlink_answer("/state/app/tempmon-rivet/current", "0.1.0")),
        )
        .with_file(LIBRARY_ROOT, Err("connection dropped mid-read".to_string()));

    let report = compare(&root, &machine).expect("the services still answer");
    assert!(report.contains("\nMATCHES 1"), "{report}");
    assert!(report.contains("\nLIBRARIES COULD NOT BE READ"), "{report}");
    assert!(report.contains("connection dropped mid-read"), "{report}");
    assert!(!report.contains("library reference(s) read"), "{report}");
    std::fs::remove_dir_all(&root).ok();
}

/// **A repository naming bytes it does not hold is refused before the machine
/// is asked anything** — skipping the entry would report its library as never
/// published, which is a claim about the machine this workstation caused.
#[test]
fn a_library_whose_object_is_missing_refuses_before_the_machine_is_asked() {
    let (root, _) =
        repository_with_library("library-missing-object", "libgreet", "1.0.0", b"soon gone");
    for object in std::fs::read_dir(root.join(super::super::OBJECTS_DIR)).expect("objects/") {
        std::fs::remove_file(object.expect("an entry").path()).expect("remove the object");
    }
    let machine = FakeMachine::new(Ok(status_answer(&[])));

    let refused = compare(&root, &machine).expect_err("an inconsistent repository");
    assert!(refused.contains("inconsistent"), "{refused}");
    assert!(refused.contains("libgreet"), "{refused}");
    assert!(machine.calls.borrow().is_empty(), "the machine was asked something");
    std::fs::remove_dir_all(&root).ok();
}
