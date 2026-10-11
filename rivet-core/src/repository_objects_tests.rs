//! The store's names and its two pure decisions: which digests a manifest
//! names, and what a prune would remove.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

fn d(fill: char) -> Digest {
    Digest::new(&fill.to_string().repeat(64)).expect("a digest")
}

fn held(fill: char, name: &str) -> Held {
    Held { digest: d(fill), name: name.to_owned() }
}

/// **Inside `rivet-fleetd`'s own data directory, and apart from the machine's
/// store** — R-184 D1 in two assertions.
#[test]
fn the_store_is_in_rivet_fleetds_data_directory_and_not_the_machine_store() {
    let store = store("/state");
    assert_eq!(store.root(), "/state/app/rivet-fleetd/data/repository-objects");
    assert_ne!(store.root(), crate::state::ARTIFACTS);
    assert!(!store.root().starts_with(crate::state::ARTIFACTS));
    assert!(ServiceName::new(APPLICATION).is_some(), "{APPLICATION} is not a service name");
    // The layout is the machine store's: a path in is built from a digest.
    assert_eq!(
        store.object_dir(&d('a')),
        format!("/state/app/rivet-fleetd/data/repository-objects/objects/{}", "a".repeat(64))
    );
}

/// A digest that could escape the store is not a digest, so no path is built
/// from it.
#[test]
fn a_digest_that_could_escape_the_store_is_not_one() {
    for bad in ["../../secrets", &"A".repeat(64), &"a".repeat(63), "", &"g".repeat(64)] {
        assert!(Digest::new(bad).is_none(), "{bad:?} was accepted as a digest");
    }
}

/// **Every kind this build knows, cards and updates included** — `ADR-063` §6.
/// A kind this build cannot open is not held.
#[test]
fn the_store_holds_every_kind_this_build_knows_and_nothing_else() {
    for kind in [Kind::application(), Kind::library(), Kind::apk(), Kind::update(), Kind::card()] {
        assert!(holds_kind(&kind), "{} is not held", kind.as_str());
    }
    let unknown = Kind::parse("extension").expect("a token a newer header could carry");
    assert!(!holds_kind(&unknown), "a kind this build cannot open was held");
}

/// **One object is read back by its digest and nothing else** — `ADR-063` §6's
/// fetch. The listing, `prune`, an object's name and anything that is not a
/// whole digest are not an object request, and the route round-trips.
#[test]
fn an_object_is_asked_for_by_its_digest_and_nothing_else_is_one() {
    let digest = d('b');
    assert_eq!(object_route(&digest), format!("/repository-objects/{}", "b".repeat(64)));
    assert_eq!(requested_object(&object_route(&digest)), Some(digest));
    for not_one in [
        PREFIX.to_owned(),
        PRUNE.to_owned(),
        "/repository-objects/hello-0.1.0-aarch64-application.rvt".to_owned(),
        format!("/repository-objects/{}/x", "b".repeat(64)),
        format!("/repository-objects/{}", "B".repeat(64)),
        format!("/repository-objects/../{}", "b".repeat(64)),
        format!("/payload/{}", "b".repeat(64)),
    ] {
        assert_eq!(requested_object(&not_one), None, "{not_one:?} was read as an object request");
    }
}

/// R-184 D2 as it stands: the operator reads and writes.
#[test]
fn the_operator_may_read_and_write() {
    assert!(permits(Caller::Operator, Access::Read));
    assert!(permits(Caller::Operator, Access::Write));
}

/// **A manifest names every kind the store holds**, cards and updates with
/// the rest, and nothing of a kind this build cannot open.
#[test]
fn a_manifest_names_its_cards_and_updates_with_the_rest() {
    let a = "a".repeat(64);
    let b = "b".repeat(64);
    let c = "c".repeat(64);
    let e = "e".repeat(64);
    let f = "f".repeat(64);
    let g = "1".repeat(64);
    let named = named(
        [
            (a.as_str(), "p", "application"),
            (b.as_str(), "p", "library"),
            (c.as_str(), "p", "update"),
            (e.as_str(), "p", "card"),
            (f.as_str(), "p", "apk"),
            (g.as_str(), "p", "extension"),
        ],
        &[],
    )
    .expect("reads");
    assert_eq!(named, BTreeSet::from([d('a'), d('b'), d('c'), d('e'), d('f')]));
}

/// **The store keeps three cards, as the repository does**: retention is the
/// manifest's, so four cards held against a manifest that names the newest
/// three prune to exactly those three.
#[test]
fn four_cards_held_against_a_manifest_naming_three_prune_to_three() {
    let names = ["a", "b", "c", "e"].map(|f| f.repeat(64));
    let named = named(names[1..].iter().map(|n| (n.as_str(), "rivet", "card")), &[]).expect("reads");
    let listing = Listing {
        manifest: Manifest::Read { commit: "c".to_owned(), named },
        held: vec![
            held('a', "rivet-0.1.0-aarch64-rpi4-card.rvt"),
            held('b', "rivet-0.1.1-aarch64-rpi4-card.rvt"),
            held('c', "rivet-0.1.2-aarch64-rpi4-card.rvt"),
            held('e', "rivet-0.1.3-aarch64-rpi4-card.rvt"),
        ],
    };
    assert_eq!(listing.prune().expect("complete"), vec![held('a', "rivet-0.1.0-aarch64-rpi4-card.rvt")]);
}

/// A manifest with an entry that does not read is refused whole.
#[test]
fn a_manifest_with_an_unreadable_entry_is_refused_whole() {
    let a = "a".repeat(64);
    let why = named([(a.as_str(), "p", "application"), ("not-a-digest", "p", "application")], &[])
        .expect_err("refused");
    assert!(why.contains("[[artifact]] 2"), "{why}");
}

/// **A hidden product's objects are not named** — `ADR-052` §7. The hidden
/// set is `(product, kind)`, so the same product name under another kind is
/// still named.
#[test]
fn a_hidden_products_objects_are_not_named_and_another_kind_of_the_same_name_is() {
    let a = "a".repeat(64);
    let b = "b".repeat(64);
    let c = "c".repeat(64);
    let named = named(
        [
            (a.as_str(), "testbed", "apk"),
            (b.as_str(), "testbed", "application"),
            (c.as_str(), "hello", "application"),
        ],
        &[("testbed", "apk")],
    )
    .expect("reads");
    assert_eq!(named, BTreeSet::from([d('b'), d('c')]));
}

/// **A hidden object the store holds is pruned, and one it lacks is not
/// missing** — the two consequences of [`named`] leaving it out, through the
/// listing's own rule.
#[test]
fn a_hidden_object_held_is_pruned_and_one_not_held_is_not_missing() {
    let a = "a".repeat(64);
    let b = "b".repeat(64);
    let c = "c".repeat(64);
    let entries = [
        (a.as_str(), "testbed", "apk"),
        (b.as_str(), "hello", "application"),
        (c.as_str(), "gone", "apk"),
    ];
    let hidden = [("testbed", "apk"), ("gone", "apk")];
    let listing = Listing {
        manifest: Manifest::Read { commit: "c".to_owned(), named: named(entries, &hidden).expect("reads") },
        held: vec![held('a', "testbed.rvt"), held('b', "hello.rvt")],
    };
    assert!(listing.missing().is_empty(), "{:?}", listing.missing());
    assert_eq!(listing.prune().expect("complete"), vec![held('a', "testbed.rvt")]);
}

/// The listing survives the wire both ways, with a reason on one line.
#[test]
fn a_listing_round_trips() {
    let listing = Listing {
        manifest: Manifest::Read { commit: "abc123".to_owned(), named: BTreeSet::from([d('a'), d('b')]) },
        held: vec![held('b', "lib-0.1.0-aarch64-library.rvt"), held('c', "old-0.1.0-aarch64-application.rvt")],
    };
    let back = Listing::parse(&listing.render()).expect("reads");
    assert_eq!(back.manifest, listing.manifest);
    assert_eq!(back.held.len(), 2);

    let unavailable = Listing {
        manifest: Manifest::Unavailable("no\nmain   yet".to_owned()),
        held: Vec::new(),
    };
    assert_eq!(
        Listing::parse(&unavailable.render()).expect("reads").manifest,
        Manifest::Unavailable("no main yet".to_owned())
    );
}

/// A listing that says nothing about its manifest, or carries a line this build
/// does not know, is refused rather than read around.
#[test]
fn a_listing_this_build_cannot_read_whole_is_refused() {
    assert!(Listing::parse("held aaaa x.rvt\n").is_err());
    assert!(Listing::parse(&format!("manifest abc\nkept {}\n", "a".repeat(64))).is_err());
    assert!(Listing::parse("# nothing\n").is_err());
}

/// **Missing** is what the manifest names and the store lacks — the completion
/// signal.
#[test]
fn missing_is_named_and_not_held() {
    let listing = Listing {
        manifest: Manifest::Read { commit: "c".to_owned(), named: BTreeSet::from([d('a'), d('b')]) },
        held: vec![held('a', "a.rvt")],
    };
    assert_eq!(listing.missing(), vec![d('b')]);
}

/// **A prune removes exactly what the manifest stops naming**, and never what
/// it names.
#[test]
fn a_complete_store_prunes_exactly_what_the_manifest_does_not_name() {
    let listing = Listing {
        manifest: Manifest::Read { commit: "c".to_owned(), named: BTreeSet::from([d('a')]) },
        held: vec![held('a', "a.rvt"), held('f', "f.rvt"), held('e', "e.rvt")],
    };
    let removed = listing.prune().expect("a complete store prunes");
    assert_eq!(removed, vec![held('e', "e.rvt"), held('f', "f.rvt")]);
}

/// **Pruning never runs in the middle of a sync**: a store still missing
/// something named refuses, and so does one whose manifest could not be read.
#[test]
fn a_prune_is_refused_while_anything_named_is_missing_or_no_manifest_was_read() {
    let incomplete = Listing {
        manifest: Manifest::Read { commit: "c".to_owned(), named: BTreeSet::from([d('a'), d('b')]) },
        held: vec![held('a', "a.rvt"), held('f', "f.rvt")],
    };
    let why = incomplete.prune().expect_err("refused");
    assert!(why.contains("not complete"), "{why}");
    assert!(why.contains(d('b').as_str()), "{why}");

    let unread = Listing {
        manifest: Manifest::Unavailable("no main".to_owned()),
        held: vec![held('f', "f.rvt")],
    };
    let why = unread.prune().expect_err("refused");
    assert!(why.contains("remove every object"), "{why}");
}
