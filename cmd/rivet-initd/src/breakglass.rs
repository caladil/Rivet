//! **Break-glass over the serial console** — `ADR-035` §8A.1.2, ruled
//! 2026-08-29 and built 2026-08-30.
//!
//! # What this is for, and the one case it exists to serve
//!
//! `ADR-035` §8A.1 is Tier 1 of break-glass: *"the same MCP the network exposes
//! can be spoken over that serial line, with a physical cable and no network
//! stack, no DHCP, no TLS and no identity — because physical presence is the
//! credential"*.
//!
//! §8A.1.2 then ruled **pid 1 hosts it**, and the reason decides the shape of
//! everything here: `rivet-mcpd` is supervised, so *an MCP server the
//! supervisor starts cannot answer when the supervisor is what failed* — which
//! is the single scenario Tier 1 is for. A third small binary loses to the same
//! test one step later, because *started by pid 1 is not the same as surviving
//! anything pid 1 survives*.
//!
//! **And the stakes are higher than they read.** Tier 1 is not a third safety
//! net with two comfortable ones behind it: what remains without it is Tier 2
//! (the FAT boot partition) and Tier 3 (rewriting the card), and **both need
//! hands on the hardware.** For a board that is not in the room, Tier 1 is the
//! last thing that works.
//!
//! # The three constraints, and they are not style
//!
//! §8A.1.2: *"pid 1's failure mode is the machine, so what goes in it must be
//! small, must not allocate unboundedly, and must not be able to block. That
//! constrains the implementation rather than the decision — a line-oriented
//! reader over the console with a fixed buffer, refusing anything it cannot
//! serve."*
//!
//! Each is answered by something visible here rather than by intent:
//!
//! * **Cannot block.** [`Console::poll`] asks `poll(2)` with a zero timeout
//!   before it reads, so a tick with nothing waiting costs one syscall and
//!   returns. Pid 1's loop is never held by a console nobody is typing at.
//! * **Cannot allocate unboundedly.** The buffer is `[u8; LINE_MAX]`, fixed at
//!   compile time. A line longer than it is **not** grown into — it is
//!   discarded to the next newline and refused, which is [`Console::overlong`].
//! * **Small.** This module frames lines. It does not parse JSON, dispatch an
//!   operation, or know what a subject is; those live where they already live.
//!
//! # Why a line, and why the reader is a state machine
//!
//! A serial console delivers whatever bytes have arrived, which is not a
//! message. Somebody typing gets one byte per keystroke; a paste arrives in
//! chunks that split wherever the driver felt like it. So the framing is
//! **newline-delimited**, and the reader holds a partial line across ticks —
//! which is the whole reason this is a struct and not a function.
//!
//! `\r` is stripped, because a terminal on the other end of a serial cable
//! sends `\r\n` and a person cannot see the difference.
//!
//! # What it does NOT do, deliberately
//!
//! **It does not echo.** A console is not a shell here: `cmd/rivet` is the
//! login shell and this is a protocol endpoint that
//! happens to share the line. Echoing would corrupt the reply stream of a
//! client that is a program rather than a person.

use crate::kernel::Kernel;

/// The longest line this will assemble, in bytes.
///
/// **A fixed array and not a `Vec`**, which is §8A.1.2's *must not allocate
/// unboundedly* taken literally: the memory is in the struct, so there is no
/// input that makes pid 1 ask for more.
///
/// 2 KiB because the largest thing break-glass has to carry is an `apply edit`
/// whose `--expect` and `--replace` are lines of a configuration document, and
/// a document line that does not fit in 2 KiB is one no operator typed. A
/// bigger buffer would cost pid 1 resident memory on every board for a case
/// that does not arise; a smaller one would refuse a legitimate repair.
pub const LINE_MAX: usize = 2048;

/// How much is read from the console in one go.
///
/// Sized to the buffer, so a full line that arrives in one write is taken in
/// one read rather than in `LINE_MAX` single-byte reads.
const CHUNK: usize = LINE_MAX;

/// The console reader — a partial line, held across ticks.
pub struct Console {
    /// The line being assembled. Never grows.
    buf: [u8; LINE_MAX],
    /// How much of `buf` is in use.
    used: usize,
    /// **The current line has already overflowed**, so the rest of it up to
    /// the next newline is discarded.
    overlong: bool,
    /// **An overlong line has been discarded and not yet reported.** Separate
    /// from `overlong` because the two are different moments: one says *keep
    /// dropping*, this one says *there is a refusal owed*. An operator whose
    /// paste vanished must be told, since the alternative is a command that
    /// looks ignored.
    pending: bool,
    /// Whether the console has been reported unreadable. Latched, so a
    /// console that fails on every tick says so once rather than filling the
    /// journal with the same line forever.
    complained: bool,
}

/// What one poll produced.
#[derive(Debug, PartialEq, Eq)]
pub enum Line {
    /// A complete line, `\r` stripped. May be empty — a bare newline is a
    /// line, and refusing it is the dispatcher's business rather than this
    /// one's.
    Ready(String),
    /// A line longer than [`LINE_MAX`] arrived and was discarded.
    TooLong,
}

impl Default for Console {
    fn default() -> Self {
        Self::new()
    }
}

impl Console {
    /// A reader with nothing buffered.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buf: [0; LINE_MAX],
            used: 0,
            overlong: false,
            pending: false,
            complained: false,
        }
    }

    /// **One non-blocking look at the console.** Returns at most one line.
    ///
    /// At most one, and not every line that arrived, because pid 1's tick has
    /// other work: draining a paste of a hundred commands before reaping a
    /// dead supervisor would be this module deciding it matters more than the
    /// loop it lives in. The rest stays buffered and comes out on the next
    /// tick.
    ///
    /// A read error is reported **once** and then swallowed. A console that
    /// cannot be read is not a reason for pid 1 to stop: the machine still has
    /// a supervisor to keep alive, and break-glass being unavailable is
    /// exactly the situation the other two tiers exist for.
    pub fn poll(&mut self, k: &mut dyn Kernel) -> Option<Line> {
        // A line may already be complete from a previous read that carried
        // more than one, so look before reading.
        if let Some(line) = self.take_line() {
            return Some(line);
        }

        let mut chunk = [0u8; CHUNK];
        let got = match k.read_console(&mut chunk) {
            Ok(0) => return None,
            Ok(n) => n.min(CHUNK),
            Err(e) => {
                if !self.complained {
                    self.complained = true;
                    k.say(&format!(
                        "rivet-initd: the console cannot be read ({e}), so break-glass over \
                         serial is unavailable on this boot. ADR-035 §8A's other tiers are the \
                         boot partition and rewriting the card"
                    ));
                }
                return None;
            }
        };

        self.absorb(&chunk[..got]);
        self.take_line()
    }

    /// Feed bytes in, dropping anything past the line limit.
    ///
    /// **The overflow is latched rather than inferred.** An earlier draft
    /// cleared the buffer when a newline arrived with no room for it and left
    /// `take_line` looking for a terminator that was no longer there, so the
    /// refusal never surfaced and the operator saw silence. `pending` is the
    /// fix and it is why this is two flags rather than one.
    fn absorb(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if byte == b'\r' {
                continue;
            }
            if byte == b'\n' {
                if self.overlong {
                    // The line is finished and unusable. Drop it whole and
                    // remember to say so.
                    self.overlong = false;
                    self.pending = true;
                    self.used = 0;
                } else if self.used < LINE_MAX {
                    self.buf[self.used] = byte;
                    self.used += 1;
                } else {
                    // Exactly full, and the terminator has nowhere to go. That
                    // is an overlong line too.
                    self.pending = true;
                    self.used = 0;
                }
                continue;
            }
            if self.used == LINE_MAX {
                self.overlong = true;
                continue;
            }
            self.buf[self.used] = byte;
            self.used += 1;
        }
    }

    /// Split off a complete line if there is one, or the refusal if a line was
    /// dropped for length.
    ///
    /// **The refusal comes first**, because it is already complete: the bytes
    /// it refers to are gone and nothing later can change what it says.
    fn take_line(&mut self) -> Option<Line> {
        if self.pending {
            self.pending = false;
            return Some(Line::TooLong);
        }
        let end = self.buf[..self.used].iter().position(|&b| b == b'\n')?;
        let line = String::from_utf8_lossy(&self.buf[..end]).into_owned();
        self.buf.copy_within(end + 1..self.used, 0);
        self.used -= end + 1;
        Some(Line::Ready(line))
    }
}

#[cfg(test)]
#[path = "breakglass_tests.rs"]
mod tests;

/// **Serve one line, and say whether the machine was asked to stop.**
///
/// The whole dispatcher, and it is short on purpose: `ADR-035` §8A.1.2 puts
/// this in the process whose failure mode is the machine, so everything it can
/// hand to code that already exists, it does.
///
/// # Words, not JSON — and that is a departure worth naming
///
/// `ADR-035` §8A.1 says *"the same MCP the network exposes can be spoken over
/// that serial line"*. This serves the **word form** instead: the same words
/// that follow `--` in `cargo rivet run`, which is what
/// `rivet_core::operation::perform` takes natively — MCP's arguments object is
/// a wrapper the control plane adds and then unwraps.
///
/// Two reasons, and the second is the one that decided it:
///
/// * **It is the same catalogue and the same verbs**, so §8A.1's *"nothing new
///   has to be learned or maintained"* holds. Nobody learns a second
///   vocabulary; they type the words they would have typed.
/// * **Break-glass is used by a person, at the worst moment.** Hand-writing
///   `{"jsonrpc":"2.0","id":1,"method":"tools/call",…}` on a serial console at
///   2am, with no shell history and no editor, is a worse interface than the
///   failure it is meant to recover from.
///
/// A JSON line is **refused with a sentence pointing at the word form** rather
/// than parsed, because a half-supported protocol is `ADR-005` §5.4.1's partial
/// promise: accepting some MCP would invite a client to assume all of it.
pub fn serve(
    line: &str,
    k: &mut dyn Kernel,
    slot: rivet_core::state::Slot,
) -> Option<crate::kernel::RebootHow> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    if line.starts_with('{') {
        k.say(
            "break-glass takes WORDS, not JSON: the same words that follow `--` in `cargo rivet \
             run`, one command per line. `get desired-state` and `apply restart confirm` are \
             two. MCP's arguments object is what the control plane wraps these in, and this \
             console does not speak it (ADR-035 §8A.1.2)",
        );
        return None;
    }

    let words: Vec<&str> = line.split_whitespace().collect();
    let verb = match rivet_core::operation::Verb::parse(words[0]) {
        Ok(verb) => verb,
        Err(e) => {
            say_refusal(k, &e);
            return None;
        }
    };

    let mut board = crate::breakglass_board::BreakGlassBoard::new(k, slot);
    let answered = rivet_core::operation::perform(verb, &words[1..], &mut board);
    let pending = board.pending;

    match answered {
        Ok(value) => k.say(&rivetos_render::json(&value)),
        Err(e) => say_refusal(k, &e),
    }
    pending
}

/// A refusal, in the same three fields the control plane reports.
///
/// **Built here rather than reused**, because `cmd/rivet`'s refusal type is in
/// a binary crate and pid 1 cannot reach it. Three fields is small enough that
/// a shared type would cost more than it saves — and if a fourth ever appears,
/// that judgement is what has to be revisited.
fn say_refusal(k: &mut dyn Kernel, e: &rivet_core::operation::OperationError) {
    let mut fields = vec![
        ("error", rivet_core::value::Value::str(e.code().as_str())),
        ("message", rivet_core::value::Value::str(e.message())),
    ];
    if let Some(subject) = e.subject() {
        fields.insert(1, ("subject", rivet_core::value::Value::str(subject)));
    }
    k.say(&rivetos_render::json(&rivet_core::value::Value::object(
        fields,
    )));
}
