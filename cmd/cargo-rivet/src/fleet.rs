//! `cargo rivet fleet` — **`ADR-052`'s fleet repository, step 1 of §9.**
//!
//! A tree on the workstation that holds what the maintainer's whole fleet is
//! made of: a git-tracked manifest naming every artifact, and a digest-keyed
//! object store beside it holding the bytes. `ADR-052` §2 rules the shape;
//! this module is the tool that builds and reads one, and the one new flag on
//! `install` that spends it.
//!
//! # Why this is `fleet` and never `repo`
//!
//! **`ADR-052` names the trap itself and asks this module to resolve it
//! explicitly, so the argument is written out rather than assumed.** A machine
//! document already declares `[repos.<name>]` — a **git repository a Rivet
//! machine hosts and serves**, `rivet-core/src/repo.rs`'s four-state
//! convergence and `cmd/rivet-sysd/src/repos.rs`'s reconciliation against disk.
//! `rivet-fleet` (`cmd/rivet-fleet`, a *different binary* from this one)
//! already has `add repo`, `set repo` and `remove repo` for exactly that thing.
//!
//! A second concept called `repo` in the same toolchain — this one being a
//! git tree **on the workstation** holding signed artifacts and desired state,
//! nothing a board serves — would be a name collision between two unrelated
//! ideas that both start `git init`. Grepping `repo` in this project would
//! answer two questions at once and a reader could not tell which. That is the
//! trap `ADR-052`'s naming problem names, and picking a synonym for the same
//! word (`vault`, `store` — the latter already means `rivet-store`, the crate
//! that persists `/state`) only moves the collision one word over.
//!
//! **So this module, its CLI verb and every string it prints spell out
//! "fleet repository" and never shorten it to "repo".** The verb is `fleet`
//! rather than `repo` or `repository` alone, because `ADR-052` itself never
//! calls this thing anything else — its title is *"The Fleet Repository"* —
//! and because `fleet` groups `init`, `publish` and `list` the same way
//! `identity` groups `new` and `key` groups a seed and `--out` (see `main.rs`'s
//! `VERBS` table). It is **not** `rivet-fleet` the binary, and it never edits
//! a `[repos.<name>]` declaration; every `--help` here says so, per the
//! maintainer's instruction that an ambiguous name be disambiguated
//! everywhere it appears rather than once. **The two meet at exactly one
//! point, by ruling** (`.rulings/` R-184 D3): `fleet push` sends this
//! repository's text INTO the repository a fleet server's `[repos.fleet]`
//! declares — `crate::fleet::push`'s own header.
//!
//! # What this module holds — `ADR-052` §7 and §9
//!
//! **The layout, the seed refusal, `fleet init`, `fleet publish`, `fleet
//! list`, `fleet drift`, and the fleet repository's half of `install`.**
//!
//! `fleet publish` takes an already-built `.rvt`. **`install` publishes before
//! it sends** (`publish_then_send`): handed a crate it packages, publishes the
//! result and sends the repository's copy; handed `--rvt` it publishes that
//! file first; handed `--artifact` it sends what the repository already holds.
//! The automatic publish verifies against the public half of the application
//! signing seed, found by the lookup `cargo rivet package` uses
//! (`deploy_app_seed`) — **except a system update, which verifies against the
//! SYSTEM signing seed instead** (`deploy_system_seed`, `R-177` D6). Every
//! kind publishes now; the update path's old "sent unpublished" exception
//! (`ADR-052` §3/§7, `.rulings/` R-157 R13) ended the day `rivet-system` was
//! given a key for it.
//!
//! # The two halves, on disk
//!
//! ```text
//! <root>/                  ~/.rivet/fleet by default, --at DIR always
//!   .git/                  the text half — ADR-052 §2's "kept in git"
//!   .gitignore             one line: /objects/
//!   manifest.toml          tracked; what the repository holds
//!   objects/
//!     <64 hex digest>       one file per artifact, named by its own BLAKE3
//! ```
//!
//! **`objects/` is never `git add`ed.** `ADR-052` §3's whole argument is that a
//! `.rvt` is already compressed and already signed, so it does not
//! delta-compress and a git history of them is unusable within months. The
//! manifest names an artifact by digest; the bytes live beside the text and
//! not inside it.
//!
//! # The refusal — `ADR-052` §4
//!
//! **Every seed this project mints is a `.seed` file** —
//! `keys/rivet-system.seed`, `keys/rivet-app.seed`, and a `device.seed` inside
//! every identity directory (`cmd/cargo-rivet/src/credentials.rs`). Nothing
//! else in this project uses that extension, so `find_seed` refuses on it
//! rather than on a hand-written list of filenames — the same posture
//! `AGENTS.md` §5 asks of a parser: refuse on what the tree actually contains,
//! not on what it was expected to contain.
//!
//! It runs **before `git init`** (so `--at ~/.rivet` itself is caught, not
//! just a mistake made after the fact) and **again immediately before every
//! commit** (`init`'s and `publish`'s). There is no flag that skips it: `ADR-052`
//! §4 is explicit that a `.gitignore` is a convention and this is a check, and a
//! check with an override is a convention wearing a check's clothes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use rivet_adapter_image::BootSlots as _;
use rivet_core::artifact::{BuildKey, KEEP, VersionOrder, newest_cmp, version_order};
use rivet_core::package::Kind;
use rivet_core::package::port::{Digest as _, PublicKey, Sign as _};
use rivet_crypto::{Blake3, Ed25519Verifier};
// `fleet drift`'s own connection flags — the same sentence `cargo rivet run`
// prints for `--identity`/`--pin`, imported rather than retyped so the two
// verbs cannot describe one pair of flags two ways.
use rivet_remote::endpoint::IDENTITY_AND_PIN;
use toml_edit::{DocumentMut, Item};

/// `cargo rivet fleet drift` — `ADR-052` §9 step 3. Its own file because the
/// comparison, the JSON reading and its tests are a real second concern
/// beside `init`/`publish`/`list`'s local file-and-git work; `fleet_tests.rs`
/// is the precedent for a flat sibling file over a nested module directory.
#[path = "fleet_drift.rs"]
mod drift;

/// `cargo rivet fleet push <host>` — `ADR-052` §9 step 4, `.rulings/` R-184.
/// Its own file for `drift`'s reason, and because the loopback relay it
/// carries is a concern no other sub-verb shares.
#[path = "fleet_push.rs"]
mod push;

/// `cargo rivet fleet sync <host>` — `ADR-052` §9 step 4, `.rulings/` R-184
/// D1. Its own file for `push`'s reason: a transport and a report no other
/// sub-verb shares.
#[path = "fleet_sync.rs"]
mod sync;

/// `cargo rivet fleet record <host>` — `ADR-052` §7, R-233: write into a
/// host's record which system update its running, committed slot holds, so a
/// publish never prunes it. Its own file for `push`'s reason.
#[path = "fleet_record.rs"]
mod record;

/// `cargo rivet fleet publish FILE.apk` — `ADR-061` §4: read the APK, hold
/// it to its product's newest version code and signer, wrap and sign it as an
/// `apk` artifact, and publish that as any artifact is published.
#[path = "fleet_publish_apk.rs"]
mod publish_apk;

/// `cargo rivet fleet phones list | rename | revoke` — `ADR-061` §6, the
/// agent's surface over the phones a fleet server's `gantry` socket also
/// serves. Its own file for `push`'s reason: a transport, a JSON reading and
/// the register's own encoding are a concern no other sub-verb shares.
#[path = "fleet_phones.rs"]
mod phones;

/// `cargo rivet fleet root <host>` — `ADR-063` §4: fetch the root a
/// fleet-root mode fleet server minted, over the pinned control plane, and
/// save it for a device's trust store.
#[path = "fleet_root.rs"]
mod root;

/// `cargo rivet fleet bookmark add | list | remove` — `ADR-063` §2: how this
/// workstation reaches each fleet, kept apart from this module's repository
/// and from the register.
#[path = "fleet_bookmark.rs"]
mod bookmark;

/// `cargo rivet fleet fetch <fleet>` and `fleet publish FILE --fleet <fleet>`
/// — `ADR-063` §6: the fleet server's repository is the authority, and a
/// workstation holds a clone that only fetches.
#[path = "fleet_remote.rs"]
mod remote;

/// `cargo rivet fleet retire | hide | unhide` — `ADR-052` §7, `.rulings/`
/// R-225: a product leaves circulation from this repository. Its own file
/// for `push`'s reason: the pin check over documents and the register, and
/// the fleet-server composition, are concerns no other sub-verb shares.
#[path = "fleet_retire.rs"]
mod retire;

/// `cargo rivet artifacts` — the menu interview over [`retire`]'s core
/// (`ADR-052` §7). Lives here beside the core it drives, as the deploy
/// interview does.
#[path = "fleet_artifacts.rs"]
pub(crate) mod artifacts;

/// `cargo rivet deploy` (bare) — the interactive deploy interview's needs
/// planner (`docs/design-candidates/Fleet-Deployment-Tool.md` §10.2). Pure and
/// board-free; its own file so the exhaustive unit tests sit beside it, as
/// `drift` and `push` do. **Lives here, in `fleet`, rather than moving to a
/// module of its own 2026-09-22** when the CLI verb it is reached through
/// changed from bare `fleet` to `deploy`: the interview still reads and plans
/// against this module's fleet repository and register, so the seam stays
/// where its dependencies are.
#[path = "fleet_deploy_plan.rs"]
pub(crate) mod deploy_plan;

/// `cargo rivet deploy` (bare) — the interview itself (§3, §10.3): the
/// host-state/manifest reads [`deploy_plan`] does not do, the
/// menu/approve/execute loop, and the mutation calls each approved step
/// runs. **Named `interview`, not `deploy`** — inside this module `deploy`
/// already means `rivet_remote::deploy`, imported all over the file below,
/// and (since 2026-09-22) `cargo rivet deploy` is ALSO this interview's own
/// CLI name, one more reason not to call the module that.
#[path = "fleet_deploy.rs"]
pub(crate) mod interview;

/// `cargo rivet install`'s NEXT STEPS — what [`deploy_plan`] says remains
/// once an install has happened, as the exact commands an agent runs. Its own
/// file for `drift`'s reason: the rendering and its tests are one concern, and
/// the whole point of it is that it adds no planning of its own.
#[path = "fleet_next_steps.rs"]
pub(crate) mod next_steps;

/// `cargo rivet deploy` with no arguments — the thin seam `main.rs`'s dispatch
/// calls, so every `fleet::` entry point main.rs reaches for
/// (`usage`/`run`/this) is spelled the same way rather than one of them
/// reaching into a submodule directly. **Moved off bare `cargo rivet fleet`
/// onto bare `cargo rivet deploy` 2026-09-22** — see `main.rs`'s `deploy` and
/// `fleet` dispatch arms; the function itself, and the interview it runs,
/// did not move.
///
/// # Errors
///
/// [`interview::run`]'s.
pub fn run_interview() -> Result<String, String> {
    interview::run()
}

/// `cargo rivet deploy --help`.
///
/// **It used to run the interview**, which on a machine with no terminal
/// answered with the interview's own refusal rather than with anything about
/// the verb. A help flag that starts the thing it describes is a flag nobody
/// can safely read.
#[must_use]
pub fn interview_usage() -> String {
    "usage: cargo rivet deploy\n\
     \n\
     THE INTERVIEW, for a person at a terminal. It takes NO ARGUMENTS: it reads\n\
     the fleet repository and the register and asks which artifacts to deploy\n\
     and to which hosts — THE HOST IS CHOSEN IN THE INTERVIEW, never on the\n\
     command line. For each host it shows the steps the planner offers —\n\
     install, per-machine material, enable, the converge, a service restart,\n\
     the rivet-proxyd restart a routed application needs — and asks before each.\n\
     \n\
     AN AGENT OR A SCRIPT USES `cargo rivet install` INSTEAD. The same planner\n\
     drives it: it installs, waits for that install to finish, and prints the\n\
     steps that remain as exact commands to run in order. `install --help`.\n\
     \n\
     `cargo rivet fleet plan <device>` prints what this interview would offer\n\
     for a host, and changes nothing.\n"
        .to_string()
}

/// **`cargo rivet deploy <anything>` is refused**, because the interview would
/// have ignored it: `deploy dev-01` used to drop `dev-01` on the floor and ask
/// which hosts, which an agent reads as the interview having taken the host
/// and a person reads as the tool not listening.
#[must_use]
pub fn interview_refuses(word: &str) -> String {
    let device = if word.starts_with('-') { "<device>" } else { word };
    format!(
        "`cargo rivet deploy` takes no arguments, and `{word}` would have been IGNORED — the \
         interview picks the host itself, from the register, once it starts. NOTHING WAS DONE.\n\
         \x20 a person              `cargo rivet deploy`, alone, at a terminal\n\
         \x20 an agent or a script  `cargo rivet install {device} --artifact <digest> [--enable] \
         confirm` — the same planner, as flags; it waits for its install and prints the steps \
         that remain (install --help)\n\
         \x20 to look first         `cargo rivet fleet plan {device}` prints what the interview \
         would offer there, and changes nothing"
    )
}

/// The manifest's file name, tracked by git. Not `manifest.toml`'s cousin
/// `Cargo.toml`'s `MANIFEST_NAME` (`rivet_core::package::application`, the
/// member *inside* a `.rvt`) — two different files, two different constants,
/// deliberately not shared.
pub(crate) const MANIFEST_FILE: &str = "manifest.toml";

/// The object store, beside the text — `ADR-052` §3.
pub(crate) const OBJECTS_DIR: &str = "objects";

// ---------------------------------------------------------------------------
// The refusal — ADR-052 §4
// ---------------------------------------------------------------------------

/// Walk `dir`, refusing nothing itself — it only **looks**, so `init` and
/// `publish` can each decide when to ask it and what to do with the answer.
///
/// **Never descends into `.git/`.** Git's own internals are not a fleet
/// repository's tree in the sense this check cares about, and a repository
/// that has been committed to many times has thousands of objects in there —
/// walking it on every publish would make the check the slow part of the tool
/// for no seed it could actually find (a seed refused at commit time never
/// reaches `.git/objects` in the first place).
///
/// # Errors
///
/// A directory that cannot be listed, naming it.
fn find_seed(dir: &Path) -> Result<Option<PathBuf>, String> {
    if !dir.is_dir() {
        return Ok(None);
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = std::fs::read_dir(&current)
            .map_err(|e| format!("cannot read {}: {e}", current.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("cannot read {}: {e}", current.display()))?;
            let path = entry.path();
            let is_dir = entry
                .file_type()
                .map_err(|e| format!("cannot stat {}: {e}", path.display()))?
                .is_dir();
            if is_dir {
                if path.file_name().is_some_and(|n| n == ".git") {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let is_seed = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("seed"));
            if is_seed {
                return Ok(Some(path));
            }
        }
    }
    Ok(None)
}

/// The refusal itself, naming the file `find_seed` found.
///
/// **There is no flag that overrides this.** `ADR-052` §4: a published signing
/// key cannot be unpublished, and every board that trusts its public half is
/// compromised at once. A tool that could be told to proceed anyway would be a
/// tool that gets told to, once, under pressure, and that is the one time it
/// matters.
fn seed_refusal(found: &Path) -> String {
    format!(
        "{} is a `.seed` file, and a fleet repository refuses to initialise or commit a tree \
         that contains one (ADR-052 §4).\n\
         \n\
         Every signing seed this project mints ends in `.seed` — `keys/rivet-system.seed`, \
         `keys/rivet-app.seed`, and `device.seed` inside every identity directory. If this \
         repository were ever pushed with one inside it, the key that signs this operating \
         system would be published, and a published key cannot be unpublished — every board \
         that trusts its public half is compromised at once.\n\
         \n\
         Move the seed out of this tree. There is no flag that overrides this refusal.",
        found.display()
    )
}

// ---------------------------------------------------------------------------
// The manifest
// ---------------------------------------------------------------------------

/// One artifact the repository holds — everything `ADR-052`'s "answers what
/// exactly is on `rivet-01`" needs an operator to recognise it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// 64 lowercase hex characters — the artifact's own BLAKE3, and the name
    /// its bytes are stored under in `objects/`.
    pub digest: String,
    pub product: String,
    pub version: String,
    pub arch: String,
    /// `ADR-027` §3's optional token — absent for an application, present for
    /// a system update.
    pub platform: Option<String>,
    /// `application`, `update`, `library` or `card` — `rivet_core::package::Kind`'s
    /// token, as read out of the artifact's own verified header.
    pub kind: String,
    pub size: u64,
    /// When this artifact was added, as this workstation's clock said at the
    /// time — `ADR-052` §5's attribution, the half a signature cannot give.
    pub added: String,
    /// **`R-177` D5, 2026-09-13.** The `rivet-config` schema version a `card`
    /// image's template `rivet.toml` declared when `cargo rivet image` wrapped
    /// it — `None` for every other kind, and for a card image whose boot
    /// partition carried no document at all.
    ///
    /// **Read from the image itself, not typed** — the whole point being that
    /// a typo cannot mint a version, and that this build's own compiled-in
    /// `SCHEMA_VERSION` is never asserted about bytes it did not compose
    /// (`ADR-027` §7's reasoning, applied to a fact this tool reads rather than
    /// one it stamps). Provisioning (item 2) is the reader: it compares this
    /// against its own build's schema version and refuses a mismatched image
    /// by name rather than discovering it as a parse failure after the
    /// interview.
    ///
    /// **Optional so an older manifest — every entry published before this
    /// field existed — still reads.** [`read_manifest`] treats an absent field
    /// as `None` rather than a parse error, and [`render`] omits the line
    /// entirely for an entry that has none, so the file `git diff`s cleanly
    /// against one written before this change.
    pub schema_version: Option<u32>,
}

impl Entry {
    /// The name `cargo rivet package`/`update` would have given this artifact
    /// — `rivet_remote::name::artifact`'s grammar, recomposed from the fields
    /// this repository verified rather than trusted from whatever the file
    /// was called on the way in (`AGENTS.md` §5: derive, never store).
    fn file_name(&self) -> Result<String, String> {
        let kind = match self.kind.as_str() {
            "application" => Kind::application(),
            "update" => Kind::update(),
            "library" => Kind::library(),
            "card" => Kind::card(),
            "apk" => Kind::apk(),
            other => {
                return Err(format!(
                    "the manifest names an artifact of kind {other:?}, and this build of \
                     cargo-rivet does not know how to name one for deploy. ADR-016's extension \
                     is unruled (ADR-027 §9)."
                ));
            }
        };
        Ok(rivet_remote::name::artifact(
            &self.product,
            &self.version,
            &self.arch,
            self.platform.as_deref(),
            &kind,
        ))
    }
}

/// `entry` as [`newest_cmp`] needs it (`.rulings/` R-213) — named once so
/// every "newest" picker in this module and in [`interview`] (`fleet_deploy.rs`,
/// which reaches this as `super::build_key`) compares the same three fields the
/// same way, rather than each spelling out the field mapping at its own site.
fn build_key(entry: &Entry) -> BuildKey<'_> {
    BuildKey {
        version: &entry.version,
        added: &entry.added,
        identity: &entry.digest,
    }
}

/// Everything the manifest says, or an empty list for a repository with
/// nothing published yet.
///
/// `[[artifact]]` is a TOML array of tables — `toml_edit`'s `parse` feature is
/// already a dependency here for `crate::crate_meta`'s reasons, and reading is
/// all this function needs; see [`render`] for why writing does not reach for
/// the same crate.
///
/// # Errors
///
/// The file exists and is not valid TOML, or an entry is missing a field or
/// has the wrong type — naming the file and the field.
pub(crate) fn read_manifest(root: &Path) -> Result<Vec<Entry>, String> {
    let Some((path, doc)) = manifest_document(root)? else {
        return Ok(Vec::new());
    };
    let Some(array) = doc.get("artifact").and_then(Item::as_array_of_tables) else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for table in array.iter() {
        let string = |key: &'static str| -> Result<String, String> {
            table
                .get(key)
                .and_then(Item::as_str)
                .map(ToString::to_string)
                .ok_or_else(|| {
                    format!("{} has an [[artifact]] with no string {key}", path.display())
                })
        };
        let size = table
            .get("size")
            .and_then(Item::as_integer)
            .and_then(|n| u64::try_from(n).ok())
            .ok_or_else(|| {
                format!("{} has an [[artifact]] with no non-negative integer size", path.display())
            })?;
        // **Absent is `None`, not a parse error** — the whole point of adding
        // this field to a manifest that already has entries without it
        // (`ADR-052`'s two-layer rule: old data must still read).
        let schema_version = table
            .get("schema_version")
            .and_then(Item::as_integer)
            .and_then(|n| u32::try_from(n).ok());
        // **Every field a path is built from is checked here, once**, so no
        // caller ever joins a manifest's text onto a directory unchecked: the
        // digest names `objects/<digest>`, and the four name fields name a
        // materialised file.
        let digest = string("digest")?;
        if !is_digest(&digest) {
            return Err(format!(
                "{} has an [[artifact]] whose digest {digest:?} is not 64 lowercase hexadecimal \
                 characters. A digest names a file under objects/ and is never a path, so this \
                 manifest is refused before anything is read or removed.",
                path.display()
            ));
        }
        let name = |key: &'static str, value: String| -> Result<String, String> {
            if is_name_token(&value) {
                Ok(value)
            } else {
                Err(format!(
                    "{} has an [[artifact]] ({digest}) whose {key} {value:?} is not a name: a name \
                     is letters, digits, `.`, `_`, `+` and `-`, not starting with `.`, because a \
                     file is named from it. Refused before anything is read or written.",
                    path.display()
                ))
            }
        };
        let platform = match table.get("platform").and_then(Item::as_str) {
            Some(value) => Some(name("platform", value.to_string())?),
            None => None,
        };
        out.push(Entry {
            product: name("product", string("product")?)?,
            version: name("version", string("version")?)?,
            arch: name("arch", string("arch")?)?,
            platform,
            digest,
            kind: string("kind")?,
            size,
            added: string("added")?,
            schema_version,
        });
    }
    Ok(out)
}

/// Whether `text` is a digest as this repository spells one: exactly 64
/// lowercase hexadecimal characters, so it can only ever name one file
/// directly under `objects/`.
pub(crate) fn is_digest(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Whether `text` is safe as part of a file name: non-empty, ASCII letters,
/// digits, `.`, `_`, `+` and `-`, and not starting with `.` — so it holds no
/// separator, no drive, and is never `.` or `..`.
fn is_name_token(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with('.')
        && text.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'))
}

/// `manifest.toml`, parsed, with its path — or `None` for a repository with
/// no manifest yet. The one reading of the file both [`read_manifest`] and
/// [`read_hidden`] take their halves from.
///
/// # Errors
///
/// The file exists and cannot be read, or is not valid TOML.
fn manifest_document(root: &Path) -> Result<Option<(PathBuf, DocumentMut)>, String> {
    let path = root.join(MANIFEST_FILE);
    if !path.is_file() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let doc: DocumentMut = text
        .parse()
        .map_err(|e| format!("{} is not valid TOML: {e}", path.display()))?;
    Ok(Some((path, doc)))
}

/// **One product withheld from every fleet** — a `[[hidden]]` entry in
/// `manifest.toml` (`ADR-052` §7). The product stays in this repository and
/// every card build still finds it; `fleet sync` does not send it, a fleet
/// server's prune removes it, and the deploy menus leave it out.
///
/// Keyed by kind as well as product, because one product name can be
/// published as more than one kind and hiding one says nothing about the
/// other.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Hidden {
    pub product: String,
    pub kind: String,
}

impl Hidden {
    /// Whether `entry` is one of this hidden product's.
    pub(crate) fn covers(&self, entry: &Entry) -> bool {
        self.product == entry.product && self.kind == entry.kind
    }
}

/// Whether `entry` belongs to a product `hidden` withholds.
pub(crate) fn is_hidden(hidden: &[Hidden], entry: &Entry) -> bool {
    hidden.iter().any(|h| h.covers(entry))
}

/// The manifest's hidden set, in [`render_manifest`]'s canonical order — an
/// empty list for a manifest with no `[[hidden]]` at all, which is every
/// manifest written before the set existed.
///
/// # Errors
///
/// [`manifest_document`]'s, `hidden` that is not an array of tables, or an
/// entry without a string `product` and `kind` — naming the file and the
/// entry, since a hidden set read around would send what the operator hid.
pub(crate) fn read_hidden(root: &Path) -> Result<Vec<Hidden>, String> {
    let Some((path, doc)) = manifest_document(root)? else {
        return Ok(Vec::new());
    };
    let Some(item) = doc.get("hidden") else {
        return Ok(Vec::new());
    };
    let array = item.as_array_of_tables().ok_or_else(|| {
        format!("{} has a `hidden` that is not an array of tables ([[hidden]])", path.display())
    })?;
    let mut out = Vec::new();
    for (at, table) in array.iter().enumerate() {
        let string = |key: &'static str| -> Result<String, String> {
            table.get(key).and_then(Item::as_str).map(ToString::to_string).ok_or_else(|| {
                format!("{} has a [[hidden]] (entry {}) with no string {key}", path.display(), at + 1)
            })
        };
        out.push(Hidden { product: string("product")?, kind: string("kind")? });
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// **Wave C3 (`.rulings/` R-178 item 1): resolve one `[carry.<product>]` pin
/// against `root`'s manifest, to the entry and its bytes' path in
/// `objects/`.**
///
/// The same algorithm `cmd/rivet-image/src/fleet_payloads.rs::resolve`
/// already performs for a *fresh build* — pinned succeeds only against that
/// exact version, unpinned succeeds only when exactly one `application`
/// candidate exists — restated here rather than shared, because that crate is
/// bin-only and has nothing to export (`ADR-011`, the same trade
/// `fleet_payloads.rs`'s own header already took against THIS module). What
/// differs is the caller: `write_image::run` widening an EXISTING image
/// carries a payload the same way a fresh card build does, using this crate's
/// own `Entry`/`read_manifest` rather than a second, bin-only copy of them.
///
/// # Errors
///
/// No `application` artifact of `product` at all, a pin naming a version the
/// repository does not hold, more than one candidate with no pin to choose
/// between them, or a manifest naming bytes `objects/` does not have — every
/// refusal names the product and what was actually published, on
/// `fleet_payloads::resolve`'s own wording.
pub(crate) fn resolve_carried_object(
    root: &Path,
    product: &str,
    pinned: Option<&str>,
) -> Result<(Entry, PathBuf), String> {
    let entries = read_manifest(root)?;
    let candidates: Vec<&Entry> =
        entries.iter().filter(|e| e.product == product && e.kind == "application").collect();

    let selected: Entry = if let Some(version) = pinned {
        let matches: Vec<&Entry> = candidates.iter().copied().filter(|e| e.version == version).collect();
        match matches.as_slice() {
            [] if candidates.is_empty() => {
                return Err(format!(
                    "the machine document pins {product:?} to version {version:?} and the fleet \
                     repository at {} has no `application` artifact of that name at all. `cargo \
                     rivet fleet publish` (or `cargo rivet fleet list` to see what it already \
                     holds) is how one gets there.",
                    root.display()
                ));
            }
            [] => {
                return Err(format!(
                    "the machine document pins {product:?} to version {version:?} and the fleet \
                     repository at {} holds no `application` artifact of that product at that \
                     version. Published: {}.",
                    root.display(),
                    candidates.iter().map(|e| e.version.as_str()).collect::<Vec<_>>().join(", "),
                ));
            }
            [one] => (*one).clone(),
            many => {
                return Err(format!(
                    "the machine document pins {product:?} to version {version:?} and the fleet \
                     repository at {} holds {} `application` artifacts of that product at that \
                     version, and nothing here is positioned to pick between them (AGENTS.md §5 \
                     — refuse rather than guess):\n{}",
                    root.display(),
                    many.len(),
                    many.iter().map(|e| format!("  {} {} {}\n", e.digest, e.product, e.version)).collect::<String>(),
                ));
            }
        }
    } else {
        match candidates.as_slice() {
            [] => {
                return Err(format!(
                    "the role requires {product:?} and the fleet repository at {} has no \
                     `application` artifact of that name. `cargo rivet fleet publish` (or \
                     `cargo rivet fleet list` to see what it already holds) is how one gets \
                     there.",
                    root.display()
                ));
            }
            [one] => (*one).clone(),
            many => {
                // `.rulings/` R-213: ordered by version then by which was
                // added to the repository more recently — never by
                // [`version_order`] alone, which ties every build sharing a
                // version's numeric core and would leave the suggestion
                // depending on the manifest's own listing order.
                let newest = many.iter().max_by(|a, b| newest_cmp(build_key(a), build_key(b)));
                let suggestion = newest.map_or_else(String::new, |e| {
                    format!(
                        "\nThe newest published is {} {}; pin it by writing `version = {:?}` \
                         under `[carry.{product}]` in the machine document, or pin a different \
                         one. A pin is never chosen silently.",
                        e.product, e.version, e.version
                    )
                });
                return Err(format!(
                    "the role requires {product:?} and the fleet repository at {} holds {} \
                     `application` artifacts of that name, and nothing here is positioned to \
                     pick between them (AGENTS.md §5 — refuse rather than guess):\n{}Publish \
                     only one, pin the version the machine document should carry, or carry it \
                     explicitly.{suggestion}",
                    root.display(),
                    many.len(),
                    many.iter().map(|e| format!("  {} {} {}\n", e.digest, e.product, e.version)).collect::<String>(),
                ));
            }
        }
    };

    let object = root.join(OBJECTS_DIR).join(&selected.digest);
    if !object.is_file() {
        return Err(format!(
            "the fleet repository's manifest names {} ({} {}) and its bytes are not at {} — the \
             repository is inconsistent. Nothing was carried.",
            selected.digest,
            selected.product,
            selected.version,
            object.display()
        ));
    }
    Ok((selected, object))
}

/// The whole manifest file, from the entries this repository holds.
///
/// **Hand-formatted, not `toml_edit`'s writer.** `cmd/cargo-rivet/Cargo.toml`
/// takes `toml_edit` with `default-features = false, features = ["parse"]`
/// and says why in a comment: *"nothing here writes TOML"*. Rather than widen
/// that dependency for a format this simple, this follows the house pattern
/// `crate::png` and `crate::qoi` already set — a handful of scalar fields per
/// entry, escaped by [`quoted`], is a smaller and more auditable surface than
/// a general-purpose TOML writer would be. The whole file is re-rendered on
/// every write rather than edited in place, so there is exactly one function
/// that has an opinion about what this file looks like.
///
/// This is [`render_manifest`] with no hidden set, for a manifest that carries
/// none (a card's staged copy, and the tests' fixtures).
pub(crate) fn render(entries: &[Entry]) -> String {
    render_manifest(entries, &[])
}

/// The whole manifest file: [`render`]'s `[[artifact]]` entries, then one
/// `[[hidden]]` per product withheld from every fleet (`ADR-052` §7), sorted
/// and deduplicated so the file is a function of the set and `git diff` shows
/// only what changed.
///
/// **An empty hidden set writes nothing at all**, so a manifest written before
/// the set existed renders byte for byte as it did. Every reader of this file
/// reads `[[artifact]]` by name and nothing else, so a build that predates the
/// set ignores `[[hidden]]` rather than refusing the file.
pub(crate) fn render_manifest(entries: &[Entry], hidden: &[Hidden]) -> String {
    let mut out = String::from(
        "# The fleet repository's manifest — ADR-052 §2. Each [[artifact]] names\n\
         # one artifact this repository holds, by the BLAKE3 digest its bytes are\n\
         # stored under in objects/ (ADR-052 §3). Written by `cargo rivet fleet\n\
         # publish`; hand-editing it will not move or rename anything in objects/.\n",
    );
    for entry in entries {
        out.push_str("\n[[artifact]]\n");
        out.push_str(&format!("digest  = {}\n", quoted(&entry.digest)));
        out.push_str(&format!("product = {}\n", quoted(&entry.product)));
        out.push_str(&format!("version = {}\n", quoted(&entry.version)));
        out.push_str(&format!("arch    = {}\n", quoted(&entry.arch)));
        if let Some(platform) = &entry.platform {
            out.push_str(&format!("platform = {}\n", quoted(platform)));
        }
        out.push_str(&format!("kind    = {}\n", quoted(&entry.kind)));
        out.push_str(&format!("size    = {}\n", entry.size));
        out.push_str(&format!("added   = {}\n", quoted(&entry.added)));
        if let Some(schema_version) = entry.schema_version {
            out.push_str(&format!("schema_version = {schema_version}\n"));
        }
    }
    let mut hidden: Vec<&Hidden> = hidden.iter().collect();
    hidden.sort();
    hidden.dedup();
    for one in hidden {
        out.push_str("\n[[hidden]]\n");
        out.push_str(&format!("product = {}\n", quoted(&one.product)));
        out.push_str(&format!("kind    = {}\n", quoted(&one.kind)));
    }
    out
}

/// **Write `entries` and `hidden` as the manifest and commit that file alone**
/// with `message` — the one commit every change to this repository's text
/// makes.
///
/// The seed refusal runs immediately before the commit (`ADR-052` §4), and the
/// commit is `--only` the manifest: `~/.rivet/fleet` is the maintainer's own
/// working tree, and something else can be staged there when this runs
/// (`ADR-061` §4).
///
/// **The write is atomic and the commit is all-or-nothing on disk**: the new
/// text goes to a temporary file in the same directory and is renamed over the
/// manifest, so a reader never sees half a file; and when the commit does not
/// land, the manifest's previous bytes are put back the same way and the index
/// is returned to `HEAD`'s copy, so the file on disk is still what `HEAD`
/// holds. A caller removes objects only after this returns `Ok`.
///
/// # Errors
///
/// The file cannot be written, [`seed_refusal`], or whatever `git` said.
fn commit_manifest(root: &Path, entries: &[Entry], hidden: &[Hidden], message: &str) -> Result<(), String> {
    let manifest_path = root.join(MANIFEST_FILE);
    let previous = match std::fs::read(&manifest_path) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("cannot read {}: {e}", manifest_path.display())),
    };
    write_atomically(&manifest_path, render_manifest(entries, hidden).as_bytes())?;
    let committed = (|| {
        if let Some(found) = find_seed(root)? {
            return Err(seed_refusal(&found));
        }
        git(root, &["add", MANIFEST_FILE])?;
        git(root, &["commit", "--quiet", "--only", "-m", message, "--", MANIFEST_FILE])?;
        Ok(())
    })();
    if let Err(why) = committed {
        let restored = match &previous {
            Some(bytes) => write_atomically(&manifest_path, bytes),
            None => std::fs::remove_file(&manifest_path)
                .map_err(|e| format!("cannot remove {}: {e}", manifest_path.display())),
        };
        // The index is put back too, by staging the restored bytes, so a later
        // commit of anything else does not carry the refused manifest with it.
        let _ = match &previous {
            Some(_) => git(root, &["add", "--", MANIFEST_FILE]),
            None => git(root, &["rm", "--cached", "--quiet", "--ignore-unmatch", "--", MANIFEST_FILE]),
        };
        return Err(match restored {
            Ok(()) => format!("{why}\nThe manifest was put back as it was; nothing was removed."),
            Err(also) => format!("{why}\nAND the manifest could not be put back: {also}"),
        });
    }
    Ok(())
}

/// Write `bytes` to `path` by way of a sibling temporary file and a rename,
/// so `path` holds either its old bytes or all of the new ones.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".new");
    let temp = PathBuf::from(temp);
    {
        use std::io::Write as _;
        let mut file =
            std::fs::File::create(&temp).map_err(|e| format!("cannot write {}: {e}", temp.display()))?;
        file.write_all(bytes).map_err(|e| format!("cannot write {}: {e}", temp.display()))?;
        file.sync_all().map_err(|e| format!("cannot write {}: {e}", temp.display()))?;
    }
    std::fs::rename(&temp, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("cannot replace {}: {e}", path.display())
    })
}

/// **Delete `removed`'s objects from `objects/`**, keeping any digest an entry
/// in `remaining` still names — the one removal of bytes from this repository,
/// shared by retention and by `fleet retire`.
///
/// Answers, per object not removed, why — an object already absent is not a
/// failure. The manifest is the source of truth for what this repository
/// holds, so a stray object nothing names is harmless and a caller decides
/// whether to say so.
fn remove_objects(root: &Path, remaining: &[Entry], removed: &[Entry]) -> Vec<String> {
    let mut failures = Vec::new();
    for victim in removed {
        if remaining.iter().any(|e| e.digest == victim.digest) {
            continue;
        }
        // **The sink checks for itself**: [`read_manifest`] refuses a digest
        // that is not one, and a removal never trusts that every caller read
        // its entries there.
        if !is_digest(&victim.digest) {
            failures.push(format!(
                "{:?} is not a digest, so nothing was removed for it: a removal only ever names \
                 one file directly under objects/",
                victim.digest
            ));
            continue;
        }
        let object = root.join(OBJECTS_DIR).join(&victim.digest);
        match std::fs::remove_file(&object) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => failures.push(format!("{} was not removed: {e}", object.display())),
        }
    }
    failures
}

/// A TOML basic string — quoted and escaped by hand, per [`render`]'s reason
/// for existing at all.
///
/// Every field this module writes comes off a verified `.rvt` header or this
/// module's own digest and clock, so none of them should ever need escaping in
/// practice — this exists so that "should" is not the thing standing between
/// an unusual product name and a manifest git cannot diff sanely.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Days since the Unix epoch to a Gregorian `(year, month, day)`.
///
/// Howard Hinnant's `civil_from_days` (`howardhinnant.github.io/date_algorithms.html`),
/// chosen for the reason `crate::png` hand-rolls a PNG encoder: a calendar is
/// thirty lines and this project carries no date/time dependency to gain one
/// timestamp nobody parses back (`AGENTS.md` §5, `ADR-052`'s own manifest is
/// "yours to shape").
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// The workstation's clock, as `ADR-052` §5's attribution timestamp —
/// `2026-09-09T12:34:56Z`. A parameter so publishing can be tested against
/// a fixed instant rather than racing the real clock.
fn iso8601(unix_seconds: u64) -> String {
    let days = (unix_seconds / 86_400) as i64;
    let of_day = unix_seconds % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        of_day / 3600,
        (of_day % 3600) / 60,
        of_day % 60,
    )
}

fn now_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        // A clock before 1970 is not this module's problem to solve; a
        // timestamp of the epoch is an honest answer to a broken clock rather
        // than a panic over a manifest entry.
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// git, shelled out to
// ---------------------------------------------------------------------------

/// `cmd/cargo-rivet/build.rs` and `rivet-remote/src/name.rs` already shell out
/// to a system `git` for a describe string; this is the same posture applied
/// to a repository this tool itself owns. No `git2`/`libgit2-sys` dependency —
/// `rivet-image/src/system.rs` records that crate as 93 crates deep for a
/// service this project does not need either.
fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("cannot run `git {}`: {e}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "`git {}` failed:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Whether `git config --get KEY` already answers something, in `root`.
fn git_configured(root: &Path, key: &str) -> bool {
    Command::new("git")
        .args(["config", "--get", key])
        .current_dir(root)
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.is_empty())
}

/// **Make a commit possible without touching this host's identity.**
///
/// Git commits refuse with no `user.name`/`user.email` anywhere in scope, and
/// a test running in a container with no `~/.gitconfig` would fail on that
/// rather than on anything this module does. When the ambient configuration
/// (this repository's own, this host's global, or the system's) already
/// answers both, **nothing is written** — a real operator's global identity,
/// already visible in `git log` on this very checkout, is what a fleet
/// repository's commits should carry, and `ADR-052` §5 makes that attribution
/// the whole reason documents live in git at all. Only the gap is filled, and
/// only in this repository's own **local** config — never `--global`, so
/// running this tool never edits a person's own git identity.
fn ensure_local_identity(root: &Path) -> Result<(), String> {
    for (key, value) in [
        ("user.name", "cargo-rivet-fleet"),
        ("user.email", "fleet@rivet.local"),
    ] {
        if !git_configured(root, key) {
            git(root, &["config", key, value])?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// init
// ---------------------------------------------------------------------------

/// `cargo rivet fleet init [--at DIR]` — create the repository.
///
/// **Running it twice is a report, not a disaster** — the same posture
/// `cargo rivet setup` already takes (`cmd/cargo-rivet/src/setup.rs`): an
/// existing `.git` here is left exactly as it is and this returns `have`
/// rather than re-initialising over it.
///
/// # Errors
///
/// [`seed_refusal`] if `root` (or anywhere under it) already holds a `.seed`
/// file — checked before anything is created, so `--at ~/.rivet` refuses
/// before `git init` ever runs there. Otherwise, whatever `git` said.
pub fn init(root: &Path) -> Result<String, String> {
    if let Some(found) = find_seed(root)? {
        return Err(seed_refusal(&found));
    }
    if root.join(".git").is_dir() {
        return Ok(format!(
            "have  fleet repository\n{}\n\nalready initialised — nothing was changed. `cargo \
             rivet fleet list --at {}` shows what it holds.\n",
            root.display(),
            root.display(),
        ));
    }

    std::fs::create_dir_all(root.join(OBJECTS_DIR))
        .map_err(|e| format!("cannot create {}: {e}", root.join(OBJECTS_DIR).display()))?;

    let gitignore = root.join(".gitignore");
    if !gitignore.is_file() {
        // A trailing slash names a directory rather than a file of the same
        // name, and the leading slash anchors it to this repository's root —
        // `ADR-052` §3's object store is never git's business.
        std::fs::write(&gitignore, "/objects/\n")
            .map_err(|e| format!("cannot write {}: {e}", gitignore.display()))?;
    }
    let manifest = root.join(MANIFEST_FILE);
    if !manifest.is_file() {
        std::fs::write(&manifest, render(&[]))
            .map_err(|e| format!("cannot write {}: {e}", manifest.display()))?;
    }

    // **`main`, named rather than inherited.** A plain `git init` takes this
    // host's `init.defaultBranch`, which may be anything; a fleet server's
    // repository is created on `main` (`cmd/rivet-sysd/src/repos.rs`) and
    // `fleet push` pushes `main`, so the two ends are made to agree here.
    git(root, &["init", "--quiet", "--initial-branch=main"])?;
    ensure_local_identity(root)?;

    // **Checked again, immediately before the commit** — `ADR-052` §4 refuses
    // to "initialise OR COMMIT" a tree with a seed in it, and `git init` above
    // just created a `.git` directory this walk skips, so re-running it here
    // costs nothing new and closes the window between the first check and the
    // first commit.
    if let Some(found) = find_seed(root)? {
        return Err(seed_refusal(&found));
    }
    git(root, &["add", ".gitignore", MANIFEST_FILE])?;
    git(
        root,
        &["commit", "--quiet", "-m", "Initialise the fleet repository (ADR-052)"],
    )?;

    Ok(format!(
        "MADE  fleet repository\n{}\n\
         \x20 .git         the text half, in git — ADR-052 §2\n\
         \x20 .gitignore   ignores objects/ — the bytes are never committed\n\
         \x20 {MANIFEST_FILE}   the manifest — empty; `fleet publish` appends to it\n\
         \x20 {OBJECTS_DIR}/      the digest-keyed object store — ADR-052 §3\n",
        root.display(),
    ))
}

/// The whole file's BLAKE3 and its length, without holding the file in
/// memory — a megabyte at a time, same as `crate::emit`'s own chunk size.
///
/// **A second copy of `rivet_remote::deploy`'s private `blake3_of`, and that
/// is deliberate rather than an oversight.** That one is `pub(crate)` to
/// `rivet-remote` for `rivet_remote::name::Version::distinguished_by`'s
/// reason — `ADR-011`, one digest function per crate boundary rather than a
/// public export that invites a second caller to depend on it changing in
/// step. This module is a different crate, so it is the same three lines
/// again rather than a dependency on `rivet-remote`'s internals.
fn blake3_of(path: &Path) -> Result<([u8; 32], u64), String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut digest = Blake3::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut total = 0u64;
    digest.start();
    loop {
        let read = std::io::Read::read(&mut file, &mut buffer)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        digest.write(&buffer[..read]);
        total += read as u64;
    }
    Ok((digest.finish(), total))
}

// ---------------------------------------------------------------------------
// publish
// ---------------------------------------------------------------------------

/// `cargo rivet fleet publish FILE.rvt --trusted-key PATH [--at DIR]`.
///
/// Verifies `file` **exactly as `cargo rivet inspect` does** — same verifier,
/// same trust posture: the one key given is trusted for every kind this build
/// reads, which `cmd/cargo-rivet/src/inspect.rs` documents as "the honest
/// shape of a developer checking their own artifact" rather than a fleet's
/// real trust (`ADR-028` §5 is that, and lives on a board). A fleet
/// repository's publish step is the same kind of self-check, one step earlier.
///
/// **Idempotent by digest** — `ADR-052`'s own words: *"Adding the same
/// artifact twice is idempotent — the digest is the name, so it either
/// matches or it is a different artifact."* An artifact already in the
/// manifest is reported and neither re-verified nor re-committed.
///
/// # Errors
///
/// [`seed_refusal`] (checked before anything is read or written), `file` does
/// not parse, is not signed by `trusted_key`, or fails any other check
/// `rivet_core::package::read::open` performs; or whatever `git` said.
pub fn publish(root: &Path, file: &Path, trusted_key: &Path) -> Result<String, String> {
    // **`None`.** A schema version is a fact about a card image's boot
    // partition, read while `cargo rivet image` still holds the `.img` open —
    // by the time a `.rvt` reaches `fleet publish` by hand, that partition is
    // sealed inside an opaque, signed member and nothing here may open it
    // without becoming a second reader of the container (`ADR-029` §11).
    publish_verified(root, file, None, &mut || {
        crate::key::read_32(trusted_key, "trusted public key").map(PublicKey)
    })
    .map(|published| published.report)
}

/// What a publish stored, or found already stored.
#[derive(Debug)]
pub struct Published {
    /// The artifact's whole-file BLAKE3, which is its name in `objects/`.
    pub digest: String,
    /// The `MADE` or `have` report, for a person.
    pub report: String,
    /// **The library references this artifact's own manifest declares** —
    /// `SPEC-02` §5 — read from the manifest that just verified. Empty for
    /// anything that is not an application, and for an application with no
    /// closure (`SPEC-02` §5: "empty is the ordinary answer" — most
    /// applications). `.rulings/` R-157 R15's check reads this before
    /// anything is sent.
    pub declared_libraries: Vec<String>,
}

// ---------------------------------------------------------------------------
// Retention — `ADR-052` §7's rule, ported to the workstation. `R-177` D4.
// ---------------------------------------------------------------------------

/// The group retention chooses between: not just a product, because this
/// repository holds several kinds and several platforms of one product where
/// a board's own store (`rivet_core::artifact`) never does.
type RetentionGroup<'a> = (&'a str, &'a str, Option<&'a str>, &'a str);

fn retention_group(entry: &Entry) -> RetentionGroup<'_> {
    (
        entry.product.as_str(),
        entry.kind.as_str(),
        entry.platform.as_deref(),
        entry.arch.as_str(),
    )
}

/// Drop every entry beyond [`KEEP`] per `(product, kind, platform, arch)`,
/// **in place**, and hand back what was dropped so the caller can remove the
/// matching objects and report them by name.
///
/// **The ordering and the tie-break are `rivet_core::artifact::evictions`'s
/// own** — componentwise numeric version, newest first, ties broken by
/// arrival and then by digest, with `just_published` always sorting ahead of
/// its group exactly as an artifact that "has just arrived" does in that
/// module (`ADR-052` §7.2.2: *"the artifact that has just arrived is never the
/// one dropped"*). This is a mechanical port rather than a call into that
/// function, because `Entry` already carries product, kind, platform and arch
/// as verified fields — there is no file name here to reparse, which is the
/// whole of what that function exists to do.
///
/// Readable and unreadable versions are kept apart, each to their own
/// [`KEEP`], for the reason `rivet_core::artifact`'s own module documentation
/// gives: an unreadable version ordered against a readable one either deletes
/// first on "not understood" or pins itself against every later deploy.
///
/// **A digest in `named` is never dropped** (`ADR-052` §7): a host's record
/// names it as what one of its slots runs. It is kept on top of the three
/// rather than counted among them, so a named build never costs a group a
/// newer one. The second list is what keep-three alone would have dropped and
/// `named` kept, for the report.
fn prune_keeping_three(
    entries: &mut Vec<Entry>,
    just_published: &str,
    named: &BTreeSet<String>,
) -> (Vec<Entry>, Vec<Entry>) {
    // Owned, so the borrow of `entries` this collects from does not outlive
    // the mutation below.
    let mut groups: Vec<(String, String, Option<String>, String)> = entries
        .iter()
        .map(retention_group)
        .map(|(p, k, f, a)| (p.to_string(), k.to_string(), f.map(ToString::to_string), a.to_string()))
        .collect();
    groups.sort();
    groups.dedup();

    let mut dropped_digests: Vec<String> = Vec::new();
    for (product, kind, platform, arch) in &groups {
        let key = |e: &Entry| {
            e.product == *product
                && e.kind == *kind
                && e.platform.as_deref() == platform.as_deref()
                && e.arch == *arch
        };
        let members: Vec<&Entry> = entries.iter().filter(|e| key(e)).collect();
        let (mut readable, mut unreadable): (Vec<&Entry>, Vec<&Entry>) = members
            .into_iter()
            .partition(|e| !matches!(version_order(&e.version), VersionOrder::Unreadable));
        for group in [&mut readable, &mut unreadable] {
            group.sort_by(|a, b| {
                let a_arriving = a.digest == just_published;
                let b_arriving = b.digest == just_published;
                b_arriving
                    .cmp(&a_arriving)
                    .then_with(|| version_order(&b.version).cmp(&version_order(&a.version)))
                    .then_with(|| b.added.cmp(&a.added))
                    .then_with(|| b.digest.cmp(&a.digest))
            });
            dropped_digests.extend(group.drain(..).skip(KEEP).map(|e| e.digest.clone()));
        }
    }

    let kept_named: Vec<Entry> = entries
        .iter()
        .filter(|e| dropped_digests.contains(&e.digest) && named.contains(&e.digest))
        .cloned()
        .collect();
    dropped_digests.retain(|digest| !named.contains(digest));
    let dropped: Vec<Entry> =
        entries.iter().filter(|e| dropped_digests.contains(&e.digest)).cloned().collect();
    entries.retain(|e| !dropped_digests.contains(&e.digest));
    (dropped, kept_named)
}

/// **Every digest a host's record in this repository names**, each with the
/// hosts and slots that name it — `ADR-052` §7, R-233. Read from the
/// repository's own `register/`, the projection `rivet-fleet` and `fleet
/// record` write, because the fleet repository is where a host's record is the
/// authority for what it runs.
///
/// A repository with no `register/` names nothing, and retention is then
/// keep-three exactly.
///
/// # Errors
///
/// A record that exists and does not read. The caller prunes nothing then,
/// because a prune that cannot see what a host runs could remove it.
fn named_by_hosts(root: &Path) -> Result<BTreeMap<String, Vec<String>>, String> {
    let register_root = root.join(rivetos_fleet_repo::REGISTER_DIR);
    let register = rivetos_register::store::load(&register_root)
        .map_err(|e| format!("the host records under {} do not read: {e}", register_root.display()))?;
    let mut named: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for device in register.devices.values() {
        for (slot, digest) in device.system_slots.digests() {
            named.entry(digest.to_owned()).or_default().push(format!("{} slot {slot}", device.name));
        }
    }
    Ok(named)
}

/// [`publish`], with the trusted key handed in by whoever knows where it comes
/// from — a `--trusted-key` file for `fleet publish`, the public half of the
/// application seed for `install`'s automatic publish, the public half of the
/// system seed for `cargo rivet image`.
///
/// `trusted` is asked only after the repository and the seed refusal have been
/// checked, so those two refusals come first whatever the key's source.
///
/// `schema_version` is `Some` only from `cargo rivet image`'s own call, which
/// alone still holds the card image open and can read its boot partition —
/// see [`Entry::schema_version`].
///
/// # Errors
///
/// [`publish`]'s, and whatever `trusted` said.
pub fn publish_verified(
    root: &Path,
    file: &Path,
    schema_version: Option<u32>,
    trusted: &mut dyn FnMut() -> Result<PublicKey, String>,
) -> Result<Published, String> {
    if !root.join(".git").is_dir() {
        return Err(format!(
            "{} is not a fleet repository yet. `cargo rivet fleet init --at {}` makes one \
             (ADR-052 §2). Nothing was read.",
            root.display(),
            root.display(),
        ));
    }
    if let Some(found) = find_seed(root)? {
        return Err(seed_refusal(&found));
    }

    let trusted = trusted()?;
    let mut digest = Blake3::new();
    let verifier = Ed25519Verifier::new();

    // `ADR-029` §4's reserved names, exactly as `cargo rivet inspect` reads
    // them — see `main.rs`'s `inspect_cmd`. An application or a library
    // ignores this list; an update payload naming a reserved file is refused
    // by it, here as it would be there.
    let boot_target = rivet_adapter_image::PiFirmware
        .payload(rivet_adapter_image::SlotId(1))
        .map_err(|e| format!("cannot describe slot A's targets: {e}"))?;
    let reserved = rivet_adapter_image::PiFirmware.reserved_names(&boot_target.kernel);

    let inspected = crate::inspect::inspect(file, trusted, reserved, &mut digest, &verifier)
        .map_err(|e| {
            format!(
                "{} is not a valid, signed artifact this key trusts: {e}\n\nNothing was stored.",
                file.display(),
            )
        })?;
    let header = inspected.package.header();

    // **An application under an image service's name is refused before it is
    // stored** (`ADR-060` §4) — the same rule the machine's install door
    // applies, so a payload no machine would take never reaches the repository
    // a card or a sync would take it from. A proxy fix ships as a system update.
    rivet_core::package::verify::not_an_image_service(&inspected.package).map_err(|why| {
        format!(
            "{} is refused: {why}. The image provides this service, so a payload of it is not \
             published; a fix to it ships in a system image (ADR-060). Nothing was stored.",
            file.display(),
        )
    })?;

    // **The declared closure — `SPEC-02` §5 — read from the manifest that just
    // verified.** Only an application carries one; enum-variant fields are
    // public with the enum in this crate (`main.rs`'s own `inspect_cmd`
    // pattern-matches the same way).
    let declared_libraries: Vec<String> = match &inspected.details {
        crate::inspect::Details::Application { manifest, .. } => {
            manifest.library().iter().map(ToString::to_string).collect()
        }
        _ => Vec::new(),
    };
    // Per-machine material (`ADR-055` §3) is not read here: `install` reads
    // every need a deploy acts on with the planner's own reader,
    // `interview::read_artifact_plan_with`, so there is one reading of it.

    let (whole_digest, size) = blake3_of(file)?;
    let hex_digest = crate::key::hex(&whole_digest);

    // **Everything this publish will write is read and checked first** — the
    // entries and the hidden set alike — so a manifest that does not read
    // stops the publish before a byte is stored or removed.
    let mut entries = read_manifest(root)?;
    // **The hidden set is carried through**: a publish adds an entry, and a
    // product the operator hid stays hidden, new version included.
    let hidden = read_hidden(root)?;
    if let Some(existing) = entries.iter().find(|e| e.digest == hex_digest) {
        let report = format!(
            "have  {hex_digest}\n\
             \x20 {} {}, added {}\n\
             \n\
             Already in the fleet repository — ADR-052's idempotent add. Nothing was changed.\n",
            existing.product, existing.version, existing.added,
        );
        return Ok(Published { digest: hex_digest, report, declared_libraries });
    }
    for (field, value) in [
        ("product", Some(header.product())),
        ("version", Some(header.version())),
        ("arch", Some(header.arch())),
        ("platform", header.platform()),
    ] {
        if let Some(value) = value
            && !is_name_token(value)
        {
            return Err(format!(
                "{} declares {field} {value:?}, which is not a name this repository can file a \
                 file under (letters, digits, `.`, `_`, `+`, `-`, not starting with `.`). Nothing \
                 was stored.",
                file.display()
            ));
        }
    }

    let object_path = root.join(OBJECTS_DIR).join(&hex_digest);
    std::fs::create_dir_all(root.join(OBJECTS_DIR))
        .map_err(|e| format!("cannot create {}: {e}", root.join(OBJECTS_DIR).display()))?;
    std::fs::copy(file, &object_path)
        .map_err(|e| format!("cannot store {}: {e}", object_path.display()))?;

    let entry = Entry {
        digest: hex_digest.clone(),
        product: header.product().to_string(),
        version: header.version().to_string(),
        arch: header.arch().to_string(),
        platform: header.platform().map(ToString::to_string),
        kind: header.kind().to_string(),
        size,
        added: iso8601(now_unix_seconds()),
        schema_version,
    };
    entries.push(entry.clone());

    // **`ADR-052` §7.2.2's rule, ported to the workstation — `R-177` D4.** The
    // board already keeps the three newest of each product; this repository
    // never kept any and would otherwise grow by every CI build, every board
    // type, for ever. Grouped one field wider than the board's own store,
    // because a workstation repository holds several kinds and several
    // platforms of one product where a board holds only what it can run.
    //
    // **What a host runs is kept on top** (`ADR-052` §7, R-233), and a record
    // set that does not read prunes nothing rather than pruning blind.
    let (pruned, kept_named, unread) = match named_by_hosts(root) {
        Ok(named) => {
            let digests: BTreeSet<String> = named.keys().cloned().collect();
            let (pruned, kept) = prune_keeping_three(&mut entries, &hex_digest, &digests);
            let kept: Vec<(Entry, Vec<String>)> = kept
                .into_iter()
                .map(|e| {
                    let hosts = named.get(&e.digest).cloned().unwrap_or_default();
                    (e, hosts)
                })
                .collect();
            (pruned, kept, None)
        }
        Err(why) => (Vec::new(), Vec::new(), Some(why)),
    };
    let mut message = format!("Add {} {} to the fleet repository ({hex_digest})", entry.product, entry.version);
    for victim in &pruned {
        message.push_str(&format!("\nPrune {} {} ({})", victim.product, victim.version, victim.digest));
    }
    // **Commit, THEN remove** — `fleet retire`'s order. A manifest that is
    // not committed still names every object it named, so nothing it names
    // may be gone yet.
    commit_manifest(root, &entries, &hidden, &message)?;
    // Best-effort: the manifest is the source of truth for what this
    // repository holds, and a stray object nothing names is harmless —
    // exactly the posture `ADR-052` §7.2.3.5 already takes for a store never
    // swept. Failing here would turn "kept one extra file" into "the publish
    // that pruned it also failed."
    let _ = remove_objects(root, &entries, &pruned);

    let mut report = format!(
        "MADE  {hex_digest}\n\
         \x20 product   {}\n\
         \x20 version   {}\n\
         \x20 kind      {}\n\
         \x20 arch      {}{}\n\
         \x20 size      {size} bytes\n\
         \x20 stored    {}\n",
        entry.product,
        entry.version,
        entry.kind,
        entry.arch,
        entry.platform.as_deref().map(|p| format!(" ({p})")).unwrap_or_default(),
        object_path.display(),
    );
    if !pruned.is_empty() {
        report.push_str(&format!(
            "pruned {} of the repository's own retention rule (ADR-052 §7.2.2, R-177 D4) — \
             keeps the {KEEP} newest of each product, kind, platform and arch:\n",
            pruned.len(),
        ));
        for victim in &pruned {
            let object = root.join(OBJECTS_DIR).join(&victim.digest);
            report.push_str(&format!(
                "  removed {} {} {} ({}{}) {}\n",
                victim.digest,
                victim.product,
                victim.version,
                victim.kind,
                victim.platform.as_deref().map(|p| format!(", {p}")).unwrap_or_default(),
                object.display(),
            ));
        }
    }
    if !kept_named.is_empty() {
        report.push_str(&format!(
            "kept {} that retention would have pruned, because a host's record names it as what \
             a slot runs (ADR-052 §7, R-233):\n",
            kept_named.len(),
        ));
        for (kept, hosts) in &kept_named {
            report.push_str(&format!(
                "  kept    {} {} {} ({}) — {}\n",
                kept.digest,
                kept.product,
                kept.version,
                kept.kind,
                hosts.join(", "),
            ));
        }
    }
    if let Some(why) = unread {
        report.push_str(&format!(
            "NOTHING WAS PRUNED: {why}. Retention keeps every digest a host's record names \
             (ADR-052 §7), and without reading them it cannot tell what a host runs, so this \
             publish kept everything. Repair the record and the next publish prunes as usual.\n"
        ));
    }
    Ok(Published { digest: hex_digest, report, declared_libraries })
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

/// `cargo rivet fleet list [--at DIR]` — what the repository holds, readable
/// by a person.
///
/// # Errors
///
/// The manifest exists and cannot be read as this module wrote it.
pub fn list(root: &Path) -> Result<String, String> {
    list_showing(root, false)
}

/// `cargo rivet fleet list [--hidden] [--at DIR]`.
///
/// **A hidden product is left out by default and counted** on one closing
/// line naming each (`ADR-052` §7): it is in this repository and withheld from
/// every fleet, which is the operator's way of saying he does not need to look
/// at it. `--hidden` lists every entry and marks each hidden one.
///
/// # Errors
///
/// The manifest exists and cannot be read as this module wrote it.
pub fn list_showing(root: &Path, show_hidden: bool) -> Result<String, String> {
    let entries = read_manifest(root)?;
    let hidden = read_hidden(root)?;
    if entries.is_empty() {
        return Ok(format!(
            "{}\n\n(empty — nothing has been published yet. `cargo rivet fleet publish FILE.rvt \
             --trusted-key PATH` adds one.)\n",
            root.display(),
        ));
    }
    let mut out = format!("{}\n\n", root.display());
    let mut folded: Vec<&Hidden> = Vec::new();
    for entry in &entries {
        let withheld = is_hidden(&hidden, entry);
        if withheld && !show_hidden {
            if let Some(one) = hidden.iter().find(|h| h.covers(entry))
                && !folded.contains(&one)
            {
                folded.push(one);
            }
            continue;
        }
        out.push_str(&format!(
            "{}{}  {} {} ({}, {}{}) {} bytes, added {}\n",
            if withheld { "[hidden] " } else { "" },
            // A short prefix is enough for a person to read and matches what
            // `deploy --artifact` will accept back — full digests belong in
            // the manifest file itself, not in a line meant to be read.
            &entry.digest[..entry.digest.len().min(12)],
            entry.product,
            entry.version,
            entry.kind,
            entry.arch,
            entry.platform.as_deref().map(|p| format!(" {p}")).unwrap_or_default(),
            entry.size,
            entry.added,
        ));
    }
    if !folded.is_empty() {
        out.push_str(&format!(
            "\n{} hidden, withheld from every fleet (ADR-052 §7): {}. `fleet list --hidden` shows them.\n",
            folded.len(),
            folded.iter().map(|h| format!("{} ({})", h.product, h.kind)).collect::<Vec<_>>().join(", "),
        ));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// `list_images` — R-177 item 9, the read side provisioning (item 2) needs.
// No provisioning behaviour is built here.
// ---------------------------------------------------------------------------

/// Every `card`-kind entry, optionally narrowed to one `platform`, **newest
/// first** — `R-176` Q6: *"the interview lists what is published, with the
/// newest preselected."*
///
/// **Reuses [`read_manifest`] and [`newest_cmp`]** rather than a second
/// reader: an entry's version is already a verified field here, not a file
/// name to reparse (`rivet_core::artifact::parse_name`'s job, for a machine's
/// own store, is not needed on a repository whose manifest already carries
/// every field structured).
///
/// **Ordered by version, then by which was added most recently** — never by
/// [`version_order`] alone. Several cards commonly share one version's
/// numeric core (a workspace version built more than once), and a plain
/// [`version_order`] sort ties every one of them; before `.rulings/` R-213 a
/// stable sort then kept whichever this manifest happened to list first,
/// which is the OLDEST of a tied group on every manifest this repository
/// writes, because publish only ever appends.
///
/// An empty result is a real, quiet answer — `read_manifest` already returns
/// an empty list for a repository with nothing published, and this is that
/// list filtered rather than a distinct "no images" state. Provisioning is
/// where `R-176` Q8's refusal — *"No images available for provisioning…"* —
/// belongs, worded exactly as ruled; this function's job is only to answer
/// accurately, not to compose that sentence.
///
/// # Errors
///
/// Whatever [`read_manifest`] refuses on — a manifest present and not valid
/// TOML, or an `[[artifact]]` missing a required field.
pub fn list_images(root: &Path, platform: Option<&str>) -> Result<Vec<Entry>, String> {
    let mut images: Vec<Entry> = read_manifest(root)?
        .into_iter()
        .filter(|e| e.kind == "card")
        .filter(|e| platform.is_none() || e.platform.as_deref() == platform)
        .collect();
    // `newest_cmp` reads `Greater` as "`a` is newer" (so `max_by` picks the
    // newest directly); a newest-FIRST listing is its reverse.
    images.sort_by(|a, b| newest_cmp(build_key(a), build_key(b)).reverse());
    Ok(images)
}

/// [`resolve`], narrowed to the `card` entries [`list_images`] would return —
/// so a digest that happens to prefix-match a non-card artifact is never
/// handed back as an image.
///
/// # Errors
///
/// [`resolve`]'s.
pub fn resolve_image<'a>(images: &'a [Entry], spec: &str) -> Result<&'a Entry, String> {
    resolve(images, spec)
}

/// Verify `entry` against `trusted_system_key` and decompress its one member
/// to a working `.img` copy under `out_dir`, named the way `cargo rivet
/// image` would name the source it wrapped:
/// `<product>-<version>-<arch>-<platform>-card.img`.
///
/// **The zstd decoder's one call site**, sibling to `cmd/cargo-rivet/src/image.rs`'s
/// encoder — both live in this one host binary; see `Cargo.toml`'s comment on
/// why that dependency stops here and never reaches `rivet-core`.
///
/// Verification is **against `trusted_system_key` alone**, entitled to `card`
/// only — not `crate::inspect::inspect`'s all-kinds self-check, and not a
/// hard-link the way [`materialise_for_deploy`] takes for a `.rvt` a board
/// will verify again itself. A card is never sent to a board, so this
/// workstation verification is the *only* one that ever runs against it —
/// `ADR-052` §7.2.3.2's own two reasons for a machine keeping its own check
/// apply here word for word: the store is not the only way bytes reach it,
/// and a verdict is true of the trust store it was reached against.
///
/// # Errors
///
/// The object is not on disk (the repository is inconsistent), the header
/// does not verify against `trusted_system_key` for `card`, a member's digest
/// does not match, or the decompressed length disagrees with the header's
/// recorded `plain_len`.
pub fn materialise_image(
    root: &Path,
    entry: &Entry,
    trusted_system_key: PublicKey,
    out_dir: &Path,
) -> Result<PathBuf, String> {
    if entry.kind != "card" {
        return Err(format!(
            "{} {} is a {:?}, not a card — nothing here decompresses any other kind",
            entry.product, entry.version, entry.kind
        ));
    }
    let object = root.join(OBJECTS_DIR).join(&entry.digest);
    if !object.is_file() {
        return Err(format!(
            "the fleet repository's manifest names {} ({} {}) and its bytes are not at {} — the \
             repository is inconsistent.",
            entry.digest,
            entry.product,
            entry.version,
            object.display(),
        ));
    }

    let mut digest = Blake3::new();
    let verifier = Ed25519Verifier::new();
    let mut trust = rivet_core::package::port::TrustSet::new();
    trust.trust(
        &mut digest,
        trusted_system_key,
        "rivet-system",
        rivet_core::package::port::MaySign::these(&[Kind::card()]),
    );

    let file_len = std::fs::metadata(&object)
        .map_err(|e| format!("cannot stat {}: {e}", object.display()))?
        .len();
    // Only the 4 KiB header — `ADR-029` §5's whole point is that nothing past
    // it is trusted before the signature has checked out, so nothing past it
    // needs to be resident here either.
    let mut head = vec![0u8; rivet_core::package::format::HEADER_BYTES];
    {
        use std::io::Read as _;
        let mut file = std::fs::File::open(&object)
            .map_err(|e| format!("cannot read {}: {e}", object.display()))?;
        file.read_exact(&mut head).map_err(|e| {
            format!(
                "{} is shorter than a package header ({e}); there is nothing to verify",
                object.display()
            )
        })?;
    }
    let package = rivet_core::package::read::open(&head, file_len, &mut digest, &verifier, &trust)
        .map_err(|e| format!("{} does not verify: {e}", object.display()))?;
    let member = package
        .members()
        .first()
        .ok_or_else(|| format!("{} is a card artifact with no member", object.display()))?;

    std::fs::create_dir_all(out_dir)
        .map_err(|e| format!("cannot create {}: {e}", out_dir.display()))?;
    let img_name = format!(
        "{}-{}-{}{}-card.img",
        entry.product,
        entry.version,
        entry.arch,
        entry.platform.as_deref().map(|p| format!("-{p}")).unwrap_or_default(),
    );
    let out_path = out_dir.join(img_name);

    // **Two passes over the stored (compressed) bytes, neither resident.**
    // Pass 1 verifies the member's digest — `ADR-029` §5.1: the digest is
    // over the stored bytes, checked before anything downstream sees them —
    // streamed and discarded, the same shape `cmd/cargo-rivet/src/inspect.rs`'s
    // `verify_member_discarding` uses for exactly this member kind. Pass 2 is
    // the only one that runs after that check has passed, and it hands the
    // SAME byte range straight to the decompressor through a bounded reader,
    // so the compressed bytes are never accumulated in memory either.
    {
        use std::io::{Read as _, Seek as _};
        let mut source = std::fs::File::open(&object)
            .map_err(|e| format!("cannot read {}: {e}", object.display()))?;
        source
            .seek(std::io::SeekFrom::Start(member.offset()))
            .map_err(|e| format!("cannot seek {}: {e}", object.display()))?;
        let mut stream = package
            .member_stream(0, &mut digest)
            .map_err(|e| format!("cannot stream the image member: {e}"))?;
        let mut left = member.stored_len();
        let mut buffer = vec![0u8; 1024 * 1024];
        while left > 0 {
            let want = usize::try_from(left.min(buffer.len() as u64)).unwrap_or(buffer.len());
            let read = source
                .read(&mut buffer[..want])
                .map_err(|e| format!("cannot read {}: {e}", object.display()))?;
            if read == 0 {
                break;
            }
            stream
                .write(&buffer[..read])
                .map_err(|e| format!("the image member does not match its recorded digest: {e}"))?;
            left -= read as u64;
        }
        stream
            .finish()
            .map_err(|e| format!("the image member does not match its recorded digest: {e}"))?;
    }

    {
        use std::io::{Read as _, Seek as _};
        let mut source = std::fs::File::open(&object)
            .map_err(|e| format!("cannot read {}: {e}", object.display()))?;
        source
            .seek(std::io::SeekFrom::Start(member.offset()))
            .map_err(|e| format!("cannot seek {}: {e}", object.display()))?;
        let bounded = source.take(member.stored_len());
        let out_file = std::fs::File::create(&out_path)
            .map_err(|e| format!("cannot write {}: {e}", out_path.display()))?;
        match member.compression() {
            rivet_core::package::read::COMPRESSION_NONE => {
                let mut bounded = bounded;
                let mut out_file = out_file;
                std::io::copy(&mut bounded, &mut out_file)
                    .map_err(|e| format!("cannot write {}: {e}", out_path.display()))?;
            }
            rivet_core::package::read::COMPRESSION_ZSTD => {
                zstd::stream::copy_decode(bounded, out_file)
                    .map_err(|e| format!("cannot decompress {}: {e}", object.display()))?;
            }
            other => {
                return Err(format!(
                    "{} names compression codec {other}, which this build does not implement",
                    object.display()
                ));
            }
        }
    }

    let written = std::fs::metadata(&out_path)
        .map_err(|e| format!("cannot stat {}: {e}", out_path.display()))?
        .len();
    if written != member.plain_len() {
        let _ = std::fs::remove_file(&out_path);
        return Err(format!(
            "{} decompressed to {written} bytes and the header recorded {} — refusing to leave \
             a card image this build cannot vouch for",
            object.display(),
            member.plain_len(),
        ));
    }

    Ok(out_path)
}

// ---------------------------------------------------------------------------
// digest / prefix resolution — shared by `list`'s reader and `deploy --artifact`
// ---------------------------------------------------------------------------

/// The one entry `spec` names — a full digest or an unambiguous prefix of
/// one, case-insensitively (`ADR-052` §7: *"selected by digest or by a short
/// unambiguous prefix of one"*).
///
/// # Errors
///
/// `spec` is empty, matches nothing, or matches more than one entry — in the
/// last case, naming every candidate so the operator can lengthen it by
/// however many characters actually distinguish them.
fn resolve<'a>(entries: &'a [Entry], spec: &str) -> Result<&'a Entry, String> {
    let spec = spec.trim().to_ascii_lowercase();
    if spec.is_empty() {
        return Err(
            "--artifact needs a digest or an unambiguous prefix of one from the fleet \
             repository's manifest (ADR-052 §7)."
                .to_string(),
        );
    }
    let matches: Vec<&Entry> = entries.iter().filter(|e| e.digest.starts_with(&spec)).collect();
    match matches.as_slice() {
        [] => Err(format!(
            "no artifact in the fleet repository starts with {spec:?}. `cargo rivet fleet list` \
             shows what it holds."
        )),
        [one] => Ok(one),
        many => Err(format!(
            "{spec:?} is ambiguous — it matches {} artifacts in the fleet repository:\n{}\nGive \
             more of the digest.",
            many.len(),
            many.iter()
                .map(|e| format!("  {} {} {}\n", e.digest, e.product, e.version))
                .collect::<String>(),
        )),
    }
}

/// Resolve `spec` against `root`'s manifest and hand back a local path
/// `install` can send exactly as it sends anything given with `--rvt`.
///
/// **A hard link where the platform allows one, a copy otherwise.** The
/// object store's bytes are never rewritten once stored, so a hard link is
/// safe — and it matters: `ADR-052` §11 measures a system update at roughly
/// 700 MB, and a *plan* (no `confirm`) that copied one just to print a
/// filename would make asking "what would this send" cost as much as sending
/// it. `std::fs::copy` is the fallback for a temp directory on a different
/// volume from the fleet repository, where a hard link cannot be made.
///
/// The destination is named by [`Entry::file_name`] — recomposed from the
/// artifact's verified header fields rather than whatever it was called when
/// it was published — because `--enable` and everything downstream in
/// `rivet_remote::deploy` reads the service name back out of the **file
/// name** (`ADR-027` §7), and `objects/<digest>` is not that name.
///
/// # Errors
///
/// [`resolve`]'s errors, an artifact of a kind this build cannot name
/// (`Entry::file_name`), the manifest naming bytes that are not on disk (the
/// repository is inconsistent), or an I/O failure making the local copy.
pub fn materialise_for_deploy(root: &Path, spec: &str) -> Result<PathBuf, String> {
    let entries = read_manifest(root)?;
    let entry = resolve(&entries, spec)?;
    let object = root.join(OBJECTS_DIR).join(&entry.digest);
    if !object.is_file() {
        return Err(format!(
            "the fleet repository's manifest names {} ({} {}) and its bytes are not at {} — the \
             repository is inconsistent. Nothing was sent.",
            entry.digest,
            entry.product,
            entry.version,
            object.display(),
        ));
    }
    let file_name = entry.file_name()?;
    let dir = std::env::temp_dir().join(format!("cargo-rivet-fleet-{}", entry.digest));
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let dest = dir.join(&file_name);
    if !dest.is_file() && std::fs::hard_link(&object, &dest).is_err() {
        std::fs::copy(&object, &dest)
            .map_err(|e| format!("cannot prepare {}: {e}", dest.display()))?;
    }
    Ok(dest)
}

// ---------------------------------------------------------------------------
// `install --artifact` — the hook `deploy_cmd` calls before `deploy::parse`
// ---------------------------------------------------------------------------

/// `~/.rivet/fleet`, or `%USERPROFILE%\.rivet\fleet` — the same rule
/// `credentials::root` gives `~/.rivet` itself, one path segment further.
///
/// # Errors
///
/// Neither `HOME` nor `USERPROFILE` is set.
fn default_root() -> Result<PathBuf, String> {
    crate::credentials::root().map(|home| home.join("fleet")).ok_or_else(|| {
        "this host sets neither HOME nor USERPROFILE, so there is nowhere to default the fleet \
         repository. Pass --at DIR."
            .to_string()
    })
}

/// What `install`'s command line says about the fleet repository, taken out of
/// argv before `deploy::parse` reads the rest.
#[derive(Debug)]
pub struct Route {
    /// argv for `deploy::parse`: `--at` removed, and `--artifact ID` rewritten
    /// into `--rvt PATH` naming a local copy of the repository's object.
    pub args: Vec<String>,
    /// `--at DIR`, or `None` for `~/.rivet/fleet`.
    pub at: Option<PathBuf>,
    /// The payload was selected from the repository with `--artifact`, so it
    /// is already published and nothing is published again.
    pub selected: bool,
}

/// **`--at DIR` and `--artifact ID`, read before `deploy::parse` sees argv.**
///
/// `--at` names the fleet repository every install publishes into (`ADR-052`
/// §7), so it is taken out of argv whether or not `--artifact` is given; a
/// crate install would otherwise hand it to `package`, which refuses it.
///
/// `--artifact` is resolved to a local path **here**, in `cmd/cargo-rivet`
/// alone, and handed to the existing `--rvt` mechanism unchanged, so
/// `rivet_remote::deploy::DeployArgs` — a library type — carries no field for
/// it. **An argv with neither flag comes back byte-for-byte**; `fleet_tests.rs`
/// pins that against a direct `deploy::parse`.
///
/// # Errors
///
/// `--artifact` given together with `--rvt` (refused: one selects a payload
/// already in the repository, the other names one directly — pick one), a flag
/// with no value, or [`materialise_for_deploy`]'s errors.
pub fn route_deploy(args: &[String]) -> Result<Route, String> {
    let mut artifact: Option<String> = None;
    let mut at: Option<PathBuf> = None;
    let mut rest: Vec<String> = Vec::new();
    let mut has_rvt = false;
    // **Where `--rvt PATH` goes back in**, not appended at the end. `install`'s
    // own rule is that `confirm` must be the LAST word (`ADR-012` §7), so an
    // operator who wrote `--artifact ID confirm` must get `--rvt PATH
    // confirm` back — appending unconditionally would move `confirm` off the
    // end and turn a valid command into a refused one.
    let mut insert_at: Option<usize> = None;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--artifact" => {
                artifact = Some(
                    args.get(index + 1)
                        .cloned()
                        .ok_or("--artifact needs a digest or a prefix of one")?,
                );
                insert_at = Some(rest.len());
                index += 2;
            }
            "--at" => {
                at = Some(PathBuf::from(
                    args.get(index + 1).cloned().ok_or("--at needs a directory")?,
                ));
                index += 2;
            }
            "--rvt" => {
                has_rvt = true;
                rest.push(args[index].clone());
                index += 1;
            }
            other => {
                rest.push(other.to_string());
                index += 1;
            }
        }
    }

    let Some(spec) = artifact else {
        return Ok(Route { args: rest, at, selected: false });
    };

    if has_rvt {
        return Err(
            "--artifact selects a payload already in the fleet repository; --rvt names one \
             directly. Pass one, not both. Nothing has been built or sent."
                .to_string(),
        );
    }

    let root = match &at {
        Some(dir) => dir.clone(),
        None => default_root()?,
    };
    let path = materialise_for_deploy(&root, &spec)?;
    let at_index = insert_at.unwrap_or(rest.len());
    rest.insert(at_index, path.display().to_string());
    rest.insert(at_index, "--rvt".to_string());
    Ok(Route { args: rest, at, selected: true })
}

/// The fleet repository `install` publishes into: `--at DIR`, or the default.
///
/// # Errors
///
/// No `--at`, and neither `HOME` nor `USERPROFILE` is set.
pub fn deploy_root(at: Option<&Path>) -> Result<PathBuf, String> {
    at.map_or_else(default_root, |dir| Ok(dir.to_path_buf()))
}

// ---------------------------------------------------------------------------
// install publishes, then sends — ADR-052 §7
// ---------------------------------------------------------------------------

/// **The application signing seed `install`'s automatic publish verifies
/// against**, found by [`crate::credentials::app_seed`] — the lookup `cargo
/// rivet package` uses — or refused naming both places it looked.
///
/// The fleet repository holds no seed and no path to one (`ADR-052` §4), so
/// the publish reaches a key exclusively through this lookup.
///
/// # Errors
///
/// No seed at either place. The refusal names `$RIVET_APP_SEED` and the default
/// path, and says nothing was published or sent.
pub fn deploy_app_seed(
    over: &crate::credentials::Overrides,
    rivet_root: Option<&Path>,
) -> Result<PathBuf, String> {
    crate::credentials::app_seed(over, rivet_root).map_err(|looked_at| {
        let default = looked_at.map_or_else(
            || "(no HOME or USERPROFILE, so no default path)".to_string(),
            |p| p.display().to_string(),
        );
        format!(
            "NOTHING WAS PUBLISHED AND NOTHING WAS SENT. `install` publishes an artifact into the \
             fleet repository before it sends it (ADR-052 §7), and a publish verifies the \
             artifact against the public half of the APPLICATION SIGNING SEED. No application \
             signing seed was found. Two places were looked at, in this order:\n\
             \x20 ${env}   not set\n\
             \x20 {default}   not a file\n\
             \n\
             Set ${env} to the seed that signed this artifact, or put that seed at the second \
             path. `cargo rivet package` looks in the same two places.",
            env = crate::APP_SEED_ENV,
        )
    })
}

/// **The system signing seed a system update's automatic publish verifies
/// against** — `R-177` D6, sibling to [`deploy_app_seed`]. Found by
/// [`crate::credentials::system_seed`], the same lookup `cargo rivet image`
/// uses.
///
/// # Errors
///
/// No seed at either place. The refusal names `$RIVET_SYSTEM_SEED` and the
/// default path, and says nothing was published or sent.
pub fn deploy_system_seed(
    over: &crate::credentials::Overrides,
    rivet_root: Option<&Path>,
) -> Result<PathBuf, String> {
    crate::credentials::system_seed(over, rivet_root).map_err(|looked_at| {
        let default = looked_at.map_or_else(
            || "(no HOME or USERPROFILE, so no default path)".to_string(),
            |p| p.display().to_string(),
        );
        format!(
            "NOTHING WAS PUBLISHED AND NOTHING WAS SENT. `install` publishes a system update into \
             the fleet repository before it sends it (ADR-052 §7, R-177 D6), and a publish \
             verifies the artifact against the public half of the SYSTEM SIGNING SEED. No system \
             signing seed was found. Two places were looked at, in this order:\n\
             \x20 ${env}   not set\n\
             \x20 {default}   not a file\n\
             \n\
             Set ${env} to the seed that signed this update, or put that seed at the second \
             path. `cargo rivet update` and `cargo rivet image` look in the same two places.",
            env = crate::credentials::SYSTEM_SEED_ENV,
        )
    })
}

/// Whether a file's header **claims** to be a system update.
///
/// **The claim is unauthenticated and decides only which seed a publish
/// verifies against** — `R-177` D6: an update publishes against the SYSTEM
/// seed and everything else against the APPLICATION seed, and this is the
/// one place that tells the two apart before either lookup runs. A file that
/// lies about its kind gains nothing: the wrong seed's public key will not
/// match its signature, and `publish_verified` refuses it exactly as it
/// refuses any other artifact signed by a key that was not asked for. A file
/// too short to hold the field claims nothing and is treated as ordinary.
fn claims_to_be_an_update(file: &Path) -> Result<bool, String> {
    use rivet_core::package::format::{KIND_BYTES, OFF_KIND, read_text};
    let mut head = vec![0u8; OFF_KIND + KIND_BYTES];
    let mut opened =
        std::fs::File::open(file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    match std::io::Read::read_exact(&mut opened, &mut head) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
        Err(e) => return Err(format!("cannot read {}: {e}", file.display())),
    }
    Ok(matches!(
        read_text(&head[OFF_KIND..]),
        Ok(Some(kind)) if kind == Kind::update().as_str()
    ))
}

/// `--signing-key PATH` among `package`'s options, when a crate deploy gave one.
fn named_signing_key(package_args: &[String]) -> Option<PathBuf> {
    package_args
        .iter()
        .position(|a| a == "--signing-key")
        .and_then(|at| package_args.get(at + 1))
        .map(PathBuf::from)
}

/// **Publish `file` into `root`, and hand back the repository's copy to send,
/// and what it declares (`SPEC-02` §5).**
///
/// Initialises the repository first when it is not one, because a deploy is
/// what the maintainer types and nothing he types changed — and says so with a
/// one-time `CREATED` line (`.rulings/` R-157 R14), printed only when `root`
/// was not a repository before this call. Publishing an artifact the
/// repository already holds is `publish`'s idempotent `have`, not an error.
///
/// **`R-177` D6 ends the system-update exception.** A system update publishes
/// exactly like anything else, verified against the SYSTEM seed
/// (`system_seed`) rather than the application one (`app_seed`) —
/// [`claims_to_be_an_update`] decides which lookup runs, so an update never
/// reaches for an application seed and nothing else ever reaches for a system
/// one. Uncompressed, per `ADR-029` §13/R-107: nothing here changes what an
/// update payload's own builder (`cargo rivet update`) produces, only what
/// happens to the bytes once they reach `deploy`.
fn publish_for_deploy(
    root: &Path,
    file: &Path,
    app_seed: &mut dyn FnMut() -> Result<PathBuf, String>,
    system_seed: &mut dyn FnMut() -> Result<PathBuf, String>,
) -> Result<(PathBuf, String, Declared), String> {
    let is_update = claims_to_be_an_update(file)?;
    let (seed_path, seed_label) = if is_update {
        (system_seed()?, "system signing seed")
    } else {
        (app_seed()?, "application signing seed")
    };
    let mut report = String::new();
    // **R14: a repository absent before this call is CREATED and said so, once
    // — never on a later deploy into the same, now-existing, repository.**
    if !root.join(".git").is_dir() {
        init(root).map_err(|e| {
            format!("NOTHING WAS SENT. The fleet repository at {} could not be made:\n{e}", root.display())
        })?;
        report.push_str(&format!(
            "fleet    CREATED the fleet repository at {} — this is the first install to use it \
             (ADR-052 §7, `.rulings/` R-157 R14)\n",
            root.display()
        ));
    }
    // A system update carries no schema version (that is a `card`-only field,
    // `R-177` item 3) and nothing else this function publishes does either.
    let published = publish_verified(root, file, None, &mut || {
        let bytes = crate::key::read_32(&seed_path, seed_label)?;
        Ok(rivet_crypto::Ed25519Signer::from_seed(&bytes).public_key())
    })
    .map_err(|e| {
        format!(
            "NOTHING WAS SENT. `install` publishes into the fleet repository at {} before it sends \
             (ADR-052 §7), and the publish refused:\n{e}",
            root.display()
        )
    })?;
    report.push_str(&format!("fleet    published into the fleet repository at {}\n", root.display()));
    report.push_str(&indent(&published.report));
    let copy = materialise_for_deploy(root, &published.digest)?;
    report.push_str(&format!(
        "fleet    sending the repository's copy, {}\n",
        copy.display()
    ));
    Ok((
        copy,
        report,
        Declared {
            libraries: published.declared_libraries,
            digest: published.digest,
        },
    ))
}

/// **What the publish that just verified says about the artifact**, carried
/// out of [`publish_for_deploy`] together because it is one read of one
/// artifact.
///
/// The declared library closure is checked before a byte is sent (`SPEC-02`
/// §5, `.rulings/` R-157 R15). Everything else a deploy needs from the
/// manifest — material, route, settings — is read by the planner's own reader,
/// `interview::read_artifact_plan_with`, from the object this digest names.
struct Declared {
    /// Every library reference the manifest names. Empty is the ordinary
    /// answer.
    libraries: Vec<String>,
    /// The digest the repository stored it under.
    digest: String,
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|line| if line.is_empty() { "\n".to_string() } else { format!("  {line}\n") })
        .collect()
}

/// `--host`/`--identity`/`--pin` as the install command that used them would
/// spell them again — for [`refuse_if_libraries_missing`]'s "the command that
/// deploys it" (`.rulings/` R-157 R15).
fn connection_flags(args: &rivet_remote::deploy::DeployArgs) -> String {
    match (&args.identity, &args.pin) {
        (Some(identity), Some(pin)) => {
            format!("--host {} --identity {} --pin {pin} ", args.host, identity.display())
        }
        _ => format!("--host {} ", args.host),
    }
}

/// **The fleet repository's own short digest for the published library whose
/// derived reference is exactly `target`**, when it holds one.
///
/// Uses the same derivation `fleet drift` compares with
/// (`crate::library::reference_of`), so a command suggested here and a match
/// reported there can never disagree.
fn find_library_digest(root: &Path, entries: &[Entry], target: &str) -> Option<String> {
    entries.iter().filter(|e| e.kind == "library").find_map(|e| {
        let object = root.join(OBJECTS_DIR).join(&e.digest);
        let reference = crate::library::reference_of(&object, &e.product, &e.version).ok()?;
        (reference.as_str() == target).then(|| e.digest[..e.digest.len().min(12)].to_string())
    })
}

/// **R15** (`.rulings/` R-157): refuse to send an application whose manifest
/// declares a library (`SPEC-02` §5) this machine does not hold — reading
/// `/state/lib` the way `cargo rivet fleet drift` already does, and refusing
/// **before anything is sent**. Names every missing reference and, where this
/// fleet repository already holds it, the exact command that deploys it.
/// **Never sends a library itself** — R-157's own words: "does not make
/// deploy install anything that wasn't named."
///
/// `declared` empty — `SPEC-02` §5's ordinary case, most applications — is
/// `Ok(())` without asking the machine anything.
///
/// # Errors
///
/// `get file /state/lib` did not answer, or one or more declared libraries are
/// not on this machine.
fn refuse_if_libraries_missing(
    args: &rivet_remote::deploy::DeployArgs,
    root: &Path,
    declared: &[String],
    remote: &mut dyn rivet_remote::deploy::Remote,
) -> Result<(), String> {
    if declared.is_empty() {
        return Ok(());
    }
    let installed = remote.installed_libraries().map_err(|why| {
        format!(
            "NOTHING WAS SENT. This application declares {n} librar{suffix} (SPEC-02 §5), and \
             `.rulings/` R-157 R15 checks {host} holds every one before anything is sent. `get \
             file /state/lib` did not answer: {why}",
            n = declared.len(),
            suffix = if declared.len() == 1 { "y" } else { "ies" },
            host = args.host,
        )
    })?;
    let missing: Vec<&String> = declared.iter().filter(|r| !installed.contains(r)).collect();
    if missing.is_empty() {
        return Ok(());
    }

    let entries = read_manifest(root).unwrap_or_default();
    let connection = connection_flags(args);
    let mut text = format!(
        "NOTHING WAS SENT. This application declares {} librar{} and {} of them {} not on {} \
         (SPEC-02 §5, `.rulings/` R-157 R15). Deploy refuses rather than sending a library that \
         was not named.\n\
         \n\
         Missing:\n",
        declared.len(),
        if declared.len() == 1 { "y" } else { "ies" },
        missing.len(),
        if missing.len() == 1 { "is" } else { "are" },
        args.host,
    );
    for reference in &missing {
        text.push_str(&format!("  {reference}\n"));
        match find_library_digest(root, &entries, reference) {
            Some(digest) => text.push_str(&format!(
                "    published in this fleet repository — install it first:\n\
                 \x20    cargo rivet install --artifact {digest} {connection}confirm\n"
            )),
            None => text.push_str(&format!(
                "    not published in this fleet repository either. Publish its `.rvt` and \
                 install it first:\n\
                 \x20    cargo rivet fleet publish <the library's .rvt> --trusted-key PATH\n\
                 \x20    cargo rivet install --artifact <the digest that reports> \
                 {connection}confirm\n"
            )),
        }
    }
    Err(text)
}

/// **Where this workstation's fleet authority lives** — the directory
/// `rivet-provision ca server` writes a machine's leaf into and
/// `cargo rivet secrets` reads it back out of.
///
/// The same rule `crate::secrets` resolves its own default by, spelled here
/// rather than imported so a test can hand a scratch directory: `$HOME/.rivet/ca`
/// or `%USERPROFILE%\.rivet\ca`, and the tilde form when neither is set — a
/// refusal that named no directory at all would be one an operator has to
/// finish themselves.
fn authority_directory() -> String {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|home| !home.is_empty())
        .map_or_else(
            || "~/.rivet/ca".to_string(),
            |home| {
                PathBuf::from(home)
                    .join(".rivet")
                    .join("ca")
                    .display()
                    .to_string()
            },
        )
}

/// **The machine's own name**, for the certificate an operator has to mint.
///
/// Read from the configuration document, which is the one place on the machine
/// that holds it — `rivet-provision ca server` takes the hostname alone and
/// composes `<hostname>.<suffix>` for every suffix the authority permits, so
/// this is exactly the word that verb wants.
///
/// `None` when the document is absent or says nothing: the refusal then names
/// the placeholder and says where to read it, rather than inventing a name for
/// a certificate that would be minted under it.
fn hostname_in(document: &str) -> Option<String> {
    for line in document.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            // Past the top-level keys; `hostname` is one of them
            // (`ADR-007` §6) and a `[table.hostname]` is not it.
            break;
        }
        if let Some(value) = line.strip_prefix("hostname")
            && let Some(value) = value.trim_start().strip_prefix('=')
        {
            let value = value.trim().trim_matches('"');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// The fleet name a leaf is minted for: the machine's hostname under the
/// fleet's own DNS suffix.
///
/// `local` is `rivet-provision`'s default suffix and the one every machine in
/// this fleet answers to; a fleet whose authority permits another reads the
/// name back off `ca server`'s own output, which is why the refusal says the
/// certificate's subject rather than only this.
const FLEET_SUFFIX: &str = "local";

/// **`.rulings/` R-194 D1: install does not touch the authority — it REFUSES,
/// naming the commands.**
///
/// An application whose signed manifest declares per-machine material
/// (`ADR-055` §3) cannot work until that material is in its data directory,
/// and nothing in a payload carries it: it is about *this* machine and it is
/// secret (`ADR-049` §1). The machine was asked what that directory holds
/// before anything is sent — `state`, read by the same
/// `interview::read_host_state_for` the interview plans from.
///
/// **What is missing is the planner's answer, not a second one.** The files
/// come from [`deploy_plan::plan`]'s `StageMaterial` step for this artifact on
/// this machine, and the two commands from [`next_steps::material_commands`],
/// which is also what prints that step after an install — so this refusal, the
/// step `install` prints, and the step the interview offers name the same
/// files and the same commands.
///
/// # What is refused, and what is only reported
///
/// **An install that would put the application INTO SERVICE without its material
/// is refused** — `--enable` on this command line, or a document that already
/// says `enabled = true`. That is the failure this exists for: three
/// applications crash-looped on exactly it, and the operator diagnosed it from
/// a journal on a headless machine.
///
/// **An install that only INSTALLS is reported and proceeds.** Installing is not
/// running (`ADR-015` §3), so an idle application with no credentials harms
/// nothing — and refusing here would be a wall rather than a refusal: the data
/// directory is made by the installer, and `apply install-secret` refuses a
/// missing one rather than creating it, so installing the application is what
/// makes the place its material goes. A refusal whose own instructions cannot
/// be carried out is worse than the crash-loop it replaces.
///
/// # Errors
///
/// The application would be running without material it declared.
fn refuse_if_material_missing(
    args: &rivet_remote::deploy::DeployArgs,
    artifact: &deploy_plan::ArtifactPlan,
    state: &deploy_plan::HostState,
    document: &str,
    reach: &next_steps::Reach<'_>,
) -> Result<String, String> {
    use rivet_core::package::application::MATERIAL_RULE;

    if artifact.material_files.is_empty() {
        return Ok(String::new());
    }
    let application = artifact.product.as_str();
    // **A machine holding its fleet's root issues the material itself**
    // (`ADR-063` §4), at this install, so there is nothing to refuse and no
    // command to offer.
    if state.issues_material {
        return Ok(format!(
            "material {} holds its fleet's root and issues {application} its {} itself at this \
             install; nothing is staged from this workstation\n",
            args.host,
            artifact.material_kinds.join(", "),
        ));
    }
    let stage = deploy_plan::plan(artifact, state).into_iter().find_map(|step| match step.kind {
        deploy_plan::StepKind::StageMaterial { kinds, missing } => Some((kinds, missing)),
        _ => None,
    });
    let Some((kinds, missing)) = stage else {
        return Ok(format!(
            "material {} holds every file {} declares ({})\n",
            args.host,
            application,
            artifact.material_kinds.join(", "),
        ));
    };

    // `None` is *no data directory*, which is an application that is not
    // installed here — see `Remote::data_directory_files`. Nothing is in it
    // either way, so the missing list is the same; the difference decides
    // whether the material could be installed right now.
    let installed = state.data_dir_files.is_some();
    // The document answers two questions: whether this application would be
    // running after the deploy, and what this machine is called — which is the
    // word `rivet-provision ca server` takes.
    let would_run = args.enable || state.standing == rivet_remote::deploy::Standing::Enabled;
    let hostname = hostname_in(document);
    let fleet_name = hostname
        .as_ref()
        .map_or_else(|| "<hostname>.local".to_string(), |h| format!("{h}.{FLEET_SUFFIX}"));

    let mut text = String::new();
    for (file, kind) in next_steps::missing_with_kinds(&kinds, &missing) {
        text.push_str(&format!("  {file}  ({kind})\n"));
    }
    let commands: String = next_steps::material_commands(application, hostname.as_deref(), reach)
        .iter()
        .map(|command| format!("  {command}\n"))
        .collect();
    let steps = format!(
        "Install it yourself, then run this install again:\n\
         \n\
         {commands}\
         \n\
         The FIRST line is once per MACHINE, not once per application: the leaf carries \
         the machine's own name and every application on it is handed that one pair \
         (ADR-046 §4). Skip it if {authority} already holds `server-{fleet_name}.crt`. \
         The SECOND line is once per application, and it is the one that puts the three \
         files where {application} reads them.\n\
         \n\
         {MATERIAL_RULE}.\n",
        authority = reach.authority,
    );
    let where_the_name_came_from = if hostname.is_some() {
        String::new()
    } else {
        format!(
            "\n\
             This machine's document did not say what it is called, so <hostname> is yours to \
             fill in — `cargo rivet run {}-- get document` is where it is written, and \
             `ca server` prints the certificate's subject, which is the name the second line \
             takes.\n",
            connection_flags(args),
        )
    };

    if !would_run {
        return Ok(format!(
            "material {application} declares material this machine does not hold, and it is \
             NOT being enabled — installing is not running (ADR-015 §3.2.2), so this install \
             proceeds and the application will sit idle until you say otherwise.\n\
             \n\
             Missing from /state/app/{application}/data:\n\
             {text}\
             \n\
             {steps}{where_the_name_came_from}"
        ));
    }

    let why_now = if installed {
        format!(
            "{application} is installed on {host} and its data directory holds none of what it \
             declared",
            host = args.host,
        )
    } else {
        format!(
            "{application} is not installed on {host} yet, so there is no data directory for \
             the material either — install it WITHOUT --enable first, which makes one, and then \
             install the material and enable it",
            host = args.host,
        )
    };
    Err(format!(
        "NOTHING WAS SENT. {why_now}. This install would put it into service without material \
         its own signed manifest declares (ADR-055 §3, `.rulings/` R-194), which is a \
         crash-loop an operator reads out of a journal rather than a refusal they read here.\n\
         \n\
         Missing from /state/app/{application}/data:\n\
         {text}\
         \n\
         {steps}{where_the_name_came_from}"
    ))
}

/// **`cargo rivet install`: publish into the fleet repository, then send.**
///
/// * Handed a crate, `package` builds and signs, the result is published into
///   `root`, and the repository's copy is what is transferred and installed by
///   digest.
/// * Handed `--rvt`, the file is published first and then the same path.
/// * Handed `--artifact` (`selected`), the payload is already the repository's
///   copy and nothing is published again.
///
/// **Without `confirm` nothing is published**: the plan changes nothing on
/// either machine, and this workstation's repository is on one of them.
///
/// `app_seed` and `system_seed` are the lookups [`deploy_app_seed`] and
/// [`deploy_system_seed`] perform; parameters so a test can hand scratch
/// seeds and never reach a real `HOME`. `publish_for_deploy` picks between
/// them by the file's claimed kind (`R-177` D6) — a crate install that named
/// `--signing-key` publishes against that key instead of `app_seed`, since it
/// is the one `package` signed with; there is no equivalent override for
/// `system_seed`, because an update never arrives from a crate build.
///
/// **The build, when there is one, happens here rather than inside
/// [`rivet_remote::deploy::run`]'s own lazy `build_payload` call** — since
/// `.rulings/` R-157 R14 and R15: publishing (which may create the
/// repository) and the library check (which needs `remote`) both have to run
/// before a byte is sent, and `run` must not be handed a `build_payload`
/// closure that itself borrows `remote` while `run` also holds it for the
/// send — two mutable borrows of one reference. Building here changes nothing
/// the maintainer types; `args.confirmed` is already known true by this point,
/// and deferring an unconfirmed build was `run`'s lazy build's entire reason
/// for existing.
///
/// **And it is the one planner behind both front ends** (`ADR-055` §3):
///
/// 1. Publish, or select, as above, and refuse a declared library the machine
///    does not hold (`.rulings/` R-157 R15, the publish path only).
/// 2. For an application or a library, read its needs with the interview's own
///    `interview::read_artifact_plan_with` and the machine with its own
///    `interview::read_host_state_for` — BEFORE anything is sent, which is the
///    reading the interview would have planned from.
/// 3. Refuse to put the application into service without its declared material
///    (`.rulings/` R-194), naming what the planner says is missing.
/// 4. Send and install, waiting for THAT install unless `--no-wait`.
/// 5. Print what [`deploy_plan::plan`] says remains, as commands
///    ([`next_steps`]), plus the fleet sync on a fleet server.
///
/// **On the `--artifact` route the manifest is read with the application
/// seed**, which that route did not use to need. A workstation without one
/// installs exactly as before and says the steps that remain could not be
/// planned, and why; it is not refused, because the install itself needs no
/// seed.
///
/// # Errors
///
/// A refused publish, a missing seed, or a declared library this machine does
/// not hold (`.rulings/` R-157 R15) — all before anything is sent; the machine
/// did not answer what it holds for this application, or the application would
/// run without its declared material — also before anything is sent; otherwise
/// [`rivet_remote::deploy::run_detailed`]'s errors, which include an install
/// that finished badly or never finished.
pub fn publish_then_send(
    root: &Path,
    args: rivet_remote::deploy::DeployArgs,
    invocation: &Invocation,
    app_seed: &mut dyn FnMut() -> Result<PathBuf, String>,
    system_seed: &mut dyn FnMut() -> Result<PathBuf, String>,
    build_payload: rivet_remote::deploy::BuildPayload<'_>,
    remote: &mut dyn rivet_remote::deploy::Remote,
) -> Result<String, String> {
    if invocation.selected && !args.confirmed {
        let report = rivet_remote::deploy::run(&args, build_payload, remote)?;
        let needs = needs(root, &args, invocation, app_seed, system_seed);
        return Ok(format!(
            "fleet    selected from the fleet repository, where it is already published\n\
             {report}{needs}"
        ));
    }

    if !args.confirmed {
        // `R-177` D6: everything publishes now, an update against the SYSTEM
        // seed and everything else against the application one.
        let seed_label = match &args.rvt {
            Some(file) if claims_to_be_an_update(file).unwrap_or(false) => "system",
            _ => "application",
        };
        let what = format!(
            "fleet    would publish into the fleet repository at {} FIRST, verified against the \
             {seed_label} signing seed, and send the repository's copy\n",
            root.display(),
        );
        let report = rivet_remote::deploy::run(&args, build_payload, remote)?;
        let needs = needs(root, &args, invocation, app_seed, system_seed);
        return Ok(format!("{what}{report}{needs}"));
    }

    install_confirmed(root, args, invocation, app_seed, system_seed, build_payload, remote)
}

/// **What the run with `confirm` will touch**, closing every plan, so whoever
/// runs it can be granted exactly that beforehand: a network destination,
/// files read and which of them hold private keys, files written, and what
/// happens on the machine.
///
/// **Composed from the arguments the confirmed run will use**, never from a
/// list kept beside this tool, so it names what the run does rather than what
/// somebody once wrote down about it. It opens nothing and connects to
/// nothing: the seed lookups report where a seed is without reading it, and a
/// seed that is not there is named as the refusal the confirmed run will give.
fn needs(
    root: &Path,
    args: &rivet_remote::deploy::DeployArgs,
    invocation: &Invocation,
    app_seed: &mut dyn FnMut() -> Result<PathBuf, String>,
    system_seed: &mut dyn FnMut() -> Result<PathBuf, String>,
) -> String {
    let update = matches!(&args.rvt, Some(file) if claims_to_be_an_update(file).unwrap_or(false));
    let building = args.rvt.is_none();
    let machine = invocation.device.as_deref().unwrap_or(&args.host);

    let mut reads: Vec<String> = Vec::new();
    if let Some(device) = &invocation.device {
        reads.push(format!(
            "rivet-fleet's register, to turn `{device}` into the address, identity and pin"
        ));
    }
    match &args.identity {
        Some(identity) => {
            reads.push(format!("{}: the operator identity, which holds a private key", identity.display()));
        }
        None => reads.push(
            "an operator identity and the machine's pin: NONE GIVEN, and the run with `confirm` \
             refuses without --identity and --pin, or a device name that supplies them"
                .to_owned(),
        ),
    }
    let (seed, role) = if update { (system_seed(), "system") } else { (app_seed(), "application") };
    let purpose = if invocation.selected { ", only to plan the steps that remain" } else { "" };
    reads.push(match seed {
        Ok(path) => format!("{}: the {role} signing seed, a private key{purpose}", path.display()),
        Err(_) => format!(
            "the {role} signing seed: NONE FOUND where the lookup looks, and the run with \
             `confirm` refuses without one"
        ),
    });
    if building {
        reads.push("the crate `cargo rivet package` builds, and its dependencies".to_owned());
    } else if let Some(file) = &args.rvt
        && !invocation.selected
    {
        reads.push(format!("{}: the payload", file.display()));
    }

    let writes = if invocation.selected {
        args.rvt.as_deref().and_then(Path::parent).map_or_else(
            || "a copy of the payload in the system's temporary directory".to_owned(),
            |dir| format!("{}: a copy of the payload, already made for this plan", dir.display()),
        )
    } else {
        format!(
            "{}: the fleet repository; the payload is published and committed there first, \
             and the repository is made if it is absent",
            root.display()
        )
    };

    let size = args
        .rvt
        .as_deref()
        .and_then(|file| std::fs::metadata(file).ok())
        .map_or_else(String::new, |meta| format!("{} bytes ", meta.len()));
    let on_machine = if update {
        format!(
            "sends {size}into its artifact store and writes the slot it is NOT running, arming \
             the next boot as a one-shot trial of it. Nothing reboots: `apply trial-restart \
             confirm` is a separate command"
        )
    } else {
        format!(
            "sends {size}into its artifact store and installs it by digest{}. Nothing starts it: \
             the steps printed after the install do, and each of them reaches the same machine",
            if args.enable {
                ", after appending its enable to /state/config/machine.toml"
            } else {
                ""
            }
        )
    };

    let mut out = String::from(
        "\nNEEDS — what the run with `confirm` will touch, so that whoever runs it can be \
         granted exactly this beforehand:\n",
    );
    out.push_str(&format!("  network   {} (TCP): the machine's control plane\n", args.host));
    if building {
        out.push_str(&format!(
            "  {:<9} cargo, to build for the board's target, fetching any crate it has not \
             already cached\n",
            "runs"
        ));
    }
    for (index, line) in reads.iter().enumerate() {
        let label = if index == 0 { "reads" } else { "" };
        out.push_str(&format!("  {label:<9} {line}\n"));
    }
    out.push_str(&format!("  writes    {writes}\n"));
    out.push_str(&format!("  on {machine}: {on_machine}\n"));
    out
}

/// **How `cargo rivet install` was invoked**, beyond what
/// [`rivet_remote::deploy::DeployArgs`] carries: whether the payload was
/// selected from the repository, and the words its printed next steps spell the
/// machine and the repository with.
#[derive(Debug, Default)]
pub struct Invocation {
    /// `--artifact`: the payload is already the repository's copy and is not
    /// published again.
    pub selected: bool,
    /// The device name a bare leading word named, before it was resolved into
    /// `--host`, `--identity` and `--pin`.
    pub device: Option<String>,
    /// `--at DIR`, when one was given.
    pub at: Option<PathBuf>,
}

/// [`publish_then_send`] once `confirm` has been typed — its steps 1 to 5.
fn install_confirmed(
    root: &Path,
    mut args: rivet_remote::deploy::DeployArgs,
    invocation: &Invocation,
    app_seed: &mut dyn FnMut() -> Result<PathBuf, String>,
    system_seed: &mut dyn FnMut() -> Result<PathBuf, String>,
    build_payload: rivet_remote::deploy::BuildPayload<'_>,
    remote: &mut dyn rivet_remote::deploy::Remote,
) -> Result<String, String> {
    // `args.confirmed` is true from here. Resolve the local file — building it
    // now when it was not given — so publish (which may print R14's `CREATED`
    // line) and R15's library check both run before `rivet_remote::deploy::run`
    // is asked to send anything. See this function's own documentation for why
    // the build cannot stay inside `run`'s lazy `build_payload` any more.
    let mut build_report = String::new();

    // `--signing-key` named for a CRATE build overrides the application seed
    // lookup, exactly as `cargo rivet package` honours it — there is no
    // equivalent override for a system update, which always reaches
    // `system_seed` (an update never arrives via a crate build; see
    // `publish_for_deploy`'s own documentation).
    let named = named_signing_key(&args.package_args);
    let mut app = || match &named {
        Some(path) => Ok(path.clone()),
        None => app_seed(),
    };

    let (copy, published, digest) = if invocation.selected {
        let file = args
            .rvt
            .take()
            .ok_or("internal error: --artifact resolved to no file, so there is nothing to send")?;
        let (bytes, _) = blake3_of(&file)?;
        (
            file,
            "fleet    selected from the fleet repository, where it is already published\n".to_string(),
            crate::key::hex(&bytes),
        )
    } else {
        let file = match args.rvt.take() {
            Some(file) => file,
            None => {
                let (built, report) = build_payload(&args.package_args)?;
                build_report = report;
                built
            }
        };
        let (copy, published, declared) = publish_for_deploy(root, &file, &mut app, system_seed)?;
        if let Err(refusal) = refuse_if_libraries_missing(&args, root, &declared.libraries, remote) {
            return Err(format!("{build_report}{published}{refusal}"));
        }
        (copy, published, declared.digest)
    };
    args.rvt = Some(copy);

    // **What the planner reads**, for an application or a library — the
    // artifact's own signed needs, by the interview's own reader. An update or
    // a card has no planner step and none of this applies.
    let entries = read_manifest(root)?;
    let entry = entries
        .iter()
        .find(|e| e.digest == digest && (e.kind == "application" || e.kind == "library"))
        .cloned();
    let mut unplanned: Option<String> = None;
    let planned: Option<(Entry, deploy_plan::ArtifactPlan, PathBuf)> = match entry {
        None => None,
        Some(entry) => {
            match app().and_then(|seed| {
                interview::read_artifact_plan_with(root, &entry, &seed).map(|artifact| (artifact, seed))
            }) {
                Ok((artifact, seed)) => Some((entry, artifact, seed)),
                Err(why) if invocation.selected => {
                    unplanned = Some(why);
                    None
                }
                // The publish verified these very bytes a moment ago, so a
                // manifest that will not read now is an inconsistency, not a
                // missing seed.
                Err(why) => return Err(format!("{build_report}{published}NOTHING WAS SENT. {why}")),
            }
        }
    };
    let artifact = planned.as_ref().map(|(_, artifact, _)| artifact);

    let authority = authority_directory();
    let reach = next_steps::Reach {
        args: &args,
        device: invocation.device.as_deref(),
        at: invocation.at.as_deref(),
        authority: &authority,
    };

    // **The machine as it stands, BEFORE anything is sent** — the reading the
    // interview would have planned from — and `.rulings/` R-194 against it,
    // beside the library check and for its reason: the answer comes from the
    // machine, it is known before a byte is sent, and the alternative is an
    // operator reading a crash-loop out of a journal. It reports rather than
    // refuses when nothing would start — see the function.
    let mut material = String::new();
    let mut unresolved = String::new();
    let before = match &planned {
        None => None,
        Some((entry, artifact, seed)) => {
            let did_not_answer = |why: String| {
                format!(
                    "{build_report}{published}NOTHING WAS SENT. Before anything is sent, install asks \
                     {host} what it holds for {product} — its document, and its data directory when \
                     material is declared (ADR-055 §3, `.rulings/` R-194) — which is also what the \
                     steps after the install are planned from. That read did not answer: {why}",
                    host = args.host,
                    product = artifact.product,
                )
            };
            let (state, document) = interview::read_host_state_for(remote, artifact).map_err(did_not_answer)?;
            material = refuse_if_material_missing(&args, artifact, &state, &document, &reach)
                .map_err(|refusal| format!("{build_report}{published}{refusal}"))?;

            // **Its unmet `connects` dependencies, exactly as the interview
            // resolves and plans them** (`ADR-055` §4) — a system service
            // switched off, or an artifact this repository holds that is not
            // installed and enabled here. The interview offers them before the
            // artifact's own steps; so does what this prints.
            let (resolved, skipped) = interview::resolve_dependencies(
                &entries,
                &[(entry.clone(), artifact.clone())],
                &mut |e| interview::read_artifact_plan_with(root, e, seed),
            )
            .map_err(|why| format!("{build_report}{published}NOTHING WAS SENT. {why}"))?;
            if let Some(why) = skipped.get(&artifact.product) {
                unresolved = format!(
                    "\nwarning  {}: {why}. No step below can resolve that; the machine will not \
                     start it until something provides that connection.\n",
                    artifact.product,
                );
            }
            let mut dependencies = Vec::new();
            for dep in &artifact.depends_on {
                let found = resolved.iter().find(|(e, _)| &e.product == dep);
                if let Some((_, dep_artifact, steps, reason)) =
                    interview::dependency_step_group(remote, &document, &artifact.product, dep, found)
                        .map_err(did_not_answer)?
                {
                    dependencies.push(next_steps::Dependency {
                        standing: rivet_remote::deploy::standing(&document, dep),
                        artifact: dep_artifact,
                        steps: steps.into_iter().map(|step| step.kind).collect(),
                        reason,
                    });
                }
            }
            Some((state, document, dependencies))
        }
    };
    let enabled_by_install = args.enable
        && before.as_ref().is_some_and(|(state, _, _)| {
            matches!(
                state.standing,
                rivet_remote::deploy::Standing::Absent | rivet_remote::deploy::Standing::Disabled
            )
        });

    let mut already_built = |_: &[String]| -> Result<(PathBuf, String), String> {
        Err("internal error: install already built and published this payload before calling \
             rivet_remote::deploy::run, which should never need to build again"
            .to_string())
    };
    // A refusal after the publish still says the publish happened: the
    // artifact is in the repository whatever the machine made of it.
    let deployed = rivet_remote::deploy::run_detailed(&args, &mut already_built, remote)
        .map_err(|refusal| format!("{build_report}{published}{material}{refusal}"))?;

    let next = match (artifact, &before) {
        (Some(artifact), Some((state, document, dependencies))) => {
            let (installed, waited) = match deployed.delivered {
                rivet_remote::deploy::Delivered::AlreadyRunning => (false, true),
                rivet_remote::deploy::Delivered::Requested => (true, false),
                _ => (true, true),
            };
            let steps = next_steps::next_steps(
                artifact,
                state,
                document,
                dependencies,
                enabled_by_install,
                installed,
                &reach,
            );
            format!("{}{unresolved}", next_steps::render(&artifact.product, &args.host, &steps, waited))
        }
        _ => unplanned.map_or_else(String::new, |why| {
            format!(
                "\nnext     THE STEPS THAT REMAIN COULD NOT BE PLANNED: the planner reads the \
                 artifact's signed manifest, verified against the application signing seed, \
                 and {why}\n"
            )
        }),
    };
    let is_update = entries.iter().any(|e| e.digest == digest && e.kind == "update");
    let record = if is_update { record_after_commit(&reach) } else { String::new() };
    Ok(format!("{build_report}{published}{material}{}{next}{record}", deployed.report))
}

/// **What a system update install says remains**: the trial, and recording
/// it once it commits (`ADR-052` §7, R-233). The install returns before the
/// trial, so it cannot itself know the update committed; `fleet record` reads
/// that from the machine and records nothing until it has.
fn record_after_commit(reach: &next_steps::Reach<'_>) -> String {
    let (machine, flags) = match reach.device {
        Some(device) => (device.to_owned(), format!("{device} ")),
        None => (reach.args.host.clone(), connection_flags(reach.args)),
    };
    let at = reach.at.map_or_else(String::new, |at| format!("--at {} ", at.display()));
    format!(
        "\nnext     a system update is recorded as what {machine} runs only once it has COMMITTED \
         (R-233), so a publish never prunes it (ADR-052 §7):\n\
         \x20  cargo rivet run {flags}-- apply trial-restart confirm\n\
         \x20  cargo rivet run {flags}-- get slot          until it reports committed: true\n\
         \x20  cargo rivet fleet record {flags}{at}confirm\n"
    )
}

// ---------------------------------------------------------------------------
// the command line
// ---------------------------------------------------------------------------

/// One of `fleet`'s sub-verbs.
enum SubVerb {
    Init,
    /// `--trusted-key` is an `.rvt`'s; `--product` and `--signing-key` an
    /// `.apk`'s (`ADR-061` §4). [`run`] refuses each given to the other.
    /// `--sync [confirm]` is either kind's (`ADR-061` §4, `.rulings/`
    /// R-222): after the publish lands in this workstation's repository,
    /// send the fleet server every object its manifest names that it lacks,
    /// then push `main` — `endpoint`'s `--host`/`--identity`/`--pin` override
    /// what the register's one `role = "fleet-server"` record would resolve.
    Publish {
        file: PathBuf,
        trusted_key: Option<PathBuf>,
        product: Option<String>,
        signing_key: Option<PathBuf>,
        sync: bool,
        sync_confirm: bool,
        endpoint: rivet_remote::endpoint::Endpoint,
        /// `--fleet NAME` (`ADR-063` §6): publish to that bookmarked fleet's
        /// server rather than into a workstation repository, then fetch its
        /// clone. `sync_confirm` is its `confirm`.
        fleet: Option<String>,
    },
    /// `cargo rivet fleet bookmark …` — `ADR-063` §2.
    Bookmark(bookmark::BookmarkArgs),
    /// `cargo rivet fleet fetch <fleet> [--into DIR]` — `ADR-063` §6.
    Fetch { fleet: String, into: Option<PathBuf> },
    /// `--hidden` lists the products withheld from every fleet too, marked
    /// (`ADR-052` §7); without it they are folded into one closing line.
    List { show_hidden: bool },
    /// `cargo rivet fleet plan <host>` — the read-only dry run of the interview
    /// (`docs/design-candidates/Fleet-Deployment-Tool.md` §10.2): for every
    /// application artifact, print the steps a deploy would plan against
    /// `<host>`, executing nothing. `<host>` is a bare device name resolved
    /// against the register, as `run`/`install` resolve one. `--all` shows every
    /// artifact and version; without it, only the latest of each non-system
    /// application (the default filter).
    Plan { host: String, all: bool },
    /// `ADR-052` §9 step 3 — the only sub-verb that reaches a machine, which
    /// is why it alone carries an [`rivet_remote::endpoint::Endpoint`].
    Drift { endpoint: rivet_remote::endpoint::Endpoint },
    /// `ADR-052` §9 step 4, `.rulings/` R-184 — pushes this repository's
    /// `main` to a fleet server's `fleet` repository. Changes a machine, so
    /// it plans without `confirm`.
    Push { device: Option<String>, endpoint: rivet_remote::endpoint::Endpoint, confirm: bool },
    /// `ADR-052` §7, R-233 — records in a host's register record which system
    /// update its running, committed slot holds. Writes the register, so it
    /// plans without `confirm`.
    Record { device: Option<String>, endpoint: rivet_remote::endpoint::Endpoint, confirm: bool },
    /// `.rulings/` R-184 D1 — sends the application and library objects this
    /// repository's manifest names to a fleet server's repository store, and
    /// with `--prune` asks it to remove what its manifest stops naming.
    Sync {
        device: Option<String>,
        endpoint: rivet_remote::endpoint::Endpoint,
        prune: bool,
        confirm: bool,
    },
    /// `cargo rivet fleet objects backup <dir> [--all] [--replace]` —
    /// `ADR-052`, R-183 D4.
    ObjectsBackup { dest: PathBuf, all: bool, replace: bool },
    /// `cargo rivet fleet objects restore <dir> [--replace]`.
    ObjectsRestore { source: PathBuf, replace: bool },
    /// `cargo rivet fleet phones list | rename | revoke` — `ADR-061` §6. No
    /// `<host>`, unlike `push`/`sync`: the ONE register record with
    /// `role = "fleet-server"` is found the same way `--sync` finds it
    /// ([`resolve_fleet_server`]), never named.
    Phones(phones::PhonesArgs),
    /// `cargo rivet fleet root <host>` — `ADR-063` §4.
    Root(root::RootArgs),
    /// `cargo rivet fleet retire | hide | unhide <product>` — `ADR-052` §7.
    Circulation(Circulation),
}

/// What `fleet retire`, `hide` or `unhide` was asked to do.
struct Circulation {
    action: CirculationAction,
    product: String,
    kind: Option<String>,
    /// `--sync [confirm]`: after the local commit, push and print the
    /// server's prune plan — never prune.
    sync: bool,
    sync_confirm: bool,
    endpoint: rivet_remote::endpoint::Endpoint,
}

#[derive(Debug, PartialEq, Eq)]
enum CirculationAction {
    /// `version` is `--version V`, or the whole product.
    Retire { version: Option<String> },
    Hide,
    Unhide,
}

pub struct FleetArgs {
    /// `--at DIR`, or `None` for the default — resolved by [`run`] rather
    /// than here, so a test can assert what the default resolves to without
    /// needing `HOME` set.
    at: Option<PathBuf>,
    verb: SubVerb,
}

pub fn usage() -> String {
    format!(
        "usage: cargo rivet fleet init    [--at DIR]\n\
     \x20             fleet publish FILE.rvt --trusted-key PATH [--at DIR]\n\
     \x20                           [--sync [--host HOST:PORT] [--identity DIR]\n\
     \x20                                   [--pin HEX] [confirm]]\n\
     \x20             fleet publish FILE.apk [--product NAME] [--signing-key PATH] [--at DIR]\n\
     \x20                           [--sync [--host HOST:PORT] [--identity DIR]\n\
     \x20                                   [--pin HEX] [confirm]]\n\
     \x20             fleet publish FILE.rvt --trusted-key PATH --fleet NAME [confirm]\n\
     \x20             fleet fetch   NAME [--into DIR]\n\
     \x20             fleet bookmark add <fleet> <server> [--host HOST:PORT] [--identity DIR]\n\
     \x20                                  [--pin HEX] [--replace]\n\
     \x20             fleet bookmark list | remove <fleet>\n\
     \x20             fleet list    [--hidden] [--at DIR]\n\
     \x20             fleet plan    <host> [--all] [--at DIR]\n\
     \x20             fleet drift   --host HOST:PORT --identity DIR --pin HEX [--at DIR]\n\
     \x20             fleet push    <host> [--host HOST:PORT] [--identity DIR] [--pin HEX]\n\
     \x20                           [--at DIR] [confirm]\n\
     \x20             fleet sync    <host> [--host HOST:PORT] [--identity DIR] [--pin HEX]\n\
     \x20                           [--at DIR] [--prune] [confirm]\n\
     \x20             fleet record  [<host>] [--host HOST:PORT] [--identity DIR] [--pin HEX]\n\
     \x20                           [--at DIR] [confirm]\n\
     \x20             fleet objects backup  <dir> [--all] [--replace] [--at DIR]\n\
     \x20             fleet objects restore <dir> [--replace] [--at DIR]\n\
     \x20             fleet phones list   [--host HOST:PORT] [--identity DIR] [--pin HEX]\n\
     \x20             fleet phones rename <identity> <label> [--host HOST:PORT]\n\
     \x20                                 [--identity DIR] [--pin HEX] [confirm]\n\
     \x20             fleet phones revoke <identity> [--host HOST:PORT] [--identity DIR]\n\
     \x20                                 [--pin HEX] [confirm]\n\
     \x20             fleet root    <host> [--out DIR] [--host HOST:PORT] [--identity DIR]\n\
     \x20                           [--pin HEX]\n\
     \x20             fleet retire  <product> [--kind K] [--version V] [--at DIR]\n\
     \x20                           [--sync [--host HOST:PORT] [--identity DIR]\n\
     \x20                                   [--pin HEX] [confirm]]\n\
     \x20             fleet hide    <product> [--kind K] [--at DIR] [--sync … [confirm]]\n\
     \x20             fleet unhide  <product> [--kind K] [--at DIR] [--sync … [confirm]]\n\
     \n\
     ADR-052's FLEET REPOSITORY — a git-tracked manifest plus a digest-keyed\n\
     object store, on THIS workstation, naming every signed artifact the\n\
     maintainer's fleet is built from. `install --artifact ID` sends one to a\n\
     machine.\n\
     \n\
     THIS IS NOT `rivet-fleet` (a different binary, for a machine's own\n\
     document and the register), AND `[repos.<name>]` IS A DIFFERENT THING — a\n\
     git repository a RIVET MACHINE hosts and serves, which `rivet-fleet add\n\
     repo` / `remove repo` manage. Two things that both start `git init`; this\n\
     tool always says \"fleet repository\" in full for exactly that reason.\n\
     They meet in one place only: `fleet push` sends THIS repository's text to\n\
     a fleet server's `[repos.fleet]` repository.\n\
     \n\
     \x20 --at DIR   where the fleet repository is. Default: ~/.rivet/fleet, or\n\
     \x20            %USERPROFILE%\\.rivet\\fleet\n\
     \n\
     init      create it. Refuses a tree that already holds a `.seed` file\n\
     \x20         (ADR-052 §4) — no flag overrides that. Running it twice\n\
     \x20         reports what is already there and changes nothing.\n\
     \n\
     publish   verify FILE.rvt — signature, digest, every member, exactly as\n\
     \x20         `cargo rivet inspect` does — and store it under its own\n\
     \x20         BLAKE3, recorded in the manifest with its product, version,\n\
     \x20         architecture, kind, size and when it was added. Publishing\n\
     \x20         the same artifact twice is idempotent: the digest is the\n\
     \x20         name, so a repeat is reported and changes nothing.\n\
     \x20         --trusted-key PATH   REQUIRED — same posture as `inspect`:\n\
     \x20                              trusted for every kind this build\n\
     \x20                              reads, which is a developer checking\n\
     \x20                              their own artifact and not a fleet's\n\
     \x20                              real trust (ADR-028 §5 is that).\n\
     \n\
     publish FILE.apk   ADR-061 §4. An Android APK — named .apk AND a zip, or\n\
     \x20         refused — read by this tool itself (package name, versionCode,\n\
     \x20         versionName, label, SDK levels, signing certificate, rotation\n\
     \x20         lineage), wrapped as an `apk` artifact (app.apk stored as it\n\
     \x20         is, apk.json), signed with the application seed and published\n\
     \x20         as above. REFUSES a versionCode not higher than the product's\n\
     \x20         newest here, and a certificate that is neither the product's\n\
     \x20         last one nor linked to it by the APK's v3 rotation lineage,\n\
     \x20         naming both. A product's first publish pins its certificate\n\
     \x20         and says so.\n\
     \x20         --product NAME       the product; default: the application\n\
     \x20                              id's last segment, `.dev` as `-dev`.\n\
     \x20         --signing-key PATH   the application seed; default:\n\
     \x20                              $RIVET_APP_SEED, then\n\
     \x20                              ~/.rivet/keys/rivet-app.seed.\n\
     \n\
     --sync    ADR-061 §4, one flow for either kind of `publish` — after\n\
     \x20         the publish lands in this workstation's repository (which\n\
     \x20         needs no confirm either way), send the fleet server every\n\
     \x20         object THIS repository's manifest names that it lacks, then\n\
     \x20         push `main` — files, then the list, never the other order:\n\
     \x20         a failed send means main is never pushed. WITHOUT `confirm`\n\
     \x20         NOTHING IS SENT: it prints what would be sent and what a\n\
     \x20         push would do. Uncommitted or untracked files never block\n\
     \x20         it — they never travel either way — and are named as left\n\
     \x20         behind instead; `fleet push` alone keeps its stricter\n\
     \x20         refusal. The server is found, never named: the one register\n\
     \x20         record with role = \"fleet-server\" (the same record\n\
     \x20         rivet-fleetd finds itself by) — none, or more than one, is\n\
     \x20         refused by name.\n\
     {IDENTITY_AND_PIN}\
     \n\
     --fleet NAME   ADR-063 §6 — THE FLEET SERVER IS THE AUTHORITY. Publish\n\
     \x20         FILE.rvt to the bookmarked fleet NAME's server instead of into a\n\
     \x20         workstation repository: one commit is composed on the server's\n\
     \x20         main in a scratch repository beside the clone, the object is sent\n\
     \x20         to the server's repository store (verified at its door against\n\
     \x20         ITS trust store), then main is pushed as a fast-forward, then the\n\
     \x20         clone fetches. Without `confirm` nothing reaches the server.\n\
     \x20         Takes no --at, --sync, --host, --identity or --pin: the bookmark\n\
     \x20         names the server.\n\
     \n\
     fetch     ADR-063 §6. Brings the fleet NAME's fetch-only clone,\n\
     \x20         ~/.rivet/fleets/NAME/ (or --into DIR), level with its server:\n\
     \x20         main fast-forwarded (a clone with history the server lacks is\n\
     \x20         refused), every object the server holds read back and kept only\n\
     \x20         when it hashes to its digest, and every object it no longer holds\n\
     \x20         removed. Two clones are stale or current, never contradictory.\n\
     \x20         `fleet list --at` and `install --artifact ID --at` read a clone.\n\
     \n\
     bookmark  ADR-063 §2. `add <fleet> <server>` records how this workstation\n\
     \x20         reaches a fleet — its server's address (a DHCP record as\n\
     \x20         <server>.local), its pin and the operator identity — read from\n\
     \x20         rivet-fleet's register, which it does not write; the flags win.\n\
     \x20         ~/.rivet/bookmarks.toml. `list`, and `remove <fleet>`.\n\
     \n\
     list      what the repository holds, one line per artifact. A hidden\n\
     \x20         product is left out and counted on one closing line;\n\
     \x20         --hidden lists it too, marked [hidden].\n\
     \n\
     drift     ADR-052 §9 step 3. Asks a machine's `get status` which\n\
     \x20         services it holds, asks `get file` what each installed\n\
     \x20         application's `current` symlink points at, and compares\n\
     \x20         that version against this repository's manifest. READS\n\
     \x20         ONLY — nothing on the machine or in this repository is\n\
     \x20         changed. A service with no `/state/app/<name>` is reported\n\
     \x20         as NOT COMPARABLE rather than as a difference: it is most\n\
     \x20         likely image-defined, and this repository has never had an\n\
     \x20         artifact for one of those to compare against. Shared rivets\n\
     \x20         are compared too: `get file /state/lib` lists the machine's\n\
     \x20         library references, and each is matched exactly against the\n\
     \x20         reference derived from every library this repository holds\n\
     \x20         (SPEC-02 §3). A MACHINE THAT DOES NOT ANSWER IS REFUSED\n\
     \x20         rather than reported as a match or a difference.\n\
     \x20         --host HOST:PORT   REQUIRED. Same flag `run` and `install`\n\
     \x20                            take — the port is not guessed.\n\
     {IDENTITY_AND_PIN}\
     \n\
     push      ADR-052 §9 step 4. Pushes THIS repository's `main` to `main`\n\
     \x20         of <host>'s `fleet` repository, over ADR-035's control plane\n\
     \x20         (7777), as the operator identity. <host> is a device the\n\
     \x20         register holds, resolved the way `run <device>` resolves one;\n\
     \x20         --host/--identity/--pin win over the register, and all three\n\
     \x20         replace a device name. WITHOUT `confirm` NOTHING IS PUSHED: it\n\
     \x20         reads the server's main and prints what a push would do.\n\
     \x20         REFUSES uncommitted changes, a repository with no `main`, a\n\
     \x20         server with no `fleet` repository (its document must declare\n\
     \x20         [repos.fleet]), and anything that is not a fast-forward — it\n\
     \x20         never forces. The TEXT only: application objects are not in\n\
     \x20         git and do not travel with it. git is handed a one-command\n\
     \x20         loopback relay that makes the pinned TLS handshake itself, so\n\
     \x20         no key file, CA file or git configuration is written.\n\
     \n\
     sync      R-184 D1. Sends the application and library objects THIS\n\
     \x20         repository's manifest names to <host>'s repository store, a\n\
     \x20         digest-keyed store in rivet-fleetd's own data directory kept by\n\
     \x20         the machine's control plane (7777). <host> resolves as for\n\
     \x20         push. It asks the server what it holds, sends ONLY the\n\
     \x20         difference from objects/, and reads the store back: a rerun\n\
     \x20         sends only what is still not held. Every object is verified\n\
     \x20         at the server's door against its trust store, cards and\n\
     \x20         updates included (ADR-063 §6). WITHOUT `confirm` NOTHING IS SENT:\n\
     \x20         it prints what it would send. An object the manifest names\n\
     \x20         and objects/ lacks is reported MISSING and the run exits\n\
     \x20         non-zero.\n\
     \x20         --prune   with confirm, and only after a complete sync, asks\n\
     \x20                   the server to remove every object ITS manifest no\n\
     \x20                   longer names. Refused while the server's fleet main\n\
     \x20                   differs from this main (push first); the server\n\
     \x20                   refuses too while anything named is missing.\n\
     \n\
     record    ADR-052 §7, R-233. WRITES INTO A HOST'S REGISTER RECORD which\n\
     \x20         system update its running slot holds, then projects the\n\
     \x20         register into this repository. Run it after a system\n\
     \x20         update's trial has committed. <host> is a device the register\n\
     \x20         holds, or --pin finds its record; a machine with no record is\n\
     \x20         refused, never given one. It reads `get slot` and REFUSES\n\
     \x20         unless the running slot is committed, then matches the header\n\
     \x20         the machine retains for that slot against this repository's\n\
     \x20         system updates. Only the running slot is written; the other\n\
     \x20         slot keeps what the record already said. WITHOUT `confirm`\n\
     \x20         NOTHING IS WRITTEN: it prints what it would record.\n\
     \x20         Every publish keeps the three newest of each product, kind,\n\
     \x20         platform and arch, AND every digest a host's record names.\n\
     \n\
     objects backup <dir>    ADR-052, R-183 D4. Copies bytes out of THIS\n\
     \x20         repository's objects/ into <dir>/objects/ — never a git\n\
     \x20         operation, since objects/ is gitignored to begin with.\n\
     \x20         Default scope: only the digests manifest.toml currently\n\
     \x20         references (what would let this fleet be recovered as it\n\
     \x20         stands today) — --all copies every digest objects/ holds,\n\
     \x20         including superseded -dirty- builds nothing references any\n\
     \x20         more. Refuses to overwrite an object already at <dir> —\n\
     \x20         naming it and its age — unless --replace is given.\n\
     \n\
     objects restore <dir>   the inverse: copies <dir>/objects/* back into\n\
     \x20         THIS repository's objects/, recomputing every object's own\n\
     \x20         BLAKE3 and refusing any whose filename does not match its\n\
     \x20         content — a restored repository is never worse-verified\n\
     \x20         than one freshly received. Nothing is written if ANY object\n\
     \x20         fails that check. Refuses to overwrite a conflicting object\n\
     \x20         already in this repository unless --replace is given.\n\
     \n\
     phones    ADR-061 §6, the agent's surface over the phones a fleet server's\n\
     \x20         `gantry` socket also serves — the same board `--sync` finds, never\n\
     \x20         named: the one register record with role = \"fleet-server\". `list`\n\
     \x20         reads `get phones` and decodes each record; `rename` reads the\n\
     \x20         current record, changes only its label, re-renders the whole record\n\
     \x20         and sends it with `apply rename-phone`; `revoke` sends `apply\n\
     \x20         revoke-phone`. WITHOUT `confirm`, `rename` prints a LOCAL preview\n\
     \x20         (old label -> new) and sends nothing, and `revoke` prints the\n\
     \x20         machine's own plan (ADR-012 §7) and changes nothing. `list` needs\n\
     \x20         no `confirm`: it changes nothing.\n\
     {IDENTITY_AND_PIN}\
     \n\
     retire    ADR-052 §7, R-225. TAKES A PRODUCT OUT OF CIRCULATION: removes\n\
     \x20         its entries from the manifest and its objects from objects/\n\
     \x20         — every version, or --version V's alone — and commits the\n\
     \x20         manifest alone. --kind K when the name is published as more\n\
     \x20         than one kind. Nothing installed changes. REFUSED for an\n\
     \x20         application a machine's card carries or pins (a document\n\
     \x20         under documents/, or the register), naming each machine and\n\
     \x20         how to unpin it. An apk's signing-key pin goes with its last\n\
     \x20         version, and the signer dropped is printed. `cargo rivet\n\
     \x20         artifacts` is the same core as a menu, for a person.\n\
     \n\
     hide      withholds a product from every fleet and keeps it here: sync\n\
     \x20         stops sending it, a fleet server's prune removes it, and the\n\
     \x20         deploy menus and `list` leave it out. Always allowed — a card\n\
     \x20         build still finds it. `unhide` returns it at the next sync.\n\
     \n\
     --sync    for retire, hide and unhide: after the local commit, send the\n\
     \x20         fleet server what it lacks and push main, as `publish --sync`\n\
     \x20         does, then print the list the server's prune would delete —\n\
     \x20         AND STOP: it never prunes, which is always `fleet sync\n\
     \x20         <server> --prune confirm`. Without `confirm` nothing is sent\n\
     \x20         or pushed; it prints what would be, and what the prune would\n\
     \x20         remove once pushed.\n\
     \n\
     `cargo rivet install` PUBLISHES INTO THIS REPOSITORY BEFORE IT SENDS: a\n\
     crate's build, or an --rvt file, is published and the repository's copy is\n\
     what reaches the machine. `install --artifact <ID> [--at DIR]` sends an\n\
     artifact already here, selected by its full digest or a short unambiguous\n\
     prefix. `install --help` for the rest of that verb; a bare `cargo rivet\n\
     deploy` is the interactive interview that composes it.\n"
    )
}

/// # Errors
///
/// An unrecognised sub-verb, a missing value, or `publish` with no file or no
/// `--trusted-key`.
pub fn parse(args: &[String]) -> Result<FleetArgs, String> {
    let Some(sub) = args.first().map(String::as_str) else {
        return Err(format!(
            "`fleet` needs a verb: init, publish, fetch, bookmark, list, plan, drift, push, sync, record, \
             objects, phones, root, retire, hide or unhide.\n\n{}",
            usage()
        ));
    };

    if sub == "objects" {
        return parse_objects(args);
    }
    if sub == "push" || sub == "sync" || sub == "record" {
        return parse_push(args);
    }
    if sub == "phones" {
        return Ok(FleetArgs { at: None, verb: SubVerb::Phones(phones::parse(args)?) });
    }
    if sub == "root" {
        return Ok(FleetArgs { at: None, verb: SubVerb::Root(root::parse(args)?) });
    }
    if sub == "bookmark" {
        return Ok(FleetArgs { at: None, verb: SubVerb::Bookmark(bookmark::parse(args)?) });
    }
    if sub == "fetch" {
        return parse_fetch(args);
    }
    if sub == "retire" || sub == "hide" || sub == "unhide" {
        return parse_circulation(args);
    }

    let mut at: Option<PathBuf> = None;
    let mut file: Option<PathBuf> = None;
    let mut trusted_key: Option<PathBuf> = None;
    // `fleet publish FILE.apk` only — `ADR-061` §4.
    let mut product: Option<String> = None;
    let mut signing_key: Option<PathBuf> = None;
    // `fleet publish FILE --sync [confirm]` only — `ADR-061` §4.
    let mut sync = false;
    let mut sync_confirm = false;
    // `fleet publish FILE --fleet NAME [confirm]` only — `ADR-063` §6.
    let mut to_fleet: Option<String> = None;
    // `fleet plan --all` only — every other verb refuses it as an unknown flag.
    let mut all = false;
    // `fleet list --hidden` only — `ADR-052` §7.
    let mut show_hidden = false;
    // Read by `drift` and by `publish`'s own `--sync` — see the loop below.
    // `init`, `list` and a plain `publish` touch no machine, and a stray
    // `--host` there is refused exactly as any other unrecognised flag is, by
    // the `other if other.starts_with("--")` arm further down.
    let mut endpoint = rivet_remote::endpoint::Endpoint::default();

    let mut index = 1;
    while index < args.len() {
        // `drift` always speaks to a machine, and `publish --sync` does when
        // `--sync` is given, so both read `--host`, `--identity` and `--pin`
        // — the one spelling `rivet_remote::endpoint` gives every verb that
        // reaches one (`ADR-011`: one parser of that trio, not a second one
        // written out here).
        if (sub == "drift" || sub == "publish")
            && let Some(next) = endpoint.read(args, index)?
        {
            index = next;
            continue;
        }
        match args[index].as_str() {
            "--at" => {
                at = Some(PathBuf::from(
                    args.get(index + 1).ok_or("--at needs a directory")?,
                ));
                index += 2;
            }
            "--trusted-key" => {
                trusted_key = Some(PathBuf::from(
                    args.get(index + 1).ok_or("--trusted-key needs a path")?,
                ));
                index += 2;
            }
            "--all" if sub == "plan" => {
                all = true;
                index += 1;
            }
            "--hidden" if sub == "list" => {
                show_hidden = true;
                index += 1;
            }
            "--product" if sub == "publish" => {
                product = Some(args.get(index + 1).ok_or("--product needs a name")?.clone());
                index += 2;
            }
            "--signing-key" if sub == "publish" => {
                signing_key = Some(PathBuf::from(
                    args.get(index + 1).ok_or("--signing-key needs a path")?,
                ));
                index += 2;
            }
            "--sync" if sub == "publish" => {
                sync = true;
                index += 1;
            }
            "--fleet" if sub == "publish" => {
                let name = args.get(index + 1).ok_or("--fleet needs a fleet's name")?;
                bookmark::check_name(name)?;
                to_fleet = Some(name.clone());
                index += 2;
            }
            "confirm" if sub == "publish" => {
                sync_confirm = true;
                index += 1;
            }
            other if other.starts_with("--") => {
                return Err(format!("{other:?} is not an option of `fleet {sub}`\n\n{}", usage()));
            }
            other => {
                if file.is_some() {
                    return Err(format!(
                        "`fleet {sub}` takes one file and was given a second, {other:?}"
                    ));
                }
                file = Some(PathBuf::from(other));
                index += 1;
            }
        }
    }
    if to_fleet.is_some() {
        // **A publish to a fleet writes no workstation repository** (`ADR-063`
        // §6), so the flags that name one, or reach a server some other way,
        // have nothing to mean here.
        if sync || at.is_some() || endpoint.host.is_some() || endpoint.identity.is_some() || endpoint.pin.is_some() {
            return Err(
                "`--fleet` publishes to that fleet's server through its bookmark and writes no \
                 workstation repository, so --sync, --at, --host, --identity and --pin are not \
                 its: the bookmark names the server (`cargo rivet fleet bookmark list`)."
                    .to_owned(),
            );
        }
    } else if sync_confirm && !sync {
        return Err(
            "`confirm` is `--sync`'s or `--fleet`'s: `fleet publish FILE --sync confirm` sends to \
             the fleet server, and `fleet publish FILE --fleet NAME confirm` publishes to a \
             bookmarked fleet. A publish into the workstation's repository alone needs no confirm \
             (ADR-052 §4) — drop `confirm`, or add one of them."
                .to_owned(),
        );
    }
    if sync {
        endpoint.paired()?;
    }

    let verb = match sub {
        "init" => SubVerb::Init,
        "list" => SubVerb::List { show_hidden },
        "plan" => SubVerb::Plan {
            host: file
                .ok_or_else(|| {
                    format!(
                        "`fleet plan` needs a host: `cargo rivet fleet plan <device>` — the \
                         read-only dry run of what a deploy would plan there.\n\n{}",
                        usage()
                    )
                })?
                .to_string_lossy()
                .into_owned(),
            all,
        },
        "publish" => SubVerb::Publish {
            file: file.ok_or_else(|| {
                format!(
                    "`fleet publish` needs a file: `cargo rivet fleet publish FILE.rvt \
                     --trusted-key PATH` or `cargo rivet fleet publish FILE.apk`\n\n{}",
                    usage()
                )
            })?,
            trusted_key,
            product,
            signing_key,
            sync,
            sync_confirm,
            endpoint,
            fleet: to_fleet,
        },
        "drift" => {
            // `ADR-025` §2's trust is mutual, so half of it given alone is
            // refused before a socket is ever considered — the same check
            // `run` and `install` make through this same type.
            endpoint.paired()?;
            if let Some(unexpected) = file {
                return Err(format!(
                    "`fleet drift` takes no file argument and {unexpected:?} is one — it asks a \
                     machine over --host/--identity/--pin, not a local file.\n\n{}",
                    usage()
                ));
            }
            SubVerb::Drift { endpoint }
        }
        "-h" | "--help" => return Err(usage()),
        other => {
            return Err(format!(
                "`fleet {other}` is not a verb. `fleet` has init, publish, fetch, bookmark, list, \
                 plan, drift, push, sync, record, objects, phones, root, retire, hide and unhide.\n\n{}",
                usage()
            ));
        }
    };

    Ok(FleetArgs { at, verb })
}

/// [`parse`]'s own reading of `fleet push [<host>] [--host …] [--identity …]
/// [--pin …] [--at DIR] [confirm]` — a leading device name and a trailing
/// `confirm`, the grammar `run <device>` and `install … confirm` already share,
/// which the one-file loop above was not shaped for.
///
/// # Errors
///
/// An unknown flag, a flag with no value, a second device name, or one half of
/// `--identity`/`--pin` without the other.
fn parse_push(args: &[String]) -> Result<FleetArgs, String> {
    // `push` or `sync`: one grammar, and `--prune` belongs to `sync` alone.
    let sub = args.first().map_or("push", String::as_str);
    let mut at: Option<PathBuf> = None;
    let mut device: Option<String> = None;
    let mut confirm = false;
    let mut prune = false;
    let mut endpoint = rivet_remote::endpoint::Endpoint::default();

    let mut index = 1;
    while index < args.len() {
        if let Some(next) = endpoint.read(args, index)? {
            index = next;
            continue;
        }
        match args[index].as_str() {
            "-h" | "--help" => return Err(usage()),
            "--at" => {
                at = Some(PathBuf::from(args.get(index + 1).ok_or("--at needs a directory")?));
                index += 2;
            }
            "--prune" if sub == "sync" => {
                prune = true;
                index += 1;
            }
            "confirm" => {
                confirm = true;
                index += 1;
            }
            other if other.starts_with("--") => {
                return Err(format!("{other:?} is not an option of `fleet {sub}`\n\n{}", usage()));
            }
            other => {
                if let Some(first) = &device {
                    return Err(format!(
                        "`fleet {sub}` reaches one machine and was given two, {first:?} and {other:?}"
                    ));
                }
                device = Some(other.to_owned());
                index += 1;
            }
        }
    }
    endpoint.paired()?;
    let verb = match sub {
        "sync" => SubVerb::Sync { device, endpoint, prune, confirm },
        "record" => SubVerb::Record { device, endpoint, confirm },
        _ => SubVerb::Push { device, endpoint, confirm },
    };
    Ok(FleetArgs { at, verb })
}

/// [`parse`]'s own reading of `fleet fetch <fleet> [--into DIR]` — `ADR-063`
/// §6. `--into` names a clone directory other than `~/.rivet/fleets/<fleet>`.
///
/// # Errors
///
/// No fleet, a second one, a name that is not a fleet's, or an unknown flag.
fn parse_fetch(args: &[String]) -> Result<FleetArgs, String> {
    let mut fleet: Option<String> = None;
    let mut into: Option<PathBuf> = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "-h" | "--help" => return Err(usage()),
            "--into" => {
                into = Some(PathBuf::from(args.get(index + 1).ok_or("--into needs a directory")?));
                index += 2;
            }
            other if other.starts_with("--") => {
                return Err(format!("{other:?} is not an option of `fleet fetch`\n\n{}", usage()));
            }
            other => {
                if let Some(first) = &fleet {
                    return Err(format!("`fleet fetch` fetches one fleet and was given two, {first:?} and {other:?}"));
                }
                bookmark::check_name(other)?;
                fleet = Some(other.to_owned());
                index += 1;
            }
        }
    }
    let fleet = fleet.ok_or_else(|| format!("`fleet fetch` needs a fleet: `cargo rivet fleet fetch <fleet>`\n\n{}", usage()))?;
    Ok(FleetArgs { at: None, verb: SubVerb::Fetch { fleet, into } })
}

/// **Open the bookmarked fleet's server** — its bookmark, and the pinned
/// transport it names.
///
/// # Errors
///
/// No bookmark for `fleet`, or no operator identity to present.
fn open_bookmarked(fleet: &str) -> Result<(bookmark::Bookmark, rivet_remote::tls::Tls), String> {
    let found = bookmark::find(&bookmark::default_file()?, fleet)?;
    let transport = found.endpoint().open()?.ok_or_else(|| {
        format!("{fleet}'s bookmark names no operator identity this workstation can present")
    })?;
    Ok((found, transport))
}

/// [`parse`]'s own reading of `fleet retire <product> [--kind K] [--version V]
/// [--at DIR] [--sync [--host …] [--identity …] [--pin …] [confirm]]`, and of
/// `hide`/`unhide`, which take no `--version`.
///
/// # Errors
///
/// No product, a second one, an unknown flag, a flag with no value,
/// `confirm` without `--sync`, or one half of `--identity`/`--pin`.
fn parse_circulation(args: &[String]) -> Result<FleetArgs, String> {
    let sub = args.first().map_or("retire", String::as_str);
    let mut at: Option<PathBuf> = None;
    let mut product: Option<String> = None;
    let mut kind: Option<String> = None;
    let mut version: Option<String> = None;
    let mut sync = false;
    let mut sync_confirm = false;
    let mut endpoint = rivet_remote::endpoint::Endpoint::default();

    let mut index = 1;
    while index < args.len() {
        if let Some(next) = endpoint.read(args, index)? {
            index = next;
            continue;
        }
        match args[index].as_str() {
            "-h" | "--help" => return Err(usage()),
            "--at" => {
                at = Some(PathBuf::from(args.get(index + 1).ok_or("--at needs a directory")?));
                index += 2;
            }
            "--kind" => {
                kind = Some(args.get(index + 1).ok_or("--kind needs a kind")?.clone());
                index += 2;
            }
            "--version" if sub == "retire" => {
                version = Some(args.get(index + 1).ok_or("--version needs a version")?.clone());
                index += 2;
            }
            "--sync" => {
                sync = true;
                index += 1;
            }
            "confirm" => {
                sync_confirm = true;
                index += 1;
            }
            other if other.starts_with("--") => {
                return Err(format!("{other:?} is not an option of `fleet {sub}`\n\n{}", usage()));
            }
            other => {
                if let Some(first) = &product {
                    return Err(format!(
                        "`fleet {sub}` takes one product and was given two, {first:?} and {other:?}"
                    ));
                }
                product = Some(other.to_owned());
                index += 1;
            }
        }
    }
    if sync_confirm && !sync {
        return Err(format!(
            "`confirm` is `--sync`'s: `fleet {sub} <product> --sync confirm` pushes to the fleet \
             server. The change to this workstation's repository needs no confirm — drop `confirm`, \
             or add `--sync`."
        ));
    }
    endpoint.paired()?;
    let product = product.ok_or_else(|| format!("`fleet {sub}` needs a product: `cargo rivet fleet {sub} <product>`\n\n{}", usage()))?;
    let action = match sub {
        "retire" => CirculationAction::Retire { version },
        "hide" => CirculationAction::Hide,
        _ => CirculationAction::Unhide,
    };
    Ok(FleetArgs {
        at,
        verb: SubVerb::Circulation(Circulation { action, product, kind, sync, sync_confirm, endpoint }),
    })
}

/// `fleet retire | hide | unhide`, run: the local change, then — with
/// `--sync` — the push and the prune plan ([`retire::then_sync`]).
///
/// # Errors
///
/// The local change's refusal (nothing was sent), or the sync's.
fn run_circulation(root: &Path, args: &Circulation) -> Result<String, String> {
    let kind = args.kind.as_deref();
    let local = match &args.action {
        CirculationAction::Retire { version } => {
            let register = load_register()?;
            retire::retire(root, &args.product, kind, version.as_deref(), &register, &mut application_key)
        }
        CirculationAction::Hide => retire::hide(root, &args.product, kind),
        CirculationAction::Unhide => retire::unhide(root, &args.product, kind),
    }?;
    if !args.sync {
        return Ok(local);
    }
    let verb = match args.action {
        CirculationAction::Retire { .. } => "fleet retire --sync",
        CirculationAction::Hide => "fleet hide --sync",
        CirculationAction::Unhide => "fleet unhide --sync",
    };
    let server = retire::connect(&args.endpoint, verb).map_err(|why| format!("{local}\n{why}"))?;
    match retire::then_sync(root, server.as_ref(), args.sync_confirm) {
        Ok(report) => Ok(format!("{local}\n{report}")),
        Err(report) => Err(format!("{local}\n{report}")),
    }
}

/// [`parse`]'s own sub-dispatch for `fleet objects backup`/`restore` —
/// separated because it needs a SECOND word (`backup`/`restore`) before its
/// flags, which the flat `--at`/`--trusted-key`/one-positional-file loop
/// every other sub-verb shares was not shaped for, and bolting a second
/// meaning onto that loop would have made it harder to read for all four of
/// its existing callers to save one `if` here.
///
/// # Errors
///
/// A missing or unrecognised action word (`backup`/`restore`), a missing
/// directory, `restore --all` (`--all` scopes `backup` only), or an
/// unrecognised flag.
fn parse_objects(args: &[String]) -> Result<FleetArgs, String> {
    let Some(action) = args.get(1).map(String::as_str) else {
        return Err(format!(
            "`fleet objects` needs an action: backup or restore.\n\n{}",
            usage()
        ));
    };
    if action == "-h" || action == "--help" {
        return Err(usage());
    }

    let mut at: Option<PathBuf> = None;
    let mut dir: Option<PathBuf> = None;
    let mut all = false;
    let mut replace = false;

    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--at" => {
                at = Some(PathBuf::from(
                    args.get(index + 1).ok_or("--at needs a directory")?,
                ));
                index += 2;
            }
            "--all" => {
                all = true;
                index += 1;
            }
            "--replace" => {
                replace = true;
                index += 1;
            }
            other if other.starts_with("--") => {
                return Err(format!(
                    "{other:?} is not an option of `fleet objects {action}`\n\n{}",
                    usage()
                ));
            }
            other => {
                if dir.is_some() {
                    return Err(format!(
                        "`fleet objects {action}` takes one directory and was given a second, \
                         {other:?}"
                    ));
                }
                dir = Some(PathBuf::from(other));
                index += 1;
            }
        }
    }

    let verb = match action {
        "backup" => {
            let dest = dir.ok_or_else(|| {
                format!(
                    "`fleet objects backup` needs a directory: `cargo rivet fleet objects backup \
                     <dir>`\n\n{}",
                    usage()
                )
            })?;
            SubVerb::ObjectsBackup { dest, all, replace }
        }
        "restore" => {
            if all {
                return Err(
                    "`fleet objects restore` takes no --all — that flag scopes what `backup` \
                     copies OUT; a restore always copies back everything the backup directory \
                     holds."
                        .to_owned(),
                );
            }
            let source = dir.ok_or_else(|| {
                format!(
                    "`fleet objects restore` needs a directory: `cargo rivet fleet objects \
                     restore <dir>`\n\n{}",
                    usage()
                )
            })?;
            SubVerb::ObjectsRestore { source, replace }
        }
        other => {
            return Err(format!(
                "`fleet objects {other}` is not an action. `fleet objects` has backup and \
                 restore.\n\n{}",
                usage()
            ));
        }
    };

    Ok(FleetArgs { at, verb })
}

/// # Errors
///
/// Whichever sub-verb's own errors — see [`init`], [`publish`], [`list`] and
/// [`drift::run`].
pub fn run(args: &FleetArgs) -> Result<String, String> {
    let root = match &args.at {
        Some(dir) => dir.clone(),
        None => default_root()?,
    };
    match &args.verb {
        SubVerb::Init => init(&root),
        SubVerb::List { show_hidden: false } => list(&root),
        SubVerb::List { show_hidden: true } => list_showing(&root, true),
        SubVerb::Plan { host, all } => interview::plan_only(&root, host, *all),
        SubVerb::Publish { file, trusted_key, sync_confirm, fleet: Some(fleet), product, signing_key, .. } => {
            if !matches!(publish_apk::classify(file)?, publish_apk::Handed::Artifact) || product.is_some() || signing_key.is_some() {
                return Err(format!(
                    "`--fleet` publishes a signed .rvt; {} is an APK or was given an APK's flags. \
                     An APK reaches a fleet through a workstation repository's `fleet publish \
                     FILE.apk --sync` for now.",
                    file.display()
                ));
            }
            let trusted_key = trusted_key.as_ref().ok_or(
                "`fleet publish --fleet` needs --trusted-key PATH. Verification without a trust set \
                 is not verification.",
            )?;
            let (_, transport) = open_bookmarked(fleet)?;
            let clone = bookmark::clone_dir(fleet)?;
            let server = std::sync::Arc::new(transport);
            let mut trusted = || crate::key::read_32(trusted_key, "trusted public key").map(PublicKey);
            remote::publish(&remote::work_dir(&clone), &clone, &server, fleet, file, &mut trusted, *sync_confirm)
        }
        SubVerb::Bookmark(bookmark_args) => bookmark::run(bookmark_args),
        SubVerb::Fetch { fleet, into } => {
            let (_, transport) = open_bookmarked(fleet)?;
            let clone = match into {
                Some(dir) => dir.clone(),
                None => bookmark::clone_dir(fleet)?,
            };
            remote::fetch(&clone, &std::sync::Arc::new(transport), fleet)
        }
        SubVerb::Publish { file, trusted_key, product, signing_key, sync, sync_confirm, endpoint, fleet: None } => {
            let published = match publish_apk::classify(file)? {
                publish_apk::Handed::Apk => {
                    if trusted_key.is_some() {
                        return Err(format!(
                            "--trusted-key is an .rvt's: it names the key a signed artifact is \
                             verified against. {} is an APK, which this command wraps and signs \
                             itself with the application seed (ADR-061 §4) — name that seed with \
                             --signing-key PATH if it is not in the usual place.",
                            file.display()
                        ));
                    }
                    let mut seed = || {
                        publish_apk::app_seed(
                            signing_key.as_deref(),
                            &crate::credentials::Overrides::from_environment(),
                            crate::credentials::root().as_deref(),
                        )
                    };
                    publish_apk::publish(&root, file, product.as_deref(), &mut seed)
                }
                publish_apk::Handed::Artifact => {
                    if product.is_some() || signing_key.is_some() {
                        return Err(format!(
                            "--product and --signing-key are an .apk's (ADR-061 §4). {} is \
                             published as the signed artifact it already is, under the product \
                             its own header names; it takes --trusted-key PATH.",
                            file.display()
                        ));
                    }
                    let trusted_key = trusted_key.as_ref().ok_or(
                        "`fleet publish` needs --trusted-key PATH. Verification without a trust \
                         set is not verification.",
                    )?;
                    publish(&root, file, trusted_key)
                }
            }?;
            if !sync {
                return Ok(published);
            }
            match sync_to_fleet_server(&root, endpoint, *sync_confirm) {
                Ok(report) => Ok(format!("{published}\n{report}")),
                Err(report) => Err(format!("{published}\n{report}")),
            }
        }
        SubVerb::Drift { endpoint } => drift::run(&root, endpoint),
        SubVerb::Push { device, endpoint, confirm } => push::run(&root, device.as_deref(), endpoint, *confirm),
        SubVerb::Record { device, endpoint, confirm } => record::run(&root, device.as_deref(), endpoint, *confirm),
        SubVerb::Sync { device, endpoint, prune, confirm } => {
            sync::run(&root, device.as_deref(), endpoint, *prune, *confirm)
        }
        SubVerb::ObjectsBackup { dest, all, replace } => objects_backup(&root, dest, *all, *replace),
        SubVerb::ObjectsRestore { source, replace } => objects_restore(&root, source, *replace),
        SubVerb::Phones(phones_args) => phones::run(phones_args),
        SubVerb::Root(root_args) => root::run(root_args),
        SubVerb::Circulation(circulation) => run_circulation(&root, circulation),
    }
}

// ---------------------------------------------------------------------------
// `fleet publish FILE --sync [confirm]` — ADR-061 §4, `.rulings/` R-222
// ---------------------------------------------------------------------------

/// **The fleet server is found, never named.** The one register record marked
/// as the fleet's server, by either of the two markings a register carries:
/// `fleet_role = "server"`, which the workstation's own register writes, or
/// `role = "fleet-server"` (`rivet_core::fleet_as_code::SERVER_ROLE`), which
/// the server's copy carries and `rivet-fleetd` finds itself by
/// (`cmd/rivet-fleetd/src/repo.rs`'s `this_server`). A record carrying both
/// is one server. Pure over an already-loaded [`rivetos_register::Register`]
/// so a test can build one in memory, with no register file at all.
///
/// # Errors
///
/// The register holds no record so marked, or more than one — either named,
/// so nothing is guessed between.
fn resolve_fleet_server(register: &rivetos_register::Register) -> Result<rivetos_register::Device, String> {
    let role = rivet_core::fleet_as_code::SERVER_ROLE;
    let servers: Vec<&rivetos_register::Device> = register
        .devices
        .values()
        .filter(|d| {
            d.fleet_role == Some(rivetos_register::FleetRole::Server) || d.role.as_deref() == Some(role)
        })
        .collect();
    match servers.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(format!(
            "--sync needs a fleet server, and this register holds no record marked as one \
             (fleet_role = \"server\", or role = {role:?}), so there is nothing to send to. \
             Provisioning marks a fleet server's record when it enrolls it (ADR-034). Nothing \
             was sent."
        )),
        several => {
            let mut names: Vec<&str> = several.iter().map(|d| d.name.as_str()).collect();
            names.sort_unstable();
            Err(format!(
                "--sync needs ONE fleet server, and this register holds {} records marked as one \
                 ({}); this will not choose which of them is the fleet's own. Nothing was sent.",
                several.len(),
                names.join(", "),
            ))
        }
    }
}

/// **`--sync`'s composition: files, then the list, and never the other
/// order.** Send every object the manifest names that `server` lacks
/// ([`sync::sync`]), then push `main` ([`push::push`], asked NOT to refuse
/// over uncommitted files: they never travel either way, and `--sync` names
/// them rather than blocking on them). A pushed list naming a file the
/// server does not hold leaves the server refusing to prune until it
/// arrives (`ADR-052` §7), so a send that fails means `main` is never
/// pushed at all — `sync::sync`'s own `?` is what stops it here.
///
/// Generic over one object that is both trait's server, so a test can supply
/// a single in-memory fake for the whole composition, exactly as the real
/// run supplies one real `rivet_remote::tls::Tls` to both halves.
///
/// # Errors
///
/// [`sync::sync`]'s (nothing was pushed) or [`push::push`]'s.
fn sync_then_push<T>(root: &Path, server: std::sync::Arc<T>, label: &str, confirm: bool) -> Result<String, String>
where
    T: sync::ObjectStore + push::Upstream + 'static,
{
    let objects_report = sync::sync(root, server.as_ref(), label, false, confirm)?;
    // **Both branches carry `objects_report`.** A push that fails after a
    // complete send should never read as though nothing happened — the
    // objects went, and the report says so either way.
    match push::push(root, server, label, confirm, false) {
        Ok(push_report) => Ok(format!("{objects_report}\n{push_report}")),
        Err(push_report) => Err(format!("{objects_report}\n{push_report}")),
    }
}

/// **`--sync`: after a publish has landed in this workstation's repository,
/// find the fleet's own server ([`resolve_fleet_server`]) and run
/// [`sync_then_push`] against it.** `given`'s `--host`/`--identity`/`--pin`
/// win over what the register resolves, exactly as they do for `fleet
/// push`/`fleet sync`.
///
/// # Errors
///
/// The register cannot be read; [`resolve_fleet_server`]'s; the resolved
/// endpoint has no operator identity to present; or [`sync_then_push`]'s.
fn sync_to_fleet_server(
    root: &Path,
    given: &rivet_remote::endpoint::Endpoint,
    confirm: bool,
) -> Result<String, String> {
    let (name, transport) = open_fleet_server(given, "fleet publish --sync")?;
    sync_then_push(root, std::sync::Arc::new(transport), &name, confirm)
}

/// **The fleet's own server, found in the register and reached over the
/// pinned control plane** — its register name and the open transport.
/// `verb` names the command in a refusal.
///
/// # Errors
///
/// The register cannot be read; [`resolve_fleet_server`]'s; the resolved
/// endpoint has no operator identity to present.
fn open_fleet_server(
    given: &rivet_remote::endpoint::Endpoint,
    verb: &str,
) -> Result<(String, rivet_remote::tls::Tls), String> {
    given.paired()?;
    let register_root = crate::device::default_register_dir()?;
    let register = rivetos_register::store::load(&register_root).map_err(|e| {
        format!("could not read the register at {}: {e}. Nothing was sent.", register_root.display())
    })?;
    let device = resolve_fleet_server(&register)?;
    let name = device.name.clone();

    let endpoint = push::resolve_endpoint(Some(&name), given, &|| Ok(register_root.clone()))
        .map_err(|e| e.replace("`fleet push`", &format!("`{verb}`")))?;
    let transport = endpoint.open()?.ok_or_else(|| {
        format!(
            "no operator identity to present: `{verb}` speaks ADR-035's control plane and has no \
             second transport. `cargo rivet identity new` mints one; --identity DIR names it."
        )
    })?;
    Ok((name, transport))
}

/// The workstation's register, where `rivet-fleet` and provisioning keep it
/// — read by `fleet retire` for the pins and carries it refuses on.
///
/// # Errors
///
/// The register's directory cannot be resolved or read.
fn load_register() -> Result<rivetos_register::Register, String> {
    let register_root = crate::device::default_register_dir()?;
    rivetos_register::store::load(&register_root).map_err(|e| {
        format!(
            "could not read the register at {}: {e}. Whether a machine pins what would be retired \
             cannot be known, so nothing was changed.",
            register_root.display()
        )
    })
}

/// The application key an `apk` product's held versions verify against — the
/// public half of the application seed, found as `fleet publish FILE.apk`
/// finds it.
///
/// # Errors
///
/// No seed is found, or it does not read.
fn application_key() -> Result<PublicKey, String> {
    let seed = publish_apk::app_seed(
        None,
        &crate::credentials::Overrides::from_environment(),
        crate::credentials::root().as_deref(),
    )
    .map_err(|_| {
        format!(
            "no application signing seed was found (${}, then the default path `fleet publish \
             FILE.apk` uses), so the held versions cannot be verified to read their signer",
            crate::APP_SEED_ENV
        )
    })?;
    let bytes = crate::key::read_32(&seed, "application signing seed")?;
    Ok(rivet_crypto::Ed25519Signer::from_seed(&bytes).public_key())
}

// ---------------------------------------------------------------------------
// objects backup / restore — ADR-052, R-183 D4
// ---------------------------------------------------------------------------

/// **Refuse a `root` that is not a fleet repository at all**, before either
/// direction reads or writes anything — the identical check [`publish_verified`]
/// already makes for its own reason, restated here because `objects backup`/
/// `restore` never call it (neither touches git).
fn require_repository(root: &Path) -> Result<(), String> {
    if !root.join(".git").is_dir() {
        return Err(format!(
            "{} is not a fleet repository yet. `cargo rivet fleet init --at {}` makes one \
             (ADR-052 §2). Nothing was read.",
            root.display(),
            root.display(),
        ));
    }
    Ok(())
}

/// `cargo rivet fleet objects backup <dest> [--all] [--replace]`.
///
/// **Default scope: only the digests `manifest.toml` currently references**
/// — `ADR-052`'s sizing rule for a member's own store (*"only the artifacts
/// its own documents name"*) applied here to a backup instead. `--all`
/// copies every digest `objects/` holds, `-dirty-` builds and every
/// superseded version included — the escape hatch for someone who wants the
/// whole historical tree rather than what would let this fleet recover as
/// it stands today.
///
/// **An empty scope is reported, not refused.** A freshly initialised
/// repository with nothing published, or `--all` against an `objects/`
/// holding no files, is a legitimate state — the same posture `fleet list`
/// already takes for an empty manifest — so this never turns "there was
/// nothing to back up" into a failed command the way an empty CA or an
/// empty seed directory correctly does (those are secrets that SHOULD be
/// there; an empty fleet repository is not a mistake).
///
/// # Errors
///
/// [`require_repository`]'s, [`rivet_filecopy::Error::WouldOverwrite`] rendered as a report
/// (naming what is already at `dest`, with `--replace` the only override),
/// or whatever the filesystem said.
fn objects_backup(root: &Path, dest: &Path, all: bool, replace: bool) -> Result<String, String> {
    require_repository(root)?;
    let objects_source = root.join(OBJECTS_DIR);
    let objects_dest = dest.join(OBJECTS_DIR);

    let (copied, scope_note) = if all {
        match rivet_filecopy::copy_all(&objects_source, &objects_dest, replace) {
            Ok(copied) => (copied, "--all"),
            Err(rivet_filecopy::Error::Empty(_)) => {
                return Ok(format!(
                    "{} holds no objects at all — 0 objects backed up (--all).\n",
                    objects_source.display()
                ));
            }
            Err(other) => return Err(objects_copy_refusal(&other, &objects_dest)),
        }
    } else {
        let entries = read_manifest(root)?;
        let mut names: Vec<String> = entries.iter().map(|e| e.digest.clone()).collect();
        names.sort();
        names.dedup();
        match rivet_filecopy::copy_named(&objects_source, &objects_dest, &names, replace) {
            Ok(copied) => (copied, "referenced"),
            Err(rivet_filecopy::Error::Missing(missing)) => {
                return Err(format!(
                    "the manifest at {} names {} digest(s) whose bytes are not in {} — the \
                     repository is inconsistent, and nothing was backed up:\n{}",
                    root.join(MANIFEST_FILE).display(),
                    missing.len(),
                    objects_source.display(),
                    missing.iter().map(|d| format!("  {d}\n")).collect::<String>(),
                ));
            }
            Err(other) => return Err(objects_copy_refusal(&other, &objects_dest)),
        }
    };

    let total_bytes: u64 = copied.iter().map(|c| c.bytes).sum();
    let mut report = format!(
        "  backed up {} object(s) from {} to {} ({scope_note}, {total_bytes} bytes total)\n",
        copied.len(),
        objects_source.display(),
        objects_dest.display(),
    );
    for c in &copied {
        report.push_str(&format!("      {:<66} {:>12} bytes\n", c.name, c.bytes));
    }
    Ok(report)
}

/// Every regular file directly inside `dir`, unsorted.
///
/// **Every entry is accounted for, one way or the other.** A directory-entry
/// read failure or a metadata read failure is reported rather than dropped:
/// treating an entry this could not even stat as "not a file" is how a
/// restore would run to completion short one object and call it done —
/// `rivet_filecopy`'s own `flat_files` closed the identical defect there for
/// I01, and this is that shape, followed here because this enumeration is
/// not that crate's.
///
/// # Errors
///
/// A directory that cannot be listed, or an entry inside it that cannot be
/// read or stat'ed, each naming the path.
fn flat_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        let is_file = std::fs::metadata(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .is_file();
        if is_file {
            files.push(path);
        }
    }
    Ok(files)
}

/// `cargo rivet fleet objects restore <source> [--replace]`.
///
/// **Recomputes every object's own BLAKE3 before anything is copied**, and
/// refuses — naming every mismatch, nothing written — if any backed-up
/// object's filename does not match the digest of its own bytes. `ADR-052`
/// §7.2.3's own door check, run here on the workstation rather than trusted
/// from the backup blindly: a restored repository is never worse-verified
/// than one freshly received.
///
/// # Errors
///
/// [`require_repository`]'s; [`flat_files`]'s, for an entry in `source` that
/// cannot be listed or stat'ed; the digest-mismatch refusal above; whatever
/// [`rivet_filecopy::copy_all`] refuses on (an empty backup, or an existing
/// conflicting object at `root`'s own `objects/` without `--replace`).
fn objects_restore(root: &Path, source: &Path, replace: bool) -> Result<String, String> {
    require_repository(root)?;
    let backup_objects = source.join(OBJECTS_DIR);
    let mut files = flat_files(&backup_objects)?;
    files.sort();
    if files.is_empty() {
        return Err(format!("{} holds no objects to restore", backup_objects.display()));
    }

    let mut mismatches: Vec<String> = Vec::new();
    for file in &files {
        let (digest, _size) = blake3_of(file)?;
        let hex_digest = crate::key::hex(&digest);
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name != hex_digest {
            mismatches.push(format!("{name} — actual content digest is {hex_digest}"));
        }
    }
    if !mismatches.is_empty() {
        mismatches.sort();
        return Err(format!(
            "{} object(s) in {} do not match their own filename's digest — refused, and \
             NOTHING WAS RESTORED:\n{}",
            mismatches.len(),
            backup_objects.display(),
            mismatches.iter().map(|m| format!("  {m}\n")).collect::<String>(),
        ));
    }

    let objects_dest = root.join(OBJECTS_DIR);
    let copied = rivet_filecopy::copy_all(&backup_objects, &objects_dest, replace)
        .map_err(|e| objects_copy_refusal(&e, &objects_dest))?;

    let total_bytes: u64 = copied.iter().map(|c| c.bytes).sum();
    let mut report = format!(
        "  restored {} object(s) from {} to {} ({total_bytes} bytes total), every digest \
         recomputed and verified against its own filename\n",
        copied.len(),
        backup_objects.display(),
        objects_dest.display(),
    );
    for c in &copied {
        report.push_str(&format!("      {:<66} {:>12} bytes\n", c.name, c.bytes));
    }
    Ok(report)
}

/// Render whatever [`rivet_filecopy::copy_all`]/`copy_named` refused on, for
/// a caller that already handled [`rivet_filecopy::Error::Empty`] (backup's
/// two scopes each word that differently) and [`rivet_filecopy::Error::Missing`]
/// (backup's referenced scope alone — restore's own digest check catches the
/// equivalent case before this is ever reached).
fn objects_copy_refusal(error: &rivet_filecopy::Error, dest: &Path) -> String {
    match error {
        rivet_filecopy::Error::Empty(dir) => format!("{} holds nothing to copy", dir.display()),
        rivet_filecopy::Error::WouldOverwrite(existing) => {
            let mut report =
                format!("{} already holds object(s) this would replace. NOT overwritten.\n", dest.display());
            for e in existing {
                report.push_str(&format!(
                    "      {:<66} {:>12} bytes  {}\n",
                    e.name,
                    e.bytes,
                    rivet_filecopy::age_text(e.age)
                ));
            }
            report.push_str("  pass --replace to replace them, or choose a different directory.\n");
            report
        }
        rivet_filecopy::Error::Missing(names) => format!(
            "{} named object(s) are not present to copy: {}",
            names.len(),
            names.join(", ")
        ),
        // A real outcome, not a defensive stub: `<directory>` naming the
        // fleet repository's own `objects/`, or a location inside it, is a
        // plausible typo — and before B05's fix in `rivet-filecopy`, this
        // reached a same-file `std::fs::copy` unguarded and truncated a
        // published artifact while reporting success.
        rivet_filecopy::Error::SameFile(paths) => format!(
            "{} names the same object(s) this would copy — not a backup or a restore:\n{}",
            dest.display(),
            paths.iter().map(|p| format!("      {}\n", p.display())).collect::<String>(),
        ),
        rivet_filecopy::Error::Io(why) => why.clone(),
    }
}

#[cfg(test)]
#[path = "fleet_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "fleet_hidden_tests.rs"]
mod hidden_tests;

#[cfg(test)]
#[path = "fleet_publish_sync_tests.rs"]
mod publish_sync_tests;
