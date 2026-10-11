//! **A unified diff, computed in the domain.** `ADR-006` §8.10.6 and §8.10.7,
//! ruled by the maintainer, Joel Lindell, 2026-08-29 (`.rulings/` R-365): the
//! administration surface should have a diff built in.
//!
//! # Where a diff earns its place, and the first is the one that matters
//!
//! **1. In the plan of every subject that changes a file's contents.**
//! `ADR-012` §7.1 makes plan-before-apply the *safety* mechanism rather than the
//! friendly one. A plan that says *"3 bytes will change"* is a much weaker
//! safety mechanism than one that shows the change: an operator whose
//! `--expect` matched the wrong one of two similar stanzas cannot tell from a
//! byte count and can tell instantly from a diff.
//!
//! **This is output, not input.** `ADR-006` §8.9.3's rule that `apply edit`
//! grows refusals rather than expressiveness is untouched — nothing about what
//! an operator may *ask for* has changed.
//!
//! **2. As `rivet get diff <path> <text>`** — the read-side answer to *"what
//! would change if I wrote this?"*, which closes `edit.rs`'s own complaint
//! against itself from the other side: an agent that can diff before it writes
//! does not have to be lucky about `--expect`.
//!
//! **3. Against the projection — NOT BUILT, and `ADR-006` §8.10.14 says why.**
//! Drift detection needs to render what the document *would* generate, which
//! needs `rivet_config::Document` — a type this crate may not name, permanently
//! (`ADR-014` §3.2). It is specified there and left for a ruling rather than
//! bodged in.
//!
//! # No crate was added, and that was decided rather than assumed
//!
//! `ADR-001`'s dependency posture and `ADR-009`'s reuse posture both point the
//! same way here, and `ADR-014` §3 makes the decision for the core outright:
//! **`rivet-core`'s `[dependencies]` table is empty by construction**, so the
//! build refuses a diff crate rather than a human noticing. The alternatives
//! were to put the algorithm in an adapter — where `ADR-014` §3.1 says it could
//! not be tested without I/O — or to write it. It is about two hundred lines.
//!
//! # It is bounded, and it says when the bound bit
//!
//! A longest-common-subsequence table is O(n·m), and this runs in the login
//! shell on a board with no swap. So: common prefix and
//! suffix are trimmed first, which is free and is the whole answer for the
//! single contiguous replacement `apply edit` makes; and above
//! [`LCS_MAX_CELLS`](diff::LCS_MAX_CELLS) the middle is not searched at all — it becomes one hunk of
//! *everything removed, everything added*, and the answer carries
//! `algorithm: coarse` so nobody reads it as a minimal diff.
//!
//! **Degrading and saying so is the same rule `get memory` follows for absent
//! pressure**: report what was computed, never a plausible substitute for what
//! was not.

use crate::value::Value;

/// The most cells a longest-common-subsequence table may have.
///
/// **250,000**, which is a `u32` table of about a megabyte — a bound chosen for
/// the board rather than for the workstation. Above it the diff is
/// [`Diff::coarse`] and says so.
pub const LCS_MAX_CELLS: usize = 250_000;

/// The most lines of diff this will emit before it stops.
///
/// **400.** A diff longer than this is not one anybody reads before typing
/// `confirm`, which is the thing it exists for. What matters more than the
/// number is [`Diff::truncated`]: a diff cut short that did not say so reads as
/// a complete description of a change it does not fully describe, which is worse
/// than no diff at all.
pub const DIFF_MAX_LINES: usize = 400;

/// How many unchanged lines surround a change.
const CONTEXT: usize = 3;

/// One computed difference between two texts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Diff {
    /// The unified diff itself, `---`/`+++` header and all. **Empty when the
    /// two texts are identical**, which [`Self::changed`] is the honest way to
    /// ask about.
    pub text: String,
    /// Lines only in the second text.
    pub added: usize,
    /// Lines only in the first.
    pub removed: usize,
    /// Whether [`Self::text`] was cut at [`DIFF_MAX_LINES`]. **The counts above
    /// are of the WHOLE difference either way** — they are computed before the
    /// text is cut, so a truncated diff still says how big the change is.
    pub truncated: bool,
    /// Whether the middle was too large to search and became one blunt hunk.
    /// See the module documentation.
    pub coarse: bool,
    /// **Every line's content is the same and the bytes are not** — a final
    /// newline added or removed, or a line-ending style changed (CRLF to LF,
    /// say). `split` compares line CONTENT, the same as `str::lines`, so a
    /// difference of this shape produces no added or removed line and would
    /// otherwise be indistinguishable from two byte-identical texts. Set
    /// instead of a line-level hunk, because there is no line to show as
    /// changed — see [`Self::note`].
    pub line_endings_only: bool,
}

impl Diff {
    /// Whether the two texts differ at all.
    ///
    /// **Byte-identical is not the same question as line-identical.** Two
    /// texts whose lines are all equal can still be different files — see
    /// [`Self::line_endings_only`] — and writing one over the other would be
    /// a real write. A caller asking *"would writing this change anything"*
    /// needs this answer, not `self.added > 0 || self.removed > 0` alone.
    pub const fn changed(&self) -> bool {
        self.added > 0 || self.removed > 0 || self.line_endings_only
    }

    /// The structured form, for a result or a plan's note.
    ///
    /// **Every field, always, including the booleans.** A consumer that had
    /// to infer `truncated` from the absence of a key would infer it wrong on
    /// the day the key was omitted for a different reason.
    pub fn value(&self) -> Value {
        Value::object(vec![
            ("changed", Value::Bool(self.changed())),
            ("added_lines", Value::Integer(count(self.added))),
            ("removed_lines", Value::Integer(count(self.removed))),
            ("line_endings_only", Value::Bool(self.line_endings_only)),
            ("truncated", Value::Bool(self.truncated)),
            (
                "algorithm",
                Value::str(if self.coarse { "coarse" } else { "lines" }),
            ),
            ("diff", Value::str(self.text.clone())),
        ])
    }

    /// The diff as a plan's note reads it — a sentence, then the diff.
    ///
    /// **A plan's note is prose that an operator reads at the moment they are
    /// deciding** (`super::plan::Effect::note`), so the diff is introduced
    /// rather than dropped in, and a truncated or coarse one says so *before*
    /// the text rather than after it.
    pub fn note(&self) -> String {
        if self.line_endings_only {
            return "THE LINES ARE THE SAME AND THE BYTES ARE NOT — only line endings or a \
                    final newline differ, which is a real change to the file and not the \
                    identical text it would look like line by line"
                .to_string();
        }
        if !self.changed() {
            return "THE TEXT IS IDENTICAL — this change would rewrite the file to exactly what \
                    it already holds"
                .to_string();
        }
        let mut lead = format!(
            "{added} line(s) added, {removed} removed",
            added = self.added,
            removed = self.removed,
        );
        if self.coarse {
            lead.push_str(
                ". THIS DIFF IS COARSE: the change is too large to line up minimally, so it is \
                 shown as everything removed and everything added rather than as a smallest \
                 edit",
            );
        }
        if self.truncated {
            lead.push_str(&format!(
                ". THE DIFF BELOW IS CUT AT {DIFF_MAX_LINES} LINES and is NOT the whole change \
                 — the counts above are"
            ));
        }
        format!("{lead}:\n{}", self.text)
    }
}

/// A count that has to become an `i64` for [`Value::Integer`].
///
/// Saturating rather than `as`: a length that wrapped negative would make the
/// field saying how big a change is unreadable, and this crate denies the lint
/// that would have caught it.
fn count(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// One step of an edit script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Same,
    Removed,
    Added,
}

/// **Compute the unified diff of `before` into `after`.**
///
/// `before_label` and `after_label` are what the `---` and `+++` header lines
/// name. They are the caller's — a plan says `a/<path>` and `b/<path>`, and
/// `get diff` says the path and *"the text you supplied"* — because the two
/// answer different questions and a shared label would answer neither.
pub fn unified(before: &str, after: &str, before_label: &str, after_label: &str) -> Diff {
    let a: Vec<&str> = split(before);
    let b: Vec<&str> = split(after);

    // **Line content equal is not bytes equal.** `split` throws away line
    // terminators exactly as `str::lines` does, so a final newline added or
    // removed, or CRLF written where the file had LF, produces the same `a`
    // and `b` here as two truly identical texts — and must not be reported
    // the same way (`AGENTS.md` §5: report what the bytes are, not a
    // line-level summary that cannot see the difference).
    if a == b {
        return if before == after {
            Diff::default()
        } else {
            Diff {
                line_endings_only: true,
                ..Diff::default()
            }
        };
    }

    // The common prefix and suffix are free, and for `apply edit` — one
    // contiguous replacement — they are the whole answer.
    let mut head = 0usize;
    while head < a.len() && head < b.len() && a[head] == b[head] {
        head += 1;
    }
    let mut tail = 0usize;
    while tail < a.len() - head && tail < b.len() - head && a[a.len() - 1 - tail] == b[b.len() - 1 - tail] {
        tail += 1;
    }

    let mid_a = &a[head..a.len() - tail];
    let mid_b = &b[head..b.len() - tail];

    let coarse = mid_a.len().saturating_mul(mid_b.len()) > LCS_MAX_CELLS;
    let middle = if coarse {
        // **Everything removed, everything added.** Not a minimal diff and it
        // does not claim to be — `Diff::coarse` is what says so.
        let mut steps = vec![Step::Removed; mid_a.len()];
        steps.extend(core::iter::repeat_n(Step::Added, mid_b.len()));
        steps
    } else {
        script(mid_a, mid_b)
    };

    let mut steps = vec![Step::Same; head];
    steps.extend(middle);
    steps.extend(core::iter::repeat_n(Step::Same, tail));

    let added = steps.iter().filter(|s| **s == Step::Added).count();
    let removed = steps.iter().filter(|s| **s == Step::Removed).count();

    let (text, truncated) = render(&steps, &a, &b, before_label, after_label);
    Diff {
        text,
        added,
        removed,
        truncated,
        coarse,
        line_endings_only: false,
    }
}

/// A text as lines, with an **empty text being no lines at all**.
///
/// `str::lines` already does this, and the distinction is worth naming: an empty
/// file has zero lines and a file holding one newline has one empty line. A
/// diff that confused the two would report a change nobody made.
fn split(text: &str) -> Vec<&str> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.lines().collect()
    }
}

/// The edit script for two slices, by longest common subsequence.
///
/// Only ever reached when `a.len() * b.len()` is within [`LCS_MAX_CELLS`], which
/// is what makes the full table affordable — see the module documentation.
fn script(a: &[&str], b: &[&str]) -> Vec<Step> {
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        return vec![Step::Added; m];
    }
    if m == 0 {
        return vec![Step::Removed; n];
    }

    // `table[i * (m + 1) + j]` is the length of the longest common subsequence
    // of `a[i..]` and `b[j..]`, so the walk below runs forwards and the script
    // comes out in order without a reversal.
    let mut table = vec![0u32; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let at = i * (m + 1) + j;
            table[at] = if a[i] == b[j] {
                table[at + m + 2] + 1
            } else {
                table[at + m + 1].max(table[at + 1])
            };
        }
    }

    let mut steps = Vec::with_capacity(n + m);
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        let at = i * (m + 1) + j;
        if a[i] == b[j] {
            steps.push(Step::Same);
            i += 1;
            j += 1;
        } else if table[at + m + 1] >= table[at + 1] {
            // **Removals before additions at a tie**, which is what makes a
            // replaced line read as `-old` then `+new` rather than the reverse.
            steps.push(Step::Removed);
            i += 1;
        } else {
            steps.push(Step::Added);
            j += 1;
        }
    }
    steps.extend(core::iter::repeat_n(Step::Removed, n - i));
    steps.extend(core::iter::repeat_n(Step::Added, m - j));
    steps
}

/// The script as unified-diff text, and whether it had to be cut.
fn render(
    steps: &[Step],
    a: &[&str],
    b: &[&str],
    before_label: &str,
    after_label: &str,
) -> (String, bool) {
    // Which steps belong to a hunk: any change, plus CONTEXT either side.
    let mut keep = vec![false; steps.len()];
    for (at, step) in steps.iter().enumerate() {
        if *step == Step::Same {
            continue;
        }
        let from = at.saturating_sub(CONTEXT);
        let to = (at + CONTEXT + 1).min(steps.len());
        for slot in keep.iter_mut().take(to).skip(from) {
            *slot = true;
        }
    }

    let mut out = format!("--- {before_label}\n+++ {after_label}\n");
    let mut lines = 2usize;
    let mut truncated = false;

    let (mut ai, mut bi) = (0usize, 0usize);
    let mut at = 0usize;
    while at < steps.len() {
        if !keep[at] {
            match steps[at] {
                Step::Same => {
                    ai += 1;
                    bi += 1;
                }
                Step::Removed => ai += 1,
                Step::Added => bi += 1,
            }
            at += 1;
            continue;
        }

        // One hunk: everything from here to the next unkept step.
        let (hunk_a, hunk_b) = (ai, bi);
        let start = at;
        let (mut count_a, mut count_b) = (0usize, 0usize);
        while at < steps.len() && keep[at] {
            match steps[at] {
                Step::Same => {
                    count_a += 1;
                    count_b += 1;
                }
                Step::Removed => count_a += 1,
                Step::Added => count_b += 1,
            }
            at += 1;
        }

        // Line numbers count from 1, and a zero-length side is numbered from the
        // line before it — the convention every unified diff uses, stated
        // because getting it wrong produces a diff no tool will apply.
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            if count_a == 0 { hunk_a } else { hunk_a + 1 },
            count_a,
            if count_b == 0 { hunk_b } else { hunk_b + 1 },
            count_b,
        ));
        lines += 1;

        for step in &steps[start..at] {
            if lines >= DIFF_MAX_LINES {
                truncated = true;
                break;
            }
            match step {
                Step::Same => {
                    out.push(' ');
                    out.push_str(a[ai]);
                    ai += 1;
                    bi += 1;
                }
                Step::Removed => {
                    out.push('-');
                    out.push_str(a[ai]);
                    ai += 1;
                }
                Step::Added => {
                    out.push('+');
                    out.push_str(b[bi]);
                    bi += 1;
                }
            }
            out.push('\n');
            lines += 1;
        }
        if truncated {
            out.push_str("... this diff is cut here; it is not the whole change\n");
            break;
        }
    }

    (out, truncated)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn diff(before: &str, after: &str) -> Diff {
        unified(before, after, "a", "b")
    }

    #[test]
    fn identical_texts_produce_no_diff_and_say_nothing_changed() {
        let d = diff("one\ntwo\n", "one\ntwo\n");
        assert!(!d.changed());
        assert_eq!(d.added, 0);
        assert_eq!(d.removed, 0);
        assert_eq!(d.text, "");
        assert!(d.note().contains("IDENTICAL"));
        assert!(!d.line_endings_only);
    }

    /// **`str::lines` cannot see a final newline, so neither could this
    /// diff, before this fix.** `"value"` and `"value\n"` split to the same
    /// one line, and that made them read as identical — a plan that would in
    /// fact rewrite the file said nothing would change.
    #[test]
    fn adding_a_final_newline_is_reported_as_changed_and_never_identical() {
        let d = diff("value", "value\n");
        assert!(d.changed(), "adding a trailing newline must count as a change");
        assert!(d.line_endings_only);
        assert_eq!((d.added, d.removed), (0, 0), "no LINE was added or removed");
        assert!(!d.note().contains("IDENTICAL"), "{}", d.note());
    }

    /// The same defect, the other direction: removing the one newline a file
    /// ends with.
    #[test]
    fn removing_a_final_newline_is_reported_as_changed_and_never_identical() {
        let d = diff("value\n", "value");
        assert!(d.changed());
        assert!(d.line_endings_only);
        assert_eq!((d.added, d.removed), (0, 0));
        assert!(!d.note().contains("IDENTICAL"), "{}", d.note());
    }

    /// **Line-ending STYLE, not just presence.** `str::lines` treats `\r\n`
    /// and `\n` as the same terminator, so CRLF text and LF text with
    /// identical content split to the same lines and must not be reported as
    /// identical either — the file on disk would still change, byte for
    /// byte.
    #[test]
    fn crlf_to_lf_is_reported_as_changed_and_never_identical() {
        let d = diff("one\r\ntwo\r\n", "one\ntwo\n");
        assert!(d.changed());
        assert!(d.line_endings_only);
        assert_eq!((d.added, d.removed), (0, 0));
        assert!(!d.note().contains("IDENTICAL"), "{}", d.note());
    }

    /// **And the note says which of the two things happened**, so an
    /// operator reading it is not left guessing why `changed` is true with
    /// nothing added or removed.
    #[test]
    fn a_line_endings_only_change_says_so_in_the_note_rather_than_showing_an_empty_diff() {
        let d = diff("value", "value\n");
        let note = d.note();
        assert!(note.to_lowercase().contains("line ending"), "{note}");
        assert!(!note.contains("line(s) added"), "{note}");
    }

    /// **The case `apply edit` actually makes**: one line replaced, everything
    /// else the same. The prefix/suffix trim is the whole answer here and the
    /// diff must be minimal, not a whole-file rewrite.
    #[test]
    fn one_replaced_line_is_one_removal_and_one_addition() {
        let d = diff(
            "alpha\nbanner = true\ngamma\n",
            "alpha\nbanner = false\ngamma\n",
        );
        assert_eq!((d.removed, d.added), (1, 1));
        assert!(d.text.contains("-banner = true\n"), "{}", d.text);
        assert!(d.text.contains("+banner = false\n"), "{}", d.text);
        assert!(d.text.contains(" alpha\n"), "{}", d.text);
        // Removal before addition, so a replacement reads in the order it
        // happened.
        let minus = d.text.find("-banner").expect("a removal");
        let plus = d.text.find("+banner").expect("an addition");
        assert!(minus < plus, "{}", d.text);
        assert!(!d.coarse);
        assert!(!d.truncated);
    }

    #[test]
    fn creating_a_file_is_every_line_added_and_an_empty_text_has_no_lines() {
        let d = diff("", "one\ntwo\n");
        assert_eq!((d.removed, d.added), (0, 2));
        let empty = diff("", "");
        assert!(!empty.changed());
    }

    #[test]
    fn emptying_a_file_is_every_line_removed() {
        let d = diff("one\ntwo\n", "");
        assert_eq!((d.removed, d.added), (2, 0));
    }

    /// The hunk header is the convention every tool reads, and a zero-length
    /// side is numbered from the line before it.
    #[test]
    fn the_hunk_header_numbers_from_one_and_counts_both_sides() {
        let d = diff("a\nb\nc\n", "a\nc\n");
        assert!(d.text.contains("@@ -1,3 +1,2 @@"), "{}", d.text);
    }

    /// **A cut diff says it was cut.** Reporting a partial change as though it
    /// were the whole one is worse than reporting no diff at all.
    #[test]
    fn a_diff_longer_than_the_cap_is_truncated_and_says_so_in_both_places() {
        let before = String::new();
        let after: String = (0..DIFF_MAX_LINES * 2)
            .map(|n| format!("line {n}\n"))
            .collect();
        let d = unified(&before, &after, "a", "b");
        assert!(d.truncated);
        // The counts are of the WHOLE change even though the text is cut.
        assert_eq!(d.added, DIFF_MAX_LINES * 2);
        assert!(d.text.contains("cut here"), "{}", d.text);
        assert!(d.note().contains("CUT AT"), "{}", d.note());
        assert!(d.text.lines().count() <= DIFF_MAX_LINES + 2);
    }

    /// Above the bound the middle is not searched, and the answer says
    /// `coarse` rather than pretending to be minimal.
    #[test]
    fn a_change_too_large_to_line_up_degrades_and_says_which_algorithm_ran() {
        let n = 700; // 700 * 700 = 490,000 cells, over LCS_MAX_CELLS
        let before: String = (0..n).map(|i| format!("a{i}\n")).collect();
        let after: String = (0..n).map(|i| format!("b{i}\n")).collect();
        let d = unified(&before, &after, "a", "b");
        assert!(d.coarse, "the bound did not bite");
        assert_eq!((d.removed, d.added), (n, n));
        assert!(d.note().contains("COARSE"), "{}", d.note());
        assert_eq!(
            d.value(),
            Value::object(vec![
                ("changed", Value::Bool(true)),
                ("added_lines", Value::Integer(count(n))),
                ("removed_lines", Value::Integer(count(n))),
                ("line_endings_only", Value::Bool(false)),
                ("truncated", Value::Bool(d.truncated)),
                ("algorithm", Value::str("coarse")),
                ("diff", Value::str(d.text.clone())),
            ])
        );
    }

    /// Under the bound it really is a longest-common-subsequence: a line moved
    /// from the top to the bottom is one removal and one addition, not a
    /// rewrite of everything between.
    #[test]
    fn a_moved_line_is_not_a_whole_file_rewrite() {
        let d = diff("x\na\nb\nc\nd\n", "a\nb\nc\nd\nx\n");
        assert_eq!((d.removed, d.added), (1, 1));
    }

    #[test]
    fn the_labels_are_the_callers_and_appear_in_the_header() {
        let d = unified("a\n", "b\n", "/state/x", "the text you supplied");
        assert!(d.text.starts_with("--- /state/x\n+++ the text you supplied\n"));
    }
}
