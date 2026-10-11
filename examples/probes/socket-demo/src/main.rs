//! Adopt the descriptors Rivet passed, and serve HTTP on a privileged port
//! without ever having held the privilege.
//!
//! The `LISTEN_FDS` protocol, in full, because it is fifteen lines and a
//! dependency would hide the thing being demonstrated:
//!
//! * `LISTEN_PID` must equal this process's pid. **If it does not, the
//!   descriptors are not ours** — the variable was inherited through an exec
//!   and belongs to an ancestor. Refusing is the entire point of the check.
//! * `LISTEN_FDS` is how many, starting at descriptor **3**.
//! * `LISTEN_FDNAMES` is systemd's extension: colon-separated, one per
//!   descriptor, in the same order. Rivet fills it from the manifest, so it
//!   reads `tcp:80:unix:api` for this application.
//!
//! `ADR-015` §5.2 tells an author to *"honour `LISTEN_FDS` when it is set and
//! bind when it is not"*, so that one binary works before and after this
//! feature exists. This does that, and says out loud which path it took.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::fd::FromRawFd;
use std::os::unix::net::UnixListener;

/// The first descriptor a passed socket can be on. 0, 1 and 2 are the streams
/// the supervisor already set up.
const FIRST_FD: i32 = 3;

fn main() {
    let fds = passed_descriptors();
    let names: Vec<String> = std::env::var("LISTEN_FDNAMES")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();

    println!(
        "socket-demo: running as uid {}, pid {}",
        own_uid().unwrap_or_else(|| "unknown".to_string()),
        std::process::id()
    );

    let listener = match fds {
        0 => {
            // The fallback §5.2 asks for. On a Rivet that has this feature it
            // is never taken; on one that does not, `bind` on 80 answers
            // EACCES and this says so rather than dying silently.
            println!("socket-demo: NO descriptors were passed — falling back to binding my own");
            match TcpListener::bind(("0.0.0.0", 80)) {
                Ok(l) => l,
                Err(e) => {
                    println!("socket-demo: and bind(80) failed, as an unprivileged uid must: {e}");
                    // Nothing to serve. Sleep rather than exit, so the failure
                    // is a running service saying what is wrong rather than a
                    // restart loop burning the card (`ADR-017` §3.5).
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(3600));
                    }
                }
            }
        }
        n => {
            println!("socket-demo: {n} descriptor(s) passed by Rivet: {}", names.join(", "));
            // SAFETY: descriptor 3 is a listening socket the supervisor bound
            // and `LISTEN_PID` confirmed is ours. Nothing else in this process
            // has touched it, so taking ownership here is the only claim on it.
            let tcp = unsafe { TcpListener::from_raw_fd(FIRST_FD) };
            if n > 1 {
                // The second one is the unix socket. It is not served — it is
                // adopted and reported, because the claim being made is that
                // BOTH kinds arrive, and a program that only ever looked at
                // descriptor 3 would not have tested the second path.
                // SAFETY: as above, for descriptor 4.
                let unix = unsafe { UnixListener::from_raw_fd(FIRST_FD + 1) };
                match unix.local_addr() {
                    Ok(addr) => println!("socket-demo: and a unix socket at {addr:?}"),
                    Err(e) => println!("socket-demo: the unix socket is unreadable: {e}"),
                }
                std::mem::forget(unix);
            }
            tcp
        }
    };

    match listener.local_addr() {
        Ok(addr) => println!("socket-demo: serving HTTP on {addr} — a privileged port, never bound by me"),
        Err(e) => println!("socket-demo: the passed socket has no address: {e}"),
    }

    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let mut scratch = [0u8; 1024];
        let _ = stream.read(&mut scratch);
        let body = format!(
            "socket-demo on a Rivet\n\
             uid            {}\n\
             pid            {}\n\
             LISTEN_FDS     {}\n\
             LISTEN_FDNAMES {}\n\
             \n\
             This process never called bind(2). Port 80 is below 1024 and this\n\
             uid has no capabilities, so bind would have answered EACCES.\n\
             rivet-sysd opened the socket and passed the descriptor.\n\
             ADR-015 §5.2, ADR-017 §3.4.\n",
            own_uid().unwrap_or_else(|| "unknown".to_string()),
            std::process::id(),
            fds,
            names.join(", "),
        );
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        println!("socket-demo: answered a request");
    }
}

/// How many descriptors this process was passed, after checking they are for
/// **this** process.
fn passed_descriptors() -> i32 {
    let Ok(count) = std::env::var("LISTEN_FDS") else {
        return 0;
    };
    let Ok(count) = count.parse::<i32>() else {
        return 0;
    };
    match std::env::var("LISTEN_PID").ok().and_then(|p| p.parse::<u32>().ok()) {
        Some(pid) if pid == std::process::id() => count,
        Some(pid) => {
            // Inherited through an exec: these belong to an ancestor. Reporting
            // it rather than silently returning 0 is the difference between a
            // diagnosable machine and a mystery.
            println!(
                "socket-demo: LISTEN_PID is {pid} and I am {} — these descriptors are not mine",
                std::process::id()
            );
            0
        }
        None => 0,
    }
}

/// This process's real uid, read from `/proc`.
///
/// `/proc/self/status` rather than `getuid(2)` so that this example needs no
/// dependency at all — the point of it is the socket, and a reader should not
/// have to wonder whether a crate is doing something clever.
fn own_uid() -> Option<String> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("Uid:"))?;
    Some(line.split_whitespace().nth(1)?.to_string())
}
