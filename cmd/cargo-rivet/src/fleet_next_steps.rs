//! **What remains after `cargo rivet install`, from the interview's own
//! planner.**
//!
//! `cargo rivet deploy` is the interview, for a person at a terminal, and
//! `cargo rivet install` is the scripted verb an agent uses. **One planner
//! drives both** (`ADR-055` §3): the interview asks
//! [`deploy_plan::plan`] what an artifact needs on a host and runs each step it
//! approves; `install` asks the same function the same question, about the same
//! `ArtifactPlan` and `HostState` — read by the same two functions,
//! `read_artifact_plan_with` and `read_host_state_for` — and prints what it did
//! not do as exact commands. A step taught to the planner for the person
//! therefore reaches the agent in the same change, rather than waiting for
//! somebody to notice that two lists of advice have drifted.
//!
//! **An unmet `connects` dependency is planned the interview's way too** —
//! `interview::resolve_dependencies` and `interview::dependency_step_group`,
//! with its steps first — so an application that needs a switched-off system
//! service says so after an install exactly as it does in the interview.
//!
//! **Each command is what the interview's `execute_step` does for that step**,
//! spelled as the verb that does it from a workstation: material through
//! `cargo rivet secrets`, the enable as the `apply edit` the interview makes,
//! the converge as `apply reload`, a restart as `apply service-restart`.
//!
//! **One step is `install`'s alone: the fleet sync.** On a fleet server
//! (`rivet-fleetd` enabled), `rivet-fleetd`'s repository store holds the
//! objects a member is installed from, and an install from this workstation
//! does not put the artifact there — `cargo rivet fleet sync` does. It is
//! reported as the last step and never run: sending to that store is a second
//! decision about a second machine's state.

use std::path::Path;

use rivet_core::package::application::{MATERIAL_RULE, Material};
use rivet_remote::deploy::{ConfigChange, DOCUMENT_PATH, DeployArgs, Standing};

use super::deploy_plan::{self, ArtifactPlan, Danger, HostState, StepKind};

/// The name of the service whose being enabled makes a machine a fleet server.
const FLEETD: &str = "rivet-fleetd";

/// **How the printed commands reach the machine** — spelled the way this
/// install was reached, so an agent can run each line as it stands.
pub(crate) struct Reach<'a> {
    /// The connection this install used. `--host`, `--identity` and `--pin`
    /// are always filled in by the time an install runs — a device name was
    /// resolved into them.
    pub args: &'a DeployArgs,
    /// The device name the install was given, when it was given one. Verbs that
    /// take one (`run`, `fleet sync`) are then spelled with it; `secrets` takes
    /// none and is always spelled with the three flags.
    pub device: Option<&'a str>,
    /// `--at DIR`, when this install named a fleet repository other than the
    /// default — `fleet sync` must read the same one.
    pub at: Option<&'a Path>,
    /// The fleet authority directory `rivet-provision ca server` writes into.
    pub authority: &'a str,
}

impl Reach<'_> {
    /// `cargo rivet run <device>`, or the three flags when no device was named.
    fn run(&self, words: &str) -> String {
        match self.device {
            Some(device) => format!("cargo rivet run {device} -- {words}"),
            None => format!("cargo rivet run {}-- {words}", super::connection_flags(self.args)),
        }
    }

    /// `cargo rivet fleet sync` for this machine, `--at` included when given.
    fn sync(&self) -> String {
        let at = self.at.map_or_else(String::new, |dir| format!("--at {} ", quoted(&dir.display().to_string())));
        match self.device {
            Some(device) => format!("cargo rivet fleet sync {device} {at}confirm"),
            None => format!("cargo rivet fleet sync {}{at}confirm", super::connection_flags(self.args)),
        }
    }
}

/// A path as a shell should receive it: single-quoted when it holds a space.
fn quoted(text: &str) -> String {
    if text.chars().any(char::is_whitespace) { format!("'{text}'") } else { text.to_string() }
}

/// **The planner's steps that remain once this install has happened.**
///
/// `before` is the host as [`super::interview::read_host_state_for`] read it
/// BEFORE anything was sent — the reading the interview would have planned
/// from. The steps are [`deploy_plan::plan`]'s for that reading, less what the
/// install itself did, plus the one host-level converge the interview adds
/// after enabling anything (`StepKind::Realize`):
///
/// * **`Install` is done.** It is dropped.
/// * **`installed` false** is a machine that refused the request as already
///   running this artifact: nothing changed, so the planner is asked with the
///   host at this digest, and a restart that would pick up new bytes is not
///   owed.
/// * **`enabled_by_install`** is `--enable` having edited the document before
///   the send. That `Enable` is done. The supervisor re-reads the document when
///   an install finishes, so a converge is owed for it only when no install
///   happened.
#[must_use]
pub(crate) fn remaining_after_install(
    artifact: &ArtifactPlan,
    before: &HostState,
    enabled_by_install: bool,
    installed: bool,
) -> Vec<StepKind> {
    let mut host = before.clone();
    if !installed {
        host.current_digest = Some(artifact.digest.clone());
    }
    let mut steps: Vec<StepKind> = deploy_plan::plan(artifact, &host)
        .into_iter()
        .map(|step| step.kind)
        .filter(|kind| !matches!(kind, StepKind::Install { .. }))
        .filter(|kind| !(enabled_by_install && matches!(kind, StepKind::Enable { .. })))
        .collect();
    let enable_pending = steps.iter().any(|kind| matches!(kind, StepKind::Enable { .. }));
    if enable_pending || (enabled_by_install && !installed) {
        steps.push(StepKind::Realize);
    }
    steps
}

/// One step an agent is told to take: the interview's own description of it,
/// its danger class, the commands, in order, and anything to know first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NextStep {
    /// [`super::interview::describe`]'s line for a planner step — the words the
    /// interview shows a person for the same step.
    pub heading: String,
    pub danger: Danger,
    pub commands: Vec<String>,
    pub note: Option<String>,
    /// **Why this step is [`Danger::Dangerous`]**, shown beside the numbered
    /// heading in [`render`] — `heading` already says WHAT the step does;
    /// this says WHY it defaults to `(y/N)`. `None` for a [`Danger::Safe`]
    /// step, since only a `(y/N)` default has a reason to explain, and for
    /// the fleet-sync pseudo-step, which carries no [`StepKind`] of its own.
    pub danger_reason: Option<String>,
}

/// [`NextStep::danger_reason`] for one planner step — `StageMaterial` handles
/// a private key; a stale-reader restart's own entry states what it costs
/// (`StepKind::RestartReader::cost`). Every other kind is
/// [`Danger::Safe`] and carries none.
fn danger_reason_for(kind: &StepKind) -> Option<String> {
    match kind {
        StepKind::StageMaterial { .. } => Some("handles a private key".to_string()),
        StepKind::RestartReader { cost, .. } => Some((*cost).to_string()),
        _ => None,
    }
}

/// **The two lines that stage per-machine material**, in the order to run them:
/// the machine's server pair minted from the authority, then installed into the
/// application's data directory. One function for both places they are
/// printed — this step, and install's refusal to put an application into
/// service without its material — so the two cannot name different commands.
#[must_use]
pub(crate) fn material_commands(product: &str, hostname: Option<&str>, reach: &Reach<'_>) -> Vec<String> {
    let bare = hostname.unwrap_or("<hostname>");
    // **The device's name when there is one**, as the other steps are spelled:
    // the flags form carries an identity path, which a shell other than the
    // one it was printed for can rewrite before `secrets` ever sees it.
    let target = reach.device.map_or_else(
        || format!("{product} {}", super::connection_flags(reach.args)),
        |device| format!("{device} {product} "),
    );
    vec![
        format!("rivet-provision ca server {bare} {}", reach.authority),
        format!("cargo rivet secrets {target}--name {} confirm", fleet_name(hostname)),
    ]
}

/// The name a machine's leaf is minted for: its hostname under the fleet's
/// suffix, or a placeholder when the document did not say.
fn fleet_name(hostname: Option<&str>) -> String {
    hostname.map_or_else(|| "<hostname>.local".to_string(), |h| format!("{h}.{}", super::FLEET_SUFFIX))
}

/// The note that goes with [`material_commands`].
fn material_note(hostname: Option<&str>, reach: &Reach<'_>) -> String {
    let name = fleet_name(hostname);
    let mut note = format!(
        "the first line is once per MACHINE — skip it if {} already holds `server-{name}.crt`. \
         {MATERIAL_RULE}.",
        reach.authority,
    );
    if hostname.is_none() {
        note.push_str(&format!(
            " This machine's document did not say what it is called: `{}` shows it.",
            reach.run("get document"),
        ));
    }
    note
}

/// **The commands for one planner step** — what the interview's `execute_step`
/// does for it, as a line to type.
fn step_for(
    kind: &StepKind,
    artifact: &ArtifactPlan,
    before: &HostState,
    hostname: Option<&str>,
    reach: &Reach<'_>,
) -> NextStep {
    let heading = super::interview::describe(kind);
    let danger = kind.danger();
    let (commands, note) = match kind {
        StepKind::Install { digest } => (
            vec![format!(
                "cargo rivet install {}--artifact {} confirm",
                reach.device.map_or_else(|| super::connection_flags(reach.args), |d| format!("{d} ")),
                digest.chars().take(12).collect::<String>(),
            )],
            None,
        ),
        StepKind::StageMaterial { .. } => {
            (material_commands(&artifact.product, hostname, reach), Some(material_note(hostname, reach)))
        }
        StepKind::Enable { service } => match rivet_remote::deploy::change_for(&before.standing, service) {
            Some(ConfigChange::Append { text }) => (
                vec![reach.run(&format!("apply edit {DOCUMENT_PATH} --append '{text}' confirm"))],
                Some(quoting_note()),
            ),
            Some(ConfigChange::Replace { expect, replace }) => (
                vec![reach.run(&format!(
                    "apply edit {DOCUMENT_PATH} --expect '{expect}' --replace '{replace}' confirm"
                ))],
                Some(quoting_note()),
            ),
            // Not reachable: the planner emits `Enable` only for a document that
            // says nothing or says `false`, and those are exactly the two
            // standings `change_for` answers. Said rather than panicked on.
            None => (Vec::new(), Some(format!("say `[services.{service}] enabled = true` in the document yourself"))),
        },
        StepKind::SetKioskAutoRun { application } => {
            match rivet_remote::deploy::kiosk_change_for(&before.kiosk_auto_run, application) {
                Some(ConfigChange::Replace { expect, replace }) => (
                    vec![
                        reach.run(&format!(
                            "apply edit {DOCUMENT_PATH} --expect '{expect}' --replace '{replace}' confirm"
                        )),
                        reach.run("apply reload confirm"),
                        reach.run("apply service-restart rivet-launcher confirm"),
                    ],
                    Some(quoting_note()),
                ),
                // `kiosk_change_for` never constructs one — `[kiosk] auto-run`
                // is never `--append`ed (`KioskAutoRun::NoTable`'s own doc
                // comment). Handled rather than unreachable!'d, `Enable`'s
                // own `None` arm's reason.
                Some(ConfigChange::Append { .. }) => (
                    Vec::new(),
                    Some("unreachable: kiosk_change_for never constructs an append".to_string()),
                ),
                None => match &before.kiosk_auto_run {
                    rivet_remote::deploy::KioskAutoRun::Named(name) if name == application => (
                        Vec::new(),
                        Some(format!("[kiosk] auto-run already names {application:?}")),
                    ),
                    rivet_remote::deploy::KioskAutoRun::NoTable => (
                        Vec::new(),
                        Some(format!(
                            "this machine's document has no [kiosk] table — check it truly \
                             carries none, then add one yourself: `apply edit {DOCUMENT_PATH} \
                             --append` with a [kiosk] table naming auto-run = {application:?}, \
                             then `apply reload confirm` and `apply service-restart \
                             rivet-launcher confirm`."
                        )),
                    ),
                    _ => (
                        Vec::new(),
                        Some(format!(
                            "this machine's document names [kiosk] in a shape this tool will \
                             not edit blind. Set `auto-run = {application:?}` yourself."
                        )),
                    ),
                },
            }
        }
        StepKind::ServiceRestart { service } => {
            (vec![reach.run(&format!("apply service-restart {service} confirm"))], None)
        }
        StepKind::RestartReader { service, .. } => (
            vec![reach.run(&format!("apply service-restart {service} confirm"))],
            Some(
                "rivet-proxyd reads its routing table once, at start; this pass rewrote it \
                 (ADR-046 §5). The proxy is part of the image (ADR-060) and is restarted like \
                 any other service. `.rulings/` R-224: the operator approves this every time — \
                 it is never done automatically."
                    .to_string(),
            ),
        ),
        StepKind::Realize => (
            vec![reach.run("apply reload confirm")],
            Some("the supervisor re-reads the document and starts what it now enables.".to_string()),
        ),
        // Never planned for either front end: `read_host_state_for` seeds the
        // declared settings as present (Phase 1). Handled rather than
        // unreachable!'d, for `execute_step`'s own reason.
        StepKind::DeliverSetting { name } => {
            (Vec::new(), Some(format!("setting {name}: delivery is deferred and nothing can carry it out yet")))
        }
    };
    let danger_reason = danger_reason_for(kind);
    NextStep { heading, danger, commands, note, danger_reason }
}

/// Why a two-line edit is printed as a two-line quoted argument.
fn quoting_note() -> String {
    "the text between the quotes is several lines, exactly as shown — a single-quoted \
     argument keeps them in both sh and PowerShell. On Git Bash set MSYS_NO_PATHCONV=1 for \
     it, or the /state path arrives rewritten."
        .to_string()
}

/// **One unmet `connects` dependency's steps**, as the interview's
/// `dependency_step_group` decided them for this machine — a system service
/// switched off (an enable), or an artifact this repository holds that is not
/// installed and enabled here.
#[derive(Debug, Clone)]
pub(crate) struct Dependency {
    pub artifact: ArtifactPlan,
    pub steps: Vec<StepKind>,
    /// What the document says about the dependency, which is what its enable
    /// edits.
    pub standing: Standing,
    /// The interview's own line saying why it is being offered.
    pub reason: Option<String>,
}

/// **Every step that remains, as commands**, in the interview's order: each
/// unmet dependency's steps first, then the artifact's own, then the one
/// converge that starts whatever any of them enabled, then the fleet sync on a
/// fleet server.
///
/// `document` is the one `read_host_state_for` read before the send: it names
/// the machine, which is the word the material's certificate is minted for,
/// and says whether this machine is a fleet server.
#[must_use]
pub(crate) fn next_steps(
    artifact: &ArtifactPlan,
    before: &HostState,
    document: &str,
    dependencies: &[Dependency],
    enabled_by_install: bool,
    installed: bool,
    reach: &Reach<'_>,
) -> Vec<NextStep> {
    let hostname = super::hostname_in(document);
    let mut steps: Vec<NextStep> = Vec::new();
    for dependency in dependencies {
        let state = HostState { standing: dependency.standing.clone(), ..before.clone() };
        for kind in &dependency.steps {
            let mut step = step_for(kind, &dependency.artifact, &state, hostname.as_deref(), reach);
            step.note = match (step.note, &dependency.reason) {
                (Some(note), Some(reason)) => Some(format!("{reason} {note}")),
                (note, reason) => note.or_else(|| reason.clone()),
            };
            steps.push(step);
        }
    }
    let own = remaining_after_install(artifact, before, enabled_by_install, installed);
    let converges = own.contains(&StepKind::Realize);
    steps.extend(own.iter().map(|kind| step_for(kind, artifact, before, hostname.as_deref(), reach)));
    let a_dependency_enables = dependencies
        .iter()
        .any(|d| d.steps.iter().any(|kind| matches!(kind, StepKind::Enable { .. })));
    if a_dependency_enables && !converges {
        steps.push(step_for(&StepKind::Realize, artifact, before, hostname.as_deref(), reach));
    }
    if is_fleet_server(document) {
        steps.push(NextStep {
            heading: format!("sync rivet-fleetd's repository store — {} is a fleet server", reach.args.host),
            danger: Danger::Safe,
            commands: vec![reach.sync()],
            note: Some(
                "rivet-fleetd installs members from its own repository store, and an install from \
                 this workstation does not put the artifact there. The sync sends only what the \
                 store does not already hold."
                    .to_string(),
            ),
            danger_reason: None,
        });
    }
    steps
}

/// Whether this machine's document enables `rivet-fleetd` — a fleet server.
#[must_use]
pub(crate) fn is_fleet_server(document: &str) -> bool {
    rivet_remote::deploy::standing(document, FLEETD) == Standing::Enabled
}

/// **The steps, as the report prints them.** `waited` false is an install this
/// run did not see finish, which every step below depends on.
#[must_use]
pub(crate) fn render(product: &str, host: &str, steps: &[NextStep], waited: bool) -> String {
    let mut out = String::new();
    if steps.is_empty() {
        out.push_str(&format!(
            "\nnext     NOTHING REMAINS for {product} on {host} — the planner `cargo rivet deploy` \
             uses has no step left for it.\n"
        ));
        return out;
    }
    out.push_str(&format!(
        "\nnext     {n} step{s} remain{v} before {product} is in service on {host}. They come from \
         the planner `cargo rivet deploy` uses. Run them in this order, and nothing else:\n",
        n = steps.len(),
        s = if steps.len() == 1 { "" } else { "s" },
        v = if steps.len() == 1 { "s" } else { "" },
    ));
    if !waited {
        out.push_str(
            "         THIS RUN DID NOT WAIT FOR THE INSTALL (--no-wait): `get install` must report \
             it finished, with `succeeded: true`, before step 1.\n",
        );
    }
    for (at, step) in steps.iter().enumerate() {
        let danger = match (&step.danger, &step.danger_reason) {
            (Danger::Dangerous, Some(reason)) => format!("  [{reason}]"),
            (Danger::Dangerous, None) => "  [dangerous]".to_string(),
            (Danger::Safe, _) => String::new(),
        };
        out.push_str(&format!("  {}. {}{danger}\n", at + 1, step.heading));
        for command in &step.commands {
            out.push_str(&format!("       {command}\n"));
        }
        if let Some(note) = &step.note {
            out.push_str(&format!("     {note}\n"));
        }
    }
    out
}

/// **Each missing file, with the kind that asked for it** — the planner's
/// `StageMaterial` step carries the kinds and the missing files separately,
/// and a refusal names each file beside the kind a person would look up.
#[must_use]
pub(crate) fn missing_with_kinds(kinds: &[String], missing: &[String]) -> Vec<(String, String)> {
    missing
        .iter()
        .map(|file| {
            let kind = kinds
                .iter()
                .find(|kind| Material::parse(kind).is_some_and(|m| m.files().contains(&file.as_str())))
                .cloned()
                .unwrap_or_else(|| kinds.join(", "));
            (file.clone(), kind)
        })
        .collect()
}

#[cfg(test)]
#[path = "fleet_next_steps_tests.rs"]
mod fleet_next_steps_tests;
