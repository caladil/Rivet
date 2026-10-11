//! **The splash returns when the display's owner exits.** `ADR-015` §6.2 point
//! 4, built 2026-08-31.
//!
//! The maintainer's rule (.rulings/ R-269): the splash screen goes back up
//! whenever any program exits.
//!
//! §6.2 is explicit that this is not a courtesy:
//!
//! > It makes the display something the supervisor **converges**, exactly as
//! > `ADR-017` §4.1 has it converge services: there is a desired state — *the
//! > splash, unless an owner is running* — an owner displaces it for a while,
//! > and the supervisor restores it when that owner stops. **A screen left
//! > showing a dead program's last frame is drift**, and drift is the thing
//! > convergence exists to remove.
//!
//! `cmd/rivet-splash` was written as a one-shot blit that opens, draws and
//! exits holding nothing — §6.1 records that there is no acquire and no release
//! on `/dev/fb0` — so restoring the idle state is simply running it again.
//!
//! # Not on a trial boot, for pid 1's reason
//!
//! `cmd/rivet-initd/src/splash.rs` refuses to draw at all during a trial boot,
//! on the maintainer's requirement that the OS splash screen be disabled during
//! a trial restart so that any errors can be seen (.rulings/ R-345). A trial
//! boot is the one somebody is watching.
//!
//! **Restoring it later would undo that from the other end.** So this asks the
//! same question pid 1 asks, off the same file — the firmware's
//! `/proc/device-tree/chosen/bootloader/tryboot` property — rather than
//! inventing a second answer (`ADR-011`).
//!
//! It reads the file directly instead of going through
//! `rivet_adapter_image::BootSlots`, which would need a `BootMedium` this
//! binary has no other use for. **That is a duplicated reading and it is
//! named**: if the firmware ever reports a trial boot another way, this is a
//! second place that has to learn it.
//!
//! # Every failure is silent about the screen and loud in the journal
//!
//! Pid 1's rule, kept: a splash that cannot be restored is a plainer screen and
//! never a reason to fail a service that has already stopped.

use rivet_core::package::application::Device;

use crate::journal::{Journal, Severity, Value};
use crate::kernel::{Errno, Kernel, Program};

/// Where `cmd/rivet-image` installs it.
///
/// Spelled here, in `cmd/rivet-initd/src/splash.rs` and in `cmd/rivet-image`'s
/// `SPLASH_PATH` — the same arrangement every binary pid 1 starts already has.
/// `ADR-014` §3 keeps the image builder out of these crates' dependencies, so
/// the agreement is a convention the card build enforces by putting the file
/// there.
const SPLASH_PATH: &str = "/system/bin/rivet-splash";

/// The firmware's own answer to *"is this a one-shot trial boot"*.
const DT_TRYBOOT: &str = "/proc/device-tree/chosen/bootloader/tryboot";

/// `ADR-035`'s port, for a machine whose document does not say.
const DEFAULT_PORT: u16 = 7777;

/// **How long the supervisor's loop waits for the splash: two seconds** —
/// `ADR-017` §3. Past it the splash is killed and the screen keeps what it
/// drew.
///
/// The splash's own work is one paint: it opens `/dev/fb0`, draws, and exits.
/// A run with no framebuffer exits in under twenty milliseconds on a Pi 4. The
/// paint itself has not been timed apart from the rest, and is one write of a
/// few megabytes to memory the kernel maps, so two seconds is a margin of
/// tens of times over any run that is not stuck.
///
/// **It deliberately cuts one thing short**: with the endpoint line enabled
/// and no address yet, the splash stays to watch for one, for up to ninety
/// seconds. Here that watch would be ninety seconds of a supervisor that reaps
/// nothing and reads no signal. The picture is already on the screen by then,
/// so what is lost is the endpoint line arriving later — on a machine that has
/// no address, during a stop edge.
///
/// **And it sits inside `rivet_core::shutdown::SUPERVISOR_MARGIN_MS`**, so a
/// hung splash on a stop edge during a shutdown cannot on its own carry the
/// supervisor past pid 1's grace.
const SPLASH_DEADLINE_MS: u64 = 2_000;

const _INSIDE_THE_SHUTDOWN_MARGIN: () = assert!(
    SPLASH_DEADLINE_MS < rivet_core::shutdown::SUPERVISOR_MARGIN_MS,
    "a splash that may hold the loop longer than the supervisor's shutdown margin can carry it \
     past pid 1's grace"
);

/// Whether this service holding the display means the splash should come back.
#[must_use]
pub fn owns_display(devices: &[Device]) -> bool {
    devices.contains(&Device::Display)
}

/// Is this boot a one-shot trial? `false` when the question cannot be answered.
///
/// **Defaulting to "not a trial" is the direction that draws**, and it is the
/// safe one here: the cost of drawing on a trial boot is a covered console, and
/// the cost of not drawing on an ordinary boot is a screen left showing a dead
/// program's last frame — which is the drift §6.2 exists to remove. A machine
/// that cannot answer is overwhelmingly a machine with no Raspberry Pi firmware
/// under it, which is every QEMU guest.
fn in_trial_boot(k: &mut dyn Kernel) -> bool {
    // Four bytes is the whole property; the bound is the port's and a
    // generous one is still a bound on a file this build did not write.
    let Ok(bytes) = k.read_file_bytes(DT_TRYBOOT, 64) else {
        return false;
    };
    // A device-tree u32 is four big-endian bytes. Anything else is a property
    // this build does not understand, and "not a trial" is the answer that
    // keeps the screen converging.
    match bytes.as_slice() {
        [a, b, c, d] => u32::from_be_bytes([*a, *b, *c, *d]) != 0,
        _ => false,
    }
}

/// Put the splash back.
///
/// Called on the stop edge of a service that held [`Device::Display`]. The
/// device grant has already been revoked by then, so the node is `root:root`
/// again and the supervisor can write it.
pub fn restore(k: &mut dyn Kernel, journal: &mut Journal, service: &str, corr: &str) {
    if in_trial_boot(k) {
        journal.emit(
            k,
            Severity::Info,
            service,
            corr,
            "the display's owner stopped and the splash was NOT restored, because this is a \
             trial boot and pid 1 deliberately left the console visible. Restoring it here \
             would undo that from the other end",
            vec![],
        );
        return;
    }

    let program = Program {
        path: SPLASH_PATH.to_string(),
        args: arguments(k),
        env: Vec::new(),
        // Root, like pid 1's own invocation. The splash writes a device node
        // that an application was just given and has just had taken back.
        identity: None,
        confinement: None,
        groups: Vec::new(),
        capabilities: Vec::new(),
    };

    match k.run_within(&program, SPLASH_DEADLINE_MS) {
        Ok(done) if done.status.is_success() => journal.emit(
            k,
            Severity::Info,
            service,
            corr,
            "the display's owner stopped, so the splash was drawn again — ADR-015 §6.2 makes \
             the screen something this supervisor converges, and a screen left showing a dead \
             program's last frame is drift",
            vec![],
        ),
        Ok(done) => journal.emit(
            k,
            Severity::Warn,
            service,
            corr,
            "the display's owner stopped and the splash refused to draw, so the screen still \
             shows whatever that program left. The machine is otherwise unaffected",
            vec![("status".to_string(), Value::Str(done.status.to_string()))],
        ),
        Err(e) if e == Errno::ETIMEDOUT => journal.emit(
            k,
            Severity::Warn,
            service,
            corr,
            "the display's owner stopped and the splash did not finish within its deadline, so \
             it was stopped and the screen shows whatever it had drawn by then. A splash still \
             watching for an address is stopped the same way, and its endpoint line does not \
             arrive",
            vec![(
                "deadline_ms".to_string(),
                Value::Int(i64::try_from(SPLASH_DEADLINE_MS).unwrap_or(i64::MAX)),
            )],
        ),
        Err(e) => journal.emit(
            k,
            Severity::Warn,
            service,
            corr,
            "the display's owner stopped and the splash could not be started, so the screen \
             still shows whatever that program left. A card without rivet-splash is a \
             supported card (ADR-005 §7.4), so this is not a fault on its own",
            vec![("detail".to_string(), Value::Str(e.to_string()))],
        ),
    }
}

/// What the document says the screen may show.
///
/// **The same composition pid 1 does**, through the same
/// `rivet_config::splash::Splash` — so the restored screen is the one the boot
/// drew rather than a second opinion about it. Every failure yields the
/// default, on pid 1's argument: a document that cannot be read is a machine
/// with bigger problems than its screen.
fn arguments(k: &mut dyn Kernel) -> Vec<String> {
    let Ok(text) = k.read_file(rivet_core::state::MACHINE_DOCUMENT) else {
        return rivet_config::splash::Splash::default().arguments(DEFAULT_PORT, None);
    };
    match rivet_config::Document::parse(&text) {
        Ok(document) => {
            let name = rivet_config::splash::Splash::endpoint_name(&document);
            document.effective_splash().arguments(
                document
                    .effective_control_plane()
                    .port()
                    .unwrap_or(DEFAULT_PORT),
                name.as_deref(),
            )
        }
        Err(_) => rivet_config::splash::Splash::default().arguments(DEFAULT_PORT, None),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn only_a_display_owner_brings_the_splash_back() {
        assert!(owns_display(&[Device::Display]));
        assert!(owns_display(&[Device::Input, Device::Display]));
        assert!(!owns_display(&[Device::Input]));
        assert!(!owns_display(&[]));
    }
}
