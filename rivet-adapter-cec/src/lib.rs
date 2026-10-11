//! **Telling the television** — HDMI-CEC over `/dev/cecN`.
//!
//! `ADR-039` §8.2. The machine owns when its screens are lit, and CEC is the
//! transport that reaches past the panel to the thing with the power supply.
//!
//! # Why this is the machine's and not an application's
//!
//! A kiosk application is leased **the CRTC and its connector**, and a lease is
//! exclusive — so while an application is drawing, the supervisor cannot even
//! blank the panel (`ADR-039` §8.1). CEC is the way out of that corner: it is a
//! *different device*, `/dev/cecN`, not in the lease and not affected by who
//! holds the display. **That independence is the reason to prefer it**, quite
//! apart from it being the only thing that actually saves power — a blanked
//! panel commonly keeps its backlight lit, and a television in standby is what
//! the electricity meter notices.
//!
//! # What this crate will not do
//!
//! **It does not decide when.** `ADR-039` §3.1's `[display]` table is the
//! policy and this is the transport; a crate that read a schedule would be a
//! second authority for the hours (`ADR-011`).
//!
//! **It does not listen.** CEC is a bus and a full implementation answers
//! polls, reports its power status and follows the active source. This
//! transmits and nothing else, because transmitting is what the schedule needs
//! and a follower is a service rather than a function. `CEC_MODE_INITIATOR`
//! says exactly that to the kernel: *I will send; do not route traffic to me.*
//!
//! # It is `libc` and the uapi, and that is the whole dependency list
//!
//! Aura sends CEC by shelling out to `cec-client`. That cannot port — no shell,
//! no binary, and `ADR-001` would not admit `libcec` — and **none of it is
//! needed**: the kernel's API is six ioctls and *Standby* is one byte.
//! [`uapi`] carries the numbers, printed from the board's own header rather
//! than derived.
//!
//! # NOT PROVEN ON HARDWARE
//!
//! Every ioctl below is what the header says and what the documentation
//! describes. **No television has been switched off by this code.** The struct
//! layouts are asserted against the kernel header, the frame is asserted byte
//! by byte, and none of that is evidence that a TV obeys — which needs a person
//! in front of one. `ADR-039` §12 carries it as unproven.

pub mod uapi;

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};

/// Where the adapters are. A Pi 4 has two — one per HDMI port.
///
/// **Both are listed and neither is assumed.** `/dev/cec0` is HDMI0, which is
/// the port a Rivet's display lease drives; `/dev/cec1` is HDMI1. A machine
/// whose HDMI0 is unplugged and whose television is on HDMI1 is a machine
/// this would silently do nothing on if only the first PRESENT node were
/// tried, so the caller is given both and [`Adapter::open_first_usable`]
/// opens and configures each in turn and says which it took.
pub const DEVICES: [&str; 2] = ["/dev/cec0", "/dev/cec1"];

/// What this machine calls itself in a television's input list.
///
/// Fifteen bytes is the protocol's limit, not a choice.
pub const OSD_NAME: &str = "Rivet";

/// What went wrong, in a sentence a journal can carry.
#[derive(Debug)]
pub enum Error {
    /// The device node is not there — no CEC in the kernel, or no adapter.
    Absent { path: String },
    /// It is there and could not be opened. Usually permissions.
    Open { path: String, errno: io::Error },
    /// An ioctl refused. Names which one, because `ENOTTY` from `TRANSMIT` and
    /// `ENOTTY` from `S_LOG_ADDRS` are different diagnoses.
    Ioctl { call: &'static str, errno: io::Error },
    /// The adapter cannot do what this crate needs.
    Incapable { what: &'static str, capabilities: u32 },
    /// No physical address, which means **no cable or no EDID**: the adapter
    /// cannot know where it sits in the HDMI tree, so it cannot address
    /// anything.
    NoPhysicalAddress,
    /// Configured, and the kernel claimed no logical address for us.
    NoLogicalAddress,
    /// The message went out and was not acknowledged.
    NotAcknowledged { tx_status: u8 },
    /// Every candidate in [`DEVICES`] was tried by
    /// [`Adapter::open_first_usable`] and none both opened and configured.
    /// Each candidate's own failure is kept, in order, because "no usable
    /// adapter" alone would hide which port had no cable and which had no
    /// device node at all.
    NoUsableAdapter { attempts: Vec<(String, Error)> },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Absent { path } => write!(
                f,
                "{path} is not there, so this machine has no CEC adapter on that port. \
                 CONFIG_CEC_CORE and CONFIG_DRM_VC4_HDMI_CEC are what put it there"
            ),
            Self::Open { path, errno } => write!(f, "{path} could not be opened: {errno}"),
            Self::Ioctl { call, errno } => write!(f, "{call} was refused: {errno}"),
            Self::Incapable { what, capabilities } => write!(
                f,
                "this adapter cannot {what} (capabilities 0x{capabilities:08x}), so nothing \
                 here can drive it"
            ),
            Self::NoPhysicalAddress => write!(
                f,
                "the adapter has no physical address, which means no HDMI cable or no EDID to \
                 read one from. It does not know where it sits in the tree and cannot address \
                 the television"
            ),
            Self::NoLogicalAddress => write!(
                f,
                "the adapter claimed no logical address, so there is nobody for a message to be \
                 FROM. Another process may hold the bus exclusively"
            ),
            Self::NotAcknowledged { tx_status } => write!(
                f,
                "the message was sent and the television did not acknowledge it \
                 (tx_status 0x{tx_status:02x}). It is off at the wall, on another input, or has \
                 CEC disabled — which many sets ship with, under a vendor name like Anynet+ or \
                 Bravia Sync"
            ),
            Self::NoUsableAdapter { attempts } => {
                write!(f, "no usable CEC adapter: ")?;
                for (i, (path, why)) in attempts.iter().enumerate() {
                    if i > 0 {
                        write!(f, "; ")?;
                    }
                    write!(f, "{path}: {why}")?;
                }
                Ok(())
            }
        }
    }
}

/// Try each candidate in order and return the first that both opens and
/// configures. Every candidate that was tried and failed is kept, in order,
/// so the caller can say why **each** port did not work rather than only the
/// last one tried.
///
/// Generic over the open and configure steps, rather than calling
/// [`Adapter::open`] and [`Adapter::configure`] directly, so this selection
/// logic can be exercised by the `tests` module without a real `/dev/cecN`
/// node to open.
fn select_first_usable<A>(
    candidates: impl IntoIterator<Item = &'static str>,
    mut open: impl FnMut(&'static str) -> Result<A, Error>,
    mut configure: impl FnMut(&mut A) -> Result<(), Error>,
) -> Result<A, Error> {
    let mut attempts = Vec::new();
    for path in candidates {
        let mut adapter = match open(path) {
            Ok(adapter) => adapter,
            Err(why) => {
                attempts.push((path.to_string(), why));
                continue;
            }
        };
        match configure(&mut adapter) {
            Ok(()) => return Ok(adapter),
            Err(why) => attempts.push((path.to_string(), why)),
        }
    }
    Err(Error::NoUsableAdapter { attempts })
}

/// One CEC adapter, open.
pub struct Adapter {
    fd: OwnedFd,
    path: String,
    /// The address the kernel claimed for us, once configured.
    logical: Option<u8>,
}

impl Adapter {
    /// Open a specific adapter.
    ///
    /// # Errors
    ///
    /// [`Error::Absent`] when the node is not there, which is an ordinary state
    /// of a machine rather than a fault, and [`Error::Open`] otherwise.
    pub fn open(path: &str) -> Result<Self, Error> {
        if !std::path::Path::new(path).exists() {
            return Err(Error::Absent { path: path.to_string() });
        }
        let c_path = CString::new(path).map_err(|_| Error::Absent { path: path.to_string() })?;
        // SAFETY: `c_path` is a NUL-terminated string that outlives the call,
        // and `open(2)` either returns a descriptor this takes ownership of or
        // -1. O_RDWR because an ioctl that writes needs a writable descriptor;
        // O_CLOEXEC so a child this process spawns does not inherit the bus.
        #[allow(unsafe_code)]
        let raw = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        if raw < 0 {
            return Err(Error::Open {
                path: path.to_string(),
                errno: io::Error::last_os_error(),
            });
        }
        // SAFETY: `raw` is a fresh descriptor from `open` that nothing else
        // holds, so `OwnedFd` may take sole ownership of closing it.
        #[allow(unsafe_code)]
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        Ok(Self {
            fd,
            path: path.to_string(),
            logical: None,
        })
    }

    /// Open and configure the first adapter in [`DEVICES`] that is USABLE,
    /// and say which.
    ///
    /// # Why "opens" is not enough
    ///
    /// `/dev/cec0` opens successfully whenever the kernel node exists, which
    /// says nothing about whether a television is reachable through it. A
    /// machine with HDMI0 unplugged and a television on HDMI1 opens `cec0`
    /// cleanly and then fails to [`Adapter::configure`] it with
    /// [`Error::NoPhysicalAddress`] — no cable, no EDID to read one from.
    /// Stopping there, as "first that opens" once did, leaves `cec1` — the
    /// port actually wired to a screen — untried. This opens AND configures
    /// every candidate in [`DEVICES`] in order and returns the first one
    /// that configures, so the port that is actually connected is the one
    /// used regardless of which `/dev/cecN` number it happens to be.
    ///
    /// # Errors
    ///
    /// [`Error::NoUsableAdapter`], naming every candidate's own failure, when
    /// none of [`DEVICES`] both opened and configured.
    pub fn open_first_usable() -> Result<Self, Error> {
        select_first_usable(DEVICES, Self::open, |adapter: &mut Self| {
            adapter.configure().map(|_logical| ())
        })
    }

    /// Which node this is.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The address the kernel claimed, once [`Adapter::configure`] has run.
    #[must_use]
    pub const fn logical_address(&self) -> Option<u8> {
        self.logical
    }

    /// One ioctl, with the call named in any refusal.
    fn ioctl<T>(&self, request: u32, call: &'static str, arg: &mut T) -> Result<(), Error> {
        // SAFETY: `arg` is a live, correctly-sized value of the type the
        // request encodes — `uapi`'s tests assert that the size baked into
        // every request number matches the struct passed with it, which is the
        // property that makes this call sound. The kernel writes at most
        // `size_of::<T>()` bytes through the pointer.
        //
        // `as _` on the request: `libc::ioctl` takes `c_ulong` on glibc and
        // `c_int` on musl, and this crate builds for both.
        #[allow(unsafe_code)]
        let rc = unsafe {
            libc::ioctl(
                self.fd.as_raw_fd(),
                request as _,
                std::ptr::from_mut::<T>(arg).cast::<libc::c_void>(),
            )
        };
        if rc < 0 {
            return Err(Error::Ioctl {
                call,
                errno: io::Error::last_os_error(),
            });
        }
        Ok(())
    }

    /// What this adapter says it can do.
    ///
    /// # Errors
    ///
    /// [`Error::Ioctl`].
    pub fn capabilities(&self) -> Result<uapi::Caps, Error> {
        let mut caps = uapi::Caps::zeroed();
        self.ioctl(uapi::ADAP_G_CAPS, "CEC_ADAP_G_CAPS", &mut caps)?;
        Ok(caps)
    }

    /// Where this adapter sits in the HDMI tree, from the display's EDID.
    ///
    /// `None` is [`uapi::PHYS_ADDR_INVALID`] — no cable, or a display with no
    /// EDID to read one from.
    ///
    /// # Errors
    ///
    /// [`Error::Ioctl`].
    pub fn physical_address(&self) -> Result<Option<u16>, Error> {
        let mut addr: u16 = 0;
        self.ioctl(uapi::ADAP_G_PHYS_ADDR, "CEC_ADAP_G_PHYS_ADDR", &mut addr)?;
        Ok((addr != uapi::PHYS_ADDR_INVALID).then_some(addr))
    }

    /// **Claim a place on the bus as a playback device**, so that messages have
    /// somebody to be from.
    ///
    /// This is the step a program that only wanted to send Standby would be
    /// most tempted to skip, and skipping it does not fail cleanly: an
    /// unconfigured adapter answers `CEC_TRANSMIT` with `ENONET`, which reads
    /// as a network error on a device that has no network.
    ///
    /// # The order matters and is the kernel's, not a preference
    ///
    /// 1. `CEC_S_MODE` to `INITIATOR` — declare that this process sends. Without
    ///    it a transmit is refused with `EPERM`.
    /// 2. `CEC_ADAP_S_LOG_ADDRS` with `num_log_addrs = 0` — clear whatever was
    ///    there. The kernel refuses a configure over a live configuration.
    /// 3. `CEC_ADAP_S_LOG_ADDRS` with the playback claim.
    /// 4. `CEC_ADAP_G_LOG_ADDRS` — read back **which** address was granted,
    ///    because it depends on what else is on the bus.
    ///
    /// # Errors
    ///
    /// [`Error::Incapable`] when the adapter cannot be configured by userspace
    /// at all, [`Error::NoPhysicalAddress`] when there is no cable, and
    /// [`Error::NoLogicalAddress`] when the claim was accepted and granted
    /// nothing.
    pub fn configure(&mut self) -> Result<u8, Error> {
        let caps = self.capabilities()?;
        if caps.capabilities & uapi::CAP_TRANSMIT == 0 {
            return Err(Error::Incapable {
                what: "transmit",
                capabilities: caps.capabilities,
            });
        }
        if caps.capabilities & uapi::CAP_LOG_ADDRS == 0 {
            return Err(Error::Incapable {
                what: "be given a logical address by userspace",
                capabilities: caps.capabilities,
            });
        }
        // Checked BEFORE configuring, because a claim on an adapter with no
        // physical address is accepted and grants nothing — a failure two steps
        // later with a worse message.
        if self.physical_address()?.is_none() {
            return Err(Error::NoPhysicalAddress);
        }

        let mut mode: u32 = uapi::MODE_INITIATOR;
        self.ioctl(uapi::S_MODE, "CEC_S_MODE", &mut mode)?;

        let mut clear = uapi::LogAddrs::zeroed();
        self.ioctl(uapi::ADAP_S_LOG_ADDRS, "CEC_ADAP_S_LOG_ADDRS (clear)", &mut clear)?;

        let mut claim = uapi::LogAddrs::playback(OSD_NAME);
        self.ioctl(uapi::ADAP_S_LOG_ADDRS, "CEC_ADAP_S_LOG_ADDRS", &mut claim)?;

        let mut granted = uapi::LogAddrs::zeroed();
        self.ioctl(uapi::ADAP_G_LOG_ADDRS, "CEC_ADAP_G_LOG_ADDRS", &mut granted)?;
        let logical = granted.claimed().ok_or(Error::NoLogicalAddress)?;
        self.logical = Some(logical);
        Ok(logical)
    }

    /// Send one command to the television.
    ///
    /// # Errors
    ///
    /// [`Error::NoLogicalAddress`] when [`Adapter::configure`] has not run,
    /// [`Error::Ioctl`] when the call itself is refused, and
    /// [`Error::NotAcknowledged`] when it went out and nothing answered.
    pub fn send(&self, opcode: u8) -> Result<(), Error> {
        let initiator = self.logical.ok_or(Error::NoLogicalAddress)?;
        let mut msg = uapi::Msg::command(initiator, uapi::LOG_ADDR_TV, opcode);
        self.ioctl(uapi::TRANSMIT, "CEC_TRANSMIT", &mut msg)?;
        // **The ioctl succeeding is not the message arriving.** It means the
        // adapter accepted it for transmission; `tx_status` is what the bus
        // said, and a television that is off at the wall NACKs.
        if msg.tx_status & uapi::TX_STATUS_OK == 0 {
            return Err(Error::NotAcknowledged {
                tx_status: msg.tx_status,
            });
        }
        Ok(())
    }

    /// **Tell the television to sleep.** `CEC_MSG_STANDBY`.
    ///
    /// # Errors
    ///
    /// As [`Adapter::send`].
    pub fn standby(&self) -> Result<(), Error> {
        self.send(uapi::MSG_STANDBY)
    }

    /// **Wake it, and ask for this input.** `CEC_MSG_IMAGE_VIEW_ON`.
    ///
    /// # Errors
    ///
    /// As [`Adapter::send`].
    pub fn image_view_on(&self) -> Result<(), Error> {
        self.send(uapi::MSG_IMAGE_VIEW_ON)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;
    use std::cell::RefCell;

    /// **Opening a node that is not there is an ordinary state**, named, and
    /// not a panic — which is what a machine with no CEC must get.
    #[test]
    fn an_absent_adapter_is_reported_by_name_rather_than_panicking() {
        // Destructured rather than `expect_err`, which would need
        // `Adapter: Debug` - and a Debug for an open descriptor is a thing to
        // print by accident.
        let Err(why) = Adapter::open("/dev/cec-nonexistent-for-this-test") else {
            panic!("a path that does not exist should not open");
        };
        assert!(matches!(why, Error::Absent { .. }));
        // The sentence has to name the kernel options, because *why is there no
        // /dev/cec0* is the question the reader is about to ask.
        let said = why.to_string();
        assert!(said.contains("CONFIG_CEC_CORE"), "{said}");
    }

    /// Every failure says which call refused and what it means. A CEC failure
    /// is diagnosed by somebody who is not in the room with the television.
    #[test]
    fn every_error_explains_itself_without_naming_an_errno_alone() {
        let cases = [
            Error::Incapable { what: "transmit", capabilities: 0 },
            Error::NoPhysicalAddress,
            Error::NoLogicalAddress,
            Error::NotAcknowledged { tx_status: 0x04 },
        ];
        for why in cases {
            let said = why.to_string();
            assert!(said.len() > 40, "too terse to act on: {said}");
            assert!(
                said.chars().filter(char::is_ascii_alphabetic).count() > 20,
                "not a sentence: {said}"
            );
        }
        // The unacknowledged case must name the thing a person can actually
        // check, because a NACK is almost always CEC switched off in a menu.
        let nack = Error::NotAcknowledged { tx_status: 0x04 }.to_string();
        assert!(nack.contains("Anynet+") || nack.contains("CEC disabled"), "{nack}");
    }

    /// **`send` before `configure` is refused rather than sent from nobody.**
    /// An unconfigured adapter has no logical address, and a frame whose
    /// initiator nibble was invented would be this machine impersonating
    /// whoever holds that address.
    #[test]
    fn sending_before_configuring_is_refused() {
        // Built without opening anything: the check under test is on the
        // logical address, which `open` never sets.
        let Ok(fd) = std::fs::File::open("/dev/null").map(OwnedFd::from) else {
            // QEMU has no /dev/null (`PLAN.md`), and this test is about a
            // struct field rather than a device — say so rather than failing.
            return;
        };
        let adapter = Adapter {
            fd,
            path: "/dev/null".to_string(),
            logical: None,
        };
        // `expect_err` would need `Adapter: Debug`, and a descriptor is not a
        // thing to derive Debug for.
        assert!(matches!(adapter.standby(), Err(Error::NoLogicalAddress)));
    }

    /// Both HDMI ports are listed, and the list is what stops a machine whose
    /// screen is on the second one from silently doing nothing.
    #[test]
    fn both_hdmi_ports_are_offered() {
        assert_eq!(DEVICES.len(), 2);
        assert!(DEVICES.contains(&"/dev/cec0"));
        assert!(DEVICES.contains(&"/dev/cec1"));
    }

    /// The name a television shows fits the protocol's field.
    #[test]
    fn the_screen_name_fits_the_fifteen_bytes_cec_allows() {
        assert!(OSD_NAME.len() <= 15, "CEC's osd_name is 15 bytes");
        assert!(!OSD_NAME.is_empty());
    }

    /// **The defect `open_first_usable` exists to fix.** `cec0` opens (the
    /// node exists) but fails to configure with [`Error::NoPhysicalAddress`]
    /// — exactly the trigger of an unplugged HDMI0 with the television on
    /// HDMI1 — and `cec1` configures cleanly. Selection must not stop at the
    /// first node that merely *opened*; it must fall through to `cec1` and
    /// select it, so the one command a transaction sends afterward goes to
    /// `cec1`.
    ///
    /// This fails against the old `open_first`, which returned as soon as
    /// `cec0` opened and never tried `cec1` at all.
    #[test]
    fn a_node_that_opens_but_wont_configure_is_skipped_for_one_that_will() {
        let opened = RefCell::new(Vec::new());
        let configured = RefCell::new(Vec::new());
        let selected = select_first_usable(
            ["/dev/cec0", "/dev/cec1"],
            |path| {
                opened.borrow_mut().push(path);
                Ok(path)
            },
            |path: &mut &str| {
                configured.borrow_mut().push(*path);
                if *path == "/dev/cec0" {
                    Err(Error::NoPhysicalAddress)
                } else {
                    Ok(())
                }
            },
        )
        .expect("cec1 configures and should be selected");
        assert_eq!(selected, "/dev/cec1");
        assert_eq!(*opened.borrow(), vec!["/dev/cec0", "/dev/cec1"]);
        assert_eq!(*configured.borrow(), vec!["/dev/cec0", "/dev/cec1"]);
    }

    /// The first node is missing outright (no `/dev/cec0` at all) rather than
    /// present-but-unconfigurable. `cec1` must still be tried and used.
    #[test]
    fn a_missing_first_node_falls_through_to_the_second() {
        let selected = select_first_usable(
            ["/dev/cec0", "/dev/cec1"],
            |path| {
                if path == "/dev/cec0" {
                    Err(Error::Absent { path: path.to_string() })
                } else {
                    Ok(path)
                }
            },
            |_path: &mut &str| Ok(()),
        )
        .expect("cec1 should be used");
        assert_eq!(selected, "/dev/cec1");
    }

    /// Every candidate is unusable: the refusal names each one's own
    /// failure, not just the last one tried.
    #[test]
    fn when_every_candidate_is_unusable_the_refusal_names_each_one() {
        let Err(why) = select_first_usable(
            ["/dev/cec0", "/dev/cec1"],
            |path| {
                if path == "/dev/cec0" {
                    Err(Error::Absent { path: path.to_string() })
                } else {
                    Ok(path)
                }
            },
            |_path: &mut &str| Err(Error::NoPhysicalAddress),
        ) else {
            panic!("no candidate configured; selection should refuse");
        };
        let Error::NoUsableAdapter { attempts } = &why else {
            panic!("wrong variant: {why:?}");
        };
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].0, "/dev/cec0");
        assert!(matches!(attempts[0].1, Error::Absent { .. }));
        assert_eq!(attempts[1].0, "/dev/cec1");
        assert!(matches!(attempts[1].1, Error::NoPhysicalAddress));
        // The message carries both candidates' own failures, not just the
        // last one tried.
        let said = why.to_string();
        assert!(said.contains("/dev/cec0"), "{said}");
        assert!(said.contains("/dev/cec1"), "{said}");
    }

    /// The first candidate is fully usable: selection stops there and the
    /// second is never even opened.
    #[test]
    fn a_fully_usable_first_candidate_means_the_second_is_never_opened() {
        let opened = RefCell::new(Vec::new());
        let selected = select_first_usable(
            ["/dev/cec0", "/dev/cec1"],
            |path| {
                opened.borrow_mut().push(path);
                Ok(path)
            },
            |_path: &mut &str| Ok(()),
        )
        .expect("cec0 configures and should be selected");
        assert_eq!(selected, "/dev/cec0");
        assert_eq!(*opened.borrow(), vec!["/dev/cec0"]);
    }
}
