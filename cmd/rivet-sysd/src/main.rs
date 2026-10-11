//! Rivet's service supervisor — PID 1's single child.
//!
//! `ADR-017` §3.1 ruled PID 1 minimal: it mounts, reaps, confirms the boot, and
//! spawns exactly this. Everything that looks like service management lives
//! here, for one reason worth restating because it shapes every decision in
//! this crate:
//!
//! > **This process is allowed to fail.** PID 1 is not. A supervisor bug
//! > restarts a process; the same bug in PID 1 panics the kernel.
//!
//! That permission is not an invitation to be careless — it is what lets the
//! complicated part of the system be written normally, with `Result`s that
//! propagate and a `main` that may return.
//!
//! ## What this process must do, and where each of those lives
//!
//! | Obligation | Where |
//! | :-- | :-- |
//! | Never exit merely because it has nothing to do (`ADR-017` §3.1) | [`run`] — the loop ends only through [`supervise::Phase::Finished`], which only a `SIGTERM` reaches |
//! | Converge rather than sequence (`ADR-017` §3.2) | [`supervise::decide`] and [`supervise::Supervisor::converge`] |
//! | Restart with backoff, and give up (`ADR-017` §3.5) | [`service::Limits`], [`supervise::Supervisor`] |
//! | Readiness distinct from started (`ADR-017` §3.3, §3.4) | [`service::Readiness`], the `NOTIFY_SOCKET` half of [`kernel::LibcKernel`] |
//! | Observe rather than remember (`ADR-017` §3.1.2, §3.1.3) | [`kernel::Kernel::observe`], `Adopted` |
//! | Handle `SIGTERM`: stop children and go | [`supervise::Supervisor::begin_shutdown`] |
//! | Reap its own children | [`kernel::Kernel::reap_ready`], called on every wakeup |
//! | Be the journal (`ADR-022` §7) | [`journal`] |
//! | Render the whole of `/etc` before anything that reads it starts (`ADR-007` §3.5, `ADR-017` §3.1.8) | [`etc`] |
//! | Restore the wall clock before anything is stamped, persist it hourly, **and persist it on a clean shutdown** (`ADR-021` §3, §3.2, §3.3) | [`clock`] |
//!
//! ## The shape of this crate
//!
//! | Module | Holds |
//! | :-- | :-- |
//! | [`kernel`] | the syscall seam: the `Kernel` port, and `LibcKernel`, which holds every `unsafe` block in this crate |
//! | [`journal`] | `ADR-022`'s record, its JSONL form, and the in-memory buffer |
//! | [`clock`] | `ADR-021` §3's stored wall clock: the floor at boot, and the hourly write |
//! | [`etc`] | the runtime `/etc` projection: a tmpfs, `rivet-core`'s six files, and a read-only remount |
//! | [`service`] | what a service is *to the supervisor*, and the numbers `ADR-017` §3.5 leaves unnamed |
//! | [`definitions`] | **the system services Rivet compiles into this binary** (`ADR-017` §4.1) |
//! | [`document`] | the stored desired state, and its union with the definitions |
//! | [`supervise`] | convergence: a pure `decide`, and the engine that performs what it decided |
//! | [`material`] | whether an application's declared per-machine material (`ADR-055` §3) is on this machine yet |
//! | `main` | the composition root: parse, build, run. No supervision logic. |
//!
//! The split follows `cmd/rivet-initd`'s, and for a related reason. PID 1 put
//! its syscalls behind a port because its branches need root, a kernel and an
//! emulator; the supervisor puts its syscalls behind a port because its
//! branches need *time* — a backoff that expires, a grace period that runs out,
//! a service that never declares readiness. With the clock behind the seam,
//! every one of those is tested in microseconds and none of them needs a real
//! process.
//!
//! ## What it is honest about not being
//!
//! **A definition is not a service that runs** (`AGENTS.md` §6). `ADR-017` §4.1
//! ruled what a definition contains and where it lives, and [`definitions`]
//! holds them; whether the image carries the binary each one names is a
//! separate fact, and a definition whose program is not on the card is refused
//! once, loudly, rather than looped over.
//!
//! The list is [`definitions::system_services`] — renamed from
//! `stage0_services`, because a name that asserts a roadmap position goes stale
//! on the day the roadmap moves past it, and that one did. The `--dev-service`
//! flag survives: it is a harness, not a configuration surface, and [`Options`]
//! says so at length.
//!
//! **Socket activation is not built.** `ADR-017` §3.2 pre-creates every
//! listening socket and §3.4 passes them by systemd's `LISTEN_FDS` convention.
//! Neither exists here, so `ADR-007` §7.2's `listen` cannot be honoured and a
//! document that carries one is told so rather than having it dropped — see
//! [`document::apply_desired_state`].

mod applications;
mod carried;
mod clock;
mod definitions;
mod display_hours;
mod deploy_request;
mod devices;
mod document;
mod durable;
mod endpoints;
mod etc;
mod installed;
mod issuance;
mod journal;
mod kernel;
mod kiosk_selection;
mod kmsg;
mod leases;
mod libraries;
mod reclaim;
// **`ADR-042`'s ruleset, and the only module that names a Landlock constant.**
// `rivet_core::confinement` decides what an application may reach; this turns
// that into three syscalls.
mod landlock;
mod material;
mod mdns;
mod naming;
mod runtime;
mod slot;
mod reload;
mod repo_delete_request;
mod repos;
mod retention;
mod splash;
mod service;
mod supervise;
mod uninstall_request;
mod uninstall_library_request;
mod verify;
#[cfg(test)]
mod testkernel;

use std::process::ExitCode;

use journal::{Journal, Severity, Value};
use kernel::{ConsoleForm, Errno, Event, Hierarchy, Kernel, LibcKernel, Program};
// `ADR-017` §6's control surface, whose grammar is the domain's.
use rivet_core::operation::service_restart::Request;
use service::{DefError, Limits, Readiness, RestartPolicy, ServiceDef};
use supervise::Supervisor;

/// Why the supervisor could not carry on.
///
/// Both variants end the process, and both are survivable *for the system*:
/// PID 1 restarts this binary with backoff (`ADR-017` §3.1.5 item 3), and
/// `ADR-017` §3.1.1 means the services it started keep running while it is
/// away.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Definition(DefError),
    Kernel(Errno),
    Arguments(String),
}

impl core::fmt::Display for Failure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Definition(e) => write!(f, "service definition: {e}"),
            Self::Kernel(e) => write!(f, "kernel: {e}"),
            Self::Arguments(m) => write!(f, "arguments: {m}"),
        }
    }
}

/// **A development harness, not a configuration surface.**
///
/// `ADR-011` names the single-authority rule and `ADR-017` §4 applies it to
/// services: the desired-state document is the only place a service can be
/// described, and the operator never writes a unit file. Nothing here is a
/// second authority — these are process arguments, they persist nowhere, they
/// survive no reboot, and no part of the system reads them back. They exist for
/// exactly one reason: **without them this binary cannot be observed doing
/// anything on real hardware**, and `AGENTS.md` §6 does not accept a green test
/// suite as evidence that a thing functions.
///
/// They come out the day `ADR-007` grows services.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub console: ConsoleForm,
    pub services: Vec<ServiceDef>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            console: ConsoleForm::Jsonl,
            services: definitions::system_services(),
        }
    }
}

/// Parse the process arguments. Pure, so every refusal is testable.
///
/// **The compiled-in definitions are the starting point**, and `--dev-service`
/// adds to them rather than replacing them. That is the honest default: PID 1
/// spawns this binary with an empty argv, so an invocation with no arguments is
/// what a real boot looks like and it must produce the real service set.
pub fn parse_args(args: &[String]) -> Result<Options, Failure> {
    let mut console = ConsoleForm::Jsonl;
    let mut services: Vec<ServiceDef> = definitions::system_services();
    let mut policies: Vec<(String, RestartPolicy)> = Vec::new();
    let mut notifiers: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let value = |name: &str, i: &mut usize| -> Result<String, Failure> {
            *i += 1;
            args.get(*i)
                .cloned()
                .ok_or_else(|| Failure::Arguments(format!("{name} needs a value")))
        };
        match arg {
            "--text" => console = ConsoleForm::Text,
            "--jsonl" => console = ConsoleForm::Jsonl,
            "--dev-service" => {
                let spec = value("--dev-service", &mut i)?;
                services.push(parse_dev_service(&spec)?);
            }
            "--dev-restart" => {
                let spec = value("--dev-restart", &mut i)?;
                let Some((name, policy)) = spec.split_once('=') else {
                    return Err(Failure::Arguments(format!(
                        "--dev-restart wants NAME=always|on-failure|never, got {spec:?}"
                    )));
                };
                let Some(policy) = RestartPolicy::parse(policy) else {
                    return Err(Failure::Arguments(format!(
                        "{policy:?} is not a restart policy; ADR-017 §3.5 has three: \
                         always, on-failure, never"
                    )));
                };
                policies.push((name.to_string(), policy));
            }
            "--dev-notify" => notifiers.push(value("--dev-notify", &mut i)?),
            other => {
                return Err(Failure::Arguments(format!("unknown argument {other:?}")));
            }
        }
        i += 1;
    }

    // Applied after collection so the flags may appear in any order, and
    // refused rather than ignored when they name nothing — a `--dev-restart`
    // that silently did nothing is the kind of quiet no-op that costs an hour.
    for (name, policy) in policies {
        let Some(target) = services.iter_mut().find(|s| s.name == name) else {
            return Err(Failure::Arguments(format!(
                "--dev-restart names {name:?}, which is not a --dev-service"
            )));
        };
        target.restart = policy;
    }
    for name in notifiers {
        let Some(target) = services.iter_mut().find(|s| s.name == name) else {
            return Err(Failure::Arguments(format!(
                "--dev-notify names {name:?}, which is not a --dev-service"
            )));
        };
        target.readiness = Readiness::Notify;
    }

    Ok(Options { console, services })
}

/// `NAME=/absolute/program arg arg` — split on spaces, which is all a harness
/// needs and deliberately less than a shell.
fn parse_dev_service(spec: &str) -> Result<ServiceDef, Failure> {
    let Some((name, command)) = spec.split_once('=') else {
        return Err(Failure::Arguments(format!(
            "--dev-service wants NAME=PROGRAM [ARGS], got {spec:?}"
        )));
    };
    let mut words = command.split_whitespace();
    let Some(path) = words.next() else {
        return Err(Failure::Arguments(format!(
            "--dev-service {name:?} has no program"
        )));
    };
    Ok(ServiceDef {
        name: name.to_string(),
        program: Program {
            path: path.to_string(),
            args: words.map(str::to_string).collect(),
            env: Vec::new(),
            // **Root, and deliberately not reachable from a flag.** A
            // `--dev-service` runs whatever the harness names, so it is uid 0
            // like every other definition that does not come from an installed
            // application. Letting an argument choose an identity would be a
            // command line deciding what `ADR-015` §5.1 grants — and the whole
            // value of `crate::applications`' identity is that it is derived
            // rather than asked for.
            identity: None,
            confinement: None,
            groups: Vec::new(),
            capabilities: Vec::new(),
        },
        enabled: true,
        restart: RestartPolicy::Always,
        readiness: Readiness::OnStart,
        // Empty on purpose: a harness entry is not something the machine loses
        // anything by. See `ServiceDef::consequence`.
        consequence: String::new(),
        // The same argument covers every field the 2026-08-24 split added, and
        // 2026-09-09's `cpu` beside it. A harness entry has no manifest, so it
        // has nothing to have requested and no sockets to have declared;
        // `IMAGE_DEFINED` is what says so, and it also means an operator's
        // `memory` and `cpu` caps still apply to a `--dev-service` — which is
        // the one of the set that should.
        ..service::IMAGE_DEFINED
    })
}

/// **Add the network's derived services to `services`**, one per DHCP and
/// per wireless interface the document names (`definitions::network_services`).
///
/// A name something else already holds is refused by name and the rest are
/// added: two definitions under one name would stop the supervisor being
/// built at all, which would take every service down for the sake of one
/// interface. Installed applications cannot take these names
/// (`rivet_core::package::verify::not_an_image_service`), so this guards a
/// card or a harness that put one there some other way.
fn derive_network_services(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    services: &mut Vec<ServiceDef>,
    document: &rivet_config::Document,
) {
    for def in definitions::network_services(document) {
        if services.iter().any(|existing| existing.name == def.name) {
            let corr = journal.next_correlation();
            journal.emit(
                k,
                Severity::Error,
                supervise::SELF_NAME,
                &corr,
                "a network mechanism the document asks for was NOT defined, because a service of \
                 the same name already exists. That interface has no client from this \
                 supervisor until the other service is removed",
                vec![("svc_name".to_string(), Value::Str(def.name))],
            );
            continue;
        }
        services.push(def);
    }
}

/// The supervisor, over any [`Kernel`].
///
/// **It returns only two ways**: a `SIGTERM` that completed, or a failure. It
/// does not return because there is nothing to supervise — PID 1 would restart
/// it immediately and the pair would become an infinite loop with a backoff
/// bolted on.
///
/// # The start-up order, which is a contract
///
/// Stated here because it is the only place it is visible, and because two of
/// the five steps are ordered for reasons a reader would otherwise have to
/// rediscover from an ADR:
///
/// | | Step | Why it is here and not elsewhere |
/// | :-- | :-- | :-- |
/// | 1 | the journal exists | everything below has to be able to report its own failure (`ADR-017` §8 item 2) |
/// | 2 | [`clock::Clock::restore`] | **between those two**: after the journal, so a failed restore can be reported (`ADR-017` §8 item 2), and **before the first record is stamped**, so no record on a machine with a stored clock is ever stamped 1970 (`ADR-021` §3, the 1970-clock finding, 2026-08-23) |
/// | 2½ | [`journal::Journal::open_on_state`] | **after the clock and before the first record that matters**: `ADR-022` §7 buffers until `/state` is available and flushes then, so this is the moment the buffer is drained. After the clock because a flushed record carries the timestamp it was *stamped* with, and a boot's first records are worth more with a restored floor than with 1970 |
/// | 3 | [`document::load`] | `enabled` is the operator's, and it has to be known before a `Supervisor` is built from the definitions |
/// | 4 | `arm_signals` | before anything is spawned, so no child exit falls between a spawn and the first wait |
/// | 4½ | `open_kmsg` | `ADR-022` §4.1. **After 2½ and before 7**: opening `/dev/kmsg` starts at the oldest surviving record, so this picks up everything the kernel said about bringing the hardware up — and it has to be after the journal reaches the card, or a boot's kernel messages are on the console only |
/// | 4¾ | `open_journal_ingest` | `ADR-022` §11. Beside 4½ and for its reason: after the journal reaches the card, so an ingested record is written rather than only shown, and before 7, so nothing this boot starts can beat the socket into existence |
/// | 4⅞ | `open_query` | `ADR-032` §3.2. Beside 4¾ and for its reason: before 7, so nothing this boot starts can find `rivet get status` unanswerable |
/// | 5 | [`Kernel::cgroup_hierarchy`] | once, for the machine, so the per-service records do not have to carry it (`ADR-017` §3.1.3.1) |
/// | 6 | [`etc::project`] | **before the first `converge`**, so nothing that reads `/etc` has started (`ADR-017` §3.1.8) |
/// | 7 | the first `converge` | without it a supervisor with services would block on a descriptor with nothing pending |
///
/// **Step 6 is the only ordering Rivet has, and it is not one between
/// services.** `ADR-017` §3.2 rules out a dependency graph and §3.2.1 records
/// that Stage 1's two apparent orderings needed no mechanism: this one is a
/// phase of the supervisor's own start-up, and *networking before the control
/// plane* is a state of the world that `rivet-mcpd` does not wait on — it binds
/// its address, and a bind that fails is retried like anything else.
///
/// **Step 2 is a second such phase, and it is ordered against a different
/// thing.** Not against a service — against the journal, in both directions at
/// once. It is the tightest constraint in the table: one line later and the
/// supervisor's own start-up record is stamped 1970 for ever on every board that
/// has one, and one line earlier there is nowhere to report a `/state` that did
/// not mount.
pub fn run(k: &mut dyn Kernel, options: Options) -> Result<(), Failure> {
    // `rivet_core::journal::boot_identifier` is the one rule, since
    // `cmd/rivet-deploy` reading the same file differently cost a defect
    // (`PLAN.md` §10.8).
    let boot = rivet_core::journal::boot_identifier(
        k.read_file(kernel::BOOT_ID_PATH).map_err(|_| ()),
    );
    // Separates the correlation identifiers of one supervisor run from the
    // next within the same boot, and ADR-017 §3.1 expects there to be a next.
    let salt = u32::try_from(k.now_ms() & 0xffff_ffff).unwrap_or(0);

    let mut journal = Journal::new(boot.clone(), salt, supervise::ACTOR.to_string());

    // **The wall clock, and it happens HERE.** `ADR-021` §3's floor, restored
    // after the journal exists so a failure can be reported (`ADR-017` §8 item
    // 2) and **before the first record is stamped** — every record carries a
    // wall clock (`ADR-022` §3.2), and one line further down would leave the
    // supervisor's own start-up record reading 1970 on every board that has a
    // stored clock. That is what that costs: a journal whose
    // timestamps are all 1970 cannot be correlated across boots or against
    // another machine.
    //
    // It restores a floor and nothing more. `ADR-021` §4's SNTP client is not
    // built, so no Rivet machine has ever had a *correct* clock — see
    // `crate::clock`, which says so on the console once per boot rather than
    // letting a plausible-looking timestamp imply otherwise.
    let mut clock = clock::Clock::new();
    clock.restore(k, &mut journal);

    // **The journal reaches the card, and it happens HERE.** `ADR-022` §7:
    // records produced before `/state` is available are buffered in memory and
    // flushed once it is. This is that moment — PID 1 mounted `/state` before
    // it spawned this process (`ADR-017` §3.1.7), and the clock has already
    // been restored so the records about to be flushed carry a wall clock
    // rather than 1970.
    //
    // **Everything before this line is on the console and in the ring, and
    // both are needed.** The ring is what survives to be written; the console
    // is what a maintainer with an HDMI capture sees when `/state` is the thing
    // that failed. Neither replaces the other.
    let disk = journal.open_on_state(k);
    let corr = journal.next_correlation();
    match &disk {
        journal::DiskOutcome::Open {
            flushed,
            existing_bytes,
        } => journal.emit(
            k,
            Severity::Info,
            supervise::SELF_NAME,
            &corr,
            "the journal is on the card, so this boot can be read back after it ends \
             (ADR-022 §7). Everything above this line was buffered and has just been \
             written; everything below goes to the card as it is emitted",
            vec![
                (
                    "path".to_string(),
                    Value::Str(rivet_core::state::ACTIVE_LOG.to_string()),
                ),
                (
                    "flushed".to_string(),
                    Value::Int(i64::try_from(*flushed).unwrap_or(i64::MAX)),
                ),
                (
                    "kept_from_before".to_string(),
                    Value::Int(i64::try_from(*existing_bytes).unwrap_or(i64::MAX)),
                ),
            ],
        ),
        journal::DiskOutcome::Refused { detail } => journal.emit(
            k,
            Severity::Warn,
            supervise::SELF_NAME,
            &corr,
            "the journal has NO file on the card, so this boot's records are on this \
             console and nowhere else — a power cycle loses every one of them, and \
             `rivet log` will show this boot nothing. Normal under QEMU, which has no \
             /dev/mmcblk0 for PID 1 to mount /state from (ADR-026 §4); on a board it \
             means /state did not mount and PID 1 said so above",
            vec![
                (
                    "path".to_string(),
                    Value::Str(rivet_core::state::LOG_DIR.to_string()),
                ),
                ("detail".to_string(), Value::Str(detail.clone())),
            ],
        ),
    }

    let mut services = options.services;
    let service_count = services.len();

    // The desired state, read BEFORE the `Supervisor` is built, because
    // `enabled` is what it changes and `ADR-017` §6 makes that key the entire
    // control surface. `ADR-017` §4.1: "the supervisor sees definitions ∪
    // desired state and converges" — this is the union, and the two records it
    // emits go through the journal directly because there is not yet a
    // `Supervisor` to speak through.
    //
    // It is also the first thing on the machine that reads what PID 1 stored
    // (`ADR-017` §3.1.9). `/etc` below is projected from it rather than from
    // `Document::default()`.
    // **Read before the record rather than inside it**, because `emit` borrows
    // the kernel for the whole call. `ADR-022` §12 item 1 wants the slot in the
    // boot record, and the answer comes from the kernel — `crate::slot` prefers
    // a partition's own sysfs number over the mount table's minor, and falls
    // back to the mount table alone; see the `slot` field below for why it is
    // not the firmware either way.
    let running_slot = slot::running_slot(k);

    let corr = journal.next_correlation();
    journal.emit(
        k,
        Severity::Info,
        supervise::SELF_NAME,
        &corr,
        "supervisor starting",
        vec![
            // No `boot` field here. Every record already carries the boot id as
            // a reserved column (`Record::boot`), and adding it again emitted
            // the key twice on one line — ambiguous to a strict JSON parser,
            // and `ADR-022` §3 makes this form authoritative. See
            // `Record::RESERVED_FIELD_NAMES`.
            (
                "boot_id_source".to_string(),
                Value::Str(kernel::BOOT_ID_PATH.to_string()),
            ),
            (
                "services".to_string(),
                Value::Int(i64::try_from(service_count).unwrap_or(i64::MAX)),
            ),
            // **`ADR-022` §12 item 1: the version running.** It was absent, and
            // its absence is what made §26.4 hard: after an A/B update the
            // question an operator asks is *did that take*, and nothing in the
            // journal answered it. `CARGO_PKG_VERSION` is `ADR-027` §10's
            // single source, so this is the same number the image is named for.
            (
                "version".to_string(),
                Value::Str(env!("CARGO_PKG_VERSION").to_string()),
            ),
            // **And the slot, item 1's other half.**
            //
            // From `/proc/self/mountinfo` and the root partition's own sysfs
            // number rather than the firmware: reading the firmware means
            // mounting the FAT boot partition, and
            // `rivet_core::operation::stop` already refused to put a FAT mount
            // on the shutdown path for a reason that applies with more force to
            // the one path a board cannot go around. `PLAN.md` §26.4.1 records
            // why the other tempting answer — hashing the running binary —
            // fails silently in exactly the direction that matters.
            //
            // **Absent rather than guessed** where there is no `/system` line,
            // which is every development host and the emulator (`ADR-026` §4)
            // and is not a fault. An operator reading a journal with no `slot`
            // is reading one from a machine that is not a card.
            (
                "slot".to_string(),
                match running_slot {
                    Some(slot) => Value::Str(slot.as_str().to_string()),
                    None => Value::Str("unknown: no /system in the mount table".to_string()),
                },
            ),
        ],
    );

    // **Applications, before the document and after the image's own.**
    // `ADR-017` §4's two origins, in the only order that works: the union the
    // document performs is over every definition, and an operator's
    // `[services.photo-index]` has nothing to apply to until the application
    // that defines `photo-index` has been discovered. Ruled by the maintainer,
    // 2026-08-24, and `crate::applications` is the whole of it.
    // **The built-in volume, and it is the only one.** `ADR-031` §8 item 1 made
    // discovery a function of a volume root; nothing attaches a second one,
    // because §11 items 1, 2 and 8 are unruled. A machine with declared volumes
    // will call this once per attached volume, after attaching them here.
    //
    // **The OOM killer never chooses the supervisor** — `ADR-017` §3. Before
    // anything is spawned, because every service is spawned from here and each
    // is given its own value at that moment (`Kernel::spawn`). A refusal is
    // survivable and said: the machine runs, and the supervisor is as
    // exposed as any other process.
    let corr = journal.next_correlation();
    match k.set_own_oom_score_adj(definitions::SUPERVISOR_OOM_SCORE_ADJ) {
        Ok(()) => journal.emit(
            k,
            Severity::Info,
            supervise::SELF_NAME,
            &corr,
            "the supervisor is exempt from the kernel's OOM killer, and the control plane is \
             chosen after everything but it",
            vec![
                (
                    "oom_score_adj".to_string(),
                    Value::Int(i64::from(definitions::SUPERVISOR_OOM_SCORE_ADJ)),
                ),
                (
                    "control_plane".to_string(),
                    Value::Int(i64::from(definitions::MCPD_OOM_SCORE_ADJ)),
                ),
            ],
        ),
        Err(e) => journal.emit(
            k,
            Severity::Warn,
            supervise::SELF_NAME,
            &corr,
            "the supervisor could NOT exempt itself from the kernel's OOM killer, so a machine \
             that runs out of memory may kill the process that restarts everything else",
            vec![("detail".to_string(), Value::Str(e.to_string()))],
        ),
    }

    // **What a power cut left set aside is handed to helpers first**
    // (`ADR-017` §3), before anything this start removes joins it.
    //
    // **An application the image now provides is retired next** (`ADR-060`
    // §3), so discovery never sees it and no second definition claims the
    // image service's name.
    reclaim::sweep_leftovers(k, &mut journal, rivet_core::service::BUILT_IN_VOLUME);
    applications::retire_carried_image_services(
        k,
        &mut journal,
        rivet_core::service::BUILT_IN_VOLUME,
    );
    applications::discover(
        k,
        &mut journal,
        rivet_core::service::BUILT_IN_VOLUME,
        &mut services,
    );

    // **The accounts `/etc` will have to name**, taken here because `services`
    // is about to move into the `Supervisor` and because `/etc` is projected
    // below rather than now. `ADR-008` §7.3: a service account arrives with the
    // service that needs it, so this list is a fact about what discovery just
    // found and not a constant anywhere.
    let app_names = etc::application_names(&services);

    let desired = document::load(k, &mut journal);

    // **The network's mechanisms, derived from the document just read** —
    // `definitions::network_services`. Before the document's `[services]`
    // table is applied, so a DHCP client is a definition like any other by
    // the time anything is converged.
    derive_network_services(k, &mut journal, &mut services, &desired.document());

    document::apply_desired_state(k, &mut journal, &mut services, &desired);

    // **The image services that run under their own uid get the home an
    // application has** (`ADR-060`), once the document has said which of them
    // run: a service it leaves off gets no directories made for it.
    applications::provide_image_homes(k, &mut journal, &services);

    // **A fleet-root machine's certificates, judged and issued before anything
    // starts** (`ADR-063` §4). After the homes above, so every directory a leaf
    // goes into exists; before the first convergence, so every program reads
    // its new leaf at its first start and nothing is restarted for it.
    issuance::converge(k, &mut journal, &services, issuance::Occasion::Boot, &[]);

    // **Who may control the kiosk, decided before the definitions move.**
    // The narrow kiosk socket, ruled 2026-09-01 (`ADR-036` §5.4). Every service
    // that declares `connects = ["rivet-sysd:kiosk"]` is a claimant; the one
    // the document names as the launcher is given the socket below, once the
    // journal exists to report the outcome, and the rest are refused by name.
    let kiosk_claimants: Vec<(String, Option<rivet_core::service::AppIdentity>)> = {
        let mut found: Vec<_> = services
            .iter()
            .filter(|d| {
                d.connects.iter().any(|e| {
                    e.owner() == rivet_core::kiosk::PROVIDER
                        && e.name() == rivet_core::kiosk::ENDPOINT
                })
            })
            .map(|d| (d.name.clone(), d.program.identity))
            .collect();
        // Sorted, so a machine with a mistake in it reports the same mistake
        // every boot rather than one that depends on directory order.
        found.sort_by(|a, b| a.0.cmp(&b.0));
        found
    };

    // **Which services the document names for the launcher and the
    // compositor**, read once for this boot: the kiosk socket below is given
    // by it, and the supervisor gives the screen by it.
    let display_authority = document::display_authority(&desired);

    let mut sup =
        Supervisor::new(services, Limits::default(), journal).map_err(Failure::Definition)?;
    sup.set_display_authority(display_authority.clone());

    // **What this machine is for, handed to the supervisor** — `PLAN.md`
    // §109.3. It is set unconditionally here, `Unreadable` included: at boot
    // nothing stands, so `unknown` with the store's own sentence is the honest
    // answer. `crate::reload` deliberately does NOT do the same — see there.
    sup.set_kiosk_mode(document::kiosk_mode(&desired));

    // **Restore which application held the screen before the reboot** —
    // `crate::kiosk_selection`. The selection is runtime state
    // (`crate::supervise`), so nothing else brings it back; without this a kiosk forgets its
    // compositor-drawing application across every reboot and comes up blank.
    // After `Supervisor::new`, so the offered set `select_display` validates
    // against is already computed; a failure is reported inside and never stops
    // the boot.
    kiosk_selection::restore(k, &mut sup);

    // Signals are armed before anything is spawned, so no child exit can fall
    // between the spawn and the first wait.
    if let Err(e) = k.arm_signals() {
        sup.note(
            k,
            Severity::Error,
            "DEGRADED could not take over SIGTERM and SIGCHLD; there is no clean shutdown path \
             and child exits are noticed by polling",
            vec![("detail".to_string(), Value::Str(e.to_string()))],
        );
    }

    // **`/dev/kmsg`, and it is opened here for a reason that is not tidiness.**
    //
    // `ADR-022` §4.1 ingests the kernel's ring buffer, and opening the
    // descriptor starts at the **oldest surviving record** — so this picks up
    // everything the kernel said about bringing the hardware up, long before
    // this process existed. That is the whole value: `ADR-004` §8's
    // firmware-loading failures appear only there.
    //
    // After the journal is on the card (step 2½), so the boot's kernel messages
    // land in a file rather than only on the console. Before the first
    // `converge`, so a service that fails to start has the kernel's account of
    // *why* on the same timeline.
    //
    // **A refusal is ordinary under the emulator and a real fault on a board.**
    // `tools/qemu/run.sh` boots an initramfs and the kernel does not auto-mount
    // devtmpfs onto one, so there is no `/dev/kmsg` there at all.
    if let Err(e) = k.open_kmsg() {
        sup.note(
            k,
            Severity::Warn,
            "the kernel's own messages are NOT in this journal. /dev/kmsg could not be opened, \
             so everything the kernel says — driver probes, firmware that did not load, a \
             filesystem that refused — reaches the console and nowhere else, and `rivet get \
             log` will not have it (ADR-022 §4.1). Normal under QEMU, whose initramfs root has \
             no devtmpfs; on a board it means /dev is not what it should be",
            vec![
                ("path".to_string(), Value::Str(kmsg::KMSG_PATH.to_string())),
                ("detail".to_string(), Value::Str(e.to_string())),
            ],
        );
    }

    // **The journal's ingest socket** — `ADR-022` §11, and it is the answer to
    // *how does a process the supervisor did not spawn get a record in here*.
    //
    // **Here, beside `open_kmsg` and for the same reason**: after the journal
    // reaches the card, so a record that arrives is written rather than being on
    // a console only, and before the first `converge`, so a service that is
    // started by this boot cannot beat the socket into existence.
    //
    // A refusal is survivable and is *said*: every sender falls back to its own
    // stderr (`ADR-022` §11.5), which for `cmd/rivet` is the operator's own
    // session — visible, and not in `rivet get log`.
    if let Err(e) = k.open_journal_ingest(rivet_core::journal::INGEST_SOCKET_PATH) {
        sup.note(
            k,
            Severity::Warn,
            "the journal will NOT accept records from processes this supervisor did not start. \
             The ingest socket could not be bound, so `rivet apply edit` and `femto` have \
             nowhere to file what they changed and will say so on their own stderr instead \
             (ADR-022 §11). Nothing else is affected: services' output is captured over pipes \
             and is unaffected by this",
            vec![
                (
                    "path".to_string(),
                    Value::Str(rivet_core::journal::INGEST_SOCKET_PATH.to_string()),
                ),
                ("detail".to_string(), Value::Str(e.to_string())),
            ],
        );
    }

    // **`ADR-032` §3.2's query socket**, beside the ingest socket and for its
    // reason: before step 7, so nothing this boot starts can beat the socket
    // into existence and find `rivet get status` unanswerable.
    //
    // **A refusal is survivable and is said**, exactly as the ingest socket's
    // is. What it costs is narrower and worth stating: everything else this
    // machine reports is on disk and survives even the supervisor, so a board
    // without this socket is one where `rivet get log` still works and only the
    // live question cannot be asked.
    if let Err(e) = k.open_query(rivet_core::operation::status::QUERY_SOCKET_PATH) {
        sup.note(
            k,
            Severity::Warn,
            "`rivet get status` will NOT answer on this machine. The query socket could not be \
             bound, so nothing can ask this supervisor what it is doing and the CLI will report \
             that it is not answering (ADR-032 §3.2, §3.3 item 5). Everything else is \
             unaffected: the journal, storage and the schema are all read without this socket, \
             and `rivet get log` is where a service that did not start said so",
            vec![
                (
                    "path".to_string(),
                    Value::Str(rivet_core::operation::status::QUERY_SOCKET_PATH.to_string()),
                ),
                ("detail".to_string(), Value::Str(e.to_string())),
            ],
        );
    }

    // **`rivet_core::facts`'s socket** — `ADR-057`, beside the two above and
    // before step 7 for their reason: an application that starts on this boot
    // must not find its facts call unanswerable and conclude from the silence
    // that this supervisor has already stopped.
    //
    // **A refusal is survivable and is said, and here it says something a
    // dashboard would otherwise get wrong.** The other two sockets going
    // missing costs an answer; this one going missing costs a *diagnosis* —
    // silence on this socket is how an application learns the supervisor has
    // wedged, so a socket that was never bound looks exactly like the failure
    // it exists to report. The record below is what tells the two apart.
    if let Err(e) = k.open_facts(rivet_core::facts::SOCKET_PATH) {
        sup.note(
            k,
            Severity::Warn,
            "an application asking this machine what it is will get NO ANSWER, and will read \
             that silence as a wedged supervisor (ADR-057 §3). The facts socket could not be \
             bound. Nothing else is affected — `rivet get status` answers the same facts over \
             the control plane — but any dashboard on this machine's own screen will report a \
             failure that has not happened, and this record is what tells the two apart",
            vec![
                (
                    "path".to_string(),
                    Value::Str(rivet_core::facts::SOCKET_PATH.to_string()),
                ),
                ("detail".to_string(), Value::Str(e.to_string())),
            ],
        );
    }

    // **The document's `[kiosk] auto-run`, applied once for this boot** —
    // `PLAN.md` §104.40. After this the selection is runtime state and the
    // document does not get another say until the next boot.
    // **`[kiosk] auto-run` is NOT read here, and that is the point.**
    //
    // Ruled 2026-09-01, correcting the language as much as the code: the
    // supervisor does not autorun the launcher, because that is the wrong
    // language — it knows whether the launcher is enabled and starts it — and
    // the autorun language belongs to the launcher, which might autostart a
    // kiosk application (.rulings/ R-344).
    //
    // Two layers, and this is the lower one. A launcher is a service: it runs
    // because the document enables it, by the same rule as `rivet-netd`, and
    // `Supervisor::offered_displays` already excludes it from the set it
    // arbitrates over -- by its `connects = ["rivet-sysd:kiosk"]` declaration
    // rather than by its name. Nothing here needs to know a launcher exists.
    //
    // Which KIOSK APPLICATION starts is the launcher's decision and it reads
    // the key itself. Until this was struck the supervisor took `auto-run` as
    // an initial display selection, which was a no-op in practice -- the one
    // name any real document put there was the launcher's, and the launcher is
    // never in the offered set -- so this removes a mechanism that never fired
    // and a sentence that taught the wrong model.

    // **`rivet_core::kiosk`'s socket** — ruled 2026-09-01 (`ADR-036` §5.4).
    //
    // **No launcher is an ordinary machine**, not a fault: the socket is still
    // bound and still `0600`, owned by root, so nothing can ask — which is
    // exactly right for a board that offers no menu.
    {
        // **One consumer, and the document names it.** The grant is a chown
        // and a chown names one uid, so the socket goes to the launcher the
        // document names (the image's own by default) and every other
        // application declaring the endpoint is refused by name. A claim the
        // document does not back takes nothing from the launcher, which is
        // what stops an installed application taking the menu away.
        let (holder, refused) =
            crate::endpoints::kiosk_holder(&kiosk_claimants, &display_authority.launcher);
        for name in &refused {
            sup.note(
                k,
                Severity::Error,
                "an application asks to control the kiosk and is not this machine's \
                 launcher, so it was NOT given the kiosk socket. The document's [kiosk] \
                 launcher names the one service that may hold it, and the image's own \
                 launcher when it names none",
                vec![
                    ("claimant".to_string(), Value::Str(name.clone())),
                    (
                        "launcher".to_string(),
                        Value::Str(display_authority.launcher.clone()),
                    ),
                ],
            );
        }
        let granted = match holder {
            Some((name, identity)) => (name, identity),
            None => (
                format!(
                    "nobody: this machine's launcher, {}, is not defined or does not declare it",
                    display_authority.launcher
                ),
                None,
            ),
        };
        if let Err(e) = k.open_kiosk(rivet_core::kiosk::SOCKET_PATH, granted.1) {
            sup.note(
                k,
                Severity::Warn,
                "the kiosk socket could not be bound, so a launcher on this machine can \
                 draw a menu and act on nothing. Everything else is unaffected: this \
                 socket carries three verbs and no operator uses it",
                vec![
                    (
                        "path".to_string(),
                        Value::Str(rivet_core::kiosk::SOCKET_PATH.to_string()),
                    ),
                    ("detail".to_string(), Value::Str(e.to_string())),
                ],
            );
        } else {
            sup.note(
                k,
                Severity::Info,
                "the kiosk socket is bound. It carries three verbs -- select, return and \
                 restart -- and is given by chown to this machine's launcher alone, so a \
                 launcher never reaches the operator's control surface (ADR-032 §3.2) to \
                 get them",
                vec![("granted_to".to_string(), Value::Str(granted.0))],
            );
        }
    }

    // **The one place the machine-wide cgroup2 fact is stated.**
    //
    // `ADR-017` §3.1.3 makes a service's cgroup the authority for what is
    // running; §3.1.3.1 rules that *whether that authority exists at all* is a
    // fact about the machine, said once here, rather than something each
    // service's records re-derive. Two things follow, and both are the point:
    //
    // * On a healthy boot the per-service path says **nothing** — a cgroup that
    //   is absent because the service has never run is not news — and this line
    //   is what keeps that silence readable rather than ambiguous. Without it a
    //   console would look identical whether cgroup2 was fine or never checked.
    // * On a blind machine this fires even with every service disabled, or with
    //   none defined at all. The old warning could only be reached through a
    //   `start`, so a machine that started nothing lost the news entirely.
    //
    // It does not replace the per-service warning: the hazard — a second copy
    // of *this* service — is per-service, and `Supervisor::report_unobservable`
    // names each one.
    match k.cgroup_hierarchy() {
        Hierarchy::Present => sup.note(
            k,
            Severity::Info,
            "the cgroup2 hierarchy is readable, so what is already running can be observed \
             (ADR-017 §3.1.3)",
            vec![(
                "hierarchy".to_string(),
                Value::Str(kernel::CGROUP_CONTROLLERS.to_string()),
            )],
        ),
        Hierarchy::Unreachable(e) => sup.note(
            k,
            Severity::Warn,
            "DEGRADED there is no readable cgroup2 hierarchy, so ADR-017 §3.1.3's authority for \
             what is running does not exist on this machine. Every service starts blind, and a \
             supervisor restart starts a second copy of anything still running",
            vec![
                (
                    "hierarchy".to_string(),
                    Value::Str(kernel::CGROUP_CONTROLLERS.to_string()),
                ),
                ("errno".to_string(), Value::Int(i64::from(e.0))),
                ("detail".to_string(), Value::Str(e.to_string())),
            ],
        ),
    }

    if service_count == 0 {
        // ~~"rivet-config declares no service schema. Supervising nothing."~~
        // That was the message until 2026-08-23 and it was unconditional,
        // because `stage0_services` returned an empty vector. It cannot be
        // reached on a real boot any more — `definitions::system_services` is
        // never empty and a test pins that — and the branch is kept for the one
        // way it still can be: an `Options` built by hand in a test.
        sup.note(
            k,
            Severity::Warn,
            "no services are defined. On a real boot this cannot happen — ADR-017 §4.1 puts the \
             system definitions in this binary and cmd/rivet-sysd compiles in two. \
             Supervising nothing, and staying up: a supervisor that exited here would be \
             restarted by PID 1 for ever",
            vec![],
        );
    }

    // `/state`'s grow — `ADR-018` §10.3, ruled the supervisor's by §10.7.3 with
    // the same placement and the same argument as the `/etc` projection below.
    //
    // **Before `/etc` rather than after, and the reason is the failure case.**
    // Both are one-shots and neither depends on the other. This one writes a
    // partition table; that one mounts a tmpfs. If a boot is going to be
    // interrupted, the earlier the GPT rewrite completes the smaller the window
    // `rivet_adapter_image::grow`'s write order is protecting.
    //
    // **It cannot withhold the boot's confirmation and does not report to PID
    // 1 at all.** §10.7.3: "a failure degrades the boot and never withholds the
    // confirmation — a rollback cannot repair a /state an update never
    // touches." `Grew` is total, so there is no error here to be tempted into
    // propagating; the outcome is a record and nothing else.
    let grew = k.grow_state();
    sup.note(
        k,
        match &grew {
            // A refusal or a failure is `Warn`: the machine works and `/state`
            // is smaller than the card. Under QEMU the failure is ENOENT on a
            // device that is not there (`ADR-026` §4) and is the ordinary
            // answer every boot — which is why it is not `Error`, and why the
            // message has to be legible as "not applicable here".
            rivet_adapter_image::Grew::Grown { .. }
            | rivet_adapter_image::Grew::AlreadyFillsTheDevice => Severity::Info,
            _ => Severity::Warn,
        },
        &grew.summary(),
        vec![("outcome".to_string(), Value::Str(format!("{grew:?}")))],
    );

    // `/etc`, and it happens HERE — after the journal exists so a failure can
    // be recorded, and before the first `converge` so that nothing which reads
    // `/etc` has started. `ADR-017` §3.1.8 and `crate::etc`'s module docs carry
    // the argument for this process owning it rather than PID 1.
    //
    // ~~The document is `Document::default()` … `ADR-014` §2's `rivet-store`
    // does not exist — checked 2026-08-23 two ways.~~ **It exists**, and the
    // claim of absence was corrected the same day it was written (`ADR-007`
    // §7.6). The document is now the *stored* one where a machine has one and
    // the schema defaults where it does not — `ADR-007` §3.4 layer 1, which is
    // still the only layer that needs no store behind it.
    //
    // ~~It costs `/etc` nothing today either way: no key of the schema is a
    // fact any of these five files carries.~~ **It costs three files.** The
    // stored document's `hostname` reaches `/etc/hosts`, its `timezone`
    // reaches `/etc/localtime`, and `[network.resolver]` reaches
    // `/etc/resolv.conf`, so a machine with a stored document renders what
    // the operator wrote and one without it renders the schema defaults.
    // `crate::etc::inputs_from` states this and a test pins it.
    //
    // **The applications reach it too, since 2026-08-24.** Their uids must have
    // names by the time anything runs as one, or `ls -l` on an application's
    // own data prints a number and `SO_PEERCRED` hands a listening service a
    // caller it cannot name (`ADR-015` §5.1, §5.3). This is before the first
    // `converge`, so it is before any of them has started.
    // **The name comes first, because `/etc/hosts` carries it.** One name,
    // three readers (`crate::naming`), and the two that exist today are decided
    // together so a machine cannot `uname` one name and resolve another.
    let naming = naming::name_this_machine(k, sup.journal_mut(), &desired.document());

    // **The supervisor is TOLD its machine's name and slot** — `ADR-057` §4.
    //
    // Both are already in hand at this point: the name was just settled above,
    // and `running_slot` came off the kernel's mount table before the boot
    // record was written. Handing them over here is what lets the facts call
    // answer them from memory — and `ADR-011` is why this is a hand-over and
    // not a second read: `crate::naming` decides the name once, and a
    // supervisor that resolved it again could answer one name while
    // `/etc/hosts` carried another.
    sup.set_machine_identity(
        naming.hostname().map(|h| h.as_str().to_string()),
        running_slot.map(|s| s.as_str().to_string()),
    );

    // **What the mDNS responder will announce from**, recorded before it
    // starts so a reload can tell when it has changed — `crate::mdns`.
    let mdns_inputs = mdns::Inputs::read(
        k,
        &desired.document(),
        naming.hostname().map(rivet_core::hostname::Hostname::as_str),
    );
    sup.follow_mdns_inputs(k, mdns_inputs);

    // **What `rivet-netd` will apply from**, recorded before it starts so the
    // reload a lease's nudge causes can tell a new lease from one it has
    // already applied — `crate::leases`.
    let leases = leases::Leases::read(k, &desired.document());
    sup.follow_leases(k, leases);

    // **The display window, published beside the /etc projection and NOT into
    // it** - `ADR-039` §3.1, moved out of /etc on 2026-09-02 on the maintainer's
    // question. `crate::runtime`'s module note has the rule that separates the
    // two: /etc is for paths a C runtime demands, and no C runtime asks for a
    // display schedule.
    //
    // Same document, same pass, so the two cannot disagree.
    // **What the television was last told** - `ADR-039` §8.2, and it lives
    // across the loop because the point is to send only on a CHANGE. A
    // `Standby` every pass would be a television nobody can switch back on by
    // hand until morning.
    let mut television = display_hours::Television::default();

    // **What OS this is**, for anything that wants to say so without assuming
    // it was built alongside the image. `generated::OS_VERSION_PATH`'s note
    // carries the launcher that assumed it and was wrong for days.
    runtime::publish_os_version(k, sup.journal_mut());

    // **Settings, handed over** - `ADR-038` §6. After the applications have
    // been discovered, so their manifests are on disk to check against, and
    // before the first convergence, so an application starts with what the
    // document says rather than with what a previous document said.
    let declared = runtime::declared_settings(k, sup.services());
    runtime::deliver_settings(k, sup.journal_mut(), &desired.document(), &declared);
    // **And the name service's projection of the document** - `ADR-047` §6,
    // from the same list, so it is written exactly where settings are.
    let machine_name = sup.machine_name().map(str::to_string);
    runtime::deliver_name_service_document(
        k,
        sup.journal_mut(),
        &desired.document(),
        machine_name.as_deref(),
        &declared,
    );
    // **And the mDNS responder's projection** - `ADR-017` §3, the inputs
    // recorded above, from the same list.
    let mdns_inputs = sup.mdns_inputs().cloned();
    runtime::deliver_mdns_projection(k, sup.journal_mut(), mdns_inputs.as_ref(), &declared);

    // **The proxy's routing table** - `ADR-046` §5. Beside the settings and for
    // the same reason: it is derived from the same installed manifests, in the
    // same pass, before the first convergence - so `rivet-proxyd` starts with
    // the routes this machine actually has rather than with the ones it had
    // when it was last restarted.
    //
    // **Nothing in it comes from the document**, which is why it takes no
    // document: every column is from a signed manifest (`ADR-046` §5).
    let applications = runtime::installed_applications(sup.services());
    let plain = runtime::publish_routes(k, sup.journal_mut(), &applications);
    // **Before the first convergence, `sup.converge(k)` below** — `ADR-046`
    // §7, `ADR-061` §7. `rivet-proxyd` has not started yet on this pass, so
    // this sets its stored `listen` before its own first start ever reads it:
    // the first listener set on this boot is already correct, with nothing to
    // journal.
    runtime::reconcile_proxy_listen(k, &mut sup, plain);
    // **And the firewall over the routed ports** - `ADR-023` §9. From the same
    // manifests, before the first convergence, so no application starts on a
    // machine whose proxied ports are not yet guarded.
    runtime::reconcile_firewall(k, &mut sup);

    // **And what those applications offer to back up** — `ADR-051`. Derived
    // from the same manifests in the same pass, and taking no document for
    // `publish_routes`' reason: every column is from a signed manifest.
    runtime::publish_backups(k, sup.journal_mut(), &applications);

    let published = runtime::publish_display_hours(k, sup.journal_mut(), &desired.document());
    sup.note(
        k,
        match published {
            runtime::Published::Failed => Severity::Warn,
            _ => Severity::Info,
        },
        "display window published",
        vec![(
            "outcome".to_string(),
            rivet_core::journal::Value::Str(format!("{published:?}")),
        )],
    );

    let accounts = etc::accounts(&app_names);
    let projection = etc::project(
        k,
        sup.journal_mut(),
        &desired.document(),
        &accounts,
        &naming,
    );
    sup.note(
        k,
        match &projection {
            etc::Projection::Rendered {
                read_only: true, ..
            } => Severity::Info,
            _ => Severity::Warn,
        },
        "/etc projected",
        vec![
            ("outcome".to_string(), Value::Str(format!("{projection:?}"))),
            // Reported separately from the outcome because it is a different
            // question and the two are routinely confused: a *perfect* render
            // still permits no login, because there is no cmd/rivet for
            // /etc/shells to name. `AGENTS.md` §6 — "wired
            // up" is never reported as "works".
            (
                "permits_a_login".to_string(),
                Value::Bool(projection.permits_a_login()),
            ),
        ],
    );

    // **The repositories, after `/etc` and before anything starts.**
    // `crate::repos` creates what the document declares, moves what it renamed,
    // and touches nothing else. It is here rather than earlier because a
    // repository is served by `cmd/rivet-gitd`, which runs as an account — and
    // that account's `/etc/passwd` line is what the projection above just
    // wrote.
    //
    // **The result is noted rather than acted on.** Every individual outcome is
    // already in the journal with its own sentence (`crate::repos` emits one
    // per repository); this line is the count, so that a boot can be read at a
    // glance without the detail being only in a count.
    let repositories = repos::converge(k, sup.journal_mut(), &desired.document());
    if repositories != repos::Converged::default() {
        sup.note(
            k,
            if repositories.failures == 0 && repositories.collisions == 0 {
                Severity::Info
            } else {
                Severity::Warn
            },
            "git repositories converged",
            vec![
                ("created".to_string(), Value::Int(repositories.created as i64)),
                ("renamed".to_string(), Value::Int(repositories.renamed as i64)),
                // Named separately from `failures` because neither is a fault
                // in this machine and both mean NOTHING WAS TOUCHED - they are
                // reports about a disk, and folding them into a failure count
                // would read as work that went wrong.
                (
                    "collisions".to_string(),
                    Value::Int(repositories.collisions as i64),
                ),
                (
                    "undeclared".to_string(),
                    Value::Int(repositories.undeclared as i64),
                ),
                ("failures".to_string(), Value::Int(repositories.failures as i64)),
            ],
        );
    }

    // **The install request, and it is looked for before the FIRST convergence
    // too.** The maintainer's 2026-08-24 install-request ruling chose a file on `/state` precisely because it
    // survives a supervisor restart and a reboot, so a machine that was powered
    // off between `rivet apply install` and the install performs it here.
    // `crate::deploy_request` is where the whole of it is argued, including why
    // the request is removed *before* anything is started.
    let mut requests = deploy_request::Watch::new();
    requests.poll(k, &mut sup);
    // The uninstall channel, polled beside the install one and for the same
    // reason — see `crate::uninstall_request`. A machine can have one of each
    // waiting; they are different files and different actors.
    let mut removals = uninstall_request::Watch::new();
    removals.poll(k, &mut sup);
    // **And the library removal channel** — `crate::uninstall_library_request`,
    // the uninstall channel's twin for a library build, polled beside it for
    // its reason. A fourth file and the same actor.
    let mut library_removals = uninstall_library_request::Watch::new();
    library_removals.poll(k, &mut sup);
    // **And the destructive channel** - `crate::repo_delete_request`, ruled
    // 2026-08-26. A third file and a third actor, beside the two above and for
    // their reason; it takes the document because one of its two checks is
    // *is this repository still declared?*, which cannot be answered from the
    // request.
    let mut repo_deletes = repo_delete_request::Watch::new();
    repo_deletes.poll(k, &mut sup, &desired.document());
    // **And the other end of the install channel** — `PLAN.md` §19.2. The two
    // above watch for a request arriving; this one watches the installer
    // finishing, and re-runs discovery so that an application installed on a
    // running machine becomes a service without a reboot. It holds `desired`
    // because a newly discovered application needs the operator's `enabled`
    // from the same document every service beside it was given.
    let mut installs = installed::Watch::new();
    installs.poll(k, &mut sup);
    // **And the applications the CARD carried** — `ADR-050` §4. Last of the
    // four, and the order is load-bearing: an operator's request goes first
    // because it is somebody waiting, and `installed` goes before this one
    // because adopting the next payload replaces the finished installer
    // instance whose edge that watch is looking for. `crate::carried` argues
    // both.
    //
    // Before the FIRST convergence as well as inside the loop, for
    // `deploy_request`'s reason one step further on: this is a fresh card's
    // first boot, and the whole point of `ADR-050` is that nothing between the
    // card being written and the application running is an operator's step.
    let mut carried = carried::Watch::new();
    carried.poll(k, &mut sup);

    // The first convergence happens before the first wait. Without it a
    // supervisor with services would block on a descriptor with nothing pending
    // and start nothing at all.
    sup.converge(k);

    // And the clock's first opportunity to write, likewise before the first
    // wait. It writes only when this boot learned something the card does not
    // already know — nothing stored at all, or a running clock ahead of what was
    // stored — and on an ordinary Pi boot it writes nothing until the hour. Here
    // rather than inside the loop so that a boot which *is* due does not spend
    // its first `poll(2)` with a zero timeout.
    clock.tick(k, sup.journal_mut());

    loop {
        if sup.is_finished() {
            // ~~**Nothing writes the clock here**, and that is an open question
            // rather than an omission.~~ **Ruled 2026-08-23, `ADR-021` §3.3: it
            // is written by this process, and it has already happened** — at the
            // `Event::Terminate` below, when the shutdown was *requested*,
            // rather than here when it completed. `Clock::persist_on_shutdown`
            // carries why: PID 1's grace period for stopping this process is
            // shorter than this process's own grace period for stopping a
            // service, so a write placed here is one a badly-behaved service can
            // cancel.
            return Ok(());
        }
        // The soonest of the two deadlines, so one `poll(2)` serves both. The
        // clock's is monotonic like the supervisor's — `ADR-021` §5, and here
        // it is not academic: the restore above moves the wall clock, so an
        // hourly deadline expressed in wall time would fire either immediately
        // or in fifty-five years depending on which side of that jump it was
        // computed on.
        let now = k.now_ms();
        let timeout = soonest(sup.next_timeout_ms(now), clock.next_timeout_ms(now));
        match k.wait(timeout) {
            Ok(events) => {
                for event in events {
                    match event {
                        Event::Terminate { how } => {
                            sup.begin_shutdown(k, how);
                            // `ADR-021` §3's "and on clean shutdown", ruled to
                            // this process in §3.3. **Here, at the request**,
                            // not at the exit: see `Clock::persist_on_shutdown`.
                            // It is idempotent, so a second SIGTERM — an
                            // impatient operator, or PID 1 escalating — is not a
                            // second card write.
                            clock.persist_on_shutdown(k, sup.journal_mut());
                        }
                        // `ADR-032` §3.2's *tell*, built 2026-08-26. It reads
                        // the document again and applies what a running machine
                        // may safely take — `crate::document::Occasion::Reload`
                        // is where that line is drawn and why. The next
                        // `converge` below starts and stops what changed, by
                        // the same rules every service is always subject to
                        // (`ADR-017` §6); a reload starts and stops nothing
                        // itself.
                        Event::Reload => reload::reload(k, &mut sup),
                        // `ADR-032` §3. The kernel holds the connection and
                        // this holds the state, so the answer travels this way
                        // — see `Kernel::answer_query`.
                        //
                        // The socket is the supervisor's control surface
                        // (`ADR-017` §6). A restart converges toward the
                        // document and is enforced in
                        // `Supervisor::restart_service`; a stop held on one
                        // application, and its release, are enforced in
                        // `Supervisor::hold_service`. Neither writes `enabled`.
                        //
                        // **The answer is written before the convergence at the
                        // bottom of this loop**, which is not incidental: a
                        // restart of `rivet-mcpd` is a restart of the process
                        // carrying the request, and this ordering is what gives
                        // the reply its chance to leave. It is still a race and
                        // `service_restart::caveat` says so.
                        Event::Queried { request } => {
                            let answer = match request {
                                Request::GetStatus => sup.status().render(),
                                Request::ServiceRestart { service, apply } => {
                                    match sup.restart_service(k, &service, apply) {
                                        Ok(done) => done.render(),
                                        Err(refusal) => refusal.render(),
                                    }
                                }
                                Request::ServiceHold {
                                    kind,
                                    service,
                                    apply,
                                } => match sup.hold_service(k, kind, &service, apply) {
                                    Ok(done) => done.render(),
                                    Err(refusal) => refusal.render(),
                                },
                                // `ADR-021` §3: the floor is this process's,
                                // and only this process writes it.
                                Request::ClockFloorReset { apply } => {
                                    match clock.reset_floor(k, sup.journal_mut(), apply) {
                                        Ok(done) => done.render(),
                                        Err(refusal) => refusal.render(),
                                    }
                                }
                            };
                            k.answer_query(&answer);
                        }
                        // **`rivet_core::facts` — `ADR-057`, R-199 D4.**
                        //
                        // One line, because the whole answer is derived from
                        // `Supervisor::status()` — the same fold `get status`
                        // renders one arm above. That is `ADR-011` in the
                        // ordinary case and it is the reason this arm is one
                        // line: a screen and an operator asking over the
                        // control plane read one derivation, so the machine
                        // cannot tell them two different stories about what is
                        // running on it.
                        //
                        // **Answered inside the pass that read it**, like a
                        // status, so the age this answer carries is the age of
                        // the view it was composed from and not of one a
                        // convergence has since replaced.
                        Event::FactsAsked => {
                            let answer = sup.facts(k).render();
                            k.answer_facts(&answer);
                        }
                        // `rivet_core::kiosk` — ruled 2026-09-01 (`ADR-036` §5.4).
                        //
                        // **The narrowing is the socket, not this arm.** Every
                        // request that reaches here already came off a
                        // descriptor chowned to one application's uid and
                        // parsed as one of three verbs; there is nothing left
                        // to authorise. That is the whole reason it is a
                        // separate socket rather than a filter on the
                        // operator's.
                        Event::KioskAsked { request } => {
                            // **The lease is taken BEFORE the request is
                            // acted on** — `PLAN.md` §106.2.
                            //
                            // `select` starts the application, and the child
                            // must already have its end of the display channel
                            // when it is spawned. Taking it afterwards would
                            // mean opening a socket for a process that had
                            // already exec'd without one.
                            //
                            // Taken on every request, not only `select`: a
                            // descriptor left on the kernel would be closed by
                            // the *next* request rather than this one, and a
                            // `return` carrying a lease is a launcher defect
                            // worth closing promptly rather than holding.
                            let lease = k.take_kiosk_lease();
                            if let (Some(lease), rivet_core::kiosk::Request::Select { name }) =
                                (lease, &request)
                            {
                                grant_display(k, &mut sup, name, lease);
                            }
                            let answer = kiosk_request(k, &mut sup, &request);
                            k.answer_kiosk(&answer.render());
                        }
                        Event::KioskUnreadable { why } => {
                            sup.note(
                                k,
                                Severity::Warn,
                                "a request on the kiosk socket could not be read. The launcher \
                                 is the only caller and it is a program in this repository, so \
                                 this means the two halves disagree about the protocol rather \
                                 than that somebody is probing",
                                vec![("detail".to_string(), Value::Str(why.to_string()))],
                            );
                            k.answer_kiosk(
                                &rivet_core::kiosk::Answer::Refused {
                                    why: why.to_string(),
                                }
                                .render(),
                            );
                        }
                        Event::Ready { service } => sup.note_ready(k, &service),
                        Event::Status { service, text } => sup.note_status(k, &service, &text),
                        // `ADR-045` §3's three outcomes. The supervisor asked
                        // and this is what came back: a health line, a badly
                        // formed one, or nothing inside its own timeout — and
                        // the third is what a WEDGED application produces,
                        // which is the case the poll exists for.
                        Event::Answered { service, line } => {
                            sup.note_answered(k, &service, &line);
                        }
                        Event::Unanswered { service, why } => {
                            sup.note_unanswered(k, &service, &why);
                        }
                        // `ADR-045` §3 — `HEALTH=` on `NOTIFY_SOCKET` means
                        // *ask me now* and nothing more. The poll is the truth.
                        Event::HealthHint { service } => sup.note_health_hint(k, &service),
                        // ADR-022 §4's second capture path. One record per
                        // line, in the service's own name — so a mechanism's
                        // words reach /state/log rather than only the console.
                        Event::Output {
                            service,
                            stream,
                            text,
                            truncated,
                        } => sup.note_output(k, &service, stream, &text, truncated),
                        // ADR-022 §4.1. The kernel's own words, which are
                        // the last thing an HDMI capture showed that
                        // `rivet get log` did not.
                        Event::KernelMessage { record } => {
                            sup.note_kernel_message(k, &record);
                        }
                        // ADR-022 §11. A record composed by a process this
                        // supervisor did not start — the login shell, the
                        // editor — arriving on the one socket that exists so
                        // that §7's single writer stays single.
                        Event::Ingest { peer, what } => sup.note_ingest(k, peer, &what),
                        // ADR-017 §3: whatever was renamed aside — scratch,
                        // or a removal — is deleted by a helper off this loop,
                        // and this is the helper's report.
                        Event::Reclaimed {
                            service,
                            dir,
                            prefix,
                            failure,
                        } => {
                            sup.note_reclaimed(k, &service, &dir, &prefix, failure.as_deref());
                        }
                        // Both mean "look again", which is what happens next
                        // regardless. SIGCHLD coalesces, so it says which
                        // *nothing* about which child — `reap` asks.
                        Event::ChildChanged | Event::Timeout => {}
                    }
                }
            }
            Err(e) => return Err(Failure::Kernel(e)),
        }
        // Unconditionally, not only on SIGCHLD: signals coalesce, and in the
        // degraded mode where signalfd was refused there is no SIGCHLD at all.
        sup.reap(k);
        // **Immediately before the convergence, on every pass** — which is what
        // the maintainer's 2026-08-24 install-request ruling says: *"each convergence pass checks for
        // one"*. Here rather than inside `converge` so that convergence stays
        // the pure-decision-plus-perform loop `ADR-017` §3.2 describes, with no
        // filesystem read of its own; the cost is that these two calls have to
        // stay adjacent, and a request found by one is started by the other.
        requests.poll(k, &mut sup);
        removals.poll(k, &mut sup);
        library_removals.poll(k, &mut sup);
        repo_deletes.poll(k, &mut sup, &desired.document());
        // After the two request watches and before the convergence, so that an
        // application discovered on this pass is started by the same pass. It
        // reads `state_of` rather than the card on most passes, so the cost of
        // being here is one match.
        installs.poll(k, &mut sup);
        // After `installs`, always — see `crate::carried`: adopting the next
        // carried payload replaces the finished installer instance, and that
        // instance's edge is what makes a just-installed application a service
        // and delivers `ADR-049`'s staged credentials into it.
        carried.poll(k, &mut sup);
        sup.converge(k);
        // **The display window, after the convergence and before the clock.**
        // `ADR-039` §3.1 makes it a PREDICATE re-evaluated every pass rather
        // than a scheduled event, so there is nothing to miss and nothing that
        // a stepped wall clock can leave waiting for a moment that has passed
        // (§5 rule 2). It is cheap on every pass but two a day: one comparison,
        // and a return.
        // The reading is taken FIRST and into a local, because `converge`
        // needs the same `&mut dyn Kernel` to write its journal line - one
        // borrow at a time, sequenced rather than nested.
        let clock_face = k.local_time().as_ref().and_then(display_hours::clock_face);
        let window = desired.document().display().copied();
        television.converge(k, sup.journal_mut(), window.as_ref(), clock_face);
        // Likewise unconditional, and likewise cheap: `tick` compares one
        // monotonic reading against a deadline and returns. It runs after the
        // convergence rather than before it, because starting a service is what
        // the machine is for and a card write is not.
        clock.tick(k, sup.journal_mut());
        report_disk_trouble(k, &mut sup);
    }
}

/// Say, once per episode, that the journal could not reach the card.
///
/// **Here rather than inside `Journal::emit`, and that is not a style
/// preference.** The record describing a failed write is itself a record, and
/// writing it fails — so emitting it from the failure path is unbounded
/// recursion, on a full card, in the process that must not fail. So the journal
/// remembers and this asks.
///
/// The report reaches the console whatever the card is doing, which is the
/// property that matters: a board whose `/state` filled is exactly a board
/// whose journal cannot tell you that, and `ADR-017` §8 item 2 is about
/// precisely this shape of silence.
fn report_disk_trouble(k: &mut dyn Kernel, sup: &mut Supervisor) {
    let Some((detail, failures)) = sup.journal_mut().take_disk_trouble() else {
        return;
    };
    let corr = sup.journal_mut().next_correlation();
    sup.journal_mut().emit(
        k,
        Severity::Error,
        supervise::SELF_NAME,
        &corr,
        "the journal could NOT write to the card. Records from here on are on this \
         console and nowhere else, so `rivet log` will be missing them and a power \
         cycle loses them. A full or read-only /state is the usual cause, and neither \
         stops this supervisor supervising (ADR-017 §8 item 2)",
        vec![
            ("detail".to_string(), Value::Str(detail)),
            (
                "failed_writes".to_string(),
                Value::Int(i64::try_from(failures).unwrap_or(i64::MAX)),
            ),
        ],
    );
}

/// The sooner of two deadlines, where `None` means "no deadline of my own".
///
/// `None` from both is what makes a supervisor with nothing pending block on a
/// descriptor rather than spin, which is the obligation `ADR-017` §3.1 puts
/// first — so it must survive the clock's deadline being folded in.
fn soonest(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(only), None) | (None, Some(only)) => Some(only),
        (None, None) => None,
    }
}

/// Act on one `rivet_core::kiosk` request.
///
/// # `restart` is built; `select` and `return` are not, and they say so
///
/// The narrow kiosk socket, ruled 2026-09-01 (`ADR-036` §5.4). A launcher may reboot this machine today and
/// may not yet move the screen, because moving it means a **runtime display
/// selection** and the gate that would honour one currently destroys the fact
/// it needs.
///
/// `crate::document::apply_auto_run` sets `enabled = false` on every
/// display-declaring application that `[kiosk] auto-run` does not name. After
/// it runs, the definitions no longer record which applications the document
/// OFFERED, so a later selection has nothing to select from. The fix is the
/// one the design named: leave `enabled` alone, hold the selection beside it,
/// and make *should this run* the conjunction. That changes what convergence
/// means in the ten places `enabled` is read, and is not a change to make in
/// passing.
///
/// **Give one application the display the launcher just leased.**
///
/// `PLAN.md` §106.2. Two steps, in this order and no other:
///
/// 1. open the channel, so the descriptor exists before the application is
///    spawned and can be handed to it as `RIVET_DISPLAY`;
/// 2. write the lease into it, which the child reads once it is running.
///
/// The write happens **before** the spawn and that is deliberate: the socket is
/// a `SOCK_SEQPACKET` pair that already exists, so the message is buffered by
/// the kernel and waiting when the application first reads. Sending afterwards
/// would need this code to know when the child had exec'd, which is a race it
/// has no way to win.
///
/// **A failure is reported and not fatal.** The alternative is a launcher whose
/// `select` is refused because the screen could not be handed over — leaving a
/// machine with no menu and no application. The application starts either way
/// and finds no `RIVET_DISPLAY`, which is a state `SPEC-03` §3.7 already
/// describes and which the drop-master path (`§104.41`) still handles.
fn grant_display(
    k: &mut dyn Kernel,
    sup: &mut Supervisor,
    service: &str,
    lease: std::os::fd::OwnedFd,
) {
    if let Err(e) = k.open_display(service) {
        sup.note(
            k,
            Severity::Warn,
            "a display channel could not be opened for the application being selected, so it \
             will start without one and will not be able to draw",
            vec![
                ("application".to_string(), Value::Str(service.to_string())),
                ("detail".to_string(), Value::Str(e.to_string())),
            ],
        );
        return;
    }
    if let Err(e) = k.send_display(service, lease) {
        sup.note(
            k,
            Severity::Warn,
            "the display lease could not be written to the application's channel. The channel \
             is open and the application will start, but it will wait for a descriptor that \
             never comes",
            vec![
                ("application".to_string(), Value::Str(service.to_string())),
                ("detail".to_string(), Value::Str(e.to_string())),
            ],
        );
        return;
    }
    sup.note(
        k,
        Severity::Info,
        "the launcher leased the display to this application and the descriptor is waiting on \
         its channel (RIVET_DISPLAY). The launcher REMAINS DRM master throughout - ADR-036 \
         §2 rules every kiosk application a lessee",
        vec![("application".to_string(), Value::Str(service.to_string()))],
    );
}

/// **Refused with that sentence rather than accepted and ignored.** A launcher
/// told `ok` that saw nothing happen would be debugging the wrong end.
fn kiosk_request(
    k: &mut dyn Kernel,
    sup: &mut Supervisor,
    request: &rivet_core::kiosk::Request,
) -> rivet_core::kiosk::Answer {
    use rivet_core::kiosk::{Answer, Request};
    match request {
        Request::Restart => {
            sup.note(
                k,
                Severity::Info,
                "the launcher asked this machine to restart, and it was asked of PID 1 \
                 rather than done here: ADR-017 §3.1.10 has pid 1 stop the supervisor, \
                 wait for it, sync the filesystems and only then ask the kernel, none of \
                 which this process could do for itself",
                Vec::new(),
            );
            match k.signal_process(1, crate::kernel::Sig::Int) {
                Ok(()) => Answer::Taken {
                    what: "asked pid 1 to restart this machine".to_string(),
                },
                Err(e) => Answer::Refused {
                    why: format!("pid 1 could not be signalled: {e}"),
                },
            }
        }
        Request::Select { name } => match sup.select_display(name) {
            Ok(what) => {
                sup.note(
                    k,
                    Severity::Info,
                    "a launcher gave the screen to an application. The convergence at the \
                     bottom of the loop starts it and stops whatever held it before, by the \
                     same rules every service is always subject to -- ADR-017 §6 still has no \
                     manual start and this is not one, it changes what the machine converges \
                     TOWARD",
                    vec![("svc_name".to_string(), Value::Str(name.clone()))],
                );
                // **Write the selection down so it survives a reboot** —
                // restored by `kiosk_selection::restore` at boot. The selection already took in memory, so a persistence
                // failure is logged and the machine carries on: it costs only
                // that the *next* boot comes up blank, never this one.
                match kiosk_selection::persist(k, name) {
                    Ok(_) => {}
                    Err(errno) => sup.note(
                        k,
                        Severity::Warn,
                        "the display selection was applied but could not be written to /state, so \
                         it will NOT survive a reboot -- the machine would come up with nothing on \
                         the screen. The selection itself is in effect now (ADR-018 §8)",
                        vec![
                            ("svc_name".to_string(), Value::Str(name.clone())),
                            ("path".to_string(), Value::Str(kiosk_selection::path())),
                            ("errno".to_string(), Value::Str(errno.to_string())),
                        ],
                    ),
                }
                Answer::Taken { what }
            }
            Err(why) => Answer::Refused { why },
        },
        // **The one verb that reads.** No `note` on the journal: a locked
        // launcher asks this on a timer, and a record per poll would bury every
        // other thing the machine had to say (`ADR-022`).
        Request::Health { name } => match sup.health_of(name) {
            Some((running, attempts, pid)) => Answer::Health {
                running,
                attempts,
                pid,
            },
            // **Refused rather than answered `stopped`.** A service this
            // machine does not have will never start, and a launcher told
            // `stopped` would wait for it for ever. This is the document naming
            // something that is not installed, and it has to read as that.
            None => Answer::Refused {
                why: format!(
                    "`{name}` is not a service on this machine, so there is no health to \
                     report. This is `[kiosk] auto-run` naming something that is not \
                     installed, which ADR-007 §3.6.3 keeps rather than refuses because a \
                     document is authored anywhere -- so it resolves here, visibly"
                ),
            },
        },
        // **The second verb that reads, and it is not journalled either.** A
        // launcher asks this once at start-up today, but `ADR-042` §8.2 expects
        // it to become the thing a launcher re-asks after a reload — and a
        // record per ask would bury everything else the machine had to say
        // (`ADR-022`), which is the argument `health` above already makes.
        Request::Offer => {
            let (mode, locked, auto_run, offered) = sup.kiosk_offer();
            Answer::Offer {
                mode,
                locked,
                auto_run,
                offered,
            }
        }
        Request::Return => {
            let what = sup.return_display(k);
            sup.note(
                k,
                Severity::Info,
                "a launcher took the screen back; nothing is selected and the convergence \
                 below stops whatever held it",
                vec![("outcome".to_string(), Value::Str(what.clone()))],
            );
            // **Forget the stored selection** so the next boot restores nothing
            // rather than re-showing an application the operator dismissed —
            // `crate::kiosk_selection`. An absent file is not a failure; any other
            // error is logged and the machine carries on.
            if let Err(errno) = kiosk_selection::clear(k) {
                sup.note(
                    k,
                    Severity::Warn,
                    "the screen was returned but the stored selection could not be cleared from \
                     /state, so the next boot may restore an application the launcher just \
                     dismissed. Nothing holds the screen now (ADR-018 §8)",
                    vec![
                        ("path".to_string(), Value::Str(kiosk_selection::path())),
                        ("errno".to_string(), Value::Str(errno.to_string())),
                    ],
                );
            }
            Answer::Taken { what }
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse_args(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("rivet-sysd: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut k = LibcKernel::new(options.console);
    match run(&mut k, options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // PID 1 restarts this with backoff (ADR-017 §3.1.5 item 3), and the
            // services it started keep running in the meantime (§3.1.1).
            eprintln!("rivet-sysd: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::ExitStatus;
    use crate::supervise::State;
    use crate::testkernel::{FakeKernel, Script};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    /// How many services the image itself defines, so that the harness tests
    /// below say what they mean rather than restating a number that changes
    /// whenever `ADR-017` §2.1's inventory does.
    fn compiled_in() -> usize {
        definitions::system_services().len()
    }

    /// One harness service by name, out of a set that also carries the image's
    /// own definitions.
    fn dev<'o>(o: &'o Options, name: &str) -> &'o ServiceDef {
        let Some(found) = o.services.iter().find(|s| s.name == name) else {
            unreachable!("{name:?} is not among the parsed services")
        };
        found
    }

    /// An `Options` with **no** services at all.
    ///
    /// Not reachable through `parse_args` any more, and deliberately so: an
    /// empty argv is what PID 1 hands this process, so it must produce the real
    /// service set. The three tests that use this are about the supervisor's
    /// behaviour with nothing to do, which is still a state it must survive.
    fn no_services() -> Options {
        Options {
            console: ConsoleForm::Jsonl,
            services: Vec::new(),
        }
    }

    // -- arguments ----------------------------------------------------------

    /// ~~No arguments means no services.~~ It means **the image's own
    /// definitions**, since 2026-08-23. PID 1 spawns this binary with an empty
    /// argv (`cmd/rivet-initd`'s `Kernel::spawn` takes a path and nothing
    /// else), so this exact call is what a real boot makes — and if it produced
    /// an empty set the machine would supervise nothing while looking healthy.
    #[test]
    fn no_arguments_means_the_images_own_service_definitions() {
        let o = parse_args(&[]).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(o.services, definitions::system_services());
        assert!(
            !o.services.is_empty(),
            "an empty argv is what PID 1 uses; it must not mean 'supervise nothing'"
        );
        assert_eq!(o.console, ConsoleForm::Jsonl);
    }

    #[test]
    fn a_dev_service_carries_its_program_and_arguments() {
        let o = parse_args(&args(&["--dev-service", "a=/bin/sleep 5"]))
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(o.services.len(), compiled_in() + 1);
        let a = dev(&o, "a");
        assert_eq!(a.program.path, "/bin/sleep");
        assert_eq!(a.program.args, vec!["5".to_string()]);
        assert_eq!(a.restart, RestartPolicy::Always);
        assert_eq!(a.readiness, Readiness::OnStart);
    }

    #[test]
    fn several_dev_services_are_all_kept() {
        let o = parse_args(&args(&[
            "--dev-service",
            "a=/bin/true",
            "--dev-service",
            "b=/bin/false",
        ]))
        .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(o.services.len(), compiled_in() + 2);
    }

    /// A harness entry that collides with a compiled-in definition is a
    /// duplicate, and `Supervisor::new` refuses the pair rather than letting
    /// two `ServiceDef`s share one cgroup and one notify socket.
    #[test]
    fn a_dev_service_that_shadows_a_real_one_is_refused_rather_than_merged() {
        let o = parse_args(&args(&[
            "--dev-service",
            &format!("{}=/bin/true", definitions::NETD_NAME),
        ]))
        .unwrap_or_else(|e| unreachable!("{e}"));
        let journal = Journal::new("b".to_string(), 1, supervise::ACTOR.to_string());
        assert!(matches!(
            Supervisor::new(o.services, Limits::default(), journal),
            Err(DefError::Duplicate { .. })
        ));
    }

    #[test]
    fn a_restart_policy_may_be_given_before_or_after_the_service_it_names() {
        for order in [
            vec!["--dev-service", "a=/bin/true", "--dev-restart", "a=never"],
            vec!["--dev-restart", "a=never", "--dev-service", "a=/bin/true"],
        ] {
            let o = parse_args(&args(&order)).unwrap_or_else(|e| unreachable!("{e}"));
            assert_eq!(dev(&o, "a").restart, RestartPolicy::Never);
        }
    }

    /// The flag names a service, so it must be able to name a compiled-in one
    /// as well as a harness one - that is how a compiled-in service on a board
    /// gets its restart policy changed for one boot without a rebuild.
    #[test]
    fn a_restart_policy_may_name_a_compiled_in_service() {
        let o = parse_args(&args(&[
            "--dev-restart",
            &format!("{}=never", definitions::NETD_NAME),
        ]))
        .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(
            dev(&o, definitions::NETD_NAME).restart,
            RestartPolicy::Never
        );
    }

    #[test]
    fn dev_notify_selects_the_declared_readiness_path() {
        let o = parse_args(&args(&[
            "--dev-service",
            "a=/bin/true",
            "--dev-notify",
            "a",
        ]))
        .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(dev(&o, "a").readiness, Readiness::Notify);
    }

    /// A flag that silently did nothing is the kind of quiet no-op that costs
    /// an hour of confusion.
    #[test]
    fn a_flag_that_names_no_service_is_refused_rather_than_ignored() {
        assert!(matches!(
            parse_args(&args(&["--dev-restart", "ghost=never"])),
            Err(Failure::Arguments(_))
        ));
        assert!(matches!(
            parse_args(&args(&["--dev-notify", "ghost"])),
            Err(Failure::Arguments(_))
        ));
    }

    #[test]
    fn an_unknown_restart_policy_is_refused_and_names_the_three_that_exist() {
        let err = parse_args(&args(&[
            "--dev-service",
            "a=/bin/true",
            "--dev-restart",
            "a=sometimes",
        ]));
        let Err(Failure::Arguments(message)) = err else {
            unreachable!("a bad policy was accepted")
        };
        assert!(message.contains("on-failure"), "{message}");
    }

    #[test]
    fn a_flag_with_no_value_is_refused_rather_than_read_past() {
        for flag in ["--dev-service", "--dev-restart", "--dev-notify"] {
            assert!(
                matches!(parse_args(&args(&[flag])), Err(Failure::Arguments(_))),
                "{flag} read past the end of the arguments"
            );
        }
    }

    #[test]
    fn an_unknown_argument_is_refused() {
        assert!(matches!(
            parse_args(&args(&["--supervise-everything"])),
            Err(Failure::Arguments(_))
        ));
    }

    #[test]
    fn a_dev_service_without_an_equals_is_refused() {
        assert!(matches!(
            parse_args(&args(&["--dev-service", "just-a-name"])),
            Err(Failure::Arguments(_))
        ));
    }

    #[test]
    fn the_console_form_can_be_switched_to_the_human_view() {
        let o = parse_args(&args(&["--text"])).unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(o.console, ConsoleForm::Text);
    }

    // -- run ----------------------------------------------------------------

    fn kernel_with_events(events: Vec<Vec<Event>>) -> FakeKernel {
        let mut k = FakeKernel::new();
        k.observations = Script::always(kernel::Observation::Empty);
        k.wait_results = Script::sequence(
            events.into_iter().map(Ok).collect(),
            Err(Errno::EIO), // stop the test rather than looping for ever
        );
        k
    }

    /// Parse a harness invocation and **drop the image's own definitions from
    /// it**, so the assertions below are about the machinery.
    ///
    /// Added 2026-08-23 with [`definitions::system_services`]. Without it every
    /// one of these tests would count the image's own services among its spawns
    /// and signals, which would make each of them assert two things at once and
    /// go stale whenever `ADR-017` §2.1's inventory grew. `definitions`'s own
    /// tests are where the real set is checked.
    fn harness(list: &[&str]) -> Options {
        let mut o = parse_args(&args(list)).unwrap_or_else(|e| unreachable!("{e}"));
        let compiled: Vec<String> = definitions::system_services()
            .into_iter()
            .map(|d| d.name)
            .collect();
        o.services.retain(|s| !compiled.contains(&s.name));
        o
    }

    /// The obligation the whole crate exists to meet: with nothing to
    /// supervise, the supervisor waits rather than exiting and being restarted
    /// by PID 1 in a loop.
    ///
    /// ~~indefinitely, on a descriptor~~ — **amended 2026-08-23 with
    /// [`clock`].** The supervisor now has one periodic obligation of its own
    /// (`ADR-021` §3's hourly write), so a machine with nothing to supervise
    /// waits for *that* rather than for ever. The property the assertion is
    /// actually about is unchanged and is what is checked: the wait is a whole
    /// hour, **not a short poll**. A supervisor that spun would show a small
    /// number here.
    #[test]
    fn with_no_services_it_waits_indefinitely_instead_of_exiting() {
        let mut k = kernel_with_events(vec![vec![Event::Timeout], vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }]]);
        let result = run(&mut k, no_services());
        assert_eq!(result, Ok(()));
        assert_eq!(
            k.waits,
            vec![
                Some(clock::WRITE_INTERVAL_MS),
                Some(clock::WRITE_INTERVAL_MS)
            ],
            "a supervisor with nothing pending must block until its own next deadline, not poll"
        );
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Warn && r.msg.starts_with("no services are defined")),
            "the empty service set must be said out loud"
        );
    }

    /// **A fleet-root machine's boot issues an installed `fleet-tls`
    /// application the certificate it lacks, and does so through the boot
    /// itself** — `ADR-063` §4. The root is minted by this test
    /// (self-generated randomness); the fake records the order and forks
    /// nothing.
    #[test]
    fn a_fleet_root_machine_issues_a_missing_certificate_at_boot() {
        let mut k = kernel_with_events(vec![vec![Event::Terminate {
            how: crate::kernel::Terminating::PowerOff,
        }]]);
        let Ok(request) =
            rivet_core::fleet_root::Request::new("dev-fleet", "dev-01", &["local".to_string()])
        else {
            unreachable!("the fixture request is legal")
        };
        let mut next = 9u8;
        let mut counting = |len: usize| -> Result<Vec<u8>, String> {
            Ok((0..len)
                .map(|_| {
                    next = next.wrapping_add(37);
                    next
                })
                .collect())
        };
        let Ok(minted) = rivet_fleet_root::mint(&request, &mut counting, 0) else {
            unreachable!("the fixture root mints")
        };
        let root = rivet_core::fleet_root::root_certificate_path();
        k.files_mut()
            .insert(root.clone(), Script::always(Ok(minted.root.certificate)));
        k.exists_mut().insert(root, Script::always(Ok(true)));
        k.files_mut().insert(
            format!("{}/{}", rivet_core::state::SECRETS, rivet_core::fleet_root::MINTED_FOR_FILE),
            Script::always(Ok(request.render())),
        );
        let Some(app) = rivet_core::service::ServiceName::new("aura-controller") else {
            unreachable!("the fixture name is a name")
        };
        k.dirs_mut().insert(
            rivet_core::service::app_root(rivet_core::service::BUILT_IN_VOLUME),
            vec!["aura-controller".to_string()],
        );
        k.files_mut().insert(
            rivet_core::service::manifest_path(rivet_core::service::BUILT_IN_VOLUME, &app),
            Script::always(Ok(rivet_core::package::application::Manifest::new(
                rivet_core::package::application::ProgramKind::Daemon,
            )
            .with_material(vec![rivet_core::package::application::Material::FleetTls])
            .render())),
        );

        let _ = run(&mut k, no_services());

        assert_eq!(k.issues.len(), 1, "{:?}", k.records.iter().map(|r| &r.msg).collect::<Vec<_>>());
        assert_eq!(
            k.issues[0].targets[0].dir,
            rivet_core::service::data_dir(rivet_core::service::BUILT_IN_VOLUME, &app)
        );
        assert!(!k.reads.contains(&rivet_core::fleet_root::root_key_path()));
    }

    /// **The supervisor exempts itself from the OOM killer before anything
    /// starts**, and says so — `ADR-017` §3.
    #[test]
    fn the_supervisor_exempts_itself_from_the_oom_killer_at_start() {
        let mut k = kernel_with_events(vec![vec![Event::Terminate {
            how: crate::kernel::Terminating::PowerOff,
        }]]);
        let _ = run(&mut k, no_services());
        assert_eq!(k.own_oom_score_adj, [definitions::SUPERVISOR_OOM_SCORE_ADJ]);
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Info && r.msg.contains("OOM killer")),
            "{:?}",
            k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
    }

    /// A refusal is survivable and said, at `Warn`.
    #[test]
    fn a_supervisor_the_kernel_will_not_exempt_says_so_and_runs() {
        let mut k = kernel_with_events(vec![vec![Event::Terminate {
            how: crate::kernel::Terminating::PowerOff,
        }]]);
        k.own_oom_score_adj_result = Err(crate::kernel::Errno::EACCES);
        assert_eq!(run(&mut k, no_services()), Ok(()));
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Warn && r.msg.contains("could NOT exempt itself")),
            "{:?}",
            k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
    }

    /// `ADR-022` §12 item 1's boot record must carry the slot a partition's own
    /// sysfs entry names, not the one a bare mount-table minor would give —
    /// `sdb3` is device 8:19, and a reader that took 19 for the partition
    /// number would leave `unknown` in a record where `a` belongs.
    #[test]
    fn the_boot_record_carries_the_slot_sysfs_names() {
        let mut k = kernel_with_events(vec![vec![Event::Timeout], vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }]]);
        k.files_mut().insert(
            kernel::MOUNTINFO_PATH.to_string(),
            Script::always(Ok("20 1 8:19 / / rw,relatime - ext4 /dev/root rw\n".to_string())),
        );
        k.files_mut().insert(
            "/sys/dev/block/8:19/partition".to_string(),
            Script::always(Ok("3\n".to_string())),
        );
        let result = run(&mut k, no_services());
        assert_eq!(result, Ok(()));

        let Some(started) = k.records.iter().find(|r| r.msg == "supervisor starting") else {
            unreachable!("the supervisor must log its own start");
        };
        let slot = started
            .fields
            .iter()
            .find(|(name, _)| name == "slot")
            .map(|(_, v)| v.clone());
        assert_eq!(
            slot,
            Some(Value::Str("a".to_string())),
            "{:?}",
            started.fields
        );
    }

    /// The whole of the change of 2026-08-23, from the outside: a boot with the
    /// argv PID 1 actually supplies starts the image's own services rather than
    /// announcing that it has none.
    #[test]
    fn a_real_boot_starts_the_images_own_services() {
        let mut k = kernel_with_events(vec![vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }], vec![Event::Timeout]]);
        k.obey_sigterm = true;
        let options = parse_args(&[]).unwrap_or_else(|e| unreachable!("{e}"));
        let result = run(&mut k, options);
        assert_eq!(result, Ok(()));

        let started: Vec<_> = k.spawned.iter().map(|(n, _)| n.as_str()).collect();
        assert!(
            started.contains(&definitions::NETD_NAME),
            "the supervisor did not start rivet-netd: {started:?}"
        );
        // **The control plane, which is what a boot has to bring up.**
        // `ADR-017` §6.2.1: since `ADR-035` phase 3 a board that boots without
        // this cannot be reached at all, which is the one failure `PLAN.md`
        // §28.2 names as unrecoverable.
        assert!(
            started.contains(&definitions::MCPD_NAME),
            "the supervisor did not start the control plane: {started:?}"
        );
        // **A disabled definition is not started**, which is the other half of
        // `ADR-017` §6 and the half a boot can get wrong silently: a card that
        // started a service its own defaults say is off would look identical in
        // the journal to one that did not.
        for def in definitions::system_services().iter().filter(|d| !d.enabled) {
            assert!(
                !started.contains(&def.name.as_str()),
                "{} ships disabled and a boot started it: {started:?}",
                def.name
            );
        }
        assert!(
            !k.records
                .iter()
                .any(|r| r.msg.starts_with("no services are defined")),
            "a real boot must not report an empty service set"
        );
    }

    /// **A boot from a document with a DHCP interface starts that interface's
    /// client as a service of its own** — R-253 point 1, from the outside:
    /// the stored document is read, the client is derived from its
    /// `[network]` table, and the supervisor spawns it beside `rivet-netd`
    /// rather than leaving netd to.
    #[test]
    fn a_boot_with_a_dhcp_interface_starts_its_client_as_a_service() {
        let mut k = kernel_with_events(vec![vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }], vec![Event::Timeout]]);
        k.obey_sigterm = true;
        k.files_mut().insert(
            rivet_store::ConfigStore::at_default().machine_path(),
            crate::testkernel::Script::always(Ok(
                "version = 1\n[network.eth0]\nmethod = \"dhcp\"\n".to_string()
            )),
        );
        let _ = run(
            &mut k,
            parse_args(&[]).unwrap_or_else(|e| unreachable!("{e}")),
        );
        let client = rivet_core::service::dhcp_service_name("eth0");
        let Some((_, program)) = k.spawned.iter().find(|(n, _)| *n == client) else {
            unreachable!(
                "{client} was never spawned: {:?}",
                k.spawned.iter().map(|(n, _)| n).collect::<Vec<_>>()
            )
        };
        assert_eq!(program.path, rivet_adapter_net::dhcp::MECHANISM);
        assert_eq!(program.args, rivet_adapter_net::dhcp::arguments("eth0"));
        assert_eq!(
            program.capabilities,
            definitions::NETWORK_CAPABILITIES,
            "uid 0 reduced to the network's three, as rivet-netd is"
        );
    }

    /// `rivet-netd` is handed the path PID 1 seeds (`ADR-017` §3.1.9), and
    /// this asserts it is true on a boot rather than in a constant.
    #[test]
    fn netd_is_started_with_the_stored_documents_path() {
        let mut k = kernel_with_events(vec![vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }], vec![Event::Timeout]]);
        k.obey_sigterm = true;
        let _ = run(
            &mut k,
            parse_args(&[]).unwrap_or_else(|e| unreachable!("{e}")),
        );
        let Some((_, program)) = k.spawned.iter().find(|(n, _)| n == definitions::NETD_NAME) else {
            unreachable!("rivet-netd was never spawned")
        };
        assert_eq!(program.path, definitions::NETD_PATH);
        assert_eq!(program.args[0], "--document");
        assert_eq!(program.args[1], "/state/config/machine.toml");
    }

    /// A card carrying **none** of the binaries: every enabled service refuses
    /// once, loudly, naming what the machine lost, and nothing backs off.
    ///
    /// `ADR-017` §3.5.1's argument: `/system` is read-only `erofs`, so a
    /// program that is not there cannot appear while the machine runs, and
    /// backing off would spend four sleeps and five journal lines to reach the
    /// answer the first attempt already gave.
    #[test]
    fn no_service_whose_program_is_not_in_the_image_loops_over_it() {
        let mut k = kernel_with_events(vec![vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }], vec![Event::Timeout]]);
        k.obey_sigterm = true;
        // Every way of executing anything fails the way a missing binary does.
        k.spawn_results = Script::always(Err(Errno::ENOENT));
        k.run_results = Script::always(Err(Errno::ENOENT));
        let _ = run(
            &mut k,
            parse_args(&[]).unwrap_or_else(|e| unreachable!("{e}")),
        );

        let refusals: Vec<_> = k
            .records
            .iter()
            .filter(|r| r.msg.starts_with("REFUSED"))
            .collect();
        // ~~`definitions::system_services().len()`~~ — **narrowed 2026-08-23**,
        // when `rivet-timed` was defined with `enabled: false`. A disabled
        // service is never spawned, so it never refuses, and counting *defined*
        // services would have this test fail for a reason that has nothing to do
        // with what it is about.
        let expected = definitions::system_services()
            .iter()
            .filter(|d| d.enabled)
            .count();
        assert_eq!(
            refusals.len(),
            expected,
            "each ENABLED service must refuse exactly once, not back off and repeat: {:?}",
            refusals.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
        assert!(
            refusals.iter().all(|r| r.sev == Severity::Error),
            "a service that cannot run at all is not a warning"
        );
        // **Every refusal is the spawn one, derived rather than a literal.** A
        // literal count here has drifted twice already: it means *these
        // particular services*, and it goes stale the day the inventory or a
        // starting value moves, while still passing.
        assert_eq!(
            refusals
                .iter()
                .filter(|r| r.msg.starts_with("REFUSED this image cannot run"))
                .count(),
            expected,
            "an enabled service did not take the spawn-refusal path"
        );
        assert!(
            !k.records
                .iter()
                .any(|r| r.msg == "will retry after backoff"),
            "a program that is not on a read-only filesystem cannot appear later"
        );
        assert_eq!(
            k.spawned.len(),
            0,
            "no spawn succeeded, so none is recorded"
        );
        // Every refusal says what the machine lost, not merely which call
        // failed — `ADR-017` §3.1.6's `consequence_of`.
        for r in &refusals {
            assert!(
                r.to_jsonl().contains("\"consequence\":"),
                "a refusal did not say what it cost: {}",
                r.to_jsonl()
            );
        }
    }

    // ----------------------------------------------------------------------
    // ADR-021 §3: the wall clock, and where the restore sits in the start-up
    // ----------------------------------------------------------------------

    /// **The whole of the change, seen from `run`.** A board with a stored clock
    /// boots believing it is 1970 and its *first* record — the supervisor's own
    /// start-up line — already carries the restored time.
    ///
    /// This is the assertion the ordering contract exists for. One line further
    /// down in `run` and it would read 1970 on every board that has a stored
    /// clock, which is exactly what the 1970-clock finding, 2026-08-23 is complaining about.
    #[test]
    fn the_supervisors_own_first_record_is_stamped_from_the_restored_clock() {
        const A_PLAUSIBLE_MS: u64 = 1_700_000_000_000;
        let mut k = kernel_with_events(vec![vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }]]);
        // What a Raspberry Pi 4 actually came up at (the 1970-clock finding, 2026-08-23).
        k.wall_ms = Some(3_000);
        k.files_mut().insert(
            "/state/clock".to_string(),
            Script::always(Ok(format!("{A_PLAUSIBLE_MS} unsynchronised\n"))),
        );
        let _ = run(&mut k, no_services());

        let Some(first) = k.records.first() else {
            unreachable!("the supervisor recorded nothing at all")
        };
        let Some(ts) = &first.ts else {
            unreachable!("the first record carries no timestamp")
        };
        assert!(
            !ts.starts_with("1970"),
            "the first record was stamped before the clock was restored: {ts}"
        );
        // ...and every record after it, which is the property that matters to
        // anybody reading a journal off a board.
        for record in &k.records {
            let Some(ts) = &record.ts else {
                unreachable!("a record with no timestamp: {record:?}")
            };
            assert!(!ts.starts_with("1970"), "{}: {ts}", record.msg);
        }
    }

    /// The restore is **after** the journal exists, which is the other half of
    /// the ordering: a `/state` that cannot be read has somewhere to say so.
    #[test]
    fn a_clock_that_cannot_be_read_is_reported_and_the_machine_still_boots() {
        let mut k = kernel_with_events(vec![vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }], vec![Event::Timeout]]);
        k.obey_sigterm = true;
        k.files_mut()
            .insert("/state/clock".to_string(), Script::always(Err(Errno::EIO)));
        let result = run(
            &mut k,
            parse_args(&[]).unwrap_or_else(|e| unreachable!("{e}")),
        );
        assert_eq!(result, Ok(()));
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Error
                    && r.msg.contains("stored clock could not be read")),
            "a card that could not be read said nothing: {:?}",
            k.records
        );
        // And the machine is still a machine: the services started.
        assert!(
            k.spawned.iter().any(|(n, _)| n == definitions::NETD_NAME),
            "a bad clock stopped the boot"
        );
    }

    /// A boot that learns something the card does not know writes it before the
    /// first `poll(2)`, rather than spending that poll with a zero timeout.
    #[test]
    fn a_first_boot_stores_the_clock_before_it_waits_at_all() {
        let mut k = kernel_with_events(vec![vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }]]);
        k.wall_ms = Some(1_700_000_000_000);
        let _ = run(&mut k, no_services());
        assert_eq!(
            k.contents("/state/clock"),
            Some("1700000000000 unsynchronised\n".to_string())
        );
        assert_eq!(
            k.waits.first(),
            Some(&Some(clock::WRITE_INTERVAL_MS)),
            "the first wait must not be a zero-timeout poll"
        );
    }

    /// **`ADR-021` §3.3 from the outside**: a machine that is asked to stop
    /// writes what it knows about the time before it goes, rather than losing up
    /// to an hour of it.
    ///
    /// The join `crate::clock`'s own tests cannot make — it is `run` that
    /// decides *when* the write happens, and the whole ruling is about when.
    #[test]
    fn a_clean_shutdown_writes_the_clock_before_the_supervisor_exits() {
        const AN_HOUR_IN: u64 = clock::WRITE_INTERVAL_MS;
        let mut k = kernel_with_events(vec![vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }], vec![Event::Timeout]]);
        k.obey_sigterm = true;
        // A board that restored a floor from its card, half an hour ago — so
        // nothing is due, and without §3.3 nothing would be written at all.
        k.wall_ms = Some(3_000);
        k.files_mut().insert(
            "/state/clock".to_string(),
            Script::always(Ok(format!("{AN_HOUR_IN} unsynchronised\n"))),
        );
        let result = run(&mut k, no_services());
        assert_eq!(result, Ok(()));
        assert_eq!(
            k.contents("/state/clock"),
            Some(format!("{AN_HOUR_IN} unsynchronised\n")),
            "the shutdown wrote nothing, or wrote something other than the running clock"
        );
        // Counted over the clock's own temporary path rather than over every
        // write this boot made: `etc::project` writes six files of its own.
        let clock_writes = k
            .writes
            .iter()
            .filter(|w| w.path.starts_with("/state/clock"))
            .count();
        assert_eq!(
            clock_writes, 1,
            "the clock was written {clock_writes} times"
        );
    }

    /// And it happens **when the shutdown is requested**, not when it completes:
    /// PID 1's grace for stopping this process (5s) is shorter than this
    /// process's own grace for stopping a service (10s + 2s), so a write placed
    /// at the exit is one a service that ignores `SIGTERM` can cancel.
    #[test]
    fn the_shutdown_write_happens_at_the_request_rather_than_at_the_exit() {
        let mut k = kernel_with_events(vec![
            vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }],
            vec![Event::Timeout],
            vec![Event::ChildChanged],
            vec![Event::Timeout],
        ]);
        // The service does NOT obey SIGTERM, so the supervisor never reaches
        // `is_finished` at all — this run ends on the failing wait instead.
        k.obey_sigterm = false;
        // A card with a stored clock, so that nothing is due and the ONLY thing
        // that can produce a write is the shutdown.
        k.wall_ms = Some(3_000);
        k.files_mut().insert(
            "/state/clock".to_string(),
            Script::always(Ok(format!("{} unsynchronised\n", clock::WRITE_INTERVAL_MS))),
        );
        let options = harness(&["--dev-service", "a=/bin/sleep 99"]);
        let result = run(&mut k, options);
        assert_eq!(
            result,
            Err(Failure::Kernel(Errno::EIO)),
            "this run was supposed to end without ever finishing its shutdown"
        );
        assert!(
            k.contents("/state/clock").is_some(),
            "a service that ignores SIGTERM cancelled the clock write"
        );
    }

    /// `soonest` is what folds the clock's deadline into the supervisor's
    /// without letting either mask the other — including the case that keeps a
    /// supervisor with nothing pending blocking rather than spinning.
    #[test]
    fn the_sooner_of_two_deadlines_wins_and_two_absent_ones_stay_absent() {
        assert_eq!(soonest(None, None), None);
        assert_eq!(soonest(Some(5), None), Some(5));
        assert_eq!(soonest(None, Some(5)), Some(5));
        assert_eq!(soonest(Some(5), Some(9)), Some(5));
        assert_eq!(soonest(Some(9), Some(5)), Some(5));
        assert_eq!(soonest(Some(0), Some(9)), Some(0));
    }

    #[test]
    fn signals_are_armed_before_anything_is_spawned() {
        let mut k = kernel_with_events(vec![vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }]]);
        let _ = run(&mut k, harness(&["--dev-service", "a=/bin/true"]));
        assert_eq!(k.arm_calls, 1);
    }

    /// If the first convergence happened after the first wait, a supervisor
    /// with services would block on a descriptor with nothing pending and start
    /// nothing at all.
    #[test]
    fn services_start_before_the_first_wait() {
        let mut k = kernel_with_events(vec![]);
        k.wait_results = Script::always(Err(Errno::EIO));
        let result = run(&mut k, harness(&["--dev-service", "a=/bin/true"]));
        assert_eq!(result, Err(Failure::Kernel(Errno::EIO)));
        assert_eq!(k.spawned.len(), 1, "nothing started before the wait failed");
    }

    #[test]
    fn a_sigterm_stops_the_children_and_the_supervisor_exits_cleanly() {
        let mut k = kernel_with_events(vec![
            vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }],
            vec![Event::ChildChanged],
            vec![Event::Timeout],
        ]);
        // The child exits when it is signalled, which is what a well-behaved
        // one does.
        k.obey_sigterm = true;
        let result = run(&mut k, harness(&["--dev-service", "a=/bin/sleep 99"]));
        assert_eq!(result, Ok(()));
        assert_eq!(
            k.signals,
            vec![(testkernel::FIRST_FAKE_PID, kernel::Sig::Term)]
        );
    }

    #[test]
    fn a_readiness_notification_reaches_the_service_it_names() {
        let mut k = kernel_with_events(vec![
            vec![Event::Ready {
                service: "a".to_string(),
            }],
            vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }],
            vec![Event::Timeout],
        ]);
        k.obey_sigterm = true;
        let options = harness(&["--dev-service", "a=/bin/sleep 99", "--dev-notify", "a"]);
        let result = run(&mut k, options);
        assert_eq!(result, Ok(()));
        assert!(
            k.records.iter().any(|r| r.svc == "a" && r.msg == "ready"),
            "{:?}",
            k.records
        );
    }

    // ----------------------------------------------------------------------
    // ADR-018 section 10.3: growing /state, and where it sits in the start-up
    // ----------------------------------------------------------------------

    /// **`/state` is grown exactly once per boot, and before `/etc`.**
    ///
    /// `ADR-018` §10.3 rewrites a partition table. "At least once" is not the
    /// contract and a second call in one boot would mean a retry nothing rules;
    /// the count is asserted rather than the fact of a call for that reason.
    ///
    /// The ordering against `/etc` is asserted through the journal, because
    /// that is the only place the two are observable in sequence — and it is
    /// also what an operator reading a board would use to check it.
    #[test]
    fn state_is_grown_once_per_boot_and_before_etc_is_projected() {
        let mut k = kernel_with_events(vec![vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }]]);
        let _ = run(&mut k, no_services());

        assert_eq!(
            k.grew_called, 1,
            "ADR-018 §10.3 writes a partition table; it is a one-shot, not a retry"
        );

        // **Found by the outcome field, not by the message.** Every `Grew`
        // variant renders a different sentence and only some of them contain
        // the word `/state`, so a message matcher would pass today and break
        // the first time a host in this test suite actually has a card. The
        // `outcome` field carries the variant's own name, which is stable.
        let is_grow = |r: &crate::journal::Record| {
            r.fields.iter().any(|(key, value)| {
                key == "outcome" && matches!(value, Value::Str(text) if text.starts_with("Grown")
                    || text.starts_with("AlreadyFills")
                    || text.starts_with("PartitionOnly")
                    || text.starts_with("Refused")
                    || text.starts_with("Failed"))
            })
        };
        let grow_at = k.records.iter().position(is_grow);
        let etc_at = k.records.iter().position(|r| r.msg.contains("/etc projected"));
        let (Some(grow_at), Some(etc_at)) = (grow_at, etc_at) else {
            unreachable!(
                "both one-shots must report: grow={grow_at:?} etc={etc_at:?}. \
                 The fake answers ENOENT for a device that is not there, which is \
                 the ordinary answer on any host these tests run on."
            )
        };
        assert!(
            grow_at < etc_at,
            "the grow must come first — the earlier the GPT rewrite completes, \
             the smaller the window its write order is protecting"
        );
    }

    /// A grow that could not happen **must not stop the boot**.
    ///
    /// `ADR-018` §10.7.3: *"a failure degrades the boot and never withholds the
    /// confirmation — a rollback cannot repair a `/state` an update never
    /// touches."* The fake's default is the failure every host running this
    /// test produces, so this asserts the ordinary case rather than a contrived
    /// one: services still start.
    #[test]
    fn a_grow_that_failed_does_not_stop_the_supervisor() {
        let mut k = kernel_with_events(vec![vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }]]);
        let result = run(&mut k, no_services());
        assert!(
            result.is_ok(),
            "a /state that did not grow is a smaller card, not a failed boot: {result:?}"
        );
        assert!(
            k.records.iter().any(|r| r.msg.contains("/etc projected")),
            "the boot must have carried on past the grow"
        );
    }

    // ----------------------------------------------------------------------
    // ADR-017 §3.1.3.1: the machine-wide cgroup2 fact, said once
    // ----------------------------------------------------------------------

    /// A healthy machine says it, at `info`, once — and the per-service path
    /// then has nothing left to add.
    ///
    /// This line is what makes `Supervisor::report_unobservable`'s silence
    /// readable. Without it a console would look the same whether cgroup2 was
    /// fine or whether nothing had ever looked.
    #[test]
    fn a_readable_cgroup2_hierarchy_is_recorded_once_for_the_machine() {
        let mut k = kernel_with_events(vec![vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }]]);
        k.hierarchy_results = Script::always(Hierarchy::Present);
        let _ = run(&mut k, no_services());

        let said: Vec<_> = k
            .records
            .iter()
            .filter(|r| r.msg.contains("cgroup2 hierarchy"))
            .collect();
        assert_eq!(said.len(), 1, "{said:?}");
        assert_eq!(said[0].sev, Severity::Info);
        assert_eq!(said[0].svc, supervise::SELF_NAME);
    }

    /// **The case the old warning could not reach at all.**
    ///
    /// With no services — or with every service disabled — nothing ever calls
    /// `start`, so a machine with no cgroup2 used to boot in total silence
    /// about it. `ADR-017` §3.1.3 makes that hierarchy the authority for what
    /// is running; its absence is news whether or not anything is supervised.
    #[test]
    fn a_machine_with_no_cgroup2_says_so_even_when_it_supervises_nothing() {
        let mut k = kernel_with_events(vec![vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }]]);
        k.hierarchy_results = Script::always(Hierarchy::Unreachable(Errno::ENOENT));
        let _ = run(&mut k, no_services());

        let said: Vec<_> = k
            .records
            .iter()
            .filter(|r| r.msg.contains("no readable cgroup2 hierarchy"))
            .collect();
        assert_eq!(said.len(), 1, "{:?}", k.records);
        assert_eq!(said[0].sev, Severity::Warn);
        assert!(said[0].msg.starts_with("DEGRADED"), "{}", said[0].msg);
        assert!(
            k.spawned.is_empty(),
            "this run supervised nothing, which is the point of it"
        );
    }

    /// A kernel that cannot be waited on is not something the supervisor can
    /// work around, and it is exactly the case `ADR-017` §3.1 says this process
    /// is allowed to lose: PID 1 restarts it, and the services it started keep
    /// running while it is away.
    #[test]
    fn a_wait_that_fails_ends_the_process_rather_than_spinning() {
        let mut k = kernel_with_events(vec![]);
        k.wait_results = Script::always(Err(Errno::EIO));
        let result = run(&mut k, no_services());
        assert_eq!(result, Err(Failure::Kernel(Errno::EIO)));
    }

    #[test]
    fn a_refused_signal_arm_degrades_loudly_and_carries_on() {
        let mut k = kernel_with_events(vec![vec![Event::Timeout], vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }]]);
        k.arm_result = Err(Errno::EPERM);
        let result = run(&mut k, no_services());
        assert_eq!(result, Ok(()));
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Error && r.msg.starts_with("DEGRADED")),
            "a supervisor with no shutdown path must say so"
        );
    }

    #[test]
    fn an_invalid_service_ends_the_run_before_anything_is_spawned() {
        let mut k = kernel_with_events(vec![]);
        let options = Options {
            console: ConsoleForm::Jsonl,
            services: vec![ServiceDef {
                name: "a".to_string(),
                program: Program {
                    path: "relative".to_string(),
                    identity: None,
                    confinement: None,
                    groups: Vec::new(),
                    capabilities: Vec::new(),
                    args: Vec::new(),
                    env: Vec::new(),
                },
                enabled: true,
                restart: RestartPolicy::Always,
                readiness: Readiness::OnStart,
                consequence: String::new(),
                ..service::IMAGE_DEFINED
            }],
        };
        let result = run(&mut k, options);
        assert!(matches!(result, Err(Failure::Definition(_))));
        assert!(k.spawned.is_empty());
    }

    #[test]
    fn the_boot_identifier_comes_from_the_kernel_and_falls_back_loudly() {
        let mut k = kernel_with_events(vec![vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }]]);
        k.files_mut().insert(
            kernel::BOOT_ID_PATH.to_string(),
            Script::always(Ok("0d3f2e1a-0000-4000-8000-000000000000\n".to_string())),
        );
        let _ = run(&mut k, no_services());
        assert!(
            k.records
                .iter()
                .all(|r| r.boot == "0d3f2e1a-0000-4000-8000-000000000000"),
            "the trailing newline must not reach the record"
        );

        let mut k = kernel_with_events(vec![vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }]]);
        let _ = run(&mut k, no_services());
        assert!(k.records.iter().all(|r| r.boot == "unknown"));
    }

    #[test]
    fn a_service_that_dies_is_restarted_while_the_supervisor_keeps_running() {
        let mut k = kernel_with_events(vec![
            vec![Event::ChildChanged],
            vec![Event::Timeout],
            vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }],
            vec![Event::Timeout],
        ]);
        k.exits
            .push((testkernel::FIRST_FAKE_PID, ExitStatus::Exited(1)));
        let result = run(&mut k, harness(&["--dev-service", "a=/bin/false"]));
        assert_eq!(result, Ok(()));
        // The first wait has no service deadline, so it is bounded by the only
        // other one there is — `ADR-021` §3's hourly clock write (~~blocks~~,
        // amended 2026-08-23 with `crate::clock`). The second is bounded by the
        // backoff, which is sooner, and **that** is the assertion this test is
        // about: a restart deadline must win over anything further away.
        assert_eq!(k.waits[0], Some(clock::WRITE_INTERVAL_MS));
        assert_eq!(k.waits[1], Some(250));
    }

    #[test]
    fn a_failure_renders_something_a_human_can_act_on() {
        assert!(Failure::Kernel(Errno::EPERM).to_string().contains("kernel"));
        assert!(
            Failure::Definition(DefError::EmptyName)
                .to_string()
                .contains("service definition")
        );
        assert!(
            Failure::Arguments("bad".to_string())
                .to_string()
                .contains("bad")
        );
    }

    #[test]
    fn a_started_service_is_reported_as_started_and_not_as_ready() {
        let mut k = kernel_with_events(vec![vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }], vec![Event::Timeout]]);
        k.obey_sigterm = true;
        let options = harness(&["--dev-service", "a=/bin/sleep 99", "--dev-notify", "a"]);
        let _ = run(&mut k, options);
        assert!(
            !k.records.iter().any(|r| r.msg == "ready"),
            "readiness must be declared, never inferred from having started"
        );
    }

    #[test]
    fn the_state_of_a_named_service_can_be_asked_for() {
        let mut k = FakeKernel::new();
        k.observations = Script::always(kernel::Observation::Empty);
        let journal = Journal::new("b".to_string(), 1, supervise::ACTOR.to_string());
        let options = harness(&["--dev-service", "a=/bin/true"]);
        let mut sup = Supervisor::new(options.services, Limits::default(), journal)
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(sup.state_of("a"), Some(&State::Idle));
        assert_eq!(sup.state_of("nothing"), None);
        sup.converge(&mut k);
        assert!(matches!(sup.state_of("a"), Some(State::Started { .. })));
    }

    // -- the journal's ingest socket (`ADR-022` §11) ------------------------

    /// **Bound once, at the path `rivet-core` names**, and before the first
    /// convergence — so nothing this boot starts can beat it into existence.
    ///
    /// The path is asserted against the constant rather than a literal because
    /// three binaries have to agree about it and only one of them can own it
    /// (`ADR-011`): a supervisor listening somewhere the CLI does not connect is
    /// a machine whose edits silently never reach `rivet get log`.
    #[test]
    fn the_ingest_socket_is_bound_once_at_the_path_the_core_names() {
        let mut k = kernel_with_events(vec![vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }]]);
        assert_eq!(run(&mut k, no_services()), Ok(()));
        assert_eq!(
            k.ingest_bound,
            vec![rivet_core::journal::INGEST_SOCKET_PATH.to_string()]
        );
    }

    /// **A socket that will not bind is survivable and is SAID.** Every sender
    /// falls back to its own stderr, so the failure is a machine whose journal
    /// is missing the operator's own edits — which is exactly the kind of
    /// silence `ADR-017` §8 item 2 rules against.
    #[test]
    fn a_socket_that_will_not_bind_is_reported_and_the_supervisor_carries_on() {
        let mut k = kernel_with_events(vec![vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }]]);
        k.ingest_result = Err(Errno::EROFS);
        assert_eq!(run(&mut k, no_services()), Ok(()));
        let said = k
            .records
            .iter()
            .find(|r| r.msg.contains("will NOT accept records"))
            .unwrap_or_else(|| unreachable!("a refused ingest socket said nothing"));
        assert_eq!(said.sev, Severity::Warn);
        assert_eq!(
            said.field("path"),
            Some(&Value::Str(
                rivet_core::journal::INGEST_SOCKET_PATH.to_string()
            ))
        );
    }

    /// One arriving record, through `run`'s own event loop rather than through
    /// `note_ingest` directly — the wiring, which a test of the method alone
    /// would not touch.
    #[test]
    fn a_record_arriving_on_the_socket_reaches_the_journal_through_the_loop() {
        let line = rivet_core::journal::Record {
            ts: None,
            boot: "the-senders-idea-of-a-boot".to_string(),
            mono: 1.0,
            sev: Severity::Info,
            svc: "femto".to_string(),
            corr: "c".to_string(),
            actor: "operator".to_string(),
            msg: "wrote /state/notes.txt".to_string(),
            fields: vec![],
        }
        .to_jsonl();
        let mut k = kernel_with_events(vec![
            vec![Event::Ingest {
                peer: rivet_core::journal::Peer {
                    uid: 0,
                    gid: 0,
                    pid: 77,
                },
                what: kernel::Ingest::Line(line.trim_end().to_string()),
            }],
            vec![Event::Terminate { how: crate::kernel::Terminating::PowerOff }],
        ]);
        assert_eq!(run(&mut k, no_services()), Ok(()));
        let mine = k
            .records
            .first()
            .unwrap_or_else(|| unreachable!("the supervisor wrote nothing at all"))
            .boot
            .clone();
        let landed = k
            .records
            .iter()
            .find(|r| r.msg == "wrote /state/notes.txt")
            .unwrap_or_else(|| unreachable!("the record never reached the journal"));
        assert_eq!(landed.svc, "femto");
        assert_eq!(landed.actor, "uid:0");
        // The boot this machine is actually on, whatever it is — not the one
        // the sender put on the wire.
        assert_eq!(landed.boot, mine, "the sender chose the boot");
        assert_ne!(landed.boot, "the-senders-idea-of-a-boot");
    }
}
