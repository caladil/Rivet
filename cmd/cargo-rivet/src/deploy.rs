//! `cargo rivet install`'s **command line**, and nothing else.
//!
//! **Renamed from `deploy` to `install` 2026-09-22** — "install" is the
//! scripted, flag-driven verb; `cargo rivet deploy` (bare) is now the
//! interactive interview that composes this verb, `secrets` and `run` for an
//! operator (`crate::fleet::interview`). The module stays `deploy.rs` and the
//! type stays [`DeployArgs`] — both are `rivet_remote`'s, shared with
//! `rivet-fleet`, and renaming either would be a second rename outside this
//! crate. Only the verb string changed.
//!
//! The sequence this verb performs — build, push, check the bytes arrived,
//! install, edit the document — moved to `rivetos-remote` on 2026-08-26.
//! `ADR-034` §11.5 is the ruling and that crate's own documentation is the
//! account; the short form is that `ADR-014` makes `cmd/*` a composition root,
//! and a composition root is not where an implementation two tools share
//! belongs.
//!
//! **What stayed is the parsing, deliberately.** `ADR-034` §11.1 rules that
//! `rivet-fleet` uses `clap` while this tool hand-tokenises — *"two parsers,
//! one vocabulary"* — and the library takes a [`DeployArgs`] rather than an
//! argv. **This is the one deployment surface**: `rivet-fleet apply deploy`
//! was removed on 2026-09-12 (`PLAN-Feature-Set-2026-09-12` §3), so nothing
//! else builds a [`DeployArgs`]. `crate::fleet::publish_then_send` takes it,
//! publishes the payload into the fleet repository, and hands it to
//! `rivet_remote::deploy::run` (`ADR-052` §7).

// Re-exported for the usage text and the parser below. The sequence itself,
// `rivet_remote::deploy::run`, is called by `crate::fleet::publish_then_send`,
// which publishes into the fleet repository first (`ADR-052` §7).
pub use rivet_remote::deploy::{DEFAULT_REMOTE_DIR, DOCUMENT_PATH, DeployArgs};

use std::path::PathBuf;

pub fn usage() -> String {
    let identity_and_pin = rivet_remote::endpoint::IDENTITY_AND_PIN;
    format!(
        "usage: cargo rivet install <device> [<package options>] [confirm]\n\
         \x20      cargo rivet install --host HOST:PORT --identity DIR --pin HEX\n\
         \x20                        [--rvt FILE.rvt | --artifact ID | <package options>]\n\
         \x20                        [--enable] [--no-wait] [confirm]\n\
         \n\
         THE SCRIPTED VERB: what an agent or a script uses. `cargo rivet deploy`\n\
         is the interview, for a person at a terminal. ONE PLANNER DRIVES BOTH, so\n\
         the steps this prints after an install are the steps the interview would\n\
         offer for the same artifact on the same machine.\n\
         \n\
         Builds an application .rvt (or takes one with --rvt), PUBLISHES it into\n\
         ADR-052's fleet repository, transfers the repository's copy into that\n\
         Rivet's own artifact store over ADR-035's control plane, checks the\n\
         bytes arrived intact, and runs `rivet apply install <digest> confirm` on\n\
         the board — the digest it just sent. An install names a digest and\n\
         never a path (ADR-052 §7). --artifact sends one the repository\n\
         already holds, and publishes nothing.\n\
         \n\
         A BARE LEADING WORD is a device name (PLAN-Feature-Set-2026-09-12 §7.1),\n\
         resolved against rivet-fleet's own register into --host, --identity and\n\
         --pin — exactly as `cargo rivet run <device>` resolves one; see that\n\
         verb's own help for what wins when you also write one of the three\n\
         explicitly, and where the identity comes from when the register does\n\
         not say.\n\
         \n\
         THE PUBLISH verifies the artifact against the public half of a signing\n\
         seed: the APPLICATION seed for everything except a system update,\n\
         found where `cargo rivet package` finds it (--signing-key for a build,\n\
         then $RIVET_APP_SEED, then ~/.rivet/keys/rivet-app.seed); a SYSTEM\n\
         UPDATE verifies against the SYSTEM seed instead ($RIVET_SYSTEM_SEED,\n\
         then ~/.rivet/keys/rivet-system.seed — `cargo rivet image`'s own\n\
         lookup, R-177 D6). A missing seed is refused before anything is sent.\n\
         Publishing an artifact the repository already holds is not an error.\n\
         An absent repository is initialised first.\n\
         \n\
         install options:\n\
         \x20 --host HOST:PORT   The address the machine's control plane is\n\
         \x20                    listening on. The port is not guessed. REQUIRED\n\
         \x20                    unless a device name supplied one.\n\
         \x20 --rvt PATH         publish and send this payload instead of building\n\
         \x20                    one. Works for a system update .rvt as well as an\n\
         \x20                    application.\n\
         \x20 --artifact ID      send an artifact already in the fleet repository,\n\
         \x20                    selected by its full BLAKE3 digest or a short\n\
         \x20                    unambiguous prefix. The explicit spelling of what\n\
         \x20                    every install does after it publishes. Mutually\n\
         \x20                    exclusive with --rvt and with every `package`\n\
         \x20                    option — `fleet --help` for the repository itself.\n\
         \x20 --at DIR           the fleet repository to publish into and select\n\
         \x20                    from. Default: ~/.rivet/fleet.\n\
         \x20 --remote-dir DIR   REFUSED unless it is {DEFAULT_REMOTE_DIR}. A machine\n\
         \x20                    decides where an artifact lands — under its own\n\
         \x20                    digest, in its artifact store — so there is no\n\
         \x20                    directory left for this to name (ADR-052 §7).\n\
         \x20                    Kept, and refused rather than ignored, so a script\n\
         \x20                    that passes it is told instead of believed.\n\
         {identity_and_pin}\
         \x20 --enable           also make the document say this service should run,\n\
         \x20                    by appending `[services.<name>] enabled = true` to\n\
         \x20                    {DOCUMENT_PATH} with `rivet apply edit`,\n\
         \x20                    BEFORE the payload is sent. Applications only.\n\
         \x20                    REFUSED, naming the commands, when the manifest\n\
         \x20                    declares per-machine material the machine does not\n\
         \x20                    hold (ADR-055 §3): install without it first.\n\
         \x20 --no-wait          return as soon as the machine has the install\n\
         \x20                    REQUEST. Without it, install WAITS FOR ITS OWN\n\
         \x20                    INSTALL: it reads `get install`'s correlation just\n\
         \x20                    before the request, waits for it to change and\n\
         \x20                    then for that install to finish (up to {patience}s),\n\
         \x20                    reports the outcome, and EXITS NON-ZERO if the\n\
         \x20                    install failed. Only for a caller that will read\n\
         \x20                    `get install` itself.\n\
         \x20 confirm            the LAST word. Without it this prints the plan and\n\
         \x20                    changes nothing, on either machine — nothing is\n\
         \x20                    published either.\n\
         \n\
         WITHOUT `confirm`, THE PLAN ENDS WITH NEEDS: what the run with `confirm`\n\
         will touch — the machine it reaches, the files it reads and which of them\n\
         hold a private key, what it writes, and what happens on the machine. The\n\
         plan connects to nothing and reads no key, so it runs where nothing has\n\
         been granted yet; an agent runs it first and asks for exactly that list.\n\
         \n\
         AFTER A SUCCESSFUL INSTALL IT PRINTS WHAT REMAINS, as exact commands in\n\
         the order to run them: a switched-off service it connects to,\n\
         per-machine material (`cargo rivet secrets`), the enable, the converge\n\
         (`apply reload`), a service restart, the rivet-proxyd restart a routed\n\
         application needs, and on a fleet server\n\
         `cargo rivet fleet sync` so rivet-fleetd holds what was installed. Run\n\
         them in that order, and nothing else. Reading them needs the artifact's\n\
         manifest, so --artifact without an application seed on this workstation\n\
         installs and says the steps could not be planned.\n\
         \n\
         SOME IDENTITY AND SOME PIN ARE ALWAYS REQUIRED — written out, or supplied\n\
         by a device name's record. ADR-025 §2's trust is mutual: a client with no\n\
         key of its own has nothing to present, and pinning nothing would be\n\
         trusting whatever answered.\n\
         \n\
         Without --rvt, every `cargo rivet package` option is accepted and\n\
         passed through, --signing-key included.\n\
         \n\
         INSTALLING DOES NOT START IT (ADR-015 §3.2.2). `enabled` is the\n\
         operator's and lives in the document; without --enable the application\n\
         is installed and idle until you say otherwise.\n\
         \n\
         It does NOT restart the board. A system update lands in the inactive\n\
         slot and `rivet apply trial-restart confirm` is the separate decision\n\
         about when a machine goes down (ADR-012 §7.1).\n",
        patience = rivet_remote::deploy::INSTALL_PATIENCE.as_secs(),
    )
}

pub fn parse(args: &[String]) -> Result<DeployArgs, String> {
    // **The three flags that name a machine are read by `rivet_remote::endpoint`**,
    // which is also what `cargo rivet run` reads them with and what opens the
    // connection for both. One spelling of `--host`, `--identity` and `--pin`
    // in this tool (`ADR-011` §3).
    let mut endpoint = rivet_remote::endpoint::Endpoint::default();
    let mut rvt: Option<PathBuf> = None;
    let mut remote_dir = DEFAULT_REMOTE_DIR.to_string();
    let mut confirmed = false;
    let mut enable = false;
    let mut wait = true;
    let mut package_args: Vec<String> = Vec::new();

    let mut index = 0;
    while index < args.len() {
        let flag = args[index].clone();
        let value = || {
            args.get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        if let Some(next) = endpoint.read(args, index)? {
            index = next;
            continue;
        }
        match args[index].as_str() {
            "--rvt" => {
                rvt = Some(PathBuf::from(value()?));
                index += 2;
            }
            "--remote-dir" => {
                remote_dir = value()?;
                index += 2;
            }
            "--enable" => {
                enable = true;
                index += 1;
            }
            "--no-wait" => {
                wait = false;
                index += 1;
            }
            // **The last word, and only the last word.** A `confirm` in the
            // middle of a command line is a mistake rather than a decision, and
            // `ADR-012` §7.1's whole point is that the confirmation is
            // deliberate. Same rule the board applies.
            "confirm" if index + 1 == args.len() => {
                confirmed = true;
                index += 1;
            }
            "confirm" => {
                return Err("`confirm` must be the LAST word, so that it confirms the whole \
                            command rather than whatever happens to follow it (ADR-012 §7.1)"
                    .to_string());
            }
            // Everything else is `package`'s, passed through untouched. This
            // tool does not restate `package`'s option list — one authority for
            // what an option means (`ADR-011`), and a flag added there works
            // here the same day.
            _ => {
                package_args.push(flag);
                index += 1;
            }
        }
    }

    let host = endpoint.host.clone().ok_or(
        "--host is required. There is deliberately no default: an install tool with a \
         remembered host installs onto the wrong machine exactly once",
    )?;

    if rvt.is_some() && !package_args.is_empty() {
        return Err(format!(
            "--rvt sends a payload that is already built, so {} would be ignored. Refusing \
             rather than ignoring it — ADR-007 §3.6.5's rule, applied to a command line",
            package_args.join(" ")
        ));
    }

    // **Both or neither**, and refused here rather than at the first
    // connection. `ADR-025` §2's trust is mutual: an identity with no pin
    // trusts whatever answers, and a pin with no identity has nothing to
    // present to a machine that requires client authentication. Either alone is
    // an install that was going to fail after the payload was built. The rule
    // lives in `rivet_remote::endpoint` because `run` needs the same one.
    endpoint.paired()?;

    Ok(DeployArgs {
        host,
        rvt,
        package_args,
        remote_dir,
        confirmed,
        enable,
        identity: endpoint.identity,
        pin: endpoint.pin,
        wait,
    })
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;


    // ---- the command line --------------------------------------------------

    fn parse_of(words: &[&str]) -> Result<DeployArgs, String> {
        let owned: Vec<String> = words.iter().map(|w| (*w).to_string()).collect();
        parse(&owned)
    }

    #[test]
    fn a_host_is_required_because_the_wrong_one_installs_exactly_once() {
        let err = parse_of(&["--signing-key", "k"]).expect_err("no host");
        assert!(err.contains("--host is required"), "{err}");
    }

    #[test]
    fn confirm_counts_only_as_the_last_word() {
        assert!(
            parse_of(&["--host", "pi", "confirm"])
                .expect("valid")
                .confirmed
        );
        assert!(
            !parse_of(&["--host", "pi"]).expect("valid").confirmed,
            "an absent confirmation is not one"
        );
        let err = parse_of(&["--host", "pi", "confirm", "--rvt", "x"]).expect_err("not last");
        assert!(err.contains("LAST word"), "{err}");
    }

    #[test]
    fn packages_options_pass_through_untouched_and_are_not_restated_here() {
        let args = parse_of(&[
            "--host", "pi", "--signing-key", "app.key", "--bin", "todo", "confirm",
        ])
        .expect("valid");
        assert_eq!(
            args.package_args,
            ["--signing-key", "app.key", "--bin", "todo"]
        );
        assert_eq!(args.host, "pi");
    }

    /// **An install waits for its own install unless told not to**, because
    /// the command an agent types next writes into a directory only the
    /// installer makes.
    #[test]
    fn an_install_waits_unless_told_not_to() {
        assert!(parse_of(&["--host", "pi", "confirm"]).expect("valid").wait);
        assert!(!parse_of(&["--host", "pi", "--no-wait", "confirm"]).expect("valid").wait);
        // And the flag is this verb's, not passed through to `package`.
        assert!(parse_of(&["--host", "pi", "--no-wait"]).expect("valid").package_args.is_empty());
    }

    /// Passing build options beside a built payload means one of them is being
    /// ignored, and `ADR-007` §3.6.5's rule is that the system never accepts an
    /// input it will discard.
    #[test]
    fn a_prebuilt_payload_and_build_options_together_are_refused() {
        let err = parse_of(&["--host", "pi", "--rvt", "x.rvt", "--bin", "todo"])
            .expect_err("both were given");
        assert!(err.contains("--rvt"), "{err}");
        assert!(err.contains("--bin"), "{err}");
    }
}
