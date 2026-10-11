//! **The gate every file-management subject passes, and the vocabulary they
//! share.** `ADR-006` §8.10.4, ruled by the maintainer, Joel Lindell,
//! 2026-08-29.
//!
//! This module implements no operation. It holds the checks that are the same
//! for `apply create-file`, `create-directory`, `set-mode`, `set-owner`,
//! `rename`, `remove-file` and `remove-directory` — and it holds them once,
//! because eight copies of *"is this path under `/state`"* is eight chances for
//! one of them to be wrong in the direction that deletes something.
//!
//! # What is shared here and what is deliberately NOT
//!
//! **Shared: the facts.** Whether a path is absolute, whether it carries a
//! control character, whether it has a `.` or `..` component, whether it is
//! under the permitted root, whether it is a view, a mechanism's, or
//! load-bearing. A fact has one answer and `ADR-011` says it has one owner.
//!
//! **Not shared: the sentences.** Each subject words its own refusal, because a
//! refusal has to tell an operator what to do *next* and the next thing differs:
//! *"name the file inside it"* for a directory given to `remove-file`, *"that is
//! `apply edit`"* for a path that already exists given to `create-file`. A
//! generic refusal would be the generic operation `ADR-006` §8.10.2 refuses,
//! arriving through the back door.
//!
//! **`apply edit` keeps its own gate and is not narrowed by this one.** §8.10.2
//! item 2: `edit` guards itself with a deny-list, which is right for it, and its
//! refusals were proven against a running guest on 2026-08-24. What it shares
//! with this module is `control_character` — the one predicate whose answer
//! could not sensibly differ — and nothing else.
//!
//! # The allow-list, and why it is an allow-list
//!
//! An edit is reversible in the sense that matters: `--expect` means the caller
//! had to be right about the old text, and the new text is visible and can be
//! edited back. **None of that is true of a delete.** There is no trash on this
//! machine, no snapshot, no undo. A deny-list is only ever as good as its
//! enumeration and the failure mode of a missing entry here is not a wrong
//! answer, it is a file that is gone.
//!
//! So the mutating subjects refuse everything outside
//! [`PERMITTED_ROOT`](filesystem::PERMITTED_ROOT), and
//! the root is `ADR-018` §4's own division rather than a convenience: `/system`
//! is read-only vendor content replaced whole by an update, `/etc` is a
//! projection remounted read-only, the boot partition is FAT the firmware reads,
//! and `/run`, `/proc`, `/sys` and `/dev` are kernel and supervisor surfaces.
//! `/state` is the only writable partition a Rivet card carries.
//!
//! # Reads pass a shorter gate, on purpose
//!
//! `rivet get file` and `rivet get diff` pass `check_readable_path` — absolute,
//! no control characters, no `.`/`..` — and **not** the root, view, mechanism or
//! load-bearing checks. Reading `/system/bin/rivet-sysd` is exactly the question
//! this surface exists to answer, and refusing it would make the surface less
//! able to answer questions without making anything safer. What `get file` does
//! instead is **report** that a path is a view, in `explain`'s own words.

use super::machine::{MachineError, PathStat};
use super::{ErrorCode, OperationError};
use crate::etc::ETC_MOUNT_POINT;
use crate::state::{CONFIG_DIR, MACHINE_DOCUMENT, STATE_MOUNT_POINT};

/// **The one root the mutating and destructive subjects may touch.**
///
/// `crate::state::STATE_MOUNT_POINT` rather than a literal, so there is no
/// second spelling of it to go stale (`ADR-011`). See the module documentation
/// for why it is an allow-list of exactly one.
pub const PERMITTED_ROOT: &str = STATE_MOUNT_POINT;

/// How many directory entries `rivet get file` will name.
///
/// **256.** The number is about what an answer *is* rather than about memory: a
/// listing longer than a screen is not a listing anybody reads, and a caller who
/// needs all of them needs a different question than *"what is in this
/// directory"*. What matters more than the number is that the answer says it was
/// cut — [`PathStat::entries_truncated`] — because a truncated list that did not
/// say so is the same lie as a wrong one.
pub const DIRECTORY_MAX_ENTRIES: usize = 256;

// ---------------------------------------------------------------------------
// The predicates. Facts, not sentences.
// ---------------------------------------------------------------------------

/// **A sentence about the caller's SHELL, when the path shows its fingerprints.**
///
/// # The failure this exists for, which cost real time on 2026-08-29
///
/// A Windows operator types `rivet run … -- get file /state/secrets/peers` in
/// Git Bash and the machine is asked about
/// `C:/Program Files/Git/state/secrets/peers`. **MSYS rewrote the argument
/// before this program ever saw it** — it translates anything shaped like a
/// Unix path into a Windows one, and an absolute path naming a directory on the
/// BOARD looks exactly like a path it should translate.
///
/// The refusal that follows is correct and was already well argued: the path it
/// received is not absolute and would resolve somewhere nobody chose. **What it
/// could not say was WHY**, and the operator is then debugging a path they did
/// not type. `ADR-012` §7.4 asks a refusal to carry its remedy, and this is a
/// remedy the machine can name from the evidence in front of it.
///
/// # It reports a shape, never a platform
///
/// This crate has no platform (`ADR-014` §3.1) and the *board* is Linux —
/// `rivet` runs there too, and there a path beginning `C:/` is simply a bad
/// path. So this adds a sentence and never changes the verdict: the refusal is
/// identical, with one more clause when the argument carries a drive letter and
/// a translated prefix. **A machine that guessed at the caller's operating
/// system would be wrong for every operator who is not on one.**
fn msys_hint(path: &str) -> String {
    let looks_translated = path.len() > 2
        && path.as_bytes()[0].is_ascii_alphabetic()
        && path[1..].starts_with(":/")
        || path.starts_with("\\\\");
    if !looks_translated {
        return String::new();
    }
    " That path carries a DRIVE LETTER, which this argument should not have: a \
     Unix-shell-on-Windows (MSYS, Git Bash) rewrites an absolute path into a Windows one \
     before the program is started, and a path on the MACHINE looks exactly like one it \
     should translate. If that is what happened, the argument was translated and never \
     typed — set `MSYS_NO_PATHCONV=1` for the command, and the path reaches this machine \
     as written."
        .to_string()
}

/// The first control character in `path`, as `(offset, byte)`.
///
/// **Shared with `super::edit`, which is the one predicate this module shares
/// with it.** A newline is the one that matters: a plan and a journal record are
/// both lines, and a path that breaks one of them describes an act that did not
/// happen.
pub(super) fn control_character(path: &str) -> Option<(usize, u8)> {
    path.bytes()
        .enumerate()
        .find(|(_, b)| b.is_ascii_control())
        .map(|(at, b)| (at, b))
}

/// The first `.`, `..` or empty component of `path`, if there is one.
///
/// **This is what makes the root check mean anything.**
/// `/state/../system/bin/rivet-sysd` passes a prefix test against `/state` and
/// is not under `/state`. An empty component — a `//` — is refused with them
/// because it is the same class of thing: a path with two spellings, only one of
/// which any check here would have seen.
///
/// **Shared with `super::edit`, on [`control_character`]'s footing exactly.**
/// That operation decides whether new content must parse as the machine
/// document by comparing a path's text to one constant, and a `.`/`..`/`//`
/// spelling of the same file would reach that comparison having skipped it.
pub(super) fn traversal_component(path: &str) -> Option<&'static str> {
    // The root is one `/` and its only component is the empty one that `/` is
    // made of. It is a real path and not a `//`.
    if path == "/" {
        return None;
    }
    for component in path.split('/').skip(1) {
        match component {
            "." => return Some("."),
            ".." => return Some(".."),
            "" => return Some("an empty component (`//`)"),
            _ => {}
        }
    }
    None
}

/// Whether `path` is `root` or lies under it, **component-wise**.
///
/// `starts_with` alone would put `/statement` under `/state`, which is the
/// classic form of this bug and the reason this is a function rather than an
/// expression at each site.
pub(super) fn under(path: &str, root: &str) -> bool {
    path == root || (path.starts_with(root) && path.as_bytes().get(root.len()) == Some(&b'/'))
}

/// The directory component of an absolute path. `/` for a path directly under
/// the root.
///
/// Purely textual, and the caller has already been refused a relative path, a
/// trailing `/` and a traversal component — so there is always a `/` and it is
/// never the last byte.
pub fn directory_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) | None => "/",
        Some(at) => &path[..at],
    }
}

// ---------------------------------------------------------------------------
// The load-bearing set
// ---------------------------------------------------------------------------

/// One path inside `/state` that the mutating subjects still refuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadBearing {
    /// The exact path, or the directory whose contents are covered too.
    pub path: &'static str,
    /// `true` when everything **under** [`Self::path`] is covered.
    pub tree: bool,
    /// What is lost, in a clause that reads after *"because"*.
    pub costs: &'static str,
}

/// **Paths under `/state` whose removal or renaming takes the machine away from
/// anybody who could put it back.** `ADR-006` §8.10.4.2.
///
/// This is a **third** kind of refusal and not a restatement of the other two.
/// A *view* (`super::explain`) says *this is generated, change the source*; a
/// *mechanism path* (`super::edit::reserved_paths`) says *something consumes
/// this, use the mechanism*; this one says *the machine needs this and you would
/// not be able to put it back*.
///
/// **`apply edit` is not bound by this list and must not be.** Changing
/// `/state/config/machine.toml` is the single most useful thing an operator
/// does; what is refused here is *removing* or *renaming* it, which is not a
/// change to the document but the absence of one.
///
/// **What is deliberately NOT here, said rather than left to be inferred:**
///
/// * **`/state/app` and `/state/log`** are already `super::edit::reserved_paths`
///   entries and are refused as mechanism paths, with the better sentence. A
///   second entry here would be `ADR-011` §3 in the small.
/// * **The update payloads themselves** — `/state/*.rvt`. They are the reason
///   this whole section exists (§8.10.1) and nothing may refuse them.
pub fn load_bearing() -> Vec<LoadBearing> {
    vec![
        LoadBearing {
            path: PERMITTED_ROOT,
            tree: false,
            costs: "it is the mount point of the only writable partition this machine has \
                    (ADR-018 §4) — everything else here is inside it",
        },
        LoadBearing {
            path: CONFIG_DIR,
            tree: true,
            costs: "it holds the desired-state document, which is WHAT THIS MACHINE IS \
                    (ADR-006 §6.4). A machine without one converges to nothing, and no verb in \
                    this build writes a new one — §6.4's `set` does not exist. `rivet apply \
                    edit` may CHANGE the document; removing it is not a change, it is the \
                    absence of one",
        },
    ]
}

/// The load-bearing entry a path falls under, if any.
fn load_bearing_for(path: &str) -> Option<LoadBearing> {
    load_bearing()
        .into_iter()
        .find(|entry| entry.path == path || (entry.tree && under(path, entry.path)))
        // The document is named explicitly so its own sentence survives even if
        // the directory entry above it is ever narrowed. Belt and braces on the
        // one file whose loss cannot be undone from the machine itself.
        .or_else(|| {
            (path == MACHINE_DOCUMENT).then_some(LoadBearing {
                path: MACHINE_DOCUMENT,
                tree: false,
                costs: "it IS the desired-state document (ADR-006 §6.4)",
            })
        })
}

// ---------------------------------------------------------------------------
// The application tree: three zones, and the surface must tell them apart
// ---------------------------------------------------------------------------

/// **Where under `ADR-015`'s application layout a path falls.**
///
/// Ruled by the maintainer, 2026-08-29, and it is `ADR-006` §8.10.4.4:
///
/// > permit, and inform — do not refuse.
///
/// **Sandboxing is not the constraint.** `ADR-015` §5's sandbox is a mount
/// namespace the *supervisor* puts an application in; the administration
/// surface runs as root and outside it. What actually differs between these
/// paths is **trust and persistence**, and a blanket refusal on the application
/// tree would recreate the gap `ADR-006` §8.10 exists to close: the control
/// plane is the only channel, and what was once done by hand has to be doable
/// by the surface.
///
/// So the tree is **permitted**, three of its paths are not, and every write
/// into it carries the consequence in its plan. `ADR-012` §7.1's
/// plan-before-apply is what does the work: an operator who is about to lose an
/// edit at the next update should learn it from the plan and not six weeks
/// later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppZone {
    /// `/state/app` itself, `/state/app/<name>`, or `/state/app/<name>/current`
    /// — **refused**, and each names the mechanism that owns it.
    Mechanism(&'static str),
    /// `/state/app/<name>/<version>/bin/…` — **the program**, whose digest a
    /// signed manifest covers and which the supervisor re-verifies at **every
    /// launch** (`ADR-015` §4.2, §4.2.1).
    Program,
    /// `/state/app/<name>/<version>/manifest.toml` — the signed record the
    /// launch check verifies against.
    Manifest,
    /// Anything else under `/state/app/<name>/<version>/` — not covered by a
    /// digest, and **replaced wholesale by the next update** (`ADR-015` §7).
    Version,
    /// `/state/app/<name>/data/…` — the application's own writable state, mode
    /// 0700, owned by its identity (`ADR-015` §5.1.1). **It survives a version
    /// swap**, and it is the one place in the tree a hand change persists.
    Data,
    /// `/state/app/<name>/settings` — the view of the document's
    /// `[applications.<name>]` table the supervisor writes (`crate::settings`).
    /// Beside the version directories, so it survives an update, and written
    /// again whenever the supervisor delivers settings.
    Settings,
    /// `/state/app/<name>/routes` — the proxy's routing table, which the
    /// supervisor writes at boot and at each finished install and uninstall
    /// (`crate::proxy`). Beside the version directories, and rewritten by the
    /// next of those.
    Routes,
    /// `/state/app/<name>/dns.toml` — the name service's projection of the
    /// document's `[dns]` table, which the supervisor writes wherever it
    /// delivers settings (`crate::name_service`, `ADR-047` §6).
    NameService,
    /// Anything else in the home of an image service that runs under its own
    /// identity (`crate::service::IMAGE_SERVICE_HOMES`, `ADR-060`). Such a home
    /// has no version directories, because the program is on the system slot,
    /// so an update replaces the program and leaves the home as it is.
    ImageService,
    /// `/state/app/<name>/tmp/…` — the application's scratch directory, which
    /// the supervisor empties before every start and after every exit
    /// (`crate::service::tmp_dir`).
    Scratch,
}

impl AppZone {
    /// The clause a plan carries, or `None` where there is nothing to say.
    ///
    /// **Written for the moment of decision.** It says what happens *later*,
    /// because what happens now is what the operator already asked for and the
    /// thing they cannot see is the consequence.
    pub fn note(self) -> Option<String> {
        match self {
            Self::Mechanism(_) => None,
            Self::Program => Some(
                "THIS IS AN APPLICATION'S PROGRAM. A signed manifest beside it carries its \
                 digest and the supervisor re-checks that digest at EVERY LAUNCH (ADR-015 \
                 §4.2.1) — so a changed program DOES NOT START, and it also shows as drift \
                 against the digest recorded in the desired-state document. Nothing fails at \
                 the moment of the change; the service fails the next time it is started. \
                 THE WAY TO CHANGE A PROGRAM IS `rivet apply install <FILE.rvt> confirm` with \
                 a rebuilt package."
                    .to_string(),
            ),
            Self::Manifest => Some(
                "THIS IS AN APPLICATION'S SIGNED MANIFEST — the record the launch-time digest \
                 check verifies against (ADR-015 §4.2.1). Changing it does not make a changed \
                 program acceptable, because the signature is over the manifest itself; what \
                 it does is stop the application starting at all. Nothing fails at the moment \
                 of the change."
                    .to_string(),
            ),
            Self::Version => Some(
                "THIS IS INSIDE AN INSTALLED VERSION'S DIRECTORY, and no signature covers it — \
                 so nothing will refuse this change and nothing will notice it. WHAT WILL \
                 HAPPEN IS THAT THE NEXT UPDATE DISCARDS IT: ADR-015 §7 installs each version \
                 into its own directory and swaps a `current` link, so this whole directory is \
                 replaced and never merged. A change that must survive an update belongs in \
                 the application's `data` directory, or — if it is declarative — in the \
                 desired-state document, which is the single source of authority (ADR-011)."
                    .to_string(),
            ),
            Self::Data => Some(
                "This is the application's own `data` directory (ADR-015 §5.1.1): mode 0700, \
                 owned by that application's identity, OUTSIDE the `current` link, and it \
                 survives an update. A change here persists."
                    .to_string(),
            ),
            Self::Settings => Some(
                "THIS IS THE VIEW OF THE DESIRED-STATE DOCUMENT'S `[applications.<name>]` TABLE \
                 that the supervisor writes for this application (ADR-038 §8). It sits beside the \
                 version directories, so an update leaves it, but the supervisor writes it again \
                 whenever it delivers settings, so a hand change lasts only until then. The \
                 document is the authority (ADR-011): change the setting there."
                    .to_string(),
            ),
            Self::Routes => Some(
                "THIS IS THE PROXY'S ROUTING TABLE, which the supervisor writes from the installed \
                 manifests at boot and at every finished install and uninstall (ADR-046 §5). An \
                 update leaves it, but the next install or uninstall rewrites it, so a hand change \
                 lasts only until then."
                    .to_string(),
            ),
            Self::NameService => Some(
                "THIS IS THE NAME SERVICE'S PROJECTION OF THE DESIRED-STATE DOCUMENT'S `[dns]` \
                 TABLE, which the supervisor writes (ADR-047 §6). An update leaves it, but the \
                 supervisor writes it again at boot, at every finished install and at a reload, so \
                 a hand change lasts only until then. The document is the authority (ADR-011): \
                 change `[dns]` there."
                    .to_string(),
            ),
            Self::ImageService => Some(
                "This is in the home of an image service (ADR-060): part of the operating system, \
                 run under its own identity from a home laid out like an application's, but with \
                 no version directories, because its program is on the system slot. An update \
                 replaces that program and leaves this directory as it is, so a change here \
                 persists."
                    .to_string(),
            ),
            Self::Scratch => Some(
                "This is the application's scratch directory, its TMPDIR: the supervisor empties \
                 it immediately before every start and after every exit, so nothing written here \
                 persists."
                    .to_string(),
            ),
        }
    }
}

/// Which [`AppZone`] a path is in, or `None` when it is not in the application
/// tree at all.
///
/// **Purely textual, like every other check in this module.** It reads the
/// layout `crate::service` declares — `<root>/<name>/<version>/bin/…`,
/// `<root>/<name>/data/…`, `<root>/<name>/current` — and names nothing this
/// crate does not already own.
pub fn app_zone(path: &str) -> Option<AppZone> {
    use crate::service::{APP_ROOT, BIN_DIR, CURRENT_LINK, DATA_DIR, TMP_DIR};

    if !under(path, APP_ROOT) {
        return None;
    }
    let rest: Vec<&str> = path[APP_ROOT.len()..]
        .split('/')
        .filter(|c| !c.is_empty())
        .collect();

    match rest.as_slice() {
        [] => Some(AppZone::Mechanism(
            "the installer, which makes one directory per installed application under it \
             (ADR-015 §4.2). There is no operation that removes the whole application root and \
             a machine without one cannot start anything it has installed",
        )),
        [name] if crate::service::image_service_home(name).is_some() => Some(AppZone::Mechanism(
            "the home of an image service (ADR-060) — part of the operating system, not an \
             installed application, so no uninstall removes it. The supervisor makes it, \
             root-owned, and the service runs as its own identity inside it",
        )),
        [_name] => Some(AppZone::Mechanism(
            "one installed application, whole — its versions, its program and its private data \
             (ADR-015 §3.2). `rivet apply uninstall <name> confirm` is how an application \
             leaves this machine; it stops the service first, which nothing here does",
        )),
        [_name, link] if *link == CURRENT_LINK => Some(AppZone::Mechanism(
            "the installer's ATOMIC SWAP (ADR-015 §7): this link is what makes an install a \
             rename rather than a copy, and it is the machine's only statement of which \
             version is live. A link changed by hand makes a machine run a version nothing \
             recorded",
        )),
        [_name, link, ..] if *link == CURRENT_LINK => Some(AppZone::Mechanism(
            "the installer's `current` link (ADR-015 §7). A path THROUGH the link is a path \
             whose meaning changes at the next install, and this surface does not follow links \
             (ADR-006 §8.10.4.3). Name the version directory itself",
        )),
        [_name, directory, ..] if *directory == DATA_DIR => Some(AppZone::Data),
        // **The three siblings of `data/` the supervisor keeps**, beside the
        // version directories rather than in one. Matched before the version
        // fallback below, which would describe each as discarded by an update.
        [_name, file] if *file == crate::settings::FILE => Some(AppZone::Settings),
        [_name, file] if *file == crate::proxy::FILE => Some(AppZone::Routes),
        [_name, file] if *file == crate::name_service::FILE => Some(AppZone::NameService),
        [_name, directory, ..] if *directory == TMP_DIR => Some(AppZone::Scratch),
        // **An image service's home has no version directories** (`ADR-060`),
        // so the three arms below, which read the second component as a
        // version, would describe its files as an application's and say an
        // update discards them. Matched before them for that reason.
        [name, ..] if crate::service::image_service_home(name).is_some() => {
            Some(AppZone::ImageService)
        }
        [_name, _version, directory, ..] if *directory == BIN_DIR => Some(AppZone::Program),
        [_name, _version, file] if *file == crate::package::application::MANIFEST_NAME => {
            Some(AppZone::Manifest)
        }
        _ => Some(AppZone::Version),
    }
}

/// The clause a plan carries about where in the application tree a path is, or
/// an empty string.
///
/// Appended by every writing subject to the note on its effect, so the answer to
/// *"will this survive an update"* is in the plan an operator reads rather than
/// in a document they would have to already know to look for.
pub fn app_zone_clause(path: &str) -> String {
    app_zone(path)
        .and_then(AppZone::note)
        .map_or_else(String::new, |note| format!(" {note}"))
}

/// **The exceptional consequence of writing to this path, if there is one.**
///
/// `ADR-012` §7.3's second confirmation tier, and the doctrine that decides
/// which paths reach it — the maintainer, 2026-08-29 (`.rulings/` R-030): the
/// WARNING tier is reserved for the exceptional case, the operating system's
/// chance to object before the operator hurts themselves.
///
/// **Exactly two paths in the file surface reach it**, and the shortness of the
/// list is the whole of its value. `PLAN.md` §94.2 found `ADR-029` §9.3's F5
/// warning firing on every install targeting slot A and concluded it *"became
/// wallpaper"*; a second tier that fires often is worth exactly as much as the
/// first.
///
/// | Path | Warning? | Why |
/// | :-- | :-- | :-- |
/// | an application's **program** | **yes** | the machine stops working, and it stops **later**, at a launch nobody is watching |
/// | an application's **manifest** | **yes** | same effect by a different route: the launch check has nothing valid to verify against |
/// | anything else in a version directory | **no** | nothing is hosed. The edit is discarded at the next update, which is the plan's PROSE — `ADR-006` §8.10.4.4 |
/// | a stale `.rvt` in `/state` | **no** | it is the routine case §8.10.1 exists to enable, and friction there would defeat the ruling |
/// | a `data` file | **no** | nothing exceptional happens |
///
/// The line the two `yes` rows share: **the harm is not undone by repeating
/// what you just did.** A discarded edit is retyped and a deleted payload is
/// re-deployed; a program that no longer matches its signed digest needs a
/// rebuilt package from a workstation, which on a headless board at `ADR-035`
/// phase 5 may be a long way away.
pub fn hazard_for(path: &str) -> Option<super::apply::Hazard> {
    match app_zone(path)? {
        AppZone::Program => Some(super::apply::Hazard {
            phrase: "the application will not start until it is reinstalled".to_string(),
            because: "a signed manifest beside this program carries its digest and the \
                      supervisor re-checks that digest at EVERY LAUNCH (ADR-015 §4.2.1), so a \
                      changed program does not run — and NOTHING FAILS NOW: the service fails \
                      the next time something starts it, which on a headless board may be at \
                      the next reboot and unattended."
                .to_string(),
        }),
        AppZone::Manifest => Some(super::apply::Hazard {
            phrase: "the application will not start until it is reinstalled".to_string(),
            because: "this is the SIGNED MANIFEST the launch-time digest check verifies against \
                      (ADR-015 §4.2.1). Changing it cannot make a changed program acceptable — \
                      the signature is over the manifest itself — so what it does is stop the \
                      application starting at all, and NOTHING FAILS NOW."
                .to_string(),
        }),
        AppZone::Mechanism(_)
        | AppZone::Version
        | AppZone::Data
        | AppZone::Settings
        | AppZone::Routes
        | AppZone::NameService
        | AppZone::ImageService
        | AppZone::Scratch => None,
    }
}

// ---------------------------------------------------------------------------
// The words after the subject
// ---------------------------------------------------------------------------

/// What the tail of a file-management command said.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct Tail<'a> {
    /// The text of [`super::apply::ACCEPT_FLAG`], if it was given.
    pub accept: Option<&'a str>,
    /// Whether the literal [`super::apply::CONFIRMATION`] was the last word.
    pub confirmed: bool,
}

/// **Read the words after a subject's own arguments**, accounting for every
/// one.
///
/// `from` is the first index this reads. One implementation for the six writing
/// subjects, because the tail is the same for all of them and six copies of
/// *"the only bare word is `confirm` and it is last"* is six chances to get the
/// `confrim` rule wrong in one of them.
///
/// **An unknown word is refused by name rather than skipped**, which is
/// `super::apply::confirmed`'s rule: an operator shown a plan again after a
/// typo would repeat the typo.
pub(super) fn read_tail<'a>(
    subject: &str,
    forms: &str,
    words: &'a [&'a str],
    from: usize,
) -> Result<Tail<'a>, OperationError> {
    use super::apply::{ACCEPT_FLAG, CONFIRMATION};

    let mut tail = Tail::default();
    let mut confirmation: Option<&str> = None;
    let mut at = from;
    while at < words.len() {
        let word = words[at];
        if word == ACCEPT_FLAG {
            if tail.accept.is_some() {
                return Err(arguments(
                    word,
                    format!(
                        "`{ACCEPT_FLAG}` is given twice. There is one warning to accept, so \
                         reading the second as the one that counts would silently discard the \
                         first."
                    ),
                ));
            }
            let Some(text) = words.get(at + 1).copied() else {
                return Err(arguments(
                    word,
                    format!(
                        "`{ACCEPT_FLAG}` needs the warning's own words after it, and this is the \
                         last word. Leave the command unconfirmed to see them."
                    ),
                ));
            };
            if text == CONFIRMATION || text == ACCEPT_FLAG {
                return Err(arguments(
                    text,
                    format!(
                        "`{ACCEPT_FLAG}` is followed by `{text}`, which is one of this command's \
                         own words rather than the warning's text — so `{ACCEPT_FLAG}` has no \
                         value and `{text}` has been eaten."
                    ),
                ));
            }
            tail.accept = Some(text);
            at += 2;
            continue;
        }

        if word.starts_with('-') {
            return Err(arguments(
                word,
                format!("`{subject}` takes {ACCEPT_FLAG} and nothing else. {forms}."),
            ));
        }

        // The only bare word is the confirmation, and it is the LAST word.
        if at + 1 != words.len() {
            return Err(arguments(
                word,
                format!(
                    "`{word}` is a word `{subject}` has no meaning for. The only bare word it \
                     takes is `{CONFIRMATION}`, as the LAST word. {forms}."
                ),
            ));
        }
        confirmation = Some(word);
        at += 1;
    }

    tail.confirmed = super::apply::confirmed(confirmation)?;
    Ok(tail)
}

/// The command a plan says to repeat, as words `Plan::confirmed_by` splices.
///
/// **The acknowledgement is part of it when there is one**, so the line an
/// operator pastes back is complete and they never have to compose the phrase
/// themselves from prose. `ADR-012` §7.3's token is meant to require *reading*
/// the plan, not transcribing it.
pub(super) fn confirmation_words(
    typed: &[&str],
    hazard: Option<&super::apply::Hazard>,
) -> Vec<String> {
    let mut words: Vec<String> = typed
        .iter()
        .filter(|word| **word != super::apply::CONFIRMATION)
        .map(|word| (*word).to_string())
        .collect();
    if let Some(hazard) = hazard {
        // Whatever the caller typed for the acknowledgement, the plan shows the
        // one that would be accepted.
        if let Some(at) = words.iter().position(|w| w == super::apply::ACCEPT_FLAG) {
            words.truncate(at);
        }
        words.extend(hazard.accept_words());
    }
    words
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// An argument refusal for a file-management subject.
pub(super) fn arguments(subject: &str, message: impl Into<String>) -> OperationError {
    OperationError::about(ErrorCode::Arguments, subject.to_string(), message)
}

/// An `E_UNSAFE` refusal that names the path.
pub(super) fn unsafe_for(path: &str, message: impl Into<String>) -> OperationError {
    OperationError::about(ErrorCode::Unsafe, path.to_string(), message)
}

/// The one place a [`MachineError`] becomes one of these subjects' refusals.
///
/// The same mapping `super::edit` uses, for the same reason — an operator does
/// two different things next — and **nothing is reworded on the way through**.
pub(super) fn refused(error: MachineError, path: &str) -> OperationError {
    let code = match error {
        MachineError::Unsafe(_) => ErrorCode::Unsafe,
        MachineError::Refused(_) => ErrorCode::MachineRefused,
    };
    OperationError::about(code, path.to_string(), error.why())
}

/// **G1–G3: everything true of a path that a READ subject still refuses.**
///
/// `verb_and_subject` is spelled as the operator typed it — `get file` — so the
/// refusal names the command rather than a module.
pub(super) fn check_readable_path(
    verb_and_subject: &str,
    path: &str,
) -> Result<(), OperationError> {
    if path.is_empty() {
        return Err(arguments(
            path,
            format!("`{verb_and_subject}` names no path at all. A path is named, in full."),
        ));
    }

    if !path.starts_with('/') {
        return Err(arguments(
            path,
            format!(
                "a path is named ABSOLUTELY. `{path}` is relative, and this process's working \
                 directory is whatever the session happened to start in — so it would resolve \
                 somewhere neither you nor `{verb_and_subject}` chose. The failure mode is the \
                 bad one: it resolves to SOMETHING often enough that the answer would name a \
                 path you never typed.{}",
                msys_hint(path)
            ),
        ));
    }

    if let Some((at, byte)) = control_character(path) {
        return Err(arguments(
            path,
            format!(
                "this path carries a control character — byte {byte:#04x} at offset {at} — and \
                 `{verb_and_subject}` will not report on a path it cannot print on one line. A \
                 newline is the one that matters: a plan and a journal record are both lines. \
                 Rivet creates no such names, so this is almost always a quoting mistake in the \
                 command that produced it."
            ),
        ));
    }

    // **Before the traversal check, because a trailing `/` is also an empty
    // component** and this is the sentence an operator can act on. The order is
    // the more-specific-refusal-wins rule `super::edit` learned on 2026-08-25.
    if path.len() > 1 && path.ends_with('/') {
        return Err(arguments(
            path,
            format!(
                "this path ends in a `/`, which is a second spelling of the same directory. \
                 `{verb_and_subject}` takes one spelling — drop the trailing slash."
            ),
        ));
    }

    if let Some(component) = traversal_component(path) {
        return Err(arguments(
            path,
            format!(
                "this path has a `{component}` in it, and `{verb_and_subject}` reads a path \
                 LITERALLY — it resolves nothing. THIS IS NOT PEDANTRY: the writing subjects of \
                 ADR-006 §8.10 decide what they may touch by testing this string against \
                 {PERMITTED_ROOT}, and `..` walks straight out of it. One spelling per path, so \
                 that a check on the spelling is a check on the path. Say the path you mean."
            ),
        ));
    }

    Ok(())
}

/// **G1–G7: the whole gate a WRITING subject passes**, before anything is read
/// and before any machine is asked.
///
/// G8 — a symbolic link on the way to the path — is **not** here and cannot be:
/// it is a fact about the machine, so it is checked by
/// [`super::machine::Editor::can_change_path`] in the adapter, on the plan path
/// and again at the point of use.
///
/// The order is `ADR-006` §8.10.4's and it is part of the design: the more
/// specific refusal wins, so a mechanism path names its mechanism rather than
/// getting the generic view sentence — `super::edit`'s ordering defect of
/// 2026-08-25, not repeated.
pub(super) fn check_writable_path(
    verb_and_subject: &str,
    path: &str,
) -> Result<(), OperationError> {
    check_readable_path(verb_and_subject, path)?;

    // G4. Before the view and mechanism checks, because it is the check that
    // holds when those two are incomplete — which is the entire argument for
    // an allow-list.
    if !under(path, PERMITTED_ROOT) {
        return Err(unsafe_for(
            path,
            format!(
                "REFUSED `{verb_and_subject}` changes nothing outside {PERMITTED_ROOT}, and \
                 NOTHING HAS BEEN CHANGED. {PERMITTED_ROOT} is the only writable partition a \
                 Rivet card carries (ADR-018 §4): {ETC_MOUNT_POINT} is a projection remounted \
                 read-only (ADR-007 §3.5.1), /system is read-only vendor content that the next \
                 update replaces whole, the boot partition is FAT the firmware reads, and /run, \
                 /proc, /sys and /dev are the kernel's and the supervisor's. THIS IS AN \
                 ALLOW-LIST AND NOT AN OVERSIGHT (ADR-006 §8.10.4.1): these subjects destroy \
                 and move things, there is no trash on this machine, and a deny-list is only \
                 ever as good as its enumeration."
            ),
        ));
    }

    // G5 and G6, in `super::edit`'s order: the specific refusal before the
    // generic one. Both borrow that module's tables rather than restating them,
    // so a path cannot be a mechanism's here and a view's there.
    super::edit::check_not_reserved(path)?;
    if let Some(refusal) = super::edit::view_refusal(path) {
        return Err(refusal);
    }

    // G7. Last, because it is the narrowest and its sentence is the one worth
    // reading when the others have all passed.
    if let Some(entry) = load_bearing_for(path) {
        let what = if entry.tree && entry.path != path {
            format!("{path} is inside {} — which", entry.path)
        } else {
            path.to_string()
        };
        return Err(unsafe_for(
            path,
            format!(
                "REFUSED {what} is LOAD-BEARING on this machine and NOTHING HAS BEEN CHANGED, \
                 because {costs}. This is a different refusal from a view and from a \
                 mechanism's path: it is not that something else owns this, it is that the \
                 machine needs it and you would have no way to put it back — on a headless \
                 board whose only channel is the control plane, that is the end of the \
                 conversation.",
                costs = entry.costs,
            ),
        ));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Rendering, so that a plan and an answer say the same thing
// ---------------------------------------------------------------------------

/// A mode as `rwxr-x---`, beside the octal a plan already carries.
///
/// **Both forms, always.** An operator reads one of them fluently and an agent
/// branches on the other, and which is which differs by reader — so a plan that
/// carried only the octal would be legible to half its audience at the moment it
/// is being confirmed.
pub fn render_mode(mode: u32) -> String {
    const BITS: [(u32, char); 9] = [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ];
    let mut out = String::with_capacity(9);
    for (bit, letter) in BITS {
        out.push(if mode & bit == 0 { '-' } else { letter });
    }
    // The three that are never set by anything here, shown when they are,
    // because a mode carrying one and rendering as though it did not would hide
    // exactly the bit an operator most needs to see (`ADR-006` §8.10.9).
    let mut extra = Vec::new();
    if mode & 0o4000 != 0 {
        extra.push("setuid");
    }
    if mode & 0o2000 != 0 {
        extra.push("setgid");
    }
    if mode & 0o1000 != 0 {
        extra.push("sticky");
    }
    if extra.is_empty() {
        out
    } else {
        format!("{out} +{}", extra.join("+"))
    }
}

/// A mode as an operator reads it: `0644 (rw-r--r--)`.
pub fn describe_mode(mode: u32) -> String {
    format!("{mode:04o} ({})", render_mode(mode))
}

/// A uid or gid with its name where the machine knows one: `1000 (app)`.
///
/// **The number first and always.** A name is a label this machine looked up and
/// may not have; the number is what the filesystem holds. `?` is what an unknown
/// id renders as — visibly not a number, so nobody reads it as one.
pub fn describe_id(id: Option<u32>, name: Option<&str>) -> String {
    match (id, name) {
        (Some(id), Some(name)) => format!("{id} ({name})"),
        (Some(id), None) => format!("{id}"),
        (None, Some(name)) => format!("? ({name})"),
        (None, None) => "?".to_string(),
    }
}

/// One line describing what is at a path, for a plan's note.
///
/// Everything an operator needs to recognise *the thing they meant* — which is
/// the property `ADR-012` §7.1 is protecting: an operator who asked for the
/// wrong path must be able to tell from the plan alone.
pub fn describe_stat(stat: &PathStat) -> String {
    let Some(kind) = stat.kind else {
        return "nothing is at this path".to_string();
    };
    let mut parts = vec![format!("a {kind}")];
    if kind == super::machine::PathKind::File {
        parts.push(format!("{} bytes", stat.size_bytes));
    }
    if kind == super::machine::PathKind::Directory {
        parts.push(format!("{} entries", stat.entry_count));
    }
    if let Some(mode) = stat.mode {
        parts.push(format!("mode {}", describe_mode(mode)));
    }
    parts.push(format!(
        "owned by {}:{}",
        describe_id(stat.uid, stat.user.as_deref()),
        describe_id(stat.gid, stat.group.as_deref()),
    ));
    if let Some(target) = &stat.link_target {
        parts.push(format!("pointing at {target}"));
    }
    if stat.mount_point {
        parts.push("A MOUNT POINT".to_string());
    }
    parts.join(", ")
}

// ---------------------------------------------------------------------------
// One fake filesystem for the whole family
// ---------------------------------------------------------------------------

/// **A filesystem in a map, shared by every `ADR-006` §8.10 subject's tests.**
///
/// One fake and not seven, which is the same argument this module makes about
/// the gate: seven map-backed filesystems is seven chances for one of them to
/// answer differently about what a directory is, and a subject would then be
/// tested against a machine no other subject agrees with.
///
/// It is a whole [`super::machine::Board`] because `super::plan::resolve` takes
/// one; the other three ports **refuse** rather than pretending
/// (`AGENTS.md` §3), so a remove test that started rebooting the machine fails
/// loudly.
#[cfg(test)]
pub(super) mod fake {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::super::machine::{
        DirectoryEntry, Editor, FileRead, FileWindow, MachineError, OwnerIds, PathChange, PathKind,
        PathStat, Write, Wrote,
    };
    use crate::deploy::{Origin, Principal};
    use std::collections::BTreeMap;

    /// One thing at one path.
    #[derive(Debug, Clone)]
    pub(in crate::operation) struct Node {
        pub kind: PathKind,
        pub contents: Vec<u8>,
        pub mode: u32,
        pub uid: u32,
        pub gid: u32,
        pub modified_ms: Option<u64>,
        pub link_target: Option<String>,
        pub mount_point: bool,
    }

    impl Node {
        pub fn file(contents: impl AsRef<[u8]>) -> Self {
            Self {
                kind: PathKind::File,
                contents: contents.as_ref().to_vec(),
                mode: 0o644,
                uid: 0,
                gid: 0,
                modified_ms: Some(1_700_000_000_000),
                link_target: None,
                mount_point: false,
            }
        }

        pub fn directory() -> Self {
            Self {
                kind: PathKind::Directory,
                mode: 0o755,
                ..Self::file([])
            }
        }

        pub fn symlink(target: &str) -> Self {
            Self {
                kind: PathKind::Symlink,
                link_target: Some(target.to_string()),
                ..Self::file([])
            }
        }

        pub const fn moded(mut self, mode: u32) -> Self {
            self.mode = mode;
            self
        }

        pub const fn owned(mut self, uid: u32, gid: u32) -> Self {
            self.uid = uid;
            self.gid = gid;
            self
        }

        pub const fn mounted(mut self) -> Self {
            self.mount_point = true;
            self
        }
    }

    /// The machine every §8.10 test drives.
    pub(in crate::operation) struct FakeFiles {
        pub nodes: BTreeMap<String, Node>,
        /// What `change_path` was asked to do, as `act path`, in order.
        pub did: Vec<String>,
        /// What `can_change_path` was asked about, in order. **The interlock
        /// must run on the plan path**, and this is what proves it did.
        pub asked: Vec<String>,
        /// A refusal `can_change_path` and `change_path` both give.
        pub interlock: Option<MachineError>,
        /// A refusal only the act itself gives.
        pub act_fails: Option<MachineError>,
        /// What `Wrote::unrecorded` carries.
        pub unrecorded: Option<String>,
        /// Every path `read_file` — the WHOLE-file reader — was called with,
        /// in order. FS4: `read_file_window` exists so `super::file`'s
        /// windowing never has to call this one, and this is what a test
        /// checks to prove it did not.
        pub whole_reads: Vec<String>,
        /// The origin `principal()` answers with. `None` is the fixed pinned
        /// key every other test here relies on; `Some` overrides it — for a
        /// test that needs a session with no pinned key to lose, such as the
        /// serial console's break-glass.
        pub origin: Option<Origin>,
        /// What `random_bytes` answers with, verbatim. `None` refuses.
        pub random: Option<Vec<u8>>,
        /// What `sha256` answers with, verbatim. `None` refuses.
        pub digest: Option<[u8; 32]>,
        /// Every buffer `sha256` was handed, in order.
        pub digested: Vec<Vec<u8>>,
    }

    impl Default for FakeFiles {
        fn default() -> Self {
            Self::new()
        }
    }

    impl FakeFiles {
        pub fn new() -> Self {
            let mut nodes = BTreeMap::new();
            // Every test path lives under /state, and /state itself is a real
            // directory on a real board — a fake in which it did not exist
            // would make every parent walk answer wrongly.
            nodes.insert("/state".to_string(), Node::directory());
            Self {
                nodes,
                did: Vec::new(),
                asked: Vec::new(),
                interlock: None,
                act_fails: None,
                unrecorded: None,
                whole_reads: Vec::new(),
                origin: None,
                random: None,
                digest: None,
                digested: Vec::new(),
            }
        }

        pub fn with(mut self, path: &str, node: Node) -> Self {
            self.nodes.insert(path.to_string(), node);
            self
        }

        /// Answer `principal()` with `origin` instead of the fixed pinned key.
        pub fn with_origin(mut self, origin: Origin) -> Self {
            self.origin = Some(origin);
            self
        }

        pub fn holding(path: &str, contents: &str) -> Self {
            Self::new().with(path, Node::file(contents))
        }

        pub fn has(&self, path: &str) -> bool {
            self.nodes.contains_key(path)
        }

        /// The names directly inside `path`, sorted — the fake's own notion of
        /// a directory, derived from the map rather than stored, so a test
        /// cannot set up a directory whose entries disagree with the paths in
        /// it.
        fn children(&self, path: &str) -> Vec<(String, &Node)> {
            let prefix = if path == "/" {
                "/".to_string()
            } else {
                format!("{path}/")
            };
            let mut out: Vec<(String, &Node)> = self
                .nodes
                .iter()
                .filter_map(|(candidate, node)| {
                    let rest = candidate.strip_prefix(&prefix)?;
                    (!rest.is_empty() && !rest.contains('/')).then(|| (rest.to_string(), node))
                })
                .collect();
            out.sort_by(|a, b| a.0.cmp(&b.0));
            out
        }

        /// The first ancestor of `path` that is a symbolic link — the fake's
        /// answer to `ADR-006` §8.10.4.3, computed from the map the same way a
        /// real one walks the filesystem.
        fn symlinked_ancestor(&self, path: &str) -> Option<String> {
            let mut walked = String::new();
            let components: Vec<&str> = path.split('/').collect();
            for component in components.iter().take(components.len().saturating_sub(1)) {
                if component.is_empty() {
                    continue;
                }
                walked.push('/');
                walked.push_str(component);
                if self
                    .nodes
                    .get(&walked)
                    .is_some_and(|n| n.kind == PathKind::Symlink)
                {
                    return Some(walked);
                }
            }
            None
        }
    }

    crate::operation::machine::refuses_to_manage!(FakeFiles);
    crate::operation::machine::refuses_to_deploy!(FakeFiles);
    crate::operation::machine::refuses_to_observe!(FakeFiles);

    impl Editor for FakeFiles {
        fn principal(&mut self) -> Principal {
            Principal::new(self.origin.clone().unwrap_or_else(|| {
                Origin::pinned(&"fedcba9876543210".repeat(4)).expect("64 hex characters")
            }))
        }

        fn read_file(&mut self, path: &str) -> Result<FileRead, MachineError> {
            self.whole_reads.push(path.to_string());
            Ok(FileRead {
                contents: self.nodes.get(path).map(|n| n.contents.clone()),
                missing_parents: Vec::new(),
            })
        }

        /// **A real override, not the trait's default** — FS4. The default
        /// would call [`Self::read_file`] and record itself in
        /// [`Self::whole_reads`], which would make the test that fake exists
        /// for fail for the wrong reason. This answers from the map directly,
        /// windowed, the way `cmd/rivet`'s real adapter answers from a
        /// positioned read.
        fn read_file_window(
            &mut self,
            path: &str,
            at: usize,
            len: usize,
        ) -> Result<Option<FileWindow>, MachineError> {
            let Some(node) = self.nodes.get(path) else {
                return Ok(None);
            };
            let total_bytes = node.contents.len();
            let start = at.min(total_bytes);
            let end = start.saturating_add(len).min(total_bytes);
            Ok(Some(FileWindow {
                bytes: node.contents[start..end].to_vec(),
                total_bytes,
            }))
        }

        // `_contents` because this double answers on the PATH alone - what a
        // real machine checks about the bytes is its own business, and a
        // fake that inspected them would be testing this file's idea of
        // that rather than the machine's.
        fn can_write_file(&mut self, path: &str, _contents: &str) -> Result<(), MachineError> {
            self.asked.push(format!("write {path}"));
            match &self.interlock {
                Some(why) => Err(why.clone()),
                None => Ok(()),
            }
        }

        fn write_file(&mut self, write: &Write<'_>) -> Result<Wrote, MachineError> {
            if let Some(why) = &self.act_fails {
                return Err(why.clone());
            }
            for directory in write.directories {
                self.nodes.insert(directory.clone(), Node::directory());
            }
            self.did.push(format!("{} {}", write.how, write.path));
            self.nodes
                .insert(write.path.to_string(), Node::file(write.contents).moded(0o600));
            Ok(Wrote {
                unrecorded: self.unrecorded.clone(),
            })
        }

        fn stat_path(&mut self, path: &str) -> Result<PathStat, MachineError> {
            let Some(node) = self.nodes.get(path).cloned() else {
                return Ok(PathStat {
                    symlinked_ancestor: self.symlinked_ancestor(path),
                    ..PathStat::default()
                });
            };
            let entries: Vec<DirectoryEntry> = if node.kind == PathKind::Directory {
                self.children(path)
                    .into_iter()
                    .map(|(name, child)| DirectoryEntry {
                        name,
                        kind: child.kind,
                        size_bytes: child.contents.len() as u64,
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let entry_count = entries.len();
            let truncated = entry_count > super::DIRECTORY_MAX_ENTRIES;
            Ok(PathStat {
                kind: Some(node.kind),
                size_bytes: node.contents.len() as u64,
                mode: Some(node.mode),
                uid: Some(node.uid),
                user: name_of_uid(node.uid),
                gid: Some(node.gid),
                group: name_of_gid(node.gid),
                modified_ms: node.modified_ms,
                link_target: node.link_target.clone(),
                entries: entries
                    .into_iter()
                    .take(super::DIRECTORY_MAX_ENTRIES)
                    .collect(),
                entry_count,
                entries_truncated: truncated,
                symlinked_ancestor: self.symlinked_ancestor(path),
                mount_point: node.mount_point,
            })
        }

        fn resolve_owner(
            &mut self,
            user: Option<&str>,
            group: Option<&str>,
        ) -> Result<OwnerIds, MachineError> {
            let mut ids = OwnerIds::default();
            if let Some(user) = user {
                let (uid, name) = look_up(user, &[(0, "root"), (1000, "app")])
                    .ok_or_else(|| MachineError::Refused(format!("no user `{user}`")))?;
                ids.uid = Some(uid);
                ids.user = Some(name);
            }
            if let Some(group) = group {
                let (gid, name) = look_up(group, &[(0, "root"), (1000, "app")])
                    .ok_or_else(|| MachineError::Refused(format!("no group `{group}`")))?;
                ids.gid = Some(gid);
                ids.group = Some(name);
            }
            Ok(ids)
        }

        fn can_change_path(&mut self, change: &PathChange<'_>) -> Result<(), MachineError> {
            self.asked
                .push(format!("{} {}", change.act(), change.path()));
            if let Some(why) = &self.interlock {
                return Err(why.clone());
            }
            // The one check that is genuinely the machine's — see the port's
            // documentation and `ADR-006` §8.10.4.3.
            if let Some(link) = self.symlinked_ancestor(change.path()) {
                return Err(MachineError::Unsafe(format!(
                    "REFUSED {link} is a symbolic link and nothing has been changed"
                )));
            }
            Ok(())
        }

        fn change_path(&mut self, change: &PathChange<'_>) -> Result<Wrote, MachineError> {
            self.can_change_path(change)?;
            if let Some(why) = &self.act_fails {
                return Err(why.clone());
            }
            self.did.push(format!("{} {}", change.act(), change.path()));
            match *change {
                PathChange::CreateDirectory { path, parents, mode } => {
                    for parent in parents {
                        self.nodes.insert(parent.clone(), Node::directory());
                    }
                    self.nodes
                        .insert(path.to_string(), Node::directory().moded(mode));
                }
                PathChange::SetMode { path, mode, .. } => {
                    if let Some(node) = self.nodes.get_mut(path) {
                        node.mode = mode;
                    }
                }
                PathChange::SetOwner { path, uid, gid, .. } => {
                    if let Some(node) = self.nodes.get_mut(path) {
                        if let Some(uid) = uid {
                            node.uid = uid;
                        }
                        if let Some(gid) = gid {
                            node.gid = gid;
                        }
                    }
                }
                PathChange::Rename { from, to } => {
                    if let Some(node) = self.nodes.remove(from) {
                        self.nodes.insert(to.to_string(), node);
                    }
                }
                PathChange::RemoveFile { path, .. } | PathChange::RemoveDirectory { path } => {
                    self.nodes.remove(path);
                }
                // **The whole point of the act, in the fake too**: the bytes,
                // the mode and the ownership arrive together, so a test can
                // assert that a secret ends up owned by the application rather
                // than by root.
                PathChange::InstallSecret {
                    path,
                    contents,
                    mode,
                    uid,
                    gid,
                    ..
                } => {
                    self.nodes.insert(
                        path.to_string(),
                        Node::file(contents).moded(mode).owned(uid, gid),
                    );
                }
            }
            Ok(Wrote {
                unrecorded: self.unrecorded.clone(),
            })
        }

        /// The bytes a test set in [`FakeFiles::random`], exactly as given —
        /// or the trait's own refusal when it set none.
        fn random_bytes(&mut self, len: usize) -> Result<Vec<u8>, MachineError> {
            match &self.random {
                Some(bytes) => Ok(bytes.clone()),
                None => Err(MachineError::Refused(format!(
                    "this fake was given no random bytes and {len} were asked for"
                ))),
            }
        }

        /// The digest a test set in [`FakeFiles::digest`], recording what it
        /// was handed — or a refusal when it set none.
        fn sha256(&mut self, bytes: &[u8]) -> Result<[u8; 32], MachineError> {
            self.digested.push(bytes.to_vec());
            self.digest.ok_or_else(|| {
                MachineError::Refused("this fake was given no digest to answer".to_owned())
            })
        }
    }

    fn look_up(what: &str, table: &[(u32, &str)]) -> Option<(u32, String)> {
        if let Ok(id) = what.parse::<u32>() {
            let name = table
                .iter()
                .find(|(candidate, _)| *candidate == id)
                .map(|(_, name)| (*name).to_string());
            return Some((id, name.unwrap_or_else(|| id.to_string())));
        }
        table
            .iter()
            .find(|(_, name)| *name == what)
            .map(|(id, name)| (*id, (*name).to_string()))
    }

    fn name_of_uid(uid: u32) -> Option<String> {
        look_up(&uid.to_string(), &[(0, "root"), (1000, "app")]).map(|(_, name)| name)
    }

    fn name_of_gid(gid: u32) -> Option<String> {
        name_of_uid(gid)
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// `starts_with` puts `/statement` under `/state`, which is the classic
    /// form of this bug and the reason [`under`] exists.
    #[test]
    fn a_root_test_is_component_wise_and_not_a_prefix() {
        assert!(under("/state", "/state"));
        assert!(under("/state/x", "/state"));
        assert!(under("/state/a/b", "/state"));
        assert!(!under("/statement", "/state"));
        assert!(!under("/state-old/x", "/state"));
        assert!(!under("/system", "/state"));
    }

    /// **The check that makes the root check mean anything.** Every one of these
    /// passes a prefix test against `/state` and none of them is under it.
    #[test]
    fn a_traversal_component_is_refused_however_it_is_hidden() {
        for path in [
            "/state/../system/bin/rivet-sysd",
            "/state/a/../../etc/passwd",
            "/state/./x",
            "/state//x",
            "/state/..",
        ] {
            assert!(
                traversal_component(path).is_some(),
                "{path} was not seen as a traversal"
            );
            assert!(
                check_writable_path("apply remove-file", path).is_err(),
                "{path} passed the writable gate"
            );
        }
        assert_eq!(traversal_component("/state/a.b/c..d"), None);
    }

    /// The allow-list, asserted through the refusal an operator would see.
    #[test]
    fn every_root_but_state_is_refused_to_a_writing_subject() {
        for path in [
            "/system/bin/rivet-sysd",
            "/etc/passwd",
            "/run/x",
            "/proc/cmdline",
            "/sys/class/thermal/thermal_zone0/temp",
            "/dev/vcio",
            "/x",
        ] {
            let refusal = check_writable_path("apply remove-file", path)
                .expect_err(&format!("{path} passed the gate"));
            assert_eq!(refusal.code(), ErrorCode::Unsafe, "{path}");
            assert!(
                refusal.message().contains(PERMITTED_ROOT),
                "{path}: {}",
                refusal.message()
            );
        }
    }

    /// A read subject passes a **shorter** gate on purpose — see the module
    /// documentation. Reading `/system` is the question this surface exists to
    /// answer.
    #[test]
    fn a_read_subject_is_not_confined_to_the_permitted_root() {
        for path in ["/system/bin/rivet-sysd", "/etc/passwd", "/proc/cmdline"] {
            assert!(
                check_readable_path("get file", path).is_ok(),
                "{path} was refused to a read"
            );
        }
    }

    /// Both directions: what the ruling is FOR must pass, and what it must never
    /// touch must not.
    #[test]
    fn a_stale_payload_passes_and_the_load_bearing_set_does_not() {
        // §8.10.1's driving case.
        assert!(check_writable_path("apply remove-file", "/state/rivet-0.1.8.rvt").is_ok());
        assert!(check_writable_path("apply remove-file", "/state/scratch/notes").is_ok());

        for path in [
            "/state",
            "/state/config",
            "/state/config/machine.toml",
            // A mechanism's, which refuses earlier and with a better sentence.
            "/state/deploy-request",
            "/state/log/journal.jsonl",
            "/state/app/thing/current",
        ] {
            let refusal = check_writable_path("apply remove-file", path)
                .expect_err(&format!("{path} passed the gate"));
            assert_eq!(refusal.code(), ErrorCode::Unsafe, "{path}");
            assert!(
                refusal.message().contains("NOTHING HAS BEEN"),
                "{path}: {}",
                refusal.message()
            );
        }
    }

    #[test]
    fn a_mode_renders_both_ways_and_never_hides_a_bit_that_is_set() {
        assert_eq!(render_mode(0o644), "rw-r--r--");
        assert_eq!(render_mode(0o700), "rwx------");
        assert_eq!(render_mode(0o000), "---------");
        assert_eq!(render_mode(0o777), "rwxrwxrwx");
        assert_eq!(render_mode(0o4755), "rwxr-xr-x +setuid");
        assert_eq!(render_mode(0o6755), "rwxr-xr-x +setuid+setgid");
        assert_eq!(render_mode(0o1777), "rwxrwxrwx +sticky");
        assert_eq!(describe_mode(0o600), "0600 (rw-------)");
    }

    #[test]
    fn an_identifier_shows_the_number_even_when_there_is_no_name() {
        assert_eq!(describe_id(Some(0), Some("root")), "0 (root)");
        assert_eq!(describe_id(Some(1000), None), "1000");
        assert_eq!(describe_id(None, None), "?");
    }

    #[test]
    fn a_relative_path_and_a_control_character_are_refused_by_both_gates() {
        for path in ["state/x", "x", "./x"] {
            assert!(check_readable_path("get file", path).is_err(), "{path}");
        }
        assert!(check_readable_path("get file", "/state/a\nb").is_err());
        assert!(check_readable_path("get file", "").is_err());
        assert!(check_readable_path("get file", "/state/x/").is_err());
        // The root itself is one character and must not be read as a trailing
        // slash.
        assert!(check_readable_path("get file", "/").is_ok());
    }

    #[test]
    fn the_directory_of_a_path_is_textual_and_bottoms_out_at_the_root() {
        assert_eq!(directory_of("/state/a/b"), "/state/a");
        assert_eq!(directory_of("/state/a"), "/state");
        assert_eq!(directory_of("/a"), "/");
    }
}
