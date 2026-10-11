//! **The aggregator** — `internal/state/state.go`.
//!
//! Every fact the display shows, held in one place, written by whichever thread
//! discovered it and read once a second by the tick loop. The Go program uses
//! an `RWMutex` and a `Snapshot` that copies; this is a `Mutex` and a
//! [`State::snapshot`] that clones, for the same reason: **the tick loop must
//! never hold the lock while it writes to a socket**, because a renderer that
//! has stopped reading would then stop the weather poll as well.
//!
//! # A `Mutex` and not an `RwLock`
//!
//! Go's is an `RWMutex` because Go's `Snapshot` is the only reader and the
//! setters are frequent. Here the shape is the opposite — one reader once a
//! second, a handful of writers even less often — so a read-write lock buys
//! nothing and costs a more delicate poisoning story. Contention on this lock
//! is not a thing that can happen at these rates.

use crate::wire;

/// The latest value of every widget.
#[derive(Debug, Default)]
pub struct State {
    inner: std::sync::Mutex<Held>,
}

/// What is actually held. Separate from [`State`] so the lock guards one thing.
#[derive(Debug, Default)]
struct Held {
    clock: wire::Clock,
    /// `None` until real data arrives, and omitted from a snapshot while it is.
    weather: Option<wire::Weather>,
    /// `None` until a TempMon reading arrives, and **`None` again the moment
    /// one stops arriving** — see [`State::clear_indoor`].
    indoor: Option<wire::Indoor>,
    /// The corner reading card as its source last described it, or `None`
    /// when the card is switched off, no addon of its class is installed, or
    /// nothing has been polled yet.
    ///
    /// **A finished card here, where its neighbours hold raw readings.** That
    /// is the shape the move to an addon changed and it is worth saying why:
    /// the staleness rule belongs to the source, so the host asks it once a
    /// second rather than holding a reading and deciding for itself. What
    /// comes back is already the answer for this instant — see
    /// `crate::addon`'s note on [`crate::addon::POLL_INTERVAL`].
    reading: Option<wire::ReadingData>,
}

impl State {
    /// A state holding nothing, which is the cold start.
    pub fn new() -> Self {
        Self::default()
    }

    /// A `state` message ready to send.
    ///
    /// Clones rather than borrows: see the module note on not holding the lock
    /// across a write.
    ///
    /// # Panics
    ///
    /// Never in practice. A poisoned lock means a thread panicked while holding
    /// it, and every writer below does nothing but assign — there is no code
    /// inside the guard that can unwind.
    pub fn snapshot(&self) -> wire::Message {
        let held = self.inner.lock().expect("no code inside this lock can panic");
        wire::Message {
            clock: Some(held.clock.clone()),
            weather: held.weather.clone(),
            indoor: held.indoor.clone(),
            reading: held.reading.clone(),
            ..wire::Message::state()
        }
    }

    /// Update the clock fields.
    ///
    /// # Panics
    ///
    /// As [`State::snapshot`].
    pub fn set_clock(&self, clock: wire::Clock) {
        let mut held = self.inner.lock().expect("no code inside this lock can panic");
        held.clock = clock;
    }

    /// Update the outdoor conditions.
    ///
    /// # Panics
    ///
    /// As [`State::snapshot`].
    pub fn set_weather(&self, weather: wire::Weather) {
        let mut held = self.inner.lock().expect("no code inside this lock can panic");
        held.weather = Some(weather);
    }

    /// Update the indoor reading.
    ///
    /// # Panics
    ///
    /// As [`State::snapshot`].
    pub fn set_indoor(&self, indoor: wire::Indoor) {
        let mut held = self.inner.lock().expect("no code inside this lock can panic");
        held.indoor = Some(indoor);
    }

    /// Replace the corner reading card.
    ///
    /// **A rendered card and not a raw reading**, which is the one shape the
    /// addon changed — see [`Held::reading`]. The poller writes this once a
    /// second, so a reading that goes stale between fetches says so without
    /// anything new having arrived.
    ///
    /// # Panics
    ///
    /// As [`State::snapshot`].
    pub fn set_reading(&self, reading: wire::ReadingData) {
        let mut held = self.inner.lock().expect("no code inside this lock can panic");
        held.reading = Some(reading);
    }

    /// Draw no corner reading card at all.
    ///
    /// **The only thing that removes the card**, and it means exactly one
    /// thing: a person switched it off, or there is no reading source
    /// installed to switch on. Every other state — nothing yet, stale, a
    /// refusal that will not fix itself — is a card with a drawing, because a
    /// card that vanishes when a sensor drops out is indistinguishable from
    /// one nobody enabled.
    ///
    /// # Panics
    ///
    /// As [`State::snapshot`].
    pub fn clear_reading(&self) {
        let mut held = self.inner.lock().expect("no code inside this lock can panic");
        held.reading = None;
    }

    /// Drop the indoor reading.
    ///
    /// **A sensor that has gone away stops being drawn rather than freezing at
    /// its last value.** This is not the same as `online: false`, which is
    /// TempMon saying *this sensor is stale*; this is Aura saying *I could not
    /// reach TempMon at all*, and both must remove the column.
    ///
    /// # Panics
    ///
    /// As [`State::snapshot`].
    pub fn clear_indoor(&self) {
        let mut held = self.inner.lock().expect("no code inside this lock can panic");
        held.indoor = None;
    }
}

#[cfg(test)]
mod tests;
