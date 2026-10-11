//! **One control-plane identity per machine** — `.rulings/` R-189 D2.
//!
//! The Identity answer defaults to `<rivet root>/identity/<hostname>` and
//! follows the Hostname answer until the operator types one of their own
//! ([`follow_hostname`]). Nothing on disk changes while fields are edited, so
//! quitting leaves nothing behind.
//!
//! At `w`, before anything is composed or written, a directory that holds an
//! identity is checked against the register ([`check`]): a pin another
//! machine's record already carries is refused by name. A directory that holds
//! none is minted ([`mint`]) once the document is written, and before the
//! staging bundle is composed, because staging reads the machine's pin and
//! writes its peers file.
//!
//! # Why one identity per machine
//!
//! The Identity answer used to default to `identity/board` for every machine,
//! so two machines provisioned with defaults held one private key. A caller
//! pinning that key reaches whichever board answers, and the register's
//! identifier check did not see it, because a record enrolled by `rivet-fleet
//! add device` carries a randomly minted identifier rather than one derived
//! from its pin (`ADR-049` §8).

use std::path::{Path, PathBuf};

use crate::field::Field;

/// The subject a machine's certificate carries — `cargo rivet setup`'s own
/// default for the board's identity. Nothing authenticates by it.
const DEVICE_NAME: &str = "rivet-device";

/// The variable `cargo rivet provision` names the operator's register with,
/// and `rivet-fleet`'s own override for it.
const REGISTER_ENV: &str = "RIVET_FLEET_REGISTER";

/// Where a machine named `hostname` keeps its identity under `root`.
pub(crate) fn directory_for(root: &Path, hostname: &str) -> PathBuf {
    root.join("identity").join(hostname)
}

/// **Make the Identity default follow the Hostname answer.**
///
/// The default becomes the directory for the current hostname. The value moves
/// with it only while it still equals the old default, so an Identity the
/// operator typed stays exactly as typed.
pub(crate) fn follow_hostname(fields: &mut [Field], root: Option<&Path>) {
    let Some(root) = root else {
        return;
    };
    let Some(hostname) = fields
        .iter()
        .find(|f| f.label == "Hostname" && f.error.is_none())
        .map(|f| f.value.clone())
    else {
        return;
    };
    if hostname.is_empty() {
        return;
    }
    let Some(identity) = fields.iter_mut().find(|f| f.label == "Identity") else {
        return;
    };
    let default = directory_for(root, &hostname).display().to_string();
    if identity.value == identity.default {
        identity.value.clone_from(&default);
        identity.error = None;
    }
    identity.default = default;
}

/// Whether `dir` holds neither half of an identity.
///
/// **A directory holding one half is not absent.** Minting over it is how a
/// half-written identity becomes a silently replaced one — `cargo rivet
/// setup`'s own rule.
pub(crate) fn is_absent(dir: &Path) -> bool {
    !dir.join(rivetos_tls::SEED_FILE).exists() && !dir.join(rivetos_tls::CERTIFICATE_FILE).exists()
}

/// The register `cargo rivet provision` named for this run, if any.
pub(crate) fn register_root() -> Option<PathBuf> {
    std::env::var_os(REGISTER_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// **Refuse an identity another machine's record already carries.**
///
/// An absent identity passes: it is minted at `w`, and a fresh key is in no
/// record. A record under the same hostname passes too: that is the same
/// machine, and `cargo rivet provision` refuses it by name, naming
/// `--existing`.
///
/// # Errors
///
/// The identity's certificate cannot be read, the register cannot be read, or
/// the pin is on another hostname's record — named with its hostname and
/// register identifier.
pub(crate) fn check(dir: &Path, hostname: &str, register: &Path) -> Result<(), String> {
    if is_absent(dir) {
        return Ok(());
    }
    let pin = crate::stage::device_pin(dir)?;
    let loaded = rivetos_register::store::load(register)
        .map_err(|e| format!("cannot read the register at {}: {e}", register.display()))?;
    match loaded.pin_held_elsewhere(&rivetos_tls::encode_hex(&pin), hostname) {
        None => Ok(()),
        // The sentence `cmd/cargo-rivet`'s `pin_collision_refusal` prints, so
        // the interview and `--document --identity` refuse alike.
        Some(holder) => Err(format!(
            "NOTHING WAS WRITTEN. The identity given for {hostname} is already {:?}'s (register \
             id {}, in {}): its pin is on that record. Two machines holding one private key \
             answer for each other. Pick or mint another identity directory: the interview \
             mints a fresh identity at `w` in a directory that holds none.",
            holder.name,
            holder.id,
            register.display(),
        )),
    }
}

/// **Mint a machine's identity into `dir`**: `device.seed`, `device.crt`, and
/// a `peers` file naming the operator, which is what `cargo rivet setup`
/// writes beside the board's.
///
/// `operator` is the operator's identity directory; only its certificate is
/// read. Answers the new identity's pin.
///
/// # Errors
///
/// `dir` already holds an identity, the operator's certificate cannot be read,
/// or a file cannot be written.
pub(crate) fn mint(dir: &Path, operator: &Path) -> Result<rivetos_tls::Pin, String> {
    if !is_absent(dir) {
        return Err(format!(
            "{} already holds an identity, and this refuses to overwrite one: every peer that \
             pinned it would be locked out",
            dir.display()
        ));
    }
    let operator_certificate = operator.join(rivetos_tls::CERTIFICATE_FILE);
    let operator_key = std::fs::read(&operator_certificate)
        .ok()
        .and_then(|der| rivetos_tls::public_key_in_certificate(&der))
        .ok_or_else(|| {
            format!(
                "{} holds no operator certificate, so a new identity would have a peers file \
                 naming nobody and the machine would refuse its operator. `cargo rivet setup` \
                 mints the operator's identity",
                operator.display()
            )
        })?;

    let minted = crate::stage::mint_self_signed(DEVICE_NAME)?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    write_secret(&dir.join(rivetos_tls::SEED_FILE), &minted.seed)?;
    let certificate = dir.join(rivetos_tls::CERTIFICATE_FILE);
    std::fs::write(&certificate, &minted.certificate_der)
        .map_err(|e| format!("{}: {e}", certificate.display()))?;
    crate::stage::add_peer(
        &dir.join(rivetos_tls::PEERS_FILE),
        &rivetos_tls::encode_hex(&operator_key),
    )?;
    Ok(minted.public)
}

/// Write a seed as tightly as the platform allows, and never over a file —
/// `cmd/cargo-rivet`'s `identity` module, restated because that crate has no
/// library target.
#[cfg(unix)]
fn write_secret(path: &Path, seed: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    file.write_all(seed).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    file.sync_all().map_err(|e| format!("cannot flush {}: {e}", path.display()))
}

/// The same, where there is no mode to set in the `open`.
#[cfg(not(unix))]
fn write_secret(path: &Path, seed: &[u8]) -> Result<(), String> {
    use std::io::Write as _;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    file.write_all(seed).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    file.sync_all().map_err(|e| format!("cannot flush {}: {e}", path.display()))
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
