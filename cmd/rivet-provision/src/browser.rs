//! **Putting the operator's certificate where their browser will find it.**
//!
//! # The problem this exists to remove
//!
//! Minting a client certificate was already automatic; *using* it was not. On
//! 2026-09-01 the maintainer had to install OpenSSL, run
//!
//! ```text
//! openssl pkcs12 -export -out browser.pfx -inkey browser.key -in browser.crt
//! ```
//!
//! invent a password to satisfy it, and then find the right certificate store
//! in a wizard. His objection to the last part is the reason this module is not
//! just a file writer: the browser's certificate has to be in one specific
//! store, which a user has no natural reason to know about (.rulings/ R-280).
//!
//! **That is the whole argument.** A person can reasonably be asked to
//! double-click a root certificate; they cannot reasonably be expected to know
//! that a client identity belongs in `Personal` under the *current user* rather
//! than the machine, which is the one choice the wizard puts in front of them
//! and the one they have no way to get right.
//!
//! # What this deliberately does NOT do
//!
//! **It does not touch a trust store.** The root stays a double-click, by the
//! maintainer's ruling: a one-time double-click on the root is an acceptable
//! setup step (.rulings/ R-280) — and that division is worth keeping for a
//! better reason than convenience: installing an authority changes what this
//! computer *believes*, and placing a client identity only files something its
//! owner already had. A tool that quietly did the first would be doing
//! something a person should perform deliberately.
//!
//! # Windows only, and the other two are named rather than implied
//!
//! `ADR-002` makes the development host Windows and `ADR-034` §2.2 records
//! that the operator's machine may be *"Windows 11, Linux or macOS"*. Only the
//! first is built. See [`Placement::Unbuilt`] — the others get a sentence
//! saying what they would need, because a tool that silently did nothing on
//! macOS would be worse than one that says it cannot.

use std::path::Path;

use crate::ca::Minted;

/// What happened when this tried to file the identity.
pub enum Placement {
    /// It is in the store named, and the browser will offer it.
    ///
    /// **Constructed on Windows only**, the mirror of `Unbuilt` below: the
    /// store this files into is a Windows one.
    #[cfg_attr(not(windows), allow(dead_code))]
    Placed(String),
    /// This platform's path is not built. Carries what to do by hand.
    ///
    /// **Unconstructed on Windows, and that is the point** — only the
    /// `#[cfg(not(windows))]` half of [`place`] returns it, so a Windows build
    /// is right to say nothing makes one. It is not dead code; it is the other
    /// two platforms, named.
    #[cfg_attr(windows, allow(dead_code))]
    Unbuilt(&'static str),
    /// It was attempted and refused. Carries whatever said so. Windows only,
    /// for `Placed`'s reason.
    #[cfg_attr(not(windows), allow(dead_code))]
    Refused(String),
}

/// The store a client identity belongs in on Windows.
///
/// `My` is the *Personal* store, and `-user` scopes it to the person rather
/// than the machine. Both halves matter: an identity in the machine store is
/// not offered to a browser running as this user, which is a failure that looks
/// exactly like the certificate not existing.
#[cfg_attr(not(windows), allow(dead_code))]
const WINDOWS_STORE: &str = "My";

/// **Build a PKCS#12 bundle** — the format Windows imports a private key from.
///
/// A certificate and a key are two PEM files, and every Windows path for
/// installing an identity wants them as one PKCS#12 blob instead. This is the
/// step that used to be OpenSSL.
///
/// # Errors
///
/// One sentence. `p12` reports failure as `None` and says nothing about why,
/// so there is nothing more truthful to pass on.
pub fn bundle(minted: &Minted, password: &str, name: &str) -> Result<Vec<u8>, String> {
    p12::PFX::new(
        &minted.certificate_der,
        &minted.key_der,
        // **No authority in the bundle.** A PKCS#12 may carry the chain, and
        // including it here would push the root into whichever store the
        // identity lands in — `Personal`, which is the wrong place for an
        // authority and is how a trust store quietly acquires a CA nobody
        // decided to install. The root is a separate file and a separate,
        // deliberate act.
        None,
        password,
        name,
    )
    .map(|pfx| pfx.to_der())
    .ok_or_else(|| {
        "the certificate and key would not go into a PKCS#12 bundle".to_owned()
    })
}

/// **File the identity where the browser looks**, on Windows.
///
/// `certutil` is in `system32` on every Windows and needs no elevation for the
/// current user's own stores, which is why it is preferred to a dependency on
/// the certificate API: this is one command, and the alternative is linking a
/// Windows crate into a tool that otherwise has no platform code at all.
///
/// The password is passed through because the bundle was written with one; an
/// empty string is what `certutil` expects for a bundle without.
#[cfg(windows)]
pub fn place(pfx: &Path, password: &str) -> Placement {
    let output = std::process::Command::new("certutil")
        .arg("-user")
        .arg("-p")
        .arg(password)
        .arg("-importpfx")
        .arg(WINDOWS_STORE)
        .arg(pfx)
        .output();

    match output {
        Ok(done) if done.status.success() => {
            Placement::Placed(format!("Personal (certutil -user {WINDOWS_STORE})"))
        }
        Ok(done) => {
            // certutil says what it did not like, on one stream or the other.
            let said = String::from_utf8_lossy(&done.stdout);
            let complained = String::from_utf8_lossy(&done.stderr);
            let text = format!("{said}{complained}");
            let last = text
                .lines()
                .filter(|l| !l.trim().is_empty())
                .next_back()
                .unwrap_or("certutil refused and said nothing");
            Placement::Refused(last.trim().to_owned())
        }
        Err(why) => Placement::Refused(format!("certutil could not be run: {why}")),
    }
}

/// **File `root.crt` into the Trusted Root store, for this user** — used only
/// by `ca restore`.
///
/// `SPEC-05` §4 makes minting's own root install DELIBERATELY a person's own
/// double-click: trusting a root certificate is a decision a person takes,
/// and a tool that installed one silently would have taken it for them. That
/// reasoning does not apply here. **Restoring is putting back a decision the
/// operator already made once, for this exact authority** — the alternative
/// is a restore that copies the files and leaves the browser in precisely the
/// state `rivet-reset` produced, which is the failure `restore` exists to be
/// the inverse of rather than a plain copy.
///
/// Same command shape as [`place`] and the same reasoning for using it:
/// `certutil` needs no elevation for the current user's own stores.
#[cfg(windows)]
pub fn install_root(path: &Path) -> Placement {
    let output = std::process::Command::new("certutil")
        .args(["-user", "-addstore", "Root"])
        .arg(path)
        .output();
    match output {
        Ok(done) if done.status.success() => {
            Placement::Placed("Trusted Root Certification Authorities (certutil -user -addstore Root)".to_owned())
        }
        Ok(done) => {
            let said = String::from_utf8_lossy(&done.stdout);
            let complained = String::from_utf8_lossy(&done.stderr);
            let text = format!("{said}{complained}");
            let last = text
                .lines()
                .rfind(|l| !l.trim().is_empty())
                .unwrap_or("certutil refused and said nothing");
            Placement::Refused(last.trim().to_owned())
        }
        Err(why) => Placement::Refused(format!("certutil could not be run: {why}")),
    }
}

/// The unbuilt platforms' half of [`install_root`].
#[cfg(not(windows))]
pub fn install_root(_path: &Path) -> Placement {
    Placement::Unbuilt(
        "Only the Windows path is built (ADR-002's development host). On macOS: \
         `security add-trusted-cert -k login.keychain root.crt`. On Linux there is no single \
         trust store either — see `place` for the same split on the identity half.",
    )
}

/// **Confirm the private key came in with the certificate — the check
/// `SPEC-05` §4 requires and a return code cannot stand in for.**
///
/// `certutil -importpfx` reports success even when the identity it names is
/// already present in the store with no key attached: a browser then
/// silently declines to offer such a certificate, so the symptom is *"none of
/// my certificates work"* with the right one invisible among them. This asks
/// the store directly instead of trusting the exit status [`place`] returned.
///
/// **`Get-PfxCertificate` derives the thumbprint the same way Windows just
/// did on import**, so this never has to carry the certificate's raw bytes
/// through a command line to find it again — it asks the bundle what its own
/// thumbprint is and looks that address up in `Cert:\CurrentUser\My`, the
/// same store `cmd/rivet-reset`'s `store.rs` reads `HasPrivateKey` out of.
///
/// # Errors
///
/// A sentence: PowerShell could not run, the bundle's own thumbprint is not
/// in the store at all (the import did not go in), or anything else
/// PowerShell said.
#[cfg(windows)]
pub fn private_key_attached(pfx: &Path, password: &str) -> Result<bool, String> {
    // Single-quoted PowerShell strings: a literal `'` doubles to escape.
    let escaped_path = pfx.display().to_string().replace('\'', "''");
    let escaped_password = password.replace('\'', "''");
    let script = format!(
        "$ErrorActionPreference='Stop'; \
         $secure = ConvertTo-SecureString '{escaped_password}' -AsPlainText -Force; \
         $pfx = Get-PfxCertificate -FilePath '{escaped_path}' -Password $secure; \
         $held = Get-Item ('Cert:\\CurrentUser\\My\\' + $pfx.Thumbprint) -ErrorAction SilentlyContinue; \
         if ($held) {{ $held.HasPrivateKey }} else {{ 'absent' }}"
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
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
    let text = String::from_utf8_lossy(&out.stdout);
    let answer = text.trim();
    if answer.eq_ignore_ascii_case("absent") {
        return Err(
            "the certificate this bundle names is not in Cert:\\CurrentUser\\My at all — the \
             import did not go in"
                .to_owned(),
        );
    }
    Ok(answer.eq_ignore_ascii_case("true"))
}

/// The unbuilt platforms' half of [`private_key_attached`].
#[cfg(not(windows))]
pub fn private_key_attached(_pfx: &Path, _password: &str) -> Result<bool, String> {
    Err("only the Windows certificate store is built (ADR-002's development host)".to_owned())
}

/// The unbuilt platforms, which say so.
///
/// **macOS** would be `security import <pfx> -k login.keychain`, which is one
/// command and is genuinely close to built.
///
/// **Linux** is the awkward one and is why neither is claimed: there is no
/// single store. Chrome and Chromium read the NSS database at `~/.pki/nssdb`
/// (`pk12util -d sql:$HOME/.pki/nssdb -i <pfx>`), Firefox keeps *its own*
/// per-profile NSS database, and neither is the system trust store that
/// `update-ca-certificates` manages. A tool that installed into one of the
/// three and reported success would be right about a third of the time.
#[cfg(not(windows))]
pub fn place(_pfx: &Path, _password: &str) -> Placement {
    Placement::Unbuilt(
        "Only the Windows path is built (ADR-002's development host). On macOS: \
         `security import browser.pfx -k login.keychain`. On Linux there is no single \
         store — Chrome reads ~/.pki/nssdb via `pk12util -d sql:$HOME/.pki/nssdb -i \
         browser.pfx`, and Firefox keeps a separate one per profile.",
    )
}

/// Write the bundle beside the rest of the authority and file it.
///
/// The file is **kept**, not deleted after import. Windows imports the key as
/// non-exportable, so the bundle is the only copy that can travel to a second
/// computer — and an operator with two machines is the ordinary case, not an
/// edge one.
///
/// # Errors
///
/// Anything the filesystem said, with the path.
pub fn install(dir: &Path, minted: &Minted, password: &str) -> Result<Placement, String> {
    let path = write_bundle(dir, minted, password)?;
    Ok(place(&path, password))
}

/// **Write the bundle and file it nowhere** — the rehearsal path.
///
/// # Why this exists, and it is not a convenience
///
/// [`place`] shells out to `certutil`, which Windows resolves from the system
/// directory: **removing it from `PATH` does not stop it**, which was measured
/// rather than assumed. So without this function there is no way to run the
/// minting ceremony without writing into the operator's real Personal store,
/// and **every rehearsal of provisioning degrades the store it is rehearsing
/// for** — three junk `CN=Rehearsal Operator` certificates plus one more from
/// an agent's run had to be picked out of it by thumbprint and deleted by hand.
/// `SPEC-05` §2 and §9.
///
/// The bundle is still written, and it is still a private key on disk, so this
/// is a *quieter* run rather than a harmless one. What it does not do is change
/// what this computer holds.
///
/// # Errors
///
/// Anything the filesystem said, with the path.
pub fn write_bundle(
    dir: &Path,
    minted: &Minted,
    password: &str,
) -> Result<std::path::PathBuf, String> {
    let der = bundle(minted, password, "Rivet operator")?;
    let path = dir.join("browser.pfx");
    std::fs::write(&path, der).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::ca::Authority;

    /// **The bundle is the step that used to be OpenSSL**, so what is asserted
    /// is that it comes back out again — parsed, MAC verified against the
    /// password, and carrying the certificate that went in.
    ///
    /// Both passwords are exercised because the empty one is the path a person
    /// who just presses Enter takes, and it is a different code path in the
    /// PBE: it must still produce something Windows will import.
    ///
    /// # The first version of this test asserted something false
    ///
    /// It looked for `certificate_der` as a byte window in the output and
    /// failed. **That was the test being wrong, not the bundle** — a PKCS#12
    /// encrypts its certificate bag as well as its key bag, so the certificate
    /// is not supposed to be findable in the clear. Reading it back with the
    /// password is the only honest way to ask the question, and it is a
    /// stronger one.
    #[test]
    fn an_identity_becomes_a_pkcs12_bundle_with_or_without_a_password() {
        let authority = Authority::root("test-fleet", &["local".to_owned()]).expect("a root");
        let identity = authority.issue(&[], "operator").expect("a leaf");

        for password in ["", "a password"] {
            let der = bundle(&identity, password, "Rivet operator").expect("a bundle");
            assert_eq!(der.first(), Some(&0x30), "a PKCS#12 is a DER SEQUENCE");

            let read = p12::PFX::parse(&der).expect("it parses as PKCS#12");
            let certificates = read.cert_bags(password).expect("the bags open");
            assert_eq!(
                certificates,
                vec![identity.certificate_der.clone()],
                "the bundle must carry exactly the certificate that went in, and no authority"
            );
        }
    }
}
