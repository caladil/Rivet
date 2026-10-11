//! What this binary was asked to do, parsed from `argv`.
//!
//! Parsing is a decision, so it lives here and not in `main`, and it takes a
//! slice rather than reading `std::env::args` — which is what makes every
//! branch below reachable from a test on any platform.
//!
//! # What `argv` stopped carrying, and why — 2026-08-23
//!
//! ~~`--interface`, `--address`, `--gateway` and `--dhcp`.~~ **Removed.** They
//! worked, they brought a board up, and they were the wrong authority.
//!
//! `ADR-007` §3.3 makes the configuration document the statement of *what the
//! machine should be*. An address in a service's argument list is a second
//! thing that also looks authoritative, and `ADR-011` §3 is entirely about that
//! failure — *"none of these is a bad decision made once; each is the
//! accumulated result of never having said which one wins"*. The maintainer
//! ruled option 2 on 2026-08-23, so the document wins, and the
//! flags that were the other answer are gone rather than kept as a convenient
//! override. A convenience writer is the exact shape `ADR-011` §5 says to look
//! for in review.
//!
//! # What `argv` still carries: a path
//!
//! ```text
//! rivet-netd --document <path>
//! ```
//!
//! **A path is not a policy**, and that is the whole distinction. `--address
//! 192.168.1.50/24` states what this machine's address should be, which the
//! document already states; `--document /state/config/machine.toml` states
//! where to read that statement from, which the document cannot state about
//! itself. Locating an input is the composition root's job (`ADR-014` §3), and
//! `cmd/rivet-initd` already takes the same shape of decision — it is told
//! nothing and derives the boot partition from the firmware.
//!
//! Nothing is defaulted, for the reason it never was: `ADR-007` §3.6.5 and
//! `AGENTS.md` §5. A default path would mean a machine that silently converged
//! onto a document nobody named.

use std::path::PathBuf;

/// What the operator asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Converge this machine onto the desired state in the document at this
    /// path.
    Apply { document: PathBuf },
    /// Print usage and stop.
    Usage,
}

/// Why `argv` was not usable.
///
/// Three variants where there were five. `Rejected(CoreError)` and `Conflict`
/// are gone because **no value is parsed here any more** — an address that is
/// not one, and a document asking for a lease and a static address at once, are
/// both refused by `rivet-config` with a line and a column, which is strictly
/// better than what this module could say about a bare `argv` word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgError {
    Unknown(String),
    /// A flag that takes a value, given none.
    NeedsValue(&'static str),
    Missing(&'static str),
}

impl core::fmt::Display for ArgError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Unknown(arg) => write!(f, "unrecognised argument {arg:?}"),
            Self::NeedsValue(flag) => write!(f, "{flag} needs a value"),
            Self::Missing(flag) => write!(f, "{flag} is required and was not given"),
        }
    }
}

pub const USAGE: &str = "\
rivet-netd — converge this machine's interfaces onto the configuration
document, which is the only thing that says what they should be (ADR-007 §3.3).

    rivet-netd --document <path>
    rivet-netd --help

The document's [network] table names one entry per interface, each declaring
`method = \"static\"` with an `address` (and an optional `gateway`), or
`method = \"dhcp\"`.

It converges once and exits. A dhcp interface's address comes from the lease
its supervised client recorded under /run/rivet/dhcp; with no lease yet it is
left for the supervisor to run this again when one arrives (ADR-004 §8).";

/// Parse the arguments after the program name.
pub fn parse(argv: &[String]) -> Result<Request, ArgError> {
    let mut document: Option<&str> = None;

    let mut rest = argv.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(Request::Usage),
            "--document" => document = Some(value(&mut rest, "--document")?),
            other => return Err(ArgError::Unknown(other.to_string())),
        }
    }

    let document = document.ok_or(ArgError::Missing("--document"))?;
    Ok(Request::Apply {
        document: PathBuf::from(document),
    })
}

fn value<'a>(
    rest: &mut impl Iterator<Item = &'a String>,
    flag: &'static str,
) -> Result<&'a str, ArgError> {
    match rest.next() {
        // A flag where a value was expected is a missing value, not a value
        // that happens to look odd. `--document --help` must not name a file
        // `--help`.
        Some(next) if next.starts_with("--") => Err(ArgError::NeedsValue(flag)),
        Some(next) => Ok(next.as_str()),
        None => Err(ArgError::NeedsValue(flag)),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn a_document_path_parses() {
        assert_eq!(
            parse(&argv(&["--document", "machine.toml"])),
            Ok(Request::Apply {
                document: PathBuf::from("machine.toml")
            })
        );
    }

    /// The path is taken as written. It is not resolved, canonicalised or
    /// checked here — this module performs no I/O, and whether the file exists
    /// is a fact about the machine that `main` reads at the moment it needs it
    /// (`AGENTS.md` §5).
    #[test]
    fn the_path_is_taken_as_written_including_the_shapes_a_windows_host_writes() {
        for path in [
            "/state/config/machine.toml",
            "relative/path.toml",
            "C:\\Users\\x\\rivet.toml",
            "a file with spaces.toml",
            "-",
        ] {
            assert_eq!(
                parse(&argv(&["--document", path])),
                Ok(Request::Apply {
                    document: PathBuf::from(path)
                }),
                "{path:?}"
            );
        }
    }

    #[test]
    fn nothing_is_defaulted() {
        assert_eq!(parse(&argv(&[])), Err(ArgError::Missing("--document")));
    }

    #[test]
    fn a_flag_where_a_value_was_expected_is_a_missing_value() {
        assert_eq!(
            parse(&argv(&["--document"])),
            Err(ArgError::NeedsValue("--document"))
        );
        assert_eq!(
            parse(&argv(&["--document", "--help"])),
            Err(ArgError::NeedsValue("--document"))
        );
    }

    #[test]
    fn an_unrecognised_argument_is_named_rather_than_ignored() {
        assert_eq!(
            parse(&argv(&["--vlan", "7"])),
            Err(ArgError::Unknown("--vlan".to_string()))
        );
    }

    /// The removal, asserted rather than described. Each of these was a flag
    /// this binary accepted until 2026-08-23, and each is now a second
    /// authority for a fact the document owns (`ADR-011`). If one is
    /// reintroduced as a convenient override, this fails.
    #[test]
    fn the_flags_that_were_a_second_authority_are_gone_rather_than_deprecated() {
        for gone in [
            vec!["--interface", "eth0"],
            vec!["--address", "192.168.1.50/24"],
            vec!["--gateway", "192.168.1.1"],
            vec!["--dhcp"],
        ] {
            let got = parse(&argv(&gone));
            assert_eq!(
                got,
                Err(ArgError::Unknown(gone[0].to_string())),
                "{gone:?} is still accepted"
            );
        }
        // ...and the usage text must not advertise them either.
        for gone in ["--interface", "--address", "--gateway", "--dhcp"] {
            assert!(!USAGE.contains(gone), "usage still mentions {gone}");
        }
    }

    #[test]
    fn help_is_recognised_before_anything_is_required() {
        assert_eq!(parse(&argv(&["--help"])), Ok(Request::Usage));
        assert_eq!(parse(&argv(&["-h"])), Ok(Request::Usage));
    }

    /// Usage text that does not name every flag the parser accepts is worse
    /// than none, because it tells an operator a flag does not exist.
    #[test]
    fn the_usage_text_names_every_flag_the_parser_accepts() {
        for flag in ["--document", "--help"] {
            assert!(USAGE.contains(flag), "usage does not mention {flag}");
        }
        // And it must name what the document has to contain, or an operator
        // holding only this text cannot write one.
        for word in ["[network]", "method", "static", "address", "gateway", "dhcp"] {
            assert!(USAGE.contains(word), "usage does not mention {word}");
        }
    }

    #[test]
    fn every_argument_error_says_what_to_change() {
        let cases: [(ArgError, &str); 3] = [
            (ArgError::Unknown("--vlan".into()), "--vlan"),
            (ArgError::NeedsValue("--document"), "--document"),
            (ArgError::Missing("--document"), "--document"),
        ];
        for (error, expected) in cases {
            assert!(
                error.to_string().contains(expected),
                "{error:?} does not mention {expected}"
            );
        }
        // The sweep above is exhaustive rather than a sample.
        let _exhaustive = |e: ArgError| match e {
            ArgError::Unknown(_) | ArgError::NeedsValue(_) | ArgError::Missing(_) => (),
        };
    }
}
