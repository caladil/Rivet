//! `rivet get network` — **what this machine's networking actually looks like.**
//!
//! `ADR-035` phase 4, the fourth of the five the maintainer named on
//! 2026-08-29, and `docs/design-candidates/Admin-Surface-Survey.md` §3.1.
//!
//! # `rivet-netd` owns this and answers nothing
//!
//! `rivet-adapter-net` brings `bcmgenet` up over netlink, assigns an address and
//! adds a route — that is `PLAN.md` §0's proven path. **Nothing in this system
//! could report what it did.** A board whose network is wrong has, until now,
//! been debugged by reaching the board over the network that is wrong — and
//! `ADR-035`'s control plane is over that same network. A read surface for it is
//! not a convenience.
//!
//! # What to look at first
//!
//! **The counters, and specifically the errors and drops** — survey §7.4 named
//! them and neither review had. A link that is up and carrying errors is a
//! cable or a marginal supply, and it looks exactly like a healthy link in
//! every other reading this system has. `errors` climbing while `packets`
//! climbs is a different fault from `dropped` climbing on its own.
//!
//! **Then `operational_state` against `carrier`.** The kernel's word is carried
//! through unchanged rather than reduced to a boolean, because `unknown` — what
//! a `tun` device reports — is not `down`, and collapsing them loses the one
//! distinction that separates *not configured* from *not connected*.
//!
//! # The gap, stated rather than shaped
//!
//! **IPv4 addresses are in neither `/proc` nor `/sys`.** The kernel exposes IPv6
//! addresses at `/proc/net/if_inet6` and has never had an IPv4 equivalent; the
//! only route is netlink. So this build reports what it has and lists what it
//! did not measure, and `unmeasured` says so in words that refuse the inference.
//!
//! That is the rule [`super::memory`] follows for PSI and [`super::thermal`] for
//! the throttle flags, and here it is at its sharpest: **an answer with no IPv4
//! address in it, on a board reached over IPv4, would be a lie by omission**
//! (`AGENTS.md` §5, §7).

use super::machine::Observed;
use super::{ErrorCode, Field, OperationError};
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "network";

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was read. Always `network` in this build.",
    },
    Field {
        name: "interfaces",
        description: "Every interface, IN NAME ORDER so that two readings of an unchanged \
                      machine produce the same bytes. Each carries `name`, \
                      `operational_state` — the KERNEL'S OWN WORD, not a boolean, because \
                      `unknown` is what a tun device reports and is not `down` — `carrier`, \
                      `hardware_address`, `mtu`, `speed_megabits`, its `addresses`, and \
                      `counters`. `carrier` is ABSENT rather than false on an \
                      administratively-down interface, where the kernel refuses the question: \
                      `false` would say the cable is out about an interface nobody brought up.",
    },
    Field {
        name: "interfaces_unreadable",
        description: "Interfaces that were listed and could not be read, each with what the \
                      system said. NEVER DROPPED: an answer that omitted one would say this \
                      machine has no such interface.",
    },
    Field {
        name: "up",
        description: "How many interfaces report an operational state of `up`. Computed here so \
                      that the commonest question — is anything actually connected — does not \
                      depend on every caller knowing to count.",
    },
    Field {
        name: "routes",
        description: "The routing table, in the order the machine listed it. Each carries \
                      `destination`, `prefix_length`, `gateway`, `interface`, `metric` and \
                      `family`. A destination of `0.0.0.0` with a zero prefix IS the default \
                      route, kept as the machine stated it. `gateway` is ABSENT for a directly \
                      connected network rather than reported as `0.0.0.0`, which is not a \
                      router. READ `unmeasured` BEFORE CONCLUDING ANYTHING FROM AN EMPTY \
                      TABLE.",
    },
    Field {
        name: "routes_read_from",
        description: "Where the routing table came from.",
    },
    Field {
        name: "nameservers",
        description: "The resolvers this machine would ask, IN ORDER, verbatim. Empty is a real \
                      answer on a machine that resolves nothing — and see `unmeasured` for the \
                      case where the file could not be read, which is a different thing.",
    },
    Field {
        name: "nameservers_read_from",
        description: "Where the resolver list came from. On a Rivet this is a generated VIEW \
                      (ADR-007 §3.6.2) rather than a file anyone edits, so it says what the \
                      document produced.",
    },
    Field {
        name: "unmeasured",
        description: "EVERYTHING THIS READING DID NOT ESTABLISH, and why — each entry a `what` \
                      and a `why`. THE MOST IMPORTANT FIELD HERE. IPv4 addresses are in neither \
                      /proc nor /sys and are reachable only over netlink, so a machine with an \
                      IPv4 address may show none above; an answer that stayed silent about that \
                      would read as `this machine has no IPv4 address` on a board you reached \
                      over IPv4. Same rule `get memory` follows for pressure and `get thermal` \
                      for the throttle flags.",
    },
    Field {
        name: "read_from",
        description: "Where the interfaces were looked for.",
    },
    Field {
        name: "means",
        description: "What this reading does and does not assert, in prose — including what was \
                      not measured, and what a climbing error counter on an interface that is \
                      otherwise healthy would mean.",
    },
];

/// Perform it.
///
/// # Errors
///
/// If a second word was given, or the machine's interface directory could not
/// be listed. **A route table or a resolver file that could not be read is not
/// an error** — it lands in `unmeasured`.
pub(super) fn perform(
    arguments: &[&str],
    observed: &mut dyn Observed,
) -> Result<Value, OperationError> {
    if let Some(extra) = arguments.get(1) {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "`{SUBJECT}` takes no second word and {extra:?} is one. The whole reading is the \
                 answer; select one interface from it with a JSON tool rather than with an \
                 argument this build does not have."
            ),
        ));
    }

    let read = observed.read_network().map_err(|e| {
        OperationError::about(ErrorCode::MachineRefused, SUBJECT, e.why().to_string())
    })?;

    let number = |n: u64| Value::Integer(i64::try_from(n).unwrap_or(i64::MAX));
    let up = read
        .interfaces
        .iter()
        .filter(|i| i.operational_state == UP)
        .count();

    let interfaces: Vec<Value> = read
        .interfaces
        .iter()
        .map(|i| {
            let mut fields = vec![
                ("name", Value::str(i.name.clone())),
                (
                    "operational_state",
                    Value::str(i.operational_state.clone()),
                ),
            ];
            // **Present or absent, never false.** The kernel refuses the
            // carrier file on an administratively-down interface, and `false`
            // would say the cable is out.
            if let Some(carrier) = i.carrier {
                fields.push(("carrier", Value::Bool(carrier)));
            }
            if let Some(address) = &i.hardware_address {
                fields.push(("hardware_address", Value::str(address.clone())));
            }
            if let Some(mtu) = i.mtu {
                fields.push(("mtu", Value::Integer(i64::from(mtu))));
            }
            // Absent where the machine does not know. A `0` here would be a
            // speed, and every interface that is down would appear to have one.
            if let Some(speed) = i.speed_megabits {
                fields.push(("speed_megabits", number(speed)));
            }
            fields.push((
                "addresses",
                Value::List(
                    i.addresses
                        .iter()
                        .map(|a| {
                            let mut address = vec![
                                ("address", Value::str(a.address.clone())),
                                (
                                    "prefix_length",
                                    Value::Integer(i64::from(a.prefix_length)),
                                ),
                                ("family", Value::str(a.family.clone())),
                            ];
                            if let Some(scope) = &a.scope {
                                address.push(("scope", Value::str(scope.clone())));
                            }
                            Value::object(address)
                        })
                        .collect(),
                ),
            ));
            if let Some(c) = &i.counters {
                fields.push((
                    "counters",
                    Value::object(vec![
                        ("received_bytes", number(c.received_bytes)),
                        ("transmitted_bytes", number(c.transmitted_bytes)),
                        ("received_packets", number(c.received_packets)),
                        ("transmitted_packets", number(c.transmitted_packets)),
                        ("received_errors", number(c.received_errors)),
                        ("transmitted_errors", number(c.transmitted_errors)),
                        ("received_dropped", number(c.received_dropped)),
                        ("transmitted_dropped", number(c.transmitted_dropped)),
                    ]),
                ));
            }
            Value::object(fields)
        })
        .collect();

    let fields = vec![
        ("subject", Value::str(SUBJECT)),
        ("interfaces", Value::List(interfaces)),
        (
            "interfaces_unreadable",
            Value::List(
                read.interfaces_unreadable
                    .iter()
                    .map(|u| {
                        Value::object(vec![
                            ("name", Value::str(u.name.clone())),
                            ("why", Value::str(u.why.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("up", Value::Integer(i64::try_from(up).unwrap_or(i64::MAX))),
        (
            "routes",
            Value::List(
                read.routes
                    .iter()
                    .map(|r| {
                        let mut route = vec![
                            ("destination", Value::str(r.destination.clone())),
                            (
                                "prefix_length",
                                Value::Integer(i64::from(r.prefix_length)),
                            ),
                        ];
                        // Absent for a directly connected network — `0.0.0.0`
                        // is not a router and reporting it as one is how a
                        // reader concludes the gateway is wrong.
                        if let Some(gateway) = &r.gateway {
                            route.push(("gateway", Value::str(gateway.clone())));
                        }
                        route.push(("interface", Value::str(r.interface.clone())));
                        route.push(("metric", Value::Integer(i64::from(r.metric))));
                        route.push(("family", Value::str(r.family.clone())));
                        Value::object(route)
                    })
                    .collect(),
            ),
        ),
        ("routes_read_from", Value::str(read.route_source.clone())),
        ("nameservers", Value::strings(read.nameservers.clone())),
        (
            "nameservers_read_from",
            Value::str(read.nameserver_source.clone()),
        ),
        (
            "unmeasured",
            Value::List(
                read.unmeasured
                    .iter()
                    .map(|u| {
                        Value::object(vec![
                            ("what", Value::str(u.what.clone())),
                            ("why", Value::str(u.why.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("read_from", Value::str(read.interface_source.clone())),
        ("means", Value::str(means(&read))),
    ];

    debug_assert!(
        super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)),
        "a get network result's keys must be a subsequence of OUTPUTS, in order"
    );
    Ok(Value::object(fields))
}

/// The kernel's word for an interface that is operationally up.
///
/// A constant rather than a literal because it is compared against a string the
/// *machine* produced, and the comparison is what `up` counts.
const UP: &str = "up";

/// **What this reading asserts**, composed from what was measured.
fn means(read: &super::machine::NetworkRead) -> String {
    let mut said = String::new();

    let has_errors = read.interfaces.iter().any(|i| {
        i.counters
            .is_some_and(|c| c.received_errors > 0 || c.transmitted_errors > 0)
    });
    let has_drops = read.interfaces.iter().any(|i| {
        i.counters
            .is_some_and(|c| c.received_dropped > 0 || c.transmitted_dropped > 0)
    });

    if has_errors {
        said.push_str(
            "AT LEAST ONE INTERFACE IS CARRYING ERRORS. A link that is up and erroring looks \
             healthy in every other reading this system has, and on a Pi it is usually a cable \
             or a marginal supply rather than a configuration — `get thermal`'s under-voltage \
             flags are the other half of that question. ",
        );
    } else if has_drops {
        said.push_str(
            "AT LEAST ONE INTERFACE IS DROPPING PACKETS WITH NO ERRORS COUNTED. A drop is a \
             separate counter from an error — the kernel discarded the packet, commonly for \
             lack of buffer space, without the interface ever registering an error — so an \
             all-clear on errors alone would miss it. Counters are cumulative SINCE THE LINK \
             CAME UP, so a nonzero count here is about this boot and not necessarily about \
             right now. ",
        );
    } else if read.interfaces.iter().any(|i| i.counters.is_some()) {
        said.push_str(
            "No interface is reporting errors or drops. Counters are cumulative SINCE THE LINK \
             CAME UP, so a zero here is about this boot and not about the hardware. ",
        );
    }

    said.push_str(
        "`operational_state` is the kernel's own word and is not a boolean: `unknown` is what a \
         tun device reports and is not `down`. ",
    );

    if read.unmeasured.is_empty() {
        said.push_str("Everything this subject reads was read.");
    } else {
        said.push_str(
            "THIS READING IS INCOMPLETE AND `unmeasured` SAYS HOW. Read it before concluding \
             anything from something's absence.",
        );
        // **Only when the IPv4 read actually failed.** ~~This sentence used to
        // be unconditional~~ — correct while nothing could read an IPv4
        // address, and a lie the moment something could: it would tell an
        // operator looking at a correctly reported address that the field
        // proves nothing. The `unmeasured` list is the authority for which
        // gaps are real, so it is consulted rather than assumed.
        if read
            .unmeasured
            .iter()
            .any(|u| u.what.contains("IPv4 address"))
        {
            said.push_str(
                " In particular, an interface showing no IPv4 address here has NOT been shown to \
                 have none — that read is one of the ones that did not happen.",
            );
        }
    }

    said
}

#[cfg(test)]
#[path = "network_tests.rs"]
mod tests;
