//! **What to announce, and the answer to a query, decided without touching a
//! socket.**
//!
//! `ADR-014` §3's shape applied to a daemon: everything that can be decided
//! from bytes is decided here and tested with no network, and `main.rs` is the
//! only thing that holds a socket.
//!
//! # Two facts, and where each comes from
//!
//! | Fact | Read from | Why there |
//! | :-- | :-- | :-- |
//! | the name | **the running kernel**, by `uname(2)` | `cmd/rivet-sysd` set it; the kernel is where the three readers agree |
//! | the address | **the supervisor's projection**, `rivet_core::mdns` | the `[network]` table `cmd/rivet-netd` converged from, with each DHCP interface's leased address filled in |
//!
//! **The name is not passed on the command line.** An address on a service's
//! argument vector was *"a second thing that also looked authoritative"*
//! (`ADR-011` §3), and a hostname would be the same mistake — worse, because
//! `rivet_core::hostname`'s ruling is **one name, three readers**, and the
//! kernel is what makes that true rather than merely intended.
//!
//! The projection carries the name too, as the supervisor settled it. The two
//! must agree, and when they do not the responder announces nothing and says
//! why ([`Silent::NameDisagrees`]): the projection was written for a machine of
//! a different name, so its addresses cannot be trusted to be this one's.
//!
//! So a machine whose kernel says `(none)` announces **nothing**, and says so.
//! That is not a degraded mode — it is the responder agreeing with the rest of
//! the system about what this machine is called.

use core::net::Ipv4Addr;

use rivet_core::hostname::{self, Hostname};
use rivet_core::mdns::Projection;
use simple_dns::rdata::{A, RData};
use simple_dns::{CLASS, Name, Packet, PacketFlag, ResourceRecord};

/// The domain a `.local` name lives in — RFC 6762.
pub const LOCAL: &str = "local";

/// How long a responder tells the network to remember an answer, in seconds.
///
/// 120, which is RFC 6762 §10's recommendation for a host record. Short enough
/// that a machine that goes away stops being believed within a couple of
/// minutes, long enough that it is not re-asked constantly. **It is not a
/// setting**: an operator has no way to know a better number than the RFC's,
/// and a knob here would be one more thing that can be wrong.
pub const TTL_SECONDS: u32 = 120;

/// What this responder will say, once something asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announcement {
    /// The machine's name, without the `.local`.
    pub hostname: Hostname,
    /// The address `<hostname>.local` resolves to.
    pub address: Ipv4Addr,
}

impl Announcement {
    /// The name as it goes on the wire: `<hostname>.local`.
    ///
    /// **Composed here and nowhere else.** `Hostname` is a *label* by
    /// construction (`rivet_core::hostname` refuses a dot for exactly this
    /// reason), so this is the one place the zone is attached.
    #[must_use]
    pub fn fqdn(&self) -> String {
        format!("{}.{LOCAL}", self.hostname)
    }
}

/// Why this machine will not announce itself.
///
/// **Every one of these is a refusal rather than a fallback**, and the reason
/// is the same in each case: a responder that guessed would put a wrong answer
/// on a multicast group that every device on the link believes. `AGENTS.md` §5.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Silent {
    /// The kernel has no name for this machine.
    Nameless,
    /// The kernel's name and the projection's are not the same name.
    NameDisagrees {
        kernel: String,
        projected: Option<String>,
    },
    /// The document names no interface with an address.
    NoAddress,
    /// Every addressed interface is DHCP and **none of them has a lease yet**.
    ///
    /// A moment rather than a state. `rivet-netd` runs `udhcpc`,
    /// `rivet-dhcp-hook` writes the lease and nudges the supervisor, and the
    /// supervisor rewrites this responder's projection with the address.
    DhcpNoLease { interfaces: Vec<String> },
    /// More than one interface carries an address, and this responder will not
    /// choose which one the network should be told about.
    Ambiguous { interfaces: Vec<String> },
}

impl core::fmt::Display for Silent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Nameless => write!(
                f,
                "this machine has no name: uname(2) reports `{}`, which is what a Linux kernel \
                 holds when nothing has called sethostname(2). rivet-sysd sets it from the \
                 document's `hostname` key, or derives one from this machine's serial - so \
                 either that ran and could not, and said why in the journal, or this responder \
                 started before it did. Nothing has been announced.",
                hostname::KERNEL_DEFAULT
            ),
            Self::NameDisagrees { kernel, projected } => write!(
                f,
                "the kernel calls this machine `{kernel}` and the supervisor's projection calls \
                 it `{}`, so the projection was written for a machine of another name and its \
                 addresses cannot be trusted to be this one's. The supervisor rewrites it at \
                 every reload and restarts this responder when the name changes. Nothing has \
                 been announced.",
                projected.as_deref().unwrap_or("(none)")
            ),
            Self::NoAddress => write!(
                f,
                "the configuration document names no interface with an address, so there is \
                 nothing for <name>.local to resolve TO. An mDNS responder announces where a \
                 machine is, not merely that it exists. Nothing has been announced."
            ),
            Self::DhcpNoLease { interfaces } => write!(
                f,
                "every addressed interface here takes its address by DHCP ({}) and none of \
                 them has a lease yet, so there is no address to announce. The lease is \
                 written by rivet-dhcp-hook when udhcpc gets one, and the supervisor then \
                 writes it into this responder's projection; rivet-netd is what runs udhcpc. \
                 This is a moment rather than a state - if it persists, the interface never \
                 got a lease and the journal's rivet-netd records are where that is said. \
                 Nothing has been announced.",
                interfaces.join(", ")
            ),
            Self::Ambiguous { interfaces } => write!(
                f,
                "this machine has {} addressed interfaces ({}) and this responder will not \
                 choose which one the network is told about - a wrong answer on a multicast \
                 group is believed by every device on the link. Nothing has been announced.",
                interfaces.len(),
                interfaces.join(", ")
            ),
        }
    }
}

/// Read the machine's name out of what the kernel reports.
///
/// Takes the text rather than asking: this module performs no I/O.
#[must_use]
pub fn name_in(kernel_hostname: &str) -> Option<Hostname> {
    Hostname::new(kernel_hostname.trim())
}

/// Decide what to announce, or why not.
///
/// `kernel_hostname` is what `uname(2)` reported; `projection` is what the
/// supervisor wrote. The document remains the authority for **how** an
/// interface is addressed and the lease for **what** a DHCP interface got
/// (`ADR-011`); the projection is both, read once by the supervisor, which is
/// the one process allowed to read either.
///
/// # Errors
///
/// A [`Silent`] naming which fact is missing and what would supply it.
pub fn decide(kernel_hostname: &str, projection: &Projection) -> Result<Announcement, Silent> {
    let hostname = name_in(kernel_hostname).ok_or(Silent::Nameless)?;
    let agrees = projection
        .hostname
        .as_ref()
        .is_some_and(|projected| projected.as_str().eq_ignore_ascii_case(hostname.as_str()));
    if !agrees {
        return Err(Silent::NameDisagrees {
            kernel: hostname.as_str().to_string(),
            projected: projection.hostname.as_ref().map(|h| h.as_str().to_string()),
        });
    }

    match projection.addressed.as_slice() {
        [] if !projection.leaseless.is_empty() => Err(Silent::DhcpNoLease {
            interfaces: projection.leaseless.clone(),
        }),
        [] => Err(Silent::NoAddress),
        [(_, address)] => Ok(Announcement {
            hostname,
            address: *address,
        }),
        several => Err(Silent::Ambiguous {
            interfaces: several.iter().map(|(name, _)| name.clone()).collect(),
        }),
    }
}

/// **The answer to one received datagram**, and whether it goes back to the
/// sender alone (`true`) or to the group (`false`).
///
/// `None` for anything that is not a query this machine answers: bytes that do
/// not parse, a response (including this responder's own, looped back), and a
/// query with no question for `<hostname>.local` of class `IN` or `ANY` and
/// type `A` or `ANY`. The name is compared without regard to case, as DNS
/// compares names.
///
/// The reply echoes the query's id, carries the one `A` record with
/// [`TTL_SECONDS`], and has the authoritative-answer bit set (RFC 6762 §18.4).
/// It goes to the sender alone when any question asked for a unicast answer —
/// RFC 6762 §5.4's QU bit — and to the group otherwise.
#[must_use]
pub fn reply(datagram: &[u8], announcement: &Announcement) -> Option<(Vec<u8>, bool)> {
    let query = Packet::parse(datagram).ok()?;
    if query.has_flags(PacketFlag::RESPONSE) {
        return None;
    }
    let fqdn = announcement.fqdn();
    let record = ResourceRecord::new(
        Name::new(&fqdn).ok()?,
        CLASS::IN,
        TTL_SECONDS,
        RData::A(A::from(announcement.address)),
    );

    let mut unicast = false;
    let mut asked = false;
    for question in &query.questions {
        unicast |= question.unicast_response;
        asked |= question.qname.to_string().eq_ignore_ascii_case(&fqdn)
            && record.match_qclass(question.qclass)
            && record.match_qtype(question.qtype);
    }
    if !asked {
        return None;
    }

    let mut answer = Packet::new_reply(query.id());
    answer.set_flags(PacketFlag::AUTHORITATIVE_ANSWER);
    answer.answers.push(record);
    answer.build_bytes_vec_compressed().ok().map(|bytes| (bytes, unicast))
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use simple_dns::{QCLASS, QTYPE, Question, TYPE};

    /// The type and class a query for this machine's address carries.
    const A_IN: (QTYPE, QCLASS) = (QTYPE::TYPE(TYPE::A), QCLASS::CLASS(CLASS::IN));

    fn projection(name: Option<&str>, addressed: &[(&str, [u8; 4])], leaseless: &[&str]) -> Projection {
        Projection {
            hostname: name.and_then(Hostname::new),
            addressed: addressed
                .iter()
                .map(|(n, a)| ((*n).to_string(), Ipv4Addr::from(*a)))
                .collect(),
            leaseless: leaseless.iter().map(|n| (*n).to_string()).collect(),
        }
    }

    fn eth0(name: &str) -> Projection {
        projection(Some(name), &[("eth0", [192, 168, 1, 50])], &[])
    }

    // ---- the name comes from the kernel, and must agree --------------------

    #[test]
    fn the_name_announced_is_the_one_the_kernel_holds() {
        let announcement = decide("smoke-guest\n", &eth0("smoke-guest")).expect("a name and address");
        assert_eq!(announcement.hostname.as_str(), "smoke-guest");
        assert_eq!(announcement.fqdn(), "smoke-guest.local");
    }

    /// **`(none)` announces nothing**, and that is agreement rather than
    /// degradation: the responder says what the rest of the system says this
    /// machine is called.
    #[test]
    fn a_kernel_that_says_none_makes_this_responder_silent() {
        let silent = decide(hostname::KERNEL_DEFAULT, &eth0("kitchen-pi")).expect_err("nameless");
        assert_eq!(silent, Silent::Nameless);
        let said = silent.to_string();
        assert!(said.contains("sethostname"), "{said}");
        assert!(said.contains("Nothing has been announced"), "{said}");
    }

    #[test]
    fn an_empty_or_unreadable_name_is_the_same_case() {
        for text in ["", "   ", "\n"] {
            assert_eq!(decide(text, &eth0("kitchen-pi")), Err(Silent::Nameless), "{text:?}");
        }
    }

    /// A projection written for another name, or for no name, is not this
    /// machine's: its addresses are refused rather than announced under the
    /// kernel's name.
    #[test]
    fn a_projection_for_another_name_is_refused_and_both_names_are_said() {
        let silent = decide("kitchen-pi", &eth0("larder")).expect_err("disagree");
        assert_eq!(
            silent,
            Silent::NameDisagrees {
                kernel: "kitchen-pi".to_string(),
                projected: Some("larder".to_string()),
            }
        );
        let said = silent.to_string();
        assert!(said.contains("kitchen-pi") && said.contains("larder"), "{said}");

        let nameless = projection(None, &[("eth0", [10, 0, 0, 2])], &[]);
        assert!(matches!(decide("kitchen-pi", &nameless), Err(Silent::NameDisagrees { projected: None, .. })));
    }

    /// DNS compares names without regard to case, and so does this.
    #[test]
    fn the_names_agree_whatever_their_case() {
        assert!(decide("Kitchen-Pi", &eth0("kitchen-pi")).is_ok());
    }

    #[test]
    fn the_newline_the_kernel_name_may_carry_is_not_part_of_the_name() {
        assert_eq!(
            name_in("kitchen-pi\n").map(|h| h.as_str().to_owned()),
            Some("kitchen-pi".to_owned())
        );
    }

    // ---- the address comes from the projection -----------------------------

    #[test]
    fn the_address_announced_is_the_one_projected() {
        let announcement = decide("kitchen-pi", &eth0("kitchen-pi")).expect("an address");
        assert_eq!(announcement.address, Ipv4Addr::new(192, 168, 1, 50));
    }

    #[test]
    fn a_machine_with_no_address_announces_nothing() {
        let silent = decide("kitchen-pi", &projection(Some("kitchen-pi"), &[], &[])).expect_err("none");
        assert_eq!(silent, Silent::NoAddress);
        assert!(silent.to_string().contains("resolve TO"), "{silent}");
    }

    /// A DHCP interface with NO lease yet is refused, and the refusal says so
    /// rather than leaving an operator to conclude the responder is broken.
    /// **It must not claim DHCP is unbuilt**, which it once did.
    #[test]
    fn a_dhcp_machine_with_no_lease_is_refused_with_the_reason() {
        let silent = decide("kitchen-pi", &projection(Some("kitchen-pi"), &[], &["eth0"])).expect_err("dhcp");
        match &silent {
            Silent::DhcpNoLease { interfaces } => assert_eq!(interfaces, &["eth0".to_string()]),
            other => panic!("{other:?}"),
        }
        let said = silent.to_string();
        assert!(said.contains("lease yet"), "{said}");
        assert!(said.contains("rivet-netd"), "{said}");
        assert!(!said.contains("not built"), "{said}");
    }

    #[test]
    fn two_addressed_interfaces_are_refused_rather_than_chosen_between() {
        let two = projection(
            Some("kitchen-pi"),
            &[("eth0", [10, 0, 0, 2]), ("wlan0", [10, 0, 1, 2])],
            &[],
        );
        let silent = decide("kitchen-pi", &two).expect_err("two");
        match &silent {
            Silent::Ambiguous { interfaces } => assert_eq!(interfaces.len(), 2),
            other => panic!("{other:?}"),
        }
        let said = silent.to_string();
        assert!(said.contains("eth0") && said.contains("wlan0"), "{said}");
    }

    /// One address beside a DHCP interface still waiting is not ambiguous:
    /// there is exactly one address, and it is the one that is knowable.
    #[test]
    fn one_address_beside_a_leaseless_interface_is_not_ambiguous() {
        let one = projection(Some("kitchen-pi"), &[("eth0", [10, 0, 0, 2])], &["wlan0"]);
        assert_eq!(decide("kitchen-pi", &one).expect("one").address, Ipv4Addr::new(10, 0, 0, 2));
    }

    // ---- the wire name -----------------------------------------------------

    #[test]
    fn the_zone_is_attached_once_and_the_label_cannot_carry_one() {
        let announcement = decide("kitchen-pi", &eth0("kitchen-pi")).expect("valid");
        assert_eq!(announcement.fqdn(), "kitchen-pi.local");
        assert_eq!(announcement.fqdn().matches(".local").count(), 1);
        assert_eq!(Hostname::new("kitchen-pi.local"), None);
    }

    #[test]
    fn the_ttl_is_the_one_rfc_6762_recommends() {
        assert_eq!(TTL_SECONDS, 120);
    }

    // ---- the answer to a query ---------------------------------------------

    fn kitchen() -> Announcement {
        decide("kitchen-pi", &eth0("kitchen-pi")).expect("valid")
    }

    /// A query as a resolver composes one, through the same library.
    fn query(id: u16, name: &str, (qtype, qclass): (QTYPE, QCLASS), unicast: bool) -> Vec<u8> {
        let mut packet = Packet::new_query(id);
        packet
            .questions
            .push(Question::new(Name::new(name).expect("a name"), qtype, qclass, unicast));
        packet.build_bytes_vec().expect("bytes")
    }

    /// **The one answer this responder exists to give**: an authoritative `A`
    /// record for `<hostname>.local`, with the RFC's TTL, to the group.
    #[test]
    fn a_query_for_the_machines_name_is_answered_with_its_address() {
        let (bytes, unicast) = reply(&query(0, "kitchen-pi.local", A_IN, false), &kitchen())
            .expect("an answer");
        assert!(!unicast, "a QM question is answered to the group");
        let answer = Packet::parse(&bytes).expect("a packet");
        assert!(answer.has_flags(PacketFlag::RESPONSE));
        assert!(answer.has_flags(PacketFlag::AUTHORITATIVE_ANSWER));
        assert_eq!(answer.answers.len(), 1);
        let record = &answer.answers[0];
        assert_eq!(record.name.to_string(), "kitchen-pi.local");
        assert_eq!(record.ttl, TTL_SECONDS);
        assert_eq!(record.rdata, RData::A(A::from(Ipv4Addr::new(192, 168, 1, 50))));
    }

    #[test]
    fn the_name_is_matched_without_regard_to_case_and_the_id_is_echoed() {
        let (bytes, _) = reply(&query(4242, "KITCHEN-PI.Local", A_IN, false), &kitchen()).expect("answered");
        assert_eq!(Packet::parse(&bytes).expect("a packet").id(), 4242);
    }

    #[test]
    fn a_question_asking_for_a_unicast_answer_gets_one() {
        let (_, unicast) = reply(&query(0, "kitchen-pi.local", A_IN, true), &kitchen()).expect("answered");
        assert!(unicast);
    }

    #[test]
    fn an_any_question_is_answered() {
        assert!(reply(&query(0, "kitchen-pi.local", (QTYPE::ANY, QCLASS::ANY), false), &kitchen()).is_some());
    }

    /// Everything that is not a question for this machine's address goes
    /// unanswered: another name, a parent of this one, another type, garbage,
    /// and a response — including this responder's own, looped back to it.
    #[test]
    fn what_is_not_a_query_for_this_machines_address_goes_unanswered() {
        let me = kitchen();
        assert!(reply(&query(0, "larder.local", A_IN, false), &me).is_none());
        assert!(reply(&query(0, "local", A_IN, false), &me).is_none());
        assert!(reply(&query(0, "kitchen-pi.local", (QTYPE::TYPE(TYPE::AAAA), QCLASS::CLASS(CLASS::IN)), false), &me).is_none());
        assert!(reply(b"not a dns message", &me).is_none());
        let (own, _) = reply(&query(0, "kitchen-pi.local", A_IN, false), &me).expect("answered");
        assert!(reply(&own, &me).is_none(), "its own answer, looped back, is not a question");
    }
}
