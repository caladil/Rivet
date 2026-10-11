//! **Services an installed application defines.** `ADR-017` §4, second bullet.
//!
//! > * **Application services** come from the application's own manifest
//! >   (`ADR-015` §3.2). Installing the application is what makes the service
//! >   exist.
//!
//! [`crate::definitions`] is the first bullet — the services the image
//! provides, compiled into this binary. This module is the second, and until
//! 2026-08-24 nothing was: `ADR-015` §3.2's manifest and `ADR-007` §7.2's
//! document both claimed the same keys, so there was no answer to *"where does
//! an application service's restart policy come from"* that did not pick a side
//! of an unresolved conflict. The maintainer's 2026-08-24 ruling picked it.
//!
//! # What comes from where, on this path exactly
//!
//! | Field | From | Authority |
//! | :-- | :-- | :-- |
//! | name | the directory under `rivet_core::service::APP_ROOT` | the filesystem — the manifest deliberately carries no name |
//! | program | `rivet_core::service::program_path` | **derived**; `ADR-015` §3.2 has no `exec` field |
//! | uid and gid | `rivet_core::service::app_identity` | **derived** from the name, 2026-08-24 — see below |
//! | `restart`, `ready`, `listen` | the manifest | inside the signature (`ADR-015` §4.1) |
//! | memory **request** | the manifest | inside the signature |
//! | memory **cap** | the document | the operator (`ADR-007` §7.2) — applied in [`crate::document`] |
//! | `enabled` | the document, over a starting value of **`false`** | the operator (`ADR-017` §6) |
//!
//! **~~`enabled` starts `true` and that is a judgement~~ — RULED 2026-08-25,
//! and it starts `false`.** The paragraph that stood here argued that "an
//! operator who installed something intended it to run", that the alternative
//! "would make every install a two-step operation", and that `ADR-015` §5.4's
//! `plan` shows the grant at install time. The maintainer ruled against it. The
//! argument it lost to is `ADR-015` §3.2.1's own, which was already in the
//! record: *an application that could declare itself enabled would be one an
//! operator could not turn off* — and a starting value of `true` is that
//! outcome by another route, because it reads silence as consent.
//!
//! **Installing is not running.** The two-step cost is real and was accepted;
//! `cargo rivet deploy --enable` is where it is paid, in the host tool, from
//! a word the operator typed. `PLAN.md` §20 carries the ruling, what it costs a
//! machine that already has applications on it, and why the image's own
//! services keep starting enabled.
//!
//! # Three things this does not do, each because nothing exists to do it with
//!
//! 1. ~~**It does not verify a signature.**~~ **Built 2026-08-25, and this text
//!    went on denying it until a board printed the denial about an application
//!    it had just verified** (`PLAN.md` §19.5). `ADR-015` §4.2's launch-time
//!    verification is `crate::verify::program`, and `crate::supervise` runs it
//!    immediately before every application spawn — so what this module reports,
//!    once per discovered application, is now that the check happens rather
//!    than that it does not.
//! 2. ~~**It does not sandbox.** `ADR-015` §5.1's UID, namespaces, Landlock and
//!    seccomp are all unbuilt; a discovered application runs as the supervisor
//!    does.~~ **Half of that stopped being true on 2026-08-24 — see the next
//!    section.** The row of §5.1's table that is now live is the first one,
//!    *Identity*; namespaces, Landlock, cgroup network isolation and seccomp
//!    are still unbuilt and a discovered application still shares every
//!    namespace with PID 1. Every declaration the manifest makes about that is
//!    named in the report — `Manifest::unenforced` is the artifact's own list,
//!    so this module cannot fall behind it.
//!
//!    **`Manifest::unenforced` still lists the identity among what is not
//!    enforced, and it is now wrong about that.** It lives in
//!    `rivet_core::package::application`, which this change did not own;
//!    `PLAN.md` §0.11 carries it as the one stale sentence this left behind,
//!    rather than it being corrected silently in a crate another agent is
//!    editing.
//! 3. **It does not install.** Nothing does (`ADR-029` §12 item 4 is ruled and
//!    unwritten), so every manifest this reads today is one a test or a harness
//!    wrote. What it means for the manifest to be *there* is `ADR-015` §7's
//!    atomic install: unpack to `/state/app/<name>/<version>/`, verify, swap the
//!    `current` symlink. This module reads through that symlink and holds no
//!    opinion about what put it there.
//!
//! # The identity, ruled 2026-08-24 — `ADR-015` §5.1, `ADR-008` §7.3
//!
//! **An installed application no longer runs as root.** Every application
//! discovered here is given a uid and a gid of its own, and
//! [`crate::kernel::Kernel::spawn`] drops to them between `fork` and `exec` —
//! so the program never executes one instruction as uid 0.
//!
//! Three properties are worth stating where they are implemented, because each
//! is a way this could have been built wrongly:
//!
//! * **The identity is a function of the name**, `rivet_core::service::app_identity`,
//!   so it is the same on the next boot and on every other machine. An
//!   application whose uid changed between boots could not read its own files,
//!   and the machine would report nothing — the `EACCES` happens inside the
//!   application, at a moment nobody is watching.
//! * **It is created with the application and never pre-created.** `ADR-008`
//!   §7.3 says so in those words, and it is enforced rather than remembered:
//!   the image's account list is a compile-time constant
//!   (`rivet_core::etc::SYSTEM_IDENTITIES`) and cannot hold a row for
//!   software the image has never seen. The account exists because a directory
//!   under `/state/app` exists, and stops existing when it does.
//! * **A collision is refused, not warned about.** Two names can derive one
//!   uid; two applications sharing one is not a weak sandbox but *no* sandbox
//!   between those two, since each could read and rewrite the other's data and
//!   each would answer the other's `SO_PEERCRED` as itself. So the second is
//!   NOT started. That is the opposite of what a colliding socket claim
//!   does, and the difference is that nothing binds a port yet
//!   while this identity is applied on the very next `execve`.
//!
//! **The human half of `ADR-008` §7.3 is deliberately not built here.** A
//! `users` section in the desired-state document and a `/etc/passwd`
//! projection of it are a different question with a different blocker — there
//! is no authorization layer for a human user to attach to — and building it
//! alongside this would have made one change out of two.
//!
//! # Why a fault here never stops the boot
//!
//! The same argument `ADR-018` §8 makes about `/state` and `crate::document`
//! reuses: **a machine that is reachable is one somebody can repair.** Every
//! failure below — an unreadable directory, a manifest that will not parse, a
//! name that is not a name — removes *that application* and leaves everything
//! else running, loudly. Escalating would trade a broken application for a
//! board nobody can log into, and the fault is on `/state`, which no rollback
//! and no update touches.

use rivet_core::package::application::{Manifest, Ready, Restart, Socket};
use rivet_core::service::{self, ServiceName};
use rivet_core::staging;

use crate::journal::{Journal, Severity, Value};
use crate::kernel::{Errno, Kernel, Program};
use crate::service::{Origin, Readiness, RestartPolicy, ServiceDef};
use crate::supervise::SELF_NAME;

/// Find every installed application and turn it into a definition.
///
/// Appended to `defs` rather than returned on their own, because the union
/// `ADR-017` §4.1 describes is over *all* definitions and the collision check
/// below has to see the image's services too.
///
/// **`volume` is a root directory** — `rivet_core::service::BUILT_IN_VOLUME`
/// for `/state` (`ADR-031` §8 item 1). It is a parameter rather than a constant
/// so that walking a second volume is a second call and not a rewrite; nothing
/// makes that call yet, because `ADR-031` §11 items 1, 2 and 8 are unruled —
/// whether a running machine can gain a volume, which process attaches one, and
/// where it is mounted.
///
/// **`ENOENT` still means "nothing installed" here, and `ADR-031` §6.1 says it
/// must stop meaning that** once a volume can be absent: an application root
/// missing because a disk did not turn up is a different fact from one missing
/// because nothing was ever installed, and reporting the second when the first
/// happened is `ADR-031`'s named trap. It is safe today only because the built-in
/// volume is always attached — the machine is running off it.
pub fn discover(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    volume: &str,
    defs: &mut Vec<ServiceDef>,
) {
    // **`PLAN-Feature-Set-2026-09-12` §6.2, `ADR-053`.** Read once per
    // discovery pass — this function already runs again after every install
    // (`crate::installed::Watch::look`), which is the same "re-runs
    // discovery" this primitive asks for and needed no new plumbing to get.
    let installed_libraries = crate::libraries::discover_installed(k, journal, volume);

    let root = service::app_root(volume);
    let root = root.as_str();

    let entries = match k.list_dir(root) {
        Ok(entries) => entries,
        Err(Errno::ENOENT) => {
            // **Not a fault, and the commonest case.** A machine with no
            // applications installed has no `/state/app`, and every machine
            // under QEMU has no `/state` at all (`ADR-026` §4). Said at `info`
            // rather than `warn`: nothing is wrong and nothing is degraded.
            note(
                k,
                journal,
                Severity::Info,
                "no applications are installed on this machine",
                vec![("path".to_string(), Value::Str(root.to_string()))],
            );
            return;
        }
        Err(e) => {
            note(
                k,
                journal,
                Severity::Error,
                "DEGRADED the application directory could not be read, so any installed \
                 application is NOT running and nothing here can say which. Carrying on rather \
                 than stopping: the fault is on /state, which no update or rollback touches \
                 (ADR-018 §8), and a machine that can be reached is one somebody can repair",
                vec![
                    ("path".to_string(), Value::Str(root.to_string())),
                    ("detail".to_string(), Value::Str(e.to_string())),
                ],
            );
            return;
        }
    };

    // **Sorted, so that "which of two colliding applications loses" is a
    // function of the two names and not of readdir order.** Without this a
    // machine could start `photo-index` on one boot and the application that
    // collides with it on the next, which is the same defect a changing uid
    // would be and is harder to see.
    let mut entries = entries;
    entries.sort_unstable();

    // **Every manifest is read before any is accepted**, so that the groups
    // below are known in full when the first application is judged. A shared
    // endpoint's group is a claim made by its PROVIDER's manifest, and an
    // application whose own gid lands on it must be refused whether its name
    // sorts before the provider's or after it.
    let mut candidates: Vec<ServiceDef> = Vec::new();
    for entry in entries {
        // **A removed application awaiting deletion** (`ADR-017` §3). It was
        // removed when it was renamed, so it is not here, and not a name to
        // complain about either.
        if service::is_set_aside(&entry) {
            continue;
        }
        // **An image service's directory, not an application.** A home
        // (`ADR-060`) holds `data`, `tmp`, a settings file and a routing
        // table; the compositor, which has no home, still keeps its screen
        // capture in `data`. Neither has a `current`, which is its ordinary
        // shape rather than a broken install, so it is passed over without a
        // word. One that DOES hold an installed manifest is read as usual and
        // refused by name below, like any application shadowing an image
        // service.
        if service::IMAGE_SERVICE_NAMES.contains(&entry.as_str())
            && let Some(name) = ServiceName::new(&entry)
            && k.file_exists(&service::manifest_path(volume, &name)) == Ok(false)
        {
            continue;
        }
        if let Some(def) = read_one(k, journal, volume, &entry, &installed_libraries) {
            candidates.push(def);
        }
    }

    apply_default_memory(k, journal, &mut candidates);

    // The endpoint groups claimed by every definition this pass can see: the
    // image's, those already accepted, and every candidate's, refused or not.
    // A refused provider still counts, so the outcome is a function of what
    // is installed and never of which refusal happened to run first.
    let mut groups: Vec<(u32, String)> = Vec::new();
    for def in defs.iter().chain(candidates.iter()) {
        for claim in endpoint_groups(def) {
            if !groups.iter().any(|(_, key)| *key == claim.1) {
                groups.push(claim);
            }
        }
    }

    for def in candidates {
        if let Some(at) = defs.iter().position(|d| d.name == def.name) {
            // **The same application, read again — not a collision.**
            //
            // Corrected 2026-09-01, `PLAN.md` §105.3. `crate::installed`
            // re-runs discovery after an install and passes in the
            // definitions the supervisor already holds, so on a
            // REINSTALL this arm sees the application's own previous
            // definition. Treating that as a name collision did two
            // wrong things at once: it wrote an ERROR about a name
            // collision that had not happened, and it **threw away the
            // fresh reading** — leaving the supervisor to go on
            // starting the program from the manifest of the payload
            // that had just been replaced.
            //
            // `crate::installed::reconsider` says it takes *"the
            // program, from the manifest that is on the card now"*.
            // This is what makes that sentence true; until today it was
            // handed the stale definition it had just passed in.
            //
            // `enabled` is the DOCUMENT's answer and never the
            // manifest's (`ADR-015` §3.2.2), so it is carried across
            // rather than reset — `crate::document::apply_desired_state`
            // runs after this and has the last word either way.
            if defs[at].origin == Origin::Application {
                let enabled = defs[at].enabled;
                defs[at] = ServiceDef { enabled, ..def };
                continue;
            }

            let existing = &defs[at];
            // An application named like a system service would shadow
            // it or be shadowed by it, and which of those happened
            // would depend on the order this ran. `ADR-011`: one
            // authority per fact, and here the fact is *what runs under
            // this name*.
            note(
                k,
                journal,
                Severity::Error,
                "an installed application has the same name as a service this machine \
                 already defines, so it is NOT started; one name cannot have two \
                 definitions (ADR-011)",
                vec![
                    ("svc_name".to_string(), Value::Str(def.name.clone())),
                    (
                        "defined_by".to_string(),
                        Value::Str(existing.origin.as_str().to_string()),
                    ),
                ],
            );
            continue;
        }
        if let Some(other) = colliding_identity(defs, &def) {
            // **Refused, not reported.** See this module's header: two
            // applications on one uid is no isolation at all between
            // them, and the identity is applied on the next `execve`
            // rather than being a declaration nothing honours.
            note(
                k,
                journal,
                Severity::Error,
                "REFUSED two installed applications derive the same uid, so this one is \
                 NOT started. One uid is one identity (ADR-011): both would read and \
                 rewrite each other's private data and each would answer the other's \
                 SO_PEERCRED as itself (ADR-015 §5.3), which is no isolation between \
                 them rather than a weak one. The remedy is to install one of them \
                 under a different name",
                vec![
                    ("svc_name".to_string(), Value::Str(def.name.clone())),
                    ("other".to_string(), Value::Str(other.0)),
                    ("uid".to_string(), Value::Int(i64::from(other.1))),
                ],
            );
            continue;
        }
        if let Some(clash) = colliding_group(defs, &groups, &def) {
            // **Refused for the uid check's reason, one level over.**
            // A shared endpoint's socket is `root:<group>` at `0660`,
            // so an application whose own gid is that group opens it
            // without declaring `connects`, and two endpoints on one
            // group open each other's. The derived numbers are on-disk
            // contracts and cannot be moved, so the ambiguity is
            // refused instead.
            note(
                k,
                journal,
                Severity::Error,
                "REFUSED this service would share the same gid with another identity or \
                 shared endpoint, so it is NOT started. A shared endpoint's socket is \
                 open to its group, so a colliding application could connect without \
                 declaring it and two colliding endpoints would each admit the other's \
                 consumers (ADR-015 §5). The remedy is to install one of them under a \
                 different name",
                vec![
                    ("svc_name".to_string(), Value::Str(def.name.clone())),
                    ("claims".to_string(), Value::Str(clash.mine)),
                    ("other".to_string(), Value::Str(clash.theirs)),
                    ("gid".to_string(), Value::Int(i64::from(clash.gid))),
                ],
            );
            continue;
        }
        // **~~`ADR-015` §6.2 point 3: one owner at a time.~~ The
        // refusal that stood here is GONE — ruled 2026-08-31.**
        //
        // It read every definition, found a second declarer of a device
        // class, and refused to start it. §6.2 says *"one owner at a
        // time | supervisor policy: refuse to start a second
        // declarer"*, and refusing at DISCOVERY is how that was read.
        //
        // **It was wrong here and the board proved it.** `enabled` does
        // not exist yet at discovery — an application starts
        // `enabled: false` and the document overrides it later — so the
        // check could not see it, and a service that was `enabled =
        // false` and idle still held the display and refused every
        // application installed after it. `PLAN.md` §104.17.3 has the
        // record: `held_by: kiosk-demo`, which was not running.
        //
        // The maintainer ruled the model rather than the bug: several kiosk
        // applications can be installed and enabled at once, and only one of
        // them is the auto launch (.rulings/ R-321).
        //
        // So **several declarers is the ordinary case** and there is
        // nothing to refuse here. §6.2's one owner is still enforced,
        // and enforced better: `crate::document::apply_auto_run` runs
        // after the document has spoken, keeps `enabled` on the one
        // `[kiosk] auto-run` names and clears it on the rest. Exactly
        // one is ever started, so exactly one is ever granted the node.
        //
        // **The uid and socket collisions above are untouched**, and
        // the difference is worth stating: those are two applications
        // that cannot BOTH be right whatever the operator wanted. Two
        // display declarers can — one of them is simply not the one on
        // screen today.
        // **The same shape for a SOCKET, and it now refuses.** Until
        // 2026-08-31 this was a warning after the fact, on a recorded
        // reason that has since stopped being true:
        //
        //   ~~"Both still start, because nothing opens these sockets
        //   yet and refusing would mean choosing which signed
        //   application does not run"~~
        //
        // Something opens them now — `ADR-015` §5.2's descriptor
        // handoff, built hours before this. So "both still start" is
        // no longer even the outcome: the second one's
        // `open_listeners` answers `EADDRINUSE` and the service fails
        // with an errno instead of a sentence.
        //
        // And the second clause is answered rather than merely
        // overtaken: `ADR-015` §6.2 rules that choosing which signed
        // application does not run is exactly the supervisor's job,
        // because it is the only thing holding every definition at
        // once.
        if let Some((socket, owner)) = defs.iter().find_map(|other| {
            other
                .listen
                .iter()
                .find(|s| def.listen.iter().any(|mine| claims_one_socket(mine, s)))
                .map(|s| (s, other.name.clone()))
        }) {
            note(
                k,
                journal,
                Severity::Error,
                "REFUSED another service already asks for this socket, so this one is \
                 NOT started. Two services claiming one socket is two authorities over \
                 one fact (ADR-011), and since ADR-015 §5.2's descriptor handoff was \
                 built the second one cannot bind it anyway — it would fail with \
                 EADDRINUSE and an errno rather than a reason. The remedy is the \
                 document: disable one of them",
                vec![
                    ("svc_name".to_string(), Value::Str(def.name.clone())),
                    ("other".to_string(), Value::Str(owner)),
                    ("socket".to_string(), Value::Str(socket.render())),
                ],
            );
            continue;
        }
        report(k, journal, &def);
        provide_data_dir(k, journal, &def);
        // **The last hop of `ADR-049`**, and it is here because this is
        // the first and only moment at which both halves exist: the
        // material the card carried, and the uid this pass has just
        // derived. It runs after `provide_data_dir` because it writes
        // into the directory that call creates.
        deliver_staged(k, journal, &def);
        provide_tmp_dir(k, journal, &def);
        defs.push(def);
    }
}

/// Where the machine's memory is read from, for the default limit.
pub const MEMINFO: &str = "/proc/meminfo";

/// **Give every application whose manifest asks for no memory limit the
/// default one** — half of this machine's memory,
/// `rivet_core::service::default_application_memory`.
///
/// The machine's memory is read once per discovery. A machine whose memory
/// cannot be read applies no default and says so, once, at `Warn`, naming the
/// applications that therefore run with no limit of their own — and only when
/// there is one, so a machine whose every application asks says nothing.
fn apply_default_memory(k: &mut dyn Kernel, journal: &mut Journal, defs: &mut [ServiceDef]) {
    let unasked: Vec<String> = defs
        .iter()
        .filter(|d| d.requested_memory.is_none())
        .map(|d| d.name.clone())
        .collect();
    if unasked.is_empty() {
        return;
    }
    let total = k
        .read_file(MEMINFO)
        .ok()
        .and_then(|text| rivet_core::service::meminfo_total_bytes(&text));
    let Some(default) = total.and_then(rivet_core::service::default_application_memory) else {
        note(
            k,
            journal,
            Severity::Warn,
            "this machine's memory could not be read, so the applications below, which ask for no \
             memory limit, get no default one and run with no limit but the operator's",
            vec![
                ("path".to_string(), Value::Str(MEMINFO.to_string())),
                ("applications".to_string(), Value::Str(unasked.join(", "))),
            ],
        );
        return;
    };
    for def in defs.iter_mut() {
        def.default_memory = Some(default);
        def.memory = rivet_core::service::effective_application_memory(
            def.requested_memory,
            def.default_memory,
            None,
        );
    }
}

/// **Remove an application's installed versions and keep its data** — the
/// `current` link by name, then every directory under `app_dir` except `data`.
///
/// Returns how many things were removed and what could not be. Shared by an
/// uninstall that keeps the data and by [`retire_carried_image_services`], so
/// the two cannot disagree about what "keep the data" leaves behind.
///
/// **`current` by name, and not by listing**: `Kernel::list_dir` answers with
/// directories only, and `current` is a symlink. `ENOENT` on it is not trouble —
/// an interrupted install may never have made it and an interrupted removal may
/// already have taken it; both re-run to the same place. Unlinking a link is
/// one syscall, so it is done here; every directory is **renamed aside** and
/// deleted by a helper (`ADR-017` §3, `crate::reclaim`), so the versions are
/// gone from the directory when this returns. `service` is who the helper
/// reports for.
pub(crate) fn remove_versions(
    k: &mut dyn Kernel,
    service: &str,
    app_dir: &str,
) -> (usize, Vec<String>) {
    let mut removed = 0usize;
    let mut trouble: Vec<String> = Vec::new();
    let link = format!("{app_dir}/{}", service::CURRENT_LINK);
    match k.remove_file(&link) {
        Ok(()) => removed += 1,
        // Not the installer's symlink but a real directory — what
        // `tools/qemu/run.sh --app` leaves. The listing below removes it as the
        // directory it is.
        Err(Errno::ENOENT | Errno::EISDIR) => {}
        Err(e) => trouble.push(format!("{}: {e}", service::CURRENT_LINK)),
    }
    let mut set_aside = false;
    for entry in k.list_dir(app_dir).unwrap_or_default() {
        // Already set aside — a version, or emptied scratch — and already a
        // helper's, or the next start's: it is not renamed twice.
        if entry == service::DATA_DIR || entry.contains(service::RECLAIM_MARK) {
            continue;
        }
        let path = format!("{app_dir}/{entry}");
        match crate::reclaim::rename_aside(k, &path, "") {
            Ok(_) => {
                removed += 1;
                set_aside = true;
            }
            Err(e) => trouble.push(format!("{entry}: {e}")),
        }
    }
    // One helper for the lot: it deletes every entry carrying the prefix.
    if set_aside && let Err(e) = k.reclaim_in_child(service, app_dir, &crate::reclaim::prefix("")) {
        trouble.push(format!(
            "the versions are removed, and the helper to delete what was set aside could not \
             start ({e}); the next start deletes it"
        ));
    }
    (removed, trouble)
}

/// **Retire an installed application whose name the image now defines** —
/// `ADR-060` §3.
///
/// On the first boot of an image that carries an image service with a home
/// ([`service::IMAGE_SERVICE_HOMES`]), a machine may still hold the application
/// that service used to be. Its `current` link and every version directory are
/// removed; its `data` directory stays, because the image service runs as the
/// same uid from the same home and reads what is already in it. Said once, at
/// `Info`, when something was removed — every later boot finds no `current` and
/// says nothing.
///
/// **Only when the image actually carries the program.** A card built without
/// it has no replacement, and deleting the only copy of a machine's front door
/// is never the right answer; the leftover is then refused by name by
/// discovery, as any application shadowing an image service is.
///
/// Runs before [`discover`], so discovery never sees the retired application
/// and no second definition claims the image service's name.
pub fn retire_carried_image_services(k: &mut dyn Kernel, journal: &mut Journal, volume: &str) {
    for home in service::IMAGE_SERVICE_HOMES {
        let Some(name) = ServiceName::new(home.name) else {
            continue;
        };
        if k.file_exists(home.program) != Ok(true) {
            continue;
        }
        let current = service::current_dir(volume, &name);
        // Nothing installed is the ordinary case, on every boot after the
        // first, and costs one stat. `file_exists` follows the link, so a
        // `current` pointing at nothing is not an installed application.
        if k.file_exists(&current) != Ok(true) {
            continue;
        }
        let app_dir = service::app_dir(volume, &name);
        let (removed, trouble) = remove_versions(k, name.as_str(), &app_dir);
        note(
            k,
            journal,
            if trouble.is_empty() {
                Severity::Info
            } else {
                Severity::Warn
            },
            if trouble.is_empty() {
                "an installed application of the same name as a service this image now provides \
                 was removed, and its data directory was kept for the image's service, which \
                 runs as the same uid from the same home (ADR-060)"
            } else {
                "an installed application of the same name as a service this image now provides \
                 was only PARTLY removed; what is left is named below, and discovery refuses it \
                 by name until it is gone (ADR-060)"
            },
            vec![
                ("svc_name".to_string(), Value::Str(name.to_string())),
                ("removed".to_string(), Value::Int(removed as i64)),
                (
                    "keeps".to_string(),
                    Value::Str(service::data_dir(volume, &name)),
                ),
                ("trouble".to_string(), Value::Str(trouble.join("; "))),
            ],
        );
    }
}

/// **Give every image service that runs under its own identity the home an
/// application has** — `ADR-060` §2.
///
/// For each definition from the image with an identity and a volume: the
/// directory under the application root, `0755` and root-owned as an installer
/// makes it, so the settings file and routing table can sit beside `data` where
/// the uid cannot rewrite them; then `data` and `tmp`, owned by the uid; then
/// any staged bundle (`ADR-049`) delivered into `data`. The same three calls
/// discovery makes for an application, so both kinds of home are made alike.
///
/// It converges, and every failure is reported and survived, for
/// [`provide_data_dir`]'s reason.
///
/// **Only for a service that is enabled.** One the document leaves off needs no
/// home, and making one anyway would report a failure on every boot of a
/// machine with no `/state` — the emulator's ordinary shape — about a service
/// nobody asked for. `crate::reload` calls this again, so a document that turns
/// one on at a reload gives it its home before it starts.
pub fn provide_image_homes(k: &mut dyn Kernel, journal: &mut Journal, defs: &[ServiceDef]) {
    'homes: for def in defs {
        if def.origin != Origin::Image || def.program.identity.is_none() || !def.enabled {
            continue;
        }
        let (Some(volume), Some(name)) = (def.volume.as_deref(), ServiceName::new(&def.name))
        else {
            continue;
        };
        for dir in [service::app_root(volume), service::app_dir(volume, &name)] {
            if let Err(e) = k.mkdir_mode(&dir, rivet_core::package::install::DIR_MODE) {
                note(
                    k,
                    journal,
                    Severity::Error,
                    "DEGRADED the home of an image service that runs under its own uid could \
                     not be created, so it starts with nowhere it may write and no settings \
                     file (ADR-060)",
                    vec![
                        ("svc_name".to_string(), Value::Str(def.name.clone())),
                        ("path".to_string(), Value::Str(dir)),
                        ("detail".to_string(), Value::Str(e.to_string())),
                    ],
                );
                continue 'homes;
            }
        }
        provide_data_dir(k, journal, def);
        deliver_staged(k, journal, def);
        provide_tmp_dir(k, journal, def);
    }
}

/// **Whether two declared sockets are one socket on this machine** — the same
/// unix name, or the same port on the same transport whatever each one's reach.
///
/// Reach does not separate them (`ADR-046` §7): one application's `tcp:8080`
/// on `127.0.0.1` and another's on every address are one port, and without
/// `SO_REUSEADDR` the second `bind` answers `EADDRINUSE`. Comparing the
/// manifest spellings would miss exactly that pair.
fn claims_one_socket(a: &Socket, b: &Socket) -> bool {
    match (a, b) {
        (Socket::Unix(x), Socket::Unix(y)) => x == y,
        _ => a.is_tcp() == b.is_tcp() && a.port().is_some() && a.port() == b.port(),
    }
}

/// The already-accepted definition whose identity this one would take, if any.
///
/// It compares **identities and not names** on purpose: two names that differ
/// can derive one uid, which is the case
/// `rivet_core::service::app_identity` documents and this is the only place
/// that can see. A system service has no identity, so it is never a candidate —
/// an image service runs as root and cannot collide with anything here.
fn colliding_identity(defs: &[ServiceDef], candidate: &ServiceDef) -> Option<(String, u32)> {
    let mine = candidate.program.identity?;
    defs.iter().find_map(|d| {
        (d.program.identity == Some(mine)).then(|| (d.name.clone(), mine.uid))
    })
}

/// The group of each SHARED endpoint a definition provides, named
/// `<provider>:<endpoint>`.
///
/// An unshared endpoint is granted by a chown to one consumer's uid and
/// carries no group of its own, so it claims nothing here.
fn endpoint_groups(def: &ServiceDef) -> Vec<(u32, String)> {
    def.provides
        .iter()
        .filter(|p| p.owner() == "socket" && p.shared())
        .map(|offered| {
            (
                service::endpoint_gid(&def.name, offered.name()),
                format!("{}:{}", def.name, offered.name()),
            )
        })
        .collect()
}

/// Every group a definition claims, each with the name a refusal reports it
/// by: its own gid when it runs under an identity, then [`endpoint_groups`].
fn claimed_groups(def: &ServiceDef) -> Vec<(u32, String)> {
    let mut claims = Vec::new();
    if let Some(id) = def.program.identity {
        claims.push((id.gid, def.name.clone()));
    }
    claims.extend(endpoint_groups(def));
    claims
}

/// Two claims on one gid, in the words a refusal reports them by.
struct GroupClash {
    mine: String,
    theirs: String,
    gid: u32,
}

/// The claim another identity or shared endpoint already makes on a gid this
/// definition would claim, if any.
///
/// `groups` is every shared endpoint group the discovery pass can see,
/// including the candidate's own and those of providers that are refused, so
/// the answer does not depend on the order definitions are judged in.
/// `defs` adds the identities already accepted, the image's among them: an
/// endpoint that lands on one of those is refused, because the identity
/// cannot be.
///
/// An application's gid and uid are one number (`rivet_core::service::app_identity`),
/// so a uid clash between two applications is [`colliding_identity`]'s and is
/// found before this runs.
fn colliding_group(
    defs: &[ServiceDef],
    groups: &[(u32, String)],
    candidate: &ServiceDef,
) -> Option<GroupClash> {
    for (gid, mine) in claimed_groups(candidate) {
        let endpoint = groups
            .iter()
            .find(|(g, key)| *g == gid && *key != mine)
            .map(|(_, key)| key.clone());
        let identity = defs
            .iter()
            .filter(|d| d.name != candidate.name)
            .find(|d| d.program.identity.is_some_and(|id| id.gid == gid))
            .map(|d| d.name.clone());
        if let Some(theirs) = endpoint.or(identity) {
            return Some(GroupClash { mine, theirs, gid });
        }
    }
    None
}

/// Make sure the application has somewhere to write, and that it owns it.
///
/// `ADR-015` §3.2's `state = "private"` promises an application *"`/state/app/<name>`,
/// mode 700"*. `rivet_core::service::data_dir` is where that lands — a sibling
/// of the versions rather than a child of `current`, so an update replaces the
/// program and leaves the data.
///
/// **This is the other half of the identity, and without it the first half
/// would be a regression.** An application that has been dropped to a
/// non-zero uid and has no directory it owns can write nothing at all: `/system`
/// is read-only `erofs` (`ADR-018` §3) and everything else on `/state` belongs
/// to root. A change that made applications unprivileged and left them with
/// nowhere to keep anything would read as sandboxing and be breakage.
///
/// **It converges.** `mkdir_mode` answers `Ok(false)` when the directory is
/// already there and the `chown` is idempotent, so the second boot does the
/// same work to the same end — which matters because this runs on **every**
/// discovery, and a first boot must not be a special case somebody has to
/// remember.
///
/// A failure removes nothing: the service still starts, because an application
/// that cannot write its data directory is a broken application and an
/// application that is not started is a broken machine, and only one of those
/// is visible to the operator. It is said at `Severity::Error` for that reason.
fn provide_data_dir(k: &mut dyn Kernel, journal: &mut Journal, def: &ServiceDef) {
    let Some(id) = def.program.identity else {
        return;
    };
    let Some(name) = ServiceName::new(&def.name) else {
        return;
    };
    // `def.volume` and not `BUILT_IN_VOLUME`: this runs over a definition the
    // caller built, and an application's data directory is on the volume the
    // application is on.
    let Some(volume) = def.volume.as_deref() else {
        return;
    };
    let path = service::data_dir(volume, &name);

    if let Err(e) = k.mkdir_mode(&path, service::DATA_DIR_MODE) {
        note(
            k,
            journal,
            Severity::Error,
            "DEGRADED an application's private data directory could not be created, so it \
             starts with nowhere it may write: /system is read-only (ADR-018 §3) and everything \
             else on /state belongs to root. ADR-015 §3.2's `state = \"private\"` is NOT honoured \
             for this application",
            vec![
                ("svc_name".to_string(), Value::Str(def.name.clone())),
                ("path".to_string(), Value::Str(path.clone())),
                ("detail".to_string(), Value::Str(e.to_string())),
            ],
        );
        return;
    }

    if let Err(e) = k.chown(&path, id.uid, id.gid) {
        note(
            k,
            journal,
            Severity::Error,
            "DEGRADED an application's private data directory exists and belongs to somebody \
             else, so the application cannot write it. It starts anyway; a service that is not \
             started is a fault an operator sees and one that cannot write is a fault only the \
             application sees, which is the worse of the two to hide",
            vec![
                ("svc_name".to_string(), Value::Str(def.name.clone())),
                ("path".to_string(), Value::Str(path)),
                ("uid".to_string(), Value::Int(i64::from(id.uid))),
                ("detail".to_string(), Value::Str(e.to_string())),
            ],
        );
    }
}

/// **Hand this application the material the card carried for it** — `ADR-049`.
///
/// # Why this hop exists at all
///
/// `cmd/rivet-image` puts a staging bundle on the boot partition and PID 1
/// moves it to `/state/staging`, exactly as a device identity travels. Neither
/// of those can finish the journey: an application's data directory is mode
/// `0700` owned by that application's uid, and the uid is
/// `rivet_core::service::app_identity`'s — derived from the name, here, in the
/// pass immediately above. It is not knowable when the card is written and not
/// knowable by PID 1.
///
/// **This is not theoretical.** On 2026-09-07 a fleet server's register and
/// credentials were placed by hand as root and `rivet-fleetd` could not read
/// one file of them. It logged nothing either time it was restarted, because
/// `rivetos_register::store::load` reports an absent register and an unreadable
/// one identically — correct for a first run and silent for this. Five
/// `apply set-owner` calls fixed it, and this function is those five calls
/// happening before the application has ever started.
///
/// # Why it is safe to run on every discovery pass
///
/// Delivery happens once and then the bundle is removed, so the ordinary boot
/// finds nothing and does nothing. Until that removal it converges: a file that
/// is already in the data directory is **kept, not overwritten**, because by
/// the time a second pass runs the application may have rewritten it — a fleet
/// server writes its own register — and a card-time copy overwriting a live one
/// would silently roll a fleet back to the day the card was made.
///
/// **The owner is set even for a file that was kept.** That is the repair path
/// for the one failure this cannot otherwise recover from: a partial delivery
/// that wrote a file and then failed before its `chown`, leaving root-owned
/// bytes in a directory the application cannot read and nothing saying so.
///
/// # Why a failure leaves the bundle where it is
///
/// The card's copy is gone by now — PID 1 destroys it, because it holds a
/// private key. `/state/staging` is therefore the only copy, and removing it
/// after a failed delivery would close the last window without an operator ever
/// seeing one. So a failure is reported at `Severity::Error`, the bundle stays,
/// and the next boot or the next install tries again.
///
/// The service still starts, for [`provide_data_dir`]'s reason exactly: an
/// application that cannot read its credentials fails in a way an operator can
/// see, and one that is never started fails in a way that looks like this
/// supervisor's fault.
///
/// # Why every path is checked for a symlink before it is used
///
/// `data` is `0700` and owned by the application ([`provide_data_dir`] chowns
/// it before this runs), so the application can replace any entry inside it
/// with a link the moment this delivery is not actually running — and a
/// bundle can stay pending for an application that has already started once,
/// because delivery converges rather than running only at install.
/// [`Kernel::mkdir_mode`] treats an existing directory symlink as "already
/// there" (it stats rather than `lstat`s), [`Kernel::file_exists`] follows a
/// link the same way, and a plain [`Kernel::write_file`] would create or
/// truncate whatever the link points at. So every parent component and the
/// item's own path is checked with [`Kernel::is_symlink`] — which never
/// follows — before it is handed to any of those, and the file itself is
/// created with [`Kernel::write_file_nofollow`], which refuses at the
/// syscall rather than at this check alone.
fn deliver_staged(k: &mut dyn Kernel, journal: &mut Journal, def: &ServiceDef) {
    let Some(id) = def.program.identity else {
        return;
    };
    let Some(name) = ServiceName::new(&def.name) else {
        return;
    };
    let Some(volume) = def.volume.as_deref() else {
        return;
    };

    let at = staging::staged_bundle(&def.name);
    let raw = match k.read_file_bytes(&at, staging::MAX_BUNDLE_BYTES) {
        Ok(bytes) => bytes,
        // **The ordinary case on every machine and every boot.** Nothing is
        // staged for most applications and nothing is staged for any of them
        // after the first delivery, so this is silent rather than `Info`: a
        // line here would appear on every discovery pass on every machine.
        Err(Errno::ENOENT) => return,
        Err(e) => {
            note(
                k,
                journal,
                Severity::Error,
                "DEGRADED this application has material waiting from the card and it could not be \
                 read, so the application starts WITHOUT the credentials it was provisioned with. \
                 The file is still there and the next boot will try again",
                vec![
                    ("svc_name".to_string(), Value::Str(def.name.clone())),
                    ("path".to_string(), Value::Str(at)),
                    ("detail".to_string(), Value::Str(e.to_string())),
                ],
            );
            return;
        }
    };

    let items = match core::str::from_utf8(&raw).map_err(|e| e.to_string()).and_then(
        |text| staging::parse(text).map_err(|e| e.to_string()),
    ) {
        Ok(items) => items,
        Err(detail) => {
            note(
                k,
                journal,
                Severity::Error,
                "DEGRADED this application's staged material will not read, so NONE of it has \
                 been delivered and the application starts without it. The file is left in place \
                 as the evidence; it was written by rivet-provision and checked by rivet-image \
                 before the card was built, so a bundle that will not read here means the card or \
                 /state is failing",
                vec![
                    ("svc_name".to_string(), Value::Str(def.name.clone())),
                    ("path".to_string(), Value::Str(at)),
                    ("detail".to_string(), Value::Str(detail)),
                ],
            );
            return;
        }
    };

    let data = service::data_dir(volume, &name);
    let mut delivered = 0usize;
    let mut kept = 0usize;

    for item in &items {
        // **A bundle addressed to somebody else is not delivered here.** PID 1
        // splits by application, so this cannot happen on a card this project
        // wrote — and if it ever does, writing another application's credential
        // into this one's directory is the failure worth refusing outright.
        if item.application != def.name {
            note(
                k,
                journal,
                Severity::Error,
                "REFUSED this application's staged bundle contains a file addressed to a \
                 DIFFERENT application, so nothing in it has been delivered. One bundle is one \
                 application's material (ADR-011); delivering it here would put somebody else's \
                 private key in this application's directory",
                vec![
                    ("svc_name".to_string(), Value::Str(def.name.clone())),
                    ("other".to_string(), Value::Str(item.application.clone())),
                    ("path".to_string(), Value::Str(at)),
                ],
            );
            return;
        }

        for parent in staging::parents(&item.path) {
            let dir = format!("{data}/{parent}");
            if !refuse_planted_symlink(k, journal, def, &at, &dir) {
                return;
            }
            if let Err(e) = k.mkdir_mode(&dir, service::DATA_DIR_MODE) {
                degraded(k, journal, def, &at, &dir, &e.to_string());
                return;
            }
            if let Err(e) = k.chown(&dir, id.uid, id.gid) {
                degraded(k, journal, def, &at, &dir, &e.to_string());
                return;
            }
        }

        let path = format!("{data}/{}", item.path);
        if !refuse_planted_symlink(k, journal, def, &at, &path) {
            return;
        }
        let present = k.file_exists(&path).unwrap_or(false);
        if present {
            kept = kept.saturating_add(1);
        } else if let Err(e) = k.write_file_nofollow(&path, &item.bytes, staging::FILE_MODE) {
            degraded(k, journal, def, &at, &path, &e.to_string());
            return;
        } else {
            delivered = delivered.saturating_add(1);
        }

        // **Always, present or not.** See this function's documentation: a file
        // written by an earlier failed pass is root-owned, and root-owned bytes
        // in a `0700` directory are the silent failure this whole mechanism
        // exists to end.
        if let Err(e) = k.chown(&path, id.uid, id.gid) {
            degraded(k, journal, def, &at, &path, &e.to_string());
            return;
        }
    }

    // **Only now.** Everything is in place and owned; the bundle is the last
    // copy of a private key that PID 1 has already destroyed on the card.
    if let Err(e) = k.remove_file(&at) {
        note(
            k,
            journal,
            Severity::Error,
            "this application's staged material was delivered but the staging copy could not be \
             removed, so a private key is still lying in /state where root can read it. Delivery \
             converges, so the next boot will not deliver it twice",
            vec![
                ("svc_name".to_string(), Value::Str(def.name.clone())),
                ("path".to_string(), Value::Str(at.clone())),
                ("detail".to_string(), Value::Str(e.to_string())),
            ],
        );
    }

    note(
        k,
        journal,
        Severity::Info,
        "this application was handed the material the card carried for it, into the directory it \
         owns and with its own uid, BEFORE it started (ADR-049). This is what a fleet server's \
         certificates, operator credential and register arrive by; until 2026-09-07 every one of \
         them was installed by hand after the machine had booted",
        vec![
            ("svc_name".to_string(), Value::Str(def.name.clone())),
            ("delivered".to_string(), Value::Int(delivered as i64)),
            ("kept".to_string(), Value::Int(kept as i64)),
            ("uid".to_string(), Value::Int(i64::from(id.uid))),
            ("path".to_string(), Value::Str(data)),
        ],
    );
}

/// One sentence for every way [`deliver_staged`] can fail part way.
fn degraded(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    def: &ServiceDef,
    bundle: &str,
    path: &str,
    detail: &str,
) {
    note(
        k,
        journal,
        Severity::Error,
        "DEGRADED this application's staged material was only PARTLY delivered, so it may start \
         holding some of its credentials and not others. The staging bundle is LEFT in place and \
         delivery converges, so the next boot completes it — a file already in the directory is \
         kept and its owner is set again",
        vec![
            ("svc_name".to_string(), Value::Str(def.name.clone())),
            ("path".to_string(), Value::Str(path.to_string())),
            ("bundle".to_string(), Value::Str(bundle.to_string())),
            ("detail".to_string(), Value::Str(detail.to_string())),
        ],
    );
}

/// **The one gate every path [`deliver_staged`] is about to use passes
/// through.** `true` means `path` is safe to hand to [`Kernel::mkdir_mode`],
/// [`Kernel::file_exists`] or a write; `false` means a refusal has already
/// been journaled through [`degraded`] and the caller must stop rather than
/// use `path` at all.
///
/// See [`deliver_staged`]'s own documentation for why this check exists: the
/// directory it writes into is one the application owns, so a link an
/// application planted there has to be caught before it is used, not after.
fn refuse_planted_symlink(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    def: &ServiceDef,
    bundle: &str,
    path: &str,
) -> bool {
    match k.is_symlink(path) {
        Ok(false) => true,
        Ok(true) => {
            degraded(
                k,
                journal,
                def,
                bundle,
                path,
                "a symbolic link is in the way, so it was left alone and nothing was delivered \
                 through it",
            );
            false
        }
        Err(e) => {
            degraded(k, journal, def, bundle, path, &e.to_string());
            false
        }
    }
}

/// Make the scratch directory `RIVET_APP_TMP` and `TMPDIR` name.
///
/// **Ruled 2026-09-07**, after a restore on the test board died with
/// `SQLITE_CANTOPEN`: an application was confined to one writable directory and
/// every convention for *somewhere to put a scratch file* pointed at a path the
/// ruleset refuses. `rivet_core::service::tmp_dir` is the whole account,
/// including why the answer is a sibling of `data` rather than a corner of it.
///
/// **Creation is here; EMPTYING is not.** This runs on every discovery pass,
/// and a discovery pass happens while applications are running — deleting the
/// contents here would take a live process's scratch file out from under it.
/// `crate::supervise` empties it at the two moments the supervisor knows
/// nothing is using it: immediately before a spawn and immediately after an
/// exit.
///
/// It converges and it degrades the same way [`provide_data_dir`] does, for the
/// same reason: an application that cannot write its scratch directory is a
/// broken application, and one that is never started is a broken machine.
fn provide_tmp_dir(k: &mut dyn Kernel, journal: &mut Journal, def: &ServiceDef) {
    let Some(id) = def.program.identity else {
        return;
    };
    let Some(name) = ServiceName::new(&def.name) else {
        return;
    };
    let Some(volume) = def.volume.as_deref() else {
        return;
    };
    let path = service::tmp_dir(volume, &name);

    if let Err(e) = k.mkdir_mode(&path, service::TMP_DIR_MODE) {
        note(
            k,
            journal,
            Severity::Error,
            "DEGRADED an application's scratch directory could not be created, so TMPDIR names \
             a path that is not there. A library reaching for a temporary file will fail with \
             something that names no path — SQLITE_CANTOPEN is the one this was found by",
            vec![
                ("svc_name".to_string(), Value::Str(def.name.clone())),
                ("path".to_string(), Value::Str(path.clone())),
                ("detail".to_string(), Value::Str(e.to_string())),
            ],
        );
        return;
    }

    if let Err(e) = k.chown(&path, id.uid, id.gid) {
        note(
            k,
            journal,
            Severity::Error,
            "DEGRADED an application's scratch directory exists and belongs to somebody else, \
             so TMPDIR names a directory the application cannot write. It starts anyway, for \
             the reason the data directory does",
            vec![
                ("svc_name".to_string(), Value::Str(def.name.clone())),
                ("path".to_string(), Value::Str(path)),
                ("uid".to_string(), Value::Int(i64::from(id.uid))),
                ("detail".to_string(), Value::Str(e.to_string())),
            ],
        );
    }
}

/// One directory under a volume's application root, or nothing plus a reason.
fn read_one(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    volume: &str,
    entry: &str,
    installed_libraries: &[rivet_core::service::InstalledLibrary],
) -> Option<ServiceDef> {
    let Some(name) = ServiceName::new(entry) else {
        note(
            k,
            journal,
            Severity::Warn,
            "a directory under the application root is not a usable service name, so nothing is \
             started for it; the name is a cgroup and a socket as well as a directory \
             (ADR-017 §3.1.3)",
            vec![
                // The *rule*, never the name: a directory name on `/state` is
                // not a secret, but it is input, and every other refusal in
                // this workspace reports a rule rather than an input
                // (`ADR-020` §4). The path below names the parent, which is
                // where an operator has to look anyway.
                ("path".to_string(), Value::Str(service::app_root(volume))),
                (
                    "rule".to_string(),
                    Value::Str(ServiceName::RULE.to_string()),
                ),
            ],
        );
        return None;
    };

    let path = service::manifest_path(volume, &name);
    let text = match k.read_file(&path) {
        Ok(text) => text,
        Err(e) => {
            note(
                k,
                journal,
                Severity::Warn,
                "an installed application has no readable manifest, so what it IS cannot be \
                 known and it is NOT started (ADR-015 §3.2)",
                vec![
                    ("svc_name".to_string(), Value::Str(name.to_string())),
                    ("path".to_string(), Value::Str(path.clone())),
                    ("detail".to_string(), Value::Str(e.to_string())),
                ],
            );
            return None;
        }
    };

    let manifest = match Manifest::parse(text.as_bytes()) {
        Ok(manifest) => manifest,
        Err(fault) => {
            note(
                k,
                journal,
                Severity::Warn,
                "an installed application's manifest could not be read, so it is NOT started. \
                 Refusing rather than guessing: a manifest is what says how this program is \
                 restarted and what it may have, and a default would be a policy nobody wrote \
                 (AGENTS.md §5)",
                vec![
                    ("svc_name".to_string(), Value::Str(name.to_string())),
                    ("path".to_string(), Value::Str(path.clone())),
                    // The fault names a key, never a value — see
                    // `ManifestFault`'s own documentation.
                    ("detail".to_string(), Value::Str(fault.to_string())),
                ],
            );
            return None;
        }
    };

    Some(definition(volume, &name, &manifest, installed_libraries))
}

/// The manifest, as a definition. **This function is the ruling.**
///
/// Every field it fills comes from the artifact; the only one it invents is
/// `enabled`, which the document then owns (see this module's header).
///
/// `installed_libraries` is `PLAN-Feature-Set-2026-09-12` §6.2's addon
/// primitive: every library this volume has installed, with the class its
/// own manifest provides, if any (`crate::libraries::discover_installed`). An
/// empty slice is a legitimate answer — it is what every call site outside
/// [`discover`] passes, because none of them exercises an addon join and
/// `rivet_core::service::addon_library_path` returns `None` for it exactly as
/// it would for a real, empty listing.
pub fn definition(
    volume: &str,
    name: &ServiceName,
    manifest: &Manifest,
    installed_libraries: &[rivet_core::service::InstalledLibrary],
) -> ServiceDef {
    let request = service::requested_memory(manifest);
    // `request`'s footing, for CPU — `ADR-007` §7 and `ADR-015` §3's
    // 2026-09-09 amendment gave `cpu` the identical split.
    let cpu_request = service::requested_cpu(manifest);
    ServiceDef {
        name: name.to_string(),
        program: Program {
            // **Derived, never declared.** `ADR-015` §3.2: the artifact is a
            // Cargo package, so the binary IS the crate's binary, and a
            // manifest that could name a program would let a signed artifact
            // launch something the signature never covered.
            path: service::program_path(volume, name),
            // No arguments for the same reason. A manifest with an `args` key
            // would be an `exec` key wearing a hat.
            args: Vec::new(),
            // **The environment is not empty since 2026-08-31, and exactly one
            // variable is in it.** Shared Rivets.
            //
            // `LD_LIBRARY_PATH`, composed from the manifest's declared closure
            // and from nothing else — see [`service::library_path`].
            //
            // **Why this is not the `args` argument in disguise.** The rule
            // above is that a manifest may not name a *program*, because the
            // signature covers the bytes of one binary and naming another would
            // escape it. A closure entry cannot do that: it is a
            // content-addressed reference, so the only thing a manifest can
            // point at is a directory whose name is a digest of an artifact this
            // machine verified and installed. An author who writes a reference
            // for bytes the machine does not hold gets a refusal at install, not
            // a different program.
            // **Two variables since 2026-08-31.** `LD_LIBRARY_PATH` as above,
            // and `RIVET_APP_DIR`, which says where this application's own
            // installed files are — see `service::APP_DIR_ENV` for why that is
            // a thing the machine composes and not a thing a manifest sets.
            env: {
                let mut env = Vec::new();
                // **The explicit closure, then the addon join** —
                // `PLAN-Feature-Set-2026-09-12` §6.2, `ADR-053`. Explicit
                // first, so a program that declares BOTH an exact reference
                // and an accepted class finds the one it named by digest
                // ahead of whatever an addon's class happened to bring —
                // `ADR-011`'s "one authority" applied to the order two
                // sources are joined in rather than to which one wins,
                // since the loader itself takes the first SONAME match.
                let mut ld_path = Vec::new();
                if let Some(value) = rivet_core::service::library_path(volume, manifest) {
                    ld_path.push(value);
                }
                if let Some(value) = rivet_core::service::addon_library_path(
                    volume,
                    manifest,
                    installed_libraries,
                ) {
                    ld_path.push(value);
                }
                if !ld_path.is_empty() {
                    env.push(("LD_LIBRARY_PATH".to_string(), ld_path.join(":")));
                }
                // **And what those class-matched directories actually
                // contain** — `PLAN-Addon-Feature-Set-2026-09-19` T2. The line
                // above says where the loader may look; this one says which
                // objects are there and what class each one provides, because
                // a host that can only derive `lib<class>.so.1` cannot be
                // joined to two libraries of one class, and `.rulings/` R-199
                // D1 makes a second background pack the case to support.
                //
                // Composed from the same list, through
                // `rivet_core::service::addon_matches`, so a directory is
                // named here exactly when it is searched above and exactly
                // when it is granted below. `service::ADDONS_ENV` has the
                // format and why an absent variable rather than an empty one.
                if let Some(value) =
                    rivet_core::service::addon_env(volume, manifest, installed_libraries)
                {
                    env.push((rivet_core::service::ADDONS_ENV.to_string(), value));
                }
                env.push((
                    rivet_core::service::APP_DIR_ENV.to_string(),
                    rivet_core::service::current_dir(volume, name),
                ));
                // Where it may WRITE, which is a different directory and a
                // different promise — see `service::APP_DATA_ENV`.
                env.push((
                    rivet_core::service::APP_DATA_ENV.to_string(),
                    rivet_core::service::data_dir(volume, name),
                ));
                // **Four variables since 2026-09-04**, and the fourth exists
                // because the settings file left `data/`. It used to be inside
                // the directory above, so an application could compose the path;
                // it is now beside `current`, root-owned, in a directory nothing
                // hands an application — `rivet_core::settings::path` has the
                // argument. Told rather than derived, the SDK's first article (`ADR-036` §2).
                env.push((
                    rivet_core::service::APP_SETTINGS_ENV.to_string(),
                    rivet_core::settings::path(volume, name),
                ));
                // **Six variables since 2026-09-07 — seven for an application
                // an addon joined, since 2026-09-19** — and two of them are one
                // value under two names. `RIVET_APP_TMP` is this project's, on
                // the footing of the three above; `TMPDIR` is the one every
                // library already reads, and setting it is what makes a crate
                // that was never written for Rivet find a writable scratch
                // path instead of `SQLITE_CANTOPEN`. `service::APP_TMP_ENV`
                // has the argument for why both.
                let scratch = rivet_core::service::tmp_dir(volume, name);
                env.push((
                    rivet_core::service::APP_TMP_ENV.to_string(),
                    scratch.clone(),
                ));
                env.push((rivet_core::service::TMPDIR_ENV.to_string(), scratch));
                env
            },
            // **Derived from the name too**, and by the same argument: an
            // identity a manifest could name would let a signed artifact ask
            // to be somebody else. `ADR-015` §5.1, ruled 2026-08-24. This is
            // the field that stops the program being run as root.
            identity: Some(service::app_identity(name)),
            // **What it may reach, derived from the same manifest** —
            // `ADR-042`, ruled 2026-09-04. `rivet_core::confinement::policy`
            // reads the declared closure and composes the two directories the
            // machine already tells the application about, so the confinement
            // and `LD_LIBRARY_PATH`, `RIVET_APP_DIR` and `RIVET_APP_DATA` above
            // come from one set of functions rather than two lists that agree
            // today (`ADR-011`).
            //
            // The device nodes a `devices` class expands to are added in
            // `crate::supervise`, where the glob against the machine already
            // happens for the chown.
            // The pid is this process's, because this process is the
            // supervisor — `ADR-056` §3, and `rivet_core::confinement`'s
            // `SUPERVISOR_WINDOWS` are the only names it changes anything for.
            //
            // **`installed_libraries` is passed for the same reason it is
            // passed to `addon_library_path` above, and 2026-09-19 is when
            // leaving it out stopped being theoretical**: a class-matched
            // addon reached `LD_LIBRARY_PATH` and no Landlock rule, so the
            // loader searched a directory the kernel refused. One argument,
            // two composers, no disagreement.
            confinement: Some(rivet_core::confinement::policy(
                volume,
                name,
                manifest,
                Some(std::process::id()),
                installed_libraries,
            )),
            groups: Vec::new(),
            capabilities: Vec::new(),
        },
        // **`false`, ruled by the maintainer 2026-08-25.** ~~`true`~~ was the
        // starting value from the day applications existed until then, and it
        // was never a decision — it was a value with a one-line comment, and
        // the comment said only that the document would replace it.
        //
        // **Installing is not running.** `crate::document::apply_desired_state`
        // replaces this wherever the document names the service, so `true` here
        // meant *an application nobody had mentioned anywhere ran on the next
        // convergence*. `ADR-015` §3.2.1 had already ruled the principle — "an
        // application that could declare itself enabled would be one an
        // operator could not turn off" — and a default of `true` is that
        // outcome by another route: silence read as consent.
        //
        // **The image's own services still start enabled, and that is not an
        // inconsistency** (`crate::definitions`). `rivet-mcpd` and `rivet-netd`
        // have a ruled reason that does not generalise: `ADR-023` §4.1 says a
        // freshly flashed headless board must be reachable, and you cannot
        // reach a machine to enable the thing you reach it with. An application
        // has no such bootstrap problem — the operator is already on the
        // machine, or they could not have installed it.
        //
        // **What it costs**, so it is not discovered: an application installed
        // before this change stops at the next boot of a machine whose document
        // does not name it. `PLAN.md` §20 carries the migration.
        enabled: false,
        // **`false`, for `enabled`'s reason and one more** — `ADR-045` §4.
        // The document owns this key and a manifest has no counterpart to
        // inherit from, because an application that could declare itself
        // critical would be setting its own severity. So an application arrives
        // here unimportant and the operator says otherwise, on their machine.
        critical: false,
        restart: match manifest.restart() {
            Restart::Always => RestartPolicy::Always,
            Restart::OnFailure => RestartPolicy::OnFailure,
            Restart::Never => RestartPolicy::Never,
        },
        readiness: match manifest.ready() {
            Ready::None => Readiness::OnStart,
            Ready::Notify => Readiness::Notify,
            // Not `OnStart`. See `Readiness::ListeningUnsupported` — saying
            // "this program has no readiness signal" about one that declared
            // `listening` would be the machine agreeing with a manifest it
            // cannot honour.
            Ready::Listening => Readiness::ListeningUnsupported,
        },
        // **Empty, and it is not a placeholder.** `ADR-017` §3.1.6's device —
        // what the machine loses while this is not running — needs somebody to
        // have written the sentence, and an application's author has nowhere to
        // write it: `ADR-015` §3.2's manifest has no such key. Inventing
        // "the application <name> is not running" would be a sentence that adds
        // nothing to the service name already on the record, which is exactly
        // what §3.1.6 says a consequence must not be.
        consequence: String::new(),
        origin: Origin::Application,
        // Recorded, so that every later path this application needs — its data
        // directory, its program at launch, its removal — comes from the volume
        // it was actually found on rather than from a constant somebody has to
        // remember to change. `ADR-031` §8 item 1.
        volume: Some(volume.to_string()),
        // **`provides` is folded into `listen` here**, so the socket is bound
        // and handed over by the one path that already does that. `ADR-015`
        // §5.3's socket IS §5.2's socket; what distinguishes it is that its
        // file may be granted to a consumer, which `crate::endpoints` decides
        // and `crate::supervise` applies.
        listen: manifest
            .listen()
            .iter()
            .cloned()
            .chain(manifest.provides().iter().filter_map(|e| {
                (e.owner() == "socket")
                    .then(|| rivet_core::package::application::Socket::Unix(e.name().to_string()))
            }))
            .collect(),
        // `ADR-046` §7: a proxied application's ports bind loopback. From the
        // signed manifest, like `listen` beside it.
        proxied: manifest.proxied(),
        provides: manifest.provides().to_vec(),
        connects: manifest.connects().to_vec(),
        // `ADR-015` §6.2. Straight from the manifest, like `listen`: it is
        // inside the signature, so it is a fact the author asserted rather
        // than anything the machine decides.
        devices: manifest.devices().to_vec(),
        // `ADR-055` §3. Straight from the manifest, like `devices` above —
        // `crate::supervise::start` is where it is checked, at every launch.
        material: manifest.material().to_vec(),
        requested_memory: request,
        // Set by `discover`, which reads the machine's memory once; a
        // definition built without it runs under the manifest's request alone.
        default_memory: None,
        // The manifest's half of the merge. The operator's cap arrives in
        // `crate::document::apply_desired_state`, and **the lower wins**.
        memory: rivet_core::service::effective_memory(request, None),
        requested_cpu: cpu_request,
        // `memory`'s footing exactly, for CPU.
        cpu: rivet_core::service::effective_cpu(cpu_request, None),
    }
}

/// Say what was found, and what is not being enforced about it.
///
/// `ADR-015` §6.1: *"an unenforced declaration is reported, not silently
/// accepted"*. The list of what is unenforced is **the manifest's own**
/// (`Manifest::unenforced`) wherever the artifact can produce it, so this
/// module cannot fall behind the artifact as enforcement arrives.
fn report(k: &mut dyn Kernel, journal: &mut Journal, def: &ServiceDef) {
    note(
        k,
        journal,
        Severity::Info,
        "an installed application defines a service; what it IS comes from its signed manifest \
         and only `enabled` and a memory or CPU cap are the operator's (ADR-017 §4, ruled \
         2026-08-24; ADR-007 §7 and ADR-015 §3's 2026-09-09 amendment gave cpu the same \
         split)",
        vec![
            ("svc_name".to_string(), Value::Str(def.name.clone())),
            ("program".to_string(), Value::Str(def.program.path.clone())),
            (
                "restart".to_string(),
                Value::Str(def.restart.as_str().to_string()),
            ),
            (
                "memory_request".to_string(),
                Value::Str(
                    def.requested_memory
                        .map_or_else(|| "none".to_string(), |m| m.to_string()),
                ),
            ),
            (
                "cpu_request".to_string(),
                Value::Str(
                    def.requested_cpu
                        .map_or_else(|| "none".to_string(), |c| c.to_string()),
                ),
            ),
            // The uid is a *grant*, in `ADR-015` §5.4's own vocabulary, so it
            // belongs on the line that says what this application is rather
            // than among the NOT ENFORCED lines below. It is also the number an
            // operator needs in order to read `ls -l` on anything the
            // application wrote.
            (
                "uid".to_string(),
                Value::Str(
                    def.program
                        .identity
                        .map_or_else(|| "0".to_string(), |i| i.uid.to_string()),
                ),
            ),
        ],
    );

    // Not a NOT ENFORCED line — the opposite. Said separately from the line
    // above because it is the one thing about this change an operator should be
    // able to grep the journal for, and because a uid on a line about restart
    // policies is a number nobody reads.
    if let Some(id) = def.program.identity {
        note(
            k,
            journal,
            Severity::Info,
            "ENFORCED this application runs under an identity of its own and never as root: the \
             supervisor drops the supplementary groups, then the gid, then the uid, between fork \
             and exec, and asks the kernel who the child is before exec'ing (ADR-015 §5.1, ruled \
             2026-08-24). The identity is derived from the name, so it is the same on the next \
             boot",
            vec![
                ("svc_name".to_string(), Value::Str(def.name.clone())),
                ("uid".to_string(), Value::Int(i64::from(id.uid))),
                ("gid".to_string(), Value::Int(i64::from(id.gid))),
            ],
        );
    }

    // The rest of `ADR-015` §5.1's table, which is not built. Said in one line
    // rather than four, because an operator who reads "runs as uid 41234" will
    // otherwise reasonably conclude the sandbox exists.
    //
    // ~~"...no Landlock, which the stock kernel does not have (ADR-024 §8);
    // and no seccomp filter. Unix permissions on its own data directory are
    // the whole of what separates it from another application"~~
    //
    // **Struck 2026-09-04, and it had been false for five days.** `ADR-024`
    // §8's conditional fired, Rivet's own kernel shipped on 2026-08-31 with
    // `CONFIG_SECURITY_LANDLOCK=y`, and the board printed this sentence on
    // every boot afterwards while its own kernel log said
    // `LSM: initializing lsm=capability,landlock` and `landlock: Up and
    // running.` two seconds later. That is `AGENTS.md` §7 in the shape that
    // costs the most: a claim of absence, in the machine's own voice, about a
    // mechanism the machine had. `ADR-042` is the ruleset; the line below is
    // what is left of the list.
    note(
        k,
        journal,
        Severity::Warn,
        "NOT ENFORCED an identity and a Landlock ruleset are what ADR-015 §5.1 this build \
         applies; two rows of that table are still missing. There is no mount namespace, so \
         every path is still THERE and anything running as uid 0 reaches it — what an \
         application may open is restricted, what exists is not hidden; and no network \
         namespace, so `network = \"none\"` and `network = \"group:...\"` mean nothing. There is \
         no seccomp filter",
        vec![("svc_name".to_string(), Value::Str(def.name.clone()))],
    );

    // ~~The one that is not a sandbox declaration, and the most serious: an
    // application's binary sits in plain ext4 on `/state` and nothing checks it
    // before executing it.~~
    //
    // **It stopped being true on 2026-08-25** and the record went on saying it
    // until it was read off a board on the same day (`PLAN.md` §19.5).
    // `crate::verify::program` runs immediately before every application spawn
    // — `crate::supervise`, and `PLAN.md` §15 is the ruling — so a machine that
    // reports "this build verifies nothing" is telling an operator their
    // applications are unchecked when they are checked. That is worse than the
    // absence of the message: it is a reason to go and build something that
    // exists.
    //
    // Replaced rather than deleted, because the honest statement is still a
    // qualified one and the qualification is `ADR-015` §4.2's own.
    note(
        k,
        journal,
        Severity::Info,
        "ENFORCED this application's signed header is verified against the machine's trust \
         store before every launch, and the program on /state is compared with the digest that \
         header covers (ADR-015 §4.2, ruled and built 2026-08-25). A key removed from the trust \
         store stops the applications it signed at their next launch, which is what makes \
         revocation mean anything (ADR-028 §5.5.4). It is verified at LAUNCH and not while \
         running: nothing re-reads the bytes of a process that is already up",
        vec![("svc_name".to_string(), Value::Str(def.name.clone()))],
    );

    // ~~"NOT ENFORCED the manifest asks for listening sockets and this
    // supervisor cannot supply them: ADR-017 §3.2's pre-created socket and
    // §3.4's LISTEN_FDS handoff are not built. The service starts with no
    // descriptors passed to it"~~
    //
    // **Struck 2026-08-31: §3.4's handoff IS built.** `AGENTS.md` §7 in the
    // direction it is usually not — a claim of ABSENCE that outlived the thing
    // it described. It is replaced rather than deleted because an operator who
    // remembers the old line needs to see that it changed, and because
    // `ADR-015` §5.2's *"if you are writing an application: bind your own
    // socket"* is now wrong in the other direction.
    //
    // **What is still not built is named, and it is not this.** §3.2's ordering
    // — starting a service *because* a connection arrived — is untouched.
    if !def.listen.is_empty() {
        note(
            k,
            journal,
            Severity::Info,
            "ENFORCED this supervisor opens the sockets this manifest declares and passes the \
             descriptors from 3 upward, with LISTEN_FDS and LISTEN_PID (ADR-015 §5.2, ADR-017 \
             §3.4, built 2026-08-31). The application never binds them and never holds \
             CAP_NET_BIND_SERVICE, so a port below 1024 is reachable to it. NOT socket \
             activation: the socket is opened when the service starts, and nothing starts a \
             service because a connection arrived. A unix socket is 0600 root-owned, so \
             application-to-application connection is still NOT possible — that is ADR-015 \
             §5.3's grant, which needs the mount namespaces this build does not have",
            vec![
                ("svc_name".to_string(), Value::Str(def.name.clone())),
                (
                    "listen".to_string(),
                    Value::Str(render_sockets(&def.listen)),
                ),
                // `ADR-046` §7: `127.0.0.1:<port>` is reachable only on this
                // machine, `0.0.0.0:<port>` from the network.
                (
                    "bound".to_string(),
                    Value::Str(render_bound(&def.listen, def.proxied)),
                ),
            ],
        );
    }

    if def.readiness == Readiness::ListeningUnsupported {
        note(
            k,
            journal,
            Severity::Warn,
            "NOT ENFORCED the manifest declares readiness over a socket Rivet would have \
             opened, and none is opened, so this service will never be reported ready \
             (ADR-017 §4.2)",
            vec![("svc_name".to_string(), Value::Str(def.name.clone()))],
        );
    }
}

/// Two applications cannot both be given the same socket.
///
/// **This check moved here from `rivet-config` on 2026-08-24**, and the move
/// is the ruling's cost rather than a tidy-up. One document held every `listen`
/// claim, so the parser could refuse a collision with no knowledge of the
/// machine; the claims are in separate signed manifests now, and the supervisor
/// is the only thing that holds them all at once.
///
/// It is **reported, not refused**, and that difference is deliberate. A parser
/// could refuse: nothing had started, and the operator was authoring a file.
/// Here, refusing would mean choosing which of two signed applications does not
/// run, at boot, on a headless machine — and neither has done anything wrong.
/// Nothing binds these sockets today anyway (`ADR-017` §3.2 is unbuilt), so a
/// collision is a fact about two manifests rather than a failure to start.
/// A socket, in the manifest's own spelling.
pub(crate) fn render_sockets(sockets: &[Socket]) -> String {
    sockets
        .iter()
        .map(Socket::render)
        .collect::<Vec<_>>()
        .join(",")
}

/// **Where each tcp and udp socket binds**, as `address:port` joined by commas
/// — `ADR-046` §7, `.rulings/` R-188. The journal's answer to *can the LAN
/// reach this port*, from the function the kernel binds with.
pub(crate) fn render_bound(sockets: &[Socket], proxied: bool) -> String {
    sockets
        .iter()
        .filter_map(|s| Some(format!("{}:{}", s.bind_address(proxied)?, s.port()?)))
        .collect::<Vec<_>>()
        .join(",")
}

/// Say something in the supervisor's own voice, before there is a `Supervisor`
/// to say it through. The same helper `crate::document` and `crate::etc` keep,
/// for the same reason.
fn note(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    sev: Severity,
    msg: &str,
    fields: Vec<(String, Value)>,
) {
    let corr = journal.next_correlation();
    journal.emit(k, sev, SELF_NAME, &corr, msg, fields);
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::testkernel::{FakeKernel, Script};
    use rivet_core::package::application::{Memory, ProgramKind};
    use rivet_core::package::library;
    use rivet_core::service::MemoryLimit;

    fn journal() -> Journal {
        Journal::new("b".to_string(), 1, crate::supervise::ACTOR.to_string())
    }

    fn name(n: &str) -> ServiceName {
        ServiceName::new(n).expect("a valid fixture name")
    }

    /// A manifest as the artifact's own writer renders it — **not** hand-written
    /// TOML. `AGENTS.md` §4: a fixture composed by the reader's own author
    /// proves the reader agrees with itself. Rendering through `Manifest::render`
    /// means these bytes are the ones `cargo rivet package` will emit.
    fn manifest_text(build: impl FnOnce(Manifest) -> Manifest) -> String {
        build(Manifest::new(ProgramKind::Daemon)).render()
    }

    /// Install one application into the fake machine: a directory entry and a
    /// manifest at the path `rivet_core::service` names.
    fn install(k: &mut FakeKernel, app: &str, text: &str) {
        k.dirs_mut()
            .entry(service::APP_ROOT.to_string())
            .or_default()
            .push(app.to_string());
        let path = service::manifest_path(service::BUILT_IN_VOLUME, &name(app));
        // A manifest that reads is a manifest that exists: discovery asks
        // both questions of an image service's name, and a fake answering
        // them differently would test a machine that cannot occur.
        k.exists_mut().insert(path.clone(), Script::always(Ok(true)));
        k.files_mut().insert(path, Script::always(Ok(text.to_string())));
    }

    /// `PLAN-Feature-Set-2026-09-12` §6.2's fixture: a library reference, on
    /// `crate::libraries::tests`' own footing.
    fn addon_reference(product: &str, digest_byte: u8) -> library::Reference {
        library::Reference::parse(&format!(
            "{product}-1.0.0-{}",
            format!("{digest_byte:02x}").repeat(32)
        ))
        .expect("a valid fixture reference")
    }

    /// Install one library into the fake machine: a directory entry under
    /// `<volume>/lib` and a manifest declaring `class`, if any.
    fn install_library(k: &mut FakeKernel, product: &str, digest_byte: u8, class: Option<&str>) {
        let reference = addon_reference(product, digest_byte);
        k.dirs_mut()
            .entry(rivet_core::package::install::lib_root(
                service::BUILT_IN_VOLUME,
            ))
            .or_default()
            .push(reference.to_string());
        let manifest = library::Manifest::new(vec![format!("{product}.so.1")])
            .expect("valid soname")
            .with_addon(class.map(ToString::to_string), None)
            .expect("valid class");
        k.files_mut().insert(
            format!(
                "{}/{}",
                rivet_core::package::install::library_dir(service::BUILT_IN_VOLUME, &reference),
                library::MANIFEST_NAME
            ),
            Script::always(Ok(manifest.render())),
        );
        // `crate::libraries::discover_installed` requires the retained
        // header too, as proof the install finished — put here as the
        // installer leaves it: a really signed package's header, which is
        // binary, so a discovery that tried to read it as text would skip it
        // exactly as it skips every real one.
        let header_path =
            rivet_core::package::install::library_header_path(service::BUILT_IN_VOLUME, &reference);
        let (whole, _) = crate::verify::fixture::signed_application(
            "fixture-library",
            crate::verify::fixture::seed(7),
        );
        k.blobs.insert(
            header_path.clone(),
            whole[..rivet_core::package::format::HEADER_BYTES].to_vec(),
        );
        k.exists_mut().insert(header_path, Script::always(Ok(true)));
    }

    /// The `LD_LIBRARY_PATH` a definition's program was given, if any.
    fn ld_library_path(def: &ServiceDef) -> Option<String> {
        def.program
            .env
            .iter()
            .find(|entry| entry.0 == "LD_LIBRARY_PATH")
            .map(|entry| entry.1.clone())
    }

    /// **The whole of the ruling, from the outside.** A service defined by an
    /// installed application's manifest, with every field coming from where the
    /// ruling says it comes from.
    #[test]
    fn an_installed_application_defines_a_service_from_its_manifest() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(
            &mut k,
            "photo-index",
            &manifest_text(|m| {
                m.with_restart(Restart::OnFailure)
                    .with_memory(Some(Memory::parse("128M").expect("valid")))
            }),
        );

        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        assert_eq!(defs.len(), 1, "{:?}", k.records);
        let def = &defs[0];
        assert_eq!(def.name, "photo-index");
        assert_eq!(def.origin, Origin::Application);
        // From the manifest, inside the signature.
        assert_eq!(def.restart, RestartPolicy::OnFailure);
        assert_eq!(
            def.requested_memory.map(MemoryLimit::bytes),
            Some(128 * 1024 * 1024)
        );
        // Derived, never declared — ADR-015 §3.2 has no `exec`.
        assert_eq!(
            def.program.path,
            "/state/app/photo-index/current/bin/photo-index"
        );
        assert!(
            def.program.args.is_empty(),
            "a manifest cannot pass arguments"
        );
        // The operator's half has not arrived yet, so the request stands alone.
        assert_eq!(
            def.memory.source,
            rivet_core::service::MemorySource::Manifest
        );
        assert_eq!(def.validate(), Ok(()));
    }

    // ---- D4: the default memory limit ---------------------------------------

    /// A Pi 4's `MemTotal` line, as one reads. Read off a board.
    const PI4_MEMINFO: &str = "MemTotal:         922976 kB\nMemFree:          772288 kB\n";

    /// **An application whose manifest asks for no limit runs under half the
    /// machine**, and one that asks keeps what it asked for.
    #[test]
    fn an_application_asking_for_no_memory_limit_is_given_half_the_machine() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        k.files_mut()
            .insert(MEMINFO.to_string(), Script::always(Ok(PI4_MEMINFO.to_string())));
        install(&mut k, "silent", &manifest_text(|m| m));
        install(
            &mut k,
            "asking",
            &manifest_text(|m| m.with_memory(Some(Memory::parse("128M").expect("valid")))),
        );

        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let silent = defs.iter().find(|d| d.name == "silent").expect("discovered");
        assert_eq!(silent.memory.limit.map(MemoryLimit::bytes), Some(922_976 * 512));
        assert_eq!(silent.memory.source, rivet_core::service::MemorySource::Default);
        let asking = defs.iter().find(|d| d.name == "asking").expect("discovered");
        assert_eq!(asking.memory.limit.map(MemoryLimit::bytes), Some(128 * 1024 * 1024));
        assert_eq!(asking.memory.source, rivet_core::service::MemorySource::Manifest);
    }

    /// **A machine whose memory cannot be read applies no default, and says
    /// which applications run with no limit because of it.**
    #[test]
    fn unreadable_machine_memory_applies_no_default_and_says_so() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, "silent", &manifest_text(|m| m));

        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        assert_eq!(defs[0].memory.limit, None);
        let warned: Vec<_> = k
            .records
            .iter()
            .filter(|r| r.sev == Severity::Warn && r.msg.contains("memory could not be read"))
            .collect();
        assert_eq!(warned.len(), 1, "{:?}", k.records);
        assert!(
            warned[0]
                .fields
                .iter()
                .any(|(key, v)| key == "applications" && *v == Value::Str("silent".to_string())),
            "{:?}",
            warned[0]
        );
    }

    // ---- addon classes — PLAN-Feature-Set-2026-09-12 §6.2, ADR-053 ---------

    /// **The whole primitive, through the real discovery path.** An installed
    /// library whose class matches what the application accepts joins its
    /// `LD_LIBRARY_PATH`, with no explicit `library` reference naming it —
    /// which is the friction §6.1 exists to remove.
    #[test]
    fn an_installed_library_of_a_matching_class_joins_at_discovery() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(
            &mut k,
            "aura-controller",
            &manifest_text(|m| {
                m.with_accepts_classes(vec!["temperature-sensor".to_string()])
                    .expect("valid")
            }),
        );
        install_library(&mut k, "libtemp", 0x11, Some("temperature-sensor"));

        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let def = defs
            .iter()
            .find(|d| d.name == "aura-controller")
            .expect("the application was discovered");
        assert_eq!(
            ld_library_path(def),
            Some(rivet_core::package::install::library_object_dir(
                service::BUILT_IN_VOLUME,
                &addon_reference("libtemp", 0x11)
            ))
        );
    }

    /// **Nothing joins by being present.** A class that does not match, an
    /// unclassed library, and an application that names no class at all are
    /// each proven separately in `rivet_core::service`'s own unit tests
    /// (no I/O, `AGENTS.md` §4); this is the one that ties the two modules
    /// together — the mismatch survives the real discovery path too.
    #[test]
    fn a_mismatched_class_does_not_join_at_discovery() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(
            &mut k,
            "aura-controller",
            &manifest_text(|m| {
                m.with_accepts_classes(vec!["temperature-sensor".to_string()])
                    .expect("valid")
            }),
        );
        install_library(&mut k, "libpressure", 0x22, Some("pressure-sensor"));

        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let def = defs
            .iter()
            .find(|d| d.name == "aura-controller")
            .expect("the application was discovered");
        assert_eq!(ld_library_path(def), None);
    }

    /// **Discovery re-runs after an install and picks up a newly installed
    /// matching library** — `crate::installed::Watch::look` calls
    /// `discover` again on exactly this event, and this proves the primitive
    /// through that re-run rather than assuming it from the first pass alone.
    #[test]
    fn a_library_installed_after_first_discovery_joins_on_the_next_pass() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(
            &mut k,
            "aura-controller",
            &manifest_text(|m| {
                m.with_accepts_classes(vec!["temperature-sensor".to_string()])
                    .expect("valid")
            }),
        );

        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);
        assert_eq!(
            ld_library_path(defs.iter().find(|d| d.name == "aura-controller").expect("found")),
            None,
            "nothing is installed yet"
        );

        // The library arrives, and discovery runs again over the SAME `defs`
        // — exactly what `crate::installed::Watch::look` does after an
        // install, re-deriving every definition from what is on the machine
        // now rather than trusting what the first pass found.
        install_library(&mut k, "libtemp", 0x11, Some("temperature-sensor"));
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let def = defs
            .iter()
            .find(|d| d.name == "aura-controller")
            .expect("still discovered");
        assert_eq!(
            ld_library_path(def),
            Some(rivet_core::package::install::library_object_dir(
                service::BUILT_IN_VOLUME,
                &addon_reference("libtemp", 0x11)
            )),
            "the second discovery pass must pick up the newly installed library"
        );
    }

    /// **`.rulings/` R-162 R28, through the real discovery path**: an
    /// application accepting two classes joins a library of each, the
    /// sibling of `rivet_core::service`'s own no-I/O proof of the same rule.
    #[test]
    fn an_application_accepting_two_classes_joins_a_library_of_each_at_discovery() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(
            &mut k,
            "aura-controller",
            &manifest_text(|m| {
                m.with_accepts_classes(vec![
                    "temperature-sensor".to_string(),
                    "humidity-sensor".to_string(),
                ])
                .expect("valid")
            }),
        );
        install_library(&mut k, "libtemp", 0x11, Some("temperature-sensor"));
        install_library(&mut k, "libhumid", 0x22, Some("humidity-sensor"));

        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let def = defs
            .iter()
            .find(|d| d.name == "aura-controller")
            .expect("the application was discovered");
        let joined = ld_library_path(def).expect("both installed libraries match");
        assert!(joined.contains(
            rivet_core::package::install::library_object_dir(
                service::BUILT_IN_VOLUME,
                &addon_reference("libtemp", 0x11)
            )
            .as_str()
        ));
        assert!(joined.contains(
            rivet_core::package::install::library_object_dir(
                service::BUILT_IN_VOLUME,
                &addon_reference("libhumid", 0x22)
            )
            .as_str()
        ));
    }

    /// **An application accepting no class is unchanged**, through the real
    /// discovery path — the sibling of `rivet_core::service`'s own
    /// no-I/O proof of the same rule.
    #[test]
    fn an_application_accepting_no_class_is_unaffected_by_what_is_installed() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, "photo-index", &manifest_text(|m| m));
        install_library(&mut k, "libtemp", 0x11, Some("temperature-sensor"));

        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let def = defs
            .iter()
            .find(|d| d.name == "photo-index")
            .expect("the application was discovered");
        assert_eq!(ld_library_path(def), None);
    }

    // ---- the enumeration channel — PLAN-Addon-Feature-Set-2026-09-19 T2 ----

    /// The `RIVET_ADDONS` a definition's program was given, if any.
    fn rivet_addons(def: &ServiceDef) -> Option<String> {
        def.program
            .env
            .iter()
            .find(|entry| entry.0 == service::ADDONS_ENV)
            .map(|entry| entry.1.clone())
    }

    /// The object the enumeration channel should name for a library
    /// `install_library` put on the machine.
    fn addon_object(product: &str, digest_byte: u8) -> String {
        format!(
            "{}/{product}.so.1",
            rivet_core::package::install::library_object_dir(
                service::BUILT_IN_VOLUME,
                &addon_reference(product, digest_byte)
            )
        )
    }

    /// **The whole channel, through the real discovery path.** The library
    /// that joined `LD_LIBRARY_PATH` is also named, by its class and by the
    /// absolute path of its object — so a host learns *which*, and not only
    /// *where*.
    #[test]
    fn a_joined_library_is_named_to_its_host_at_discovery() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(
            &mut k,
            "aura-controller",
            &manifest_text(|m| {
                m.with_accepts_classes(vec!["temperature-sensor".to_string()])
                    .expect("valid")
            }),
        );
        install_library(&mut k, "libtemp", 0x11, Some("temperature-sensor"));

        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let def = defs
            .iter()
            .find(|d| d.name == "aura-controller")
            .expect("the application was discovered");
        assert_eq!(
            rivet_addons(def),
            Some(format!(
                "temperature-sensor={}",
                addon_object("libtemp", 0x11)
            ))
        );
    }

    /// **The case the naming rule could not express, on the machine's own
    /// path** — `.rulings/` R-199 D1. Two libraries of one class are two
    /// entries; under `soname_for_class` they were one filename asked for
    /// twice, and the loader answered with whichever it found first.
    #[test]
    fn two_libraries_of_one_class_are_both_named_at_discovery() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(
            &mut k,
            "aura-renderer",
            &manifest_text(|m| {
                m.with_accepts_classes(vec!["aura-background-pack".to_string()])
                    .expect("valid")
            }),
        );
        install_library(&mut k, "packone", 0x11, Some("aura-background-pack"));
        install_library(&mut k, "packtwo", 0x22, Some("aura-background-pack"));

        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let def = defs
            .iter()
            .find(|d| d.name == "aura-renderer")
            .expect("the application was discovered");
        let named = rivet_addons(def).expect("both libraries join");
        let entries: Vec<&str> = named.split(':').collect();
        assert_eq!(entries.len(), 2, "two libraries, two entries: {named}");
        assert!(entries.contains(
            &format!("aura-background-pack={}", addon_object("packone", 0x11)).as_str()
        ));
        assert!(entries.contains(
            &format!("aura-background-pack={}", addon_object("packtwo", 0x22)).as_str()
        ));
    }

    /// **The three composers agree because they share a list.** Every named
    /// object sits in a directory on `LD_LIBRARY_PATH` and in a directory the
    /// Landlock ruleset grants — the disagreement that cost an afternoon on
    /// `rivet-06f5c` on 2026-09-19, now checked on all three at once.
    #[test]
    fn every_named_object_is_searchable_and_openable() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(
            &mut k,
            "aura-renderer",
            &manifest_text(|m| {
                m.with_accepts_classes(vec!["aura-background-pack".to_string()])
                    .expect("valid")
            }),
        );
        install_library(&mut k, "packone", 0x11, Some("aura-background-pack"));
        install_library(&mut k, "packtwo", 0x22, Some("aura-background-pack"));

        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let def = defs
            .iter()
            .find(|d| d.name == "aura-renderer")
            .expect("the application was discovered");
        let searched: Vec<String> = ld_library_path(def)
            .expect("both libraries join")
            .split(':')
            .map(ToString::to_string)
            .collect();
        let policy = def
            .program
            .confinement
            .as_ref()
            .expect("an application is confined");

        for entry in rivet_addons(def).expect("both libraries join").split(':') {
            let (_, path) = entry.split_once('=').expect("every entry is class=path");
            let dir = path
                .rsplit_once('/')
                .expect("an absolute path has a directory")
                .0;
            assert!(
                searched.iter().any(|s| s == dir),
                "{path} is named and {dir} is not on LD_LIBRARY_PATH: {searched:?}"
            );
            assert!(
                policy.grants.iter().any(|g| g.path == dir),
                "{path} is named and {dir} is granted by no Landlock rule"
            );
        }
    }

    /// Nothing joined, nothing said — a host with no addon reads an absent
    /// variable rather than an empty one, which is the same answer it read
    /// before this channel existed.
    #[test]
    fn an_application_no_library_joined_is_told_nothing() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(
            &mut k,
            "aura-controller",
            &manifest_text(|m| {
                m.with_accepts_classes(vec!["temperature-sensor".to_string()])
                    .expect("valid")
            }),
        );
        install_library(&mut k, "libpressure", 0x22, Some("pressure-sensor"));

        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let def = defs
            .iter()
            .find(|d| d.name == "aura-controller")
            .expect("the application was discovered");
        assert_eq!(rivet_addons(def), None);
    }

    /// **The application is told where its settings are** — `ADR-038` §8.1,
    /// 2026-09-04.
    ///
    /// It used to be able to compose the path, because the file was inside the
    /// directory `RIVET_APP_DATA` names. It is not any more, and nothing else in
    /// the handed environment points at the directory it moved to — so a
    /// supervisor that forgot this variable would deliver a file no application
    /// could find, and every symptom would be *the setting did not apply*.
    #[test]
    fn an_application_is_handed_the_path_to_its_own_settings_file() {
        let manifest = Manifest::new(ProgramKind::Daemon);
        let def = definition(service::BUILT_IN_VOLUME, &name("aura-renderer"), &manifest, &[]);

        let handed = def
            .program
            .env
            .iter()
            .find(|(k, _)| k == rivet_core::service::APP_SETTINGS_ENV)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| panic!("nothing named the settings file: {:?}", def.program.env));

        // The same function the writer uses, so the two cannot drift.
        assert_eq!(
            handed,
            rivet_core::settings::path(service::BUILT_IN_VOLUME, &name("aura-renderer"))
        );
        // And it is NOT inside what the application may write, which is the
        // whole of what the move bought.
        assert!(
            !handed.starts_with(&service::data_dir(
                service::BUILT_IN_VOLUME,
                &name("aura-renderer")
            )),
            "{handed}"
        );
    }

    /// The restart policy is the manifest's, whichever it is. If this ever
    /// starts coming from anywhere else, the signature stops meaning anything
    /// about how the program is run (`ADR-015` §4.1).
    #[test]
    fn every_restart_policy_survives_the_crossing_from_manifest_to_definition() {
        for (declared, expected) in [
            (Restart::Always, RestartPolicy::Always),
            (Restart::OnFailure, RestartPolicy::OnFailure),
            (Restart::Never, RestartPolicy::Never),
        ] {
            let manifest = Manifest::new(ProgramKind::Daemon).with_restart(declared);
            assert_eq!(
                definition(service::BUILT_IN_VOLUME, &name("x"), &manifest, &[]).restart,
                expected
            );
        }
    }

    /// `ADR-015` §6.1 — an unenforced declaration must be visible. A manifest
    /// that declares readiness over a socket Rivet does not open must not be
    /// recorded as a program with no readiness signal.
    #[test]
    fn a_readiness_this_build_cannot_supply_is_distinguished_rather_than_flattened() {
        let listening = Manifest::new(ProgramKind::Daemon).with_ready(Ready::Listening);
        assert_eq!(
            definition(service::BUILT_IN_VOLUME, &name("x"), &listening, &[]).readiness,
            Readiness::ListeningUnsupported
        );
        let silent = Manifest::new(ProgramKind::Daemon).with_ready(Ready::None);
        assert_eq!(
            definition(service::BUILT_IN_VOLUME, &name("x"), &silent, &[]).readiness,
            Readiness::OnStart
        );
        // And neither of them notifies, which is what everything downstream
        // actually asks.
        assert!(
            !definition(service::BUILT_IN_VOLUME, &name("x"), &listening, &[])
                .readiness
                .notifies()
        );
    }

    /// A machine with nothing installed is not a machine with a problem.
    #[test]
    fn a_machine_with_no_applications_says_so_and_is_not_degraded() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);
        assert!(defs.is_empty());
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Info && r.msg.contains("no applications are installed")),
            "{:?}",
            k.records
        );
        assert!(
            !k.records.iter().any(|r| r.sev == Severity::Error),
            "an ordinary machine reported an error"
        );
    }

    /// **Refuse rather than guess** (`AGENTS.md` §5). A manifest that will not
    /// parse must not become a service with defaulted fields — a restart policy
    /// nobody wrote is a machine behaving in a way nobody wrote down.
    #[test]
    fn an_application_whose_manifest_will_not_parse_is_not_started_and_says_why() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, "broken", "kind = \"daemon\"\nnonsense\n");
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);
        assert!(defs.is_empty(), "a broken manifest produced a service");
        assert!(
            k.records
                .iter()
                .any(|r| r.msg.contains("could not be read") && r.msg.contains("NOT started")),
            "{:?}",
            k.records
        );
    }

    /// An application whose directory has no manifest at all — a half-finished
    /// install, or a directory somebody made by hand.
    #[test]
    fn an_application_with_no_manifest_is_not_started_and_says_why() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        k.dirs_mut()
            .entry(service::APP_ROOT.to_string())
            .or_default()
            .push("halfway".to_string());
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);
        assert!(defs.is_empty());
        assert!(
            k.records
                .iter()
                .any(|r| r.msg.contains("no readable manifest")),
            "{:?}",
            k.records
        );
    }

    /// A directory name that is not a service name — it would be a cgroup path
    /// component and a socket name as well as a directory.
    #[test]
    fn a_directory_that_is_not_a_service_name_is_refused_without_echoing_it() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        k.dirs_mut()
            .entry(service::APP_ROOT.to_string())
            .or_default()
            .push("not a name".to_string());
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);
        assert!(defs.is_empty());
        let said = k
            .records
            .iter()
            .find(|r| r.msg.contains("not a usable service name"))
            .unwrap_or_else(|| panic!("nothing said why: {:?}", k.records));
        assert!(
            !said.to_jsonl().contains("not a name"),
            "the record echoed the input"
        );
    }

    /// One name, one definition. An application called `rivet-mcpd` would
    /// otherwise shadow the image's control plane, or be shadowed by it,
    /// depending on which list was consulted first — and the machine would be
    /// unreachable either way.
    #[test]
    fn an_application_cannot_take_the_name_of_a_service_the_image_defines() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, "rivet-mcpd", &manifest_text(|m| m));
        let mut defs = crate::definitions::system_services();
        let before = defs.len();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);
        assert_eq!(
            defs.len(),
            before,
            "the application replaced or joined a system service"
        );
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Error && r.msg.contains("same name")),
            "{:?}",
            k.records
        );
        // And the image's definition is untouched.
        let mcpd = defs
            .iter()
            .find(|d| d.name == crate::definitions::MCPD_NAME)
            .expect("still there");
        assert_eq!(mcpd.origin, Origin::Image);
    }

    /// **Installing is not running — ruled by the maintainer 2026-08-25.**
    ///
    /// There was no test here at all until that ruling, which is why the
    /// default went eight days without being examined: `enabled: true` was a
    /// value in a struct literal, and flipping it broke nothing anywhere in the
    /// workspace. A property that no test asserts is a property nobody chose.
    ///
    /// The rule is `ADR-015` §3.2.1's — `enabled` belongs to the document
    /// alone. A discovered definition may not answer it, and silence is not
    /// consent.
    #[test]
    fn a_discovered_application_is_not_enabled_by_the_fact_of_being_installed() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, "photo-index", &manifest_text(|m| m));
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let app = defs
            .iter()
            .find(|d| d.name == "photo-index")
            .expect("discovered");
        assert!(
            !app.enabled,
            "an application nobody mentioned in the document would run on the next convergence"
        );
        // And it is the ONLY field discovery invents, so nothing else here is
        // an answer to a question the operator was never asked.
        assert_eq!(app.origin, Origin::Application);
    }

    /// The other half of the same rule: the document is what turns it on, and
    /// discovery must not have made that impossible.
    #[test]
    fn the_document_is_what_enables_a_discovered_application() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, "photo-index", &manifest_text(|m| m));
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let document = rivet_config::Document::parse(
            "version = 1\n[services.photo-index]\nenabled = true\n",
        )
        .expect("a valid fixture document");
        let desired = crate::document::Desired::Stored(document);
        crate::document::apply_desired_state(&mut k, &mut j, &mut defs, &desired);

        assert!(
            defs.iter()
                .find(|d| d.name == "photo-index")
                .is_some_and(|d| d.enabled),
            "the document said to run it and the machine did not agree"
        );
    }

    /// **This test used to assert the opposite, and that is the point of it.**
    ///
    /// It was written to fail *"on the day somebody deletes the warning without
    /// building the check"*. The check was built on 2026-08-25 (`PLAN.md` §15)
    /// and nobody deleted the warning — so for a day this test held a false
    /// statement in place, and a board printed it about an application it had
    /// just verified. It was read off hardware and corrected (`PLAN.md` §19.5).
    ///
    /// It is still a tripwire, pointed the other way: a build that stops
    /// verifying at launch must not go on reporting that it does.
    #[test]
    fn every_discovery_reports_that_a_binary_is_verified_at_launch() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, "photo-index", &manifest_text(|m| m));
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);
        assert!(
            k.records
                .iter()
                .any(|r| r.msg.contains("ENFORCED") && r.msg.contains("§4.2")),
            "nothing said whether §4.2 is honoured: {:?}",
            k.records
        );
        assert!(
            !k.records
                .iter()
                .any(|r| r.msg.contains("NOT ENFORCED") && r.msg.contains("§4.2")),
            "it still reports launch verification as absent, and `crate::verify::program` runs \
             before every application spawn: {:?}",
            k.records
        );
    }

    /// ~~...are reported and both still start.~~ **Renamed and reversed
    /// 2026-08-31**, because the behaviour it pinned was correct until that day
    /// and is not any more: both used to start, on the recorded reason that
    /// *"nothing opens these sockets yet and refusing would mean choosing which
    /// signed application does not run"*. `ADR-015` §5.2's descriptor handoff
    /// answered the first half — the supervisor opens them now — and §6.2
    /// answered the second by ruling that choosing IS the supervisor's job.
    ///
    /// The name changed with the assertion. A test called
    /// `..._and_both_still_start` that asserted a refusal would be worse than
    /// no test at all.
    ///
    /// The check `rivet-config` gave up when `listen` left the document. It
    /// is a *report* here rather than a refusal — see
    /// [`refuse_colliding_sockets`].
    #[test]
    fn two_applications_asking_for_one_socket_are_refused_rather_than_both_started() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        let listening = |port: u16| {
            manifest_text(move |m| {
                m.with_listen(vec![Socket::Tcp(port)])
                    .expect("one socket is under the limit")
            })
        };
        install(&mut k, "first", &listening(8080));
        install(&mut k, "second", &listening(8080));
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["first"], "the second must NOT be defined");
        let said = k
            .records
            .iter()
            .find(|r| r.msg.contains("already asks for this socket"))
            .unwrap_or_else(|| panic!("the collision went unreported: {:?}", k.records));
        assert!(said.to_jsonl().contains("tcp:8080"), "{}", said.to_jsonl());
    }

    /// Different ports are not a collision, or every machine with two
    /// applications on it would report one.
    #[test]
    fn two_applications_on_different_sockets_are_not_reported() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        for (app, port) in [("first", 8080_u16), ("second", 8081)] {
            install(
                &mut k,
                app,
                &manifest_text(move |m| m.with_listen(vec![Socket::Tcp(port)]).expect("valid")),
            );
        }
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);
        assert_eq!(defs.len(), 2);
        assert!(!k.records.iter().any(|r| r.msg.contains("same socket")));
    }

    /// **One port on two reaches is still one port** — `ADR-046` §7. A proxied
    /// application's `tcp:8080` binds loopback and an unproxied one's binds
    /// every address, and the second `bind` would answer `EADDRINUSE`; the
    /// collision is refused by name, as for two identical spellings. And the
    /// definition carries `proxied` from the manifest, which is what the kernel
    /// binds by.
    #[test]
    fn one_port_on_two_reaches_is_one_socket_and_the_definition_knows_it_is_proxied() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(
            &mut k,
            "first",
            &manifest_text(|m| {
                m.with_listen(vec![Socket::Tcp(9000), Socket::TcpDirect(8080)])
                    .expect("valid")
                    .with_proxy_path(Some("first".to_string()))
                    .expect("valid")
            }),
        );
        install(
            &mut k,
            "second",
            &manifest_text(|m| m.with_listen(vec![Socket::Tcp(8080)]).expect("valid")),
        );
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["first"], "the second must NOT be defined");
        assert!(defs[0].proxied, "a manifest with a proxy-path is proxied");
        let said = k
            .records
            .iter()
            .find(|r| r.msg.contains("already asks for this socket"))
            .unwrap_or_else(|| panic!("the collision went unreported: {:?}", k.records));
        assert!(said.to_jsonl().contains("tcp:8080"), "{}", said.to_jsonl());
        let reported = k
            .records
            .iter()
            .find(|r| r.msg.starts_with("ENFORCED this supervisor opens the sockets"))
            .unwrap_or_else(|| panic!("no socket report: {:?}", k.records));
        assert!(
            reported
                .to_jsonl()
                .contains("127.0.0.1:9000,0.0.0.0:8080"),
            "{}",
            reported.to_jsonl()
        );
    }

    // ---- the identity, ruled 2026-08-24 ------------------------------------

    /// **The whole of the ruling, from the outside.** An installed application
    /// carries a uid of its own, it is not root, and it is the one its name
    /// derives — which is what makes it the same on the next boot.
    #[test]
    fn an_installed_application_runs_under_an_identity_of_its_own_and_never_as_root() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, "photo-index", &manifest_text(|m| m));
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let id = defs[0]
            .program
            .identity
            .unwrap_or_else(|| panic!("no identity: {:?}", k.records));
        assert_ne!(id.uid, 0, "the application would have run as root");
        assert_eq!(id, service::app_identity(&name("photo-index")));
        assert_eq!(id.uid, id.gid, "an application's group is its own");
        // And the definition is one the supervisor will accept.
        assert_eq!(defs[0].validate(), Ok(()));
    }

    /// **The property a restart depends on**, checked through the supervisor
    /// rather than only through the function. A second boot reads the same
    /// directory and must produce the same uid, or the application comes back
    /// unable to read anything it wrote.
    #[test]
    fn the_next_boot_gives_the_same_application_the_same_uid() {
        let uid_after_a_boot = || {
            let mut k = FakeKernel::new();
            let mut j = journal();
            install(&mut k, "photo-index", &manifest_text(|m| m));
            let mut defs = Vec::new();
            discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);
            defs[0].program.identity.map(|i| i.uid)
        };
        assert_eq!(uid_after_a_boot(), uid_after_a_boot());
        assert!(uid_after_a_boot().is_some());
    }

    /// A system service is **not** given one, and this is the test that fails
    /// the day somebody makes the identity unconditional. `rivet-netd` speaks
    /// netlink and `rivet-mcpd` execs the CLI that installs payloads and
    /// reboots the machine; either one running as uid 41234 starts and cannot
    /// do its job.
    ///
    /// **The image services with a home are the exception, and exactly they
    /// are** (`ADR-060`): each runs as the uid its name derives, and nothing
    /// else the image defines has one.
    #[test]
    fn no_service_the_image_defines_is_given_an_identity_unless_it_has_a_home() {
        for def in crate::definitions::system_services() {
            let expected = service::image_service_home(&def.name)
                .and_then(|home| ServiceName::new(home.name))
                .map(|name| service::app_identity(&name));
            assert_eq!(
                def.program.identity, expected,
                "{} would not be able to do its job",
                def.name
            );
        }
    }

    /// `ADR-015` §3.2's `state = "private"` — *"gets `/state/app/<name>`, mode
    /// 700"*. Without it the identity would be a regression: `/system` is
    /// read-only and everything else on `/state` belongs to root, so an
    /// unprivileged application would have nowhere at all to write.
    #[test]
    fn an_installed_application_is_given_a_private_directory_it_owns() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, "photo-index", &manifest_text(|m| m));
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let id = defs[0].program.identity.expect("an identity");
        let path = service::data_dir(service::BUILT_IN_VOLUME, &name("photo-index"));
        assert!(
            k.mkdir_mode_calls
                .contains(&(path.clone(), service::DATA_DIR_MODE)),
            "{:?}",
            k.mkdir_mode_calls
        );
        // `contains` and not equality since 2026-09-07: the scratch directory
        // is chowned in the same pass. Equality here asserted that the data
        // directory was the ONLY thing given to the application, which was
        // never what this test was about.
        assert!(
            k.chowns.contains(&(path.clone(), id.uid, id.gid)),
            "the application's own directory belongs to somebody else: {:?}",
            k.chowns
        );
        // And it is NOT under `current`, or the next update would take the
        // application's data with it.
        assert!(!path.starts_with(&service::current_dir(service::BUILT_IN_VOLUME, &name("photo-index"))));
    }

    /// **The scratch directory is made, owned, and named in the environment —
    /// under BOTH names.**
    ///
    /// The two names are not redundant: `TMPDIR` is what a library that was
    /// never written for Rivet reads, and `RIVET_APP_TMP` is what this project
    /// names on `RIVET_APP_DATA`'s footing. Setting only the second would have
    /// left `SQLITE_CANTOPEN` exactly where it was.
    #[test]
    fn an_installed_application_is_given_a_scratch_directory_under_both_names() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, "photo-index", &manifest_text(|m| m));
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let id = defs[0].program.identity.expect("an identity");
        let path = service::tmp_dir(service::BUILT_IN_VOLUME, &name("photo-index"));
        assert!(
            k.mkdir_mode_calls
                .contains(&(path.clone(), service::TMP_DIR_MODE)),
            "{:?}",
            k.mkdir_mode_calls
        );
        assert!(
            k.chowns.contains(&(path.clone(), id.uid, id.gid)),
            "the scratch directory belongs to somebody else: {:?}",
            k.chowns
        );

        let env = &defs[0].program.env;
        let value = |key: &str| {
            env.iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| panic!("{key} is not in the environment: {env:?}"))
        };
        assert_eq!(value(service::APP_TMP_ENV), path);
        assert_eq!(
            value(service::TMPDIR_ENV),
            path,
            "TMPDIR is the name every library already reads; disagreeing with RIVET_APP_TMP \
             would send them to different places"
        );

        // Not inside `data`, or emptying it would risk the history.
        assert!(!path.starts_with(&service::data_dir(
            service::BUILT_IN_VOLUME,
            &name("photo-index")
        )));
    }

    /// A directory that cannot be given to the application is reported and the
    /// service still starts: a service that is not started is a fault the
    /// operator sees, and one that cannot write is a fault only the application
    /// sees.
    #[test]
    fn a_data_directory_that_cannot_be_owned_is_reported_and_the_service_still_starts() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        k.chown_results = Script::always(Err(Errno::EROFS));
        install(&mut k, "photo-index", &manifest_text(|m| m));
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);
        assert_eq!(defs.len(), 1, "the application was dropped over a chown");
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Error && r.msg.contains("belongs to somebody")),
            "{:?}",
            k.records
        );
    }

    /// **The collision, with a pair found by search over the real function
    /// rather than written down.** Two applications on one uid is not a weak
    /// sandbox between them — it is none — so the second is NOT started.
    ///
    /// The pair cannot be a fixture: it is a property of
    /// `rivet_core::service::app_identity`, and a hand-written pair would go
    /// stale the moment that function changed and would then silently test the
    /// non-colliding case.
    #[test]
    fn two_applications_that_derive_one_uid_do_not_both_start() {
        let Some((first, second)) = colliding_pair() else {
            panic!("no colliding pair found; app_identity's range or hash has changed shape");
        };
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, &first, &manifest_text(|m| m));
        install(&mut k, &second, &manifest_text(|m| m));
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        assert_eq!(defs.len(), 1, "both applications took the same uid");
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Error && r.msg.contains("same uid")),
            "{:?}",
            k.records
        );
        // Deterministic: the one that survives is the lower name, not whichever
        // the directory listing happened to yield first.
        let (lower, _) = if first < second {
            (&first, &second)
        } else {
            (&second, &first)
        };
        assert_eq!(&defs[0].name, lower);
    }

    /// Two applications that do **not** collide both start, or the refusal
    /// above would be indistinguishable from a rule that refuses everything.
    #[test]
    fn two_applications_with_different_uids_both_start() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, "photo-index", &manifest_text(|m| m));
        install(&mut k, "metrics", &manifest_text(|m| m));
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);
        assert_eq!(defs.len(), 2);
        assert!(!k.records.iter().any(|r| r.msg.contains("same uid")));
        assert_ne!(defs[0].program.identity, defs[1].program.identity);
    }

    /// `ADR-015` §6.1 — the identity is *enforced* and the rest of §5.1's table
    /// is not, and both have to be legible on a console. A machine that said
    /// only the first would have an operator believing in a sandbox that does
    /// not exist.
    #[test]
    fn both_what_is_enforced_and_what_is_not_are_said_out_loud() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, "photo-index", &manifest_text(|m| m));
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Info && r.msg.contains("ENFORCED this application")),
            "{:?}",
            k.records
        );
        assert!(
            k.records.iter().any(|r| r.sev == Severity::Warn
                && r.msg.contains("NOT ENFORCED")
                && r.msg.contains("mount namespace")),
            "{:?}",
            k.records
        );
    }

    /// A provider of `socket:<endpoint>:shared`, as a signed manifest says it.
    fn shared_provider(endpoint: &str) -> String {
        let provided = format!("socket:{endpoint}:shared");
        manifest_text(move |m| {
            m.with_provides(vec![
                rivet_core::package::application::Endpoint::parse("provides", &provided)
                    .expect("a valid socket:name:shared"),
            ])
            .expect("one endpoint is under the limit")
        })
    }

    /// A consumer that declares `connects = ["<wanted>"]`.
    fn consumer_of(wanted: &str) -> String {
        let wanted = wanted.to_string();
        manifest_text(move |m| {
            m.with_connects(vec![
                rivet_core::package::application::Endpoint::parse("connects", &wanted)
                    .expect("a valid owner:name"),
            ])
            .expect("one endpoint is under the limit")
        })
    }

    /// **An application whose own gid IS a shared endpoint's group is not
    /// started** — the external audit's synthetic pair, recomputed here through
    /// the real functions rather than trusted.
    ///
    /// A shared socket is `root:<endpoint gid>` at `0660`, so an application
    /// whose primary gid equals that group passes the socket's permission check
    /// without declaring `connects` — the declaration stops being enforcement.
    /// The provider, a legitimate consumer and an unrelated control all start;
    /// only the application whose identity is ambiguous is refused.
    #[test]
    fn an_application_whose_gid_is_a_shared_endpoints_group_is_not_started() {
        let colliding = "audit-consumer-292297";
        let gid = service::app_identity(&name(colliding)).gid;
        assert_eq!(
            gid,
            service::endpoint_gid("audit-provider", "socket"),
            "the pair no longer collides; app_identity or endpoint_gid has changed shape, and \
             both are on-disk contracts"
        );

        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, "audit-provider", &shared_provider("socket"));
        install(&mut k, colliding, &manifest_text(|m| m));
        install(&mut k, "legit-consumer", &consumer_of("audit-provider:socket"));
        install(&mut k, "metrics", &manifest_text(|m| m));
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["audit-provider", "legit-consumer", "metrics"],
            "the application on the endpoint's group must NOT be defined, and nothing else may \
             be lost: {:?}",
            k.records
        );
        let said = k
            .records
            .iter()
            .find(|r| r.sev == Severity::Error && r.msg.contains("same gid"))
            .unwrap_or_else(|| panic!("the collision went unreported: {:?}", k.records));
        let line = said.to_jsonl();
        assert!(line.contains(colliding), "{line}");
        assert!(line.contains("audit-provider:socket"), "{line}");
    }

    /// **Two shared endpoints on one group are refused, both of them.** Each
    /// endpoint's consumers would be able to open the other's socket, and
    /// neither provider is more right than the other, so neither is started —
    /// a function of the two declarations and never of which was read first.
    #[test]
    fn two_shared_endpoints_on_one_group_are_both_refused() {
        let (first, second) = colliding_endpoint_pair();
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, &first.0, &shared_provider(&first.1));
        install(&mut k, &second.0, &shared_provider(&second.1));
        install(&mut k, "metrics", &manifest_text(|m| m));
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["metrics"], "{:?}", k.records);
        assert_eq!(
            k.records
                .iter()
                .filter(|r| r.sev == Severity::Error && r.msg.contains("same gid"))
                .count(),
            2,
            "{:?}",
            k.records
        );
    }

    /// **The image's own services do not collide with one another**: every
    /// identity and every shared endpoint group the image defines is distinct.
    /// Discovery can refuse an application, but it cannot refuse the image, so
    /// a clash here has to fail the build instead.
    #[test]
    fn the_images_own_identities_and_endpoint_groups_are_all_distinct() {
        let mut seen: Vec<(u32, String)> = Vec::new();
        for def in crate::definitions::system_services() {
            for claim in claimed_groups(&def) {
                if let Some((_, other)) = seen.iter().find(|(gid, _)| *gid == claim.0) {
                    panic!("{} and {other} share gid {}", claim.1, claim.0);
                }
                seen.push(claim);
            }
        }
    }

    /// Two `(provider, endpoint)` pairs whose groups collide, **found by
    /// searching the real function** for [`colliding_pair`]'s reason. Each
    /// endpoint has a name of its own, so the pair cannot also be refused as
    /// two claims on one socket name.
    fn colliding_endpoint_pair() -> ((String, String), (String, String)) {
        let mut seen: Vec<(u32, (String, String))> = Vec::new();
        for i in 0..20_000u32 {
            let pair = (format!("prov{i}"), format!("ep{i}"));
            let gid = service::endpoint_gid(&pair.0, &pair.1);
            if let Some((_, other)) = seen.iter().find(|(g, _)| *g == gid) {
                return (other.clone(), pair);
            }
            seen.push((gid, pair));
        }
        panic!("no colliding endpoint pair found; endpoint_gid's range or hash has changed shape");
    }

    /// Two names that derive one uid, **found by searching the real function**.
    /// See [`two_applications_that_derive_one_uid_do_not_both_start`].
    fn colliding_pair() -> Option<(String, String)> {
        let mut seen: Vec<(u32, String)> = Vec::new();
        for i in 0..20_000u32 {
            let text = format!("app{i}");
            let uid = service::app_identity(&name(&text)).uid;
            if let Some((_, other)) = seen.iter().find(|(u, _)| *u == uid) {
                return Some((other.clone(), text));
            }
            seen.push((uid, text));
        }
        None
    }

    /// A card returning `EIO` on the application root is not "no applications
    /// installed", and reading it as one would be a machine silently running
    /// nothing an operator installed.
    #[test]
    fn an_unreadable_application_root_degrades_loudly_rather_than_reading_as_empty() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        k.dir_errors_mut()
            .insert(service::APP_ROOT.to_string(), Errno::EIO);
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);
        assert!(defs.is_empty());
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Error && r.msg.starts_with("DEGRADED")),
            "{:?}",
            k.records
        );
    }

    // -- ADR-049, the last hop -------------------------------------------------

    /// A staging bundle for `photo-index`, as PID 1 leaves it.
    fn stage(k: &mut FakeKernel, app: &str, files: &[(&str, &[u8])]) {
        let items: Vec<staging::Staged> = files
            .iter()
            .map(|(p, b)| staging::Staged::new(app, p, (*b).to_vec()).expect("a legal entry"))
            .collect();
        k.blobs.insert(
            staging::staged_bundle(app),
            staging::render(&items).into_bytes(),
        );
    }

    /// **The whole of Wave 3, from the outside.** Material that reached the
    /// card lands in the application's own data directory, owned by the
    /// application, before it has ever started — and the staging copy is gone.
    #[test]
    fn staged_material_is_delivered_into_the_data_directory_owned_by_the_application() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, "photo-index", &manifest_text(|m| m));
        stage(
            &mut k,
            "photo-index",
            &[
                ("server.crt", b"CERT"),
                ("server.key", b"KEY"),
                ("register/devices/abc.toml", b"id = \"abc\"\n"),
            ],
        );

        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let id = defs[0].program.identity.expect("an identity");
        let data = service::data_dir(service::BUILT_IN_VOLUME, &name("photo-index"));

        for (path, want) in [
            (format!("{data}/server.crt"), &b"CERT"[..]),
            (format!("{data}/server.key"), &b"KEY"[..]),
            (
                format!("{data}/register/devices/abc.toml"),
                &b"id = \"abc\"\n"[..],
            ),
        ] {
            let write = k
                .writes
                .iter()
                .find(|w| w.path == path)
                .unwrap_or_else(|| panic!("{path} was never written"));
            assert_eq!(write.contents, want, "{path} holds the wrong bytes");
            assert_eq!(
                write.mode,
                staging::FILE_MODE,
                "{path} was not written 0600"
            );
            assert!(
                k.chowns.contains(&(path.clone(), id.uid, id.gid)),
                "{path} was left owned by root, which is the silent failure this exists to end: \
                 {:?}",
                k.chowns
            );
        }

        // Every directory on the way, made and owned — a `0700` directory owned
        // by root is as unreadable to the application as a root-owned file.
        for dir in [format!("{data}/register"), format!("{data}/register/devices")] {
            assert!(
                k.mkdir_mode_calls
                    .contains(&(dir.clone(), service::DATA_DIR_MODE)),
                "{dir} was not created: {:?}",
                k.mkdir_mode_calls
            );
            assert!(
                k.chowns.contains(&(dir.clone(), id.uid, id.gid)),
                "{dir} belongs to somebody else: {:?}",
                k.chowns
            );
        }

        assert!(
            k.removed.contains(&staging::staged_bundle("photo-index")),
            "the staging bundle was left holding a private key: {:?}",
            k.removed
        );
    }

    /// The ordinary machine: nothing staged, nothing written, nothing said.
    #[test]
    fn an_application_with_nothing_staged_is_untouched_and_silent() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, "photo-index", &manifest_text(|m| m));
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let data = service::data_dir(service::BUILT_IN_VOLUME, &name("photo-index"));
        assert!(
            !k.writes.iter().any(|w| w.path.starts_with(&data)),
            "something was written into a data directory nothing was staged for: {:?}",
            k.writes.iter().map(|w| &w.path).collect::<Vec<_>>()
        );
        assert!(k.removed.is_empty(), "{:?}", k.removed);
        assert!(
            !k.records.iter().any(|r| r.msg.contains("ADR-049")),
            "a machine with nothing staged said something about staging"
        );
    }

    /// **A file the application has since rewritten is not rolled back.** A
    /// fleet server writes its own register; overwriting it from a card-time
    /// copy would return the fleet to the day the card was made.
    #[test]
    fn a_file_the_application_already_has_is_kept_and_still_given_to_it() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, "photo-index", &manifest_text(|m| m));
        stage(&mut k, "photo-index", &[("register/register.toml", b"OLD")]);
        let data = service::data_dir(service::BUILT_IN_VOLUME, &name("photo-index"));
        let path = format!("{data}/register/register.toml");
        k.exists_mut()
            .insert(path.clone(), Script::always(Ok(true)));

        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);
        let id = defs[0].program.identity.expect("an identity");

        assert!(
            !k.writes.iter().any(|w| w.path == path),
            "a live file was overwritten from the card"
        );
        // Still chowned: the repair path for a delivery that wrote a file and
        // then failed before its chown.
        assert!(
            k.chowns.contains(&(path, id.uid, id.gid)),
            "{:?}",
            k.chowns
        );
        assert!(k.removed.contains(&staging::staged_bundle("photo-index")));
    }

    /// **A failure leaves the bundle where it is.** PID 1 has already destroyed
    /// the card's copy, so this is the last one and removing it after a failed
    /// delivery would close the window with nobody having seen it.
    #[test]
    fn a_delivery_that_fails_keeps_the_bundle_and_says_so_and_the_service_still_starts() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, "photo-index", &manifest_text(|m| m));
        stage(&mut k, "photo-index", &[("server.key", b"KEY")]);
        k.chown_results = Script::always(Err(Errno::EROFS));

        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        assert_eq!(defs.len(), 1, "the application was dropped over a delivery");
        assert!(
            !k.removed.contains(&staging::staged_bundle("photo-index")),
            "the last copy of a private key was deleted after a failed delivery"
        );
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Error && r.msg.contains("PARTLY delivered")),
            "{:?}",
            k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
    }

    /// A bundle addressed to somebody else delivers nothing. It cannot arise
    /// from a card this project wrote — PID 1 splits by application — and if it
    /// ever does, one application's private key in another's directory is the
    /// outcome worth refusing outright.
    #[test]
    fn a_bundle_naming_another_application_is_refused_whole() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, "photo-index", &manifest_text(|m| m));
        let items = vec![
            staging::Staged::new("rivet-fleetd", "server.key", b"KEY".to_vec()).expect("legal"),
        ];
        k.blobs.insert(
            staging::staged_bundle("photo-index"),
            staging::render(&items).into_bytes(),
        );

        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        let data = service::data_dir(service::BUILT_IN_VOLUME, &name("photo-index"));
        assert!(
            !k.writes.iter().any(|w| w.path.starts_with(&data)),
            "another application's material was delivered here"
        );
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Error && r.msg.starts_with("REFUSED")),
            "{:?}",
            k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
    }

    /// A bundle that will not read delivers nothing and is kept as evidence.
    #[test]
    fn a_bundle_that_will_not_read_delivers_nothing_and_is_kept() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, "photo-index", &manifest_text(|m| m));
        k.blobs.insert(
            staging::staged_bundle("photo-index"),
            b"version = 1\nthis is not three fields at all\n".to_vec(),
        );

        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        assert_eq!(defs.len(), 1);
        assert!(k.removed.is_empty(), "{:?}", k.removed);
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Error && r.msg.contains("will not read")),
            "{:?}",
            k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
    }

    /// **A parent directory the application replaced with a symlink is
    /// refused, not followed.** `data` is `0700` and owned by the
    /// application (`ADR-049`), so an application that has already run once
    /// can plant a link where `register` belongs before a second bundle is
    /// delivered into it. `mkdir_mode` and `chown` must never be reached for
    /// the planted path, and neither parent nor file beneath it may be
    /// touched either.
    #[test]
    fn a_parent_directory_replaced_with_a_symlink_is_refused_and_nothing_is_written() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, "photo-index", &manifest_text(|m| m));
        stage(
            &mut k,
            "photo-index",
            &[("register/devices/abc.toml", b"id = \"abc\"\n")],
        );
        let data = service::data_dir(service::BUILT_IN_VOLUME, &name("photo-index"));
        let planted = format!("{data}/register");
        k.symlinks.insert(planted.clone());

        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        assert_eq!(defs.len(), 1, "the application was dropped over a refusal");
        assert!(
            !k.mkdir_mode_calls.iter().any(|(p, _)| p == &planted),
            "mkdir_mode was called on the planted link itself: {:?}",
            k.mkdir_mode_calls
        );
        assert!(
            !k.chowns.iter().any(|(p, _, _)| p.starts_with(&planted)),
            "something at or under the planted link was chowned: {:?}",
            k.chowns
        );
        assert!(
            !k.writes
                .iter()
                .any(|w| w.path.starts_with(&format!("{data}/register/"))),
            "something was written past the planted link: {:?}",
            k.writes.iter().map(|w| &w.path).collect::<Vec<_>>()
        );
        assert!(
            k.records.iter().any(|r| r.sev == Severity::Error
                && r.msg.starts_with("DEGRADED")
                && r.fields.iter().any(|(key, v)| key == "path"
                    && matches!(v, Value::Str(s) if s == &planted))
                && r.fields.iter().any(|(key, v)| key == "detail"
                    && matches!(v, Value::Str(s) if s.contains("symbolic link")))),
            "the refusal was not journaled against the planted path: {:?}",
            k.records
        );
        assert!(
            !k.removed.contains(&staging::staged_bundle("photo-index")),
            "the bundle was removed after a refusal, losing the only copy of its material"
        );
    }

    /// **The item's own path, replaced with a symlink, is refused rather than
    /// followed into a write.** Whether the link resolves to something or to
    /// nothing, `write_file` would create or truncate the target as root;
    /// this must never be attempted.
    #[test]
    fn the_items_own_path_replaced_with_a_symlink_is_refused_and_nothing_is_written() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, "photo-index", &manifest_text(|m| m));
        stage(&mut k, "photo-index", &[("server.key", b"KEY")]);
        let data = service::data_dir(service::BUILT_IN_VOLUME, &name("photo-index"));
        let planted = format!("{data}/server.key");
        k.symlinks.insert(planted.clone());

        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);

        assert_eq!(defs.len(), 1, "the application was dropped over a refusal");
        assert!(
            !k.writes.iter().any(|w| w.path == planted),
            "the write reached the planted link: {:?}",
            k.writes.iter().map(|w| &w.path).collect::<Vec<_>>()
        );
        assert!(
            k.nofollow_writes.is_empty(),
            "the guarded write was even attempted against the planted link: {:?}",
            k.nofollow_writes
        );
        assert!(
            !k.chowns.iter().any(|(p, _, _)| p == &planted),
            "the planted link was chowned: {:?}",
            k.chowns
        );
        assert!(
            k.records.iter().any(|r| r.sev == Severity::Error
                && r.msg.starts_with("DEGRADED")
                && r.fields.iter().any(|(key, v)| key == "path"
                    && matches!(v, Value::Str(s) if s == &planted))
                && r.fields.iter().any(|(key, v)| key == "detail"
                    && matches!(v, Value::Str(s) if s.contains("symbolic link")))),
            "the refusal was not journaled against the planted path: {:?}",
            k.records
        );
        assert!(
            !k.removed.contains(&staging::staged_bundle("photo-index")),
            "the bundle was removed after a refusal, losing the only copy of its material"
        );
    }

    // -- ADR-060, an image service with a home -------------------------------

    const PROXY: &str = service::PROXYD_SERVICE_NAME;

    /// The image's own definitions with the proxy's `enabled` as given.
    fn image_services(proxy_enabled: bool) -> Vec<ServiceDef> {
        let mut defs = crate::definitions::system_services();
        for d in &mut defs {
            if d.name == PROXY {
                d.enabled = proxy_enabled;
            }
        }
        defs
    }

    /// A machine holding the proxy as the APPLICATION it used to be: a
    /// `current` link, one version directory, `tmp`, and `data` with its
    /// certificate — on a card whose image carries `/system/bin/rivet-proxyd`.
    fn carried_proxy_installed(k: &mut FakeKernel) {
        let dir = service::app_dir(service::BUILT_IN_VOLUME, &name(PROXY));
        k.exists_mut()
            .insert(service::PROXYD_PATH.to_string(), Script::always(Ok(true)));
        // Present on the first look and gone after it, as a real removal leaves
        // it — so a second boot can be asserted to say nothing.
        k.exists_mut().insert(
            service::current_dir(service::BUILT_IN_VOLUME, &name(PROXY)),
            Script::sequence(vec![Ok(true)], Ok(false)),
        );
        k.files_mut().insert(
            service::current_dir(service::BUILT_IN_VOLUME, &name(PROXY)),
            Script::always(Ok(String::new())),
        );
        k.dirs_mut()
            .insert(service::APP_ROOT.to_string(), vec![PROXY.to_string()]);
        k.dirs_mut().insert(
            dir.clone(),
            vec!["0.1.100".to_string(), "data".to_string(), "tmp".to_string()],
        );
        k.dirs_mut().insert(format!("{dir}/0.1.100"), Vec::new());
        k.dirs_mut().insert(format!("{dir}/tmp"), Vec::new());
        k.dirs_mut().insert(format!("{dir}/data"), Vec::new());
    }

    /// **D3: an installed proxy application is deleted on the first boot of
    /// an image that carries the proxy, and its data stays** — `ADR-060` §3.
    /// Said once, at `Info`, and nothing at `Error`.
    #[test]
    fn an_installed_proxy_application_is_retired_and_its_data_kept() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        carried_proxy_installed(&mut k);
        let dir = service::app_dir(service::BUILT_IN_VOLUME, &name(PROXY));

        retire_carried_image_services(&mut k, &mut j, service::BUILT_IN_VOLUME);

        assert!(
            k.removed.contains(&format!("{dir}/current")),
            "the current link was not removed: {:?}",
            k.removed
        );
        assert!(
            k.removed_paths().contains(&format!("{dir}/0.1.100")),
            "{:?}",
            k.removed_paths()
        );
        assert!(
            !k.removed_paths().iter().any(|t| t.ends_with("/data")),
            "the data directory the image's proxy reads was removed: {:?}",
            k.removed_paths()
        );
        assert!(k.dirs_mut().contains_key(&format!("{dir}/data")), "data is gone");

        let said: Vec<_> = k.records.iter().filter(|r| r.msg.contains("ADR-060")).collect();
        assert_eq!(said.len(), 1, "{:?}", k.records.iter().map(|r| &r.msg).collect::<Vec<_>>());
        assert_eq!(said[0].sev, Severity::Info);
        assert!(!k.records.iter().any(|r| r.sev == Severity::Error));

        // The next boot finds nothing to retire and says nothing about it.
        let before = k.records.len();
        retire_carried_image_services(&mut k, &mut j, service::BUILT_IN_VOLUME);
        assert_eq!(k.records.len(), before, "a second boot spoke about a finished migration");

        // And discovery neither starts the retired application nor complains
        // about the home it left behind.
        let mut defs = image_services(true);
        let count = defs.len();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);
        assert_eq!(defs.len(), count, "the retired application was discovered");
        assert!(
            !k.records.iter().any(|r| matches!(r.sev, Severity::Warn | Severity::Error)),
            "{:?}",
            k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
    }

    /// **A removed application awaiting deletion is not an application**, and
    /// not a name to complain about: it was removed when it was renamed aside
    /// (`ADR-017` §3), and a purge's helper may still be deleting it when
    /// discovery runs at the next install.
    #[test]
    fn a_removed_application_awaiting_deletion_is_passed_over_without_a_word() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        let aside = format!("{}1700-4", service::RECLAIM_MARK);
        k.dirs_mut()
            .insert(service::APP_ROOT.to_string(), vec![aside.clone()]);
        k.dirs_mut().insert(
            format!("{}/{aside}", service::APP_ROOT),
            vec!["data".to_string(), "0.1.0".to_string()],
        );
        let mut defs = Vec::new();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);
        assert!(defs.is_empty(), "{defs:?}");
        assert!(
            !k.records.iter().any(|r| r.sev >= Severity::Warn),
            "{:?}",
            k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
    }

    /// **Only an image that carries the program retires the application.** A
    /// card without it has no replacement, and deleting a machine's only front
    /// door is never the answer.
    #[test]
    fn an_image_without_the_proxy_retires_nothing() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        carried_proxy_installed(&mut k);
        k.exists_mut()
            .insert(service::PROXYD_PATH.to_string(), Script::always(Ok(false)));
        retire_carried_image_services(&mut k, &mut j, service::BUILT_IN_VOLUME);
        assert!(k.removed.is_empty(), "{:?}", k.removed);
        assert!(k.removed_paths().is_empty(), "{:?}", k.removed_paths());
    }

    /// **The proxy's home is made as an installer makes an application's**:
    /// the directory root-owned `0755`, `data` and `tmp` owned by its uid, and
    /// the card's staged certificate delivered into `data`, owned, and the
    /// bundle removed — `ADR-060` §2, `ADR-049`.
    #[test]
    fn the_image_proxy_is_given_its_home_and_its_staged_material() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        stage(&mut k, PROXY, &[("tls-cert", b"CERT"), ("tls-key", b"KEY")]);
        // A file the fake will let be removed, as PID 1 leaves it on /state.
        k.sizes_mut().insert(staging::staged_bundle(PROXY), 1);
        let defs = image_services(true);

        provide_image_homes(&mut k, &mut j, &defs);

        let volume = service::BUILT_IN_VOLUME;
        let id = service::app_identity(&name(PROXY));
        let dir = service::app_dir(volume, &name(PROXY));
        let data = service::data_dir(volume, &name(PROXY));
        let tmp = service::tmp_dir(volume, &name(PROXY));
        for (path, mode) in [
            (service::APP_ROOT.to_string(), 0o755),
            (dir.clone(), 0o755),
            (data.clone(), service::DATA_DIR_MODE),
            (tmp.clone(), service::TMP_DIR_MODE),
        ] {
            assert!(
                k.mkdir_mode_calls.contains(&(path.clone(), mode)),
                "{path} was not made {mode:o}: {:?}",
                k.mkdir_mode_calls
            );
        }
        assert!(
            !k.chowns.iter().any(|(p, _, _)| *p == dir),
            "the home itself was given away; the settings file and routing table sit in it and \
             must stay root's"
        );
        for path in [&data, &tmp] {
            assert!(
                k.chowns.contains(&(path.clone(), id.uid, id.gid)),
                "{path} is not the proxy's: {:?}",
                k.chowns
            );
        }
        for (file, want) in [("tls-cert", &b"CERT"[..]), ("tls-key", &b"KEY"[..])] {
            let path = format!("{data}/{file}");
            let written = k.writes.iter().find(|w| w.path == path).expect("delivered");
            assert_eq!(written.contents, want);
            assert!(k.chowns.contains(&(path, id.uid, id.gid)));
        }
        assert!(k.removed.contains(&staging::staged_bundle(PROXY)), "{:?}", k.removed);
        assert!(
            !k.records.iter().any(|r| r.sev == Severity::Error),
            "{:?}",
            k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
    }

    /// **A proxy the document leaves off gets no home**: nothing is made, and
    /// on a machine with no `/state` nothing fails.
    #[test]
    fn a_disabled_image_proxy_is_given_no_home() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        k.mkdir_mode_results = Script::always(Err(Errno::ENOENT));
        // The time client is a homed image service enabled by default, so it
        // is turned off here too: this is about the proxy's home alone.
        let mut defs = image_services(false);
        for d in &mut defs {
            if d.name == crate::definitions::TIMED_NAME {
                d.enabled = false;
            }
        }
        provide_image_homes(&mut k, &mut j, &defs);
        assert!(k.mkdir_mode_calls.is_empty(), "{:?}", k.mkdir_mode_calls);
        assert!(k.records.is_empty(), "{:?}", k.records);
    }

    /// **The proxy's home is not an application with no manifest.** It holds
    /// `data`, `tmp`, settings and routes and no `current`, which is its
    /// ordinary shape; discovery passes over it without a warning.
    #[test]
    fn the_image_proxys_home_is_not_warned_about_by_discovery() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        k.dirs_mut()
            .insert(service::APP_ROOT.to_string(), vec![PROXY.to_string()]);
        let mut defs = image_services(true);
        let count = defs.len();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);
        assert_eq!(defs.len(), count);
        assert!(
            !k.records.iter().any(|r| matches!(r.sev, Severity::Warn | Severity::Error)),
            "{:?}",
            k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
    }

    /// **The compositor's directory is passed over.** It keeps its screen
    /// capture at `<volume>/app/rivet-displayd/data/screen.qoi`, and that
    /// directory has no manifest by its ordinary shape.
    #[test]
    fn the_compositors_capture_directory_is_not_warned_about_by_discovery() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        k.dirs_mut().insert(
            service::APP_ROOT.to_string(),
            vec![rivet_core::compositor::PROVIDER.to_string()],
        );
        let mut defs = image_services(true);
        let count = defs.len();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);
        assert_eq!(defs.len(), count);
        assert!(
            !k.records.iter().any(|r| matches!(r.sev, Severity::Warn | Severity::Error)),
            "{:?}",
            k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
    }

    /// **But a proxy APPLICATION that is still installed is refused by name**,
    /// as any application shadowing an image service is — the migration
    /// above is what removes it, and discovery must not quietly start it.
    #[test]
    fn a_proxy_application_still_installed_is_refused_by_name() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        install(&mut k, PROXY, &manifest_text(|m| m));
        k.exists_mut().insert(
            service::manifest_path(service::BUILT_IN_VOLUME, &name(PROXY)),
            Script::always(Ok(true)),
        );
        let mut defs = image_services(true);
        let count = defs.len();
        discover(&mut k, &mut j, service::BUILT_IN_VOLUME, &mut defs);
        assert_eq!(defs.len(), count);
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Error && r.msg.contains("same name as a service")),
            "{:?}",
            k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
    }
}
