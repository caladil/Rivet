//! The seam between this crate's decisions and a netlink socket.
//!
//! Everything below this line is a syscall; everything above it is a decision.
//! It is the same split `cmd/rivet-initd`'s `kernel` module makes, and for the
//! same reason: the branches that matter — an interface that is not there, an
//! address already assigned, a route the kernel says is unreachable, `EPERM`
//! from a process without `CAP_NET_ADMIN` — **cannot be reached from a test
//! that needs a kernel and a network**, and at Stage 1 they cannot be reached
//! from the emulator either. `PLAN.md` §2.2's exit criterion is reaching the
//! board over the network, so a failure here is a failure to reach the machine
//! that would tell you about it.
//!
//! **[`SendRecv`] implementations must stay thin, and should get thinner rather
//! than smarter.** Whatever logic lives in one is logic no test can drive. The
//! real one is [`crate::socket`], which holds every `unsafe` block in this
//! crate; the fake is `crate::testnetlink`.
//!
//! # Two deliberate differences from `cmd/rivet-initd`'s port
//!
//! Recorded because they look like inconsistency and are not.
//!
//! 1. **This port does not carry a `say`.** PID 1's `Kernel` reports to the
//!    console through the port, so its tests assert on strings. Here the
//!    decisions return typed outcomes ([`crate::wired`]) and `cmd/rivet-netd`
//!    renders them, which is what `ADR-006` §4 asks for — rendering is an
//!    adapter concern and the operation's result is a value. Tests then assert
//!    on the value rather than on prose that a reword would break.
//! 2. **[`Errno`] does not ask the host platform for its message.** PID 1's
//!    does, correctly: its errnos come from syscalls made by that same process
//!    on that same kernel. **These come from a Linux kernel over a socket, and
//!    the process reading them may have been compiled on Windows** — where
//!    `strerror(19)` is "Cross-device link", not "No such device". So the
//!    handful of numbers netlink actually returns are named here, and anything
//!    else is reported as a bare number rather than mislabelled.

use crate::wire::{self, Malformed};

/// A Linux error number, as the kernel sent it.
///
/// Positive, always: netlink puts a *negative* errno in `NLMSG_ERROR` and
/// [`Failure::from_netlink_error`] negates it once, here, so nothing above has
/// to remember the sign.
///
/// The constants are Linux's own, from `asm-generic/errno-base.h` and
/// `asm-generic/errno.h`, for the reason in this module's documentation. They
/// are checked against `libc` by a test that compiles on Linux only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Errno(pub i32);

impl Errno {
    pub const EPERM: Self = Self(1);
    /// The object named is not there. From nf_tables it also means an
    /// expression the kernel was not built with.
    pub const ENOENT: Self = Self(2);
    pub const ESRCH: Self = Self(3);
    pub const EINTR: Self = Self(4);
    pub const EIO: Self = Self(5);
    pub const EAGAIN: Self = Self(11);
    pub const ENOMEM: Self = Self(12);
    pub const EACCES: Self = Self(13);
    pub const EBUSY: Self = Self(16);
    /// The address or the route is already there. **The good case**, almost
    /// everywhere in this crate — see [`crate::wired`].
    pub const EEXIST: Self = Self(17);
    /// No such interface.
    pub const ENODEV: Self = Self(19);
    pub const EINVAL: Self = Self(22);
    pub const EPROTONOSUPPORT: Self = Self(93);
    pub const EOPNOTSUPP: Self = Self(95);
    pub const EAFNOSUPPORT: Self = Self(97);
    pub const EADDRNOTAVAIL: Self = Self(99);
    /// There is no route to the next hop. What a gateway outside the
    /// interface's subnet produces — which `rivet_core::net` refuses before
    /// the syscall, so seeing this means something else is wrong.
    pub const ENETUNREACH: Self = Self(101);
    pub const ENOBUFS: Self = Self(105);
    pub const ETIMEDOUT: Self = Self(110);

    /// The name and meaning, for the errnos this crate can actually receive.
    ///
    /// `None` for anything else, so the caller prints the number rather than a
    /// guess. `AGENTS.md` §5 — report what the system said, and a number is
    /// what the system said.
    fn described(self) -> Option<(&'static str, &'static str)> {
        Some(match self {
            Self::EPERM => ("EPERM", "operation not permitted"),
            Self::ENOENT => ("ENOENT", "the object named is not there"),
            Self::ESRCH => ("ESRCH", "no such process"),
            Self::EINTR => ("EINTR", "interrupted"),
            Self::EIO => ("EIO", "input/output error"),
            Self::EAGAIN => ("EAGAIN", "would block, or the receive timed out"),
            Self::ENOMEM => ("ENOMEM", "out of memory"),
            Self::EACCES => ("EACCES", "permission denied"),
            Self::EBUSY => ("EBUSY", "device or resource busy"),
            Self::EEXIST => ("EEXIST", "it is already there"),
            Self::ENODEV => ("ENODEV", "no such device"),
            Self::EINVAL => ("EINVAL", "invalid argument"),
            Self::EPROTONOSUPPORT => ("EPROTONOSUPPORT", "protocol not supported"),
            Self::EOPNOTSUPP => ("EOPNOTSUPP", "operation not supported"),
            Self::EAFNOSUPPORT => ("EAFNOSUPPORT", "address family not supported"),
            Self::EADDRNOTAVAIL => ("EADDRNOTAVAIL", "address not available"),
            Self::ENETUNREACH => ("ENETUNREACH", "network is unreachable"),
            Self::ENOBUFS => ("ENOBUFS", "no buffer space"),
            Self::ETIMEDOUT => ("ETIMEDOUT", "timed out"),
            _ => return None,
        })
    }
}

impl core::fmt::Display for Errno {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.described() {
            Some((name, meaning)) => write!(f, "{name} ({meaning}, errno {})", self.0),
            None => write!(f, "errno {}", self.0),
        }
    }
}

/// Everything that can go wrong between asking the kernel and believing it.
///
/// **Eight variants and they are genuinely different things**, which is why
/// this is not one `String`: [`Kernel`](Self::Kernel) is the kernel declining
/// a well-formed request and is often expected (`EEXIST`); the other seven all
/// mean the conversation itself went wrong and none of them should ever
/// happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// `send(2)` or `recv(2)` failed. The socket, not the request.
    Socket(Errno),
    /// The kernel refused the request and said why.
    Kernel(Errno),
    /// The reply did not parse.
    Malformed(Malformed),
    /// A reply to somebody else's request, or to an earlier one of ours.
    ///
    /// Refused rather than skipped. Skipping is the tempting handling and it is
    /// wrong: the answer to *our* request may be the one being skipped, and the
    /// caller would then act on a stale verdict about the state of the network.
    WrongSequence { expected: u32, got: u32 },
    /// A message type that is not an answer to what was asked.
    Unexpected { kind: u16 },
    /// `NLMSG_OVERRUN` — the kernel dropped messages because the socket's
    /// buffer filled. The reply is incomplete by the kernel's own admission,
    /// so there is nothing to salvage from it.
    Overrun,
    /// A dump ran past [`DUMP_DATAGRAM_LIMIT`] datagrams without the kernel
    /// sending `NLMSG_DONE`.
    ///
    /// **Refused rather than returned as what arrived**, for the reason every
    /// other variant here exists: a truncated dump is a *shorter list*, and a
    /// shorter list of addresses reads as a machine with fewer addresses. The
    /// caller cannot tell one from the other, so it is never handed one.
    Unterminated { datagrams: usize },
    /// A dump carried [`wire::NLM_F_DUMP_INTR`] on the resend [`dump`] already
    /// makes for exactly this reason.
    ///
    /// The kernel sets this bit — on a data message or on `NLMSG_DONE`
    /// itself — when something else changed the table being dumped while the
    /// dump was in flight, so what was collected is not a consistent
    /// snapshot. `dump` resends the request once on its own; refused rather
    /// than resent again without bound, since a table under constant change
    /// would otherwise keep this loop going forever.
    Interrupted,
}

impl Failure {
    /// Turn the `error` field of an `NLMSG_ERROR` into a [`Failure`].
    ///
    /// The sign is handled exactly once, here. `saturating_neg` rather than
    /// `-code`: `i32::MIN` has no positive counterpart and negating it would
    /// panic in debug and wrap in release, which is a poor way for a network
    /// bring-up to end.
    pub fn from_netlink_error(code: i32) -> Self {
        Self::Kernel(Errno(code.saturating_neg()))
    }

    /// The errno the kernel gave, if the failure came from the kernel at all.
    ///
    /// Callers use this to recognise the outcomes that are not failures —
    /// [`Errno::EEXIST`] above all.
    pub fn kernel_errno(&self) -> Option<Errno> {
        match self {
            Self::Kernel(errno) => Some(*errno),
            _ => None,
        }
    }
}

impl core::fmt::Display for Failure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Socket(errno) => write!(f, "the netlink socket failed: {errno}"),
            Self::Kernel(errno) => write!(f, "the kernel refused: {errno}"),
            Self::Malformed(why) => write!(f, "the kernel's reply did not parse: {why}"),
            Self::WrongSequence { expected, got } => write!(
                f,
                "a netlink reply carried sequence {got} where {expected} was asked for"
            ),
            Self::Unexpected { kind } => {
                write!(f, "the kernel answered with message type {kind}, which was not asked for")
            }
            Self::Overrun => write!(
                f,
                "the kernel reported NLMSG_OVERRUN — messages were dropped and the reply is incomplete"
            ),
            Self::Unterminated { datagrams } => write!(
                f,
                "a netlink dump ran to {datagrams} datagrams without the kernel sending NLMSG_DONE, so the answer is incomplete and is refused rather than reported short"
            ),
            Self::Interrupted => write!(
                f,
                "a netlink dump was interrupted by a concurrent change (NLM_F_DUMP_INTR), \
                 twice in a row including the resend, so it is refused rather than reported \
                 as a consistent snapshot"
            ),
        }
    }
}

impl From<Malformed> for Failure {
    fn from(why: Malformed) -> Self {
        Self::Malformed(why)
    }
}

/// A netlink socket, reduced to the two things this crate does with one.
///
/// Object-safe on purpose: the decisions take `&mut dyn SendRecv`, so there is
/// one compiled copy of them and the fake drives exactly the code the board
/// runs.
pub trait SendRecv {
    /// Send one already-encoded message to the kernel.
    fn send(&mut self, message: &[u8]) -> Result<(), Errno>;

    /// Receive whatever the kernel sends next.
    ///
    /// **Must not block for ever.** A netlink request that gets no answer would
    /// otherwise wedge the network bring-up with no console and no way in;
    /// [`crate::socket`] sets a receive timeout and reports
    /// [`Errno::EAGAIN`] when it expires.
    fn recv(&mut self) -> Result<Vec<u8>, Errno>;
}

/// Sequence numbers for one conversation with the kernel.
///
/// Netlink does not correlate replies for you — the sequence number is the only
/// thing that says an answer belongs to a question, which is why
/// [`Failure::WrongSequence`] exists rather than a `skip`.
#[derive(Debug, Clone)]
pub struct Sequence {
    next: u32,
}

impl Default for Sequence {
    fn default() -> Self {
        Self::new()
    }
}

impl Sequence {
    /// Starts at 1. **Zero is skipped deliberately**: the kernel uses sequence
    /// zero for unsolicited notifications, so a request numbered zero cannot be
    /// told apart from a broadcast.
    pub fn new() -> Self {
        Self { next: 1 }
    }

    pub fn take(&mut self) -> u32 {
        let seq = self.next;
        // On overflow, back to 1 rather than to 0 — see `new`.
        self.next = self.next.checked_add(1).unwrap_or(1);
        seq
    }
}

/// Send one request and collect the kernel's reply.
///
/// Returns the raw bytes rather than parsed messages, because a `MessageRef`
/// borrows the buffer it came from and a function cannot return both. The
/// caller parses with [`wire::messages`].
///
/// ~~**A dump — anything sent with `NLM_F_DUMP` — is not this function's job.**
/// It reads exactly one datagram. Stage 1 asks four questions, each answered in
/// one; adding multi-datagram reads before something needs them would be code
/// with no test that could fail.~~
///
/// **Still true of *this* function, and the second half expired 2026-08-31**
/// (`AGENTS.md` §2 rule 3 — struck rather than edited, because the condition it
/// named is the thing that changed). Something needs them: `get network` on a
/// board reported `"addresses":[]` for the interface it was reached over, and
/// the only way to an IPv4 address is `RTM_GETADDR`, which is a dump. The
/// multi-datagram read is [`dump`], beside this rather than inside it, and it
/// has tests that can fail.
pub fn transact(nl: &mut dyn SendRecv, message: Vec<u8>) -> Result<Vec<u8>, Failure> {
    nl.send(&message).map_err(Failure::Socket)?;
    nl.recv().map_err(Failure::Socket)
}

/// How many datagrams a dump may span before it is refused as unterminated.
///
/// **A guard against a kernel that never says `NLMSG_DONE`, not a budget.** The
/// receive timeout in [`crate::socket`] already covers a kernel that goes
/// *silent*; what it cannot cover is one that keeps talking, and the loop below
/// would then run for ever inside a process with no console. A `RECV_BUFFER` of
/// 32 KiB holds on the order of a hundred address messages, so a thousand
/// datagrams is around a hundred thousand addresses — orders of magnitude past
/// anything a Rivet has, and still bounded.
pub const DUMP_DATAGRAM_LIMIT: usize = 1024;

/// Send one dump request and collect every datagram of the answer.
///
/// # What a dump is, and why it cannot be [`transact`]
///
/// A request carrying `NLM_F_DUMP` is answered with a *sequence* of messages,
/// each flagged `NLM_F_MULTI`, spread over as many datagrams as the kernel
/// needs, and terminated by an `NLMSG_DONE`. `transact` reads exactly one
/// datagram, so pointing it at a dump returns whatever fit in the first — which
/// is **a prefix that parses**, the failure mode `AGENTS.md` §5 names: it does
/// not error, it succeeds with a short list.
///
/// # What this refuses, and why each refusal rather than a skip
///
/// * **A reply to another sequence.** Same reasoning as [`expect_ack`]: the
///   answer to *our* request may be the message being skipped.
/// * **`NLMSG_ERROR`.** The kernel declined the dump; the errno is the answer.
///   Note that unlike an ack, an error code of zero has no meaning here —
///   nothing asked for an acknowledgement — so it is reported as a refusal with
///   whatever the kernel put in the field rather than read as success.
/// * **`NLMSG_OVERRUN`.** The kernel's own admission that it dropped messages.
/// * **Running past [`DUMP_DATAGRAM_LIMIT`].**
/// * **`NLM_F_DUMP_INTR`, twice in a row.** The kernel's own admission that a
///   concurrent change means what was collected is not a consistent snapshot
///   — see [`Failure::Interrupted`]. The first occurrence is not refused: the
///   request is resent once, silently, before this function gives up.
/// * **A negative error packed into `NLMSG_DONE` itself**, which a kernel
///   recent enough to set `NLM_F_DUMP_INTR` also uses to report the dump
///   failing outright rather than merely being interrupted.
///
/// # What it returns
///
/// The datagrams, raw and in order, for the same reason [`transact`] returns
/// bytes: a `MessageRef` borrows the buffer it came from. `NLMSG_DONE` is
/// consumed here and does not reach the caller; everything before it does,
/// including message types this function does not recognise — a caller that
/// wants to report what arrived cannot do so from a filtered list.
pub fn dump(
    nl: &mut dyn SendRecv,
    message: Vec<u8>,
    expected_seq: u32,
) -> Result<Vec<Vec<u8>>, Failure> {
    // **Resent at most once.** A dump disturbed by one concurrent change is
    // ordinary enough to retry silently; a table changing on every attempt is
    // a reason to stop and say so rather than loop against it forever.
    match dump_attempt(nl, &message, expected_seq)? {
        DumpAttempt::Complete(datagrams) => return Ok(datagrams),
        DumpAttempt::Interrupted => {}
    }
    match dump_attempt(nl, &message, expected_seq)? {
        DumpAttempt::Complete(datagrams) => Ok(datagrams),
        DumpAttempt::Interrupted => Err(Failure::Interrupted),
    }
}

/// One send of a dump request and everything the kernel answered before
/// [`dump`] decides whether to trust it, resend it, or give up.
enum DumpAttempt {
    /// Reached `NLMSG_DONE` with `NLM_F_DUMP_INTR` on no message in the
    /// sequence, and no error packed into `NLMSG_DONE` itself.
    Complete(Vec<Vec<u8>>),
    /// `NLM_F_DUMP_INTR` was set on some message in the sequence — a data
    /// message or `NLMSG_DONE` itself, checked the same way either time.
    Interrupted,
}

/// One full send-and-collect cycle of [`dump`]'s protocol, without deciding
/// what an interruption means — that is [`dump`]'s call, since it is the one
/// that knows whether this is the first attempt or the retry.
fn dump_attempt(
    nl: &mut dyn SendRecv,
    message: &[u8],
    expected_seq: u32,
) -> Result<DumpAttempt, Failure> {
    nl.send(message).map_err(Failure::Socket)?;

    let mut datagrams: Vec<Vec<u8>> = Vec::new();
    let mut interrupted = false;
    loop {
        if datagrams.len() >= DUMP_DATAGRAM_LIMIT {
            return Err(Failure::Unterminated {
                datagrams: datagrams.len(),
            });
        }
        let datagram = nl.recv().map_err(Failure::Socket)?;
        let mut done = false;
        for message in wire::messages(&datagram)? {
            // **Checked on every message, not only `NLMSG_DONE`.** The kernel
            // may set this on whichever message it was mid-writing when the
            // change happened, and the terminator is not guaranteed to be it.
            if message.flags & wire::NLM_F_DUMP_INTR != 0 {
                interrupted = true;
            }
            match message.kind {
                wire::NLMSG_NOOP => continue,
                wire::NLMSG_OVERRUN => return Err(Failure::Overrun),
                _ if message.seq != expected_seq => {
                    return Err(Failure::WrongSequence {
                        expected: expected_seq,
                        got: message.seq,
                    });
                }
                wire::NLMSG_ERROR => {
                    return Err(Failure::from_netlink_error(wire::error_code(
                        message.payload,
                    )?));
                }
                // **Set and then kept reading the rest of this datagram.** The
                // kernel packs `NLMSG_DONE` behind the last batch of data, so
                // returning the moment it is seen would be correct only because
                // nothing after it is examined — and would be wrong the day a
                // caller wanted the sequence number off it.
                wire::NLMSG_DONE => {
                    if let Some(code) = wire::dump_done_error(message.payload)? {
                        return Err(Failure::from_netlink_error(code));
                    }
                    done = true;
                }
                _ => {}
            }
        }
        // The datagram is kept whether or not it carried the terminator: an
        // empty one is a real answer (a machine with no addresses) and dropping
        // it would make "none" and "the last datagram was only DONE" the same.
        datagrams.push(datagram);
        if done {
            return Ok(if interrupted {
                DumpAttempt::Interrupted
            } else {
                DumpAttempt::Complete(datagrams)
            });
        }
    }
}

/// Read an acknowledgement: the kernel's answer to a request that asked for
/// one and returns nothing else.
///
/// The three outcomes, and the middle one is the one that gets written wrong:
///
/// | Reply | Means |
/// | :-- | :-- |
/// | `NLMSG_ERROR`, error 0 | **success** — this is what an ack *is* |
/// | `NLMSG_ERROR`, error < 0 | the kernel refused, with an errno |
/// | anything else | not an answer to this request |
pub fn expect_ack(reply: &[u8], expected_seq: u32) -> Result<(), Failure> {
    let messages = wire::messages(reply)?;
    for message in messages {
        match message.kind {
            wire::NLMSG_NOOP => continue,
            wire::NLMSG_OVERRUN => return Err(Failure::Overrun),
            wire::NLMSG_ERROR => {
                if message.seq != expected_seq {
                    return Err(Failure::WrongSequence {
                        expected: expected_seq,
                        got: message.seq,
                    });
                }
                let code = wire::error_code(message.payload)?;
                return if code == 0 {
                    Ok(())
                } else {
                    Err(Failure::from_netlink_error(code))
                };
            }
            kind => return Err(Failure::Unexpected { kind }),
        }
    }
    // An empty reply is not an ack. Treating it as one is how a request that
    // never reached the kernel gets reported as applied.
    Err(Failure::Malformed(Malformed::ShortHeader { available: 0 }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::wire::Message;

    #[test]
    fn a_negative_netlink_error_becomes_a_positive_errno_exactly_once() {
        assert_eq!(
            Failure::from_netlink_error(-19),
            Failure::Kernel(Errno::ENODEV)
        );
        assert_eq!(
            Failure::from_netlink_error(-17).kernel_errno(),
            Some(Errno::EEXIST)
        );
    }

    /// `i32::MIN.neg()` panics in debug and wraps in release. Neither is an
    /// acceptable way for a network bring-up to end, so it saturates.
    #[test]
    fn the_most_negative_error_code_does_not_panic_or_wrap() {
        assert_eq!(
            Failure::from_netlink_error(i32::MIN),
            Failure::Kernel(Errno(i32::MAX))
        );
    }

    #[test]
    fn only_a_kernel_failure_has_an_errno() {
        assert_eq!(Failure::Socket(Errno::EPERM).kernel_errno(), None);
        assert_eq!(Failure::Overrun.kernel_errno(), None);
        assert_eq!(
            Failure::WrongSequence {
                expected: 1,
                got: 2
            }
            .kernel_errno(),
            None
        );
    }

    #[test]
    fn a_known_errno_is_named_and_an_unknown_one_is_a_bare_number() {
        assert!(Errno::ENODEV.to_string().contains("ENODEV"));
        assert!(Errno::ENODEV.to_string().contains("19"));
        assert_eq!(Errno(4242).to_string(), "errno 4242");
    }

    #[test]
    fn sequence_numbers_start_at_one_and_never_reach_zero() {
        let mut seq = Sequence::new();
        assert_eq!([seq.take(), seq.take(), seq.take()], [1, 2, 3]);

        let mut seq = Sequence { next: u32::MAX };
        assert_eq!(seq.take(), u32::MAX);
        assert_eq!(seq.take(), 1, "the wrap must skip zero, not land on it");
    }

    // -- expect_ack -------------------------------------------------------

    fn ack(seq: u32, code: i32) -> Vec<u8> {
        let mut m = Message::new(wire::NLMSG_ERROR, 0, seq);
        m.push_payload(&code.to_ne_bytes());
        m.finish()
    }

    #[test]
    fn an_error_message_carrying_zero_is_the_success_case() {
        assert_eq!(expect_ack(&ack(5, 0), 5), Ok(()));
    }

    #[test]
    fn an_error_message_carrying_an_errno_is_reported_as_that_errno() {
        assert_eq!(
            expect_ack(&ack(5, -1), 5),
            Err(Failure::Kernel(Errno::EPERM))
        );
    }

    /// The refusal that has to be a refusal. If a reply to an earlier request
    /// were skipped, the *answer* to this one might be what got skipped, and
    /// the caller would believe an address was assigned that was not.
    #[test]
    fn a_reply_to_a_different_request_is_refused_rather_than_skipped() {
        assert_eq!(
            expect_ack(&ack(4, 0), 5),
            Err(Failure::WrongSequence {
                expected: 5,
                got: 4
            })
        );
    }

    #[test]
    fn a_noop_is_stepped_over_and_the_real_answer_is_read() {
        let mut reply = Message::new(wire::NLMSG_NOOP, 0, 5).finish();
        reply.extend(ack(5, 0));
        assert_eq!(expect_ack(&reply, 5), Ok(()));
    }

    #[test]
    fn an_overrun_is_reported_rather_than_read_past() {
        let reply = Message::new(wire::NLMSG_OVERRUN, 0, 5).finish();
        assert_eq!(expect_ack(&reply, 5), Err(Failure::Overrun));
    }

    #[test]
    fn a_message_that_is_not_an_acknowledgement_is_refused() {
        let reply = Message::new(wire::RTM_NEWLINK, 0, 5).finish();
        assert_eq!(
            expect_ack(&reply, 5),
            Err(Failure::Unexpected {
                kind: wire::RTM_NEWLINK
            })
        );
    }

    /// An empty datagram is what a socket returns when nothing was sent. It is
    /// emphatically not an acknowledgement, and reading it as one would report
    /// a request that never reached the kernel as applied.
    #[test]
    fn an_empty_reply_is_not_an_acknowledgement() {
        assert_eq!(
            expect_ack(&[], 5),
            Err(Failure::Malformed(Malformed::ShortHeader { available: 0 }))
        );
    }

    #[test]
    fn an_acknowledgement_with_no_payload_is_refused_rather_than_assumed_ok() {
        let reply = Message::new(wire::NLMSG_ERROR, 0, 5).finish();
        assert!(matches!(
            expect_ack(&reply, 5),
            Err(Failure::Malformed(Malformed::ShortPayload { .. }))
        ));
    }

    #[test]
    fn every_failure_renders_something_an_operator_can_act_on() {
        let cases: [(Failure, &str); 8] = [
            (Failure::Unterminated { datagrams: 1024 }, "NLMSG_DONE"),
            (Failure::Socket(Errno::EPERM), "EPERM"),
            (Failure::Kernel(Errno::ENODEV), "ENODEV"),
            (
                Failure::Malformed(Malformed::ShortHeader { available: 0 }),
                "did not parse",
            ),
            (
                Failure::WrongSequence {
                    expected: 5,
                    got: 4,
                },
                "5",
            ),
            (Failure::Unexpected { kind: 16 }, "16"),
            (Failure::Overrun, "OVERRUN"),
            (Failure::Interrupted, "NLM_F_DUMP_INTR"),
        ];
        for (failure, expected) in cases {
            assert!(
                failure.to_string().contains(expected),
                "{failure:?} does not mention {expected}"
            );
        }
    }

    // -- dump -------------------------------------------------------------

    /// The failure this whole function exists to prevent, made reachable.
    ///
    /// A dump spread over two datagrams: `transact` would return the first and
    /// the caller would report half the machine's addresses, with no error to
    /// notice. Break the loop below to a single `recv` and this test fails.
    #[test]
    fn a_dump_spread_over_two_datagrams_returns_both() {
        use crate::testnetlink::{FakeNetlink, Script, addr_reply, dump_done};

        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(
            vec![
                Ok(addr_reply(1, 2, [192, 168, 50, 66], 24, wire::RT_SCOPE_UNIVERSE)),
                Ok(addr_reply(1, 3, [10, 0, 0, 1], 8, wire::RT_SCOPE_UNIVERSE)),
            ],
            Ok(dump_done(1)),
        );
        let got = dump(&mut nl, Message::new(wire::RTM_GETADDR, 0, 1).finish(), 1)
            .expect("a terminated dump");
        assert_eq!(got.len(), 3, "two data datagrams and the terminator");
        assert_eq!(nl.recv_calls, 3);
    }

    /// `NLMSG_DONE` packed behind the last data in one datagram, which is what
    /// the kernel does whenever the answer fits. A reader that stopped at the
    /// first `DONE` message without finishing the datagram would pass this by
    /// accident; a reader that required a datagram of its own would hang.
    #[test]
    fn a_terminator_sharing_a_datagram_with_data_ends_the_dump() {
        use crate::testnetlink::{FakeNetlink, Script, addr_reply, dump_done};

        let mut both = addr_reply(1, 2, [127, 0, 0, 1], 8, wire::RT_SCOPE_HOST);
        both.extend(dump_done(1));
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(both));
        let got = dump(&mut nl, Vec::new(), 1).expect("a terminated dump");
        assert_eq!(got.len(), 1);
        assert_eq!(nl.recv_calls, 1, "no second receive after the terminator");
    }

    #[test]
    fn a_dump_the_kernel_refuses_reports_the_errno_rather_than_an_empty_list() {
        use crate::testnetlink::{FakeNetlink, Script, netlink_error};

        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(netlink_error(1, -1)));
        assert_eq!(
            dump(&mut nl, Vec::new(), 1),
            Err(Failure::Kernel(Errno::EPERM))
        );
    }

    /// The same reasoning as `expect_ack`'s: skipping is what would let the
    /// answer to *this* question be the message thrown away.
    #[test]
    fn a_dump_datagram_from_another_conversation_is_refused_rather_than_skipped() {
        use crate::testnetlink::{FakeNetlink, Script, addr_reply};

        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(addr_reply(
            4,
            2,
            [10, 0, 0, 1],
            24,
            wire::RT_SCOPE_UNIVERSE,
        )));
        assert_eq!(
            dump(&mut nl, Vec::new(), 5),
            Err(Failure::WrongSequence {
                expected: 5,
                got: 4
            })
        );
    }

    #[test]
    fn an_overrun_during_a_dump_is_reported_rather_than_read_past() {
        use crate::testnetlink::{FakeNetlink, Script};

        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(Message::new(wire::NLMSG_OVERRUN, 0, 1).finish()));
        assert_eq!(dump(&mut nl, Vec::new(), 1), Err(Failure::Overrun));
    }

    /// A kernel that never says `DONE` must end the loop, not the process.
    #[test]
    fn a_dump_that_is_never_terminated_is_refused_rather_than_looping_for_ever() {
        use crate::testnetlink::{FakeNetlink, Script, addr_reply};

        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(addr_reply(
            1,
            2,
            [10, 0, 0, 1],
            24,
            wire::RT_SCOPE_UNIVERSE,
        )));
        assert_eq!(
            dump(&mut nl, Vec::new(), 1),
            Err(Failure::Unterminated {
                datagrams: DUMP_DATAGRAM_LIMIT
            })
        );
    }

    /// The receive timeout is the other guard, and it reaches the caller as a
    /// socket failure rather than as an empty answer.
    #[test]
    fn a_dump_that_times_out_reports_the_socket_rather_than_no_addresses() {
        use crate::testnetlink::{FakeNetlink, Script};

        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Err(Errno::EAGAIN));
        assert_eq!(
            dump(&mut nl, Vec::new(), 1),
            Err(Failure::Socket(Errno::EAGAIN))
        );
    }

    // -- dump: NLM_F_DUMP_INTR --------------------------------------------

    /// The whole point of the resend: one concurrent change does not have to
    /// end in a refusal, and it is the SECOND attempt's data that comes back
    /// — not the first, discarded, attempt's.
    #[test]
    fn an_interrupted_dump_followed_by_a_clean_one_returns_the_clean_ones_data() {
        use crate::testnetlink::{FakeNetlink, Script, addr_reply, dump_done, mark_interrupted};

        let disturbed = addr_reply(1, 2, [10, 0, 0, 1], 24, wire::RT_SCOPE_UNIVERSE);
        let settled = addr_reply(1, 3, [10, 0, 0, 2], 24, wire::RT_SCOPE_UNIVERSE);
        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(
            vec![
                Ok(disturbed),
                Ok(mark_interrupted(dump_done(1))),
                Ok(settled.clone()),
                Ok(dump_done(1)),
            ],
            Ok(Vec::new()),
        );

        let got = dump(&mut nl, Message::new(wire::RTM_GETADDR, 0, 1).finish(), 1)
            .expect("the retry completes cleanly");
        assert_eq!(got, vec![settled, dump_done(1)], "the discarded attempt's data must not leak through");
        assert_eq!(nl.sent.len(), 2, "the request is resent once");
        assert_eq!(nl.sent[0], nl.sent[1], "the SAME request, not a different one");
    }

    /// A table under constant change is a reason to stop and say so, not to
    /// loop against it forever.
    #[test]
    fn two_interrupted_dumps_in_a_row_give_the_new_failure() {
        use crate::testnetlink::{FakeNetlink, Script, dump_done, mark_interrupted};

        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(mark_interrupted(dump_done(1))));
        assert_eq!(dump(&mut nl, Vec::new(), 1), Err(Failure::Interrupted));
        assert_eq!(nl.sent.len(), 2, "resent once, then given up on");
    }

    /// The kernel may set the flag on whichever message it was mid-writing
    /// when the change happened — the terminator is not guaranteed to be it.
    #[test]
    fn an_interrupt_flag_on_an_intermediate_message_is_caught() {
        use crate::testnetlink::{FakeNetlink, Script, addr_reply, dump_done, mark_interrupted};

        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(
            vec![Ok(mark_interrupted(addr_reply(
                1,
                2,
                [10, 0, 0, 1],
                24,
                wire::RT_SCOPE_UNIVERSE,
            )))],
            Ok(dump_done(1)),
        );
        let outcome = dump_attempt(&mut nl, &Vec::new(), 1).expect("a terminated attempt");
        assert!(matches!(outcome, DumpAttempt::Interrupted));
    }

    /// The other place the flag can land: on `NLMSG_DONE` itself.
    #[test]
    fn an_interrupt_flag_on_the_terminator_itself_is_caught() {
        use crate::testnetlink::{FakeNetlink, Script, addr_reply, dump_done, mark_interrupted};

        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(
            vec![Ok(addr_reply(1, 2, [10, 0, 0, 1], 24, wire::RT_SCOPE_UNIVERSE))],
            Ok(mark_interrupted(dump_done(1))),
        );
        let outcome = dump_attempt(&mut nl, &Vec::new(), 1).expect("a terminated attempt");
        assert!(matches!(outcome, DumpAttempt::Interrupted));
    }

    /// A kernel new enough to set `NLM_F_DUMP_INTR` can also fail the dump
    /// outright by packing a negative errno into `NLMSG_DONE` itself, in the
    /// same encoding `NLMSG_ERROR` uses. Refused through the existing error
    /// path, and not retried — this is the kernel refusing, not a transient
    /// interruption.
    #[test]
    fn a_negative_done_error_refuses_with_that_errno() {
        use crate::testnetlink::{FakeNetlink, Script, dump_done_with_error};

        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(dump_done_with_error(1, -19)));
        assert_eq!(
            dump(&mut nl, Vec::new(), 1),
            Err(Failure::Kernel(Errno::ENODEV))
        );
        assert_eq!(nl.sent.len(), 1, "a kernel refusal is not retried");
    }

    /// One to three bytes in `NLMSG_DONE`'s payload is malformed rather than
    /// zero-extended into a plausible errno.
    #[test]
    fn a_short_done_payload_is_refused() {
        use crate::testnetlink::{FakeNetlink, Script};

        // `push_payload` pads to a four-byte boundary, so a genuinely short
        // payload cannot come from the normal builder — it is hand-patched
        // afterwards. `nlmsg_len` may legally be unaligned: alignment governs
        // where the NEXT message starts, not what one message may claim.
        let mut done = Message::new(wire::NLMSG_DONE, wire::NLM_F_MULTI, 1);
        done.push_payload(&[0xAB, 0xCD]);
        let mut bytes = done.finish();
        bytes[..4].copy_from_slice(&18u32.to_ne_bytes()); // NLMSG_HDRLEN(16) + 2

        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(bytes));
        assert_eq!(
            dump(&mut nl, Vec::new(), 1),
            Err(Failure::Malformed(Malformed::ShortPayload { want: 4, got: 2 })),
            "a short NLMSG_DONE payload must be refused, not zero-extended"
        );
    }

    /// The claim in this module's documentation, checked wherever it can be:
    /// these numbers are Linux's. It cannot run on the Windows development
    /// host, which is exactly why they are not taken from `libc`.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_errno_constants_match_this_platforms_libc() {
        let pairs = [
            (Errno::EPERM, libc::EPERM),
            (Errno::ENOENT, libc::ENOENT),
            (Errno::ESRCH, libc::ESRCH),
            (Errno::EINTR, libc::EINTR),
            (Errno::EIO, libc::EIO),
            (Errno::EAGAIN, libc::EAGAIN),
            (Errno::ENOMEM, libc::ENOMEM),
            (Errno::EACCES, libc::EACCES),
            (Errno::EBUSY, libc::EBUSY),
            (Errno::EEXIST, libc::EEXIST),
            (Errno::ENODEV, libc::ENODEV),
            (Errno::EINVAL, libc::EINVAL),
            (Errno::EPROTONOSUPPORT, libc::EPROTONOSUPPORT),
            (Errno::EOPNOTSUPP, libc::EOPNOTSUPP),
            (Errno::EAFNOSUPPORT, libc::EAFNOSUPPORT),
            (Errno::EADDRNOTAVAIL, libc::EADDRNOTAVAIL),
            (Errno::ENETUNREACH, libc::ENETUNREACH),
            (Errno::ENOBUFS, libc::ENOBUFS),
            (Errno::ETIMEDOUT, libc::ETIMEDOUT),
        ];
        for (ours, theirs) in pairs {
            assert_eq!(ours.0, theirs, "{ours:?}");
        }
    }
}
