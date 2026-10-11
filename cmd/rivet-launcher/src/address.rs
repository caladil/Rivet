//! **Where this machine answers**, for the person standing in front of it.
//!
//! The maintainer asked the header to read
//! `hostname.local (192.168.x.y:<controlplane>)`, and the hard half is the
//! part after the colon: **nothing on this machine could already say it.**
//!
//! # Why it is not read from the document
//!
//! `ADR-042` §8 took the operator's document away from the launcher and it
//! is not coming back — a confined application may not open it, and the
//! launcher asks the supervisor for everything it used to read there. But the
//! supervisor does not carry the control plane's address either:
//! `Supervisor::facts` is documented as *"no file is opened, no directory
//! walked and no service asked"*, because that socket is reachable by every
//! application and a question costing I/O would put I/O in the supervisor's
//! loop (`.rulings/` R-195 D5).
//!
//! # Why it is not the default port
//!
//! `rivet_config::control_plane::DEFAULT_PORT` is 7777 and a machine whose
//! document says otherwise would be shown an address **that does not work**.
//! A wrong address on a screen is worse than none: somebody types it, fails to
//! reach the machine, and concludes the machine is down.
//!
//! # So it is derived from the process that actually answers
//!
//! [`control_plane`] takes `rivet-mcpd`'s pid — which `rivet_core::facts`
//! already carries — walks its open descriptors for socket inodes, and finds
//! the listening TCP port with one of those inodes in `/proc/net/tcp`. That is
//! not a guess about configuration; it is the port the control plane is
//! bound to, read from the kernel, and it is right on a machine whose document
//! says 9000 as surely as on one that says nothing.
//!
//! **This costs the launcher nothing it did not already have.** It runs as
//! uid 0 with no Landlock ruleset (`crate::service::IMAGE_DEFINED` — an image
//! service has no manifest to compute one from), so `/proc` is readable. An
//! APPLICATION could not do this, which is the right asymmetry: the program
//! that draws the machine's own identity is part of the machine.

/// The machine's own IPv4 address on the interface carrying the default route.
///
/// `None` on a machine with no route, which is a real state — a board on a
/// dead switch — and is drawn as an absent address rather than as `0.0.0.0`.
#[must_use]
pub fn ipv4() -> Option<String> {
    let interface = default_route_interface()?;
    address_of(&interface)
}

/// The interface name in the `/proc/net/route` row whose destination is
/// `00000000` — the default route.
fn default_route_interface() -> Option<String> {
    let text = std::fs::read_to_string("/proc/net/route").ok()?;
    for line in text.lines().skip(1) {
        let mut fields = line.split_whitespace();
        let name = fields.next()?;
        let destination = fields.next()?;
        if destination == "00000000" {
            return Some(name.to_string());
        }
    }
    None
}

/// The IPv4 address bound to `interface`, via `SIOCGIFADDR`.
///
/// `getifaddrs` would do the same job and allocates a linked list the caller
/// must free; one ioctl on one interface name is less to get wrong, and the
/// interface is already known.
fn address_of(interface: &str) -> Option<String> {
    if interface.len() >= libc::IFNAMSIZ {
        return None;
    }

    // SAFETY: an AF_INET datagram socket, used only as an ioctl handle and
    // closed below. A failure returns -1 and is checked before use.
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return None;
    }

    // SAFETY: `ifreq` is a C struct of plain integers and bytes; all-zero is a
    // valid value for every field and is what the kernel expects for the parts
    // the caller does not set.
    let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
    for (slot, byte) in request
        .ifr_name
        .iter_mut()
        .zip(interface.as_bytes().iter())
    {
        // `c_char` is signed on aarch64, so the cast is not a no-op and is the
        // reason this is a loop rather than a `copy_from_slice`.
        *slot = *byte as libc::c_char;
    }

    // SAFETY: `fd` is an open AF_INET socket and `request` is a correctly
    // sized `ifreq` whose name field is NUL-terminated by the zeroing above.
    let got = unsafe { libc::ioctl(fd, libc::SIOCGIFADDR as _, &raw mut request) };
    // SAFETY: `fd` was opened above, has not been closed, and nothing else
    // holds it.
    unsafe { libc::close(fd) };
    if got < 0 {
        return None;
    }

    // SAFETY: `SIOCGIFADDR` succeeded, which means the kernel filled
    // `ifr_addr` as a `sockaddr_in` — that is what the ioctl's contract is.
    let sockaddr = unsafe { &*std::ptr::addr_of!(request.ifr_ifru).cast::<libc::sockaddr_in>() };
    if u32::from(sockaddr.sin_family) != u32::from(libc::AF_INET as u16) {
        return None;
    }
    let octets = u32::from_be(sockaddr.sin_addr.s_addr).to_be_bytes();
    Some(format!(
        "{}.{}.{}.{}",
        octets[0], octets[1], octets[2], octets[3]
    ))
}

/// The TCP port `pid` is listening on, or `None` if it is listening on none.
///
/// # Why the first one is the answer
///
/// `rivet-mcpd` binds exactly one listener — `ADR-035`'s control plane — so
/// there is nothing to choose between. A process with several would make this
/// arbitrary, which is why the argument is a pid and not a service name: the
/// caller states whose port it wants, and this answers for that process only.
#[must_use]
pub fn listening_port(pid: u32) -> Option<u16> {
    let mut inodes = Vec::new();
    let fds = std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?;
    for entry in fds.flatten() {
        let Ok(target) = std::fs::read_link(entry.path()) else {
            continue;
        };
        // A socket descriptor reads back as `socket:[12345]`, and the number
        // is the inode `/proc/net/tcp` files its row under.
        let text = target.to_string_lossy();
        if let Some(rest) = text.strip_prefix("socket:[") {
            if let Some(number) = rest.strip_suffix(']') {
                if let Ok(inode) = number.parse::<u64>() {
                    inodes.push(inode);
                }
            }
        }
    }
    if inodes.is_empty() {
        return None;
    }

    let table = std::fs::read_to_string("/proc/net/tcp").ok()?;
    for line in table.lines().skip(1) {
        let mut fields = line.split_whitespace();
        let _slot = fields.next()?;
        let local = fields.next()?;
        let _remote = fields.next()?;
        let state = fields.next()?;
        // `0A` is TCP_LISTEN. Anything else is a connection, including the one
        // this question arrived over.
        if state != "0A" {
            continue;
        }
        // uid is field 7 and inode field 9, counting from the slot column.
        let inode = fields.nth(5).and_then(|t| t.parse::<u64>().ok())?;
        if !inodes.contains(&inode) {
            continue;
        }
        let port = local.split(':').nth(1)?;
        if let Ok(port) = u16::from_str_radix(port, 16) {
            return Some(port);
        }
    }
    None
}

/// `192.168.50.66:7777`, or as much of it as this machine can state.
///
/// `mcpd` is the pid of `rivet-mcpd` from the fact set, when it is running.
#[must_use]
pub fn control_plane(mcpd: Option<u32>) -> Option<String> {
    let ip = ipv4()?;
    match mcpd.and_then(listening_port) {
        Some(port) => Some(format!("{ip}:{port}")),
        // **The address alone, rather than the address and a guess.** A
        // machine whose control plane is not running has no port to name, and
        // naming one would send somebody to a socket nothing is on.
        None => Some(ip),
    }
}
