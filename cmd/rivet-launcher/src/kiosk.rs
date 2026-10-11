//! **Asking the supervisor** — the launcher's end of `rivet_core::kiosk`.
//!
//! The narrow kiosk socket, ruled 2026-09-01 (`ADR-036` §5.4). The launcher cannot start, stop or reboot
//! anything: `ADR-015` makes that the supervisor's, and this is the whole of how
//! it asks.
//!
//! # Why the socket is reachable at all
//!
//! It is `0600` and owned by **this application's uid**, chowned there by the
//! supervisor because the manifest declares
//! `connects = ["rivet-sysd:kiosk"]` — `ADR-015` §5.3's grant, the same one an
//! application-to-application endpoint uses. Nothing else on the machine can
//! open it, and this can only because it was given it.
//!
//! **The operator's control surface is a different socket** (`ADR-032` §3.2)
//! and this application cannot reach that one. That is the point: a launcher
//! that could install payloads and rewrite the document would be a much larger
//! thing to trust than one that can pick a kiosk application.
//!
//! # One connection per request
//!
//! Connect, write one line, read one line, close. No pooling and no keepalive:
//! a request happens when somebody presses a button, and a connection held open
//! between button presses is a connection the supervisor has to reason about
//! across a restart of either end.

use std::io::{Read as _, Write as _};

/// Ask, and return what the supervisor said.
///
/// The `Err` is *this end could not ask* — no socket, no permission, a write
/// that failed. A refusal from the supervisor is `Ok` carrying the refusal,
/// because the asking worked and the answer was no; conflating the two would
/// make a launcher report a missing socket and a declined request identically.
pub fn ask(request: &rivet_core::kiosk::Request) -> Result<rivet_core::kiosk::Answer, String> {
    ask_with_display(request, None)
}

/// **Ask, and hand over a display lease with the word.**
///
/// `PLAN.md` §106.2. The descriptor travels as `SCM_RIGHTS` beside the request
/// text — one message, so the supervisor's `recvmsg` gets both or neither.
///
/// `lease` of `None` is exactly [`ask`], and the supervisor treats a `select`
/// with no descriptor as the drop-master path (`§104.41`) it has always
/// handled. That is what keeps a launcher and a supervisor of different
/// versions working together.
///
/// # The descriptor is DUPLICATED, not moved
///
/// `SCM_RIGHTS` gives the receiver its own descriptor for the same open file.
/// This end still owns `lease` afterwards and still has to close it — which the
/// caller does by dropping it, because the launcher keeps the *lessee id* to
/// revoke with, not the descriptor.
///
/// # Errors
///
/// As [`ask`], plus a `sendmsg` that failed.
pub fn ask_with_display(
    request: &rivet_core::kiosk::Request,
    lease: Option<std::os::fd::BorrowedFd<'_>>,
) -> Result<rivet_core::kiosk::Answer, String> {
    let path = rivet_core::kiosk::SOCKET_PATH;
    let mut stream = std::os::unix::net::UnixStream::connect(path).map_err(|e| {
        format!(
            "{path}: {e} — this application may not have been granted the kiosk socket. It is \
             granted by declaring connects = [\"rivet-sysd:kiosk\"] in the manifest, and only \
             one application on a machine may hold it"
        )
    })?;
    match lease {
        // **One `sendmsg`, carrying the text and the descriptor together.**
        // They cannot be separated: `SCM_RIGHTS` attaches to a message, so a
        // descriptor written on its own would arrive attached to nothing.
        Some(fd) => send_with_fd(&stream, request.render().as_bytes(), fd)
            .map_err(|e| format!("{path}: the request and its display could not be sent: {e}"))?,
        None => stream
            .write_all(request.render().as_bytes())
            .map_err(|e| format!("{path}: the request could not be written: {e}"))?,
    }
    // **The write half is closed, and that IS the frame.** The supervisor reads
    // to EOF, exactly as `ADR-032` §3.2's socket does, so a request that did not
    // close would hold it in a read until the connection dropped.
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|e| format!("{path}: the request could not be finished: {e}"))?;

    // **The ANSWER's cap, not the REQUEST's** — corrected 2026-09-05,
    // `rivet_core::kiosk::ANSWER_MAX_BYTES`. This read `REQUEST_MAX_BYTES + 1`,
    // which is 257 bytes, and a refusal from `select` quotes the whole offered
    // set: 253 bytes on a machine offering five applications, so a sixth would
    // have delivered a sentence that stopped mid-word. `offer` made it certain
    // rather than likely, because its answer grows with the machine.
    let mut said = String::new();
    std::io::Read::take(&mut stream, rivet_core::kiosk::ANSWER_MAX_BYTES as u64 + 1)
        .read_to_string(&mut said)
        .map_err(|e| format!("{path}: the answer could not be read: {e}"))?;

    // **The ANSWER, not a sentence** — `PLAN.md` §106.4. Flattening a refusal
    // into a string made the caller unable to tell one from success, and it
    // did not: the launcher stopped drawing and kept a lease the supervisor
    // had declined.
    Ok(rivet_core::kiosk::Answer::parse(&said))
}

/// Send `payload` and one descriptor in a single message.
///
/// The mirror of the supervisor's `recv_with_fd`. Both are short and both are
/// written out rather than shared, because `rivet-core` may not touch the world
/// (`ADR-014` §3) and a socket syscall is the world.
fn send_with_fd(
    stream: &std::os::unix::net::UnixStream,
    payload: &[u8],
    fd: std::os::fd::BorrowedFd<'_>,
) -> std::io::Result<()> {
    use std::os::fd::{AsRawFd as _, AsFd as _};

    let mut bytes = payload.to_vec();
    let mut iov = libc::iovec {
        iov_base: bytes.as_mut_ptr().cast::<libc::c_void>(),
        iov_len: bytes.len(),
    };
    let mut control = [0u8; 64];

    // SAFETY: a plain C struct; every pointer set below outlives the call.
    let mut msg: libc::msghdr = unsafe { core::mem::zeroed() };
    msg.msg_iov = &raw mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr().cast::<libc::c_void>();
    // SAFETY: a pure computation over a constant.
    msg.msg_controllen =
        unsafe { libc::CMSG_SPACE(core::mem::size_of::<libc::c_int>() as u32) } as _;

    let raw = fd.as_raw_fd();
    // SAFETY: `msg.msg_control` points at `control`, which `CMSG_SPACE` above
    // fits inside; `CMSG_FIRSTHDR` returns a pointer into it or null.
    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&raw const msg);
        if cmsg.is_null() {
            return Err(std::io::Error::other("no room for a control message"));
        }
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(core::mem::size_of::<libc::c_int>() as u32) as _;
        core::ptr::copy_nonoverlapping(
            (&raw const raw).cast::<u8>(),
            libc::CMSG_DATA(cmsg),
            core::mem::size_of::<libc::c_int>(),
        );
    }

    // SAFETY: `stream` is an open socket this process owns.
    let sent = unsafe { libc::sendmsg(stream.as_fd().as_raw_fd(), &raw const msg, 0) };
    if sent < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}
