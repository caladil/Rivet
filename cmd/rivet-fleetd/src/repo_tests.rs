//! Fleet-as-code's repository half against a fake control plane: an in-memory
//! repository that reads and answers with `rivet_core::fleet_as_code`'s own
//! formats and keeps the compare-and-swap `rivet-mcpd` keeps.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::cell::RefCell;
use std::collections::BTreeMap;

use rivet_core::fleet_as_code::{Operation, Refusal};

use super::*;
use crate::code::{Fleet, Pass};

const RIVET_01: &str = "version = 1\nhostname = \"rivet-01\"\n";

#[derive(Clone)]
struct Commit {
    files: BTreeMap<String, String>,
    message: String,
}

/// One repository named `machines`, one branch `main`.
struct State {
    commits: BTreeMap<String, Commit>,
    main: Option<String>,
    next: u32,
}

impl State {
    fn add(&mut self, files: BTreeMap<String, String>, message: &str) -> String {
        self.next += 1;
        let id = format!("{:040x}", self.next);
        self.commits.insert(id.clone(), Commit { files, message: message.to_owned() });
        self.main = Some(id.clone());
        id
    }

    fn tip_files(&self) -> BTreeMap<String, String> {
        self.main.as_ref().map(|id| self.commits[id].files.clone()).unwrap_or_default()
    }
}

type Hook = Box<dyn FnOnce(&mut State)>;

struct Fake {
    state: RefCell<State>,
    /// Every route asked, in order.
    asked: RefCell<Vec<String>>,
    /// Every commit request's `expected-head`.
    expected: RefCell<Vec<String>>,
    /// Runs when a commit arrives, before its compare-and-swap: another commit
    /// landing in between.
    before_commit: RefCell<Option<Hook>>,
    /// Runs after a document is read and answered: another commit landing
    /// between this caller's read and its commit.
    after_file: RefCell<Option<Hook>>,
    /// When set, every request fails in transport.
    unreachable: bool,
}

impl Fake {
    fn with(files: &[(&str, &str)]) -> Self {
        let mut state = State { commits: BTreeMap::new(), main: None, next: 0 };
        let files = files.iter().map(|(p, t)| ((*p).to_owned(), (*t).to_owned())).collect();
        state.add(files, "seed\n\nFleet-Override: overwrite-drift rivet-01\n");
        Self {
            state: RefCell::new(state),
            asked: RefCell::new(Vec::new()),
            expected: RefCell::new(Vec::new()),
            before_commit: RefCell::new(None),
            after_file: RefCell::new(None),
            unreachable: false,
        }
    }

    fn tip(&self) -> String {
        self.state.borrow().main.clone().unwrap()
    }

    fn answer(&self, route: &str, body: &[u8]) -> Result<String, Refusal> {
        let op = Operation::from_route(route).ok_or_else(|| Refusal::new(Kind::NotAllowed, "an unknown route"))?;
        let request = Request::parse(op, body)?;
        let mut state = self.state.borrow_mut();
        let name = match &request {
            Request::Head(r) => r.repository.as_str().to_owned(),
            Request::Machines(r) => r.repository.as_str().to_owned(),
            Request::File(r) => r.repository.as_str().to_owned(),
            Request::Commit(r) => r.repository.as_str().to_owned(),
        };
        if name != "machines" {
            return Err(Refusal::new(Kind::NoRepository, format!("no {name}")));
        }
        match request {
            Request::Head(_) => {
                let id = state.main.clone().ok_or_else(|| Refusal::new(Kind::NoBranch, "no main"))?;
                let message = state.commits[&id].message.clone();
                Ok(Head { commit: CommitId::new(&id).unwrap(), message }.render())
            }
            Request::Machines(r) => {
                let commit = state.commits.get(r.commit.as_str()).ok_or_else(|| Refusal::new(Kind::NoCommit, "gone"))?;
                let labels = commit
                    .files
                    .keys()
                    .filter_map(|p| p.strip_prefix("fleet/machines/")?.strip_suffix(".toml"))
                    .map(str::to_owned)
                    .collect();
                Ok(Machines { labels, ignored: Vec::new() }.render())
            }
            Request::File(r) => {
                let commit = state.commits.get(r.commit.as_str()).ok_or_else(|| Refusal::new(Kind::NoCommit, "gone"))?;
                let answer = commit.files.get(&r.path.as_path()).map_or(wire::File::Absent, |t| wire::File::Present(t.clone())).render();
                if let Some(hook) = self.after_file.borrow_mut().take() {
                    hook(&mut state);
                }
                Ok(answer)
            }
            Request::Commit(r) => {
                self.expected.borrow_mut().push(r.expected_head.as_str().to_owned());
                if let Some(hook) = self.before_commit.borrow_mut().take() {
                    hook(&mut state);
                }
                let tip = state.main.clone().ok_or_else(|| Refusal::new(Kind::NoBranch, "no main"))?;
                if tip != r.expected_head.as_str() {
                    return Err(Refusal::new(
                        Kind::StaleHead,
                        format!("REFUSED, nothing was committed: main is at {tip}, composed against {}", r.expected_head.as_str()),
                    ));
                }
                let mut files = state.tip_files();
                files.insert(r.path.as_path(), r.document.clone());
                let id = state.add(files, &r.message);
                Ok(Committed { commit: CommitId::new(&id).unwrap() }.render())
            }
        }
    }
}

impl ControlPlane for Fake {
    fn post(&self, route: &str, body: &[u8]) -> Result<(u16, Vec<u8>), String> {
        self.asked.borrow_mut().push(route.to_owned());
        if self.unreachable {
            return Err("127.0.0.1:7777: Connection refused (os error 111)".to_owned());
        }
        Ok(match self.answer(route, body) {
            Ok(text) => (200, text.into_bytes()),
            Err(refusal) => (refusal.kind.status().0, refusal.render().into_bytes()),
        })
    }
}

fn repository(fake: &Fake) -> Repository<'_> {
    Repository { name: "machines", control: fake }
}

// ---- reads ------------------------------------------------------------------

#[test]
fn the_head_its_machines_and_a_document_are_read_through_the_control_plane() {
    let fake = Fake::with(&[("fleet/machines/rivet-01.toml", RIVET_01), ("README.md", "x")]);
    let repo = repository(&fake);
    let head = repo.head("main").expect("a head");
    assert_eq!(head.commit.as_str(), fake.tip());
    assert!(head.message.contains("Fleet-Override: overwrite-drift rivet-01"), "{}", head.message);
    assert_eq!(repo.machines(head.commit.as_str()).expect("machines").labels, vec!["rivet-01".to_owned()]);
    assert_eq!(repo.file(head.commit.as_str(), &path_of("rivet-01")).expect("a file"), Some(RIVET_01.to_owned()));
    assert_eq!(repo.file(head.commit.as_str(), &path_of("kitchen-pi")).expect("an answer"), None);
    assert_eq!(
        *fake.asked.borrow(),
        vec!["/fleet-as-code/head", "/fleet-as-code/machines", "/fleet-as-code/file", "/fleet-as-code/file"]
    );
}

/// **A setting that is not a repository name on this server — the URL the
/// setting used to take — is refused before anything is asked**, and says
/// which setting and why.
#[test]
fn a_repository_setting_that_is_a_url_or_the_fleet_repository_is_refused_without_asking() {
    let fake = Fake::with(&[]);
    for name in ["https://example.invalid/fleet.git", "/state/git/machines.git", "fleet"] {
        let why = Repository { name, control: &fake }.head("main").expect_err(name);
        assert!(why.contains("fleet-repository"), "{why}");
    }
    assert!(fake.asked.borrow().is_empty(), "a refused setting reached the control plane");
}

#[test]
fn a_path_outside_the_machines_directory_is_refused_without_asking() {
    let fake = Fake::with(&[]);
    let why = repository(&fake).file(&fake.tip(), "fleet/machines/../../etc/passwd.toml").expect_err("refused");
    assert!(why.contains("not a machine document"), "{why}");
    assert!(fake.asked.borrow().is_empty());
}

#[test]
fn an_unreachable_control_plane_and_an_answer_this_build_cannot_read_are_named() {
    let mut fake = Fake::with(&[]);
    fake.unreachable = true;
    let why = repository(&fake).head("main").expect_err("unreachable");
    assert!(why.contains("did not answer /fleet-as-code/head") && why.contains("Connection refused"), "{why}");

    struct Old;
    impl ControlPlane for Old {
        fn post(&self, _: &str, _: &[u8]) -> Result<(u16, Vec<u8>), String> {
            Ok((404, b"not a route of this build\n".to_vec()))
        }
    }
    let why = Repository { name: "machines", control: &Old }.head("main").expect_err("old");
    assert!(why.contains("predate"), "{why}");
}

// ---- the commit -------------------------------------------------------------

/// The write lands on the head the caller read, and the head it names as
/// expected is that one.
#[test]
fn a_commit_is_asked_for_on_the_head_the_document_was_read_at() {
    let fake = Fake::with(&[("fleet/machines/rivet-01.toml", RIVET_01)]);
    let before = fake.tip();
    let made = repository(&fake)
        .commit_file("main", &path_of("rivet-01"), Some(RIVET_01), "version = 1\nhostname = \"renamed\"\n", "Edit")
        .expect("committed");
    assert_eq!(made, fake.tip());
    assert_eq!(*fake.expected.borrow(), vec![before]);
    assert_eq!(fake.state.borrow().tip_files()["fleet/machines/rivet-01.toml"], "version = 1\nhostname = \"renamed\"\n");
}

/// **A document that changed since the caller read it is refused, and no
/// commit is asked for.**
#[test]
fn a_stale_expected_document_is_refused_before_a_commit_is_asked_for() {
    let fake = Fake::with(&[("fleet/machines/rivet-01.toml", RIVET_01)]);
    let before = fake.tip();
    let why = repository(&fake)
        .commit_file("main", &path_of("rivet-01"), Some("version = 1\nhostname = \"old\"\n"), "x", "Edit")
        .expect_err("refused");
    assert!(why.contains("REFUSED") && why.contains("has changed since it was read"), "{why}");
    assert!(!fake.asked.borrow().iter().any(|r| r == "/fleet-as-code/commit"), "{:?}", fake.asked.borrow());
    assert_eq!(fake.tip(), before);

    let why = repository(&fake).commit_file("main", &path_of("rivet-01"), None, "x", "Edit").expect_err("exists");
    assert!(why.contains("already exists"), "{why}");
    let why = repository(&fake).commit_file("main", &path_of("gone"), Some("x"), "y", "Edit").expect_err("gone");
    assert!(why.contains("no longer exists"), "{why}");
}

/// Another commit, changing `rivet-01`'s document, landing on `main`.
fn theirs(state: &mut State) {
    let mut files = state.tip_files();
    files.insert("fleet/machines/rivet-01.toml".into(), "version = 1\nhostname = \"theirs\"\n".into());
    state.add(files, "somebody else\n");
}

/// **The race the document comparison cannot see**: another commit lands
/// between this caller's read of the document and its commit — after the
/// read, or while the commit is arriving. Either way the control plane refuses
/// it as a stale head, the refusal says so, and the other commit is not lost.
#[test]
fn a_commit_that_loses_a_race_is_refused_as_a_stale_head_and_loses_nothing() {
    for when in ["after the read", "as the commit arrives"] {
        let fake = Fake::with(&[("fleet/machines/rivet-01.toml", RIVET_01)]);
        let read_at = fake.tip();
        let hook: Hook = Box::new(theirs);
        if when == "after the read" {
            *fake.after_file.borrow_mut() = Some(hook);
        } else {
            *fake.before_commit.borrow_mut() = Some(hook);
        }
        let why = repository(&fake)
            .commit_file("main", &path_of("rivet-01"), Some(RIVET_01), "version = 1\nhostname = \"mine\"\n", "Mine")
            .expect_err(when);
        assert!(why.contains("stale-head") && why.contains("REFUSED"), "{when}: {why}");
        assert_eq!(*fake.expected.borrow(), vec![read_at.clone()], "{when}: the commit named a head it did not read at");
        assert_ne!(fake.tip(), read_at, "{when}");
        assert_eq!(
            fake.state.borrow().tip_files()["fleet/machines/rivet-01.toml"],
            "version = 1\nhostname = \"theirs\"\n",
            "{when}: the other commit was overwritten"
        );
    }
}

#[test]
fn a_new_machine_document_is_committed_when_none_was_expected() {
    let fake = Fake::with(&[("fleet/machines/rivet-01.toml", RIVET_01)]);
    repository(&fake).commit_file("main", &path_of("kitchen pi"), None, "version = 1\n", "Add").expect("committed");
    let files = fake.state.borrow().tip_files();
    assert_eq!(files["fleet/machines/kitchen pi.toml"], "version = 1\n");
    assert_eq!(files["fleet/machines/rivet-01.toml"], RIVET_01);
}

// ---- POST /api/commit/<device>, through the fake ----------------------------

fn register_with(records: &[(&str, &str, Option<&str>)]) -> Register {
    let mut register = Register::default();
    for (n, (name, role, listen)) in records.iter().enumerate() {
        let id = format!("00112233445566{n:02x}");
        let mut device = Device {
            id: id.clone(),
            name: (*name).to_owned(),
            role: (!role.is_empty()).then(|| (*role).to_owned()),
            pin: Some("00".repeat(32)),
            listen: listen.map(str::to_owned),
            ..Device::default()
        };
        device.interfaces.insert(
            "eth0".to_owned(),
            rivetos_register::Interface { method: "static".into(), address: Some("127.0.0.1".into()), gateway: None },
        );
        register.devices.insert(id, device);
    }
    register
}

fn fleet() -> Fleet {
    Fleet {
        repository: "machines".to_owned(),
        branch: "main".to_owned(),
        baselines: std::env::temp_dir().join(format!("rivet-fleetd-repo-tests-{}", std::process::id())),
        dry_run: true,
    }
}

/// `/api/commit`'s contract is unchanged: the same body in, `{"commit":…}`
/// out, and the same refusal for a document that changed since it was read.
#[test]
fn the_commit_route_writes_through_the_control_plane_and_keeps_its_contract() {
    let fake = Fake::with(&[("fleet/machines/rivet-01.toml", RIVET_01)]);
    let register = register_with(&[("rivet-01", "fleet-server", Some("0.0.0.0:7777"))]);
    let body = "{\"expected\":\"version = 1\\nhostname = \\\"rivet-01\\\"\\n\",\
                \"document\":\"version = 1\\nhostname = \\\"renamed\\\"\\n\",\"message\":\"Edit rivet-01\"}";
    let answer = crate::commit::handle(Some(&fleet()), &register, Ok(&fake), "rivet-01", body).expect("committed");
    assert_eq!(answer, format!("{{\"commit\":\"{}\"}}", fake.tip()));

    let again = crate::commit::handle(Some(&fleet()), &register, Ok(&fake), "rivet-01", body).expect_err("stale");
    assert!(again.contains("REFUSED"), "{again}");

    let secret = "{\"expected\":null,\"document\":\"version = 1\\n[applications.x]\\napi-token = \\\"t\\\"\\n\"}";
    let asked = fake.asked.borrow().len();
    assert!(crate::commit::handle(Some(&fleet()), &register, Ok(&fake), "rivet-01", secret).is_err());
    assert_eq!(fake.asked.borrow().len(), asked, "a credential-shaped document reached the control plane");
}

// ---- the pass, through the fake ---------------------------------------------

/// **The pass's own judgements are unchanged** — a document carrying a
/// credential and a machine the register does not know are the repository's
/// fault; a machine that does not answer is not a failure.
#[test]
fn a_pass_reads_the_repository_through_the_control_plane_and_judges_as_before() {
    let fake = Fake::with(&[
        ("fleet/machines/kitchen-pi.toml", "version = 1\n[network.eth0]\npsk = \"a-credential\"\n"),
        ("fleet/machines/nobody.toml", "version = 1\n"),
        ("fleet/machines/rivet-01.toml", RIVET_01),
    ]);
    let register = register_with(&[("rivet-01", "fleet-server", Some("127.0.0.1:9"))]);
    let pass: Pass = crate::code::pass(&fleet(), &register, &fake).expect("a pass");
    let rejected: Vec<&str> = pass.rejected.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(rejected, vec!["kitchen-pi", "nobody"], "{pass:?}");
    assert!(pass.rejected[0].1.contains("install-secret"), "{}", pass.rejected[0].1);
    assert_eq!(pass.unreachable.len(), 1, "{pass:?}");
    assert_eq!(pass.unreachable[0].0, "rivet-01");
    assert!(pass.applied.is_empty() && pass.refused.is_empty());
}

// ---- this server's own control plane ----------------------------------------

#[test]
fn this_servers_control_plane_is_its_one_server_record_on_loopback() {
    let one = register_with(&[("rivet-01", "fleet-server", Some("0.0.0.0:7777")), ("kitchen", "", None)]);
    assert_eq!(this_server(&one).expect("one"), Tls { address: "127.0.0.1:7777".into(), pin: "00".repeat(32) });

    for (listen, address) in [
        (Some("127.0.0.1:7000"), "127.0.0.1:7000"),
        (Some("[::]:7777"), "[::1]:7777"),
        (Some("192.168.50.66:7777"), "192.168.50.66:7777"),
        (None, "127.0.0.1:7777"),
    ] {
        let register = register_with(&[("rivet-01", "fleet-server", listen)]);
        assert_eq!(this_server(&register).expect("one").address, address, "{listen:?}");
    }

    let none = register_with(&[("rivet-01", "", Some("0.0.0.0:7777"))]);
    assert!(this_server(&none).expect_err("none").contains("role = \"fleet-server\""));
    let two = register_with(&[("a", "fleet-server", None), ("b", "fleet-server", None)]);
    assert!(this_server(&two).expect_err("two").contains("will not choose"));
    let mut unpinned = register_with(&[("rivet-01", "fleet-server", None)]);
    unpinned.devices.values_mut().for_each(|d| d.pin = None);
    assert!(this_server(&unpinned).expect_err("no pin").contains("no pin"));
}

// ---- the regression guard ---------------------------------------------------

/// **This program runs no program.** `ADR-042`'s policy grants a confined
/// application no execute right outside its own installed files, so a
/// `Command` anywhere in this crate is a feature that works in a test and is
/// refused on a machine. `cmd/rivet-sysd`'s Landlock probe asserts the refusal;
/// this asserts nothing here needs it.
#[test]
fn no_source_file_in_this_crate_starts_a_process() {
    let needles = [concat!("Command", "::new"), concat!("process", "::Command"), concat!("process", "::Stdio"), concat!("Command", "::")];
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut stack = vec![dir];
    let mut read = 0;
    while let Some(at) = stack.pop() {
        for entry in std::fs::read_dir(&at).expect("src").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).expect("source");
                read += 1;
                for needle in needles {
                    assert!(!text.contains(needle), "{} contains `{needle}`", path.display());
                }
            }
        }
    }
    assert!(read >= 10, "only {read} source files were read, so the scan proves nothing");
}
