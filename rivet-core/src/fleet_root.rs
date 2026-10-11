//! **A fleet's own certificate root, minted on its fleet server** — `ADR-063`
//! §4.2. The request a card carries, where the root rests, and what the minted
//! material is handed to.
//!
//! # The request travels in the staging bundle
//!
//! A card for a fleet-root mode fleet server carries a request rather than a
//! root: one entry of the `ADR-049` staging bundle, addressed to
//! [`REQUEST_APPLICATION`] at [`REQUEST_PATH`]. PID 1 reads the bundle on first
//! boot anyway, so the request needs no second file on the boot partition, no
//! second courier and no second blank-and-unlink. An entry addressed to PID 1 is
//! consumed by PID 1 and never reaches `/state/staging`.
//!
//! The request holds no secret. It names the fleet, the host, and the DNS
//! suffixes the root is constrained to; the machine mints everything secret
//! itself, from its own random source.
//!
//! # The suffixes are the fleet's names and nothing else
//!
//! A root in a browser's trust store vouches for every name its constraint
//! permits. So a request may name only [`fleet_suffixes`] — `local`,
//! `<fleet>.home.arpa` and `<fleet>.lan` — and at least one of them. A request
//! naming anything else is refused whole, which is also what refuses a root
//! that would permit everything: an empty suffix, a bare dot and a public
//! domain are all outside the set. `cmd/rivet-provision`'s `ca.rs` holds a
//! workstation-minted root to the same constraint.
//!
//! # The format
//!
//! Line-oriented, as the staging bundle is, so PID 1 parses it in one function
//! and a person holding the card can read it:
//!
//! ```text
//! version = 1
//! fleet = dev-fleet
//! host = dev-01
//! suffix = local
//! suffix = dev-fleet.home.arpa
//! ```
//!
//! The server's certificate carries `<host>.<suffix>` for every suffix, in the
//! order given, the first being its subject — the composition `ca server`
//! makes for a workstation-minted root.

use crate::state::SECRETS;

/// The staging bundle's addressee for a request: PID 1, which mints.
pub const REQUEST_APPLICATION: &str = "rivet-initd";

/// The path a request is staged under, beside its addressee.
pub const REQUEST_PATH: &str = "fleet-root";

/// The format's version, the first line of every request.
pub const VERSION: u32 = 1;

/// The root's certificate, under `/state/secrets` — public, and what
/// `rivet get fleet-root` hands a workstation.
pub const ROOT_CERTIFICATE_FILE: &str = "fleet-root.crt";

/// The root's private key, under `/state/secrets`. Root-owned, `0600`, and
/// read by nothing that faces a network: the proxy holds only its own leaf.
pub const ROOT_KEY_FILE: &str = "fleet-root.key";

/// The server's certificate, under `/state/secrets` beside the root it was
/// signed by.
///
/// **It rests here as well as with the programs it is handed to**, so that a
/// first boot interrupted after minting and before the hand-over hands over
/// the SAME bytes when it is retried, rather than a second certificate for the
/// same names — and so that PID 1 never has to parse a certificate back.
pub const SERVER_CERTIFICATE_FILE: &str = "fleet-root-server.crt";

/// The server certificate's private key, under `/state/secrets`.
pub const SERVER_KEY_FILE: &str = "fleet-root-server.key";

/// The request the resting material was minted for, under `/state/secrets`.
/// A later request that differs from it is refused rather than minted over.
pub const MINTED_FOR_FILE: &str = "fleet-root.request";

/// The root's certificate, as an absolute path on the machine.
#[must_use]
pub fn root_certificate_path() -> String {
    format!("{SECRETS}/{ROOT_CERTIFICATE_FILE}")
}

/// The root's private key, as an absolute path on the machine.
#[must_use]
pub fn root_key_path() -> String {
    format!("{SECRETS}/{ROOT_KEY_FILE}")
}

/// Who is handed the server's certificate, and under which names.
///
/// **One leaf for the machine, under each program's own names**, as a
/// workstation-minted leaf is staged (`ADR-049` §8): `rivet-proxyd` opens
/// `tls-cert`, `tls-key` and `tls-root`; `rivet-fleetd` opens `server.crt`,
/// `server.key` and `ca.crt`. Each program is the authority on what it opens.
pub const HANDED_TO: &[Handed] = &[
    Handed {
        application: "rivet-proxyd",
        certificate: "tls-cert",
        key: "tls-key",
        root: "tls-root",
    },
    Handed {
        application: "rivet-fleetd",
        certificate: "server.crt",
        key: "server.key",
        root: "ca.crt",
    },
];

/// One application's names for the server certificate, its key and the root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handed {
    /// The application whose data directory receives them.
    pub application: &'static str,
    /// The file the server certificate is written to.
    pub certificate: &'static str,
    /// The file the server certificate's private key is written to.
    pub key: &'static str,
    /// The file the root's certificate is written to.
    pub root: &'static str,
}

/// Every staged path the minted material occupies, as `(application, path)`.
///
/// A bundle that also carries one of these from a workstation has two
/// answers for one file, and is refused rather than resolved.
#[must_use]
pub fn handed_paths() -> Vec<(&'static str, &'static str)> {
    HANDED_TO
        .iter()
        .flat_map(|h| {
            [
                (h.application, h.certificate),
                (h.application, h.key),
                (h.application, h.root),
            ]
        })
        .collect()
}

/// The reserved namespace for home networks, RFC 8375.
pub const HOME_ARPA: &str = "home.arpa";

/// The mDNS suffix every Rivet answers under.
pub const LOCAL: &str = "local";

/// The suffixes a root for `fleet` may be constrained to: `local`,
/// `<fleet>.home.arpa` and `<fleet>.lan`, in that order.
#[must_use]
pub fn fleet_suffixes(fleet: &str) -> [String; 3] {
    [
        LOCAL.to_owned(),
        format!("{fleet}.{HOME_ARPA}"),
        format!("{fleet}.lan"),
    ]
}

/// The suffixes offered when nobody names them: `local` and
/// `<fleet>.home.arpa`. `<fleet>.lan` is accepted when named and never
/// offered, for `cmd/rivet-provision`'s reason: `.lan` is undelegated today
/// rather than reserved, and a root's constraint lasts as long as the root.
#[must_use]
pub fn default_suffixes(fleet: &str) -> Vec<String> {
    vec![LOCAL.to_owned(), format!("{fleet}.{HOME_ARPA}")]
}

/// A validated request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The fleet's name, one DNS label. The root's common name.
    pub fleet: String,
    /// The fleet server's host name, one DNS label.
    pub host: String,
    /// The DNS suffixes the root is constrained to, each one of
    /// [`fleet_suffixes`], none repeated, at least one.
    pub suffixes: Vec<String>,
}

impl Request {
    /// Build one, checking every field.
    ///
    /// # Errors
    ///
    /// [`Error`], naming the field and the rule.
    pub fn new(fleet: &str, host: &str, suffixes: &[String]) -> Result<Self, Error> {
        if !is_label(fleet) {
            return Err(Error::Fleet(fleet.to_owned()));
        }
        if !is_label(host) {
            return Err(Error::Host(host.to_owned()));
        }
        if suffixes.is_empty() {
            return Err(Error::NoSuffix);
        }
        let allowed = fleet_suffixes(fleet);
        let mut kept: Vec<String> = Vec::new();
        for suffix in suffixes {
            let suffix = suffix.trim().trim_end_matches('.').to_ascii_lowercase();
            if !allowed.contains(&suffix) {
                return Err(Error::Suffix {
                    suffix,
                    allowed: allowed.to_vec(),
                });
            }
            if kept.contains(&suffix) {
                return Err(Error::Repeated(suffix));
            }
            kept.push(suffix);
        }
        Ok(Self {
            fleet: fleet.to_owned(),
            host: host.to_owned(),
            suffixes: kept,
        })
    }

    /// Every name the server's certificate carries: `<host>.<suffix>`, in
    /// suffix order. The first is its subject.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.suffixes
            .iter()
            .map(|s| format!("{}.{s}", self.host))
            .collect()
    }

    /// The request as text, in the one spelling [`parse`] reads.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = format!(
            "# A fleet-root mode fleet server's request: rivet-initd mints this fleet's root\n\
             # on first boot, from getrandom(2), constrained to the suffixes below (ADR-063).\n\
             version = {VERSION}\nfleet = {}\nhost = {}\n",
            self.fleet, self.host
        );
        for suffix in &self.suffixes {
            out.push_str("suffix = ");
            out.push_str(suffix);
            out.push('\n');
        }
        out
    }
}

/// Read a request, refusing anything [`Request::new`] would refuse.
///
/// # Errors
///
/// [`Error`], naming the line or the field.
pub fn parse(text: &str) -> Result<Request, Error> {
    let mut version = None;
    let mut fleet = None;
    let mut host = None;
    let mut suffixes = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(Error::Line(index + 1));
        };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "version" => version = Some(value.to_owned()),
            "fleet" if fleet.is_none() => fleet = Some(value.to_owned()),
            "host" if host.is_none() => host = Some(value.to_owned()),
            "suffix" => suffixes.push(value.to_owned()),
            _ => return Err(Error::Line(index + 1)),
        }
    }
    match version.as_deref() {
        Some(v) if v == VERSION.to_string() => {}
        other => return Err(Error::Version(other.unwrap_or_default().to_owned())),
    }
    Request::new(
        fleet.as_deref().unwrap_or_default(),
        host.as_deref().unwrap_or_default(),
        &suffixes,
    )
}

/// One DNS label: lower-case ASCII letters, digits and hyphens, one to 63
/// bytes, beginning and ending with a letter or digit.
#[must_use]
pub fn is_label(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 63
        && !text.starts_with('-')
        && !text.ends_with('-')
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Why a request was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The version line is missing or names another format.
    Version(String),
    /// A line that is not `key = value` for a known key, or a key given twice.
    Line(usize),
    /// The fleet name is not one DNS label.
    Fleet(String),
    /// The host name is not one DNS label.
    Host(String),
    /// No suffix was named, and a root must be constrained to something.
    NoSuffix,
    /// A suffix outside the fleet's names.
    Suffix {
        /// What was asked for.
        suffix: String,
        /// Every suffix this fleet's root may be constrained to.
        allowed: Vec<String>,
    },
    /// A suffix named twice.
    Repeated(String),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Version(v) => write!(
                f,
                "the request's version is {v:?}, and this build reads version {VERSION}"
            ),
            Self::Line(n) => write!(
                f,
                "line {n} of the request is not `version`, `fleet`, `host` or `suffix` = value, \
                 or repeats `fleet` or `host`"
            ),
            Self::Fleet(name) => write!(
                f,
                "the fleet name {name:?} is not one DNS label (lower-case letters, digits and \
                 hyphens, at most 63, not beginning or ending with a hyphen)"
            ),
            Self::Host(name) => write!(f, "the host name {name:?} is not one DNS label"),
            Self::NoSuffix => f.write_str(
                "the request names no suffix. A root a browser trusts must be constrained to the \
                 fleet's names, or it vouches for every site on the internet",
            ),
            Self::Suffix { suffix, allowed } => write!(
                f,
                "{suffix:?} is not one of this fleet's names, and a fleet's root is constrained \
                 to its own names only: {}",
                allowed.join(", ")
            ),
            Self::Repeated(suffix) => write!(f, "the suffix {suffix:?} is named twice"),
        }
    }
}

#[cfg(test)]
#[path = "fleet_root_tests.rs"]
mod tests;
