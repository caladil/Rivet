//! `cargo rivet fleet drift` — **`ADR-052` §9 step 3: ask a machine what it
//! is running, and say how that differs from what the fleet repository says.**
//!
//! `ADR-052` §7 is why this is worth having at all: once the repository names
//! what a machine should be running, *"`rivet-01` is running an artifact that
//! is not in the repository"* becomes a question with an answer. §1's own
//! symptom is the motivating case — three applications on the maintainer's
//! board stamped `-dirty-<digest>`, because the build that produced them was
//! ambiguous and nothing recorded which artifact was actually sent.
//!
//! # What is genuinely comparable, and what is not
//!
//! **This module compares two things: which artifact an application is
//! running, and which shared rivets a machine holds, against what the fleet
//! repository's manifest says it published.** That is deliberately narrower
//! than everything `get status` can report, and the narrowing is the point
//! rather than a shortcut:
//!
//! * `rivet_core::operation::status::Status` carries a service's name, whether
//!   it is enabled, its state, restart count, health word, caption and whether
//!   the document calls it critical. **None of that has a counterpart in the
//!   fleet repository** — the manifest records what was published, never what
//!   is healthy, restarting or critical, so there is nothing on the
//!   repository's side to compare those fields against. Reporting a
//!   `degraded` service as "drift" would be inventing a question the
//!   repository was never asked.
//! * `Status` carries no version for a service at all — investigated rather
//!   than assumed, and confirmed absent by reading the type itself.
//! * `get file` against an installed application's own directory does carry
//!   one. `rivet_core::package::install::Plan` unpacks an application to
//!   `/state/app/<name>/<version>/` and swaps `/state/app/<name>/current` — a
//!   symlink — onto it as the last, atomic step. So `get file` on that
//!   symlink's `link_target` is the version this machine is actually running,
//!   read the same way an operator would: `ADR-006` §8.10.4.3 already reports
//!   a symlink AS a link, unresolved, for exactly this kind of question.
//!
//! **So the comparison is: for each service `get status` names, ask what
//! `/state/app/<name>/current` points at, and look up that product in the
//! manifest.** A service without that symlink is not a gap in this module —
//! it is a service the image defines rather than an application, and an
//! image-defined service is outside what a fleet repository of `.rvt`
//! artifacts could ever name a version for. It reads as `NOT COMPARABLE`
//! rather than as a difference, which is `SPEC-05` §2's rule applied here: an
//! absence of information is not evidence of a mismatch.
//!
//! # Shared rivets are compared too, and more exactly than applications
//!
//! `PLAN-Feature-Set-2026-09-12` §2 decision 7: shared rivets are compared by
//! drift. A library is not a service, so `get status` never names one; it is a
//! directory under `/state/lib` whose name is its **reference** —
//! `<name>-<version>-<digest>` (`SPEC-02` §3) — and there is no `current`
//! symlink to read (`SPEC-02` §4). So the machine side is one `get file
//! /state/lib`, and every entry that parses as a reference is one library.
//!
//! **The repository side is the same reference, derived from the stored
//! bytes.** The manifest's `digest` is BLAKE3 over the WHOLE file — the object
//! store's key — and a reference's digest is BLAKE3 over the header's signed
//! region; they are different numbers for one artifact. Rather than store the
//! second in the manifest, each published library's reference is recomputed
//! from its object's header with `crate::library::reference_of`, the function
//! `cargo rivet library` prints references with. A machine's reference either
//! equals one of those strings or it does not, so MATCHES means the exact
//! artifact and not merely the same version string.
//!
//! **A non-match is one heading, `LIBRARY NOT MATCHED`, since `.rulings/`
//! R-157 R17.** It used to be split into DIFFERS and NOT IN THE FLEET
//! REPOSITORY by whether the reference's name matched a published product —
//! two headings for one verdict, which read as though the split itself
//! decided something. It never did: `SPEC-02` §3 says a reference's name is
//! "a convenience for a person," and printing that convenience as if it were
//! two different findings overstated what the prefix check actually knows. A
//! non-match now gets one heading and, where a published library's name
//! followed by `-` prefixes the installed reference's `<name>-<version>`, a
//! labelled HINT beside it — *"closest published name: libgreet (a guess from
//! the name)"* — never a decision. Where two published products are both a
//! prefix, the longer wins.
//!
//! # A machine that does not answer is not drift
//!
//! `compare` never turns a transport failure into a finding. The first call
//! — `get status` — either answers or this returns an error saying the
//! machine did not answer at all; nothing about drift is reported for a
//! machine nothing was heard from. A `get file` that fails once connected is
//! narrower still: it is reported against that one service, in its own
//! column, and never folded into MATCHES, DIFFERS or NOT COMPARABLE — a
//! reading that could not be taken is not the same claim as a reading that was
//! taken and agreed.
//!
//! # No JSON parser, on purpose — `rivet-config`'s own ruling
//!
//! This workspace carries no JSON parser by rule, so the two calls below read
//! `get status` and `get file`'s answers through `rivet_remote::tls`'s
//! field-by-field decoder — the same one `get document` already used, widened
//! rather than duplicated (`ADR-052` §9's own note on that module).
//!
//! # Nothing here writes anything, to the machine or to the repository
//!
//! Every call this module makes is a `get`. `ADR-052` §9 step 3 is read-only
//! by its own ordering — drift is a question, and answering one with a write
//! would be a different and much larger decision than this step is.

use std::path::Path;

use rivet_core::package::library::Reference;
use rivet_remote::endpoint::Endpoint;
use rivet_remote::tls::{CallsTools, json_bool_field, json_string_field, json_string_field_all};

/// `cargo rivet fleet drift --host H:PORT --identity DIR --pin HEX [--at DIR]`,
/// with a connection behind it — [`compare`] is the testable half that takes
/// any [`CallsTools`], and this is the composition root that opens a real one.
///
/// # Errors
///
/// No identity or pin was given (`ADR-025` §2's trust is mutual and there is
/// no second transport to fall back to — the same refusal `cargo rivet run`
/// gives), or [`compare`]'s own errors.
pub(super) fn run(root: &Path, endpoint: &Endpoint) -> Result<String, String> {
    let transport = endpoint.open()?.ok_or_else(|| {
        "--identity and --pin are required: `fleet drift` speaks ADR-035's control plane and has \
         no second transport to fall back to. `cargo rivet identity new` mints an identity; \
         `cargo rivet key <seed>` derives a machine's pin from its seed file."
            .to_string()
    })?;
    compare(root, &transport)
}

/// One service `get status` named, and what this module could tell about the
/// artifact it is running.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Finding {
    /// `/state/app/<name>/current` points at a version the manifest also
    /// names for this product.
    Matches { version: String },
    /// The symlink names a version, and the manifest carries no entry for
    /// this product at all — `ADR-052` §1's own symptom, read back off a
    /// real machine.
    NotInRepository { version: String },
    /// The symlink names a version, the manifest knows this product, and
    /// none of its entries carry this exact version.
    Differs { version: String, repository_versions: Vec<String> },
    /// No symlink to read — an image-defined service, most likely, and its
    /// version is not something a fleet repository of `.rvt` artifacts was
    /// ever asked to record.
    NotComparable,
}

/// Where a machine keeps its shared rivets — `SPEC-02` §4, one directory per
/// reference.
const LIBRARY_ROOT: &str = "/state/lib";

/// One reference `/state/lib` holds, and what the fleet repository says about
/// it. See this module's documentation for how each is decided.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LibraryFinding {
    /// The repository holds an artifact whose reference is this exact string.
    Matches,
    /// No published reference is this exact string — `.rulings/` R-157 R17's
    /// one heading for every non-match. `hint` is a guess from the name only,
    /// never part of deciding this is a non-match.
    NotMatched { hint: Option<Hint> },
}

/// **A guess, and labelled as one** (`.rulings/` R-157 R17): the repository
/// holds no reference equal to the one being reported, but it does hold
/// `product`, whose name followed by `-` prefixes the reference's
/// `<name>-<version>` (`SPEC-02` §3.2). `references` is every one of that
/// product's references the repository holds, in manifest order, so a reader
/// can see whether this is another version or another build of the same one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Hint {
    product: String,
    references: Vec<String>,
}

/// What `get file /state/lib` answered, before anything is compared.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Installed {
    /// Every entry that parses as a reference, in the machine's order.
    references: Vec<Reference>,
    /// Every entry that does not — named in the report, never compared,
    /// because a name that is not a reference names no artifact.
    other: Vec<String>,
    /// The machine cut the listing (`get file`'s `entries_truncated`), so
    /// what follows it was never seen and is not reported as absent.
    truncated: bool,
}

/// The libraries half of the report.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Libraries {
    findings: Vec<(Reference, LibraryFinding)>,
    other: Vec<String>,
    truncated: bool,
}

/// **The whole comparison, against any [`CallsTools`].** Takes no ownership of
/// a connection, so a fake in a test exercises every branch below with no
/// socket and no board — the same seam `rivet_remote::tls`'s own document
/// methods are tested through.
///
/// # Errors
///
/// The manifest cannot be read (`super::read_manifest`'s own errors), a
/// library it names has no readable object to derive a reference from, or
/// `get status` itself did not answer — see this module's documentation for
/// why that is refused rather than reported as an empty comparison. A `get
/// file` that fails for one particular service does **not** stop this: it is
/// recorded against that service and everything else is still reported, and
/// the same is true of the one `get file` that lists `/state/lib`.
pub(super) fn compare(root: &Path, caller: &dyn CallsTools) -> Result<String, String> {
    let entries = super::read_manifest(root)?;
    // Before the machine is asked anything: an inconsistent repository is this
    // workstation's fault, and says so without a connection being spent on it.
    let published = published_libraries(root, &entries)?;
    let names = service_names(caller)?;

    let mut findings: Vec<(String, Finding)> = Vec::new();
    let mut unreadable: Vec<(String, String)> = Vec::new();

    for name in names {
        match installed_version(caller, &name) {
            Ok(None) => findings.push((name, Finding::NotComparable)),
            Ok(Some(version)) => {
                let candidates: Vec<&str> = entries
                    .iter()
                    .filter(|e| e.product == name && e.kind == "application")
                    .map(|e| e.version.as_str())
                    .collect();
                let finding = if candidates.is_empty() {
                    Finding::NotInRepository { version }
                } else if candidates.contains(&version.as_str()) {
                    Finding::Matches { version }
                } else {
                    Finding::Differs {
                        version,
                        repository_versions: candidates.into_iter().map(String::from).collect(),
                    }
                };
                findings.push((name, finding));
            }
            Err(why) => unreadable.push((name, why)),
        }
    }

    let libraries = installed_libraries(caller).map(|installed| Libraries {
        findings: installed
            .references
            .into_iter()
            .map(|reference| {
                let finding = library_finding(&reference, &published);
                (reference, finding)
            })
            .collect(),
        other: installed.other,
        truncated: installed.truncated,
    });

    Ok(render(root, &findings, &unreadable, &libraries))
}

/// Every library the manifest names, as `(product, reference)` — the reference
/// recomputed from the stored object's own header, never read from a name.
///
/// # Errors
///
/// The manifest names a library whose bytes are not in `objects/`, or whose
/// header cannot be read — the repository is inconsistent, and a comparison
/// that silently skipped the entry would report its library as unpublished.
fn published_libraries(
    root: &Path,
    entries: &[super::Entry],
) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    for entry in entries.iter().filter(|e| e.kind == "library") {
        let object = root.join(super::OBJECTS_DIR).join(&entry.digest);
        if !object.is_file() {
            return Err(format!(
                "the fleet repository's manifest names the library {} {} ({}) and its bytes are \
                 not at {} — the repository is inconsistent, so its reference cannot be derived \
                 and no drift is reported. Nothing was asked of the machine.",
                entry.product,
                entry.version,
                entry.digest,
                object.display(),
            ));
        }
        let reference = crate::library::reference_of(&object, &entry.product, &entry.version)?;
        out.push((entry.product.clone(), reference.to_string()));
    }
    Ok(out)
}

/// What `/state/lib` holds, from one `get file`.
///
/// **Nothing there is an answer, not an error**: the installer creates
/// `/state/lib` when it writes the first library, so a machine without one
/// holds none.
///
/// # Errors
///
/// `get file` did not answer, or `/state/lib` is something other than a
/// directory — either way the libraries are reported as unreadable, and the
/// services beside them still are not.
fn installed_libraries(caller: &dyn CallsTools) -> Result<Installed, String> {
    let answer = rivet_remote::command::perform(caller, &["get", "file", LIBRARY_ROOT])
        .map_err(|why| format!("`get file {LIBRARY_ROOT}` did not answer: {why}"))?;
    if !json_bool_field(&answer, "present")? {
        return Ok(Installed::default());
    }
    // The FIRST `type` is the path's own — `get file` writes it before
    // `entries`, whose elements carry a `type` each.
    let kind = json_string_field(&answer, "type")?;
    if kind != "directory" {
        return Err(format!(
            "{LIBRARY_ROOT} is a {kind} and not a directory, so it lists no library"
        ));
    }
    // Every `name` in a `get file` answer is a directory entry's: nothing else
    // that subject writes carries the key.
    let mut installed = Installed {
        truncated: json_bool_field(&answer, "entries_truncated")?,
        ..Installed::default()
    };
    for name in json_string_field_all(&answer, "name")? {
        match Reference::parse(&name) {
            Ok(reference) => installed.references.push(reference),
            Err(_) => installed.other.push(name),
        }
    }
    Ok(installed)
}

/// How one installed reference stands against the repository's. A match is
/// string equality and nothing else; see this module's documentation for the
/// hint that never decides one.
fn library_finding(reference: &Reference, published: &[(String, String)]) -> LibraryFinding {
    if published.iter().any(|(_, r)| r == reference.as_str()) {
        return LibraryFinding::Matches;
    }
    let name_and_version = reference.name_and_version();
    let product = published
        .iter()
        .map(|(product, _)| product.as_str())
        .filter(|product| {
            name_and_version
                .strip_prefix(product)
                .is_some_and(|rest| rest.len() > 1 && rest.starts_with('-'))
        })
        .max_by_key(|product| product.len());
    let hint = product.map(|product| Hint {
        product: product.to_string(),
        references: published
            .iter()
            .filter(|(p, _)| p == product)
            .map(|(_, r)| r.clone())
            .collect(),
    });
    LibraryFinding::NotMatched { hint }
}

/// Every service `get status` currently names, in the order the machine gave
/// them.
///
/// **A machine that does not answer this call is not drift** — see this
/// module's documentation. The error names the call that failed and says in
/// as many words that nothing about drift follows from it, so a caller
/// reading only the error text still gets the distinction right.
fn service_names(caller: &dyn CallsTools) -> Result<Vec<String>, String> {
    let answer = rivet_remote::command::perform(caller, &["get", "status"]).map_err(|why| {
        format!(
            "the machine did not answer `get status`: {why}\n\nA MACHINE THAT DID NOT ANSWER IS \
             NOT DRIFT (ADR-052 §9). Nothing here says whether it matches the fleet repository \
             or differs from it — only that nothing was heard back, which is a different claim."
        )
    })?;
    json_string_field_all(&answer, "name")
}

/// What `/state/app/<name>/current` says this machine is running.
///
/// `Ok(None)` covers every shape that is not "a symlink, present" — absent
/// entirely, or present as something else — and all of them read as `NOT
/// COMPARABLE` rather than a guess. See this module's documentation for why
/// an absent application directory is the ordinary answer for an
/// image-defined service and not a gap in this reading.
///
/// # Errors
///
/// `get file` itself did not answer — a transport failure once already
/// connected, reported against this one service by [`compare`] rather than
/// stopping the rest of the comparison.
fn installed_version(caller: &dyn CallsTools, name: &str) -> Result<Option<String>, String> {
    let path = format!("/state/app/{name}/current");
    let answer = rivet_remote::command::perform(caller, &["get", "file", &path])
        .map_err(|why| format!("`get file {path}` did not answer: {why}"))?;
    if !json_bool_field(&answer, "present")? {
        return Ok(None);
    }
    if json_string_field(&answer, "type")? != "symlink" {
        // Present, but not ADR-015 §7's atomic activation link — some other
        // shape at a path this build did not expect. Read as "nothing to
        // compare" for `get file`'s own reason: `AGENTS.md` §5 refuses rather
        // than guesses, and a version cannot be guessed from a shape.
        return Ok(None);
    }
    Ok(Some(json_string_field(&answer, "link_target")?))
}

/// The report, as an operator reads it — one section per [`Finding`] kind,
/// each printed only with what it holds, plus the services this end could not
/// even ask, and then the libraries in their own sections.
fn render(
    root: &Path,
    findings: &[(String, Finding)],
    unreadable: &[(String, String)],
    libraries: &Result<Libraries, String>,
) -> String {
    let mut matches = Vec::new();
    let mut not_in_repository = Vec::new();
    let mut differs = Vec::new();
    let mut not_comparable = Vec::new();
    for (name, finding) in findings {
        match finding {
            Finding::Matches { version } => matches.push((name, version)),
            Finding::NotInRepository { version } => not_in_repository.push((name, version)),
            Finding::Differs { version, repository_versions } => {
                differs.push((name, version, repository_versions));
            }
            Finding::NotComparable => not_comparable.push(name),
        }
    }

    let mut out = format!("drift against {} — ADR-052 §9 step 3\n", root.display());

    out.push_str(&format!(
        "\nMATCHES {} — running the version the fleet repository names for it\n",
        matches.len()
    ));
    for (name, version) in &matches {
        out.push_str(&format!("  {name} {version}\n"));
    }

    out.push_str(&format!(
        "\nNOT IN THE FLEET REPOSITORY {} — running an artifact this repository has never \
         published (ADR-052 §1)\n",
        not_in_repository.len()
    ));
    for (name, version) in &not_in_repository {
        out.push_str(&format!("  {name} {version}\n"));
    }

    out.push_str(&format!(
        "\nDIFFERS {} — running a version the repository does not hold for this product\n",
        differs.len()
    ));
    for (name, version, repository_versions) in &differs {
        out.push_str(&format!(
            "  {name} runs {version}; the repository holds {}\n",
            repository_versions.join(", ")
        ));
    }

    out.push_str(&format!(
        "\nNOT COMPARABLE {} — no application directory to read a version from, most likely an \
         image-defined service this repository was never asked to publish\n",
        not_comparable.len()
    ));
    for name in &not_comparable {
        out.push_str(&format!("  {name}\n"));
    }

    if !unreadable.is_empty() {
        out.push_str(&format!(
            "\nCOULD NOT BE READ {} — get file failed once connected; not counted as matching, \
             differing, or not comparable\n",
            unreadable.len()
        ));
        for (name, why) in unreadable {
            out.push_str(&format!("  {name}: {why}\n"));
        }
    }

    let library_count = render_libraries(&mut out, libraries);

    out.push_str(&format!(
        "\n{} service(s) read from `get status`. Each was asked `get file \
         /state/app/<name>/current`: a symlink's target is the version running, which is the \
         one fact both sides of this comparison actually hold — health, restarts and whether a \
         service is critical are `get status`'s alone and the fleet repository has no opinion \
         about them, so none of that is reported here as drift.\n",
        findings.len() + unreadable.len()
    ));
    if let Some(count) = library_count {
        out.push_str(&format!(
            "{count} library reference(s) read from `get file {LIBRARY_ROOT}`, each compared \
             with the reference derived from every library the fleet repository holds — name, \
             version and the digest of the signed header (SPEC-02 §3).\n"
        ));
    }

    out
}

/// The libraries' sections, appended to `out`. Returns how many references
/// were compared, or `None` when the listing could not be read at all.
///
/// **Every heading starts `LIBRARY`**, so a reader — or a test — looking for
/// the application headings above never reads a library count by accident.
fn render_libraries(out: &mut String, libraries: &Result<Libraries, String>) -> Option<usize> {
    let libraries = match libraries {
        Ok(libraries) => libraries,
        Err(why) => {
            out.push_str(&format!(
                "\nLIBRARIES COULD NOT BE READ — {why}. Not counted as matching, differing or \
                 absent: nothing is known about this machine's shared rivets.\n"
            ));
            return None;
        }
    };

    let mut matches = Vec::new();
    let mut not_matched = Vec::new();
    for (reference, finding) in &libraries.findings {
        match finding {
            LibraryFinding::Matches => matches.push(reference),
            LibraryFinding::NotMatched { hint } => not_matched.push((reference, hint)),
        }
    }

    out.push_str(&format!(
        "\nLIBRARY MATCHES {} — installed under {LIBRARY_ROOT}, and the fleet repository holds \
         this exact artifact\n",
        matches.len()
    ));
    for reference in &matches {
        out.push_str(&format!("  {reference}\n"));
    }

    out.push_str(&format!(
        "\nLIBRARY NOT MATCHED {} — installed under {LIBRARY_ROOT}, and no reference this fleet \
         repository publishes is this exact string (`.rulings/` R-157 R17)\n",
        not_matched.len()
    ));
    for (reference, hint) in &not_matched {
        match hint {
            None => out.push_str(&format!("  {reference}\n")),
            Some(Hint { product, references }) => out.push_str(&format!(
                "  {reference} — closest published name: {product} (a guess from the name); \
                 repository holds {}\n",
                references.join(", ")
            )),
        }
    }

    if !libraries.other.is_empty() {
        out.push_str(&format!(
            "\nLIBRARY DIRECTORY ENTRIES THAT ARE NOT REFERENCES {} — under {LIBRARY_ROOT} but \
             named as no artifact is (SPEC-02 §3), so not compared\n",
            libraries.other.len()
        ));
        for name in &libraries.other {
            out.push_str(&format!("  {name}\n"));
        }
    }

    if libraries.truncated {
        out.push_str(&format!(
            "\nLIBRARY LISTING CUT BY THE MACHINE — `get file {LIBRARY_ROOT}` named only part of \
             the directory. The entries it did not name were NOT compared, and are not reported \
             as absent.\n"
        ));
    }

    Some(libraries.findings.len())
}

#[cfg(test)]
#[path = "fleet_drift_tests.rs"]
mod tests;
