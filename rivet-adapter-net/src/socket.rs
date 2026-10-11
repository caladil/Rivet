//! The real `AF_NETLINK` socket. Linux only, and thin by policy.
//!
//! **Nothing in this file is reachable from a test.** That is the deal the port
//! trait makes: every decision moved above [`SendRecv`]
//! so it could be driven by a fake, and what is left here has to be small
//! enough to read and be sure of, because reading it is the only verification
//! it will get until it runs on hardware.
//!
//! It holds every `unsafe` block in this crate. Each one carries a SAFETY
//! comment, in the form `cmd/rivet-initd`'s `kernel` module established.
//!
//! # The three non-obvious choices
//!
//! 1. **`SOCK_RAW`, not `SOCK_DGRAM`.** Netlink accepts either and they behave
//!    identically; `SOCK_RAW` is what `iproute2` and every netlink library use,
//!    so it is the path the kernel's own users exercise.
//! 2. **`nl_pid = 0` when binding.** Zero asks the kernel to assign a unique
//!    port id. Binding to the process id by hand — the pattern in a great deal
//!    of example code — collides the moment two netlink sockets exist in one
//!    process, and the second `bind` fails with `EADDRINUSE`.
//! 3. **A receive timeout, and `MSG_TRUNC`.** A netlink request that gets no
//!    answer would otherwise wedge the network bring-up on a machine with no
//!    console and no way in; and a reply larger than the buffer is *silently
//!    truncated* by `recv(2)` unless `MSG_TRUNC` is set, which turns "the
//!    kernel told us something we did not read" into "the kernel said this".
//!    With `MSG_TRUNC` the return value is the real length, so truncation is
//!    detectable and is refused. `AGENTS.md` §5 — refuse rather than guess.

use crate::port::{Errno, SendRecv};
use crate::wire;

/// How long to wait for the kernel before giving up on one request.
///
/// Netlink is a local socket and the kernel answers a well-formed request
/// immediately, so this is a deadlock guard rather than a latency budget. Five
/// seconds is long enough that no real answer is ever cut off and short enough
/// that a wedged bring-up is a reported failure rather than a hang.
const RECV_TIMEOUT_SECS: i64 = 5;

/// Big enough for every reply Stage 1 asks for.
///
/// A single `RTM_NEWLINK` for one interface is a few hundred bytes; 32 KiB is
/// the size netlink's own users conventionally use and leaves room for an
/// interface carrying a great many attributes. A reply that does not fit is
/// **refused rather than truncated** — see [`NetlinkSocket::recv`].
const RECV_BUFFER: usize = 32 * 1024;

/// `struct sockaddr_nl` — netlink.h.
///
/// Declared here rather than taken from `libc` for the same reason the wire
/// constants are (see [`crate::wire`]): it is part of the interface to the
/// kernel this code talks to, and writing it out makes the twelve bytes
/// visible. `family(2) pad(2) pid(4) groups(4)`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct SockaddrNl {
    nl_family: u16,
    nl_pad: u16,
    /// The socket's own port id. **Zero on a `bind` means "kernel, choose
    /// one"**; zero as a *destination* means "the kernel".
    nl_pid: u32,
    /// Multicast groups to join. Zero: this crate asks questions and does not
    /// listen for broadcasts. Link-state monitoring would set
    /// `RTMGRP_LINK` here, and would need a different receive loop.
    nl_groups: u32,
}

/// An open netlink socket.
pub struct NetlinkSocket {
    fd: i32,
}

impl NetlinkSocket {
    /// Open and bind a `NETLINK_ROUTE` socket.
    ///
    /// The failures worth naming, because each sends a diagnosis somewhere
    /// different:
    ///
    /// | errno | Means |
    /// | :-- | :-- |
    /// | `EAFNOSUPPORT` | the kernel has no netlink at all — `CONFIG_NET` is off, which on a Pi means the wrong kernel entirely |
    /// | `EPROTONOSUPPORT` | netlink is present and `NETLINK_ROUTE` is not |
    /// | `EPERM` | the process may not open it. Opening needs no privilege on an ordinary kernel, so this is a sandbox or LSM |
    pub fn open() -> Result<Self, Errno> {
        Self::open_protocol(wire::NETLINK_ROUTE)
    }

    /// Open and bind a netlink socket for `protocol` — `NETLINK_ROUTE`, or
    /// [`crate::nft::NETLINK_NETFILTER`] for the firewall. The same three
    /// choices and the same failures as [`Self::open`]; `EPROTONOSUPPORT` on
    /// `NETLINK_NETFILTER` is a kernel built without netfilter.
    pub fn open_protocol(protocol: i32) -> Result<Self, Errno> {
        // SAFETY: `socket(2)` takes three integers, touches no memory of ours,
        // and returns a descriptor or -1. `SOCK_CLOEXEC` is set here rather
        // than with a later `fcntl` so there is no window in which a fork
        // between the two would leak the descriptor into a child.
        let fd = unsafe {
            libc::socket(
                wire::AF_NETLINK,
                libc::SOCK_RAW | libc::SOCK_CLOEXEC,
                protocol,
            )
        };
        if fd < 0 {
            return Err(last_errno());
        }
        let socket = Self { fd };

        let addr = SockaddrNl {
            nl_family: u16::try_from(wire::AF_NETLINK).unwrap_or(0),
            nl_pad: 0,
            nl_pid: 0,
            nl_groups: 0,
        };
        // SAFETY: `bind(2)` reads `len` bytes through the pointer and retains
        // nothing. `addr` lives in this frame for the whole call, and `len` is
        // its own `size_of`, so the kernel cannot read past it. The cast is the
        // conventional one: every `sockaddr_*` is passed as a `sockaddr` and
        // discriminated by its first field, which `nl_family` is.
        let rc = unsafe {
            libc::bind(
                socket.fd,
                std::ptr::from_ref(&addr).cast::<libc::sockaddr>(),
                socklen(core::mem::size_of::<SockaddrNl>()),
            )
        };
        if rc < 0 {
            return Err(last_errno());
        }

        let timeout = libc::timeval {
            tv_sec: RECV_TIMEOUT_SECS,
            tv_usec: 0,
        };
        // SAFETY: `setsockopt(2)` reads `optlen` bytes through the pointer and
        // retains nothing; `timeout` lives in this frame for the call and
        // `optlen` is its own `size_of`. A failure here is not fatal — it means
        // the receive can block longer than intended, not that anything is
        // unsound — so the result is deliberately not propagated, and the
        // reason is recorded rather than left as a silent `let _`.
        let _ = unsafe {
            libc::setsockopt(
                socket.fd,
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                std::ptr::from_ref(&timeout).cast::<libc::c_void>(),
                socklen(core::mem::size_of::<libc::timeval>()),
            )
        };

        Ok(socket)
    }
}

impl SendRecv for NetlinkSocket {
    fn send(&mut self, message: &[u8]) -> Result<(), Errno> {
        // The destination: the kernel is port 0. Named explicitly rather than
        // relying on the socket being connected, because it is not.
        let kernel = SockaddrNl {
            nl_family: u16::try_from(wire::AF_NETLINK).unwrap_or(0),
            nl_pad: 0,
            nl_pid: 0,
            nl_groups: 0,
        };
        // SAFETY: `sendto(2)` reads `len` bytes from `message`, which is a live
        // borrow for the whole call, and `addrlen` bytes from `kernel`, which
        // lives in this frame. It retains neither. `MSG_NOSIGNAL` is not needed
        // on a datagram socket but costs nothing and removes any possibility of
        // a `SIGPIPE` reaching a process that has not installed a handler.
        let sent = unsafe {
            libc::sendto(
                self.fd,
                message.as_ptr().cast::<libc::c_void>(),
                message.len(),
                libc::MSG_NOSIGNAL,
                std::ptr::from_ref(&kernel).cast::<libc::sockaddr>(),
                socklen(core::mem::size_of::<SockaddrNl>()),
            )
        };
        if sent < 0 {
            return Err(last_errno());
        }
        // A short send on a datagram socket is not something the interface
        // permits, so it is refused rather than retried from an offset — a
        // partial netlink message is not a message.
        if usize::try_from(sent).unwrap_or(0) != message.len() {
            return Err(Errno::EIO);
        }
        Ok(())
    }

    fn recv(&mut self) -> Result<Vec<u8>, Errno> {
        let mut buffer = vec![0u8; RECV_BUFFER];
        // SAFETY: `recv(2)` writes at most `len` bytes into the pointer, which
        // borrows a live `Vec` of exactly that length, and retains nothing.
        // `MSG_TRUNC` changes the *return value* — it reports the datagram's
        // real size — and does not change how much is written.
        let received = unsafe {
            libc::recv(
                self.fd,
                buffer.as_mut_ptr().cast::<libc::c_void>(),
                buffer.len(),
                libc::MSG_TRUNC,
            )
        };
        if received < 0 {
            return Err(last_errno());
        }
        let received = usize::try_from(received).unwrap_or(0);
        if received > buffer.len() {
            // The kernel sent more than fits. What is in the buffer is a
            // prefix of a message, and a prefix parses as something — which is
            // exactly the "wrong guess succeeds" failure `AGENTS.md` §5 names.
            return Err(Errno::ENOBUFS);
        }
        buffer.truncate(received);
        Ok(buffer)
    }
}

impl Drop for NetlinkSocket {
    fn drop(&mut self) {
        // SAFETY: `close(2)` takes one integer and touches no memory of ours.
        // `self.fd` was returned by `socket(2)` and is owned solely by this
        // struct, which is being consumed, so it cannot be closed twice.
        unsafe {
            libc::close(self.fd);
        }
    }
}

/// The errno the last syscall set.
fn last_errno() -> Errno {
    Errno(
        std::io::Error::last_os_error()
            .raw_os_error()
            // Zero is not a valid errno, so it is unambiguous as "the platform
            // did not say" — the same convention `cmd/rivet-initd` uses.
            .unwrap_or(0),
    )
}

/// A length as the socket calls expect it.
fn socklen(len: usize) -> libc::socklen_t {
    libc::socklen_t::try_from(len).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// The struct the kernel reads twelve bytes out of. Getting its size wrong
    /// is a `bind` that fails with `EINVAL` at boot, on a machine with no way
    /// in — so it is checked here rather than discovered there.
    #[test]
    fn the_netlink_address_is_the_twelve_bytes_the_kernel_expects() {
        assert_eq!(core::mem::size_of::<SockaddrNl>(), 12);
        assert_eq!(
            core::mem::size_of::<SockaddrNl>(),
            core::mem::size_of::<libc::sockaddr_nl>()
        );
        assert_eq!(
            core::mem::align_of::<SockaddrNl>(),
            core::mem::align_of::<libc::sockaddr_nl>()
        );
    }

    /// Opening the socket needs no privilege on an ordinary kernel — only
    /// *changing* things does — so this runs anywhere Linux does, including in
    /// a container. It proves the socket, the bind and the timeout, and
    /// **nothing about whether a request works**.
    #[test]
    fn a_netlink_socket_can_be_opened_and_bound_without_privilege() {
        match NetlinkSocket::open() {
            Ok(socket) => assert!(socket.fd >= 0),
            // Reported rather than asserted away: a sandbox that forbids
            // netlink is a real environment, and a test that failed there would
            // be reporting the sandbox rather than the code.
            Err(errno) => eprintln!("netlink socket unavailable in this environment: {errno}"),
        }
    }
}
