//! Tests for [`super::plan`] — the needs planner. A fixture in, the expected
//! step list (with danger classes) out; no I/O, no board (§10.6).
//!
//! Covers: install idempotence, the material `None`-vs-empty-list data-dir
//! distinction, enable's three-way standing (Absent/Disabled emit,
//! Enabled/Unreadable don't), the install-tie rule for service-restart and
//! proxy-restart, settings delivery order, full §10.2 ordering, and
//! [`StepKind::danger`] pinned for every variant.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

// ---------------------------------------------------------------------------
// fixture builders
// ---------------------------------------------------------------------------

/// An artifact declaring nothing beyond a product and digest — every other
/// field starts empty/false, and each test adds only the needs it is about.
fn artifact(product: &str, digest: &str) -> ArtifactPlan {
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

/// Flip an artifact to a kiosk application — R-214's own fixture, the
/// `library` helper's twin.
fn kiosk(mut a: ArtifactPlan) -> ArtifactPlan {
    a.is_kiosk = true;
    a
}

/// Flip an artifact to a library — Defect C's fixture. Deliberately applied
/// on top of a fully-loaded artifact in the test below, so the assertion
/// proves `is_library` gates the steps rather than the fixture merely never
/// having declared them.
fn library(mut a: ArtifactPlan) -> ArtifactPlan {
    a.is_library = true;
    a
}

/// An artifact declaring one material kind (`fleet-tls`) and the two files it
/// carries — the shape a real `Material::files()` flattening would produce.
fn with_material(mut a: ArtifactPlan) -> ArtifactPlan {
    a.material_kinds = vec!["fleet-tls".to_string()];
    a.material_files = vec!["cert.pem".to_string(), "key.pem".to_string()];
    a
}

fn with_settings(mut a: ArtifactPlan, names: &[&str]) -> ArtifactPlan {
    a.settings = names.iter().map(|s| s.to_string()).collect();
    a
}

fn proxied(mut a: ArtifactPlan) -> ArtifactPlan {
    a.proxied = true;
    a
}

/// Flip a proxied artifact to also declare a `plain:` rule — R-224's proxy
/// stale-reader entry names port 80 in its reason exactly for this fixture.
fn with_plain_route(mut a: ArtifactPlan) -> ArtifactPlan {
    a.declares_plain = true;
    a
}

/// A host with nothing on it at all: no digest, no data directory, no service
/// table entry, no settings — the state a brand-new machine reads back as.
fn fresh_host() -> HostState {
    HostState {
        current_digest: None,
        data_dir_files: None,
        standing: Standing::Absent,
        settings_present: Vec::new(),
        kiosk_auto_run: rivet_remote::deploy::KioskAutoRun::NoTable,
        kiosk_auto_run_installed: false,
        issues_material: false,
    }
}

// ---------------------------------------------------------------------------
// the full acceptance case — every kind at once, in order
// ---------------------------------------------------------------------------

/// **The design doc's own worked example**: a fresh host and an artifact that
/// declares material, is proxied, and carries one setting. Every step kind
/// fires exactly once, in the §10.2 order, and each carries the danger class
/// [`StepKind::danger`] assigns it.
///
/// **Updated for Fix 1, 2026-09-22**: a fresh host's standing is
/// [`Standing::Absent`], so this pass ALSO enables the service — and per
/// Fix 1 that is exactly the case where a restart would be refused, so no
/// `ServiceRestart` appears here any more. `StepKind::Realize` (the host-level
/// converge that starts it) is not in this list either: `plan` never
/// constructs it (see `StepKind::Realize`'s own doc comment) —
/// `an_enabled_reinstall_still_gets_a_service_restart` below pins the case
/// that DOES still emit one.
#[test]
fn a_fresh_host_against_a_fully_loaded_artifact_emits_every_step_in_order() {
    let a = proxied(with_settings(with_material(artifact("notes-rivet", "digest-1")), &["timezone"]));
    let host = fresh_host();

    let steps = plan(&a, &host);

    assert_eq!(
        steps,
        vec![
            Step { kind: StepKind::Install { digest: "digest-1".to_string() }, danger: Danger::Safe },
            Step {
                kind: StepKind::StageMaterial {
                    kinds: vec!["fleet-tls".to_string()],
                    missing: vec!["cert.pem".to_string(), "key.pem".to_string()],
                },
                danger: Danger::Dangerous,
            },
            Step {
                kind: StepKind::Enable { service: "notes-rivet".to_string() },
                danger: Danger::Safe,
            },
            Step {
                kind: StepKind::RestartReader {
                    service: "rivet-proxyd".to_string(),
                    reason: "this pass changes what rivet-proxyd serves, and its route needs to \
                              be re-derived"
                        .to_string(),
                    cost: "every application behind the machine's front door is unreachable \
                           while it restarts",
                },
                danger: Danger::Dangerous,
            },
            Step {
                kind: StepKind::DeliverSetting { name: "timezone".to_string() },
                danger: Danger::Safe,
            },
        ]
    );
}

/// **Fix 1's other half, pinned directly.** A service that is ALREADY
/// enabled and running, reinstalled this pass, still gets a `ServiceRestart`
/// — that one is never refused, because the supervisor's own live view
/// already calls it enabled at the moment the restart is asked for.
#[test]
fn an_enabled_reinstall_still_gets_a_service_restart() {
    let a = artifact("notes-rivet", "digest-2");
    let host = HostState {
        current_digest: Some("digest-1".to_string()),
        data_dir_files: None,
        standing: Standing::Enabled,
        settings_present: Vec::new(),
        kiosk_auto_run: rivet_remote::deploy::KioskAutoRun::NoTable,
        kiosk_auto_run_installed: false,
        issues_material: false,
    };

    let steps = plan(&a, &host);

    assert_eq!(
        steps,
        vec![
            Step { kind: StepKind::Install { digest: "digest-2".to_string() }, danger: Danger::Safe },
            Step {
                kind: StepKind::ServiceRestart { service: "notes-rivet".to_string() },
                danger: Danger::Safe,
            },
        ]
    );
}

/// **Fix 1, the `Disabled` half.** A service the document says is disabled
/// (not merely absent) is the same "this pass is about to enable it" case as
/// `Absent` — no restart, the enable-plus-converge combination starts it.
#[test]
fn a_disabled_reinstall_also_skips_the_restart() {
    let a = artifact("notes-rivet", "digest-2");
    let host = HostState {
        current_digest: Some("digest-1".to_string()),
        data_dir_files: None,
        standing: Standing::Disabled,
        settings_present: Vec::new(),
        kiosk_auto_run: rivet_remote::deploy::KioskAutoRun::NoTable,
        kiosk_auto_run_installed: false,
        issues_material: false,
    };

    let steps = plan(&a, &host);

    assert_eq!(
        steps,
        vec![
            Step { kind: StepKind::Install { digest: "digest-2".to_string() }, danger: Danger::Safe },
            Step {
                kind: StepKind::Enable { service: "notes-rivet".to_string() },
                danger: Danger::Safe,
            },
        ]
    );
}

// ---------------------------------------------------------------------------
// idempotence — a fully satisfied host asks for nothing
// ---------------------------------------------------------------------------

/// A host already at the artifact's digest, with every material file present,
/// the service enabled, the setting delivered, and proxied — the steady
/// state a second run of the same deploy should settle into (§4.4).
#[test]
fn a_fully_satisfied_host_yields_no_steps() {
    let a = proxied(with_settings(with_material(artifact("notes-rivet", "digest-1")), &["timezone"]));
    let host = HostState {
        current_digest: Some("digest-1".to_string()),
        data_dir_files: Some(vec!["cert.pem".to_string(), "key.pem".to_string()]),
        standing: Standing::Enabled,
        settings_present: vec!["timezone".to_string()],
        kiosk_auto_run: rivet_remote::deploy::KioskAutoRun::NoTable,
        kiosk_auto_run_installed: false,
        issues_material: false,
    };

    assert_eq!(plan(&a, &host), Vec::new());
}

// ---------------------------------------------------------------------------
// the install-tie rule — service-restart and proxy-restart follow INSTALL,
// not any other step
// ---------------------------------------------------------------------------

/// **Pins the install-tie rule.** Same digest (no install), but material is
/// missing: the material step fires on its own terms, and neither
/// service-restart nor proxy-restart follows it, because nothing was
/// (re)installed this pass.
#[test]
fn same_digest_but_missing_material_emits_material_alone_no_restarts() {
    let a = proxied(with_material(artifact("notes-rivet", "digest-1")));
    let host = HostState {
        current_digest: Some("digest-1".to_string()),
        data_dir_files: Some(Vec::new()),
        standing: Standing::Enabled,
        settings_present: Vec::new(),
        kiosk_auto_run: rivet_remote::deploy::KioskAutoRun::NoTable,
        kiosk_auto_run_installed: false,
        issues_material: false,
    };

    let steps = plan(&a, &host);

    assert_eq!(
        steps,
        vec![Step {
            kind: StepKind::StageMaterial {
                kinds: vec!["fleet-tls".to_string()],
                missing: vec!["cert.pem".to_string(), "key.pem".to_string()],
            },
            danger: Danger::Dangerous,
        }]
    );
}

/// A new digest against an already-enabled host: install and service-restart
/// (and proxy-restart, since this artifact is proxied) fire, but enable does
/// not — the service table already says the service is on.
#[test]
fn a_new_digest_on_an_already_enabled_host_restarts_but_does_not_enable() {
    let a = proxied(artifact("notes-rivet", "digest-2"));
    let host = HostState {
        current_digest: Some("digest-1".to_string()),
        data_dir_files: None,
        standing: Standing::Enabled,
        settings_present: Vec::new(),
        kiosk_auto_run: rivet_remote::deploy::KioskAutoRun::NoTable,
        kiosk_auto_run_installed: false,
        issues_material: false,
    };

    let steps = plan(&a, &host);

    assert_eq!(
        steps,
        vec![
            Step { kind: StepKind::Install { digest: "digest-2".to_string() }, danger: Danger::Safe },
            Step {
                kind: StepKind::ServiceRestart { service: "notes-rivet".to_string() },
                danger: Danger::Safe,
            },
            Step {
                kind: StepKind::RestartReader {
                    service: "rivet-proxyd".to_string(),
                    reason: "this pass changes what rivet-proxyd serves, and its route needs to \
                              be re-derived"
                        .to_string(),
                    cost: "every application behind the machine's front door is unreachable \
                           while it restarts",
                },
                danger: Danger::Dangerous,
            },
        ]
    );
}

/// An install happens, but the artifact is not proxied: proxy-restart does
/// not fire. **Updated for Fix 1**: a fresh host also fires `Enable` rather
/// than `ServiceRestart` — see
/// `a_fresh_host_against_a_fully_loaded_artifact_emits_every_step_in_order`'s
/// own note.
#[test]
fn an_install_without_proxying_does_not_restart_the_proxy() {
    let a = artifact("tempmon-rivet", "digest-1");
    let host = fresh_host();

    let steps = plan(&a, &host);

    assert_eq!(
        steps,
        vec![
            Step { kind: StepKind::Install { digest: "digest-1".to_string() }, danger: Danger::Safe },
            Step {
                kind: StepKind::Enable { service: "tempmon-rivet".to_string() },
                danger: Danger::Safe,
            },
        ]
    );
}

// ---------------------------------------------------------------------------
// StepKind::RestartReader — `.rulings/` R-224, `ADR-055` §6. The proxy's
// own entry, and the table itself as the extension point.
// ---------------------------------------------------------------------------

/// **A plain-declaring artifact's reason names port 80; a non-plain one's
/// does not** — `ADR-046` §7. Both are otherwise the same fixture (proxied,
/// installed this pass), so the only difference in the step is the reason
/// text.
#[test]
fn the_proxy_reason_names_port_80_only_for_a_plain_declaring_artifact() {
    let plain = with_plain_route(proxied(artifact("gantry-rivet", "digest-1")));
    let ordinary = proxied(artifact("notes-rivet", "digest-1"));
    let host = fresh_host();

    let plain_reason = plan(&plain, &host)
        .into_iter()
        .find_map(|s| match s.kind {
            StepKind::RestartReader { service, reason, .. } if service == "rivet-proxyd" => Some(reason),
            _ => None,
        })
        .expect("a proxied, freshly-installed artifact always offers the restart");
    assert!(plain_reason.contains("port 80"), "{plain_reason:?}");

    let ordinary_reason = plan(&ordinary, &host)
        .into_iter()
        .find_map(|s| match s.kind {
            StepKind::RestartReader { service, reason, .. } if service == "rivet-proxyd" => Some(reason),
            _ => None,
        })
        .expect("a proxied, freshly-installed artifact always offers the restart");
    assert!(!ordinary_reason.contains("port 80"), "{ordinary_reason:?}");
}

/// **The cost is carried onto the step, for both front ends to state beside
/// the reason** (`ADR-055` §6's "what a restart costs, which the prompt
/// says out loud").
#[test]
fn the_proxy_entrys_cost_is_carried_onto_its_step() {
    let a = proxied(artifact("notes-rivet", "digest-1"));
    let cost = plan(&a, &fresh_host())
        .into_iter()
        .find_map(|s| match s.kind {
            StepKind::RestartReader { service, cost, .. } if service == "rivet-proxyd" => Some(cost),
            _ => None,
        })
        .expect("the fixture always offers the restart");
    assert_eq!(cost, "every application behind the machine's front door is unreachable while it restarts");
}

/// **The table is the extension point** — `.rulings/` R-224: a second,
/// test-only entry added to the table `plan` drives produces its own step,
/// with no other change to `plan` itself. Proves `STALE_READERS` is not
/// special-cased anywhere in `plan`'s own logic.
#[test]
fn a_second_table_entry_produces_its_own_step_with_no_other_change_to_plan() {
    fn always_stale(_artifact: &ArtifactPlan, _host: &HostState, installed_this_pass: bool) -> Option<String> {
        installed_this_pass.then(|| "test-only: this pass always makes it stale".to_string())
    }
    let readers: &[StaleReader] = &[
        StaleReader {
            service: "rivet-proxyd",
            trigger: |a, _h, installed| {
                // Mirrors the real entry's own trigger exactly, so this test
                // proves ADDING an entry, not replacing the real one.
                if a.proxied && installed {
                    Some("this pass changes what rivet-proxyd serves, and its route needs to be \
                          re-derived"
                        .to_string())
                } else {
                    None
                }
            },
            cost: "every application behind the machine's front door is unreachable while it restarts",
        },
        StaleReader {
            service: "test-only-reader",
            trigger: always_stale,
            cost: "a made-up cost, for this test alone",
        },
    ];
    let a = artifact("tempmon-rivet", "digest-1");
    let host = fresh_host();

    let steps = plan_with(readers, &a, &host);

    assert!(
        steps.iter().any(|s| matches!(
            &s.kind,
            StepKind::RestartReader { service, .. } if service == "test-only-reader"
        )),
        "{steps:?}"
    );
    // And `plan` itself (the real table) never emits it — the table, not
    // `plan`'s logic, is what changed.
    assert!(
        !plan(&a, &host).iter().any(|s| matches!(
            &s.kind,
            StepKind::RestartReader { service, .. } if service == "test-only-reader"
        )),
        "{:?}",
        plan(&a, &host)
    );
}

// ---------------------------------------------------------------------------
// material — the None-vs-empty-list data-dir distinction, and partial gaps
// ---------------------------------------------------------------------------

/// `data_dir_files: None` — no data directory at all, which is what a host the
/// application was never installed on reports — reads as every declared file
/// missing —
/// there is nowhere for any of them to be, which is a different claim from an
/// installed app whose directory happens to be empty.
#[test]
fn an_absent_data_directory_treats_every_declared_file_as_missing() {
    let a = with_material(artifact("notes-rivet", "digest-1"));
    let host = HostState {
        current_digest: Some("digest-1".to_string()), // no install this pass
        data_dir_files: None,
        standing: Standing::Enabled,
        settings_present: Vec::new(),
        kiosk_auto_run: rivet_remote::deploy::KioskAutoRun::NoTable,
        kiosk_auto_run_installed: false,
        issues_material: false,
    };

    let steps = plan(&a, &host);

    assert_eq!(
        steps,
        vec![Step {
            kind: StepKind::StageMaterial {
                kinds: vec!["fleet-tls".to_string()],
                missing: vec!["cert.pem".to_string(), "key.pem".to_string()],
            },
            danger: Danger::Dangerous,
        }]
    );
}

/// **A host that issues its own material is never offered a material step** —
/// `ADR-063` §4 — even with no data directory at all; the same host reading as
/// a workstation-root machine is, which pins that the flag alone decides it.
#[test]
fn a_host_that_issues_its_own_material_gets_no_material_step() {
    let a = with_material(artifact("notes-rivet", "digest-1"));
    let issuing = HostState {
        issues_material: true,
        ..fresh_host()
    };
    let steps = plan(&a, &issuing);
    assert!(
        !steps.iter().any(|s| matches!(s.kind, StepKind::StageMaterial { .. })),
        "{steps:?}"
    );
    assert!(steps.iter().any(|s| matches!(s.kind, StepKind::Install { .. })));
    assert!(steps.iter().any(|s| matches!(s.kind, StepKind::Enable { .. })));

    assert!(
        plan(&a, &fresh_host())
            .iter()
            .any(|s| matches!(s.kind, StepKind::StageMaterial { .. })),
        "a workstation-root host still stages its material"
    );
}

/// Only the files genuinely absent are named, and in `material_files`'
/// declared order — not the order they happen to sit in the host's listing.
#[test]
fn a_partially_present_data_directory_names_only_the_absent_files_in_declared_order() {
    let a = with_material(artifact("notes-rivet", "digest-1"));
    let host = HostState {
        current_digest: Some("digest-1".to_string()),
        data_dir_files: Some(vec!["key.pem".to_string(), "unrelated.txt".to_string()]),
        standing: Standing::Enabled,
        settings_present: Vec::new(),
        kiosk_auto_run: rivet_remote::deploy::KioskAutoRun::NoTable,
        kiosk_auto_run_installed: false,
        issues_material: false,
    };

    let steps = plan(&a, &host);

    assert_eq!(
        steps,
        vec![Step {
            kind: StepKind::StageMaterial {
                kinds: vec!["fleet-tls".to_string()],
                missing: vec!["cert.pem".to_string()],
            },
            danger: Danger::Dangerous,
        }]
    );
}

/// An artifact that declares no material at all emits no material step
/// regardless of what the host's data directory looks like.
#[test]
fn no_declared_material_means_no_material_step_ever() {
    let a = artifact("tempmon-rivet", "digest-1");
    let host = HostState {
        current_digest: Some("digest-1".to_string()),
        data_dir_files: None,
        standing: Standing::Enabled,
        settings_present: Vec::new(),
        kiosk_auto_run: rivet_remote::deploy::KioskAutoRun::NoTable,
        kiosk_auto_run_installed: false,
        issues_material: false,
    };

    assert_eq!(plan(&a, &host), Vec::new());
}

// ---------------------------------------------------------------------------
// enable — Absent and Disabled emit, Enabled and Unreadable do not
// ---------------------------------------------------------------------------

/// `Standing::Disabled` (the document says `enabled = false`) still needs an
/// enable step, same as `Absent`.
#[test]
fn a_disabled_standing_still_emits_enable() {
    let a = artifact("tempmon-rivet", "digest-1");
    let host = HostState {
        current_digest: Some("digest-1".to_string()),
        data_dir_files: None,
        standing: Standing::Disabled,
        settings_present: Vec::new(),
        kiosk_auto_run: rivet_remote::deploy::KioskAutoRun::NoTable,
        kiosk_auto_run_installed: false,
        issues_material: false,
    };

    assert_eq!(
        plan(&a, &host),
        vec![Step {
            kind: StepKind::Enable { service: "tempmon-rivet".to_string() },
            danger: Danger::Safe,
        }]
    );
}

/// **`Standing::Unreadable` is deliberately skipped, not treated as "needs
/// enabling".** The document names the service in a shape this tool did not
/// write, and it will not edit that blind — the operator gets the command
/// instead, same reasoning as `rivet_remote::deploy::Standing::Unreadable`'s
/// own doc comment.
#[test]
fn an_unreadable_standing_emits_no_enable_step() {
    let a = artifact("tempmon-rivet", "digest-1");
    let host = HostState {
        current_digest: Some("digest-1".to_string()),
        data_dir_files: None,
        standing: Standing::Unreadable,
        settings_present: Vec::new(),
        kiosk_auto_run: rivet_remote::deploy::KioskAutoRun::NoTable,
        kiosk_auto_run_installed: false,
        issues_material: false,
    };

    assert_eq!(plan(&a, &host), Vec::new());
}

// ---------------------------------------------------------------------------
// settings — only the undelivered ones, in declared order
// ---------------------------------------------------------------------------

/// Some settings already present, some not: only the absent ones are
/// emitted, and in the order the artifact declares them — not the order the
/// host happens to carry them in.
#[test]
fn only_undelivered_settings_are_emitted_in_declared_order() {
    let a = with_settings(artifact("tempmon-rivet", "digest-1"), &["timezone", "poll-interval", "units"]);
    let host = HostState {
        current_digest: Some("digest-1".to_string()),
        data_dir_files: None,
        standing: Standing::Enabled,
        settings_present: vec!["units".to_string()],
        kiosk_auto_run: rivet_remote::deploy::KioskAutoRun::NoTable,
        kiosk_auto_run_installed: false,
        issues_material: false,
    };

    let steps = plan(&a, &host);

    assert_eq!(
        steps,
        vec![
            Step {
                kind: StepKind::DeliverSetting { name: "timezone".to_string() },
                danger: Danger::Safe,
            },
            Step {
                kind: StepKind::DeliverSetting { name: "poll-interval".to_string() },
                danger: Danger::Safe,
            },
        ]
    );
}

/// Every declared setting already present emits nothing.
#[test]
fn every_setting_already_present_emits_nothing() {
    let a = with_settings(artifact("tempmon-rivet", "digest-1"), &["timezone"]);
    let host = HostState {
        current_digest: Some("digest-1".to_string()),
        data_dir_files: None,
        standing: Standing::Enabled,
        settings_present: vec!["timezone".to_string()],
        kiosk_auto_run: rivet_remote::deploy::KioskAutoRun::NoTable,
        kiosk_auto_run_installed: false,
        issues_material: false,
    };

    assert_eq!(plan(&a, &host), Vec::new());
}

// ---------------------------------------------------------------------------
// Defect C — a library deploys INSTALL-ONLY
// ---------------------------------------------------------------------------

/// **The heart of Defect C.** A library artifact declares material, is
/// proxied and carries a setting — the same fully-loaded fixture the
/// application worked example above uses — against a fresh host. Every one of
/// those needs is ignored: `is_library` yields exactly one step, `Install`,
/// never material/enable/service-restart/proxy-restart/settings.
#[test]
fn a_library_against_a_fresh_host_yields_install_only() {
    let a = library(proxied(with_settings(
        with_material(artifact("shared-rivets", "digest-1")),
        &["timezone"],
    )));
    let host = fresh_host();

    assert_eq!(
        plan(&a, &host),
        vec![Step {
            kind: StepKind::Install { digest: "digest-1".to_string() },
            danger: Danger::Safe,
        }]
    );
}

/// A library already at the host's current digest yields no steps at all —
/// idempotence (§4.4) applies to `Install` the same way it does for an
/// application; there is simply nothing after it to ever fire for a library.
#[test]
fn a_library_already_at_the_hosts_digest_yields_no_steps() {
    let a = library(with_material(artifact("shared-rivets", "digest-1")));
    let host = HostState {
        current_digest: Some("digest-1".to_string()),
        data_dir_files: None,
        standing: Standing::Absent,
        settings_present: Vec::new(),
        kiosk_auto_run: rivet_remote::deploy::KioskAutoRun::NoTable,
        kiosk_auto_run_installed: false,
        issues_material: false,
    };

    assert_eq!(plan(&a, &host), Vec::new());
}

/// The counterpart fixed point: an artifact with `is_library: false` (an
/// ordinary application) still gets the full §10.2 sequence — Defect C's fix
/// must never have touched application planning. Same fixture and host as
/// [`a_fresh_host_against_a_fully_loaded_artifact_emits_every_step_in_order`],
/// spelled out again here so the two behaviors are pinned side by side.
///
/// **Updated for Fix 1**: five steps, not six — a fresh host's `Enable` this
/// pass means no `ServiceRestart` (see that test's own note).
#[test]
fn an_application_still_gets_the_full_sequence_not_install_only() {
    let a = proxied(with_settings(with_material(artifact("notes-rivet", "digest-1")), &["timezone"]));
    let host = fresh_host();

    let steps = plan(&a, &host);

    assert_eq!(steps.len(), 5, "an application's full sequence is unaffected by Defect C");
    assert!(matches!(steps[0].kind, StepKind::Install { .. }));
    assert!(matches!(steps[1].kind, StepKind::StageMaterial { .. }));
    assert!(matches!(steps[2].kind, StepKind::Enable { .. }));
    assert!(matches!(steps[3].kind, StepKind::RestartReader { .. }));
    assert!(matches!(steps[4].kind, StepKind::DeliverSetting { .. }));
}

// ---------------------------------------------------------------------------
// SetKioskAutoRun — `.rulings/` R-214
// ---------------------------------------------------------------------------

/// A host whose document already names `named` as `[kiosk] auto-run`,
/// confirmed installed exactly when `installed` says so.
fn host_with_auto_run(named: &str, installed: bool) -> HostState {
    HostState {
        kiosk_auto_run: rivet_remote::deploy::KioskAutoRun::Named(named.to_string()),
        kiosk_auto_run_installed: installed,
        issues_material: false,
        ..fresh_host()
    }
}

/// **A kiosk application gets the step** — a fresh host has no `[kiosk]`
/// table at all, so the freshly-enabled kiosk application is offered as the
/// one to name.
#[test]
fn a_kiosk_application_gets_the_set_kiosk_auto_run_step() {
    let a = kiosk(artifact("aura-renderer", "digest-1"));
    let steps = plan(&a, &fresh_host());
    assert!(
        steps.iter().any(|s| matches!(
            &s.kind,
            StepKind::SetKioskAutoRun { application } if application == "aura-renderer"
        )),
        "{steps:?}"
    );
}

/// **A service application does not** — otherwise identical to the case
/// above, `is_kiosk` is simply never set.
#[test]
fn a_service_application_gets_no_set_kiosk_auto_run_step() {
    let a = artifact("tempmon-rivet", "digest-1");
    let steps = plan(&a, &fresh_host());
    assert!(!steps.iter().any(|s| matches!(s.kind, StepKind::SetKioskAutoRun { .. })), "{steps:?}");
}

/// **An application that is already the auto-run gets none** — offering it
/// would change nothing.
#[test]
fn an_application_already_named_auto_run_gets_no_step() {
    let a = kiosk(artifact("aura-renderer", "digest-1"));
    let host = host_with_auto_run("aura-renderer", true);
    let steps = plan(&a, &host);
    assert!(!steps.iter().any(|s| matches!(s.kind, StepKind::SetKioskAutoRun { .. })), "{steps:?}");
}

/// **The step's edit text is exact.** The document names `rivet-launcher`;
/// the change the step leads to must replace exactly that line with exactly
/// this application's name, and nothing else.
#[test]
fn the_kiosk_auto_run_edit_text_is_exact() {
    let a = kiosk(artifact("aura-renderer", "digest-1"));
    let host = host_with_auto_run("rivet-launcher", true);
    let steps = plan(&a, &host);
    let application = steps
        .iter()
        .find_map(|s| match &s.kind {
            StepKind::SetKioskAutoRun { application } => Some(application.clone()),
            _ => None,
        })
        .expect("a kiosk application not yet named must get the step");
    assert_eq!(application, "aura-renderer");

    let change = rivet_remote::deploy::kiosk_change_for(&host.kiosk_auto_run, &application)
        .expect("something to do");
    let rivet_remote::deploy::ConfigChange::Replace { expect, replace } = change else {
        panic!("a named auto-run already has a line; this must be a replace");
    };
    assert_eq!(expect, "auto-run = \"rivet-launcher\"");
    assert_eq!(replace, "auto-run = \"aura-renderer\"");
}

/// **Its place in the order is after the enable step and before the
/// restarts.** A fresh host both enables the kiosk application and offers
/// the auto-run step — in that order, and with no restart in between (Fix 1
/// suppresses `ServiceRestart` on the very pass that enables it).
#[test]
fn the_step_follows_enable_on_a_fresh_host() {
    let a = kiosk(artifact("aura-renderer", "digest-1"));
    let steps = plan(&a, &fresh_host());
    assert_eq!(steps.len(), 3, "install, enable, kiosk auto-run — {steps:?}");
    assert!(matches!(steps[0].kind, StepKind::Install { .. }), "{steps:?}");
    assert!(matches!(steps[1].kind, StepKind::Enable { .. }), "{steps:?}");
    assert!(matches!(steps[2].kind, StepKind::SetKioskAutoRun { .. }), "{steps:?}");
}

/// **The other half of the same ordering claim**: a REINSTALL of an
/// already-enabled, proxied kiosk application gets a service restart and a
/// proxy restart too, and the auto-run step still sits before both of them.
#[test]
fn the_step_precedes_the_restarts_on_a_reinstall() {
    let a = proxied(kiosk(artifact("aura-renderer", "digest-2")));
    let host = HostState {
        current_digest: Some("digest-1".to_string()),
        standing: Standing::Enabled,
        ..fresh_host()
    };
    let steps = plan(&a, &host);
    assert_eq!(
        steps
            .iter()
            .map(|s| match &s.kind {
                StepKind::Install { .. } => "install",
                StepKind::SetKioskAutoRun { .. } => "kiosk",
                StepKind::ServiceRestart { .. } => "service-restart",
                StepKind::RestartReader { .. } => "proxy-restart",
                other => panic!("unexpected step in this fixture: {other:?}"),
            })
            .collect::<Vec<_>>(),
        vec!["install", "kiosk", "service-restart", "proxy-restart"],
    );
}

/// **The context-dependent default.** Replacing a DIFFERENT, INSTALLED kiosk
/// application's auto-run defaults to `No`; anything else — nobody named yet,
/// an unconfirmed installation, or this same application — defaults to `Yes`.
#[test]
fn the_default_is_no_only_for_a_different_installed_application() {
    let a = kiosk(artifact("aura-renderer", "digest-1"));

    let danger_of = |host: &HostState| {
        plan(&a, host)
            .into_iter()
            .find_map(|s| matches!(s.kind, StepKind::SetKioskAutoRun { .. }).then_some(s.danger))
            .expect("the fixture always offers the step")
    };

    assert_eq!(danger_of(&fresh_host()), Danger::Safe, "nobody named yet");
    assert_eq!(
        danger_of(&host_with_auto_run("rivet-launcher", false)),
        Danger::Safe,
        "a different name, but not confirmed installed"
    );
    assert_eq!(
        danger_of(&host_with_auto_run("rivet-launcher", true)),
        Danger::Dangerous,
        "a different name, confirmed installed"
    );
}

// ---------------------------------------------------------------------------
// StepKind::danger — pinned for every variant (§5, ruling 5). A reboot step is
// not in the vocabulary — §10.2's deliberate Phase 1 absence is now enforced
// structurally by there being no `StepKind::Reboot` to emit.
// ---------------------------------------------------------------------------

#[test]
fn danger_matches_the_six_class_split_from_section_5() {
    assert_eq!(StepKind::Install { digest: "d".to_string() }.danger(), Danger::Safe);
    assert_eq!(
        StepKind::StageMaterial { kinds: Vec::new(), missing: Vec::new() }.danger(),
        Danger::Dangerous
    );
    assert_eq!(StepKind::Enable { service: "s".to_string() }.danger(), Danger::Safe);
    assert_eq!(StepKind::ServiceRestart { service: "s".to_string() }.danger(), Danger::Safe);
    // `.rulings/` R-224 — every stale-reader restart is `Dangerous`, whichever
    // entry of `STALE_READERS` produced it.
    assert_eq!(
        StepKind::RestartReader {
            service: "rivet-proxyd".to_string(),
            reason: "r".to_string(),
            cost: "c",
        }
        .danger(),
        Danger::Dangerous
    );
    assert_eq!(StepKind::DeliverSetting { name: "n".to_string() }.danger(), Danger::Safe);
    // R-214 — `StepKind::danger`'s own doc comment: this is the
    // CONTEXT-INDEPENDENT default only; `plan` overrides it per instance.
    assert_eq!(StepKind::SetKioskAutoRun { application: "a".to_string() }.danger(), Danger::Safe);
    // Fix 1 — the host-level converge is as safe as an enable: it changes
    // nothing the document does not already say.
    assert_eq!(StepKind::Realize.danger(), Danger::Safe);
}
