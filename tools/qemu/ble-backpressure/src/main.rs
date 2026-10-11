//! **Take the advertisement endpoint, and never read it.**
//!
//! `ADR-041` §6.3's drop-and-count path, driven by a real consumer over a real
//! granted socket, which is what `tools/qemu/adr-041-proof.sh` stage 2 is for.
//! The evidence this program produces is mostly not its own output: it is the
//! **daemon's** record —
//!
//! > an application is not reading advertisements as fast as this machine hears
//! > them, so records were DROPPED rather than waited for
//!
//! — which cannot appear unless something is connected and behind.
//!
//! # It reads [`FIRST`] advertisements first, and that is the control
//!
//! A proof that only showed the drops could not tell back-pressure from a
//! socket that never carried anything: **both report identically**. So this
//! reads a handful, prints them, and only then stops — on one boot, over one
//! connection, to one consumer, the same socket is shown delivering and then
//! being dropped on. `AGENTS.md` §7 is the rule and `tools/qemu/adr-042-proof
//! .sh`'s second half is the same shape.
//!
//! **It must not exit**, and `restart = "never"` in its manifest is the other
//! half of that: a consumer that came and went would take the drop counter's
//! subject with it.

use std::time::Duration;

/// How many to read before going deaf. Small, so that the buffer behind it
/// fills promptly, and non-zero, so that the control above exists at all.
const FIRST: usize = 5;

/// How long to keep trying to open the endpoint.
///
/// `rivet_sdk::bluetooth::advertisements` says it in its own refusal: *"nothing
/// orders a consumer after its provider, so retry"*. The supervisor starts
/// services in one pass and this one can win the race with `rivet-bluetoothd`'s
/// bind, so a single attempt would make the proof depend on scheduling.
const ATTEMPTS: u32 = 60;
const BETWEEN: Duration = Duration::from_millis(250);

fn main() {
    let mut last = String::new();
    for attempt in 1..=ATTEMPTS {
        match rivet_sdk::bluetooth::advertisements() {
            Ok(stream) => {
                println!(
                    "ble-backpressure: HOLDING {} on attempt {attempt}",
                    stream.path().display()
                );
                // The stream is kept alive deliberately. Dropping it would
                // close the socket, and the daemon would report the consumer
                // gone rather than behind — the opposite of what is being
                // shown.
                park(stream);
            }
            Err(why) => last = why.to_string(),
        }
        std::thread::sleep(BETWEEN);
    }
    println!("ble-backpressure: NEVER CONNECTED after {ATTEMPTS} attempts - {last}");
    std::process::exit(1);
}

/// Read [`FIRST`], then hold the endpoint open and read no more.
fn park(mut stream: rivet_sdk::bluetooth::Advertisements) -> ! {
    let mut read = 0usize;
    while read < FIRST {
        match stream.next() {
            Some(Ok(advertisement)) => {
                read += 1;
                println!(
                    "ble-backpressure: read {read} - {} rssi {:?} {} byte(s) of AD",
                    advertisement.address,
                    advertisement.rssi,
                    advertisement.data.len()
                );
            }
            Some(Err(e)) => println!("ble-backpressure: A LINE DID NOT PARSE - {e}"),
            None => {
                println!("ble-backpressure: THE STREAM ENDED before {FIRST} arrived");
                break;
            }
        }
    }
    println!(
        "ble-backpressure: DEAF from here on, holding the endpoint. Every advertisement after \
         this one is a record the daemon has nowhere to put"
    );
    loop {
        std::thread::sleep(Duration::from_secs(3600));
        // Unreachable in the life of a proof run, and here so that `stream` is
        // owned by this frame rather than dropped at the end of `main`.
        let _ = &stream;
    }
}
