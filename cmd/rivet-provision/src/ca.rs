//! **The certificate authority.** The one new credential system this direction
//! needs, and the reason it needs one.
//!
//! # Why a CA at all, when Rivet already has certificates
//!
//! `rivet-tls` already presents X.509 — a **self-signed leaf pinned by the
//! Ed25519 key inside it**, and `ADR-035` §7.2 makes that deliberate: there is
//! no authority to chain to, because the pin *is* the trust.
//!
//! **A browser will not pin.** It only trusts chains. So the web interface —
//! and only the web interface — needs something Rivet has never had.
//!
//! # The certificates
//!
//! | | Signed by | Held by |
//! | :-- | :-- | :-- |
//! | **root** | itself | this workstation |
//! | **server** | the root | a fleet server |
//! | **browser** | the root | the person |
//!
//! # Name constraints are not optional here
//!
//! A CA in a browser's trust store can vouch for **any** name. If this key
//! leaks, whoever holds it can mint a certificate for a bank that the browser
//! accepts without complaint.
//!
//! `nameConstraints` limits it to a suffix. A constrained CA that leaks can
//! forge certificates for **this fleet and nothing else** — a catastrophe
//! becomes an inconvenience. Windows and Chrome both honour it, and it costs
//! one field.
//!
//! **So this module will not mint an unconstrained authority.** Not as an
//! option, not behind a flag.

//! # Why this chain is ECDSA and the control plane is Ed25519
//!
//! **Chrome does not accept Ed25519 certificates.** Found the hard way on
//! 2026-09-01: `openssl s_client` completed the handshake and served the page,
//! and Chrome answered `ERR_SSL_PROTOCOL_ERROR` — a HANDSHAKE failure, not a
//! trust one. A client restricted to `ECDSA+SHA256:RSA+SHA256` reproduces it
//! exactly, which is what proved the cause rather than guessed it.
//!
//! So the two trust systems use different curves, for different reasons:
//!
//! | | Algorithm | Because |
//! | :-- | :-- | :-- |
//! | control plane | Ed25519, pinned | `ADR-025` §2, and no browser is involved |
//! | this authority | **ECDSA P-256** | a browser has to accept it |
//!
//! That is not a compromise of `ADR-009`'s posture. It is the observation that
//! a certificate a BROWSER validates is constrained by what browsers do, and
//! Ed25519 in the web PKI is not yet one of those things.

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose,
    GeneralSubtree, IsCa, Issuer, KeyPair, KeyUsagePurpose, NameConstraints,
    PKCS_ECDSA_P256_SHA256,
};

/// How long the authority lives.
///
/// **Long, deliberately.** An expired certificate makes the web interface
/// unreachable — and the control plane still works, which is what makes that
/// survivable rather than fatal. Renewal machinery nobody runs is worse than a
/// date far enough away to be a non-event.
const ROOT_YEARS: i32 = 10;

/// Leaves outlive any hardware they are minted for.
const LEAF_YEARS: i32 = 5;

/// Certificate and key. What gets written to disk.
pub struct Minted {
    /// For a trust store, or a server. PEM.
    pub certificate: String,
    /// **The secret**, and the only copy. PEM.
    pub key: String,
    /// The same certificate, DER.
    ///
    /// **Carried rather than re-derived.** A PKCS#12 bundle is DER all the way
    /// down, and stripping PEM armour back off to get here would be parsing
    /// what this module had just printed — `ADR-011`'s second representation,
    /// for no gain. `rcgen` has both in hand at the moment of minting.
    pub certificate_der: Vec<u8>,
    /// The same key, PKCS#8 DER — what `p12` wants.
    pub key_der: Vec<u8>,
}

/// **Issue a leaf from a root already on disk** — `PLAN.md`'s bootstrap
/// problem, in one function.
///
/// A fleet server's certificate cannot be minted when the authority is, because
/// **the authority exists before the machine does**. Its name is chosen while
/// provisioning, which is after. So the root is written once and comes back
/// here when there is a hostname to put in a certificate.
///
/// **Every leaf this function mints is a machine's, and names that machine.**
/// `rivet-fleetd` and `rivet-proxyd` tell the operator from a machine by
/// exactly that (`rivetos_tls::classify_caller`, `.rulings/` R-187): a client
/// certificate naming no host is the operator. So an empty name list, or a
/// blank name in it, is refused here rather than minted into a leaf that would
/// pass as the operator everywhere its authority is trusted.
///
/// # Errors
///
/// [`Error::NamesNoHost`] for an empty or blank name; whatever `rcgen` says
/// about the PEM it was handed.
pub fn issue_from(
    root_certificate: &str,
    root_key: &str,
    dns_names: &[String],
    common: &str,
) -> Result<Minted, Error> {
    if dns_names.is_empty() || dns_names.iter().any(|name| name.trim().is_empty()) {
        return Err(Error::NamesNoHost);
    }

    // **Every name is checked against the authority's own certificate, here,
    // where both adapters pass.** `ADR-014` puts the decision in the domain
    // precisely so neither the menu nor the command line can be the one that
    // forgets — `PLAN.md` §156.2 is the record of what happens when they
    // disagree, and the failure it produced was a certificate that minted
    // cleanly and matched nothing a browser would ask for.
    let permitted = permitted_suffixes(root_certificate)?;
    if !permitted.is_empty() {
        for name in dns_names {
            if !permitted.iter().any(|s| covered_by(name, s)) {
                return Err(Error::OutsideConstraint {
                    name: name.clone(),
                    permitted,
                });
            }
        }
    }

    let signing = KeyPair::from_pem(root_key)?;
    let issuer = Issuer::from_ca_cert_pem(root_certificate, signing)?;
    leaf(&issuer, dns_names, common)
}

/// **The one shape every leaf this authority issues takes**, a machine's and
/// the operator's alike.
///
/// A chain engine that builds the path itself — Windows' does — finds the
/// issuer by key identifier and reads the leaf's purpose, so every leaf
/// carries an authority key identifier equal to the root's subject key
/// identifier, its own subject key identifier, basic constraints saying it is
/// no authority, and a critical key usage of `digitalSignature` alone.
///
/// The extended key usage follows from whether the leaf names a host. A
/// machine's leaf is `serverAuth` and `clientAuth`: it serves browsers, and its
/// proxy presents the same file to the applications behind it as a client
/// certificate (`ADR-046` §4). The operator's names none and is `clientAuth`
/// alone, because it identifies a person and serves nothing.
fn leaf<S: rcgen::SigningKey>(
    issuer: &Issuer<'_, S>,
    dns_names: &[String],
    common: &str,
) -> Result<Minted, Error> {
    let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
    let mut params = CertificateParams::new(dns_names.to_vec())?;
    params.distinguished_name = named(common);
    params.not_after = rcgen::date_time_ymd(this_year() + LEAF_YEARS, 1, 1);
    // **A leaf, and never an authority** — `cmd/cargo-rivet` says the same
    // about the identities it mints, for the same reason. `ExplicitNoCa` is
    // also what writes the subject key identifier.
    params.is_ca = IsCa::ExplicitNoCa;
    params.use_authority_key_identifier_extension = true;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = if dns_names.is_empty() {
        vec![ExtendedKeyUsagePurpose::ClientAuth]
    } else {
        vec![ExtendedKeyUsagePurpose::ServerAuth, ExtendedKeyUsagePurpose::ClientAuth]
    };
    let certificate = params.signed_by(&key, issuer)?;
    Ok(Minted {
        certificate: certificate.pem(),
        key: key.serialize_pem(),
        certificate_der: certificate.der().to_vec(),
        key_der: key.serialize_der(),
    })
}

/// The DNS suffixes an authority's name constraint permits, read from the
/// authority's own certificate.
///
/// **The certificate is the authority on what the authority permits.** There is
/// a `suffix` note written beside `root.crt` at mint time and it is a
/// convenience, not a record: `ADR-011` §3 allows one spelling of a fact, and
/// this is the one that a browser will actually enforce. The note can be
/// missing, hand-edited, or written by the repair prompt in `mint_for_host`,
/// which defaults to `local` alone — an authority permitting three suffixes
/// whose note says one issues leaves short by two names, and nothing reports it.
///
/// An empty result means the certificate constrains **nothing**, which
/// [`Authority::root`] cannot produce; a root from somewhere else might, and a
/// caller that gets an empty list has been handed a CA that can vouch for any
/// site on the internet.
///
/// # Errors
///
/// [`Error::Unreadable`] if the PEM is not a certificate, or the extension is
/// malformed.
pub fn permitted_suffixes(root_certificate: &str) -> Result<Vec<String>, Error> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(root_certificate.as_bytes())
        .map_err(|e| Error::Unreadable(e.to_string()))?;
    let (_, certificate) = x509_parser::parse_x509_certificate(&pem.contents)
        .map_err(|e| Error::Unreadable(e.to_string()))?;
    let Some(constraints) = certificate
        .name_constraints()
        .map_err(|e| Error::Unreadable(e.to_string()))?
    else {
        return Ok(Vec::new());
    };
    Ok(constraints
        .value
        .permitted_subtrees
        .iter()
        .flatten()
        .filter_map(|subtree| match subtree.base {
            x509_parser::extensions::GeneralName::DNSName(d) => Some(d.to_owned()),
            // A constraint on an IP range or an email address is not a thing
            // this tool mints, and it does not permit a DNS name either way.
            _ => None,
        })
        .collect())
}

/// Whether `name` falls under the `dNSName` subtree `suffix`, RFC 5280
/// §4.2.1.10.
///
/// The subtree covers itself and everything below it, so `local` covers
/// `arbor.local` and `local` — and does **not** cover `arbor`, which is the
/// exact shape of the defect in `PLAN.md` §156.2: `ca server arbor` produced a
/// certificate whose only name was a bare label no browser will ever match, and
/// the mint accepted it because nothing asked this question.
///
/// Comparison is ASCII-case-insensitive because DNS is, and a person typing
/// `Arbor.Local` has not made a mistake.
fn covered_by(name: &str, suffix: &str) -> bool {
    let name = name.trim_end_matches('.');
    let suffix = suffix.trim_start_matches('.').trim_end_matches('.');
    if suffix.is_empty() {
        return false;
    }
    if name.eq_ignore_ascii_case(suffix) {
        return true;
    }
    // **Split on the dot, not on the string.** `notlocal` ends with `local`
    // and is a different name owned by somebody else.
    name.len() > suffix.len() + 1
        && name.as_bytes()[name.len() - suffix.len() - 1] == b'.'
        && name[name.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

/// An authority that can sign.
///
/// **Held as parameters and a key rather than as PEM**, because signing needs
/// both and reconstructing them from text would mean parsing back what this
/// module just produced — a second representation of the same fact, which is
/// the thing `ADR-011` is about.
pub struct Authority {
    params: CertificateParams,
    key: KeyPair,
}

/// What went wrong.
#[derive(Debug)]
pub enum Error {
    /// `rcgen` refused.
    Rcgen(rcgen::Error),
    /// The suffix would constrain nothing.
    UnusableSuffix(String),
    /// A name was asked for that this authority's constraint does not cover.
    OutsideConstraint {
        /// The name that was asked for.
        name: String,
        /// Every suffix the authority permits, in the order it permits them.
        permitted: Vec<String>,
    },
    /// The authority's own certificate could not be read.
    Unreadable(String),
    /// A machine's leaf was asked for with no name to put in it.
    NamesNoHost,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Rcgen(e) => write!(f, "{e}"),
            Self::UnusableSuffix(s) => write!(
                f,
                "{s:?} would not constrain this authority to anything. A CA a browser trusts \
                 must be limited to names you control, or it can vouch for any site on the \
                 internet"
            ),
            // **The permitted list is printed, always.** A refusal that only
            // says no leaves a person guessing at a list frozen ten years ago
            // in a file they have never opened.
            Self::OutsideConstraint { name, permitted } => write!(
                f,
                "{name:?} is not under any suffix this authority permits: {}",
                permitted.join(", ")
            ),
            Self::Unreadable(why) => write!(f, "the authority's certificate could not be read: {why}"),
            Self::NamesNoHost => f.write_str(
                "a machine's certificate must name the host it is for, and no name was given. A \
                 certificate from this authority that names no host is the operator's, and \
                 rivet-fleetd and rivet-proxyd serve it as the operator (.rulings/ R-187)",
            ),
        }
    }
}

impl From<rcgen::Error> for Error {
    fn from(e: rcgen::Error) -> Self {
        Self::Rcgen(e)
    }
}

fn named(common: &str) -> DistinguishedName {
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, common);
    dn
}

/// The current year, from the clock.
fn this_year() -> i32 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    1970 + i32::try_from(secs / 31_557_600).unwrap_or(56)
}

impl Authority {
    /// **Mint the root**, constrained to `suffixes`.
    ///
    /// Each entry is a DNS suffix this authority may vouch for — `local` for a
    /// fleet reached by mDNS, or a domain you own. Anything outside all of them
    /// is refused by the verifier rather than by us, which is what makes the
    /// constraint worth having.
    ///
    /// # Why it is a LIST, ruled by the maintainer 2026-09-06
    ///
    /// Certificates must be mintable for use in an ordinary DNS environment, so
    /// the design must not assume Rivet will only ever serve mDNS hosts
    /// (.rulings/ R-339).
    ///
    /// **Today Rivet serves mDNS and only mDNS** — `rivet-mdnsd` announces
    /// `<hostname>.local` and there is no DNS server in this project. The list
    /// is not about what is supported now; it is about what cannot be changed
    /// later. **A root's name constraint is fixed for the life of the root**,
    /// and a fleet that outgrows `.local` under a single-suffix authority needs
    /// a new root re-trusted on every browser and every device that ever
    /// pinned it. Naming a second suffix at minting time costs one field and
    /// buys the whole option.
    ///
    /// So a person minting for a home fleet passes `["local"]` and is exactly
    /// where they were; a person who owns a domain passes both and never has to
    /// think about this again.
    ///
    /// # Errors
    ///
    /// [`Error::UnusableSuffix`] for an empty list, or for any entry that
    /// constrains nothing — **one bad entry refuses the whole authority**,
    /// because a permitted subtree that matches everything makes the others
    /// decorative and a warning is not a thing a person reads at 1am.
    pub fn root(name: &str, suffixes: &[String]) -> Result<Self, Error> {
        let mut permitted = Vec::new();
        let mut seen: Vec<String> = Vec::new();
        for suffix in suffixes {
            let suffix = suffix.trim().trim_start_matches('.');
            // **An empty or bare-dot suffix permits everything**, which is the
            // failure this module exists to prevent. Refused, not warned about.
            if suffix.is_empty() || !suffix.contains(|c: char| c.is_ascii_alphanumeric()) {
                return Err(Error::UnusableSuffix(suffix.to_owned()));
            }
            // A repeat is a typo rather than a second permission, and two
            // identical subtrees in one certificate is a thing somebody will
            // one day have to explain.
            if !seen.iter().any(|s| s == suffix) {
                seen.push(suffix.to_owned());
                permitted.push(GeneralSubtree::DnsName(suffix.to_owned()));
            }
        }
        if permitted.is_empty() {
            return Err(Error::UnusableSuffix(String::new()));
        }

        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
        let mut params = CertificateParams::default();
        params.distinguished_name = named(name);
        params.not_after = rcgen::date_time_ymd(this_year() + ROOT_YEARS, 1, 1);
        // Path length 1: it may sign a fleet server's intermediate, and that
        // intermediate may sign leaves. It may not sign a third level.
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(1));
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        params.name_constraints = Some(NameConstraints {
            permitted_subtrees: permitted,
            excluded_subtrees: Vec::new(),
        });
        Ok(Self { params, key })
    }

    /// The authority itself, as PEM — the file that goes into a trust store.
    ///
    /// # Errors
    ///
    /// Whatever `rcgen` says.
    pub fn pem(&self) -> Result<Minted, Error> {
        let certificate = self.params.clone().self_signed(&self.key)?;
        Ok(Minted {
            certificate: certificate.pem(),
            key: self.key.serialize_pem(),
            certificate_der: certificate.der().to_vec(),
            key_der: self.key.serialize_der(),
        })
    }

    /// **The operator's browser certificate**: a leaf for `person`, naming no
    /// host.
    ///
    /// **The absence of a name is the certificate's meaning.** A client
    /// certificate identifies a person, and a person has no hostname;
    /// `rivet-fleetd` and `rivet-proxyd` serve an operator route only to a
    /// certificate naming none (`rivetos_tls::classify_caller`, `.rulings/`
    /// R-187). Every place this tool mints a browser certificate calls this, so
    /// the shape is decided once.
    ///
    /// # Errors
    ///
    /// Whatever `rcgen` says.
    pub fn operator(&self, person: &str) -> Result<Minted, Error> {
        self.issue(&[], person)
    }

    /// **Issue a leaf** for `dns_names`.
    ///
    /// [`Authority::operator`] is this with no names. A machine's leaf comes
    /// from [`issue_from`], which refuses to mint one that names nothing.
    ///
    /// # Errors
    ///
    /// Whatever `rcgen` says.
    pub fn issue(&self, dns_names: &[String], common: &str) -> Result<Minted, Error> {
        let issuer = Issuer::new(self.params.clone(), &self.key);
        leaf(&issuer, dns_names, common)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// **The rule this module exists for.** An authority that constrains
    /// nothing is refused, because a browser would then trust it for the whole
    /// internet.
    #[test]
    fn an_unconstrained_authority_is_refused() {
        assert!(Authority::root("Rivet", &["".to_owned()]).is_err());
        assert!(Authority::root("Rivet", &[".".to_owned()]).is_err());
        assert!(Authority::root("Rivet", &["   ".to_owned()]).is_err());
        assert!(Authority::root("Rivet", &["local".to_owned()]).is_ok());
    }

    /// The root is an authority, and carries its constraint.
    #[test]
    fn the_root_is_a_constrained_authority() {
        let a = Authority::root("Rivet fleet", &["local".to_owned()]).expect("mints");
        assert!(matches!(a.params.is_ca, IsCa::Ca(_)), "the root must be a CA");
        let constraints = a
            .params
            .name_constraints
            .as_ref()
            .expect("the root carries constraints");
        assert!(
            constraints
                .permitted_subtrees
                .iter()
                .any(|s| matches!(s, GeneralSubtree::DnsName(d) if d == "local")),
            "the permitted subtree is not there"
        );
        let pem = a.pem().expect("serialises");
        assert!(pem.certificate.contains("BEGIN CERTIFICATE"));
        assert!(pem.key.contains("PRIVATE KEY"));
    }

    /// A leaf is signed by the root, is not an authority, and does not reuse
    /// its key. A leaf that could sign would make every fleet server a second
    /// CA.
    #[test]
    fn a_leaf_is_issued_and_cannot_sign() {
        let a = Authority::root("Rivet fleet", &["local".to_owned()]).expect("mints");
        let server = a
            .issue(&["fleet.local".to_owned()], "fleet")
            .expect("issues");
        assert!(server.certificate.contains("BEGIN CERTIFICATE"));
        assert_ne!(
            server.key,
            a.pem().expect("serialises").key,
            "a leaf must not reuse the authority key"
        );
    }

    /// The server certificate and the browser certificate come from one
    /// authority, and are not each other.
    #[test]
    fn the_server_and_the_browser_are_different_leaves() {
        let a = Authority::root("Rivet fleet", &["local".to_owned()]).expect("mints");
        let server = a.issue(&["fleet.local".to_owned()], "fleet").expect("a");
        let browser = a.issue(&["someone".to_owned()], "someone").expect("b");
        assert_ne!(server.certificate, browser.certificate);
        assert_ne!(server.key, browser.key);
    }

    /// The verifier `rivet-proxyd` builds, over one root.
    ///
    /// **The point of building the real one.** The proxy's own tests mint an
    /// *unconstrained* Ed25519 authority, which is a convenient fixture and is
    /// not the certificate this program produces — so nothing in this
    /// repository was checking that a **constrained ECDSA** root, of the shape
    /// a person actually mints, admits the leaves minted beside it. This does.
    fn verifier(root: &Minted) -> std::sync::Arc<dyn rustls::server::danger::ClientCertVerifier> {
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(rustls_pki_types::CertificateDer::from(
                root.certificate_der.clone(),
            ))
            .expect("the root is a certificate");
        rustls::server::WebPkiClientVerifier::builder_with_provider(
            std::sync::Arc::new(roots),
            std::sync::Arc::new(rustls::crypto::ring::default_provider()),
        )
        .build()
        .expect("a verifier over one root")
    }

    /// Whether `leaf` would be admitted as a caller's certificate.
    fn admits(root: &Minted, leaf: &Minted) -> bool {
        verifier(root)
            .verify_client_cert(
                &rustls_pki_types::CertificateDer::from(leaf.certificate_der.clone()),
                &[],
                rustls_pki_types::UnixTime::now(),
            )
            .is_ok()
    }

    /// **A person's own name, with a space in it, is admitted by the gate.**
    ///
    /// The default is `fleet-owner`, and an operator replaces it with their own
    /// name, such as `Your Name`. That string becomes
    /// the browser certificate's `commonName`, which `rivet-proxyd` prints as
    /// *"Authenticated as: …"* — so it has to survive minting **and** the
    /// verification the proxy performs.
    ///
    /// **This is not obviously safe and that is why it is tested.** The root
    /// carries a `dNSName` name constraint, and RFC 5280 §4.2.1.10 applies
    /// `dNSName` constraints to the subject's `commonName` when the certificate
    /// has no subject alternative name and that name looks like a host. A
    /// browser certificate deliberately has no SAN — it identifies a person,
    /// not a host — so the question of what a verifier does with its `CN` is a
    /// real one, and answering it from the specification rather than from the
    /// verifier would be a guess.
    ///
    /// Asserted the day before the maintainer mints a production authority,
    /// because a root's name constraint cannot be changed afterwards.
    #[test]
    fn an_operator_named_like_a_person_is_admitted_by_the_gate() {
        let a = Authority::root("my-fleet", &["local".to_owned()]).expect("mints");
        let root = a.pem().expect("serialises");
        // No DNS names: this identifies a person. Exactly what `main` issues.
        let browser = a.operator("Your Name").expect("issues");

        assert!(
            admits(&root, &browser),
            "a certificate named for a person is refused by the verifier the proxy builds"
        );
        assert_eq!(
            rivetos_tls::subject_common_name(&browser.certificate_der).as_deref(),
            Some("Your Name"),
            "the name minted is not the name in the subject"
        );
    }

    /// **The suffix is a real constraint, and this is what it costs.**
    ///
    /// A server leaf inside the suffix verifies; one outside it does not. The
    /// second half is the important one: it is the failure a person would meet
    /// months later, having minted an authority for `.local` and then wanted a
    /// name on a domain they own — and the remedy at that point is a new root
    /// and re-trusting it everywhere.
    #[test]
    fn a_name_outside_the_suffix_is_refused_by_the_constraint() {
        let a = Authority::root("my-fleet", &["local".to_owned()]).expect("mints");
        let root = a.pem().expect("serialises");

        let inside = a
            .issue(&["arbor.local".to_owned()], "arbor.local")
            .expect("issues");
        let outside = a
            .issue(&["arbor.example.com".to_owned()], "arbor.example.com")
            .expect("issues");

        assert!(admits(&root, &inside), "a name inside the suffix must verify");
        assert!(
            !admits(&root, &outside),
            "the name constraint is not being enforced — a leaked root could then \
             vouch for anything"
        );
    }

    /// **An ordinary DNS name works, and so does the mDNS one, from one root.**
    ///
    /// The maintainer's ruling of 2026-09-06: certificates must be mintable for
    /// an ordinary DNS environment, and nothing may assume only mDNS hosts are
    /// ever served (.rulings/ R-339).
    ///
    /// This is the test that says the assumption is gone. One authority,
    /// permitted `local` and a real domain, admits a leaf under either — and
    /// still refuses a third name, so the constraint has been widened rather
    /// than abandoned.
    #[test]
    fn one_authority_serves_mdns_and_ordinary_dns_and_still_constrains() {
        let a = Authority::root(
            "my-fleet",
            &["local".to_owned(), "lindell.house".to_owned()],
        )
        .expect("mints");
        let root = a.pem().expect("serialises");

        for name in ["arbor.local", "arbor.lindell.house"] {
            let leaf = a.issue(&[name.to_owned()], name).expect("issues");
            assert!(admits(&root, &leaf), "{name} must verify under this root");
        }

        let elsewhere = a
            .issue(&["arbor.example.com".to_owned()], "arbor.example.com")
            .expect("issues");
        assert!(
            !admits(&root, &elsewhere),
            "widening the constraint must not remove it"
        );
    }

    /// **One certificate carries every name the host may be reached by.**
    ///
    /// An authority permitted two suffixes is only useful if a server's leaf
    /// carries both: a certificate for `arbor.local` alone would mean the root
    /// *could* vouch for the DNS name and the machine still could not present
    /// it. A browser matches what it dialled against the SANs, so both names in
    /// one leaf is what makes the second suffix worth having.
    #[test]
    fn a_server_leaf_may_carry_every_name_and_all_of_them_verify() {
        let a = Authority::root(
            "my-fleet",
            &["local".to_owned(), "lindell.house".to_owned()],
        )
        .expect("mints");
        let root = a.pem().expect("serialises");

        let both = a
            .issue(
                &["arbor.local".to_owned(), "arbor.lindell.house".to_owned()],
                "arbor.local",
            )
            .expect("issues");

        assert!(admits(&root, &both), "a leaf naming both must verify");
        assert_eq!(
            rivetos_tls::subject_common_name(&both.certificate_der).as_deref(),
            Some("arbor.local"),
            "the subject is the first name, which is the one a person types"
        );
    }

    /// **RFC 8375's namespace works as a constraint, subdomain and all.**
    ///
    /// The default offered at minting is `local` plus `<fleet>.home.arpa`, and
    /// a *subdomain* used as a permitted subtree is not the same shape as a
    /// bare TLD — a verifier has to accept `arbor.my-fleet.home.arpa` under
    /// `my-fleet.home.arpa` and refuse a sibling fleet's. Asserted rather than
    /// reasoned about, because this is what a person will actually mint.
    #[test]
    fn a_subdomain_of_the_reserved_namespace_constrains_correctly() {
        let a = Authority::root(
            "my-fleet",
            &["local".to_owned(), "my-fleet.home.arpa".to_owned()],
        )
        .expect("mints");
        let root = a.pem().expect("serialises");

        for name in ["arbor.local", "arbor.my-fleet.home.arpa"] {
            let leaf = a.issue(&[name.to_owned()], name).expect("issues");
            assert!(admits(&root, &leaf), "{name} must verify");
        }

        // Another fleet's subdomain of the same reserved namespace. This is the
        // one that would matter if two people on one network each minted a
        // root: neither may vouch for the other's names.
        let neighbour = a
            .issue(
                &["arbor.their-fleet.home.arpa".to_owned()],
                "arbor.their-fleet.home.arpa",
            )
            .expect("issues");
        assert!(
            !admits(&root, &neighbour),
            "one fleet's authority must not reach another fleet's namespace"
        );
    }

    /// **One unusable entry refuses the whole authority.** A permitted subtree
    /// that matches everything makes every other entry decorative, so this is a
    /// refusal rather than a warning — a warning is not a thing anybody reads
    /// while minting a root they will trust for ten years.
    #[test]
    fn a_list_is_only_as_constrained_as_its_worst_entry() {
        assert!(Authority::root("Rivet", &[]).is_err(), "an empty list");
        assert!(
            Authority::root("Rivet", &["local".to_owned(), String::new()]).is_err(),
            "an empty entry beside a good one"
        );
        assert!(
            Authority::root("Rivet", &["local".to_owned(), ".".to_owned()]).is_err(),
            "a bare dot beside a good one"
        );
    }

    /// The same suffix twice is a typo, not a second permission — and two
    /// identical subtrees in one certificate is a thing somebody would later
    /// have to explain.
    #[test]
    fn a_repeated_suffix_is_recorded_once() {
        let a = Authority::root(
            "Rivet",
            &["local".to_owned(), ".local".to_owned(), "local".to_owned()],
        )
        .expect("mints");
        let permitted = &a
            .params
            .name_constraints
            .as_ref()
            .expect("constraints")
            .permitted_subtrees;
        assert_eq!(permitted.len(), 1, "{permitted:?}");
    }

    /// Two roots are two different keys. Deterministic output would mean every
    /// fleet in the world shared one.
    #[test]
    fn every_root_is_its_own_key() {
        let a = Authority::root("Rivet", &["local".to_owned()]).expect("mints").pem().expect("pem");
        let b = Authority::root("Rivet", &["local".to_owned()]).expect("mints").pem().expect("pem");
        assert_ne!(a.key, b.key);
    }

    // -----------------------------------------------------------------------
    // Can a root be rolled over? — `PLAN.md` §149, measured 2026-09-06
    // -----------------------------------------------------------------------

    /// A verifier over **two** roots, which is the overlapping-trust window.
    fn verifier_over(roots_der: &[Vec<u8>]) -> std::sync::Arc<dyn rustls::server::danger::ClientCertVerifier>
    {
        let mut roots = rustls::RootCertStore::empty();
        for der in roots_der {
            roots
                .add(rustls_pki_types::CertificateDer::from(der.clone()))
                .expect("a root");
        }
        rustls::server::WebPkiClientVerifier::builder_with_provider(
            std::sync::Arc::new(roots),
            std::sync::Arc::new(rustls::crypto::ring::default_provider()),
        )
        .build()
        .expect("a verifier")
    }

    fn admitted_by(roots_der: &[Vec<u8>], leaf: &Minted) -> bool {
        verifier_over(roots_der)
            .verify_client_cert(
                &rustls_pki_types::CertificateDer::from(leaf.certificate_der.clone()),
                &[],
                rustls_pki_types::UnixTime::now(),
            )
            .is_ok()
    }

    /// **§149.1 question 1: overlapping trust works, and it needs no new code.**
    ///
    /// Every root store in this repository is built by looping over *every*
    /// certificate in a PEM file — `rivet-sdk`'s `serve::tls::roots`,
    /// `cmd/rivet-proxyd`'s `client_config`, and the verifier above. So a trust
    /// file carrying two concatenated roots produces a two-root store with
    /// nothing changed but the file.
    ///
    /// That is the mechanism a rollover needs: a window in which leaves signed
    /// by **either** authority are admitted, so devices can be migrated one at a
    /// time instead of all at once.
    #[test]
    fn a_store_holding_two_roots_admits_a_leaf_from_either() {
        let old = Authority::root("Rivet old", &["local".to_owned()]).expect("mints");
        let new = Authority::root("Rivet new", &["local".to_owned()]).expect("mints");
        let old_pem = old.pem().expect("pem");
        let new_pem = new.pem().expect("pem");

        let from_old = old.issue(&["a.local".to_owned()], "a").expect("a leaf");
        let from_new = new.issue(&["b.local".to_owned()], "b").expect("a leaf");

        let both = [old_pem.certificate_der.clone(), new_pem.certificate_der.clone()];
        assert!(admitted_by(&both, &from_old), "the old authority's leaf");
        assert!(admitted_by(&both, &from_new), "the new authority's leaf");

        // And each root alone still refuses the other's leaf, which is what
        // makes the two-root window a *choice* rather than an accident.
        assert!(!admitted_by(std::slice::from_ref(&old_pem.certificate_der), &from_new));
        assert!(!admitted_by(std::slice::from_ref(&new_pem.certificate_der), &from_old));
    }

    /// **§149.1 question 2, the one it said would decide the answer: a
    /// cross-signed root INHERITS the old root's name constraint.**
    ///
    /// Cross-signing is the mechanism that would make a rollover *invisible* —
    /// a new root signed by the old one reaches existing trust stores without
    /// anybody touching a device. §149.1 asked whether it composes with
    /// `nameConstraints`, *"which is the whole thing being escaped"*.
    ///
    /// **It does not, and this is the measurement rather than the reasoning.**
    /// A cross-signed certificate is verified through the old root, so RFC 5280
    /// §6.1's constraint state is the old root's: the permitted subtree still
    /// applies all the way down. A leaf under a suffix the *new* authority
    /// permits and the *old* one does not is refused.
    ///
    /// So the answer to §149.2 is narrower than it hoped: **cross-signing can
    /// roll over a root's KEY, and cannot widen its NAME CONSTRAINT.** Adding a
    /// suffix is a two-root window and a touch of every device — §149.3's
    /// caution stands for the suffix list even though question 1 succeeded.
    #[test]
    fn a_cross_signed_root_cannot_escape_the_old_roots_name_constraint() {
        let old = Authority::root("Rivet old", &["local".to_owned()]).expect("mints");
        let old_pem = old.pem().expect("pem");

        // The new authority permits a suffix the old one never did.
        let new = Authority::root("Rivet new", &["lan".to_owned()]).expect("mints");

        // Cross-sign: the NEW authority's parameters, signed by the OLD key.
        let issuer = Issuer::new(old.params.clone(), &old.key);
        let crossed = new
            .params
            .clone()
            .signed_by(&new.key, &issuer)
            .expect("the old authority signs the new one");

        // A leaf the new authority would issue under its own new suffix.
        let leaf = new.issue(&["host.lan".to_owned()], "host").expect("a leaf");

        // Presented as a chain through the cross-signed root, against a store
        // holding only the OLD root — which is every untouched device.
        let admitted = verifier_over(std::slice::from_ref(&old_pem.certificate_der))
            .verify_client_cert(
                &rustls_pki_types::CertificateDer::from(leaf.certificate_der.clone()),
                &[rustls_pki_types::CertificateDer::from(crossed.der().to_vec())],
                rustls_pki_types::UnixTime::now(),
            )
            .is_ok();

        assert!(
            !admitted,
            "a cross-signed root escaped the old root's name constraint — if this ever passes, \
             PLAN.md §149's answer changes and .lan becomes a safe default"
        );
    }

    /// **The authority is its own record of what it permits.**
    ///
    /// Everything below rests on this: if the constraint cannot be read back
    /// out of the certificate, the only record is the `suffix` note beside it,
    /// and `PLAN.md` §153 is two days of evidence about what a second spelling
    /// of one fact does.
    #[test]
    fn the_permitted_suffixes_are_read_back_out_of_the_certificate() {
        let suffixes = [
            "local".to_owned(),
            "home-fleet.home.arpa".to_owned(),
            "home-fleet.lan".to_owned(),
        ];
        let a = Authority::root("home-fleet", &suffixes).expect("mints");
        let root = a.pem().expect("serialises");

        assert_eq!(
            permitted_suffixes(&root.certificate).expect("reads back"),
            suffixes.to_vec(),
            "the list read out of the certificate is not the list that went in"
        );
    }

    /// **The defect `PLAN.md` §156.2 measured, now a refusal.**
    ///
    /// `ca server arbor` against an authority permitting `local` produced a
    /// certificate whose only subject alternative name was `DNS:arbor` — a bare
    /// label no browser will ever match. It minted cleanly, and the failure
    /// arrived later, in a browser, saying nothing about where it came from.
    #[test]
    fn a_bare_label_is_refused_because_no_suffix_can_cover_it() {
        let a = Authority::root("home-fleet", &["local".to_owned()]).expect("mints");
        let minted_root = a.pem().expect("serialises");
        let (root, key) = (&minted_root.certificate, &minted_root.key);

        let refused = issue_from(root, key, &["arbor".to_owned()], "arbor");

        match refused {
            Err(Error::OutsideConstraint { name, permitted }) => {
                assert_eq!(name, "arbor");
                assert_eq!(permitted, vec!["local".to_owned()]);
            }
            // Not `{other:?}`: `Minted` holds a private key and does not derive
            // `Debug`, so a panic here cannot print one.
            Err(other) => panic!("refused for the wrong reason: {other}"),
            Ok(_) => panic!("a bare label was not refused at all"),
        }
    }

    /// **The refusal names the suffixes that would have worked.**
    ///
    /// A person minting a certificate is reading a list frozen at a mint they
    /// may have performed months ago, in a file they have never opened. "No" on
    /// its own sends them to the source.
    #[test]
    fn the_refusal_says_what_the_authority_would_have_permitted() {
        let a = Authority::root(
            "home-fleet",
            &["local".to_owned(), "home-fleet.lan".to_owned()],
        )
        .expect("mints");
        let minted_root = a.pem().expect("serialises");
        let (root, key) = (&minted_root.certificate, &minted_root.key);

        // `expect_err` is not available: `Minted` deliberately does not derive
        // `Debug`, because it holds a private key and a panic message is a
        // place a key must never appear.
        let Err(refused) = issue_from(
            root,
            key,
            &["arbor.example.com".to_owned()],
            "arbor.example.com",
        ) else {
            panic!("a name outside the constraint must be refused");
        };
        let why = refused.to_string();

        assert!(why.contains("arbor.example.com"), "{why}");
        assert!(why.contains("local"), "{why}");
        assert!(why.contains("home-fleet.lan"), "{why}");
    }

    /// **One bad name in a list refuses the whole certificate.**
    ///
    /// Minting the good names and dropping the bad one would hand back a
    /// certificate that is not the one asked for, and the person would find out
    /// when the name they cared about did not work.
    #[test]
    fn a_list_carrying_one_name_outside_the_constraint_is_refused_entirely() {
        let a = Authority::root("home-fleet", &["local".to_owned()]).expect("mints");
        let minted_root = a.pem().expect("serialises");
        let (root, key) = (&minted_root.certificate, &minted_root.key);

        let names = [
            "arbor.local".to_owned(),
            "arbor.example.com".to_owned(),
            "cedar.local".to_owned(),
        ];
        assert!(
            issue_from(root, key, &names, "arbor.local").is_err(),
            "two good names carried a third past the constraint"
        );
    }

    /// **Every permitted name at once still mints**, which is the case the
    /// whole feature exists for: one certificate a machine presents over mDNS
    /// and over DNS both.
    #[test]
    fn every_name_under_the_constraint_mints_in_one_certificate() {
        let suffixes = [
            "local".to_owned(),
            "home-fleet.home.arpa".to_owned(),
            "home-fleet.lan".to_owned(),
        ];
        let a = Authority::root("home-fleet", &suffixes).expect("mints");
        let minted_root = a.pem().expect("serialises");
        let (root, key) = (&minted_root.certificate, &minted_root.key);

        let names: Vec<String> = suffixes.iter().map(|s| format!("arbor.{s}")).collect();
        let minted = issue_from(root, key, &names, &names[0]).expect("issues");

        assert!(
            admits(&minted_root, &minted),
            "the leaf must verify against its own root"
        );
    }

    // -----------------------------------------------------------------------
    // The operator is a certificate that names no host — `.rulings/` R-187 D1
    // -----------------------------------------------------------------------

    /// The DNS names in a certificate's `subjectAltName`, read by `x509-parser`
    /// — a reader independent of the walk `rivetos_tls::classify_caller` makes,
    /// so the two are held against each other rather than one against itself.
    /// `None` when the certificate carries no `subjectAltName` at all.
    fn alternative_names(der: &[u8]) -> Option<Vec<String>> {
        let (_, certificate) = x509_parser::parse_x509_certificate(der).expect("a certificate");
        let extension = certificate
            .subject_alternative_name()
            .expect("a readable subjectAltName")?;
        Some(
            extension
                .value
                .general_names
                .iter()
                .map(|name| match name {
                    x509_parser::extensions::GeneralName::DNSName(d) => (*d).to_owned(),
                    other => format!("{other:?}"),
                })
                .collect(),
        )
    }

    /// **The pin: every leaf `issue_from` mints names a host**, by both readers,
    /// for every shape of name list the callers compose — one suffix, several,
    /// RFC 8375's subdomain, and a literal list as `ca server --names` passes it.
    /// `stage_tests` and `main`'s tests hold each caller's own output to the same
    /// rule; this holds the function they all go through.
    #[test]
    fn every_leaf_issue_from_mints_names_a_host() {
        for suffixes in [
            vec!["local".to_owned()],
            vec!["local".to_owned(), "my-fleet.home.arpa".to_owned()],
            vec![
                "local".to_owned(),
                "home-fleet.home.arpa".to_owned(),
                "home-fleet.lan".to_owned(),
            ],
        ] {
            let a = Authority::root("my-fleet", &suffixes).expect("mints");
            let root = a.pem().expect("serialises");
            let composed: Vec<String> = suffixes.iter().map(|s| format!("rivet-02.{s}")).collect();
            let literal = vec![format!("gateway.{}", suffixes[0])];
            for names in [composed, literal] {
                let leaf = issue_from(&root.certificate, &root.key, &names, &names[0])
                    .expect("issues");
                assert_eq!(
                    rivetos_tls::classify_caller(&leaf.certificate_der),
                    rivetos_tls::Caller::Machine(names.clone()),
                    "{names:?}"
                );
                assert_eq!(
                    alternative_names(&leaf.certificate_der),
                    Some(names.clone()),
                    "{names:?}"
                );
            }
        }
    }

    /// **A machine's leaf that names nothing is never minted**, because it
    /// would pass as the operator wherever this authority is trusted.
    #[test]
    fn issue_from_refuses_a_leaf_that_names_no_host() {
        let a = Authority::root("my-fleet", &["local".to_owned()]).expect("mints");
        let root = a.pem().expect("serialises");
        for names in [
            Vec::new(),
            vec![String::new()],
            vec!["rivet-02.local".to_owned(), "  ".to_owned()],
        ] {
            match issue_from(&root.certificate, &root.key, &names, "rivet-02.local") {
                Err(Error::NamesNoHost) => {}
                Err(other) => panic!("{names:?} refused for the wrong reason: {other}"),
                Ok(_) => panic!("{names:?} minted a machine leaf naming no host"),
            }
        }
        let why = Error::NamesNoHost.to_string();
        assert!(why.contains("operator"), "{why}");
    }

    /// **The operator's certificate names no host, and is the operator**, by
    /// both readers, whatever the person typed — including a name shaped like a
    /// host, because nothing reads the subject for this.
    #[test]
    fn the_operator_certificate_names_no_host_and_is_the_operator() {
        let a = Authority::root("my-fleet", &["local".to_owned()]).expect("mints");
        for person in ["Your Name", "fleet-owner", "your.name", "operator"] {
            let browser = a.operator(person).expect("a browser certificate");
            assert_eq!(alternative_names(&browser.certificate_der), None, "{person:?}");
            assert_eq!(
                rivetos_tls::classify_caller(&browser.certificate_der),
                rivetos_tls::Caller::Operator,
                "{person:?}"
            );
            assert_eq!(
                rivetos_tls::subject_common_name(&browser.certificate_der).as_deref(),
                Some(person)
            );
        }
    }

    /// The standard leaf fields, read back by `x509-parser`:
    /// `(authority key id, subject key id, (KU critical, digitalSignature,
    /// keyCertSign), (serverAuth, clientAuth))`.
    type Fields = (
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<(bool, bool, bool)>,
        Option<(bool, bool)>,
    );

    fn fields(der: &[u8]) -> Fields {
        use x509_parser::extensions::ParsedExtension;
        let (_, certificate) = x509_parser::parse_x509_certificate(der).expect("a certificate");
        let mut out: Fields = (None, None, None, None);
        for extension in certificate.extensions() {
            match extension.parsed_extension() {
                ParsedExtension::AuthorityKeyIdentifier(aki) => {
                    out.0 = aki.key_identifier.as_ref().map(|k| k.0.to_vec());
                }
                ParsedExtension::SubjectKeyIdentifier(ski) => out.1 = Some(ski.0.to_vec()),
                ParsedExtension::KeyUsage(ku) => {
                    out.2 = Some((extension.critical, ku.digital_signature(), ku.key_cert_sign()));
                }
                ParsedExtension::ExtendedKeyUsage(eku) => {
                    out.3 = Some((eku.server_auth, eku.client_auth));
                }
                _ => {}
            }
        }
        out
    }

    /// **A machine's leaf carries the fields a chain engine builds its path
    /// from**: an authority key identifier equal to the root's subject key
    /// identifier, its own subject key identifier, a critical key usage of
    /// digitalSignature, and serverAuth with clientAuth — clientAuth because a
    /// machine's proxy presents its leaf to its backends. Both ways of issuing
    /// one, from a root in hand and from a root on disk, and the leaf still
    /// verifies.
    #[test]
    fn a_machines_leaf_carries_the_standard_fields() {
        let a = Authority::root("my-fleet", &["local".to_owned()]).expect("mints");
        let root = a.pem().expect("serialises");
        let root_ski = fields(&root.certificate_der).1.expect("the root has an identifier");
        let names = vec!["rivet-02.local".to_owned()];
        for leaf in [
            issue_from(&root.certificate, &root.key, &names, &names[0]).expect("issues"),
            a.issue(&names, &names[0]).expect("issues"),
        ] {
            let (aki, ski, ku, eku) = fields(&leaf.certificate_der);
            assert_eq!(aki.as_ref(), Some(&root_ski));
            assert!(ski.is_some_and(|s| s != root_ski), "the leaf has its own identifier");
            assert_eq!(ku, Some((true, true, false)));
            assert_eq!(eku, Some((true, true)));
            assert!(admits(&root, &leaf), "the leaf must still verify as a caller");
        }
    }

    /// **The operator's certificate is a client's and nothing else**: the same
    /// identifiers and key usage, and an extended key usage of clientAuth
    /// alone, because it identifies a person and serves nothing.
    #[test]
    fn the_operator_certificate_is_for_client_authentication_only() {
        let a = Authority::root("my-fleet", &["local".to_owned()]).expect("mints");
        let root = a.pem().expect("serialises");
        let root_ski = fields(&root.certificate_der).1.expect("the root has an identifier");
        let browser = a.operator("Your Name").expect("issues");
        let (aki, ski, ku, eku) = fields(&browser.certificate_der);
        assert_eq!(aki.as_ref(), Some(&root_ski));
        assert!(ski.is_some());
        assert_eq!(ku, Some((true, true, false)));
        assert_eq!(eku, Some((false, true)));
        assert!(admits(&root, &browser), "the operator must still verify as a caller");
    }

    /// **A suffix is a subtree, not a string ending.**
    ///
    /// `notlocal` ends with `local` and belongs to somebody else. Getting this
    /// wrong is how a constraint check becomes decoration.
    #[test]
    fn a_suffix_covers_a_subtree_and_not_a_name_that_merely_ends_with_it() {
        assert!(covered_by("arbor.local", "local"));
        assert!(covered_by("a.b.arbor.local", "local"));
        assert!(covered_by("local", "local"), "the subtree covers itself");
        assert!(covered_by("Arbor.LOCAL", "local"), "DNS is case-insensitive");
        assert!(covered_by("arbor.local.", "local"), "a trailing root dot is still that name");

        assert!(!covered_by("notlocal", "local"));
        assert!(!covered_by("arbor.notlocal", "local"));
        assert!(!covered_by("arbor", "local"));
        assert!(!covered_by("local.example.com", "local"));
        assert!(!covered_by("arbor.local", ""));
    }
}
