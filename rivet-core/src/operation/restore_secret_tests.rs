//! `apply restore-secret`'s tests.
//!
//! **The fixture is `device.seed`**: 32 raw bytes with a null byte in them,
//! because a null byte is what `apply edit` cannot carry.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::deploy::Origin;
use crate::operation::ErrorCode;
use crate::operation::filesystem::fake::{FakeFiles, Node};

/// The key `FakeFiles::principal` answers with — its own fixed token.
fn session_key() -> String {
    "fedcba9876543210".repeat(4)
}

fn seed() -> Vec<u8> {
    let mut bytes = vec![0x5a, 0x00, 0xee, 0x01];
    bytes.extend((4u8..32).map(|n| n.wrapping_mul(11)));
    bytes
}

fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A board with `/state/secrets`, root-owned, mode 0700 — as `rivet-initd`
/// makes it.
fn provisioned() -> FakeFiles {
    FakeFiles::new().with(SECRETS, Node::directory().moded(0o700))
}

fn restore(fake: &mut FakeFiles, words: &[&str]) -> Result<Restored, OperationError> {
    perform(words, fake)
}

fn confirmed(fake: &mut FakeFiles, name: &str, bytes: &[u8]) -> Result<Restored, OperationError> {
    restore(fake, &[SUBJECT, name, HEX_FLAG, &hex_of(bytes), CONFIRMATION])
}

#[test]
fn a_binary_seed_is_put_back_whole_root_owned_and_private() {
    let mut fake = provisioned();
    let bytes = seed();
    let done = confirmed(&mut fake, "device.seed", &bytes).expect("restores");
    let path = format!("{SECRETS}/device.seed");
    let node = &fake.nodes[&path];
    assert_eq!(node.contents, bytes);
    assert_eq!(node.contents[1], 0, "the null byte is the whole point");
    assert_eq!((node.uid, node.gid), (0, 0));
    assert_eq!(node.mode, SECRET_MODE);
    assert_eq!(fake.did, vec![format!("install-secret {path}")]);
    assert!(done.journalled());
}

#[test]
fn without_the_confirmation_it_plans_writes_nothing_and_never_echoes_the_value() {
    let mut fake = provisioned().with(&format!("{SECRETS}/wlan0.psk"), Node::file("OLD VALUE"));
    let hex = hex_of(b"NEW VALUE!");
    let planned = restore(&mut fake, &[SUBJECT, "wlan0.psk", HEX_FLAG, &hex])
        .expect_err("a plan");
    assert_eq!(planned.code(), ErrorCode::ConfirmationRequired);
    let words = planned.plan().expect("a plan").narrative();
    assert!(words.contains("REPLACES WHAT IS THERE"), "{words}");
    assert!(words.contains("9 bytes"), "{words}");
    assert!(words.contains("10 bytes"), "{words}");
    assert!(words.contains("NOTHING IS RESTARTED"), "{words}");
    assert!(!words.contains(&hex), "{words}");
    assert!(!words.contains("OLD VALUE"), "{words}");
    assert!(words.contains(REDACTED_VALUE), "{words}");
    assert!(fake.did.is_empty());
    assert_eq!(fake.nodes[&format!("{SECRETS}/wlan0.psk")].contents, b"OLD VALUE");
}

#[test]
fn a_replacement_reports_what_is_gone() {
    let mut fake = provisioned().with(&format!("{SECRETS}/device.crt"), Node::file("0123456789"));
    let done = confirmed(&mut fake, "device.crt", b"ab").expect("restores");
    let value = done.value();
    let Value::Object(fields) = value else { panic!("an object") };
    let replaced = fields.iter().find(|(k, _)| *k == "replaced-bytes").map(|(_, v)| v.clone());
    assert_eq!(replaced, Some(Value::Integer(10)));
}

/// **The path is composed, never given.** No name reaches outside
/// `/state/secrets`.
#[test]
fn a_name_that_is_a_path_is_refused_before_anything_is_composed() {
    for name in ["../config/machine.toml", "..", ".", "a/b", "back\\slash", ".hidden", "-x", ""] {
        let mut fake = provisioned();
        let refused = restore(&mut fake, &[SUBJECT, name, HEX_FLAG, "00", CONFIRMATION])
            .expect_err("refused");
        assert_eq!(refused.code(), ErrorCode::Arguments, "{name} was accepted");
        assert!(fake.did.is_empty(), "{name} reached the machine");
    }
}

#[test]
fn a_board_with_no_secret_store_is_refused_rather_than_having_one_made() {
    let mut fake = FakeFiles::new();
    let refused = confirmed(&mut fake, "device.seed", &seed()).expect_err("refused");
    assert_eq!(refused.code(), ErrorCode::Unsafe);
    assert!(refused.message().contains("not there"), "{}", refused.message());
    assert!(!fake.has(SECRETS));
    assert!(fake.did.is_empty());
}

#[test]
fn a_symbolic_link_or_a_directory_at_the_name_is_refused() {
    for node in [Node::symlink("/state/config/machine.toml"), Node::directory()] {
        let mut fake = provisioned().with(&format!("{SECRETS}/peers"), node);
        let refused = restore(
            &mut fake,
            &[SUBJECT, "peers", HEX_FLAG, &hex_of(session_key().as_bytes()), CONFIRMATION],
        )
        .expect_err("refused");
        assert_eq!(refused.code(), ErrorCode::Unsafe);
    }
}

/// **The lock-out refusal.** A `peers` file that does not name the session's
/// own key would leave the board answering nobody after its next boot.
#[test]
fn a_peers_file_that_drops_the_callers_key_is_refused() {
    let mut fake = provisioned();
    let other = "0123456789abcdef".repeat(4);
    let text = format!("# somebody else\n{other}\n");
    let refused = confirmed(&mut fake, PEERS_NAME, text.as_bytes()).expect_err("lock-out");
    assert_eq!(refused.code(), ErrorCode::Unsafe);
    assert!(refused.message().contains(&session_key()), "{}", refused.message());
    assert!(fake.did.is_empty());
    // And on the plan path too, before any confirmation.
    let planned = restore(&mut fake, &[SUBJECT, PEERS_NAME, HEX_FLAG, &hex_of(text.as_bytes())])
        .expect_err("refused");
    assert_eq!(planned.code(), ErrorCode::Unsafe);
}

#[test]
fn a_peers_file_that_names_the_callers_key_in_any_case_is_accepted() {
    let mut fake = provisioned();
    let text = format!("# the operator\n  {}  # me\n", session_key().to_ascii_uppercase());
    confirmed(&mut fake, PEERS_NAME, text.as_bytes()).expect("admits the caller");
    assert_eq!(fake.nodes[&format!("{SECRETS}/{PEERS_NAME}")].contents, text.as_bytes());
}

/// **The whole file's grammar is checked, not just whether the caller's own
/// key is present somewhere in it.** A membership-only check asks only "does
/// any line match my key", so a file naming the caller correctly on one line
/// and garbage on another would be accepted and written — and `rivet-mcpd`
/// refuses that whole file at its next start, over a line the caller's own
/// key never protected. Every malformed line here sits beside a line naming
/// the caller's real key, both before and after it, so a membership-only
/// check would let each one through.
#[test]
fn a_malformed_peers_line_is_refused_even_beside_the_callers_own_key() {
    let key = session_key();
    let malformed_lines = [
        "not-a-key".to_string(),
        "a".repeat(62),
        "a".repeat(66),
        format!("{key} extra-token"),
    ];
    for bad in &malformed_lines {
        for text in [format!("{key}\n{bad}\n"), format!("{bad}\n{key}\n")] {
            let mut fake = provisioned();
            let refused = confirmed(&mut fake, PEERS_NAME, text.as_bytes())
                .expect_err(&format!("{text:?} should be refused"));
            assert_eq!(refused.code(), ErrorCode::Unsafe, "{text:?}: {}", refused.message());
            assert!(fake.did.is_empty(), "{text:?} wrote something");
        }
    }
}

/// **The grammar check does not live behind the membership check.** A
/// session with no pinned key skips the membership check entirely (there is
/// no key to look for), but that must not mean it can write anything — the
/// file it writes is read by the same `rivet-mcpd` at the next boot
/// regardless of who restored it.
#[test]
fn a_malformed_peers_file_is_refused_even_with_no_pinned_key_to_protect() {
    let key = session_key();
    let text = format!("{key}\nnot-a-key\n");
    let mut fake = provisioned().with_origin(Origin::Console);
    let refused = confirmed(&mut fake, PEERS_NAME, text.as_bytes()).expect_err("refused");
    assert_eq!(refused.code(), ErrorCode::Unsafe, "{}", refused.message());
    assert!(fake.did.is_empty());
    // And on the plan path too, before any confirmation.
    let planned = restore(&mut fake, &[SUBJECT, PEERS_NAME, HEX_FLAG, &hex_of(text.as_bytes())])
        .expect_err("refused");
    assert_eq!(planned.code(), ErrorCode::Unsafe);
}

/// **Valid grammar, several shapes at once, is still accepted and written.**
/// Several keys, a comment on its own line, a comment after a key, and blank
/// lines all in the one file.
#[test]
fn a_peers_file_with_several_keys_comments_and_blank_lines_is_accepted() {
    let mut fake = provisioned();
    let key = session_key();
    let other = "0123456789abcdef".repeat(4);
    let text = format!("# fleet keys\n\n{other}  # a teammate\n\n{key}\n# trailing comment\n");
    confirmed(&mut fake, PEERS_NAME, text.as_bytes()).expect("valid grammar, names the caller");
    assert_eq!(fake.nodes[&format!("{SECRETS}/{PEERS_NAME}")].contents, text.as_bytes());
}

#[test]
fn peers_matching_reads_the_pin_file_grammar() {
    let key = session_key();
    assert!(peers_name_key(&format!("{key}\n"), &key));
    assert!(peers_name_key(&format!("# c\n\n{key} # me"), &key));
    assert!(!peers_name_key(&format!("# {key}\n"), &key), "a commented-out key is not a key");
    assert!(!peers_name_key(&format!("{key}00\n"), &key));
    assert!(!peers_name_key("", &key));
}

#[test]
fn the_origin_the_fake_answers_with_is_the_key_these_tests_use() {
    assert_eq!(
        Origin::pinned(&session_key()).and_then(|o| o.pinned_key().map(str::to_string)),
        Some(session_key())
    );
}

#[test]
fn the_flag_cannot_swallow_the_confirmation_and_an_acknowledgement_is_superfluous() {
    let mut fake = provisioned();
    let refused = restore(&mut fake, &[SUBJECT, "x", HEX_FLAG, CONFIRMATION]).expect_err("eaten");
    assert_eq!(refused.code(), ErrorCode::Arguments);
    assert!(refused.message().contains("has been eaten"), "{}", refused.message());
    let refused = restore(
        &mut fake,
        &[SUBJECT, "x", HEX_FLAG, "00", super::super::apply::ACCEPT_FLAG, "anything", CONFIRMATION],
    )
    .expect_err("no warning to accept");
    assert_eq!(refused.code(), ErrorCode::Arguments);
    assert!(fake.did.is_empty());
}

#[test]
fn a_value_that_is_not_hexadecimal_is_refused_without_being_printed() {
    for bad in ["0x00ff", "abc", "zz", "00 ff"] {
        let mut fake = provisioned();
        let refused = restore(&mut fake, &[SUBJECT, "x", HEX_FLAG, bad, CONFIRMATION])
            .expect_err("refused");
        assert_eq!(refused.code(), ErrorCode::Arguments, "{bad}");
        assert!(!refused.message().contains(bad), "{}", refused.message());
    }
}

#[test]
fn a_machine_that_refuses_the_act_is_reported_as_a_machine_refusal() {
    let mut fake = provisioned();
    fake.act_fails = Some(crate::operation::machine::MachineError::Refused(
        "the filesystem is read-only".to_string(),
    ));
    let refused = confirmed(&mut fake, "device.seed", &seed()).expect_err("refused");
    assert_eq!(refused.code(), ErrorCode::MachineRefused);
}

/// Through the verb's own dispatcher, so the catalogue gate is exercised too.
#[test]
fn it_is_reachable_through_apply() {
    let mut fake = provisioned();
    let hex = hex_of(&seed());
    super::super::apply::perform(&[SUBJECT, "device.seed", HEX_FLAG, &hex, CONFIRMATION], &mut fake)
        .expect("apply restore-secret");
    assert_eq!(fake.nodes[&format!("{SECRETS}/device.seed")].contents, seed());
}
