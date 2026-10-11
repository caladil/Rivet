//! **A fleet server's repository objects** — the application and library
//! artifacts its fleet repository's manifest names, held on the server itself.
//! `.rulings/` R-182, R-184 D1 and D2; `ADR-052` §7.
//!
//! # What this is, in one sentence
//!
//! A fleet server's card carries the fleet repository's text and none of its
//! objects (R-182), `cargo rivet fleet push` puts the text in `/state/git/fleet.git`,
//! and the objects follow **over the control plane** into a digest-keyed store
//! inside `rivet-fleetd`'s own data directory, apart from the machine-wide
//! `/state/artifacts`, pruned by what the repository's manifest names (R-184 D1).
//!
//! # Why the control plane writes it, and not `rivet-fleetd`
//!
//! **A confined application cannot run the door.** `crate::confinement::policy`
//! grants an application its own directories, `/lib`, `/etc`, the typefaces and
//! `/run/rivet`, and refuses `/system/trust`, `/system/arch`, `/system/bin` and
//! everything under `/state` that is not its own — each asserted by that
//! module's `nothing_outside_the_declaration_is_granted`. So `rivet-fleetd`
//! could neither verify an artifact against this machine's trust store nor read
//! `/state/git/fleet.git` to learn what the manifest names. `cmd/rivet-mcpd`
//! runs as uid 0 unconfined, already holds the door `ADR-052` §7 describes, and
//! already runs git for `ADR-035` §4. R-182's own words are that the objects
//! *"follow over the control plane after first boot, as system updates do"*.
//!
//! The directory is still `rivet-fleetd`'s: it sits in that application's data
//! directory and every entry is handed to that application's uid, the way
//! `ADR-049`'s staged material is handed over by the supervisor.
//!
//! # What this module holds
//!
//! The names both ends must agree on — the routes, the listing's vocabulary,
//! which kinds the store holds — and the two decisions that are pure: which
//! digests a manifest names, and what a prune would remove. **No I/O**, per
//! `ADR-014` §3: `cmd/rivet-mcpd` reads directories and runs git, and
//! `cmd/cargo-rivet` reads the answer.
//!
//! # Who may ask — R-184 D2, and the one rule a member fetch would add
//!
//! **Only the operator pushes and fetches for now.** On the control plane that
//! is whoever holds a key in this machine's `peers`, which provisioning fills
//! with the operator's identity and the fleet server's own. [`permits`] is the
//! decision, written as a table of caller and access so that `ADR-052` §9 step
//! 5's member fetch is one more [`Caller`] and one more row granting it
//! [`Access::Read`], and nothing here has to be taken apart to add it.

use std::collections::BTreeSet;

use crate::artifact::{Digest, Store};
use crate::package::kind::Kind;
use crate::service::{self, ServiceName};

/// The application whose data directory holds the store.
///
/// Spelled here once for this module: no other constant in this crate names
/// `rivet-fleetd`, and `cmd/rivet-mcpd` must not spell it a second time.
pub const APPLICATION: &str = "rivet-fleetd";

/// The repository whose manifest decides what the store holds — `.rulings/`
/// R-184 D3. `rivetos-fleet-repo`'s `SERVED_NAME` is the workstation's spelling
/// and a test there holds the two together.
pub const REPOSITORY: &str = "fleet";

/// The manifest's file name at the root of that repository — `cmd/cargo-rivet`'s
/// `fleet::MANIFEST_FILE`, the file `cargo rivet fleet publish` writes.
pub const MANIFEST_FILE: &str = "manifest.toml";

/// The branch the manifest is read from — the one `cargo rivet fleet push`
/// pushes.
pub const BRANCH: &str = "main";

/// The store's directory inside [`APPLICATION`]'s data directory.
pub const DIR: &str = "repository-objects";

/// Every route on the control plane that reaches the store starts with this.
///
/// * `PUT /repository-objects/<ADR-027 name>` with `Rivet-Blake3` — one object;
/// * `GET /repository-objects/` — the listing ([`Listing`]);
/// * `GET /repository-objects/<digest>` — one held object ([`object_route`]);
/// * `POST /repository-objects/prune` — remove what the manifest stops naming.
pub const PREFIX: &str = "/repository-objects/";

/// The prune route. `prune` cannot collide with an object's name: an object's
/// name ends in `.rvt` (`cmd/rivet-mcpd`'s `safe_name`).
pub const PRUNE: &str = "/repository-objects/prune";

/// The header an object's answer carries its `ADR-027` name in — `GET
/// /repository-objects/<digest>` (`ADR-063` §6). The digest is `Rivet-Blake3`,
/// the header a `PUT` already carries.
pub const NAME_HEADER: &str = "Rivet-Name";

/// **The route that reads one held object back** — `GET
/// /repository-objects/<digest>`, the half a workstation's clone fetches
/// through (`ADR-063` §6). A digest is 64 lowercase hex characters, so it
/// collides with neither `prune` nor an object's name, which ends in `.rvt`.
#[must_use]
pub fn object_route(digest: &Digest) -> String {
    format!("{PREFIX}{digest}")
}

/// The digest a `GET` path asks for, when it is [`object_route`]'s shape and
/// nothing else — `None` for the listing, `prune`, a name, or anything with a
/// further `/` in it.
#[must_use]
pub fn requested_object(path: &str) -> Option<Digest> {
    let rest = path.strip_prefix(PREFIX)?;
    Digest::new(rest)
}

/// The store on `volume`: `<volume>/app/rivet-fleetd/data/repository-objects`,
/// laid out as `crate::artifact::Store` lays out the machine's own —
/// `objects/<digest>/<name>.rvt` and `incoming/<digest>/<name>.rvt` — so every
/// path into it is built from a checked [`Digest`].
#[must_use]
pub fn store(volume: &str) -> Store {
    Store::at(format!("{}/{DIR}", data_dir(volume)))
}

/// [`APPLICATION`]'s data directory on `volume`.
#[must_use]
pub fn data_dir(volume: &str) -> String {
    // `APPLICATION` is a valid service name, which this module's tests assert;
    // the fallback spells the same path rather than panicking in a daemon.
    ServiceName::new(APPLICATION).map_or_else(
        || format!("{}/{APPLICATION}/{}", service::app_root(volume), service::DATA_DIR),
        |name| service::data_dir(volume, &name),
    )
}

/// **Whether the store holds an artifact of this kind** — every kind this
/// build knows: an `application`, a `library`, an `apk`, a system `update` and
/// a `card` (`ADR-063` §6).
///
/// A fleet server is the authority for its fleet's artifacts, so it holds
/// everything needed to restore the fleet: a server that cannot hold a card
/// cannot write one for a host it recovers, and one that cannot hold an update
/// cannot bring a member current. Retention is the manifest's — the fleet
/// repository keeps three of each product, kind, platform and architecture,
/// and this store prunes what the manifest stops naming — so the store keeps
/// three per kind exactly as the repository does.
///
/// **A kind this build does not know is not held.** A token read out of a
/// newer header is text this build cannot open, and a store holding what it
/// cannot open could not say what it holds.
///
/// This rule is the fleet server's store only. A member's own
/// `/state/artifacts` holds what it installs (`ADR-052` §6, §7).
#[must_use]
pub fn holds_kind(kind: &Kind) -> bool {
    *kind == Kind::application()
        || *kind == Kind::library()
        || *kind == Kind::apk()
        || *kind == Kind::update()
        || *kind == Kind::card()
}

// ---------------------------------------------------------------------------
// who may ask — R-184 D2
// ---------------------------------------------------------------------------

/// What a request does to the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// The listing, or one object read back.
    Read,
    /// An object sent, or a prune.
    Write,
}

/// Who is asking, as the store's rule sees them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caller {
    /// A key in this machine's `peers` — the operator, by provisioning.
    Operator,
}

/// **The whole of R-184 D2**: the operator reads and writes, and nobody else is
/// a caller yet. A member fetch is a new [`Caller`] and a new row here.
#[must_use]
pub const fn permits(caller: Caller, access: Access) -> bool {
    match (caller, access) {
        (Caller::Operator, Access::Read | Access::Write) => true,
    }
}

// ---------------------------------------------------------------------------
// what a manifest names
// ---------------------------------------------------------------------------

/// **The digests a manifest names that this store holds** — every
/// `[[artifact]]` whose kind [`holds_kind`], by digest, except a hidden
/// product's.
///
/// `entries` is `(digest, product, kind)` per `[[artifact]]`, and `hidden` is
/// `(product, kind)` per `[[hidden]]`, as whoever read the TOML found them.
///
/// **A hidden product's objects are not named** (`ADR-052` §7): a hidden
/// product stays in the fleet repository and is withheld from every fleet, so
/// a store holding one prunes it, and a store lacking one does not count it
/// missing.
///
/// # Errors
///
/// A digest that is not 64 lowercase hex characters, or a kind that does not
/// parse, naming the entry. **A manifest that does not read is refused whole**
/// rather than read around: a prune decided from part of a manifest removes
/// what the unread part names.
pub fn named<'a>(
    entries: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>,
    hidden: &[(&str, &str)],
) -> Result<BTreeSet<Digest>, String> {
    let mut out = BTreeSet::new();
    for (at, (digest, product, kind_text)) in entries.into_iter().enumerate() {
        let kind = Kind::parse(kind_text)
            .map_err(|e| format!("[[artifact]] {} has a kind that does not read: {e}", at + 1))?;
        let parsed = Digest::new(digest).ok_or_else(|| {
            format!(
                "[[artifact]] {} names {digest:?}, which is not 64 lowercase hex characters",
                at + 1
            )
        })?;
        let withheld = hidden.iter().any(|(p, k)| *p == product && *k == kind_text);
        if holds_kind(&kind) && !withheld {
            out.insert(parsed);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// the listing
// ---------------------------------------------------------------------------

/// What the server's own manifest said, or why it could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Manifest {
    /// `main` was read, at this commit, and names these digests.
    Read {
        commit: String,
        named: BTreeSet<Digest>,
    },
    /// Nothing the store's rule can be decided from. Carries the reason, on
    /// one line.
    Unavailable(String),
}

/// One object the store holds.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Held {
    pub digest: Digest,
    /// The `ADR-027` name it arrived under.
    pub name: String,
}

/// **The answer to `GET /repository-objects/`** — what the store holds and
/// what the server's manifest names. The completion signal: a sync is complete
/// when nothing named is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub manifest: Manifest,
    pub held: Vec<Held>,
}

/// The listing's line words. One word, then fields separated by one space.
const COMMIT: &str = "manifest";
const UNAVAILABLE: &str = "manifest-unavailable";
const NAMED: &str = "named";
const HELD: &str = "held";

impl Listing {
    /// The text on the wire. `#` lines are for a person and are ignored by
    /// [`Listing::parse`].
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = format!(
            "# a fleet server's repository objects (R-184 D1): what the store holds, and what \
             the {REPOSITORY} repository's {BRANCH}:{MANIFEST_FILE} names\n"
        );
        match &self.manifest {
            Manifest::Read { commit, named } => {
                out.push_str(&format!("{COMMIT} {commit}\n"));
                for digest in named {
                    out.push_str(&format!("{NAMED} {digest}\n"));
                }
            }
            Manifest::Unavailable(why) => {
                out.push_str(&format!("{UNAVAILABLE} {}\n", one_line(why)));
            }
        }
        let mut held = self.held.clone();
        held.sort();
        for one in &held {
            out.push_str(&format!("{HELD} {} {}\n", one.digest, one.name));
        }
        out
    }

    /// Read [`Listing::render`]'s text back.
    ///
    /// # Errors
    ///
    /// A line this build does not know, a digest that is not one, or a listing
    /// that says nothing about the manifest — each quoted. **Refused rather
    /// than read around**: a listing missing a `held` line would send an object
    /// again, and one missing a `named` line would make a sync look complete.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut commit: Option<String> = None;
        let mut unavailable: Option<String> = None;
        let mut named = BTreeSet::new();
        let mut held = Vec::new();
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
            let digest = |text: &str| {
                Digest::new(text).ok_or_else(|| format!("{text:?} in {line:?} is not a digest"))
            };
            match word {
                COMMIT => commit = Some(rest.to_owned()),
                UNAVAILABLE => unavailable = Some(rest.to_owned()),
                NAMED => {
                    named.insert(digest(rest)?);
                }
                HELD => {
                    let (d, name) = rest
                        .split_once(' ')
                        .ok_or_else(|| format!("{line:?} carries no name after its digest"))?;
                    held.push(Held { digest: digest(d)?, name: name.to_owned() });
                }
                other => return Err(format!("{other:?} is not a line this build reads: {line:?}")),
            }
        }
        let manifest = match (commit, unavailable) {
            (Some(commit), None) => Manifest::Read { commit, named },
            (None, Some(why)) => Manifest::Unavailable(why),
            _ => {
                return Err(
                    "the listing says neither which manifest it was read against nor why none \
                     could be, so nothing is decided from it"
                        .to_owned(),
                );
            }
        };
        Ok(Self { manifest, held })
    }

    /// Whether the store holds `digest`.
    #[must_use]
    pub fn holds(&self, digest: &Digest) -> bool {
        self.held.iter().any(|h| &h.digest == digest)
    }

    /// Every digest the manifest names that the store does not hold, in order.
    /// Empty when the manifest could not be read.
    #[must_use]
    pub fn missing(&self) -> Vec<Digest> {
        match &self.manifest {
            Manifest::Read { named, .. } => {
                named.iter().filter(|d| !self.holds(d)).cloned().collect()
            }
            Manifest::Unavailable(_) => Vec::new(),
        }
    }

    /// **What a prune would remove**: every object the manifest does not name.
    ///
    /// # Errors
    ///
    /// **A prune is refused while anything named is missing**, and that is how
    /// pruning never runs in the middle of a sync: a store still receiving what
    /// the manifest names is incomplete, and the server says so rather than
    /// trusting a client to have finished. Refused too when the manifest could
    /// not be read, because a manifest that names nothing would remove
    /// everything.
    pub fn prune(&self) -> Result<Vec<Held>, String> {
        let (commit, named) = match &self.manifest {
            Manifest::Read { commit, named } => (commit, named),
            Manifest::Unavailable(why) => {
                return Err(format!(
                    "nothing is pruned: the {REPOSITORY} repository's manifest could not be read \
                     ({why}), and a rule decided from no manifest would remove every object"
                ));
            }
        };
        let missing = self.missing();
        if !missing.is_empty() {
            return Err(format!(
                "nothing is pruned: the manifest at {commit} names {} object(s) this store does \
                 not hold yet, so a sync is not complete. Pruning runs only against a complete \
                 store. Missing: {}",
                missing.len(),
                missing.iter().map(Digest::as_str).collect::<Vec<_>>().join(", ")
            ));
        }
        let mut out: Vec<Held> =
            self.held.iter().filter(|h| !named.contains(&h.digest)).cloned().collect();
        out.sort();
        Ok(out)
    }
}

/// A reason, flattened onto the one line the listing gives it.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "repository_objects_tests.rs"]
mod tests;
