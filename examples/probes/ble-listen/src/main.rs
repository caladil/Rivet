//! **Read the advertisement endpoint and say what arrived.** `ADR-041` §8.3.
//!
//! Everything this prints goes to stdout, which the supervisor captures into
//! the journal (`ADR-022`), so the whole finding is readable with
//! `rivet get log latest all`.
//!
//! Three kinds of line, and the shapes are deliberate:
//!
//! * **`first`** — the opening [`DETAIL_COUNT`] advertisements, one line each,
//!   in full. This is the evidence that a real radio heard a real device: an
//!   address, a signal strength and the bytes.
//! * **`heard`** — a periodic roll-up, so a long run says something without
//!   writing a line per advertisement. A board in a house hears a great many
//!   per second and `ADR-022` §8's journal is a rolling 1 MB.
//! * **`device`** — one line per address in the roll-up, with how many times it
//!   was heard, the last RSSI and the name it gave if it gave one.
//!
//! **Nothing here decodes a vendor payload.** `ADR-041` §6.3 keeps that out of
//! the platform; putting it in the probe would move the mistake rather than
//! avoid it. A Complete Local Name is rendered as text because AD type `0x09`
//! means a name for every device that sends one — a sensor names itself, and
//! this program was told nothing about any sensor.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use rivet_sdk::bluetooth::{Advertisement, Advertisements};

/// How many advertisements are printed in full before the roll-up takes over.
const DETAIL_COUNT: usize = 24;

/// How often the roll-up is written.
///
/// **This probe is expensive for a journal and should be disabled when it is
/// not being read.** `ADR-022` §8's journal is a rolling ~1 MB, and a roll-up
/// naming [`DEVICES_LISTED`] devices is a dozen records; left enabled it
/// displaces whatever else the board is recording.
const SUMMARY_EVERY: Duration = Duration::from_secs(60);

/// How many devices the roll-up names. The rest are counted, not listed.
const DEVICES_LISTED: usize = 16;

/// AD type 0x09, *Complete Local Name*, and 0x08, *Shortened Local Name*.
const AD_COMPLETE_NAME: u8 = 0x09;
const AD_SHORT_NAME: u8 = 0x08;

/// What has been heard from one address.
#[derive(Default)]
struct Seen {
    count: u64,
    last_rssi: Option<i8>,
    name: Option<String>,
    last_data: Vec<u8>,
}

fn main() {
    let stream = match rivet_sdk::bluetooth::advertisements() {
        Ok(stream) => stream,
        Err(why) => {
            println!("ble-listen: NOT LISTENING - {why}");
            std::process::exit(1);
        }
    };
    println!(
        "ble-listen: reading the advertisement stream at {}",
        stream.path().display()
    );
    listen(stream);
}

fn listen(stream: Advertisements) {
    let mut seen: BTreeMap<String, Seen> = BTreeMap::new();
    let mut total: u64 = 0;
    let mut malformed: u64 = 0;
    let mut detailed = 0usize;
    let mut last_summary = Instant::now();

    for item in stream {
        let advertisement = match item {
            Ok(advertisement) => advertisement,
            Err(e) => {
                malformed = malformed.saturating_add(1);
                // Not skipped silently: the producer and this decoder are the
                // two halves of one format, so a line that does not parse means
                // they disagree, and that is worth a record.
                println!("ble-listen: A LINE DID NOT PARSE - {e}");
                continue;
            }
        };
        total = total.saturating_add(1);

        if detailed < DETAIL_COUNT {
            detailed += 1;
            println!("ble-listen: first {detailed} - {}", describe(&advertisement));
        }

        let entry = seen.entry(advertisement.address.to_string()).or_default();
        entry.count = entry.count.saturating_add(1);
        entry.last_rssi = advertisement.rssi;
        entry.last_data = advertisement.data.clone();
        if entry.name.is_none() {
            entry.name = local_name(&advertisement);
        }

        if last_summary.elapsed() >= SUMMARY_EVERY {
            last_summary = Instant::now();
            summarise(&seen, total, malformed);
        }
    }

    // EOF. `rivet-bluetoothd` closed the socket, which is what a restart of it
    // looks like from here. Say so and let `restart = "on-failure"` bring this
    // back rather than exiting 0 and looking like a clean stop.
    summarise(&seen, total, malformed);
    println!(
        "ble-listen: THE STREAM ENDED - rivet-bluetoothd closed the socket, which is what its \
         restart looks like from here. {total} advertisement(s) had been heard"
    );
    std::process::exit(1);
}

fn summarise(seen: &BTreeMap<String, Seen>, total: u64, malformed: u64) {
    println!(
        "ble-listen: heard {total} advertisement(s) from {} device(s), {malformed} line(s) that \
         did not parse",
        seen.len()
    );
    // **A device that named itself is listed before a busier one that did
    // not**, and that is not cosmetic. In a house most of what a radio hears is
    // a rotating random address broadcasting a manufacturer blob, and the
    // handful that send a Complete Local Name are the ones somebody can
    // recognise — a sensor says what model it is. Sorting by count alone buried
    // the only identifiable device in this room below eleven anonymous ones.
    let mut by_count: Vec<(&String, &Seen)> = seen.iter().collect();
    by_count.sort_by(|a, b| {
        b.1.name
            .is_some()
            .cmp(&a.1.name.is_some())
            .then(b.1.count.cmp(&a.1.count))
            .then(a.0.cmp(b.0))
    });
    for (address, entry) in by_count.iter().take(DEVICES_LISTED) {
        println!(
            "ble-listen: device {address} x{} {} {} {}",
            entry.count,
            entry
                .last_rssi
                .map_or_else(|| "rssi ?".to_string(), |r| format!("rssi {r}dBm")),
            entry
                .name
                .as_deref()
                .map_or_else(|| "unnamed".to_string(), |n| format!("name `{n}`")),
            hex(&entry.last_data),
        );
    }
    if by_count.len() > DEVICES_LISTED {
        println!(
            "ble-listen: and {} more device(s) not listed",
            by_count.len() - DEVICES_LISTED
        );
    }
}

/// One advertisement, whole, on one line.
fn describe(advertisement: &Advertisement) -> String {
    let structures = advertisement
        .structures()
        .into_iter()
        .map(|(kind, value)| format!("{kind:02x}:{}", hex(&value)))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "{} {:?} {:?} {} [{}]",
        advertisement.address,
        advertisement.kind,
        advertisement.report,
        advertisement
            .rssi
            .map_or_else(|| "rssi ?".to_string(), |r| format!("rssi {r}dBm")),
        structures,
    )
}

/// The device's own name, if it sent one.
fn local_name(advertisement: &Advertisement) -> Option<String> {
    advertisement
        .structures()
        .into_iter()
        .find(|(kind, _)| *kind == AD_COMPLETE_NAME || *kind == AD_SHORT_NAME)
        .map(|(_, value)| String::from_utf8_lossy(&value).into_owned())
}

fn hex(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "-".to_string();
    }
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
