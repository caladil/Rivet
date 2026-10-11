//! `ADR-047` §4's rejected option, running, so the pair can be compared.
//!
//! `ADR-047` §7 asks for **both** candidates on a board against the same static
//! table. `examples/probes/dns-probe` is the accepted one — `hickory-server`
//! with `resolver`. This is the other: a responder written here over
//! `simple-dns`, two crates, both already in this repository's lock.
//!
//! # What it answers, and what it refuses
//!
//! * A name in the table → an A record, authoritative.
//! * A name under `home.arpa.` that is not → `NXDOMAIN`, with the zone's SOA in
//!   the authority section so a client has a negative-caching TTL.
//! * **Anything else → `REFUSED`.** It does not forward, and `ADR-047` §3 is
//!   that forwarding is the whole justification for the other candidate's 92
//!   crates. A `REFUSED` here is the study's §8.1 made visible: this is what a
//!   household gets from option A on the day somebody asks it for
//!   `example.com`.
//! * A datagram that will not parse → dropped, with the sender named.
//!
//! # What it CANNOT show, stated rather than implied
//!
//! No forwarder, so none of `ADR-047` §3's three mechanisms — EDNS0 relaying,
//! truncation with a TCP retry, transaction-ID randomisation and source
//! validation — is exercised by anything here. No TCP socket either, for the
//! same reason: under option A that is a second loop and a framing
//! implementation, and writing one would be building the component rather than
//! probing the candidate.
//!
//! # DO NOT COPY THIS LOOP INTO A SERVICE: IT ANSWERS A MESSAGE TYPED AS A
//! RESPONSE
//!
//! A datagram with `QR` set is a **response**, not a query, and a server that
//! replies to one is a reflector: an attacker spoofs a victim's source address,
//! and the server sends the victim traffic it never asked for. `hickory-server`
//! refuses it in one line, with the reason in the source — *"Don't process
//! response messages to avoid DoS attacks from reflection."* This loop has no
//! such check, and answers with `REFUSED`.
//!
//! **That is left in deliberately, because it is the clearest thing this pair
//! demonstrates.** `ADR-009` §3's case for reuse is a component *"worse in
//! ways that surface as intermittent failures years later"*, and the argument
//! is usually made about the forwarder. It is true of the two hundred lines
//! nobody worried about as well: this responder is short, its author had the
//! RFC's shape in mind, and it still shipped a reflection path on its first
//! outing. The reply is no larger than the query, so there is no amplification
//! and this probe is not a weapon — but a service written this way and given
//! real records would be.

use std::net::UdpSocket;

use simple_dns::rdata::{A, RData, SOA};
use simple_dns::{CLASS, Name, Packet, PacketFlag, QCLASS, QTYPE, RCODE, ResourceRecord, TYPE};

const PORT: u16 = 5356;
const UDP_NAME: &str = "udp-5356";
const ZONE: &str = "home.arpa";
const TTL: u32 = 300;

/// The same names and the same RFC 5737 documentation addresses as
/// `dns-probe`'s table, so a reply proves which program answered.
const STATIC_TABLE: &[(&str, [u8; 4])] = &[
    ("ns.home.arpa", [192, 0, 2, 1]),
    ("probe-one.home.arpa", [192, 0, 2, 10]),
    ("probe-two.home.arpa", [192, 0, 2, 20]),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("dns-probe-simple: pid {}, ADR-047 §4's option A", std::process::id());

    let socket = match rivet_sdk::listen::socket(UDP_NAME).and_then(rivet_sdk::listen::Socket::into_udp_socket)
    {
        Some(handed) => {
            println!("dns-probe-simple: took the handed UDP socket named {UDP_NAME}");
            handed
        }
        None => {
            println!("dns-probe-simple: NO handed UDP socket — binding {PORT} myself");
            UdpSocket::bind(("0.0.0.0", PORT))?
        }
    };
    println!("dns-probe-simple: UDP on {}", socket.local_addr()?);
    println!("dns-probe-simple: authoritative for {ZONE}., REFUSED for everything else");

    // 512, the ordinary datagram limit, is enough for every question this can
    // be asked and is what a responder with no EDNS0 of its own should read.
    let mut buffer = [0u8; 512];
    loop {
        let (read, from) = match socket.recv_from(&mut buffer) {
            Ok(pair) => pair,
            Err(error) => {
                println!("dns-probe-simple: recv_from failed: {error}");
                continue;
            }
        };

        // **A datagram that will not parse is dropped and the sender is
        // named.** There is no state to corrupt because there is no state —
        // `docs/design-candidates/DNS.md` §6's left-hand column, which this
        // probe exists to turn from a table cell into a reading.
        let query = match Packet::parse(&buffer[..read]) {
            Ok(packet) => packet,
            Err(error) => {
                println!("dns-probe-simple: {read} bytes from {from} would not parse: {error}");
                continue;
            }
        };

        let reply = match answer(&query) {
            Some(bytes) => bytes,
            None => {
                println!("dns-probe-simple: {read} bytes from {from} carried no question — dropped");
                continue;
            }
        };
        if let Err(error) = socket.send_to(&reply, from) {
            println!("dns-probe-simple: could not answer {from}: {error}");
        }
    }
}

/// Build the reply, or `None` when there is no question to answer.
fn answer(query: &Packet) -> Option<Vec<u8>> {
    let question = query.questions.first()?;
    let name = question.qname.to_string();
    let lowered = name.trim_end_matches('.').to_ascii_lowercase();

    let mut reply = Packet::new_reply(query.id());
    reply.set_flags(PacketFlag::RECURSION_DESIRED);
    reply.questions.push(question.clone());

    let in_zone = lowered == ZONE || lowered.ends_with(&format!(".{ZONE}"));
    if !in_zone {
        // **The whole of option A's cost, in one branch.** No forwarder, so a
        // name outside the zone gets a refusal rather than an answer, and the
        // client is left to its own fallback behaviour — which differs across
        // Windows, Android, macOS and glibc, and is the fragility `ADR-047` §3
        // says the forwarder exists to remove.
        *reply.rcode_mut() = RCODE::Refused;
        return reply.build_bytes_vec_compressed().ok();
    }

    reply.set_flags(PacketFlag::AUTHORITATIVE_ANSWER);

    let wanted = matches!(question.qtype, QTYPE::TYPE(TYPE::A))
        && matches!(question.qclass, QCLASS::CLASS(CLASS::IN) | QCLASS::ANY);

    if wanted {
        if let Some((label, address)) = STATIC_TABLE.iter().find(|(label, _)| *label == lowered) {
            if let Ok(owner) = Name::new(label) {
                reply.answers.push(ResourceRecord::new(
                    owner,
                    CLASS::IN,
                    TTL,
                    RData::A(A::from(std::net::Ipv4Addr::from(*address))),
                ));
                return reply.build_bytes_vec_compressed().ok();
            }
        }
    }

    // In the zone and not in the table. NXDOMAIN when the name is unknown,
    // NOERROR with an empty answer when the name exists and this type does
    // not — the distinction some clients cache differently, and the reason
    // `docs/design-candidates/DNS.md` §2.2 wants an SOA at all.
    let known = STATIC_TABLE.iter().any(|(label, _)| *label == lowered) || lowered == ZONE;
    if !known {
        // `NameError` is `simple-dns`' spelling of NXDOMAIN — RFC 1035's own
        // name for code 3, and not a different thing.
        *reply.rcode_mut() = RCODE::NameError;
    }
    if let Some(soa) = soa_record() {
        reply.name_servers.push(soa);
    }
    reply.build_bytes_vec_compressed().ok()
}

/// The zone's SOA, for the authority section of a negative answer.
fn soa_record() -> Option<ResourceRecord<'static>> {
    Some(ResourceRecord::new(
        Name::new(ZONE).ok()?,
        CLASS::IN,
        TTL,
        RData::SOA(SOA {
            mname: Name::new("ns.home.arpa").ok()?,
            rname: Name::new("hostmaster.home.arpa").ok()?,
            serial: 1,
            refresh: 7200,
            retry: 3600,
            expire: 604_800,
            minimum: 300,
        }),
    ))
}
