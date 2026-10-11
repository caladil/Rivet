//! Every branch of the hook's judgement, driven without a filesystem.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

/// A `bound` environment as `udhcpc` delivers one. `AGENTS.md` §8.3 — a
/// fixture, not a network anybody has.
fn bound() -> Vec<(&'static str, &'static str)> {
    vec![
        ("interface", "wlan0"),
        ("ip", "192.0.2.50"),
        ("subnet", "255.255.255.0"),
        ("mask", "24"),
        ("router", "192.0.2.1"),
        ("dns", "192.0.2.1"),
        ("lease", "3600"),
    ]
}

fn env(pairs: Vec<(&'static str, &'static str)>) -> impl Fn(&str) -> Option<String> {
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

#[test]
fn a_bound_event_writes_the_lease_where_netd_looks() {
    let action = decide("bound", &env(bound())).expect("a complete lease");
    let Action::Write { path, text } = action else {
        panic!("bound did not produce a write: {action:?}");
    };
    assert_eq!(path, "/run/rivet/dhcp/wlan0.lease");
    assert!(text.contains("ip=192.0.2.50"), "{text}");
    assert!(text.contains("mask=24"), "{text}");
}

/// A renewal is the same lease again, and must land in the same file rather
/// than beside it.
#[test]
fn a_renewal_writes_the_same_path() {
    let bound_action = decide("bound", &env(bound())).expect("bound");
    let renew_action = decide("renew", &env(bound())).expect("renew");
    assert_eq!(bound_action, renew_action);
}

/// **The three events that mean "no address" all take the file away.** A stale
/// lease would have `rivet-netd` keep configuring an address that is gone.
#[test]
fn the_events_meaning_no_address_remove_the_lease() {
    for word in ["deconfig", "nak", "leasefail"] {
        let action = decide(word, &env(bound())).unwrap_or_else(|e| panic!("{word}: {e}"));
        assert_eq!(
            action,
            Action::Remove {
                path: "/run/rivet/dhcp/wlan0.lease".to_string()
            },
            "{word} did not withdraw the lease"
        );
    }
}

/// **`deconfig` carries no lease and that is correct**, so it must not be
/// refused for the absence of one — which is exactly what would happen if the
/// decision were "did the environment parse".
#[test]
fn deconfig_without_any_lease_variables_still_withdraws() {
    let action = decide("deconfig", &env(vec![("interface", "eth0")])).expect("deconfig is fine");
    assert_eq!(
        action,
        Action::Remove {
            path: "/run/rivet/dhcp/eth0.lease".to_string()
        }
    );
}

/// A `bound` with the same missing variables IS a fault, and the two must not
/// produce the same answer.
#[test]
fn bound_without_a_lease_is_refused_where_deconfig_is_not() {
    let sparse = vec![("interface", "eth0")];
    assert!(decide("deconfig", &env(sparse.clone())).is_ok());
    let refused = decide("bound", &env(sparse)).expect_err("a bound with no address is a fault");
    assert!(refused.contains("eth0"), "{refused}");
}

/// **Nothing is written when the lease is partial.** An address completed from
/// half a lease is a machine that comes up looking reachable and is not.
#[test]
fn a_partial_lease_writes_nothing_and_names_what_was_missing() {
    for key in ["ip", "mask", "lease"] {
        let refused = decide("bound", &env(without(key)))
            .unwrap_err_or_else(|| panic!("a lease with no `{key}` was accepted"));
        assert!(
            refused.contains(key),
            "the refusal does not name `{key}`: {refused}"
        );
    }
}

/// Without an interface there is no way to know whose address this is, and
/// guessing would write one interface's lease into another's file.
#[test]
fn an_event_with_no_interface_is_refused() {
    let refused = decide("bound", &env(without("interface"))).expect_err("no interface");
    assert!(refused.contains("interface"), "{refused}");
}

#[test]
fn an_empty_event_word_is_refused_and_says_what_udhcpc_sends() {
    let refused = decide("", &env(bound())).expect_err("no event");
    assert!(refused.contains("argv[1]"), "{refused}");
    assert!(refused.contains("bound"), "{refused}");
}

/// An unknown event carries no lease, so it withdraws — the safe direction. A
/// version of `udhcpc` that grows a state must not make this program refuse to
/// run at all.
#[test]
fn an_unknown_event_withdraws_rather_than_failing() {
    let action = decide("something-new", &env(bound())).expect("not a failure");
    assert!(matches!(action, Action::Remove { .. }), "{action:?}");
}

/// **Each interface has its own file**, because `eth0` static while `wlan0`
/// holds a lease is the configuration that was asked for by name.
#[test]
fn two_interfaces_do_not_share_a_file() {
    let mut other = without("interface");
    other.push(("interface", "eth0"));
    let a = decide("bound", &env(bound())).expect("wlan0");
    let b = decide("bound", &env(other)).expect("eth0");
    let (Action::Write { path: pa, .. }, Action::Write { path: pb, .. }) = (a, b) else {
        panic!("expected two writes");
    };
    assert_ne!(pa, pb);
    assert!(pa.contains("wlan0") && pb.contains("eth0"), "{pa} {pb}");
}

/// What the hook writes is what `rivet-netd` reads, through one parser. If this
/// ever fails, the two have drifted and the whole reason for one parser is gone.
#[test]
fn what_is_written_reads_back_as_the_same_lease() {
    let Action::Write { text, .. } = decide("bound", &env(bound())).expect("bound") else {
        panic!("expected a write");
    };
    let read_back = Lease::parse_pairs_text(&text).expect("netd must be able to read this");
    assert_eq!(read_back.interface, "wlan0");
    assert_eq!(read_back.address.prefix_len(), 24);
    assert_eq!(read_back.lease_seconds, 3600);
}

/// A small helper so a refusal test reads as one.
trait UnwrapErrOrElse<T, E> {
    fn unwrap_err_or_else(self, f: impl FnOnce() -> E) -> E;
}

impl<T: core::fmt::Debug, E> UnwrapErrOrElse<T, E> for Result<T, E> {
    fn unwrap_err_or_else(self, f: impl FnOnce() -> E) -> E {
        match self {
            Ok(_) => f(),
            Err(e) => e,
        }
    }
}
