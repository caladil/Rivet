//! `cargo rivet setup` — **the first run.** It walks the credentials, in the
//! order they depend on each other, into `~/.rivet`.
//!
//! # Why this verb exists at all
//!
//! Every piece of it already worked. `key new` mints a signing seed,
//! `identity new` mints a control-plane identity, `cmd/rivet-provision` mints a
//! certificate authority and files a browser identity into the store a browser
//! actually reads. **What did not exist was an order**, or anything that said
//! the set was complete — so the documented happy path went straight from a
//! clone to a card build, and produced a board that refuses the person who
//! built it.
//!
//! # Where it lives, and the argument for it — decided 2026-09-05
//!
//! The other candidate was a mode of `cmd/rivet-provision`, which describes
//! itself as *"the one command-line tool meant for a person"* and is the right
//! model for tone. **It is not where this goes, for two reasons that are
//! specific rather than aesthetic.**
//!
//! **First, everything this calls is already here.** `key new` and `identity
//! new` are `cargo rivet`'s verbs, `rivet-crypto` and `rivetos-tls` are already
//! this crate's dependencies, and `cargo rivet package` has looked in
//! `~/.rivet/keys` since 2026-08-30. Putting the sequencer in the other binary
//! would mean either duplicating those or having one tool shell out to the
//! other, and `ADR-011` is about exactly the first.
//!
//! **Second, `rivet-provision` argues against it itself.** Its `ask_secret`
//! carries the reasoning in as many words: that binary mints and holds
//! `root.key`, the fleet's whole trust, *"so every dependency it gains is one
//! more thing with the opportunity to read one"* — which is why it hand-rolls
//! twenty lines of console FFI rather than taking a crate. Handing it
//! `rivet-crypto`, `rivetos-tls` and a signing seed would spend the budget that
//! reasoning was protecting.
//!
//! So the split is: **`cargo rivet setup` mints the keys; `rivet-provision`
//! mints the authority and writes the document.** Each stays the tool that
//! already owned its half, and this one ends by naming the other.
//!
//! # It is not a full-screen interface, and that is deliberate
//!
//! `rivet-provision`'s screen exists because composing a document is a set of
//! choices a person has to make. **Minting credentials is not** — every path is
//! conventional, every name is cosmetic, and the only real decision (*do I want
//! my own signing key?*) has one answer for anybody running this. A prompt with
//! one sensible answer teaches a person to press Return without reading, which
//! is the habit that makes a real confirmation worthless.
//!
//! It also means an agent can run it. A guided screen that blocks on stdin is a
//! guided screen that hangs in CI and in every non-interactive session, and
//! `ADR-012` makes an agent a first-class operator here.
//!
//! # Two promises
//!
//! **It never overwrites a key.** Every step is *look, and mint only what is
//! absent*. `key new` and `identity new` already refuse to overwrite, and this
//! does not reach past those refusals — it checks first so that a second run is
//! a report rather than four errors. Overwriting a seed orphans every board and
//! every payload signed with it, and there is no undo.
//!
//! **It prints no secret.** Not a seed, not a byte of one, on any path.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::credentials::{
    self, BOARD_CREDENTIAL, Credential, OPERATOR_CREDENTIAL, Overrides, Shape, Survey,
};
use crate::identity::{IdentityArgs, Verb};
use crate::key::KeyArgs;

/// The default subject on the operator's own identity.
///
/// **`fleet-owner`**, ruled by the maintainer 2026-09-06. The name a person is
/// offered should be the role they are about to take, not the name of the tool
/// offering it.
///
/// # It is NOT cosmetic, whatever this constant used to say
///
/// It was, once, and the sentence outlived the fact. Two things read this now:
///
/// * **`rivet-proxyd`'s index prints it** — *"Authenticated as: …"* — so it is a
///   string a person reads on a page, every time they open the machine.
/// * **The journal records it** as the principal of every mutating operation
///   (`ADR-028` §4.5), which is the only thing that will ever distinguish one
///   operator from another.
///
/// So a person typing their own name here is doing something real, and
/// [`resolve_name`] asks rather than assuming. A person's name with a space in
/// it — `Your Name` — is the case that must work, because it is the one an
/// operator is most likely to type.
const OPERATOR_NAME: &str = "fleet-owner";

/// The same, for the board's. `identity new`'s own default.
const DEVICE_NAME: &str = "rivet-device";

#[derive(Debug)]
pub struct SetupArgs {
    /// Where `~/.rivet` is. A parameter so that a cold run can be performed
    /// against a scratch directory without a scratch `HOME`.
    pub at: Option<PathBuf>,
    /// The operator certificate's subject.
    ///
    /// **`None` means the caller did not say**, which is a different fact from
    /// *the caller chose the default* — it is the one case where this verb
    /// stops and asks. [`resolve_name`] is where the two become one string, and
    /// keeping them apart until then is what lets a test drive either path
    /// without a terminal.
    pub name: Option<String>,
    /// Report and mint nothing.
    pub check: bool,
    /// Where the environment says a credential already lives.
    ///
    /// **A field rather than a read inside [`run`]**, and the reason is a test
    /// this file nearly shipped: with `RIVET_IDENTITY` set in the shell, a
    /// `run` that consulted the environment itself would mint the board's
    /// identity somewhere other than `--at`, and the cold-run test would fail
    /// on a developer's machine and pass on everybody else's. [`parse`] fills
    /// it from the environment, which is where an environment read belongs.
    pub over: Overrides,
}

pub fn usage() -> String {
    "usage: cargo rivet setup [--at DIR] [--name NAME] [--check]\n\
     \n\
     Mints the credentials a Rivet card is built AGAINST, in order, into\n\
     ~/.rivet — and skips whatever is already there.\n\
     \n\
     \x20 --at DIR     where ~/.rivet is. Default: $HOME/.rivet, or\n\
     \x20              %USERPROFILE%\\.rivet\n\
     \x20 --name NAME  the operator certificate's subject. This is PRINTED on\n\
     \x20              the machine's index page and recorded in the journal\n\
     \x20              beside everything you change, so give the name you want\n\
     \x20              to see there — a name with spaces is fine. Without this\n\
     \x20              flag a terminal is asked; a script gets the default.\n\
     \x20              Default: fleet-owner\n\
     \x20 --check      report what is here and mint nothing\n\
     \n\
     It mints four things, in this order, because each needs the one above it:\n\
     \n\
     \x20 1  a SYSTEM signing seed        signs update payloads\n\
     \x20 2  an APPLICATION signing seed  signs applications and libraries\n\
     \x20 3  YOUR control-plane identity  your end of the TLS control plane\n\
     \x20 4  the BOARD's identity         and a peers file naming yours\n\
     \n\
     THE TWO SEEDS ARE SEPARATE ON PURPOSE (ADR-028 §5.5). A machine that can\n\
     sign an application must not be able to ship an operating system, so the\n\
     system key is not entitled to sign an application and cannot be widened\n\
     into one.\n\
     \n\
     IT NEVER OVERWRITES A KEY. A seed that is already there is reported and\n\
     left alone. Overwriting one orphans every board that trusts its public\n\
     half and every payload signed with it, and there is no undo.\n\
     \n\
     What it does NOT mint is the fleet certificate authority — `rivet-provision`\n\
     owns that, offers it on first run, and files the browser identity into the\n\
     store a browser actually reads. This verb ends by saying so.\n"
        .to_string()
}

/// # Errors
///
/// An unknown option, or one missing its value.
pub fn parse(args: &[String]) -> Result<SetupArgs, String> {
    let mut out = SetupArgs {
        at: None,
        name: None,
        check: false,
        over: Overrides::from_environment(),
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--at" => {
                out.at = Some(PathBuf::from(
                    args.get(i + 1).ok_or("--at needs a directory")?,
                ));
                i += 2;
            }
            "--name" => {
                out.name = Some(args.get(i + 1).ok_or("--name needs a value")?.clone());
                i += 2;
            }
            "--check" => {
                out.check = true;
                i += 1;
            }
            other => {
                return Err(format!("{other:?} is not an option of `setup`\n\n{}", usage()));
            }
        }
    }
    Ok(out)
}

/// # Errors
///
/// A host with no home directory and no `--at`, or anything a mint refused.
pub fn run(args: &SetupArgs) -> Result<String, String> {
    let at = match &args.at {
        Some(p) => p.clone(),
        None => credentials::root().ok_or(
            "this host sets neither HOME nor USERPROFILE, so there is nowhere to put \
             ~/.rivet. Pass --at DIR.",
        )?,
    };

    // **One set of overrides, used by every survey in this run.** A survey
    // taken before minting and one taken after must agree about WHERE things
    // are, or the report at the end describes a different set from the one
    // that was written.
    let over = &args.over;
    let before = Survey::take(&at, over);

    if args.check {
        return Ok(format!(
            "{at}\n\n{report}\n{next}",
            at = at.display(),
            report = before.report(),
            next = next_steps(&at, &before),
        ));
    }

    let name = resolve_name(args.name.as_deref(), &before)?;

    let mut said = String::new();
    said.push_str(&format!("--- {}\n\n", at.display()));

    for credential in &before.items {
        // The authority is `rivet-provision`'s and is named rather than made
        // here — see this module's header for the argument.
        if credential.shape == Shape::Authority {
            continue;
        }
        said.push_str(&mint(credential, &name)?);
    }

    // **The peers file is the last step and the one that matters.** Everything
    // above it can be repaired from a workstation; this is the file that
    // decides whether the board will speak to you at all, and it needs both
    // identities to exist before it can be written.
    said.push_str(&enroll(&Survey::take(&at, over))?);

    let after = Survey::take(&at, over);
    said.push('\n');
    said.push_str(&after.report());
    said.push_str(&next_steps(&at, &after));
    Ok(said)
}

/// Six spaces, put in at RUN TIME rather than typed into a literal.
///
/// **`tools/source/prose.py` refuses a run of six or more spaces inside a
/// string literal**, which is what a lost `\` line continuation leaves behind —
/// and it is right to, because that is indistinguishable from column alignment
/// somebody meant. Alignment this tool means is therefore composed, not typed.
fn indent(line: &str) -> String {
    format!("{:<6}{line}", "")
}

/// The subject to mint the operator's certificate with.
///
/// # Why this asks, when every other name in this tool is defaulted
///
/// Because this one ends up in front of people. `rivet-proxyd`'s index prints
/// it as *"Authenticated as: …"* and the journal records it as the principal of
/// every mutating operation, so it is the string that answers *who did this* for
/// the life of the machine. A default silently accepted is a machine whose
/// records all say `fleet-owner` — which is honest and useless the moment there
/// is a second operator.
///
/// # It asks ONCE, and only when there is something to ask about
///
/// * **`--name` given** — the caller has said, and is not asked again.
/// * **The identity already exists** — nothing is being minted, so there is no
///   name to choose. Re-minting is refused by [`mint`] either way, and asking a
///   question whose answer is then discarded trains a person to stop reading.
/// * **Not a terminal** — a script, or CI. It takes the default and says so in
///   the report rather than blocking on a prompt nobody can answer.
///
/// A name with a **space** in it is the case that must work: an operator types
/// their own name, such as `Your Name`, so nothing here splits on whitespace or
/// refuses one.
fn resolve_name(given: Option<&str>, survey: &Survey) -> Result<String, String> {
    if let Some(name) = given {
        return Ok(name.to_owned());
    }
    let minting = survey
        .named(OPERATOR_CREDENTIAL)
        .is_some_and(|c| !c.present && c.incomplete.is_none());
    if !minting || !std::io::stdin().is_terminal() {
        return Ok(OPERATOR_NAME.to_owned());
    }

    println!("\n  Your operator certificate's name.");
    println!("  This is printed on the machine's index page and recorded in the");
    println!("  journal beside everything you change, so use the name you want to");
    println!("  see there — your own name is a good answer.");
    print!("\n  name [{OPERATOR_NAME}]: ");
    std::io::stdout().flush().ok();

    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|e| format!("cannot read a name: {e}"))?;
    let typed = line.trim();
    if typed.is_empty() {
        return Ok(OPERATOR_NAME.to_owned());
    }
    Ok(typed.to_owned())
}

/// Mint one credential if it is absent, and say which happened.
///
/// **The absent branch is the only one that writes.** A present credential is
/// reported with its public key, which is the thing a person actually needs
/// from a second run.
fn mint(credential: &Credential, operator_name: &str) -> Result<String, String> {
    if credential.present {
        let key = credential
            .public_key()
            .unwrap_or_else(|why| format!("(cannot be read: {why})"));
        return Ok(format!(
            "have  {name}\n{at}\n{key}\n\n",
            name = credential.name,
            at = indent(&credential.path.display().to_string()),
            key = indent(&key),
        ));
    }

    match credential.shape {
        Shape::Seed => {
            let published = published_beside(&credential.path);
            // **A published half with no seed behind it is refused, not minted
            // over.** `key --out` will not overwrite a public key, so minting a
            // seed here would leave a `.pub` that belongs to a key nobody
            // holds — and that file is precisely what a card's trust store is
            // composed from. It is the failure this whole verb exists to stop,
            // in miniature and one directory earlier.
            if published.exists() {
                return Err(format!(
                    "{published} is here and {seed} is not.\n\n\
                     That public key belongs to a seed this host does not have, and a card \
                     built to trust it would refuse everything you sign. Minting a new seed \
                     beside it would leave the pair disagreeing silently, so this stops \
                     instead.\n\n\
                     Move the public key aside if the seed is genuinely gone, or put the seed \
                     back if it is somewhere else.",
                    published = published.display(),
                    seed = credential.path.display(),
                ));
            }
            if let Some(parent) = credential.path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
            }
            // `key new` writes the seed and derives the public half; `key` with
            // `--out` publishes it. Two calls into the one implementation
            // rather than a second one here — `ADR-011` — and the seed's bytes
            // stay inside that module either way.
            crate::key::run(&KeyArgs {
                seed: PathBuf::from("new"),
                out: Some(credential.path.clone()),
            })?;
            crate::key::run(&KeyArgs {
                seed: credential.path.clone(),
                out: Some(published.clone()),
            })?;
            Ok(format!(
                "MADE  {name}\n{at}\n{pub_at}\n{key}\n\n",
                name = credential.name,
                at = indent(&credential.path.display().to_string()),
                pub_at = indent(&format!("public half {}", published.display())),
                key = indent(&credential.public_key()?),
            ))
        }
        Shape::Identity | Shape::EnrolledIdentity => {
            // A directory that exists and is short of a file is NOT re-minted:
            // `identity new` refuses when either file is present, and reaching
            // past that refusal is how a half-written identity becomes a
            // silently replaced one.
            if let Some(part) = &credential.incomplete {
                return Ok(format!(
                    "PART  {name}\n{at}\n{what}\n{advice}\n\n",
                    name = credential.name,
                    at = indent(&credential.path.display().to_string()),
                    what = indent(part),
                    advice = indent(
                        "left alone: move the directory aside if you mean to re-mint it"
                    ),
                ));
            }
            let name = if credential.shape == Shape::Identity {
                operator_name.to_owned()
            } else {
                DEVICE_NAME.to_owned()
            };
            crate::identity::run(&IdentityArgs {
                verb: Verb::New,
                directory: credential.path.clone(),
                name,
            })?;
            Ok(format!(
                "MADE  {name}\n{at}\n{key}\n\n",
                name = credential.name,
                at = indent(&credential.path.display().to_string()),
                key = indent(&credential.public_key()?),
            ))
        }
        // `rivet-provision` mints the authority. See this module's header.
        Shape::Authority => Ok(String::new()),
    }
}

/// **Put the operator's public key in the board's `peers` file.**
///
/// This is the step whose absence produces the failure the whole verb exists
/// for: a card that boots, addresses itself, brings up the control plane, and
/// refuses the only person who has any reason to call it.
///
/// # It writes `peers` only when there is none
///
/// An existing `peers` file is left exactly as it is, even when the operator's
/// key is not in it. **Adding a key to a trust list that already exists is not
/// this verb's decision** — `AGENTS.md` §1.1 puts anything reaching a machine's
/// trust in the maintainer's column — so it says what to add and where, and
/// stops.
///
/// # Errors
///
/// If the operator's certificate cannot be read, or the file cannot be written.
fn enroll(survey: &Survey) -> Result<String, String> {
    let (Some(operator), Some(board)) = (
        survey.named(OPERATOR_CREDENTIAL),
        survey.named(BOARD_CREDENTIAL),
    ) else {
        return Ok(String::new());
    };
    // Nothing to enroll into, and nothing to enroll: an identity that was not
    // minted has no certificate to read a key out of, and the missing-list at
    // the end of the report is where that is said.
    if !operator.path.join(rivetos_tls::CERTIFICATE_FILE).is_file() {
        return Ok(String::new());
    }
    let key = operator.public_key()?;
    let at = board.path.join(credentials::PEERS_FILE);

    if at.is_file() {
        let text = std::fs::read_to_string(&at)
            .map_err(|e| format!("cannot read {}: {e}", at.display()))?;
        let carries = text
            .lines()
            .any(|line| line.trim().eq_ignore_ascii_case(&key));
        return Ok(if carries {
            format!(
                "have  peers\n{at}\n{note}\n\n",
                at = indent(&at.display().to_string()),
                note = indent("names your operator key"),
            )
        } else {
            format!(
                "KEEP  peers\n{at}\n{a}\n{b}\n{c}\n\n",
                at = indent(&at.display().to_string()),
                a = indent("exists and does NOT name your key, so it was left alone —"),
                b = indent("adding a key to a trust list is not this verb's decision."),
                c = indent(&format!("add this line yourself: {key}")),
            )
        });
    }

    std::fs::create_dir_all(&board.path)
        .map_err(|e| format!("cannot create {}: {e}", board.path.display()))?;
    std::fs::write(&at, format!("{key}\n"))
        .map_err(|e| format!("cannot write {}: {e}", at.display()))?;
    Ok(format!(
        "MADE  peers\n{at}\n{note}\n\n",
        at = indent(&at.display().to_string()),
        note = indent("naming your operator key, and nothing else"),
    ))
}

/// `<seed>.pub`, next to the seed. `rivet-app.seed` becomes `rivet-app.pub`.
fn published_beside(seed: &Path) -> PathBuf {
    seed.with_extension("pub")
}

/// **What to do next, composed from what is actually on disk.**
///
/// The environment block is the half a first run cannot guess, and it is the
/// one that produces the failure this whole verb exists for: a card built from
/// a clean checkout composes its trust store out of the public keys committed
/// in `keys/`, which are **the maintainer's and not yours**. Nothing warns at
/// build time, because from the build script's side a trust store was composed
/// exactly as it was asked to compose one.
///
/// # The two public keys have to be copied into the repository
///
/// **`tools/container/build-card.sh` mounts only the repository**, and refuses
/// `RIVET_TRUSTED_KEY` or `RIVET_APP_KEY` pointing anywhere else — *"only the
/// repository is mounted"*, which is the right refusal and is why this block
/// does not simply name `~/.rivet/keys/*.pub` and leave it there. A public key
/// is exactly the kind of thing `keys/README.md` says belongs in the
/// repository; the seeds stay out of it, for ever (`AGENTS.md` §8.3).
fn next_steps(at: &Path, survey: &Survey) -> String {
    let mut out = String::from("\nnext\n\n");
    let missing: Vec<&str> = survey.missing_required().iter().map(|c| c.name).collect();
    if !missing.is_empty() {
        out.push_str(&format!(
            "  STILL MISSING: {}. Nothing below will work until they are here.\n\n",
            missing.join(", ")
        ));
    }

    let authority_here = survey
        .items
        .iter()
        .any(|c| c.shape == Shape::Authority && c.present);
    let interview = if authority_here {
        "the interview"
    } else {
        "the interview (it offers to mint the fleet authority)"
    };
    // **Built once, selected per machine** — `ADR-054` §2. Until 2026-09-14
    // these steps built a card per machine, which R-175 had already ruled out.
    for (n, step) in [
        (
            1,
            "tools/container/build-card.sh, then cargo rivet image — build the system\n\
             \x20    image ONCE and publish it into the fleet repository"
                .to_owned(),
        ),
        (
            2,
            "cargo rivet fleet publish — publish each application a role carries".to_owned(),
        ),
        (
            3,
            format!(
                "cargo rivet provision — per machine: {interview}, an image\n\
                 \x20    selected from the fleet repository, and a written card"
            ),
        ),
    ] {
        out.push_str(&format!("  {n}  {step}\n"));
    }

    let keys = at.join("keys");
    let identity = at.join("identity");

    out.push_str(
        "\n  THE IMAGE IS BUILT AGAINST THESE KEYS, so step 1 has to be told about them.\n\
         \x20 Left to itself it composes a trust store out of this checkout's COMMITTED\n\
         \x20 public keys, which are not yours, and the card then refuses everything you\n\
         \x20 sign. From the repository root:\n\n",
    );
    for (from, to) in [
        (keys.join("rivet-system.pub"), "keys/my-system.pub"),
        (keys.join("rivet-app.pub"), "keys/my-app.pub"),
    ] {
        out.push_str(&format!("    cp {} {to}\n", from.display()));
    }
    for (name, value) in [
        ("RIVET_TRUSTED_KEY", "\"$PWD/keys/my-system.pub\""),
        ("RIVET_APP_KEY", "\"$PWD/keys/my-app.pub\""),
    ] {
        out.push_str(&format!("    export {name}={value}\n"));
    }
    out.push_str(
        "\n  The COPY is not optional: the card build mounts only the repository and\n\
         \x20 refuses a key path outside it. A public key belongs there anyway — see\n\
         \x20 keys/README.md. THE SEEDS DO NOT, ever (AGENTS.md 8.3). Name those when you\n\
         \x20 sign, and never anywhere a checkout can reach:\n\n",
    );
    for (name, value) in [
        (credentials::SYSTEM_SEED_ENV, keys.join("rivet-system.seed")),
        (crate::APP_SEED_ENV, keys.join("rivet-app.seed")),
    ] {
        out.push_str(&format!("    export {name}={}\n", value.display()));
    }

    out.push_str(&format!(
        "\n  and reach the board, once it is running, with YOUR identity and the BOARD's key:\n\n\
         \x20   cargo rivet run --host <address>:7777 --identity {operator} --pin <hex> -- get status\n\
         \n\
         \x20 THE PIN IS THE BOARD'S PUBLIC KEY, NOT YOURS. It is the one asymmetry in this\n\
         \x20 whole surface that catches everybody. Read it back with\n\n\
         \x20   cargo rivet identity show --directory {board}\n",
        operator = identity.join("operator").display(),
        board = identity.join("board").display(),
    ));
    out
}

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;
