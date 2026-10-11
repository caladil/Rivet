//! **An application installed on a running machine starts on this machine.**
//! `PLAN.md` §19.2, 2026-08-25.
//!
//! # The defect this closes
//!
//! Five messages in three crates said the supervisor starts a newly installed
//! application *"on its next convergence"*, and until this module none of them
//! was true. `crate::main` calls `crate::applications::discover` **once**,
//! before the `Supervisor` is constructed, and `Supervisor::converge` decides
//! about the service list it was given — nothing ever added to it. An
//! application could be verified, unpacked and `current`-linked, and it waited
//! for a reboot.
//!
//! It survived §14 and §16 because both reinstalled an application and then
//! rebooted the board for unrelated reasons, so the application was running
//! afterwards and nobody asked why. `cargo rivet deploy` against a board with
//! no reason to reboot is what made it visible.
//!
//! # Why discovery is re-run rather than the installer being believed
//!
//! `cmd/rivet-deploy` is a separate process (the maintainer's 2026-08-24 ruling) and
//! its records are the installer's account of what it wrote. Taking a service
//! definition from that account would make the installer a **second** authority
//! for what an application is — the manifest on `/state` is the first, and
//! `ADR-017` §4.1 puts it there. So this notices only *that* an install
//! finished and then asks the filesystem the same question `crate::main` asks at
//! boot, through the same function, subject to the same four refusals: a name
//! that is not a name, a manifest that will not parse, a collision with a system
//! service, and a collision with another application's uid.
//!
//! The cost is that discovery runs a second time over applications it already
//! knows, and that is the intended shape rather than waste:
//! `crate::applications::provide_data_dir` converges and the reporting is a
//! fresh statement of the whole set at the moment it changed.
//!
//! # What it does NOT do, and this is the half that needs a ruling
//!
//! **An application that was already running is left alone.** If `deploy`
//! installs version 2 of something running version 1, `current` now points at
//! the new tree and the running process is still executing the old binary —
//! and this module says so, loudly, rather than acting.
//!
//! It does not act because every available action is a decision `ADR-017` §6
//! gives the operator: `enabled` in the document, and the `service-restart`
//! and `service-stop` operations. Stopping the service to pick up the new binary
//! would be a stop by another name, and whether it came back would then depend
//! on a `restart` policy the *artifact's author* wrote for the case of crashing
//! — `on-failure` would not restart it at all, so the machine would be left
//! with the application down and nothing having failed.
//!
//! `PLAN.md` §19.4 is the written question. Until it is ruled, the honest
//! behaviour is the loud one: the record names the version that is installed,
//! the version that is running, and what to do about it.

use rivet_core::deploy;
use rivet_core::service::BUILT_IN_VOLUME;

use crate::journal::{Severity, Value};
use crate::kernel::Kernel;
use crate::service::{Origin, ServiceDef};
use crate::supervise::{State, Supervisor};

/// Watches the installer for a completed application install.
pub struct Watch {
    /// The correlation of the finished installer instance this last acted on,
    /// and `None` before it has acted on any.
    ///
    /// # It was a `bool`, and the second install of a boot was invisible
    ///
    /// **Corrected 2026-09-01, `PLAN.md` §105.3.** This recorded only *whether*
    /// the installer was finished, and fired on the rising edge of that. The
    /// reasoning was sound as far as it went: `State::Done` persists —
    /// `Supervisor::adopt` leaves a finished instance in place until a *second*
    /// request replaces it — so a level check would re-run discovery on every
    /// convergence pass for the rest of the boot.
    ///
    /// What it missed is that **a boolean cannot tell a new finished instance
    /// from the old one still sitting there.** After one install the flag is
    /// `Some(true)` for the rest of the boot; a second install that is adopted
    /// and finishes *between two convergence passes* leaves it `Some(true)` at
    /// the next poll too, and the rising edge never happens. Discovery does not
    /// run, the supervisor keeps the previous manifest, and
    /// [`Watch::look`]'s whole purpose is skipped in silence.
    ///
    /// Found by deploying the same application twice on 2026-09-01: the second
    /// payload installed successfully and the supervisor went on starting the
    /// service with the **first** payload's `listen`, reporting the stale port
    /// in the journal as though it were current.
    ///
    /// [`Managed::corr`](crate::supervise::Managed::corr) is *"re-minted for
    /// each attempt"*, so it is the identity a boolean was standing in for.
    /// `rivet get install` already tells an operator to watch the correlation
    /// and **not** `finished` — `PLAN.md` §73.2 records that polling `finished`
    /// alone had already cost this project a deploy cycle once. The tool
    /// learned that lesson; this watch had not.
    ///
    /// The two properties the `bool` was chosen for still hold. A boot with no
    /// install never reaches a finished installer, so nothing fires; and an
    /// installer that came and went inside one pass is caught, because its
    /// correlation is one this has not acted on.
    acted_on: Option<String>,
}

impl Watch {
    pub fn new() -> Self {
        Self { acted_on: None }
    }

    /// Called on every convergence pass, beside the two request watches.
    pub fn poll(&mut self, k: &mut dyn Kernel, sup: &mut Supervisor) {
        // The installer's own record of THIS run. See `acted_on` for why a
        // boolean here made the second install of a boot invisible.
        let Some(installer) = sup
            .services()
            .iter()
            .find(|m| m.def.name == deploy::SERVICE_NAME)
        else {
            // No instance means no install this boot.
            return;
        };
        if !matches!(installer.state, State::Done | State::Failed) {
            return;
        }
        let corr = installer.corr.clone();

        // Finished, and not the instance this already acted on. A second
        // install that was adopted and finished between two passes has a new
        // correlation and is caught here; the same finished instance sitting
        // there for the rest of the boot is not.
        if self.acted_on.as_deref() == Some(corr.as_str()) {
            return;
        }
        self.acted_on = Some(corr);
        self.look(k, sup);
    }

    /// Ask the filesystem what is installed, and adopt whatever is new.
    fn look(&mut self, k: &mut dyn Kernel, sup: &mut Supervisor) {
        // **Before discovery, and the order is the whole of it** —
        // `.rulings/` R-202, `SPEC-02` §12. A library's identity is the digest
        // of its bytes and Rust builds are not byte-reproducible, so a rebuilt
        // addon installs BESIDE its own older build; two libraries of one
        // class is what R-200 makes a host refuse, and the card that addon
        // feeds goes dark on the upgrade. Retention keeps one build per
        // product, never evicting one an installed application names in its
        // declared closure.
        //
        // It runs here rather than at boot because an install is the operator
        // asking for something — this module's own argument for why discovery
        // re-runs at this edge and nowhere else — and it runs *before*
        // `discover` so that the `RIVET_ADDONS` and `LD_LIBRARY_PATH` that
        // discovery composes describe the set that is left rather than the one
        // about to change.
        crate::retention::sweep(k, sup.journal_mut(), BUILT_IN_VOLUME);

        rediscover(k, sup);
    }
}

/// **Discovery, re-run on a machine that is running** — the half of the install
/// edge that does not depend on there having been an install.
///
/// Asks the filesystem what is installed and what each library provides,
/// composes every application's definition afresh — `RIVET_ADDONS` and
/// `LD_LIBRARY_PATH` included — takes the document's answer, and adopts or
/// reconsiders every application, exactly as [`Watch`] does after an install.
/// `crate::uninstall_library_request` calls it after removing a library on
/// purpose, so that the library set every service is given describes what is
/// left without a reboot. **Retention is not part of it**: that sweep is about
/// an install having added a build, and a removal added nothing.
///
/// **Nothing running is restarted**, for the reason this module's header gives:
/// a running process keeps the environment it was exec'd with, and whether to
/// restart it is the operator's.
pub fn rediscover(k: &mut dyn Kernel, sup: &mut Supervisor) {
    // The definitions the supervisor already has, so that discovery's
    // collision checks see them — a newly installed application that
    // shadows a service the image defines, or that derives a uid another
    // application already has, is refused here exactly as it is at boot.
    let known: Vec<String> = sup.services().iter().map(|m| m.def.name.clone()).collect();
    let mut defs: Vec<ServiceDef> = sup.services().iter().map(|m| m.def.clone()).collect();

    crate::applications::discover(k, sup.journal_mut(), BUILT_IN_VOLUME, &mut defs);

    // ~~**The document is read again, here, and this is the only moment in
    // a boot that it is.** `crate::main` reads it once before the supervisor
    // exists; nothing else re-reads it, so a configuration change does not
    // otherwise reach a running machine at all (`PLAN.md` §20.3, which is
    // the larger question this deliberately does not answer).~~
    //
    // **Struck 2026-08-26 — that larger question is answered.**
    // `crate::reload` re-reads it on `SIGHUP` (`ADR-032` §3.2), so this is
    // no longer the only moment. The rest of the paragraph stands, and the
    // two readers apply DIFFERENT amounts of the document:
    // `crate::document::Occasion::Boot` here, because an install edge is
    // not the operator asking for their whole document to be taken, and
    // `Occasion::Reload` there. Corrected where it stands rather than
    // deleted — `AGENTS.md` §7.
    //
    // Re-reading at *this* moment is the narrow thing rather than the
    // general one. An install is a discrete event at which the machine
    // already re-examines what is on the card, and an operator who is
    // installing an application is the operator most likely to have just
    // said whether it should run — `cargo rivet deploy --enable` writes
    // that line immediately before sending the payload, precisely because
    // of this. Holding the boot-time copy instead would mean the line was
    // written, accepted, and ignored until the next restart.
    let desired = crate::document::load(k, sup.journal_mut());
    crate::document::apply_desired_state(k, sup.journal_mut(), &mut defs, &desired);

    // **A fleet-root machine issues an installed application its
    // certificate here** (`ADR-063` §4), after discovery has made its data
    // directory and before it is adopted, so it is never held waiting for
    // material this machine was always going to issue. A running
    // application keeps the leaf it started with until its next start.
    let running: Vec<String> = sup
        .services()
        .iter()
        .filter(|m| matches!(m.state, State::Started { .. } | State::Adopted { .. }))
        .map(|m| m.def.name.clone())
        .collect();
    crate::issuance::converge(
        k,
        sup.journal_mut(),
        &defs,
        crate::issuance::Occasion::Install,
        &running,
    );

    // Said before anything is adopted or re-enabled, because it is about
    // what is running NOW against what has just been written to the card.
    report_replaced_versions(k, sup, &defs);

    for def in defs {
        if known.iter().any(|name| name == &def.name) {
            reconsider(k, sup, &def);
            continue;
        }
        adopt_new(k, sup, def);
    }

    // **The convergence half of `ADR-038` §8.1** — `PLAN.md` §134.3 named
    // this gap rather than closing it: `crate::reload` hands every
    // installed application its settings on `SIGHUP`, and `crate::main`
    // does the same once at boot, but an application installed and
    // enabled on a RUNNING machine started here, through `adopt_new`
    // above, with no settings file ever written for it. It read nothing
    // until an operator separately sent a reload — a step nothing told
    // them to take, because installing is supposed to be enough
    // (`PLAN.md` §19.2's whole argument, now missing one delivery).
    //
    // **Not "every convergence pass" — `Watch::poll` already isn't
    // that.** `runtime::declared_settings`'s own doc comment argues why a
    // read of every installed manifest on every pass would be wasteful,
    // and this method only reaches here once per finished install
    // (`acted_on`'s guard), which is the same "an event happened, not a
    // clock ticked" shape `crate::reload` already has. `ADR-038` §8.1's
    // sentence is corrected to name this event rather than "every
    // convergence", because the literal reading is the one thing this
    // deliberately does not do.
    //
    // **The same `!Unreadable` guard `crate::reload` uses, and for the
    // same reason.** `desired.document()` answers a default, empty
    // document when the store could not be read, and delivering that
    // would WRITE OVER every application's settings with nothing — the
    // one case worse than not delivering at all, since it turns a read
    // failure into a write.
    if !matches!(desired, crate::document::Desired::Unreadable { .. }) {
        let declared = crate::runtime::declared_settings(k, sup.services());
        crate::runtime::deliver_settings(k, sup.journal_mut(), &desired.document(), &declared);
        // The name service's projection, from the same list (`ADR-047` §6).
        let machine_name = sup.machine_name().map(str::to_string);
        crate::runtime::deliver_name_service_document(
            k,
            sup.journal_mut(),
            &desired.document(),
            machine_name.as_deref(),
            &declared,
        );
        // And the mDNS responder's, from the same list (`ADR-017` §3).
        let mdns_inputs = sup.mdns_inputs().cloned();
        crate::runtime::deliver_mdns_projection(
            k,
            sup.journal_mut(),
            mdns_inputs.as_ref(),
            &declared,
        );
    }

    // **And the routing table** — `ADR-046` §5, and the same gap the
    // paragraph above closes for settings: an application installed on a
    // running machine that declares a `proxy-path` would otherwise be
    // unreachable through the proxy until the next boot, with nothing
    // saying so.
    //
    // **Outside the `!Unreadable` guard, deliberately.** Every column of
    // that table comes from a signed manifest and none of it from the
    // document, so a document the store could not read says nothing about
    // what is routed — where delivering an empty document would have
    // erased every setting on the machine, deriving routes from manifests
    // is the same answer either way.
    let applications = crate::runtime::installed_applications(sup.services());
    let plain = crate::runtime::publish_routes(k, sup.journal_mut(), &applications);
    // **`rivet-proxyd` may already be running here, unlike `main.rs`'s
    // first call** — `ADR-046` §7, `ADR-061` §7. `reconcile_proxy_listen`
    // stores the new listen set either way and journals a warning naming
    // the restart command when it is, rather than restarting on its own:
    // `ADR-055` §6 leaves whether an install may restart a service
    // nobody asked it to as the maintainer's open question.
    crate::runtime::reconcile_proxy_listen(k, sup, plain);
    // **And the firewall over the routed ports** — `ADR-023` §9. A newly
    // installed proxied application's port is guarded from this pass on;
    // an install that changes no routed port leaves the table alone.
    crate::runtime::reconcile_firewall(k, sup);
    // **And what those applications offer to back up** — `ADR-051`, beside
    // the routing table for the same reason and derived from the same
    // manifests. An application installed on a running machine that
    // declares a backup surface would otherwise be invisible to a backup
    // client until the next boot.
    crate::runtime::publish_backups(k, sup.journal_mut(), &applications);

    // **A-2's fix, and it must run last.** `reconsider` above just took
    // the document's `enabled` for every application this supervisor
    // already knows, including any display application the kiosk gate had
    // previously narrowed away — the document can go on saying more than
    // one is enabled; the gate, not the document, is what narrows that to
    // the one selection. `adopt_new` leaves a freshly installed display
    // application ungated the same way, and absent from the offered set
    // besides. `refresh_display_offered` re-reads the offered set from
    // the service list this loop just changed and re-applies the gate
    // over it — the same order `Supervisor::new` and `crate::reload` both
    // use — so the convergence this pass ends with starts only the
    // selected application, and a newly installed one is selectable
    // without waiting for a reload.
    sup.refresh_display_offered();
}

/// An application this supervisor already knows, with the document's answer
/// applied to a fresh reading of it.
///
/// **This is the case `--enable` actually hits**, and it was missed the
/// first time: after `ADR-015` §3.2.2 every installed application is
/// disabled until the document says otherwise, so the *usual* state at an
/// install edge is a definition the supervisor already holds, sitting idle.
/// Discovery refuses to add a second definition for a name it already has —
/// correctly — so nothing new appears and nothing would have happened.
///
/// `Supervisor::set_application_enabled` carries the argument for why this
/// reaches applications and never the image's own services.
fn reconsider(k: &mut dyn Kernel, sup: &mut Supervisor, def: &ServiceDef) {
    if def.origin != Origin::Application {
        return;
    }

    // **The program, from the manifest that is on the card now** — Shared
    // Rivets, 2026-08-31. See `Supervisor::take_application_definition` for
    // the defect: until that day this function took `enabled` out of a
    // fresh reading and threw the rest of it away, which was harmless only
    // while nothing else in a manifest reached the spawn. Two things now do
    // — the declared closure and `listen` — and the second was found four
    // hours after the first, in the same shape, because the first fix took
    // one field instead of the rule.
    if sup.take_application_definition(&def.name, def) {
        sup.note(
            k,
            Severity::Info,
            "this application's manifest changed something this supervisor acts on, and it \
             has taken the new reading. NOTHING WAS RESTARTED: a running process keeps the \
             environment and the descriptors it was exec'd with and cannot be given others, \
             so the change applies at its next start — a crash restart, `rivet apply \
             service-restart`, or the next boot. Whether an install should restart a running \
             application is PLAN.md §19.4 and is the operator's",
            vec![
                ("application".to_string(), Value::Str(def.name.clone())),
                (
                    "library_path".to_string(),
                    Value::Str(
                        def.program
                            .env
                            .iter()
                            .find(|(key, _)| key == "LD_LIBRARY_PATH")
                            .map(|(_, value)| value.clone())
                            .unwrap_or_else(|| "none".to_string()),
                    ),
                ),
                (
                    "listen".to_string(),
                    Value::Str(match def.listen.is_empty() {
                        true => "none".to_string(),
                        false => crate::applications::render_sockets(&def.listen),
                    }),
                ),
            ],
        );
    }

    if !sup.set_application_enabled(&def.name, def.enabled) {
        return;
    }
    sup.note(
        k,
        Severity::Info,
        if def.enabled {
            "the document now says this installed application should run, and this \
             supervisor has taken that answer. The next convergence starts it — no restart \
             is needed (PLAN.md §20)"
        } else {
            "the document now says this installed application should NOT run, and this \
             supervisor has taken that answer. The next convergence stops it — `enabled` is \
             the operator's whole control surface (ADR-017 §6)"
        },
        vec![
            ("application".to_string(), Value::Str(def.name.clone())),
            ("enabled".to_string(), Value::Bool(def.enabled)),
        ],
    );
}

/// An application this supervisor has never seen.
fn adopt_new(k: &mut dyn Kernel, sup: &mut Supervisor, def: ServiceDef) {
    let name = def.name.clone();
    let enabled = def.enabled;
    match sup.adopt(def) {
        Ok(()) => sup.note(
            k,
            Severity::Info,
            if enabled {
                "an application was installed while this machine was running, and it is now \
                 a service this supervisor knows. The next convergence starts it — no \
                 restart is needed (PLAN.md §19.2)"
            } else {
                "an application was installed while this machine was running and this \
                 supervisor now knows it, but the document does not enable it, so it is NOT \
                 started. Installing is not running (ADR-015 §3.2.2), and `enabled` is the \
                 operator's whole control surface (ADR-017 §6)"
            },
            vec![
                ("application".to_string(), Value::Str(name)),
                ("enabled".to_string(), Value::Bool(enabled)),
            ],
        ),
        Err(e) => sup.note(
            k,
            Severity::Error,
            "DEGRADED an application was installed and this supervisor will NOT run it \
             until the machine restarts. Reported rather than forced: the definition was \
             refused, and a supervisor that overrode its own refusal would be the one place \
             a bad definition could not be caught",
            vec![
                ("application".to_string(), Value::Str(name)),
                ("detail".to_string(), Value::Str(e.to_string())),
            ],
        ),
    }
}

/// Say when the installed program is not the running one.
///
/// See this module's header: acting on it is `ADR-017` §6's territory and
/// `PLAN.md` §19.4 is the question. Saying nothing is the one option that is
/// certainly wrong — a machine whose running program is not its installed
/// program, silently, is the divergence this whole project refuses.
fn report_replaced_versions(
    k: &mut dyn Kernel,
    sup: &mut Supervisor,
    defs: &[ServiceDef],
) {
    let running: Vec<(String, String)> = sup
        .services()
        .iter()
        .filter(|m| m.def.origin == Origin::Application)
        .filter(|m| {
            matches!(
                m.state,
                State::Started { .. } | State::Adopted { .. } | State::Stopping { .. }
            )
        })
        .filter_map(|m| {
            // The program path is derived from the name and the volume, so
            // two definitions of one application always agree about it —
            // what can differ is what `current` now points at, which is not
            // in the definition at all. So the comparison that is available
            // here is of the manifest, and a changed one is the signal that
            // something under `current` moved.
            let found = defs.iter().find(|d| d.name == m.def.name)?;
            (found.program != m.def.program
                || found.readiness != m.def.readiness
                || found.requested_memory != m.def.requested_memory
                || found.requested_cpu != m.def.requested_cpu)
                .then(|| (m.def.name.clone(), found.consequence.clone()))
        })
        .collect();

    for (name, _) in running {
        sup.note(
            k,
            Severity::Warn,
            "an installed application CHANGED while it was running, so the program this \
             machine is executing is NOT the program on the card. Nothing was stopped: \
             ADR-017 §6 has no manual restart and stopping it here would depend on a \
             `restart` policy the artifact's author wrote for the case of crashing — \
             `on-failure` would leave it down. Restart the machine, or toggle `enabled` \
             in the document (PLAN.md §19.4 is the open question)",
            vec![("application".to_string(), Value::Str(name))],
        );
    }
}


#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::journal::Journal;
    use crate::kernel::ExitStatus;
    use crate::service::{IMAGE_DEFINED, Limits, RestartPolicy};
    use crate::supervise::ACTOR;
    use crate::testkernel::{FakeKernel, Script};
    use rivet_core::service::{self, app_identity};

    fn journal() -> Journal {
        Journal::new("test-boot".to_string(), 1, ACTOR.to_string())
    }

    fn name(text: &str) -> rivet_core::service::ServiceName {
        rivet_core::service::ServiceName::new(text).expect("a valid fixture name")
    }

    fn supervisor(defs: Vec<ServiceDef>) -> Supervisor {
        match Supervisor::new(defs, Limits::default(), journal()) {
            Ok(s) => s,
            Err(e) => unreachable!("fixture refused: {e}"),
        }
    }

    /// The installer's definition, as `crate::definitions::deploy` builds one:
    /// image-defined, enabled, and run once.
    fn installer_def() -> ServiceDef {
        ServiceDef {
            name: deploy::SERVICE_NAME.to_string(),
            program: crate::kernel::Program {
                path: "/system/bin/rivet-deploy".to_string(),
                args: vec!["/state/x.rvt".to_string()],
                env: Vec::new(),
                identity: None,
                confinement: None,
                groups: Vec::new(),
                capabilities: Vec::new(),
            },
            enabled: true,
            restart: RestartPolicy::Never,
            consequence: "the fixture would lose the install".to_string(),
            ..IMAGE_DEFINED
        }
    }

    /// The manifest a packaged daemon carries, rendered by the type that also
    /// reads it — so this fixture cannot drift from the parser.
    fn manifest_text() -> String {
        rivet_core::package::application::Manifest::new(
            rivet_core::package::application::ProgramKind::Daemon,
        )
        .render()
    }

    /// Put an application on the fake card, exactly as `crate::applications`
    /// own tests do.
    fn install(k: &mut FakeKernel, app: &str) {
        k.dirs_mut()
            .entry(service::app_root(service::BUILT_IN_VOLUME))
            .or_default()
            .push(app.to_string());
        k.files_mut().insert(
            service::manifest_path(service::BUILT_IN_VOLUME, &name(app)),
            Script::always(Ok(manifest_text())),
        );
    }

    /// The same, with a manifest that DECLARES one setting — `ADR-038` §4.3's
    /// *complete on the machine* validation means an application declaring
    /// nothing is handed nothing, which would make the settings test below
    /// pass for the wrong reason (`crate::reload`'s own fixture carries the
    /// same comment).
    fn install_with_settings(k: &mut FakeKernel, app: &str, setting: &str) {
        let text = rivet_core::package::application::Manifest::new(
            rivet_core::package::application::ProgramKind::Daemon,
        )
        .with_settings(vec![setting.to_string()])
        .expect("a valid setting name")
        .render();

        k.dirs_mut()
            .entry(service::app_root(service::BUILT_IN_VOLUME))
            .or_default()
            .push(app.to_string());
        k.files_mut().insert(
            service::manifest_path(service::BUILT_IN_VOLUME, &name(app)),
            Script::always(Ok(text)),
        );
    }

    /// Plant the document the fake card is storing, the way `crate::reload`'s
    /// own fixture does — `rivet_store::ConfigStore::at_default().machine_path()`
    /// is where `crate::document::load` reads it from.
    fn store_document(k: &mut FakeKernel, stored: &str) {
        k.files_mut().insert(
            rivet_store::ConfigStore::at_default().machine_path(),
            Script::always(Ok(stored.to_string())),
        );
    }

    /// The same, with a `listen` — so a second install can differ from the
    /// first in a field the supervisor acts on.
    ///
    /// The directory entry is pushed only once: discovery reads it as the list
    /// of installed applications, and a name appearing twice would be testing
    /// a card this machine cannot produce.
    fn install_listening(k: &mut FakeKernel, app: &str, port: u16) {
        let text = rivet_core::package::application::Manifest::new(
            rivet_core::package::application::ProgramKind::Daemon,
        )
        .with_listen(vec![rivet_core::package::application::Socket::Tcp(port)])
        .expect("a tcp port is a valid listen")
        .render();

        let root = k
            .dirs_mut()
            .entry(service::app_root(service::BUILT_IN_VOLUME))
            .or_default();
        if !root.iter().any(|e| e == app) {
            root.push(app.to_string());
        }
        k.files_mut().insert(
            service::manifest_path(service::BUILT_IN_VOLUME, &name(app)),
            Script::always(Ok(text)),
        );
    }

    /// The same, with a `connects` — the field the enumerated copy dropped.
    fn install_connecting(
        k: &mut FakeKernel,
        app: &str,
        port: u16,
        provider: &str,
        endpoint: &str,
    ) {
        let wanted = rivet_core::package::application::Endpoint::parse(
            "connects",
            &format!("{provider}:{endpoint}"),
        )
        .expect("owner:name is a valid endpoint");
        let text = rivet_core::package::application::Manifest::new(
            rivet_core::package::application::ProgramKind::Daemon,
        )
        .with_listen(vec![rivet_core::package::application::Socket::Tcp(port)])
        .expect("a tcp port is a valid listen")
        .with_connects(vec![wanted])
        .expect("one endpoint is a valid connects")
        .render();

        let root = k
            .dirs_mut()
            .entry(service::app_root(service::BUILT_IN_VOLUME))
            .or_default();
        if !root.iter().any(|e| e == app) {
            root.push(app.to_string());
        }
        k.files_mut().insert(
            service::manifest_path(service::BUILT_IN_VOLUME, &name(app)),
            Script::always(Ok(text)),
        );
    }

    /// What the supervisor currently believes this application connects to.
    fn connects_of(
        sup: &Supervisor,
        app: &str,
    ) -> Vec<rivet_core::package::application::Endpoint> {
        sup.services()
            .iter()
            .find(|m| m.def.name == app)
            .map(|m| m.def.connects.clone())
            .unwrap_or_default()
    }

    /// What the supervisor currently believes this application listens on.
    fn listen_of(
        sup: &Supervisor,
        app: &str,
    ) -> Vec<rivet_core::package::application::Socket> {
        sup.services()
            .iter()
            .find(|m| m.def.name == app)
            .map(|m| m.def.listen.clone())
            .unwrap_or_default()
    }

    /// Run the installer to completion through the real machinery: adopt it,
    /// converge so it spawns, then hand back the exit.
    ///
    /// **Not a state forced into the table.** The edge this module watches is
    /// produced by `converge` and `reap`, so a fixture that assigned
    /// `State::Done` directly would be testing an edge the supervisor never
    /// produces.
    fn run_the_installer(k: &mut FakeKernel, sup: &mut Supervisor, watch: &mut Watch) {
        sup.adopt(installer_def()).expect("the installer is adoptable");
        sup.converge(k);
        watch.poll(k, sup);
        let Some(State::Started { pid, .. }) = sup.state_of(deploy::SERVICE_NAME).cloned() else {
            unreachable!(
                "the installer did not start: {:?}",
                sup.state_of(deploy::SERVICE_NAME)
            );
        };
        k.exits.push((pid, ExitStatus::Exited(0)));
        sup.reap(k);
        sup.converge(k);
    }

    /// **The whole of the defect, from the outside.** An application appears on
    /// the card while the machine is running; the installer finishes; the
    /// supervisor knows about it, with no reboot.
    #[test]
    fn an_application_installed_while_the_machine_runs_becomes_a_service_it_knows() {
        let mut k = FakeKernel::new();
        let mut sup = supervisor(Vec::new());
        let mut watch = Watch::new();

        install(&mut k, "photo-index");
        run_the_installer(&mut k, &mut sup, &mut watch);
        assert!(
            sup.state_of("photo-index").is_none(),
            "it discovered before the installer had finished"
        );

        watch.poll(&mut k, &mut sup);
        assert!(
            sup.state_of("photo-index").is_some(),
            "the application is still not a service this supervisor knows"
        );
    }

    /// **`ADR-038` §8.1's convergence half** — `PLAN.md` §134.3 named this
    /// gap: `crate::reload` hands an already-known application its settings,
    /// but one installed and enabled on a running machine had never received
    /// any, because `deliver_settings` had no caller on this path at all.
    ///
    /// The document already carries the value BEFORE the install finishes —
    /// exactly `cargo rivet deploy --enable`'s shape, an operator who wrote
    /// the setting and then sent the payload — so a delivery that only fired
    /// on the NEXT reload would still leave a first start with nothing.
    #[test]
    fn an_application_installed_while_the_machine_runs_is_handed_its_settings() {
        let mut k = FakeKernel::new();
        store_document(&mut k, "version = 1\n[applications.photo-index]\nrotate-seconds = \"30\"\n");
        let mut sup = supervisor(Vec::new());
        let mut watch = Watch::new();

        install_with_settings(&mut k, "photo-index", "rotate-seconds");
        run_the_installer(&mut k, &mut sup, &mut watch);
        watch.poll(&mut k, &mut sup);

        let want =
            rivet_core::settings::path(service::BUILT_IN_VOLUME, &name("photo-index"));
        let written = k
            .writes
            .iter()
            .find(|w| w.path == want)
            .map(|w| String::from_utf8(w.contents.clone()).expect("utf-8"))
            .unwrap_or_else(|| {
                panic!("an install wrote no settings file for a newly adopted application")
            });
        assert!(
            written.contains("rotate-seconds") && written.contains("30"),
            "the settings file was written without the document's value: {written:?}"
        );
    }

    /// Put an application declaring `fleet-tls` on the fake card.
    fn install_with_fleet_tls(k: &mut FakeKernel, app: &str) {
        let text = rivet_core::package::application::Manifest::new(
            rivet_core::package::application::ProgramKind::Daemon,
        )
        .with_material(vec![rivet_core::package::application::Material::FleetTls])
        .render();
        k.dirs_mut()
            .entry(service::app_root(service::BUILT_IN_VOLUME))
            .or_default()
            .push(app.to_string());
        k.files_mut().insert(
            service::manifest_path(service::BUILT_IN_VOLUME, &name(app)),
            Script::always(Ok(text)),
        );
    }

    /// Make the fake a fleet-root machine: a root minted by this test
    /// (self-generated randomness) resting in `/state/secrets`.
    fn hold_a_fleet_root(k: &mut FakeKernel) {
        let request = rivet_core::fleet_root::Request::new(
            "dev-fleet",
            "dev-01",
            &["local".to_string()],
        )
        .expect("a legal request");
        let mut next = 5u8;
        let mut counting = |len: usize| -> Result<Vec<u8>, String> {
            Ok((0..len)
                .map(|_| {
                    next = next.wrapping_add(31);
                    next
                })
                .collect())
        };
        let minted = rivet_fleet_root::mint(&request, &mut counting, 0).expect("mints");
        let root = rivet_core::fleet_root::root_certificate_path();
        k.files_mut()
            .insert(root.clone(), Script::always(Ok(minted.root.certificate)));
        k.exists_mut().insert(root, Script::always(Ok(true)));
        k.files_mut().insert(
            format!(
                "{}/{}",
                rivet_core::state::SECRETS,
                rivet_core::fleet_root::MINTED_FOR_FILE
            ),
            Script::always(Ok(request.render())),
        );
    }

    /// **On a fleet-root machine an install issues the application its
    /// certificate before the application is adopted**, into its own data
    /// directory — so it is never held waiting for material.
    #[test]
    fn an_install_on_a_fleet_root_machine_issues_the_application_its_certificate() {
        let mut k = FakeKernel::new();
        hold_a_fleet_root(&mut k);
        let mut sup = supervisor(Vec::new());
        let mut watch = Watch::new();

        install_with_fleet_tls(&mut k, "aura-controller");
        run_the_installer(&mut k, &mut sup, &mut watch);
        watch.poll(&mut k, &mut sup);

        assert_eq!(k.issues.len(), 1, "{:?}", k.records);
        assert_eq!(
            k.issues[0].targets[0].dir,
            service::data_dir(service::BUILT_IN_VOLUME, &name("aura-controller"))
        );
        assert_eq!(k.issues[0].names, ["dev-01.local"]);
        assert!(sup.state_of("aura-controller").is_some());
        assert!(!k.reads.contains(&rivet_core::fleet_root::root_key_path()));
    }

    /// **On a machine whose root is a workstation's, an install issues
    /// nothing**: `cargo rivet secrets` stages its material, as it always has.
    #[test]
    fn an_install_on_a_workstation_root_machine_issues_nothing() {
        let mut k = FakeKernel::new();
        let mut sup = supervisor(Vec::new());
        let mut watch = Watch::new();

        install_with_fleet_tls(&mut k, "aura-controller");
        run_the_installer(&mut k, &mut sup, &mut watch);
        watch.poll(&mut k, &mut sup);

        assert!(k.issues.is_empty(), "{:?}", k.issues);
        assert!(sup.state_of("aura-controller").is_some());
    }

    /// The edge and not the level: a finished installer that stays finished
    /// must not re-run discovery for the rest of the boot.
    #[test]
    fn a_finished_installer_is_looked_at_once_and_not_on_every_pass() {
        let mut k = FakeKernel::new();
        let mut sup = supervisor(Vec::new());
        let mut watch = Watch::new();

        install(&mut k, "photo-index");
        run_the_installer(&mut k, &mut sup, &mut watch);
        watch.poll(&mut k, &mut sup);
        let after_the_edge = k.records.len();

        watch.poll(&mut k, &mut sup);
        watch.poll(&mut k, &mut sup);
        assert_eq!(
            k.records.len(),
            after_the_edge,
            "it re-ran discovery on a pass where nothing had happened"
        );
    }

    /// A boot on which no install ever happened must not fire discovery. The
    /// watch starts believing the installer is up for exactly this reason.
    #[test]
    fn a_watch_that_never_saw_the_installer_run_does_not_fire() {
        let mut k = FakeKernel::new();
        install(&mut k, "photo-index");
        let mut sup = supervisor(Vec::new());
        let mut watch = Watch::new();

        watch.poll(&mut k, &mut sup);
        watch.poll(&mut k, &mut sup);
        assert!(
            sup.state_of("photo-index").is_none(),
            "it discovered on a boot where no install happened"
        );
    }

    /// An application whose name a system service already has is refused by
    /// `crate::applications`, and this module must not put it back.
    #[test]
    fn an_application_that_collides_with_a_system_service_is_still_refused() {
        let mut k = FakeKernel::new();
        let system = ServiceDef {
            name: "photo-index".to_string(),
            program: crate::kernel::Program {
                path: "/system/bin/photo-index".to_string(),
                args: Vec::new(),
                env: Vec::new(),
                identity: None,
                confinement: None,
                groups: Vec::new(),
                capabilities: Vec::new(),
            },
            enabled: false,
            consequence: "the fixture's own service".to_string(),
            ..IMAGE_DEFINED
        };
        let mut sup = supervisor(vec![system]);
        let mut watch = Watch::new();

        install(&mut k, "photo-index");
        run_the_installer(&mut k, &mut sup, &mut watch);
        watch.poll(&mut k, &mut sup);

        let found = sup
            .services()
            .iter()
            .filter(|m| m.def.name == "photo-index")
            .count();
        assert_eq!(found, 1, "one name now has two definitions");
        assert_eq!(
            sup.services()
                .iter()
                .find(|m| m.def.name == "photo-index")
                .map(|m| m.def.origin),
            Some(Origin::Image),
            "the application displaced the system service"
        );
    }

    /// Everything a discovered application needs in order to be startable
    /// arrives with it — the identity `ServiceDef::validate` refuses an
    /// application without, and the volume `PLAN.md` §18.5 made every path a
    /// function of.
    #[test]
    fn a_discovered_application_arrives_with_its_identity_and_its_volume() {
        let mut k = FakeKernel::new();
        let mut sup = supervisor(Vec::new());
        let mut watch = Watch::new();

        install(&mut k, "photo-index");
        run_the_installer(&mut k, &mut sup, &mut watch);
        watch.poll(&mut k, &mut sup);

        let def = sup
            .services()
            .iter()
            .find(|m| m.def.name == "photo-index")
            .map(|m| m.def.clone())
            .expect("discovered");
        assert_eq!(def.origin, Origin::Application);
        assert_eq!(
            def.program.identity,
            Some(app_identity(&name("photo-index")))
        );
        assert_eq!(def.volume.as_deref(), Some(service::BUILT_IN_VOLUME));
    }

    /// **The case that failed on hardware on 2026-08-25**, and the reason this
    /// test exists rather than the ones above being trusted.
    ///
    /// After `ADR-015` §3.2.2 every installed application is disabled until the
    /// document says otherwise, so the *usual* state at an install edge is a
    /// definition the supervisor already holds, sitting idle. Discovery refuses
    /// to add a second definition for a name it already has — correctly — so
    /// the first version of this module saw "nothing new" and did nothing, and
    /// `cargo rivet deploy --enable` wrote a correct line into the document
    /// that nothing read. Every unit test passed.
    #[test]
    fn an_application_the_supervisor_already_knows_takes_the_documents_new_answer() {
        let mut k = FakeKernel::new();
        install(&mut k, "photo-index");
        // The document says to run it. `crate::document::load` reads the
        // machine's stored document, which this fake serves.
        k.files_mut().insert(
            "/state/config/machine.toml".to_string(),
            Script::always(Ok(
                "version = 1\n[services.photo-index]\nenabled = true\n".to_string()
            )),
        );

        // It is already a definition, and disabled — which is what a boot after
        // the ruling leaves behind.
        let existing = crate::applications::definition(
            service::BUILT_IN_VOLUME,
            &name("photo-index"),
            &rivet_core::package::application::Manifest::new(
                rivet_core::package::application::ProgramKind::Daemon,
            ),
            &[],
        );
        assert!(!existing.enabled, "the fixture is not the state being tested");
        let mut sup = supervisor(vec![existing]);
        let mut watch = Watch::new();

        run_the_installer(&mut k, &mut sup, &mut watch);
        watch.poll(&mut k, &mut sup);

        assert!(
            sup.services()
                .iter()
                .find(|m| m.def.name == "photo-index")
                .is_some_and(|m| m.def.enabled),
            "the document said to run it and the supervisor did not take the answer"
        );
    }

    /// **The second install of a boot was invisible, and this is it from the
    /// outside.** `PLAN.md` §105.3, found on hardware 2026-09-01.
    ///
    /// `Watch` recorded only *whether* the installer was finished. `State::Done`
    /// persists, so after one install the flag stayed set for the rest of the
    /// boot — and a second install that was adopted and finished between two
    /// convergence passes produced no rising edge. Discovery never ran, and the
    /// supervisor went on starting the service from the FIRST payload's
    /// manifest while reporting that stale value in the journal as though it
    /// were current.
    ///
    /// The deploy that found it was this exact shape: the same application
    /// twice, differing only in `listen`, because the first port was already
    /// taken by another service.
    #[test]
    fn a_second_install_in_the_same_boot_is_noticed_and_its_manifest_taken() {
        let mut k = FakeKernel::new();
        let mut sup = supervisor(Vec::new());
        let mut watch = Watch::new();

        install_listening(&mut k, "photo-index", 8080);
        run_the_installer(&mut k, &mut sup, &mut watch);
        watch.poll(&mut k, &mut sup);
        assert_eq!(
            listen_of(&sup, "photo-index"),
            vec![rivet_core::package::application::Socket::Tcp(8080)],
            "the fixture is wrong: the FIRST install was not adopted"
        );

        // The same application again, differing only in the field the
        // supervisor acts on.
        install_listening(&mut k, "photo-index", 8088);
        run_the_installer(&mut k, &mut sup, &mut watch);
        watch.poll(&mut k, &mut sup);

        assert_eq!(
            listen_of(&sup, "photo-index"),
            vec![rivet_core::package::application::Socket::Tcp(8088)],
            "the second install finished between two passes and the watch never \
             noticed, so the supervisor kept the first manifest"
        );
    }

    /// **Every manifest field, not a list somebody has to maintain.**
    /// `PLAN.md` §105.7.
    ///
    /// `Supervisor::take_application_definition` enumerated the fields it
    /// copied, and that list was wrong three times — `program` alone, then
    /// `listen` four hours later, then `connects` on 2026-09-01, each found by
    /// something in the field rather than by a test.
    ///
    /// This asserts the property instead of the list: a reinstall carries the
    /// whole definition. It is deliberately written on `connects`, which is the
    /// field that was missing, and it fails against the enumerated version.
    #[test]
    fn a_reinstall_takes_every_manifest_field_and_not_a_remembered_list() {
        let mut k = FakeKernel::new();
        let mut sup = supervisor(Vec::new());
        let mut watch = Watch::new();

        install_listening(&mut k, "photo-index", 8080);
        run_the_installer(&mut k, &mut sup, &mut watch);
        watch.poll(&mut k, &mut sup);
        assert!(
            connects_of(&sup, "photo-index").is_empty(),
            "the fixture is wrong: the first manifest should declare no connects"
        );

        // The same application, now asking for another application's endpoint.
        // `listen` changes too, because the supervisor only reconsiders a
        // definition it can see has changed at all.
        install_connecting(&mut k, "photo-index", 8088, "store", "api");
        run_the_installer(&mut k, &mut sup, &mut watch);
        watch.poll(&mut k, &mut sup);

        let connects = connects_of(&sup, "photo-index");
        assert_eq!(
            connects.len(),
            1,
            "the new manifest's `connects` was dropped: the supervisor took the \
             program and the listen from it and kept the old, empty connects"
        );
        assert_eq!(connects[0].owner(), "store");
        assert_eq!(connects[0].name(), "api");
    }

    /// The other direction, which is `ADR-017` §6's own semantics: a document
    /// that now says no is taken too.
    #[test]
    fn an_application_the_document_now_disables_takes_that_answer_too() {
        let mut k = FakeKernel::new();
        install(&mut k, "photo-index");
        k.files_mut().insert(
            "/state/config/machine.toml".to_string(),
            Script::always(Ok(
                "version = 1\n[services.photo-index]\nenabled = false\n".to_string()
            )),
        );

        let mut enabled_already = crate::applications::definition(
            service::BUILT_IN_VOLUME,
            &name("photo-index"),
            &rivet_core::package::application::Manifest::new(
                rivet_core::package::application::ProgramKind::Daemon,
            ),
            &[],
        );
        enabled_already.enabled = true;
        let mut sup = supervisor(vec![enabled_already]);
        let mut watch = Watch::new();

        run_the_installer(&mut k, &mut sup, &mut watch);
        watch.poll(&mut k, &mut sup);

        assert!(
            sup.services()
                .iter()
                .find(|m| m.def.name == "photo-index")
                .is_some_and(|m| !m.def.enabled),
            "the document said not to run it and the supervisor kept it enabled"
        );
    }

    /// **The image's own services are never re-answered**, and this is the
    /// guard on it: a badly edited document must not be able to take the
    /// control plane away from a running machine at the moment somebody
    /// installs something. `ADR-018` §8 — a machine that can be reached is one
    /// somebody can repair, and nothing rolls `/state` back.
    #[test]
    fn a_document_that_disables_the_control_plane_does_not_stop_it_at_an_install() {
        let mut k = FakeKernel::new();
        install(&mut k, "photo-index");
        k.files_mut().insert(
            "/state/config/machine.toml".to_string(),
            Script::always(Ok(format!(
                "version = 1\n[services.{}]\nenabled = false\n",
                crate::definitions::MCPD_NAME
            ))),
        );

        let mcpd = ServiceDef {
            name: crate::definitions::MCPD_NAME.to_string(),
            program: crate::kernel::Program {
                path: crate::definitions::MCPD_PATH.to_string(),
                args: Vec::new(),
                env: Vec::new(),
                identity: None,
                confinement: None,
                groups: Vec::new(),
                capabilities: Vec::new(),
            },
            enabled: true,
            consequence: "there is no control plane".to_string(),
            ..IMAGE_DEFINED
        };
        let mut sup = supervisor(vec![mcpd]);
        let mut watch = Watch::new();

        run_the_installer(&mut k, &mut sup, &mut watch);
        watch.poll(&mut k, &mut sup);

        assert!(
            sup.services()
                .iter()
                .find(|m| m.def.name == crate::definitions::MCPD_NAME)
                .is_some_and(|m| m.def.enabled),
            "an install took the control plane away from a running machine"
        );
    }

    // -- A-2: an install must not bypass the kiosk selection -----------------

    /// The same as [`install`], declaring `devices = ["display"]` — an
    /// application offering a screen (`ADR-015` §6).
    fn install_display(k: &mut FakeKernel, app: &str) {
        let text = rivet_core::package::application::Manifest::new(
            rivet_core::package::application::ProgramKind::Daemon,
        )
        .with_devices(vec![rivet_core::package::application::Device::Display])
        .render();

        k.dirs_mut()
            .entry(service::app_root(service::BUILT_IN_VOLUME))
            .or_default()
            .push(app.to_string());
        k.files_mut().insert(
            service::manifest_path(service::BUILT_IN_VOLUME, &name(app)),
            Script::always(Ok(text)),
        );
        // **The version directory, not an empty string.** `Supervisor::start`
        // derives it from `program.path` by stripping `bin/<name>` off the
        // end, and a discovered application's path is the real
        // `.../current/bin/<name>` — unlike the flat `/bin/svc` fixtures
        // `crate::supervise`'s own tests use, whose derived version directory
        // really is empty. Signing the header at the wrong path would leave
        // `verify::program` unable to find it and refuse to launch.
        crate::verify::fixture::install_signed(
            k,
            app,
            &service::current_dir(service::BUILT_IN_VOLUME, &name(app)),
            &service::program_path(service::BUILT_IN_VOLUME, &name(app)),
        );
    }

    /// **A-2's fix, first half.** An install must not re-enable an offered
    /// display application the kiosk gate has already narrowed away. The
    /// document can go on saying more than one display application is
    /// enabled — an operator enables them over time — and the gate, not the
    /// document, is what narrows that to the one selection (`ADR-015` §6).
    #[test]
    fn an_install_does_not_restart_every_offered_display_application() {
        let mut k = FakeKernel::new();
        install_display(&mut k, "screen-a");
        install_display(&mut k, "screen-b");
        k.files_mut().insert(
            "/state/config/machine.toml".to_string(),
            Script::always(Ok(
                "version = 1\n[services.screen-a]\nenabled = true\n[services.screen-b]\n\
                 enabled = true\n"
                    .to_string(),
            )),
        );
        let mut sup = supervisor(Vec::new());
        let mut watch = Watch::new();

        run_the_installer(&mut k, &mut sup, &mut watch);
        watch.poll(&mut k, &mut sup);
        sup.select_display("screen-a")
            .expect("screen-a was just installed and the document enables it");
        sup.converge(&mut k);
        assert!(
            matches!(sup.state_of("screen-a"), Some(State::Started { .. })),
            "the fixture is wrong: the selected application did not start: {:?}",
            sup.state_of("screen-a")
        );
        assert!(
            !matches!(sup.state_of("screen-b"), Some(State::Started { .. })),
            "the fixture is wrong: the unselected application started before any second \
             install: {:?}",
            sup.state_of("screen-b")
        );

        // A second install — of anything — re-triggers the install edge over
        // a document that has not changed.
        install(&mut k, "unrelated");
        run_the_installer(&mut k, &mut sup, &mut watch);
        watch.poll(&mut k, &mut sup);
        sup.converge(&mut k);

        assert!(
            !matches!(sup.state_of("screen-b"), Some(State::Started { .. })),
            "an install re-enabled and started every offered display application, bypassing \
             the kiosk selection: {:?}",
            sup.state_of("screen-b")
        );
        assert!(
            matches!(sup.state_of("screen-a"), Some(State::Started { .. })),
            "the selected display application was displaced by the install: {:?}",
            sup.state_of("screen-a")
        );
    }

    /// **A-2's fix, second half.** `adopt_new` takes `enabled` from the
    /// document alone and applies no gate, and until the offered set is
    /// re-read a freshly installed display application is absent from it —
    /// so an operator who just installed a kiosk application could not select
    /// it without a reload.
    #[test]
    fn a_newly_installed_display_application_is_offered_without_a_reload() {
        let mut k = FakeKernel::new();
        install_display(&mut k, "screen-a");
        k.files_mut().insert(
            "/state/config/machine.toml".to_string(),
            Script::always(Ok("version = 1\n[services.screen-a]\nenabled = true\n".to_string())),
        );
        let mut sup = supervisor(Vec::new());
        let mut watch = Watch::new();

        run_the_installer(&mut k, &mut sup, &mut watch);
        watch.poll(&mut k, &mut sup);

        assert!(
            sup.select_display("screen-a").is_ok(),
            "a display application installed while the machine was running could not be \
             selected without a reload"
        );
    }
}
