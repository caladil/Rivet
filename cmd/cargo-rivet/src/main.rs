//! `cargo rivet package` — `ADR-015` §3.2's build tool.
//!
//! > `cargo rivet package` builds for `aarch64-unknown-linux-musl` and emits
//! > the signed artifact. `cargo-deb` and `cargo-dist` are the shape being
//! > followed; this is deliberately unremarkable.
//!
//! It became writable on 2026-08-24, when the maintainer's ruling settled
//! `ADR-027` §9 items 2–4 — one `.rvt` suffix for every artifact kind, `<kind>`
//! inside the file, and **no `<platform>` component for an application**.
//! `PLAN.md` §0.8 called this *"unremarkable"* too, and put it fourth on a list
//! of five; what makes it worth having before the three ahead of it is that
//! **nothing else in the chain can be exercised until something produces an
//! artifact to exercise it with.**
//!
//! # The shape of this crate
//!
//! | Module | Holds |
//! | :-- | :-- |
//! | [`crate_meta`] | reading the user's `Cargo.toml` — `[package.metadata.rivet]` |
//! | [`name`] | `ADR-027`'s grammar, and the git claim a version field makes |
//! | [`emit`] | the I/O around `rivet_core::package::write` |
//! | [`update`] | `cargo rivet update` — the **system update** payload, `ADR-029` §15 |
//! | [`inspect`] | verify and read back, through the same code a board would run |
//! | [`key`] | 32 bytes off a path — **and not a key-management design** |
//! | `main` | the composition root: parse arguments, wire it up, report |
//!
//! # It emits two of the three kinds, and that is why it holds the key
//!
//! Added 2026-08-24 with [`update`]. `ADR-029` §11 rules **one container for
//! three artifact kinds**, differing in the interpretation of the metadata; this
//! tool now produces two of them, and `ADR-016`'s extension is the third and is
//! unruled. **Both signings happen here and nowhere else** — `cmd/cargo-rivet`
//! is the only crate in this workspace that reads a private seed, which is
//! `ADR-015` §4's first bullet made structural and is `ADR-029` §15.2's whole
//! argument for the update verb living here rather than in `cmd/rivet-image`.
//!
//! # What this tool cannot do, stated here because it is easy to overclaim
//!
//! `AGENTS.md` §6. A signed `.rvt` on a build host is not a running service,
//! and the distance between the two is `PLAN.md` §0.8's remaining links:
//!
//! * **No machine can verify one.** `ADR-028` §5.2's root set ships in
//!   `/system` and nothing puts one there, so no image carries a trust anchor.
//!   **This is the sentence that has not moved**, and it is the one that keeps
//!   everything else theoretical.
//! * ~~**No machine can install one.** `ADR-029` §12 item 4 was ruled on
//!   2026-08-24 — a new supervised binary — and it does not exist yet.~~
//!   **False within hours of being written** — `cmd/rivet-deploy` was built
//!   the same day (`ADR-029` §14) and has installed a payload onto a loop device
//!   (§14.6). Struck rather than deleted, because `AGENTS.md` §7 is about
//!   exactly this class of claim. **What is true is narrower:** no machine can
//!   install one *that it has verified*, because of the bullet above.
//! * **No machine could run an installed application.** The supervisor's service
//!   list is a compile-time constant and the desired-state document cannot name
//!   a fourth service (`PLAN.md` §0.8, link 7).
//! * **Where a signing key lives is not ruled** — `PLAN.md` §0.4.2, deferred by
//!   the maintainer as needing a real conversation. [`key`] does the smallest
//!   thing that closes a round trip and picks no default.

/// **`ADR-051`'s consumer** — reads what a machine's applications offer to back
/// up, fetches each over HTTPS with the operator's certificate, checks it at
/// the FORMAT, and stores it. It schedules nothing and keeps nothing.
mod backup;
/// `cargo rivet restore` — `ADR-051` §12, `.rulings/` R-183 D1: one
/// application's own backup, checked against its declared format and sent to
/// the `restore-path` it declares. The application restores itself; this
/// stops nothing.
mod restore;
/// `cargo rivet bump` — increment a standalone application's patch version and
/// commit it alone, the deliberate pre-package ritual `tools/card/bump.sh` is
/// for the system image (`ADR-027` §10), generalized to a target manifest.
/// PATCH ONLY, and it refuses an inherited `version.workspace = true`.
mod bump;
/// Rivet's own reader of an Android APK — `ADR-061` §4: the zip, the binary
/// manifest, enough of the resource table for a label, and the signing block's
/// current certificate and rotation lineage. It reads and does not verify.
mod apk_read;
mod crate_meta;
mod gpt;
mod elf;
mod deploy;
/// `cargo rivet fleet` — `ADR-052`'s fleet repository, step 1 of `§9`: a
/// git-tracked manifest and a digest-keyed object store on the workstation.
/// **Not** `rivet-fleet` (a different binary) and not `[repos.<name>]` (a git
/// repository a Rivet machine hosts) — see the module for why the name is
/// argued rather than assumed.
mod fleet;
/// Which disks this host can see, and which of them the platform itself
/// calls removable — `cargo rivet provision`'s `SPEC-05` §4.10 menu.
mod enumerate;
/// Asking Windows for administrator, and nowhere else: the elevated child
/// `cargo rivet provision --elevated-write` runs under, and the progress and
/// result files that stand in for a pipe `ShellExecuteEx` cannot give it —
/// `SPEC-05` §4's write and verify steps. See the module for why.
mod elevate;
/// A JSON value, parsed just far enough to read [`enumerate`]'s PowerShell
/// output. Not a dependency: see the module for why.
mod json;
/// **What a person must hold before a card is worth building**, and the one
/// place that knows what the set is. `Path::exists` and good sentences; it
/// mints nothing.
mod credentials;
/// A bare device name, resolved against `rivet-fleet`'s own register —
/// `PLAN-Feature-Set-2026-09-12` §7.1. `run` and `install` each take one.
mod device;
mod emit;
// `--host`, `--identity`, `--pin` — one spelling, for every verb that reaches a
// machine — **moved to `rivet_remote::endpoint` on 2026-08-29**. `ADR-034`
// §11.5.1 made `rivet-fleet` a control-plane client, so the three flags now have
// a third caller in a second binary and `ADR-014` puts a shared implementation
// out of a composition root.
mod identity;
mod library;
mod inspect;
mod key;
/// `cargo rivet key backup`/`restore` — `.rulings/` R-183 D2/D3, the
/// passphrase-encrypted seed backup. Its own module rather than folded into
/// `key`: that module derives a PUBLIC key from a seed and never writes a
/// seed anywhere; this one reads and writes the seeds themselves, under
/// encryption, and the two have almost no code in common.
mod key_backup;
/// The passphrase envelope — Argon2id and XChaCha20-Poly1305 behind a typed,
/// no-echo prompt. One format for every backup this tool encrypts: the signing
/// seeds (R-183 D3) and a board's secrets (R-185 D1).
mod envelope;
/// `cargo rivet backup-secrets`/`restore-secrets` — `.rulings/` R-183 D5 and
/// R-185: one board's `/state/secrets` pulled over the control plane,
/// passphrase-encrypted on the workstation, and put back onto the same board
/// and refused on any other (`ADR-030` §5).
mod machine_secrets;
/// `cargo rivet recover` — `ADR-063` §7, `.rulings/` R-235: a host rebuilt
/// from its recorded document, its artifacts by digest, its sealed secrets and
/// its applications' backups. It composes `provision --existing`,
/// `restore-secrets`, `install --artifact` and `restore`, and adds the plan and
/// the refusal to write over data unasked.
mod recover;
/// `cargo rivet image` — `R-177`. Wraps an already-built card image as a
/// signed `card` `.rvt` and publishes it into the fleet repository; never
/// builds one.
mod image;
/// Writing a PNG by hand — chunk framing, CRC-32 and a zlib stream of stored
/// blocks. No dependency, for `ADR-009` §5's reason; see the module.
mod png;
/// Reading QOI, which is what a Rivet writes when it photographs its screen.
/// **The SDK's QOI encoder, compiled into the test binary itself.**
///
/// Not a dev-dependency: `rivetos-sdk` does not build on Windows and this is a
/// host tool. Not a vendored copy either — a copy would agree with itself for
/// ever, and the point is that an edit to the encoder changes what this
/// crate's decoder is tested against, in the same commit.
///
/// The path is relative to `src/`, which exists, so it resolves identically on
/// the workstation and in the build container.
#[cfg(test)]
#[path = "../../../rivet-sdk/src/qoi.rs"]
mod device_qoi;

mod qoi;
/// Scratch directories for this crate's tests — one root per process, swept by
/// the run after. The helpers here used to leak every fixture directory into
/// `%TEMP%` for ever; `scratch.rs` explains what that cost.
#[cfg(test)]
mod scratch;
/// `cargo rivet run` — one catalogue operation over `ADR-035`'s control plane.
mod run;
/// `cargo rivet screenshot` — a capture off a board, as a PNG on your disk.
/// `PLAN.md` §104.39: the board writes QOI and this end is supposed to convert
/// it, and until 2026-09-01 nothing here did.
mod screenshot;
/// `cargo rivet secrets` — `ADR-046` step 3: the authority's three TLS files
/// into one application's data directory. It mints nothing and installs what
/// this host already has; see the module for what was actually missing.
mod secrets;
/// The Armada model's host-side enrollment client — `apply enroll-member`, sent
/// over the same pinned control plane `secrets` already speaks. Modeled on
/// `secrets::run`'s word-building exactly; see the module for the wire
/// contract it does not deviate from.
mod enroll;
/// `cargo rivet armada` — `ADR-059` §6's interactive CRUD tool over the
/// Armada Registry (`rivetos_register::armada`): this workstation's list of
/// Fleets. Bare is the interview, `armada <verb>` is the scripted twin —
/// `crate::fleet`'s own shape, one level up the `ADR-059` §5 split.
mod armada;
/// `cargo rivet setup` — the first run: the credentials, in order, into
/// `~/.rivet`. See the module for why the sequencer is here and not in
/// `cmd/rivet-provision`.
mod setup;
// What this workstation has and lacks for each goal — ADR-062.
mod doctor;
mod update;
/// `cargo rivet write-image` — inject a document into a built card image and,
/// with `confirm`, write it to a device. Renamed from `provision` 2026-09-13
/// (`docs/PLAN-Feature-Set-2026-09-12.md` §3.0, §5.2 item B; `.rulings/`
/// R-162, Wave 3b) once `provision` became the orchestrator below.
mod write_image;
/// Plain stage announcements for `cargo rivet provision` — eye candy, never
/// load-bearing. See the module for why there is no spinner.
mod progress;
/// `cargo rivet provision` — the one-command orchestrator: the generated
/// interview, composing the document, then `write_image::run` to put it on a
/// device. `docs/PLAN-Feature-Set-2026-09-12.md` §5.2 item B, `.rulings/`
/// R-162, R-174 and its reversal by R-175/R-176 — see the module for what
/// stayed and what was reverted.
mod provision;
mod provision_image;
mod carry;

// `ADR-027` §7's artifact naming moved to `rivetos-remote` on 2026-08-26, with
// the deploy sequence and for the same reason (`ADR-034` §11.5): `rivet-fleet`
// reads a payload's name back too, and two implementations of a grammar is the
// failure `ADR-011` names. Imported under its old name so that every call site
// in this crate reads as it did.
use rivet_remote::name;

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use rivet_adapter_image::BootSlots as _;
use rivet_core::package::application::{MANIFEST_NAME, TargetClass};
use rivet_core::package::kind::Kind;
use rivet_core::package::port::{Digest, KeyId, Sign};
use rivet_core::package::update::{SlotVariance, TargetClass as UpdateTarget};
use rivet_crypto::{Blake3, Ed25519Signer, Ed25519Verifier, SEED_BYTES};

/// `ADR-003` §4, and `AGENTS.md` §10.5: *"cross-compilation target is
/// `aarch64-unknown-linux-musl`"*.
const DEFAULT_TARGET: &str = "aarch64-unknown-linux-musl";

fn main() -> ExitCode {
    match run() {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        // An empty message is a verb that has already reported, on stdout,
        // why it failed — `doctor` — and wants only the exit status.
        Err(message) => {
            if !message.is_empty() {
                eprintln!("cargo-rivet: {message}");
            }
            ExitCode::FAILURE
        }
    }
}

/// One verb of this tool, and the line `--help` shows for it.
struct Verb {
    name: &'static str,
    synopsis: &'static str,
}

/// **Every verb `cargo rivet` has** — the authority for what [`usage`] lists
/// *and* for what [`fn@run`] will dispatch.
///
/// # Why one table rather than a match and a help string
///
/// `cargo rivet identity` worked and was **not in `cargo rivet --help`**. A cold
/// agent found it on 2026-08-29 only because `rivet-mcpd --help` happened to
/// mention `cargo rivet identity new`, and typed it speculatively. A verb that
/// works and is not in its own usage output is a verb nobody can find.
///
/// A test asserting "usage mentions identity" would have fixed that one line and
/// left the next one to be discovered the same way — `AGENTS.md` §7's *"grep for
/// the claim, not the paragraph"*. So the containment goes both ways and is
/// structural rather than asserted:
///
/// * **usage ⊇ table**, because [`usage`] is generated from it;
/// * **dispatch ⊆ table**, because [`fn@run`] refuses any word this table does not
///   carry *before* it reaches a match arm — so a verb added to the dispatcher
///   and not to the table does not work at all, rather than working invisibly;
/// * **table ⊆ dispatch**, because a word in the table with no arm behind it
///   reaches [`fn@run`]'s last arm, which calls it a defect and says so.
const VERBS: &[Verb] = &[
    // **First in the list because it is first in the story.** Every other verb
    // here presumes credentials that nothing sequenced until 2026-09-05; see
    // `crate::setup` for the argument about where the sequencer lives.
    Verb {
        name: "setup",
        synopsis: "[--at DIR] [--name NAME] [--check]",
    },
    // Before anything is built: what this machine has and lacks, per goal.
    Verb { name: "doctor", synopsis: "[--for run|app|image]..." },
    Verb { name: "package", synopsis: "[options]" },
    // Shared Rivets, 2026-08-31. A library is a payload in the same container
    // and its own verb, because it is usually not a Cargo crate — see
    // `cmd/cargo-rivet/src/library.rs` for why this is not `package --library`.
    Verb {
        name: "library",
        synopsis: "--name N --version V --object PATH [options]",
    },
    // **`bump` added 2026-09-22** (Joel, option A). The deliberate,
    // committed pre-package step: increment a standalone app's PATCH version
    // and commit it alone, `tools/card/bump.sh` generalized to a target
    // manifest. Refuses an inherited `version.workspace = true` — see
    // `crate::bump`. Not folded into `package`, which stays a pure read.
    Verb { name: "bump", synopsis: "[PATH]" },
    Verb { name: "update", synopsis: "[options]" },
    // **Renamed from `deploy` to `install`, 2026-09-22.** "Install" is the
    // verb; "deploy" is the interview — see the new `deploy` entry just
    // below. This is the scripted, flag-driven form: `--host` and the rest,
    // no prompts.
    Verb {
        name: "install",
        synopsis: "--host HOST [--rvt F.rvt | --artifact ID] [--enable] [confirm]",
    },
    // **`deploy` added 2026-09-22**, taking the bare shape `fleet` used to
    // have (`docs/design-candidates/Fleet-Deployment-Tool.md` §9). No
    // options: it is the interactive deploy interview, start to `Exit`, and
    // composes `install`, `secrets` and `run` for the operator rather than
    // asking for any of their flags. A script or an agent wants `install`
    // instead — see `deploy --help`.
    Verb { name: "deploy", synopsis: "(interactive — no arguments; an agent uses `install`)" },
    // **`ADR-052` §9 step 1, 2026-09-09.** The fleet repository — not
    // `rivet-fleet` and not `[repos.<name>]`; see `crate::fleet` for the
    // naming argument.
    Verb {
        name: "fleet",
        synopsis: "init|publish|list|plan|drift|push|sync|record|objects|phones|retire|hide|unhide [--at DIR] [options]",
    },
    // **`ADR-052` §7, `.rulings/` R-225.** The menu over the fleet repository:
    // delete or hide a product. Bare only, as `deploy` is; `fleet retire`,
    // `hide` and `unhide` are its scripted twin.
    Verb { name: "artifacts", synopsis: "(interactive — no arguments; an agent uses `fleet retire|hide|unhide`)" },
    // **`R-177`, 2026-09-13.** The verb R-176 named "the missing piece": a
    // card image joins the fleet repository the same way an application or a
    // system update does. Never builds one — see `crate::image`.
    Verb {
        name: "image",
        synopsis: "[--img PATH] --version V --arch TOKEN [options]",
    },
    Verb {
        name: "write-image",
        synopsis: "[--image PATH] [--document PATH] [--device PATH]",
    },
    Verb {
        name: "provision",
        synopsis: "preflight, image selection, the interview, an enrollment check, then write-image and recording [--image PATH-OR-DIGEST] [--platform TOKEN] [--device PATH] [--document PATH | --existing HOSTNAME] [--staging PATH] [--identity PATH | --pin HEX] [confirm]",
    },
    Verb {
        name: "run",
        synopsis: "--host HOST:PORT --identity DIR --pin HEX -- <rivet command>",
    },
    // **The other half of `rivet-sdk/src/qoi.rs`'s split**, 2026-09-01. A board
    // writes a capture as QOI because an encoder is sixty lines where a
    // compressor is a dependency; the conversion is this end's, and until this
    // verb nothing in the repository could do it.
    Verb {
        name: "screenshot",
        synopsis: "--host HOST:PORT --identity DIR --pin HEX [--path P | --holder] [--out F.png] [--json]",
    },
    // **`ADR-046` step 3, 2026-09-05.** The three TLS files an application
    // needs, from this host's authority into its data directory. It composes
    // `apply install-secret` three times and mints nothing — see
    // `crate::secrets` for why there is nothing to mint.
    Verb {
        name: "secrets",
        synopsis: "[DEVICE] APPLICATION [--host HOST:PORT --identity DIR --pin HEX] [--name N] [confirm]",
    },
    // **`ADR-051`, 2026-09-08.** `ADR-030` rules application data out of
    // Rivet's own backup; this is the consumer of the contract that lets an
    // application offer it anyway. Two transports at once — the control plane
    // for the declaration, 443 for the payload — and neither can do the
    // other's job.
    Verb {
        name: "backup",
        synopsis: "--host HOST:PORT --identity DIR --pin HEX --into DIR [--service NAME]... [confirm]",
    },
    // **`ADR-051` §12, `.rulings/` R-183 D1, 2026-09-14.** `backup`'s other
    // direction: one application's own backup, sent to the `restore-path` its
    // signed manifest declares. The application restores itself.
    Verb {
        name: "restore",
        synopsis: "--host HOST:PORT --identity DIR --pin HEX --service NAME --from FILE [confirm]",
    },
    // **`.rulings/` R-183 D5, 2026-09-14.** A board's own secrets, pulled to
    // this workstation and put back onto the same board only — `backup`'s flag
    // set, because it is `backup`'s shape over the control plane alone.
    Verb {
        name: "backup-secrets",
        synopsis: "--host HOST:PORT --identity DIR --pin HEX --into DIR [confirm]",
    },
    Verb {
        name: "restore-secrets",
        synopsis: "--host HOST:PORT --identity DIR --pin HEX --from DIR [--replace] [confirm]",
    },
    // **`ADR-063` §7, `.rulings/` R-235.** A host rebuilt from what is
    // already taken: the card, then after its first boot the secrets, the
    // applications by digest, and their data. The plan connects to nothing.
    Verb {
        name: "recover",
        synopsis: "HOSTNAME --from DIR [--secrets DIR] [--at DIR] [--after-boot] [--replace APP]... [confirm]",
    },
    Verb {
        name: "inspect",
        synopsis: "FILE.rvt --trusted-key PATH [--extract-program PATH]",
    },
    // **`ADR-059` §6.** The Armada Registry — this workstation's list of
    // Fleets — over the same bare/`<verb>` shape as `fleet` above.
    Verb {
        name: "armada",
        synopsis: "list|add|forget|set-backup-dir|set-keep",
    },
    Verb { name: "identity", synopsis: "new --directory DIR" },
    Verb {
        name: "key",
        synopsis: "SEED [--out PATH] | backup DIR | restore DIR [--replace]",
    },
];

/// **Which build this is**, as `--help` and `--version` both print it.
///
/// The version is `ADR-027` §10's — the workspace `Cargo.toml`'s, incremented
/// and committed alone before every image build. The commit is `build.rs`'s
/// describe string, and it is the half that distinguishes two *host* builds:
/// between image bumps a workstation builds this tool from many commits, all of
/// them the same version.
///
/// It cost an agent more time than anything else on 2026-08-29 that neither was
/// here — see `build.rs` for the measurement.
fn build_stamp() -> String {
    format!("cargo-rivet {}", build_version())
}

/// [`build_stamp`] without the program's own name — `0.1.87 (v0.1.86-9-g…)`.
///
/// **Separated because a second binary is now compared against it.**
/// `cargo rivet provision` runs `cmd/rivet-provision` as the interview and
/// refuses the pair when the two builds disagree (`.rulings/` R-191 D3), and
/// what it compares is exactly this — the half of each tool's `--version`
/// line that describes the build rather than which program printed it.
pub(crate) fn build_version() -> String {
    format!("{} ({})", env!("CARGO_PKG_VERSION"), env!("RIVET_BUILD_DESCRIBE"))
}

// ---------------------------------------------------------------------------
// Which binary is this — the 2026-08-30 finding
// ---------------------------------------------------------------------------

/// **The paragraph that tells a reader their tool is out of date**, and the one
/// thing an old build's *absence* of it can be read as.
///
/// # The finding
///
/// The second cold-agent run, given this repository and a board, ran
/// `cargo rivet run` because the README says that is the way in. The
/// `cargo-rivet` on its `PATH` was a build from 28 August, which answered:
///
/// ```text
/// cargo-rivet: "run" is not a verb.
/// ```
///
/// So the tool contradicted the README about the only transport that exists and
/// pointed at a channel this project had already retired, which `AGENTS.md` §10
/// names as the wrong path in capital letters. The agent recovered only by
/// guessing that `target/release/` might hold something newer. Its own words:
/// *"a cold agent following the README with the tool on its PATH concludes the
/// README is fiction, or falls back to the old way, which is the one thing it
/// was told not to do."*
///
/// # The asymmetry, which is why this is worded the way it is
///
/// **A binary already on somebody's `PATH` cannot be fixed from here.** Nothing
/// this build prints is read by the build that is actually answering. So the
/// only thing that can be built is a *positive* signal in the new one whose
/// absence is diagnostic of the old:
///
/// * this build answers `--version` and prints [`build_stamp`]; the 28 August
///   build has no `-V` and no `--version` at all;
/// * this build's refusals and its usage text **open with that stamp**, so a
///   `cargo-rivet:` line with no version above it came from something older;
/// * and `--version` says **which file answered** and whether that is the one
///   `cargo rivet` resolves to on this `PATH`.
///
/// That last one is the part that closes the agent's actual question — *"is the
/// repo's `target/release/cargo-rivet` the one I should be driving, given
/// `~/.cargo/bin` has an older one? Which is canonical?"* — because running the
/// repo's build now answers it in one line instead of leaving it a guess.
const WHICH_BINARY: &str = "\
WHICH `cargo-rivet` IS THIS? The line above is this build's version and the\n\
commit it came from. If a `cargo rivet` you ran printed NO version line, or\n\
refused `--version`, or told you `deploy` SENDS A PAYLOAD OVER A LOGIN SESSION,\n\
you are talking to an old binary on your PATH and not to this repository's\n\
build: there is one transport here, and the current tool has `run`, which\n\
speaks ADR-035's control plane. Install this one over it with\n\
\x20 cargo install --path cmd/cargo-rivet --force --locked\n\
\x20 cargo install --path cmd/rivet-provision --force --locked\n\
from the repository root, or invoke the built binary by path. BOTH lines are\n\
the install: `provision` runs rivet-provision as its interview, and the two\n\
builds have to match. There is no self-update: `cargo rivet update` builds a\n\
SYSTEM UPDATE payload for a board and does nothing to this tool.";

/// **The `cargo-rivet` an unqualified `cargo rivet` would run**, given a `PATH`.
///
/// Pure, and separated from the environment so it can be tested without one:
/// `directories` is the raw `PATH` value, `separator` is `;` or `:`, and
/// `exists` answers whether a candidate file is there. The real caller passes
/// [`std::path::Path::is_file`]; a test passes a closure over a fixed list, so
/// this is checked against several shapes of `PATH` on any host — which is what
/// `AGENTS.md` §7 wants of a claim that something is or is not somewhere.
///
/// **Both spellings are tried on every platform**, in the order `cargo` itself
/// resolves them: a Windows host has `cargo-rivet.exe`, a Unix one has
/// `cargo-rivet`, and a tool that only looked for its own host's spelling would
/// answer *nothing is shadowing you* on the other for the wrong reason.
pub(crate) fn first_on_path(
    directories: &str,
    separator: char,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    for directory in directories.split(separator) {
        if directory.is_empty() {
            continue;
        }
        for name in ["cargo-rivet.exe", "cargo-rivet"] {
            let candidate = Path::new(directory).join(name);
            if exists(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// Whether two paths name the same file, as well as this host can say.
///
/// **Canonicalised when both can be**, because `~/.cargo/bin/cargo-rivet.exe`
/// and a relative path to the same file are the same binary and must not be
/// reported as a shadow. When either cannot be canonicalised — it was deleted
/// between the two calls, or the host refuses — the comparison falls back to the
/// literal paths, which can say *different* about one file and never says *same*
/// about two. Erring towards showing the reader both paths is the right error:
/// the failure this exists to prevent is a silent one.
pub(crate) fn same_file(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

/// The two lines `--version` prints under the stamp: where this build is, and
/// what `cargo rivet` would run instead.
///
/// **It reports rather than refuses.** Running a binary directly is a
/// legitimate thing to do and is exactly what the agent that hit this ended up
/// doing; the defect was never that it happened, it was that nothing said so.
fn which_binary() -> String {
    let Ok(exe) = std::env::current_exe() else {
        return "  running   this host will not say where this binary is, so it cannot be \
                compared against your PATH\n"
            .to_string();
    };
    let mut out = format!("  running   {}\n", exe.display());

    let path = std::env::var("PATH").unwrap_or_default();
    let separator = if cfg!(windows) { ';' } else { ':' };
    match first_on_path(&path, separator, &|p: &Path| p.is_file()) {
        None => out.push_str(
            "  on PATH   nothing — `cargo rivet` will not find a subcommand at all unless you \
             invoke this file by path\n",
        ),
        Some(found) if same_file(&exe, &found) => {
            out.push_str("  on PATH   this one — `cargo rivet` runs the binary above\n");
        }
        Some(found) => out.push_str(&format!(
            "  on PATH   {}\n\
             \n\
             THIS IS NOT THE BINARY `cargo rivet` RUNS. The file on your PATH answers\n\
             first, and it is a different file from the one that just printed this.\n\
             Compare its `--version` against the stamp above before believing anything\n\
             either of them says about what a board can do.\n",
            found.display(),
        )),
    }
    out
}

/// The `usage:` block, generated from [`VERBS`] and aligned.
fn synopses() -> String {
    let widest = VERBS.iter().map(|v| v.name.len()).max().unwrap_or(0);
    VERBS
        .iter()
        .enumerate()
        .map(|(at, verb)| {
            let lead = if at == 0 { "usage:" } else { "      " };
            format!(
                "{lead} cargo rivet {name:<widest$}  {synopsis}\n",
                name = verb.name,
                synopsis = verb.synopsis,
            )
        })
        .collect()
}

/// A word that is not one of [`VERBS`], refused with the whole list.
fn unknown_verb(word: &str) -> String {
    format!(
        "`{word}` is not a `cargo rivet` verb. This build has {}: {}.\n\n{}",
        VERBS.len(),
        VERBS.iter().map(|v| v.name).collect::<Vec<_>>().join(", "),
        usage()
    )
}

fn usage() -> String {
    format!(
        "{build}\n\
         \n\
         {synopses}\
         \n\
         {which}\n\
         \n\
         `package` emits an APPLICATION .rvt from a Rust crate; `update` emits a\n\
         SYSTEM UPDATE .rvt from a built system image; `install` builds one, sends\n\
         it to a board over ADR-035's control plane and installs it. `deploy`\n\
         (bare — no arguments) is the interactive interview that composes\n\
         `install`, `secrets` and `run` for you; a script or an agent wants\n\
         `install` instead. ONE PLANNER DRIVES BOTH: `install` waits for its own\n\
         install to finish and then prints the steps that remain, as the exact\n\
         commands the interview would have run.\n\
         \n\
         `bump` increments a standalone application's PATCH version (the Z of\n\
         X.Y.Z, never the minor or major) and commits it alone — the deliberate\n\
         pre-package step tools/card/bump.sh already is for the system image\n\
         (ADR-027 §10.2): refuse a dirty tree, bump the patch, commit \"Version\n\
         X.Y.Z\", so version and commit stay a 1:1 mapping. It refuses a version\n\
         written `version.workspace = true` (inherited from a workspace root —\n\
         for a cmd/* OS binary that number is bump.sh's). `package` and `library`\n\
         stay pure reads; a LIBRARY has no versioned source, so it is out of\n\
         scope. `bump --help` is the whole story.\n\
         \n\
         INSTALLING AN APPLICATION DOES NOT RUN IT (ADR-015 §3.2.2). `enabled`\n\
         is the operator's and lives in the machine's document, so an install on\n\
         its own leaves the application idle with nothing to diagnose. `install\n\
         --enable` writes it for you; `install --help` is the whole story.\n\
         \n\
         THE MANIFEST IS `[package.metadata.rivet]` IN YOUR Cargo.toml. It is a\n\
         REQUEST, not a description -- ADR-015 §3.3. These are the keys and the\n\
         only values each accepts:\n\
         \n\
         \x20 kind             = daemon | oneshot | socket-activated\n\
         \x20 restart          = always | on-failure | never\n\
         \x20 ready            = none | notify | listening\n\
         \x20 network          = none | egress | group:<name>\n\
         \x20 state            = none | private\n\
         \x20 memory           = 128M          -- digits, then K, M or G\n\
         \x20 cpu              = 0.5           -- decimal cores; ADR-015 §3\n\
         \x20 devices          = [\"display\", \"input\"]\n\
         \x20 library          = [\"<name>-<version>-<64 hex>\"]   -- see `library`\n\
         \x20 listen           = [\"tcp:8080\", \"tcp:8443:direct\", \"udp:53\", \"unix:name\"]\n\
         \x20 connects         = [\"<owner>:<endpoint>\"]\n\
         \x20 provides         = [\"socket:<endpoint>\"]\n\
         \x20 title            = \"Aura Controller\"  -- what a PERSON calls it\n\
         \x20 settings         = [\"<name>\"]     -- ADR-038; declared, not valued\n\
         \x20 proxy-path       = <one segment>  -- https://<host>/<segment>/\n\
         \x20 proxy-auth       = operator | none    -- absent means operator\n\
         \x20 proxy-auth-paths = [\"none:pair\", \"operator:pair/admin\"]\n\
         \x20 backup-path      = api/v1/backup  -- relative to proxy-path\n\
         \x20 backup-format    = sqlite | csv | json | tar\n\
         \x20 backup-feed      = api/v1/blobs   -- the OTHER shape; not both\n\
         \x20 accepts-class    = [\"temperature-sensor\"]  -- ADR-053; absent/empty takes no addon\n\
         \x20 restore-path     = api/v1/restore -- needs backup-path; operator-gated\n\
         \x20 material         = [\"fleet-tls\"]  -- ADR-055; per-machine, operator-installed\n\
         \n\
         THIS LIST IS THE WHOLE OF IT. It was six keys short from 2026-09-02\n\
         until 2026-09-08 -- memory, title, settings and all three proxy keys\n\
         existed, were accepted, and were documented nowhere a person types.\n\
         \n\
         `backup-path` + `backup-format` say WHERE this application serves a\n\
         backup and WHAT one fetch returns; `backup-feed` is the other shape, a\n\
         resumable collection, and a manifest may not declare both. Either needs\n\
         a `proxy-path` to be relative to. `cargo rivet backup` is what reads\n\
         them -- ADR-051. `restore-path` says where the application accepts that\n\
         backup BACK; it needs `backup-path` and `backup-format`, the route must\n\
         require an operator certificate, and `cargo rivet restore` sends to it.\n\
         \n\
         `material` NAMES A KIND, NEVER A FILE -- ADR-055 §3. `fleet-tls` is\n\
         tls-cert, tls-key and tls-root, the three files an application serving\n\
         HTTPS behind rivet-proxyd reads out of its own data directory, and the\n\
         same three a caller presenting a client certificate reads. They are\n\
         PER-MACHINE and per-operator: `install` never mints them (R-194), it\n\
         refuses to put an application into service without them and prints the\n\
         `rivet-provision ca server` and `cargo rivet secrets` lines that install\n\
         them. Declare it and the refusal arrives before the payload does;\n\
         declare nothing and install says nothing, exactly as it always has.\n\
         \n\
         A PROXIED APPLICATION'S PORTS LISTEN ON 127.0.0.1 (ADR-046 §7). With a\n\
         `proxy-path`, every `tcp:` and `udp:` port binds loopback, so only\n\
         rivet-proxyd and the machine's own programs reach it. A port that must be\n\
         reached directly -- a phone's listener -- says `tcp:8443:direct`. The\n\
         routed port (the first tcp one) may not be `:direct`, and `:direct` needs\n\
         a `proxy-path`: without one every port already binds every address.\n\
         \n\
         THERE IS NO `kind = \"kiosk\"`. An application is a kiosk one because it\n\
         declares devices = [\"display\"], and it is then OFFERED on the launcher's\n\
         menu rather than started (ADR-036 §8A). `examples/templates/` has a\n\
         working manifest of each shape, with the reasoning beside every key.\n\
         \n\
         `run` performs ONE operation from the machine's own catalogue over\n\
         ADR-035's control plane and prints the machine's answer, unchanged —\n\
         `-- get storage`, `-- apply reload confirm`. It is how you ask a board a\n\
         question without opening a shell on it.\n\
         \n\
         `screenshot` fetches a capture from a board and writes it as a PNG. A\n\
         Rivet photographs its screen as QOI, because an encoder is sixty lines\n\
         on a device where a compressor is a dependency; turning one back into a\n\
         picture is this end's job. IT DOES NOT TAKE THE PICTURE — the launcher\n\
         does that when it is sent `apply input capture`, and it refuses while an\n\
         application holds the display. `screenshot --help` says why that matters.\n\
         \n\
         `fleet` is ADR-052's FLEET REPOSITORY — a git-tracked manifest and a\n\
         digest-keyed object store, on THIS workstation, naming every signed\n\
         artifact the fleet is built from. `fleet init` makes one, `fleet\n\
         publish FILE.rvt --trusted-key PATH` verifies and stores an artifact,\n\
         `fleet list` reads it back. `install` publishes into it before it\n\
         sends, and `install --artifact ID` sends one it holds. NOT `rivet-fleet` (a different binary) and NOT `[repos.<name>]`\n\
         (a git repository a Rivet MACHINE hosts, managed by `rivet-fleet add\n\
         repo`) — `fleet --help` says so again, at length.\n\
         \n\
         `cargo rivet update --help`, `install --help`, `run --help`,\n\
         `screenshot --help` and `fleet --help` for those verbs' options.\n\
         \n\
         package options:\n\
         \x20 --manifest-path PATH  the crate's Cargo.toml (default ./Cargo.toml)\n\
         \x20 --signing-key PATH    32 raw bytes, or 64 hex characters. Optional\n\
         \x20                       since 2026-08-30: without it, $RIVET_APP_SEED\n\
         \x20                       then ~/.rivet/keys/rivet-app.seed. If neither\n\
         \x20                       is there the refusal names all three and says\n\
         \x20                       what a board will accept. NOT the same key as\n\
         \x20                       `cargo rivet identity new`, which mints a TLS\n\
         \x20                       identity\n\
         \x20 --target TRIPLE       default {DEFAULT_TARGET}\n\
         \x20 --profile NAME        cargo profile (default release)\n\
         \x20 --bin NAME            which binary (default: the package name)\n\
         \x20 --binary PATH         use this already-built binary and run no\n\
         \x20                       cargo build at all\n\
         \x20 --target-dir DIR      cargo's target directory (default <crate>/target)\n\
         \x20 --out-dir DIR         where the .rvt goes (default <crate>/target/rivet)\n\
         \x20 --asset NAME=PATH     an extra member, repeatable (ADR-015 §3)\n\
         \x20 --no-assets           do not walk <crate>/assets/\n\
         \x20 --public-key-out PATH write the PUBLIC key here, 32 raw bytes\n\
         \x20 --print-manifest      render [package.metadata.rivet] and print it.\n\
         \x20                       Builds nothing, signs nothing, needs no key —\n\
         \x20                       only --manifest-path is read alongside it.\n\
         \x20                       For a harness that needs a rendered manifest\n\
         \x20                       and would otherwise hand-copy one\n\
         \x20                       (SDK-Adoption-Friction.md §2.3)\n\
         \n\
         EVERY FILE IN <crate>/assets/ IS PACKAGED, with no flag and no list.\n\
         `aura-renderer` shipped without its four typefaces because --asset is\n\
         easy not to type, exited on a missing face, and left a board's screen\n\
         black for hours with the display hand-over working perfectly\n\
         (PLAN.md §119.3). Members are FLAT — 32 bytes, no path separator, each\n\
         installed beside the program — so a subdirectory under assets/ is\n\
         refused rather than flattened into a path your program will not open.\n\
         --no-assets turns the walk off; --asset still adds to it.\n\
         \n\
         The output name is composed from what the tool knows and there is no\n\
         override — ADR-027 §7. An application carries no <platform> component.\n\
         \n\
         $RIVET_PACKAGE_DIRTY overrides whether the version is `-dirty`, when\n\
         set to one of 1/true/yes/dirty or 0/false/no/clean. Unset (the\n\
         default), the tree's dirty state is asked of the git this runs with —\n\
         which is wrong inside this project's build container on an otherwise\n\
         clean tree (AGENTS.md §10.5). The escape is one fact, not a whole\n\
         describe string: git rev-parse and git describe --exact-match do not\n\
         disagree between the two gits, only git status --porcelain does.\n\
         \n\
         key options:\n\
         \x20 new --out PATH        ask the OPERATING SYSTEM for 32 bytes and write\n\
         \x20                       them there. It REFUSES an existing file: every\n\
         \x20                       board that trusts the public half would refuse\n\
         \x20                       every future update, and there is no backup\n\
         \x20 --out PATH            write the PUBLIC key here, 64 hex characters\n\
         \x20 backup DIR            encrypt ~/.rivet/keys' seed(s) under a passphrase\n\
         \x20                       (asked twice, no echo) and write them to DIR.\n\
         \x20                       `key backup --help` for the file format and why\n\
         \x20                       a forgotten passphrase cannot be recovered from.\n\
         \x20 restore DIR           decrypt DIR's seed(s) back to ~/.rivet/keys.\n\
         \x20                       Refuses a wrong passphrase or an existing seed —\n\
         \x20                       `--replace` overrides only the second.\n\
         The seed is read and never printed, on any path.\n\
         \n\
         START WITH `cargo rivet setup`. It mints both signing seeds, your\n\
         control-plane identity and the board's, in the order they depend on\n\
         each other, and skips whatever is already there. Running the four\n\
         commands by hand works and gets the ORDER wrong, which is how a card\n\
         comes to be built before the key it is supposed to be built against.\n\
         \n\
         `setup --check` reports what this host holds and mints nothing.\n\
         Never commit a seed (AGENTS.md §8.3).",
        build = build_stamp(),
        which = WHICH_BINARY,
        synopses = synopses(),
    )
}

fn run() -> Result<String, String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // cargo runs `cargo-rivet rivet package …`, handing the subcommand's own
    // name as the first argument. Invoked directly it is absent. Both work.
    if args.first().is_some_and(|a| a == "rivet") {
        args.remove(0);
    }
    dispatch(&args)
}

/// **The verb table, applied.** Split from [`fn@run`] so that the tests can hand it
/// a word list: `run` reads `std::env::args`, and a dispatcher that can only be
/// reached through the process's own argv is one no test can walk.
fn dispatch(args: &[String]) -> Result<String, String> {
    match args.first().map(String::as_str) {
        None | Some("-h" | "--help" | "help") => Ok(format!("{}\n", usage())),
        // **`--version` says which FILE answered**, since 2026-08-30. See
        // `WHICH_BINARY`: the only fix available for a stale binary already
        // on somebody's PATH is a positive signal in the new one.
        Some("-V" | "--version") => Ok(format!(
            "{}\n{}\n{}\n",
            build_stamp(),
            which_binary(),
            WHICH_BINARY,
        )),
        // **The table gates the dispatch**, so a verb that is not in `--help`
        // is not a verb at all. See `VERBS` for why that containment is
        // structural rather than a test on one line of usage text.
        Some(word) if !VERBS.iter().any(|verb| verb.name == word) => Err(unknown_verb(word)),
        // The first run. It mints only what is absent and never overwrites a
        // key — see `crate::setup`, which also argues why this is a verb here
        // rather than a mode of `rivet-provision`.
        Some("setup") => match args.get(1).map(String::as_str) {
            Some("-h" | "--help") => Ok(setup::usage()),
            _ => setup::run(&setup::parse(&args[1..])?),
        },
        // Reads and changes nothing; `crate::doctor` argues why it is a verb.
        Some("doctor") => match args.get(1).map(String::as_str) {
            Some("-h" | "--help") => Ok(doctor::usage()),
            _ => doctor::run(&doctor::parse(&args[1..])?),
        },
        // `--print-manifest` is answered without a signing key, a target or a
        // build — see `print_manifest` for why it is not routed through
        // `parse_package`, which requires a key this verb has no use for.
        Some("package") if args[1..].iter().any(|a| a == "--print-manifest") => {
            let rest: Vec<String> =
                args[1..].iter().filter(|a| *a != "--print-manifest").cloned().collect();
            print_manifest(&rest)
        }
        Some("package") => package(&args[1..]),
        // Shared Rivets. Emits the other half of a closure: the artifact an
        // application's `library = [...]` names.
        Some("library") => match args.get(1).map(String::as_str) {
            Some("-h" | "--help") | None => Ok(library::usage()),
            _ => library::run(&library::parse(&args[1..])?),
        },
        // The deliberate pre-package version bump — `ADR-027` §10's sequence
        // (refuse dirty, increment the patch, commit alone) pointed at a target
        // manifest. It composes nothing here; `crate::bump` is the whole story.
        Some("bump") => bump::run(&args[1..]),
        // `ADR-029` §15. The verb that closes the hole in the middle of the
        // deploy chain: `cmd/rivet-deploy` could install a payload nothing in
        // this project could produce.
        Some("update") => match args.get(1).map(String::as_str) {
            Some("-h" | "--help") => Ok(update::usage()),
            _ => update::run(&args[1..]),
        },
        // `PLAN.md` §0.8's chain, as one command. It composes `package` and the
        // board's own `apply install`; it implements neither. Renamed from
        // `deploy` to `install` 2026-09-22 — `deploy` (below) is now the
        // interactive interview; this is its scripted, flag-driven twin.
        Some("install") => match args.get(1).map(String::as_str) {
            Some("-h" | "--help") => Ok(deploy::usage()),
            _ => deploy_cmd(&args[1..]),
        },
        // The interview, taking the bare shape `fleet` used to have — see the
        // `VERBS` entry above. Bare runs it; `-h`/`--help` describes it; and
        // ANY OTHER WORD IS REFUSED, because the interview picks its hosts
        // itself and would have ignored it (`fleet::interview_refuses`).
        Some("deploy") => match args.get(1).map(String::as_str) {
            None => fleet::run_interview(),
            Some("-h" | "--help") => Ok(fleet::interview_usage()),
            Some(word) => Err(fleet::interview_refuses(word)),
        },
        // `R-177`. Sibling to `update` above: it does not build, it wraps
        // and signs bytes another tool already produced.
        Some("image") => match args.get(1).map(String::as_str) {
            Some("-h" | "--help") | None => Ok(image::usage()),
            _ => image::run(&args[1..]),
        },
        Some("write-image") => match args.get(1).map(String::as_str) {
            Some("-h" | "--help") | None => Ok(write_image::usage()),
            _ => write_image::run(&write_image::parse(&args[1..])?),
        },
        Some("provision") => match args.get(1).map(String::as_str) {
            Some("-h" | "--help") => Ok(provision::usage()),
            _ => provision::run(&args[1..]),
        },
        // `ADR-052` §9 step 1 — same shape as `library` and `provision` above:
        // no sub-verb, or an explicit `--help`, prints usage and exits clean
        // rather than being treated as the refusal `fleet::parse` gives a
        // genuinely unknown sub-verb. **Changed 2026-09-22**: a bare
        // `cargo rivet fleet` used to drop into the interactive deploy
        // interview (Ruling 4,
        // `docs/design-candidates/Fleet-Deployment-Tool.md` §9); that
        // interview is now `cargo rivet deploy` (see the `deploy` arm above),
        // so `fleet` bare is ONLY its sub-verbs from here on, same as
        // `library` and `provision`.
        Some("fleet") => match args.get(1).map(String::as_str) {
            Some("-h" | "--help") | None => Ok(fleet::usage()),
            _ => fleet::run(&fleet::parse(&args[1..])?),
        },
        // `ADR-052` §7 — the `deploy` arm's shape: bare runs the menu,
        // `--help` describes it, and any other word is refused rather than
        // ignored.
        Some("artifacts") => match args.get(1).map(String::as_str) {
            None => fleet::artifacts::run(),
            Some("-h" | "--help") => Ok(fleet::artifacts::usage()),
            Some(word) => Err(fleet::artifacts::refuses(word)),
        },
        // `ADR-059` §6 — same shape as `fleet` immediately above: bare
        // drops into the interview, `armada <verb>` is the scripted twin.
        Some("armada") => match args.get(1).map(String::as_str) {
            Some("-h" | "--help") => Ok(armada::usage()),
            None => armada::run_interview(),
            _ => armada::run(&armada::parse(&args[1..])?),
        },
        // `ADR-035`'s control plane, carrying an operation that is not part of
        // a deploy. Until this existed, the only way to run one from a
        // workstation was a login session — see `run`'s own module
        // documentation for the measurement that says so.
        Some("run") => match args.get(1).map(String::as_str) {
            Some("-h" | "--help") | None => Ok(run::usage()),
            _ => run::execute(&args[1..]),
        },
        // `PLAN.md` §104.39's ruling: the board writes QOI, and the software
        // that pulls a capture hands the maintainer a PNG (.rulings/ R-326). It
        // fetches and converts
        // and deliberately does NOT press `apply input capture` — the module
        // documentation has the three reasons, and the first is that the
        // launcher's refusal to capture goes to its own log rather than to this
        // caller.
        Some("screenshot") => match args.get(1).map(String::as_str) {
            Some("-h" | "--help") | None => Ok(screenshot::usage()),
            _ => screenshot::execute(&args[1..]),
        },
        // `ADR-046` step 3. Three `apply install-secret` calls with the files
        // read here rather than pasted as hex — one of them is a private key,
        // and a command line is visible in the process table.
        Some("secrets") => match args.get(1).map(String::as_str) {
            Some("-h" | "--help") | None => Ok(secrets::usage()),
            _ => secrets::run(&secrets::parse(&secrets_words(&args[1..])?)?),
        },
        // `ADR-051`. It reads the machine's own declaration over the control
        // plane and fetches the payloads through `rivet-proxyd` — see the
        // module for why those are two transports and why neither can be the
        // other. It schedules nothing.
        Some("backup") => match args.get(1).map(String::as_str) {
            Some("-h" | "--help") | None => Ok(backup::usage()),
            _ => backup::execute(&args[1..]),
        },
        // `ADR-051` §12. The same table and the same two transports as
        // `backup`, in the other direction; it stops nothing on the machine.
        Some("restore") => match args.get(1).map(String::as_str) {
            Some("-h" | "--help") | None => Ok(restore::usage()),
            _ => restore::execute(&args[1..]),
        },
        // R-183 D5 and R-185. Both read and write `/state/secrets` over the
        // control plane only; the copy on this workstation is sealed under a
        // typed passphrase (`crate::envelope`).
        Some("backup-secrets") => match args.get(1).map(String::as_str) {
            Some("-h" | "--help") | None => Ok(machine_secrets::backup_usage()),
            _ => machine_secrets::backup_execute(&args[1..]),
        },
        Some("restore-secrets") => match args.get(1).map(String::as_str) {
            Some("-h" | "--help") | None => Ok(machine_secrets::restore_usage()),
            _ => machine_secrets::restore_execute(&args[1..]),
        },
        // R-235. The plan reads files only; the card step needs a terminal;
        // `--after-boot` composes the verbs above against a running machine.
        Some("recover") => match args.get(1).map(String::as_str) {
            Some("-h" | "--help") | None => Ok(recover::usage()),
            _ => recover::execute(&args[1..]),
        },
        Some("inspect") => inspect_cmd(&args[1..]),
        // Not an operation of ADR-006's closed verb set — this is a HOST tool
        // and its verbs are its own. It answers the one question a person has
        // after making 32 random bytes: what is the public key, and what will a
        // board call it in a refusal.
        // `.rulings/` R-183 D2/D3 — the passphrase-encrypted seed backup.
        // Intercepted here, before `key::parse` ever sees `backup`/`restore`
        // as a candidate seed path — the same trade `key::parse` already
        // makes for `new` (its own comment: a seed genuinely named `new` is
        // still reachable as `./new`; the same is true of `./backup` and
        // `./restore` here).
        Some("key") => match args.get(1).map(String::as_str) {
            Some("backup") => match args.get(2).map(String::as_str) {
                Some("-h" | "--help") => Ok(key_backup::backup_usage()),
                _ => key_backup::backup(&args[2..]),
            },
            Some("restore") => match args.get(2).map(String::as_str) {
                Some("-h" | "--help") => Ok(key_backup::restore_usage()),
                _ => key_backup::restore(&args[2..]),
            },
            _ => key::run(&key::parse(&args[1..])?),
        },
        // A device TLS identity: ADR-025 §3 puts generation in the fleet
        // tool, and this is the only binary here that may hold a private
        // key at all (ADR-015 §3.2).
        Some("identity") => identity::run(&identity::parse(&args[1..])?),
        // **Everything left is in `VERBS` and has no arm above**, which is a
        // verb this tool advertises and cannot perform. Said out loud rather
        // than answered with the usage text, because it is a defect in this
        // file and not a mistake by whoever typed it — `cmd/rivet`'s `main`
        // says the same thing about `ADR-032` §4.2's internal handlers.
        Some(other) => Err(format!(
            "`{other}` is listed in this tool's own usage and this build has no implementation \
             behind it; that is a defect in cargo-rivet, not in what you typed."
        )),
    }
}

// ---------------------------------------------------------------------------
// package
// ---------------------------------------------------------------------------

struct PackageArgs {
    manifest_path: PathBuf,
    signing_key: PathBuf,
    target: String,
    profile: String,
    bin: Option<String>,
    binary: Option<PathBuf>,
    target_dir: Option<PathBuf>,
    out_dir: Option<PathBuf>,
    assets: Vec<(String, PathBuf)>,
    /// Do not walk `<crate>/assets/`. The escape hatch for a crate whose
    /// `assets/` holds source material rather than shipped files.
    no_assets: bool,
    public_key_out: Option<PathBuf>,
}

/// The environment variable naming the application signing seed.
///
/// **The convention is not new.** `tools/update/build.sh` reads
/// `RIVET_SYSTEM_SEED` and `tools/card/build.sh` reads `RIVET_APP_KEY` for the
/// public half; this is the third member of a family, spelled the way the other
/// two are.
pub const APP_SEED_ENV: &str = "RIVET_APP_SEED";

/// A variable that is set **and not empty**.
///
/// **`var_os` returns `Some("")` for a variable that is set to nothing**, and
/// this cost a wrong answer within a minute of the code being written: testing
/// the refusal with `RIVET_APP_SEED=` took the "the operator named a path"
/// branch with an empty path, and the failure surfaced as
/// *"cannot read the signing key ␠: The system cannot find the path specified"*
/// — a message about a missing file, naming no file, which is precisely the
/// shape this whole change exists to remove.
///
/// An empty value is how a shell spells *unset* in practice, so it is treated
/// as one.
fn set_to_something(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name).filter(|v| !v.is_empty())
}

/// Find the signing seed, or refuse by naming every place that was looked.
///
/// # The defect this closes, and it stopped a cold agent
///
/// `--signing-key` was **required with no default**, and `keys/README.md` says
/// the seed's location is *"deliberately not written down here"* — which is a
/// correct thing to say about a secret and leaves a reader with no way to
/// proceed. A cold agent on 2026-08-30 found the file by running `find ~/.rivet`
/// and then signed with it **on a hunch**, having no way to know it was the
/// right key. It called this one of three blockers that would stop an agent
/// dead.
///
/// **The seed's location staying unwritten is the ruling and it is kept.** What
/// changes is that the TOOL knows where to look, so the fact reaches an operator
/// without the repository publishing a path. Discoverability and secrecy were
/// never actually in tension here; they only looked it because the only channel
/// considered was the README.
///
/// # The refusal names all three places, and the constraint nobody states
///
/// A key you generate yourself is **useless against an existing board**: the
/// trust store lives on the read-only `/system` slot and is composed when the
/// image is built (`ADR-028` §5.5.4), so a board accepts exactly the keys it was
/// built with. That sentence is the one the cold agent most needed and could
/// find nowhere — `keys/README.md`'s *"any 32 bytes"* framing is true of the
/// FORMAT and misleading about deploying to a machine.
/// The environment variable that lets a caller supply the DIRTY half of a
/// package's version, instead of `cargo rivet package` asking the git it runs
/// with — `AGENTS.md` §10.5's two-git disagreement, `rivet_remote::name`'s
/// `Version::compose_with_dirty` doc comment for the full argument.
///
/// **Named for the one fact it overrides**, not for the whole describe
/// string: `git rev-parse` and `git describe --exact-match` do not disagree
/// between a container's git and Windows', so only `git status --porcelain`
/// needed an escape, and giving this the shape of `RIVET_DESCRIBE` (a whole
/// opaque string, `tools/card/build.sh`'s convention for the SYSTEM image)
/// would mean re-deriving a fact from a string this tool did not compose,
/// for a payload whose naming grammar `RIVET_DESCRIBE`'s producer has never
/// had to match.
pub const PACKAGE_DIRTY_ENV: &str = "RIVET_PACKAGE_DIRTY";

/// Read [`PACKAGE_DIRTY_ENV`], or `None` when the caller left it unset — in
/// which case `cargo rivet package` keeps asking the git it runs with, which
/// is wrong in the safe direction (AGENTS.md §5) and stays that way by
/// default.
///
/// # Errors
///
/// The variable is set to something that is neither a recognised spelling of
/// "the tree is dirty" nor of "the tree is clean" — refused rather than
/// guessed, naming what it was given and what it will take.
fn dirty_override() -> Result<Option<bool>, String> {
    let Some(value) = set_to_something(PACKAGE_DIRTY_ENV) else {
        return Ok(None);
    };
    match value.to_string_lossy().trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "dirty" => Ok(Some(true)),
        "0" | "false" | "no" | "clean" => Ok(Some(false)),
        other => Err(format!(
            "${PACKAGE_DIRTY_ENV} is {other:?}, which is neither a spelling of dirty (1, true, \
             yes, dirty) nor of clean (0, false, no, clean). Unset it to let cargo rivet package \
             ask the git it runs with, or set it to one of those words — this build will not \
             guess which you meant."
        )),
    }
}

fn resolve_signing_key() -> Result<PathBuf, String> {
    let over = credentials::Overrides::from_environment();
    let fallback = match credentials::app_seed(&over, credentials::root().as_deref()) {
        Ok(found) => return Ok(found),
        Err(looked_at) => looked_at,
    };
    let looked = fallback
        .as_ref()
        .map_or_else(|| "  (no HOME, so no default path)".to_string(), |p| format!("  {}", p.display()));
    Err(format!(
        "no application signing seed. Three places were looked at, in this order, and NOTHING \
         WAS BUILT:\n\
         \x20 --signing-key PATH   not given\n\
         \x20 ${APP_SEED_ENV}     not set\n\
         {looked}   not a file\n\
         \n\
         \x20 what   32 raw bytes, or 64 hex characters. `cargo rivet identity new` does NOT \
         mint one — that writes a TLS identity, which is a different key.\n\
         \x20 and    a key you generate yourself will not deploy to an existing board. Its \
         trust store is composed when the IMAGE is built and lives on the read-only /system \
         slot (ADR-028 §5.5.4), so a machine accepts exactly the keys it was built with. On a \
         board you did not build, the seed you need is the one whose public half is already in \
         its trust store — `rivet get trust` asks the machine which those are."
    ))
}

fn parse_package(args: &[String]) -> Result<PackageArgs, String> {
    let mut manifest_path = PathBuf::from("Cargo.toml");
    let mut signing_key: Option<PathBuf> = None;
    let mut target = DEFAULT_TARGET.to_string();
    let mut profile = "release".to_string();
    let mut bin = None;
    let mut binary = None;
    let mut target_dir = None;
    let mut out_dir = None;
    let mut assets = Vec::new();
    let mut no_assets = false;
    let mut public_key_out = None;

    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = || {
            args.get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        // Every option here takes a value except `--no-assets`, which is why
        // this is a variable rather than the `index += 2` it used to be. A
        // boolean flag advanced by two would silently eat the option after it.
        let mut step = 2;
        match flag {
            "--no-assets" => {
                no_assets = true;
                step = 1;
            }
            "--manifest-path" => manifest_path = PathBuf::from(value()?),
            "--signing-key" => signing_key = Some(PathBuf::from(value()?)),
            "--target" => target = value()?,
            "--profile" => profile = value()?,
            "--bin" => bin = Some(value()?),
            "--binary" => binary = Some(PathBuf::from(value()?)),
            "--target-dir" => target_dir = Some(PathBuf::from(value()?)),
            "--out-dir" => out_dir = Some(PathBuf::from(value()?)),
            "--public-key-out" => public_key_out = Some(PathBuf::from(value()?)),
            "--asset" => {
                let text = value()?;
                let (name, path) = text
                    .split_once('=')
                    .ok_or_else(|| format!("--asset takes NAME=PATH and got {text:?}"))?;
                assets.push((name.to_string(), PathBuf::from(path)));
            }
            other => return Err(format!("{other:?} is not an option.\n\n{}", usage())),
        }
        index += step;
    }

    Ok(PackageArgs {
        manifest_path,
        signing_key: match signing_key {
            Some(named) => named,
            None => resolve_signing_key()?,
        },
        target,
        profile,
        bin,
        binary,
        target_dir,
        out_dir,
        assets,
        no_assets,
        public_key_out,
    })
}

fn package(args: &[String]) -> Result<String, String> {
    package_to(args).map(|(_, report)| report)
}

/// **`cargo rivet package --print-manifest`** — render what a crate's
/// `[package.metadata.rivet]` would produce, and write nothing.
///
/// `docs/design-candidates/SDK-Adoption-Friction.md` §2.3:
/// [`Manifest::render`](rivet_core::package::application::Manifest::render)
/// (`rivet_core::package::application`) is the manifest format's only
/// specification, and three fixtures under `tools/qemu/` were hand-written
/// renderings of what it emits, kept in sync by a person reading the function
/// rather than by anything checking them against it. This is the fix the
/// friction entry names — a harness that needs a rendered manifest asks the
/// tool for one instead.
///
/// # Why this is not a mode of [`package_to`]
///
/// [`parse_package`] resolves a signing key unconditionally — `resolve_signing_key`
/// refuses loudly when none is found, which is correct for a verb that is about
/// to sign something and wrong for one that only reads `Cargo.toml`. Routing
/// this through the same parser would mean a manifest could not be printed on a
/// machine with no seed at all, which is exactly the machine a CI fixture or a
/// first read of this tool runs on. So this takes the one flag it needs
/// (`--manifest-path`) itself and never asks [`resolve_signing_key`] anything.
///
/// # Errors
///
/// An option this is not `--manifest-path`, a manifest path that does not
/// exist, or a `[package.metadata.rivet]` table [`crate_meta::read`] refuses.
fn print_manifest(args: &[String]) -> Result<String, String> {
    let mut manifest_path = PathBuf::from("Cargo.toml");
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--manifest-path" => {
                let value = args
                    .get(index + 1)
                    .cloned()
                    .ok_or("--manifest-path needs a value")?;
                manifest_path = PathBuf::from(value);
                index += 2;
            }
            other => {
                return Err(format!(
                    "{other:?} is not an option `--print-manifest` takes. Only \
                     --manifest-path PATH is read; nothing else is built or signed."
                ));
            }
        }
    }

    let manifest_path = manifest_path
        .canonicalize()
        .map_err(|e| format!("cannot find {}: {e}", manifest_path.display()))?;
    let meta = crate_meta::read(&manifest_path)?;
    Ok(meta.manifest.render())
}

/// **Refuse a program the supervisor will refuse to launch**, here, where the
/// person who can shrink it is standing.
///
/// `ADR-015` §4.2 hashes a program at every launch and reads at most
/// [`MAX_PROGRAM_BYTES`](rivet_core::package::application::MAX_PROGRAM_BYTES)
/// to do it. Nothing before this checked, so the whole
/// chain — package, sign, transfer, unpack, install — succeeded and reported
/// success, and the limit announced itself as `File too large (os error 27)` in
/// a supervisor's log on another machine. That is a true message about the
/// wrong moment.
///
/// **The advice matters more than the refusal.** Every over-limit binary this
/// project has produced was over because it carried debug symbols, so the fix
/// is `strip` and not architecture.
///
/// # Errors
///
/// The size, the limit, and what to do about it.
fn size_check(program: &Path, bytes: usize) -> Result<(), String> {
    use rivet_core::package::application::MAX_PROGRAM_BYTES;

    if bytes <= MAX_PROGRAM_BYTES {
        return Ok(());
    }
    Err(format!(
        "{} is {bytes} bytes, and a supervisor will not launch a program over {MAX_PROGRAM_BYTES}.\n\
         \n\
         ADR-015 §4.2 hashes a program at every launch, and reads at most that much to do\n\
         it — so this would package, sign, transfer and install without complaint, and then\n\
         refuse to start with `File too large (os error 27)`.\n\
         \n\
         It is almost certainly debug symbols rather than code. Strip it with the cross\n\
         toolchain's strip — tools/toolchain/env.sh sets $RIVET_STRIP — and package the\n\
         stripped copy with --binary.",
        program.display()
    ))
}

/// **A line about DWARF, when a program carries it** — `PLAN.md` §123, Wave 4.4.
///
/// # Why this is worth a line and is not a refusal
///
/// `ADR-015` §4.2 hashes the whole program **at every launch**. Debug symbols
/// are usually most of a binary — `rivet-sysd` is 14.4 MB with them and 1.1 MB
/// stripped, and `out/rivet-launcher-0.1.61` shipped at 10.6 MB — so an
/// unstripped application pays roughly a tenfold hash on every start and
/// restart, on a Pi, reading from an SD card. `size_check` above only refuses
/// past `MAX_PROGRAM_BYTES`, so everything under 16 MiB went through with
/// nothing said.
///
/// Not a refusal, because there is a real reason to ship symbols: `ADR-005`
/// §7.5 records that stripping the only copy of a released binary left a
/// debugger with `0x22d0d0 in ?? ()` against a board. That is the author's
/// trade to make; this is the sentence that makes them aware they are making
/// it.
///
/// # ~~It searches for a name, and that is the honest limit~~
///
/// ~~`crate::elf` reads program headers, and section names live in the section
/// header string table, which it does not parse — adding that to read one
/// boolean would be more ELF than this tool has any other reason to know. So
/// this looks for the literal `.debug_info` in the file's bytes.~~
///
/// ~~**A search can be wrong in one direction only**, which is why it is allowed
/// to be a search: a program that merely CONTAINS that string in its own data
/// gets a line advising a strip that will do nothing. It cannot go the other
/// way — a binary with a `.debug_info` section has that name in its string
/// table — and the cost of the false positive is one sentence.~~
///
/// # It reads the section headers — corrected 2026-09-04
///
/// **The false positive was every stripped Rust binary, and one sentence was
/// not what it cost.** `crate::elf::debug_sections` carries the measurement:
/// `std` links a DWARF reader for its own backtraces, that reader holds the
/// section names as data, and so `.debug_info` is present in the bytes of a
/// program with no debug sections at all. Found while packaging
/// `examples/probes/slow-stop-probe` for the board — the note fired, the cross
/// toolchain's `strip` removed nothing, and `readelf -S` showed nineteen
/// sections and no `.debug_*` among them.
///
/// A warning an author cannot act on is worse than no warning, because the
/// habit it teaches is to skip the next one.
///
/// **What it reports now** is which sections and how many bytes they are, so
/// the advice can be checked rather than believed — and an unreadable section
/// table says so instead of producing silence, because silence here is
/// indistinguishable from a clean binary.
fn debug_note(bytes: &[u8]) -> String {
    let sections = match elf::debug_sections(bytes) {
        Ok(sections) => sections,
        Err(fault) => {
            return format!(
                "symbols  this program's section headers could not be read: {fault}. Whether it\n\
                 \x20        carries DWARF is UNKNOWN — this is not a clean answer, and\n\
                 \x20        ADR-015 §4.2 hashes the whole program at every launch either way.\n"
            );
        }
    };
    if sections.is_empty() {
        return String::new();
    }
    let carried: u64 = sections.iter().map(|(_, size)| *size).sum();
    let names: Vec<&str> = sections.iter().map(|(name, _)| name.as_str()).collect();
    format!(
        "symbols  this program carries DWARF — {} section(s), {carried} bytes of {} as\n\
         \x20        packaged: {}\n\
         \x20        ADR-015 §4.2 hashes the whole program at EVERY launch, so the symbols are\n\
         \x20        paid for on every start and restart — typically ten times the stripped\n\
         \x20        size. Strip it with the cross toolchain's strip ($RIVET_STRIP, set by\n\
         \x20        tools/toolchain/env.sh) and package the stripped copy with --binary.\n\
         \x20        Keep the unstripped one: ADR-005 §7.5 is why a released binary needs a\n\
         \x20        copy with symbols SOMEWHERE, just not on the card.\n",
        sections.len(),
        bytes.len(),
        names.join(" "),
    )
}

/// The directory beside `Cargo.toml` whose contents are packaged as assets.
///
/// One name, not a flag with a default, because the point is that an author does
/// not have to know about it — see [`assets_in`].
const ASSETS_DIR: &str = "assets";

/// **Everything in `<crate>/assets/`, as members** — `ADR-015` §3's
/// *"optional assets"*, register `R-26`.
///
/// # What this cost before it existed
///
/// `--asset NAME=PATH` was the only way in, so an application's assets shipped
/// **only if somebody remembered to name each one on the command line**.
/// `PLAN.md` §119.3: `aura-renderer` was deployed without its four typefaces,
/// exited on `SourceCodePro-Bold.ttf: No such file or directory`, and the screen
/// stayed black for hours with the display hand-over working perfectly. Its own
/// source says the faces ship in the `.rvt`, and they did not. **Nothing
/// refused, because nothing had been asked for** — the failure was a silent
/// omission at package time surfacing as an unrelated-looking crash on a board.
///
/// A convention that needs no flag is the only fix that would have caught it,
/// since the defect was the flag not being typed.
///
/// # Flat, because the container is flat, and a subdirectory REFUSES
///
/// A member name is 32 bytes of printable ASCII with no `/` or `\`
/// (`rivet_core::package::format::read_name`), and
/// `rivet_core::package::install` writes each asset at
/// `<volume>/app/<name>/<version>/<member name>` — **beside the program, one
/// level, no directories.** So `assets/fonts/Geist.ttf` cannot be expressed.
///
/// It is refused rather than flattened. Flattening would install the file as
/// `Geist.ttf` while its author wrote `fonts/Geist.ttf` in their source, which
/// is a program that packages cleanly, installs cleanly and cannot open its own
/// font — the exact failure above, wearing a different hat.
///
/// # Everything there ships, and nothing is skipped quietly
///
/// A README in `assets/` is packaged. That is deliberate: the alternative is a
/// filter, and a filter is a rule about which files silently do not arrive,
/// which is the class of defect this function exists to remove. `--no-assets`
/// is the way out, and it is a thing the author typed.
///
/// # Errors
///
/// A name the container cannot hold, a subdirectory, or a directory that cannot
/// be read — each naming the path.
/// The members this tool is about to write, as `ADR-027` §3.2.2's digest takes
/// them.
///
/// **A projection, not a second list.** `emit::Part` stays the writer's type —
/// it carries a `Source` that owns its bytes — and `name::Member` borrows from
/// it, so the two cannot describe different payloads. A version composed from
/// anything but the parts that are written would be a name for bytes nobody
/// packaged, which is the whole class §3.2.2 is about.
fn payload_members(parts: &[emit::Part]) -> Vec<name::Member<'_>> {
    parts
        .iter()
        .map(|part| name::Member {
            name: &part.name,
            class: part.class,
            variant: part.variant,
            content: match &part.source {
                emit::Source::Bytes(bytes) => name::Content::Bytes(bytes),
                emit::Source::File(path) => name::Content::File(path),
            },
        })
        .collect()
}

fn assets_in(dir: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    use rivet_core::package::format::{M_NAME_BYTES, read_name};

    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            return Err(format!(
                "{} is a directory, and a package's members are FLAT.\n\
                 \n\
                 A member name is 32 bytes of printable ASCII with no path separator, and an\n\
                 asset installs beside the program at <volume>/app/<name>/<version>/<name> — so\n\
                 there is nowhere for a subdirectory to land. Packaging it as its basename would\n\
                 install a file at a path your program does not look in, which is worse than\n\
                 this refusal.\n\
                 \n\
                 Flatten {}/, or pass --no-assets and name what you want with --asset NAME=PATH.",
                path.display(),
                dir.display(),
            ));
        }
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            format!("{} is not a name this container can hold: it is not UTF-8", path.display())
        })?;
        if name.len() > M_NAME_BYTES {
            return Err(format!(
                "{} is {} bytes and a member name is at most {M_NAME_BYTES}. Rename it, or \
                 pass --no-assets and name it with --asset NAME=PATH.",
                path.display(),
                name.len(),
            ));
        }
        // The reader's own rule, run here rather than at `emit::write`, so the
        // refusal names a FILE the author can rename instead of a member of an
        // archive that was half composed.
        let mut field = [0u8; M_NAME_BYTES];
        field[..name.len()].copy_from_slice(name.as_bytes());
        read_name(&field).map_err(|fault| {
            format!(
                "{} is not a name this container can hold: {fault}",
                path.display()
            )
        })?;
        found.push((name.to_string(), path));
    }
    // **Sorted, so the artifact is a function of the directory and not of the
    // order a filesystem happened to hand the entries back.** Two packagings of
    // the same tree must produce the same bytes, and `ADR-027` §3.2.2 makes the
    // digest part of a name.
    found.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(found)
}

/// The walked assets and the named ones, as one list.
///
/// **A collision is a refusal, not an override.** Two members with one name is a
/// package whose reader has to choose, and `rivet_core::package::install` writes
/// them to the same path in file order — so the second silently wins and the
/// author finds out on a board. Whichever they meant, saying so here is cheaper.
///
/// # Errors
///
/// A `--asset` naming a member `assets/` already provides, naming both paths.
fn merge_assets(
    walked: Vec<(String, PathBuf)>,
    named: Vec<(String, PathBuf)>,
) -> Result<Vec<(String, PathBuf)>, String> {
    let mut all = walked;
    for (name, path) in named {
        if let Some((_, already)) = all.iter().find(|(n, _)| *n == name) {
            return Err(format!(
                "--asset {name}={} names a member that is already provided by {}.\n\
                 \n\
                 A package cannot hold two members with one name — both would install to the\n\
                 same path and the later one would silently win. Rename the file, or pass\n\
                 --no-assets and name every asset yourself.",
                path.display(),
                already.display(),
            ));
        }
        all.push((name, path));
    }
    Ok(all)
}

/// Refuse an asset whose name is one the installer writes into the
/// application's own version directory on its own — before the packager
/// reports anything or builds a single member.
///
/// **The same list `rivet_core::package::install::plan` refuses by**, so the
/// two cannot check against different names. This is the earlier, friendlier
/// half of that refusal: a name here costs the author a rename before a
/// build, rather than a signed artifact the installer accepts and then
/// refuses on a board.
///
/// # Errors
///
/// The first such name, naming both the file and what it collides with.
fn reject_reserved_asset_names(assets: &[(String, PathBuf)]) -> Result<(), String> {
    for (name, path) in assets {
        if rivet_core::package::install::RESERVED_APPLICATION_ASSET_NAMES.contains(&name.as_str()) {
            return Err(format!("{} cannot be an asset's name: the installer writes a file named {name:?} into the application's own version directory on its own, and an asset reusing that name would land on top of it rather than beside it. Rename the file, or pass --no-assets and name it differently with --asset NAME=PATH.", path.display()));
        }
    }
    Ok(())
}

/// What the program links, and whether the manifest accounts for it.
///
/// Three outcomes, and only one of them is a refusal:
///
/// * **static** — the ordinary case (`ADR-003` §5) and the case for every Rivet
///   application before 2026-08-31. One line, and no opinion.
/// * **dynamic, and the closure is not empty** — reported so the author can see
///   the SONAMEs beside the references they declared. This tool cannot check
///   that the declared libraries *provide* those SONAMEs, because a reference
///   names an artifact this process does not have; the board can and does.
/// * **dynamic, needing something outside the platform's public surface, and
///   nothing declared** — refused. That artifact cannot run anywhere, and the
///   only thing downstream of here that would notice is a loader.
fn linkage_report(
    program: &Path,
    manifest: &rivet_core::package::application::Manifest,
) -> Result<String, String> {
    use rivet_core::package::library::{LOADER_PATH, PUBLIC_LIBRARIES};

    let bytes = std::fs::read(program)
        .map_err(|e| format!("cannot read {}: {e}", program.display()))?;
    size_check(program, bytes.len())?;
    let dynamic = match elf::read(&bytes) {
        Ok(dynamic) => dynamic,
        // **Not a refusal.** `--binary` accepts a program this tool did not
        // build, and `ADR-015` §3.1 says a package is one static binary and is
        // deliberately language-agnostic about which. A file this reader cannot
        // parse is a file it has nothing to say about, and saying nothing is
        // better than refusing something the board would have accepted.
        Err(fault) => {
            return Ok(format!(
                "linkage  not read: {fault}. Nothing is claimed about what this program needs.\n"
            ));
        }
    };

    let debug = debug_note(&bytes);

    if dynamic.is_static() {
        return Ok(format!(
            "linkage  static — needs no loader and no library (ADR-003 §5)\n{debug}"
        ));
    }

    let interpreter = dynamic.interpreter.clone().unwrap_or_default();
    let mut out = format!("linkage  dynamic — interpreter {interpreter}\n{debug}");

    if interpreter != LOADER_PATH {
        return Err(format!(
            "{} names {interpreter:?} as its interpreter, and a Rivet has {LOADER_PATH:?}.\n\
             \n\
             The interpreter is a path the KERNEL opens, before the program's first\n\
             instruction, so a wrong one is not a missing library — it is `ENOENT` on exec\n\
             with nothing to diagnose it by. It comes from the toolchain that linked this\n\
             binary: a glibc cross-compiler writes a glibc loader's path, and musl is not\n\
             ABI-compatible with glibc (ADR-003 §5, and §8 of the shared-libraries design).\n\
             \n\
             Build it against the musl cross toolchain — tools/toolchain/env.sh sets the\n\
             variables that point at it.",
            program.display()
        ));
    }

    let unaccounted: Vec<&String> = dynamic
        .needed
        .iter()
        .filter(|soname| !PUBLIC_LIBRARIES.contains(&soname.as_str()))
        .collect();

    out.push_str(&format!(
        "         needs [{}]\n",
        dynamic.needed.join(", ")
    ));

    if unaccounted.is_empty() {
        out.push_str(
            "         every one of those is on the platform's public surface, so the closure \
             is empty and correct\n",
        );
        return Ok(out);
    }

    if manifest.library().is_empty() {
        return Err(format!(
            "{} needs {} at run time and declares no library closure.\n\
             \n\
             The platform promises exactly {:?} and nothing else — every other shared object\n\
             an application links has to arrive as its own signed library .rvt and be named\n\
             in this application's closure. As it stands this artifact could be installed and\n\
             would never start, and the only thing that would notice is a loader.\n\
             \n\
             Emit the libraries with `cargo rivet library`, then paste the references it\n\
             prints into Cargo.toml:\n\
             \n\
             \x20 [package.metadata.rivet]\n\
             \x20 library = [\"<name>-<version>-<64 hex>\"]",
            program.display(),
            unaccounted
                .iter()
                .map(|s| format!("{s:?}"))
                .collect::<Vec<_>>()
                .join(", "),
            PUBLIC_LIBRARIES,
        ));
    }

    out.push_str(&format!(
        "         closure declares {} reference(s); the board checks that they provide the \
         names above\n",
        manifest.library().len()
    ));
    Ok(out)
}

/// `package`, plus the path it wrote.
///
/// `cargo rivet install` needs the artifact and not the sentence about it, and
/// composing the two verbs is why this is split rather than having `install`
/// re-derive the name: `ADR-027` §7 gives the tool the name, and two places
/// computing it would be two answers to what the file is called (`ADR-011`).
fn package_to(args: &[String]) -> Result<(PathBuf, String), String> {
    let args = parse_package(args)?;

    let manifest_path = args
        .manifest_path
        .canonicalize()
        .map_err(|e| format!("cannot find {}: {e}", args.manifest_path.display()))?;
    let crate_dir = manifest_path
        .parent()
        .ok_or("the manifest path has no directory")?
        .to_path_buf();

    let meta = crate_meta::read(&manifest_path)?;
    let arch = name::arch_for(&args.target).ok_or_else(|| {
        format!(
            "{} is not a target this tool knows an <arch> token for (ADR-027 §3.1). Known: \
             aarch64-unknown-linux-musl, x86_64-unknown-linux-musl, arm-unknown-linux-musleabihf",
            args.target
        )
    })?;

    let seed = key::read_32(&args.signing_key, "signing key")?;
    let mut signer = Ed25519Signer::from_seed(&seed);
    let mut digest = Blake3::new();

    // **Absolute, because the cargo child is run in the crate's directory** —
    // see `build`. A relative `--target-dir` is relative to where the OPERATOR
    // stood, and it must not silently start meaning something else.
    let target_dir = match args.target_dir {
        Some(given) => absolute(&given)?,
        None => crate_dir.join("target"),
    };
    let out_dir = args.out_dir.unwrap_or_else(|| target_dir.join("rivet"));
    // **The BINARY name, not the package name** — `crate_meta::product_name`.
    // `rivetos-fleetd` builds `rivet-fleetd`, and defaulting to the package name
    // meant `--bin` was wrong for every crate whose two names differ.
    let bin = args
        .bin
        .clone()
        .or_else(|| meta.bin_name.clone())
        .unwrap_or_else(|| meta.name.clone());

    let mut report = String::new();
    let program = match args.binary {
        Some(path) => {
            report.push_str(&format!(
                "binary   {} (taken as given, not built)\n",
                path.display()
            ));
            path
        }
        None => {
            let built = build(
                &crate_dir,
                &manifest_path,
                &target_dir,
                &args.target,
                &args.profile,
                &bin,
            )?;
            report.push_str(&format!("built    {}\n", built.display()));
            built
        }
    };
    if !program.is_file() {
        return Err(format!("{} is not a file", program.display()));
    }

    // **Shared Rivets, 2026-08-31: what the program actually links, checked
    // against what its author declared.**
    //
    // The design's rule is *"refusal at install, never a loader error at
    // start"*. This is that rule moved one step further back again, to the only
    // place where the program and its `Cargo.toml` are both in front of the same
    // process — a mismatch found here costs a rebuild, and the same mismatch
    // found on a board costs a deploy, an install and a service that will not
    // start.
    report.push_str(&linkage_report(&program, &meta.manifest)?);

    let manifest_text = meta.manifest.render();

    // Every application member's variant is zero — an application has no A/B,
    // and the byte is half of a member's identity (`ADR-029` §13.2 item 4).
    let variant = rivet_core::package::application::VARIANT_NONE;
    let mut parts = vec![
        emit::Part {
            name: MANIFEST_NAME.to_string(),
            class: TargetClass::Manifest.as_byte(),
            variant,
            source: emit::Source::Bytes(manifest_text.clone().into_bytes()),
        },
        emit::Part {
            name: bin.clone(),
            class: TargetClass::Program.as_byte(),
            variant,
            source: emit::Source::File(program.clone()),
        },
    ];
    // **`assets/` first, then whatever was named on the command line** — see
    // [`assets_in`] for what the flag-only version cost.
    //
    // A `--asset` that repeats a walked name is a REFUSAL rather than an
    // override: two members with one name is a package whose reader has to pick,
    // and `rivet_core::package::install` would write one over the other in file
    // order. Whichever the author meant, saying so is cheaper than finding out
    // on a board.
    let walked = if args.no_assets {
        Vec::new()
    } else {
        assets_in(&crate_dir.join(ASSETS_DIR))?
    };
    let asset_parts = merge_assets(walked, args.assets)?;
    reject_reserved_asset_names(&asset_parts)?;
    if asset_parts.is_empty() {
        report.push_str(&format!("assets   none ({ASSETS_DIR}/ is absent or empty)\n"));
    } else {
        report.push_str(&format!(
            "assets   {} — {}\n",
            asset_parts.len(),
            asset_parts
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for (asset_name, path) in asset_parts {
        parts.push(emit::Part {
            name: asset_name,
            class: TargetClass::Asset.as_byte(),
            variant,
            source: emit::Source::File(path),
        });
    }

    // **`ADR-027` §3.2.2: a `-dirty` version is made unique by the bytes it
    // names.** Composed HERE, after every member exists, because the digest is
    // of the payload — a version computed before the build would be a name for
    // bytes nothing had produced yet. A clean version is returned untouched.
    //
    // **It moved down the function on 2026-09-04**, and that move IS the fix.
    // It used to sit immediately after the build and hash the program alone, so
    // an edit to the manifest — the half the machine acts on — produced the same
    // version, `ADR-015` §7.2 refused the install as already present, and the
    // change silently did not happen. `name::Member` is the whole payload.
    // **The escape hatch `tools/card/bump.sh` already has for the system
    // image** — `AGENTS.md` §10.5, `PLAN.md` §137.7's neighbour: a payload
    // built in the container is stamped `-dirty` on a clean tree because the
    // container's `git status --porcelain` disagrees with Windows' about line
    // endings. Unset (the default), this asks the git it runs with, same as
    // before — wrong in the safe direction, and this does not touch that.
    // Set, it takes the caller's word for the one fact that actually
    // disagrees and asks git for nothing else.
    let version = match dirty_override()? {
        Some(dirty) => name::Version::compose_with_dirty(&meta.version, &crate_dir, dirty),
        None => name::Version::compose(&meta.version, &crate_dir),
    }
    .distinguished_by(&payload_members(&parts))?;

    let out = out_dir.join(name::artifact(
        // The machine-facing name, so the FILE and the header agree — see
        // `crate_meta::product_name`.
        crate_meta::product_name(&meta),
        version.as_str(),
        arch,
        None,
        &Kind::application(),
    ));
    let written = emit::write(
        &out,
        &emit::Identity {
            // The name this application has ON A MACHINE — see
            // `crate_meta::product_name` for why it is not `meta.name`.
            product: crate_meta::product_name(&meta),
            version: version.as_str(),
            arch,
            // the maintainer's 2026-08-24 ruling: an application is architecture-shaped.
            platform: None,
            kind: Kind::application(),
        },
        &parts,
        // An application has no boot directory, so there is nothing a platform
        // reserves on it — `ADR-029` §4 is about a payload target.
        &[],
        &mut digest,
        &mut signer,
    )?;

    let public = signer.public_key();
    let key_id = KeyId(digest.of(&public.0));
    if let Some(path) = args.public_key_out {
        std::fs::write(&path, public.0)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        report.push_str(&format!("public   {} (32 raw bytes)\n", path.display()));
    }

    report.push_str(&format!(
        "wrote    {}\n\
         product  {}\n\
         version  {}  ({})\n\
         arch     {}   (no <platform>: an application is architecture-shaped — ADR-027 §9 item 3)\n\
         kind     {}\n\
         size     {} bytes\n\
         blake3   {}\n\
         key      {} (id {})\n",
        written.path.display(),
        crate_meta::product_name(&meta),
        version.as_str(),
        version.how(),
        arch,
        Kind::application(),
        written.total_len,
        key::hex(&written.digest),
        key::hex(&public.0),
        key_id,
    ));
    report.push_str("members\n");
    for (member_name, length) in &written.members {
        report.push_str(&format!("  {member_name:<24} {length:>12} bytes\n"));
    }
    report.push_str("\nmanifest\n");
    for line in manifest_text.lines() {
        report.push_str(&format!("  {line}\n"));
    }

    // `ADR-015` §6.1: an unenforced declaration must be VISIBLE. Today the list
    // is everything, because nothing installs and nothing runs an application.
    report.push('\n');
    for field in meta.manifest.unenforced() {
        report.push_str(&format!(
            "  ⚠ {field} is NOT ENFORCED by any Rivet that exists\n"
        ));
    }
    report.push_str(
        "\nA card built by tools/card/build.sh carries a trust store (ADR-028 §5.8), so a\n\
         Rivet can verify this — IF the key that signed it is in that store AND is\n\
         entitled for `application`. A system key deliberately is NOT: cmd/rivet-image\n\
         asserts it. Install with `cargo rivet install --rvt FILE.rvt … confirm`, which\n\
         transfers it into the machine's artifact store and then asks for\n\
         `rivet apply install <digest> confirm` — an install names a digest and never\n\
         a path (ADR-052 §7), and the work goes to rivet-deploy (ADR-029 §14)\n\
         rather than to the caller that asked.\n\
         \n\
         THAT INSTALLS IT AND DOES NOT RUN IT (ADR-015 §3.2.2). `enabled` is the\n\
         operator's and lives in the machine's document, so an install on its own\n\
         leaves the application idle with nothing to diagnose. `cargo rivet install\n\
         --enable` does both in one command; by hand it is `rivet apply edit` on the\n\
         document, adding `[services.<name>]` with `enabled = true`, then\n\
         `rivet apply reload confirm`.\n",
    );
    Ok((written.path, report))
}

/// Run `cargo build`. **Its output reaches the caller as cargo writes it** — a
/// cross build prints warnings and a linker error is the thing the caller most
/// needs to see, and swallowing it to re-print a summary would be this tool
/// standing between a developer and their compiler.
///
/// # Since 2026-08-30 the caller's stderr is also *read*
///
/// `stdout` is still inherited and untouched. `stderr` is piped and pumped
/// line by line to this process's own stderr, so nothing is buffered, reordered
/// or withheld — and the text is kept, because of what the second cold-agent run
/// hit:
///
/// > `cargo build --release --target aarch64-unknown-linux-musl` in my own
/// > crate: `error: linker 'cc' not found`
///
/// The README presents that exact command with *"cross-compiling for the board
/// needs no C toolchain, because Rivet's own binaries are pure Rust"*, which is
/// true and quietly depends on **this repository's** `.cargo/config.toml`
/// setting `linker = "rust-lld"`. **An application crate outside the repository
/// does not inherit it**, and `cargo rivet package` shells out to `cargo build`
/// and hits the same wall with the same bare error and no hint.
///
/// # Why this explains rather than passing the linker itself
///
/// Setting `CARGO_TARGET_<TRIPLE>_LINKER` here would make the build succeed, and
/// it was rejected:
///
/// 1. **It would make two commands disagree about one crate.** `cargo build` and
///    `cargo rivet package` in the same directory would link differently and
///    produce different binaries from the same source, with nothing saying so.
///    That is `ADR-011`'s failure in the place it is hardest to see, and
///    `AGENTS.md` §5's *"a wrong guess does not fail — it succeeds"*.
/// 2. **The linker is the crate author's to choose.** An environment variable
///    set by this tool overrides a `.cargo/config.toml` the author wrote
///    deliberately, so the one case where the guess is wrong is the case where
///    somebody made a decision.
/// 3. **The fact is worth knowing once.** A crate that cannot cross-compile on
///    its own cannot be built by CI, by `cargo build`, or by anything but this
///    tool; papering over it here hides that from the person who can fix it.
///
/// **What it costs is a round trip, and the round trip is not extended**: the
/// explanation arrives on the failure that already happened, not on a re-run.
fn build(
    crate_dir: &Path,
    manifest_path: &Path,
    target_dir: &Path,
    target: &str,
    profile: &str,
    bin: &str,
) -> Result<PathBuf, String> {
    let mut command = Command::new("cargo");
    command
        // **The child builds in the crate's own directory**, since 2026-08-30,
        // and until then it inherited whatever directory the operator happened
        // to be standing in. Cargo resolves `.cargo/config.toml` from the
        // WORKING DIRECTORY and its ancestors, so the same crate packaged from
        // inside this repository picked up this repository's `rust-lld` and
        // packaged from its own directory did not — the same command, the same
        // crate, and a different answer depending on where it was typed. That
        // is worse than failing: it makes the diagnosis above fire for some
        // callers and not others, and it would let a payload be built with a
        // linker setting the crate does not have.
        //
        // Every path handed to cargo below is absolute, so moving the child
        // changes what it READS and nothing about what it is asked to do.
        .current_dir(crate_dir)
        .arg("build")
        .arg("--manifest-path")
        .arg(manifest_path)
        .arg("--target")
        .arg(target)
        .arg("--target-dir")
        .arg(target_dir)
        .arg("--bin")
        .arg(bin)
        .arg("--profile")
        .arg(profile)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        // Piped so it can be READ as well as shown — see this function's
        // documentation. It is written straight back out a line at a time, so
        // the caller sees what cargo said, when cargo said it.
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|e| format!("cannot run cargo: {e}"))?;
    let mut said = String::new();
    if let Some(stream) = child.stderr.take() {
        for line in BufRead::lines(BufReader::new(stream)) {
            let line = line.map_err(|e| format!("cannot read cargo's stderr: {e}"))?;
            eprintln!("{line}");
            said.push_str(&line);
            said.push('\n');
        }
    }
    let status = child
        .wait()
        .map_err(|e| format!("cannot wait for cargo: {e}"))?;
    if !status.success() {
        return Err(format!(
            "cargo build failed ({status}); nothing was packaged.{}{}",
            linker_hint(&said, target),
            compiler_hint(&said, target),
        ));
    }

    // `--target-dir` is passed explicitly so this path is arithmetic rather than
    // a guess: without it, a crate that is a workspace member builds into the
    // workspace's target directory and a standalone crate into its own, and a
    // tool that guessed would find a stale binary from a previous build about as
    // often as it found nothing.
    //
    // Cargo's directory for the two built-in profiles is not their name: `dev`
    // builds into `debug`. Every other profile's directory is its name.
    let directory = match profile {
        "dev" => "debug",
        other => other,
    };
    let built = target_dir.join(target).join(directory).join(bin);
    if built.is_file() {
        return Ok(built);
    }
    // Windows hosts building for Windows would want `.exe`; this tool targets
    // Linux, so an absent binary here means the `--bin` name is wrong rather
    // than that a suffix is missing. Say which.
    Err(format!(
        "cargo build succeeded and {} is not there; is {bin:?} the right --bin name?",
        built.display()
    ))
}

/// A path as this process sees it, resolved against the process's own working
/// directory.
///
/// **`std::path::absolute` rather than `canonicalize`**, because the directory
/// may not exist yet: `--target-dir` names where cargo should put things, and
/// requiring it to be there already would refuse a first build. It resolves no
/// symlinks and touches no disk, which is the whole of what is needed — the
/// child is moved, so the only thing that must not change is what the path
/// MEANS.
fn absolute(path: &Path) -> Result<PathBuf, String> {
    std::path::absolute(path)
        .map_err(|e| format!("cannot resolve {} against this directory: {e}", path.display()))
}

/// **What a bare `linker 'cc' not found` is actually telling you**, appended to
/// this tool's own failure when — and only when — cargo said it.
///
/// Pure, and reads only what the compiler wrote: `AGENTS.md` §5's *"report what
/// the system said, not what it was expected to say"*. It matches nothing on a
/// build that failed for any other reason, so a compile error is never
/// decorated with advice about a linker.
///
/// `cc` is the name a *missing* linker takes; the message quotes whatever cargo
/// named, so a host whose default is spelled differently still gets a hint that
/// matches what it saw.
fn linker_hint(said: &str, target: &str) -> String {
    let missing = said
        .lines()
        .find(|line| line.contains("linker") && line.contains("not found"));
    let Some(missing) = missing else {
        return String::new();
    };
    let variable = format!(
        "CARGO_TARGET_{}_LINKER",
        target.replace('-', "_").to_uppercase()
    );
    format!(
        "\n\n\
         THAT IS A MISSING CROSS LINKER, NOT A MISSING C COMPILER, and this tool did not\n\
         set one for you on purpose. Cargo said:\n\
         \x20 {missing}\n\
         \n\
         Rivet's own binaries are pure Rust and need no C toolchain — but cargo still\n\
         asks a LINKER to produce the executable, and for\n\
         \x20 {target}\n\
         it defaults to `cc`, which is a C compiler your host does not have for that\n\
         architecture. The Rust toolchain ships one that works: `rust-lld`. THIS\n\
         REPOSITORY SETS IT in its own `.cargo/config.toml`, and a crate outside the\n\
         repository does not inherit that — cargo reads that file from the CRATE, and\n\
         this tool builds your crate in your crate's own directory so that\n\
         `cargo rivet package` and `cargo build` resolve it the same way.\n\
         \n\
         Say it once, in YOUR crate, in `.cargo/config.toml` beside its `Cargo.toml`:\n\
         \n\
         \x20 [target.{target}]\n\
         \x20 linker = \"rust-lld\"\n\
         \n\
         Then `cargo build --target {target}` works on its own, which is the point:\n\
         this tool is not the only thing that has to be able to build your crate. For\n\
         one command without editing anything, set this in the environment:\n\
         \x20 {variable}=rust-lld\n\
         \n\
         cargo-rivet deliberately does NOT set either for you: it would make\n\
         `cargo build` and `cargo rivet package` link the same source differently with\n\
         nothing saying so, and the linker is yours to choose (ADR-011)."
    )
}

/// **What a bare `Failed to find tool` is actually telling you** — the same job
/// [`linker_hint`] does, for the failure on the other side of the line it draws.
///
/// # Why this exists, and it is not hypothetical
///
/// `linker_hint` opens by saying *"THAT IS A MISSING CROSS LINKER, NOT A MISSING
/// C COMPILER"*. This is the missing C compiler, and until 2026-08-30 an author
/// who hit it got a bare `cc-rs` error and no hint at all — while `README.md`
/// told them *"cross-compiling for the board needs no C toolchain"*, which had
/// been false since 2026-08-28.
///
/// **Two cold agents hit it and neither diagnosed it.** Both reached for
/// `rusqlite`, both failed to build it, and both independently converged on
/// `redb` and presented the switch as a preference rather than as a wall they
/// had been stopped by. A capability nobody wrote down is one nobody has.
///
/// # Why it explains rather than setting `CC` itself
///
/// The same three reasons `build`'s own documentation gives for the linker, and
/// the first is decisive: `cargo build` and `cargo rivet package` in one
/// directory would produce different binaries from one source with nothing
/// saying so — `ADR-011`'s failure in the place it is hardest to see.
///
/// The second reason is *weaker* here and it is worth saying so rather than
/// pretending the cases are identical: a linker is a genuine choice, whereas
/// there is exactly one compiler that can emit `aarch64` objects for this
/// target. What survives is the third reason — a crate that cannot cross-compile
/// on its own cannot be built by CI or by `cargo build`, and papering over that
/// here hides it from the only person who can fix it.
fn compiler_hint(said: &str, target: &str) -> String {
    // What `cc-rs` says when no compiler for the target can be found. Matched on
    // the phrases it writes rather than on a whole line, because the message is
    // wrapped differently depending on how cargo renders the build-script
    // failure — and matched on BOTH so that an unrelated line containing the
    // word "tool" cannot decorate a compile error with advice about a compiler.
    let missing = said.lines().find(|line| {
        (line.contains("Failed to find tool") || line.contains("failed to find tool"))
            || (line.contains("No such file or directory") && line.contains("gcc"))
    });
    let Some(missing) = missing else {
        return String::new();
    };
    let variable = format!("CC_{}", target.replace('-', "_"));
    format!(
        "

         THAT IS A MISSING CROSS C COMPILER, and it is a different failure from the
         missing linker above. Your crate compiles C — a `cc`-based build script, which
         `rusqlite`, `ring` and anything with a vendored library all have. cargo said:
           {missing}
         
         THIS IS SUPPORTED AND IT IS NOT A DEAD END. Rivet builds C for the board every
         time it builds itself: `rivetos-tls` links `ring`, whose build script compiles
         C and assembly. What is missing is only the name of the compiler.
         
         It comes from Buildroot's output tree, which `RIVET_BR_OUTPUT` names:
           {variable}=$RIVET_BR_OUTPUT/host/bin/aarch64-linux-gcc
         
         and the matching archiver, which some build scripts also want:
           AR_{ar}=$RIVET_BR_OUTPUT/host/bin/aarch64-linux-ar
         
         Two things about the target that decide whether a crate can work at all,
         BEFORE you spend an afternoon on one — README.md has the full list:
           * linking is STATIC ONLY. A .so builds cleanly and then fails at exec,
             naming an interpreter rather than the binary (ADR-003 §4, ADR-008).
           * there is NO pkg-config. A crate that probes for a system library finds
             the HOST's and produces an x86-64 object. Prefer the feature that
             compiles its own C from source -- `rusqlite`'s is `bundled`.
         
         cargo-rivet deliberately does NOT set the compiler for you: it would make
         `cargo build` and `cargo rivet package` produce different binaries from one
         source with nothing saying so (ADR-011), and a crate that cannot
         cross-compile on its own cannot be built by CI either.",
        ar = target.replace('-', "_")
    )
}

// ---------------------------------------------------------------------------
// inspect
// ---------------------------------------------------------------------------

/// `install`, with the real control plane and the real `package` behind it.
///
/// The seam is [`fleet::publish_then_send`]'s parameters — the repository, the
/// seed lookup, the builder and the machine — and only this function fills
/// them with things that touch the world.
fn deploy_cmd(args: &[String]) -> Result<String, String> {
    // **`ADR-052` §7: an install publishes into the fleet repository, then
    // sends.** `--at` and `--artifact` are read out of argv first; an argv with
    // neither reaches `deploy::parse` unchanged.
    let route = fleet::route_deploy(args)?;
    // `PLAN-Feature-Set-2026-09-12` §7.1: a bare leading device name is
    // resolved against `rivet-fleet`'s own register into `--host`,
    // `--identity` and `--pin` before `deploy::parse` ever runs — see
    // `crate::run::execute` for the same rewrite on `run`'s side. The name is
    // kept, because the next steps an install prints are spelled with it.
    let invocation = fleet::Invocation {
        selected: route.selected,
        device: device::leading_device_name(&route.args).map(str::to_string),
        at: route.at.clone(),
    };
    // A plan connects to nothing, so a device name that does not resolve from
    // here is named in it rather than refused: its NEEDS say what the run with
    // `confirm` will reach.
    let expanded = if route.args.last().is_some_and(|word| word == "confirm") {
        device::expand_from_default(&route.args)?
    } else {
        device::expand_from_default_for_a_plan(&route.args)?
    };
    let parsed = deploy::parse(&expanded)?;
    let root = fleet::deploy_root(route.at.as_deref())?;

    // The same lookup `cargo rivet package` uses, asked only when a publish is
    // attempted (`.rulings/` R-155 R3 is the ruling; `ADR-052` §7 the record).
    let mut app_seed = || {
        fleet::deploy_app_seed(
            &credentials::Overrides::from_environment(),
            credentials::root().as_deref(),
        )
    };
    // `R-177` D6: a system update publishes too, verified against the SYSTEM
    // seed rather than the application one — `cargo rivet image`'s own lookup,
    // asked here only when the file being deployed claims to be an update.
    let mut system_seed = || {
        fleet::deploy_system_seed(
            &credentials::Overrides::from_environment(),
            credentials::root().as_deref(),
        )
    };

    // **A plan is given no transport**, so it reads no private key and needs
    // nothing it has not been granted: its NEEDS are how it says what the run
    // with `confirm` will read and reach.
    if !parsed.confirmed {
        return fleet::publish_then_send(
            &root,
            parsed,
            &invocation,
            &mut app_seed,
            &mut system_seed,
            &mut package_to,
            &mut PlanOnly,
        );
    }

    // **One transport, opened by `rivet_remote::endpoint`** — which is also
    // what `cargo rivet run` opens a machine with, so both verbs reach one
    // through exactly this code rather than a second spelling of it
    // (`ADR-011` §3).
    let endpoint = rivet_remote::endpoint::Endpoint {
        host: Some(parsed.host.clone()),
        identity: parsed.identity.clone(),
        pin: parsed.pin.clone(),
    };
    let mut board = endpoint.open()?.ok_or(
        "install needs --identity and --pin: ADR-035 §7.2's control plane is how a payload \
         reaches a machine, and its trust is mutual — a client with no key of its own has \
         nothing to present. NOTHING HAS BEEN BUILT and nothing has been sent.\n\
         \x20 fix  `cargo rivet identity new --directory <dir>` mints one, and the machine's \
         own pin is what `cargo rivet key <seed>` derives.",
    )?;
    fleet::publish_then_send(
        &root,
        parsed,
        &invocation,
        &mut app_seed,
        &mut system_seed,
        &mut package_to,
        &mut board,
    )
}

/// **The machine a plan is handed.** A plan connects to nothing, so every call
/// is refused, and one arriving here would be a plan doing what it says it
/// does not.
struct PlanOnly;

/// What [`PlanOnly`] answers every call with.
const A_PLAN_CONNECTS_TO_NOTHING: &str =
    "internal error: a plan reached for the machine, and a plan connects to nothing";

impl rivet_remote::deploy::Remote for PlanOnly {
    fn push(&mut self, _: &std::path::Path, _: &str) -> Result<String, String> {
        Err(A_PLAN_CONNECTS_TO_NOTHING.to_owned())
    }
    fn install(&mut self, _: &str) -> Result<String, String> {
        Err(A_PLAN_CONNECTS_TO_NOTHING.to_owned())
    }
    fn read_document(&mut self) -> Result<Option<String>, String> {
        Err(A_PLAN_CONNECTS_TO_NOTHING.to_owned())
    }
    fn edit_document(&mut self, _: &rivet_remote::deploy::ConfigChange) -> Result<String, String> {
        Err(A_PLAN_CONNECTS_TO_NOTHING.to_owned())
    }
    fn put_document(&mut self, _: &str) -> Result<(), String> {
        Err(A_PLAN_CONNECTS_TO_NOTHING.to_owned())
    }
    fn ask(&mut self, _: &[&str]) -> Result<String, String> {
        Err(A_PLAN_CONNECTS_TO_NOTHING.to_owned())
    }
    fn blake3(&mut self, _: &str) -> Result<[u8; 32], String> {
        Err(A_PLAN_CONNECTS_TO_NOTHING.to_owned())
    }
}

/// `secrets`' words with a leading device resolved: `<device> <application>
/// …` becomes `<application> --host … --identity … --pin … …`, through the
/// same register lookup `run <device>` and `install <device>` use. Words with
/// one bare word, the application alone, pass through unchanged.
fn secrets_words(args: &[String]) -> Result<Vec<String>, String> {
    let Some((device, application, rest)) = secrets::leading_device(args) else {
        return Ok(args.to_vec());
    };
    let mut named = vec![device.to_owned()];
    named.extend(rest.iter().cloned());
    let expanded = device::expand_from_default(&named).map_err(|why| {
        format!(
            "{why}\n\nTwo bare words are read as DEVICE APPLICATION, so `{device}` was taken for a \
             device. This verb installs into ONE application at a time."
        )
    })?;
    let mut words = vec![application.to_owned()];
    words.extend(expanded);
    Ok(words)
}

/// `apk.json`'s fields as `inspect` prints them — `ADR-061` §3. The key column
/// is two wider than the longest key, so every key is followed by a space.
fn apk_metadata_lines(metadata: &rivet_core::package::apk::Metadata) -> String {
    const WIDTH: usize = "application_id".len() + 2;
    let rows = [
        ("application_id", metadata.application_id().to_string()),
        ("version", format!("{} ({})", metadata.version_name(), metadata.version_code())),
        ("label", metadata.label().to_string()),
        ("sdk", format!("min {}, target {}", metadata.min_sdk(), metadata.target_sdk())),
        ("signer", metadata.signer_sha256().to_string()),
        ("development", metadata.development().to_string()),
    ];
    rows.iter().map(|(key, value)| format!("{key:<WIDTH$}{value}\n")).collect()
}

fn inspect_cmd(args: &[String]) -> Result<String, String> {
    let file = args
        .first()
        .filter(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .ok_or_else(|| format!("inspect needs a file.\n\n{}", usage()))?;

    let mut trusted_key: Option<PathBuf> = None;
    let mut extract_program: Option<PathBuf> = None;
    let mut index = 1;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = args
            .get(index + 1)
            .cloned()
            .ok_or_else(|| format!("{flag} needs a value"))?;
        match flag {
            "--trusted-key" => trusted_key = Some(PathBuf::from(value)),
            "--extract-program" => extract_program = Some(PathBuf::from(value)),
            other => return Err(format!("{other:?} is not an option.\n\n{}", usage())),
        }
        index += 2;
    }
    let trusted_key = trusted_key
        .ok_or("--trusted-key is required: verification without a trust set is not verification")?;

    let raw = key::read_32(&trusted_key, "trusted public key")?;
    let trusted = rivet_core::package::port::PublicKey(raw);
    let mut digest = Blake3::new();
    let verifier = Ed25519Verifier::new();

    // `ADR-029` §4's reserved names, from the port — the same answer a board
    // gets. An update payload carrying one is refused here rather than being
    // listed as if it were installable.
    let boot_target = rivet_adapter_image::PiFirmware
        .payload(rivet_adapter_image::SlotId(1))
        .map_err(|e| format!("cannot describe slot A's targets: {e}"))?;
    let reserved = rivet_adapter_image::PiFirmware.reserved_names(&boot_target.kernel);

    let found = inspect::inspect(&file, trusted, reserved, &mut digest, &verifier)?;
    let header = found.package.header();

    let mut report = format!(
        "verified {}\n\
         product  {}\n\
         version  {}\n\
         arch     {}\n\
         platform {}\n\
         kind     {}\n\
         key      {}\n\
         members  {} (all verified)\n",
        file.display(),
        header.product(),
        header.version(),
        header.arch(),
        header
            .platform()
            .unwrap_or("none — an application carries none (ADR-027 §9 item 3)"),
        header.kind(),
        header.key_id(),
        found.members.len(),
    );

    match &found.details {
        inspect::Details::Application {
            manifest,
            program,
            identity,
        } => {
            for (index, member_name, class, _, length) in &found.members {
                let described = match TargetClass::from_byte(*class) {
                    Some(TargetClass::Manifest) => "manifest",
                    Some(TargetClass::Program) => "program",
                    Some(TargetClass::Asset) => "asset",
                    None => "unknown",
                };
                report.push_str(&format!(
                    "  {index}  {member_name:<24} {described:<9} {length:>12} bytes\n"
                ));
            }

            // **Who it will be, before what it asks for.** `ADR-015` §5.1.1.
            // A permission request means something different depending on the
            // account making it, and the account is the thing this tool could
            // not previously say (`PLAN.md` §102.6, Pile B item 2).
            report.push_str("\nruns as\n");
            match identity {
                Some(id) => report.push_str(&format!(
                    "  uid {uid}, gid {gid} — DERIVED from the name `{name}`, not allocated, so \
                     it is the same on every machine for ever (ADR-015 §5.1.1). The account is \
                     created with the application and is never pre-created; its shell field is \
                     empty, because an application account is an identity and not a way in.\n  \
                     writes {dir} — mode {mode:04o}, owned by that uid, and it is the ONLY path \
                     this application can write (ADR-015 §3.2A). /system is read-only and the \
                     rest of /state belongs to root. The directory is a SIBLING of the version \
                     directories, so installing a new version replaces the program and leaves \
                     the data.\n  a uid COLLISION with another application on the target machine \
                     is refused there rather than here: this tool sees one package and the \
                     supervisor is the only thing that holds every definition at once \
                     (ADR-015 §5.1.1).\n",
                    uid = id.uid,
                    gid = id.gid,
                    name = id.name,
                    dir = id.data_dir,
                    mode = id.data_dir_mode,
                )),
                None => report.push_str(&format!(
                    "  NOTHING — the header's product {:?} is not a name a machine can make a \
                     service of, so no identity is derived from it and this payload would be \
                     REFUSED at install rather than run as anything (ADR-015 §7.1.1, \
                     ADR-017 §3.1.3). A number is not reported here, because a number for a \
                     name nothing will accept is worse than none.\n",
                    header.product(),
                )),
            }

            report.push_str("\nrequests\n");
            let requests = inspect::requests(manifest);
            if requests.is_empty() {
                report.push_str("  nothing beyond a private data directory\n");
            }
            for request in requests {
                report.push_str(&format!("  {request}\n"));
            }
            for field in manifest.unenforced() {
                report.push_str(&format!(
                    "  ⚠ {field} is NOT ENFORCED by any Rivet that exists (ADR-015 §6.1)\n"
                ));
            }

            if let Some(to) = extract_program {
                let (index, program_name, _) = program;
                let length = inspect::extract(&file, &found.package, *index, &mut digest, &to)?;
                report.push_str(&format!(
                    "\nextracted {program_name} to {} ({length} bytes), after its digest \
                     matched\n",
                    to.display()
                ));
            }
        }
        // Shared Rivets, 2026-08-31. The `reference` line is the point of this
        // arm: it is what an application's `library = [...]` holds, and a build
        // script can read it back out of the artifact rather than keeping a
        // copy of what `cargo rivet library` once printed.
        inspect::Details::Library {
            manifest,
            objects,
            reference,
        } => {
            report.push_str(&format!("reference {reference}\n"));
            report.push_str("\nobjects\n");
            for (index, object_name, length) in objects {
                report.push_str(&format!(
                    "  {index}  {object_name:<32} {length:>12} bytes\n"
                ));
            }
            report.push_str("\nprovides\n");
            for soname in manifest.soname() {
                report.push_str(&format!("  {soname}\n"));
            }
            report.push_str("\nneeds\n");
            if manifest.needs().is_empty() {
                report.push_str(
                    "  nothing beyond the platform's public surface — see \
                     rivet_core::package::library::PUBLIC_LIBRARIES\n",
                );
            }
            for needed in manifest.needs() {
                report.push_str(&format!("  {needed}\n"));
            }
            report.push_str(
                "\nNOTHING RUNS A LIBRARY. It installs to /state/lib/<reference>/lib on a \
                 board and sits there for the applications that declare it; there is no \
                 `enabled`, no service and no `current` link — a library has no single \
                 current version, which is the whole point of the reference above.\n",
            );
        }
        inspect::Details::Update {
            system,
            slot_a,
            slot_b,
        } => {
            for (index, member_name, class, variant, length) in &found.members {
                let described = match UpdateTarget::from_byte(*class) {
                    Some(UpdateTarget::WholePartition) => "/system",
                    Some(UpdateTarget::BootDirectory) => "boot",
                    None => "unknown",
                };
                let slot = match SlotVariance::from_byte(*variant) {
                    Some(SlotVariance::Invariant) => "either",
                    Some(SlotVariance::SlotA) => "slot A",
                    Some(SlotVariance::SlotB) => "slot B",
                    None => "?",
                };
                report.push_str(&format!(
                    "  {index}  {member_name:<24} {described:<8} {slot:<7} {length:>12} bytes\n"
                ));
            }
            // Not the member table read again — this is `Payload`'s answer, the
            // same selection `rivet_adapter_image::install` makes on a board.
            report.push_str(&format!(
                "\ninstalled into slot A: member {system} to the /system partition, then \
                 {}\ninstalled into slot B: member {system} to the /system partition, then \
                 {}\n",
                slot_a.join(", "),
                slot_b.join(", "),
            ));
            if extract_program.is_some() {
                return Err(
                    "--extract-program is an application's flag; an update payload has no \
                     program member. ADR-029 §11: one container, and the metadata means \
                     different things per kind."
                        .to_string(),
                );
            }
            report.push_str(
                "\nThis is a SYSTEM UPDATE. A card built by tools/card/build.sh carries a trust\n\
                 store (ADR-028 §5.8) and a board verifies against THAT; `--trusted-key` above is\n\
                 one key on a command line, which is a developer checking their own artifact\n\
                 rather than a fleet.\n",
            );
        }
        // `R-177`, 2026-09-13. One opaque member; nothing here interprets its
        // bytes, so there is nothing beyond the container's own fields to add.
        inspect::Details::Card { stored_len, plain_len, compression } => {
            let codec = match *compression {
                rivet_core::package::read::COMPRESSION_NONE => "none",
                rivet_core::package::read::COMPRESSION_ZSTD => "zstd",
                _ => "?",
            };
            report.push_str(&format!(
                "\ncompression {codec}\nstored bytes {stored_len}\nplain bytes  {plain_len}\n"
            ));
            if extract_program.is_some() {
                return Err(
                    "--extract-program is an application's flag; a card image has one opaque \
                     member and no program member to name. ADR-029 §11: one container, and the \
                     metadata means different things per kind."
                        .to_string(),
                );
            }
            report.push_str(
                "\nTHIS IS A CARD IMAGE (R-177). It carries no manifest and installs on no\n\
                 board — provisioning is the only consumer, and it materialises this member\n\
                 back to a plain `.img` with zstd decompression before writing a device.\n\
                 `--trusted-key` above is one key on a command line; a board never verifies\n\
                 this kind at all, because a card is never sent to one.\n",
            );
        }
        // `ADR-061` §3. An APK, held by a fleet server for a phone.
        inspect::Details::Apk { metadata, apk, icon } => {
            let (index, name, length) = apk;
            report.push('\n');
            report.push_str(&apk_metadata_lines(metadata));
            report.push_str(&format!("\n  {index}  {name:<24} {length:>12} bytes\n"));
            match icon {
                Some((index, name, length)) => {
                    report.push_str(&format!("  {index}  {name:<24} {length:>12} bytes (icon)\n"));
                }
                None => report.push_str("  (no icon.png; a phone shows a generic launcher icon)\n"),
            }
            if extract_program.is_some() {
                return Err(
                    "--extract-program is an application's flag; an apk has no program member — \
                     app.apk is the whole of what it carries. ADR-029 §11: one container, and the \
                     metadata means different things per kind."
                        .to_string(),
                );
            }
            report.push_str(
                "\nTHIS IS AN APK (ADR-061 §3). It has no meaning on a Rivet machine — a fleet\n\
                 server's repository holds it so `gantry` can hand it to a phone, and an\n\
                 installer refuses it by name.\n",
            );
        }
    }

    Ok(report)
}

// The seed length this tool reads and the one the signer takes are the same
// number, checked rather than believed.
const _: () = assert!(SEED_BYTES == 32);

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use rivet_crypto::public_key_for;

    /// **Every key in `inspect`'s apk details is followed by a space**, the
    /// longest (`application_id`) included, and the values line up.
    #[test]
    fn inspect_separates_every_apk_key_from_its_value() {
        let json = format!(
            "{{\"application_id\":\"io.example.gantry\",\"version_code\":3,\
             \"version_name\":\"0.1.3\",\"label\":\"Gantry\",\"min_sdk\":31,\
             \"target_sdk\":36,\"signer_sha256\":\"{}\",\"development\":false}}",
            "ab".repeat(32)
        );
        let metadata = rivet_core::package::apk::Metadata::parse(json.as_bytes()).expect("parses");
        let text = apk_metadata_lines(&metadata);
        assert!(text.starts_with("application_id  io.example.gantry\n"), "{text}");
        let columns: Vec<usize> = text
            .lines()
            .map(|line| {
                let key_end = line.find(' ').expect("a key then a space");
                key_end + line[key_end..].find(|c: char| c != ' ').expect("a value")
            })
            .collect();
        assert_eq!(columns.len(), 6);
        assert!(columns.iter().all(|c| *c == columns[0]), "{text}");
    }

    /// The limit is the supervisor's, so the boundary is the thing worth
    /// pinning: exactly at it must package, because a program that fits is not
    /// this tool's business to have an opinion about.
    #[test]
    fn a_program_exactly_at_the_limit_is_not_refused() {
        use rivet_core::package::application::MAX_PROGRAM_BYTES;
        let path = Path::new("whatever");
        assert!(size_check(path, MAX_PROGRAM_BYTES).is_ok());
        assert!(size_check(path, MAX_PROGRAM_BYTES + 1).is_err());
    }

    /// **A program carrying DWARF gets a line about it, and one that does not
    /// gets silence.**
    ///
    /// `PLAN.md` §123, Wave 4.4. `size_check` refuses only past 16 MiB, so
    /// everything under it — `out/rivet-launcher-0.1.61` at 10.6 MB, ten times
    /// its stripped size — packaged with nothing said, and `ADR-015` §4.2 hashes
    /// the whole thing at every launch.
    #[test]
    fn a_program_carrying_debug_symbols_is_mentioned_and_a_stripped_one_is_not() {
        let with = elf_with_sections(&[".text", ".debug_info", ".debug_abbrev"]);
        let said = debug_note(&with);
        assert!(said.contains("DWARF"), "{said}");
        assert!(
            said.contains(".debug_info .debug_abbrev"),
            "it names the sections, so the advice can be checked rather than believed: {said}"
        );
        assert!(said.contains("RIVET_STRIP"), "the advice must name the tool: {said}");
        assert!(said.contains("ADR-005 §7.5"), "it must not advise throwing the symbols away");

        let without = elf_with_sections(&[".text", ".rodata"]);
        assert_eq!(debug_note(&without), "", "a stripped program must produce no line");
    }

    /// **The false positive that made this check worthless** — found 2026-09-04
    /// while packaging `examples/probes/slow-stop-probe` for the board.
    ///
    /// `debug_note` used to search the file's bytes for the literal
    /// `.debug_info`, and defended it on the grounds that a search *"can be
    /// wrong in one direction only"*. It can, and the direction it is wrong in
    /// is **every stripped Rust binary**: `std` links a DWARF reader for its
    /// own backtraces and that reader carries the section names as data, so
    /// `.debug_info` sits in `.rodata` of a program with nineteen sections and
    /// no debug information at all.
    ///
    /// A warning an author cannot act on teaches them to skip the next one.
    ///
    /// Break it by putting the byte search back and this fails on the first
    /// line — which is what says the test is wired to the defect and not to the
    /// fix.
    #[test]
    fn a_stripped_program_whose_data_holds_the_words_debug_info_is_not_accused() {
        let mut bytes = elf_with_sections(&[".text", ".rodata"]);
        bytes.extend_from_slice(b"....debug_info....debug_abbrev....debug_line....");
        assert_eq!(
            debug_note(&bytes),
            "",
            "a program with no .debug_* SECTION must produce no line, whatever its data says"
        );
    }

    /// A section table this tool cannot read is **UNKNOWN**, not clean.
    ///
    /// Silence is the answer for a stripped program, so silence for a file that
    /// could not be parsed would make the two indistinguishable — which is
    /// `AGENTS.md` §7's shape, in a check written to avoid exactly that.
    #[test]
    fn an_unreadable_section_table_says_so_rather_than_answering_clean() {
        let mut bytes = elf_with_sections(&[".text", ".debug_info"]);
        // `e_shoff` past the end of the file.
        bytes[40..48].copy_from_slice(&u64::from(u32::MAX).to_le_bytes());
        let said = debug_note(&bytes);
        assert!(said.contains("UNKNOWN"), "{said}");
        assert!(said.contains("section headers could not be read"), "{said}");
    }

    /// A 64-bit little-endian aarch64 ELF with a **real section header table**.
    ///
    /// **Provenance: hand-composed**, from the field offsets in `crate::elf`.
    /// The same builder exists in `tests/roundtrip.rs`, which cannot see this
    /// one — an integration test links the binary rather than the module.
    fn elf_with_sections(named: &[&str]) -> Vec<u8> {
        const EM_AARCH64: u16 = 183;
        const SHDR: usize = 64;

        let mut shstrtab = vec![0u8];
        let mut offsets = Vec::new();
        for name in named.iter().copied().chain(std::iter::once(".shstrtab")) {
            offsets.push(u32::try_from(shstrtab.len()).unwrap());
            shstrtab.extend_from_slice(name.as_bytes());
            shstrtab.push(0);
        }

        let strtab_off = 64usize;
        let shoff = strtab_off + shstrtab.len();
        let count = named.len() + 2;
        let mut bytes = vec![0u8; shoff + count * SHDR];

        bytes[0..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 2;
        bytes[5] = 1;
        bytes[18..20].copy_from_slice(&EM_AARCH64.to_le_bytes());
        bytes[40..48].copy_from_slice(&(shoff as u64).to_le_bytes());
        bytes[58..60].copy_from_slice(&u16::try_from(SHDR).unwrap().to_le_bytes());
        bytes[60..62].copy_from_slice(&u16::try_from(count).unwrap().to_le_bytes());
        bytes[62..64].copy_from_slice(&u16::try_from(count - 1).unwrap().to_le_bytes());
        bytes[strtab_off..shoff].copy_from_slice(&shstrtab);

        for (index, offset) in offsets.iter().enumerate() {
            let at = shoff + (index + 1) * SHDR;
            bytes[at..at + 4].copy_from_slice(&offset.to_le_bytes());
            let size = if index < named.len() { 4096 } else { shstrtab.len() as u64 };
            bytes[at + 32..at + 40].copy_from_slice(&size.to_le_bytes());
            if index == named.len() {
                bytes[at + 24..at + 32].copy_from_slice(&(strtab_off as u64).to_le_bytes());
            }
        }
        bytes
    }

    /// A scratch directory named after this test, under this process's scratch
    /// root, which the next run sweeps. `AGENTS.md` §8.3: nothing a suite needs
    /// is committed, and no absolute path is written down.
    fn scratch(name: &str) -> PathBuf {
        crate::scratch::dir(&format!("assets-{name}"))
    }

    /// **Every file in `assets/` is a member, named by its own filename, in a
    /// fixed order** — register `R-26`.
    ///
    /// `--asset NAME=PATH` was the only way in, so `aura-renderer` was deployed
    /// without its four typefaces and exited on a missing face with the display
    /// working perfectly (`PLAN.md` §119.3). Nothing refused, because nothing
    /// had been asked for.
    ///
    /// The order is asserted because a packaging is supposed to be a function of
    /// the tree: `read_dir` returns entries in whatever order the filesystem
    /// holds them, and `ADR-027` §3.2.2 puts the artifact's digest in its name.
    #[test]
    fn every_file_in_the_assets_directory_becomes_a_member_in_a_fixed_order() {
        let crate_dir = scratch("walk");
        let assets = crate_dir.join(ASSETS_DIR);
        std::fs::create_dir_all(&assets).expect("make assets/");
        for name in ["SourceCodePro-Bold.ttf", "AuraRain.ttf", "README.md"] {
            std::fs::write(assets.join(name), b"x").expect("write an asset");
        }

        let got = assets_in(&assets).expect("a readable directory");
        std::fs::remove_dir_all(&crate_dir).ok();

        let names: Vec<&str> = got.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            ["AuraRain.ttf", "README.md", "SourceCodePro-Bold.ttf"],
            "sorted, so two packagings of one tree produce one artifact"
        );
        // The README ships too. A filter would be a rule about which files
        // silently do not arrive, which is the defect this closes.
        assert_eq!(got.len(), 3);
    }

    /// An absent `assets/` is an ordinary state of a crate, not an error. Most
    /// applications have none.
    #[test]
    fn a_crate_with_no_assets_directory_packages_nothing_and_does_not_refuse() {
        let crate_dir = scratch("none");
        let got = assets_in(&crate_dir.join(ASSETS_DIR)).expect("absent is not a fault");
        std::fs::remove_dir_all(&crate_dir).ok();
        assert!(got.is_empty());
    }

    /// **A subdirectory REFUSES rather than being flattened.**
    ///
    /// A member name carries no path separator and an asset installs beside the
    /// program, so `assets/fonts/Geist.ttf` cannot be expressed. Packaging it as
    /// `Geist.ttf` would install a file at a path the program does not open —
    /// which is `PLAN.md` §119.3's failure again, wearing a different hat.
    #[test]
    fn a_subdirectory_under_assets_is_refused_because_members_are_flat() {
        let crate_dir = scratch("nested");
        let assets = crate_dir.join(ASSETS_DIR);
        std::fs::create_dir_all(assets.join("fonts")).expect("make assets/fonts");
        std::fs::write(assets.join("fonts").join("Geist.ttf"), b"x").expect("write");

        let why = assets_in(&assets).expect_err("a directory cannot be a member");
        std::fs::remove_dir_all(&crate_dir).ok();
        assert!(why.contains("fonts"), "it names the directory: {why}");
        assert!(why.contains("FLAT"), "it says why: {why}");
        assert!(why.contains("--no-assets"), "it says what to do: {why}");
    }

    /// The container's own name rules, applied where the author can act on them
    /// — a filename to rename, rather than a member of a half-composed archive.
    #[test]
    fn a_name_the_container_cannot_hold_is_refused_by_the_walker() {
        let crate_dir = scratch("names");
        let assets = crate_dir.join(ASSETS_DIR);
        std::fs::create_dir_all(&assets).expect("make assets/");

        // 33 bytes: one past `M_NAME_BYTES`.
        let long = "x".repeat(33);
        std::fs::write(assets.join(&long), b"x").expect("write");
        let why = assets_in(&assets).expect_err("33 bytes is one too many");
        assert!(why.contains("33 bytes"), "{why}");
        assert!(why.contains("at most 32"), "{why}");
        std::fs::remove_file(assets.join(&long)).expect("remove");

        // A space is not printable ASCII by this format's rule — these names go
        // into filenames and into `ADR-027` §3's grammar.
        std::fs::write(assets.join("two words.ttf"), b"x").expect("write");
        let why = assets_in(&assets).expect_err("a space is refused");
        std::fs::remove_dir_all(&crate_dir).ok();
        assert!(why.contains("two words.ttf"), "it names the file: {why}");
    }

    /// **A `--asset` that repeats a walked name refuses rather than overriding.**
    /// Both would install to one path and the later would silently win.
    #[test]
    fn an_asset_flag_colliding_with_a_walked_file_is_refused_naming_both() {
        let walked = vec![("Geist.ttf".to_string(), PathBuf::from("assets/Geist.ttf"))];
        let named = vec![("Geist.ttf".to_string(), PathBuf::from("elsewhere/Geist.ttf"))];
        let why = merge_assets(walked.clone(), named).expect_err("two members, one name");
        assert!(why.contains("elsewhere"), "{why}");
        assert!(why.contains("assets"), "{why}");

        // A name that does not collide is simply added, after the walked ones.
        let extra = vec![("other.bin".to_string(), PathBuf::from("out/other.bin"))];
        let got = merge_assets(walked, extra).expect("no collision");
        assert_eq!(
            got.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            ["Geist.ttf", "other.bin"]
        );
    }

    /// **The packager's own early refusal** for every name the installer
    /// writes into a version directory on its own — before this asset ever
    /// reaches a `Builder`, so the author sees a package-time refusal naming
    /// the file rather than a signed artifact that fails on a board.
    #[test]
    fn an_asset_reusing_a_name_the_installer_writes_for_itself_is_refused() {
        for reserved in rivet_core::package::install::RESERVED_APPLICATION_ASSET_NAMES {
            let assets = vec![(reserved.to_string(), PathBuf::from(format!("assets/{reserved}")))];
            let why = reject_reserved_asset_names(&assets)
                .expect_err(&format!("{reserved:?} was accepted as an asset name"));
            assert!(why.contains(reserved), "{why}");
        }
    }

    /// The refusal above must not have widened into refusing every asset.
    #[test]
    fn an_ordinary_asset_name_is_accepted() {
        let assets = vec![("banner.txt".to_string(), PathBuf::from("assets/banner.txt"))];
        assert!(reject_reserved_asset_names(&assets).is_ok());
    }

    /// **`--no-assets` takes no value, and the parser must not eat the option
    /// after it.** Every other option here is `--flag VALUE` and the loop used
    /// to advance by two unconditionally; a boolean added to that table would
    /// have silently swallowed whatever followed.
    #[test]
    fn the_no_assets_flag_takes_no_value_and_does_not_swallow_the_next_option() {
        let args: Vec<String> = ["--no-assets", "--bin", "thing", "--signing-key", "k"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let parsed = parse_package(&args).expect("parses");
        assert!(parsed.no_assets);
        assert_eq!(parsed.bin.as_deref(), Some("thing"), "--bin was still read");
        assert_eq!(parsed.signing_key, PathBuf::from("k"));
    }

    /// **The refusal has to be actionable or it is just the `EFBIG` earlier.**
    /// What made the original failure expensive was that it named a limit and
    /// not a remedy.
    #[test]
    fn the_refusal_says_what_to_do_about_it() {
        use rivet_core::package::application::MAX_PROGRAM_BYTES;
        let why = size_check(Path::new("big"), MAX_PROGRAM_BYTES * 2).expect_err("over the limit");
        assert!(why.contains("big"), "it names the file: {why}");
        assert!(why.contains("strip") || why.contains("Strip"), "it says strip: {why}");
        assert!(why.contains("os error 27"), "it names what would happen: {why}");
    }

    #[test]
    fn the_public_key_a_seed_makes_is_the_one_the_signer_reports() {
        let seed = [7u8; SEED_BYTES];
        let signer = Ed25519Signer::from_seed(&seed);
        assert_eq!(signer.public_key(), public_key_for(&seed));
    }

    /// **This test used to assert the opposite, and it only passed on a machine
    /// with no key.** It read
    /// `a_package_invocation_without_a_key_is_refused_rather_than_defaulted`,
    /// which was true until [`resolve_signing_key`] was added on 2026-08-30 to
    /// close the cold agent's blocker. After that it passed or failed according
    /// to whether the developer running it happened to have
    /// `~/.rivet/keys/rivet-app.seed` — so on the machine that had one it had
    /// been failing, unnoticed, and on CI it kept vouching for behaviour the
    /// project had deliberately overturned.
    ///
    /// What is actually promised is environment-independent, and it is the part
    /// the agent needed: **an absent flag is never a bare refusal.** Either a
    /// key is resolved, or the refusal says where it looked.
    #[test]
    fn an_absent_key_flag_either_resolves_or_says_where_it_looked() {
        match parse_package(&["--manifest-path".into(), "Cargo.toml".into()]) {
            Ok(args) => assert!(
                !args.signing_key.as_os_str().is_empty(),
                "resolving produced no path"
            ),
            Err(why) => assert!(
                why.contains(APP_SEED_ENV) || why.contains(".rivet"),
                "a refusal that names nowhere to look is the defect this closed: {why}"
            ),
        }
    }

    #[test]
    fn the_default_target_is_the_one_the_project_cross_compiles_for() {
        let args = parse_package(&["--signing-key".into(), "k".into()]).unwrap();
        assert_eq!(args.target, DEFAULT_TARGET);
        assert_eq!(name::arch_for(&args.target), Some("aarch64"));
    }

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|w| (*w).to_string()).collect()
    }

    // -----------------------------------------------------------------------
    // Which binary is this, and the linker a crate outside the repo needs
    // -----------------------------------------------------------------------

    /// A `PATH` resolver over a fixed set of files, so the real one is not
    /// consulted. `AGENTS.md` §4: a test that needs the host's environment is a
    /// test that passes for the wrong reason on somebody else's machine.
    fn on(files: &[&str]) -> impl Fn(&Path) -> bool + use<> {
        let files: Vec<String> = files.iter().map(|f| (*f).to_string()).collect();
        move |candidate: &Path| files.iter().any(|f| Path::new(f) == candidate)
    }

    /// The first directory wins, which is what `cargo` does.
    #[test]
    fn the_binary_on_path_is_the_one_in_the_earliest_directory_that_has_it() {
        let exists = on(&["/a/cargo-rivet", "/b/cargo-rivet"]);
        assert_eq!(
            first_on_path("/a:/b", ':', &exists),
            Some(PathBuf::from("/a/cargo-rivet"))
        );
        assert_eq!(
            first_on_path("/b:/a", ':', &exists),
            Some(PathBuf::from("/b/cargo-rivet"))
        );
    }

    /// **Both spellings are looked for on every host**, so a Unix build does not
    /// answer *nothing is shadowing you* about a Windows `PATH` for the wrong
    /// reason.
    ///
    /// The directories are written with forward slashes and the separator is
    /// passed in, because `Path::join` is the host's and this property is not:
    /// a test spelling a Windows path on a Linux runner would fail for
    /// `std::path`'s reasons rather than for this function's, which is
    /// `AGENTS.md` §7's *"an error and a true negative become
    /// indistinguishable"* wearing a test's clothes. It fired exactly that way
    /// in the container on 2026-08-30.
    #[test]
    fn the_windows_and_unix_spellings_are_both_looked_for() {
        let windows = PathBuf::from("bin").join("cargo-rivet.exe");
        let exists = on(&[&windows.to_string_lossy()]);
        assert_eq!(first_on_path("bin;other", ';', &exists), Some(windows));

        let unix = PathBuf::from("bin").join("cargo-rivet");
        let exists = on(&[&unix.to_string_lossy()]);
        assert_eq!(first_on_path("bin:other", ':', &exists), Some(unix));
    }

    /// An empty entry in `PATH` means the current directory to some shells and
    /// nothing here: it is skipped rather than joined, which would produce a
    /// bare `cargo-rivet` and report a relative path as though it were resolved.
    #[test]
    fn an_empty_path_entry_is_skipped_rather_than_treated_as_a_directory() {
        let exists = on(&["cargo-rivet", "/a/cargo-rivet"]);
        assert_eq!(
            first_on_path("::/a", ':', &exists),
            Some(PathBuf::from("/a/cargo-rivet"))
        );
        assert_eq!(first_on_path("", ':', &exists), None);
    }

    /// A `PATH` with no `cargo-rivet` at all answers `None` rather than
    /// something plausible — the same shape every other refusal here takes.
    #[test]
    fn a_path_with_no_cargo_rivet_answers_nothing() {
        assert_eq!(first_on_path("/x:/y", ':', &on(&["/x/cargo"])), None);
    }

    /// **`--version` says which file answered**, which is the whole of the
    /// 2026-08-30 fix that can be built: a stale binary already on somebody's
    /// `PATH` cannot be changed from here, so the new one has to carry a signal
    /// whose absence is diagnostic.
    #[test]
    fn version_names_the_build_the_file_and_how_to_tell_a_stale_one() {
        let said = dispatch(&words(&["--version"])).expect("--version answers");
        assert!(said.contains(&build_stamp()), "{said}");
        assert!(said.contains("running"), "{said}");
        assert!(said.contains("on PATH"), "{said}");
        assert!(said.contains("OVER A LOGIN SESSION"), "{said}");
        assert!(said.contains("cargo install --path cmd/cargo-rivet"), "{said}");
    }

    /// `cargo rivet update` is a payload verb, and the `--version` text says so
    /// — because *"update the tool"* is the first thing a reader told their tool
    /// is stale will reach for, and it is not what that verb does.
    #[test]
    fn version_says_that_update_is_not_a_self_update() {
        let said = dispatch(&words(&["--version"])).expect("--version answers");
        // **Unwrapped before the match**: the sentence is what matters, and
        // the line it breaks on moves whenever a word above it changes — as
        // it did when the install grew its second line (`.rulings/` R-192).
        let flat = said.replace('\n', " ");
        assert!(flat.contains("There is no self-update"), "{said}");
        assert!(said.contains("SYSTEM UPDATE payload"), "{said}");
    }

    /// **`--help` says that installing an application does not run it.** The
    /// concept lives fifty lines away in `install --help`, which is excellent and
    /// which an agent following a *"using it"* recipe never opens.
    #[test]
    fn the_usage_says_that_installing_an_application_does_not_run_it() {
        let usage = usage();
        assert!(usage.contains("DOES NOT RUN IT"), "{usage}");
        assert!(usage.contains("--enable"), "{usage}");
        assert!(usage.contains("ADR-015 §3.2.2"), "{usage}");
    }

    /// **The compiler hint fires on what `cc-rs` said and on nothing else.**
    /// The same discipline the linker hint has, for the failure two cold agents
    /// hit and neither diagnosed.
    #[test]
    fn the_compiler_hint_answers_a_missing_cross_compiler() {
        let said = "error occurred: Failed to find tool. Is `aarch64-linux-gcc` installed?";
        let hint = compiler_hint(said, DEFAULT_TARGET);
        assert!(hint.contains("MISSING CROSS C COMPILER"), "{hint}");
        // It quotes what cargo said rather than what it was expected to say.
        assert!(hint.contains("aarch64-linux-gcc"), "{hint}");
        // And it names the variable, spelled the way cc-rs reads it: the triple
        // with underscores, NOT upper-cased the way the linker variable is.
        assert!(
            hint.contains("CC_aarch64_unknown_linux_musl"),
            "the variable is misspelled, which is worse than no hint: {hint}"
        );
        assert!(hint.contains("AR_aarch64_unknown_linux_musl"), "{hint}");
    }

    /// **It must say the thing is POSSIBLE.** Both cold agents concluded the
    /// platform could not do it and moved to a pure-Rust crate; a hint that only
    /// named a variable would leave that conclusion standing.
    #[test]
    fn the_compiler_hint_says_the_platform_supports_this() {
        let hint = compiler_hint("Failed to find tool", DEFAULT_TARGET);
        assert!(hint.contains("NOT A DEAD END"), "{hint}");
        // The two facts that decide whether a crate can work at all.
        assert!(hint.contains("STATIC ONLY"), "{hint}");
        assert!(hint.contains("pkg-config"), "{hint}");
        assert!(
            hint.contains("bundled"),
            "the hint must name the path that works: {hint}"
        );
    }

    /// Silent on anything else — a compile error is never decorated with advice
    /// about a toolchain.
    #[test]
    fn the_compiler_hint_stays_silent_on_an_ordinary_compile_error() {
        for other in [
            "error[E0308]: mismatched types",
            "error: could not compile `whatever` due to 1 previous error",
            "error: linker `cc` not found",
        ] {
            assert_eq!(
                compiler_hint(other, DEFAULT_TARGET),
                "",
                "it decorated an unrelated failure: {other}"
            );
        }
    }

    /// **The two hints do not both fire on one failure.** A missing linker and a
    /// missing compiler are different problems, and printing both would tell an
    /// author their crate has two faults when it has one.
    #[test]
    fn a_missing_linker_produces_exactly_one_hint() {
        let said = "error: linker `cc` not found";
        assert!(!linker_hint(said, DEFAULT_TARGET).is_empty());
        assert_eq!(compiler_hint(said, DEFAULT_TARGET), "");
    }

    /// **The linker hint fires on what cargo said and on nothing else.** A
    /// compile error must never be decorated with advice about a linker.
    #[test]
    fn the_linker_hint_answers_a_missing_linker_and_stays_silent_otherwise() {
        let said = "error: linker `cc` not found\n  |\n  = note: No such file or directory";
        let hint = linker_hint(said, DEFAULT_TARGET);
        assert!(hint.contains("MISSING CROSS LINKER"), "{hint}");
        // It quotes what cargo said rather than what it was expected to say.
        assert!(hint.contains("linker `cc` not found"), "{hint}");
        // And it names the one-line fix, in the crate rather than in this tool.
        assert!(hint.contains("[target.aarch64-unknown-linux-musl]"), "{hint}");
        assert!(hint.contains("linker = \"rust-lld\""), "{hint}");
        assert!(
            hint.contains("CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER"),
            "{hint}"
        );

        for other in [
            "error[E0425]: cannot find value `x` in this scope",
            "error: could not compile `todo` (bin \"todo\") due to 1 previous error",
            "",
        ] {
            assert_eq!(
                linker_hint(other, DEFAULT_TARGET),
                "",
                "a build that failed for another reason must get no linker advice: {other}"
            );
        }
    }

    /// **A relative `--target-dir` keeps meaning what the operator meant**, even
    /// though the cargo child is now run in the crate's directory rather than in
    /// theirs. Absolute paths are left alone.
    #[test]
    fn a_relative_target_dir_is_resolved_before_the_child_is_moved() {
        let here = std::env::current_dir().expect("a working directory");
        assert_eq!(absolute(Path::new("out")).unwrap(), here.join("out"));
        let already = here.join("elsewhere");
        assert_eq!(absolute(&already).unwrap(), already);
    }

    /// The hint quotes the linker the host actually named, so a toolchain whose
    /// default is spelled differently is not told about `cc`.
    #[test]
    fn the_linker_hint_reports_the_linker_the_host_named() {
        let hint = linker_hint("error: linker `aarch64-linux-gnu-gcc` not found", DEFAULT_TARGET);
        assert!(hint.contains("aarch64-linux-gnu-gcc"), "{hint}");
    }

    /// And it says out loud that this tool did not set one, with the reason —
    /// because a reader whose build just failed will otherwise ask why a tool
    /// that knows the answer did not apply it.
    #[test]
    fn the_linker_hint_says_why_this_tool_does_not_set_it_for_you() {
        let hint = linker_hint("error: linker `cc` not found", DEFAULT_TARGET);
        assert!(hint.contains("deliberately does NOT set either for you"), "{hint}");
        assert!(hint.contains("ADR-011"), "{hint}");
    }

    /// **Every verb this tool has is in its own usage output.**
    ///
    /// `cargo rivet identity` worked and was not listed. The agent that needed
    /// it on 2026-08-29 found it only because a *different* program's `--help`
    /// mentioned it, and typed it speculatively.
    #[test]
    fn the_usage_lists_every_verb_this_build_has() {
        let usage = usage();
        for verb in VERBS {
            assert!(
                usage.contains(&format!("cargo rivet {}", verb.name)),
                "`{}` is not in the usage output:\n{usage}",
                verb.name
            );
        }
    }

    /// **Every manifest key this tool accepts is in its own usage output** —
    /// the same containment `the_usage_lists_every_verb_this_build_has`
    /// enforces, applied to the other list a person has to type.
    ///
    /// It was not enforced until 2026-09-08 and the block was **six keys
    /// short**: `memory`, `title`, `settings`, `proxy-path`, `proxy-auth` and
    /// `proxy-auth-paths` were all accepted by `crate::crate_meta` and
    /// documented nowhere a person looks. That is `identity`'s failure in a
    /// different costume — a thing that works and cannot be found — and a test
    /// naming those six would have fixed six lines and left the seventh to be
    /// discovered the same way, so this asserts the containment instead.
    #[test]
    fn the_usage_lists_every_manifest_key_this_build_accepts() {
        let usage = usage();
        for key in crate::crate_meta::DECLARED {
            assert!(
                usage.contains(&format!("\n\x20 {key} ")),
                "the manifest key `{key}` is not in the usage output:\n{usage}"
            );
        }
    }

    /// **The verbs this tool publishes, written out once more on purpose.**
    ///
    /// [`VERBS`] is now the only authority for what is listed *and* for what
    /// dispatches, which is the fix — and it means dropping an entry silently
    /// removes a verb from both at once. That is coherent and it is not
    /// harmless: `identity` and `run` are exactly the two an agent could not
    /// find on 2026-08-29, and losing one to a refactor would look like nothing
    /// happened.
    ///
    /// So this restates the list deliberately. It is not a second authority for
    /// the tool's behaviour — it cannot be, it asserts nothing about
    /// dispatch — it is the published surface pinned, so that removing a verb
    /// has to be a decision somebody wrote down.
    #[test]
    fn the_published_verbs_are_these_twenty_five() {
        let have: Vec<&str> = VERBS.iter().map(|v| v.name).collect();
        assert_eq!(
            have,
            [
                // **`setup` added 2026-09-05**, and first in the list because
                // it is first in the story: every other verb here presumes
                // credentials that nothing sequenced. It mints the two signing
                // seeds, the operator identity and the board's, in the order
                // they depend on each other, and never overwrites a key.
                "setup",
                // **`doctor`**, `ADR-062`. Second because it is what a person runs
                // before anything else works: it reads what this workstation has
                // for each goal and changes nothing.
                "doctor",
                "package",
                // **`library` added 2026-08-31**, Shared Rivets. Said here
                // because this test exists to make adding one a decision
                // somebody wrote down, and this is where it is written down.
                "library",
                // **`bump` added 2026-09-22** (Joel, option A). The deliberate,
                // committed pre-package patch bump — `tools/card/bump.sh`
                // generalized to a target manifest. Named here because losing it
                // would remove the one place an app's version is minted on a
                // clean tree before it is packaged, and because it must stay
                // PATCH-only and refuse an inherited version — a decision
                // somebody has to write down to change.
                "bump",
                "update",
                // **Renamed from `deploy` to `install` 2026-09-22** — see the
                // `VERBS` table. The scripted, flag-driven verb: `--host` and
                // the rest, no prompts.
                "install",
                // **`deploy` added 2026-09-22**, taking the bare shape
                // `fleet` used to have. No flags: the interactive deploy
                // interview, composing `install`, `secrets` and `run` for the
                // operator. Named here because losing it would leave that
                // interview with no verb to be reached through at all.
                "deploy",
                // **`fleet` added 2026-09-09**, `ADR-052` §9 step 1. The fleet
                // repository — a git-tracked manifest and a digest-keyed
                // object store on the workstation, wholly separate from
                // `rivet-fleet` the binary and from `[repos.<name>]`. Named
                // here because losing it would remove the one place `install
                // --artifact` reads from.
                "fleet",
                // **`artifacts` added 2026-09-27, `ADR-052` §7, R-225.** The
                // menu that deletes or hides a product in the fleet
                // repository. Named here because losing it would leave the
                // operator only the scripted `fleet retire`/`hide`.
                "artifacts",
                // **`image` added 2026-09-13, `R-177`.** Wraps an
                // already-built card image as a signed `card` and publishes it
                // into the fleet repository — the verb R-176 named "the
                // missing piece". Named here because losing it would remove
                // the only way a card image ever reaches the repository.
                "image",
                // **`write-image` added 2026-09-05 (as `provision`; renamed
                // 2026-09-13).** The card writer: it puts a document into a
                // built image's boot partitions and, with `confirm`, writes
                // the result to a device. Named here because losing it would
                // remove the only tool that closes the gap between a built
                // image and a running board.
                "write-image",
                // **`provision` added 2026-09-05, and rebuilt into the
                // one-command orchestrator 2026-09-13**
                // (`docs/PLAN-Feature-Set-2026-09-12.md` §5.2 item B,
                // `.rulings/` R-162, Wave 3b): the generated interview, the
                // composed document, the image, then `write-image`. Named
                // here because losing it would put SPEC-05's twelve steps
                // back behind four separate commands to run in order by hand.
                "provision",
                "run",
                // **`screenshot` added 2026-09-01**, `PLAN.md` §104.39. The
                // workstation half of the QOI/PNG split: the board writes the
                // capture and this end is what turns it into a picture.
                "screenshot",
                // **`secrets` added 2026-09-05**, `ADR-046` step 3. Three
                // `apply install-secret` calls with the authority's files read
                // here rather than pasted as hex. Named because losing it
                // would put a private key back on a command line.
                "secrets",
                // **`backup` added 2026-09-08**, `ADR-051`. The consumer of
                // the backup contract, and named here because a contract with
                // no consumer is worse than no contract — it looks built.
                // `ADR-045` shipped a health vocabulary the supervisor read
                // nowhere, which is the failure this entry exists to make
                // somebody write down before it happens again.
                "backup",
                // **`restore` added 2026-09-14**, `ADR-051` §12. Named because
                // losing it would leave an application's backup with no way
                // back that checks the file before the application sees it.
                "restore",
                // **`backup-secrets` and `restore-secrets` added 2026-09-14**,
                // R-183 D5. Named because losing either would leave a board's
                // own secrets with no copy off the board, or a copy with no way
                // back that refuses the wrong board.
                "backup-secrets",
                "restore-secrets",
                // **`recover`, `ADR-063` §7, R-235.** A host rebuilt from its
                // recorded document, its artifacts by digest, its sealed
                // secrets and its applications' backups. Named here because
                // losing it would leave a dead machine to be rebuilt by hand
                // from five verbs in an order nothing writes down.
                "recover",
                "inspect",
                // **`armada` added 2026-09-22, `ADR-059`.** The interactive CRUD
                // over the Armada Registry — the PC-side list of Fleets. Named
                // here because losing it would remove the only tool that adds a
                // Fleet to, or reads one from, the Armada the other verbs act on.
                "armada",
                "identity",
                "key"
            ],
            "a verb was added or removed; if that was meant, say so here"
        );
    }

    /// **…and the other direction: a verb not in the table is not a verb.**
    ///
    /// This is what makes the property above structural rather than a promise.
    /// A verb added to [`dispatch`] and forgotten in [`VERBS`] does not quietly
    /// work — it is refused, by the guard that runs before any arm.
    #[test]
    fn a_word_the_table_does_not_carry_never_reaches_a_dispatch_arm() {
        let refused = dispatch(&words(&["wibble"])).expect_err("not a verb");
        assert!(refused.contains("is not a `cargo rivet` verb"), "{refused}");
        // The refusal names them all, from the same table, so a mistyped verb
        // costs one command rather than a search.
        for verb in VERBS {
            assert!(refused.contains(verb.name), "{} missing: {refused}", verb.name);
        }
    }

    /// **Every verb in the table reaches an implementation.**
    ///
    /// Bare, so each fails on its own missing arguments — which is exactly the
    /// answer that proves it got past the gate. What must NOT come back is the
    /// unknown-verb refusal (the table has a word the dispatcher does not) or
    /// the defect sentence (the dispatcher has no arm for a word the table
    /// advertises).
    ///
    /// **Two verbs are reached through `--help` instead**, because bare they do
    /// not fail — they act on this host. Bare `setup` with no terminal mints a
    /// full credential set into the real `~/.rivet`, which is how a test run
    /// left one on a machine that had none; bare `doctor` probes Docker and
    /// rustup. `--help` is answered inside each one's own arm, so it proves the
    /// arm exists as well as a bare call would.
    #[test]
    fn every_verb_in_the_table_has_an_implementation_behind_it() {
        for verb in VERBS {
            let answer = if ["setup", "doctor"].contains(&verb.name) {
                dispatch(&words(&[verb.name, "--help"]))
            } else {
                dispatch(&words(&[verb.name]))
            };
            let text = match &answer {
                Ok(said) | Err(said) => said,
            };
            assert!(
                !text.contains("is not a `cargo rivet` verb"),
                "`{}` is advertised and is not dispatched: {text}",
                verb.name
            );
            assert!(
                !text.contains("that is a defect in cargo-rivet"),
                "`{}` is advertised and has no arm: {text}",
                verb.name
            );
        }
    }

    /// **`cargo rivet deploy <host>` is refused rather than ignored.** The
    /// interview picks its hosts itself; `deploy dev-01` used to drop `dev-01`
    /// and start asking, and nothing said the word had gone nowhere. The
    /// refusal names `install` — the same planner, as flags — for an agent.
    #[test]
    fn deploy_refuses_an_argument_it_would_ignore() {
        for word in ["dev-01", "--artifact", "confirm"] {
            let refused = dispatch(&words(&["deploy", word])).expect_err("an argument is refused");
            assert!(refused.contains("takes no arguments"), "{refused}");
            assert!(refused.contains("IGNORED"), "{refused}");
            assert!(refused.contains("cargo rivet install"), "{refused}");
            assert!(refused.contains("NOTHING WAS DONE"), "{refused}");
        }
        let refused = dispatch(&words(&["deploy", "dev-01"])).expect_err("refused");
        assert!(refused.contains("cargo rivet install dev-01 --artifact"), "{refused}");
        // And `--help` describes the verb instead of starting it.
        let help = dispatch(&words(&["deploy", "--help"])).expect("help");
        assert!(help.contains("takes NO ARGUMENTS"), "{help}");
        assert!(help.contains("`cargo rivet install`"), "{help}");
    }

    /// **`--help` and `--version` say which build this is.**
    ///
    /// Neither did, and it cost the agent of 2026-08-29 more than anything
    /// else: the `cargo-rivet` on that workstation's `PATH` was a stale build
    /// with five verbs and no control-plane client, `target/debug`'s had seven,
    /// and nothing in either output distinguished them. Trusting `PATH` — the
    /// normal thing to do — led to the conclusion that `ADR-035`'s control plane
    /// has no client at all.
    ///
    /// The commit is asserted as well as the version, because between image
    /// bumps every host build carries the same version (`ADR-027` §10.2).
    #[test]
    fn help_and_version_both_name_the_version_and_the_commit() {
        let stamp = build_stamp();
        assert!(stamp.contains(env!("CARGO_PKG_VERSION")), "{stamp}");
        assert!(
            stamp.contains(env!("RIVET_BUILD_DESCRIBE")),
            "the commit is missing from: {stamp}"
        );
        // Not the empty parenthesis a missing stamp would leave.
        assert!(!stamp.contains("()"), "{stamp}");

        for asked in [vec!["--help"], vec!["--version"], vec!["-V"], vec![]] {
            let said = dispatch(&words(&asked)).expect("help and version always answer");
            assert!(said.contains(&stamp), "{asked:?} does not say which build:\n{said}");
        }
    }
}
