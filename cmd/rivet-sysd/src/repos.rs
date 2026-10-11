//! **Making the repositories on this disk match the document.**
//!
//! `rivet_config::repos` is the declaration and `rivet_core::repo` is the
//! arithmetic; this is the only part that touches a filesystem. It runs on
//! every boot and converges — a second run has nothing to do, which is the
//! property `rivet_core::repo`'s `a_plan_applied_leaves_nothing_to_do` holds.
//!
//! # The three primitives, and why there is no fourth
//!
//! | | |
//! | :-- | :-- |
//! | create | `git init --bare -b main`, then write the identifier |
//! | move | `rename(2)` |
//! | leave alone | everything else |
//!
//! **There is no delete.** Not "there is a delete that is not called here" —
//! this module has no code that removes a repository, so no future edit to a
//! document can reach one. Destroying a repository is
//! `crate::repo_delete_request`'s — a separate operation with its own
//! confirmation, `ADR-012` §7.1's rule about the actions that cannot be taken
//! back — and the call that does it lives over there precisely so that the
//! assertion above can be about *this* file. **This module may not even name
//! those two kernel calls in prose**, because the test that holds the claim is
//! a textual search and is deliberately too crude to be reasoned around.
//!
//! # A failure here never stops a boot
//!
//! Every fault is reported and the next repository is attempted, on
//! `crate::etc`'s account-projection terms exactly: a machine that refused to
//! finish booting because one repository could not be created would be a
//! machine an operator cannot reach to fix the repository. The services that
//! keep it reachable are `rivet-netd`, `rivet-mcpd` and the supervisor, and
//! none of them is here.

use rivet_core::etc;
use rivet_core::git;
use rivet_core::repo::{self, Action, Declared, OnDisk};

use crate::journal::{Journal, Severity, Value};
use crate::kernel::{Errno, Kernel, Program};
use crate::supervise::SELF_NAME;

/// What one convergence did, for the caller and for a test.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Converged {
    pub created: usize,
    pub renamed: usize,
    /// Names claimed by two different repositories. **Nothing was touched.**
    pub collisions: usize,
    /// Directories nobody declared. **Nothing was touched.**
    pub undeclared: usize,
    /// Attempts that failed — a `git init` that would not run, a rename that
    /// would not move. Reported, and the boot continued.
    pub failures: usize,
    /// Repositories brought to `PLAN.md` §53.2's `root:repo 2775`, whether or
    /// not they were already in it. **Counted, not conditional**: the fix is
    /// idempotent and this number says how many trees were walked, which is
    /// the cost, rather than how many were wrong, which nothing here measures.
    pub shared: usize,
}

/// Read what is on the disk under `rivet_core::git::REPOSITORY_ROOT`.
///
/// **A missing root is an empty machine, not a fault** — the same reading
/// `crate::applications` gives a missing application root. Every other error is
/// a fault and is returned, because a card answering `EIO` must not be
/// indistinguishable from a machine with no repositories: that would make the
/// next step *create every declared repository over the top of them*.
fn read_disk(k: &mut dyn Kernel, journal: &mut Journal) -> Result<Vec<OnDisk>, Errno> {
    let entries = match k.list_dir(git::REPOSITORY_ROOT) {
        Ok(entries) => entries,
        Err(Errno::ENOENT) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };

    let mut out = Vec::new();
    for entry in entries {
        // A destroyed repository awaiting deletion (`ADR-017` §3). It was
        // destroyed when it was renamed, so it is not on this disk as far as
        // the document is concerned, and reporting it as undeclared would be
        // telling an operator about something already gone.
        if rivet_core::service::is_set_aside(&entry) {
            continue;
        }
        // A directory that is not `<something>.git` is not a repository this
        // machine made. It is still reported as undeclared, because it is under
        // the repository root and an operator should be told what is there.
        let name = entry.strip_suffix(git::BARE_SUFFIX).unwrap_or(&entry).to_owned();
        let id_path = format!("{}/{entry}/{}", git::REPOSITORY_ROOT, repo::ID_FILE);
        let id = match k.read_file(&id_path) {
            Ok(text) => {
                let trimmed = text.trim().to_owned();
                if rivet_core::identifier::is_identifier(&trimmed) {
                    Some(trimmed)
                } else {
                    // A file that is there and says something else is worth a
                    // sentence: it is how a repository loses its identity, and
                    // it turns a rename into a collision without explanation.
                    note(
                        k,
                        journal,
                        Severity::Warn,
                        "a repository's identifier file does not contain an identifier, so this \
                         machine cannot tell which repository the directory is. It will be \
                         treated as one nobody declared, which means it is LEFT ALONE",
                        vec![
                            ("path".to_string(), Value::Str(id_path)),
                            ("rule".to_string(), Value::Str(rivet_core::identifier::RULE.to_string())),
                        ],
                    );
                    None
                }
            }
            // No identifier is an ordinary answer: something Rivet did not
            // create. `rivet_core::repo::OnDisk::id` carries the argument.
            Err(_) => None,
        };
        out.push(OnDisk { name, id });
    }
    Ok(out)
}

/// Make the disk match the document.
///
/// Returns what happened. **Never returns an error**: a boot that stopped here
/// would take away the machine over a repository.
pub fn converge(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    document: &rivet_config::Document,
) -> Converged {
    let mut out = Converged::default();

    let declared: Vec<Declared> = document
        .repos()
        .iter()
        .map(|r| Declared {
            name: r.name().to_string(),
            id: r.id().to_string(),
        })
        .collect();

    let on_disk = match read_disk(k, journal) {
        Ok(found) => found,
        Err(errno) => {
            // **Not treated as an empty machine.** That reading would create
            // every declared repository over the top of whatever is actually
            // there, which is the one way this module could destroy data.
            note(
                k,
                journal,
                Severity::Error,
                "cannot read the repository directory, so this machine does not know what is on \
                 its own disk. NOTHING HAS BEEN CREATED OR MOVED: treating an unreadable \
                 directory as an empty one would create every declared repository over the top \
                 of whatever is really there",
                vec![
                    ("path".to_string(), Value::Str(git::REPOSITORY_ROOT.to_string())),
                    ("errno".to_string(), Value::Str(errno.to_string())),
                ],
            );
            out.failures += 1;
            return out;
        }
    };

    // Nothing declared and nothing on disk is the ordinary machine. Saying so
    // once a boot would be noise; saying nothing is right.
    if declared.is_empty() && on_disk.is_empty() {
        return out;
    }

    // **The root first, and on every convergence rather than at creation.**
    // `PLAN.md` §53.2 was ruled on 2026-08-27, after `/state/git` already
    // existed on a board — `root:root 0755`, which is the arrangement the
    // ruling replaced. A supervisor that set the mode only when it made the
    // directory could never repair that machine, so this is a convergence
    // target like everything else here (`ADR-017` §3.1).
    //
    // The setgid bit is what makes the rest cheap: every repository git
    // creates underneath inherits `repo` without anything having to say so.
    if !declared.is_empty() {
        share_root(k, journal, &mut out);
    }

    for action in repo::plan(&declared, &on_disk) {
        match &action {
            Action::Create { name, id } => {
                if create(k, journal, name, id) {
                    out.created += 1;
                    note(k, journal, Severity::Info, &action.describe(), fields(&action));
                } else {
                    out.failures += 1;
                }
            }
            Action::Rename { from, to, .. } => {
                if rename(k, journal, from, to) {
                    out.renamed += 1;
                    note(k, journal, Severity::Info, &action.describe(), fields(&action));
                } else {
                    out.failures += 1;
                }
            }
            // Both of these are reports. There is deliberately no branch here
            // that touches a disk.
            Action::Collision { .. } => {
                out.collisions += 1;
                note(k, journal, Severity::Error, &action.describe(), fields(&action));
            }
            Action::Undeclared { .. } => {
                out.undeclared += 1;
                note(k, journal, Severity::Warn, &action.describe(), fields(&action));
            }
        }
    }

    // **Every declared repository, not only the ones just created.** A
    // repository created before the ruling is `root:root` throughout, and
    // `git init --bare --shared=group` does not repair one — measured
    // 2026-08-27, and `rivet_core::git::SHARED_INIT` records what it leaves
    // behind. One that was created a moment ago is already right, and walking
    // it again costs a `chown` and a `chmod` per directory of an empty
    // repository. Doing both through one path means the invariant is stated
    // once rather than depended on twice.
    for d in &declared {
        share_repository(k, journal, &mut out, &d.name);
    }
    out
}

/// Bring `rivet_core::git::REPOSITORY_ROOT` to `root:repo 2775`.
///
/// A failure here is reported and the boot continues — the same reading every
/// other fault in this module gets. What it costs is that repositories created
/// this boot do not inherit the group, which the per-repository walk then fixes
/// anyway; what it would cost to stop is the machine.
fn share_root(k: &mut dyn Kernel, journal: &mut Journal, out: &mut Converged) {
    if let Err(errno) = k.mkdir_p(git::REPOSITORY_ROOT) {
        let root = git::REPOSITORY_ROOT;
        fault(k, journal, "cannot make the repository directory", root, &errno.to_string());
        out.failures += 1;
        return;
    }
    if let Err(errno) = k.chown(git::REPOSITORY_ROOT, ROOT_UID, etc::REPO_GID) {
        fault(k, journal, OWNERSHIP_FAILED, git::REPOSITORY_ROOT, &errno.to_string());
        out.failures += 1;
        return;
    }
    if let Err(errno) = k.chmod(git::REPOSITORY_ROOT, git::SHARED_MODE) {
        fault(k, journal, OWNERSHIP_FAILED, git::REPOSITORY_ROOT, &errno.to_string());
        out.failures += 1;
    }
}

/// Bring one repository's whole tree to `PLAN.md` §53.2's arrangement.
fn share_repository(k: &mut dyn Kernel, journal: &mut Journal, out: &mut Converged, name: &str) {
    let dir = git::repository_dir(name);
    match k.share_tree(&dir, etc::REPO_GID) {
        Ok(()) => out.shared += 1,
        Err(Errno::ENOENT) => {
            // The `create` for it failed earlier this convergence and said so.
            // A second sentence about the same repository would be noise.
        }
        Err(errno) => {
            fault(k, journal, OWNERSHIP_FAILED, name, &format!("{dir}: {errno}"));
            out.failures += 1;
        }
    }
}

/// uid 0, named so that the pair `(ROOT_UID, etc::REPO_GID)` reads as the
/// ruling does — *owned by root, grouped to the accounts*.
const ROOT_UID: u32 = 0;

/// One sentence for every way the ownership fix can fail, because the
/// consequence is the same one and it is not obvious from the errno: git
/// refuses a repository it does not own and could not write to it anyway, so a
/// push is refused with a message about *dubious ownership* that names nothing
/// on this machine.
const OWNERSHIP_FAILED: &str = "cannot give a repository to the account group, so a push \
                                will be refused with `detected dubious ownership`, and the \
                                account could not write to it in any case - PLAN.md 53.2";

/// `git init --bare -b main`, then the identifier.
///
/// **The identifier is written after git succeeds and is checked afterwards.**
/// A repository with no identifier is one this machine cannot recognise on the
/// next boot — it would be reported as undeclared and a second, empty one
/// created beside it — so the failure is worth a sentence of its own rather
/// than being folded into "could not create".
fn create(k: &mut dyn Kernel, journal: &mut Journal, name: &str, id: &str) -> bool {
    let dir = git::repository_dir(name);

    // The root, which does not exist on a machine that has never had one.
    if let Err(errno) = k.mkdir_p(git::REPOSITORY_ROOT) {
        fault(k, journal, "cannot make the repository directory", name, &errno.to_string());
        return false;
    }

    let program = Program {
        path: git::BINARY.to_string(),
        args: vec![
            "init".to_string(),
            "--bare".to_string(),
            // **What git will keep the repository as**, distinct from what
            // this module sets it to. See `rivet_core::git::SHARED_INIT`:
            // without it, the `receive-pack` a push runs creates its objects
            // `0755` and breaks the arrangement for the next push.
            git::SHARED_INIT.to_string(),
            "-b".to_string(),
            repo::DEFAULT_BRANCH.to_string(),
            dir.clone(),
        ],
        env: Vec::new(),
        // Root. A repository is served by `cmd/rivet-gitd`, which runs as the
        // account whatever admitted the client authenticated.
        //
        // **That account's access was ruled on 2026-08-27** — `PLAN.md` §53.2,
        // `root:repo` mode `2775` — and the ruling is why this stays `None`
        // rather than becoming the account: the group is what grants access,
        // and root is what owns it, because there is no per-repository access
        // control and *every* account reaches *every* repository. Creating them
        // as root is also what the rest of `/state` already does.
        identity: None,
        confinement: None,
        groups: Vec::new(),
        capabilities: Vec::new(),
    };

    match k.run_to_completion(&program) {
        Ok(done) if done.status.is_success() => {}
        Ok(done) => {
            // git said why, and an exit code alone is what makes a board
            // session necessary — `crate::precondition`'s reasoning exactly.
            fault(
                k,
                journal,
                "git could not create a repository",
                name,
                &format!("{} said: {}", git::BINARY, said(&done.output)),
            );
            return false;
        }
        Err(errno) => {
            fault(
                k,
                journal,
                "cannot run git, so no repository can be created on this machine",
                name,
                &format!("{}: {errno}", git::BINARY),
            );
            return false;
        }
    }

    // The identifier, without which this repository is unrecognisable on the
    // next boot.
    //
    // **Written to a sibling and renamed** since 2026-09-04, for the audit of
    // 2026-09-03. The truncate-in-place this replaced left a window in which
    // the identifier file existed and was empty, and nothing was flushed — so a
    // power cut here produced exactly the outcome the message below describes,
    // *without* the message, on the next boot. `crate::durable` carries the
    // sequence.
    let id_path = format!("{dir}/{}", repo::ID_FILE);
    match crate::durable::commit(k, &id_path, format!("{id}\n").as_bytes(), ID_FILE_MODE) {
        Ok(crate::durable::Durability::Flushed) => {}
        // The file is in place and readable; what was not established is that
        // its name survives a power cut. That is not a failed creation — the
        // repository exists and serves — so it is said rather than returned.
        Ok(crate::durable::Durability::NameNotFlushed(errno)) => note(
            k,
            journal,
            Severity::Warn,
            "a repository's identifier was written and the directory holding it could not be \
             flushed, so a power cut before the next flush can lose the file - the repository \
             would then be reported as undeclared on the next boot",
            vec![
                ("repository".to_string(), Value::Str(name.to_string())),
                ("detail".to_string(), Value::Str(format!("{id_path}: {errno}"))),
            ],
        ),
        Err(errno) => {
            fault(
                k,
                journal,
                "a repository was created and its identifier could not be written, so this \
                 machine will not recognise it on the next boot - it will be reported as \
                 undeclared and a second, empty one created beside it. Remove the directory or \
                 write the file by hand",
                name,
                &format!("{id_path}: {errno}"),
            );
            return false;
        }
    }
    true
}

/// The identifier file's mode — readable, and writable only by root.
///
/// `0644` rather than `0600`. **The mode is unchanged and its reason is not**:
/// it used to be that `cmd/rivet-gitd` ran as the authenticated account and
/// nothing stopped that account reading it. That program is gone (`ADR-035`
/// §13) and the control plane serves git as root, so nothing but root reads
/// this today. It stays `0644` on the surviving half of the argument — it is
/// not a secret, so making it unreadable would only mean a future reader has to
/// become root for no reason.
/// It is not a secret — `ADR-025` §4.1, and `rivet_core::identifier`'s header.
const ID_FILE_MODE: u32 = 0o644;

/// Move a repository. `rename(2)`, and nothing else.
fn rename(k: &mut dyn Kernel, journal: &mut Journal, from: &str, to: &str) -> bool {
    let (from_dir, to_dir) = (git::repository_dir(from), git::repository_dir(to));
    if let Err(errno) = k.rename(&from_dir, &to_dir) {
        fault(
            k,
            journal,
            "cannot move a repository to the name the document gives it, so it is still \
             answering under the old one",
            from,
            &format!("{from_dir} -> {to_dir}: {errno}"),
        );
        return false;
    }
    true
}

/// The journal fields one action carries.
fn fields(action: &Action) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    match action {
        Action::Create { name, id } => {
            out.push(("repository".to_string(), Value::Str(name.clone())));
            out.push(("id".to_string(), Value::Str(id.clone())));
            out.push(("branch".to_string(), Value::Str(repo::DEFAULT_BRANCH.to_string())));
        }
        Action::Rename { from, to, id } => {
            out.push(("from".to_string(), Value::Str(from.clone())));
            out.push(("to".to_string(), Value::Str(to.clone())));
            out.push(("id".to_string(), Value::Str(id.clone())));
        }
        Action::Collision { name, wanted, found } => {
            out.push(("repository".to_string(), Value::Str(name.clone())));
            out.push(("declared".to_string(), Value::Str(wanted.clone())));
            if let Some(found) = found {
                out.push(("found".to_string(), Value::Str(found.clone())));
            }
        }
        Action::Undeclared { name, id } => {
            out.push(("repository".to_string(), Value::Str(name.clone())));
            if let Some(id) = id {
                out.push(("id".to_string(), Value::Str(id.clone())));
            }
        }
    }
    out
}

/// One line of what a program said, for a journal field.
fn said(output: &[(crate::kernel::Stream, String)]) -> String {
    output
        .iter()
        .map(|(_, line)| line.as_str())
        .collect::<Vec<_>>()
        .join("; ")
}

fn fault(k: &mut dyn Kernel, journal: &mut Journal, msg: &str, name: &str, detail: &str) {
    note(
        k,
        journal,
        Severity::Error,
        msg,
        vec![
            ("repository".to_string(), Value::Str(name.to_string())),
            ("detail".to_string(), Value::Str(detail.to_string())),
        ],
    );
}

fn note(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    sev: Severity,
    msg: &str,
    fields: Vec<(String, Value)>,
) {
    let corr = journal.next_correlation();
    journal.emit(k, sev, SELF_NAME, &corr, msg, fields);
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::kernel::{ExitStatus, Stream};
    use crate::testkernel::FakeKernel;

    const A: &str = "4f2c9a01bd7e3865";
    const B: &str = "a1b2c3d4e5f60718";

    fn document(toml: &str) -> rivet_config::Document {
        rivet_config::Document::parse(toml).expect("the fixture is a valid document")
    }

    fn declaring(name: &str, id: &str) -> rivet_config::Document {
        document(&format!("version = 1\n[repos.{name}]\nid = \"{id}\"\n"))
    }

    fn run(k: &mut FakeKernel, doc: &rivet_config::Document) -> Converged {
        let mut journal = Journal::new("b".to_string(), 1, crate::supervise::ACTOR.to_string());
        converge(k, &mut journal, doc)
    }

    /// Put a repository on the fake's disk.
    ///
    /// **The fake does not model a filesystem**, and this is deliberate rather
    /// than a gap: it models the *calls*, so a test asserts what Rivet asked
    /// the kernel to do. `git init` creating a directory and `rename(2)` moving
    /// a tree are the kernel's business and are proved on a board (`AGENTS.md`
    /// §6), not here.
    fn on_disk(k: &mut FakeKernel, name: &str, id: Option<&str>) {
        let dir = format!("{name}.git");
        k.dirs_mut()
            .entry(git::REPOSITORY_ROOT.to_string())
            .or_default()
            .push(dir.clone());
        if let Some(id) = id {
            k.files_mut().insert(
                format!("{}/{dir}/{}", git::REPOSITORY_ROOT, repo::ID_FILE),
                crate::testkernel::Script::always(Ok(format!("{id}\n"))),
            );
        }
    }

    /// An empty repository root, which is what a machine that has served git
    /// before but has none right now looks like.
    fn empty_root(k: &mut FakeKernel) {
        k.dirs_mut().insert(git::REPOSITORY_ROOT.to_string(), Vec::new());
    }

    // ---- creating ----------------------------------------------------------

    /// **`git init --bare --shared=group -b main`, and nothing else.** The
    /// arguments are asserted rather than the outcome, because the outcome is
    /// git's and the arguments are Rivet's.
    ///
    /// `--shared=group` arrived with `PLAN.md` §53.2 on 2026-08-27 and is not
    /// cosmetic: it writes `core.sharedRepository` into the repository, and
    /// without it the `receive-pack` a push runs creates its own objects `0755`
    /// and breaks the arrangement for the next push. It is the half git keeps;
    /// `Kernel::share_tree` is the half this program sets.
    #[test]
    fn a_declared_repository_is_created_with_this_projects_default_branch() {
        let mut k = FakeKernel::new();
        empty_root(&mut k);
        let done = run(&mut k, &declaring("example-repo", A));

        assert_eq!(done.created, 1, "{done:?}");
        assert_eq!(done.failures, 0, "{done:?}");

        assert_eq!(k.ran.len(), 1, "{:?}", k.ran);
        assert_eq!(k.ran[0].path, git::BINARY);
        assert_eq!(
            k.ran[0].args,
            ["init", "--bare", "--shared=group", "-b", "main", "/state/git/example-repo.git"]
        );
        // Never `master`, and never git's compiled-in default by omission.
        assert!(!k.ran[0].args.iter().any(|a| a == "master"), "{:?}", k.ran[0].args);
    }

    /// The identifier is what makes the repository recognisable on the next
    /// boot. Without it the machine would create a second, empty one beside it.
    #[test]
    fn a_created_repository_carries_its_identifier_on_disk() {
        let mut k = FakeKernel::new();
        empty_root(&mut k);
        run(&mut k, &declaring("example-repo", A));

        let written = k
            .contents("/state/git/example-repo.git/rivet-id")
            .expect("the identifier was written");
        assert_eq!(written.trim(), A);
        // World-readable because it is NOT A SECRET. It also used to be read
        // by `rivet-gitd` as the authenticated account; that program is gone
        // and the mode rests on the first reason alone.
        //
        // **The write itself lands on the sibling temp** since 2026-09-04 and
        // the identifier gets its contents by `rename(2)` — see the test below.
        // The mode is asserted on that write, because a rename preserves it.
        let call = k
            .writes
            .iter()
            .find(|w| w.path.ends_with(TEMPORARY_ID))
            .expect("the write was recorded");
        assert_eq!(call.mode, 0o644);
    }

    /// The temp the identifier is committed through — a **sibling**, which is
    /// what makes the rename atomic, and a fixed name, so one left by a power
    /// cut is overwritten rather than accumulating (`ADR-018` §7).
    const TEMPORARY_ID: &str = "rivet-id.new";

    /// **The regression test for the audit of 2026-09-03.**
    ///
    /// The identifier was written by truncate-in-place with no flush of either
    /// the file or its directory. A power cut in that window leaves the file
    /// present and empty, or absent altogether — and this module's own failure
    /// message already says what that costs: the repository is reported as
    /// undeclared on the next boot and a second, empty one is created beside
    /// it. Losing it silently to a power cut is the same outcome without the
    /// message.
    ///
    /// The assertion is on the calls, because the two versions are
    /// indistinguishable on a machine that does not lose power.
    #[test]
    fn a_repositorys_identifier_is_never_truncated_and_both_flushes_happen() {
        let mut k = FakeKernel::new();
        empty_root(&mut k);
        run(&mut k, &declaring("example-repo", A));

        let target = format!("/state/git/example-repo.git/{}", repo::ID_FILE);
        let temp = format!("/state/git/example-repo.git/{TEMPORARY_ID}");
        assert!(
            !k.truncations.contains(&target),
            "the identifier was opened O_TRUNC: {:?}",
            k.truncations
        );
        assert!(k.syncs.contains(&temp), "the bytes were not flushed: {:?}", k.syncs);
        assert!(
            k.renames.contains(&(temp, target)),
            "the identifier was not committed by rename: {:?}",
            k.renames
        );
        assert!(
            k.dir_syncs.contains(&"/state/git/example-repo.git".to_string()),
            "the directory entry was not flushed: {:?}",
            k.dir_syncs
        );
    }

    /// A machine that has never served git has no root, and creating one is
    /// part of creating the first repository rather than a separate step
    /// somebody has to remember.
    #[test]
    fn a_machine_that_has_never_served_git_gets_its_repository_root() {
        let mut k = FakeKernel::new();
        let done = run(&mut k, &declaring("example-repo", A));
        assert_eq!(done.created, 1, "{done:?}");
        assert!(k.mkdir_calls.iter().any(|p| p == git::REPOSITORY_ROOT), "{:?}", k.mkdir_calls);
    }

    // ---- converging --------------------------------------------------------

    /// **Converging.** A boot that finds the repository already there runs no
    /// git and moves nothing — which is what makes this safe on every boot.
    ///
    /// **It is no longer "does nothing", and the rename of this test is the
    /// record of why.** `PLAN.md` §53.2 made ownership a convergence target on
    /// 2026-08-27: the board it was ruled for already had a repository, created
    /// `root:root` by the code that shipped the day before, and a supervisor
    /// that left an existing repository alone would leave that machine refusing
    /// every push for ever. So the ownership walk runs on every boot and the
    /// two claims that mattered — no git, no rename — are asserted individually
    /// rather than through `Converged::default()`.
    #[test]
    fn a_boot_that_finds_the_repository_already_there_creates_nothing_and_still_shares_it() {
        let mut k = FakeKernel::new();
        on_disk(&mut k, "example-repo", Some(A));

        let done = run(&mut k, &declaring("example-repo", A));
        assert_eq!(done.created, 0, "{done:?}");
        assert_eq!(done.renamed, 0, "{done:?}");
        assert_eq!(done.failures, 0, "{done:?}");
        assert_eq!(done.shared, 1, "{done:?}");
        assert!(k.ran.is_empty(), "{:?}", k.ran);
        assert!(k.renames.is_empty(), "{:?}", k.renames);
        assert_eq!(
            k.share_trees,
            vec![(git::repository_dir("example-repo"), etc::REPO_GID)],
            "{:?}",
            k.share_trees
        );
        // And the root itself, which is the half that makes every repository
        // created later inherit the group without being told.
        assert_eq!(
            k.chmods,
            vec![(git::REPOSITORY_ROOT.to_string(), git::SHARED_MODE)],
            "{:?}",
            k.chmods
        );
        assert!(
            k.chowns.contains(&(git::REPOSITORY_ROOT.to_string(), 0, etc::REPO_GID)),
            "{:?}",
            k.chowns
        );
    }

    /// The repair half, stated on its own: a tree that is walked is walked with
    /// the group `PLAN.md` §53.2 named, and a failure to do it is a failure
    /// rather than a silence. A push refused with `detected dubious ownership`
    /// names nothing on this machine, so the journal has to.
    #[test]
    fn a_tree_that_cannot_be_shared_is_a_failure_with_a_sentence() {
        let mut k = FakeKernel::new();
        on_disk(&mut k, "example-repo", Some(A));
        k.share_tree_results = crate::testkernel::Script::always(Err(Errno::EPERM));

        let done = run(&mut k, &declaring("example-repo", A));
        assert_eq!(done.shared, 0, "{done:?}");
        assert_eq!(done.failures, 1, "{done:?}");
        assert!(
            k.records.iter().any(|r| r.msg.contains("dubious ownership")),
            "{:?}",
            k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
    }

    /// A repository whose `create` failed is not reported twice. The `git init`
    /// that would not run already said so, and a second sentence about the
    /// directory it did not leave behind describes a consequence rather than a
    /// cause.
    #[test]
    fn a_repository_that_was_never_created_is_not_reported_missing_as_well() {
        let mut k = FakeKernel::new();
        k.share_tree_results = crate::testkernel::Script::always(Err(Errno::ENOENT));

        let done = run(&mut k, &declaring("example-repo", A));
        assert_eq!(done.shared, 0, "{done:?}");
        assert_eq!(done.failures, 0, "{done:?}");
        assert!(
            !k.records.iter().any(|r| r.msg.contains("dubious ownership")),
            "{:?}",
            k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
        );
    }

    /// A machine with no repositories and no declarations says nothing. A line
    /// per boot about the absence of repositories is noise.
    #[test]
    fn a_machine_with_no_repositories_says_nothing() {
        let mut k = FakeKernel::new();
        let done = run(&mut k, &document("version = 1\n"));
        assert_eq!(done, Converged::default());
        assert!(k.ran.is_empty());
        assert!(k.records.is_empty(), "{:?}", k.records);
    }

    // ---- renaming ----------------------------------------------------------

    /// **The reason the table carries an identifier.** Renaming the table moves
    /// the directory; it does not create an empty repository and orphan the old
    /// one, which is what a name-keyed projection would do with somebody's
    /// history.
    #[test]
    fn renaming_the_declaration_moves_the_directory_and_runs_no_git() {
        let mut k = FakeKernel::new();
        on_disk(&mut k, "old-name", Some(A));

        let done = run(&mut k, &declaring("new-name", A));
        assert_eq!(done.renamed, 1, "{done:?}");
        assert_eq!(done.created, 0, "{done:?}");
        assert_eq!(done.undeclared, 0, "{done:?}");
        assert!(k.ran.is_empty(), "git ran for a rename: {:?}", k.ran);
        assert_eq!(
            k.renames,
            [(
                "/state/git/old-name.git".to_string(),
                "/state/git/new-name.git".to_string()
            )]
        );
    }

    /// A rename that will not move is reported and the repository keeps
    /// answering under its old name — which is the safe direction, and is said
    /// in those words.
    #[test]
    fn a_rename_that_fails_says_the_repository_is_still_at_the_old_name() {
        let mut k = FakeKernel::new();
        on_disk(&mut k, "old-name", Some(A));
        k.rename_results = crate::testkernel::Script::always(Err(Errno::EIO));

        let done = run(&mut k, &declaring("new-name", A));
        assert_eq!(done.failures, 1, "{done:?}");
        assert_eq!(done.renamed, 0, "{done:?}");
        assert!(
            journalled(&k).iter().any(|m| m.contains("still")),
            "{:?}",
            journalled(&k)
        );
    }

    // ---- the two that never touch a disk -----------------------------------

    /// **Undeclaring deletes nothing.**
    #[test]
    fn undeclaring_a_repository_leaves_every_byte_of_it_alone() {
        let mut k = FakeKernel::new();
        on_disk(&mut k, "example-repo", Some(A));

        let done = run(&mut k, &document("version = 1\n"));
        assert_eq!(done.undeclared, 1, "{done:?}");
        assert_eq!(done.failures, 0, "{done:?}");
        assert!(k.removed_paths().is_empty(), "{:?}", k.removed_paths());
        assert!(k.removed.is_empty(), "{:?}", k.removed);
        assert!(k.renames.is_empty(), "{:?}", k.renames);
        assert!(
            journalled(&k).iter().any(|m| m.contains("LEFT EXACTLY WHERE IT IS")),
            "{:?}",
            journalled(&k)
        );
    }

    /// **A destroyed repository awaiting deletion is not on this disk** as
    /// far as the document is concerned (`ADR-017` §3): not undeclared, not
    /// reported, and not touched.
    #[test]
    fn a_destroyed_repository_awaiting_deletion_is_not_reported_as_undeclared() {
        let mut k = FakeKernel::new();
        k.dirs_mut().insert(
            git::REPOSITORY_ROOT.to_string(),
            vec![format!("{}5-1", rivet_core::service::RECLAIM_MARK)],
        );

        let done = run(&mut k, &document("version = 1\n"));
        assert_eq!(done.undeclared, 0, "{done:?}");
        assert!(k.renames.is_empty(), "{:?}", k.renames);
        assert!(
            !journalled(&k).iter().any(|m| m.contains("LEFT EXACTLY WHERE IT IS")),
            "{:?}",
            journalled(&k)
        );
    }

    /// **There is no code here that removes a repository.** Asserted against
    /// the source, because the alternative is asserting that something did not
    /// happen in one scenario — and the claim is about every scenario.
    #[test]
    fn this_module_cannot_delete_a_repository_at_all() {
        let source = include_str!("repos.rs");
        let body = source.split("#[cfg(test)]").next().expect("a module body");
        // **Deliberately crude.** A search for a call expression could be
        // argued with; a search for the identifier cannot, which is why this
        // module may not name either call even in a comment. It fired once, on
        // the day `crate::repo_delete_request` was written and this header
        // gained a sentence about where the removal went.
        assert!(!body.contains("remove_tree"), "this module can remove a tree");
        assert!(!body.contains("remove_file"), "this module can remove a file");
        assert!(!body.contains("reclaim"), "this module can set a tree aside");
    }

    /// A name claimed by a different repository: **nothing is touched**, and
    /// both identifiers are in the record.
    #[test]
    fn a_collision_touches_nothing_and_names_both_identifiers() {
        let mut k = FakeKernel::new();
        on_disk(&mut k, "example-repo", Some(A));

        let done = run(&mut k, &declaring("example-repo", B));
        assert_eq!(done.collisions, 1, "{done:?}");
        assert_eq!(done.created, 0, "{done:?}");
        assert_eq!(done.renamed, 0, "{done:?}");
        assert!(k.ran.is_empty(), "{:?}", k.ran);
        assert!(k.renames.is_empty(), "{:?}", k.renames);

        let said = journalled(&k);
        assert!(said.iter().any(|m| m.contains("NOTHING HAS BEEN DONE")), "{said:?}");
        assert!(said.iter().any(|m| m.contains(A)), "{said:?}");
        assert!(said.iter().any(|m| m.contains(B)), "{said:?}");
    }

    /// A directory with no identifier under a declared name is **not adopted**
    /// — adopting it would hand somebody's stray directory an identity and a
    /// URL.
    #[test]
    fn an_unidentified_directory_under_a_declared_name_is_not_adopted() {
        let mut k = FakeKernel::new();
        on_disk(&mut k, "example-repo", None);

        let done = run(&mut k, &declaring("example-repo", A));
        assert_eq!(done.collisions, 1, "{done:?}");
        assert!(k.ran.is_empty(), "git ran over a directory it did not create");
    }

    /// An identifier file holding something that is not an identifier is worth
    /// a sentence: it is how a repository loses its identity, and it silently
    /// turns a rename into a collision.
    #[test]
    fn an_identifier_file_that_is_not_an_identifier_is_reported() {
        let mut k = FakeKernel::new();
        on_disk(&mut k, "example-repo", Some("not-an-identifier"));

        let done = run(&mut k, &declaring("example-repo", A));
        assert_eq!(done.collisions, 1, "{done:?}");
        assert!(
            journalled(&k)
                .iter()
                .any(|m| m.contains("does not contain an identifier")),
            "{:?}",
            journalled(&k)
        );
    }

    // ---- failures the machine survives -------------------------------------

    /// **An unreadable root is not an empty machine.** Reading it as one would
    /// create every declared repository over the top of what is really there,
    /// which is the single way this module could destroy data.
    #[test]
    fn an_unreadable_repository_directory_creates_nothing() {
        let mut k = FakeKernel::new();
        k.dir_errors_mut().insert(git::REPOSITORY_ROOT.to_string(), Errno::EIO);

        let done = run(&mut k, &declaring("example-repo", A));
        assert_eq!(done.failures, 1, "{done:?}");
        assert_eq!(done.created, 0, "{done:?}");
        assert!(k.ran.is_empty(), "git ran against an unreadable disk: {:?}", k.ran);
        assert!(
            journalled(&k).iter().any(|m| m.contains("NOTHING HAS BEEN CREATED OR MOVED")),
            "{:?}",
            journalled(&k)
        );
    }

    /// git failing is reported with **what git said**, because an exit code
    /// alone is what makes a board session necessary.
    #[test]
    fn a_failing_git_is_reported_with_what_it_said() {
        let mut k = FakeKernel::new();
        empty_root(&mut k);
        k.run_results = crate::testkernel::Script::always(Ok(ExitStatus::Exited(128)));
        k.run_says = vec![(
            Stream::Stderr,
            "fatal: cannot mkdir: Read-only file system".to_string(),
        )];

        let done = run(&mut k, &declaring("example-repo", A));
        assert_eq!(done.failures, 1, "{done:?}");
        assert_eq!(done.created, 0, "{done:?}");
        assert!(
            journalled(&k).iter().any(|m| m.contains("Read-only file system")),
            "{:?}",
            journalled(&k)
        );
    }

    /// A repository created whose identifier could not be written is the one
    /// failure that gets its own sentence, because the next boot would create a
    /// second empty one beside it.
    #[test]
    fn a_repository_whose_identifier_could_not_be_written_says_what_happens_next() {
        let mut k = FakeKernel::new();
        empty_root(&mut k);
        k.write_results = crate::testkernel::Script::always(Err(Errno::ENOSPC));

        let done = run(&mut k, &declaring("example-repo", A));
        assert_eq!(done.failures, 1, "{done:?}");
        assert_eq!(done.created, 0, "{done:?}");
        assert!(
            journalled(&k)
                .iter()
                .any(|m| m.contains("second, empty one created beside it")),
            "{:?}",
            journalled(&k)
        );
    }

    /// A failure on one repository does not stop the next — a boot that
    /// stopped here would take away the machine over a repository.
    #[test]
    fn one_repository_failing_does_not_stop_the_others() {
        let mut k = FakeKernel::new();
        on_disk(&mut k, "taken", Some(B));

        let done = run(
            &mut k,
            &document(&format!(
                "version = 1\n[repos.taken]\nid = \"{A}\"\n[repos.fresh]\nid = \"0000000000000001\"\n"
            )),
        );
        assert_eq!(done.collisions, 1, "{done:?}");
        assert_eq!(done.created, 1, "{done:?}");
    }

    /// Everything this module put in the journal, message and fields together.
    ///
    /// **Both, because a refusal is not only its sentence.** What git said goes
    /// in a field, and a test that read the message alone would pass against a
    /// machine that reported "git could not create a repository" and nothing
    /// else — which is exactly the diagnostic that makes a board session
    /// necessary.
    fn journalled(k: &FakeKernel) -> Vec<String> {
        k.records
            .iter()
            .map(|r| {
                let fields: Vec<String> = r
                    .fields
                    .iter()
                    .map(|(name, value)| format!("{name}={value:?}"))
                    .collect();
                format!("{} {}", r.msg, fields.join(" "))
            })
            .collect()
    }
}
