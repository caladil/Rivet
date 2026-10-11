//! What `get network` answers, and the absences it must not shape.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::machine::{
    Interface, InterfaceAddress, InterfaceCounters, MachineError, NetworkRead, Route, Unmeasured,
    UnreadableInterface,
};

struct Fake(Result<NetworkRead, MachineError>);

impl crate::operation::machine::Observed for Fake {
    /// **Refuses rather than answering that this machine trusts nobody.**
    /// An empty trust store is a REAL reading -- it is what `ADR-028` §5.5.5's
    /// `--no-trust` image produces -- and this fake has not taken it. The
    /// direction is the dangerous one: *"trusts nobody"* is the answer
    /// somebody acts on by re-signing an artifact.
    fn read_trust(&mut self) -> Result<crate::operation::machine::TrustRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering an idle machine.** Zeroed counters
    /// are what a machine that has run nothing since it booted looks like,
    /// and a load average of zero is what an idle one looks like — both are
    /// readings, and this fake has taken neither (`AGENTS.md` §3, §5).
    fn read_cpu(&mut self) -> Result<crate::operation::machine::CpuRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering that this machine has no block
    /// devices**, which is not a state a machine that booted off one can
    /// be in.
    fn read_devices(&mut self) -> Result<crate::operation::machine::DeviceRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_network(&mut self) -> Result<NetworkRead, MachineError> {
        match &self.0 {
            Ok(v) => Ok(v.clone()),
            Err(e) => Err(MachineError::Refused(e.why().to_string())),
        }
    }
    fn read_slots(&mut self) -> Result<crate::operation::machine::SlotRead, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn read_boot(&mut self) -> Result<crate::operation::machine::BootRead, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn read_thermal(&mut self) -> Result<crate::operation::machine::ThermalRead, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn read_processes(&mut self) -> Result<crate::operation::machine::ProcessRead, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn read_memory(&mut self) -> Result<crate::operation::machine::MemoryRead, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn read_desired_state(&mut self) -> Result<Option<String>, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn read_journal(&mut self) -> Result<crate::operation::machine::JournalRead, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn read_filesystems(&mut self) -> Result<crate::operation::machine::StorageRead, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn configuration_schema(&mut self) -> Value {
        Value::str("<not this test's port>")
    }
    fn supervisor_status(&mut self) -> Result<crate::operation::status::Status, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
}

fn clean_counters() -> InterfaceCounters {
    InterfaceCounters {
        received_bytes: 1_000_000,
        transmitted_bytes: 900_000,
        received_packets: 8_000,
        transmitted_packets: 7_000,
        ..InterfaceCounters::default()
    }
}

/// The shape a Pi 4 on a wired network produces: `lo` and `bcmgenet`'s `eth0`.
fn board_says() -> NetworkRead {
    NetworkRead {
        interfaces: vec![
            Interface {
                name: "eth0".to_string(),
                operational_state: "up".to_string(),
                carrier: Some(true),
                hardware_address: Some("dc:a6:32:00:00:00".to_string()),
                mtu: Some(1500),
                speed_megabits: Some(1000),
                addresses: vec![InterfaceAddress {
                    address: "fe80:0000:0000:0000:0a00:27ff:fe8e:4d3a".to_string(),
                    prefix_length: 64,
                    family: "inet6".to_string(),
                    scope: Some("link".to_string()),
                }],
                counters: Some(clean_counters()),
            },
            Interface {
                name: "lo".to_string(),
                operational_state: "unknown".to_string(),
                carrier: Some(true),
                hardware_address: Some("00:00:00:00:00:00".to_string()),
                mtu: Some(65536),
                speed_megabits: None,
                addresses: Vec::new(),
                counters: Some(InterfaceCounters::default()),
            },
        ],
        interfaces_unreadable: Vec::new(),
        interface_source: "/sys/class/net".to_string(),
        routes: vec![Route {
            destination: "0.0.0.0".to_string(),
            prefix_length: 0,
            gateway: Some("10.0.0.254".to_string()),
            interface: "eth0".to_string(),
            metric: 202,
            family: "inet".to_string(),
        }],
        route_source: "/proc/net/route".to_string(),
        nameservers: vec!["10.0.0.254".to_string()],
        nameserver_source: "/etc/resolv.conf".to_string(),
        unmeasured: vec![Unmeasured {
            what: "IPv4 addresses".to_string(),
            why: "only over netlink".to_string(),
        }],
    }
}

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(fields) => fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v),
        other => panic!("not an object: {other:?}"),
    }
}

fn number(value: &Value, key: &str) -> i64 {
    match field(value, key) {
        Some(Value::Integer(n)) => *n,
        other => panic!("{key} is not an integer: {other:?}"),
    }
}

fn text(value: &Value, key: &str) -> String {
    match field(value, key) {
        Some(Value::Str(s)) => s.clone(),
        other => panic!("{key} is not a string: {other:?}"),
    }
}

fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    match field(value, key) {
        Some(Value::List(items)) => items,
        other => panic!("{key} is not a list: {other:?}"),
    }
}

/// The whole shape, once.
#[test]
fn the_interfaces_routes_and_resolvers_are_all_answered() {
    let mut fake = Fake(Ok(board_says()));
    let value = perform(&["network"], &mut fake).expect("a reading");
    let interfaces = list(&value, "interfaces");
    assert_eq!(interfaces.len(), 2);
    assert_eq!(text(&interfaces[0], "name"), "eth0");
    assert_eq!(number(&interfaces[0], "mtu"), 1500);
    assert_eq!(number(&interfaces[0], "speed_megabits"), 1000);

    let routes = list(&value, "routes");
    assert_eq!(text(&routes[0], "gateway"), "10.0.0.254");
    assert_eq!(number(&routes[0], "prefix_length"), 0);
}

/// **`up` counts operational state and nothing else**, which is the cheapest
/// answer to *is anything actually connected* — and `lo`'s `unknown` must not
/// be counted as up.
#[test]
fn up_counts_only_interfaces_the_kernel_calls_up() {
    let mut fake = Fake(Ok(board_says()));
    let value = perform(&["network"], &mut fake).expect("a reading");
    assert_eq!(number(&value, "up"), 1, "`unknown` was counted as up");
    assert_eq!(list(&value, "interfaces").len(), 2, "both are still reported");
}

/// **The kernel's word survives**, because `unknown` is not `down` and reducing
/// it to a boolean loses the distinction between *not configured* and *not
/// connected*.
#[test]
fn the_operational_state_is_the_kernels_own_word_and_not_a_boolean() {
    let mut fake = Fake(Ok(board_says()));
    let value = perform(&["network"], &mut fake).expect("a reading");
    let interfaces = list(&value, "interfaces");
    assert_eq!(text(&interfaces[0], "operational_state"), "up");
    assert_eq!(text(&interfaces[1], "operational_state"), "unknown");
}

/// **An unread carrier is absent, never `false`.** The kernel refuses the file
/// on an administratively-down interface, and `false` says the cable is out
/// about one nobody brought up.
#[test]
fn an_unread_carrier_is_absent_rather_than_reported_as_no_cable() {
    let mut reading = board_says();
    reading.interfaces[0].carrier = None;
    reading.interfaces[0].operational_state = "down".to_string();
    let mut fake = Fake(Ok(reading));
    let value = perform(&["network"], &mut fake).expect("a reading");
    let eth = &list(&value, "interfaces")[0];
    assert!(
        field(eth, "carrier").is_none(),
        "a cable fault was invented for an interface nobody brought up"
    );
    assert_eq!(text(eth, "operational_state"), "down");
}

/// **An unknown speed is absent, never zero.** Every interface that is down
/// would otherwise appear to have a speed of nothing.
#[test]
fn an_unknown_link_speed_is_absent_rather_than_zero() {
    let mut fake = Fake(Ok(board_says()));
    let value = perform(&["network"], &mut fake).expect("a reading");
    let loopback = &list(&value, "interfaces")[1];
    assert!(field(loopback, "speed_megabits").is_none(), "a zero speed was invented");
}

/// **A directly connected route has no gateway field at all**, rather than
/// `0.0.0.0` — which is not a router, and reads as a misconfiguration.
#[test]
fn a_directly_connected_route_omits_the_gateway_rather_than_reporting_a_zero_address() {
    let mut reading = board_says();
    reading.routes.push(Route {
        destination: "10.0.0.0".to_string(),
        prefix_length: 24,
        gateway: None,
        interface: "eth0".to_string(),
        metric: 202,
        family: "inet".to_string(),
    });
    let mut fake = Fake(Ok(reading));
    let value = perform(&["network"], &mut fake).expect("a reading");
    let routes = list(&value, "routes");
    assert!(field(&routes[1], "gateway").is_none(), "0.0.0.0 was reported as a router");
    assert_eq!(text(&routes[1], "destination"), "10.0.0.0");
}

/// **The gap is carried through and the prose refuses the inference.** An
/// interface showing no IPv4 address has not been shown to have none — on a
/// board reached over IPv4, that omission would be a lie.
#[test]
fn what_was_not_measured_is_reported_and_the_prose_refuses_the_inference() {
    let mut fake = Fake(Ok(board_says()));
    let value = perform(&["network"], &mut fake).expect("a reading");
    let gaps = list(&value, "unmeasured");
    assert_eq!(gaps.len(), 1);
    assert_eq!(text(&gaps[0], "what"), "IPv4 addresses");
    let means = text(&value, "means");
    assert!(means.contains("INCOMPLETE"), "{means}");
    assert!(means.contains("has NOT been shown to have none"), "{means}");
}

/// **The other half of that sentence, added 2026-08-31 with the netlink read.**
///
/// The caveat above was unconditional while nothing could read an IPv4 address,
/// and became a lie the moment something could: an operator looking at a
/// correctly reported `192.168.50.66/24` would be told the field proves
/// nothing. So a reading that is incomplete for some *other* reason must carry
/// the incompleteness warning and NOT that clause.
#[test]
fn a_reading_whose_only_gap_is_elsewhere_does_not_disclaim_the_addresses_it_has() {
    let mut reading = board_says();
    reading.unmeasured = vec![Unmeasured {
        what: "IPv6 routes".to_string(),
        why: "/proc/net/ipv6_route is a separate format and is not parsed here".to_string(),
    }];
    reading.interfaces[0].addresses.insert(
        0,
        InterfaceAddress {
            address: "192.168.50.66".to_string(),
            prefix_length: 24,
            family: "inet".to_string(),
            scope: Some("global".to_string()),
        },
    );
    let mut fake = Fake(Ok(reading));
    let value = perform(&["network"], &mut fake).expect("a reading");
    let means = text(&value, "means");
    assert!(means.contains("INCOMPLETE"), "{means}");
    assert!(
        !means.contains("shown to have none"),
        "the IPv4 caveat survived a reading that measured IPv4: {means}"
    );
}

/// The address the board is actually reached over, carried through the subject
/// with its prefix. The fixture is `rivet-01`'s own static configuration.
#[test]
fn an_ipv4_address_reaches_the_answer_with_its_prefix_and_family() {
    let mut reading = board_says();
    reading.unmeasured.clear();
    reading.interfaces[0].addresses.insert(
        0,
        InterfaceAddress {
            address: "192.168.50.66".to_string(),
            prefix_length: 24,
            family: "inet".to_string(),
            scope: Some("global".to_string()),
        },
    );
    let mut fake = Fake(Ok(reading));
    let value = perform(&["network"], &mut fake).expect("a reading");
    let interfaces = list(&value, "interfaces");
    let addresses = list(&interfaces[0], "addresses");
    assert_eq!(text(&addresses[0], "address"), "192.168.50.66");
    assert_eq!(number(&addresses[0], "prefix_length"), 24);
    assert_eq!(text(&addresses[0], "family"), "inet");
    assert_eq!(text(&addresses[0], "scope"), "global");
    // **IPv4 first, IPv6 after**, and the second one is still there: a reader
    // that replaced the list rather than prepending to it would pass every
    // assertion above.
    assert_eq!(text(&addresses[1], "family"), "inet6");
}

/// A machine with nothing missing says so, rather than carrying the caveat
/// unconditionally — a warning that is always there is a warning nobody reads.
#[test]
fn a_complete_reading_does_not_carry_an_incompleteness_warning() {
    let mut reading = board_says();
    reading.unmeasured.clear();
    let mut fake = Fake(Ok(reading));
    let value = perform(&["network"], &mut fake).expect("a reading");
    assert!(list(&value, "unmeasured").is_empty());
    assert!(!text(&value, "means").contains("INCOMPLETE"));
}

/// **Errors on an otherwise healthy link get their own sentence**, because they
/// look identical to health in every other reading this system has — and the
/// prose points at the other half of the question.
#[test]
fn an_interface_carrying_errors_is_called_out_and_points_at_the_supply() {
    let mut reading = board_says();
    if let Some(counters) = reading.interfaces[0].counters.as_mut() {
        counters.received_errors = 17;
    }
    let mut fake = Fake(Ok(reading));
    let value = perform(&["network"], &mut fake).expect("a reading");
    let means = text(&value, "means");
    assert!(means.contains("CARRYING ERRORS"), "{means}");
    assert!(means.contains("under-voltage"), "{means}");
    let counters = field(&list(&value, "interfaces")[0], "counters").expect("counters");
    assert_eq!(number(counters, "received_errors"), 17);
}

/// **A drop with no error is still worth an all-clear denying**, because
/// `means` used to check only the error counters and would call a link with
/// real receive drops clean. The numeric field is kept either way — this is
/// about the sentence, not the count.
#[test]
fn an_interface_dropping_received_packets_with_no_errors_is_called_out_not_cleared() {
    let mut reading = board_says();
    if let Some(counters) = reading.interfaces[0].counters.as_mut() {
        counters.received_dropped = 4;
    }
    let mut fake = Fake(Ok(reading));
    let value = perform(&["network"], &mut fake).expect("a reading");
    let means = text(&value, "means");
    assert!(means.contains("DROPPING PACKETS"), "{means}");
    assert!(!means.contains("CARRYING ERRORS"), "{means}");
    assert!(
        !means.contains("No interface is reporting errors or drops"),
        "the all-clear survived a reading with real drops: {means}"
    );
    let counters = field(&list(&value, "interfaces")[0], "counters").expect("counters");
    assert_eq!(number(counters, "received_dropped"), 4);
}

/// The transmit-side counter is the same claim from the other adapter field —
/// checked separately because the original defect tested only the receive
/// side of each pair (`received_errors`/`transmitted_errors` were both
/// checked; `received_dropped`/`transmitted_dropped` were both ignored).
#[test]
fn an_interface_dropping_transmitted_packets_with_no_errors_is_called_out_not_cleared() {
    let mut reading = board_says();
    if let Some(counters) = reading.interfaces[0].counters.as_mut() {
        counters.transmitted_dropped = 9;
    }
    let mut fake = Fake(Ok(reading));
    let value = perform(&["network"], &mut fake).expect("a reading");
    let means = text(&value, "means");
    assert!(means.contains("DROPPING PACKETS"), "{means}");
    assert!(!means.contains("CARRYING ERRORS"), "{means}");
    assert!(
        !means.contains("No interface is reporting errors or drops"),
        "the all-clear survived a reading with real drops: {means}"
    );
    let counters = field(&list(&value, "interfaces")[0], "counters").expect("counters");
    assert_eq!(number(counters, "transmitted_dropped"), 9);
}

/// **The all-clear itself, unchanged**, so the new branch above cannot have
/// swallowed it: a reading with real, fully-measured, all-zero counters must
/// still say plainly that nothing is wrong.
#[test]
fn a_fully_measured_all_zero_reading_still_gets_the_all_clear() {
    let mut fake = Fake(Ok(board_says()));
    let value = perform(&["network"], &mut fake).expect("a reading");
    let means = text(&value, "means");
    assert!(means.contains("No interface is reporting errors or drops"), "{means}");
    assert!(!means.contains("CARRYING ERRORS"), "{means}");
    assert!(!means.contains("DROPPING PACKETS"), "{means}");
}

/// **The all-clear is scoped to what was measured.** An interface whose
/// counters were never read is not evidence of a clean link, so neither the
/// error sentence nor the all-clear may fire from it alone.
#[test]
fn an_interface_with_no_counters_at_all_gets_neither_claim() {
    let mut reading = board_says();
    for interface in &mut reading.interfaces {
        interface.counters = None;
    }
    let mut fake = Fake(Ok(reading));
    let value = perform(&["network"], &mut fake).expect("a reading");
    let means = text(&value, "means");
    assert!(!means.contains("CARRYING ERRORS"), "{means}");
    assert!(!means.contains("DROPPING PACKETS"), "{means}");
    assert!(
        !means.contains("No interface is reporting errors or drops"),
        "an unmeasured interface must not be reported as clean: {means}"
    );
}

/// **An interface that could not be read is reported, never omitted** — an
/// answer without it says the machine has no such interface.
#[test]
fn an_interface_that_could_not_be_read_is_reported_rather_than_omitted() {
    let mut reading = board_says();
    reading.interfaces_unreadable.push(UnreadableInterface {
        name: "wlan0".to_string(),
        why: "no operstate".to_string(),
    });
    let mut fake = Fake(Ok(reading));
    let value = perform(&["network"], &mut fake).expect("a reading");
    let missed = list(&value, "interfaces_unreadable");
    assert_eq!(text(&missed[0], "name"), "wlan0");
    assert_eq!(text(&missed[0], "why"), "no operstate");
}

/// A machine that cannot answer says so rather than reporting a machine with no
/// interfaces, which is not a state a running Linux is in.
#[test]
fn a_port_that_refuses_is_not_a_machine_with_no_network() {
    let mut fake = Fake(Err(MachineError::Refused("no /sys/class/net".to_string())));
    let error = perform(&["network"], &mut fake).expect_err("must not report an empty network");
    assert_eq!(error.code(), ErrorCode::MachineRefused);
}

/// A second word is refused rather than ignored — `schema`'s precedent.
#[test]
fn a_second_word_is_refused() {
    let mut fake = Fake(Ok(board_says()));
    let error = perform(&["network", "eth0"], &mut fake).expect_err("refused");
    assert_eq!(error.code(), ErrorCode::Arguments);
}
