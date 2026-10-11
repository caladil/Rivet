//! **Fleet convergence sees every difference between two documents.**
//!
//! These parse real TOML through [`crate::leaves::leaves`] and hand the
//! result to `rivet_core::converge::decide`, rather than hand-building
//! [`rivet_core::leaf::Leaf`] values directly: both defects live in what
//! happens BETWEEN a document's text and its leaves — `rivet-config`'s own
//! `scalar()`/`entry()` — so a hand-built fixture would skip the exact step
//! that is under test.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use rivet_core::converge::{Verdict, decide, report};
use rivet_core::leaf::Leaves;

use crate::leaves::leaves;

fn doc(text: &str) -> Leaves {
    leaves(text).unwrap_or_else(|e| panic!("{text} did not parse: {e}"))
}

// ---------------------------------------------------------------------------
// An addition or a reorder inside an existing list
// ---------------------------------------------------------------------------

/// **The finding's own example.** A member holds one resolver search domain;
/// the desired document extends the list. With the machine as its own
/// baseline this is a plain push and must APPLY — `between` alone finds no
/// loss (nothing on the machine is missing from the desired list) and
/// `additions` alone finds no new path (`network.resolver.search` already
/// exists), so before the fix the two together read as `Converged` and the
/// new domain is never sent.
#[test]
fn a_list_addition_against_an_unchanged_baseline_applies() {
    let machine = doc("version = 1\n[network.resolver]\nsearch = [\"east.example\"]\n");
    let desired =
        doc("version = 1\n[network.resolver]\nsearch = [\"east.example\", \"west.example\"]\n");

    let verdict = decide(&machine, &desired, Some(&machine), false);
    assert!(matches!(verdict, Verdict::Apply { .. }), "{verdict:?}");
}

/// **A reorder alone**, nothing added or removed. `between` calls this no
/// loss on purpose (`rivet-core/src/leaf.rs`'s own
/// `a_reordered_list_is_not_a_loss`), and that stays true — but convergence
/// still has to see the document differ, because a resolver's `nameservers`
/// and `search` are tried in the order they are written.
#[test]
fn a_reordered_list_is_not_converged() {
    let machine =
        doc("version = 1\n[network.resolver]\nnameservers = [\"192.0.2.1\", \"192.0.2.2\"]\n");
    let desired =
        doc("version = 1\n[network.resolver]\nnameservers = [\"192.0.2.2\", \"192.0.2.1\"]\n");

    let verdict = decide(&machine, &desired, Some(&machine), false);
    assert!(!matches!(verdict, Verdict::Converged), "{verdict:?}");
}

/// **The drift half of the same gap.** The machine gained a list element out
/// of band, relative to the baseline; the desired document changes something
/// unrelated. Before the fix this is a silent `Apply` that overwrites the
/// local addition with no warning at all — the machine's own addition is
/// simply gone, replaced by whatever the old, un-extended list said.
#[test]
fn a_local_list_addition_is_drift_not_a_silent_apply() {
    let baseline =
        doc("version = 1\ntimezone = \"UTC\"\n[network.resolver]\nsearch = [\"east.example\"]\n");
    let machine = doc(
        "version = 1\ntimezone = \"UTC\"\n[network.resolver]\nsearch = [\"east.example\", \
         \"local.example\"]\n",
    );
    let desired = doc(
        "version = 1\ntimezone = \"America/Chicago\"\n[network.resolver]\nsearch = \
         [\"east.example\"]\n",
    );

    let verdict = decide(&machine, &desired, Some(&baseline), false);
    assert!(matches!(verdict, Verdict::Drifted { .. }), "{verdict:?}");

    let overwritten = decide(&machine, &desired, Some(&baseline), true);
    assert!(matches!(overwritten, Verdict::Overwrite { .. }), "{overwritten:?}");
}

/// Two spellings of the same document — an extra comment, reflowed blank
/// lines — still converge, and so does a document compared with itself.
#[test]
fn identical_lists_and_reformatted_documents_still_converge() {
    let machine = doc(
        "# a comment nobody asked for\nversion = 1\n[network.resolver]\nnameservers = \
         [\"192.0.2.1\", \"192.0.2.2\"]\n",
    );
    let desired = doc(
        "version = 1\n\n\n[network.resolver]\nnameservers = [\"192.0.2.1\", \"192.0.2.2\"]\n",
    );

    assert_eq!(decide(&machine, &desired, Some(&machine), false), Verdict::Converged);
    assert_eq!(decide(&machine, &machine, Some(&machine), false), Verdict::Converged);
}

// ---------------------------------------------------------------------------
// A bare TOML literal and a quoted string render identically
// ---------------------------------------------------------------------------

/// **The finding's own example.** `brightness = 60` on the machine,
/// `brightness = "60"` in the repository. `scalar()` renders both as the text
/// `60`, so before the fix the two compare equal and the correction is never
/// sent. Both directions must see a real change.
#[test]
fn a_bare_versus_quoted_scalar_applies_in_both_directions() {
    let bare = doc("version = 1\n[applications.aura-renderer]\nbrightness = 60\n");
    let quoted = doc("version = 1\n[applications.aura-renderer]\nbrightness = \"60\"\n");

    let verdict = decide(&bare, &quoted, Some(&bare), false);
    assert!(matches!(verdict, Verdict::Apply { .. }), "bare -> quoted: {verdict:?}");

    let verdict = decide(&quoted, &bare, Some(&quoted), false);
    assert!(matches!(verdict, Verdict::Apply { .. }), "quoted -> bare: {verdict:?}");
}

/// The same collision for a boolean, and for a single-element list.
#[test]
fn boolean_and_list_element_collisions_are_not_invisible() {
    let bare_bool = doc("version = 1\n[services.rivet-mdnsd]\nenabled = true\n");
    let quoted_bool = doc("version = 1\n[services.rivet-mdnsd]\nenabled = \"true\"\n");
    assert!(matches!(
        decide(&bare_bool, &quoted_bool, Some(&bare_bool), false),
        Verdict::Apply { .. }
    ));

    let bare_list = doc("version = 1\n[network.resolver]\nsearch = [60]\n");
    let quoted_list = doc("version = 1\n[network.resolver]\nsearch = [\"60\"]\n");
    assert!(!matches!(
        decide(&bare_list, &quoted_list, Some(&bare_list), false),
        Verdict::Converged
    ));
}

/// **The report names both types instead of printing the same text twice.**
/// Every other part of the line — the path, the header sentence — reads
/// exactly as it does for an ordinary change.
#[test]
fn a_type_only_change_names_both_types_in_the_report() {
    let bare = doc("version = 1\n[applications.aura-renderer]\nbrightness = 60\n");
    let quoted = doc("version = 1\n[applications.aura-renderer]\nbrightness = \"60\"\n");

    let verdict = decide(&bare, &quoted, Some(&bare), false);
    let said = report("rivet-01", &verdict).expect("a change is reported");
    assert!(
        said.starts_with("rivet-01: converging onto the repository's document.\n"),
        "{said}"
    );
    assert!(said.contains("applications.aura-renderer.brightness"), "{said}");
    assert!(said.contains("(unquoted)"), "{said}");
    assert!(said.contains("(quoted)"), "{said}");
    assert!(!said.contains("60 -> 60"), "printed the identical value on both sides: {said}");
}
