//! Tests for `cargo rivet fleet push` — the grammar, the relay's rules, and a
//! REAL `git push` through the real relay into a real smart-HTTP server.
//!
//! **No board, and no stand-in for git.** The server end of the end-to-end
//! tests is this host's own `git http-backend` — the same CGI program
//! `cmd/rivet-mcpd/src/git.rs` fronts on a machine — run by [`Cgi`] exactly as
//! that module runs it: a replaced environment, `REMOTE_USER` set, the CGI
//! `Status:` header translated into a status line. What stands in for the
//! machine is only the pinned TLS hop, which `rivet_remote::tls`'s own
//! `exchange` tests cover and `tools/qemu/adr-035-git-proof.sh` runs for real.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::io::Cursor;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::AtomicU32;

use super::*;

fn scratch(label: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    crate::scratch::dir(&format!(
        "fleet-push-test-{label}-{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|w| (*w).to_owned()).collect()
}

fn git_in(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").args(args).current_dir(dir).output().expect("git runs");
    assert!(out.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

// ---------------------------------------------------------------------------
// The grammar
// ---------------------------------------------------------------------------

fn parsed_push(list: &[&str]) -> Result<(Option<PathBuf>, Option<String>, Endpoint, bool), String> {
    match super::super::parse(&words(list))? {
        super::super::FleetArgs { at, verb: super::super::SubVerb::Push { device, endpoint, confirm } } => {
            Ok((at, device, endpoint, confirm))
        }
        _ => panic!("`{list:?}` did not parse as a push"),
    }
}

#[test]
fn a_device_name_and_confirm_are_read_in_the_run_and_deploy_grammar() {
    let (at, device, endpoint, confirm) = parsed_push(&["push", "rivet-01", "confirm"]).expect("parses");
    assert_eq!(device.as_deref(), Some("rivet-01"));
    assert!(confirm);
    assert!(at.is_none());
    assert!(endpoint.host.is_none());

    let (at, device, endpoint, confirm) = parsed_push(&[
        "push", "--host", "127.0.0.1:7777", "--identity", "./op", "--pin", "ab", "--at", "./fleet",
    ])
    .expect("flags alone parse");
    assert!(device.is_none());
    assert!(!confirm, "no confirm, no push");
    assert_eq!(at, Some(PathBuf::from("./fleet")));
    assert_eq!(endpoint.host.as_deref(), Some("127.0.0.1:7777"));
}

#[test]
fn two_machines_an_unknown_flag_and_half_the_trust_are_each_refused() {
    let two = parsed_push(&["push", "a", "b"]).expect_err("two machines");
    assert!(two.contains("two"), "{two}");
    let unknown = parsed_push(&["push", "rivet-01", "--force"]).expect_err("no such flag");
    assert!(unknown.contains("--force"), "{unknown}");
    let half = parsed_push(&["push", "rivet-01", "--identity", "./op"]).expect_err("identity without pin");
    assert!(half.contains("--identity was given without --pin"), "{half}");
}

// ---------------------------------------------------------------------------
// Resolving the machine
// ---------------------------------------------------------------------------

fn scratch_register(label: &str) -> PathBuf {
    let root = scratch(label);
    let mut register = rivetos_register::Register::empty(&root);
    let mut device = rivetos_register::Device {
        id: "0011223344556677".to_owned(),
        name: "rivet-01".to_owned(),
        listen: Some("0.0.0.0:7777".to_owned()),
        pin: Some("c0ffee".repeat(11)[..64].to_owned()),
        ..rivetos_register::Device::default()
    };
    device.interfaces.insert(
        "eth0".to_owned(),
        rivetos_register::Interface {
            method: "static".to_owned(),
            address: Some("192.168.50.10/24".to_owned()),
            gateway: None,
        },
    );
    register.enroll(device).expect("fresh");
    rivetos_register::store::save(&register).expect("writable");
    root
}

/// **`<host>` resolves through the register**, as `run <device>` does, and an
/// explicit flag still wins over what the register holds.
#[test]
fn a_device_name_resolves_through_the_register_and_explicit_flags_win() {
    let root = scratch_register("resolve");
    let given = Endpoint { identity: Some(PathBuf::from("./op")), ..Endpoint::default() };
    let resolved = resolve_endpoint(Some("rivet-01"), &given, &|| Ok(root.clone())).expect("resolves");
    assert_eq!(resolved.host.as_deref(), Some("192.168.50.10:7777"));
    assert_eq!(resolved.pin.as_deref(), Some(&"c0ffee".repeat(11)[..64]));
    assert_eq!(resolved.identity, Some(PathBuf::from("./op")), "the explicit identity must win");

    let refused = resolve_endpoint(Some("nonesuch"), &given, &|| Ok(root.clone())).expect_err("unknown");
    assert!(refused.contains("rivet-01"), "the refusal lists what the register holds: {refused}");
    let _ = std::fs::remove_dir_all(&root);
}

/// With no device name every one of the three is required, and the register
/// is never opened.
#[test]
fn without_a_device_all_three_flags_are_required_and_no_register_is_read() {
    let partial = Endpoint { host: Some("127.0.0.1:7777".to_owned()), ..Endpoint::default() };
    let refused = resolve_endpoint(None, &partial, &|| panic!("the register must not be read"))
        .expect_err("two flags missing");
    assert!(refused.contains("guesses none of them"), "{refused}");

    let whole = Endpoint {
        host: Some("127.0.0.1:7777".to_owned()),
        identity: Some(PathBuf::from("./op")),
        pin: Some("ab".to_owned()),
    };
    let kept = resolve_endpoint(None, &whole, &|| panic!("the register must not be read")).expect("all three");
    assert_eq!(kept.host, whole.host);
}

// ---------------------------------------------------------------------------
// The relay's rules, against bytes in memory
// ---------------------------------------------------------------------------

/// Records what reached "the machine" and answers a fixed response.
struct Recorder {
    got: Mutex<Vec<(String, Vec<u8>)>>,
    answer: Result<Vec<u8>, String>,
}

impl Recorder {
    fn answering(answer: &str) -> Self {
        Self { got: Mutex::new(Vec::new()), answer: Ok(answer.as_bytes().to_vec()) }
    }
    fn failing(why: &str) -> Self {
        Self { got: Mutex::new(Vec::new()), answer: Err(why.to_owned()) }
    }
    fn calls(&self) -> Vec<(String, Vec<u8>)> {
        self.got.lock().unwrap().clone()
    }
}

impl Upstream for Recorder {
    fn exchange(&self, head: &str, body: &[u8]) -> Result<Vec<u8>, String> {
        self.got.lock().unwrap().push((head.to_owned(), body.to_vec()));
        self.answer.clone()
    }
}

const TOKEN: &str = "00112233445566778899aabbccddeeff";

fn relayed(request: &[u8], upstream: &dyn Upstream) -> (Seen, String) {
    let mut reader = Cursor::new(request.to_vec());
    let mut written = Vec::new();
    let seen = relay_one(&mut reader, &mut written, upstream, TOKEN, "fleet");
    (seen, String::from_utf8_lossy(&written).into_owned())
}

/// **A request without the token never reaches the machine.** The relay is
/// on loopback for the length of one command; the token is what stops another
/// local process borrowing the operator's identity through it.
#[test]
fn a_request_without_the_token_is_refused_and_never_forwarded() {
    let machine = Recorder::answering("HTTP/1.1 200 OK\r\n\r\n");
    for target in [
        "/fleet.git/info/refs?service=git-upload-pack",
        "/ffffffffffffffffffffffffffffffff/fleet.git/info/refs",
        &format!("/{TOKEN}fleet.git/info/refs"),
    ] {
        let request = format!("GET {target} HTTP/1.1\r\nHost: x\r\n\r\n");
        let (seen, written) = relayed(request.as_bytes(), &machine);
        assert_eq!(seen.status, Some(403), "{target}: {written}");
        assert!(written.starts_with("HTTP/1.1 403"), "{written}");
    }
    assert!(machine.calls().is_empty(), "a refused request reached the machine");
}

/// **Only the `fleet` repository is forwarded**, whatever else the machine
/// serves.
#[test]
fn another_repository_is_refused_even_with_the_token() {
    let machine = Recorder::answering("HTTP/1.1 200 OK\r\n\r\n");
    let request = format!("GET /{TOKEN}/notes.git/info/refs HTTP/1.1\r\n\r\n");
    let (seen, written) = relayed(request.as_bytes(), &machine);
    assert_eq!(seen.status, Some(403), "{written}");
    assert!(written.contains("only the `fleet` repository"), "{written}");
    assert!(machine.calls().is_empty());
}

/// **A chunked body leaves as a `Content-Length` body** — the whole reason the
/// relay reads bodies at all, since `cmd/rivet-mcpd` refuses chunked with 411.
/// `Expect: 100-continue` is answered here, before the body is read.
#[test]
fn a_chunked_post_is_forwarded_whole_with_a_content_length() {
    let machine = Recorder::answering("HTTP/1.1 200 OK\r\nContent-Type: application/x-git-receive-pack-result\r\n\r\nok");
    let request = format!(
        "POST /{TOKEN}/fleet.git/git-receive-pack HTTP/1.1\r\n\
         Host: 127.0.0.1\r\n\
         Content-Type: application/x-git-receive-pack-request\r\n\
         Transfer-Encoding: chunked\r\n\
         Expect: 100-continue\r\n\r\n\
         5\r\nhello\r\n7;ext=1\r\n, world\r\n0\r\nTrailer: x\r\n\r\n"
    );
    let (seen, written) = relayed(request.as_bytes(), &machine);
    assert_eq!(seen.status, Some(200), "{written}");
    assert!(written.starts_with("HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK"), "{written}");

    let calls = machine.calls();
    assert_eq!(calls.len(), 1);
    let (head, body) = &calls[0];
    assert_eq!(body, b"hello, world");
    assert!(head.starts_with("POST /fleet.git/git-receive-pack HTTP/1.1\r\n"), "the token must be gone: {head}");
    assert!(head.contains("Content-Length: 12\r\n"), "{head}");
    assert!(head.contains("Content-Type: application/x-git-receive-pack-request\r\n"), "{head}");
    assert!(!head.to_ascii_lowercase().contains("chunked"), "{head}");
    assert!(!head.to_ascii_lowercase().contains("expect"), "{head}");
}

/// A body declared larger than the relay will hold is refused before a byte of
/// it is allocated, and never forwarded.
#[test]
fn a_body_over_the_limit_is_refused_before_it_is_read() {
    let machine = Recorder::answering("HTTP/1.1 200 OK\r\n\r\n");
    let request = format!(
        "POST /{TOKEN}/fleet.git/git-receive-pack HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
        BODY_LIMIT + 1
    );
    let (seen, written) = relayed(request.as_bytes(), &machine);
    assert_eq!(seen.status, Some(400), "{written}");
    assert!(machine.calls().is_empty());
}

/// **A repeated `Content-Length` is refused before a byte of body is read**,
/// identical values included — `.rulings/` R-210 D17. `forward_head` always
/// emits its OWN `Content-Length` computed from the bytes it actually read,
/// so nothing downstream of this parser should ever have to decide which of
/// two same-named headers `git` sent was the real one.
#[test]
fn a_repeated_identical_content_length_is_refused() {
    let machine = Recorder::answering("HTTP/1.1 200 OK\r\n\r\n");
    let request = format!(
        "POST /{TOKEN}/fleet.git/git-receive-pack HTTP/1.1\r\n\
         Content-Length: 5\r\nContent-Length: 5\r\n\r\nhello"
    );
    let (seen, written) = relayed(request.as_bytes(), &machine);
    assert_eq!(seen.status, Some(400), "{written}");
    assert!(machine.calls().is_empty(), "a repeated Content-Length reached the machine");
}

/// **Two `Content-Length` fields that disagree are refused** too — the
/// classic smuggling shape.
#[test]
fn two_disagreeing_content_lengths_are_refused() {
    let machine = Recorder::answering("HTTP/1.1 200 OK\r\n\r\n");
    let request = format!(
        "POST /{TOKEN}/fleet.git/git-receive-pack HTTP/1.1\r\n\
         Content-Length: 5\r\nContent-Length: 7\r\n\r\nhello"
    );
    let (seen, written) = relayed(request.as_bytes(), &machine);
    assert_eq!(seen.status, Some(400), "{written}");
    assert!(machine.calls().is_empty(), "a disagreeing Content-Length reached the machine");
}

/// **A machine that could not be reached is a 502 to git and a transport error
/// to the report** — never an empty repository.
#[test]
fn a_control_plane_that_does_not_answer_is_a_bad_gateway_with_the_reason() {
    let machine = Recorder::failing("received fatal alert: CertificateRequired");
    let request = format!("GET /{TOKEN}/fleet.git/info/refs?service=git-receive-pack HTTP/1.1\r\n\r\n");
    let (seen, written) = relayed(request.as_bytes(), &machine);
    assert!(written.starts_with("HTTP/1.1 502"), "{written}");
    assert!(written.contains("CertificateRequired"), "{written}");
    assert_eq!(seen.transport_error.as_deref(), Some("received fatal alert: CertificateRequired"));
    let head = &machine.calls()[0].0;
    assert!(head.starts_with("GET /fleet.git/info/refs?service=git-receive-pack HTTP/1.1\r\n"), "{head}");
    assert!(!head.contains("Content-Length"), "a GET carries no body: {head}");
}

// ---------------------------------------------------------------------------
// End to end: a real git push through the real relay into git http-backend
// ---------------------------------------------------------------------------

/// **`git http-backend`, run the way `cmd/rivet-mcpd/src/git.rs` runs it on a
/// machine** — `GIT_PROJECT_ROOT`, `GIT_HTTP_EXPORT_ALL`, `REMOTE_USER` (what
/// lets receive-pack take a push), and the CGI `Status:` header turned into a
/// status line. It also refuses a chunked or length-less POST, as the machine
/// does, so a relay that forwarded one would fail here rather than pass.
struct Cgi {
    project_root: PathBuf,
}

impl Upstream for Cgi {
    fn exchange(&self, head: &str, body: &[u8]) -> Result<Vec<u8>, String> {
        let mut lines = head.split("\r\n");
        let mut first = lines.next().unwrap_or_default().split(' ');
        let method = first.next().unwrap_or_default().to_owned();
        let target = first.next().unwrap_or_default().to_owned();
        let header = |name: &str| {
            head.split("\r\n")
                .filter_map(|l| l.split_once(':'))
                .find(|(n, _)| n.trim().eq_ignore_ascii_case(name))
                .map(|(_, v)| v.trim().to_owned())
        };
        if header("transfer-encoding").is_some() {
            return Ok(b"HTTP/1.1 411 Length Required\r\n\r\nchunked".to_vec());
        }
        if method == "POST" && header("content-length").is_none() {
            return Ok(b"HTTP/1.1 411 Length Required\r\n\r\nno length".to_vec());
        }
        let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
        let mut command = Command::new("git");
        command
            .arg("http-backend")
            .env("REQUEST_METHOD", &method)
            .env("PATH_INFO", path)
            .env("QUERY_STRING", query)
            .env("GIT_PROJECT_ROOT", &self.project_root)
            .env("GIT_HTTP_EXPORT_ALL", "1")
            .env("REMOTE_USER", "pinned-test")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(kind) = header("content-type") {
            command.env("CONTENT_TYPE", kind);
        }
        if let Some(encoding) = header("content-encoding") {
            command.env("HTTP_CONTENT_ENCODING", encoding);
        }
        if method == "POST" {
            command.env("CONTENT_LENGTH", body.len().to_string());
        }
        let mut child = command.spawn().map_err(|e| format!("git http-backend: {e}"))?;
        let mut stdin = child.stdin.take().expect("stdin");
        let body = body.to_vec();
        let feeder = std::thread::spawn(move || {
            let _ = stdin.write_all(&body);
        });
        let output = child.wait_with_output().map_err(|e| format!("git http-backend: {e}"))?;
        let _ = feeder.join();
        let raw = output.stdout;
        let (at, gap) = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|p| (p, 4))
            .or_else(|| raw.windows(2).position(|w| w == b"\n\n").map(|p| (p, 2)))
            .ok_or_else(|| format!("no CGI head: {}", String::from_utf8_lossy(&output.stderr)))?;
        let cgi_head = String::from_utf8_lossy(&raw[..at]).into_owned();
        let mut status = "200 OK".to_owned();
        let mut headers = String::new();
        for line in cgi_head.lines() {
            if let Some(value) = line.strip_prefix("Status:") {
                status = value.trim().to_owned();
            } else if !line.trim().is_empty() {
                headers.push_str(line.trim_end());
                headers.push_str("\r\n");
            }
        }
        let mut answer = format!("HTTP/1.1 {status}\r\n{headers}Connection: close\r\n\r\n").into_bytes();
        answer.extend_from_slice(&raw[at + gap..]);
        Ok(answer)
    }
}

/// A workstation fleet repository with one committed document, and a server
/// project root holding an empty bare `fleet.git` on `main` — what
/// `rivet-sysd` converges `[repos.fleet]` into.
struct Rig {
    base: PathBuf,
    local: PathBuf,
    server_root: PathBuf,
}

impl Rig {
    fn new(label: &str) -> Self {
        let base = scratch(label);
        let local = base.join("fleet");
        super::super::init(&local).expect("fleet init");
        let server_root = base.join("state-git");
        std::fs::create_dir_all(&server_root).expect("server root");
        git_in(&server_root, &["init", "--quiet", "--bare", "--initial-branch=main", "fleet.git"]);
        let rig = Self { base, local, server_root };
        rig.commit_document("rivet-01", "version = 1\nhostname = \"rivet-01\"\n");
        rig
    }

    fn commit_document(&self, host: &str, text: &str) {
        let documents = self.local.join("documents");
        std::fs::create_dir_all(&documents).expect("documents/");
        std::fs::write(documents.join(format!("{host}.provisioning.toml")), text).expect("a document");
        git_in(&self.local, &["add", "-A"]);
        git_in(&self.local, &["commit", "--quiet", "-m", &format!("record {host}")]);
    }

    fn server_main(&self) -> Option<String> {
        let out = Command::new("git")
            .args(["rev-parse", "--verify", "--quiet", "refs/heads/main"])
            .current_dir(self.server_root.join("fleet.git"))
            .output()
            .expect("git runs");
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    fn upstream(&self) -> Arc<dyn Upstream> {
        Arc::new(Cgi { project_root: self.server_root.clone() })
    }

    fn local_main(&self) -> String {
        git_in(&self.local, &["rev-parse", "refs/heads/main"])
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

/// **`fleet init` makes `main`**, whatever this host's `init.defaultBranch`
/// says — the branch a server's repository is created on and `push` sends.
#[test]
fn fleet_init_names_main_as_the_branch() {
    let rig = Rig::new("init-main");
    assert_eq!(git_in(&rig.local, &["symbolic-ref", "--short", "HEAD"]), "main");
}

/// **The whole verb, twice over: plan, push, read back, then a fast-forward.**
#[test]
fn a_plan_pushes_nothing_and_confirm_creates_then_fast_forwards_the_servers_main() {
    let rig = Rig::new("push-e2e");

    let plan = push(&rig.local, rig.upstream(), "rivet-01", false, true).expect("a plan");
    assert!(plan.contains("create main, carrying 2 commit(s)"), "{plan}");
    assert!(plan.contains("Nothing was pushed. Add `confirm`"), "{plan}");
    assert_eq!(rig.server_main(), None, "a plan must not push");

    let pushed = push(&rig.local, rig.upstream(), "rivet-01", true, true).expect("the push");
    assert!(pushed.contains("PUSHED"), "{pushed}");
    assert!(pushed.contains("read back over the control plane"), "{pushed}");
    assert_eq!(rig.server_main(), Some(rig.local_main()));

    rig.commit_document("rivet-02", "version = 1\nhostname = \"rivet-02\"\n");
    let plan = push(&rig.local, rig.upstream(), "rivet-01", false, true).expect("a second plan");
    assert!(plan.contains("fast-forward main by 1 commit(s)"), "{plan}");
    let pushed = push(&rig.local, rig.upstream(), "rivet-01", true, true).expect("the fast-forward");
    assert!(pushed.contains("PUSHED"), "{pushed}");
    assert_eq!(rig.server_main(), Some(rig.local_main()));

    let again = push(&rig.local, rig.upstream(), "rivet-01", true, true).expect("nothing to do");
    assert!(again.contains("NOTHING TO PUSH"), "{again}");
}

/// **A push bigger than `http.postBuffer`** — git sends it chunked, and the
/// server end here refuses chunked exactly as `cmd/rivet-mcpd` does. It lands
/// only because the relay de-chunks. Three megabytes of bytes that do not
/// compress, so the pack is genuinely larger than git's 1 MiB default.
#[test]
fn a_push_larger_than_git_s_post_buffer_arrives_whole() {
    let rig = Rig::new("push-chunked");
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let noise: Vec<u8> = (0..3 * 1024 * 1024)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect();
    std::fs::create_dir_all(rig.local.join("register")).expect("register/");
    std::fs::write(rig.local.join("register").join("noise.bin"), &noise).expect("noise");
    git_in(&rig.local, &["add", "-A"]);
    git_in(&rig.local, &["commit", "--quiet", "-m", "a large text-half commit"]);

    // **Chunking forced, not hoped for**: a host whose own git configuration
    // raised `http.postBuffer` would otherwise send this with a length and the
    // test would prove nothing. One `-c` on this one invocation, as `push`
    // itself does everything.
    let relay = Relay::start(rig.upstream(), "fleet").expect("a relay");
    let url = relay.url();
    let out = remote_git(&rig.local, &["-c", "http.postBuffer=65536", "push", &url, "refs/heads/main:refs/heads/main"])
        .expect("git runs");
    assert!(out.status.success(), "{}", quoted_output(&out));
    let seen = relay.seen();
    drop(relay);
    assert!(
        seen.iter().any(|s| s.method == "POST" && s.chunked && s.status == Some(200)),
        "git did not send a chunked POST the relay then delivered: {seen:?}"
    );
    assert_eq!(rig.server_main(), Some(rig.local_main()));

    // And through the verb itself, a second large commit on top.
    std::fs::write(rig.local.join("register").join("noise.bin"), noise.iter().rev().copied().collect::<Vec<u8>>())
        .expect("more noise");
    git_in(&rig.local, &["commit", "--quiet", "-am", "a second large commit"]);
    let pushed = push(&rig.local, rig.upstream(), "rivet-01", true, true).expect("a large push through the verb");
    assert!(pushed.contains("PUSHED"), "{pushed}");
    assert_eq!(rig.server_main(), Some(rig.local_main()));
}

/// **Uncommitted work is refused**, tracked or not, and nothing reaches the
/// server.
#[test]
fn uncommitted_changes_are_refused_before_the_server_is_asked() {
    let rig = Rig::new("push-dirty");
    std::fs::write(rig.local.join("documents").join("half.provisioning.toml"), "version = 1\n").expect("untracked");
    let refused = push(&rig.local, rig.upstream(), "rivet-01", true, true).expect_err("dirty");
    assert!(refused.contains("uncommitted changes"), "{refused}");
    assert!(refused.contains("half.provisioning.toml"), "{refused}");
    assert_eq!(rig.server_main(), None);
}

/// **Not a fast-forward is refused, and never forced.** The server's `main` is
/// moved to a commit this workstation never received.
#[test]
fn a_server_holding_history_this_repository_lacks_is_refused_and_left_alone() {
    let rig = Rig::new("push-diverged");
    push(&rig.local, rig.upstream(), "rivet-01", true, true).expect("the first push");

    let elsewhere = rig.base.join("elsewhere");
    git_in(&rig.base, &["clone", "--quiet", rig.server_root.join("fleet.git").to_str().unwrap(), "elsewhere"]);
    git_in(&elsewhere, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "--quiet", "--allow-empty", "-m", "someone else"]);
    git_in(&elsewhere, &["push", "--quiet", "origin", "main"]);
    let theirs = rig.server_main().expect("a main");

    rig.commit_document("rivet-03", "version = 1\n");
    let refused = push(&rig.local, rig.upstream(), "rivet-01", true, true).expect_err("diverged");
    assert!(refused.contains("NOT A FAST-FORWARD"), "{refused}");
    assert!(refused.contains("never forces"), "{refused}");
    assert_eq!(rig.server_main(), Some(theirs), "a refused push must leave the server's main alone");
}

/// **A server with no `fleet` repository is refused by name**, with the
/// declaration that creates one.
#[test]
fn a_server_with_no_fleet_repository_is_refused_naming_the_declaration() {
    let rig = Rig::new("push-no-repo");
    std::fs::remove_dir_all(rig.server_root.join("fleet.git")).expect("remove the repository");
    let refused = push(&rig.local, rig.upstream(), "rivet-01", false, true).expect_err("no repository");
    assert!(refused.contains("serves no `fleet` repository"), "{refused}");
    assert!(refused.contains("[repos.fleet]"), "{refused}");
}

/// **A machine that will not answer is reported as that**, never as an empty
/// repository or a create.
#[test]
fn a_control_plane_refusal_is_reported_and_nothing_is_planned() {
    let rig = Rig::new("push-refused");
    let machine: Arc<dyn Upstream> = Arc::new(Recorder::failing("received fatal alert: CertificateRequired"));
    let refused = push(&rig.local, machine, "rivet-01", true, true).expect_err("the machine refused");
    assert!(refused.contains("control plane did not answer"), "{refused}");
    assert!(refused.contains("CertificateRequired"), "{refused}");
    assert!(!refused.contains("create main"), "{refused}");
}

/// A repository on another branch is refused, naming the rename.
#[test]
fn a_repository_with_no_main_is_refused_naming_the_rename() {
    let rig = Rig::new("push-no-main");
    git_in(&rig.local, &["branch", "-m", "main", "trunk"]);
    let refused = push(&rig.local, rig.upstream(), "rivet-01", false, true).expect_err("no main");
    assert!(refused.contains("no `main` branch"), "{refused}");
    assert!(refused.contains("branch -m trunk main"), "{refused}");
}
