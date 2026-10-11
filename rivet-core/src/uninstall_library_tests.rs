//! The channel's grammar and the decision, with no I/O — `ADR-014` §3's
//! standing check holds for all of it.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::package::application::{Manifest, ProgramKind};
use crate::package::closure::MemoryTree;
use crate::service::{self, BUILT_IN_VOLUME, ServiceName};

fn principal() -> Principal {
    Principal::no_session()
}

/// A valid principal and the marker — everything a request line needs before
/// its selector.
fn prefix() -> String {
    format!(
        "{}{} {}",
        crate::deploy::FROM_FIELD,
        crate::deploy::NO_SESSION_TOKEN,
        crate::deploy::PAYLOAD_MARKER
    )
}

fn build(product: &str, version: &str, byte: u8) -> Reference {
    Reference::compose(product, version, &[byte; 32]).expect("a reference")
}

fn name(text: &str) -> ServiceName {
    ServiceName::new(text).expect("a service name")
}

// ---------------------------------------------------------------------------
// The selector
// ---------------------------------------------------------------------------

/// **A full reference selects one build and a product name selects a
/// product**, and a word shaped like both is read as the more specific.
#[test]
fn a_reference_is_a_build_and_anything_else_usable_is_a_product() {
    let one = build("aura-background-pack", "0.2.0", 0xaa);
    assert_eq!(
        Selector::parse(one.as_str()),
        Ok(Selector::Build(one.clone()))
    );
    assert_eq!(
        Selector::parse("aura-background-pack"),
        Ok(Selector::Product("aura-background-pack".to_string()))
    );
}

/// **Nothing that is not one path component is a selector.** This is what
/// stops a request reaching `../../system` through a directory removal.
#[test]
fn a_selector_that_is_not_one_path_component_is_refused() {
    let long = "a".repeat(library::MAX_REFERENCE + 1);
    for hostile in [
        "", "..", "../..", "a/b", "/etc", ".hidden", "-x", "a..b", "lost+found", "a\\b",
        long.as_str(),
    ] {
        let got = Selector::parse(hostile);
        assert!(got.is_err(), "{hostile:?} was accepted: {got:?}");
        assert!(
            got.unwrap_err().why().contains("neither a full library reference"),
            "{hostile:?}"
        );
    }
}

/// A product selects every build of itself and **no build of a product whose
/// name it merely begins** — the grouping is `Reference::product`'s, which is
/// retention's.
#[test]
fn a_product_selects_its_own_builds_and_not_a_product_it_is_a_prefix_of() {
    let a = build("aura-background-pack", "0.1.0", 0x11);
    let b = build("aura-background-pack", "0.2.0", 0x22);
    let other = build("aura-background-pack-extra", "0.1.0", 0x33);
    let selector = Selector::parse("aura-background-pack").expect("a product");
    assert!(selector.matches(&a));
    assert!(selector.matches(&b));
    assert!(!selector.matches(&other), "a longer product was selected");

    let exact = Selector::parse(a.as_str()).expect("a reference");
    assert!(exact.matches(&a));
    assert!(!exact.matches(&b), "a reference selected a second build");
}

// ---------------------------------------------------------------------------
// The request
// ---------------------------------------------------------------------------

/// Round trip, both selectors, because the writer and the reader are in one
/// module precisely so they cannot drift.
#[test]
fn a_rendered_request_parses_back_to_what_produced_it() {
    for selector in [
        Selector::parse("aura-background-pack").expect("a product"),
        Selector::Build(build("libgreet", "1.0.0", 0xcc)),
    ] {
        let text = render_request(&selector, &principal());
        let back = parse_request(&text).expect("it parses");
        assert_eq!(back.selector, selector, "{text}");
        assert_eq!(back.principal, principal(), "{text}");
    }
}

/// The same malformed-file refusals `crate::uninstall` has, each on a line
/// that is well-formed up to the one thing being tested.
#[test]
fn a_malformed_file_is_refused_rather_than_guessed_at() {
    let good = prefix();
    assert_eq!(parse_request(""), Err(RequestProblem::Empty));
    assert_eq!(
        parse_request(&format!("{good} a\n{good} b\n")),
        Err(RequestProblem::MoreThanOneLine)
    );
    assert!(matches!(
        parse_request(&format!("{good} a\u{7}b\n")),
        Err(RequestProblem::ControlCharacter { .. })
    ));
    let huge = format!("{good} {}\n", "a".repeat(REQUEST_MAX_BYTES as usize));
    assert!(matches!(
        parse_request(&huge),
        Err(RequestProblem::TooLarge { .. })
    ));
    assert!(matches!(
        parse_request(&format!("{good} ../system\n")),
        Err(RequestProblem::UnusableSelector { .. })
    ));
    // A second word is refused rather than ignored: a request that carried
    // one this build does not know is not one it understood.
    assert!(matches!(
        parse_request(&format!("{good} libgreet purge\n")),
        Err(RequestProblem::UnknownWord { .. })
    ));
    assert!(matches!(
        parse_request("libgreet\n"),
        Err(RequestProblem::Principal { .. })
    ));
}

// ---------------------------------------------------------------------------
// The decision
// ---------------------------------------------------------------------------

fn declaring(app: &str, library: &[&Reference], accepts: &[&str]) -> Declaring {
    Declaring {
        name: name(app),
        library: library.iter().map(|r| (*r).clone()).collect(),
        accepts_classes: accepts.iter().map(ToString::to_string).collect(),
    }
}

/// **Nothing installed matching is a refusal that lists what is**, so the
/// next attempt can be right.
#[test]
fn nothing_matching_is_refused_and_names_what_is_installed() {
    let installed = vec![build("libgreet", "1.0.0", 0xaa)];
    let selector = Selector::parse("libgreeter").expect("a product");
    let refusal = select(&selector, &installed).expect_err("nothing matches");
    let why = refusal.why();
    assert!(why.contains("NOTHING HAS BEEN REMOVED"), "{why}");
    assert!(why.contains(installed[0].as_str()), "{why}");
    assert!(why.contains("product `libgreet`"), "{why}");
}

/// **The guard.** A build an installed application names in its declared
/// closure is never removed, and the refusal names every application and every
/// build involved — the whole request, not the part that happened to be safe.
#[test]
fn a_build_named_in_a_declared_closure_refuses_the_whole_request() {
    let old = build("libgreet", "1.0.0", 0xaa);
    let new = build("libgreet", "2.0.0", 0xbb);
    let builds = vec![
        (old.clone(), Class::Nothing),
        (new.clone(), Class::Nothing),
    ];
    let applications = vec![
        declaring("greeter", &[&old], &[]),
        declaring("welcomer", &[&new], &[]),
        declaring("bystander", &[], &[]),
    ];

    let refusal = decide(&builds, &applications).expect_err("both are declared");
    assert_eq!(
        refusal,
        Refusal::Declared {
            holders: vec![
                Holder {
                    application: "greeter".to_string(),
                    reference: old.clone()
                },
                Holder {
                    application: "welcomer".to_string(),
                    reference: new.clone()
                },
            ]
        }
    );
    let why = refusal.why();
    assert!(why.contains("`greeter` declares"), "{why}");
    assert!(why.contains("`welcomer` declares"), "{why}");
    assert!(!why.contains("bystander"), "{why}");
    assert!(
        matches!(refusal.machine_error(), MachineError::Unsafe(_)),
        "a declared closure is an interlock, not a fault"
    );
}

/// **An application that merely accepts the class is reported, not refused.**
/// Removing an addon on purpose is what the verb is for.
#[test]
fn an_application_accepting_the_class_is_reported_and_does_not_refuse() {
    let pack = build("aura-background-pack", "0.2.0", 0xaa);
    let builds = vec![(
        pack.clone(),
        Class::Provides("aura-background-pack".to_string()),
    )];
    let applications = vec![
        declaring("aura-renderer", &[], &["aura-background-pack"]),
        declaring("unrelated", &[], &["temperature-sensor"]),
    ];

    let removal = decide(&builds, &applications).expect("an acceptor is not a holder");
    assert_eq!(removal.builds, vec![pack.clone()]);
    assert_eq!(
        removal.acceptors,
        vec![Acceptor {
            application: "aura-renderer".to_string(),
            class: "aura-background-pack".to_string(),
            reference: pack,
        }]
    );
    assert!(removal.unread.is_empty());
}

/// A build whose own manifest cannot be read is still removable — a class
/// join is never load-bearing the way a declared closure is — but the account
/// says its acceptors are unknown rather than being silently short.
#[test]
fn a_build_whose_class_cannot_be_read_is_reported_as_unread() {
    let pack = build("aura-background-pack", "0.2.0", 0xaa);
    let builds = vec![(pack.clone(), Class::Unread("no manifest".to_string()))];
    let removal = decide(&builds, &[]).expect("removable");
    assert_eq!(removal.unread.len(), 1);
    assert_eq!(removal.unread[0].reference, pack);
}

// ---------------------------------------------------------------------------
// The survey, over a tree in maps
// ---------------------------------------------------------------------------

/// A library on the tree, providing `class`, with its manifest rendered by the
/// manifest's own writer.
fn install_library(tree: &mut MemoryTree, reference: &Reference, class: Option<&str>) {
    tree.directories
        .entry(lib_root(BUILT_IN_VOLUME))
        .or_default()
        .push(reference.to_string());
    let manifest = library::Manifest::new(vec!["libx.so.1".to_string()])
        .expect("a soname")
        .with_addon(class.map(ToString::to_string), None)
        .expect("a class");
    tree.files.insert(
        format!(
            "{}/{}",
            library_dir(BUILT_IN_VOLUME, reference),
            library::MANIFEST_NAME
        ),
        manifest.render(),
    );
}

fn install_application(tree: &mut MemoryTree, app: &str, library: &[&Reference], accepts: &[&str]) {
    tree.directories
        .entry(service::app_root(BUILT_IN_VOLUME))
        .or_default()
        .push(app.to_string());
    let manifest = Manifest::new(ProgramKind::Daemon)
        .with_library(library.iter().map(|r| (*r).clone()).collect())
        .expect("a closure")
        .with_accepts_classes(accepts.iter().map(ToString::to_string).collect())
        .expect("classes");
    tree.files.insert(
        service::manifest_path(BUILT_IN_VOLUME, &name(app)),
        manifest.render(),
    );
}

/// **The whole question, end to end with no I/O**: every build of the product,
/// its class read from its own manifest, and the application joining it named.
#[test]
fn a_survey_selects_every_build_of_a_product_and_names_its_acceptors() {
    let mut tree = MemoryTree::default();
    let a = build("aura-background-pack", "0.1.0", 0x11);
    let b = build("aura-background-pack", "0.2.0", 0x22);
    let other = build("libgreet", "1.0.0", 0x33);
    install_library(&mut tree, &b, Some("aura-background-pack"));
    install_library(&mut tree, &a, Some("aura-background-pack"));
    install_library(&mut tree, &other, None);
    install_application(&mut tree, "aura-renderer", &[], &["aura-background-pack"]);

    let selector = Selector::parse("aura-background-pack").expect("a product");
    let removal = survey(&mut tree, BUILT_IN_VOLUME, &selector).expect("removable");
    assert_eq!(removal.builds, vec![a.clone(), b.clone()], "sorted, and only the product");
    assert_eq!(removal.acceptors.len(), 2, "{:?}", removal.acceptors);
    assert!(removal.acceptors.iter().all(|x| x.application == "aura-renderer"));
}

/// **An unreadable application manifest refuses**, for `closure`'s reason.
#[test]
fn a_survey_refuses_when_an_application_manifest_cannot_be_read() {
    let mut tree = MemoryTree::default();
    let pack = build("aura-background-pack", "0.2.0", 0x22);
    install_library(&mut tree, &pack, Some("aura-background-pack"));
    tree.directories
        .entry(service::app_root(BUILT_IN_VOLUME))
        .or_default()
        .push("half-installed".to_string());

    let selector = Selector::parse("aura-background-pack").expect("a product");
    let refusal = survey(&mut tree, BUILT_IN_VOLUME, &selector).expect_err("a hole");
    assert!(matches!(refusal, Refusal::ClosureUnknown(_)), "{refusal:?}");
    assert!(refusal.why().contains("half-installed"), "{}", refusal.why());
    assert!(refusal.why().contains("NOTHING HAS BEEN REMOVED"), "{}", refusal.why());
}

/// A library root that cannot be listed is UNKNOWN, never empty — which would
/// read as "nothing matches" and send the operator looking for a typo.
#[test]
fn a_library_root_that_cannot_be_listed_is_not_reported_as_empty() {
    let mut tree = MemoryTree::default();
    tree.unlistable
        .insert(lib_root(BUILT_IN_VOLUME), "Input/output error".to_string());
    let selector = Selector::parse("libgreet").expect("a product");
    let refusal = survey(&mut tree, BUILT_IN_VOLUME, &selector).expect_err("unlisted");
    assert!(matches!(refusal, Refusal::LibrariesUnlisted { .. }), "{refusal:?}");
}

/// What a removal set aside is not a build, even before its helper deletes it.
#[test]
fn an_entry_set_aside_is_never_selected() {
    let mut tree = MemoryTree::default();
    tree.directories.insert(
        lib_root(BUILT_IN_VOLUME),
        vec![format!("{}1-1", service::RECLAIM_MARK)],
    );
    let selector = Selector::parse("libgreet").expect("a product");
    let refusal = survey(&mut tree, BUILT_IN_VOLUME, &selector).expect_err("nothing");
    assert!(matches!(refusal, Refusal::NothingMatches { ref installed, .. } if installed.is_empty()));
}
