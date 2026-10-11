//! **Starting the boot splash, and knowing when not to.**
//!
//! `cmd/rivet-splash` draws the shield, the name and the version on `/dev/fb0`
//! once and exits. This is the twenty lines that decide whether it runs.
//!
//! # The decision is `in_trial_boot`, and nothing else
//!
//! The maintainer's requirement, 2026-08-30: the OS splash screen is disabled
//! during a trial restart so that any errors can be seen (.rulings/ R-345). A
//! trial boot is the one somebody is watching — it is the boot that may be
//! about to be rolled back — so covering its console with a logo would hide the
//! output at exactly the moment it is worth most.
//!
//! **The first design put this in the kernel command line** and would have
//! failed silently. `config.txt` and `tryboot.txt` are byte-identical on
//! purpose: `ADR-018` §5.3 records that `tryboot_a_b=1` makes a tryboot read
//! the ordinary `config.txt`, while a card without `autoboot.txt` was observed
//! reading `tryboot.txt` — so **which file the firmware reads depends on state
//! the image builder cannot see**. A quiet cmdline in one and a verbose cmdline
//! in the other would have given a trial boot the quiet one, which is the
//! opposite of the requirement.
//!
//! The slot primitive answers it directly and pid 1 already holds one, so that
//! is what is asked.
//!
//! # Drawing last was not enough, and a photograph of the television said so
//!
//! The first build put the splash after everything pid 1 had to say and
//! stopped there. It drew, and then it was painted over — a capture at
//! 2026-08-30T22:47 has the shield gone under `rivet-mcpd` connection records
//! at eight seconds and `not a tryboot boot` at thirty-three. **Three writers
//! own that framebuffer**: the kernel console, pid 1, and the supervisor
//! echoing every journal record to the stdout it inherits from pid 1.
//!
//! So the splash TAKES the console — [`crate::kernel::Kernel::quiet_console`] —
//! and the order matters enough to be a test: taken first, drawn second.
//!
//! # Every failure is silent about the screen and loud in the journal
//!
//! A splash that cannot be started is a plainer boot. It is never a reason to
//! stop, which is why nothing here returns an error.

use rivet_adapter_image::BootSlots;

use crate::kernel::Kernel;
use crate::slots::KernelMedium;

/// Where `cmd/rivet-image` installs it.
///
/// **Spelled here and in `cmd/rivet-image`'s `SPLASH_PATH`**, which is the same
/// arrangement every other binary pid 1 starts already has: `ADR-014` §3 keeps
/// the image builder out of pid 1's dependencies, so the agreement is a
/// convention the card build enforces by putting the file there.
const SPLASH_PATH: &str = "/system/bin/rivet-splash";

/// What the document says the screen may show.
///
/// # Read here, and defaulted rather than refused
///
/// The stored document is the authority — it is what `apply edit` changes, so
/// reading the provisioning file instead would ignore an operator who turned
/// the endpoint line off after first boot. **Every failure yields the default**,
/// which is both lines: a document that cannot be read is a machine with bigger
/// problems than its screen, and refusing to draw would take away the picture
/// AND the diagnosis.
fn arguments(k: &mut dyn Kernel) -> Vec<String> {
    let Ok(text) = k.read_file(rivet_core::state::MACHINE_DOCUMENT) else {
        // No stored document is an ordinary first boot, not a fault.
        return rivet_config::splash::Splash::default().arguments(DEFAULT_PORT, None);
    };
    match rivet_config::Document::parse(&text) {
        Ok(document) => {
            // **The name is decided by the document, not by this file.** Both a
            // hostname and an enabled `rivet-mdnsd` are required, because
            // either alone gives a name nothing resolves —
            // `rivet_config::splash::Splash::endpoint_name` holds that
            // reasoning so there is one place it can be wrong.
            let name = rivet_config::splash::Splash::endpoint_name(&document);
            document.effective_splash().arguments(
                document
                    .effective_control_plane()
                    .port()
                    .unwrap_or(DEFAULT_PORT),
                name.as_deref(),
            )
        }
        Err(why) => {
            // Said rather than swallowed: a document pid 1 cannot parse is the
            // thing `PLAN.md` §102.18 cost two reboots to find.
            k.say(&format!(
                "rivet-initd: the stored document could not be read for the splash ({why}); the \
                 screen shows both lines, which is the default"
            ));
            rivet_config::splash::Splash::default().arguments(DEFAULT_PORT, None)
        }
    }
}

/// `ADR-035`'s port, for a machine whose document does not say.
const DEFAULT_PORT: u16 = 7777;

/// Draw the splash, unless this is a trial boot.
pub fn maybe_draw(k: &mut dyn Kernel, slots: &dyn BootSlots) {
    let trial = {
        let mut medium = KernelMedium::new(k);
        slots.in_trial_boot(&mut medium)
    };

    match trial {
        Ok(true) => {
            k.say(
                "rivet-initd: TRIAL BOOT - the splash is not drawn, so the console stays visible. \
                 This is the boot that may be rolled back and its output is the point",
            );
        }
        Ok(false) => {
            // **Said BEFORE the console is taken**, so the last thing on a
            // screen that is about to stop updating explains why it stopped.
            k.say(
                "rivet-initd: the splash is being drawn, so the console stops here. Every \
                 line after this one is in the journal - `cargo rivet run -- get log`",
            );
            // **Then TAKE the screen, and only then draw on it.** Drawing last
            // is not enough and the first attempt proved it: the picture went
            // up at six seconds and was gone under console output by eight.
            let taken = k.quiet_console();
            // Composed before the spawn rather than inside it: both borrow the
            // kernel, and reading the document is a separate act from starting
            // the program that draws what it says.
            let args = arguments(k);
            match k.spawn_with(SPLASH_PATH, &args) {
                Ok(pid) => k.say(&format!(
                    "rivet-initd: started {SPLASH_PATH} as pid {pid}; console taken: {taken}"
                )),
                // A card built without `--splash` has no such file, which is a
                // supported card and not a fault. The console has already been
                // taken by this point and is deliberately NOT given back: undoing
                // it would need a second set of failure paths for the case where
                // the giving-back fails, and the machine is reachable over the
                // control plane either way.
                Err(e) => k.say(&format!(
                    "rivet-initd: no splash ({SPLASH_PATH}: {e}); the screen stays as it was"
                )),
            }
        }
        Err(why) => {
            // **Not drawn when the answer is unknown.** A machine that cannot
            // say whether it is on trial is one whose console is worth more
            // than its logo.
            k.say(&format!(
                "rivet-initd: cannot tell whether this is a trial boot ({why}), so the splash is \
                 NOT drawn and the console stays visible"
            ));
        }
    }
}

#[cfg(test)]
#[path = "splash_tests.rs"]
mod tests;
