//! Finding an interface, and bringing it up.
//!
//! Two requests, and the first one is doing more work than it looks like.
//! `RTM_GETLINK` filtered by `IFLA_IFNAME` answers **both** questions this
//! crate has about a link — *does it exist, and what is its index* — and
//! carries `ifi_flags` as well, so whether it is already up is known without a
//! second round trip. That is what makes [`crate::wired`] able to report
//! `Converged` for an interface that was already up rather than sending a
//! request whose success would say nothing (`ADR-015` §7: re-running converges,
//! it does not fail).
//!
//! `AGENTS.md` §5 — **the kernel is the authority for system state; never cache
//! its verdicts.** The index is read at the point of use and is not stored:
//! interface indices are reused after an interface is removed, so a cached one
//! is a request aimed at whatever took its place.

use rivet_core::net::InterfaceName;

use crate::port::{Failure, Sequence, SendRecv, expect_ack, transact};
use crate::wire::{self, IfInfo, Malformed, Message};

/// Ask the kernel about one interface by name.
///
/// `ENODEV` from this is the "interface does not exist" case, and it is the one
/// an operator meets most: a typo, or `eth0` on a board where the driver did
/// not bind. `ADR-004` §2.3's tripwire is the same question asked by hand.
pub fn find(
    nl: &mut dyn SendRecv,
    seq: &mut Sequence,
    name: &InterfaceName,
) -> Result<IfInfo, Failure> {
    let this_seq = seq.take();
    let mut message = Message::new(wire::RTM_GETLINK, wire::NLM_F_REQUEST, this_seq);
    message.push_payload(&ifinfomsg(0, 0, 0));
    // NUL-terminated: `IFLA_IFNAME` is a C string, and the kernel compares it
    // as one. A name sent without the terminator does not match anything.
    let mut name_bytes = name.as_str().as_bytes().to_vec();
    name_bytes.push(0);
    message.push_attr(wire::IFLA_IFNAME, &name_bytes);

    let reply = transact(nl, message.finish())?;
    read_link_reply(&reply, this_seq)
}

/// Set `IFF_UP` on an interface.
///
/// `ifi_change` is the mask of flags this request is allowed to touch, and it
/// is set to `IFF_UP` alone. **Sending `ifi_change = !0` — "I am authoritative
/// for every flag" — is the netlink mistake that silently clears
/// `IFF_MULTICAST`, `IFF_BROADCAST` and anything else the kernel had set**,
/// because every flag not present in `ifi_flags` is then taken as a request to
/// clear it.
/// Bring an interface up **without addressing it**, and say whether anything
/// changed.
///
/// # Why this exists separately from `wired::bring_up`
///
/// `ADR-004` §2.1 says *"almost nothing in the path is link-specific"*, and
/// this is the *almost*. A static plan raises the link as one step of applying
/// an address it already knows. **DHCP has to broadcast before it knows an
/// address at all**, and a broadcast on a down interface gets `sendto: Network
/// is down` — which was measured under QEMU on 2026-08-30, where `udhcpc`
/// started, sent a discover, and was answered by the kernel rather than by a
/// server.
///
/// So the order for DHCP is: raise, ask, address. This is the raise.
///
/// # Errors
///
/// The interface not existing, or the kernel refusing to raise it. Returns
/// `Ok(false)` when it was already up, which is not a failure and is the
/// ordinary case on a second run.
pub fn raise(nl: &mut dyn SendRecv, seq: &mut Sequence, interface: &InterfaceName)
-> Result<bool, Failure> {
    let info = find(nl, seq, interface)?;
    // **Asked rather than assumed**, the same shape `wired::bring_up` uses: a
    // request whose success says nothing is not a check.
    if info.is_up() {
        return Ok(false);
    }
    set_up(nl, seq, info.index)?;
    Ok(true)
}

pub fn set_up(nl: &mut dyn SendRecv, seq: &mut Sequence, index: i32) -> Result<(), Failure> {
    let this_seq = seq.take();
    let mut message = Message::new(
        wire::RTM_NEWLINK,
        wire::NLM_F_REQUEST | wire::NLM_F_ACK,
        this_seq,
    );
    message.push_payload(&ifinfomsg(index, wire::IFF_UP, wire::IFF_UP));
    let reply = transact(nl, message.finish())?;
    expect_ack(&reply, this_seq)
}

/// `struct ifinfomsg`, laid out by hand.
///
/// family(1) pad(1) type(2) index(4) flags(4) change(4) = 16 bytes. The family
/// is `AF_UNSPEC` in both requests: a link is not an address and has no family.
fn ifinfomsg(index: i32, flags: u32, change: u32) -> [u8; wire::IFINFOMSG_LEN] {
    let mut out = [0u8; wire::IFINFOMSG_LEN];
    out[0] = wire::AF_UNSPEC;
    out[4..8].copy_from_slice(&index.to_ne_bytes());
    out[8..12].copy_from_slice(&flags.to_ne_bytes());
    out[12..16].copy_from_slice(&change.to_ne_bytes());
    out
}

/// Read an `RTM_NEWLINK` answer to an `RTM_GETLINK` question.
///
/// Note what a bare `NLMSG_ERROR` means here, and that it is *not* the ack case
/// [`expect_ack`] handles: this request did not ask for an acknowledgement, so
/// the only reason the kernel would send an error message is that it is
/// refusing. A zero in one would therefore be a contradiction — an error
/// message saying nothing went wrong, in answer to a request that wanted data —
/// and it is refused rather than turned into an interface with index 0.
fn read_link_reply(reply: &[u8], expected_seq: u32) -> Result<IfInfo, Failure> {
    for message in wire::messages(reply)? {
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
                return Err(if code == 0 {
                    Failure::Unexpected {
                        kind: wire::NLMSG_ERROR,
                    }
                } else {
                    Failure::from_netlink_error(code)
                });
            }
            wire::RTM_NEWLINK => {
                if message.seq != expected_seq {
                    return Err(Failure::WrongSequence {
                        expected: expected_seq,
                        got: message.seq,
                    });
                }
                return Ok(wire::parse_ifinfo(message.payload)?);
            }
            kind => return Err(Failure::Unexpected { kind }),
        }
    }
    Err(Failure::Malformed(Malformed::ShortHeader { available: 0 }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::port::Errno;
    use crate::testnetlink::{FakeNetlink, Script, link_reply, netlink_error};

    fn eth0() -> InterfaceName {
        InterfaceName::new("eth0").expect("a valid name")
    }

    // -----------------------------------------------------------------
    // raise — ADR-004, for DHCP
    // -----------------------------------------------------------------

    /// An interface that is already up is not raised again, and the caller is
    /// told nothing changed. Asking the kernel first is the same shape
    /// `wired::bring_up` uses: a request whose success says nothing is not a
    /// check.
    #[test]
    fn raising_an_interface_that_is_already_up_changes_nothing() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(link_reply(1, 2, wire::IFF_UP)));
        let changed = raise(&mut nl, &mut Sequence::new(), &eth0()).expect("the fake answers");
        assert!(!changed, "an interface already up was raised again");
        assert_eq!(nl.sent.len(), 1, "it should have asked and stopped");
    }

    /// A down interface is raised, and the caller is told so — that line is
    /// worth printing because it is a change to the machine.
    #[test]
    fn a_down_interface_is_raised() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(
            vec![Ok(link_reply(1, 2, 0)), Ok(netlink_error(2, 0))],
            Ok(netlink_error(3, 0)),
        );
        let changed = raise(&mut nl, &mut Sequence::new(), &eth0()).expect("raised");
        assert!(changed);
        assert_eq!(nl.sent.len(), 2, "it should have asked, then set");
    }

    #[test]
    fn a_get_link_request_carries_the_name_the_caller_asked_for() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(link_reply(1, 2, wire::IFF_UP)));
        let mut seq = Sequence::new();

        let info = find(&mut nl, &mut seq, &eth0()).expect("the fake answers");

        assert_eq!(info.index, 2);
        assert!(info.is_up());
        let sent = nl.sent.first().expect("one request was sent");
        let parsed = wire::messages(sent).expect("well-formed");
        assert_eq!(parsed[0].kind, wire::RTM_GETLINK);
        assert_eq!(parsed[0].flags, wire::NLM_F_REQUEST);
        let attrs = wire::attributes(&parsed[0].payload[wire::IFINFOMSG_LEN..]).expect("attrs");
        assert_eq!(attrs, vec![(wire::IFLA_IFNAME, &b"eth0\0"[..])]);
    }

    /// The terminator is not decoration. `IFLA_IFNAME` is compared as a C
    /// string, so a name sent without it matches nothing and the interface
    /// reads as absent on a board where it is present.
    #[test]
    fn the_interface_name_is_nul_terminated() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(link_reply(1, 2, 0)));
        let _ = find(&mut nl, &mut Sequence::new(), &eth0());
        let sent = nl.sent.first().expect("one request");
        assert!(sent.ends_with(b"eth0\0\0\0\0"), "name is not NUL-terminated");
    }

    #[test]
    fn an_interface_that_does_not_exist_is_enodev() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(netlink_error(1, -19)));
        assert_eq!(
            find(&mut nl, &mut Sequence::new(), &eth0()),
            Err(Failure::Kernel(Errno::ENODEV))
        );
    }

    #[test]
    fn a_socket_that_refuses_to_send_is_reported_as_the_socket_failing() {
        let mut nl = FakeNetlink::new();
        nl.send_results = Script::always(Err(Errno::EPERM));
        assert_eq!(
            find(&mut nl, &mut Sequence::new(), &eth0()),
            Err(Failure::Socket(Errno::EPERM))
        );
        assert!(nl.recv_calls == 0, "nothing may be read after a failed send");
    }

    #[test]
    fn a_receive_that_times_out_is_reported_as_the_socket_failing() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Err(Errno::EAGAIN));
        assert_eq!(
            find(&mut nl, &mut Sequence::new(), &eth0()),
            Err(Failure::Socket(Errno::EAGAIN))
        );
    }

    /// An error message carrying zero is an *acknowledgement*, and this request
    /// did not ask for one — it asked for data. Reading it as "index 0" would
    /// aim every later request at whatever interface 0 is.
    #[test]
    fn an_acknowledgement_is_not_an_answer_to_a_question_that_wanted_data() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(netlink_error(1, 0)));
        assert_eq!(
            find(&mut nl, &mut Sequence::new(), &eth0()),
            Err(Failure::Unexpected {
                kind: wire::NLMSG_ERROR
            })
        );
    }

    #[test]
    fn an_answer_to_a_different_question_is_refused() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(link_reply(99, 2, wire::IFF_UP)));
        assert_eq!(
            find(&mut nl, &mut Sequence::new(), &eth0()),
            Err(Failure::WrongSequence {
                expected: 1,
                got: 99
            })
        );
    }

    #[test]
    fn a_truncated_link_reply_is_refused_rather_than_zero_filled() {
        let mut nl = FakeNetlink::new();
        let mut short = Message::new(wire::RTM_NEWLINK, 0, 1);
        short.push_payload(&[0u8; 8]);
        nl.replies = Script::always(Ok(short.finish()));
        assert!(matches!(
            find(&mut nl, &mut Sequence::new(), &eth0()),
            Err(Failure::Malformed(Malformed::ShortPayload { .. }))
        ));
    }

    // -- set_up -----------------------------------------------------------

    #[test]
    fn setting_a_link_up_asks_for_an_acknowledgement_and_changes_only_that_flag() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(netlink_error(1, 0)));

        assert_eq!(set_up(&mut nl, &mut Sequence::new(), 2), Ok(()));

        let sent = nl.sent.first().expect("one request");
        let parsed = wire::messages(sent).expect("well-formed");
        assert_eq!(parsed[0].kind, wire::RTM_NEWLINK);
        assert_eq!(
            parsed[0].flags,
            wire::NLM_F_REQUEST | wire::NLM_F_ACK,
            "without NLM_F_ACK the kernel says nothing and success is assumed"
        );
        let payload = parsed[0].payload;
        assert_eq!(i32::from_ne_bytes(payload[4..8].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_ne_bytes(payload[8..12].try_into().unwrap()),
            wire::IFF_UP
        );
        // The mask. `!0` here would clear every flag the kernel already had.
        assert_eq!(
            u32::from_ne_bytes(payload[12..16].try_into().unwrap()),
            wire::IFF_UP,
            "ifi_change must name IFF_UP alone"
        );
    }

    #[test]
    fn a_link_the_process_may_not_change_is_eperm() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(netlink_error(1, -1)));
        assert_eq!(
            set_up(&mut nl, &mut Sequence::new(), 2),
            Err(Failure::Kernel(Errno::EPERM))
        );
    }

    /// Every request in one bring-up must carry a different sequence number, or
    /// the correlation that [`Failure::WrongSequence`] exists to enforce is
    /// meaningless.
    #[test]
    fn consecutive_requests_carry_consecutive_sequence_numbers() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(
            vec![Ok(link_reply(1, 2, 0))],
            Ok(netlink_error(2, 0)),
        );
        let mut seq = Sequence::new();
        let _ = find(&mut nl, &mut seq, &eth0());
        let _ = set_up(&mut nl, &mut seq, 2);

        let seqs: Vec<u32> = nl
            .sent
            .iter()
            .map(|m| wire::messages(m).expect("well-formed")[0].seq)
            .collect();
        assert_eq!(seqs, [1, 2]);
    }
}
