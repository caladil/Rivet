//! **An address from a DHCP server**, through `udhcpc` run as a supervised
//! service, and the lease it leaves behind.
//!
//! # Who does what
//!
//! | Program | Its job | Its lifetime |
//! | :-- | :-- | :-- |
//! | `udhcpc`, one per DHCP interface | speaks DHCP: discovers, takes a lease, renews it at T1, rebinds at T2 | a service `cmd/rivet-sysd` keeps running and restarts, started with [`arguments`] |
//! | `cmd/rivet-dhcp-hook` | the program `udhcpc` execs per event: writes the lease to [`crate::lease::lease_path`], or removes it, and nudges the supervisor | one event |
//! | `cmd/rivet-netd` | reads the lease with [`current_lease`] and applies its address and route over netlink | runs once, and is run again whenever a lease changes |
//!
//! **Every netlink write stays in `rivet-netd`** (`ADR-011`): the client
//! configures nothing, the hook only records, and the one program that sets
//! an interface's address is the one that sets a static address too. The
//! supervisor re-runs `rivet-netd` when the lease file says something new,
//! which is how a renewal that changes the address reaches the interface.
//!
//! # Why the client is a service and not netd's child
//!
//! A client netd started was in netd's cgroup, so when netd exited the
//! supervisor's sweep of that cgroup signalled it, and the lease was never
//! renewed. A supervised client has its own cgroup, its own captured output,
//! and a restart when it dies — the three things a long-lived mechanism needs
//! and a one-shot program's child cannot have.

use rivet_core::net::InterfaceName;

use crate::lease::{Lease, lease_path};

/// The DHCP client, and where the image puts it.
///
/// **Spelled here and in `cmd/rivet-image`'s `UDHCPC_PATH`.** The same
/// arrangement every other program this project execs already has: `ADR-014`
/// §3 keeps the image builder out of the adapter's dependencies, so the
/// agreement is a convention the card build enforces by putting the file there.
pub const MECHANISM: &str = "/system/bin/udhcpc";

/// How many DISCOVERs `udhcpc` sends in one round — its `-t`.
///
/// **BusyBox's default is 3, and 3 is short on real hardware**, where a link
/// can take seconds to come up after it is raised and swallow every attempt.
/// With no `-n` the client does not give up after a round: it waits and starts
/// another, for as long as the service runs.
pub const DISCOVER_ATTEMPTS: &str = "8";

/// The client's argument vector for one interface.
///
/// * `-i` — the interface.
/// * `-f` — the foreground, so the supervisor holds the process it started
///   rather than a parent that forked and left.
/// * `-t` — [`DISCOVER_ATTEMPTS`].
///
/// **No `-q` and no `-n`.** `-q` quits once a lease is taken, which ends
/// renewal; `-n` quits when no server answers, which turns a router that is
/// slow to come back after a power cut into a service the supervisor gives up
/// on. Without either the client stays, retries, and renews for as long as it
/// runs.
///
/// **No `-s`.** The script is compiled into the binary as
/// `CONFIG_UDHCPC_DEFAULT_SCRIPT` (`tools/buildroot/busybox/busybox.config`),
/// and naming it here would be a second place that has to agree with it.
#[must_use]
pub fn arguments(interface: &str) -> Vec<String> {
    vec![
        "-i".to_owned(),
        interface.to_owned(),
        "-f".to_owned(),
        "-t".to_owned(),
        DISCOVER_ATTEMPTS.to_owned(),
    ]
}

/// Why a DHCP interface has no address to apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DhcpUnavailable {
    /// The client is not in this image.
    ///
    /// A card built without `--udhcpc` is a supported card, and this is what it
    /// says rather than a crash.
    MechanismNotBuilt {
        interface: String,
        /// The program that would do it. Carried rather than written into the
        /// message so the two cannot drift apart (`ADR-009` §2).
        mechanism: &'static str,
    },
    /// The lease was written and could not be read back.
    Unreadable { interface: String, detail: String },
}

impl core::fmt::Display for DhcpUnavailable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MechanismNotBuilt {
                interface,
                mechanism,
            } => write!(
                f,
                "DHCP on {interface} is not available: Rivet drives {mechanism} (ADR-004 §8) and \
                 this image does not contain it. A card is built with it by passing --udhcpc to \
                 cmd/rivet-image; without it, configure a static address instead"
            ),
            Self::Unreadable { interface, detail } => write!(
                f,
                "DHCP on {interface}: a lease was written and cannot be read back — {detail}. \
                 NOTHING WAS CONFIGURED, because an address assembled from a damaged lease is a \
                 machine that looks reachable and is not"
            ),
        }
    }
}

/// **The lease one interface holds right now**, if it holds one.
///
/// `read` answers the contents of a file or `None` when it is absent, and
/// `mechanism_present` whether the client is on this card at all; both are
/// arguments so that every answer here is reachable from a test.
///
/// * `Ok(Some(lease))` — the client has a lease and the hook recorded it.
/// * `Ok(None)` — no lease yet. **Not a failure**: the client is asking, and
///   the supervisor runs `rivet-netd` again when the hook records the answer.
///
/// # Errors
///
/// [`DhcpUnavailable`]: the client is not on this card, or the recorded lease
/// does not parse. Either way nothing is configured.
pub fn current_lease(
    interface: &InterfaceName,
    read: &mut dyn FnMut(&str) -> Option<String>,
    mechanism_present: bool,
) -> Result<Option<Lease>, DhcpUnavailable> {
    let name = interface.to_string();
    if !mechanism_present {
        return Err(DhcpUnavailable::MechanismNotBuilt {
            interface: name,
            mechanism: MECHANISM,
        });
    }
    let Some(text) = read(&lease_path(&name)) else {
        return Ok(None);
    };
    Lease::parse_pairs_text(&text)
        .map(Some)
        .map_err(|why| DhcpUnavailable::Unreadable {
            interface: name,
            detail: why.to_string(),
        })
}

#[cfg(test)]
#[path = "dhcp_tests.rs"]
mod tests;
