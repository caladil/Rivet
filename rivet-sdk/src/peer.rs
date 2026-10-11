//! **Who is at the other end of a unix socket** — `SO_PEERCRED`.
//!
//! An application that provides a socket (`ADR-015` §5) is handed it by the
//! supervisor, which grants the socket's file to the consumer that declared the
//! connection. An application that also wants to know *which* uid connected —
//! a second lock behind the grant — asks the kernel, and this is that question.
//!
//! The answer is the kernel's, recorded when the peer connected, and cannot be
//! claimed by the peer. `std` offers it only behind an unstable feature, so the
//! one `getsockopt(2)` call lives here, beside the other descriptor-level calls this
//! crate makes on an application's behalf, rather than in every application.

use std::os::unix::net::UnixStream;

/// What the kernel recorded about the process at the other end of a
/// connection, at the moment it connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Credentials {
    /// The peer's process id.
    pub pid: i32,
    /// The peer's effective user id.
    pub uid: u32,
    /// The peer's effective group id.
    pub gid: u32,
}

/// The credentials of the process at the other end of `stream`.
///
/// # Errors
///
/// Whatever `getsockopt(2)` said, or an error naming a reply of the wrong
/// size. Never a guessed answer: a caller deciding whether to serve a peer
/// must be refused rather than handed a zeroed uid, which is root's.
pub fn credentials(stream: &UnixStream) -> std::io::Result<Credentials> {
    use std::os::fd::AsRawFd as _;
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let size = core::mem::size_of::<libc::ucred>();
    let mut len = libc::socklen_t::try_from(size).map_err(std::io::Error::other)?;
    // SAFETY: `stream` owns an open descriptor for the life of this call;
    // `cred` is a live, writable `ucred` and `len` says exactly its size, which
    // is what `SO_PEERCRED` writes and no more.
    #[allow(unsafe_code)]
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut cred).cast::<libc::c_void>(),
            &raw mut len,
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    if usize::try_from(len).ok() != Some(size) {
        return Err(std::io::Error::other(format!(
            "SO_PEERCRED answered {len} bytes where a ucred is {size}"
        )));
    }
    Ok(Credentials {
        pid: cred.pid,
        uid: cred.uid,
        gid: cred.gid,
    })
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// Both ends of a pair are this process, so the kernel names it: the pid
    /// is ours, and both ends agree on the uid and gid.
    #[test]
    fn a_connected_pair_names_this_process_at_both_ends() {
        let (a, b) = UnixStream::pair().expect("a socket pair");
        let from_a = credentials(&a).expect("the kernel answers");
        let from_b = credentials(&b).expect("the kernel answers");
        let ours = i32::try_from(std::process::id()).expect("a pid fits");
        assert_eq!(from_a.pid, ours);
        assert_eq!(from_a, from_b);
    }
}
