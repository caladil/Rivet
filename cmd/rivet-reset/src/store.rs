//! **The Windows certificate stores — reading them, and removing exactly what
//! was named.**
//!
//! # The three stores, and why all three
//!
//! `SPEC-05` §2 names them, and each one holds a different half of the
//! problem:
//!
//! | store | what a fleet puts there |
//! | :-- | :-- |
//! | `CurrentUser\My` | the operator's own identity — the certificate a browser OFFERS |
//! | `CurrentUser\Root` | the authority, once somebody double-clicked `root.crt` |
//! | `CurrentUser\CA` | the authority again: the *Intermediate* store is where the import wizard puts a constrained CA, and it is the one nobody thinks to look in |
//!
//! Leaving any of the three behind reproduces the 2026-09-07 state, where a
//! browser offered four certificates and none of them could authenticate.
//!
//! # Why PowerShell and not `certutil`
//!
//! `cmd/rivet-provision`'s `browser.rs` shells out to `certutil` to IMPORT, and
//! its reasoning is sound for that job: one command, no elevation, no Windows
//! crate linked into a tool that otherwise has no platform code.
//!
//! **Reading is a different job and `certutil` is the wrong tool for it.** Its
//! `-store` output is prose, and prose that is *localised* — the line a parser
//! would look for is translated on a machine that is not running English
//! Windows, so a matcher built on it would silently find nothing and report a
//! clean store. The certificate provider hands back `RawData`, which is the
//! certificate itself, in every locale.
//!
//! And for removal it buys one more thing that matters here: `Remove-Item
//! -DeleteKey` removes the **private key** with the certificate. `certutil
//! -delstore` leaves the key container behind, and an orphaned private key for
//! an identity that no longer exists is exactly the kind of leftover this whole
//! program exists to stop.
//!
//! # Nothing here is matched by name
//!
//! This module reads and deletes by **thumbprint**, which is a SHA-1 over the
//! whole certificate: the path handed to `Remove-Item` cannot address a
//! certificate other than the one that was reported. [`crate::authority`] is
//! what decides which those are.

/// The stores a fleet's material can be in, and the only ones this touches.
///
/// **`CurrentUser` throughout.** `ADR`-level reasoning is in `browser.rs`: a
/// client identity in the machine store is not offered to a browser running as
/// this user, so nothing this project files is ever machine-wide — and a tool
/// that deleted from `LocalMachine` would be reaching outside what it put there.
pub const STORES: [&str; 3] = ["My", "Root", "CA"];

/// One certificate as a store holds it.
pub struct Held {
    /// `My`, `Root` or `CA`.
    pub store: &'static str,
    /// SHA-1 over the certificate, upper-case hex. **The address.**
    pub thumbprint: String,
    /// Whether Windows has a private key attached.
    ///
    /// **Reported because its absence is a whole class of failure.** On
    /// 2026-09-07 the one correct certificate sat in the Personal store with no
    /// key, and a browser silently declines to offer such a certificate —
    /// so *"none of my certificates work"* with the right one invisible among
    /// them (`SPEC-05` §4).
    pub has_private_key: bool,
    /// The certificate itself.
    pub der: Vec<u8>,
}

/// Everything in the three stores.
///
/// # Errors
///
/// Whatever stopped a store being read — never an empty list standing in for a
/// failure, because a reset that reported *"no certificates to remove"* after
/// failing to open a store would be the worst possible answer.
#[cfg(windows)]
pub fn survey() -> Result<Vec<Held>, String> {
    let mut out = Vec::new();
    for store in STORES {
        out.extend(read(store)?);
    }
    Ok(out)
}

/// The unbuilt platforms, which say so rather than reporting a clean store.
///
/// `browser.rs` names what the other two would need to FILE an identity;
/// removing one is the same three-way split — macOS keychains, and on Linux the
/// per-profile NSS databases that are not the system trust store. Neither is
/// built, and a silent success on either would be a program that claims to have
/// cleaned a store it never opened.
///
/// # Errors
///
/// Always, and says which platform it is.
#[cfg(not(windows))]
pub fn survey() -> Result<Vec<Held>, String> {
    Err("only the Windows certificate stores are built (ADR-002's development \
         host). On macOS the equivalent lives in the login keychain; on Linux there is no \
         single store — Chrome reads ~/.pki/nssdb and Firefox keeps one per profile."
        .to_owned())
}

/// Read one store.
#[cfg(windows)]
fn read(store: &'static str) -> Result<Vec<Held>, String> {
    // One line per certificate: thumbprint, whether a private key is attached,
    // and the certificate as hex. **Hex rather than base64** so that decoding it
    // needs no dependency and no hand-written decoder, and
    // `[BitConverter]::ToString` renders a kilobyte in one call rather than one
    // pipeline element per byte.
    let script = format!(
        "$ErrorActionPreference='Stop'; \
         Get-ChildItem -Path Cert:\\CurrentUser\\{store} | ForEach-Object {{ \
         $_.Thumbprint + ' ' + $_.HasPrivateKey + ' ' + \
         [BitConverter]::ToString($_.RawData).Replace('-','') }}"
    );
    let text = powershell(&script).map_err(|e| format!("CurrentUser\\{store}: {e}"))?;

    let mut out = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let mut parts = line.split(' ');
        let (Some(thumbprint), Some(key), Some(hex)) =
            (parts.next(), parts.next(), parts.next())
        else {
            return Err(format!(
                "CurrentUser\\{store} answered a line this cannot read: {line:?}"
            ));
        };
        let der = unhex(hex).ok_or_else(|| {
            format!("CurrentUser\\{store}: {thumbprint} came back as something that is not hex")
        })?;
        out.push(Held {
            store,
            thumbprint: thumbprint.to_ascii_uppercase(),
            // PowerShell renders a boolean as `True`/`False`.
            has_private_key: key.eq_ignore_ascii_case("true"),
            der,
        });
    }
    Ok(out)
}

/// **Remove one certificate, addressed by its thumbprint.**
///
/// `-DeleteKey` is passed only when there is a key to delete, because it is a
/// dynamic parameter of the certificate provider and asking it to delete a key
/// that is not there is a failure this has no reason to risk.
///
/// # Errors
///
/// Whatever PowerShell said, with the store and the thumbprint.
#[cfg(windows)]
pub fn remove(held: &Held) -> Result<(), String> {
    let path = format!("Cert:\\CurrentUser\\{}\\{}", held.store, held.thumbprint);
    let key = if held.has_private_key { " -DeleteKey" } else { "" };
    let script =
        format!("$ErrorActionPreference='Stop'; Remove-Item -Path '{path}'{key} -Force");
    powershell(&script)
        .map(|_| ())
        .map_err(|e| format!("{path}: {e}"))
}

/// The same, refused by name anywhere but Windows.
///
/// # Errors
///
/// Always.
#[cfg(not(windows))]
pub fn remove(_held: &Held) -> Result<(), String> {
    Err("removing a certificate is built only on Windows".to_owned())
}

/// Run a script and hand back its output.
///
/// **`-NoProfile` is not tidiness.** A profile can print, and its output would
/// arrive on the same stream as the certificates — so a person with a banner in
/// their profile would have every store misread. `-NonInteractive` is the same
/// argument for prompts.
#[cfg(windows)]
fn powershell(script: &str) -> Result<String, String> {
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .map_err(|e| format!("powershell could not be run: {e}"))?;
    if !out.status.success() {
        let said = String::from_utf8_lossy(&out.stderr);
        let last = said
            .lines()
            .map(str::trim)
            .rfind(|l| !l.is_empty())
            .unwrap_or("powershell refused and said nothing");
        return Err(last.to_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Hex to bytes, or `None` for anything that is not an even run of hex digits.
///
/// **`None` rather than a partial answer.** A truncated certificate parses as
/// nothing, which this program would report as *unreadable* — an honest-looking
/// answer to a question that was never asked properly.
#[cfg(windows)]
fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || text.is_empty() {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(text.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push(u8::try_from(hi * 16 + lo).ok()?);
    }
    Some(out)
}

#[cfg(all(test, windows))]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn hex_comes_back_as_the_bytes_that_went_in() {
        assert_eq!(unhex("3082").expect("even hex"), vec![0x30, 0x82]);
        assert_eq!(unhex("ff00AB").expect("mixed case"), vec![0xff, 0x00, 0xab]);
        assert!(unhex("308").is_none(), "an odd run is not half a byte");
        assert!(unhex("30zz").is_none(), "a non-hex digit is not a byte");
        assert!(unhex("").is_none(), "nothing is not an empty certificate");
    }

    /// **Reading the operator's own store, and asserting nothing about what is
    /// in it.** What is asserted is that the read works and produces
    /// certificates — a store this cannot read is a reset that would report
    /// nothing to remove.
    ///
    /// Read-only, and the one test here that touches the machine.
    #[test]
    fn the_three_stores_can_be_read_and_hold_certificates() {
        let held = survey().expect("the three CurrentUser stores must be readable");
        assert!(
            !held.is_empty(),
            "no certificate at all in My, Root or CA — a Windows install always has roots"
        );
        for one in &held {
            assert_eq!(one.thumbprint.len(), 40, "a SHA-1 thumbprint is 40 hex digits");
            assert!(
                crate::authority::subject_of(&one.der) != "(unreadable)",
                "{} in {} did not come back as a certificate",
                one.thumbprint,
                one.store
            );
        }
    }
}
