//! The server's end of TLS, built from what the platform handed the
//! application — `ADR-040` §5.
//!
//! # The files are per-machine and they are not in the payload
//!
//! A certificate names a machine, and a payload is copied to every machine. So
//! the certificate, its key and the fleet authority live in the application's
//! own data directory (`RIVET_APP_DATA`), placed there by provisioning or by
//! `rivet apply install-secret`. Putting them in the artifact would put one
//! machine's identity in every copy of it.
//!
//! # There is one crypto provider in this project and it is not named here
//!
//! [`provider`] delegates to `rivet-tls`, which is where it is named. That is
//! `ADR-011`, and it is not theoretical: `rivet-proxyd` wrote its own
//! `rivet_provider()` returning `ring::default_provider()` with a comment
//! saying it matched the others, which is a second answer with a note attached
//! rather than one answer.

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The provider this project uses, named once in `rivet-tls`.
#[must_use]
pub fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    rivetos_tls::provider()
}

/// A server certificate, its key, and who is allowed to present one back.
///
/// ```no_run
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let config = rivet_sdk::serve::Tls::handed("tls-cert", "tls-key")?
///     .requiring_a_caller_certificate_from("tls-root")
///     .server_config()?;
/// # Ok(()) }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tls {
    directory: PathBuf,
    certificate: String,
    key: String,
    authority: Option<String>,
    /// Whether a caller presenting NO certificate is admitted — see
    /// [`Tls::asking_for_a_caller_certificate_from`]. Meaningless without
    /// `authority`, and ignored when it is `None`.
    admit_unauthenticated: bool,
}

impl Tls {
    /// The certificate and key by name, in the directory this application was
    /// handed.
    ///
    /// # Errors
    ///
    /// [`crate::NotHanded`] when `RIVET_APP_DATA` is not set, which means this
    /// process was not started by a Rivet supervisor.
    pub fn handed(certificate: &str, key: &str) -> Result<Self, crate::NotHanded> {
        Ok(Self::in_directory(crate::app::data()?, certificate, key))
    }

    /// The same, in a directory you name.
    ///
    /// **The environment is separated from the decision** so that every branch
    /// below is testable, which is the same reason `listen::from_parts` exists.
    #[must_use]
    pub fn in_directory(directory: impl Into<PathBuf>, certificate: &str, key: &str) -> Self {
        Self {
            directory: directory.into(),
            certificate: certificate.to_owned(),
            key: key.to_owned(),
            authority: None,
            admit_unauthenticated: false,
        }
    }

    /// **Require** a client certificate minted by the authority in this file.
    ///
    /// # It is required, never merely requested
    ///
    /// `rustls` will happily build a server that asks for a certificate and
    /// serves whoever declines, and an unauthenticated administrative surface
    /// on a LAN is the failure this whole arrangement exists to avoid. There is
    /// no *optional* here on purpose: an application that wants to authenticate
    /// some other way does not call this.
    #[must_use]
    pub fn requiring_a_caller_certificate_from(mut self, authority: &str) -> Self {
        self.authority = Some(authority.to_owned());
        self.admit_unauthenticated = false;
        self
    }

    /// **Ask for a caller certificate, verify one that is presented, and ADMIT
    /// A CALLER THAT PRESENTS NONE.**
    ///
    /// The verb is the whole difference from
    /// [`Self::requiring_a_caller_certificate_from`], and it is not a
    /// softening of it: **this is not a gate.** What it buys is the ability to
    /// *know* whether the peer authenticated —
    /// `rustls::ServerConnection::peer_certificates` answers `Some` only for a
    /// caller whose chain verified against this authority — while leaving the
    /// decision of what to do about it to the code above.
    ///
    /// # There is exactly one caller and it is `ADR-046`
    ///
    /// `rivet-proxyd` terminates TLS once for every application on the machine
    /// and routes by path, so *whether an operator is required* is a property
    /// of the route and cannot be known until the request line has been read —
    /// which is after the handshake. `R-152` ruled that shape; §4.3 makes the
    /// per-route answer come from a signed manifest, and the proxy refuses a
    /// certificate-less caller on every route that did not say otherwise.
    ///
    /// **Any other caller of this is almost certainly wrong.** A server that
    /// calls this and then never checks `peer_certificates` has an
    /// unauthenticated administrative surface on a LAN and a configuration
    /// that reads as though it does not. Use
    /// [`Self::requiring_a_caller_certificate_from`] unless the decision
    /// genuinely cannot be made until after the handshake.
    #[must_use]
    pub fn asking_for_a_caller_certificate_from(mut self, authority: &str) -> Self {
        self.authority = Some(authority.to_owned());
        self.admit_unauthenticated = true;
        self
    }

    /// Build it.
    ///
    /// # Errors
    ///
    /// A sentence naming the file and what to do about it. **Every one of these
    /// happens at startup**, before a socket is served on: a server that
    /// accepted a connection and only then discovered its key was unreadable
    /// would report the failure to whoever connected, which is the one party
    /// that must not be told about it.
    pub fn server_config(&self) -> Result<rustls::ServerConfig, String> {
        let chain_pem = read(&self.directory, &self.certificate)?;
        let key_pem = read(&self.directory, &self.key)?;

        let chain: Vec<_> = rustls_pemfile::certs(&mut chain_pem.as_slice())
            .collect::<Result<_, _>>()
            .map_err(|e| format!("{}: {e}", self.certificate))?;
        if chain.is_empty() {
            return Err(format!("{} carries no certificate", self.certificate));
        }
        let key = rustls_pemfile::private_key(&mut key_pem.as_slice())
            .map_err(|e| format!("{}: {e}", self.key))?
            .ok_or_else(|| format!("{} carries no private key", self.key))?;

        let builder = rustls::ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .map_err(|e| format!("protocol versions: {e}"))?;

        let builder = match &self.authority {
            None => builder.with_no_client_auth(),
            Some(name) => {
                let roots = self.roots(name)?;
                let building = rustls::server::WebPkiClientVerifier::builder_with_provider(
                    Arc::new(roots),
                    provider(),
                );
                // The one line the two constructors differ by. It is here
                // rather than in two branches so that everything else about
                // the verifier — the authority, the provider, the refusal
                // message — is provably the same for both.
                let building = if self.admit_unauthenticated {
                    building.allow_unauthenticated()
                } else {
                    building
                };
                let verifier = building
                    .build()
                    .map_err(|e| format!("{name}: no caller could ever be verified against it: {e}"))?;
                builder.with_client_cert_verifier(verifier)
            }
        };

        builder
            .with_single_cert(chain, key)
            .map_err(|e| format!("{} and {} do not go together: {e}", self.certificate, self.key))
    }

    /// The authority a caller's certificate must chain to.
    fn roots(&self, name: &str) -> Result<rustls::RootCertStore, String> {
        let pem = read(&self.directory, name)?;
        let mut roots = rustls::RootCertStore::empty();
        for root in rustls_pemfile::certs(&mut pem.as_slice()) {
            let root = root.map_err(|e| format!("{name}: {e}"))?;
            roots.add(root).map_err(|e| format!("{name}: {e}"))?;
        }
        if roots.is_empty() {
            return Err(format!(
                "{name} carries no certificate, so requiring a caller certificate from it would \
                 refuse every caller"
            ));
        }
        Ok(roots)
    }
}

/// Read one file out of the handed directory, and say how to put it there.
fn read(directory: &Path, name: &str) -> Result<Vec<u8>, String> {
    let path = directory.join(name);
    std::fs::read(&path).map_err(|e| {
        format!(
            "{}: {e}\n\nThis file is per-machine and is deliberately not in the payload. Install \
             it with `rivet apply install-secret <application> {name} --hex <the PEM, as hex> \
             confirm`, or have provisioning mint it.",
            path.display()
        )
    })
}
