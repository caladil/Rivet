//! **`SIGHUP` — re-read the desired-state document.** `ADR-032` §3.2, built
//! 2026-08-26.
//!
//! `PLAN.md` §28.5 Phase 1 item 4. The signal is the Unix answer to §1.1's
//! *tell*, and the ADR takes both halves of §3.2's ruling — the query socket
//! answers *ask*, this answers *tell*:
//!
//! > **`SIGHUP` means re-read the document**, with one rule that is part of the
//! > ruling rather than an implementation note: **a document that will not parse
//! > leaves the supervisor running on the last good one, and says so loudly in
//! > the journal.** A reload that takes a machine down is worse than one that
//! > never happened.
//!
//! # The sentence this module makes false, and what replaces it
//!
//! `crate::installed` says, in a comment written 2026-08-24:
//!
//! > **The document is read again, here, and this is the only moment in a boot
//! > that it is.** … nothing else re-reads it, so a configuration change does
//! > not otherwise reach a running machine at all (`PLAN.md` §20.3, which is the
//! > larger question this deliberately does not answer).
//!
//! This is the larger question, answered. That comment is corrected where it
//! stands rather than deleted (`AGENTS.md` §7): the install edge is still a
//! moment the document is read, and it is no longer the only one.
//!
//! # What a reload may change, and the one thing it may not
//!
//! **Ruled by the maintainer 2026-08-26**, recorded in `ADR-032` §3.2.1: a
//! reload applies everything *except* a named reachability set. The line is
//! drawn in [`crate::document::Occasion`] rather than here, because it is a fact
//! about applying a document and not about signals; [`crate::definitions::REACHABILITY_SET`]
//! is the list and carries the argument.
//!
//! The short form: `ADR-018` §8 — **a machine that is reachable is one somebody
//! can repair** — and `PLAN.md` §28.2 names the failure it prevents as the one
//! that is unrecoverable. A board that honours `[services.rivet-mcpd] enabled =
//! false` at 3am is up, healthy, and gone.
//!
//! *(Which service that sentence is about moved on 2026-08-29, when `ADR-035`
//! phase 3 made the control plane the administration surface and
//! `REACHABILITY_SET` was retargeted with it. The sentence did not change.)*
//!
//! # It reports and does not refuse
//!
//! Every withheld value is **kept and said out loud**, at `Severity::Warn`,
//! naming what was withheld and what to do instead — `ADR-007` §3.6, the system
//! never accepts an input it will discard. The values apply at the next boot,
//! when `crate::main` reads the document with a console attached and `ADR-018`
//! §5.1's rollback still underneath.
//!
//! # Why this does not converge, and why that is not an omission
//!
//! It sets fields. `crate::main`'s loop calls `Supervisor::converge` on every
//! pass, immediately after the event that reached this module, so what changed
//! is started or stopped there — by the rules every service is always subject
//! to. A signal that did its own starting or stopping would be a second place
//! deciding what runs, and `ADR-017` §6 keeps that decision in one pass. A
//! reload also leaves an operator's held stop in place: the hold is not part
//! of the document.

use crate::document::{self, Occasion};
use crate::journal::{Severity, Value};
use crate::kernel::Kernel;
use crate::service::ServiceDef;
use crate::supervise::Supervisor;

/// Re-read the document and apply what a running machine may take.
pub fn reload(k: &mut dyn Kernel, sup: &mut Supervisor) {
    // Said BEFORE the read, so that a read which hangs on a card that has gone
    // away still leaves a record of somebody having asked. The journal is the
    // only account an operator gets — the signal cannot carry a reply
    // (`ADR-032` §3.2's honest limit), so a reload that said nothing until it
    // finished would be indistinguishable from one that never arrived.
    sup.note(
        k,
        Severity::Info,
        "SIGHUP: re-reading the desired-state document (ADR-032 §3.2). Nothing has changed yet, \
         and what a running machine may take is narrower than what a boot may — see the records \
         below",
        Vec::new(),
    );

    let mut defs: Vec<ServiceDef> = sup.services().iter().map(|m| m.def.clone()).collect();
    let before: Vec<(String, bool)> = defs
        .iter()
        .map(|d| (d.name.clone(), d.enabled))
        .collect();

    // **An unreadable document changes nothing**, and it does not need a branch
    // here to be true: `document::load` answers `Unreadable` and journals the
    // reason at `Severity::Error`, and `apply_on` returns immediately for
    // anything that is not `Stored`. So the last good answer stands because
    // nothing overwrote it — `ADR-032` §3.2's rule, held by the shape rather
    // than by a check that could be forgotten.
    let desired = document::load(k, sup.journal_mut());
    document::apply_on(k, sup.journal_mut(), &mut defs, &desired, Occasion::Reload);

    // **`get status`'s `mode`, re-read — and NOT overwritten by a document that
    // would not parse.** `PLAN.md` §109.3.
    //
    // This is the same rule as the paragraph above, applied by hand because
    // this fact does not go through `apply_on`: an unreadable document leaves
    // the last good answer standing, because nothing on the machine changed and
    // replacing `kiosk` with `unknown` would report a machine that is still
    // running its application as one nobody can describe. The read failure is
    // in the journal at `Severity::Error`, put there by `document::load`, which
    // is where the honesty about it lives.
    //
    // Boot is the other case and `crate::main` takes it: there, nothing stands,
    // so `unknown` is the only true answer.
    if !matches!(desired, document::Desired::Unreadable { .. }) {
        sup.set_kiosk_mode(document::kiosk_mode(&desired));

        // **Application settings are handed over again** — `ADR-038` §6.
        //
        // Until 2026-09-05 `runtime::deliver_settings` had exactly one caller,
        // in `crate::main` before the first convergence, so a setting written
        // into the document on a running machine reached the application only
        // at the next boot. The rest of that chain was already built and
        // working: the supervisor renders the file, `rivet_sdk::settings::all`
        // re-reads it on every call and caches nothing, and TempMon polls it
        // every thirty seconds. The file it polled was the one written at boot.
        //
        // Measured on `rivet-01`: `sensor-labels` was set at 20:12Z and an
        // hour later `/api/v1/sensors` still answered `"label": null`, with
        // the value plainly present in the desired-state document.
        //
        // **This is not the `enabled` case and takes no withholding.** A
        // settings file is data an application re-reads; changing it does not
        // move a running process between states, which is what makes reload's
        // narrower rule (`ADR-032` §3.2) apply there and not here. An
        // application that only reads its settings at startup sees the new file
        // at its next start, which is exactly what it saw before.
        let declared = crate::runtime::declared_settings(k, sup.services());
        crate::runtime::deliver_settings(k, sup.journal_mut(), &desired.document(), &declared);

        // **Re-project `/etc`, so a reload actually reaches the files musl
        // reads.** `/etc` is otherwise projected once — `crate::main`, before
        // the first convergence and therefore before DHCP — so two things could
        // not reach a running machine until it rebooted: a resolver learned from
        // a DHCP lease (`ADR-023`; `crate::etc::leased_resolver`), which is the
        // post-DHCP re-render the lease half needs, and a changed `timezone` or
        // `hostname`. `project` is re-runnable onto the existing tmpfs —
        // `a_supervisor_restart_converges_onto_the_existing_tmpfs` proves it —
        // and it is byte-identical when nothing changed. **Gated on a readable
        // document by the enclosing `if`**, so a document that would not parse
        // leaves `/etc` exactly as it was, the same rule the rest of this holds.
        let document = desired.document();
        let naming = crate::naming::name_this_machine(k, sup.journal_mut(), &document);
        // The name the supervisor answers with follows the one `/etc` is about
        // to carry, so the facts call and the name service's projection below
        // agree with `/etc/hostname` rather than with the boot's.
        sup.set_machine_name(naming.hostname().map(|h| h.as_str().to_string()));
        let defs: Vec<ServiceDef> = sup.services().iter().map(|m| m.def.clone()).collect();
        let app_names = crate::etc::application_names(&defs);
        let accounts = crate::etc::accounts(&app_names);
        let _ = crate::etc::project(k, sup.journal_mut(), &document, &accounts, &naming);

        // **The mDNS responder follows its name and address** — `crate::mdns`,
        // `ADR-017` §3. A reload is how both arrive: a renamed machine
        // through the document, a renewed lease through `rivet-dhcp-hook`'s
        // nudge. Like everything here it only asks; the next convergence does
        // the restart.
        let inputs = crate::mdns::Inputs::read(
            k,
            &document,
            naming.hostname().map(rivet_core::hostname::Hostname::as_str),
        );
        sup.follow_mdns_inputs(k, inputs);

        // **And `rivet-netd` follows the leases** — `crate::leases`. The
        // hook's nudge for a lease it has just recorded arrives here, and this
        // is what puts that lease's address on the interface. Like the rest it
        // only asks; the next convergence runs it.
        let leases = crate::leases::Leases::read(k, &document);
        sup.follow_leases(k, leases);
    }

    // **The offered set is re-read from what the document just said.**
    // `PLAN.md` §106.5. The loop below hands each service the document's
    // answer; this is the moment after that and before the gate narrows it,
    // which is the only moment `enabled` means *offered*. It is done at the end
    // of the loop rather than here — see the call below.
    let mut changed = 0usize;
    for (name, was) in before {
        let Some(def) = defs.iter().find(|d| d.name == name) else {
            continue;
        };
        if def.enabled == was {
            continue;
        }
        if sup.set_enabled(&name, def.enabled) {
            changed += 1;
            sup.note(
                k,
                Severity::Info,
                if def.enabled {
                    "the reloaded document says this service should run. The next convergence \
                     starts it — no restart, and nothing here started it directly (ADR-017 §6)"
                } else {
                    "the reloaded document says this service should NOT run. The next \
                     convergence stops it, with ADR-017 §3.5's deadline and a SIGKILL if it \
                     will not go"
                },
                vec![
                    ("svc_name".to_string(), Value::Str(name)),
                    ("was".to_string(), Value::Bool(was)),
                    ("now".to_string(), Value::Bool(def.enabled)),
                ],
            );
        }
    }

    // **An image service the reload turned on gets its home before the next
    // convergence starts it** (`ADR-060`). It converges, so one that already
    // had its home costs a stat per directory.
    crate::applications::provide_image_homes(k, sup.journal_mut(), &defs);

    // **The name service's projection of the document, after its home exists**
    // (`ADR-047` §6). After `set_enabled` above, so a reload that turns the
    // service on writes the file the service is about to read rather than
    // leaving it to refuse at its first start; and gated on a readable document
    // for the reason everything else here is.
    if !matches!(desired, document::Desired::Unreadable { .. }) {
        let declared = crate::runtime::declared_settings(k, sup.services());
        let machine_name = sup.machine_name().map(str::to_string);
        crate::runtime::deliver_name_service_document(
            k,
            sup.journal_mut(),
            &desired.document(),
            machine_name.as_deref(),
            &declared,
        );
        // **And the mDNS responder's, from the inputs recorded above** -
        // `ADR-017` §3. Every lease's nudge arrives here, so a responder
        // waiting for its first lease reads it from this file.
        let mdns_inputs = sup.mdns_inputs().cloned();
        crate::runtime::deliver_mdns_projection(
            k,
            sup.journal_mut(),
            mdns_inputs.as_ref(),
            &declared,
        );
    }

    // **Now the offered set, from the answers just applied** — `PLAN.md`
    // §106.5. Between the document and the gate, which is the only moment
    // `enabled` means *offered* rather than *running*, and the same order the
    // `Supervisor` constructor uses.
    //
    // Unconditional, and not inside the `changed` branch above: a document that
    // enabled a display application this supervisor had never seen offered
    // changes nothing about any *existing* service's `enabled`, so a
    // change-gated refresh would miss exactly the case this exists for.
    sup.refresh_display_offered();

    // **Zero is a result and is said**, not a silence. An operator who sends
    // `SIGHUP` and sees nothing cannot tell *the document matched what is
    // running* from *the signal never arrived*, and those need different next
    // moves. `ADR-017` §3.1.11's argument about a console that cries wolf, in
    // its other direction.
    sup.note(
        k,
        Severity::Info,
        "SIGHUP: the document has been re-read and applied",
        vec![(
            "changed".to_string(),
            Value::Int(i64::try_from(changed).unwrap_or(i64::MAX)),
        )],
    );
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::definitions::{MCPD_NAME, MDNSD_NAME, NETD_NAME, TIMED_NAME};
    use crate::journal::Journal;
    use crate::service::{Limits, Origin};
    use crate::testkernel::{FakeKernel, Script};

    /// A document naming services, with `enabled` as given.
    fn document_saying(entries: &[(&str, bool)]) -> String {
        let mut out = String::from("version = 1\n");
        for (name, enabled) in entries {
            out.push_str(&format!("[services.\"{name}\"]\nenabled = {enabled}\n"));
        }
        out
    }

    /// Build a supervisor over the image's own services plus one application,
    /// with a kernel whose stored document is `stored`.
    fn fixture(stored: &str) -> (FakeKernel, Supervisor) {
        fixture_with(stored, |_| {})
    }

    /// [`fixture`], with the application's definition reshaped by `shape`
    /// before the supervisor is built.
    fn fixture_with(stored: &str, shape: impl FnOnce(&mut ServiceDef)) -> (FakeKernel, Supervisor) {
        let mut k = FakeKernel::new();
        k.files_mut().insert(
            rivet_store::ConfigStore::at_default().machine_path(),
            Script::always(Ok(stored.to_string())),
        );
        let mut defs = crate::definitions::system_services();
        // An application, cloned off a system service so that this fixture
        // cannot drift from what `ServiceDef::validate` accepts.
        let mut app = defs
            .iter()
            .find(|d| d.name == TIMED_NAME)
            .cloned()
            .unwrap_or_else(|| panic!("{TIMED_NAME} is not a system service"));
        app.name = "photo-index".to_string();
        app.origin = Origin::Application;
        app.enabled = true;
        // The time client keeps `CAP_SYS_TIME` and runs under the image
        // service ruleset; an application keeps no capability and its ruleset
        // is its manifest's, so neither is carried into the clone.
        app.program.capabilities = Vec::new();
        app.program.confinement = None;
        // `ADR-015` §5.1: an application runs under a uid of its own or it does
        // not run at all, and `ServiceDef::validate` refuses one without.
        // Derived by the same function `crate::applications` uses, so this
        // fixture cannot hold a pair the real path would not produce.
        app.program.identity = Some(rivet_core::service::app_identity(
            &rivet_core::service::ServiceName::new("photo-index")
                .unwrap_or_else(|| panic!("a valid fixture name")),
        ));
        // **A volume and a manifest**, because an installed application has
        // both and `runtime::declared_settings` skips anything without them —
        // an application that declares nothing is handed nothing, which would
        // make the settings test below pass for the wrong reason.
        let app_name = rivet_core::service::ServiceName::new("photo-index")
            .unwrap_or_else(|| panic!("a valid fixture name"));
        app.volume = Some(rivet_core::service::BUILT_IN_VOLUME.to_string());
        k.files_mut().insert(
            rivet_core::service::manifest_path(rivet_core::service::BUILT_IN_VOLUME, &app_name),
            Script::always(Ok(rivet_core::package::application::Manifest::new(
                rivet_core::package::application::ProgramKind::Daemon,
            )
            .with_settings(vec!["rotate-seconds".to_string()])
            .unwrap_or_else(|_| panic!("a valid setting name"))
            .render())),
        );
        shape(&mut app);
        defs.push(app);
        // `rivet-timed` ships disabled, and two of these tests are about an
        // image service being turned OFF by a reload — so it starts on.
        for d in &mut defs {
            if d.name == TIMED_NAME {
                d.enabled = true;
            }
        }
        let sup = match Supervisor::new(defs, Limits::default(), journal()) {
            Ok(s) => s,
            Err(e) => unreachable!("fixture refused: {e}"),
        };
        (k, sup)
    }

    fn journal() -> Journal {
        Journal::new("b".to_string(), 1, crate::supervise::ACTOR.to_string())
    }

    fn enabled_of(sup: &Supervisor, name: &str) -> bool {
        sup.services()
            .iter()
            .find(|m| m.def.name == name)
            .unwrap_or_else(|| panic!("no {name}"))
            .def
            .enabled
    }

    /// Everything the supervisor said, as one string to assert against.
    fn said(k: &FakeKernel) -> String {
        k.records
            .iter()
            .map(|r| r.msg.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// **A setting written on a running machine reaches the application.**
    ///
    /// `ADR-038` §6. The defect this closes, measured on `rivet-01` 2026-09-05:
    /// `sensor-labels` sat in the desired-state document for an hour while
    /// TempMon's thirty-second poll kept re-reading the file written at boot,
    /// because `deliver_settings` had one caller and it was in `crate::main`.
    #[test]
    fn a_reload_hands_an_application_the_settings_the_document_now_carries() {
        let stored = document_saying(&[(MCPD_NAME, true)])
            + "[applications.photo-index]\nrotate-seconds = \"30\"\n";
        let (mut k, mut sup) = fixture(&stored);
        reload(&mut k, &mut sup);
        let want = rivet_core::settings::path(
            rivet_core::service::BUILT_IN_VOLUME,
            &rivet_core::service::ServiceName::new("photo-index").expect("a name"),
        );
        let written = k
            .writes
            .iter()
            .find(|w| w.path == want)
            .map(|w| String::from_utf8(w.contents.clone()).expect("utf-8"))
            .unwrap_or_else(|| panic!("a reload wrote no settings file; it said:\n{}", said(&k)));
        assert!(
            written.contains("rotate-seconds") && written.contains("30"),
            "the settings file was written without the document's value: {written:?}"
        );
    }

    /// **A reload that renames the machine changes the name the supervisor
    /// answers with**, so the facts call says the new name without a reboot.
    /// The slot it booted from is not the document's and is left alone.
    #[test]
    fn a_reload_that_renames_the_machine_updates_the_name_the_supervisor_answers() {
        let stored = "version = 1\nhostname = \"kitchen-pi\"\n";
        let (mut k, mut sup) = fixture(stored);
        sup.set_machine_identity(Some("old-name".to_string()), Some("a".to_string()));

        reload(&mut k, &mut sup);

        assert_eq!(sup.machine_name(), Some("kitchen-pi"), "it said:\n{}", said(&k));
        let facts = sup.facts(&mut k);
        assert_eq!(facts.hostname.as_deref(), Some("kitchen-pi"));
        assert_eq!(facts.slot.as_deref(), Some("a"));
    }

    /// A document that will not parse leaves the name the supervisor holds, by
    /// the rule every other value of a reload keeps.
    #[test]
    fn an_unreadable_document_leaves_the_machine_name_standing() {
        let (mut k, mut sup) = fixture("this is not toml [");
        sup.set_machine_identity(Some("old-name".to_string()), Some("a".to_string()));
        reload(&mut k, &mut sup);
        assert_eq!(sup.machine_name(), Some("old-name"));
    }

    /// **The reachability set is not stopped by a reload**, which is the whole
    /// of the 2026-08-26 ruling. `ADR-032` §3.2.1, `ADR-018` §8.
    ///
    /// **The set itself was retargeted on 2026-08-29**, and this test did not
    /// change shape because the ruling did not: only which service can take
    /// the way in has moved.
    #[test]
    fn a_reload_will_not_stop_the_services_that_keep_a_machine_reachable() {
        let (mut k, mut sup) = fixture(&document_saying(&[
            (MCPD_NAME, false),
            (NETD_NAME, false),
        ]));
        reload(&mut k, &mut sup);
        assert!(
            enabled_of(&sup, MCPD_NAME),
            "a reload stopped the control plane; there is now no way back in"
        );
        assert!(
            enabled_of(&sup, NETD_NAME),
            "a reload stopped the interface bring-up"
        );
    }

    /// And it says so, rather than dropping the line — `ADR-007` §3.6.
    #[test]
    fn a_withheld_value_is_reported_and_names_what_to_do_instead() {
        let (mut k, mut sup) = fixture(&document_saying(&[(MCPD_NAME, false)]));
        reload(&mut k, &mut sup);
        let said = said(&k);
        assert!(
            said.contains("a RELOAD does not change this service's enabled value"),
            "{said}"
        );
        assert!(said.contains("apply restart confirm"), "{said}");
    }

    /// Everything else applies — including an `Origin::Image` service that is
    /// not in the set, which is what makes this a reload rather than the
    /// install edge's narrower re-read.
    #[test]
    fn a_reload_applies_to_every_service_outside_the_reachability_set() {
        let (mut k, mut sup) = fixture(&document_saying(&[
            (TIMED_NAME, false),
            ("photo-index", false),
        ]));
        assert!(enabled_of(&sup, TIMED_NAME));
        reload(&mut k, &mut sup);
        assert!(
            !enabled_of(&sup, TIMED_NAME),
            "an image service outside the set was not reloaded"
        );
        assert!(!enabled_of(&sup, "photo-index"));
    }

    /// **A document that will not parse leaves the last good one running**, and
    /// says so loudly. `ADR-032` §3.2's rule, and the reason it is a rule: a
    /// reload that takes a machine down is worse than one that never happened.
    #[test]
    fn a_document_that_will_not_parse_changes_nothing_and_is_loud_about_it() {
        let (mut k, mut sup) = fixture("version = 1\n[services.rivet-timed]\nenabled = \"yes\"\n");
        let before: Vec<bool> = sup.services().iter().map(|m| m.def.enabled).collect();
        reload(&mut k, &mut sup);
        let after: Vec<bool> = sup.services().iter().map(|m| m.def.enabled).collect();
        assert_eq!(before, after, "an unreadable document changed something");
        let said = said(&k);
        assert!(
            said.contains("error") || said.contains("could not"),
            "the refusal was not said loudly: {said}"
        );
    }

    /// A reload that changes nothing says so. An operator cannot otherwise tell
    /// *the document already matched* from *the signal never arrived*.
    #[test]
    fn a_reload_that_changes_nothing_still_reports_that_it_ran() {
        let (mut k, mut sup) = fixture(&document_saying(&[(TIMED_NAME, true)]));
        reload(&mut k, &mut sup);
        let said = said(&k);
        assert!(said.contains("the document has been re-read and applied"), "{said}");
        // The count is a FIELD, not part of the message — which is `ADR-022`
        // §2's whole shape and the reason a test that grepped the prose for it
        // would be asserting the wrong thing.
        let closing = k
            .records
            .iter()
            .find(|r| r.msg.contains("re-read and applied"))
            .unwrap_or_else(|| panic!("no closing record: {said}"));
        assert_eq!(
            closing
                .fields
                .iter()
                .find(|(name, _)| name == "changed")
                .map(|(_, v)| v.clone()),
            Some(Value::Int(0)),
            "a reload that changed nothing must report zero rather than stay quiet: {:?}",
            closing.fields
        );
    }

    /// **A reload re-projects `/etc`, so a DHCP lease that landed after boot
    /// reaches `/etc/resolv.conf`.** `ADR-023`, the post-DHCP re-render — `/etc`
    /// is projected once, before DHCP, so this SIGHUP path is what carries a
    /// lease-learned resolver to the file musl reads. End-to-end with
    /// `crate::etc::leased_resolver` and the document-is-silent rule.
    #[test]
    fn a_reload_reprojects_etc_and_a_dhcp_lease_reaches_resolv_conf() {
        let (mut k, mut sup) = fixture(&document_saying(&[(MCPD_NAME, true)]));
        k.dir_entries
            .insert("/run/rivet/dhcp".to_string(), vec!["eth0.lease".to_string()]);
        k.files_mut().insert(
            "/run/rivet/dhcp/eth0.lease".to_string(),
            Script::always(Ok("interface=eth0\nip=192.168.50.66\nmask=22\n\
                 router=192.168.50.1\ndns=192.168.50.1\nlease=7200\n"
                .to_string())),
        );
        reload(&mut k, &mut sup);
        let resolv = k
            .writes
            .iter()
            .find(|w| w.path == rivet_core::etc::RESOLV_CONF)
            .map(|w| String::from_utf8_lossy(&w.contents).into_owned())
            .unwrap_or_default();
        assert!(
            resolv.contains("nameserver 192.168.50.1"),
            "a reload did not carry the DHCP lease's nameserver to resolv.conf; wrote {resolv:?}, \
             and said:\n{}",
            said(&k)
        );
    }

    // ---- D7: the responder follows its name and address ---------------------

    const STATIC_ETH0: &str =
        "version = 1\n[network.eth0]\nmethod = \"static\"\naddress = \"192.168.1.50/24\"\n";
    const DHCP_ETH0: &str = "version = 1\n[network.eth0]\nmethod = \"dhcp\"\n";

    /// A supervisor whose `rivet-mdnsd` is running, having started on
    /// `at_start`; and its pid.
    fn running_responder(
        stored: &str,
        at_start: crate::mdns::Inputs,
    ) -> (FakeKernel, Supervisor, i32) {
        let (mut k, mut sup) = fixture(stored);
        assert!(sup.set_enabled(MDNSD_NAME, true));
        sup.converge(&mut k);
        let pid = match sup.state_of(MDNSD_NAME) {
            Some(crate::supervise::State::Started { pid, .. }) => *pid,
            other => panic!("the responder did not start: {other:?}"),
        };
        sup.follow_mdns_inputs(&mut k, at_start);
        (k, sup, pid)
    }

    fn eth0(hostname: Option<&str>, address: Option<[u8; 4]>) -> crate::mdns::Inputs {
        crate::mdns::Inputs {
            hostname: hostname.map(str::to_string),
            addressed: address
                .map(|a| vec![("eth0".to_string(), std::net::Ipv4Addr::from(a))])
                .unwrap_or_default(),
            leaseless: if address.is_some() {
                vec![]
            } else {
                vec!["eth0".to_string()]
            },
        }
    }

    fn lease(k: &mut FakeKernel, ip: &str) {
        k.files_mut().insert(
            "/run/rivet/dhcp/eth0.lease".to_string(),
            Script::always(Ok(format!("interface=eth0\nip={ip}\nmask=24\n"))),
        );
    }

    fn signalled(k: &FakeKernel, pid: i32) -> bool {
        k.signals.iter().any(|(p, _)| *p == pid)
    }

    /// **A reload that renames the machine restarts the responder**, so it
    /// stops announcing the old name.
    #[test]
    fn a_reload_that_renames_the_machine_restarts_the_responder() {
        let stored = STATIC_ETH0.replacen("version = 1\n", "version = 1\nhostname = \"larder\"\n", 1);
        let (mut k, mut sup, pid) =
            running_responder(&stored, eth0(Some("kitchen"), Some([192, 168, 1, 50])));
        reload(&mut k, &mut sup);
        sup.converge(&mut k);
        assert!(signalled(&k, pid), "the responder kept announcing the old name:\n{}", said(&k));
        assert!(
            k.records
                .iter()
                .any(|r| r.svc == MDNSD_NAME && r.msg.contains("name or address changed")),
            "the restart was not said under the responder's name:\n{}",
            said(&k)
        );
    }

    /// **A DHCP renewal with a different address restarts it**, by the nudge
    /// `rivet-dhcp-hook` sends with every lease it writes.
    #[test]
    fn a_lease_with_a_different_address_restarts_the_responder() {
        let (mut k, mut sup, pid) = running_responder(DHCP_ETH0, eth0(None, Some([10, 0, 0, 66])));
        lease(&mut k, "10.0.0.67");
        reload(&mut k, &mut sup);
        sup.converge(&mut k);
        assert!(signalled(&k, pid), "the responder kept announcing the old address:\n{}", said(&k));
    }

    /// **The first lease reaches the responder through its projection, and
    /// restarts nothing** — `ADR-017` §3. The responder may not read the
    /// lease, so the reload the lease's nudge causes rewrites the projection
    /// with the address, and the responder, still waiting, reads it there.
    #[test]
    fn a_first_lease_is_written_into_the_responders_projection_without_a_restart() {
        let stored = "version = 1\nhostname = \"kitchen\"\n[network.eth0]\nmethod = \"dhcp\"\n\
                      [services.rivet-mdnsd]\nenabled = true\n";
        let (mut k, mut sup, pid) = running_responder(stored, eth0(Some("kitchen"), None));
        lease(&mut k, "10.0.0.66");
        reload(&mut k, &mut sup);
        sup.converge(&mut k);
        assert!(!signalled(&k, pid), "{:?}", k.signals);

        let path = rivet_core::mdns::path(
            rivet_core::service::BUILT_IN_VOLUME,
            &rivet_core::service::ServiceName::new(MDNSD_NAME).expect("a name"),
        );
        let write = k
            .writes
            .iter()
            .rev()
            .find(|w| w.path == path)
            .unwrap_or_else(|| panic!("no projection was written:\n{}", said(&k)));
        assert_eq!(write.mode, rivet_core::mdns::MODE);
        assert!(k.nofollow_writes.contains(&path), "written without the guards");
        let projection = rivet_core::mdns::Projection::parse(
            std::str::from_utf8(&write.contents).expect("utf-8"),
        )
        .expect("a complete projection");
        assert_eq!(
            projection.hostname.as_ref().map(rivet_core::hostname::Hostname::as_str),
            Some("kitchen")
        );
        assert_eq!(
            projection.addressed,
            [("eth0".to_string(), std::net::Ipv4Addr::new(10, 0, 0, 66))]
        );
        assert!(projection.leaseless.is_empty());
    }

    /// **A first lease is the responder's own to read**: it is running,
    /// waiting for exactly this, and restarting it would only cost a restart.
    #[test]
    fn a_first_lease_the_responder_is_waiting_for_restarts_nothing() {
        let (mut k, mut sup, pid) = running_responder(DHCP_ETH0, eth0(None, None));
        lease(&mut k, "10.0.0.66");
        reload(&mut k, &mut sup);
        sup.converge(&mut k);
        assert!(!signalled(&k, pid), "{:?}", k.signals);
    }

    /// And a reload that changes neither restarts nothing.
    #[test]
    fn a_reload_that_leaves_name_and_address_alone_restarts_nothing() {
        let (mut k, mut sup, pid) = running_responder(DHCP_ETH0, eth0(None, Some([10, 0, 0, 66])));
        lease(&mut k, "10.0.0.66");
        reload(&mut k, &mut sup);
        sup.converge(&mut k);
        assert!(!signalled(&k, pid), "{:?}", k.signals);
    }

    // ---- R-253: rivet-netd follows the leases ---------------------------------

    /// A supervisor whose `rivet-netd` has run to completion on `stored`, with
    /// the leases recorded as boot records them, and how many times netd has
    /// been spawned so far.
    fn netd_done(stored: &str) -> (FakeKernel, Supervisor) {
        let (mut k, mut sup) = fixture(stored);
        sup.converge(&mut k);
        let pid = match sup.state_of(NETD_NAME) {
            Some(crate::supervise::State::Started { pid, .. }) => *pid,
            other => panic!("netd did not start: {other:?}"),
        };
        k.exits.push((pid, crate::kernel::ExitStatus::Exited(0)));
        sup.reap(&mut k);
        assert_eq!(sup.state_of(NETD_NAME), Some(&crate::supervise::State::Done));
        let document = rivet_config::Document::parse(stored).expect("a valid document");
        let at_boot = crate::leases::Leases::read(&mut k, &document);
        sup.follow_leases(&mut k, at_boot);
        (k, sup)
    }

    fn netd_spawns(k: &FakeKernel) -> usize {
        k.spawned.iter().filter(|(n, _)| n == NETD_NAME).count()
    }

    /// **The first lease reaches the interface**: the hook's nudge is a reload,
    /// and the reload runs `rivet-netd` again, which is the one program that
    /// writes the address. Before R-253 netd took the lease itself and nothing
    /// had to follow it.
    #[test]
    fn a_first_lease_runs_netd_again_to_apply_it() {
        let (mut k, mut sup) = netd_done(DHCP_ETH0);
        assert_eq!(netd_spawns(&k), 1);
        lease(&mut k, "10.0.0.66");
        reload(&mut k, &mut sup);
        sup.converge(&mut k);
        assert_eq!(netd_spawns(&k), 2, "the lease was not applied:\n{}", said(&k));
        assert!(
            k.records
                .iter()
                .any(|r| r.svc == NETD_NAME
                    && r.msg.contains("DHCP lease was recorded, changed or withdrawn")),
            "the re-run was not said under netd's name:\n{}",
            said(&k)
        );
    }

    /// **A renewal that changed the address is applied** — the case a netd
    /// that had exited could never reach.
    #[test]
    fn a_renewal_with_a_different_address_runs_netd_again() {
        let (mut k, mut sup) = netd_done(DHCP_ETH0);
        lease(&mut k, "10.0.0.66");
        reload(&mut k, &mut sup);
        sup.converge(&mut k);
        let pid = match sup.state_of(NETD_NAME) {
            Some(crate::supervise::State::Started { pid, .. }) => *pid,
            other => panic!("netd was not run again: {other:?}"),
        };
        k.exits.push((pid, crate::kernel::ExitStatus::Exited(0)));
        sup.reap(&mut k);

        lease(&mut k, "10.0.0.67");
        reload(&mut k, &mut sup);
        sup.converge(&mut k);
        assert_eq!(netd_spawns(&k), 3, "the changed lease was not applied:\n{}", said(&k));
    }

    /// **The ordinary renewal — the same lease written again — runs nothing.**
    #[test]
    fn a_renewal_of_the_same_lease_runs_nothing() {
        let (mut k, mut sup) = netd_done(DHCP_ETH0);
        lease(&mut k, "10.0.0.66");
        reload(&mut k, &mut sup);
        sup.converge(&mut k);
        let pid = match sup.state_of(NETD_NAME) {
            Some(crate::supervise::State::Started { pid, .. }) => *pid,
            other => panic!("netd was not run again: {other:?}"),
        };
        k.exits.push((pid, crate::kernel::ExitStatus::Exited(0)));
        sup.reap(&mut k);

        reload(&mut k, &mut sup);
        sup.converge(&mut k);
        assert_eq!(netd_spawns(&k), 2, "an unchanged lease ran netd:\n{}", said(&k));
    }

    /// **A withdrawn lease runs netd again**, so the address it put on the
    /// interface comes off: `nak` and `leasefail` remove the hook's file, and
    /// the interface carries exactly its lease's address — none.
    #[test]
    fn a_withdrawn_lease_runs_netd_again_to_take_the_address_off() {
        let (mut k, mut sup) = netd_done(DHCP_ETH0);
        lease(&mut k, "10.0.0.66");
        reload(&mut k, &mut sup);
        sup.converge(&mut k);
        let pid = match sup.state_of(NETD_NAME) {
            Some(crate::supervise::State::Started { pid, .. }) => *pid,
            other => panic!("netd was not run again: {other:?}"),
        };
        k.exits.push((pid, crate::kernel::ExitStatus::Exited(0)));
        sup.reap(&mut k);

        k.files_mut().remove("/run/rivet/dhcp/eth0.lease");
        reload(&mut k, &mut sup);
        sup.converge(&mut k);
        assert_eq!(netd_spawns(&k), 3, "the withdrawn lease was not followed:\n{}", said(&k));
    }

    /// A static machine has no lease to follow, and a reload runs nothing.
    #[test]
    fn a_static_machine_reloading_runs_netd_nothing() {
        let (mut k, mut sup) = netd_done(STATIC_ETH0);
        reload(&mut k, &mut sup);
        sup.converge(&mut k);
        assert_eq!(netd_spawns(&k), 1, "{}", said(&k));
    }

    /// **A reload that enables a provider and its consumer together starts
    /// both**, as a boot that found both enabled would. Endpoint resolution
    /// reads every enabled definition, so the provider must already count as
    /// enabled when the consumer is checked, whichever order the two are
    /// stored in.
    #[test]
    fn a_reload_enabling_a_provider_and_its_consumer_together_starts_both() {
        use crate::definitions::BLUETOOTHD_NAME;
        use rivet_core::package::application::Endpoint;

        // The consumer, declared and switched off, as an installed application
        // waiting for the operator to turn both on.
        let (mut k, mut sup) = fixture_with(
            &document_saying(&[(BLUETOOTHD_NAME, true), ("photo-index", true)]),
            |app| {
                app.connects = vec![
                    Endpoint::parse(
                        "connects",
                        &format!("{BLUETOOTHD_NAME}:{}", rivet_core::bluetooth::ENDPOINT),
                    )
                    .expect("a valid endpoint"),
                ];
                app.enabled = false;
            },
        );
        k.observations = Script::always(crate::kernel::Observation::Empty);
        // A signed header, without which the consumer is refused at launch
        // for a reason that has nothing to do with its endpoint.
        let program = sup
            .services()
            .iter()
            .find(|m| m.def.name == "photo-index")
            .map(|m| m.def.program.path.clone())
            .expect("the fixture's application");
        let version_dir = program
            .rsplit_once('/')
            .and_then(|(dir, _)| dir.rsplit_once('/'))
            .map(|(dir, _)| dir.to_string())
            .unwrap_or_default();
        crate::verify::fixture::install_signed(&mut k, "photo-index", &version_dir, &program);
        assert!(!enabled_of(&sup, BLUETOOTHD_NAME), "the provider must start disabled");
        assert!(!enabled_of(&sup, "photo-index"), "the consumer must start disabled");
        sup.converge(&mut k);

        reload(&mut k, &mut sup);
        sup.converge(&mut k);

        for name in [BLUETOOTHD_NAME, "photo-index"] {
            assert!(
                matches!(
                    sup.state_of(name),
                    Some(crate::supervise::State::Started { .. })
                ),
                "{name} did not start after one reload enabled both: {:?}\n{}",
                sup.state_of(name),
                said(&k)
            );
        }
    }

    /// **A consumer refused because its provider was switched off is started
    /// once a reload switches the provider on.** The refusal names the document
    /// key that fixes it, so the fix has to work without an extra restart: the
    /// set of enabled providers is what the refusal was about, and a reload is
    /// that set changing.
    #[test]
    fn a_consumer_refused_for_a_disabled_provider_starts_when_a_reload_enables_it() {
        use crate::definitions::BLUETOOTHD_NAME;
        use rivet_core::package::application::Endpoint;

        // The consumer is enabled and its provider is not, which is how a
        // machine looks after the application was enabled first.
        let (mut k, mut sup) = fixture_with(
            &document_saying(&[(BLUETOOTHD_NAME, true), ("photo-index", true)]),
            |app| {
                app.connects = vec![
                    Endpoint::parse(
                        "connects",
                        &format!("{BLUETOOTHD_NAME}:{}", rivet_core::bluetooth::ENDPOINT),
                    )
                    .expect("a valid endpoint"),
                ];
            },
        );
        k.observations = Script::always(crate::kernel::Observation::Empty);
        let program = sup
            .services()
            .iter()
            .find(|m| m.def.name == "photo-index")
            .map(|m| m.def.program.path.clone())
            .expect("the fixture's application");
        let version_dir = program
            .rsplit_once('/')
            .and_then(|(dir, _)| dir.rsplit_once('/'))
            .map(|(dir, _)| dir.to_string())
            .unwrap_or_default();
        crate::verify::fixture::install_signed(&mut k, "photo-index", &version_dir, &program);
        sup.converge(&mut k);
        assert!(
            matches!(sup.state_of("photo-index"), Some(crate::supervise::State::Failed)),
            "the fixture did not reach the connects refusal: {:?}\n{}",
            sup.state_of("photo-index"),
            said(&k)
        );

        reload(&mut k, &mut sup);
        sup.converge(&mut k);

        for name in [BLUETOOTHD_NAME, "photo-index"] {
            assert!(
                matches!(
                    sup.state_of(name),
                    Some(crate::supervise::State::Started { .. })
                ),
                "{name} is not running after the reload that enabled the provider: {:?}\n{}",
                sup.state_of(name),
                said(&k)
            );
        }
        let consumer = sup
            .services()
            .iter()
            .find(|m| m.def.name == "photo-index")
            .expect("the fixture's application");
        assert_eq!(
            consumer.reason, None,
            "a running service still carries the refusal that no longer applies"
        );
    }

    /// It sets a field and starts nothing. `ADR-017` §6 has no manual start.
    #[test]
    fn a_reload_starts_and_stops_nothing_itself() {
        let (mut k, mut sup) = fixture(&document_saying(&[("photo-index", false)]));
        reload(&mut k, &mut sup);
        assert!(
            k.spawned.is_empty(),
            "a reload spawned something: {:?}",
            k.spawned
        );
        assert!(
            k.signals.is_empty(),
            "a reload signalled something: {:?}",
            k.signals
        );
    }
}
