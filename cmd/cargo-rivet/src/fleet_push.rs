//! `cargo rivet fleet push <host>` — **the fleet repository's text, pushed to a
//! fleet server's `fleet` repository over `ADR-035`'s control plane.**
//!
//! `.rulings/` R-182 put a fleet server's card on a diet: it carries the fleet
//! repository's text and none of its objects. R-184 found the machine side
//! already built — `cmd/rivet-mcpd/src/git.rs` fronts `git-http-backend` on
//! 7777 — and ruled the three pieces left: a server's document declares
//! `[repos.fleet]` (`cmd/rivet-provision`), the text is pushed to it (this
//! module), and the objects follow (`crate::fleet::sync`). This is the second
//! piece.
//!
//! # What it pushes, and what it refuses
//!
//! **`main` of the workstation's fleet repository, to `main` of the server's
//! `fleet` repository, and nothing else.** No other branch, no tags, and never
//! a force: R-184 D2 makes the operator the only writer for now, and a push
//! that could rewrite the server's history is a decision nobody has taken.
//!
//! * **Uncommitted changes are refused.** A push sends commits; a document
//!   edited and not committed would silently stay behind, and the operator
//!   would believe the server holds what the working tree shows.
//! * **A server whose `main` is not an ancestor of the local `main` is
//!   refused.** Not a fast-forward means the server holds history this
//!   workstation lacks — nothing here decides which side is right.
//! * **A server with no `fleet` repository is refused by name**, and the
//!   refusal says the usual cause: a document that declares no `[repos.fleet]`.
//!
//! **Without `confirm` nothing is pushed.** The plan — both `main`s and what a
//! push would do — is read from the server first, the `backup-secrets` shape:
//! a verb that changes a remote shows what it will change and waits for the
//! last word.
//!
//! # Why a loopback relay, and not TLS settings handed to `git`
//!
//! **A stock `git` cannot be given this connection's trust without writing
//! the operator's private key somewhere new.** `http.sslKey` takes a PEM file,
//! and the operator identity is `device.seed`: thirty-two raw bytes this
//! project deliberately keeps in exactly one file. Converting it to a PEM for
//! the length of a push is a second copy of a private key on disk, which the
//! whole `ADR-052` §4 refusal exists to prevent. The machine's side has two
//! more problems a one-shot `-c` cannot settle: its certificate is a
//! self-signed leaf named by whatever `--name` it was minted with, so a client
//! dialling an address fails name verification whatever CA file it is given;
//! and `http.pinnedPubkey` is checked in addition to chain verification, never
//! instead of it, so pinning alone cannot pass. `http.sslVerify=false` would
//! pass all of it and is not an option this tool will ever take.
//!
//! **So the pinned handshake is made by the same code every other
//! control-plane call makes**, `rivet_remote::tls::Tls`, and `git` is handed a
//! plain-HTTP URL on `127.0.0.1`. The relay listens only on loopback, only for
//! the length of one command, and only answers a path that begins with a
//! 128-bit token from the operating system's random source — so another local
//! process cannot borrow the operator's identity by guessing the port.
//!
//! # Chunked bodies, and `http.postBuffer`
//!
//! `git` sends a request body larger than `http.postBuffer` (1 MiB by default)
//! as `Transfer-Encoding: chunked`, and `cmd/rivet-mcpd` refuses a chunked
//! body with `411` rather than hand git its framing. **The relay de-chunks**:
//! it reads the whole body, up to `BODY_LIMIT`, and forwards it with a
//! `Content-Length`, so no `http.postBuffer` setting is needed and none is
//! written. `Expect: 100-continue` is answered locally for the same reason.
//! The body is held in memory — the machine holds a git answer whole too
//! (`crate::git::serve`'s own read), so this end is not the tighter bound.
//!
//! # Never global configuration
//!
//! Every setting reaches `git` as a `-c` on one invocation or as an
//! environment variable on that one child process: credential helpers are
//! emptied so a `403` cannot open a login window, terminal prompts are off,
//! and loopback is excluded from any proxy the environment names.

use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use rivet_remote::endpoint::Endpoint;

/// The repository on the server — R-184 D3's fixed name.
pub(super) const REPOSITORY: &str = rivetos_fleet_repo::SERVED_NAME;

/// The one branch pushed.
pub(super) const BRANCH: &str = "main";

/// The largest request head the relay reads before refusing.
const HEAD_LIMIT: usize = 64 * 1024;

/// **The largest request body the relay will hold, 512 MiB.** A fleet
/// repository's git history is its text half alone — objects are never in git
/// (`ADR-052` §3) — so a push anywhere near this is not a fleet repository.
pub(super) const BODY_LIMIT: usize = 512 * 1024 * 1024;

/// How long one relayed connection may sit silent before it is dropped, so a
/// `git` that died mid-request cannot hold the relay open for ever.
const IDLE: Duration = Duration::from_secs(300);

/// **Send one HTTP request to the machine and hand back its whole answer.**
///
/// `rivet_remote::tls::Tls::exchange` on a real run; a local
/// `git http-backend` in the tests, so a real `git push` is exercised end to
/// end with no board.
pub(super) trait Upstream: Send + Sync {
    /// # Errors
    ///
    /// The machine could not be reached, would not accept this identity, or
    /// answered nothing readable.
    fn exchange(&self, head: &str, body: &[u8]) -> Result<Vec<u8>, String>;
}

impl Upstream for rivet_remote::tls::Tls {
    fn exchange(&self, head: &str, body: &[u8]) -> Result<Vec<u8>, String> {
        rivet_remote::tls::Tls::exchange(self, head, body)
    }
}

// ---------------------------------------------------------------------------
// resolving the machine
// ---------------------------------------------------------------------------

/// **`<host>` through the register, the way `run <device>` and `deploy
/// <device>` resolve one** — `crate::device::expand`, with every explicit
/// `--host`/`--identity`/`--pin` winning over what the register holds.
///
/// With no device name, all three flags are required: this verb changes a
/// machine, and there is no address, identity or pin it will guess.
///
/// `register_root` is consulted only when a device is named — a test passes a
/// scratch one, and the real run passes `crate::device::default_register_dir`.
///
/// # Errors
///
/// `crate::device::expand`'s own refusals, or a flag-only invocation missing
/// one of the three.
pub(super) fn resolve_endpoint(
    device: Option<&str>,
    given: &Endpoint,
    register_root: &dyn Fn() -> Result<std::path::PathBuf, String>,
) -> Result<Endpoint, String> {
    let Some(name) = device else {
        if given.host.is_none() || given.identity.is_none() || given.pin.is_none() {
            return Err(
                "`fleet push` needs a machine: name a device the register holds (`cargo rivet \
                 fleet push <host>`), or give all three of --host HOST:PORT, --identity DIR and \
                 --pin HEX. It changes a machine, so it guesses none of them."
                    .to_owned(),
            );
        }
        return Ok(given.clone());
    };
    let mut words: Vec<String> = Vec::new();
    if let Some(host) = &given.host {
        words.extend(["--host".to_owned(), host.clone()]);
    }
    if let Some(identity) = &given.identity {
        words.extend(["--identity".to_owned(), identity.display().to_string()]);
    }
    if let Some(pin) = &given.pin {
        words.extend(["--pin".to_owned(), pin.clone()]);
    }
    let expanded = crate::device::expand(name, &words, &register_root()?)?;
    let mut endpoint = Endpoint::default();
    let mut index = 0;
    while index < expanded.len() {
        index = endpoint.read(&expanded, index)?.unwrap_or(index + 1);
    }
    Ok(endpoint)
}

/// `cargo rivet fleet push [<host>] [--host …] [--identity …] [--pin …]
/// [--at DIR] [confirm]` — the composition root: resolve the machine, open
/// the pinned transport, and [`push`].
///
/// # Errors
///
/// [`resolve_endpoint`]'s, `Endpoint::open`'s, or [`push`]'s.
pub(super) fn run(root: &Path, device: Option<&str>, given: &Endpoint, confirm: bool) -> Result<String, String> {
    given.paired()?;
    let endpoint = resolve_endpoint(device, given, &crate::device::default_register_dir)?;
    let transport = endpoint.open()?.ok_or_else(|| {
        "no operator identity to present: `fleet push` speaks ADR-035's control plane and has no \
         second transport. `cargo rivet identity new` mints one; --identity DIR names it."
            .to_owned()
    })?;
    let label = device.map_or_else(|| endpoint.host.clone().unwrap_or_default(), str::to_owned);
    push(root, Arc::new(transport), &label, confirm, true)
}

// ---------------------------------------------------------------------------
// the push
// ---------------------------------------------------------------------------

/// What pushing `main` would do to the server's `main`.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Plan {
    /// The server's repository has no `main`: it is created, carrying
    /// `commits` commits.
    Create { commits: u64 },
    /// The server's `main` is an ancestor of the local one.
    FastForward { commits: u64 },
    /// Both name the same commit.
    UpToDate,
    /// The server holds a commit this repository does not, or one that is not
    /// an ancestor of the local `main`. **Refused**: never forced.
    NotFastForward { known_here: bool },
}

/// **Push `main` to `label`'s `fleet` repository, through `upstream`** — or,
/// without `confirm`, read both `main`s and say what a push would do.
///
/// **`strict` is the standalone `cargo rivet fleet push`'s posture**:
/// uncommitted or untracked files refuse the whole run (`refuse_uncommitted`).
/// `fleet publish --sync` passes `false` instead — `ADR-061` §4: a publish
/// sends commits, and files never travel either way, so they do not block it;
/// what would have refused is named as left behind instead
/// ([`uncommitted_note`]).
///
/// # Errors
///
/// `root` is not a fleet repository, has no `main`, or (`strict` only) has
/// uncommitted changes; the machine cannot be asked, serves no `fleet`
/// repository, or refuses; the push is not a fast-forward; or `git` itself
/// fails, quoted.
pub(super) fn push(
    root: &Path,
    upstream: Arc<dyn Upstream>,
    label: &str,
    confirm: bool,
    strict: bool,
) -> Result<String, String> {
    super::require_repository(root)?;
    let left_behind = if strict {
        refuse_uncommitted(root)?;
        String::new()
    } else {
        uncommitted_note(root)?
    };
    let local = local_main(root)?;

    let relay = Relay::start(upstream, REPOSITORY)?;
    let url = relay.url();
    let outcome = push_through(root, &relay, &url, &local, label, confirm);
    drop(relay);
    match outcome {
        Ok(report) => Ok(format!("{left_behind}{report}")),
        Err(report) => Err(format!("{left_behind}{report}")),
    }
}

/// **`strict = false`'s posture**: uncommitted or untracked files never
/// travel — a push sends commits — so they do not block a run; they are
/// named instead, so nobody believes they went (`ADR-061` §4).
fn uncommitted_note(root: &Path) -> Result<String, String> {
    let status = git_text(root, &["status", "--porcelain", "--untracked-files=all"])?;
    if status.trim().is_empty() {
        return Ok(String::new());
    }
    Ok(format!(
        "left behind (never sent): {} has uncommitted or untracked changes, and a publish's \
         --sync sends commits, never a working tree:\n{status}\n",
        root.display()
    ))
}

fn push_through(
    root: &Path,
    relay: &Relay,
    url: &str,
    local: &str,
    label: &str,
    confirm: bool,
) -> Result<String, String> {
    let server = remote_main(root, url, relay, label)?;
    let plan = decide(root, local, server.as_deref())?;
    let subject = git_text(root, &["log", "-1", "--format=%s", local]).unwrap_or_default();

    let mut report = format!(
        "fleet push  {label}\n\
         \x20 from          {root} ({BRANCH})\n\
         \x20 to            {label}'s `{REPOSITORY}` repository, over the control plane\n\
         \x20 local main    {local}  {subject}\n\
         \x20 server main   {}\n",
        server.as_deref().unwrap_or("(none: the server's repository has no main)"),
        root = root.display(),
        subject = subject.trim(),
    );

    match plan {
        Plan::UpToDate => {
            report.push_str("\nNOTHING TO PUSH: the server's main is already this commit.\n");
            return Ok(report);
        }
        Plan::NotFastForward { known_here } => {
            let why = if known_here {
                "the server's main is a commit this repository has, and it is not an ancestor of \
                 the local main"
            } else {
                "the server's main is a commit this repository does not have"
            };
            return Err(format!(
                "{report}\nREFUSED, NOT A FAST-FORWARD: {why}. The server holds history this \
                 workstation lacks, and this tool never forces a push — nothing here decides \
                 which side is right. Nothing was pushed.\n"
            ));
        }
        Plan::Create { commits } => {
            report.push_str(&format!("  would         create main, carrying {commits} commit(s)\n"));
        }
        Plan::FastForward { commits } => {
            report.push_str(&format!("  would         fast-forward main by {commits} commit(s)\n"));
        }
    }

    if !confirm {
        report.push_str("\nNothing was pushed. Add `confirm` to push.\n");
        return Ok(report);
    }

    let pushed = remote_git(root, &["push", "--porcelain", url, &format!("refs/heads/{BRANCH}:refs/heads/{BRANCH}")])
        .map_err(|e| format!("{report}\ncannot run `git push`: {e}"))?;
    if !pushed.status.success() {
        return Err(format!(
            "{report}\nTHE PUSH FAILED.\n{}{}",
            diagnose(relay, label),
            quoted_output(&pushed)
        ));
    }

    // **Read back, over the same control plane**, rather than trusting git's
    // own summary line: the question is what the machine now holds.
    let after = remote_main(root, url, relay, label)?;
    if after.as_deref() != Some(local) {
        return Err(format!(
            "{report}\n`git push` reported success and the server's main now reads {}, not {local}. \
             Something between this workstation and that repository did not keep what was sent.\n",
            after.as_deref().unwrap_or("(none)")
        ));
    }
    report.push_str(&format!(
        "\nPUSHED: {label}'s `{REPOSITORY}` main is {local}, read back over the control plane.\n\
         \x20 note  this is the repository's TEXT. Its application objects are not in git \
         (ADR-052 §3) and did not travel with it: `cargo rivet fleet sync {label} confirm` \
         sends them.\n"
    ));
    Ok(report)
}

/// **Refuse a working tree with anything uncommitted in it** — tracked edits
/// and untracked files alike. `objects/` is ignored by the repository's own
/// `.gitignore`, so the object store never counts.
fn refuse_uncommitted(root: &Path) -> Result<(), String> {
    let status = git_text(root, &["status", "--porcelain", "--untracked-files=all"])?;
    if status.trim().is_empty() {
        return Ok(());
    }
    Err(format!(
        "{} has uncommitted changes, and a push sends commits — these would stay behind while \
         the server was told it holds this repository:\n{status}\
         Commit them (or remove them) first. Nothing was pushed.",
        root.display()
    ))
}

/// The local `main`'s commit, or a refusal naming the branch that is there.
fn local_main(root: &Path) -> Result<String, String> {
    let verify = Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", &format!("refs/heads/{BRANCH}^{{commit}}")])
        .current_dir(root)
        .output()
        .map_err(|e| format!("cannot run `git rev-parse`: {e}"))?;
    if verify.status.success() {
        return Ok(String::from_utf8_lossy(&verify.stdout).trim().to_owned());
    }
    let current = git_text(root, &["symbolic-ref", "--short", "-q", "HEAD"]).unwrap_or_default();
    Err(format!(
        "{} has no `{BRANCH}` branch to push (the checked-out branch is {:?}). A fleet server's \
         repository is created with `{BRANCH}` as its branch; `git -C {} branch -m {} {BRANCH}` \
         renames yours. Nothing was pushed.",
        root.display(),
        current.trim(),
        root.display(),
        if current.trim().is_empty() { "<branch>" } else { current.trim() },
    ))
}

/// **The server's `main`, read over the control plane** — `None` for a
/// repository that has none yet, which a freshly converged one does not.
pub(super) fn remote_main(root: &Path, url: &str, relay: &Relay, label: &str) -> Result<Option<String>, String> {
    let listed = remote_git(root, &["ls-remote", "--refs", url, &format!("refs/heads/{BRANCH}")])
        .map_err(|e| format!("cannot run `git ls-remote`: {e}"))?;
    if !listed.status.success() {
        return Err(format!(
            "could not read {label}'s `{REPOSITORY}` repository, so nothing was pushed.\n{}{}",
            diagnose(relay, label),
            quoted_output(&listed)
        ));
    }
    let text = String::from_utf8_lossy(&listed.stdout);
    Ok(text
        .lines()
        .find_map(|line| line.split_once('\t').filter(|(_, r)| *r == format!("refs/heads/{BRANCH}")))
        .map(|(sha, _)| sha.trim().to_owned()))
}

/// **What a push of `local` over `server` would be.**
fn decide(root: &Path, local: &str, server: Option<&str>) -> Result<Plan, String> {
    let Some(server) = server else {
        let commits = git_text(root, &["rev-list", "--count", local])?.trim().parse::<u64>().unwrap_or(0);
        return Ok(Plan::Create { commits });
    };
    if server == local {
        return Ok(Plan::UpToDate);
    }
    let known_here = Command::new("git")
        .args(["cat-file", "-e", &format!("{server}^{{commit}}")])
        .current_dir(root)
        .output()
        .map(|o| o.status.success())
        .map_err(|e| format!("cannot run `git cat-file`: {e}"))?;
    if !known_here {
        return Ok(Plan::NotFastForward { known_here: false });
    }
    let ancestor = Command::new("git")
        .args(["merge-base", "--is-ancestor", server, local])
        .current_dir(root)
        .output()
        .map_err(|e| format!("cannot run `git merge-base`: {e}"))?;
    if !ancestor.status.success() {
        return Ok(Plan::NotFastForward { known_here: true });
    }
    let commits = git_text(root, &["rev-list", "--count", &format!("{server}..{local}")])?
        .trim()
        .parse::<u64>()
        .unwrap_or(0);
    Ok(Plan::FastForward { commits })
}

/// **`git` against the relay, with nothing this host's configuration could
/// add on top** — every setting on the command line or in this one child's
/// environment, never written anywhere.
pub(super) fn remote_git(root: &Path, args: &[&str]) -> std::io::Result<Output> {
    Command::new("git")
        // An empty value RESETS the helper list: a 403 must not open a
        // credential manager's login window over a relay that needs none.
        .args(["-c", "credential.helper=", "-c", "http.followRedirects=false"])
        .args(args)
        .current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS")
        // Loopback never goes through a proxy the environment names.
        .env("NO_PROXY", "127.0.0.1")
        .env("no_proxy", "127.0.0.1")
        .output()
}

fn git_text(root: &Path, args: &[&str]) -> Result<String, String> {
    super::git(root, args)
}

pub(super) fn quoted_output(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut out = String::new();
    for (name, text) in [("git said", stderr.trim()), ("git printed", stdout.trim())] {
        if !text.is_empty() {
            out.push_str(&format!("  {name}:\n"));
            for line in text.lines() {
                out.push_str(&format!("    {line}\n"));
            }
        }
    }
    out
}

/// **Why a `git` run against the relay failed, in the machine's terms** —
/// read from what the relay saw rather than from git's own summary, which
/// says `502` or `404` and nothing about Rivet.
fn diagnose(relay: &Relay, label: &str) -> String {
    let seen = relay.seen();
    if let Some(failed) = seen.iter().find_map(|s| s.transport_error.as_ref()) {
        return format!(
            "  why   the control plane did not answer: {failed}\n\
             \x20       A machine that will not take this operator identity, or a pin that is not \
             this machine's, fails here. `cargo rivet run {label} -- get status` asks the same \
             question with the same credentials.\n"
        );
    }
    if seen.iter().any(|s| s.status == Some(404) && s.path.contains("/info/refs")) {
        return format!(
            "  why   {label} serves no `{REPOSITORY}` repository (404). Its document must \
             declare [repos.{REPOSITORY}] — every fleet server provisioned since R-184 does — and \
             rivet-sysd creates the repository when it converges that declaration.\n"
        );
    }
    if let Some(refused) = seen.iter().find(|s| s.status.is_some_and(|c| c >= 400)) {
        return format!(
            "  why   {label} answered {} for {} {}: {}\n",
            refused.status.unwrap_or_default(),
            refused.method,
            refused.path,
            refused.detail.trim()
        );
    }
    String::new()
}

// ---------------------------------------------------------------------------
// the relay
// ---------------------------------------------------------------------------

/// One request the relay handled.
#[derive(Debug, Clone, Default)]
pub(super) struct Seen {
    pub method: String,
    /// The path as forwarded — the token already removed.
    pub path: String,
    /// The machine's status, or the relay's own for a request it refused.
    pub status: Option<u16>,
    /// The first bytes of an error answer's body, for [`diagnose`].
    pub detail: String,
    /// The connection to the machine failed; nothing was answered.
    pub transport_error: Option<String>,
    /// `git` sent the body chunked, and the relay forwarded it with a length.
    pub chunked: bool,
}

/// **A plain-HTTP listener on `127.0.0.1`, for one command, that forwards
/// each request to the machine over the pinned control plane.** See the
/// module header for why.
pub(super) struct Relay {
    port: u16,
    token: String,
    repository: String,
    stop: Arc<AtomicBool>,
    seen: Arc<Mutex<Vec<Seen>>>,
    thread: Option<JoinHandle<()>>,
}

impl Relay {
    /// Bind, mint the token, and start answering.
    ///
    /// # Errors
    ///
    /// No loopback port, or no randomness from the operating system.
    pub(super) fn start(upstream: Arc<dyn Upstream>, repository: &str) -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|e| format!("cannot open a loopback port for the push relay: {e}"))?;
        let port = listener
            .local_addr()
            .map_err(|e| format!("cannot read the relay's own port: {e}"))?
            .port();
        let mut raw = [0_u8; 16];
        getrandom::fill(&mut raw).map_err(|e| format!("the operating system gave no randomness for the relay's token: {e}"))?;
        let token: String = raw.iter().map(|b| format!("{b:02x}")).collect();

        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let thread = {
            let stop = Arc::clone(&stop);
            let seen = Arc::clone(&seen);
            let token = token.clone();
            let repository = repository.to_owned();
            std::thread::spawn(move || {
                for connection in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = connection else { continue };
                    let record = serve(stream, upstream.as_ref(), &token, &repository);
                    if let Ok(mut list) = seen.lock() {
                        list.push(record);
                    }
                }
            })
        };
        Ok(Self { port, token, repository: repository.to_owned(), stop, seen, thread: Some(thread) })
    }

    /// The URL `git` is handed.
    pub(super) fn url(&self) -> String {
        format!("http://127.0.0.1:{}/{}/{}.git", self.port, self.token, self.repository)
    }

    /// Everything handled so far.
    pub(super) fn seen(&self) -> Vec<Seen> {
        self.seen.lock().map(|list| list.clone()).unwrap_or_default()
    }
}

impl Drop for Relay {
    /// **The listener goes when the command does.** The accept loop is woken
    /// by one last connection to itself, which it reads the stop flag on and
    /// never serves.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// One connection: relay one request and close.
fn serve(stream: TcpStream, upstream: &dyn Upstream, token: &str, repository: &str) -> Seen {
    let _ = stream.set_read_timeout(Some(IDLE));
    let mut writer = match stream.try_clone() {
        Ok(writer) => writer,
        Err(e) => return Seen { transport_error: Some(format!("the relay could not use its own socket: {e}")), ..Seen::default() },
    };
    let mut reader = BufReader::new(stream);
    let seen = relay_one(&mut reader, &mut writer, upstream, token, repository);
    let _ = writer.flush();
    let _ = writer.shutdown(Shutdown::Both);
    seen
}

/// **One request, from `git` to the machine and back.** Generic over the two
/// halves of the socket so the rules below run against bytes in memory.
pub(super) fn relay_one(
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
    upstream: &dyn Upstream,
    token: &str,
    repository: &str,
) -> Seen {
    let head = match read_head(reader) {
        Ok(head) => head,
        Err(why) => return refuse(writer, 400, "Bad Request", Seen::default(), &why),
    };
    let mut seen = Seen { method: head.method.clone(), ..Seen::default() };
    let path = match forwarded_path(&head.target, token, repository) {
        Ok(path) => path,
        Err(why) => return refuse(writer, 403, "Forbidden", seen, &why),
    };
    seen.path.clone_from(&path);

    if head.header("expect").is_some_and(|v| v.eq_ignore_ascii_case("100-continue")) {
        let _ = writer.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
        let _ = writer.flush();
    }
    seen.chunked = head.header("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    let body = match read_body(reader, &head) {
        Ok(body) => body,
        Err(why) => return refuse(writer, 400, "Bad Request", seen, &why),
    };

    let forwarded = forward_head(&head, &path, body.len());
    match upstream.exchange(&forwarded, &body) {
        Ok(answer) => {
            seen.status = status_of(&answer);
            if seen.status.is_some_and(|c| c >= 400) {
                let at = answer.windows(4).position(|w| w == b"\r\n\r\n").map_or(0, |p| p + 4);
                seen.detail = String::from_utf8_lossy(&answer[at.min(answer.len())..answer.len().min(at + 400)]).into_owned();
            }
            let _ = writer.write_all(&answer);
            seen
        }
        Err(why) => {
            seen.transport_error = Some(why.clone());
            refuse(writer, 502, "Bad Gateway", seen, &format!("the control plane did not answer: {why}\n"))
        }
    }
}

/// A request head, as the relay reads it.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Head {
    pub method: String,
    pub target: String,
    /// `(lower-case name, value)`, in arrival order.
    pub headers: Vec<(String, String)>,
}

impl Head {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

/// Read a request line and headers, up to the blank line.
///
/// # Errors
///
/// A head over [`HEAD_LIMIT`], a connection closed inside one, or a request
/// line that is not `METHOD target HTTP/1.x`.
pub(super) fn read_head(reader: &mut dyn BufRead) -> Result<Head, String> {
    let mut lines = Vec::new();
    let mut total = 0usize;
    loop {
        let mut line = String::new();
        let got = reader.read_line(&mut line).map_err(|e| format!("the request could not be read: {e}"))?;
        if got == 0 {
            return Err("the connection closed inside the request head".to_owned());
        }
        total += got;
        if total > HEAD_LIMIT {
            return Err(format!("the request head is over {HEAD_LIMIT} bytes"));
        }
        let line = line.trim_end_matches(['\r', '\n']).to_owned();
        if line.is_empty() {
            break;
        }
        lines.push(line);
    }
    let mut first = lines.first().map(String::as_str).unwrap_or_default().split(' ');
    let (Some(method), Some(target), Some(version)) = (first.next(), first.next(), first.next()) else {
        return Err(format!("{:?} is not a request line", lines.first()));
    };
    if !version.starts_with("HTTP/1.") {
        return Err(format!("the request speaks {version:?}, and this relay speaks HTTP/1.1"));
    }
    let headers: Vec<(String, String)> = lines
        .iter()
        .skip(1)
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    // **A repeated `Content-Length` is refused whether or not the two values
    // agree** (`.rulings/` R-210 D17). `forward_head` below always emits its
    // OWN `Content-Length`, computed from the bytes this relay actually read,
    // so this is not the field that would reach the machine wrong — but
    // nothing here should have to decide which of two same-named headers
    // `git` sent was the real one, and a hand-composed request carrying two
    // is refused the same as everywhere else in this repository.
    let lengths: Vec<&str> =
        headers.iter().filter(|(n, _)| n == "content-length").map(|(_, v)| v.as_str()).collect();
    if lengths.len() > 1 {
        return Err(format!(
            "two Content-Length fields, {}: this request has no one length",
            lengths.join(" and ")
        ));
    }
    Ok(Head { method: method.to_ascii_uppercase(), target: target.to_owned(), headers })
}

/// **The path the machine is asked for** — the token removed, and only this
/// one repository's paths let through.
///
/// # Errors
///
/// A target that does not begin with the token, or names another repository.
pub(super) fn forwarded_path(target: &str, token: &str, repository: &str) -> Result<String, String> {
    let rest = target
        .strip_prefix('/')
        .and_then(|t| t.strip_prefix(token))
        .filter(|rest| rest.starts_with('/'))
        .ok_or_else(|| "this relay answers only the URL `cargo rivet fleet push` handed to git".to_owned())?;
    let prefix = format!("/{repository}.git/");
    if !rest.starts_with(&prefix) {
        return Err(format!("this relay forwards only the `{repository}` repository"));
    }
    Ok(rest.to_owned())
}

/// **The body, whole** — exactly `Content-Length` bytes, or a chunked body
/// decoded; none for a request with neither.
///
/// # Errors
///
/// A body over [`BODY_LIMIT`], a malformed chunk, or a connection that closed
/// before the body did.
pub(super) fn read_body(reader: &mut dyn BufRead, head: &Head) -> Result<Vec<u8>, String> {
    if head.header("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked")) {
        return dechunk(reader);
    }
    let Some(length) = head.header("content-length") else {
        return Ok(Vec::new());
    };
    let length: usize = length.parse().map_err(|_| format!("Content-Length: {length:?} is not a number"))?;
    if length > BODY_LIMIT {
        return Err(format!("a request body of {length} bytes is over this relay's {BODY_LIMIT}"));
    }
    let mut body = vec![0_u8; length];
    reader.read_exact(&mut body).map_err(|e| format!("the request body ended early: {e}"))?;
    Ok(body)
}

/// Decode `Transfer-Encoding: chunked` (RFC 9112 §7.1), trailers discarded.
fn dechunk(reader: &mut dyn BufRead) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    loop {
        let mut size_line = String::new();
        if reader.read_line(&mut size_line).map_err(|e| format!("a chunk could not be read: {e}"))? == 0 {
            return Err("the connection closed inside a chunked body".to_owned());
        }
        let size_text = size_line.trim_end_matches(['\r', '\n']);
        let size_text = size_text.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| format!("{size_text:?} is not a chunk size"))?;
        if size == 0 {
            // Trailers, then the blank line that ends the message.
            loop {
                let mut trailer = String::new();
                let got = reader.read_line(&mut trailer).map_err(|e| format!("a trailer could not be read: {e}"))?;
                if got == 0 || trailer.trim_end_matches(['\r', '\n']).is_empty() {
                    return Ok(body);
                }
            }
        }
        if body.len().saturating_add(size) > BODY_LIMIT {
            return Err(format!("a chunked request body is over this relay's {BODY_LIMIT} bytes"));
        }
        let start = body.len();
        body.resize(start + size, 0);
        reader.read_exact(&mut body[start..]).map_err(|e| format!("a chunk ended early: {e}"))?;
        let mut crlf = [0_u8; 2];
        reader.read_exact(&mut crlf).map_err(|e| format!("a chunk was not terminated: {e}"))?;
        if &crlf != b"\r\n" {
            return Err("a chunk was not followed by CRLF".to_owned());
        }
    }
}

/// **The head the machine is sent.** Only what `git-http-backend` reads is
/// carried — the content type and encoding — plus a `Content-Length` for any
/// request with a body, which is the whole point of de-chunking.
pub(super) fn forward_head(head: &Head, path: &str, body_len: usize) -> String {
    let mut out = format!("{} {path} HTTP/1.1\r\nHost: rivet\r\n", head.method);
    for name in ["content-type", "content-encoding"] {
        if let Some(value) = head.header(name) {
            let canonical = if name == "content-type" { "Content-Type" } else { "Content-Encoding" };
            out.push_str(&format!("{canonical}: {value}\r\n"));
        }
    }
    if body_len > 0 || head.method == "POST" {
        out.push_str(&format!("Content-Length: {body_len}\r\n"));
    }
    out.push_str("Connection: close\r\n\r\n");
    out
}

fn status_of(answer: &[u8]) -> Option<u16> {
    let line_end = answer.iter().position(|b| *b == b'\n').unwrap_or(answer.len());
    let line = String::from_utf8_lossy(&answer[..line_end]);
    line.split(' ').nth(1).and_then(|code| code.trim().parse().ok())
}

fn refuse(writer: &mut dyn Write, status: u16, reason: &str, mut seen: Seen, why: &str) -> Seen {
    let body = if why.ends_with('\n') { why.to_owned() } else { format!("{why}\n") };
    let _ = writer.write_all(
        format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .as_bytes(),
    );
    seen.status = seen.status.or(Some(status));
    if seen.detail.is_empty() {
        seen.detail = body;
    }
    seen
}

#[cfg(test)]
#[path = "fleet_push_tests.rs"]
mod tests;
