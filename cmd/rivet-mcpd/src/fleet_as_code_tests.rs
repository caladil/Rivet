//! Fleet-as-code's operations, end to end through [`serve`], against a real
//! bare git repository in a temporary directory.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use super::*;

const REPO: &str = "machines";
const RIVET_01: &str = "version = 1\nhostname = \"rivet-01\"\n";

/// A root holding `machines.git`, seeded with one commit carrying
/// `fleet/machines/rivet-01.toml`, a file outside the directory, a document
/// that is a directory, a name that is not a label and a file that is not a
/// document.
struct Scratch {
    root: PathBuf,
    machine: Machine,
    first: String,
}

fn git_ok(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(["-c", "user.name=test", "-c", "user.email=test@example.invalid", "-c", "init.defaultBranch=main"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("cannot run git {args:?}: {e}"));
    assert!(output.status.success(), "git {args:?} in {}: {}", dir.display(), String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

impl Scratch {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!("rivet-mcpd-fleet-as-code-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a root");
        let bare = root.join(format!("{REPO}.git"));
        git_ok(&root, &["init", "--bare", "--initial-branch=main", &bare.to_string_lossy()]);

        let work = root.join("work");
        git_ok(&root, &["init", "--initial-branch=main", &work.to_string_lossy()]);
        let write = |path: &str, text: &str| {
            let at = work.join(path);
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::write(at, text).unwrap();
        };
        write("fleet/machines/rivet-01.toml", RIVET_01);
        write("fleet/machines/README.md", "not a document\n");
        write("fleet/machines/.hidden.toml", "version = 1\n");
        write("fleet/machines/sub/deeper.toml", "version = 1\n");
        write("fleet/machines/a-directory.toml/inside", "x\n");
        write("etc/passwd.toml", "root:x:0:0\n");
        git_ok(&work, &["add", "-A"]);
        git_ok(&work, &["commit", "-q", "-m", "seed"]);
        git_ok(&work, &["remote", "add", "origin", &bare.to_string_lossy()]);
        git_ok(&work, &["push", "-q", "origin", "main"]);
        let first = git_ok(&work, &["rev-parse", "HEAD"]);
        let machine = Machine::at(&root);
        Self { root, machine, first }
    }

    fn bare(&self) -> PathBuf {
        self.root.join(format!("{REPO}.git"))
    }

    fn post(&self, route: &str, body: &[u8]) -> Response {
        serve(&self.machine, "POST", route, body, Some(Caller::Operator))
    }

    fn send(&self, request: &Request) -> Response {
        self.post(request.operation().route(), &request.render())
    }

    fn branch_tip(&self) -> String {
        git_ok(&self.bare(), &["rev-parse", "refs/heads/main"])
    }

    fn commit(&self, expected: &str, path: &str, message: &str, document: &str) -> Response {
        self.send(&Request::Commit(CommitRequest {
            repository: RepoName::new(REPO).unwrap(),
            branch: Branch::new("main").unwrap(),
            expected_head: CommitId::new(expected).unwrap(),
            path: DocumentPath::new(path).unwrap(),
            message: message.to_owned(),
            document: document.to_owned(),
        }))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn refused(response: &Response) -> Refusal {
    Refusal::parse(&response.body).unwrap_or_else(|| panic!("not a refusal: {} {}", response.status, response.body))
}

fn head_body(repository: &str) -> Vec<u8> {
    format!("repository {repository}\nbranch main\n").into_bytes()
}

// ---- who may ask, and what is a route ---------------------------------------

/// **A connection with no pinned key reaches no operation**, reads included.
#[test]
fn a_caller_with_no_pinned_key_is_refused_every_operation() {
    let scratch = Scratch::new("nobody");
    let before = scratch.branch_tip();
    for op in Operation::ALL {
        let response = serve(&scratch.machine, "POST", op.route(), &head_body(REPO), None);
        assert_eq!(response.status, 403, "{op:?}: {}", response.body);
        assert_eq!(refused(&response).kind, Kind::Forbidden);
    }
    assert_eq!(scratch.branch_tip(), before);
}

#[test]
fn a_method_or_route_that_is_not_an_operation_is_refused_by_name() {
    let scratch = Scratch::new("routes");
    for (method, target) in [
        ("GET", "/fleet-as-code/head"),
        ("PUT", "/fleet-as-code/commit"),
        ("POST", "/fleet-as-code/"),
        ("POST", "/fleet-as-code/push"),
    ] {
        let response = serve(&scratch.machine, method, target, &head_body(REPO), Some(Caller::Operator));
        assert_eq!(response.status, 405, "{method} {target}: {}", response.body);
        assert!(response.body.contains("POST /fleet-as-code/commit"), "{}", response.body);
    }
    assert!(is_fleet_as_code("/fleet-as-code/head"));
    assert!(is_fleet_as_code("/fleet-as-code"));
    assert!(!is_fleet_as_code("/repository-objects/"));
    assert!(!is_fleet_as_code("/machines.git/info/refs"));
}

// ---- the repository ---------------------------------------------------------

#[test]
fn a_repository_that_is_not_on_this_machine_is_not_found() {
    let scratch = Scratch::new("missing");
    let response = scratch.post(Operation::Head.route(), &head_body("elsewhere"));
    assert_eq!(response.status, 404, "{}", response.body);
    let refusal = refused(&response);
    assert_eq!(refusal.kind, Kind::NoRepository);
    assert!(refusal.why.contains("[repos.elsewhere]"), "{}", refusal.why);
}

/// `ADR-052`'s repository is refused by name even when it exists.
#[test]
fn the_fleet_repository_is_refused_even_where_it_exists() {
    let scratch = Scratch::new("fleet-name");
    std::fs::create_dir_all(scratch.root.join("fleet.git")).unwrap();
    let response = scratch.post(Operation::Head.route(), &head_body("fleet"));
    assert_eq!(response.status, 400, "{}", response.body);
    assert!(response.body.contains("cargo rivet fleet push"), "{}", response.body);
}

#[test]
fn an_empty_repository_has_no_branch_and_commits_nothing() {
    let scratch = Scratch::new("empty");
    git_ok(&scratch.root, &["init", "--bare", "--initial-branch=main", "empty.git"]);
    let response = scratch.post(Operation::Head.route(), &head_body("empty"));
    assert_eq!(response.status, 404, "{}", response.body);
    assert_eq!(refused(&response).kind, Kind::NoBranch);
}

// ---- reads ------------------------------------------------------------------

#[test]
fn the_head_names_the_tip_and_carries_the_whole_message() {
    let scratch = Scratch::new("head");
    let response = scratch.post(Operation::Head.route(), &head_body(REPO));
    assert_eq!(response.status, 200, "{}", response.body);
    let head = Head::parse(&response.body).expect("a head");
    assert_eq!(head.commit.as_str(), scratch.first);
    assert_eq!(head.message.trim(), "seed");
}

/// Only regular `.toml` files directly in `fleet/machines/` whose name is a
/// label; a name that is not one is reported, never read.
#[test]
fn the_listing_names_every_machine_document_and_nothing_else() {
    let scratch = Scratch::new("listing");
    let request = Request::Machines(MachinesRequest {
        repository: RepoName::new(REPO).unwrap(),
        commit: CommitId::new(&scratch.first).unwrap(),
    });
    let response = scratch.send(&request);
    assert_eq!(response.status, 200, "{}", response.body);
    let listing = Machines::parse(&response.body).expect("a listing");
    assert_eq!(listing.labels, vec!["rivet-01".to_owned()]);
    assert_eq!(listing.ignored, vec!["\".hidden.toml\"".to_owned()]);
}

#[test]
fn a_document_is_read_at_a_commit_and_an_absent_one_is_an_answer() {
    let scratch = Scratch::new("file");
    let read = |path: &str, commit: &str| {
        scratch.send(&Request::File(FileRequest {
            repository: RepoName::new(REPO).unwrap(),
            commit: CommitId::new(commit).unwrap(),
            path: DocumentPath::new(path).unwrap(),
        }))
    };
    let present = read("fleet/machines/rivet-01.toml", &scratch.first);
    assert_eq!(wire::File::parse(&present.body).unwrap(), wire::File::Present(RIVET_01.to_owned()));
    let absent = read("fleet/machines/kitchen-pi.toml", &scratch.first);
    assert_eq!(wire::File::parse(&absent.body).unwrap(), wire::File::Absent);

    let directory = read("fleet/machines/a-directory.toml", &scratch.first);
    assert_eq!(directory.status, 422, "{}", directory.body);
    assert_eq!(refused(&directory).kind, Kind::NotADocument);

    let gone = read("fleet/machines/rivet-01.toml", &"0".repeat(40));
    assert_eq!(gone.status, 404, "{}", gone.body);
    assert_eq!(refused(&gone).kind, Kind::NoCommit);
}

/// **A path outside `fleet/machines/` never reaches git**, even for a file the
/// repository holds. Each body is written by hand, because the typed request
/// cannot hold such a path.
#[test]
fn a_path_outside_the_machines_directory_is_refused_before_git_runs() {
    let scratch = Scratch::new("paths");
    for path in [
        "etc/passwd.toml",
        "/etc/passwd",
        "fleet/machines/../../etc/passwd.toml",
        "fleet/machines/sub/deeper.toml",
        "fleet/machines/README.md",
        "../machines.git/config",
    ] {
        let body = format!("repository {REPO}\ncommit {}\npath {path}\n", scratch.first);
        let response = scratch.post(Operation::File.route(), body.as_bytes());
        assert_eq!(response.status, 400, "{path}: {}", response.body);
        assert_eq!(refused(&response).kind, Kind::BadRequest, "{path}");
        assert!(!response.body.contains("root:x"), "{path} was read: {}", response.body);

        let commit = format!(
            "repository {REPO}\nbranch main\nexpected-head {}\npath {path}\nmessage-bytes 2\ndocument-bytes 2\n\nm\nd\n",
            scratch.first
        );
        let response = scratch.post(Operation::Commit.route(), commit.as_bytes());
        assert_eq!(response.status, 400, "{path}: {}", response.body);
    }
    assert_eq!(scratch.branch_tip(), scratch.first, "a refused path moved the branch");
}

// ---- the commit -------------------------------------------------------------

/// **The primary case**: the document lands on the branch, as one commit on
/// the expected tip, with the message as sent and nothing else in the tree
/// changed.
#[test]
fn a_commit_on_the_expected_head_lands_and_changes_only_that_document() {
    let scratch = Scratch::new("commit");
    let message = "Edit rivet-01: hostname\n\nFleet-Override: overwrite-drift rivet-01\n";
    let document = "version = 1\nhostname = \"renamed\"\n";
    let response = scratch.commit(&scratch.first, "fleet/machines/rivet-01.toml", message, document);
    assert_eq!(response.status, 200, "{}", response.body);
    let made = Committed::parse(&response.body).expect("a commit");

    let bare = scratch.bare();
    assert_eq!(scratch.branch_tip(), made.commit.as_str());
    assert_eq!(git_ok(&bare, &["rev-parse", &format!("{}^", made.commit.as_str())]), scratch.first);
    assert_eq!(git_ok(&bare, &["show", &format!("{}:fleet/machines/rivet-01.toml", made.commit.as_str())]), document.trim());
    assert_eq!(git_ok(&bare, &["log", "-1", "--format=%B", made.commit.as_str()]), message.trim());
    assert_eq!(git_ok(&bare, &["log", "-1", "--format=%an <%ae>", made.commit.as_str()]), "rivet-fleetd <rivet-fleetd@localhost>");
    let changed = git_ok(&bare, &["diff", "--name-only", &scratch.first, made.commit.as_str()]);
    assert_eq!(changed, "fleet/machines/rivet-01.toml");
    assert_eq!(git_ok(&bare, &["show", &format!("{}:etc/passwd.toml", made.commit.as_str())]), "root:x:0:0");

    let leftovers: Vec<String> = std::fs::read_dir(&bare)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains("rivet-fleet-as-code"))
        .collect();
    assert!(leftovers.is_empty(), "a private index was left behind: {leftovers:?}");
}

/// **A stale head is refused, distinctly, and nothing is committed** — the
/// case two operators, or an operator and a poller, arrive at together.
#[test]
fn a_commit_composed_against_a_stale_head_is_refused_and_moves_nothing() {
    let scratch = Scratch::new("stale");
    let first = scratch.commit(&scratch.first, "fleet/machines/rivet-01.toml", "one\n", "version = 1\nhostname = \"one\"\n");
    assert_eq!(first.status, 200, "{}", first.body);
    let after_first = scratch.branch_tip();

    let second = scratch.commit(&scratch.first, "fleet/machines/rivet-01.toml", "two\n", "version = 1\nhostname = \"two\"\n");
    assert_eq!(second.status, 409, "{}", second.body);
    let refusal = refused(&second);
    assert_eq!(refusal.kind, Kind::StaleHead);
    assert!(refusal.why.contains(&after_first) && refusal.why.contains(&scratch.first), "{}", refusal.why);
    assert_eq!(scratch.branch_tip(), after_first, "a stale commit moved the branch");
    assert_eq!(
        git_ok(&scratch.bare(), &["show", "refs/heads/main:fleet/machines/rivet-01.toml"]),
        "version = 1\nhostname = \"one\""
    );
}

#[test]
fn a_new_machine_document_is_committed_beside_the_others() {
    let scratch = Scratch::new("new");
    let response = scratch.commit(&scratch.first, "fleet/machines/kitchen pi.toml", "Add kitchen pi\n", "version = 1\n");
    assert_eq!(response.status, 200, "{}", response.body);
    let tip = scratch.branch_tip();
    assert_eq!(git_ok(&scratch.bare(), &["show", &format!("{tip}:fleet/machines/kitchen pi.toml")]), "version = 1");
    assert_eq!(git_ok(&scratch.bare(), &["show", &format!("{tip}:fleet/machines/rivet-01.toml")]), RIVET_01.trim());
}

#[test]
fn a_commit_that_changes_nothing_is_refused() {
    let scratch = Scratch::new("unchanged");
    let response = scratch.commit(&scratch.first, "fleet/machines/rivet-01.toml", "nothing\n", RIVET_01);
    assert_eq!(response.status, 422, "{}", response.body);
    assert_eq!(refused(&response).kind, Kind::NothingToCommit);
    assert_eq!(scratch.branch_tip(), scratch.first);
}

#[test]
fn a_commit_onto_a_path_that_is_not_a_regular_file_is_refused() {
    let scratch = Scratch::new("not-a-file");
    let response = scratch.commit(&scratch.first, "fleet/machines/a-directory.toml", "m\n", "version = 1\n");
    assert_eq!(response.status, 422, "{}", response.body);
    assert_eq!(refused(&response).kind, Kind::NotADocument);
    assert_eq!(scratch.branch_tip(), scratch.first);
}
