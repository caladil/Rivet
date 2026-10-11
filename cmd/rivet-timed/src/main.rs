//! `rivet-timed` — **the wall clock, corrected against something outside this
//! machine.**
//!
//! `ADR-021` §2 splits the time problem in two and this binary is the second
//! half:
//!
//! | Problem | Whose |
//! | :-- | :-- |
//! | the clock is wrong at boot — it is 1970 | `ADR-021` §3's persist-and-restore: `rivet_store::clock` decides, `cmd/rivet-sysd/src/clock.rs` performs |
//! | keeping the clock accurate once running | **this binary**, `ADR-021` §4 |
//!
//! The first gives a *floor* and never correct time. This gives correct time, to
//! the accuracy `ADR-021` §2 asks for — *"seconds on a home network, not the
//! microsecond discipline a time server needs"*.
//!
//! # The shape of this crate
//!
//! | Module | Holds |
//! | :-- | :-- |
//! | [`args`] | what `argv` asked for, and **why a hostname is refused** |
//! | [`discipline`] | `ADR-021` §4.2.1's ruling as a pure decision, and the two numbers it delegates |
//! | [`journal`] | `ADR-022` §3.2's record, in the one form a separate binary can reach it |
//! | [`kernel`] | the clock, the console and the sleep, behind a port |
//! | [`sntp`] | one UDP round trip, and the wall around `rsntp` |
//! | [`run`] | the loop that wires the four together. No decisions of its own |
//! | `main` | the composition root: parse, construct, run, exit. No logic |
//!
//! Everything that decides anything is in [`discipline`], which touches nothing
//! and is exhaustively tested. Everything that touches the world is a port with
//! a fake behind it (`testkernel`). This file is the part that cannot be
//! tested without a kernel and a network, which is why there is so little of it.
//!
//! # It is a service — `ADR-021` §7
//!
//! > *"It is a service under `ADR-017` §2 — supervised, in the desired-state
//! > document, configurable (servers, poll interval, enabled). It is **not**
//! > part of `rivet-netd`. It needs the network, but under `ADR-017` §3.2's
//! > convergence model that is not a dependency to declare: it retries until the
//! > network works, like everything else."*
//!
//! The definition is `cmd/rivet-sysd/src/definitions.rs`, per `ADR-017`
//! §4.1, and the retrying is [`run::RETRY_INTERVAL_MS`].
//!
//! # What it runs as
//!
//! Its own uid, under the image-service Landlock ruleset, holding
//! `CAP_SYS_TIME` and no other capability — the supervisor keeps that one
//! across the uid change as an ambient capability and reduces the bounding set
//! to it (`ADR-017` §3, `ADR-021` §7). It needs nothing else: its UDP socket
//! binds an ephemeral port, it resolves no names, it reads only the boot id,
//! and it writes only to its stdout. A step refused with `EPERM` is reported
//! at `error` and names the capability.
//!
//! Every test in this crate drives `testkernel::FakeKernel`, deliberately: a
//! test that really set the clock would set the clock of the machine running
//! the suite.

mod args;
mod discipline;
mod journal;
mod kernel;
mod run;
mod sntp;
#[cfg(test)]
mod testkernel;

/// `ADR-017` §2.1's name for this service. Also the cgroup name and the notify
/// socket name on the supervisor's side, and the `svc` column of every record
/// this binary writes (`ADR-022` §3.2).
pub const SELF_NAME: &str = "rivet-timed";

/// `ADR-022` §5's actor, composing with `ADR-012` §4's provenance. There is
/// exactly one actor here: this process, acting on its own timer. Nothing an
/// operator or an agent asked for reaches this binary.
pub const ACTOR: &str = "timed";

/// Exit codes. `ADR-017` §3.5 restarts a failed service with backoff, so these
/// are policy rather than decoration.
///
/// **The long-running case never uses them**: the loop does not return, and the
/// process ends when `SIGTERM` terminates it. They exist for `--once`, which is
/// the hand-run that turns *"it compiles"* into *"it ran"* (`AGENTS.md` §6).
mod exit {
    /// A server answered. What was decided about the offset is in the journal;
    /// a `within-floor` is as much a success as a step.
    pub const ANSWERED: i32 = 0;
    /// `--once`, and no server answered. **Not a fault** on a board whose
    /// network is not up — see `ADR-017` §3.2.
    pub const NO_ANSWER: i32 = 1;
    /// `argv` was not usable.
    pub const BAD_ARGUMENTS: i32 = 2;
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let options = match args::parse(&argv) {
        Ok(args::Request::Usage) => {
            print!("{}", args::usage());
            return;
        }
        Ok(args::Request::Poll(options)) => options,
        Err(why) => {
            // Before the journal exists, so it is a plain line on stderr rather
            // than a record. A record would need a boot id and a wall clock,
            // and this failure happens before either is worth reading.
            eprintln!("{SELF_NAME}: {why}");
            eprint!("{}", args::usage());
            std::process::exit(exit::BAD_ARGUMENTS);
        }
    };

    let mut k = kernel::LibcKernel::new(options.console);
    let mut client = sntp::RsntpClient::new();

    // `ADR-022` §3.2's `boot`. A machine whose boot id cannot be read still has
    // a usable journal — ordering within a run is `mono`, which is always
    // readable — so this degrades to a marker rather than refusing to start.
    // The marker is a word rather than an empty string so that a reader meeting
    // it knows it is an absence rather than a truncation.
    let boot = {
        use kernel::Kernel as _;
        k.read_file(kernel::BOOT_ID_PATH)
            .map(|text| text.trim().to_string())
            .unwrap_or_else(|_| "unknown".to_string())
    };

    // The salt separates correlation identifiers minted by different runs of
    // this binary within one boot, and `ADR-017` §3.5 makes several runs the
    // expected case rather than an unusual one. The pid is what distinguishes
    // them and costs nothing.
    let salt = std::process::id();

    let mut journal = journal::Journal::new(boot, salt, ACTOR.to_string());
    let mut timed = run::Timed::new(&options);

    let max_polls = if options.once { Some(1) } else { None };
    let outcome = timed.run(&mut k, &mut client, &mut journal, max_polls);

    std::process::exit(match outcome {
        run::PollOutcome::Answered(_) | run::PollOutcome::NoWallClock { .. } => exit::ANSWERED,
        run::PollOutcome::NoServerAnswered => exit::NO_ANSWER,
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// The name is a path component: the supervisor uses it for a cgroup
    /// directory and a notify socket, and `ServiceDef::validate` refuses a
    /// separator in it. Asserted here as well as there, because this is the
    /// crate that owns the name.
    #[test]
    fn the_service_name_is_usable_as_a_path_component_and_a_json_value() {
        assert!(!SELF_NAME.is_empty());
        assert!(!SELF_NAME.contains('/'));
        assert!(!SELF_NAME.contains('"'));
        assert_ne!(SELF_NAME, ".");
        assert_ne!(SELF_NAME, "..");
    }

    /// The three codes are distinct, and zero is the one that means a server
    /// answered — `ADR-017` §3.5 reads a non-zero exit as a failure to retry.
    #[test]
    fn the_exit_codes_are_distinct_and_zero_means_a_server_answered() {
        assert_eq!(exit::ANSWERED, 0);
        assert_ne!(exit::NO_ANSWER, exit::ANSWERED);
        assert_ne!(exit::BAD_ARGUMENTS, exit::NO_ANSWER);
    }
}
