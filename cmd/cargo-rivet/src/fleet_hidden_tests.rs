//! The hidden set in `manifest.toml` — `ADR-052` §7: written and read by this
//! module, carried through a publish, and honoured by `fleet list`.
//!
//! The manifests here are rendered by [`render_manifest`] itself, or written
//! by hand in the shape it writes (said where they are); the artifacts are
//! real, signed by a seed minted in a scratch directory.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::sync::atomic::{AtomicU32, Ordering};

use super::*;

fn scratch(label: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    crate::scratch::dir(&format!("fleet-hidden-{label}-{}", NEXT.fetch_add(1, Ordering::Relaxed)))
}

fn entry(digest_fill: char, product: &str, version: &str, kind: &str) -> Entry {
    Entry {
        digest: digest_fill.to_string().repeat(64),
        product: product.to_owned(),
        version: version.to_owned(),
        arch: if kind == "apk" { "any" } else { "aarch64" }.to_owned(),
        platform: (kind == "apk").then(|| "android".to_owned()),
        kind: kind.to_owned(),
        size: 30,
        added: "2026-09-27T00:00:00Z".to_owned(),
        schema_version: None,
    }
}

fn hidden(product: &str, kind: &str) -> Hidden {
    Hidden { product: product.to_owned(), kind: kind.to_owned() }
}

/// **The hidden set round-trips**, and is written canonically: sorted and
/// deduplicated, whatever order and repetition it was handed in.
#[test]
fn the_hidden_set_round_trips_through_render_and_read_in_canonical_order() {
    let dir = scratch("round-trip");
    let entries = vec![
        entry('a', "testbed", "0.1.3", "apk"),
        entry('b', "hello", "0.1.0", "application"),
        entry('c', "gantry", "0.1.17", "apk"),
    ];
    let given = vec![hidden("testbed", "apk"), hidden("gantry", "apk"), hidden("testbed", "apk")];
    let text = render_manifest(&entries, &given);
    std::fs::write(dir.join(MANIFEST_FILE), &text).expect("the manifest");

    assert_eq!(read_manifest(&dir).expect("entries"), entries);
    assert_eq!(read_hidden(&dir).expect("hidden"), vec![hidden("gantry", "apk"), hidden("testbed", "apk")]);
    // Rendering what was read gives the same bytes: the file is a function of
    // the set, not of the order it was built in.
    let again = render_manifest(&read_manifest(&dir).expect("e"), &read_hidden(&dir).expect("h"));
    assert_eq!(again, text);
    assert_eq!(text.matches("[[hidden]]").count(), 2, "{text}");
}

/// **A manifest written before the hidden set existed renders byte for byte
/// as it did** — the hand-written text below is the shape `render` has always
/// written, a card image's schema version included.
#[test]
fn an_old_shape_manifest_is_unchanged_byte_for_byte_after_a_render() {
    let dir = scratch("old-shape");
    let old = format!(
        "# The fleet repository's manifest — ADR-052 §2. Each [[artifact]] names\n\
         # one artifact this repository holds, by the BLAKE3 digest its bytes are\n\
         # stored under in objects/ (ADR-052 §3). Written by `cargo rivet fleet\n\
         # publish`; hand-editing it will not move or rename anything in objects/.\n\
         \n[[artifact]]\ndigest  = \"{a}\"\nproduct = \"hello\"\nversion = \"0.1.0\"\n\
         arch    = \"aarch64\"\nkind    = \"application\"\nsize    = 30\nadded   = \"2026-09-27T00:00:00Z\"\n\
         \n[[artifact]]\ndigest  = \"{b}\"\nproduct = \"rivet\"\nversion = \"0.1.117\"\n\
         arch    = \"aarch64\"\nplatform = \"rpi4\"\nkind    = \"card\"\nsize    = 30\n\
         added   = \"2026-09-27T00:00:00Z\"\nschema_version = 3\n",
        a = "a".repeat(64),
        b = "b".repeat(64),
    );
    std::fs::write(dir.join(MANIFEST_FILE), &old).expect("the manifest");
    let hidden = read_hidden(&dir).expect("no hidden set");
    assert!(hidden.is_empty());
    assert_eq!(render_manifest(&read_manifest(&dir).expect("entries"), &hidden), old);
}

/// A `[[hidden]]` that does not read is refused by name, never read around: a
/// hidden set read around would send what the operator hid.
#[test]
fn a_hidden_entry_without_a_kind_is_refused() {
    let dir = scratch("malformed");
    std::fs::write(dir.join(MANIFEST_FILE), "[[hidden]]\nproduct = \"testbed\"\n").expect("the manifest");
    let why = read_hidden(&dir).expect_err("refused");
    assert!(why.contains("[[hidden]] (entry 1) with no string kind"), "{why}");
}

/// **A publish keeps the hidden set**, and a new version of a hidden product
/// is hidden too — the set names a product, not a digest.
#[test]
fn a_publish_carries_the_hidden_set_through() {
    let dir = scratch("publish");
    init(&dir).expect("init");
    let fixtures = scratch("publish-fixtures");
    let (first, key) = super::tests::fixture(&fixtures, "testbed", "0.1.0", 31);
    publish(&dir, &first, &key).expect("published");

    let entries = read_manifest(&dir).expect("entries");
    std::fs::write(dir.join(MANIFEST_FILE), render_manifest(&entries, &[hidden("testbed", "application")]))
        .expect("the manifest");
    git(&dir, &["commit", "-q", "--allow-empty", "-am", "hide testbed"]).expect("commit");

    let (second, key) = super::tests::fixture(&fixtures, "testbed", "0.1.1", 31);
    publish(&dir, &second, &key).expect("published");
    assert_eq!(read_hidden(&dir).expect("hidden"), vec![hidden("testbed", "application")]);
    assert_eq!(read_manifest(&dir).expect("entries").len(), 2);
    let listed = list(&dir).expect("list");
    assert!(!listed.contains("testbed 0.1.1"), "{listed}");
}

/// **`fleet list` folds a hidden product into one line, and `--hidden` shows
/// it marked.**
#[test]
fn list_folds_hidden_products_and_hidden_shows_them_marked() {
    let dir = scratch("list");
    let entries = vec![
        entry('a', "testbed", "0.1.3", "apk"),
        entry('d', "testbed", "0.1.4", "apk"),
        entry('b', "hello", "0.1.0", "application"),
    ];
    std::fs::write(dir.join(MANIFEST_FILE), render_manifest(&entries, &[hidden("testbed", "apk")]))
        .expect("the manifest");

    let folded = list(&dir).expect("list");
    assert!(folded.contains("hello 0.1.0"), "{folded}");
    assert!(!folded.contains("testbed 0.1.3"), "{folded}");
    assert!(folded.contains("1 hidden, withheld from every fleet"), "{folded}");
    assert!(folded.contains("testbed (apk)"), "{folded}");

    let shown = list_showing(&dir, true).expect("list --hidden");
    assert!(shown.contains("[hidden] aaaaaaaaaaaa  testbed 0.1.3"), "{shown}");
    assert!(shown.contains("[hidden] dddddddddddd  testbed 0.1.4"), "{shown}");
    assert!(shown.contains("bbbbbbbbbbbb  hello 0.1.0"), "{shown}");
    assert!(!shown.contains("[hidden] bbbb"), "{shown}");
}

/// `--hidden` belongs to `list` alone.
#[test]
fn hidden_is_a_flag_of_list_only() {
    let words = |list: &[&str]| list.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
    match parse(&words(&["list", "--hidden"])).expect("parses") {
        FleetArgs { verb: SubVerb::List { show_hidden }, .. } => assert!(show_hidden),
        _ => panic!("not a list"),
    }
    let refused = parse(&words(&["plan", "rivet-01", "--hidden"])).err().expect("refused");
    assert!(refused.contains("--hidden"), "{refused}");
}
