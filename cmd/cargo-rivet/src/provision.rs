//! `cargo rivet provision` — the one-command orchestrator SPEC-05 asks for:
//! the interview, the document, the image, then the write.
//!
//! `docs/PLAN-Feature-Set-2026-09-12.md` §3.0 and §5.2 item B, `.rulings/`
//! R-162 (Wave 3b), decision 4: without a terminal it names the agent path and
//! exits non-zero.
//!
//! # R-174 was reversed by R-175 and R-176 — what stayed, what did not
//!
//! `.rulings/` R-174 made this module cross-build, sign and publish a role's
//! own `rivet-fleetd`/`rivet-proxyd` from this tree, pin the versions it just
//! built into the document, and build a whole card image, inside every
//! `cargo rivet provision` run (`crate::payloads`, `crate::card_build` — both
//! gone now). `.rulings/` R-175 ruled that shape wrong: software is built and
//! signed once, published into the artifact repository, and provisioning is
//! a consumer of that repository, never a producer — it never compiles,
//! signs, or publishes anything. `.rulings/` R-176 Q11 then ruled the
//! remedy: the build-per-run code is reverted; the fixes made alongside it
//! are kept. This module is that revert.
//!
//! **Selecting a prebuilt, signed image from the artifact repository is
//! R-176's later work** (its Q1–Q3), not this module's. Until it lands, an
//! image still comes from `--image PATH` or the same menu `write-image`
//! itself has always offered (`crate::write_image::run`'s own `out/*-card.img`
//! scan) — exactly the shape this module had before R-174.
//!
//! **What R-174 got right, and what stayed:**
//!
//! * `RealPreflight` still surveys credentials before the interview runs,
//!   rather than after ten minutes of typing. Docker and the fleet repository
//!   are gone from it — this module never builds or publishes anything now,
//!   so it never needs either.
//! * `require_staging` still refuses a server or member document with no
//!   `ADR-049` staging bundle beside it, so a hand-composed `--document PATH`
//!   that skips the interview cannot silently produce a card missing the
//!   credentials its own application service needs. `cmd/rivet-provision`'s
//!   own interview already writes this bundle before this orchestrator ever
//!   sees the document, so the case this actually catches is the
//!   non-interactive one.
//! * `resolve_interview_source` still finds the interview from a real
//!   install rather than only from a `target/debug`-shaped sibling directory.
//! * `crate::progress` still narrates the interview as it runs — eye candy,
//!   never required for acceptance.
//!
//! # What this module is, and what it deliberately is not
//!
//! **It sequences; it does not re-implement.** `cmd/rivet-provision` already
//! builds the interview in full — the menu, `?`, save-and-resume, the
//! CA-derived `[applications.rivet-proxyd] upstream-name`, the staged
//! register — and `write_image::run` (renamed from `provision`, §3.0)
//! already builds the write — choosing or naming an image, the plan,
//! `Write it? [y/N]`, the elevated write, the read-back digest. A second copy
//! of either is exactly the duplication plan §5.2 item B refuses. So this
//! module contributes only the hand-off between them — see `Interview` and
//! `Writer` below — plus the credentials preflight and the staging-bundle
//! refusal, and calls each existing piece exactly once.
//!
//! **The interview runs as a subprocess, not a re-implementation of the
//! menu.** `SubprocessInterview` runs `cmd/rivet-provision`, with this
//! process's own stdio, so an operator sees the identical tool SPEC-05 §4
//! already describes — nothing here draws a screen or reads a keypress. The
//! hand-off is a breadcrumb file, `~/.rivet/provision/.latest-run.txt`, that
//! `cmd/rivet-provision` writes the moment `w` succeeds
//! (`cmd/rivet-provision/src/main.rs`'s `write_document` call site) naming
//! the document it just wrote — the one fact this orchestrator needs, and
//! nothing but the interview knows it.
//!
//! # Finding that binary, and why no checkout is needed to run this verb
//!
//! **`.rulings/` R-191/R-192: a Rust toolchain is a given, a SOURCE CHECKOUT
//! is not.** `provision` was the only verb of the nineteen that needed one at
//! the moment it ran, and it needed it twice over: a gate refused the verb
//! outright unless the working directory held `cmd/rivet-provision/Cargo.toml`
//! — including `--document`, `--existing` and `--verify-only`, which never
//! start an interview — and, having passed it, the interview was rebuilt from
//! source with `cargo run` on every host where no sibling binary existed. The
//! maintainer's first real run met both: refused outside the checkout, then
//! two minutes of compiling once he was inside it.
//!
//! **Both are gone.** The install is two lines rather than one
//! (`INSTALL_BOTH`), so `rivet-provision` sits beside `cargo-rivet` on a real
//! host; `resolve_interview_source` takes the sibling when it is there, falls
//! back to `cargo run` only when the working directory really is a checkout
//! (the developer's route — an edit to `cmd/rivet-provision` takes effect on
//! the very next run, with nothing reinstalled), and otherwise refuses by
//! naming the install command rather than a directory.
//!
//! **A sibling pair is version-checked, and the `cargo run` route is not**
//! (R-191 D3) — `version_refusal`, and `SubprocessInterview::run`'s own
//! comment at the call site for why the asymmetry is right.
//!
//! # Testability, and what is honestly not tested
//!
//! `docs/PLAN-Feature-Set-2026-09-12.md`'s own words: nothing this runs may
//! write a disk, and no test may touch `\\.\PHYSICALDRIVE*`. Tests fake the
//! writer; the orchestrator proves only its sequencing and hand-offs.
//! `run_with` takes the terminal check, `Preflight`, the interview, the
//! staging check and the writer as parameters for exactly that reason —
//! `provision_tests.rs`
//! proves the ORDER (a terminal is checked before the interview runs;
//! `--verify-only` and the two internal `--elevated-*` re-invocations skip
//! preflight and the interview entirely; a document already in hand skips
//! preflight and the interview but is still checked for a required staging
//! bundle; a refusal from preflight, the interview, or the staging check
//! stops the sequence there and the writer is never reached) with fakes that
//! touch no terminal, no subprocess and no disk beyond a scratch directory.
//! `resolve_interview_source`'s own three cases (a sibling; no sibling in a
//! checkout; neither), and `version_refusal`'s wording, are proven with paths
//! that do and do not exist and with version strings — no process at all.
//! **What is NOT proven by a unit test**: that the `cargo run` fallback
//! actually reaches a real `cmd/rivet-provision`, and that the breadcrumb it
//! reads is the one a real interview really wrote. `cmd/cargo-rivet/tests/
//! pty_interview.rs` drives the real binaries under a real terminal for
//! those — including, since R-192, a run whose working directory is not a
//! checkout at all.
//!
//! # R-178 P3/P4/Q7/Q10, added 2026-09-13: version selection, and recording
//! after the write
//!
//! **P3 — the interview picks application versions, not this module.**
//! `cmd/rivet-provision` lists what a role's card must carry
//! (`rivet_config::Document::carried_services`) against the fleet repository
//! and writes `[carry.<product>] version = "…"` into the document itself,
//! before `w` ever hands one back here. This module's only part in it is
//! telling the interview WHICH repository to read from —
//! `RIVET_FLEET_REPOSITORY`, set on `SubprocessInterview`'s child process
//! to the same root this module resolves for itself below, so the two
//! halves of one run never disagree about which repository a run means.
//!
//! **P4/Q10 — recording moves to after the write.** Until this date,
//! `cmd/rivet-provision`'s `w` key committed the composed document into the
//! fleet repository the moment it was written locally — before any card
//! existed, let alone one that had been verified. `.rulings/` R-176 Q10:
//! *"after the write, the final document with its pins is committed to the
//! repository, and the register gains the computer's record."* So that call
//! moved HERE, to `run_with`, gated on `write_image::WriteOutcome::written`
//! — a device actually written and verified, rather than merely "prepared an
//! image". A declined write (`N`, or a cancelled administrator prompt), a
//! failed one, no removable device, or a run that only prepares an image
//! commits nothing.
//!
//! # 2026-09-14: the bare interactive run writes the card
//!
//! `.rulings/` R-174: typing `cargo rivet provision` ends in a provisioned SD
//! card. Joel's first real run did the whole interview and stopped at "the
//! image is ready at …" — a scratch copy removed when the run exited. So a run
//! with a terminal sets `ProvisionArgs::offer_device_menu`: with no `--device`
//! and no `confirm`, the removable devices are offered as a menu, the plan is
//! printed, `Write it? [y/N]` is asked, and only a `y` raises the
//! administrator prompt. `confirm` still means "already confirmed" and skips
//! the question. When nothing is written, the run says so and names the
//! `--document … --identity … --image … confirm` command that writes the same
//! card without the interview. A run with no terminal is unchanged.
//!
//! **Q7 — enrollment.** A server or member's document is also enrolled into
//! the OPERATOR's register (`cmd/rivet-fleet`'s own, resolved the same way
//! [`crate::device::default_register_dir`] already does) — a `Device` record
//! built the way `cmd/rivet-fleet`'s `add_device` builds one, except its
//! identifier is derived from the machine's own control-plane pin
//! (`ADR-049` §8.1, `rivet_core::identifier::from_entropy`) rather than
//! minted at random, so re-provisioning the same board produces the same
//! record rather than a second one. A stand-alone document enrolls nothing.
//! **The duplicate check runs BEFORE the write**, not after — an id or a
//! hostname already in the register is a named refusal with nothing written,
//! exactly as every other refusal in this module is.
//!
//! **Where the pin comes from.** This module never reads a card's identity
//! directory itself — "Identity" lives only inside the interview's own
//! fields, a workstation fact the composed document never carries. So
//! `cmd/rivet-provision` reads it once, at `w`, and passes the hex-encoded
//! pin down on a second breadcrumb line (`parse_breadcrumb_pin`) —
//! best-effort, the same footing the breadcrumb's path line already stands
//! on. `--identity PATH` or `--pin HEX`, taken here before
//! `write_image::parse` ever sees argv (on `--staging`'s own footing),
//! override it — the route a hand-composed `--document PATH` needs, since it
//! never runs the interview and so never produces a breadcrumb at all.
//!
//! # Wave C2, R-176/R-178/R-180: the image now comes from the fleet
//! repository, selected before anything else runs
//!
//! Until this wave, an image was `--image PATH` or the menu
//! `write_image::run`'s own `out/*-card.img` scan offers — the shape this
//! module had before R-174 and reverted back to. **That is now the fallback
//! for a literal file only.** `crate::provision_image` holds the decisions
//! (platform, schema); `ImageSelector`/`RealImageSelector` below are the
//! seam, on `Preflight`/`Interview`'s own footing: `run_with` calls
//! `ImageSelector::select` once, before the interview ever runs (R-178 P3)
//! and identically on R-180's `--existing HOSTNAME` route (item 8) — a
//! refusal (no fleet repository content, an unknown platform, nothing
//! selectable for one) costs the operator no typing either way.
//!
//! **`--image` now means two things** (R-178 item 7): a filesystem path,
//! unchanged, or a fleet-repository digest/prefix, resolved with
//! `fleet::resolve_image` and then checked against the platform and schema
//! exactly like a menu pick. `RealImageSelector::select` tells the two
//! apart with one `Path::is_file` check — a literal file bypasses the
//! repository (and the fleet-repository offer) entirely, which is why an
//! agent's `--document PATH --image /a/real/file.img` run costs nothing new.
//!
//! **The materialised image is a scratch working copy, removed after the
//! write — success or failure.** `run_with` never returns without cleaning
//! it up: seeded once, right after selection, and removed once
//! `Writer::write` has returned, whichever way.
//!
//! # 2026-09-14: `--at` did not reach the interview, or the register
//!
//! Found building `cmd/cargo-rivet/tests/pty_interview.rs` (the transcript,
//! kept outside this repository, is `r178-p6-pty-proof/README.md` in the
//! maintainer's scratchpad): **`SubprocessInterview`
//! set only `RIVET_FLEET_REPOSITORY` on the `cmd/rivet-provision` child.**
//! That binary reads `HOME`/`USERPROFILE` directly for its CA
//! (`authority_directory`), its saved-answers notebook, the document and the
//! `ADR-049` staging bundle it writes (all three `save::directory`), and its
//! default "Identity" suggestion — every one of them this HOST's real ones,
//! `--at` or no `--at`. A scratch `--at` run offered `resume which?` naming
//! a real saved notebook from the maintainer's own machine before a single
//! keystroke was sent.
//!
//! **The fix is one more variable on the child's environment: `RIVET_AT`.**
//! `SubprocessInterview` now carries the SAME `at` this process already
//! resolved for its own credentials survey and image verification
//! (`RealPreflight`/`RealImageSelector`'s own `at` field) and sets it on the
//! child, always — `--at DIR` given or not, since without `--at` this is
//! simply `credentials::root()`'s own answer, the identical directory
//! `cmd/rivet-provision` would have computed for itself from `HOME`/
//! `USERPROFILE` anyway. `cmd/rivet-provision`'s own `rivet_root` (that
//! crate's module documentation) reads `RIVET_AT` first and falls back to
//! its historical `$HOME/.rivet`/`$USERPROFILE\.rivet` default only when the
//! variable is unset — so running it directly, with nobody setting
//! `RIVET_AT`, is unchanged.
//!
//! `breadcrumb_path` is part of the same fix: it used to call
//! `crate::credentials::root()` fresh, ignoring whatever `at` this run had
//! actually resolved — an orchestrator launched with `--at DIR` looked for
//! the breadcrumb under this host's real home, a second, independent
//! divergence from `--at` sitting right beside the one inside the child. It
//! now takes `at: &Path` from `SubprocessInterview` directly, the exact
//! value the (now fixed) child was told to use, so reader and writer are
//! provably the same directory.
//!
//! **The register is a second root `--at` did not reach either, and closing
//! it needed a different rule.** [`crate::device::default_register_dir`]
//! resolves `rivet-fleet`'s register through `ignition::config` —
//! `%APPDATA%\rivet-fleet\register` on Windows by default — which has no
//! relationship to `HOME`/`USERPROFILE`, and none to `--at`. Left alone, a
//! scratch `--at` run's enrollment (R-178 Q7) would still land in the
//! OPERATOR'S REAL register. `register_root_for` branches on
//! `ProvisionArgs::at` — the LITERAL `--at DIR` the caller typed, **not**
//! the resolved `at` used everywhere else in this module — because the two
//! must not be conflated here: without `--at`, `at` still defaults to
//! `credentials::root()`, and switching the ordinary run's register to
//! follow IT would silently move every already-provisioned operator's
//! enrollment out from under their real, already-populated register.
//! Given `--at DIR`, the register is `DIR/register`, never
//! `default_register_dir`'s ignition-resolved answer; without it, nothing
//! changes — real provisioning keeps enrolling into the real register at
//! its real, `RIVET_FLEET_REGISTER`-overridable location, exactly as
//! before.
//!
//! **The fleet repository's own default is the one root where using the
//! resolved `at` (rather than the literal flag) is exactly right.**
//! `fleet_repository_root` fell back to `rivetos_fleet_repo::default_root`
//! — a fresh `HOME`/`USERPROFILE` read, `--at` or no `--at` — whenever
//! `RIVET_FLEET_REPOSITORY` was not set. It now falls back to
//! `at.join("fleet")` instead, `at` being the resolved credential root every
//! other step of this run already uses. **This changes nothing about the
//! ordinary, no-`--at` answer**: `at` itself defaults to
//! `credentials::root()`, the identical `$HOME/.rivet` shape
//! `rivetos_fleet_repo::default_root` computed independently, byte for
//! byte — the only observable change is that a `--at`-only run (neither
//! `HOME` nor `USERPROFILE` set, previously refused outright for image
//! selection) now resolves a repository, and a scratch `--at` run defaults
//! to a scratch repository instead of the operator's real one when
//! `RIVET_FLEET_REPOSITORY` is left unset.
//!
//! `cmd/cargo-rivet/tests/pty_interview.rs` no longer redirects
//! `HOME`/`USERPROFILE` at all — `--at` alone governs the whole run now —
//! and instead points them at a directory that must never come to exist,
//! asserting exactly that after every run: the harness's own proof that
//! nothing under the real home is consulted any more.

use crate::fleet::{self, Entry};
use crate::write_image::{self, ProvisionArgs};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

/// `rivet-fleetd` — a server's own product, `ADR-050` §5.
const FLEETD_SERVICE: &str = "rivet-fleetd";
/// `rivet-proxyd` — a server's and a member's own front door, `ADR-050` §5.
const PROXYD_SERVICE: &str = "rivet-proxyd";

/// The file `cmd/rivet-provision` leaves beside a document it just wrote —
/// this orchestrator's whole hand-off from the interview to `write-image`.
const BREADCRUMB_NAME: &str = ".latest-run.txt";

/// What the interview handed back — the document it wrote, and, best-effort,
/// this card's own control-plane pin (`ADR-049` §8.1) if `cmd/rivet-provision`
/// could read one off the "Identity" answer. See the module documentation's
/// "Where the pin comes from".
#[derive(Debug, Clone, PartialEq, Eq)]
struct InterviewResult {
    document_path: PathBuf,
    /// `None` for a stand-alone machine, an Identity answer that named
    /// nothing readable, or an interview build old enough not to write this
    /// line at all.
    pin: Option<[u8; 32]>,
    /// **Wave C3.** The "Identity" answer, verbatim, when the interview build
    /// wrote a third breadcrumb line for it — `None` on the same three
    /// conditions as [`Self::pin`], plus a build old enough to write a pin
    /// but not this. `write_image::run`'s widened injection (R-178 item 1)
    /// falls back to `credentials::Survey`'s own default when this is `None`.
    identity_dir: Option<PathBuf>,
}

/// What runs the interview and returns what it produced.
/// Injected so [`run_with`]'s sequencing is provable with no terminal and no
/// real subprocess — see the module documentation.
trait Interview {
    /// # Errors
    ///
    /// The interview could not be started or exited without composing a
    /// document — see [`SubprocessInterview::run`] for the concrete cases.
    fn run(&self) -> Result<InterviewResult, String>;
}

/// What turns a composed document into a device write — `write_image::run`,
/// or a fake that records what it was asked to do.
trait Writer {
    /// # Errors
    ///
    /// Whatever `write_image::run` itself returns.
    fn write(&self, args: &ProvisionArgs) -> Result<write_image::WriteOutcome, String>;
}

/// The real writer: `write_image::run`, called directly rather than
/// re-implemented — see the module documentation.
struct RealWriter;

impl Writer for RealWriter {
    fn write(&self, args: &ProvisionArgs) -> Result<write_image::WriteOutcome, String> {
        write_image::run_outcome(args)
    }
}

// ---------------------------------------------------------------------------
// Wave C2 — R-176/R-178: selecting (and, when needed, materialising) the
// system image, before the interview ever runs.
// ---------------------------------------------------------------------------

/// What [`ImageSelector::select`] hands back — a real file ready for
/// `write_image::run`'s own `--image`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SelectedImage {
    path: PathBuf,
    /// `Some(path)` (always equal to [`Self::path`]) when this is a SCRATCH
    /// working copy this run materialised from the fleet repository, and
    /// must be removed once the write has run — success or failure. `None`
    /// for a literal `--image PATH` naming a real file directly, which is
    /// never removed.
    scratch: Option<PathBuf>,
    /// **Wave C3, item 5.** `"<product> <version> (<digest prefix>)"` when
    /// this was resolved against a fleet-repository `Entry` (both the digest
    /// route and the menu route) — the summary's own line, threaded down
    /// from here rather than re-derived, since a literal `--image PATH` has
    /// no `Entry` to name it from at all (`None`, and the summary names the
    /// path instead).
    descriptor: Option<String>,
    /// The fleet-repository digest this was resolved from (`None` for a
    /// literal `--image PATH`) — what the rerun command a declined write names
    /// passes as `--image`, since the scratch copy is gone by then.
    digest: Option<String>,
    /// The release this image was published as (`None` for a literal `--image
    /// PATH`, whose version this workstation has no record of) — what
    /// [`refuse_an_image_without_the_proxy`] compares.
    version: Option<String>,
}

/// **`ADR-060` §5: a server or member card needs an image that carries the
/// proxy.** Such a machine's front door is part of its image now and can no
/// longer be carried, so an older image would come up with nothing on 443.
///
/// `Ok(None)` when there is nothing to say — a stand-alone machine, or an image
/// at or after `rivet_core::service::PROXY_IN_IMAGE_SINCE`. `Ok(Some(note))`
/// when the image was named by path and its version cannot be known here: the
/// write goes ahead and the note says what was not checked. `Err` is the
/// refusal, naming the version, the floor and what to do.
///
/// # Errors
///
/// A server or member document and an image older than the floor, or one
/// whose version does not read as a release.
fn refuse_an_image_without_the_proxy(
    doc: &rivet_config::Document,
    version: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(role) = role_of(doc) else {
        return Ok(None);
    };
    let floor = rivet_core::service::PROXY_IN_IMAGE_SINCE;
    let role = match role {
        rivetos_register::FleetRole::Server => "fleet server",
        _ => "fleet member",
    };
    let Some(version) = version else {
        return Ok(Some(format!(
            "--- NOT CHECKED: this image was named by path, so its release is not recorded here \
             and whether it carries rivet-proxyd could not be checked. A {role}'s front door is \
             part of the image since {floor} (ADR-060); an older image would boot with nothing on \
             443.\n"
        )));
    };
    match rivet_core::service::release_at_least(version, floor) {
        Some(true) => Ok(None),
        Some(false) => Err(format!(
            "NOTHING WAS WRITTEN. This document is a {role}'s, and the selected image is {version}, \
             older than {floor} — the first release whose image carries rivet-proxyd (ADR-060). A \
             {role} whose image has no proxy has no front door, and a card can no longer carry one. \
             Publish a newer image to the fleet repository and select it."
        )),
        None => Err(format!(
            "NOTHING WAS WRITTEN. This document is a {role}'s, and the selected image's version \
             {version:?} does not read as a release, so whether it carries rivet-proxyd ({floor} and \
             later, ADR-060) cannot be told. Select an image published with a release version."
        )),
    }
}

/// **R-178 items 1–7.** What selects and, if needed, materialises the system
/// image a card is written from — injected on [`Preflight`]/[`Interview`]'s
/// own footing, so `run_with`'s ordering (before the interview; the
/// identical selection on R-180's `--existing` route) is provable with fakes
/// touching no real fleet repository and no real image bytes.
trait ImageSelector {
    /// `requested_platform`: `--platform TOKEN`. `requested_image`: `--image`
    /// exactly as given — a filesystem path (unchanged meaning) or a
    /// repository digest/prefix (item 7).
    ///
    /// **`Ok(None)` is the operator leaving the platform or system-image menu
    /// with `x`/`X`** (`cmd/cargo-rivet/src/write_image.rs::Choice::Exit`) —
    /// not a failure, and `run_with` ends the whole run right there with
    /// exit status 0, before the interview, the staging check or the writer
    /// ever run.
    ///
    /// # Errors
    ///
    /// Every refusal items 1–4 and item 7 name — see
    /// `crate::provision_image`'s own module documentation.
    fn select(
        &self,
        requested_platform: Option<&str>,
        requested_image: Option<&Path>,
    ) -> Result<Option<SelectedImage>, String>;
}

/// The real selector — `crate::provision_image`'s own functions, called in
/// order: item 7's literal-path bypass first, then item 1's offer, item 7's
/// digest resolution OR items 2–4's platform/listing/schema menu, then item
/// 6's verify-and-decompress.
struct RealImageSelector<'a> {
    /// `None` only when this host sets neither `HOME` nor `USERPROFILE` — the
    /// same condition [`fleet_repository_root`] already reports for the
    /// commit step.
    fleet_repo_root: Option<&'a Path>,
    /// `~/.rivet` (or `--at DIR`) — where `keys/rivet-system.pub` is read
    /// from for item 6's verification.
    at: &'a Path,
    is_terminal: bool,
    /// `None` prints nothing beyond what a menu prompt itself prints — see
    /// `crate::progress`.
    stages: Option<&'a crate::progress::Stages>,
}

impl RealImageSelector<'_> {
    /// Item 6, common to both the digest route and the menu route.
    fn materialise_selected(&self, root: &Path, entry: &Entry) -> Result<SelectedImage, String> {
        for line in crate::provision_image::sweep_stale_scratch(self.at, std::time::SystemTime::now()) {
            eprintln!("cargo-rivet: {line}");
        }
        let out_dir = crate::provision_image::scratch_dir(self.at);
        let stage = self.stages.map(|s| {
            s.start(&format!("verifying and decompressing {} {}", entry.product, entry.version))
        });
        match crate::provision_image::materialise(root, entry, self.at, &out_dir) {
            Ok(path) => {
                if let Some(t) = stage {
                    t.done();
                }
                Ok(SelectedImage {
                    path: path.clone(),
                    scratch: Some(path),
                    descriptor: Some(format!(
                        "{} {} ({})",
                        entry.product,
                        entry.version,
                        &entry.digest[..entry.digest.len().min(12)],
                    )),
                    digest: Some(entry.digest.clone()),
                    version: Some(entry.version.clone()),
                })
            }
            Err(e) => {
                if let Some(t) = &stage {
                    t.failed(&e);
                }
                Err(e)
            }
        }
    }
}

impl ImageSelector for RealImageSelector<'_> {
    fn select(
        &self,
        requested_platform: Option<&str>,
        requested_image: Option<&Path>,
    ) -> Result<Option<SelectedImage>, String> {
        // **Item 7, unchanged half.** A real file named directly needs no
        // repository, and so no fleet-repository offer either — an agent's
        // `--document PATH --image /a/real/file.img` run costs nothing new.
        if let Some(spec) = requested_image
            && spec.is_file()
        {
            return Ok(Some(SelectedImage {
                path: spec.to_path_buf(),
                scratch: None,
                descriptor: None,
                digest: None,
                version: None,
            }));
        }

        let Some(root) = self.fleet_repo_root else {
            return Err(
                "this host sets neither HOME nor USERPROFILE, so there is nowhere to default \
                 the fleet repository a system image would come from. Pass --at DIR, or \
                 --image PATH naming one directly."
                    .to_owned(),
            );
        };

        // **Item 1.**
        crate::provision_image::ensure_fleet_repository(root, self.is_terminal)?;

        let list_stage = self.stages.map(|s| s.start("listing images for this platform"));
        let fail = |stage: &Option<crate::progress::Stage>, e: String| -> String {
            if let Some(t) = stage {
                t.failed(&e);
            }
            e
        };
        let all_images = fleet::list_images(root, None).map_err(|e| fail(&list_stage, e))?;
        let tool_schema = rivet_config::SCHEMA_VERSION.get();

        // **Item 7, digest half.** Resolved against every platform's card
        // images, then platform and schema are checked exactly like a menu
        // pick — an agent naming a digest never needs the multi-platform
        // prompt at all (`.rulings/` R-178 item 7).
        if let Some(spec) = requested_image {
            let spec_str = spec.to_string_lossy();
            let entry = fleet::resolve_image(&all_images, &spec_str)
                .map_err(|e| fail(&list_stage, e))?
                .clone();
            if let Some(token) = requested_platform
                && entry.platform.as_deref() != Some(token)
            {
                return Err(fail(
                    &list_stage,
                    format!(
                        "{} {} is built for platform {:?}, not {token:?} (--platform). Nothing \
                         was selected.",
                        entry.product,
                        entry.version,
                        entry.platform.as_deref().unwrap_or("(none)"),
                    ),
                ));
            }
            if let Some(reason) = crate::provision_image::schema_block_reason(&entry, tool_schema) {
                return Err(fail(
                    &list_stage,
                    format!(
                        "{} {} cannot be provisioned: {reason}. Nothing was selected.",
                        entry.product, entry.version,
                    ),
                ));
            }
            if let Some(t) = list_stage {
                t.done();
            }
            return self.materialise_selected(root, &entry).map(Some);
        }

        // **Items 2/3 — the menu route.** Platform first, then that
        // platform's own images.
        let all_platforms = crate::provision_image::distinct_platforms(&all_images);
        let platform = match crate::provision_image::decide_platform(&all_platforms, requested_platform)
        {
            Ok(crate::provision_image::PlatformDecision::Resolved(p)) => p,
            Ok(crate::provision_image::PlatformDecision::Ask(many)) => {
                if !self.is_terminal {
                    return Err(fail(
                        &list_stage,
                        format!(
                            "more than one platform has images in the fleet repository ({}), \
                             and this run has no terminal to ask which at. Pass --platform \
                             TOKEN.",
                            many.join(", "),
                        ),
                    ));
                }
                match crate::write_image::choose_with(
                    self.is_terminal,
                    "platform",
                    &many,
                    "Pass --platform TOKEN.",
                ) {
                    Ok(crate::write_image::Choice::Selected(idx)) => many[idx].clone(),
                    // `x`/`X`: leave without choosing a platform, before
                    // anything else has run — see `ImageSelector::select`'s
                    // own doc on what `Ok(None)` means to `run_with`.
                    Ok(crate::write_image::Choice::Exit) => return Ok(None),
                    Err(e) => return Err(fail(&list_stage, e)),
                }
            }
            Err(e) => return Err(fail(&list_stage, e)),
        };
        println!("--- platform: {platform}");

        let for_platform: Vec<Entry> = all_images
            .into_iter()
            .filter(|e| e.platform.as_deref() == Some(platform.as_str()))
            .collect();
        let marked = crate::provision_image::mark_schema(for_platform, tool_schema);
        let selectable: Vec<Entry> =
            marked.iter().filter(|c| c.blocked.is_none()).map(|c| c.entry.clone()).collect();
        let blocked: Vec<crate::provision_image::Candidate> =
            marked.into_iter().filter(|c| c.blocked.is_some()).collect();
        for c in &blocked {
            println!(
                "--- {} {} is not selectable: {}",
                c.entry.product,
                c.entry.version,
                c.blocked.as_deref().unwrap_or(""),
            );
        }
        if selectable.is_empty() {
            let e = crate::provision_image::none_selectable_refusal(&platform, &blocked, tool_schema);
            return Err(fail(&list_stage, e));
        }

        let names: Vec<String> = selectable.iter().map(crate::provision_image::describe_entry).collect();
        let idx = match crate::write_image::choose_with(
            self.is_terminal,
            "system image",
            &names,
            "Pass --image <digest-or-path>.",
        ) {
            Ok(crate::write_image::Choice::Selected(idx)) => idx,
            // `x`/`X`: leave without choosing a system image.
            Ok(crate::write_image::Choice::Exit) => return Ok(None),
            Err(e) => return Err(fail(&list_stage, e)),
        };
        if let Some(t) = list_stage {
            t.done();
        }

        self.materialise_selected(root, &selectable[idx]).map(Some)
    }
}

/// **Preflight** — credentials, surveyed before the interview runs rather
/// than after it. Injected for the reason every other step here is: a test
/// proves this runs before the interview with no real `~/.rivet`.
///
/// This is deliberately narrower than the credentials/Docker/fleet-repository
/// preflight R-174 built: this module builds and publishes nothing any more,
/// so it needs neither a container runtime nor a fleet repository to check.
trait Preflight {
    /// # Errors
    ///
    /// A required credential is missing — see [`RealPreflight::check`].
    fn check(&self) -> Result<String, String>;
}

/// The real preflight: [`crate::credentials::Survey`]'s existing refusal,
/// run here so an operator who is about to spend ten minutes on the
/// interview is not told afterward that a key was missing all along.
/// `write_image::run` performs the identical check again before it writes
/// (belt and braces for the `--document PATH` path, which skips this one).
struct RealPreflight<'a> {
    at: &'a Path,
}

impl Preflight for RealPreflight<'_> {
    fn check(&self) -> Result<String, String> {
        let over = crate::credentials::Overrides::from_environment();
        let survey = crate::credentials::Survey::take(self.at, &over);
        if let Some(why) = survey.refusal() {
            return Err(why);
        }
        Ok("--- credentials: present\n".to_owned())
    }
}

/// **The staging-bundle check** — [`require_staging`], run on whichever
/// document `run_with` ends up with, from the interview or `--document
/// PATH`. Injected on the same footing as [`Preflight`] and [`Interview`]: a
/// test proves `run_with`'s sequencing with a fake that touches no
/// filesystem at all; [`require_staging`] itself is proven separately,
/// directly, against real scratch files.
trait StagingCheck {
    /// **Wave C3**: returns the staging bundle's own path when the document
    /// needed one and one was found — [`require_staging`]'s own `Ok(Some(_))`,
    /// threaded through rather than discarded, so `run_with_image_selected`
    /// can hand the SAME file to the writer for injection (item 1) instead of
    /// this function and the writer each re-deriving where it is.
    ///
    /// **And what to say about it**: empty when the bundle was already there,
    /// and the builder's own report when one had to be built
    /// ([`ensure_staging`]). `identity` is the machine's identity directory,
    /// which a build needs; `None` falls back to the board identity.
    ///
    /// # Errors
    ///
    /// The document cannot be read or does not parse, or [`ensure_staging`]
    /// itself refuses — see [`RealStagingCheck::check`].
    fn check(&self, document_path: &Path, identity: Option<&Path>) -> Result<(Option<PathBuf>, String), String>;
}

/// The real check: read `document_path`, parse it, and hand it to
/// [`ensure_staging`].
struct RealStagingCheck<'a> {
    /// `--staging PATH`, when the caller already knows where the bundle is —
    /// [`require_staging`]'s own override, tried before the naming
    /// convention. **An override is never built**: a bundle named on the
    /// command line and absent is a mistake to report, not a gap to fill.
    staging_override: Option<&'a Path>,
    /// The credential root the build reads the authority and the Armada under.
    at: &'a Path,
    /// What builds a missing bundle — `None` on `--existing HOSTNAME`'s route,
    /// which rewrites a machine already provisioned. Building there would mint
    /// a new `rivet-fleetd` operator key for a server whose members already
    /// trust the old one, so that route still refuses by name (`.rulings/`
    /// R-180: locate it where provisioning left it, or say what is missing).
    builder: Option<&'a dyn StagingBuilder>,
}

impl StagingCheck for RealStagingCheck<'_> {
    fn check(&self, document_path: &Path, identity: Option<&Path>) -> Result<(Option<PathBuf>, String), String> {
        let text = std::fs::read_to_string(document_path)
            .map_err(|e| format!("cannot read {}: {e}", document_path.display()))?;
        let doc = rivet_config::Document::parse(&text)
            .map_err(|e| format!("{} does not parse: {e:?}", document_path.display()))?;
        let Some(builder) = self.builder else {
            return require_staging(document_path, &doc, self.staging_override).map(|p| (p, String::new()));
        };
        let board = identity.is_none().then(|| board_identity(self.at)).flatten();
        ensure_staging(document_path, &doc, self.staging_override, identity.or(board.as_deref()), self.at, builder)
    }
}

/// The board identity `write_image::run` falls back to when no machine
/// identity was named — `credentials::Survey`'s own `BOARD_CREDENTIAL`, so a
/// bundle is built against the identity the card will actually carry.
pub(crate) fn board_identity(at: &Path) -> Option<PathBuf> {
    crate::credentials::Survey::take(at, &crate::credentials::Overrides::from_environment())
        .named(crate::credentials::BOARD_CREDENTIAL)
        .map(|board| board.path.clone())
}

/// **What builds a missing staging bundle** — `rivet-provision stage`, or a
/// fake that writes one. `ADR-049` §12.
pub(crate) trait StagingBuilder {
    /// Build the bundle for `document` beside it, with the machine identity
    /// `identity` and the credential root `at`. Answers what the builder
    /// printed, which names every file staged and its size and no contents.
    ///
    /// # Errors
    ///
    /// The builder could not be run, or refused — its own words.
    fn build(&self, document: &Path, identity: &Path, at: &Path) -> Result<String, String>;
}

/// **The real builder: `rivet-provision stage`**, found exactly as the
/// interview is found — the sibling binary, version-checked against this one
/// (`.rulings/` R-191 D3), or `cargo run` from a checkout, or a refusal
/// naming the install ([`resolve_interview_source`]).
pub(crate) struct RealStagingBuilder;

impl StagingBuilder for RealStagingBuilder {
    fn build(&self, document: &Path, identity: &Path, at: &Path) -> Result<String, String> {
        run_stage(document, identity, at, &[])
    }
}

/// [`RealStagingBuilder`] with `rivet-provision stage`'s further flags —
/// `--fleet-root`, `--suffixes`, `--passkeys` — passed through verbatim.
struct StagingWith {
    extra: Vec<String>,
}

impl StagingBuilder for StagingWith {
    fn build(&self, document: &Path, identity: &Path, at: &Path) -> Result<String, String> {
        run_stage(document, identity, at, &self.extra)
    }
}

/// `--fleet-root FLEET`, `--suffixes LIST` and `--passkeys FILE`, taken out of
/// `args` for `rivet-provision stage`, in the order given.
///
/// # Errors
///
/// A flag with no value after it.
fn extract_stage_extras(args: &[String]) -> Result<(Vec<String>, Vec<String>), String> {
    let mut rest = Vec::with_capacity(args.len());
    let mut extra = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let word = &args[index];
        if matches!(word.as_str(), "--fleet-root" | "--suffixes" | "--passkeys") {
            let Some(value) = args.get(index + 1) else {
                return Err(format!("{word} needs a value"));
            };
            extra.push(word.clone());
            extra.push(value.clone());
            index += 2;
        } else {
            rest.push(word.clone());
            index += 1;
        }
    }
    Ok((rest, extra))
}

/// Run `rivet-provision stage` for `document`, with `extra` flags.
fn run_stage(document: &Path, identity: &Path, at: &Path, extra: &[String]) -> Result<String, String> {
    {
        let sibling = sibling_binary("rivet-provision")?;
        let cwd = std::env::current_dir().ok();
        let mut command = match resolve_interview_source(&sibling, cwd.as_deref())? {
            InterviewSource::Sibling(exe) => {
                let mine = crate::build_version();
                let theirs = sibling_build(&exe);
                if let Some(why) = version_refusal(&mine, theirs.as_deref().map_err(String::as_str), &exe) {
                    return Err(why);
                }
                std::process::Command::new(exe)
            }
            InterviewSource::CargoRun(checkout) => {
                let mut cargo = std::process::Command::new("cargo");
                cargo
                    .args(["run", "--release", "-q", "-p", "rivetos-provision", "--bin", "rivet-provision", "--"])
                    .current_dir(checkout);
                cargo
            }
        };
        let out = command
            .arg("stage")
            .arg("--document")
            .arg(document)
            .arg("--identity")
            .arg(identity)
            .arg("--at")
            .arg(at)
            .args(extra)
            .output()
            .map_err(|e| format!("cannot run rivet-provision stage: {e}"))?;
        let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        if out.status.success() {
            Ok(said)
        } else {
            Err(format!("rivet-provision stage exited {}:\n{}", out.status, said.trim_end()))
        }
    }
}

/// **The staging bundle that belongs beside `document`** — the rule
/// `cmd/rivet-provision`'s `save::staging_beside` writes by, restated because
/// neither crate is a library the other can call. A `<stem>.provisioning.toml`
/// has `<stem>.staged.txt` ([`staging_path_beside`]); any other name has
/// `<hostname>.staged.txt`, the hostname made safe as a file name.
#[must_use]
pub(crate) fn staging_path_for_document(document: &Path, doc: &rivet_config::Document) -> PathBuf {
    staging_path_beside(document)
        .filter(|path| path.file_name().is_some_and(|name| name != ".staged.txt"))
        .unwrap_or_else(|| {
            let hostname = doc.hostname().map_or_else(|| "unnamed".to_owned(), ToString::to_string);
            document.with_file_name(format!("{}.staged.txt", safe_name(&hostname)))
        })
}

/// **A server or member document's staging bundle, built when it is missing**
/// — `ADR-049` §12. Answers the bundle's path (`None` for a document with
/// no role) and what to say: nothing when the bundle was already there, the
/// builder's own report when it was built.
///
/// `staging_override` (`--staging PATH`) is checked as given and never
/// built. Otherwise the bundle beside the document
/// ([`staging_path_for_document`]) is used when it exists, and built with
/// `builder` when it does not — then checked by [`require_staging`] exactly as
/// one the interview wrote would be.
///
/// # Errors
///
/// [`require_staging`]'s refusals; no identity to build with; the builder's
/// own refusal; or a build that left no readable bundle behind.
pub(crate) fn ensure_staging(
    document_path: &Path,
    doc: &rivet_config::Document,
    staging_override: Option<&Path>,
    identity: Option<&Path>,
    at: &Path,
    builder: &dyn StagingBuilder,
) -> Result<(Option<PathBuf>, String), String> {
    if !requires_staging(doc) || staging_override.is_some() {
        return require_staging(document_path, doc, staging_override).map(|p| (p, String::new()));
    }
    let bundle = staging_path_for_document(document_path, doc);
    if bundle.is_file() {
        return require_staging(document_path, doc, Some(&bundle)).map(|p| (p, String::new()));
    }
    let Some(identity) = identity else {
        return Err(format!(
            "NOTHING WAS WRITTEN. {} enables a fleet role whose services are handed material from \
             the card (ADR-060 §5), it has no staging bundle at {}, and there is no machine identity \
             to build one with. Pass --identity DIR.",
            document_path.display(),
            bundle.display()
        ));
    };
    let said = builder.build(document_path, identity, at).map_err(|why| {
        format!(
            "NOTHING WAS WRITTEN. {} has no staging bundle at {}, and building one failed:\n{why}",
            document_path.display(),
            bundle.display()
        )
    })?;
    let found = require_staging(document_path, doc, Some(&bundle))?;
    let mut report = format!(
        "--- staging bundle: none beside {}, so `rivet-provision stage` built {} (ADR-049) — \
         names and sizes only:\n",
        document_path.display(),
        bundle.display()
    );
    for line in said.lines() {
        report.push_str(&format!("    {line}\n"));
    }
    Ok((found, report))
}

/// **The install, in the one spelling every refusal in this module uses.**
///
/// Two lines, because `cargo install --path cmd/cargo-rivet` copies exactly
/// the `[[bin]]` that crate declares and never `cmd/rivet-provision`'s — and
/// `provision` is the one verb that runs a second binary. `.rulings/` R-192
/// D3: install both, and stop refusing on the working directory.
///
/// **`--locked`**, because without it `cargo install` resolves dependencies
/// afresh rather than reading the committed `Cargo.lock`, so the tool on a
/// PATH is built from versions the repository never tested.
const INSTALL_BOTH: &str = "\
\x20 cargo install --path cmd/cargo-rivet --force --locked\n\
\x20 cargo install --path cmd/rivet-provision --force --locked";

/// **Where the interview comes from, decided once, before anything else runs.**
///
/// Three outcomes, and the order matters:
///
/// 1. **A sibling `rivet-provision` beside this binary** — what the install
///    above leaves in `~/.cargo/bin`, and what a `target/<profile>` build
///    leaves beside a developer's own `cargo-rivet`. This route needs no
///    `cargo`, no source checkout and no compilation, so **the verb works
///    from any directory**.
/// 2. **No sibling, but the working directory IS a Rivet checkout** — the
///    developer's route, kept deliberately: `cargo run` from that checkout is
///    what makes an edit to `cmd/rivet-provision` take effect in the very
///    next `cargo rivet provision`, without reinstalling anything.
/// 3. **Neither** — a refusal naming the install, never the directory. The
///    directory is not the problem and saying it was is what sent the
///    maintainer's first real run looking for a checkout it did not need.
#[derive(Debug, PartialEq, Eq)]
enum InterviewSource {
    /// Run this binary directly.
    Sibling(PathBuf),
    /// `cargo run --release -q -p rivetos-provision --bin rivet-provision`,
    /// from the checkout this names.
    CargoRun(PathBuf),
}

/// The decision itself, pure: is there a sibling, and failing that, is `cwd`
/// a checkout that could build one?
///
/// A function rather than inlined into [`SubprocessInterview::run`] so all
/// three outcomes are provable with paths that do and do not exist, and
/// nothing else — no process, no terminal, no `HOME`.
///
/// `cwd` is `None` when this host will not say what the working directory is
/// (it was removed underneath the process, or the platform refused). That is
/// not fatal on its own: a sibling still answers, and only case 3's refusal
/// has to word it.
///
/// # Errors
///
/// Case 3 above: no sibling and no checkout to build one from.
fn resolve_interview_source(sibling: &Path, cwd: Option<&Path>) -> Result<InterviewSource, String> {
    if sibling.is_file() {
        return Ok(InterviewSource::Sibling(sibling.to_path_buf()));
    }
    if let Some(dir) = cwd
        && is_rivet_checkout(dir)
    {
        return Ok(InterviewSource::CargoRun(dir.to_path_buf()));
    }
    Err(format!(
        "cargo rivet provision needs the interview, and this host has no rivet-provision to \
         run.\n\
         \x20 looked beside this binary at {}\n\
         \x20 and there is no Rivet checkout at {} to build one from\n\
         Install BOTH workstation binaries from a Rivet checkout, then run this again from \
         wherever you like:\n\
         {INSTALL_BOTH}\n\
         A document you already have skips the interview entirely: cargo rivet provision \
         --document PATH.",
        sibling.display(),
        match cwd {
            Some(dir) => dir.display().to_string(),
            None => "the current directory, which this host will not name".to_owned(),
        },
    ))
}

/// Whether `dir` is a Rivet checkout — it holds the crate `cargo run` would
/// build. **Asked only when there is no sibling**, and never as a condition
/// on the verb itself.
fn is_rivet_checkout(dir: &Path) -> bool {
    dir.join("cmd").join("rivet-provision").join("Cargo.toml").is_file()
}

/// **`.rulings/` R-191 D3 — a mismatched pair is refused, naming both
/// builds.** `None` when they agree, which is the ordinary answer.
///
/// `mine` is [`crate::build_version`]; `theirs` is what the sibling's own
/// `--version` said, or `Err` with why it could not be read at all — a build
/// old enough to refuse the flag, a file that will not start, or output
/// nothing can parse. Both are refusals: the two binaries share the interview's
/// files, the breadcrumb and the notebook format, so a pair that cannot be
/// shown to match is a pair that fails in odd ways later instead.
///
/// Pure, so the wording is provable without a second binary on the host.
fn version_refusal(mine: &str, theirs: Result<&str, &str>, exe: &Path) -> Option<String> {
    let (line, why) = match theirs {
        Ok(theirs) if comparable(theirs) == comparable(mine) => return None,
        Ok(theirs) => (theirs.to_owned(), "the two builds are not the same"),
        Err(reason) => (
            format!("would not say ({reason})"),
            "the interview would not say which build it is",
        ),
    };
    Some(format!(
        "cargo rivet provision refuses a mismatched pair: {why}. They share the interview's \
         files, its breadcrumb and the saved-answers notebook, so a mismatch fails in odd ways \
         rather than cleanly (`.rulings/` R-191).\n\
         \x20 {:<16}{mine}\n\
         \x20 {:<16}{line}\n\
         \x20 {:<16}{}\n\
         Reinstall both from the same checkout:\n\
         {INSTALL_BOTH}",
        "cargo-rivet",
        "rivet-provision",
        "",
        exe.display(),
    ))
}

/// **What is compared, which is the version and the commit and NOT the
/// dirtiness.** `-dirty` is dropped from both sides first.
///
/// The reason is in `cmd/cargo-rivet/build.rs`'s own caveat: a build script
/// re-runs on `HEAD` and `index` moving, so editing a tracked file without
/// staging it leaves a stamp that still says clean. Two binaries built
/// minutes apart in one working tree therefore disagree about `-dirty`
/// routinely, while agreeing about the commit — and refusing that pair would
/// fire on every developer with an edit in flight while still proving nothing
/// about the edit, since the flag is as likely to be stale as not. The commit
/// is the half that identifies a build, and it is the half compared.
pub(crate) fn comparable(build: &str) -> String {
    build.replace("-dirty", "")
}

/// What a binary's `--version` says about its build — the text after the
/// program's own name, which is the half [`version_refusal`] compares.
///
/// Pure, and separate from running anything, so the shapes this has to
/// tolerate are provable: `cargo-rivet 0.1.87 (v0.1.86-9-gabc)` and
/// `rivet-provision 0.1.87 (v0.1.86-9-gabc)` both yield the same string, and
/// anything without a space after the first word yields `None` rather than a
/// half-read answer treated as a version.
fn parse_version_line(text: &str) -> Option<String> {
    let line = text.lines().find(|l| !l.trim().is_empty())?.trim();
    let (_program, build) = line.split_once(' ')?;
    let build = build.trim();
    if build.is_empty() { None } else { Some(build.to_owned()) }
}

/// Ask the sibling what build it is, by running it. `Err` carries a reason
/// short enough to sit inside [`version_refusal`]'s own line.
fn sibling_build(exe: &Path) -> Result<String, String> {
    let out = std::process::Command::new(exe)
        .arg("--version")
        .output()
        .map_err(|e| format!("it would not start: {e}"))?;
    if !out.status.success() {
        return Err(format!("--version exited {}", out.status));
    }
    let said = String::from_utf8_lossy(&out.stdout);
    parse_version_line(&said)
        .ok_or_else(|| format!("--version printed {:?}", said.trim().chars().take(60).collect::<String>()))
}

/// The real interview, from wherever [`resolve_interview_source`] says, with
/// this process's own stdio inherited, so the operator sees the exact tool
/// `SPEC-05` §4 already builds.
struct SubprocessInterview<'a> {
    /// **This process's own working directory, or `None` when the host will
    /// not say what it is.** Nothing here requires it to be a checkout:
    /// [`resolve_interview_source`] consults it only when no sibling
    /// `rivet-provision` was found, and only to decide whether `cargo run`
    /// has a crate to build.
    cwd: Option<&'a Path>,
    /// Where stage announcements go — see `crate::progress`. `None` prints
    /// nothing beyond what the interview's own subprocess prints.
    stages: Option<&'a crate::progress::Stages>,
    /// **R-178 P3.** Set as `RIVET_FLEET_REPOSITORY` on the interview's own
    /// environment, so it lists application versions against the same
    /// repository this module resolves for its own commit later — see the
    /// module documentation. `None` when this process could not resolve one
    /// either; the interview then falls back to its own default, exactly as
    /// it does when run outside this orchestrator entirely.
    fleet_repo_root: Option<&'a Path>,
    /// **The fix this module's own "2026-09-14" section documents.** Set as
    /// `RIVET_AT` on the interview's own environment — the same root
    /// `RealPreflight`/`RealImageSelector` already use for this process's
    /// own credentials survey and image verification, `--at DIR` or this
    /// host's own default. Also where [`breadcrumb_path`] looks for what the
    /// interview wrote, so the writer and the reader are provably the same
    /// directory rather than two independent reads that happen to agree
    /// today.
    at: &'a Path,
    /// **`.rulings/` R-189 D2.** Set as `RIVET_FLEET_REGISTER` on the
    /// interview's own environment, so its `w` key refuses an identity another
    /// machine's record already carries against the SAME register
    /// [`RealFleetRecorder::plan`] checks afterwards.
    register_root: &'a Path,
}

impl Interview for SubprocessInterview<'_> {
    fn run(&self) -> Result<InterviewResult, String> {
        let breadcrumb = breadcrumb_path(self.at);
        // A breadcrumb left by an EARLIER run must not be mistaken for this
        // one if the operator quits this run without ever pressing `w`.
        let _ = std::fs::remove_file(&breadcrumb);

        let sibling = sibling_binary("rivet-provision")?;
        let (mut command, what, missing_tool_hint) = match resolve_interview_source(&sibling, self.cwd)? {
            InterviewSource::Sibling(exe) => {
                // **`.rulings/` R-191 D3, and it is asked HERE rather than at
                // the `cargo run` route as well, deliberately.** An installed
                // pair is two files that were installed at two different
                // moments and can be from two different commits, which is the
                // mismatch that has to be caught. `cargo run` compiles the
                // interview out of the checkout it is pointed at, every run,
                // so that route cannot be stale with respect to the source it
                // is asked to use — and the developer editing that source is
                // the whole reason the route exists.
                let mine = crate::build_version();
                let theirs = sibling_build(&exe);
                if let Some(why) =
                    version_refusal(&mine, theirs.as_deref().map_err(String::as_str), &exe)
                {
                    return Err(why);
                }
                let described = exe.display().to_string();
                (
                    std::process::Command::new(exe),
                    described,
                    "It is there but would not start -- check its own permissions, or pass \
                     --document PATH with a document already composed."
                        .to_owned(),
                )
            }
            InterviewSource::CargoRun(checkout) => {
                let mut cargo = std::process::Command::new("cargo");
                cargo
                    .args(["run", "--release", "-q", "-p", "rivetos-provision", "--bin", "rivet-provision"])
                    .current_dir(&checkout);
                (
                    cargo,
                    "cargo run -p rivetos-provision --bin rivet-provision".to_owned(),
                    "This needs `cargo` on PATH -- the same toolchain every other command in \
                     this project already assumes -- and the repository this was cloned from \
                     at the current directory. The first run compiles cmd/rivet-provision in \
                     release mode, which can take a while; later runs are near-instant."
                        .to_owned(),
                )
            }
        };

        // **R-178 P3** — the interview lists application versions against
        // THIS repository, the same one this process resolves for its own
        // document commit below, so the two halves of one run agree.
        if let Some(root) = self.fleet_repo_root {
            command.env("RIVET_FLEET_REPOSITORY", root);
        }
        // **The fix this module's own "2026-09-14" section documents.**
        // Without this, `cmd/rivet-provision` reads `HOME`/`USERPROFILE`
        // directly for its CA, its saved-answers notebook and the
        // breadcrumb below — this host's real ones, `--at` or no `--at`.
        command.env("RIVET_AT", self.at);
        // **Tells the interview this run writes the card itself**, so it
        // prints no advice about getting its document onto one — found on
        // the maintainer's first real run, 2026-09-14, where it told him to
        // build a card by hand moments before this process wrote one.
        // `cmd/rivet-provision`'s `orchestrated` reads it.
        command.env("RIVET_PROVISION_ORCHESTRATED", "1");
        // **`.rulings/` R-189 D2**: the register the interview's `w` checks
        // an identity against. `rivet-fleet`'s own override variable, and the
        // same root this process enrolls into.
        command.env("RIVET_FLEET_REGISTER", self.register_root);

        let timer = self.stages.map(|s| s.start(&format!("running the interview ({what})")));
        let fail = |why: String| -> String {
            if let Some(t) = &timer {
                t.failed(&why);
            }
            why
        };

        let status = command
            .status()
            .map_err(|e| fail(format!("cannot start the interview via {what}: {e}. {missing_tool_hint}")))?;
        if !status.success() {
            return Err(fail(format!(
                "{what} exited ({status}) without writing a document — declining to write, \
                 saving, or a refusal it already reported. Run it directly to see why, or \
                 compose a document yourself and pass --document PATH.",
            )));
        }

        let text = std::fs::read_to_string(&breadcrumb).map_err(|e| {
            fail(format!(
                "{what} exited successfully but left no document behind at {}: {e}. It writes \
                 this file the moment `w` succeeds, so either nothing was written this run or \
                 an older build of it answered instead.",
                breadcrumb.display(),
            ))
        })?;
        let path = parse_breadcrumb(&text).ok_or_else(|| {
            fail(format!("{} is empty — {what} wrote no path to it", breadcrumb.display()))
        })?;
        if !path.is_file() {
            return Err(fail(format!("{what} named a document that is not there: {}", path.display())));
        }
        if let Some(t) = timer {
            t.done();
        }
        Ok(InterviewResult {
            document_path: path,
            pin: parse_breadcrumb_pin(&text),
            identity_dir: parse_breadcrumb_identity(&text),
        })
    }
}

/// The document path out of the breadcrumb's text — pure, so the parsing is
/// provable with no file at all.
fn parse_breadcrumb(text: &str) -> Option<PathBuf> {
    let line = text.lines().next()?.trim();
    if line.is_empty() {
        return None;
    }
    Some(PathBuf::from(line))
}

/// The pin off the breadcrumb's SECOND line, `pin=<64 hex characters>` —
/// `.rulings/` R-178 Q7. `None` for anything else: a shorter breadcrumb (a
/// stand-alone machine, an Identity answer `cmd/rivet-provision` could not
/// read, or a build old enough not to write this line at all), a second line
/// with the wrong prefix, or hex that does not decode to exactly 32 bytes.
/// Never a reason [`SubprocessInterview::run`] itself fails — the document
/// path on line one is the one fact that is load-bearing there.
fn parse_breadcrumb_pin(text: &str) -> Option<[u8; 32]> {
    let line = text.lines().nth(1)?.trim();
    decode_pin_hex(line.strip_prefix("pin=")?).ok()
}

/// The identity directory off the breadcrumb's THIRD line,
/// `identity=<path>` — **Wave C3**, the same footing [`parse_breadcrumb_pin`]
/// stands on, one line further. `None` for a breadcrumb with fewer than
/// three lines, or a third line with the wrong prefix.
fn parse_breadcrumb_identity(text: &str) -> Option<PathBuf> {
    let line = text.lines().nth(2)?.trim();
    let path = line.strip_prefix("identity=")?;
    if path.is_empty() { None } else { Some(PathBuf::from(path)) }
}

/// Where `cmd/rivet-provision` writes [`BREADCRUMB_NAME`] — `<at>/provision/`,
/// the same directory `SPEC-05` §4 already writes the document itself into.
///
/// **Takes `at` directly, rather than re-resolving `crate::credentials::root()`
/// on its own** — this module's own "2026-09-14" section: a second,
/// independent read of `HOME`/`USERPROFILE` here meant this function looked
/// in a different place than `--at DIR` told the interview to write to.
/// [`SubprocessInterview::run`] passes its own `at` field, which is always
/// already resolved (`--at DIR`, or [`run`]'s own default) by the time an
/// interview is ever launched — so this is now infallible.
fn breadcrumb_path(at: &Path) -> PathBuf {
    at.join("provision").join(BREADCRUMB_NAME)
}

/// The sibling binary next to this one, if there happens to be one there —
/// `current_exe()`'s own directory, the same place `crate::elevate` finds
/// itself for the elevated relaunch. **Expected but not assumed**: README §2
/// installs both binaries, so on a host installed since R-192 this is where
/// the interview is; a host installed before it has only `cargo-rivet` here,
/// and [`resolve_interview_source`] falls back or refuses rather than
/// treating the absence as an error of its own.
///
/// # Errors
///
/// This host's own path cannot be found.
pub(crate) fn sibling_binary(name: &str) -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("cannot find this host's own path to look beside it: {e}"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", exe.display()))?;
    Ok(sibling_binary_in(dir, name))
}

/// [`sibling_binary`], with the directory taken as a parameter so the
/// filename join is provable without `current_exe()` in the loop.
fn sibling_binary_in(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

/// **Whether `doc`'s role needs the `ADR-049` staging bundle** — a fleet server
/// or a fleet member, read by [`role_of`] off the services the document
/// enables (`ADR-060` §5).
///
/// By role and not by what is carried: both `rivet-fleetd` and `rivet-proxyd`
/// are handed material from the card — the register and credentials, the
/// front door's certificate — and since the proxy is part of the image a
/// member's card carries nothing at all, so "carries a payload" would stop
/// asking a member for the certificate its proxy cannot start without.
#[must_use]
fn requires_staging(doc: &rivet_config::Document) -> bool {
    role_of(doc).is_some()
}

/// The staging bundle path beside `document_path`, on
/// `cmd/rivet-provision/src/save.rs`'s own naming convention:
/// `<host>.provisioning.toml` beside `<host>.staged.txt`, one stem, two
/// suffixes. `None` when `document_path` does not end in the document suffix
/// at all — a hand-named document from outside `~/.rivet/provision`, where
/// there is no convention to follow and `--staging PATH` is how a caller
/// supplies one instead.
#[must_use]
pub(crate) fn staging_path_beside(document_path: &Path) -> Option<PathBuf> {
    const DOCUMENT_SUFFIX: &str = ".provisioning.toml";
    const STAGING_SUFFIX: &str = ".staged.txt";
    let name = document_path.file_name()?.to_str()?;
    let stem = name.strip_suffix(DOCUMENT_SUFFIX)?;
    Some(document_path.with_file_name(format!("{stem}{STAGING_SUFFIX}")))
}

/// **Refuse a server or member document with no staged credentials.**
///
/// `cmd/rivet-provision`'s own interview already writes this bundle before
/// `w` succeeds, so an interactive run never trips this. What it actually
/// catches is a hand-composed `--document PATH` that skips the interview: a
/// document a card is about to be written from that enables a fleet server's
/// or member's services with no credentials on the card for them.
///
/// `doc` is checked with [`requires_staging`] first: a document with no fleet
/// role (a stand-alone machine) needs none of this, and
/// `Ok(None)` says so. Otherwise the staging bundle beside `document_path` (or
/// `staging_override`, when the caller already knows where it is) must exist
/// and must parse as `rivet_core::staging::Staged` bytes — the same read-back
/// check `cmd/rivet-provision`'s own `w` key already performs on the file it
/// just wrote (`SPEC-05` §4).
///
/// # Errors
///
/// The document has a fleet role and no staging bundle is
/// found or it does not parse — named, with the path this looked at and what
/// produces one.
pub(crate) fn require_staging(
    document_path: &Path,
    doc: &rivet_config::Document,
    staging_override: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    if !requires_staging(doc) {
        return Ok(None);
    }
    let candidate = staging_override.map(Path::to_path_buf).or_else(|| staging_path_beside(document_path));
    let Some(path) = candidate else {
        return Err(format!(
            "NOTHING WAS WRITTEN. {} enables a fleet role whose services are handed material \
             from the card (ADR-060 §5) and this run cannot locate its staging bundle — it is not named \
             `<host>.provisioning.toml`, the convention cmd/rivet-provision writes, so there is \
             no `<host>.staged.txt` to look beside it. Pass --staging PATH naming the bundle \
             cmd/rivet-provision composed beside the document it wrote.",
            document_path.display(),
        ));
    };
    let bytes = std::fs::read(&path).map_err(|e| {
        format!(
            "NOTHING WAS WRITTEN. {} enables a fleet role whose services are handed material \
             from the card (ADR-060 §5), and its staging bundle is not at {}: {e}. Run the interview through \
             to `w` — cmd/rivet-provision composes this beside the document — or pass \
             --staging PATH.",
            document_path.display(),
            path.display(),
        )
    })?;
    let text = std::str::from_utf8(&bytes).map_err(|e| {
        format!("{} is not valid UTF-8, so it is not a staging bundle: {e}", path.display())
    })?;
    rivet_core::staging::parse(text).map_err(|e| {
        format!(
            "{} does not parse as a staging bundle: {e:?}. A server or member card written from \
             it would carry no credentials for the services its role enables. Run the interview \
             again through to `w`.",
            path.display()
        )
    })?;
    Ok(Some(path))
}

/// Decision 4: without a terminal, `cargo rivet provision` names the agent
/// path and exits non-zero — checked BEFORE the interview runs, not inside
/// it, so an agent gets one sentence rather than a subprocess that itself
/// refuses the same way one layer down.
const NO_TERMINAL_REFUSAL: &str = "\
cargo rivet provision has no terminal to run the interview at, so nothing was composed and \
nothing was written. An agent should compose a document directly -- rivet_config::Document::parse \
validates it, the same parser a board refuses with -- and pass it with --document PATH: \
`cargo rivet provision --document PATH --image <digest-or-path> [--platform TOKEN] [--device PATH] \
[confirm]` makes the same write this command would have made, skipping the interview entirely. \
--image names a fleet-repository digest (or an unambiguous prefix) or a real file directly -- a \
menu needs a terminal, so an agent must name one. `cargo rivet write-image` is the same verb this \
command calls once a document exists, callable directly.";

pub fn usage() -> String {
    "usage: cargo rivet provision [--document PATH | --existing HOSTNAME]\n\
     \x20                              [--image PATH-OR-DIGEST] [--platform TOKEN]\n\
     \x20                              [--device PATH] [--staging PATH]\n\
     \x20                              [--identity PATH | --pin HEX]\n\
     \x20                              [--at DIR] [--not-mine] [confirm] [--verify-only]\n\
     \n\
     The one command SPEC-05 asks for: preflight, an image selected from the fleet\n\
     repository, the interview, then the write -- a finished card or a named refusal,\n\
     nothing in between.\n\
     \n\
     \x20 1  preflight        credentials -- checked before the interview runs. A missing\n\
     \x20                     fleet repository is offered (`cargo rivet fleet init`'s own\n\
     \x20                     work, declinable) the way a missing CA already is\n\
     \x20 2  image selection  the platform (asked when the repository holds more than one,\n\
     \x20                     used silently and named on screen when it holds one), then\n\
     \x20                     that platform's newest selectable image, verified and\n\
     \x20                     decompressed -- BEFORE the interview runs, so a refusal here\n\
     \x20                     (no images, an unknown platform, nothing this build's schema\n\
     \x20                     can provision) costs no typing (`.rulings/` R-178)\n\
     \x20 3  the interview    cmd/rivet-provision, run as it always has been --\n\
     \x20                     skipped by --document PATH or --existing HOSTNAME\n\
     \x20 4  a staging check  a server or member document must have an ADR-049 staging\n\
     \x20                     bundle beside it before anything is written\n\
     \x20 5  an enrollment    a server or member's own pin, derived id and hostname must\n\
     \x20     check           not already be in the operator's register -- checked before\n\
     \x20                     the write, not after (R-176 Q10); a stand-alone document, or\n\
     \x20                     --existing HOSTNAME's redirect route, needs none of this\n\
     \x20 6  the write        `cargo rivet write-image`, called directly rather than a\n\
     \x20                     second copy of it, with the image step 2 selected. With\n\
     \x20                     a terminal and no --device, the removable devices are\n\
     \x20                     offered as a menu, the plan is printed and `Write it?\n\
     \x20                     [y/N]` is asked (`confirm` skips that question); only y\n\
     \x20                     asks Windows for administrator. N, or no removable\n\
     \x20                     device, writes and records nothing and names the\n\
     \x20                     --document ... confirm command that writes this card\n\
     \x20                     later without the interview\n\
     \x20 7  recording        ONLY once a device was actually written and verified, and\n\
     \x20                     NEVER on --existing HOSTNAME's route: the document is\n\
     \x20                     committed to the fleet repository and, for a server or\n\
     \x20                     member, the register gains the record step 5 planned\n\
     \x20                     (`.rulings/` R-178 P4/Q7/Q10)\n\
     \n\
     See `cargo rivet write-image --help` for what every flag below does once an\n\
     image and a document both exist; this adds ten -- and widens what --at DIR\n\
     means beyond that other command's own words:\n\
     \n\
     \x20 --document PATH   skip the interview -- a document already composed, by an\n\
     \x20                   agent or an earlier run of this command or of\n\
     \x20                   cmd/rivet-provision itself\n\
     \x20 --existing HOSTNAME\n\
     \x20                   write HOSTNAME's card again WITHOUT provisioning it again\n\
     \x20                   (`.rulings/` R-180) -- takes the document already recorded\n\
     \x20                   for HOSTNAME in the fleet repository (committed only after an\n\
     \x20                   earlier, verified write), runs the SAME image selection (step\n\
     \x20                   2) as any other run, and writes and verifies the card. Runs no\n\
     \x20                   interview, mints nothing, enrolls nothing, commits nothing.\n\
     \x20                   A hostname with no recorded document is refused by name.\n\
     \x20                   WITH --document PATH it RE-ROLES the machine instead (ADR-063\n\
     \x20                   section 9 step 5): the new document, which must name\n\
     \x20                   HOSTNAME, is written with the machine's own identity and pin;\n\
     \x20                   its staging bundle is built when absent; nothing is recorded\n\
     \x20 --fleet-root FLEET  fleet-root mode for a SERVER document: the card carries a\n\
     \x20                   request, and the machine mints FLEET's root and its own\n\
     \x20                   certificate on first boot (ADR-063 section 4.2). Passed to\n\
     \x20                   `rivet-provision stage` when it builds the bundle\n\
     \x20 --suffixes LIST   that root's constraint, from local, FLEET.home.arpa, FLEET.lan\n\
     \x20 --passkeys FILE   a passkeys file read off the machine's proxy, carried to the\n\
     \x20                   proxy on the new card\n\
     \x20 --image PATH-OR-DIGEST\n\
     \x20                   a real file directly (unchanged), or a fleet-repository digest\n\
     \x20                   or an unambiguous prefix of one -- selected and checked against\n\
     \x20                   the platform and schema exactly like a menu pick. Omitted, the\n\
     \x20                   platform's newest selectable image is offered as a menu (a\n\
     \x20                   terminal is needed only when there is more than one)\n\
     \x20 --platform TOKEN  which platform's images to select from -- needed only when the\n\
     \x20                   fleet repository holds images for more than one and none was\n\
     \x20                   named; a token with no image is refused, naming what does\n\
     \x20 --staging PATH    the ADR-049 staging bundle a server or member role needs,\n\
     \x20                   when it is not named `<host>.provisioning.toml` beside\n\
     \x20                   `<host>.staged.txt` -- cmd/rivet-provision's own naming\n\
     \x20                   convention, which this tool follows without being told\n\
     \x20 --identity PATH   this card's own identity directory (`cargo rivet setup`'s own\n\
     \x20                   output) -- ADR-049 §8.1 derives the register identifier a\n\
     \x20                   server or member is enrolled under from its device.crt. The\n\
     \x20                   interactive interview supplies this on its own; --document\n\
     \x20                   PATH runs no interview and needs one of this or --pin\n\
     \x20 --pin HEX         the same pin, given directly (64 hex characters) rather than\n\
     \x20                   read from a directory -- --identity and --pin both given, the\n\
     \x20                   pin wins\n\
     \x20 --at DIR          governs the WHOLE run, not only the credential check\n\
     \x20                   write-image --help describes: the interview (step 3) reads\n\
     \x20                   its CA, its saved-answers notebook and the staging bundle it\n\
     \x20                   writes from under DIR too (RIVET_AT, set on its own\n\
     \x20                   environment); the fleet repository step 2 lists and step 7\n\
     \x20                   commits into defaults to DIR/fleet when\n\
     \x20                   --fleet-repository/RIVET_FLEET_REPOSITORY is not given; and\n\
     \x20                   step 5's enrollment check is against DIR/register, never the\n\
     \x20                   operator's real register. Omitted, every one of these keeps\n\
     \x20                   its ordinary default -- this flag changes nothing about a\n\
     \x20                   real, unscratched run\n\
     \n\
     A REFUSAL AT ANY STEP STOPS THE SEQUENCE THERE. Nothing is written until every\n\
     step before it has succeeded, and every refusal names the step and what to do\n\
     about it. A server or member already in the register is refused by name, naming\n\
     the --existing HOSTNAME command that redoes its card instead.\n\
     \n\
     WITHOUT A TERMINAL AND WITH NEITHER --document NOR --existing, THIS REFUSES: there\n\
     is nothing to run the interview at, and no interactive prompt is answered by a\n\
     closed stdin. Compose a document directly and pass --document PATH, or call\n\
     `cargo rivet write-image` with one that already exists.\n"
        .to_string()
}

// ---------------------------------------------------------------------------
// R-178 Q7/Q10 — enrolling a server or member into the OPERATOR's register,
// and committing the document, both AFTER a verified write.
// ---------------------------------------------------------------------------

/// 64 hex characters into 32 bytes — a control-plane pin,
/// `rivetos_tls::Pin`'s own shape. A local decoder rather than
/// `rivetos_tls`'s own (private to that crate): this is the one place outside
/// it that needs one, for `--pin HEX` and the breadcrumb's second line alike.
///
/// # Errors
///
/// `text` is not exactly 64 hex characters.
fn decode_pin_hex(text: &str) -> Result<[u8; 32], String> {
    if text.len() != 64 {
        return Err(format!(
            "{text:?} is {} characters; a control-plane pin is exactly 64 hex characters (32 \
             bytes)",
            text.len()
        ));
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        let pair = text.get(i * 2..i * 2 + 2).ok_or_else(|| format!("{text:?} is not hex"))?;
        *byte = u8::from_str_radix(pair, 16)
            .map_err(|_| format!("{text:?} is not hex: {pair:?} is not a hex byte"))?;
    }
    Ok(out)
}

/// `--identity PATH` and `--pin HEX`, taken out of `args` before the rest
/// reaches `write_image::parse` — `--staging`'s own footing: neither flag is
/// `write-image`'s business.
///
/// **Both name the same fact** — this card's own control-plane pin,
/// `ADR-049` §8.1's source for the register identifier Q7 enrolls it under.
/// The interactive interview supplies it on its own (the breadcrumb's second
/// line); a hand-composed `--document PATH` runs no interview and has no
/// other way to give it, which is the case these two flags exist for. Given
/// both, `--pin` wins — it names the fact directly rather than through a
/// file this process would still have to open.
///
/// **Wave C3: also returns the identity DIRECTORY**, when `--identity PATH`
/// named one — `write_image::run`'s widened injection (R-178 item 1) needs
/// the directory itself, not only the pin derived from it, to place
/// `device.seed`/`device.crt`/`peers` onto the card. `--pin HEX` names no
/// directory, so that route leaves it `None` and injection falls back to
/// `credentials::Survey`'s own `BOARD_CREDENTIAL` default.
///
/// The remaining argv, the identity directory (if `--identity` named one),
/// and the pin — named so [`extract_identity_and_pin`]'s own signature stays
/// readable (`clippy::type_complexity`).
type IdentityAndPin = (Vec<String>, Option<PathBuf>, Option<[u8; 32]>);

/// # Errors
///
/// `--identity PATH` is given and `<PATH>/device.crt` cannot be read or
/// carries no Ed25519 public key, or `--pin HEX` is not 64 hex characters.
fn extract_identity_and_pin(args: &[String]) -> Result<IdentityAndPin, String> {
    let mut rest = Vec::with_capacity(args.len());
    let mut identity: Option<PathBuf> = None;
    let mut pin_hex: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--identity" => {
                identity = args.get(index + 1).map(PathBuf::from);
                index += 2;
            }
            "--pin" => {
                pin_hex = args.get(index + 1).cloned();
                index += 2;
            }
            _ => {
                rest.push(args[index].clone());
                index += 1;
            }
        }
    }

    let pin = match (&pin_hex, &identity) {
        (Some(hex), _) => Some(decode_pin_hex(hex)?),
        (None, Some(dir)) => {
            let cert = dir.join(rivetos_tls::CERTIFICATE_FILE);
            let der = std::fs::read(&cert).map_err(|e| {
                format!(
                    "cannot read {}: {e}. --identity should name the directory `cargo rivet \
                     setup` wrote (device.seed and device.crt).",
                    cert.display()
                )
            })?;
            Some(rivetos_tls::public_key_in_certificate(&der).ok_or_else(|| {
                format!("{} carries no Ed25519 public key, so it is not a device certificate", cert.display())
            })?)
        }
        (None, None) => None,
    };
    Ok((rest, identity, pin))
}

/// Which of `ADR-050` §5.1's roles a composed document names, read the way
/// `Document::carried_services` reads its own answer — off the services the
/// document actually enables, rather than a `fleet-role` key
/// `cmd/rivet-provision`'s own `compose` never writes (that key states no
/// default this document composes without contradicting, so it is left out
/// entirely; see that crate's own tests pinning the exact bytes `compose`
/// produces).
///
/// `None` for a stand-alone machine, or a kiosk with no fleet role — neither
/// is enrolled (R-178 Q7: "for a server or member").
/// **Enroll a MEMBER into its Fleet's server over the control plane** —
/// `ADR-059` §6, the server half of "provisioning is the ceremony". Reads the
/// Armada Registry to find the Fleet this workstation manages, then sends
/// `apply enroll-member` to its server with the operator's identity.
///
/// **Everything here is REPORTED, never fatal.** The card is written and the
/// local register updated before this runs; a member that did not reach the
/// server register is a follow-up an operator fixes, not a failed provision. A
/// registry with no Fleet, or with several and no way yet to pick one (the
/// interactive selector is a later phase), is reported and skipped rather than
/// guessed.
fn enroll_member_into_fleet(device: &rivetos_register::Device) -> String {
    let root = match crate::armada::default_dir() {
        Ok(root) => root,
        Err(why) => {
            return format!("  fleet enroll  SKIPPED — could not locate the Armada Registry: {why}\n");
        }
    };
    let armada = match rivetos_register::armada::store::load(&root) {
        Ok(armada) => armada,
        Err(why) => {
            return format!(
                "  fleet enroll  SKIPPED — could not read the Armada Registry at {}: {why}\n",
                root.display()
            );
        }
    };
    // 0 / 1 / many, without unwrap: a single Fleet is the one this member joins;
    // several is the selector's job, which is not built yet.
    let mut fleets = armada.fleets.values();
    let fleet = match (fleets.next(), fleets.next()) {
        (None, _) => {
            return format!(
                "  fleet enroll  SKIPPED — the Armada Registry holds no Fleet to enroll {} into. \
                 Add one with `cargo rivet armada`, then enroll this member.\n",
                device.name,
            );
        }
        (Some(only), None) => only,
        (Some(_), Some(_)) => {
            return format!(
                "  fleet enroll  SKIPPED — the Armada holds more than one Fleet and this build \
                 cannot yet pick which one {} joins (the interactive selector is a later phase). \
                 Enroll it into the right Fleet's server by hand.\n",
                device.name,
            );
        }
    };
    let Some(identity) = crate::device::default_identity_dir() else {
        return "  fleet enroll  SKIPPED — no operator identity directory to present to the fleet \
                server.\n"
            .to_string();
    };
    let endpoint = rivet_remote::endpoint::Endpoint {
        host: Some(fleet.server.host.clone()),
        identity: Some(identity),
        pin: Some(fleet.server.pin.clone()),
    };
    match crate::enroll::enroll_member(&endpoint, device) {
        Ok(_) => format!(
            "  fleet enroll  {} is enrolled into Fleet {:?} — it appears in that server's \
             member list on its next read.\n",
            device.name, fleet.name,
        ),
        Err(why) => format!(
            "  fleet enroll  FAILED to enroll {} into Fleet {:?}: {why}\n\
            \x20               The card is written and the local register updated; re-run the \
             enroll once the cause is fixed.\n",
            device.name, fleet.name,
        ),
    }
}

pub(crate) fn role_of(doc: &rivet_config::Document) -> Option<rivetos_register::FleetRole> {
    let enabled =
        |name: &str| doc.service(name).is_some_and(rivet_config::services::Service::enabled);
    if enabled(FLEETD_SERVICE) {
        Some(rivetos_register::FleetRole::Server)
    } else if enabled(PROXYD_SERVICE) {
        Some(rivetos_register::FleetRole::Member)
    } else {
        None
    }
}

/// The `Device` record Q7 enrolls — `cmd/rivet-fleet`'s own `add_device`
/// construction, except the identifier is derived from `pin`
/// (`ADR-049` §8.1) rather than minted at random, because THIS record is not
/// a fresh enrollment an operator is naming — it is the same board's own
/// document saying what it already is.
///
/// **R-180.1: a static address is recorded, so the fleet can reach the
/// machine straight away.** Mirrors `cmd/rivet-fleet`'s own `add document`
/// loss-repair shape for `["network", interface, field]`
/// (`register::Interface { method, address, gateway }`, keyed by interface
/// name) rather than inventing a second one. **A DHCP interface records
/// nothing, as today** — the maintainer's own words, and consistent with
/// [`rivetos_register::Device::listen`]'s own "absent means the default"
/// rule: there is nothing here for the register to remember about a leased
/// interface until it has an address, which is the point `ADR-034` §9's
/// `Device::address` already documents.
/// **A fleet server's own fleet-operator public key, out of its staged
/// bundle.** `cmd/rivet-provision`'s `stage::register` writes the server's
/// operator credential as the staged register's ONE `fleet.control_plane_keys`
/// entry (`reg.fleet.control_plane_keys = vec![operator_key]`), staged under
/// the `rivet-fleetd` application at `register/register.toml` — the layout that
/// crate's `stage.rs` composes (`REGISTER_DIR`/`rivetos_register::REGISTER_FILE`).
///
/// Reading it back here is what carries the key from the moment a server is
/// provisioned onto its workstation register record, so the armada can later
/// hand it to a member joining this fleet whose peers must name it or the server
/// can list the member but never manage it (`docs/PLAN.md` §183.2).
///
/// **`None` for a bundle that does not parse, carries no staged register, or
/// names no key**, so a server record whose bundle is unavailable keeps `operator_key`
/// unset — exactly as before this existed. There is only ever one key to take
/// (a fleet server stages its own operator credential and no other), so
/// [`Iterator::next`] is unambiguous.
fn operator_key_from_staged_bundle(bundle: &str) -> Option<String> {
    let items = rivet_core::staging::parse(bundle).ok()?;
    // The path `stage::register` renders — `REGISTER_DIR` ("register") joined to
    // `rivetos_register::REGISTER_FILE` — staged under the fleetd application.
    let register_path = format!("register/{}", rivetos_register::REGISTER_FILE);
    let register = items
        .iter()
        .find(|item| item.application == FLEETD_SERVICE && item.path == register_path)?;
    let text = std::str::from_utf8(&register.bytes).ok()?;
    let fleet: rivetos_register::Fleet = toml_edit::de::from_str(text).ok()?;
    fleet.control_plane_keys.into_iter().next()
}

fn enrollment_device(
    hostname: &str,
    doc: &rivet_config::Document,
    role: rivetos_register::FleetRole,
    pin: &[u8; 32],
    staged: Option<&str>,
) -> rivetos_register::Device {
    let mut device = rivetos_register::Device {
        id: rivet_core::identifier::from_entropy(pin),
        name: hostname.to_owned(),
        fleet_role: Some(role),
        pin: Some(rivetos_tls::encode_hex(pin)),
        listen: Some(doc.effective_control_plane().listen().to_owned()),
        // **A SERVER's own fleet-operator key, off its staged bundle** — the
        // capture half of `docs/PLAN.md` §183.2. A member or stand-alone record
        // never carries one (it runs no `rivet-fleetd` and stages no such
        // register), which is why this is gated on the role, not just on the
        // bundle happening to hold a key.
        operator_key: (role == rivetos_register::FleetRole::Server)
            .then(|| staged.and_then(operator_key_from_staged_bundle))
            .flatten(),
        ..rivetos_register::Device::default()
    };
    // **Only a pin a card can honour.** A `[carry]` pin naming an image service
    // — the proxy, since `ADR-060` — names something no card carries, so the
    // record would hold a pin nothing reads and every later reader would have
    // to know to ignore it. Documents recorded before the proxy joined the
    // image still hold one.
    for carry in doc.carry() {
        let product = carry.product().as_str();
        if rivet_core::service::IMAGE_SERVICE_NAMES.contains(&product) {
            continue;
        }
        device.carry.insert(product.to_owned(), carry.version().to_owned());
    }
    for interface in doc.network() {
        if let rivet_config::Addressing::Static { address, gateway } = interface.addressing() {
            device.interfaces.insert(
                interface.name().as_str().to_owned(),
                rivetos_register::Interface {
                    method: "static".to_owned(),
                    address: Some(address.to_string()),
                    gateway: gateway.map(|g| g.to_string()),
                },
            );
        }
    }
    device
}

/// **The refusal for an identity another machine's record already carries** —
/// `.rulings/` R-189 D2. `cmd/rivet-provision`'s `w` key prints the same
/// sentence, so the interview and `--document --identity` refuse alike.
fn pin_collision_refusal(
    hostname: &str,
    holder: &rivetos_register::Device,
    register_root: &Path,
) -> String {
    format!(
        "NOTHING WAS WRITTEN. The identity given for {hostname} is already {:?}'s (register id \
         {}, in {}): its pin is on that record. Two machines holding one private key answer for \
         each other. Pick or mint another identity directory: the interview mints a fresh \
         identity at `w` in a directory that holds none.",
        holder.name,
        holder.id,
        register_root.display(),
    )
}

/// **R-180: the redirect a duplicate refusal names in full.** Writing an
/// already-provisioned hostname's card again is `--existing HOSTNAME`, never
/// provisioning it a second time — the redirect the ruling asked a duplicate
/// refusal to offer (`.rulings/` R-180).
fn redirect_for(hostname: &str) -> String {
    format!(
        "To write {hostname}'s card again WITHOUT provisioning it again: `cargo rivet provision \
         --existing {hostname:?} [--image PATH] [--device PATH] [confirm]` (`.rulings/` R-180) — it \
         takes the document already recorded for {hostname}, lets you pick an image, and writes and \
         verifies the card. No interview, no re-enrollment."
    )
}

// ---------------------------------------------------------------------------
// R-180: `--existing HOSTNAME` — the redirect itself.
// ---------------------------------------------------------------------------

/// `rivetos_fleet_repo`'s own `safe_name`, restated. That crate exports no
/// function for turning a hostname back into the file name it recorded a
/// document under — only the constants (`DOCUMENTS_DIR`, `DOCUMENT_SUFFIX`)
/// — so this is the THIRD copy of the algorithm,
/// `cmd/rivet-provision/src/save.rs`'s own copy being the second, for that
/// crate's identical "a bin-only crate has nothing to export" reason.
fn safe_name(name: &str) -> String {
    let safe: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect();
    if safe.is_empty() { "unnamed".to_owned() } else { safe }
}

/// What locates an already-provisioned hostname's recorded document and
/// makes it available at a real path — injected so [`run_with`]'s R-180
/// route (no interview, no enrollment, no commit) is provable with an unknown
/// hostname and no real fleet repository.
trait ExistingDocument {
    /// # Errors
    ///
    /// No fleet repository could be resolved, or `hostname` names nothing
    /// recorded there — see [`RealExistingDocument::locate`].
    fn locate(&self, hostname: &str) -> Result<PathBuf, String>;
}

/// The real lookup — `rivetos_fleet_repo::DOCUMENTS_DIR`'s own naming, the
/// ONLY place a document is committed after R-178 P4, so it is the one
/// answer to "has this hostname already been provisioned" that a verified
/// write, and only a verified write, ever produces.
///
/// **Never `~/.rivet/provision/` as the SOURCE.** A document can sit there
/// with no committed counterpart at all — an abandoned notebook, or a
/// composed document that was never carried through to a verified write —
/// and reading from it would let `--existing` "redo" a card R-174's own
/// acceptance test says was never finished provisioning in the first place.
///
/// The recorded text is then written to the SAME local path
/// `cmd/rivet-provision`'s own `w` key would have used —
/// `<at>/provision/<host>.provisioning.toml` — so [`RealStagingCheck`]'s
/// existing naming convention finds the ORIGINAL staging bundle beside it,
/// on the workstation that ran the interview. On a different one, that
/// check refuses by name, exactly what R-180 asks for ("locate it where
/// provisioning left it, or refuse by name saying what is missing") rather
/// than a second lookup invented for this route alone.
struct RealExistingDocument<'a> {
    fleet_repo_root: Option<&'a Path>,
    at: &'a Path,
}

impl ExistingDocument for RealExistingDocument<'_> {
    fn locate(&self, hostname: &str) -> Result<PathBuf, String> {
        let Some(root) = self.fleet_repo_root else {
            return Err(format!(
                "NOTHING WAS WRITTEN. cargo rivet provision --existing {hostname:?} needs the fleet \
                 repository to find its recorded document in, and none could be found. Set \
                 RIVET_FLEET_REPOSITORY, or create one at the default location with `cargo rivet \
                 fleet init`."
            ));
        };
        let recorded = root
            .join(rivetos_fleet_repo::DOCUMENTS_DIR)
            .join(format!("{}{}", safe_name(hostname), rivetos_fleet_repo::DOCUMENT_SUFFIX));
        let text = std::fs::read_to_string(&recorded).map_err(|e| {
            format!(
                "NOTHING WAS WRITTEN. cargo rivet provision --existing {hostname:?} found no recorded \
                 document at {}: {e}. Only a hostname whose card has already been written and verified \
                 has one (`.rulings/` R-180) — check the spelling, or run `cargo rivet provision` \
                 without --existing to provision it for the first time.",
                recorded.display()
            )
        })?;

        let local =
            self.at.join("provision").join(format!("{}{}", safe_name(hostname), rivetos_fleet_repo::DOCUMENT_SUFFIX));
        if let Some(dir) = local.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        std::fs::write(&local, &text).map_err(|e| format!("cannot write {}: {e}", local.display()))?;
        Ok(local)
    }
}

/// **The identity a recorded machine was provisioned with**, when it has its
/// own: `<at>/identity/<hostname>`, the interview's default since `.rulings/`
/// R-189 D2. `None` when that directory holds no certificate, so the writer
/// falls back to the board identity every earlier machine was provisioned
/// with.
fn existing_identity_dir(at: &Path, hostname: &str) -> Option<PathBuf> {
    let dir = at.join("identity").join(safe_name(hostname));
    dir.join(rivetos_tls::CERTIFICATE_FILE).is_file().then_some(dir)
}

/// `--existing HOSTNAME`, taken out of `args` before the rest reaches
/// `write_image::parse` — `--staging`'s own footing.
fn extract_existing(args: &[String]) -> (Vec<String>, Option<String>) {
    let mut rest = Vec::with_capacity(args.len());
    let mut existing = None;
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--existing" {
            existing = args.get(index + 1).cloned();
            index += 2;
        } else {
            rest.push(args[index].clone());
            index += 1;
        }
    }
    (rest, existing)
}

/// **What plans and, later, records a server or member's enrollment and the
/// document's commit** — injected on [`Preflight`]/[`Interview`]/
/// [`StagingCheck`]'s own footing, so [`run_with`]'s ordering (a duplicate
/// refused BEFORE the write; a commit and an enrollment only AFTER it) is
/// provable with fakes touching no real fleet repository and no real
/// register.
trait FleetRecorder {
    /// **Q10: called before the write.** Reads `document_path`, decides
    /// [`role_of`] it, and — for a server or member — refuses if `pin` is
    /// absent or if the id it derives, or the document's own hostname, is
    /// already in the operator's register. `Ok(None)` for a stand-alone
    /// document: nothing to enroll, nothing to check.
    ///
    /// # Errors
    ///
    /// The document cannot be read or does not parse; a server or member
    /// document with no `pin` available at all; or a duplicate — named,
    /// against the register's own record.
    fn plan(
        &self,
        document_path: &Path,
        pin: Option<&[u8; 32]>,
    ) -> Result<Option<rivetos_register::Device>, String>;

    /// **Called only after `Writer::write` has returned `Ok`, on a route that
    /// targeted a device** (R-174: a "finished card", not merely a prepared
    /// image). Commits `document_path`'s bytes to the fleet repository
    /// (`rivetos_fleet_repo::record_document`) and, if `plan` is `Some`,
    /// enrolls the device and projects the register
    /// (`rivetos_fleet_repo::record_register`). Best-effort throughout, on
    /// `rivetos_fleet_repo::Outcome::Refused`'s own footing: a card that was
    /// just written and verified must go on being reported as written
    /// whether or not either of these second copies could be made.
    fn commit(&self, document_path: &Path, plan: Option<rivetos_register::Device>) -> String;
}

/// The real recorder — the fleet repository root R-178 P3 already resolved
/// (`None` when this host has neither `HOME` nor `USERPROFILE`), and the
/// operator's register root, `crate::device::default_register_dir`'s own
/// answer in real use, or a scratch directory in a test — see [`Preflight`]'s
/// own `at` field for the identical shape.
struct RealFleetRecorder<'a> {
    fleet_repo_root: Option<&'a Path>,
    register_root: &'a Path,
    /// **The Armada a freshly enrolled fleet SERVER is added to** —
    /// `<at>/armada`, the directory the interview reads a member's fleet
    /// server key from. `None` adds nothing: a test that is not about it.
    /// [`crate::armada::bootstrap_provisioned_server`] says when it adds and
    /// when it declines.
    armada_root: Option<&'a Path>,
}

impl RealFleetRecorder<'_> {
    /// The document, read once, parsed once — `plan` and `commit` each need
    /// this and neither is handed the bytes by `run_with`, which never opens
    /// the document itself (see that function's own reasoning: every I/O
    /// this module performs lives behind one of these traits).
    fn read(&self, document_path: &Path) -> Result<(String, rivet_config::Document), String> {
        let text = std::fs::read_to_string(document_path)
            .map_err(|e| format!("cannot read {}: {e}", document_path.display()))?;
        let doc = rivet_config::Document::parse(&text)
            .map_err(|e| format!("{} does not parse: {e:?}", document_path.display()))?;
        Ok((text, doc))
    }
}

impl FleetRecorder for RealFleetRecorder<'_> {
    fn plan(
        &self,
        document_path: &Path,
        pin: Option<&[u8; 32]>,
    ) -> Result<Option<rivetos_register::Device>, String> {
        let (_, doc) = self.read(document_path)?;
        let Some(role) = role_of(&doc) else {
            return Ok(None);
        };
        let hostname = doc.hostname().map_or_else(|| "unnamed".to_owned(), ToString::to_string);

        let Some(pin) = pin else {
            return Err(format!(
                "NOTHING WAS WRITTEN. {hostname} is a fleet {role} and ADR-049 §8.1 derives its \
                 register identifier from its own control-plane pin, which this run could not \
                 find. Pass --identity PATH (the directory `cargo rivet setup` wrote for this \
                 card) or --pin HEX, or run the interactive interview, which supplies one \
                 automatically."
            ));
        };
        // **The server's staged bundle, beside its document** — best-effort, so
        // its own fleet-operator key can be recorded on its register record
        // (`docs/PLAN.md` §183.2). A bundle that is not at the conventional
        // path (a `--staging` override under a hand-named document) simply
        // yields no key here, which leaves the record exactly as it was before
        // this existed; capturing the key is a convenience for a later member's
        // enroll, never something a provision run fails for.
        let staged =
            staging_path_beside(document_path).and_then(|path| std::fs::read_to_string(path).ok());
        let device = enrollment_device(&hostname, &doc, role, pin, staged.as_deref());

        let register = rivetos_register::store::load(self.register_root).map_err(|e| {
            format!("cannot read the register at {}: {e}", self.register_root.display())
        })?;
        if let Some(existing) = register.devices.get(&device.id) {
            return Err(format!(
                "NOTHING WAS WRITTEN. {hostname}'s own identifier ({}) is already enrolled in \
                 the register at {} as {:?}. ADR-049 §8.1 derives this identifier from the \
                 card's own pin — either this is the same machine, already enrolled, or its \
                 identity does not match what was expected. {}",
                device.id,
                self.register_root.display(),
                existing.name,
                redirect_for(&hostname),
            ));
        }
        // **One identity per machine** (`.rulings/` R-189 D2). The identifier
        // check above misses a record enrolled by `rivet-fleet add device`,
        // whose identifier was minted at random rather than derived from its
        // pin (`ADR-049` §8).
        if let Some(existing) =
            register.pin_held_elsewhere(&rivetos_tls::encode_hex(pin), &hostname)
        {
            return Err(pin_collision_refusal(&hostname, existing, self.register_root));
        }
        if let Some(existing) = register.devices.values().find(|d| d.name == hostname) {
            return Err(format!(
                "NOTHING WAS WRITTEN. {hostname:?} is already enrolled in the register at {} \
                 (as {}). Two devices under one name is more often a repeated run than an \
                 intention (ADR-025 §4.1) — remove the existing record first, or give this \
                 machine a different hostname. {}",
                self.register_root.display(),
                existing.id,
                redirect_for(&hostname),
            ));
        }
        Ok(Some(device))
    }

    fn commit(&self, document_path: &Path, plan: Option<rivetos_register::Device>) -> String {
        let mut report = String::new();
        match (self.fleet_repo_root, self.read(document_path)) {
            (Some(root), Ok((text, doc))) => {
                let hostname =
                    doc.hostname().map_or_else(|| "unnamed".to_owned(), ToString::to_string);
                report.push_str(&rivetos_fleet_repo::record_document(root, &hostname, &text).to_string());
                report.push('\n');
            }
            (None, _) => report.push_str("fleet repository: NOT recorded — no repository root resolved\n"),
            (Some(_), Err(e)) => {
                report.push_str(&format!("fleet repository: NOT recorded — {e}\n"));
            }
        }

        let Some(device) = plan else {
            return report;
        };
        let server = (device.fleet_role == Some(rivetos_register::FleetRole::Server)).then(|| device.clone());
        match rivetos_register::store::load(self.register_root) {
            Ok(mut register) => match register.enroll(device) {
                Ok(()) => match rivetos_register::store::save(&register) {
                    Ok(_) => {
                        report.push_str(&format!("register: enrolled in {}\n", self.register_root.display()));
                        if let Some(root) = self.fleet_repo_root {
                            report.push_str(&rivetos_fleet_repo::record_register(root, &register).to_string());
                            report.push('\n');
                        }
                        // **A fresh fleet server becomes the Armada's Fleet**,
                        // so the member provisioned next is staged trusting
                        // its `rivet-fleetd` operator key and is enrolled into
                        // it. Without this a member of a server provisioned
                        // into an empty Armada is staged trusting the human
                        // operator alone, and the server can list it and
                        // manage nothing on it.
                        if let (Some(server), Some(armada)) = (&server, self.armada_root) {
                            report.push_str(&crate::armada::bootstrap_provisioned_server(armada, server));
                        }
                    }
                    Err(e) => report.push_str(&format!("register: NOT saved — {e}\n")),
                },
                Err(e) => report.push_str(&format!("register: NOT enrolled — {e}\n")),
            },
            Err(e) => report.push_str(&format!(
                "register: NOT enrolled — cannot read {}: {e}\n",
                self.register_root.display()
            )),
        }
        report
    }
}

/// `--staging PATH`, taken out of `args` before the rest reaches
/// `write_image::parse` — that parser refuses any flag it does not itself
/// know, and this one is `provision`'s alone (`write-image` never checks a
/// document for a staging bundle, so it has no use for one).
fn extract_staging(args: &[String]) -> (Vec<String>, Option<PathBuf>) {
    let mut rest = Vec::with_capacity(args.len());
    let mut staging = None;
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--staging" {
            staging = args.get(index + 1).map(PathBuf::from);
            index += 2;
        } else {
            rest.push(args[index].clone());
            index += 1;
        }
    }
    (rest, staging)
}

pub fn run(args: &[String]) -> Result<String, String> {
    let (rest, staging_override) = extract_staging(args);
    let (rest, existing_hostname) = extract_existing(&rest);
    let (rest, requested_platform) = extract_platform(&rest);
    let (rest, stage_extra) = extract_stage_extras(&rest)?;
    let (rest, explicit_identity_dir, explicit_pin) = extract_identity_and_pin(&rest)?;
    let mut write_args = write_image::parse(&rest)?;

    // **`--existing HOSTNAME --document PATH` re-roles a recorded machine** —
    // `ADR-063` §9 step 5, which makes an existing member the server of a new
    // fleet. The machine keeps its identity, its pin and its register record,
    // nothing is recorded again, and the document is the new one. Only
    // `--existing` alone rewrites the RECORDED document (`.rulings/` R-180).
    let rerole = existing_hostname.is_some() && write_args.document_path.is_some();
    if !stage_extra.is_empty() && existing_hostname.is_some() && !rerole {
        return Err(
            "--fleet-root, --suffixes and --passkeys shape a NEW staging bundle, and --existing \
             HOSTNAME alone rewrites the recorded one. Give the new document with --document PATH \
             as well."
                .to_owned(),
        );
    }

    let at = match &write_args.at {
        Some(p) => p.clone(),
        None => crate::credentials::root().ok_or_else(|| {
            "this host sets neither HOME nor USERPROFILE, so there is nowhere for a Rivet \
             credential to be. Pass --at DIR."
                .to_owned()
        })?,
    };
    // **`--existing HOSTNAME` reuses the recorded machine's own identity** —
    // `.rulings/` R-189 D2. A machine provisioned with its own identity keeps
    // it under its hostname; one provisioned before that has none there, and
    // falls back to the board identity it was provisioned with.
    let explicit_identity_dir = match (&existing_hostname, explicit_identity_dir, explicit_pin) {
        (Some(hostname), None, None) => existing_identity_dir(&at, hostname),
        (_, given, _) => given,
    };
    // **The working directory is NOT a condition on this verb** — `.rulings/`
    // R-192 D3, and R-191 D2. It used to be: a gate here refused every route,
    // `--document`, `--existing` and `--verify-only` included, unless the
    // current directory held `cmd/rivet-provision/Cargo.toml`. None of those
    // three starts an interview at all, and the one that does now finds it
    // beside this binary on any installed host. So the directory is read, and
    // handed on as nothing more than a candidate for `cargo run`'s fallback;
    // [`resolve_interview_source`] decides, at the moment an interview is
    // actually needed, and refuses by naming the install rather than a path.
    let cwd = std::env::current_dir().ok();

    // **R-178 P3/P4**: resolved once, here — the repository the interview
    // lists application versions against (P3), and the one this process
    // commits the final document into after a verified write (P4). The
    // interview is told the same answer via `RIVET_FLEET_REPOSITORY`
    // (`SubprocessInterview::run`) so the two halves of one run never
    // disagree. `at` is this run's own resolved credential root (2026-09-14):
    // the fallback when `RIVET_FLEET_REPOSITORY` is not set, so a `--at`
    // scratch run defaults to a scratch repository rather than the
    // operator's real one — see this module's own "2026-09-14" section.
    let fleet_repo_root = fleet_repository_root(&at);
    // **R-178 Q7**: the operator's own register — the same one
    // `crate::device` already resolves `run`/`deploy <device>` against.
    //
    // **Not resolved at all for `--verify-only`, an internal `--elevated-*`
    // re-invocation, or R-180's `--existing` redirect.** `run_with` never
    // consults the fleet recorder on any of those routes — resolving a
    // register root none of them has a use for would make a route fail on a
    // host where Ignition cannot find a configuration directory, for a flag
    // that was never going to need one.
    //
    // **`write_args.at` — the LITERAL `--at DIR`, not the resolved `at`
    // above.** See this module's own "2026-09-14" section for why the two
    // must not be conflated here: an ordinary, no-`--at` run must keep
    // enrolling into the real register exactly as it always has.
    let register_root = if write_args.verify_only
        || write_args.elevated_write_task.is_some()
        || write_args.elevated_verify_task.is_some()
        || existing_hostname.is_some()
    {
        PathBuf::new()
    } else {
        register_root_for(write_args.at.as_deref())?
    };

    // **Wave C3 (R-178 item 1): the widened injection needs the same fleet
    // repository this process already resolved, to resolve `[carry.<product>]`
    // pins to bytes.** Set once, here, on `ProvisionArgs` itself, rather than
    // re-resolved inside `write_image::run` — one root, read by every module
    // that needs it, `fleet_repository_root`'s own module doc.
    write_args.fleet_repo_root = fleet_repo_root.clone();

    let preflight = RealPreflight { at: &at };
    // **Up to three stage announcements** (R-174.1, cosmetic only): listing
    // images for the platform, verifying and decompressing the one selected
    // (both `RealImageSelector`'s, run only when the repository is actually
    // consulted — see `provision_image.rs`), then the interview. A route
    // that skips one (a literal `--image PATH`, or `--document`/`--existing`
    // skipping the interview) simply never announces it; the numbering is
    // not claimed to be exact across every route, which R-174.1 says is fine.
    let stages = crate::progress::Stages::new(3);
    let interview = SubprocessInterview {
        cwd: cwd.as_deref(),
        stages: Some(&stages),
        fleet_repo_root: fleet_repo_root.as_deref(),
        at: &at,
        register_root: &register_root,
    };
    // **A missing bundle is built** (`ensure_staging`) on every route but
    // `--existing HOSTNAME`'s alone — see `RealStagingCheck::builder`. A
    // re-roled machine's new document gets a new bundle: a new fleet's server
    // has no members trusting an old `rivet-fleetd` key.
    let builder = StagingWith { extra: stage_extra };
    let staging_check = RealStagingCheck {
        staging_override: staging_override.as_deref(),
        at: &at,
        builder: (existing_hostname.is_none() || rerole).then_some(&builder as &dyn StagingBuilder),
    };
    // **`<at>/armada`**: the same directory the interview reads a member's
    // fleet server key from (`RIVET_AT`), and `crate::armada::default_dir`'s
    // own answer on an ordinary run with no `--at`.
    let armada_root = at.join("armada");
    let fleet_recorder = RealFleetRecorder {
        fleet_repo_root: fleet_repo_root.as_deref(),
        register_root: &register_root,
        armada_root: Some(&armada_root),
    };
    let existing_document = RealExistingDocument { fleet_repo_root: fleet_repo_root.as_deref(), at: &at };
    let image_selector = RealImageSelector {
        fleet_repo_root: fleet_repo_root.as_deref(),
        at: &at,
        is_terminal: std::io::stdin().is_terminal(),
        stages: Some(&stages),
    };
    run_with(
        std::io::stdin().is_terminal(),
        &preflight,
        &interview,
        &staging_check,
        &fleet_recorder,
        &RealWriter,
        &image_selector,
        requested_platform.as_deref(),
        write_args,
        explicit_pin,
        explicit_identity_dir,
        existing_hostname.as_deref(),
        &existing_document,
    )
}

/// `--platform TOKEN`, taken out of `args` before the rest reaches
/// `write_image::parse` — that parser refuses any flag it does not itself
/// know, and this one, like `--staging`, is `provision`'s alone
/// (`.rulings/` R-178: which platform's images to select from).
fn extract_platform(args: &[String]) -> (Vec<String>, Option<String>) {
    let mut rest = Vec::with_capacity(args.len());
    let mut platform = None;
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--platform" {
            platform = args.get(index + 1).cloned();
            index += 2;
        } else {
            rest.push(args[index].clone());
            index += 1;
        }
    }
    (rest, platform)
}

/// The fleet repository this process resolves a document commit against —
/// `RIVET_FLEET_REPOSITORY` when set, else `at.join("fleet")` —
/// `cmd/rivet-provision`'s own `fleet_repository_root`, restated: that crate
/// has no `[lib]` target this one could depend on instead, and the two must
/// resolve identically or the interview's version listing and this
/// process's own commit would silently mean two different repositories.
///
/// **`at` is this run's own resolved credential root** (`--at DIR`, or this
/// host's own `credentials::root()` default) — this module's own
/// "2026-09-14" section: falling back to it, rather than a fresh
/// `rivetos_fleet_repo::default_root()` read of `HOME`/`USERPROFILE`, does
/// not change the ordinary answer at all (`credentials::root()` computes
/// the identical `$HOME/.rivet` shape), and makes a `--at`-only run (no
/// `HOME`, no `USERPROFILE`) resolve a repository instead of refusing
/// outright, and a scratch `--at` run default to a scratch repository
/// rather than the operator's real one.
pub(crate) fn fleet_repository_root(at: &Path) -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("RIVET_FLEET_REPOSITORY") {
        let path = PathBuf::from(value);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    Some(fleet_repository_fallback(at))
}

/// [`fleet_repository_root`]'s own answer when `RIVET_FLEET_REPOSITORY` is
/// not set — split out, pure, so "under `--at`" is provable with a path and
/// nothing else, no ambient environment in the loop (this process's own
/// `RIVET_FLEET_REPOSITORY`, unlike `HOME`, is not something a test can rely
/// on being unset).
fn fleet_repository_fallback(at: &Path) -> PathBuf {
    at.join("fleet")
}

/// **The register a `--at DIR` run enrolls into** — this module's own
/// "2026-09-14" section.
///
/// [`crate::device::default_register_dir`] always resolves `rivet-fleet`'s
/// own register through `ignition::config`
/// (`%APPDATA%\rivet-fleet\register` by default), which has no relationship
/// to `HOME`/`USERPROFILE` or to `--at` at all. Left alone, a scratch
/// `--at` run's enrollment (R-178 Q7) would still land in the OPERATOR'S
/// REAL register.
///
/// `explicit_at` is [`ProvisionArgs::at`] — the LITERAL `--at DIR` the
/// caller typed, **never** the resolved credential root ([`run`]'s own
/// `at`, which is `Some` even without `--at`, via `credentials::root`'s own
/// default). Branching on the resolved value instead would silently move
/// every ordinary, no-`--at` provisioning run's register out from under
/// it — real provisioning must keep enrolling into the real,
/// already-populated register exactly as it always has,
/// `RIVET_FLEET_REGISTER`'s own override included.
///
/// # Errors
///
/// Only on the `explicit_at.is_none()` route: whatever
/// [`crate::device::default_register_dir`] itself returns.
fn register_root_for(explicit_at: Option<&Path>) -> Result<PathBuf, String> {
    /// A sibling of `ca`/`provision`/`fleet` under `--at DIR`, on the same
    /// footing.
    const AT_REGISTER_DIR: &str = "register";
    match explicit_at {
        Some(at) => Ok(at.join(AT_REGISTER_DIR)),
        None => crate::device::default_register_dir(),
    }
}

/// The whole sequencing, with the terminal check, preflight, the interview,
/// the staging check, the fleet recorder and the writer all taken as
/// parameters — see the module documentation for why: this is the one
/// function `provision_tests.rs` exercises with fakes, and it is deliberately
/// the ONLY place that decides the order.
///
/// `explicit_pin` is `--identity PATH`/`--pin HEX`, already resolved by the
/// caller — it wins over whatever the interview's own breadcrumb carries,
/// and it is the ONLY source of a pin at all on the `--document PATH` route,
/// which runs no interview and so produces no breadcrumb.
///
/// `existing_hostname` is `--existing HOSTNAME` (R-180) — the caller has
/// already refused a run that gave both this and `--document PATH`, so this
/// function's own precondition is that at most one of `args.document_path`
/// and `existing_hostname` is `Some`. When it is, the whole document/
/// preflight/interview/enrollment machinery above is bypassed: `existing_document`
/// locates the recorded document, `staging_check` still runs against it (R-180:
/// "locate [staging material] where provisioning left it, or refuse by name"),
/// and neither `fleet_recorder.plan` nor `.commit` is ever called — no
/// interview, no re-enrollment, no second commit for a card that is already on
/// record.
///
/// **Twelve parameters, and each one is a seam a test above stands on** —
/// this function's whole reason to exist is being callable with fakes for
/// every dependency and nothing bundled into a struct that would hide which
/// one changed between two tests. `cmd/rivet-deploy/src/install.rs` and
/// `cmd/rivet-fleet/src/tui/device.rs` take the identical trade for the
/// identical reason.
///
/// **Wave C2 (R-178 items 1–8): `image_selector` runs first, and its scratch
/// working copy — when it made one — is removed once this returns, success
/// or failure.** `run_with` itself is now a thin wrapper for exactly that
/// cleanup guarantee; [`run_with_image_selected`] is the sequencing this
/// function delegated to before this wave and still does, taking the same
/// arguments minus `image_selector`/`requested_platform` and with
/// `args.image_path` already the concrete file [`ImageSelector::select`]
/// produced.
#[allow(clippy::too_many_arguments)]
fn run_with(
    is_terminal: bool,
    preflight: &dyn Preflight,
    interview: &dyn Interview,
    staging_check: &dyn StagingCheck,
    fleet_recorder: &dyn FleetRecorder,
    writer: &dyn Writer,
    image_selector: &dyn ImageSelector,
    requested_platform: Option<&str>,
    mut args: ProvisionArgs,
    explicit_pin: Option<[u8; 32]>,
    explicit_identity_dir: Option<PathBuf>,
    existing_hostname: Option<&str>,
    existing_document: &dyn ExistingDocument,
) -> Result<String, String> {
    // `--verify-only` and the two internal `--elevated-*` re-invocations need
    // no document, no preflight, no interview, no staging check, no image
    // selection and no enrollment planning at all — `write_image::run` handles
    // every one of them itself, before it ever looks at `document_path` or
    // `image_path`. Checked first so none of them can be made to wait on a
    // terminal they do not need.
    if args.verify_only
        || args.elevated_write_task.is_some()
        || args.elevated_verify_task.is_some()
    {
        return writer.write(&args).map(|outcome| outcome.report);
    }

    // **R-178 items 2–7, P3: selection happens before the interview, and
    // identically on R-180's `--existing` route (item 8).** A refusal here
    // (no fleet-repository content for any platform, an unknown platform,
    // nothing this build's schema can provision) costs the operator no
    // typing — nothing below this point has run yet.
    let requested_image = args.image_path.clone();
    // `Ok(None)` is the operator leaving the platform or system-image menu
    // with `x`/`X` — see `ImageSelector::select`'s own doc. The run ends
    // right here, cleanly (exit status 0, not a refusal), before the
    // interview, the staging check or the writer have run at all.
    let Some(selected) = image_selector.select(requested_platform, requested_image.as_deref())?
    else {
        return Ok(
            "exited — no system image was chosen, and nothing else has run: no interview, no \
             staging check, no write.\n"
                .to_string(),
        );
    };
    args.image_path = Some(selected.path.clone());
    args.image_from_repository = selected.scratch.is_some();
    // What a later rerun names as `--image`: the digest, or the literal file.
    // Never the scratch copy, which is removed below.
    let rerun_image = selected
        .digest
        .clone()
        .or_else(|| selected.scratch.is_none().then(|| selected.path.display().to_string()));

    // **Item 6: the scratch working copy is removed once the rest of this
    // run has finished, success or failure alike** — every refusal below
    // this point (preflight, the interview, the staging check, a duplicate
    // enrollment, a declined `[y/N]`, the write itself) goes through this one
    // exit, so there is no path that leaves a decompressed image, or the
    // `image-work-<pid>` directory holding it, behind.
    let outcome = run_with_image_selected(
        is_terminal,
        preflight,
        interview,
        staging_check,
        fleet_recorder,
        writer,
        args,
        explicit_pin,
        explicit_identity_dir,
        existing_hostname,
        existing_document,
        selected.descriptor.clone(),
        rerun_image,
        selected.version.as_deref(),
    );
    if let Some(scratch) = &selected.scratch {
        crate::provision_image::remove_scratch(scratch);
    }
    outcome
}

/// The sequencing `run_with` performed directly before this wave — see that
/// function's own documentation for why it is now split out this way.
#[allow(clippy::too_many_arguments)]
fn run_with_image_selected(
    is_terminal: bool,
    preflight: &dyn Preflight,
    interview: &dyn Interview,
    staging_check: &dyn StagingCheck,
    fleet_recorder: &dyn FleetRecorder,
    writer: &dyn Writer,
    mut args: ProvisionArgs,
    explicit_pin: Option<[u8; 32]>,
    explicit_identity_dir: Option<PathBuf>,
    existing_hostname: Option<&str>,
    existing_document: &dyn ExistingDocument,
    image_descriptor: Option<String>,
    rerun_image: Option<String>,
    image_version: Option<&str>,
) -> Result<String, String> {
    let mut report = String::new();
    let mut pin = explicit_pin;
    let mut identity_dir = explicit_identity_dir;
    // **R-180: the redirect route.** No preflight, no interview — the whole
    // point is that this hostname was provisioned already.
    let document_path = if let Some(hostname) = existing_hostname {
        match args.document_path.clone() {
            // Re-roling a recorded machine: the new document must be for it.
            Some(path) => {
                let named = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|text| rivet_config::Document::parse(&text).ok())
                    .and_then(|doc| doc.hostname().map(ToString::to_string));
                if named.as_deref() != Some(hostname) {
                    return Err(format!(
                        "NOTHING WAS WRITTEN. --existing {hostname} keeps that machine's identity, \
                         and {} names {}. A document written onto a recorded machine must be for \
                         that machine.",
                        path.display(),
                        named.map_or_else(|| "no hostname".to_owned(), |n| format!("{n:?}"))
                    ));
                }
                path
            }
            None => existing_document.locate(hostname)?,
        }
    } else {
        // **The document path, whichever way it arrives** — the interview's
        // own, or `--document PATH`'s already in `args`. Bound here, once,
        // rather than re-extracted from `args.document_path` afterward, so
        // there is no later point in this function that has to assume a
        // `Some` an earlier branch put there.
        match args.document_path.clone() {
            Some(path) => path,
            None => {
                // **Preflight runs before the interview** — never after an
                // operator has already spent ten minutes on it only to be
                // told a credential was missing all along.
                report.push_str(&preflight.check()?);

                if !is_terminal {
                    return Err(NO_TERMINAL_REFUSAL.to_owned());
                }
                let result = interview.run()?;
                args.document_path = Some(result.document_path.clone());
                // An explicit `--pin`/`--identity` still wins even when the
                // interview also supplied one — the flag is what the
                // operator or an agent stated on purpose.
                if pin.is_none() {
                    pin = result.pin;
                }
                if identity_dir.is_none() {
                    identity_dir = result.identity_dir;
                }
                result.document_path
            }
        }
    };
    args.document_path = Some(document_path.clone());
    // **Wave C3 (R-178 item 1)**: `write_image::run`'s widened injection reads
    // this to place `device.seed`/`device.crt`/`peers` — `None` here simply
    // means it falls back to `credentials::Survey`'s own `BOARD_CREDENTIAL`
    // default, which is right whenever the interview (or an agent) never named
    // a different directory.
    args.identity_path = identity_dir;

    // **`ADR-060` §5: the image must carry the proxy a server or member
    // needs.** Checked here, once the document — and so the role — is known,
    // and before anything is built, planned or written: a refusal here leaves
    // no staging bundle minted for a card that was never going to be written.
    // A document this cannot read is refused by the staging check below.
    if let Some(doc) = std::fs::read_to_string(&document_path)
        .ok()
        .and_then(|text| rivet_config::Document::parse(&text).ok())
        && let Some(note) = refuse_an_image_without_the_proxy(&doc, image_version)?
    {
        report.push_str(&note);
    }

    // **Checked on every document, whichever way it arrived** — the
    // interview's own, `--document PATH`'s, or R-180's redirect. An
    // interactive run finds the bundle the interview wrote; a hand-composed
    // document has one BUILT here (`ensure_staging`), and the report says so.
    //
    // **Wave C3**: the resolved path — `Some` whenever this document required
    // one — is handed to the writer too, so injection (item 1) carries the
    // SAME bundle this check just validated rather than re-deriving it.
    let (staging_path, staging_note) = staging_check.check(&document_path, args.identity_path.as_deref())?;
    args.staging_path = staging_path;
    report.push_str(&staging_note);

    // **R-180: no enrollment planning on the redirect route at all.**
    // `existing_hostname` is `Some` only when the hostname is ALREADY
    // enrolled — replanning it would immediately trip the very duplicate
    // check that sent the operator here.
    let plan = if existing_hostname.is_some() {
        None
    } else {
        // **R-178 Q10: checked BEFORE the write, not after.** A duplicate
        // enrollment, or a server/member document with no pin to derive an
        // identifier from, is refused here — nothing has been written yet.
        fleet_recorder.plan(&document_path, pin.as_ref())?
    };

    // **R-174: the bare interactive run ends in a written card.** With a
    // terminal and no `--device`/`confirm`, the writer offers the removable
    // devices as a menu and asks `Write it? [y/N]` rather than stopping at a
    // prepared image. Without a terminal, nothing changes.
    args.offer_device_menu = is_terminal;
    // On an interactive run, the command that writes this same card later
    // without the interview — named whenever this run does not finish one.
    let rerun = if is_terminal && existing_hostname.is_none() {
        Some(rerun_command(&args, pin.as_ref(), rerun_image.as_deref()))
    } else {
        None
    };

    let outcome = match writer.write(&args) {
        Ok(outcome) => outcome,
        Err(why) => {
            return Err(match &rerun {
                Some(command) => format!(
                    "{why}\nNothing was recorded. To write this same card without redoing the \
                     interview:\n  {command}"
                ),
                None => why,
            });
        }
    };

    // **R-178 P4 / R-180: only once a device was actually written and
    // verified, and never on the redirect route at all.** A prepared image,
    // a `N` at `Write it? [y/N]`, a cancelled administrator prompt or no
    // removable device is not a finished card (R-174) — nothing is committed
    // for it. R-180's redirect commits nothing regardless: the document is
    // already on record and enrolling it again is the exact mistake the
    // redirect exists to avoid.
    if !outcome.written {
        let next = match &rerun {
            Some(command) => format!(
                "Nothing was written to a device and nothing was recorded. To write this same \
                 card later without redoing the interview:\n  {command}\n"
            ),
            None => String::new(),
        };
        return Ok(format!("{report}{}{next}", outcome.report));
    }
    let enrolled_id = plan.as_ref().map(|d| d.id.clone());
    if existing_hostname.is_none() {
        // A MEMBER is also enrolled into its Fleet's server over the control
        // plane (`ADR-059` §6) — provisioning is the ceremony. The local enroll
        // below already succeeded; the server enroll is additive and reported,
        // never fatal (see [`enroll_member_into_fleet`]).
        let member = plan
            .as_ref()
            .filter(|d| d.fleet_role == Some(rivetos_register::FleetRole::Member))
            .cloned();
        report.push_str(&fleet_recorder.commit(&document_path, plan));
        if let Some(device) = member {
            report.push_str(&enroll_member_into_fleet(&device));
        }
    }

    // **Wave C3, item 5 (draft §8): the summary, printed after a verified
    // write, never before.** `outcome.written` is `write_image::run`'s own
    // BLAKE3 read-back comparison having passed; a failed or declined write
    // never reaches this line at all.
    let summary =
        summarize(&document_path, image_descriptor.as_deref(), args.device_path.as_deref(), enrolled_id.as_deref());

    Ok(format!("{report}{}{summary}", outcome.report))
}

/// **The command that writes this same card later without the interview** —
/// the document and identity this run already has, the image by digest (its
/// scratch copy is removed when this run exits), and `confirm`, which offers
/// the device menu and skips `Write it? [y/N]`.
fn rerun_command(args: &ProvisionArgs, pin: Option<&[u8; 32]>, image: Option<&str>) -> String {
    fn word(path: &Path) -> String {
        let text = path.display().to_string();
        if text.contains(' ') { format!("\"{text}\"") } else { text }
    }
    let mut command = String::from("cargo rivet provision");
    if let Some(document) = &args.document_path {
        command.push_str(&format!(" --document {}", word(document)));
    }
    match (&args.identity_path, pin) {
        (Some(identity), _) => command.push_str(&format!(" --identity {}", word(identity))),
        (None, Some(pin)) => {
            let hex: String = pin.iter().map(|b| format!("{b:02x}")).collect();
            command.push_str(&format!(" --pin {hex}"));
        }
        (None, None) => {}
    }
    if let Some(image) = image {
        command.push_str(&format!(" --image {}", word(Path::new(image))));
    }
    if let (Some(staging), Some(document)) = (&args.staging_path, &args.document_path)
        && staging_path_beside(document).as_ref() != Some(staging)
    {
        command.push_str(&format!(" --staging {}", word(staging)));
    }
    if let Some(at) = &args.at {
        command.push_str(&format!(" --at {}", word(at)));
    }
    if args.not_mine {
        command.push_str(" --not-mine");
    }
    command.push_str(" confirm");
    command
}

/// **Wave C3, item 5.** The summary a verified write ends with — hostname,
/// role, the image, every carried product and version, the enrolled
/// register id (if any), the device written, and the one step this tool
/// cannot prove.
///
/// **Never claims the board boots.** `write_image::run`'s own BLAKE3
/// read-back proves the bytes on the device match what was written — it
/// does not, and cannot, prove `rivet-fleetd` or `rivet-mcpd` ever started.
/// `ADR-049` §9 and `ADR-050` §11 both draw this line already; this is
/// that same honesty applied to the run's own closing lines rather than left
/// to a reader who has to find it in the middle of the report above.
///
/// Best-effort: a document that cannot be read back or parsed here — it
/// already parsed once, earlier in this same run, so this would mean the
/// file changed under this process — prints nothing rather than failing a
/// run whose card has ALREADY been written and verified.
fn summarize(
    document_path: &Path,
    image_descriptor: Option<&str>,
    device_path: Option<&Path>,
    enrolled_id: Option<&str>,
) -> String {
    let Ok(text) = std::fs::read_to_string(document_path) else {
        return String::new();
    };
    let Ok(doc) = rivet_config::Document::parse(&text) else {
        return String::new();
    };
    let hostname = doc.hostname().map_or_else(|| "(derived by the machine)".to_owned(), ToString::to_string);
    let role = match role_of(&doc) {
        Some(rivetos_register::FleetRole::Server) => "server",
        Some(rivetos_register::FleetRole::Member) => "member",
        // No `FleetRole::StandAlone` variant exists (removed as a ratchet
        // increment: it never named a default distinct from `None`) — this
        // is still the right label for a summary line, just no longer a
        // typed role.
        None => "stand-alone",
    };
    // **What the card carries, not what the document pins.** The carry step
    // walks `carried_services`, so that is the list; an unpinned product is the
    // one version the fleet repository holds, because the carry step refuses
    // an unpinned product with more than one. A pin naming an image service is
    // one the carry step skipped, and it is named as such rather than listed
    // as carried, which would contradict the write report above.
    let mut carried = String::new();
    for service in doc.carried_services() {
        let product = service.name().as_str();
        let version = doc.carried_version(product).unwrap_or("(unpinned: the one version published)");
        carried.push_str(&format!("    {product} {version}\n"));
    }
    if carried.is_empty() {
        carried.push_str("    (nothing)\n");
    }
    for carry in doc.carry() {
        let product = carry.product().as_str();
        if rivet_core::service::IMAGE_SERVICE_NAMES.contains(&product) {
            carried.push_str(&format!(
                "    not carried: {product} {} is pinned, but it is part of the image, so no card \
                 carries it\n",
                carry.version()
            ));
        }
    }
    let next = if role == "server" { server_next_steps(&doc, &hostname) } else { String::new() };
    format!(
        "\n--- summary ---\n\
         \x20 hostname        {hostname}\n\
         \x20 role            {role}\n\
         \x20 image           {}\n\
         \x20 carried\n{carried}\
         \x20 register id     {}\n\
         \x20 device          {}\n\
         \n\
         UNPROVEN: that {hostname} boots as this document describes. Writing and reading back a \
         device proves the BYTES match; it does not prove `rivet-fleetd`, `rivet-proxyd` or \
         `rivet-mcpd` ever started. Once the board has powered on, `cargo rivet run {hostname} \
         -- get status` (SPEC-05 §4) is the command that closes that gap.\n{next}",
        image_descriptor.unwrap_or("(a literal --image PATH; no fleet-repository entry to name)"),
        enrolled_id.unwrap_or("(not enrolled — a stand-alone document)"),
        // `summarize` is only called once a device was written, so an absent
        // `device_path` here means one was chosen from the menu rather than
        // that nothing was written — named honestly rather than misread as
        // "nothing written", which the write report above already contradicts.
        device_path.map_or_else(
            || "chosen from the menu above — see the write report for which".to_owned(),
            |p| p.display().to_string(),
        ),
    )
}

/// **What a fleet server still needs after its first boot** — `ADR-054` §7,
/// `.rulings/` R-182 and R-184.
///
/// A server's card carries the fleet repository's TEXT and none of its
/// objects (R-182), so two things stand between a booted server and one
/// holding the repository this workstation holds: the text, which `cargo rivet
/// fleet push` sends, and then the objects, which `cargo rivet fleet sync`
/// sends into the server's repository store (R-184 D1). Both are said, in the
/// order they run.
///
/// **A server document that declares no `[repos.fleet]`** — one recorded
/// before R-184, rewritten with `--existing`, or composed by hand — would have
/// the push refused by the machine, which serves no repository of that name.
/// That is said instead of the push, with the tool that mints a declaration.
fn server_next_steps(doc: &rivet_config::Document, hostname: &str) -> String {
    let name = rivetos_fleet_repo::SERVED_NAME;
    let declared = doc.repos().iter().any(|r| r.name().as_str() == name);
    let text = if declared {
        format!(
            "\x20 1. its text   `cargo rivet fleet push {hostname}` shows the plan, and `cargo rivet \
             fleet push {hostname} confirm` pushes this workstation's fleet repository (`main`) \
             into the server's `{name}` repository over the control plane.\n"
        )
    } else {
        format!(
            "\x20 1. its text   THIS DOCUMENT DECLARES NO [repos.{name}], so the server serves no \
             repository to push into and `cargo rivet fleet push {hostname}` will be refused. \
             `rivet-fleet add repo --device {hostname} --name {name}` mints the declaration, and \
             its own report names how it reaches the machine.\n"
        )
    };
    format!(
        "\nNEXT, for a fleet server, once `get status` answers:\n\
         {text}\
         \x20 2. its objects   NOT carried on this card (R-182). After the push, `cargo rivet \
         fleet sync {hostname}` shows what would be sent, and `cargo rivet fleet sync {hostname} \
         confirm` sends every application and library object the manifest names into the \
         server's repository store over the control plane (R-184 D1). A rerun sends only what \
         is still missing.\n"
    )
}

#[cfg(test)]
#[path = "provision_tests.rs"]
mod tests;
