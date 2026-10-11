//! **A BLE advertisement, and the line an application receives it as.**
//!
//! `ADR-041`, ruled 2026-09-05 (`.rulings/` R-144). Tier 1 and only tier 1:
//! **reading advertisements.** No pairing, no GATT, no Classic. Nothing here
//! knows what a connection is.
//!
//! # Why a codec is in the core
//!
//! Two reasons, and they are the same two [`crate::input`] and
//! [`crate::journal`] give.
//!
//! **Both ends parse from here.** `cmd/rivet-bluetoothd` writes these lines and
//! `rivet_sdk::bluetooth` reads them; two spellings of one vocabulary is two
//! things that come to disagree (`ADR-011`). The SDK re-declares rather than
//! imports — an application is a foreign program (`ADR-036` §5) — and
//! `rivet-sdk/tests/agreement.rs` is what buys the right to.
//!
//! **And it is pure.** Turning 34 bytes of an HCI event into a record needs no
//! socket, no adapter and no radio, so it belongs where `cargo test -p
//! rivetos-core` can reach it with none of those. `ADR-014` §3's rule is that
//! the core names no platform type, and it does not: the socket that carries
//! these bytes is `cmd/rivet-bluetoothd`'s and appears nowhere below.
//!
//! # There is no vendor in this file, deliberately
//!
//! The capability was ruled for the maintainer's Govee sensors, and **nothing
//! here decodes a Govee payload.** An advertisement is an address, an RSSI, a
//! timestamp and a bag of bytes; a platform that unpacked one vendor's
//! manufacturer data would have put that vendor's product into the operating
//! system, and the next sensor would need a second one. The application
//! decodes; Rivet delivers.
//!
//! What IS here is the Bluetooth Core specification's own vocabulary — an
//! address type, a report type, a length-tagged AD structure — because those
//! are facts about the air interface rather than about anybody's product.

use core::fmt;

/// The service that provides the advertisement stream.
///
/// **`rivet-bluetoothd` is the arbiter**, and that is R-144.1 item 2 rather
/// than a preference: an HCI user-channel socket is exclusive, so an SDK module
/// that opened one directly would work for exactly one application and then
/// silently stop for the second. The shape is `rivet-launcher`'s with the
/// display (`ADR-015` §6.2, `ADR-036` §3.1) — the platform holds the scarce
/// thing and hands applications what they need.
pub const PROVIDER: &str = "rivet-bluetoothd";

/// The endpoint name, the other half of
/// `connects = ["rivet-bluetoothd:advertisements"]`.
///
/// **The endpoint name, not a path.** The daemon declares
/// `provides = ["socket:advertisements"]`, the supervisor binds it and hands
/// over the descriptor, and where it lives is Rivet's business and not the
/// application's (`ADR-015` §5.3). [`crate::input`] says the same thing from
/// the launcher's side.
pub const ENDPOINT: &str = "advertisements";

/// The most one advertisement line may be, including its newline.
///
/// Derived rather than chosen: the fixed fields are under 60 bytes and the AD
/// payload is at most [`MAX_AD_BYTES`] rendered as two hex characters each, so
/// 256 has room to spare and still bounds a reader's buffer. A line longer than
/// this did not come from [`Advertisement::encode`].
pub const LINE_MAX_BYTES: usize = 256;

/// The most advertising data a legacy LE advertising PDU can carry.
///
/// Bluetooth Core, the `AdvData` field of an `ADV_IND` PDU. A controller that
/// reported more than this in an `LE Advertising Report` would be reporting
/// something that cannot have arrived over the air, so [`parse_advertising_report`]
/// refuses it rather than growing a buffer.
pub const MAX_AD_BYTES: usize = 31;

/// How a device's address is to be read.
///
/// The two the air interface distinguishes. A **random** address may be static,
/// resolvable or non-resolvable, and telling those apart needs the top two bits
/// of the address itself — which is the application's business, because what it
/// is for is deciding whether the same sensor has been heard twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressKind {
    /// An IEEE-assigned public device address. Stable for the device's life.
    Public,
    /// A random device address. May rotate, and for a privacy-enabled device it
    /// does.
    Random,
}

impl AddressKind {
    /// The word that appears on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Random => "random",
        }
    }

    /// The `Address_Type` field of an `LE Advertising Report`.
    ///
    /// `0x02` and `0x03` are the *identity* address types, which a controller
    /// reports when its resolving list matched a private address. Tier 1 has no
    /// resolving list — that needs a bond, which needs pairing, which is tier 2
    /// — so they cannot occur here and are refused rather than folded into the
    /// two above. Folding would report an identity address as though the device
    /// had advertised it.
    #[must_use]
    pub const fn from_hci(byte: u8) -> Option<Self> {
        match byte {
            0x00 => Some(Self::Public),
            0x01 => Some(Self::Random),
            _ => None,
        }
    }

    /// The inverse of [`AddressKind::as_str`].
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "public" => Some(Self::Public),
            "random" => Some(Self::Random),
            _ => None,
        }
    }
}

/// Which kind of advertising PDU carried this report.
///
/// All five of the Bluetooth Core specification's legacy advertising event
/// types, kept apart because they mean different things to a consumer: a
/// `NonConnectable` is a beacon and a `ScanResponse` is an answer to a question
/// somebody asked. **Tier 1 scans passively and asks nothing**, so
/// [`ReportKind::ScanResponse`] should not arrive — and it is defined anyway,
/// because a controller that sent one would otherwise be a parse failure and
/// this file would be guessing about the air rather than reporting it
/// (`AGENTS.md` §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    /// `ADV_IND` — connectable and scannable, undirected.
    Connectable,
    /// `ADV_DIRECT_IND` — connectable, directed at one initiator.
    Directed,
    /// `ADV_SCAN_IND` — scannable, undirected, not connectable.
    Scannable,
    /// `ADV_NONCONN_IND` — a beacon. What a sensor broadcasting a reading uses.
    NonConnectable,
    /// `SCAN_RSP` — an answer to a scan request.
    ScanResponse,
}

impl ReportKind {
    /// The word that appears on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Connectable => "ind",
            Self::Directed => "direct",
            Self::Scannable => "scan",
            Self::NonConnectable => "nonconn",
            Self::ScanResponse => "scan-rsp",
        }
    }

    /// The `Event_Type` field of an `LE Advertising Report`.
    #[must_use]
    pub const fn from_hci(byte: u8) -> Option<Self> {
        match byte {
            0x00 => Some(Self::Connectable),
            0x01 => Some(Self::Directed),
            0x02 => Some(Self::Scannable),
            0x03 => Some(Self::NonConnectable),
            0x04 => Some(Self::ScanResponse),
            _ => None,
        }
    }

    /// The inverse of [`ReportKind::as_str`].
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "ind" => Some(Self::Connectable),
            "direct" => Some(Self::Directed),
            "scan" => Some(Self::Scannable),
            "nonconn" => Some(Self::NonConnectable),
            "scan-rsp" => Some(Self::ScanResponse),
            _ => None,
        }
    }
}

/// A device address, in the order a person writes it.
///
/// **Not the order it arrives in.** An HCI packet carries a `BD_ADDR`
/// least-significant byte first, and every printed form in every tool reverses
/// it. Storing the printed order and converting once, at the parse, is
/// `ADR-011`'s rule applied to six bytes: the alternative is a record whose
/// meaning depends on which end of the system you read it at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Address(pub [u8; 6]);

impl Address {
    /// From the six bytes of an HCI `BD_ADDR`, which are little-endian.
    #[must_use]
    pub const fn from_hci(le: [u8; 6]) -> Self {
        Self([le[5], le[4], le[3], le[2], le[1], le[0]])
    }

    /// `AA:BB:CC:DD:EE:FF`, upper case.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut out = [0u8; 6];
        let mut seen = 0usize;
        for (slot, field) in out.iter_mut().zip(text.split(':')) {
            if field.len() != 2 {
                return None;
            }
            *slot = u8::from_str_radix(field, 16).ok()?;
            seen += 1;
        }
        // `zip` stops at the shorter side, so a seventh field would go
        // unnoticed without this. Counting the separators is cheaper than
        // collecting.
        if seen != 6 || text.split(':').count() != 6 {
            return None;
        }
        Some(Self(out))
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c, d, e, g] = self.0;
        write!(f, "{a:02X}:{b:02X}:{c:02X}:{d:02X}:{e:02X}:{g:02X}")
    }
}

/// One advertisement, as an application receives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advertisement {
    /// When the daemon read it off the controller, in milliseconds on
    /// `CLOCK_MONOTONIC`.
    ///
    /// **Not a wall clock, and that is the point.** `ADR-021` is the wall
    /// clock's authority and a Rivet's may be unset, wrong, or about to be
    /// stepped by NTP — none of which should make a reading that is two seconds
    /// old look like a reading from next year. `CLOCK_MONOTONIC` is machine
    /// wide on Linux, so a consumer reads its own and subtracts to get the
    /// **age**, which is the only thing a freshness decision needs.
    pub at_ms: u64,
    /// Who advertised.
    pub address: Address,
    /// How to read [`Advertisement::address`].
    pub kind: AddressKind,
    /// Which advertising PDU carried it.
    pub report: ReportKind,
    /// Received signal strength in dBm, or `None` when the controller said it
    /// was not available.
    ///
    /// `0x7F` is the specification's *"RSSI is not available"*, and it is
    /// carried as `None` rather than as 127 dBm — which is not a signal
    /// strength any radio has ever measured and would be a plausible-looking
    /// number for something upstream to act on (`AGENTS.md` §5).
    pub rssi: Option<i8>,
    /// The advertising data, verbatim, at most [`MAX_AD_BYTES`] bytes.
    ///
    /// A sequence of length-tagged AD structures. Rivet does not interpret
    /// them; [`ad_structures`] will walk them for a consumer that wants to.
    pub data: Vec<u8>,
}

impl Advertisement {
    /// The line an application reads.
    ///
    /// ```text
    /// adv <at_ms> <AA:BB:CC:DD:EE:FF> <public|random> <ind|…> <rssi|?> <hex|->
    /// ```
    ///
    /// **Six fixed fields and no newline** — the caller adds one, because the
    /// caller is the one that knows whether it is writing to a socket or a
    /// journal record.
    ///
    /// # Why a line and not a struct on the wire
    ///
    /// The same argument [`crate::kiosk`] makes: this surface has exactly one
    /// producer and one consumer, both in this repository, and both parsed from
    /// this module — so the wire form is an implementation detail that a test
    /// holds in place rather than a published contract. What a text line buys
    /// over a packed struct is that **the QEMU proof and the journal are
    /// readable**, which is worth more here than the bytes it costs: an
    /// advertisement that arrived wrong should be legible in a log without a
    /// decoder.
    ///
    /// An empty payload is `-` rather than nothing, so the field count is
    /// always six and a truncated line cannot be read as a valid short one.
    #[must_use]
    pub fn encode(&self) -> String {
        let mut hex = String::with_capacity(self.data.len() * 2);
        for byte in &self.data {
            // `write!` into a String cannot fail, and the workspace denies
            // `unwrap`. Two nibbles by hand is shorter than the argument for
            // ignoring a Result.
            const DIGITS: &[u8; 16] = b"0123456789abcdef";
            hex.push(DIGITS[usize::from(byte >> 4)] as char);
            hex.push(DIGITS[usize::from(byte & 0x0f)] as char);
        }
        if hex.is_empty() {
            hex.push('-');
        }
        let rssi = match self.rssi {
            Some(dbm) => dbm.to_string(),
            None => "?".to_string(),
        };
        format!(
            "adv {} {} {} {} {} {}",
            self.at_ms,
            self.address,
            self.kind.as_str(),
            self.report.as_str(),
            rssi,
            hex
        )
    }

    /// Read one back.
    ///
    /// The line may carry a trailing newline or not; anything else that does
    /// not parse is [`LineError`] naming which part, because a consumer that
    /// cannot say *which* field was wrong sends its reader nowhere
    /// (`AGENTS.md` §5).
    ///
    /// # Errors
    ///
    /// [`LineError`], one variant per part of the line.
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
        let address = fields.next().and_then(Address::parse).ok_or(LineError::BadAddress)?;
        let kind = fields
            .next()
            .and_then(AddressKind::parse)
            .ok_or(LineError::BadAddressKind)?;
        let report = fields
            .next()
            .and_then(ReportKind::parse)
            .ok_or(LineError::BadReportKind)?;
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

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(text.len() / 2);
    for pair in bytes.as_chunks::<2>().0 {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        // Both are one hex digit, so the shift and the or cannot exceed a byte.
        out.push(((hi << 4) | lo) as u8);
    }
    Some(out)
}

/// Why a line could not be read as an advertisement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineError {
    /// Longer than [`LINE_MAX_BYTES`].
    TooLong,
    /// The first word was not `adv`. A future verb on this socket would land
    /// here, which is why the variant names the verb rather than the format.
    NotAnAdvertisement,
    BadTimestamp,
    BadAddress,
    BadAddressKind,
    BadReportKind,
    BadRssi,
    BadPayload,
    /// A seventh field. Refused rather than ignored: a reader that skipped it
    /// would accept a line from a newer producer and silently drop whatever it
    /// added.
    TrailingField,
}

impl fmt::Display for LineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
            "{said}. The line is `adv <ms> <addr> <public|random> <kind> <rssi|?> <hex|->` \
             and both ends of it are parsed from rivet_core::bluetooth"
        )
    }
}

impl core::error::Error for LineError {}

// ---------------------------------------------------------------------------
// The HCI side: what a controller says, turned into what an application reads
// ---------------------------------------------------------------------------

/// The HCI event code for an `LE Meta Event`.
pub const HCI_EV_LE_META: u8 = 0x3E;

/// The `LE Meta Event` subevent code for `LE Advertising Report`.
pub const HCI_SUBEV_LE_ADVERTISING_REPORT: u8 = 0x02;

/// Parse the parameters of an `LE Advertising Report` into records.
///
/// `params` is what follows the HCI event header — the subevent code first,
/// then `Num_Reports`, then that many reports. `at_ms` stamps every record it
/// produces, because they arrived in one event and pretending otherwise would
/// invent an ordering the controller did not give.
///
/// # Record-major, and it was read out of the kernel rather than remembered
///
/// The Core specification writes this event's parameters as parallel arrays
/// indexed by report, which reads as though every `Event_Type` came first. It
/// does not. `include/net/bluetooth/hci.h` in the tree this project builds
/// declares
///
/// ```text
/// struct hci_ev_le_advertising_info {
///     __u8     type;
///     __u8     bdaddr_type;
///     bdaddr_t bdaddr;
///     __u8     length;
///     __u8     data[];
/// } __packed;
/// ```
///
/// and `net/bluetooth/hci_event.c` reads the RSSI at `info->data[info->length]`
/// — so the layout is one whole report after another, with the RSSI last.
/// Getting this wrong does not fail: it produces a plausible address belonging
/// to nobody, which is exactly the class `AGENTS.md` §5 refuses.
///
/// # What it refuses
///
/// A truncated event, a report longer than [`MAX_AD_BYTES`], an address type
/// that is an *identity* address, and an event type the specification does not
/// define. Every one of those is [`ReportError`] carrying how far it got — a
/// partial parse is discarded rather than returned, because half an event is
/// not a smaller event.
///
/// # Errors
///
/// [`ReportError`].
pub fn parse_advertising_report(
    params: &[u8],
    at_ms: u64,
) -> Result<Vec<Advertisement>, ReportError> {
    let (&subevent, rest) = params.split_first().ok_or(ReportError::Truncated { at: 0 })?;
    if subevent != HCI_SUBEV_LE_ADVERTISING_REPORT {
        return Err(ReportError::NotAnAdvertisingReport { subevent });
    }
    let (&count, mut rest) = rest.split_first().ok_or(ReportError::Truncated { at: 1 })?;
    let mut out = Vec::with_capacity(usize::from(count));
    let mut consumed = 2usize;
    for _ in 0..count {
        // type, bdaddr_type, 6 address bytes, length = 9, then data, then rssi.
        if rest.len() < 9 {
            return Err(ReportError::Truncated { at: consumed });
        }
        let report = ReportKind::from_hci(rest[0]).ok_or(ReportError::UnknownEventType {
            value: rest[0],
            at: consumed,
        })?;
        let kind = AddressKind::from_hci(rest[1]).ok_or(ReportError::UnknownAddressType {
            value: rest[1],
            at: consumed + 1,
        })?;
        let mut le = [0u8; 6];
        le.copy_from_slice(&rest[2..8]);
        let length = usize::from(rest[8]);
        if length > MAX_AD_BYTES {
            return Err(ReportError::PayloadTooLong {
                length,
                at: consumed + 8,
            });
        }
        if rest.len() < 9 + length + 1 {
            return Err(ReportError::Truncated { at: consumed + 9 });
        }
        let data = rest[9..9 + length].to_vec();
        let raw_rssi = rest[9 + length] as i8;
        out.push(Advertisement {
            at_ms,
            address: Address::from_hci(le),
            kind,
            report,
            // 0x7F is the specification's "not available". See the field.
            rssi: (raw_rssi != 0x7F).then_some(raw_rssi),
            data,
        });
        let used = 9 + length + 1;
        consumed += used;
        rest = &rest[used..];
    }
    if !rest.is_empty() {
        return Err(ReportError::TrailingBytes { count: rest.len() });
    }
    Ok(out)
}

/// Why an `LE Advertising Report` could not be read.
///
/// Every variant carries a byte offset or a value, because the thing a reader
/// needs from this is the bytes to look at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportError {
    /// The event ended in the middle of a report.
    Truncated { at: usize },
    /// The subevent code is not [`HCI_SUBEV_LE_ADVERTISING_REPORT`].
    NotAnAdvertisingReport { subevent: u8 },
    /// An `Event_Type` outside the five the specification defines.
    UnknownEventType { value: u8, at: usize },
    /// An `Address_Type` that is not public or random — an identity address,
    /// which tier 1 cannot produce because it has no resolving list.
    UnknownAddressType { value: u8, at: usize },
    /// More advertising data than a legacy PDU can hold.
    PayloadTooLong { length: usize, at: usize },
    /// `Num_Reports` reports were read and bytes were left over.
    TrailingBytes { count: usize },
}

impl fmt::Display for ReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { at } => write!(
                f,
                "the LE Advertising Report ends inside a report, at byte {at} of its parameters"
            ),
            Self::NotAnAdvertisingReport { subevent } => write!(
                f,
                "this is LE Meta subevent 0x{subevent:02x} and an advertising report is 0x02"
            ),
            Self::UnknownEventType { value, at } => write!(
                f,
                "byte {at} is Event_Type 0x{value:02x}, which is not one of the five legacy \
                 advertising PDU types"
            ),
            Self::UnknownAddressType { value, at } => write!(
                f,
                "byte {at} is Address_Type 0x{value:02x}. Tier 1 has no resolving list, so an \
                 identity address (0x02, 0x03) cannot be produced and is not folded into public \
                 or random"
            ),
            Self::PayloadTooLong { length, at } => write!(
                f,
                "byte {at} claims {length} bytes of advertising data and a legacy PDU carries at \
                 most {MAX_AD_BYTES}"
            ),
            Self::TrailingBytes { count } => write!(
                f,
                "{count} byte(s) are left over after Num_Reports reports were read, so the event \
                 is not the shape it says it is"
            ),
        }
    }
}

impl core::error::Error for ReportError {}

/// Walk the length-tagged AD structures in [`Advertisement::data`].
///
/// Each is `(type, value)`; the length byte is consumed. A structure whose
/// length runs past the end **ends the walk** rather than being reported — the
/// remaining bytes are not a structure and guessing at them is the thing this
/// file does not do.
///
/// **This is a convenience and not an interpretation.** Nothing here knows what
/// AD type `0xFF` means beyond *manufacturer specific*, and that is where every
/// sensor's reading lives. Decoding it is the application's.
#[must_use]
pub fn ad_structures(data: &[u8]) -> Vec<(u8, Vec<u8>)> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < data.len() {
        let length = usize::from(data[at]);
        // A zero length is the specification's early terminator for the rest of
        // the field, so the walk stops rather than looping.
        // The second clause is the whole bound: reading a structure of `length`
        // bytes starting at `at + 1` touches `at + length` at the highest, so
        // it fits exactly when `at + 1 + length <= data.len()`. A third clause
        // stood here saying `at + length >= data.len() + 1`, which is the same
        // inequality one byte looser and could therefore never fire first.
        if length == 0 || at + 1 + length > data.len() {
            break;
        }
        out.push((data[at + 1], data[at + 2..at + 1 + length].to_vec()));
        at += 1 + length;
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **A hand-composed vector, and it is labelled as one** (`AGENTS.md` §4).
    ///
    /// It is the shape a Govee H5075 broadcasts — a non-connectable beacon from
    /// a public address with a manufacturer-specific structure — and the bytes
    /// were composed here rather than captured off the air, because no board
    /// has yet had a radio to capture them with. When one does, this vector
    /// should be replaced by one that was heard, and the difference will be
    /// visible in this comment.
    fn a_composed_report() -> Vec<u8> {
        let mut ev = vec![
            HCI_SUBEV_LE_ADVERTISING_REPORT,
            0x01, // one report
            0x03, // ADV_NONCONN_IND
            0x00, // public address
            // BD_ADDR, least significant byte first: A4:C1:38:11:22:33
            0x33, 0x22, 0x11, 0x38, 0xC1, 0xA4,
        ];
        let data: Vec<u8> = vec![
            0x02, 0x01, 0x06, // flags
            0x09, 0xFF, 0x88, 0xEC, 0x00, 0x03, 0x5B, 0x4C, 0x64, 0x00, // manufacturer
        ];
        ev.push(u8::try_from(data.len()).unwrap());
        ev.extend_from_slice(&data);
        ev.push(0xC1); // -63 dBm as a two's-complement byte
        ev
    }

    #[test]
    fn a_report_becomes_an_advertisement_with_the_address_the_right_way_round() {
        let got = parse_advertising_report(&a_composed_report(), 1234).unwrap();
        assert_eq!(got.len(), 1);
        let adv = &got[0];
        assert_eq!(adv.address.to_string(), "A4:C1:38:11:22:33");
        assert_eq!(adv.kind, AddressKind::Public);
        assert_eq!(adv.report, ReportKind::NonConnectable);
        assert_eq!(adv.rssi, Some(-63));
        assert_eq!(adv.at_ms, 1234);
        assert_eq!(adv.data.len(), 13);
    }

    /// The failure this reverses is silent: a big-endian read produces a
    /// perfectly plausible address belonging to a different device.
    #[test]
    fn the_hci_address_is_reversed_and_not_copied() {
        let addr = Address::from_hci([0x33, 0x22, 0x11, 0x38, 0xC1, 0xA4]);
        assert_eq!(addr.to_string(), "A4:C1:38:11:22:33");
        assert_ne!(addr.to_string(), "33:22:11:38:C1:A4");
    }

    #[test]
    fn a_line_round_trips_through_both_halves() {
        let adv = parse_advertising_report(&a_composed_report(), 987_654)
            .unwrap()
            .remove(0);
        let line = adv.encode();
        let expected = concat!(
            "adv 987654 A4:C1:38:11:22:33 public nonconn -63 ",
            "02010609ff88ec00035b4c6400"
        );
        assert_eq!(line, expected);
        assert_eq!(Advertisement::decode(&line).unwrap(), adv);
    }

    /// The same assertion over a record built by hand rather than parsed, so
    /// the two directions are not proved only against each other
    /// (`AGENTS.md` §4).
    #[test]
    fn the_wire_format_is_six_fields_in_this_order() {
        let adv = Advertisement {
            at_ms: 42,
            address: Address([0xA4, 0xC1, 0x38, 0x11, 0x22, 0x33]),
            kind: AddressKind::Random,
            report: ReportKind::Connectable,
            rssi: Some(-100),
            data: vec![0xDE, 0xAD],
        };
        assert_eq!(adv.encode(), "adv 42 A4:C1:38:11:22:33 random ind -100 dead");
        assert_eq!(Advertisement::decode(&adv.encode()).unwrap(), adv);
    }

    #[test]
    fn an_empty_payload_is_a_dash_and_survives_the_round_trip() {
        let adv = Advertisement {
            at_ms: 0,
            address: Address([0; 6]),
            kind: AddressKind::Public,
            report: ReportKind::NonConnectable,
            rssi: None,
            data: Vec::new(),
        };
        assert_eq!(adv.encode(), "adv 0 00:00:00:00:00:00 public nonconn ? -");
        assert_eq!(Advertisement::decode(&adv.encode()).unwrap(), adv);
    }

    /// `0x7F` means "not available" and must not become 127 dBm.
    #[test]
    fn an_unavailable_rssi_is_none_and_not_a_signal_strength() {
        let mut ev = a_composed_report();
        let last = ev.len() - 1;
        ev[last] = 0x7F;
        let adv = parse_advertising_report(&ev, 0).unwrap().remove(0);
        assert_eq!(adv.rssi, None);
        assert!(adv.encode().contains(" ? "));
    }

    #[test]
    fn two_reports_in_one_event_are_both_read_and_share_the_timestamp() {
        let mut ev = a_composed_report();
        ev[1] = 0x02;
        let one = ev[2..].to_vec();
        ev.extend_from_slice(&one);
        let got = parse_advertising_report(&ev, 55).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], got[1]);
        assert!(got.iter().all(|a| a.at_ms == 55));
    }

    #[test]
    fn a_truncated_event_is_refused_and_says_where() {
        let full = a_composed_report();
        for cut in 1..full.len() {
            let err = parse_advertising_report(&full[..cut], 0).unwrap_err();
            assert!(
                matches!(err, ReportError::Truncated { .. }),
                "cutting at {cut} gave {err:?}"
            );
        }
    }

    #[test]
    fn an_identity_address_type_is_refused_rather_than_folded() {
        let mut ev = a_composed_report();
        ev[3] = 0x02;
        assert!(matches!(
            parse_advertising_report(&ev, 0),
            Err(ReportError::UnknownAddressType { value: 0x02, .. })
        ));
    }

    #[test]
    fn a_payload_longer_than_a_pdu_can_carry_is_refused() {
        let mut ev = a_composed_report();
        ev[10] = 60;
        assert!(matches!(
            parse_advertising_report(&ev, 0),
            Err(ReportError::PayloadTooLong { length: 60, .. })
        ));
    }

    #[test]
    fn bytes_after_the_last_report_are_refused() {
        let mut ev = a_composed_report();
        ev.push(0x00);
        assert!(matches!(
            parse_advertising_report(&ev, 0),
            Err(ReportError::TrailingBytes { count: 1 })
        ));
    }

    #[test]
    fn another_le_subevent_is_named_rather_than_misread() {
        let mut ev = a_composed_report();
        ev[0] = 0x01; // LE Connection Complete
        assert!(matches!(
            parse_advertising_report(&ev, 0),
            Err(ReportError::NotAnAdvertisingReport { subevent: 0x01 })
        ));
    }

    #[test]
    fn every_way_a_line_can_be_wrong_names_the_field() {
        let good = "adv 42 A4:C1:38:11:22:33 random ind -100 dead";
        for (line, want) in [
            ("hello", LineError::NotAnAdvertisement),
            ("adv x A4:C1:38:11:22:33 random ind -100 dead", LineError::BadTimestamp),
            ("adv 42 nonsense random ind -100 dead", LineError::BadAddress),
            ("adv 42 A4:C1:38:11:22:33 blue ind -100 dead", LineError::BadAddressKind),
            ("adv 42 A4:C1:38:11:22:33 random hop -100 dead", LineError::BadReportKind),
            ("adv 42 A4:C1:38:11:22:33 random ind loud dead", LineError::BadRssi),
            ("adv 42 A4:C1:38:11:22:33 random ind -100 dea", LineError::BadPayload),
            ("adv 42 A4:C1:38:11:22:33 random ind -100 dead more", LineError::TrailingField),
        ] {
            assert_eq!(Advertisement::decode(line), Err(want), "{line}");
        }
        assert!(Advertisement::decode(good).is_ok());
    }

    #[test]
    fn a_line_over_the_bound_is_refused_before_it_is_parsed() {
        let line = format!("adv 42 A4:C1:38:11:22:33 random ind -100 {}", "ab".repeat(200));
        assert_eq!(Advertisement::decode(&line), Err(LineError::TooLong));
    }

    #[test]
    fn a_seventh_address_field_is_not_ignored() {
        assert_eq!(Address::parse("A4:C1:38:11:22:33:44"), None);
        assert_eq!(Address::parse("A4:C1:38:11:22"), None);
        assert_eq!(Address::parse("A4:C1:38:11:22:3"), None);
    }

    #[test]
    fn the_ad_walk_splits_structures_and_does_not_interpret_them() {
        let adv = parse_advertising_report(&a_composed_report(), 0)
            .unwrap()
            .remove(0);
        let structures = ad_structures(&adv.data);
        assert_eq!(structures.len(), 2);
        assert_eq!(structures[0], (0x01, vec![0x06]));
        assert_eq!(structures[1].0, 0xFF);
        assert_eq!(structures[1].1.len(), 8);
    }

    /// A length that runs past the end ends the walk. It must not panic and it
    /// must not return a structure built from bytes that are not there.
    #[test]
    fn an_overlong_ad_structure_ends_the_walk_rather_than_reading_past_it() {
        assert_eq!(ad_structures(&[0x05, 0xFF, 0x01]), Vec::new());
        assert_eq!(
            ad_structures(&[0x02, 0x01, 0x06, 0x09, 0xFF, 0x01]),
            vec![(0x01, vec![0x06])]
        );
        assert_eq!(ad_structures(&[0x00, 0x01]), Vec::new());
        assert_eq!(ad_structures(&[]), Vec::new());
    }

    #[test]
    fn the_two_names_are_the_ones_a_manifest_writes() {
        assert_eq!(PROVIDER, "rivet-bluetoothd");
        assert_eq!(ENDPOINT, "advertisements");
    }
}
