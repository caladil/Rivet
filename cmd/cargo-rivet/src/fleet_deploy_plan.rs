//! **The needs planner for `cargo rivet deploy`'s interview** — the pure heart
//! of the interactive fleet deploy tool (`docs/design-candidates/Fleet-Deployment-Tool.md`
//! §10.2).
//!
//! Given one artifact's declared needs ([`ArtifactPlan`]) and one host's
//! read-back state ([`HostState`]), [`plan`] returns the ordered list of
//! [`Step`]s the interview will offer, each already carrying its danger class
//! (§4.2, ruling 5). **This module does no I/O and parses no manifest.** The
//! caller (`crate::fleet_deploy`) reads the fleet [`crate::fleet::Entry`], the
//! artifact's `manifest.toml`, and the host over the control plane, and packs
//! both sides into these plain structs; here the decision is a pure function of
//! them, so the correctness lives in a unit test with a fixture in and a step
//! list out, and it never needs a board (§10.6).
//!
//! **Danger class is set by the step kind, never per step** (§4.2). The
//! interview turns [`Danger::Safe`] into a `(Y/n)` prompt whose default is yes
//! and [`Danger::Dangerous`] into a `(y/N)` prompt whose default is no; this
//! module only classifies.
//!
//! **Data-restore is deferred** (ruling 2) and is deliberately not a step this
//! planner emits. It stays catalogued in §5 as a known need-type; if it
//! returns it slots in here as one more [`StepKind`] with no other change to
//! the tool's shape.
//!
//! # Fix 1 — a freshly-enabled service is realized by a converge, not a restart
//!
//! Driving a real deploy found this: a fresh host plans `enable <svc>` then
//! `service-restart <svc>`, and the restart is REFUSED — `ADR-017` §6,
//! `E_UNSAFE` — because at the moment the restart is asked for, the
//! supervisor's live view predates the document the enable just edited and
//! still calls the service disabled, and a converge-only
//! service control may only ask the supervisor to CONVERGE toward the
//! document, never to start what its current view still disables.
//!
//! `cmd/rivet-sysd/src/reload.rs` settles what actually starts it: a reload
//! (`apply reload confirm`) sets fields and **starts and stops nothing
//! itself** — the very next `Supervisor::converge` does, by the ordinary
//! rules every service is always subject to. So [`StepKind::ServiceRestart`]
//! now fires only for a service that is **already enabled and running**,
//! reinstalled this pass (new bytes that need a restart to take effect —
//! `Supervisor::take_application_definition`'s own doc comment: a running
//! process keeps the environment it was `execve`'d with, and nothing but a
//! restart, a crash, or the next boot picks up a reinstall). It never fires
//! for a service THIS pass is enabling for the first time.
//!
//! **The converge itself, [`StepKind::Realize`], is host-level and this
//! function never emits it.** A converge is one `apply reload confirm` for
//! the WHOLE host's enables at once — this function only ever sees one
//! artifact — so `crate::fleet_deploy::execute_host_batch` constructs
//! exactly one, after every selected artifact's (and every dependency's, Fix
//! 2 below) own install/material/enable steps, whenever any of them actually
//! enabled a service this pass. See [`StepKind::Realize`]'s own doc comment.
//!
//! # Fix 2 — an unmet `connects` dependency is its own approvable step
//!
//! An artifact can declare `connects = ["<owner>:<endpoint>"]` — it needs
//! another service to be running to work at all (`tempmon-rivet` needs
//! `rivet-bluetoothd`'s advertisements; `aura-renderer` needs
//! `aura-controller`). If that owner is not enabled on the host, the
//! supervisor will refuse to run the artifact that connects to it. Joel's
//! ruling: the interview should say it is about to install/enable a
//! dependency and ask `(y/n)` before installing the software that needs it.
//!
//! **This module only carries the data** — [`ArtifactPlan::depends_on`] — the
//! resolution (a present system service vs. a deployable artifact vs.
//! unresolvable) and the step-building both need the fleet repository and the
//! host, which this pure function does not touch; both live in
//! `crate::fleet_deploy::dependency_step_group` and its caller.
//!
//! [`ArtifactPlan`]: crate::fleet::deploy_plan::ArtifactPlan
//! [`ArtifactPlan::depends_on`]: crate::fleet::deploy_plan::ArtifactPlan::depends_on
//! [`HostState`]: crate::fleet::deploy_plan::HostState
//! [`plan`]: crate::fleet::deploy_plan::plan
//! [`Step`]: crate::fleet::deploy_plan::Step
//! [`Danger::Safe`]: crate::fleet::deploy_plan::Danger::Safe
//! [`Danger::Dangerous`]: crate::fleet::deploy_plan::Danger::Dangerous
//! [`StepKind`]: crate::fleet::deploy_plan::StepKind
//! [`StepKind::ServiceRestart`]: crate::fleet::deploy_plan::StepKind::ServiceRestart
//! [`StepKind::Realize`]: crate::fleet::deploy_plan::StepKind::Realize

// The link targets above are spelled from the crate root on purpose. This
// module is declared with an outer `///` comment on its `mod` line in
// `fleet.rs`, and rustdoc then resolves every link in these `//!` lines from
// the PARENT module's scope, where a bare `plan` or `StepKind` names nothing.

use rivet_remote::deploy::Standing;

/// The prompt default a step carries — §4.2 / ruling 5.
///
/// The class, not the operator, decides the default answer, so a safe step is
/// one keypress (`Enter` = yes) and a dangerous one has to be typed (`Enter` =
/// no).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Danger {
    /// Reversible, brief, or non-destructive → `(Y/n)`, default yes.
    Safe,
    /// Destroys data, holds a private key, or takes the machine down → `(y/N)`,
    /// default no.
    Dangerous,
}

/// One post-install step, ready to render as an approve line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// What the step does — the interview matches on this to run the right
    /// mutation core once the step is approved.
    pub kind: StepKind,
    /// The prompt default, from [`StepKind`]'s fixed class (§5).
    pub danger: Danger,
}

/// The fixed, tool-understood vocabulary of remediations (§4.1, §5).
///
/// **An application declares NEEDS; this enum is the REMEDIATIONS the tool
/// knows.** It is grown only by teaching the tool a new variant — never by an
/// application shipping a command string — which is the load-bearing security
/// line (§4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepKind {
    /// Transfer + install by digest. Emitted when the host's current digest is
    /// not already the artifact's (idempotent — §4.4). Carries the artifact
    /// digest to install.
    Install { digest: String },
    /// Stage per-machine material (e.g. `fleet-tls`) from the operator CA.
    /// **Dangerous** (holds a private key) and still passes through the
    /// secret-store gate (§4.3) — the `(y/N)` here is not a substitute for it.
    /// Carries the declared material kinds and the specific files found absent.
    StageMaterial { kinds: Vec<String>, missing: Vec<String> },
    /// Add `[services.<name>] enabled = true` to the machine document. Emitted
    /// when the document does not already say the service is enabled.
    Enable { service: String },
    /// `apply service-restart <service>`. Emitted only when this pass
    /// reinstalled a service that was **already enabled and running** — new
    /// bytes a restart is what picks up. **Never emitted for a service this
    /// SAME pass is enabling for the first time** (Fix 1, module header): the
    /// supervisor's converge-only service control refuses that one with
    /// `E_UNSAFE` — `THE DOCUMENT DISABLES IT` in its own still-unconverged
    /// view — and [`StepKind::Realize`] is what starts it instead.
    ServiceRestart { service: String },
    /// **Restart a reader whose behavior this pass may have made stale** —
    /// `.rulings/` R-224, `ADR-055` §6. Never done automatically: the
    /// interview offers it as a step the operator approves, defaulting to
    /// **No** ([`StepKind::danger`] classifies every entry [`Danger::Dangerous`]);
    /// the scripted `cargo rivet install` prints the same command among what
    /// remains; the supervisor never restarts a reader by itself.
    ///
    /// **The mechanism is a table, not a special case** — [`STALE_READERS`].
    /// Teaching the deploy a new reader is one more entry there, and the
    /// prompt, the default, the printed command and the tests follow from it
    /// with no other change to [`plan`]. `service` and `reason` are that
    /// entry's own [`StaleReader::service`] and whatever its
    /// [`StaleReader::trigger`] found; `cost` is the entry's own
    /// [`StaleReader::cost`], carried onto the step so both front ends can
    /// state it beside the reason without a second lookup into the table.
    ///
    /// **`rivet-proxyd` is the first entry.** It fires whenever a pass
    /// installs or reinstalls a proxied artifact — the same trigger
    /// `StepKind::ProxyRestart` used before this table existed
    /// (`memory: rivet-proxy-restart-after-listen-change`) — and its reason
    /// names port 80 exactly when the artifact declares a `plain:` rule,
    /// since `ADR-046` §7 ties that listener's open-or-closed state to
    /// whether one exists anywhere in the routing table.
    RestartReader { service: String, reason: String, cost: &'static str },
    /// Write a declared setting the machine document does not carry. Carries
    /// the setting name (the value is the interview's concern, not the
    /// planner's).
    DeliverSetting { name: String },
    /// **Make `application` the machine's `[kiosk] auto-run`** — `.rulings/`
    /// R-214. Emitted only for a kiosk application ([`ArtifactPlan::is_kiosk`])
    /// this pass installs or enables, and only when the document does not
    /// already name it (see [`plan`]'s own §3.5). Carries the artifact's own
    /// product, which is always what `application` names — this step is
    /// never constructed for one artifact naming another.
    ///
    /// Execution edits `[kiosk] auto-run` with `--expect`/`--replace`
    /// (`rivet_remote::deploy::kiosk_change_for`), then converges
    /// (`apply reload confirm`) and restarts `rivet-launcher` alone, so the
    /// launcher reads the new setting now rather than at the machine's next
    /// boot. **Never any OTHER service** — in particular, never
    /// `aura-controller` or `aura-renderer`: this step's whole job is making
    /// the launcher pick up one line, and it has no reason to touch what the
    /// launcher goes on to auto-run.
    SetKioskAutoRun { application: String },
    /// **`apply reload confirm` — converge the document.** Fix 1, module
    /// header. **Host-level, not per-artifact**: [`plan`] never constructs
    /// one — it only ever sees one artifact, and a converge is one call for
    /// the WHOLE host's enables at once. So it belongs to the host batch:
    /// `crate::fleet_deploy::execute_host_batch` constructs it, exactly once
    /// per host, after every selected
    /// artifact's (and every unmet dependency's, Fix 2) own steps, and only
    /// when at least one of them actually enabled a service this pass.
    Realize,
}

impl StepKind {
    /// The fixed danger class for this kind (§5, ruling 5): install / enable /
    /// service-restart / settings are safe; staging material and restarting a
    /// stale reader are dangerous. A reboot step is not in the vocabulary yet
    /// — §10.2 rules that Phase 1 never emits one, so the variant is added
    /// the day Phase 2's "reports it is required" logic first constructs it.
    ///
    /// **`SetKioskAutoRun` is the one exception**, and this is its
    /// CONTEXT-INDEPENDENT default only — `R-214`: [`plan`] builds that one
    /// `Step` directly rather than through [`push`], and sets `Dangerous`
    /// instead whenever offering it would replace a DIFFERENT, installed
    /// kiosk application's own auto-run (`plan`'s own §3.5 has the
    /// reasoning). Nothing else in this tool varies a step's danger by what
    /// it is about to do; this one does because that specific case is an
    /// operator's earlier, deliberate choice of a different application to
    /// show — a `(Y/n)` default would make declining the ONE thing that
    /// takes an extra keystroke instead of the thing that changes it.
    ///
    /// **`.rulings/` R-224 rules every [`StepKind::RestartReader`]
    /// `Dangerous`**, whichever entry of [`STALE_READERS`] produced it,
    /// because every one of them is an outage for something behind it that
    /// only the operator can time.
    #[must_use]
    pub fn danger(&self) -> Danger {
        match self {
            StepKind::Install { .. }
            | StepKind::Enable { .. }
            | StepKind::ServiceRestart { .. }
            | StepKind::DeliverSetting { .. }
            | StepKind::SetKioskAutoRun { .. }
            | StepKind::Realize => Danger::Safe,
            StepKind::StageMaterial { .. } | StepKind::RestartReader { .. } => Danger::Dangerous,
        }
    }
}

/// The artifact side of the planner input — what the selected artifact and its
/// `manifest.toml` declare, packed by `crate::fleet_deploy` from a
/// [`crate::fleet::Entry`] and the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactPlan {
    /// The product / service name — the data directory the material goes into
    /// and the `[services.<name>]` the enable step names.
    pub product: String,
    /// The artifact's own digest — what [`StepKind::Install`] installs, and
    /// what a host's current digest is compared against.
    pub digest: String,
    /// The declared material kinds (`manifest.material()`, e.g. `fleet-tls`),
    /// as their wire tokens. Empty is the ordinary answer.
    pub material_kinds: Vec<String>,
    /// Every file the declared material kinds require, flattened
    /// (`Material::files()` for each kind). This is what a host's data
    /// directory is checked against to find the material gap.
    pub material_files: Vec<String>,
    /// The declared setting names (`manifest.settings()`). Empty is ordinary.
    pub settings: Vec<String>,
    /// Whether the manifest declares a `proxy-path` — a proxied application
    /// whose route `rivet-proxyd` must re-derive after a listen change.
    pub proxied: bool,
    /// **Whether the manifest declares any `plain:` `proxy-auth-paths` rule**
    /// — `.rulings/` R-224, `ADR-046` §7. Read directly from the signed
    /// manifest (`crate::fleet_deploy::read_artifact_plan_with`), never
    /// guessed: `rivet-proxyd`'s [`STALE_READERS`] entry names port 80 in its
    /// reason exactly when this is `true`, since `ADR-046` §7 ties that
    /// listener's open-or-closed state to a `plain:` rule.
    pub declares_plain: bool,
    /// **Whether this artifact is a library (`kind == "library"`) rather than
    /// an application** — Defect C. A library is INSTALL-ONLY: nothing on a
    /// host *starts* one, so [`plan`] emits [`StepKind::Install`] for it and
    /// nothing else, whatever `material_kinds`/`settings`/`proxied` above
    /// happen to hold. Set by `crate::fleet_deploy::read_artifact_plan` from
    /// the artifact's own manifest kind, never guessed from the product name.
    pub is_library: bool,
    /// **The distinct owners of the artifact's declared `connects`
    /// endpoints** (`manifest.connects()`, e.g. `["rivet-bluetoothd"]` for a
    /// manifest declaring `connects = ["rivet-bluetoothd:advertisements"]`) —
    /// Fix 2, module header. Deduplicated and in declared order. **This
    /// module does not act on it**: resolving a name to a present system
    /// service or a deployable artifact, and deciding what — if anything —
    /// needs approving, is `crate::fleet_deploy::dependency_step_group`'s job,
    /// because it needs the fleet repository and the host and this function
    /// takes neither. **v1 is one level deep**: a dependency's OWN
    /// `depends_on` is never read by anything in this tool (converge-first —
    /// resolve what was asked for before chasing what that names in turn).
    pub depends_on: Vec<String>,
    /// **Whether the artifact's own manifest declares itself a kiosk
    /// application** — `.rulings/` R-214. Decided by the same fact the
    /// supervisor and `cmd/rivet-launcher` use to put an application on the
    /// kiosk menu (`cmd/rivet-sysd/src/supervise.rs::offered_displays`):
    /// `devices = ["display"]`, or `connects` naming the compositor's
    /// endpoint. Set by `crate::fleet_deploy::read_artifact_plan` from the
    /// manifest directly — never guessed from the product name.
    pub is_kiosk: bool,
}

/// The host side of the planner input — one machine's read-back state, packed
/// by `crate::fleet_deploy` from the [`rivet_remote::deploy::Remote`] reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostState {
    /// The digest of this product currently installed on the host, or `None`
    /// when the product is not installed here. Compared against
    /// [`ArtifactPlan::digest`] to decide the install step (idempotent).
    pub current_digest: Option<String>,
    /// The names in the application's own data directory, or `None` when that
    /// directory is absent — which the installer makes, so `None` means "not
    /// installed here" and is not the same as an empty list
    /// (`Remote::data_directory_files`).
    pub data_dir_files: Option<Vec<String>>,
    /// What the machine document says about the service —
    /// [`rivet_remote::deploy::standing`].
    pub standing: Standing,
    /// The declared setting names the machine document already carries, so a
    /// declared setting not in this list is undelivered.
    pub settings_present: Vec<String>,
    /// What the machine document says about `[kiosk] auto-run` —
    /// [`rivet_remote::deploy::kiosk_auto_run`]. `.rulings/` R-214.
    pub kiosk_auto_run: rivet_remote::deploy::KioskAutoRun,
    /// **Only meaningful when [`HostState::kiosk_auto_run`] is
    /// [`rivet_remote::deploy::KioskAutoRun::Named`]**: whether THAT named
    /// application has a data directory here. `false` for anything else,
    /// including a read this host's own reply could not confirm — an
    /// UNCONFIRMED installation is not grounds to call offering the step
    /// dangerous, so the uncertain case reads the same as "not installed".
    pub kiosk_auto_run_installed: bool,
    /// **Whether this machine holds its fleet's root and so issues every
    /// `fleet-tls` application its certificate itself** — `ADR-063` §4. Read
    /// from `rivet get fleet-root`. When it does, no material is staged from a
    /// workstation: the machine issues it at install, and a workstation's
    /// authority could not sign for this fleet's root anyway.
    pub issues_material: bool,
}

/// **One entry in the table of stale readers** [`plan`] drives — `.rulings/`
/// R-224, `ADR-055` §6. Teaching the deploy a new reader is adding one of
/// these to [`STALE_READERS`], with no other change to [`plan`] itself; the
/// module's own tests prove that with a test-only second entry passed
/// through [`plan_with`].
#[derive(Clone, Copy)]
pub struct StaleReader {
    /// The service [`StepKind::RestartReader::service`] names, and what
    /// `apply service-restart` restarts.
    pub service: &'static str,
    /// **Pure**: does THIS artifact's pass make `service` stale on `host`,
    /// and if so, why — the one-line reason [`StepKind::RestartReader::reason`]
    /// carries. `installed_this_pass` is [`plan`]'s own (a fresh install or a
    /// same-pass reinstall).
    pub trigger: fn(&ArtifactPlan, &HostState, bool) -> Option<String>,
    /// **What restarting `service` costs everything behind it** — carried
    /// onto [`StepKind::RestartReader::cost`] so both front ends can state it
    /// beside the reason (`ADR-055` §6: whether now is a good moment is the
    /// operator's alone to judge).
    pub cost: &'static str,
}

/// [`STALE_READERS`]'s only entry so far. Fires whenever this pass installs
/// or reinstalls a proxied artifact — the same trigger `StepKind::ProxyRestart`
/// used before this table existed
/// (`memory: rivet-proxy-restart-after-listen-change`) — and names port 80 in
/// its reason exactly when the artifact declares a `plain:` rule, since
/// `ADR-046` §7 ties that listener's open-or-closed state to one.
fn proxy_restart_reason(
    artifact: &ArtifactPlan,
    _host: &HostState,
    installed_this_pass: bool,
) -> Option<String> {
    if !(artifact.proxied && installed_this_pass) {
        return None;
    }
    Some(if artifact.declares_plain {
        "this pass changes what rivet-proxyd serves, and port 80 is what changes for its plain \
         route (ADR-046 §7)"
            .to_string()
    } else {
        "this pass changes what rivet-proxyd serves, and its route needs to be re-derived"
            .to_string()
    })
}

/// **The table of stale readers** — `.rulings/` R-224, `ADR-055` §6.
/// `rivet-proxyd` is the first entry; the launcher is the natural second
/// (`ADR-055` §6) and is not one yet.
pub const STALE_READERS: &[StaleReader] = &[StaleReader {
    service: "rivet-proxyd",
    trigger: proxy_restart_reason,
    cost: "every application behind the machine's front door is unreachable while it restarts",
}];

/// **Compute the ordered post-install steps for one artifact on one host.**
///
/// Pure and total: no I/O, no board, no panics — a fixture in, the step list
/// out, in the fixed order of §10.2 (install → material → enable → kiosk
/// auto-run → service-restart → stale-reader restarts → settings; reboot is
/// not auto-emitted). Each step carries its danger class from
/// [`StepKind::danger`]. An idempotent re-run yields fewer steps as needs are
/// met (§4.4); a host already at the artifact's digest with everything
/// satisfied yields an empty list.
///
/// **Never [`StepKind::Realize`]** — Fix 1, module header. That step is a
/// fact about the whole HOST's pass (one converge realizes every artifact's
/// enables at once), and this function only ever sees one artifact.
///
/// See the module header for what each side of the input means and where it is
/// read. Always drives [`STALE_READERS`], the real table — [`plan_with`] is
/// the test seam that lets the module's own tests substitute one.
#[must_use]
pub fn plan(artifact: &ArtifactPlan, host: &HostState) -> Vec<Step> {
    plan_with(STALE_READERS, artifact, host)
}

/// [`plan`], parameterized over the table of stale readers it drives — the
/// test seam that proves [`STALE_READERS`] is the extension point: a
/// test-only second entry, passed here instead of the real table, produces
/// its own step with no other change to this function (module tests).
/// [`plan`] always calls this with [`STALE_READERS`] itself.
#[must_use]
pub(crate) fn plan_with(readers: &[StaleReader], artifact: &ArtifactPlan, host: &HostState) -> Vec<Step> {
    let mut steps = Vec::new();

    // 1. Install — idempotent: nothing to do once the host already holds this
    // artifact's exact digest. `current_digest: None` (nothing installed here
    // yet) and `current_digest: Some(other)` (a different build) both need it.
    let installed_this_pass = host.current_digest.as_deref() != Some(artifact.digest.as_str());
    if installed_this_pass {
        push(&mut steps, StepKind::Install { digest: artifact.digest.clone() });
    }

    // A library is INSTALL-ONLY (Defect C, `ArtifactPlan::is_library`'s own
    // doc): nothing below applies to one — nothing enables it, restarts a
    // service for it, or delivers a setting to it, because nothing on a host
    // *starts* a library; the host discovers it once it is present. This
    // returns before steps 2-6 rather than gating each individually, so a
    // library's plan is structurally exactly `[Install]` (or `[]`, idle) —
    // never a partial application sequence.
    if artifact.is_library {
        return steps;
    }

    // 2. StageMaterial — only when the artifact declares material at all, and
    // only for the files actually absent. `data_dir_files: None` means the
    // application's data directory itself does not exist (never installed
    // here), which is NOT the same as `Some(vec![])` (installed, directory
    // empty) — in the `None` case every declared file is missing by
    // definition, because there is nowhere for any of them to be.
    //
    // Never on a machine that issues its own material (`ADR-063` §4): it
    // issues the files at install, so there is nothing to stage.
    if !artifact.material_files.is_empty() && !host.issues_material {
        let missing: Vec<String> = match &host.data_dir_files {
            None => artifact.material_files.clone(),
            Some(present) => artifact
                .material_files
                .iter()
                .filter(|f| !present.contains(f))
                .cloned()
                .collect(),
        };
        if !missing.is_empty() {
            push(
                &mut steps,
                StepKind::StageMaterial { kinds: artifact.material_kinds.clone(), missing },
            );
        }
    }

    // 3. Enable — only when the document says the service is not already on.
    // `Unreadable` is deliberately left alone: the document names the service
    // in a shape this tool did not write, and it will not edit that blind
    // (mirrors `rivet_remote::deploy::Standing::Unreadable`'s own doc comment).
    let enabled_this_pass = matches!(host.standing, Standing::Absent | Standing::Disabled);
    if enabled_this_pass {
        push(&mut steps, StepKind::Enable { service: artifact.product.clone() });
    }

    // 3.5 SetKioskAutoRun — `.rulings/` R-214. Offered only for a kiosk
    // application this SAME pass installs or enables — never for one that
    // was already fully deployed, which would be offering to move the
    // screen away from whatever the operator last chose for no reason this
    // pass gave — and never when the document already names it, which is
    // "offer it only when it would change something" read literally.
    //
    // The step's OWN danger varies with what accepting it would cost,
    // unlike every other step in this vocabulary (`StepKind::danger`'s own
    // doc comment): replacing a DIFFERENT kiosk application's auto-run is
    // undoing a choice that reads as deliberate exactly when that other
    // application is actually installed here, so the prompt defaults to
    // `No` only then. A `[kiosk]` table this tool cannot read a name out of
    // (`KioskAutoRun::NoTable`/`Unreadable`) is not "a different one, kept
    // on purpose" — there is no name to have chosen on purpose — so it
    // defaults `Yes` like an ordinary safe step, and execution is what
    // refuses to guess at the edit itself.
    if artifact.is_kiosk && (installed_this_pass || enabled_this_pass) {
        let already_this_one = matches!(
            &host.kiosk_auto_run,
            rivet_remote::deploy::KioskAutoRun::Named(name) if name == &artifact.product
        );
        if !already_this_one {
            let replaces_a_different_installed_app = host.kiosk_auto_run_installed
                && matches!(
                    &host.kiosk_auto_run,
                    rivet_remote::deploy::KioskAutoRun::Named(name) if name != &artifact.product
                );
            steps.push(Step {
                kind: StepKind::SetKioskAutoRun { application: artifact.product.clone() },
                danger: if replaces_a_different_installed_app { Danger::Dangerous } else { Danger::Safe },
            });
        }
    }

    // 4. ServiceRestart — Fix 1 (module header). Tied to install, as before,
    // but NOT emitted when step 3 just above ALSO fired this pass
    // (`Standing::Absent | Standing::Disabled`): that service is not running
    // yet, and a restart of it is refused — `ADR-017` §6's converge-only
    // guard reads the supervisor's own still-unconverged view, which still
    // calls it disabled at the moment the restart is asked for. The host-
    // level converge (`StepKind::Realize`, constructed in
    // `crate::fleet_deploy::execute_host_batch`, never here) is what starts
    // it instead.
    //
    // Every OTHER standing still gets a restart on install: `Enabled` is
    // exactly the reinstall-of-an-already-running-service case a restart
    // exists for, and `Unreadable` is left as this tool already leaves it
    // everywhere else — untouched, on the assumption nothing here changed
    // whether it is running.
    if installed_this_pass && !matches!(host.standing, Standing::Absent | Standing::Disabled) {
        push(&mut steps, StepKind::ServiceRestart { service: artifact.product.clone() });
    }

    // 5. RestartReader — the table of stale readers (`.rulings/` R-224,
    // `ADR-055` §6). Every entry's pure trigger runs in table order, and a
    // firing entry becomes its own `StepKind::RestartReader`, carrying that
    // entry's service, its trigger's own reason, and its static cost.
    for reader in readers {
        if let Some(reason) = (reader.trigger)(artifact, host, installed_this_pass) {
            push(
                &mut steps,
                StepKind::RestartReader {
                    service: reader.service.to_string(),
                    reason,
                    cost: reader.cost,
                },
            );
        }
    }

    // 6. DeliverSetting — one step per declared setting the machine document
    // does not already carry, in the artifact's own declared order.
    for name in &artifact.settings {
        if !host.settings_present.contains(name) {
            push(&mut steps, StepKind::DeliverSetting { name: name.clone() });
        }
    }

    // 7. A reboot is never auto-emitted in Phase 1 (§10.2) — there is no
    // `StepKind::Reboot` in the vocabulary to emit.

    steps
}

/// Append `kind` as a [`Step`], deriving `danger` from [`StepKind::danger`] so
/// classification always has exactly one source (module header, §4.2).
fn push(steps: &mut Vec<Step>, kind: StepKind) {
    steps.push(Step { danger: kind.danger(), kind });
}

#[cfg(test)]
#[path = "fleet_deploy_plan_tests.rs"]
mod fleet_deploy_plan_tests;
