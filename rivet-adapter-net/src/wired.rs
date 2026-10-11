//! The whole wired bring-up, in order, and the verdict it produces.
//!
//! This module is to [`crate::link`], [`crate::address`] and [`crate::route`]
//! what `cmd/rivet-initd`'s `boot` module is to its mount plan: **everything
//! here is a decision, so everything here is above the
//! [`SendRecv`] seam and drivable by a fake.** No
//! kernel, no network, no emulator — which matters more here than it did at
//! Stage 0, because `PLAN.md` §2.2 records that the emulator has no interface
//! to bring up at all.
//!
//! # The order, and what each step depends on
//!
//! | | Step | Cannot happen before |
//! | :-- | :-- | :-- |
//! | 1 | [`Step::Validate`] | — it touches no kernel at all |
//! | 2 | [`Step::FindInterface`] | — but everything after needs its index |
//! | 3 | [`Step::BringUp`] | the index |
//! | 4 | [`Step::AssignAddress`] | the interface being up |
//! | 5 | [`Step::InstallRoute`] | **the address** — see [`crate::route`]; the kernel calls a gateway unreachable until an on-link route to it exists, and assigning the address is what creates one |
//! | 6 | [`Step::Confirm`] | everything, because it re-reads rather than trusts |
//!
//! Step 6 is not ceremony. The kernel acknowledges `RTM_NEWLINK` when it has
//! *accepted* the request; `AGENTS.md` §5 says the kernel is the authority for
//! system state and its verdicts are never cached, so the flags are read back
//! rather than inferred from an ack.
//!
//! # What a failure means here
//!
//! Unlike PID 1, this process is allowed to fail — `ADR-017` §3.1 — so there is
//! no equivalent of `BootHealth::Unconfirmable`. What there *is* is the
//! distinction between a machine that is configured and one that is
//! configured-but-unplugged, because on a headless board those look identical
//! from the outside and only one of them is Rivet's problem.

use rivet_core::net::StaticAddressing;

use crate::port::{Errno, Failure, Sequence, SendRecv};
use crate::wire::IfInfo;
use crate::{address, link, route};

/// One stage of the bring-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Validate,
    FindInterface,
    BringUp,
    AssignAddress,
    InstallRoute,
    Confirm,
}

impl core::fmt::Display for Step {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let name = match self {
            Self::Validate => "validate the plan",
            Self::FindInterface => "find the interface",
            Self::BringUp => "bring the interface up",
            Self::AssignAddress => "assign the address",
            Self::InstallRoute => "install the default route",
            Self::Confirm => "read the interface back",
        };
        f.write_str(name)
    }
}

/// What one [`Step`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    /// The kernel made the change on this call.
    Applied,
    /// It was already so. `ADR-015` §7 — re-running converges, it does not
    /// fail. This is what `EEXIST` and an interface that is already `IFF_UP`
    /// both become.
    Converged,
    /// A question rather than a change; the kernel answered it.
    Observed,
    /// The plan does not ask for this. A plan with no gateway does not want a
    /// default route, and saying so is different from saying it failed.
    NotRequested,
    /// An earlier step refused, so this one was never attempted. Reported
    /// rather than omitted: a console line that simply stops leaves the reader
    /// to guess whether the rest ran.
    Skipped,
    /// Reported, and the run stops. `failure` is `None` for
    /// [`Step::Validate`], which refuses before any socket is involved.
    Refused {
        failure: Option<Failure>,
        detail: String,
    },
}

impl StepOutcome {
    pub fn refused(&self) -> bool {
        matches!(self, Self::Refused { .. })
    }
}

/// What the bring-up left the interface in.
///
/// The middle variant is the one worth having. A Pi with a correct
/// configuration and no cable in it reaches exactly the same netlink state as
/// one that is working, and reporting them alike is how twenty minutes get
/// spent re-reading correct configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetHealth {
    /// Up, addressed, routed, and the kernel reports a carrier.
    Configured,
    /// Up, addressed, routed — and **no carrier**. The configuration is
    /// applied; nothing is plugged in, or the other end is dark.
    ConfiguredWithoutCarrier,
    Failed { at: Step, detail: String },
}

impl NetHealth {
    /// Whether every step the plan asked for was applied or already true.
    ///
    /// Deliberately true for [`ConfiguredWithoutCarrier`](Self::ConfiguredWithoutCarrier):
    /// a cable is not Rivet's to plug in, and exiting non-zero because of one
    /// would make `rivet-netd` a service that fails on a correct machine and
    /// gets restarted for ever (`ADR-017` §3.5).
    pub fn applied(&self) -> bool {
        !matches!(self, Self::Failed { .. })
    }
}

impl core::fmt::Display for NetHealth {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Configured => write!(f, "configured, carrier present"),
            Self::ConfiguredWithoutCarrier => write!(
                f,
                "configured, but the kernel reports no carrier — nothing is plugged in, or the other end is dark"
            ),
            Self::Failed { at, detail } => write!(f, "could not {at}: {detail}"),
        }
    }
}

/// Everything one bring-up did, step by step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WiredReport {
    pub interface: String,
    pub outcomes: Vec<(Step, StepOutcome)>,
    /// What the kernel said about the link the last time it was asked.
    pub link: Option<IfInfo>,
}

impl WiredReport {
    pub fn outcome(&self, step: Step) -> Option<&StepOutcome> {
        self.outcomes
            .iter()
            .find(|(s, _)| *s == step)
            .map(|(_, o)| o)
    }

    /// Fold the steps into one verdict.
    pub fn health(&self) -> NetHealth {
        for (step, outcome) in &self.outcomes {
            if let StepOutcome::Refused { detail, .. } = outcome {
                return NetHealth::Failed {
                    at: *step,
                    detail: detail.clone(),
                };
            }
        }
        match self.link {
            Some(info) if info.has_carrier() => NetHealth::Configured,
            // No carrier, or no reading at all. The second cannot happen after
            // a clean Confirm and is folded in here rather than given a third
            // variant, because "the kernel did not say" and "the kernel said
            // no" are the same thing to act on: look at the cable.
            _ => NetHealth::ConfiguredWithoutCarrier,
        }
    }
}

/// Converge one interface onto `plan`.
///
/// Never panics and never propagates: every failure becomes a
/// [`StepOutcome::Refused`] in the report, so the caller always gets the whole
/// picture rather than the first error. That is the same choice
/// `cmd/rivet-initd`'s mount phase makes, for the same reason — the console is
/// the only diagnostic channel a board has before Stage 1 finishes, and
/// stopping at the first line loses the rest.
pub fn bring_up(nl: &mut dyn SendRecv, plan: &StaticAddressing) -> WiredReport {
    let mut report = WiredReport {
        interface: plan.interface.to_string(),
        outcomes: Vec::new(),
        link: None,
    };
    let mut seq = Sequence::new();

    // 1. Validate. No socket is touched, so a bad plan costs nothing and
    //    cannot half-apply.
    if let Err(e) = plan.validate() {
        report.outcomes.push((
            Step::Validate,
            StepOutcome::Refused {
                failure: None,
                detail: e.to_string(),
            },
        ));
        skip_the_rest(&mut report, Step::Validate);
        return report;
    }
    report.outcomes.push((Step::Validate, StepOutcome::Observed));

    // 2. Find it. ENODEV here is the interface not existing, which is the
    //    failure an operator meets most and the one `ADR-004` §2.3's tripwire
    //    asks by hand.
    let info = match link::find(nl, &mut seq, &plan.interface) {
        Ok(info) => info,
        Err(failure) => {
            let detail = describe(Step::FindInterface, &plan.interface.to_string(), &failure);
            report
                .outcomes
                .push((Step::FindInterface, StepOutcome::Refused {
                    failure: Some(failure),
                    detail,
                }));
            skip_the_rest(&mut report, Step::FindInterface);
            return report;
        }
    };
    report
        .outcomes
        .push((Step::FindInterface, StepOutcome::Observed));
    report.link = Some(info);

    // 3. Up. Idempotence by asking the kernel rather than by swallowing an
    //    error — the same shape as the mount phase's read-the-table-first, and
    //    for the same reason: a request whose success says nothing is not a
    //    check.
    if info.is_up() {
        report.outcomes.push((Step::BringUp, StepOutcome::Converged));
    } else {
        match link::set_up(nl, &mut seq, info.index) {
            Ok(()) => report.outcomes.push((Step::BringUp, StepOutcome::Applied)),
            Err(failure) => {
                let detail = describe(Step::BringUp, &report.interface, &failure);
                report.outcomes.push((
                    Step::BringUp,
                    StepOutcome::Refused {
                        failure: Some(failure),
                        detail,
                    },
                ));
                skip_the_rest(&mut report, Step::BringUp);
                return report;
            }
        }
    }

    // 4. The address.
    match address::assign(nl, &mut seq, info.index, &plan.address) {
        Ok(()) => report
            .outcomes
            .push((Step::AssignAddress, StepOutcome::Applied)),
        Err(failure) if failure.kernel_errno() == Some(Errno::EEXIST) => report
            .outcomes
            .push((Step::AssignAddress, StepOutcome::Converged)),
        Err(failure) => {
            let detail = describe(Step::AssignAddress, &report.interface, &failure);
            report.outcomes.push((
                Step::AssignAddress,
                StepOutcome::Refused {
                    failure: Some(failure),
                    detail,
                },
            ));
            skip_the_rest(&mut report, Step::AssignAddress);
            return report;
        }
    }

    // 5. The route. After the address, always — see this module's table.
    match plan.gateway {
        None => report
            .outcomes
            .push((Step::InstallRoute, StepOutcome::NotRequested)),
        Some(gateway) => match route::add_default(nl, &mut seq, info.index, gateway) {
            Ok(()) => report
                .outcomes
                .push((Step::InstallRoute, StepOutcome::Applied)),
            Err(failure) if failure.kernel_errno() == Some(Errno::EEXIST) => report
                .outcomes
                .push((Step::InstallRoute, StepOutcome::Converged)),
            Err(failure) => {
                let detail = describe(Step::InstallRoute, &report.interface, &failure);
                report.outcomes.push((
                    Step::InstallRoute,
                    StepOutcome::Refused {
                        failure: Some(failure),
                        detail,
                    },
                ));
                skip_the_rest(&mut report, Step::InstallRoute);
                return report;
            }
        },
    }

    // 6. Read it back. An acknowledgement says the kernel accepted a request,
    //    not that the interface is up.
    match link::find(nl, &mut seq, &plan.interface) {
        Ok(info) => {
            report.link = Some(info);
            if info.is_up() {
                report.outcomes.push((Step::Confirm, StepOutcome::Observed));
            } else {
                report.outcomes.push((
                    Step::Confirm,
                    StepOutcome::Refused {
                        failure: None,
                        detail: format!(
                            "{} is not up after the kernel acknowledged the request to bring it up",
                            report.interface
                        ),
                    },
                ));
            }
        }
        Err(failure) => {
            let detail = describe(Step::Confirm, &report.interface, &failure);
            report.outcomes.push((
                Step::Confirm,
                StepOutcome::Refused {
                    failure: Some(failure),
                    detail,
                },
            ));
        }
    }

    report
}

/// One address [`withdraw_all_but`] found on the interface and took off it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Withdrawal {
    pub address: std::net::Ipv4Addr,
    pub prefix_len: u8,
    /// [`StepOutcome::Applied`] when the kernel removed it on this call,
    /// [`StepOutcome::Converged`] when it was already gone, and
    /// [`StepOutcome::Refused`] when the kernel would not.
    pub outcome: StepOutcome,
}

/// **Leave `interface` carrying no IPv4 address but `keep`** — `ADR-004` §8.
///
/// A DHCP interface carries exactly its lease's address. This is the half of
/// that [`bring_up`] does not do: it asks the kernel which IPv4 addresses the
/// interface has and withdraws every one that is not `keep`, so a lease that
/// was withdrawn leaves the interface with none, and a renewal that moved the
/// address leaves only the new one once [`bring_up`] has assigned it.
///
/// An address is `keep` when both the address and the prefix length match;
/// the same address under another mask is a different assignment and is
/// withdrawn, so [`bring_up`] puts back the one the lease names.
///
/// **Run before [`bring_up`]**, not after: deleting an interface's primary
/// address takes the secondaries in its subnet with it unless the kernel is
/// told to promote them, so the stale address goes first and the lease's is
/// assigned onto an interface that no longer has it.
///
/// # Errors
///
/// A sentence naming what could not be asked: the interface could not be
/// found, or the kernel would not list its addresses. Nothing was withdrawn.
/// A single address the kernel refuses to remove is not an error here; it is a
/// [`StepOutcome::Refused`] in the list, and the rest are still attempted.
pub fn withdraw_all_but(
    nl: &mut dyn SendRecv,
    interface: &rivet_core::net::InterfaceName,
    keep: Option<&rivet_core::net::Ipv4Cidr>,
) -> Result<Vec<Withdrawal>, String> {
    let mut seq = Sequence::new();
    let info = link::find(nl, &mut seq, interface)
        .map_err(|failure| describe(Step::FindInterface, interface.as_str(), &failure))?;
    let listed = address::dump(nl, &mut seq).map_err(|failure| {
        format!("the kernel would not list {interface}'s addresses: {failure}")
    })?;
    let index = u32::try_from(info.index).unwrap_or(u32::MAX);
    let mut out = Vec::new();
    for assigned in listed.addresses.iter().filter(|a| a.index == index) {
        let is_kept = keep.is_some_and(|cidr| {
            cidr.address() == assigned.address && cidr.prefix_len() == assigned.prefix_len
        });
        if is_kept {
            continue;
        }
        let outcome = match address::remove(nl, &mut seq, assigned) {
            Ok(()) => StepOutcome::Applied,
            Err(failure) if failure.kernel_errno() == Some(Errno::EADDRNOTAVAIL) => {
                StepOutcome::Converged
            }
            Err(failure) => {
                let detail = match failure.kernel_errno() {
                    Some(Errno::EPERM | Errno::EACCES) => format!(
                        "{failure} — removing an address needs CAP_NET_ADMIN, so this is a \
                         privilege problem rather than a lease problem"
                    ),
                    _ => failure.to_string(),
                };
                StepOutcome::Refused {
                    failure: Some(failure),
                    detail,
                }
            }
        };
        out.push(Withdrawal {
            address: assigned.address,
            prefix_len: assigned.prefix_len,
            outcome,
        });
    }
    Ok(out)
}

/// One default route [`withdraw_other_defaults`] found on the interface and
/// took off it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteWithdrawal {
    pub gateway: Option<std::net::Ipv4Addr>,
    /// [`StepOutcome::Applied`] when the kernel removed it on this call,
    /// [`StepOutcome::Converged`] when it was already gone, and
    /// [`StepOutcome::Refused`] when the kernel would not.
    pub outcome: StepOutcome,
}

/// **Leave `interface` with no default route but one via `keep`** — `ADR-004`
/// §8.
///
/// A DHCP interface's default route goes via exactly its lease's router. This
/// lists the main table's IPv4 default routes and deletes every one that
/// leaves by `interface` and does not go via `keep`, so a renewal that moved
/// only the router leaves no route via the old one, and a lease with no
/// router, or no lease, leaves none at all.
///
/// **Only routes leaving by this interface are considered.** A default route
/// on another interface — the one a static plan installs — is left alone
/// whichever router it names.
///
/// **Delete, then let [`bring_up`] add**, rather than one `RTM_NEWROUTE` with
/// `NLM_F_REPLACE`. The kernel matches a replacement on table, destination,
/// type of service and metric, never on the interface, so a replace would
/// overwrite a static interface's default of the same metric. And while the
/// old default stands, an exclusive add of the new one is refused as already
/// present, which [`bring_up`] reads as converged.
///
/// **Run before [`bring_up`]**, for that reason.
///
/// # Errors
///
/// A sentence naming what could not be asked: the interface could not be
/// found, or the kernel would not list its routes. Nothing was deleted. A
/// single route the kernel refuses to remove is not an error here; it is a
/// [`StepOutcome::Refused`] in the list, and the rest are still attempted.
pub fn withdraw_other_defaults(
    nl: &mut dyn SendRecv,
    interface: &rivet_core::net::InterfaceName,
    keep: Option<std::net::Ipv4Addr>,
) -> Result<Vec<RouteWithdrawal>, String> {
    let mut seq = Sequence::new();
    let info = link::find(nl, &mut seq, interface)
        .map_err(|failure| describe(Step::FindInterface, interface.as_str(), &failure))?;
    let defaults = route::dump_defaults(nl, &mut seq).map_err(|failure| {
        format!("the kernel would not list the routes leaving {interface}: {failure}")
    })?;
    let index = u32::try_from(info.index).unwrap_or(u32::MAX);
    let mut out = Vec::new();
    for listed in defaults.iter().filter(|r| r.oif == Some(index)) {
        if keep.is_some() && listed.gateway == keep {
            continue;
        }
        let outcome = match route::remove(nl, &mut seq, listed) {
            Ok(()) => StepOutcome::Applied,
            Err(failure) if failure.kernel_errno() == Some(Errno::ESRCH) => StepOutcome::Converged,
            Err(failure) => {
                let detail = match failure.kernel_errno() {
                    Some(Errno::EPERM | Errno::EACCES) => format!(
                        "{failure} — removing a route needs CAP_NET_ADMIN, so this is a \
                         privilege problem rather than a lease problem"
                    ),
                    _ => failure.to_string(),
                };
                StepOutcome::Refused {
                    failure: Some(failure),
                    detail,
                }
            }
        };
        out.push(RouteWithdrawal {
            gateway: listed.gateway,
            outcome,
        });
    }
    Ok(out)
}

/// Fill in the steps that never ran, so the report is complete.
fn skip_the_rest(report: &mut WiredReport, after: Step) {
    let all = [
        Step::Validate,
        Step::FindInterface,
        Step::BringUp,
        Step::AssignAddress,
        Step::InstallRoute,
        Step::Confirm,
    ];
    let start = all.iter().position(|s| *s == after).map_or(0, |i| i + 1);
    for step in all.iter().skip(start) {
        report.outcomes.push((*step, StepOutcome::Skipped));
    }
}

/// Turn a [`Failure`] into a line an operator with no shell can act on.
///
/// The errnos that get a sentence are the ones whose bare name would send a
/// diagnosis the wrong way. `ENODEV` at [`Step::FindInterface`] is not a broken
/// kernel, it is a name that is not on this board — and `ADR-004` §8.2 already
/// established that `eth0` *is*, so an operator seeing it should suspect the
/// spelling before the driver.
fn describe(step: Step, interface: &str, failure: &Failure) -> String {
    match (step, failure.kernel_errno()) {
        (Step::FindInterface, Some(Errno::ENODEV)) => format!(
            "there is no interface called {interface} — check the name against `ip -br link`; ADR-004 §8.2 read `eth0` and `wlan0` on a Pi 4"
        ),
        (_, Some(Errno::EPERM | Errno::EACCES)) => format!(
            "{failure} — changing an interface needs CAP_NET_ADMIN, so this is a privilege or namespace problem rather than a plan problem"
        ),
        (Step::InstallRoute, Some(Errno::ENETUNREACH)) => format!(
            "{failure} — the gateway is not reachable on {interface}. The address must be assigned before the route is installed; if it was, the address did not take"
        ),
        (Step::AssignAddress, Some(Errno::EADDRNOTAVAIL)) => {
            format!("{failure} — the kernel will not put that address on {interface}")
        }
        _ => failure.to_string(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::testnetlink::{FakeNetlink, Script, link_reply, netlink_error};
    use crate::wire;
    use rivet_core::net::{InterfaceName, Ipv4Cidr};

    fn plan(address: &str, gateway: Option<&str>) -> StaticAddressing {
        StaticAddressing {
            interface: InterfaceName::new("eth0").expect("a valid name"),
            address: Ipv4Cidr::parse(address).expect("a valid fixture"),
            gateway: gateway.map(|g| g.parse().expect("a valid fixture")),
        }
    }

    /// A kernel that answers a bring-up of a **down** interface: find it down,
    /// ack the set-up, ack the address, ack the route, then find it up with a
    /// carrier.
    fn cooperative_kernel(final_flags: u32) -> FakeNetlink {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(
            vec![
                Ok(link_reply(1, 2, 0)),
                Ok(netlink_error(2, 0)),
                Ok(netlink_error(3, 0)),
                Ok(netlink_error(4, 0)),
                Ok(link_reply(5, 2, final_flags)),
            ],
            Err(crate::port::Errno::EAGAIN),
        );
        nl
    }

    fn steps(report: &WiredReport) -> Vec<(Step, StepOutcome)> {
        report.outcomes.clone()
    }

    // -----------------------------------------------------------------------
    // The happy paths
    // -----------------------------------------------------------------------

    #[test]
    fn a_down_interface_is_brought_up_addressed_and_routed_in_that_order() {
        let mut nl = cooperative_kernel(wire::IFF_UP | wire::IFF_LOWER_UP);
        let report = bring_up(&mut nl, &plan("192.168.1.50/24", Some("192.168.1.1")));

        assert_eq!(
            steps(&report),
            vec![
                (Step::Validate, StepOutcome::Observed),
                (Step::FindInterface, StepOutcome::Observed),
                (Step::BringUp, StepOutcome::Applied),
                (Step::AssignAddress, StepOutcome::Applied),
                (Step::InstallRoute, StepOutcome::Applied),
                (Step::Confirm, StepOutcome::Observed),
            ]
        );
        assert_eq!(report.health(), NetHealth::Configured);
        assert!(report.health().applied());

        // The order on the wire, which is the claim that actually matters.
        let kinds: Vec<u16> = nl
            .sent
            .iter()
            .map(|m| wire::messages(m).expect("well-formed")[0].kind)
            .collect();
        assert_eq!(
            kinds,
            [
                wire::RTM_GETLINK,
                wire::RTM_NEWLINK,
                wire::RTM_NEWADDR,
                wire::RTM_NEWROUTE,
                wire::RTM_GETLINK
            ]
        );
    }

    /// The ordering rule from [`crate::route`], asserted rather than left in
    /// prose: reversing these two produces `ENETUNREACH` on a configuration
    /// that is entirely correct.
    #[test]
    fn the_address_is_always_sent_before_the_route() {
        let mut nl = cooperative_kernel(wire::IFF_UP | wire::IFF_LOWER_UP);
        let _ = bring_up(&mut nl, &plan("192.168.1.50/24", Some("192.168.1.1")));
        let kinds: Vec<u16> = nl
            .sent
            .iter()
            .map(|m| wire::messages(m).expect("well-formed")[0].kind)
            .collect();
        let addr = kinds.iter().position(|k| *k == wire::RTM_NEWADDR);
        let route = kinds.iter().position(|k| *k == wire::RTM_NEWROUTE);
        assert!(addr < route, "the route was installed before the address");
    }

    /// The second run on a machine that is already configured. Every step
    /// converges, nothing is applied, and the verdict is the same.
    #[test]
    fn a_second_run_converges_without_changing_anything() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(
            vec![
                Ok(link_reply(1, 2, wire::IFF_UP | wire::IFF_LOWER_UP)),
                Ok(netlink_error(2, -17)),
                Ok(netlink_error(3, -17)),
                Ok(link_reply(4, 2, wire::IFF_UP | wire::IFF_LOWER_UP)),
            ],
            Err(crate::port::Errno::EAGAIN),
        );
        let report = bring_up(&mut nl, &plan("192.168.1.50/24", Some("192.168.1.1")));

        assert_eq!(report.outcome(Step::BringUp), Some(&StepOutcome::Converged));
        assert_eq!(
            report.outcome(Step::AssignAddress),
            Some(&StepOutcome::Converged)
        );
        assert_eq!(
            report.outcome(Step::InstallRoute),
            Some(&StepOutcome::Converged)
        );
        assert_eq!(report.health(), NetHealth::Configured);
        // Four requests, not five: an interface that is already up is not asked
        // to come up again.
        assert_eq!(nl.sent.len(), 4);
    }

    #[test]
    fn a_plan_with_no_gateway_installs_no_route_and_does_not_call_that_a_failure() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(
            vec![
                Ok(link_reply(1, 2, 0)),
                Ok(netlink_error(2, 0)),
                Ok(netlink_error(3, 0)),
                Ok(link_reply(4, 2, wire::IFF_UP | wire::IFF_LOWER_UP)),
            ],
            Err(crate::port::Errno::EAGAIN),
        );
        let report = bring_up(&mut nl, &plan("192.168.1.50/24", None));

        assert_eq!(
            report.outcome(Step::InstallRoute),
            Some(&StepOutcome::NotRequested)
        );
        assert_eq!(report.health(), NetHealth::Configured);
        assert!(
            !nl.sent.iter().any(|m| wire::messages(m).expect("ok")[0].kind
                == wire::RTM_NEWROUTE),
            "no route may be sent when none was asked for"
        );
    }

    /// The distinction the middle verdict exists for. Everything applied, and
    /// the machine is still unreachable — because nothing is plugged in.
    #[test]
    fn a_configured_interface_with_no_carrier_says_so_rather_than_claiming_success() {
        let mut nl = cooperative_kernel(wire::IFF_UP);
        let report = bring_up(&mut nl, &plan("192.168.1.50/24", Some("192.168.1.1")));

        assert_eq!(report.health(), NetHealth::ConfiguredWithoutCarrier);
        assert!(report.health().applied(), "a missing cable is not a failure to configure");
        assert!(report.health().to_string().contains("carrier"));
    }

    // -----------------------------------------------------------------------
    // The failure paths
    // -----------------------------------------------------------------------

    #[test]
    fn a_gateway_off_the_subnet_is_refused_before_a_single_byte_is_sent() {
        let mut nl = cooperative_kernel(wire::IFF_UP | wire::IFF_LOWER_UP);
        let report = bring_up(&mut nl, &plan("192.168.1.50/24", Some("10.9.9.9")));

        assert!(nl.sent.is_empty(), "a bad plan must not half-apply");
        assert!(report.outcome(Step::Validate).is_some_and(StepOutcome::refused));
        assert_eq!(
            report.outcomes.len(),
            6,
            "every step is reported, including the ones that never ran"
        );
        assert!(matches!(report.health(), NetHealth::Failed { at: Step::Validate, .. }));
    }

    #[test]
    fn an_interface_that_does_not_exist_stops_the_run_and_names_the_name() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(netlink_error(1, -19)));
        let report = bring_up(&mut nl, &plan("192.168.1.50/24", Some("192.168.1.1")));

        assert_eq!(nl.sent.len(), 1, "nothing is attempted after ENODEV");
        let NetHealth::Failed { at, detail } = report.health() else {
            panic!("expected a failure, got {:?}", report.health());
        };
        assert_eq!(at, Step::FindInterface);
        assert!(detail.contains("eth0"), "{detail}");
        assert!(detail.contains("ip -br link"), "{detail}");
    }

    #[test]
    fn an_interface_that_will_not_come_up_stops_before_the_address_is_assigned() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(
            vec![Ok(link_reply(1, 2, 0)), Ok(netlink_error(2, -1))],
            Err(crate::port::Errno::EAGAIN),
        );
        let report = bring_up(&mut nl, &plan("192.168.1.50/24", Some("192.168.1.1")));

        assert_eq!(nl.sent.len(), 2);
        assert_eq!(
            report.outcome(Step::AssignAddress),
            Some(&StepOutcome::Skipped)
        );
        let NetHealth::Failed { at, detail } = report.health() else {
            panic!("expected a failure");
        };
        assert_eq!(at, Step::BringUp);
        assert!(detail.contains("CAP_NET_ADMIN"), "{detail}");
    }

    #[test]
    fn an_address_the_kernel_refuses_stops_before_the_route() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(
            vec![
                Ok(link_reply(1, 2, wire::IFF_UP)),
                Ok(netlink_error(2, -99)),
            ],
            Err(crate::port::Errno::EAGAIN),
        );
        let report = bring_up(&mut nl, &plan("192.168.1.50/24", Some("192.168.1.1")));

        assert_eq!(
            report.outcome(Step::InstallRoute),
            Some(&StepOutcome::Skipped)
        );
        let NetHealth::Failed { at, detail } = report.health() else {
            panic!("expected a failure");
        };
        assert_eq!(at, Step::AssignAddress);
        assert!(detail.contains("EADDRNOTAVAIL"), "{detail}");
    }

    /// `ENETUNREACH` cannot come from a plan this crate validated, so the
    /// message must send the reader at the ordering rather than at the
    /// configuration.
    #[test]
    fn an_unreachable_route_points_at_the_ordering_and_not_at_the_gateway() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(
            vec![
                Ok(link_reply(1, 2, wire::IFF_UP)),
                Ok(netlink_error(2, 0)),
                Ok(netlink_error(3, -101)),
            ],
            Err(crate::port::Errno::EAGAIN),
        );
        let report = bring_up(&mut nl, &plan("192.168.1.50/24", Some("192.168.1.1")));

        let NetHealth::Failed { at, detail } = report.health() else {
            panic!("expected a failure");
        };
        assert_eq!(at, Step::InstallRoute);
        assert!(detail.contains("before the route"), "{detail}");
    }

    /// A socket that cannot be opened is the caller's problem, but a socket
    /// that dies mid-conversation is this module's, and it must not be reported
    /// as the kernel refusing.
    #[test]
    fn a_socket_that_fails_mid_run_is_reported_as_the_socket_and_not_as_the_kernel() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(
            vec![Ok(link_reply(1, 2, wire::IFF_UP))],
            Err(crate::port::Errno::EAGAIN),
        );
        let report = bring_up(&mut nl, &plan("192.168.1.50/24", Some("192.168.1.1")));

        assert!(matches!(
            report.outcome(Step::AssignAddress),
            Some(StepOutcome::Refused {
                failure: Some(Failure::Socket(_)),
                ..
            })
        ));
    }

    /// The reason step 6 exists. The kernel acknowledged every request and the
    /// interface is still down — which an ack alone would have reported as
    /// success.
    #[test]
    fn an_interface_that_is_still_down_after_a_clean_run_is_a_failure() {
        let mut nl = cooperative_kernel(0);
        let report = bring_up(&mut nl, &plan("192.168.1.50/24", Some("192.168.1.1")));

        let NetHealth::Failed { at, detail } = report.health() else {
            panic!("an acknowledged request that did not take must not read as success");
        };
        assert_eq!(at, Step::Confirm);
        assert!(detail.contains("acknowledged"), "{detail}");
    }

    /// The report is the console output. Every step must appear in it whatever
    /// happens, or a reader cannot tell "it stopped" from "it was skipped".
    #[test]
    fn every_step_appears_in_every_report() {
        let all = [
            Step::Validate,
            Step::FindInterface,
            Step::BringUp,
            Step::AssignAddress,
            Step::InstallRoute,
            Step::Confirm,
        ];
        let cases: Vec<WiredReport> = vec![
            bring_up(
                &mut cooperative_kernel(wire::IFF_UP | wire::IFF_LOWER_UP),
                &plan("192.168.1.50/24", Some("192.168.1.1")),
            ),
            bring_up(
                &mut FakeNetlink::new(),
                &plan("192.168.1.50/24", Some("10.9.9.9")),
            ),
            {
                let mut nl = FakeNetlink::new();
                nl.replies = Script::always(Ok(netlink_error(1, -19)));
                bring_up(&mut nl, &plan("192.168.1.50/24", Some("192.168.1.1")))
            },
        ];
        for report in cases {
            for step in all {
                assert!(
                    report.outcome(step).is_some(),
                    "{step:?} is missing from {report:?}"
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Withdrawing what the lease no longer names — ADR-004 §8
    // -----------------------------------------------------------------------

    use crate::testnetlink::{addr_reply, dump_done};

    fn eth0() -> InterfaceName {
        InterfaceName::new("eth0").expect("a valid name")
    }

    /// A kernel whose `eth0` is index 2 and carries `addresses`, then acks
    /// each deletion with `delete_code`. Sequence 1 is the link lookup, 2 the
    /// dump, and 3 onwards the deletions.
    fn kernel_with(addresses: &[([u8; 4], u8, u32)], delete_code: i32) -> FakeNetlink {
        let mut replies = vec![Ok(link_reply(1, 2, wire::IFF_UP | wire::IFF_LOWER_UP))];
        for (octets, prefix, index) in addresses {
            replies.push(Ok(addr_reply(2, *index, *octets, *prefix, wire::RT_SCOPE_UNIVERSE)));
        }
        replies.push(Ok(dump_done(2)));
        for n in 0..addresses.len() {
            let seq = 3 + u32::try_from(n).expect("a handful");
            replies.push(Ok(netlink_error(seq, delete_code)));
        }
        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(replies, Err(crate::port::Errno::EAGAIN));
        nl
    }

    /// The deletions taken back off the wire: (index, address) of each
    /// `RTM_DELADDR`.
    fn deleted(nl: &FakeNetlink) -> Vec<(u32, [u8; 4])> {
        nl.sent
            .iter()
            .filter_map(|m| {
                let parsed = wire::messages(m).expect("well-formed");
                let msg = parsed.first()?;
                if msg.kind != wire::RTM_DELADDR {
                    return None;
                }
                let header = wire::parse_ifaddr(msg.payload).expect("a header");
                let attrs = wire::attributes(&msg.payload[wire::IFADDRMSG_LEN..]).expect("attrs");
                let local = attrs
                    .iter()
                    .find(|(k, _)| *k == wire::IFA_LOCAL)
                    .map(|(_, v)| <[u8; 4]>::try_from(*v).expect("four bytes"))
                    .expect("IFA_LOCAL names the address deleted");
                Some((header.index, local))
            })
            .collect()
    }

    /// **A renewal that moved the address withdraws the old one.** The
    /// interface carries the previous lease's `192.168.1.50/24`; the new lease
    /// names `192.168.1.77/24`; the old one is deleted, so `bring_up` assigns
    /// the new one onto an interface that carries nothing else.
    #[test]
    fn a_renewal_that_moved_the_address_withdraws_the_old_one() {
        let mut nl = kernel_with(&[([192, 168, 1, 50], 24, 2)], 0);
        let lease = Ipv4Cidr::parse("192.168.1.77/24").expect("valid");
        let withdrawn = withdraw_all_but(&mut nl, &eth0(), Some(&lease)).expect("asked");
        assert_eq!(deleted(&nl), [(2, [192, 168, 1, 50])]);
        assert_eq!(
            withdrawn,
            [Withdrawal {
                address: std::net::Ipv4Addr::new(192, 168, 1, 50),
                prefix_len: 24,
                outcome: StepOutcome::Applied,
            }]
        );
    }

    /// **The lease's own address is never touched**, which is the ordinary
    /// rerun on a machine whose lease did not move — and the address the
    /// control plane is reached over.
    #[test]
    fn the_leases_own_address_is_kept_and_nothing_is_deleted() {
        let mut nl = kernel_with(&[([192, 168, 1, 50], 24, 2)], 0);
        let lease = Ipv4Cidr::parse("192.168.1.50/24").expect("valid");
        let withdrawn = withdraw_all_but(&mut nl, &eth0(), Some(&lease)).expect("asked");
        assert!(withdrawn.is_empty());
        assert!(deleted(&nl).is_empty());
    }

    /// Both on the interface — the lease's and a stale one — and only the
    /// stale one goes.
    #[test]
    fn only_the_address_the_lease_does_not_name_is_withdrawn() {
        let mut nl = kernel_with(&[([10, 0, 0, 2], 24, 2), ([10, 0, 0, 9], 24, 2)], 0);
        let lease = Ipv4Cidr::parse("10.0.0.2/24").expect("valid");
        withdraw_all_but(&mut nl, &eth0(), Some(&lease)).expect("asked");
        assert_eq!(deleted(&nl), [(2, [10, 0, 0, 9])]);
    }

    /// **A withdrawn lease leaves no address.** `nak` or `leasefail` removes
    /// the lease, and the interface keeps none of what it had.
    #[test]
    fn a_withdrawn_lease_leaves_the_interface_with_no_address() {
        let mut nl = kernel_with(&[([192, 168, 1, 50], 24, 2)], 0);
        let withdrawn = withdraw_all_but(&mut nl, &eth0(), None).expect("asked");
        assert_eq!(deleted(&nl), [(2, [192, 168, 1, 50])]);
        assert_eq!(withdrawn.len(), 1);
    }

    /// Another interface's addresses are not this interface's business — the
    /// dump lists every interface, and only index 2 is `eth0`.
    #[test]
    fn another_interfaces_address_is_left_alone() {
        let mut nl = kernel_with(&[([127, 0, 0, 1], 8, 1), ([10, 0, 1, 2], 24, 3)], 0);
        let withdrawn = withdraw_all_but(&mut nl, &eth0(), None).expect("asked");
        assert!(withdrawn.is_empty());
        assert!(deleted(&nl).is_empty());
    }

    /// The same address under another mask is a different assignment: it is
    /// withdrawn so the lease's own can be assigned.
    #[test]
    fn the_same_address_under_another_mask_is_withdrawn() {
        let mut nl = kernel_with(&[([10, 0, 0, 2], 16, 2)], 0);
        let lease = Ipv4Cidr::parse("10.0.0.2/24").expect("valid");
        withdraw_all_but(&mut nl, &eth0(), Some(&lease)).expect("asked");
        assert_eq!(deleted(&nl), [(2, [10, 0, 0, 2])]);
    }

    /// An address gone by the time it is deleted is already withdrawn, not a
    /// failure.
    #[test]
    fn an_address_already_gone_is_converged() {
        let mut nl = kernel_with(&[([192, 168, 1, 50], 24, 2)], -99);
        let withdrawn = withdraw_all_but(&mut nl, &eth0(), None).expect("asked");
        assert_eq!(withdrawn[0].outcome, StepOutcome::Converged);
    }

    /// A refusal is reported per address and names the privilege.
    #[test]
    fn a_refused_withdrawal_is_reported_and_names_the_privilege() {
        let mut nl = kernel_with(&[([192, 168, 1, 50], 24, 2)], -1);
        let withdrawn = withdraw_all_but(&mut nl, &eth0(), None).expect("asked");
        let StepOutcome::Refused { detail, .. } = &withdrawn[0].outcome else {
            panic!("{withdrawn:?}");
        };
        assert!(detail.contains("CAP_NET_ADMIN"), "{detail}");
    }

    /// An interface that is not there is said, and nothing is listed or
    /// deleted.
    #[test]
    fn an_interface_that_does_not_exist_is_an_error_and_nothing_is_deleted() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::always(Ok(netlink_error(1, -19)));
        let err = withdraw_all_but(&mut nl, &eth0(), None).expect_err("no interface");
        assert!(err.contains("eth0"), "{err}");
        assert_eq!(nl.sent.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Converging the default route onto the lease's router — ADR-004 §8
    // -----------------------------------------------------------------------

    use crate::testnetlink::route_reply;
    use std::net::Ipv4Addr;

    /// A kernel whose `eth0` is index 2 and whose main table holds `routes` as
    /// `(oif, gateway)` default routes, then acks each deletion with
    /// `delete_code`. Sequence 1 is the link lookup, 2 the dump, and 3 onwards
    /// the deletions.
    fn kernel_with_defaults(routes: &[(u32, [u8; 4])], delete_code: i32) -> FakeNetlink {
        let mut replies = vec![Ok(link_reply(1, 2, wire::IFF_UP | wire::IFF_LOWER_UP))];
        // The subnet route the address put there, which is never a default.
        replies.push(Ok(route_reply(2, 24, 254, 2, None, None)));
        for (oif, gateway) in routes {
            replies.push(Ok(route_reply(2, 0, 254, *oif, Some(*gateway), None)));
        }
        replies.push(Ok(dump_done(2)));
        for n in 0..routes.len() {
            let seq = 3 + u32::try_from(n).expect("a handful");
            replies.push(Ok(netlink_error(seq, delete_code)));
        }
        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(replies, Err(crate::port::Errno::EAGAIN));
        nl
    }

    /// The deleted routes taken back off the wire: (oif, gateway) of each
    /// `RTM_DELROUTE`.
    fn deleted_routes(nl: &FakeNetlink) -> Vec<(u32, [u8; 4])> {
        nl.sent
            .iter()
            .filter_map(|m| {
                let parsed = wire::messages(m).expect("well-formed");
                let msg = parsed.first()?;
                if msg.kind != wire::RTM_DELROUTE {
                    return None;
                }
                let attrs = wire::attributes(&msg.payload[wire::RTMSG_LEN..]).expect("attrs");
                let oif = attrs
                    .iter()
                    .find(|(k, _)| *k == wire::RTA_OIF)
                    .map(|(_, v)| u32::from_ne_bytes(<[u8; 4]>::try_from(*v).expect("four")))
                    .expect("RTA_OIF names the interface");
                let gateway = attrs
                    .iter()
                    .find(|(k, _)| *k == wire::RTA_GATEWAY)
                    .map(|(_, v)| <[u8; 4]>::try_from(*v).expect("four"))
                    .expect("RTA_GATEWAY names the next hop");
                Some((oif, gateway))
            })
            .collect()
    }

    /// **A renewal that changed only the router withdraws the old default.**
    /// The interface keeps its address, the lease now names another router,
    /// and the default via the old one is deleted.
    #[test]
    fn a_renewal_that_changed_only_the_router_withdraws_the_old_default() {
        let mut nl = kernel_with_defaults(&[(2, [192, 168, 1, 1])], 0);
        let withdrawn =
            withdraw_other_defaults(&mut nl, &eth0(), Some(Ipv4Addr::new(192, 168, 1, 254)))
                .expect("asked");
        assert_eq!(deleted_routes(&nl), [(2, [192, 168, 1, 1])]);
        assert_eq!(
            withdrawn,
            [RouteWithdrawal {
                gateway: Some(Ipv4Addr::new(192, 168, 1, 1)),
                outcome: StepOutcome::Applied,
            }]
        );
    }

    /// The ordinary rerun: the default already goes via the lease's router,
    /// and nothing is deleted.
    #[test]
    fn the_leases_own_default_is_kept_and_nothing_is_deleted() {
        let mut nl = kernel_with_defaults(&[(2, [192, 168, 1, 1])], 0);
        let withdrawn =
            withdraw_other_defaults(&mut nl, &eth0(), Some(Ipv4Addr::new(192, 168, 1, 1)))
                .expect("asked");
        assert!(withdrawn.is_empty());
        assert!(deleted_routes(&nl).is_empty());
    }

    /// **A default the document declares on another interface is not this
    /// interface's to touch**, whichever router it goes via.
    #[test]
    fn a_static_default_on_another_interface_is_left_alone() {
        let mut nl = kernel_with_defaults(&[(3, [10, 0, 0, 1]), (2, [192, 168, 1, 1])], 0);
        withdraw_other_defaults(&mut nl, &eth0(), Some(Ipv4Addr::new(192, 168, 1, 254)))
            .expect("asked");
        assert_eq!(deleted_routes(&nl), [(2, [192, 168, 1, 1])]);
    }

    /// A lease that names no router, or no lease at all, leaves the interface
    /// with no default route.
    #[test]
    fn a_lease_with_no_router_withdraws_every_default_on_its_interface() {
        let mut nl = kernel_with_defaults(&[(2, [192, 168, 1, 1]), (2, [192, 168, 1, 2])], 0);
        withdraw_other_defaults(&mut nl, &eth0(), None).expect("asked");
        assert_eq!(
            deleted_routes(&nl),
            [(2, [192, 168, 1, 1]), (2, [192, 168, 1, 2])]
        );
    }

    #[test]
    fn a_default_already_gone_is_converged() {
        let mut nl = kernel_with_defaults(&[(2, [192, 168, 1, 1])], -3);
        let withdrawn = withdraw_other_defaults(&mut nl, &eth0(), None).expect("asked");
        assert_eq!(withdrawn[0].outcome, StepOutcome::Converged);
    }

    #[test]
    fn a_refused_route_withdrawal_is_reported_and_names_the_privilege() {
        let mut nl = kernel_with_defaults(&[(2, [192, 168, 1, 1])], -1);
        let withdrawn = withdraw_other_defaults(&mut nl, &eth0(), None).expect("asked");
        let StepOutcome::Refused { detail, .. } = &withdrawn[0].outcome else {
            panic!("{withdrawn:?}");
        };
        assert!(detail.contains("CAP_NET_ADMIN"), "{detail}");
    }

    /// A routing table the kernel would not list is an error, and nothing is
    /// deleted on the strength of a list that was never read.
    #[test]
    fn a_table_the_kernel_will_not_list_is_an_error_and_nothing_is_deleted() {
        let mut nl = FakeNetlink::new();
        nl.replies = Script::sequence(
            vec![
                Ok(link_reply(1, 2, wire::IFF_UP | wire::IFF_LOWER_UP)),
                Ok(netlink_error(2, -1)),
            ],
            Err(crate::port::Errno::EAGAIN),
        );
        let err = withdraw_other_defaults(&mut nl, &eth0(), None).expect_err("refused");
        assert!(err.contains("eth0"), "{err}");
        assert!(deleted_routes(&nl).is_empty());
    }

    /// **End to end on the wire.** While any default route stands, an
    /// exclusive `RTM_NEWROUTE` for the new router is answered `EEXIST` and
    /// read as converged, so the old default is deleted before the new one is
    /// added, and the add is applied.
    #[test]
    fn after_withdrawal_the_new_router_is_applied_rather_than_read_as_converged() {
        let mut nl = kernel_with_defaults(&[(2, [192, 168, 1, 1])], 0);
        withdraw_other_defaults(&mut nl, &eth0(), Some(Ipv4Addr::new(192, 168, 1, 254)))
            .expect("asked");
        // The bring-up that follows, against a kernel that now has no default.
        nl.replies = Script::sequence(
            vec![
                Ok(link_reply(1, 2, wire::IFF_UP | wire::IFF_LOWER_UP)),
                Ok(netlink_error(2, -17)),
                Ok(netlink_error(3, 0)),
                Ok(link_reply(4, 2, wire::IFF_UP | wire::IFF_LOWER_UP)),
            ],
            Err(crate::port::Errno::EAGAIN),
        );
        let report = bring_up(&mut nl, &plan("192.168.1.50/24", Some("192.168.1.254")));
        assert_eq!(
            report.outcome(Step::InstallRoute),
            Some(&StepOutcome::Applied)
        );
        let kinds: Vec<u16> = nl
            .sent
            .iter()
            .map(|m| wire::messages(m).expect("well-formed")[0].kind)
            .collect();
        let del = kinds.iter().position(|k| *k == wire::RTM_DELROUTE);
        let add = kinds.iter().position(|k| *k == wire::RTM_NEWROUTE);
        assert!(del.is_some() && del < add, "the old default must go first: {kinds:?}");
    }

    /// Nothing in this crate is allowed to panic on a hostile kernel: a
    /// `panic = "abort"` release profile turns one into an abrupt death with no
    /// console line, before there is any way to reach the machine.
    #[test]
    fn a_kernel_that_answers_with_garbage_produces_a_report_rather_than_a_panic() {
        for garbage in [
            vec![],
            vec![0xFF; 3],
            vec![0xFF; 64],
            vec![0; 16],
            {
                let mut m = wire::Message::new(wire::NLMSG_ERROR, 0, 1);
                m.push_payload(&[0u8; 2]);
                m.finish()
            },
        ] {
            let mut nl = FakeNetlink::new();
            nl.replies = Script::always(Ok(garbage.clone()));
            let report = bring_up(&mut nl, &plan("192.168.1.50/24", Some("192.168.1.1")));
            assert!(
                !report.health().applied(),
                "garbage {garbage:?} was read as a working network"
            );
        }
    }
}
