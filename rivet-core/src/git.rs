//! **Where git lives on a Rivet machine**, and the two settings it is always
//! given.
//!
//! `ADR-032` §4.2 ruled the account; `PLAN.md` §43 is the survey that chose the
//! mechanism. Nothing in this module does any I/O — it holds the facts that
//! three separate programs would otherwise each spell for themselves, which is
//! `ADR-011` in the place it bites hardest: **`cmd/rivet-image` writes a binary
//! to a path, `cmd/rivet-sysd` creates repositories under a root, and
//! `git-http-backend` runs the one and serves out of the other.** A disagreement
//! between any two of those is a machine that boots, passes its tests, and
//! cannot serve a repository.
//!
//! # Why there is a C binary here at all
//!
//! `ADR-009` §6's Tier B: Rivet writes the policy and reuses the mechanism.
//! `PLAN.md` §43's survey found that `gix`, the obvious Rust answer, is 93
//! crates and **has no receive-pack side at all** — the half that serves a push,
//! which is the whole job. So git is a **vendored binary the image ships and
//! Rivet drives**, and what this module owns is the handful of paths and names
//! that would otherwise be spelled in two crates.

/// The one git binary, at `/system/bin/git`.
///
/// **One file, not a directory of 171.** Buildroot installs
/// `usr/libexec/git-core` with 171 entries of which 147 are hardlinks to this
/// same 3.7 MB inode; copying that tree into an image naively costs 554 MB.
/// Rivet installs the single executable, and can, because everything it needs
/// is a **builtin** reached as `git <verb>` — measured on 2026-08-26 by tracing
/// a real push and finding every subprocess `receive-pack` spawns is `git
/// <builtin>` and nothing else.
///
/// `/system` and not `/usr/bin`: `ADR-008` §4 — the system image is one
/// read-only tree and there is no `/usr`.
pub const BINARY: &str = "/system/bin/git";

/// `git-http-backend` — git's own smart-HTTP server, `ADR-035` §4.
///
/// # It is a SEPARATE binary, and that is the whole reason this constant exists
///
/// [`BINARY`]'s note records that every subprocess `receive-pack` spawns is a
/// **builtin** reached as `git <verb>`, so one file serves them all. **This is
/// not one of them.** Measured 2026-08-28 (`PLAN.md` §84.1): in Buildroot's
/// tree `git` and `git-upload-pack` share an inode and `git-http-backend` does
/// not — it is 2,330,088 bytes of its own, which is why `git http-backend` on a
/// card carrying only [`BINARY`] answered *"not a git command"*.
///
/// # What it needs, which is one thing and is easy to get wrong
///
/// **`git` on `PATH`.** It does not reach `upload-pack` through `--exec-path`;
/// it runs `git` and lets that dispatch the builtin, which is [`BINARY`]'s
/// property working one level up. `PLAN.md` §85.2 measured the failure and it
/// is nearly silent: with no `PATH` it emits a **valid but empty**
/// advertisement and puts `error: cannot run upload-pack` on stderr, so a
/// client reports having cloned an empty repository rather than an error.
pub const HTTP_BACKEND: &str = "/system/bin/git-http-backend";

/// Where repositories live.
///
/// **On `/state`, because a repository is the machine's data and not its
/// software.** `ADR-018`'s A/B slots replace `/system` wholesale on every
/// update; a repository under it would be destroyed by the next one, and would
/// also be on a filesystem `ADR-008` §4 mounts read-only.
///
/// # The volume is not yet a choice, and that is a known gap
///
/// The maintainer ruled on 2026-08-26 that on a machine with more than one
/// volume, git should be able to live on the second — so that a repository
/// cannot fill the disk the updater and the journal need. `ADR-031` already
/// made application paths volume-parameterised (`crate::service::app_dir` takes
/// a volume), so the route exists and this constant is what a `[repos]` volume
/// key will displace. **Until it does, a single-volume machine has no isolation
/// between a push and its own free space**, which is written here rather than
/// discovered.
pub const REPOSITORY_ROOT: &str = "/state/git";

/// The suffix a bare repository's directory carries.
///
/// Convention rather than requirement — git will serve a bare repository from
/// any directory — and it is honoured because every client already strips it
/// from a URL, so a directory without it makes a request for `x.git` and one
/// for `x` name different things. They must name one.
pub const BARE_SUFFIX: &str = ".git";

/// The directory one repository occupies.
///
/// **The only place a repository path is composed.** The control plane calls this
/// with a name it has already checked has no separator in it, and
/// `cmd/rivet-sysd` calls it with a name the document declared; neither builds
/// the string itself, so neither can build a different one.
#[must_use]
pub fn repository_dir(name: &str) -> String {
    format!("{REPOSITORY_ROOT}/{name}{BARE_SUFFIX}")
}

/// The mode a repository directory carries, and the mode of the root they sit
/// in — `PLAN.md` §53.2's ruling, in one number.
///
/// **`2775`, owned `root:repo`**, ruled by the maintainer 2026-08-27. `775`
/// is so that a member of [`crate::etc::REPO_GROUP`] may create entries, which
/// is what a push does; the leading `2` is **setgid**, and it is the half that
/// makes the arrangement survive. A directory with setgid gives its own group
/// to everything created inside it, so a repository created under a setgid
/// root belongs to `repo` without anything having to remember to say so — and
/// so does every directory git creates inside *that*.
///
/// It is applied explicitly rather than passed to `mkdir(2)`, because
/// `mkdir(2)` masks the mode with the process umask and would silently produce
/// `2775 & ~umask` — the same reason `Kernel::mkdir_mode` sets it afterwards.
pub const SHARED_MODE: u32 = 0o2775;

/// `git init`'s own name for the same arrangement.
///
/// **Both are needed and neither is redundant.** [`SHARED_MODE`] is what the
/// directories *are*; this is what git will *keep* them as — it writes
/// `core.sharedRepository` into the repository's config, and every later
/// invocation, including the `receive-pack` a push runs, then creates its own
/// files and directories group-writable instead of `0755`. Without the config
/// a push would fix the permissions of the objects it wrote and break them for
/// the next one.
///
/// **What it does not do is repair an existing repository**, which was
/// measured rather than assumed on 2026-08-27: re-running `git init --bare
/// --shared=group` over a repository created without it brings `$GIT_DIR` and
/// `refs` to `2775` and leaves **`objects` at `755`** — the one directory a
/// push must write. `crate::git` therefore cannot repair by re-initialising,
/// and `Kernel::share_tree` exists for it.
pub const SHARED_INIT: &str = "--shared=group";

/// The settings git is given on every invocation, as `-c` arguments.
///
/// **On the command line rather than in a repository's `config`**, because a
/// setting written into a repository is one an operator can change by pushing
/// to it, and both of these are the machine's decision rather than the
/// repository's.
///
/// * **`receive.denyNonFastForwards=false`** — force pushes are allowed. Ruled
///   by the maintainer 2026-08-26. A household fleet's owner rewriting his own
///   branch is the ordinary case, not the dangerous one.
/// * **`receive.autogc=false`** — found by tracing a real push (`PLAN.md`
///   §43.6): `receive-pack` otherwise forks `git maintenance run --auto
///   --detach`, a background process `ADR-017`'s supervisor did not spawn, does
///   not know about and cannot stop. That is exactly what §3.1 exists to
///   prevent, and it would appear only under load, on a machine that had been
///   running for weeks.
pub const SETTINGS: &[&str] = &[
    "receive.denyNonFastForwards=false",
    "receive.autogc=false",
];

/// Why every invocation names `safe.directory`, and why that is not the option
/// `PLAN.md` §53.2 ruled out.
///
/// **§53.2's table treated `root:<group> 2775` and `safe.directory` as
/// competing answers, and they are answers to different questions.** That was
/// found by running a push rather than by reading git: on 2026-08-27 a QEMU
/// guest with the ruling fully built — `/state/git` `root:repo 2775`, the
/// repository walked by `Kernel::share_tree`, the account a member of `repo` —
/// authenticated, reached `cmd/rivet-gitd`, ran the vendored git, and was
/// refused with
///
/// > `fatal: detected dubious ownership in repository at
/// > '/state/git/smoke-repo.git'`
///
/// **git's ownership check compares the directory's OWNER UID with the caller's
/// euid.** A group cannot satisfy it, because a group is not an owner. So the
/// two halves are:
///
/// * **`root:repo`, mode `2775`** — what makes the account *able* to write. It
///   is the ruling, and nothing here touches it.
/// * **`safe.directory`** — what makes git *willing* to. Without it a
///   repository owned by root is refused before any permission is consulted,
///   and the refusal names a `git config --global` command an operator cannot
///   usefully run on a machine with no per-user git configuration.
///
/// §53.2 said of the third option that it *"solves the message, not the
/// problem"*. That is right about `safe.directory` **alone** — it would leave
/// an account that git permits and the filesystem refuses — and it is what
/// makes the pairing correct rather than a way around the ruling.
///
/// **Scoped to the one repository, never `*`.** The claim being made is *this
/// machine intends root to own the repository this account is pushing to*, and
/// a wildcard would make it about every repository on every path, including
/// ones nothing here created.
///
/// **On the command line, which is `protected` scope.** git honours
/// `safe.directory` only from system, global and `-c` configuration — never
/// from a repository's own config, which is the whole point of the check. `-c`
/// is one of the three, and it is also where [`SETTINGS`] already lives for the
/// reason stated there: a setting written into a repository is one an operator
/// can change by pushing to it.
///
/// The arguments that run one verb against one repository.
///
/// Composed here so that the settings cannot be omitted by a caller that
/// assembles its own argument list and forgets one — which is the failure mode
/// `SETTINGS` being a `const` slice would otherwise still permit.
#[must_use]
pub fn arguments(verb: &str, name: &str) -> Vec<String> {
    let dir = repository_dir(name);
    let mut args = Vec::with_capacity(SETTINGS.len() * 2 + 4);
    for setting in SETTINGS {
        args.push("-c".to_owned());
        args.push((*setting).to_owned());
    }
    // **`safe.directory`, and it is the other half of `PLAN.md` 53.2 rather
    // than the alternative that section ruled out.** See [`SAFE_DIRECTORY`].
    args.push("-c".to_owned());
    args.push(format!("safe.directory={dir}"));
    args.push(verb.to_owned());
    args.push(dir);
    args
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// A repository is one directory under one root, and the composition is
    /// asserted rather than trusted — three programs depend on it agreeing.
    #[test]
    fn a_repository_is_one_directory_under_the_root() {
        assert_eq!(repository_dir("example-repo"), "/state/git/example-repo.git");
        assert!(repository_dir("x").starts_with(REPOSITORY_ROOT));
        assert!(repository_dir("x").ends_with(BARE_SUFFIX));
    }

    /// **On `/state`, never under `/system`.** A repository beneath the system
    /// image would be destroyed by the next A/B update and would be on a
    /// read-only filesystem in the meantime.
    #[test]
    fn repositories_are_not_under_the_system_image() {
        assert!(REPOSITORY_ROOT.starts_with("/state"));
        assert!(!REPOSITORY_ROOT.starts_with("/system"));
        // The binary is the other way round: software, replaced by an update.
        assert!(BINARY.starts_with("/system/"));
    }

    /// The settings cannot be dropped by a caller assembling its own arguments.
    #[test]
    fn every_invocation_carries_both_settings() {
        let args = arguments("receive-pack", "example-repo");
        for setting in SETTINGS {
            assert!(args.iter().any(|a| a == setting), "{setting} missing");
        }
        // `SETTINGS` plus `safe.directory`, which is composed rather than
        // constant because it names the repository.
        assert_eq!(args.iter().filter(|a| *a == "-c").count(), SETTINGS.len() + 1);
        assert_eq!(args.last().map(String::as_str), Some("/state/git/example-repo.git"));
    }

    /// **The half `PLAN.md` 53.2's table did not have.** A QEMU guest with the
    /// whole ruling built was still refused with `detected dubious ownership`,
    /// because git compares the directory's OWNER UID with the caller's euid
    /// and a group is not an owner. See `arguments`' own documentation.
    #[test]
    fn every_invocation_declares_the_repository_safe_by_its_own_path() {
        for name in ["example-repo", "smoke-repo", "a_b-c.d"] {
            let args = arguments("receive-pack", name);
            let want = format!("safe.directory={}", repository_dir(name));
            assert!(args.iter().any(|a| *a == want), "{want} missing from {args:?}");
        }
    }

    /// Never a wildcard. The claim is about the one repository being served,
    /// not about every path a future caller might hand this program.
    #[test]
    fn the_safe_directory_claim_is_never_a_wildcard() {
        let args = arguments("receive-pack", "example-repo");
        assert!(
            !args.iter().any(|a| a.starts_with("safe.directory=") && a.contains('*')),
            "{args:?}"
        );
    }

    /// `receive-pack` forks a `git maintenance --detach` the supervisor did not
    /// spawn unless this is off. Named so that removing it is a visible change.
    #[test]
    fn autogc_is_off_because_the_supervisor_would_not_know_about_the_process() {
        assert!(SETTINGS.contains(&"receive.autogc=false"));
    }

    /// Force pushes are allowed. Ruled by the maintainer 2026-08-26.
    #[test]
    fn force_pushes_are_allowed_by_ruling() {
        assert!(SETTINGS.contains(&"receive.denyNonFastForwards=false"));
    }
}
