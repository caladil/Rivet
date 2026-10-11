//! **Handing an advertisement to whoever is listening, without ever waiting
//! for them.**
//!
//! `ADR-041` §6.3 is the argument; this is the mechanism.
//!
//! # The daemon fans out and does not filter
//!
//! A board in a house hears a great many advertisements per second, and the
//! obvious economy is a filter in the daemon. It is declined, and the reason is
//! `ADR-011` rather than laziness: a filter here is a **second place that has to
//! know what an application wants**, and the application already knows. What
//! the daemon would filter on — an address, a manufacturer id, an RSSI floor —
//! is a policy nobody has ruled, and inventing one would be the operating
//! system deciding which of the maintainer's sensors count.
//!
//! **What the volume costs is bounded here instead.** A consumer that cannot
//! keep up has records dropped, and the drops are **counted and reported**. It
//! is never blocked on: the writer is non-blocking, and a `EWOULDBLOCK` is a
//! full socket buffer rather than a fault. The alternative — a blocking write —
//! would let a slow application stall the process that is reading the radio,
//! which loses advertisements for everybody and reports nothing.
//!
//! # One consumer, and the refusal is the supervisor's
//!
//! `ADR-015` §5.3's grant is a `chown` and a `chown` names one uid, so an
//! endpoint has exactly one consumer and `cmd/rivet-sysd/src/endpoints.rs`
//! refuses a second **by name**. This module does not re-implement that. It
//! accepts whatever reaches the socket, because by the time a connection
//! arrives the kernel has already enforced who may open the path.
//!
//! It does accept more than one *connection* — an application that reconnects
//! after a restart, and the old descriptor not yet reaped — bounded by
//! [`MAX_CONSUMERS`] so a client in a reconnect loop cannot exhaust this
//! process's descriptors.

use std::os::unix::net::UnixStream;

/// The most connections held at once.
///
/// Not a policy about consumers — there is one of those (see the module note) —
/// but a bound on descriptors. A client that reconnects in a tight loop without
/// its old connections being noticed as closed would otherwise take this
/// process's whole descriptor table with it, and a daemon that cannot open its
/// controller because an application misbehaved is the wrong failure.
pub const MAX_CONSUMERS: usize = 8;

/// One connected reader, and what it has missed.
#[derive(Debug)]
pub struct Consumer {
    stream: UnixStream,
    dropped: u64,
    /// Whether the drop count has changed since it was last reported, so a
    /// steady state does not fill the journal with the same number.
    unreported: bool,
}

impl Consumer {
    /// Take a connection. Non-blocking from here on.
    ///
    /// # Errors
    ///
    /// The `set_nonblocking` call, which is the only thing that can fail and is
    /// the one that must not be skipped: a blocking stream here is the stall
    /// this module exists to prevent.
    pub fn new(stream: UnixStream) -> std::io::Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(Self {
            stream,
            dropped: 0,
            unreported: false,
        })
    }

    /// Take the drop count if it has moved since the last time it was taken.
    ///
    /// `None` means nothing new was lost, which is the ordinary case and is why
    /// this is not a plain getter: a daemon reporting "0 dropped" every second
    /// is a daemon whose journal cannot be read.
    pub fn take_report(&mut self) -> Option<u64> {
        self.unreported.then(|| {
            self.unreported = false;
            self.dropped
        })
    }
}

/// What happened to one write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// The whole line went into the socket buffer.
    Sent,
    /// The buffer was full. Counted, not waited for.
    Dropped,
    /// The consumer went away. The connection is closed.
    Gone,
}

/// Every consumer, and the fan-out.
#[derive(Debug, Default)]
pub struct Consumers {
    live: Vec<Consumer>,
    refused: u64,
}

impl Consumers {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            live: Vec::new(),
            refused: 0,
        }
    }

    /// How many connections are open.
    #[must_use]
    pub fn len(&self) -> usize {
        self.live.len()
    }

    /// **Present because [`Self::len`] is, and for no other reason.**
    ///
    /// `clippy::len_without_is_empty` asks for the pair on any type that has a
    /// length, on the ground that a caller reaching for one will reach for the
    /// other. Nothing in this daemon calls it today — the only question asked
    /// of this list is how many are on it, for the journal.
    #[must_use]
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    /// How many connections were refused for want of room.
    #[must_use]
    pub const fn refused(&self) -> u64 {
        self.refused
    }

    /// Add one, unless [`MAX_CONSUMERS`] are already held.
    ///
    /// Returns whether it was taken. A refusal is counted rather than reported
    /// per connection, for the reason [`Consumer::take_report`] gives.
    pub fn push(&mut self, consumer: Consumer) -> bool {
        if self.live.len() >= MAX_CONSUMERS {
            self.refused += 1;
            return false;
        }
        self.live.push(consumer);
        true
    }

    /// Write one line to every consumer, dropping rather than waiting.
    ///
    /// Returns one [`Delivery`] per consumer that was held when the call
    /// started, in order — so a caller can journal what happened without the
    /// borrow outliving the write. Consumers reported [`Delivery::Gone`] are
    /// already removed.
    pub fn broadcast(&mut self, line: &str) -> Vec<Delivery> {
        let mut out = Vec::with_capacity(self.live.len());
        let mut keep = Vec::with_capacity(self.live.len());
        for mut consumer in self.live.drain(..) {
            let delivery = write_line(&mut consumer, line);
            out.push(delivery);
            if delivery != Delivery::Gone {
                keep.push(consumer);
            }
        }
        self.live = keep;
        out
    }

    /// Drop counts worth reporting, with the index of the consumer they belong
    /// to.
    pub fn reports(&mut self) -> Vec<(usize, u64)> {
        self.live
            .iter_mut()
            .enumerate()
            .filter_map(|(at, c)| c.take_report().map(|n| (at, n)))
            .collect()
    }
}

fn write_line(consumer: &mut Consumer, line: &str) -> Delivery {
    write_to(&mut consumer.stream, line, &mut consumer.dropped, &mut consumer.unreported)
}

/// [`write_line`]'s decision, taking the write target as `&mut dyn Write` —
/// the seam a test uses to force a short write on demand, which a real
/// socket cannot be made to produce reliably.
///
/// One `write` and no retry.
///
/// **A write that lands only part of the line closes the consumer, rather
/// than being counted as an ordinary drop.** Those first `n` bytes are
/// already in the socket buffer and cannot be un-sent, so treating this like
/// a full drop and moving on would let the next successful write land right
/// after them: the consumer reads a torn line glued to a whole one, which
/// parses and is wrong — worse than losing a line cleanly, which is what a
/// drop is supposed to mean. Retrying the remainder here would be the
/// blocking wait this module refuses to do, and holding it to prefix the
/// next write would interleave it with whatever that call sends. Closing has
/// neither problem: `ADR-017` restarts the application, and the SDK client
/// reconnects.
///
/// **A write that lands nothing (`WouldBlock`) stays an ordinary drop.**
/// Nothing reached the consumer, so nothing is torn — the buffer was simply
/// full, which is the case this module exists to survive without blocking.
fn write_to(
    writer: &mut dyn std::io::Write,
    line: &str,
    dropped: &mut u64,
    unreported: &mut bool,
) -> Delivery {
    match writer.write(line.as_bytes()) {
        Ok(n) if n == line.len() => Delivery::Sent,
        Ok(_) => Delivery::Gone,
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
            *dropped += 1;
            *unreported = true;
            Delivery::Dropped
        }
        // BrokenPipe, ConnectionReset, and anything else: the consumer is not
        // there. Closing is the whole handling — `ADR-017` restarts the
        // application, and the supervisor re-grants the socket when the
        // provider next binds it.
        Err(_) => Delivery::Gone,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use std::io::Read as _;

    fn pair() -> (Consumer, UnixStream) {
        let (a, b) = UnixStream::pair().expect("a socketpair");
        (Consumer::new(a).expect("non-blocking"), b)
    }

    #[test]
    fn a_line_reaches_the_consumer_intact() {
        let (consumer, mut far) = pair();
        let mut consumers = Consumers::new();
        assert!(consumers.push(consumer));
        assert_eq!(consumers.broadcast("adv 1 x\n"), vec![Delivery::Sent]);
        let mut buf = [0u8; 32];
        let n = far.read(&mut buf).expect("a read");
        assert_eq!(&buf[..n], b"adv 1 x\n");
    }

    /// **The property the whole module exists for.** A consumer that never
    /// reads must not stall the daemon, and whatever happens to it must be
    /// resolved without waiting for it.
    ///
    /// **Which of two outcomes ends the loop depends on where a line lands
    /// relative to the socket's remaining buffer space, and this test
    /// accepts either rather than pinning one.** If the buffer's last bit of
    /// room happens to be exactly enough for a clean `WouldBlock` boundary,
    /// every write past that point is a full drop and the consumer stays
    /// open. If instead the boundary falls in the middle of a line, that
    /// write lands part of it — which `write_to` closes the consumer for,
    /// per its own contract, rather than counting as a further drop. Both are
    /// "not waited for"; only the exact byte where a fixed-size kernel buffer
    /// fills is not something this test controls or should assume.
    #[test]
    fn a_consumer_that_never_reads_is_never_waited_for() {
        let (consumer, _far) = pair();
        let mut consumers = Consumers::new();
        consumers.push(consumer);
        let line = format!("adv {}\n", "x".repeat(200));
        let mut sent = 0u64;
        let mut dropped = 0u64;
        let mut closed = false;
        // Far more than a socket buffer holds. If this blocked, the test would
        // hang rather than fail — which is itself the signal.
        for _ in 0..20_000 {
            match consumers.broadcast(&line).as_slice() {
                [Delivery::Sent] => sent += 1,
                [Delivery::Dropped] => dropped += 1,
                [Delivery::Gone] => {
                    closed = true;
                    break;
                }
                other => panic!("{other:?}"),
            }
        }
        assert!(sent > 0, "nothing was ever delivered");
        assert!(dropped > 0 || closed, "a full buffer neither dropped nor closed the consumer");
        if closed {
            assert!(consumers.is_empty(), "Delivery::Gone was reported but the consumer was kept");
        } else {
            assert_eq!(consumers.reports(), vec![(0, dropped)]);
            // And a steady state says nothing a second time.
            assert_eq!(consumers.reports(), Vec::new());
        }
    }

    #[test]
    fn a_consumer_that_went_away_is_closed_rather_than_retried() {
        let (consumer, far) = pair();
        let mut consumers = Consumers::new();
        consumers.push(consumer);
        drop(far);
        // The first write after the peer closes may succeed into the buffer;
        // the second gets EPIPE. Both outcomes end with the consumer gone.
        for _ in 0..3 {
            if consumers.is_empty() {
                break;
            }
            consumers.broadcast("adv 1 x\n");
        }
        assert!(consumers.is_empty(), "a dead consumer was kept");
    }

    // -----------------------------------------------------------------------
    // `write_to` directly — B1 / I04. A real socket cannot be made to
    // produce a short write on demand, so these use an injected writer to
    // force exactly that, deterministically.
    // -----------------------------------------------------------------------

    /// A writer that accepts at most `cap` bytes of any call and remembers
    /// every byte it actually accepted, across every call — so a test can
    /// force a short write on demand and then check nothing more arrived.
    struct ShortWriter {
        cap: usize,
        accepted: Vec<u8>,
    }

    impl std::io::Write for ShortWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let n = buf.len().min(self.cap);
            self.accepted.extend_from_slice(&buf[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A writer whose buffer is always full.
    struct AlwaysWouldBlock;

    impl std::io::Write for AlwaysWouldBlock {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from(std::io::ErrorKind::WouldBlock))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// **The regression test for B1 / I04.** Before the fix, a write that
    /// landed only part of a line was counted the same as a full drop and the
    /// consumer was kept — so the next successful write landed immediately
    /// after the torn prefix, and a reader saw one garbled record rather than
    /// one dropped one.
    ///
    /// **What happens to the consumer next is
    /// `a_consumer_that_went_away_is_closed_rather_than_retried`'s property,
    /// not this test's**: `Consumers::broadcast` removes any consumer this
    /// reports `Delivery::Gone` for, whatever produced it, so a torn write
    /// and a dead pipe are already handled identically once
    /// `write_to` has classified them the same way. What this test adds is
    /// that classification — and the accepted-bytes log below is the direct
    /// evidence that nothing beyond the torn prefix ever reached the wire, so
    /// there is no second call this or any other test needs to make to prove
    /// silence: a `ShortWriter` that was written to again would show it.
    #[test]
    fn a_partial_write_closes_the_consumer_instead_of_being_counted_as_a_drop() {
        let line = "adv 1 aabbccdd\n";
        let mut sink = ShortWriter { cap: 4, accepted: Vec::new() };
        let mut dropped = 0u64;
        let mut unreported = false;

        let delivery = write_to(&mut sink, line, &mut dropped, &mut unreported);

        assert_eq!(delivery, Delivery::Gone, "a torn write must close the consumer");
        assert_eq!(
            sink.accepted,
            line.as_bytes()[..4],
            "only the torn prefix reached the wire, and nothing was written after it"
        );
        assert_eq!(dropped, 0, "a torn write is not counted as an ordinary drop");
        assert!(!unreported);
    }

    /// A write that lands NOTHING (`WouldBlock`) is still an ordinary,
    /// counted drop: nothing reached the consumer, so nothing is torn — the
    /// exact case `write_to`'s contract carves out from the rule above.
    #[test]
    fn a_write_that_lands_nothing_is_still_counted_as_a_drop() {
        let mut dropped = 0u64;
        let mut unreported = false;

        let delivery = write_to(&mut AlwaysWouldBlock, "adv 1 x\n", &mut dropped, &mut unreported);

        assert_eq!(delivery, Delivery::Dropped);
        assert_eq!(dropped, 1);
        assert!(unreported);
    }

    /// A write that lands the whole line is `Sent`, whatever it is sent to.
    #[test]
    fn a_write_that_lands_the_whole_line_is_sent() {
        let mut sink = ShortWriter { cap: 64, accepted: Vec::new() };
        let mut dropped = 0u64;
        let mut unreported = false;
        let line = "adv 1 x\n";

        let delivery = write_to(&mut sink, line, &mut dropped, &mut unreported);

        assert_eq!(delivery, Delivery::Sent);
        assert_eq!(sink.accepted, line.as_bytes());
        assert_eq!(dropped, 0);
        assert!(!unreported);
    }

    #[test]
    fn more_connections_than_the_bound_are_refused_and_counted() {
        let mut consumers = Consumers::new();
        let mut far = Vec::new();
        for _ in 0..MAX_CONSUMERS {
            let (c, f) = pair();
            far.push(f);
            assert!(consumers.push(c));
        }
        let (c, f) = pair();
        far.push(f);
        assert!(!consumers.push(c));
        assert_eq!(consumers.len(), MAX_CONSUMERS);
        assert_eq!(consumers.refused(), 1);
    }

    #[test]
    fn no_consumer_is_not_an_error() {
        let mut consumers = Consumers::new();
        assert_eq!(consumers.broadcast("adv 1 x\n"), Vec::new());
        assert!(consumers.is_empty());
    }
}
