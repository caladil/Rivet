//! A [`SendRecv`] that answers however a test needs it to.
//!
//! Compiled only under `cfg(test)`. It exists so this crate's decisions can be
//! driven through states a developer's machine does not reach: an interface
//! that is not there, an address already assigned, a route already present, a
//! kernel refusing for want of `CAP_NET_ADMIN`, a socket that will not accept a
//! write. At Stage 1 **none of those is reachable under the emulator either** —
//! `PLAN.md` §0/§2.2 records that the Pi kernel binds no virtio device under
//! `-M virt`, so there is no emulated interface to bring up.
//!
//! Every answer is a [`Script`], so a *sequence* of answers can be expressed —
//! which is the point. The interesting boundary is rarely one call's result; it
//! is "the interface is found, then the address is refused", which is what a
//! half-configured re-run actually does.
//!
//! `Script` is a near-copy of the one in `cmd/rivet-initd/src/testkernel.rs`,
//! deliberately. Sharing it would mean either an `adapter → cmd` dependency,
//! which is not a thing a binary crate can offer, or a shared test crate, which
//! `ADR-014` §3 has no room for and which would make two suites fail together.
//! Twenty lines duplicated is the cheaper of the three.

use std::collections::VecDeque;

use crate::port::{Errno, SendRecv};
use crate::wire::{self, Message};

/// A queue of answers ending in one that repeats for ever.
///
/// `Script::sequence(vec![a, b], z)` answers `a`, then `b`, then `z` on every
/// call after. `Script::always(z)` answers `z` from the start.
#[derive(Debug, Clone)]
pub struct Script<T: Clone> {
    queue: VecDeque<T>,
    forever: T,
}

impl<T: Clone> Script<T> {
    pub fn always(forever: T) -> Self {
        Self {
            queue: VecDeque::new(),
            forever,
        }
    }

    pub fn sequence(steps: Vec<T>, forever: T) -> Self {
        Self {
            queue: steps.into(),
            forever,
        }
    }

    pub fn next(&mut self) -> T {
        self.queue.pop_front().unwrap_or_else(|| self.forever.clone())
    }
}

/// The fake socket.
///
/// The default is a socket that accepts everything and answers nothing, which
/// is deliberately *not* a happy path: a test has to say what the kernel
/// replied, because "what did the kernel say" is the whole subject.
pub struct FakeNetlink {
    /// Every message handed to [`SendRecv::send`], in order.
    pub sent: Vec<Vec<u8>>,
    pub recv_calls: usize,
    pub send_results: Script<Result<(), Errno>>,
    pub replies: Script<Result<Vec<u8>, Errno>>,
}

impl FakeNetlink {
    pub fn new() -> Self {
        Self {
            sent: Vec::new(),
            recv_calls: 0,
            send_results: Script::always(Ok(())),
            // An empty datagram, which `expect_ack` refuses. A fake whose
            // default was "success" would let a test that forgot to say what
            // the kernel replied pass anyway.
            replies: Script::always(Ok(Vec::new())),
        }
    }
}

impl SendRecv for FakeNetlink {
    fn send(&mut self, message: &[u8]) -> Result<(), Errno> {
        let result = self.send_results.next();
        // Recorded even when the send "fails", so a test can assert on what
        // was about to go out. A real socket that returns an error may still
        // have transmitted, and pretending otherwise would hide that.
        self.sent.push(message.to_vec());
        result
    }

    fn recv(&mut self) -> Result<Vec<u8>, Errno> {
        self.recv_calls += 1;
        self.replies.next()
    }
}

/// An `NLMSG_ERROR` message: an acknowledgement when `code` is 0, a refusal
/// when it is a negative errno.
pub fn netlink_error(seq: u32, code: i32) -> Vec<u8> {
    let mut m = Message::new(wire::NLMSG_ERROR, 0, seq);
    m.push_payload(&code.to_ne_bytes());
    m.finish()
}

/// An `RTM_NEWADDR` reply describing one assigned IPv4 address.
///
/// Built the way the kernel builds one: the eight-byte `ifaddrmsg`, then
/// `IFA_ADDRESS` and `IFA_LOCAL` **in that order**. The order is deliberate and
/// is the point of several tests below — a reader that took whichever came
/// first would be right on Ethernet and wrong on a tunnel, and this fixture
/// puts the tempting one first so the mistake is reachable.
pub fn addr_reply(seq: u32, index: u32, octets: [u8; 4], prefix_len: u8, scope: u8) -> Vec<u8> {
    let mut header = [0u8; wire::IFADDRMSG_LEN];
    header[0] = wire::AF_INET;
    header[1] = prefix_len;
    header[3] = scope;
    header[4..8].copy_from_slice(&index.to_ne_bytes());
    let mut m = Message::new(wire::RTM_NEWADDR, wire::NLM_F_MULTI, seq);
    m.push_payload(&header);
    m.push_attr(wire::IFA_ADDRESS, &octets);
    m.push_attr(wire::IFA_LOCAL, &octets);
    m.finish()
}

/// An `RTM_NEWROUTE` reply describing one IPv4 route, hand-composed in the
/// shape the kernel sends: the twelve-byte `rtmsg` with `rtm_table` set, then
/// `RTA_TABLE`, `RTA_PRIORITY` when there is a metric, `RTA_GATEWAY` when there
/// is a next hop, and `RTA_OIF`.
pub fn route_reply(
    seq: u32,
    dst_len: u8,
    table: u32,
    oif: u32,
    gateway: Option<[u8; 4]>,
    priority: Option<u32>,
) -> Vec<u8> {
    let mut header = [0u8; wire::RTMSG_LEN];
    header[0] = wire::AF_INET;
    header[1] = dst_len;
    header[4] = u8::try_from(table).unwrap_or(252);
    header[5] = wire::RTPROT_STATIC;
    header[6] = wire::RT_SCOPE_UNIVERSE;
    header[7] = wire::RTN_UNICAST;
    let mut m = Message::new(wire::RTM_NEWROUTE, wire::NLM_F_MULTI, seq);
    m.push_payload(&header);
    m.push_attr(wire::RTA_TABLE, &table.to_ne_bytes());
    if let Some(priority) = priority {
        m.push_attr(wire::RTA_PRIORITY, &priority.to_ne_bytes());
    }
    if let Some(gateway) = gateway {
        m.push_attr(wire::RTA_GATEWAY, &gateway);
    }
    m.push_attr(wire::RTA_OIF, &oif.to_ne_bytes());
    m.finish()
}

/// The `NLMSG_DONE` that terminates a dump.
pub fn dump_done(seq: u32) -> Vec<u8> {
    let mut m = Message::new(wire::NLMSG_DONE, wire::NLM_F_MULTI, seq);
    // The kernel puts a zero `int` in a DONE. A recent-enough kernel can put
    // a negative errno here instead; see `dump_done_with_error`.
    m.push_payload(&0i32.to_ne_bytes());
    m.finish()
}

/// As [`dump_done`], but reporting the dump itself failed: `code` is a
/// negative errno, in the same encoding `NLMSG_ERROR` uses.
pub fn dump_done_with_error(seq: u32, code: i32) -> Vec<u8> {
    let mut m = Message::new(wire::NLMSG_DONE, wire::NLM_F_MULTI, seq);
    m.push_payload(&code.to_ne_bytes());
    m.finish()
}

/// Sets [`wire::NLM_F_DUMP_INTR`] on an already-built message, for a test
/// that wants to mark an ordinary reply as part of an interrupted dump
/// without a second builder for every message shape that can carry it.
pub fn mark_interrupted(mut message: Vec<u8>) -> Vec<u8> {
    if let Some(flags) = message.get_mut(6..8) {
        let current = u16::from_ne_bytes([flags[0], flags[1]]);
        flags.copy_from_slice(&(current | wire::NLM_F_DUMP_INTR).to_ne_bytes());
    }
    message
}

/// An `RTM_NEWLINK` reply describing one interface.
pub fn link_reply(seq: u32, index: i32, flags: u32) -> Vec<u8> {
    let mut payload = [0u8; wire::IFINFOMSG_LEN];
    payload[4..8].copy_from_slice(&index.to_ne_bytes());
    payload[8..12].copy_from_slice(&flags.to_ne_bytes());
    let mut m = Message::new(wire::RTM_NEWLINK, 0, seq);
    m.push_payload(&payload);
    m.finish()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_script_repeats_its_last_answer_for_ever() {
        let mut s = Script::sequence(vec![1, 2], 9);
        assert_eq!([s.next(), s.next(), s.next(), s.next()], [1, 2, 9, 9]);
    }

    #[test]
    fn an_always_script_never_changes_its_answer() {
        let mut s = Script::always(7);
        assert_eq!([s.next(), s.next()], [7, 7]);
    }

    /// The default must not be a success. A test that forgets to script a reply
    /// should fail, not pass.
    #[test]
    fn the_default_reply_is_not_an_acknowledgement() {
        let mut nl = FakeNetlink::new();
        assert_eq!(nl.recv(), Ok(Vec::new()));
        assert!(crate::port::expect_ack(&[], 1).is_err());
    }

    #[test]
    fn a_failed_send_is_still_recorded_because_a_real_one_may_have_transmitted() {
        let mut nl = FakeNetlink::new();
        nl.send_results = Script::always(Err(Errno::ENOBUFS));
        assert_eq!(nl.send(b"hello"), Err(Errno::ENOBUFS));
        assert_eq!(nl.sent, vec![b"hello".to_vec()]);
    }

    /// The two builders are what every other suite in this crate asserts
    /// against, so they are checked against the parser rather than trusted.
    #[test]
    fn the_builders_produce_messages_the_parser_agrees_with() {
        let ack = netlink_error(3, 0);
        let parsed = wire::messages(&ack).expect("well-formed");
        assert_eq!(parsed[0].kind, wire::NLMSG_ERROR);
        assert_eq!(parsed[0].seq, 3);
        assert_eq!(wire::error_code(parsed[0].payload), Ok(0));

        let link = link_reply(4, 7, wire::IFF_UP | wire::IFF_LOWER_UP);
        let parsed = wire::messages(&link).expect("well-formed");
        assert_eq!(parsed[0].kind, wire::RTM_NEWLINK);
        let info = wire::parse_ifinfo(parsed[0].payload).expect("well-formed");
        assert_eq!(info.index, 7);
        assert!(info.is_up() && info.has_carrier());
    }

    #[test]
    fn marking_a_message_interrupted_sets_only_that_flag() {
        let plain = dump_done(1);
        let marked = mark_interrupted(plain.clone());
        assert_ne!(marked, plain, "the bytes must actually change");

        let parsed = wire::messages(&marked).expect("well-formed");
        assert_eq!(parsed[0].flags, wire::NLM_F_MULTI | wire::NLM_F_DUMP_INTR);
        // Everything else about the message is untouched.
        assert_eq!(parsed[0].kind, wire::NLMSG_DONE);
        assert_eq!(parsed[0].seq, 1);
    }
}
