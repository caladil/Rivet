//! **A fleet-root mode fleet server mints its fleet's root on first boot** —
//! `ADR-063` §4.
//!
//! # What happens, and when
//!
//! The card's staging bundle carries a request addressed to this program
//! (`rivet_core::fleet_root`). [`take`] runs inside [`crate::staging`], after
//! the bundle is read and before any of it is split onto `/state`:
//!
//! 1. the request is taken out of the bundle — it is PID 1's, and no
//!    application's share ever holds it;
//! 2. if `/state/secrets` holds no root, one is minted with the server's
//!    certificate, from `getrandom(2)`, and the whole set is written there,
//!    the root's certificate last;
//! 3. the server's certificate, its key and the root's certificate are added
//!    to the bundle for `rivet-proxyd` and `rivet-fleetd`, under the names each
//!    of them opens, and [`crate::staging`] carries on exactly as it does for
//!    a workstation-minted leaf.
//!
//! The card's bundle is blanked and unlinked only after everything is on
//! `/state`, so a second boot finds no request and does nothing. **A root is
//! never minted over one that rests**: a retried first boot finds the set it
//! wrote and hands over the same bytes, and a request that differs from the
//! one the resting root was minted for is refused.
//!
//! # Why PID 1, and not the fleet service
//!
//! * **The root's key must not live in anything that faces a network.**
//!   `rivet-fleetd` and `rivet-proxyd` both serve browsers; a compromise of
//!   either would hand over a key that forges every name in the fleet. PID 1
//!   writes it to `/state/secrets`, root-owned and `0700`, beside the
//!   machine's own identity — which is also where `ADR-063` §7's encrypted
//!   backup takes each host's secrets from.
//! * **The proxy needs its certificate before it first starts.** It is an
//!   image service and starts on the first pass; `rivet-fleetd` is a carried
//!   application installed after PID 1 has exited, and its install may fail
//!   without stopping the machine (`ADR-050` §10). A root minted there would
//!   leave the front door failing its first starts, or never served at all.
//! * **The hand-over already exists here.** The staging bundle's split and the
//!   supervisor's delivery with each application's own uid are the path a
//!   workstation-minted leaf takes; minting at the split puts the board's leaf
//!   on the same path with nothing new downstream.
//!
//! The cost is minting code in PID 1: `rcgen` and its date library. It runs on
//! one boot of one kind of card, returns every failure as a value, and a
//! failure here refuses the staging and leaves the card's bundle for the next
//! boot — the machine still boots and its control plane still answers.

use rivet_core::fleet_root::{
    HANDED_TO, MINTED_FOR_FILE, REQUEST_APPLICATION, REQUEST_PATH, ROOT_CERTIFICATE_FILE,
    ROOT_KEY_FILE, Request, SERVER_CERTIFICATE_FILE, SERVER_KEY_FILE, handed_paths,
};
use rivet_core::staging::Staged;

use crate::kernel::{Errno, Kernel};

/// Where the root rests.
pub const SECRETS_DIR: &str = rivet_core::state::SECRETS;

/// What [`take`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The bundle carried no request. Every card but a fleet-root mode fleet
    /// server's.
    NotAsked,
    /// A root was minted this boot.
    Minted {
        /// The names the server's certificate carries.
        names: Vec<String>,
        /// SHA-256 of the root's DER.
        fingerprint: String,
    },
    /// A root minted by an earlier boot of this card was found resting, and its
    /// set was handed over again. Nothing was minted.
    Kept {
        /// The names the server's certificate carries.
        names: Vec<String>,
        /// SHA-256 of the root's DER.
        fingerprint: String,
    },
}

impl Outcome {
    /// One line for the console, or none when nothing was asked.
    #[must_use]
    pub fn summary(&self) -> Option<String> {
        match self {
            Self::NotAsked => None,
            Self::Minted { names, fingerprint } => Some(format!(
                "minted this fleet's root (SHA-256 {fingerprint}) into {SECRETS_DIR}/\
                 {ROOT_CERTIFICATE_FILE}, and its server's certificate for {}; both are handed to \
                 rivet-proxyd and rivet-fleetd through the staging below",
                names.join(", ")
            )),
            Self::Kept { names, fingerprint } => Some(format!(
                "this fleet's root (SHA-256 {fingerprint}) already rests in {SECRETS_DIR}, so \
                 NOTHING WAS MINTED; its server's certificate for {} is handed over again",
                names.join(", ")
            )),
        }
    }
}

/// Take the request out of `items`, mint or find the root, and hand the
/// server's material to the applications that open it.
///
/// # Errors
///
/// A sentence saying why nothing may be staged: a request that does not read,
/// a bundle that also carries a workstation's leaf, a random source that did
/// not answer, a resting root minted for another request, or a write that
/// failed. [`crate::staging`] leaves the card's bundle in place on any of
/// them.
pub fn take(k: &mut dyn Kernel, items: Vec<Staged>) -> Result<(Vec<Staged>, Outcome), String> {
    let (requests, mut rest): (Vec<Staged>, Vec<Staged>) = items
        .into_iter()
        .partition(|item| item.application == REQUEST_APPLICATION);
    let [request] = requests.as_slice() else {
        if requests.is_empty() {
            return Ok((rest, Outcome::NotAsked));
        }
        return Err(format!(
            "the bundle addresses {} entries to {REQUEST_APPLICATION}, and a card carries one \
             fleet-root request",
            requests.len()
        ));
    };
    if request.path != REQUEST_PATH {
        return Err(format!(
            "the bundle addresses {:?} to {REQUEST_APPLICATION}, which reads only {REQUEST_PATH:?}",
            request.path
        ));
    }
    let text = core::str::from_utf8(&request.bytes)
        .map_err(|_| "the fleet-root request is not UTF-8 text".to_owned())?;
    let parsed = rivet_core::fleet_root::parse(text)
        .map_err(|why| format!("the fleet-root request will not read: {why}"))?;
    // The canonical spelling, so a resting request compares by bytes.
    let rendered = parsed.render();

    let handed = handed_paths();
    if let Some(clash) = rest
        .iter()
        .find(|i| handed.contains(&(i.application.as_str(), i.path.as_str())))
    {
        return Err(format!(
            "the bundle carries a fleet-root request AND {}'s {} from elsewhere — two answers for \
             one file. A fleet-root card carries the request and no workstation-minted leaf",
            clash.application, clash.path
        ));
    }

    let (set, outcome) = match resting(k, &rendered)? {
        Some(set) => {
            let fingerprint = fingerprint_of(&set.root_certificate);
            (set, Outcome::Kept { names: parsed.names(), fingerprint })
        }
        None => {
            let set = mint(k, &parsed)?;
            write(k, &rendered, &set)?;
            let fingerprint = fingerprint_of(&set.root_certificate);
            (set, Outcome::Minted { names: parsed.names(), fingerprint })
        }
    };

    for to in HANDED_TO {
        for (path, bytes) in [
            (to.certificate, &set.server_certificate),
            (to.key, &set.server_key),
            (to.root, &set.root_certificate),
        ] {
            rest.push(
                Staged::new(to.application, path, bytes.as_bytes().to_vec())
                    .map_err(|why| format!("internal defect: {why}"))?,
            );
        }
    }
    Ok((rest, outcome))
}

/// The material that rests in `/state/secrets`, as PEM.
struct Set {
    root_certificate: String,
    root_key: String,
    server_certificate: String,
    server_key: String,
}

fn path(file: &str) -> String {
    format!("{SECRETS_DIR}/{file}")
}

/// The resting set, `None` when no root rests.
///
/// **The root's certificate is the marker**, written last: without it nothing
/// counts as minted and whatever else is there is overwritten. With it, every
/// other file must be there and the request it was minted for must be this
/// one, or the boot refuses rather than minting a second root.
fn resting(k: &mut dyn Kernel, request: &str) -> Result<Option<Set>, String> {
    let root_certificate = match read_text(k, ROOT_CERTIFICATE_FILE)? {
        None => return Ok(None),
        Some(text) => text,
    };
    let mut need = |file: &str| -> Result<String, String> {
        read_text(k, file)?.ok_or_else(|| {
            format!(
                "{} rests but {} does not. A root is never minted over one that may already be \
                 trusted somewhere, so NOTHING WAS MINTED; restore the file or remove the root \
                 on purpose",
                path(ROOT_CERTIFICATE_FILE),
                path(file)
            )
        })
    };
    let minted_for = need(MINTED_FOR_FILE)?;
    let set = Set {
        root_key: need(ROOT_KEY_FILE)?,
        server_certificate: need(SERVER_CERTIFICATE_FILE)?,
        server_key: need(SERVER_KEY_FILE)?,
        root_certificate,
    };
    if minted_for != request {
        return Err(format!(
            "the root resting in {SECRETS_DIR} was minted for another request than this card \
             carries, and a root is never minted over one that may already be trusted. NOTHING \
             WAS MINTED. Resting:\n{minted_for}Carried:\n{request}"
        ));
    }
    Ok(Some(set))
}

fn read_text(k: &mut dyn Kernel, file: &str) -> Result<Option<String>, String> {
    match k.read_bytes(&path(file)) {
        Err(Errno::ENOENT) => Ok(None),
        Err(errno) => Err(format!("{} could not be read: {errno}", path(file))),
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| format!("{} is not text", path(file))),
    }
}

fn mint(k: &mut dyn Kernel, request: &Request) -> Result<Set, String> {
    let now = k.unix_time_secs();
    let mut random = |len: usize| -> Result<Vec<u8>, String> {
        k.random_bytes(len)
            .map_err(|errno| format!("getrandom(2) answered {errno}"))
    };
    let minted = rivet_fleet_root::mint(request, &mut random, now).map_err(|why| why.to_string())?;
    Ok(Set {
        root_certificate: minted.root.certificate,
        root_key: minted.root.key,
        server_certificate: minted.server.certificate,
        server_key: minted.server.key,
    })
}

/// Everything to `/state/secrets`, the root's certificate last, then flushed.
fn write(k: &mut dyn Kernel, request: &str, set: &Set) -> Result<(), String> {
    k.mkdir_private(SECRETS_DIR)
        .map_err(|errno| format!("cannot create {SECRETS_DIR}: {errno}"))?;
    for (file, text) in [
        (MINTED_FOR_FILE, request),
        (SERVER_KEY_FILE, set.server_key.as_str()),
        (SERVER_CERTIFICATE_FILE, set.server_certificate.as_str()),
        (ROOT_KEY_FILE, set.root_key.as_str()),
        (ROOT_CERTIFICATE_FILE, set.root_certificate.as_str()),
    ] {
        k.write_secret(&path(file), text.as_bytes())
            .map_err(|errno| format!("cannot write {}: {errno}. NOTHING WAS STAGED", path(file)))?;
    }
    k.sync();
    Ok(())
}

fn fingerprint_of(pem: &str) -> String {
    rivet_fleet_root::der_of_pem(pem).map_or_else(
        || "unreadable".to_owned(),
        |der| rivet_fleet_root::fingerprint(&der),
    )
}

#[cfg(test)]
#[path = "fleet_root_tests.rs"]
mod tests;
