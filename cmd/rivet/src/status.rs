//! **Asking the supervisor what it is doing.** `ADR-032` §3.2's query socket,
//! from the client end.
//!
//! `crate::journal`'s `send` is the sibling and this is the same shape with a
//! read added: connect, write one line, and — unlike the ingest socket, which is
//! one-way — read the answer back. `ADR-022` §11's socket takes records and
//! answers nothing; this one answers and takes nothing but a request.
//!
//! # The refusal is the interesting half
//!
//! `ADR-032` §3.3 item 5 rules it, and the reason is worth keeping in front of
//! whoever changes this:
//!
//! > A machine whose supervisor is down is exactly when somebody asks.
//!
//! So a connection that fails is **the ordinary path**, not an exception, and
//! the refusal has to be a sentence an operator can act on: it names the socket,
//! says the supervisor is not answering, and points at the journal — which is on
//! disk and survives a supervisor that is not (`ADR-022` §8). A bare
//! `ECONNREFUSED` would be true and useless.
//!
//! # What is deliberately not here
//!
//! **Any retry.** `crate::deploy` records the same posture for its own channel: a caller
//! who wants one can ask again, and a client that retried would turn *the
//! supervisor is down* — the answer being asked for — into a pause.
//!
//! **Any parsing of its own.** `rivet_core::operation::status::Status::parse` is
//! the reader, beside the writer, with the round trip asserted. This module
//! moves bytes.
//!
//! # It is the socket's client, and it is named for the first thing that
//! crossed it
//!
//! **Amended 2026-08-29.** `ADR-017` §6.3.1 made this socket the supervisor's
//! control surface and `apply service-restart` is its second tenant, so
//! [`exchange_at`] is the byte-moving half and [`ask`] and [`restart`] are two
//! callers of it. The module keeps the name `status` — a rename would touch
//! every site that reaches the socket for no gain today — and this paragraph is
//! here so that is a known compromise rather than a discovered one.

// **Unused on Windows, load-bearing on the target.** The `Detached`/`Syscall`
// implementations below are `cfg`-gated to unix, so a Windows build sees no
// use of these names and warns. They are NOT removable: deleting them on that
// warning's word broke the aarch64-musl cross build -- which is the build that
// makes a card -- and it went unnoticed because the check had only ever been
// run on the host. `PLAN.md`: a warning is a claim about ONE configuration.
#[cfg_attr(windows, allow(unused_imports))]
use rivet_core::operation::{MachineError, Restarting, Status, StatusError};
// `ADR-017` §6.3.1's control surface, 2026-08-29 — the request grammar and both
// halves of the answer's wire format are the domain's.
#[cfg_attr(windows, allow(unused_imports))]
use rivet_core::operation::service_restart::Request;
// **Unused on Windows, load-bearing on the target.** The `Detached`/`Syscall`
// implementations below are `cfg`-gated to unix, so a Windows build sees no
// use of these names and warns. They are NOT removable: deleting them on that
// warning's word broke the aarch64-musl cross build -- which is the build that
// makes a card -- and it went unnoticed because the check had only ever been
// run on the host. `PLAN.md`: a warning is a claim about ONE configuration.
#[cfg_attr(windows, allow(unused_imports))]
use rivet_core::operation::status::{QUERY_SOCKET_PATH, REQUEST, RESPONSE_MAX_BYTES};

/// Ask, and return what the supervisor said.
pub fn ask() -> Result<Status, MachineError> {
    ask_at(QUERY_SOCKET_PATH)
}

/// **Ask the supervisor to restart one service, or to say what it would do.**
/// `ADR-017` §6.3.2, added 2026-08-29. `apply` is `false` for the plan path.
///
/// The supervisor's refusal comes back as a value rather than as prose, so the
/// sentence an operator reads is composed once, in `rivet-core`, from the facts
/// — see `rivet_core::operation::ServiceRestartRefusal`.
pub fn restart(service: &str, apply: bool) -> Result<Restarting, MachineError> {
    restart_at(QUERY_SOCKET_PATH, service, apply)
}

fn restart_at(path: &str, service: &str, apply: bool) -> Result<Restarting, MachineError> {
    let request = Request::ServiceRestart {
        service: service.to_string(),
        apply,
    };
    let answer = exchange_at(path, &request.render())?;
    match rivet_core::operation::service_restart::parse_answer(&answer) {
        Ok(Ok(done)) => Ok(done),
        Ok(Err(refusal)) => Err(refusal.as_machine_error()),
        Err(e) => Err(MachineError::Refused(format!(
            "the supervisor answered at {path} and this build could not read the answer: {e} The \
             answer was {answer:?}. NOTHING IS ASSERTED about whether the restart was taken — \
             `rivet get status` and `rivet get log {service}` are the authority."
        ))),
    }
}

/// **Ask the supervisor to stop one application and hold it stopped, or to
/// release that hold — or to say what it would do.** `ADR-017` §6. `apply` is
/// `false` for the plan path.
pub fn hold(
    kind: rivet_core::operation::ServiceHoldKind,
    service: &str,
    apply: bool,
) -> Result<rivet_core::operation::ServiceHold, MachineError> {
    hold_at(QUERY_SOCKET_PATH, kind, service, apply)
}

fn hold_at(
    path: &str,
    kind: rivet_core::operation::ServiceHoldKind,
    service: &str,
    apply: bool,
) -> Result<rivet_core::operation::ServiceHold, MachineError> {
    let request = Request::ServiceHold {
        kind,
        service: service.to_string(),
        apply,
    };
    let answer = exchange_at(path, &request.render())?;
    match rivet_core::operation::service_hold::parse_answer(&answer) {
        Ok(Ok(done)) if done.kind == kind && done.service == service => Ok(done),
        Ok(Ok(other)) => Err(MachineError::Refused(format!(
            "the supervisor at {path} answered about `{} {}` when `{kind} {service}` was asked. \
             NOTHING IS ASSERTED about what it did — `rivet get status` is the authority.",
            other.kind, other.service
        ))),
        Ok(Err(refusal)) => Err(refusal.as_machine_error()),
        Err(e) => Err(MachineError::Refused(format!(
            "the supervisor answered at {path} and this build could not read the answer: {e} The \
             answer was {answer:?}. NOTHING IS ASSERTED about whether `{kind}` was taken — `rivet \
             get status` and `rivet get log {service}` are the authority."
        ))),
    }
}

/// **Ask the supervisor to make the running clock the clock floor, or to say
/// what that would do.** `ADR-021` §3. `apply` is `false` for the plan path.
pub fn clock_floor(apply: bool) -> Result<rivet_core::operation::clock_floor::Floor, MachineError> {
    clock_floor_at(QUERY_SOCKET_PATH, apply)
}

fn clock_floor_at(
    path: &str,
    apply: bool,
) -> Result<rivet_core::operation::clock_floor::Floor, MachineError> {
    let answer = exchange_at(path, &Request::ClockFloorReset { apply }.render())?;
    match rivet_core::operation::clock_floor::parse_answer(&answer) {
        Ok(Ok(done)) => Ok(done),
        Ok(Err(refusal)) => Err(refusal.as_machine_error()),
        Err(e) => Err(MachineError::Refused(format!(
            "the supervisor answered at {path} and this build could not read the answer: {e} The \
             answer was {answer:?}. NOTHING IS ASSERTED about whether the floor was reset — `rivet \
             get log latest all` carries the supervisor's own record of it."
        ))),
    }
}

/// The same, to a named socket.
///
/// The path is a parameter for `crate::journal::send_to`'s reason and
/// `AGENTS.md` §8.3's: a test binds one in a temporary directory and drives the
/// real syscalls, so every branch below is reachable without a board and nothing
/// here writes to a path a development host might actually have.
fn ask_at(path: &str) -> Result<Status, MachineError> {
    let answer = exchange_at(path, REQUEST)?;
    Status::parse(&answer).map_err(|e: StatusError| {
        MachineError::Refused(format!(
            "the supervisor answered at {path} and this build could not read the answer: {e}"
        ))
    })
}

/// **One request, one answer, and nothing understood.** The whole of this
/// module's I/O, shared by every operation that crosses the control surface.
fn exchange_at(path: &str, request: &str) -> Result<String, MachineError> {
    #[cfg(unix)]
    {
        use std::io::{Read as _, Write as _};

        let mut socket = std::os::unix::net::UnixStream::connect(path).map_err(|e| {
            MachineError::Refused(format!(
                "the supervisor is not answering at {path} ({e}). `rivet get status` is the one \
                 question that has to be asked of the running process — everything else this \
                 machine reports is on disk and survives it (ADR-032 §3.2). If the supervisor is \
                 down, `rivet get log` still says what it did before it stopped, and that is \
                 where to look."
            ))
        })?;

        socket.write_all(request.as_bytes()).map_err(|e| {
            MachineError::Refused(format!(
                "the supervisor accepted a connection at {path} and would not take the request \
                 ({e}). NOTHING WAS READ."
            ))
        })?;
        // The write half is closed so the far end sees the request end without
        // needing a length — the same framing `crate::journal::send_to` uses,
        // where the close IS the terminator.
        let _ = socket.shutdown(std::net::Shutdown::Write);

        // **Bounded, and the bound is the domain's.** A response longer than
        // `RESPONSE_MAX_BYTES` is not truncated and read: a half-read status is
        // a status with services missing from the end of it, which is worse
        // than no answer because it looks like a complete one.
        let mut answer = String::new();
        let read = std::io::Read::take(&mut socket, RESPONSE_MAX_BYTES as u64 + 1)
            .read_to_string(&mut answer)
            .map_err(|e| {
                MachineError::Refused(format!(
                    "the request reached the supervisor at {path} and the answer could not be \
                     read ({e}). NOTHING IS ASSERTED about what it is doing."
                ))
            })?;
        if read > RESPONSE_MAX_BYTES {
            return Err(MachineError::Refused(format!(
                "the supervisor's answer is longer than {RESPONSE_MAX_BYTES} bytes, which is \
                 more than this build will read. It is REFUSED rather than cut: a status missing \
                 the services at the end of it looks exactly like a complete one."
            )));
        }

        Ok(answer)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, request);
        Err(MachineError::Refused(
            format!(
                "this rivet was built for a platform with no AF_UNIX sockets, so there is no way  \
                 to ask a supervisor anything. {}",
                rivet_core::control::ASK_A_BOARD
            ),
        ))
    }
}

#[cfg(all(test, unix))]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use rivet_core::operation::ServiceStatus;
    use std::io::{Read as _, Write as _};

    /// A socket in a temporary directory that answers `reply` once, and records
    /// the request it was given.
    fn supervisor_saying(reply: String) -> (String, std::thread::JoinHandle<String>) {
        let dir = std::env::temp_dir().join(format!(
            "rivet-status-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("query").to_string_lossy().into_owned();
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind");
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept");
            let mut asked = String::new();
            socket.read_to_string(&mut asked).expect("read the request");
            socket.write_all(reply.as_bytes()).expect("write the reply");
            asked
        });
        (path, handle)
    }

    fn sample() -> Status {
        Status {
            phase: "converging".to_string(),
            // `Some`, so the round trip below carries a line with something
            // in every field — `R33`, `.rulings/` R-163.
            version: Some("0.1.86".to_string()),
            // **The mode crosses the socket like everything else here**, and
            // this fixture is a kiosk so the round-trip test below covers a
            // line with something in every field rather than four dashes.
            kiosk: rivet_core::operation::status::KioskMode::kiosk(
                true,
                Some("aura-renderer".to_string()),
            ),
            // **The observations, not the declaration** — `rivet_core::
            // operation::status::Status` keeps these beside `kiosk` rather than
            // inside it, and its own field comments say why. Both are `Some`
            // here so the round trip below carries a line with something in
            // every field; the same module's tests cover the `None` halves.
            display_master: Some("rivet-launcher".to_string()),
            display_leased: Some("aura-renderer".to_string()),
            // **`ADR-045` §4's fold, and the caption beside it.** Not
            // `healthy` and not `None`, so the round trip below carries
            // something in every one of the three fields the health strand
            // added rather than in their defaults.
            machine: rivet_core::health::State::Degraded,
            services: vec![ServiceStatus {
                name: "rivet-mdnsd".to_string(),
                enabled: true,
                state: "started".to_string(),
                pid: Some(214),
                restarts: 0,
                ready: true,
                reason: None,
                health: rivet_core::health::State::Degraded,
                caption: Some("no broadcast from GVH5075_2222 in 34 minutes".to_string()),
                critical: true,
            }],
        }
    }

    /// **The whole round trip over a real socket**, which is what this module
    /// exists to do and the only part of it a unit test can reach.
    #[test]
    fn a_supervisor_that_answers_is_read_back_exactly() {
        let status = sample();
        let (path, server) = supervisor_saying(status.render());
        assert_eq!(ask_at(&path), Ok(status));
        // And it sent an OPERATION, not a bare subject — ADR-032 §3.3 item 1.
        assert_eq!(server.join().expect("the server thread"), REQUEST);
        let _ = std::fs::remove_file(&path);
    }

    /// **A supervisor that is not there is the ordinary case**, and the refusal
    /// has to be actionable: it names the socket and points at the journal,
    /// which is on disk and survives a supervisor that is not.
    #[test]
    fn a_supervisor_that_is_not_answering_says_so_and_says_where_to_look() {
        let missing = std::env::temp_dir()
            .join("rivet-status-nothing-here/query")
            .to_string_lossy()
            .into_owned();
        let refusal = ask_at(&missing).expect_err("nothing is listening");
        let why = refusal.why();
        assert!(why.contains(&missing), "{why}");
        assert!(why.contains("not answering"), "{why}");
        assert!(why.contains("rivet get log"), "{why}");
    }

    /// An answer this build cannot read is refused with the reader's own
    /// diagnostic, rather than an empty status that looks like a quiet machine.
    #[test]
    fn an_unreadable_answer_is_refused_and_carries_the_readers_reason() {
        let (path, server) = supervisor_saying("phase\tconverging\nvolume\t/state\n".to_string());
        let refusal = ask_at(&path).expect_err("an unknown line must refuse");
        assert!(
            refusal.why().contains("newer than this CLI"),
            "{}",
            refusal.why()
        );
        let _ = server.join();
        let _ = std::fs::remove_file(&path);
    }

    // -----------------------------------------------------------------------
    // ADR-017 §6.3.2 — service control over the same socket
    // -----------------------------------------------------------------------

    /// A socket that **parses the request with the domain's own parser** and
    /// answers with the domain's own renderer — so this is the whole exchange
    /// and not a hand-written reply. `AGENTS.md` §4: where a format crosses an
    /// implementation boundary, drive the boundary.
    fn supervisor_that_answers_restarts(
        answer: Result<Restarting, rivet_core::operation::ServiceRestartRefusal>,
    ) -> (String, std::thread::JoinHandle<Request>) {
        let dir = std::env::temp_dir().join(format!(
            "rivet-restart-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("query").to_string_lossy().into_owned();
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind");
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept");
            let mut asked = String::new();
            socket.read_to_string(&mut asked).expect("read the request");
            let request = Request::parse(&asked).expect("a request this build knows");
            let reply = match &answer {
                Ok(done) => done.render(),
                Err(refusal) => refusal.render(),
            };
            socket.write_all(reply.as_bytes()).expect("write the reply");
            request
        });
        (path, handle)
    }

    fn restarting(service: &str, reachability: bool) -> Restarting {
        Restarting {
            service: service.to_string(),
            was: "started".to_string(),
            doing: rivet_core::operation::Doing::Stop,
            reachability,
        }
    }

    /// **The whole round trip for the second thing that ever crossed this
    /// socket**, over a real one: the CLI renders the request, the supervisor's
    /// parser reads it, the answer is rendered and read back exactly.
    #[test]
    fn a_restart_request_and_its_answer_survive_a_real_socket() {
        let expected = restarting("photo-index", false);
        let (path, server) = supervisor_that_answers_restarts(Ok(expected.clone()));
        assert_eq!(restart_at(&path, "photo-index", true), Ok(expected));
        assert_eq!(
            server.join().expect("the server thread"),
            Request::ServiceRestart {
                service: "photo-index".to_string(),
                apply: true,
            }
        );
        let _ = std::fs::remove_file(&path);
    }

    /// **A plan is a different request on the wire**, so a supervisor cannot
    /// mistake a dry run for the act. `ADR-006` §6.5.
    #[test]
    fn the_plan_path_sends_plan_and_not_apply() {
        let (path, server) = supervisor_that_answers_restarts(Ok(restarting("photo-index", false)));
        let _ = restart_at(&path, "photo-index", false).expect("an answer");
        assert_eq!(
            server.join().expect("the server thread"),
            Request::ServiceRestart {
                service: "photo-index".to_string(),
                apply: false,
            }
        );
        let _ = std::fs::remove_file(&path);
    }

    /// **The supervisor's refusal arrives as a refusal**, with the domain's
    /// sentence and the right severity — `Unsafe` for one that will not change
    /// on a retry. `ADR-017` §6.3.2.
    #[test]
    fn a_refusal_arrives_as_the_domains_sentence_and_names_both_remedies() {
        let (path, server) = supervisor_that_answers_restarts(Err(
            rivet_core::operation::ServiceRestartRefusal::Disabled {
                service: "photo-index".to_string(),
            },
        ));
        let refusal = restart_at(&path, "photo-index", true).expect_err("a disabled service");
        assert!(
            matches!(refusal, MachineError::Unsafe(_)),
            "a refusal that will not change on a retry must be Unsafe: {refusal:?}"
        );
        let why = refusal.why();
        assert!(why.contains("REFUSED"), "{why}");
        assert!(why.contains("apply edit"), "{why}");
        assert!(why.contains("apply reload"), "{why}");
        let _ = server.join();
        let _ = std::fs::remove_file(&path);
    }

    /// The caveat travels: a service in the supervisor's reachability set comes
    /// back flagged, and the sentence is the domain's.
    #[test]
    fn the_reachability_caveat_arrives_from_the_supervisors_own_answer() {
        let (path, server) = supervisor_that_answers_restarts(Ok(restarting("rivet-mcpd", true)));
        let done = restart_at(&path, "rivet-mcpd", true).expect("permitted");
        assert!(
            done.caveat()
                .is_some_and(|c| c.contains("MAY NOT ARRIVE")),
            "no caveat"
        );
        let _ = server.join();
        let _ = std::fs::remove_file(&path);
    }

    /// **A stop request and its answer survive a real socket**, with the
    /// supervisor's own parser on the far end — `ADR-017` §6.
    #[test]
    fn a_hold_request_and_its_answer_survive_a_real_socket() {
        use rivet_core::operation::service_hold::{Doing, Hold, Kind};
        for kind in [Kind::Stop, Kind::Start] {
            for apply in [true, false] {
                let dir = std::env::temp_dir().join(format!(
                    "rivet-hold-{}-{:?}-{kind}-{apply}",
                    std::process::id(),
                    std::thread::current().id()
                ));
                let _ = std::fs::create_dir_all(&dir);
                let path = dir.join("query").to_string_lossy().into_owned();
                let _ = std::fs::remove_file(&path);
                let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind");
                let expected = Hold {
                    kind,
                    service: "photo-index".to_string(),
                    was: "started".to_string(),
                    doing: Doing::Stop,
                };
                let reply = expected.render();
                let server = std::thread::spawn(move || {
                    let (mut socket, _) = listener.accept().expect("accept");
                    let mut asked = String::new();
                    socket.read_to_string(&mut asked).expect("read the request");
                    socket.write_all(reply.as_bytes()).expect("write the reply");
                    Request::parse(&asked).expect("a request this build knows")
                });
                assert_eq!(hold_at(&path, kind, "photo-index", apply), Ok(expected));
                assert_eq!(
                    server.join().expect("the server thread"),
                    Request::ServiceHold {
                        kind,
                        service: "photo-index".to_string(),
                        apply,
                    }
                );
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    /// **A clock-floor request and its answer survive a real socket**, with
    /// the supervisor's own parser on the far end — `ADR-021` §3.
    #[test]
    fn a_clock_floor_request_and_its_answer_survive_a_real_socket() {
        use rivet_core::operation::clock_floor::Floor;
        for apply in [true, false] {
            let dir = std::env::temp_dir().join(format!(
                "rivet-floor-{}-{:?}-{apply}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join("query").to_string_lossy().into_owned();
            let _ = std::fs::remove_file(&path);
            let listener = std::os::unix::net::UnixListener::bind(&path).expect("bind");
            let expected = Floor {
                was_ms: Some(7_258_118_400_000),
                now_ms: 1_790_000_000_000,
            };
            let reply = expected.render();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().expect("accept");
                let mut asked = String::new();
                socket.read_to_string(&mut asked).expect("read the request");
                socket.write_all(reply.as_bytes()).expect("write the reply");
                Request::parse(&asked).expect("a request this build knows")
            });
            assert_eq!(clock_floor_at(&path, apply), Ok(expected));
            assert_eq!(
                server.join().expect("the server thread"),
                Request::ClockFloorReset { apply }
            );
            let _ = std::fs::remove_file(&path);
        }
    }

    /// **A refusal arrives as the domain's sentence**, and the control plane's
    /// refusal is `Unsafe`.
    #[test]
    fn a_hold_refusal_arrives_as_the_domains_sentence() {
        use rivet_core::operation::service_hold::{Kind, Refusal};
        let refusal = Refusal::CarriesTheWayIn {
            service: "rivet-mcpd".to_string(),
        };
        let (path, server) = supervisor_saying(refusal.render());
        let got = hold_at(&path, Kind::Stop, "rivet-mcpd", true).expect_err("refused");
        assert!(matches!(got, MachineError::Unsafe(_)), "{got:?}");
        assert!(got.why().contains("REACHABLE"), "{}", got.why());
        let _ = server.join();
        let _ = std::fs::remove_file(&path);
    }

    /// An answer about a different service or act is not taken as this one's.
    #[test]
    fn a_hold_answer_about_something_else_asserts_nothing() {
        use rivet_core::operation::service_hold::{Doing, Hold, Kind};
        let other = Hold {
            kind: Kind::Start,
            service: "photo-index".to_string(),
            was: "idle".to_string(),
            doing: Doing::Start,
        };
        let (path, server) = supervisor_saying(other.render());
        let got = hold_at(&path, Kind::Stop, "photo-index", true).expect_err("mismatch");
        assert!(got.why().contains("NOTHING IS ASSERTED"), "{}", got.why());
        let _ = server.join();
        let _ = std::fs::remove_file(&path);
    }

    /// A supervisor that is not there refuses here too, and the refusal is the
    /// transport's — the same sentence `get status` gets, because it is the
    /// same absence.
    #[test]
    fn a_restart_asked_of_a_supervisor_that_is_not_there_says_so() {
        let missing = std::env::temp_dir()
            .join("rivet-restart-nothing-here/query")
            .to_string_lossy()
            .into_owned();
        let refusal = restart_at(&missing, "photo-index", true).expect_err("nothing is listening");
        assert!(refusal.why().contains("not answering"), "{}", refusal.why());
    }

    /// An answer this build cannot read is refused, and the refusal says
    /// **nothing is asserted** — because the request may well have been taken.
    /// That is the honest state and it is different from a refusal.
    #[test]
    fn an_unreadable_restart_answer_asserts_nothing_about_what_happened() {
        let (path, server) = supervisor_saying("rebuilt\tphoto-index\n".to_string());
        let refusal = restart_at(&path, "photo-index", true).expect_err("an unknown line");
        let why = refusal.why();
        assert!(why.contains("NOTHING IS ASSERTED"), "{why}");
        assert!(why.contains("rivet get status"), "{why}");
        let _ = server.join();
        let _ = std::fs::remove_file(&path);
    }

    /// **An over-long answer is refused rather than cut.** A status missing the
    /// services at the end of it looks exactly like a complete one, which is the
    /// failure this bound exists to prevent rather than a resource limit.
    #[test]
    fn an_answer_longer_than_the_bound_is_refused_rather_than_truncated() {
        let mut huge = String::from("phase\tconverging\n");
        while huge.len() <= RESPONSE_MAX_BYTES {
            huge.push_str("svc\tfiller\ttrue\tidle\t-\t0\tfalse\n");
        }
        let (path, server) = supervisor_saying(huge);
        let refusal = ask_at(&path).expect_err("an over-long answer must refuse");
        assert!(
            refusal.why().contains("REFUSED rather than cut"),
            "{}",
            refusal.why()
        );
        let _ = server.join();
        let _ = std::fs::remove_file(&path);
    }
}
