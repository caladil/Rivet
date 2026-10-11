//! **A display arriving, leaving and changing** — `ADR-058` §5.
//!
//! Two halves, kept apart so the one that decides can be tested without a
//! kernel:
//!
//! * [`decide`] is the whole policy. It is handed what the connectors say now
//!   and what is being driven, and answers whether to keep it, bring one up,
//!   take it down or swap it. Nothing in it touches a device.
//! * [`Uevents`] is the kernel's half: a `NETLINK_KOBJECT_UEVENT` socket that
//!   wakes the compositor when the DRM subsystem announces a hot-plug. It
//!   carries no state; the connectors are re-read when it fires, because the
//!   kernel is the authority on what is plugged in (`AGENTS.md` §5) and an
//!   event is only ever a prompt to ask again.
//!
//! # Why the kernel's socket and not `udev`
//!
//! Smithay's `backend_udev` links `libudev`, a C library and a new crate in
//! this program's lockfile, to read the same messages this module reads with
//! one `socket` call. Nothing on a Rivet runs `udevd` either, so the events
//! that arrive are the kernel's own, on multicast group 1.

use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

/// A mode as far as hot-plug cares: what a client would be configured to.
///
/// Two displays offering the same size and rate are the same screen to every
/// client, so swapping one for the other changes nothing anybody can see.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeKey {
    pub width: u16,
    pub height: u16,
    /// Millihertz, as `Output` carries it.
    pub refresh: i32,
}

/// What one connector says when asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Probe {
    /// The kernel's connector id.
    pub connector: u32,
    /// The mode it would be driven at: the preferred one, or the first it
    /// offers. `None` when nothing is plugged in, or when something is and
    /// offers no mode at all — neither can be driven.
    pub mode: Option<ModeKey>,
}

/// What the compositor should do about the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// What is being driven is still there at the same mode, or nothing is
    /// driven and nothing can be.
    Keep,
    /// Nothing is driven, and the probe at this index can be.
    Up(usize),
    /// What was driven is gone and nothing else can be driven.
    Down,
    /// What was driven is gone or has changed mode, and the probe at this
    /// index replaces it.
    Swap(usize),
}

/// **The whole of the hot-plug policy**, given what is driven and what the
/// connectors say now.
///
/// A connector that is still plugged in at the mode it was driven at is kept,
/// whatever else changed. One whose mode changed is a different display on the
/// same port, and is swapped for itself at the new mode. Otherwise the first
/// connector that can be driven is taken: one screen at a time is driven, so
/// which of two plugged-in displays is chosen is the connectors' order.
#[must_use]
pub fn decide(driven: Option<Probe>, now: &[Probe]) -> Change {
    let usable = |p: &Probe| p.mode.is_some();
    match driven {
        Some(current) => {
            if now.contains(&current) {
                return Change::Keep;
            }
            let same_port = now
                .iter()
                .position(|p| p.connector == current.connector && usable(p));
            match same_port.or_else(|| now.iter().position(usable)) {
                Some(index) => Change::Swap(index),
                None => Change::Down,
            }
        }
        None => match now.iter().position(usable) {
            Some(index) => Change::Up(index),
            None => Change::Keep,
        },
    }
}

/// The kernel's announcements, on a socket the event loop can wait on.
pub struct Uevents(OwnedFd);

impl AsFd for Uevents {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

/// The kernel's own multicast group for uevents. Group 2 is `udevd`'s
/// rebroadcast, which nothing on a Rivet sends.
const KERNEL_GROUP: u32 = 1;

impl Uevents {
    /// Join the kernel's uevent group, non-blocking.
    ///
    /// # Errors
    ///
    /// The socket could not be made or bound, with the kernel's reason.
    pub fn open() -> Result<Self, String> {
        // SAFETY: `socket` takes no pointers; a negative answer is checked
        // before the descriptor is used.
        let fd = unsafe {
            libc::socket(
                libc::AF_NETLINK,
                libc::SOCK_DGRAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                libc::NETLINK_KOBJECT_UEVENT,
            )
        };
        if fd < 0 {
            return Err(format!(
                "the uevent socket could not be made: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: `fd` was just returned by `socket` and has no other owner.
        let owned = unsafe { OwnedFd::from_raw_fd(fd) };

        // SAFETY: an all-zero `sockaddr_nl` is valid — its private padding
        // must be zero, which is why it is built this way at all.
        let mut address: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        address.nl_family = libc::AF_NETLINK as libc::sa_family_t;
        address.nl_pid = 0;
        address.nl_groups = KERNEL_GROUP;
        // SAFETY: `address` lives on this frame for the call and the length
        // passed is its own size.
        let bound = unsafe {
            libc::bind(
                owned.as_raw_fd(),
                (&raw const address).cast::<libc::sockaddr>(),
                std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
            )
        };
        if bound < 0 {
            return Err(format!(
                "the uevent socket could not join the kernel's group: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self(owned))
    }

    /// Read every message waiting and say whether any of them is a display
    /// hot-plug — [`concerns_a_display`].
    ///
    /// **A lost message counts as one.** `ENOBUFS` means the kernel dropped
    /// messages this socket did not read in time, and one of them may have
    /// been the unplug; asking the connectors again costs one probe.
    pub fn drain(&self) -> bool {
        let mut buffer = [0u8; 8192];
        let mut asked = false;
        loop {
            // SAFETY: `buffer` is writable for its whole length for the
            // duration of the call.
            let read = unsafe {
                libc::recv(
                    self.0.as_raw_fd(),
                    buffer.as_mut_ptr().cast::<libc::c_void>(),
                    buffer.len(),
                    libc::MSG_DONTWAIT,
                )
            };
            if read < 0 {
                let error = std::io::Error::last_os_error();
                match error.raw_os_error() {
                    Some(libc::EAGAIN) => return asked,
                    Some(libc::EINTR) => continue,
                    Some(libc::ENOBUFS) => {
                        asked = true;
                        continue;
                    }
                    _ => {
                        println!("rivet-displayd: the uevent socket refused a read — {error}");
                        return asked;
                    }
                }
            }
            let length = usize::try_from(read).unwrap_or(0).min(buffer.len());
            if let Some(message) = buffer.get(..length) {
                asked |= concerns_a_display(message);
            }
        }
    }
}

/// Whether one kernel uevent is the DRM subsystem announcing a hot-plug.
///
/// The message is `ACTION@DEVPATH` and then `KEY=VALUE` fields, each ended by
/// a NUL. A hot-plug is `SUBSYSTEM=drm` with `HOTPLUG=1`, which the kernel
/// sends for a connector whose state changed — and for nothing else the
/// compositor needs to hear about. A message from `udevd`, which begins with
/// its own `libudev` header, is not the kernel's and is ignored.
#[must_use]
pub fn concerns_a_display(message: &[u8]) -> bool {
    if message.starts_with(b"libudev") {
        return false;
    }
    let mut fields = message.split(|b| *b == 0);
    let Some(header) = fields.next() else {
        return false;
    };
    if !header.contains(&b'@') {
        return false;
    }
    let (mut drm, mut hotplug) = (false, false);
    for field in fields {
        match field {
            b"SUBSYSTEM=drm" => drm = true,
            b"HOTPLUG=1" => hotplug = true,
            _ => {}
        }
    }
    drm && hotplug
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic, clippy::unwrap_used)]
    use super::*;

    const HD: ModeKey = ModeKey { width: 1920, height: 1080, refresh: 60_000 };
    const SMALL: ModeKey = ModeKey { width: 1280, height: 720, refresh: 60_000 };

    fn plugged(connector: u32, mode: ModeKey) -> Probe {
        Probe { connector, mode: Some(mode) }
    }

    fn empty(connector: u32) -> Probe {
        Probe { connector, mode: None }
    }

    /// **A boot with no screen, then a screen, then none, then a different
    /// one** — the sequence `.rulings/` R-236 asks for, walked end to end
    /// through the one function that decides it.
    #[test]
    fn no_screen_then_one_then_none_then_a_different_one() {
        // Two HDMI ports, nothing in either: nothing to drive, and waiting is
        // the answer rather than a failure.
        let boot = [empty(32), empty(42)];
        assert_eq!(decide(None, &boot), Change::Keep);

        // A display arrives on the second port.
        let arrived = [empty(32), plugged(42, HD)];
        assert_eq!(decide(None, &arrived), Change::Up(1));
        let driven = arrived[1];

        // A spurious event: nothing changed, nothing is rebuilt.
        assert_eq!(decide(Some(driven), &arrived), Change::Keep);

        // It leaves.
        assert_eq!(decide(Some(driven), &boot), Change::Down);

        // A different display arrives on the first port, smaller.
        let other = [plugged(32, SMALL), empty(42)];
        assert_eq!(decide(None, &other), Change::Up(0));
    }

    /// A different display on the SAME port, with no unplug seen in between —
    /// two events the kernel coalesced, or a switch that was quick — is a
    /// change of mode, and the port is swapped for itself.
    #[test]
    fn a_new_mode_on_the_same_port_is_a_swap() {
        let driven = plugged(42, HD);
        let now = [plugged(32, HD), plugged(42, SMALL)];
        assert_eq!(decide(Some(driven), &now), Change::Swap(1));
    }

    /// The driven port emptied while another was plugged in: the other is
    /// taken rather than leaving a screen dark that something could drive.
    #[test]
    fn a_driven_port_that_empties_hands_over_to_another() {
        let driven = plugged(42, HD);
        let now = [plugged(32, SMALL), empty(42)];
        assert_eq!(decide(Some(driven), &now), Change::Swap(0));
    }

    /// Plugged in and offering no mode — an EDID that would not read — is not
    /// a screen that can be driven, and must not be brought up.
    #[test]
    fn a_display_offering_no_mode_is_not_brought_up() {
        let now = [Probe { connector: 42, mode: None }];
        assert_eq!(decide(None, &now), Change::Keep);
        assert_eq!(decide(Some(plugged(42, HD)), &now), Change::Down);
    }

    /// A message shaped exactly as the kernel's `drm_sysfs_hotplug_event`
    /// sends it. Hand-composed from the kernel's field names, not captured
    /// from a board.
    #[test]
    fn a_drm_hotplug_is_recognised() {
        let message = b"change@/devices/platform/gpu/drm/card1\0ACTION=change\0\
                        DEVPATH=/devices/platform/gpu/drm/card1\0SUBSYSTEM=drm\0HOTPLUG=1\0\
                        DEVNAME=dri/card1\0DEVTYPE=drm_minor\0SEQNUM=2041\0MAJOR=226\0MINOR=1\0";
        assert!(concerns_a_display(message));
    }

    #[test]
    fn other_uevents_are_not_a_hotplug() {
        // A DRM change with no HOTPLUG — a lease, for instance.
        assert!(!concerns_a_display(
            b"change@/devices/platform/gpu/drm/card1\0ACTION=change\0SUBSYSTEM=drm\0"
        ));
        // A hot-plug of something that is not a display.
        assert!(!concerns_a_display(
            b"add@/devices/usb1/1-1\0ACTION=add\0SUBSYSTEM=usb\0HOTPLUG=1\0"
        ));
        // udevd's rebroadcast of the same event.
        assert!(!concerns_a_display(b"libudev\0\xfe\xed\xca\xfe SUBSYSTEM=drm\0HOTPLUG=1\0"));
        // Nothing at all, and a field that merely contains the words.
        assert!(!concerns_a_display(b""));
        assert!(!concerns_a_display(b"change@/x\0XSUBSYSTEM=drm\0HOTPLUG=10\0"));
    }
}
