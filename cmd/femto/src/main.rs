//! **`femto` — Rivet's editor.** `nano`-shaped, about a thousand lines, and
//! it refuses to edit a view.
//!
//! # The name
//!
//! `nano` descends from `pico`, and the SI prefixes run downward as the editors
//! get smaller: nano → pico → **femto**. This one is smaller than either, so
//! the name says *tiny editor of that family* with nothing to explain. `Fe` is
//! also iron, which is where Rivet's own name comes from.
//!
//! **It deliberately carries no `rivet-` prefix**, and that is a decision
//! rather than an inconsistency. The prefix means *Rivet's own machinery* —
//! `rivet`, `rivet-initd`, `rivet-sysd`, `rivet-netd`. This is a
//! general tool that happens to ship on the card, exactly as `nu` does
//! (`ADR-005` §5.1), and `nu` correctly wears no prefix either. There is a
//! separate, agent-facing `rivet apply edit` — structured, non-interactive,
//! plan-then-confirm — and *that* is Rivet machinery. Two things both called
//! "edit" would be a question an operator has to answer at 2am.
//!
//! # Why it exists
//!
//! `ADR-005` §5.4.2 took BusyBox out of the image, and §5.5 recorded what that
//! left behind:
//!
//! > **As the design stands, a Rivet system has no way to edit a text file at
//! > all.** … an editor is a convenience, not infrastructure — but it is the
//! > convenience whose absence is felt hardest at 2am, and a system that cannot
//! > be edited at all is hostile in a way that is difficult to argue is
//! > principled.
//!
//! §5.5 ruled *"a small Rust editor in the base image"*, chose size over
//! capability, named `kibi` as the archetype rather than the choice, and left
//! the specific project as an implementation-time decision under `ADR-009`'s
//! survey-first rule. This is that decision, and the argument for **writing**
//! rather than importing is `crate::view`: nothing off the shelf knows that on
//! this machine most files are a read-only projection, and an imported editor
//! would do the thing `ADR-007` §3.6 calls silent data loss.
//!
//! §5.5 also settled the naming rule this crate obeys: *"Whatever is chosen, it
//! is Rivet's editor and not `vi`."*
//!
//! # The shape
//!
//! ```text
//!   main            argv, the open, and the one place a platform is chosen
//!   run             the loop: draw, read a byte, decode, dispatch, journal
//!     host          the real terminal and the real filesystem   ─┐
//!     syscall       tcgetattr/tcsetattr/ioctl — every `unsafe`   │ platform
//!     port          the two traits, and the fakes                │
//!     journal       what a write tells the machine (ADR-022 §11)─┘
//!     key           bytes → keys                                ─┐
//!     editor        keys → acts                                  │ testable
//!     text          the buffer, the cursor, search               │ on any
//!     screen        state → a frame of styled spans              │ machine
//!     view          which paths must not be written             ─┘
//! ```
//!
//! Everything in the lower group is driven by `cargo test` on the Windows
//! development host with no terminal involved, which is `AGENTS.md` §4's
//! standing preference and is the reason the seam exists at all.
//!
//! # What it is not
//!
//! It is not `helix`, which `ADR-005` §5.5 keeps out of the base image as a
//! system extension (`ADR-016`) because it is large and brings its own
//! configuration surface — *"a faintly absurd thing to ship permanently in a
//! project whose central argument is about configuration sprawl."* `femto` has
//! no configuration file, and adding one would be the moment to reread that
//! sentence.

mod editor;
mod host;
mod journal;
mod key;
mod port;
mod screen;
mod syscall;
mod text;
mod view;

use std::io::IsTerminal as _;
use std::process::ExitCode;

use editor::{Editor, Step};
use journal::{Journal, Unfiled};
use key::{Decoded, Decoder};
use port::{Files, Terminal};
use text::Text;

/// What `femto` exits with.
///
/// Small and conventional rather than invented: `ADR-006` §4.1's coded refusal
/// surface belongs to `rivet`, which is an administration interface with
/// agents reading its output. This is a program a human runs interactively;
/// what an exit status has to distinguish is *the file was not opened* from
/// *the editor ran*.
const OK: u8 = 0;
const COULD_NOT_OPEN: u8 = 1;
const USAGE: u8 = 2;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    ExitCode::from(run_from_argv(&args))
}

fn run_from_argv(args: &[String]) -> u8 {
    let mut path: Option<&str> = None;
    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("{}", usage());
                return OK;
            }
            "--version" | "-V" => {
                println!("femto {}", env!("CARGO_PKG_VERSION"));
                return OK;
            }
            other if other.starts_with('-') && other != "-" => {
                eprintln!("femto: unknown option {other}\n\n{}", usage());
                return USAGE;
            }
            other if path.is_none() => path = Some(other),
            other => {
                // One file, deliberately. `nano` opens several into buffers and
                // has a key to move between them; this does not, and refusing
                // is better than opening the first and ignoring the rest.
                eprintln!("femto: one file at a time, and {other} is a second\n\n{}", usage());
                return USAGE;
            }
        }
    }

    let Some(path) = path else {
        eprintln!("{}", usage());
        return USAGE;
    };

    let mut files = host::HostFiles;
    let editor = match open(path, &mut files) {
        Ok(editor) => editor,
        Err(message) => {
            eprintln!("femto: {message}");
            return COULD_NOT_OPEN;
        }
    };

    // Refuse rather than draw a screen nobody can see. Piped input would also
    // mean no raw mode, so every key would arrive a line at a time.
    if !std::io::stdin().is_terminal() {
        eprintln!("femto: standard input is not a terminal, so there is nothing to edit with.");
        return COULD_NOT_OPEN;
    }

    let mut terminal = match host::terminal() {
        Ok(terminal) => terminal,
        Err(error) => {
            eprintln!("femto: the terminal could not be put into raw mode: {error}");
            return COULD_NOT_OPEN;
        }
    };

    let mut editor = editor;
    let mut filing = journal::HostJournal;
    // **Owned here rather than returned**, so that a terminal which fails
    // half way through a session does not take the records with it. A run that
    // wrote two files, could not reach the journal, and then lost its pty would
    // otherwise lose exactly the account `ADR-022` §11.5 exists to keep.
    let mut unfiled: Vec<Unfiled> = Vec::new();
    let outcome = run(&mut editor, &mut terminal, &mut files, &mut filing, &mut unfiled);
    // **After the terminal, not before it.** `ADR-022` §11.5's floor is stderr,
    // and this program's stderr is an alternate screen in raw mode until
    // `terminal` is dropped — a record written there would paint over the buffer
    // the operator is editing and then be discarded with the screen.
    drop(terminal);
    report(&unfiled);
    match outcome {
        Ok(()) => OK,
        Err(error) => {
            eprintln!("femto: {error}");
            COULD_NOT_OPEN
        }
    }
}

/// Put the records that did not reach the journal on stderr — `ADR-022` §11.5.
///
/// **The line itself**, so an operator can paste it back and an agent reading
/// the session gets the same structure `rivet get log` would have given it.
/// A summary would be the half of the answer that cannot be acted on.
fn report(unfiled: &[Unfiled]) {
    let Some(first) = unfiled.first() else {
        return;
    };
    eprintln!(
        "femto: {n} record{s} did NOT reach this machine's journal, and here {they} are.\n\
         femto: `rivet get log` will not show {these}. Reason: {why}",
        n = unfiled.len(),
        s = if unfiled.len() == 1 { "" } else { "s" },
        they = if unfiled.len() == 1 { "it" } else { "they" },
        these = if unfiled.len() == 1 {
            "this write"
        } else {
            "these writes"
        },
        why = first.why,
    );
    for held in unfiled {
        eprint!("{}", held.line);
        if !held.line.ends_with('\n') {
            eprintln!();
        }
    }
}

/// Read the file and classify the path, in that order.
///
/// **Both halves refuse rather than guess** (`AGENTS.md` §5): a file that is
/// not text is not opened, and a path that cannot be read for any reason other
/// than not existing is an error carrying what the kernel said.
fn open(path: &str, files: &mut dyn Files) -> Result<Editor, String> {
    let opening = view::classify(path);
    let text = match files.read(path) {
        Ok(Some(bytes)) => Text::from_bytes(&bytes).map_err(|why| format!("{path}: {why}"))?,
        // A name that is not there yet is a new file, which is how one gets
        // created — unless the path is one that could never be written, in
        // which case offering to create it would be a promise this program
        // cannot keep.
        Ok(None) => match &opening {
            view::Opening::ReadOnly(refusal) => {
                return Err(format!(
                    "{path} does not exist, and could not be created: {}",
                    refusal.headline
                ));
            }
            view::Opening::Editable => Text::default(),
        },
        Err(error) => return Err(format!("{path}: {error}")),
    };
    Ok(Editor::open(path.to_owned(), text, opening))
}

/// The loop.
///
/// **Draw, then read.** In that order, so the first frame is on screen before
/// anything has been typed — and so a scripted terminal in a test can assert on
/// what the operator would have been looking at when they pressed the key.
///
/// It ends on `^X`, or when the terminal has no more input. The second is not a
/// theoretical case: it is what a test's script does when it runs out, and it
/// is why `crate::port::testing::ScriptedTerminal` can drive the whole program
/// without pressing `^X` at all.
fn run(
    editor: &mut Editor,
    terminal: &mut dyn Terminal,
    files: &mut dyn Files,
    journal: &mut dyn Journal,
    unfiled: &mut Vec<Unfiled>,
) -> std::io::Result<()> {
    let mut decoder = Decoder::default();
    loop {
        let size = terminal.size();
        editor.scroll(size);
        terminal.present(&screen::render(editor, size))?;

        let Some(byte) = terminal.read_byte()? else {
            return Ok(());
        };
        let step = match decoder.feed(byte) {
            Decoded::Nothing => Step::Continue,
            Decoded::One(key) => editor.key(key, size, files),
            Decoded::Two(first, second) => match editor.key(first, size, files) {
                Step::Quit => Step::Quit,
                Step::Continue => editor.key(second, size, files),
            },
        };
        // **`ADR-022` §11, and it is here rather than inside the editor.** A
        // record goes to a socket, which is a seam `crate::editor` does not
        // have and should not grow to serve one keystroke in a thousand; this
        // is the composition root, where the machine already is. `take_wrote`
        // is empty on every key that is not a write.
        if let Some(wrote) = editor.take_wrote()
            && let Some(held) = journal::file(journal, &wrote)
        {
            editor.note_unjournalled();
            unfiled.push(held);
        }
        if step == Step::Quit {
            return Ok(());
        }
    }
}

fn usage() -> String {
    format!(
        "femto {} — a tiny editor for Rivet.\n\
         \n\
         usage: femto <file>\n\
         \n\
         Arrow keys move, typing inserts, and the two lines at the bottom of the\n\
         screen are the keys. ^O writes, ^X exits, ^G explains.\n\
         \n\
         Files under /etc are a read-only view of state owned elsewhere\n\
         (ADR-007 §3.6.2). femto opens them, refuses to write them, and says what\n\
         to change instead. `rivet explain <path>` is the same answer as a\n\
         structured value.",
        env!("CARGO_PKG_VERSION")
    )
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use journal::testing::MemoryJournal;
    use port::testing::{MemoryFiles, ScriptedTerminal};

    /// Drive the whole program the way an operator would: a script of bytes in,
    /// frames and a filesystem out.
    ///
    /// **This is the closest an automated test gets to the thing running.** It
    /// is the whole loop — decode, dispatch, render — over a terminal that is
    /// scripted rather than absent, and it still proves nothing about how it
    /// *looks*. See the report accompanying this crate for what was driven by
    /// hand and what is still unproven.
    fn drive(path: &str, on_disk: &str, script: &[u8]) -> (ScriptedTerminal, MemoryFiles) {
        let (terminal, files, _, _) = drive_on(path, on_disk, script, MemoryJournal::default());
        (terminal, files)
    }

    /// The same, over a journal the caller chose — a machine whose supervisor is
    /// listening, or one whose is not.
    ///
    /// It hands back the journal and the held records as well, because
    /// `ADR-022` §11's whole claim is about what happens to a record on both of
    /// those machines.
    fn drive_on(
        path: &str,
        on_disk: &str,
        script: &[u8],
        journal: MemoryJournal,
    ) -> (ScriptedTerminal, MemoryFiles, MemoryJournal, Vec<Unfiled>) {
        let mut files = MemoryFiles::with(path, on_disk);
        let mut terminal = ScriptedTerminal::typing(script);
        let mut journal = journal;
        let mut editor = open(path, &mut files).expect("the fixture opens");
        let mut unfiled = Vec::new();
        run(
            &mut editor,
            &mut terminal,
            &mut files,
            &mut journal,
            &mut unfiled,
        )
        .expect("the scripted terminal never fails");
        (terminal, files, journal, unfiled)
    }

    /// `^O`, then Return, then `^X`. The three keys that make an editor an
    /// editor.
    #[test]
    fn a_file_can_be_opened_typed_into_saved_and_left() {
        let (_, files) = drive(
            "/state/notes.txt",
            "second\n",
            b"first line\r\x0f\r\x18",
        );
        assert_eq!(
            files.text("/state/notes.txt").as_deref(),
            Some("first line\nsecond\n"),
            "the bytes on disk are not what was typed"
        );
    }

    #[test]
    fn quitting_without_saving_leaves_the_file_alone() {
        // Type, then `^X`, then `n` for "no, do not save".
        let (_, files) = drive("/state/notes.txt", "original\n", b"typed\x18n");
        assert_eq!(files.text("/state/notes.txt").as_deref(), Some("original\n"));
        assert!(files.attempts.is_empty(), "nothing should have been written");
    }

    #[test]
    fn arrow_keys_move_before_the_typing_lands() {
        // Down, End, then type — so the text goes at the end of line 2.
        let (_, files) = drive(
            "/state/x",
            "one\ntwo\n",
            b"\x1b[B\x1b[F!\x0f\r\x18",
        );
        assert_eq!(files.text("/state/x").as_deref(), Some("one\ntwo!\n"));
    }

    #[test]
    fn cut_and_uncut_move_a_line_through_the_whole_program() {
        // `^K` on line 1, Down, `^U`, write, exit.
        let (_, files) = drive("/state/x", "one\ntwo\nthree\n", b"\x0b\x1b[B\x15\x0f\r\x18");
        assert_eq!(files.text("/state/x").as_deref(), Some("two\none\nthree\n"));
    }

    #[test]
    fn search_moves_the_cursor_and_the_next_keystroke_lands_there() {
        // `^W` three <Return>, then type — the insert must land at the match.
        let (_, files) = drive(
            "/state/x",
            "alpha\nbeta\ngamma\n",
            b"\x17gam\r!\x0f\r\x18",
        );
        assert_eq!(files.text("/state/x").as_deref(), Some("alpha\nbeta\n!gamma\n"));
    }

    /// The property `crate::view` exists for, driven end to end rather than
    /// asserted on a function: a `/etc` path opens, is not writable, and the
    /// screen says what to change instead.
    #[test]
    fn a_view_opens_read_only_and_the_screen_says_what_to_change() {
        let (terminal, files) = drive(
            "/etc/hosts",
            "127.0.0.1 localhost\n",
            // Type into it, then try to save it, then leave.
            b"broken\x0f\r\x18",
        );
        assert_eq!(
            files.text("/etc/hosts").as_deref(),
            Some("127.0.0.1 localhost\n"),
            "a view was written"
        );
        assert!(
            files.attempts.is_empty(),
            "the refusal must come before the syscall, not from EROFS afterwards"
        );
        let seen = terminal.everything_seen();
        assert!(seen.contains("VIEW"), "the title bar never said so");
        assert!(seen.contains("loopback"), "the refusal never named the change-with");
    }

    #[test]
    fn a_file_that_is_not_text_is_refused_rather_than_opened() {
        let mut files = MemoryFiles::default();
        files.files.insert("/state/blob".to_owned(), vec![0x7f, 0x45, 0x4c, 0x46, 0x00]);
        let error = open("/state/blob", &mut files).expect_err("a binary must not open");
        assert!(error.contains("NUL"), "{error}");
    }

    #[test]
    fn a_name_that_does_not_exist_yet_opens_as_an_empty_new_file() {
        let mut files = MemoryFiles::default();
        let editor = open("/state/new.txt", &mut files).expect("a new file opens");
        assert_eq!(editor.text().line_count(), 1);
        assert!(!editor.text().is_dirty());
    }

    /// Offering to create `/etc/nginx.conf` would be a promise this program
    /// cannot keep: the mount is read-only, so the write at the end would fail
    /// after the operator had typed the file.
    #[test]
    fn a_name_that_does_not_exist_under_a_read_only_path_is_refused_at_the_open() {
        let mut files = MemoryFiles::default();
        let error = open("/etc/nginx.conf", &mut files).expect_err("this cannot be created");
        assert!(error.contains("could not be created"), "{error}");
    }

    #[test]
    fn the_first_frame_is_drawn_before_a_key_is_read() {
        let (terminal, _) = drive("/state/x", "hello\n", b"");
        assert_eq!(
            terminal.frames.len(),
            1,
            "an editor that read before it drew would show a blank screen until the first key"
        );
        assert!(terminal.last().text().contains("hello"));
    }

    #[test]
    fn no_argument_is_a_usage_error_and_says_what_to_type() {
        assert_eq!(run_from_argv(&[]), USAGE);
        assert_eq!(run_from_argv(&["--version".to_owned()]), OK);
        assert_eq!(run_from_argv(&["--nonsense".to_owned()]), USAGE);
        assert_eq!(
            run_from_argv(&["a".to_owned(), "b".to_owned()]),
            USAGE,
            "two files must be refused rather than one silently ignored"
        );
    }

    #[test]
    fn the_usage_text_names_the_keys_a_nano_user_would_reach_for() {
        let usage = usage();
        for key in ["^O", "^X", "^G"] {
            assert!(usage.contains(key), "{usage}");
        }
    }

    // -- journalling (`ADR-022` §11) ----------------------------------------

    use rivet_core::journal::Record;

    /// **"Who changed this file, and when" now has an answer for the
    /// interactive editor.** Driven the way an operator would: type, `^O`,
    /// Return, `^X` — and a record comes out the other end that
    /// `rivet get log`'s own parser reads.
    #[test]
    fn a_write_puts_one_record_in_the_journal() {
        let (_, files, journal, unfiled) = drive_on(
            "/state/notes.txt",
            "second\n",
            b"first line\r\x0f\r\x18",
            MemoryJournal::default(),
        );
        assert_eq!(
            files.text("/state/notes.txt").as_deref(),
            Some("first line\nsecond\n")
        );
        assert!(unfiled.is_empty(), "{unfiled:?}");
        let filed = journal.filed();
        assert_eq!(filed.len(), 1, "{filed:?}");
        let record = Record::from_jsonl(&filed[0])
            .unwrap_or_else(|e| panic!("rivet get log could not read {:?}: {e}", filed[0]));
        assert_eq!(record.svc, "femto");
        assert_eq!(
            record.field("path"),
            Some(&rivet_core::journal::Value::Str(
                "/state/notes.txt".to_owned()
            ))
        );
        assert_eq!(
            record.field("after"),
            Some(&rivet_core::journal::Value::Int(18))
        );
    }

    /// **A write that did not happen is not journalled**, in both of the ways
    /// this program refuses one: quitting without saving, and a view.
    #[test]
    fn nothing_is_journalled_when_nothing_was_written() {
        for (path, script) in [
            // Type, `^X`, `n` — "no, do not save".
            ("/state/notes.txt", &b"typed\x18n"[..]),
            // A view: typed into, `^O`, Return — refused before the syscall.
            ("/etc/hosts", &b"broken\x0f\r\x18"[..]),
        ] {
            let (_, _, journal, unfiled) =
                drive_on(path, "original\n", script, MemoryJournal::default());
            assert!(journal.sent.is_empty(), "{path}: {:?}", journal.sent);
            assert!(unfiled.is_empty(), "{path}: {unfiled:?}");
        }
    }

    /// A write that the filesystem refused is not journalled either — the
    /// record says what happened, and nothing happened.
    #[test]
    fn a_refused_write_is_not_journalled() {
        let mut files = MemoryFiles::with("/state/x", "a\n");
        files.read_only.push("/state/x".to_owned());
        let mut terminal = ScriptedTerminal::typing(b"typed\x0f\r\x18");
        let mut journal = MemoryJournal::default();
        let mut editor = open("/state/x", &mut files).expect("the fixture opens");
        let mut unfiled = Vec::new();
        run(
            &mut editor,
            &mut terminal,
            &mut files,
            &mut journal,
            &mut unfiled,
        )
        .expect("the scripted terminal never fails");
        assert_eq!(files.attempts, ["/state/x"], "it never tried to write");
        assert!(journal.sent.is_empty(), "{:?}", journal.sent);
        assert!(unfiled.is_empty(), "{unfiled:?}");
    }

    /// **The floor.** On a machine whose supervisor is not listening the record
    /// is held whole — so `crate::report` can put it on stderr once the terminal
    /// is the operator's again — and the operator is told at the moment of the
    /// write rather than only on the way out.
    #[test]
    fn a_machine_with_no_supervisor_keeps_the_record_and_says_so_on_the_status_line() {
        let (terminal, files, journal, unfiled) = drive_on(
            "/state/notes.txt",
            "second\n",
            b"first line\r\x0f\r\x18",
            MemoryJournal::refusing(),
        );
        // The file was still written. That is the point: an unreachable journal
        // is not a reason to refuse an edit.
        assert_eq!(
            files.text("/state/notes.txt").as_deref(),
            Some("first line\nsecond\n")
        );
        assert!(journal.filed().is_empty());
        assert_eq!(unfiled.len(), 1, "{unfiled:?}");
        Record::from_jsonl(&unfiled[0].line).expect("what is held is still a record");
        assert!(
            terminal.everything_seen().contains("NOT journalled"),
            "the operator was never told: {}",
            terminal.everything_seen()
        );
    }

    /// Two writes are two records, and neither is filed twice — `take_wrote`
    /// empties, so a keystroke that is not a write cannot re-file the one
    /// before it.
    #[test]
    fn two_writes_are_two_records_and_no_write_is_filed_twice() {
        let (_, _, journal, _) = drive_on(
            "/state/notes.txt",
            "a\n",
            // `^O` Return, type, `^O` Return, then several keys that are not
            // writes, then `^X`.
            b"\x0f\rmore\x0f\r\x1b[B\x1b[A\x18",
            MemoryJournal::default(),
        );
        assert_eq!(journal.filed().len(), 2, "{:?}", journal.filed());
    }

    /// **A `^O` to a different name records both**, because "which buffer was
    /// this" is the question a reader of the journal actually has.
    #[test]
    fn writing_under_a_new_name_records_the_name_it_was_opened_under() {
        let mut files = MemoryFiles::with("/etc/hosts", "127.0.0.1 localhost\n");
        // `^O` fills the prompt with the current path, as `nano` does, so the
        // script deletes it a character at a time before typing the new one.
        let mut script = vec![0x0f];
        script.extend(std::iter::repeat_n(0x7f, "/etc/hosts".len()));
        script.extend_from_slice(b"/state/copy.txt\r\x18");
        let mut terminal = ScriptedTerminal::typing(&script);
        let mut journal = MemoryJournal::default();
        let mut editor = open("/etc/hosts", &mut files).expect("the fixture opens");
        run(
            &mut editor,
            &mut terminal,
            &mut files,
            &mut journal,
            &mut Vec::new(),
        )
        .expect("the scripted terminal never fails");
        let filed = journal.filed();
        assert_eq!(filed.len(), 1, "{filed:?}");
        let record = Record::from_jsonl(&filed[0]).expect("a record");
        assert_eq!(
            record.field("path"),
            Some(&rivet_core::journal::Value::Str(
                "/state/copy.txt".to_owned()
            ))
        );
        assert_eq!(
            record.field("opened_as"),
            Some(&rivet_core::journal::Value::Str("/etc/hosts".to_owned()))
        );
    }
}
