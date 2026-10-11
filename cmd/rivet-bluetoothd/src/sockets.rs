//! **The three descriptors this daemon holds**, and the `unsafe` that reaches
//! them. `crate::privilege` holds the rest.
//!
//! `ADR-041` §4.2. Every block below carries a SAFETY note, and each one is a
//! syscall `std` does not cover: `AF_BLUETOOTH`, `poll`, `CLOCK_MONOTONIC`, and
//! turning a handed descriptor into a listener.
//!
//! | Descriptor | What it is | Where it comes from |
//! | :-- | :-- | :-- |
//! | management | the kernel's answer to *is there an adapter* | opened here, `HCI_CHANNEL_CONTROL` |
//! | user channel | **the adapter**, exclusively | opened here, `HCI_CHANNEL_USER` |
//! | the listener | where an application connects | **handed over** — `LISTEN_FDS` |
//!
//! The third one is the platform's first article and it is worth saying out
//! loud: this daemon does not bind its own socket. The supervisor opens it,
//! calls `listen`, and passes the descriptor (`ADR-015` §5.2, `ADR-017` §3.4),
//! so where it lives is Rivet's business — which is why nothing here spells a
//! path.

use std::os::fd::{FromRawFd as _, OwnedFd, RawFd};
use std::os::unix::net::UnixListener;

/// `AF_BLUETOOTH`, from `include/linux/socket.h`.
///
/// Spelled here rather than taken from `libc` because the `libc` crate's
/// definition of it is not present on every target this workspace type-checks
/// against, and a constant that is 31 on Linux and absent elsewhere is a build
/// failure whose message is about a missing item rather than about Bluetooth.
pub const AF_BLUETOOTH: libc::c_int = 31;

/// `BTPROTO_HCI`.
pub const BTPROTO_HCI: libc::c_int = 1;

/// `HCI_CHANNEL_USER` — **the adapter, taken away from the kernel's own
/// stack.**
///
/// This is the exclusivity R-144.1 item 2 is about: a second process binding it
/// for the same controller gets `EUSERS`, so the arbiter has to be one program
/// and it is this one.
pub const HCI_CHANNEL_USER: u16 = 1;

/// `HCI_CHANNEL_CONTROL` — the management interface. Binds with no controllers.
pub const HCI_CHANNEL_CONTROL: u16 = 3;

/// `HCI_DEV_NONE` — the management socket is not about one controller.
pub const HCI_DEV_NONE: u16 = 0xFFFF;

/// `struct sockaddr_hci`, from `include/net/bluetooth/hci_sock.h`.
///
/// Not in the `libc` crate. Three little-endian shorts and no padding, which is
/// why `repr(C)` with three `u16` fields is the whole of it — there is nothing
/// here that could be laid out a second way.
#[repr(C)]
#[derive(Clone, Copy)]
struct SockAddrHci {
    hci_family: libc::sa_family_t,
    hci_dev: u16,
    hci_channel: u16,
}

/// A socket this process owns.
pub struct Socket(OwnedFd);

impl Socket {
    /// The raw descriptor, for `poll`.
    #[must_use]
    pub fn fd(&self) -> RawFd {
        use std::os::fd::AsRawFd as _;
        self.0.as_raw_fd()
    }

    /// Write one packet. A short write is reported as such rather than retried:
    /// an HCI command is a datagram to the controller and half of one is not a
    /// smaller command.
    ///
    /// # Errors
    ///
    /// The `write` syscall's errno, or [`std::io::ErrorKind::WriteZero`] for a
    /// short write.
    pub fn send(&self, packet: &[u8]) -> std::io::Result<()> {
        // SAFETY: `self.0` is a socket this process opened and still owns, and
        // `packet` is a live slice whose length is passed with it. `write` on a
        // socket reads the buffer and does not retain it.
        let n = unsafe { libc::write(self.fd(), packet.as_ptr().cast(), packet.len()) };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // A negative `n` is handled above, so the cast cannot lose the sign.
        #[allow(clippy::cast_sign_loss)]
        if n as usize != packet.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "the kernel took only part of an HCI packet, and half a command is not a \
                 smaller command",
            ));
        }
        Ok(())
    }

    /// Read one packet into `buf`, returning how many bytes arrived.
    ///
    /// # Errors
    ///
    /// The `read` syscall's errno.
    pub fn recv(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        // SAFETY: `buf` is a live, exclusively borrowed slice and its length is
        // passed with it, so the kernel cannot write past it. `self.0` is owned.
        let n = unsafe { libc::read(self.fd(), buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // Checked above.
        #[allow(clippy::cast_sign_loss)]
        Ok(n as usize)
    }
}

fn bind_hci(dev: u16, channel: u16) -> std::io::Result<Socket> {
    // SAFETY: three integer arguments, no pointers. A negative return is the
    // error path below.
    let fd = unsafe {
        libc::socket(
            AF_BLUETOOTH,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            BTPROTO_HCI,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` is a fresh descriptor from `socket` above and is not owned
    // by anything else, so this is the one place it is adopted. From here the
    // `OwnedFd` closes it, including on the error path below.
    let owned = unsafe { OwnedFd::from_raw_fd(fd) };
    let addr = SockAddrHci {
        // `AF_BLUETOOTH` is 31 and `sa_family_t` is a `u16` on Linux.
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        hci_family: AF_BLUETOOTH as libc::sa_family_t,
        hci_dev: dev,
        hci_channel: channel,
    };
    // SAFETY: `addr` is a live, correctly sized `sockaddr_hci` on this stack
    // frame for the duration of the call, and its length is passed with it.
    // `bind` copies out of it and does not retain the pointer.
    let rc = unsafe {
        libc::bind(
            fd,
            std::ptr::from_ref(&addr).cast(),
            // `size_of` of a three-field struct fits a `socklen_t`.
            #[allow(clippy::cast_possible_truncation)]
            {
                std::mem::size_of::<SockAddrHci>() as libc::socklen_t
            },
        )
    };
    if rc < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(Socket(owned))
}

/// Open the management socket.
///
/// **This works with no controller at all**, which is the whole reason it is
/// the presence authority — see [`crate::mgmt`].
///
/// # Errors
///
/// `EAFNOSUPPORT` when the kernel has no Bluetooth subsystem, which on a Rivet
/// means a kernel built before `ADR-041`. Any other errno from `socket` or
/// `bind`.
pub fn management() -> std::io::Result<Socket> {
    bind_hci(HCI_DEV_NONE, HCI_CHANNEL_CONTROL)
}

/// Take controller `index` for this process alone.
///
/// # Errors
///
/// `EUSERS` when something already holds this controller's user channel, `EBUSY`
/// when the kernel's own stack has it up, `ENODEV` when the index is gone. Each
/// is a different sentence to an operator, which is why the errno is returned
/// rather than folded.
pub fn user_channel(index: u16) -> std::io::Result<Socket> {
    bind_hci(index, HCI_CHANNEL_USER)
}

/// The listening socket the supervisor handed over.
///
/// `ADR-017` §3.4: descriptors start at 3, `LISTEN_FDS` says how many, and
/// `LISTEN_FDNAMES` names them in the order the manifest declared. This looks
/// the endpoint up **by name** rather than taking the first, because a
/// definition that grows a second socket must not silently reassign this one.
///
/// # Errors
///
/// A message naming which of the three variables was missing or did not agree,
/// because *"no socket"* sends its reader nowhere (`AGENTS.md` §5).
pub fn handed_listener(endpoint: &str) -> Result<UnixListener, String> {
    let count: usize = std::env::var("LISTEN_FDS")
        .map_err(|_| {
            "LISTEN_FDS is not set, so no socket was handed to this process. A Rivet \
             service does not bind its own (ADR-015 5.2); the supervisor opens it from \
             the definition's `provides` and passes the descriptor"
                .to_string()
        })?
        .parse()
        .map_err(|_| "LISTEN_FDS is set and is not a count".to_string())?;
    let pid: u32 = std::env::var("LISTEN_PID")
        .map_err(|_| "LISTEN_FDS is set and LISTEN_PID is not".to_string())?
        .parse()
        .map_err(|_| "LISTEN_PID is set and is not a pid".to_string())?;
    if pid != std::process::id() {
        return Err(format!(
            "LISTEN_PID names process {pid} and this is {}. The descriptors belong to \
             something else and must not be adopted",
            std::process::id()
        ));
    }
    let names = std::env::var("LISTEN_FDNAMES").unwrap_or_default();
    let at = names
        .split(':')
        .position(|n| n == endpoint)
        .ok_or_else(|| {
            format!(
                "LISTEN_FDNAMES is `{names}` and does not name `{endpoint}`. The service \
                 definition's provides = [\"socket:{endpoint}\"] is what puts it there"
            )
        })?;
    if at >= count {
        return Err(format!(
            "LISTEN_FDNAMES names `{endpoint}` at position {at} and LISTEN_FDS is {count}"
        ));
    }
    // Descriptors start at 3. `at` is bounded by `count` above, and a count
    // that large would already have failed the parse.
    let fd = RawFd::try_from(3 + at).map_err(|_| "the descriptor number does not fit".to_string())?;
    // SAFETY: the supervisor opened this descriptor, called `listen` on it, and
    // passed it to this process — which is what `LISTEN_PID` matching our own
    // pid establishes. Nothing else in this process has adopted it: this
    // function is called once, from `main`, before any other descriptor work.
    Ok(unsafe { UnixListener::from_raw_fd(fd) })
}

/// Milliseconds on `CLOCK_MONOTONIC`.
///
/// The stamp on every advertisement — see `rivet_core::bluetooth::Advertisement`
/// for why it is not a wall clock. Returns 0 if the clock cannot be read, which
/// cannot happen for `CLOCK_MONOTONIC` on Linux and is handled rather than
/// unwrapped because this crate denies `unwrap`.
#[must_use]
pub fn monotonic_ms() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a live, correctly typed `timespec` on this stack frame
    // for the duration of the call, and the kernel writes only into it.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &raw mut ts) };
    if rc != 0 {
        return 0;
    }
    // `CLOCK_MONOTONIC` counts from boot, so neither field is negative and
    // neither product can overflow a u64 in any uptime a board will see.
    #[allow(clippy::cast_sign_loss)]
    let (secs, nanos) = (ts.tv_sec as u64, ts.tv_nsec as u64);
    secs.saturating_mul(1000)
        .saturating_add(nanos / 1_000_000)
}

/// Which descriptors are readable.
///
/// `fds` is polled for `POLLIN`; the returned vector has one flag per input, in
/// the same order. A timeout of `None` blocks.
///
/// # Errors
///
/// The `poll` syscall's errno. `EINTR` is returned rather than retried — the
/// caller's loop is where a signal should be noticed, not here.
pub fn readable(fds: &[RawFd], timeout_ms: Option<i32>) -> std::io::Result<Vec<bool>> {
    let mut pollfds: Vec<libc::pollfd> = fds
        .iter()
        .map(|&fd| libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    // SAFETY: `pollfds` is a live, exclusively borrowed vector and its length
    // is passed with it, so the kernel writes only inside it. `nfds_t` is wide
    // enough for a handful of descriptors.
    let rc = unsafe {
        libc::poll(
            pollfds.as_mut_ptr(),
            pollfds.len() as libc::nfds_t,
            timeout_ms.unwrap_or(-1),
        )
    };
    if rc < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(pollfds
        .iter()
        // POLLHUP and POLLERR count as readable: the read that follows is what
        // turns them into an end-of-file or an errno, and a poll loop that
        // ignored them would spin.
        .map(|p| p.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0)
        .collect())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// `sockaddr_hci` is three shorts. A layout that grew padding would bind
    /// the wrong channel, and the kernel would answer `EINVAL` about a length
    /// rather than about a channel.
    #[test]
    fn the_hci_address_is_six_bytes_and_no_padding() {
        assert_eq!(std::mem::size_of::<SockAddrHci>(), 6);
    }

    #[test]
    fn the_monotonic_clock_moves_forward_and_is_not_a_wall_clock() {
        let a = monotonic_ms();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let b = monotonic_ms();
        assert!(b >= a, "{b} is before {a}");
        // A wall clock in milliseconds since the epoch is over 1.7e12. An
        // uptime is not, on any machine this will run on.
        assert!(a < 1_000_000_000_000, "this looks like a wall clock: {a}");
    }

    /// The three variables are the contract, and each way of getting them wrong
    /// has to name itself — this is what an operator reads when a service comes
    /// up with no socket.
    #[test]
    fn a_missing_or_disagreeing_handoff_says_which_part_was_wrong() {
        // The environment is process-wide, so this test sets and clears rather
        // than running in parallel with another that reads it. There is only
        // one such test in this crate.
        unsafe {
            std::env::remove_var("LISTEN_FDS");
            std::env::remove_var("LISTEN_PID");
            std::env::remove_var("LISTEN_FDNAMES");
        }
        let err = handed_listener("advertisements").unwrap_err();
        assert!(err.contains("LISTEN_FDS"), "{err}");

        unsafe { std::env::set_var("LISTEN_FDS", "1") };
        let err = handed_listener("advertisements").unwrap_err();
        assert!(err.contains("LISTEN_PID"), "{err}");

        unsafe { std::env::set_var("LISTEN_PID", "1") };
        let err = handed_listener("advertisements").unwrap_err();
        assert!(err.contains("names process 1"), "{err}");

        unsafe { std::env::set_var("LISTEN_PID", std::process::id().to_string()) };
        unsafe { std::env::set_var("LISTEN_FDNAMES", "something-else") };
        let err = handed_listener("advertisements").unwrap_err();
        assert!(err.contains("does not name"), "{err}");

        unsafe {
            std::env::remove_var("LISTEN_FDS");
            std::env::remove_var("LISTEN_PID");
            std::env::remove_var("LISTEN_FDNAMES");
        }
    }

    /// `poll` over a socketpair, so the wrapper is exercised rather than only
    /// compiled.
    #[test]
    fn poll_reports_a_readable_descriptor_and_not_an_idle_one() {
        use std::io::Write as _;
        use std::os::fd::AsRawFd as _;
        let (mut a, b) = std::os::unix::net::UnixStream::pair().expect("a socketpair");
        let flags = readable(&[b.as_raw_fd()], Some(0)).expect("poll");
        assert_eq!(flags, vec![false]);
        a.write_all(b"x").expect("a write");
        let flags = readable(&[b.as_raw_fd()], Some(50)).expect("poll");
        assert_eq!(flags, vec![true]);
    }
}
