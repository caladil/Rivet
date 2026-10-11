//! **The mDNS responder's socket, and the name the kernel answers to** —
//! `ADR-017` §3, `ADR-014` §2's mDNS row.
//!
//! `cmd/rivet-mdnsd` opens its one socket with [`open`] on its main thread,
//! before it reads anything else, so a socket it cannot have is a refusal
//! naming the step and the errno rather than a responder that reports itself
//! running and answers nothing. It takes the machine's name with
//! [`kernel_name`], `uname(2)`, which needs no path: the responder runs
//! confined and the kernel's hostname file is not in its ruleset.
//!
//! # One socket, for both directions
//!
//! Bound to `0.0.0.0:5353` and joined to `224.0.0.251` on the default
//! interface, it receives multicast queries and unicast ones alike, and
//! replies leave it from port 5353 as RFC 6762 §6 requires.
//!
//! # `SO_REUSEADDR`, and not `SO_REUSEPORT`
//!
//! Another program on the same machine may hold port 5353 too: the mDNS
//! client `rivetos-mdns` binds it with both options set when it resolves a
//! `.local` name. Linux lets two UDP sockets share a port when both set
//! `SO_REUSEADDR`, whichever uid owns each, so that is the option set here.
//! `SO_REUSEPORT` is not: between sockets of one uid it makes a group the
//! kernel load-balances unicast datagrams across, so a query addressed to
//! this responder could be handed to a socket that never reads it.
//! Between different uids it is refused, so it would buy nothing anyway.
//!
//! Nothing here is reachable from a test without a kernel; it is small enough
//! to read, and the responder's own decisions sit above it.

use std::net::{Ipv4Addr, UdpSocket};

/// The mDNS group — RFC 6762 §3.
pub const GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);

/// The mDNS port — RFC 6762 §3.
pub const PORT: u16 = 5353;

/// Which step of [`open`] failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// `socket(2)`.
    Socket,
    /// `setsockopt(SO_REUSEADDR)`.
    ReuseAddress,
    /// `bind(2)` to `0.0.0.0:5353`.
    Bind,
    /// `setsockopt(IP_ADD_MEMBERSHIP)` for [`GROUP`].
    Join,
}

impl core::fmt::Display for Step {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Socket => "creating a UDP socket",
            Self::ReuseAddress => "setting SO_REUSEADDR so the port can be shared",
            Self::Bind => "binding UDP 0.0.0.0:5353",
            Self::Join => "joining the multicast group 224.0.0.251",
        })
    }
}

/// [`open`] could not finish, at `step`, with what the kernel said.
#[derive(Debug)]
pub struct OpenFailure {
    pub step: Step,
    pub error: std::io::Error,
}

impl core::fmt::Display for OpenFailure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.error.raw_os_error() {
            Some(errno) => write!(f, "{} failed with errno {errno}: {}", self.step, self.error),
            None => write!(f, "{} failed: {}", self.step, self.error),
        }
    }
}

impl std::error::Error for OpenFailure {}

/// `ENODEV`, which `IP_ADD_MEMBERSHIP` on the default interface returns while
/// no interface has a route a multicast group can use.
#[cfg(target_os = "linux")]
const ENODEV: i32 = libc::ENODEV;
/// The same number on the one platform the responder runs on, so the
/// classification below is testable where `libc` is not linked.
#[cfg(not(target_os = "linux"))]
const ENODEV: i32 = 19;

impl OpenFailure {
    /// **Whether this failure is a moment rather than a state**: the join
    /// refused with `ENODEV` because no interface is up and routed yet. Early
    /// in a boot, `rivet-netd` has not finished — and for a DHCP interface the
    /// lease has not arrived — so the group has nowhere to be joined. That
    /// changes within seconds and without anyone acting; every other failure
    /// here is a fact about the machine that waiting does not change.
    #[must_use]
    pub fn is_waiting_for_an_interface(&self) -> bool {
        self.step == Step::Join && self.error.raw_os_error() == Some(ENODEV)
    }
}

/// Open the responder's socket: bound to `0.0.0.0:5353` with `SO_REUSEADDR`,
/// joined to [`GROUP`], close-on-exec.
///
/// # Errors
///
/// The step that failed and the kernel's errno.
#[cfg(target_os = "linux")]
pub fn open() -> Result<UdpSocket, OpenFailure> {
    use std::os::fd::{AsRawFd as _, FromRawFd as _};

    let fail = |step| OpenFailure {
        step,
        error: std::io::Error::last_os_error(),
    };

    // SAFETY: `socket(2)` takes three integers and returns a descriptor or -1.
    let fd = unsafe {
        libc::socket(
            libc::AF_INET,
            libc::SOCK_DGRAM | libc::SOCK_CLOEXEC,
            libc::IPPROTO_UDP,
        )
    };
    if fd < 0 {
        return Err(fail(Step::Socket));
    }
    // SAFETY: `fd` was returned by `socket(2)` above and nothing else holds
    // it, so the `UdpSocket` owns it and closes it on every path from here,
    // including the failures below.
    let socket = unsafe { UdpSocket::from_raw_fd(fd) };

    let one: libc::c_int = 1;
    // SAFETY: the option value is a live `c_int` and its length is passed
    // with it; `fd` is the open socket `socket` owns.
    let rc = unsafe {
        libc::setsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            (&raw const one).cast::<libc::c_void>(),
            core::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if rc != 0 {
        return Err(fail(Step::ReuseAddress));
    }

    // SAFETY: `sockaddr_in` is plain data and all-zero is a valid value of it.
    let mut address: libc::sockaddr_in = unsafe { core::mem::zeroed() };
    address.sin_family = libc::AF_INET as libc::sa_family_t;
    address.sin_port = PORT.to_be();
    address.sin_addr = libc::in_addr {
        s_addr: u32::from(Ipv4Addr::UNSPECIFIED).to_be(),
    };
    // SAFETY: the address is a live `sockaddr_in` and its length is passed
    // with it; `fd` is the open socket `socket` owns.
    let rc = unsafe {
        libc::bind(
            socket.as_raw_fd(),
            (&raw const address).cast::<libc::sockaddr>(),
            core::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    };
    if rc != 0 {
        return Err(fail(Step::Bind));
    }

    socket
        .join_multicast_v4(&GROUP, &Ipv4Addr::UNSPECIFIED)
        .map_err(|error| OpenFailure {
            step: Step::Join,
            error,
        })?;
    Ok(socket)
}

/// Not Linux: refused, never a socket that only looks open.
///
/// # Errors
///
/// Always.
#[cfg(not(target_os = "linux"))]
pub fn open() -> Result<UdpSocket, OpenFailure> {
    Err(OpenFailure {
        step: Step::Socket,
        error: std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "the mDNS responder's socket is opened only on Linux",
        ),
    })
}

/// The name the running kernel answers to: `uname(2)`'s `nodename`.
///
/// # Errors
///
/// The errno `uname(2)` returned, or a name that is not UTF-8.
#[cfg(target_os = "linux")]
pub fn kernel_name() -> std::io::Result<String> {
    // SAFETY: `utsname` is plain data and all-zero is a valid value of it.
    let mut uts: libc::utsname = unsafe { core::mem::zeroed() };
    // SAFETY: `uts` is a live, writable `utsname`, which is all `uname(2)`
    // writes to.
    if unsafe { libc::uname(&raw mut uts) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // `c_char` is `u8` on aarch64 and `i8` on x86_64; either way the cast is
    // the byte the kernel wrote.
    #[allow(clippy::cast_sign_loss, clippy::unnecessary_cast)]
    let bytes: Vec<u8> = uts
        .nodename
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8(bytes).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the kernel's nodename is not UTF-8",
        )
    })
}

/// Not Linux: refused.
///
/// # Errors
///
/// Always.
#[cfg(not(target_os = "linux"))]
pub fn kernel_name() -> std::io::Result<String> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "uname(2) is read only on Linux",
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// The refusal names the step and the errno, which is the whole of what an
    /// operator reading the journal needs to tell a busy port from a missing
    /// route.
    #[test]
    fn a_failure_names_its_step_and_its_errno() {
        let failure = OpenFailure {
            step: Step::Join,
            error: std::io::Error::from_raw_os_error(19),
        };
        let said = failure.to_string();
        assert!(said.contains("joining the multicast group 224.0.0.251"), "{said}");
        assert!(said.contains("errno 19"), "{said}");
    }

    /// Only a join refused for want of an interface is waited for. The errno
    /// is the one dev-01's journal carried when the responder started before
    /// its DHCP lease: `No such device (os error 19)`.
    #[test]
    fn only_a_join_with_no_interface_yet_is_a_moment() {
        let failure = |step, errno| OpenFailure {
            step,
            error: std::io::Error::from_raw_os_error(errno),
        };
        assert!(failure(Step::Join, 19).is_waiting_for_an_interface());
        assert!(!failure(Step::Bind, 19).is_waiting_for_an_interface());
        assert!(!failure(Step::Join, 1).is_waiting_for_an_interface());
        assert!(!failure(Step::Bind, 98).is_waiting_for_an_interface());
    }

    #[test]
    fn the_group_and_port_are_rfc_6762s() {
        assert_eq!(GROUP, Ipv4Addr::new(224, 0, 0, 251));
        assert_eq!(PORT, 5353);
    }

    /// On Linux the socket opens, bound to 5353 and joined, and a second one
    /// opens beside it: the port is shared, as the client's sockets need.
    #[cfg(target_os = "linux")]
    #[test]
    fn two_responders_sockets_share_the_port() {
        let first = match open() {
            Ok(socket) => socket,
            // A build container may have no multicast-capable interface. That
            // is the environment, and only the join may fail for it.
            Err(failure) if failure.step == Step::Join => return,
            Err(failure) => panic!("{failure}"),
        };
        assert_eq!(first.local_addr().expect("bound").port(), PORT);
        let second = open().expect("a second socket shares the port");
        assert_eq!(second.local_addr().expect("bound").port(), PORT);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_kernel_answers_with_a_name() {
        let name = kernel_name().expect("uname");
        assert!(!name.is_empty());
    }
}
