//! `rivet-dnsd` — **the fleet's names, answered from a table, and everything
//! else forwarded.** `ADR-047`.
//!
//! ```text
//! rivet-dnsd --document /state/app/rivet-dnsd/dns.toml
//! ```
//!
//! It runs as its own uid under a Landlock ruleset, and the document it is
//! given is the supervisor's projection of the machine document — `version`,
//! `hostname` and `[dns]`, nothing else (`ADR-047` §6). It reads that once, at
//! start, builds the zone with the hostname as the server the SOA and NS
//! records name, and serves it on the UDP and TCP sockets the supervisor bound
//! on port 53. It binds nothing itself; a process that is not root could not
//! bind port 53. A
//! name under the zone is answered from the table — `NXDOMAIN` with the zone's
//! SOA when the table does not hold it — and every other name is forwarded to
//! Quad9 (`crate::table::UPSTREAM`). A change to the table is served after the
//! service restarts.
//!
//! # What a refusal means
//!
//! It exits non-zero, naming the reason, when the document cannot be read,
//! has no `[dns]` table, names no usable hostname, or the sockets were not
//! handed over. Each is a
//! statement about the machine that repeats unchanged, which is why the
//! definition's restart policy is `on-failure` and the supervisor gives up
//! after a few attempts rather than filling the journal.

mod serve;
mod table;

use std::process::ExitCode;

/// What `LISTEN_FDNAMES` calls the two descriptors (`rivet-sdk/src/listen.rs`).
const UDP_NAME: &str = "udp-53";
const TCP_NAME: &str = "tcp-53";

const REFUSED: u8 = 1;

fn refuse(message: &str) -> ExitCode {
    eprintln!("rivet-dnsd: REFUSED: {message}");
    ExitCode::from(REFUSED)
}

/// `--document PATH`, and nothing else.
fn parse(argv: &[String]) -> Result<String, String> {
    match argv {
        [flag, path] if flag == "--document" => Ok(path.clone()),
        _ => Err("usage: rivet-dnsd --document PATH".to_owned()),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let document_path = match parse(std::env::args().skip(1).collect::<Vec<_>>().as_slice()) {
        Ok(path) => path,
        Err(message) => return refuse(&message),
    };
    let text = match std::fs::read_to_string(&document_path) {
        Ok(text) => text,
        Err(e) => return refuse(&format!("cannot read {document_path}: {e}")),
    };
    let document = match rivet_config::Document::parse(&text) {
        Ok(document) => document,
        Err(e) => return refuse(&format!("{document_path} is not a document this build can read: {e}")),
    };
    let Some(dns) = document.dns() else {
        return refuse(&format!(
            "{document_path} has no [dns] table, so there is no zone to answer for. Write one \
             (`[dns] zone = \"<fleet>.home.arpa\"` and a `[dns.records.<label>]` per name, \
             ADR-047 §8) or set `[services.rivet-dnsd] enabled = false`."
        ));
    };

    // **The machine's name comes from the same handed document**, never from
    // the kernel: this process is confined, and a read it was refused must not
    // quietly become a different server name (`ADR-047` §8).
    let zone = match table::zone(dns, document.hostname().map(|name| name.as_str())) {
        Ok(zone) => zone,
        Err(e) => return refuse(&format!("{document_path}: {e}")),
    };
    let catalog = match serve::catalog(&zone) {
        Ok(catalog) => catalog,
        Err(e) => return refuse(&e.to_string()),
    };

    // **The UDP handoff, and the middle call is the one that matters**
    // (`ADR-047` §6): the supervisor binds with `std::net::UdpSocket::bind` and
    // sets no options, so the descriptor arrives blocking, and tokio does not
    // refuse a blocking socket — it hangs on it.
    let udp = match rivet_sdk::listen::socket(UDP_NAME).and_then(rivet_sdk::listen::Socket::into_udp_socket) {
        Some(socket) => socket,
        None => return refuse(&format!("no UDP socket named {UDP_NAME} was handed to this process")),
    };
    let tcp = match rivet_sdk::listen::socket(TCP_NAME).and_then(rivet_sdk::listen::Socket::into_tcp_listener) {
        Some(socket) => socket,
        None => return refuse(&format!("no TCP socket named {TCP_NAME} was handed to this process")),
    };
    if let Err(e) = udp.set_nonblocking(true).and_then(|()| tcp.set_nonblocking(true)) {
        return refuse(&format!("the handed sockets could not be made non-blocking: {e}"));
    }
    let udp = match tokio::net::UdpSocket::from_std(udp) {
        Ok(socket) => socket,
        Err(e) => return refuse(&format!("the UDP socket was not taken: {e}")),
    };
    let tcp = match tokio::net::TcpListener::from_std(tcp) {
        Ok(socket) => socket,
        Err(e) => return refuse(&format!("the TCP socket was not taken: {e}")),
    };

    println!(
        "rivet-dnsd: SERVING {} with {} record(s) (ttl {}, negative-ttl {}); every other name \
         is forwarded to {:?}",
        zone.origin,
        zone.records.len(),
        zone.ttl,
        zone.negative_ttl,
        table::UPSTREAM
    );
    for record in &zone.records {
        println!("rivet-dnsd: {} A {} (ttl {})", record.name, record.address, record.ttl);
    }

    match serve::run(catalog, udp, tcp).await {
        Ok(()) => refuse("the server stopped without an error, and a name service that stops has stopped serving"),
        Err(e) => refuse(&e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn words(w: &[&str]) -> Vec<String> {
        w.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_document_path_is_the_only_argument() {
        assert_eq!(
            parse(&words(&["--document", "/state/config/machine.toml"])).as_deref(),
            Ok("/state/config/machine.toml")
        );
        for bad in [&[][..], &["--document"][..], &["--doc", "x"][..], &["--document", "x", "y"][..]] {
            assert!(parse(&words(bad)).is_err(), "{bad:?} was accepted");
        }
    }
}
