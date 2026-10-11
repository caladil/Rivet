//! **The screen an application was handed** — `ADR-036` §8.1, `PLAN.md` §106.2.
//!
//! # The first article, applied to a display
//!
//! > An application declares what it needs and is **handed** it. It does not go
//! > and take one.
//!
//! A kiosk application on a Rivet **never opens `/dev/dri/card0`**. It gets
//! `EACCES` if it tries: `rivet-launcher` holds DRM master for the life of the
//! machine and leases the display out (`ADR-036` §2). What arrives here is
//! a **lessee descriptor** — a DRM device the application can modeset and scan
//! out on, and which can be revoked without the launcher ever giving up master.
//!
//! # Why this is a channel and not a variable
//!
//! `RIVET_DISPLAY` names a **socket**, not a card. Descriptors arrive on it:
//! the first at start, and **another whenever the launcher re-issues**.
//!
//! That is not decoration. A revoked lease cannot be resumed — the lessee's
//! descriptor dies with the revocation — so answering *"no, don't exit"* to an
//! exit confirmation means delivering a **new** one. `ADR-036` §8.1 makes
//! accepting that part of what `devices = ["display"]` means, and a mechanism that
//! could only fire once would have shipped the obligation without the means.
//!
//! **So an application must be prepared to tear its rendering down and build it
//! again.** For a CPU renderer on a dumb buffer that is unmap, remap, redraw.
//! For a GPU renderer it is the whole EGL/GBM context, which is the expensive
//! half and the reason `ADR-036` §8.1 records the risk it does.

// **`unsafe` for the whole module** — `recvmsg` and the `CMSG_*` layout.
//
// The crate denies it and [`listen`](crate::listen) lifts it for one function.
// Here it is the module, because receiving a descriptor is four unsafe steps
// that only make sense together — the message, the control header, the payload
// copy and the `OwnedFd` — and annotating each separately would suggest they
// could be reasoned about apart. Each carries its own SAFETY note.
#![allow(unsafe_code)]

use std::os::fd::OwnedFd;

/// The variable naming this application's display channel.
///
/// **Absent for everything that is not a kiosk application**, which is most
/// things. Its absence is not a fault.
pub const DISPLAY_ENV: &str = "RIVET_DISPLAY";

/// Why a display could not be taken.
#[derive(Debug)]
pub enum DisplayError {
    /// No `RIVET_DISPLAY`. This process was not given a screen — either it is
    /// not a kiosk application, or the launcher has not selected it.
    NotGranted,
    /// `RIVET_DISPLAY` was set to something that is not a descriptor number.
    Malformed(String),
    /// The channel is open and the far end has gone. The launcher revoked and
    /// nothing more is coming.
    Closed,
    /// A signal arrived during the wait. **Internal**: [`Channel::next`] and
    /// [`Channel::try_next`] both retry on it and neither returns it, so a
    /// caller never has to know about `EINTR`.
    Interrupted,
    /// The channel worked and the display could not be driven — the lease
    /// arrived and `crate::screen` could not make a screen out of it.
    ///
    /// **Separate from [`Self::Failed`] on purpose.** They mean different
    /// things to a caller: `Failed` is the channel, this is the hardware behind
    /// it, and neither is [`Self::NotGranted`], which is not a problem at all.
    NotDriveable(String),
    /// The read itself failed.
    Failed(std::io::Error),
}

impl std::fmt::Display for DisplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotGranted => write!(
                f,
                "{DISPLAY_ENV} is not set, so this process was not handed a display. A kiosk \
                 application is given one when the launcher selects it; it must never open \
                 /dev/dri/card0, which answers EACCES (ADR-036 §3.1)"
            ),
            Self::Malformed(v) => write!(f, "{DISPLAY_ENV} is {v:?}, which is not a descriptor"),
            Self::Closed => write!(
                f,
                "the display channel closed: the launcher has taken the screen back and will \
                 send no more descriptors on it"
            ),
            Self::Interrupted => write!(
                f,
                "a signal arrived while waiting for a display. This is retried and is never \
                 returned to a caller; seeing it means one escaped"
            ),
            Self::NotDriveable(why) => write!(
                f,
                "a display was handed over and could not be driven: {why}. The channel is \
                 working — this is about the display behind it"
            ),
            Self::Failed(e) => write!(f, "the display channel could not be read: {e}"),
        }
    }
}

impl std::error::Error for DisplayError {}

/// The channel this application receives displays on.
///
/// Held for the life of the process, because a **later** descriptor arrives on
/// the same socket. An application that read one and dropped this would work
/// until the first re-issue and then never draw again.
#[derive(Debug)]
pub struct Channel {
    fd: std::os::fd::RawFd,
}

impl Channel {
    /// Take this application's display channel from the environment.
    ///
    /// # Errors
    ///
    /// [`DisplayError::NotGranted`] when there is none, which is the ordinary
    /// state of every application that is not currently on the screen.
    ///
    /// [`DisplayError::Malformed`] for anything that is not a descriptor the
    /// supervisor could have placed.
    pub fn from_env() -> Result<Self, DisplayError> {
        let raw = std::env::var(DISPLAY_ENV).map_err(|_| DisplayError::NotGranted)?;
        Self::from_value(&raw)
    }

    /// The value, as a channel — [`Self::from_env`] minus the environment.
    ///
    /// **Split out so the rule below can be TESTED.** `std::env::set_var` is
    /// unsound in a process running tests on several threads, so a test that
    /// went through the variable would either be racy or would not exist; this
    /// is the same code reached by a caller that already has the string.
    ///
    /// # Errors
    ///
    /// [`DisplayError::Malformed`] for anything that is not a descriptor the
    /// supervisor could have placed.
    pub fn from_value(raw: &str) -> Result<Self, DisplayError> {
        let fd: std::os::fd::RawFd = raw
            .trim()
            .parse()
            .map_err(|_| DisplayError::Malformed(raw.to_owned()))?;
        // **0, 1 and 2 are refused, not just negatives** — the audit of
        // 2026-09-03's LOW list, *"`RIVET_DISPLAY=0` accepted"*.
        //
        // `cmd/rivet-sysd`'s `spawn` places the display end at `3 + n` and can
        // never name a standard stream, so a value below three did not come
        // from a supervisor. Accepting `0` meant an application doing `recvmsg`
        // on its own **stdin** and waiting there for a lease that cannot
        // arrive: a kiosk that starts, draws nothing, and reports nothing —
        // which is the failure this crate exists to make impossible to have
        // silently.
        //
        // **The three is `listen::LISTEN_FDS_START`'s and not a second copy of
        // it** (`ADR-011`). It is one fact — *the lowest descriptor a supervisor
        // can hand over, because 0, 1 and 2 are the standard streams* — and the
        // display end is placed immediately after the listening sockets by the
        // same loop in `cmd/rivet-sysd`.
        if fd < crate::listen::LISTEN_FDS_START {
            return Err(DisplayError::Malformed(raw.to_owned()));
        }
        Ok(Self { fd })
    }

    /// **Wait for the next display descriptor**, and return it.
    ///
    /// Blocks. The first call is how an application gets its screen at start;
    /// a later one returns when the launcher re-issues after a revocation.
    ///
    /// **Restarted on `EINTR`**, which `crate::flip`'s `await_flip` already did
    /// and this did not (`PLAN.md` §123, Wave 5.1). A signal arriving while an
    /// application waits for its screen is an ordinary event — `SIGCHLD` from
    /// anything it spawned will do it — and it used to surface as
    /// [`DisplayError::Failed`] and, in every consumer, as a program that gave
    /// up on having a display.
    ///
    /// # Errors
    ///
    /// [`DisplayError::Closed`] when the launcher has gone and nothing more is
    /// coming — which is a reason to exit, not to retry.
    pub fn next(&self) -> Result<OwnedFd, DisplayError> {
        loop {
            match self.receive(0) {
                Err(DisplayError::Interrupted) => {}
                Ok(Some(lease)) => return Ok(lease),
                // Unreachable rather than impossible: `receive` answers
                // `Ok(None)` only for `EAGAIN` under `MSG_DONTWAIT`, and this
                // passes no flags. Reported rather than unwrapped, because a
                // panic in the SDK is a kiosk application that dies at start.
                Ok(None) => {
                    return Err(DisplayError::Failed(std::io::Error::other(
                        "a blocking receive on the display channel returned nothing",
                    )));
                }
                Err(other) => return Err(other),
            }
        }
    }

    /// **Is there a display descriptor waiting?** — without blocking to find
    /// out.
    ///
    /// `Ok(None)` means *not right now*, which is what an application drawing
    /// frames needs: `ADR-036` §8.1 obliges a kiosk application to accept a
    /// re-issued descriptor, and until 2026-09-03 the only way to ask was
    /// [`Channel::next`], which blocks for ever if the launcher never re-issues.
    /// An application cannot both draw and wait on that, so in practice no
    /// consumer asked at all.
    ///
    /// Call it once a frame. It costs one `recvmsg` that returns immediately.
    ///
    /// # Errors
    ///
    /// As [`Channel::next`]. `Closed` still means the launcher has gone.
    pub fn try_next(&self) -> Result<Option<OwnedFd>, DisplayError> {
        loop {
            match self.receive(libc::MSG_DONTWAIT) {
                Err(DisplayError::Interrupted) => {}
                other => return other,
            }
        }
    }

    /// One `recvmsg`. `Ok(None)` only when `MSG_DONTWAIT` was asked for and
    /// nothing had arrived.
    fn receive(&self, flags: libc::c_int) -> Result<Option<OwnedFd>, DisplayError> {
        use std::os::fd::FromRawFd as _;

        // One byte of payload: `SCM_RIGHTS` attaches to a MESSAGE, and a
        // message with no data is not sent, so the sender pads it.
        let mut byte = [0u8; 1];
        let mut control = ControlBuffer::zeroed();
        let mut iov = libc::iovec {
            iov_base: byte.as_mut_ptr().cast::<libc::c_void>(),
            iov_len: byte.len(),
        };
        // SAFETY: a plain C struct; the pointers below outlive the call.
        let mut msg: libc::msghdr = unsafe { core::mem::zeroed() };
        msg.msg_iov = &raw mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.0.as_mut_ptr().cast::<libc::c_void>();
        msg.msg_controllen = CONTROL_BYTES as _;

        // SAFETY: `self.fd` is the descriptor the supervisor passed; `msg`
        // points at live buffers.
        let read = unsafe {
            libc::recvmsg(self.fd, &raw mut msg, libc::MSG_CMSG_CLOEXEC | flags)
        };
        if read < 0 {
            let e = std::io::Error::last_os_error();
            return match e.raw_os_error() {
                Some(libc::EINTR) => Err(DisplayError::Interrupted),
                // Only reachable with `MSG_DONTWAIT`: nothing had arrived.
                Some(libc::EAGAIN | libc::EWOULDBLOCK) if flags != 0 => Ok(None),
                _ => Err(DisplayError::Failed(e)),
            };
        }
        if read == 0 {
            // A zero-length read on a SEQPACKET socket is the far end closing.
            return Err(DisplayError::Closed);
        }

        // **`MSG_CTRUNC` says the kernel had more to give than would fit**, and
        // a descriptor it could not deliver is one **already open in this
        // process** with no name for it — a leak for the life of the program,
        // once per re-issue. The control buffer is sized for several, and the
        // supervisor sends exactly one, so this is not expected; it is checked
        // because the failure is silent and permanent (`PLAN.md` §123's LOW
        // list).
        //
        // Read BEFORE the walk and acted on AFTER it, so that whatever did
        // arrive is closed rather than left open — the shape
        // `cmd/rivet-sysd`'s `recv_with_fd` settled on under Wave 7c, and this
        // is deliberately the same treatment rather than a second one.
        let truncated = (msg.msg_flags & libc::MSG_CTRUNC) != 0;

        // **Walk EVERY control message and take EVERY descriptor**, keeping the
        // first and closing the rest.
        //
        // Until 2026-09-04 this read one `c_int` out of the first cmsg and
        // stopped, so a message carrying two descriptors that both FIT left the
        // second one open with nothing naming it — the leak the `MSG_CTRUNC`
        // check above cannot see, because nothing was truncated. Neither half is
        // reachable from this build's supervisor; both are reachable from
        // anything else on the far end of the socket, and a leaked descriptor
        // in a program that runs for months is not a thing that announces
        // itself.
        let mut taken: Option<OwnedFd> = None;
        let mut carried = 0usize;
        // SAFETY: `msg` was filled by `recvmsg`, which is what these macros read.
        let mut cmsg = unsafe { libc::CMSG_FIRSTHDR(&raw const msg) };
        while !cmsg.is_null() {
            // SAFETY: non-null and pointing into `control`, which is live.
            let (level, kind, len) =
                unsafe { ((*cmsg).cmsg_level, (*cmsg).cmsg_type, (*cmsg).cmsg_len) };
            if level == libc::SOL_SOCKET && kind == libc::SCM_RIGHTS {
                // SAFETY: `CMSG_DATA` on a cmsg `recvmsg` produced.
                let data = unsafe { libc::CMSG_DATA(cmsg) };
                // **The cast is kept and the lint allowed**, not the other way
                // round: `cmsg_len`'s Rust type is whatever the target's header
                // says, and removing a cast that is a no-op *here* is how a
                // cross build stops compiling.
                //
                // SAFETY: `CMSG_LEN` is a pure computation over a constant.
                #[allow(clippy::unnecessary_cast)]
                let count = descriptors_in(len as usize, unsafe { libc::CMSG_LEN(0) } as usize);
                for i in 0..count {
                    let mut raw: libc::c_int = -1;
                    // SAFETY: reading `count` descriptors out of the data area
                    // the header says holds exactly that many, unaligned
                    // because the control buffer has no alignment guarantee for
                    // `c_int`.
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            data.add(i * core::mem::size_of::<libc::c_int>()),
                            (&raw mut raw).cast::<u8>(),
                            core::mem::size_of::<libc::c_int>(),
                        );
                    }
                    if raw < 0 {
                        continue;
                    }
                    carried += 1;
                    // SAFETY: a descriptor the kernel just created in this
                    // process; nothing else owns it. Made owned before any
                    // early return below, so every path out closes it.
                    let owned = unsafe { OwnedFd::from_raw_fd(raw) };
                    if taken.is_none() {
                        taken = Some(owned);
                    }
                    // Any second descriptor is dropped here, which closes it.
                }
            }
            // SAFETY: as above.
            cmsg = unsafe { libc::CMSG_NXTHDR(&raw const msg, cmsg) };
        }

        if truncated {
            // `taken` is dropped here, which closes it. **Refuse rather than
            // guess**: a message this build cannot fully account for is not one
            // to draw on.
            return Err(DisplayError::Failed(std::io::Error::other(
                "a message on the display channel carried more descriptors than fit, so at \
                 least one is open in this process with nothing naming it",
            )));
        }
        match taken {
            Some(fd) => Ok(Some(fd)),
            None if carried == 0 => Err(DisplayError::Failed(std::io::Error::other(
                "a message arrived on the display channel with no descriptor",
            ))),
            // Every descriptor in the message was negative, which no kernel
            // produces; saying so is better than reporting "no descriptor" for
            // a message that had some.
            None => Err(DisplayError::Failed(std::io::Error::other(
                "the display channel delivered an invalid descriptor",
            ))),
        }
    }
}

/// The control-message buffer, **aligned for a `cmsghdr`**.
///
/// A bare `[u8; N]` is aligned to 1, and the `CMSG_*` macros cast pointers into
/// it to `*const cmsghdr` and dereference them. On aarch64 and x86_64 that
/// worked; it was undefined behaviour that happened to be benign, which is the
/// class of defect that stops being benign when a compiler gets a new
/// optimisation. `cmd/rivet-sysd`'s `ControlBuffer` is the same type for the
/// same reason and this is deliberately its twin.
#[repr(C, align(8))]
struct ControlBuffer([u8; CONTROL_BYTES]);

impl ControlBuffer {
    const fn zeroed() -> Self {
        Self([0u8; CONTROL_BYTES])
    }
}

/// Room for several descriptors, so that a sender attaching more than one is
/// **visible** rather than silently truncated into looking like a single one.
const CONTROL_BYTES: usize = 64;

/// How many descriptors a `SCM_RIGHTS` control message carries.
///
/// `cmsg_len` counts the header as well as the data, so the header comes off
/// before the division — with `checked_sub`, because the value is read out of a
/// buffer through a raw pointer after a `recvmsg` that may have set
/// `MSG_CTRUNC`. An underflow on `usize` is a number near `2^64`, and the loop
/// that follows would then read descriptors out of memory past the end of the
/// control buffer.
fn descriptors_in(cmsg_len: usize, header_bytes: usize) -> usize {
    cmsg_len
        .checked_sub(header_bytes)
        .map_or(0, |payload| payload / core::mem::size_of::<libc::c_int>())
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test — the same block every
    // inline test module in this workspace carries.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// A `SOCK_SEQPACKET` pair, the shape `cmd/rivet-sysd` makes for a display
    /// channel. Returns (this application's end, the supervisor's end).
    fn channel_pair() -> (Channel, OwnedFd) {
        use std::os::fd::FromRawFd as _;
        let mut ends = [-1i32; 2];
        // SAFETY: two `c_int`s on this stack; `socketpair` writes exactly two.
        let made = unsafe {
            libc::socketpair(libc::AF_UNIX, libc::SOCK_SEQPACKET, 0, ends.as_mut_ptr())
        };
        assert_eq!(made, 0, "socketpair: {}", std::io::Error::last_os_error());
        // SAFETY: fresh descriptors, owned only here.
        let theirs = unsafe { OwnedFd::from_raw_fd(ends[1]) };
        (Channel { fd: ends[0] }, theirs)
    }

    /// Send one descriptor the way the supervisor does: one byte of payload
    /// with an `SCM_RIGHTS` control message beside it.
    fn send_one(over: &OwnedFd, payload: libc::c_int) {
        use std::os::fd::AsRawFd as _;
        let mut byte = [b'!'; 1];
        let mut control = [0u8; 64];
        let mut iov = libc::iovec {
            iov_base: byte.as_mut_ptr().cast::<libc::c_void>(),
            iov_len: 1,
        };
        // SAFETY: a plain C struct filled in below; the buffers outlive the call.
        let mut msg: libc::msghdr = unsafe { core::mem::zeroed() };
        msg.msg_iov = &raw mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast::<libc::c_void>();
        // SAFETY: the space one `c_int` of `SCM_RIGHTS` needs.
        msg.msg_controllen = unsafe { libc::CMSG_SPACE(4) } as _;
        // SAFETY: `msg_control` points at a buffer at least that long.
        unsafe {
            let cmsg = libc::CMSG_FIRSTHDR(&raw const msg);
            (*cmsg).cmsg_level = libc::SOL_SOCKET;
            (*cmsg).cmsg_type = libc::SCM_RIGHTS;
            (*cmsg).cmsg_len = libc::CMSG_LEN(4) as _;
            core::ptr::copy_nonoverlapping(
                (&raw const payload).cast::<u8>(),
                libc::CMSG_DATA(cmsg),
                4,
            );
            let sent = libc::sendmsg(over.as_raw_fd(), &raw const msg, 0);
            assert!(sent > 0, "sendmsg: {}", std::io::Error::last_os_error());
        }
    }

    /// Send several descriptors in ONE `SCM_RIGHTS` control message, which is
    /// what a sender that is not this build's supervisor may do.
    fn send_many(over: &OwnedFd, payload: &[libc::c_int]) {
        use std::os::fd::AsRawFd as _;
        let bytes = core::mem::size_of_val(payload) as u32;
        let mut byte = [b'!'; 1];
        let mut control = [0u8; 64];
        let mut iov = libc::iovec {
            iov_base: byte.as_mut_ptr().cast::<libc::c_void>(),
            iov_len: 1,
        };
        // SAFETY: a plain C struct filled in below; the buffers outlive the call.
        let mut msg: libc::msghdr = unsafe { core::mem::zeroed() };
        msg.msg_iov = &raw mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast::<libc::c_void>();
        // SAFETY: the space `payload.len()` descriptors need. The buffer above
        // is 64 bytes, so this asserts rather than overflowing silently.
        let space = unsafe { libc::CMSG_SPACE(bytes) } as usize;
        assert!(space <= control.len(), "the test's own control buffer is too small");
        msg.msg_controllen = space as _;
        // SAFETY: `msg_control` points at a buffer at least that long.
        unsafe {
            let cmsg = libc::CMSG_FIRSTHDR(&raw const msg);
            (*cmsg).cmsg_level = libc::SOL_SOCKET;
            (*cmsg).cmsg_type = libc::SCM_RIGHTS;
            (*cmsg).cmsg_len = libc::CMSG_LEN(bytes) as _;
            core::ptr::copy_nonoverlapping(
                payload.as_ptr().cast::<u8>(),
                libc::CMSG_DATA(cmsg),
                bytes as usize,
            );
            let sent = libc::sendmsg(over.as_raw_fd(), &raw const msg, 0);
            assert!(sent > 0, "sendmsg: {}", std::io::Error::last_os_error());
        }
    }

    /// **A second descriptor in one message is CLOSED, not leaked** — the
    /// audit of 2026-09-03's LOW list, *"`Channel::next` ignores `MSG_CTRUNC`
    /// and leaks any second fd"*.
    ///
    /// The `MSG_CTRUNC` half was closed on 2026-09-03. This is the other half,
    /// and it is the one the truncation check cannot see: two descriptors that
    /// both FIT in the control buffer arrive with no flag set, the old code
    /// read the first `c_int` and stopped, and the second stayed open in the
    /// application with nothing naming it — for the life of a program that runs
    /// for months.
    ///
    /// # The pipe is the instrument, and it is not a descriptor count
    ///
    /// Counting `/proc/self/fd` would answer this and would answer it
    /// unreliably: these tests run on several threads and any other one opening
    /// a file between the two readings changes the number. **A pipe's read end
    /// reports end-of-file exactly when every write end is closed**, so the
    /// question becomes one this process can settle by itself. The first
    /// descriptor sent is the read end and the second is the write end: if the
    /// second was closed, reading the first gives EOF; if it was leaked, the
    /// same read gives `EAGAIN`, because this process is still holding the
    /// thing it would be waiting for.
    #[test]
    fn a_second_descriptor_in_one_message_is_closed_rather_than_leaked() {
        use std::os::fd::AsRawFd as _;

        let (channel, supervisor) = channel_pair();

        let mut ends = [-1i32; 2];
        // SAFETY: two `c_int`s on this stack.
        assert_eq!(unsafe { libc::pipe(ends.as_mut_ptr()) }, 0);
        // Read end first, write end second — so the descriptor the SDK KEEPS is
        // the read end and the one it must close is the write end.
        send_many(&supervisor, &ends);
        // SAFETY: the sender's own copies, sent and no longer wanted here.
        // `SCM_RIGHTS` duplicates rather than moves, so the in-flight copies
        // survive this. Both go, so the only write end left anywhere is the one
        // the receive is about to produce.
        unsafe {
            libc::close(ends[0]);
            libc::close(ends[1]);
        }

        let got = channel.try_next().expect("the receive worked");
        let got = got.expect("two descriptors were sent, so one must arrive");

        // Non-blocking, so a leak is an answer rather than a hang.
        // SAFETY: `got` is an open descriptor this process owns.
        assert_eq!(unsafe { libc::fcntl(got.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) }, 0);
        let mut buf = [0u8; 1];
        // SAFETY: reading at most one byte into a live buffer on this stack.
        let read = unsafe { libc::read(got.as_raw_fd(), buf.as_mut_ptr().cast(), 1) };
        let why = std::io::Error::last_os_error();
        eprintln!("reading the received pipe gave {read} ({why})");
        assert_eq!(
            read, 0,
            "the pipe's read end did not reach end-of-file, so a write end is still open in \
             this process — the second descriptor was leaked"
        );
        drop(got);
    }

    /// **Asking does not block when nothing has arrived**, which is the whole
    /// reason `try_next` exists — `PLAN.md` §123, Wave 5.1.
    ///
    /// `ADR-036` §8.1 obliges a kiosk application to accept a re-issued
    /// display, and the only way to ask used to be `next()`, which blocks for
    /// ever if the launcher never re-issues. An application cannot both draw
    /// and wait on that, so in practice none of them asked.
    #[test]
    fn asking_for_a_display_that_has_not_arrived_answers_at_once() {
        let (channel, _supervisor) = channel_pair();
        let began = std::time::Instant::now();
        let answer = channel.try_next();
        let waited = began.elapsed();
        assert!(
            matches!(answer, Ok(None)),
            "an empty channel must answer 'not yet', not {answer:?}"
        );
        assert!(waited < std::time::Duration::from_millis(100), "it blocked for {waited:?}");
    }

    /// And a descriptor that HAS arrived comes back, on the same call.
    #[test]
    fn a_descriptor_sent_on_the_channel_is_received_and_is_usable() {
        use std::io::{Read as _, Write as _};
        let (channel, supervisor) = channel_pair();

        // Something recognisable to send: a pipe with a byte in it, so the
        // received descriptor can be shown to be the same open file and not
        // merely some number.
        let mut ends = [-1i32; 2];
        // SAFETY: two `c_int`s on this stack.
        assert_eq!(unsafe { libc::pipe(ends.as_mut_ptr()) }, 0);
        // SAFETY: fresh descriptors from `pipe`.
        let (read_end, write_end) = unsafe {
            use std::os::fd::FromRawFd as _;
            (OwnedFd::from_raw_fd(ends[0]), OwnedFd::from_raw_fd(ends[1]))
        };
        let mut writer = std::fs::File::from(write_end);
        writer.write_all(b"Z").expect("a byte into the pipe");
        drop(writer);

        use std::os::fd::AsRawFd as _;
        send_one(&supervisor, read_end.as_raw_fd());

        let got = channel.try_next().expect("the receive worked");
        let got = got.expect("a descriptor was sent, so one must arrive");
        let mut back = String::new();
        std::fs::File::from(got).read_to_string(&mut back).expect("readable");
        assert_eq!(back, "Z", "the descriptor received is not the one sent");
    }

    /// **A closed far end is `Closed` and not an error**, because the two mean
    /// opposite things to an application: one is *exit*, the other is *try
    /// again*.
    #[test]
    fn a_supervisor_that_has_gone_reads_as_closed() {
        let (channel, supervisor) = channel_pair();
        drop(supervisor);
        assert!(
            matches!(channel.try_next(), Err(DisplayError::Closed)),
            "a channel whose far end has gone must say so by name"
        );
    }

    /// The absence of a display is an ordinary state and must read as one.
    #[test]
    fn a_process_with_no_display_is_told_so_by_name() {
        let why = DisplayError::NotGranted.to_string();
        assert!(why.contains("RIVET_DISPLAY"));
        // And it must say the thing an author would otherwise try.
        assert!(why.contains("/dev/dri/card0"));
    }

    #[test]
    fn a_closed_channel_says_the_launcher_took_the_screen_back() {
        assert!(DisplayError::Closed.to_string().contains("launcher"));
    }

    #[test]
    fn a_malformed_value_is_quoted_back() {
        let why = DisplayError::Malformed("banana".to_string()).to_string();
        assert!(why.contains("banana"));
    }

    /// **A standard stream is not a display channel** — the audit of
    /// 2026-09-03's LOW list, *"`RIVET_DISPLAY=0` accepted"*.
    ///
    /// `RIVET_DISPLAY=0` used to build a `Channel` on this process's own stdin,
    /// which the supervisor makes an empty pipe: the application would
    /// `recvmsg` there and wait for a lease that cannot arrive. A kiosk that
    /// starts, draws nothing and says nothing.
    #[test]
    fn a_standard_stream_is_refused_as_a_display_channel() {
        for stream in ["0", "1", "2", " 2 "] {
            let answer = Channel::from_value(stream);
            assert!(
                matches!(answer, Err(DisplayError::Malformed(_))),
                "{stream:?} was accepted as a display channel: {answer:?}"
            );
        }
        assert!(Channel::from_value("-1").is_err());
        assert!(Channel::from_value("banana").is_err());
        // And the first number a supervisor can actually place still works.
        assert!(Channel::from_value("3").is_ok());
    }
}
