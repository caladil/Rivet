//! **The declared device capability, granted and revoked.** `ADR-015` §6.2,
//! built 2026-08-31 — kiosk mode.
//!
//! > 1. An application that draws **declares it** in its manifest, beside
//! >    `network` and `state`.
//! > 2. **The supervisor arbitrates**, because it is the only thing that starts
//! >    and stops services and therefore the only thing that can know who holds
//! >    what.
//! > 3. **One owner at a time.**
//! > 4. **The splash returns when the owner exits.**
//!
//! This module is points 1's consequence and part of 2. Point 3 is
//! [`crate::supervise::Supervisor`]'s arbitration and point 4 is its stop edge.
//!
//! # The grant is a `chown`, and §6.2 offered two options
//!
//! > the supervisor **chowns the node**, or adds a supplementary gid, when it
//! > starts that service
//!
//! The chown is taken. A supplementary gid needs the node to carry a group and
//! a `0660` mode, so it is a `chgrp` and a `chmod` instead of a `chown` — the
//! same number of syscalls to the same node, plus a group nothing else uses.
//! It would matter if two services could share a device, and §6.2 point 3 rules
//! that they cannot.
//!
//! **The render node is the one device two services do share**, and it takes
//! the other option. The compositor holds it as the display arbiter and every
//! application declaring `gpu` holds it too, each under its own uid, so it is
//! `root:render 0660` and the render group is the grant — see
//! `share_render_node`. Card nodes keep the chown: DRM master is exclusive.
//!
//! # Why the revoke exists, when §6.2 says the grant is per boot
//!
//! > The grant is **per boot**, because devtmpfs recreates the node every boot.
//! > That is the right lifetime: a permission outliving the manifest that asked
//! > for it is a permission nobody can audit.
//!
//! That settles what happens across a reboot and says nothing about what
//! happens when a service *stops*. Leaving the node owned by a stopped
//! application would leave a permission held by nothing — and, since the same
//! uid is derived from the same name (`ADR-015` §5.1.1), it would be a
//! permission the application still has the next time it is started by any
//! route at all. So the node goes back to `root:root` on the stop edge, which
//! makes the grant track the *running* service rather than the boot.
//!
//! # What is NOT enforced, and it is the same sentence §6.1 has always had
//!
//! **This grants; it does not isolate.** Anything running as uid 0 writes to
//! any of these nodes whatever a manifest says — §6.3 says so — and there is no
//! mount namespace to hide a node from an application that did not declare it
//! (`ADR-015` §5.1). What the capability describes is what an *application* may
//! do, which is the population it is for.

use rivet_core::package::application::Device;
use rivet_core::service::{
    AppIdentity, DisplayRole, device_globs, device_name_matches, is_render_node,
    split_device_glob,
};

use crate::journal::{Journal, Severity, Value};
use crate::kernel::Kernel;

/// Root owns every device node on a Rivet, because there is no udev to say
/// otherwise (`ADR-024`).
const ROOT: (u32, u32) = (0, 0);

/// Every node on this machine that a declared class covers, in the order the
/// globs name them.
///
/// **A pattern that matches nothing is not an error.** `/dev/dri/*` is absent
/// on every board built to date, and a board with no keyboard has fewer
/// `event*` nodes than one with three — devtmpfs creates what the hardware
/// needs. An application asking for `input` on a machine with no input devices
/// gets an empty grant and starts, which is the honest outcome: it will find
/// nothing to read, and that is a fact about the machine rather than a refusal
/// the supervisor should invent.
///
/// **`role` decides what `display` expands to** — `rivet_core::service::DisplayRole`,
/// 2026-09-04. It is threaded through rather than defaulted, because a default
/// here would be a policy decision made by whichever caller forgot to think
/// about it, and the safe default and the useful one are opposites.
pub fn nodes(k: &mut dyn Kernel, devices: &[Device], role: DisplayRole) -> Vec<String> {
    let mut found = Vec::new();
    for device in devices {
        for pattern in device_globs(*device, role) {
            let Some((dir, name)) = split_device_glob(pattern) else {
                // A constant in `device_globs` this build cannot expand. Not
                // reachable from anything a caller wrote.
                continue;
            };
            // **One mechanism for a literal and for a pattern.** The first
            // version asked `file_exists` for a literal and listed the
            // directory for a `*`, which is two ways to answer one question —
            // and they disagreed the moment a literal (`/dev/input/mice`) sat
            // in a directory already being listed. Listing and matching covers
            // both, and a literal is simply a pattern with nothing to expand.
            //
            // `list_dir_entries` and NOT `list_dir`: that one answers
            // directories only, and nothing under `/dev` is one. See the trait
            // — writing this against `list_dir` first would have granted zero
            // input nodes on every board while every test passed.
            let Ok(entries) = k.list_dir_entries(dir) else {
                // The directory is not there — `/dev/dri` on every board so
                // far. Nothing to grant and nothing to report as wrong.
                continue;
            };
            for entry in entries {
                if device_name_matches(name, &entry) {
                    found.push(format!("{dir}/{entry}"));
                }
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// Give this service's identity every node its manifest declared.
///
/// Returns the paths granted. A node that could not be chowned is **reported
/// and skipped rather than fatal**: the alternative is that one unexpected node
/// under `/dev/input` stops a kiosk application from starting at all, and
/// `ADR-017` §3.1.4's principle is that one bad thing is a failed part rather
/// than a broken system. What the application then finds is a device it cannot
/// open, which is exactly what it would find on a machine without the hardware.
pub fn grant(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    service: &str,
    corr: &str,
    devices: &[Device],
    role: DisplayRole,
    identity: AppIdentity,
) -> Vec<String> {
    // **The console stops owning the screen the moment something else is given
    // it** — `PLAN.md` §104.43.
    //
    // `cmd/rivet-initd` unbinds the framebuffer console before it draws the
    // splash, and that was the only thing that ever did. It is not enough: on a
    // TRIAL boot the splash is deliberately not drawn — *"this is the boot that
    // may be rolled back and its output is the point"* — so the console stays
    // bound, and every moment no process holds DRM master it draws over
    // whatever is there. A maintainer watching an application start saw a
    // screen full of journal lines first.
    //
    // Doing it HERE rather than at boot keeps the property tied to its cause: a
    // machine that gives no application a display keeps its console, on every
    // boot path, and one that gives an application a display stops competing
    // with itself. It is idempotent, so granting a second display application
    // costs one write of a byte that is already zero.
    if devices.contains(&Device::Display) {
        quiet_the_framebuffer_console(k, journal, service, corr);
    }

    let mut granted = Vec::new();
    for path in nodes(k, devices, role) {
        let given = if is_render_node(&path) {
            share_render_node(k, &path)
        } else {
            k.chown(&path, identity.uid, identity.gid)
        };
        match given {
            Ok(()) => granted.push(path),
            Err(e) => journal.emit(
                k,
                Severity::Warn,
                service,
                corr,
                "a device node this application declared could not be given to it, so it will \
                 find that one unopenable. The others were granted",
                vec![
                    ("path".to_string(), Value::Str(path)),
                    ("detail".to_string(), Value::Str(e.to_string())),
                ],
            ),
        }
    }
    granted
}

/// **A render node is shared, not handed over** — `root:render`, `0660`.
///
/// Any number of processes render on one GPU, and the compositor and every
/// application declaring `gpu` hold this node at once, each under its own uid.
/// A chown to one uid gives the node to whichever was granted last and takes
/// it from the others. So the node is owned by root and the render group, and
/// [`groups_for`] hands that group to exactly the services granted it. A card
/// node is not shared: DRM master is exclusive, and it keeps the chown.
fn share_render_node(k: &mut dyn Kernel, path: &str) -> Result<(), crate::kernel::Errno> {
    k.chown(path, ROOT.0, rivet_core::etc::RENDER_GID)?;
    k.chmod(path, RENDER_NODE_MODE)
}

/// A render node's mode: read and write for root and the render group, and
/// nothing for anybody else — never `0666`, which would make the declaration
/// a comment.
const RENDER_NODE_MODE: u32 = 0o660;

/// The supplementary groups a grant of these nodes earns: the render group
/// when it includes a render node, and nothing otherwise. The caller adds them
/// to the program it spawns, so the group is held by exactly the services the
/// node was granted to.
pub fn groups_for(nodes: &[String]) -> Vec<u32> {
    if nodes.iter().any(|n| is_render_node(n)) {
        vec![rivet_core::etc::RENDER_GID]
    } else {
        Vec::new()
    }
}

/// Put every node back to `root:root` — a render node back to its shared
/// form, because another running service may hold it through the group.
///
/// Takes the same declaration rather than a remembered list, so a supervisor
/// that restarted with a changed manifest revokes what the machine *has* rather
/// than what it once granted. Infallible by design: a node that has gone away
/// needs no revoking, and there is nothing useful to do about one that will not
/// change hands — reporting it is the whole of the response.
///
/// **The role must be the one the grant used**, and the caller derives both from
/// the same definition for that reason: revoking a lessee's set from something
/// that was granted the arbiter's would leave `card*` owned by a stopped
/// program, which is the permission-held-by-nothing this function exists to
/// prevent.
pub fn revoke(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    service: &str,
    corr: &str,
    devices: &[Device],
    role: DisplayRole,
) {
    for path in nodes(k, devices, role) {
        let taken = if is_render_node(&path) {
            share_render_node(k, &path)
        } else {
            k.chown(&path, ROOT.0, ROOT.1)
        };
        if let Err(e) = taken {
            journal.emit(
                k,
                Severity::Warn,
                service,
                corr,
                "a device node could not be taken back from a service that stopped, so it is \
                 still owned by that application's uid. devtmpfs recreates it at the next boot, \
                 which is when this corrects itself",
                vec![
                    ("path".to_string(), Value::Str(path)),
                    ("detail".to_string(), Value::Str(e.to_string())),
                ],
            );
        }
    }
}

/// Unbind whichever `vtconsole` drives the framebuffer.
///
/// `PLAN.md` §104.43. Writing `0` to a `vtconsole`'s `bind` is the documented
/// way; `cmd/rivet-initd` does the same thing for the splash and this is the
/// same write for the same reason, at the moment an application is given a
/// screen instead.
///
/// **Every failure is survivable and none is fatal.** A kernel built without the
/// attribute has no such file, and a console that stays bound costs a screen
/// that flickers rather than a machine that does not work — so this reports and
/// carries on, exactly as `cmd/rivet-initd` does.
///
/// **Not undone.** There is nothing to restore it for: once an application has
/// been given the display, a console drawing on it is the defect. A machine
/// that wants its console back reboots without granting one, which is what
/// happens already on a board with no display application.
fn quiet_the_framebuffer_console(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    service: &str,
    corr: &str,
) {
    // `vtcon1` on this board, `vtcon0` on some. Both are tried rather than one
    // being detected: the read that would tell them apart is another thing to
    // get wrong, and writing `0` to a console that is not the framebuffer's is
    // refused by the kernel rather than harmful.
    let mut unbound = Vec::new();
    for which in ["vtcon0", "vtcon1"] {
        let path = format!("/sys/class/vtconsole/{which}/bind");
        if k.write_file(&path, b"0", 0o644).is_ok() {
            unbound.push(which);
        }
    }
    journal.emit(
        k,
        if unbound.is_empty() {
            Severity::Warn
        } else {
            Severity::Info
        },
        service,
        corr,
        if unbound.is_empty() {
            "the framebuffer console could not be unbound, so kernel and journal output may draw \
             over whatever this application puts on the screen. It is cosmetic and it is loud: \
             every line the console prints lands on top of the frame (PLAN.md §104.43)"
        } else {
            "the framebuffer console is unbound, so nothing else draws on the screen this \
             application was given. It is NOT rebound: once an application has the display, a \
             console drawing on it is the defect rather than a feature"
        },
        vec![(
            "vtconsole".to_string(),
            Value::Str(if unbound.is_empty() {
                "none".to_string()
            } else {
                unbound.join(", ")
            }),
        )],
    );
}

/// **Point 3: one owner at a time.** Which services collide over an exclusive
/// class, given every definition at once.
///
/// Returns `(device, names)` per contested class, with the names in the order
/// the definitions were given — so the refusal can name the one that will run
/// and the ones that will not.
///
/// Pure, and it takes the whole set because that is the question: no single
/// definition is wrong, and only something holding all of them can see the
/// conflict. That is §6.2's argument for the supervisor being the arbiter,
/// written as a function signature.
/// # Why nothing calls this — answered 2026-09-01 by `§107.5`'s audit
///
/// `§104.45.5` asked for *"either wire it or say why it exists"*. Saying why is
/// the right answer, and it is more interesting than the dead code:
///
/// **`display` is arbitrated, but not by refusing.** `Supervisor::offered_displays`
/// and the display gate let SEVERAL applications declare a display and start
/// only the selected one. That is arbitration by selection, and it is better
/// than this function's refusal: a machine with two kiosk applications
/// installed is an ordinary machine, and refusing to converge would be wrong.
///
/// **`input` is not arbitrated at all, and must not be.** `Device::exclusive`
/// says `Input` is exclusive; the launcher requires the opposite. It goes on
/// reading `/dev/input` the whole time an application holds the display —
/// **that is how LONG BACK works at all** (`§106`, `§108.1`). So this function,
/// wired as written, would refuse the exact arrangement the kiosk depends on.
///
/// Which leaves a real question rather than dead weight: **`Device::Input`'s
/// `exclusive() == true` is probably wrong**, and the model has not caught up
/// with the launcher. Recorded in `PLAN.md` §104.45.5 rather than changed here,
/// because it is a manifest-semantics decision and not a cleanup.
#[must_use]
#[allow(dead_code)]
pub fn contested(defs: &[(String, Vec<Device>)]) -> Vec<(Device, Vec<String>)> {
    let mut out = Vec::new();
    for device in Device::ALL {
        if !device.exclusive() {
            continue;
        }
        let claimants: Vec<String> = defs
            .iter()
            .filter(|(_, declared)| declared.contains(device))
            .map(|(name, _)| name.clone())
            .collect();
        if claimants.len() > 1 {
            out.push((*device, claimants));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **A display grant takes the console off the screen**, and a grant that
    /// includes no display leaves it alone.
    ///
    /// `PLAN.md` §104.43. The failure this guards is a machine that stopped
    /// competing with itself only on the boot path where the splash happened to
    /// run — which is what it did, and what the maintainer watched happen: a
    /// screen full of journal lines before an application appeared.
    #[test]
    fn granting_a_display_takes_the_console_off_the_screen() {
        let identity = AppIdentity { uid: 5, gid: 5 };

        let mut k = crate::testkernel::FakeKernel::new();
        let mut j = Journal::new("boot".to_string(), 1, "test".to_string());
        grant(
            &mut k,
            &mut j,
            "a",
            "corr",
            &[Device::Display],
            DisplayRole::Arbiter,
            identity,
        );
        assert!(
            k.writes
                .iter()
                .any(|w| w.path.contains("vtconsole") && w.path.ends_with("bind")),
            "a display grant left the console bound: {:?}",
            k.writes.iter().map(|w| &w.path).collect::<Vec<_>>()
        );

        let mut k = crate::testkernel::FakeKernel::new();
        let mut j = Journal::new("boot".to_string(), 1, "test".to_string());
        grant(
            &mut k,
            &mut j,
            "a",
            "corr",
            &[Device::Input],
            DisplayRole::Lessee,
            identity,
        );
        assert!(
            !k.writes.iter().any(|w| w.path.contains("vtconsole")),
            "an input-only grant touched the console: {:?}",
            k.writes.iter().map(|w| &w.path).collect::<Vec<_>>()
        );
    }

    /// A board that has DRM loaded: a framebuffer, two card nodes and a render
    /// node.
    ///
    /// **Labelled by provenance** — `AGENTS.md` §4. `card0`, `card1` and
    /// `renderD128` is what a Raspberry Pi 4 with both VC4 and V3D bound
    /// reported under `/dev/dri`, read off the board and recorded in
    /// `docs/PLAN-Archive-08-2026-08-30-to-08-31.md`. It is not a set somebody
    /// composed to make a test pass: `card1` is there because the Pi has two
    /// DRM devices, and a glob that expanded only the first would be wrong in a
    /// way a one-card fixture could never show.
    fn board_with_drm() -> crate::testkernel::FakeKernel {
        let mut k = crate::testkernel::FakeKernel::new();
        k.dir_entries.insert(
            "/dev".to_string(),
            vec!["fb0".to_string(), "null".to_string(), "tty1".to_string()],
        );
        k.dir_entries.insert(
            "/dev/dri".to_string(),
            vec![
                "card0".to_string(),
                "card1".to_string(),
                "renderD128".to_string(),
            ],
        );
        k
    }

    /// **A LESSEE is given the render node and nothing else** — `ADR-036` §3.1
    /// as amended 2026-09-04 (`.rulings/` R-138), register `R-32`.
    ///
    /// The defect this replaces: the grant chowned `/dev/dri/card*` and
    /// `/dev/fb0` to every application that declared `display`, so an
    /// application handed a lease could open the card by path instead and take
    /// DRM master the moment the launcher's own dropped — *"no compatibility
    /// path"* being untrue by the width of two device nodes.
    #[test]
    fn a_lessee_is_granted_the_render_node_and_neither_card_nor_framebuffer() {
        let identity = AppIdentity { uid: 5, gid: 5 };
        let mut k = board_with_drm();
        let mut j = Journal::new("boot".to_string(), 1, "test".to_string());
        let granted = grant(
            &mut k,
            &mut j,
            "a-kiosk-application",
            "corr",
            &[Device::Display],
            DisplayRole::Lessee,
            identity,
        );

        assert_eq!(
            granted,
            vec!["/dev/dri/renderD128".to_string()],
            "a lessee was granted more than the render node"
        );
        // The chowns are the enforcement; the returned list is the report of
        // them. Asserting both is what stops a future refactor satisfying one.
        assert!(
            !k.chowns.iter().any(|(p, _, _)| p.contains("card")),
            "a card node changed hands: {:?}",
            k.chowns
        );
        assert!(
            !k.chowns.iter().any(|(p, _, _)| p == "/dev/fb0"),
            "the framebuffer changed hands: {:?}",
            k.chowns
        );
    }

    /// **And the ARBITER still gets everything it had.** The narrowing is about
    /// the lessee; taking the card away from the one program that holds DRM
    /// master would leave the machine with nobody able to drive the screen —
    /// which is a worse outcome than the defect being repaired.
    #[test]
    fn the_arbiter_is_still_granted_the_card_and_the_framebuffer() {
        let identity = AppIdentity { uid: 5, gid: 5 };
        let mut k = board_with_drm();
        let mut j = Journal::new("boot".to_string(), 1, "test".to_string());
        let granted = grant(
            &mut k,
            &mut j,
            "rivet-launcher",
            "corr",
            &[Device::Display],
            DisplayRole::Arbiter,
            identity,
        );

        for expected in [
            "/dev/dri/card0",
            "/dev/dri/card1",
            "/dev/dri/renderD128",
            "/dev/fb0",
        ] {
            assert!(
                granted.iter().any(|p| p == expected),
                "the arbiter was not granted {expected}: {granted:?}"
            );
        }
    }

    /// **The revoke covers what the grant covered, and no more.** A lessee that
    /// stopped must not leave a chown of a node it was never given: `revoke`
    /// walks the same expansion, so an arbiter's `card*` is not silently
    /// touched on a lessee's stop edge.
    #[test]
    fn a_lessee_revoke_puts_back_the_render_node_only() {
        let mut k = board_with_drm();
        let mut j = Journal::new("boot".to_string(), 1, "test".to_string());
        revoke(
            &mut k,
            &mut j,
            "a-kiosk-application",
            "corr",
            &[Device::Display],
            DisplayRole::Lessee,
        );
        assert_eq!(
            k.chowns,
            vec![("/dev/dri/renderD128".to_string(), 0, rivet_core::etc::RENDER_GID)],
            "a lessee's revoke reached beyond the render node, or took it out of its shared form"
        );
    }

    /// Whether a process running as `uid` with supplementary `groups` may open
    /// `path` for reading and writing, by the last owner and mode the kernel
    /// was told. A devtmpfs node starts `0600 root:root`.
    fn opens(k: &crate::testkernel::FakeKernel, path: &str, uid: u32, groups: &[u32]) -> bool {
        let (owner, group) = k
            .chowns
            .iter()
            .rev()
            .find(|(p, _, _)| p == path)
            .map_or((0, 0), |(_, u, g)| (*u, *g));
        let mode = k
            .chmods
            .iter()
            .rev()
            .find(|(p, _)| p == path)
            .map_or(0o600, |(_, m)| *m);
        (uid == owner && mode & 0o600 == 0o600)
            || (groups.contains(&group) && mode & 0o060 == 0o060)
    }

    /// **Two holders of the render node both open it, whichever starts
    /// first.** The compositor holds it as the display arbiter and an
    /// application holds it by declaring `gpu`; both run under their own uids.
    /// A grant that chowns the node to one uid hands it to whichever was
    /// granted last and takes it from the other — which is what a board did,
    /// with the compositor stopping on `Permission denied` and the
    /// application's next restart due to fail the same way.
    #[test]
    fn two_holders_of_the_render_node_both_open_it_in_either_order() {
        let compositor = AppIdentity { uid: 23_470, gid: 23_470 };
        let application = AppIdentity { uid: 35_766, gid: 35_766 };
        let holders = [
            (compositor, Device::Display, DisplayRole::Arbiter),
            (application, Device::Gpu, DisplayRole::Lessee),
        ];
        for order in [[0, 1], [1, 0]] {
            let mut k = board_with_drm();
            let mut j = Journal::new("boot".to_string(), 1, "test".to_string());
            let mut earned = Vec::new();
            for &i in &order {
                let (identity, device, role) = holders[i];
                let granted = grant(&mut k, &mut j, "svc", "corr", &[device], role, identity);
                earned.push((identity, groups_for(&granted)));
            }
            for (identity, groups) in &earned {
                assert!(
                    opens(&k, "/dev/dri/renderD128", identity.uid, groups),
                    "uid {} cannot open the render node in order {order:?}; chowns {:?}, chmods \
                     {:?}, groups {groups:?}",
                    identity.uid,
                    k.chowns,
                    k.chmods
                );
            }
            assert!(
                !opens(&k, "/dev/dri/renderD128", 4_242, &[]),
                "a uid that was granted nothing opens the render node"
            );
            assert!(
                opens(&k, "/dev/dri/card0", compositor.uid, &[])
                    && !opens(&k, "/dev/dri/card0", application.uid, &earned[0].1),
                "a card node is the arbiter's alone"
            );
        }
    }

    /// The `input` class is the same set whichever side of the arbitration the
    /// service is on — the launcher reads `/dev/input` the whole time an
    /// application holds the screen, which is how LONG BACK works at all.
    #[test]
    fn the_input_class_does_not_depend_on_the_display_role() {
        let mut k = crate::testkernel::FakeKernel::new();
        let arbiter = nodes(&mut k, &[Device::Input], DisplayRole::Arbiter);
        let lessee = nodes(&mut k, &[Device::Input], DisplayRole::Lessee);
        assert_eq!(arbiter, lessee);
    }

    fn defs(rows: &[(&str, &[Device])]) -> Vec<(String, Vec<Device>)> {
        rows.iter()
            .map(|(name, d)| ((*name).to_string(), d.to_vec()))
            .collect()
    }

    #[test]
    fn one_claimant_is_not_a_conflict() {
        let found = contested(&defs(&[("kiosk", &[Device::Display, Device::Input])]));
        assert!(found.is_empty());
    }

    #[test]
    fn two_claimants_of_one_class_are_named_in_order() {
        let found = contested(&defs(&[
            ("kiosk", &[Device::Display]),
            ("other", &[Device::Display]),
        ]));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, Device::Display);
        assert_eq!(found[0].1, vec!["kiosk".to_string(), "other".to_string()]);
    }

    /// Two classes contested at once are two findings, because an operator
    /// fixing one has not fixed the other.
    #[test]
    fn each_contested_class_is_reported_separately() {
        let found = contested(&defs(&[
            ("a", &[Device::Display, Device::Input]),
            ("b", &[Device::Display, Device::Input]),
        ]));
        assert_eq!(found.len(), 2);
    }

    /// Services that declare nothing never contest anything, which is every
    /// application written before 2026-08-31.
    #[test]
    fn declaring_nothing_contests_nothing() {
        let found = contested(&defs(&[("a", &[]), ("b", &[]), ("c", &[Device::Input])]));
        assert!(found.is_empty());
    }
}
