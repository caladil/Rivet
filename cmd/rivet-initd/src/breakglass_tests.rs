//! What the console reader must do, and the three §8A.1.2 constraints asserted
//! rather than described.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::testkernel::FakeKernel;

fn feed(k: &mut FakeKernel, bytes: &str) {
    k.console_in.extend_from_slice(bytes.as_bytes());
}

#[test]
fn nothing_waiting_is_no_line_and_not_an_error() {
    let mut k = FakeKernel::new();
    let mut c = Console::new();
    assert_eq!(c.poll(&mut k), None);
}

#[test]
fn a_whole_line_arrives_as_one() {
    let mut k = FakeKernel::new();
    let mut c = Console::new();
    feed(&mut k, "get slot\n");
    assert_eq!(c.poll(&mut k), Some(Line::Ready("get slot".to_string())));
    assert_eq!(c.poll(&mut k), None);
}

/// **The case a serial console actually produces.** A person typing delivers
/// one byte per read, and a reader that only worked on whole lines would never
/// see a command at all.
#[test]
fn a_line_typed_one_byte_at_a_time_is_assembled() {
    let mut k = FakeKernel::new();
    let mut c = Console::new();
    for ch in "get slot".chars() {
        feed(&mut k, &ch.to_string());
        assert_eq!(c.poll(&mut k), None, "a partial line must not be served");
    }
    feed(&mut k, "\n");
    assert_eq!(c.poll(&mut k), Some(Line::Ready("get slot".to_string())));
}

/// A terminal on a serial cable sends `\r\n`, and a person cannot see the
/// difference between that and `\n`.
#[test]
fn carriage_returns_are_stripped() {
    let mut k = FakeKernel::new();
    let mut c = Console::new();
    feed(&mut k, "get slot\r\n");
    assert_eq!(c.poll(&mut k), Some(Line::Ready("get slot".to_string())));
}

/// **One line per poll, and the rest is kept.** Pid 1's tick has a supervisor
/// to keep alive; draining a paste before reaping would be this module
/// deciding it outranks the loop it lives in.
#[test]
fn two_lines_in_one_read_come_out_one_per_poll() {
    let mut k = FakeKernel::new();
    let mut c = Console::new();
    feed(&mut k, "first\nsecond\n");
    assert_eq!(c.poll(&mut k), Some(Line::Ready("first".to_string())));
    assert_eq!(c.poll(&mut k), Some(Line::Ready("second".to_string())));
    assert_eq!(c.poll(&mut k), None);
}

/// An empty line is a line. Whether it means anything is the dispatcher's
/// question, and answering it here would put policy in the framing.
#[test]
fn a_bare_newline_is_an_empty_line_rather_than_nothing() {
    let mut k = FakeKernel::new();
    let mut c = Console::new();
    feed(&mut k, "\n");
    assert_eq!(c.poll(&mut k), Some(Line::Ready(String::new())));
}

/// **`ADR-035` §8A.1.2's *must not allocate unboundedly*, asserted.** The
/// buffer is fixed, so the only correct answer to a line that will not fit is
/// to refuse it — and to SAY so, because a command that looks ignored is worse
/// than one that was refused.
#[test]
fn a_line_longer_than_the_buffer_is_refused_and_not_silently_truncated() {
    let mut k = FakeKernel::new();
    let mut c = Console::new();
    let huge = "x".repeat(LINE_MAX + 100);
    feed(&mut k, &huge);
    feed(&mut k, "\n");
    // Reading is chunked, so poll until it resolves; it must resolve to the
    // refusal and never to a truncated command.
    let mut got = None;
    for _ in 0..10 {
        if let Some(line) = c.poll(&mut k) {
            got = Some(line);
            break;
        }
    }
    assert_eq!(got, Some(Line::TooLong));
}

/// And the reader RECOVERS. An overlong paste must not wedge break-glass for
/// the rest of the boot — the next command has to work.
#[test]
fn the_line_after_an_overlong_one_is_served_normally() {
    let mut k = FakeKernel::new();
    let mut c = Console::new();
    feed(&mut k, &"x".repeat(LINE_MAX + 10));
    feed(&mut k, "\napply restart confirm\n");
    let mut seen = Vec::new();
    for _ in 0..10 {
        if let Some(line) = c.poll(&mut k) {
            seen.push(line);
        }
    }
    assert!(seen.contains(&Line::TooLong), "{seen:?}");
    assert!(
        seen.contains(&Line::Ready("apply restart confirm".to_string())),
        "the reader did not recover: {seen:?}"
    );
}

/// **A console that cannot be read is not a reason for pid 1 to stop.** The
/// machine still has a supervisor to keep alive, and break-glass being
/// unavailable is the situation §8A's other tiers exist for.
#[test]
fn an_unreadable_console_is_reported_once_and_pid_one_carries_on() {
    let mut k = FakeKernel::new();
    k.console_fails = true;
    let mut c = Console::new();
    for _ in 0..5 {
        assert_eq!(c.poll(&mut k), None);
    }
    let complaints = k
        .lines
        .iter()
        .filter(|l| l.contains("console cannot be read"))
        .count();
    assert_eq!(complaints, 1, "said it {complaints} times: {:?}", k.lines);
}

/// Invalid UTF-8 must not panic pid 1. Serial lines carry noise — an unplugged
/// cable produces a framing error and a byte nobody sent.
#[test]
fn rubbish_bytes_do_not_panic_and_come_back_as_a_line() {
    let mut k = FakeKernel::new();
    k.console_in.extend_from_slice(&[0xff, 0xfe, b'\n']);
    let mut c = Console::new();
    assert!(matches!(c.poll(&mut k), Some(Line::Ready(_))));
}

// ---------------------------------------------------------------------------
// The dispatcher — `ADR-035` §8A.1.2's *refusing anything it cannot serve*
// ---------------------------------------------------------------------------

use rivet_core::state::Slot;

fn serve_line(k: &mut FakeKernel, line: &str) -> Option<crate::kernel::RebootHow> {
    super::serve(line, k, Slot::A)
}

/// The last thing said, which is the answer to the command just served.
fn answer(k: &FakeKernel) -> String {
    k.lines.last().cloned().unwrap_or_default()
}

/// **The repair begins by reading the thing that is broken.** If break-glass
/// could not do this, an operator would be editing a document blind.
#[test]
fn the_document_can_be_read_over_the_console() {
    let mut k = FakeKernel::new();
    k.files.insert(
        rivet_core::state::MACHINE_DOCUMENT.to_string(),
        crate::testkernel::Script::always(Ok("version = 1\n".to_string())),
    );
    assert_eq!(serve_line(&mut k, "get desired-state"), None);
    let said = answer(&k);
    assert!(said.contains("\"version = 1"), "{said}");
    assert!(said.contains("\"present\":true"), "{said}");
}

/// **A first boot has no document, and that is an answer rather than a fault.**
/// Reporting it as an error would make an ordinary state look like the thing
/// break-glass was called for.
#[test]
fn a_machine_with_no_document_says_so_rather_than_failing() {
    let mut k = FakeKernel::new();
    assert_eq!(serve_line(&mut k, "get desired-state"), None);
    assert!(answer(&k).contains("\"present\":false"), "{}", answer(&k));
}

/// **`get status` is the sharp refusal.** It is the first thing an operator
/// reaches for and its absence looks like an oversight — it is not. The
/// supervisor answers it, and Tier 1 exists for the case where the supervisor
/// is what failed.
#[test]
fn get_status_is_refused_and_says_why_rather_than_hanging() {
    let mut k = FakeKernel::new();
    assert_eq!(serve_line(&mut k, "get status"), None);
    let said = answer(&k);
    assert!(said.contains("BREAK-GLASS cannot answer"), "{said}");
    assert!(said.contains("get status"), "{said}");
    assert!(
        said.contains("CONTROL PLANE"),
        "a refusal must name where the answer lives: {said}"
    );
}

/// Everything that would need the network, `/proc` or the package format is
/// refused for the same reason and in the same words.
#[test]
fn the_subjects_pid_one_cannot_serve_are_all_refused_by_name() {
    for command in [
        "get network",
        "get storage",
        "get memory",
        "get log",
        "get trust",
    ] {
        let mut k = FakeKernel::new();
        serve_line(&mut k, command);
        let said = answer(&k);
        assert!(
            said.contains("BREAK-GLASS cannot answer"),
            "{command} was not refused: {said}"
        );
    }
}

/// **Whoever is refused `apply uninstall` is refused `apply uninstall-library`
/// too.** Both are a request the supervisor performs, and this console exists
/// for the case where the supervisor is what failed — so the console refuses
/// both, on the plan and on the confirmed command, in the same words.
#[test]
fn the_console_refused_an_uninstall_is_refused_a_library_uninstall_too() {
    for command in [
        "apply uninstall photo-index",
        "apply uninstall photo-index confirm",
        "apply uninstall-library aura-background-pack",
        "apply uninstall-library aura-background-pack confirm",
    ] {
        let mut k = FakeKernel::new();
        assert_eq!(serve_line(&mut k, command), None, "{command}");
        let said = answer(&k);
        assert!(
            said.contains("BREAK-GLASS cannot answer"),
            "{command} was not refused: {said}"
        );
        assert!(said.contains("E_MACHINE_REFUSED"), "{command}: {said}");
    }
}

/// **A stop is recorded and reported before it happens.** The answer is written
/// first, so an operator sees the machine agree rather than watching it die
/// with nothing said.
#[test]
fn a_restart_is_answered_and_then_asked_for() {
    let mut k = FakeKernel::new();
    let pending = serve_line(&mut k, "apply restart confirm");
    assert!(
        matches!(pending, Some(crate::kernel::RebootHow::Restart)),
        "{pending:?}"
    );
    assert!(
        !k.lines.is_empty(),
        "the machine agreed to stop and said nothing"
    );
}

/// Without the confirmation there is no stop — the same rule the control plane
/// enforces, from the same code.
#[test]
fn a_restart_without_the_confirmation_stops_nothing() {
    let mut k = FakeKernel::new();
    assert_eq!(serve_line(&mut k, "apply restart"), None);
}

/// **JSON is refused with a sentence pointing at the word form.** Accepting
/// some MCP would be `ADR-005` §5.4.1's partial promise: it would invite a
/// client to assume the rest.
#[test]
fn a_json_line_is_refused_and_names_the_form_this_console_takes() {
    let mut k = FakeKernel::new();
    assert_eq!(serve_line(&mut k, "{\"jsonrpc\":\"2.0\"}"), None);
    let said = answer(&k);
    assert!(said.contains("WORDS"), "{said}");
    assert!(said.contains("cargo rivet run"), "{said}");
}

/// A word that is not a verb is refused by the catalogue's own parser, so the
/// console cannot disagree with the control plane about what a verb is.
#[test]
fn a_word_that_is_not_a_verb_is_refused_by_the_catalogue() {
    let mut k = FakeKernel::new();
    assert_eq!(serve_line(&mut k, "frobnicate everything"), None);
    assert!(!answer(&k).is_empty());
}

/// An empty line is not a command and must not produce an answer — a console
/// that replied to every stray newline would be unusable by a person.
#[test]
fn an_empty_line_says_nothing() {
    let mut k = FakeKernel::new();
    assert_eq!(serve_line(&mut k, "   "), None);
    assert!(k.lines.is_empty(), "{:?}", k.lines);
}
