//! **A stand-in for an installed Rivet application**, so that `ADR-017` §4's
//! second origin can be exercised before anything can install one.
//!
//! The maintainer's 2026-08-24 ruling settled that an application service's
//! restart policy, readiness contract and sockets come from its signed manifest
//! and that the operator's document may only cap it. `cmd/rivet-sysd`'s
//! `applications` module implements that; **nothing installs an application**
//! (`ADR-029` §12 item 4 is ruled and unwritten), so without this there is
//! nothing on any machine for that code to read.
//!
//! # What it is honest about
//!
//! It is **not** an application in the `ADR-015` sense and must never be
//! described as one: it is not packaged, not signed, and not verified. The
//! harness copies it to `/state/app/<name>/current/bin/<name>` beside a
//! hand-written manifest, which is exactly what an installer would leave behind
//! and is reached by a different route. What a run of it settles is what the
//! *supervisor* does with an installed application, and nothing at all about
//! installing one.
//!
//! # Four behaviours, chosen by the name it was installed under
//!
//! **`argv[0]`, and not an argument or a variable.** The
//! harness installs this one binary under several names, and an application's
//! definition comes from its signed manifest: there is no field an operator
//! could put a mode in, and inventing one would be testing a plumbing path that
//! does not exist for real applications.
//!
//! | installed as | what it does |
//! | :-- | :-- |
//! | `…provider…` | accepts on the descriptor it was handed, reports each connection |
//! | `…consumer…` | reads `RIVET_CONNECT`, connects, reports what happened |
//! | `…interloper…` | connects to the SAME path, knowing it, having declared nothing |
//! | `…confinement…` | opens what it was HANDED and what it was NOT, and reports both |
//! | anything else | prints its pid and sleeps, which is what it always did |
//!
//! The interloper is the half that makes the other two mean anything. A
//! consumer that connects proves a socket is reachable; only a **third**
//! application that knows the path and is refused proves the grant was to
//! *that* consumer rather than to everyone — which is the sentence
//! `rivet_core::service::LISTEN_SOCKET_MODE` said could not be expressed before
//! `ADR-015` §5.3 was built.
//!
//! # `confined` is the same idea one layer down — `ADR-042` §6
//!
//! Added 2026-09-05, after a Landlock ruleset that only ever *refused* reached
//! hardware and took a display down. **A probe that shows only the refusal
//! cannot tell a working ruleset from a missing file**, so this one opens both
//! kinds of path in one process, on one boot, at one uid, and prints the
//! `errno` for each. Two failures it is specifically built to catch:
//!
//! * a refusal that is `ENOENT` rather than `EACCES` — nothing was enforced,
//!   the file was simply not there; and
//! * a grant that is *also* refused — the ruleset works and the application is
//!   broken, which is exactly what shipped on 2026-09-04.
//!
//! **It also tries to `bind` a loopback TCP port** — `ADR-042` §7's
//! amendment. There is no *handed* side to this one: no application has a
//! lawful reason to bind a TCP port at all, so the only thing to show is the
//! refusal, printed on its own line with whatever errno the kernel actually
//! returned.

use std::io::{Read as _, Write as _};
use std::os::fd::FromRawFd as _;
use std::os::unix::net::{UnixListener, UnixStream};

/// The descriptor a supervisor passes the first listening socket on —
/// `ADR-017` §3.4. `LISTEN_FDS` counts them and they start here.
const FIRST_LISTEN_FD: i32 = 3;

fn main() {
    let name = std::env::args().next().unwrap_or_default();
    if name.contains("provider") {
        provider();
    } else if name.contains("consumer") {
        consumer();
    } else if name.contains("interloper") {
        interloper();
    } else if name.contains("confinement") {
        confined();
    } else {
        sleeper();
    }
}

/// **What the machine HANDED this application**, and what it did not.
///
/// `ADR-042` §6. Each row is a path and which side of the policy it is meant to
/// be on; the point of running both in one process is that the *same* uid, the
/// *same* boot and the *same* binary produce both answers, so neither can be
/// explained away.
///
/// `RIVET_APP_DIR` and `RIVET_APP_DATA` are read out of the environment rather
/// than composed — `rivet_core::service`'s whole argument — and the two
/// `/system` and `/state/config` rows are spelled out, because the point of
/// those is that this program *knows* the path and is still refused.
fn confined() -> ! {
    println!("confined: pid {}", std::process::id());

    let handed: Vec<(&str, String)> = [
        ("RIVET_APP_DIR", std::env::var("RIVET_APP_DIR")),
        ("RIVET_APP_DATA", std::env::var("RIVET_APP_DATA")),
        ("RIVET_APP_SETTINGS", std::env::var("RIVET_APP_SETTINGS")),
    ]
    .into_iter()
    .filter_map(|(label, value)| value.ok().map(|v| (label, v)))
    .collect();
    for (label, path) in &handed {
        println!("confined: {label}={path}");
    }

    let mut granted_ok = 0usize;
    let mut granted_total = 0usize;
    // **The typeface is in this list and it is the row the regression was
    // about.** The path is spelled here because this fixture does not link
    // `rivet-core`; `tools/qemu/run.sh` packs the file at the path
    // `rivet_core::etc::SYSTEM_FONT` names, and a drift between the two shows
    // up as `ENOENT` on the line below, which is reported as *not proven*
    // rather than as a pass.
    let mut reachable: Vec<String> = vec!["/system/share/fonts/Geist.ttf".to_string()];
    reachable.extend(
        handed
            .iter()
            .filter(|(label, _)| *label != "RIVET_APP_DIR")
            .map(|(_, path)| path.clone()),
    );
    if let Some((_, dir)) = handed.iter().find(|(label, _)| *label == "RIVET_APP_DIR") {
        reachable.push(format!("{dir}/manifest.toml"));
    }
    // `metadata` is used ONLY to choose between reading a file and listing a
    // directory; the answer that counts is the `read`/`read_dir` below, because
    // `stat(2)` is not a thing Landlock restricts — see the note on `outside`.
    for path in &reachable {
        granted_total += 1;
        match std::fs::metadata(path) {
            Err(e) => println!("confined: HANDED  {path}: NOT OPENABLE - {e}"),
            Ok(meta) if meta.is_dir() => match std::fs::read_dir(path) {
                Ok(entries) => {
                    granted_ok += 1;
                    println!(
                        "confined: HANDED  {path}: LISTED, {} entr(ies)",
                        entries.count()
                    );
                }
                Err(e) => println!("confined: HANDED  {path}: NOT LISTABLE - {e}"),
            },
            Ok(_) => match std::fs::read(path) {
                Ok(bytes) => {
                    granted_ok += 1;
                    println!("confined: HANDED  {path}: READ {} byte(s)", bytes.len());
                }
                Err(e) => println!("confined: HANDED  {path}: NOT READABLE - {e}"),
            },
        }
    }

    // **Paths this application knows and never declared.** Every one of them
    // exists on the machine — `run.sh` packs the document and the supervisor's
    // own binary — which is the whole point: a refusal that is `ENOENT` proves
    // nothing at all and is counted as such below.
    //
    // **`open`, and deliberately NOT `stat`.** Landlock governs opening a file
    // and opening a directory; it does not govern `stat(2)`, so a probe written
    // with `std::fs::metadata` reports every one of these REACHED on a machine
    // where the ruleset is working perfectly. That is exactly what the first
    // draft of this function did, and it is the same species of mistake as the
    // regression it was written for: a check that cannot tell the two answers
    // apart. Opening a directory read-only is what `LANDLOCK_ACCESS_FS_READ_DIR`
    // covers, so one call answers for both kinds of path here.
    let outside = [
        "/state/config/machine.toml",
        "/system/bin/rivet-sysd",
        "/state/app",
        "/tmp",
    ];
    let mut refused = 0usize;
    let mut hollow = 0usize;
    for path in outside {
        match std::fs::File::open(path) {
            Ok(_) => println!(
                "confined: OUTSIDE {path}: OPENED IT — the ruleset did not deny a path nothing \
                 declared"
            ),
            Err(e) if e.raw_os_error() == Some(EACCES) => {
                refused += 1;
                println!("confined: OUTSIDE {path}: refused, {e}");
            }
            Err(e) if e.raw_os_error() == Some(ENOENT) => {
                hollow += 1;
                println!(
                    "confined: OUTSIDE {path}: NOT THERE, {e} — this row proves NOTHING about \
                     the ruleset; it is a missing file"
                );
            }
            Err(e) => {
                hollow += 1;
                println!("confined: OUTSIDE {path}: refused for another reason, {e}");
            }
        }
    }

    // **The verdict names both halves and refuses to call one of them the
    // proof.** A run where everything was refused and nothing was reachable is
    // an application that cannot work; a run where everything was reachable is
    // a ruleset that enforces nothing. Only the pair says anything.
    println!(
        "confined: VERDICT handed {granted_ok}/{granted_total} reachable, {refused}/{} refused \
         with EACCES ({hollow} of them prove nothing) - {}",
        outside.len(),
        if granted_ok == granted_total && refused == outside.len() {
            "BOTH HALVES"
        } else {
            "NOT PROVEN"
        }
    );

    // **`ADR-042` §7's amendment: `LANDLOCK_ACCESS_NET_BIND_TCP` is handled
    // and granted to nothing.** A separate line and a separate verdict from
    // the filesystem VERDICT above, on purpose: this right did not exist
    // before ABI 4, so a machine running an older-but-present Landlock is
    // expected to print `BIND_TCP: OPENED IT` here and that is not a defect
    // in the ruleset — it is `handled_access_net_for` correctly doing
    // nothing below the ABI that defines the field at all. The harness reads
    // `landlock_abi` off the same console line the filesystem half already
    // prints and decides which answer this row owes.
    //
    // **The errno is whatever the kernel actually returned**, printed via
    // `{e}` rather than assumed — `AGENTS.md` §7: a claim about what a
    // refusal looks like is worth exactly what it was measured against.
    match std::net::TcpListener::bind("127.0.0.1:0") {
        Ok(listener) => println!(
            "confined: BIND_TCP {:?}: OPENED IT — the ruleset did not deny a TCP bind nothing \
             declared",
            listener.local_addr()
        ),
        Err(e) => println!("confined: BIND_TCP: refused, {e}"),
    }

    sleeper();
}

/// `EACCES`, spelled here because this fixture links no `libc`.
const EACCES: i32 = 13;
/// `ENOENT`, likewise. **The one a refusal must NOT be**, which is why it is
/// counted apart rather than folded into the total.
const ENOENT: i32 = 2;

/// Print the pid and stay up, which is what this fixture has always been for.
fn sleeper() -> ! {
    // Standard output is inherited from the supervisor, which is either the
    // console or `ADR-022` §4's capture. One line, so that a restart loop is
    // legible rather than a wall of text.
    println!(
        "rivet-testapp: running as pid {}, and doing nothing else",
        std::process::id()
    );

    // A long sleep in a loop rather than a park: parking is indistinguishable
    // from a hang if the process ever stops being woken, and this fixture's job
    // is to be a service that stays up rather than one that is merely present.
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

/// Accept on the descriptor the supervisor handed over, and say what arrived.
///
/// **It never opens a path.** `ADR-015` §5.2 is that Rivet opens the socket and
/// passes the descriptor, and a provider that also knew where the file was
/// would be testing something the design says does not happen.
fn provider() -> ! {
    let names = std::env::var("LISTEN_FDNAMES").unwrap_or_default();
    println!("provider: LISTEN_FDNAMES={names}");
    if names.is_empty() {
        println!("provider: NO DESCRIPTOR WAS PASSED, so there is nothing to accept on");
        sleeper();
    }

    // Safety: `ADR-017` §3.4 gives descriptor 3 to the first declared socket,
    // and `LISTEN_FDNAMES` above is the supervisor saying it did so.
    let listener = unsafe { UnixListener::from_raw_fd(FIRST_LISTEN_FD) };
    println!("provider: listening, pid {}", std::process::id());
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let mut said = String::new();
                let _ = stream.read_to_string(&mut said);
                println!("provider: ACCEPTED a connection saying {said:?}");
                let _ = stream.write_all(b"provider heard you");
            }
            Err(e) => {
                println!("provider: accept failed: {e}");
                sleeper();
            }
        }
    }
}

/// Read the grant out of the environment and use it.
///
/// The path is **not** derived. `rivet_core::service::CONNECT_ENV` carries why:
/// it is a function of the provider's name and where Rivet puts sockets, and an
/// application that computed it would be coupled to both.
fn consumer() -> ! {
    let Ok(granted) = std::env::var("RIVET_CONNECT") else {
        println!("consumer: RIVET_CONNECT IS NOT SET, so nothing was granted");
        sleeper();
    };
    println!("consumer: RIVET_CONNECT={granted}");

    // `owner:name=path`, space separated. One entry here.
    let Some(path) = granted.split(' ').next().and_then(|e| e.split_once('=')) else {
        println!("consumer: RIVET_CONNECT is not owner:name=path");
        sleeper();
    };
    let path = path.1;

    match UnixStream::connect(path) {
        Ok(mut stream) => {
            let _ = stream.write_all(b"hello from the consumer");
            let _ = stream.shutdown(std::net::Shutdown::Write);
            let mut back = String::new();
            let _ = stream.read_to_string(&mut back);
            println!("consumer: CONNECTED to {path} and heard {back:?}");
        }
        Err(e) => println!("consumer: COULD NOT CONNECT to {path}: {e}"),
    }
    sleeper();
}

/// Know the path, declare nothing, and try anyway.
///
/// This is the negative half and the only one that proves a grant is a grant.
/// It hardcodes what the consumer was told, so the difference between the two
/// runs is **the declaration and nothing else** — same path, same machine, same
/// binary, different manifest.
fn interloper() -> ! {
    println!(
        "interloper: RIVET_CONNECT={:?}, which should be absent",
        std::env::var("RIVET_CONNECT").ok()
    );
    let path = "/run/rivet/listen/a-socket-provider/api";
    match UnixStream::connect(path) {
        Ok(_) => println!(
            "interloper: CONNECTED to {path} — THE GRANT IS NOT A GRANT, anything on this \
             machine can reach a socket meant for one application"
        ),
        Err(e) => println!("interloper: refused at {path}: {e}"),
    }
    sleeper();
}
