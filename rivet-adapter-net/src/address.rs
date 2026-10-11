//! Assigning an IPv4 address to an interface — and, since 2026-08-31, reading
//! back which ones are assigned.
//!
//! One request, `RTM_NEWADDR`, and the only interesting decisions are which
//! attributes go on it and what `EEXIST` means.
//!
//! # Why this crate reads as well as writes now
//!
//! `crate::lib`'s survey describes this crate as a writer, and until this
//! change it was one. What overturned that is a defect rather than a
//! preference: `rivet get network` on a board reported `"addresses":[]` for
//! `eth0` — **on the interface the question arrived over**, statically
//! addressed in that machine's own document. `cmd/rivet`'s network module read
//! `/sys/class/net` and `/proc/net/*`, and the kernel exposes IPv6 addresses at
//! `/proc/net/if_inet6` and has never had an IPv4 equivalent, so there was
//! nowhere for the address to come from. `ADR-034` §3.3 promises a lost fleet
//! register can be rebuilt *"by asking the fleet"*; a machine that cannot say
//! what address it has cannot honour that.
//!
//! **The tripwire in `crate::lib`'s survey was checked rather than stepped
//! over.** It names two things that would overturn the no-dependency decision,
//! and the first is *parsing broadcasts* — decoding messages Rivet did not
//! construct, "the full `IFLA_*` and `IFA_*` attribute space, which is a
//! parser". This is not that, and the difference is worth stating because the
//! two look alike:
//!
//! * It is **request/response, not a broadcast**. Nothing joins a multicast
//!   group, nothing runs a receive loop, and the answer is correlated to the
//!   question by sequence number exactly as the four existing requests are.
//! * The attribute space read is **two constants** — `IFA_LOCAL` and
//!   `IFA_ADDRESS`, both of which this module already *sends* — plus the eight
//!   fixed bytes of `struct ifaddrmsg`, which [`assign`] already writes. Every
//!   other attribute is carried through unexamined.
//!
//! The one genuinely new mechanism is the multi-datagram read, and it is in
//! [`crate::port::dump`] with tests of its own.
//!
//! # Why `IFA_LOCAL` *and* `IFA_ADDRESS`
//!
//! On a point-to-point link they differ: `IFA_LOCAL` is this end and
//! `IFA_ADDRESS` is the peer. On a broadcast link like Ethernet they are the
//! same value, and the kernel will fill one in from the other. Sending both is
//! what `iproute2` does, and it is the version that stays correct if this is
//! ever pointed at a tunnel — where sending only `IFA_ADDRESS` would set the
//! *peer's* address and quietly produce a link that cannot be reached.
//!
//! # Why the broadcast address is sent explicitly
//!
//! The kernel derives one when it is not told, so this is redundant on a `/24`.
//! It is sent because [`Ipv4Cidr::broadcast`](rivet_core::net::Ipv4Cidr::broadcast)
//! already returns `None` for `/31` and `/32` — the two prefixes that have no
//! broadcast address at all — so Rivet's answer and the kernel's cannot
//! disagree, and the case where they would is the case nobody tests.

use std::net::Ipv4Addr;

use rivet_core::net::Ipv4Cidr;

use crate::port::{Failure, Sequence, SendRecv, dump as dump_datagrams, expect_ack, transact};
use crate::wire::{self, Message};

/// Assign `cidr` to the interface at `index`.
///
/// Sent with `NLM_F_CREATE | NLM_F_EXCL`, so an address that is already there
/// comes back as `EEXIST` rather than being silently replaced.
/// [`crate::wired`] turns that into `Converged`. **Without `NLM_F_EXCL` the
/// kernel would overwrite**, and "it was already correct" and "we just replaced
/// somebody else's address" would be the same answer — which is `ADR-011`'s
/// rule in miniature: one fact, one place, and the kernel is the place.
pub fn assign(
    nl: &mut dyn SendRecv,
    seq: &mut Sequence,
    index: i32,
    cidr: &Ipv4Cidr,
) -> Result<(), Failure> {
    let this_seq = seq.take();
    let mut message = Message::new(
        wire::RTM_NEWADDR,
        wire::NLM_F_REQUEST | wire::NLM_F_ACK | wire::NLM_F_CREATE | wire::NLM_F_EXCL,
        this_seq,
    );
    message.push_payload(&ifaddrmsg(index, cidr.prefix_len()));
    let octets = cidr.address().octets();
    message.push_attr(wire::IFA_LOCAL, &octets);
    message.push_attr(wire::IFA_ADDRESS, &octets);
    if let Some(broadcast) = cidr.broadcast() {
        message.push_attr(wire::IFA_BROADCAST, &broadcast.octets());
    }

    let reply = transact(nl, message.finish())?;
    expect_ack(&reply, this_seq)
}

/// Withdraw `assigned` from its interface — `RTM_DELADDR`.
///
/// The request names the interface and this end's address, `IFA_LOCAL`, which
/// is what the kernel matches a deletion on. `IFA_ADDRESS` is left off: on a
/// point-to-point link it is the peer, and [`Assigned`] carries only this end.
/// An address that is already gone comes back as `EADDRNOTAVAIL`, which
/// [`crate::wired::withdraw_all_but`] reads as already withdrawn.
///
/// # Errors
///
/// The kernel's refusal, or a socket failure, as [`assign`]'s.
pub fn remove(
    nl: &mut dyn SendRecv,
    seq: &mut Sequence,
    assigned: &Assigned,
) -> Result<(), Failure> {
    let this_seq = seq.take();
    let mut message = Message::new(
        wire::RTM_DELADDR,
        wire::NLM_F_REQUEST | wire::NLM_F_ACK,
        this_seq,
    );
    let mut header = [0u8; wire::IFADDRMSG_LEN];
    header[0] = wire::AF_INET;
    header[1] = assigned.prefix_len;
    header[3] = assigned.scope;
    header[4..8].copy_from_slice(&assigned.index.to_ne_bytes());
    message.push_payload(&header);
    message.push_attr(wire::IFA_LOCAL, &assigned.address.octets());

    let reply = transact(nl, message.finish())?;
    expect_ack(&reply, this_seq)
}

/// `struct ifaddrmsg`: family(1) prefixlen(1) flags(1) scope(1) index(4).
///
/// `ifa_scope` is `RT_SCOPE_UNIVERSE`, which means "globally routable" and is
/// what an ordinary address on an ordinary link is. It is not derived from the
/// address: 192.168.x.x is private but its *scope* is still universe — RFC 1918
/// is about who routes it, not about how far the kernel thinks it reaches.
fn ifaddrmsg(index: i32, prefix_len: u8) -> [u8; wire::IFADDRMSG_LEN] {
    let mut out = [0u8; wire::IFADDRMSG_LEN];
    out[0] = wire::AF_INET;
    out[1] = prefix_len;
    out[2] = 0;
    out[3] = wire::RT_SCOPE_UNIVERSE;
    // ifa_index is unsigned in the header; a negative index is not a thing the
    // kernel ever reports, and casting rather than refusing here would send a
    // request for interface 4294967295.
    out[4..8].copy_from_slice(&u32::try_from(index).unwrap_or(0).to_ne_bytes());
    out
}

/// One IPv4 address the kernel says is assigned.
///
/// **The prefix length is carried as the byte the kernel sent, not as an
/// [`Ipv4Cidr`].** That type refuses a prefix above 32, which is right for
/// something Rivet is about to *ask for* and wrong for something the kernel has
/// *reported*: the only two things a reader could do with the refusal are drop
/// the address — which would say the interface has none — or fail the whole
/// dump because one entry was odd. `AGENTS.md` §5 says report what the system
/// said, so the number is carried outward and a caller may judge it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assigned {
    /// `/sys/class/net/<name>/ifindex` is the other end of this. **Netlink
    /// never sends the interface's name on an address message** — `IFA_LABEL`
    /// is close but is the *alias* label (`eth0:0` for a secondary), not the
    /// interface — so resolving it is the caller's job.
    pub index: u32,
    pub address: Ipv4Addr,
    pub prefix_len: u8,
    /// An `RT_SCOPE_*` value. [`scope_name`] renders the ones that have names.
    pub scope: u8,
}

/// An address message that arrived and could not be read.
///
/// **Counted, never dropped**, for the reason `rivet get network` already
/// applies to interfaces: an answer that quietly omits an address says the
/// machine does not have one, and that is the confidently-wrong class of
/// failure rather than a gap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unreadable {
    /// The interface, where the fixed header parsed far enough to say.
    pub index: Option<u32>,
    pub why: String,
}

/// Everything one `RTM_GETADDR` dump established, and what it could not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignments {
    /// In the order the kernel listed them, which is not arbitrary: the primary
    /// address of an interface precedes its secondaries. Sorting would produce
    /// stabler bytes and lose that, and the first address of an interface is
    /// the one a reader acts on.
    pub addresses: Vec<Assigned>,
    pub unreadable: Vec<Unreadable>,
}

/// The word `ip` prints for an `RT_SCOPE_*` value, where there is one.
///
/// `None` for anything else, so the caller prints the number rather than a
/// guess — the same contract `crate::port::Errno::described` has, and for the
/// same reason (`AGENTS.md` §5).
///
/// **These are not `/proc/net/if_inet6`'s scope numbers.** See
/// [`wire::RT_SCOPE_SITE`]; mapping one table's numbers with the other's names
/// prints `global` for a link-local address and does not fail.
pub fn scope_name(scope: u8) -> Option<&'static str> {
    Some(match scope {
        wire::RT_SCOPE_UNIVERSE => "global",
        wire::RT_SCOPE_SITE => "site",
        wire::RT_SCOPE_LINK => "link",
        wire::RT_SCOPE_HOST => "host",
        wire::RT_SCOPE_NOWHERE => "nowhere",
        _ => return None,
    })
}

/// Ask the kernel for every IPv4 address on this machine.
///
/// `RTM_GETADDR` with `NLM_F_DUMP`, filtered to [`wire::AF_INET`] by the
/// `ifaddrmsg` payload. **This needs no privilege** — reading the address list
/// is not `CAP_NET_ADMIN`'s business, only changing it is — so it works from an
/// unprivileged `rivet get network`.
///
/// # Which attribute is the address
///
/// `IFA_LOCAL` where it is present, `IFA_ADDRESS` otherwise. The order matters
/// and it is the mirror of the reason [`assign`] sends both: on a
/// point-to-point link `IFA_LOCAL` is **this end** and `IFA_ADDRESS` is the
/// peer's, so a reader that took `IFA_ADDRESS` first would report a tunnel's
/// far end as this machine's address. On Ethernet the two are equal and the
/// choice cannot be observed, which is exactly why it has to be reasoned about
/// rather than discovered.
///
/// # Errors
///
/// Anything [`crate::port::dump`] refuses: a socket failure, a kernel refusal,
/// a reply out of sequence, an overrun, or a dump the kernel never terminated.
/// **A single unreadable message is not one of them** — it lands in
/// [`Assignments::unreadable`].
pub fn dump(nl: &mut dyn SendRecv, seq: &mut Sequence) -> Result<Assignments, Failure> {
    let this_seq = seq.take();
    let mut message = Message::new(
        wire::RTM_GETADDR,
        wire::NLM_F_REQUEST | wire::NLM_F_DUMP,
        this_seq,
    );
    // The filter: family only. Every other field of the `ifaddrmsg` is zero,
    // which is what "no further filter" is spelled as.
    let mut filter = [0u8; wire::IFADDRMSG_LEN];
    filter[0] = wire::AF_INET;
    message.push_payload(&filter);

    let datagrams = dump_datagrams(nl, message.finish(), this_seq)?;

    let mut out = Assignments {
        addresses: Vec::new(),
        unreadable: Vec::new(),
    };
    for datagram in &datagrams {
        for reply in wire::messages(datagram)? {
            // `NLMSG_DONE` and `NLMSG_NOOP` reach here; `crate::port::dump`
            // refuses everything that is not one of those or an answer.
            if reply.kind != wire::RTM_NEWADDR {
                continue;
            }
            let header = match wire::parse_ifaddr(reply.payload) {
                Ok(header) => header,
                Err(why) => {
                    out.unreadable.push(Unreadable {
                        index: None,
                        why: format!("an address message's fixed header did not parse: {why}"),
                    });
                    continue;
                }
            };
            // **Checked even though the request asked for one family.** The
            // family field in a `GET` is a filter and a filter can be ignored;
            // reading an IPv6 address's first four bytes as an IPv4 address
            // would produce a plausible number.
            if header.family != wire::AF_INET {
                continue;
            }
            let attributes = match wire::attributes(
                reply.payload.get(wire::IFADDRMSG_LEN..).unwrap_or(&[]),
            ) {
                Ok(attributes) => attributes,
                Err(why) => {
                    out.unreadable.push(Unreadable {
                        index: Some(header.index),
                        why: format!("an address message's attributes did not parse: {why}"),
                    });
                    continue;
                }
            };
            let pick = |kind: u16| -> Option<Ipv4Addr> {
                attributes
                    .iter()
                    .find(|(k, _)| *k == kind)
                    .and_then(|(_, value)| <[u8; 4]>::try_from(*value).ok())
                    .map(Ipv4Addr::from)
            };
            match pick(wire::IFA_LOCAL).or_else(|| pick(wire::IFA_ADDRESS)) {
                Some(address) => out.addresses.push(Assigned {
                    index: header.index,
                    address,
                    prefix_len: header.prefix_len,
                    scope: header.scope,
                }),
                None => out.unreadable.push(Unreadable {
                    index: Some(header.index),
                    why: "an IPv4 address message carried neither a four-byte IFA_LOCAL nor a \
                          four-byte IFA_ADDRESS, so there is no address in it to report"
                        .to_string(),
                }),
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::port::Errno;
    use crate::testnetlink::{FakeNetlink, Script, netlink_error};
    use std::net::Ipv4Addr;

    /// One `RTM_NEWADDR` taken back off the wire.
    struct Sent {
        kind: u16,
        flags: u16,
        /// The `ifaddrmsg`, byte for byte.
        header: Vec<u8>,
        attrs: Vec<(u16, Vec<u8>)>,
    }

    fn sent_message(nl: &FakeNetlink) -> Sent {
        let sent = nl.sent.first().expect("one request was sent");
        let parsed = wire::messages(sent).expect("well-formed");
        let payload = parsed[0].payload;
        Sent {
            kind: parsed[0].kind,
            flags: parsed[0].flags,
            header: payload[..wire::IFADDRMSG_LEN].to_vec(),
            attrs: wire::attributes(&payload[wire::IFADDRMSG_LEN..])
                .expect("well-formed")
                .into_iter()
                .map(|(k, v)| (k, v.to_vec()))
                .collect(),
        }
    }

    fn assign_ok(cidr: &str) -> FakeNetlink {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(netlink_error(1, 0)));
        let cidr = Ipv4Cidr::parse(cidr).expect("a valid fixture");
        assert_eq!(assign(&mut nl, &mut Sequence::new(), 2, &cidr), Ok(()));
        nl
    }

    #[test]
    fn an_ordinary_address_carries_local_address_and_broadcast() {
        let nl = assign_ok("192.168.1.50/24");
        let sent = sent_message(&nl);

        assert_eq!(sent.kind, wire::RTM_NEWADDR);
        assert_eq!(
            sent.flags,
            wire::NLM_F_REQUEST | wire::NLM_F_ACK | wire::NLM_F_CREATE | wire::NLM_F_EXCL
        );
        assert_eq!(sent.header[0], wire::AF_INET);
        assert_eq!(sent.header[1], 24, "ifa_prefixlen");
        assert_eq!(sent.header[3], wire::RT_SCOPE_UNIVERSE);
        assert_eq!(u32::from_ne_bytes(sent.header[4..8].try_into().unwrap()), 2);

        assert_eq!(
            sent.attrs,
            vec![
                (wire::IFA_LOCAL, vec![192, 168, 1, 50]),
                (wire::IFA_ADDRESS, vec![192, 168, 1, 50]),
                (wire::IFA_BROADCAST, vec![192, 168, 1, 255]),
            ]
        );
    }

    /// The bug this catches: reaching for `to_be_bytes` on a `u32` form of the
    /// address. `struct in_addr` is network byte order, which is the octets in
    /// the order they are written — so the bytes on the wire must read
    /// `192, 168, 1, 50` and never `50, 1, 168, 192`.
    #[test]
    fn the_address_bytes_are_in_the_order_the_address_is_written() {
        let nl = assign_ok("10.20.30.40/24");
        assert_eq!(sent_message(&nl).attrs[0].1, vec![10, 20, 30, 40]);
    }

    #[test]
    fn a_slash_thirty_one_carries_no_broadcast_attribute() {
        let sent = sent_message(&assign_ok("10.0.0.1/31"));
        assert_eq!(sent.header[1], 31);
        assert_eq!(
            sent.attrs.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            [wire::IFA_LOCAL, wire::IFA_ADDRESS]
        );
    }

    #[test]
    fn a_slash_thirty_two_carries_no_broadcast_attribute() {
        let sent = sent_message(&assign_ok("10.0.0.1/32"));
        assert!(!sent.attrs.iter().any(|(k, _)| *k == wire::IFA_BROADCAST));
    }

    /// The failure path that is not a failure. Re-running `rivet-netd` on a
    /// machine that is already addressed must converge, not refuse — `ADR-015`
    /// §7. This asserts the *errno* reaches the caller; [`crate::wired`] is
    /// where it becomes `Converged`.
    #[test]
    fn an_address_that_is_already_assigned_comes_back_as_eexist() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(netlink_error(1, -17)));
        let cidr = Ipv4Cidr::parse("192.168.1.50/24").expect("valid");
        assert_eq!(
            assign(&mut nl, &mut Sequence::new(), 2, &cidr),
            Err(Failure::Kernel(Errno::EEXIST))
        );
    }

    #[test]
    fn permission_denied_reaches_the_caller_as_eperm() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(netlink_error(1, -1)));
        let cidr = Ipv4Cidr::parse("192.168.1.50/24").expect("valid");
        assert_eq!(
            assign(&mut nl, &mut Sequence::new(), 2, &cidr),
            Err(Failure::Kernel(Errno::EPERM))
        );
    }

    #[test]
    fn an_address_on_an_interface_that_vanished_is_enodev() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(netlink_error(1, -19)));
        let cidr = Ipv4Cidr::parse("192.168.1.50/24").expect("valid");
        assert_eq!(
            assign(&mut nl, &mut Sequence::new(), 2, &cidr),
            Err(Failure::Kernel(Errno::ENODEV))
        );
    }

    /// A `/0` is a legal prefix and the arithmetic behind its broadcast address
    /// is the shift that would be undefined in C. It must reach the wire rather
    /// than panicking on the way.
    #[test]
    fn a_slash_zero_reaches_the_wire_without_panicking() {
        let sent = sent_message(&assign_ok("10.1.2.3/0"));
        assert_eq!(sent.header[1], 0);
        assert_eq!(
            sent.attrs.last().map(|(k, v)| (*k, v.clone())),
            Some((wire::IFA_BROADCAST, vec![255, 255, 255, 255]))
        );
    }

    /// `Ipv4Addr::octets` is the only conversion in the path, so the whole
    /// address space is cheap to sweep for one that survives the round trip
    /// wrong. This is the test that would have caught an endianness slip on any
    /// address, not just the fixtures above.
    #[test]
    fn every_octet_value_survives_the_encoding() {
        for octet in 0u8..=255 {
            let addr = Ipv4Addr::new(octet, 0, 0, 1);
            let cidr = Ipv4Cidr::new(addr, 24).expect("24 is a prefix");
            let mut nl = FakeNetlink::new();
            nl.replies = Script::always(Ok(netlink_error(1, 0)));
            assert_eq!(assign(&mut nl, &mut Sequence::new(), 2, &cidr), Ok(()));
            assert_eq!(sent_message(&nl).attrs[0].1, vec![octet, 0, 0, 1]);
        }
    }

    // -- reading them back ---------------------------------------------------

    use crate::testnetlink::{addr_reply, dump_done};

    /// A dump with `replies` scripted, run to completion.
    fn dumped(replies: Vec<Result<Vec<u8>, Errno>>) -> (FakeNetlink, Assignments) {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(replies, Ok(dump_done(1)));
        let got = dump(&mut nl, &mut Sequence::new()).expect("a terminated dump");
        (nl, got)
    }

    /// **The defect this whole reader exists for**, in the shape it was found:
    /// a board reached over `eth0` reporting no address on `eth0`. The fixture
    /// is the board's own configuration — `192.168.50.66/24`, statically
    /// assigned, `ADR-034` §3.3's "ask the fleet".
    #[test]
    fn the_address_a_board_is_reached_over_comes_back_with_its_prefix() {
        let (_, got) = dumped(vec![Ok(addr_reply(
            1,
            2,
            [192, 168, 50, 66],
            24,
            wire::RT_SCOPE_UNIVERSE,
        ))]);
        assert_eq!(
            got.addresses,
            vec![Assigned {
                index: 2,
                address: Ipv4Addr::new(192, 168, 50, 66),
                prefix_len: 24,
                scope: wire::RT_SCOPE_UNIVERSE,
            }]
        );
        assert!(got.unreadable.is_empty());
    }

    /// The request itself, taken back off the wire: a `GET`, a dump, and the
    /// family filter. `NLM_F_ACK` must NOT be set — asking for both an
    /// acknowledgement and a dump is a request the kernel answers twice.
    #[test]
    fn the_request_is_a_family_filtered_dump_and_asks_for_no_acknowledgement() {
        let (nl, _) = dumped(vec![]);
        let sent = nl.sent.first().expect("one request");
        let parsed = wire::messages(sent).expect("well-formed");
        assert_eq!(parsed[0].kind, wire::RTM_GETADDR);
        assert_eq!(parsed[0].flags, wire::NLM_F_REQUEST | wire::NLM_F_DUMP);
        assert_eq!(parsed[0].flags & wire::NLM_F_ACK, 0);
        assert_eq!(parsed[0].payload.len(), wire::IFADDRMSG_LEN);
        assert_eq!(parsed[0].payload[0], wire::AF_INET);
        assert!(
            parsed[0].payload[1..].iter().all(|b| *b == 0),
            "every field but the family filter is zero"
        );
    }

    /// The decision this module documents at length, made observable. The
    /// fixture sends `IFA_ADDRESS` first, so a reader that took the first
    /// four-byte address attribute it saw would pass every other test here and
    /// report a tunnel's far end as this machine's address.
    #[test]
    fn the_local_address_is_preferred_over_the_peers() {
        let mut header = [0u8; wire::IFADDRMSG_LEN];
        header[0] = wire::AF_INET;
        header[1] = 32;
        header[4..8].copy_from_slice(&7u32.to_ne_bytes());
        let mut m = Message::new(wire::RTM_NEWADDR, wire::NLM_F_MULTI, 1);
        m.push_payload(&header);
        // The peer, then this end — the order a point-to-point link produces.
        m.push_attr(wire::IFA_ADDRESS, &[203, 0, 113, 2]);
        m.push_attr(wire::IFA_LOCAL, &[203, 0, 113, 1]);

        let (_, got) = dumped(vec![Ok(m.finish())]);
        assert_eq!(
            got.addresses.first().map(|a| a.address),
            Some(Ipv4Addr::new(203, 0, 113, 1)),
            "IFA_LOCAL is this end; IFA_ADDRESS is the peer's"
        );
    }

    /// An address message with only `IFA_ADDRESS`, which is what the kernel
    /// sends on some interfaces. It must still be read.
    #[test]
    fn an_address_with_no_local_attribute_falls_back_to_the_address_attribute() {
        let mut header = [0u8; wire::IFADDRMSG_LEN];
        header[0] = wire::AF_INET;
        header[1] = 24;
        header[4..8].copy_from_slice(&2u32.to_ne_bytes());
        let mut m = Message::new(wire::RTM_NEWADDR, wire::NLM_F_MULTI, 1);
        m.push_payload(&header);
        m.push_attr(wire::IFA_ADDRESS, &[10, 1, 2, 3]);

        let (_, got) = dumped(vec![Ok(m.finish())]);
        assert_eq!(
            got.addresses.first().map(|a| a.address),
            Some(Ipv4Addr::new(10, 1, 2, 3))
        );
    }

    /// Two addresses on one interface — a primary and a secondary — must both
    /// arrive, **in the kernel's order**. The primary is the one a reader acts
    /// on, and sorting would lose which it was.
    #[test]
    fn a_secondary_address_arrives_after_the_primary_and_neither_is_dropped() {
        let (_, got) = dumped(vec![
            Ok(addr_reply(1, 2, [192, 168, 50, 66], 24, wire::RT_SCOPE_UNIVERSE)),
            Ok(addr_reply(1, 2, [192, 168, 50, 67], 24, wire::RT_SCOPE_UNIVERSE)),
        ]);
        assert_eq!(
            got.addresses.iter().map(|a| a.address).collect::<Vec<_>>(),
            vec![
                Ipv4Addr::new(192, 168, 50, 66),
                Ipv4Addr::new(192, 168, 50, 67)
            ]
        );
    }

    /// The filter is a request, not a guarantee. An IPv6 message that arrived
    /// anyway must be skipped rather than have its first four bytes read as an
    /// IPv4 address — which would produce a number that looks like one.
    #[test]
    fn an_address_of_another_family_is_not_read_as_four_bytes_of_ipv4() {
        let mut header = [0u8; wire::IFADDRMSG_LEN];
        header[0] = 10; // AF_INET6
        header[1] = 64;
        header[4..8].copy_from_slice(&2u32.to_ne_bytes());
        let mut m = Message::new(wire::RTM_NEWADDR, wire::NLM_F_MULTI, 1);
        m.push_payload(&header);
        m.push_attr(wire::IFA_ADDRESS, &[0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);

        let (_, got) = dumped(vec![Ok(m.finish())]);
        assert!(got.addresses.is_empty());
        assert!(got.unreadable.is_empty(), "skipped, not a fault");
    }

    /// **Counted, not dropped.** An address message with no address in it is
    /// the one case where silence would say "this interface has none".
    #[test]
    fn an_address_message_carrying_no_address_is_reported_rather_than_dropped() {
        let mut header = [0u8; wire::IFADDRMSG_LEN];
        header[0] = wire::AF_INET;
        header[4..8].copy_from_slice(&9u32.to_ne_bytes());
        let mut m = Message::new(wire::RTM_NEWADDR, wire::NLM_F_MULTI, 1);
        m.push_payload(&header);
        m.push_attr(wire::IFA_BROADCAST, &[10, 0, 0, 255]);

        let (_, got) = dumped(vec![Ok(m.finish())]);
        assert!(got.addresses.is_empty());
        assert_eq!(got.unreadable.len(), 1);
        assert_eq!(got.unreadable[0].index, Some(9));
    }

    /// A machine with no IPv4 address is a real state — `ADR-028` §5.5.5's
    /// image before `rivet-netd` runs — and it is an empty list, not a fault.
    #[test]
    fn a_machine_with_no_addresses_answers_an_empty_list_without_failing() {
        let (_, got) = dumped(vec![]);
        assert!(got.addresses.is_empty() && got.unreadable.is_empty());
    }

    /// A refusal must not be reported as "no addresses". This is the whole
    /// difference between the gap and the lie.
    #[test]
    fn a_kernel_refusal_is_an_error_and_never_an_empty_list() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(netlink_error(1, -1)));
        assert_eq!(
            dump(&mut nl, &mut Sequence::new()),
            Err(Failure::Kernel(Errno::EPERM))
        );
    }

    /// The same failure, reached through a dump that never settles: this
    /// reader must see `crate::port::dump`'s refusal rather than convergence
    /// planning against however many addresses arrived before the kernel gave
    /// up on the snapshot.
    #[test]
    fn an_interrupted_dump_is_an_error_and_never_a_partial_list() {
        use crate::testnetlink::mark_interrupted;

        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(mark_interrupted(dump_done(1))));
        assert_eq!(
            dump(&mut nl, &mut Sequence::new()),
            Err(Failure::Interrupted)
        );
    }

    /// **A deletion names the interface and this end's address, and nothing
    /// else the kernel would match on.** `IFA_ADDRESS` is the peer on a
    /// point-to-point link, so it is absent.
    #[test]
    fn a_removal_is_a_deladdr_naming_the_interface_and_the_local_address() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(netlink_error(1, 0)));
        let assigned = Assigned {
            index: 2,
            address: Ipv4Addr::new(192, 168, 50, 66),
            prefix_len: 24,
            scope: wire::RT_SCOPE_UNIVERSE,
        };
        assert_eq!(remove(&mut nl, &mut Sequence::new(), &assigned), Ok(()));
        let sent = sent_message(&nl);
        assert_eq!(sent.kind, wire::RTM_DELADDR);
        assert_eq!(sent.flags, wire::NLM_F_REQUEST | wire::NLM_F_ACK);
        assert_eq!(sent.header[0], wire::AF_INET);
        assert_eq!(sent.header[1], 24);
        assert_eq!(u32::from_ne_bytes(sent.header[4..8].try_into().unwrap()), 2);
        assert_eq!(sent.attrs, vec![(wire::IFA_LOCAL, vec![192, 168, 50, 66])]);
    }

    /// `ifa_scope` is not `/proc/net/if_inet6`'s scope, and the numbers that
    /// collide are the interesting ones: `0` is `global` in both, and `254` is
    /// `host` here where `0x10` is `host` there.
    #[test]
    fn the_scope_words_are_the_routing_tables_and_not_the_inet6_files() {
        assert_eq!(scope_name(wire::RT_SCOPE_UNIVERSE), Some("global"));
        assert_eq!(scope_name(wire::RT_SCOPE_LINK), Some("link"));
        assert_eq!(scope_name(wire::RT_SCOPE_HOST), Some("host"));
        assert_eq!(scope_name(wire::RT_SCOPE_SITE), Some("site"));
        // `0x20` is `link` in an if_inet6 line and is nothing here. It must
        // come back unnamed rather than be mapped to the nearest word.
        assert_eq!(scope_name(0x20), None);
    }

    /// The two halves of this module driven against each other: what
    /// [`assign`] writes is what [`dump`] reads. It proves the offsets agree
    /// and nothing about the kernel, which is exactly what a round trip is
    /// worth (`AGENTS.md` §4) — the reading against real hardware is in the
    /// report that introduced this change.
    #[test]
    fn the_eight_bytes_this_module_writes_are_the_eight_bytes_it_reads() {
        let cidr = Ipv4Cidr::parse("192.168.50.66/24").expect("a valid fixture");
        let nl = assign_ok("192.168.50.66/24");
        let sent = nl.sent.first().expect("one request");
        let parsed = wire::messages(sent).expect("well-formed");
        let header = wire::parse_ifaddr(parsed[0].payload).expect("the header assign wrote");
        assert_eq!(header.family, wire::AF_INET);
        assert_eq!(header.prefix_len, cidr.prefix_len());
        assert_eq!(header.index, 2);
        assert_eq!(header.scope, wire::RT_SCOPE_UNIVERSE);
    }
}
