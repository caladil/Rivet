//! What this binary was asked to do, parsed from `argv`.
//!
//! Parsing is a decision, so it lives here and not in `main`, and it takes a
//! slice rather than reading `std::env::args` — which is what makes every branch
//! below reachable from a test on any platform. `cmd/rivet-netd/src/args.rs`
//! is the precedent and the shape is deliberately the same.
//!
//! ```text
//! rivet-timed --server <ip[:port]> [--server <ip[:port]>]…
//!               [--poll-seconds <n>] [--once] [--text | --jsonl]
//! ```
//!
//! # `--server` takes an ADDRESS, and a hostname is refused
//!
//! **This is the design question this crate had to answer, and it is answered in
//! the parser rather than in a comment**, so that a hostname cannot be
//! configured into a machine where it would silently never work.
//!
//! `rsntp` accepts either — its `ToServerAddrs` implementation for `str` will
//! happily hand a hostname to `std::net::ToSocketAddrs` and let libc resolve it.
//! Rivet refuses that shape, and the reason is a fact about the image rather
//! than a preference:
//!
//! > `ADR-008` §5.1.1, read out of the `libc.a` Rivet actually links:
//! > **`/etc/resolv.conf` absent means musl uses `127.0.0.1`.** The literal is in
//! > `.rodata.__get_resolv_conf.str1.8`.
//!
//! And `/etc/resolv.conf` **is** absent: `rivet_core::etc` lists it in
//! `MUSL_ETC_PATHS` and deliberately does not render it — *"leave out — DHCP's,
//! `ADR-023`"* — with a test (`the_projection_writes_none_of_the_files_it_is_not_the_authority_for`)
//! that keeps it absent. Nothing on a Rivet board today listens on port 53 of
//! `127.0.0.1`. **So a hostname resolves to nothing, on every Rivet machine
//! that exists**, and a `rivet-timed` configured with one would be a service
//! that fails identically for ever while the network around it is perfectly
//! healthy.
//!
//! ## What configuring hostnames would have bought, and why it loses
//!
//! It is the better answer against the NTP **pool**, whose whole design is that
//! `2.pool.ntp.org` resolves to a rotating set of nearby volunteers. Pinning
//! addresses out of the pool is explicitly against its operators' wishes and
//! goes stale. That is a real cost and it is accepted, by not using the pool:
//! [`crate::run`]'s documentation records which addresses the service definition
//! ships instead and why anycast is the shape that survives being pinned.
//!
//! The decisive asymmetry is that the two costs are not comparable. A pinned
//! address that goes stale fails **later**, visibly, on one server, with three
//! others still answering. A hostname fails **now**, on every server at once, on
//! every board, for a reason whose evidence is in a `.rodata` section of a
//! statically linked libc. `ADR-017` §3.2's convergence model absorbs the first
//! by retrying; nothing absorbs the second.
//!
//! ## When this is reopened
//!
//! The day something writes `/etc/resolv.conf` — which `ADR-008` §5.1.1 says is
//! the lease's to own, so `ADR-004` §8's DHCP client is the event. At that point
//! a hostname becomes a *convergence dependency* rather than a permanent
//! failure, which `ADR-017` §3.2 can absorb, and this parser is the one place
//! that has to change.
//!
//! # There is no default server, deliberately
//!
//! An empty `argv` is a refusal, not a fallback. A compiled-in default here
//! would be a second place that answers *which time servers does this machine
//! use* — `ADR-011` §3 is entirely about that — and the answer already lives in
//! the service definition (`cmd/rivet-sysd/src/definitions.rs`), which
//! is where `ADR-017` §4.1 puts *what a service is*. A human running this by
//! hand names a server; the supervisor names the same ones the image ships.

use std::net::SocketAddr;

use crate::kernel::ConsoleForm;

/// The IANA port for NTP, and SNTP is NTP's packet format
/// (`RFC 5905`). `rsntp` uses the same number internally; it is spelled here
/// because this parser applies it when `--server` carries no port, and a value
/// used in a decision belongs where the decision is.
pub const NTP_PORT: u16 = 123;

/// What this run was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Poll the given servers.
    Poll(Options),
    /// Print usage and stop.
    Usage,
}

/// The options of a poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// In the order they will be tried. At least one, always.
    pub servers: Vec<SocketAddr>,
    /// How long to wait between polls, on the **monotonic** clock.
    pub poll_interval_ms: u64,
    /// Poll once, act on it, and exit.
    ///
    /// **This exists so a human can prove the thing works.** `AGENTS.md` §6 —
    /// the shortest path from "it compiles" to "it ran" is
    /// `rivet-timed --once --server <a> --server <b>` on a board, which prints
    /// one outcome record and exits with a code saying whether a server
    /// answered. Two addresses, because one alone never steps the clock.
    pub once: bool,
    pub console: ConsoleForm,
}

/// Why `argv` was not usable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgError {
    /// A flag that takes a value was last on the line.
    MissingValue { flag: String },
    Unknown { arg: String },
    /// `--server` was given something that is not an address.
    NotAnAddress { value: String },
    /// No `--server` at all. See this module's documentation: there is no
    /// default, on purpose.
    NoServers,
    BadInterval { value: String },
}

impl core::fmt::Display for ArgError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MissingValue { flag } => write!(f, "{flag} needs a value after it"),
            Self::Unknown { arg } => write!(f, "{arg} is not an option this program takes"),
            Self::NotAnAddress { value } => write!(
                f,
                "--server takes an IP address, optionally with a port, and {value:?} is not one. \
                 A HOSTNAME IS NOT ACCEPTED: a Rivet image ships no /etc/resolv.conf, and musl \
                 with no resolv.conf sends every query to 127.0.0.1, where nothing listens \
                 (ADR-008 §5.1.1). A name would fail identically for ever on a healthy network"
            ),
            Self::NoServers => write!(
                f,
                "no --server was given and there is no default: which time servers a machine uses \
                 is stated in one place, and that place is the service definition (ADR-011, \
                 ADR-017 §4.1)"
            ),
            Self::BadInterval { value } => write!(
                f,
                "--poll-seconds takes a whole number of seconds greater than zero, and {value:?} \
                 is not one"
            ),
        }
    }
}

/// The usage text, which is also the documentation a board can print.
pub fn usage() -> String {
    format!(
        "\
{name} — Rivet's time client (ADR-021 §4, §7)

  --server <ip[:port]>   a time server, by ADDRESS. Repeatable; tried in order.
                         Port defaults to {NTP_PORT}. A hostname is refused: a
                         Rivet image ships no /etc/resolv.conf (ADR-008 §5.1.1).
                         The clock is stepped only when two different servers
                         agree, so one server alone never moves it (ADR-021 §3).
  --poll-seconds <n>     seconds between polls. Default {default}.
  --once                 poll once, act on it, and exit.
  --text | --jsonl       console form. Default jsonl (ADR-022 §3).
  --help                 this.

Exit codes with --once: 0 a server answered, 1 none did, 2 argv was not usable.
",
        name = crate::SELF_NAME,
        default = crate::run::POLL_INTERVAL_MS / 1000,
    )
}

/// Parse `argv` minus the program name.
pub fn parse(args: &[String]) -> Result<Request, ArgError> {
    let mut servers: Vec<SocketAddr> = Vec::new();
    let mut poll_interval_ms = crate::run::POLL_INTERVAL_MS;
    let mut once = false;
    let mut console = ConsoleForm::Jsonl;

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        let value = |i: &mut usize| -> Result<String, ArgError> {
            *i += 1;
            args.get(*i).cloned().ok_or(ArgError::MissingValue {
                flag: arg.to_string(),
            })
        };
        match arg {
            "--help" | "-h" => return Ok(Request::Usage),
            "--once" => once = true,
            "--text" => console = ConsoleForm::Text,
            "--jsonl" => console = ConsoleForm::Jsonl,
            "--server" => {
                let raw = value(&mut i)?;
                servers.push(parse_server(&raw)?);
            }
            "--poll-seconds" => {
                let raw = value(&mut i)?;
                let seconds: u64 = raw
                    .parse()
                    .ok()
                    .filter(|s| *s > 0)
                    .ok_or(ArgError::BadInterval { value: raw.clone() })?;
                poll_interval_ms = seconds.saturating_mul(1_000);
            }
            other => {
                return Err(ArgError::Unknown {
                    arg: other.to_string(),
                });
            }
        }
        i += 1;
    }

    if servers.is_empty() {
        return Err(ArgError::NoServers);
    }

    Ok(Request::Poll(Options {
        servers,
        poll_interval_ms,
        once,
        console,
    }))
}

/// One `--server` word: an address, with or without a port. **Never a name.**
///
/// Refusing rather than resolving is the whole of this module's opening
/// argument; the refusal is here so that the failure is a message on the console
/// at start-up rather than a timeout every poll for the life of the board.
fn parse_server(raw: &str) -> Result<SocketAddr, ArgError> {
    if let Ok(addr) = raw.parse::<SocketAddr>() {
        return Ok(addr);
    }
    if let Ok(ip) = raw.parse::<std::net::IpAddr>() {
        return Ok(SocketAddr::new(ip, NTP_PORT));
    }
    Err(ArgError::NotAnAddress {
        value: raw.to_string(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    fn poll(list: &[&str]) -> Options {
        match parse(&args(list)) {
            Ok(Request::Poll(o)) => o,
            other => unreachable!("{other:?}"),
        }
    }

    #[test]
    fn one_server_with_no_port_gets_the_ntp_port() {
        let o = poll(&["--server", "192.0.2.1"]);
        assert_eq!(
            o.servers,
            vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)), 123)]
        );
    }

    #[test]
    fn a_port_on_the_word_is_honoured() {
        let o = poll(&["--server", "192.0.2.1:1123"]);
        assert_eq!(o.servers[0].port(), 1123);
    }

    #[test]
    fn ipv6_works_bare_and_bracketed() {
        let bare = poll(&["--server", "2001:db8::1"]);
        assert_eq!(
            bare.servers[0],
            SocketAddr::new(IpAddr::V6("2001:db8::1".parse::<Ipv6Addr>().unwrap_or(Ipv6Addr::UNSPECIFIED)), 123)
        );
        let with_port = poll(&["--server", "[2001:db8::1]:1123"]);
        assert_eq!(with_port.servers[0].port(), 1123);
    }

    #[test]
    fn servers_are_kept_in_the_order_they_were_given() {
        let o = poll(&[
            "--server", "192.0.2.1", "--server", "192.0.2.2", "--server", "192.0.2.3",
        ]);
        let ports_and_ips: Vec<String> = o.servers.iter().map(|s| s.ip().to_string()).collect();
        assert_eq!(ports_and_ips, ["192.0.2.1", "192.0.2.2", "192.0.2.3"]);
    }

    /// **The decision this parser exists to enforce.** A hostname is refused,
    /// and the refusal says why rather than saying "not an address" — an
    /// operator typing `pool.ntp.org` has a correct model of NTP and a wrong
    /// model of this image.
    #[test]
    fn a_hostname_is_refused_and_the_refusal_names_the_reason() {
        for name in ["pool.ntp.org", "time.example", "localhost", "0.pool.ntp.org:123"] {
            let err = parse(&args(&["--server", name])).expect_err(name);
            assert_eq!(
                err,
                ArgError::NotAnAddress {
                    value: name.to_string()
                },
                "{name}"
            );
            let rendered = err.to_string();
            assert!(rendered.contains("resolv.conf"), "{rendered}");
            assert!(rendered.contains("127.0.0.1"), "{rendered}");
            assert!(rendered.contains("ADR-008"), "{rendered}");
        }
    }

    /// No default, and the refusal says where the answer does live.
    #[test]
    fn an_empty_argv_is_refused_rather_than_defaulted() {
        let err = parse(&[]).expect_err("an empty argv produced a server list");
        assert_eq!(err, ArgError::NoServers);
        assert!(err.to_string().contains("ADR-011"), "{err}");
    }

    #[test]
    fn a_flag_with_no_value_is_refused_by_name() {
        assert_eq!(
            parse(&args(&["--server"])),
            Err(ArgError::MissingValue {
                flag: "--server".to_string()
            })
        );
        assert_eq!(
            parse(&args(&["--server", "192.0.2.1", "--poll-seconds"])),
            Err(ArgError::MissingValue {
                flag: "--poll-seconds".to_string()
            })
        );
    }

    #[test]
    fn an_unknown_flag_is_refused_rather_than_ignored() {
        assert_eq!(
            parse(&args(&["--server", "192.0.2.1", "--slew"])),
            Err(ArgError::Unknown {
                arg: "--slew".to_string()
            }),
            "ADR-021 §4.2.1 — there is no slew, and silently ignoring the flag would let a \
             machine be configured as though there were"
        );
    }

    #[test]
    fn the_poll_interval_is_seconds_and_zero_is_refused() {
        assert_eq!(
            poll(&["--server", "192.0.2.1", "--poll-seconds", "60"]).poll_interval_ms,
            60_000
        );
        for bad in ["0", "-1", "", "1.5", "sixty"] {
            assert_eq!(
                parse(&args(&["--server", "192.0.2.1", "--poll-seconds", bad])),
                Err(ArgError::BadInterval {
                    value: bad.to_string()
                }),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_defaults_are_the_crates_own_constants() {
        let o = poll(&["--server", "192.0.2.1"]);
        assert_eq!(o.poll_interval_ms, crate::run::POLL_INTERVAL_MS);
        assert!(!o.once);
        assert_eq!(o.console, ConsoleForm::Jsonl, "ADR-022 §3 is the on-disk form");
    }

    #[test]
    fn once_and_the_console_form_are_taken_in_any_order() {
        for order in [
            vec!["--once", "--text", "--server", "192.0.2.1"],
            vec!["--server", "192.0.2.1", "--text", "--once"],
            vec!["--text", "--server", "192.0.2.1", "--once"],
        ] {
            let o = poll(&order);
            assert!(o.once, "{order:?}");
            assert_eq!(o.console, ConsoleForm::Text, "{order:?}");
        }
    }

    #[test]
    fn help_stops_before_anything_else_is_required() {
        assert_eq!(parse(&args(&["--help"])), Ok(Request::Usage));
        assert_eq!(parse(&args(&["-h"])), Ok(Request::Usage));
    }

    /// The usage text has to name the refusal, because it is the thing a person
    /// will get wrong.
    #[test]
    fn the_usage_text_says_a_hostname_will_not_work() {
        let text = usage();
        assert!(text.contains("resolv.conf"), "{text}");
        assert!(text.contains("ADDRESS"), "{text}");
        assert!(text.contains(&NTP_PORT.to_string()), "{text}");
    }
}
