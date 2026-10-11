//! Turning a [`WiredReport`] into console lines.
//!
//! Separate from `main` because `ADR-006` §4 makes rendering an adapter concern
//! and the operation's result a value — so the adapter returns typed outcomes
//! and this is the only place that decides how they read. It is also the only
//! way to test the output: `main` writes to a descriptor, and a function that
//! returns lines can be asserted on.
//!
//! The format is `cmd/rivet-initd`'s: a fixed-width verdict column so a
//! photograph of an HDMI console can be read at a glance, which `ADR-004` §8
//! notes is how a panic gets diagnosed before serial is attached.

// On the Windows development host `main`'s `converge` is the refusing stub, so
// nothing in this module is called from the binary — only from its own tests.
// The allow is scoped to that platform on purpose: on Linux, dead code here
// would be a real defect and must still warn.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use rivet_adapter_net::dhcp::DhcpUnavailable;
use rivet_adapter_net::wired::{NetHealth, RouteWithdrawal, StepOutcome, WiredReport, Withdrawal};

/// The prefix every line carries, so netd's output is distinguishable from
/// PID 1's and the supervisor's on a shared console.
const TAG: &str = "rivet-netd:";

/// Width of the verdict column. One constant, so every line in this module
/// lines up whichever function produced it — a console photographed off HDMI
/// is read by scanning that column (`ADR-004` §8).
const VERDICT: usize = 9;

/// One tagged line, for a message that is not part of a per-interface report.
pub fn tagged(text: &str) -> String {
    format!("{TAG} {text}")
}

/// The lines for an interface whose document asked for a lease and whose
/// lease cannot be used: the client is not on this card, or what it recorded
/// does not parse.
///
/// Formatted exactly like [`report_lines`] — same tag, same verdict column — so
/// that a console carrying a mixture of leased and static interfaces reads as
/// one report rather than two formats. [`lease_taken_lines`] is the other
/// half, and [`lease_awaited_lines`] the state between them.
pub fn lease_refused_lines(interface: &str, why: &DhcpUnavailable) -> Vec<String> {
    vec![
        format!("{TAG} converging {interface}"),
        format!("{TAG}   {:<VERDICT$} take a lease", "REFUSED"),
        format!("{TAG}             {why}"),
        format!("{TAG} could not configure {interface}"),
    ]
}

/// What a lease turned out to be, in the same shape a convergence reports.
///
/// **The server is named.** A board that came up on an address nobody expected
/// is a real morning, and the first question is always *who gave it that* —
/// which the journal can only answer if this line carries it.
///
/// **So is the expiry.** The supervised client renews it at half its length;
/// a lease that outlives the number printed here is one that was renewed, and
/// each renewal that changes it runs this program again.
pub fn lease_taken_lines(lease: &rivet_adapter_net::lease::Lease) -> Vec<String> {
    let gateway = lease
        .router
        .map_or_else(|| "no gateway".to_string(), |r| r.to_string());
    let from = lease
        .server
        .map_or_else(|| "an unnamed server".to_string(), |s| s.to_string());
    vec![
        format!("{TAG} converging {}", lease.interface),
        format!(
            "{TAG}   {:<VERDICT$} took a lease from {from}",
            "OK"
        ),
        format!(
            "{TAG}             {}/{} via {gateway}, {}s, renewed by its client",
            lease.address.address(),
            lease.address.prefix_len(),
            lease.lease_seconds
        ),
    ]
}

/// The lines for a DHCP interface whose client has not recorded a lease yet.
///
/// **Not a refusal and not a failure**: the client is a service of its own,
/// asking, and the supervisor runs this program again when the lease
/// arrives. Named so that a console read before then says what it is waiting
/// on rather than nothing.
pub fn lease_awaited_lines(interface: &str, client: &str) -> Vec<String> {
    vec![
        format!("{TAG} converging {interface}"),
        format!("{TAG}   {:<VERDICT$} take a lease", "WAITING"),
        format!(
            "{TAG}             {client} has not recorded one yet; this runs again when it does"
        ),
    ]
}

/// One line per address taken off a DHCP interface because its lease no
/// longer names it, in the verdict column [`report_lines`] uses. Nothing is
/// said when nothing was withdrawn, which is every ordinary run.
pub fn withdrawn_lines(interface: &str, withdrawn: &[Withdrawal]) -> Vec<String> {
    let mut lines = Vec::new();
    for w in withdrawn {
        lines.push(format!(
            "{TAG}   {:<VERDICT$} withdraw {}/{} from {interface}, which its lease does not name",
            verdict(&w.outcome),
            w.address,
            w.prefix_len
        ));
        if let StepOutcome::Refused { detail, .. } = &w.outcome {
            lines.push(format!("{TAG}             {detail}"));
        }
    }
    lines
}

/// Whether any withdrawal was refused, which is a failure to converge.
pub fn any_refused(withdrawn: &[Withdrawal]) -> bool {
    withdrawn.iter().any(|w| w.outcome.refused())
}

/// One line per default route taken off a DHCP interface because it does not
/// go via the lease's router, in the same verdict column. Nothing is said when
/// nothing was withdrawn.
pub fn routes_withdrawn_lines(interface: &str, withdrawn: &[RouteWithdrawal]) -> Vec<String> {
    let mut lines = Vec::new();
    for w in withdrawn {
        let via = w
            .gateway
            .map_or_else(|| "no gateway".to_string(), |g| format!("via {g}"));
        lines.push(format!(
            "{TAG}   {:<VERDICT$} withdraw the default route {via} from {interface}, which \
             its lease does not name",
            verdict(&w.outcome)
        ));
        if let StepOutcome::Refused { detail, .. } = &w.outcome {
            lines.push(format!("{TAG}             {detail}"));
        }
    }
    lines
}

/// Whether any route withdrawal was refused, which is a failure to converge.
pub fn any_route_refused(withdrawn: &[RouteWithdrawal]) -> bool {
    withdrawn.iter().any(|w| w.outcome.refused())
}

/// One line per step, then the verdict.
pub fn report_lines(report: &WiredReport) -> Vec<String> {
    let mut lines = Vec::with_capacity(report.outcomes.len() + 2);
    lines.push(format!("{TAG} converging {}", report.interface));
    for (step, outcome) in &report.outcomes {
        lines.push(format!("{TAG}   {:<VERDICT$} {step}", verdict(outcome)));
        if let StepOutcome::Refused { detail, .. } = outcome {
            lines.push(format!("{TAG}             {detail}"));
        }
    }
    if let Some(info) = report.link {
        lines.push(format!(
            "{TAG}   link index {}, flags {:#x}{}",
            info.index,
            info.flags,
            if info.has_carrier() {
                ", carrier"
            } else {
                ", NO CARRIER"
            }
        ));
    }
    lines.push(format!("{TAG} {}", report.health()));
    lines
}

fn verdict(outcome: &StepOutcome) -> &'static str {
    match outcome {
        StepOutcome::Applied => "applied",
        StepOutcome::Converged => "already",
        StepOutcome::Observed => "read",
        StepOutcome::NotRequested => "not asked",
        StepOutcome::Skipped => "skipped",
        StepOutcome::Refused { .. } => "REFUSED",
    }
}

/// What the process exits with.
///
/// `ADR-017` §3.5 restarts a service that fails, with backoff, and gives up
/// eventually. That makes the exit code a policy decision rather than a
/// courtesy, and the policy is: **a configuration that was applied exits 0,
/// including when there is no carrier.** A cable is not Rivet's to plug in,
/// and exiting non-zero over one would restart this binary for ever on a
/// correct machine.
pub fn exit_code(health: &NetHealth) -> i32 {
    if health.applied() { 0 } else { 1 }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use rivet_adapter_net::port::Failure;
    use rivet_adapter_net::wire::IfInfo;
    use rivet_adapter_net::wired::Step;

    /// A withdrawal names the address with its prefix and the interface, in
    /// the verdict column; a refusal carries its reason and counts as a
    /// failure, and nothing withdrawn says nothing.
    #[test]
    fn a_withdrawal_names_the_address_and_a_refusal_counts_as_a_failure() {
        let applied = Withdrawal {
            address: std::net::Ipv4Addr::new(192, 168, 1, 50),
            prefix_len: 24,
            outcome: StepOutcome::Applied,
        };
        let lines = withdrawn_lines("eth0", std::slice::from_ref(&applied));
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("applied"), "{}", lines[0]);
        assert!(lines[0].contains("192.168.1.50/24 from eth0"), "{}", lines[0]);
        assert!(!any_refused(std::slice::from_ref(&applied)));

        let refused = Withdrawal {
            outcome: StepOutcome::Refused {
                failure: None,
                detail: "needs CAP_NET_ADMIN".to_string(),
            },
            ..applied
        };
        let lines = withdrawn_lines("eth0", std::slice::from_ref(&refused));
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("REFUSED"));
        assert!(lines[1].contains("CAP_NET_ADMIN"));
        assert!(any_refused(&[refused]));

        assert!(withdrawn_lines("eth0", &[]).is_empty());
    }

    /// A route withdrawal names the router it went via and the interface; a
    /// refusal carries its reason and counts as a failure.
    #[test]
    fn a_route_withdrawal_names_the_old_router_and_a_refusal_counts_as_a_failure() {
        let applied = RouteWithdrawal {
            gateway: Some(std::net::Ipv4Addr::new(192, 168, 1, 1)),
            outcome: StepOutcome::Applied,
        };
        let lines = routes_withdrawn_lines("eth0", std::slice::from_ref(&applied));
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("applied"), "{}", lines[0]);
        assert!(lines[0].contains("via 192.168.1.1 from eth0"), "{}", lines[0]);
        assert!(!any_route_refused(std::slice::from_ref(&applied)));

        let refused = RouteWithdrawal {
            gateway: None,
            outcome: StepOutcome::Refused {
                failure: None,
                detail: "needs CAP_NET_ADMIN".to_string(),
            },
        };
        let lines = routes_withdrawn_lines("eth0", std::slice::from_ref(&refused));
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("REFUSED") && lines[0].contains("no gateway"));
        assert!(lines[1].contains("CAP_NET_ADMIN"));
        assert!(any_route_refused(&[refused]));

        assert!(routes_withdrawn_lines("eth0", &[]).is_empty());
    }

    fn report(outcomes: Vec<(Step, StepOutcome)>, link: Option<IfInfo>) -> WiredReport {
        WiredReport {
            interface: "eth0".to_string(),
            outcomes,
            link,
        }
    }

    fn all_good() -> WiredReport {
        report(
            vec![
                (Step::Validate, StepOutcome::Observed),
                (Step::FindInterface, StepOutcome::Observed),
                (Step::BringUp, StepOutcome::Applied),
                (Step::AssignAddress, StepOutcome::Applied),
                (Step::InstallRoute, StepOutcome::Converged),
                (Step::Confirm, StepOutcome::Observed),
            ],
            Some(IfInfo {
                index: 2,
                flags: 0x1_0001,
            }),
        )
    }

    #[test]
    fn every_step_gets_a_line_and_the_verdict_is_last() {
        let lines = report_lines(&all_good());
        assert!(lines.iter().all(|l| l.starts_with(TAG)));
        for step in ["bring the interface up", "install the default route"] {
            assert!(lines.iter().any(|l| l.contains(step)), "{step} is missing");
        }
        assert!(
            lines.last().is_some_and(|l| l.contains("carrier present")),
            "{lines:?}"
        );
    }

    #[test]
    fn a_refusal_puts_its_detail_on_its_own_line_rather_than_off_the_edge() {
        let detail = "there is no interface called eth0 — check the name".to_string();
        let lines = report_lines(&report(
            vec![
                (Step::Validate, StepOutcome::Observed),
                (
                    Step::FindInterface,
                    StepOutcome::Refused {
                        failure: Some(Failure::Kernel(
                            rivet_adapter_net::port::Errno::ENODEV,
                        )),
                        detail: detail.clone(),
                    },
                ),
                (Step::BringUp, StepOutcome::Skipped),
            ],
            None,
        ));
        assert!(lines.iter().any(|l| l.contains("REFUSED")));
        assert!(lines.iter().any(|l| l.contains(&detail)));
    }

    /// The absent carrier has to be *loud*. It is the state that looks like
    /// success from inside the machine and like a dead machine from outside.
    #[test]
    fn a_missing_carrier_is_shouted_rather_than_mentioned() {
        let mut r = all_good();
        r.link = Some(IfInfo {
            index: 2,
            flags: 0x1,
        });
        let lines = report_lines(&r);
        assert!(
            lines.iter().any(|l| l.contains("NO CARRIER")),
            "{lines:?}"
        );
    }

    #[test]
    fn a_skipped_step_still_appears_so_the_reader_knows_it_did_not_run() {
        let lines = report_lines(&report(
            vec![(Step::InstallRoute, StepOutcome::Skipped)],
            None,
        ));
        assert!(lines.iter().any(|l| l.contains("skipped")));
    }

    #[test]
    fn a_route_nobody_asked_for_reads_as_not_asked_and_not_as_a_failure() {
        let lines = report_lines(&report(
            vec![(Step::InstallRoute, StepOutcome::NotRequested)],
            None,
        ));
        assert!(lines.iter().any(|l| l.contains("not asked")));
        assert!(!lines.iter().any(|l| l.contains("REFUSED")));
    }

    /// The policy in [`exit_code`]'s documentation, asserted so it cannot drift
    /// into "any imperfection is a failure" — which would restart this binary
    /// for ever on a machine whose only problem is an unplugged cable.
    #[test]
    fn an_applied_configuration_exits_zero_even_with_no_carrier() {
        assert_eq!(exit_code(&NetHealth::Configured), 0);
        assert_eq!(exit_code(&NetHealth::ConfiguredWithoutCarrier), 0);
        assert_eq!(
            exit_code(&NetHealth::Failed {
                at: Step::BringUp,
                detail: "no".to_string()
            }),
            1
        );
    }

    /// When more than one step refuses, the verdict names the **first**. The
    /// later ones are usually consequences of it, and a report that led with
    /// the last would send a reader to the symptom.
    #[test]
    fn the_verdict_names_the_first_refusal_and_not_the_last() {
        let r = report(
            vec![
                (Step::Validate, StepOutcome::Observed),
                (
                    Step::BringUp,
                    StepOutcome::Refused {
                        failure: None,
                        detail: "the-first-one".to_string(),
                    },
                ),
                (
                    Step::Confirm,
                    StepOutcome::Refused {
                        failure: None,
                        detail: "the-second-one".to_string(),
                    },
                ),
            ],
            None,
        );
        let verdict = r.health().to_string();
        assert!(verdict.contains("the-first-one"), "{verdict}");
        assert!(!verdict.contains("the-second-one"), "{verdict}");
    }

    /// The lease refusal has to be as legible as a netlink one, because to the
    /// operator it is the same event: the interface named in the document did
    /// not come up. It must name the interface, the mechanism, and the way
    /// forward, without an ADR open beside it.
    #[test]
    fn a_refused_lease_reads_like_the_rest_of_the_report() {
        let why = rivet_adapter_net::dhcp::DhcpUnavailable::MechanismNotBuilt {
            interface: "wlan0".to_string(),
            mechanism: rivet_adapter_net::dhcp::MECHANISM,
        };
        let lines = lease_refused_lines("wlan0", &why);
        assert!(lines.iter().all(|l| l.starts_with(TAG)), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("REFUSED")), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("udhcpc")), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("static address")), "{lines:?}");
        assert!(
            lines.last().is_some_and(|l| l.contains("wlan0")),
            "the verdict must name the interface: {lines:?}"
        );
    }

    /// Both formats share one verdict column, so a console carrying a leased
    /// and a static interface reads as one report. A mismatch here is invisible
    /// in code review and obvious in a photograph of an HDMI console.
    #[test]
    fn the_verdict_column_is_the_same_width_in_both_report_formats() {
        let why = rivet_adapter_net::dhcp::DhcpUnavailable::MechanismNotBuilt {
            interface: "wlan0".to_string(),
            mechanism: rivet_adapter_net::dhcp::MECHANISM,
        };
        let column = |line: &str| line.find(|c: char| !c.is_whitespace()).unwrap_or(0);
        let wired = report_lines(&all_good());
        let leased = lease_refused_lines("wlan0", &why);
        let step_of = |lines: &[String], needle: &str| {
            lines
                .iter()
                .find(|l| l.contains(needle))
                .map(|l| column(l.trim_start_matches(TAG)))
        };
        assert_eq!(
            step_of(&wired, "bring the interface up"),
            step_of(&leased, "take a lease"),
            "the two formats do not line up"
        );
    }

    #[test]
    fn a_tagged_line_is_attributable_on_a_shared_console() {
        assert!(tagged("anything").starts_with(TAG));
        assert!(tagged("anything").contains("anything"));
    }

    /// Every outcome variant must render as something. A `_ =>` arm added to
    /// `verdict` later would make a new outcome print as an existing one, which
    /// is how a report starts lying.
    #[test]
    fn every_outcome_variant_renders_distinctly() {
        let all = [
            StepOutcome::Applied,
            StepOutcome::Converged,
            StepOutcome::Observed,
            StepOutcome::NotRequested,
            StepOutcome::Skipped,
            StepOutcome::Refused {
                failure: None,
                detail: String::new(),
            },
        ];
        let mut rendered: Vec<&str> = all.iter().map(verdict).collect();
        let before = rendered.len();
        rendered.sort_unstable();
        rendered.dedup();
        assert_eq!(rendered.len(), before, "two outcomes render the same way");
    }
}
