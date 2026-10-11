//! `apply install-secret`'s tests.
//!
//! **The fixture is the case the subject was written for**, throughout:
//! `cmd/rivet-fleetd`'s Ed25519 seed, 32 bytes, **with a null byte in it** —
//! because a null byte is precisely what `apply edit` cannot carry, and a test
//! whose fixture was ASCII would pass against a text-only implementation.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::filesystem::fake::{FakeFiles, Node};
use crate::operation::ErrorCode;
use crate::operation::machine::PathStat;

/// The application every test here installs into.
const APP: &str = "fleetd";
/// Its data directory, spelled out rather than composed, so this file is a
/// second opinion about the path rather than a copy of the code's arithmetic.
const DATA: &str = "/state/app/fleetd/data";

/// A 32-byte seed **whose second byte is zero**. See this module's
/// documentation.
fn seed() -> Vec<u8> {
    let mut bytes = vec![0xa3, 0x00, 0xff, 0x10];
    bytes.extend((4u8..32).map(|n| n.wrapping_mul(7)));
    bytes
}

fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A machine with `fleetd` installed — which for this subject means its data
/// directory exists, owned by the application at mode 0700, exactly as
/// `ADR-015` §3.2 has the installer make it.
fn installed() -> FakeFiles {
    let id = app_identity(&ServiceName::new(APP).expect("a valid name"));
    FakeFiles::new()
        .with("/state/app", Node::directory())
        .with("/state/app/fleetd", Node::directory())
        .with(
            DATA,
            Node::directory().moded(0o700).owned(id.uid, id.gid),
        )
}

fn install(fake: &mut FakeFiles, words: &[&str]) -> Result<Installed, OperationError> {
    perform(words, fake)
}

fn refusal(result: Result<Installed, OperationError>) -> OperationError {
    result.expect_err("that command must be refused")
}

/// The whole command, with the confirmation.
fn confirmed(fake: &mut FakeFiles, name: &str, bytes: &[u8]) -> Installed {
    install(
        fake,
        &[
            SUBJECT,
            APP,
            name,
            HEX_FLAG,
            &hex_of(bytes),
            CONFIRMATION,
        ],
    )
    .expect("the secret must install")
}

// -- the gap this closes -----------------------------------------------------

/// **The defect, in one test.** A binary seed reaches the application's data
/// directory, owned by the application, with its null byte intact.
///
/// Every one of the four assertions below is something the three-command
/// `edit` + `set-owner` + `set-mode` sequence got wrong or could not do.
#[test]
fn a_binary_seed_arrives_whole_owned_by_the_application_and_private() {
    let mut fake = installed();
    let bytes = seed();
    let done = confirmed(&mut fake, "operator-device.seed", &bytes);

    let path = format!("{DATA}/operator-device.seed");
    // **The path is composed and reported**, so a caller learns where the
    // secret went without knowing this operation's arithmetic.
    assert_eq!(text(&done.value(), "path"), path);
    let node = &fake.nodes[&path];

    // 1. **The bytes survived.** A `&str` round trip could not carry these.
    assert_eq!(node.contents, bytes);
    assert_eq!(node.contents[1], 0, "the null byte is the whole point");
    assert_eq!(node.contents.len(), 32);

    // 2. **It is owned by the application**, not by root.
    let id = app_identity(&ServiceName::new(APP).expect("a valid name"));
    assert_eq!((node.uid, node.gid), (id.uid, id.gid));
    assert_ne!(node.uid, 0, "root ownership is the defect");

    // 3. **It is private**, and the answer says so.
    assert_eq!(node.mode, SECRET_MODE);

    // 4. **One act**, not three. The journal and the confirmation both hang off
    //    this: three acts is three plans and three ways to stop half way.
    assert_eq!(fake.did, vec![format!("install-secret {path}")]);
}

/// The answer names the owner it chose, because a caller cannot otherwise tell
/// whether the file is reachable by the application without a second command.
#[test]
fn the_answer_states_the_uid_the_mode_and_the_length() {
    let mut fake = installed();
    let done = confirmed(&mut fake, "cert", &seed());
    let value = done.value();
    let id = app_identity(&ServiceName::new(APP).expect("a valid name"));

    assert_eq!(text(&value, "subject"), SUBJECT);
    assert_eq!(text(&value, "application"), APP);
    assert_eq!(number(&value, "bytes"), 32);
    assert_eq!(number(&value, "uid"), i64::from(id.uid));
    assert_eq!(number(&value, "gid"), i64::from(id.gid));
    assert!(text(&value, "mode").starts_with("0600"), "{}", text(&value, "mode"));
    assert!(
        field(&value, "replaced-bytes").is_none(),
        "a first install replaced nothing and must not report a zero"
    );
    assert!(done.journalled());
}

// -- the plan ----------------------------------------------------------------

/// Nothing happens without the confirmation, and the plan says what would.
#[test]
fn without_the_confirmation_it_plans_and_writes_nothing() {
    let mut fake = installed();
    let hex = hex_of(&seed());
    let planned = refusal(install(
        &mut fake,
        &[SUBJECT, APP, "operator.seed", HEX_FLAG, &hex],
    ));

    assert_eq!(planned.code(), ErrorCode::ConfirmationRequired);
    let plan = planned.plan().expect("a plan");
    let words = plan.narrative();
    assert!(words.contains("32 bytes"), "{words}");
    assert!(words.contains("0600"), "{words}");
    assert!(words.contains("NOT restarted"), "{words}");
    assert!(
        !fake.has(&format!("{DATA}/operator.seed")),
        "the plan wrote something"
    );
    assert!(fake.did.is_empty());
    // **The interlock ran on the plan path**, which is what makes a refusal
    // arrive before the confirmation rather than after it.
    assert_eq!(
        fake.asked,
        vec![format!("install-secret {DATA}/operator.seed")]
    );
}

/// **The plan must never print the secret.** `ADR-020` §4, and the one place
/// this operation could leak: a plan is written to a terminal and often pasted.
#[test]
fn neither_the_plan_nor_the_answer_ever_contains_the_secret() {
    let mut fake = installed().with(
        &format!("{DATA}/cert"),
        Node::file("OLD SECRET CONTENT"),
    );
    let hex = hex_of(b"NEW SECRET CONTENT");
    let planned = refusal(install(&mut fake, &[SUBJECT, APP, "cert", HEX_FLAG, &hex]));
    let plan = planned.plan().expect("a plan");
    let words = plan.narrative();

    assert!(!words.contains("NEW SECRET"), "{words}");
    assert!(!words.contains("OLD SECRET"), "{words}");
    assert!(!words.contains(&hex), "the hex itself is a secret: {words}");
    // What it says instead: two lengths, and a stand-in where the value was.
    assert!(words.contains("18 bytes"), "{words}");
    assert!(words.contains(REDACTED_VALUE), "{words}");
    assert!(
        words.contains("rivet apply install-secret fleetd cert --hex"),
        "the line to repeat must still be there, minus the value: {words}"
    );
}

/// **The leak this subject nearly shipped**, kept as its own test because the
/// one above could pass on a plan that had dropped the confirmation line
/// entirely — which would be a different defect wearing this one's clothes.
///
/// Break `confirmation_words_without_the_secret` to `confirmation_words` and
/// this fails.
#[test]
fn the_line_the_plan_tells_you_to_repeat_does_not_carry_the_credential() {
    let mut fake = installed();
    let hex = hex_of(&seed());
    let planned = refusal(install(&mut fake, &[SUBJECT, APP, "s", HEX_FLAG, &hex]));
    let plan = planned.plan().expect("a plan");
    assert!(
        !plan.narrative().contains(&hex),
        "{}",
        plan.narrative()
    );
    assert!(plan.narrative().contains("NOT ECHOED"), "{}", plan.narrative());
}

/// A replacement is named as one, and the old length is reported because it is
/// the one thing nobody can recover afterwards.
#[test]
fn replacing_an_existing_secret_says_so_and_reports_what_is_gone() {
    let mut fake = installed().with(&format!("{DATA}/cert"), Node::file("0123456789"));
    let planned = refusal(install(
        &mut fake,
        &[SUBJECT, APP, "cert", HEX_FLAG, &hex_of(b"ab")],
    ));
    let words = planned.plan().expect("a plan").narrative();
    assert!(words.contains("REPLACES AN EXISTING SECRET"), "{words}");
    assert!(words.contains("10 bytes"), "{words}");

    let done = confirmed(&mut fake, "cert", b"ab");
    assert_eq!(number(&done.value(), "replaced-bytes"), 10);
    assert_eq!(fake.nodes[&format!("{DATA}/cert")].contents, b"ab");
}

/// **Rotation is a destructive act and the journal must be able to say so.**
/// The first install is not.
#[test]
fn only_a_replacement_counts_as_destroying_something() {
    let first = PathChange::InstallSecret {
        path: "/state/app/fleetd/data/cert",
        contents: b"x",
        mode: SECRET_MODE,
        uid: 4000,
        gid: 4000,
        was_bytes: 0,
    };
    let rotation = PathChange::InstallSecret {
        path: "/state/app/fleetd/data/cert",
        contents: b"x",
        mode: SECRET_MODE,
        uid: 4000,
        gid: 4000,
        was_bytes: 40,
    };
    assert!(!first.destroys());
    assert!(rotation.destroys());
    assert_eq!(first.act(), SUBJECT);
}

// -- what it refuses ---------------------------------------------------------

/// **The refusal that keeps this subject inside one directory.** No name that
/// composes a path outside the data directory is representable.
#[test]
fn a_name_that_is_a_path_is_refused_before_anything_is_composed() {
    for name in [
        "../../../etc/shadow",
        "..",
        ".",
        "sub/dir",
        "back\\slash",
        ".hidden",
        "-flag",
        "has space",
        "",
    ] {
        let mut fake = installed();
        let refused = refusal(install(
            &mut fake,
            &[SUBJECT, APP, name, HEX_FLAG, "00", CONFIRMATION],
        ));
        assert_eq!(refused.code(), ErrorCode::Arguments, "{name} was accepted");
        assert!(fake.did.is_empty(), "{name} reached the machine");
    }
}

/// An application whose data directory is not there is not installed, and a
/// credential written for it would sit where nothing reads it.
#[test]
fn an_application_with_no_data_directory_is_refused_rather_than_having_one_made() {
    let mut fake = FakeFiles::new();
    let refused = refusal(install(
        &mut fake,
        &[SUBJECT, APP, "cert", HEX_FLAG, "00", CONFIRMATION],
    ));
    assert_eq!(refused.code(), ErrorCode::Unsafe);
    assert!(refused.message().contains("no data directory"), "{}", refused.message());
    assert!(!fake.has(DATA), "a directory was created");
    assert!(fake.did.is_empty());
}

/// **The one that would be a way out.** A symbolic link at the leaf is not an
/// ancestor, so neither the domain's path test nor the adapter's ancestor gate
/// sees it — and writing through it would put the credential wherever it
/// points.
#[test]
fn a_symbolic_link_at_the_name_is_refused_rather_than_written_through() {
    let mut fake = installed().with(&format!("{DATA}/cert"), Node::symlink("/state/elsewhere"));
    let refused = refusal(install(
        &mut fake,
        &[SUBJECT, APP, "cert", HEX_FLAG, "00", CONFIRMATION],
    ));
    assert_eq!(refused.code(), ErrorCode::Unsafe);
    assert!(fake.did.is_empty());
    assert!(!fake.has("/state/elsewhere"));
}

#[test]
fn a_directory_at_the_name_is_refused() {
    let mut fake = installed().with(&format!("{DATA}/cert"), Node::directory());
    let refused = refusal(install(
        &mut fake,
        &[SUBJECT, APP, "cert", HEX_FLAG, "00", CONFIRMATION],
    ));
    assert_eq!(refused.code(), ErrorCode::Unsafe);
}

/// **A refusal must not quote the value it could not read.** The commonest
/// reason a credential is unreadable is that it is the right credential with
/// something stuck to it (`ADR-020` §4).
#[test]
fn a_value_that_is_not_hexadecimal_is_refused_without_being_printed() {
    for bad in ["0x00ff", "abc", "zz", "00 ff", "00-ff"] {
        let mut fake = installed();
        let refused = refusal(install(
            &mut fake,
            &[SUBJECT, APP, "cert", HEX_FLAG, bad, CONFIRMATION],
        ));
        assert_eq!(refused.code(), ErrorCode::Arguments, "{bad} was accepted");
        assert!(
            !refused.message().contains(bad),
            "the refusal quoted the value: {}",
            refused.message()
        );
        assert!(fake.did.is_empty());
    }
}

/// Upper case is accepted on the way in, which is `crate::secret`'s rule and
/// matters because a person retyping a fingerprint should not be refused over
/// case.
#[test]
fn upper_case_hexadecimal_is_accepted() {
    let mut fake = installed();
    install(
        &mut fake,
        &[SUBJECT, APP, "cert", HEX_FLAG, "DEADBEEF", CONFIRMATION],
    )
    .expect("upper case hex is hex");
    assert_eq!(
        fake.nodes[&format!("{DATA}/cert")].contents,
        vec![0xde, 0xad, 0xbe, 0xef]
    );
}

/// **The word-eating case.** `--hex confirm` must not install a secret whose
/// content is the confirmation, unconfirmed.
#[test]
fn the_flag_cannot_swallow_the_confirmation() {
    let mut fake = installed();
    let refused = refusal(install(
        &mut fake,
        &[SUBJECT, APP, "cert", HEX_FLAG, CONFIRMATION],
    ));
    assert_eq!(refused.code(), ErrorCode::Arguments);
    assert!(refused.message().contains("has been eaten"), "{}", refused.message());
    assert!(fake.did.is_empty());
}

#[test]
fn the_content_must_be_named_by_its_encoding() {
    let mut fake = installed();
    let refused = refusal(install(&mut fake, &[SUBJECT, APP, "cert", "deadbeef"]));
    assert_eq!(refused.code(), ErrorCode::Arguments);
    assert!(refused.message().contains(HEX_FLAG), "{}", refused.message());
}

/// An empty secret is a thing somebody may mean, and it has a spelling.
#[test]
fn an_empty_secret_is_installable_and_is_not_an_omission() {
    let mut fake = installed();
    let done = install(
        &mut fake,
        &[SUBJECT, APP, "cert", HEX_FLAG, "", CONFIRMATION],
    )
    .expect("an empty secret is a secret");
    assert_eq!(number(&done.value(), "bytes"), 0);
    assert_eq!(fake.nodes[&format!("{DATA}/cert")].contents, b"");
}

#[test]
fn a_secret_over_the_cap_is_refused_and_the_cap_is_named() {
    let mut fake = installed();
    let big = hex_of(&vec![0u8; SECRET_MAX_BYTES + 1]);
    let refused = refusal(install(
        &mut fake,
        &[SUBJECT, APP, "cert", HEX_FLAG, &big, CONFIRMATION],
    ));
    assert_eq!(refused.code(), ErrorCode::Arguments);
    assert!(
        refused.message().contains(&SECRET_MAX_BYTES.to_string()),
        "{}",
        refused.message()
    );
}

#[test]
fn an_application_name_that_is_not_a_service_name_is_refused() {
    let mut fake = installed();
    for app in ["../etc", "-x", "", "a/b"] {
        let refused = refusal(install(
            &mut fake,
            &[SUBJECT, app, "cert", HEX_FLAG, "00", CONFIRMATION],
        ));
        assert_eq!(refused.code(), ErrorCode::Arguments, "{app} was accepted");
    }
}

#[test]
fn a_trailing_word_that_is_not_the_confirmation_is_refused_by_name() {
    let mut fake = installed();
    let refused = refusal(install(
        &mut fake,
        &[SUBJECT, APP, "cert", HEX_FLAG, "00", "confrim"],
    ));
    assert_eq!(refused.code(), ErrorCode::Arguments);
    assert!(refused.message().contains("confirm"), "{}", refused.message());
    assert!(fake.did.is_empty());
}

// -- the machine's own refusals reach the caller unreworded -------------------

/// The act failing after the plan succeeded leaves the answer saying so, and
/// nothing is invented about what is on disk.
#[test]
fn a_machine_that_refuses_the_act_is_reported_as_a_machine_refusal() {
    let mut fake = installed();
    fake.act_fails = Some(crate::operation::machine::MachineError::Refused(
        "the filesystem is read-only".to_string(),
    ));
    let refused = refusal(install(
        &mut fake,
        &[SUBJECT, APP, "cert", HEX_FLAG, "00", CONFIRMATION],
    ));
    assert_eq!(refused.code(), ErrorCode::MachineRefused);
    assert!(refused.message().contains("read-only"), "{}", refused.message());
}

/// A journal that could not take the record is reported and is **not** a
/// refusal: the secret is on the machine either way, and saying nothing
/// happened would be the worse lie.
#[test]
fn a_record_that_could_not_be_filed_does_not_undo_the_install() {
    let mut fake = installed();
    fake.unrecorded = Some("the supervisor's socket was not there".to_string());
    let done = confirmed(&mut fake, "cert", b"\x00\x01");
    assert!(!done.journalled());
    assert!(text(&done.value(), "unrecorded").contains("socket"));
    assert_eq!(fake.nodes[&format!("{DATA}/cert")].contents, vec![0, 1]);
}

// -- helpers -----------------------------------------------------------------

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(fields) => fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v),
        _ => None,
    }
}

fn text(value: &Value, key: &str) -> String {
    match field(value, key) {
        Some(Value::Str(s)) => s.clone(),
        other => panic!("{key} is {other:?} and not a string"),
    }
}

fn number(value: &Value, key: &str) -> i64 {
    match field(value, key) {
        Some(Value::Integer(n)) => *n,
        other => panic!("{key} is {other:?} and not a number"),
    }
}

/// `PathStat` is constructed nowhere in this file; the fake builds them. Named
/// here so the import is used and a reader can see what the fake answers with.
#[allow(dead_code)]
fn stat_shape(_: &PathStat) {}
