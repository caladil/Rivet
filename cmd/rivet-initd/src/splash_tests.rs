//! **The order the television proved was load-bearing**, plus the trial-boot
//! carve-out.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::testkernel::FakeKernel;
use rivet_adapter_image::{
    ArmRefusal, ArmedReboot, BootMedium, Committed, Listing, PayloadTarget, SlotId, SlotPayload,
    SlotRecord,
};

/// A `BootSlots` that answers one question, which is the only one the splash
/// asks. Every other method is unreachable rather than plausible: a double that
/// quietly answers a question its subject never asks is a test that stops
/// describing the code.
struct Slots(Result<bool, String>);

impl rivet_adapter_image::BootSlots for Slots {
    fn in_trial_boot(&self, _m: &mut dyn BootMedium) -> Result<bool, String> {
        self.0.clone()
    }
    fn platform(&self) -> &'static str {
        "a test"
    }
    fn platform_token(&self) -> &'static str {
        "test"
    }
    fn running_slot(&self, _m: &mut dyn BootMedium) -> Result<SlotId, String> {
        unreachable!("the splash does not ask which slot is running")
    }
    fn other_slot(&self, _slot: SlotId) -> Result<SlotId, String> {
        unreachable!("the splash does not ask about the other slot")
    }
    fn default_slot_record(&self) -> SlotRecord {
        unreachable!("the splash does not read the default-slot record")
    }
    fn default_slot(&self, _m: &mut dyn BootMedium) -> Result<SlotId, String> {
        unreachable!("the splash does not ask which slot is the default")
    }
    fn payload(&self, _slot: SlotId) -> Result<SlotPayload, String> {
        unreachable!("the splash does not look at payloads")
    }
    fn reserved_names(&self, _target: &PayloadTarget) -> &'static [&'static str] {
        unreachable!("the splash does not install anything")
    }
    fn commit(&self, _m: &mut dyn BootMedium, _slot: SlotId) -> Result<Committed, ArmRefusal> {
        unreachable!("the splash commits nothing")
    }
    fn record_medium_listing(
        &self,
        _m: &mut dyn BootMedium,
        _limit: usize,
    ) -> Result<Listing, String> {
        unreachable!("the splash lists nothing")
    }
    fn arm_trial_boot(
        &self,
        _m: &mut dyn BootMedium,
        _slot: SlotId,
    ) -> Result<ArmedReboot, ArmRefusal> {
        unreachable!("the splash arms nothing")
    }
    fn can_arm_trial_boot(
        &self,
        _m: &mut dyn BootMedium,
        _slot: SlotId,
    ) -> Result<(), ArmRefusal> {
        unreachable!("the splash arms nothing")
    }
}

fn draw(trial: Result<bool, String>) -> FakeKernel {
    let mut k = FakeKernel::new();
    maybe_draw(&mut k, &Slots(trial));
    k
}

/// **The defect this file exists for.** A capture of the television on
/// 2026-08-30 showed the shield drawn and then buried: the picture went up at
/// six seconds and console output was still arriving at thirty-three. Drawing
/// last is not enough — the screen has to be TAKEN, and taken BEFORE the draw,
/// or the window between the two is a race the picture loses.
#[test]
fn the_console_is_taken_before_the_splash_is_drawn() {
    let k = draw(Ok(false));
    let taken = k
        .order
        .iter()
        .position(|c| c == "quiet_console")
        .expect("the console was never taken; the splash will be painted over");
    let drawn = k
        .order
        .iter()
        .position(|c| c.contains("spawn"))
        .expect("the splash was never started");
    assert!(
        taken < drawn,
        "the splash was started before the console was taken: {:?}",
        k.order
    );
}

#[test]
fn an_ordinary_boot_starts_the_splash() {
    let k = draw(Ok(false));
    assert_eq!(k.spawn_calls, vec![SPLASH_PATH.to_string()]);
}

/// **The maintainer's requirement of 2026-08-30**, and the reason the whole
/// decision exists: a trial is the boot somebody is watching.
#[test]
fn a_trial_boot_draws_nothing_and_keeps_its_console() {
    let k = draw(Ok(true));
    assert!(k.spawn_calls.is_empty(), "{:?}", k.spawn_calls);
    assert_eq!(
        k.quiet_console_calls, 0,
        "a trial boot had its console taken, which is the failure this carve-out exists to prevent"
    );
    assert!(
        k.lines.iter().any(|l| l.contains("TRIAL BOOT")),
        "{:?}",
        k.lines
    );
}

/// A machine that cannot say which kind of boot this is keeps its console, for
/// the same reason: the doubt is itself worth seeing.
#[test]
fn an_unanswerable_slot_question_keeps_the_console() {
    let k = draw(Err("no boot partition".to_string()));
    assert!(k.spawn_calls.is_empty());
    assert_eq!(k.quiet_console_calls, 0);
    assert!(
        k.lines.iter().any(|l| l.contains("cannot tell")),
        "{:?}",
        k.lines
    );
}

/// **The last line on a screen that is about to freeze must explain itself**,
/// and it has to be written while the console can still show it.
#[test]
fn the_screen_says_why_it_stopped_before_it_stops() {
    let k = draw(Ok(false));
    let said = k
        .lines
        .iter()
        .position(|l| l.contains("the console stops here"))
        .expect("the console went quiet without saying why");
    let taken = k.order.iter().position(|c| c == "quiet_console").unwrap();
    // Both lists are ordered, so this compares the say that precedes the take
    // against every say that follows it.
    assert!(
        k.order.iter().take(taken).all(|c| !c.starts_with("spawn")),
        "{:?}",
        k.order
    );
    assert!(said < k.lines.len());
    assert!(
        k.lines[said].contains("get log"),
        "the last line must say where the output went: {}",
        k.lines[said]
    );
}

/// **What the document says reaches the screen.** The splash holds no policy:
/// pid 1 reads the document and translates, so a machine told to keep quiet
/// about itself is quiet because of one decision recorded in one place.
#[test]
fn the_documents_suppression_is_passed_to_the_splash() {
    let mut k = FakeKernel::new();
    k.files.insert(
        rivet_core::state::MACHINE_DOCUMENT.to_string(),
        crate::testkernel::Script::always(Ok(
            "version = 1
[splash]
endpoint = false
version = true
".to_string(),
        )),
    );
    maybe_draw(&mut k, &Slots(Ok(false)));
    let (_, args) = k.spawn_args.first().expect("the splash was started");
    assert!(args.contains(&"--no-endpoint".to_string()), "{args:?}");
    assert!(!args.contains(&"--no-version".to_string()), "{args:?}");
}

/// **The port comes from the document too**, so a machine whose control plane
/// was moved does not have a screen naming the port it used to be on.
#[test]
fn the_port_on_the_screen_is_the_one_the_document_names() {
    let mut k = FakeKernel::new();
    k.files.insert(
        rivet_core::state::MACHINE_DOCUMENT.to_string(),
        crate::testkernel::Script::always(Ok(
            "version = 1
[control-plane]
listen = \"0.0.0.0:9999\"
".to_string(),
        )),
    );
    maybe_draw(&mut k, &Slots(Ok(false)));
    let (_, args) = k.spawn_args.first().expect("started");
    assert!(args.contains(&"9999".to_string()), "{args:?}");
}

/// **A document that cannot be parsed still gets a picture**, and says so. It
/// is the machine with bigger problems than its screen, and refusing to draw
/// would take away the picture AND the diagnosis.
#[test]
fn an_unreadable_document_draws_the_default_and_says_why() {
    let mut k = FakeKernel::new();
    k.files.insert(
        rivet_core::state::MACHINE_DOCUMENT.to_string(),
        crate::testkernel::Script::always(Ok("this is not toml {{{".to_string())),
    );
    maybe_draw(&mut k, &Slots(Ok(false)));
    let (_, args) = k.spawn_args.first().expect("the splash was still started");
    assert!(!args.contains(&"--no-endpoint".to_string()), "{args:?}");
    assert!(
        k.lines.iter().any(|l| l.contains("could not be read for the splash")),
        "{:?}",
        k.lines
    );
}

/// A machine with no stored document at all — an ordinary first boot — gets
/// both lines rather than a refusal.
#[test]
fn a_first_boot_with_no_document_draws_both_lines() {
    let mut k = FakeKernel::new();
    maybe_draw(&mut k, &Slots(Ok(false)));
    let (_, args) = k.spawn_args.first().expect("started");
    assert_eq!(args, &vec!["--port".to_string(), "7777".to_string()]);
}

/// A card built before 2026-08-30 has no splash binary, which is a supported
/// card. It must not become a boot failure.
#[test]
fn a_card_without_the_binary_boots_and_says_so() {
    let mut k = FakeKernel::new();
    k.spawn_results = crate::testkernel::Script::always(Err(crate::kernel::Errno::ENOENT));
    maybe_draw(&mut k, &Slots(Ok(false)));
    assert!(
        k.lines.iter().any(|l| l.contains("no splash")),
        "{:?}",
        k.lines
    );
}
