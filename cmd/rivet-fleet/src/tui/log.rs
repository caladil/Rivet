//! **The journal, read off a machine** — the pane that was a note about a
//! defect until the maintainer pointed out that a note is not a fix.
//!
//! # Polling, and what that costs
//!
//! `rivet get log --follow` streams, and **`--follow` never reaches the
//! operation catalogue**: `cmd/rivet` strips it before `perform`, deliberately,
//! because it says how an answer is *delivered* rather than what is being asked
//! (`ADR-022` §9.1). So it exists in the CLI and in no other adapter.
//!
//! Whether that should change is a **ruling**, not a defect to fix here: making
//! it an operation input contradicts §9.1's *the operation answers once*, and
//! the alternative is MCP notifications, which `ADR-032` §4.3 lists as open.
//! Both are the maintainer's.
//!
//! What needs no ruling is this pane working, so it **polls** — `rivet get log`
//! on a timer, which every adapter can already do. That is worse than a stream
//! by exactly one round trip per refresh, and a round trip is 213 ms since
//! `PLAN.md` §51.
//!
//! # It reads, and nothing here writes
//!
//! A journal is the machine's account of itself. This pane cannot change one
//! and does not try — so unlike the panes beside it there is no draft, no
//! dialog and nothing to confirm.

use std::sync::mpsc::{Receiver, Sender, channel};

/// How many records to ask for.
///
/// **200**, which is `ADR-022` §9.6's own default for a caller that names no
/// limit — so this pane asks for what the CLI would give and not for something
/// only it knows to want.
pub const RECORDS: usize = 200;

/// One line, as this pane shows it.
///
/// **Already flattened.** A record is JSON and this is the three fields a
/// person reads; keeping the rest would mean the pane deciding what to hide at
/// draw time, and a filter that runs during a draw is a filter that runs on
/// every keystroke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub severity: String,
    pub service: String,
    pub message: String,
}

impl Entry {
    /// Whether this line matches what a person typed.
    ///
    /// **Case-insensitive, and across all three fields**, because a person
    /// filtering a journal is looking for a word and does not know or care
    /// which column it is in.
    #[must_use]
    pub fn matches(&self, needle: &str) -> bool {
        if needle.is_empty() {
            return true;
        }
        let needle = needle.to_lowercase();
        self.severity.to_lowercase().contains(&needle)
            || self.service.to_lowercase().contains(&needle)
            || self.message.to_lowercase().contains(&needle)
    }

    /// Whether this is a line a person filtering for trouble wants.
    #[must_use]
    pub fn is_trouble(&self) -> bool {
        matches!(self.severity.as_str(), "warn" | "error")
    }
}

/// What one fetch produced.
#[derive(Debug, Clone)]
pub enum Fetched {
    Entries(Vec<Entry>),
    /// **The machine's own refusal, or the connection's.** Shown rather than
    /// swallowed: a journal pane that silently showed nothing would be
    /// indistinguishable from a machine with nothing to say.
    Failed(String),
}

/// Fetches running in the background.
///
/// `crate::tui::reach::Probes`' shape and its reason: the interface never waits
/// for a network.
pub struct Fetches {
    to_ui: Sender<(String, Fetched)>,
    from_fetch: Receiver<(String, Fetched)>,
    /// Whether one is in flight, so a slow machine does not accumulate a queue
    /// of identical requests behind it.
    running: std::cell::Cell<bool>,
}

impl Default for Fetches {
    fn default() -> Self {
        Self::new()
    }
}

impl Fetches {
    #[must_use]
    pub fn new() -> Self {
        let (to_ui, from_fetch) = channel();
        Self {
            to_ui,
            from_fetch,
            running: std::cell::Cell::new(false),
        }
    }

    /// Whether a fetch is already in flight.
    ///
    /// Read by the test that holds the one-at-a-time rule; `ask` enforces it
    /// itself, so nothing in the interface has to ask first.
    #[allow(dead_code)]
    #[must_use]
    pub fn busy(&self) -> bool {
        self.running.get()
    }

    /// Ask a machine for its journal, over a transport already pointed at it.
    /// Returns at once.
    ///
    /// **Does nothing if one is already running.** A person holding `r`, or a
    /// follow timer firing faster than a slow machine answers, would otherwise
    /// open a connection per tick.
    ///
    /// The transport is built by the caller, because building one reads this
    /// operator's identity off the disk and that is the application's business
    /// rather than this pane's.
    pub fn ask(&self, id: &str, transport: rivet_remote::tls::Tls) {
        if self.running.get() {
            return;
        }
        self.running.set(true);
        let (id, to_ui) = (id.to_owned(), self.to_ui.clone());
        let named = transport.host.clone();
        let _ = std::thread::Builder::new()
            .name(format!("log:{named}"))
            .spawn(move || {
                let _ = to_ui.send((id, fetch(&transport)));
            });
    }

    /// **Say why no machine was asked**, without opening anything.
    ///
    /// A run with no `--identity`, or a device with no pin, has an answer this
    /// end already knows. It arrives through the same channel as a real one, so
    /// the pane has one place answers come from and shows a refusal exactly as
    /// it shows a machine's.
    pub fn cannot_ask(&self, id: &str, why: String) {
        let _ = self.to_ui.send((id.to_owned(), Fetched::Failed(why)));
    }

    /// Whatever has arrived. Never blocks.
    pub fn collected(&self) -> Vec<(String, Fetched)> {
        let out: Vec<_> = self.from_fetch.try_iter().collect();
        if !out.is_empty() {
            self.running.set(false);
        }
        out
    }
}

/// Ask one machine, synchronously. This is what runs on the fetch thread.
///
/// # The words are the transport's only input
///
/// `ADR-034` §11.5.1 refused every shell-out — *"there is nothing left at the
/// far end"* — so a process opened per refresh was never a fallback; it was a
/// transport that could not work.
///
/// **The words are the same words** and that matters: they are what an operator
/// would type, which is `ADR-006` §3.1's conformance property reaching as far as
/// this pane. What changed is that they are bound to
/// `rivet_core::operation::CATALOGUE` by `rivet_remote::command` and sent as a
/// tool call, rather than handed to a shell. `get log latest 200` is one
/// operation with two of its declared inputs filled, on both routes.
fn fetch(transport: &rivet_remote::tls::Tls) -> Fetched {
    match rivet_remote::command::perform(
        transport,
        &["get", "log", "latest", &RECORDS.to_string()],
    ) {
        Ok(text) => Fetched::Entries(parse(&text)),
        Err(e) => Fetched::Failed(e),
    }
}

/// Read what `rivet get log` answered.
///
/// **Tolerant of a line it cannot read, and not of nothing at all.** A journal
/// with one malformed record is still worth showing; a response that yields no
/// records when the machine had some is a bug this pane must not paper over,
/// which is why an empty answer stays empty rather than becoming a friendly
/// sentence.
#[must_use]
pub fn parse(text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    // The answer is one JSON object with a `records` array of JSONL strings.
    // Rather than carry a second JSON reader here, the fields are lifted with
    // the same string scan the journal's own `from_jsonl` uses — this pane
    // shows three fields and does not need a parse tree for them.
    for line in text.lines() {
        for record in line.split("{\\\"ts\\\"").skip(1) {
            if let Some(entry) = entry_from(record) {
                out.push(entry);
            }
        }
    }
    if out.is_empty() {
        // Not JSONL-in-JSON: try it as plain records, one per line, which is
        // what `--output text` and a raw journal file both give.
        for line in text.lines() {
            if let Some(entry) = entry_from(line) {
                out.push(entry);
            }
        }
    }
    out
}

/// One record's three fields, lifted out of its text.
fn entry_from(record: &str) -> Option<Entry> {
    let severity = field(record, "sev")?;
    Some(Entry {
        service: field(record, "svc").unwrap_or_else(|| "—".to_string()),
        message: field(record, "msg").unwrap_or_default(),
        severity,
    })
}

/// The value of one string field, however many backslashes it is wrapped in.
fn field(record: &str, name: &str) -> Option<String> {
    for opener in [
        format!("\"{name}\":\""),
        format!("\\\"{name}\\\":\\\""),
    ] {
        let Some(start) = record.find(&opener) else {
            continue;
        };
        let rest = &record[start + opener.len()..];
        // The value ends at the first quote that is not escaped.
        let mut out = String::new();
        let mut chars = rest.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    // `\"` inside, or `\\"` in the doubly-wrapped form. Either
                    // way the next character is literal unless it closes.
                    match chars.peek() {
                        Some('"') => {
                            chars.next();
                            if out.is_empty() {
                                continue;
                            }
                            return Some(out);
                        }
                        Some('n') => {
                            chars.next();
                            out.push(' ');
                        }
                        Some(other) => {
                            let other = *other;
                            chars.next();
                            out.push(other);
                        }
                        None => break,
                    }
                }
                '"' => return Some(out),
                c => out.push(c),
            }
        }
        if !out.is_empty() {
            return Some(out);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **A real answer**, shaped as `rivet get log` actually renders one:
    /// an object whose `records` are JSONL strings, so every inner quote is
    /// escaped.
    const REAL: &str = r#"{"subject":"log","records":["{\"ts\":\"2026-08-27T00:15:50Z\",\"sev\":\"info\",\"svc\":\"rivet-mdnsd\",\"msg\":\"announcing board.local\"}","{\"ts\":\"2026-08-27T00:15:56Z\",\"sev\":\"warn\",\"svc\":\"rivet-timed\",\"msg\":\"something worth reading\"}"]}"#;

    #[test]
    fn a_real_answer_becomes_the_lines_a_person_reads() {
        let entries = parse(REAL);
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_eq!(entries[0].severity, "info");
        assert_eq!(entries[0].service, "rivet-mdnsd");
        assert!(entries[0].message.contains("announcing"), "{:?}", entries[0]);
        assert_eq!(entries[1].severity, "warn");
        assert_eq!(entries[1].service, "rivet-timed");
    }

    /// **Filtering is one word across every column**, because a person looking
    /// for something does not know which column it is in.
    #[test]
    fn a_filter_matches_any_column_and_ignores_case() {
        let entries = parse(REAL);
        assert!(entries[0].matches("MDNSD"), "service, wrong case");
        assert!(entries[0].matches("announcing"), "message");
        assert!(entries[0].matches("info"), "severity");
        assert!(!entries[0].matches("rivet-timed"), "matched another row");
        // An empty filter shows everything, which is what no filter means.
        assert!(entries[0].matches(""));
    }

    #[test]
    fn trouble_is_warn_and_error_and_nothing_else() {
        let entries = parse(REAL);
        assert!(!entries[0].is_trouble(), "info is not trouble");
        assert!(entries[1].is_trouble(), "warn is trouble");
    }

    /// **An empty answer stays empty.** A pane that turned "no records" into a
    /// friendly sentence would hide a fetch that silently returned nothing.
    #[test]
    fn nothing_parses_to_nothing_rather_than_to_a_reassurance() {
        assert!(parse("").is_empty());
        assert!(parse("not json at all").is_empty());
        assert!(parse(r#"{"subject":"log","records":[]}"#).is_empty());
    }

    /// A record missing the field that says how bad it is, is not a record this
    /// pane can place — so it is skipped rather than shown as unclassified.
    #[test]
    fn a_record_with_no_severity_is_skipped_and_the_rest_survive() {
        let mixed = r#"{"records":["{\"ts\":\"x\",\"svc\":\"a\",\"msg\":\"no severity\"}","{\"ts\":\"y\",\"sev\":\"error\",\"svc\":\"b\",\"msg\":\"kept\"}"]}"#;
        let entries = parse(mixed);
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].message, "kept");
    }

    /// A transport pointed at a name that does not resolve, so the fetch thread
    /// finishes quickly and reaches no machine.
    fn nowhere() -> rivet_remote::tls::Tls {
        rivet_remote::tls::Tls::new(
            "this-name-does-not-resolve.invalid:7777".to_owned(),
            rivetos_tls::Identity { certificate: vec![1, 2, 3], seed: [7_u8; 32] },
            vec![[9_u8; 32]],
        )
    }

    /// **One fetch at a time.** A person holding a key, or a follow timer
    /// firing faster than a slow machine answers, must not open a connection
    /// per tick.
    #[test]
    fn a_second_fetch_is_refused_while_the_first_is_in_flight() {
        let fetches = Fetches::new();
        assert!(!fetches.busy());
        fetches.ask("an-id", nowhere());
        assert!(fetches.busy(), "the first did not register as running");
        // The second is dropped rather than queued.
        fetches.ask("an-id", nowhere());
        assert!(fetches.busy());
    }

    /// **A refusal this end composed arrives the same way a machine's does.**
    /// A pane with two ways of learning it has nothing to show is a pane with
    /// two ways of drawing it.
    #[test]
    fn being_unable_to_ask_arrives_through_the_same_channel_as_an_answer() {
        let fetches = Fetches::new();
        fetches.cannot_ask("an-id", "no --identity was given".to_owned());
        let collected = fetches.collected();
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].0, "an-id");
        match &collected[0].1 {
            Fetched::Failed(why) => assert!(why.contains("--identity"), "{why}"),
            other => panic!("a refusal must not read as records: {other:?}"),
        }
    }

    /// Collecting never blocks, so a draw between fetches is a draw.
    #[test]
    fn collecting_never_blocks() {
        let fetches = Fetches::new();
        assert!(fetches.collected().is_empty());
    }

    /// **A failure is shown, not swallowed.** A pane that quietly showed
    /// nothing would look the same as a machine with nothing to say.
    #[test]
    fn a_failure_is_a_thing_the_pane_can_display() {
        let failed = Fetched::Failed("the connection ended".to_string());
        match failed {
            Fetched::Failed(why) => assert!(!why.is_empty()),
            Fetched::Entries(_) => panic!("wrong variant"),
        }
    }
}
