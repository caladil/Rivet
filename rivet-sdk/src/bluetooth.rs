//! **Reading BLE advertisements**, from the application's side. `ADR-041`.
//!
//! An application does not open a Bluetooth socket. It cannot: an HCI user
//! channel is exclusive, so the platform holds the adapter and hands over what
//! it hears — the same arrangement `rivet-launcher` has for the display
//! (`ADR-015` §6.2, `ADR-036` §3.1). What arrives is a line per advertisement
//! on a socket that was **granted** to this application by name.
//!
//! # Two declarations, and both are needed
//!
//! ```toml
//! # Cargo.toml
//! rivetos-sdk = { version = "...", features = ["bluetooth"] }
//!
//! [package.metadata.rivet]
//! connects = ["rivet-bluetoothd:advertisements"]
//! ```
//!
//! The feature gets the code; the manifest gets the socket. Without the
//! manifest key this module compiles and [`advertisements`] returns
//! [`NotGranted`], which names the key rather than reporting an absent file.
//!
//! **And the service has to be enabled.** `rivet-bluetoothd` ships **disabled**
//! (`ADR-041` §6.5) because it takes an exclusive claim on a radio whose power
//! cost has not been measured. An application declaring `connects` on a machine
//! where it is off is refused **by name** and is not started at all — the
//! supervisor says *"no such service is enabled"* and names the document key.
//! That is a five-second diagnosis and it is written here so it is not a
//! puzzle.
//!
//! # What this gives you, and what it deliberately does not
//!
//! An address, how to read it, which advertising PDU carried it, an RSSI, a
//! monotonic timestamp, and **the advertising bytes verbatim**. Rivet does not
//! decode a vendor's payload — see [`Advertisement::data`] — because a platform
//! that unpacked one company's sensor would have put that company's product
//! into the operating system. [`Advertisement::structures`] walks the
//! length-tagged AD structures for you, which is as far as the specification
//! goes without knowing whose device it is.
//!
//! There is **no scanning control, no connection and no GATT.** Tier 1 is
//! reading advertisements (`.rulings/` R-144), and tiers 2 and 3 are deferred
//! rather than rejected.
//!
//! # The constants and the parser are RE-DECLARED
//!
//! `ADR-036` §5: an application is a foreign program and must not link
//! `rivet-core`. So the line format below is a second implementation of
//! `rivet_core::bluetooth`'s, and `tests/agreement.rs` is what buys the right
//! to have one — it feeds this decoder a line the operating system's own
//! encoder produced.
//!
//! # Half of this module compiles on a development host
//!
//! [`Advertisement::decode`], [`Advertisement::structures`], [`Address`] and
//! every [`LineError`] are pure Rust and are **not** gated. Only
//! [`advertisements`] and [`Advertisements`] are `cfg(unix)`, because the stream
//! is an `AF_UNIX` socket and nothing else about the module is.
//!
//! That split exists because a sensor port's own decoding is the part its author
//! most wants a test for, and until 2026-09-05 the whole crate refused to compile
//! on Windows — so the port target-gated the dependency and lost those tests on
//! the machine it was written on. See the crate note.

#[cfg(unix)]
use std::io::{BufRead as _, BufReader};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::path::PathBuf;

/// The service that provides the stream. Re-declared from
/// `rivet_core::bluetooth::PROVIDER`.
pub const PROVIDER: &str = "rivet-bluetoothd";

/// The endpoint name. Re-declared from `rivet_core::bluetooth::ENDPOINT`.
pub const ENDPOINT: &str = "advertisements";

/// The most one line may be, re-declared from
/// `rivet_core::bluetooth::LINE_MAX_BYTES`.
pub const LINE_MAX_BYTES: usize = 256;

/// The most advertising data a legacy LE PDU carries, re-declared from
/// `rivet_core::bluetooth::MAX_AD_BYTES`.
pub const MAX_AD_BYTES: usize = 31;

/// How a device's address is to be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressKind {
    /// IEEE-assigned and stable for the device's life.
    Public,
    /// Random. May rotate, and for a privacy-enabled device it does — so an
    /// application that identifies a device by address should expect to lose
    /// track of one.
    Random,
}

/// Which advertising PDU carried the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    /// `ADV_IND` — connectable and scannable.
    Connectable,
    /// `ADV_DIRECT_IND` — directed at one initiator.
    Directed,
    /// `ADV_SCAN_IND` — scannable, not connectable.
    Scannable,
    /// `ADV_NONCONN_IND` — a beacon. What a sensor broadcasting a reading uses.
    NonConnectable,
    /// `SCAN_RSP`. Rivet scans **passively** and asks nothing, so this should
    /// not arrive.
    ScanResponse,
}

/// A device address, in the order a person writes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Address(pub [u8; 6]);

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [a, b, c, d, e, g] = self.0;
        write!(f, "{a:02X}:{b:02X}:{c:02X}:{d:02X}:{e:02X}:{g:02X}")
    }
}

/// One advertisement this machine heard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advertisement {
    /// When the platform read it, in milliseconds on `CLOCK_MONOTONIC`.
    ///
    /// **Not a wall clock.** Compare it with your own `CLOCK_MONOTONIC` read to
    /// get an **age**, which is the only thing a freshness decision needs and
    /// the only one that survives the wall clock being unset or stepped
    /// (`ADR-021`).
    pub at_ms: u64,
    /// Who advertised.
    pub address: Address,
    /// How to read [`Advertisement::address`].
    pub kind: AddressKind,
    /// Which advertising PDU carried it.
    pub report: ReportKind,
    /// Received signal strength in dBm, or `None` when the controller said it
    /// was not available. Never a made-up number.
    pub rssi: Option<i8>,
    /// The advertising data, verbatim. **Yours to decode.**
    pub data: Vec<u8>,
}

impl Advertisement {
    /// Walk the length-tagged AD structures, as `(type, value)`.
    ///
    /// A structure whose length runs past the end ends the walk. Nothing here
    /// interprets a type: `0xFF` is *manufacturer specific* and is where a
    /// sensor's reading lives, and what those bytes mean is the device's
    /// documentation and not Rivet's.
    #[must_use]
    pub fn structures(&self) -> Vec<(u8, Vec<u8>)> {
        let data = &self.data;
        let mut out = Vec::new();
        let mut at = 0usize;
        while at < data.len() {
            let length = usize::from(data[at]);
            if length == 0 || at + 1 + length > data.len() {
                break;
            }
            out.push((data[at + 1], data[at + 2..at + 1 + length].to_vec()));
            at += 1 + length;
        }
        out
    }

    /// Read one line.
    ///
    /// # Errors
    ///
    /// [`LineError`], naming which field was wrong.
    pub fn decode(line: &str) -> Result<Self, LineError> {
        let line = line.strip_suffix('\n').unwrap_or(line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.len() > LINE_MAX_BYTES {
            return Err(LineError::TooLong);
        }
        let mut fields = line.split(' ');
        if fields.next() != Some("adv") {
            return Err(LineError::NotAnAdvertisement);
        }
        let at_ms = fields
            .next()
            .and_then(|f| f.parse::<u64>().ok())
            .ok_or(LineError::BadTimestamp)?;
        let address = fields
            .next()
            .and_then(parse_address)
            .ok_or(LineError::BadAddress)?;
        let kind = match fields.next() {
            Some("public") => AddressKind::Public,
            Some("random") => AddressKind::Random,
            _ => return Err(LineError::BadAddressKind),
        };
        let report = match fields.next() {
            Some("ind") => ReportKind::Connectable,
            Some("direct") => ReportKind::Directed,
            Some("scan") => ReportKind::Scannable,
            Some("nonconn") => ReportKind::NonConnectable,
            Some("scan-rsp") => ReportKind::ScanResponse,
            _ => return Err(LineError::BadReportKind),
        };
        let rssi = match fields.next() {
            Some("?") => None,
            Some(text) => Some(text.parse::<i8>().map_err(|_| LineError::BadRssi)?),
            None => return Err(LineError::BadRssi),
        };
        let hex = fields.next().ok_or(LineError::BadPayload)?;
        if fields.next().is_some() {
            return Err(LineError::TrailingField);
        }
        let data = if hex == "-" {
            Vec::new()
        } else {
            decode_hex(hex).ok_or(LineError::BadPayload)?
        };
        if data.len() > MAX_AD_BYTES {
            return Err(LineError::BadPayload);
        }
        Ok(Self {
            at_ms,
            address,
            kind,
            report,
            rssi,
            data,
        })
    }
}

fn parse_address(text: &str) -> Option<Address> {
    let mut out = [0u8; 6];
    if text.split(':').count() != 6 {
        return None;
    }
    for (slot, field) in out.iter_mut().zip(text.split(':')) {
        if field.len() != 2 {
            return None;
        }
        *slot = u8::from_str_radix(field, 16).ok()?;
    }
    Some(Address(out))
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(text.len() / 2);
    for pair in bytes.as_chunks::<2>().0 {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push(((hi << 4) | lo) as u8);
    }
    Some(out)
}

/// Why a line could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineError {
    TooLong,
    NotAnAdvertisement,
    BadTimestamp,
    BadAddress,
    BadAddressKind,
    BadReportKind,
    BadRssi,
    BadPayload,
    TrailingField,
}

impl std::fmt::Display for LineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let said = match self {
            Self::TooLong => "the line is longer than an advertisement can be",
            Self::NotAnAdvertisement => "the line does not begin with `adv`",
            Self::BadTimestamp => "the timestamp is not a count of milliseconds",
            Self::BadAddress => "the address is not six colon-separated hex bytes",
            Self::BadAddressKind => "the address kind is neither `public` nor `random`",
            Self::BadReportKind => {
                "the report kind is not one of ind, direct, scan, nonconn, scan-rsp"
            }
            Self::BadRssi => "the RSSI is neither a signed number of dBm nor `?`",
            Self::BadPayload => "the payload is neither `-` nor an even run of hex digits",
            Self::TrailingField => "there is a field after the payload",
        };
        write!(
            f,
            "{said}. A Rivet writes `adv <ms> <addr> <public|random> <kind> <rssi|?> <hex|->`, \
             so a line that is not that came from something else"
        )
    }
}

impl std::error::Error for LineError {}

/// This application was not granted the advertisement socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotGranted {
    /// What went wrong, in the shape an author can act on.
    pub detail: String,
}

impl std::fmt::Display for NotGranted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.detail)
    }
}

impl std::error::Error for NotGranted {}

// ---------------------------------------------------------------------------
// The socket half — `cfg(unix)` from here down
// ---------------------------------------------------------------------------

/// Connect to the advertisement stream.
///
/// # Errors
///
/// [`NotGranted`] when the manifest does not declare
/// `connects = ["rivet-bluetoothd:advertisements"]`, when the granted socket
/// will not open — which on a machine where `rivet-bluetoothd` is enabled means
/// it has not bound yet, and retrying is the answer (`ADR-017` §3.2: nothing
/// orders a consumer after its provider) — or when the process cannot spare the
/// one extra descriptor [`Stopper`] needs.
#[cfg(unix)]
pub fn advertisements() -> Result<Advertisements, NotGranted> {
    let endpoint = crate::app::endpoint(PROVIDER, ENDPOINT).ok_or_else(|| NotGranted {
        detail: format!(
            "this application was not granted {PROVIDER}:{ENDPOINT}. Declare connects = \
             [\"{PROVIDER}:{ENDPOINT}\"] in [package.metadata.rivet], and make sure \
             {PROVIDER} is enabled on the machine -- it ships disabled"
        ),
    })?;
    let path = endpoint.path.clone();
    let socket = UnixStream::connect(&path).map_err(|e| NotGranted {
        detail: format!(
            "the granted socket at {} could not be opened: {e}. If {PROVIDER} is enabled, it \
             may not have bound yet -- nothing orders a consumer after its provider, so retry",
            path.display()
        ),
    })?;
    Advertisements::over(socket, path).map_err(|e| NotGranted {
        detail: format!(
            "the granted socket opened and could not be duplicated for the stop handle: {e}. \
             One spare descriptor is needed so that a shutdown can wake the reader"
        ),
    })
}

/// **A way to end a read that has not returned** — `ADR-041`, added 2026-09-05.
///
/// [`Advertisements::next`] blocks in `read` until the platform hears something.
/// On a quiet board that is unbounded, and there is exactly one thing an
/// application wants to do about it: **stop**. This is the handle for that, and
/// it is `Send`, `Sync` and [`Clone`], so it goes wherever a shutdown is decided
/// — a signal handler, a `tokio` task, the other end of a channel.
///
/// ```no_run
/// # #[cfg(unix)] fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let stream = rivet_sdk::bluetooth::advertisements()?;
/// let stopper = stream.stopper();
/// std::thread::spawn(move || {
///     for heard in stream {
///         let _ = heard;
///     }
///     // Reached promptly after `stopper.stop()`, on a silent board too.
/// });
/// stopper.stop();
/// # Ok(()) }
/// # #[cfg(not(unix))] fn main() {}
/// ```
///
/// # Why this and not `AsFd`
///
/// The port that reported this said one `AsFd` implementation would remove it,
/// and it would — an author could then call `shutdown(2)` themselves, or `poll`,
/// or set `O_NONBLOCK`. It was rejected for three reasons, recorded here because
/// the cheap answer was the obvious one:
///
/// 1. **A borrowed descriptor is an obligation, not a solution.** `AsFd` hands
///    an author the same problem with a lower-level vocabulary; every adopter
///    then writes the same `shutdown` call, each making the same argument
///    privately. The crate note says exactly this about `unsafe` and
///    [`crate::listen`]: it happens here so it happens once.
/// 2. **Reading from a borrowed descriptor silently loses data.** The stream is
///    a [`BufReader`], so bytes the platform has already delivered are sitting
///    in this crate's buffer. A `read` on the descriptor beside it returns the
///    *next* bytes and skips them, with no error anywhere.
/// 3. **A descriptor number can be recycled.** A `Stopper` holds a real
///    duplicate of the socket, so the number it acts on cannot be closed and
///    reissued to an unrelated file underneath it. A `RawFd` copied out of an
///    `AsFd` and stored for shutdown can be, and the failure — shutting down
///    somebody else's socket — is silent and intermittent.
///
/// If an adopter later needs to *select* over this stream rather than end it,
/// that is a different request and should arrive as a named method with a
/// contract, not as a descriptor with none.
#[cfg(unix)]
#[derive(Debug, Clone)]
pub struct Stopper {
    socket: std::sync::Arc<UnixStream>,
}

#[cfg(unix)]
impl Stopper {
    /// **Wake the reader and end the stream.**
    ///
    /// `shutdown(2)` on the socket. A thread parked in [`Advertisements::next`]
    /// returns immediately with `None`, and every later `next` answers `None`
    /// too — the same ending as the platform closing the socket, which is what
    /// the iterator already documents.
    ///
    /// **Both directions**, so `rivet-bluetoothd` learns that this consumer has
    /// finished rather than discovering it at the next write.
    ///
    /// **Idempotent, and it returns nothing.** The only thing `shutdown(2)` can
    /// report on a socket this type owns is that the connection was already
    /// gone, which is the state the caller was asking for. There is no outcome
    /// here that is worth a `Result` at a call site whose whole job is to stop
    /// caring — and this is not a stub reporting a success it did not achieve
    /// (`AGENTS.md` §3): the postcondition holds in both branches.
    pub fn stop(&self) {
        let _ = self.socket.shutdown(std::net::Shutdown::Both);
    }
}

/// The stream. An [`Iterator`], so a reading loop is a `for`.
///
/// **It blocks.** Each `next` waits for the platform to hear something, and on
/// a quiet board that is a long time. An application that also has to draw or
/// serve should read this on its own thread.
///
/// **And that thread can be woken** — [`Advertisements::stopper`], added
/// 2026-09-05 after the second Bluetooth port found that it could not be. There
/// is still no non-blocking variant and no timeout, because adding one would be
/// guessing at which of `poll`, a channel or a deadline an author wants; ending
/// the stream is the one thing every author needs and the one thing that was
/// impossible.
#[cfg(unix)]
pub struct Advertisements {
    reader: BufReader<UnixStream>,
    stopper: Stopper,
    path: PathBuf,
    line: String,
}

#[cfg(unix)]
impl Advertisements {
    /// Wrap an already-connected socket. The one constructor, so the duplicate
    /// [`Stopper`] holds cannot be forgotten on some path.
    fn over(socket: UnixStream, path: PathBuf) -> std::io::Result<Self> {
        let stopper = Stopper {
            socket: std::sync::Arc::new(socket.try_clone()?),
        };
        Ok(Self {
            reader: BufReader::new(socket),
            stopper,
            path,
            line: String::new(),
        })
    }

    /// Where the socket is, for a diagnostic. **Told rather than assumed** —
    /// the layout is Rivet's (`ADR-015` §5.3).
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// **A handle that can end this stream from another thread.**
    ///
    /// Take it before you move the stream into the thread that reads it. Cheap
    /// — the duplicate descriptor was made when the stream was, so this clones
    /// an `Arc` and nothing else, and it cannot fail.
    #[must_use]
    pub fn stopper(&self) -> Stopper {
        self.stopper.clone()
    }
}

#[cfg(unix)]
impl Iterator for Advertisements {
    /// A line that did not parse is yielded as an error rather than skipped.
    ///
    /// **Skipping would be the wrong shape.** The producer and this decoder are
    /// the two halves of one format, so a line that does not parse means they
    /// disagree — which an application should be able to notice and report,
    /// rather than experience as a sensor that has gone quiet.
    type Item = Result<Advertisement, LineError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            self.line.clear();
            match self.reader.read_line(&mut self.line) {
                // End of file: the platform closed the socket, which happens
                // when `rivet-bluetoothd` restarts. The stream is over; an
                // application that wants to keep reading calls
                // `advertisements()` again.
                Ok(0) => return None,
                Ok(_) => {
                    if self.line.trim().is_empty() {
                        continue;
                    }
                    return Some(Advertisement::decode(&self.line));
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_line_becomes_a_record_with_the_fields_in_the_right_places() {
        let adv = Advertisement::decode("adv 987654 A4:C1:38:11:22:33 public nonconn -63 020106")
            .expect("a valid line");
        assert_eq!(adv.at_ms, 987_654);
        assert_eq!(adv.address.to_string(), "A4:C1:38:11:22:33");
        assert_eq!(adv.kind, AddressKind::Public);
        assert_eq!(adv.report, ReportKind::NonConnectable);
        assert_eq!(adv.rssi, Some(-63));
        assert_eq!(adv.data, vec![0x02, 0x01, 0x06]);
    }

    #[test]
    fn an_unavailable_rssi_is_none_and_an_empty_payload_is_a_dash() {
        let adv = Advertisement::decode("adv 0 00:00:00:00:00:00 random ind ? -").unwrap();
        assert_eq!(adv.rssi, None);
        assert!(adv.data.is_empty());
    }

    #[test]
    fn every_way_a_line_can_be_wrong_names_the_field() {
        for (line, want) in [
            ("hello", LineError::NotAnAdvertisement),
            ("adv x A4:C1:38:11:22:33 random ind -100 dead", LineError::BadTimestamp),
            ("adv 42 nope random ind -100 dead", LineError::BadAddress),
            ("adv 42 A4:C1:38:11:22:33 blue ind -100 dead", LineError::BadAddressKind),
            ("adv 42 A4:C1:38:11:22:33 random hop -100 dead", LineError::BadReportKind),
            ("adv 42 A4:C1:38:11:22:33 random ind loud dead", LineError::BadRssi),
            ("adv 42 A4:C1:38:11:22:33 random ind -100 dea", LineError::BadPayload),
            ("adv 42 A4:C1:38:11:22:33 random ind -100 dead x", LineError::TrailingField),
        ] {
            assert_eq!(Advertisement::decode(line), Err(want), "{line}");
        }
    }

    #[test]
    fn the_ad_walk_splits_structures_and_stops_at_one_that_runs_past_the_end() {
        let adv = Advertisement::decode(
            "adv 1 A4:C1:38:11:22:33 public nonconn -50 02010609ff88ec00035b4c6400",
        )
        .unwrap();
        let structures = adv.structures();
        assert_eq!(structures.len(), 2);
        assert_eq!(structures[0], (0x01, vec![0x06]));
        assert_eq!(structures[1].0, 0xFF);

        let short = Advertisement::decode("adv 1 A4:C1:38:11:22:33 public nonconn -50 05ff01")
            .unwrap();
        assert_eq!(short.structures(), Vec::new());
    }

    /// The iterator is what an application actually writes, so it is exercised
    /// over a real socket rather than only reasoned about.
    #[cfg(unix)]
    #[test]
    fn the_stream_yields_a_record_per_line_and_ends_when_the_platform_closes() {
        use std::io::Write as _;
        let (near, mut far) = UnixStream::pair().expect("a socketpair");
        far.write_all(b"adv 1 A4:C1:38:11:22:33 public nonconn -50 020106\n")
            .expect("a write");
        far.write_all(b"adv 2 A4:C1:38:11:22:33 public nonconn -51 -\n")
            .expect("a write");
        far.write_all(b"not a line\n").expect("a write");
        drop(far);
        let stream = Advertisements::over(near, test_path()).expect("a stream");
        let got: Vec<_> = stream.collect();
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].as_ref().map(|a| a.at_ms), Ok(1));
        assert_eq!(got[1].as_ref().map(|a| a.at_ms), Ok(2));
        // The bad line is REPORTED, not skipped. See `Iterator::Item`.
        assert_eq!(got[2], Err(LineError::NotAnAdvertisement));
    }

    #[cfg(unix)]
    fn test_path() -> PathBuf {
        PathBuf::from("/run/rivet/listen/rivet-bluetoothd/advertisements")
    }

    // -----------------------------------------------------------------------
    // Waking a blocked reader — `Stopper`
    // -----------------------------------------------------------------------
    //
    // **A SILENT stream, in every one of these.** A test against a busy stream
    // proves nothing about this defect: the reported symptom is that a scanner
    // notices a shutdown only when the next advertisement arrives, which on a
    // busy board is about 55 ms and on a silent one is never. So the far end is
    // held open and written to exactly nothing.
    //
    // Each waits with a timeout rather than a `join`, so a regression FAILS
    // rather than hanging the suite for ever.

    /// **The defect, directly.** A thread parked in `next` on a stream that
    /// will never deliver anything, ended from another thread.
    #[cfg(unix)]
    #[test]
    fn a_reader_blocked_on_a_silent_stream_is_woken_by_the_stopper() {
        let (near, far) = UnixStream::pair().expect("a socketpair");
        let stream = Advertisements::over(near, test_path()).expect("a stream");
        let stopper = stream.stopper();

        let (tx, rx) = std::sync::mpsc::channel();
        let reading = std::thread::spawn(move || {
            let mut stream = stream;
            // Blocks here. Nothing is ever written to `far`.
            let first = stream.next();
            tx.send(first.is_none()).expect("the test is still there");
        });

        // Long enough that the thread is certainly inside `read`, and short
        // enough that this test is cheap. The assertion does not depend on it:
        // if `stop` were called before the read began, `shutdown` would still
        // have set the flag the read checks.
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert!(
            rx.try_recv().is_err(),
            "the reader returned before anything stopped it, so this test is not \
             measuring what it claims to"
        );

        stopper.stop();

        let ended = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap_or_else(|e| {
                panic!(
                    "the reader was still blocked five seconds after stop(): {e}. \
                     This is the defect the Stopper exists to fix"
                )
            });
        assert!(ended, "a stopped stream ends, so `next` answers None");
        reading.join().expect("the reading thread");
        drop(far);
    }

    /// Stopping before anybody reads works too, and a second `stop` is not an
    /// error. The shutdown path of a real application calls this from wherever
    /// it noticed, which may be before the reader has started.
    ///
    /// **Read on a thread with a timeout, like the one above**, and for the same
    /// reason: on a silent stream a broken `stop` does not make this assertion
    /// false, it makes it never return.
    #[cfg(unix)]
    #[test]
    fn stopping_is_idempotent_and_works_before_the_first_read() {
        let (near, far) = UnixStream::pair().expect("a socketpair");
        let stream = Advertisements::over(near, test_path()).expect("a stream");
        let stopper = stream.stopper();
        stopper.stop();
        stopper.stop();
        stopper.clone().stop();

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut stream = stream;
            let ended = stream.next().is_none() && stream.next().is_none();
            let _ = tx.send(ended);
        });
        let ended = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap_or_else(|e| panic!("a stream stopped before its first read still blocks: {e}"));
        assert!(ended, "a stopped stream has ended, and it stays ended");
        drop(far);
    }

    /// **The handle outlives nothing it should not, and it is `Send`.** The
    /// reported port puts the read on its own OS thread and decides shutdown
    /// somewhere else, so the handle has to cross a thread boundary and be
    /// clonable to more than one shutdown path.
    #[cfg(unix)]
    #[test]
    fn the_stopper_is_send_sync_and_clone() {
        const fn assert_send_sync<T: Send + Sync + Clone>() {}
        assert_send_sync::<Stopper>();
    }

    /// A stopper taken and the stream dropped: the duplicate keeps the socket
    /// alive, so `stop` is still safe to call and cannot act on a recycled
    /// descriptor. This is argument 3 in `Stopper`'s own note, asserted.
    #[cfg(unix)]
    #[test]
    fn a_stopper_that_outlives_its_stream_is_still_safe_to_call() {
        let (near, far) = UnixStream::pair().expect("a socketpair");
        let stream = Advertisements::over(near, test_path()).expect("a stream");
        let stopper = stream.stopper();
        drop(stream);
        stopper.stop();
        drop(far);
    }

    #[cfg(unix)]
    #[test]
    fn an_ungranted_application_is_told_which_manifest_key_to_write() {
        // `RIVET_CONNECT` is absent in the test harness, which is exactly the
        // state of an application whose manifest omits the key.
        let Err(err) = advertisements() else {
            panic!("nothing granted this test a socket and one was returned anyway")
        };
        assert!(err.to_string().contains("connects"), "{err}");
        assert!(err.to_string().contains("rivet-bluetoothd"), "{err}");
    }

    // -----------------------------------------------------------------------
    // The agreement with the platform — `ADR-036` §8.3
    // -----------------------------------------------------------------------
    //
    // **These belong in `tests/agreement.rs` with every other one and they
    // cannot be there**, for a reason that is cargo's rather than a
    // preference: an integration test links this crate with whatever features
    // the build enabled, and under `cargo test --workspace` that is none — so
    // an agreement test for an optional module would not compile. A unit test
    // does, because `lib.rs` gates this module on `any(feature = "bluetooth",
    // test)` and `rivetos-core` is a dev-dependency. `tests/agreement.rs`
    // carries a note pointing here.

    /// **The two names an application writes into its manifest** — `ADR-041`.
    ///
    /// A drift here does not produce a wrong reading; it produces an
    /// application the supervisor refuses to start, naming an endpoint nobody
    /// provides.
    #[test]
    fn the_endpoint_is_spelled_the_same_on_both_sides() {
        assert_eq!(PROVIDER, rivet_core::bluetooth::PROVIDER);
        assert_eq!(ENDPOINT, rivet_core::bluetooth::ENDPOINT);
        assert_eq!(LINE_MAX_BYTES, rivet_core::bluetooth::LINE_MAX_BYTES);
        assert_eq!(MAX_AD_BYTES, rivet_core::bluetooth::MAX_AD_BYTES);
    }

    /// **The central one**, and the test that buys the right to re-declare the
    /// parser.
    ///
    /// A line the OPERATING SYSTEM encodes must decode here to the same values.
    /// The record is built by `rivet_core`'s own HCI parser from an event, so
    /// this exercises the whole path a byte takes: controller event, platform
    /// record, wire line, application record. A round trip of this decoder
    /// against its own encoder would prove the two halves agree and nothing
    /// about the format (`AGENTS.md` §4) — which is why there is no encoder
    /// here at all.
    #[test]
    fn a_line_the_platform_writes_decodes_here_to_the_same_values() {
        // The shape of a sensor beacon, hand-composed — labelled as such,
        // because no board has yet had a radio to capture one with.
        let event: Vec<u8> = vec![
            rivet_core::bluetooth::HCI_SUBEV_LE_ADVERTISING_REPORT,
            0x01, // one report
            0x03, // ADV_NONCONN_IND
            0x00, // public
            0x33, 0x22, 0x11, 0x38, 0xC1, 0xA4, // BD_ADDR, little-endian
            0x0D, // 13 bytes of advertising data
            0x02, 0x01, 0x06, 0x09, 0xFF, 0x88, 0xEC, 0x00, 0x03, 0x5B, 0x4C, 0x64, 0x00,
            0xC1, // RSSI, -63
        ];
        let mut platform = rivet_core::bluetooth::parse_advertising_report(&event, 987_654)
            .unwrap_or_else(|e| panic!("the platform could not read its own vector: {e}"));
        assert_eq!(platform.len(), 1);
        let platform = platform.remove(0);
        let line = platform.encode();

        let sdk = Advertisement::decode(&line)
            .unwrap_or_else(|e| panic!("the SDK could not read {line:?}: {e}"));

        assert_eq!(sdk.at_ms, platform.at_ms);
        assert_eq!(sdk.address.0, platform.address.0);
        assert_eq!(sdk.rssi, platform.rssi);
        assert_eq!(sdk.data, platform.data);
        assert_eq!(sdk.kind, AddressKind::Public);
        assert_eq!(sdk.report, ReportKind::NonConnectable);
        assert_eq!(sdk.address.to_string(), platform.address.to_string());
        // And the AD walk, which each side implements separately.
        assert_eq!(
            sdk.structures(),
            rivet_core::bluetooth::ad_structures(&platform.data)
        );
    }

    /// Every address kind and every report kind, not just the pair the test
    /// above happened to use. A word spelled differently on one side is a line
    /// the application refuses for a device it can see.
    #[test]
    fn every_word_in_the_wire_vocabulary_survives_the_crossing() {
        use rivet_core::bluetooth as core;
        for kind in [core::AddressKind::Public, core::AddressKind::Random] {
            for report in [
                core::ReportKind::Connectable,
                core::ReportKind::Directed,
                core::ReportKind::Scannable,
                core::ReportKind::NonConnectable,
                core::ReportKind::ScanResponse,
            ] {
                for rssi in [Some(-127i8), Some(0), Some(20), None] {
                    let adv = core::Advertisement {
                        at_ms: 7,
                        address: core::Address([0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01]),
                        kind,
                        report,
                        rssi,
                        data: vec![0x02, 0x01, 0x06],
                    };
                    let line = adv.encode();
                    let sdk = Advertisement::decode(&line)
                        .unwrap_or_else(|e| panic!("the SDK refused {line:?}: {e}"));
                    assert_eq!(sdk.rssi, adv.rssi, "{line}");
                    assert_eq!(sdk.address.0, adv.address.0, "{line}");
                    assert_eq!(sdk.data, adv.data, "{line}");
                    // The words themselves, compared as text, so a renaming on
                    // one side is caught rather than mapped through two enums
                    // that happen to have the same variant order.
                    assert!(line.contains(kind.as_str()), "{line}");
                    assert!(line.contains(report.as_str()), "{line}");
                }
            }
        }
    }
}
