//! The watch over a fake kernel: what it removes, what it refuses, the order it
//! does those in, and the library path every service is given afterwards.
//! Every library, application and request here is **hand-composed** by the
//! test that uses it, through the writers `rivet-core` owns.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::journal::Journal;
use crate::service::{Limits, ServiceDef};
use crate::supervise::ACTOR;
use crate::testkernel::{FakeKernel, Script};
use rivet_core::journal::Record;
use rivet_core::package::application::{Manifest, ProgramKind};
use rivet_core::package::install::{lib_root, library_header_path};
use rivet_core::package::library;
use rivet_core::service::{self, ServiceName};

fn journal() -> Journal {
    Journal::new("b".to_string(), 1, ACTOR.to_string())
}

fn supervisor(defs: Vec<ServiceDef>) -> Supervisor {
    match Supervisor::new(defs, Limits::default(), journal()) {
        Ok(s) => s,
        Err(e) => unreachable!("fixture refused: {e}"),
    }
}

fn build(product: &str, version: &str, byte: u8) -> Reference {
    Reference::compose(product, version, &[byte; 32]).expect("a reference")
}

fn name(text: &str) -> ServiceName {
    ServiceName::new(text).expect("a service name")
}

/// A FINISHED library install on the fake card: its directory, its manifest
/// providing `class`, and the retained header the installer writes last.
fn install_library(k: &mut FakeKernel, reference: &Reference, class: Option<&str>) {
    k.dirs_mut()
        .entry(lib_root(BUILT_IN_VOLUME))
        .or_default()
        .push(reference.to_string());
    k.dirs_mut()
        .insert(library_dir(BUILT_IN_VOLUME, reference), vec!["lib".to_string()]);
    let manifest = library::Manifest::new(vec!["libpack.so.1".to_string()])
        .expect("a soname")
        .with_addon(class.map(ToString::to_string), None)
        .expect("a class");
    k.files_mut().insert(
        format!(
            "{}/{}",
            library_dir(BUILT_IN_VOLUME, reference),
            library::MANIFEST_NAME
        ),
        Script::always(Ok(manifest.render())),
    );
    k.exists_mut().insert(
        library_header_path(BUILT_IN_VOLUME, reference),
        Script::always(Ok(true)),
    );
}

/// An application on the fake card declaring `closure` and accepting
/// `accepts`, its manifest rendered by the manifest's own writer.
fn install_application(k: &mut FakeKernel, app: &str, closure: &[&Reference], accepts: &[&str]) {
    k.dirs_mut()
        .entry(service::app_root(BUILT_IN_VOLUME))
        .or_default()
        .push(app.to_string());
    let manifest = Manifest::new(ProgramKind::Daemon)
        .with_library(closure.iter().map(|r| (*r).clone()).collect())
        .expect("a closure")
        .with_accepts_classes(accepts.iter().map(ToString::to_string).collect())
        .expect("classes");
    k.files_mut().insert(
        service::manifest_path(BUILT_IN_VOLUME, &name(app)),
        Script::always(Ok(manifest.render())),
    );
}

/// The request on the card, in the bytes `cmd/rivet` writes — rendered by the
/// one writer both halves share.
fn request(k: &mut FakeKernel, selector: &str) {
    let text = uninstall_library::render_request(
        &Selector::parse(selector).expect("a selector"),
        &rivet_core::deploy::Principal::no_session(),
    );
    k.sizes_mut()
        .insert(uninstall_library::REQUEST_PATH.to_string(), text.len() as u64);
    k.files_mut().insert(
        uninstall_library::REQUEST_PATH.to_string(),
        Script::always(Ok(text)),
    );
}

fn said<'a>(k: &'a FakeKernel, words: &str) -> Vec<&'a Record> {
    k.records.iter().filter(|r| r.msg.contains(words)).collect()
}

fn text(record: &Record, field: &str) -> String {
    match record.field(field) {
        Some(Value::Str(s)) => s.clone(),
        other => panic!("{field} is {other:?} on {record:?}"),
    }
}

/// **The whole of it, from the outside.** Every build of the product goes,
/// each journalled by name with its reference and path; a build of another
/// product stays; and the request is gone from the card.
#[test]
fn a_request_removes_every_selected_build_and_journals_each_by_name() {
    let mut k = FakeKernel::new();
    let a = build("aura-background-pack", "0.1.0", 0x11);
    let b = build("aura-background-pack", "0.2.0", 0x22);
    let other = build("libgreet", "1.0.0", 0x33);
    install_library(&mut k, &a, Some("aura-background-pack"));
    install_library(&mut k, &b, Some("aura-background-pack"));
    install_library(&mut k, &other, None);
    request(&mut k, "aura-background-pack");
    let mut sup = supervisor(Vec::new());

    Watch::new().poll(&mut k, &mut sup);

    assert_eq!(
        k.removed_paths(),
        vec![library_dir(BUILT_IN_VOLUME, &a), library_dir(BUILT_IN_VOLUME, &b)],
        "{:?}",
        k.records
    );
    let removals = said(&k, "an installed library build was removed");
    assert_eq!(removals.len(), 2, "{:?}", k.records);
    assert_eq!(text(removals[0], "reference"), a.to_string());
    assert_eq!(text(removals[0], "path"), library_dir(BUILT_IN_VOLUME, &a));
    assert_eq!(text(removals[1], "reference"), b.to_string());
    assert_eq!(
        k.list_dir(&lib_root(BUILT_IN_VOLUME)),
        Ok(vec![other.to_string()]),
        "the other product's build was touched"
    );
    assert!(k.removed.contains(&uninstall_library::REQUEST_PATH.to_string()));
    let done = said(&k, "request is done");
    assert_eq!(done.len(), 1, "{:?}", k.records);
    assert_eq!(text(done[0], "not_removed"), "none");
}

/// **The guard, where it governs.** A build an installed application names in
/// its declared closure refuses the whole request — including the build no
/// application names — and the refusal names the application and the build.
#[test]
fn a_build_named_in_a_declared_closure_refuses_the_request_and_removes_nothing() {
    let mut k = FakeKernel::new();
    let old = build("libgreet", "1.0.0", 0x11);
    let new = build("libgreet", "2.0.0", 0x22);
    install_library(&mut k, &old, None);
    install_library(&mut k, &new, None);
    install_application(&mut k, "greeter", &[&old], &[]);
    request(&mut k, "libgreet");
    let mut sup = supervisor(Vec::new());

    Watch::new().poll(&mut k, &mut sup);

    assert!(k.removed_paths().is_empty(), "{:?}", k.removed_paths());
    let refused = said(&k, "REFUSED the library uninstall request");
    assert_eq!(refused.len(), 1, "{:?}", k.records);
    let detail = text(refused[0], "detail");
    assert!(detail.contains("`greeter` declares"), "{detail}");
    assert!(detail.contains(old.as_str()), "{detail}");
}

/// **An application whose manifest cannot be read refuses**: a version
/// directory with no manifest beside it is a closure with a hole in it.
#[test]
fn an_unreadable_application_manifest_refuses_the_request_and_removes_nothing() {
    let mut k = FakeKernel::new();
    let pack = build("aura-background-pack", "0.2.0", 0x22);
    install_library(&mut k, &pack, Some("aura-background-pack"));
    k.dirs_mut()
        .entry(service::app_root(BUILT_IN_VOLUME))
        .or_default()
        .push("half-installed".to_string());
    k.dirs_mut().insert(
        service::app_dir(BUILT_IN_VOLUME, &name("half-installed")),
        vec!["0.1.0".to_string()],
    );
    request(&mut k, "aura-background-pack");
    let mut sup = supervisor(Vec::new());

    Watch::new().poll(&mut k, &mut sup);

    assert!(k.removed_paths().is_empty(), "{:?}", k.removed_paths());
    let refused = said(&k, "REFUSED the library uninstall request");
    assert_eq!(refused.len(), 1, "{:?}", k.records);
    assert!(text(refused[0], "detail").contains("half-installed"));
}

/// **The request is removed before it is acted on.** A request that cannot be
/// taken off the card is not acted on at all, because one that stayed would be
/// acted on again every convergence pass.
#[test]
fn a_request_that_cannot_be_removed_from_the_card_is_not_acted_on() {
    let mut k = FakeKernel::new();
    let pack = build("aura-background-pack", "0.2.0", 0x22);
    install_library(&mut k, &pack, Some("aura-background-pack"));
    request(&mut k, "aura-background-pack");
    k.remove_file_results = Script::always(Err(Errno::EACCES));
    let mut sup = supervisor(Vec::new());

    Watch::new().poll(&mut k, &mut sup);

    assert!(k.removed_paths().is_empty(), "{:?}", k.removed_paths());
    assert_eq!(said(&k, "NOTHING WAS DONE").len(), 1, "{:?}", k.records);
    assert!(
        said(&k, "a library uninstall was requested").is_empty(),
        "the request was taken up although it is still on the card: {:?}",
        k.records
    );
}

/// **Not on the way down.** A request found while the supervisor is stopping
/// stays on the card for the next boot, and nothing is removed.
#[test]
fn a_request_is_not_taken_up_while_the_machine_is_stopping() {
    let mut k = FakeKernel::new();
    let pack = build("aura-background-pack", "0.2.0", 0x22);
    install_library(&mut k, &pack, Some("aura-background-pack"));
    request(&mut k, "aura-background-pack");
    let mut sup = supervisor(Vec::new());
    sup.begin_shutdown(&mut k, crate::kernel::Terminating::PowerOff);

    Watch::new().poll(&mut k, &mut sup);

    assert!(k.removed_paths().is_empty(), "{:?}", k.removed_paths());
    assert!(
        !k.removed.contains(&uninstall_library::REQUEST_PATH.to_string()),
        "the request was taken off the card during a shutdown"
    );
}

/// **A partial failure says exactly what was and was not removed**, and never
/// reports the request done.
#[test]
fn a_build_that_cannot_be_removed_is_reported_as_still_installed() {
    let mut k = FakeKernel::new();
    let a = build("aura-background-pack", "0.1.0", 0x11);
    let b = build("aura-background-pack", "0.2.0", 0x22);
    install_library(&mut k, &a, Some("aura-background-pack"));
    install_library(&mut k, &b, Some("aura-background-pack"));
    request(&mut k, "aura-background-pack");
    // The first rename the removal makes is refused; the second succeeds.
    k.rename_results = Script::sequence(vec![Err(Errno::EACCES)], Ok(()));
    let mut sup = supervisor(Vec::new());

    Watch::new().poll(&mut k, &mut sup);

    // What is LEFT, rather than `removed_paths`: the fake records a rename it
    // refused as well as one it performed.
    assert_eq!(
        k.list_dir(&lib_root(BUILT_IN_VOLUME)),
        Ok(vec![a.to_string()]),
        "the build whose removal failed should be the one left"
    );
    assert_eq!(said(&k, "STILL INSTALLED and still given").len(), 1, "{:?}", k.records);
    assert!(said(&k, "request is done").is_empty(), "a partial removal was reported done");
    let partly = said(&k, "only PARTLY done");
    assert_eq!(partly.len(), 1, "{:?}", k.records);
    assert_eq!(text(partly[0], "removed"), b.to_string());
    assert_eq!(text(partly[0], "not_removed"), a.to_string());
}

/// The value of one variable in a definition's environment.
fn env_of(def: &ServiceDef, key: &str) -> Option<String> {
    def.program
        .env
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}

/// **Without a reboot, the library set every service is given describes what
/// is left.** The application joins the addon by class before the removal;
/// after it, the definition the supervisor holds — composed afresh by
/// discovery — carries the build in neither `RIVET_ADDONS` nor
/// `LD_LIBRARY_PATH`.
#[test]
fn after_a_removal_a_freshly_composed_definition_no_longer_carries_the_build() {
    let mut k = FakeKernel::new();
    let pack = build("aura-background-pack", "0.2.0", 0x22);
    install_library(&mut k, &pack, Some("aura-background-pack"));
    install_application(&mut k, "aura-renderer", &[], &["aura-background-pack"]);
    let mut defs = Vec::new();
    crate::applications::discover(&mut k, &mut journal(), BUILT_IN_VOLUME, &mut defs);
    let dir = library_object_dir(BUILT_IN_VOLUME, &pack);
    let before = defs
        .iter()
        .find(|d| d.name == "aura-renderer")
        .expect("discovered");
    assert!(
        env_of(before, "LD_LIBRARY_PATH").is_some_and(|p| p.contains(&dir)),
        "the fixture is wrong: the addon was never joined: {:?}",
        before.program.env
    );
    assert!(
        env_of(before, rivet_core::service::ADDONS_ENV).is_some_and(|p| p.contains(&dir)),
        "the fixture is wrong: the addon was never named: {:?}",
        before.program.env
    );
    let mut sup = supervisor(defs);

    request(&mut k, "aura-background-pack");
    Watch::new().poll(&mut k, &mut sup);

    let after = &sup
        .services()
        .iter()
        .find(|m| m.def.name == "aura-renderer")
        .expect("still known")
        .def;
    assert!(
        !env_of(after, "LD_LIBRARY_PATH").is_some_and(|p| p.contains(&dir)),
        "the removed build is still on the library path: {:?}",
        after.program.env
    );
    assert!(
        !env_of(after, rivet_core::service::ADDONS_ENV).is_some_and(|p| p.contains(&dir)),
        "the removed build is still named as an addon: {:?}",
        after.program.env
    );
}
