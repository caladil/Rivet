// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::credentials::Overrides;
use rivet_core::package::port::Sign as _;
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicU32, Ordering};

/// A default, empty [`ProvisionArgs`] — every field `None`/`false`, the shape
/// `write_image::parse(&[])` itself produces.
fn empty_args() -> ProvisionArgs {
    ProvisionArgs {
        image_path: None,
        document_path: None,
        device_path: None,
        confirm: false,
        not_mine: false,
        at: None,
        verify_only: false,
        elevated_write_task: None,
        elevated_verify_task: None,
        elevated_task_digest: None,
        staging_path: None,
        identity_path: None,
        fleet_repo_root: None,
        offer_device_menu: false,
        image_from_repository: false,
    }
}

/// Records whether it was called, and returns a canned answer — never
/// touches a real credential.
struct FakePreflight {
    called: Cell<bool>,
    answer: Result<String, String>,
}

impl FakePreflight {
    fn ok() -> Self {
        Self { called: Cell::new(false), answer: Ok("preflight ok\n".to_owned()) }
    }
    fn err(why: &str) -> Self {
        Self { called: Cell::new(false), answer: Err(why.to_owned()) }
    }
}

impl Preflight for FakePreflight {
    fn check(&self) -> Result<String, String> {
        self.called.set(true);
        self.answer.clone()
    }
}

/// Records whether it was called, and returns a canned answer — never
/// touches a terminal or a subprocess.
struct FakeInterview {
    called: Cell<bool>,
    answer: Result<InterviewResult, String>,
}

impl FakeInterview {
    fn ok(path: &str) -> Self {
        Self {
            called: Cell::new(false),
            answer: Ok(InterviewResult {
                document_path: PathBuf::from(path),
                pin: None,
                identity_dir: None,
            }),
        }
    }
    /// As [`Self::ok`], but the interview also read a pin off its breadcrumb
    /// — `.rulings/` R-178 Q7.
    fn ok_with_pin(path: &str, pin: [u8; 32]) -> Self {
        Self {
            called: Cell::new(false),
            answer: Ok(InterviewResult {
                document_path: PathBuf::from(path),
                pin: Some(pin),
                identity_dir: None,
            }),
        }
    }
    fn err(why: &str) -> Self {
        Self { called: Cell::new(false), answer: Err(why.to_owned()) }
    }
}

impl Interview for FakeInterview {
    fn run(&self) -> Result<InterviewResult, String> {
        self.called.set(true);
        self.answer.clone()
    }
}

/// Records every document path it was asked to check, and returns a canned
/// answer — never touches a filesystem.
struct FakeStagingCheck {
    seen: RefCell<Vec<PathBuf>>,
    answer: Result<Option<PathBuf>, String>,
}

impl FakeStagingCheck {
    fn ok() -> Self {
        Self { seen: RefCell::new(Vec::new()), answer: Ok(None) }
    }
    /// **Wave C3**: as though this document required a staging bundle and one
    /// was found at `path` — the shape `run_with_image_selected` must thread
    /// into `args.staging_path` for the writer's own injection (item 1).
    fn ok_with_path(path: &str) -> Self {
        Self { seen: RefCell::new(Vec::new()), answer: Ok(Some(PathBuf::from(path))) }
    }
    fn err(why: &str) -> Self {
        Self { seen: RefCell::new(Vec::new()), answer: Err(why.to_owned()) }
    }
}

impl StagingCheck for FakeStagingCheck {
    fn check(&self, document_path: &Path, _identity: Option<&Path>) -> Result<(Option<PathBuf>, String), String> {
        self.seen.borrow_mut().push(document_path.to_path_buf());
        self.answer.clone().map(|path| (path, String::new()))
    }
}

/// `(staging_path, identity_path, fleet_repo_root)` — [`FakeWriter`]'s own
/// record of an injection-relevant call, named so its field reads rather
/// than tripping `clippy::type_complexity`.
type InjectionArgsSeen = (Option<PathBuf>, Option<PathBuf>, Option<PathBuf>);

/// Records every [`ProvisionArgs`] it was handed, and returns a canned
/// answer — never touches a disk.
struct FakeWriter {
    seen: RefCell<Vec<Option<PathBuf>>>,
    /// **Wave C3**: for the same call `seen` already recorded `document_path`
    /// for — a second field rather than widening `seen` itself, so every
    /// existing assertion against `seen` keeps reading exactly what it
    /// always has.
    injection_args_seen: RefCell<Vec<InjectionArgsSeen>>,
    /// `(confirm, offer_device_menu)` for every call — what the writer was
    /// told about the device menu and `Write it? [y/N]`.
    device_flags_seen: RefCell<Vec<(bool, bool)>>,
    answer: Result<write_image::WriteOutcome, String>,
}

impl FakeWriter {
    /// A device written and verified.
    fn ok(report: &str) -> Self {
        Self::answering(Ok(write_image::WriteOutcome { report: report.to_owned(), written: true }))
    }
    /// `Ok`, but nothing written — `N`, no removable device, or a prepared
    /// image alone.
    fn nothing_written(report: &str) -> Self {
        Self::answering(Ok(write_image::WriteOutcome { report: report.to_owned(), written: false }))
    }
    fn err(why: &str) -> Self {
        Self::answering(Err(why.to_owned()))
    }
    fn answering(answer: Result<write_image::WriteOutcome, String>) -> Self {
        Self {
            seen: RefCell::new(Vec::new()),
            injection_args_seen: RefCell::new(Vec::new()),
            device_flags_seen: RefCell::new(Vec::new()),
            answer,
        }
    }
}

impl Writer for FakeWriter {
    fn write(&self, args: &ProvisionArgs) -> Result<write_image::WriteOutcome, String> {
        self.seen.borrow_mut().push(args.document_path.clone());
        self.injection_args_seen.borrow_mut().push((
            args.staging_path.clone(),
            args.identity_path.clone(),
            args.fleet_repo_root.clone(),
        ));
        self.device_flags_seen.borrow_mut().push((args.confirm, args.offer_device_menu));
        self.answer.clone()
    }
}

/// Records every call to [`FleetRecorder::plan`] and [`FleetRecorder::commit`]
/// — never touches a real fleet repository or a real register. See
/// `RealFleetRecorder` (in `provision.rs`) for the real thing, exercised by
/// the scratch end-to-end test near the bottom of this file instead.
struct FakeFleetRecorder {
    plan_calls: RefCell<Vec<PathBuf>>,
    plan_pins: RefCell<Vec<Option<[u8; 32]>>>,
    plan_answer: Result<Option<rivetos_register::Device>, String>,
    commit_calls: RefCell<Vec<(PathBuf, Option<rivetos_register::Device>)>>,
    commit_answer: String,
}

impl FakeFleetRecorder {
    fn ok(plan: Option<rivetos_register::Device>) -> Self {
        Self {
            plan_calls: RefCell::new(Vec::new()),
            plan_pins: RefCell::new(Vec::new()),
            plan_answer: Ok(plan),
            commit_calls: RefCell::new(Vec::new()),
            commit_answer: "fleet repository: recorded\n".to_owned(),
        }
    }
    fn plan_err(why: &str) -> Self {
        Self {
            plan_calls: RefCell::new(Vec::new()),
            plan_pins: RefCell::new(Vec::new()),
            plan_answer: Err(why.to_owned()),
            commit_calls: RefCell::new(Vec::new()),
            commit_answer: "fleet repository: recorded\n".to_owned(),
        }
    }
}

impl FleetRecorder for FakeFleetRecorder {
    fn plan(
        &self,
        document_path: &Path,
        pin: Option<&[u8; 32]>,
    ) -> Result<Option<rivetos_register::Device>, String> {
        self.plan_calls.borrow_mut().push(document_path.to_path_buf());
        self.plan_pins.borrow_mut().push(pin.copied());
        self.plan_answer.clone()
    }

    fn commit(&self, document_path: &Path, plan: Option<rivetos_register::Device>) -> String {
        self.commit_calls.borrow_mut().push((document_path.to_path_buf(), plan));
        self.commit_answer.clone()
    }
}

/// Records every hostname it was asked to locate, and returns a canned
/// answer — never touches a real fleet repository or `~/.rivet`.
struct FakeExistingDocument {
    calls: RefCell<Vec<String>>,
    answer: Result<PathBuf, String>,
}

impl FakeExistingDocument {
    fn ok(path: &str) -> Self {
        Self { calls: RefCell::new(Vec::new()), answer: Ok(PathBuf::from(path)) }
    }
    fn err(why: &str) -> Self {
        Self { calls: RefCell::new(Vec::new()), answer: Err(why.to_owned()) }
    }
    /// The sentinel for every test where `--existing` is not on the route at
    /// all — `run_with` must never call this.
    fn never() -> Self {
        Self::err("must not be called: --existing is not on this route")
    }
}

impl ExistingDocument for FakeExistingDocument {
    fn locate(&self, hostname: &str) -> Result<PathBuf, String> {
        self.calls.borrow_mut().push(hostname.to_owned());
        self.answer.clone()
    }
}

/// Every fake bundled together, so a test that only cares about one of them
/// does not have to spell the other five.
struct Fakes {
    preflight: FakePreflight,
    interview: FakeInterview,
    staging_check: FakeStagingCheck,
    fleet_recorder: FakeFleetRecorder,
    writer: FakeWriter,
    existing_document: FakeExistingDocument,
    image_selector: FakeImageSelector,
}

impl Fakes {
    fn happy() -> Self {
        Self {
            preflight: FakePreflight::ok(),
            interview: FakeInterview::ok("/tmp/arbor.provisioning.toml"),
            staging_check: FakeStagingCheck::ok(),
            fleet_recorder: FakeFleetRecorder::ok(None),
            writer: FakeWriter::ok("wrote it"),
            existing_document: FakeExistingDocument::never(),
            image_selector: FakeImageSelector::ok(),
        }
    }

    fn run(&self, is_terminal: bool, args: ProvisionArgs) -> Result<String, String> {
        run_with(
            is_terminal,
            &self.preflight,
            &self.interview,
            &self.staging_check,
            &self.fleet_recorder,
            &self.writer,
            &self.image_selector,
            None,
            args,
            None,
            None,
            None,
            &self.existing_document)
    }
}

/// Records every call and returns a canned answer — never touches a real
/// fleet repository or any image bytes. See `RealImageSelector` (in
/// `provision.rs`) for the real thing, exercised by
/// `provision_image_tests.rs` and the scratch end-to-end test near the
/// bottom of this file instead.
struct FakeImageSelector {
    calls: RefCell<Vec<(Option<String>, Option<PathBuf>)>>,
    answer: Result<Option<SelectedImage>, String>,
}

impl FakeImageSelector {
    /// A literal image, as if `--image PATH` had named a real file — no
    /// scratch copy to clean up.
    fn ok() -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            answer: Ok(Some(SelectedImage {
                path: PathBuf::from("/fixture/selected.img"),
                scratch: None,
                descriptor: None,
                digest: None,
                version: None,
            })),
        }
    }
    /// As if the operator had left the platform or system-image menu with
    /// `x`/`X` — `run_with` must end the run right here, before the
    /// interview, the staging check or the writer are ever reached.
    fn exited() -> Self {
        Self { calls: RefCell::new(Vec::new()), answer: Ok(None) }
    }
    /// As selected FROM the fleet repository — a scratch working copy that
    /// `run_with` must remove once this run finishes, whichever way.
    fn ok_with_scratch(path: &str) -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            answer: Ok(Some(SelectedImage {
                path: PathBuf::from(path),
                scratch: Some(PathBuf::from(path)),
                descriptor: Some("fixture-product 0.0.0-fixture (abc123abc123)".to_owned()),
                digest: Some("abc123abc123fixturedigest".to_owned()),
                // An image recent enough to carry the proxy (`ADR-060`), so
                // the server and member documents below are not refused for
                // the image rather than for what each test is about.
                version: Some(rivet_core::service::PROXY_IN_IMAGE_SINCE.to_owned()),
            })),
        }
    }
    fn err(why: &str) -> Self {
        Self { calls: RefCell::new(Vec::new()), answer: Err(why.to_owned()) }
    }
}

impl ImageSelector for FakeImageSelector {
    fn select(
        &self,
        requested_platform: Option<&str>,
        requested_image: Option<&Path>,
    ) -> Result<Option<SelectedImage>, String> {
        self.calls
            .borrow_mut()
            .push((requested_platform.map(str::to_owned), requested_image.map(Path::to_path_buf)));
        self.answer.clone()
    }
}

// ---------------------------------------------------------------------------
// R-178 P3/item 6: `image_selector` runs before the interview, on every
// route, and its scratch working copy is removed once the run finishes --
// success or failure alike.
// ---------------------------------------------------------------------------

#[test]
fn image_selection_runs_before_the_interview_and_is_handed_platform_and_image_unchanged() {
    let preflight = FakePreflight::ok();
    let interview = FakeInterview::ok("/tmp/arbor.provisioning.toml");
    let staging_check = FakeStagingCheck::ok();
    let fleet_recorder = FakeFleetRecorder::ok(None);
    let writer = FakeWriter::ok("wrote it");
    let image_selector = FakeImageSelector::ok();

    let mut args = empty_args();
    args.image_path = Some(PathBuf::from("deadbeef1234"));

    run_with(
        true,
        &preflight,
        &interview,
        &staging_check,
        &fleet_recorder,
        &writer,
        &image_selector,
        Some("rpi4"),
        args,
        None,
        None,
        None,
        &FakeExistingDocument::never())
    .expect("every fake answers Ok");

    assert_eq!(
        image_selector.calls.borrow().as_slice(),
        [(Some("rpi4".to_owned()), Some(PathBuf::from("deadbeef1234")))],
    );
}

/// A refusal from `image_selector.select` stops the sequence before the
/// interview ever runs, and before preflight too — R-178 P3's whole point:
/// no image, no typing.
#[test]
fn a_refusal_from_image_selection_stops_before_preflight_and_the_interview() {
    let preflight = FakePreflight::ok();
    let interview = FakeInterview::err("must not run -- image selection refused first");
    let staging_check = FakeStagingCheck::ok();
    let fleet_recorder = FakeFleetRecorder::ok(None);
    let writer = FakeWriter::ok("must not run");
    let image_selector = FakeImageSelector::err("No images available for provisioning.");

    let why = run_with(
        true,
        &preflight,
        &interview,
        &staging_check,
        &fleet_recorder,
        &writer,
        &image_selector,
        None,
        empty_args(),
        None,
        None,
        None,
        &FakeExistingDocument::never())
    .expect_err("no images to select");

    assert_eq!(why, "No images available for provisioning.");
    assert!(!preflight.called.get(), "preflight must not run once image selection has refused");
    assert!(!interview.called.get());
    assert!(writer.seen.borrow().is_empty());
}

/// **An operator leaving the platform or system-image menu with `x`/`X`
/// ends the run cleanly — `Ok`, not `Err`** — and stops the sequence in
/// exactly the same place a refusal does: before preflight, before the
/// interview, before the writer. `FakeImageSelector::exited()` is
/// `ImageSelector::select`'s `Ok(None)`, the shape `write_image::Choice::Exit`
/// takes once it reaches this trait's boundary.
#[test]
fn an_exit_from_image_selection_is_ok_and_stops_before_anything_else_runs() {
    let preflight = FakePreflight::ok();
    let interview = FakeInterview::err("must not run -- the operator exited at image selection");
    let staging_check = FakeStagingCheck::ok();
    let fleet_recorder = FakeFleetRecorder::ok(None);
    let writer = FakeWriter::ok("must not run");
    let image_selector = FakeImageSelector::exited();

    let report = run_with(
        true,
        &preflight,
        &interview,
        &staging_check,
        &fleet_recorder,
        &writer,
        &image_selector,
        None,
        empty_args(),
        None,
        None,
        None,
        &FakeExistingDocument::never())
    .expect("an exit is Ok, not a refusal");

    assert!(report.contains("no write"), "{report}");
    assert!(!preflight.called.get(), "preflight must not run once the operator has exited");
    assert!(!interview.called.get());
    assert!(writer.seen.borrow().is_empty());
}

/// **Item 6.** A scratch working copy `image_selector` materialised is
/// removed once the run finishes successfully.
#[test]
fn a_scratch_working_copy_is_removed_after_a_successful_run() {
    let scratch = Scratch::new("image-cleanup-ok");
    let work_dir = crate::provision_image::scratch_dir(&scratch.0);
    std::fs::create_dir_all(&work_dir).expect("the fixture image-work directory");
    let working_copy = work_dir.join("materialised.img");
    std::fs::write(&working_copy, b"decompressed bytes").expect("write the fixture working copy");

    let fakes = Fakes {
        image_selector: FakeImageSelector::ok_with_scratch(&working_copy.display().to_string()),
        ..Fakes::happy()
    };
    fakes.run(true, empty_args()).expect("every fake answers Ok");

    assert!(!working_copy.exists(), "the scratch working copy must be removed after a successful run");
    assert!(!work_dir.exists(), "{} must be removed after a successful run", work_dir.display());
}

/// **Item 6, the other half.** The same cleanup happens when the run fails
/// AFTER image selection succeeded (here: the interview refuses) -- nothing
/// decompressed is ever left behind.
#[test]
fn a_scratch_working_copy_is_removed_after_a_failed_run() {
    let scratch = Scratch::new("image-cleanup-fail");
    let work_dir = crate::provision_image::scratch_dir(&scratch.0);
    std::fs::create_dir_all(&work_dir).expect("the fixture image-work directory");
    let working_copy = work_dir.join("materialised.img");
    std::fs::write(&working_copy, b"decompressed bytes").expect("write the fixture working copy");

    let fakes = Fakes {
        interview: FakeInterview::err("declined to write"),
        image_selector: FakeImageSelector::ok_with_scratch(&working_copy.display().to_string()),
        ..Fakes::happy()
    };
    let why = fakes.run(true, empty_args()).expect_err("the interview refused");

    assert_eq!(why, "declined to write");
    assert!(!working_copy.exists(), "the scratch working copy must be removed even when the run fails");
    assert!(!work_dir.exists(), "{} must be removed even when the run fails", work_dir.display());
}

// ---------------------------------------------------------------------------
// Decision 4: without a terminal, this refuses -- checked before anything
// else runs.
// ---------------------------------------------------------------------------

#[test]
fn no_terminal_and_no_document_refuses_before_the_interview_runs() {
    let fakes = Fakes::happy();
    let why = fakes.run(false, empty_args()).expect_err("must refuse");
    assert!(why.contains("no terminal"), "{why}");
    assert!(why.contains("--document"), "{why}");
    assert!(!fakes.interview.called.get(), "the interview must not have run");
    assert!(fakes.writer.seen.borrow().is_empty(), "the writer must not have run");
}

// ---------------------------------------------------------------------------
// Preflight runs before the interview, and its refusal stops the sequence.
// ---------------------------------------------------------------------------

#[test]
fn preflight_runs_before_the_interview_when_there_is_no_document_yet() {
    let fakes = Fakes::happy();
    fakes.run(true, empty_args()).expect("every fake answers Ok");
    assert!(fakes.preflight.called.get(), "preflight must have run");
    assert!(fakes.interview.called.get(), "the interview must have run after it");
}

#[test]
fn a_preflight_refusal_stops_before_the_interview_ever_runs() {
    let preflight = FakePreflight::err("no application signing seed");
    let interview = FakeInterview::ok("/should/not/be/read.toml");
    let staging_check = FakeStagingCheck::ok();
    let fleet_recorder = FakeFleetRecorder::ok(None);
    let writer = FakeWriter::ok("should not run");
    let why = run_with(
        true, &preflight, &interview, &staging_check, &fleet_recorder, &writer, &FakeImageSelector::ok(),
        None, empty_args(), None,
        None, None,
        &FakeExistingDocument::never())
        .expect_err("preflight refused");
    assert_eq!(why, "no application signing seed");
    assert!(!interview.called.get(), "the interview must not have run");
    assert!(staging_check.seen.borrow().is_empty(), "the staging check must not have run");
    assert!(writer.seen.borrow().is_empty(), "the writer must not have run");
}

/// A document already in hand needs no preflight and no interview — but the
/// staging check still runs on it.
#[test]
fn a_document_already_in_hand_skips_preflight_and_the_interview_but_not_the_staging_check() {
    let fakes = Fakes::happy();
    let mut args = empty_args();
    args.document_path = Some(PathBuf::from("/agent/composed.toml"));
    fakes.run(false, args).expect("no terminal needed once a document is in hand");
    assert!(!fakes.preflight.called.get(), "a document already in hand needs no preflight");
    assert!(!fakes.interview.called.get(), "a document already in hand needs no interview");
    assert_eq!(fakes.staging_check.seen.borrow().as_slice(), [PathBuf::from("/agent/composed.toml")]);
}

// ---------------------------------------------------------------------------
// The interview's result reaches the staging check and then the writer,
// unchanged.
// ---------------------------------------------------------------------------

#[test]
fn a_terminal_runs_the_interview_then_the_staging_check_then_the_writer() {
    let fakes = Fakes::happy();
    let report = fakes.run(true, empty_args()).expect("every fake answers Ok");
    assert!(report.contains("wrote it"), "{report}");
    assert!(fakes.interview.called.get(), "the interview must have run");
    assert_eq!(
        fakes.staging_check.seen.borrow().as_slice(),
        [PathBuf::from("/tmp/arbor.provisioning.toml")],
        "the interview's document must reach the staging check unchanged"
    );
    assert_eq!(
        fakes.writer.seen.borrow().as_slice(),
        [Some(PathBuf::from("/tmp/arbor.provisioning.toml"))],
        "the same document must reach the writer unchanged"
    );
}

/// A failed interview stops the sequence there — the staging check and the
/// writer are never reached, and the interview's own words are what comes
/// back.
#[test]
fn a_failed_interview_is_returned_and_nothing_after_it_runs() {
    let preflight = FakePreflight::ok();
    let interview = FakeInterview::err("declined to write");
    let staging_check = FakeStagingCheck::ok();
    let fleet_recorder = FakeFleetRecorder::ok(None);
    let writer = FakeWriter::ok("should not run");
    let why = run_with(
        true, &preflight, &interview, &staging_check, &fleet_recorder, &writer, &FakeImageSelector::ok(),
        None, empty_args(), None,
        None, None,
        &FakeExistingDocument::never())
        .expect_err("must fail");
    assert_eq!(why, "declined to write");
    assert!(staging_check.seen.borrow().is_empty(), "the staging check must not have run");
    assert!(writer.seen.borrow().is_empty(), "the writer must not have run");
}

/// A failed staging check (a server or member document with no bundle beside
/// it) stops the sequence there — the writer never sees a document promising
/// a card that would carry no credentials for its own application service.
#[test]
fn a_failed_staging_check_is_returned_and_the_writer_is_never_reached() {
    let preflight = FakePreflight::ok();
    let interview = FakeInterview::ok("/tmp/arbor.provisioning.toml");
    let staging_check = FakeStagingCheck::err("no staging bundle beside the document");
    let fleet_recorder = FakeFleetRecorder::ok(None);
    let writer = FakeWriter::ok("should not run");
    let why = run_with(
        true, &preflight, &interview, &staging_check, &fleet_recorder, &writer, &FakeImageSelector::ok(),
        None, empty_args(), None,
        None, None,
        &FakeExistingDocument::never())
        .expect_err("the staging check refused");
    assert_eq!(why, "no staging bundle beside the document");
    assert!(writer.seen.borrow().is_empty(), "the writer must not have run");
}

#[test]
fn verify_only_skips_preflight_the_interview_and_the_staging_check_even_without_a_terminal() {
    let fakes = Fakes::happy();
    let mut args = empty_args();
    args.verify_only = true;

    let report = fakes.run(false, args).expect("no terminal needed here");
    assert_eq!(report, "wrote it");
    assert!(!fakes.preflight.called.get());
    assert!(!fakes.interview.called.get());
    assert!(fakes.staging_check.seen.borrow().is_empty());
    assert_eq!(fakes.writer.seen.borrow().len(), 1);
}

/// The two internal `--elevated-*` re-invocations (`crate::elevate`
/// relaunching this binary, elevated) never see a terminal and must never be
/// made to wait on one.
#[test]
fn an_elevated_write_task_skips_preflight_the_interview_and_the_staging_check() {
    let fakes = Fakes::happy();
    let mut args = empty_args();
    args.elevated_write_task = Some(PathBuf::from("/tmp/task.json"));

    let report = fakes.run(false, args).expect("no terminal needed here");
    assert_eq!(report, "wrote it");
    assert!(!fakes.preflight.called.get());
    assert!(!fakes.interview.called.get());
    assert!(fakes.staging_check.seen.borrow().is_empty());
}

#[test]
fn an_elevated_verify_task_skips_preflight_the_interview_and_the_staging_check() {
    let fakes = Fakes::happy();
    let mut args = empty_args();
    args.elevated_verify_task = Some(PathBuf::from("/tmp/task.json"));

    let report = fakes.run(false, args).expect("no terminal needed here");
    assert_eq!(report, "wrote it");
    assert!(!fakes.preflight.called.get());
    assert!(!fakes.interview.called.get());
    assert!(fakes.staging_check.seen.borrow().is_empty());
}

// ---------------------------------------------------------------------------
// R-178 P4/Q7/Q10: the fleet recorder — a duplicate is refused BEFORE the
// write; the commit and any enrollment happen only AFTER a successful write,
// and only on a route that actually targeted a device.
// ---------------------------------------------------------------------------

/// A device stand-in — enough of a `Device` for these tests to tell one
/// canned answer from another; never saved anywhere real.
fn fixture_device(name: &str) -> rivetos_register::Device {
    rivetos_register::Device {
        id: "0011223344556677".to_owned(),
        name: name.to_owned(),
        fleet_role: Some(rivetos_register::FleetRole::Server),
        ..rivetos_register::Device::default()
    }
}

/// `--device PATH` — the shape a route that "targets a device" takes.
fn args_targeting_a_device() -> ProvisionArgs {
    let mut args = empty_args();
    args.document_path = Some(PathBuf::from("/agent/composed.toml"));
    args.device_path = Some(PathBuf::from("/dev/sd-fixture"));
    args
}

/// **The primary case.** A successful, device-targeting write is followed by
/// exactly one commit, carrying the plan `plan` produced.
#[test]
fn a_successful_device_targeting_write_is_followed_by_exactly_one_commit() {
    let preflight = FakePreflight::ok();
    let interview = FakeInterview::err("must not run — a document is already in hand");
    let staging_check = FakeStagingCheck::ok();
    let fleet_recorder = FakeFleetRecorder::ok(Some(fixture_device("arbor")));
    let writer = FakeWriter::ok("wrote and verified it");

    let report = run_with(
        false,
        &preflight,
        &interview,
        &staging_check,
        &fleet_recorder,
        &writer,
        &FakeImageSelector::ok(),
        None,
        args_targeting_a_device(),
        None,
        None,
        None,
        &FakeExistingDocument::never())
    .expect("every fake answers Ok");

    assert!(report.contains("wrote and verified it"), "{report}");
    assert!(report.contains("fleet repository: recorded"), "{report}");
    assert_eq!(fleet_recorder.plan_calls.borrow().len(), 1, "plan must run exactly once");
    assert_eq!(fleet_recorder.commit_calls.borrow().len(), 1, "commit must run exactly once");
    assert_eq!(
        fleet_recorder.commit_calls.borrow()[0].1,
        Some(fixture_device("arbor")),
        "the plan plan() produced must reach commit() unchanged"
    );
}

/// **Q10, restated as a test.** A declined or failed write commits nothing —
/// `plan` still ran (the duplicate check is unconditional on having a
/// document), but `commit` never does.
#[test]
fn a_failed_write_commits_and_enrolls_nothing() {
    let preflight = FakePreflight::ok();
    let interview = FakeInterview::err("must not run");
    let staging_check = FakeStagingCheck::ok();
    let fleet_recorder = FakeFleetRecorder::ok(Some(fixture_device("arbor")));
    let writer = FakeWriter::err("the card is NOT usable — digest mismatch");

    let why = run_with(
        false,
        &preflight,
        &interview,
        &staging_check,
        &fleet_recorder,
        &writer,
        &FakeImageSelector::ok(),
        None,
        args_targeting_a_device(),
        None,
        None,
        None,
        &FakeExistingDocument::never())
    .expect_err("the writer refused");

    assert_eq!(why, "the card is NOT usable — digest mismatch");
    assert_eq!(fleet_recorder.plan_calls.borrow().len(), 1, "the duplicate check still ran");
    assert!(fleet_recorder.commit_calls.borrow().is_empty(), "nothing must be committed");
}

/// **P4.** A route that writes no device at all — no `--device`, no
/// `confirm` — is `write_image::run`'s own "image ready, nothing written"
/// outcome, not a finished card (R-174). Nothing is committed for it.
#[test]
fn nothing_is_committed_when_no_device_was_targeted() {
    let preflight = FakePreflight::ok();
    let interview = FakeInterview::err("must not run");
    let staging_check = FakeStagingCheck::ok();
    let fleet_recorder = FakeFleetRecorder::ok(Some(fixture_device("arbor")));
    let writer = FakeWriter::nothing_written("the image is ready; nothing was written to a device");

    let mut args = empty_args();
    args.document_path = Some(PathBuf::from("/agent/composed.toml"));
    // Deliberately neither `--device` nor `confirm`.

    let report = run_with(
        false, &preflight, &interview, &staging_check, &fleet_recorder, &writer, &FakeImageSelector::ok(),
        None, args, None,
        None, None,
        &FakeExistingDocument::never())
    .expect("the writer still succeeds — it just wrote nothing to a device");

    assert!(report.contains("the image is ready"), "{report}");
    assert_eq!(fleet_recorder.plan_calls.borrow().len(), 1, "the duplicate check still ran");
    assert!(fleet_recorder.commit_calls.borrow().is_empty(), "nothing must be committed");
}

/// `confirm` alone (no `--device`) is also "targets a device" —
/// `write_image::run`'s own menu-driven route.
#[test]
fn confirm_with_no_device_path_still_commits() {
    let preflight = FakePreflight::ok();
    let interview = FakeInterview::err("must not run");
    let staging_check = FakeStagingCheck::ok();
    let fleet_recorder = FakeFleetRecorder::ok(None);
    let writer = FakeWriter::ok("wrote and verified it");

    let mut args = empty_args();
    args.document_path = Some(PathBuf::from("/agent/composed.toml"));
    args.confirm = true;

    run_with(
        false, &preflight, &interview, &staging_check, &fleet_recorder, &writer, &FakeImageSelector::ok(),
        None, args, None,
        None, None,
        &FakeExistingDocument::never())
    .expect("every fake answers Ok");
    assert_eq!(fleet_recorder.commit_calls.borrow().len(), 1);
}

// ---------------------------------------------------------------------------
// 2026-09-14, R-174: the bare interactive `cargo rivet provision` ends in a
// written card — the device menu and `Write it? [y/N]`, not "the image is
// ready at" a scratch copy about to be removed.
// ---------------------------------------------------------------------------

/// The interview as Joel's run had it: a document under `~/.rivet/provision`
/// and the identity directory it named.
fn interview_with_identity() -> FakeInterview {
    FakeInterview {
        called: Cell::new(false),
        answer: Ok(InterviewResult {
            document_path: PathBuf::from("/home/op/.rivet/provision/arbor.provisioning.toml"),
            pin: Some([0xabu8; 32]),
            identity_dir: Some(PathBuf::from("/home/op/.rivet/identity/arbor")),
        }),
    }
}

/// **The bare interactive run offers the device menu and does NOT confirm**
/// — the writer is told to offer removable devices and to ask `Write it?
/// [y/N]` itself.
#[test]
fn the_bare_interactive_run_offers_the_device_menu_and_asks_before_writing() {
    let fakes = Fakes { interview: interview_with_identity(), ..Fakes::happy() };
    fakes.run(true, empty_args()).expect("every fake answers Ok");
    assert_eq!(
        fakes.writer.device_flags_seen.borrow().as_slice(),
        [(false, true)],
        "(confirm, offer_device_menu): the menu is offered and the question is still asked"
    );
}

/// **`N` at `Write it? [y/N]` writes nothing and records nothing**, says so,
/// and names the command that writes this same card later without the
/// interview — the image by digest, since its scratch copy is removed.
#[test]
fn answering_no_records_nothing_and_names_the_command_that_writes_the_card_later() {
    let scratch = Scratch::new("declined-write");
    let working_copy = scratch.0.join("materialised.img");
    std::fs::write(&working_copy, b"decompressed bytes").expect("write the fixture working copy");
    let fakes = Fakes {
        interview: interview_with_identity(),
        fleet_recorder: FakeFleetRecorder::ok(Some(fixture_device("arbor"))),
        writer: FakeWriter::nothing_written(
            "nothing was written. Answer y at the prompt, or add `confirm` as the last word, to write \\\\.\\PHYSICALDRIVE2\n",
        ),
        image_selector: FakeImageSelector::ok_with_scratch(&working_copy.display().to_string()),
        ..Fakes::happy()
    };

    let report = fakes.run(true, empty_args()).expect("declining is an ordinary outcome, not an error");

    assert!(report.contains("nothing was written"), "{report}");
    assert!(report.contains("nothing was recorded"), "{report}");
    assert!(
        report.contains(
            "cargo rivet provision --document /home/op/.rivet/provision/arbor.provisioning.toml \
             --identity /home/op/.rivet/identity/arbor --image abc123abc123fixturedigest confirm"
        ),
        "{report}"
    );
    assert!(!report.contains("ready at"), "never point at the scratch copy: {report}");
    assert!(!report.contains("--- summary ---"), "{report}");
    assert!(fakes.fleet_recorder.commit_calls.borrow().is_empty(), "nothing must be recorded");
    assert!(!working_copy.exists(), "the scratch copy is still removed");
}

/// `confirm` still means "already confirmed": the writer is told so, and a
/// verified write is recorded.
#[test]
fn confirm_on_an_interactive_run_is_passed_as_already_confirmed() {
    let fakes = Fakes { interview: interview_with_identity(), ..Fakes::happy() };
    let mut args = empty_args();
    args.confirm = true;
    fakes.run(true, args).expect("every fake answers Ok");
    assert_eq!(fakes.writer.device_flags_seen.borrow().as_slice(), [(true, true)]);
    assert_eq!(fakes.fleet_recorder.commit_calls.borrow().len(), 1);
}

/// **A run with no terminal is unchanged**: no device menu offered, and the
/// report is the writer's own, with no rerun command appended.
#[test]
fn a_run_with_no_terminal_offers_no_device_menu() {
    let fakes = Fakes {
        interview: FakeInterview::err("must not run"),
        writer: FakeWriter::nothing_written("the image is ready at /x.img. Nothing was written to a device\n"),
        ..Fakes::happy()
    };
    let mut args = empty_args();
    args.document_path = Some(PathBuf::from("/agent/composed.toml"));
    let report = fakes.run(false, args).expect("every fake answers Ok");
    assert_eq!(fakes.writer.device_flags_seen.borrow().as_slice(), [(false, false)]);
    assert!(!report.contains("nothing was recorded"), "{report}");
    assert!(fakes.fleet_recorder.commit_calls.borrow().is_empty());
}

/// **Q10: refused BEFORE the write.** A duplicate enrollment stops the
/// sequence before the writer is ever reached — nothing is written.
#[test]
fn a_duplicate_enrollment_is_refused_before_the_write() {
    let preflight = FakePreflight::ok();
    let interview = FakeInterview::err("must not run");
    let staging_check = FakeStagingCheck::ok();
    let fleet_recorder = FakeFleetRecorder::plan_err(
        "NOTHING WAS WRITTEN. arbor's own identifier is already enrolled",
    );
    let writer = FakeWriter::ok("must not run");

    let why = run_with(
        false,
        &preflight,
        &interview,
        &staging_check,
        &fleet_recorder,
        &writer,
        &FakeImageSelector::ok(),
        None,
        args_targeting_a_device(),
        None,
        None,
        None,
        &FakeExistingDocument::never())
    .expect_err("a duplicate must refuse");

    assert!(why.starts_with("NOTHING WAS WRITTEN"), "{why}");
    assert!(why.contains("already enrolled"), "{why}");
    assert!(writer.seen.borrow().is_empty(), "the writer must never have run");
}

/// **The pin reaches `plan` unchanged**, whichever source it came from. Here:
/// an explicit one, given to `run_with` the way `--identity`/`--pin` supply
/// it on the `--document PATH` route.
#[test]
fn an_explicit_pin_reaches_the_fleet_recorders_plan_call() {
    let preflight = FakePreflight::ok();
    let interview = FakeInterview::err("must not run — a document is already in hand");
    let staging_check = FakeStagingCheck::ok();
    let fleet_recorder = FakeFleetRecorder::ok(None);
    let writer = FakeWriter::ok("wrote and verified it");
    let pin = [0x11u8; 32];

    run_with(
        false,
        &preflight,
        &interview,
        &staging_check,
        &fleet_recorder,
        &writer,
        &FakeImageSelector::ok(),
        None,
        args_targeting_a_device(),
        Some(pin),
        None,
        None,
        &FakeExistingDocument::never())
    .expect("every fake answers Ok");

    assert_eq!(fleet_recorder.plan_pins.borrow().as_slice(), [Some(pin)]);
}

/// **The interview's own breadcrumb pin reaches `plan` too**, when no
/// explicit `--identity`/`--pin` was given — the interactive route's whole
/// answer to "where does the pin come from" (see the module documentation).
#[test]
fn the_interviews_own_pin_reaches_the_fleet_recorders_plan_call_when_nothing_explicit_was_given() {
    let preflight = FakePreflight::ok();
    let pin = [0x22u8; 32];
    let interview = FakeInterview::ok_with_pin("/tmp/arbor.provisioning.toml", pin);
    let staging_check = FakeStagingCheck::ok();
    let fleet_recorder = FakeFleetRecorder::ok(None);
    let writer = FakeWriter::ok("wrote and verified it");

    let mut args = empty_args();
    args.confirm = true;

    run_with(
        true, &preflight, &interview, &staging_check, &fleet_recorder, &writer, &FakeImageSelector::ok(),
        None, args, None,
        None, None,
        &FakeExistingDocument::never())
    .expect("every fake answers Ok");

    assert_eq!(fleet_recorder.plan_pins.borrow().as_slice(), [Some(pin)]);
}

/// An explicit `--identity`/`--pin` wins over the interview's own breadcrumb
/// pin — the flag is what the operator or an agent stated on purpose.
#[test]
fn an_explicit_pin_wins_over_the_interviews_own() {
    let preflight = FakePreflight::ok();
    let breadcrumb_pin = [0x22u8; 32];
    let explicit_pin = [0x33u8; 32];
    let interview = FakeInterview::ok_with_pin("/tmp/arbor.provisioning.toml", breadcrumb_pin);
    let staging_check = FakeStagingCheck::ok();
    let fleet_recorder = FakeFleetRecorder::ok(None);
    let writer = FakeWriter::ok("wrote and verified it");

    let mut args = empty_args();
    args.confirm = true;

    run_with(
        true,
        &preflight,
        &interview,
        &staging_check,
        &fleet_recorder,
        &writer,
        &FakeImageSelector::ok(),
        None,
        args,
        Some(explicit_pin),
        None,
        None,
        &FakeExistingDocument::never())
    .expect("every fake answers Ok");

    assert_eq!(fleet_recorder.plan_pins.borrow().as_slice(), [Some(explicit_pin)]);
}

// ---------------------------------------------------------------------------
// Wave C3 (R-178 item 1): the writer receives what it needs to inject —
// the staging bundle's resolved path, the identity directory, and the fleet
// repository root — rather than re-deriving any of them itself.
// ---------------------------------------------------------------------------

/// **The staging check's own resolved path reaches the writer**, so
/// `write_image::run`'s injection (item 1) carries the SAME bundle
/// `require_staging` already validated rather than re-deriving it.
#[test]
fn the_staging_checks_resolved_path_reaches_the_writer() {
    let preflight = FakePreflight::ok();
    let interview = FakeInterview::ok("/tmp/arbor.provisioning.toml");
    let staging_check = FakeStagingCheck::ok_with_path("/tmp/arbor.staged.txt");
    let fleet_recorder = FakeFleetRecorder::ok(None);
    let writer = FakeWriter::ok("wrote and verified it");

    let mut args = empty_args();
    args.confirm = true;

    run_with(
        true, &preflight, &interview, &staging_check, &fleet_recorder, &writer, &FakeImageSelector::ok(),
        None, args, None, None, None,
        &FakeExistingDocument::never())
    .expect("every fake answers Ok");

    assert_eq!(
        writer.injection_args_seen.borrow().as_slice(),
        [(Some(PathBuf::from("/tmp/arbor.staged.txt")), None, None)],
    );
}

/// **The interview's own "Identity" answer, off the breadcrumb's third line,
/// reaches the writer too** — the same footing the pin already stands on
/// (`an_explicit_pin_wins_over_the_interviews_own`, one line up).
#[test]
fn the_interviews_identity_directory_reaches_the_writer() {
    let preflight = FakePreflight::ok();
    let mut interview_answer = FakeInterview::ok("/tmp/arbor.provisioning.toml");
    interview_answer.answer =
        Ok(InterviewResult {
            document_path: PathBuf::from("/tmp/arbor.provisioning.toml"),
            pin: None,
            identity_dir: Some(PathBuf::from("/tmp/arbor-identity")),
        });
    let staging_check = FakeStagingCheck::ok();
    let fleet_recorder = FakeFleetRecorder::ok(None);
    let writer = FakeWriter::ok("wrote and verified it");

    let mut args = empty_args();
    args.confirm = true;

    run_with(
        true, &preflight, &interview_answer, &staging_check, &fleet_recorder, &writer, &FakeImageSelector::ok(),
        None, args, None, None, None,
        &FakeExistingDocument::never())
    .expect("every fake answers Ok");

    assert_eq!(
        writer.injection_args_seen.borrow()[0].1,
        Some(PathBuf::from("/tmp/arbor-identity")),
    );
}

/// An explicit `--identity PATH` wins over the interview's own breadcrumb —
/// `extract_identity_and_pin`'s own precedence, restated at the sequencing
/// level `an_explicit_pin_wins_over_the_interviews_own` already covers for
/// the pin.
#[test]
fn an_explicit_identity_directory_wins_over_the_interviews_own() {
    let preflight = FakePreflight::ok();
    let mut interview_answer = FakeInterview::ok("/tmp/arbor.provisioning.toml");
    interview_answer.answer = Ok(InterviewResult {
        document_path: PathBuf::from("/tmp/arbor.provisioning.toml"),
        pin: None,
        identity_dir: Some(PathBuf::from("/tmp/from-the-interview")),
    });
    let staging_check = FakeStagingCheck::ok();
    let fleet_recorder = FakeFleetRecorder::ok(None);
    let writer = FakeWriter::ok("wrote and verified it");

    let mut args = empty_args();
    args.confirm = true;

    run_with(
        true, &preflight, &interview_answer, &staging_check, &fleet_recorder, &writer, &FakeImageSelector::ok(),
        None, args, None, Some(PathBuf::from("/tmp/explicit-identity")), None,
        &FakeExistingDocument::never())
    .expect("every fake answers Ok");

    assert_eq!(
        writer.injection_args_seen.borrow()[0].1,
        Some(PathBuf::from("/tmp/explicit-identity")),
        "the explicit flag must win over the interview's own breadcrumb",
    );
}

/// **The fleet repository root `run` already resolved reaches the writer
/// unchanged** — set directly on `ProvisionArgs` before `run_with` is ever
/// called (`run`'s own `write_args.fleet_repo_root = fleet_repo_root.clone()`),
/// so this proves `run_with`/`run_with_image_selected` never clear or
/// overwrite it on the way to the writer.
#[test]
fn the_fleet_repository_root_reaches_the_writer_unchanged() {
    let preflight = FakePreflight::ok();
    let interview = FakeInterview::ok("/tmp/arbor.provisioning.toml");
    let staging_check = FakeStagingCheck::ok();
    let fleet_recorder = FakeFleetRecorder::ok(None);
    let writer = FakeWriter::ok("wrote and verified it");

    let mut args = empty_args();
    args.confirm = true;
    args.fleet_repo_root = Some(PathBuf::from("/tmp/fleet-repo"));

    run_with(
        true, &preflight, &interview, &staging_check, &fleet_recorder, &writer, &FakeImageSelector::ok(),
        None, args, None, None, None,
        &FakeExistingDocument::never())
    .expect("every fake answers Ok");

    assert_eq!(writer.injection_args_seen.borrow()[0].2, Some(PathBuf::from("/tmp/fleet-repo")));
}

// ---------------------------------------------------------------------------
// fleet_repository_fallback / register_root_for — 2026-09-14: `--at DIR`
// governing the fleet repository's own default and the register, pure. See
// `provision.rs`'s own "2026-09-14" module-doc section for the defect these
// close and why the register (unlike the fleet repository) branches on the
// LITERAL `--at` flag rather than the resolved credential root.
// ---------------------------------------------------------------------------

/// **Under `--at`, the fleet repository's own fallback is `<at>/fleet`** —
/// never a fresh `rivetos_fleet_repo::default_root()` read of this host's
/// real `HOME`/`USERPROFILE`.
#[test]
fn the_fleet_repository_fallback_is_under_at() {
    let at = Path::new("/scratch/some-at-directory");
    assert_eq!(fleet_repository_fallback(at), Path::new("/scratch/some-at-directory/fleet"));
}

/// **Given `--at DIR`, the register is `DIR/register`** — never
/// `crate::device::default_register_dir`'s ignition/`%APPDATA%`-resolved
/// answer, which is exactly the operator's real register.
#[test]
fn given_at_the_register_root_is_under_it() {
    let at = Path::new("/scratch/some-at-directory");
    assert_eq!(
        register_root_for(Some(at)).expect("an explicit --at always resolves"),
        Path::new("/scratch/some-at-directory/register")
    );
}

/// **Without `--at`, `register_root_for` still delegates to
/// `crate::device::default_register_dir`** — real, unscratched provisioning
/// must keep enrolling into the real, already-populated register exactly as
/// it always has, `RIVET_FLEET_REGISTER`'s own override included. This does
/// not assert what that function returns (this process's real `HOME`/
/// `USERPROFILE`/`RIVET_FLEET_REGISTER` are not this test's business) — only
/// that the two answers agree, i.e. that `register_root_for(None)` is
/// nothing more than a passthrough.
#[test]
fn without_at_the_register_root_is_the_ordinary_default_unchanged() {
    assert_eq!(register_root_for(None), crate::device::default_register_dir());
}

// ---------------------------------------------------------------------------
// --staging extraction, pure.
// ---------------------------------------------------------------------------

#[test]
fn extract_staging_pulls_the_flag_and_its_value_out_of_argv() {
    let args = vec![
        "--document".to_owned(),
        "doc.toml".to_owned(),
        "--staging".to_owned(),
        "bundle.staged.txt".to_owned(),
        "confirm".to_owned(),
    ];
    let (rest, staging) = extract_staging(&args);
    assert_eq!(rest, ["--document", "doc.toml", "confirm"]);
    assert_eq!(staging, Some(PathBuf::from("bundle.staged.txt")));
}

#[test]
fn extract_staging_with_no_flag_returns_argv_unchanged() {
    let args = vec!["--document".to_owned(), "doc.toml".to_owned()];
    let (rest, staging) = extract_staging(&args);
    assert_eq!(rest, args);
    assert_eq!(staging, None);
}

// ---------------------------------------------------------------------------
// --identity / --pin extraction, and hex decoding — pure.
// ---------------------------------------------------------------------------

#[test]
fn decode_pin_hex_round_trips_a_pin() {
    let pin = [0xabu8; 32];
    let hex = rivetos_tls::encode_hex(&pin);
    assert_eq!(decode_pin_hex(&hex), Ok(pin));
}

#[test]
fn decode_pin_hex_refuses_anything_that_is_not_64_hex_characters() {
    assert!(decode_pin_hex("ab").is_err(), "too short");
    assert!(decode_pin_hex(&"1".repeat(64)).is_ok(), "exactly 64 is fine");
    assert!(decode_pin_hex(&"1".repeat(68)).is_err(), "too long");
    let not_hex = "z".repeat(64);
    assert!(decode_pin_hex(&not_hex).is_err(), "not hex at all");
}

#[test]
fn extract_identity_and_pin_takes_an_explicit_pin_directly() {
    let pin = [0x42u8; 32];
    let args = vec![
        "--document".to_owned(),
        "doc.toml".to_owned(),
        "--pin".to_owned(),
        rivetos_tls::encode_hex(&pin),
        "confirm".to_owned(),
    ];
    let (rest, identity_dir, resolved) = extract_identity_and_pin(&args).expect("a valid pin");
    assert_eq!(rest, ["--document", "doc.toml", "confirm"]);
    assert_eq!(resolved, Some(pin));
    assert_eq!(identity_dir, None, "--pin names no directory");
}

#[test]
fn extract_identity_and_pin_with_neither_flag_returns_argv_unchanged() {
    let args = vec!["--document".to_owned(), "doc.toml".to_owned()];
    let (rest, identity_dir, resolved) = extract_identity_and_pin(&args).expect("no flag is not an error");
    assert_eq!(rest, args);
    assert_eq!(resolved, None);
    assert_eq!(identity_dir, None);
}

#[test]
fn extract_identity_and_pin_refuses_a_pin_that_is_not_64_hex_characters() {
    let args = vec!["--pin".to_owned(), "not-hex".to_owned()];
    let err = extract_identity_and_pin(&args).expect_err("not a real pin");
    assert!(err.contains("64 hex characters"), "{err}");
}

#[test]
fn extract_identity_and_pin_reads_the_certificate_at_the_named_directory() {
    let dir = crate::scratch::dir(&format!("provision-identity-{}", line!()));
    let cert = dir.join(rivetos_tls::CERTIFICATE_FILE);
    // A real Ed25519 self-signed certificate, minted the same way
    // `crate::identity::run` does — this crate already depends on `rcgen`.
    let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("a keypair");
    let params = rcgen::CertificateParams::new(vec!["fixture".to_owned()]).expect("params");
    let certificate = params.self_signed(&pair).expect("self-signed");
    std::fs::write(&cert, certificate.der()).expect("write the fixture certificate");
    let expected_pin =
        rivetos_tls::public_key_in_certificate(certificate.der()).expect("an Ed25519 key");

    let args = vec!["--identity".to_owned(), dir.display().to_string()];
    let (rest, identity_dir, resolved) = extract_identity_and_pin(&args).expect("a readable certificate");
    assert!(rest.is_empty());
    assert_eq!(resolved, Some(expected_pin));
    assert_eq!(identity_dir, Some(dir.clone()), "the directory itself must be returned too");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_explicit_pin_wins_over_identity_when_both_are_given() {
    let pin = [0x77u8; 32];
    let args = vec![
        "--identity".to_owned(),
        "/does/not/exist".to_owned(),
        "--pin".to_owned(),
        rivetos_tls::encode_hex(&pin),
    ];
    let (_, identity_dir, resolved) =
        extract_identity_and_pin(&args).expect("--pin wins, so --identity is never opened");
    assert_eq!(resolved, Some(pin));
    // **The directory is still returned even though --pin decided the pin** —
    // `run_with_image_selected` threads it into `args.identity_path`
    // independently of the pin, for injection (item 1) rather than enrollment.
    assert_eq!(identity_dir, Some(PathBuf::from("/does/not/exist")));
}

// ---------------------------------------------------------------------------
// The breadcrumb hand-off, pure.
// ---------------------------------------------------------------------------

/// **2026-09-14.** `breadcrumb_path` used to re-resolve
/// `crate::credentials::root()` on its own, ignoring whatever `--at DIR` a
/// run had actually been given — a second, independent divergence from
/// `--at` sitting right beside the one inside `cmd/rivet-provision` itself.
/// It now takes `at` directly, so this is provable with a path and nothing
/// else: no `HOME`, no `USERPROFILE`, no real credential root in the loop.
#[test]
fn the_breadcrumb_is_read_from_under_at_not_from_credentials_root() {
    let at = Path::new("/scratch/some-at-directory");
    assert_eq!(
        breadcrumb_path(at),
        Path::new("/scratch/some-at-directory/provision/.latest-run.txt")
    );
}

#[test]
fn the_breadcrumb_names_the_document_on_its_first_line() {
    assert_eq!(
        parse_breadcrumb("/home/j/.rivet/provision/arbor.provisioning.toml\n"),
        Some(PathBuf::from("/home/j/.rivet/provision/arbor.provisioning.toml"))
    );
}

#[test]
fn a_breadcrumb_with_more_than_one_line_reads_only_the_first() {
    assert_eq!(parse_breadcrumb("/a/b.toml\narbor\narbor.local\n"), Some(PathBuf::from("/a/b.toml")));
}

#[test]
fn an_empty_or_blank_breadcrumb_names_nothing() {
    assert_eq!(parse_breadcrumb(""), None);
    assert_eq!(parse_breadcrumb("\n\n"), None);
    assert_eq!(parse_breadcrumb("   \n"), None);
}

/// **R-178 Q7.** The breadcrumb's second line, `pin=<hex>`, is the pin.
#[test]
fn the_breadcrumbs_second_line_names_the_pin() {
    let pin = [0x9au8; 32];
    let text = format!("/a/b.provisioning.toml\npin={}\n", rivetos_tls::encode_hex(&pin));
    assert_eq!(parse_breadcrumb_pin(&text), Some(pin));
}

/// A breadcrumb with only one line — every build before this one, and every
/// stand-alone machine's — carries no pin, and that is not an error.
#[test]
fn a_one_line_breadcrumb_carries_no_pin() {
    assert_eq!(parse_breadcrumb_pin("/a/b.provisioning.toml\n"), None);
    assert_eq!(parse_breadcrumb_pin(""), None);
}

/// A second line that is not `pin=...`, or hex that does not decode, is also
/// simply no pin — never a reason the interview's own document path (line
/// one) is doubted.
#[test]
fn a_malformed_second_line_is_no_pin_rather_than_an_error() {
    assert_eq!(parse_breadcrumb_pin("/a/b.toml\nsomething else\n"), None);
    assert_eq!(parse_breadcrumb_pin("/a/b.toml\npin=not-hex\n"), None);
}

// ---------------------------------------------------------------------------
// The sibling-binary path.
// ---------------------------------------------------------------------------

#[test]
fn the_sibling_binary_is_named_in_the_same_directory() {
    let found = sibling_binary_in(Path::new("/opt/rivet/bin"), "rivet-provision");
    let expected =
        Path::new("/opt/rivet/bin").join(format!("rivet-provision{}", std::env::consts::EXE_SUFFIX));
    assert_eq!(found, expected);
}

// ---------------------------------------------------------------------------
// resolve_interview_source — three outcomes, and the working directory is a
// condition on NONE of them (`.rulings/` R-192 D3). `cargo install --path
// cmd/cargo-rivet --force` copies only that crate's own binary, which is why
// the install is two lines and why the third outcome names both.
// ---------------------------------------------------------------------------

/// **Outcome 1 — a sibling is there.** A `target/<profile>`-shaped directory,
/// or `~/.cargo/bin` after README §2's two install lines. Proven with a real
/// temporary file standing in for the binary, so `.is_file()` has something
/// true to answer without needing an actual executable.
///
/// **The working directory is deliberately NOT a checkout here** — a
/// directory that does not exist at all — because that is the whole of
/// `.rulings/` R-192 D3: with the interview installed, the verb works from
/// anywhere.
#[test]
fn a_sibling_that_exists_is_chosen_from_any_directory() {
    let dir = crate::scratch::dir("provision-sibling-present");
    let sibling = dir.join(format!("rivet-provision{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&sibling, b"not a real binary, just a file").expect("write the fixture");
    let nowhere = dir.join("not-a-checkout");

    let resolved = resolve_interview_source(&sibling, Some(&nowhere));
    assert_eq!(resolved, Ok(InterviewSource::Sibling(sibling)));

    let _ = std::fs::remove_dir_all(&dir);
}

/// **Outcome 2 — no sibling, but the working directory is a checkout.** The
/// developer's route: `cargo run` from that checkout is what makes an edit to
/// `cmd/rivet-provision` take effect on the very next run. The fixture is a
/// real `cmd/rivet-provision/Cargo.toml` under a scratch directory, which is
/// exactly what [`is_rivet_checkout`] looks for.
#[test]
fn no_sibling_in_a_checkout_falls_back_to_cargo_run_from_it() {
    let dir = crate::scratch::dir("provision-checkout");
    let crate_dir = dir.join("cmd").join("rivet-provision");
    std::fs::create_dir_all(&crate_dir).expect("a scratch checkout");
    std::fs::write(crate_dir.join("Cargo.toml"), b"[package]\nname = \"rivetos-provision\"\n")
        .expect("write the fixture manifest");
    let sibling = dir.join("bin").join(format!("rivet-provision{}", std::env::consts::EXE_SUFFIX));
    assert!(!sibling.exists(), "the sibling must genuinely be absent");

    assert_eq!(
        resolve_interview_source(&sibling, Some(&dir)),
        Ok(InterviewSource::CargoRun(dir.clone()))
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// **Outcome 3 — neither, and the refusal names the INSTALL, not the
/// directory.** The old gate said *"has to run from the repository root"*,
/// which sent the maintainer looking for a checkout he did not need
/// (`.rulings/` R-191). What he needs instead is the second install line, so
/// the refusal carries both of them verbatim.
#[test]
fn neither_a_sibling_nor_a_checkout_refuses_by_naming_the_install() {
    let dir = crate::scratch::dir("provision-neither");
    let sibling = dir.join(format!("rivet-provision{}", std::env::consts::EXE_SUFFIX));
    assert!(!sibling.exists(), "the sibling must genuinely be absent");

    let why = resolve_interview_source(&sibling, Some(&dir)).expect_err("neither is a refusal");
    assert!(why.contains("cargo install --path cmd/cargo-rivet --force --locked"), "{why}");
    assert!(why.contains("cargo install --path cmd/rivet-provision --force --locked"), "{why}");
    // The old gate's own words, which must not come back: the directory is
    // reported as a fact about where a checkout was looked for, never as the
    // thing to fix.
    assert!(!why.contains("has to run from the repository root"), "{why}");

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// The version check on an installed pair — `.rulings/` R-191 D3.
// ---------------------------------------------------------------------------

/// **A matching pair proceeds**, and that is the ordinary answer: no refusal
/// at all.
#[test]
fn a_matching_pair_is_not_refused() {
    let exe = Path::new("/opt/rivet/bin/rivet-provision");
    let build = "0.1.87 (v0.1.77-459-gabcdef1)";
    assert_eq!(version_refusal(build, Ok(build), exe), None);
}

/// **`-dirty` alone is not a mismatch.** Two binaries built minutes apart in
/// one working tree disagree about it routinely — `comparable`'s own doc has
/// the reason — while agreeing about the commit, which is the half that
/// identifies a build.
#[test]
fn a_dirty_stamp_on_one_side_is_not_a_mismatch() {
    let exe = Path::new("/opt/rivet/bin/rivet-provision");
    assert_eq!(
        version_refusal(
            "0.1.87 (v0.1.77-459-gabcdef1)",
            Ok("0.1.87 (v0.1.77-459-gabcdef1-dirty)"),
            exe
        ),
        None
    );
}

/// **A mismatch names BOTH versions and the command that reinstalls** —
/// R-191 D3 in as many words.
#[test]
fn a_mismatched_pair_names_both_versions_and_the_reinstall() {
    let exe = Path::new("/opt/rivet/bin/rivet-provision");
    let why = version_refusal(
        "0.1.87 (v0.1.77-459-gabcdef1)",
        Ok("0.1.85 (v0.1.77-12-g9999999)"),
        exe,
    )
    .expect("a mismatched pair must refuse");
    assert!(why.contains("0.1.87 (v0.1.77-459-gabcdef1)"), "{why}");
    assert!(why.contains("0.1.85 (v0.1.77-12-g9999999)"), "{why}");
    assert!(why.contains("cargo install --path cmd/cargo-rivet --force"), "{why}");
    assert!(why.contains("cargo install --path cmd/rivet-provision --force"), "{why}");
    assert!(why.contains("rivet-provision"), "{why}");
}

/// **A sibling that will not say what it is refuses too**, naming the same
/// reinstall — an old build with no `--version` flag at all is exactly the
/// stale pair this check exists to catch, and it cannot be caught by
/// comparing two strings when one of them is missing.
#[test]
fn a_sibling_that_will_not_say_its_version_is_refused() {
    let exe = Path::new("/opt/rivet/bin/rivet-provision");
    let why = version_refusal("0.1.87 (v0.1.77-459-gabcdef1)", Err("--version exited 2"), exe)
        .expect("an unreadable version must refuse");
    assert!(why.contains("--version exited 2"), "{why}");
    assert!(why.contains("0.1.87 (v0.1.77-459-gabcdef1)"), "{why}");
    assert!(why.contains("cargo install --path cmd/rivet-provision --force"), "{why}");
}

/// The parsing itself: both tools' own `--version` first line, and the shapes
/// that are not one.
#[test]
fn a_version_line_is_the_text_after_the_programs_name() {
    assert_eq!(
        parse_version_line("cargo-rivet 0.1.87 (v1-g1)\n  running  ...\n").as_deref(),
        Some("0.1.87 (v1-g1)")
    );
    assert_eq!(
        parse_version_line("rivet-provision 0.1.87 (v1-g1)\n").as_deref(),
        Some("0.1.87 (v1-g1)")
    );
    // A leading blank line is skipped rather than read as the answer.
    assert_eq!(parse_version_line("\n\nrivet-provision 0.1.87\n").as_deref(), Some("0.1.87"));
    // Nothing usable: no second word, or no output at all.
    assert_eq!(parse_version_line("rivet-provision\n"), None);
    assert_eq!(parse_version_line(""), None);
}

// ---------------------------------------------------------------------------
// requires_staging / staging_path_beside / require_staging — pure or
// against real scratch files, never through run_with's own fakes.
// ---------------------------------------------------------------------------

/// A scratch directory this test owns outright, removed when it drops.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        Self(crate::scratch::dir(&format!(
            "provision-staging-{label}-{n}"
        )))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_servers_document_requires_staging_and_a_stand_alone_document_does_not() {
    let server = "version = 1\n[services.rivet-fleetd]\nenabled = true\n";
    let doc = rivet_config::Document::parse(server).expect("a server document");
    assert!(requires_staging(&doc));

    let stand_alone = rivet_config::Document::parse("version = 1\n").expect("the minimal document");
    assert!(!requires_staging(&stand_alone));
}

/// **A MEMBER requires staging by its role, though its card carries nothing**
/// — `ADR-060` §5. Its proxy is in the image now, so `carried_services` is
/// empty for it; the proxy still needs the certificate the bundle brings.
#[test]
fn a_members_document_requires_staging_though_it_carries_nothing() {
    let member = "version = 1\n[services.rivet-proxyd]\nenabled = true\n";
    let doc = rivet_config::Document::parse(member).expect("a member document");
    assert!(doc.carried_services().is_empty(), "the proxy is not carried any more");
    assert!(requires_staging(&doc), "the member's proxy needs its certificate");
}

/// **`ADR-060` §5: a server or member card is refused an image older than the
/// first that carries the proxy**, and a stand-alone card is not asked.
#[test]
fn a_server_or_member_card_is_refused_an_image_without_the_proxy() {
    let floor = rivet_core::service::PROXY_IN_IMAGE_SINCE;
    for (role, src) in [
        ("server", "version = 1\n[services.rivet-fleetd]\nenabled = true\n"),
        ("member", "version = 1\n[services.rivet-proxyd]\nenabled = true\n"),
    ] {
        let doc = rivet_config::Document::parse(src).expect("a document");
        let why = refuse_an_image_without_the_proxy(&doc, Some("0.1.113"))
            .expect_err("an image older than the proxy's");
        assert!(why.contains("NOTHING WAS WRITTEN"), "{role}: {why}");
        assert!(why.contains(floor) && why.contains("0.1.113"), "{role}: {why}");
        assert_eq!(refuse_an_image_without_the_proxy(&doc, Some(floor)), Ok(None), "{role}");
        assert!(
            refuse_an_image_without_the_proxy(&doc, Some("not-a-release")).is_err(),
            "{role}: a version that cannot be read is not guessed at"
        );
        let note = refuse_an_image_without_the_proxy(&doc, None)
            .expect("an image named by path is written")
            .expect("and what was not checked is said");
        assert!(note.contains("NOT CHECKED"), "{role}: {note}");
    }
    let stand_alone = rivet_config::Document::parse("version = 1\n").expect("the minimal document");
    assert_eq!(refuse_an_image_without_the_proxy(&stand_alone, Some("0.1.1")), Ok(None));
}

#[test]
fn a_document_with_no_carried_service_needs_no_staging_bundle() {
    let doc = rivet_config::Document::parse("version = 1\n").expect("the minimal document");
    let result = require_staging(Path::new("/nowhere/rivet-1.provisioning.toml"), &doc, None)
        .expect("stand-alone needs nothing");
    assert_eq!(result, None);
}

#[test]
fn a_server_document_with_no_staging_bundle_is_refused_by_name() {
    let scratch = Scratch::new("no-staging");
    let doc_path = scratch.0.join("rivet-1.provisioning.toml");
    let src = "version = 1\n[services.rivet-fleetd]\nenabled = true\n";
    std::fs::write(&doc_path, src).expect("write the document");
    let doc = rivet_config::Document::parse(src).expect("a server document");
    let err = require_staging(&doc_path, &doc, None).expect_err("no staging bundle exists");
    assert!(err.contains("staging bundle"), "{err}");
    assert!(err.contains("rivet-1.staged.txt"), "{err}");
}

#[test]
fn a_server_document_with_a_staging_bundle_beside_it_is_found() {
    let scratch = Scratch::new("with-staging");
    let doc_path = scratch.0.join("rivet-1.provisioning.toml");
    let staging_path = scratch.0.join("rivet-1.staged.txt");
    let src = "version = 1\n[services.rivet-fleetd]\nenabled = true\n";
    std::fs::write(&doc_path, src).expect("write the document");
    let bundle = rivet_core::staging::render(&[
        rivet_core::staging::Staged::new("rivet-fleetd", "server.crt", b"not a real cert".to_vec())
            .expect("a staged file"),
    ]);
    std::fs::write(&staging_path, bundle).expect("write the staging bundle");
    let doc = rivet_config::Document::parse(src).expect("a server document");
    let found = require_staging(&doc_path, &doc, None).expect("a staging bundle is there");
    assert_eq!(found, Some(staging_path));
}

#[test]
fn an_explicit_staging_override_is_used_before_the_convention() {
    let scratch = Scratch::new("override");
    let doc_path = scratch.0.join("weird-name.toml");
    let staging_path = scratch.0.join("elsewhere.staged.txt");
    let src = "version = 1\n[services.rivet-fleetd]\nenabled = true\n";
    std::fs::write(&doc_path, src).expect("write the document");
    let bundle = rivet_core::staging::render(&[
        rivet_core::staging::Staged::new("rivet-fleetd", "server.crt", b"not a real cert".to_vec())
            .expect("a staged file"),
    ]);
    std::fs::write(&staging_path, bundle).expect("write the staging bundle");
    let doc = rivet_config::Document::parse(src).expect("a server document");
    let found =
        require_staging(&doc_path, &doc, Some(&staging_path)).expect("the override is used");
    assert_eq!(found, Some(staging_path));
}

// ---------------------------------------------------------------------------
// RealStagingCheck — the seam between run_with and require_staging, against
// a real scratch file.
// ---------------------------------------------------------------------------

#[test]
fn real_staging_check_reads_and_parses_the_document_it_is_given() {
    let scratch = Scratch::new("real-check");
    let doc_path = scratch.0.join("rivet-1.provisioning.toml");
    std::fs::write(&doc_path, "version = 1\n").expect("write the stand-alone document");
    let builder = FakeStagingBuilder::writing();
    let check = RealStagingCheck { staging_override: None, at: &scratch.0, builder: Some(&builder) };
    check.check(&doc_path, None).expect("a stand-alone document needs no staging bundle");
    assert!(builder.calls.borrow().is_empty(), "nothing is built for a stand-alone document");
}

#[test]
fn real_staging_check_refuses_a_document_that_does_not_parse() {
    let scratch = Scratch::new("bad-document");
    let doc_path = scratch.0.join("not-toml-at-all.toml");
    std::fs::write(&doc_path, "this is not TOML {{{").expect("write the fixture");
    let check = RealStagingCheck { staging_override: None, at: &scratch.0, builder: None };
    let err = check.check(&doc_path, None).expect_err("must refuse to parse");
    assert!(err.contains("does not parse"), "{err}");
}

// ---------------------------------------------------------------------------
// D8 — a server or member document with no staging bundle has one BUILT
// (`ensure_staging`), with a fake builder standing in for `rivet-provision
// stage`, whose own tests prove what it composes.
// ---------------------------------------------------------------------------

/// Records every build it was asked for and writes a one-file bundle where
/// `rivet-provision stage` would — or refuses, when told to.
struct FakeStagingBuilder {
    calls: RefCell<Vec<(PathBuf, PathBuf, PathBuf)>>,
    refuse: Option<String>,
}

impl FakeStagingBuilder {
    fn writing() -> Self {
        Self { calls: RefCell::new(Vec::new()), refuse: None }
    }
    fn refusing(why: &str) -> Self {
        Self { calls: RefCell::new(Vec::new()), refuse: Some(why.to_owned()) }
    }
}

impl StagingBuilder for FakeStagingBuilder {
    fn build(&self, document: &Path, identity: &Path, at: &Path) -> Result<String, String> {
        self.calls
            .borrow_mut()
            .push((document.to_path_buf(), identity.to_path_buf(), at.to_path_buf()));
        if let Some(why) = &self.refuse {
            return Err(why.clone());
        }
        let doc = rivet_config::Document::parse(&std::fs::read_to_string(document).expect("the document"))
            .expect("a document");
        let bundle = rivet_core::staging::render(&[
            rivet_core::staging::Staged::new("rivet-proxyd", "tls-cert", b"a fixture leaf".to_vec())
                .expect("a staged file"),
        ]);
        std::fs::write(staging_path_for_document(document, &doc), bundle).expect("the bundle");
        Ok("    rivet-proxyd/tls-cert  14 bytes\n".to_owned())
    }
}

/// A fleet member's document, as the interview composes one for a kiosk.
const FIXTURE_KIOSK_MEMBER_DOCUMENT: &str = "\
version = 1
hostname = \"example-member\"

[network.eth0]
method = \"dhcp\"

[control-plane]
listen = \"0.0.0.0:7777\"

[kiosk]
auto-run = \"rivet-launcher\"

[applications.rivet-proxyd]
upstream-name = \"example-member.local\"

[services.rivet-displayd]
enabled = true

[services.rivet-launcher]
enabled = true

[services.rivet-mdnsd]
enabled = true

[services.rivet-proxyd]
enabled = true
";

/// **The D8 test.** A member's document arrives with `--document` and no
/// bundle beside it; `run_with` builds one with the machine's identity and
/// hands THAT bundle to the writer, and the report says it was built.
#[test]
fn a_member_document_with_no_bundle_gets_one_built_and_handed_to_the_writer() {
    let scratch = Scratch::new("d8-member");
    let doc_path = scratch.0.join("example-member.provisioning.toml");
    std::fs::write(&doc_path, FIXTURE_KIOSK_MEMBER_DOCUMENT).expect("the document");
    let identity = scratch.0.join("identity").join("example-member");
    let bundle = scratch.0.join("example-member.staged.txt");
    assert!(!bundle.exists(), "the fixture starts with no bundle");

    let builder = FakeStagingBuilder::writing();
    let staging_check = RealStagingCheck { staging_override: None, at: &scratch.0, builder: Some(&builder) };
    let writer = FakeWriter::nothing_written("the image is ready");
    let mut args = empty_args();
    args.document_path = Some(doc_path.clone());

    let report = run_with(
        false,
        &FakePreflight::ok(),
        &FakeInterview::err("must not run -- --document PATH was given"),
        &staging_check,
        &FakeFleetRecorder::ok(None),
        &writer,
        &FakeImageSelector::ok(),
        None,
        args,
        Some([0x42u8; 32]),
        Some(identity.clone()),
        None,
        &FakeExistingDocument::never(),
    )
    .expect("the bundle is built and the run goes on");

    assert_eq!(
        *builder.calls.borrow(),
        vec![(doc_path.clone(), identity, scratch.0.clone())],
        "built once, for this document, with this machine's identity, under --at"
    );
    assert!(bundle.is_file(), "the bundle is beside the document");
    let seen = writer.injection_args_seen.borrow();
    assert_eq!(seen.len(), 1, "the writer ran");
    assert_eq!(seen[0].0.as_deref(), Some(bundle.as_path()), "the writer was handed the built bundle");
    assert!(report.contains("`rivet-provision stage` built"), "{report}");
    assert!(report.contains("rivet-proxyd/tls-cert"), "{report}");
}

/// A bundle already beside the document is used as it is — nothing is built.
#[test]
fn a_bundle_already_beside_the_document_is_not_built_again() {
    let scratch = Scratch::new("d8-present");
    let doc_path = scratch.0.join("example-member.provisioning.toml");
    std::fs::write(&doc_path, FIXTURE_KIOSK_MEMBER_DOCUMENT).expect("the document");
    let bundle = scratch.0.join("example-member.staged.txt");
    std::fs::write(&bundle, rivet_core::staging::render(&[])).expect("a bundle");

    let builder = FakeStagingBuilder::writing();
    let check = RealStagingCheck { staging_override: None, at: &scratch.0, builder: Some(&builder) };
    let (found, said) = check.check(&doc_path, Some(&scratch.0)).expect("found");
    assert_eq!(found, Some(bundle));
    assert!(said.is_empty(), "{said}");
    assert!(builder.calls.borrow().is_empty());
}

/// A document named some other way is built a bundle named for its host —
/// the rule `rivet-provision stage` writes by.
#[test]
fn a_hand_named_document_is_built_a_bundle_named_for_its_host() {
    let scratch = Scratch::new("d8-hand-named");
    let doc_path = scratch.0.join("dev.toml");
    std::fs::write(&doc_path, FIXTURE_KIOSK_MEMBER_DOCUMENT).expect("the document");
    let builder = FakeStagingBuilder::writing();
    let check = RealStagingCheck { staging_override: None, at: &scratch.0, builder: Some(&builder) };
    let (found, _) = check.check(&doc_path, Some(&scratch.0)).expect("built");
    assert_eq!(found, Some(scratch.0.join("example-member.staged.txt")));
}

/// **`--staging PATH` naming nothing is refused, never built**, and so is
/// `--existing HOSTNAME`'s route, which runs with no builder.
#[test]
fn a_named_bundle_or_the_existing_route_is_never_built() {
    let scratch = Scratch::new("d8-never");
    let doc_path = scratch.0.join("example-member.provisioning.toml");
    std::fs::write(&doc_path, FIXTURE_KIOSK_MEMBER_DOCUMENT).expect("the document");

    let builder = FakeStagingBuilder::writing();
    let named = scratch.0.join("somewhere.staged.txt");
    let check = RealStagingCheck { staging_override: Some(&named), at: &scratch.0, builder: Some(&builder) };
    let why = check.check(&doc_path, Some(&scratch.0)).expect_err("a named bundle that is not there");
    assert!(why.contains("somewhere.staged.txt"), "{why}");

    let existing = RealStagingCheck { staging_override: None, at: &scratch.0, builder: None };
    let why = existing.check(&doc_path, Some(&scratch.0)).expect_err("the existing route refuses");
    assert!(why.contains("staging bundle"), "{why}");

    assert!(builder.calls.borrow().is_empty(), "nothing was built");
}

/// A builder that refuses stops the run, in its own words, before the writer.
#[test]
fn a_build_that_fails_stops_the_run_before_the_writer() {
    let scratch = Scratch::new("d8-refused");
    let doc_path = scratch.0.join("example-member.provisioning.toml");
    std::fs::write(&doc_path, FIXTURE_KIOSK_MEMBER_DOCUMENT).expect("the document");
    let builder = FakeStagingBuilder::refusing("no certificate authority at /nowhere/ca");
    let staging_check = RealStagingCheck { staging_override: None, at: &scratch.0, builder: Some(&builder) };
    let writer = FakeWriter::ok("must not be reached");
    let mut args = empty_args();
    args.document_path = Some(doc_path);

    let why = run_with(
        false,
        &FakePreflight::ok(),
        &FakeInterview::err("must not run"),
        &staging_check,
        &FakeFleetRecorder::ok(None),
        &writer,
        &FakeImageSelector::ok(),
        None,
        args,
        Some([0x42u8; 32]),
        Some(scratch.0.clone()),
        None,
        &FakeExistingDocument::never(),
    )
    .expect_err("the build refused");
    assert!(why.contains("NOTHING WAS WRITTEN"), "{why}");
    assert!(why.contains("no certificate authority at /nowhere/ca"), "{why}");
    assert!(writer.seen.borrow().is_empty(), "the writer was reached");
}

/// **`ADR-060` §5 refuses an old image before anything is built.** A card that
/// was never going to be written leaves no bundle minted behind it.
#[test]
fn an_image_without_the_proxy_is_refused_before_a_bundle_is_built() {
    let scratch = Scratch::new("d8-old-image");
    let doc_path = scratch.0.join("example-member.provisioning.toml");
    std::fs::write(&doc_path, FIXTURE_KIOSK_MEMBER_DOCUMENT).expect("the document");
    let builder = FakeStagingBuilder::writing();
    let staging_check = RealStagingCheck { staging_override: None, at: &scratch.0, builder: Some(&builder) };
    let mut selector = FakeImageSelector::ok_with_scratch("/fixture/old.img");
    if let Ok(Some(selected)) = &mut selector.answer {
        selected.version = Some("0.1.113".to_owned());
    }
    let mut args = empty_args();
    args.document_path = Some(doc_path);

    let why = run_with(
        false,
        &FakePreflight::ok(),
        &FakeInterview::err("must not run"),
        &staging_check,
        &FakeFleetRecorder::ok(None),
        &FakeWriter::ok("must not be reached"),
        &selector,
        None,
        args,
        Some([0x42u8; 32]),
        Some(scratch.0.clone()),
        None,
        &FakeExistingDocument::never(),
    )
    .expect_err("an image older than the proxy's");
    assert!(why.contains("0.1.113"), "{why}");
    assert!(builder.calls.borrow().is_empty(), "a bundle was built for a refused card");
}

// ---------------------------------------------------------------------------
// A freshly enrolled fleet server becomes the Armada's Fleet, so the member
// provisioned next is staged trusting its fleetd key.
// ---------------------------------------------------------------------------

/// A fleet server on a static address, with the operator key its staging
/// bundle names read onto its record.
fn server_record(name: &str, address: &str, operator_key: Option<&str>) -> rivetos_register::Device {
    let mut device = rivetos_register::Device {
        id: rivet_core::identifier::from_entropy(name.as_bytes()),
        name: name.to_owned(),
        fleet_role: Some(rivetos_register::FleetRole::Server),
        pin: Some("ab".repeat(32)),
        listen: Some("0.0.0.0:7777".to_owned()),
        operator_key: operator_key.map(str::to_owned),
        ..rivetos_register::Device::default()
    };
    device.interfaces.insert(
        "eth0".to_owned(),
        rivetos_register::Interface {
            method: "static".to_owned(),
            address: Some(address.to_owned()),
            gateway: None,
        },
    );
    device
}

#[test]
fn committing_a_fresh_server_adds_it_to_an_empty_armada_with_its_operator_key() {
    let scratch = Scratch::new("armada-fresh");
    let doc_path = write_document_fixture(&scratch, "example-server.provisioning.toml", FIXTURE_SERVER_DOCUMENT);
    let register_root = scratch.0.join("register");
    let armada_root = scratch.0.join("armada");
    let recorder =
        RealFleetRecorder { fleet_repo_root: None, register_root: &register_root, armada_root: Some(&armada_root) };
    let key = "cd".repeat(32);

    let report = recorder.commit(&doc_path, Some(server_record("example-server", "192.0.2.10/24", Some(&key))));
    assert!(report.contains("armada: Fleet \"example-server\" added"), "{report}");

    let armada = rivetos_register::armada::store::load(&armada_root).expect("readable");
    let fleet = armada.fleets.get("example-server").expect("the Fleet");
    assert_eq!(fleet.server.host, "192.0.2.10:7777");
    assert_eq!(fleet.server.pin, "ab".repeat(32));
    assert_eq!(fleet.server.operator_key.as_deref(), Some(key.as_str()));
}

#[test]
fn an_armada_holding_another_fleet_is_left_alone_and_the_command_is_named() {
    let scratch = Scratch::new("armada-other");
    let doc_path = write_document_fixture(&scratch, "example-server.provisioning.toml", FIXTURE_SERVER_DOCUMENT);
    let register_root = scratch.0.join("register");
    let armada_root = scratch.0.join("armada");
    let mut armada = rivetos_register::armada::Armada::empty(armada_root.clone());
    armada.fleets.insert(
        "older".to_owned(),
        rivetos_register::armada::Fleet {
            name: "older".to_owned(),
            server: rivetos_register::armada::Server {
                host: "192.0.2.99:7777".to_owned(),
                pin: "ef".repeat(32),
                operator_key: Some("01".repeat(32)),
            },
        },
    );
    rivetos_register::armada::store::save(&armada).expect("an Armada");
    let recorder =
        RealFleetRecorder { fleet_repo_root: None, register_root: &register_root, armada_root: Some(&armada_root) };

    let report = recorder.commit(&doc_path, Some(server_record("example-server", "192.0.2.10/24", Some("cd"))));
    assert!(report.contains("NOT added"), "{report}");
    assert!(report.contains("cargo rivet armada add example-server"), "{report}");
    let after = rivetos_register::armada::store::load(&armada_root).expect("readable");
    assert_eq!(after.fleets.keys().collect::<Vec<_>>(), vec!["older"]);
}

/// A MEMBER's commit never touches the Armada.
#[test]
fn committing_a_member_adds_nothing_to_the_armada() {
    let scratch = Scratch::new("member-commit");
    let doc_path = write_document_fixture(&scratch, "example-member.provisioning.toml", FIXTURE_KIOSK_MEMBER_DOCUMENT);
    let register_root = scratch.0.join("register");
    let armada_root = scratch.0.join("armada");
    let recorder =
        RealFleetRecorder { fleet_repo_root: None, register_root: &register_root, armada_root: Some(&armada_root) };
    let mut member = server_record("example-member", "192.0.2.11/24", None);
    member.fleet_role = Some(rivetos_register::FleetRole::Member);
    let report = recorder.commit(&doc_path, Some(member));
    assert!(!report.contains("armada:"), "{report}");
    assert!(!armada_root.exists());
}

// ---------------------------------------------------------------------------
// role_of / enrollment_device — pure.
// ---------------------------------------------------------------------------

/// A fictional but realistic server document — `AGENTS.md` §9, R-172.1: a
/// made-up hostname, a TEST-NET address is not even needed here (DHCP), and
/// two `[carry.<product>]` pins as a document recorded before `ADR-060` holds
/// them: `rivet-fleetd`'s, which a card still carries, and `rivet-proxyd`'s,
/// which names an image service and must be ignored. `resolve_carry` (in
/// `cmd/rivet-provision`) writes only the first kind.
const FIXTURE_SERVER_DOCUMENT: &str = "\
version = 1
hostname = \"example-server\"

[network.eth0]
method = \"dhcp\"

[control-plane]
listen = \"0.0.0.0:7777\"

[applications.rivet-proxyd]
upstream-name = \"example-server.local\"

[services.rivet-fleetd]
enabled = true

[services.rivet-mdnsd]
enabled = true

[services.rivet-proxyd]
enabled = true

[carry.rivet-fleetd]
version = \"0.1.86\"

[carry.rivet-proxyd]
version = \"0.1.86\"
";

/// The same shape, for a member — no `rivet-fleetd`, so its one pin is the
/// legacy proxy pin and it carries nothing.
const FIXTURE_MEMBER_DOCUMENT: &str = "\
version = 1
hostname = \"example-member\"

[network.eth0]
method = \"dhcp\"

[control-plane]
listen = \"0.0.0.0:7777\"

[applications.rivet-proxyd]
upstream-name = \"example-member.local\"

[services.rivet-mdnsd]
enabled = true

[services.rivet-proxyd]
enabled = true

[carry.rivet-proxyd]
version = \"0.1.86\"
";

const FIXTURE_STANDALONE_DOCUMENT: &str = "version = 1\nhostname = \"example-standalone\"\n";

#[test]
fn role_of_reads_server_member_and_stand_alone_off_the_documents_own_services() {
    let server = rivet_config::Document::parse(FIXTURE_SERVER_DOCUMENT).expect("a server document");
    assert_eq!(role_of(&server), Some(rivetos_register::FleetRole::Server));

    let member = rivet_config::Document::parse(FIXTURE_MEMBER_DOCUMENT).expect("a member document");
    assert_eq!(role_of(&member), Some(rivetos_register::FleetRole::Member));

    let standalone =
        rivet_config::Document::parse(FIXTURE_STANDALONE_DOCUMENT).expect("a stand-alone document");
    assert_eq!(role_of(&standalone), None);
}

/// A kiosk with neither `rivet-fleetd` nor `rivet-proxyd` enabled is also
/// `None` — Q7's "for a server or member" excludes it exactly as a
/// stand-alone machine is excluded.
#[test]
fn role_of_a_kiosk_with_no_fleet_role_is_none() {
    let kiosk = rivet_config::Document::parse(
        "version = 1\nhostname = \"kiosk-1\"\n\n[kiosk]\nauto-run = \"aura-controller\"\n\n\
         [services.aura-controller]\nenabled = true\n",
    )
    .expect("a kiosk document");
    assert_eq!(role_of(&kiosk), None);
}

#[test]
fn enrollment_device_derives_its_id_from_the_pin_and_carries_the_documents_own_pins() {
    let doc = rivet_config::Document::parse(FIXTURE_SERVER_DOCUMENT).expect("a server document");
    let pin = [0x55u8; 32];
    let device = enrollment_device("example-server", &doc, rivetos_register::FleetRole::Server, &pin, None);

    assert_eq!(device.id, rivet_core::identifier::from_entropy(&pin));
    assert_eq!(device.name, "example-server");
    assert_eq!(device.fleet_role, Some(rivetos_register::FleetRole::Server));
    assert_eq!(device.pin.as_deref(), Some(rivetos_tls::encode_hex(&pin).as_str()));
    assert_eq!(device.listen.as_deref(), Some("0.0.0.0:7777"));
    assert_eq!(device.carry.get("rivet-fleetd").map(String::as_str), Some("0.1.86"));
}

/// **A legacy pin on an image service is not recorded.** The proxy is part of
/// the image (`ADR-060`), so a register record holding its pin would hold one
/// no card build reads — the shape dev-01's record had, from a document
/// written before the proxy joined the image.
#[test]
fn enrollment_device_does_not_record_a_pin_on_an_image_service() {
    let pin = [0x57u8; 32];
    let server = rivet_config::Document::parse(FIXTURE_SERVER_DOCUMENT).expect("a server document");
    let device = enrollment_device("example-server", &server, rivetos_register::FleetRole::Server, &pin, None);
    assert_eq!(device.carry.get("rivet-proxyd"), None, "{:?}", device.carry);

    let member = rivet_config::Document::parse(FIXTURE_MEMBER_DOCUMENT).expect("a member document");
    let device = enrollment_device("example-member", &member, rivetos_register::FleetRole::Member, &pin, None);
    assert!(device.carry.is_empty(), "{:?}", device.carry);
}

/// **Re-provisioning the same board produces the same identifier** —
/// `ADR-049` §8's own words — because it is a pure function of the pin.
#[test]
fn the_same_pin_always_derives_the_same_identifier() {
    let doc = rivet_config::Document::parse(FIXTURE_SERVER_DOCUMENT).expect("a server document");
    let pin = [0x66u8; 32];
    let first = enrollment_device("example-server", &doc, rivetos_register::FleetRole::Server, &pin, None);
    let second = enrollment_device("example-server", &doc, rivetos_register::FleetRole::Server, &pin, None);
    assert_eq!(first.id, second.id);
}

/// **A SERVER's enrollment record carries its own fleet-operator key off the
/// staged bundle; a MEMBER's does not.** `docs/PLAN.md` §183.2, the capture
/// half: `stage::register` writes the server's operator credential as the
/// staged register's one `fleet.control_plane_keys` entry, and this is where it
/// crosses onto the workstation register record so the armada (and, in turn, a
/// member joining this fleet) can reach it. A member runs no `rivet-fleetd` and
/// stages no register of its own, so even handed the same bundle it records
/// none.
#[test]
fn a_server_captures_its_operator_key_from_the_staged_bundle_and_a_member_does_not() {
    // A staging bundle shaped exactly as `stage::register` stages it: the
    // fleet-wide `register/register.toml`, under the `rivet-fleetd`
    // application, whose `fleet.control_plane_keys` holds the one operator key.
    let key = "ab".repeat(32);
    let fleet_toml = format!("control_plane_keys = [\"{key}\"]\n");
    let item = rivet_core::staging::Staged::new(
        "rivet-fleetd",
        "register/register.toml",
        fleet_toml.into_bytes(),
    )
    .expect("a staged item");
    let bundle = rivet_core::staging::render(std::slice::from_ref(&item));

    let pin = [0x5au8; 32];

    // A server, handed the bundle, records the key.
    let server_doc = rivet_config::Document::parse(FIXTURE_SERVER_DOCUMENT).expect("a server document");
    let server = enrollment_device(
        "example-server",
        &server_doc,
        rivetos_register::FleetRole::Server,
        &pin,
        Some(&bundle),
    );
    assert_eq!(server.operator_key.as_deref(), Some(key.as_str()));

    // A server with no bundle available records none — best-effort, unchanged
    // from before this existed.
    let server_no_bundle = enrollment_device(
        "example-server",
        &server_doc,
        rivetos_register::FleetRole::Server,
        &pin,
        None,
    );
    assert_eq!(server_no_bundle.operator_key, None);

    // A member, handed the SAME bundle, records none — gated on the role.
    let member_doc = rivet_config::Document::parse(FIXTURE_MEMBER_DOCUMENT).expect("a member document");
    let member = enrollment_device(
        "example-member",
        &member_doc,
        rivetos_register::FleetRole::Member,
        &pin,
        Some(&bundle),
    );
    assert_eq!(member.operator_key, None);
}

// ---------------------------------------------------------------------------
// R-180.1: a static address is recorded at enrollment; a DHCP one is not.
// ---------------------------------------------------------------------------

const FIXTURE_STATIC_SERVER_DOCUMENT: &str = "\
version = 1
hostname = \"static-server\"

[network.eth0]
method = \"static\"
address = \"192.168.50.10/24\"
gateway = \"192.168.50.1\"

[control-plane]
listen = \"0.0.0.0:7777\"

[applications.rivet-proxyd]
upstream-name = \"static-server.local\"

[services.rivet-fleetd]
enabled = true

[services.rivet-mdnsd]
enabled = true

[services.rivet-proxyd]
enabled = true
";

#[test]
fn a_static_address_is_recorded_in_the_devices_interfaces_at_enrollment() {
    let doc = rivet_config::Document::parse(FIXTURE_STATIC_SERVER_DOCUMENT).expect("a server document");
    let pin = [0x77u8; 32];
    let device = enrollment_device("static-server", &doc, rivetos_register::FleetRole::Server, &pin, None);

    let eth0 = device.interfaces.get("eth0").expect("eth0 must be recorded");
    assert_eq!(eth0.method, "static");
    assert_eq!(eth0.address.as_deref(), Some("192.168.50.10/24"));
    assert_eq!(eth0.gateway.as_deref(), Some("192.168.50.1"));
}

/// **A DHCP interface records nothing, as today** — the maintainer's own
/// words (R-180.1). `FIXTURE_SERVER_DOCUMENT`'s `eth0` takes a lease.
#[test]
fn a_dhcp_interface_records_nothing() {
    let doc = rivet_config::Document::parse(FIXTURE_SERVER_DOCUMENT).expect("a server document");
    let pin = [0x88u8; 32];
    let device = enrollment_device("example-server", &doc, rivetos_register::FleetRole::Server, &pin, None);
    assert!(device.interfaces.is_empty(), "{:?}", device.interfaces);
}

/// A static address with no gateway records `gateway: None` rather than
/// inventing one.
#[test]
fn a_static_address_with_no_gateway_records_none_for_it() {
    let text = "\
version = 1
hostname = \"static-no-gw\"

[network.eth0]
method = \"static\"
address = \"10.0.0.5/24\"

[control-plane]
listen = \"0.0.0.0:7777\"

[applications.rivet-proxyd]
upstream-name = \"static-no-gw.local\"

[services.rivet-proxyd]
enabled = true

[services.rivet-mdnsd]
enabled = true
";
    let doc = rivet_config::Document::parse(text).expect("a member document");
    let pin = [0x99u8; 32];
    let device = enrollment_device("static-no-gw", &doc, rivetos_register::FleetRole::Member, &pin, None);
    let eth0 = device.interfaces.get("eth0").expect("eth0 must be recorded");
    assert_eq!(eth0.address.as_deref(), Some("10.0.0.5/24"));
    assert_eq!(eth0.gateway, None);
}

// ---------------------------------------------------------------------------
// R-180: `--existing HOSTNAME` — safe_name, redirect_for, RealExistingDocument.
// ---------------------------------------------------------------------------

#[test]
fn extract_existing_pulls_the_flag_and_its_value_out_of_argv() {
    let args = vec!["--existing".to_owned(), "arbor".to_owned(), "--image".to_owned(), "x.img".to_owned()];
    let (rest, existing) = extract_existing(&args);
    assert_eq!(rest, ["--image", "x.img"]);
    assert_eq!(existing, Some("arbor".to_owned()));
}

#[test]
fn extract_existing_with_no_flag_returns_argv_unchanged() {
    let args = vec!["--image".to_owned(), "x.img".to_owned()];
    let (rest, existing) = extract_existing(&args);
    assert_eq!(rest, args);
    assert_eq!(existing, None);
}

#[test]
fn redirect_for_names_the_full_command_with_the_hostname_filled_in() {
    let said = redirect_for("arbor");
    assert!(said.contains("cargo rivet provision"), "{said}");
    assert!(said.contains("--existing"), "{said}");
    assert!(said.contains("\"arbor\""), "{said}");
}

#[test]
fn safe_name_matches_a_plain_hostname_and_replaces_anything_hostile() {
    assert_eq!(safe_name("arbor"), "arbor");
    assert_eq!(safe_name("arbor-01"), "arbor-01");
    assert_eq!(safe_name("../../etc/passwd"), "------etc-passwd");
    assert_eq!(safe_name(""), "unnamed");
}

/// A duplicate refusal names the redirect, with the hostname filled in.
#[test]
fn a_duplicate_refusal_names_the_existing_redirect() {
    let scratch = Scratch::new("redirect-dup-id");
    let doc_path = write_document_fixture(&scratch, "doc.toml", FIXTURE_SERVER_DOCUMENT);
    let register_root = scratch.0.join("register");
    let pin = [0xa1u8; 32];
    let id = rivet_core::identifier::from_entropy(&pin);

    let mut register = rivetos_register::Register::empty(&register_root);
    register
        .enroll(rivetos_register::Device { id: id.clone(), name: "already-here".to_owned(), ..Default::default() })
        .expect("fresh");
    rivetos_register::store::save(&register).expect("writable");

    let recorder = RealFleetRecorder { fleet_repo_root: None, register_root: &register_root, armada_root: None };
    let err = recorder.plan(&doc_path, Some(&pin)).expect_err("the id is already enrolled");
    assert!(err.contains("--existing \"example-server\""), "{err}");
}

#[test]
fn real_existing_document_locates_the_recorded_document_and_writes_it_locally() {
    let scratch = Scratch::new("existing-found");
    let fleet_repo_root = scratch.0.join("fleet-repo");
    let at = scratch.0.join("home");
    std::fs::create_dir_all(fleet_repo_root.join(rivetos_fleet_repo::DOCUMENTS_DIR))
        .expect("scratch fleet repo");
    std::fs::write(
        fleet_repo_root.join(rivetos_fleet_repo::DOCUMENTS_DIR).join("arbor.provisioning.toml"),
        FIXTURE_SERVER_DOCUMENT,
    )
    .expect("write the fixture recorded document");

    let locator = RealExistingDocument { fleet_repo_root: Some(&fleet_repo_root), at: &at };
    let local = locator.locate("arbor").expect("recorded and found");

    assert_eq!(local, at.join("provision").join("arbor.provisioning.toml"));
    assert_eq!(std::fs::read_to_string(&local).expect("written locally"), FIXTURE_SERVER_DOCUMENT);
}

#[test]
fn real_existing_document_refuses_an_unknown_hostname_by_name() {
    let scratch = Scratch::new("existing-unknown");
    let fleet_repo_root = scratch.0.join("fleet-repo");
    std::fs::create_dir_all(&fleet_repo_root).expect("scratch fleet repo");
    let at = scratch.0.join("home");

    let locator = RealExistingDocument { fleet_repo_root: Some(&fleet_repo_root), at: &at };
    let err = locator.locate("nonesuch").expect_err("nothing was ever recorded for it");
    assert!(err.starts_with("NOTHING WAS WRITTEN"), "{err}");
    assert!(err.contains("nonesuch"), "{err}");
    assert!(!at.join("provision").join("nonesuch.provisioning.toml").exists());
}

#[test]
fn real_existing_document_refuses_with_no_fleet_repository_at_all() {
    let locator = RealExistingDocument { fleet_repo_root: None, at: Path::new("/nowhere") };
    let err = locator.locate("arbor").expect_err("no repository to look in");
    assert!(err.starts_with("NOTHING WAS WRITTEN"), "{err}");
}

// ---------------------------------------------------------------------------
// R-180: the redirect route through `run_with` — no interview, no
// enrollment, no commit; the write happens; an unknown hostname is refused.
// ---------------------------------------------------------------------------

#[test]
fn the_existing_route_runs_no_interview_plans_no_enrollment_and_commits_nothing_and_the_write_happens() {
    let preflight = FakePreflight::ok();
    let interview = FakeInterview::err("must not run -- --existing skips the interview entirely");
    let staging_check = FakeStagingCheck::ok();
    // Would enroll Some(device) if `plan` were ever called at all.
    let fleet_recorder = FakeFleetRecorder::ok(Some(fixture_device("arbor")));
    let writer = FakeWriter::ok("wrote and verified it");
    let existing_document = FakeExistingDocument::ok("/agent/arbor.provisioning.toml");

    let mut args = empty_args();
    args.device_path = Some(PathBuf::from("/dev/sd-fixture"));

    let report = run_with(
        false,
        &preflight,
        &interview,
        &staging_check,
        &fleet_recorder,
        &writer,
        &FakeImageSelector::ok(),
        None,
        args,
        None,
        None,
        Some("arbor"),
        &existing_document)
    .expect("every fake answers Ok");

    assert!(report.contains("wrote and verified it"), "{report}");
    assert!(!preflight.called.get(), "no preflight on the redirect route");
    assert!(!interview.called.get(), "no interview on the redirect route");
    assert_eq!(existing_document.calls.borrow().as_slice(), ["arbor".to_owned()]);
    assert_eq!(
        staging_check.seen.borrow().as_slice(),
        [PathBuf::from("/agent/arbor.provisioning.toml")]
    );
    assert!(fleet_recorder.plan_calls.borrow().is_empty(), "no enrollment planning on the redirect route");
    assert!(fleet_recorder.commit_calls.borrow().is_empty(), "no commit on the redirect route");
}

/// A document for `host` in a scratch file, for the re-role route.
fn scratch_document(tag: &str, host: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("rivet-rerole-{tag}-{}.provisioning.toml", std::process::id()));
    std::fs::write(&path, format!("version = 1\nhostname = \"{host}\"\n")).expect("written");
    path
}

/// **`--existing HOSTNAME --document PATH` re-roles a recorded machine**
/// (`ADR-063` §9 step 5): the NEW document is written, the recorded one is not
/// looked up, and nothing is planned or committed — the machine keeps its
/// identity and its record.
#[test]
fn the_rerole_route_writes_the_new_document_and_records_nothing() {
    let document = scratch_document("ok", "arbor");
    let staging_check = FakeStagingCheck::ok();
    let fleet_recorder = FakeFleetRecorder::ok(Some(fixture_device("arbor")));
    let writer = FakeWriter::ok("wrote and verified it");
    let existing_document = FakeExistingDocument::ok("/agent/recorded.provisioning.toml");
    let mut args = empty_args();
    args.document_path = Some(document.clone());
    args.device_path = Some(PathBuf::from("/dev/sd-fixture"));

    let report = run_with(
        false,
        &FakePreflight::ok(),
        &FakeInterview::err("must not run"),
        &staging_check,
        &fleet_recorder,
        &writer,
        &FakeImageSelector::ok(),
        None,
        args,
        None,
        None,
        Some("arbor"),
        &existing_document,
    )
    .expect("every fake answers Ok");

    assert!(report.contains("wrote and verified it"), "{report}");
    assert!(existing_document.calls.borrow().is_empty(), "the recorded document was used");
    assert_eq!(staging_check.seen.borrow().as_slice(), [document.clone()]);
    assert!(fleet_recorder.plan_calls.borrow().is_empty());
    assert!(fleet_recorder.commit_calls.borrow().is_empty());
    let _ = std::fs::remove_file(document);
}

/// A new document for ANOTHER machine is never written with this one's
/// identity.
#[test]
fn the_rerole_route_refuses_a_document_for_another_machine() {
    let document = scratch_document("other", "vega");
    let writer = FakeWriter::ok("must not run");
    let mut args = empty_args();
    args.document_path = Some(document.clone());
    let why = run_with(
        false,
        &FakePreflight::ok(),
        &FakeInterview::err("must not run"),
        &FakeStagingCheck::ok(),
        &FakeFleetRecorder::ok(None),
        &writer,
        &FakeImageSelector::ok(),
        None,
        args,
        None,
        None,
        Some("arbor"),
        &FakeExistingDocument::ok("/agent/recorded.provisioning.toml"),
    )
    .expect_err("refused");
    assert!(why.contains("NOTHING WAS WRITTEN"), "{why}");
    assert!(why.contains("vega"), "{why}");
    let _ = std::fs::remove_file(document);
}

#[test]
fn the_stage_flags_are_taken_out_for_rivet_provision_stage_in_order() {
    let argv: Vec<String> = [
        "--document", "d.toml", "--fleet-root", "dev-fleet", "confirm", "--passkeys", "p", "--suffixes",
        "local,dev-fleet.lan",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    let (rest, extra) = extract_stage_extras(&argv).expect("parses");
    assert_eq!(rest, vec!["--document", "d.toml", "confirm"]);
    assert_eq!(
        extra,
        vec!["--fleet-root", "dev-fleet", "--passkeys", "p", "--suffixes", "local,dev-fleet.lan"]
    );
    assert!(extract_stage_extras(&["--fleet-root".to_owned()]).is_err());
}

#[test]
fn an_unknown_hostname_is_refused_by_name_on_the_existing_route() {
    let preflight = FakePreflight::ok();
    let interview = FakeInterview::err("must not run");
    let staging_check = FakeStagingCheck::ok();
    let fleet_recorder = FakeFleetRecorder::ok(None);
    let writer = FakeWriter::ok("must not run");
    let existing_document = FakeExistingDocument::err(
        "NOTHING WAS WRITTEN. cargo rivet provision --existing \"nonesuch\" found no recorded document",
    );

    let why = run_with(
        false,
        &preflight,
        &interview,
        &staging_check,
        &fleet_recorder,
        &writer,
        &FakeImageSelector::ok(),
        None,
        empty_args(),
        None,
        None,
        Some("nonesuch"),
        &existing_document)
    .expect_err("an unknown hostname must refuse");

    assert!(why.starts_with("NOTHING WAS WRITTEN"), "{why}");
    assert!(why.contains("nonesuch"), "{why}");
    assert!(writer.seen.borrow().is_empty(), "the writer must never have run");
}

// ---------------------------------------------------------------------------
// RealFleetRecorder — R-178 Q7/Q10, against real scratch directories.
// Never the maintainer's real fleet repository or register — every path
// here is a fresh temporary directory, removed when the test's `Scratch`
// drops.
// ---------------------------------------------------------------------------

fn write_document_fixture(scratch: &Scratch, name: &str, text: &str) -> PathBuf {
    let path = scratch.0.join(name);
    std::fs::write(&path, text).expect("write the fixture document");
    path
}

#[test]
fn plan_returns_none_for_a_stand_alone_document_and_never_touches_the_register() {
    let scratch = Scratch::new("plan-standalone");
    let doc_path = write_document_fixture(&scratch, "doc.toml", FIXTURE_STANDALONE_DOCUMENT);
    // A register root that does not even exist — if this were consulted at
    // all, `store::load` would still answer "empty" rather than erroring, so
    // the real proof is that NOTHING is written to it, checked below.
    let register_root = scratch.0.join("register");
    let recorder = RealFleetRecorder { fleet_repo_root: None, register_root: &register_root, armada_root: None };

    let plan = recorder.plan(&doc_path, None).expect("a stand-alone document needs no pin");
    assert_eq!(plan, None);
    assert!(!register_root.exists(), "a stand-alone plan must not create a register");
}

#[test]
fn plan_refuses_a_server_document_with_no_pin_available() {
    let scratch = Scratch::new("plan-no-pin");
    let doc_path = write_document_fixture(&scratch, "doc.toml", FIXTURE_SERVER_DOCUMENT);
    let register_root = scratch.0.join("register");
    let recorder = RealFleetRecorder { fleet_repo_root: None, register_root: &register_root, armada_root: None };

    let err = recorder.plan(&doc_path, None).expect_err("no pin was given");
    assert!(err.starts_with("NOTHING WAS WRITTEN"), "{err}");
    assert!(err.contains("example-server"), "{err}");
    assert!(err.contains("--identity"), "{err}");
    assert!(err.contains("--pin"), "{err}");
}

#[test]
fn plan_returns_the_device_for_a_fresh_server_document() {
    let scratch = Scratch::new("plan-fresh");
    let doc_path = write_document_fixture(&scratch, "doc.toml", FIXTURE_SERVER_DOCUMENT);
    let register_root = scratch.0.join("register");
    let recorder = RealFleetRecorder { fleet_repo_root: None, register_root: &register_root, armada_root: None };
    let pin = [0x99u8; 32];

    let device = recorder.plan(&doc_path, Some(&pin)).expect("a fresh register").expect("a server");
    assert_eq!(device.id, rivet_core::identifier::from_entropy(&pin));
    assert_eq!(device.name, "example-server");
}

#[test]
fn plan_refuses_when_the_derived_identifier_is_already_enrolled() {
    let scratch = Scratch::new("plan-dup-id");
    let doc_path = write_document_fixture(&scratch, "doc.toml", FIXTURE_SERVER_DOCUMENT);
    let register_root = scratch.0.join("register");
    let pin = [0xaau8; 32];
    let id = rivet_core::identifier::from_entropy(&pin);

    let mut register = rivetos_register::Register::empty(&register_root);
    register
        .enroll(rivetos_register::Device { id: id.clone(), name: "already-here".to_owned(), ..Default::default() })
        .expect("fresh");
    rivetos_register::store::save(&register).expect("writable");

    let recorder = RealFleetRecorder { fleet_repo_root: None, register_root: &register_root, armada_root: None };
    let err = recorder.plan(&doc_path, Some(&pin)).expect_err("the id is already enrolled");
    assert!(err.starts_with("NOTHING WAS WRITTEN"), "{err}");
    assert!(err.contains(&id), "{err}");
}

#[test]
fn plan_refuses_when_the_hostname_is_already_enrolled_under_a_different_id() {
    let scratch = Scratch::new("plan-dup-name");
    let doc_path = write_document_fixture(&scratch, "doc.toml", FIXTURE_SERVER_DOCUMENT);
    let register_root = scratch.0.join("register");

    let mut register = rivetos_register::Register::empty(&register_root);
    register
        .enroll(rivetos_register::Device {
            id: "ffeeddccbbaa9988".to_owned(),
            name: "example-server".to_owned(),
            ..Default::default()
        })
        .expect("fresh");
    rivetos_register::store::save(&register).expect("writable");

    let recorder = RealFleetRecorder { fleet_repo_root: None, register_root: &register_root, armada_root: None };
    let pin = [0xbbu8; 32]; // derives a DIFFERENT id than "ffeeddccbbaa9988".
    let err = recorder.plan(&doc_path, Some(&pin)).expect_err("the hostname is already enrolled");
    assert!(err.starts_with("NOTHING WAS WRITTEN"), "{err}");
    assert!(err.contains("example-server"), "{err}");
}

/// A device certificate in `dir`, answering its pin — what `--identity PATH`
/// reads.
fn identity_fixture(dir: &Path) -> [u8; 32] {
    std::fs::create_dir_all(dir).expect("a directory");
    let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("a keypair");
    let params = rcgen::CertificateParams::new(vec!["rivet-device".to_owned()]).expect("params");
    let der = params.self_signed(&pair).expect("self-signed").der().to_vec();
    std::fs::write(dir.join(rivetos_tls::CERTIFICATE_FILE), &der).expect("written");
    rivetos_tls::public_key_in_certificate(&der).expect("a public key")
}

/// **`.rulings/` R-189 D2 on the `--document --identity` route: an identity
/// whose pin is on another hostname's record is refused**, naming that
/// record's hostname and identifier — including a record whose identifier was
/// minted at random and so is not the one the pin derives.
#[test]
fn plan_refuses_an_identity_whose_pin_another_hostnames_record_carries() {
    let scratch = Scratch::new("plan-dup-pin");
    let doc_path = write_document_fixture(&scratch, "doc.toml", FIXTURE_SERVER_DOCUMENT);
    let identity = scratch.0.join("identity").join("board");
    let pin = identity_fixture(&identity);
    let register_root = scratch.0.join("register");

    let mut register = rivetos_register::Register::empty(&register_root);
    register
        .enroll(rivetos_register::Device {
            id: "1f0532852afc2842".to_owned(),
            name: "board".to_owned(),
            pin: Some(rivetos_tls::encode_hex(&pin)),
            ..Default::default()
        })
        .expect("fresh");
    rivetos_register::store::save(&register).expect("writable");

    let args: Vec<String> = ["--identity", identity.to_str().expect("utf8")].map(str::to_owned).to_vec();
    let (_, _, from_flag) = extract_identity_and_pin(&args).expect("the identity reads");
    let recorder = RealFleetRecorder { fleet_repo_root: None, register_root: &register_root, armada_root: None };
    let err = recorder.plan(&doc_path, from_flag.as_ref()).expect_err("the pin is another machine's");
    assert!(err.starts_with("NOTHING WAS WRITTEN"), "{err}");
    assert!(err.contains("\"board\""), "{err}");
    assert!(err.contains("1f0532852afc2842"), "{err}");
    assert!(err.contains("Pick or mint another identity directory"), "{err}");
}

/// **`--existing HOSTNAME` takes the machine's own identity when it has one**,
/// and nothing otherwise, so the writer's board default still serves a machine
/// provisioned before identities were per machine.
#[test]
fn the_existing_route_finds_the_machines_own_identity() {
    let scratch = Scratch::new("existing-identity");
    assert_eq!(existing_identity_dir(&scratch.0, "arbor"), None);
    let dir = scratch.0.join("identity").join("arbor");
    identity_fixture(&dir);
    assert_eq!(existing_identity_dir(&scratch.0, "arbor"), Some(dir));
}

#[test]
fn commit_records_the_document_and_enrolls_and_projects_the_register() {
    let scratch = Scratch::new("commit");
    let doc_path = write_document_fixture(&scratch, "doc.toml", FIXTURE_SERVER_DOCUMENT);
    let fleet_repo_root = scratch.0.join("fleet-repo");
    let register_root = scratch.0.join("register");
    let recorder =
        RealFleetRecorder { fleet_repo_root: Some(&fleet_repo_root), register_root: &register_root, armada_root: None };
    let pin = [0xccu8; 32];

    let device = recorder.plan(&doc_path, Some(&pin)).expect("planning").expect("a server");
    let report = recorder.commit(&doc_path, Some(device.clone()));
    assert!(report.contains("example-server"), "{report}");

    // The document landed in the fleet repository, byte for byte.
    let committed_document =
        fleet_repo_root.join(rivetos_fleet_repo::DOCUMENTS_DIR).join("example-server.provisioning.toml");
    assert_eq!(
        std::fs::read_to_string(&committed_document).expect("committed"),
        FIXTURE_SERVER_DOCUMENT
    );

    // The register itself gained the device, and can be read back.
    let saved = rivetos_register::store::load(&register_root).expect("readable");
    assert_eq!(saved.devices.get(&device.id), Some(&device));

    // And the register's projection landed in the fleet repository too.
    let projected_root = fleet_repo_root.join(rivetos_fleet_repo::REGISTER_DIR);
    assert!(
        projected_root.join(rivetos_register::DEVICES_DIR).join(format!("{}.toml", device.id)).is_file(),
        "the device must be projected into the fleet repository's register/ too"
    );
}

/// **A stand-alone document commits itself and enrolls nothing.**
#[test]
fn commit_records_a_stand_alone_document_and_enrolls_nothing() {
    let scratch = Scratch::new("commit-standalone");
    let doc_path = write_document_fixture(&scratch, "doc.toml", FIXTURE_STANDALONE_DOCUMENT);
    let fleet_repo_root = scratch.0.join("fleet-repo");
    let register_root = scratch.0.join("register");
    let recorder =
        RealFleetRecorder { fleet_repo_root: Some(&fleet_repo_root), register_root: &register_root, armada_root: None };

    recorder.commit(&doc_path, None);

    let committed_document = fleet_repo_root
        .join(rivetos_fleet_repo::DOCUMENTS_DIR)
        .join("example-standalone.provisioning.toml");
    assert!(committed_document.is_file(), "the document must still be committed");
    assert!(!register_root.exists(), "nothing must be enrolled for a stand-alone machine");
}

// ---------------------------------------------------------------------------
// R-178 P4/Q7/Q10 — the scratch end-to-end: the non-interactive --document
// route, through `run_with` with a FAKE writer (no disk, no real card — the
// writer's own contract is proven separately by write_image_tests.rs) and
// the REAL fleet recorder against scratch fleet-repo and register roots.
// Proves the ordering AND the real on-disk outcome together: the document
// ends up committed and the register ends up holding the record, and
// neither the real fleet repository nor the real register is ever named.
// ---------------------------------------------------------------------------

#[test]
fn scratch_end_to_end_the_document_route_commits_and_enrolls_after_a_successful_write() {
    let scratch = Scratch::new("e2e");
    let doc_path = write_document_fixture(&scratch, "example-server.provisioning.toml", FIXTURE_SERVER_DOCUMENT);
    let fleet_repo_root = scratch.0.join("fleet-repo");
    let register_root = scratch.0.join("register");
    let recorder =
        RealFleetRecorder { fleet_repo_root: Some(&fleet_repo_root), register_root: &register_root, armada_root: None };

    let preflight = FakePreflight::ok();
    let interview = FakeInterview::err("must not run -- --document PATH was given");
    let staging_check = FakeStagingCheck::ok();
    // **No disk, and no real card**: this stands in for `write_image::run`
    // having already written and BLAKE3-verified one — the outcome this
    // module reacts to, not the mechanism, which `write_image_tests.rs`
    // proves on its own scratch files.
    let writer = FakeWriter::ok("wrote and verified it");
    let pin = [0xddu8; 32];

    let mut args = empty_args();
    args.document_path = Some(doc_path.clone());
    args.device_path = Some(PathBuf::from("scratch-device-stand-in"));

    let report = run_with(
        false,
        &preflight,
        &interview,
        &staging_check,
        &recorder,
        &writer,
        &FakeImageSelector::ok(),
        None,
        args,
        Some(pin),
        None,
        None,
        &FakeExistingDocument::never())
    .expect("every fake answers Ok, and the real recorder succeeds against scratch paths");

    assert!(report.contains("wrote and verified it"), "{report}");
    assert!(!interview.called.get(), "--document PATH must skip the interview entirely");

    // The document is committed to the SCRATCH fleet repository.
    let committed = fleet_repo_root
        .join(rivetos_fleet_repo::DOCUMENTS_DIR)
        .join("example-server.provisioning.toml");
    assert_eq!(std::fs::read_to_string(&committed).expect("committed"), FIXTURE_SERVER_DOCUMENT);

    // The register — the SCRATCH one — holds the enrolled device, with the
    // pin-derived id, the role, the listener and the one pin a card honours.
    let id = rivet_core::identifier::from_entropy(&pin);
    let register = rivetos_register::store::load(&register_root).expect("readable");
    let device = register.devices.get(&id).expect("enrolled");
    assert_eq!(device.name, "example-server");
    assert_eq!(device.fleet_role, Some(rivetos_register::FleetRole::Server));
    assert_eq!(device.listen.as_deref(), Some("0.0.0.0:7777"));
    assert_eq!(device.carry.get("rivet-fleetd").map(String::as_str), Some("0.1.86"));
    assert_eq!(device.carry.get("rivet-proxyd"), None, "{:?}", device.carry);

    // **Item 5: the summary, appended after the write's own report.** Every
    // fact this ordering promises — hostname, role, every carried product, the
    // register id `plan` just enrolled, the device path, and the explicit
    // "unproven" line naming the proof command — must be there, and "works"
    // must never be.
    assert!(report.contains("--- summary ---"), "{report}");
    assert!(report.contains("hostname        example-server"), "{report}");
    assert!(report.contains("role            server"), "{report}");
    assert!(report.contains("    rivet-fleetd 0.1.86\n"), "{report}");
    assert!(!report.contains("    rivet-proxyd 0.1.86\n"), "the proxy is not carried: {report}");
    assert!(report.contains("not carried: rivet-proxyd 0.1.86 is pinned"), "{report}");
    assert!(report.contains(&format!("register id     {id}")), "{report}");
    assert!(report.contains("scratch-device-stand-in"), "{report}");
    assert!(report.contains("UNPROVEN"), "{report}");
    assert!(report.contains("get status"), "{report}");
    assert!(!report.to_lowercase().contains("it works"), "{report}");
    // **The next steps a server still needs** (`ADR-054` §7, R-182/R-184).
    // This fixture predates `[repos.fleet]`, so the push is named as refused.
    assert!(report.contains("NEXT, for a fleet server"), "{report}");
    assert!(report.contains("DECLARES NO [repos.fleet]"), "{report}");
    assert!(report.contains("cargo rivet fleet sync example-server confirm"), "{report}");
}

/// **R-184 D3's declaration, read back by the summary.** A server document
/// that declares `[repos.fleet]` is told to push its text and then to sync its
/// objects; a member is told neither.
#[test]
fn the_summary_names_the_push_for_a_declared_server_and_nothing_for_a_member() {
    let scratch = Scratch::new("summary-next");
    let declared = format!("{FIXTURE_SERVER_DOCUMENT}\n[repos.fleet]\nid = \"0123456789abcdef\"\n");
    let server = write_document_fixture(&scratch, "declared.provisioning.toml", &declared);
    let summary = summarize(&server, None, Some(Path::new("dev")), Some("0011223344556677"));
    assert!(summary.contains("cargo rivet fleet push example-server confirm"), "{summary}");
    assert!(summary.contains("`fleet` repository over the control plane"), "{summary}");
    assert!(!summary.contains("DECLARES NO"), "{summary}");
    assert!(summary.contains("its objects   NOT carried on this card"), "{summary}");
    assert!(summary.contains("cargo rivet fleet sync example-server confirm"), "{summary}");
    assert!(!summary.contains("not built"), "{summary}");
    // Push, then sync: the order they run in.
    let push_at = summary.find("fleet push example-server confirm").expect("push");
    let sync_at = summary.find("fleet sync example-server confirm").expect("sync");
    assert!(push_at < sync_at, "{summary}");

    let member_text = "version = 1\nhostname = \"example-member\"\n\n[network.eth0]\nmethod = \"dhcp\"\n\n\
                       [control-plane]\nlisten = \"0.0.0.0:7777\"\n\n[applications.rivet-proxyd]\n\
                       upstream-name = \"example-member.local\"\n\n[services.rivet-proxyd]\nenabled = true\n";
    let member = write_document_fixture(&scratch, "member.provisioning.toml", member_text);
    let summary = summarize(&member, None, Some(Path::new("dev")), Some("0011223344556677"));
    assert!(summary.contains("role            member"), "{summary}");
    assert!(!summary.contains("NEXT, for a fleet server"), "{summary}");
    assert!(!summary.contains("fleet push"), "{summary}");
}

/// **A member recorded before `ADR-060` carries nothing, and its summary says
/// so.** dev-01's recorded document pinned `rivet-proxyd`; the carry step
/// skipped it, and the summary listed it as carried anyway. It now lists
/// nothing carried and names the pin as one no card honours.
#[test]
fn a_legacy_proxy_pin_is_named_as_not_carried_in_the_summary() {
    let scratch = Scratch::new("summary-legacy-pin");
    let member = write_document_fixture(&scratch, "legacy.provisioning.toml", FIXTURE_MEMBER_DOCUMENT);
    let summary = summarize(&member, None, Some(Path::new("dev")), None);
    assert!(summary.contains("  carried\n    (nothing)\n"), "{summary}");
    assert!(summary.contains("not carried: rivet-proxyd 0.1.86 is pinned"), "{summary}");
    assert!(!summary.contains("    rivet-proxyd 0.1.86\n"), "{summary}");
}

/// **A run that only prepares an image — no `--device`, no `confirm` — gets
/// no summary at all.** Nothing was written to a device, so item 5's own
/// promise ("printed after a verified write") does not apply; the report is
/// exactly the writer's own.
#[test]
fn a_prepared_but_unwritten_image_gets_no_summary() {
    let scratch = Scratch::new("e2e-no-summary");
    let doc_path = write_document_fixture(&scratch, "example-server.provisioning.toml", FIXTURE_SERVER_DOCUMENT);
    let fleet_repo_root = scratch.0.join("fleet-repo");
    let register_root = scratch.0.join("register");
    let recorder =
        RealFleetRecorder { fleet_repo_root: Some(&fleet_repo_root), register_root: &register_root, armada_root: None };
    let preflight = FakePreflight::ok();
    let interview = FakeInterview::err("must not run -- --document PATH was given");
    let staging_check = FakeStagingCheck::ok();
    let writer = FakeWriter::nothing_written("the image is ready");

    let mut args = empty_args();
    args.document_path = Some(doc_path.clone());
    // Deliberately neither `--device` nor `confirm`.

    let report = run_with(
        false, &preflight, &interview, &staging_check, &recorder, &writer, &FakeImageSelector::ok(),
        None, args, Some([0xffu8; 32]), None, None,
        &FakeExistingDocument::never())
    .expect("every fake answers Ok");

    assert!(!report.contains("--- summary ---"), "{report}");
}

/// The same route, run a second time with the SAME pin, is refused before
/// anything is written — the duplicate check, proven end to end rather than
/// only against `RealFleetRecorder::plan` directly.
#[test]
fn scratch_end_to_end_a_second_run_with_the_same_pin_is_refused_before_the_write() {
    let scratch = Scratch::new("e2e-duplicate");
    let doc_path = write_document_fixture(&scratch, "example-server.provisioning.toml", FIXTURE_SERVER_DOCUMENT);
    let fleet_repo_root = scratch.0.join("fleet-repo");
    let register_root = scratch.0.join("register");
    let pin = [0xeeu8; 32];

    let run_once = || {
        let recorder =
            RealFleetRecorder { fleet_repo_root: Some(&fleet_repo_root), register_root: &register_root, armada_root: None };
        let preflight = FakePreflight::ok();
        let interview = FakeInterview::err("must not run");
        let staging_check = FakeStagingCheck::ok();
        let writer = FakeWriter::ok("wrote and verified it");
        let mut args = empty_args();
        args.document_path = Some(doc_path.clone());
        args.confirm = true;
        run_with(
            false, &preflight, &interview, &staging_check, &recorder, &writer, &FakeImageSelector::ok(),
            None, args, Some(pin),
        None, None,
            &FakeExistingDocument::never())
    };

    run_once().expect("the first run enrolls cleanly");
    let why = run_once().expect_err("the second run is a duplicate");
    assert!(why.starts_with("NOTHING WAS WRITTEN"), "{why}");
}

// ---------------------------------------------------------------------------
// R-178 items 1-7 — RealImageSelector, against real scratch fleet
// repositories. Fixture card images are built the exact way
// `image_tests.rs`'s own end-to-end test builds one (a real GPT+FAT image,
// real zstd, real BLAKE3, real Ed25519) — reused via `crate::image::tests`
// rather than re-implemented a second time.
// ---------------------------------------------------------------------------

/// Publishes one real fixture `card` artifact into `fleet_root`, signed by
/// `seed`, and returns the version published — the same steps
/// `image_tests.rs`'s own end-to-end test performs, reused here rather than
/// re-implemented.
fn publish_fixture_card(work: &Scratch, fleet_root: &Path, seed: &Path, version: &str, schema: i64) {
    let image = crate::image::tests::fixture_image(&format!("selector-{version}"), Some(schema), b"tail-bytes");
    crate::image::run(&[
        "--img".into(),
        image.display().to_string(),
        "--version".into(),
        version.into(),
        "--arch".into(),
        "aarch64".into(),
        "--signing-key".into(),
        seed.display().to_string(),
        "--out-dir".into(),
        work.0.join("out").display().to_string(),
        "--at".into(),
        fleet_root.display().to_string(),
    ])
    .expect("publish a fixture card");
}

/// A scratch `~/.rivet`-shaped directory carrying `keys/rivet-system.pub` —
/// item 6's trust anchor — matching the seed [`publish_fixture_card`] signed
/// with.
fn scratch_rivet_root_with_system_key(work: &Scratch, seed_bytes: [u8; 32]) -> PathBuf {
    let at = work.0.join("home");
    std::fs::create_dir_all(at.join("keys")).expect("keys dir");
    let public = rivet_crypto::Ed25519Signer::from_seed(&seed_bytes).public_key();
    std::fs::write(at.join("keys").join("rivet-system.pub"), public.0).expect("write the public key");
    at
}

#[test]
fn real_image_selector_uses_a_literal_existing_file_without_touching_the_repository() {
    let scratch = Scratch::new("literal-file");
    let real_file = scratch.0.join("hand-built.img");
    std::fs::write(&real_file, b"not a real image, just a file").expect("write the fixture");

    // `fleet_repo_root: None` would refuse if this were ever consulted --
    // proving the literal-path branch never reaches the repository at all.
    let selector = RealImageSelector { fleet_repo_root: None, at: Path::new("/nowhere"), is_terminal: false, stages: None };
    let selected = selector
        .select(None, Some(&real_file))
        .expect("a real file needs no repository")
        .expect("a literal file is never an exit");
    assert_eq!(selected.path, real_file);
    assert_eq!(selected.scratch, None, "a literal --image PATH is never a scratch copy");
}

#[test]
fn real_image_selector_refuses_with_no_fleet_repository_root_and_no_literal_file() {
    let selector = RealImageSelector { fleet_repo_root: None, at: Path::new("/nowhere"), is_terminal: false, stages: None };
    let err = selector.select(None, None).expect_err("no repository and no literal file");
    assert!(err.contains("HOME"), "{err}");
    assert!(err.contains("USERPROFILE"), "{err}");
}

/// **Item 1, declined route.** No terminal to ask at, and nothing at the
/// target path yet -- `ensure_fleet_repository` continues silently rather
/// than creating anything, and the empty repository then produces R-176
/// Q8's own refusal, verbatim.
#[test]
fn real_image_selector_creates_nothing_with_no_terminal_and_refuses_q8_verbatim() {
    let scratch = Scratch::new("no-repo-no-terminal");
    let fleet_root = scratch.0.join("fleet");
    let at = scratch.0.join("home");

    let selector = RealImageSelector { fleet_repo_root: Some(&fleet_root), at: &at, is_terminal: false, stages: None };
    let err = selector.select(None, None).expect_err("nothing was ever published");
    assert_eq!(err, crate::provision_image::NO_IMAGES_REFUSAL);
    assert!(!fleet_root.exists(), "nothing must be created when there is no terminal to ask at");
}

/// **Items 2/3/6, the ordinary case.** One platform, one selectable image --
/// no terminal needed at all -- verified and decompressed back to the
/// original bytes.
#[test]
fn real_image_selector_selects_the_only_image_with_no_terminal_and_materialises_it() {
    let scratch = Scratch::new("single-image");
    let fleet_root = scratch.0.join("fleet");
    let seed_bytes = [0xABu8; 32];
    let seed = crate::image::tests::write_seed(&scratch.0, "rivet-system.seed", 0xAB);
    publish_fixture_card(&scratch, &fleet_root, &seed, "0.1.0", 1);
    let at = scratch_rivet_root_with_system_key(&scratch, seed_bytes);

    let selector = RealImageSelector { fleet_repo_root: Some(&fleet_root), at: &at, is_terminal: false, stages: None };
    let selected = selector
        .select(None, None)
        .expect("one platform, one selectable image")
        .expect("no terminal, so no menu to exit from");
    assert!(selected.path.is_file());
    assert!(selected.scratch.is_some(), "a repository-selected image is a scratch copy");
}

/// **Item 4, R-178 P2.** Two images for the one platform, one schema-blocked
/// -- only the matching one is selectable, so no terminal is needed even
/// though the repository holds more than one image for the platform.
#[test]
fn real_image_selector_skips_a_schema_mismatched_image_and_selects_the_matching_one() {
    let scratch = Scratch::new("schema-skip");
    let fleet_root = scratch.0.join("fleet");
    let seed_bytes = [0xCDu8; 32];
    let seed = crate::image::tests::write_seed(&scratch.0, "rivet-system.seed", 0xCD);
    publish_fixture_card(&scratch, &fleet_root, &seed, "0.1.0", 2); // wrong schema
    publish_fixture_card(&scratch, &fleet_root, &seed, "0.1.1", 1); // matches
    let at = scratch_rivet_root_with_system_key(&scratch, seed_bytes);

    let selector = RealImageSelector { fleet_repo_root: Some(&fleet_root), at: &at, is_terminal: false, stages: None };
    let selected = selector
        .select(None, None)
        .expect("exactly one selectable image")
        .expect("no terminal, so no menu to exit from");
    // The materialised bytes must be `0.1.1`'s -- the schema-matching one --
    // proven by decompressing and checking the fixture tail this version's
    // own image was built with is present.
    let bytes = std::fs::read(&selected.path).expect("read the materialised image");
    assert!(bytes.windows(b"tail-bytes".len()).any(|w| w == b"tail-bytes"));
}

/// **Item 4: none selectable is a refusal by name, before the interview.**
#[test]
fn real_image_selector_refuses_by_name_when_nothing_is_selectable() {
    let scratch = Scratch::new("none-selectable");
    let fleet_root = scratch.0.join("fleet");
    let seed = crate::image::tests::write_seed(&scratch.0, "rivet-system.seed", 0xEE);
    publish_fixture_card(&scratch, &fleet_root, &seed, "0.1.0", 2);
    publish_fixture_card(&scratch, &fleet_root, &seed, "0.1.1", 3);
    let at = scratch.0.join("home");

    let selector = RealImageSelector { fleet_repo_root: Some(&fleet_root), at: &at, is_terminal: false, stages: None };
    let err = selector.select(None, None).expect_err("both images are schema-mismatched");
    assert!(err.contains("rpi4"), "{err}");
    assert!(err.contains("needs schema 2"), "{err}");
    assert!(err.contains("needs schema 3"), "{err}");
}

/// **Item 7, digest half.** A real digest prefix resolves, is checked
/// against the platform and schema exactly like a menu pick, and is
/// materialised -- no terminal needed at all.
#[test]
fn real_image_selector_resolves_a_digest_prefix_and_materialises_it() {
    let scratch = Scratch::new("digest-resolve");
    let fleet_root = scratch.0.join("fleet");
    let seed_bytes = [0x12u8; 32];
    let seed = crate::image::tests::write_seed(&scratch.0, "rivet-system.seed", 0x12);
    publish_fixture_card(&scratch, &fleet_root, &seed, "0.1.0", 1);
    let at = scratch_rivet_root_with_system_key(&scratch, seed_bytes);

    let images = crate::fleet::list_images(&fleet_root, None).expect("list");
    let prefix = images[0].digest[..12].to_owned();

    let selector = RealImageSelector { fleet_repo_root: Some(&fleet_root), at: &at, is_terminal: false, stages: None };
    let selected = selector
        .select(None, Some(Path::new(&prefix)))
        .expect("the digest resolves")
        .expect("a digest resolution is never an exit");
    assert!(selected.path.is_file());
    assert!(selected.scratch.is_some());
}

/// An unresolvable digest is refused naming what the repository actually
/// holds -- `fleet::resolve_image`'s own wording, surfaced unchanged.
#[test]
fn real_image_selector_refuses_a_digest_that_matches_nothing() {
    let scratch = Scratch::new("digest-unknown");
    let fleet_root = scratch.0.join("fleet");
    let seed = crate::image::tests::write_seed(&scratch.0, "rivet-system.seed", 0x13);
    publish_fixture_card(&scratch, &fleet_root, &seed, "0.1.0", 1);
    let at = scratch.0.join("home");

    let selector = RealImageSelector { fleet_repo_root: Some(&fleet_root), at: &at, is_terminal: false, stages: None };
    let err = selector.select(None, Some(Path::new("deadbeef"))).expect_err("nothing starts with deadbeef");
    assert!(err.contains("no artifact"), "{err}");
}

/// A digest naming a schema-mismatched image is refused by the same reason
/// the menu route would have marked it with.
#[test]
fn real_image_selector_refuses_a_digest_with_a_schema_mismatch() {
    let scratch = Scratch::new("digest-schema");
    let fleet_root = scratch.0.join("fleet");
    let seed = crate::image::tests::write_seed(&scratch.0, "rivet-system.seed", 0x14);
    publish_fixture_card(&scratch, &fleet_root, &seed, "0.1.0", 2);
    let at = scratch.0.join("home");

    let images = crate::fleet::list_images(&fleet_root, None).expect("list");
    let prefix = images[0].digest[..12].to_owned();

    let selector = RealImageSelector { fleet_repo_root: Some(&fleet_root), at: &at, is_terminal: false, stages: None };
    let err = selector.select(None, Some(Path::new(&prefix))).expect_err("schema 2 vs this tool's 1");
    assert!(err.contains("needs schema 2"), "{err}");
    assert!(err.contains("this tool speaks 1"), "{err}");
}

// ---------------------------------------------------------------------------
// R-178: platform -- more than one platform, a real repository (a
// hand-written manifest, since this build's own `cargo rivet image` has only
// one `BootSlots` implementor, `rpi4`, to publish real bytes against; the
// listed platforms and the schema check are read off the manifest exactly
// the same way regardless of whether real object bytes exist behind them).
// ---------------------------------------------------------------------------

fn write_two_platform_manifest(fleet_root: &Path) {
    std::fs::create_dir_all(fleet_root).expect("fleet dir");
    std::fs::write(
        fleet_root.join("manifest.toml"),
        "\n\
[[artifact]]\n\
digest  = \"aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11\"\n\
product = \"rivet\"\n\
version = \"0.1.0\"\n\
arch    = \"aarch64\"\n\
platform = \"rpi4\"\n\
kind    = \"card\"\n\
size    = 10\n\
added   = \"2026-09-13T00:00:00Z\"\n\
schema_version = 1\n\
\n\
[[artifact]]\n\
digest  = \"bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22\"\n\
product = \"rivet\"\n\
version = \"0.1.0\"\n\
arch    = \"aarch64\"\n\
platform = \"rpi5\"\n\
kind    = \"card\"\n\
size    = 10\n\
added   = \"2026-09-13T00:00:00Z\"\n\
schema_version = 1\n",
    )
    .expect("write the fixture manifest");
}

#[test]
fn real_image_selector_asks_the_platform_when_more_than_one_has_images_and_refuses_with_no_terminal() {
    let scratch = Scratch::new("multi-platform-ask");
    let fleet_root = scratch.0.join("fleet");
    write_two_platform_manifest(&fleet_root);
    let at = scratch.0.join("home");

    let selector = RealImageSelector { fleet_repo_root: Some(&fleet_root), at: &at, is_terminal: false, stages: None };
    let err = selector.select(None, None).expect_err("more than one platform, no terminal to ask at");
    assert!(err.contains("more than one platform"), "{err}");
    assert!(err.contains("rpi4"), "{err}");
    assert!(err.contains("rpi5"), "{err}");
}

/// A requested platform that has images is used without asking -- reaching
/// all the way to item 6's materialisation step (which then fails on this
/// fixture's missing key, proving how far selection itself got).
#[test]
fn real_image_selector_uses_a_requested_platform_without_asking() {
    let scratch = Scratch::new("multi-platform-named");
    let fleet_root = scratch.0.join("fleet");
    write_two_platform_manifest(&fleet_root);
    let at = scratch.0.join("home"); // deliberately no keys/rivet-system.pub

    let selector = RealImageSelector { fleet_repo_root: Some(&fleet_root), at: &at, is_terminal: false, stages: None };
    let err = selector.select(Some("rpi5"), None).expect_err("no system key at this scratch `at`");
    assert!(err.contains("rivet-system.pub"), "{err}: platform selection must have reached materialise");
}

#[test]
fn real_image_selector_refuses_an_unknown_platform_token_naming_what_exists() {
    let scratch = Scratch::new("multi-platform-unknown");
    let fleet_root = scratch.0.join("fleet");
    write_two_platform_manifest(&fleet_root);
    let at = scratch.0.join("home");

    let selector = RealImageSelector { fleet_repo_root: Some(&fleet_root), at: &at, is_terminal: false, stages: None };
    let err = selector.select(Some("uefi"), None).expect_err("uefi has no images");
    assert!(err.contains("uefi"), "{err}");
    assert!(err.contains("rpi4"), "{err}");
    assert!(err.contains("rpi5"), "{err}");
}

/// **Item 7: a digest is checked against `--platform` exactly like a menu
/// pick.** The `bb22…` fixture is `rpi5`; asking for `rpi4` refuses.
#[test]
fn real_image_selector_refuses_a_digest_whose_platform_disagrees_with_requested_platform() {
    let scratch = Scratch::new("digest-platform-mismatch");
    let fleet_root = scratch.0.join("fleet");
    write_two_platform_manifest(&fleet_root);
    let at = scratch.0.join("home");

    let selector = RealImageSelector { fleet_repo_root: Some(&fleet_root), at: &at, is_terminal: false, stages: None };
    let err = selector
        .select(Some("rpi4"), Some(Path::new("bb22bb22bb22")))
        .expect_err("bb22... is rpi5, not rpi4");
    assert!(err.contains("rpi5"), "{err}");
    assert!(err.contains("rpi4"), "{err}");
}

// ---------------------------------------------------------------------------
// Wave C3: the real, non-mocked scratch end-to-end proof — a REAL 2 GiB card
// image out of this checkout's own `out/`, REAL scratch credentials
// (`cargo rivet setup`), a REAL scratch fleet repository published into with
// REAL zstd/BLAKE3/Ed25519 (`cargo rivet image`, `cargo rivet fleet
// publish`'s own verifier), run through `RealWriter` — `write_image::run`
// itself, never a fake — to a scratch image FILE. Never a device: no
// `--device`, no `confirm`, on `AGENTS.md` §9's own rule.
// ---------------------------------------------------------------------------

/// The repository root this crate is built from — `cargo test`'s own CWD is
/// the crate's manifest directory, never the workspace root, so every path
/// below is resolved from here rather than assumed relative.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("the workspace root exists two directories up")
}

/// A real, signed application payload, `carry_tests.rs`'s own `a_payload`
/// restated (`cmd/rivet-image/src/application.rs::tests::a_payload`'s
/// shape) — this module has no access to that one (a different `mod tests`),
/// and `ADR-011`'s "no `[lib]` target" trade applies to test code exactly as
/// it does to the crates themselves.
fn e2e_fixture_application(product: &str, seed: &[u8; 32]) -> Vec<u8> {
    use rivet_core::package::application::{Manifest, ProgramKind, TargetClass, VARIANT_NONE};
    use rivet_core::package::write::{Builder, NewMember};
    use rivet_crypto::{Blake3, Ed25519Signer};

    let manifest = Manifest::new(ProgramKind::Daemon).render().into_bytes();
    let program = b"\x7fELF not really, and nothing here execs it".to_vec();
    let mut digest = Blake3::new();
    let mut signer = Ed25519Signer::from_seed(seed);
    let mut builder =
        Builder::new(product, "1.0.0-e2e", "aarch64", None, rivet_core::package::Kind::application())
            .expect("valid header fields");
    builder
        .add(
            NewMember::stored(
                rivet_core::package::application::MANIFEST_NAME,
                TargetClass::Manifest.as_byte(),
                VARIANT_NONE,
                &manifest,
                &mut digest,
            )
            .expect("a manifest member"),
        )
        .expect("the manifest fits");
    builder
        .add(
            NewMember::stored(product, TargetClass::Program.as_byte(), VARIANT_NONE, &program, &mut digest)
                .expect("a program member"),
        )
        .expect("the program fits");
    builder.seal_in_memory(&mut digest, &mut signer, &[&manifest, &program]).expect("the payload seals")
}

/// **The proof itself.** Fictional values throughout (`AGENTS.md` §9,
/// R-172.1): a made-up hostname and TEST-NET-1 address, a scratch `~/.rivet`
/// this run mints with `cargo rivet setup`, a scratch fleet repository this
/// run publishes a REAL `out/*-card.img` and two real fixture applications
/// into, all signed with scratch seeds `cargo rivet key new` would mint.
///
/// Runs `run_with` with every REAL component (`RealPreflight`,
/// `RealStagingCheck`, `RealFleetRecorder`, `RealImageSelector`,
/// `RealWriter`) except the interview, which `--document PATH` skips
/// regardless — proving the actual `write_image::run` call this module
/// exists to sequence, not a fake standing in for it.
#[test]
fn real_end_to_end_a_real_card_image_is_widened_with_every_carried_file() {
    use std::fs::OpenOptions;
    use std::io::Read as _;

    let img_dir = repo_root().join("out");
    let images = crate::write_image::candidates(
        img_dir.to_str().expect("a valid utf-8 path"),
        "-card.img",
    );
    let Some(real_image) = images.first() else {
        eprintln!("SKIPPED: no *-card.img in {}", img_dir.display());
        return;
    };

    let scratch = Scratch::new("real-e2e");
    let at = scratch.0.join("home");
    crate::setup::run(&crate::setup::SetupArgs {
        at: Some(at.clone()),
        name: Some("e2e-operator".to_owned()),
        check: false,
        over: Overrides::default(),
    })
    .expect("a cold setup run mints every required credential");

    let fleet_root = scratch.0.join("fleet");
    crate::fleet::init(&fleet_root).expect("a fresh fleet repository");

    // --- publish the REAL card image, signed with the scratch SYSTEM seed --
    let system_seed = at.join("keys").join("rivet-system.seed");
    crate::image::run(&[
        "--img".into(),
        real_image.display().to_string(),
        "--version".into(),
        "1.0.0-e2e".into(),
        "--arch".into(),
        "aarch64".into(),
        "--platform".into(),
        "rpi4".into(),
        "--signing-key".into(),
        system_seed.display().to_string(),
        "--out-dir".into(),
        scratch.0.join("card-out").display().to_string(),
        "--at".into(),
        fleet_root.display().to_string(),
    ])
    .unwrap_or_else(|e| panic!("publishing the real card failed: {e}"));

    // --- publish the fixture application a server carries, signed with the
    //     scratch APP seed. Not the proxy: the image provides it (`ADR-060`).
    let app_seed_bytes = crate::key::read_32(&at.join("keys").join("rivet-app.seed"), "app seed")
        .expect("setup minted an app seed");
    let bytes = e2e_fixture_application("rivet-fleetd", &app_seed_bytes);
    let rvt_path = scratch.0.join("rivet-fleetd.rvt");
    std::fs::write(&rvt_path, &bytes).expect("write the fixture .rvt");
    crate::fleet::publish(&fleet_root, &rvt_path, &at.join("keys").join("rivet-app.pub"))
        .unwrap_or_else(|e| panic!("publishing rivet-fleetd failed: {e}"));

    // --- the fictional server document, staged, pinned to what was just
    //     published --------------------------------------------------------
    let doc_text = "\
version = 1
hostname = \"e2e-example-server\"

[network.eth0]
method = \"static\"
address = \"192.0.2.10/24\"

[control-plane]
listen = \"0.0.0.0:7777\"

[applications.rivet-proxyd]
upstream-name = \"e2e-example-server.local\"

[services.rivet-fleetd]
enabled = true

[services.rivet-proxyd]
enabled = true

[carry.rivet-fleetd]
version = \"1.0.0-e2e\"
";
    let doc_path = write_document_fixture(&scratch, "e2e-example-server.provisioning.toml", doc_text);
    let staging_path = staging_path_beside(&doc_path).expect("a .provisioning.toml path");
    std::fs::write(&staging_path, rivet_core::staging::render(&[])).expect("an empty staging bundle");

    // --- run it: preflight, REAL image selection (decompress + verify a
    //     real 2 GiB image), the staging check, injection, and stop at "the
    //     image is ready" -- no --device, no confirm, never a device -------
    let preflight = RealPreflight { at: &at };
    let staging_check = RealStagingCheck { staging_override: None, at: &at, builder: None };
    let register_root = scratch.0.join("register");
    let fleet_recorder = RealFleetRecorder { fleet_repo_root: Some(&fleet_root), register_root: &register_root, armada_root: None };
    let interview = FakeInterview::err("must not run -- --document PATH was given");
    let image_selector =
        RealImageSelector { fleet_repo_root: Some(&fleet_root), at: &at, is_terminal: false, stages: None };

    let mut args = empty_args();
    args.document_path = Some(doc_path.clone());
    args.at = Some(at.clone());
    // **Never the real default.** `provision::run` always sets this on
    // `ProvisionArgs` itself before calling `run_with` (see that function's
    // own body) — done by hand here because this test calls `run_with`
    // directly, skipping `run`'s own resolution entirely.
    args.fleet_repo_root = Some(fleet_root.clone());
    // The REUSED real `out/*-card.img` carries the maintainer's own real
    // `peers` file, naming his real operator key — never this run's scratch
    // one. This test is about the widened injection, not about reaching the
    // card, so it takes the same escape hatch a genuinely shared image would
    // need (`write_image::run`'s own `--not-mine`).
    args.not_mine = true;

    // **Called one layer below `run_with`, on purpose.** `run_with` itself
    // removes the scratch working copy the instant it returns — correct
    // behaviour for every other caller, and exactly what stands between this
    // test and reading the prepared image back. Selecting here and calling
    // `run_with_image_selected` directly is the identical sequence `run_with`
    // performs, minus that one cleanup, so this test can look before it runs.
    let selected = image_selector
        .select(Some("rpi4"), None)
        .unwrap_or_else(|e| panic!("image selection failed: {e}"))
        .expect("no terminal, so no menu to exit from");
    args.image_path = Some(selected.path.clone());

    let report = run_with_image_selected(
        false,
        &preflight,
        &interview,
        &staging_check,
        &fleet_recorder,
        &RealWriter,
        args,
        Some([0x5eu8; 32]),
        None,
        None,
        &FakeExistingDocument::never(),
        selected.descriptor.clone(),
        None,
        selected.version.as_deref(),
    )
    .unwrap_or_else(|e| panic!("the real end-to-end run failed: {e}"));

    assert!(report.contains("the image is ready"), "{report}");
    assert!(!report.contains("--- summary ---"), "no device was targeted: {report}");

    // --- read the WRITTEN, WIDENED image's boot partitions back
    //     independently -- a Rust FAT reader, never trusting `report`'s own
    //     words for what actually landed -----------------------------------
    let after_prefix = report
        .split("the image is ready at ")
        .nth(1)
        .expect("the report names the prepared image's path");
    let path_text = after_prefix
        .split_once(". Nothing was written")
        .map_or(after_prefix, |(path, _)| path);
    let prepared = PathBuf::from(path_text);
    assert!(prepared.is_file(), "{}", prepared.display());

    let mut file = OpenOptions::new().read(true).write(false).open(&prepared).expect("reopen the prepared image");
    let parts = crate::gpt::read(&mut file).expect("a readable GPT");
    let mut found: Vec<(String, String, u64)> = Vec::new(); // (label, name, size)
    for label in write_image::BOOT_LABELS {
        let part = crate::gpt::by_label(&parts, label).expect("the label exists");
        let mut view = write_image::Partition::new(&mut file, part.offset_bytes(), part.len_bytes());
        let fs = fatfs::FileSystem::new(&mut view, fatfs::FsOptions::new()).expect("a filesystem");
        for entry in fs.root_dir().iter() {
            let entry = entry.expect("a directory entry");
            if entry.is_file() {
                found.push((label.to_owned(), entry.file_name(), entry.len()));
            }
        }
    }

    for label in write_image::BOOT_LABELS {
        let names: Vec<&str> =
            found.iter().filter(|(l, ..)| l == label).map(|(_, n, _)| n.as_str()).collect();
        for expected in [
            "rivet.toml",
            "staged.txt",
            "device.seed",
            "device.crt",
            "peers",
            "app-rivet-fleetd.rvt",
        ] {
            assert!(names.contains(&expected), "{label} is missing {expected}: {names:?}");
        }
        assert!(
            !names.contains(&"app-rivet-proxyd.rvt"),
            "{label} carries the proxy, which the image provides (ADR-060): {names:?}"
        );
    }

    // Bytes, not just names: the carried application on the card is exactly
    // what was published, unchanged by the trip through the fleet
    // repository and the injection.
    let mut file = OpenOptions::new().read(true).write(false).open(&prepared).expect("reopen");
    let parts = crate::gpt::read(&mut file).expect("a readable GPT");
    let part = crate::gpt::by_label(&parts, write_image::BOOT_LABELS[0]).expect("boot-a");
    let mut view = write_image::Partition::new(&mut file, part.offset_bytes(), part.len_bytes());
    let fs = fatfs::FileSystem::new(&mut view, fatfs::FsOptions::new()).expect("a filesystem");
    let mut on_card = Vec::new();
    fs.root_dir()
        .open_file("app-rivet-fleetd.rvt")
        .expect("the carried payload")
        .read_to_end(&mut on_card)
        .expect("read it back");
    assert_eq!(on_card, e2e_fixture_application("rivet-fleetd", &app_seed_bytes));

    eprintln!(
        "real end-to-end: {} boot-partition files on each of boot-a/boot-b, prepared at {}",
        found.len() / 2,
        prepared.display(),
    );
}

/// **R-178 item 1 — the C3 byte-for-byte proof against `tools/container/build-card.sh`.**
///
/// `real_end_to_end` above proves the widened injection against a REUSED
/// card this checkout happened to have in `out/`. It does not prove the
/// bytes `write_image::inject` places agree with what a FRESH build of
/// `cmd/rivet-image` would place for the identical inputs — that gap is
/// this wave's own closing step (`ADR-054` §8).
///
/// This is NOT that whole proof by itself — a Rust test cannot shell out to
/// `docker` and stay something `cargo test` can run everywhere this suite
/// runs. What it IS: the half of the proof that has to run inside this
/// process, because `run_with` deletes its own scratch working copy the
/// instant it returns (`run_with`'s own doc comment) and nothing outside
/// this process can read the widened image before that happens.
///
/// So this calls [`run_with_image_selected`] one layer below `run_with`,
/// exactly as `real_end_to_end` does, against fixtures a human operator
/// (or a shell session driving this same binary) built first with the real
/// CLI: `cargo rivet setup`, `cargo rivet fleet init`, `cargo rivet image`
/// publishing a GENERIC card `tools/container/build-card.sh` built with no
/// `RIVET_DOCUMENT`, and `cargo rivet fleet publish` for two fictional
/// `rivet-fleetd`/`rivet-proxyd` applications. It copies the widened image
/// out to a STABLE path before returning, so the byte diff against a
/// second, independent `tools/container/build-card.sh` run (fed the
/// widened `staged.txt` this run produces, as `RIVET_STAGING`) can happen
/// entirely outside this process, on both platforms this repository
/// actually ships to.
///
/// **Gated on `RIVET_C3_PROOF_AT` rather than run by default** — unlike
/// `real_end_to_end`'s graceful skip on an absent `out/*.img`, this needs a
/// whole scratch fleet repository and two published fictional applications
/// nothing here can conjure from inside `cargo test`; building those is the
/// external procedure `docs/ADR-054-Provisioning-Selects-It-Does-Not-Build.md`
/// §8 and this wave's own closing evidence describe. Point it at the `--at`
/// directory `cargo rivet setup` wrote (`RIVET_C3_PROOF_FLEET` for the fleet
/// repository beside it, `RIVET_C3_PROOF_DOC` for the `.provisioning.toml`,
/// `RIVET_C3_PROOF_OUT` for where to copy the widened image) and run:
///
/// ```text
/// RIVET_C3_PROOF_AT=out/c3-byte-proof/home \
/// RIVET_C3_PROOF_FLEET=out/c3-byte-proof/fleet \
/// RIVET_C3_PROOF_DOC=out/c3-byte-proof/doc/example-c3-server.provisioning.toml \
/// RIVET_C3_PROOF_OUT=out/c3-byte-proof/cardB-provisioned/card.img \
///     cargo test -p cargo-rivet --bin cargo-rivet c3_byte_proof -- --nocapture
/// ```
#[test]
fn c3_byte_proof_widen_a_selected_image_and_copy_it_out_for_external_comparison() {
    use std::io::Read as _;

    let Some(at) = std::env::var_os("RIVET_C3_PROOF_AT") else {
        eprintln!(
            "SKIPPED: RIVET_C3_PROOF_AT is not set. See this test's own doc comment for how to \
             run the C3 byte-for-byte proof (R-178 item 1) — it needs a scratch fleet repository \
             this process cannot build for itself."
        );
        return;
    };
    let at = PathBuf::from(at);
    let fleet_root = std::env::var_os("RIVET_C3_PROOF_FLEET")
        .map(PathBuf::from)
        .unwrap_or_else(|| at.parent().expect("a parent").join("fleet"));
    let doc_path = std::env::var_os("RIVET_C3_PROOF_DOC")
        .map(PathBuf::from)
        .expect("RIVET_C3_PROOF_DOC — the fictional server document beside its staging bundle");
    let out_path = std::env::var_os("RIVET_C3_PROOF_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| at.parent().expect("a parent").join("cardB-provisioned/card.img"));

    let image_selector =
        RealImageSelector { fleet_repo_root: Some(&fleet_root), at: &at, is_terminal: false, stages: None };
    let selected = image_selector
        .select(Some("rpi4"), None)
        .unwrap_or_else(|e| panic!("image selection against {} failed: {e}", fleet_root.display()))
        .expect("no terminal, so no menu to exit from");

    let preflight = RealPreflight { at: &at };
    let staging_check = RealStagingCheck { staging_override: None, at: &at, builder: None };
    let register_root = at.join("register");
    let fleet_recorder = RealFleetRecorder { fleet_repo_root: Some(&fleet_root), register_root: &register_root, armada_root: None };
    let interview = FakeInterview::err("must not run -- --document PATH was given");

    let mut args = empty_args();
    args.document_path = Some(doc_path.clone());
    args.at = Some(at.clone());
    args.fleet_repo_root = Some(fleet_root.clone());
    args.image_path = Some(selected.path.clone());
    // The generic image's own `peers` file is absent by construction — it was
    // built with no `RIVET_IDENTITY` on purpose, so the SAME image can be
    // published once and widened for any number of fictional documents. This
    // run's identity (and so its `peers`) comes from `crate::carry::gather`
    // reading `at`'s own `identity/board`, exactly as `real_end_to_end` above.
    args.not_mine = true;

    let report = run_with_image_selected(
        false,
        &preflight,
        &interview,
        &staging_check,
        &fleet_recorder,
        &RealWriter,
        args,
        Some([0x5eu8; 32]),
        None,
        None,
        &FakeExistingDocument::never(),
        selected.descriptor.clone(),
        None,
        selected.version.as_deref(),
    )
    .unwrap_or_else(|e| panic!("the provisioning run failed: {e}"));

    assert!(report.contains("the image is ready"), "{report}");
    let after_prefix = report
        .split("the image is ready at ")
        .nth(1)
        .expect("the report names the prepared image's path");
    let path_text = after_prefix
        .split_once(". Nothing was written")
        .map_or(after_prefix, |(path, _)| path);
    let prepared = PathBuf::from(path_text);
    assert!(prepared.is_file(), "{}", prepared.display());

    // **Copy it out before this test function returns.** `run_with` — never
    // called here — is the only thing that deletes this scratch copy;
    // calling `run_with_image_selected` directly, exactly as `real_end_to_end`
    // does, leaves it on disk. The copy is what survives past `cargo test`'s
    // own exit for the external byte diff to read.
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent).expect("create the destination directory");
    }
    std::fs::copy(&prepared, &out_path)
        .unwrap_or_else(|e| panic!("copy {} to {}: {e}", prepared.display(), out_path.display()));

    let mut file = std::fs::File::open(&out_path).expect("reopen the copy");
    let parts = crate::gpt::read(&mut file).expect("a readable GPT");
    let mut found: Vec<(String, String, u64)> = Vec::new();
    for label in write_image::BOOT_LABELS {
        let part = crate::gpt::by_label(&parts, label).expect("the label exists");
        let mut view = write_image::Partition::new(&mut file, part.offset_bytes(), part.len_bytes());
        let fs = fatfs::FileSystem::new(&mut view, fatfs::FsOptions::new()).expect("a filesystem");
        for entry in fs.root_dir().iter() {
            let entry = entry.expect("a directory entry");
            if entry.is_file() {
                found.push((label.to_owned(), entry.file_name(), entry.len()));
            }
        }
    }
    found.sort();
    eprintln!(
        "c3 byte proof: widened image copied to {} ({} bytes)",
        out_path.display(),
        std::fs::metadata(&out_path).expect("stat the copy").len(),
    );
    for (label, name, len) in &found {
        eprintln!("  {label:6} {name:24} {len} bytes");
    }

    // Extract the widened staged.txt too, beside the image, so an external
    // `tools/container/build-card.sh` run can feed it back in as
    // `RIVET_STAGING` and produce a byte-comparable fresh build.
    let staged_out = out_path.with_file_name("card.staged.txt");
    let mut file = std::fs::File::open(&out_path).expect("reopen the copy");
    let parts = crate::gpt::read(&mut file).expect("a readable GPT");
    let part = crate::gpt::by_label(&parts, write_image::BOOT_LABELS[0]).expect("boot-a");
    let mut view = write_image::Partition::new(&mut file, part.offset_bytes(), part.len_bytes());
    let fs = fatfs::FileSystem::new(&mut view, fatfs::FsOptions::new()).expect("a filesystem");
    let mut staged_bytes = Vec::new();
    fs.root_dir()
        .open_file("staged.txt")
        .expect("staged.txt is on the card")
        .read_to_end(&mut staged_bytes)
        .expect("read it back");
    std::fs::write(&staged_out, &staged_bytes).expect("write the extracted staged.txt");
    eprintln!("c3 byte proof: widened staged.txt extracted to {}", staged_out.display());
}

/// Every file on one boot partition — `(name, bytes)`, sorted by name so two
/// listings compare regardless of directory-entry order.
fn boot_partition_files(image: &Path, label: &str) -> Vec<(String, Vec<u8>)> {
    use std::io::Read as _;
    let mut file = std::fs::File::open(image).unwrap_or_else(|e| panic!("open {}: {e}", image.display()));
    let parts = crate::gpt::read(&mut file).expect("a readable GPT");
    let part = crate::gpt::by_label(&parts, label).unwrap_or_else(|e| panic!("{label}: {e}"));
    let mut view = write_image::Partition::new(&mut file, part.offset_bytes(), part.len_bytes());
    let fs = fatfs::FileSystem::new(&mut view, fatfs::FsOptions::new())
        .unwrap_or_else(|e| panic!("{label} is not a readable FAT filesystem: {e}"));
    let mut out = Vec::new();
    for entry in fs.root_dir().iter() {
        let entry = entry.expect("a directory entry");
        if !entry.is_file() {
            continue;
        }
        let mut bytes = Vec::new();
        entry.to_file().read_to_end(&mut bytes).expect("read the file back");
        out.push((entry.file_name(), bytes));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// **R-178 item 1 — the second half of the C3 byte-for-byte proof.**
///
/// Diffs `RIVET_C3_PROOF_CARD_A` (a card `tools/container/build-card.sh`
/// built directly, with `RIVET_DOCUMENT`/`RIVET_STAGING`/`RIVET_IDENTITY`/
/// `RIVET_FLEET_REPOSITORY` given at build time) against
/// `RIVET_C3_PROOF_OUT` (the widened image the test above copied out) —
/// every file on `boot-a` and `boot-b`, both names and bytes. See this
/// module's `c3_byte_proof_widen_a_selected_image_and_copy_it_out_for_external_comparison`
/// doc comment for how `RIVET_STAGING` for Card A has to be the EXACT
/// `card.staged.txt` that test extracts — R-178 P1's repository carry is
/// computed at provisioning time and a fresh build has no other way to
/// reproduce those exact bytes (`ADR-054` §5).
///
/// **Gated on `RIVET_C3_PROOF_CARD_A` rather than run by default** — same
/// reason as the test above: nothing here can run `docker`.
#[test]
fn c3_byte_proof_diff_against_a_fresh_build() {
    let Some(card_a) = std::env::var_os("RIVET_C3_PROOF_CARD_A") else {
        eprintln!(
            "SKIPPED: RIVET_C3_PROOF_CARD_A is not set. See \
             c3_byte_proof_widen_a_selected_image_and_copy_it_out_for_external_comparison's own \
             doc comment for the full procedure."
        );
        return;
    };
    let card_a = PathBuf::from(card_a);
    let card_b = std::env::var_os("RIVET_C3_PROOF_OUT")
        .map(PathBuf::from)
        .expect("RIVET_C3_PROOF_OUT — the widened image the other test copied out");
    assert!(card_a.is_file(), "{}", card_a.display());
    assert!(card_b.is_file(), "{}", card_b.display());

    let mut mismatches = 0usize;
    for label in write_image::BOOT_LABELS {
        let a = boot_partition_files(&card_a, label);
        let b = boot_partition_files(&card_b, label);
        let a_names: Vec<&str> = a.iter().map(|(n, _)| n.as_str()).collect();
        let b_names: Vec<&str> = b.iter().map(|(n, _)| n.as_str()).collect();
        eprintln!("{label}: card A has {} file(s), card B (provisioned) has {} file(s)", a.len(), b.len());
        if a_names != b_names {
            mismatches += 1;
            eprintln!("  NAME MISMATCH\n    A: {a_names:?}\n    B: {b_names:?}");
            continue;
        }
        for ((name, a_bytes), (_, b_bytes)) in a.iter().zip(b.iter()) {
            if a_bytes == b_bytes {
                eprintln!("  {name:24} {:>10} bytes  IDENTICAL", a_bytes.len());
            } else {
                mismatches += 1;
                eprintln!(
                    "  {name:24} A {} bytes, B {} bytes  MISMATCH",
                    a_bytes.len(),
                    b_bytes.len()
                );
                let first_diff = a_bytes.iter().zip(b_bytes.iter()).position(|(x, y)| x != y);
                eprintln!("    first differing byte: {first_diff:?}");
            }
        }
    }
    assert_eq!(mismatches, 0, "{mismatches} boot-partition difference(s) — see the output above");
    eprintln!("c3 byte proof: every file on boot-a and boot-b matches, name and bytes, between {} and {}", card_a.display(), card_b.display());
}
