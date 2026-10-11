//! Tests for `super` — what `cargo rivet install` says remains.
//!
//! **Pure**: an [`ArtifactPlan`] and a [`HostState`] in, steps out, no board.
//! The test that ties these steps to a real install, through
//! `crate::fleet::publish_then_send` and the interview's own readers, lives in
//! `fleet_tests.rs` beside the fixtures it needs.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::path::PathBuf;

use rivet_remote::deploy::{DeployArgs, Standing};

use super::*;

fn args() -> DeployArgs {
    DeployArgs {
        host: "board.local:7777".to_string(),
        rvt: None,
        package_args: Vec::new(),
        remote_dir: rivet_remote::deploy::DEFAULT_REMOTE_DIR.to_string(),
        confirmed: true,
        identity: Some(PathBuf::from("id-dir")),
        pin: Some("abc123".to_string()),
        enable: false,
        wait: true,
    }
}

/// A routed application declaring `fleet-tls` — every step the planner knows
/// can apply to it.
fn routed() -> ArtifactPlan {
    ArtifactPlan {
        product: "notes-rivet".to_string(),
        digest: "d1".repeat(32),
        material_kinds: vec!["fleet-tls".to_string()],
        material_files: ["tls-cert", "tls-key", "tls-root"].map(String::from).to_vec(),
        settings: Vec::new(),
        proxied: true,
        declares_plain: false,
        is_library: false,
        depends_on: Vec::new(),
        is_kiosk: false,
    }
}

/// The `StepKind::RestartReader` `routed()` offers — its exact reason and
/// cost, matching `deploy_plan::proxy_restart_reason`'s own non-plain text,
/// so a test can compare a planned step list without duplicating that
/// string a second time.
fn proxy_restart_step() -> StepKind {
    StepKind::RestartReader {
        service: "rivet-proxyd".to_string(),
        reason: "this pass changes what rivet-proxyd serves, and its route needs to be \
                  re-derived"
            .to_string(),
        cost: "every application behind the machine's front door is unreachable while it restarts",
    }
}

fn host(standing: Standing, data: Option<&[&str]>) -> HostState {
    HostState {
        current_digest: None,
        data_dir_files: data.map(|files| files.iter().map(ToString::to_string).collect()),
        standing,
        settings_present: Vec::new(),
        kiosk_auto_run: rivet_remote::deploy::KioskAutoRun::NoTable,
        kiosk_auto_run_installed: false,
        issues_material: false,
    }
}

const DOCUMENT: &str = "version = 1\nhostname = \"rivet-06f5c\"\n";

/// **A fresh machine**: nothing installed, nothing enabled. What remains after
/// the install is the material, the enable, the proxy restart the route needs,
/// and the one converge that starts what was enabled — the interview's own
/// order, less the install.
#[test]
fn a_fresh_routed_install_leaves_material_enable_proxy_and_converge() {
    let steps = remaining_after_install(&routed(), &host(Standing::Absent, None), false, true);
    assert_eq!(
        steps,
        vec![
            StepKind::StageMaterial {
                kinds: vec!["fleet-tls".to_string()],
                missing: ["tls-cert", "tls-key", "tls-root"].map(String::from).to_vec(),
            },
            StepKind::Enable { service: "notes-rivet".to_string() },
            proxy_restart_step(),
            StepKind::Realize,
        ]
    );
}

/// **A reinstall of a running, provisioned application**: the new bytes need
/// the service restarted and the route re-derived, and nothing else.
#[test]
fn a_reinstall_of_a_running_application_leaves_its_restart_and_the_proxys() {
    let steps = remaining_after_install(
        &routed(),
        &host(Standing::Enabled, Some(&["tls-cert", "tls-key", "tls-root"])),
        false,
        true,
    );
    assert_eq!(
        steps,
        vec![StepKind::ServiceRestart { service: "notes-rivet".to_string() }, proxy_restart_step()]
    );
}

/// **`--enable` did the enable, and the install's finish realised it** — the
/// supervisor re-reads the document when an install finishes — so neither the
/// enable nor a converge is owed. A restart is not either: the service was
/// started by that finish, with the new bytes.
#[test]
fn an_enable_the_install_made_is_neither_repeated_nor_converged_again() {
    let mut artifact = routed();
    artifact.material_files.clear();
    artifact.material_kinds.clear();
    let steps = remaining_after_install(&artifact, &host(Standing::Absent, None), true, true);
    assert_eq!(steps, vec![proxy_restart_step()]);
}

/// **A machine that refused the request as already running this artifact**:
/// nothing changed, so no restart is owed for new bytes — and an enable that
/// `--enable` wrote before the send was NOT re-read by anything, so the
/// converge is.
#[test]
fn an_already_running_artifact_owes_no_restart_but_a_converge_for_its_enable() {
    let mut artifact = routed();
    artifact.material_files.clear();
    artifact.material_kinds.clear();
    let steps = remaining_after_install(&artifact, &host(Standing::Absent, None), true, false);
    assert_eq!(steps, vec![StepKind::Realize]);
    let untouched = remaining_after_install(&artifact, &host(Standing::Enabled, None), false, false);
    assert!(untouched.is_empty(), "{untouched:?}");
}

/// **A library is install-only**, so nothing remains after its install.
#[test]
fn a_library_leaves_nothing() {
    let mut artifact = routed();
    artifact.is_library = true;
    assert!(remaining_after_install(&artifact, &host(Standing::Absent, None), false, true).is_empty());
}

fn reach<'a>(args: &'a DeployArgs, device: Option<&'a str>) -> Reach<'a> {
    Reach { args, device, at: None, authority: "/ca" }
}

/// **The commands are what the interview's `execute_step` does, spelled for a
/// workstation**, and a routed application's list carries the proxy restart.
#[test]
fn each_step_is_the_command_that_does_it() {
    let args = args();
    let steps = next_steps(&routed(), &host(Standing::Absent, None), DOCUMENT, &[], false, true, &reach(&args, Some("dev-01")));
    let commands: Vec<Vec<String>> = steps.iter().map(|s| s.commands.clone()).collect();
    assert_eq!(
        commands,
        vec![
            vec![
                "rivet-provision ca server rivet-06f5c /ca".to_string(),
                "cargo rivet secrets dev-01 notes-rivet --name rivet-06f5c.local confirm".to_string(),
            ],
            vec![
                "cargo rivet run dev-01 -- apply edit /state/config/machine.toml --append '\n\
                 [services.notes-rivet]\nenabled = true\n' confirm"
                    .to_string()
            ],
            vec!["cargo rivet run dev-01 -- apply service-restart rivet-proxyd confirm".to_string()],
            vec!["cargo rivet run dev-01 -- apply reload confirm".to_string()],
        ]
    );
    // The heading is the interview's own line for the step.
    assert_eq!(steps[2].heading, super::super::interview::describe(&proxy_restart_step()));
    assert_eq!(steps[0].danger, Danger::Dangerous);
    // `.rulings/` R-224 — the restart is dangerous too, and its heading
    // names the reason.
    assert_eq!(steps[2].danger, Danger::Dangerous);
    assert!(steps[2].heading.contains("route needs to be re-derived"), "{}", steps[2].heading);
}

/// **R-214: the scripted twin prints the kiosk auto-run step among what
/// remains, exactly as the interview's own `execute_step` would carry it
/// out** — `next_steps` only ever builds command strings; nothing in this
/// module runs anything, which is the whole point of `cargo rivet install`
/// naming it rather than doing it.
#[test]
fn a_kiosk_application_prints_its_auto_run_step() {
    let args = args();
    let mut artifact = routed();
    artifact.material_files.clear();
    artifact.proxied = false;
    artifact.is_kiosk = true;
    let mut host = host(Standing::Disabled, None);
    host.kiosk_auto_run = rivet_remote::deploy::KioskAutoRun::Named("rivet-launcher".to_string());
    let steps = next_steps(&artifact, &host, DOCUMENT, &[], false, true, &reach(&args, Some("dev-01")));
    let kiosk_step = steps
        .iter()
        .find(|s| s.heading.contains("auto-run"))
        .expect("a kiosk application not yet named auto-run must get the step");
    assert_eq!(
        kiosk_step.commands,
        vec![
            "cargo rivet run dev-01 -- apply edit /state/config/machine.toml --expect 'auto-run \
             = \"rivet-launcher\"' --replace 'auto-run = \"notes-rivet\"' confirm"
                .to_string(),
            "cargo rivet run dev-01 -- apply reload confirm".to_string(),
            "cargo rivet run dev-01 -- apply service-restart rivet-launcher confirm".to_string(),
        ]
    );
}

/// **The material line names the device when the install was given one**, and
/// the connection flags only when it was not. The flags form carries an
/// identity path; printed on Windows and pasted into Git Bash, its backslashes
/// were eaten before `secrets` saw them.
#[test]
fn the_secrets_line_names_the_device_when_there_is_one_and_the_flags_otherwise() {
    let args = args();
    let named = material_commands("notes-rivet", Some("dev-01"), &reach(&args, Some("dev-01")));
    assert_eq!(named[1], "cargo rivet secrets dev-01 notes-rivet --name dev-01.local confirm");

    let flagged = material_commands("notes-rivet", Some("dev-01"), &reach(&args, None));
    assert_eq!(
        flagged[1],
        "cargo rivet secrets notes-rivet --host board.local:7777 --identity id-dir --pin abc123 \
         --name dev-01.local confirm"
    );
}

/// A document that says `enabled = false` is changed by one word, exactly as
/// the interview's own enable changes it.
#[test]
fn a_disabled_service_is_enabled_by_replacing_one_word() {
    let args = args();
    let mut artifact = routed();
    artifact.material_files.clear();
    artifact.proxied = false;
    let steps = next_steps(&artifact, &host(Standing::Disabled, None), DOCUMENT, &[], false, true, &reach(&args, None));
    assert_eq!(
        steps[0].commands,
        vec![
            "cargo rivet run --host board.local:7777 --identity id-dir --pin abc123 -- apply edit \
             /state/config/machine.toml --expect '[services.notes-rivet]\nenabled = false' --replace \
             '[services.notes-rivet]\nenabled = true' confirm"
                .to_string()
        ]
    );
}

/// **On a fleet server the last step is the fleet sync**, and on any other
/// machine there is none — reported, never run.
#[test]
fn a_fleet_server_is_told_to_sync_and_nothing_else_is() {
    let args = args();
    let server = format!("{DOCUMENT}\n[services.rivet-fleetd]\nenabled = true\n");
    let reached = Reach { args: &args, device: Some("rivet-01"), at: Some(std::path::Path::new("/scratch/fleet")), authority: "/ca" };
    let steps = next_steps(&routed(), &host(Standing::Enabled, Some(&["tls-cert", "tls-key", "tls-root"])), &server, &[], false, true, &reached);
    let last = steps.last().expect("steps");
    assert!(last.heading.contains("fleet server"), "{}", last.heading);
    assert_eq!(last.commands, vec!["cargo rivet fleet sync rivet-01 --at /scratch/fleet confirm".to_string()]);

    let member = next_steps(&routed(), &host(Standing::Enabled, Some(&["tls-cert", "tls-key", "tls-root"])), DOCUMENT, &[], false, true, &reached);
    assert!(member.iter().all(|s| !s.commands.iter().any(|c| c.contains("fleet sync"))), "{member:?}");
    assert!(!is_fleet_server("version = 1\n[services.rivet-fleetd]\nenabled = false\n"));
}

/// A machine whose document does not say what it is called gets placeholders,
/// and is told which command shows the name — never an invented one.
#[test]
fn an_unnamed_machine_gets_placeholders_and_the_command_that_names_it() {
    let args = args();
    let steps = next_steps(&routed(), &host(Standing::Absent, None), "version = 1\n", &[], false, true, &reach(&args, Some("dev-01")));
    assert!(steps[0].commands[0].ends_with("ca server <hostname> /ca"), "{:?}", steps[0].commands);
    assert!(steps[0].commands[1].contains("--name <hostname>.local"), "{:?}", steps[0].commands);
    assert!(
        steps[0].note.as_deref().is_some_and(|n| n.contains("cargo rivet run dev-01 -- get document")),
        "{:?}",
        steps[0].note
    );
}

/// The rendered list numbers the steps in order, marks the one that handles a
/// key, and says so plainly when nothing remains — or when the install was not
/// waited for.
#[test]
fn the_rendering_numbers_the_steps_and_says_when_nothing_remains() {
    let args = args();
    let steps = next_steps(&routed(), &host(Standing::Absent, None), DOCUMENT, &[], false, true, &reach(&args, None));
    let text = render("notes-rivet", "board.local:7777", &steps, true);
    assert!(text.contains("4 steps remain"), "{text}");
    assert!(text.contains("  1. stage material"), "{text}");
    assert!(text.contains("[handles a private key]"), "{text}");
    assert!(text.contains("  3. restart rivet-proxyd"), "{text}");
    // `.rulings/` R-224 — the restart is dangerous too, and its own bracket
    // names its cost, not the material step's "handles a private key".
    assert!(
        text.contains("[every application behind the machine's front door is unreachable while \
                        it restarts]"),
        "{text}"
    );
    assert!(!text.contains("DID NOT WAIT"), "{text}");
    assert!(render("x", "h", &steps, false).contains("DID NOT WAIT"));
    assert!(render("x", "h", &[], true).contains("NOTHING REMAINS"));
}

/// **An unmet dependency comes first, as in the interview**, and one converge
/// at the end starts whatever it and the artifact enabled — even when the
/// artifact itself enabled nothing.
#[test]
fn a_dependency_is_enabled_first_and_converged_with_the_rest() {
    let args = args();
    let dependency = Dependency {
        artifact: ArtifactPlan {
            product: "rivet-bluetoothd".to_string(),
            digest: String::new(),
            material_kinds: Vec::new(),
            material_files: Vec::new(),
            settings: Vec::new(),
            proxied: false,
            declares_plain: false,
            is_library: false,
            depends_on: Vec::new(),
            is_kiosk: false,
        },
        steps: vec![StepKind::Enable { service: "rivet-bluetoothd".to_string() }],
        standing: Standing::Absent,
        reason: Some("tempmon connects to rivet-bluetoothd, which is not enabled here.".to_string()),
    };
    let mut artifact = routed();
    artifact.material_files.clear();
    artifact.proxied = false;
    let running = host(Standing::Enabled, None);
    let steps = next_steps(&artifact, &running, DOCUMENT, &[dependency], false, true, &reach(&args, Some("dev-01")));
    let headings: Vec<&str> = steps.iter().map(|s| s.heading.as_str()).collect();
    assert_eq!(
        headings,
        vec![
            "enable rivet-bluetoothd in the machine document",
            "restart service notes-rivet",
            "converge (apply reload confirm) — starts every service just enabled",
        ]
    );
    assert!(steps[0].commands[0].contains("[services.rivet-bluetoothd]"), "{:?}", steps[0].commands);
    assert!(steps[0].note.as_deref().is_some_and(|n| n.contains("tempmon connects")), "{:?}", steps[0].note);
}

/// Each missing file is named beside the kind that asked for it.
#[test]
fn a_missing_file_is_named_beside_its_kind() {
    let named = missing_with_kinds(&["fleet-tls".to_string()], &["tls-key".to_string()]);
    assert_eq!(named, vec![("tls-key".to_string(), "fleet-tls".to_string())]);
}
