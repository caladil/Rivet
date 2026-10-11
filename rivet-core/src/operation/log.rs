//! `rivet get log` — **`ADR-022` §9's query, and the machine's account of
//! itself.**
//!
//! > `rivet log` is an operation in the catalogue (`ADR-006` §6.4), emitting
//! > structured output by default and rendering text for a terminal — filtered
//! > by service, severity, time window, correlation identifier or actor.
//! >
//! > Because it is a catalogued operation rather than a special case, it is
//! > reachable from every adapter (`ADR-006` §3.1): the CLI now, the dashboard
//! > later, an MCP tool for an agent diagnosing its own failed `apply`.
//!
//! # It is `get log` and not `log`, and that is a conflict in the record rather
//! # than a preference
//!
//! **`ADR-022` §9 writes `rivet log`. `ADR-006` §6.4 forbids it.** §6.4's
//! verb set is closed — `get`, `set`, `list`, `add`, `remove`, `apply`, `plan`,
//! `export`, `explain` — and says it "is extended only by amending this ADR".
//! `ADR-006` §8.4 and §8.5 each faced the same question, for `reboot` and for
//! `shutdown`, and each answered it the same way: *"`AGENTS.md` §1 puts adding a
//! tenth out of an agent's reach."*
//!
//! So this is `get`, with `log` as a subject — exactly the shape `apply` takes
//! for its four subjects, and priced in `ADR-006` §8.5.1. **Nothing about the
//! operation depends on which it is**: the verb is one line in `super::get`
//! and a match arm there, so a ruling either way was cheap. The maintainer's
//! ruling settled it: `get log` stays.
//!
//! **This module was `get.rs` until 2026-08-24**, and the rename is the same
//! restructure `apply` took when it grew a second subject: the *verb* is what
//! has a catalogue entry, so the entry lives in `super::get` and each subject
//! declares only its own [`Field`] list. Nothing about the operation changed.
//!
//! # Records come back as their own lines, which is a decision
//!
//! Each record in the result is **the JSONL line the machine wrote**, verbatim,
//! not a re-encoding of it through [`crate::value::Value`]. Three reasons, and
//! the first is the one that decides it:
//!
//! 1. **`ADR-022` §3 makes the JSONL form authoritative.** Re-encoding would
//!    produce a second form of the same record, and `ADR-011` is about exactly
//!    that. A reader comparing the two would have no way to tell which was
//!    right.
//! 2. **`Value` cannot carry a record without being widened twice.** Its object
//!    keys are `&'static str` and a record's field names are the *writer's*
//!    (`ADR-022` §3.2), and it has no floating point while `mono` is one.
//!    `crate::value`'s own documentation says both widenings are for the day
//!    something needs them; taking two of them for this would be a change to
//!    every operation's output type, which is `ADR-006` §7's shape.
//! 3. **It is what a pipeline wants anyway.** `ADR-005` §5.1 ruled nushell the
//!    login shell precisely so structured output composes:
//!
//!    ```text
//!    rivet get log | from json | get records | each { from json } | where sev == error
//!    ```
//!
//!    The [`Field`] descriptions below carry that incantation, because the
//!    catalogue is what an agent reads (`ADR-006` §7).
//!
//! # What this operation deliberately does not do
//!
//! **Filter by service, severity, correlation or actor.** `ADR-022` §9 lists
//! all of those and [`crate::journal::Filter`] implements every one — they are
//! reachable, tested, and not wired to an argument. The reason is `ADR-006`
//! §6.4's positional shape: five optional positional arguments would be the
//! polymorphic-input problem `ADR-006` §8.5.1 already priced once, and the
//! machine that reads this has a **shell** on it now that
//! does the same job better. What is wired is the one selection a shell cannot
//! do afterwards — **which boot** — because that depends on the order the
//! journal was appended in and the records do not carry it.
//!
//! **That absence used to be silent, and it cost a cold agent a finding on
//! 2026-08-30.** The machine's own install answers ended *"read it with `rivet
//! get log rivet-sysd`"* — advice naming an input this operation does not have,
//! so `rivet-sysd` bound to the boot selector and the machine answered
//! `"records":[],"returned":0`. An operation that has no service filter must
//! say so where a service name lands, and `refuse_unknown_boot` is that
//! sentence. The prose that gave the advice is corrected at its source, in
//! `super::install` and [`super::machine`](machine); the refusal is here because prose
//! can go stale again and a refusal is read at the moment of the mistake.
//!
//! ~~**Follow.** There is no `--follow`; the operation answers from what is on
//! the card at the moment it is asked.~~
//!
//! **Superseded 2026-08-25 — and only half of it.** `rivet get log --follow`
//! exists, and this operation is still exactly what the struck sentence says it
//! is: **it answers once, from what is on the card at the moment it is asked.**
//! Following is a property of the *transport*, not of the operation — an
//! adapter calls this, holds a cursor, and calls again. `cmd/rivet`'s
//! `follow` module is that loop, [`crate::journal::Follow`] is the cursor, and
//! neither is reachable from here. `PLAN.md` §21.1 carries the argument and
//! what it rejected; `ADR-022` §9.5 is the ruling.
//!
//! **A default limit arrived at the same time**, and it changes what a bare
//! `rivet get log` answers. See [`LIMIT_DEFAULT`](log::LIMIT_DEFAULT).

use super::{ErrorCode, Field, JournalRead, Observed, OperationError};
use crate::journal::{BootSelector, Cursor, Filter, Placed, boots_in_order, parse_lines};
use crate::value::Value;

/// The subject this build accepts. `ADR-006` §6.4's nouns are values, not
/// catalogue keys — `ADR-006` §8.5.1.
pub const SUBJECT: &str = "log";

/// How many records a caller who did not ask for a number gets — **200**, the
/// last 200 after `boot` has selected.
///
/// **Until 2026-08-25 there was no default and a bare `rivet get log`
/// answered with the whole journal**, which `ADR-022` §8 caps at eight
/// megabytes. That is not a limit, it is the absence of one: the answer is
/// rendered into a `String` in this process, sent whole down whatever channel
/// asked for it, and — for the caller this surface is designed for (`ADR-012`) —
/// spent as context. `PLAN.md` §20.6 named it; `ADR-022` §9.6 rules it.
///
/// **200 rather than a round 100 or 1000.** A boot of the supervisor writes on
/// the order of thirty records (`rivet_core::state::LOG_FILE_MAX_BYTES` says
/// so and the board agrees), so 200 is several boots' worth — enough that the
/// usual question *"what did this machine just do"* is answered in full, and
/// small enough that the answer is tens of kilobytes rather than eight
/// megabytes.
///
/// **Nothing is hidden to make it fit.** The result reports `limit` and `held`
/// beside `returned`, so an answer that was cut says by how much, and
/// [`LIMIT_ALL`] is the word that turns it off.
///
/// **That was not enough, and 2026-08-31 is what it cost.** `returned < held`
/// is *also* what naming one boot out of thirty looks like, so a reader cannot
/// tell an answer that was cut from one that was selected. A search for `vc4`
/// against this default came back empty — **because a boot's kernel records are
/// at its start and this keeps the newest** — and the conclusion drawn was that
/// the machine had no kernel log at all. It has ingested `/dev/kmsg` since
/// `ADR-022` §4.1. A whole `get kernel-log` subject was built and reverted
/// before that was found (`PLAN.md` §104.10). The `truncated` field is the
/// sentence that says which of the two happened, and it appears only when this
/// limit is the answer.
pub const LIMIT_DEFAULT: usize = 200;

/// The word that means *no limit at all* — `rivet get log latest all`.
///
/// A word rather than `0`, because `0` is a number a caller can arrive at by
/// arithmetic and *"give me nothing"* and *"give me everything"* must not be
/// the same request. It is spelled the same as `BootSelector::All` on purpose:
/// one word, one meaning, in both positions.
pub const LIMIT_ALL: &str = "all";

/// What this subject's result carries, declared beside the implementation so the
/// two cannot drift — `super`'s module documentation, property 1.
///
/// **This is the authority**, and since 2026-08-24 nothing restates it:
/// [`super::get::subject_table`] names this list, and the flat union
/// `rivet --version` publishes is generated from it by
/// [`super::Operation::outputs`]. There is nothing left for the two to disagree
/// about.
pub(super) const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was read. Always `log` in this build.",
    },
    Field {
        name: "boot",
        description: "The selector that was applied, as given.",
    },
    Field {
        name: "limit",
        description: "How many records were kept, counting from the END. ABSENT means no \
                      limit was applied, which happens only when `all` was asked for: a \
                      caller who names no number gets 200 (ADR-022 §9.6), so an answer \
                      whose `returned` equals its `limit` is very likely CUT — compare \
                      `held`, and ask again with `all` or a bigger number.",
    },
    Field {
        name: "boots",
        description: "Every boot identifier the journal still holds, OLDEST FIRST. \
                      Retention drops the oldest file (ADR-022 §8), so this is what the \
                      machine actually has rather than what it has ever had.",
    },
    Field {
        name: "records",
        description: "The selected records, oldest first, each as the JSONL line the \
                      machine wrote (ADR-022 §3). In nushell: \
                      `rivet get log | from json | get records | each { from json }`.",
    },
    Field {
        name: "returned",
        description: "How many records are in `records`.",
    },
    Field {
        name: "held",
        description: "How many records the journal holds in total, before `boot` and \
                      `limit` selected. `returned` < `held` is ordinary; `held` of 0 on a \
                      machine that has been running is not.",
    },
    Field {
        name: "truncated",
        description: "PRESENT ONLY WHEN THE LIMIT CUT RECORDS, and it says how many went, from \
                      WHICH END, and how to get them. `returned` < `held` has two causes a \
                      caller cannot tell apart — selecting one boot out of thirty is working as \
                      asked, and the default limit quietly dropping the older half of the boot \
                      you asked for is not. THE OLDEST GO, because a journal is read from the \
                      end (ADR-022 §9.6), AND A BOOT\'S KERNEL RECORDS ARE AT ITS START: a \
                      search for a driver name against the default limit comes back empty on a \
                      machine that has been up for ten minutes, and reads as `the kernel never \
                      said that`. PLAN.md §104.10 is what that cost.",
    },
    Field {
        name: "unreadable",
        description: "Lines the journal holds that this build could not read. NOT dropped \
                      silently: a log that answers `there is nothing about that` when it \
                      in fact could not parse four lines is the one failure a log must not \
                      have (AGENTS.md §5). A non-zero count usually means a NEWER Rivet \
                      wrote them.",
    },
    Field {
        name: "unreadable-detail",
        description: "Up to five of those lines, by line number and what was wrong. \
                      Absent when `unreadable` is 0.",
    },
    Field {
        name: "files",
        description: "Which journal files were read, oldest first. Empty on a machine \
                      whose journal has never been written — see `source`.",
    },
    Field {
        name: "source",
        description: "Where this machine keeps its journal. Reported even when nothing was \
                      found, so an empty answer names a place rather than being a shrug.",
    },
    Field {
        name: "cursor",
        description: "A position in this journal, to hand back as `--since` next time \
                      (ADR-035 §4A). It names the LAST record in `records`, so the next call \
                      answers with what happened after it. ABSENT when nothing was returned \
                      — there is no new position to report, and the caller should keep using \
                      the one it already has rather than treating absence as a reset.",
    },
    Field {
        name: "since",
        description: "What the `--since` cursor turned out to mean. `at` — it was placed and \
                      records after it were selected. `boot-gone` — THE JOURNAL NO LONGER \
                      HOLDS THAT BOOT, so the cursor could not be placed and the answer holds \
                      everything the other inputs selected rather than nothing. Absent when \
                      no cursor was given.",
    },
];

/// How many refused lines the result quotes.
///
/// A count is not actionable and the whole file is not readable; five is enough
/// to see whether they are all the same kind of wrong, which is the question a
/// reader actually has.
const REFUSALS_QUOTED: usize = 5;

/// Perform `get log`. The subject has already matched — [`super::get::perform`]
/// dispatches, and an unknown one is refused there so that the refusal can name
/// every subject rather than only this one.
/// Read the `since` cursor — the fourth word, and optional.
///
/// # Why a position rather than `--since <cursor>`
///
/// A flag was written first and gave the operation a **fifth word**, which
/// `Operation::word_ceiling` refuses: every input in this catalogue is one word
/// unless it declares itself `several`, and declaring `since` several-word
/// would lift the ceiling for the whole `get` verb to buy one flag. The other
/// four subjects already refuse their own extra words, but `log` did not, so
/// the trade was *one convenience for one real check*.
///
/// The position also happens to be what the MCP adapter needs. `cmd/rivet`'s
/// `mcp` builds the argument vector by walking the declared inputs in order and
/// pushing what it was given, so a caller writing
/// `{"subject":"log","boot":"latest","limit":"all","since":"…"}` produces
/// exactly these four words. A flag would have needed the adapter to know that
/// this one input is spelled differently from every other — a concept the
/// catalogue does not have and should not grow for one case.
///
/// # Errors
///
/// If the word is not a cursor this build wrote. **Refused rather than
/// ignored**: silently dropping it answers a tail request with the whole
/// journal, which is the wrong answer delivered convincingly.
fn parse_since(word: Option<&str>) -> Result<Option<Cursor>, OperationError> {
    let Some(raw) = word else {
        return Ok(None);
    };
    Cursor::parse(raw).map(Some).ok_or_else(|| {
        OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "{raw:?} is not a cursor this build wrote. A cursor is OPAQUE: hand back the \
                 `cursor` field of a previous `get log` answer, verbatim, as the FOURTH word — \
                 `get log <boot> <limit> <cursor>`. Refusing rather than ignoring it, because \
                 ignoring a tail request answers with the whole journal, which is the wrong \
                 answer delivered convincingly"
            ),
        )
    })
}

/// Refuse a fifth word by name.
///
/// **This check is new and replaces one that was free.** Until `since` existed,
/// `Operation::word_ceiling` refused any command with more words than the verb
/// declares inputs, and `log` leaned on it — the other four `get` subjects each
/// refuse their own. The ceiling still applies, so this is not strictly
/// necessary today; it is here because the ceiling's message counts words and
/// this one names the offending one, and because a `log` that quietly ignored a
/// word would be the failure `storage`'s own refusal exists to prevent.
fn refuse_extra_words(arguments: &[&str]) -> Result<(), OperationError> {
    if let Some(extra) = arguments.get(4) {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "{extra:?} is a fifth word and `get log` takes four — the subject, the boot \
                 selector, the limit, and a cursor from a previous answer. It is refused \
                 rather than ignored, because a word that does nothing is a word whose author \
                 believed it did something"
            ),
        ));
    }
    Ok(())
}

/// **Refuse a boot selector that names no boot this journal holds.**
///
/// Added 2026-08-30, and the defect it closes was found by a cold agent rather
/// than argued for. The machine's own install answers said *"read it with
/// `rivet get log rivet-sysd`"* — and `get log` has no service filter, so
/// `rivet-sysd` bound to the SECOND input, which is the boot selector.
/// [`parse_selector`] reads anything it does not recognise as a boot
/// identifier, so the machine answered `"records":[],"returned":0` for a boot
/// that has never existed. **The agent read that as an empty journal**, which
/// is a different and much worse claim than *you asked for a boot I do not
/// have*: the first says the machine did nothing, the second says the question
/// missed.
///
/// `AGENTS.md` §5's *refuse rather than guess* in its exact shape — an answer
/// that does not fail but succeeds, reporting a plausible nothing that
/// something upstream will act on.
///
/// # Why `held == 0` is exempt
///
/// A journal holding no records at all has no boots to compare against, so
/// every selector names a boot it does not have and refusing would turn *this
/// machine has not written a journal yet* into an argument error. That case is
/// already reported honestly: `held` is 0 and `source` names where the journal
/// would be. [`OUTPUTS`]'s own `held` description says a zero there on a
/// running machine is the finding.
///
/// # Why only [`BootSelector::Id`]
///
/// `latest`, `previous` and `all` are keywords whose empty answers are
/// meaningful — `previous` on a machine that has booted once is *there is no
/// previous boot*, and `Filter::wanted_boot`'s three-way
/// answer already keeps that distinct from *every boot*. Only an identifier can
/// be a word that was never a boot in the first place.
fn refuse_unknown_boot(
    boot: &BootSelector,
    boots: &[String],
    held: usize,
) -> Result<(), OperationError> {
    let BootSelector::Id(named) = boot else {
        return Ok(());
    };
    if held == 0 || boots.iter().any(|b| b == named) {
        return Ok(());
    }
    Err(OperationError::about(
        ErrorCode::Arguments,
        named,
        format!(
            "the SECOND word of `get log` is the BOOT SELECTOR, and {named:?} is not a boot \
             this journal holds. The words it takes there are `latest`, `previous`, `all`, or \
             a boot identifier copied out of a record. NOTHING WAS FILTERED OUT — this is \
             refused rather than answered with an empty `records`, because an empty answer to \
             a question the journal cannot ask looks exactly like a machine that did nothing. \
             THERE IS NO SERVICE FILTER ON THIS OPERATION: `get log` takes (subject, boot, \
             limit, cursor) and nothing else, so a service name in this position selects a \
             boot rather than a service. To read one service, take the whole answer and select \
             on each record's `svc` field — every record is the JSONL line the machine wrote \
             (ADR-022 §3). This journal holds {} boot(s): {}",
            boots.len(),
            boots
                .iter()
                .map(|b| format!("`{b}`"))
                .collect::<Vec<_>>()
                .join(", "),
        ),
    ))
}

pub(super) fn perform(
    arguments: &[&str],
    observed: &mut dyn Observed,
) -> Result<Value, OperationError> {
    refuse_extra_words(arguments)?;
    let selector = arguments.get(1).copied().unwrap_or("latest");
    let boot = parse_selector(selector);
    let limit = parse_limit(arguments.get(2).copied())?;
    let since = parse_since(arguments.get(3).copied())?;

    let read: JournalRead = observed
        .read_journal()
        .map_err(|e| OperationError::about(ErrorCode::MachineRefused, SUBJECT, e.to_string()))?;

    let (records, refused) = parse_lines(read.lines.iter().map(String::as_str));
    let held = records.len();
    let boots = boots_in_order(&records);
    refuse_unknown_boot(&boot, &boots, held)?;

    let filter = Filter {
        boot,
        limit,
        since: since.clone(),
        ..Filter::default()
    };
    let placed = filter.placed(&records);
    let (selected, truncated) = filter.apply_counting(records);

    let mut fields = vec![("subject", Value::str(SUBJECT)), ("boot", Value::str(selector))];
    if let Some(n) = limit {
        fields.push(("limit", Value::Integer(i64::try_from(n).unwrap_or(i64::MAX))));
    }
    fields.extend([
        ("boots", Value::strings(boots)),
        (
            "records",
            Value::List(
                selected
                    .iter()
                    .map(|r| Value::str(r.to_jsonl().trim_end()))
                    .collect(),
            ),
        ),
        (
            "returned",
            Value::Integer(i64::try_from(selected.len()).unwrap_or(i64::MAX)),
        ),
        (
            "held",
            Value::Integer(i64::try_from(held).unwrap_or(i64::MAX)),
        ),
    ]);
    // **Present exactly when the limit cut something**, on `unreadable-detail`'s
    // precedent: a field that is always there and usually zero is a field
    // nobody reads. This one appears only when it changes what the answer means.
    if truncated > 0 {
        fields.push(("truncated", Value::str(truncated_because(truncated, held))));
    }
    fields.push((
        "unreadable",
        Value::Integer(i64::try_from(refused.len()).unwrap_or(i64::MAX)),
    ));
    if !refused.is_empty() {
        fields.push((
            "unreadable-detail",
            Value::strings(
                refused
                    .iter()
                    .take(REFUSALS_QUOTED)
                    .map(|(line, why)| format!("line {line}: {why}")),
            ),
        ));
    }
    fields.push(("files", Value::strings(read.files)));
    fields.push(("source", Value::str(read.source)));
    // **The cursor names the LAST record returned**, so the next call answers
    // with what came after it. Absent when nothing was returned: there is no
    // new position to report, and a caller must keep the cursor it already has
    // rather than read the absence as a reset (ADR-035 §4A).
    if let Some(last) = selected.last() {
        fields.push(("cursor", Value::str(Cursor::after(last).to_string())));
    }
    match placed {
        Placed::NotAsked => {}
        Placed::At => fields.push(("since", Value::str("at"))),
        // Said out loud rather than left for the caller to infer from a sudden
        // flood of records it has already seen.
        Placed::BootGone => fields.push(("since", Value::str("boot-gone"))),
    }
    debug_assert!(
        super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)),
        "a log result's keys must be a subsequence of OUTPUTS, in the declared order"
    );
    Ok(Value::object(fields))
}

/// **What the limit cut, which end it cut from, and how to ask for it.**
///
/// Composed rather than constant because the numbers are the argument: *"200 of
/// 4 812"* is a different statement from *"200 of 201"*, and a reader deciding
/// whether their search was complete needs the first one to look alarming.
///
/// `PLAN.md` §104.10 is why the kernel sentence is here. A grep for `vc4`
/// against `get log latest` answered nothing, and the conclusion drawn was that
/// the machine had no kernel log at all — it has ingested `/dev/kmsg` since
/// `ADR-022` §4.1, and the records were four thousand behind the window.
fn truncated_because(truncated: usize, held: usize) -> String {
    format!(
        "the limit kept the newest records and dropped {truncated} OLDER ones, out of {held} the \
         journal holds. A JOURNAL IS READ FROM THE END (ADR-022 §9.6) and A BOOT\'S KERNEL \
         RECORDS ARE AT ITS START, so a search that came back with nothing may have been looking \
         at the wrong end of the boot rather than at a machine that never logged it. \
         `get log <boot> all` returns everything, which ADR-022 §8 caps at eight megabytes."
    )
}

/// Read the `limit` argument, and supply [`LIMIT_DEFAULT`] when there is none.
///
/// **Three cases, and the middle one is the whole point of the change:** a
/// number is that number, [`LIMIT_ALL`] is no limit, and *nothing said* is the
/// default rather than no limit. Before 2026-08-25 the second and third were
/// the same answer, so the only way to ask for the whole journal was to not ask
/// for anything — which is what every caller did.
///
/// It is `pub(crate)` because [`crate::journal::Follow`] takes the same word in
/// the same meaning for its backfill, and two readers of one argument is
/// `ADR-011` in miniature.
pub fn parse_limit(text: Option<&str>) -> Result<Option<usize>, OperationError> {
    match text {
        None => Ok(Some(LIMIT_DEFAULT)),
        Some(LIMIT_ALL) => Ok(None),
        Some(text) => Ok(Some(text.parse::<usize>().map_err(|_| {
            OperationError::about(
                ErrorCode::Arguments,
                text,
                format!(
                    "`limit` is a count of records, so it has to be a whole number that is \
                     not negative — or the word `{LIMIT_ALL}` for every record the journal \
                     holds. A caller who names neither gets the last {LIMIT_DEFAULT}."
                ),
            )
        })?)),
    }
}

/// Read the `boot` argument.
///
/// **Anything unrecognised is a boot identifier**, not an error, and that is
/// deliberate: the identifiers are whatever `/proc/sys/kernel/random/boot_id`
/// produced, this build has no pattern to check them against, and an operator
/// pasting one out of a record must not be told it is a bad keyword.
///
/// ~~A selector that matches no boot returns nothing, and `boots` in the result
/// is the list to compare it against.~~ **Corrected 2026-08-30.** That was this
/// function's own account of what happened next and it was the defect: an
/// identifier naming no boot the journal holds is now refused by
/// `refuse_unknown_boot` rather than answered with an empty `records`, and
/// the refusal names the boots there are. Reading anything as an identifier is
/// still right; treating an identifier that matches nothing as a *selection*
/// was not.
pub fn parse_selector(text: &str) -> BootSelector {
    match text {
        "latest" => BootSelector::Latest,
        "previous" => BootSelector::Previous,
        "all" => BootSelector::All,
        id => BootSelector::Id(id.to_string()),
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::operation::machine::MachineError;

    /// A machine whose journal is whatever the test says it is.
    struct FakeJournal {
        read: Result<JournalRead, MachineError>,
        reads: usize,
    }

    impl FakeJournal {
        fn holding(lines: &[&str]) -> Self {
            Self {
                read: Ok(JournalRead {
                    lines: lines.iter().map(|s| (*s).to_string()).collect(),
                    files: vec!["/state/log/journal.jsonl".to_string()],
                    source: "/state/log".to_string(),
                }),
                reads: 0,
            }
        }

        fn refusing(why: &str) -> Self {
            Self {
                read: Err(MachineError::Refused(why.to_string())),
                reads: 0,
            }
        }
    }

    /// **Five panicking stubs used to stand here** — `running_slot`,
    /// `can_arm_trial_boot`, `arm_trial_boot`, `can_request_stop` and
    /// `request_stop` — saying things like *"get log must not touch the boot
    /// slots"*. They were a type system written by hand, and on 2026-08-24 the
    /// real one took the job: `get log` takes `&mut dyn Observed`, so those
    /// methods are not reachable to be asserted about.
    impl Observed for FakeJournal {
        /// **Refuses rather than answering that this machine trusts nobody.**
        /// An empty trust store is a REAL reading -- it is what `ADR-028` §5.5.5's
        /// `--no-trust` image produces -- and this fake has not taken it. The
        /// direction is the dangerous one: *"trusts nobody"* is the answer
        /// somebody acts on by re-signing an artifact.
        fn read_trust(&mut self) -> Result<crate::operation::machine::TrustRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering an idle machine.** Zeroed counters
        /// are what a machine that has run nothing since it booted looks like,
        /// and a load average of zero is what an idle one looks like — both are
        /// readings, and this fake has taken neither (`AGENTS.md` §3, §5).
        fn read_cpu(&mut self) -> Result<crate::operation::machine::CpuRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering that this machine has no block
        /// devices**, which is not a state a machine that booted off one can
        /// be in.
        fn read_devices(&mut self) -> Result<crate::operation::machine::DeviceRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering an empty network.** No interfaces at
        /// all reads as a machine with none, which is not a state a running
        /// Linux is in.
        fn read_network(&mut self) -> Result<crate::operation::machine::NetworkRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than naming a slot.** Naming the wrong one is how
        /// an update overwrites the running system, so a fake that observes
        /// nothing names none.
        fn read_slots(&mut self) -> Result<crate::operation::machine::SlotRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering an uptime of zero.** `0` would say
        /// this machine booted a moment ago, which is a reading nobody took.
        fn read_boot(&mut self) -> Result<crate::operation::machine::BootRead, MachineError> {
            Err(MachineError::Refused("not this fake's port".to_string()))
        }
        /// **Refuses rather than answering with unset flags.** Reporting a
        /// `Throttle` of all-`false` would assert that this machine has never
        /// been under-volted since it booted — the one claim this subject must
        /// never make without a measurement (`AGENTS.md` §3, §5).
        fn read_thermal(
            &mut self,
        ) -> Result<crate::operation::machine::ThermalRead, MachineError> {
            Err(MachineError::Refused(
                    "not this fake's port; unread throttle flags are never reported as false"
                        .to_string(),
            ))
        }
    fn read_processes(
        &mut self,
    ) -> Result<crate::operation::machine::ProcessRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_memory(&mut self) -> Result<crate::operation::machine::MemoryRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_desired_state(&mut self) -> Result<Option<String>, MachineError> {
        Ok(None)
    }
        /// `ADR-032` §3.2's query socket, added 2026-08-26. This fake asks no
        /// supervisor and refuses rather than pretending (`AGENTS.md` §3).
        fn supervisor_status(
            &mut self,
        ) -> Result<crate::operation::status::Status, MachineError> {
            Err(MachineError::Refused(
                "this fake answers no supervisor; an operation that reached this port has changed"
                    .to_string(),
            ))
        }

        fn read_journal(&mut self) -> Result<JournalRead, MachineError> {
            self.reads += 1;
            self.read.clone()
        }
        /// The fake supplies no schema: `rivet-core` cannot name one
        /// (`ADR-014` §3.2), so a test that needs the real document belongs in
        /// the adapter that has both crates. Loud rather than empty, so an
        /// assertion against real schema content fails instead of passing.
        fn configuration_schema(&mut self) -> Value {
            Value::str("<no schema: this is a test fake>")
        }

        fn read_filesystems(
            &mut self,
        ) -> Result<crate::operation::machine::StorageRead, MachineError> {
            panic!("get log must not measure a filesystem")
        }
    }

    fn line(boot: &str, mono: f64, sev: &str, svc: &str, msg: &str) -> String {
        format!(
            r#"{{"boot":"{boot}","mono":{mono:.3},"sev":"{sev}","svc":"{svc}","corr":"c","actor":"supervisor","msg":"{msg}"}}"#
        )
    }

    fn two_boots() -> Vec<String> {
        vec![
            line("boot-a", 1.0, "info", "rivet-sysd", "old one"),
            line("boot-a", 2.0, "error", "rivet-netd", "old two"),
            line("boot-b", 1.0, "info", "rivet-sysd", "new one"),
            line("boot-b", 2.0, "warn", "rivet-netd", "new two"),
        ]
    }

    fn run(lines: &[String], args: &[&str]) -> Value {
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let mut m = FakeJournal::holding(&refs);
        perform(args, &mut m).unwrap_or_else(|e| panic!("get {args:?} failed: {e}"))
    }

    fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
        match v {
            Value::Object(fields) => fields
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v)
                .unwrap_or_else(|| panic!("no {key} in {v:?}")),
            other => panic!("not an object: {other:?}"),
        }
    }

    fn maybe<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
        match v {
            Value::Object(fields) => fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v),
            other => panic!("not an object: {other:?}"),
        }
    }

    fn messages(v: &Value) -> Vec<String> {
        match field(v, "records") {
            Value::List(items) => items
                .iter()
                .map(|item| match item {
                    Value::Str(line) => {
                        crate::journal::Record::from_jsonl(line)
                            .unwrap_or_else(|e| panic!("a returned record does not parse: {e}"))
                            .msg
                    }
                    other => panic!("a record is not a string: {other:?}"),
                })
                .collect(),
            other => panic!("records is not a list: {other:?}"),
        }
    }

    #[test]
    fn the_default_is_this_boot() {
        let got = run(&two_boots(), &["log"]);
        assert_eq!(messages(&got), ["new one", "new two"]);
        assert_eq!(field(&got, "boot"), &Value::str("latest"));
        assert_eq!(field(&got, "returned"), &Value::Integer(2));
        assert_eq!(field(&got, "held"), &Value::Integer(4));
    }

    /// The one an agent asks after a board came back, and the one a shell
    /// cannot answer afterwards — the records do not carry which boot was
    /// first, only the journal's append order does (`ADR-021`, `ADR-022` §3.2).
    #[test]
    fn previous_selects_the_boot_before_the_last_one() {
        let got = run(&two_boots(), &["log", "previous"]);
        assert_eq!(messages(&got), ["old one", "old two"]);
    }

    #[test]
    fn all_returns_every_boot_oldest_first() {
        let got = run(&two_boots(), &["log", "all"]);
        assert_eq!(messages(&got), ["old one", "old two", "new one", "new two"]);
        assert_eq!(
            field(&got, "boots"),
            &Value::strings(["boot-a", "boot-b"]),
            "boots must be oldest first"
        );
    }

    #[test]
    fn a_boot_identifier_selects_that_boot() {
        let got = run(&two_boots(), &["log", "boot-a"]);
        assert_eq!(messages(&got), ["old one", "old two"]);
    }

    /// ~~An identifier this journal no longer holds is an empty answer, not an
    /// error — `ADR-022` §8 drops the oldest file, so asking for a boot that has
    /// aged out is an ordinary thing to do. `boots` is what tells the operator
    /// which ones are left.~~
    ///
    /// **Overturned 2026-08-30, by a cold agent rather than by an argument.**
    /// The reasoning above is about a boot that *was* a boot; it says nothing
    /// about a word that never was one, and from inside the journal the two are
    /// the same value. The machine's own install answers told an operator to
    /// run `rivet get log rivet-sysd`; `get log` has no service filter, so
    /// `rivet-sysd` bound here, and the answer was `"records":[],"returned":0`
    /// — which the agent read as an empty journal.
    ///
    /// The precedent that decides it is three fields down in [`OUTPUTS`]:
    /// `since` reports `boot-gone` rather than quietly answering everything,
    /// *"said out loud rather than left for the caller to infer"*. A cursor's
    /// missing boot is announced and a selector's was not.
    ///
    /// **Nothing is lost in the aged-out case**, which is why this is a
    /// straight replacement: the refusal names the boots that remain, which is
    /// the whole of what the old `boots` field gave a reader who noticed it.
    #[test]
    fn a_boot_the_journal_does_not_hold_is_refused_and_the_refusal_names_the_ones_it_has() {
        let lines = two_boots();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let mut m = FakeJournal::holding(&refs);
        let refused = perform(&["log", "boot-z"], &mut m)
            .expect_err("a boot identifier this journal does not hold must be refused");
        let said = refused.to_string();
        assert!(said.contains("boot-z"), "{said}");
        assert!(said.contains("BOOT SELECTOR"), "{said}");
        assert!(said.contains("NO SERVICE FILTER"), "{said}");
        assert!(said.contains("NOTHING WAS FILTERED OUT"), "{said}");
        assert!(said.contains("`boot-a`"), "{said}");
        assert!(said.contains("`boot-b`"), "{said}");
    }

    /// **The exact command the machine used to tell operators to run**, and the
    /// regression this refusal exists for. `rivet-sysd` is a service name and
    /// this position is the boot selector; the failure it replaces was an
    /// answer, not an error.
    #[test]
    fn a_service_name_in_the_boot_position_is_refused_by_name() {
        let lines = two_boots();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let mut m = FakeJournal::holding(&refs);
        let refused = perform(&["log", "rivet-sysd"], &mut m)
            .expect_err("a service name in the boot position must be refused");
        let said = refused.to_string();
        assert!(said.contains("rivet-sysd"), "{said}");
        assert!(said.contains("svc"), "the refusal must say how to filter by service: {said}");
    }

    /// **What every corrected hint now says instead, proven end to end.**
    /// `rivet-core/src/operation/uninstall.rs` and `input.rs` used to tell an
    /// operator to run `rivet get log rivet-sysd` / `rivet get log
    /// rivet-launcher` — the exact command the test above shows this parser
    /// refuses. The correction is `rivet get log`, bare, with the service
    /// found by reading each record's `svc` field out of the answer. This
    /// drives that command vector through the real parser and confirms both:
    /// the bare command is accepted (unlike the one above), and the service
    /// each hint names is actually present under `svc` in what comes back —
    /// so the hint's claim is checked against the parser, not just against
    /// the wording of the hint itself.
    #[test]
    fn the_command_every_corrected_hint_now_suggests_finds_the_named_service_by_its_svc_field() {
        let lines = vec![
            line("boot-a", 1.0, "info", "rivet-sysd", "stopped rivet-photo-index"),
            line("boot-a", 2.0, "info", "rivet-launcher", "pressed ok"),
        ];
        let got = run(&lines, &["log"]);

        let records = match field(&got, "records") {
            Value::List(items) => items
                .iter()
                .map(|item| match item {
                    Value::Str(text) => crate::journal::Record::from_jsonl(text)
                        .unwrap_or_else(|e| panic!("a returned record does not parse: {e}")),
                    other => panic!("a record is not a string: {other:?}"),
                })
                .collect::<Vec<_>>(),
            other => panic!("records is not a list: {other:?}"),
        };

        let for_service = |svc: &str| {
            records
                .iter()
                .find(|r| r.svc == svc)
                .unwrap_or_else(|| panic!("no record with svc {svc:?} in {records:?}"))
        };
        assert_eq!(for_service("rivet-sysd").msg, "stopped rivet-photo-index");
        assert_eq!(for_service("rivet-launcher").msg, "pressed ok");
    }

    /// **A journal with nothing in it is exempt**, because it has no boots to
    /// compare a selector against and *"this machine has not written a journal
    /// yet"* is not an argument error. `held` of 0 is the honest report and
    /// [`OUTPUTS`]'s own description says a zero there on a running machine is
    /// the finding.
    #[test]
    fn an_empty_journal_answers_rather_than_refusing_a_selector_it_cannot_check() {
        let got = run(&[], &["log", "boot-z"]);
        assert_eq!(field(&got, "held"), &Value::Integer(0));
        assert_eq!(field(&got, "returned"), &Value::Integer(0));
    }

    #[test]
    fn a_limit_keeps_the_last_n_records() {
        let got = run(&two_boots(), &["log", "all", "3"]);
        assert_eq!(messages(&got), ["old two", "new one", "new two"]);
        assert_eq!(field(&got, "returned"), &Value::Integer(3));
        assert_eq!(field(&got, "held"), &Value::Integer(4));
    }

    /// **The failure this field was built after** — `PLAN.md` §104.10.
    ///
    /// A search for `vc4` against `get log latest` answered nothing, because the
    /// 200-record default keeps the NEWEST and a boot's kernel records are at
    /// its START. The conclusion drawn was that the machine had no kernel log
    /// at all; it has ingested `/dev/kmsg` since `ADR-022` §4.1, and a whole
    /// `get kernel-log` subject was built and reverted before that was found.
    ///
    /// `returned` and `held` were both already in that answer and both correct.
    /// **They were not enough**, because `returned < held` is also what
    /// selecting one boot out of thirty looks like, and a reader cannot tell
    /// the two apart. This says which happened.
    #[test]
    fn a_limit_that_cut_records_says_so_and_says_which_end_went() {
        let got = run(&two_boots(), &["log", "all", "3"]);
        let said = match field(&got, "truncated") {
            Value::Str(s) => s.clone(),
            other => panic!("truncated is not text: {other:?}"),
        };
        assert!(said.contains('1') && said.contains('4'), "the numbers are the argument: {said}");
        assert!(said.contains("OLDER"), "which end went: {said}");
        assert!(
            said.contains("KERNEL RECORDS ARE AT ITS START"),
            "the reason an empty search misleads: {said}"
        );
        assert!(said.contains("all"), "how to get the rest: {said}");
    }

    /// **Absent when the limit cut nothing**, on `unreadable-detail`'s
    /// precedent: a field that is always there and usually says nothing is a
    /// field nobody reads, and this one has to be noticed the once it appears.
    #[test]
    fn an_answer_the_limit_did_not_cut_carries_no_truncation_notice() {
        let got = run(&two_boots(), &["log", "all", "all"]);
        assert_eq!(maybe(&got, "truncated"), None);
        let got = run(&two_boots(), &["log", "all", "99"]);
        assert_eq!(
            maybe(&got, "truncated"),
            None,
            "a limit larger than the journal cut nothing and must not claim to"
        );
    }

    /// **A boot selector is not a truncation.** Records another boot's
    /// selection excluded were never candidates, and reporting them here would
    /// send a reader after records that are not missing — the same
    /// over-claiming, in the opposite direction.
    #[test]
    fn selecting_one_boot_is_not_reported_as_the_limit_having_cut_anything() {
        let got = run(&two_boots(), &["log", "boot-b", "all"]);
        assert_eq!(field(&got, "returned"), &Value::Integer(2));
        assert_eq!(field(&got, "held"), &Value::Integer(4));
        assert_eq!(
            maybe(&got, "truncated"),
            None,
            "two of four were returned because a boot was NAMED, not because a limit cut"
        );
    }

    /// **The property no test held until 2026-08-25.** A bare `get log`
    /// answered with everything, and nothing anywhere asserted that it should —
    /// which is `PLAN.md` §20.1's lesson arriving a second time in the same
    /// week. These three tests are the limit, not the constant.
    #[test]
    fn naming_no_limit_keeps_the_last_two_hundred_rather_than_everything() {
        let many: Vec<String> = (0..LIMIT_DEFAULT + 50)
            .map(|n| {
                #[allow(clippy::cast_precision_loss)]
                let mono = n as f64;
                line("boot-a", mono, "info", "rivet-sysd", &format!("r{n}"))
            })
            .collect();
        let got = run(&many, &["log"]);
        assert_eq!(
            field(&got, "returned"),
            &Value::Integer(i64::try_from(LIMIT_DEFAULT).unwrap())
        );
        assert_eq!(
            field(&got, "held"),
            &Value::Integer(i64::try_from(LIMIT_DEFAULT + 50).unwrap())
        );
        assert_eq!(
            messages(&got).first().map(String::as_str),
            Some("r50"),
            "a journal is read from the END, so the 200 kept are the last 200"
        );
    }

    /// The answer has to *say* it was cut, or a reader takes 200 records for
    /// the whole journal — `AGENTS.md` §5.
    #[test]
    fn the_answer_reports_the_limit_that_was_applied_even_when_nobody_asked_for_one() {
        let got = run(&two_boots(), &["log"]);
        assert_eq!(
            field(&got, "limit"),
            &Value::Integer(i64::try_from(LIMIT_DEFAULT).unwrap())
        );
    }

    /// And `all` is the way back to what a bare `get log` used to do. The field
    /// is **absent** rather than zero: `crate::value` has no null, and
    /// `Integer(0)` is the answer to a different question.
    #[test]
    fn the_word_all_turns_the_limit_off_and_the_field_disappears_with_it() {
        let many: Vec<String> = (0..LIMIT_DEFAULT + 50)
            .map(|n| {
                #[allow(clippy::cast_precision_loss)]
                let mono = n as f64;
                line("boot-a", mono, "info", "rivet-sysd", &format!("r{n}"))
            })
            .collect();
        let got = run(&many, &["log", "latest", LIMIT_ALL]);
        assert_eq!(
            field(&got, "returned"),
            &Value::Integer(i64::try_from(LIMIT_DEFAULT + 50).unwrap())
        );
        assert_eq!(maybe(&got, "limit"), None);
    }

    #[test]
    fn a_limit_that_is_not_a_number_is_refused_by_name() {
        let lines = two_boots();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let mut m = FakeJournal::holding(&refs);
        let got = perform(&["log", "all", "soon"], &mut m);
        assert!(
            matches!(got, Err(ref e) if e.code() == ErrorCode::Arguments),
            "{got:?}"
        );
    }

    /// The records come back as the machine's own lines, so what an adapter
    /// hands on is byte-for-byte what was written (`ADR-022` §3).
    #[test]
    fn a_returned_record_is_the_line_the_machine_wrote() {
        let lines = two_boots();
        let got = run(&lines, &["log", "all"]);
        match field(&got, "records") {
            Value::List(items) => {
                assert_eq!(items.first(), Some(&Value::str(lines[0].as_str())));
            }
            other => panic!("{other:?}"),
        }
    }

    /// **The honesty field.** A journal that quietly dropped what it could not
    /// read would answer "there is nothing about that", which is the one answer
    /// a log must never give falsely (`AGENTS.md` §5).
    #[test]
    fn lines_that_will_not_parse_are_counted_and_quoted_rather_than_dropped() {
        let mut lines = two_boots();
        lines.insert(0, "rivet-initd: starting as pid 1".to_string());
        lines.push(r#"{"boot":"boot-b","mono":9.0,"sev":"catastrophe","svc":"s","corr":"c","actor":"a","msg":"m"}"#.to_string());

        let got = run(&lines, &["log", "all"]);
        assert_eq!(field(&got, "unreadable"), &Value::Integer(2));
        assert_eq!(field(&got, "returned"), &Value::Integer(4));
        match maybe(&got, "unreadable-detail") {
            Some(Value::List(items)) => {
                assert_eq!(items.len(), 2);
                let text = format!("{items:?}");
                assert!(text.contains("line 1"), "{text}");
                // And it says WHY, including the one that matters: a severity
                // this build does not know means a newer Rivet wrote it.
                assert!(text.contains("catastrophe"), "{text}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_journal_with_nothing_wrong_omits_the_detail_field_rather_than_emitting_an_empty_list() {
        let got = run(&two_boots(), &["log", "all"]);
        assert_eq!(field(&got, "unreadable"), &Value::Integer(0));
        assert_eq!(maybe(&got, "unreadable-detail"), None);
    }

    /// A machine that has never written a record is not an error, and the
    /// answer names where it looked — so an operator can tell "logged nothing"
    /// from "looked in the wrong place".
    #[test]
    fn an_empty_journal_is_an_answer_and_it_says_where_it_looked() {
        let mut m = FakeJournal::holding(&[]);
        let got = perform(&["log"], &mut m).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(field(&got, "returned"), &Value::Integer(0));
        assert_eq!(field(&got, "held"), &Value::Integer(0));
        assert_eq!(field(&got, "source"), &Value::str("/state/log"));
    }

    /// A machine that cannot be read is an `E_MACHINE_REFUSED` carrying what it
    /// said — never an empty log, which would report a machine that had done
    /// nothing.
    #[test]
    fn a_machine_that_refuses_is_an_error_rather_than_an_empty_log() {
        let mut m = FakeJournal::refusing("this build has no /state");
        let got = perform(&["log"], &mut m);
        match got {
            Err(e) => {
                assert_eq!(e.code(), ErrorCode::MachineRefused);
                assert!(e.to_string().contains("no /state"), "{e}");
            }
            Ok(v) => panic!("a refusal was reported as a log: {v:?}"),
        }
    }

    /// The journal is read **once** per invocation. Not an efficiency point: a
    /// second read could return a different set of records, and `held` would
    /// then describe a journal the returned records did not come from.
    #[test]
    fn the_journal_is_read_once() {
        let mut m = FakeJournal::holding(&[]);
        let _ = perform(&["log", "all"], &mut m);
        assert_eq!(m.reads, 1);
    }

    #[test]
    fn every_selector_word_is_understood_and_anything_else_is_an_identifier() {
        assert_eq!(parse_selector("latest"), BootSelector::Latest);
        assert_eq!(parse_selector("previous"), BootSelector::Previous);
        assert_eq!(parse_selector("all"), BootSelector::All);
        assert_eq!(
            parse_selector("e569923b-b507-4c4a-a64d-3e88da40e32b"),
            BootSelector::Id("e569923b-b507-4c4a-a64d-3e88da40e32b".to_string())
        );
    }

    /// The catalogue is what an agent reads instead of documentation
    /// (`ADR-006` §7), so the pipeline that makes `records` usable has to be in
    /// it rather than in this module's comments.
    #[test]
    fn the_catalogue_tells_a_reader_how_to_use_the_records_field() {
        let records = OUTPUTS
            .iter()
            .find(|f| f.name == "records")
            .expect("records is a declared output");
        assert!(records.description.contains("from json"), "{records:?}");
    }

    // ----------------------------------------------------------------------
    // `--since`, ADR-035 §4A(b)
    // ----------------------------------------------------------------------

    fn cursor_of(v: &Value) -> String {
        match field(v, "cursor") {
            Value::Str(s) => s.clone(),
            other => panic!("cursor is not a string: {other:?}"),
        }
    }

    /// **The whole point, as one test.** Ask, take the cursor, ask again, and
    /// the second answer holds only what happened after the first — which is
    /// what makes repeated calls a tail without any protocol decision.
    #[test]
    fn a_cursor_from_one_answer_selects_only_what_came_after_it() {
        let first = run(&two_boots(), &["log", "all"]);
        assert_eq!(messages(&first), ["old one", "old two", "new one", "new two"]);
        let cursor = cursor_of(&first);

        // Nothing has happened since, so the tail is empty and says so.
        let idle = run(&two_boots(), &["log", "all", "all", &cursor]);
        assert_eq!(messages(&idle), Vec::<String>::new());
        assert_eq!(field(&idle, "returned"), &Value::Integer(0));
        assert_eq!(field(&idle, "since"), &Value::str("at"));
        assert_eq!(
            maybe(&idle, "cursor"),
            None,
            "an empty answer must report NO new position, or a caller polling an idle \
             machine would keep replacing a good cursor with nothing"
        );

        // Then something happens.
        let mut later = two_boots();
        later.push(line("boot-b", 3.0, "info", "rivet-sysd", "brand new"));
        let tail = run(&later, &["log", "all", "all", &cursor]);
        assert_eq!(messages(&tail), ["brand new"]);
        assert_eq!(field(&tail, "since"), &Value::str("at"));
    }

    /// **A later boot is after, whatever its `mono` says.** Every boot's
    /// monotonic clock restarts at zero, so a cursor at `boot-a@2.0` must not
    /// hide `boot-b@1.0` — which it would if the comparison were on `mono`
    /// alone. This is the bug the two-part cursor exists to prevent.
    #[test]
    fn a_record_in_a_later_boot_is_after_a_cursor_with_a_bigger_mono() {
        let got = run(&two_boots(), &["log", "all", "all", "boot-a@2"]);
        assert_eq!(
            messages(&got),
            ["new one", "new two"],
            "boot-b's records have a SMALLER mono than the cursor and are still later"
        );
    }

    /// **Retention dropped the boot the cursor names.** The choice is between
    /// showing nothing — silently skipping everything since — and showing
    /// everything, repeating records the caller has seen. It shows everything
    /// and says `boot-gone`, because repetition is visible to the caller and a
    /// gap is not (`AGENTS.md` §5).
    #[test]
    fn a_cursor_whose_boot_is_gone_returns_everything_and_says_so() {
        let got = run(&two_boots(), &["log", "all", "all", "boot-from-last-week@9"]);
        assert_eq!(messages(&got), ["old one", "old two", "new one", "new two"]);
        assert_eq!(field(&got, "since"), &Value::str("boot-gone"));
    }

    /// Absent unless asked for — an answer to a question nobody put is noise.
    #[test]
    fn no_cursor_asked_for_means_no_since_field() {
        let got = run(&two_boots(), &["log", "all"]);
        assert_eq!(maybe(&got, "since"), None);
        assert!(maybe(&got, "cursor").is_some(), "every non-empty answer offers a position");
    }

    /// **A malformed cursor is refused, not ignored.** Ignoring it would answer
    /// a tail request with the whole journal: the wrong answer, delivered
    /// convincingly.
    #[test]
    fn a_cursor_this_build_did_not_write_is_refused_by_name() {
        let lines = two_boots();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let mut m = FakeJournal::holding(&refs);
        let err = perform(&["log", "all", "all", "yesterday"], &mut m)
            .expect_err("a made-up cursor must be refused");
        let said = err.to_string();
        assert!(said.contains("OPAQUE"), "{said}");
        assert!(said.contains("verbatim"), "{said}");
        assert!(said.contains("FOURTH word"), "{said}");
    }

    /// **A fifth word is refused by name**, which the arity ceiling would also
    /// do — but by counting rather than by naming, and `log` is the one `get`
    /// subject that never had a refusal of its own.
    #[test]
    fn a_fifth_word_is_refused_and_the_refusal_names_it() {
        let lines = two_boots();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let mut m = FakeJournal::holding(&refs);
        let err = perform(&["log", "all", "all", "boot-a@1", "extra"], &mut m)
            .expect_err("a fifth word must be refused");
        let said = err.to_string();
        assert!(said.contains("\"extra\""), "the refusal does not name the word: {said}");
        assert!(said.contains("believed it did something"), "{said}");
    }

    /// `since` composes with `limit`, in the order a caller would expect:
    /// select what is new, then keep the last N of it.
    #[test]
    fn since_and_limit_compose() {
        let mut lines = two_boots();
        for n in 3..7 {
            lines.push(line("boot-b", f64::from(n), "info", "rivet-sysd", &format!("msg {n}")));
        }
        let got = run(&lines, &["log", "all", "2", "boot-b@2"]);
        assert_eq!(messages(&got), ["msg 5", "msg 6"]);
    }

    /// **The reboot case `since` exists for.** A client polling `latest`
    /// saves the cursor it is handed, the machine reboots, and `latest` now
    /// names a different boot than the one the cursor was placed in. The
    /// cursor's own boot has no records left once `latest` has selected only
    /// the new one, so a tail that places the cursor against what a selector
    /// already narrowed down cannot find it — and must not go silent instead
    /// of reporting what is actually new.
    #[test]
    fn latest_after_a_reboot_still_finds_the_new_boots_tail() {
        let boot_a = vec![
            line("boot-a", 1.0, "info", "rivet-sysd", "old one"),
            line("boot-a", 2.0, "error", "rivet-netd", "old two"),
        ];
        let first = run(&boot_a, &["log", "latest"]);
        assert_eq!(messages(&first), ["old one", "old two"]);
        let cursor = cursor_of(&first);

        let mut after_reboot = boot_a;
        after_reboot.push(line("boot-b", 1.0, "info", "rivet-sysd", "new one"));
        after_reboot.push(line("boot-b", 2.0, "warn", "rivet-netd", "new two"));

        let tail = run(&after_reboot, &["log", "latest", "all", &cursor]);
        assert_eq!(
            messages(&tail),
            ["new one", "new two"],
            "a cursor saved from boot A must still find boot B's records once `latest` means \
             boot B"
        );
        assert_eq!(field(&tail, "since"), &Value::str("at"));
        let new_cursor = cursor_of(&tail);
        assert!(new_cursor.starts_with("boot-b@"), "{new_cursor}");
    }

    /// The same reboot case, asked for by the new boot's own id rather than by
    /// `latest` — both resolve to one specific boot inside the filter, and
    /// both must place the cursor the same way.
    #[test]
    fn a_named_later_boot_after_reboot_still_finds_the_tail_past_the_cursor() {
        let boot_a = vec![line("boot-a", 1.0, "info", "rivet-sysd", "old one")];
        let first = run(&boot_a, &["log", "boot-a"]);
        let cursor = cursor_of(&first);

        let mut after_reboot = boot_a;
        after_reboot.push(line("boot-b", 1.0, "info", "rivet-sysd", "new one"));
        after_reboot.push(line("boot-b", 2.0, "warn", "rivet-netd", "new two"));

        let tail = run(&after_reboot, &["log", "boot-b", "all", &cursor]);
        assert_eq!(messages(&tail), ["new one", "new two"]);
        assert_eq!(field(&tail, "since"), &Value::str("at"));
    }
}
