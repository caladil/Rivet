//! **The one module that names the dependency** — `ADR-009` §4's wall.
//!
//! [`catalog`] turns a [`crate::table::Zone`] into `hickory-server`'s catalog:
//! the zone as an in-memory primary, and a forwarder at the root for every
//! other name. The catalog dispatches by longest suffix, so a name under the
//! zone that the table does not hold is answered `NXDOMAIN` from the zone and
//! never reaches the forwarder (`ADR-047` §7).

use std::net::IpAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use hickory_server::Server;
use hickory_server::net::runtime::TokioRuntimeProvider;
use hickory_server::proto::rr::rdata::{A, NS, SOA};
use hickory_server::proto::rr::{Name, RData, Record};
use hickory_server::resolver::config::NameServerConfig;
use hickory_server::store::forwarder::{ForwardConfig, ForwardZoneHandler};
use hickory_server::store::in_memory::InMemoryZoneHandler;
use hickory_server::zone_handler::{AxfrPolicy, Catalog, ZoneType};

use crate::table::{UPSTREAM, Zone};

/// How long a TCP connection may sit without sending a request, the probe's
/// value. A client that retries after truncation sends at once.
pub const TCP_IDLE: Duration = Duration::from_secs(10);

/// Outgoing buffer per TCP connection: the largest message a two-byte length
/// prefix can carry.
pub const TCP_RESPONSE_BUFFER: usize = 65_535;

/// The zone's serial. Nothing transfers this zone (`ADR-047` §5 wants no
/// AXFR), so no secondary ever compares it and it never has to move.
const SERIAL: u32 = 1;

/// **How many upstreams the forwarder asks at once: one.** The dependency's
/// default asks two in parallel, and its pool then cannot fetch a large answer:
/// the first upstream's truncated UDP reply switches the query to TCP, and the
/// second upstream's truncated UDP reply, arriving after that switch, is read
/// as a TCP reply that was still truncated, so the query fails and the client
/// is answered `SERVFAIL`. Every answer over the forwarder's 1232-byte EDNS0
/// buffer is truncated by both upstreams, so it failed every time. Asked one at
/// a time, the truncated reply is retried over TCP to the same upstream, which
/// is `ADR-047` §3's second mechanism. The cost is that the second upstream is
/// asked after the first has failed rather than alongside it, so an upstream
/// that never answers costs a question the resolver's timeout before the other
/// is tried (`ADR-047` §3 records the trade).
const UPSTREAMS_AT_ONCE: usize = 1;

/// Why the catalog could not be built, in words a journal line can carry.
#[derive(Debug)]
pub struct NotBuilt(pub String);

impl core::fmt::Display for NotBuilt {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

fn name(text: &str) -> Result<Name, NotBuilt> {
    Name::from_str(text).map_err(|e| NotBuilt(format!("`{text}` is not a DNS name: {e}")))
}

/// The zone and the forwarder to [`UPSTREAM`], in one catalog.
///
/// # Errors
///
/// A name the table produced that the wire format will not take, or a
/// forwarder the dependency refuses to build.
pub fn catalog(zone: &Zone) -> Result<Catalog, NotBuilt> {
    catalog_forwarding_to(
        zone,
        UPSTREAM
            .iter()
            .map(|address| NameServerConfig::udp_and_tcp(IpAddr::V4(*address)))
            .collect(),
    )
}

/// [`catalog`], forwarding to the given upstreams. Separate so a test can put
/// a resolver on loopback in Quad9's place and still exercise every option
/// [`catalog`] sets.
fn catalog_forwarding_to(zone: &Zone, name_servers: Vec<NameServerConfig>) -> Result<Catalog, NotBuilt> {
    let origin = name(&zone.origin)?;
    let server = name(&zone.server)?;

    let mut handler = InMemoryZoneHandler::<TokioRuntimeProvider>::empty(
        origin.clone(),
        ZoneType::Primary,
        AxfrPolicy::Deny,
    );

    // The SOA's own TTL and its `minimum` are both the negative TTL: a
    // resolver caches an absence for the lower of the two (RFC 2308 §5).
    handler.upsert_mut(
        Record::from_rdata(
            origin.clone(),
            zone.negative_ttl,
            RData::SOA(SOA::new(
                server.clone(),
                name(&zone.mailbox)?,
                SERIAL,
                7200,
                3600,
                604_800,
                zone.negative_ttl,
            )),
        ),
        SERIAL,
    );
    // One NS at the apex: some clients treat a zone whose apex has none as
    // broken.
    handler.upsert_mut(Record::from_rdata(origin.clone(), zone.ttl, RData::NS(NS(server))), SERIAL);
    for record in &zone.records {
        handler.upsert_mut(
            Record::from_rdata(name(&record.name)?, record.ttl, RData::A(A::from(record.address))),
            SERIAL,
        );
    }

    let mut forwarder = ForwardZoneHandler::builder_tokio(ForwardConfig {
        name_servers,
        options: None,
    })
    .with_origin(Name::root());
    forwarder.options_mut().num_concurrent_reqs = UPSTREAMS_AT_ONCE;
    let forwarder = forwarder
        .build()
        .map_err(|e| NotBuilt(format!("the forwarder was not built: {e}")))?;

    let mut catalog = Catalog::new();
    catalog.upsert(origin.into(), vec![Arc::new(handler)]);
    catalog.upsert(Name::root().into(), vec![Arc::new(forwarder)]);
    Ok(catalog)
}

/// Serve `catalog` on the two sockets until the server stops.
///
/// # Errors
///
/// Whatever the dependency reports when its loop ends.
pub async fn run(
    catalog: Catalog,
    udp: tokio::net::UdpSocket,
    tcp: tokio::net::TcpListener,
) -> Result<(), NotBuilt> {
    let mut server = Server::new(catalog);
    server.register_socket(udp);
    server.register_listener(tcp, TCP_IDLE, TCP_RESPONSE_BUFFER);
    server
        .block_until_done()
        .await
        .map_err(|e| NotBuilt(format!("the server stopped: {e}")))
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    //! These run the real catalog on loopback and put real datagrams through
    //! it. A question inside the zone never reaches the forwarder; a forwarded
    //! one is answered by a stand-in upstream on loopback replaying bytes a real
    //! resolver sent, so none of them needs the internet.

    use super::*;
    use hickory_server::proto::op::{Edns, Message, Query, ResponseCode};
    use hickory_server::proto::rr::RecordType;
    use hickory_server::resolver::config::ConnectionConfig;
    use std::io::{Read, Write};
    use std::net::Ipv4Addr;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn zone() -> Zone {
        let doc = rivet_config::Document::parse(
            "version = 1\n[dns]\nzone = \"home-fleet.home.arpa\"\nnegative-ttl = 45\n\
             [dns.records.dev-01]\naddress = \"192.168.50.66\"\n\
             [dns.records.printer]\naddress = \"192.168.50.40\"\nttl = 30\n",
        )
        .unwrap();
        crate::table::zone(doc.dns().unwrap(), Some("dev-01")).unwrap()
    }

    /// A server on an ephemeral loopback port, and a client socket to ask it.
    async fn serving() -> (Server<Catalog>, tokio::net::UdpSocket) {
        let udp = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let at = udp.local_addr().unwrap();
        let mut server = Server::new(catalog(&zone()).unwrap());
        server.register_socket(udp);
        let client = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        client.connect(at).await.unwrap();
        (server, client)
    }

    fn question(qname: &str, qtype: RecordType) -> Vec<u8> {
        let mut m = Message::query();
        m.metadata.recursion_desired = true;
        m.add_query(Query::query(Name::from_str(qname).unwrap(), qtype));
        m.to_vec().unwrap()
    }

    async fn ask(client: &tokio::net::UdpSocket, bytes: &[u8]) -> Option<Message> {
        client.send(bytes).await.unwrap();
        let mut buf = vec![0u8; 4096];
        match tokio::time::timeout(Duration::from_millis(700), client.recv(&mut buf)).await {
            Ok(Ok(n)) => Some(Message::from_vec(&buf[..n]).unwrap()),
            Ok(Err(e)) => panic!("recv failed: {e}"),
            Err(_) => None,
        }
    }

    #[tokio::test]
    async fn a_name_in_the_table_answers_its_address_authoritatively_with_its_ttl() {
        let (_server, client) = serving().await;
        let reply = ask(&client, &question("dev-01.home-fleet.home.arpa.", RecordType::A)).await.unwrap();
        assert_eq!(reply.metadata.response_code, ResponseCode::NoError);
        assert!(reply.metadata.authoritative);
        let answers: Vec<(String, u32, RData)> = reply
            .answers
            .iter()
            .map(|r| (r.name.to_string(), r.ttl, r.data.clone()))
            .collect();
        assert_eq!(
            answers,
            [(
                "dev-01.home-fleet.home.arpa.".to_owned(),
                300,
                RData::A(A::from(Ipv4Addr::new(192, 168, 50, 66)))
            )]
        );

        let printer = ask(&client, &question("printer.home-fleet.home.arpa.", RecordType::A)).await.unwrap();
        assert_eq!(printer.answers.first().map(|r| r.ttl), Some(30), "a record's own TTL");
    }

    /// `ADR-047` §7's row that keeps a hole in our zone from leaking: a name
    /// under the zone the table does not hold is NXDOMAIN from the zone, with
    /// its SOA carrying the negative TTL — not a forwarded question.
    #[tokio::test]
    async fn a_gap_in_the_zone_is_nxdomain_with_the_negative_ttl() {
        let (_server, client) = serving().await;
        let reply = ask(&client, &question("nobody.home-fleet.home.arpa.", RecordType::A)).await.unwrap();
        assert_eq!(reply.metadata.response_code, ResponseCode::NXDomain);
        assert!(reply.metadata.authoritative, "answered by the zone, not forwarded");
        let soa = reply
            .authorities
            .iter()
            .find(|r| r.record_type() == RecordType::SOA)
            .expect("an SOA in the authority section");
        assert_eq!(soa.ttl, 45);
        match &soa.data {
            RData::SOA(s) => assert_eq!(s.minimum, 45),
            other => panic!("not an SOA: {other:?}"),
        }
    }

    /// `ADR-047` §7's finding, held: a datagram with `QR` set is a response,
    /// and answering one would make this a reflector. Nothing comes back — and
    /// the server still answers an ordinary question afterwards.
    #[tokio::test]
    async fn a_response_flagged_packet_is_not_answered() {
        let (_server, client) = serving().await;
        let mut bytes = question("dev-01.home-fleet.home.arpa.", RecordType::A);
        bytes[2] |= 0x80;
        assert!(ask(&client, &bytes).await.is_none(), "a response was answered");
        let after = ask(&client, &question("dev-01.home-fleet.home.arpa.", RecordType::A)).await;
        assert_eq!(after.map(|m| m.metadata.response_code), Some(ResponseCode::NoError));
    }

    // ---- forwarding an answer too large for a datagram -----------------------

    /// **Captured from Quad9, not composed here.** `apple.com TXT`, asked of
    /// 9.9.9.9 with an EDNS0 buffer of 1232 — the forwarder's own default. Over
    /// UDP it answered 38 bytes: the header with `TC` set, the question, its
    /// OPT record and no answers. Over TCP the same question was answered with
    /// 2149 bytes and 23 records. Both are hex, one message per file.
    const QUAD9_TXT_UDP: &str = include_str!("fixtures/quad9-apple-com-txt.udp.hex");
    const QUAD9_TXT_TCP: &str = include_str!("fixtures/quad9-apple-com-txt.tcp.hex");

    fn unhex(text: &str) -> Vec<u8> {
        let text = text.trim();
        (0..text.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
            .collect()
    }

    /// What one stand-in upstream was asked, by transport.
    #[derive(Default)]
    struct Asked {
        udp: AtomicUsize,
        tcp: AtomicUsize,
    }

    /// **A stand-in for one of Quad9's two addresses**, on loopback: every UDP
    /// question is answered with `udp_reply` and every TCP one with
    /// `tcp_reply`, each carrying the question's own ID, as the real one did.
    fn upstream(ip: Ipv4Addr, udp_reply: Vec<u8>, tcp_reply: Vec<u8>) -> (NameServerConfig, Arc<Asked>) {
        let asked = Arc::new(Asked::default());
        let udp = std::net::UdpSocket::bind((ip, 0)).unwrap();
        let tcp = std::net::TcpListener::bind((ip, 0)).unwrap();
        let (udp_port, tcp_port) = (udp.local_addr().unwrap().port(), tcp.local_addr().unwrap().port());

        let counted = Arc::clone(&asked);
        std::thread::spawn(move || {
            let mut buffer = [0u8; 1500];
            while let Ok((n, from)) = udp.recv_from(&mut buffer) {
                if n < 2 {
                    continue;
                }
                counted.udp.fetch_add(1, Ordering::SeqCst);
                let mut reply = udp_reply.clone();
                reply[..2].copy_from_slice(&buffer[..2]);
                let _ = udp.send_to(&reply, from);
            }
        });

        let counted = Arc::clone(&asked);
        std::thread::spawn(move || {
            for stream in tcp.incoming() {
                let Ok(mut stream) = stream else { return };
                let (reply, counted) = (tcp_reply.clone(), Arc::clone(&counted));
                std::thread::spawn(move || {
                    loop {
                        let mut length = [0u8; 2];
                        if stream.read_exact(&mut length).is_err() {
                            return;
                        }
                        let mut query = vec![0u8; usize::from(u16::from_be_bytes(length))];
                        if stream.read_exact(&mut query).is_err() || query.len() < 2 {
                            return;
                        }
                        counted.tcp.fetch_add(1, Ordering::SeqCst);
                        let mut message = reply.clone();
                        message[..2].copy_from_slice(&query[..2]);
                        let mut framed = u16::try_from(message.len()).unwrap().to_be_bytes().to_vec();
                        framed.extend(message);
                        if stream.write_all(&framed).is_err() {
                            return;
                        }
                    }
                });
            }
        });

        let mut over_udp = ConnectionConfig::udp();
        over_udp.port = udp_port;
        let mut over_tcp = ConnectionConfig::tcp();
        over_tcp.port = tcp_port;
        (NameServerConfig::new(IpAddr::V4(ip), true, vec![over_udp, over_tcp]), asked)
    }

    /// **`ADR-047` §3's second mechanism, with both upstreams in place.** An
    /// answer larger than the forwarder's 1232-byte EDNS0 buffer arrives from
    /// each upstream truncated, is fetched again over TCP, and reaches a client
    /// whose own buffer holds it whole — every record, `NOERROR`, not
    /// `SERVFAIL`.
    #[tokio::test]
    async fn a_forwarded_answer_too_large_for_a_datagram_is_fetched_over_tcp_and_relayed_whole() {
        let (udp, tcp) = (unhex(QUAD9_TXT_UDP), unhex(QUAD9_TXT_TCP));
        let expected = Message::from_vec(&tcp).unwrap().answers.len();
        assert!(Message::from_vec(&udp).unwrap().metadata.truncation, "the UDP fixture is the truncated one");

        let (first, first_asked) = upstream(Ipv4Addr::new(127, 0, 0, 1), udp.clone(), tcp.clone());
        let (second, second_asked) = upstream(Ipv4Addr::new(127, 0, 0, 2), udp, tcp);
        let socket = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let at = socket.local_addr().unwrap();
        let mut server = Server::new(catalog_forwarding_to(&zone(), vec![first, second]).unwrap());
        server.register_socket(socket);
        let client = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        client.connect(at).await.unwrap();

        let mut query = Message::query();
        query.metadata.recursion_desired = true;
        query.add_query(Query::query(Name::from_str("apple.com.").unwrap(), RecordType::TXT));
        let mut edns = Edns::new();
        edns.set_max_payload(4096);
        query.set_edns(edns);
        client.send(&query.to_vec().unwrap()).await.unwrap();
        let mut buffer = vec![0u8; 4096];
        let n = tokio::time::timeout(Duration::from_secs(10), client.recv(&mut buffer))
            .await
            .expect("the forwarded question was never answered")
            .unwrap();
        let reply = Message::from_vec(&buffer[..n]).unwrap();

        let tcp_asked = first_asked.tcp.load(Ordering::SeqCst) + second_asked.tcp.load(Ordering::SeqCst);
        assert_eq!(
            reply.metadata.response_code,
            ResponseCode::NoError,
            "upstream asked over TCP {tcp_asked} time(s)"
        );
        assert!(!reply.metadata.truncation, "the client's 4096-byte buffer holds it");
        assert_eq!(reply.answers.len(), expected);
        assert!(tcp_asked >= 1, "the whole answer can only have come over TCP");
    }
}
