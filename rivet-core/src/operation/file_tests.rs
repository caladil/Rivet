//! `rivet get file`'s tests. In their own file for `super::memory`'s reason:
//! the subject is short and its refusals are not, and a module whose tests are
//! four times its code is one nobody reads either half of.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::filesystem::fake::{FakeFiles, Node};

fn get(fake: &mut FakeFiles, path: &str) -> Value {
    perform(&[SUBJECT, path], fake).expect("this path must answer")
}

fn refusal(fake: &mut FakeFiles, words: &[&str]) -> OperationError {
    perform(words, fake).expect_err("that command must be refused")
}

/// `get file <path> <offset> [length]` — the window added 2026-09-21.
fn window(fake: &mut FakeFiles, path: &str, at: usize, want: Option<usize>) -> Value {
    let at = at.to_string();
    let mut words = vec![SUBJECT, path, at.as_str()];
    let want = want.map(|n| n.to_string());
    if let Some(want) = &want {
        words.push(want.as_str());
    }
    perform(&words, fake).expect("this path must answer")
}

/// One field of an object answer, or `None`.
fn field<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    match value {
        Value::Object(fields) => fields.iter().find(|(key, _)| *key == name).map(|(_, v)| v),
        _ => panic!("the answer is not an object"),
    }
}

fn text(value: &Value, name: &str) -> String {
    match field(value, name) {
        Some(Value::Str(s)) => s.clone(),
        other => panic!("{name} is {other:?}"),
    }
}

fn flag(value: &Value, name: &str) -> bool {
    match field(value, name) {
        Some(Value::Bool(b)) => *b,
        other => panic!("{name} is {other:?}"),
    }
}

// -- the ordinary answer -----------------------------------------------------

/// **The gap this closes, in one test**: a file `apply edit` could change is a
/// file this can read back.
#[test]
fn a_regular_file_answers_with_its_contents_and_its_meta_information() {
    let mut fake = FakeFiles::holding("/state/notes", "one\ntwo\n");
    let answer = get(&mut fake, "/state/notes");

    assert!(flag(&answer, "present"));
    assert_eq!(text(&answer, "type"), "file");
    assert_eq!(field(&answer, "size_bytes"), Some(&Value::Integer(8)));
    assert_eq!(text(&answer, "mode"), "0644");
    assert_eq!(text(&answer, "mode_rendered"), "rw-r--r--");
    assert_eq!(field(&answer, "uid"), Some(&Value::Integer(0)));
    assert_eq!(text(&answer, "user"), "root");
    assert_eq!(text(&answer, "content"), "one\ntwo\n");
    assert_eq!(field(&answer, "content_omitted"), None);
    assert!(flag(&answer, "changeable"));
}

/// A path with nothing at it is `Ok`, and the answer says so in a field rather
/// than by omitting everything — `super::document`'s precedent.
#[test]
fn nothing_at_a_path_is_an_answer_and_not_an_error() {
    let mut fake = FakeFiles::new();
    let answer = get(&mut fake, "/state/nothing-here");

    assert!(!flag(&answer, "present"));
    assert_eq!(field(&answer, "type"), None);
    assert_eq!(field(&answer, "size_bytes"), None);
    assert_eq!(field(&answer, "mode"), None);
    assert!(text(&answer, "next").contains("NOTHING IS AT"));
    // The question worth asking about a path that is not there.
    assert!(flag(&answer, "changeable"));
}

#[test]
fn a_directory_answers_with_its_entries_sorted_and_counted() {
    let mut fake = FakeFiles::new()
        .with("/state/things", Node::directory())
        .with("/state/things/b", Node::file("bb"))
        .with("/state/things/a", Node::file("a"))
        .with("/state/things/sub", Node::directory());
    let answer = get(&mut fake, "/state/things");

    assert_eq!(text(&answer, "type"), "directory");
    assert_eq!(field(&answer, "entry_count"), Some(&Value::Integer(3)));
    assert!(!flag(&answer, "entries_truncated"));
    let Some(Value::List(entries)) = field(&answer, "entries") else {
        panic!("no entries");
    };
    assert_eq!(entries.len(), 3);
    assert_eq!(text(&entries[0], "name"), "a");
    assert_eq!(text(&entries[1], "name"), "b");
    assert_eq!(text(&entries[2], "name"), "sub");
    assert_eq!(text(&entries[2], "type"), "directory");
    // A directory has no contents, and the answer says which of the reasons
    // applies rather than leaving `content` silently absent.
    assert!(text(&answer, "content_omitted").contains("not a regular file"));
}

/// **A link is reported as a link and never followed.** `ADR-006` §8.10.4.3.
#[test]
fn a_symbolic_link_reports_its_target_and_is_not_followed() {
    let mut fake = FakeFiles::new()
        .with("/state/link", Node::symlink("/system/bin/rivet"))
        .with("/system/bin/rivet", Node::file("ELF"));
    let answer = get(&mut fake, "/state/link");

    assert_eq!(text(&answer, "type"), "symlink");
    assert_eq!(text(&answer, "link_target"), "/system/bin/rivet");
    assert_eq!(field(&answer, "content"), None);
    assert!(text(&answer, "next").contains("NOT FOLLOWED"));
}

// -- content is present or explained, never lossy ----------------------------

/// **Never a guess about an encoding** — `AGENTS.md` §5 — and since
/// 2026-09-01 that is served by ENCODING the bytes rather than by refusing
/// them, ruled by the maintainer (`ADR-006` §8.10).
///
/// ~~A file that is not UTF-8 has its content omitted, and the answer names the
/// offset of the first byte that is not.~~ **Struck.** The rule was never
/// "refuse binary", it was "do not guess": replacement characters are a guess
/// that does not fail, and base64 is neither a guess nor mistakable for text. A
/// Rivet now produces binary an operator legitimately reads off the machine —
/// a captured frame is the case that forced it — and there was no way to get
/// one off the board.
#[test]
fn a_file_that_is_not_utf8_comes_back_base64_and_says_so() {
    let mut fake = FakeFiles::new().with("/state/blob", Node::file([0x41, 0x42, 0xff, 0x43]));
    let answer = get(&mut fake, "/state/blob");

    // `AB\xffC` is `QUL/Qw==`. Written out rather than computed, so this test
    // does not agree with the encoder by construction.
    assert_eq!(text(&answer, "content"), "QUL/Qw==");
    assert_eq!(text(&answer, "content_encoding"), "base64");
    assert_eq!(field(&answer, "content_omitted"), None);
    assert_eq!(field(&answer, "size_bytes"), Some(&Value::Integer(4)));
}

/// The other half, and the one a reader is likelier to assume: text is still
/// text, verbatim, and says which it is. A build that base64'd everything would
/// pass the test above.
#[test]
fn a_text_file_comes_back_verbatim_and_says_it_is_text() {
    let mut fake = FakeFiles::new().with("/state/note", Node::file(b"hello\n"));
    let answer = get(&mut fake, "/state/note");

    assert_eq!(text(&answer, "content"), "hello\n");
    assert_eq!(text(&answer, "content_encoding"), "text");
    assert_eq!(field(&answer, "content_omitted"), None);
}

/// **A file larger than one answer is READ IN PIECES, not refused** — changed
/// 2026-09-21.
///
/// It used to answer `content_omitted` and nothing else, and the sentence
/// said *"this operation reads a file whole"*. That was the cap doing the
/// wrong job: it bounds what one ANSWER may carry, which is a memory question
/// on a board with no swap, and it was being used to bound what a FILE may be.
///
/// The defect it produced was `cargo rivet screenshot` failing on a busy
/// screen — `rivet-01` drawing Contours made a 1,090,017-byte QOI, 41 KB over,
/// and the tool whose whole job is to fetch that file could not.
#[test]
fn a_file_larger_than_one_answer_comes_back_in_pieces_and_says_there_is_more() {
    let size = super::super::edit::FILE_MAX_BYTES + 1;
    let big = vec![b'x'; size];
    let mut fake = FakeFiles::new().with("/state/big", Node::file(&big));

    let first = get(&mut fake, "/state/big");
    assert_eq!(field(&first, "content_omitted"), None, "it was refused");
    assert_eq!(
        text(&first, "content").len(),
        super::super::edit::FILE_MAX_BYTES,
        "the first answer should carry exactly what one answer may"
    );
    assert_eq!(field(&first, "content_offset"), Some(&Value::Integer(0)));
    assert_eq!(
        field(&first, "content_bytes"),
        Some(&Value::Integer(
            i64::try_from(super::super::edit::FILE_MAX_BYTES).unwrap()
        ))
    );
    assert_eq!(
        field(&first, "content_complete"),
        Some(&Value::Bool(false)),
        "a file with a byte left must not claim to be complete"
    );
    assert_eq!(
        field(&first, "size_bytes"),
        Some(&Value::Integer(i64::try_from(size).unwrap()))
    );

    // The second window: the one remaining byte, and it ends the file.
    let rest = window(&mut fake, "/state/big", super::super::edit::FILE_MAX_BYTES, None);
    assert_eq!(text(&rest, "content"), "x");
    assert_eq!(field(&rest, "content_complete"), Some(&Value::Bool(true)));
}

/// **FS4: the same file, and it is never read WHOLE to answer either
/// window.** `content()` used to call `Editor::read_file` — the whole-file
/// reader — and slice the cap out of what came back, which is the exact
/// allocation this operation exists to let a caller avoid. It now calls
/// `Editor::read_file_window`, and `FakeFiles` records every call to the
/// whole-file reader so this is provable rather than inferred from the
/// answer alone: the bytes and the length above are already proof the VALUE
/// did not change; `whole_reads` is proof the PATH did.
#[test]
fn a_file_larger_than_the_cap_is_never_read_whole() {
    let size = super::super::edit::FILE_MAX_BYTES + 1;
    let big = vec![b'x'; size];
    let mut fake = FakeFiles::new().with("/state/big", Node::file(&big));

    let first = get(&mut fake, "/state/big");
    assert_eq!(
        text(&first, "content").len(),
        super::super::edit::FILE_MAX_BYTES
    );
    assert_eq!(field(&first, "content_complete"), Some(&Value::Bool(false)));

    let rest = window(&mut fake, "/state/big", super::super::edit::FILE_MAX_BYTES, None);
    assert_eq!(text(&rest, "content"), "x");

    assert!(
        fake.whole_reads.is_empty(),
        "the whole-file reader was called: {:?}",
        fake.whole_reads
    );
}

/// **A window may split a UTF-8 character**, and the piece comes back base64
/// rather than as a refusal or a replacement character.
///
/// A caller reassembling windows concatenates the BYTES and decodes once at
/// the end; `AGENTS.md` §5's rule is that a guess must not silently succeed,
/// and base64 is not a guess.
#[test]
fn a_window_that_splits_a_character_answers_base64_rather_than_guessing() {
    let mut fake = FakeFiles::new().with("/state/snow", Node::file("héllo".as_bytes()));
    // `é` is two bytes at offset 1; take only the first of them.
    let cut = window(&mut fake, "/state/snow", 0, Some(2));
    assert_eq!(text(&cut, "content_encoding"), "base64");
    assert_eq!(field(&cut, "content_complete"), Some(&Value::Bool(false)));

    let whole = get(&mut fake, "/state/snow");
    assert_eq!(text(&whole, "content_encoding"), "text");
    assert_eq!(text(&whole, "content"), "héllo");
    assert_eq!(field(&whole, "content_complete"), Some(&Value::Bool(true)));
}

/// An offset past the end is a refusal that says what to compare it against,
/// and a length of zero on a file with bytes left is refused rather than
/// answering an empty window for ever.
#[test]
fn a_window_that_asks_for_nothing_readable_says_so_rather_than_looping() {
    let mut fake = FakeFiles::new().with("/state/five", Node::file(b"12345"));

    let past = window(&mut fake, "/state/five", 99, None);
    assert!(text(&past, "content_omitted").contains("past its end"));

    let nothing = window(&mut fake, "/state/five", 0, Some(0));
    assert!(text(&nothing, "content_omitted").contains("length of zero"));

    let at_end = window(&mut fake, "/state/five", 5, None);
    assert_eq!(text(&at_end, "content"), "");
    assert_eq!(field(&at_end, "content_complete"), Some(&Value::Bool(true)));
}

/// A window's two numbers are refused rather than read as zero, because a
/// caller who wrote `1e6` meant a number and would otherwise silently get the
/// first piece of the file back.
#[test]
fn a_window_whose_numbers_are_not_numbers_is_refused_by_name() {
    let mut fake = FakeFiles::new().with("/state/five", Node::file(b"12345"));

    let offset = refusal(&mut fake, &[SUBJECT, "/state/five", "1e6"]);
    assert_eq!(offset.code(), ErrorCode::Arguments);
    assert!(offset.message().contains("third word"));

    let length = refusal(&mut fake, &[SUBJECT, "/state/five", "0", "1_024"]);
    assert_eq!(length.code(), ErrorCode::Arguments);
    assert!(length.message().contains("fourth word"));
}

/// An empty file is a file, and `content` is the empty string rather than
/// absent — the distinction `FileRead::contents` draws and this must not lose.
#[test]
fn an_empty_file_has_content_and_is_not_confused_with_an_absent_one() {
    let mut fake = FakeFiles::holding("/state/empty", "");
    let answer = get(&mut fake, "/state/empty");

    assert!(flag(&answer, "present"));
    assert_eq!(text(&answer, "content"), "");
    assert_eq!(field(&answer, "content_omitted"), None);
}

// -- it answers about writing, without writing -------------------------------

/// **The caller should not have to attempt a change to learn it is refused.**
#[test]
fn a_path_the_writing_subjects_refuse_says_so_and_says_why() {
    let mut fake = FakeFiles::new().with("/state/config/machine.toml", Node::file("[machine]\n"));
    let answer = get(&mut fake, "/state/config/machine.toml");

    assert!(flag(&answer, "present"));
    assert_eq!(text(&answer, "content"), "[machine]\n");
    assert!(!flag(&answer, "changeable"));
    assert!(text(&answer, "unchangeable_because").contains("LOAD-BEARING"));
}

/// **Reading outside `/state` is not refused**, which is `ADR-006` §8.10.4's
/// shorter gate for reads — and the answer still says a write would be.
#[test]
fn a_path_outside_the_permitted_root_reads_and_says_it_cannot_be_changed() {
    let mut fake = FakeFiles::new().with("/system/bin/rivet-sysd", Node::file("ELF"));
    let answer = get(&mut fake, "/system/bin/rivet-sysd");

    assert!(flag(&answer, "present"));
    assert_eq!(text(&answer, "content"), "ELF");
    assert!(!flag(&answer, "changeable"));
    assert!(text(&answer, "unchangeable_because").contains("/state"));
}

/// A view is READ, not refused, and the answer carries `explain`'s own words.
#[test]
fn a_generated_path_is_read_and_reported_as_generated_rather_than_refused() {
    let mut fake = FakeFiles::new().with(crate::etc::PASSWD, Node::file("root:x:0:0:::\n"));
    let answer = get(&mut fake, crate::etc::PASSWD);

    assert!(flag(&answer, "present"));
    assert!(text(&answer, "content").starts_with("root:"));
    let generated = field(&answer, "generated").expect("a view says it is one");
    assert_eq!(field(generated, "writable"), Some(&Value::Bool(false)));
    assert!(!text(generated, "change_with").is_empty());
}

/// **A link on the way to a path is reported as the refusal it will be**, so a
/// caller does not discover it by attempting a write. `ADR-006` §8.10.4.3.
#[test]
fn a_symbolic_link_in_the_path_is_reported_and_makes_the_path_unchangeable() {
    let mut fake = FakeFiles::new()
        .with("/state/scratch", Node::symlink("/system"))
        .with("/state/scratch/thing", Node::file("x"));
    let answer = get(&mut fake, "/state/scratch/thing");

    assert_eq!(text(&answer, "symlinked_ancestor"), "/state/scratch");
    assert!(!flag(&answer, "changeable"));
    assert!(text(&answer, "unchangeable_because").contains("SYMBOLIC LINK"));
}

// -- the application tree ----------------------------------------------------

/// `ADR-006` §8.10.4.4: permit, and inform. Each zone says the thing an
/// operator would otherwise learn weeks later.
#[test]
fn each_zone_of_the_application_tree_says_what_follows_from_being_in_it() {
    let cases = [
        ("/state/app/thing/0.2.0/bin/thing", "program", "EVERY LAUNCH"),
        (
            "/state/app/thing/0.2.0/config.toml",
            "version",
            "NEXT UPDATE DISCARDS IT",
        ),
        ("/state/app/thing/data/store.db", "data", "survives an update"),
        ("/state/app/thing/current", "mechanism", ""),
        // **The supervisor's three siblings of `data/`**, beside the version
        // directories: the settings view and the routing table survive an
        // update and are rewritten by the supervisor; scratch is emptied.
        ("/state/app/thing/settings", "settings", "whenever it delivers settings"),
        ("/state/app/thing/routes", "routes", "the next install or uninstall rewrites it"),
        ("/state/app/thing/tmp/restore.part", "scratch", "nothing written here persists"),
        // A file of that name INSIDE a version directory is still the version's.
        ("/state/app/thing/0.2.0/settings", "version", "NEXT UPDATE DISCARDS IT"),
    ];
    for (path, zone, phrase) in cases {
        let mut fake = FakeFiles::new().with(path, Node::file("x"));
        let answer = get(&mut fake, path);
        let application = field(&answer, "application")
            .unwrap_or_else(|| panic!("{path} is not reported as an application path"));
        assert_eq!(text(application, "zone"), zone, "{path}");
        if phrase.is_empty() {
            // A mechanism path has no note: the refusal says everything.
            assert_eq!(field(application, "note"), None, "{path}");
            assert!(!flag(&answer, "changeable"), "{path}");
        } else {
            assert!(
                text(application, "note").contains(phrase),
                "{path}: {}",
                text(application, "note")
            );
            assert!(flag(&answer, "changeable"), "{path} is refused");
        }
    }
}

/// **An image service's home is not an application's**, and nothing in it is
/// discarded by an update: it has no version directories, because its program
/// is on the system slot (`ADR-060`). Every home `IMAGE_SERVICE_HOMES` names is
/// asked, so a home added there is described correctly without a second edit.
#[test]
fn a_file_in_an_image_services_home_is_not_described_as_discarded_by_an_update() {
    for home in crate::service::IMAGE_SERVICE_HOMES {
        let root = format!("/state/app/{}", home.name);
        let cases = [
            (format!("{root}/keys/leaf.pem"), "image-service", "a change here persists"),
            (format!("{root}/0.2.0/bin/thing"), "image-service", "a change here persists"),
            (format!("{root}/data/store.db"), "data", "survives an update"),
            (format!("{root}/routes"), "routes", "An update leaves it"),
            (format!("{root}/dns.toml"), "name-service", "change `[dns]` there"),
        ];
        for (path, zone, phrase) in cases {
            let mut fake = FakeFiles::new().with(&path, Node::file("x"));
            let answer = get(&mut fake, &path);
            let application = field(&answer, "application")
                .unwrap_or_else(|| panic!("{path} is not reported as an application path"));
            assert_eq!(text(application, "zone"), zone, "{path}");
            let note = text(application, "note");
            assert!(note.contains(phrase), "{path}: {note}");
            assert!(!note.contains("DISCARDS"), "{path}: {note}");
        }
        // The home itself is still refused, and the refusal names what it is:
        // part of the image, not an application an uninstall removes.
        let mut fake = FakeFiles::new().with(&root, Node::directory());
        let answer = get(&mut fake, &root);
        assert!(!flag(&answer, "changeable"), "{root}");
        let because = text(&answer, "unchangeable_because");
        assert!(because.contains("home of an image service"), "{root}: {because}");
    }
}

// -- refusals ----------------------------------------------------------------

#[test]
fn a_path_that_is_not_absolute_or_carries_a_traversal_is_refused_before_anything_is_read() {
    let mut fake = FakeFiles::new();
    for path in ["state/x", "../etc/passwd", "/state/../system", "/state/x/"] {
        let refused = refusal(&mut fake, &[SUBJECT, path]);
        assert_eq!(refused.code(), ErrorCode::Arguments, "{path}");
    }
}

#[test]
fn naming_no_path_and_naming_two_are_both_refused_by_name() {
    let mut fake = FakeFiles::new();
    let none = refusal(&mut fake, &[SUBJECT]);
    assert_eq!(none.code(), ErrorCode::Arguments);
    assert!(none.message().contains("ABSOLUTE path"));

    // A second path is now read as a byte offset and refused as one, which is
    // the cost of `ADR-006` §8's polymorphic positional input paid again.
    // It is still refused by NAME and still says what that position is for.
    let two = refusal(&mut fake, &[SUBJECT, "/state/a", "/state/b"]);
    assert_eq!(two.code(), ErrorCode::Arguments);
    assert!(two.message().contains("an offset in bytes"));

    let five = refusal(&mut fake, &[SUBJECT, "/state/a", "0", "1", "extra"]);
    assert_eq!(five.code(), ErrorCode::Arguments);
    assert!(five.message().contains("fifth word"));
}

/// The catalogue naming a field no result carries is `ADR-006` §7's *"trusted
/// and wrong"*, and the `debug_assert` in `perform` is what holds it — this
/// walks the shapes that assert has to survive.
#[test]
fn every_shape_this_subject_produces_keeps_the_declared_field_order() {
    let mut fake = FakeFiles::new()
        .with("/state/f", Node::file("x"))
        .with("/state/d", Node::directory())
        .with("/state/d/inside", Node::file("y"))
        .with("/state/l", Node::symlink("/state/f"));
    for path in ["/state/f", "/state/d", "/state/l", "/state/gone"] {
        let answer = get(&mut fake, path);
        let Value::Object(fields) = &answer else {
            panic!("{path} did not answer with an object");
        };
        let declared: Vec<&str> = OUTPUTS.iter().map(|f| f.name).collect();
        let mut at = 0usize;
        for (key, _) in fields {
            let found = declared
                .iter()
                .skip(at)
                .position(|d| d == key)
                .unwrap_or_else(|| panic!("{path}: `{key}` is out of order or undeclared"));
            at += found + 1;
        }
    }
}

// ---------------------------------------------------------------------------
// base64 — ruled by the maintainer, ADR-006 §8.10
// ---------------------------------------------------------------------------

// **The codec's own tests moved to `crate::base64` on 2026-09-01**, with the
// encoder itself — `the_encoder_agrees_with_rfc_4648s_published_vectors` and
// `every_byte_value_encodes_to_a_well_formed_quad`, both intact and both now
// also covering the DECODER that another module had been carrying separately.
// Nothing was dropped; this note is here so a reader looking for them in the
// file that used to own them is told where they went.
//
// What stays here is what is about `get file`: that a file which is not UTF-8
// comes back as base64 and says so, which is
// `a_file_that_is_not_utf8_comes_back_base64_and_says_so` above, and that the
// two encodings name themselves, which is below.

/// **The bytes `get file` publishes are the file's bytes**, checked through the
/// shared codec rather than by re-implementing it here.
///
/// The point of this test is the *seam*, not the alphabet: `cargo rivet
/// screenshot` reads `content` back with `crate::base64::decode` and hands the
/// result to a QOI decoder, so a capture only survives if what this operation
/// wrote is exactly what the file held. Every byte value is in the fixture
/// because a screenshot contains all of them.
#[test]
fn a_binary_files_content_decodes_back_to_the_bytes_it_held() {
    let bytes: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
    let encoded = crate::base64::encode(&bytes);
    assert_eq!(crate::base64::decode(&encoded), Some(bytes));
}

/// The two variants say which they are, so a caller cannot read the value
/// without having been told how.
#[test]
fn an_encoding_names_itself() {
    use super::Encoded;
    assert_eq!(Encoded::Text("hi".to_string()).word(), "text");
    assert_eq!(Encoded::Base64("aGk=".to_string()).word(), "base64");
    assert_eq!(Encoded::Base64("aGk=".to_string()).value(), "aGk=");
}
