//! **What a fleet server does with one machine's document** — `ADR-044`.
//!
//! A repository holds one desired-state document per machine. A fleet server
//! polls it and, for each machine, has to answer one question: *given what the
//! repository asks for, what this machine currently holds, and what was last
//! written to it from the repository — what happens now?*
//!
//! Nothing here reaches a machine, a repository or a disk. It takes three sets
//! of leaves and returns a [`Verdict`]. The whole of the decision is in
//! [`decide`], and it is here rather than in the fleet server because a rule
//! that lives in a `main.rs` is a rule that is only ever exercised against a
//! board.
//!
//! # The three inputs, and why the third one is not optional
//!
//! * **`desired`** — the document in the repository at its current commit.
//! * **`machine`** — the document the machine holds right now, read over the
//!   control plane.
//! * **`baseline`** — the document the repository held at the commit this
//!   server last converged this machine onto, or `None` if it never has.
//!
//! **Without the baseline, drift is indistinguishable from a push.** Both are
//! *the machine and the repository disagree*, and the correct answer is
//! opposite in the two cases: one is what this whole mechanism is for, and the
//! other is the thing `.rulings/` R-145 says must refuse. Two inputs cannot
//! tell them apart, which is why there are three.
//!
//! # The five answers
//!
//! | machine vs desired | baseline | machine vs baseline | verdict |
//! | :-- | :-- | :-- | :-- |
//! | same | — | — | [`Verdict::Converged`] — say nothing |
//! | differs | none | — | [`Verdict::Unadopted`] — refuse |
//! | differs | some | same | [`Verdict::Apply`] — write it |
//! | differs | some | differs | [`Verdict::Drifted`] — refuse and name it |
//!
//! The fifth is [`Verdict::Overwrite`], which is the fourth with the operator's
//! override word: the push carried it, so the drift is written over and the
//! report says what was lost.
//!
//! # Two things this deliberately does not do
//!
//! **It never compares bytes.** A machine's document accumulates the
//! formatting of every `apply edit --append` that has ever run against it —
//! measured on the live board, 2026-09-05, where the file carries ten
//! consecutive blank lines and two tables with no blank line between them. A
//! server that re-wrote a machine because a comment moved would write on every
//! cycle for ever and bury its own journal. So the comparison is over
//! [`crate::leaf`]'s leaves: **the repository is authoritative for values, not
//! for bytes.**
//!
//! **It has nothing to say about ordering across machines.** `.rulings/` R-145.2
//! leaves that open and `ADR-044` §7 argues it; a function that answers about
//! one machine cannot be the place a fleet-wide policy is smuggled in.

use crate::leaf::{Change, Diff, Kind, Leaf, Leaves, between};

/// What a fleet server should do with one machine, this cycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The machine already says what the repository asks for. **Do nothing and
    /// say nothing** — this is the answer on almost every cycle, and a server
    /// that journalled it would hide everything else in the journal.
    Converged,
    /// The repository asks for something else and the machine is exactly what
    /// this server last wrote to it, so nothing has been touched out of band.
    /// Write it.
    Apply {
        /// What the write moves, for the record.
        diff: Diff,
    },
    /// The machine has been changed out of band since this server last wrote
    /// to it, and the repository is now asking for a different document.
    /// **Refuse**, and name what drifted.
    Drifted {
        /// How the machine differs from what was last written to it. This is
        /// the drift, and it is what the operator has to see.
        drift: Diff,
        /// Keys the machine has that the last-written document did not.
        added: Vec<(Vec<String>, String)>,
    },
    /// The same as [`Verdict::Drifted`], with the override word present in the
    /// push. The write happens; the report still names what it overwrote.
    Overwrite {
        /// What is about to be overwritten.
        drift: Diff,
        /// Keys the machine has that the last-written document did not.
        added: Vec<(Vec<String>, String)>,
    },
    /// This server has never converged this machine, and the machine does not
    /// already hold the repository's document. **Refuse.**
    ///
    /// The bootstrap that avoids this is the one `ADR-044` §4 describes: a
    /// machine's current document is copied into the repository verbatim, so
    /// the first cycle is [`Verdict::Converged`] and the baseline is recorded
    /// without anything being written. Reaching this verdict means the file in
    /// the repository was composed rather than captured, and adopting a
    /// machine onto a document nobody has compared is exactly the blind write
    /// the drift rule exists to prevent.
    Unadopted {
        /// How the machine differs from the repository's document.
        difference: Diff,
        /// Keys the repository's document has that the machine does not.
        added: Vec<(Vec<String>, String)>,
    },
}

impl Verdict {
    /// Whether this verdict writes to the machine.
    #[must_use]
    pub const fn writes(&self) -> bool {
        matches!(self, Self::Apply { .. } | Self::Overwrite { .. })
    }

    /// Whether this verdict is a refusal the operator has to act on.
    #[must_use]
    pub const fn refuses(&self) -> bool {
        matches!(self, Self::Drifted { .. } | Self::Unadopted { .. })
    }

    /// Whether this verdict is worth a journal record.
    ///
    /// **[`Verdict::Converged`] is not**, and that is the whole of the
    /// quietness rule: a poller running every minute against a fleet that is
    /// already right must produce no records at all, or the one cycle that
    /// mattered is unfindable.
    #[must_use]
    pub const fn worth_saying(&self) -> bool {
        !matches!(self, Self::Converged)
    }
}

/// The decision, for one machine, this cycle.
///
/// `overridden` is whether the push carried the operator's override word for
/// **this** machine — `ADR-044` §6.3. It is only ever consulted on the drift
/// branch: it cannot turn an unadopted machine into an adopted one, because
/// there is no baseline for it to overwrite and the word is spelled *overwrite
/// the drift*, not *write whatever is in the repository*.
#[must_use]
pub fn decide(
    machine: &Leaves,
    desired: &Leaves,
    baseline: Option<&Leaves>,
    overridden: bool,
) -> Verdict {
    let wanted = both_ways(machine, desired);
    if wanted.0.is_empty() && wanted.1.is_empty() {
        return Verdict::Converged;
    }

    let Some(baseline) = baseline else {
        return Verdict::Unadopted {
            difference: wanted.0,
            added: wanted.1,
        };
    };

    let (drift, added) = both_ways(baseline, machine);
    if drift.is_empty() && added.is_empty() {
        return Verdict::Apply { diff: wanted.0 };
    }
    if overridden {
        Verdict::Overwrite { drift, added }
    } else {
        Verdict::Drifted { drift, added }
    }
}

/// [`between`] plus the additions it cannot see, as one answer.
///
/// `between` is directional by design — its first caller asks *what would this
/// write take away* — so on its own it reports two identical-looking documents
/// when one of them simply has more in it. Every question here is *how do
/// these differ*, and that needs both halves.
///
/// # A third thing `between` cannot see: a list gaining a member or changing order
///
/// An element ADDED to an existing list, or the same elements in a new ORDER,
/// leaves `between` with nothing to report: every element of `from` is still
/// present in `to`, so there is no loss, and the path already existed, so
/// `additions` has nothing either. Two documents that genuinely differ then
/// compare as identical.
///
/// So for every path that is a list on both sides, if the two lists are not
/// the same sequence AND `between` found no loss there for this path —
/// meaning nothing genuinely went missing, only something arrived or moved —
/// this adds ONE [`Change`] carrying both renderings of the whole list.
/// **Only when `between` is silent for that path.** A path where an element
/// really did go missing already has that reported, precisely, by `between`'s
/// own per-element loss; this must not repeat it as a second, vaguer entry
/// for the same path.
fn both_ways(from: &Leaves, to: &Leaves) -> (Diff, Vec<(Vec<String>, String)>) {
    let mut diff = between(from, to);
    name_kind_conflicts(from, to, &mut diff);
    add_invisible_list_changes(from, to, &mut diff);
    (diff, crate::leaf::additions(from, to))
}

/// **When a change's two renderings are the identical text.** `between`
/// already tells a bare `60` apart from a quoted `"60"` (their
/// [`crate::leaf::Scalar`]s differ), so it pushes a [`Change`] for them like
/// any other — but that change's two renderings are the identical text `60`,
/// which reads as `between` repeating itself. This looks the pair back up in
/// `from`/`to` for the [`Kind`] that got erased and names both, so the report
/// says why the line is there at all.
fn name_kind_conflicts(from: &Leaves, to: &Leaves, diff: &mut Diff) {
    for change in &mut diff.changes {
        if change.machine != change.register {
            continue; // an ordinary change; the text already says everything
        }
        let kinds = match (from.get(&change.path), to.get(&change.path)) {
            (Some(Leaf::One(machine)), Some(Leaf::One(register))) => {
                Some((machine.kind, register.kind))
            }
            _ => None,
        };
        let Some((machine_kind, register_kind)) = kinds else { continue };
        change.machine = format!("{} ({})", change.machine, kind_word(machine_kind));
        change.register = format!("{} ({})", change.register, kind_word(register_kind));
    }
}

/// How a report names a [`Kind`], next to the value it belongs to.
fn kind_word(kind: Kind) -> &'static str {
    match kind {
        Kind::String => "quoted",
        Kind::Bare => "unquoted",
    }
}

/// A [`Change`] for every list `between` found nothing missing from, but
/// which is not the same sequence as its counterpart — see [`both_ways`].
fn add_invisible_list_changes(from: &Leaves, to: &Leaves, diff: &mut Diff) {
    let lost: std::collections::HashSet<&Vec<String>> =
        diff.losses.iter().map(|loss| &loss.path).collect();
    for (path, leaf) in from {
        let (Leaf::Many(theirs), Some(Leaf::Many(ours))) = (leaf, to.get(path)) else {
            continue;
        };
        if theirs == ours || lost.contains(path) {
            continue;
        }
        diff.changes.push(Change {
            path: path.clone(),
            machine: leaf.shown(),
            register: crate::leaf::joined(ours),
        });
    }
}

/// The refusal, as an operator reads it.
///
/// **Every key and both values, per key.** `ADR-034` §6.2 already argued this
/// shape for the register's own refusal and the reasoning carries: a report
/// that said *"refused: drift"* leaves an operator to go and find it, and the
/// natural wrong move from there is to overwrite without looking.
#[must_use]
pub fn report(machine: &str, verdict: &Verdict) -> Option<String> {
    match verdict {
        Verdict::Converged => None,
        Verdict::Apply { diff } => {
            let mut out = format!("{machine}: converging onto the repository's document.\n");
            out.push_str(&moved(diff));
            Some(out)
        }
        Verdict::Drifted { drift, added } => {
            let trailer_name = quoted(machine);
            Some(format!(
                "{machine}: REFUSED. NOTHING HAS BEEN WRITTEN and the machine is exactly as it \
                 was.\n\
                 \x20 why  this machine has been changed since the repository last wrote to it, \
                 and the repository is now asking for a different document. Applying it would \
                 silently undo the change below, which nobody recorded a reason for.\n\
                 {}\
                 \x20 fix  either put the change into the repository - it is the fleet's \
                 definition, and a value only a machine knows is a value the next apply \
                 deletes - or, if the machine is the one that is wrong, add the line\n\
                 \x20        Fleet-Override: overwrite-drift {trailer_name}\n\
                 \x20      to the commit message of the push that should win. The word is \
                 per-push and per-machine on purpose: a standing permission in a file would \
                 authorise every future push as well.\n",
                drifted(drift, added),
            ))
        }
        Verdict::Overwrite { drift, added } => Some(format!(
            "{machine}: the push carried `overwrite-drift`, so the repository wins. \
             OVERWRITING:\n{}",
            drifted(drift, added),
        )),
        Verdict::Unadopted { difference, added } => Some(format!(
            "{machine}: REFUSED. NOTHING HAS BEEN WRITTEN and the machine is exactly as it \
             was.\n\
             \x20 why  this server has never converged this machine, and the document in the \
             repository is not the one the machine is running. There is nothing to compare \
             against, so writing it would be a blind overwrite of a machine nobody has \
             looked at.\n\
             {}\
             \x20 fix  read the machine's own document and commit THAT as the starting \
             point - `rivet get desired-state` prints exactly the file this repository \
             holds. The first cycle is then a no-op, the machine is adopted, and every \
             change after it is a diff somebody can read.\n",
            drifted(difference, added),
        )),
    }
}

/// The moved values, indented, for a report.
fn moved(diff: &Diff) -> String {
    let mut out = String::new();
    for change in &diff.changes {
        out.push_str(&format!(
            "\x20 {}   {} -> {}\n",
            change.where_it_is(),
            change.machine,
            change.register
        ));
    }
    for loss in &diff.losses {
        out.push_str(&format!(
            "\x20 {}   {} -> (removed)\n",
            loss.where_it_is(),
            loss.value
        ));
    }
    out
}

/// One drift, key by key, with both values.
fn drifted(diff: &Diff, added: &[(Vec<String>, String)]) -> String {
    let mut out = String::new();
    for change in &diff.changes {
        out.push_str(&format!("\x20 {}\n", change.where_it_is()));
        // The names are the two authorities, spelled out. `Change::machine` is
        // whichever document was passed first, which for a drift report is the
        // one the repository last wrote.
        out.push_str(&format!("      repository  {}\n", change.machine));
        out.push_str(&format!("      machine     {}\n", change.register));
    }
    for loss in &diff.losses {
        out.push_str(&format!("\x20 {}\n", loss.where_it_is()));
        out.push_str(&format!("      repository  {}\n", loss.value));
        out.push_str("      machine     (not there)\n");
    }
    for (path, value) in added {
        out.push_str(&format!("\x20 {}\n", path.join(".")));
        out.push_str("      repository  (not there)\n");
        out.push_str(&format!("      machine     {value}\n"));
    }
    out
}

/// The trailer a commit message carries to overwrite one machine's drift.
///
/// **In the commit message and not in a file**, `ADR-044` §6.3: a file in the
/// repository is a standing permission that authorises every future push as
/// well, and the one thing R-145's refusal is protecting is the *next* push
/// nobody thought about. A trailer belongs to one commit, is signed by
/// whatever signs the commit, and is in the history with an author and a date.
pub const OVERRIDE_TRAILER: &str = "Fleet-Override:";

/// The word itself, after the trailer and before the machine's name.
pub const OVERRIDE_WORD: &str = "overwrite-drift";

/// Whether a commit message overrides the drift refusal for `machine`.
///
/// **The machine must be named.** A blanket override would clear the drift on
/// a machine the operator was not thinking about, which is the failure the
/// refusal exists to prevent arriving through the door marked *fix*.
///
/// **A name is a bare, whitespace-delimited word, or double-quoted** —
/// [`crate::fleet_as_code::check_label`] allows a space in a label, and
/// splitting the line on whitespace alone would then read "kitchen pi" as the
/// two machines "kitchen" and "pi" rather than the one it is. [`report`]
/// prints the same form this reads. A bare word is unchanged, so every
/// trailer already written keeps meaning what it always meant.
#[must_use]
pub fn overrides(message: &str, machine: &str) -> bool {
    message.lines().any(|line| {
        let line = line.trim();
        let Some(rest) = line.strip_prefix(OVERRIDE_TRAILER) else {
            return false;
        };
        let rest = rest.trim_start();
        let Some(after_word) = rest.strip_prefix(OVERRIDE_WORD) else {
            return false;
        };
        if !after_word.is_empty() && !after_word.starts_with(char::is_whitespace) {
            return false; // a longer word, e.g. `overwrite-driftx` - not this one
        }
        let Some(names) = trailer_names(after_word) else {
            return false; // an unterminated quote authorises nothing on this line
        };
        names.iter().any(|name| name == machine)
    })
}

/// The machine names after `overwrite-drift` on one trailer line.
///
/// Each is a bare, whitespace-delimited word, taken literally, or a
/// double-quoted name with `\"` and `\\` as the only two escapes - the form
/// [`quoted`] writes for a name that contains whitespace.
///
/// `None` for an opening quote with no matching close: the line does not say
/// where the name was meant to end, so nothing on it is authorised rather
/// than guessed.
fn trailer_names(rest: &str) -> Option<Vec<String>> {
    let mut names = Vec::new();
    let mut chars = rest.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let Some(&next) = chars.peek() else { return Some(names) };
        if next == '"' {
            chars.next();
            let mut name = String::new();
            let mut closed = false;
            while let Some(c) = chars.next() {
                match c {
                    '"' => {
                        closed = true;
                        break;
                    }
                    '\\' => match chars.next() {
                        Some(escaped @ ('"' | '\\')) => name.push(escaped),
                        Some(other) => {
                            name.push('\\');
                            name.push(other);
                        }
                        None => return None,
                    },
                    other => name.push(other),
                }
            }
            if !closed {
                return None;
            }
            names.push(name);
        } else {
            let mut name = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                name.push(c);
                chars.next();
            }
            names.push(name);
        }
    }
}

/// How a machine's name is written after `overwrite-drift`: bare, or
/// double-quoted with `"` and `\` escaped, when the name contains whitespace,
/// a `"` or a `\`. A name with none of those is written bare and unchanged,
/// so every trailer already written keeps meaning what it always meant.
///
/// **A `"` quotes the name as well as whitespace does**, because
/// [`trailer_names`] reads a word that begins with one as the start of a
/// quoted name: `check_label` allows a `"`, and a label like `"lab"` written
/// bare would come back as `lab`. `check_label` refuses a `\` outright, so a
/// label never needs that escape today; it is still read back by
/// [`trailer_names`], which is the other half of this pair and has to agree
/// with it rather than with today's grammar.
fn quoted(machine: &str) -> String {
    if !machine.chars().any(|c| c.is_whitespace() || c == '"' || c == '\\') {
        return machine.to_owned();
    }
    let mut out = String::with_capacity(machine.len() + 2);
    out.push('"');
    for ch in machine.chars() {
        if ch == '"' || ch == '\\' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

/// Every drifted or changed key in a verdict, by path — for a caller that
/// wants the names without the prose.
#[must_use]
pub fn keys(verdict: &Verdict) -> Vec<String> {
    let (diff, added): (&Diff, &[(Vec<String>, String)]) = match verdict {
        Verdict::Converged => return Vec::new(),
        Verdict::Apply { diff } => (diff, &[]),
        Verdict::Drifted { drift, added } | Verdict::Overwrite { drift, added } => (drift, added),
        Verdict::Unadopted { difference, added } => (difference, added),
    };
    let mut out: Vec<String> = diff
        .changes
        .iter()
        .map(Change::where_it_is)
        .chain(diff.losses.iter().map(crate::leaf::Loss::where_it_is))
        .chain(added.iter().map(|(path, _)| path.join(".")))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::leaf::Scalar;

    fn doc(pairs: &[(&str, &str)]) -> Leaves {
        pairs
            .iter()
            .map(|(path, value)| {
                (
                    path.split('.').map(str::to_owned).collect::<Vec<String>>(),
                    Leaf::One(Scalar::bare(*value)),
                )
            })
            .collect()
    }

    const AS_SHIPPED: &[(&str, &str)] = &[
        ("version", "1"),
        ("hostname", "rivet-01"),
        ("timezone", "America/Chicago"),
        ("services.rivet-mcpd.enabled", "true"),
    ];

    /// **The quiet cycle**, and the most important test here. A poller that
    /// journalled this would produce a record a minute for ever.
    #[test]
    fn a_machine_that_already_agrees_is_converged_and_silent() {
        let same = doc(AS_SHIPPED);
        let verdict = decide(&same, &same, Some(&same), false);
        assert_eq!(verdict, Verdict::Converged);
        assert!(!verdict.worth_saying(), "a converged machine must say nothing");
        assert!(!verdict.writes());
        assert_eq!(report("rivet-01", &verdict), None);
    }

    /// And it is converged whether or not it has ever been adopted — the
    /// bootstrap `ADR-044` §4 describes, where the repository is seeded with
    /// the machine's own document and the first cycle writes nothing.
    #[test]
    fn the_first_cycle_after_a_verbatim_copy_writes_nothing() {
        let same = doc(AS_SHIPPED);
        assert_eq!(decide(&same, &same, None, false), Verdict::Converged);
    }

    /// **A push, on a machine nobody has touched.** This is the path the whole
    /// mechanism exists for.
    #[test]
    fn a_changed_repository_against_an_untouched_machine_applies() {
        let baseline = doc(AS_SHIPPED);
        let machine = baseline.clone();
        let mut desired = baseline.clone();
        desired.insert(vec!["timezone".to_owned()], Leaf::One(Scalar::bare("UTC")));

        let verdict = decide(&machine, &desired, Some(&baseline), false);
        let Verdict::Apply { ref diff } = verdict else {
            panic!("{verdict:?}");
        };
        assert_eq!(diff.changes.len(), 1, "{diff:?}");
        assert_eq!(diff.changes[0].machine, "America/Chicago");
        assert_eq!(diff.changes[0].register, "UTC");
        assert!(verdict.writes());
    }

    /// **The refusal.** The machine was edited out of band and the repository
    /// wants something different. Nothing is written.
    #[test]
    fn a_push_that_would_overwrite_drift_refuses() {
        let baseline = doc(AS_SHIPPED);
        let mut machine = baseline.clone();
        machine.insert(vec!["hostname".to_owned()], Leaf::One(Scalar::bare("renamed")));
        let mut desired = baseline.clone();
        desired.insert(vec!["timezone".to_owned()], Leaf::One(Scalar::bare("UTC")));

        let verdict = decide(&machine, &desired, Some(&baseline), false);
        let Verdict::Drifted { ref drift, .. } = verdict else {
            panic!("{verdict:?}");
        };
        assert!(!verdict.writes(), "a refusal must not write");
        assert!(verdict.refuses());
        assert_eq!(drift.changes.len(), 1, "{drift:?}");
    }

    /// **And the refusal names the key and BOTH values**, which is the
    /// difference between a report an operator can act on and one they cannot.
    #[test]
    fn the_refusal_names_the_key_and_both_values() {
        let baseline = doc(AS_SHIPPED);
        let mut machine = baseline.clone();
        machine.insert(vec!["hostname".to_owned()], Leaf::One(Scalar::bare("renamed")));
        let mut desired = baseline.clone();
        desired.insert(vec!["timezone".to_owned()], Leaf::One(Scalar::bare("UTC")));

        let verdict = decide(&machine, &desired, Some(&baseline), false);
        let said = report("rivet-01", &verdict).expect("a refusal says why");
        assert!(said.contains("hostname"), "the key is not named: {said}");
        assert!(said.contains("rivet-01"), "the repository's value is missing: {said}");
        assert!(said.contains("renamed"), "the machine's value is missing: {said}");
        assert!(said.contains("NOTHING HAS BEEN WRITTEN"), "{said}");
        assert_eq!(keys(&verdict), vec!["hostname".to_owned()]);
    }

    /// A value ADDED to the machine out of band is drift too, and `between`
    /// alone cannot see it — the reason [`both_ways`] exists.
    #[test]
    fn a_table_added_to_the_machine_by_hand_is_drift() {
        let baseline = doc(AS_SHIPPED);
        let mut machine = baseline.clone();
        machine.insert(
            vec!["services".to_owned(), "rivet-mdnsd".to_owned(), "enabled".to_owned()],
            Leaf::One(Scalar::bare("true")),
        );
        let mut desired = baseline.clone();
        desired.insert(vec!["timezone".to_owned()], Leaf::One(Scalar::bare("UTC")));

        let verdict = decide(&machine, &desired, Some(&baseline), false);
        let Verdict::Drifted { ref added, .. } = verdict else {
            panic!("{verdict:?}");
        };
        assert_eq!(added.len(), 1, "{added:?}");
        let said = report("rivet-01", &verdict).expect("a refusal");
        assert!(said.contains("services.rivet-mdnsd.enabled"), "{said}");
        // **And its value.** A key named without its value is the one line of
        // the report an operator cannot act on.
        assert!(said.contains("machine     true"), "{said}");
    }

    /// The override word turns the refusal into a write, and the report still
    /// names what it overwrote.
    #[test]
    fn the_override_word_overwrites_and_still_reports() {
        let baseline = doc(AS_SHIPPED);
        let mut machine = baseline.clone();
        machine.insert(vec!["hostname".to_owned()], Leaf::One(Scalar::bare("renamed")));
        let mut desired = baseline.clone();
        desired.insert(vec!["timezone".to_owned()], Leaf::One(Scalar::bare("UTC")));

        let verdict = decide(&machine, &desired, Some(&baseline), true);
        assert!(verdict.writes(), "{verdict:?}");
        let said = report("rivet-01", &verdict).expect("it still says what it did");
        assert!(said.contains("renamed"), "{said}");
    }

    /// **The override cannot adopt.** There is no baseline to overwrite, so the
    /// word does not apply and the refusal stands.
    #[test]
    fn the_override_word_does_not_adopt_an_unknown_machine() {
        let machine = doc(AS_SHIPPED);
        let mut desired = machine.clone();
        desired.insert(vec!["timezone".to_owned()], Leaf::One(Scalar::bare("UTC")));

        for overridden in [false, true] {
            let verdict = decide(&machine, &desired, None, overridden);
            assert!(
                matches!(verdict, Verdict::Unadopted { .. }),
                "overridden={overridden}: {verdict:?}"
            );
            assert!(!verdict.writes());
        }
    }

    /// A trailer naming another machine does not override this one.
    #[test]
    fn the_override_trailer_must_name_this_machine() {
        let message = "The kitchen pi takes the new resolver\n\n\
                       Fleet-Override: overwrite-drift kitchen-pi\n";
        assert!(overrides(message, "kitchen-pi"));
        assert!(!overrides(message, "rivet-01"));
    }

    /// And a message that merely mentions the words does not.
    #[test]
    fn prose_about_the_override_is_not_an_override() {
        for message in [
            "I nearly used Fleet-Override overwrite-drift rivet-01 here and did not\n",
            "Fleet-Override: rivet-01\n",
            "Fleet-Override: overwrite\n",
            "Override: overwrite-drift rivet-01\n",
        ] {
            assert!(!overrides(message, "rivet-01"), "{message}");
        }
    }

    /// One trailer may name several machines, because one push often changes
    /// several — and each still has to be named.
    #[test]
    fn one_trailer_may_name_more_than_one_machine() {
        let message = "Fleet-Override: overwrite-drift rivet-01 kitchen-pi\n";
        assert!(overrides(message, "rivet-01"));
        assert!(overrides(message, "kitchen-pi"));
        assert!(!overrides(message, "garage-pi"));
    }

    /// **A label may contain a space** — `check_label` allows it — so a bare
    /// split on whitespace reads "kitchen pi" as the two machines "kitchen"
    /// and "pi" rather than the one it is. Quoted, it overrides only the
    /// whole name.
    #[test]
    fn a_quoted_label_containing_whitespace_overrides_only_the_whole_name() {
        let message = "Fleet-Override: overwrite-drift \"kitchen pi\"\n";
        assert!(overrides(message, "kitchen pi"));
        assert!(!overrides(message, "kitchen"));
        assert!(!overrides(message, "pi"));
    }

    /// Bare words are unchanged, including several on one line, and a line
    /// may mix a bare word with a quoted name.
    #[test]
    fn bare_words_keep_their_meaning_and_mix_freely_with_a_quoted_name() {
        let bare = "Fleet-Override: overwrite-drift rivet-01 kitchen-pi\n";
        assert!(overrides(bare, "rivet-01"));
        assert!(overrides(bare, "kitchen-pi"));

        let mixed = "Fleet-Override: overwrite-drift rivet-01 \"kitchen pi\"\n";
        assert!(overrides(mixed, "rivet-01"));
        assert!(overrides(mixed, "kitchen pi"));
    }

    /// **An opening quote with no closing one authorises nothing on that
    /// line** — not even the bare word that came before it — rather than
    /// guessing where the name was meant to end.
    #[test]
    fn an_unterminated_quote_grants_nothing_on_its_line() {
        let message = "Fleet-Override: overwrite-drift rivet-01 \"kitchen pi\n";
        assert!(!overrides(message, "rivet-01"));
        assert!(!overrides(message, "kitchen pi"));
        assert!(!overrides(message, "kitchen"));
    }

    /// **The round trip.** What [`report`] prints as the fix for a drifted
    /// machine is what [`overrides`] must accept back — for a label with a
    /// space and for a plain one — pinning the printer and the parser to each
    /// other.
    #[test]
    fn overrides_accepts_the_trailer_line_report_prints() {
        let baseline = doc(AS_SHIPPED);
        let mut machine = baseline.clone();
        machine.insert(vec!["hostname".to_owned()], Leaf::One(Scalar::bare("renamed")));
        let mut desired = baseline.clone();
        desired.insert(vec!["timezone".to_owned()], Leaf::One(Scalar::bare("UTC")));
        let verdict = decide(&machine, &desired, Some(&baseline), false);

        for label in ["kitchen pi", "rivet-01", "\"quoted\"", "kitchen\"pi"] {
            let said = report(label, &verdict).expect("a refusal names the fix");
            let trailer = said
                .lines()
                .find(|line| line.trim_start().starts_with(OVERRIDE_TRAILER))
                .unwrap_or_else(|| panic!("no {OVERRIDE_TRAILER} line in:\n{said}"));
            assert!(overrides(trailer, label), "{trailer:?} did not authorise {label:?}");
        }
    }
}
