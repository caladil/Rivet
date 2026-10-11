//! **The fleet repository's text half** — `ADR-052` §9 step 2.
//!
//! Step 1 (`cmd/cargo-rivet/src/fleet.rs`) built the tree on the workstation
//! that holds the maintainer's fleet: a git-tracked manifest naming every
//! signed artifact, and a digest-keyed object store beside it. This crate
//! gives that same tree its other half — text, rather than bytes:
//!
//! ```text
//! <root>/                        ~/.rivet/fleet, same tree step 1 built
//!   .git/                        the text half — ADR-052 §2
//!   manifest.toml, objects/      step 1's artifact half — untouched here
//!   documents/<host>.provisioning.toml   [`record_document`]
//!   register/register.toml               [`record_register`]
//!   register/devices/<id>.toml
//! ```
//!
//! **Two callers, and this is deliberately the only place either primitive is
//! written.** `cmd/rivet-provision` calls [`record_document`] after it writes
//! a composed document to `~/.rivet/provision/` (`ADR-052` §9: *"the document
//! already exists at `~/.rivet/provision/`; the repository copy is what
//! carries history and attribution"*). `cmd/rivet-fleet` calls
//! [`record_register`] everywhere it calls `rivetos_register::store::save` or
//! `save_fleet` on the register's own on-disk copy.
//!
//! **The removal is here too, for the same reason.** `cmd/rivet-reset
//! --keep-authority` calls [`clear_records`] to empty `register/` and
//! `documents/` for a fresh fleet, and it goes through git exactly as the two
//! writes do: the removal is a commit that says what removed it, and the
//! history of the fleet it cleared stays answerable. `SPEC-05` §2.
//!
//! # A PROJECTION, not the register's new home
//!
//! **The register's real files stay exactly where they are —
//! `%APPDATA%\rivet-fleet\register` on Windows, resolved through
//! `rivetos-config`'s own loader — and this crate never reads or writes
//! there.** [`record_register`] is handed a [`rivetos_register::Register`]
//! already loaded by the caller and writes a second, read-only-in-spirit copy
//! of it under `<root>/register/`, in the identical shape
//! `rivetos_register::render` would put at the real location.
//!
//! This is the answer to the question `ADR-052` §9 step 2 poses explicitly —
//! *"where does the repository live relative to the register, and who owns
//! the copy"* — taken deliberately rather than by default, for three reasons:
//!
//! 1. **It changes no existing read path.** Every place that reads the
//!    register today — `rivet-fleet`'s own commands, `rivet-fleetd` on a
//!    board — goes on reading `%APPDATA%\rivet-fleet\register` tomorrow,
//!    unchanged. Moving the register's real home into a git working tree
//!    would touch `rivetos-config`'s loader, every one of those readers, and
//!    the maintainer's rehearsed twelve steps, none of which `ADR-052` §9
//!    step 2 asks for.
//! 2. **`rivetos-register` cannot know about a fleet repository.** It is
//!    linked by `cmd/rivet-fleetd`, which runs *on a Rivet machine* — no
//!    `git`, no `~/.rivet/fleet`, no workstation to be one. Teaching
//!    `store::save` to also commit into a git tree would put a workstation
//!    concept behind a call a board makes on every convergence, which
//!    `AGENTS.md` §3's ports-and-adapters boundary and `ADR-014` §3 both
//!    refuse: I/O belongs at the composition root, and the fleet repository
//!    is a workstation-only fact about `ADR-052` — the board's register store
//!    has no business knowing it exists.
//! 3. **A projection can never itself go stale in a way that mattered before
//!    it existed.** The real register is still the one thing `rivet-fleet`'s
//!    own commands read back and act on; the projection is written *after* a
//!    save succeeds and is best-effort from there — see [`Outcome::Refused`].
//!    If the projection ever falls behind (a crashed process, a `.seed` file
//!    left in the tree), the fix is calling [`record_register`] again with
//!    the register's current state, exactly what the next `rivet-fleet`
//!    command does anyway.
//!
//! **What this gives up, said out loud:** the projection can be one save
//! behind the real register — the window between `store::save` succeeding and
//! this crate's own commit landing. `ADR-052` §5's whole argument is that a
//! document is attributed, not authoritative; a repository one save behind is
//! still answerable to "what did this fleet look like on Tuesday", which is
//! the question step 2 exists to give an answer to, and it is not the
//! question `%APPDATA%\rivet-fleet\register` itself answers.
//!
//! # `ADR-052` §4's refusal, unchanged in substance
//!
//! **Every write here checks for a `.seed` file first, exactly as step 1
//! does, and refuses with no override.** `find_seed` and `seed_refusal`
//! are this crate's own copies rather than an import from
//! `cmd/cargo-rivet/src/fleet.rs` — that module's own header explains why
//! (no `[lib]` target, and step 1's own path is not this change's to touch).
//! Two implementations of one refusal is a real cost; the ADR names it as a
//! check rather than a convention for a reason that does not shrink when the
//! check is copied — a published signing key cannot be unpublished — so the
//! duplication is accepted rather than the risk of touching step 1 to avoid
//! it. `cmd/cargo-rivet`'s own test suite is untouched by this crate existing
//! at all.
//!
//! # No signing here either
//!
//! `ADR-052` §5 is as explicit for a document and a register record as it is
//! for an artifact under `documents/` step 1 never claimed one for: a
//! document is applied by a machine that has already authenticated whoever
//! handed it over, over `ADR-035`'s mutual TLS. What was missing was
//! attribution, and git supplies that with no cryptography — see this
//! module's own commit messages, which name a file and a git author rather
//! than carrying a signature.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use rivetos_register::Register;

/// Where a machine's composed document is filed, under the fleet repository's
/// root.
pub const DOCUMENTS_DIR: &str = "documents";

/// The suffix a document is filed under — the same one
/// `cmd/rivet-provision/src/save.rs`'s `DOCUMENT_SUFFIX` gives the copy at
/// `~/.rivet/provision/`, so a person who has learned to recognise one
/// recognises the other. The two crates cannot share the constant (one is a
/// bin-only crate), so this is a second literal rather than a second
/// definition — the string itself, `.provisioning.toml`, is the fact, and
/// `save_tests.rs`-equivalent coverage below pins it against drifting apart.
pub const DOCUMENT_SUFFIX: &str = ".provisioning.toml";

/// Where the register's projection is filed, under the fleet repository's
/// root — a subtree, never the root itself, so `documents/` and `objects/`
/// (step 1's) are siblings rather than neighbours a careless glob could
/// confuse.
pub const REGISTER_DIR: &str = "register";

/// **The name a fleet server serves this repository's text under** —
/// `[repos.fleet]` in that server's own document, `/state/git/fleet.git` on
/// the machine (`rivet_core::git::repository_dir`), and the repository
/// `cargo rivet fleet push` pushes `main` to.
///
/// **Fixed, and never asked for** (`.rulings/` R-184 D3). A server carries
/// exactly one repository's text, and a free-typed name would only buy a
/// place for what provisioning declared and what `fleet push` assumes to
/// disagree. Two crates spell it — `cmd/rivet-provision`, which declares it,
/// and `cmd/cargo-rivet`, which pushes to it — and both read this one.
///
/// **This names a `[repos.<name>]` repository, which the rest of this crate
/// otherwise never touches** — see `cmd/cargo-rivet/src/fleet.rs`'s header
/// for why the two concepts are kept apart everywhere else.
pub const SERVED_NAME: &str = "fleet";

// ---------------------------------------------------------------------------
// the refusal — ADR-052 §4, this crate's own copy (see the module doc)
// ---------------------------------------------------------------------------

/// Walk `dir` looking for a `.seed` file, never descending into `.git/` —
/// `cmd/cargo-rivet/src/fleet.rs::find_seed`'s own reasoning: git's internals
/// are not this tree's business, and a repository committed to many times has
/// thousands of objects in there for no seed the walk could ever find (one is
/// refused before it reaches `.git/objects` in the first place).
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
        let entries = fs::read_dir(&current)
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

/// The refusal itself, naming the file [`find_seed`] found.
///
/// **There is no flag that overrides this**, `ADR-052` §4's own words: a
/// published signing key cannot be unpublished, and every board that trusts
/// its public half is compromised at once.
fn seed_refusal(found: &Path) -> String {
    format!(
        "{} is a `.seed` file, and the fleet repository refuses to write or commit a tree that \
         contains one (ADR-052 §4). Every signing seed this project mints ends in `.seed`. Move \
         it out of this tree — there is no flag that overrides this refusal.",
        found.display()
    )
}

// ---------------------------------------------------------------------------
// git, shelled out to — the same posture step 1 takes
// ---------------------------------------------------------------------------

// Extra environment variables the current TEST thread wants every `git`
// child process this crate spawns to see, layered on top of whatever the
// real environment already provides.
//
// Thread-local, never `std::env::set_var`. Rust's test harness runs each
// test on its own thread; `set_var` is process-wide, so a test isolating its
// own `git init` behind a synthetic `init.defaultBranch` would leak that
// override into every other test's concurrently running `git` calls. A
// thread-local cannot — no other thread ever observes it. `tests::GitEnvGuard`
// is the only writer.
#[cfg(test)]
thread_local! {
    static TEST_GIT_ENV: std::cell::RefCell<Vec<(String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// A `git` [`Command`] against `root`, with [`TEST_GIT_ENV`] layered on in a
/// test build — the one seam every git-spawning site in this crate goes
/// through, so a test can isolate ambient git configuration without
/// reaching for a process-wide environment mutation anywhere in this file.
fn git_command(root: &Path) -> Command {
    let mut command = Command::new("git");
    command.current_dir(root);
    #[cfg(test)]
    TEST_GIT_ENV.with(|env| {
        for (key, value) in env.borrow().iter() {
            command.env(key, value);
        }
    });
    command
}

/// No `git2`/`libgit2-sys`, matching `cmd/cargo-rivet/src/fleet.rs::git`'s own
/// reasoning: a system `git` is already a dependency of this workflow (the
/// maintainer runs it to push the fleet repository outward, `ADR-052` §6),
/// and `rivet-image/src/system.rs` records the crate alternative as 93 crates
/// deep for a service this project does not need either.
fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = git_command(root)
        .args(args)
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

fn git_configured(root: &Path, key: &str) -> bool {
    git_command(root)
        .args(["config", "--get", key])
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.is_empty())
}

/// Fill `user.name`/`user.email` **only when nothing in scope already
/// answers**, and only in this repository's own local config —
/// `cmd/cargo-rivet/src/fleet.rs::ensure_local_identity`'s own argument,
/// restated: a real operator's global identity is what a fleet repository's
/// commits should carry, and this never runs `--global`.
fn ensure_local_identity(root: &Path) -> Result<(), String> {
    for (key, value) in [("user.name", "cargo-rivet-fleet"), ("user.email", "fleet@rivet.local")] {
        if !git_configured(root, key) {
            git(root, &["config", key, value])?;
        }
    }
    Ok(())
}

/// Whether staging so far left the index different from `HEAD`, scoped to
/// `pathspec` — `git diff --cached --quiet`'s own exit code, read rather than
/// re-derived by comparing bytes ourselves. Reading git's own answer is what
/// makes "running the same save twice produces no empty commit" true without
/// this crate keeping any state of its own between calls: a second run stages
/// identical bytes over identical bytes, the index does not move, and there
/// is nothing for `git diff --cached` to report.
///
/// # Errors
///
/// `git` could not be run, or exited with neither 0 nor 1.
fn staged_changes(root: &Path, pathspecs: &[&str]) -> Result<Option<String>, String> {
    let status = git_command(root)
        .args(["diff", "--cached", "--quiet", "--"])
        .args(pathspecs)
        .status()
        .map_err(|e| format!("cannot run `git diff --cached --quiet`: {e}"))?;
    match status.code() {
        Some(0) => return Ok(None),
        Some(1) => {}
        _ => return Err(format!("`git diff --cached --quiet` exited abnormally: {status}")),
    }

    // Something changed — name what, so the commit message says more than
    // "something changed". `--name-status` is one letter and one path per
    // line; turned into "path added"/"path changed"/"path removed" rather
    // than repeated verbatim, because `A`/`M`/`D` means nothing to a person
    // reading `git log` later and the ADR's whole argument for this crate is
    // attribution a person can read.
    let mut args = vec!["diff", "--cached", "--name-status", "--"];
    args.extend_from_slice(pathspecs);
    let output = git(root, &args)?;
    let mut parts = Vec::new();
    for line in output.lines() {
        let mut fields = line.splitn(2, '\t');
        let (Some(code), Some(path)) = (fields.next(), fields.next()) else {
            continue;
        };
        let verb = match code.chars().next() {
            Some('A') => "added",
            Some('M') => "changed",
            Some('D') => "removed",
            Some('R') => "renamed",
            _ => "touched",
        };
        parts.push(format!("{path} {verb}"));
    }
    Ok(Some(parts.join(", ")))
}

/// Commit exactly `pathspecs` — **never the rest of the index** — with
/// `messages` as the commit's `-m` arguments in order.
///
/// **`git commit -- <pathspec>` already restricts a commit to the given
/// paths by default**, disregarding anything else staged in the same index;
/// `--only` names that behaviour explicitly here so a later edit that adds
/// `-a`/`--include` cannot silently widen it back to the whole index. This is
/// the fix for the defect every commit in this crate used to carry: each one
/// ran a bare `git commit` with no pathspec at all, which commits the ENTIRE
/// index — so a file staged in this SAME repository by something else
/// (`~/.rivet/fleet` is the maintainer's own working tree, not a dedicated
/// checkout) went into a commit this crate attributes to itself, and a
/// `.seed` file staged and then removed from disk stayed eligible for that
/// commit even though [`find_seed`]'s worktree walk could no longer see it.
/// Every commit this crate makes now only ever touches the paths it owns;
/// anything else stays exactly as staged as it was before the call.
///
/// # Errors
///
/// Whatever `git` said — most often a hook that refused.
fn commit_only(root: &Path, pathspecs: &[&str], messages: &[&str]) -> Result<(), String> {
    let mut args = vec!["commit", "--quiet", "--only"];
    for message in messages {
        args.push("-m");
        args.push(message);
    }
    args.push("--");
    args.extend_from_slice(pathspecs);
    git(root, &args)?;
    Ok(())
}

/// The first path among `pathspecs` that is about to be ADDED, COPIED,
/// MODIFIED or RENAMED INTO — never merely removed — by a commit scoped to
/// those same paths, whose name is a `.seed` file.
///
/// **Deliberately index-based, not [`find_seed`]'s worktree walk.** A file
/// staged and then removed from disk is invisible to a walk of the working
/// tree, but it is still exactly what [`commit_only`] would commit if its
/// pathspec reached it — so the check that runs immediately before a commit
/// has to look at the same thing the commit itself is about to look at: the
/// staged difference from `HEAD`, scoped to the same paths. `--diff-filter`
/// excludes a pure deletion on purpose — [`clear_records`] legitimately
/// removes whatever [`find_seed`] already refused to let anything ADD, and a
/// removal must never itself be refused.
///
/// **A path outside `pathspecs` is never this function's business, and
/// [`commit_only`] is what excludes it** — that is the more common shape of
/// B04's defect (an unrelated file staged somewhere else entirely) and does
/// not need an index read to defend against, only a commit that never
/// widens past its own paths. This function is the second, narrower layer:
/// a name inside the very paths being committed. For every caller in this
/// crate today, [`git`]'s own `add <pathspec>` already resyncs a
/// staged-then-deleted path under a DIRECTORY pathspec before this ever
/// runs, so this is defense in depth against a future caller whose `add`
/// and commit pathspecs diverge, or a caller that stages more narrowly —
/// not a case any test here can force through the public functions as they
/// are wired today. See `staged_seed_finds_an_added_seed_within_its_pathspec`
/// for direct coverage of what it does.
///
/// # Errors
///
/// `git` could not be run, or exited with a non-zero status.
fn staged_seed(root: &Path, pathspecs: &[&str]) -> Result<Option<PathBuf>, String> {
    let mut args = vec!["diff", "--cached", "--name-only", "--diff-filter=ACMR", "--"];
    args.extend_from_slice(pathspecs);
    let output = git(root, &args)?;
    for line in output.lines() {
        let path = Path::new(line);
        let is_seed = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("seed"));
        if is_seed {
            return Ok(Some(root.join(path)));
        }
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// the repository itself — auto-created, ADR-052 §4 checked first
// ---------------------------------------------------------------------------

/// `~/.rivet/fleet`, or `%USERPROFILE%\.rivet\fleet` — the same default
/// `cmd/cargo-rivet/src/fleet.rs::default_root` gives step 1's repository,
/// because step 2 writes into the SAME tree, not a second one.
///
/// # Errors are not this function's — it returns `None`
///
/// Neither `HOME` nor `USERPROFILE` is set. Every caller here already treats
/// "no fleet repository to write to" as non-fatal (see [`Outcome::Refused`]),
/// so this hands back `None` for the caller to fold into that same path
/// rather than a `Result` nothing else in this crate would do anything with.
#[must_use]
pub fn default_root() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".rivet").join("fleet"))
}

/// Make `root` a git repository if it is not already one — [`record_document`]
/// and [`record_register`] both call this before writing anything, so neither
/// needs the maintainer to have run `cargo rivet fleet init` first. **Running
/// it against an existing repository changes nothing**, `cargo rivet fleet
/// init`'s own posture: an existing `.git` is left exactly as it is.
///
/// Deliberately narrower than `cargo rivet fleet init`: it writes `.gitignore`
/// and starts the repository but never touches `manifest.toml` — that file is
/// step 1's, and a fresh repository this function creates is read by
/// `cargo rivet fleet list` as correctly empty whether `manifest.toml` exists
/// yet or not (`fleet::read_manifest` treats an absent file as no artifacts).
/// The first `cargo rivet fleet publish` against this same tree creates it.
///
/// # Errors
///
/// [`seed_refusal`] — checked before `git init` runs, so a seed already
/// somewhere in `root` is caught before anything is created there — or
/// whatever `git` said.
fn ensure_repository(root: &Path) -> Result<(), String> {
    if root.join(".git").is_dir() {
        return Ok(());
    }
    if let Some(found) = find_seed(root)? {
        return Err(seed_refusal(&found));
    }

    fs::create_dir_all(root).map_err(|e| format!("cannot create {}: {e}", root.display()))?;
    let gitignore = root.join(".gitignore");
    if !gitignore.is_file() {
        fs::write(&gitignore, "/objects/\n")
            .map_err(|e| format!("cannot write {}: {e}", gitignore.display()))?;
    }

    // **`main`, named rather than inherited.** A plain `git init` takes this
    // host's `init.defaultBranch`, which may be anything; `cargo rivet fleet
    // push` requires a commit at `refs/heads/main` and the explicit `cargo
    // rivet fleet init` already names `main` here for the same reason
    // (`cmd/cargo-rivet/src/fleet.rs::init`), so an automatically created
    // repository is made to agree with it.
    git(root, &["init", "--quiet", "--initial-branch=main"])?;
    ensure_local_identity(root)?;

    // Checked again immediately before the first commit — ADR-052 §4 covers a
    // commit as much as the first `git init`, and `git init` just created a
    // `.git` directory the walk above skips, so this costs nothing new.
    if let Some(found) = find_seed(root)? {
        return Err(seed_refusal(&found));
    }
    git(root, &["add", ".gitignore"])?;
    commit_only(root, &[".gitignore"], &["Initialise the fleet repository (ADR-052)"])?;
    Ok(())
}

// ---------------------------------------------------------------------------
// the outcome every caller here reports
// ---------------------------------------------------------------------------

/// What happened, in the caller's terms — a status line to print, never a
/// reason to fail the operation that called this.
///
/// **[`Outcome::Refused`] is not an [`Err`]**, and that is the whole point of
/// this type existing rather than a `Result`. `ADR-052` §9's own constraint
/// on this step: `rivet-provision` must go on writing
/// `~/.rivet/provision/<host>.provisioning.toml` and `rivet-fleet` must go on
/// saving the real register exactly as each does today, whether or not the
/// fleet repository can be written to at all. A caller that turned this into
/// a hard failure would make an operator's `w` keystroke or `add device`
/// start refusing because of a machine this task never touches at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A commit landed. The message is the one git recorded — the same words
    /// `git log` will show later.
    Committed(String),
    /// Nothing to commit: the fleet repository already held exactly these
    /// bytes. Not an error — `ADR-052`'s idempotent-add posture, applied to
    /// text the way step 1 applies it to an artifact's digest.
    Unchanged,
    /// The write did not happen, and why — most often `seed_refusal`, but
    /// also "no fleet repository root could be resolved" and anything `git`
    /// itself refused.
    Refused(String),
}

impl core::fmt::Display for Outcome {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Committed(message) => write!(f, "fleet repository: {message}"),
            Self::Unchanged => {
                write!(f, "fleet repository: unchanged — already recorded, nothing to commit")
            }
            Self::Refused(why) => write!(f, "fleet repository: NOT recorded — {why}"),
        }
    }
}

// ---------------------------------------------------------------------------
// record_document — cmd/rivet-provision's half
// ---------------------------------------------------------------------------

/// A filename `safe_name` produces, for `name` — the hostname a document was
/// composed for. Deliberately the same algorithm
/// `cmd/rivet-provision/src/save.rs::safe_name` uses (letters, digits and `-`
/// survive; everything else becomes `-`), copied rather than shared for that
/// module's own reason: a bin-only crate has nothing to export.
fn safe_name(name: &str) -> String {
    let safe: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect();
    if safe.is_empty() { "unnamed".to_owned() } else { safe }
}

/// Record a machine's already-composed document in the fleet repository, and
/// commit — `ADR-052` §9 step 2, part 1.
///
/// `name` is the hostname the document was composed for and `text` is the
/// exact bytes `cmd/rivet-provision` already wrote to
/// `~/.rivet/provision/<host>.provisioning.toml`; this writes the same bytes
/// to `<root>/documents/<safe_name(name)>.provisioning.toml` and commits with
/// a message naming the file and whether it was added, changed, or (on a
/// second identical save) neither.
///
/// **Never returns an error.** See [`Outcome::Refused`] for why — a document
/// this project's own interview just composed and already wrote to disk must
/// go on being reported as written whether or not this second copy could be
/// made.
#[must_use]
pub fn record_document(root: &Path, name: &str, text: &str) -> Outcome {
    match record_document_inner(root, name, text) {
        Ok(outcome) => outcome,
        Err(why) => Outcome::Refused(why),
    }
}

fn record_document_inner(root: &Path, name: &str, text: &str) -> Result<Outcome, String> {
    ensure_repository(root)?;
    if let Some(found) = find_seed(root)? {
        return Err(seed_refusal(&found));
    }

    let file_name = format!("{}{DOCUMENT_SUFFIX}", safe_name(name));
    let documents_dir = root.join(DOCUMENTS_DIR);
    fs::create_dir_all(&documents_dir)
        .map_err(|e| format!("cannot create {}: {e}", documents_dir.display()))?;
    let path = documents_dir.join(&file_name);
    fs::write(&path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;

    // Checked again immediately before the commit — the same second look
    // `init` and `publish` take in step 1, for the same reason: this is the
    // moment right before these bytes become part of the repository's
    // history.
    if let Some(found) = find_seed(root)? {
        return Err(seed_refusal(&found));
    }
    let relative = format!("{DOCUMENTS_DIR}/{file_name}");
    git(root, &["add", "--", &relative])?;
    let Some(summary) = staged_changes(root, &[&relative])? else {
        return Ok(Outcome::Unchanged);
    };
    if let Some(found) = staged_seed(root, &[&relative])? {
        return Err(seed_refusal(&found));
    }
    let message = format!("Record {name}'s provisioning document in the fleet repository ({summary})");
    commit_only(root, &[&relative], &[&message])?;
    Ok(Outcome::Committed(message))
}

// ---------------------------------------------------------------------------
// record_register — cmd/rivet-fleet's half
// ---------------------------------------------------------------------------

/// Project `register` into the fleet repository and commit — `ADR-052` §9
/// step 2, part 2.
///
/// **A projection, rendered fresh from `register` every call — see this
/// crate's module documentation for the argument that it stays one rather
/// than becoming the register's new home.** `register` is rendered with
/// [`rivetos_register::render`] into `<root>/register/`, in the identical
/// layout the real register uses — `register.toml` beside `devices/<id>.toml`
/// — so the copy can be read back with `rivetos_register::store::load`
/// exactly as the real one is.
///
/// **Self-cleaning.** A device this register no longer holds is removed from
/// `<root>/register/devices/` before anything is rendered, so a device
/// `rivet-fleet remove device` forgot does not go on existing in the
/// projection forever — the projection's whole promise is that it reflects
/// the register's CURRENT state, an enrollment and a removal alike.
///
/// **Never returns an error** — see [`Outcome::Refused`]; the caller has
/// already saved the real register by the time this runs and that save must
/// stand regardless of what this one does.
#[must_use]
pub fn record_register(root: &Path, register: &Register) -> Outcome {
    match record_register_inner(root, register) {
        Ok(outcome) => outcome,
        Err(why) => Outcome::Refused(why),
    }
}

fn record_register_inner(root: &Path, register: &Register) -> Result<Outcome, String> {
    ensure_repository(root)?;
    if let Some(found) = find_seed(root)? {
        return Err(seed_refusal(&found));
    }

    let projected_root = root.join(REGISTER_DIR);
    let devices_dir = projected_root.join(rivetos_register::DEVICES_DIR);

    if devices_dir.is_dir() {
        let entries = fs::read_dir(&devices_dir)
            .map_err(|e| format!("cannot read {}: {e}", devices_dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("cannot read {}: {e}", devices_dir.display()))?;
            let path = entry.path();
            let still_current = path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|id| register.devices.contains_key(id));
            if !still_current {
                fs::remove_file(&path).map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
            }
        }
    }

    let projected =
        Register { root: projected_root, fleet: register.fleet.clone(), devices: register.devices.clone() };
    let files = rivetos_register::render(&projected).map_err(|e| e.to_string())?;
    for (path, text) in &files {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        fs::write(path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }

    // Checked again immediately before the commit, for [`record_document`]'s
    // reason.
    if let Some(found) = find_seed(root)? {
        return Err(seed_refusal(&found));
    }
    git(root, &["add", "--", REGISTER_DIR])?;
    let Some(summary) = staged_changes(root, &[REGISTER_DIR])? else {
        return Ok(Outcome::Unchanged);
    };
    if let Some(found) = staged_seed(root, &[REGISTER_DIR])? {
        return Err(seed_refusal(&found));
    }
    let message = format!("Project the register into the fleet repository ({summary})");
    commit_only(root, &[REGISTER_DIR], &[&message])?;
    Ok(Outcome::Committed(message))
}

// ---------------------------------------------------------------------------
// clear_records — cmd/rivet-reset's half
// ---------------------------------------------------------------------------

/// **What the working tree holds that `HEAD` does not**, as `git status
/// --porcelain` lines, or `None` when the tree is clean.
///
/// Untracked files count: a file nobody has committed yet is still somebody's
/// work. `objects/` does not, because `.gitignore` excludes it (`ADR-052` §3).
///
/// # Errors
///
/// `root` is not a git repository, or `git` could not be run.
pub fn uncommitted(root: &Path) -> Result<Option<String>, String> {
    if !root.join(".git").exists() {
        return Err(format!("{} is not a git repository", root.display()));
    }
    let out = git(root, &["status", "--porcelain", "--untracked-files=all"])?;
    let lines = out.trim_end();
    Ok(if lines.is_empty() { None } else { Some(lines.to_owned()) })
}

/// **Remove every machine record — `register/` and `documents/` — and commit
/// the removal.** `rivet-reset --keep-authority`, `SPEC-05` §2.
///
/// `manifest.toml` and `objects/` are not touched. They are the published
/// software, not a record of any machine, and a fresh fleet installs the same
/// artifacts the old one did.
///
/// **A dirty tree is refused rather than committed through.** A commit made
/// here would carry whatever else was staged or lying in the tree, and a reset
/// has no business attributing somebody else's unfinished work to itself. The
/// caller checks with [`uncommitted`] first, so that it can refuse before it
/// removes anything anywhere; this checks again because this is the moment
/// the commit happens.
///
/// `by` names the program in the commit subject. Returns that subject, or
/// `None` when there was nothing to remove.
///
/// # Errors
///
/// A dirty tree, naming what is in it; a `.seed` file in the tree (`ADR-052`
/// §4); or whatever `git` said.
pub fn clear_records(root: &Path, by: &str) -> Result<Option<String>, String> {
    if let Some(dirty) = uncommitted(root)? {
        return Err(format!(
            "{} has uncommitted changes, and a reset will not commit somebody else's work:\n{dirty}",
            root.display()
        ));
    }
    if let Some(found) = find_seed(root)? {
        return Err(seed_refusal(&found));
    }
    let pathspecs = [REGISTER_DIR, DOCUMENTS_DIR];
    let mut args = vec!["rm", "-r", "--quiet", "--ignore-unmatch", "--"];
    args.extend_from_slice(&pathspecs);
    git(root, &args)?;
    let Some(summary) = staged_changes(root, &pathspecs)? else {
        return Ok(None);
    };
    ensure_local_identity(root)?;
    let count = summary.split(", ").count();
    let subject = format!(
        "{by} removed the fleet's {count} machine record(s) from {REGISTER_DIR}/ and \
         {DOCUMENTS_DIR}/"
    );
    commit_only(root, &pathspecs, &[&subject, &summary])?;
    Ok(Some(subject))
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use rivetos_register::Device;

    /// A real directory under the OS temporary directory, removed at the end
    /// — `rivetos_register::store`'s own `Scratch` pattern, copied rather
    /// than shared for the same reason as everything else in this file: there
    /// is nowhere for a `#[cfg(test)]` helper to be shared from without a
    /// `dev-dependency` on a crate this one already depends on for real.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("rivet-fleet-repo-{label}-{n}"));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("a scratch directory");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Sets [`TEST_GIT_ENV`] for as long as the guard lives, and clears it on
    /// drop — even if the test panics, so a failing assertion never leaves
    /// the override in place for whatever this thread runs next. Thread-local
    /// underneath, so no other test's concurrently running `git` calls are
    /// ever affected by this one setting or clearing it.
    struct GitEnvGuard;

    impl GitEnvGuard {
        fn set(vars: Vec<(String, String)>) -> Self {
            TEST_GIT_ENV.with(|env| *env.borrow_mut() = vars);
            Self
        }
    }

    impl Drop for GitEnvGuard {
        fn drop(&mut self) {
            TEST_GIT_ENV.with(|env| env.borrow_mut().clear());
        }
    }

    /// **The workstation pushes to the repository the machine reads objects'
    /// manifest out of** — two spellings of R-184 D3's name, held together.
    #[test]
    fn the_served_name_is_the_one_the_machine_reads_the_manifest_from() {
        assert_eq!(SERVED_NAME, rivet_core::repository_objects::REPOSITORY);
    }

    fn log(root: &Path) -> String {
        Command::new("git")
            .args(["log", "--format=%s"])
            .current_dir(root)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    }

    /// `HEAD`'s own symbolic ref, such as `refs/heads/main` — read directly
    /// rather than through [`git_command`], since resolving `HEAD` consults
    /// no ambient configuration a test would need to isolate.
    fn head_ref(root: &Path) -> String {
        let output = Command::new("git")
            .args(["symbolic-ref", "HEAD"])
            .current_dir(root)
            .output()
            .expect("git symbolic-ref runs");
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    /// Whether `relative` differs between the index and `HEAD` — staged,
    /// whether or not it still exists on disk.
    fn is_staged(root: &Path, relative: &str) -> bool {
        let status = Command::new("git")
            .args(["diff", "--cached", "--quiet", "--", relative])
            .current_dir(root)
            .status()
            .expect("git diff runs");
        status.code() == Some(1)
    }

    /// Whether `relative` exists in `HEAD`'s tree — a commit actually
    /// carries it, not merely that it was staged at some point.
    fn is_in_head(root: &Path, relative: &str) -> bool {
        Command::new("git")
            .args(["cat-file", "-e", &format!("HEAD:{relative}")])
            .current_dir(root)
            .status()
            .is_ok_and(|s| s.success())
    }

    /// Every byte `git log -p` would print for the whole repository — used
    /// only to prove a secret's CONTENT never reached any commit, not merely
    /// that its path is absent from the tip.
    fn full_history_text(root: &Path) -> String {
        Command::new("git")
            .args(["log", "-p", "--all"])
            .current_dir(root)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default()
    }

    /// `git` itself decides whether a hook runs by checking the EXECUTE bit
    /// on Unix; Windows carries no execute-permission bit at all, and Git for
    /// Windows instead invokes a hook script through its own shebang-aware
    /// runtime, so this is a no-op there.
    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path).expect("hook metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).expect("chmod the hook");
    }

    #[cfg(not(unix))]
    fn make_executable(_path: &Path) {}

    // ---- record_document ----------------------------------------------

    /// **The primary case**: a document lands under `documents/`, is
    /// readable back, and the commit names the file and that it was added.
    #[test]
    fn a_document_is_recorded_and_committed_with_a_message_naming_what_changed() {
        let scratch = Scratch::new("document-new");
        let outcome = record_document(&scratch.0, "rivet-01", "hostname = \"rivet-01\"\n");
        match &outcome {
            Outcome::Committed(message) => {
                assert!(message.contains("rivet-01"), "{message}");
                assert!(message.contains("added"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        let path = scratch.0.join(DOCUMENTS_DIR).join("rivet-01.provisioning.toml");
        assert_eq!(fs::read_to_string(&path).expect("written"), "hostname = \"rivet-01\"\n");
        assert!(log(&scratch.0).contains("rivet-01"), "{}", log(&scratch.0));
    }

    /// **No empty commit.** Saving the identical document a second time
    /// reports `Unchanged` and the repository gains no second commit.
    #[test]
    fn recording_the_same_document_twice_makes_no_second_commit() {
        let scratch = Scratch::new("document-repeat");
        let _ = record_document(&scratch.0, "rivet-01", "hostname = \"rivet-01\"\n");
        let before = log(&scratch.0);
        let second = record_document(&scratch.0, "rivet-01", "hostname = \"rivet-01\"\n");
        assert_eq!(second, Outcome::Unchanged);
        assert_eq!(log(&scratch.0), before, "a second identical save must not add a commit");
    }

    /// A changed document is a second commit, and the message says so rather
    /// than repeating "added".
    #[test]
    fn a_changed_document_is_recorded_as_changed_not_added() {
        let scratch = Scratch::new("document-change");
        let _ = record_document(&scratch.0, "rivet-01", "hostname = \"rivet-01\"\n");
        let outcome = record_document(&scratch.0, "rivet-01", "hostname = \"rivet-01\"\ntimezone = \"UTC\"\n");
        match outcome {
            Outcome::Committed(message) => assert!(message.contains("changed"), "{message}"),
            other => panic!("{other:?}"),
        }
    }

    /// **`ADR-052` §4, as a test rather than a promise.** A `.seed` file
    /// anywhere in the tree refuses the write, and the document is not on
    /// disk afterwards.
    #[test]
    fn a_seed_anywhere_in_the_tree_refuses_a_document() {
        let scratch = Scratch::new("document-seed");
        fs::write(scratch.0.join("rivet-app.seed"), b"not a real seed, just the extension").unwrap();
        let outcome = record_document(&scratch.0, "rivet-01", "hostname = \"rivet-01\"\n");
        match &outcome {
            Outcome::Refused(why) => assert!(why.contains("ADR-052"), "{why}"),
            other => panic!("{other:?}"),
        }
        assert!(!scratch.0.join(DOCUMENTS_DIR).exists(), "nothing should have been written");
    }

    /// **Removing the seed lets the same directory record afterwards** —
    /// `cmd/cargo-rivet/src/fleet_tests.rs`'s own coverage of `init`, proven
    /// here for the write path this crate adds.
    #[test]
    fn removing_the_seed_unblocks_the_same_directory() {
        let scratch = Scratch::new("document-seed-removed");
        let seed = scratch.0.join("rivet-app.seed");
        fs::write(&seed, b"x").unwrap();
        assert!(matches!(
            record_document(&scratch.0, "rivet-01", "hostname = \"rivet-01\"\n"),
            Outcome::Refused(_)
        ));
        fs::remove_file(&seed).unwrap();
        assert!(matches!(
            record_document(&scratch.0, "rivet-01", "hostname = \"rivet-01\"\n"),
            Outcome::Committed(_)
        ));
    }

    // ---- B04: an automatic commit touches only its own paths -------------

    /// **An unrelated file already staged — with a space in its name, so a
    /// pathspec built from it has to be right — is untouched by a
    /// document's own commit.** It never reaches `HEAD`, and it is still
    /// staged afterward exactly as the caller left it.
    #[test]
    fn recording_a_document_leaves_an_unrelated_staged_file_staged_and_out_of_head() {
        let scratch = Scratch::new("document-unrelated-staged");
        let _ = record_document(&scratch.0, "warm-up", "hostname = \"warm-up\"\n");
        let unrelated = "an unrelated file.txt";
        fs::write(scratch.0.join(unrelated), b"not part of any document").unwrap();
        git(&scratch.0, &["add", "--", unrelated]).unwrap();
        assert!(is_staged(&scratch.0, unrelated), "the fixture itself must be staged");

        let outcome = record_document(&scratch.0, "rivet-01", "hostname = \"rivet-01\"\n");
        assert!(matches!(outcome, Outcome::Committed(_)), "{outcome:?}");

        assert!(!is_in_head(&scratch.0, unrelated), "an unrelated staged file must not be committed");
        assert!(is_staged(&scratch.0, unrelated), "an unrelated staged file must remain staged");
    }

    /// **A `.seed` file staged and then deleted from disk never enters a
    /// document's commit** — B04's sharpest case. [`find_seed`]'s worktree
    /// walk cannot see it once it is off disk, so the guard that stops it
    /// has to be that the commit is scoped to the document's own path in the
    /// first place.
    #[test]
    fn a_seed_staged_and_then_deleted_from_disk_never_enters_a_document_commit() {
        let scratch = Scratch::new("document-seed-staged-deleted");
        let _ = record_document(&scratch.0, "warm-up", "hostname = \"warm-up\"\n");
        let seed = scratch.0.join("unrelated.seed");
        fs::write(&seed, b"SYNTHETIC-NOT-A-KEY").unwrap();
        git(&scratch.0, &["add", "--", "unrelated.seed"]).unwrap();
        fs::remove_file(&seed).unwrap();
        assert!(is_staged(&scratch.0, "unrelated.seed"), "the fixture must still be staged though absent from disk");

        let outcome = record_document(&scratch.0, "rivet-01", "hostname = \"rivet-01\"\n");
        assert!(matches!(outcome, Outcome::Committed(_)), "{outcome:?}");

        assert!(!is_in_head(&scratch.0, "unrelated.seed"), "the seed's path must never reach HEAD");
        assert!(
            !full_history_text(&scratch.0).contains("SYNTHETIC-NOT-A-KEY"),
            "the seed's content must never appear in any commit"
        );
    }

    /// **A failing `pre-commit` hook refuses the write, and hooks are never
    /// bypassed to make one succeed.** The document's bytes reach disk (this
    /// crate's own write happens first) but the commit itself does not land.
    #[test]
    fn a_failing_pre_commit_hook_refuses_a_document_commit_without_disabling_it() {
        let scratch = Scratch::new("document-hook-fails");
        let _ = record_document(&scratch.0, "warm-up", "hostname = \"warm-up\"\n");
        let hooks_dir = scratch.0.join(".git").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();
        let hook = hooks_dir.join("pre-commit");
        fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
        make_executable(&hook);

        let outcome = record_document(&scratch.0, "rivet-01", "hostname = \"rivet-01\"\n");
        match &outcome {
            Outcome::Refused(why) => assert!(why.contains("commit"), "{why}"),
            other => panic!("a failing hook must refuse rather than succeed: {other:?}"),
        }
        assert!(!log(&scratch.0).contains("rivet-01"), "a refused commit must not land: {}", log(&scratch.0));
    }

    // ---- record_register ------------------------------------------------

    fn a_register(root: &Path) -> Register {
        let mut register = Register::empty(root);
        register.fleet.name = Some("home".to_owned());
        register
            .enroll(Device { id: "0011223344556677".to_owned(), name: "kitchen-pi".to_owned(), ..Device::default() })
            .expect("fresh");
        register
    }

    /// **The register projected in `rivetos_register`'s own layout, and read
    /// back by `rivetos_register` itself** — the house rule `SPEC-05` §4.9
    /// states for the card, applied here: this crate never composes
    /// `register.toml`/`devices/<id>.toml` by hand, so the same reader that
    /// opens the real register opens this copy too.
    #[test]
    fn the_register_projects_in_rivetos_registers_own_layout_and_reads_back() {
        let scratch = Scratch::new("register-projects");
        let register = a_register(&scratch.0.join("real-register"));
        let outcome = record_register(&scratch.0, &register);
        assert!(matches!(outcome, Outcome::Committed(_)), "{outcome:?}");

        let projected_root = scratch.0.join(REGISTER_DIR);
        assert!(projected_root.join(rivetos_register::REGISTER_FILE).is_file());
        assert!(projected_root
            .join(rivetos_register::DEVICES_DIR)
            .join("0011223344556677.toml")
            .is_file());

        let read_back = rivetos_register::store::load(&projected_root).expect("readable");
        assert_eq!(read_back.fleet, register.fleet);
        assert_eq!(read_back.devices, register.devices);
    }

    /// No empty commit for the register either.
    #[test]
    fn projecting_the_same_register_twice_makes_no_second_commit() {
        let scratch = Scratch::new("register-repeat");
        let register = a_register(&scratch.0.join("real-register"));
        let _ = record_register(&scratch.0, &register);
        let before = log(&scratch.0);
        let second = record_register(&scratch.0, &register);
        assert_eq!(second, Outcome::Unchanged);
        assert_eq!(log(&scratch.0), before);
    }

    /// **Self-cleaning.** A device removed from the register disappears from
    /// the projection too, rather than surviving as a stale file forever.
    #[test]
    fn a_device_removed_from_the_register_disappears_from_the_projection() {
        let scratch = Scratch::new("register-remove");
        let mut register = a_register(&scratch.0.join("real-register"));
        let _ = record_register(&scratch.0, &register);
        let projected_devices = scratch.0.join(REGISTER_DIR).join(rivetos_register::DEVICES_DIR);
        assert!(projected_devices.join("0011223344556677.toml").is_file());

        register.devices.remove("0011223344556677");
        let outcome = record_register(&scratch.0, &register);
        match outcome {
            Outcome::Committed(message) => assert!(message.contains("removed"), "{message}"),
            other => panic!("{other:?}"),
        }
        assert!(!projected_devices.join("0011223344556677.toml").exists());
    }

    /// **What a host runs survives the projection** (`ADR-052` §7). A record
    /// naming each slot's system update is loaded from a register on disk,
    /// exactly as `rivet-fleet` loads one, and projected: both digests reach
    /// the fleet repository's copy, which is the one a publish's prune reads.
    /// Hand-composed record text, written the way the register renders one.
    #[test]
    fn what_each_slot_runs_survives_a_projection_of_a_register_read_from_disk() {
        let scratch = Scratch::new("register-system-slots");
        let real = scratch.0.join("real-register");
        let devices = real.join(rivetos_register::DEVICES_DIR);
        fs::create_dir_all(&devices).unwrap();
        let a = "a1".repeat(32);
        let b = "b2".repeat(32);
        fs::write(
            devices.join("0011223344556677.toml"),
            format!("id = \"0011223344556677\"\nname = \"kitchen-pi\"\n\n[system_slots]\na = \"{a}\"\nb = \"{b}\"\n"),
        )
        .unwrap();
        let register = rivetos_register::store::load(&real).expect("the register reads");

        let outcome = record_register(&scratch.0, &register);
        assert!(matches!(outcome, Outcome::Committed(_)), "{outcome:?}");

        let projected = fs::read_to_string(
            scratch.0.join(REGISTER_DIR).join(rivetos_register::DEVICES_DIR).join("0011223344556677.toml"),
        )
        .expect("projected");
        assert!(projected.contains(&a), "slot a's digest must survive the projection:\n{projected}");
        assert!(projected.contains(&b), "slot b's digest must survive the projection:\n{projected}");
    }

    /// `ADR-052` §4 binds the register's projection exactly as it binds a
    /// document — a seed refuses either half of this crate's write path.
    #[test]
    fn a_seed_anywhere_in_the_tree_refuses_the_register_too() {
        let scratch = Scratch::new("register-seed");
        fs::write(scratch.0.join("rivet-system.seed"), b"x").unwrap();
        let register = a_register(&scratch.0.join("real-register"));
        let outcome = record_register(&scratch.0, &register);
        assert!(matches!(outcome, Outcome::Refused(_)), "{outcome:?}");
        assert!(!scratch.0.join(REGISTER_DIR).exists());
    }

    // ---- B04: the register's projection also touches only its own paths --

    /// **An unrelated staged file survives projecting the register**, the
    /// same promise [`recording_a_document_leaves_an_unrelated_staged_file_staged_and_out_of_head`]
    /// proves for a document.
    #[test]
    fn projecting_the_register_leaves_an_unrelated_staged_file_staged_and_out_of_head() {
        let scratch = Scratch::new("register-unrelated-staged");
        let warm_up = a_register(&scratch.0.join("real-register"));
        let _ = record_register(&scratch.0, &warm_up);
        let unrelated = "an unrelated file.txt";
        fs::write(scratch.0.join(unrelated), b"not part of the register").unwrap();
        git(&scratch.0, &["add", "--", unrelated]).unwrap();
        assert!(is_staged(&scratch.0, unrelated), "the fixture itself must be staged");

        let mut register = a_register(&scratch.0.join("real-register"));
        register.fleet.name = Some("changed".to_owned());
        let outcome = record_register(&scratch.0, &register);
        assert!(matches!(outcome, Outcome::Committed(_)), "{outcome:?}");

        assert!(!is_in_head(&scratch.0, unrelated), "an unrelated staged file must not be committed");
        assert!(is_staged(&scratch.0, unrelated), "an unrelated staged file must remain staged");
    }

    /// **A `.seed` file staged OUTSIDE `register/`, then deleted from disk,
    /// never enters the register's commit** — the same B04 scenario
    /// [`a_seed_staged_and_then_deleted_from_disk_never_enters_a_document_commit`]
    /// proves for a document, proven here for the register half, where the
    /// owned pathspec is a whole directory rather than one file.
    #[test]
    fn a_seed_staged_and_then_deleted_from_disk_never_enters_a_register_commit() {
        let scratch = Scratch::new("register-seed-staged-deleted");
        let warm_up = a_register(&scratch.0.join("real-register"));
        let _ = record_register(&scratch.0, &warm_up);
        let seed = scratch.0.join("unrelated.seed");
        fs::write(&seed, b"SYNTHETIC-NOT-A-KEY").unwrap();
        git(&scratch.0, &["add", "--", "unrelated.seed"]).unwrap();
        fs::remove_file(&seed).unwrap();
        assert!(is_staged(&scratch.0, "unrelated.seed"), "the fixture must still be staged though absent from disk");

        let mut register = a_register(&scratch.0.join("real-register"));
        register.fleet.name = Some("changed".to_owned());
        let outcome = record_register(&scratch.0, &register);
        assert!(matches!(outcome, Outcome::Committed(_)), "{outcome:?}");

        assert!(!is_in_head(&scratch.0, "unrelated.seed"), "the seed's path must never reach HEAD");
        assert!(
            !full_history_text(&scratch.0).contains("SYNTHETIC-NOT-A-KEY"),
            "the seed's content must never appear in any commit"
        );
    }

    // ---- staged_seed, directly ---------------------------------------------
    //
    // The two integration tests just above cover B04's acceptance case: a
    // `.seed` staged OUTSIDE the operation's own pathspec, then deleted from
    // disk. `staged_seed` itself exists for the narrower case the brief also
    // names — a name that IS inside the pathspec — and that case is tested
    // directly here rather than through `record_register`, because `git add`
    // given a DIRECTORY pathspec already stages away a staged-then-deleted
    // path under it before `staged_seed` ever runs (proven while writing
    // this fix: reverting the fix and re-running an integration test built
    // for that case left it passing regardless, because `record_register`'s
    // own `git add -- register` had already cleaned the index by the time
    // the check would have fired). `staged_seed` is still correct, and still
    // this brief's explicit ask, for whichever call site or future edit
    // does not enjoy that same accident.

    /// [`staged_seed`] finds a `.seed` file about to be ADDED within its
    /// pathspec.
    #[test]
    fn staged_seed_finds_an_added_seed_within_its_pathspec() {
        let scratch = Scratch::new("staged-seed-added");
        git(&scratch.0, &["init", "--quiet"]).unwrap();
        fs::create_dir_all(scratch.0.join("sub")).unwrap();
        fs::write(scratch.0.join("sub").join("a.seed"), b"x").unwrap();
        git(&scratch.0, &["add", "--", "sub/a.seed"]).unwrap();
        let found = staged_seed(&scratch.0, &["sub"]).expect("git ran");
        assert_eq!(found, Some(scratch.0.join("sub/a.seed")));
    }

    /// An ordinary staged file within the same pathspec is not flagged.
    #[test]
    fn staged_seed_ignores_an_ordinary_file() {
        let scratch = Scratch::new("staged-seed-ordinary");
        git(&scratch.0, &["init", "--quiet"]).unwrap();
        fs::create_dir_all(scratch.0.join("sub")).unwrap();
        fs::write(scratch.0.join("sub").join("a.toml"), b"x").unwrap();
        git(&scratch.0, &["add", "--", "sub/a.toml"]).unwrap();
        assert_eq!(staged_seed(&scratch.0, &["sub"]).expect("git ran"), None);
    }

    /// **A pure removal of an already-committed `.seed` is never flagged** —
    /// [`clear_records`] must be free to remove a seed some earlier defect
    /// let through, and refusing that removal would be exactly backwards.
    #[test]
    fn staged_seed_ignores_a_pure_deletion() {
        let scratch = Scratch::new("staged-seed-deletion");
        git(&scratch.0, &["init", "--quiet"]).unwrap();
        git(&scratch.0, &["config", "user.email", "test@example.com"]).unwrap();
        git(&scratch.0, &["config", "user.name", "Test"]).unwrap();
        fs::create_dir_all(scratch.0.join("sub")).unwrap();
        fs::write(scratch.0.join("sub").join("a.seed"), b"x").unwrap();
        git(&scratch.0, &["add", "--", "sub/a.seed"]).unwrap();
        git(&scratch.0, &["commit", "--quiet", "-m", "a seed, committed by the fixture on purpose"]).unwrap();
        git(&scratch.0, &["rm", "--quiet", "--", "sub/a.seed"]).unwrap();
        assert_eq!(staged_seed(&scratch.0, &["sub"]).expect("git ran"), None);
    }

    // ---- the write paths this crate must never touch --------------------

    /// **The register's own root is untouched by projecting it elsewhere.**
    /// `record_register` is handed a `Register` whose `root` already points
    /// at the real register location; this asserts the projection never
    /// writes there, which is this crate's whole "projection, not a move"
    /// argument made concrete rather than only argued in the module doc.
    #[test]
    fn projecting_the_register_never_writes_to_its_own_root() {
        let scratch = Scratch::new("register-untouched");
        let real_root = scratch.0.join("real-register");
        let register = a_register(&real_root);
        let _ = record_register(&scratch.0.join("fleet-repo"), &register);
        assert!(!real_root.exists(), "the real register root must not be created by a projection");
    }

    // ---- ensure_repository always starts a fresh root on main -------------

    /// **A workstation whose own `init.defaultBranch` names something other
    /// than `main` must not change which branch an automatically created
    /// fleet repository starts on.** `git`'s own "global" configuration is
    /// pointed at a synthetic file naming `trunk` and a synthetic identity,
    /// isolated to this thread by [`GitEnvGuard`] so no other test's `git`
    /// calls are affected — proving this is not merely inheriting whatever
    /// the host machine's own global config already says.
    #[test]
    fn record_register_and_record_document_land_a_fresh_root_on_main_regardless_of_the_workstations_default_branch()
    {
        let scratch = Scratch::new("default-branch-trunk");
        let global_config = scratch.0.join("global-gitconfig");
        fs::write(
            &global_config,
            "[init]\n\tdefaultBranch = trunk\n[user]\n\tname = Test\n\temail = test@example.com\n",
        )
        .unwrap();
        let _guard = GitEnvGuard::set(vec![
            ("GIT_CONFIG_GLOBAL".to_owned(), global_config.to_string_lossy().into_owned()),
            ("GIT_CONFIG_NOSYSTEM".to_owned(), "1".to_owned()),
        ]);

        let register_root = scratch.0.join("via-register");
        let outcome =
            record_register(&register_root, &a_register(&scratch.0.join("real-register-for-branch")));
        assert!(matches!(outcome, Outcome::Committed(_)), "{outcome:?}");
        assert_eq!(head_ref(&register_root), "refs/heads/main");

        let document_root = scratch.0.join("via-document");
        let outcome = record_document(&document_root, "rivet-01", "hostname = \"rivet-01\"\n");
        assert!(matches!(outcome, Outcome::Committed(_)), "{outcome:?}");
        assert_eq!(head_ref(&document_root), "refs/heads/main");
    }

    /// **An existing repository on another branch keeps its branch and its
    /// history.** `ensure_repository` only runs `git init` when `.git` is
    /// absent; this proves the short-circuit for an already-initialised
    /// repository leaves both the branch and the pre-existing commit alone.
    #[test]
    fn an_existing_repository_on_another_branch_keeps_its_branch_and_history() {
        let scratch = Scratch::new("existing-other-branch");
        let root = scratch.0.join("repo");
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "--quiet", "--initial-branch=develop"]).unwrap();
        git(&root, &["config", "user.name", "Test"]).unwrap();
        git(&root, &["config", "user.email", "test@example.com"]).unwrap();
        fs::write(root.join("README.txt"), "pre-existing history\n").unwrap();
        git(&root, &["add", "README.txt"]).unwrap();
        git(&root, &["commit", "--quiet", "-m", "a pre-existing commit on develop"]).unwrap();
        let before = log(&root);
        assert_eq!(head_ref(&root), "refs/heads/develop", "fixture must actually start on develop");

        let outcome = record_document(&root, "rivet-01", "hostname = \"rivet-01\"\n");
        assert!(matches!(outcome, Outcome::Committed(_)), "{outcome:?}");

        assert_eq!(
            head_ref(&root),
            "refs/heads/develop",
            "an existing repository's own branch must be left alone"
        );
        assert!(log(&root).ends_with(&before), "the pre-existing history must still be there");
    }

    // ---- clear_records ----------------------------------------------------

    /// A repository holding a manifest, as `cargo rivet fleet init` leaves
    /// one, and two documents and a register projection, as the two callers
    /// above leave them.
    fn a_populated_repository(label: &str) -> Scratch {
        let scratch = Scratch::new(label);
        let _ = record_document(&scratch.0, "rivet-01", "hostname = \"rivet-01\"\n");
        let _ = record_document(&scratch.0, "rivet-02", "hostname = \"rivet-02\"\n");
        let _ = record_register(&scratch.0, &a_register(&scratch.0.join("real-register")));
        fs::write(scratch.0.join("manifest.toml"), "# the published artifacts\n").unwrap();
        git(&scratch.0, &["add", "manifest.toml"]).unwrap();
        git(&scratch.0, &["commit", "--quiet", "-m", "A manifest"]).unwrap();
        scratch
    }

    /// **The records go, the manifest stays, and the removal is one commit
    /// that names who made it.**
    #[test]
    fn clearing_removes_the_records_and_keeps_the_manifest_in_one_commit() {
        let scratch = a_populated_repository("clear");
        let before = log(&scratch.0).lines().count();

        let subject = clear_records(&scratch.0, "rivet-reset --keep-authority")
            .expect("a clean repository clears")
            .expect("there were records to clear");
        assert!(subject.starts_with("rivet-reset --keep-authority"), "{subject}");
        assert!(subject.contains("4 machine record(s)"), "{subject}");

        assert!(!scratch.0.join(REGISTER_DIR).exists());
        assert!(!scratch.0.join(DOCUMENTS_DIR).exists());
        assert_eq!(
            fs::read_to_string(scratch.0.join("manifest.toml")).unwrap(),
            "# the published artifacts\n"
        );
        assert_eq!(log(&scratch.0).lines().count(), before + 1);
        assert_eq!(uncommitted(&scratch.0).unwrap(), None, "the tree is clean afterwards");
    }

    /// Clearing an already-clear repository is not an error and makes no
    /// empty commit.
    #[test]
    fn clearing_twice_makes_no_second_commit() {
        let scratch = a_populated_repository("clear-twice");
        let _ = clear_records(&scratch.0, "rivet-reset").expect("clears");
        let before = log(&scratch.0);
        assert_eq!(clear_records(&scratch.0, "rivet-reset").expect("clean"), None);
        assert_eq!(log(&scratch.0), before);
    }

    /// **A dirty tree is refused by name, and nothing is removed.** Neither a
    /// modified tracked file nor an untracked one may be swept into a commit
    /// the reset attributes to itself.
    #[test]
    fn a_dirty_repository_is_refused_and_left_as_it_was() {
        let scratch = a_populated_repository("clear-dirty");
        fs::write(scratch.0.join("manifest.toml"), "# half-published\n").unwrap();
        let why = clear_records(&scratch.0, "rivet-reset").expect_err("dirty");
        assert!(why.contains("uncommitted"), "{why}");
        assert!(why.contains("manifest.toml"), "{why}");
        assert!(scratch.0.join(DOCUMENTS_DIR).join("rivet-01.provisioning.toml").is_file());

        let untracked = a_populated_repository("clear-untracked");
        fs::write(untracked.0.join(DOCUMENTS_DIR).join("new.provisioning.toml"), "x").unwrap();
        assert!(clear_records(&untracked.0, "rivet-reset").is_err());
        assert!(untracked.0.join(REGISTER_DIR).join(rivetos_register::REGISTER_FILE).is_file());
    }
}
