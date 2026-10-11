//! Tests for `cargo rivet setup`.
//!
//! **The first one is a cold run**, and it is the point of the file: an empty
//! directory, the verb, and then the same survey the card writer will take —
//! asserting that a person who runs this and nothing else is no longer in the
//! state the whole task exists to remove.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

fn scratch(what: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    crate::scratch::absent(&format!(
        "setup-{what}-{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn cold(at: &Path) -> SetupArgs {
    SetupArgs {
        at: Some(at.to_path_buf()),
        name: Some(OPERATOR_NAME.to_owned()),
        check: false,
        over: Overrides::default(),
    }
}

/// **THE COLD RUN.** An empty directory in, a complete credential set out.
///
/// It asserts against [`Survey`] rather than against this verb's own words,
/// because the survey is what `cargo rivet provision` refuses on — a test that
/// checked the report would prove the two agree with each other and nothing
/// about whether a card can now be written (`AGENTS.md` §4).
#[test]
fn a_cold_run_leaves_a_host_with_every_required_credential() {
    let at = scratch("cold");
    let survey_before = Survey::take(&at, &Overrides::default());
    assert_eq!(survey_before.missing_required().len(), 4, "the fixture is not cold");

    run(&cold(&at)).expect("a cold run");

    let after = Survey::take(&at, &Overrides::default());
    assert!(
        after.refusal().is_none(),
        "still refused after setup: {:?}",
        after.missing_required().iter().map(|c| c.name).collect::<Vec<_>>()
    );

    // And the file that decides whether the board answers at all.
    let peers = std::fs::read_to_string(
        at.join("identity").join("board").join(credentials::PEERS_FILE),
    )
    .expect("a peers file");
    let operator = after.named(OPERATOR_CREDENTIAL).expect("in the set");
    assert_eq!(
        peers.trim(),
        operator.public_key().expect("a minted identity"),
        "the board's peers file must name the operator this run minted"
    );
}

/// **A person's own name reaches the certificate, spaces and all.**
///
/// The default is `fleet-owner`, and an operator replaces it with their own
/// name, such as `Your Name`. A name with a space in it is the case that breaks if anything on the path
/// splits on whitespace or sanitises a subject, and it is the case that
/// matters, because the string ends up on `rivet-proxyd`'s index as
/// *"Authenticated as: …"* and in the journal beside every change he makes.
///
/// Asserted by reading the `commonName` back out of the minted certificate
/// rather than out of this verb's report — the report is what the tool says it
/// did, and the certificate is what it did.
#[test]
fn an_operators_own_name_survives_into_the_certificate_spaces_included() {
    const WANTED: &str = "Your Name";

    let at = scratch("named");
    let mut args = cold(&at);
    args.name = Some(WANTED.to_owned());
    run(&args).expect("a cold run with a name");

    let der = std::fs::read(
        at.join("identity")
            .join("operator")
            .join(rivetos_tls::CERTIFICATE_FILE),
    )
    .expect("a minted certificate");
    assert_eq!(
        rivetos_tls::subject_common_name(&der).as_deref(),
        Some(WANTED),
        "the name typed is not the name in the certificate"
    );
}

/// The default is the one a person is offered when they say nothing, and it is
/// the maintainer's word rather than the tool's own name — `fleet-owner`
/// describes the role being taken.
#[test]
fn saying_nothing_takes_the_role_name_and_not_the_tools_name() {
    let at = scratch("default-name");
    let before = Survey::take(&at, &Overrides::default());
    // Not a terminal under `cargo test`, so this is the non-interactive path.
    assert_eq!(resolve_name(None, &before).expect("a name"), "fleet-owner");
}

/// **The two seeds are two seeds.** `ADR-028` §5.5 separates them so that a
/// machine able to ship an application cannot ship an operating system, and a
/// sequencer that wrote one file twice would quietly undo that.
#[test]
fn the_system_and_application_seeds_are_different_keys() {
    let at = scratch("two");
    run(&cold(&at)).expect("a cold run");
    let keys = at.join("keys");
    let system = std::fs::read(keys.join("rivet-system.seed")).expect("system");
    let app = std::fs::read(keys.join("rivet-app.seed")).expect("app");
    assert_eq!(system.len(), 32);
    assert_ne!(system, app, "one seed was written to both paths");

    // And each has its public half beside it, which is what a card is built to
    // trust and what the environment block at the end of the report names.
    for name in ["rivet-system.pub", "rivet-app.pub"] {
        let text = std::fs::read_to_string(keys.join(name)).expect(name);
        assert_eq!(text.trim().len(), 64, "{name}: {text}");
    }
}

/// **It never overwrites a key.** The single most important property here:
/// overwriting a seed orphans every board that trusts its public half and every
/// payload signed with it, and there is no undo.
#[test]
fn a_second_run_changes_nothing_on_disk() {
    let at = scratch("idempotent");
    run(&cold(&at)).expect("the first run");

    let before: Vec<Vec<u8>> = fingerprint(&at);
    let said = run(&cold(&at)).expect("the second run");
    let after: Vec<Vec<u8>> = fingerprint(&at);

    assert_eq!(before, after, "a second run rewrote something");
    assert!(!said.contains("MADE"), "a second run claimed to make something:\n{said}");
    assert!(said.contains("have"), "{said}");
}

/// Every secret and every public half, as bytes, for the comparison above.
fn fingerprint(at: &Path) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for path in [
        at.join("keys").join("rivet-system.seed"),
        at.join("keys").join("rivet-system.pub"),
        at.join("keys").join("rivet-app.seed"),
        at.join("keys").join("rivet-app.pub"),
        at.join("identity").join("operator").join(rivetos_tls::SEED_FILE),
        at.join("identity").join("operator").join(rivetos_tls::CERTIFICATE_FILE),
        at.join("identity").join("board").join(rivetos_tls::SEED_FILE),
        at.join("identity").join("board").join(rivetos_tls::CERTIFICATE_FILE),
        at.join("identity").join("board").join(credentials::PEERS_FILE),
    ] {
        out.push(std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())));
    }
    out
}

/// **An existing `peers` file that does not name you is left exactly as it
/// is.** Adding a key to a trust list is `AGENTS.md` §1.1's third row, so this
/// says what to add and stops.
#[test]
fn an_existing_peers_file_is_never_edited() {
    let at = scratch("peers");
    let board = at.join("identity").join("board");
    std::fs::create_dir_all(&board).expect("mkdir");
    let someone_else = "0".repeat(64);
    let peers = board.join(credentials::PEERS_FILE);
    std::fs::write(&peers, format!("{someone_else}\n")).expect("write");

    let said = run(&cold(&at)).expect("a run");

    assert_eq!(
        std::fs::read_to_string(&peers).expect("read"),
        format!("{someone_else}\n"),
        "the existing peers file was edited"
    );
    assert!(said.contains("does NOT name your key"), "{said}");
    // It names the line to add, so the person can act without going looking.
    let operator = Survey::take(&at, &Overrides::default())
        .named(OPERATOR_CREDENTIAL)
        .expect("in the set")
        .public_key()
        .expect("minted");
    assert!(said.contains(&operator), "{said}");
}

/// **A published key with no seed behind it stops the run**, rather than
/// getting a new seed minted beside it that it does not belong to.
///
/// That `.pub` is exactly what a card's trust store is composed from, so the
/// silent version of this is a board that refuses everything its owner signs —
/// the failure the whole verb exists to prevent, one directory earlier.
#[test]
fn a_public_key_with_no_seed_behind_it_is_refused_rather_than_minted_over() {
    let at = scratch("orphan");
    let keys = at.join("keys");
    std::fs::create_dir_all(&keys).expect("mkdir");
    let published = keys.join("rivet-system.pub");
    std::fs::write(&published, format!("{}\n", "a".repeat(64))).expect("write");

    let why = run(&cold(&at)).expect_err("an orphan public key must stop the run");
    assert!(why.contains("rivet-system.pub"), "{why}");
    assert!(why.contains("Move the public key aside"), "{why}");
    assert!(
        !keys.join("rivet-system.seed").exists(),
        "a seed was minted beside a public key it does not match"
    );
    assert_eq!(
        std::fs::read_to_string(&published).expect("read"),
        format!("{}\n", "a".repeat(64)),
        "the public key was touched"
    );
}

/// **`--check` writes nothing.** A verb whose reporting mode has a side effect
/// is one nobody can use to look.
#[test]
fn check_reports_and_mints_nothing() {
    let at = scratch("check");
    let said = run(&SetupArgs {
        at: Some(at.clone()),
        name: Some(OPERATOR_NAME.to_owned()),
        check: true,
        over: Overrides::default(),
    })
    .expect("a check on a cold host");

    assert!(!at.exists(), "--check created {}", at.display());
    assert!(said.contains("MISSING"), "{said}");
    assert!(said.contains("STILL MISSING"), "{said}");
}

/// **No seed reaches the report, on any path.** The one byte string in this
/// tool that must never be printed.
#[test]
fn the_report_never_contains_a_seed() {
    let at = scratch("secret");
    let said = run(&cold(&at)).expect("a cold run");
    for name in ["rivet-system.seed", "rivet-app.seed"] {
        let bytes = std::fs::read(at.join("keys").join(name)).expect(name);
        assert!(!said.contains(&crate::key::hex(&bytes)), "{name} was printed");
        for window in bytes.windows(8) {
            assert!(!said.contains(&crate::key::hex(window)), "part of {name} was printed");
        }
    }
    for who in ["operator", "board"] {
        let bytes = std::fs::read(at.join("identity").join(who).join(rivetos_tls::SEED_FILE))
            .expect("a device seed");
        assert!(!said.contains(&rivetos_tls::encode_hex(&bytes)), "{who}'s seed was printed");
    }
}

/// The report ends by naming the environment the system image build needs,
/// because a build given none of it composes a trust store out of this
/// checkout's committed public keys — which are not the ones this run just
/// minted.
#[test]
fn the_report_names_the_environment_a_card_build_needs() {
    let at = scratch("env");
    let said = run(&cold(&at)).expect("a cold run");
    for variable in [
        "RIVET_SYSTEM_SEED",
        "RIVET_APP_SEED",
        "RIVET_TRUSTED_KEY",
        "RIVET_APP_KEY",
    ] {
        assert!(said.contains(variable), "{variable} is not in the report:\n{said}");
    }
    assert!(said.contains("THE PIN IS THE BOARD'S PUBLIC KEY"), "{said}");
}

/// **The next steps publish once and provision per machine** — `ADR-054` §2.
/// Found 2026-09-14: they named `rivet-provision` and a card build per
/// machine, and a per-machine `RIVET_IDENTITY` for it, which R-175 ruled out.
#[test]
fn the_next_steps_publish_once_and_provision_from_the_repository() {
    let at = scratch("next");
    let said = run(&cold(&at)).expect("a cold run");
    let next = said.split("\nnext\n").nth(1).expect("a next block");
    for wanted in ["cargo rivet image", "cargo rivet fleet publish", "cargo rivet provision"] {
        assert!(next.contains(wanted), "{wanted} is not in the next steps:\n{next}");
    }
    assert!(!next.contains("RIVET_IDENTITY"), "a card is not built per machine:\n{next}");
    assert!(!next.contains("rivet-provision "), "the interview runs inside cargo rivet provision:\n{next}");
}

/// An unknown option is refused by name with the usage, rather than taken for a
/// path — the same rule every other verb here follows.
#[test]
fn an_unknown_option_is_refused_by_name() {
    let words: Vec<String> = ["--force"].iter().map(|s| (*s).to_string()).collect();
    let why = parse(&words).expect_err("must refuse");
    assert!(why.contains("--force"), "{why}");
    assert!(why.contains("usage:"), "{why}");
}

/// `--at` and `--name` take their values, and `--check` takes none.
#[test]
fn the_options_parse() {
    let words: Vec<String> = ["--at", "d", "--name", "me", "--check"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    let args = parse(&words).expect("parses");
    assert_eq!(args.at, Some(PathBuf::from("d")));
    assert_eq!(args.name.as_deref(), Some("me"));
    assert!(args.check);
    assert!(parse(&["--at".to_owned()]).is_err(), "--at with no value");
}

/// The usage says the promise that matters, in the words a person needs.
#[test]
fn the_usage_says_it_never_overwrites_a_key() {
    let text = usage();
    assert!(text.contains("NEVER OVERWRITES"), "{text}");
    assert!(text.contains("no undo"), "{text}");
}
