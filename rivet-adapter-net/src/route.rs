//! Installing the default route.
//!
//! One request, `RTM_NEWROUTE`, and a default route is expressed by what it
//! *omits*: `rtm_dst_len = 0` with no `RTA_DST` attribute is `0.0.0.0/0` — the
//! destination that matches everything. There is no "default" flag; the absence
//! is the meaning.
//!
//! # Ordering, and why this module cannot be run on its own
//!
//! **The address must be assigned before the route is installed.** The kernel
//! decides whether a next hop is reachable by looking for an on-link route to
//! it, and assigning an address is what creates that on-link route. Reversed,
//! this request fails with `ENETUNREACH` on a configuration that is entirely
//! correct. [`crate::wired`] owns the order and is where it is tested;
//! `ADR-004` §2 makes the same point one level up about bring-up order
//! generally.
//!
//! `rivet_core::net::StaticAddressing::validate` refuses a gateway outside
//! the interface's subnet before any of this runs, so `ENETUNREACH` reaching
//! here means the *ordering* went wrong rather than the configuration — which
//! is why the two are distinguishable at all.

use std::net::Ipv4Addr;

use crate::port::{Failure, Sequence, SendRecv, dump as dump_datagrams, expect_ack, transact};
use crate::wire::{self, Malformed, Message};

/// Install `0.0.0.0/0 via gateway dev <index>` in the main table.
///
/// `NLM_F_CREATE | NLM_F_EXCL` for the same reason as
/// [`crate::address::assign`]: an existing default route comes back as `EEXIST`
/// rather than being replaced. Replacing would be worse than it sounds — the
/// route being replaced could be the one carrying the very session that is
/// asking for the change.
pub fn add_default(
    nl: &mut dyn SendRecv,
    seq: &mut Sequence,
    index: i32,
    gateway: Ipv4Addr,
) -> Result<(), Failure> {
    let this_seq = seq.take();
    let mut message = Message::new(
        wire::RTM_NEWROUTE,
        wire::NLM_F_REQUEST | wire::NLM_F_ACK | wire::NLM_F_CREATE | wire::NLM_F_EXCL,
        this_seq,
    );
    message.push_payload(&rtmsg());
    message.push_attr(wire::RTA_GATEWAY, &gateway.octets());
    // RTA_OIF is a u32, not the i32 the link messages use for the same number.
    message.push_attr(
        wire::RTA_OIF,
        &u32::try_from(index).unwrap_or(0).to_ne_bytes(),
    );

    let reply = transact(nl, message.finish())?;
    expect_ack(&reply, this_seq)
}

/// One IPv4 default route in the main table, as the kernel listed it.
///
/// Every field the kernel matches a deletion on is carried, so [`remove`]
/// names exactly the route that was read and not a neighbour of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultRoute {
    /// The interface the route leaves by, from `RTA_OIF`. `None` for a
    /// multipath route, which carries its interfaces elsewhere.
    pub oif: Option<u32>,
    /// The next hop, from `RTA_GATEWAY`. `None` for a route straight onto a
    /// link.
    pub gateway: Option<Ipv4Addr>,
    /// The metric, from `RTA_PRIORITY`. `None` is metric 0.
    pub priority: Option<u32>,
    pub tos: u8,
    /// One of the `RTN_*` types: unicast, blackhole, unreachable.
    pub kind: u8,
}

/// Ask the kernel for every IPv4 default route in the main table.
///
/// `RTM_GETROUTE` with `NLM_F_DUMP`, filtered to [`wire::AF_INET`], and then
/// narrowed here to a zero-length destination in [`wire::RT_TABLE_MAIN`]: the
/// family field of a `GET` is a filter the kernel may ignore, and the dump
/// carries every table, the local table included.
///
/// # Errors
///
/// Anything [`crate::port::dump`] refuses, and **a route message that does not
/// parse**. This list decides what is deleted, so a reply it cannot read is a
/// refusal rather than a route silently left out.
pub fn dump_defaults(
    nl: &mut dyn SendRecv,
    seq: &mut Sequence,
) -> Result<Vec<DefaultRoute>, Failure> {
    let this_seq = seq.take();
    let mut message = Message::new(
        wire::RTM_GETROUTE,
        wire::NLM_F_REQUEST | wire::NLM_F_DUMP,
        this_seq,
    );
    let mut filter = [0u8; wire::RTMSG_LEN];
    filter[0] = wire::AF_INET;
    message.push_payload(&filter);

    let datagrams = dump_datagrams(nl, message.finish(), this_seq)?;

    let mut out = Vec::new();
    for datagram in &datagrams {
        for reply in wire::messages(datagram)? {
            if reply.kind != wire::RTM_NEWROUTE {
                continue;
            }
            let header = reply.payload.get(..wire::RTMSG_LEN).ok_or(Malformed::ShortPayload {
                want: wire::RTMSG_LEN,
                got: reply.payload.len(),
            })?;
            let family = header[0];
            let dst_len = header[1];
            let tos = header[3];
            let kind = header[7];
            if family != wire::AF_INET || dst_len != 0 {
                continue;
            }
            let attributes = wire::attributes(&reply.payload[wire::RTMSG_LEN..])?;
            let u32_of = |kind: u16| -> Option<u32> {
                attributes
                    .iter()
                    .find(|(k, _)| *k == kind)
                    .and_then(|(_, value)| <[u8; 4]>::try_from(*value).ok())
                    .map(u32::from_ne_bytes)
            };
            let table = u32_of(wire::RTA_TABLE).unwrap_or(u32::from(header[4]));
            if table != u32::from(wire::RT_TABLE_MAIN) {
                continue;
            }
            let gateway = attributes
                .iter()
                .find(|(k, _)| *k == wire::RTA_GATEWAY)
                .and_then(|(_, value)| <[u8; 4]>::try_from(*value).ok())
                .map(Ipv4Addr::from);
            out.push(DefaultRoute {
                oif: u32_of(wire::RTA_OIF),
                gateway,
                priority: u32_of(wire::RTA_PRIORITY),
                tos,
                kind,
            });
        }
    }
    Ok(out)
}

/// Withdraw `route` from the main table — `RTM_DELROUTE`.
///
/// The request carries what the kernel matches on: the destination, the type
/// of service, the metric, the interface and the next hop. The protocol is
/// left zero and the scope `RT_SCOPE_NOWHERE`, which is how a deletion says it
/// does not filter on either. A route already gone comes back as `ESRCH`.
///
/// # Errors
///
/// The kernel's refusal, or a socket failure, as [`add_default`]'s.
pub fn remove(
    nl: &mut dyn SendRecv,
    seq: &mut Sequence,
    route: &DefaultRoute,
) -> Result<(), Failure> {
    let this_seq = seq.take();
    let mut message = Message::new(
        wire::RTM_DELROUTE,
        wire::NLM_F_REQUEST | wire::NLM_F_ACK,
        this_seq,
    );
    let mut header = [0u8; wire::RTMSG_LEN];
    header[0] = wire::AF_INET;
    header[3] = route.tos;
    header[4] = wire::RT_TABLE_MAIN;
    header[6] = wire::RT_SCOPE_NOWHERE;
    header[7] = route.kind;
    message.push_payload(&header);
    if let Some(gateway) = route.gateway {
        message.push_attr(wire::RTA_GATEWAY, &gateway.octets());
    }
    if let Some(oif) = route.oif {
        message.push_attr(wire::RTA_OIF, &oif.to_ne_bytes());
    }
    if let Some(priority) = route.priority {
        message.push_attr(wire::RTA_PRIORITY, &priority.to_ne_bytes());
    }

    let reply = transact(nl, message.finish())?;
    expect_ack(&reply, this_seq)
}

/// `struct rtmsg`: family, dst_len, src_len, tos, table, protocol, scope, type,
/// flags.
///
/// Every field is a deliberate value:
///
/// | Field | Value | Because |
/// | :-- | :-- | :-- |
/// | `rtm_family` | `AF_INET` | IPv4. `ADR-023` §7 wants IPv6 covered from the first ruleset; that is the firewall and this is not it — a v6 default route is its own request and is not written yet |
/// | `rtm_dst_len` | 0 | with no `RTA_DST`, this is what makes it *default* |
/// | `rtm_table` | `RT_TABLE_MAIN` | the table `ip route` shows without being asked |
/// | `rtm_protocol` | `RTPROT_STATIC` | "an administrator put this here" — distinguishes it from a route a DHCP client installs, which matters as soon as `ADR-004` §8's `udhcpc` exists |
/// | `rtm_scope` | `RT_SCOPE_UNIVERSE` | the destination is more than one hop away, which is what a gateway route means |
/// | `rtm_type` | `RTN_UNICAST` | not a blackhole, not unreachable, not local |
fn rtmsg() -> [u8; wire::RTMSG_LEN] {
    let mut out = [0u8; wire::RTMSG_LEN];
    out[0] = wire::AF_INET;
    out[1] = 0; // rtm_dst_len
    out[2] = 0; // rtm_src_len
    out[3] = 0; // rtm_tos
    out[4] = wire::RT_TABLE_MAIN;
    out[5] = wire::RTPROT_STATIC;
    out[6] = wire::RT_SCOPE_UNIVERSE;
    out[7] = wire::RTN_UNICAST;
    // rtm_flags stays zero.
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::port::Errno;
    use crate::testnetlink::{FakeNetlink, Script, netlink_error};

    fn add_ok(gateway: [u8; 4], index: i32) -> FakeNetlink {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(netlink_error(1, 0)));
        assert_eq!(
            add_default(
                &mut nl,
                &mut Sequence::new(),
                index,
                Ipv4Addr::from(gateway)
            ),
            Ok(())
        );
        nl
    }

    #[test]
    fn a_default_route_is_a_zero_length_destination_with_no_destination_attribute() {
        let nl = add_ok([192, 168, 1, 1], 2);
        let sent = nl.sent.first().expect("one request");
        let parsed = wire::messages(sent).expect("well-formed");
        let payload = parsed[0].payload;

        assert_eq!(parsed[0].kind, wire::RTM_NEWROUTE);
        assert_eq!(
            parsed[0].flags,
            wire::NLM_F_REQUEST | wire::NLM_F_ACK | wire::NLM_F_CREATE | wire::NLM_F_EXCL
        );
        assert_eq!(payload[0], wire::AF_INET);
        assert_eq!(payload[1], 0, "rtm_dst_len must be zero for a default route");
        assert_eq!(payload[4], wire::RT_TABLE_MAIN);
        assert_eq!(payload[5], wire::RTPROT_STATIC);
        assert_eq!(payload[6], wire::RT_SCOPE_UNIVERSE);
        assert_eq!(payload[7], wire::RTN_UNICAST);

        let attrs = wire::attributes(&payload[wire::RTMSG_LEN..]).expect("well-formed");
        assert_eq!(
            attrs,
            vec![
                (wire::RTA_GATEWAY, &[192, 168, 1, 1][..]),
                (wire::RTA_OIF, &2u32.to_ne_bytes()[..]),
            ]
        );
        // Named explicitly: an RTA_DST here would make this a host route to
        // 0.0.0.0 rather than the default route.
        assert!(!attrs.iter().any(|(k, _)| *k == 1), "RTA_DST must be absent");
    }

    #[test]
    fn the_gateway_bytes_are_in_the_order_the_address_is_written() {
        let nl = add_ok([10, 20, 30, 40], 2);
        let sent = nl.sent.first().expect("one request");
        let parsed = wire::messages(sent).expect("well-formed");
        let attrs = wire::attributes(&parsed[0].payload[wire::RTMSG_LEN..]).expect("well-formed");
        assert_eq!(attrs[0].1, &[10, 20, 30, 40]);
    }

    /// The converging case. A machine that already has a default route must not
    /// have it replaced by a re-run — the route being replaced could be the one
    /// carrying the session asking for the change.
    #[test]
    fn a_route_that_is_already_present_comes_back_as_eexist() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(netlink_error(1, -17)));
        assert_eq!(
            add_default(
                &mut nl,
                &mut Sequence::new(),
                2,
                Ipv4Addr::new(192, 168, 1, 1)
            ),
            Err(Failure::Kernel(Errno::EEXIST))
        );
    }

    /// What the kernel says when the next hop is not on any interface's subnet.
    /// `rivet_core::net` refuses that configuration before the syscall, so
    /// this arriving means the *ordering* went wrong — the route was installed
    /// before the address that makes the gateway on-link.
    #[test]
    fn an_unreachable_gateway_is_enetunreach() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(netlink_error(1, -101)));
        assert_eq!(
            add_default(&mut nl, &mut Sequence::new(), 2, Ipv4Addr::new(10, 9, 9, 9)),
            Err(Failure::Kernel(Errno::ENETUNREACH))
        );
    }

    #[test]
    fn permission_denied_reaches_the_caller_as_eperm() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(netlink_error(1, -1)));
        assert_eq!(
            add_default(&mut nl, &mut Sequence::new(), 2, Ipv4Addr::new(1, 1, 1, 1)),
            Err(Failure::Kernel(Errno::EPERM))
        );
    }

    // -- reading defaults back, and withdrawing one --------------------------

    use crate::testnetlink::{dump_done, route_reply};

    fn dumped(replies: Vec<Result<Vec<u8>, Errno>>) -> (FakeNetlink, Vec<DefaultRoute>) {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(replies, Ok(dump_done(1)));
        let got = dump_defaults(&mut nl, &mut Sequence::new()).expect("a terminated dump");
        (nl, got)
    }

    #[test]
    fn the_dump_request_is_a_family_filtered_getroute_with_no_acknowledgement() {
        let (nl, _) = dumped(vec![]);
        let parsed = wire::messages(&nl.sent[0]).expect("well-formed");
        assert_eq!(parsed[0].kind, wire::RTM_GETROUTE);
        assert_eq!(parsed[0].flags, wire::NLM_F_REQUEST | wire::NLM_F_DUMP);
        assert_eq!(parsed[0].payload.len(), wire::RTMSG_LEN);
        assert_eq!(parsed[0].payload[0], wire::AF_INET);
    }

    #[test]
    fn a_default_route_in_the_main_table_is_read_with_its_gateway_and_interface() {
        let (_, got) = dumped(vec![Ok(route_reply(1, 0, 254, 2, Some([192, 168, 1, 1]), None))]);
        assert_eq!(
            got,
            [DefaultRoute {
                oif: Some(2),
                gateway: Some(Ipv4Addr::new(192, 168, 1, 1)),
                priority: None,
                tos: 0,
                kind: wire::RTN_UNICAST,
            }]
        );
    }

    /// The dump carries every route in every table. Only a zero-length
    /// destination in the main table is a default route this crate acts on.
    #[test]
    fn a_subnet_route_and_another_tables_default_are_not_defaults() {
        let (_, got) = dumped(vec![
            Ok(route_reply(1, 24, 254, 2, None, None)),
            Ok(route_reply(1, 0, 255, 2, Some([10, 0, 0, 1]), None)),
            Ok(route_reply(1, 0, 1000, 2, Some([10, 0, 0, 1]), None)),
        ]);
        assert!(got.is_empty(), "{got:?}");
    }

    /// A reply this list cannot read is a refusal: the list decides what is
    /// deleted, and leaving a route out of it would leave that route in place
    /// with nothing said.
    #[test]
    fn a_route_message_with_a_short_header_is_a_refusal() {
        let mut m = Message::new(wire::RTM_NEWROUTE, wire::NLM_F_MULTI, 1);
        m.push_payload(&[wire::AF_INET, 0, 0]);
        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(vec![Ok(m.finish())], Ok(dump_done(1)));
        assert!(matches!(
            dump_defaults(&mut nl, &mut Sequence::new()),
            Err(Failure::Malformed(_))
        ));
    }

    /// A removal names the route as it was read: table, type, gateway,
    /// interface and metric, with the protocol and scope left unfiltered.
    #[test]
    fn a_removal_is_a_delroute_naming_what_the_kernel_matches_on() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(netlink_error(1, 0)));
        let route = DefaultRoute {
            oif: Some(2),
            gateway: Some(Ipv4Addr::new(192, 168, 1, 1)),
            priority: Some(100),
            tos: 0,
            kind: wire::RTN_UNICAST,
        };
        assert_eq!(remove(&mut nl, &mut Sequence::new(), &route), Ok(()));
        let parsed = wire::messages(&nl.sent[0]).expect("well-formed");
        assert_eq!(parsed[0].kind, wire::RTM_DELROUTE);
        assert_eq!(parsed[0].flags, wire::NLM_F_REQUEST | wire::NLM_F_ACK);
        let payload = parsed[0].payload;
        assert_eq!(payload[0], wire::AF_INET);
        assert_eq!(payload[1], 0, "a default route's destination length");
        assert_eq!(payload[4], wire::RT_TABLE_MAIN);
        assert_eq!(payload[5], 0, "the protocol is not filtered on");
        assert_eq!(payload[6], wire::RT_SCOPE_NOWHERE);
        assert_eq!(payload[7], wire::RTN_UNICAST);
        let attrs = wire::attributes(&payload[wire::RTMSG_LEN..]).expect("well-formed");
        assert_eq!(
            attrs,
            vec![
                (wire::RTA_GATEWAY, &[192, 168, 1, 1][..]),
                (wire::RTA_OIF, &2u32.to_ne_bytes()[..]),
                (wire::RTA_PRIORITY, &100u32.to_ne_bytes()[..]),
            ]
        );
    }

    /// What [`dump_defaults`] reads is what [`remove`] sends back, field for
    /// field. It proves the two agree and nothing about the kernel.
    #[test]
    fn a_route_read_from_the_dump_is_removed_with_the_same_fields() {
        let (_, got) = dumped(vec![Ok(route_reply(
            1,
            0,
            254,
            3,
            Some([10, 0, 0, 1]),
            Some(50),
        ))]);
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(netlink_error(1, 0)));
        remove(&mut nl, &mut Sequence::new(), &got[0]).expect("acknowledged");
        let parsed = wire::messages(&nl.sent[0]).expect("well-formed");
        let attrs = wire::attributes(&parsed[0].payload[wire::RTMSG_LEN..]).expect("ok");
        assert!(attrs.contains(&(wire::RTA_GATEWAY, &[10, 0, 0, 1][..])));
        assert!(attrs.contains(&(wire::RTA_OIF, &3u32.to_ne_bytes()[..])));
        assert!(attrs.contains(&(wire::RTA_PRIORITY, &50u32.to_ne_bytes()[..])));
    }

    #[test]
    fn a_route_already_gone_comes_back_as_esrch() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(netlink_error(1, -3)));
        let route = DefaultRoute {
            oif: Some(2),
            gateway: Some(Ipv4Addr::new(192, 168, 1, 1)),
            priority: None,
            tos: 0,
            kind: wire::RTN_UNICAST,
        };
        assert_eq!(
            remove(&mut nl, &mut Sequence::new(), &route),
            Err(Failure::Kernel(Errno::ESRCH))
        );
    }

    #[test]
    fn a_socket_that_cannot_be_written_to_is_reported_as_the_socket() {
        let mut nl = FakeNetlink::new();
        nl.send_results = Script::always(Err(Errno::ENOBUFS));
        assert_eq!(
            add_default(&mut nl, &mut Sequence::new(), 2, Ipv4Addr::new(1, 1, 1, 1)),
            Err(Failure::Socket(Errno::ENOBUFS))
        );
    }
}
