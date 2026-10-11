//! Tests for `super` — the interactive fleet deploy interview.
//!
//! **Hermetic on purpose.** No real register and no real board: a fake
//! [`Remote`] stands in for the control plane, and every terminal-reading
//! entry point takes `is_terminal` as a parameter precisely so a test can
//! drive it without a real stdin (`write_image::choose_with`'s own reason).
//! The manifest reader and the register enumeration are NOT exercised here —
//! both need real files or real keys this suite must never touch (the
//! maintainer's own rule: never read his real register).
//!
//! Covers: [`read_host_state_for`] mapping the fake board's reads into
//! [`HostState`] (installed vs. absent data directory, every [`Standing`]);
//! [`parse_approval`] (Enter takes the danger-class default, `y`/`yes`
//! accepts, everything else declines); and [`parse_multi_select`] (a numbers
//! string to the right zero-based indices, duplicates dropped, an
//! out-of-range or unparsable token refused outright).

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::path::Path;

use rivet_remote::deploy::{ConfigChange, Standing};

use super::*;

// ---------------------------------------------------------------------------
// a fake board — `rivet_remote::deploy`'s own test fakes are the model
// ---------------------------------------------------------------------------

/// Just enough of [`Remote`] for [`read_host_state_for`]: a data directory and a
/// configuration document. Every other method is a fake that refuses,
/// exactly as `rivet_remote::deploy`'s own `EnablingBoard` leaves the methods
/// a given test does not exercise — a fake that quietly answered them would
/// be claiming this suite covers a path it does not.
struct FakeBoard {
    data_dir: Option<Vec<String>>,
    document: Option<String>,
    /// Whether anything asked about the data directory — an artifact that
    /// declares no material is planned without asking.
    data_dir_asked: std::cell::Cell<bool>,
    /// Every word list [`Remote::perform_operation`] was called with, in
    /// order — what [`execute_step`]'s `ServiceRestart`/`RestartReader` arms
    /// are proved to reach instead of [`Remote::ask`] (Fix 1).
    performed: std::cell::RefCell<Vec<Vec<String>>>,
}

impl Remote for FakeBoard {
    fn push(&mut self, _local: &Path, _remote_path: &str) -> Result<String, String> {
        Err("this fake does not push".to_string())
    }
    fn blake3(&mut self, _remote_path: &str) -> Result<[u8; 32], String> {
        Err("this fake does not hash".to_string())
    }
    fn install(&mut self, _digest: &str) -> Result<String, String> {
        Err("this fake does not install".to_string())
    }
    fn read_document(&mut self) -> Result<Option<String>, String> {
        Ok(self.document.clone())
    }
    /// **Actually applies the edit**, unlike every other mutation this fake
    /// refuses — Fix 1/2's tests need a genuinely FRESH `Enable` (one that
    /// really does call this, rather than short-circuiting on
    /// `Standing::Enabled`) to succeed, so that `execute_host_batch`'s
    /// `any_enable_succeeded` trigger has something real behind it. Every
    /// other test in this suite that exercises `Enable` still sidesteps this
    /// by pre-seeding `document` with `enabled = true`, exactly as before.
    fn edit_document(&mut self, change: &ConfigChange) -> Result<String, String> {
        let mut doc = self.document.clone().unwrap_or_default();
        match change {
            ConfigChange::Append { text } => doc.push_str(text),
            ConfigChange::Replace { expect, replace } => {
                if !doc.contains(expect.as_str()) {
                    return Err(format!("fake: {expect:?} not found in the document"));
                }
                doc = doc.replacen(expect.as_str(), replace.as_str(), 1);
            }
        }
        self.document = Some(doc);
        Ok(String::new())
    }
    fn put_document(&mut self, _text: &str) -> Result<(), String> {
        Err("this fake does not put".to_string())
    }
    fn ask(&mut self, words: &[&str]) -> Result<String, String> {
        Err(format!("this fake does not run commands as a word list: {}", words.join(" ")))
    }
    fn data_directory_files(&mut self, _application: &str) -> Result<Option<Vec<String>>, String> {
        self.data_dir_asked.set(true);
        Ok(self.data_dir.clone())
    }
    /// **Overridden, unlike [`ask`](Remote::ask) above** — a real `Tls`
    /// routes a restart through this and not `ask` (Fix 1), and a fake that
    /// left this on the trait's default (`self.ask(words)`) would make a
    /// test of that indistinguishable from the bug it fixes.
    fn perform_operation(&mut self, words: &[&str]) -> Result<String, String> {
        self.performed.borrow_mut().push(words.iter().map(|w| (*w).to_string()).collect());
        Ok("performed".to_string())
    }
}

impl FakeBoard {
    fn new(data_dir: Option<Vec<String>>, document: Option<String>) -> Self {
        Self {
            data_dir,
            document,
            data_dir_asked: std::cell::Cell::new(false),
            performed: std::cell::RefCell::new(Vec::new()),
        }
    }
}

// ---------------------------------------------------------------------------
// read_host_state_for
// ---------------------------------------------------------------------------

/// `notes-rivet`, declaring `fleet-tls` — so its data directory is asked
/// about, which is what the data-directory tests below are about.
fn needing_material() -> ArtifactPlan {
    let mut artifact = plan_for("notes-rivet", "digest-1");
    artifact.material_kinds = vec!["fleet-tls".to_string()];
    artifact.material_files = ["tls-cert", "tls-key", "tls-root"].map(String::from).to_vec();
    artifact
}

#[test]
fn read_host_state_reports_an_absent_data_directory_as_not_installed() {
    let mut board = FakeBoard::new(None, None);
    let (state, _) = read_host_state_for(&mut board, &needing_material()).expect("reads");
    assert_eq!(state.data_dir_files, None);
}

#[test]
fn read_host_state_reports_an_installed_apps_own_data_directory() {
    let mut board = FakeBoard::new(Some(vec!["tls-cert".to_string()]), None);
    let (state, _) = read_host_state_for(&mut board, &needing_material()).expect("reads");
    assert_eq!(state.data_dir_files, Some(vec!["tls-cert".to_string()]));
}

/// **An artifact declaring no material never asks about its data
/// directory** — `deploy_plan::plan` reads it for nothing else, and the one
/// reading both front ends share must not cost the machine a round trip the
/// install verb never used to make.
#[test]
fn read_host_state_asks_nothing_about_a_data_directory_nobody_declared_material_for() {
    let mut board = FakeBoard::new(Some(vec!["anything".to_string()]), Some(String::new()));
    let (state, _) = read_host_state_for(&mut board, &plan_for("notes-rivet", "d")).expect("reads");
    assert!(!board.data_dir_asked.get(), "the data directory was asked about");
    assert_eq!(state.data_dir_files, None);
}

#[test]
fn read_host_state_reads_absent_standing_when_the_document_names_no_such_service() {
    let mut board = FakeBoard::new(None, Some(String::new()));
    let (state, _) = read_host_state_for(&mut board, &needing_material()).expect("reads");
    assert_eq!(state.standing, Standing::Absent);
}

#[test]
fn read_host_state_reads_enabled_standing_from_the_document() {
    let doc = "[services.notes-rivet]\nenabled = true\n";
    let mut board = FakeBoard::new(None, Some(doc.to_string()));
    let (state, document) = read_host_state_for(&mut board, &needing_material()).expect("reads");
    assert_eq!(state.standing, Standing::Enabled);
    assert_eq!(document, doc, "the document is handed back for the caller's other reads");
}

#[test]
fn read_host_state_reads_disabled_standing_from_the_document() {
    let doc = "[services.notes-rivet]\nenabled = false\n";
    let mut board = FakeBoard::new(None, Some(doc.to_string()));
    let (state, _) = read_host_state_for(&mut board, &needing_material()).expect("reads");
    assert_eq!(state.standing, Standing::Disabled);
}

#[test]
fn read_host_state_reads_unreadable_standing_from_a_hand_written_document() {
    let doc = "[services.notes-rivet]\n# no canonical enabled line before the next table\n\
               [services.other]\nenabled = true\n";
    let mut board = FakeBoard::new(None, Some(doc.to_string()));
    let (state, _) = read_host_state_for(&mut board, &needing_material()).expect("reads");
    assert_eq!(state.standing, Standing::Unreadable);
}

/// **Phase 1's own deviation, pinned by a test** — see the module header.
/// Even a data directory and a document that a real board could answer
/// "what digest is this" from are not consulted; the field is
/// unconditionally `None` until Phase 2's exact read exists.
#[test]
fn read_host_state_never_reports_a_current_digest_in_phase_1() {
    let mut board = FakeBoard::new(
        Some(vec!["anything".to_string()]),
        Some("[services.notes-rivet]\nenabled = true\n".to_string()),
    );
    let (state, _) = read_host_state_for(&mut board, &needing_material()).expect("reads");
    assert_eq!(state.current_digest, None);
}

/// **The other Phase 1 deviation, pinned the same way**: `settings_present` is
/// the artifact's own declared settings, so `plan` never offers a
/// `DeliverSetting` step nothing could carry out — applied inside the one
/// reading, so neither front end can forget it.
#[test]
fn read_host_state_seeds_settings_present_with_the_artifacts_declared_settings() {
    let mut board = FakeBoard::new(None, None);
    let mut artifact = needing_material();
    artifact.settings = vec!["timezone".to_string()];
    let (state, _) = read_host_state_for(&mut board, &artifact).expect("reads");
    assert_eq!(state.settings_present, vec!["timezone".to_string()]);
    assert!(
        deploy_plan::plan(&artifact, &state)
            .iter()
            .all(|s| !matches!(s.kind, StepKind::DeliverSetting { .. })),
        "a setting nothing can deliver was offered"
    );
}

// ---------------------------------------------------------------------------
// parse_approval
// ---------------------------------------------------------------------------

#[test]
fn parse_approval_takes_the_safe_default_on_a_bare_enter() {
    assert!(parse_approval("", Danger::Safe));
    assert!(parse_approval("\n", Danger::Safe));
    assert!(parse_approval("   ", Danger::Safe));
}

#[test]
fn parse_approval_takes_the_dangerous_default_on_a_bare_enter() {
    assert!(!parse_approval("", Danger::Dangerous));
    assert!(!parse_approval("\n", Danger::Dangerous));
}

#[test]
fn parse_approval_accepts_y_or_yes_case_insensitively() {
    for answer in ["y", "Y", "yes", "YES", "Yes"] {
        assert!(parse_approval(answer, Danger::Dangerous), "{answer:?} should accept");
        assert!(parse_approval(answer, Danger::Safe), "{answer:?} should accept");
    }
}

#[test]
fn parse_approval_declines_anything_else_for_a_dangerous_step() {
    for answer in ["n", "N", "no", "nope", "maybe", "yesplease"] {
        assert!(!parse_approval(answer, Danger::Dangerous), "{answer:?} should decline");
    }
}

/// The default is yes for a safe step, but a typed answer is still read —
/// `Enter` is the one shortcut, not "anything but `y` is fine".
#[test]
fn parse_approval_declines_an_explicit_no_on_a_safe_step_too() {
    assert!(!parse_approval("n", Danger::Safe));
}

// ---------------------------------------------------------------------------
// parse_multi_select
// ---------------------------------------------------------------------------

#[test]
fn parse_multi_select_reads_comma_and_space_separated_numbers() {
    assert_eq!(parse_multi_select("1,3", 5).expect("valid"), Some(vec![0, 2]));
    assert_eq!(parse_multi_select("1 3 5", 5).expect("valid"), Some(vec![0, 2, 4]));
    assert_eq!(parse_multi_select(" 2,  4 ", 5).expect("valid"), Some(vec![1, 3]));
}

#[test]
fn parse_multi_select_drops_duplicates_and_keeps_first_seen_order() {
    assert_eq!(parse_multi_select("3,1,3", 5).expect("valid"), Some(vec![2, 0]));
}

#[test]
fn parse_multi_select_refuses_a_number_out_of_range() {
    let err = parse_multi_select("1,9", 5).expect_err("9 is out of range");
    assert!(err.contains('9'), "{err}");
}

#[test]
fn parse_multi_select_refuses_zero_because_the_menu_is_one_based() {
    let err = parse_multi_select("0", 5).expect_err("zero is not one-based");
    assert!(err.contains('0'), "{err}");
}

/// **`x` is only the escape hatch alone.** Mixed with a number, it is a
/// token like any other stray one and is refused rather than silently
/// dropped or taken as an exit.
#[test]
fn parse_multi_select_refuses_a_non_number_token_rather_than_dropping_it() {
    let err = parse_multi_select("1,x,3", 5).expect_err("x is not a number");
    assert!(err.contains('x'), "{err}");
    assert!(err.contains("or x alone to exit"), "{err}");
}

#[test]
fn parse_multi_select_refuses_an_empty_answer() {
    let err = parse_multi_select("   ", 5).expect_err("nothing was typed");
    assert!(err.contains("nothing"), "{err}");
}

/// **The whole trimmed answer, and only the whole answer, is the exit.**
/// Case-insensitive, and tolerant of the same surrounding whitespace a
/// number already is.
#[test]
fn parse_multi_select_x_alone_exits() {
    assert_eq!(parse_multi_select("x", 5), Ok(None));
    assert_eq!(parse_multi_select("X", 5), Ok(None));
    assert_eq!(parse_multi_select("  x  ", 5), Ok(None));
}

// ---------------------------------------------------------------------------
// parse_kiosk_auto_run_choice / strip_multiple_kiosk_candidates — R-214
// ---------------------------------------------------------------------------

#[test]
fn parse_kiosk_auto_run_choice_picks_a_candidate_by_number() {
    let candidates = ["aura-renderer".to_string(), "kiosk-demo".to_string()];
    assert_eq!(
        parse_kiosk_auto_run_choice("1", &candidates).expect("valid"),
        Some("aura-renderer".to_string())
    );
    assert_eq!(
        parse_kiosk_auto_run_choice("2", &candidates).expect("valid"),
        Some("kiosk-demo".to_string())
    );
}

/// **The "or none" half.** The option one past the last candidate means
/// leave `[kiosk] auto-run` alone.
#[test]
fn parse_kiosk_auto_run_choice_reads_the_last_option_as_none() {
    let candidates = ["aura-renderer".to_string(), "kiosk-demo".to_string()];
    assert_eq!(parse_kiosk_auto_run_choice("3", &candidates).expect("valid"), None);
}

#[test]
fn parse_kiosk_auto_run_choice_refuses_out_of_range_and_non_numeric() {
    let candidates = ["aura-renderer".to_string()];
    assert!(parse_kiosk_auto_run_choice("0", &candidates).is_err());
    assert!(parse_kiosk_auto_run_choice("3", &candidates).is_err(), "1 candidate + none = 2 options");
    assert!(parse_kiosk_auto_run_choice("banana", &candidates).is_err());
}

/// **`x`/`X` still refuses — there is no candidate to hand back — but with
/// the wording a person asked to leave, never the parse-failure wording a
/// person who mistyped a number gets.**
#[test]
fn x_or_capital_x_refuses_with_an_exit_message_not_a_parse_failure() {
    let candidates = ["aura-renderer".to_string()];
    for typed in ["x", "X"] {
        let err = parse_kiosk_auto_run_choice(typed, &candidates)
            .expect_err("x must not choose a candidate or none");
        assert!(err.contains("exited"), "{err}");
        assert!(!err.contains("is not a number"), "{err}");
        assert!(err.contains("nothing was deployed to this host"), "{err}");
    }
}

/// **The defect this exists to prevent, proven directly.** Two artifacts in
/// one pass each offer `SetKioskAutoRun`; without stripping, approving both
/// steps in the per-artifact loop would let the second one silently
/// overwrite the first's edit. Stripping removes both and hands back both
/// names, so the caller can ask once instead.
#[test]
fn two_kiosk_candidates_are_both_stripped_and_both_returned() {
    let mut host_steps = vec![
        (
            entry("aura-renderer", "1.0.0"),
            plan_for("aura-renderer", "d1"),
            vec![
                Step { kind: StepKind::Enable { service: "aura-renderer".to_string() }, danger: Danger::Safe },
                Step {
                    kind: StepKind::SetKioskAutoRun { application: "aura-renderer".to_string() },
                    danger: Danger::Safe,
                },
            ],
            None,
        ),
        (
            entry("kiosk-demo", "1.0.0"),
            plan_for("kiosk-demo", "d2"),
            vec![
                Step { kind: StepKind::Enable { service: "kiosk-demo".to_string() }, danger: Danger::Safe },
                Step {
                    kind: StepKind::SetKioskAutoRun { application: "kiosk-demo".to_string() },
                    danger: Danger::Safe,
                },
            ],
            None,
        ),
    ];

    let candidates = strip_multiple_kiosk_candidates(&mut host_steps);

    assert_eq!(candidates, vec!["aura-renderer".to_string(), "kiosk-demo".to_string()]);
    for (_, _, steps, _) in &host_steps {
        assert!(
            !steps.iter().any(|s| matches!(s.kind, StepKind::SetKioskAutoRun { .. })),
            "every per-artifact occurrence must be stripped: {steps:?}"
        );
        // Nothing else in that artifact's own steps is touched.
        assert!(steps.iter().any(|s| matches!(s.kind, StepKind::Enable { .. })));
    }
}

/// **Exactly one candidate is left alone entirely** — the ordinary case
/// `deploy_plan::plan` already places correctly needs no consolidation.
#[test]
fn a_single_kiosk_candidate_is_not_stripped() {
    let mut host_steps = vec![(
        entry("aura-renderer", "1.0.0"),
        plan_for("aura-renderer", "d1"),
        vec![Step {
            kind: StepKind::SetKioskAutoRun { application: "aura-renderer".to_string() },
            danger: Danger::Safe,
        }],
        None,
    )];

    let candidates = strip_multiple_kiosk_candidates(&mut host_steps);

    assert_eq!(candidates, vec!["aura-renderer".to_string()]);
    assert!(
        host_steps[0].2.iter().any(|s| matches!(s.kind, StepKind::SetKioskAutoRun { .. })),
        "the sole candidate stays in its own artifact's step list"
    );
}

/// No candidate at all is the ordinary, unremarkable case.
#[test]
fn no_kiosk_candidate_strips_nothing() {
    let mut host_steps = vec![(
        entry("tempmon-rivet", "1.0.0"),
        plan_for("tempmon-rivet", "d1"),
        vec![Step { kind: StepKind::Enable { service: "tempmon-rivet".to_string() }, danger: Danger::Safe }],
        None,
    )];
    assert!(strip_multiple_kiosk_candidates(&mut host_steps).is_empty());
}

// ---------------------------------------------------------------------------
// the terminal refusal — no register or board touched to prove it
// ---------------------------------------------------------------------------

/// `run_with` refuses before it reads the fleet repository, the register or
/// anything else, so this is safe to run with no fixtures at all.
#[test]
fn run_with_refuses_cleanly_with_no_terminal_and_names_the_scripted_alternative() {
    let err = run_with(false).expect_err("no terminal");
    assert!(err.contains("cargo rivet install"), "{err}");
}

/// Same for the plain multi-select and approve prompts, each on its own —
/// [`run_with`]'s refusal is not the only path that can be asked with no
/// terminal.
#[test]
fn multi_choose_with_refuses_cleanly_with_no_terminal() {
    let options = vec!["one".to_string(), "two".to_string()];
    let err = multi_choose_with(false, "things", &options, "cargo rivet install")
        .expect_err("no terminal");
    assert!(err.contains("cargo rivet install"), "{err}");
}

#[test]
fn approve_with_refuses_cleanly_with_no_terminal() {
    let err = approve_with(false, "a step", Danger::Safe).expect_err("no terminal");
    assert!(err.contains("cargo rivet install"), "{err}");
}

// ---------------------------------------------------------------------------
// describe — the plan preview and approve-prompt text
// ---------------------------------------------------------------------------

#[test]
fn describe_names_the_service_for_enable_and_restart_steps() {
    assert!(describe(&StepKind::Enable { service: "notes-rivet".to_string() })
        .contains("notes-rivet"));
    assert!(describe(&StepKind::ServiceRestart { service: "notes-rivet".to_string() })
        .contains("notes-rivet"));
}

#[test]
fn describe_marks_a_deferred_setting_as_such() {
    let text = describe(&StepKind::DeliverSetting { name: "timezone".to_string() });
    assert!(text.contains("timezone"), "{text}");
    assert!(text.contains("Phase 2"), "{text}");
}

// ---------------------------------------------------------------------------
// step_outcome — Defect B's concise per-step reporting, and its
// "already running" special case (Defect B follow-up)
// ---------------------------------------------------------------------------

#[test]
fn step_outcome_reports_an_ordinary_install_as_installed() {
    let (label, is_failure) =
        step_outcome(&StepKind::Install { digest: "d".to_string() }, &Ok("install requested".to_string()));
    assert_eq!(label, "installed");
    assert!(!is_failure);
}

/// **The special case.** The control plane's own `Ok(..)` report embeds
/// `rivet_core::deploy::ALREADY_RUNNING`'s sentence when the host already
/// holds the digest asked for — `rivet_remote::deploy::run`'s own ordinary
/// path for it. This must read as a no-op, not an install.
#[test]
fn step_outcome_reports_an_already_running_install_ok_as_skipped_not_installed() {
    let said = format!("running  dev-01 is {}, so NO INSTALL WAS REQUESTED.", rivet_core::deploy::ALREADY_RUNNING);
    let (label, is_failure) = step_outcome(&StepKind::Install { digest: "d".to_string() }, &Ok(said));
    assert_eq!(label, "already current — skipped");
    assert!(!is_failure);
}

/// The same marker on an `Err` (defensive: it should never reach here as an
/// `Err` given `rivet_remote::deploy::run`'s own handling, but this is the
/// exact case the coordinator's ruling named) is still a no-op, never a
/// `FAILED` and never the thing that triggers a skip confirmation.
#[test]
fn step_outcome_reports_an_already_running_install_err_as_skipped_not_failed() {
    let err = format!("REFUSED: {} — nothing was installed.", rivet_core::deploy::ALREADY_RUNNING);
    let (label, is_failure) = step_outcome(&StepKind::Install { digest: "d".to_string() }, &Err(err));
    assert_eq!(label, "already current — skipped");
    assert!(!is_failure);
}

#[test]
fn step_outcome_reports_a_successful_non_install_step_as_done() {
    let (label, is_failure) = step_outcome(
        &StepKind::Enable { service: "notes-rivet".to_string() },
        &Ok("enabled".to_string()),
    );
    assert_eq!(label, "done");
    assert!(!is_failure);
}

/// A genuine failure is reported concisely — the ONE-LINE reason, never the
/// control plane's own multi-line/JSON response — and counts as a failure.
#[test]
fn step_outcome_reports_a_genuine_failure_concisely_and_as_a_failure() {
    let err = "the payload on the board is NOT the payload on this machine.\n here  aaaa\n there bbbb\nSend it again.".to_string();
    let (label, is_failure) =
        step_outcome(&StepKind::ServiceRestart { service: "notes-rivet".to_string() }, &Err(err));
    assert_eq!(label, "FAILED: the payload on the board is NOT the payload on this machine.");
    assert!(is_failure);
    assert!(!label.contains('\n'), "{label}");
}

// ---------------------------------------------------------------------------
// execute_step — Fix 1: a restart step reaches `Remote::perform_operation`,
// never `Remote::ask`. An MCP-backed board (`crate::tls::Tls`) refuses a word
// list through `ask` by design, so before this fix the installed+enabled
// service was never actually restarted — `FakeBoard::ask` above still errors
// on every word list, so this fails loudly if a restart step ever regresses
// to calling it again.
// ---------------------------------------------------------------------------

#[test]
fn a_service_restart_step_is_executed_through_perform_operation_not_ask() {
    let mut board = FakeBoard::new(None, None);
    let parsed = no_op_deploy_args();
    let root = Path::new(".");
    let artifact = plan_for("notes-rivet", "digest-1");

    let mut approve = never_approve();
    let result = execute_step(
        &mut board,
        &parsed,
        root,
        &artifact,
        &StepKind::ServiceRestart { service: "notes-rivet".to_string() },
        &mut approve,
    );

    assert_eq!(result, Ok("performed".to_string()), "{result:?}");
    assert_eq!(
        *board.performed.borrow(),
        vec![vec![
            "apply".to_string(),
            "service-restart".to_string(),
            "notes-rivet".to_string(),
            "confirm".to_string(),
        ]]
    );
}

#[test]
fn a_proxy_restart_step_is_executed_through_perform_operation_not_ask() {
    let mut board = FakeBoard::new(None, None);
    let parsed = no_op_deploy_args();
    let root = Path::new(".");
    let artifact = plan_for("notes-rivet", "digest-1");

    let mut approve = never_approve();
    let kind = StepKind::RestartReader {
        service: "rivet-proxyd".to_string(),
        reason: "this pass changes what rivet-proxyd serves, and its route needs to be \
                  re-derived"
            .to_string(),
        cost: "every application behind the machine's front door is unreachable while it restarts",
    };
    let result = execute_step(&mut board, &parsed, root, &artifact, &kind, &mut approve);

    assert_eq!(result, Ok("performed".to_string()), "{result:?}");
    assert_eq!(
        *board.performed.borrow(),
        vec![vec![
            "apply".to_string(),
            "service-restart".to_string(),
            "rivet-proxyd".to_string(),
            "confirm".to_string(),
        ]]
    );
}

/// An `approve` closure for `execute_step` calls that must never actually ask
/// anything — every arm exercised with it either does not prompt at all, or
/// the test means to prove it does not reach Fix 3's mint offer. Panics
/// loudly rather than silently answering yes or no, so a regression that
/// starts prompting here fails the test that uses it instead of passing on a
/// made-up answer.
fn never_approve() -> impl FnMut(&str, Danger) -> Result<bool, String> {
    |what: &str, _danger: Danger| -> Result<bool, String> {
        panic!("this test's `approve` should never be asked, and was asked: {what}");
    }
}

// ---------------------------------------------------------------------------
// execute_host_batch — Defect A (a decline/failure never abandons the other
// artifacts) and Defect A' (a decline is not final until confirmed)
// ---------------------------------------------------------------------------

fn plan_for(product: &str, digest: &str) -> ArtifactPlan {
    ArtifactPlan {
        product: product.to_string(),
        digest: digest.to_string(),
        material_kinds: Vec::new(),
        material_files: Vec::new(),
        settings: Vec::new(),
        proxied: false,
        declares_plain: false,
        is_library: false,
        depends_on: Vec::new(),
        is_kiosk: false,
    }
}

fn no_op_deploy_args() -> DeployArgs {
    DeployArgs {
        host: "dev-01:7777".to_string(),
        rvt: None,
        package_args: Vec::new(),
        remote_dir: String::new(),
        confirmed: true,
        identity: None,
        pin: None,
        enable: false,
        wait: false,
    }
}

/// **Defect A, pinned directly on the executor.** A real run selected four
/// applications, declined one step of the second, and only the FIRST got
/// installed — every artifact after the declined one was abandoned. Here two
/// artifacts are batched; the first's only step is declined (and the skip
/// confirmed), and the second's own step still runs to completion — the
/// batch never abandons it.
#[test]
fn a_confirmed_decline_skips_only_its_own_artifact_the_next_artifact_still_runs() {
    let e1 = entry("app-one", "1.0.0");
    let a1 = plan_for("app-one", "digest-1");
    let e2 = entry("app-two", "1.0.0");
    let a2 = plan_for("app-two", "digest-2");

    let host_steps = vec![
        (
            e1,
            a1,
            vec![Step {
                kind: StepKind::Enable { service: "app-one".to_string() },
                danger: Danger::Safe,
            }],
            None,
        ),
        (
            e2,
            a2,
            vec![Step {
                kind: StepKind::Enable { service: "app-two".to_string() },
                danger: Danger::Safe,
            }],
            None,
        ),
    ];

    // `app-two` already enabled, so its Enable step's own execution succeeds
    // deterministically with no network and no board mutation.
    let mut board = FakeBoard::new(None, Some("[services.app-two]\nenabled = true\n".to_string()));
    let parsed = no_op_deploy_args();
    let root = Path::new(".");

    // app-one's step: decline, then confirm the skip. app-two's step:
    // approve, which succeeds ("already enabled") and triggers Fix 1's
    // converge prompt — approved too, as the fourth and last answer.
    let mut answers = vec![false, true, true, true].into_iter();
    let mut approve =
        move |_: &str, _: Danger| -> Result<bool, String> { Ok(answers.next().expect("enough answers queued")) };

    let mut summary = String::new();
    execute_host_batch(&mut board, &parsed, root, "dev-01", host_steps, None, &mut summary, &mut approve)
        .expect("no I/O error");

    assert!(summary.contains("declined"), "{summary}");
    assert!(summary.contains("app-one"), "{summary}");
    // app-two was reached and its own step completed — the batch did not
    // abort after app-one's decline.
    assert!(summary.contains("done — app-two"), "{summary}");
    // Fix 1 — app-two's successful enable triggered exactly one converge.
    assert_eq!(
        *board.performed.borrow(),
        vec![vec!["apply".to_string(), "reload".to_string(), "confirm".to_string()]],
        "{:?}",
        board.performed.borrow()
    );
}

/// **Defect A', pinned directly on the executor.** Declining a step's
/// approve prompt does not skip it outright: the skip itself must be
/// confirmed, and declining THAT confirmation (a bare `Enter` included)
/// re-offers the ORIGINAL step rather than silently dropping it. The
/// maintainer's ruling (.rulings/ R-324): an answer that would end an install
/// step early is confirmed first — a stray/fat-fingered `n` must not cost a
/// whole artifact.
#[test]
fn declining_the_skip_confirmation_re_offers_the_original_step_instead_of_skipping() {
    let e1 = entry("app-one", "1.0.0");
    let a1 = plan_for("app-one", "digest-1");
    let host_steps = vec![(
        e1,
        a1,
        vec![Step { kind: StepKind::Enable { service: "app-one".to_string() }, danger: Danger::Safe }],
        None,
    )];

    let mut board = FakeBoard::new(None, Some("[services.app-one]\nenabled = true\n".to_string()));
    let parsed = no_op_deploy_args();
    let root = Path::new(".");

    // First ask: decline the step. Confirmation: decline the skip too, so
    // the step must be re-offered. Second ask of the SAME step: approve,
    // which succeeds and triggers Fix 1's converge prompt — declined here,
    // as the fourth and last answer, to prove a declined converge is not an
    // I/O error either.
    let mut answers = vec![false, false, true, false].into_iter();
    let mut approve =
        move |_: &str, _: Danger| -> Result<bool, String> { Ok(answers.next().expect("enough answers queued")) };

    let mut summary = String::new();
    execute_host_batch(&mut board, &parsed, root, "dev-01", host_steps, None, &mut summary, &mut approve)
        .expect("no I/O error");

    // The step was never actually skipped: it completed once the operator's
    // second answer approved it, not the confirmation-declined attempt.
    assert!(!summary.contains("declined — app-one"), "{summary}");
    assert!(summary.contains("done — app-one"), "{summary}");
    // The converge itself was declined — no `perform_operation` call at all.
    assert!(board.performed.borrow().is_empty(), "{:?}", board.performed.borrow());
    assert!(summary.contains("declined — converge"), "{summary}");
}

// ---------------------------------------------------------------------------
// Fix 1 — a freshly-enabled service converges instead of being restarted
// ---------------------------------------------------------------------------

/// **`deploy_plan::plan` itself never offers a `service-restart` for this
/// case any more** — `fleet_deploy_plan_tests.rs`'s own tests pin that.
/// What THIS test proves is the other half, at the executor: a REAL,
/// freshly-written `Enable` (not the "already enabled" shortcut the other
/// `execute_host_batch` tests use — see `FakeBoard::edit_document`'s own
/// doc comment) triggers exactly one converge, and nothing is ever sent to
/// `perform_operation` for a per-service restart.
#[test]
fn a_freshly_enabled_service_converges_once_instead_of_being_restarted() {
    let e1 = entry("notes-rivet", "1.0.0");
    let a1 = plan_for("notes-rivet", "digest-1");
    let host_steps = vec![(
        e1,
        a1,
        vec![Step {
            kind: StepKind::Enable { service: "notes-rivet".to_string() },
            danger: Danger::Safe,
        }],
        None,
    )];

    // An empty document: `Standing::Absent`, so this really is a first-time
    // enable — `execute_step`'s `Enable` arm reaches `board.edit_document`
    // rather than short-circuiting on "already enabled".
    let mut board = FakeBoard::new(None, Some(String::new()));
    let parsed = no_op_deploy_args();
    let root = Path::new(".");

    // Enable approved; the converge prompt approved second.
    let mut answers = vec![true, true].into_iter();
    let mut approve =
        move |_: &str, _: Danger| -> Result<bool, String> { Ok(answers.next().expect("enough answers queued")) };

    let mut summary = String::new();
    execute_host_batch(&mut board, &parsed, root, "dev-01", host_steps, None, &mut summary, &mut approve)
        .expect("no I/O error");

    assert_eq!(
        *board.performed.borrow(),
        vec![vec!["apply".to_string(), "reload".to_string(), "confirm".to_string()]],
        "expected exactly one converge and no per-service restart: {:?}",
        board.performed.borrow()
    );
    assert!(summary.contains("converge dev-01"), "{summary}");
    // And the document really was edited — this was not the "already
    // enabled" shortcut.
    assert!(board.document.unwrap_or_default().contains("[services.notes-rivet]"));
}

// ---------------------------------------------------------------------------
// Fix 2 — dependency_step_group and skipped_consumers_for
// ---------------------------------------------------------------------------

#[test]
fn a_met_dependency_yields_no_step_group() {
    let mut board = FakeBoard::new(None, None);
    let document = "[services.rivet-bluetoothd]\nenabled = true\n".to_string();
    let group =
        dependency_step_group(&mut board, &document, "tempmon-rivet", "rivet-bluetoothd", None)
            .expect("no I/O error");
    assert!(group.is_none(), "{group:?}");
}

/// A present system service (the `rivet-*` naming `is_system_or_dev` already
/// uses to hide Rivet's own daemons from the menu), not enabled here: one
/// `Enable` step, with a reason naming the consumer and the connection.
#[test]
fn an_unmet_system_service_dependency_yields_one_enable_step_with_a_reason() {
    let mut board = FakeBoard::new(None, None);
    let document = String::new(); // Standing::Absent
    let (dep_entry, _artifact, steps, note) =
        dependency_step_group(&mut board, &document, "tempmon-rivet", "rivet-bluetoothd", None)
            .expect("no I/O error")
            .expect("an unmet dependency yields a step group");

    assert_eq!(dep_entry.product, "rivet-bluetoothd");
    assert_eq!(
        steps,
        vec![Step {
            kind: StepKind::Enable { service: "rivet-bluetoothd".to_string() },
            danger: Danger::Safe,
        }]
    );
    let reason = note.expect("a reason");
    assert!(reason.contains("tempmon-rivet"), "{reason}");
    assert!(reason.contains("rivet-bluetoothd"), "{reason}");
    assert!(reason.contains("enable"), "{reason}");
}

/// A dependency this repository holds as a deployable artifact (not a system
/// service): `deploy_plan::plan`'s own sequence for it — install then enable
/// on a host that has neither — wrapped in one step group.
#[test]
fn an_unmet_artifact_dependency_yields_install_then_enable() {
    let dep_entry = entry("aura-controller", "0.1.0");
    let dep_plan = plan_for("aura-controller", "digest-dep");
    let resolved = (dep_entry, dep_plan);

    let mut board = FakeBoard::new(None, Some(String::new()));
    let document = String::new(); // Standing::Absent
    let (dep_entry_out, _artifact, steps, note) = dependency_step_group(
        &mut board,
        &document,
        "aura-renderer",
        "aura-controller",
        Some(&resolved),
    )
    .expect("no I/O error")
    .expect("an unmet dependency yields a step group");

    assert_eq!(dep_entry_out.product, "aura-controller");
    assert_eq!(steps.len(), 2, "{steps:?}");
    assert!(matches!(steps[0].kind, StepKind::Install { .. }));
    assert!(matches!(steps[1].kind, StepKind::Enable { .. }));
    let reason = note.expect("a reason");
    assert!(reason.contains("aura-renderer"), "{reason}");
    assert!(reason.contains("aura-controller"), "{reason}");
    // **Only what is known.** The document says the dependency is not
    // enabled; nothing read says whether it is installed, and dev-01's
    // deploy on 2026-09-26 offered to install a controller that was.
    assert!(reason.contains("not enabled here"), "{reason}");
    assert!(!reason.contains("not installed"), "{reason}");
    assert!(reason.contains("already holds this build changes nothing"), "{reason}");
}

/// A resolved artifact dependency the document already shows enabled yields
/// nothing to approve — `dependency_step_group` short-circuits on
/// `Standing::Enabled` before it ever consults `resolved`.
#[test]
fn an_already_enabled_artifact_dependency_yields_no_step_group() {
    let dep_entry = entry("aura-controller", "0.1.0");
    let dep_plan = plan_for("aura-controller", "digest-dep");
    let resolved = (dep_entry, dep_plan);

    let mut board = FakeBoard::new(None, None);
    let document = "[services.aura-controller]\nenabled = true\n".to_string();
    let group = dependency_step_group(
        &mut board,
        &document,
        "aura-renderer",
        "aura-controller",
        Some(&resolved),
    )
    .expect("no I/O error");
    assert!(group.is_none(), "{group:?}");
}

#[test]
fn an_artifact_whose_dependency_cannot_be_resolved_is_skipped_with_a_clear_reason() {
    let e1 = entry("tempmon-rivet", "1.0.0");
    let mut a1 = plan_for("tempmon-rivet", "digest-1");
    a1.depends_on = vec!["mystery-service".to_string()];
    let artifact_plans = vec![(e1, a1)];

    let mut unresolved = std::collections::BTreeMap::new();
    unresolved.insert(
        "mystery-service".to_string(),
        "it is neither a Rivet system service nor an artifact this fleet repository holds"
            .to_string(),
    );

    let skipped = skipped_consumers_for(&artifact_plans, &unresolved);
    let why = skipped.get("tempmon-rivet").expect("tempmon-rivet should be skipped");
    assert!(why.contains("mystery-service"), "{why}");
}

#[test]
fn a_resolvable_dependency_does_not_skip_its_consumer() {
    let e1 = entry("tempmon-rivet", "1.0.0");
    let mut a1 = plan_for("tempmon-rivet", "digest-1");
    a1.depends_on = vec!["rivet-bluetoothd".to_string()];
    let artifact_plans = vec![(e1, a1)];
    let unresolved = std::collections::BTreeMap::new();
    assert!(skipped_consumers_for(&artifact_plans, &unresolved).is_empty());
}

// ---------------------------------------------------------------------------
// the menu/dry-run filter — latest-per-product and hide-system-artifacts
// ---------------------------------------------------------------------------

fn entry(product: &str, version: &str) -> crate::fleet::Entry {
    crate::fleet::Entry {
        digest: format!("{product}-{version}"),
        product: product.to_string(),
        version: version.to_string(),
        arch: "aarch64".to_string(),
        platform: None,
        kind: "application".to_string(),
        size: 0,
        added: "2026-09-22T00:00:00Z".to_string(),
        schema_version: None,
    }
}

#[test]
fn is_system_or_dev_hides_daemons_probes_and_gpu_examples_only() {
    for hidden in ["rivet-proxyd", "rivet-fleetd", "gpu-flip", "gpu-surface", "mesa-demo",
        "facts-probe", "addon-probe-host", "gpu-pack-probe"]
    {
        assert!(is_system_or_dev(hidden), "{hidden} should be hidden");
    }
    for kept in ["trends-rivet", "tempmon-rivet", "notes-rivet", "reliquary-rivet",
        "aura-controller", "aura-renderer"]
    {
        assert!(!is_system_or_dev(kept), "{kept} is an application, not system/dev");
    }
}

#[test]
fn deployable_applications_defaults_to_latest_non_system_per_product() {
    let entries = vec![
        entry("trends-rivet", "0.1.6"),
        entry("trends-rivet", "0.1.8"),
        entry("trends-rivet", "0.1.7"),
        entry("tempmon-rivet", "0.1.0"),
        entry("rivet-proxyd", "0.1.87"),
        entry("gpu-flip", "1.0.0"),
        entry("addon-probe-host", "1.0.0"),
    ];
    let shown = deployable_applications(entries, &[], false);
    // System/dev dropped; one row per product; sorted by product name.
    let got: Vec<(&str, &str)> =
        shown.iter().map(|e| (e.product.as_str(), e.version.as_str())).collect();
    assert_eq!(got, vec![("tempmon-rivet", "0.1.0"), ("trends-rivet", "0.1.8")]);
}

/// **`ADR-052` §7: a hidden product is never offered, "show all" included** —
/// and a product of the same name under another kind is still offered.
#[test]
fn deployable_applications_leaves_out_a_hidden_product_even_with_show_all() {
    let mut library = entry("testbed", "1.0.0");
    library.kind = "library".to_string();
    let entries = vec![entry("testbed", "0.1.3"), library, entry("trends-rivet", "0.1.8")];
    let hidden = [crate::fleet::Hidden { product: "testbed".into(), kind: "application".into() }];
    for show_all in [false, true] {
        let shown = deployable_applications(entries.clone(), &hidden, show_all);
        let got: Vec<(&str, &str)> = shown.iter().map(|e| (e.product.as_str(), e.kind.as_str())).collect();
        assert_eq!(
            got,
            vec![("testbed", "library"), ("trends-rivet", "application")],
            "show_all={show_all}"
        );
    }
}

#[test]
fn deployable_applications_with_show_all_keeps_everything() {
    let entries = vec![
        entry("trends-rivet", "0.1.6"),
        entry("trends-rivet", "0.1.8"),
        // Still a carried application (`ADR-060` §6), so still offerable.
        entry("rivet-fleetd", "0.1.87"),
        entry("gpu-flip", "1.0.0"),
    ];
    let shown = deployable_applications(entries, &[], true);
    assert_eq!(shown.len(), 4, "show_all keeps system artifacts and every version");
    // trends-rivet's two versions are adjacent, newest first.
    let trends: Vec<&str> = shown
        .iter()
        .filter(|e| e.product == "trends-rivet")
        .map(|e| e.version.as_str())
        .collect();
    assert_eq!(trends, vec!["0.1.8", "0.1.6"]);
}

/// **`ADR-060`: the proxy is part of the image, and no planner offers to
/// install it.** A fleet repository can still hold an old `rivet-proxyd`
/// payload, which the machine's install door refuses by name — so it is
/// dropped even by "show all", which otherwise reveals every system artifact,
/// and a `connects` naming it never resolves to an artifact to install.
#[test]
fn an_image_service_is_never_offered_for_installation() {
    let entries = vec![entry("rivet-proxyd", "0.1.113"), entry("trends-rivet", "0.1.8")];
    for show_all in [false, true] {
        let shown = deployable_applications(entries.clone(), &[], show_all);
        let got: Vec<&str> = shown.iter().map(|e| e.product.as_str()).collect();
        assert_eq!(got, vec!["trends-rivet"], "show_all = {show_all}");
    }
    assert!(latest_entry_for(&entries, "rivet-proxyd").is_none());
}

/// **Updated for Defect C, 2026-09-22**: a library used to be dropped here
/// alongside a card — that was the bug (`cargo rivet fleet secrets` era,
/// before the interview could offer one at all). A library is now a
/// selectable, install-only artifact, so it is kept; only `card` (never
/// fleet-deployed by this interview) is still dropped.
#[test]
fn deployable_applications_keeps_libraries_but_drops_other_non_application_kinds() {
    let mut lib = entry("shared-thing", "1.0.0");
    lib.kind = "library".to_string();
    let mut card = entry("rivet", "0.1.87");
    card.kind = "card".to_string();
    let shown = deployable_applications(vec![lib, card, entry("trends-rivet", "0.1.8")], &[], true);
    let got: Vec<&str> = shown.iter().map(|e| e.product.as_str()).collect();
    assert_eq!(got, vec!["shared-thing", "trends-rivet"]);
}

/// An entry with a distinct `added` stamp and `digest` — needed to tell two
/// builds of the SAME version apart, which the plain `entry` helper cannot.
fn build(product: &str, version: &str, added: &str, digest: &str) -> crate::fleet::Entry {
    crate::fleet::Entry {
        digest: digest.to_string(),
        product: product.to_string(),
        version: version.to_string(),
        arch: "aarch64".to_string(),
        platform: None,
        kind: "application".to_string(),
        size: 0,
        added: added.to_string(),
        schema_version: None,
    }
}

/// A higher version always wins the latest-per-product pick, no matter how the
/// `added` stamps fall — the newer build's earlier arrival must not demote it.
#[test]
fn deployable_applications_prefers_higher_version_over_newer_added() {
    let entries = vec![
        // Lower version but added most recently.
        build("tempmon-rivet", "0.1.0", "2026-09-22T00:00:00Z", "low"),
        // Higher version, added earliest.
        build("tempmon-rivet", "0.2.0", "2026-09-01T00:00:00Z", "high"),
    ];
    let shown = deployable_applications(entries, &[], false);
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].version, "0.2.0");
    assert_eq!(shown[0].digest, "high", "the higher version wins regardless of `added`");
}

/// The defect's own case: three builds that share one version string and differ
/// only in the build metadata `version_order` discards (`-g<hash>`). The tie
/// must break to the build **added most recently**, not the first seen — before
/// the fix the oldest-added build won and kept getting deployed.
#[test]
fn deployable_applications_breaks_a_version_tie_by_newest_added() {
    let entries = vec![
        build("tempmon-rivet", "0.1.0-gaaaaaa", "2026-09-20T00:00:00Z", "oldest"),
        build("tempmon-rivet", "0.1.0-gbbbbbb", "2026-09-22T00:00:00Z", "newest"),
        build("tempmon-rivet", "0.1.0-gcccccc", "2026-09-21T00:00:00Z", "middle"),
    ];
    let shown = deployable_applications(entries, &[], false);
    assert_eq!(shown.len(), 1);
    assert_eq!(
        shown[0].digest, "newest",
        "on a version tie the most recently added build wins"
    );
}

/// The dependency-resolution path ([`latest_entry_for`]) shares the same tie
/// rule, so it must pick the most recently added of equal-version builds too.
#[test]
fn latest_entry_for_breaks_a_version_tie_by_newest_added() {
    let entries = vec![
        build("shared-lib", "1.0.0-g111111", "2026-09-20T00:00:00Z", "oldest"),
        build("shared-lib", "1.0.0-g333333", "2026-09-22T00:00:00Z", "newest"),
        build("shared-lib", "1.0.0-g222222", "2026-09-21T00:00:00Z", "middle"),
    ];
    let got = latest_entry_for(&entries, "shared-lib").expect("product is present");
    assert_eq!(got.digest, "newest", "dependency pick also favours newest `added` on a tie");

    // A higher version still wins here too.
    let with_higher = vec![
        build("shared-lib", "1.0.0", "2026-09-22T00:00:00Z", "low"),
        build("shared-lib", "1.1.0", "2026-09-01T00:00:00Z", "high"),
    ];
    let got = latest_entry_for(&with_higher, "shared-lib").expect("product is present");
    assert_eq!(got.digest, "high", "higher version wins regardless of `added`");
}

/// **`.rulings/` R-213, `show_all`'s own copy of the defect.** Unlike the
/// default view above, `show_all` never collapsed a product to one entry, so
/// its ordering bug was purely cosmetic — but the same manifest that made
/// `deployable_applications_breaks_a_version_tie_by_newest_added` necessary
/// also lists two tied builds out of `added` order here, in "show all".
#[test]
fn deployable_applications_show_all_breaks_a_version_tie_by_newest_added() {
    let entries = vec![
        build("tempmon-rivet", "0.1.0-gaaaaaa", "2026-09-20T00:00:00Z", "oldest"),
        build("tempmon-rivet", "0.1.0-gbbbbbb", "2026-09-22T00:00:00Z", "newest"),
    ];
    let shown = deployable_applications(entries, &[], true);
    assert_eq!(shown.len(), 2, "show_all keeps every version");
    assert_eq!(shown[0].digest, "newest", "the later-added build sorts first even with show_all");
    assert_eq!(shown[1].digest, "oldest");
}

/// **The full tie**: two builds share both version and `added` — a burst of
/// deploys inside the fleet repository's own one-second-granularity clock
/// (`ADR-021`'s reason `rivet_core::artifact`'s retention needed the same
/// third key). Before [`newest_cmp`]'s digest tie-break, the answer was
/// whichever `deployable_applications` happened to see first; proven here by
/// running the identical pair in both orders and requiring one answer.
#[test]
fn deployable_applications_breaks_a_full_tie_by_digest_regardless_of_input_order() {
    let a = build("tempmon-rivet", "0.1.0-gaaaaaa", "2026-09-22T00:00:00Z", "aaaa");
    let b = build("tempmon-rivet", "0.1.0-gbbbbbb", "2026-09-22T00:00:00Z", "bbbb");

    let forward = deployable_applications(vec![a.clone(), b.clone()], &[], false);
    let backward = deployable_applications(vec![b, a], &[], false);
    assert_eq!(forward.len(), 1);
    assert_eq!(backward.len(), 1);
    assert_eq!(
        forward[0].digest, backward[0].digest,
        "the same pair handed in the opposite order must resolve to the same winner"
    );
}

// ---------------------------------------------------------------------------
// Fix 3 — a `StageMaterial` step self-heals a board provisioned before Fix 2
// existed, rather than only relaying `crate::secrets::run`'s refusal.
//
// **What is NOT tested here, and why.** `execute_step`'s `StageMaterial` arm
// itself is not driven end to end through a test: it calls
// `default_authority()`, which reads the REAL `HOME`/`USERPROFILE` with no
// way to inject a scratch directory (the same reason no earlier test in this
// file exercises that arm at all — grep finds none). Reaching it would mean
// touching this machine's real `~/.rivet`, which this suite — and this whole
// task's hard constraints — never does. What IS tested is the two functions
// Fix 3 actually added, both of which take their directory as a plain
// parameter: the refusal matcher, and the decline path of the mint offer
// (which never touches a filesystem or spawns anything, since that only
// happens after an accept). The ACCEPT path — actually spawning
// `rivet-provision ca server` — is exercised nowhere in this suite, on
// purpose: this task's own constraints forbid running `ca server` for real,
// even against a scratch directory, so that path is UNPROVEN by anything
// here and needs a real run to close.
// ---------------------------------------------------------------------------

#[test]
fn missing_server_certificate_matches_only_this_exact_refusal_for_this_name() {
    // The exact shape `crate::secrets::server_pair` composes (path prefix,
    // then this sentence) — matched on the sentence, not the whole sentence
    // or the leading path.
    let refusal = "C:\\Users\\operator\\.rivet\\ca\\server-rivet-02.local.crt: no certificate for \
                    rivet-02.local here.\n why  --name takes a machine's fleet name";
    assert!(missing_server_certificate(refusal, "rivet-02.local"));

    // The identical SHAPE of refusal for a DIFFERENT name is not a match —
    // this offers to mint the name that was actually asked for, never a
    // neighbour's.
    assert!(!missing_server_certificate(refusal, "rivet-03.local"));

    // Every other refusal `crate::secrets::run` can give must never be
    // mistaken for this one, or Fix 3 would offer to mint over an unrelated
    // failure (the host unreachable, the secret-store gate, several
    // certificates with none named).
    assert!(!missing_server_certificate("there is no authority directory here.", "rivet-02.local"));
    assert!(!missing_server_certificate(
        "this authority has 2 server certificates and nothing said which:",
        "rivet-02.local"
    ));
    assert!(!missing_server_certificate(
        "the secret-store gate refused this install",
        "rivet-02.local"
    ));
}

/// **Declining the offer never touches a filesystem or spawns a process** —
/// `offer_to_mint_missing_certificate` returns before either, which is what
/// makes this test safe to run with a directory that does not exist.
#[test]
fn offer_to_mint_missing_certificate_names_the_manual_command_when_declined() {
    let authority = Path::new("/scratch/does-not-exist/ca");
    let args = crate::secrets::SecretsArgs {
        application: "tempmon-rivet".to_string(),
        authority: authority.to_path_buf(),
        name: Some("rivet-02.local".to_string()),
        endpoint: rivet_remote::endpoint::Endpoint::default(),
        confirm: true,
    };

    let mut asked: Vec<(String, Danger)> = Vec::new();
    let mut decline = |what: &str, danger: Danger| -> Result<bool, String> {
        asked.push((what.to_string(), danger));
        Ok(false)
    };

    let err = offer_to_mint_missing_certificate(
        "rivet-02",
        "rivet-02.local",
        authority,
        &args,
        &mut decline,
    )
    .expect_err("declined, so this refuses rather than proceeding");

    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(asked[0].0.contains("rivet-02.local"), "{}", asked[0].0);
    assert!(asked[0].0.contains("mint"), "{}", asked[0].0);
    // `[Y/n]` — default yes — because minting a certificate is reversible and
    // brief, the same class `StepKind::Enable` gets.
    assert_eq!(asked[0].1, Danger::Safe, "{asked:?}");

    assert!(err.contains("declined"), "{err}");
    assert!(err.contains("rivet-provision ca server rivet-02 "), "{err}");
    assert!(err.contains(&authority.display().to_string()), "{err}");
}
