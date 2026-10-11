//! **Two desired-state documents, compared leaf by leaf.**
//!
//! This is the vocabulary for *what writing one document over another would
//! do*: what it would take away, and what it would move. It says nothing about
//! who is allowed to do it or what should happen next — those are the callers'.
//!
//! # It was `cmd/rivet-fleet`'s and it moved here, because a second caller
//! arrived
//!
//! `ADR-034` §6.2 needs it: `rivet-fleet apply document` reads the machine's
//! current document, diffs it against the one the register composes, and
//! REFUSES if the write would remove anything the register does not account
//! for. That refusal was measured against real hardware on 2026-08-29 — the
//! composed document was missing `[control-plane]` *and*
//! `[services.rivet-mcpd]`, so one apply would have dropped the listener to
//! loopback and turned the control plane off, on a headless machine, at once.
//!
//! `ADR-044` needs the same comparison for a different question: a fleet
//! server converging a machine onto the document a git repository holds has to
//! be able to say **which keys drifted and to what**, and a refusal that said
//! only *"drift"* would be useless. Two implementations of *"how do these two
//! documents differ"* is the second authority `ADR-011` is about, and they
//! would eventually disagree about the one case that matters.
//!
//! # There is no parser here, and that is the crate boundary working
//!
//! Reading TOML needs `toml_edit`, and this crate depends on nothing that
//! touches the world. So the *types* and the *comparison* live here, where both
//! callers can reach them with no dependency at all, and
//! `rivet_config::leaves` is the reader that turns a document's text into
//! them — in the crate that already owns the schema and already carries the
//! parser.

use std::collections::BTreeMap;

/// The one TOML type distinction [`Leaf`] equality has to keep.
///
/// **A bare literal and a quoted string can render to the same text.** `60`
/// and `"60"` — a bare integer and a quoted string — render to
/// the same text, and so do `true` and `"true"`. `rivet_config::leaves`'s
/// `scalar()` renders every kind of value **unquoted**, because the text goes
/// straight into a command line a refusal composes, and that rendering is
/// deliberately kept — see [`Scalar`]. Comparing leaves by that text alone
/// let a machine holding the bare value and a document asking for the quoted
/// one compare equal, so a correction that mattered was never sent.
///
/// **Only string versus everything else.** An integer, a float, a boolean
/// and a datetime already render to different text from one another, so
/// nothing here needs to tell those apart — the one pairing whose text can
/// coincide is a bare literal against a quoted string of the same digits or
/// word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Came from a TOML string.
    String,
    /// Came from anything else: an integer, a float, a boolean, a datetime,
    /// or an already-rendered nested array or inline table (see
    /// `rivet_config::leaves`'s `rendered`, which compares a nested array or
    /// table by its whole rendered text and does not distinguish the TOML
    /// kinds of the values inside it).
    Bare,
}

/// A scalar's rendered text, plus enough of its TOML kind to tell `60` from
/// `"60"` — see [`Kind`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scalar {
    /// As an operator would type it — **unquoted**, because it goes into a
    /// command line a refusal composes. Unaffected by [`Scalar::kind`].
    pub text: String,
    /// A TOML string, or anything else.
    pub kind: Kind,
}

impl Scalar {
    /// A value that came from a TOML string.
    #[must_use]
    pub fn string(text: impl Into<String>) -> Self {
        Self { text: text.into(), kind: Kind::String }
    }

    /// A value that came from anything else TOML can hold as a scalar.
    #[must_use]
    pub fn bare(text: impl Into<String>) -> Self {
        Self { text: text.into(), kind: Kind::Bare }
    }
}

/// One comparable leaf of a document.
///
/// **A list is its own kind rather than one rendered scalar**, because losing
/// one element of a nameserver list is a loss of that nameserver and not a change of
/// the array — and a path-only comparison would call it a change and let it
/// through. That distinction is the reason this enum exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Leaf {
    /// A scalar: a string, a boolean, a number, a date.
    One(Scalar),
    /// A list of scalars, compared element by element and **not** by order.
    ///
    /// An empty one is also what an empty *table* flattens to — see
    /// `rivet_config::leaves`, which has the reason.
    Many(Vec<Scalar>),
}

impl Leaf {
    /// This leaf as a report prints it.
    ///
    /// **A list is joined rather than rendered as TOML**, because what reads
    /// it is a person comparing it with the value on the next line, not a
    /// parser.
    #[must_use]
    pub fn shown(&self) -> String {
        match self {
            Self::One(scalar) => scalar.text.clone(),
            Self::Many(values) => joined(values),
        }
    }
}

/// [`Leaf::Many`]'s elements, joined the way a report prints them.
///
/// Shared rather than inlined into [`Leaf::shown`] alone: `crate::converge`
/// needs the identical join to render one whole list against another when a
/// list gained a member or changed order — see its `both_ways`.
pub(crate) fn joined(values: &[Scalar]) -> String {
    if values.is_empty() {
        EMPTY.to_owned()
    } else {
        values.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(", ")
    }
}

/// Every leaf of a document, by its path from the root.
///
/// **Keyed by the path's segments and not by a dotted string.** A segment
/// containing a `.` — which TOML permits and quotes — would make a joined path
/// ambiguous, and this map is what a caller matches on to name a command.
pub type Leaves = BTreeMap<Vec<String>, Leaf>;

/// A value the machine has that the other document does not carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loss {
    /// Where it is in the document.
    pub path: Vec<String>,
    /// What the machine currently holds there.
    pub value: String,
    /// `true` when the whole leaf goes; `false` when one element of a list
    /// does and the rest survive.
    pub whole: bool,
}

impl Loss {
    /// The path as a reader sees it.
    #[must_use]
    pub fn where_it_is(&self) -> String {
        self.path.join(".")
    }
}

/// A value both documents carry and disagree about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// Where it is in the document.
    pub path: Vec<String>,
    /// What the machine holds now.
    pub machine: String,
    /// What the other document would write.
    pub register: String,
}

impl Change {
    /// The path as a reader sees it.
    #[must_use]
    pub fn where_it_is(&self) -> String {
        self.path.join(".")
    }
}

/// What writing one document over another would do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    /// Values that would go.
    pub losses: Vec<Loss>,
    /// Values that would move.
    pub changes: Vec<Change>,
}

impl Diff {
    /// Whether the two documents say the same thing.
    ///
    /// **Not whether they are the same bytes.** Two documents with the same
    /// leaves differ only in comment, order and whitespace, and a caller that
    /// re-wrote a machine's document over that difference would write on every
    /// cycle for ever.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.losses.is_empty() && self.changes.is_empty()
    }
}

/// Compare what the machine holds against what the other document says.
///
/// **Directional.** A leaf the other document adds is not reported: adding is
/// what applying a document is for.
#[must_use]
pub fn between(machine: &Leaves, composed: &Leaves) -> Diff {
    let mut diff = Diff::default();
    for (path, theirs) in machine {
        match (theirs, composed.get(path)) {
            (Leaf::One(value), None) => diff.losses.push(Loss {
                path: path.clone(),
                value: value.text.clone(),
                whole: true,
            }),
            (Leaf::Many(values), None) => {
                if values.is_empty() {
                    diff.losses.push(Loss {
                        path: path.clone(),
                        value: EMPTY.to_owned(),
                        whole: true,
                    });
                } else {
                    for value in values {
                        diff.losses.push(Loss {
                            path: path.clone(),
                            value: value.text.clone(),
                            whole: true,
                        });
                    }
                }
            }
            // **`theirs != ours` compares the KIND too, not only the text** —
            // `Scalar`'s derived equality is what makes `60` and `"60"` two
            // different values here even though they render identically.
            (Leaf::One(theirs), Some(Leaf::One(ours))) => {
                if theirs != ours {
                    diff.changes.push(Change {
                        path: path.clone(),
                        machine: theirs.text.clone(),
                        register: ours.text.clone(),
                    });
                }
            }
            (Leaf::Many(theirs), Some(Leaf::Many(ours))) => {
                for value in theirs {
                    if !ours.contains(value) {
                        diff.losses.push(Loss {
                            path: path.clone(),
                            value: value.text.clone(),
                            whole: false,
                        });
                    }
                }
            }
            // A leaf that is a scalar on one side and a list on the other. The
            // machine's value does not survive, so it is a loss rather than a
            // change — the safe reading, and the one that cannot let a value
            // out of the document quietly.
            (Leaf::One(value), Some(Leaf::Many(_))) => diff.losses.push(Loss {
                path: path.clone(),
                value: value.text.clone(),
                whole: true,
            }),
            (Leaf::Many(values), Some(Leaf::One(_))) => {
                for value in values {
                    diff.losses.push(Loss {
                        path: path.clone(),
                        value: value.text.clone(),
                        whole: true,
                    });
                }
            }
        }
    }
    diff
}

/// What an empty table or an empty list reads as in a report.
pub const EMPTY: &str = "(declared, and empty)";

/// What the OTHER document carries that the machine does not.
///
/// [`between`] is deliberately one-directional, because its caller
/// (`ADR-034` §6.2) is asking *what would this write take away*. A caller
/// asking *how do these two differ* — `ADR-044`'s drift report — needs the
/// additions as well, and an addition is not a `Loss`: nothing is being
/// removed from anything.
///
/// **The value comes back with the path.** A refusal has to name the key *and*
/// both values, and a path on its own would make a table somebody added by
/// hand the one entry in the report an operator cannot read.
#[must_use]
pub fn additions(machine: &Leaves, other: &Leaves) -> Vec<(Vec<String>, String)> {
    other
        .iter()
        .filter(|(path, _)| !machine.contains_key(*path))
        .map(|(path, leaf)| (path.clone(), leaf.shown()))
        .collect()
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    // **`Scalar::bare` throughout**, not because these values are bare TOML
    // literals — most are not — but because every test below is about
    // `between`'s LOSS/CHANGE logic over the text, not about the
    // string-versus-bare kind distinction, and a single consistent kind on
    // both sides of every comparison here reduces `Scalar` equality back to
    // the plain text equality these tests were written against.
    fn one(path: &[&str], value: &str) -> (Vec<String>, Leaf) {
        (
            path.iter().map(|s| (*s).to_owned()).collect(),
            Leaf::One(Scalar::bare(value)),
        )
    }

    fn many(path: &[&str], values: &[&str]) -> (Vec<String>, Leaf) {
        (
            path.iter().map(|s| (*s).to_owned()).collect(),
            Leaf::Many(values.iter().map(|s| Scalar::bare(*s)).collect()),
        )
    }

    /// Two identical documents produce nothing, which is the case every quiet
    /// cycle of a poller depends on.
    #[test]
    fn documents_that_agree_produce_no_diff() {
        let leaves: Leaves = [one(&["hostname"], "rivet-01"), many(&["network", "resolver"], &["a", "b"])]
            .into_iter()
            .collect();
        assert!(between(&leaves, &leaves).is_empty());
    }

    /// A list compares by membership and not by order, so a reordered array is
    /// not a loss.
    #[test]
    fn a_reordered_list_is_not_a_loss() {
        let machine: Leaves = [many(&["network", "resolver"], &["a", "b"])].into_iter().collect();
        let other: Leaves = [many(&["network", "resolver"], &["b", "a"])].into_iter().collect();
        assert!(between(&machine, &other).is_empty());
    }

    /// One element gone out of a list is a loss of that element, not a change
    /// of the array — the distinction [`Leaf::Many`] exists for.
    #[test]
    fn one_element_of_a_list_going_is_a_partial_loss() {
        let machine: Leaves = [many(&["network", "resolver"], &["a", "b"])].into_iter().collect();
        let other: Leaves = [many(&["network", "resolver"], &["a"])].into_iter().collect();
        let diff = between(&machine, &other);
        assert_eq!(diff.losses.len(), 1, "{diff:?}");
        assert_eq!(diff.losses[0].value, "b");
        assert!(!diff.losses[0].whole, "the rest of the list survives");
    }

    /// A scalar that moved is a change carrying BOTH values, which is what a
    /// drift report has to print.
    #[test]
    fn a_changed_scalar_carries_both_values() {
        let machine: Leaves = [one(&["timezone"], "UTC")].into_iter().collect();
        let other: Leaves = [one(&["timezone"], "America/Chicago")].into_iter().collect();
        let diff = between(&machine, &other);
        assert_eq!(diff.changes.len(), 1, "{diff:?}");
        assert_eq!(diff.changes[0].machine, "UTC");
        assert_eq!(diff.changes[0].register, "America/Chicago");
        assert_eq!(diff.changes[0].where_it_is(), "timezone");
    }

    /// **`between` cannot see an addition, and `additions` is why that is not
    /// a defect.** A drift report that listed only removals and changes would
    /// silently omit a table somebody added to a machine by hand.
    #[test]
    fn an_addition_is_invisible_to_between_and_visible_to_additions() {
        let machine: Leaves = [one(&["hostname"], "rivet-01")].into_iter().collect();
        let other: Leaves = [
            one(&["hostname"], "rivet-01"),
            one(&["services", "rivet-mdnsd", "enabled"], "true"),
        ]
        .into_iter()
        .collect();
        assert!(between(&machine, &other).is_empty(), "between is directional");
        let added = additions(&machine, &other);
        assert_eq!(added.len(), 1, "{added:?}");
        assert_eq!(added[0].0, vec!["services", "rivet-mdnsd", "enabled"]);
        assert_eq!(added[0].1, "true", "an addition without its value is unreadable");
    }

    /// A list and an empty table each read as something a person can compare.
    #[test]
    fn a_leaf_shows_itself_in_a_form_a_report_can_print() {
        assert_eq!(Leaf::One(Scalar::bare("v")).shown(), "v");
        assert_eq!(Leaf::Many(vec![Scalar::bare("a"), Scalar::bare("b")]).shown(), "a, b");
        assert_eq!(Leaf::Many(Vec::new()).shown(), EMPTY);
    }

    /// `60` and `"60"` render to the same text and must not compare equal —
    /// nor may `true` and `"true"`, nor a bare list element against a quoted
    /// one.
    #[test]
    fn a_bare_scalar_and_a_quoted_string_of_the_same_text_are_different_leaves() {
        assert_ne!(Leaf::One(Scalar::bare("60")), Leaf::One(Scalar::string("60")));
        assert_ne!(Leaf::One(Scalar::bare("true")), Leaf::One(Scalar::string("true")));
        assert_ne!(
            Leaf::Many(vec![Scalar::bare("60")]),
            Leaf::Many(vec![Scalar::string("60")])
        );
        // And the ordinary cases are unaffected: same kind, same text, equal;
        // same kind, different text, unequal.
        assert_eq!(Leaf::One(Scalar::bare("60")), Leaf::One(Scalar::bare("60")));
        assert_ne!(Leaf::One(Scalar::bare("60")), Leaf::One(Scalar::bare("61")));
    }

    /// A leaf that is a scalar on one side and a list on the other loses the
    /// scalar — the safe reading.
    #[test]
    fn a_scalar_that_became_a_list_is_a_loss() {
        let machine: Leaves = [one(&["x"], "v")].into_iter().collect();
        let other: Leaves = [many(&["x"], &["v"])].into_iter().collect();
        let diff = between(&machine, &other);
        assert_eq!(diff.losses.len(), 1, "{diff:?}");
        assert!(diff.losses[0].whole);
    }
}
