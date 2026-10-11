//! **Build it, put it on a board, install it** — the sequence behind
//! `cargo rivet deploy`. `rivet-fleet` reaches a machine through this module's
//! [`Remote`] port for a document, and performs no deploy of its own.
//!
//! Extracted from `cmd/cargo-rivet` on 2026-08-26 (`ADR-034` §11.5); see this
//! crate's own documentation for why. What moved is everything below the
//! command line — the argv parsing stayed in `cargo-rivet`, because the two
//! callers parse differently and mean the same thing (`ADR-034` §11.1).
//!
//! # What this automates, and why that is worth a verb
//!
//! Every link of the chain existed before this module (`PLAN.md` §0.8). What did
//! not exist was a way to *run* them: getting an artifact onto a machine and
//! installed meant three commands typed by hand, and `PLAN.md` §14 and §16 are
//! the transcripts of doing exactly that, twice, on hardware. This is those
//! three commands, with the one check the manual sequence never did.
//!
//! ```text
//!   cargo rivet package …            →  target/rivet/<name>.rvt
//!   PUT /payload/<name>.rvt          →  the machine's artifact store
//!   rivet apply install <digest> confirm              on the machine
//! ```
//!
//! # The install names a digest, since 2026-09-12 — `ADR-052` §7
//!
//! It named the path the payload had been pushed to. That is gone: the transfer
//! puts the artifact into the machine's own artifact store under its BLAKE3
//! digest, and the install resolves that digest there. **Nothing an operator
//! types changed** — this end already computed the digest, for the transfer
//! check below, so it had the one thing the new form needs.
//!
//! `--remote-dir` is the one casualty and it is refused rather than ignored:
//! there is no longer a directory for it to name.
//!
//! # The transport is a port, and there is one implementation of it
//!
//! [`Remote`] is the seam; `crate::tls` implements it over `ADR-035` §7.2's
//! pinned control plane and nothing else does. That is `ADR-034` §11.5.1's
//! ruling — *there is nothing left at the far end* of any other channel — and it
//! is why this module holds a sequence rather than a connection.
//!
//! **Nothing about the connection is retried.** What the far end said is
//! reported as it arrives.
//!
//! # The digest check is the reason this is not a shell script
//!
//! After the push, the board is asked for the payload's BLAKE3 digest —
//! `rivet measure hash`, which is on every Rivet — and it is compared with
//! the digest of the local file. **Both ends compute it with this project's own
//! `rivet-crypto`**, so the comparison is of one algorithm's output against
//! itself rather than of two implementations that might disagree.
//!
//! Without it, a corrupted or truncated push fails later, inside the installer,
//! as a **signature refusal** — and an operator reading *"the header is not
//! signed by a key this machine trusts"* will go and look at their key. The
//! check turns that into a sentence about the transfer, at the moment the
//! transfer is what went wrong.
//!
//! It is a check on the *channel* and not on the payload. Whether the artifact
//! is any good is the installer's question and stays entirely there
//! (`ADR-029` §11: one verifier, not three).
//!
//! # It confirms, and it does not reboot
//!
//! **`confirm` is required as the last word**, and it is not ceremony copied
//! from the board. `rivet apply install` requires a confirmation
//! (`ADR-012` §7.1); a host tool that supplied that word on the operator's
//! behalf would be answering a question the operator was never asked. Without
//! it, this prints the plan — what would be built, where it would be put, what
//! would be run — and changes nothing anywhere.
//!
//! **It never restarts the board**, and for a system update that is the whole
//! design: the payload lands in the inactive slot and
//! `rivet apply trial-restart confirm` is a *separate* decision about when a
//! machine goes down, which is the operator's and is exactly the kind of thing
//! `ADR-012` §7.1 refuses to bundle into another command.
//!
//! # It waits for its own install, unless told not to
//!
//! `rivet apply install` returns before the installer has written a byte, by
//! ruling, so the machine's answer to the request says nothing about whether
//! the install happened. With [`DeployArgs::wait`] this reads `rivet get
//! install`'s `correlation` immediately before the request, waits for it to
//! change, then for that chain to finish, and reports how it ended — failing
//! when it ended badly. The next command a caller types — `cargo rivet
//! secrets`, which writes into a data directory only the installer makes — then
//! runs against an install that has happened rather than one that was asked
//! for. `wait_for_own_install` has the argument for each half.
//!
//! # `--enable`, and why it edits the document first
//!
//! **Installing an application does not start it** — ruled 2026-08-25,
//! `ADR-015` §3.2.2. `enabled` is the operator's and belongs in the
//! configuration document; a machine that ran whatever was installed would be
//! reading silence as consent. So a deploy is one statement and running it is a
//! second, and `--enable` is where the second one is made.
//!
//! It is made **in the document**, with `rivet apply edit`, and not around
//! it: `ADR-011`'s single authority survives because this tool writes to the
//! file the machine already reads. The convenience lives here, in a word the
//! operator typed, rather than in a default on the board that nobody chose.
//!
//! **The edit happens before a byte is sent**, and the order is load-bearing.
//! `cmd/rivet-sysd` re-reads the document at the moment an install finishes
//! and at no other moment while it runs (`PLAN.md` §20). A line written
//! afterwards would be accepted, correct, and ignored until the next restart.
//! Doing it first also fails in the better direction: a refused edit leaves a
//! machine with nothing installed, rather than an application nothing will
//! start.
//!
//! **The service name comes from the payload's file name**, through
//! `crate::name::product_and_kind` — the inverse of the function that composed
//! it, not a second reader of the container (`ADR-029` §11). Two things fall
//! out: a `.rvt` this tool did not name is refused rather than guessed at, and
//! a **system update is refused by kind** before anything is sent, because
//! `[services.rivet]` is not a thing.
//!
//! # What it leaves on the board
//!
//! The `.rvt`, in the machine's artifact store. It is not deleted, because
//! deleting the input to an install that may have failed removes the evidence,
//! and because rolling back to it is now a local act. What bounds it is the
//! store's own retention — three newest per product — which is the answer to
//! the question this paragraph used to wave at (*"`/state` is large enough"*).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rivet_core::package::port::Digest;
use rivet_crypto::Blake3;

/// What `--remote-dir` must be, which is what it already defaulted to.
///
/// **It stopped naming a destination on 2026-09-12.** A payload goes into the
/// machine's artifact store, at a path the machine computes from the bytes'
/// own digest; `run` refuses any other value rather than accepting a flag it
/// would then ignore. The constant is kept because `cargo rivet deploy` still
/// fills the field, and because the refusal has to compare against something.
///
/// `/state` was the value for the reason it is still the root of the store: it
/// is the only writable filesystem the system requires (`ADR-030` §1), and
/// deliberately not `/tmp`, which `ADR-008` gives a Rivet none of.
pub const DEFAULT_REMOTE_DIR: &str = "/state";

/// The configuration document, which is where `enabled` lives.
///
/// `ADR-007` §3.4's machine layer. Spelled here because this tool reaches it by
/// path and has no `rivet-config` on the other end to ask.
pub const DOCUMENT_PATH: &str = "/state/config/machine.toml";

/// A change to the configuration document, as `rivet apply edit` takes one.
///
/// **Two shapes and not one**, because a document that already names the
/// service cannot be appended to: a second `[services.<name>]` table is a TOML
/// error, and `apply edit --append` would write one happily.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigChange {
    /// Add a `[services.<name>]` table the document does not have.
    Append { text: String },
    /// Turn an existing `enabled = false` into `true`.
    ///
    /// `expect` must occur **exactly once** or `rivet apply edit` refuses —
    /// which is the property being relied on here, not worked around.
    Replace { expect: String, replace: String },
}

/// What the document already says about a service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    /// It says nothing; a table has to be added.
    Absent,
    /// It says `enabled = false`; one word has to change.
    Disabled,
    /// It already says `enabled = true`.
    Enabled,
    /// It names the service in a shape this tool will not edit blind.
    Unreadable,
}

/// What the document says about `name`, and how to change it.
///
/// **The document is canonical when Rivet wrote it** — `rivet export` and
/// every `apply` render it through `rivet-config`'s serialiser, so
/// `[services.x]` is followed immediately by `enabled = <bool>`. A document a
/// person hand-edited need not be, and [`Standing::Unreadable`] is that case:
/// this tool refuses and hands the operator the command rather than guessing at
/// a file it did not write.
///
/// **A header counts only when it is a whole line**, trimmed, equal to
/// `[services.<name>]` — never a raw substring, which a comment
/// (`# [services.demo]`) or another service's own line can contain just as
/// easily as a real table can. A line that only LOOKS like the header because
/// it sits inside a TOML multi-line string is not distinguished from a real
/// one; that is a known gap in this heuristic, not a case this function
/// claims to read correctly.
#[must_use]
pub fn standing(document: &str, name: &str) -> Standing {
    let header = format!("[services.{name}]");
    let mut lines = document.lines();
    // `by_ref` so that once the header line is found, `lines` is left
    // positioned just after it — not reset to the start of the document.
    if !lines.by_ref().any(|line| line.trim() == header) {
        return Standing::Absent;
    }
    // The first `enabled` line after the header, stopping at the next table so
    // that a service with no `enabled` of its own cannot borrow the next one's.
    for line in lines {
        let line = line.trim();
        if line.starts_with('[') {
            break;
        }
        let Some(value) = line.strip_prefix("enabled") else {
            continue;
        };
        return match value.trim_start().strip_prefix('=').map(str::trim) {
            Some("true") => Standing::Enabled,
            Some("false") => Standing::Disabled,
            _ => Standing::Unreadable,
        };
    }
    Standing::Unreadable
}

/// The edit that makes the document say `enabled = true` for `name`.
///
/// `None` when there is nothing to do or nothing safe to do — the caller
/// distinguishes those by asking [`standing`] first.
#[must_use]
pub fn change_for(standing: &Standing, name: &str) -> Option<ConfigChange> {
    match standing {
        // A leading newline, because `apply edit --append` adds the text
        // "exactly as given — no newline is added and none is removed", and a
        // document whose last line has no newline would otherwise gain a table
        // header glued to it.
        Standing::Absent => Some(ConfigChange::Append {
            text: format!("\n[services.{name}]\nenabled = true\n"),
        }),
        Standing::Disabled => Some(ConfigChange::Replace {
            expect: format!("[services.{name}]\nenabled = false"),
            replace: format!("[services.{name}]\nenabled = true"),
        }),
        Standing::Enabled | Standing::Unreadable => None,
    }
}

// ---------------------------------------------------------------------------
// `[kiosk] auto-run` — `.rulings/` R-214
// ---------------------------------------------------------------------------

/// What the document currently says about `[kiosk] auto-run`.
///
/// [`kiosk_auto_run`] reads this off the document's own text with the same
/// line-scanning [`standing`] already uses for `[services.<name>]` — a
/// second table this tool edits, read the same careful way rather than a
/// second parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KioskAutoRun {
    /// No `[kiosk]` table at all. There is no existing `auto-run` line to
    /// `--expect` against, and `apply edit --append` risks silently
    /// commenting out whatever follows it — so this is never auto-edited;
    /// [`kiosk_change_for`] answers `None` and the caller reports the manual
    /// fix instead.
    NoTable,
    /// The table is present and names an application.
    Named(String),
    /// The table is present, but no `auto-run` line was found in a shape
    /// this tool reads with confidence — [`Standing::Unreadable`]'s own
    /// posture, restated: refuse rather than guess at an edit.
    Unreadable,
}

/// Read `[kiosk] auto-run` off `document`'s own text.
///
/// **A header counts only when it is a whole line**, trimmed, equal to
/// `[kiosk]` — [`standing`]'s own rule against a comment or a multi-line
/// string being mistaken for a real table.
#[must_use]
pub fn kiosk_auto_run(document: &str) -> KioskAutoRun {
    let mut lines = document.lines();
    if !lines.by_ref().any(|line| line.trim() == "[kiosk]") {
        return KioskAutoRun::NoTable;
    }
    for line in lines {
        let line = line.trim();
        if line.starts_with('[') {
            break;
        }
        let Some(value) = line.strip_prefix("auto-run") else {
            continue;
        };
        return match value.trim_start().strip_prefix('=').map(str::trim).and_then(unquoted) {
            Some(name) => KioskAutoRun::Named(name),
            None => KioskAutoRun::Unreadable,
        };
    }
    KioskAutoRun::Unreadable
}

/// The text between one pair of double quotes, or `None` for anything else —
/// [`kiosk_auto_run`]'s reason to read [`KioskAutoRun::Unreadable`] rather
/// than an unquoted or malformed value's meaning guessed at.
fn unquoted(text: &str) -> Option<String> {
    let inner = text.strip_prefix('"')?.strip_suffix('"')?;
    Some(inner.to_string())
}

/// The edit that makes `[kiosk] auto-run` name `target`.
///
/// `None` when `target` is already named, when the document has no `[kiosk]`
/// table to edit ([`KioskAutoRun::NoTable`]'s own doc says why this is never
/// `--append`ed instead), or when the current line is not one this tool
/// reads with confidence ([`KioskAutoRun::Unreadable`]) — the caller
/// distinguishes those by asking [`kiosk_auto_run`] first, exactly as
/// [`change_for`]'s own caller asks [`standing`].
#[must_use]
pub fn kiosk_change_for(current: &KioskAutoRun, target: &str) -> Option<ConfigChange> {
    match current {
        KioskAutoRun::Named(name) if name == target => None,
        KioskAutoRun::Named(name) => Some(ConfigChange::Replace {
            expect: format!("auto-run = {name:?}"),
            replace: format!("auto-run = {target:?}"),
        }),
        KioskAutoRun::NoTable | KioskAutoRun::Unreadable => None,
    }
}

/// What the caller asked for.
#[derive(Debug)]
pub struct DeployArgs {
    /// Where the machine is, as `host:port`. Never parsed here — it is
    /// reported, and `crate::endpoint` is what resolved it.
    pub host: String,
    /// A payload to send instead of building one.
    pub rvt: Option<PathBuf>,
    /// The options `package` takes, passed through untouched. Used only when
    /// [`DeployArgs::rvt`] is `None`.
    pub package_args: Vec<String>,
    pub remote_dir: String,
    /// Whether the operator typed the last word.
    pub confirmed: bool,
    /// This operator's own TLS identity, when the control plane is the
    /// transport — `ADR-035` §7.2.
    ///
    /// **Present means TLS.** There is no `--tls` boolean beside it, because a
    /// flag that said *use TLS* and a flag that said *with this identity* could
    /// disagree, and two flags for one decision is one more than the decision
    /// has. `crate::tls` cannot work without an identity anyway: `ADR-025` §2's
    /// trust is mutual, so a client with no key of its own has nothing to
    /// present.
    pub identity: Option<PathBuf>,
    /// The machine's public key, as the fleet register records it.
    ///
    /// **Required with `identity` and meaningless without it**, which
    /// `crate::deploy::run`'s caller checks: pinning nothing would be trusting
    /// whatever answered, and `ADR-025` §2 makes the pin the whole of the trust
    /// model.
    pub pin: Option<String>,
    /// Whether to make the document say this service should run.
    ///
    /// Since 2026-08-25 installing an application does not start it
    /// (`ADR-015` §3.2.2), and this is where the second statement is made —
    /// in the host tool, from a word the operator typed, written **into** the
    /// document rather than around it.
    pub enable: bool,
    /// Whether to wait for THIS install to finish before returning — see
    /// this module's header, and `wait_for_own_install` beside [`run`].
    ///
    /// **`true` is what `cargo rivet install` does unless told otherwise**, and
    /// the reason is the command an agent types next. `cargo rivet secrets`
    /// writes into the application's data directory, which the INSTALLER makes;
    /// a caller that returns as soon as the request is accepted hands its next
    /// command a race against a directory that may not exist yet, and `apply
    /// install-secret` refuses a missing one. `false` returns once the machine
    /// has the request, as every install did before.
    pub wait: bool,
}

/// The board, as the three things this verb does to one.
///
/// A trait so that the tests below can exercise every path — including the
/// digest mismatch, which is the branch that matters most and the one no test
/// could reach by talking to a real machine.
pub trait Remote {
    /// Send `local`'s bytes to `remote_path`, replacing whatever is there.
    ///
    /// **It answers what the machine said**, and that is the change of
    /// 2026-09-12. The bulk endpoint's `201` body carries a `pruned <names>`
    /// line, which is how retention announces an eviction, and
    /// this method used to read the answer far enough to check the status and
    /// then drop it. So a machine deleted artifacts and told nobody, and an
    /// operator learned one was gone by asking for it (`ADR-052` §7).
    /// [`run`] relays the lines that say something this end does not already
    /// know.
    fn push(&mut self, local: &Path, remote_path: &str) -> Result<String, String>;
    /// The board's own BLAKE3 of `remote_path`.
    fn blake3(&mut self, remote_path: &str) -> Result<[u8; 32], String>;
    /// `rivet apply install <digest> confirm`, and whatever it said.
    ///
    /// **A digest and not a path, since 2026-09-12** — `ADR-052` §7. The
    /// transfer above put the artifact into the machine's own store, keyed by
    /// exactly this digest; there is no form of the remote operation that names
    /// a path any more, so there is none here either.
    fn install(&mut self, digest: &str) -> Result<String, String>;
    /// The configuration document's text, for `--enable` to read before it
    /// decides what to change. `Ok(None)` when there is no document — a machine
    /// that has never been configured, which this tool will not write one for.
    fn read_document(&mut self) -> Result<Option<String>, String>;
    /// `rivet apply edit <document> … confirm`, and whatever it said.
    fn edit_document(&mut self, change: &ConfigChange) -> Result<String, String>;

    /// Place `text` at the configuration document's path, replacing whatever
    /// is there.
    ///
    /// **The document and no other path**, deliberately. A general
    /// "write these bytes anywhere" would make this trait a remote filesystem,
    /// and the one thing a fleet tool has business replacing wholesale is the
    /// document it composed (`ADR-034` §11.4). Everything else on a Rivet is
    /// either the image's or a generated view.
    ///
    /// Added 2026-08-26 for `rivet-fleet apply document`.
    fn put_document(&mut self, text: &str) -> Result<(), String>;

    /// Run `rivet <words…>` on the machine and give back its stdout.
    ///
    /// The machine's own CLI, spoken in the machine's own vocabulary — which is
    /// `ADR-034` §11.1's rule that the tools share one vocabulary however
    /// different the machinery beneath them (`.rulings/` R-055), arriving at the
    /// wire rather than at the command line. `rivet get status`,
    /// `rivet apply reload confirm`.
    ///
    /// **The words are quoted by the implementation**, because the remote
    /// command string is `nu` source (see this module's header) and a caller
    /// that had to know that would be a caller that gets it wrong.
    ///
    /// **[`Remote::blake3`], [`Remote::install`] and [`Remote::edit_document`]
    /// could all be expressed through this and are not.** That is deliberate
    /// and it is a debt rather than a design: those three are the
    /// `cargo rivet deploy` sequence, proved on hardware across two update
    /// cycles, and rewriting a proven path to remove three methods would be
    /// trading something that works for something tidier. The tidy is worth
    /// doing on the day one of them needs to change anyway.
    fn ask(&mut self, words: &[&str]) -> Result<String, String>;

    /// Perform one catalogue operation named as `words` — **the transport
    /// that does not refuse a word list**, unlike [`Remote::ask`] above.
    ///
    /// [`crate::command::perform`] is the translation from a word list to a
    /// `tools/call` — bind the words to [`rivet_core::operation::CATALOGUE`]
    /// and send the result — and it needs `&dyn crate::tls::CallsTools`, not
    /// `&mut dyn Remote`. [`crate::tls::Tls`] is both: it implements
    /// `CallsTools` for real (a socket underneath), so its own override of
    /// this method is `crate::command::perform(self, words)`. **Added so
    /// `cargo rivet fleet`'s restart steps have somewhere to call that is not
    /// `ask`** — `ask`'s own refusal, once a board is MCP-backed, names this
    /// method as the fix, and a caller that has a service or the proxy to
    /// restart calls it rather than guessing at a second translation.
    ///
    /// **Defaulted to `self.ask(words)`.** Every implementor that is not
    /// `Tls` — every test fake in this crate and its callers — has no
    /// `CallsTools` transport underneath it to route through, and inherits
    /// this default rather than being made to say so individually. A fake
    /// that means to exercise the MCP-catalogue path on purpose (this
    /// crate's own `FakeTls`, which wraps a `CallsTools` fake) overrides it
    /// the same way `Tls` does.
    fn perform_operation(&mut self, words: &[&str]) -> Result<String, String> {
        self.ask(words)
    }

    /// Every name `get file /state/lib` lists — the same read `cargo rivet
    /// fleet drift` already performs (`SPEC-02` §5, `ADR-052` §9).
    ///
    /// **Added for `.rulings/` R-157 R15**, so `crate::fleet::publish_then_send`
    /// (in `cmd/cargo-rivet`) can refuse to send an application whose declared
    /// closure this machine cannot satisfy, before a single byte crosses the
    /// wire — rather than let the board's own installer refusal (`SPEC-02`
    /// §5.1) arrive only after the whole transfer.
    ///
    /// **Defaulted, not required of every implementor.** Most callers of this
    /// trait never deploy an application that declares a closure — `SPEC-02`
    /// §5's "empty is the ordinary answer" means [`run`] never calls this
    /// method at all, and `cmd/rivet-fleet`'s own uses of [`Remote`] are for
    /// `apply document`, never for [`run`]. Only [`crate::tls::Tls`] overrides
    /// it with a real read; every other implementor inherits a refusal naming
    /// itself, which is safe precisely because nothing calls it unless an
    /// application actually declares something to check.
    ///
    /// # Errors
    ///
    /// The default: this implementation cannot answer. A real one: the machine
    /// did not answer, or `/state/lib` exists and is not a directory.
    fn installed_libraries(&mut self) -> Result<Vec<String>, String> {
        Err(
            "this connection cannot read a machine's installed libraries (SPEC-02 §5)"
                .to_string(),
        )
    }

    /// Every name in an application's own data directory, or `None` when that
    /// directory is absent.
    ///
    /// **`None` is not an empty list and the difference decides a refusal.**
    /// The directory is made by the INSTALLER (`ADR-015` §3), so its absence
    /// means this application is not installed here — and nothing can be put
    /// into it until one is, which is why `apply install-secret` refuses a
    /// missing directory rather than creating one. An empty directory, by
    /// contrast, is an installed application that has been given nothing.
    ///
    /// **Added for `ADR-055` §3 and `.rulings/` R-194**, so `cargo rivet
    /// deploy` can say which declared material is absent before it sends a
    /// payload — the read `installed_libraries` already makes, one directory
    /// along.
    ///
    /// **Defaulted, not required of every implementor**, for
    /// [`Remote::installed_libraries`]' reason: nothing calls this unless an
    /// application actually declares material.
    ///
    /// # Errors
    ///
    /// The default: this implementation cannot answer. A real one: the machine
    /// did not answer, or the path exists and is not a directory.
    fn data_directory_files(&mut self, application: &str) -> Result<Option<Vec<String>>, String> {
        Err(format!(
            "this connection cannot read {application}'s data directory (ADR-055 §3)"
        ))
    }

    /// **Whether the machine holds its fleet's root**, and so issues every
    /// `fleet-tls` application its certificate itself (`ADR-063` §4) — the
    /// `present` of `rivet get fleet-root`.
    ///
    /// **Defaulted, not required of every implementor**, for
    /// [`Remote::data_directory_files`]' reason: nothing calls this unless an
    /// application declares material.
    ///
    /// # Errors
    ///
    /// The default: this implementation cannot answer. A real one: the machine
    /// did not answer.
    fn holds_fleet_root(&mut self) -> Result<bool, String> {
        Err("this connection cannot ask whether the machine holds its fleet's root".to_string())
    }
}

/// What produces the payload: `crate::package_to`, or a test's stand-in.
///
/// Named so that `run` has a seam rather than a `cargo` invocation inside it.
/// It returns the artifact **and** the report `package` would have printed,
/// because the report is what says which key signed it and what the members
/// are, and a deploy that swallowed it would be less informative than the two
/// commands it replaces.
pub type BuildPayload<'a> = &'a mut dyn FnMut(&[String]) -> Result<(PathBuf, String), String>;


/// The service `--enable` would name, and why it may refuse before anything is
/// sent.
///
/// **From the file name, never from the payload** — see
/// `crate::name::product_and_kind`. Two consequences are deliberate: a payload
/// this tool did not name is refused rather than guessed at, and a **system
/// update is refused by kind**, because `[services.rivet]` is not a thing and
/// an operator who typed `--enable` for one has a wrong model rather than a
/// typo.
fn service_to_enable(args: &DeployArgs, file_name: &str) -> Result<String, String> {
    let (product, kind) = crate::name::product_and_kind(file_name).ok_or_else(|| {
        format!(
            "--enable needs to know which service to name in the document, and {file_name:?} is \
             not a name `cargo rivet package` composed (ADR-027 §7), so it cannot be read back. \
             Deploy without --enable and then say it yourself:\n\
             \x20 rivet apply edit {DOCUMENT_PATH} --append \"...\" confirm"
        )
    })?;
    if kind != rivet_core::package::kind::Kind::application().to_string() {
        return Err(format!(
            "--enable is for an application and this payload is a {kind}. A system update has no \
             `[services.<name>]` table to write — it becomes the machine rather than something \
             the machine runs — and it takes effect at \
             `rivet apply trial-restart confirm`. Nothing has been sent."
        ));
    }
    // Belt and braces: the document key is a service name, and the board will
    // refuse one that is not. Better to refuse here, before an edit.
    if rivet_core::service::ServiceName::new(&product).is_none() {
        return Err(format!(
            "{product:?} came out of the payload's file name and is not a usable service name, \
             so nothing will be written to the document. Nothing has been sent."
        ));
    }
    let _ = args;
    Ok(product)
}

/// Make the document say the service should run, before the payload arrives.
///
/// # Why before, and not after
///
/// `cmd/rivet-sysd` re-reads the document at the moment an install finishes
/// (`PLAN.md` §20) and at no other moment while it is running. So the line has
/// to be in the file *before* the installer completes, or the supervisor
/// discovers the application, consults a document that does not name it, and
/// leaves it idle until the next restart — which is the friction `--enable`
/// exists to remove.
///
/// It is also the safer order on its own terms: a document edit that is refused
/// leaves a machine with nothing installed, rather than an installed
/// application nothing will start.
fn enable_service(
    remote: &mut dyn Remote,
    name: &str,
    report: &mut String,
) -> Result<(), String> {
    let Some(document) = remote.read_document()? else {
        return Err(format!(
            "there is no {DOCUMENT_PATH} on this machine, so there is nothing for --enable to \
             add a service to. A document without `version` is not a valid one (ADR-007 §7.1) \
             and writing the first one is provisioning's job (ADR-004 §6), not a deploy tool's. \
             Nothing has been sent."
        ));
    };

    match standing(&document, name) {
        Standing::Enabled => {
            report.push_str(&format!(
                "enabled  already — the document says `[services.{name}] enabled = true`\n"
            ));
            Ok(())
        }
        Standing::Unreadable => Err(format!(
            "the document names `[services.{name}]` in a shape this tool will not edit blind — \
             it is not the canonical `enabled = <bool>` on the line after the header, so a \
             hand-written layout is in play and a blind edit could mean something other than \
             what it looks like. Say it yourself and deploy without --enable:\n\
             \x20 rivet apply edit {DOCUMENT_PATH} --expect \"...\" --replace \"...\" confirm\n\
             Nothing has been sent."
        )),
        state => {
            let change = change_for(&state, name).ok_or("nothing to change")?;
            let said = remote.edit_document(&change)?;
            report.push_str(&format!(
                "enabled  the document now says `[services.{name}] enabled = true` ({})\n",
                match state {
                    Standing::Absent => "the table was added",
                    _ => "one word changed",
                }
            ));
            if !said.trim().is_empty() {
                report.push_str(&indent(said.trim()));
            }
            Ok(())
        }
    }
}

/// What happened to the install a [`run_detailed`] asked for.
///
/// **A caller that plans what comes next needs this as a value**, not as a
/// sentence to search: an install the machine refused as already running
/// changed nothing, so a restart that would pick up new bytes is not owed, and
/// a document edit `--enable` wrote before the send was not re-read by the
/// supervisor either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivered {
    /// Nothing was confirmed, so nothing was built, sent or installed.
    Planned,
    /// The machine already runs this artifact and no install was requested.
    AlreadyRunning,
    /// The request was accepted and this call did not wait for it
    /// ([`DeployArgs::wait`] was false).
    Requested,
    /// The request was accepted, THIS install's chain was seen to finish, and
    /// it ended well. `outcome` is the installer's own token, verbatim.
    Installed { outcome: String },
}

/// [`run_detailed`]'s answer: the report a person reads, and what happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deployed {
    pub report: String,
    pub delivered: Delivered,
}

/// Build or find the payload, push it, check it, install it.
///
/// `build_payload` is passed in rather than called directly so that this
/// function is testable with no `cargo` and no board.
///
/// # Errors
///
/// [`run_detailed`]'s.
pub fn run(
    args: &DeployArgs,
    build_payload: BuildPayload<'_>,
    remote: &mut dyn Remote,
) -> Result<String, String> {
    run_detailed(args, build_payload, remote).map(|deployed| deployed.report)
}

/// [`run`], answering what happened as a value as well as a report.
///
/// # Errors
///
/// A payload that cannot be built or read, a refused document edit, a
/// transfer that did not arrive intact, a refused install request — and, when
/// [`DeployArgs::wait`] is set, an install that FINISHED BADLY, never finished
/// inside [`INSTALL_PATIENCE`], or whose progress could not be read. Every one
/// of those is a failure the caller exits non-zero on.
pub fn run_detailed(
    args: &DeployArgs,
    build_payload: BuildPayload<'_>,
    remote: &mut dyn Remote,
) -> Result<Deployed, String> {
    let mut report = String::new();

    // **The plan changes nothing on either machine**, which includes this one:
    // an unconfirmed `deploy` does not run `cargo build`. Building is a side
    // effect on the caller's disk and `ADR-012` §7.1's rule is not "no remote
    // side effects", it is "nothing happens without the word".
    let local = match (&args.rvt, args.confirmed) {
        (Some(path), _) => path
            .canonicalize()
            .map_err(|e| format!("cannot find {}: {e}", path.display()))?,
        (None, true) => {
            let (path, build_report) = build_payload(&args.package_args)?;
            report.push_str(&build_report);
            path
        }
        // The name cannot be known without building — it carries the crate's
        // version and a git describe string (`ADR-027` §7) — so the plan says
        // so rather than inventing a filename.
        (None, false) => PathBuf::from("<the .rvt `cargo rivet package` would write>"),
    };

    let file_name = local
        .file_name()
        .map_or_else(|| "payload.rvt".to_string(), |n| n.to_string_lossy().into());
    let remote_path = format!("{}/{}", args.remote_dir.trim_end_matches('/'), file_name);

    // **`--remote-dir` no longer names anywhere, and it is refused rather than
    // ignored.** Since `ADR-052` §7 the machine decides where an artifact
    // lands: the bulk endpoint stores it under its own digest, and an install
    // names the digest. A flag that was accepted and quietly did nothing would
    // be worse than one that is gone — an operator would believe it.
    if args.remote_dir.trim_end_matches('/') != DEFAULT_REMOTE_DIR {
        return Err(format!(
            "--remote-dir {dir} names a directory, and a machine no longer takes one. An \
             artifact is transferred into that machine's own ARTIFACT STORE, keyed by its \
             BLAKE3 digest, and `rivet apply install <digest> confirm` resolves it there — \
             there is no form of an install that names a path (ADR-052 §7). Drop the flag; \
             there is nothing it could mean. Nothing has been sent.",
            dir = args.remote_dir,
        ));
    }

    if !args.confirmed {
        report.push_str(&format!(
            "PLAN — nothing has been built, sent or installed.\n\
             \n\
             {:<13} {}\n\
             would send to {}, into that machine's artifact store\n\
             would check   the board's BLAKE3 of it against this machine's\n\
             would run     rivet apply install <that artifact's digest> confirm\n\
             {waiting}\
             {enabling}\
             \n\
             It would NOT restart the board.\n\
             \n\
             Add `confirm` as the last word to do it.\n",
            if args.rvt.is_some() {
                "payload"
            } else {
                "would build"
            },
            shown(&local),
            args.host,
            waiting = if args.wait {
                format!(
                    "would wait   for THAT install to finish — `rivet get install`'s correlation \
                     read before the request, then its outcome — for up to {:.0}s\n",
                    INSTALL_PATIENCE.as_secs_f32(),
                )
            } else {
                "would NOT    wait for the install to finish (--no-wait)\n".to_string()
            },
            enabling = if args.enable {
                format!(
                    "would enable  a `[services.<name>]` table in {DOCUMENT_PATH}, BEFORE \
                     sending, so the supervisor sees it when the install finishes\n"
                )
            } else {
                format!(
                    "would NOT    enable it. Installing does not start an application \
                     (ADR-015 §3.2.2) — pass --enable, or say it in {DOCUMENT_PATH} yourself\n"
                )
            },
        ));
        return Ok(Deployed { report, delivered: Delivered::Planned });
    }

    // **Before a single byte is sent.** See `enable_service`: the supervisor
    // re-reads the document when an install finishes and at no other moment, so
    // the line has to be there first — and a refused edit then leaves a machine
    // with nothing installed rather than something that will not start.
    if args.enable {
        let name = service_to_enable(args, &file_name)?;
        enable_service(remote, &name, &mut report)?;
    }

    let (local_digest, local_len) = blake3_of(&local)?;
    report.push_str(&format!(
        "payload  {} ({local_len} bytes)\n\
         blake3   {}\n",
        shown(&local),
        hex(&local_digest),
    ));

    let said = remote.push(&local, &remote_path)?;
    report.push_str(&format!(
        "sent     {}, into its artifact store\n",
        args.host
    ));
    // **What the machine said that this end did not already know.** See
    // [`relayed`]: retention's evictions are announced here and nowhere else.
    for line in relayed(&said) {
        report.push_str(&format!("{line}\n"));
    }

    // **The check, and it is why this is a program.** See the module header: a
    // bad channel would otherwise surface as a signature refusal, which points
    // an operator at their key.
    let seen = remote.blake3(&remote_path)?;
    if seen != local_digest {
        return Err(format!(
            "the payload on the board is NOT the payload on this machine.\n\
             \x20 here  {}\n\
             \x20 there {}\n\
             Nothing was installed. This is a fault in the transfer, not in the artifact and \
             not in your signing key — the same bytes were hashed by the same algorithm at \
             both ends. Send it again.",
            hex(&local_digest),
            hex(&seen),
        ));
    }
    report.push_str("checked  the board's copy is byte-identical\n");

    // **The digest is the whole of what is named**, and it is the one this end
    // computed from the file it sent — not one read back from the machine.
    // `ADR-052` §7: an install resolves a digest in the machine's own store, so
    // asking for the digest we transferred is asking for the bytes we
    // transferred, and a machine that has something else under that name is a
    // machine whose store is broken rather than one that installs the wrong
    // thing.
    //
    // **A machine already running this artifact is where the deploy stops, and
    // it is not a failure** — `PLAN-Feature-Set-2026-09-12` §3. Found on
    // `rivet-01`: the running system update was re-sent, the install was
    // requested, and the installer replaced the rollback with a second copy of
    // it. The machine now refuses that request before writing it, in words
    // carrying `rivet_core::deploy::ALREADY_RUNNING`, and this reports the stop
    // plainly and exits zero: the machine is in the state the deploy was for,
    // and a script that deploys what is already there should not fail.
    let (said, before) = match install_when_the_slot_is_free(
        remote,
        &args.host,
        &hex(&local_digest),
        &mut report,
        SLOT_PATIENCE,
        &mut std::thread::sleep,
        args.wait,
    ) {
        Ok(accepted) => accepted,
        Err(refusal) if refusal.contains(rivet_core::deploy::ALREADY_RUNNING) => {
            report.push_str(&format!(
                "running  {host} is ALREADY RUNNING this artifact, so NO INSTALL WAS REQUESTED — \
                 the machine refused the request before writing it, and there is nothing to do. \
                 The artifact is in that machine's store.\n",
                host = args.host,
            ));
            report.push_str(&indent(refusal.trim()));
            return Ok(Deployed { report, delivered: Delivered::AlreadyRunning });
        }
        Err(refusal) => return Err(refusal),
    };
    report.push_str(&format!("install  requested by digest {}\n", hex(&local_digest)));
    if !said.trim().is_empty() {
        report.push_str(&indent(said.trim()));
    }

    let delivered = match before {
        Some(before) => {
            // **The failure is returned on its own, first line first**, the
            // way every refusal above is: a caller that reports one line per
            // step (`cargo rivet deploy`'s interview) reads the first, and a
            // failed install must lead with the fact that it failed.
            let outcome = wait_for_own_install(
                remote,
                &args.host,
                &before,
                &mut report,
                INSTALL_PATIENCE,
                &mut std::thread::sleep,
            )?;
            Delivered::Installed { outcome }
        }
        None => {
            report.push_str(
                "\nThe board has the request and THIS RUN DID NOT WAIT FOR IT (--no-wait); the \
                 installer is a separate supervised program and its account is in the journal — \
                 `rivet get install` is the short answer and `rivet get log` the long one. Read \
                 `get install`'s `correlation` and wait for it to change, then for `finished`, \
                 before anything that needs the install to have happened.\n",
            );
            Delivered::Requested
        }
    };
    report.push_str(
        "\nNOTHING WAS RESTARTED. For a system update the payload is in the inactive slot \
         and `rivet apply trial-restart confirm` is the separate decision about when this \
         machine goes down.\n",
    );

    Ok(Deployed { report, delivered })
}
/// The lines of a machine's transfer answer that an operator must see.
///
/// **Most of that `201` body repeats what this tool has already printed** — the
/// object's path, the length, the digest, what to type next — and relaying it
/// whole would bury the two lines that carry something only the machine knows:
///
/// * `pruned <names>` — **which artifacts retention evicted.** The bulk endpoint
///   announces an eviction there, and nothing else announces one. It was composed
///   and then discarded (`ADR-052` §7), so a machine deleted artifacts and
///   told nobody; an operator found out by asking for one that was gone.
/// * `note …` — the transport saying the exchange took an unusual path. Today
///   that is one thing: the machine answered before this end had finished
///   sending. See `crate::tls::transfer`.
/// * `verified …` — **that the machine checked the signature before it stored
///   the artifact**, and by which of its keys (`ADR-052` §7). This end
///   cannot know it: the signature is checked against the machine's trust store,
///   not this workstation's, so the line is the machine's to say and the report
///   is the place an operator learns it.
///
/// **Selected by their first word rather than by a parse**, because the body is
/// a report for a person and this is a relay rather than a protocol. A line a
/// newer machine adds is ignored here until this list names it, which is the
/// failure that loses information rather than the one that invents it.
fn relayed(said: &str) -> impl Iterator<Item = &str> {
    said.lines().filter(|line| {
        matches!(
            line.split_whitespace().next(),
            Some("pruned") | Some("note") | Some("verified")
        )
    })
}

/// How long [`run`] waits for a machine's one install-request slot to clear.
///
/// **Two minutes, and the number is a ceiling rather than an expectation.** The
/// supervisor takes a request up on its next convergence pass, which is moments;
/// what this has to survive is the pass after an install that is still running,
/// because `cmd/rivet-deploy` holds the slot until it is finished and a system
/// update writes 683 MB to a card. A limit shorter than that would turn a
/// correct deploy into a failure, and one much longer would leave an operator
/// watching a tool that has nothing to wait for.
const SLOT_PATIENCE: Duration = Duration::from_secs(120);

/// The first pause, which doubles up to [`SLOT_LONGEST_PAUSE`].
///
/// **It starts short because the usual clear is immediate** — one convergence
/// pass — and grows because the unusual one is a 683 MB write, where asking
/// every quarter-second is a machine being pestered by a tool that could wait.
const SLOT_FIRST_PAUSE: Duration = Duration::from_millis(250);
const SLOT_LONGEST_PAUSE: Duration = Duration::from_secs(5);

/// Ask for the install, waiting out the one refusal that is temporary.
///
/// # The defect this exists for
///
/// `rivet_core::deploy`'s request file is **one slot**, and `rivet apply
/// install` refuses a second request while one is still waiting. That refusal is
/// correct — two installers would be two writers on one inactive slot — and
/// until 2026-09-12 this tool did not wait it out: it transferred, asked once,
/// and returned. **Four deploys back to back failed on the second**, found while
/// proving `ADR-052` §7 in QEMU, and anybody scripting deployments meets it
/// immediately.
///
/// `tools/qemu/adr-052-proof.sh` works around it by waiting on the installer's
/// own record between deploys. That is the right shape for a script that has a
/// console to read; from here there is no console, so what is waited on is the
/// machine's own answer to the question actually being asked.
///
/// # Why it retries the request rather than polling `get install`
///
/// **Because the state that blocks this is not one `get install` reports.** That
/// operation reads the installer's *journal*; the window that loses a deploy is
/// the one between a request being written and the supervisor taking it up,
/// where the journal's latest install is still the PREVIOUS one and reads as
/// `finished: true`. A poll there would answer *go ahead* and the request would
/// be refused anyway. Asking for the install is the question whose answer
/// is about the slot.
///
/// So the interlock's own refusal is the signal, recognised by
/// [`rivet_core::deploy::REQUEST_WAITING`] — one constant, stated by the machine
/// that composes the sentence. **Every other refusal means stop**, and is
/// returned on the first attempt without a pause: a machine with no `/state`, an
/// unreadable request file or a digest it does not hold will still be in that
/// state in two minutes.
///
/// `rest` is the pause, a parameter so the tests below do not sleep.
///
/// # `observe`, and why the baseline is read before EVERY attempt
///
/// With `observe`, `get install`'s `correlation` is read immediately before
/// each request and the one read before the ACCEPTED request is returned beside
/// what the machine said — the baseline [`wait_for_own_install`] waits to see
/// change. Read once before the first attempt instead, a slot held by somebody
/// else's request would hand the wait a baseline two installs old: the other
/// install starts, its correlation differs, and the wait reports THEIR outcome
/// as this one's. `get install` reports the LATEST install and nothing else
/// (`AGENTS.md` §10.6), so the only baseline that means anything is the one
/// nearest the request.
///
/// A baseline that cannot be read refuses before the request, with nothing
/// installed: an install whose finish cannot be recognised is an install this
/// caller asked to wait for and could not.
fn install_when_the_slot_is_free(
    remote: &mut dyn Remote,
    host: &str,
    digest: &str,
    report: &mut String,
    patience: Duration,
    rest: &mut dyn FnMut(Duration),
    observe: bool,
) -> Result<(String, Option<Option<String>>), String> {
    let mut waited = Duration::ZERO;
    let mut pause = SLOT_FIRST_PAUSE;
    let mut refusals = 0u32;
    loop {
        let before = if observe {
            let latest = latest_install(remote).map_err(|why| {
                format!(
                    "{host} could not say which install it last ran, so NOTHING WAS INSTALLED — \
                     the artifact is in that machine's store, and no install was requested.\n\
                     \x20 why   this install waits for ITS OWN install to finish, and recognises \
                     it by `rivet get install`'s `correlation` changing from the one read just \
                     before the request. That read did not answer: {why}\n\
                     \x20 fix   `--no-wait` requests the install without waiting for it; then \
                     read `get install` yourself until its correlation changes and it reports \
                     `finished`."
                )
            })?;
            Some(latest.correlation)
        } else {
            None
        };
        let refusal = match remote.install(digest) {
            Ok(said) => {
                if refusals > 0 {
                    report.push_str(&format!(
                        "waited   {:.1}s for the machine's install request slot to clear; it \
                         held one install request that this deploy's could not replace\n",
                        waited.as_secs_f32(),
                    ));
                }
                return Ok((said, before));
            }
            Err(refusal) => refusal,
        };

        if !refusal.contains(rivet_core::deploy::REQUEST_WAITING) {
            return Err(refusal);
        }
        if waited >= patience {
            return Err(format!(
                "{refusal}\n\
                 \x20 waited  {:.0}s, asking {refusals} time(s), and the machine's install \
                 request slot never cleared. THE ARTIFACT WAS TRANSFERRED and is in that \
                 machine's store; nothing has been installed from it.\n\
                 \x20 why it matters  {path} holds ONE request. A request that does not clear \
                 means the supervisor is not running, or is not converging, or an installer it \
                 started has not finished — `rivet get install` reports the last install and \
                 `rivet get log` this boot's account.\n\
                 \x20 what to do  nothing needs re-sending. When the machine is settled, \
                 `rivet apply install {digest} confirm` asks for exactly what this deploy \
                 would have asked for.",
                patience.as_secs_f32(),
                path = rivet_core::deploy::REQUEST_PATH,
            ));
        }

        // Said once, on the first refusal, so an operator watching a deploy that
        // has gone quiet knows what it is quiet about.
        if refusals == 0 {
            report.push_str(&format!(
                "waiting  {host} already has an install request waiting and takes one at a \
                 time; this deploy's cannot replace it. Asking again until it clears, for up \
                 to {:.0}s. THE ARTIFACT IS ALREADY TRANSFERRED.\n",
                patience.as_secs_f32(),
            ));
        }
        refusals += 1;
        rest(pause);
        waited += pause;
        pause = (pause * 2).min(SLOT_LONGEST_PAUSE);
    }
}

/// How long [`run_detailed`] waits for its own install to finish.
///
/// **Ten minutes, as a ceiling and not an expectation.** An application
/// installs in seconds. A system update writes 683 MB into the inactive slot
/// and verifies it, and on a slow card that is minutes; a limit shorter than the
/// slowest correct install turns a correct install into a reported failure. A
/// wait that reaches this reports which of two things it saw — the install
/// never started, or it started and never finished — rather than a bare
/// timeout.
pub const INSTALL_PATIENCE: Duration = Duration::from_secs(600);

/// The first pause between two reads of `get install`, which doubles up to
/// [`INSTALL_LONGEST_PAUSE`].
///
/// **Each read is a control-plane call that reads the machine's journal**, so a
/// tight loop is a tool flooding the very record an operator will read to find
/// out what went wrong. Short at first because an application's install is
/// usually over in one or two; capped at five seconds because a system update's
/// is not.
const INSTALL_FIRST_PAUSE: Duration = Duration::from_millis(500);
const INSTALL_LONGEST_PAUSE: Duration = Duration::from_secs(5);

/// What `rivet get install` says about the LATEST install on a machine —
/// `rivet_core::operation::install`'s `GET_OUTPUTS`, the fields a wait needs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LatestInstall {
    /// `ADR-022` §5's identifier for the latest install, or `None` when the
    /// journal holds no install at all.
    correlation: Option<String>,
    /// Whether the journal holds that install's last record.
    finished: bool,
    /// How it ended, as a boolean — present exactly when `finished` is and the
    /// outcome is one this build knows. `None` for a token a newer Rivet wrote.
    succeeded: Option<bool>,
    /// The installer's own token, verbatim.
    outcome: Option<String>,
    /// The message of the chain's last record, for a person.
    last: Option<String>,
}

/// Ask the machine `get install` and read the fields a wait needs.
///
/// **An answer that is not `get install`'s is refused rather than read.** A
/// wait that took an unrelated answer for *no install yet* would wait the whole
/// of [`INSTALL_PATIENCE`] for nothing and then blame the machine.
fn latest_install(remote: &mut dyn Remote) -> Result<LatestInstall, String> {
    use crate::tls::{json_bool_field, json_string_field};

    let answer = remote.perform_operation(&["get", "install"])?;
    if json_string_field(&answer, "subject").ok().as_deref() != Some("install") {
        return Err(format!(
            "the answer to `get install` is not a `get install` answer, so nothing about an \
             install can be read from it. It said:\n{answer}"
        ));
    }
    // **Absent, not empty**: `get install` omits `correlation` exactly when the
    // journal holds no install, which is a real answer — a fresh machine's.
    // Every field below is read by its unescaped key, and the `records` that
    // follow carry theirs escaped, so a record's own `outcome` cannot be read
    // as the answer's (`crate::tls::json_string_field`'s own argument).
    let optional = |key: &str| -> Result<Option<String>, String> {
        if answer.contains(&format!("\"{key}\":")) {
            json_string_field(&answer, key).map(Some)
        } else {
            Ok(None)
        }
    };
    let correlation = optional("correlation")?;
    let finished = if correlation.is_some() { json_bool_field(&answer, "finished")? } else { false };
    let succeeded = if answer.contains("\"succeeded\":") {
        Some(json_bool_field(&answer, "succeeded")?)
    } else {
        None
    };
    Ok(LatestInstall { correlation, finished, succeeded, outcome: optional("outcome")?, last: optional("last")? })
}

/// **Wait for THIS install to finish, and say how it ended.**
///
/// `before` is the latest install's correlation as it stood immediately before
/// the request was accepted ([`install_when_the_slot_is_free`]'s `observe`).
/// This install is the first chain whose correlation is not that one; it is
/// finished when `get install` says `finished`, and it worked when it says
/// `succeeded`. `finished` on its own is the trap `AGENTS.md` §10.6 names: a
/// machine that installed something an hour ago already answers
/// `finished: true`, about that install.
///
/// `rest` is the pause, a parameter so the tests below do not sleep.
///
/// # Errors
///
/// The install finished and did not succeed (its outcome and last message are
/// named); it finished with an outcome this build does not know; it did not
/// finish within `patience`; or `get install` stopped answering. Each says the
/// request WAS delivered, because re-sending is not what any of them needs.
fn wait_for_own_install(
    remote: &mut dyn Remote,
    host: &str,
    before: &Option<String>,
    report: &mut String,
    patience: Duration,
    rest: &mut dyn FnMut(Duration),
) -> Result<String, String> {
    let mut waited = Duration::ZERO;
    let mut pause = INSTALL_FIRST_PAUSE;
    let mut started = false;
    loop {
        let latest = latest_install(remote).map_err(|why| {
            format!(
                "{host}'s install was REQUESTED and whether it finished could not be read: {why}\n\
                 \x20 what to do  `rivet get install` on that machine is the answer this was \
                 waiting for; nothing needs re-sending."
            )
        })?;
        let ours = latest.correlation.is_some() && latest.correlation != *before;
        if ours {
            started = true;
            let correlation = latest.correlation.clone().unwrap_or_default();
            if latest.finished {
                let outcome = latest.outcome.clone().unwrap_or_else(|| "<none>".to_string());
                let last = latest.last.clone().unwrap_or_default();
                return match latest.succeeded {
                    Some(true) => {
                        report.push_str(&format!(
                            "finished {host}'s installer ended `{outcome}` after {:.1}s \
                             (correlation {correlation})\n",
                            waited.as_secs_f32(),
                        ));
                        Ok(outcome)
                    }
                    Some(false) => Err(format!(
                        "THE INSTALL FAILED on {host}: its installer ended `{outcome}` \
                         (correlation {correlation}).\n\
                         \x20 last  {last}\n\
                         \x20 what to do  `rivet get install` on that machine has the whole \
                         chain in `records`. The artifact is in that machine's store; what \
                         failed is the install of it, and sending it again will not change \
                         the answer."
                    )),
                    None => Err(format!(
                        "{host}'s install FINISHED with an outcome this build does not know — \
                         `{outcome}` (correlation {correlation}) — so whether it worked cannot \
                         be said here.\n\
                         \x20 last  {last}\n\
                         \x20 why   a newer Rivet wrote that token. `rivet get install` on that \
                         machine reports its own verdict, and `records` is the installer's \
                         own account."
                    )),
                };
            }
        }
        if waited >= patience {
            return Err(if started {
                format!(
                    "{host}'s install STARTED and did not finish within {:.0}s. It may still be \
                     running, or its installer may have been stopped; an unfinished chain is \
                     not proof of either (`rivet get install`'s own `next` says why).\n\
                     \x20 what to do  read `rivet get install` on that machine until it says \
                     `finished`, and act on `succeeded`. Nothing needs re-sending.",
                    patience.as_secs_f32(),
                )
            } else {
                format!(
                    "{host} ACCEPTED the install request and no install began within {:.0}s — \
                     `rivet get install` still reports the install from before this request.\n\
                     \x20 why it matters  the supervisor takes a request up on its next pass, \
                     which is moments. One that never does is a supervisor that is not \
                     converging, or a request it discarded because another install was \
                     already running; its own record of that is in `rivet get log`.\n\
                     \x20 what to do  read `rivet get status` and `rivet get log latest all` on \
                     that machine. The artifact is in its store; nothing needs re-sending.",
                    patience.as_secs_f32(),
                )
            });
        }
        rest(pause);
        waited += pause;
        pause = (pause * 2).min(INSTALL_LONGEST_PAUSE);
    }
}

/// A path as a person would write it.
///
/// `Path::canonicalize` on Windows returns an extended-length path — the one
/// beginning with two backslashes and a question mark — which is correct, is
/// what the API is for, and is not what an operator typed. A report is read by
/// a person, so the prefix is dropped for display and nothing else: every path
/// this module *uses* is the canonical one, because that is the one that
/// resolves.
fn shown(path: &Path) -> String {
    let text = path.display().to_string();
    // The four characters are backslash, backslash, question mark, backslash.
    text.strip_prefix("\\\\?\\")
        .map_or_else(|| text.clone(), str::to_string)
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn indent(text: &str) -> String {
    text.lines().map(|line| format!("  {line}\n")).collect()
}

/// The whole file's BLAKE3 and its length, without holding it.
///
/// **`pub(crate)` since 2026-08-29**, for `crate::name::Version::distinguished_by`
/// — `ADR-027` §3.2.2 makes a `-dirty` version carry a digest of the payload it
/// names, and a second streaming hasher in the sibling module would be two
/// answers to *what is BLAKE3 of this file* (`ADR-011`).
pub(crate) fn blake3_of(path: &Path) -> Result<([u8; 32], u64), String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut digest = Blake3::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut total = 0u64;
    digest.start();
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        digest.write(&buffer[..read]);
        total += read as u64;
    }
    Ok((digest.finish(), total))
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    // ---- the run, against a board that is not there -------------------------

    struct FakeBoard {
        pushed: Vec<(PathBuf, String)>,
        answer: [u8; 32],
        installed: Vec<String>,
        /// The `201` body a transfer answers with, as the bulk endpoint
        /// composes one. Empty unless a test is about what the machine said.
        answer_to_a_push: String,
        /// How many more times `install` refuses with the interlock's own
        /// refusal before it accepts. `ADR-052` §7's second defect.
        refuse_installs: u32,
        /// What a push is refused with, when a test is about a machine that
        /// refused the artifact at its store's door.
        refuse_push: Option<String>,
        /// Whether `install` refuses as a machine already running the artifact.
        already_running: bool,
    }

    impl FakeBoard {
        fn agreeing(answer: [u8; 32]) -> Self {
            Self {
                pushed: Vec::new(),
                answer,
                installed: Vec::new(),
                answer_to_a_push: String::new(),
                refuse_installs: 0,
                refuse_push: None,
                already_running: false,
            }
        }
    }

    impl Remote for FakeBoard {
        fn push(&mut self, local: &Path, remote_path: &str) -> Result<String, String> {
            self.pushed.push((local.to_path_buf(), remote_path.to_string()));
            if let Some(refusal) = &self.refuse_push {
                return Err(refusal.clone());
            }
            Ok(self.answer_to_a_push.clone())
        }
        fn blake3(&mut self, _remote_path: &str) -> Result<[u8; 32], String> {
            Ok(self.answer)
        }
        fn read_document(&mut self) -> Result<Option<String>, String> {
            unreachable!("this fixture is for deploys that do not touch the document")
        }
        fn edit_document(&mut self, _change: &ConfigChange) -> Result<String, String> {
            unreachable!("this fixture is for deploys that do not touch the document")
        }
        fn install(&mut self, remote_path: &str) -> Result<String, String> {
            if self.already_running {
                // `rivet_core::deploy::already_running_refusal`'s shape: the
                // constant, inside the machine's own sentence.
                return Err(format!(
                    "REFUSED: {} — thing.rvt is the build in slot b, which this machine booted. \
                     NOTHING WAS INSTALLED AND NOTHING NEEDS TO BE.",
                    rivet_core::deploy::ALREADY_RUNNING,
                ));
            }
            if self.refuse_installs > 0 {
                self.refuse_installs -= 1;
                // The machine's own sentence, composed the way `cmd/rivet`'s
                // interlock composes it — around the one constant both ends read.
                return Err(format!(
                    "REFUSED {} at {} and NOTHING HAS BEEN DELIVERED.",
                    rivet_core::deploy::REQUEST_WAITING,
                    rivet_core::deploy::REQUEST_PATH,
                ));
            }
            self.installed.push(remote_path.to_string());
            Ok("{\"delivered\":true}".to_string())
        }
        // The two methods `cargo rivet deploy` does not use. A fake that
        // answered them would be claiming this test exercises a path it does
        // not; `rivet-fleet`'s own tests cover them.
        fn put_document(&mut self, _text: &str) -> Result<(), String> {
            Err("this fake does not write documents".to_owned())
        }

        fn ask(&mut self, _words: &[&str]) -> Result<String, String> {
            Err("this fake does not run commands".to_owned())
        }
    }

    /// A real file with real bytes, so the local digest is computed rather than
    /// asserted.
    ///
    /// **The directory is keyed by the content's digest and not its length**,
    /// which is not fussiness: it was keyed by length first, two tests happened
    /// to use payloads of the same size, and they raced to write one path. The
    /// symptom was a digest mismatch in a test about something else entirely.
    ///
    /// **The file name is one `crate::name::artifact` would compose**, because
    /// `--enable` reads the service name back out of it.
    fn payload(bytes: &[u8]) -> PathBuf {
        payload_named(bytes, "thing-0.1.0-aarch64-application.rvt")
    }

    /// [`payload`], under a name the test chooses — for a test about what
    /// `--enable` reads OUT of the name rather than what carries the bytes.
    fn payload_named(bytes: &[u8], name: &str) -> PathBuf {
        let (digest, _) = {
            let mut d = Blake3::new();
            d.start();
            d.write(bytes);
            (d.finish(), ())
        };
        let dir = std::env::temp_dir().join(format!("rivet-deploy-{}", hex(&digest)));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let path = dir.join(name);
        std::fs::write(&path, bytes).expect("a payload");
        path
    }

    fn args_for(path: &Path, confirmed: bool) -> DeployArgs {
        DeployArgs {
            host: "pi".to_string(),
            rvt: Some(path.to_path_buf()),
            package_args: Vec::new(),
            remote_dir: "/state".to_string(),

            confirmed,
            enable: false,
            identity: None,
            pin: None,
            // **Off for every fixture that predates the wait**: none of them
            // answers `get install`, and the wait has its own tests below with a
            // fake that does.
            wait: false,
        }
    }

    fn never_builds(_: &[String]) -> Result<(PathBuf, String), String> {
        panic!("the plan must not build anything")
    }

    #[test]
    fn without_the_word_nothing_is_built_sent_or_installed() {
        let path = payload(b"an unconfirmed payload");
        let mut board = FakeBoard::agreeing([0; 32]);
        let report = run(&args_for(&path, false), &mut never_builds, &mut board).expect("a plan");
        assert!(report.contains("PLAN"), "{report}");
        assert!(report.contains("thing-0.1.0-aarch64-application.rvt"), "{report}");
        // **The plan cannot name the digest and says so rather than inventing
        // one.** Nothing has been hashed at this point — an unconfirmed deploy
        // does not even build — so a plan that printed sixty-four characters
        // would be printing sixty-four characters somebody could compare
        // against the wrong thing.
        assert!(
            report.contains("rivet apply install <that artifact's digest> confirm"),
            "{report}"
        );
        assert!(report.contains("artifact store"), "{report}");
        assert!(board.pushed.is_empty() && board.installed.is_empty());
    }

    /// **The ruling, from the client's end**: what is asked for is a digest, and
    /// it is the digest of the file that was just sent.
    #[test]
    fn the_install_names_the_digest_of_the_payload_that_was_sent() {
        let path = payload(b"bytes whose digest is the request");
        let (digest, _) = blake3_of(&path).expect("a digest");
        let mut board = FakeBoard::agreeing(digest);
        let report = run(&args_for(&path, true), &mut never_builds, &mut board).expect("a deploy");
        assert_eq!(board.installed, [hex(&digest)]);
        assert!(report.contains(&hex(&digest)), "{report}");
        // Nothing in the whole exchange names a path on the machine.
        assert!(
            !board.installed[0].contains('/'),
            "a path reached the install: {:?}",
            board.installed
        );
    }

    /// **`--remote-dir` is refused rather than ignored**, because a machine no
    /// longer has a directory it could name — `ADR-052` §7.
    #[test]
    fn a_remote_directory_is_refused_because_there_is_nowhere_for_it_to_name() {
        let path = payload(b"bytes nobody gets to place");
        let mut board = FakeBoard::agreeing([0; 32]);
        let mut args = args_for(&path, true);
        args.remote_dir = "/state/elsewhere".to_string();
        let err = run(&args, &mut never_builds, &mut board).expect_err("refused");
        assert!(err.contains("ADR-052 §7"), "{err}");
        assert!(err.contains("Nothing has been sent"), "{err}");
        assert!(board.pushed.is_empty() && board.installed.is_empty());
    }

    /// **The branch this trait exists for.** A board whose copy differs must
    /// stop before the install and must say the transfer is what went wrong —
    /// the whole point being that a signature refusal would send the operator
    /// to look at their key.
    #[test]
    fn a_board_whose_copy_differs_is_not_installed_onto() {
        let path = payload(b"bytes that will not survive the wire");
        let mut board = FakeBoard::agreeing([0xab; 32]);
        let err = run(&args_for(&path, true), &mut never_builds, &mut board)
            .expect_err("the digests differ");
        assert!(err.contains("NOT the payload"), "{err}");
        assert!(err.contains("Nothing was installed"), "{err}");
        assert!(err.contains("not in your signing key"), "{err}");
        assert!(
            board.installed.is_empty(),
            "it installed a payload it had just refused"
        );
    }

    #[test]
    fn a_board_that_agrees_is_pushed_to_and_then_installed_onto_in_that_order() {
        let path = payload(b"bytes that arrive intact");
        let (digest, _) = blake3_of(&path).expect("a digest");
        let mut board = FakeBoard::agreeing(digest);
        let report = run(&args_for(&path, true), &mut never_builds, &mut board).expect("a deploy");
        assert_eq!(board.pushed.len(), 1);
        // The transfer still carries the `ADR-027` name — the store keeps it as
        // the name of the file inside the object's directory.
        assert_eq!(board.pushed[0].1, "/state/thing-0.1.0-aarch64-application.rvt");
        assert_eq!(board.installed, [hex(&digest)]);
        assert!(report.contains("byte-identical"), "{report}");
        // And it says what it did not do, which for a system update is the half
        // the operator has to act on.
        assert!(report.contains("NOTHING WAS RESTARTED"), "{report}");
        assert!(report.contains("trial-restart"), "{report}");
    }

    // ---- what the machine said about the transfer ---------------------------

    /// **`ADR-052` §7's first recorded defect, as a test.** The bulk endpoint's `201`
    /// body is the ONLY place a Rivet states which artifacts retention evicted,
    /// and `push` used to read it far enough to check the status line and then
    /// drop it. So a machine deleted artifacts and told nobody: an operator
    /// learned an artifact was gone by asking for it.
    #[test]
    fn what_retention_pruned_reaches_the_operators_report() {
        let path = payload(b"the fourth version of a product");
        let (digest, _) = blake3_of(&path).expect("a digest");
        let mut board = FakeBoard::agreeing(digest);
        // The endpoint's own lines, in the endpoint's own shape.
        board.answer_to_a_push = "/state/artifacts/objects/ab/thing.rvt\n\
                                  \x20 bytes   402198\n\
                                  \x20 blake3  ab12\n\
                                  \x20 pruned  photo-index-0.1.0-aarch64-application.rvt — this \
                                  store keeps the 3 newest of each product\n\
                                  \x20 next    nothing has been installed.\n"
            .to_string();

        let report = run(&args_for(&path, true), &mut never_builds, &mut board).expect("a deploy");
        assert!(
            report.contains("pruned  photo-index-0.1.0-aarch64-application.rvt"),
            "an artifact was deleted and the report did not say so:\n{report}"
        );
        // And only the lines that say something this end does not already know:
        // the digest and the length are printed by this tool itself, above.
        assert!(!report.contains("blake3  ab12"), "{report}");
    }

    /// The transport's own note — the machine answered before this end had
    /// finished sending — reaches the report by the same route.
    #[test]
    fn a_transfer_the_transport_had_to_explain_reaches_the_report() {
        let path = payload(b"an artifact the machine already held");
        let (digest, _) = blake3_of(&path).expect("a digest");
        let mut board = FakeBoard::agreeing(digest);
        board.answer_to_a_push =
            "/state/artifacts/objects/ab/thing.rvt\n\x20 note    this end's send FAILED and the \
             machine had already answered\n"
                .to_string();

        let report = run(&args_for(&path, true), &mut never_builds, &mut board).expect("a deploy");
        assert!(report.contains("note    this end's send FAILED"), "{report}");
    }

    /// **The machine's word that it verified the artifact before storing it**
    /// reaches the report — this end has no way to know it, because the check is
    /// against the machine's trust store rather than this workstation's.
    #[test]
    fn the_machines_verification_at_the_door_reaches_the_report() {
        let path = payload(b"an artifact the door admitted");
        let (digest, _) = blake3_of(&path).expect("a digest");
        let mut board = FakeBoard::agreeing(digest);
        board.answer_to_a_push = "/state/artifacts/objects/ab/thing.rvt\n\
                                  \x20 checked this machine hashed what it received.\n\
                                  \x20 verified signed by \"rivet-app\", which this machine's \
                                  trust store permits to sign application\n"
            .to_string();

        let report = run(&args_for(&path, true), &mut never_builds, &mut board).expect("a deploy");
        assert!(report.contains("verified signed by \"rivet-app\""), "{report}");
    }

    /// **A refused push stops the deploy before an install is asked for**, and
    /// the refusal reaches the operator whole.
    #[test]
    fn an_artifact_the_machine_refused_is_never_asked_to_install() {
        let path = payload(b"an artifact signed by a stranger");
        let (digest, _) = blake3_of(&path).expect("a digest");
        let mut board = FakeBoard::agreeing(digest);
        board.refuse_push = Some(
            "board.local REFUSED THE ARTIFACT thing.rvt. The transfer itself was intact — this \
             is not a transfer failure"
                .to_string(),
        );

        let err = run(&args_for(&path, true), &mut never_builds, &mut board)
            .expect_err("a refused artifact is not deployed");
        assert!(err.contains("REFUSED THE ARTIFACT"), "{err}");
        assert!(board.installed.is_empty(), "an install was asked for: {:?}", board.installed);
    }

    // ---- a machine already running the artifact ------------------------------

    /// **`PLAN-Feature-Set-2026-09-12` §3, from the tool's end.** The artifact
    /// is transferred — so the store has it — and then the deploy STOPS: it
    /// reports that the machine already runs it and that no install was
    /// requested, exits zero, and never says *install requested*. Until
    /// 2026-09-12 the request was delivered and the installer wrote a second
    /// copy of the running build over the rollback.
    #[test]
    fn a_machine_already_running_the_artifact_is_sent_it_and_then_the_deploy_stops() {
        let path = payload(b"the build the board is already running");
        let (digest, _) = blake3_of(&path).expect("a digest");
        let mut board = FakeBoard::agreeing(digest);
        board.already_running = true;

        let report = run(&args_for(&path, true), &mut never_builds, &mut board)
            .expect("a machine already in the state the deploy was for is not a failure");
        assert_eq!(board.pushed.len(), 1, "the artifact must still be transferred");
        assert!(board.installed.is_empty(), "{:?}", board.installed);
        assert!(report.contains("ALREADY RUNNING"), "{report}");
        assert!(report.contains("NO INSTALL WAS REQUESTED"), "{report}");
        assert!(report.contains(rivet_core::deploy::ALREADY_RUNNING), "{report}");
        assert!(!report.contains("install  requested"), "{report}");
    }

    /// And it is **not waited out**: the refusal describes a state that will be
    /// the same in two minutes.
    #[test]
    fn already_running_is_answered_on_the_first_request_without_a_pause() {
        let mut board = FakeBoard::agreeing([0; 32]);
        board.already_running = true;
        let mut report = String::new();
        let err = install_when_the_slot_is_free(
            &mut board,
            "board.local:7777",
            &"ab".repeat(32),
            &mut report,
            SLOT_PATIENCE,
            &mut |_| panic!("already running must not be waited out"),
            false,
        )
        .expect_err("refused");
        assert!(err.contains(rivet_core::deploy::ALREADY_RUNNING), "{err}");
    }

    // ---- one install request at a time --------------------------------------

    /// **`ADR-052` §7's second recorded defect, as a test.** The request file is one
    /// slot; four deploys back to back failed on the second. The tool asks again
    /// until the slot clears.
    ///
    /// No wall-clock time passes: `rest` is a parameter for exactly this.
    #[test]
    fn an_occupied_install_slot_is_waited_out_rather_than_failing_the_deploy() {
        let mut board = FakeBoard::agreeing([0; 32]);
        board.refuse_installs = 3;
        let mut report = String::new();
        let mut slept: Vec<Duration> = Vec::new();

        let (said, before) = install_when_the_slot_is_free(
            &mut board,
            "board.local:7777",
            &"ab".repeat(32),
            &mut report,
            SLOT_PATIENCE,
            &mut |how_long| slept.push(how_long),
            false,
        )
        .expect("the slot cleared and the install was requested");
        assert_eq!(before, None, "nothing was observed, so there is no baseline");

        assert!(said.contains("delivered"), "{said}");
        assert_eq!(board.installed.len(), 1, "{:?}", board.installed);
        // It backs off rather than hammering, and stays under the ceiling.
        assert_eq!(slept.len(), 3, "{slept:?}");
        assert_eq!(slept[0], SLOT_FIRST_PAUSE);
        assert!(slept[1] > slept[0] && slept[2] > slept[1], "{slept:?}");
        assert!(slept.iter().all(|d| *d <= SLOT_LONGEST_PAUSE), "{slept:?}");
        // And the operator is told what the quiet was about, both times.
        assert!(report.contains("already has an install request waiting"), "{report}");
        assert!(report.contains("ALREADY TRANSFERRED"), "{report}");
        assert!(report.contains("waited"), "{report}");
    }

    /// **Only that one refusal is waited out.** Every other refusal from the
    /// install operation describes a state that will be the same in two minutes,
    /// and retrying it would turn a clear error into a long one.
    #[test]
    fn a_refusal_that_is_not_the_interlock_is_returned_at_once() {
        struct Adamant(u32);
        impl Remote for Adamant {
            fn push(&mut self, _: &Path, _: &str) -> Result<String, String> {
                unreachable!("this fixture is only asked for an install")
            }
            fn blake3(&mut self, _: &str) -> Result<[u8; 32], String> {
                unreachable!("this fixture is only asked for an install")
            }
            fn install(&mut self, _: &str) -> Result<String, String> {
                self.0 += 1;
                Err("REFUSED this machine's store does not hold ab12… and NOTHING HAS BEEN \
                     DELIVERED."
                    .to_string())
            }
            fn read_document(&mut self) -> Result<Option<String>, String> {
                unreachable!()
            }
            fn edit_document(&mut self, _: &ConfigChange) -> Result<String, String> {
                unreachable!()
            }
            fn put_document(&mut self, _: &str) -> Result<(), String> {
                unreachable!()
            }
            fn ask(&mut self, _: &[&str]) -> Result<String, String> {
                unreachable!()
            }
        }

        let mut board = Adamant(0);
        let mut report = String::new();
        let err = install_when_the_slot_is_free(
            &mut board,
            "board.local:7777",
            &"ab".repeat(32),
            &mut report,
            SLOT_PATIENCE,
            &mut |_| panic!("a refusal that is not the interlock must not be waited out"),
            false,
        )
        .expect_err("the store does not hold it, and will not in two minutes");
        assert!(err.contains("does not hold"), "{err}");
        assert_eq!(board.0, 1, "it asked more than once");
        assert!(report.is_empty(), "{report}");
    }

    /// **A slot that never clears is an honest failure**, naming what was waited
    /// for, for how long, and — the part that matters to an operator staring at
    /// a deploy that failed — that the artifact is already on the machine and
    /// does not need re-sending.
    #[test]
    fn a_slot_that_never_clears_says_what_it_waited_for_and_that_nothing_needs_resending() {
        let mut board = FakeBoard::agreeing([0; 32]);
        board.refuse_installs = u32::MAX;
        let mut report = String::new();
        let mut slept = Duration::ZERO;

        let err = install_when_the_slot_is_free(
            &mut board,
            "board.local:7777",
            &"ab".repeat(32),
            &mut report,
            // A second's patience, so the loop reaches its ceiling in a test.
            Duration::from_secs(1),
            &mut |how_long| slept += how_long,
            false,
        )
        .expect_err("the slot never cleared");

        assert!(err.contains(rivet_core::deploy::REQUEST_WAITING), "{err}");
        assert!(err.contains("THE ARTIFACT WAS TRANSFERRED"), "{err}");
        assert!(err.contains("nothing needs re-sending"), "{err}");
        assert!(err.contains(rivet_core::deploy::REQUEST_PATH), "{err}");
        assert!(board.installed.is_empty(), "{:?}", board.installed);
        assert!(slept >= Duration::from_secs(1), "it gave up early: {slept:?}");
    }

    /// And the wait is **wired into the deploy**, not merely available beside
    /// it: a machine that refuses the first install request still ends with the
    /// artifact installed. This one really sleeps, once, for
    /// [`SLOT_FIRST_PAUSE`].
    #[test]
    fn a_deploy_whose_first_install_request_is_refused_still_installs() {
        let path = payload(b"the second of two deploys in a row");
        let (digest, _) = blake3_of(&path).expect("a digest");
        let mut board = FakeBoard::agreeing(digest);
        board.refuse_installs = 1;

        let report = run(&args_for(&path, true), &mut never_builds, &mut board)
            .expect("the second deploy in a row must not be lost");
        assert_eq!(board.installed, [hex(&digest)]);
        assert!(report.contains("waiting"), "{report}");
        assert!(report.contains("install  requested by digest"), "{report}");
    }

    // ---- what the document already says -------------------------------------

    /// A document `rivet-config` rendered, which is what one on a board is.
    const CANONICAL: &str = "version = 1\n\n[services.rivet-mdnsd]\nenabled = true\n\n\
                             [services.photo-index]\nenabled = false\n\n\
                             [network.eth0]\nmethod = \"dhcp\"\n";

    #[test]
    fn a_document_that_says_nothing_about_a_service_gets_a_table() {
        assert_eq!(standing(CANONICAL, "metrics"), Standing::Absent);
        let change = change_for(&Standing::Absent, "metrics").expect("something to do");
        assert_eq!(
            change,
            ConfigChange::Append {
                // A leading newline, or a document whose last line lacks one
                // would gain a table header glued onto it.
                text: "\n[services.metrics]\nenabled = true\n".to_string()
            }
        );
    }

    #[test]
    fn a_document_that_says_false_gets_one_word_changed() {
        assert_eq!(standing(CANONICAL, "photo-index"), Standing::Disabled);
        let change = change_for(&Standing::Disabled, "photo-index").expect("something to do");
        let ConfigChange::Replace { expect, replace } = change else {
            panic!("appending would make a second [services.photo-index] table");
        };
        // The expect has to be unique in the file or `rivet apply edit`
        // refuses — which is the property relied on. `enabled = false` alone
        // would not be, once two services are disabled.
        assert_eq!(CANONICAL.matches(&expect).count(), 1, "{expect:?}");
        assert!(replace.ends_with("enabled = true"));
    }

    #[test]
    fn a_document_that_already_says_true_is_left_alone() {
        assert_eq!(standing(CANONICAL, "rivet-mdnsd"), Standing::Enabled);
        assert_eq!(change_for(&Standing::Enabled, "rivet-mdnsd"), None);
    }

    /// **The refusal that matters.** A hand-written document need not put
    /// `enabled` on the line after the header, and a tool that edited one blind
    /// could mean something other than what it looks like.
    #[test]
    fn a_document_this_tool_did_not_write_is_refused_rather_than_edited() {
        for odd in [
            // No `enabled` at all before the next table — which the machine
            // refuses too, but for its own reasons.
            "version = 1\n[services.x]\n[network.eth0]\nmethod = \"dhcp\"\n",
            // A value that is not a bool.
            "version = 1\n[services.x]\nenabled = \"yes\"\n",
            // The last table, with nothing under it.
            "version = 1\n[services.x]\n",
        ] {
            assert_eq!(standing(odd, "x"), Standing::Unreadable, "{odd:?}");
            assert_eq!(change_for(&Standing::Unreadable, "x"), None);
        }
    }

    /// A service whose `enabled` is missing must not borrow the next service's.
    #[test]
    fn a_service_with_no_enabled_does_not_read_the_next_tables() {
        let text = "version = 1\n[services.a]\n[services.b]\nenabled = true\n";
        assert_eq!(standing(text, "a"), Standing::Unreadable);
        assert_eq!(standing(text, "b"), Standing::Enabled);
    }

    /// **A header inside a comment is not a table.** `other`'s own
    /// `enabled = true` sits between the comment and the real
    /// `[services.demo]` table, which a raw substring search reads as
    /// `demo`'s line; the real table, two lines further down, says `false`.
    #[test]
    fn a_commented_heading_does_not_stand_in_for_the_real_table() {
        let text = "version = 1\n\
                     [services.other]\n\
                     # [services.demo]\n\
                     enabled = true\n\
                     [services.demo]\n\
                     enabled = false\n";
        assert_eq!(standing(text, "demo"), Standing::Disabled);
        let change = change_for(&Standing::Disabled, "demo").expect("something to do");
        let ConfigChange::Replace { expect, replace } = change else {
            panic!("demo already has a table; this must be a replace");
        };
        assert_eq!(expect, "[services.demo]\nenabled = false");
        assert_eq!(replace, "[services.demo]\nenabled = true");
    }

    /// A comment is never mistaken for a table that does not otherwise exist.
    #[test]
    fn a_commented_heading_with_no_real_table_is_absent() {
        let text = "version = 1\n# [services.ghost]\nenabled = true\n";
        assert_eq!(standing(text, "ghost"), Standing::Absent);
    }

    // ---- `[kiosk] auto-run` — `.rulings/` R-214 ------------------------------

    #[test]
    fn no_kiosk_table_at_all_is_no_table() {
        assert_eq!(kiosk_auto_run(CANONICAL), KioskAutoRun::NoTable);
        assert_eq!(kiosk_change_for(&KioskAutoRun::NoTable, "aura-renderer"), None);
    }

    #[test]
    fn a_kiosk_table_names_its_application() {
        let text = "version = 1\n[kiosk]\nauto-run = \"rivet-launcher\"\nlocked = false\n";
        assert_eq!(kiosk_auto_run(text), KioskAutoRun::Named("rivet-launcher".to_string()));
    }

    /// **The defect's own case.** A different application is already the
    /// auto-run; the change replaces exactly that line and nothing else.
    #[test]
    fn a_different_named_application_is_replaced() {
        let text = "version = 1\n[kiosk]\nauto-run = \"rivet-launcher\"\n";
        let current = kiosk_auto_run(text);
        let change = kiosk_change_for(&current, "aura-renderer").expect("something to do");
        let ConfigChange::Replace { expect, replace } = change else {
            panic!("a named auto-run already has a line; this must be a replace");
        };
        assert_eq!(text.matches(&expect).count(), 1, "{expect:?}");
        assert_eq!(expect, "auto-run = \"rivet-launcher\"");
        assert_eq!(replace, "auto-run = \"aura-renderer\"");
    }

    /// Naming the application already there is nothing to do.
    #[test]
    fn the_same_application_already_named_is_left_alone() {
        let text = "version = 1\n[kiosk]\nauto-run = \"aura-renderer\"\n";
        let current = kiosk_auto_run(text);
        assert_eq!(current, KioskAutoRun::Named("aura-renderer".to_string()));
        assert_eq!(kiosk_change_for(&current, "aura-renderer"), None);
    }

    /// **The refusal that matters, restated for the second table this tool
    /// edits.** A hand-written or malformed `[kiosk]` table is never guessed
    /// at — the same posture `Standing::Unreadable` takes.
    #[test]
    fn a_kiosk_table_this_tool_did_not_write_is_refused_rather_than_edited() {
        for odd in [
            "version = 1\n[kiosk]\n[network.eth0]\nmethod = \"dhcp\"\n",
            "version = 1\n[kiosk]\nauto-run = aura-renderer\n",
            "version = 1\n[kiosk]\nauto-run = \"aura-renderer\n",
        ] {
            assert_eq!(kiosk_auto_run(odd), KioskAutoRun::Unreadable, "{odd:?}");
            assert_eq!(kiosk_change_for(&KioskAutoRun::Unreadable, "aura-renderer"), None);
        }
    }

    /// A `[kiosk]` table inside a comment is not a table, exactly as
    /// `standing`'s own equivalent case reads `[services.<name>]`.
    #[test]
    fn a_commented_kiosk_heading_is_not_a_table() {
        let text = "version = 1\n# [kiosk]\nauto-run = \"ghost\"\n";
        assert_eq!(kiosk_auto_run(text), KioskAutoRun::NoTable);
    }

    // ---- which service --enable names ---------------------------------------

    fn enable_args(rvt: &str) -> DeployArgs {
        DeployArgs {
            host: "pi".to_string(),
            rvt: Some(PathBuf::from(rvt)),
            package_args: Vec::new(),
            remote_dir: "/state".to_string(),

            confirmed: true,
            enable: true,
            identity: None,
            pin: None,
            wait: false,
        }
    }

    #[test]
    fn the_service_to_enable_is_the_payloads_product() {
        let args = enable_args("x.rvt");
        let name = service_to_enable(&args, "rivet-todo-0.1.1-g3d28ede-aarch64-application.rvt")
            .expect("an application");
        assert_eq!(name, "rivet-todo");
    }

    /// **A digit-led product suffix is part of the product, not the version.**
    /// `sensor-2` and `sensor` are two different, equally valid services
    /// (`ServiceName` allows a digit-containing name), and `--enable` must
    /// never fold the one into the other.
    #[test]
    fn the_service_to_enable_keeps_a_numeric_product_suffix() {
        let args = enable_args("x.rvt");
        let name = service_to_enable(&args, "sensor-2-0.1.0-aarch64-application.rvt")
            .expect("an application");
        assert_eq!(name, "sensor-2");
    }

    /// **A system update has no service to enable**, and saying so before
    /// anything is sent is the point: `[services.rivet]` is not a thing.
    #[test]
    fn enable_refuses_a_system_update_before_anything_is_sent() {
        let args = enable_args("x.rvt");
        let err = service_to_enable(&args, "rivet-0.0.1-9d3f3a0-aarch64-rpi4-update.rvt")
            .expect_err("a system update");
        assert!(err.contains("update"), "{err}");
        assert!(err.contains("trial-restart"), "{err}");
        assert!(err.contains("Nothing has been sent"), "{err}");
    }

    #[test]
    fn enable_refuses_a_name_this_tool_did_not_compose() {
        let args = enable_args("x.rvt");
        let err = service_to_enable(&args, "something-somebody-renamed.rvt")
            .expect_err("not a composed name");
        assert!(err.contains("ADR-027"), "{err}");
        assert!(err.contains("apply edit"), "{err}");
    }

    // ---- the order, which is the whole design -------------------------------

    struct EnablingBoard {
        document: Option<String>,
        edits: Vec<ConfigChange>,
        /// Every side effect, in the order it happened.
        order: Vec<&'static str>,
        answer: [u8; 32],
    }

    impl Remote for EnablingBoard {
        fn push(&mut self, _local: &Path, _remote_path: &str) -> Result<String, String> {
            self.order.push("push");
            Ok(String::new())
        }
        fn blake3(&mut self, _remote_path: &str) -> Result<[u8; 32], String> {
            Ok(self.answer)
        }
        fn install(&mut self, _remote_path: &str) -> Result<String, String> {
            self.order.push("install");
            Ok(String::new())
        }
        fn read_document(&mut self) -> Result<Option<String>, String> {
            Ok(self.document.clone())
        }
        fn edit_document(&mut self, change: &ConfigChange) -> Result<String, String> {
            self.order.push("edit");
            self.edits.push(change.clone());
            Ok(String::new())
        }
        // The two methods `cargo rivet deploy` does not use. A fake that
        // answered them would be claiming this test exercises a path it does
        // not; `rivet-fleet`'s own tests cover them.
        fn put_document(&mut self, _text: &str) -> Result<(), String> {
            Err("this fake does not write documents".to_owned())
        }

        fn ask(&mut self, _words: &[&str]) -> Result<String, String> {
            Err("this fake does not run commands".to_owned())
        }
    }

    fn enabling_board(document: Option<&str>, answer: [u8; 32]) -> EnablingBoard {
        EnablingBoard {
            document: document.map(str::to_string),
            edits: Vec::new(),
            order: Vec::new(),
            answer,
        }
    }

    /// **The document is edited BEFORE the payload is sent**, and the whole of
    /// `--enable` depends on it: `cmd/rivet-sysd` re-reads the document when
    /// an install finishes and at no other moment, so a line written afterwards
    /// is not seen until the machine restarts.
    #[test]
    fn the_document_is_edited_before_the_payload_is_sent() {
        let path = payload(b"an application that should run");
        let (digest, _) = blake3_of(&path).expect("a digest");
        let mut board = enabling_board(Some("version = 1\n"), digest);
        let mut args = args_for(&path, true);
        args.enable = true;
        args.rvt = Some(path.clone());

        let report = run(&args, &mut never_builds, &mut board).expect("a deploy");
        assert_eq!(
            board.order,
            ["edit", "push", "install"],
            "the supervisor would not have seen the line when the install finished"
        );
        assert!(report.contains("enabled"), "{report}");
    }

    /// A payload named `sensor-2-...` must have its OWN table added — never
    /// `sensor`'s, which is a different product this same document could
    /// separately name. Observed through the actual edit the fake remote
    /// receives, not just the service name `service_to_enable` computes.
    #[test]
    fn enabling_a_numeric_suffixed_product_never_edits_the_shorter_name() {
        let path = payload_named(
            b"a sensor build with a numeric product suffix",
            "sensor-2-0.1.0-aarch64-application.rvt",
        );
        let (digest, _) = blake3_of(&path).expect("a digest");
        let mut board = enabling_board(Some("version = 1\n"), digest);
        let mut args = args_for(&path, true);
        args.enable = true;

        run(&args, &mut never_builds, &mut board).expect("a deploy");
        assert_eq!(board.edits.len(), 1, "{:?}", board.edits);
        let ConfigChange::Append { text } = &board.edits[0] else {
            panic!("sensor-2 has no table yet, so this must be an append: {:?}", board.edits[0]);
        };
        assert!(text.contains("[services.sensor-2]"), "{text:?}");
        assert!(!text.contains("[services.sensor]"), "{text:?}");
    }

    /// A document edit that is refused must leave the machine with nothing
    /// installed, rather than an application nothing will start.
    #[test]
    fn a_refused_document_edit_sends_nothing() {
        let path = payload(b"an application nobody can enable");
        let mut board = enabling_board(None, [0; 32]);
        let mut args = args_for(&path, true);
        args.enable = true;

        let err = run(&args, &mut never_builds, &mut board).expect_err("no document");
        assert!(err.contains("Nothing has been sent"), "{err}");
        assert!(board.order.is_empty(), "{:?}", board.order);
    }

    /// Without the flag, nothing touches the document — and the report says so,
    /// because an operator whose application did not start needs to know why.
    #[test]
    fn without_the_flag_the_document_is_not_touched_and_the_plan_says_so() {
        let path = payload(b"an application left idle");
        let (digest, _) = blake3_of(&path).expect("a digest");
        let mut board = enabling_board(Some("version = 1\n"), digest);
        let args = args_for(&path, true);

        run(&args, &mut never_builds, &mut board).expect("a deploy");
        assert_eq!(board.order, ["push", "install"]);
        assert!(board.edits.is_empty());

        let mut planned = args_for(&path, false);
        planned.enable = false;
        let plan = run(&planned, &mut never_builds, &mut board).expect("a plan");
        assert!(plan.contains("would NOT"), "{plan}");
        assert!(plan.contains("§3.2.2"), "{plan}");
    }

    // ---- waiting for THIS install, and not somebody else's -------------------

    /// **`get install`'s answer, in the shape `rivet_core::operation::install`
    /// composes it.** HAND-COMPOSED (`AGENTS.md` §4), from that module's
    /// `GET_OUTPUTS` and `perform_get`: the keys in their declared order, compact
    /// JSON, and a `records` entry carrying its own `outcome` ESCAPED — the decoy
    /// a reader that searched for `"outcome":` naively would read instead of the
    /// answer's own.
    fn get_install(corr: Option<&str>, finished: bool, outcome: Option<&str>, succeeded: Option<bool>) -> String {
        let Some(corr) = corr else {
            return "{\"subject\":\"install\",\"installs\":0,\"records\":[],\"returned\":0,\
                    \"unreadable\":0,\"source\":\"/state/log\",\"next\":\"no install\"}"
                .to_string();
        };
        let mut out = format!(
            "{{\"subject\":\"install\",\"installs\":2,\"correlation\":\"{corr}\",\"boot\":\"b1\",\
             \"finished\":{finished}"
        );
        if let Some(outcome) = outcome {
            out.push_str(&format!(",\"outcome\":\"{outcome}\""));
        }
        if let Some(succeeded) = succeeded {
            out.push_str(&format!(",\"succeeded\":{succeeded},\"means\":\"it ended\""));
        }
        out.push_str(&format!(
            ",\"errors\":0,\"warnings\":0,\"last\":\"the installer's last words\",\
             \"records\":[\"{{\\\"svc\\\":\\\"rivet-deploy\\\",\\\"corr\\\":\\\"{corr}\\\",\
             \\\"outcome\\\":\\\"decoy\\\"}}\"],\"returned\":1,\"unreadable\":0,\
             \"source\":\"/state/log\",\"next\":\"read succeeded\"}}"
        ));
        out
    }

    /// A machine whose `get install` answers are a script, consumed one per
    /// read — the LAST one repeats — and which remembers every install asked of
    /// it and every read, in order.
    struct JournalBoard {
        answer: [u8; 32],
        answers: std::collections::VecDeque<String>,
        order: Vec<String>,
        installed: Vec<String>,
    }

    impl JournalBoard {
        fn scripted(answer: [u8; 32], answers: &[String]) -> Self {
            Self { answer, answers: answers.iter().cloned().collect(), order: Vec::new(), installed: Vec::new() }
        }
    }

    impl Remote for JournalBoard {
        fn push(&mut self, _: &Path, _: &str) -> Result<String, String> {
            self.order.push("push".to_string());
            Ok(String::new())
        }
        fn blake3(&mut self, _: &str) -> Result<[u8; 32], String> {
            Ok(self.answer)
        }
        fn install(&mut self, digest: &str) -> Result<String, String> {
            self.order.push("install".to_string());
            self.installed.push(digest.to_string());
            Ok("{\"delivered\":true}".to_string())
        }
        fn read_document(&mut self) -> Result<Option<String>, String> {
            unreachable!("these deploys do not touch the document")
        }
        fn edit_document(&mut self, _: &ConfigChange) -> Result<String, String> {
            unreachable!("these deploys do not touch the document")
        }
        fn put_document(&mut self, _: &str) -> Result<(), String> {
            Err("this fake does not write documents".to_owned())
        }
        fn ask(&mut self, words: &[&str]) -> Result<String, String> {
            Err(format!("`get install` is a catalogue operation and was asked through ask: {words:?}"))
        }
        fn perform_operation(&mut self, words: &[&str]) -> Result<String, String> {
            assert_eq!(words, ["get", "install"], "the only operation a wait performs");
            self.order.push("get install".to_string());
            let answer = if self.answers.len() > 1 { self.answers.pop_front() } else { self.answers.front().cloned() };
            answer.ok_or_else(|| "this fake has no get install answer".to_string())
        }
    }

    /// **The trap `AGENTS.md` §10.6 names, as a test.** The machine installed
    /// something earlier and `get install` already answers `finished: true,
    /// succeeded: true` about THAT. A wait that polled `finished` alone would
    /// return at once and report someone else's outcome; this one keeps reading
    /// until the correlation changes, then until THIS chain finishes.
    #[test]
    fn the_wait_is_for_this_installs_correlation_and_not_an_earlier_finished_one() {
        let earlier = get_install(Some("corr-earlier"), true, Some("installed"), Some(true));
        let mut board = JournalBoard::scripted(
            [0; 32],
            &[
                earlier.clone(), // the baseline, read immediately before the request
                earlier.clone(), // the supervisor has not taken the request up yet
                earlier,
                get_install(Some("corr-ours"), false, None, None),
                get_install(Some("corr-ours"), true, Some("installed-application"), Some(true)),
            ],
        );
        let mut report = String::new();
        let (_, before) = install_when_the_slot_is_free(
            &mut board,
            "board.local:7777",
            &"ab".repeat(32),
            &mut report,
            SLOT_PATIENCE,
            &mut |_| panic!("the slot was free"),
            true,
        )
        .expect("requested");
        assert_eq!(before, Some(Some("corr-earlier".to_string())));
        assert_eq!(board.order, ["get install", "install"], "the baseline is read BEFORE the request");

        let mut slept: Vec<Duration> = Vec::new();
        let outcome = wait_for_own_install(
            &mut board,
            "board.local:7777",
            &Some("corr-earlier".to_string()),
            &mut report,
            INSTALL_PATIENCE,
            &mut |d| slept.push(d),
        )
        .expect("this install finished well");
        assert_eq!(outcome, "installed-application", "the earlier install's outcome was reported");
        assert_eq!(slept.len(), 3, "it stopped reading before THIS install finished: {slept:?}");
        assert!(slept.iter().all(|d| *d <= INSTALL_LONGEST_PAUSE), "{slept:?}");
        assert!(report.contains("corr-ours"), "{report}");
        assert!(!report.contains("decoy"), "a record's own outcome was read as the answer's:\n{report}");
    }

    /// **A slot held by somebody else's request moves the baseline**: the one a
    /// wait uses is read immediately before the request that was ACCEPTED, so
    /// the other install — which starts in between — is not taken for this one.
    #[test]
    fn the_baseline_is_the_one_read_before_the_accepted_request() {
        struct Busy {
            inner: JournalBoard,
            refuse: u32,
        }
        impl Remote for Busy {
            fn push(&mut self, l: &Path, r: &str) -> Result<String, String> {
                self.inner.push(l, r)
            }
            fn blake3(&mut self, r: &str) -> Result<[u8; 32], String> {
                self.inner.blake3(r)
            }
            fn install(&mut self, digest: &str) -> Result<String, String> {
                if self.refuse > 0 {
                    self.refuse -= 1;
                    return Err(format!("REFUSED {} and NOTHING HAS BEEN DELIVERED.", rivet_core::deploy::REQUEST_WAITING));
                }
                self.inner.install(digest)
            }
            fn read_document(&mut self) -> Result<Option<String>, String> {
                self.inner.read_document()
            }
            fn edit_document(&mut self, c: &ConfigChange) -> Result<String, String> {
                self.inner.edit_document(c)
            }
            fn put_document(&mut self, t: &str) -> Result<(), String> {
                self.inner.put_document(t)
            }
            fn ask(&mut self, w: &[&str]) -> Result<String, String> {
                self.inner.ask(w)
            }
            fn perform_operation(&mut self, w: &[&str]) -> Result<String, String> {
                self.inner.perform_operation(w)
            }
        }
        let mut board = Busy {
            inner: JournalBoard::scripted(
                [0; 32],
                &[
                    get_install(Some("corr-old"), true, Some("installed"), Some(true)),
                    // somebody else's request was taken up while this one waited
                    get_install(Some("corr-theirs"), true, Some("installed-application"), Some(true)),
                ],
            ),
            refuse: 1,
        };
        let mut report = String::new();
        let (_, before) = install_when_the_slot_is_free(
            &mut board,
            "board.local:7777",
            &"ab".repeat(32),
            &mut report,
            SLOT_PATIENCE,
            &mut |_| {},
            true,
        )
        .expect("requested once the slot cleared");
        assert_eq!(before, Some(Some("corr-theirs".to_string())));
    }

    /// **A failed install exits non-zero, naming its outcome** — and it is the
    /// ordinary path through [`run_detailed`], not a helper on its own: a fresh
    /// machine, one request, one read that says it finished badly.
    #[test]
    fn an_install_that_finishes_badly_is_a_failure_naming_its_outcome() {
        let path = payload(b"a payload the installer refuses");
        let (digest, _) = blake3_of(&path).expect("a digest");
        let mut board = JournalBoard::scripted(
            digest,
            &[get_install(None, false, None, None), get_install(Some("corr-1"), true, Some("refused-payload"), Some(false))],
        );
        let mut args = args_for(&path, true);
        args.wait = true;

        let err = run_detailed(&args, &mut never_builds, &mut board).expect_err("the install failed");
        assert!(err.starts_with("THE INSTALL FAILED"), "the first line must say so:\n{err}");
        assert!(err.contains("refused-payload"), "{err}");
        assert!(err.contains("corr-1"), "{err}");
        assert!(err.contains("the installer's last words"), "{err}");
        assert_eq!(board.order, ["push", "get install", "install", "get install"]);
    }

    /// And the success of the same path says what happened as a value, so a
    /// caller planning what comes next does not read it out of prose.
    #[test]
    fn an_install_that_finishes_well_is_reported_as_installed() {
        let path = payload(b"a payload the installer accepts");
        let (digest, _) = blake3_of(&path).expect("a digest");
        let mut board = JournalBoard::scripted(
            digest,
            &[get_install(None, false, None, None), get_install(Some("corr-1"), true, Some("installed-application"), Some(true))],
        );
        let mut args = args_for(&path, true);
        args.wait = true;

        let deployed = run_detailed(&args, &mut never_builds, &mut board).expect("installed");
        assert_eq!(deployed.delivered, Delivered::Installed { outcome: "installed-application".to_string() });
        assert!(deployed.report.contains("finished"), "{}", deployed.report);
        assert!(!deployed.report.contains("DID NOT WAIT"), "{}", deployed.report);
    }

    /// **A baseline that cannot be read refuses before the request.** An install
    /// this caller asked to wait for and could not recognise is not requested.
    #[test]
    fn an_unreadable_baseline_requests_nothing() {
        let path = payload(b"a payload whose finish could not be recognised");
        let (digest, _) = blake3_of(&path).expect("a digest");
        let mut board = JournalBoard::scripted(digest, &["{\"subject\":\"log\"}".to_string()]);
        let mut args = args_for(&path, true);
        args.wait = true;

        let err = run_detailed(&args, &mut never_builds, &mut board).expect_err("refused");
        assert!(err.contains("NOTHING WAS INSTALLED"), "{err}");
        assert!(err.contains("--no-wait"), "{err}");
        assert!(board.installed.is_empty(), "{:?}", board.installed);
    }

    /// An install that never begins is reported as that — not as a timeout
    /// with nothing said — and the answer says nothing needs re-sending.
    #[test]
    fn an_install_that_never_begins_says_so_within_its_patience() {
        let baseline = get_install(Some("corr-old"), true, Some("installed"), Some(true));
        let mut board = JournalBoard::scripted([0; 32], &[baseline]);
        let mut report = String::new();
        let mut slept = Duration::ZERO;
        let err = wait_for_own_install(
            &mut board,
            "board.local:7777",
            &Some("corr-old".to_string()),
            &mut report,
            Duration::from_secs(2),
            &mut |d| slept += d,
        )
        .expect_err("it never began");
        assert!(err.contains("no install began"), "{err}");
        assert!(err.contains("nothing needs re-sending"), "{err}");
        assert!(slept >= Duration::from_secs(2), "it gave up early: {slept:?}");
    }

    /// `--no-wait` is the old behaviour exactly: no `get install` at all, and a
    /// report that says the install was not waited for.
    #[test]
    fn without_the_wait_nothing_reads_get_install() {
        let path = payload(b"a payload nobody waits for");
        let (digest, _) = blake3_of(&path).expect("a digest");
        let mut board = JournalBoard::scripted(digest, &[]);
        let deployed = run_detailed(&args_for(&path, true), &mut never_builds, &mut board).expect("requested");
        assert_eq!(deployed.delivered, Delivered::Requested);
        assert_eq!(board.order, ["push", "install"]);
        assert!(deployed.report.contains("DID NOT WAIT"), "{}", deployed.report);
    }
}
