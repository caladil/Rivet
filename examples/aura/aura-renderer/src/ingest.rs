//! **Where cards arrive**, and the only such place.
//!
//! Four listening sockets, one per slot, all handed over by the supervisor —
//! this process binds nothing (`SPEC-03` §3.1). A producer connects to the one
//! it was granted and writes cards until it stops.
//!
//! # There is no way to tell a shipped producer from a third party here
//!
//! `SPEC-04` §4.2, and it is the reason this module is as short as it is.
//! Weather ships in the box; it reaches this code through the same `accept`,
//! the same [`aura_card::Framer`] and the same refusal path as an addon written
//! by somebody Aura has never heard of. **There is no branch on who is
//! calling** — the only thing this module knows about a peer is which socket it
//! came in on, and that is what decides its slot.

use std::io::Read as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::Instant;

use aura_card::{Framer, Slot};

use crate::store::Store;

/// One slot's socket, and the one producer currently on it.
struct Endpoint {
    slot: Slot,
    listener: UnixListener,
    /// At most one. The supervisor grants an endpoint to exactly one consumer
    /// (`SPEC-03` §3.4), so a second connection is a producer that reconnected
    /// — take the new one and drop the old, which is what a restarted producer
    /// needs and what a stuck one deserves.
    peer: Option<(UnixStream, Framer)>,
}

/// Every slot socket this process was handed.
pub struct Ingest {
    endpoints: Vec<Endpoint>,
}

impl Ingest {
    /// Take the listening sockets the supervisor passed.
    ///
    /// A slot whose socket was not handed over is simply absent: that is what a
    /// build with fewer `provides` looks like, and it is not a fault.
    ///
    /// # Errors
    ///
    /// A sentence naming the slot, if a handed descriptor cannot be put into
    /// non-blocking mode.
    pub fn take() -> Result<Self, String> {
        let mut endpoints = Vec::new();
        for slot in Slot::ALL {
            // The endpoint's manifest name IS the slot's wire name, so this
            // lookup and an addon's `connects = ["aura-renderer:secondary"]`
            // cannot drift apart.
            let Some(socket) = rivet_sdk::listen::socket(slot.as_str()) else {
                continue;
            };
            // `None` means this process already owns that descriptor. It cannot
            // happen here — each slot is looked up once — and it is a refusal
            // rather than a skip, because a slot that silently stopped
            // accepting is an addon that connects to nothing with nobody
            // saying why.
            let listener = socket.into_unix_listener().ok_or_else(|| {
                format!("{}: its descriptor was already taken in this process", slot.as_str())
            })?;
            // **Non-blocking, because this process is also a renderer.** A
            // blocking `accept` would stop the frame loop, and the frame loop
            // is the thing holding the screen.
            listener
                .set_nonblocking(true)
                .map_err(|e| format!("{}: {e}", slot.as_str()))?;
            endpoints.push(Endpoint {
                slot,
                listener,
                peer: None,
            });
        }
        Ok(Self { endpoints })
    }

    /// Which slots have a socket at all.
    pub fn slots(&self) -> impl Iterator<Item = Slot> + '_ {
        self.endpoints.iter().map(|e| e.slot)
    }

    /// Accept, read, decode, apply. Returns anything worth a journal line.
    ///
    /// Non-blocking throughout: called once per frame, does whatever is ready,
    /// and returns.
    pub fn poll(&mut self, store: &mut Store, now: Instant) -> Vec<String> {
        let mut notes = Vec::new();
        for endpoint in &mut self.endpoints {
            if let Ok((stream, _)) = endpoint.listener.accept()
                && stream.set_nonblocking(true).is_ok()
            {
                notes.push(format!("{}: a producer connected", endpoint.slot.as_str()));
                endpoint.peer = Some((stream, Framer::new()));
            }
            let Some((stream, framer)) = &mut endpoint.peer else {
                continue;
            };

            let mut buffer = [0u8; 4096];
            let mut hung_up = false;
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) => {
                        hung_up = true;
                        break;
                    }
                    Ok(n) => {
                        if framer.push(&buffer[..n]).is_err() {
                            hung_up = true;
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        notes.push(format!("{}: read failed: {e}", endpoint.slot.as_str()));
                        hung_up = true;
                        break;
                    }
                }
            }

            while let Some(message) = framer.next_message() {
                match message {
                    Ok(message) => {
                        if let Err(why) = store.apply(endpoint.slot, &message, now) {
                            notes.push(format!("{}: {why}", endpoint.slot.as_str()));
                        }
                    }
                    Err(fault) => {
                        notes.push(format!("{}: refused a card: {fault}", endpoint.slot.as_str()));
                        // **One bad card is not a bad producer.** Only a
                        // framing failure ends the conversation, because only
                        // then is there no position to resume from
                        // (`Fault::is_fatal`).
                        if fault.is_fatal() {
                            notes.push(format!(
                                "{}: the stream cannot be resumed, so the connection is closed",
                                endpoint.slot.as_str()
                            ));
                            hung_up = true;
                            break;
                        }
                    }
                }
            }

            if hung_up {
                endpoint.peer = None;
                notes.push(format!(
                    "{}: the producer is gone; its card stands until its ttl expires",
                    endpoint.slot.as_str()
                ));
            }
        }
        notes
    }
}
