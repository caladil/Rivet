//! **The kiosk socket — the renderer's end of Aura.**
//!
//! `internal/ipc/ipc.go`'s `Sender`, ported. Newline-delimited JSON, one-way,
//! the controller listening and the renderer connecting and reconnecting.
//!
//! # This program does not bind it, and that is the whole point
//!
//! The Go program keeps both paths — adopt a passed descriptor if there is one,
//! otherwise `net.Listen("unix", "/run/aura/kiosk.sock")` — because one binary
//! runs on a developer's machine and on a Rivet. **There is no bind here.** An
//! application runs as its own unprivileged uid and cannot create a directory
//! under a root-owned `/run`; that is `ADR-015` §5.3's point, and it is the
//! error the Go controller died on before its manifest declared the socket:
//!
//!     listen unix /run/aura/kiosk.sock: bind: no such file or directory
//!
//! **Where the socket lives is not the application's to say.** The supervisor
//! binds it, listens on it, chowns the file to the one consumer that declared
//! `connects`, and passes the descriptor with the endpoint's name in
//! `LISTEN_FDNAMES`. A missing descriptor is therefore a refusal to start, not
//! a fallback: falling back would mean binding a socket the renderer has not
//! been granted and cannot open, and then reporting success.
//!
//! # One connection, and a new one replaces the old
//!
//! The endpoint has exactly one consumer — the grant is a chown and a chown
//! names one uid — so a second connection is **the renderer having restarted**,
//! not a second reader. Taking the new one and dropping the old is what a
//! restarted renderer needs and what a wedged one deserves.

use std::io::Write as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};

use crate::wire::Message;

/// The endpoint's name, in the manifest and in `LISTEN_FDNAMES`.
///
/// **One spelling.** It is `provides = ["socket:kiosk.sock"]` here and
/// `connects = ["aura-controller:kiosk.sock"]` in the renderer, and it is the
/// last component of the Go program's `/run/aura/kiosk.sock` so that a
/// developer reading either tree finds the same word.
pub const ENDPOINT: &str = "kiosk.sock";

/// The listening socket and the renderer currently on it.
pub struct Sender {
    /// `None` whenever no renderer is connected, which is most of a cold start.
    connection: Mutex<Option<UnixStream>>,
}

impl Sender {
    /// Take the handed descriptor and begin accepting.
    ///
    /// `on_connect` is called on the accept thread each time a renderer
    /// connects, **after** the connection is installed, so that anything it
    /// sends reaches the renderer that triggered it. It is what replays the
    /// display configuration: the schema promises that *"on each new connection
    /// the controller replays the display configuration … so the renderer never
    /// has to persist any of it."*
    ///
    /// # Errors
    ///
    /// A sentence, when the supervisor handed no socket by this name, when the
    /// descriptor was already taken in this process, or when the accept thread
    /// cannot be started. All three are refusals to start.
    pub fn take(
        on_connect: impl Fn(&Self) + Send + 'static,
    ) -> Result<Arc<Self>, String> {
        let socket = rivet_sdk::listen::socket(ENDPOINT).ok_or_else(|| {
            format!(
                "kiosk: this machine handed no socket named {ENDPOINT}. The manifest must \
                 declare `provides = [\"socket:{ENDPOINT}\"]` and the supervisor must have \
                 opened it; this program does not bind one of its own"
            )
        })?;
        let listener = socket.into_unix_listener().ok_or_else(|| {
            format!("kiosk: the descriptor for {ENDPOINT} was already taken in this process")
        })?;

        let sender = Arc::new(Self {
            connection: Mutex::new(None),
        });
        let accepting = Arc::clone(&sender);
        std::thread::Builder::new()
            .name("kiosk-accept".to_string())
            .spawn(move || accepting.accept_loop(&listener, &on_connect))
            .map_err(|e| format!("kiosk: the accept thread could not be started: {e}"))?;
        Ok(sender)
    }

    /// Accept for ever. Returns only when the listener stops working, which on
    /// a live socket does not happen.
    fn accept_loop(
        self: &Arc<Self>,
        listener: &UnixListener,
        on_connect: &(impl Fn(&Self) + Send + 'static),
    ) {
        loop {
            let Ok((stream, _)) = listener.accept() else {
                // The listener is gone. There is nothing this thread can do and
                // nothing for it to say that the supervisor will not say better
                // when the process exits.
                return;
            };
            // Installed before the callback runs, so what the callback sends
            // goes to the renderer that just arrived. The old connection is
            // dropped here, which closes it.
            {
                let mut held = self.connection.lock().unwrap_or_else(|e| e.into_inner());
                *held = Some(stream);
            }
            on_connect(self);
        }
    }

    /// A sender already holding `stream`, for tests.
    ///
    /// The framing and the close-on-error rule are the two things worth
    /// asserting here, and neither needs a supervisor — but [`Sender::take`] is
    /// the only other constructor and it needs a handed descriptor. A socket
    /// pair gets the same code under test with the platform left out.
    #[cfg(test)]
    fn on(stream: UnixStream) -> Self {
        Self {
            connection: Mutex::new(Some(stream)),
        }
    }

    /// Whether a renderer is connected.
    ///
    /// **Only the schedule asks**, and only so it can journal a directive as
    /// *dropped* rather than as sent. Nothing branches on it: a state message
    /// with nobody to read it is not worth a log line once a second, and a
    /// directive with nobody to read it is worth one, because a person set that
    /// directive and will otherwise wonder why the screen did not change.
    pub fn is_connected(&self) -> bool {
        self.connection
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    /// Render `message` as one line of JSON and write it.
    ///
    /// **Silently does nothing when no renderer is connected**, which is the Go
    /// behaviour and is the right one: the controller's job is to hold the
    /// facts, and it goes on holding them while the renderer is restarting.
    ///
    /// # Errors
    ///
    /// A sentence, if the message cannot be rendered or the write fails. **A
    /// failed write closes the connection**: a half-written line would leave
    /// the renderer's framer parsing the next message as the tail of this one,
    /// and reconnecting is the only way back from that.
    pub fn send(&self, message: &Message) -> Result<(), String> {
        let mut line =
            serde_json::to_vec(message).map_err(|e| format!("kiosk: render: {e}"))?;
        line.push(b'\n');

        let mut held = self.connection.lock().unwrap_or_else(|e| e.into_inner());
        let Some(stream) = held.as_mut() else {
            return Ok(());
        };
        // `write_all`, not `write`: a short write is exactly the half-line this
        // doc comment is about, and std will not do it in a loop unasked.
        if let Err(e) = stream.write_all(&line) {
            *held = None;
            return Err(format!("kiosk: send: {e}"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
