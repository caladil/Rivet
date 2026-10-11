//! `ADR-047` §7's experiment: the accepted DNS candidate, on a board, answering
//! real queries.
//!
//! `ADR-047` chose `hickory-server` with the `resolver` feature on a study that
//! measured builds, dependency trees, licences and binary sizes and ran
//! nothing. Its §7 says so in as many words — malformed-packet behaviour was
//! read from the API rather than exercised, and execution on the Cortex-A72 is
//! *"a judgement about two closures that ship no assembly"*. This program is
//! what turns those two sentences into readings.
//!
//! # What it is not
//!
//! It is not the name service. There is no configuration schema, no
//! `rivet-config` key, and no wall keeping `hickory` types out of Rivet's own
//! vocabulary (`ADR-009` §4). The static table below is hardcoded on purpose:
//! a schema is real work that says nothing about whether the dependency runs.
//!
//! # The four things it is driven with
//!
//! 1. **A name the table holds** — answered from the zone.
//! 2. **A name it does not** — forwarded to Quad9, which is the mechanism the
//!    whole dependency was bought for (`ADR-047` §3).
//! 3. **An answer large enough to truncate**, so `TC` comes back set and the
//!    client retries over TCP. That is why this declares a TCP socket as well:
//!    `ADR-047` §3 calls a UDP-only server a dead end here.
//! 4. **A malformed datagram.**
//!
//! # The addresses in the table are TEST-NET-1, and that is evidence
//!
//! RFC 5737 reserves `192.0.2.0/24` for documentation and no resolver on earth
//! answers from it. So an answer of `192.0.2.10` **cannot** have come from
//! anywhere but this table, which is what makes the first query prove something
//! rather than merely succeed.
//!
//! # Two properties of the dependency that a reader of `ADR-047` will not
//! expect
//!
//! * **The forwarder CACHES, and nothing here asked it to.** `ADR-047` §3 calls
//!   a cache *"the honest omission for a first version"* and
//!   `docs/design-candidates/DNS.md` §9 lists it as a decision still owed. It
//!   is not one: `ForwardZoneHandler` is `hickory-resolver` behind a zone
//!   handler, and a resolver caches by default. The composition below has no
//!   cache setting in it and the deployed program still counts a cached TTL
//!   down between queries.
//! * **A truncated UDP answer carries the bytes of the record it dropped.**
//!   When an answer exceeds the client's buffer, `TC` is set and the counts are
//!   reduced — and the datagram is still filled to the limit, so what follows
//!   the last counted record is the partially serialised next one. Tolerant
//!   parsers ignore it and retry over TCP, which is the behaviour that matters;
//!   a strict parser rejects the datagram outright.

use std::net::Ipv4Addr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use hickory_server::Server;
use hickory_server::proto::rr::rdata::{A, NS, SOA};
use hickory_server::proto::rr::{Name, RData, Record};
use hickory_server::resolver::config::NameServerConfig;
use hickory_server::store::forwarder::{ForwardConfig, ForwardZoneHandler};
use hickory_server::store::in_memory::InMemoryZoneHandler;
use hickory_server::zone_handler::{AxfrPolicy, Catalog, ZoneType};

/// The port, and the reason it is not 53.
///
/// `ADR-047` §2 keeps the name service off the development board, because a
/// machine that takes two A/B updates in a day is not the machine a household's
/// name resolution lives on. That rule is about a resolver the router
/// advertises; a port nothing looks for cannot be stumbled onto, so a query
/// reaches this program only when somebody sends it here on purpose.
const PORT: u16 = 5354;

/// What the supervisor calls the descriptors — `udp-<port>` and `tcp-<port>`,
/// not the manifest's `udp:5354`, because `LISTEN_FDNAMES` is colon-separated
/// (`rivet-sdk/src/listen.rs`).
const UDP_NAME: &str = "udp-5354";
const TCP_NAME: &str = "tcp-5354";

/// How long a TCP connection may sit without sending a request.
const TCP_IDLE: Duration = Duration::from_secs(10);

/// Outgoing response buffer per TCP connection. 65535 is the largest a DNS
/// message over TCP can be, its two-byte length prefix being a `u16`.
const TCP_RESPONSE_BUFFER: usize = 65_535;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("dns-probe: pid {}, ADR-047 §7's experiment", std::process::id());

    let catalog = catalog()?;
    let mut server = Server::new(catalog);

    // **The UDP handoff, and the middle line is the one that matters.**
    // `ADR-047` §6: the supervisor binds with `std::net::UdpSocket::bind` and
    // sets no options, so the descriptor arrives blocking.
    // `tokio::net::UdpSocket::from_std` does not refuse a blocking socket — it
    // puts the duty on the caller — so omitting `set_nonblocking` fails at run
    // time and not at compile time.
    let udp = match rivet_sdk::listen::socket(UDP_NAME).and_then(rivet_sdk::listen::Socket::into_udp_socket) {
        Some(handed) => {
            println!("dns-probe: took the handed UDP socket named {UDP_NAME}");
            handed.set_nonblocking(true)?;
            tokio::net::UdpSocket::from_std(handed)?
        }
        None => {
            // The fallback `ADR-015` §5 asks every application to carry, so
            // one binary works before and after the feature exists. On a Rivet
            // this branch is never taken; run by hand it is the only one.
            println!("dns-probe: NO handed UDP socket — binding {PORT} myself");
            tokio::net::UdpSocket::bind(("0.0.0.0", PORT)).await?
        }
    };
    println!("dns-probe: UDP on {}", udp.local_addr()?);
    server.register_socket(udp);

    // **TCP, because truncation implies it.** `ADR-047` §3.
    let tcp = match rivet_sdk::listen::socket(TCP_NAME).and_then(rivet_sdk::listen::Socket::into_tcp_listener) {
        Some(handed) => {
            println!("dns-probe: took the handed TCP socket named {TCP_NAME}");
            handed.set_nonblocking(true)?;
            tokio::net::TcpListener::from_std(handed)?
        }
        None => {
            println!("dns-probe: NO handed TCP socket — binding {PORT} myself");
            tokio::net::TcpListener::bind(("0.0.0.0", PORT)).await?
        }
    };
    println!("dns-probe: TCP on {}", tcp.local_addr()?);
    server.register_listener(tcp, TCP_IDLE, TCP_RESPONSE_BUFFER);

    println!("dns-probe: serving home.arpa. from a static table, forwarding the rest to 9.9.9.9");
    server.block_until_done().await?;
    Ok(())
}

/// The static zone and the forwarder, in one catalog.
///
/// `Catalog` dispatches by longest-suffix match, so `home.arpa.` reaches the
/// in-memory zone and everything else falls through to the forwarder registered
/// at the root. That dispatch is one of the things `ADR-047` §4 counted as
/// coming for free and nobody here had watched happen.
fn catalog() -> Result<Catalog, Box<dyn std::error::Error>> {
    let origin = Name::from_str("home.arpa.")?;
    let ns_name = Name::from_str("ns.home.arpa.")?;

    let mut zone = InMemoryZoneHandler::<hickory_server::net::runtime::TokioRuntimeProvider>::empty(
        origin.clone(),
        ZoneType::Primary,
        // No zone transfer. `ADR-047` §5 does not want one, and
        // `docs/design-candidates/DNS.md` §1.1's second-server argument depends
        // on there being none.
        AxfrPolicy::Deny,
    );

    // An SOA, because a zone without one has no negative-caching TTL and an
    // NXDOMAIN from here would be cached for however long each client feels
    // like — `docs/design-candidates/DNS.md` §2.2.
    zone.upsert_mut(
        Record::from_rdata(
            origin.clone(),
            300,
            RData::SOA(SOA::new(
                ns_name.clone(),
                Name::from_str("hostmaster.home.arpa.")?,
                1,
                7200,
                3600,
                604_800,
                300,
            )),
        ),
        1,
    );
    // One NS at the apex: some clients treat a zone whose apex answers no NS as
    // broken.
    zone.upsert_mut(Record::from_rdata(origin.clone(), 300, RData::NS(NS(ns_name.clone()))), 1);

    for (label, address) in STATIC_TABLE {
        zone.upsert_mut(
            Record::from_rdata(Name::from_str(label)?, 300, RData::A(A::from(*address))),
            1,
        );
    }

    // **The forwarder — the half `ADR-047` §3 says the dependency exists for.**
    // EDNS0 relaying, TCP on truncation, random transaction IDs, source
    // validation and a bounded outstanding-query table are all inside this
    // call, and none of them was ever exercised before this probe ran.
    let forwarder = ForwardZoneHandler::builder_tokio(ForwardConfig {
        name_servers: vec![NameServerConfig::udp_and_tcp(Ipv4Addr::new(9, 9, 9, 9).into())],
        options: None,
    })
    .with_origin(Name::root())
    .build()?;

    let mut catalog = Catalog::new();
    catalog.upsert(origin.into(), vec![Arc::new(zone)]);
    catalog.upsert(Name::root().into(), vec![Arc::new(forwarder)]);
    Ok(catalog)
}

/// The household's names, as a probe has them: hardcoded, ASCII labels under
/// one suffix, and addresses from RFC 5737's documentation range so that an
/// answer carrying one could not have come from anywhere else.
const STATIC_TABLE: &[(&str, Ipv4Addr)] = &[
    ("ns.home.arpa.", Ipv4Addr::new(192, 0, 2, 1)),
    ("probe-one.home.arpa.", Ipv4Addr::new(192, 0, 2, 10)),
    ("probe-two.home.arpa.", Ipv4Addr::new(192, 0, 2, 20)),
];
