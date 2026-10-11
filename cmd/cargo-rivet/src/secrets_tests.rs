//! **What this verb refuses, and what it says while refusing.**
//!
//! Nothing here reaches a board. The three `apply install-secret` calls are
//! `crate::run`'s and the operation is `rivet_core`'s, both of which have their
//! own suites; what is asserted is the part this file added — finding the pair,
//! reading the files, and a plan that does not print a key.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

/// A real-shaped PEM certificate. The body is not a certificate and does not
/// need to be: nothing here parses one, and the check under test is the label.
const A_CERTIFICATE: &str =
    "-----BEGIN CERTIFICATE-----\nMIIBmarker\n-----END CERTIFICATE-----\n";
/// A PKCS#8 private key, the form `rcgen` writes.
const A_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMIGHsecret\n-----END PRIVATE KEY-----\n";

/// An authority directory with a root and one server pair.
fn an_authority() -> tempdir::TempDir {
    let dir = tempdir::TempDir::new();
    std::fs::write(dir.path().join(ROOT_FILE), A_CERTIFICATE).expect("root");
    std::fs::write(dir.path().join("server-rivet-01.local.crt"), A_CERTIFICATE).expect("crt");
    std::fs::write(dir.path().join("server-rivet-01.local.key"), A_KEY).expect("key");
    dir
}

fn args_for(dir: &std::path::Path, extra: &[&str]) -> SecretsArgs {
    let mut words: Vec<String> = vec![
        "aura-controller".to_owned(),
        "--authority".to_owned(),
        dir.display().to_string(),
    ];
    words.extend(extra.iter().map(|w| (*w).to_string()));
    parse(&words).expect("valid arguments")
}

// ---- Parsing --------------------------------------------------------------

#[test]
fn the_application_is_the_one_bare_word() {
    let parsed = parse(&["aura-controller".to_owned()]).expect("valid");
    assert_eq!(parsed.application, "aura-controller");
    assert!(!parsed.confirm);
}

#[test]
fn an_application_is_required() {
    let refused = parse(&[]).expect_err("nothing to install into");
    assert!(refused.contains("an application to install into is required"), "{refused}");
}

/// **Two bare words are a device and an application**, split before `parse`
/// sees them; one bare word is an application alone, and a flag or `confirm`
/// in second place is not the device form.
#[test]
fn a_leading_device_is_two_bare_words() {
    let words = |w: &[&str]| w.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let two = words(&["dev-01", "tempmon-rivet", "--name", "dev-01.local", "confirm"]);
    let (device, application, rest) = leading_device(&two).expect("the device form");
    assert_eq!((device, application), ("dev-01", "tempmon-rivet"));
    assert_eq!(rest, &two[2..]);

    assert!(leading_device(&words(&["tempmon-rivet", "--host", "pi:7777"])).is_none());
    assert!(leading_device(&words(&["tempmon-rivet", "confirm"])).is_none());
    assert!(leading_device(&words(&["tempmon-rivet"])).is_none());
    assert!(leading_device(&[]).is_none());
}

/// **One at a time.** Three secrets into two applications in one command is a
/// partial failure with no good answer, so it is not offered.
#[test]
fn a_second_bare_word_is_refused_and_names_the_first() {
    let refused = parse(&["aura-controller".to_owned(), "tempmon-rivet".to_owned()])
        .expect_err("two applications");
    assert!(refused.contains("ONE application at a time"), "{refused}");
    assert!(refused.contains("aura-controller"), "{refused}");
}

#[test]
fn the_endpoint_flags_are_read_and_confirm_is_a_word() {
    let parsed = parse(
        &["aura-controller", "--host", "board:7777", "--identity", "./id", "--pin", "ab", "confirm"]
            .map(str::to_owned),
    )
    .expect("valid");
    assert_eq!(parsed.endpoint.host.as_deref(), Some("board:7777"));
    assert!(parsed.confirm);
}

/// `ADR-025` §2's trust is mutual, and it is refused here rather than at the
/// first connection.
#[test]
fn an_identity_without_a_pin_is_refused_before_anything_is_read() {
    let refused =
        parse(&["aura-controller", "--host", "b:7777", "--identity", "./id"].map(str::to_owned))
            .expect_err("half a credential");
    assert!(refused.contains("pin"), "{refused}");
}

#[test]
fn a_flag_this_verb_does_not_take_is_named() {
    let refused = parse(&["aura-controller".to_owned(), "--rvt".to_owned()])
        .expect_err("not a flag here");
    assert!(refused.contains("`--rvt` is not a flag this verb takes"), "{refused}");
}

// ---- Finding the pair -----------------------------------------------------

#[test]
fn one_server_pair_in_the_directory_is_chosen_without_being_named() {
    let dir = an_authority();
    let (name, pieces) = gather(&args_for(dir.path(), &[])).expect("one pair");
    assert_eq!(name, "rivet-01.local");
    assert_eq!(pieces.len(), 3);
    assert_eq!(pieces[0].secret, CERTIFICATE);
    assert_eq!(pieces[1].secret, KEY);
    assert_eq!(pieces[2].secret, AUTHORITY);
}

/// **Several is a question, not a guess.** Installing another machine's
/// certificate is a handshake failure whose message is about a name nobody
/// typed.
#[test]
fn several_server_pairs_are_refused_and_all_of_them_are_listed() {
    let dir = an_authority();
    std::fs::write(dir.path().join("server-rivet-02.local.crt"), A_CERTIFICATE).expect("crt");
    std::fs::write(dir.path().join("server-rivet-02.local.key"), A_KEY).expect("key");

    let refused = gather(&args_for(dir.path(), &[])).expect_err("ambiguous");
    assert!(refused.contains("rivet-01.local"), "{refused}");
    assert!(refused.contains("rivet-02.local"), "{refused}");
    assert!(refused.contains("--name"), "{refused}");
}

#[test]
fn a_named_pair_is_taken_even_when_there_are_several() {
    let dir = an_authority();
    std::fs::write(dir.path().join("server-rivet-02.local.crt"), A_CERTIFICATE).expect("crt");
    std::fs::write(dir.path().join("server-rivet-02.local.key"), A_KEY).expect("key");

    let (name, _) = gather(&args_for(dir.path(), &["--name", "rivet-02.local"]))
        .expect("named");
    assert_eq!(name, "rivet-02.local");
}

/// An authority with no server certificate is a machine whose hostname was
/// never turned into one, and the refusal says which command does that.
#[test]
fn an_authority_with_no_server_certificate_names_the_command_that_makes_one() {
    let dir = tempdir::TempDir::new();
    std::fs::write(dir.path().join(ROOT_FILE), A_CERTIFICATE).expect("root");
    let refused = gather(&args_for(dir.path(), &[])).expect_err("no pair");
    assert!(refused.contains("rivet-provision ca server"), "{refused}");
}

#[test]
fn a_named_pair_that_is_not_there_says_what_it_looked_for() {
    let dir = an_authority();
    let refused = gather(&args_for(dir.path(), &["--name", "rivet-99.local"]))
        .expect_err("no such pair");
    assert!(refused.contains("server-rivet-99.local.crt"), "{refused}");
}

#[test]
fn a_directory_that_is_not_an_authority_says_so_rather_than_a_file_not_found() {
    let dir = tempdir::TempDir::new();
    let missing = dir.path().join("nowhere");
    let refused = gather(&args_for(&missing, &[])).expect_err("no directory");
    assert!(refused.contains("no authority directory here"), "{refused}");
    assert!(refused.contains("rivet-provision ca"), "{refused}");
}

// ---- Reading the files ----------------------------------------------------

/// **The mistake that gets past everything else.** A key that does not go with
/// its certificate is caught at start-up, by name; a certificate installed AS
/// the key parses fine at both ends and fails with a message about neither.
#[test]
fn a_certificate_where_the_key_should_be_is_refused_by_name() {
    let dir = an_authority();
    std::fs::write(dir.path().join("server-rivet-01.local.key"), A_CERTIFICATE).expect("swap");
    let refused = gather(&args_for(dir.path(), &[])).expect_err("swapped");
    assert!(refused.contains("does not look like a private key"), "{refused}");
    assert!(refused.contains("BEGIN PRIVATE KEY"), "{refused}");
}

#[test]
fn a_key_where_the_certificate_should_be_is_refused_by_name() {
    let dir = an_authority();
    std::fs::write(dir.path().join("server-rivet-01.local.crt"), A_KEY).expect("swap");
    let refused = gather(&args_for(dir.path(), &[])).expect_err("swapped");
    assert!(refused.contains("does not look like a certificate"), "{refused}");
}

/// The older PEM spellings are accepted: a key that came from somewhere other
/// than `rcgen` is still something rustls reads.
#[test]
fn the_older_private_key_spellings_are_accepted() {
    for label in ["EC PRIVATE KEY", "RSA PRIVATE KEY"] {
        let dir = an_authority();
        std::fs::write(
            dir.path().join("server-rivet-01.local.key"),
            format!("-----BEGIN {label}-----\nx\n-----END {label}-----\n"),
        )
        .expect("key");
        assert!(gather(&args_for(dir.path(), &[])).is_ok(), "{label} was refused");
    }
}

/// **An empty secret installs cleanly and refuses at start-up**, which is the
/// wrong end to find out.
#[test]
fn an_empty_file_is_refused_here_rather_than_at_start_up() {
    let dir = an_authority();
    std::fs::write(dir.path().join(ROOT_FILE), "").expect("empty");
    let refused = gather(&args_for(dir.path(), &[])).expect_err("empty");
    assert!(refused.contains("empty"), "{refused}");
}

#[test]
fn a_missing_root_names_the_file() {
    let dir = an_authority();
    std::fs::remove_file(dir.path().join(ROOT_FILE)).expect("remove");
    let refused = gather(&args_for(dir.path(), &[])).expect_err("no root");
    assert!(refused.contains(ROOT_FILE), "{refused}");
}

// ---- The plan -------------------------------------------------------------

/// **A plan is rendered to a terminal, returned over a control plane, and read
/// by an agent whose transcript outlives the session.** One of these three
/// files is a private key. `rivet_core::operation::install_secret` learned this
/// from a test and this is that test, one layer up.
#[test]
fn the_plan_never_contains_the_content_of_any_file() {
    let dir = an_authority();
    let args = args_for(dir.path(), &["--host", "board:7777"]);
    let (name, pieces) = gather(&args).expect("gathered");
    let rendered = plan(&args, &name, &pieces);

    assert!(!rendered.contains("MIGHsecret"), "the key reached the plan: {rendered}");
    assert!(!rendered.contains("MIIBmarker"), "a certificate reached the plan: {rendered}");
    assert!(!rendered.contains("BEGIN"), "PEM armour reached the plan: {rendered}");
}

/// **Nor does a `{:?}`.** A derived `Debug` would have printed the key the
/// first time one appeared in a diagnostic, and the first one to appear was a
/// test's own `expect_err`.
#[test]
fn a_debug_rendering_redacts_the_content_and_keeps_the_length() {
    let dir = an_authority();
    let (_, pieces) = gather(&args_for(dir.path(), &[])).expect("gathered");
    let key = pieces.iter().find(|p| p.secret == KEY).expect("the key");
    let rendered = format!("{key:?}");
    assert!(!rendered.contains("MIGHsecret"), "{rendered}");
    assert!(rendered.contains("redacted"), "{rendered}");
    assert!(rendered.contains(&key.content.len().to_string()), "{rendered}");
}

/// It names the lengths instead, which is what an operator compares against
/// what the machine reports back.
#[test]
fn the_plan_names_each_file_its_length_and_where_it_came_from() {
    let dir = an_authority();
    let args = args_for(dir.path(), &["--host", "board:7777"]);
    let (name, pieces) = gather(&args).expect("gathered");
    let rendered = plan(&args, &name, &pieces);

    for piece in &pieces {
        assert!(rendered.contains(piece.secret), "{rendered}");
        assert!(rendered.contains(&piece.content.len().to_string()), "{rendered}");
    }
    assert!(rendered.contains("aura-controller"), "{rendered}");
    assert!(rendered.contains("board:7777"), "{rendered}");
    assert!(rendered.contains("Nothing was sent"), "{rendered}");
}

/// The name in the certificate has to match the proxy's `upstream-name`, and
/// the plan is the one place a person can compare them before acting.
#[test]
fn the_plan_names_the_certificates_machine_and_how_to_check_it() {
    let dir = an_authority();
    let args = args_for(dir.path(), &["--host", "board:7777"]);
    let (name, pieces) = gather(&args).expect("gathered");
    let rendered = plan(&args, &name, &pieces);
    assert!(rendered.contains("rivet-01.local"), "{rendered}");
    assert!(rendered.contains("upstream-name"), "{rendered}");
    // **The file the check reads is where the supervisor writes it**, beside
    // `data/` and not in it — this line named the old place until 2026-09-26.
    assert!(rendered.contains("get file /state/app/rivet-proxyd/settings\n"), "{rendered}");
    assert!(!rendered.contains("rivet-proxyd/data/settings"), "{rendered}");
}

/// **Without `confirm`, nothing is sent** — and with no `--host` either, the
/// plan is still worth printing, because the files are on this host.
#[test]
fn without_confirm_it_plans_even_with_no_machine_named() {
    let dir = an_authority();
    let rendered = run(&args_for(dir.path(), &[])).expect("a plan needs no board");
    assert!(rendered.contains("a machine not yet named"), "{rendered}");
    assert!(rendered.contains("Nothing was sent"), "{rendered}");
}

/// With `confirm` and no `--host` there is nothing to send it to, and that is a
/// refusal rather than a plan printed as if it had acted.
#[test]
fn confirm_without_a_host_is_refused() {
    let dir = an_authority();
    let refused = run(&args_for(dir.path(), &["confirm"])).expect_err("nowhere to send it");
    assert!(refused.contains("--host is required"), "{refused}");
}

// ---- Usage ----------------------------------------------------------------

/// The three names are the fleet's, and the usage has to print all three or an
/// operator installing into a fifth application invents a fourth spelling.
#[test]
fn the_usage_names_all_three_files_and_the_restart_that_follows() {
    let text = usage();
    for name in [CERTIFICATE, KEY, AUTHORITY] {
        assert!(text.contains(name), "{name} is missing from the usage");
    }
    assert!(text.contains("apply service-restart"), "{text}");
    assert!(text.contains("does not restart"), "{text}");
}

/// A tiny temporary directory, removed on drop.
///
/// Written here rather than taken as a dependency: `tempfile` is not in this
/// workspace and one test module is not a reason to admit a crate (`ADR-009`).
mod tempdir {
    pub struct TempDir(std::path::PathBuf);

    impl TempDir {
        pub fn new() -> Self {
            // The process id and a counter, which is unique within a run and
            // between concurrent runs on one host.
            static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let count = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Self(crate::scratch::dir(&format!("secrets-{count}")))
        }

        pub fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
