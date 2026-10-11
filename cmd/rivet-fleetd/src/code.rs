//! **Fleet as code** — one pass over the repository, `ADR-044`.
//!
//! `.rulings/` R-145: the fleet's desired-state documents live in a git
//! repository, and a push is what changes the fleet. This is the part of
//! `rivet-fleetd` that makes that true — it polls the repository, works out
//! which machines the current commit asks to be different, and applies each
//! document to its machine over the control plane.
//!
//! # What it is composed of, and where the thinking is
//!
//! | | |
//! | :-- | :-- |
//! | [`crate::repo`] | the repository's head, its machines and a document at a commit, asked of this server's own control plane |
//! | [`crate::document`] | reading and writing one machine's document over `ADR-035` |
//! | `rivet_core::converge` | **the decision.** Three sets of leaves in, a verdict out |
//! | `rivet_config::leaves` | a document's text as those leaves |
//! | this module | the wiring, the baseline, and what gets said |
//!
//! **None of the judgement is here.** That is deliberate: the rule about when a
//! push may overwrite a machine is the part that has to be tested, and a rule
//! that lives beside a `git fetch` is only ever tested against a board.
//!
//! # Three properties that are the difference between this and a cron job
//!
//! **It is quiet.** A machine that already says what the repository asks for
//! produces no journal record at all. A poller running every minute against a
//! fleet that is already right must be silent, or the one cycle that mattered
//! is unfindable in what it wrote.
//!
//! **A machine that is off is not a failure.** The document is intent;
//! convergence happens when the machine is reachable. Reporting an unreachable
//! machine as a failed apply would train an operator to stop reading the
//! report, which is worse than not writing one.
//!
//! **A push that would overwrite out-of-band drift refuses**, and names every
//! key with both values. `apply edit` still exists and must — a fleet you
//! cannot touch when the repository is unreachable is worse than no fleet — so
//! drift is possible by design, and the dangerous direction needs a word.
//!
//! # The secret rule is the schema's, and this only has to invoke it
//!
//! `.rulings/` R-145.1 item 3: a document in git carries a *reference* to a
//! secret and never a secret. **There is no key in the schema a credential
//! could be written under** — `ADR-007` §7.3, and `rivet-config`'s own tests
//! assert it for `psk`, `passphrase`, `password`, `pmk` and `key`. So every
//! file this reads goes through `rivet_config::Document::parse` before anything
//! else looks at it, and the guard that refuses an unknown key is the guard
//! that refuses a literal. **That check happens on the fleet server**, so a
//! push carrying one is refused before it reaches a machine.

use std::path::PathBuf;

use rivet_core::converge::{Verdict, decide, overrides, report};
use rivet_core::leaf::Leaves;

use crate::document::Machine;
use crate::repo::{ControlPlane, Repository};

/// The settings this application understands for fleet-as-code.
///
/// **In the machine document, under `[applications.rivet-fleetd]`** — the
/// mechanism `ADR-038` already built, rather than a configuration file of this
/// feature's own. It is also pleasingly recursive: the fleet server's own
/// document says where the fleet's documents are, so a fleet server's
/// configuration is itself fleet-as-code.
pub mod setting {
    /// The name of the repository on THIS server that holds the fleet's
    /// documents, as this server's own document declares it under
    /// `[repos.<name>]`. **Unset means fleet-as-code is not running**, which is
    /// the default.
    pub const REPOSITORY: &str = "fleet-repository";
    /// Which branch is the fleet. Defaults to [`super::DEFAULT_BRANCH`].
    pub const BRANCH: &str = "fleet-branch";
    /// How often to look, in seconds. Defaults to [`super::DEFAULT_INTERVAL`].
    pub const INTERVAL: &str = "fleet-poll-seconds";
}

/// The branch a fleet lives on unless told otherwise.
pub const DEFAULT_BRANCH: &str = "main";

/// How often the repository is looked at, unless told otherwise.
///
/// **A minute, and the number is not interesting** — R-145.1 chose polling
/// precisely because it is slow and dull. What matters is that a fetch of an
/// unchanged repository is one round trip and costs nothing, so this can be
/// short without being expensive, and that nothing about correctness depends on
/// it: a cycle that is missed is a cycle that happens next time.
pub const DEFAULT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// Where the baselines live, inside `RIVET_APP_DATA`.
pub const DIR: &str = "fleet-as-code";

/// The baselines, under [`DIR`].
///
/// One file per device, named by its **identifier** and not by its label —
/// unlike the repository, where a human reads the filenames. A label is
/// mutable (`ADR-025` §4.1) and a baseline that moved when somebody renamed a
/// machine would silently become *this machine has never been converged*,
/// which is a refusal an operator would then override.
pub const BASELINES: &str = "converged";

/// Everything one pass needs that is not in the repository.
pub struct Fleet {
    /// The repository's name on this server, as [`setting::REPOSITORY`] gives it.
    pub repository: String,
    /// The branch that is the fleet.
    pub branch: String,
    /// Where the baselines are kept.
    pub baselines: PathBuf,
    /// Whether to write anything at all. `true` reports and touches nothing —
    /// the mode an operator uses to see what a push would do.
    pub dry_run: bool,
}

/// What one pass did, so a caller can say so and a test can assert it.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Pass {
    /// Machines that already agreed. Counted, never named — see the module
    /// note on being quiet.
    pub converged: usize,
    /// Machines written to.
    pub applied: Vec<String>,
    /// Machines refused, and why.
    pub refused: Vec<(String, String)>,
    /// Machines that could not be reached. **Not a failure.**
    pub unreachable: Vec<(String, String)>,
    /// Entries in the repository that are wrong about themselves — a document
    /// that is not a valid one, or a machine the register does not know.
    ///
    /// **Kept apart from [`Pass::refused`]**, which is about a machine's state.
    /// These are fixed by editing the repository and nothing about the fleet
    /// will change them.
    pub rejected: Vec<(String, String)>,
}

impl Pass {
    /// Whether anything happened that an operator has to act on.
    #[must_use]
    pub const fn needs_reading(&self) -> bool {
        !self.refused.is_empty() || !self.rejected.is_empty()
    }
}

/// **One pass over the repository.**
///
/// Reads the branch's head over `control`, then walks the machines it names, in
/// the order git lists them. `ADR-044` §7 is why that order is not a policy: nothing here stops at
/// the first refusal, because a refusal writes nothing and a machine's failure
/// to converge says nothing about the next machine's document. What a
/// fleet-wide change that *bricks* the first machine should do is R-145.2 and
/// is not built.
///
/// # Errors
///
/// Only what stops the pass happening at all: an unreachable repository, or a
/// branch that does not resolve. **Everything about an individual machine is a
/// [`Pass`] entry rather than an error**, because one machine being off must
/// not stop the other nine converging.
pub fn pass(
    fleet: &Fleet,
    register: &rivetos_register::Register,
    control: &dyn ControlPlane,
) -> Result<Pass, String> {
    let repository = Repository { name: &fleet.repository, control };
    let head = repository.head(&fleet.branch)?;
    let commit = head.commit.as_str();
    let listing = repository.machines(commit)?;

    let mut pass = Pass::default();
    for name in listing.ignored {
        let why = format!(
            "{name} in {}/ is not named by a label a machine can have, so it was not read. Rename \
             it to the machine's label.",
            rivet_core::fleet_as_code::MACHINES_DIR
        );
        println!("rivet-fleetd: fleet-as-code: {why}");
        pass.rejected.push((name, why));
    }
    for label in listing.labels {
        // **Every outcome is printed HERE and nowhere below**, so that one
        // machine's account is one place and a reader can see the order they
        // were tried in. `Converged` prints nothing at all, which is the whole
        // of the quietness rule.
        match one(fleet, &repository, register, commit, &head.message, &label) {
            Ok(Outcome::Converged) => pass.converged += 1,
            Ok(Outcome::Applied(said)) => {
                println!("rivet-fleetd: fleet-as-code: {said}");
                pass.applied.push(label);
            }
            Ok(Outcome::Refused(why)) => {
                println!("rivet-fleetd: fleet-as-code: {why}");
                pass.refused.push((label, why));
            }
            Ok(Outcome::Unreachable(why)) => {
                println!(
                    "rivet-fleetd: fleet-as-code: {label} was not reachable this cycle, which \
                     is not a failure - the document is the intent and it converges when the \
                     machine answers: {why}"
                );
                pass.unreachable.push((label, why));
            }
            Err(why) => {
                println!("rivet-fleetd: fleet-as-code: {label}: {why}");
                pass.rejected.push((label, why));
            }
        }
    }
    Ok(pass)
}

/// What happened to one machine, and what to say about it.
enum Outcome {
    /// Nothing to do. **Carries no words**, because there are none.
    Converged,
    Applied(String),
    Refused(String),
    Unreachable(String),
}

/// One machine, this pass.
///
/// # Errors
///
/// Only things about the **repository's** entry for this machine that no
/// amount of waiting fixes: a document that is not a valid one, or a name the
/// register does not know. A machine that is switched off is
/// [`Outcome::Unreachable`], which is not an error.
fn one(
    fleet: &Fleet,
    repository: &Repository<'_>,
    register: &rivetos_register::Register,
    head: &str,
    message: &str,
    label: &str,
) -> Result<Outcome, String> {
    let path = crate::repo::path_of(label);
    let Some(text) = repository.file(head, &path)? else {
        // `machines` listed it out of the same commit, so this is a race with a
        // push rather than a state. Next cycle.
        return Ok(Outcome::Unreachable(format!(
            "{path} was listed at {head} and is not there any more"
        )));
    };

    // **The schema first, and on the fleet server.** This is where an unknown
    // key is refused, which is where a credential written into a document is
    // refused (R-145.1 item 3, `ADR-007` §7.3). Doing it here rather than
    // letting the machine do it means a bad push never reaches a machine at
    // all.
    rivet_config::Document::parse(&text).map_err(|e| {
        format!(
            "{path} is not a document this fleet can apply: {e}. Nothing was sent to any \
             machine. Note that the schema has NO key a credential could be written under - \
             `ADR-007` section 7.3 - so a refusal naming an unknown key may be a secret that \
             belongs in `apply install-secret` and not in this repository."
        )
    })?;
    let desired = rivet_config::leaves::leaves(&text)?;

    let device = register.resolve(label).map_err(|e| {
        format!(
            "{e}. The repository names this machine and the register does not, so there is \
             no address and no pin for it. Enroll it, or take the file out of the repository."
        )
    })?;
    let pin = device.pin.as_deref().ok_or_else(|| {
        format!(
            "{}'s record carries no pin, so there is nothing to trust at the other end and \
             nothing was sent. ADR-025 section 2 makes the pinned key the whole trust model.",
            device.name
        )
    })?;
    // **`dial_host`, not `address`.** A DHCP member's record carries no
    // address; `dial_host` derives `<name>.local` for it, which `crate::node`
    // resolves over mDNS (`rivet_mdns`). The strict `address()` refused such a
    // member outright, so fleet-as-code could never converge one — the same
    // gap the sidebar render (`crate::fleet::device`) was fixed for.
    let address = format!("{}:{}", device.dial_host()?, device.control_plane_port());
    let machine = Machine { address: &address, pin };

    // **From here down, a failure is the machine being unreachable.** It is
    // separated deliberately: everything above is about the repository and the
    // register, which an operator fixes; everything below is about a machine
    // that may simply be switched off.
    let current = match machine.read() {
        Ok(Some(text)) => text,
        Ok(None) => {
            return Ok(Outcome::Refused(format!(
                "{label}: this machine has no desired-state document at all, so there is \
                 nothing to compare against and nothing was written. A first document is \
                 provisioning's job (ADR-004 section 6) - a document without `version` is \
                 not a valid one, and writing one blind is exactly what this refuses."
            )));
        }
        Err(why) => return Ok(Outcome::Unreachable(why)),
    };
    let held = rivet_config::leaves::leaves(&current).map_err(|e| {
        format!("{label}: what the machine holds is not readable as TOML: {e}")
    })?;

    let baseline = read_baseline(fleet, repository, &device.id, &path)?;
    let verdict = decide(&held, &desired, baseline.as_ref(), overrides(message, label));
    let said = report(label, &verdict).unwrap_or_else(|| format!("{label}: nothing to do"));
    let said = if fleet.dry_run {
        format!("WOULD DO -- {said}")
    } else {
        said
    };

    match verdict {
        Verdict::Converged => {
            // **The baseline is recorded even though nothing was written**,
            // and that is the bootstrap `ADR-044` §4 describes: a machine's
            // document is copied into the repository verbatim, the first cycle
            // finds them equal, and the machine is adopted without anything
            // being touched.
            if !fleet.dry_run {
                write_baseline(fleet, &device.id, head)?;
            }
            Ok(Outcome::Converged)
        }
        Verdict::Drifted { .. } | Verdict::Unadopted { .. } => Ok(Outcome::Refused(said)),
        Verdict::Apply { .. } | Verdict::Overwrite { .. } => {
            if fleet.dry_run {
                return Ok(Outcome::Applied(said));
            }
            match machine.write(&current, &text) {
                Ok(()) => {
                    write_baseline(fleet, &device.id, head)?;
                    Ok(Outcome::Applied(format!(
                        "{said}{label} is converged onto {} ({} bytes)",
                        &head[..head.len().min(12)],
                        text.len()
                    )))
                }
                // **A write that failed halfway is not a machine being off**,
                // and this carries the machine's own words rather than a
                // summary: `Machine::write` reads the document back and its
                // refusal says whether what is on the machine is the old
                // document, the new one, or neither.
                Err(why) => Ok(Outcome::Unreachable(why)),
            }
        }
    }
}

/// The document the repository held at the commit this machine was last
/// converged onto.
///
/// `None` means it never has been. **A baseline naming a commit that is no
/// longer in the repository is also `None`** — a history rewritten under a
/// fleet server is exactly when it should stop believing what it wrote down,
/// and refusing as unadopted is the safe direction.
fn read_baseline(
    fleet: &Fleet,
    repository: &Repository<'_>,
    id: &str,
    path: &str,
) -> Result<Option<Leaves>, String> {
    let at = fleet.baselines.join(id);
    let Ok(commit) = std::fs::read_to_string(&at) else {
        return Ok(None);
    };
    let commit = commit.trim();
    if commit.is_empty() {
        return Ok(None);
    }
    match repository.file(commit, path) {
        Ok(Some(text)) => Ok(Some(rivet_config::leaves::leaves(&text)?)),
        // The commit is gone, or that machine was not in it. Either way there
        // is nothing to compare against.
        Ok(None) | Err(_) => Ok(None),
    }
}

/// Record which commit this machine is now converged onto.
///
/// **The commit and not the document.** Git already holds every version of the
/// document with an author and a date; a second copy of the text here would be
/// a second authority for what the repository said, and it would be the copy
/// nobody could diff.
fn write_baseline(fleet: &Fleet, id: &str, commit: &str) -> Result<(), String> {
    std::fs::create_dir_all(&fleet.baselines)
        .map_err(|e| format!("{}: {e}", fleet.baselines.display()))?;
    let at = fleet.baselines.join(id);
    std::fs::write(&at, format!("{commit}\n")).map_err(|e| format!("{}: {e}", at.display()))
}

/// What one pass is worth saying, or nothing.
///
/// **A pass in which every machine agreed says nothing at all.** That is the
/// quietness rule at the level of the pass rather than of one machine: a fleet
/// server whose fleet is correct writes no journal records, so every record it
/// does write is one somebody should read.
#[must_use]
pub fn summary(pass: &Pass) -> Option<String> {
    if pass.applied.is_empty()
        && pass.refused.is_empty()
        && pass.unreachable.is_empty()
        && pass.rejected.is_empty()
    {
        return None;
    }
    let mut out = format!("fleet-as-code: {} machine(s) already agreed", pass.converged);
    if !pass.applied.is_empty() {
        out.push_str(&format!("; converged {}", pass.applied.join(", ")));
    }
    if !pass.refused.is_empty() {
        let named: Vec<&str> = pass.refused.iter().map(|(name, _)| name.as_str()).collect();
        out.push_str(&format!("; REFUSED {}", named.join(", ")));
    }
    if !pass.unreachable.is_empty() {
        // **The wording matters.** These are not failures and must not read as
        // any: a machine that is off is the ordinary state of a fleet, and a
        // report that called it an error would be one an operator learns to
        // skip.
        let named: Vec<&str> = pass
            .unreachable
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        out.push_str(&format!(
            "; not reachable this cycle, which is not a failure: {}",
            named.join(", ")
        ));
    }
    if !pass.rejected.is_empty() {
        let named: Vec<&str> = pass.rejected.iter().map(|(name, _)| name.as_str()).collect();
        out.push_str(&format!(
            "; THE REPOSITORY IS WRONG ABOUT {}, and each said why above",
            named.join(", ")
        ));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **The quiet pass says nothing**, which is the property that keeps a
    /// poller's journal readable.
    #[test]
    fn a_pass_in_which_everything_agreed_produces_no_summary() {
        let pass = Pass { converged: 9, ..Pass::default() };
        assert_eq!(summary(&pass), None);
        assert!(!pass.needs_reading());
    }

    /// A machine that is off is reported, and NOT as a failure.
    #[test]
    fn an_unreachable_machine_is_not_reported_as_a_failure() {
        let pass = Pass {
            converged: 2,
            unreachable: vec![("garage-pi".to_owned(), "connection refused".to_owned())],
            ..Pass::default()
        };
        let said = summary(&pass).expect("something happened");
        assert!(said.contains("garage-pi"), "{said}");
        assert!(said.contains("not a failure"), "{said}");
        assert!(!said.contains("REFUSED"), "{said}");
        assert!(
            !pass.needs_reading(),
            "an unreachable machine must not demand an operator's attention"
        );
    }

    /// A refusal does.
    #[test]
    fn a_refusal_demands_attention_and_is_shouted() {
        let pass = Pass {
            refused: vec![("rivet-01".to_owned(), "drift".to_owned())],
            ..Pass::default()
        };
        assert!(pass.needs_reading());
        let said = summary(&pass).expect("something happened");
        assert!(said.contains("REFUSED rivet-01"), "{said}");
    }

    /// **A document the repository got wrong is not the same thing as a
    /// machine that refused**, and the two are named differently: one is fixed
    /// by editing a file, the other by looking at a machine. A summary that
    /// blended them would send an operator to the wrong place.
    #[test]
    fn a_bad_document_in_the_repository_is_reported_as_the_repositorys_fault() {
        let pass = Pass {
            rejected: vec![("kitchen-pi".to_owned(), "unknown key `psk`".to_owned())],
            ..Pass::default()
        };
        assert!(pass.needs_reading());
        let said = summary(&pass).expect("something happened");
        assert!(said.contains("THE REPOSITORY IS WRONG ABOUT kitchen-pi"), "{said}");
        assert!(!said.contains("REFUSED"), "{said}");
    }

    /// The settings are the names an operator writes under
    /// `[applications.rivet-fleetd]`, and the manifest has to declare exactly
    /// these — a mismatch is a setting the supervisor silently does not
    /// deliver.
    #[test]
    fn the_declared_settings_are_the_ones_the_manifest_names() {
        let manifest = include_str!("../Cargo.toml");
        for name in [setting::REPOSITORY, setting::BRANCH, setting::INTERVAL] {
            assert!(
                manifest.contains(&format!("\"{name}\"")),
                "`{name}` is read by this program and is not declared in its manifest, so \
                 the supervisor will never hand it over"
            );
        }
    }
}
