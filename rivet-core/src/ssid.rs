//! **The name of a wireless network**, and the one thing about wireless that
//! belongs in a configuration document.
//!
//! # Why an SSID is here and a passphrase is not
//!
//! `ADR-007` §7.3 keeps *wireless credentials* out of the configuration
//! document — the PMK belongs to `ADR-020`'s store, and §7.6.3 records that the
//! schema deliberately declares no key one could be written under.
//!
//! **An SSID is not a credential.** Every access point broadcasts it, in the
//! clear, several times a second; it is on the screen of every phone in the
//! building. Keeping it out of the document would protect nothing and would
//! leave a machine unable to say which network it is *for* — which is desired
//! state in exactly `ADR-007` §3.3's sense, and the same kind of fact as an
//! address.
//!
//! So the split is: **the document names the network, the store holds the key
//! to it.** A document is readable, diffable, committable and safe to paste
//! into a bug report; none of that changes because it says which WiFi a board
//! is on.
//!
//! # The rule, and why it is bytes rather than characters
//!
//! 802.11 gives the SSID element a length field of one octet and caps it at 32
//! — **octets, not characters.** A name of 32 emoji is not a valid SSID and a
//! type that counted characters would accept one and produce a frame no radio
//! can send. The standard says nothing about the encoding at all, which is why
//! this holds a `String` and validates its *byte* length: a Rivet document is
//! UTF-8, so a name that reached here is text, and the length is the part that
//! has to be checked against the radio's rule rather than the file's.
//!
//! **Empty is refused.** A zero-length SSID is the wildcard a client sends when
//! it is asking *what is out there* — it is a probe, not a network, and a
//! document asking to join it is asking for something that cannot be satisfied.

use crate::CoreError;

/// The largest SSID 802.11 can carry, in **octets**.
pub const SSID_MAX_BYTES: usize = 32;

/// The name of a wireless network.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ssid(String);

impl Ssid {
    /// The whole rule in one sentence, for a diagnostic that must not quote its
    /// input.
    ///
    /// `&'static str` for [`crate::net::InterfaceName::RULE`]'s reason, and it
    /// applies with more force here: the document that carries an SSID is the
    /// document `ADR-020` §4 is about, and a refusal that echoed a line from it
    /// would be one edit away from echoing the wrong line.
    pub const RULE: &'static str =
        "a wireless network name is 1 to 32 BYTES — 802.11's own limit, which counts octets and \
         not characters, so a short name in a wide script can still be too long. An empty name is \
         the wildcard a client uses to ask what is out there, and is not a network to join";

    /// Build one, refusing what a radio could not carry.
    ///
    /// # Errors
    ///
    /// [`CoreError::InvalidSsid`] for an empty name or one over
    /// [`SSID_MAX_BYTES`] octets.
    pub fn new(text: &str) -> Result<Self, CoreError> {
        // **Bytes, not `chars().count()`.** See the module documentation: the
        // limit is the radio's and the radio counts octets.
        let length = text.len();
        if length == 0 || length > SSID_MAX_BYTES {
            return Err(CoreError::InvalidSsid {
                length,
                detail: Self::RULE,
            });
        }
        Ok(Self(text.to_owned()))
    }

    /// The name, as the supplicant will be given it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for Ssid {
    /// **An SSID prints as itself**, unlike a `crate::secret::Secret`. It is
    /// public by construction — it travels in the clear whenever a device joins,
    /// even on a network that does not advertise it — and a
    /// machine that could not say which network it was configured for would be
    /// hiding the one wireless fact worth reporting.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
#[path = "ssid_tests.rs"]
mod tests;
