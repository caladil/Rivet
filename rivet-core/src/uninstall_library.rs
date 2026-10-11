//! **Removing an installed library on purpose.** The operator's verb is
//! `rivet apply uninstall-library <selector> [confirm]`; this module holds its
//! request channel and the decision both of its halves make.
//!
//! # A twin of `crate::uninstall`, not a generalisation of it
//!
//! The channel is that one's shape exactly: one line on `/state`, mode `0600`,
//! written to a temporary name and renamed into place, read once per
//! convergence pass by the supervisor and removed before it is acted on. It is
//! a second file rather than a second kind of line in that one, because the two
//! name different things — an application, a library build — are refused for
//! different reasons and leave different things behind. A request file whose
//! reader had to ask which kind of line it held would say *"a request"* in every
//! sentence it wrote, which is the vaguer wording `AGENTS.md` §5 exists to
//! prevent.
//!
//! # What a selector selects
//!
//! A **product name** selects every build of that product under `<volume>/lib`
//! — [`Reference::product`]'s grouping, the one retention keeps one build per.
//! A **full reference** selects exactly that build. Either way the selector is
//! one path component by construction: a product name obeys the rules a
//! reference's name half obeys
//! ([`crate::package::library::check_component`]).
//!
//! # Three refusals, and one thing that is a report instead
//!
//! The decision is [`select`] and [`decide`], with no I/O; [`survey`] reads the
//! machine for them through [`closure::Tree`]. The command that plans the
//! removal and the supervisor that performs it both call [`survey`], so the plan
//! and the act cannot disagree about what is refused:
//!
//! * **nothing installed matches** the selector;
//! * **an installed application's declared closure names a build** this would
//!   remove. The whole request is refused, naming every application and build,
//!   because that application resolves the build by exact reference at every
//!   start (`SPEC-02` §5) and removing it stops the application at its next;
//! * **an installed application's manifest cannot be read**, because a closure
//!   computed without it has a hole in it ([`closure`]'s argument).
//!
//! **An application that merely accepts the library's class is not a refusal.**
//! Nothing names a class-joined addon — the machine joined it by reading
//! manifests — and removing one on purpose is what this verb is for. Each such
//! application is reported instead, so the operator sees what loses an addon.

use crate::deploy::Principal;
use crate::operation::machine::MachineError;
use crate::package::closure::{self, Declaring};
use crate::package::install::{lib_root, library_dir};
use crate::package::library::{self, Reference, ReferenceFault};

/// Where the request is written. Read and removed by `cmd/rivet-sysd`.
pub const REQUEST_PATH: &str = "/state/uninstall-library-request";

/// The temporary half of the write-sync-rename that produces it.
pub const REQUEST_TEMP_PATH: &str = "/state/uninstall-library-request.new";

/// `0600`, as `crate::uninstall::REQUEST_MODE` is and for the same reason: it
/// names a principal, and nothing that is not root has any business reading who
/// asked for what.
pub const REQUEST_MODE: u32 = 0o600;

/// The largest request this build will read. A selector and a principal; a
/// selector is at most [`library::MAX_REFERENCE`] bytes.
pub const REQUEST_MAX_BYTES: u64 = 1024;

// ---------------------------------------------------------------------------
// The selector
// ---------------------------------------------------------------------------

/// What an operator named for removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    /// Every build of one library product.
    Product(String),
    /// Exactly one build.
    Build(Reference),
}

/// Why a word is not a selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorProblem {
    /// The word, carried back verbatim.
    pub text: String,
    fault: ReferenceFault,
}

impl SelectorProblem {
    /// One sentence, naming the rule the word broke.
    #[must_use]
    pub fn why(&self) -> String {
        let rule = match self.fault {
            ReferenceFault::BadLength => format!(
                "it is empty or longer than {} characters",
                library::MAX_REFERENCE
            ),
            _ => "a library name is letters, digits, `.`, `_` and `-`, does not begin with `.` \
                  or `-`, and never contains `..`"
                .to_string(),
        };
        format!(
            "{text:?} is neither a full library reference (`<name>-<version>-<64 lowercase \
             hex>`) nor a usable product name, so nothing under /state/lib could correspond to \
             it: {rule}",
            text = self.text
        )
    }
}

impl Selector {
    /// Read one word as a selector.
    ///
    /// **A full reference first**, because every reference is also shaped like
    /// a product name and the more specific reading is the one meant. Anything
    /// else must pass [`library::check_component`], which is what stops `..`, a
    /// leading `.` or `-` and a path separator from ever reaching a path.
    ///
    /// # Errors
    ///
    /// [`SelectorProblem`] for a word that is neither.
    pub fn parse(text: &str) -> Result<Self, SelectorProblem> {
        if let Ok(reference) = Reference::parse(text) {
            return Ok(Self::Build(reference));
        }
        match library::check_component(text) {
            Ok(()) => Ok(Self::Product(text.to_string())),
            Err(fault) => Err(SelectorProblem {
                text: text.to_string(),
                fault,
            }),
        }
    }

    /// Whether `reference` is a build this selects.
    #[must_use]
    pub fn matches(&self, reference: &Reference) -> bool {
        match self {
            Self::Product(product) => reference.product() == product,
            Self::Build(build) => build == reference,
        }
    }

    /// The word, as the operator typed it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Product(product) => product,
            Self::Build(build) => build.as_str(),
        }
    }
}

impl core::fmt::Display for Selector {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// The request
// ---------------------------------------------------------------------------

/// What an operator asked to be removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Already checked by [`Selector::parse`], so it is one path component.
    pub selector: Selector,
    /// Who the writing process said asked. Attribution, not authority —
    /// [`Principal`]'s own first paragraph.
    pub principal: Principal,
}

/// Why a request on disk is not one. Closed, per `ADR-006` §6.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RequestProblem {
    Empty,
    TooLarge { bytes: usize },
    MoreThanOneLine,
    ControlCharacter { at: usize },
    /// The selector is not one — which is also what stops a request naming
    /// `../../system`.
    UnusableSelector { why: String },
    /// A word after the selector. This request carries exactly one.
    UnknownWord { word: String },
    /// The principal half did not parse, in `crate::deploy`'s own words.
    Principal { why: String },
}

impl RequestProblem {
    /// One sentence, for the journal.
    #[must_use]
    pub fn why(&self) -> String {
        match self {
            Self::Empty => "it is empty".to_string(),
            Self::TooLarge { bytes } => format!(
                "it is {bytes} bytes and a request may be at most {REQUEST_MAX_BYTES}"
            ),
            Self::MoreThanOneLine => "it has more than one line".to_string(),
            Self::ControlCharacter { at } => {
                format!("it carries a control character at byte {at}")
            }
            Self::UnusableSelector { why } | Self::Principal { why } => why.clone(),
            Self::UnknownWord { word } => format!(
                "there is a {word:?} after the selector and a library uninstall request carries \
                 nothing else"
            ),
        }
    }
}

/// Render a request. The inverse of [`parse_request`], and the only writer.
#[must_use]
pub fn render_request(selector: &Selector, principal: &Principal) -> String {
    // The space before the marker is load-bearing, for `crate::uninstall`'s
    // reason: the principal is read a whitespace token at a time.
    format!(
        "{} {} {selector}\n",
        principal.render_fields(),
        crate::deploy::PAYLOAD_MARKER
    )
}

/// Read one back.
///
/// **The selector is validated here and not by the caller**, because this is
/// the boundary a hostile file crosses.
///
/// # Errors
///
/// [`RequestProblem`], one per way a file can fail to be a request.
pub fn parse_request(contents: &str) -> Result<Request, RequestProblem> {
    let cap = usize::try_from(REQUEST_MAX_BYTES).unwrap_or(usize::MAX);
    if contents.len() > cap {
        return Err(RequestProblem::TooLarge {
            bytes: contents.len(),
        });
    }
    let body = contents
        .strip_suffix('\n')
        .map_or(contents, |s| s.strip_suffix('\r').unwrap_or(s));
    if body.is_empty() {
        return Err(RequestProblem::Empty);
    }
    if body.contains('\n') || body.contains('\r') {
        return Err(RequestProblem::MoreThanOneLine);
    }
    if let Some(at) = body.bytes().position(|b| b < 0x20 || b == 0x7f) {
        return Err(RequestProblem::ControlCharacter { at });
    }

    let (principal, rest) = crate::deploy::split_principal(body)
        .map_err(|problem| RequestProblem::Principal { why: problem.why() })?;

    let mut words = rest.split_whitespace();
    let Some(word) = words.next() else {
        return Err(RequestProblem::Empty);
    };
    let selector = Selector::parse(word)
        .map_err(|problem| RequestProblem::UnusableSelector { why: problem.why() })?;
    if let Some(word) = words.next() {
        return Err(RequestProblem::UnknownWord {
            word: word.to_string(),
        });
    }

    Ok(Request {
        selector,
        principal,
    })
}

// ---------------------------------------------------------------------------
// The decision
// ---------------------------------------------------------------------------

/// What one selected build's own manifest says it provides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Class {
    /// It provides this class, so an application accepting it joins it.
    Provides(String),
    /// It provides none, so nothing joins it by class.
    Nothing,
    /// Its manifest could not be read, so which applications join it is
    /// unknown. Not a refusal — a class join is never load-bearing the way a
    /// declared closure is — but reported, so the account is not silently
    /// short.
    Unread(String),
}

/// An application that joins a build being removed by its class, and so loses
/// an addon at its next start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acceptor {
    pub application: String,
    pub class: String,
    pub reference: Reference,
}

/// A build being removed whose class could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unread {
    pub reference: Reference,
    pub detail: String,
}

/// An application whose declared closure names a build being removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub application: String,
    pub reference: Reference,
}

/// What a request would remove, as the machine found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    /// Every selected build, sorted.
    pub builds: Vec<Reference>,
    /// Every installed application joining one of them by class.
    pub acceptors: Vec<Acceptor>,
    /// Every one of them whose class could not be read.
    pub unread: Vec<Unread>,
}

/// Why a request is refused. **Nothing is removed for any of these.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// No installed build matches. `installed` is everything that is, so the
    /// next attempt can be right.
    NothingMatches {
        selector: Selector,
        installed: Vec<Reference>,
    },
    /// Installed applications declare builds this would remove.
    Declared { holders: Vec<Holder> },
    /// What the installed applications declare is not known.
    ClosureUnknown(closure::Unknown),
    /// The library root is there and could not be listed.
    LibrariesUnlisted { path: String, detail: String },
}

impl Refusal {
    /// The account, in the words both halves use.
    #[must_use]
    pub fn why(&self) -> String {
        match self {
            Self::NothingMatches {
                selector,
                installed,
            } => {
                let listed = if installed.is_empty() {
                    "no library at all is installed".to_string()
                } else {
                    format!(
                        "installed: {}",
                        installed
                            .iter()
                            .map(|r| format!("`{r}` (product `{}`)", r.product()))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                format!(
                    "REFUSED nothing installed under {root} matches `{selector}`, so NOTHING HAS \
                     BEEN REMOVED. A product name selects every build of that product and a full \
                     reference selects exactly one; {listed}",
                    root = lib_root(crate::service::BUILT_IN_VOLUME),
                )
            }
            Self::Declared { holders } => format!(
                "REFUSED an installed application declares a build this would remove, so \
                 NOTHING HAS BEEN REMOVED: {}. An application resolves its declared `library` \
                 closure by exact reference at every start (SPEC-02 §5), so removing one of \
                 these would stop it at its next start. Uninstall the application first, or \
                 install a version of it that no longer names the build",
                holders
                    .iter()
                    .map(|h| format!("`{}` declares `{}`", h.application, h.reference))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            Self::ClosureUnknown(unknown) => format!(
                "REFUSED {}, so which builds the installed applications declare is UNKNOWN and \
                 NOTHING HAS BEEN REMOVED. A closure computed without that manifest has a hole \
                 in it, and the hole is an application that would not start after its next \
                 restart",
                unknown.why()
            ),
            Self::LibrariesUnlisted { path, detail } => format!(
                "REFUSED {path} could not be listed ({detail}), so what is installed is UNKNOWN \
                 and NOTHING HAS BEEN REMOVED"
            ),
        }
    }

    /// The refusal as a port answers it. A declared closure is a safety
    /// interlock — retrying cannot help until the application changes — and the
    /// rest are the machine answering about itself.
    #[must_use]
    pub fn machine_error(&self) -> MachineError {
        match self {
            Self::Declared { .. } => MachineError::Unsafe(self.why()),
            _ => MachineError::Refused(self.why()),
        }
    }
}

/// The installed builds `selector` selects, sorted, or the refusal that none
/// does.
///
/// # Errors
///
/// [`Refusal::NothingMatches`], carrying everything that is installed.
pub fn select(selector: &Selector, installed: &[Reference]) -> Result<Vec<Reference>, Refusal> {
    let mut selected: Vec<Reference> = installed
        .iter()
        .filter(|r| selector.matches(r))
        .cloned()
        .collect();
    selected.sort();
    selected.dedup();
    if selected.is_empty() {
        let mut installed = installed.to_vec();
        installed.sort();
        return Err(Refusal::NothingMatches {
            selector: selector.clone(),
            installed,
        });
    }
    Ok(selected)
}

/// **The decision**, with no I/O: refuse if any application declares a build
/// being removed, and otherwise report every application joining one by class.
///
/// `builds` is every selected build with what its own manifest provides, and
/// `applications` is every installed application's declaration —
/// [`closure::declared`]'s whole answer, never a partial one.
///
/// # Errors
///
/// [`Refusal::Declared`], naming every application and build involved.
pub fn decide(builds: &[(Reference, Class)], applications: &[Declaring]) -> Result<Removal, Refusal> {
    let mut holders = Vec::new();
    for application in applications {
        for declared in &application.library {
            if builds.iter().any(|(build, _)| build == declared) {
                holders.push(Holder {
                    application: application.name.to_string(),
                    reference: declared.clone(),
                });
            }
        }
    }
    if !holders.is_empty() {
        return Err(Refusal::Declared { holders });
    }

    let mut acceptors = Vec::new();
    let mut unread = Vec::new();
    for (reference, class) in builds {
        match class {
            Class::Provides(class) => {
                for application in applications {
                    if application.accepts_classes.iter().any(|a| a == class) {
                        acceptors.push(Acceptor {
                            application: application.name.to_string(),
                            class: class.clone(),
                            reference: reference.clone(),
                        });
                    }
                }
            }
            Class::Nothing => {}
            Class::Unread(detail) => unread.push(Unread {
                reference: reference.clone(),
                detail: detail.clone(),
            }),
        }
    }

    Ok(Removal {
        builds: builds.iter().map(|(reference, _)| reference.clone()).collect(),
        acceptors,
        unread,
    })
}

/// **Read the machine and decide** — what both the command that plans a
/// removal and the supervisor that performs it call.
///
/// # Errors
///
/// Every [`Refusal`]: the library root unlistable, nothing matching, a closure
/// unknown, or a closure naming a selected build — in that order, so the
/// cheapest true refusal is the one given.
pub fn survey(
    tree: &mut dyn closure::Tree,
    volume: &str,
    selector: &Selector,
) -> Result<Removal, Refusal> {
    let root = lib_root(volume);
    let mut installed: Vec<Reference> = match tree.directories(&root) {
        // A directory under the library root that is not a reference was not
        // put there by this project — and an entry a removal set aside begins
        // with `.`, which no reference may — so neither is a candidate.
        Ok(Some(entries)) => entries
            .iter()
            .filter_map(|entry| Reference::parse(entry).ok())
            .collect(),
        Ok(None) => Vec::new(),
        Err(detail) => return Err(Refusal::LibrariesUnlisted { path: root, detail }),
    };
    installed.sort();

    let selected = select(selector, &installed)?;
    let applications = closure::declared(tree, volume).map_err(Refusal::ClosureUnknown)?;
    let builds: Vec<(Reference, Class)> = selected
        .into_iter()
        .map(|reference| {
            let class = class_of(tree, volume, &reference);
            (reference, class)
        })
        .collect();
    decide(&builds, &applications)
}

/// What one installed build's own manifest provides.
fn class_of(tree: &mut dyn closure::Tree, volume: &str, reference: &Reference) -> Class {
    let path = format!("{}/{}", library_dir(volume, reference), library::MANIFEST_NAME);
    let text = match tree.read(&path) {
        Ok(text) => text,
        Err(detail) => return Class::Unread(format!("{path}: {detail}")),
    };
    match library::Manifest::parse(text.as_bytes()) {
        Ok(manifest) => manifest
            .provides_class()
            .map_or(Class::Nothing, |class| Class::Provides(class.to_string())),
        Err(fault) => Class::Unread(format!("{path}: {fault}")),
    }
}

#[cfg(test)]
#[path = "uninstall_library_tests.rs"]
mod tests;
