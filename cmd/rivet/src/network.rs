//! Reading this machine's networking — `ADR-035` phase 4.
//!
//! **`/sys/class/net/*`, `/proc/net/*`, the resolver file — and, for IPv4
//! addresses only, one netlink dump.** `crate::journal`'s note carries why the
//! first three belong here rather than behind `BootMedium`: no `mount(2)`, no
//! device tree, no pid 1.
//!
//! # ~~Why not netlink, when `rivet-adapter-net` already speaks it~~
//!
//! ~~That crate is a *writer*: it brings a link up, assigns an address and adds
//! a route (`ADR-014` §2). It has no dump operation, and adding `RTM_GETADDR`
//! to it would be a change to a crate this work does not otherwise touch — and
//! would link a netlink socket into the login shell, which `Cargo.toml`'s own
//! entries argue against at length for `ADR-002` §5.1's reason.~~
//!
//! ~~**The cost of that decision is one real gap and it is reported rather than
//! hidden:** IPv4 addresses are in neither `/proc` nor `/sys`.~~
//!
//! **Overturned 2026-08-31 by what the decision actually cost on a board**
//! (`AGENTS.md` §2 rule 3 — struck rather than edited, because the reasoning
//! above is sound and the conclusion is still wrong). `rivet get network` on
//! `rivet-01` answered `"addresses":[]` for `eth0` — **the interface the
//! question had just arrived over**, statically configured `192.168.50.66/24`
//! in that machine's own document. `unmeasured` said so in capitals and the
//! answer was still one nobody could act on, and `ADR-034` §3.3 promises a lost
//! fleet register can be rebuilt *"by asking the fleet"*: a machine that cannot
//! say what address it has cannot honour that.
//!
//! What is true and unchanged is the *premise*: IPv4 addresses are in neither
//! `/proc` nor `/sys`. The kernel exposes IPv6 addresses at
//! `/proc/net/if_inet6` and has never had an equivalent for IPv4 —
//! `/proc/net/fib_trie` is a debug dump of a data structure, not an interface.
//! There is no third option. So `rivet_adapter_net::address::dump` was written,
//! that crate's own tripwire was checked rather than stepped over (its module
//! documentation records the check), and this binary links it.
//!
//! **What that costs, said plainly, because `Cargo.toml` argues every entry
//! here:** one more workspace crate in the login shell, whose only dependencies
//! are `rivet-core` and the `libc` this binary already links. It opens an
//! `AF_NETLINK` socket, sends one request, reads the answer and closes it —
//! **only when `get network` is run**, never at startup, and it needs no
//! privilege, because reading the address list is not `CAP_NET_ADMIN`'s
//! business. The `unsafe` is inside that crate's `socket` module, behind its
//! port trait, and none of it is new to the image: `rivet-netd` already ships it.
//!
//! **The gap did not disappear, it moved.** On any platform that is not Linux
//! there is no netlink, and on Linux the socket can still be refused by a
//! sandbox — so the `unmeasured` entry is now written from *what actually went
//! wrong*, and appears only when something did.
//!
//! # Hexadecimal, little endian, and the trap in `/proc/net/route`
//!
//! `/proc/net/route` prints addresses as 32-bit words in **host byte order**,
//! hex, so `0102A8C0` is `192.168.2.1` — the bytes come out backwards from the
//! way the address is written. Getting that wrong produces addresses that look
//! plausible and are reversed, which is the class of failure `AGENTS.md` §5 is
//! about. [`ipv4_of_hex`] does it in one place and there is a test with a real
//! address in it.

use rivet_core::operation::machine::{
    Interface, InterfaceAddress, InterfaceCounters, MachineError, NetworkRead, Route, Unmeasured,
    UnreadableInterface,
};

/// One directory per interface.
const CLASS_NET: &str = "/sys/class/net";
/// The IPv4 routing table, in hex.
const ROUTE: &str = "/proc/net/route";
/// Every IPv6 address on the machine, with its interface and prefix.
const IF_INET6: &str = "/proc/net/if_inet6";
/// The resolver list. A *view* on a Rivet (`ADR-007` §3.6.2) and a real file
/// elsewhere; either way it is what a resolver would read.
const RESOLV_CONF: &str = "/etc/resolv.conf";

/// ~~**The one gap this build has, stated where an operator will see it.**~~
///
/// ~~"…an IPv4 address is reachable only over netlink (RTM_GETADDR), which this
/// build does not speak."~~ **This build speaks it since 2026-08-31.** The
/// constant is kept, with its first clause intact, because the *premise* is
/// still the reason there is nothing to fall back to when netlink is
/// unavailable — [`ipv4_gap`] prefixes it with whatever actually failed.
const NO_IPV4_ADDRESSES: &str =
    "the kernel exposes IPv6 addresses at /proc/net/if_inet6 and has never had an equivalent for \
     IPv4 — /proc/net/fib_trie is a dump of a kernel data structure rather than an interface — so \
     an IPv4 address is reachable only over netlink (RTM_GETADDR) and there is no file to read \
     instead. THE ABSENCE OF AN IPv4 ADDRESS BELOW IS NOT EVIDENCE THAT THIS MACHINE HAS NONE.";

/// The `unmeasured` entry for an IPv4 read that did not happen.
///
/// **Written from what went wrong, never from a standing assumption.** Until
/// this change the same sentence was emitted on every reading, including the
/// ones where the machine plainly had an address — which is how a truthful gap
/// becomes wallpaper that an operator learns to scroll past.
fn ipv4_gap(what_failed: &str) -> Unmeasured {
    Unmeasured {
        what: "IPv4 addresses".to_string(),
        why: format!("{what_failed}. {NO_IPV4_ADDRESSES}"),
    }
}

/// Every IPv4 address on this machine, by the interface index it is on.
///
/// `Err` carries what failed, in the form [`ipv4_gap`] wants. **A machine with
/// no IPv4 address is `Ok` with an empty list**, and the difference between
/// that and a refusal is the whole reason this returns a `Result` rather than a
/// list: they are not the same answer and must not render as the same absence.
///
/// The second half of the pair is address messages that arrived and could not
/// be read — carried out so they land in `unmeasured` rather than being
/// dropped, because a dropped address reads as an interface that has none.
#[cfg(target_os = "linux")]
fn ipv4_addresses() -> Result<(Vec<(u32, InterfaceAddress)>, Vec<String>), String> {
    use rivet_adapter_net::address;
    use rivet_adapter_net::port::Sequence;
    use rivet_adapter_net::socket::NetlinkSocket;

    let mut netlink = NetlinkSocket::open()
        .map_err(|errno| format!("an AF_NETLINK socket could not be opened: {errno}"))?;
    let dumped = address::dump(&mut netlink, &mut Sequence::new())
        .map_err(|failure| format!("RTM_GETADDR was not answered: {failure}"))?;

    let addresses = dumped
        .addresses
        .iter()
        .map(|a| {
            (
                a.index,
                InterfaceAddress {
                    address: a.address.to_string(),
                    prefix_length: u32::from(a.prefix_len),
                    family: "inet".to_string(),
                    // The number where this build has no word for it, rather
                    // than the nearest word — `AGENTS.md` §5, and the same rule
                    // [`scope_name`] follows for the if_inet6 table, whose
                    // numbers these are NOT.
                    scope: Some(
                        address::scope_name(a.scope)
                            .map_or_else(|| a.scope.to_string(), str::to_string),
                    ),
                },
            )
        })
        .collect();
    let unreadable = dumped
        .unreadable
        .iter()
        .map(|u| match u.index {
            Some(index) => format!("on interface index {index}: {}", u.why),
            None => u.why.clone(),
        })
        .collect();
    Ok((addresses, unreadable))
}

/// The same question on a platform that has no netlink to ask.
///
/// **Refuses rather than answering an empty list** (`AGENTS.md` §3): a
/// development host has addresses, and reporting none would be the same lie
/// this change exists to remove, told about a different machine.
#[cfg(not(target_os = "linux"))]
fn ipv4_addresses() -> Result<(Vec<(u32, InterfaceAddress)>, Vec<String>), String> {
    Err("netlink is a Linux interface and this build is not running on Linux".to_string())
}

/// This machine's interfaces, addresses, routes and resolvers.
///
/// # Errors
///
/// If `/sys/class/net` cannot be listed, which on a running Linux is a fault
/// and on any other platform means there is no such thing. **Nothing else here
/// is fatal**: routes, addresses and resolvers each degrade into
/// [`NetworkRead::unmeasured`] with the reason.
pub fn read() -> Result<NetworkRead, MachineError> {
    let entries = std::fs::read_dir(CLASS_NET).map_err(|e| {
        MachineError::Refused(format!(
            "{CLASS_NET} could not be listed: {e}. On Linux that is a fault; on any other \
             platform there is no such directory and nothing about this machine's networking was \
             measured."
        ))
    })?;

    let mut unmeasured = Vec::new();

    let ipv4 = match ipv4_addresses() {
        Ok((addresses, unreadable)) => {
            for why in unreadable {
                // **A separate entry per unreadable message, not a count.**
                // "one address could not be read" and *which* interface it was
                // on are different facts, and only the second is actionable.
                unmeasured.push(Unmeasured {
                    what: "one IPv4 address".to_string(),
                    why,
                });
            }
            addresses
        }
        Err(what_failed) => {
            unmeasured.push(ipv4_gap(&what_failed));
            Vec::new()
        }
    };

    let addresses = match std::fs::read_to_string(IF_INET6) {
        Ok(text) => ipv6_addresses(&text),
        Err(e) => {
            unmeasured.push(Unmeasured {
                what: "IPv6 addresses".to_string(),
                why: format!("{IF_INET6} could not be read: {e}"),
            });
            Vec::new()
        }
    };

    let mut interfaces = Vec::new();
    let mut interfaces_unreadable = Vec::new();
    for entry in entries.flatten() {
        let file = entry.file_name();
        let Some(name) = file.to_str() else { continue };
        match interface(name, &ipv4, &addresses) {
            Ok(one) => interfaces.push(one),
            // **Counted, not dropped**: an answer that omits an interface says
            // the machine does not have one.
            Err(why) => interfaces_unreadable.push(UnreadableInterface {
                name: name.to_string(),
                why,
            }),
        }
    }
    // Name order, so two readings of an unchanged machine produce the same
    // bytes — `crate::process`'s rule, and a directory's own order is nobody's.
    interfaces.sort_by(|a, b| a.name.cmp(&b.name));
    interfaces_unreadable.sort_by(|a, b| a.name.cmp(&b.name));

    let routes = match std::fs::read_to_string(ROUTE) {
        Ok(text) => ipv4_routes(&text),
        Err(e) => {
            unmeasured.push(Unmeasured {
                what: "IPv4 routes".to_string(),
                why: format!("{ROUTE} could not be read: {e}"),
            });
            Vec::new()
        }
    };
    // **Stated even when everything else worked.** `/proc/net/ipv6_route` is a
    // different format with different semantics, and this build does not parse
    // it — so an all-IPv4 route table must not read as the whole table.
    unmeasured.push(Unmeasured {
        what: "IPv6 routes".to_string(),
        why: "this build reads /proc/net/route, which is the IPv4 table. /proc/net/ipv6_route is \
              a separate file in a separate format and is not parsed here, so the routes below \
              are IPv4 ONLY and their absence for IPv6 says nothing."
            .to_string(),
    });

    let nameservers = match std::fs::read_to_string(RESOLV_CONF) {
        Ok(text) => nameservers_of(&text),
        Err(e) => {
            unmeasured.push(Unmeasured {
                what: "resolvers".to_string(),
                why: format!("{RESOLV_CONF} could not be read: {e}"),
            });
            Vec::new()
        }
    };

    Ok(NetworkRead {
        interfaces,
        interfaces_unreadable,
        interface_source: CLASS_NET.to_string(),
        routes,
        route_source: ROUTE.to_string(),
        nameservers,
        nameserver_source: RESOLV_CONF.to_string(),
        unmeasured,
    })
}

/// One interface out of its `/sys/class/net` directory.
///
/// `operstate` is the one that must be there: a directory under
/// `/sys/class/net` without it is not an interface, and reporting `down` would
/// be a claim rather than a reading.
///
/// The two address lists are keyed differently and that is the kernel's doing
/// rather than an inconsistency here: `/proc/net/if_inet6` names the *device*
/// in its last column, and a netlink address message carries only `ifa_index`.
/// `/sys/class/net/<name>/ifindex` is the join, and it is read here — where the
/// name is already in hand — rather than by dumping links as well.
fn interface(
    name: &str,
    ipv4: &[(u32, InterfaceAddress)],
    addresses: &[(String, InterfaceAddress)],
) -> Result<Interface, String> {
    let dir = format!("{CLASS_NET}/{name}");
    let text = |leaf: &str| -> Option<String> {
        std::fs::read_to_string(format!("{dir}/{leaf}"))
            .ok()
            .map(|s| s.trim().to_string())
    };
    let number = |leaf: &str| -> Option<u64> { text(leaf)?.parse().ok() };

    let operational_state = text("operstate")
        .ok_or_else(|| format!("{dir}/operstate could not be read, so this is not an interface"))?;
    // Read once. `None` is absent only on something that is not an interface,
    // which `operstate` has already ruled out — and it must then match no
    // address rather than every address.
    let ifindex = number("ifindex");

    Ok(Interface {
        name: name.to_string(),
        operational_state,
        // **`None` and not `false`.** The kernel returns `EINVAL` for this file
        // on an administratively-down interface, and `false` would say the
        // cable is out about one nobody brought up.
        carrier: number("carrier").map(|n| n != 0),
        hardware_address: text("address").filter(|a| !a.is_empty()),
        mtu: number("mtu").and_then(|n| u32::try_from(n).ok()),
        // The kernel spells *"I do not know"* as `-1`, which does not parse as
        // an unsigned number — so the parse failing IS the answer here, and
        // carrying a `-1` outward would have made it a speed.
        speed_megabits: number("speed"),
        // **IPv4 first, then IPv6, each in the order its source listed them.**
        // Neither is sorted. The kernel's dump order puts an interface's
        // primary address before its secondaries, and that is a fact about the
        // machine rather than an accident of iteration — sorting would produce
        // stabler bytes and throw it away. `/sys/class/net` gets sorted (see
        // `read`) precisely because a directory's order is nobody's.
        addresses: ipv4
            .iter()
            .filter(|(index, _)| ifindex.is_some_and(|n| n == u64::from(*index)))
            .map(|(_, address)| address.clone())
            .chain(
                addresses
                    .iter()
                    .filter(|(on, _)| on == name)
                    .map(|(_, address)| address.clone()),
            )
            .collect(),
        counters: counters(&dir),
    })
}

/// An interface's counters, or `None` where the machine keeps none.
///
/// **All eight or nothing.** A partial set would invite a reader to compare a
/// present error count against an absent packet count, and the ratio is the
/// only thing either number is good for.
fn counters(dir: &str) -> Option<InterfaceCounters> {
    let count = |leaf: &str| -> Option<u64> {
        std::fs::read_to_string(format!("{dir}/statistics/{leaf}"))
            .ok()?
            .trim()
            .parse()
            .ok()
    };
    Some(InterfaceCounters {
        received_bytes: count("rx_bytes")?,
        transmitted_bytes: count("tx_bytes")?,
        received_packets: count("rx_packets")?,
        transmitted_packets: count("tx_packets")?,
        received_errors: count("rx_errors")?,
        transmitted_errors: count("tx_errors")?,
        received_dropped: count("rx_dropped")?,
        transmitted_dropped: count("tx_dropped")?,
    })
}

/// Every IPv6 address, paired with the interface it is on.
///
/// `/proc/net/if_inet6` is six whitespace-separated columns: the address as 32
/// hex digits with no colons, the interface index, the prefix length in hex,
/// the scope in hex, the flags, and the device name.
fn ipv6_addresses(text: &str) -> Vec<(String, InterfaceAddress)> {
    text.lines()
        .filter_map(|line| {
            let mut column = line.split_whitespace();
            let packed = column.next()?;
            let _index = column.next()?;
            let prefix_length = u32::from_str_radix(column.next()?, 16).ok()?;
            let scope = column.next()?;
            let _flags = column.next()?;
            let device = column.next()?;
            Some((
                device.to_string(),
                InterfaceAddress {
                    address: ipv6_of_hex(packed)?,
                    prefix_length,
                    family: "inet6".to_string(),
                    scope: Some(scope_name(scope).to_string()),
                },
            ))
        })
        .collect()
}

/// 32 hex digits as `fe80:0000:…:0001`.
///
/// **Not compressed to `::`.** A canonical rendering is a second implementation
/// of a fiddly rule (RFC 5952 has a whole section on where the `::` may go), and
/// an address written out in full is unambiguous and comparable by string. It is
/// longer to read and it is never wrong.
fn ipv6_of_hex(packed: &str) -> Option<String> {
    if packed.len() != 32 || !packed.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let groups: Vec<&str> = (0..8)
        .filter_map(|n| packed.get(n * 4..n * 4 + 4))
        .collect();
    (groups.len() == 8).then(|| groups.join(":"))
}

/// The kernel's scope number as the word `ip` would print.
///
/// Anything this build does not know is returned **as the number the machine
/// gave**, rather than mapped onto the nearest word (`AGENTS.md` §5).
fn scope_name(hex: &str) -> &str {
    match hex {
        "00" => "global",
        "10" => "host",
        "20" => "link",
        "40" => "site",
        other => other,
    }
}

/// The IPv4 routing table.
///
/// `/proc/net/route` is a header line and then eleven tab-separated columns:
/// interface, destination, gateway, flags, refcount, use, metric, mask, mtu,
/// window, irtt — with every address a hex word in **host byte order**.
fn ipv4_routes(text: &str) -> Vec<Route> {
    text.lines()
        .skip(1) // the header
        .filter_map(|line| {
            let mut column = line.split_whitespace();
            let interface = column.next()?;
            let destination = ipv4_of_hex(column.next()?)?;
            let gateway = ipv4_of_hex(column.next()?)?;
            let _flags = column.next()?;
            let _refcount = column.next()?;
            let _used = column.next()?;
            let metric = column.next()?.parse().ok()?;
            let mask = ipv4_of_hex(column.next()?)?;
            Some(Route {
                destination,
                prefix_length: prefix_of_mask(&mask),
                // **`0.0.0.0` means directly connected**, and `None` says that
                // rather than reporting an address that is not a router.
                gateway: (gateway != "0.0.0.0").then_some(gateway),
                interface: interface.to_string(),
                metric,
                family: "inet".to_string(),
            })
        })
        .collect()
}

/// A `/proc/net` hex word as dotted quad.
///
/// **Host byte order, so the bytes come out backwards from the way the address
/// is written.** `0102A8C0` is `192.168.2.1`. Getting this the other way round
/// produces addresses that look plausible and are reversed.
fn ipv4_of_hex(word: &str) -> Option<String> {
    let raw = u32::from_str_radix(word, 16).ok()?;
    let [a, b, c, d] = raw.to_le_bytes();
    Some(format!("{a}.{b}.{c}.{d}"))
}

/// A dotted-quad netmask as a prefix length.
///
/// Counts set bits rather than matching a table, so a mask that is not
/// contiguous still yields a number instead of nothing — a non-contiguous mask
/// is not a thing Linux produces, and refusing the whole route over one would
/// lose a route that is otherwise fine.
fn prefix_of_mask(mask: &str) -> u32 {
    mask.split('.')
        .filter_map(|octet| octet.parse::<u8>().ok())
        .map(u8::count_ones)
        .sum()
}

/// The resolvers, in order, as `resolv.conf` names them.
///
/// **The keyword is a whole word.** `nameserverX 9.9.9.9` is not a nameserver
/// directive, and an earlier version of this function read it as one — found by
/// running the test rather than by reading the code, which is `crate::memory`'s
/// colon in a third spelling.
fn nameservers_of(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let line = line.split('#').next()?.trim();
            let mut word = line.split_whitespace();
            (word.next()? == "nameserver").then(|| word.next())?
        })
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// **The byte order, against a real address.** `0102A8C0` is `192.168.2.1`
    /// and not `1.2.168.192`; the reversed reading looks plausible, which is
    /// exactly why it needs a test rather than a comment.
    #[test]
    fn a_proc_net_hex_word_is_host_byte_order() {
        assert_eq!(ipv4_of_hex("0102A8C0").as_deref(), Some("192.168.2.1"));
        assert_eq!(ipv4_of_hex("00000000").as_deref(), Some("0.0.0.0"));
        assert_eq!(ipv4_of_hex("0000FEA9").as_deref(), Some("169.254.0.0"));
        assert_eq!(ipv4_of_hex("nonsense"), None);
    }

    /// The default route, as `/proc/net/route` actually prints it — including
    /// the header line, which must not become a route.
    #[test]
    fn the_default_route_and_its_gateway_come_out_of_the_hex_table() {
        let table = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
                     eth0\t00000000\tFE00000A\t0003\t0\t0\t202\t00000000\t0\t0\t0\n\
                     eth0\t0000000A\t00000000\t0001\t0\t0\t202\t00FFFFFF\t0\t0\t0\n";
        let routes = ipv4_routes(table);
        assert_eq!(routes.len(), 2, "the header became a route");

        assert_eq!(routes[0].destination, "0.0.0.0");
        assert_eq!(routes[0].prefix_length, 0, "this is the default route");
        assert_eq!(routes[0].gateway.as_deref(), Some("10.0.0.254"));
        assert_eq!(routes[0].interface, "eth0");
        assert_eq!(routes[0].metric, 202);

        // **A directly connected network has NO gateway**, and `None` says so
        // rather than reporting 0.0.0.0 as if it were a router.
        assert_eq!(routes[1].destination, "10.0.0.0");
        assert_eq!(routes[1].prefix_length, 24);
        assert_eq!(routes[1].gateway, None);
    }

    /// A mask becomes a prefix length by counting bits.
    #[test]
    fn a_netmask_becomes_a_prefix_length() {
        for (mask, prefix) in [
            ("255.255.255.0", 24),
            ("255.255.0.0", 16),
            ("0.0.0.0", 0),
            ("255.255.255.255", 32),
        ] {
            assert_eq!(prefix_of_mask(mask), prefix, "{mask}");
        }
    }

    /// **32 hex digits become eight groups, uncompressed.** The full form is
    /// unambiguous; a `::` rendering is a second implementation of RFC 5952's
    /// rule about where the `::` may go.
    #[test]
    fn an_if_inet6_address_becomes_eight_groups_written_out_in_full() {
        assert_eq!(
            ipv6_of_hex("fe800000000000000a0027fffe8e4d3a").as_deref(),
            Some("fe80:0000:0000:0000:0a00:27ff:fe8e:4d3a")
        );
        assert_eq!(
            ipv6_of_hex("00000000000000000000000000000001").as_deref(),
            Some("0000:0000:0000:0000:0000:0000:0000:0001"),
            "the loopback is written out rather than compressed to ::1"
        );
        assert_eq!(ipv6_of_hex("tooshort"), None);
        assert_eq!(ipv6_of_hex("zzzz0000000000000000000000000001"), None);
    }

    /// The whole `if_inet6` line, including the interface it belongs to and the
    /// prefix length — which is in **hex**, so `40` is 64 and not 40.
    #[test]
    fn an_if_inet6_line_names_its_interface_and_its_prefix_in_hex() {
        let text = "fe800000000000000a0027fffe8e4d3a 02 40 20 80 eth0\n\
                    00000000000000000000000000000001 01 80 10 80 lo\n";
        let found = ipv6_addresses(text);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].0, "eth0");
        assert_eq!(found[0].1.prefix_length, 64, "0x40 is 64, not 40");
        assert_eq!(found[0].1.family, "inet6");
        assert_eq!(found[0].1.scope.as_deref(), Some("link"));
        assert_eq!(found[1].0, "lo");
        assert_eq!(found[1].1.prefix_length, 128);
        assert_eq!(found[1].1.scope.as_deref(), Some("host"));
    }

    /// A scope this build does not know comes out as the machine's own number
    /// rather than being mapped onto the nearest word it does know.
    #[test]
    fn an_unknown_scope_is_reported_as_the_number_the_machine_gave() {
        assert_eq!(scope_name("05"), "05");
        assert_eq!(scope_name("20"), "link");
    }

    /// Resolvers in order, comments ignored, other directives ignored.
    #[test]
    fn the_nameservers_come_out_in_order_and_nothing_else_does() {
        let text = "# generated\nsearch example\nnameserver 10.0.0.254\noptions ndots:1\n\
                    nameserver 1.1.1.1 # fallback\nnameserverX 9.9.9.9\nnameserver\n";
        assert_eq!(
            nameservers_of(text),
            vec!["10.0.0.254".to_string(), "1.1.1.1".to_string()],
            "`nameserverX` is not the keyword and a bare `nameserver` names nothing"
        );
    }

    /// **The IPv4 gap is stated in words that refuse the inference**, because
    /// an answer with no IPv4 address in it otherwise reads as a machine that
    /// has none — on a board reached over IPv4.
    ///
    /// The entry is now conditional, so what is checked is the *sentence it is
    /// built from* plus the fact that the reason gets prefixed onto it.
    #[test]
    fn the_ipv4_address_gap_refuses_the_inference_explicitly() {
        assert!(NO_IPV4_ADDRESSES.contains("netlink"), "{NO_IPV4_ADDRESSES}");
        assert!(
            NO_IPV4_ADDRESSES.contains("NOT EVIDENCE"),
            "{NO_IPV4_ADDRESSES}"
        );
        let gap = ipv4_gap("the socket was refused");
        assert_eq!(gap.what, "IPv4 addresses");
        assert!(gap.why.starts_with("the socket was refused"), "{}", gap.why);
        assert!(gap.why.contains("NOT EVIDENCE"), "{}", gap.why);
    }

    /// **The read either answers or refuses, and it never reports an unmeasured
    /// IPv4 read that in fact happened.** Runs on whatever host the suite is
    /// on, and the two hosts answer differently on purpose:
    ///
    /// * **Under Linux** — the container `ADR-026` §7 made the build host — the
    ///   netlink dump works, so there must be NO `IPv4 addresses` gap. Before
    ///   2026-08-31 this assertion was the opposite one, and it passed.
    /// * **On the Windows development host** there is no netlink, so the gap
    ///   must be reported, with the platform named in it.
    ///
    /// Either way `IPv6 routes` is unconditional: `/proc/net/ipv6_route` is a
    /// separate format and is still not parsed.
    #[test]
    fn a_reading_on_this_host_always_names_what_it_could_not_measure() {
        match read() {
            Ok(reading) => {
                assert_eq!(reading.interface_source, CLASS_NET);
                let ipv4_gap = reading
                    .unmeasured
                    .iter()
                    .find(|u| u.what == "IPv4 addresses");
                if cfg!(target_os = "linux") {
                    assert!(
                        ipv4_gap.is_none(),
                        "a Linux host read its IPv4 addresses and still declared the gap: {:?}",
                        ipv4_gap.map(|u| &u.why)
                    );
                } else {
                    let why = &ipv4_gap.expect("no netlink off Linux, so the gap is real").why;
                    assert!(why.contains("Linux"), "{why}");
                }
                assert!(
                    reading.unmeasured.iter().any(|u| u.what.contains("IPv6 routes")),
                    "an IPv4-only route table was reported as the whole table"
                );
            }
            Err(refusal) => assert!(refusal.why().contains(CLASS_NET), "{}", refusal.why()),
        }
    }

    /// **The address of the interface this suite's host is reached over.**
    ///
    /// The defect in one assertion: a Linux host with a route out has an IPv4
    /// address on the interface that route names, and before 2026-08-31 this
    /// reading had none. It is skipped off Linux and on a host with no default
    /// route — reported rather than asserted away, because a container with no
    /// network is a real environment and a test that failed there would be
    /// reporting the environment (`crate::socket`'s own convention).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_linux_host_reports_an_ipv4_address_on_the_interface_its_route_names() {
        let Ok(reading) = read() else {
            eprintln!("/sys/class/net is not readable in this environment");
            return;
        };
        let Some(route) = reading.routes.iter().find(|r| r.prefix_length == 0) else {
            eprintln!("this environment has no default route; nothing to check against");
            return;
        };
        let carrying = reading
            .interfaces
            .iter()
            .find(|i| i.name == route.interface)
            .expect("the interface the default route names is one of the interfaces listed");
        let ipv4: Vec<&InterfaceAddress> =
            carrying.addresses.iter().filter(|a| a.family == "inet").collect();
        assert!(
            !ipv4.is_empty(),
            "{} carries the default route and reports no IPv4 address",
            carrying.name
        );
        for address in ipv4 {
            assert!(
                address.address.split('.').count() == 4,
                "{} is not dotted quad",
                address.address
            );
            assert!(address.prefix_length <= 32, "{}", address.prefix_length);
        }
    }
}
