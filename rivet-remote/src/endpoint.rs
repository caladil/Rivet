//! **`--host`, `--identity` and `--pin`** — the three flags that name a machine
//! and the trust in it, in one spelling.
//!
//! `deploy` had them first and `run` needs exactly the same three, meaning
//! exactly the same things. `ADR-011` §3's table names a CLI that owns its own
//! logic as the failure mode, and two verbs each parsing `--pin` its own way is
//! that failure at the smallest possible scale: the two would agree today and
//! diverge the first time one of them grew a check.
//!
//! So the reading, the pairing rule and the connection live here, and each verb
//! keeps its own `--host` sentence — because that flag genuinely means different
//! things to the two of them: `run` speaks the control plane and nothing else,
//! so its `--host` is always `host:port`, and `deploy` says what it does with
//! one in its own words.
//!
//! **Not a parser.** `ADR-034` §11.1 rules two parsers over one vocabulary and
//! this is neither of them — it is three flags' worth of reading, offered to a
//! loop that still belongs to the verb.

use std::path::PathBuf;

/// **`--identity` and `--pin`, as both verbs print them.**
///
/// One block rather than two copies. What each flag *is* is the same claim
/// wherever it appears; what it is *for* differs by verb, and each verb says
/// that in its own prose above or below the option list.
pub const IDENTITY_AND_PIN: &str = "\
\x20 --identity DIR     present this operator identity over ADR-035's CONTROL\n\
\x20                    PLANE. `cargo rivet identity new` writes one, and\n\
\x20                    --host is then host:port.\n\
\x20                    The port is REQUIRED and is not guessed; a machine that\n\
\x20                    was not told otherwise listens on 7777.\n\
\x20 --pin HEX          the machine's public key, 64 hex characters. REQUIRED\n\
\x20                    with --identity: ADR-025 §2 makes the pin the whole\n\
\x20                    of the trust model.\n\
\x20                    TO GET IT: `cargo rivet key <seed>` derives it from\n\
\x20                    the machine's seed file -- same binary, one verb,\n\
\x20                    nothing to install. For a fleet with a register,\n\
\x20                    `rivet-fleet get device` prints the one it holds.\n";

/// Which machine, and what this end will present to it.
#[derive(Debug, Default, Clone)]
pub struct Endpoint {
    pub host: Option<String>,
    pub identity: Option<PathBuf>,
    pub pin: Option<String>,
}

impl Endpoint {
    /// **Read one of the three flags at `index`.**
    ///
    /// `Ok(None)` means the word there is not one of them and still belongs to
    /// the caller's own loop — which is how `deploy` keeps passing everything
    /// it does not recognise through to `package`.
    ///
    /// # Errors
    ///
    /// A flag with no value after it.
    pub fn read(&mut self, args: &[String], index: usize) -> Result<Option<usize>, String> {
        let flag = args[index].as_str();
        if !matches!(flag, "--host" | "--identity" | "--pin") {
            return Ok(None);
        }
        let value = args
            .get(index + 1)
            .cloned()
            .ok_or_else(|| format!("{flag} needs a value"))?;
        match flag {
            "--host" => self.host = Some(value),
            "--identity" => self.identity = Some(PathBuf::from(value)),
            _ => self.pin = Some(value),
        }
        Ok(Some(index + 2))
    }

    /// **`ADR-025` §2's trust is mutual, so it is both or neither.**
    ///
    /// Refused here rather than at the first connection, which for `deploy`
    /// means before a payload is built.
    ///
    /// # Errors
    ///
    /// One of the two without the other.
    pub fn paired(&self) -> Result<(), String> {
        match (&self.identity, &self.pin) {
            (Some(_), None) => Err("--identity was given without --pin, so this would send a \
                                    payload to whatever answered at that address. ADR-025 §2 \
                                    makes the pin the whole of the trust model. GET IT WITH \
                                    `cargo rivet key <seed>`, which derives it from the \
                                    machine's seed file using this same binary; on a fleet \
                                    with a register, `rivet-fleet get device` prints the one \
                                    it holds."
                .to_string()),
            (None, Some(_)) => Err("--pin was given without --identity. ADR-025 §2's trust is \
                                    MUTUAL, so the machine will ask this end for a certificate \
                                    and there is none to offer. `cargo rivet identity new` \
                                    mints one."
                .to_string()),
            _ => Ok(()),
        }
    }

    /// **The control-plane transport**, when an identity was given.
    ///
    /// `Ok(None)` means no identity was given, which each verb answers in its
    /// own words — for `run` it is a refusal that verb writes itself.
    ///
    /// # Errors
    ///
    /// An identity this end cannot read, a `--host` that names no port, or a
    /// pin that is not 64 hex characters. All before anything is connected to.
    pub fn open(&self) -> Result<Option<crate::tls::Tls>, String> {
        let Some(directory) = &self.identity else {
            return Ok(None);
        };
        let host = self
            .host
            .clone()
            .ok_or("--host is required: there is nothing to connect to")?;
        let host = with_port(&host)?;
        let identity = rivetos_tls::read_identity(directory)?;
        let pins = rivetos_tls::parse_pins(
            self.pin
                .as_deref()
                .ok_or("--pin is required with --identity")?,
        )
        .map_err(|e| format!("--pin: {e}"))?;
        Ok(Some(crate::tls::Tls::new(host, identity, pins)))
    }
}

/// **`host:port`, or a refusal that names the port a board actually uses.**
///
/// # Why this is a refusal and not a default
///
/// A cold agent given this repository and a board on 2026-08-29 found **three
/// control-plane ports in three places**: the README's examples said `7423`,
/// `rivet-mcpd --help` said `127.0.0.1:7777`, and the board listened on `7777`.
/// Two of the three were wrong and nothing said which.
///
/// A tool that silently appended the default would make `--host pi` work and
/// still leave the operator with no idea what number it used. So the port stays
/// the caller's to write, and the refusal is
/// where the authoritative number is published: it is read from
/// `rivet_config::control_plane::DEFAULT_PORT`, which is the same constant
/// `DEFAULT_LISTEN` is checked against, so this message cannot be the fourth
/// spelling.
///
/// **What "names a port" means here**: the text after the last `:` is a number.
/// That is deliberately loose — a name this end cannot resolve is the operating
/// system's to refuse, not this function's, and `[::1]` (bracketed, no port)
/// correctly fails it.
fn with_port(host: &str) -> Result<String, String> {
    let has_port = host
        .rsplit_once(':')
        .is_some_and(|(before, port)| !before.is_empty() && port.parse::<u16>().is_ok());
    if has_port {
        return Ok(host.to_owned());
    }
    Err(format!(
        "`{host}` names no port, and this end will not guess one.\n\
         \x20 why  ADR-035 names no port at all; {default} is what a machine binds when its \
         document has no [control-plane] table, and a document may say otherwise. A tool that \
         appended a default silently would connect somewhere the operator never chose.\n\
         \x20 fix  write it out — `--host {host}:{default}` — or read the machine's own \
         `listen` value out of its document.",
        default = rivet_config::control_plane::DEFAULT_PORT,
    ))
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|w| (*w).to_string()).collect()
    }

    #[test]
    fn a_word_that_is_not_one_of_the_three_is_left_for_the_caller() {
        let mut endpoint = Endpoint::default();
        let args = words(&["--rvt", "x.rvt"]);
        assert_eq!(endpoint.read(&args, 0), Ok(None));
        assert!(endpoint.host.is_none());
    }

    #[test]
    fn each_of_the_three_is_read_and_consumes_its_value() {
        let mut endpoint = Endpoint::default();
        let args = words(&["--host", "pi:7777", "--identity", "./id", "--pin", "ab"]);
        assert_eq!(endpoint.read(&args, 0), Ok(Some(2)));
        assert_eq!(endpoint.read(&args, 2), Ok(Some(4)));
        assert_eq!(endpoint.read(&args, 4), Ok(Some(6)));
        assert_eq!(endpoint.host.as_deref(), Some("pi:7777"));
        assert_eq!(endpoint.identity, Some(PathBuf::from("./id")));
        assert_eq!(endpoint.pin.as_deref(), Some("ab"));
    }

    #[test]
    fn a_flag_at_the_end_with_no_value_is_refused() {
        let mut endpoint = Endpoint::default();
        let args = words(&["--pin"]);
        assert!(endpoint.read(&args, 0).is_err_and(|e| e.contains("--pin")));
    }

    /// `ADR-025` §2's trust is mutual and either half alone is a connection
    /// that was going to fail — after a payload had been built, in `deploy`'s
    /// case.
    #[test]
    fn one_half_of_the_trust_model_without_the_other_is_refused_by_name() {
        let identity_only = Endpoint {
            identity: Some(PathBuf::from("./id")),
            ..Endpoint::default()
        };
        assert!(
            identity_only
                .paired()
                .is_err_and(|e| e.contains("--identity was given without --pin"))
        );
        let pin_only = Endpoint {
            pin: Some("ab".to_string()),
            ..Endpoint::default()
        };
        assert!(
            pin_only
                .paired()
                .is_err_and(|e| e.contains("--pin was given without --identity"))
        );
        assert_eq!(Endpoint::default().paired(), Ok(()));
    }

    /// No identity is not an error here — it is the refusal each verb writes in
    /// its own words.
    #[test]
    fn no_identity_opens_no_control_plane_and_is_not_itself_a_failure() {
        assert!(Endpoint::default().open().expect("not an error").is_none());
    }

    /// **A `--host` with a port is taken exactly as written**, whatever the
    /// port is. This end chooses nothing: a document may say any address, and a
    /// tool that "corrected" one would be a second authority for where a
    /// machine listens.
    #[test]
    fn a_host_that_names_a_port_is_taken_as_it_was_written() {
        for given in ["pi:7777", "10.0.0.2:7778", "127.0.0.1:1", "[::1]:7777"] {
            assert_eq!(with_port(given).as_deref(), Ok(given));
        }
    }

    /// **A `--host` with no port is refused, and the refusal names 7777.**
    ///
    /// This is `PLAN.md` §92's fourth defect at the point it bites. Three
    /// control-plane ports existed in three places on 2026-08-29 and an
    /// operator had no way to tell which was real; the number now comes from
    /// `rivet_config::control_plane::DEFAULT_PORT`, so this message cannot
    /// become the fourth spelling.
    #[test]
    fn a_host_with_no_port_is_refused_and_the_refusal_names_the_default() {
        for given in ["pi", "192.168.0.10", "board.local", "[::1]"] {
            let refused = with_port(given).expect_err("no port was given");
            assert!(refused.contains("names no port"), "{given}: {refused}");
            assert!(
                refused.contains(&rivet_config::control_plane::DEFAULT_PORT.to_string()),
                "{given}: the refusal does not say what to write: {refused}"
            );
            // …and it shows the whole thing to type, not just the number.
            assert!(refused.contains(&format!("--host {given}:7777")), "{given}: {refused}");
        }
    }

    /// **`open` refuses before it reads a key or opens a socket.**
    ///
    /// The identity path here does not exist, so reaching the read would be a
    /// different error entirely — which is what says the port check comes
    /// first.
    #[test]
    fn a_port_less_host_is_refused_before_an_identity_is_read() {
        let endpoint = Endpoint {
            host: Some("pi".to_string()),
            identity: Some(PathBuf::from("./no-such-identity-directory")),
            pin: Some("ab".to_string()),
        };
        let refused = match endpoint.open() {
            Err(why) => why,
            Ok(_) => panic!("a host with no port must not open a transport"),
        };
        assert!(refused.contains("names no port"), "{refused}");
        assert!(!refused.contains("no-such-identity-directory"), "{refused}");
    }
}
