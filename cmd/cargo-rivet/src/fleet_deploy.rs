//! **`cargo rivet deploy` (bare) — the interactive deploy interview**, Phase 1
//! (`docs/design-candidates/Fleet-Deployment-Tool.md` §3, §10.3). **Reached
//! through bare `cargo rivet fleet` until 2026-09-22**, when the interview
//! moved to its own top-level verb, `deploy`, and `fleet` (bare) became only
//! its sub-verbs' usage text — see `main.rs`'s `deploy` and `fleet` dispatch
//! arms. `install` (renamed from `deploy` the same day) is the scripted twin
//! this interview composes.
//!
//! `super::deploy_plan` is the pure needs planner and does no I/O; this module
//! is the other half — it reads the fleet repository, an artifact's own
//! manifest and a host's read-back state into [`super::deploy_plan`]'s two
//! input structs, drives the `provision`-shaped menu/approve/execute loop
//! (§3), and turns each approved [`super::deploy_plan::StepKind`] into a call
//! on an operation that already exists (§6, §10.1) — `install`'s install,
//! `secrets`' staged material, `apply edit`, `apply service-restart`.
//!
//! # Two Phase 1 deviations from §10.2, by design and both a judgment call
//!
//! **`HostState::current_digest` is always `None`.** There is no cheap
//! "what digest is this host running" read yet (`get install` is Phase 2,
//! §10.4's "exact listen-change detection" sibling for this fact), so every
//! pass offers `Install`. This is safe rather than merely convenient: a board
//! that already holds the digest no-ops the install (`ADR-052` §7, §4.4) — the
//! menu shows one more line than is strictly needed, and nothing is at risk.
//!
//! **`HostState::settings_present` is always set to the artifact's own
//! declared settings**, so [`super::deploy_plan::plan`] never emits
//! `DeliverSetting`. A setting has a *value* as well as a name, and nothing in
//! this tool has a source for one — typing it in was ruled out
//! (`docs/design-candidates/Fleet-Deployment-Tool.md`'s own posture on
//! declared needs vs. typed input, §4.1). Offering the step anyway would put a
//! prompt in front of the operator this tool cannot actually carry out.
//! [`read_host_state_for`] sets this field to the artifact's declared settings,
//! which is what suppresses the step for both front ends. This is a deviation
//! from §10.2's table as written and is flagged for the maintainer to rule on.
//!
//! # What this module adds no power to do
//!
//! Every mutation below calls a function `cmd/cargo-rivet` already ships:
//! [`rivet_remote::deploy::run`] for install, [`crate::secrets::run`] for
//! staged material (through the same secret-store gate `cargo rivet secrets`
//! always goes through — §4.3, never bypassed), `Remote::edit_document` for
//! enable, `Remote::perform_operation` for a service or proxy restart — never
//! `Remote::ask`, which an MCP-backed board refuses a word list through by
//! design (its own doc comment names `perform_operation` as the fix). The
//! vocabulary of
//! [`super::deploy_plan::StepKind`] is fixed and this module does not grow it
//! at runtime from anything a manifest says (§4.1's load-bearing line).
//!
//! [`super::deploy_plan`]: crate::fleet::deploy_plan
//! [`super::deploy_plan::StepKind`]: crate::fleet::deploy_plan::StepKind
//! [`super::deploy_plan::plan`]: crate::fleet::deploy_plan::plan
//! [`read_host_state_for`]: crate::fleet::interview::read_host_state_for

// The link targets above are spelled from the crate root for
// `fleet_deploy_plan.rs`'s reason: an outer `///` on this module's `mod` line
// in `fleet.rs` makes rustdoc resolve these `//!` lines from `crate::fleet`,
// where `super` is the crate root. `read_host_state_for` and
// `read_artifact_plan_with` are `pub(crate)` for a second reason: `cargo rivet
// install` plans its next steps from them, which is what makes it the
// interview's twin rather than a second account of the same steps.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use rivet_core::artifact::newest_cmp;
use rivet_remote::deploy::{DeployArgs, Remote, Standing};

use super::deploy_plan::{self, ArtifactPlan, Danger, HostState, Step, StepKind};

// ---------------------------------------------------------------------------
// which artifacts the operator actually deploys — the menu/dry-run filter
// ---------------------------------------------------------------------------

/// Whether `product` is a Rivet system component or a development/example
/// artifact rather than one of the operator's own applications.
///
/// **Hidden by default** from both the interactive menu and the dry run (the
/// maintainer's ruling, 2026-09-22): the fleet repository accumulates the OS's
/// own daemons and dev-time probes, and neither is something an operator
/// fleet-deploys on purpose. The categories:
///
/// * `rivet-*` — Rivet's own daemons (`rivet-proxyd`, `rivet-fleetd`, …),
///   delivered by the system image / update channel, never fleet-deployed as
///   an application. The application products keep Rivet in a *suffix*
///   (`trends-rivet`, `tempmon-rivet`), so the prefix cleanly separates them.
/// * `gpu-*`, `mesa-*`, `*probe*` — the GPU/graphics examples and diagnostic
///   probes from the repo's `examples/` tree, published into the repo during
///   development. A **name heuristic, deliberately**: there is no manifest
///   marker for "this is an example", so this classifies by the shapes the repo
///   actually holds. The `show_all` escape hatch (menu) and `--all` (dry run)
///   reveal everything when a genuine one of these must be deployed.
fn is_system_or_dev(product: &str) -> bool {
    product.starts_with("rivet-")
        || product.starts_with("gpu-")
        || product.starts_with("mesa-")
        || product.contains("probe")
}

/// Whether `product` names a service the SYSTEM IMAGE defines —
/// `rivet_core::service::IMAGE_SERVICE_NAMES`, `rivet-proxyd` among them since
/// `ADR-060`.
///
/// **Never offered for installation, not even by "show all"** — unlike
/// [`is_system_or_dev`], which only hides. A fleet repository can still hold an
/// old `rivet-proxyd` payload from before the proxy joined the image
/// (`ADR-060` §4 leaves them to age out), and the machine's install door
/// refuses every one of them by name. Offering one would be offering a step
/// the machine is certain to refuse; the proxy is reached by enabling it and
/// fixed by a system update.
fn is_image_service(product: &str) -> bool {
    rivet_core::service::IMAGE_SERVICE_NAMES.contains(&product)
}

/// **Whether an application's own manifest declares it a kiosk application —
/// `.rulings/` R-214.** The same fact `cmd/rivet-sysd/src/supervise.rs`'s
/// `Supervisor::offered_displays` reads to decide what may go on the
/// launcher's menu, restated here from the manifest rather than a live
/// `ServiceDef`: `devices = ["display"]` (the pre-`ADR-058` shape, still how a
/// launcher that took DRM master directly asked for it) OR `connects` naming
/// the compositor's endpoint (`ADR-058`'s shape: a client that draws through
/// `rivet-displayd` asks for a surface instead of a node). Neither alone is
/// enough for every card, so both count, exactly as `offered_displays` does.
///
/// **What `offered_displays` ALSO excludes, and why this does not need to**:
/// that function additionally excludes whichever service provides the
/// compositor or controls the kiosk socket — `rivet-displayd` and
/// `rivet-launcher` — because it decides across every definition an image
/// might carry. Neither can ever reach this function: both are
/// `rivet_core::service::IMAGE_SERVICE_NAMES`, and `is_image_service` above
/// already keeps every image service off the deploy menu, "show all"
/// included, before an artifact's manifest is even opened.
fn is_kiosk_application(manifest: &rivet_core::package::application::Manifest) -> bool {
    manifest.devices().contains(&rivet_core::package::application::Device::Display)
        || manifest.connects().iter().any(|e| e.name() == rivet_core::compositor::ENDPOINT)
}

/// Orders two entries **of the same product** by which is the later build to
/// deploy: higher `version_order` first, and on equal versions the one
/// **added to the repository most recently**, then `digest` as a last,
/// deterministic tie-break. Several builds share one version string whenever
/// they differ only in the build metadata `version_order` discards
/// (`-g<hash>`, `-dirty`): without the `added` tie-break the OLDEST such
/// build won (first-seen) and kept getting deployed; without the final
/// `digest` tie-break, two builds added within one repository clock second
/// (`ADR-021`) still resolved by whichever a caller's iterator listed first,
/// rather than by a fact of the builds themselves.
///
/// `Greater` means `a` is the one to keep — [`newest_cmp`]'s own convention
/// exactly, so this is that shared rule and not a second copy of it
/// (`.rulings/` R-213).
fn latest_build_cmp(a: &super::Entry, b: &super::Entry) -> std::cmp::Ordering {
    newest_cmp(super::build_key(a), super::build_key(b))
}

/// The deployable artifacts a menu or dry run shows: `kind == "application"`
/// or `kind == "library"` always — **a library deploys INSTALL-ONLY** (Defect
/// C / `.rulings/` unruled at time of writing: nothing on a host starts a
/// library, so [`deploy_plan::plan`] gates every step but `Install` on
/// [`ArtifactPlan::is_library`]) — and, unless `show_all`, system/dev
/// artifacts ([`is_system_or_dev`]) dropped and each remaining product
/// collapsed to its newest version ([`newest_cmp`], the same ordering
/// `fleet list` and image selection use). Sorted by product name so the menu
/// is stable between runs; `show_all` keeps every version, newest first
/// within each product — [`newest_cmp`] again (reversed: this is a listing,
/// not a pick), so two builds sharing a version's numeric core still order
/// by `added` rather than by manifest position (`.rulings/` R-213).
///
/// **A hidden product is never offered, "show all" included** (`ADR-052` §7):
/// hiding it is the operator saying he does not need to look at it, and
/// "show all" is for system components and older versions, not for undoing a
/// hide. `install --artifact <digest>` still sends one, by its digest.
fn deployable_applications(
    entries: Vec<super::Entry>,
    hidden: &[super::Hidden],
    show_all: bool,
) -> Vec<super::Entry> {
    let mut apps: Vec<super::Entry> = entries
        .into_iter()
        .filter(|e| e.kind == "application" || e.kind == "library")
        .filter(|e| !is_image_service(&e.product))
        .filter(|e| !super::is_hidden(hidden, e))
        .filter(|e| show_all || !is_system_or_dev(&e.product))
        .collect();

    if show_all {
        apps.sort_by(|a, b| {
            a.product
                .cmp(&b.product)
                .then_with(|| newest_cmp(super::build_key(a), super::build_key(b)).reverse())
        });
        return apps;
    }

    // Newest build per product ([`latest_build_cmp`]: higher version, then
    // newer `added`). A first-seen product is kept; a later one of the same
    // product replaces it only when it is strictly the later build, so on a
    // version tie the more recently added one wins rather than the first seen.
    let mut latest: std::collections::BTreeMap<String, super::Entry> =
        std::collections::BTreeMap::new();
    for entry in apps {
        match latest.get(&entry.product) {
            Some(kept) if latest_build_cmp(kept, &entry) != std::cmp::Ordering::Less => {}
            _ => {
                latest.insert(entry.product.clone(), entry);
            }
        }
    }
    // A `BTreeMap` iterates by key, so this is already sorted by product name.
    latest.into_values().collect()
}

/// `cargo rivet deploy` (bare) — the whole interview, start to `Exit`.
///
/// # Errors
///
/// No terminal (names `cargo rivet install`, `secrets` and `run` as the
/// scripted way in — [`refusal_for_a_scripted_caller`]); the fleet
/// repository or the register cannot be read; or a menu/approval answer this
/// build cannot parse. A refusal or a failed step on one host is reported
/// inline and does not stop the pass — see [`run_host`] — so an `Err` here
/// means the interview itself could not continue, not that a deploy failed.
pub(crate) fn run() -> Result<String, String> {
    run_with(std::io::stdin().is_terminal())
}

/// **`cargo rivet fleet plan <host>` — the read-only dry run.** For every
/// application artifact the fleet repository holds, read its own manifest and
/// the host's read-back state and print the steps a deploy WOULD plan (§10.2) —
/// the interview's plan phase with nothing approved and nothing executed. Every
/// call it makes is a read.
///
/// It shares [`read_artifact_plan`], [`read_host_state_for`] and
/// [`deploy_plan::plan`] with the interview, so a green dry run is also proof
/// the read side works against a real machine before any deploy is approved. A
/// per-artifact read that fails is reported against that artifact and the run
/// continues, so one bad manifest does not hide the rest.
///
/// # Errors
///
/// The fleet repository cannot be read, or the host does not resolve / carries
/// no identity to open a connection with — both before any planning. Once the
/// connection is open, a per-artifact failure is reported inline, not returned.
pub(crate) fn plan_only(root: &Path, host_name: &str, show_all: bool) -> Result<String, String> {
    let entries = super::read_manifest(root)?;
    let hidden = super::read_hidden(root)?;
    let applications = deployable_applications(entries, &hidden, show_all);
    if applications.is_empty() {
        return Ok(format!(
            "The fleet repository at {} holds no {}application artifacts to plan.\n",
            root.display(),
            if show_all { "" } else { "non-system " },
        ));
    }

    let expanded = crate::device::expand_from_default(&[host_name.to_string()])?;
    let parsed = crate::deploy::parse(&expanded)?;
    let endpoint = rivet_remote::endpoint::Endpoint {
        host: Some(parsed.host.clone()),
        identity: parsed.identity.clone(),
        pin: parsed.pin.clone(),
    };
    let mut board = endpoint.open()?.ok_or_else(|| {
        format!(
            "{host_name} resolves, but its register record carries no identity/pin to open a \
             control-plane connection with. `cargo rivet install`'s own refusal has the fix."
        )
    })?;

    let mut out = String::new();
    out.push_str(&format!(
        "Dry run against {host_name} — the steps a deploy of each application artifact WOULD \
         plan. READ-ONLY: nothing was sent, approved or executed.\n\
         {}\
         Phase 1 does not read the running digest, so `install` is listed for every artifact; a \
         board no-ops an install of a digest it already holds (ADR-052 §7).\n",
        if show_all {
            "Showing ALL artifacts (system components + every version).\n"
        } else {
            "Showing the latest of each application; system/dev artifacts hidden — `--all` shows everything.\n"
        },
    ));
    for entry in &applications {
        out.push_str(&format!("\n{} {} ({}):\n", entry.product, entry.version, entry.arch));
        let artifact = match read_artifact_plan(root, entry) {
            Ok(a) => a,
            Err(e) => {
                out.push_str(&format!("  could not read its manifest: {e}\n"));
                continue;
            }
        };
        let state = match read_host_state_for(&mut board, &artifact) {
            Ok((s, _)) => s,
            Err(e) => {
                out.push_str(&format!("  could not read {host_name}'s state for it: {e}\n"));
                continue;
            }
        };
        let steps = deploy_plan::plan(&artifact, &state);
        if steps.is_empty() {
            out.push_str("  already current — no steps\n");
            continue;
        }
        for step in &steps {
            let tag = match step.danger {
                Danger::Safe => "safe",
                Danger::Dangerous => "DANGER",
            };
            out.push_str(&format!("  - [{tag}] {}\n", describe(&step.kind)));
        }
    }
    Ok(out)
}

/// [`run`], with whether this process has a terminal taken as a parameter —
/// `write_image::choose_with`'s own reason: a test drives the refusal, and
/// every other menu, without a real stdin.
fn run_with(is_terminal: bool) -> Result<String, String> {
    if !is_terminal {
        return Err(refusal_for_a_scripted_caller());
    }

    let root = super::deploy_root(None)?;
    let register_dir = crate::device::default_register_dir()?;
    let mut summary = String::new();
    // **Filtered by default (maintainer's ruling): the latest version of each
    // application, with Rivet's own daemons and dev probes hidden.** A menu
    // entry toggles it — no flag to type — for the rare deploy of an older
    // version or a hidden artifact.
    let mut show_all = false;

    loop {
        let entries = super::read_manifest(&root)?;
        let hidden = super::read_hidden(&root)?;
        // Cloned rather than moved: Fix 2's dependency resolution below needs
        // the WHOLE repository (a dependency may be a `rivet-*` system
        // component or an older/hidden artifact `deployable_applications`
        // would otherwise filter out of the menu), and `entries` is not large
        // enough for the clone to be worth avoiding.
        let applications = deployable_applications(entries.clone(), &hidden, show_all);
        if applications.is_empty() {
            summary.push_str(&format!(
                "\nThe fleet repository at {} holds no {}application or library artifacts to \
                 deploy. `cargo rivet fleet publish` or `cargo rivet install` (which publishes on \
                 the way) adds one.\n",
                root.display(),
                if show_all { "" } else { "non-system " },
            ));
            return Ok(summary);
        }

        // A library is labelled so it reads distinctly from an application in
        // the menu — the operator is about to get an install-only deploy of it
        // (Defect C), not the full sequence an application gets.
        let mut options: Vec<String> = applications
            .iter()
            .map(|e| {
                let tag = if e.kind == "library" { " (library)" } else { "" };
                format!("{} {} ({}){tag} — added {}", e.product, e.version, e.arch, e.added)
            })
            .collect();
        let show_all_at = options.len();
        options.push(
            if show_all {
                "Show fewer (latest of each application only)".to_string()
            } else {
                "Show all artifacts (system components + every version)".to_string()
            },
        );
        // **No numbered `Exit` item.** `x` is the one way to leave this menu,
        // as it is every other menu `multi_choose_with`/`choose_with` back,
        // on the removable-device menu's own convention (`.rulings/` R-219).
        let chosen = match multi_choose_with(
            is_terminal,
            "artifacts to deploy",
            &options,
            "cargo rivet install",
        )? {
            None => {
                summary.push_str("\nExited.\n");
                return Ok(summary);
            }
            Some(chosen) => chosen,
        };
        // The toggle wins over any artifact also ticked this submission: it
        // re-renders the list rather than acting on a stale view.
        if chosen.contains(&show_all_at) {
            show_all = !show_all;
            continue;
        }

        let selected: Vec<super::Entry> =
            chosen.iter().filter(|&&i| i < show_all_at).map(|&i| applications[i].clone()).collect();
        if selected.is_empty() {
            continue;
        }

        let register = rivetos_register::store::load(&register_dir).map_err(|e| {
            format!("could not read the register at {}: {e}", register_dir.display())
        })?;
        let mut devices: Vec<&rivetos_register::Device> =
            register.devices.values().filter(|d| !d.hidden).collect();
        if devices.is_empty() {
            summary.push_str(&format!(
                "\nThe register at {} holds no visible device. Nothing to deploy to; back to \
                 the artifact list.\n",
                register_dir.display()
            ));
            continue;
        }
        devices.sort_by(|a, b| a.name.cmp(&b.name));
        let host_options: Vec<String> = devices.iter().map(|d| d.name.clone()).collect();
        let host_chosen = match multi_choose_with(
            is_terminal,
            "hosts to deploy to",
            &host_options,
            "cargo rivet install",
        )? {
            None => {
                summary.push_str("\nExited — no host was chosen, and nothing was deployed.\n");
                return Ok(summary);
            }
            Some(chosen) => chosen,
        };
        let host_names: Vec<String> = host_chosen.iter().map(|&i| devices[i].name.clone()).collect();

        // Read every selected artifact's own manifest once for this pass —
        // it does not vary per host, so re-reading it per host would be the
        // same object opened and verified once per selected machine.
        let mut artifact_plans: Vec<(super::Entry, ArtifactPlan)> = Vec::new();
        for entry in &selected {
            let plan = read_artifact_plan(&root, entry)?;
            artifact_plans.push((entry.clone(), plan));
        }

        // Fix 2 — resolve every selected artifact's declared `connects`
        // dependency once here, the same reason `artifact_plans` above is:
        // WHAT a dependency name resolves to (a present system service, or
        // which digest is the newest fleet-repo artifact by that name) does
        // not vary per host. Only whether it is ALREADY met does, and
        // `run_host` checks that per host below.
        let (dependency_plans, skipped_consumers) =
            resolve_dependencies(&entries, &artifact_plans, &mut |e| read_artifact_plan(&root, e))?;

        for host_name in &host_names {
            summary.push_str(&format!("\n== {host_name} ==\n"));
            if let Err(e) = run_host(
                is_terminal,
                &root,
                host_name,
                &artifact_plans,
                &dependency_plans,
                &skipped_consumers,
                &mut summary,
            ) {
                summary.push_str(&format!("  {host_name}: {e}\n"));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Fix 2 — an unmet `connects` dependency is its own approvable step, ordered
// before the artifact that declares it. The maintainer's ruling (.rulings/
// R-325): the interview says it is going to install or enable a dependency,
// and lets the operator answer y/n to that before the software itself is
// installed.
// ---------------------------------------------------------------------------

/// The newest deployable entry (`application` or `library`) in `entries` for
/// `product` — mirroring `deployable_applications`'s own latest-per-product
/// rule, but for one product name rather than the whole menu. This is Fix
/// 2's way of telling "not in this repository at all" apart from "found, use
/// its newest build".
fn latest_entry_for<'a>(entries: &'a [super::Entry], product: &str) -> Option<&'a super::Entry> {
    entries
        .iter()
        .filter(|e| e.product == product && (e.kind == "application" || e.kind == "library"))
        .filter(|e| !is_image_service(&e.product))
        .max_by(|a, b| latest_build_cmp(a, b))
}

/// A dependency this repository has no artifact for, but no `ArtifactPlan`
/// verification (`read_artifact_plan` opens and signature-checks an object)
/// is needed to name it, because a system service is never in the fleet
/// repository to begin with — Fix 2's [`system_dependency_entry`] /
/// [`system_dependency_plan`] carry only what a bare `Enable` step needs.
fn system_dependency_entry(name: &str) -> super::Entry {
    super::Entry {
        digest: String::new(),
        product: name.to_string(),
        version: "(system service)".to_string(),
        arch: String::new(),
        platform: None,
        kind: "dependency".to_string(),
        size: 0,
        added: String::new(),
        schema_version: None,
    }
}

/// See [`system_dependency_entry`] — the `ArtifactPlan` half. Nothing in
/// [`deploy_plan::StepKind::Enable`]'s execution reads any of these fields;
/// only `service` on the step itself does.
fn system_dependency_plan(name: &str) -> ArtifactPlan {
    ArtifactPlan {
        product: name.to_string(),
        digest: String::new(),
        material_kinds: Vec::new(),
        material_files: Vec::new(),
        settings: Vec::new(),
        proxied: false,
        declares_plain: false,
        is_library: false,
        depends_on: Vec::new(),
        // A system service dependency (`rivet-bluetoothd` and the like) is
        // never itself a kiosk application, and `is_system_or_dev` has
        // already narrowed `name` to that shape before this is reached.
        is_kiosk: false,
    }
}

/// Resolve every selected artifact's declared dependency (`ArtifactPlan::depends_on`)
/// to what this repository can say about it, once for the whole pass.
///
/// Returns the `ArtifactPlan` this repository holds for every dependency that
/// turned out to be a deployable artifact (never a duplicate — a dependency
/// also independently selected by the operator is skipped here, because its
/// own entry in `artifact_plans` already covers it), and which SELECTED
/// artifact must be skipped because it names a dependency neither a present
/// system service (`is_system_or_dev`) nor an artifact this repository holds
/// — Fix 2 step 4: report clearly, never crash.
///
/// `read` is how a dependency's own manifest is read — [`read_artifact_plan`]
/// for the interview, [`read_artifact_plan_with`] and the seed its publish
/// used for `cargo rivet install`, which resolves the same dependencies so it
/// can print the same steps.
///
/// # Errors
///
/// Whatever `read` refuses with for a dependency this repository DOES hold an
/// entry for is recorded against that dependency, so its consumer is skipped
/// with the reason rather than the dependency being reported as merely "not
/// found" (which would be the wrong diagnosis). The `Result` is kept for a
/// failure of the pass as a whole.
pub(super) fn resolve_dependencies(
    entries: &[super::Entry],
    artifact_plans: &[(super::Entry, ArtifactPlan)],
    read: &mut dyn FnMut(&super::Entry) -> Result<ArtifactPlan, String>,
) -> Result<(Vec<(super::Entry, ArtifactPlan)>, std::collections::BTreeMap<String, String>), String>
{
    let selected_products: std::collections::BTreeSet<&str> =
        artifact_plans.iter().map(|(e, _)| e.product.as_str()).collect();

    let mut dependency_plans: Vec<(super::Entry, ArtifactPlan)> = Vec::new();
    let mut unresolved: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (_, artifact) in artifact_plans {
        for dep in &artifact.depends_on {
            if selected_products.contains(dep.as_str())
                || is_system_or_dev(dep)
                || !seen.insert(dep.clone())
            {
                continue;
            }
            match latest_entry_for(entries, dep) {
                Some(dep_entry) => match read(dep_entry) {
                    Ok(dep_plan) => dependency_plans.push((dep_entry.clone(), dep_plan)),
                    Err(e) => {
                        unresolved.insert(dep.clone(), format!("its own manifest could not be read: {e}"));
                    }
                },
                None => {
                    unresolved.insert(
                        dep.clone(),
                        "it is neither a Rivet system service nor an artifact this fleet \
                         repository holds"
                            .to_string(),
                    );
                }
            }
        }
    }

    Ok((dependency_plans, skipped_consumers_for(artifact_plans, &unresolved)))
}

/// Fix 2 step 4 — which selected artifacts must be skipped because they name
/// a dependency `resolve_dependencies` could not resolve to anything, and
/// why. Pure: `unresolved` is already the answer, computed (with I/O) by the
/// caller; this only maps consumers to a reason worth printing.
fn skipped_consumers_for(
    artifact_plans: &[(super::Entry, ArtifactPlan)],
    unresolved: &std::collections::BTreeMap<String, String>,
) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    for (entry, artifact) in artifact_plans {
        for dep in &artifact.depends_on {
            if let Some(why) = unresolved.get(dep) {
                out.entry(entry.product.clone())
                    .or_insert_with(|| format!("it connects to {dep}, and {why}"));
            }
        }
    }
    out
}

/// **One dependency's need, decided for one host.** `resolved` is
/// `Some(&(Entry, ArtifactPlan))` when `dep` is a fleet-repo artifact
/// [`resolve_dependencies`] already looked up (outside the per-host loop, the
/// same reason `artifact_plans` is) and `None` when `dep` is not a Rivet
/// system service either — the unresolvable case, which the caller reports
/// through `skipped_consumers_for` rather than reaching this function at all
/// (a consumer named there is skipped before its dependencies are ever
/// walked, so `None` here is unreached in practice and handled rather than
/// panicked on for the same reason `execute_step`'s own unreachable arms are:
/// the vocabulary is fixed at compile time, not proven unreachable by the
/// call graph a future change could not disturb).
///
/// Returns `Ok(None)` when the dependency is already met (`Standing::Enabled`)
/// or, for a fleet-repo dependency, already fully satisfied by
/// `deploy_plan::plan` on its own terms (nothing to approve is nothing to
/// add). Otherwise the step group to insert BEFORE the consumer's own steps,
/// carrying a one-line reason naming the consumer and the connection.
///
/// # Errors
///
/// Whatever `read_host_state_for` refuses with, reading the dependency's own data
/// directory and the document over `board`.
pub(super) fn dependency_step_group(
    board: &mut dyn Remote,
    document: &str,
    consumer_product: &str,
    dep: &str,
    resolved: Option<&(super::Entry, ArtifactPlan)>,
) -> Result<Option<(super::Entry, ArtifactPlan, Vec<Step>, Option<String>)>, String> {
    if matches!(rivet_remote::deploy::standing(document, dep), Standing::Enabled) {
        return Ok(None);
    }

    if is_system_or_dev(dep) {
        let steps =
            vec![Step { kind: StepKind::Enable { service: dep.to_string() }, danger: Danger::Safe }];
        return Ok(Some((
            system_dependency_entry(dep),
            system_dependency_plan(dep),
            steps,
            Some(format!(
                "{consumer_product} connects to {dep}, which is not enabled here — enable it?"
            )),
        )));
    }

    let Some((dep_entry, dep_artifact)) = resolved else {
        return Ok(None);
    };

    let (dep_state, _) = read_host_state_for(board, dep_artifact)?;
    let steps = deploy_plan::plan(dep_artifact, &dep_state);
    if steps.is_empty() {
        return Ok(None);
    }
    Ok(Some((
        dep_entry.clone(),
        dep_artifact.clone(),
        steps,
        // **"Not enabled" is what is known; "not installed" is not.** The
        // install step is offered either way (the module header's Phase 1
        // deviation), so the question says what running it costs where the
        // build is already there: nothing.
        Some(format!(
            "{consumer_product} connects to {dep}, which is not enabled here — install it (a \
             machine that already holds this build changes nothing) and enable it?"
        )),
    )))
}

/// Refuse to ask a question with nowhere to type an answer.
///
/// **The interview is the human tool, and `cargo rivet install` is its
/// scripted twin** — one planner drives both. A no-terminal caller — an agent,
/// a script, piped stdin — is sent to `install`, which installs, waits for
/// that install, and prints every step the interview would have offered after
/// it (material, enable, the converge, the restarts, the proxy's) as the exact
/// command to run. The list is not repeated here: it is the planner's, and a
/// second copy of it in a refusal is how the two front ends once drifted.
fn refusal_for_a_scripted_caller() -> String {
    "this run has no terminal, and `cargo rivet deploy` is the interactive deploy interview — a \
     human tool. A script or an agent uses its scripted twin, driven by the same planner:\n\
     \x20 cargo rivet install <device> --artifact <digest> [--enable] confirm   (install --help)\n\
     It installs, waits for that install to finish, and prints the steps that remain — \
     material, enable, the converge, the restarts — as the exact commands to run, in order.\n\
     \n\
     `cargo rivet fleet <verb>` (list, plan, publish, push, drift, …) stays scripted and is \
     unchanged; only the bare `cargo rivet deploy` interview refuses here."
        .to_string()
}

/// `write_image::refuse_unless_terminal`'s own text, copied rather than
/// reached for across a module boundary that does not expose it.
fn refuse_unless_terminal(is_terminal: bool, what: &str, instead: &str) -> Result<(), String> {
    if is_terminal {
        return Ok(());
    }
    Err(format!(
        "this run has no terminal to ask about {what} at, so nothing was read from stdin and \
         nothing was done. `{instead}` is the flag-driven, non-interactive way in."
    ))
}

// ---------------------------------------------------------------------------
// menus
// ---------------------------------------------------------------------------

/// A numbered multi-select menu: several of `options`, comma- or
/// space-separated, or an out-of-range/unparsable answer refused outright
/// (never silently narrowed to what did parse) — or `x`/`X` alone, to leave
/// without choosing anything.
///
/// `None` is that `x`/`X`; `Some` is never empty — seeing nothing chosen at
/// all is [`parse_multi_select`]'s own refusal, not a value this returns.
fn multi_choose_with(
    is_terminal: bool,
    what: &str,
    options: &[String],
    instead: &str,
) -> Result<Option<Vec<usize>>, String> {
    if options.is_empty() {
        return Err(format!("there are no {what} to choose from"));
    }
    refuse_unless_terminal(is_terminal, what, instead)?;
    println!("\n  {what} (one or more, comma- or space-separated, e.g. \"1,3\"):");
    for (n, option) in options.iter().enumerate() {
        println!("    {}) {option}", n + 1);
    }
    println!("    x) exit — deploy nothing and leave");
    print!("\n  choose 1-{}, or x: ", options.len());
    std::io::stdout().flush().ok();

    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(|e| format!("cannot read a choice: {e}"))?;
    parse_multi_select(&line, options.len())
}

/// [`multi_choose_with`]'s parsing half, pure and unit-tested directly: a
/// numbers string in, the zero-based indices it names out, in the order they
/// were first typed and with duplicates dropped — or the WHOLE trimmed
/// answer is `x`/`X` (case-insensitively), which is `None`.
///
/// **`x` mixed with numbers is not the escape hatch.** `"1,x"` tokenises to
/// `"1"` and `"x"`, and the second token is refused as not a number — the
/// same refusal any other stray token gets, naming `x` as the one this menu
/// takes alone rather than silently dropping it or silently exiting on it.
///
/// # Errors
///
/// Nothing was typed, a token is not a number (including `x` alongside
/// anything else), or a number is not in `1..=max` — never a silent drop of
/// the bad token.
fn parse_multi_select(input: &str, max: usize) -> Result<Option<Vec<usize>>, String> {
    if input.trim().eq_ignore_ascii_case("x") {
        return Ok(None);
    }
    let mut chosen: Vec<usize> = Vec::new();
    for token in input.split(|c: char| c == ',' || c.is_whitespace()).filter(|t| !t.is_empty()) {
        let n: usize = token
            .parse()
            .map_err(|_| format!("{token:?} is not a number between 1 and {max} (or x alone to exit)"))?;
        if n == 0 || n > max {
            return Err(format!("{n} is not between 1 and {max}"));
        }
        let index = n - 1;
        if !chosen.contains(&index) {
            chosen.push(index);
        }
    }
    if chosen.is_empty() {
        return Err("nothing was chosen".to_string());
    }
    Ok(Some(chosen))
}

/// **`.rulings/` R-214, "with more than one kiosk application in one plan,
/// ask which one, or none".** A numbered menu of every candidate this pass
/// would otherwise separately offer to make `[kiosk] auto-run`, plus a
/// "none of them" option — asked ONCE for the whole host, so approving two
/// artifacts' own steps in a row can never leave the auto-run set twice,
/// with the second silently winning.
fn choose_kiosk_auto_run(is_terminal: bool, candidates: &[String]) -> Result<Option<String>, String> {
    refuse_unless_terminal(
        is_terminal,
        "which application becomes [kiosk] auto-run",
        "cargo rivet install",
    )?;
    println!(
        "\n  more than one selected application could become this machine's [kiosk] auto-run \
         this pass — which one, or none:"
    );
    for (n, name) in candidates.iter().enumerate() {
        println!("    {}) {name}", n + 1);
    }
    let none_at = candidates.len() + 1;
    println!("    {none_at}) none — leave [kiosk] auto-run as it is");
    println!("    x) exit — deploy nothing to this host");
    print!("\n  choose 1-{none_at}, or x: ");
    std::io::stdout().flush().ok();

    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(|e| format!("cannot read a choice: {e}"))?;
    parse_kiosk_auto_run_choice(&line, candidates)
}

/// [`choose_kiosk_auto_run`]'s parsing half, pure and unit-tested directly:
/// the last option (`candidates.len() + 1`) is "none", answered `Ok(None)`.
/// `x`/`X` is refused with a clean message rather than chosen; nothing has
/// been written to the host by the time this question is asked (`run_host`
/// only reads and plans before it), so `run_host`'s `?` propagating that
/// refusal is accurate as "nothing was deployed" and stops this host's own
/// batch there, before `execute_host_batch` runs any step.
///
/// # Errors
///
/// `x`/`X`, nothing typed, a token that is not a number, or a number not in
/// `1..=candidates.len() + 1`.
fn parse_kiosk_auto_run_choice(input: &str, candidates: &[String]) -> Result<Option<String>, String> {
    let max = candidates.len() + 1;
    let trimmed = input.trim();
    if trimmed.eq_ignore_ascii_case("x") {
        return Err(
            "exited — no [kiosk] auto-run application was chosen, and nothing was deployed to \
             this host."
                .to_string(),
        );
    }
    let n: usize = trimmed
        .parse()
        .map_err(|_| format!("{trimmed:?} is not a number between 1 and {max} (or x to exit)"))?;
    if n == 0 || n > max {
        return Err(format!("{n} is not between 1 and {max}"));
    }
    if n == max {
        return Ok(None);
    }
    Ok(Some(candidates[n - 1].clone()))
}

/// One step's approval prompt: `[Y/n]` for [`Danger::Safe`], `[y/N]` for
/// [`Danger::Dangerous`] (§4.2, ruling 3/5) — the class picks the default and
/// the operator never configures it per step.
fn approve_with(is_terminal: bool, what: &str, danger: Danger) -> Result<bool, String> {
    refuse_unless_terminal(is_terminal, what, "cargo rivet install")?;
    let prompt = match danger {
        Danger::Safe => "[Y/n]",
        Danger::Dangerous => "[y/N]",
    };
    print!("  approve — {what} {prompt}: ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).map_err(|e| format!("cannot read an answer: {e}"))?;
    Ok(parse_approval(&line, danger))
}

/// [`approve_with`]'s parsing half, pure and unit-tested directly.
///
/// A bare `Enter` (empty, once trimmed) takes `danger`'s own default —
/// `Safe` is yes, `Dangerous` is no. Any other answer accepts only when it is
/// `y` or `yes`, case-insensitively; everything else declines, whatever the
/// danger class — a mistyped answer must never silently become approval.
#[must_use]
fn parse_approval(input: &str, danger: Danger) -> bool {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return matches!(danger, Danger::Safe);
    }
    matches!(trimmed.to_ascii_lowercase().as_str(), "y" | "yes")
}

// ---------------------------------------------------------------------------
// reading an artifact's own manifest — §10.1's "per artifact" read
// ---------------------------------------------------------------------------

/// Read `entry`'s own object, verify it against the application signing seed
/// exactly as [`super::publish_verified`] does, and pack its manifest into
/// [`ArtifactPlan`].
///
/// **The same verification a publish already performed on this bytes.** This
/// is a second open of an object this workstation already trusted once, not a
/// second trust decision — `entry.digest` is only ever a name in the manifest
/// [`super::read_manifest`] already parsed out of a git-tracked file.
///
/// # Errors
///
/// No application signing seed (names both places looked, exactly as
/// [`super::deploy_app_seed`]'s own refusal does); the object at
/// `root/objects/<digest>` is missing or does not verify; or `entry` is
/// neither an application nor a library (the artifact menu already filters to
/// `kind == "application"` or `kind == "library"`, so this is a repository
/// inconsistency rather than an ordinary path).
fn read_artifact_plan(root: &Path, entry: &super::Entry) -> Result<ArtifactPlan, String> {
    let seed = super::deploy_app_seed(
        &crate::credentials::Overrides::from_environment(),
        crate::credentials::root().as_deref(),
    )?;
    read_artifact_plan_with(root, entry, &seed)
}

/// [`read_artifact_plan`], verified against the seed at `seed` rather than the
/// one the environment names — **the one reading of an artifact's needs both
/// front ends use.** `cargo rivet install` hands it the seed its own publish
/// verified against (`--signing-key` included), so the steps it prints come
/// from exactly the [`ArtifactPlan`] the interview would build, and a test can
/// hand it a scratch seed without reaching a real `HOME`.
///
/// # Errors
///
/// [`read_artifact_plan`]'s, less the seed lookup.
pub(crate) fn read_artifact_plan_with(
    root: &Path,
    entry: &super::Entry,
    seed: &Path,
) -> Result<ArtifactPlan, String> {
    use rivet_adapter_image::BootSlots as _;
    use rivet_core::package::port::Sign as _;

    let object = root.join(super::OBJECTS_DIR).join(&entry.digest);
    let bytes = crate::key::read_32(seed, "application signing seed")?;
    let trusted = rivet_crypto::Ed25519Signer::from_seed(&bytes).public_key();

    // `ADR-029` §4's reserved names, exactly as `publish_verified` and
    // `main.rs`'s `inspect_cmd` read them.
    let boot_target = rivet_adapter_image::PiFirmware
        .payload(rivet_adapter_image::SlotId(1))
        .map_err(|e| format!("cannot describe slot A's targets: {e}"))?;
    let reserved = rivet_adapter_image::PiFirmware.reserved_names(&boot_target.kernel);

    let inspected = crate::inspect::inspect(
        &object,
        trusted,
        reserved,
        &mut rivet_crypto::Blake3::new(),
        &rivet_crypto::Ed25519Verifier::new(),
    )
    .map_err(|e| {
        format!(
            "{} ({} {}) is not a valid, signed artifact this key trusts: {e}",
            entry.digest, entry.product, entry.version
        )
    })?;

    // An application declares needs this planner turns into steps; a library
    // declares none of them — Defect C — and deploys INSTALL-ONLY, because
    // nothing on a host *starts* a library (`deploy_plan::plan` gates every
    // step but `Install` on `is_library`, so an empty needs tuple here is
    // exactly what a library's own manifest would have said anyway).
    let (material_kinds, material_files, settings, proxied, declares_plain, is_library, depends_on, is_kiosk) =
        match inspected.details {
            crate::inspect::Details::Application { manifest, .. } => (
                manifest.material().iter().map(|m| m.as_str().to_string()).collect(),
                manifest
                    .material()
                    .iter()
                    .flat_map(|m| m.files().iter().map(|f| (*f).to_string()))
                    .collect(),
                manifest.settings().to_vec(),
                manifest.proxy_path().is_some(),
                // `.rulings/` R-224, `ADR-046` §7 — read directly from the
                // signed manifest's own per-path rules, never guessed:
                // `ADR-046` §7 ties `rivet-proxyd`'s port 80 listener to a
                // `plain:` rule, and the proxy stale-reader entry's reason
                // names it exactly when one is declared.
                manifest
                    .proxy_auth_paths()
                    .iter()
                    .any(|rule| rule.auth() == rivet_core::package::application::ProxyAuth::Plain),
                false,
                // Fix 2 — the distinct owners of `connects`, in declared
                // order. `Endpoint::owner()` is the providing application's
                // name for a `connects` (`ADR-015` §5) — exactly the
                // `[services.<name>]` a dependency step would enable.
                dedup_preserving_order(manifest.connects().iter().map(|e| e.owner().to_string())),
                // R-214 — `is_kiosk_application`'s own doc comment.
                is_kiosk_application(&manifest),
            ),
            crate::inspect::Details::Library { .. } => {
                (Vec::new(), Vec::new(), Vec::new(), false, false, true, Vec::new(), false)
            }
            _ => {
                return Err(format!(
                    "{} ({} {}) is a {} artifact, and this interview only deploys applications and \
                     libraries — the artifact menu should already have filtered it out.",
                    entry.digest, entry.product, entry.version, entry.kind
                ));
            }
        };

    Ok(ArtifactPlan {
        product: entry.product.clone(),
        digest: entry.digest.clone(),
        material_kinds,
        material_files,
        settings,
        proxied,
        declares_plain,
        is_kiosk,
        is_library,
        depends_on,
    })
}

/// `items`, deduplicated, keeping the first occurrence's position — the same
/// rule `parse_multi_select` already applies to a typed menu answer, reused
/// here for a manifest's declared `connects` (two endpoints on the same
/// owner name that owner once, not twice).
fn dedup_preserving_order(items: impl Iterator<Item = String>) -> Vec<String> {
    let mut out = Vec::new();
    for item in items {
        if !out.contains(&item) {
            out.push(item);
        }
    }
    out
}

/// **`.rulings/` R-214, "with more than one kiosk application in one plan,
/// ask which one, or none".** More than one selected artifact's own plan can
/// offer `SetKioskAutoRun` in the SAME pass (two kiosk applications deployed
/// together). Approving each in turn would let the LAST one silently win, so
/// every per-artifact occurrence is stripped out of `host_steps` here and
/// returned as the candidate list — the caller asks once, across the whole
/// host, instead. Exactly one candidate is left untouched: it stays where
/// [`deploy_plan::plan`] put it, in its own artifact's own step list, offered
/// and approved the ordinary way.
///
/// Pure and total, split out from [`run_host`] so this decision — which
/// candidates existed and whether any were removed — is provable without a
/// board.
fn strip_multiple_kiosk_candidates(
    host_steps: &mut [(super::Entry, ArtifactPlan, Vec<Step>, Option<String>)],
) -> Vec<String> {
    let candidates: Vec<String> = host_steps
        .iter()
        .flat_map(|(_, _, steps, _)| steps.iter())
        .filter_map(|step| match &step.kind {
            StepKind::SetKioskAutoRun { application } => Some(application.clone()),
            _ => None,
        })
        .collect();
    if candidates.len() > 1 {
        for (_, _, steps, _) in host_steps {
            steps.retain(|step| !matches!(step.kind, StepKind::SetKioskAutoRun { .. }));
        }
    }
    candidates
}

// ---------------------------------------------------------------------------
// one host's pass: connect, plan, approve, execute
// ---------------------------------------------------------------------------

/// Open `host_name`, plan every one of `artifact_plans` against it, and run
/// the approved steps **per artifact** — Defect A's fix. A decline or a
/// failed step stops only the REST of THAT ARTIFACT's own steps (you cannot
/// enable what did not install), and never abandons the other artifacts in
/// this pass: a real run that selected four applications and declined one
/// step of the second used to install only the first — see
/// [`execute_host_batch`], which is where that per-artifact boundary is
/// actually kept. This function always returns `Ok` once a connection is
/// open, because a decline or a failed step is reported, not a failure of
/// the interview itself; other hosts still get their own pass (§3 step 4,
/// §10.3 step 6).
///
/// # Errors
///
/// The device does not resolve in the register, or its record carries no
/// identity/pin to open a connection with — before any planning happens.
/// Once a connection is open, every failure is per-step and reported into
/// `summary` rather than returned here.
fn run_host(
    is_terminal: bool,
    root: &Path,
    host_name: &str,
    artifact_plans: &[(super::Entry, ArtifactPlan)],
    dependency_plans: &[(super::Entry, ArtifactPlan)],
    skipped_consumers: &std::collections::BTreeMap<String, String>,
    summary: &mut String,
) -> Result<(), String> {
    let expanded = crate::device::expand_from_default(&[host_name.to_string()])?;
    let parsed = crate::deploy::parse(&expanded)?;
    let endpoint = rivet_remote::endpoint::Endpoint {
        host: Some(parsed.host.clone()),
        identity: parsed.identity.clone(),
        pin: parsed.pin.clone(),
    };
    let mut board = endpoint.open()?.ok_or_else(|| {
        "needs --identity and --pin: this device's record in the register carries no identity \
         directory this build could fill in. `cargo rivet identity new` mints one; see `cargo \
         rivet install`'s own refusal for the rest."
            .to_string()
    })?;

    // Fix 2's dependency standing checks read this once, rather than each
    // calling `board.read_document()` on its own — `read_host_state_for` below
    // still does its own read per artifact/dependency for that PRODUCT's own
    // standing, which is the same small redundancy this function already had
    // for N artifacts before this fix.
    let document = board.read_document()?.unwrap_or_default();

    // Plan every selected artifact against this host, and show the whole
    // batch before touching anything — §3 step 3's "plan and post-install
    // review". Each artifact keeps its OWN step list rather than being
    // flattened into one vec across artifacts, so the execution phase below
    // can tell where one artifact's steps end and the next one's begin.
    // Owned, not borrowed: Fix 2's dependency groups are constructed here,
    // not read out of `artifact_plans`.
    let mut host_steps: Vec<(super::Entry, ArtifactPlan, Vec<Step>, Option<String>)> = Vec::new();
    for (entry, artifact) in artifact_plans {
        if let Some(why) = skipped_consumers.get(&entry.product) {
            summary.push_str(&format!("  {} {}: SKIPPED — {why}\n", entry.product, entry.version));
            continue;
        }

        // Fix 2 — each of this artifact's unmet dependencies becomes its own
        // approvable step group, ordered BEFORE the artifact's own steps
        // (Joel's ruling, module header). One level deep only — a
        // dependency's own `depends_on` is never walked here
        // (`ArtifactPlan::depends_on`'s own doc).
        for dep in &artifact.depends_on {
            if host_steps.iter().any(|(e, _, _, _)| &e.product == dep) {
                continue; // an earlier consumer this same pass already queued it
            }
            let resolved = dependency_plans.iter().find(|(e, _)| &e.product == dep);
            if let Some((dep_entry, dep_artifact, steps, note)) =
                dependency_step_group(&mut board, &document, &artifact.product, dep, resolved)?
            {
                summary.push_str(&format!(
                    "  {} {} (a dependency {} connects to):\n",
                    dep_entry.product, dep_entry.version, artifact.product
                ));
                for step in &steps {
                    summary.push_str(&format!("    - {}\n", describe(&step.kind)));
                }
                host_steps.push((dep_entry, dep_artifact, steps, note));
            }
        }

        let (state, _) = read_host_state_for(&mut board, artifact)?;
        let steps = deploy_plan::plan(artifact, &state);
        if steps.is_empty() {
            summary.push_str(&format!(
                "  {} {}: already current, nothing to do\n",
                entry.product, entry.version
            ));
            continue;
        }
        summary.push_str(&format!("  {} {}:\n", entry.product, entry.version));
        for step in &steps {
            summary.push_str(&format!("    - {}\n", describe(&step.kind)));
        }
        host_steps.push((entry.clone(), artifact.clone(), steps, None));
    }

    // R-214, "with more than one kiosk application in one plan, ask which
    // one, or none" — `strip_multiple_kiosk_candidates`'s own doc comment.
    let kiosk_candidates = strip_multiple_kiosk_candidates(&mut host_steps);
    let chosen_kiosk_auto_run = if kiosk_candidates.len() > 1 {
        choose_kiosk_auto_run(is_terminal, &kiosk_candidates)?
    } else {
        None
    };

    execute_host_batch(
        &mut board,
        &parsed,
        root,
        host_name,
        host_steps,
        chosen_kiosk_auto_run,
        summary,
        &mut |what, danger| approve_with(is_terminal, what, danger),
    )
}

/// **Execute an already-planned batch, one artifact at a time — Defects A,
/// A' and B live here.**
///
/// Defect A: a decline or a failed step `break`s out of the CURRENT
/// artifact's own step list only (the `'artifact` loop below), never the
/// outer loop over `host_steps` — the next artifact always gets its turn.
///
/// Defect A': a decline is not final by itself. It is re-asked as an
/// explicit skip confirmation — `[y/N]`, default no — and only an
/// affirmative answer to THAT question actually skips the rest of the
/// artifact; declining the confirmation (a bare `Enter` included) re-offers
/// the ORIGINAL step rather than treating silence as agreement to skip. This
/// is the maintainer's ruling (.rulings/ R-324): an answer that would end an
/// install step early is confirmed first — a stray or fat-fingered `n` must not
/// silently drop the rest of a deploy the way it did in the run that
/// surfaced Defect A.
///
/// Defect B: a live `[i/n] product version[ (library)] on host` header
/// prints before each artifact, and an inline `installed` / `already
/// current — skipped` / `done` / `declined` / `FAILED: <reason>` prints
/// after each step — [`step_outcome`]'s job — so the operator sees progress
/// as it happens rather than only in `summary` at Exit. `summary` still
/// accumulates the same lines for the end-of-run record.
///
/// `approve` stands in for [`approve_with`] bound to a real `is_terminal` —
/// passed in rather than reached for directly so a test can drive a fixed
/// sequence of decisions through a fake board with no real stdin, the same
/// reason every terminal-reading entry point in this module takes
/// `is_terminal` as a parameter.
///
/// **Fix 1 lives here too**, at the end: after every group above has had its
/// turn, one converge (`StepKind::Realize`) realizes every service any of
/// them just enabled — see that variant's own doc comment for why this is
/// the one place that constructs it: a converge belongs where the WHOLE
/// batch is visible, and this is where the batch is assembled. `any_enable_succeeded` is the trigger, set the moment an
/// approved `StepKind::Enable` step's execution does not fail — that
/// includes both a real artifact's own enable and a Fix 2 dependency's.
///
/// **`chosen_kiosk_auto_run` is R-214's consolidated choice**, from the
/// caller's own "which one, or none" question when more than one selected
/// artifact's plan offered `SetKioskAutoRun` this pass — `None` either
/// because there was at most one candidate (already in its own artifact's
/// step list, handled by the ordinary loop above and never reaching this
/// field at all) or because the operator chose none of them. Executed once,
/// after every artifact's own steps, the same reason `StepKind::Realize` is.
///
/// # Errors
///
/// Whatever `approve` refuses with — a real stdin read failure, not an
/// ordinary decline (which is `Ok(false)`, not an `Err`).
#[allow(clippy::too_many_arguments)]
fn execute_host_batch(
    board: &mut dyn Remote,
    parsed: &DeployArgs,
    root: &Path,
    host_name: &str,
    host_steps: Vec<(super::Entry, ArtifactPlan, Vec<Step>, Option<String>)>,
    chosen_kiosk_auto_run: Option<String>,
    summary: &mut String,
    approve: &mut dyn FnMut(&str, Danger) -> Result<bool, String>,
) -> Result<(), String> {
    let total = host_steps.len();
    let mut any_enable_succeeded = false;
    for (index, (entry, artifact, steps, note)) in host_steps.into_iter().enumerate() {
        let tag = if artifact.is_library { " (library)" } else { "" };
        println!("\n  [{}/{total}] {}{} {} on {host_name}", index + 1, entry.product, tag, entry.version);
        // Fix 2 — the operator sees WHY, right above the approve prompt for
        // a dependency's own steps (Joel's ruling, module header). `None`
        // for an ordinary selected artifact.
        if let Some(note) = &note {
            println!("    {note}");
        }

        'artifact: for step in &steps {
            let what = format!("{} on {host_name}: {}", entry.product, describe(&step.kind));
            loop {
                if approve(&what, step.danger)? {
                    let result = execute_step(board, parsed, root, &artifact, &step.kind, approve);
                    let (label, is_failure) = step_outcome(&step.kind, &result);
                    summary.push_str(&format!("    {label} — {what}\n"));
                    println!("    {label}");
                    if !is_failure && matches!(step.kind, StepKind::Enable { .. }) {
                        any_enable_succeeded = true;
                    }
                    if is_failure {
                        summary.push_str(&format!(
                            "    remaining steps for {} are skipped; the other artifacts in \
                             this pass still run.\n",
                            entry.product
                        ));
                        break 'artifact;
                    }
                    break;
                }

                // Defect A' — a decline must itself be confirmed before it
                // takes effect. Declining the confirmation is not a second
                // decline of the original step; it re-asks that step, and
                // the `loop` does exactly that by falling through to its
                // next iteration.
                let confirm_what = format!(
                    "declining this skips the rest of {} on {host_name} — skip it?",
                    entry.product
                );
                if approve(&confirm_what, Danger::Dangerous)? {
                    summary.push_str(&format!(
                        "    declined — {what}. Remaining steps for {} are skipped; the other \
                         artifacts in this pass still run.\n",
                        entry.product
                    ));
                    println!("    declined");
                    break 'artifact;
                }
                println!("    (skip not confirmed — asking again)");
            }
        }
    }

    // Fix 1 — see this function's own doc comment. One converge for the
    // WHOLE host, after every group above, rather than one restart per
    // service (which `deploy_plan::plan` no longer even offers for a
    // freshly-enabled service — see `StepKind::ServiceRestart`'s own doc
    // comment for why one would be refused).
    if any_enable_succeeded {
        let what = format!(
            "converge {host_name}: apply reload confirm — starts every service just enabled above"
        );
        if approve(&what, Danger::Safe)? {
            // `StepKind::Realize` names no artifact of its own — every field
            // below is unread by its `execute_step` arm, which only calls
            // `board.perform_operation`.
            let placeholder = ArtifactPlan {
                product: String::new(),
                digest: String::new(),
                material_kinds: Vec::new(),
                material_files: Vec::new(),
                settings: Vec::new(),
                proxied: false,
                declares_plain: false,
                is_library: false,
                depends_on: Vec::new(),
                is_kiosk: false,
            };
            let result = execute_step(board, parsed, root, &placeholder, &StepKind::Realize, approve);
            let (label, _) = step_outcome(&StepKind::Realize, &result);
            summary.push_str(&format!("    {label} — {what}\n"));
            println!("    {label}");
        } else {
            summary.push_str(&format!(
                "    declined — {what}. The document already enables the service(s) above; \
                 nothing starts them until a converge (another `apply reload confirm`) or the \
                 machine's next boot.\n"
            ));
            println!("    declined");
        }
    }

    // R-214's consolidated choice, executed once — see this function's own
    // doc comment. `Danger::Safe`: the meaningful decision was the menu
    // choice itself, not this confirmation of it.
    if let Some(application) = chosen_kiosk_auto_run {
        let what =
            format!("make {application} {host_name}'s [kiosk] auto-run (chosen from several)");
        if approve(&what, Danger::Safe)? {
            let placeholder = ArtifactPlan {
                product: String::new(),
                digest: String::new(),
                material_kinds: Vec::new(),
                material_files: Vec::new(),
                settings: Vec::new(),
                proxied: false,
                declares_plain: false,
                is_library: false,
                depends_on: Vec::new(),
                is_kiosk: false,
            };
            let kind = StepKind::SetKioskAutoRun { application };
            let result = execute_step(board, parsed, root, &placeholder, &kind, approve);
            let (label, _) = step_outcome(&kind, &result);
            summary.push_str(&format!("    {label} — {what}\n"));
            println!("    {label}");
        } else {
            summary.push_str(&format!("    declined — {what}\n"));
            println!("    declined");
        }
    }

    Ok(())
}

/// A concise, human status for one step's outcome, and whether it counts as
/// a failure that should stop the rest of its artifact's own steps —
/// Defect B's second half.
///
/// **Never the control plane's own response text.** A real deploy's
/// end-of-run summary was an unreadable wall — raw `{"subject":"install",
/// ...}` JSON, multi-paragraph `means`/`next` fields, a full refusal body —
/// because the old code pushed [`execute_step`]'s returned string into the
/// summary verbatim. This reports what happened in one short line and drops
/// the rest; the detail is not kept anywhere, concise is the requirement.
///
/// **The one outcome that is not what it first looks like:** the control
/// plane refuses an install with [`rivet_core::deploy::ALREADY_RUNNING`]
/// when the host already holds the digest asked for.
/// `rivet_remote::deploy::run` already turns that into `Ok(..)` on its own
/// ordinary path (see that function's own header comment), but this checks
/// the same marker on an `Err` too, so however it arrives, a step carrying
/// it is a NORMAL, non-error no-op — reported `already current — skipped`,
/// never `FAILED`, and never the trigger for [`execute_host_batch`]'s skip
/// confirmation.
fn step_outcome(kind: &StepKind, result: &Result<String, String>) -> (String, bool) {
    let is_install = matches!(kind, StepKind::Install { .. });
    match result {
        Ok(said) if is_install && already_running(said) => {
            ("already current — skipped".to_string(), false)
        }
        Ok(_) if is_install => ("installed".to_string(), false),
        Ok(_) => ("done".to_string(), false),
        Err(e) if already_running(e) => ("already current — skipped".to_string(), false),
        Err(e) => (format!("FAILED: {}", first_line(e)), true),
    }
}

/// Whether `text` carries the control plane's own "already running this
/// artifact" refusal — [`step_outcome`]'s marker check, its own doc comment.
fn already_running(text: &str) -> bool {
    text.contains(rivet_core::deploy::ALREADY_RUNNING)
}

/// The first line of `text`, trimmed — [`step_outcome`]'s concise `FAILED`
/// reason. A real refusal is often several lines of context a human reading
/// a live status line has no room for; the first line is consistently
/// written to carry the reason on its own (every refusal in this codebase
/// leads with the one-line claim and follows with why), so taking it alone
/// rather than the whole string is what makes a `FAILED` line short without
/// hand-parsing any particular refusal's shape.
fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or(text).trim()
}

/// **One host's read-back state for one artifact, packed into [`HostState`]**
/// — the one reading both front ends plan from. The interview calls it before
/// it shows a plan; `cargo rivet install` calls it before it sends, so the
/// steps it prints afterwards are the ones the interview would have offered
/// for the same artifact on the same machine.
///
/// Returns the machine's document beside the state, because `install` reads
/// two more facts out of it — what the machine is called, and whether it is a
/// fleet server — and a second `get document` would be a second answer to
/// read.
///
/// * **The data directory is asked about only when the artifact declares
///   material.** [`deploy_plan::plan`] reads `data_dir_files` for nothing else,
///   so an application that declares no material costs the machine one round
///   trip fewer (`ADR-055` §3).
/// * **`settings_present` is the artifact's own declared settings** — the
///   Phase 1 deferral in the module header, applied here so neither caller can
///   forget it.
/// * **`current_digest` is `None`** — the other Phase 1 deviation.
/// * **`kiosk_auto_run`/`kiosk_auto_run_installed` are read only for a kiosk
///   artifact** (`ArtifactPlan::is_kiosk`) — R-214, `data_dir_files`'s own
///   reason: a service application costs the machine no round trip for a
///   question its own plan can never ask.
///
/// # Errors
///
/// Whatever `board`'s reads refuse with.
pub(crate) fn read_host_state_for(
    board: &mut dyn Remote,
    artifact: &ArtifactPlan,
) -> Result<(HostState, String), String> {
    let (data_dir_files, issues_material) = if artifact.material_files.is_empty() {
        (None, false)
    } else {
        // **A machine that could not say whether it holds its fleet's root
        // is planned as one that does not**: the material step is printed as
        // it always was, which is the plan a workstation-root machine needs
        // and one an operator can see is wrong; the opposite guess would
        // leave an application with nothing and no step saying so.
        (
            board.data_directory_files(&artifact.product)?,
            board.holds_fleet_root().unwrap_or(false),
        )
    };
    let document = board.read_document()?.unwrap_or_default();
    let standing = rivet_remote::deploy::standing(&document, &artifact.product);
    let (kiosk_auto_run, kiosk_auto_run_installed) = if artifact.is_kiosk {
        let current = rivet_remote::deploy::kiosk_auto_run(&document);
        // Only meaningful for a DIFFERENT application — `plan`'s own
        // `already_this_one` check is what this artifact's own name is for.
        // **Best-effort, deliberately**: `HostState::kiosk_auto_run_installed`'s
        // own doc comment says why an unconfirmed read is not the same as a
        // confirmed "no" and must not be escalated past it — this board
        // already answered every OTHER refusal in this function with `?`,
        // and this is the one read a failure of does not stop the plan,
        // because the fact it is for is a prompt default, not a step.
        let installed = match &current {
            rivet_remote::deploy::KioskAutoRun::Named(name) if name != &artifact.product => {
                board.data_directory_files(name).ok().flatten().is_some()
            }
            _ => false,
        };
        (current, installed)
    } else {
        (rivet_remote::deploy::KioskAutoRun::NoTable, false)
    };
    Ok((
        HostState {
            // Phase 1 deviation — see the module header. There is no cheap
            // "what digest is running" read yet, and offering Install when the
            // host already holds it is a harmless no-op (ADR-052 §7, §4.4).
            current_digest: None,
            data_dir_files,
            standing,
            // Phase 1 settings deferral — see the module header.
            settings_present: artifact.settings.clone(),
            kiosk_auto_run,
            kiosk_auto_run_installed,
            issues_material,
        },
        document,
    ))
}

/// One line describing a step, for the plan preview and the approve prompt —
/// and for the heading of each step `cargo rivet install` prints afterwards,
/// so a person and an agent read the same words for the same step.
pub(super) fn describe(kind: &StepKind) -> String {
    match kind {
        StepKind::Install { digest } => format!("install {}", short_digest(digest)),
        StepKind::StageMaterial { kinds, missing } => {
            format!("stage material ({}): {}", kinds.join(", "), missing.join(", "))
        }
        StepKind::Enable { service } => format!("enable {service} in the machine document"),
        StepKind::ServiceRestart { service } => format!("restart service {service}"),
        StepKind::RestartReader { service, reason, cost } => {
            format!("restart {service} — {reason} ({cost})")
        }
        StepKind::DeliverSetting { name } => {
            format!("deliver setting {name} (deferred to Phase 2 — will be skipped)")
        }
        StepKind::SetKioskAutoRun { application } => {
            format!("make {application} the machine's [kiosk] auto-run")
        }
        StepKind::Realize => {
            "converge (apply reload confirm) — starts every service just enabled".to_string()
        }
    }
}

/// The first twelve hex characters — long enough to be unambiguous in a menu
/// and short enough to read, the same truncation
/// [`super::find_library_digest`] uses for the same reason.
fn short_digest(digest: &str) -> String {
    digest.chars().take(12).collect()
}

// ---------------------------------------------------------------------------
// executing one approved step — the fixed vocabulary's remediations, §5/§10.1
// ---------------------------------------------------------------------------

/// Run one approved [`StepKind`] against `board`, returning whatever it said.
///
/// **Every arm calls an operation this tool already ships** (module header) —
/// this function is the composition, not a new capability.
///
/// # Errors
///
/// Whatever the underlying operation refuses with — a failed transfer, a
/// document this tool will not edit blind, a missing authority directory, or
/// the secret-store gate inside [`crate::secrets::run`] (never bypassed,
/// §4.3). `StageMaterial`'s one exception: a workstation with no server
/// certificate for this fleet name is offered a self-heal (Fix 3,
/// [`offer_to_mint_missing_certificate`]) before that refusal is returned.
fn execute_step(
    board: &mut dyn Remote,
    parsed: &DeployArgs,
    root: &Path,
    artifact: &ArtifactPlan,
    kind: &StepKind,
    approve: &mut dyn FnMut(&str, Danger) -> Result<bool, String>,
) -> Result<String, String> {
    match kind {
        StepKind::Install { digest } => {
            let object = super::materialise_for_deploy(root, digest)?;
            let install_args = DeployArgs {
                host: parsed.host.clone(),
                rvt: Some(object),
                package_args: Vec::new(),
                remote_dir: parsed.remote_dir.clone(),
                confirmed: true,
                identity: parsed.identity.clone(),
                pin: parsed.pin.clone(),
                // Enable is its own, separately approved step here (§5's
                // table has `enabled` as a distinct row) — never bundled into
                // the install itself.
                enable: false,
                // **The next step may be staging material**, which writes into
                // a data directory only the installer makes — so this step is
                // not done until THAT install is (`rivet_remote::deploy`'s
                // own header). `cargo rivet install` waits for the same reason.
                wait: true,
            };
            let mut build = |_: &[String]| -> Result<(PathBuf, String), String> {
                Err("no build: the artifact is already in the fleet repository".to_string())
            };
            rivet_remote::deploy::run(&install_args, &mut build, board)
        }
        StepKind::Enable { service } => {
            let Some(document) = board.read_document()? else {
                return Err(format!(
                    "there is no configuration document on this machine yet, so there is \
                     nothing to add [services.{service}] to — provisioning writes the first \
                     one, not a deploy tool."
                ));
            };
            let standing = rivet_remote::deploy::standing(&document, service);
            match rivet_remote::deploy::change_for(&standing, service) {
                Some(change) => board.edit_document(&change),
                None if standing == rivet_remote::deploy::Standing::Unreadable => Err(format!(
                    "the document names [services.{service}] in a shape this tool will not \
                     edit blind. Enable it yourself with `rivet apply edit` and re-run this \
                     pass."
                )),
                None => Ok(format!("{service}: already enabled")),
            }
        }
        StepKind::SetKioskAutoRun { application } => {
            let Some(document) = board.read_document()? else {
                return Err(
                    "there is no configuration document on this machine yet, so there is \
                     nothing to set [kiosk] auto-run in — provisioning writes the first one, \
                     not a deploy tool."
                        .to_string(),
                );
            };
            // Re-read fresh rather than trust the plan-time `HostState` —
            // `StepKind::Enable`'s own arm does the same, for the same
            // reason: an earlier step this same pass may already have
            // changed the document.
            let current = rivet_remote::deploy::kiosk_auto_run(&document);
            match rivet_remote::deploy::kiosk_change_for(&current, application) {
                Some(change) => {
                    board.edit_document(&change)?;
                    board.perform_operation(&["apply", "reload", "confirm"])?;
                    // The launcher, and never `aura-controller` or
                    // `aura-renderer` — `StepKind::SetKioskAutoRun`'s own
                    // doc comment says why this step has no reason to
                    // restart either of that pair.
                    board.perform_operation(&["apply", "service-restart", "rivet-launcher", "confirm"])
                }
                None => match current {
                    rivet_remote::deploy::KioskAutoRun::Named(name) if name == *application => {
                        Ok(format!("[kiosk] auto-run: already {application:?}"))
                    }
                    rivet_remote::deploy::KioskAutoRun::NoTable => Err(format!(
                        "this machine's document has no [kiosk] table, so there is no existing \
                         `auto-run` line to replace — and appending one risks silently \
                         commenting out whatever follows it. Add it yourself, after checking \
                         the document truly carries no [kiosk] table: `rivet apply edit \
                         {} --append '\n[kiosk]\nauto-run = \"{application}\"\n' confirm`, then \
                         `apply reload confirm` and `apply service-restart rivet-launcher \
                         confirm`.",
                        rivet_remote::deploy::DOCUMENT_PATH,
                    )),
                    _ => Err(format!(
                        "this machine's document names [kiosk] in a shape this tool will not \
                         edit blind. Set `auto-run = \"{application}\"` yourself with `rivet \
                         apply edit` and re-run this pass."
                    )),
                },
            }
        }
        StepKind::ServiceRestart { service } => {
            board.perform_operation(&["apply", "service-restart", service, "confirm"])
        }
        StepKind::RestartReader { service, .. } => {
            board.perform_operation(&["apply", "service-restart", service, "confirm"])
        }
        // Fix 1 — the one converge `execute_host_batch` constructs, at most
        // once per host, after every artifact's own steps above. Unlike
        // `ServiceRestart`, this is never refused by an enable this SAME
        // pass just wrote: it asks the supervisor to re-read the document,
        // not to run something its own still-unconverged view calls
        // disabled. `artifact` is unread here — see this arm's caller.
        StepKind::Realize => board.perform_operation(&["apply", "reload", "confirm"]),
        StepKind::StageMaterial { .. } => {
            let document = board.read_document()?.unwrap_or_default();
            let Some(hostname) = super::hostname_in(&document) else {
                return Err(format!(
                    "{}'s document did not say what it is called, so this tool cannot derive \
                     the fleet name `cargo rivet secrets` needs. `cargo rivet run {} -- get \
                     document` shows it once the machine has one.",
                    parsed.host, parsed.host
                ));
            };
            let name = format!("{hostname}.{}", super::FLEET_SUFFIX);
            let authority = default_authority().ok_or_else(|| {
                "no HOME or USERPROFILE, so there is no default authority directory for `cargo \
                 rivet secrets` — this pass cannot stage material."
                    .to_string()
            })?;
            let args = crate::secrets::SecretsArgs {
                application: artifact.product.clone(),
                authority: authority.clone(),
                name: Some(name.clone()),
                endpoint: rivet_remote::endpoint::Endpoint {
                    host: Some(parsed.host.clone()),
                    identity: parsed.identity.clone(),
                    pin: parsed.pin.clone(),
                },
                // The secret-store gate inside `crate::secrets::run` still
                // fires on this — the operator's approval just given is not
                // a substitute for it (§4.3).
                confirm: true,
            };
            match crate::secrets::run(&args) {
                Ok(said) => Ok(said),
                // **Fix 3 — self-heal for a board whose workstation copy is
                // missing** (provisioned before Fix 2 existed, or the file
                // was otherwise lost). Every OTHER refusal `secrets::run` can
                // give — the host unreachable, the secret-store gate, a
                // malformed file — is relayed unchanged; only this exact one
                // is caught.
                Err(why) if missing_server_certificate(&why, &name) => {
                    offer_to_mint_missing_certificate(&hostname, &name, &authority, &args, approve)
                }
                Err(why) => Err(why),
            }
        }
        // Not reachable from `deploy_plan::plan` in Phase 1: `settings_present`
        // is seeded so `DeliverSetting` never fires. Handled rather than
        // `unreachable!`'d: the vocabulary is fixed at compile time, not at the
        // call site, so an arm here costs nothing and a future Phase 2 that
        // starts emitting one is not a silent panic.
        StepKind::DeliverSetting { name } => Ok(format!(
            "{name}: setting delivery is deferred to Phase 2 (no typed-value source exists \
             yet) — skipped."
        )),
    }
}

/// `~/.rivet/ca` — the same default `cargo rivet secrets`' own parser applies
/// when `--authority` is not given (`crate::secrets::default_authority`,
/// private to that module and three lines long, so mirrored here rather than
/// widened to `pub(crate)` for one caller).
fn default_authority() -> Option<PathBuf> {
    crate::credentials::root().map(|home| home.join("ca"))
}

// ---------------------------------------------------------------------------
// Fix 3 — a `StageMaterial` step self-heals a board provisioned before Fix 2
// existed, rather than only relaying `crate::secrets::run`'s refusal.
// ---------------------------------------------------------------------------

/// Whether `why` is `crate::secrets::run`'s own (`server_pair`'s) refusal for
/// **this exact fleet name having no certificate on the workstation** —
/// `~/.rivet/ca/server-<fleet_name>.crt` absent — and not some other reason
/// `secrets::run` can fail: the host unreachable, the secret-store gate
/// (§4.3), a malformed PEM, several certificates with no `--name` to choose
/// between (which cannot happen here — a name is always passed).
///
/// Matched on the exact phrase `server_pair` composes
/// (`cmd/cargo-rivet/src/secrets.rs`), not on the whole refusal text, so a
/// change to its wording that drops this phrase fails LOUDLY — this stops
/// firing and Fix 3 stops self-healing — rather than this arm silently
/// matching something it should not.
fn missing_server_certificate(why: &str, fleet_name: &str) -> bool {
    why.contains(&format!("no certificate for {fleet_name} here"))
}

/// **Fix 3 itself.** `crate::secrets::run` just refused because this
/// workstation holds no `server-<fleet_name>.crt`/`.key` — the exact gap Fix 2
/// closes for every machine provisioned from here on, and the one a board
/// provisioned before Fix 2 existed (or whose workstation copy was otherwise
/// lost) still has. This offers to mint it right now rather than sending the
/// operator to a second command by hand.
///
/// # Why a subprocess, and not an in-process mint
///
/// The routine that mints a server leaf
/// (`cmd/rivet-provision/src/ca.rs`'s `issue_from`, and the name-constrained
/// `Authority` it is issued from) lives ONLY inside the `rivetos-provision`
/// binary crate — that crate has no `[lib]` target `cargo-rivet` could depend
/// on (checked: its `Cargo.toml` declares `[[bin]]` alone). Pulling that
/// routine into a shared crate blind, to save one subprocess call, is exactly
/// the kind of extraction the brief for this fix says not to force. So this
/// calls the one thing that already does it correctly end to end —
/// `rivet-provision ca server <hostname> <authority>`, the same subcommand
/// `cargo rivet install`'s own material-missing refusal
/// (`crate::fleet::refuse_if_material_missing`) already names — **on PATH**,
/// exactly as `rivet-provision` ships beside `cargo-rivet`.
///
/// **Follow-up this leaves behind**: if this on-demand mint earns its keep,
/// extracting `rivet-provision`'s `ca` module into a shared crate (`rivet-core`
/// or a new `rivet-ca`) would let this mint in-process instead of shelling
/// out, and would let `cmd/rivet-provision` itself depend on it rather than
/// owning the only copy. Not done here — unproven need, and a blind
/// extraction is a worse risk than one subprocess call.
///
/// # Errors
///
/// The operator declined (names the manual command); the subprocess could not
/// be started (`rivet-provision` not found on PATH); the subprocess exited
/// non-zero (its own stdout/stderr, quoted); or whatever the RETRY of
/// `crate::secrets::run` says — which, if it is this same "no certificate"
/// refusal again, means the mint wrote the file somewhere this call did not
/// look, and is reported rather than retried a second time.
fn offer_to_mint_missing_certificate(
    hostname: &str,
    fleet_name: &str,
    authority: &Path,
    args: &crate::secrets::SecretsArgs,
    approve: &mut dyn FnMut(&str, Danger) -> Result<bool, String>,
) -> Result<String, String> {
    let authority_display = authority.display().to_string();
    let offer = format!(
        "no server certificate for {fleet_name} here yet — mint one now \
         (`rivet-provision ca server {hostname} {authority_display}`) and stage it?"
    );
    if !approve(&offer, Danger::Safe)? {
        return Err(format!(
            "no certificate for {fleet_name} here, and minting one was declined. Mint it \
             yourself and re-run this step:\n\
             \x20 rivet-provision ca server {hostname} {authority_display}"
        ));
    }

    match std::process::Command::new("rivet-provision")
        .args(["ca", "server", hostname, &authority_display])
        .output()
    {
        Ok(output) if output.status.success() => {
            println!("    minted a server certificate for {fleet_name}");
        }
        Ok(output) => {
            return Err(format!(
                "`rivet-provision ca server {hostname} {authority_display}` exited {}:\n{}{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            ));
        }
        Err(why) => {
            return Err(format!(
                "could not run `rivet-provision ca server {hostname} {authority_display}`: \
                 {why}\n\
                 \x20 why  `rivet-provision` is expected on PATH, the way it ships beside \
                 `cargo-rivet`; this process could not start it at all.\n\
                 \x20 fix  mint it yourself and re-run this step:\n\
                 \x20\x20 rivet-provision ca server {hostname} {authority_display}"
            ));
        }
    }

    // Retry ONCE, now that the file should exist. Success or a DIFFERENT
    // refusal is returned as-is; this never loops back into this same
    // function a second time.
    crate::secrets::run(args)
}

#[cfg(test)]
#[path = "fleet_deploy_tests.rs"]
mod fleet_deploy_tests;
