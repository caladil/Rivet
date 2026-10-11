//! What a lease must survive, and what it must refuse.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

/// One real `bound` event, as `udhcpc` delivers it. Values are a fixture and
/// not a network anybody has — `AGENTS.md` §8.3.
fn bound() -> Vec<(&'static str, &'static str)> {
    vec![
        ("interface", "eth0"),
        ("ip", "192.0.2.50"),
        ("subnet", "255.255.255.0"),
        ("mask", "24"),
        ("router", "192.0.2.1"),
        ("dns", "192.0.2.1 192.0.2.2"),
        ("serverid", "192.0.2.1"),
        ("lease", "86400"),
        ("domain", "example.invalid"),
    ]
}

fn lookup_of(pairs: Vec<(&'static str, &'static str)>) -> impl Fn(&str) -> Option<String> {
    move |key: &str| {
        pairs
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| (*v).to_string())
    }
}

fn without(key: &str) -> Vec<(&'static str, &'static str)> {
    bound().into_iter().filter(|(k, _)| *k != key).collect()
}

fn with(key: &'static str, value: &'static str) -> Vec<(&'static str, &'static str)> {
    let mut pairs = without(key);
    pairs.push((key, value));
    pairs
}

// ---------------------------------------------------------------------------
// Reading what udhcpc says
// ---------------------------------------------------------------------------

#[test]
fn a_whole_lease_is_read() {
    let lease = Lease::from_pairs(&lookup_of(bound())).expect("a complete lease");
    assert_eq!(lease.interface, "eth0");
    assert_eq!(lease.address.address(), Ipv4Addr::new(192, 0, 2, 50));
    assert_eq!(lease.address.prefix_len(), 24);
    assert_eq!(lease.router, Some(Ipv4Addr::new(192, 0, 2, 1)));
    assert_eq!(
        lease.nameservers,
        vec![Ipv4Addr::new(192, 0, 2, 1), Ipv4Addr::new(192, 0, 2, 2)]
    );
    assert_eq!(lease.lease_seconds, 86400);
    assert_eq!(lease.domain.as_deref(), Some("example.invalid"));
}

/// **The prefix comes from `mask`, not from `subnet`.** Both are offered.
/// Converting a dotted mask would invent a prefix for a mask that is not
/// contiguous, and a broken or hostile server can send one.
#[test]
fn the_prefix_is_taken_from_mask_rather_than_converted_from_subnet() {
    // A `subnet` that disagrees with `mask`. The prefix must follow `mask`.
    let lease = Lease::from_pairs(&lookup_of(with("subnet", "255.255.0.0")))
        .expect("mask is the authority");
    assert_eq!(lease.address.prefix_len(), 24);
}

/// **A machine that cannot be addressed must not be reported as addressed.**
/// Each of these is a field a lease cannot do without, and the answer is a
/// refusal rather than a default.
#[test]
fn a_lease_missing_something_it_needs_is_refused_by_name() {
    for key in ["interface", "ip", "mask", "lease"] {
        let refused = Lease::from_pairs(&lookup_of(without(key)));
        assert_eq!(
            refused.unwrap_err(),
            LeaseError::Missing { key },
            "a lease with no `{key}` was accepted"
        );
    }
}

/// An empty value is an absent one. `udhcpc` sets a variable to the empty
/// string for an option the server did not send.
#[test]
fn an_empty_value_is_treated_as_absent() {
    assert_eq!(
        Lease::from_pairs(&lookup_of(with("ip", ""))).unwrap_err(),
        LeaseError::Missing { key: "ip" }
    );
}

#[test]
fn a_value_that_is_not_what_it_claims_is_refused_rather_than_repaired() {
    let refused = Lease::from_pairs(&lookup_of(with("ip", "not-an-address")));
    assert!(
        matches!(refused.unwrap_err(), LeaseError::Malformed { key: "ip", .. }),
        "an unparseable address was accepted"
    );
    let refused = Lease::from_pairs(&lookup_of(with("mask", "33")));
    assert!(
        matches!(
            refused.unwrap_err(),
            LeaseError::Malformed { key: "mask", .. }
        ),
        "a 33-bit IPv4 prefix was accepted"
    );
    let refused = Lease::from_pairs(&lookup_of(with("lease", "forever")));
    assert!(matches!(
        refused.unwrap_err(),
        LeaseError::Malformed { key: "lease", .. }
    ));
}

/// The optional halves really are optional: a lease with an address and nothing
/// else is a lease, and a network with no gateway is a real configuration.
#[test]
fn a_lease_without_router_dns_or_domain_is_still_a_lease() {
    let minimal = vec![
        ("interface", "wlan0"),
        ("ip", "192.0.2.50"),
        ("mask", "24"),
        ("lease", "600"),
    ];
    let lease = Lease::from_pairs(&lookup_of(minimal)).expect("a lease");
    assert_eq!(lease.router, None);
    assert!(lease.nameservers.is_empty());
    assert_eq!(lease.domain, None);
    assert_eq!(lease.server, None);
}

/// **Only the first router is taken**, because `ADR-023` has one default route.
/// A server offering several is ordinary and must not be an error.
#[test]
fn several_routers_are_offered_and_the_first_is_taken() {
    let lease = Lease::from_pairs(&lookup_of(with("router", "192.0.2.1 192.0.2.9")))
        .expect("several routers is not an error");
    assert_eq!(lease.router, Some(Ipv4Addr::new(192, 0, 2, 1)));
}

// ---------------------------------------------------------------------------
// The file, which is the same vocabulary
// ---------------------------------------------------------------------------

/// **The property `rivet-netd` depends on**: what the hook writes is what netd
/// reads, through one parser.
#[test]
fn a_lease_survives_being_written_and_read_back() {
    let lease = Lease::from_pairs(&lookup_of(bound())).expect("a lease");
    let text = lease.to_pairs_text();
    assert_eq!(Lease::parse_pairs_text(&text).expect("reads back"), lease);
}

/// Identical leases produce identical bytes, so netd can compare a file rather
/// than re-parse it to be told nothing changed.
#[test]
fn the_written_form_is_stable() {
    let lease = Lease::from_pairs(&lookup_of(bound())).expect("a lease");
    assert_eq!(lease.to_pairs_text(), lease.to_pairs_text());
}

/// **The file and the environment are one vocabulary.** If this ever fails, the
/// two sources have drifted and the whole reason there is one parser is gone.
#[test]
fn the_file_uses_the_same_keys_udhcpc_does() {
    let lease = Lease::from_pairs(&lookup_of(bound())).expect("a lease");
    let text = lease.to_pairs_text();
    for key in ["interface", "ip", "mask", "router", "dns", "lease"] {
        assert!(
            text.lines().any(|l| l.starts_with(&format!("{key}="))),
            "the written lease has no `{key}`, which udhcpc's environment does: {text}"
        );
    }
}

#[test]
fn a_trailing_newline_or_blank_line_does_not_break_the_file() {
    let lease = Lease::from_pairs(&lookup_of(bound())).expect("a lease");
    let text = format!("\n{}\n\n", lease.to_pairs_text());
    assert_eq!(Lease::parse_pairs_text(&text).expect("reads back"), lease);
}

#[test]
fn a_damaged_file_is_refused_rather_than_half_applied() {
    let refused = Lease::parse_pairs_text("interface=eth0\nip=192.0.2.50\n");
    assert_eq!(refused.unwrap_err(), LeaseError::Missing { key: "mask" });
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[test]
fn the_events_udhcpc_sends_are_understood() {
    assert_eq!(Event::parse("bound"), Event::Bound);
    assert_eq!(Event::parse("renew"), Event::Renew);
    assert_eq!(Event::parse("deconfig"), Event::Deconfig);
    assert_eq!(Event::parse("nak"), Event::Nak);
    assert_eq!(Event::parse("leasefail"), Event::LeaseFail);
}

/// An unknown event must not stop the hook. A version that grows a state is a
/// thing that happens, and the answer is to say so and carry on.
#[test]
fn an_unknown_event_is_carried_rather_than_refused() {
    let event = Event::parse("something-new");
    assert_eq!(event, Event::Other("something-new".to_string()));
    assert_eq!(event.as_str(), "something-new");
    assert!(!event.carries_a_lease());
}

/// **`deconfig` with no address is correct; `bound` with no address is a
/// fault.** These must not produce the same answer, and they would if the only
/// question asked were whether the environment parsed.
#[test]
fn only_bound_and_renew_are_expected_to_carry_a_lease() {
    assert!(Event::Bound.carries_a_lease());
    assert!(Event::Renew.carries_a_lease());
    for quiet in [Event::Deconfig, Event::Nak, Event::LeaseFail] {
        assert!(
            !quiet.carries_a_lease(),
            "{quiet:?} was expected to carry a lease"
        );
    }
}
