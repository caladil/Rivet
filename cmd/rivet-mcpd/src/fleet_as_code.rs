//! **Fleet-as-code's git operations, run as root against this machine's own
//! repository** — `ADR-044`, `ADR-035`.
//!
//! Four routes beside `crate::bulk`, `crate::git` and `crate::repository` on
//! the same listener. `rivet_core::fleet_as_code` holds the formats, the path
//! rule and the refusals; this module checks the caller, finds the repository
//! and runs git's plumbing.
//!
//! # Where git runs
//!
//! Against `rivet_core::git::repository_dir(<name>)`, composed from a name the
//! request's parser has already accepted as a repository name. The caller never
//! supplies a path to a repository, and a document path is always
//! `fleet/machines/<label>.toml` (`rivet_core::fleet_as_code::DocumentPath`).
//! git runs with its environment replaced, `safe.directory` naming the one
//! repository, and pathspecs literal.
//!
//! # How a commit is made without a working tree
//!
//! The repository is bare and served to clients at the same time. A commit is
//! git's plumbing and touches no checkout: the document is written as a blob, a
//! private index file is read from the expected tip and has that one path
//! replaced, the tree is written, a commit is made on the tip, and
//! `update-ref <branch> <new> <expected>` moves the branch **only if it still
//! names the expected tip**. A tip that moved between the check and the update
//! is refused as a stale head, and the objects written for the lost commit are
//! unreachable and harmless.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use rivet_core::fleet_as_code::{
    self as wire, Branch, Caller, CommitId, CommitRequest, Committed, DocumentPath, FileRequest,
    Head, HeadRequest, Kind, Machines, MachinesRequest, Operation, Refusal, Request,
};
use rivet_core::repo::RepoName;

use crate::bulk::Response;

/// Whether a request target reaches this module.
#[must_use]
pub fn is_fleet_as_code(target: &str) -> bool {
    let path = target.split('?').next().unwrap_or(target);
    path.starts_with(wire::PREFIX) || path == wire::PREFIX.trim_end_matches('/')
}

/// Where this machine's repositories are and which git reads them.
///
/// **The root is a parameter** so a test works in a temporary directory and can
/// never reach `/state/git`.
#[derive(Debug, Clone)]
pub struct Machine {
    /// `None` on a machine: the path comes from `rivet_core::git::repository_dir`.
    root: Option<String>,
    git: String,
    /// `Some(PATH)`: git runs with a replaced environment carrying only this
    /// `PATH`. `None` inherits this process's environment, for a test on a host
    /// whose git is wherever that host put it.
    git_path: Option<String>,
}

impl Machine {
    /// The repositories on this machine, and its vendored git.
    #[must_use]
    pub fn on_this_machine() -> Self {
        Self {
            root: None,
            git: rivet_core::git::BINARY.to_owned(),
            git_path: Some(
                rivet_core::git::BINARY
                    .rsplit_once('/')
                    .map_or(rivet_core::git::BINARY, |(dir, _)| dir)
                    .to_owned(),
            ),
        }
    }

    /// Repositories under `root`, and whichever `git` this host runs. For a test.
    #[cfg(test)]
    #[must_use]
    pub fn at(root: &Path) -> Self {
        Self { root: Some(root.to_string_lossy().into_owned()), git: "git".to_owned(), git_path: None }
    }

    fn repository_dir(&self, name: &RepoName) -> String {
        match &self.root {
            None => rivet_core::git::repository_dir(name.as_str()),
            Some(root) => format!("{root}/{}{}", name.as_str(), rivet_core::git::BARE_SUFFIX),
        }
    }
}

/// **One request**, from its head and whole body to its answer.
///
/// `caller` is `None` when the connection presented no pinned key — the
/// plaintext loopback transport — and such a caller is refused.
#[must_use]
pub fn serve(machine: &Machine, method: &str, target: &str, body: &[u8], caller: Option<Caller>) -> Response {
    match answer(machine, method, target, body, caller) {
        Ok(text) => Response { status: 200, reason: "OK", body: text },
        Err(refusal) => {
            let (status, reason) = refusal.kind.status();
            Response { status, reason, body: refusal.render() }
        }
    }
}

fn answer(
    machine: &Machine,
    method: &str,
    target: &str,
    body: &[u8],
    caller: Option<Caller>,
) -> Result<String, Refusal> {
    let path = target.split('?').next().unwrap_or(target);
    let Some(operation) = Operation::from_route(path).filter(|_| method == "POST") else {
        let routes: Vec<&str> = Operation::ALL.iter().map(|op| op.route()).collect();
        return Err(Refusal::new(
            Kind::NotAllowed,
            format!(
                "{method} {target} is not one of fleet-as-code's operations. They are POST {}.",
                routes.join(", POST ")
            ),
        ));
    };

    let Some(caller) = caller else {
        return Err(Refusal::new(
            Kind::Forbidden,
            "this connection presented no pinned key, and fleet-as-code's repository answers only \
             a key in this machine's peers. Connect over the control plane's TLS.",
        ));
    };
    if !wire::permits(caller, operation.access()) {
        return Err(Refusal::new(
            Kind::Forbidden,
            format!("{caller:?} may not {:?} fleet-as-code's repository.", operation.access()),
        ));
    }

    match Request::parse(operation, body)? {
        Request::Head(request) => head(machine, &request).map(|head| head.render()),
        Request::Machines(request) => machines(machine, &request).map(|m| m.render()),
        Request::File(request) => file(machine, &request).map(|file| file.render()),
        Request::Commit(request) => commit(machine, &request).map(|done| done.render()),
    }
}

/// The repository's directory, refused when it is not there.
fn existing(machine: &Machine, name: &RepoName) -> Result<String, Refusal> {
    let dir = machine.repository_dir(name);
    if Path::new(&dir).is_dir() {
        Ok(dir)
    } else {
        Err(Refusal::new(
            Kind::NoRepository,
            format!(
                "{dir} is absent: this machine's document declares no [repos.{name}], or \
                 rivet-sysd has not created it yet",
                name = name.as_str()
            ),
        ))
    }
}

/// `POST /fleet-as-code/head`.
fn head(machine: &Machine, request: &HeadRequest) -> Result<Head, Refusal> {
    let repo = existing(machine, &request.repository)?;
    let commit = tip(machine, &repo, &request.branch)?.ok_or_else(|| no_branch(&request.repository, &request.branch))?;
    let raw = run(machine, &repo, &["cat-file", "commit", commit.as_str()], None, &[])?;
    let text = String::from_utf8_lossy(&raw);
    let message = text.split_once("\n\n").map_or("", |(_, message)| message).to_owned();
    Ok(Head { commit, message })
}

/// `POST /fleet-as-code/machines`.
fn machines(machine: &Machine, request: &MachinesRequest) -> Result<Machines, Refusal> {
    let repo = existing(machine, &request.repository)?;
    must_exist(machine, &repo, &request.commit)?;
    let listing = format!("{}/", wire::MACHINES_DIR);
    let raw = run(machine, &repo, &["ls-tree", "-z", request.commit.as_str(), &listing], None, &[])?;
    let mut out = Machines::default();
    for entry in tree_entries(&raw) {
        if entry.kind != "blob" {
            continue;
        }
        let Some(file) = entry.path.strip_prefix(&listing) else {
            continue;
        };
        let Some(label) = file.strip_suffix(wire::DOCUMENT_SUFFIX) else {
            continue;
        };
        if wire::check_label(label).is_ok() {
            out.labels.push(label.to_owned());
        } else {
            out.ignored.push(format!("{file:?}"));
        }
    }
    Ok(out)
}

/// `POST /fleet-as-code/file`.
fn file(machine: &Machine, request: &FileRequest) -> Result<wire::File, Refusal> {
    let repo = existing(machine, &request.repository)?;
    must_exist(machine, &repo, &request.commit)?;
    let Some(entry) = entry_at(machine, &repo, request.commit.as_str(), &request.path)? else {
        return Ok(wire::File::Absent);
    };
    let blob = document_blob(&entry, &request.path)?;
    let size = run(machine, &repo, &["cat-file", "-s", &blob], None, &[])?;
    let size = String::from_utf8_lossy(&size).trim().parse::<usize>().unwrap_or(usize::MAX);
    if size > wire::MAX_DOCUMENT_BYTES {
        return Err(Refusal::new(
            Kind::TooLarge,
            format!("{} is {size} bytes, past the {}-byte limit", request.path.as_path(), wire::MAX_DOCUMENT_BYTES),
        ));
    }
    let bytes = run(machine, &repo, &["cat-file", "blob", &blob], None, &[])?;
    let text = String::from_utf8(bytes).map_err(|_| {
        Refusal::new(Kind::NotADocument, format!("{} is not UTF-8 text, so it is not a machine document", request.path.as_path()))
    })?;
    Ok(wire::File::Present(text))
}

/// `POST /fleet-as-code/commit`.
fn commit(machine: &Machine, request: &CommitRequest) -> Result<Committed, Refusal> {
    let repo = existing(machine, &request.repository)?;
    let current = tip(machine, &repo, &request.branch)?.ok_or_else(|| no_branch(&request.repository, &request.branch))?;
    if current != request.expected_head {
        return Err(stale(&request.branch, &request.expected_head, &current));
    }

    let path = request.path.as_path();
    let mode = match entry_at(machine, &repo, current.as_str(), &request.path)? {
        Some(entry) => {
            document_blob(&entry, &request.path)?;
            entry.mode
        }
        None => "100644".to_owned(),
    };

    let blob = text(&run(machine, &repo, &["hash-object", "-w", "--no-filters", "--stdin"], Some(request.document.as_bytes()), &[])?);
    let tree = {
        let index = PrivateIndex::new(&repo);
        let env = [("GIT_INDEX_FILE", index.path.as_str())];
        run(machine, &repo, &["read-tree", current.as_str()], None, &env)?;
        let info = format!("{mode},{blob},{path}");
        run(machine, &repo, &["update-index", "--add", "--cacheinfo", &info], None, &env)?;
        text(&run(machine, &repo, &["write-tree"], None, &env)?)
    };
    let before = text(&run(machine, &repo, &["rev-parse", &format!("{}^{{tree}}", current.as_str())], None, &[])?);
    if tree == before {
        return Err(Refusal::new(
            Kind::NothingToCommit,
            format!("{path} at the tip of {} is already this document, so nothing was committed", request.branch.as_str()),
        ));
    }

    let mut message = request.message.clone();
    if !message.ends_with('\n') {
        message.push('\n');
    }
    let identity = [
        ("GIT_AUTHOR_NAME", wire::COMMITTER_NAME),
        ("GIT_AUTHOR_EMAIL", wire::COMMITTER_EMAIL),
        ("GIT_COMMITTER_NAME", wire::COMMITTER_NAME),
        ("GIT_COMMITTER_EMAIL", wire::COMMITTER_EMAIL),
    ];
    let made = text(&run(machine, &repo, &["commit-tree", &tree, "-p", current.as_str()], Some(message.as_bytes()), &identity)?);
    let made = CommitId::new(&made).map_err(|refusal| Refusal::new(Kind::Failed, format!("git commit-tree answered {made:?}: {}", refusal.why)))?;

    let reference = request.branch.reference();
    if let Err(failed) = run(machine, &repo, &["update-ref", &reference, made.as_str(), current.as_str()], None, &[]) {
        // **The tip moved between the check and the update**, which is the race
        // the compare-and-swap exists for. Anything else is git's own failure.
        return match tip(machine, &repo, &request.branch)? {
            Some(now) if now != current => Err(stale(&request.branch, &request.expected_head, &now)),
            _ => Err(failed),
        };
    }
    Ok(Committed { commit: made })
}

fn no_branch(repository: &RepoName, branch: &Branch) -> Refusal {
    Refusal::new(
        Kind::NoBranch,
        format!(
            "the {} repository has no branch {} yet. A first commit reaches it by a git push over \
             the control plane (ADR-035 section 4); nothing is committed onto a missing branch",
            repository.as_str(),
            branch.as_str()
        ),
    )
}

fn stale(branch: &Branch, expected: &CommitId, actual: &CommitId) -> Refusal {
    Refusal::new(
        Kind::StaleHead,
        format!(
            "REFUSED, nothing was committed: {} is at {} and this commit was composed against {}. \
             Somebody else's commit landed in between; read the document again and compose the \
             change against what is there now",
            branch.as_str(),
            actual.as_str(),
            expected.as_str()
        ),
    )
}

/// The commit a branch names, or `None` when the branch is absent.
fn tip(machine: &Machine, repo: &str, branch: &Branch) -> Result<Option<CommitId>, Refusal> {
    let spec = format!("{}^{{commit}}", branch.reference());
    match run(machine, repo, &["rev-parse", "--verify", "--quiet", &spec], None, &[]) {
        Ok(raw) if !text(&raw).is_empty() => {
            let id = text(&raw);
            CommitId::new(&id)
                .map(Some)
                .map_err(|refusal| Refusal::new(Kind::Failed, format!("git rev-parse answered {id:?}: {}", refusal.why)))
        }
        _ => Ok(None),
    }
}

fn must_exist(machine: &Machine, repo: &str, commit: &CommitId) -> Result<(), Refusal> {
    let spec = format!("{}^{{commit}}", commit.as_str());
    run(machine, repo, &["cat-file", "-e", &spec], None, &[]).map(|_| ()).map_err(|_| {
        Refusal::new(Kind::NoCommit, format!("there is no commit {} in this repository", commit.as_str()))
    })
}

/// One `ls-tree -z` line.
struct Entry {
    mode: String,
    kind: String,
    object: String,
    path: String,
}

fn tree_entries(raw: &[u8]) -> Vec<Entry> {
    raw.split(|b| *b == 0)
        .filter(|record| !record.is_empty())
        .filter_map(|record| {
            let record = String::from_utf8_lossy(record);
            let (meta, path) = record.split_once('\t')?;
            let mut fields = meta.split(' ');
            Some(Entry {
                mode: fields.next()?.to_owned(),
                kind: fields.next()?.to_owned(),
                object: fields.next()?.to_owned(),
                path: path.to_owned(),
            })
        })
        .collect()
}

/// The tree entry at exactly `path` in `commit`, or `None`.
fn entry_at(machine: &Machine, repo: &str, commit: &str, path: &DocumentPath) -> Result<Option<Entry>, Refusal> {
    let wanted = path.as_path();
    let raw = run(machine, repo, &["ls-tree", "-z", commit, &wanted], None, &[])?;
    Ok(tree_entries(&raw).into_iter().find(|entry| entry.path == wanted))
}

/// The blob an entry names, refused when it is not a regular file.
fn document_blob(entry: &Entry, path: &DocumentPath) -> Result<String, Refusal> {
    if entry.kind == "blob" && (entry.mode == "100644" || entry.mode == "100755") {
        Ok(entry.object.clone())
    } else {
        Err(Refusal::new(
            Kind::NotADocument,
            format!("{} is a {} of mode {}, not a regular file", path.as_path(), entry.kind, entry.mode),
        ))
    }
}

fn text(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw).trim().to_owned()
}

/// An index file of this request's own, inside the repository, removed when
/// dropped whatever happened.
struct PrivateIndex {
    path: String,
}

impl PrivateIndex {
    fn new(repo: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        Self { path: format!("{repo}/rivet-fleet-as-code-{}-{n}.index", std::process::id()) }
    }
}

impl Drop for PrivateIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(format!("{}.lock", self.path));
    }
}

/// One `git` against one repository, with nothing a home directory or a
/// daemon's environment could add, and git's stdout back.
///
/// # Errors
///
/// [`Kind::Failed`], carrying git's own stderr.
fn run(machine: &Machine, repo: &str, args: &[&str], stdin: Option<&[u8]>, env: &[(&str, &str)]) -> Result<Vec<u8>, Refusal> {
    let mut command = Command::new(&machine.git);
    if let Some(path) = &machine.git_path {
        // **Replaced, not extended**: an inherited `GIT_DIR` or `GIT_INDEX_FILE`
        // would point this at something other than the repository named here.
        command.env_clear().env("PATH", path).env("HOME", "/nonexistent");
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_LITERAL_PATHSPECS", "1")
        .envs(env.iter().copied())
        .arg("-c")
        .arg(format!("safe.directory={repo}"))
        .arg("--git-dir")
        .arg(repo)
        .args(args)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let failed = |why: String| Refusal::new(Kind::Failed, why);
    let mut child = command.spawn().map_err(|e| failed(format!("cannot run {}: {e}", machine.git)))?;
    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(bytes).map_err(|e| failed(format!("git {} would not take its input: {e}", args.join(" "))))?;
    }
    let output = child.wait_with_output().map_err(|e| failed(format!("git {}: {e}", args.join(" "))))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(failed(format!(
            "`git {}` failed ({}): {}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

#[cfg(test)]
#[path = "fleet_as_code_tests.rs"]
mod tests;
