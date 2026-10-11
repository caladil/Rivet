//! Every answer [`current_lease`] gives, and the client's argument vector,
//! driven without a network.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

/// Hand-composed, in the `key=value` shape `cmd/rivet-dhcp-hook` writes.
fn a_lease() -> String {
    "interface=eth0\nip=192.0.2.50\nmask=24\nrouter=192.0.2.1\nlease=3600\n".to_string()
}

fn eth0() -> InterfaceName {
    InterfaceName::new("eth0").expect("a valid name")
}

/// A reader that answers `text` for the one path it expects and records what
/// it was asked.
fn reading(text: Option<String>) -> impl FnMut(&str) -> Option<String> {
    move |path: &str| {
        assert_eq!(path, "/run/rivet/dhcp/eth0.lease", "read the wrong file");
        text.clone()
    }
}

#[test]
fn a_recorded_lease_is_read_and_parsed() {
    let lease = current_lease(&eth0(), &mut reading(Some(a_lease())), true)
        .expect("a readable lease")
        .expect("a lease");
    assert_eq!(lease.address.address().to_string(), "192.0.2.50");
    assert_eq!(lease.address.prefix_len(), 24);
    assert_eq!(lease.lease_seconds, 3600);
}

/// **No lease yet is not a failure.** The client is a service that keeps
/// asking, and the supervisor runs `rivet-netd` again when the hook records
/// the answer — so a run before that has nothing to do for this interface.
#[test]
fn no_lease_yet_is_an_answer_and_not_a_refusal() {
    assert_eq!(current_lease(&eth0(), &mut reading(None), true), Ok(None));
}

/// A card built without `--udhcpc` is a supported card. It says so rather than
/// waiting for ever for a lease nothing on the card can take.
#[test]
fn a_card_without_the_client_says_so_and_names_it() {
    let refused = current_lease(&eth0(), &mut reading(Some(a_lease())), false)
        .expect_err("no client");
    assert_eq!(
        refused,
        DhcpUnavailable::MechanismNotBuilt {
            interface: "eth0".to_string(),
            mechanism: MECHANISM,
        }
    );
    assert!(refused.to_string().contains("--udhcpc"), "{refused}");
}

/// A damaged lease configures NOTHING. An address assembled from half a lease
/// is a machine that looks reachable and is not.
#[test]
fn a_damaged_lease_is_refused_rather_than_half_applied() {
    let refused = current_lease(
        &eth0(),
        &mut reading(Some("interface=eth0\nip=192.0.2.50\n".to_string())),
        true,
    )
    .expect_err("damaged");
    assert!(matches!(refused, DhcpUnavailable::Unreadable { .. }), "{refused:?}");
    assert!(refused.to_string().contains("NOTHING WAS CONFIGURED"), "{refused}");
}

/// **A lease becomes the same plan a static document produces** — `ADR-004`
/// §2.1's "almost nothing in the path is link-specific", asserted rather than
/// described. If this ever stops holding, DHCP has grown its own convergence.
#[test]
fn a_lease_becomes_a_plan_rivet_netd_already_knows_how_to_apply() {
    let lease = current_lease(&eth0(), &mut reading(Some(a_lease())), true)
        .expect("readable")
        .expect("a lease");
    let plan = lease.to_addressing().expect("a usable plan");
    assert_eq!(plan.interface.as_str(), "eth0");
    assert_eq!(plan.address.address().to_string(), "192.0.2.50");
    assert_eq!(plan.gateway.map(|g| g.to_string()).as_deref(), Some("192.0.2.1"));
}

/// A network with no gateway is a real configuration, and the plan says so
/// rather than inventing one.
#[test]
fn a_lease_with_no_router_becomes_a_plan_with_no_gateway() {
    let text = "interface=eth0\nip=192.0.2.50\nmask=24\nlease=600\n".to_string();
    let lease = current_lease(&eth0(), &mut reading(Some(text)), true)
        .expect("readable")
        .expect("a lease");
    assert_eq!(lease.to_addressing().expect("a plan").gateway, None);
}

/// **The client neither quits once it has a lease nor gives up when nobody
/// answers.** `-q` would end renewal and `-n` would turn a slow router into a
/// service the supervisor stops restarting; both are what the supervised
/// client exists to avoid.
#[test]
fn the_client_stays_in_the_foreground_and_never_quits_on_its_own() {
    let args = arguments("eth0");
    assert_eq!(args, ["-i", "eth0", "-f", "-t", DISCOVER_ATTEMPTS]);
    for quits in ["-q", "-n", "-b", "-s"] {
        assert!(!args.iter().any(|a| a == quits), "{quits} in {args:?}");
    }
}

/// Every refusal names the interface, because a machine with several is the
/// case this is for.
#[test]
fn every_refusal_names_the_interface_it_is_about() {
    let cases = [
        DhcpUnavailable::MechanismNotBuilt {
            interface: "wlan0".to_string(),
            mechanism: MECHANISM,
        },
        DhcpUnavailable::Unreadable {
            interface: "wlan0".to_string(),
            detail: "x".to_string(),
        },
    ];
    for case in cases {
        assert!(
            case.to_string().contains("wlan0"),
            "a refusal did not name its interface: {case}"
        );
    }
}
