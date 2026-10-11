//! **The set of credentials a person must hold before a card is worth
//! building**, and the one place that knows what it is.
//!
//! # The defect this closes
//!
//! Every capability already existed and nothing sequenced them. `key new` mints
//! a signing seed, `identity new` mints a control-plane identity,
//! `cmd/rivet-provision` mints a certificate authority and files a browser
//! identity — and **none of them says what comes next, and nothing checked that
//! the set was complete.** So the cheapest way to lose an afternoon here was to
//! build a card in ninety seconds and discover, from a board that is headless
//! and refuses you, that its `peers` file does not carry your key.
//!
//! That failure is not recoverable over the control plane, because the control
//! plane is the thing refusing. It is recoverable only by rewriting the card.
//!
//! # What this module is, and what it deliberately is not
//!
//! It is `Path::exists` and good sentences. **It mints nothing, reads no
//! private key it does not have to, and prints no secret** — a seed's bytes
//! never leave `key.rs`, and the only private material this module opens is a
//! seed it must derive a public key from in order to answer *"is the card going
//! to trust you"*.
//!
//! It is **not** a new credential type and it does not change what an existing
//! one means. The four things below already existed, already had a producer,
//! and already had a conventional location. `.rulings/` R-151 rules how a
//! *second* developer's key is authorised — a signed delegation — and nothing
//! here touches that.
//!
//! # Where each of these lives, and why it is not a new decision
//!
//! `~/.rivet` is a tool-owned directory this project already uses:
//! `tools/card/build.sh` defaults `RIVET_BR_OUTPUT` under it, `cargo rivet
//! package` has looked for `~/.rivet/keys/rivet-app.seed` since 2026-08-30, and
//! `STATE.md`'s own board command reads `~/.rivet/identity/operator`. The three
//! paths named here are the ones already on disk; writing them down is the
//! whole change.
//!
//! **The environment still wins.** `RIVET_APP_SEED`, `RIVET_SYSTEM_SEED` and
//! `RIVET_IDENTITY` are read first, exactly as the tools that consume them do,
//! so an operator who keeps a seed elsewhere is not told it is missing.

use std::path::{Path, PathBuf};

use rivet_core::package::port::Sign as _;
use rivet_crypto::Ed25519Signer;

/// `$HOME/.rivet`, or `$USERPROFILE\.rivet`.
///
/// `None` on a host with neither, which is a real state — a container run with
/// no `HOME` — and one that must produce a sentence rather than a panic.
#[must_use]
pub fn root() -> Option<PathBuf> {
    let home = set_to_something("HOME").or_else(|| set_to_something("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".rivet"))
}

/// A variable that is set **and not empty**.
///
/// `var_os` answers `Some("")` for a variable set to nothing, and a shell
/// spells *unset* that way in practice. `main.rs` learned this the expensive
/// way; the rule is repeated here rather than imported because this module is
/// deliberately importable without the rest of the tool.
fn set_to_something(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name).filter(|v| !v.is_empty())
}

/// **Where the environment says a credential lives**, gathered once and handed
/// in rather than read inside the survey.
///
/// The tools that consume these read them, so a survey that ignored them would
/// tell an operator with a seed at `$RIVET_SYSTEM_SEED` that they have no seed.
/// It is a **parameter** and not an ambient read for the reason `AGENTS.md` §9
/// gives about naming a target: a function whose answer depends on the process
/// environment is one no test can pin, and this one is the input to a refusal.
#[derive(Debug, Clone, Default)]
pub struct Overrides {
    pub system_seed: Option<PathBuf>,
    pub app_seed: Option<PathBuf>,
    pub board_identity: Option<PathBuf>,
}

impl Overrides {
    /// What this process's environment says. The real caller's source.
    #[must_use]
    pub fn from_environment() -> Self {
        Self {
            system_seed: set_to_something(SYSTEM_SEED_ENV).map(PathBuf::from),
            app_seed: set_to_something(crate::APP_SEED_ENV).map(PathBuf::from),
            board_identity: set_to_something(IDENTITY_ENV).map(PathBuf::from),
        }
    }
}

/// **Where the application signing seed is**, when no `--signing-key` names
/// one: `$RIVET_APP_SEED`, then `<rivet root>/keys/rivet-app.seed`.
///
/// This is the one lookup. `cargo rivet package` asks it when it is given no
/// `--signing-key`, and `cargo rivet install`'s automatic publish asks it for the
/// key an artifact is verified against before it enters the fleet repository
/// (`ADR-052` §7) — so the two can never look in different places.
///
/// A path named by the environment is returned whether or not a file is there,
/// because the operator named it and the reader of the seed says what is wrong
/// with it. The default is returned only when it is a file.
///
/// # Errors
///
/// Nothing was found: the error carries the default path that was looked at,
/// or `None` when there is no `rivet_root` to compose one from, so the caller
/// can name every place in its refusal.
pub fn app_seed(over: &Overrides, rivet_root: Option<&Path>) -> Result<PathBuf, Option<PathBuf>> {
    if let Some(named) = &over.app_seed {
        return Ok(named.clone());
    }
    let fallback = rivet_root.map(|root| root.join("keys").join(APP_SEED_FILE));
    match fallback {
        Some(path) if path.is_file() => Ok(path),
        other => Err(other),
    }
}

/// The application seed's file name under `<rivet root>/keys/`.
pub const APP_SEED_FILE: &str = "rivet-app.seed";

/// **Where the system signing seed is**, when no `--signing-key` names one:
/// `$RIVET_SYSTEM_SEED`, then `<rivet root>/keys/rivet-system.seed`.
///
/// The sibling of [`app_seed`], for exactly the callers `ADR-028` §5 keeps
/// apart from it: `cargo rivet image` signs a `card` with this seed rather than
/// the application one, because the two keys exist precisely so that a machine
/// holding one cannot ship what the other is entitled to. Same search order,
/// same shape, same reason a caller cannot get the wrong lookup by accident —
/// one function per seed, not a parameter that picks between them.
///
/// # Errors
///
/// Nothing was found: the error carries the default path that was looked at,
/// or `None` when there is no `rivet_root` to compose one from.
pub fn system_seed(over: &Overrides, rivet_root: Option<&Path>) -> Result<PathBuf, Option<PathBuf>> {
    if let Some(named) = &over.system_seed {
        return Ok(named.clone());
    }
    let fallback = rivet_root.map(|root| root.join("keys").join(SYSTEM_SEED_FILE));
    match fallback {
        Some(path) if path.is_file() => Ok(path),
        other => Err(other),
    }
}

/// The system seed's file name under `<rivet root>/keys/`.
pub const SYSTEM_SEED_FILE: &str = "rivet-system.seed";

/// The variable `tools/update/build.sh` and `tools/container/exec.sh` read for
/// the system signing seed. **Named, not defaulted** — those scripts still have
/// no default and this does not give them one.
pub const SYSTEM_SEED_ENV: &str = "RIVET_SYSTEM_SEED";

/// The variable `tools/card/build.sh` reads for the device identity directory.
pub const IDENTITY_ENV: &str = "RIVET_IDENTITY";

/// Whether a missing credential stops a card being written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// Its absence makes the board unreachable or unupdatable. Refuse.
    Required,
    /// Its absence costs a feature and not the board. Say so and carry on.
    Advised,
}

/// What a credential is made of on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// One file of 32 bytes, or 64 hex characters.
    Seed,
    /// A directory holding `device.seed` and `device.crt`.
    Identity,
    /// The same, and a `peers` file naming who may reach the machine.
    EnrolledIdentity,
    /// A directory holding `root.crt`.
    Authority,
}

/// One thing a first run has to produce.
#[derive(Debug, Clone)]
pub struct Credential {
    /// What a person calls it.
    pub name: &'static str,
    /// Where this host expects it.
    pub path: PathBuf,
    pub shape: Shape,
    pub need: Need,
    /// One line: what breaks without it.
    pub why: &'static str,
    /// The command that produces it, ready to run.
    pub how: String,
    /// Whether every file [`Shape`] requires is there.
    pub present: bool,
    /// A named part of a present credential that is not there.
    pub incomplete: Option<String>,
}

impl Credential {
    /// The public half of a seed, or of an identity's certificate.
    ///
    /// **Never the private half.** For a [`Shape::Seed`] this reads the seed
    /// and derives; for an identity it reads only `device.crt`, which is what
    /// `identity show` does and for the reason stated there.
    ///
    /// # Errors
    ///
    /// Whatever the filesystem or the key material said, with the path.
    pub fn public_key(&self) -> Result<String, String> {
        match self.shape {
            Shape::Seed => {
                let seed = crate::key::read_32(&self.path, "signing seed")?;
                let signer = Ed25519Signer::from_seed(&seed);
                Ok(crate::key::hex(&signer.public_key().0))
            }
            Shape::Identity | Shape::EnrolledIdentity => {
                let at = self.path.join(rivetos_tls::CERTIFICATE_FILE);
                let der = std::fs::read(&at)
                    .map_err(|e| format!("cannot read {}: {e}", at.display()))?;
                rivetos_tls::public_key_in_certificate(&der)
                    .map(|pin| rivetos_tls::encode_hex(&pin))
                    .ok_or_else(|| {
                        format!("{} carries no Ed25519 public key", at.display())
                    })
            }
            Shape::Authority => Err(format!(
                "{} is a certificate authority and has no Ed25519 pin",
                self.path.display()
            )),
        }
    }
}

/// Every credential, and whether this host has it.
#[derive(Debug, Clone)]
pub struct Survey {
    pub items: Vec<Credential>,
}

/// The names [`Survey::take`] gives the two identities, so a caller can ask for
/// one by [`Survey::named`] without a second spelling of the string.
pub const OPERATOR_CREDENTIAL: &str = "operator control-plane identity";
/// The board's end of the same pair.
pub const BOARD_CREDENTIAL: &str = "board control-plane identity";

/// The name of the file listing who may reach a machine's control plane.
///
/// Spelled here because `rivetos-tls` does not own it — `cmd/rivet-image`'s
/// `boot::IDENTITY_PEERS_FILE` does, and this crate does not depend on that
/// one. Two spellings of one filename is what `ADR-011` is about, so the test
/// at the foot of this file asserts they agree by value.
pub const PEERS_FILE: &str = "peers";

impl Survey {
    /// Look, and report. **Writes nothing and prompts for nothing.**
    #[must_use]
    pub fn take(at: &Path, over: &Overrides) -> Self {
        let keys = at.join("keys");
        let identity = at.join("identity");

        // Each of these honours the environment variable the tool that
        // CONSUMES it honours, so an operator who keeps a seed somewhere else
        // is never told it is missing by a survey that looked in one place.
        let system_seed = over
            .system_seed
            .clone()
            .unwrap_or_else(|| keys.join("rivet-system.seed"));
        let app_seed = over
            .app_seed
            .clone()
            .unwrap_or_else(|| keys.join(APP_SEED_FILE));
        let board = over
            .board_identity
            .clone()
            .unwrap_or_else(|| identity.join("board"));

        let operator = identity.join("operator");
        let authority = at.join("ca");

        let mut items = vec![
            Credential {
                name: "system signing seed",
                how: format!("cargo rivet key new --out {}", system_seed.display()),
                path: system_seed,
                shape: Shape::Seed,
                need: Need::Required,
                why: "signs SYSTEM UPDATE payloads. Without it a board can never be updated \
                      over the wire, which is the one failure an A/B image cannot repair",
                present: false,
                incomplete: None,
            },
            Credential {
                name: "application signing seed",
                how: format!("cargo rivet key new --out {}", app_seed.display()),
                path: app_seed,
                shape: Shape::Seed,
                need: Need::Required,
                why: "signs APPLICATION and LIBRARY payloads. Deliberately not the system \
                      seed (ADR-028 §5.5): a machine that can ship an application must not \
                      be able to ship an operating system",
                present: false,
                incomplete: None,
            },
            Credential {
                name: OPERATOR_CREDENTIAL,
                how: format!(
                    "cargo rivet identity new --directory {} --name rivet-operator",
                    operator.display()
                ),
                path: operator,
                shape: Shape::Identity,
                need: Need::Required,
                why: "YOUR end of ADR-035's control plane. It is what --identity names, and \
                      its public half is what a board's peers file has to carry",
                present: false,
                incomplete: None,
            },
            Credential {
                name: BOARD_CREDENTIAL,
                how: format!(
                    "cargo rivet identity new --directory {}, then a peers file beside it",
                    board.display()
                ),
                path: board,
                shape: Shape::EnrolledIdentity,
                need: Need::Required,
                why: "the BOARD's end, written onto the card by the image build. Its peers \
                      file is the machine's whole answer to who may reach it, and a card \
                      built without one boots with rivet-mcpd refusing to start",
                present: false,
                incomplete: None,
            },
            Credential {
                name: "fleet certificate authority",
                how: "rivet-provision  (it offers to mint one on first run)".to_owned(),
                path: authority,
                shape: Shape::Authority,
                need: Need::Advised,
                why: "the web administration portal's TLS, and the client certificate a \
                      browser presents. A board is fully reachable from the command line \
                      without it",
                present: false,
                incomplete: None,
            },
        ];

        for item in &mut items {
            let (present, incomplete) = inspect(&item.path, item.shape);
            item.present = present;
            item.incomplete = incomplete;
        }
        Self { items }
    }

    /// The [`Need::Required`] credentials that are not there.
    #[must_use]
    pub fn missing_required(&self) -> Vec<&Credential> {
        self.items
            .iter()
            .filter(|c| c.need == Need::Required && !c.present)
            .collect()
    }

    /// One credential, by the name [`Survey::take`] gave it.
    #[must_use]
    pub fn named(&self, name: &str) -> Option<&Credential> {
        self.items.iter().find(|c| c.name == name)
    }

    /// **The refusal.** Every missing piece, and the command that makes it.
    ///
    /// `None` when nothing required is missing, so a caller writes
    /// `if let Some(why) = survey.refusal() { return Err(why) }` and cannot
    /// accidentally refuse a complete set.
    #[must_use]
    pub fn refusal(&self) -> Option<String> {
        let missing = self.missing_required();
        if missing.is_empty() {
            return None;
        }
        let mut out = format!(
            "NOTHING WAS WRITTEN. {} of the credentials a Rivet card is built AGAINST are not \
             on this host:\n\n",
            missing.len()
        );
        for credential in &missing {
            out.push_str(&format!("  {}\n", credential.name));
            out.push_str(&format!("    at    {}\n", credential.path.display()));
            if let Some(part) = &credential.incomplete {
                out.push_str(&format!("    but   {part}\n"));
            }
            out.push_str(&format!("    why   {}\n", credential.why));
            out.push_str(&format!("    make  {}\n\n", credential.how));
        }
        out.push_str(
            "`cargo rivet setup` does all of them, in order, and skips whatever is already \
             there. It never overwrites a key.\n\
             \n\
             THIS IS A REFUSAL AND NOT A WARNING because the image is built AGAINST these \
             keys. A card written before you hold them is a board that refuses you over a \
             control plane you cannot authenticate to, on hardware with no shell and no \
             serial console — and the only repair is to write the card again.",
        );
        Some(out)
    }

    /// What is here, for a person who asked rather than for one who was
    /// refused. **Present and absent are both listed**, because a report that
    /// prints only problems cannot be read as confirmation of anything.
    #[must_use]
    pub fn report(&self) -> String {
        let mut out = String::new();
        for credential in &self.items {
            let mark = match (credential.present, credential.need) {
                (true, _) => "have",
                (false, Need::Required) => "MISSING",
                (false, Need::Advised) => "none",
            };
            out.push_str(&format!(
                "  {mark:<8}{:<32}{}\n",
                credential.name,
                credential.path.display()
            ));
            if let Some(part) = &credential.incomplete {
                out.push_str(&format!("          {part}\n"));
            }
        }
        out
    }
}

/// Whether everything a [`Shape`] is made of is on disk, and what is not.
fn inspect(path: &Path, shape: Shape) -> (bool, Option<String>) {
    let parts: &[&str] = match shape {
        Shape::Seed => {
            return (path.is_file(), None);
        }
        Shape::Identity => &[rivetos_tls::SEED_FILE, rivetos_tls::CERTIFICATE_FILE],
        Shape::EnrolledIdentity => &[
            rivetos_tls::SEED_FILE,
            rivetos_tls::CERTIFICATE_FILE,
            PEERS_FILE,
        ],
        Shape::Authority => &["root.crt"],
    };
    let absent: Vec<&str> = parts
        .iter()
        .copied()
        .filter(|name| !path.join(name).is_file())
        .collect();
    match absent.as_slice() {
        [] => (true, None),
        // **A half-written identity is reported as such**, because "the
        // directory is missing" and "the directory is there and has no peers
        // file" send a person to two different places.
        some if some.len() == parts.len() => (false, None),
        some => (false, Some(format!("{} is not in it", some.join(" and ")))),
    }
}

/// **Does the public key of `seed` match the public key file `published`?**
///
/// This is the check behind the note `cargo rivet provision` prints. A card is
/// built with a trust store composed from `keys/*.pub` (`ADR-028` §5.5.4), and
/// a developer who minted their own seed and left the checkout's committed
/// public key in place has built a card that trusts somebody else.
///
/// # Errors
///
/// If either file cannot be read or parsed. **Both are reported as a note and
/// never as a refusal**, because a caller may have pointed the build at another
/// key entirely and this function has no way to know.
pub fn agrees(seed: &Path, published: &Path) -> Result<bool, String> {
    let bytes = crate::key::read_32(seed, "signing seed")?;
    let mine = crate::key::hex(&Ed25519Signer::from_seed(&bytes).public_key().0);
    let theirs = std::fs::read_to_string(published)
        .map_err(|e| format!("cannot read {}: {e}", published.display()))?;
    Ok(theirs.trim().eq_ignore_ascii_case(&mine))
}

#[cfg(test)]
#[path = "credentials_tests.rs"]
mod tests;
