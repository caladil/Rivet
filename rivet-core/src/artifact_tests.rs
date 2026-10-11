//! The store's arithmetic, exercised where every state it has to be right
//! about is reachable — which is the reason `super` performs no I/O.
//!
//! A separate file for `super`'s reason: the module is a layout and a rule, and
//! the rule's interesting cases are numerous enough that inlining them would
//! bury the twenty lines they are about.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

/// A digest that is a digest, spelled once.
const A: &str = "aa11111111111111111111111111111111111111111111111111111111111111";
const B: &str = "bb22222222222222222222222222222222222222222222222222222222222222";
const C: &str = "cc33333333333333333333333333333333333333333333333333333333333333";
const D: &str = "dd44444444444444444444444444444444444444444444444444444444444444";

fn digest(text: &str) -> Digest {
    Digest::new(text).unwrap_or_else(|| unreachable!("{text} is meant to be a digest"))
}

/// The store on a machine, which is what the layout assertions are about.
fn store() -> Store {
    Store::on_this_machine()
}

fn stored(text: &str, name: &str, added: u64) -> Stored {
    Stored {
        digest: digest(text),
        name: name.to_string(),
        added,
    }
}

// -- the digest ------------------------------------------------------------

#[test]
fn a_digest_is_sixty_four_lowercase_hex_characters_and_nothing_else() {
    assert!(Digest::new(A).is_some());
    for not in [
        "",
        "abc",
        // 63 and 65.
        &A[..63],
        &format!("{A}a"),
        // Upper case is refused rather than normalised: two spellings would be
        // two directories on a case-sensitive filesystem.
        &A.to_ascii_uppercase(),
        // Not hex.
        &A.replace("aa", "gg"),
        // The two shapes that would escape the store if a path were built from
        // them without this check.
        "../../../../etc/passwd",
        &format!("{}/x", &A[..62]),
    ] {
        assert!(Digest::new(not).is_none(), "{not:?} was accepted");
    }
}

/// **The property the type exists for**: there is no way to reach a path in the
/// store without the digest having been checked, so every path this module can
/// produce stays inside it.
#[test]
fn every_path_a_digest_produces_is_inside_the_store() {
    let d = digest(A);
    let store = store();
    for path in [
        store.object_dir(&d),
        store.incoming_for(&d),
        store.object_path(&d, "thing-0.1.0-aarch64-application.rvt"),
        store.incoming_path(&d, "thing-0.1.0-aarch64-application.rvt"),
    ] {
        assert!(
            path.starts_with(&format!("{}/", crate::state::ARTIFACTS)),
            "{path} left the store"
        );
        assert!(!path.contains(".."), "{path}");
    }
}

/// **The root is a parameter and every path honours it** — `ADR-052`'s own
/// History is the reason this is asserted rather than assumed: the last root
/// that resolved itself wrote a live tree on the machine that ran the tests.
#[test]
fn a_store_rooted_elsewhere_puts_every_path_there() {
    let d = digest(A);
    let store = Store::at("/tmp/somewhere");
    for path in [
        store.objects_dir(),
        store.incoming_dir(),
        store.object_dir(&d),
        store.incoming_for(&d),
        store.object_path(&d, "x.rvt"),
        store.incoming_path(&d, "x.rvt"),
    ] {
        assert!(path.starts_with("/tmp/somewhere/"), "{path}");
        assert!(
            !path.starts_with(crate::state::ARTIFACTS),
            "a store given a root reached the machine's own: {path}"
        );
    }
}

/// Incoming and objects are siblings, so a listing of one is not a listing of
/// the other — the property a reader relies on when it treats *present in
/// `objects/`* as *this machine holds it*.
#[test]
fn a_transfer_in_flight_is_not_in_the_objects_directory() {
    let d = digest(A);
    let store = store();
    assert!(!store.incoming_for(&d).starts_with(&store.objects_dir()));
    assert!(!store.object_dir(&d).starts_with(&store.incoming_dir()));
    assert_ne!(store.incoming_for(&d), store.object_dir(&d));
}

#[test]
fn the_object_directory_is_named_by_the_digest_and_the_file_keeps_its_adr_027_name() {
    let d = digest(A);
    let store = store();
    assert_eq!(store.object_dir(&d), format!("/state/artifacts/objects/{A}"));
    assert_eq!(
        store.object_path(&d, "rivet-todo-0.1.1-g3d28ede-aarch64-application.rvt"),
        format!("/state/artifacts/objects/{A}/rivet-todo-0.1.1-g3d28ede-aarch64-application.rvt")
    );
}

// -- the name --------------------------------------------------------------

#[test]
fn an_applications_name_reads_back_with_no_platform() {
    let name = parse_name("hello-0.1.0-aarch64-application.rvt").expect("an application");
    assert_eq!(name.product, "hello");
    assert_eq!(name.version, "0.1.0");
    assert_eq!(name.arch, "aarch64");
    assert_eq!(name.platform, None);
    assert_eq!(name.kind, "application");
}

/// A hyphen runs through the product and twice through the version, which is
/// the case the right-to-left read exists for.
#[test]
fn a_hyphenated_product_and_a_dirty_version_both_survive() {
    let name = parse_name("rivet-todo-0.1.1-g3d28ede-dirty-aarch64-application.rvt")
        .expect("an application");
    assert_eq!(name.product, "rivet-todo");
    assert_eq!(name.version, "0.1.1-g3d28ede-dirty");
    assert_eq!(name.platform, None);
}

/// **A digit-led token that is not shaped like a crate version stays in the
/// product.** `sensor-2` and `sensor-3` are two different, equally valid
/// products — `ServiceName` allows a digit-containing name — and the `2`/`3`
/// must not be mistaken for the start of the version just because it begins
/// with a digit.
#[test]
fn a_numeric_product_suffix_does_not_start_the_version() {
    for (file_name, product) in [
        ("sensor-2-0.1.0-aarch64-application.rvt", "sensor-2"),
        ("sensor-3-0.1.0-aarch64-application.rvt", "sensor-3"),
        ("sensor-2fa-0.1.0-aarch64-application.rvt", "sensor-2fa"),
    ] {
        let name = parse_name(file_name).unwrap_or_else(|| panic!("{file_name} did not parse"));
        assert_eq!(name.product, product, "{file_name}");
        assert_eq!(name.version, "0.1.0", "{file_name}");
    }
}

/// **A product that itself starts with a digit now reads back**, where it did
/// not before: `3d` is not shaped like a crate version, so it no longer looks
/// like a (zero-length, and therefore refused) version sitting at token zero.
#[test]
fn a_product_that_starts_with_a_digit_is_not_mistaken_for_the_version() {
    let name = parse_name("3d-probe-0.1.0-aarch64-application.rvt").expect("an application");
    assert_eq!(name.product, "3d-probe");
    assert_eq!(name.version, "0.1.0");
}

/// A system update carries `<platform>` and an application never does —
/// `ADR-027` §9 item 3, asserted in the direction that would break retention if
/// it were read as part of the version.
#[test]
fn a_system_updates_platform_is_read_as_a_platform() {
    let name = parse_name("rivet-0.2.0-aarch64-rpi4-update.rvt").expect("an update");
    assert_eq!(name.product, "rivet");
    assert_eq!(name.version, "0.2.0");
    assert_eq!(name.arch, "aarch64");
    assert_eq!(name.platform.as_deref(), Some("rpi4"));
    assert_eq!(name.kind, "update");
}

#[test]
fn a_library_reads_back_as_a_library() {
    let name = parse_name("greet-0.3.0-aarch64-library.rvt").expect("a library");
    assert_eq!(name.product, "greet");
    assert_eq!(name.kind, "library");
}

#[test]
fn a_name_this_project_did_not_compose_is_not_parsed() {
    for odd in [
        "something-somebody-renamed.rvt",
        "hello.rvt",
        "hello-0.1.0-application.rvt",
        // No suffix at all.
        "hello-0.1.0-aarch64-application",
        // A version that does not begin with a digit, so nothing marks where
        // the product ends.
        "hello-vone-aarch64-application.rvt",
        // An arch token where the product belongs, with nothing before it.
        "aarch64-application.rvt",
        "",
    ] {
        assert!(parse_name(odd).is_none(), "{odd:?} was parsed");
    }
}

/// **An unparseable name is its own product**, which is the safe direction: it
/// forms a group of one and never displaces a real product's three.
#[test]
fn an_unparseable_name_groups_as_itself() {
    assert_eq!(product_of("hello-0.1.0-aarch64-application.rvt"), "hello");
    assert_eq!(product_of("renamed.rvt"), "renamed.rvt");
    assert_eq!(product_of(""), "");
}

// -- the version's order ---------------------------------------------------

fn numbered(version: &str) -> Vec<u64> {
    match version_order(version) {
        VersionOrder::Numbered(parts) => parts,
        VersionOrder::Unreadable => panic!("{version} was meant to be readable"),
    }
}

/// **The arithmetic string comparison gets wrong.** `0.3.10` is newer than
/// `0.3.9`; as text it is older, and that is the whole reason this is a number
/// comparison rather than a `str` one.
#[test]
fn ten_is_newer_than_nine_even_though_the_text_says_otherwise() {
    assert!(version_order("0.3.10") > version_order("0.3.9"));
    assert!(version_order("0.10.0") > version_order("0.9.0"));
    assert!(version_order("10.0.0") > version_order("9.99.99"));
    // The text order this is correcting, asserted so the test is about the
    // thing it claims to be about.
    assert!("0.3.10" < "0.3.9");
}

#[test]
fn a_plain_version_reads_as_its_components() {
    assert_eq!(numbered("0.3.4"), vec![0, 3, 4]);
    assert_eq!(numbered("1.20.300"), vec![1, 20, 300]);
    assert_eq!(numbered("7"), vec![7]);
}

/// **`ADR-027` §3's build metadata carries no order**, so it is discarded
/// rather than compared: a `-g<sha>`, a `-dirty` and a dirty build's payload
/// digest all describe *which* bytes, never *when*.
#[test]
fn build_metadata_is_discarded_rather_than_ordered() {
    assert_eq!(numbered("0.1.1-g3d28ede"), vec![0, 1, 1]);
    assert_eq!(numbered("0.1.1-g3d28ede-dirty"), vec![0, 1, 1]);
    assert_eq!(numbered("0.1.1-g3d28ede-dirty-1a2b3c4d"), vec![0, 1, 1]);
    // All four are one version, and arrival is what separates them.
    assert_eq!(version_order("0.1.1"), version_order("0.1.1-g3d28ede-dirty"));
}

/// A described build must not sort BELOW the bare tag. SemVer would read
/// `-g3d28ede` as a pre-release and say exactly that, which is backwards for
/// `ADR-027` §3's grammar — the dash is there because `+` is a nuisance in a
/// filename, not because the token is a pre-release.
#[test]
fn a_git_described_build_is_not_treated_as_a_pre_release() {
    assert!(version_order("0.1.1-g3d28ede") >= version_order("0.1.1"));
    assert!(version_order("0.1.1-g3d28ede") > version_order("0.1.0"));
}

/// `0.3` and `0.3.0` are one version. `Vec`'s own `Ord` would put the shorter
/// below the longer, which is why the comparison zero-extends.
#[test]
fn a_missing_trailing_component_is_a_zero() {
    assert_eq!(version_order("0.3"), version_order("0.3.0"));
    assert_eq!(version_order("1"), version_order("1.0.0"));
    assert!(version_order("0.3.1") > version_order("0.3"));
}

/// **Every shape that is not dot-separated numbers is unreadable, said
/// explicitly** — a letter, nothing at all, an empty component, and a number
/// too large for a `u64` — rather than left to whatever a failed parse
/// happened to return. Retention keeps these apart from readable versions, so
/// the direction between the two kinds is asserted only to show the type stays
/// totally ordered, which a sort is entitled to rely on.
#[test]
fn every_version_that_is_not_numbers_is_unreadable() {
    for odd in ["9abc", "", "0.3.x", "0.-1.0", "0.3.99999999999999999999999"] {
        assert_eq!(version_order(odd), VersionOrder::Unreadable, "{odd:?}");
        assert!(version_order(odd) > version_order("999.999.999"), "{odd:?}");
    }
    assert_eq!(version_order("a"), version_order("b"), "and they tie");
}

/// A name this project did not compose has no version to read. It costs nothing
/// — `product_of` has already made it a group of one.
#[test]
fn a_name_with_no_readable_version_is_unreadable() {
    assert_eq!(
        version_order_of("hello-0.3.4-aarch64-application.rvt"),
        VersionOrder::Numbered(vec![0, 3, 4])
    );
    assert_eq!(
        version_order_of("rivet-0.1.81-aarch64-rpi4-update.rvt"),
        VersionOrder::Numbered(vec![0, 1, 81])
    );
    assert_eq!(version_order_of("renamed.rvt"), VersionOrder::Unreadable);
    assert_eq!(version_order_of(""), VersionOrder::Unreadable);
}

// -- comparing two builds for "which is newest" -----------------------------

fn build<'a>(version: &'a str, added: &'a str, identity: &'a str) -> BuildKey<'a> {
    BuildKey { version, added, identity }
}

/// **`.rulings/` R-213's own case.** Two builds share one version's numeric
/// core — the ordinary shape of "the same crate version, built twice" — and
/// the one added to the repository more recently is the newer of the two.
#[test]
fn a_version_tie_is_broken_by_the_later_added_stamp() {
    let older = build("0.1.114-gAAAAAAA", "2026-09-24T10:00:00Z", A);
    let newer = build("0.1.114-gBBBBBBB", "2026-09-24T11:00:00Z", B);
    assert_eq!(newest_cmp(newer, older), core::cmp::Ordering::Greater, "newer is greater");
    assert_eq!(newest_cmp(older, newer), core::cmp::Ordering::Less);

    // The two conventions the doc comment promises: `max_by` picks the
    // newest directly, and a listing is the reverse of a plain `sort_by`.
    assert_eq!(
        [older, newer].into_iter().max_by(|a, b| newest_cmp(*a, *b)).map(|k| k.identity),
        Some(B)
    );
    let mut listed = [older, newer];
    listed.sort_by(|a, b| newest_cmp(*a, *b).reverse());
    assert_eq!(listed[0].identity, B, "sorted newest first");
}

/// The version always outranks `added` — a later build of a LOWER version
/// must never read as "newest".
#[test]
fn a_higher_version_wins_however_recently_the_lower_one_was_added() {
    let low_but_recent = build("0.1.113", "2026-09-25T00:00:00Z", A);
    let high_but_older = build("0.1.114-gAAAAAAA", "2026-09-20T00:00:00Z", B);
    assert_eq!(
        newest_cmp(high_but_older, low_but_recent),
        core::cmp::Ordering::Greater,
        "the higher version is newer even though it was added first"
    );
}

/// **The last, deterministic tie-break.** Two builds can share `added` too —
/// `ADR-021`'s one-second granularity is the reason `evicted` needed the same
/// third key — so without `identity` the answer would depend on iteration
/// order rather than being total.
#[test]
fn a_version_and_added_tie_is_broken_by_identity() {
    let a = build("0.1.114-gAAAAAAA", "2026-09-24T10:00:00Z", A);
    let b = build("0.1.114-gBBBBBBB", "2026-09-24T10:00:00Z", B);
    assert_eq!(
        version_order(a.version),
        version_order(b.version),
        "same numeric core, so version_order alone ties"
    );
    assert_eq!(a.added, b.added, "and added ties too, on purpose, for this test");
    assert_eq!(newest_cmp(a, b), a.identity.cmp(b.identity), "identity decides, plainly");
}

/// [`evicted`]'s own tie-break, restated: **arrival is never compared across
/// two different versions.** A build a whole minor version behind is never
/// "newest" merely for being freshly added — proven here the same way the
/// version-wins case above is, from the other side of a wider gap.
#[test]
fn added_never_overrides_version_across_a_real_version_gap() {
    let recently_added_lower_version = build("0.1.0", "2026-09-25T23:59:59Z", A);
    let long_ago_added_higher_version = build("0.2.0", "2026-01-01T00:00:00Z", B);
    assert_eq!(
        newest_cmp(long_ago_added_higher_version, recently_added_lower_version),
        core::cmp::Ordering::Greater
    );
}

/// **The exact mistake this convention exists to make impossible.** Pairing
/// [`newest_cmp`] with [`Iterator::min_by`] instead of `max_by` silently picks
/// the OLDEST build — proven here rather than only asserted, because this is
/// the mistake an early draft of the picker fix actually made.
#[test]
fn min_by_would_silently_pick_the_oldest_build_not_the_newest() {
    let older = build("0.1.114-gAAAAAAA", "2026-09-24T10:00:00Z", A);
    let newer = build("0.1.114-gBBBBBBB", "2026-09-24T11:00:00Z", B);
    let wrong = [older, newer].into_iter().min_by(|a, b| newest_cmp(*a, *b));
    assert_eq!(wrong.map(|k| k.identity), Some(A), "min_by is the wrong tool here, by construction");
    let right = [older, newer].into_iter().max_by(|a, b| newest_cmp(*a, *b));
    assert_eq!(right.map(|k| k.identity), Some(B));
}

fn app(product: &str, version: &str) -> String {
    format!("{product}-{version}-aarch64-application.rvt")
}

/// A store below the limit drops nothing, which is the state a machine is
/// usually in and the one a bug here would be least visible in.
#[test]
fn a_store_holding_fewer_than_three_of_a_product_drops_nothing() {
    let held = vec![
        stored(A, &app("photo-index", "0.1.0"), 1),
        stored(B, &app("photo-index", "0.2.0"), 2),
    ];
    assert!(evictions(&held, KEEP, None).is_empty());
}

#[test]
fn exactly_three_is_not_over_the_limit() {
    let held = vec![
        stored(A, &app("photo-index", "0.1.0"), 1),
        stored(B, &app("photo-index", "0.2.0"), 2),
        stored(C, &app("photo-index", "0.3.0"), 3),
    ];
    assert!(evictions(&held, KEEP, None).is_empty());
}

/// **The rule.** A fourth artifact drops the lowest version of the four and
/// nothing else.
#[test]
fn a_fourth_artifact_of_one_product_drops_the_lowest_version() {
    let held = vec![
        stored(A, &app("photo-index", "0.1.0"), 1),
        stored(B, &app("photo-index", "0.2.0"), 2),
        stored(C, &app("photo-index", "0.3.0"), 3),
        stored(D, &app("photo-index", "0.4.0"), 4),
    ];
    let dropped = evictions(&held, KEEP, None);
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert_eq!(dropped[0].digest.as_str(), A);
}

/// Products are counted separately, which is the half that makes the rule
/// *"three of each"* rather than *"three"*.
#[test]
fn one_products_arrivals_never_displace_anothers() {
    let held = vec![
        stored(A, &app("photo-index", "0.1.0"), 1),
        stored(B, &app("tempmon-rivet", "0.1.0"), 2),
        stored(C, &app("tempmon-rivet", "0.2.0"), 3),
        stored(D, &app("tempmon-rivet", "0.3.0"), 4),
    ];
    assert!(
        evictions(&held, KEEP, None).is_empty(),
        "a second product's three counted against the first's one"
    );
}

/// **The defect this guards against**: `1` through `4` each begin with a
/// digit, so a version-start rule that only checked for a leading digit read
/// every one of these as product `sensor`, version `N-0.1.0` — four different,
/// equally valid products (`ServiceName` allows a digit-containing name)
/// folded into one group of four, where the fourth deploy would evict a
/// different product's only copy.
#[test]
fn distinct_numeric_suffixed_products_never_evict_each_other() {
    let held = vec![
        stored(A, &app("sensor-1", "0.1.0"), 1),
        stored(B, &app("sensor-2", "0.1.0"), 2),
        stored(C, &app("sensor-3", "0.1.0"), 3),
        stored(D, &app("sensor-4", "0.1.0"), 4),
    ];
    assert!(
        evictions(&held, KEEP, None).is_empty(),
        "a numeric product suffix was folded into another product's group"
    );
}

/// A system update and an application of the same product name are the same
/// product, and that is deliberate: `rivet-0.1.80-…-update` and
/// `rivet-0.1.81-…-update` are what a board accumulates, and three of them at
/// 700 MB each is already the whole card.
#[test]
fn updates_are_retained_by_product_like_everything_else() {
    let held = vec![
        stored(A, "rivet-0.1.78-aarch64-rpi4-update.rvt", 1),
        stored(B, "rivet-0.1.79-aarch64-rpi4-update.rvt", 2),
        stored(C, "rivet-0.1.80-aarch64-rpi4-update.rvt", 3),
        stored(D, "rivet-0.1.81-aarch64-rpi4-update.rvt", 4),
    ];
    let dropped = evictions(&held, KEEP, None);
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0].digest.as_str(), A);
}

/// **Newest is the version, not the arrival.** An old version sent back to a
/// board does not become the newest thing in the store: the highest three
/// versions are what a rollback needs to reach, whatever order they landed in.
#[test]
fn re_storing_an_old_version_does_not_make_it_the_newest() {
    let held = vec![
        // 0.4.0 arrived first and 0.1.0 was sent back afterwards.
        stored(A, &app("photo-index", "0.4.0"), 1),
        stored(B, &app("photo-index", "0.3.0"), 2),
        stored(C, &app("photo-index", "0.2.0"), 3),
        stored(D, &app("photo-index", "0.1.0"), 4),
    ];
    let dropped = evictions(&held, KEEP, None);
    assert_eq!(dropped.len(), 1);
    assert_eq!(
        dropped[0].name,
        app("photo-index", "0.1.0"),
        "the newest arrival was kept rather than the highest versions"
    );
}

/// And when that old version is the one that just landed, it stays — the
/// arriving object wins its group, so what falls off is the lowest version of
/// the **rest**. The two rules meeting, asserted where they meet.
#[test]
fn an_old_version_sent_back_survives_its_own_prune_and_costs_the_next_lowest() {
    let held = vec![
        stored(A, &app("photo-index", "0.4.0"), 1),
        stored(B, &app("photo-index", "0.3.0"), 1),
        stored(C, &app("photo-index", "0.2.0"), 1),
        stored(D, &app("photo-index", "0.1.0"), 1),
    ];
    let dropped = evictions(&held, KEEP, Some(&digest(D)));
    assert_eq!(dropped.len(), 1);
    assert_eq!(
        dropped[0].name,
        app("photo-index", "0.2.0"),
        "the artifact that had just been transferred was pruned, or the wrong \
         one of the rest was"
    );
}

/// Two artifacts stored in the same second must produce **one** answer, and it
/// must be the version's. A machine with no RTC stamps a whole burst of
/// arrivals identically, so this is the ordinary case on a board rather than a
/// corner of it.
#[test]
fn a_tie_in_arrival_time_is_broken_the_same_way_every_time() {
    let held = vec![
        stored(A, &app("photo-index", "0.1.0"), 7),
        stored(B, &app("photo-index", "0.2.0"), 7),
        stored(C, &app("photo-index", "0.3.0"), 7),
        stored(D, &app("photo-index", "0.4.0"), 7),
    ];
    let first = evictions(&held, KEEP, None);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].digest.as_str(), A);

    // And it does not depend on the order the filesystem listed them in.
    let reversed: Vec<Stored> = held.iter().rev().cloned().collect();
    let second = evictions(&reversed, KEEP, None);
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].digest.as_str(), A);
}

// -- a burst of deploys inside one clock second ----------------------------

/// **Four ascending versions landing inside one clock second keep the three
/// highest.** That is what a burst of deploys to a board produces: one
/// modification time for all four, so nothing but the version can tell them
/// apart, and a newer version deleted while an older one survives is the
/// failure a local rollback cannot afford.
///
/// **Every assignment of digests to versions, because the digest is where luck
/// hides.** A content digest bears no relation to a version, and a rule that
/// fell through to the digest would pass this test whenever the digests
/// happened to ascend with the versions — as they do in the same-second test
/// above. All twenty-four permutations, so luck is not available.
#[test]
fn four_versions_landing_in_one_second_keep_the_three_highest_versions() {
    let versions = ["0.3.1", "0.3.2", "0.3.3", "0.3.4"];
    let digests = [A, B, C, D];
    let mut checked = 0;
    for a in 0..4 {
        for b in 0..4 {
            for c in 0..4 {
                for d in 0..4 {
                    let perm = [a, b, c, d];
                    // Only the permutations; the rest of the 256 have repeats.
                    if (0..4).any(|i| (0..i).any(|j| perm[i] == perm[j])) {
                        continue;
                    }
                    checked += 1;
                    let held: Vec<Stored> = (0..4)
                        .map(|i| {
                            // The same modification time for all four: what a
                            // board's clock — `ADR-021`, a floor with one-second
                            // granularity — stamps on a burst of deploys.
                            stored(digests[perm[i]], &app("hello-rivet", versions[i]), 7)
                        })
                        .collect();
                    let dropped = evictions(&held, KEEP, None);
                    assert_eq!(dropped.len(), 1, "{perm:?}: {dropped:?}");
                    assert_eq!(
                        dropped[0].name,
                        app("hello-rivet", "0.3.1"),
                        "digest assignment {perm:?} chose the eviction, not the version"
                    );
                }
            }
        }
    }
    assert_eq!(checked, 24, "the permutations were not all reached");
}

/// The same burst with the arrival named, as a real transfer names it: the
/// arrival is protected and the lowest version of the rest goes, with the
/// digests descending as the versions ascend.
#[test]
fn the_fifth_deploy_of_one_second_drops_the_lowest_version_of_the_rest() {
    let held = vec![
        stored(D, &app("hello-rivet", "0.3.1"), 7),
        stored(C, &app("hello-rivet", "0.3.2"), 7),
        stored(B, &app("hello-rivet", "0.3.3"), 7),
        stored(A, &app("hello-rivet", "0.3.4"), 7),
    ];
    let dropped = evictions(&held, KEEP, Some(&digest(A)));
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert_eq!(dropped[0].name, app("hello-rivet", "0.3.1"));
}

/// **Ten is newer than nine, in the store and not only in [`version_order`].**
/// `rivet-0.1.9` and `rivet-0.1.10` are two versions a string comparison puts in
/// the wrong order.
///
/// The digests descend as the versions ascend, so no tie-break beneath the
/// version can produce this answer by accident.
#[test]
fn a_two_digit_patch_version_is_newer_than_a_one_digit_one() {
    let held = vec![
        stored(D, "rivet-0.1.9-aarch64-rpi4-update.rvt", 7),
        stored(C, "rivet-0.1.10-aarch64-rpi4-update.rvt", 7),
        stored(B, "rivet-0.1.11-aarch64-rpi4-update.rvt", 7),
        stored(A, "rivet-0.1.12-aarch64-rpi4-update.rvt", 7),
    ];
    let dropped = evictions(&held, KEEP, None);
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert_eq!(dropped[0].name, "rivet-0.1.9-aarch64-rpi4-update.rvt");
}

// -- a run of deploys, one prune each --------------------------------------

/// A digest nobody else in this file uses, for runs longer than four.
fn nth(i: u8) -> String {
    format!("{i:02x}").repeat(32)
}

/// **One deploy, as `cmd/rivet-mcpd` performs it**: the object lands, then the
/// store is pruned with that object named as the arrival. Every deploy in a
/// burst carries the same `added`, because that is what a board's clock gives
/// a burst.
fn deploy(mut held: Vec<Stored>, name: &str, digest_text: &str) -> Vec<Stored> {
    held.push(stored(digest_text, name, 7));
    let gone: Vec<Digest> = evictions(&held, KEEP, Some(&digest(digest_text)))
        .into_iter()
        .map(|s| s.digest.clone())
        .collect();
    held.retain(|s| !gone.contains(&s.digest));
    held
}

fn names(held: &[Stored]) -> Vec<String> {
    let mut out: Vec<String> = held.iter().map(|s| s.name.clone()).collect();
    out.sort();
    out
}

/// **The sequence as a board runs it**, one prune per deploy rather than one
/// prune over a finished store: four ascending versions back to back, with the
/// digests descending as the versions ascend so nothing beneath the version
/// can produce the answer.
#[test]
fn four_deploys_back_to_back_leave_the_three_highest_versions() {
    let mut held = Vec::new();
    for (i, version) in ["0.3.1", "0.3.2", "0.3.3", "0.3.4"].iter().enumerate() {
        held = deploy(held, &app("hello-rivet", version), &nth(0xf0 - i as u8));
    }
    assert_eq!(
        names(&held),
        vec![
            app("hello-rivet", "0.3.2"),
            app("hello-rivet", "0.3.3"),
            app("hello-rivet", "0.3.4"),
        ]
    );
}

/// **`tools/qemu/adr-052-proof.sh`'s sequence, so what that script asserts is
/// known to follow from this rule before a guest boots.** It deploys `0.1.1`,
/// then an untrusted `0.9.9` — which the store's door refuses since 2026-09-12,
/// so it never arrives and is not in this sequence — then `0.1.2`, `0.1.3` and
/// `0.1.4`, and asserts that exactly those three are held.
///
/// Until the door existed the `0.9.9` reached the store and, being the highest
/// version, held one of the three: the proof asserted `0.1.3`, `0.1.4`, `0.9.9`.
/// That was the defect the maintainer's ruling removed.
#[test]
fn the_qemu_proofs_sequence_keeps_the_three_highest_trusted_versions() {
    let mut held = vec![stored(&nth(0x50), &app("slow-stop-probe", "0.1.0"), 7)];
    for (i, version) in ["0.1.1", "0.1.2", "0.1.3", "0.1.4"].iter().enumerate() {
        held = deploy(held, &app("hello-rivet", version), &nth(0x10 + i as u8));
    }
    assert_eq!(
        names(&held),
        vec![
            app("hello-rivet", "0.1.2"),
            app("hello-rivet", "0.1.3"),
            app("hello-rivet", "0.1.4"),
            app("slow-stop-probe", "0.1.0"),
        ]
    );
}

/// **The cost of ordering by version, pinned where it still shows**: a GENUINE
/// artifact numbered above the release line — a test build `0.9.9` signed with
/// the real key, which the door admits because it is authentic — is the highest
/// version, so it stays and holds one of the product's three until something
/// higher arrives. The module header states that cost; this asserts it, so a
/// change to it is a decision rather than a surprise.
#[test]
fn a_genuine_stray_high_version_holds_its_place() {
    let mut held = vec![stored(&nth(0x50), &app("slow-stop-probe", "0.1.0"), 7)];
    for (i, version) in ["0.1.1", "0.9.9", "0.1.2", "0.1.3", "0.1.4"].iter().enumerate() {
        held = deploy(held, &app("hello-rivet", version), &nth(0x10 + i as u8));
    }
    assert_eq!(
        names(&held),
        vec![
            app("hello-rivet", "0.1.3"),
            app("hello-rivet", "0.1.4"),
            app("hello-rivet", "0.9.9"),
            app("slow-stop-probe", "0.1.0"),
        ]
    );
}

/// **A version this build cannot read is not counted against the readable
/// three**, so its presence never costs a readable version its place. The
/// unreadable one carries the lowest digest, so a rule that let the two kinds
/// compete and fell through to the digest would drop it — and a rule that
/// ranked it above them would drop `0.1.0`.
#[test]
fn an_unreadable_version_is_retained_apart_from_the_readable_three() {
    let held = vec![
        stored(A, &app("photo-index", "9.9.9abc"), 7),
        stored(B, &app("photo-index", "0.3.0"), 7),
        stored(C, &app("photo-index", "0.2.0"), 7),
        stored(D, &app("photo-index", "0.1.0"), 7),
    ];
    assert!(
        evictions(&held, KEEP, None).is_empty(),
        "one unreadable version and three readable ones are within both limits"
    );
}

/// **Unreadable versions do not erode rollback.** Three of them are held; four
/// readable versions are then deployed back to back. If the two kinds shared
/// one order with the unreadable above, every later deploy would evict the
/// readable version before it and the store would end holding the current one
/// and nothing to roll back to. Retained apart, the readable three are intact
/// and not one unreadable version was deleted to make room for them.
#[test]
fn unreadable_versions_never_cost_a_readable_rollback_target() {
    let mut held = Vec::new();
    for (i, version) in ["9.9.9abc", "9.9.9def", "9.9.9ghi"].iter().enumerate() {
        held = deploy(held, &app("rivet", version), &nth(0x20 + i as u8));
    }
    for (i, version) in ["0.1.0", "0.2.0", "0.3.0", "0.4.0"].iter().enumerate() {
        held = deploy(held, &app("rivet", version), &nth(0x30 + i as u8));
    }
    assert_eq!(
        names(&held),
        vec![
            app("rivet", "0.2.0"),
            app("rivet", "0.3.0"),
            app("rivet", "0.4.0"),
            app("rivet", "9.9.9abc"),
            app("rivet", "9.9.9def"),
            app("rivet", "9.9.9ghi"),
        ]
    );
}

/// **And they are bounded among themselves, by arrival** — the only order they
/// have. A fourth unreadable version drops the stalest unreadable one and
/// touches no readable version.
#[test]
fn unreadable_versions_age_out_among_themselves_by_arrival() {
    let held = vec![
        stored(A, &app("rivet", "9.9.9abc"), 1),
        stored(B, &app("rivet", "9.9.9def"), 2),
        stored(C, &app("rivet", "9.9.9ghi"), 3),
        stored(D, &app("rivet", "9.9.9jkl"), 4),
        stored(&nth(0x40), &app("rivet", "0.1.0"), 0),
    ];
    let dropped = evictions(&held, KEEP, Some(&digest(D)));
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert_eq!(dropped[0].name, app("rivet", "9.9.9abc"));
}

/// Two artifacts carrying the **same** version are what arrival is left to
/// separate — two dirty builds of one commit (`ADR-027` §3), which differ by
/// their payload digest and by nothing orderable.
#[test]
fn two_builds_of_one_version_are_separated_by_arrival() {
    let dirty = |short: &str, at: u64, d: &str| {
        stored(
            d,
            &format!("photo-index-0.3.4-g3d28ede-dirty-{short}-aarch64-application.rvt"),
            at,
        )
    };
    let held = vec![
        dirty("11111111", 1, A),
        dirty("22222222", 2, B),
        dirty("33333333", 3, C),
        dirty("44444444", 4, D),
    ];
    let dropped = evictions(&held, KEEP, None);
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert_eq!(dropped[0].digest.as_str(), A, "the stalest build was not the one dropped");
}

/// Several products over the limit at once, so the answer is not one group's by
/// accident.
#[test]
fn every_product_over_the_limit_is_pruned_in_one_pass() {
    let mut held = Vec::new();
    for (i, d) in [A, B, C, D].iter().enumerate() {
        held.push(stored(d, &app("photo-index", &format!("0.{i}.0")), i as u64));
    }
    // A second product, with its own five, using distinct digests.
    for i in 0..5u64 {
        let text = format!("{i}{}", &A[1..]);
        held.push(Stored {
            digest: Digest::new(&text).expect("a digest"),
            name: app("tempmon-rivet", &format!("1.{i}.0")),
            added: i,
        });
    }
    let dropped = evictions(&held, KEEP, None);
    assert_eq!(dropped.len(), 1 + 2, "{dropped:?}");
    let names: Vec<&str> = dropped.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&app("photo-index", "0.0.0").as_str()), "{names:?}");
    assert!(names.contains(&app("tempmon-rivet", "1.0.0").as_str()), "{names:?}");
    assert!(names.contains(&app("tempmon-rivet", "1.1.0").as_str()), "{names:?}");
}

/// The answer is stable, so a caller can compare two runs and a test can assert
/// one.
#[test]
fn the_same_store_produces_the_same_answer_however_it_was_listed() {
    let held = vec![
        stored(A, &app("photo-index", "0.1.0"), 1),
        stored(B, &app("photo-index", "0.2.0"), 2),
        stored(C, &app("photo-index", "0.3.0"), 3),
        stored(D, &app("photo-index", "0.4.0"), 4),
    ];
    let forwards: Vec<&Digest> = evictions(&held, KEEP, None).iter().map(|s| &s.digest).collect();
    let shuffled: Vec<Stored> = vec![held[2].clone(), held[0].clone(), held[3].clone(), held[1].clone()];
    let other: Vec<&Digest> = evictions(&shuffled, KEEP, None).iter().map(|s| &s.digest).collect();
    assert_eq!(forwards, other);
}

/// An empty store is an answer rather than a panic — the state every machine
/// starts in.
#[test]
fn an_empty_store_evicts_nothing() {
    assert!(evictions(&[], KEEP, None).is_empty());
}

// -- the arrival is never what falls off -----------------------------------

/// **The defect this rule exists for**, reproduced exactly: four artifacts of
/// one product with the SAME arrival time, which is what a machine writing four
/// directories inside one clock tick produces. Without `arriving`, the ordering
/// below it decides, and it can decide against the artifact that has just been
/// transferred — so a deploy would upload the artifact, delete it, and then ask
/// for an install by a digest the store no longer held. Ordering by version
/// narrowed this case (the lowest version is rarely the arrival) and does not
/// close it: sending an old build back is exactly when it would fire.
#[test]
fn the_artifact_that_just_arrived_is_never_the_one_dropped() {
    let held = vec![
        stored(A, &app("photo-index", "0.1.0"), 7),
        stored(B, &app("photo-index", "0.2.0"), 7),
        stored(C, &app("photo-index", "0.3.0"), 7),
        stored(D, &app("photo-index", "0.4.0"), 7),
    ];
    // Every one of the four, in turn, as the one that just landed.
    for arriving in [A, B, C, D] {
        let dropped = evictions(&held, KEEP, Some(&digest(arriving)));
        assert_eq!(dropped.len(), 1, "{arriving}: {dropped:?}");
        assert_ne!(
            dropped[0].digest.as_str(),
            arriving,
            "the artifact that had just been transferred was pruned"
        );
    }
}

/// And the protection does not stop it being pruned **later**: it is an
/// ordering, not a pin. The next arrival can drop it like anything else.
#[test]
fn an_arrival_is_ordinary_again_on_the_next_prune() {
    let held = vec![
        stored(A, &app("photo-index", "0.1.0"), 7),
        stored(B, &app("photo-index", "0.2.0"), 7),
        stored(C, &app("photo-index", "0.3.0"), 7),
        stored(D, &app("photo-index", "0.4.0"), 7),
    ];
    // `A` landed last time and survived; now `D` lands, and `A` carries the
    // lowest version of the rest with nothing protecting it.
    let dropped = evictions(&held, KEEP, Some(&digest(D)));
    assert_eq!(dropped.len(), 1);
    assert_eq!(dropped[0].digest.as_str(), A);
}

/// An arrival protects only **its own** product's group, because retention is
/// per product and an arrival says nothing about anybody else's.
#[test]
fn an_arrival_does_not_protect_another_products_artifacts() {
    let mut held = vec![stored(A, &app("tempmon-rivet", "1.0.0"), 9)];
    for (i, d) in [B, C, D].iter().enumerate() {
        held.push(stored(d, &app("photo-index", &format!("0.{i}.0")), 1));
    }
    // A fourth photo-index, so that product is over the limit.
    held.push(stored(
        "ee55555555555555555555555555555555555555555555555555555555555555",
        &app("photo-index", "0.9.0"),
        1,
    ));
    let dropped = evictions(&held, KEEP, Some(&digest(A)));
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert!(
        dropped[0].name.starts_with("photo-index-"),
        "the arrival's protection reached another product: {dropped:?}"
    );
}

// -- the second store the same rule serves: `<volume>/lib`. R-202. ---------
//
// `SPEC-02` §12. These are here rather than beside `crate::package::install`
// because the rule being asserted is **this module's** — grouping by product,
// ordering by version, holding the unreadable apart, never dropping the
// arrival — and the two stores share one implementation of it. A second file
// asserting the same four properties over a different entry type is how one of
// them eventually gets a fifth property the other does not.

mod libraries {
    use crate::package::install::{HeldLibrary, KEEP_LIBRARY, library_evictions};
    use crate::package::library::Reference;

    fn reference(product: &str, version: &str, digest_byte: u8) -> Reference {
        Reference::compose(product, version, &[digest_byte; 32])
            .unwrap_or_else(|_| unreachable!("{product}-{version} is meant to be a reference"))
    }

    fn held(product: &str, version: &str, digest_byte: u8, added: u64) -> HeldLibrary {
        HeldLibrary {
            reference: reference(product, version, digest_byte),
            added,
        }
    }

    fn dropped_names(dropped: &[&HeldLibrary]) -> Vec<String> {
        dropped
            .iter()
            .map(|entry| entry.reference.to_string())
            .collect()
    }

    /// **The case that could not ship.** The same source built twice is two
    /// references at one version, and until R-202 both stayed — which is two
    /// libraries of one class, R-200's refusal, and a dark card on every
    /// upgrade.
    #[test]
    fn two_builds_of_one_product_keep_the_one_that_just_arrived() {
        let old = held("libaura-reading-source", "0.1.0", 0xaa, 1_000);
        let new = held("libaura-reading-source", "0.1.0", 0xbb, 2_000);
        let arriving = new.reference.clone();
        let store = vec![old.clone(), new];

        let dropped = library_evictions(&store, KEEP_LIBRARY, Some(&arriving), &[]);

        assert_eq!(dropped_names(&dropped), vec![old.reference.to_string()]);
    }

    /// And with no arrival named — which is how `cmd/rivet-sysd` asks —
    /// the installation time separates them, newest kept.
    #[test]
    fn with_no_arrival_named_the_newer_installation_is_the_one_kept() {
        let old = held("libaura-reading-source", "0.1.0", 0xbb, 1_000);
        let new = held("libaura-reading-source", "0.1.0", 0xaa, 2_000);
        // Deliberately ordered so that the reference tie-break would choose
        // the other way: `aa` sorts below `bb`, so a rule that fell through to
        // the identity would keep the OLD one.
        let store = vec![old.clone(), new];

        let dropped = library_evictions(&store, KEEP_LIBRARY, None, &[]);

        assert_eq!(dropped_names(&dropped), vec![old.reference.to_string()]);
    }

    /// **Two products of one class are not an upgrade.** R-200's refusal is
    /// written for exactly this and retention must not resolve it by deleting
    /// one — the two `probe-source` libraries on `rivet-06f5c` stand as the
    /// evidence of that refusal.
    #[test]
    fn two_different_products_are_two_groups_and_both_are_kept() {
        let store = vec![
            held("libprobe-source-one", "0.1.0", 0x11, 1_000),
            held("libprobe-source-two", "0.1.0", 0x22, 2_000),
        ];

        let dropped = library_evictions(&store, KEEP_LIBRARY, None, &[]);

        assert!(dropped.is_empty(), "{:?}", dropped_names(&dropped));
    }

    /// **A digit-led product suffix is a different product, not a different
    /// version of one.** `mqtt-5` and `mqtt-6` each hold one build; if either
    /// read as product `mqtt` at version `5-1.0.0` or `6-1.0.0`, one would
    /// count against the other's group and this would evict something.
    #[test]
    fn numeric_suffixed_products_do_not_evict_each_other() {
        let store = vec![
            held("mqtt-5", "1.0.0", 0x11, 1_000),
            held("mqtt-6", "1.0.0", 0x22, 2_000),
        ];

        let dropped = library_evictions(&store, KEEP_LIBRARY, None, &[]);

        assert!(dropped.is_empty(), "{:?}", dropped_names(&dropped));
    }

    /// **The guard.** A reference named in a signed manifest is load-bearing:
    /// evicting it breaks that application at its next start, which is the
    /// failure `SPEC-02` §5 exists to prevent.
    #[test]
    fn a_declared_library_is_never_evicted_however_old_it_is() {
        let declared = held("libgreet", "1.0.0", 0xcc, 1_000);
        let store = vec![
            declared.clone(),
            held("libgreet", "2.0.0", 0xdd, 2_000),
            held("libgreet", "3.0.0", 0xee, 3_000),
        ];

        let dropped = library_evictions(
            &store,
            KEEP_LIBRARY,
            None,
            core::slice::from_ref(&declared.reference),
        );

        assert_eq!(
            dropped_names(&dropped),
            vec![held("libgreet", "2.0.0", 0xdd, 0).reference.to_string()],
            "the declared 1.0.0 is off the table; 3.0.0 is the newest of what is left"
        );
    }

    /// A version this build cannot order is retained **apart** from the ones it
    /// can, exactly as `ADR-052` §7 has it for the artifact store: it is
    /// never deleted first for being unreadable and never pins by outranking.
    #[test]
    fn an_unreadable_version_is_retained_apart() {
        let numbered_old = held("libgreet", "1.0.0", 0xaa, 1_000);
        let numbered_new = held("libgreet", "2.0.0", 0xbb, 2_000);
        let odd = held("libgreet", "0.1.84rc1", 0xcc, 3_000);
        let store = vec![numbered_old.clone(), numbered_new, odd];

        let dropped = library_evictions(&store, KEEP_LIBRARY, None, &[]);

        assert_eq!(
            dropped_names(&dropped),
            vec![numbered_old.reference.to_string()],
            "the unreadable version keeps its own place and costs the numbered ones nothing"
        );
    }

    /// A reference whose head carries no version at all forms a group of one,
    /// so it is never evicted by, or in favour of, anything else.
    #[test]
    fn a_reference_with_no_version_is_a_group_of_its_own() {
        let bare = HeldLibrary {
            reference: Reference::parse(&format!("libgreet-{}", "ab".repeat(32)))
                .unwrap_or_else(|_| unreachable!("a reference with no version")),
            added: 1_000,
        };
        let store = vec![bare, held("libgreet", "1.0.0", 0xcc, 2_000)];

        let dropped = library_evictions(&store, KEEP_LIBRARY, None, &[]);

        assert!(dropped.is_empty(), "{:?}", dropped_names(&dropped));
    }

    /// One answer, whatever order the filesystem listed in.
    #[test]
    fn the_answer_does_not_depend_on_the_listing_order() {
        let mut store = vec![
            held("libgreet", "1.0.0", 0xaa, 1_000),
            held("libgreet", "2.0.0", 0xbb, 1_000),
            held("libgreet", "3.0.0", 0xcc, 1_000),
        ];
        let forwards = dropped_names(&library_evictions(&store, KEEP_LIBRARY, None, &[]));
        store.reverse();
        let backwards = dropped_names(&library_evictions(&store, KEEP_LIBRARY, None, &[]));
        assert_eq!(forwards, backwards);
        assert_eq!(forwards.len(), 2);
    }
}
