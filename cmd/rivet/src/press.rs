//! **Delivering a press to the launcher** — `ADR-012` §9.4.
//!
//! The whole of the adapter half. It connects to the socket `rivet-launcher`
//! declares with `provides = ["socket:input"]`, writes one action, and closes.
//!
//! # Why this process may open it at all
//!
//! `ADR-015` §5.3 has the supervisor bind a `provides` socket and chown it to
//! the **one consumer** granted it. Nothing declares
//! `connects = ["rivet-launcher:input"]`, so it is never granted to anybody and
//! stays `0600 root:root`.
//!
//! This process is root — it is reached over `ADR-035`'s control plane, which
//! `rivet-mcpd` serves — so it can open it, and **nothing unprivileged can.**
//! That is the grant mechanism doing its job by *not* granting: the narrowest
//! possible reach, with no new permission invented for it.
//!
//! # The interlock is the socket's existence
//!
//! `ADR-012` §9.3: with no launcher there are no kiosk applications and nothing
//! an input could mean. A missing socket is exactly that state, so it is the
//! interlock — and the refusal says so in those words rather than reporting a
//! file that is not there.

use std::io::Write as _;

use rivet_core::operation::machine::MachineError;

/// Where the launcher listens.
///
/// Composed from the two constants both ends share (`rivet_core::input`) rather
/// than written out, because a second spelling of a path is a second thing to
/// keep in step (`ADR-011`).
fn socket_path() -> Option<String> {
    let name = rivet_core::service::ServiceName::new(rivet_core::input::PROVIDER)?;
    Some(format!(
        "{}/{}",
        rivet_core::service::listen_dir(&name),
        rivet_core::input::ENDPOINT
    ))
}

/// **Is there a launcher listening?**
///
/// # Errors
///
/// [`MachineError::Refused`] when there is not, saying what that means rather
/// than naming a missing file.
pub fn can_press() -> Result<(), MachineError> {
    let Some(path) = socket_path() else {
        return Err(MachineError::Refused(format!(
            "`{}` is not a service name this build can compose a socket path from, which is a \
             defect in this binary rather than a state of the machine",
            rivet_core::input::PROVIDER,
        )));
    };
    if std::path::Path::new(&path).exists() {
        return Ok(());
    }
    Err(MachineError::Refused(format!(
        "nothing is listening for input on this machine. `{provider}` declares \
         provides = [\"{endpoint}\"] and the supervisor binds it at {path} while that \
         application runs — so an absent socket means the launcher is not running.\n\n\
         ADR-012 §9.3: without a launcher there are no kiosk applications, and without those \
         there is nothing a press could mean. This is refused rather than queued: a press \
         held for a consumer that may never exist is the silent failure this project keeps \
         naming.\n\n\
         `rivet get status` says whether {provider} is running, and `rivet get log {provider}` \
         says why it is not.",
        provider = rivet_core::input::PROVIDER,
        endpoint = rivet_core::input::ENDPOINT,
    )))
}

/// **Press it.**
///
/// One connection, one line, close — `cmd/rivet-launcher/src/kiosk.rs`'s shape
/// in the other direction, and for its reason: a press happens when somebody
/// asks for one, and a connection held between presses is a connection both
/// ends have to reason about across a restart of either.
///
/// # Errors
///
/// [`MachineError::Refused`] with what the write said. **A failure here is not
/// a press that half happened**: the launcher acts on a whole line, so a
/// connection that failed delivered nothing.
pub fn press(action: &str) -> Result<(), MachineError> {
    can_press()?;
    let Some(path) = socket_path() else {
        return Err(MachineError::Refused(
            "the launcher's socket path could not be composed".to_string(),
        ));
    };

    let mut socket = std::os::unix::net::UnixStream::connect(&path).map_err(|e| {
        MachineError::Refused(format!(
            "{path}: {e}. The socket is there and this process could not open it, which is a \
             different problem from no launcher: it is 0600 root:root and this operation \
             reaches it as root over ADR-035's control plane"
        ))
    })?;

    socket
        .write_all(format!("{action}\n").as_bytes())
        .map_err(|e| MachineError::Refused(format!("{path}: the press could not be written: {e}")))?;

    // **The close is the frame**, as it is on every other socket here. The
    // launcher reads to EOF, so a connection that stayed open would hold it in
    // a read rather than delivering anything.
    socket.shutdown(std::net::Shutdown::Write).map_err(|e| {
        MachineError::Refused(format!("{path}: the press could not be finished: {e}"))
    })?;
    Ok(())
}
