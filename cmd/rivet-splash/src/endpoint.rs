//! **The third line: where to reach this machine.**
//!
//! The maintainer's requirement, 2026-08-30: the splash gets a third line of
//! text giving the IP address and port of the admin port, with an option to
//! suppress it (.rulings/ R-343).
//!
//! # Why the splash discovers the ADDRESS and is told the PORT
//!
//! Because of when it runs. Pid 1 starts this program before the supervisor,
//! and therefore long before `rivet-netd` has configured anything — on a DHCP
//! boot the address does not exist for another ten to forty seconds. **Nothing
//! that starts the splash can know the address**, so the splash finds it out.
//!
//! The port is the opposite: it is in the document, pid 1 has already read the
//! document, and a second reader would be a second authority for one fact
//! (`ADR-011`). So it arrives as an argument.
//!
//! # Which address, when a machine can have several
//!
//! The first IPv4 address on an interface that is **up, not loopback, and not
//! link-local**. Ordered by the kernel's own enumeration rather than by a
//! preference this file invents, because the alternative is a rule that quietly
//! prefers the wrong link on the first machine with two.
//!
//! Link-local (`169.254.0.0/16`) is excluded deliberately: it is what an
//! interface has when DHCP has FAILED, and printing it would tell an operator
//! to try an address that will not answer.

use std::net::Ipv4Addr;

/// A usable IPv4 address for this machine, if it has one yet.
///
/// Returns `None` before the network is configured, which is the ordinary state
/// at the moment the splash is first drawn.
#[must_use]
pub fn current_address() -> Option<Ipv4Addr> {
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: `getifaddrs` writes a pointer to a list it allocates, and the
    // list is freed below on every path out of this function.
    if unsafe { libc::getifaddrs(&raw mut list) } != 0 {
        return None;
    }
    let mut found = None;
    let mut node = list;
    while !node.is_null() {
        // SAFETY: `node` is non-null and came from `getifaddrs`, which
        // guarantees a well-formed list terminated by a null `ifa_next`.
        let entry = unsafe { &*node };
        if let Some(address) = usable_address(entry) {
            found = Some(address);
            break;
        }
        node = entry.ifa_next;
    }
    // SAFETY: `list` came from `getifaddrs` and is freed exactly once.
    unsafe { libc::freeifaddrs(list) };
    found
}

/// Whether one entry is an address worth printing, and what it is.
fn usable_address(entry: &libc::ifaddrs) -> Option<Ipv4Addr> {
    if entry.ifa_addr.is_null() {
        return None;
    }
    // SAFETY: checked non-null above; `sa_family` is the first field of every
    // `sockaddr` variant, so reading it through the base type is defined.
    let family = unsafe { (*entry.ifa_addr).sa_family };
    if i32::from(family) != libc::AF_INET {
        return None;
    }
    let flags = entry.ifa_flags;
    if flags & (libc::IFF_UP as u32) == 0 || flags & (libc::IFF_LOOPBACK as u32) != 0 {
        return None;
    }
    // SAFETY: the family is `AF_INET`, so the pointer is a `sockaddr_in`.
    let sin = unsafe { &*entry.ifa_addr.cast::<libc::sockaddr_in>() };
    // `s_addr` is network byte order; `from_bits` wants host order.
    let address = Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr));
    // **Link-local is what an interface has when DHCP FAILED.** Printing it
    // would send an operator to an address that will not answer.
    if address.is_link_local() || address.is_unspecified() {
        return None;
    }
    Some(address)
}

/// The line as it is drawn: `192.168.50.66:7777`.
#[must_use]
pub fn line(address: Ipv4Addr, port: u16) -> String {
    format!("{address}:{port}")
}

#[cfg(test)]
#[path = "endpoint_tests.rs"]
mod tests;
