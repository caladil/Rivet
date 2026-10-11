//! **The pairwise master key**, derived from a passphrase the way 802.11
//! says to and nowhere else.
//!
//! `ADR-004` §5: *"The plaintext passphrase is never written to persistent
//! storage. Rivet performs the PBKDF2 derivation itself and stores only the
//! 32-byte PMK, which `wpa_supplicant` accepts directly in its `psk=<hex>`
//! form."* `ADR-034` §7.1 adds where it happens — **once, when a human types
//! the passphrase** — because the PMK is per-SSID and not per-device, so one
//! derivation provisions a whole fleet.
//!
//! # What this buys, stated exactly
//!
//! **Possessing the PMK is enough to join the network**, so this is not a
//! protection against somebody who wants your LAN. `ADR-034` §7.1 says what it
//! is for and it is worth repeating where the code is:
//!
//! > a human's WiFi passphrase is frequently a passphrase they have used
//! > elsewhere, and the PMK is not reversible to it. The thing that is never
//! > persisted is the thing whose loss reaches beyond this network.
//!
//! # The parameters are the standard's, and none of them is a choice
//!
//! IEEE 802.11i defines the PSK as
//! `PBKDF2(HMAC-SHA1, passphrase, ssid, 4096, 256)`. The salt is **the SSID
//! itself**, which is why the same passphrase on two networks gives two
//! different keys — and why a derivation needs both.
//!
//! **SHA-1 is not a weakness here and swapping it is not an option.** It is
//! what every access point computes; a PMK derived with anything else does not
//! join. PBKDF2-HMAC-SHA1 is also not the place SHA-1's collision weaknesses
//! apply: this is a PRF over a secret key, not a collision-resistant hash over
//! attacker-chosen input.
//!
//! # The survey `ADR-009` §3.2 requires
//!
//! | Candidate | |
//! | :-- | :-- |
//! | **`pbkdf2` (RustCrypto)** | **Taken.** Pure Rust, no build script compiles C, builds for `aarch64-unknown-linux-musl` and for the Windows development host (`ADR-009` §3.6.1). With `hmac` and `sha1` it adds **four** crates to this workspace's graph, not twelve: `hmac`, `digest`, `crypto-common`, `typenum`, `block-buffer` and `cpufeatures` are already here through `ed25519-dalek`. Measured, not assumed. |
//! | writing it | Rejected on `ADR-009` §3.3, which names this exact case: cryptography is where "a bespoke version is not merely slower to write but *worse*". PBKDF2 is simple enough to be tempting and the failure mode is a key that is subtly wrong on some inputs and right on the ones anybody tested. |
//! | `ring` | Rejected outright — assembly and C, `ADR-001` §4.4. The same answer `rivet-crypto` already gave for Ed25519. |

use hmac::Hmac;
use sha1::Sha1;

/// The size of a pairwise master key, in bytes. 802.11i says 256 bits.
pub const PMK_BYTES: usize = 32;

/// The iteration count 802.11i fixes. **Not a tuning knob**: every access point
/// computes 4096, and a different number produces a key that does not join.
pub const ITERATIONS: u32 = 4096;

/// Why a passphrase could not become a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PmkError {
    /// The passphrase is not a length 802.11 allows.
    PassphraseLength { bytes: usize },
}

impl core::fmt::Display for PmkError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            // **The length and never the passphrase.** `ADR-020` §4 — this is
            // the one error in the project whose careless form would print a
            // credential.
            Self::PassphraseLength { bytes } => write!(
                f,
                "a WPA passphrase is 8 to 63 bytes and this one is {bytes} — 802.11's own rule. \
                 A 64-character value is a PMK rather than a passphrase and is taken as one \
                 without being derived"
            ),
        }
    }
}

/// The shortest passphrase 802.11 allows.
pub const PASSPHRASE_MIN_BYTES: usize = 8;

/// The longest. **63 and not 64**, deliberately: 64 hexadecimal characters is
/// how a PMK itself is written, and the standard leaves the shorter range to
/// passphrases so the two can never be confused.
pub const PASSPHRASE_MAX_BYTES: usize = 63;

/// Derive the pairwise master key for one network.
///
/// # Errors
///
/// [`PmkError::PassphraseLength`] for a passphrase outside 802.11's 8..=63
/// bytes. The SSID is not checked here — `rivet_core::ssid::Ssid` is the type
/// that refuses an unusable one, and taking a `&str` would be a second place
/// that decides what a network name may be (`ADR-011`).
pub fn derive(ssid: &str, passphrase: &str) -> Result<[u8; PMK_BYTES], PmkError> {
    let bytes = passphrase.len();
    if !(PASSPHRASE_MIN_BYTES..=PASSPHRASE_MAX_BYTES).contains(&bytes) {
        return Err(PmkError::PassphraseLength { bytes });
    }
    let mut pmk = [0u8; PMK_BYTES];
    // **The SSID is the salt.** That is the whole reason a derivation needs
    // both: the same passphrase on two networks is two different keys.
    //
    // `expect`-free: `pbkdf2_hmac` writes into a fixed-size buffer and cannot
    // fail for a 32-byte output.
    pbkdf2::pbkdf2_hmac::<Sha1>(passphrase.as_bytes(), ssid.as_bytes(), ITERATIONS, &mut pmk);
    let _ = core::marker::PhantomData::<Hmac<Sha1>>;
    Ok(pmk)
}

/// The key as `wpa_supplicant` takes it — 64 lowercase hexadecimal characters.
///
/// **This is the form that goes in `SET_NETWORK <id> psk <hex>`**, without
/// quotes: a quoted value is a *passphrase* the supplicant would derive from
/// again, which would produce a different key and fail to associate.
#[must_use]
pub fn to_hex(pmk: &[u8; PMK_BYTES]) -> String {
    pmk.iter().fold(String::with_capacity(PMK_BYTES * 2), |mut out, byte| {
        use core::fmt::Write as _;
        // Writing to a String cannot fail; dropped rather than unwrapped so
        // this cannot become a panic on a board.
        let _ = write!(out, "{byte:02x}");
        out
    })
}

#[cfg(test)]
#[path = "pmk_tests.rs"]
mod tests;
