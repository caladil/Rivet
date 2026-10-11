//! **Try a loopback TCP connection to each port, as this application's own
//! uid, and print what happened.** `ADR-023` §3A.
//!
//! The guard drops a new loopback connection to a proxied application's port
//! unless it comes from the proxy or root. An application runs as its own uid,
//! so this program is exactly the caller the guard exists to refuse, and a
//! dropped connection reads here as a timeout rather than a refusal: `drop`
//! sends nothing back.
//!
//! The ports are the arguments a build gives it, [`TARGETS`]. Each line names
//! the port, what happened and how long it took, so a reader of the journal
//! can tell *refused by the firewall* (a timeout) from *nothing listening* (an
//! immediate refusal) from *reached* (connected).

use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

/// The ports tried, in order.
///
/// * **8080** is a routed port when an application behind the proxy declares
///   it, which is the case the guard covers.
/// * **7777** is the control plane's port, which the guard never covers. A
///   connection there shows this program can make a loopback connection at
///   all, so a timeout on the first is the firewall and not the probe.
const TARGETS: [u16; 2] = [8080, 7777];

/// Long enough for a live listener on loopback by orders of magnitude, short
/// enough that a dropped SYN is reported rather than waited out.
const PATIENCE: Duration = Duration::from_secs(3);

fn main() {
    for port in TARGETS {
        let address = SocketAddr::from(([127, 0, 0, 1], port));
        let began = Instant::now();
        let outcome = match TcpStream::connect_timeout(&address, PATIENCE) {
            Ok(_) => "connected".to_string(),
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                format!("timed out after {}s - nothing answered, as a dropped SYN looks", PATIENCE.as_secs())
            }
            Err(e) => format!("refused at once: {e}"),
        };
        println!(
            "loopback-probe: {address} {outcome} ({} ms)",
            began.elapsed().as_millis()
        );
    }
}
