//! **Where an application's files go, and in what order.** `ADR-015` §7.
//!
//! > *"Unpack to `/state/app/<name>/<version>/`, verify the signature, then swap
//! > a symlink — a `rename`, not a copy. This matters more than it sounds: a Pi
//! > has no battery, is usually on an SD card, and gets its power pulled. A
//! > half-installed application is an ordinary failure, not a theoretical one."*
//!
//! This module is the **plan** and performs no I/O. A [`Plan`] is an ordered
//! list of [`Step`]s that some binary carries out, which is `ADR-014` §2's split
//! and also `ADR-006` §6.2's — *"every mutating operation supports a dry run"* —
//! reached by construction rather than by a flag that skips writes. A dry run
//! renders the plan; a real run performs it; there is one description of what
//! would happen and it is the same object.
//!
//! # The name comes from the header, and that is why the header grew
//!
//! `ADR-015` §3.2's manifest **deliberately carries no `name`**, so that a
//! machine cannot hold two answers to what an application is called, and
//! `cmd/rivet-sysd` observes the name from the directory. But an
//! installer has no directory yet — it is the thing deciding which directory —
//! so it takes the name from the container header's `product` field.
//!
//! That is the field `ADR-029` §3.1 grew from 16 bytes to 32 on 2026-08-24,
//! because `metrics-collector` is 17 characters and `ADR-015` §5.4's own example
//! could not be packaged. The reason to grow it *then* was that no artifact had
//! ever been verified on a Rivet; this module is what would have made that
//! expensive.
//!
//! # What this refuses, and where the line actually falls
//!
//! **Every name in a package is attacker-chosen.** The header's `product` and
//! `version` and every member's name are bytes in a file somebody else wrote,
//! and this module turns them into *paths on the machine*.
//!
//! It is not the first thing between them, and saying so accurately matters
//! more than claiming credit for the whole defence:
//!
//! | | refused by | what it catches |
//! | :-- | :-- | :-- |
//! | member names | `format::read_name` | `/`, a backslash, `.`, `..`, empty, anything not printable ASCII — **before a signature is ever computed** |
//! | header text | `format::read_text` | non-printable ASCII, and an empty `version` |
//! | the name | [`ServiceName`] | a leading dot or dash, spaces, anything outside `[A-Za-z0-9._-]` — because a name is a cgroup and a socket as well as a directory (`ADR-017` §3.1.3) |
//! | **the rest** | **`safe_component`, here** | a `/` in a *version* (which `read_text` allows), and any name that already **means** something under an application's root |
//!
//! That last row is this module's own contribution and it is the least obvious
//! one. `current`, `data` and [`STAGED_LINK`] are all perfectly well-formed
//! names. A version called `data` would put a directory where everything the
//! application has ever written lives; a version called `current` would replace
//! the link that selects a version with a directory. Neither is a traversal and
//! both are worse than one. [`RESERVED_IN_APPLICATION_DIR`] is the whole set a
//! version is checked against, `tmp` and the supervisor's own files included.
//!
//! `the_container_refuses_a_traversal_before_this_module_ever_sees_it` asserts
//! the first row from here, so that a regression in the format fails a test in
//! the module that leans on it.
//!
//! A refusal here happens **after** the signature verified, and that is the
//! point rather than a redundancy: a signed artifact is one somebody vouched
//! for, not one that is well-formed, and `ADR-028` §2 makes this the code where
//! the difference matters.
//!
//! # What it does not do
//!
//! **It removes nothing.** Older version directories stay where they are, which
//! is what makes going back a symlink swap rather than a reinstall. Nothing
//! reaps them yet and `ADR-015` §7 does not say when to — an application's
//! second install therefore costs disk, and that is a named consequence rather
//! than an oversight.
//!
//! **It does not touch `data`.** `rivet_core::service::data_dir` is a sibling
//! of the versions rather than a child of `current` precisely so that installing
//! a new version replaces the program and leaves the data. The plan never names
//! it, which is stronger than being careful about it.
//!
//! **It does not decide whether the application should run.** That is the
//! desired-state document's `enabled` (`ADR-015` §3.2.1) and the supervisor's.

use crate::package::application::{Application, MANIFEST_NAME};
use crate::package::error::PackageError;
use crate::package::kind::Kind;
use crate::package::library;
use crate::package::read::Package;
use crate::service::{self, BIN_DIR, CURRENT_LINK, DATA_DIR, ServiceName};

/// The mode a version directory and its `bin` are created with.
///
/// `0755` rather than something tighter, and it is the identity that decides it:
/// `cmd/rivet-sysd` drops to the application's own uid **between `fork`
/// and `exec`** (`ADR-015` §5.1), so the `execve` happens as that uid and needs
/// search permission on every directory of the path and execute on the file. A
/// root-owned `0700` tree would install perfectly and start nothing.
pub const DIR_MODE: u32 = 0o755;

/// The retained signed header, beside the tree it describes.
///
/// **A fourth name in a version directory** — `current`, `bin`,
/// `manifest.toml` and now this; `PLAN.md` §0.4.3(d) asked that they all be
/// written down and this is the one added since. No extension, because it is
/// opaque: it is 4096 bytes of `ADR-029` §3's header, not a document anybody
/// edits or a format anything else reads.
///
/// It is `FILE_MODE` — `0444`, root-owned like the rest of the version tree —
/// so the identity that runs the program cannot rewrite the thing that
/// authenticates it. That is not the main defence (an attacker with root can
/// write it either way); the main defence is that rewriting it without the
/// signing key produces a header that fails to verify.
pub const HEADER_NAME: &str = "package-header";

/// **The recorded size, beside the header.** `R34`, `.rulings/` R-163.
///
/// A fifth name in a version directory. Holds the decimal count of bytes
/// [`Step::Size`] summed while writing the version's own members and its
/// header — the *installed* size, not what the application's data directory
/// grows to afterward (`.rulings/` R-163.1 says why those are two different
/// numbers and why only this one is recorded).
///
/// No extension, for [`HEADER_NAME`]'s reason: nothing but this installer
/// reads it, and there is no format to name.
pub const SIZE_NAME: &str = "install-size";

/// Every name this installer writes directly into an application's version
/// directory on its own, whether or not the package carries any asset at
/// all: [`BIN_DIR`], [`MANIFEST_NAME`], [`HEADER_NAME`] and [`SIZE_NAME`].
///
/// An asset sharing one of these would not sit beside the file it is named
/// for — it would land on top of it, wherever [`plan`] happens to place that
/// asset's own [`Step::Member`] relative to the step that writes the real
/// one. One list, exposed here rather than restated by whatever packages an
/// application, so the two cannot come to check against different names.
pub const RESERVED_APPLICATION_ASSET_NAMES: [&str; 4] =
    [BIN_DIR, MANIFEST_NAME, HEADER_NAME, SIZE_NAME];

/// Every name this installer writes directly into a library's directory on
/// its own: [`library::OBJECT_DIR`], [`library::MANIFEST_NAME`] and
/// [`HEADER_NAME`]. The library equivalent of
/// [`RESERVED_APPLICATION_ASSET_NAMES`], for the same reason.
pub const RESERVED_LIBRARY_ASSET_NAMES: [&str; 3] =
    [library::OBJECT_DIR, library::MANIFEST_NAME, HEADER_NAME];

/// The mode the program is written with. Executable by the uid that will run it,
/// writable by nobody — including that uid, which is `ADR-015` §4.2's argument
/// in miniature: an application that can rewrite its own binary makes
/// launch-time verification a formality.
pub const PROGRAM_MODE: u32 = 0o555;

/// The mode the manifest and any asset are written with. Read-only for the same
/// reason.
pub const FILE_MODE: u32 = 0o444;

/// The temporary name the new `current` link is made under before it is renamed
/// into place.
///
/// A leading dot so it cannot be a [`ServiceName`] and therefore cannot collide
/// with a version directory — `ServiceName::new` refuses a name whose first
/// character is not alphanumeric, so no legitimate version can be called this.
pub const STAGED_LINK: &str = ".current.new";

/// One thing an installer does, in the order it does it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Create a directory if it is not there. Idempotent by requirement:
    /// `ADR-006` §6.2 makes every operation re-runnable, and an install that
    /// failed halfway is exactly the case where it is re-run.
    Directory { path: String, mode: u32 },
    /// Write the package's own **signed header** beside the installed tree.
    ///
    /// `ADR-015` §4.2's launch-time verification needs something signed to
    /// check against, and the `.rvt` is not kept — for a system update it is
    /// 683 MB. This is the 4 KiB that carries the signature, the key id and
    /// every member's digest, so re-reading it at launch re-establishes the
    /// whole chain against the trust store **as it is at that moment**.
    ///
    /// **That "at that moment" is the feature**, ruled 2026-08-25: `ADR-028`
    /// §5.5.4 makes revocation an image update, and an image update replaces
    /// the trust store. Without this file a revoked key stops future installs
    /// and does nothing about what it already put on the machine.
    ///
    /// It carries no `index` because it is not a member — it is the thing the
    /// members are described *by*.
    Header { path: String, mode: u32 },
    /// Write one member of the package to a path.
    ///
    /// `index` is the member's position in the package. The executor reads it,
    /// checks its digest against the header, and only then writes — so a member
    /// that does not match is never on the filesystem at all, rather than being
    /// written and deleted.
    Member {
        index: usize,
        path: String,
        mode: u32,
    },
    /// **Write the recorded size of this install, beside the header.**
    /// `R34`, `.rulings/` R-163.
    ///
    /// No `bytes` field: the executor already sums every [`Step::Member`] and
    /// [`Step::Header`] it writes before it, so this step carries only where
    /// the total goes — carrying a precomputed number here would be a second
    /// place the byte count could be wrong, when the executor's own write
    /// loop is already an exhaustive walk of what the version directory
    /// holds.
    ///
    /// **Before [`Step::Activate`] and after everything it counts**, the same
    /// placement [`Step::Header`] argues for: a version directory must never
    /// be live at `current` without every fact about it already on disk.
    Size { path: String },
    /// **The atomic step.** Make `staged` a symlink to `target`, then rename it
    /// over `link`.
    ///
    /// `rename(2)` on the same filesystem is atomic, so a reader either sees the
    /// old version or the new one and never neither — which is the whole of
    /// `ADR-015` §7's argument about a Pi whose power gets pulled. `target` is
    /// **relative**, so the link keeps working if the tree is ever moved and so
    /// that nothing on the machine has to agree about an absolute prefix.
    Activate {
        link: String,
        staged: String,
        target: String,
    },
}

/// Everything that will happen, and nothing that has happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    name: ServiceName,
    version: String,
    app_dir: String,
    version_dir: String,
    steps: Vec<Step>,
}

impl Plan {
    /// The application's name, as the header gave it and as the directory will
    /// spell it.
    #[must_use]
    pub fn name(&self) -> &ServiceName {
        &self.name
    }

    /// The version, as the header gave it. Never parsed and never compared:
    /// `ADR-029` §12 item 7 records that nothing in this project knows how to
    /// order two version strings, and a wrong ordering would report a downgrade
    /// as an upgrade.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// `/state/app/<name>`.
    #[must_use]
    pub fn app_dir(&self) -> &str {
        &self.app_dir
    }

    /// `/state/app/<name>/<version>` — where this install's files land.
    #[must_use]
    pub fn version_dir(&self) -> &str {
        &self.version_dir
    }

    /// The steps, in order. Order is part of the plan: every directory is made
    /// before anything is written into it, and the link is swapped **last**, so
    /// a plan abandoned at any point leaves `current` pointing at whatever it
    /// pointed at before.
    #[must_use]
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// **Nothing is ever unpacked over a version directory that is already
    /// there.** `ADR-015` §7.2, ruled 2026-08-29. `None` to proceed; `Some` is
    /// the refusal, in the words an operator reads.
    ///
    /// # The defect
    ///
    /// [`Step::Directory`]'s executor is convergent — `Kernel::mkdir_mode`
    /// succeeds when the directory is already there, deliberately, so that an
    /// install which died halfway can be re-run. Combined with [`Self::version_dir`]
    /// being a **function of the version**, that means installing a payload whose
    /// version matches one already on the machine writes its files **on top of
    /// the directory `current` may be pointing at right now** — the program the
    /// supervisor execs, the manifest it reads, and the signed header
    /// `ADR-015` §4.2 re-verifies at every launch.
    ///
    /// Two things break at once and neither is visible:
    ///
    /// 1. **§7's atomicity is gone.** The whole argument is that a plan abandoned
    ///    anywhere leaves an unreferenced directory and a `current` that still
    ///    points where it did. A plan that writes *into* the live directory
    ///    leaves `current` pointing at a half-replaced install instead.
    /// 2. **A running program is being overwritten.** On Linux `open(O_TRUNC)`
    ///    over a busy executable answers `ETXTBSY`, so the failure lands after
    ///    the manifest and the header have already been replaced — an
    ///    application whose header no longer describes its program.
    ///
    /// It was reached in practice on 2026-08-29 (`PLAN.md` §102.6): a `-dirty`
    /// version was not unique, so two different builds of one working tree
    /// wanted one directory. `ADR-027` §3.2.2 fixed the naming half; this is the
    /// half that does not depend on a name being right.
    ///
    /// # Why refusing, rather than staging and swapping
    ///
    /// Because there is no atomic swap of a **directory** to be had.
    /// `rename(2)` over a non-empty directory answers `ENOTEMPTY`, so
    /// "stage then swap" would be two renames with a window between them — and a
    /// window is exactly what §7 spent the `current` symlink to avoid. The
    /// symlink is already the atomic switch, and it switches between
    /// *version directories*, so the correct shape is that a version directory is
    /// written **once** and never again.
    ///
    /// **What this costs, stated rather than discovered:** re-running an install
    /// of a version already on the machine is now a refusal rather than a no-op,
    /// which is a narrowing of `ADR-006` §6.2's re-runnability. It is accepted
    /// because the alternative is writing over files something may be executing,
    /// and because the refusal names the two ways forward — the bytes changed, so
    /// the version changes with them (`ADR-027` §3.2.2); or they did not, and the
    /// application is already installed.
    #[must_use]
    pub fn occupied(&self, version_dir_exists: bool) -> Option<String> {
        if !version_dir_exists {
            return None;
        }
        Some(format!(
            "REFUSED: {} is already on this machine, and nothing is ever unpacked over a \
             directory that is already there (ADR-015 §7.2). The supervisor may be executing the \
             program inside it right now — `current` selects a version directory and this is one \
             — so writing into it would replace a running application's files a few at a time, \
             which is the opposite of the atomic install ADR-015 §7 exists for. NOTHING WAS \
             WRITTEN. If the bytes changed, the version changes with them: since ADR-027 §3.2.2 a \
             build from a dirty tree carries a digest of its whole payload - the program, the \
             manifest and every asset - so rebuild and package again. If they did not change, \
             `{}` version `{}` is already installed and there is \
             nothing to do. To replace this exact version deliberately, uninstall it first.",
            self.version_dir,
            self.name.as_str(),
            self.version,
        ))
    }
}

/// Build the plan for installing a verified application package **onto a
/// volume**.
///
/// `package` must already have been through `crate::package::read::open` — its
/// signature checked against a trusted key that is permitted to sign this kind.
/// Nothing here re-checks that, and nothing here could: this module sees a
/// `Package`, which is the type that only exists once verification has passed.
///
/// **`volume` is a root directory, not a name** — `service::BUILT_IN_VOLUME` for
/// `/state`, a mount point for a declared volume (`ADR-031` §8 item 1). Which
/// volume an application is installed onto is the caller's decision and there is
/// nothing in this crate that could make it: it needs the attached set, which is
/// I/O. Every caller passes the built-in volume today.
pub fn plan(volume: &str, package: &Package) -> Result<Plan, PackageError> {
    let header = package.header();
    if *header.kind() != Kind::application() {
        return Err(PackageError::WrongKindForInstall {
            kind: header.kind().as_str().to_owned(),
            wanted: Kind::application().as_str().to_owned(),
        });
    }

    let application = Application::open(package)?;

    let name = ServiceName::new(header.product()).ok_or_else(|| PackageError::UnusableName {
        product: header.product().to_owned(),
    })?;
    let version =
        safe_version(header.version()).ok_or_else(|| PackageError::UnusableVersion {
            version: header.version().to_owned(),
        })?;

    let app_dir = service::app_dir(volume, &name);
    let version_dir = format!("{app_dir}/{version}");
    let bin_dir = format!("{version_dir}/{BIN_DIR}");

    let mut steps = vec![
        // **`/state/app` itself, and its absence cost the first install on real
        // hardware — 2026-08-25.** The plan began at `app_dir` and the port
        // deliberately does not create parents: `Kernel::mkdir_mode` says so in
        // as many words, on the argument that "a missing parent means the plan
        // is wrong and that should be a refusal rather than a `mkdir -p`
        // papering over it". It was right, and the plan was the thing that was
        // wrong. `ADR-015` §4.2 describes this directory; nothing created it,
        // which `PLAN.md` §0.11 had already noticed and filed as the installer's
        // business.
        //
        // The board's refusal was exactly the one the port promised —
        // `cannot create /state/app/growfix: No such file or directory` —
        // which is worth recording as the design working: it failed loudly, at
        // the first step, having written nothing.
        //
        // **`DIR_MODE` and not `ADR-015` §4.2's `0700`.** §4.2 says this
        // directory is "root-owned and mode 0700", and `PLAN.md` §0.11 already
        // established that cannot stand: a program is resolved by the
        // application's OWN uid, so every directory from `/` down must be
        // searchable by it, and `0700` makes the binary unreachable by the only
        // identity permitted to execute it. Nothing secret lives at this level
        // — the private thing is `data`, which is [`service::DATA_DIR_MODE`]'s
        // `0700` and stays that way.
        Step::Directory {
            path: service::app_root(volume),
            mode: DIR_MODE,
        },
        Step::Directory {
            path: app_dir.clone(),
            mode: DIR_MODE,
        },
        Step::Directory {
            path: version_dir.clone(),
            mode: DIR_MODE,
        },
        Step::Directory {
            path: bin_dir.clone(),
            mode: DIR_MODE,
        },
        // The program is named after the APPLICATION, not after the member.
        // `rivet_core::service::program_path` derives what the supervisor
        // will execute from the name alone (`ADR-015` §3.2 — there is no `exec`
        // field), so the two cannot disagree, and a member name has no say in
        // where the program lands.
        Step::Member {
            index: application.program_member().index(),
            path: format!("{bin_dir}/{name}"),
            mode: PROGRAM_MODE,
        },
        Step::Member {
            index: application.manifest_member().index(),
            path: format!("{version_dir}/{MANIFEST_NAME}"),
            mode: FILE_MODE,
        },
        // **Before the activation and after everything it describes**, so a
        // version directory never has a header without the files it covers.
        // `ADR-015` §4.2, ruled 2026-08-25.
        Step::Header {
            path: format!("{version_dir}/{HEADER_NAME}"),
            mode: FILE_MODE,
        },
    ];

    for asset in application.assets() {
        let asset_name =
            safe_component(asset.name()).ok_or_else(|| PackageError::UnsafeMemberName {
                name: asset.name().to_owned(),
            })?;
        // The installer writes `bin`, `manifest.toml`, the retained header and
        // the recorded size into this same directory on its own; an asset
        // reusing one of those names would not sit beside the file it is
        // named for, it would land on top of it. Checked before this asset's
        // own step is ever pushed, so a package that fails this refuses
        // whole rather than planning the collision.
        if RESERVED_APPLICATION_ASSET_NAMES.contains(&asset_name.as_str()) {
            return Err(PackageError::ReservedAssetName { name: asset_name });
        }
        steps.push(Step::Member {
            index: asset.index(),
            path: format!("{version_dir}/{asset_name}"),
            mode: FILE_MODE,
        });
    }

    // **Before activation and after everything it sums** — see [`Step::Size`].
    steps.push(Step::Size {
        path: format!("{version_dir}/{SIZE_NAME}"),
    });

    steps.push(Step::Activate {
        link: format!("{app_dir}/{CURRENT_LINK}"),
        staged: format!("{app_dir}/{STAGED_LINK}"),
        target: version.clone(),
    });

    Ok(Plan {
        name,
        version,
        app_dir,
        version_dir,
        steps,
    })
}

// --- Shared Rivets: installing a library. 2026-08-31. -----------------------

/// Everything that will happen to install one **library** artifact.
///
/// A separate type from [`Plan`] and not a variant of it, because the two
/// differ in the one place that matters: an application's plan ends in
/// [`Step::Activate`] and a library's **has no activation at all**. A library
/// installs to a path that already names its content, so there is no link to
/// swap and no version to select — which is
/// `Shared-Libraries-and-the-Build-Target.md` §4.4's *"never `current`"* showing
/// up as a missing field rather than as a rule somebody has to remember.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryPlan {
    reference: library::Reference,
    lib_root: String,
    library_dir: String,
    object_dir: String,
    header_path: String,
    steps: Vec<Step>,
}

impl LibraryPlan {
    /// The reference — which is the directory name, the closure element an
    /// application declares, and the identity, all one string.
    #[must_use]
    pub fn reference(&self) -> &library::Reference {
        &self.reference
    }

    /// `<volume>/lib`.
    #[must_use]
    pub fn lib_root(&self) -> &str {
        &self.lib_root
    }

    /// `<volume>/lib/<reference>`.
    #[must_use]
    pub fn library_dir(&self) -> &str {
        &self.library_dir
    }

    /// `<volume>/lib/<reference>/lib` — what a loader is pointed at.
    #[must_use]
    pub fn object_dir(&self) -> &str {
        &self.object_dir
    }

    /// `<volume>/lib/<reference>/package-header` — the one file
    /// [`Self::already_installed`] and [`unsatisfied`] ask about, and the
    /// reason is the same one [`Step::Header`] documents: this plan writes it
    /// **last**, after every object and the manifest, so its presence is the
    /// one fact on the filesystem that a directory alone cannot fake.
    #[must_use]
    pub fn header_path(&self) -> &str {
        &self.header_path
    }

    #[must_use]
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// **Installing a library that is already there is a no-op, not a refusal.**
    ///
    /// This is the one place a library deliberately behaves *differently* from
    /// an application, and the difference falls out of content addressing rather
    /// than being chosen. [`Plan::occupied`] refuses because an application's
    /// version directory is a function of a version *string*, so two different
    /// builds can want one directory and the second would overwrite a running
    /// program. A library's directory is a function of its **bytes**: once its
    /// retained header is on disk, that header — and everything else this plan
    /// wrote before it — provably describes the bytes being installed now, and
    /// there is nothing left to write and nothing at risk.
    ///
    /// **`header_exists`, not the directory's.** A directory answers this
    /// question wrongly whenever an install is interrupted after
    /// [`Step::Directory`] and before every later step finishes: the
    /// directory's own existence says nothing about whether its bytes are the
    /// ones its reference names. [`Self::header_path`] is the file this plan
    /// writes only once nothing is left to fail, so a caller that asks about
    /// the header instead of the directory cannot mistake an interrupted
    /// install for a finished one.
    ///
    /// That is §4.3's *"two applications needing different versions of one
    /// library is … ordinary"* arriving as a property of the installer: two
    /// applications declaring the same reference install it once and share it,
    /// with no reference counting needed to make the second install safe.
    #[must_use]
    pub fn already_installed(&self, header_exists: bool) -> bool {
        header_exists
    }
}

/// Build the plan for installing a verified **library** package onto a volume.
///
/// `identity` is the artifact's own digest — [`library::identity`], BLAKE3 over
/// the signed region of the header. It is passed in rather than computed here
/// for `rivet-core`'s standing reason: this crate has no dependencies and
/// therefore cannot hash (`ADR-029` §10). The caller that opened the package
/// already held the header bytes and a [`crate::package::port::Digest`].
///
/// **The caller must have computed it over the header of THIS package.** There
/// is no way for this function to check that, which is worth stating rather
/// than leaving implicit: a caller that passed some other digest would install
/// correct bytes under a name that lies about them, and every later resolution
/// would be wrong in the same direction. `cmd/rivet-deploy` computes it inside
/// the same function that opens the package, so the two cannot come apart.
pub fn library_plan(
    volume: &str,
    package: &Package,
    identity: &[u8; 32],
) -> Result<LibraryPlan, PackageError> {
    let header = package.header();
    if *header.kind() != Kind::library() {
        return Err(PackageError::WrongKindForInstall {
            kind: header.kind().as_str().to_owned(),
            wanted: Kind::library().as_str().to_owned(),
        });
    }

    let lib = library::Library::open(package)?;

    // The name goes through `ServiceName` for the reason an application's does:
    // it becomes a path component, and the header field it came from is
    // attacker-chosen. A library is not a service and never will be one, so this
    // is the *validator* being reused and not the concept.
    let name = ServiceName::new(header.product()).ok_or_else(|| PackageError::UnusableName {
        product: header.product().to_owned(),
    })?;
    let version =
        safe_component(header.version()).ok_or_else(|| PackageError::UnusableVersion {
            version: header.version().to_owned(),
        })?;

    let reference = library::Reference::compose(name.as_str(), &version, identity).map_err(|_| {
        // Reached only if a name or version that passed the two checks above
        // still cannot be a reference — a longer combination than
        // `MAX_REFERENCE`. It is a version problem either way, and the field
        // named is the one the builder can change.
        PackageError::UnusableVersion {
            version: header.version().to_owned(),
        }
    })?;

    let lib_root_path = lib_root(volume);
    let library_dir = format!("{lib_root_path}/{reference}");
    let object_dir = format!("{library_dir}/{}", library::OBJECT_DIR);
    let header_path = format!("{library_dir}/{HEADER_NAME}");

    let mut steps = vec![
        Step::Directory {
            path: lib_root_path.clone(),
            mode: DIR_MODE,
        },
        Step::Directory {
            path: library_dir.clone(),
            mode: DIR_MODE,
        },
        Step::Directory {
            path: object_dir.clone(),
            mode: DIR_MODE,
        },
        Step::Member {
            index: lib.manifest_member().index(),
            path: format!("{library_dir}/{}", library::MANIFEST_NAME),
            mode: FILE_MODE,
        },
    ];

    // **An object lands under its MEMBER's name**, which is the one place a
    // library differs from an application on purpose. An application's program
    // is named after the application because there is exactly one and no `exec`
    // field to choose with (`ADR-015` §3.2). A library's objects are found by
    // SONAME — the loader opens a filename it read out of another object's
    // dynamic section — so the name is load-bearing and cannot be derived.
    //
    // That makes a member name a path here, so it goes through `safe_component`
    // like an asset does, after the container has already refused a traversal.
    for object in lib.objects() {
        let object_name =
            safe_component(object.name()).ok_or_else(|| PackageError::UnsafeMemberName {
                name: object.name().to_owned(),
            })?;
        steps.push(Step::Member {
            index: object.index(),
            // `PROGRAM_MODE` — `0555`. A shared object is mapped executable, and
            // it is writable by nobody for `ADR-015` §4.2's reason: a library an
            // application could rewrite would make the header beside it a
            // formality.
            path: format!("{object_dir}/{object_name}"),
            mode: PROGRAM_MODE,
        });
    }

    for asset in lib.assets() {
        let asset_name =
            safe_component(asset.name()).ok_or_else(|| PackageError::UnsafeMemberName {
                name: asset.name().to_owned(),
            })?;
        // The library equivalent of the check [`plan`] makes for an
        // application's own assets: `lib`, `manifest.toml` and the retained
        // header are this installer's own names in this same directory.
        if RESERVED_LIBRARY_ASSET_NAMES.contains(&asset_name.as_str()) {
            return Err(PackageError::ReservedAssetName { name: asset_name });
        }
        steps.push(Step::Member {
            index: asset.index(),
            path: format!("{library_dir}/{asset_name}"),
            mode: FILE_MODE,
        });
    }

    // Last, exactly as an application's is, and for a reason that survives the
    // absence of an activation step: a directory must never hold a header
    // describing files that are not all there yet.
    steps.push(Step::Header {
        path: header_path.clone(),
        mode: FILE_MODE,
    });

    Ok(LibraryPlan {
        reference,
        lib_root: lib_root_path,
        library_dir,
        object_dir,
        header_path,
        steps,
    })
}

/// Which of `closure` this machine does not hold, given the references it does.
///
/// **The refusal is at install and it names what is missing** —
/// `Shared-Libraries-and-the-Build-Target.md` §4.4. The alternative is a loader
/// error at start, which reaches an operator as a service that will not run and
/// a journal line about a filename, with nothing saying which artifact to go
/// and fetch.
///
/// `present` is asked one question per reference — *is [`library_header_path`]
/// there* — and that is the whole of the I/O. **Not the directory**, for
/// [`LibraryPlan::already_installed`]'s reason: an install interrupted after
/// [`Step::Directory`] leaves a directory whose own existence says nothing
/// about whether its bytes are the ones its reference names, and a dependent
/// application must see that as missing rather than satisfied. **No
/// directory listing is needed and none is
/// used**, which is worth stating because it is a property of content
/// addressing rather than an optimisation: a reference *is* a directory name, so
/// satisfying a closure is a series of `stat` calls with no search and no
/// matching rule to get wrong.
///
/// Returns the missing references in the order they were declared, so the
/// refusal reads in the order the author wrote them.
#[must_use]
pub fn unsatisfied(
    volume: &str,
    closure: &[library::Reference],
    mut present: impl FnMut(&str) -> bool,
) -> Vec<String> {
    closure
        .iter()
        .filter(|needed| !present(&library_header_path(volume, needed)))
        .map(|needed| needed.as_str().to_owned())
        .collect()
}

/// `<volume>/lib` — the directory every library installs under, and the one
/// an adapter lists to discover what a machine has (`PLAN-Feature-Set-
/// 2026-09-12` §6.2, `ADR-053`). One function, for [`library_dir`]'s reason:
/// the installer, the supervisor and anything that lists the directory must
/// not be able to spell it differently.
#[must_use]
pub fn lib_root(volume: &str) -> String {
    format!("{volume}/{}", library::LIB_DIR)
}

/// Where a reference is installed. **One function, so that the installer that
/// writes the directory and the supervisor that resolves it cannot spell it
/// differently.**
#[must_use]
pub fn library_dir(volume: &str, reference: &library::Reference) -> String {
    format!("{}/{reference}", lib_root(volume))
}

/// The directory a loader is pointed at for one reference.
#[must_use]
pub fn library_object_dir(volume: &str, reference: &library::Reference) -> String {
    format!("{}/{}", library_dir(volume, reference), library::OBJECT_DIR)
}

/// The one file [`unsatisfied`] and a caller building
/// [`LibraryPlan::already_installed`]'s argument ask about — never
/// [`library_dir`] alone. See [`LibraryPlan::header_path`] for why a
/// directory's own presence is not enough.
#[must_use]
pub fn library_header_path(volume: &str, reference: &library::Reference) -> String {
    format!("{}/{HEADER_NAME}", library_dir(volume, reference))
}

// --- Retention: a library is kept per product. `.rulings/` R-202. ----------

/// How many builds of one library product a machine keeps under `<volume>/lib`.
///
/// **One**, and the number is what `SPEC-02` §12 argues at length; the short
/// form is that any other number leaves the state this rule exists to prevent.
/// A library's identity is the digest of its bytes and Rust builds are not
/// byte-reproducible, so *the same source built twice* is two directories.
/// Keeping two of them keeps two libraries of one class installed — which is
/// exactly what `.rulings/` R-200 makes `Loaded::open(class)` refuse, and the
/// card that addon feeds goes dark on the upgrade rather than on the mistake.
///
/// **It is not a smaller [`crate::artifact::KEEP`], because it is not the same
/// job.** That constant keeps three `.rvt` payloads of a product in the
/// machine's artifact store so that a rollback is local. It still does:
/// rolling a library back is installing the older digest the store still
/// holds, and this constant governs only how many are **unpacked** at once.
pub const KEEP_LIBRARY: usize = 1;

/// One directory under `<volume>/lib`, as a caller that listed it observed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldLibrary {
    /// The directory's name, parsed — which `SPEC-02` §3 makes the identity
    /// too.
    pub reference: library::Reference,
    /// When it was installed, as any ordinal that increases. On a machine this
    /// is the directory's modification time in seconds. **A tie-break and not
    /// the rule**, exactly as [`crate::artifact::Stored::added`] is.
    pub added: u64,
}

impl crate::artifact::Retained for HeldLibrary {
    /// The `<name>` half of the reference, or the whole head when it carries
    /// no version — which forms a group of one and is therefore never evicted
    /// by, or in favour of, anything else.
    fn product(&self) -> String {
        self.reference.product().to_string()
    }

    fn version_order(&self) -> crate::artifact::VersionOrder {
        self.reference
            .product_and_version()
            .map_or(crate::artifact::VersionOrder::Unreadable, |(_, version)| {
                crate::artifact::version_order(version)
            })
    }

    fn added(&self) -> u64 {
        self.added
    }

    /// **The whole reference**, which already ends in the digest of the bytes
    /// — so it is unique in this store by construction and needs no second
    /// field to make the order total.
    fn identity(&self) -> &str {
        self.reference.as_str()
    }
}

/// Which installed libraries a machine should remove, given everything under
/// `<volume>/lib` and every reference its applications declare.
///
/// `SPEC-02` §12, `.rulings/` R-202. The arithmetic is
/// [`crate::artifact::evicted`]'s in full — group by product, hold unreadable
/// versions apart, order by version and never by arrival, and never drop
/// `arriving`. **This function is the guard around it**, and the guard is the
/// half that makes retention safe to run at all.
///
/// # The guard: a declared library is never evicted
///
/// **The two kinds of library are not alike.** A class-joined addon is
/// *discovered* — nothing names it, the machine chose it by reading manifests,
/// and the machine may therefore replace it. A library in `declared` is named
/// by **exact reference inside a signed manifest** (`SPEC-02` §5) and is
/// load-bearing: the application resolves it by `stat` at every start, so
/// evicting it breaks that application at its next start. That is precisely
/// the failure §5.1's install-time closure check exists to prevent, arriving
/// later and by another route.
///
/// **Taken off the table rather than held back inside the sort**, so a
/// declared library costs its product's quota nothing. Holding it back inside
/// the ordering would mean protecting an old library evicted a newer one to
/// pay for it — the guard doing harm in the direction it was written to
/// prevent.
///
/// # What the caller owes this function
///
/// **`declared` must be every installed application's closure, or the call
/// must not happen.** A caller that could not read one application's manifest
/// does not know what that application needs, and a partial `declared` is a
/// guard with a hole in it. The safe answer to *"I could not read it"* is to
/// evict nothing, and the adapters do that rather than passing what they have.
#[must_use]
pub fn library_evictions<'a>(
    held: &'a [HeldLibrary],
    keep: usize,
    arriving: Option<&library::Reference>,
    declared: &[library::Reference],
) -> Vec<&'a HeldLibrary> {
    let candidates: Vec<&HeldLibrary> = held
        .iter()
        .filter(|entry| !declared.contains(&entry.reference))
        .collect();
    crate::artifact::evicted(&candidates, keep, arriving.map(library::Reference::as_str))
        .into_iter()
        .copied()
        .collect()
}

/// One path component that is safe to append to a directory this machine owns,
/// or nothing.
///
/// **The last of four checks, not the whole defence** — see this module's table.
/// The container refuses a traversal in a *member name* before a signature is
/// computed, and `ServiceName` refuses a name that could not be one. What is
/// left for this function is a header `version`, which goes through the looser
/// `read_text` and may therefore hold a `/`, and the reserved names that already
/// mean something under an application's root.
///
/// The redundant checks are kept anyway. They cost a comparison, this is the
/// code where `ADR-028` §2 says the failure mode is *"an attacker's payload
/// verifies"*, and a function whose safety depends on a caller elsewhere having
/// already checked is one that becomes wrong the day it gains a second caller.
fn safe_component(text: &str) -> Option<String> {
    if text.is_empty() || text.len() > ServiceName::MAX_LEN {
        return None;
    }
    if text == "." || text == ".." {
        return None;
    }
    // `current` would be replaced by a directory, and `data` holds everything
    // the application has ever written. Neither is a hypothetical: a version
    // string is chosen by whoever built the package.
    if text == CURRENT_LINK || text == DATA_DIR || text == STAGED_LINK {
        return None;
    }
    if !text
        .chars()
        .all(|c| c.is_ascii_graphic() && c != '/' && c != '\\')
    {
        return None;
    }
    Some(text.to_owned())
}

/// Every name the supervisor gives its own meaning inside an application's
/// directory, which a version directory beside them must therefore never take:
/// the `current` link and its staged replacement, `data`, the scratch
/// directory `tmp`, and the three files written there for the application
/// rather than by it (its settings, the proxy's routing table, the name
/// service's projection).
///
/// A version named `tmp` would be the directory the supervisor empties and
/// hands the application as scratch space; a version named `settings` would be
/// a directory where a file is about to be written. Neither is a traversal.
pub const RESERVED_IN_APPLICATION_DIR: &[&str] = &[
    CURRENT_LINK,
    STAGED_LINK,
    DATA_DIR,
    service::TMP_DIR,
    crate::settings::FILE,
    crate::proxy::FILE,
    crate::name_service::FILE,
];

/// An application version that is safe to name a directory beside `current`,
/// `data` and `tmp`, or nothing: [`safe_component`], never one of
/// [`RESERVED_IN_APPLICATION_DIR`], and **never a name carrying
/// [`service::RECLAIM_MARK`]**. Such a version would share its directory's name
/// with a removed version or an emptied scratch directory awaiting deletion
/// beside it, and the helper deleting those would take it too (`ADR-017` §3).
fn safe_version(text: &str) -> Option<String> {
    if text.contains(service::RECLAIM_MARK) || RESERVED_IN_APPLICATION_DIR.contains(&text) {
        return None;
    }
    safe_component(text)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    // -- the traversal defence -------------------------------------------------

    /// **The one function this module's safety rests on.** Every case here is a
    /// real shape somebody has used to escape a directory, plus the two names
    /// that already mean something under an application's root.
    #[test]
    fn no_component_that_could_leave_the_directory_is_accepted() {
        for bad in [
            "",
            ".",
            "..",
            "../..",
            "a/b",
            "/etc/passwd",
            "..\\windows",
            "a\\b",
            "with space",
            "with\ttab",
            "nul\0byte",
            "café",
            CURRENT_LINK,
            DATA_DIR,
            STAGED_LINK,
        ] {
            assert_eq!(safe_component(bad), None, "{bad:?} was accepted");
        }
    }

    #[test]
    fn an_ordinary_version_is_one_component() {
        for good in ["0.1.0", "1.2.3-rc1", "0.0.1-g7b84d4b", "2026.08.24", "v1"] {
            assert_eq!(safe_component(good).as_deref(), Some(good), "{good:?}");
            assert_eq!(safe_version(good).as_deref(), Some(good), "{good:?}");
        }
    }

    /// **A version never carries the mark of a removed thing**, so it can
    /// never be the name a removal renames to, nor be taken by the helper that
    /// deletes one (`ADR-017` §3).
    #[test]
    fn a_version_that_names_something_the_supervisor_reserves_is_refused() {
        for bad in RESERVED_IN_APPLICATION_DIR {
            assert_eq!(safe_version(bad), None, "{bad:?} was accepted");
        }
        // Spelled out, so a change to a constant cannot quietly drop one.
        for bad in ["tmp", "settings", "routes", "dns.toml", "data", "current", ".current.new"] {
            assert_eq!(safe_version(bad), None, "{bad:?} was accepted");
        }
    }

    #[test]
    fn a_version_carrying_the_reclaim_mark_is_refused() {
        for bad in [".reclaim-1-1", "tmp.reclaim-1-1", "1.0.0-a.reclaim-5-1"] {
            assert_eq!(safe_version(bad), None, "{bad:?} was accepted");
        }
    }

    /// A component may not be longer than a name may be. The bound is
    /// `ServiceName`'s rather than a second number, because the longest thing
    /// that can appear here is a name.
    #[test]
    fn a_component_longer_than_a_name_is_refused() {
        let long = "a".repeat(ServiceName::MAX_LEN);
        assert_eq!(safe_component(&long).as_deref(), Some(long.as_str()));
        let too_long = "a".repeat(ServiceName::MAX_LEN + 1);
        assert_eq!(safe_component(&too_long), None);
    }
}

#[cfg(test)]
mod plan_tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::package::application::{
        Manifest, Memory, Network, ProgramKind, Ready, Restart, StateGrant, TargetClass,
        VARIANT_NONE,
    };
    use crate::package::port::{MaySign, TrustSet};
    use crate::package::read::open;
    use crate::package::testing::{ToyDigest, ToyKey, ToySigner, ToyVerifier};
    use crate::package::write::{Builder, NewMember};

    const SIGNING_KEY: ToyKey = ToyKey(7);

    fn trust() -> TrustSet {
        let mut digest = ToyDigest::new();
        let mut trust = TrustSet::new();
        trust.trust(
            &mut digest,
            SIGNING_KEY.public(),
            "test-apps",
            // All three kinds, so that a package this module refuses is
            // refused HERE rather than one layer earlier by the entitlement
            // check — which would pass the test for the wrong reason.
            MaySign::these(&[Kind::application(), Kind::update(), Kind::library()]),
        );
        trust
    }

    fn manifest_bytes() -> Vec<u8> {
        Manifest::new(ProgramKind::Daemon)
            .with_restart(Restart::OnFailure)
            .with_ready(Ready::Listening)
            .with_network(Network::None)
            .with_state(StateGrant::Private)
            .with_memory(Some(Memory::parse("128M").unwrap()))
            .render()
            .into_bytes()
    }

    fn packaged(
        product: &str,
        version: &str,
        kind: Kind,
        members: &[(&str, u8, u8, Vec<u8>)],
    ) -> Result<Package, PackageError> {
        let mut digest = ToyDigest::new();
        let mut signer = ToySigner::new(SIGNING_KEY);
        let mut builder = Builder::new(product, version, "aarch64", None, kind)?;
        for (name, class, variant, bytes) in members {
            builder.add(NewMember::stored(
                name,
                *class,
                *variant,
                bytes,
                &mut digest,
            )?)?;
        }
        let slices: Vec<&[u8]> = members.iter().map(|(_, _, _, b)| b.as_slice()).collect();
        let file = builder.seal_in_memory(&mut digest, &mut signer, &slices)?;
        let mut digest = ToyDigest::new();
        let verifier = ToyVerifier::new();
        open(&file, file.len() as u64, &mut digest, &verifier, &trust())
    }

    fn members(asset: Option<&str>) -> Vec<(String, u8, u8, Vec<u8>)> {
        let mut v = vec![
            (
                MANIFEST_NAME.to_string(),
                TargetClass::Manifest.as_byte(),
                VARIANT_NONE,
                manifest_bytes(),
            ),
            (
                // Deliberately NOT the application's name: the program lands at
                // a path derived from the name, and a member name has no say in
                // where it goes.
                "whatever-the-crate-called-it".to_string(),
                TargetClass::Program.as_byte(),
                VARIANT_NONE,
                b"not really an ELF".to_vec(),
            ),
        ];
        if let Some(name) = asset {
            v.push((
                name.to_string(),
                TargetClass::Asset.as_byte(),
                VARIANT_NONE,
                b"asset\n".to_vec(),
            ));
        }
        v
    }

    fn app(product: &str, version: &str, asset: Option<&str>) -> Package {
        let owned = members(asset);
        let borrowed: Vec<(&str, u8, u8, Vec<u8>)> = owned
            .iter()
            .map(|(n, c, v, b)| (n.as_str(), *c, *v, b.clone()))
            .collect();
        packaged(product, version, Kind::application(), &borrowed).expect("a valid package")
    }

    // -- the library equivalent, for `library_plan` and `unsatisfied` ----------

    fn library_manifest_bytes() -> Vec<u8> {
        library::Manifest::new(vec!["libgreet.so.1".to_string()])
            .expect("a well-formed soname")
            .render()
            .into_bytes()
    }

    /// A manifest, one object, and — if named — one asset. Mirrors
    /// [`members`], the application equivalent.
    fn lib_members(asset: Option<&str>) -> Vec<(String, u8, u8, Vec<u8>)> {
        let mut v = vec![
            (
                library::MANIFEST_NAME.to_string(),
                library::TargetClass::Manifest.as_byte(),
                VARIANT_NONE,
                library_manifest_bytes(),
            ),
            (
                "libgreet.so.1".to_string(),
                library::TargetClass::Object.as_byte(),
                VARIANT_NONE,
                b"not really an ELF".to_vec(),
            ),
        ];
        if let Some(name) = asset {
            v.push((
                name.to_string(),
                library::TargetClass::Asset.as_byte(),
                VARIANT_NONE,
                b"asset\n".to_vec(),
            ));
        }
        v
    }

    fn lib(product: &str, version: &str, asset: Option<&str>) -> Package {
        let owned = lib_members(asset);
        let borrowed: Vec<(&str, u8, u8, Vec<u8>)> = owned
            .iter()
            .map(|(n, c, v, b)| (n.as_str(), *c, *v, b.clone()))
            .collect();
        packaged(product, version, Kind::library(), &borrowed).expect("a valid library package")
    }

    // -- the shape of an ordinary install --------------------------------------

    /// **Every directory the plan needs is a step in it**, including the ones
    /// that are somebody else's in spirit.
    ///
    /// `Kernel::mkdir_mode` does not create parents, on the argument that a
    /// missing parent means the *plan* is wrong. That argument is sound and
    /// this plan was wrong: it began at `/state/app/<name>` and nothing on a
    /// Rivet had ever created `/state/app`, so the first install on real
    /// hardware refused at the first step with `No such file or directory`
    /// (2026-08-25). Derived from the plan rather than written out, so a fourth
    /// directory cannot be added without its parent.
    #[test]
    fn the_plan_names_every_ancestor_it_relies_on() {
        let package = app("photo-index", "0.1.0", None);
        let plan = plan(service::BUILT_IN_VOLUME, &package).expect("a plan");
        let made: Vec<&str> = plan
            .steps()
            .iter()
            .filter_map(|s| match s {
                Step::Directory { path, .. } => Some(path.as_str()),
                _ => None,
            })
            .collect();
        for dir in &made {
            let Some((parent, _)) = dir.rsplit_once('/') else {
                unreachable!("{dir} is not an absolute path")
            };
            // `/state` is PID 1's mount, not this plan's to create.
            if parent == crate::state::STATE_MOUNT_POINT || parent.is_empty() {
                continue;
            }
            assert!(
                made.contains(&parent),
                "the plan creates {dir} and never creates {parent}: {made:?}"
            );
        }
        assert!(
            made.contains(&crate::service::APP_ROOT),
            "ADR-015 §4.2's /state/app is nobody else's to make: {made:?}"
        );
    }

    // ---- ADR-015 §7.2: nothing unpacks over a directory that is there ------

    /// A version directory that is not there is the ordinary case, and it must
    /// stay ordinary — a check that refused everything would be a check nobody
    /// could keep.
    #[test]
    fn a_version_directory_that_is_not_there_is_installed_into() {
        let package = app("photo-index", "0.1.0", None);
        let plan = plan(service::BUILT_IN_VOLUME, &package).expect("a plan");
        assert_eq!(plan.occupied(false), None);
    }

    /// **The refusal, and what it has to contain.** `AGENTS.md` §2 and this
    /// project's standing rule: a refusal names itself and cites something. The
    /// two ways forward are asserted too, because a refusal that stops an
    /// operator without telling them what to do next is how a workaround gets
    /// invented.
    #[test]
    fn a_version_directory_that_is_already_there_is_refused_by_name() {
        let package = app("photo-index", "0.1.0", None);
        let plan = plan(service::BUILT_IN_VOLUME, &package).expect("a plan");
        let refusal = plan
            .occupied(true)
            .expect("an occupied version directory is refused");

        // It names the directory rather than the application, because the
        // directory is the thing that would have been written over.
        assert!(
            refusal.contains("/state/app/photo-index/0.1.0"),
            "{refusal}"
        );
        // It cites the ruling, and the naming ruling that is the other half.
        assert!(refusal.contains("ADR-015 §7.2"), "{refusal}");
        assert!(refusal.contains("ADR-027 §3.2.2"), "{refusal}");
        // It says nothing was written, which is the fact an operator acts on.
        assert!(refusal.contains("NOTHING WAS WRITTEN"), "{refusal}");
        // And both ways forward.
        assert!(refusal.contains("rebuild"), "{refusal}");
        assert!(refusal.contains("uninstall"), "{refusal}");
    }

    /// The refusal must be about **this** install, not a template: two
    /// applications produce two different sentences, so a machine's journal
    /// says which one was refused.
    #[test]
    fn the_refusal_names_the_application_that_was_refused() {
        let first = plan(
            service::BUILT_IN_VOLUME,
            &app("photo-index", "0.1.0", None),
        )
        .expect("a plan")
        .occupied(true)
        .expect("refused");
        let second = plan(
            service::BUILT_IN_VOLUME,
            &app("metrics-collector", "2.0.0", None),
        )
        .expect("a plan")
        .occupied(true)
        .expect("refused");
        assert_ne!(first, second);
        assert!(second.contains("metrics-collector"), "{second}");
        assert!(second.contains("2.0.0"), "{second}");
    }

    /// **The whole plan for a plain application**, written out rather than
    /// summarised, because the ORDER is the correctness argument: directories
    /// before what goes in them, and the link swap last.
    #[test]
    fn an_ordinary_application_installs_into_its_version_and_is_activated_last() {
        let package = app("photo-index", "0.1.0", None);
        let plan = plan(service::BUILT_IN_VOLUME, &package).expect("a plan");

        assert_eq!(plan.name().as_str(), "photo-index");
        assert_eq!(plan.version(), "0.1.0");
        assert_eq!(plan.app_dir(), "/state/app/photo-index");
        assert_eq!(plan.version_dir(), "/state/app/photo-index/0.1.0");

        assert_eq!(
            plan.steps(),
            &[
                // `/state/app` itself. Absent until 2026-08-25, when a board
                // refused the first real install because nothing had ever
                // created it — the port does not make parents, deliberately.
                Step::Directory {
                    path: "/state/app".to_string(),
                    mode: DIR_MODE
                },
                Step::Directory {
                    path: "/state/app/photo-index".to_string(),
                    mode: DIR_MODE
                },
                Step::Directory {
                    path: "/state/app/photo-index/0.1.0".to_string(),
                    mode: DIR_MODE
                },
                Step::Directory {
                    path: "/state/app/photo-index/0.1.0/bin".to_string(),
                    mode: DIR_MODE
                },
                Step::Member {
                    index: 1,
                    path: "/state/app/photo-index/0.1.0/bin/photo-index".to_string(),
                    mode: PROGRAM_MODE
                },
                Step::Member {
                    index: 0,
                    path: "/state/app/photo-index/0.1.0/manifest.toml".to_string(),
                    mode: FILE_MODE
                },
                // The signed header, last before the activation: a version
                // directory never carries a header without the files it
                // covers. `ADR-015` §4.2, 2026-08-25.
                Step::Header {
                    path: "/state/app/photo-index/0.1.0/package-header".to_string(),
                    mode: FILE_MODE
                },
                // The recorded size, after everything it sums and before the
                // activation that makes this version live. `R34`, `.rulings/`
                // R-163.
                Step::Size {
                    path: "/state/app/photo-index/0.1.0/install-size".to_string(),
                },
                Step::Activate {
                    link: "/state/app/photo-index/current".to_string(),
                    staged: "/state/app/photo-index/.current.new".to_string(),
                    target: "0.1.0".to_string()
                },
            ]
        );
    }

    /// **The program lands where the supervisor will look for it**, checked
    /// against `service::program_path` rather than against a literal. Two
    /// spellings of one path is `ADR-011` §3, and the failure it produces is an
    /// application that installs perfectly and never starts.
    #[test]
    fn the_program_is_written_exactly_where_the_supervisor_execs_from() {
        let package = app("photo-index", "0.1.0", None);
        let plan = plan(service::BUILT_IN_VOLUME, &package).expect("a plan");
        let expected = service::program_path(service::BUILT_IN_VOLUME, plan.name());
        let written = plan
            .steps()
            .iter()
            .find_map(|s| match s {
                Step::Member { path, mode, .. } if *mode == PROGRAM_MODE => Some(path.clone()),
                _ => None,
            })
            .expect("the program is written");
        // The plan writes through the version directory; the supervisor reads
        // through `current`, which the last step points at that version.
        assert_eq!(
            written.replace("/0.1.0/", &format!("/{CURRENT_LINK}/")),
            expected
        );
    }

    /// And the manifest lands where `service::manifest_path` says.
    #[test]
    fn the_manifest_is_written_where_the_supervisor_reads_it() {
        let package = app("photo-index", "2.0.0", None);
        let plan = plan(service::BUILT_IN_VOLUME, &package).expect("a plan");
        let expected = service::manifest_path(service::BUILT_IN_VOLUME, plan.name());
        let written = plan
            .steps()
            .iter()
            .find_map(|s| match s {
                Step::Member { path, .. } if path.ends_with(MANIFEST_NAME) => Some(path.clone()),
                _ => None,
            })
            .expect("the manifest is written");
        assert_eq!(
            written.replace("/2.0.0/", &format!("/{CURRENT_LINK}/")),
            expected
        );
    }

    /// An asset is one more file in the version directory, under the name it was
    /// packaged with.
    #[test]
    fn an_asset_lands_beside_the_manifest_under_the_name_it_was_packaged_with() {
        let package = app("photo-index", "0.1.0", Some("banner.txt"));
        let plan = plan(service::BUILT_IN_VOLUME, &package).expect("a plan");
        assert!(
            plan.steps().contains(&Step::Member {
                index: 2,
                path: "/state/app/photo-index/0.1.0/banner.txt".to_string(),
                mode: FILE_MODE,
            }),
            "{:?}",
            plan.steps()
        );
    }

    /// **The link is swapped last, always.** A plan abandoned at any earlier
    /// point leaves `current` pointing where it pointed before, which is the
    /// whole of `ADR-015` §7's argument about a Pi whose power is pulled.
    #[test]
    fn the_activation_is_the_last_step_whatever_else_is_in_the_package() {
        for asset in [None, Some("a.txt")] {
            let package = app("photo-index", "0.1.0", asset);
            let plan = plan(service::BUILT_IN_VOLUME, &package).expect("a plan");
            let last = plan.steps().last().expect("steps");
            assert!(
                matches!(last, Step::Activate { .. }),
                "the last step is {last:?}"
            );
            // ...and it is the ONLY one, so there is exactly one moment at which
            // the machine changes its mind about which version is live.
            assert_eq!(
                plan.steps()
                    .iter()
                    .filter(|s| matches!(s, Step::Activate { .. }))
                    .count(),
                1
            );
        }
    }

    /// Every directory is created before anything is written into it. Walked
    /// rather than read, because this is the property an executor relies on and
    /// a reordering would break it silently.
    #[test]
    fn nothing_is_written_into_a_directory_the_plan_has_not_made_yet() {
        let package = app("photo-index", "0.1.0", Some("banner.txt"));
        let plan = plan(service::BUILT_IN_VOLUME, &package).expect("a plan");
        let mut made: Vec<String> = Vec::new();
        for step in plan.steps() {
            match step {
                Step::Directory { path, .. } => made.push(path.clone()),
                Step::Member { path, .. } | Step::Header { path, .. } | Step::Size { path } => {
                    let parent = path.rsplit_once('/').expect("an absolute path").0;
                    assert!(
                        made.iter().any(|d| d == parent),
                        "{path} is written into {parent}, which no earlier step creates"
                    );
                }
                Step::Activate { staged, .. } => {
                    let parent = staged.rsplit_once('/').expect("an absolute path").0;
                    assert!(made.iter().any(|d| d == parent), "{staged}");
                }
            }
        }
    }

    /// **No step ever names the data directory.** Stronger than being careful
    /// about it: `ADR-015` §3.2's `state = "private"` survives an upgrade
    /// because the plan cannot express touching it.
    #[test]
    fn no_step_names_the_directory_holding_everything_the_application_wrote() {
        let package = app("photo-index", "0.1.0", Some("banner.txt"));
        let plan = plan(service::BUILT_IN_VOLUME, &package).expect("a plan");
        let data = service::data_dir(service::BUILT_IN_VOLUME, plan.name());
        for step in plan.steps() {
            let paths = match step {
                Step::Directory { path, .. }
                | Step::Member { path, .. }
                | Step::Header { path, .. }
                | Step::Size { path } => vec![path.clone()],
                Step::Activate { link, staged, .. } => vec![link.clone(), staged.clone()],
            };
            for path in paths {
                assert_ne!(path, data, "{step:?}");
                assert!(!path.starts_with(&format!("{data}/")), "{step:?}");
            }
        }
    }

    // -- the refusals ----------------------------------------------------------

    /// **What this module catches that the container format does not.**
    ///
    /// `format::read_name` already refuses `/`, a backslash, `.`, `..`, an empty
    /// name and anything that is not printable ASCII — asserted by
    /// `the_container_refuses_a_traversal_before_this_module_ever_sees_it`. So
    /// the traversal shapes never reach here at all, and what is left for this
    /// module is the narrower and far less obvious danger: **a name that is
    /// perfectly well-formed and already means something under an application's
    /// root.**
    ///
    /// The signature is valid and the key is trusted in every case below. What
    /// the package is not is well-formed.
    #[test]
    fn an_asset_named_for_something_that_already_exists_there_is_refused() {
        for hostile in [CURRENT_LINK, DATA_DIR, STAGED_LINK] {
            let package = app("photo-index", "0.1.0", Some(hostile));
            let refusal =
                plan(service::BUILT_IN_VOLUME, &package).expect_err(&format!("{hostile:?} was planned rather than refused"));
            assert!(
                matches!(refusal, PackageError::UnsafeMemberName { .. }),
                "{hostile:?}: {refusal:?}"
            );
            let message = refusal.to_string();
            assert!(message.contains(hostile), "{message}");
            assert!(!message.contains("  "), "{message}");
        }
    }

    /// **A name that is safe as a path component, and reserved anyway**,
    /// because this installer writes a file of exactly that name into the
    /// same directory on its own. `manifest.toml` belongs to the same
    /// reserved list and is asserted separately below: a package cannot even
    /// be built with a second member sharing the real manifest's name, so
    /// this fixture has no way to reach it through the ordinary asset path.
    #[test]
    fn an_asset_reusing_a_name_the_installer_writes_for_itself_is_refused() {
        assert!(RESERVED_APPLICATION_ASSET_NAMES.contains(&MANIFEST_NAME));
        for hostile in [BIN_DIR, HEADER_NAME, SIZE_NAME] {
            let package = app("photo-index", "0.1.0", Some(hostile));
            let refusal = plan(service::BUILT_IN_VOLUME, &package)
                .expect_err(&format!("{hostile:?} was planned rather than refused"));
            assert!(
                matches!(refusal, PackageError::ReservedAssetName { .. }),
                "{hostile:?}: {refusal:?}"
            );
            let message = refusal.to_string();
            assert!(message.contains(hostile), "{message}");
            assert!(!message.contains("  "), "{message}");
        }
    }

    /// The refusal above must not have widened into refusing every asset —
    /// an ordinary name still plans and lands at the expected path.
    #[test]
    fn an_ordinary_asset_still_installs() {
        let package = app("photo-index", "0.1.0", Some("banner.txt"));
        let plan = plan(service::BUILT_IN_VOLUME, &package).expect("a plan");
        assert!(
            plan.steps().iter().any(|s| matches!(
                s,
                Step::Member { path, .. } if path.ends_with("/banner.txt")
            )),
            "{:?}",
            plan.steps()
        );
    }

    /// The library equivalent: `lib` and the retained header are reserved,
    /// and `manifest.toml` is checked the same separated way for the same
    /// reason.
    #[test]
    fn a_library_asset_reusing_a_name_the_installer_writes_for_itself_is_refused() {
        assert!(RESERVED_LIBRARY_ASSET_NAMES.contains(&library::MANIFEST_NAME));
        for hostile in [library::OBJECT_DIR, HEADER_NAME] {
            let package = lib("libgreet", "1.0.0", Some(hostile));
            let refusal = library_plan(service::BUILT_IN_VOLUME, &package, &[0x33; 32])
                .expect_err(&format!("{hostile:?} was planned rather than refused"));
            assert!(
                matches!(refusal, PackageError::ReservedAssetName { .. }),
                "{hostile:?}: {refusal:?}"
            );
            let message = refusal.to_string();
            assert!(message.contains(hostile), "{message}");
            assert!(!message.contains("  "), "{message}");
        }
    }

    /// A library asset with an ordinary name still installs.
    #[test]
    fn an_ordinary_library_asset_still_installs() {
        let package = lib("libgreet", "1.0.0", Some("README"));
        let plan = library_plan(service::BUILT_IN_VOLUME, &package, &[0x44; 32]).expect("a plan");
        assert!(
            plan.steps().iter().any(|s| matches!(
                s,
                Step::Member { path, .. } if path.ends_with("/README")
            )),
            "{:?}",
            plan.steps()
        );
    }

    // -- a library is installed only once its header is written ----------------

    /// **The API shape the fix rests on**: the header lives inside the
    /// directory the plan names, at a path distinct from the directory
    /// itself, and [`LibraryPlan::already_installed`] is a plain function of
    /// whether THAT file — never the directory — is there.
    #[test]
    fn a_library_is_installed_only_when_its_retained_header_is_there() {
        let package = lib("libgreet", "1.0.0", None);
        let plan = library_plan(service::BUILT_IN_VOLUME, &package, &[0x11; 32]).expect("a plan");

        assert!(plan.header_path().starts_with(plan.library_dir()));
        assert_ne!(plan.header_path(), plan.library_dir());
        assert!(plan.header_path().ends_with(HEADER_NAME));

        // A caller that can only say the DIRECTORY is there — exactly the
        // state an install interrupted right after `Step::Directory` and
        // before anything else leaves — must not read as installed.
        assert!(!plan.already_installed(false));
        // A caller that can say the HEADER is there reads as installed: this
        // plan writes it last, after every object and the manifest.
        assert!(plan.already_installed(true));
    }

    /// **The regression, at the level `unsatisfied` actually answers from.**
    /// A `present` that only knows about the library's directory — not its
    /// header — must not satisfy a dependent application's closure; the
    /// directory is exactly what an interrupted install leaves behind.
    #[test]
    fn unsatisfied_reports_a_library_whose_header_is_not_yet_written_as_missing() {
        let package = lib("libgreet", "1.0.0", None);
        let plan = library_plan(service::BUILT_IN_VOLUME, &package, &[0x22; 32]).expect("a plan");
        let closure = vec![plan.reference().clone()];

        let missing = unsatisfied(service::BUILT_IN_VOLUME, &closure, |path| {
            path == plan.library_dir()
        });
        assert_eq!(
            missing,
            vec![plan.reference().as_str().to_string()],
            "a directory with no header is an interrupted install and must be reported missing"
        );

        let missing = unsatisfied(service::BUILT_IN_VOLUME, &closure, |path| {
            path == plan.header_path()
        });
        assert!(missing.is_empty(), "{missing:?}");
    }

    /// **The first line of defence is the container**, asserted here so that a
    /// regression in it fails a test in the module that leans on it.
    ///
    /// These packages cannot even be BUILT: `format::read_name` refuses the name
    /// before a signature is ever computed. If that stops being true, this fails
    /// — which matters, because [`plan`] would then be the only thing standing
    /// between an artifact and an arbitrary path on the machine.
    #[test]
    fn the_container_refuses_a_traversal_before_this_module_ever_sees_it() {
        for hostile in ["../../../etc/passwd", "..", ".", "a/b", "/absolute"] {
            let owned = members(Some(hostile));
            let borrowed: Vec<(&str, u8, u8, Vec<u8>)> = owned
                .iter()
                .map(|(n, c, v, b)| (n.as_str(), *c, *v, b.clone()))
                .collect();
            let built = packaged("photo-index", "0.1.0", Kind::application(), &borrowed);
            assert!(
                matches!(built, Err(PackageError::MalformedMemberName { .. })),
                "{hostile:?} was packaged rather than refused: {built:?}"
            );
        }
    }

    /// A version that would collide with `current` or `data` is refused rather
    /// than replacing either — and the second of those would destroy every byte
    /// the application had ever written.
    #[test]
    fn a_version_that_is_a_reserved_name_or_not_a_path_component_is_refused() {
        // Every one of these is printable ASCII, so `format::read_text` — which
        // is what a header field goes through, and which is looser than
        // `read_name` — hands them straight here.
        for hostile in [
            CURRENT_LINK,
            DATA_DIR,
            STAGED_LINK,
            "..",
            ".",
            "1.0/../..",
            "a\\b",
            // The supervisor's own names in an application's directory: the
            // scratch directory, and the three files it writes beside `data`.
            "tmp",
            "settings",
            "routes",
            "dns.toml",
        ] {
            let package = app("photo-index", hostile, None);
            let refusal = plan(service::BUILT_IN_VOLUME, &package).expect_err(&format!("version {hostile:?} was planned"));
            assert!(
                matches!(refusal, PackageError::UnusableVersion { .. }),
                "{hostile:?}: {refusal:?}"
            );
        }
    }

    /// A product that could not be a service name is refused, because a name is
    /// a cgroup and a socket as well as a directory.
    #[test]
    fn a_product_that_could_not_be_a_service_name_is_refused() {
        // Printable ASCII, so the header accepts them; a name is a cgroup and a
        // socket as well as a directory, so `ServiceName` does not.
        for hostile in ["../evil", ".hidden", "-leading-dash", "a/b", "a\\b"] {
            let package = app(hostile, "0.1.0", None);
            let refusal = plan(service::BUILT_IN_VOLUME, &package).expect_err(&format!("product {hostile:?} was planned"));
            assert!(
                matches!(refusal, PackageError::UnusableName { .. }),
                "{hostile:?}: {refusal:?}"
            );
        }
    }

    /// A system update is a valid, signed, trusted artifact and is not something
    /// this installs. The refusal names which it is and which was wanted.
    #[test]
    fn a_system_update_handed_to_the_application_installer_is_refused_by_kind() {
        let package = packaged(
            "rivet",
            "0.0.2",
            Kind::update(),
            &[("system", 0u8, 1u8, b"not really an image".to_vec())],
        )
        .expect("a valid update package");
        let refusal = plan(service::BUILT_IN_VOLUME, &package).expect_err("an update was planned as an application");
        let PackageError::WrongKindForInstall { kind, wanted } = &refusal else {
            panic!("{refusal:?}");
        };
        assert_eq!(kind, "update");
        assert_eq!(wanted, "application");
        assert!(refusal.to_string().contains("update"), "{refusal}");
    }
}
