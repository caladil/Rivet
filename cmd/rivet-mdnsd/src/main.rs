//! `rivet-mdnsd` — **so a machine is reachable by name.**
//!
//! ```text
//! rivet-mdnsd --projection /state/app/rivet-mdnsd/mdns
//! ```
//!
//! It announces `<hostname>.local` and nothing else: an `A` record, the
//! machine's own name as the floor. Services are not announced here.
//!
//! # What it runs as, and what it is handed
//!
//! Its own uid, under the image-service Landlock ruleset (`ADR-017` §4). It
//! may read neither the machine document nor the DHCP leases, so the
//! supervisor hands it a projection of exactly what it announces from —
//! `rivet_core::mdns` — and it takes the machine's name from the kernel by
//! `uname(2)`. `crate::announce` carries why the name comes from the kernel.
//!
//! # The order of a start
//!
//! 1. **Its socket, before anything else**, on this thread:
//!    `rivet_adapter_net::mdns::open` binds UDP 5353 and joins the group. A
//!    socket it cannot have is a refusal naming the step and the errno, and a
//!    non-zero exit the supervisor reports, never a responder that is
//!    `started` and answers nothing. The one exception is a moment: a join
//!    refused because no interface is up yet, which early in a boot is
//!    ordinary, is tried again for up to [`INTERFACE_WAIT_SECONDS`].
//! 2. **The name and the projection**, waiting up to [`LEASE_WAIT_SECONDS`]
//!    for a DHCP interface's first lease, which the supervisor writes into the
//!    projection when it arrives. That first lease restarts nothing: this
//!    process is already watching for it (`ADR-017` §3).
//! 3. **Answering, on the same thread**, for as long as it runs. A receive or
//!    a send that fails is a non-zero exit naming which, so the restart policy
//!    sees it.
//!
//! # Reach is not this program's decision
//!
//! UDP 5353 is under `ADR-023` §4's operator decision like every other port.
//! The service starts disabled, so an operator who turns it on has made the
//! reach decision by hand.
//!
//! # What is not built
//!
//! * **Services.** Announcing an SMB share or a git endpoint would make this
//!   the first service whose behaviour is a function of what else is
//!   installed.
//! * **Withdrawal.** A service record must stop being announced the moment its
//!   service stops; a host record's announcement ends when the machine stops
//!   answering. That matters on the day services are built.
//! * **IPv6.** One `A` record. `AAAA` is not refused on principle; the
//!   configuration has no IPv6 addressing to read it from.

mod announce;

use std::net::{SocketAddr, UdpSocket};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use rivet_core::mdns::{ParseError, Projection};

use crate::announce::{Announcement, Silent, TTL_SECONDS};

/// What this program was told.
#[derive(Debug)]
struct Args {
    projection: String,
}

/// The exit status for a refusal.
///
/// One, and there is no second code: this program either announces or does not,
/// and `ADR-017`'s supervisor is what decides whether that is worth restarting
/// for.
const REFUSED: u8 = 1;

/// How long to wait for a DHCP lease before giving up on it.
///
/// Thirty seconds: a lease on the maintainer's network arrives in under two,
/// and thirty covers a slow server without becoming a service that is silently
/// up for a minute.
const LEASE_WAIT_SECONDS: u64 = 30;

/// How often to look at the projection while waiting. Frequent enough that a
/// normal boot loses nothing to polling, and cheap: one small file.
const LEASE_POLL_MILLIS: u64 = 250;

/// The largest datagram read. An mDNS message may be up to 9000 bytes
/// (RFC 6762 §17).
const DATAGRAM: usize = 9000;

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1).collect::<Vec<_>>().as_slice()) {
        Ok(args) => args,
        Err(message) => return refuse(&message),
    };

    let socket = match open_waiting(
        &rivet_adapter_net::mdns::open,
        Duration::from_secs(INTERFACE_WAIT_SECONDS),
        Duration::from_millis(LEASE_POLL_MILLIS),
        &|failure| {
            println!(
                "rivet-mdnsd: waiting up to {INTERFACE_WAIT_SECONDS}s for an interface that can \
                 carry the multicast group: {failure}. rivet-netd brings the interfaces up, and \
                 this responder is simply early"
            );
        },
    ) {
        Ok(socket) => socket,
        Err(message) => return refuse(&message),
    };

    let announcement = match wait_for_announcement(
        &|| rivet_adapter_net::mdns::kernel_name().map_err(|e| format!("uname(2) failed: {e}")),
        &|| {
            std::fs::read_to_string(&args.projection).map_err(|e| {
                format!(
                    "cannot read {}: {e}. The supervisor writes it into this service's home \
                     before starting it, and it is this responder's only source for what to \
                     announce",
                    args.projection
                )
            })
        },
        Duration::from_secs(LEASE_WAIT_SECONDS),
        Duration::from_millis(LEASE_POLL_MILLIS),
        &|interfaces| {
            println!(
                "rivet-mdnsd: waiting up to {LEASE_WAIT_SECONDS}s for a DHCP lease on {} before \
                 announcing anything. rivet-netd runs udhcpc, and the supervisor writes the \
                 lease into this responder's projection; this responder is simply early",
                interfaces.join(", ")
            );
        },
    ) {
        Ok(announcement) => announcement,
        Err(message) => return refuse(&message),
    };

    println!(
        "rivet-mdnsd: announcing {} -> {} (ttl {TTL_SECONDS}s)",
        announcement.fqdn(),
        announcement.address
    );
    refuse(&serve(&socket, &announcement))
}

/// Read the argument vector, or say what was wrong with it.
///
/// Hand-parsed rather than `clap`ped, as every on-device binary here is.
fn parse(words: &[String]) -> Result<Args, String> {
    let mut projection = None;
    let mut rest = words.iter();
    while let Some(word) = rest.next() {
        match word.as_str() {
            "--projection" => {
                projection = Some(
                    rest.next()
                        .ok_or("--projection takes a path and none followed it")?
                        .clone(),
                );
            }
            other => {
                return Err(format!(
                    "`{other}` is not an option this program takes. usage: rivet-mdnsd \
                     --projection <path>"
                ));
            }
        }
    }
    Ok(Args {
        projection: projection.ok_or(
            "--projection is required: it is where the address to announce is read from. \
             usage: rivet-mdnsd --projection <path>",
        )?,
    })
}

/// How long to wait for an interface the multicast group can be joined on.
///
/// [`LEASE_WAIT_SECONDS`]' bound, for its reason: the interface that is not
/// up yet is the one the lease is for, so the two waits cover one moment.
const INTERFACE_WAIT_SECONDS: u64 = LEASE_WAIT_SECONDS;

/// **Open the socket, waiting through the one failure that is a moment**: a
/// join refused because no interface is up and routed yet
/// ([`rivet_adapter_net::mdns::OpenFailure::is_waiting_for_an_interface`]).
/// A responder started early in a boot meets exactly that, and spending the
/// supervisor's restart budget on it would mark the service failed seconds
/// before it could have answered. Bounded by `wait`; every other failure, and
/// that one once `wait` has passed, is refused at once with its step and
/// errno. `waiting` is told once.
fn open_waiting<T>(
    open: &dyn Fn() -> Result<T, rivet_adapter_net::mdns::OpenFailure>,
    wait: Duration,
    poll: Duration,
    waiting: &dyn Fn(&rivet_adapter_net::mdns::OpenFailure),
) -> Result<T, String> {
    let deadline = Instant::now() + wait;
    let mut said = false;
    loop {
        match open() {
            Ok(socket) => return Ok(socket),
            Err(failure) if failure.is_waiting_for_an_interface() && Instant::now() < deadline => {
                if !said {
                    waiting(&failure);
                    said = true;
                }
                std::thread::sleep(poll);
            }
            Err(failure) if failure.is_waiting_for_an_interface() => {
                return Err(format!(
                    "cannot open its socket, so it answers nothing: {failure}, still, after {}s \
                     waiting for an interface to come up. Nothing has been announced.",
                    wait.as_secs()
                ));
            }
            Err(failure) => {
                return Err(format!(
                    "cannot open its socket, so it answers nothing: {failure}. Nothing has been \
                     announced."
                ));
            }
        }
    }
}

/// Why [`wait_for_announcement`] is still waiting.
enum Waiting {
    /// The projection is being rewritten: it does not end with its terminator.
    Incomplete,
    /// Every addressed interface is DHCP and none has a lease yet.
    Lease(Vec<String>),
}

/// **Decide, waiting through the two moments that are not states**: a
/// projection caught mid-rewrite, and a DHCP interface with no lease yet.
///
/// Both are re-read every `poll` until `wait` has passed, and the name with
/// them, so a lease the supervisor writes into the projection is announced
/// without a restart. Everything else — a projection that cannot be read or
/// holds a line the format does not, a name the kernel will not give, no
/// address, two addresses, two names — is a fact about the machine that waiting
/// does not change, and is refused at once.
///
/// `waiting` is told once, the first time a lease is waited for: a responder
/// that filled the journal while waiting would bury what it is waiting for.
fn wait_for_announcement(
    kernel_name: &dyn Fn() -> Result<String, String>,
    projection: &dyn Fn() -> Result<String, String>,
    wait: Duration,
    poll: Duration,
    waiting: &dyn Fn(&[String]),
) -> Result<Announcement, String> {
    let deadline = Instant::now() + wait;
    let mut said = false;
    loop {
        let name = kernel_name()?;
        let text = projection()?;
        let now = match Projection::parse(&text) {
            Err(ParseError::Incomplete) => Waiting::Incomplete,
            Err(malformed) => return Err(format!("the projection is refused: {malformed}")),
            Ok(projection) => match announce::decide(&name, &projection) {
                Ok(announcement) => return Ok(announcement),
                Err(Silent::DhcpNoLease { interfaces }) => Waiting::Lease(interfaces),
                Err(other) => return Err(other.to_string()),
            },
        };
        if Instant::now() >= deadline {
            return Err(match now {
                Waiting::Incomplete => format!(
                    "{} for {}s, so it is not being rewritten but was left cut short. Nothing \
                     has been announced.",
                    ParseError::Incomplete,
                    wait.as_secs()
                ),
                Waiting::Lease(interfaces) => Silent::DhcpNoLease { interfaces }.to_string(),
            });
        }
        if let Waiting::Lease(interfaces) = &now
            && !said
        {
            waiting(interfaces);
            said = true;
        }
        std::thread::sleep(poll);
    }
}

/// **Answer queries for ever**, on this thread. Returns only when the socket
/// fails, with what failed, for the caller to refuse with.
fn serve(socket: &UdpSocket, announcement: &Announcement) -> String {
    let group = SocketAddr::from((
        rivet_adapter_net::mdns::GROUP,
        rivet_adapter_net::mdns::PORT,
    ));
    let mut buffer = vec![0u8; DATAGRAM];
    loop {
        let (count, from) = match socket.recv_from(&mut buffer) {
            Ok(received) => received,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return socket_failure("receiving a query on UDP 5353", &e),
        };
        let Some(datagram) = buffer.get(..count) else {
            continue;
        };
        let Some((answer, unicast)) = announce::reply(datagram, announcement) else {
            continue;
        };
        let to = if unicast { from } else { group };
        if let Err(e) = socket.send_to(&answer, to) {
            return socket_failure(&format!("sending an answer to {to}"), &e);
        }
    }
}

fn socket_failure(step: &str, error: &std::io::Error) -> String {
    match error.raw_os_error() {
        Some(errno) => format!(
            "{step} failed with errno {errno}: {error}. It has stopped answering, and exits so \
             the supervisor restarts it"
        ),
        None => format!(
            "{step} failed: {error}. It has stopped answering, and exits so the supervisor \
             restarts it"
        ),
    }
}

fn refuse(message: &str) -> ExitCode {
    eprintln!("rivet-mdnsd: {message}");
    ExitCode::from(REFUSED)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use std::cell::{Cell, RefCell};
    use std::net::Ipv4Addr;

    use rivet_core::hostname::Hostname;

    use super::*;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|w| (*w).to_string()).collect()
    }

    #[test]
    fn the_projection_path_is_required_and_the_refusal_says_what_it_is_for() {
        let error = parse(&[]).expect_err("--projection is required");
        assert!(error.contains("--projection"), "{error}");
        assert!(error.contains("address to announce"), "{error}");
    }

    #[test]
    fn a_path_is_taken_as_given_and_never_parsed() {
        let args = parse(&words(&["--projection", "/state/app/rivet-mdnsd/mdns"])).expect("valid");
        assert_eq!(args.projection, "/state/app/rivet-mdnsd/mdns");
    }

    /// `--hostname` specifically: it is the obvious thing to reach for, and
    /// `crate::announce` explains why the name is NOT passed here. And
    /// `--document`: the machine document is not this program's to read.
    #[test]
    fn an_option_this_program_does_not_take_is_refused_by_name() {
        for option in ["--hostname", "--document"] {
            let error = parse(&words(&[option, "x"])).expect_err("not an option");
            assert!(error.contains(option), "{error}");
            assert!(error.contains("usage"), "{error}");
        }
    }

    #[test]
    fn an_option_with_no_value_is_refused_rather_than_defaulted() {
        let error = parse(&words(&["--projection"])).expect_err("no value");
        assert!(error.contains("none followed it"), "{error}");
    }

    // ---- waiting on the projection -----------------------------------------

    fn text(addressed: &[(&str, [u8; 4])], leaseless: &[&str]) -> String {
        Projection {
            hostname: Hostname::new("kitchen-pi"),
            addressed: addressed
                .iter()
                .map(|(n, a)| ((*n).to_string(), Ipv4Addr::from(*a)))
                .collect(),
            leaseless: leaseless.iter().map(|n| (*n).to_string()).collect(),
        }
        .render()
    }

    /// Runs the wait over a sequence of projections, one per read, the last
    /// repeated. Returns the outcome, how many reads it took, and how many
    /// times it said it was waiting.
    fn run(reads: &[String], wait: Duration) -> (Result<Announcement, String>, usize, usize) {
        let at = Cell::new(0usize);
        let said = RefCell::new(0usize);
        let outcome = wait_for_announcement(
            &|| Ok("kitchen-pi".to_string()),
            &|| {
                let i = at.get();
                at.set(i + 1);
                Ok(reads[i.min(reads.len() - 1)].clone())
            },
            wait,
            Duration::from_millis(1),
            &|_| *said.borrow_mut() += 1,
        );
        (outcome, at.get(), said.into_inner())
    }

    /// **The first lease is picked up from the projection, without a
    /// restart.** The supervisor leaves a responder that started before its
    /// interface's first lease alone (`ADR-017` §3); this is the other half,
    /// the responder reading that lease when the supervisor writes it.
    #[test]
    fn a_lease_written_into_the_projection_while_waiting_is_announced() {
        let reads = [text(&[], &["eth0"]), text(&[], &["eth0"]), text(&[("eth0", [192, 168, 50, 66])], &[])];
        let (outcome, reads_taken, said) = run(&reads, Duration::from_secs(5));
        assert_eq!(outcome.expect("announced").address, Ipv4Addr::new(192, 168, 50, 66));
        assert_eq!(reads_taken, 3);
        assert_eq!(said, 1, "said once, not every poll");
    }

    /// A projection caught mid-rewrite is waited through, never read short.
    #[test]
    fn a_projection_caught_mid_rewrite_is_read_again() {
        let full = text(&[("eth0", [10, 0, 0, 2])], &[]);
        let cut = full[..full.len() - 4].to_string();
        let (outcome, reads_taken, said) = run(&[String::new(), cut, full], Duration::from_secs(5));
        assert_eq!(outcome.expect("announced").address, Ipv4Addr::new(10, 0, 0, 2));
        assert_eq!(reads_taken, 3);
        assert_eq!(said, 0, "a rewrite is not a lease being waited for");
    }

    /// Bounded: a lease that never comes is the refusal it always was.
    #[test]
    fn a_lease_that_never_comes_is_refused_when_the_wait_is_over() {
        let (outcome, _, _) = run(&[text(&[], &["eth0"])], Duration::from_millis(20));
        let error = outcome.expect_err("no lease");
        assert!(error.contains("none of them has a lease yet"), "{error}");
    }

    /// Facts that waiting does not change are refused on the first read.
    #[test]
    fn a_refusal_that_is_a_state_is_not_waited_for() {
        let two = text(&[("eth0", [10, 0, 0, 2]), ("wlan0", [10, 0, 1, 2])], &[]);
        let (outcome, reads_taken, _) = run(&[two], Duration::from_secs(5));
        assert!(outcome.expect_err("ambiguous").contains("will not"));
        assert_eq!(reads_taken, 1);

        let (outcome, reads_taken, _) = run(&["gateway=10.0.0.1\nend\n".to_string()], Duration::from_secs(5));
        assert!(outcome.expect_err("malformed").contains("refused"));
        assert_eq!(reads_taken, 1);
    }

    /// A projection that cannot be read at all is said, with the path.
    #[test]
    fn a_projection_that_cannot_be_read_is_refused_at_once() {
        let outcome = wait_for_announcement(
            &|| Ok("kitchen-pi".to_string()),
            &|| Err("cannot read /state/app/rivet-mdnsd/mdns: No such file".to_string()),
            Duration::from_secs(5),
            Duration::from_millis(1),
            &|_| {},
        );
        assert!(outcome.expect_err("unreadable").contains("/state/app/rivet-mdnsd/mdns"));
    }

    // ---- opening the socket -------------------------------------------------

    use rivet_adapter_net::mdns::{OpenFailure, Step};

    fn failing(step: Step, errno: i32) -> OpenFailure {
        OpenFailure {
            step,
            error: std::io::Error::from_raw_os_error(errno),
        }
    }

    /// **The boot dev-01 showed**: the responder starts before its interface
    /// is up, the join is refused with `ENODEV`, and the open is tried again
    /// rather than the restart budget being spent on a moment.
    #[test]
    fn a_join_refused_for_want_of_an_interface_is_tried_again() {
        let tries = Cell::new(0);
        let said = Cell::new(0);
        let opened = open_waiting(
            &|| {
                tries.set(tries.get() + 1);
                if tries.get() < 3 { Err(failing(Step::Join, 19)) } else { Ok("socket") }
            },
            Duration::from_secs(5),
            Duration::from_millis(1),
            &|_| said.set(said.get() + 1),
        );
        assert_eq!(opened, Ok("socket"));
        assert_eq!(tries.get(), 3);
        assert_eq!(said.get(), 1, "said once, not every try");
    }

    /// Bounded, and the refusal says it waited, with the step and the errno.
    #[test]
    fn an_interface_that_never_comes_is_refused_when_the_wait_is_over() {
        let error = open_waiting::<()>(
            &|| Err(failing(Step::Join, 19)),
            Duration::from_millis(20),
            Duration::from_millis(1),
            &|_| {},
        )
        .expect_err("no interface");
        assert!(error.contains("joining the multicast group"), "{error}");
        assert!(error.contains("errno 19"), "{error}");
        assert!(error.contains("waiting for an interface"), "{error}");
    }

    /// Any other failure is a state, refused on the first try.
    #[test]
    fn any_other_socket_failure_is_refused_at_once() {
        let tries = Cell::new(0);
        let error = open_waiting::<()>(
            &|| {
                tries.set(tries.get() + 1);
                Err(failing(Step::Bind, 98))
            },
            Duration::from_secs(5),
            Duration::from_millis(1),
            &|_| {},
        )
        .expect_err("address in use");
        assert_eq!(tries.get(), 1);
        assert!(error.contains("binding UDP 0.0.0.0:5353") && error.contains("errno 98"), "{error}");
    }

    #[test]
    fn a_socket_failure_names_the_step_and_the_errno() {
        let said = socket_failure("receiving a query on UDP 5353", &std::io::Error::from_raw_os_error(9));
        assert!(said.contains("receiving a query on UDP 5353"), "{said}");
        assert!(said.contains("errno 9"), "{said}");
    }
}
