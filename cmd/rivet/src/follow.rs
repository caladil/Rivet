//! `rivet get log --follow` — **the one thing that can ask a running Rivet
//! anything without waiting to be asked again.**
//!
//! `PLAN.md` §20.6 states the gap this closes:
//!
//! > Every question an operator or an agent asks this machine costs a
//! > connection and a process spawn, and there is no way for the machine to say
//! > anything unprompted.
//!
//! and names this as the cheap first step, because it needs no new protocol:
//! the journal is already JSONL with a stable schema (`ADR-022` §3) on a file
//! the supervisor appends to, so streaming it turns polling into subscription
//! with no bus, no socket and no dependency.
//!
//! # Following is a property of the transport, and the operation stayed
//! # one-shot
//!
//! **This is the decision this module exists to record**, and `PLAN.md` §21.1
//! carries what it rejected. `rivet_core::operation::log` answers once, from
//! what is on the card at the moment it is asked. It could not do otherwise
//! without `rivet-core` growing a notion of *time passing*, which is exactly
//! the platform fact `AGENTS.md` §3 keeps out of it — a core that can wait is a
//! core with a clock in it, and every operation would then have a duration.
//!
//! So the loop is here, in an adapter, and the two things it must not decide
//! for itself are elsewhere:
//!
//! | | Where | Why there |
//! | :-- | :-- | :-- |
//! | Which records are new, and whether the machine restarted | `rivet_core::journal::Follow` | boot identifiers are the domain's (`ADR-022` §3.2) |
//! | Which bytes have not been read yet | [`crate::journal::ActiveFile`] | file offsets are this crate's, and the core may not name one |
//! | How long to run, how many records, how often to look | `rivet_core::journal`'s constants | every adapter that grows a follow must bound it the same way |
//!
//! # What lands on which stream, and it is not the usual split
//!
//! **stdout is nothing but journal lines** — one record per line, byte for byte
//! as the machine wrote it (`ADR-022` §3). Not an object with a `records` field
//! in it: a stream has no end, so it cannot be a value, and an agent reading it
//! wants `rivet get log --follow | each { from json }` to work on the first
//! line rather than at the end of the last one.
//!
//! **Everything that is not a record goes to stderr** — the closing summary,
//! and a restart or a rotation at the moment it happens. That is the same rule
//! `crate::journal::send_or_say` already follows for the same reason: stdout is
//! the caller's structured answer and a sentence written into it corrupts what
//! they are parsing. It is **not** a refusal and the exit status is 0.
//!
//! # It is bounded, and that is the point of it rather than a detail
//!
//! `PLAN.md` §20.6's second half is *"nothing bounds the output or the runtime
//! of a command issued remotely"*. A follow is the first command in this binary
//! that would not return on its own, so it is the first that has to:
//!
//! * **A deadline** — `FOLLOW_SECONDS_DEFAULT`, and `--for forever` to say
//!   otherwise deliberately.
//! * **A record ceiling** — `FOLLOW_RECORDS_MAX`, because a service in a
//!   restart loop writes faster than a deadline can catch (`ADR-017` §3.5).
//! * **The reader hanging up** — a channel that closes ends the stream at
//!   the next line rather than at the deadline.
//!
//! Every one of them ends the stream by **saying so**. A follow that stopped
//! because it hit a ceiling and a machine that went quiet are the same silence,
//! and telling them apart is the whole of `AGENTS.md` §5 here.

use rivet_core::journal::{
    FOLLOW_FOREVER, FOLLOW_POLL_MS, FOLLOW_RECORDS_MAX, FOLLOW_SECONDS_DEFAULT,
    Follow, FollowNote, parse_lines,
};
use rivet_core::operation::MachineError;

use crate::journal::Appending;
use crate::refusal::{Code, Refusal};
use crate::value::Value;

/// The flag itself.
pub const FLAG: &str = "--follow";

/// How long to run for.
pub const FOR: &str = "--for";

/// The terminal a follower writes to, and the clock it runs against.
///
/// **An adapter's own I/O seam** (`AGENTS.md` §3.1): a test drives a fake with a
/// clock that jumps, so every bound is provable without waiting for it.
pub trait Console {
    /// One journal line to stdout. **`false` means the far end is gone** — an
    /// channel that closed, a `head -5` that has what it wanted — and the
    /// stream ends at the next check rather than at the deadline.
    fn line(&mut self, text: &str) -> bool;

    /// Something that is not a record, to stderr.
    fn note(&mut self, text: &str);

    /// Milliseconds since the follow started. Monotonic — `ADR-021` is why
    /// this is not a wall clock, and it is the same argument `mono` in a record
    /// is there for.
    fn elapsed_ms(&mut self) -> u64;

    /// Wait. The only place a follow spends time.
    fn sleep_ms(&mut self, ms: u64);
}

/// What the caller asked for, once the words have been read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Which boot, as the caller spelled it — carried so the summary can say it
    /// back in the caller's own words.
    pub boot: String,
    /// The backfill's limit. `None` is the word `all`.
    pub limit: Option<usize>,
    /// The deadline in seconds. `None` is [`FOLLOW_FOREVER`].
    pub seconds: Option<u64>,
}

/// Read `--follow` and `--for` out of the words after the verb.
///
/// Returns the options, if any, and the words with them removed — which are
/// then `get log`'s ordinary positional arguments and are parsed by the core,
/// not here. **This function does not know what `latest` means**, and that is
/// deliberate: `rivet_core::operation::log` is the one reader of a boot
/// selector (`ADR-011`).
///
/// # Errors
///
/// A `--follow` on anything but `get log`, a `--for` without a `--follow`, and
/// a duration that is not a number are all refused by name.
pub fn extract(verb_is_get: bool, rest: &[String]) -> Result<(Option<Options>, Vec<String>), Refusal> {
    let mut following = false;
    let mut duration: Option<String> = None;
    let mut kept: Vec<String> = Vec::new();
    let mut words = rest.iter();

    while let Some(word) = words.next() {
        match word.as_str() {
            FLAG => following = true,
            FOR => {
                let Some(value) = words.next() else {
                    return Err(Refusal::about(
                        Code::ARGUMENTS,
                        FOR,
                        format!(
                            "`{FOR}` needs a value: a number of seconds, or `{FOLLOW_FOREVER}`."
                        ),
                    ));
                };
                duration = Some(value.clone());
            }
            other if other.starts_with(&format!("{FOR}=")) => {
                duration = Some(other.trim_start_matches(&format!("{FOR}=")).to_string());
            }
            other => kept.push(other.to_string()),
        }
    }

    if !following {
        if duration.is_some() {
            return Err(Refusal::about(
                Code::ARGUMENTS,
                FOR,
                format!("`{FOR}` bounds a `{FLAG}`, and there is no `{FLAG}` here."),
            ));
        }
        return Ok((None, kept));
    }

    // The subject is the first positional word, and `log` is the only one with
    // anything to follow: `get storage` and `get schema` are answers, not
    // streams.
    let subject = kept.first().map(String::as_str).unwrap_or_default();
    if !verb_is_get || subject != rivet_core::operation::log::SUBJECT {
        return Err(Refusal::about(
            Code::ARGUMENTS,
            FLAG,
            format!(
                "`{FLAG}` follows `rivet get log` and nothing else. The other subjects are \
                 readings of what a machine IS — storage, a schema, an install — and a \
                 reading does not have more of itself later. The journal does, because it \
                 only ever appends (ADR-022 §3)."
            ),
        ));
    }

    let seconds = match duration.as_deref() {
        None => Some(FOLLOW_SECONDS_DEFAULT),
        Some(FOLLOW_FOREVER) => None,
        Some(text) => Some(text.parse::<u64>().map_err(|_| {
            Refusal::about(
                Code::ARGUMENTS,
                text.to_string(),
                format!(
                    "`{FOR}` is a whole number of seconds, or the word `{FOLLOW_FOREVER}`. A \
                     caller who names neither gets {FOLLOW_SECONDS_DEFAULT} seconds, because a \
                     remote command that never returns holds the connection open until \
                     something else kills it."
                ),
            )
        })?),
    };

    let boot = kept.get(1).cloned().unwrap_or_else(|| "latest".to_string());
    let limit = rivet_core::operation::log::parse_limit(kept.get(2).map(String::as_str))
        .map_err(Refusal::from)?;

    Ok((
        Some(Options {
            boot,
            limit,
            seconds,
        }),
        kept,
    ))
}

/// Why a stream ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stopped {
    /// `--for` ran out. The ordinary ending.
    Deadline,
    /// [`FOLLOW_RECORDS_MAX`] records were emitted.
    Ceiling,
    /// The far end closed — a channel, a pipe into `head`.
    ReaderGone,
}

impl Stopped {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Deadline => "deadline",
            Self::Ceiling => "record-ceiling",
            Self::ReaderGone => "reader-closed",
        }
    }

    fn why(self) -> String {
        match self {
            Self::Deadline => format!(
                "the `{FOR}` deadline passed. This is the ordinary ending; `{FOR} <seconds>` \
                 or `{FOR} {FOLLOW_FOREVER}` runs longer."
            ),
            Self::Ceiling => format!(
                "{FOLLOW_RECORDS_MAX} records were emitted, which is this stream's ceiling. \
                 A machine writing that fast is itself the finding — ADR-017 §3.5's restart \
                 loop is what does it — and `rivet get log` will show the rest."
            ),
            Self::ReaderGone => "the reader closed the stream.".to_string(),
        }
    }
}

/// Run the stream to its end, and return the summary as a value.
///
/// # Errors
///
/// Only a journal that exists and will not be read — the same refusal
/// `rivet get log` gives for the same fault. A journal that is simply absent
/// is not one: it is a true statement about a machine that has not written
/// anything, and the summary says where it looked.
pub fn stream(
    options: &Options,
    console: &mut dyn Console,
    journal: &mut dyn Appending,
) -> Result<Value, Refusal> {
    let selector = rivet_core::operation::log::parse_selector(&options.boot);
    let mut follow = Follow::start(selector).map_err(|why| {
        Refusal::about(Code::ARGUMENTS, options.boot.clone(), why.to_string())
    })?;

    let deadline_ms = options.seconds.map(|s| s.saturating_mul(1_000));
    let mut emitted = 0usize;
    let mut unreadable = 0usize;
    let mut first = true;

    let stopped = loop {
        let appended = journal.appended().map_err(refusal_from)?;
        let (records, refused) = parse_lines(appended.lines.iter().map(String::as_str));
        unreadable += refused.len();
        for (line, why) in &refused {
            console.note(&format!(
                "rivet: line {line} of the journal could not be read: {why}. It is NOT in \
                 this stream. A newer Rivet usually wrote it."
            ));
        }
        if appended.rotated {
            console.note(
                "rivet: the journal rotated (ADR-022 §8). Records already written are \
                 carried across; the stream continues.",
            );
        }
        if let Some(missed) = &appended.missed {
            console.note(&format!("rivet: {missed}"));
        }

        // The first read is the backfill — the records already on the card —
        // and it is the act that pins which boot is being followed.
        let (out, note) = if first {
            first = false;
            (follow.backfill(records, options.limit), None)
        } else {
            let got = follow.more(records);
            (got.records, got.note)
        };
        if let Some(note) = note {
            console.note(&say(&note));
        }

        let mut ended = None;
        for record in &out {
            if !console.line(record.to_jsonl().trim_end()) {
                ended = Some(Stopped::ReaderGone);
                break;
            }
            emitted += 1;
            if emitted >= FOLLOW_RECORDS_MAX {
                ended = Some(Stopped::Ceiling);
                break;
            }
        }
        if let Some(end) = ended {
            break end;
        }

        // **Checked after the records rather than before them**, so that a
        // follow whose deadline has already passed still emits its backfill.
        // `--for 0` is then "what is on the card now, as a stream", which is a
        // sensible thing to have asked for rather than an empty answer.
        if let Some(limit) = deadline_ms
            && console.elapsed_ms() >= limit
        {
            break Stopped::Deadline;
        }
        console.sleep_ms(FOLLOW_POLL_MS);
    };

    let summary = Value::Object(vec![
        ("subject", Value::str("log")),
        ("follow", Value::str(follow.boot().unwrap_or("none yet"))),
        (
            "emitted",
            Value::Integer(i64::try_from(emitted).unwrap_or(i64::MAX)),
        ),
        (
            "unreadable",
            Value::Integer(i64::try_from(unreadable).unwrap_or(i64::MAX)),
        ),
        (
            "seconds",
            Value::Integer(i64::try_from(console.elapsed_ms() / 1_000).unwrap_or(i64::MAX)),
        ),
        ("stopped", Value::str(stopped.as_str())),
        ("why", Value::str(stopped.why())),
        ("source", Value::str(journal.source().to_string())),
    ]);
    Ok(summary)
}

/// The sentence a [`FollowNote`] becomes.
fn say(note: &FollowNote) -> String {
    match note {
        FollowNote::BootChanged {
            from,
            to,
            switched: true,
        } => format!(
            "rivet: this machine restarted — the journal is now being written under boot \
             {to}, and was under {from}. `latest` was asked for, so the stream follows it."
        ),
        FollowNote::BootChanged {
            from,
            to,
            switched: false,
        } => format!(
            "rivet: the journal is now being written under boot {to}. This stream follows \
             {from}, which was asked for by name, so nothing more will appear in it."
        ),
    }
}

/// A machine that would not answer, carried out unchanged.
fn refusal_from(e: MachineError) -> Refusal {
    Refusal::about(Code::MACHINE_REFUSED, "log", e.to_string())
}

/// [`Console`] over the real streams and the real clock.
///
/// **The only part of a follow that touches the world**, and it is deliberately
/// this small: three of its four methods are one statement.
pub struct LiveConsole {
    started: std::time::Instant,
    /// Set once stdout has refused a write. Checked rather than retried — a
    /// pipe that has closed does not reopen.
    broken: bool,
}

impl LiveConsole {
    #[must_use]
    pub fn new() -> Self {
        Self {
            started: std::time::Instant::now(),
            broken: false,
        }
    }
}

impl Default for LiveConsole {
    fn default() -> Self {
        Self::new()
    }
}

impl Console for LiveConsole {
    /// **`writeln!` and an explicit flush, not `println!`.**
    ///
    /// Two reasons, and both are about the far end of a channel. `println!`
    /// *panics* when stdout will not take the write, which would turn a reader
    /// closing a pipe — `rivet get log --follow | head -5`, an ordinary thing
    /// to do — into a crash with a backtrace; and Rust's stdout is line
    /// buffered when it is a terminal and **block** buffered when it is not, so
    /// a stream down a pipe would arrive in eight-kilobyte lumps and stop being
    /// a stream at all.
    fn line(&mut self, text: &str) -> bool {
        use std::io::Write as _;

        if self.broken {
            return false;
        }
        let mut out = std::io::stdout().lock();
        if writeln!(out, "{text}").is_err() || out.flush().is_err() {
            self.broken = true;
            return false;
        }
        true
    }

    fn note(&mut self, text: &str) {
        eprintln!("{text}");
    }

    fn elapsed_ms(&mut self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn sleep_ms(&mut self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::journal::Appended;

    /// A console with a clock that only moves when the follower sleeps.
    ///
    /// **This is what makes the bounds testable at all.** A five-minute
    /// deadline proved by waiting five minutes is a test nobody runs twice, and
    /// `AGENTS.md` §4 is explicit that a suite which cannot be run rots.
    struct FakeConsole {
        lines: Vec<String>,
        notes: Vec<String>,
        now_ms: u64,
        /// Stop accepting lines after this many. `None` never refuses.
        accepts: Option<usize>,
    }

    impl FakeConsole {
        fn new() -> Self {
            Self {
                lines: Vec::new(),
                notes: Vec::new(),
                now_ms: 0,
                accepts: None,
            }
        }

        fn hanging_up_after(n: usize) -> Self {
            Self {
                accepts: Some(n),
                ..Self::new()
            }
        }
    }

    impl Console for FakeConsole {
        fn line(&mut self, text: &str) -> bool {
            if self.accepts.is_some_and(|n| self.lines.len() >= n) {
                return false;
            }
            self.lines.push(text.to_string());
            true
        }
        fn note(&mut self, text: &str) {
            self.notes.push(text.to_string());
        }
        fn elapsed_ms(&mut self) -> u64 {
            self.now_ms
        }
        fn sleep_ms(&mut self, ms: u64) {
            self.now_ms += ms;
        }
    }

    /// A journal that hands out scripted reads and then nothing for ever.
    struct FakeJournal {
        reads: std::collections::VecDeque<Result<Appended, MachineError>>,
    }

    impl FakeJournal {
        fn of(chunks: &[&[&str]]) -> Self {
            Self {
                reads: chunks
                    .iter()
                    .map(|lines| {
                        Ok(Appended {
                            lines: lines.iter().map(|l| (*l).to_string()).collect(),
                            rotated: false,
                            missed: None,
                        })
                    })
                    .collect(),
            }
        }

        fn refusing(why: &str) -> Self {
            Self {
                reads: std::collections::VecDeque::from(vec![Err(MachineError::Refused(
                    why.to_string(),
                ))]),
            }
        }
    }

    impl Appending for FakeJournal {
        fn appended(&mut self) -> Result<Appended, MachineError> {
            self.reads
                .pop_front()
                .unwrap_or_else(|| Ok(Appended::default()))
        }
        fn source(&self) -> &str {
            "/state/log/journal.jsonl"
        }
    }

    fn line(boot: &str, mono: f64, msg: &str) -> String {
        format!(
            r#"{{"boot":"{boot}","mono":{mono:.3},"sev":"info","svc":"rivet-sysd","corr":"c","actor":"supervisor","msg":"{msg}"}}"#
        )
    }

    fn words(w: &[&str]) -> Vec<String> {
        w.iter().map(|s| (*s).to_string()).collect()
    }

    fn options(seconds: Option<u64>, limit: Option<usize>) -> Options {
        Options {
            boot: "latest".to_string(),
            limit,
            seconds,
        }
    }

    fn field(v: &Value, key: &str) -> Value {
        match v {
            Value::Object(fields) => fields
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| panic!("no {key} in {v:?}")),
            other => panic!("not an object: {other:?}"),
        }
    }

    // -- reading the words --------------------------------------------------

    #[test]
    fn the_flag_is_taken_out_and_the_positional_words_are_left_alone() {
        let (got, kept) = extract(true, &words(&["log", "latest", "20", FLAG])).unwrap();
        assert_eq!(kept, words(&["log", "latest", "20"]));
        let got = got.expect("--follow was not recognised");
        assert_eq!(got.boot, "latest");
        assert_eq!(got.limit, Some(20));
        assert_eq!(got.seconds, Some(FOLLOW_SECONDS_DEFAULT));
    }

    #[test]
    fn a_command_without_the_flag_is_untouched_and_follows_nothing() {
        let (got, kept) = extract(true, &words(&["log", "previous"])).unwrap();
        assert_eq!(got, None);
        assert_eq!(kept, words(&["log", "previous"]));
    }

    #[test]
    fn the_duration_is_read_in_both_spellings() {
        for spelling in [
            words(&["log", FLAG, FOR, "60"]),
            words(&["log", FLAG, "--for=60"]),
        ] {
            let (got, _) = extract(true, &spelling).unwrap();
            assert_eq!(got.expect("--follow lost").seconds, Some(60), "{spelling:?}");
        }
    }

    #[test]
    fn forever_is_the_word_that_removes_the_deadline() {
        let (got, _) = extract(true, &words(&["log", FLAG, FOR, FOLLOW_FOREVER])).unwrap();
        assert_eq!(got.expect("--follow lost").seconds, None);
    }

    /// The default is the whole point of the bound: a caller who says nothing
    /// gets a command that returns.
    #[test]
    fn saying_nothing_about_the_duration_is_not_forever() {
        let (got, _) = extract(true, &words(&["log", FLAG])).unwrap();
        assert_eq!(
            got.expect("--follow lost").seconds,
            Some(FOLLOW_SECONDS_DEFAULT)
        );
        assert_ne!(FOLLOW_SECONDS_DEFAULT, 0);
    }

    #[test]
    fn the_backfills_limit_defaults_the_same_way_a_one_shot_get_log_does() {
        let (got, _) = extract(true, &words(&["log", FLAG])).unwrap();
        assert_eq!(
            got.expect("--follow lost").limit,
            Some(rivet_core::operation::log::LIMIT_DEFAULT)
        );
    }

    #[test]
    fn following_a_subject_that_is_not_the_journal_is_refused_by_name() {
        let refused = extract(true, &words(&["storage", FLAG]));
        let Err(r) = refused else {
            panic!("--follow was accepted on storage");
        };
        assert_eq!(r.subject.as_deref(), Some(FLAG));
        assert!(r.message.contains("get log"), "{}", r.message);
    }

    #[test]
    fn following_a_verb_that_is_not_get_is_refused_too() {
        assert!(extract(false, &words(&["log", FLAG])).is_err());
    }

    #[test]
    fn a_duration_without_a_follow_is_refused_rather_than_ignored() {
        let Err(r) = extract(true, &words(&["log", FOR, "60"])) else {
            panic!("--for was accepted without --follow");
        };
        assert_eq!(r.subject.as_deref(), Some(FOR));
    }

    #[test]
    fn a_duration_that_is_not_a_number_is_refused_and_names_what_it_accepts() {
        let Err(r) = extract(true, &words(&["log", FLAG, FOR, "soon"])) else {
            panic!("`--for soon` was accepted");
        };
        assert_eq!(r.subject.as_deref(), Some("soon"));
        assert!(r.message.contains(FOLLOW_FOREVER), "{}", r.message);
    }

    #[test]
    fn a_duration_with_nothing_after_it_is_refused() {
        assert!(extract(true, &words(&["log", FLAG, FOR])).is_err());
    }

    // -- the stream ---------------------------------------------------------

    #[test]
    fn the_backfill_comes_first_and_then_what_is_appended() {
        let mut console = FakeConsole::new();
        let mut journal = FakeJournal::of(&[
            &[&line("boot-a", 1.0, "one"), &line("boot-a", 2.0, "two")],
            &[],
            &[&line("boot-a", 3.0, "three")],
        ]);
        let summary = stream(&options(Some(2), None), &mut console, &mut journal).unwrap();

        assert_eq!(console.lines.len(), 3, "{:?}", console.lines);
        assert!(console.lines[0].contains("one"));
        assert!(console.lines[2].contains("three"));
        assert_eq!(field(&summary, "emitted"), Value::Integer(3));
        assert_eq!(field(&summary, "stopped"), Value::str("deadline"));
        assert_eq!(field(&summary, "follow"), Value::str("boot-a"));
    }

    /// **The bound, proved rather than asserted.** The clock only moves when
    /// the follower sleeps, so this test says exactly how far a two-second
    /// follow gets before it stops.
    #[test]
    fn the_deadline_ends_the_stream_and_the_summary_says_so() {
        let mut console = FakeConsole::new();
        let mut journal = FakeJournal::of(&[&[]]);
        let summary = stream(&options(Some(2), None), &mut console, &mut journal).unwrap();
        assert_eq!(console.now_ms, 2_000, "it did not stop at its deadline");
        assert_eq!(field(&summary, "stopped"), Value::str("deadline"));
        assert_eq!(field(&summary, "seconds"), Value::Integer(2));
    }

    /// A stream with no deadline is only reachable by asking for one, and it
    /// still stops — on the record ceiling, which is the bound a clock cannot
    /// enforce.
    #[test]
    fn forever_still_stops_at_the_record_ceiling_and_says_which_bound_it_was() {
        let flood: Vec<String> = (0..FOLLOW_RECORDS_MAX + 10)
            .map(|n| {
                #[allow(clippy::cast_precision_loss)]
                let mono = n as f64;
                line("boot-a", mono, "flooding")
            })
            .collect();
        let refs: Vec<&str> = flood.iter().map(String::as_str).collect();
        let mut console = FakeConsole::new();
        let mut journal = FakeJournal::of(&[&refs]);

        let summary = stream(&options(None, None), &mut console, &mut journal).unwrap();
        assert_eq!(console.lines.len(), FOLLOW_RECORDS_MAX);
        assert_eq!(field(&summary, "stopped"), Value::str("record-ceiling"));
    }

    /// `rivet get log --follow | head -5` is an ordinary thing to type, and
    /// the far end closing must end the stream rather than crash it.
    #[test]
    fn a_reader_that_hangs_up_ends_the_stream_at_the_next_line() {
        let mut console = FakeConsole::hanging_up_after(1);
        let mut journal =
            FakeJournal::of(&[&[&line("boot-a", 1.0, "one"), &line("boot-a", 2.0, "two")]]);
        let summary = stream(&options(None, None), &mut console, &mut journal).unwrap();
        assert_eq!(console.lines.len(), 1);
        assert_eq!(field(&summary, "stopped"), Value::str("reader-closed"));
        assert_eq!(field(&summary, "emitted"), Value::Integer(1));
    }

    /// **The honesty field, in a stream.** A line the machine wrote and this
    /// build cannot read is counted and named — it must not simply be absent,
    /// which is indistinguishable from it never having been written.
    #[test]
    fn a_line_that_will_not_parse_is_counted_and_noted_rather_than_dropped() {
        let mut console = FakeConsole::new();
        let mut journal = FakeJournal::of(&[&[&line("boot-a", 1.0, "one"), "{not a record"]]);
        let summary = stream(&options(Some(0), None), &mut console, &mut journal).unwrap();

        assert_eq!(console.lines.len(), 1);
        assert_eq!(field(&summary, "unreadable"), Value::Integer(1));
        assert!(
            console.notes.iter().any(|n| n.contains("could not be read")),
            "{:?}",
            console.notes
        );
    }

    /// A machine that will not answer is a refusal, with the machine's own
    /// sentence carried out unchanged.
    #[test]
    fn a_journal_that_refuses_to_be_read_is_a_refusal_and_not_an_empty_stream() {
        let mut console = FakeConsole::new();
        let mut journal = FakeJournal::refusing("/state/log/journal.jsonl: permission denied");
        let Err(r) = stream(&options(Some(1), None), &mut console, &mut journal) else {
            panic!("an unreadable journal was reported as an empty stream");
        };
        assert!(r.message.contains("permission denied"), "{}", r.message);
        assert!(console.lines.is_empty());
    }

    #[test]
    fn following_the_previous_boot_is_refused_before_anything_is_read() {
        let mut console = FakeConsole::new();
        let mut journal = FakeJournal::of(&[&[&line("boot-a", 1.0, "one")]]);
        let asked = Options {
            boot: "previous".to_string(),
            limit: None,
            seconds: Some(1),
        };
        let Err(r) = stream(&asked, &mut console, &mut journal) else {
            panic!("`--follow previous` was accepted");
        };
        assert_eq!(r.subject.as_deref(), Some("previous"));
        assert!(console.lines.is_empty());
    }

    /// **stdout carries records and nothing else.** Everything a follower says
    /// about itself is on stderr, so a pipeline reading stdout never has to
    /// skip a line — measured on hardware, and it costs exactly one.
    #[test]
    fn every_line_on_stdout_is_a_record_the_machine_wrote() {
        let mut console = FakeConsole::new();
        let mut journal = FakeJournal::of(&[&[&line("boot-a", 1.0, "one")], &[]]);
        stream(&options(Some(1), None), &mut console, &mut journal).unwrap();
        assert!(!console.lines.is_empty());
        for out in &console.lines {
            rivet_core::journal::Record::from_jsonl(out).unwrap_or_else(|e| {
                panic!("stdout carried a line that is not a record: {out} ({e})")
            });
        }
    }

    /// The backfill is limited; what arrives afterwards is not. A limit that
    /// kept applying would drop records the stream had been asked to show.
    #[test]
    fn the_limit_cuts_the_backfill_and_nothing_after_it() {
        let mut console = FakeConsole::new();
        let mut journal = FakeJournal::of(&[
            &[
                &line("boot-a", 1.0, "one"),
                &line("boot-a", 2.0, "two"),
                &line("boot-a", 3.0, "three"),
            ],
            &[&line("boot-a", 4.0, "four"), &line("boot-a", 5.0, "five")],
        ]);
        stream(&options(Some(1), Some(1)), &mut console, &mut journal).unwrap();
        assert_eq!(console.lines.len(), 3, "{:?}", console.lines);
        assert!(
            console.lines[0].contains("three"),
            "the backfill keeps the LAST one"
        );
        assert!(console.lines[1].contains("four"));
        assert!(console.lines[2].contains("five"));
    }
}
