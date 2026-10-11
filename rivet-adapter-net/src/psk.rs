//! **Where a network's key lives on a machine**, and what is in the file.
//!
//! `ADR-020` §5: the document says which secrets exist, the values live under
//! `rivet_core::state::SECRETS`. `ADR-007` §7.3 keeps the key out of the
//! document entirely, so `[network.wlan0] ssid = "..."` is the document saying
//! *this interface joins a network and therefore needs its key*, and this is
//! where the key is found.
//!
//! # One file per interface, named after the interface
//!
//! Not after the SSID. An SSID can contain a slash, a NUL is the only byte it
//! cannot contain, and turning one into a filename is an escaping problem with
//! no upside. An interface name is already constrained to something a path can
//! hold — `rivet_core::net::InterfaceName` enforces the kernel's own rule — and
//! it is what the caller has in hand.
//!
//! **The consequence, stated so it is not discovered:** a machine holds one key
//! per interface, not one per network. Roaming between two networks on one
//! radio is not expressible, and `ADR-034` §7.5 already lists multiple networks
//! as deliberately unsettled.
//!
//! # What is in the file, and why it is not `rivet_core::secret`'s form
//!
//! **64 lowercase hexadecimal characters and nothing else** — the PMK, in
//! exactly the form `SET_NETWORK <id> psk` takes.
//!
//! `Secret::to_stored` encodes *text* as hex so that grepping a card for a
//! known password does not hit. A PMK is not text: it is 32 bytes of key
//! material that is already written as hex by universal convention, and passing
//! it through that encoding would produce 128 characters that have to be
//! decoded before use and would decode to something `from_stored` would refuse
//! as `NotText`. The file therefore holds the wire form directly, which is
//! also the form a person pastes from `wpa_passphrase`.
//!
//! That is a deliberate exception to `ADR-020` §11.2.3.1's hex-encoding rule,
//! and it costs nothing that rule was buying: the rule exists so a plaintext
//! password does not sit on a card, and a PMK is not a password — it is not
//! reversible to one, which is the whole point of deriving it.
//!
//! **The FORM in the file is the exception; the TYPE in memory is not.** The
//! paragraph above is about how the key is written down. Once read, it is a
//! [`Secret`] like every other credential on the machine — `ADR-020` §4's rule
//! that secrecy is a property of the type has no exception here and this module
//! stopped being one on 2026-09-04.

use rivet_core::net::InterfaceName;
use rivet_core::secret::Secret;

/// How many characters a stored key has.
pub const PMK_HEX_LENGTH: usize = 64;

/// Where one interface's key lives.
///
/// Composed here and nowhere else (`ADR-011`): `cmd/rivet-netd` reads it and
/// whatever writes it must agree, and two spellings of a path is how a machine
/// comes to have a key nothing can find.
#[must_use]
pub fn path_of(interface: &InterfaceName) -> String {
    rivet_core::secret::path_of(&format!("{interface}.psk"))
}

/// Why a stored key could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PskError {
    /// There is no key for this interface.
    Absent { path: String },
    /// The file holds something that is not a PMK.
    NotAPmk { path: String, length: usize },
}

impl core::fmt::Display for PskError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Absent { path } => write!(
                f,
                "this interface names a wireless network and there is no key for it at {path}. \
                 The document says WHICH network to join and ADR-007 §7.3 keeps the key out of \
                 the document, so the key is put there separately — nothing was joined"
            ),
            // **The length, never the contents.** `ADR-020` §4: this is a file
            // whose whole purpose is to hold a credential, and a diagnostic
            // that quoted it would put one in the journal.
            Self::NotAPmk { path, length } => write!(
                f,
                "{path} holds {length} characters and a pairwise master key is {PMK_HEX_LENGTH} \
                 hexadecimal ones. A passphrase is not a key — it must be derived first, which is \
                 what `rivet_crypto::pmk` is for. Nothing was joined"
            ),
        }
    }
}

/// Read one interface's key, checking it is one.
///
/// **Validated here rather than by the supplicant**, because the supplicant's
/// answer to a malformed key is to try to associate and fail — which looks
/// exactly like a wrong password, and sends an operator to the router.
///
/// # A [`Secret`] and not a `String`, since 2026-09-04
///
/// The audit of 2026-09-03's LOW list: *"PSK returned as a plain `String`"*.
///
/// `ADR-020` §11.2.3 removed the envelope and the key provider and kept the
/// half it called the highest-value: **secrecy is a property of the type.**
/// [`Secret`] redacts in `Debug` and has no `Display` at all, so the only way to
/// a pairwise master key is [`Secret::expose`] — a name somebody has to type and
/// a reviewer can grep for — and `zeroize` clears the bytes when it drops.
///
/// This function returned a bare `String`, so the one credential on a Rivet that
/// `ADR-020` was written about was the one value not carrying the type. That is
/// not a hypothetical: `rivet-netd` builds `format!` diagnostics from what it
/// reads all through this path, and the failure §4 names is *the accident*.
///
/// # Errors
///
/// [`PskError`], both variants of which mean nothing was joined.
pub fn read(text: Option<String>, path: &str) -> Result<Secret, PskError> {
    let Some(text) = text else {
        return Err(PskError::Absent {
            path: path.to_owned(),
        });
    };
    let key = text.trim().to_ascii_lowercase();
    if key.len() != PMK_HEX_LENGTH || !key.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(PskError::NotAPmk {
            path: path.to_owned(),
            length: key.len(),
        });
    }
    Ok(Secret::new(key))
}

#[cfg(test)]
#[path = "psk_tests.rs"]
mod tests;
