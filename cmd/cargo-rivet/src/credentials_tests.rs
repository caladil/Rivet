//! Tests for the credential survey.
//!
//! **Every one of these runs against a scratch directory**, never against the
//! host's real `~/.rivet`. The survey takes its root and its overrides as
//! parameters for exactly this reason: a check whose answer depends on the
//! machine it runs on is one that passes on the maintainer's workstation and
//! fails on everybody else's, which is the failure the whole task is about.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

/// A directory nothing else is using.
///
/// The process id alone is not enough on Windows, which recycles them — the
/// same finding `identity.rs`'s own `scratch` records, and the same fix.
fn scratch(what: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0);
    crate::scratch::absent(&format!(
        "cred-{what}-{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn write(path: &Path, body: &[u8]) {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(path, body).expect("write");
}

/// RFC 8032 §7.1's first seed, so the derived public key is the RFC's and not
/// this project's — `AGENTS.md` §4 on fixtures by provenance.
const SEED: [u8; 32] = [
    0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c, 0xc4,
    0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae, 0x7f, 0x60,
];
const PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

/// **A completely cold host has all four, and the refusal says so.**
///
/// This is the state the whole task exists for: a clone, an empty home, and a
/// person about to build a card.
#[test]
fn a_cold_host_is_missing_every_required_credential() {
    let at = scratch("cold");
    let survey = Survey::take(&at, &Overrides::default());

    let missing = survey.missing_required();
    assert_eq!(missing.len(), 4, "{:?}", survey.items);

    let why = survey.refusal().expect("a cold host must be refused");
    assert!(why.starts_with("NOTHING WAS WRITTEN"), "{why}");
    // Each missing piece names the command that makes it — the half of a
    // refusal that decides whether a person can act on it.
    assert!(why.contains("cargo rivet key new"), "{why}");
    assert!(why.contains("cargo rivet identity new"), "{why}");
    assert!(why.contains("cargo rivet setup"), "{why}");
    // And it says why it is a refusal rather than a warning.
    assert!(why.contains("refuses you"), "{why}");
}

/// **The authority is advised and never refuses.** A board is completely
/// reachable from a command line without one; only the web portal needs it.
#[test]
fn the_certificate_authority_alone_does_not_refuse() {
    let at = scratch("advised");
    let survey = Survey::take(&at, &Overrides::default());
    let authority = survey
        .items
        .iter()
        .find(|c| c.shape == Shape::Authority)
        .expect("an authority in the set");
    assert_eq!(authority.need, Need::Advised);
    assert!(
        !survey.missing_required().iter().any(|c| c.shape == Shape::Authority),
        "the authority must never be in the refusal list"
    );
}

/// A complete set is not refused, and the report says `have` for each.
#[test]
fn a_complete_set_is_not_refused() {
    let at = scratch("complete");
    lay_out(&at);
    let survey = Survey::take(&at, &Overrides::default());
    assert!(
        survey.refusal().is_none(),
        "{:?}",
        survey.missing_required().iter().map(|c| c.name).collect::<Vec<_>>()
    );
    assert!(!survey.report().contains("MISSING"), "{}", survey.report());
}

/// Put a plausible, complete credential set on disk. The identities are files
/// of the right names rather than real certificates — presence is what
/// [`Survey`] answers, and the tests that need a real key mint one.
fn lay_out(at: &Path) {
    write(&at.join("keys").join("rivet-system.seed"), &SEED);
    write(&at.join("keys").join("rivet-app.seed"), &SEED);
    for who in ["operator", "board"] {
        let dir = at.join("identity").join(who);
        write(&dir.join(rivetos_tls::SEED_FILE), &SEED);
        write(&dir.join(rivetos_tls::CERTIFICATE_FILE), b"not a real DER");
    }
    write(&at.join("identity").join("board").join(PEERS_FILE), b"\n");
    write(&at.join("ca").join("root.crt"), b"not a real PEM");
}

/// **A half-written identity is reported as half-written**, because "there is
/// no directory" and "the directory is there and has no peers file" send a
/// person to two different places.
#[test]
fn an_identity_missing_only_its_peers_file_says_which_part_is_absent() {
    let at = scratch("halfway");
    lay_out(&at);
    std::fs::remove_file(at.join("identity").join("board").join(PEERS_FILE)).expect("rm");

    let survey = Survey::take(&at, &Overrides::default());
    let board = survey.named(BOARD_CREDENTIAL).expect("in the set");
    assert!(!board.present);
    assert_eq!(board.incomplete.as_deref(), Some("peers is not in it"));

    let why = survey.refusal().expect("refused");
    assert!(why.contains("peers is not in it"), "{why}");
}

/// A directory that is absent entirely carries no part-list, because naming
/// three absent files inside a directory that does not exist reads as though
/// the directory did.
#[test]
fn an_identity_that_is_wholly_absent_names_no_parts() {
    let at = scratch("absent");
    let survey = Survey::take(&at, &Overrides::default());
    let board = survey.named(BOARD_CREDENTIAL).expect("in the set");
    assert!(!board.present);
    assert_eq!(board.incomplete, None);
}

/// **An operator who keeps a seed elsewhere is not told they have none.** The
/// tools that consume these variables honour them, so a survey that did not
/// would refuse the maintainer's own machine.
#[test]
fn the_environment_overrides_where_a_credential_is_looked_for() {
    let at = scratch("override");
    let elsewhere = scratch("override-seed").join("mine.seed");
    write(&elsewhere, &SEED);

    let over = Overrides {
        app_seed: Some(elsewhere.clone()),
        ..Overrides::default()
    };
    let survey = Survey::take(&at, &over);
    let app = survey.named("application signing seed").expect("in the set");
    assert!(app.present, "the override was not honoured");
    assert_eq!(app.path, elsewhere);
    assert!(app.how.contains(&elsewhere.display().to_string()), "{}", app.how);
}

/// The public key of a seed is the RFC's, and comes back as hex.
#[test]
fn a_seeds_public_key_is_derived_and_never_the_seed() {
    let at = scratch("public");
    lay_out(&at);
    let survey = Survey::take(&at, &Overrides::default());
    let seed = survey.named("system signing seed").expect("in the set");
    let key = seed.public_key().expect("a valid seed");
    assert_eq!(key, PUBLIC);
    assert_ne!(key, crate::key::hex(&SEED), "the seed itself was returned");
}

/// `agrees` is the check behind the trust-store note: does the public half a
/// card was built to trust belong to the seed this host holds?
#[test]
fn agrees_says_whether_a_published_key_belongs_to_a_seed() {
    let at = scratch("agrees");
    let seed = at.join("rivet-app.seed");
    write(&seed, &SEED);

    let mine = at.join("mine.pub");
    write(&mine, format!("{PUBLIC}\n").as_bytes());
    assert!(agrees(&seed, &mine).expect("both readable"));

    let theirs = at.join("theirs.pub");
    write(&theirs, b"0000000000000000000000000000000000000000000000000000000000000000\n");
    assert!(!agrees(&seed, &theirs).expect("both readable"));

    // A published key that is not there is an error and never a silent `true`
    // — `AGENTS.md` §7, a probe that cannot tell an error from a negative.
    assert!(agrees(&seed, &at.join("nothing.pub")).is_err());
}

/// **`peers` is spelled in two crates and this is the tie.**
///
/// `cmd/rivet-image`'s `boot::IDENTITY_PEERS_FILE` is the authority for the
/// name, and this crate does not depend on that one. Two spellings of one fact
/// is `ADR-011`, so the value is asserted here rather than left to agree by
/// inspection — the same guard `key.rs` puts on `PUBLIC_KEY_BYTES`.
#[test]
fn the_peers_filename_is_the_one_the_image_writer_uses() {
    assert_eq!(PEERS_FILE, "peers");
    assert_eq!(rivetos_tls::SEED_FILE, "device.seed");
    assert_eq!(rivetos_tls::CERTIFICATE_FILE, "device.crt");
}

/// No refusal, report or `how` line may carry a seed. The whole set is walked
/// so a credential added later is covered without anybody remembering to.
#[test]
fn nothing_the_survey_prints_contains_a_seed() {
    let at = scratch("secret");
    lay_out(&at);
    let survey = Survey::take(&at, &Overrides::default());
    let text = format!("{}{}", survey.report(), survey.refusal().unwrap_or_default());
    assert!(!text.contains(&crate::key::hex(&SEED)), "a seed reached the output");
    for window in SEED.windows(8) {
        assert!(!text.contains(&crate::key::hex(window)), "part of a seed reached the output");
    }
}

// ---------------------------------------------------------------------------
// `system_seed` — R-177's `cargo rivet image`, sibling to `app_seed`.
// ---------------------------------------------------------------------------

/// **The environment wins, exactly as it does for `app_seed`.** A path named
/// by `$RIVET_SYSTEM_SEED` is returned whether or not a file is there yet —
/// the operator named it, and the reader of the seed is what says what is
/// wrong with it.
#[test]
fn system_seed_prefers_the_environment_over_the_default() {
    let at = scratch("system-seed-env");
    let elsewhere = at.join("elsewhere.seed");
    let over = Overrides { system_seed: Some(elsewhere.clone()), ..Overrides::default() };
    assert_eq!(system_seed(&over, Some(&at)), Ok(elsewhere));
}

/// With no override, the default is returned only when it is actually a file.
#[test]
fn system_seed_falls_back_to_the_default_path_only_once_it_exists() {
    let at = scratch("system-seed-default");
    let refused = system_seed(&Overrides::default(), Some(&at));
    assert_eq!(refused, Err(Some(at.join("keys").join(SYSTEM_SEED_FILE))));

    write(&at.join("keys").join(SYSTEM_SEED_FILE), &SEED);
    assert_eq!(
        system_seed(&Overrides::default(), Some(&at)),
        Ok(at.join("keys").join(SYSTEM_SEED_FILE))
    );
}

/// With no `rivet_root` to compose a default from, the refusal carries `None`
/// rather than a path that was never actually looked at.
#[test]
fn system_seed_with_no_root_refuses_naming_no_default() {
    assert_eq!(system_seed(&Overrides::default(), None), Err(None));
}

/// `system_seed` and `app_seed` never share a file name, which is the whole
/// point of `ADR-028` §5's two keys.
#[test]
fn system_seed_and_app_seed_are_different_files() {
    assert_ne!(SYSTEM_SEED_FILE, APP_SEED_FILE);
}
