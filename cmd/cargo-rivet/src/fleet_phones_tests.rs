//! `cargo rivet fleet phones`'s tests: the grammar, and that `list`/
//! `rename`/`revoke` compose the right board command against a fake
//! transport — `resolve`'s register lookup and `run`'s real TLS transport
//! need a real register and a real board and are not exercised here.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use std::cell::RefCell;

fn parse_of(list: &[&str]) -> Result<PhonesArgs, String> {
    let owned: Vec<String> = list.iter().map(|w| (*w).to_string()).collect();
    parse(&owned)
}

#[test]
fn list_takes_no_positional_and_no_confirm() {
    let parsed = parse_of(&["phones", "list"]).expect("valid");
    assert!(matches!(parsed.verb, PhonesVerb::List));
}

#[test]
fn list_refuses_a_positional() {
    let err = parse_of(&["phones", "list", "extra"]).expect_err("refused");
    assert!(err.contains("takes no positional"), "{err}");
}

#[test]
fn list_refuses_confirm() {
    let err = parse_of(&["phones", "list", "confirm"]).expect_err("refused");
    assert!(err.contains("takes no `confirm`"), "{err}");
}

#[test]
fn rename_needs_an_identity_and_a_label() {
    let err = parse_of(&["phones", "rename"]).expect_err("refused");
    assert!(err.contains("needs an identity and a label"), "{err}");
    let err = parse_of(&["phones", "rename", "abc"]).expect_err("refused");
    assert!(err.contains("needs a label"), "{err}");
}

#[test]
fn rename_parses_identity_label_and_confirm() {
    let parsed = parse_of(&["phones", "rename", "abc", "Pixel 8", "confirm"]).expect("valid");
    match parsed.verb {
        PhonesVerb::Rename { identity, label, confirm } => {
            assert_eq!(identity, "abc");
            assert_eq!(label, "Pixel 8");
            assert!(confirm);
        }
        _ => panic!("not a Rename"),
    }
}

#[test]
fn rename_refuses_a_third_word() {
    let err = parse_of(&["phones", "rename", "abc", "label", "extra"]).expect_err("refused");
    assert!(err.contains("third word"), "{err}");
}

#[test]
fn revoke_needs_an_identity() {
    let err = parse_of(&["phones", "revoke"]).expect_err("refused");
    assert!(err.contains("needs an identity"), "{err}");
}

#[test]
fn revoke_parses_identity_and_confirm() {
    let parsed = parse_of(&["phones", "revoke", "abc", "confirm"]).expect("valid");
    match parsed.verb {
        PhonesVerb::Revoke { identity, confirm } => {
            assert_eq!(identity, "abc");
            assert!(confirm);
        }
        _ => panic!("not a Revoke"),
    }
}

#[test]
fn revoke_refuses_a_second_word() {
    let err = parse_of(&["phones", "revoke", "abc", "extra"]).expect_err("refused");
    assert!(err.contains("second word"), "{err}");
}

#[test]
fn an_unknown_sub_verb_is_refused() {
    let err = parse_of(&["phones", "pair"]).expect_err("refused");
    assert!(err.contains("has list, rename and revoke"), "{err}");
}

#[test]
fn endpoint_flags_are_read_around_the_positionals() {
    let parsed = parse_of(&[
        "phones", "rename", "--host", "pi:7777", "--identity", "./id", "--pin", "ab", "abc",
        "Pixel 8", "confirm",
    ])
    .expect("valid");
    assert_eq!(parsed.endpoint.host.as_deref(), Some("pi:7777"));
    match parsed.verb {
        PhonesVerb::Rename { identity, label, confirm } => {
            assert_eq!(identity, "abc");
            assert_eq!(label, "Pixel 8");
            assert!(confirm);
        }
        _ => panic!("not a Rename"),
    }
}

// ---------------------------------------------------------------------------
// A fake transport, recording every call and answering with fixed JSON.
// ---------------------------------------------------------------------------

struct FakeTransport {
    calls: RefCell<Vec<(String, String)>>,
    /// What `call_tool` answers with, keyed by tool name — the same shape a
    /// real machine's answer is: a JSON string, unparsed by this fake.
    answers: Vec<(&'static str, String)>,
}

impl rivet_remote::tls::CallsTools for FakeTransport {
    fn call_tool(&self, tool: &str, arguments: &str) -> Result<String, String> {
        self.calls.borrow_mut().push((tool.to_owned(), arguments.to_owned()));
        self.answers
            .iter()
            .find(|(t, _)| *t == tool)
            .map(|(_, a)| a.clone())
            .ok_or_else(|| format!("the fake has no answer for {tool}"))
    }
}

/// A phone record, rendered the exact way `rivetos_register::phones::render`
/// renders one — hand-composed here rather than pulled from that crate, so
/// this test does not depend on its exact pretty-printing, and hex-encoded
/// the same way `get phones` encodes one.
fn record_hex(identity: &str, label: &str) -> String {
    let text = format!(
        "identity = \"{identity}\"\nlabel = \"{label}\"\npublic_key = \"MFkBAgM=\"\nenrolled = \
         \"2000-01-02T03:04:05Z\"\n"
    );
    rivet_core::package::key_material::to_hex(text.as_bytes())
}

fn get_phones_answer(identity: &str, label: &str) -> String {
    format!(
        r#"{{"subject":"phones","count":1,"phones":[{{"identity":"{identity}","record":"{hex}"}}]}}"#,
        hex = record_hex(identity, label),
    )
}

const IDENTITY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

#[test]
fn list_calls_get_phones_and_decodes_the_label() {
    let fake = FakeTransport {
        calls: RefCell::new(Vec::new()),
        answers: vec![("rivet_get", get_phones_answer(IDENTITY, "Pixel 8"))],
    };
    let printed = list(&fake).expect("lists");
    assert!(printed.contains(IDENTITY), "{printed}");
    assert!(printed.contains("Pixel 8"), "{printed}");
    let calls = fake.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "rivet_get");
    assert!(calls[0].1.contains(r#""subject":"phones""#), "{}", calls[0].1);
}

#[test]
fn rename_reads_the_current_record_then_sends_the_whole_new_one() {
    let fake = FakeTransport {
        calls: RefCell::new(Vec::new()),
        answers: vec![
            ("rivet_get", get_phones_answer(IDENTITY, "Old Name")),
            ("rivet_apply", r#"{"subject":"rename-phone","identity":""}"#.to_owned()),
        ],
    };
    rename(&fake, IDENTITY, "New Name", true).expect("renames");
    let calls = fake.calls.borrow();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert_eq!(calls[0].0, "rivet_get");
    assert_eq!(calls[1].0, "rivet_apply");
    assert!(calls[1].1.contains(r#""subject":"rename-phone""#), "{}", calls[1].1);
    assert!(calls[1].1.contains(IDENTITY), "{}", calls[1].1);
    assert!(calls[1].1.contains(r#""confirmation":"confirm""#), "{}", calls[1].1);

    // **The label changed and nothing else did.** Decode the hex this test
    // just sent and read it back the same way `rivetos_register::phones`
    // would.
    let sent = crate::json::parse(&calls[1].1).expect("valid JSON");
    let hex = sent
        .get("change")
        .and_then(crate::json::Json::as_array)
        .and_then(|words| words.last())
        .and_then(crate::json::Json::as_str)
        .expect("the hex is the last `change` word");
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).expect("hex"))
        .collect();
    let text = String::from_utf8(bytes).expect("utf8");
    assert!(text.contains("label = \"New Name\""), "{text}");
    assert!(text.contains(&format!("identity = \"{IDENTITY}\"")), "{text}");
    assert!(text.contains("public_key = \"MFkBAgM=\""), "{text}");
    assert!(text.contains("enrolled = \"2000-01-02T03:04:05Z\""), "{text}");
}

/// **Never sent unconfirmed** — see `rename`'s own documentation for why:
/// `apply`'s binder reserves the LAST word for `confirmation` no matter what
/// it is, so a `rename-phone` sent with no trailing `confirm` would have its
/// hex payload's tail misbound instead. This previews LOCALLY from the record
/// it already read, and touches the network once, not twice.
#[test]
fn rename_without_confirm_previews_locally_and_sends_nothing() {
    let fake = FakeTransport {
        calls: RefCell::new(Vec::new()),
        answers: vec![("rivet_get", get_phones_answer(IDENTITY, "Old Name"))],
    };
    let printed = rename(&fake, IDENTITY, "New Name", false).expect("previews");
    assert!(printed.contains("Old Name"), "{printed}");
    assert!(printed.contains("New Name"), "{printed}");
    assert!(printed.contains("confirm"), "{printed}");
    let calls = fake.calls.borrow();
    assert_eq!(calls.len(), 1, "only the read, {calls:?}");
    assert_eq!(calls[0].0, "rivet_get");
}

const OTHER_IDENTITY: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

#[test]
fn rename_refuses_an_identity_the_list_does_not_hold() {
    let fake = FakeTransport {
        calls: RefCell::new(Vec::new()),
        answers: vec![("rivet_get", get_phones_answer(OTHER_IDENTITY, "X"))],
    };
    let err = rename(&fake, IDENTITY, "New Name", true).expect_err("refused");
    assert!(err.contains("no phone is enrolled"), "{err}");
    // Only the read happened; nothing was sent.
    assert_eq!(fake.calls.borrow().len(), 1);
}

#[test]
fn revoke_sends_the_identity_alone() {
    let fake = FakeTransport {
        calls: RefCell::new(Vec::new()),
        answers: vec![("rivet_apply", r#"{"subject":"revoke-phone"}"#.to_owned())],
    };
    revoke(&fake, IDENTITY, true).expect("revokes");
    let calls = fake.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "rivet_apply");
    assert!(calls[0].1.contains(r#""subject":"revoke-phone""#), "{}", calls[0].1);
    assert!(calls[0].1.contains(IDENTITY), "{}", calls[0].1);
    assert!(calls[0].1.contains(r#""confirmation":"confirm""#), "{}", calls[0].1);
}

#[test]
fn revoke_without_confirm_omits_the_word() {
    let fake = FakeTransport {
        calls: RefCell::new(Vec::new()),
        answers: vec![("rivet_apply", r#"{"ok":false}"#.to_owned())],
    };
    revoke(&fake, IDENTITY, false).expect("the fake always answers Ok");
    assert!(!fake.calls.borrow()[0].1.contains("confirmation"));
}
