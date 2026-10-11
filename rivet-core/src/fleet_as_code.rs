//! **Fleet-as-code's git operations, as the control plane serves them** —
//! `ADR-044`, `ADR-035`.
//!
//! A fleet server's desired-state documents live in a bare repository on that
//! server, under `crate::git::REPOSITORY_ROOT`, declared by the server's own
//! document as `[repos.<name>]`. `cmd/rivet-mcpd` runs git against it as root
//! and answers four operations; `cmd/rivet-fleetd` asks for them over its own
//! machine's control plane and runs no program itself.
//!
//! # Why the control plane holds the git, and not `rivet-fleetd`
//!
//! `crate::confinement::policy` grants an application no execute right under
//! `/system/bin` and no access to `/state/git`, so a confined `rivet-fleetd`
//! can neither run git nor open the repository. `cmd/rivet-mcpd` already runs
//! git as root for smart HTTP and for the repository store. The operations are
//! the whole of what the poller and `POST /api/commit/<device>` need, and the
//! application sandbox is unchanged.
//!
//! # What this module holds
//!
//! The words both ends must agree on: the routes, the request and answer
//! formats, the refusals, and the rules a path, a branch, a commit and a
//! repository name must satisfy. **No I/O**, per `ADR-014` §3.
//!
//! # The operations
//!
//! ```text
//! POST /fleet-as-code/head       repository, branch                  the commit the branch names, and its message
//! POST /fleet-as-code/machines   repository, commit                  every machine document that commit holds
//! POST /fleet-as-code/file       repository, commit, path            one machine document, as that commit has it
//! POST /fleet-as-code/commit     repository, branch, expected-head,  one machine document committed onto the branch,
//!                                path, message, document             only if its tip is still expected-head
//! ```
//!
//! A request body is `word value` lines. A commit's ends with a blank line, the
//! message and then the document, their lengths declared in the lines above.
//! A body of lines needs no URL encoding for a label a person typed, and every
//! value is checked here before either end acts on it.
//!
//! # Who may ask
//!
//! A key in the machine's `peers`: the operator's identity, and the fleet
//! server's own operator identity, which provisioning writes. [`permits`] is
//! the rule as a table of caller and access. A connection that presented no
//! key is nobody and is refused.

use crate::repo::RepoName;

/// Every route starts with this.
pub const PREFIX: &str = "/fleet-as-code/";

/// Where a machine's document lives inside the repository.
///
/// The label is the name a person uses (`ADR-025` §4), resolved in the
/// register by `rivet-fleetd`. **Nothing outside this directory is readable or
/// writable through these operations.**
pub const MACHINES_DIR: &str = "fleet/machines";

/// The suffix a machine's document carries.
pub const DOCUMENT_SUFFIX: &str = ".toml";

/// The role word a fleet server's own register record carries.
///
/// `cmd/rivet-provision` writes it into the one-record register a server's card
/// carries, and `rivet-fleetd` reads it to find its own control plane.
pub const SERVER_ROLE: &str = "fleet-server";

/// The largest machine document these operations carry, in bytes.
pub const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;

/// The largest commit message, in bytes.
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;

/// The largest request body, in bytes: a document, a message, and the lines
/// that declare them.
pub const MAX_REQUEST_BYTES: usize = MAX_DOCUMENT_BYTES + MAX_MESSAGE_BYTES + 4096;

/// Who a commit made here is attributed to, as author and committer.
///
/// The operator's identity is the pinned key the control plane saw, which is
/// recorded in the machine's journal; the commit names the program that
/// composed it.
pub const COMMITTER_NAME: &str = "rivet-fleetd";

/// The committer's address.
pub const COMMITTER_EMAIL: &str = "rivet-fleetd@localhost";

/// The longest a machine label may be, in bytes.
pub const MAX_LABEL_BYTES: usize = 200;

/// The longest a branch name may be, in bytes.
pub const MAX_BRANCH_BYTES: usize = 100;

// ---------------------------------------------------------------------------
// the operations and who may ask
// ---------------------------------------------------------------------------

/// One of the four operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// The commit a branch names, and its message.
    Head,
    /// The machine documents a commit holds.
    Machines,
    /// One machine document at a commit.
    File,
    /// One machine document committed onto a branch, compare-and-swap on its
    /// tip.
    Commit,
}

impl Operation {
    /// Every operation, in the order the module documentation lists them.
    pub const ALL: [Self; 4] = [Self::Head, Self::Machines, Self::File, Self::Commit];

    /// The route that reaches it.
    #[must_use]
    pub const fn route(self) -> &'static str {
        match self {
            Self::Head => "/fleet-as-code/head",
            Self::Machines => "/fleet-as-code/machines",
            Self::File => "/fleet-as-code/file",
            Self::Commit => "/fleet-as-code/commit",
        }
    }

    /// The operation a request path names, or `None`. A query string is not
    /// part of any route.
    #[must_use]
    pub fn from_route(path: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.route() == path)
    }

    /// What it does to the repository.
    #[must_use]
    pub const fn access(self) -> Access {
        match self {
            Self::Head | Self::Machines | Self::File => Access::Read,
            Self::Commit => Access::Write,
        }
    }
}

/// What a request does to the repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// A head, a listing or a document read.
    Read,
    /// A commit.
    Write,
}

/// Who is asking, as these operations see them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caller {
    /// A key in this machine's `peers`: the operator, or the fleet server's
    /// own operator identity.
    Operator,
}

/// **Who may do what.** The operator reads and writes; nobody else is a caller.
///
/// A fleet member that later fetches from its server over the control plane
/// would be a new [`Caller`], and this table grants it nothing until a row
/// says so.
#[must_use]
pub const fn permits(caller: Caller, access: Access) -> bool {
    match (caller, access) {
        (Caller::Operator, Access::Read | Access::Write) => true,
    }
}

// ---------------------------------------------------------------------------
// the values a request carries, each checked
// ---------------------------------------------------------------------------

/// The repository these operations work against, by name.
///
/// # Errors
///
/// A name that is not a repository name, or the `ADR-052` `fleet` repository.
/// That repository's `main` is the operator's workstation's, pushed with
/// `cargo rivet fleet push` and read by the repository store's prune; a commit
/// made here would be undone by the next push, so it is refused by name.
pub fn repository(name: &str) -> Result<RepoName, Refusal> {
    let Some(parsed) = RepoName::new(name) else {
        return Err(Refusal::new(
            Kind::BadRequest,
            format!(
                "{name:?} is not the name of a repository on this machine: {}. Fleet-as-code's \
                 repository is one this fleet server's own document declares under \
                 [repos.<name>], named here by that name and not by a URL or a path.",
                RepoName::RULE
            ),
        ));
    };
    if parsed.as_str() == crate::repository_objects::REPOSITORY {
        return Err(Refusal::new(
            Kind::BadRequest,
            format!(
                "the {name} repository holds the operator's fleet repository text, pushed from \
                 the workstation with `cargo rivet fleet push`, and a commit made here would be \
                 overwritten by the next push. Fleet-as-code's desired-state documents live in \
                 a repository of their own, declared under a different [repos.<name>]."
            ),
        ));
    }
    Ok(parsed)
}

/// A branch name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch(String);

impl Branch {
    /// The rule, in the words a refusal uses.
    pub const RULE: &'static str = "a branch here is 1 to 100 characters of ASCII letters, digits, \
         `.`, `_` and `-`, not beginning with `-` or `.`, not ending with `.` or `.lock`, with no \
         `..`, and not `HEAD`";

    /// Read a branch name, or refuse it.
    ///
    /// # Errors
    ///
    /// [`Kind::BadRequest`], naming the rule.
    pub fn new(text: &str) -> Result<Self, Refusal> {
        let allowed = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-');
        let ok = !text.is_empty()
            && text.len() <= MAX_BRANCH_BYTES
            && text.bytes().all(allowed)
            && !text.starts_with('-')
            && !text.starts_with('.')
            && !text.ends_with('.')
            && !text.ends_with(".lock")
            && !text.contains("..")
            && text != "HEAD";
        if ok {
            Ok(Self(text.to_owned()))
        } else {
            Err(Refusal::new(Kind::BadRequest, format!("{text:?} is not a branch: {}", Self::RULE)))
        }
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The full reference, `refs/heads/<name>`.
    #[must_use]
    pub fn reference(&self) -> String {
        format!("refs/heads/{}", self.0)
    }
}

/// A full commit id: 40 or 64 lowercase hexadecimal characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitId(String);

impl CommitId {
    /// Read a commit id, or refuse it.
    ///
    /// # Errors
    ///
    /// [`Kind::BadRequest`]. An abbreviation is refused, because git would
    /// resolve it against whatever the repository holds and two commits can
    /// share a prefix.
    pub fn new(text: &str) -> Result<Self, Refusal> {
        let hex = text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if hex && (text.len() == 40 || text.len() == 64) {
            Ok(Self(text.to_owned()))
        } else {
            Err(Refusal::new(
                Kind::BadRequest,
                format!("{text:?} is not a full commit id: 40 or 64 lowercase hex characters"),
            ))
        }
    }

    /// The id.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whether `label` is a machine label a document path may carry.
///
/// # Errors
///
/// [`Kind::BadRequest`], naming the rule.
pub fn check_label(label: &str) -> Result<(), Refusal> {
    let refuse = |why: &str| {
        Err(Refusal::new(
            Kind::BadRequest,
            format!("{label:?} is not a machine label a document may be named by: {why}"),
        ))
    };
    if label.is_empty() {
        return refuse("it is empty");
    }
    if label.len() > MAX_LABEL_BYTES {
        return refuse("it is longer than 200 bytes");
    }
    if label.contains('/') || label.contains('\\') {
        return refuse("it contains a path separator, and a machine's document is one file directly in fleet/machines/");
    }
    if label.chars().any(char::is_control) {
        return refuse("it contains a control character");
    }
    if label.starts_with('.') || label.starts_with('-') {
        return refuse("it begins with `.` or `-`");
    }
    Ok(())
}

/// Where one machine's document is: `fleet/machines/<label>.toml`.
#[must_use]
pub fn path_of(label: &str) -> String {
    format!("{MACHINES_DIR}/{label}{DOCUMENT_SUFFIX}")
}

/// A path inside the repository that names one machine's document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentPath {
    label: String,
}

impl DocumentPath {
    /// Read a path, or refuse it.
    ///
    /// **The only paths these operations touch are `fleet/machines/<label>.toml`**
    /// for a label [`check_label`] accepts. An absolute path, a `..`, a deeper
    /// directory and a file elsewhere in the repository are all refused here,
    /// before git is given anything.
    ///
    /// # Errors
    ///
    /// [`Kind::BadRequest`], naming the path and the rule.
    pub fn new(path: &str) -> Result<Self, Refusal> {
        let outside = || {
            Refusal::new(
                Kind::BadRequest,
                format!(
                    "{path:?} is not a machine document: these operations read and write only \
                     {MACHINES_DIR}/<label>{DOCUMENT_SUFFIX}, relative to the repository's root"
                ),
            )
        };
        let rest = path
            .strip_prefix(MACHINES_DIR)
            .and_then(|rest| rest.strip_prefix('/'))
            .ok_or_else(outside)?;
        let label = rest.strip_suffix(DOCUMENT_SUFFIX).ok_or_else(outside)?;
        check_label(label).map_err(|refusal| {
            Refusal::new(Kind::BadRequest, format!("{path:?} is not a machine document: {}", refusal.why))
        })?;
        Ok(Self { label: label.to_owned() })
    }

    /// The path, relative to the repository's root.
    #[must_use]
    pub fn as_path(&self) -> String {
        path_of(&self.label)
    }

    /// The machine label the file is named by.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
}

// ---------------------------------------------------------------------------
// refusals
// ---------------------------------------------------------------------------

/// Which refusal an operation answered with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// No pinned key, or a caller [`permits`] does not name.
    Forbidden,
    /// A request this module refuses to read: an unknown line, a missing
    /// value, a path outside `fleet/machines/`, a name that is not one.
    BadRequest,
    /// A route that is not one of the four, or a method other than `POST`.
    NotAllowed,
    /// The repository is absent from this machine.
    NoRepository,
    /// The branch is absent from the repository.
    NoBranch,
    /// The commit is absent from the repository.
    NoCommit,
    /// **The branch's tip is not the head the caller expected.** Nothing was
    /// committed.
    StaleHead,
    /// The document is already what the branch's tip holds.
    NothingToCommit,
    /// The path is not a regular file, or its contents are not UTF-8 text.
    NotADocument,
    /// A body, a document or a message larger than this module's limits.
    TooLarge,
    /// A chunked body, which these routes do not decode.
    LengthRequired,
    /// git failed, carrying its own words.
    Failed,
}

impl Kind {
    /// Every kind.
    pub const ALL: [Self; 12] = [
        Self::Forbidden,
        Self::BadRequest,
        Self::NotAllowed,
        Self::NoRepository,
        Self::NoBranch,
        Self::NoCommit,
        Self::StaleHead,
        Self::NothingToCommit,
        Self::NotADocument,
        Self::TooLarge,
        Self::LengthRequired,
        Self::Failed,
    ];

    /// The word on the wire.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Forbidden => "forbidden",
            Self::BadRequest => "bad-request",
            Self::NotAllowed => "not-allowed",
            Self::NoRepository => "no-repository",
            Self::NoBranch => "no-branch",
            Self::NoCommit => "no-commit",
            Self::StaleHead => "stale-head",
            Self::NothingToCommit => "nothing-to-commit",
            Self::NotADocument => "not-a-document",
            Self::TooLarge => "too-large",
            Self::LengthRequired => "length-required",
            Self::Failed => "failed",
        }
    }

    /// The HTTP status and reason phrase.
    #[must_use]
    pub const fn status(self) -> (u16, &'static str) {
        match self {
            Self::Forbidden => (403, "Forbidden"),
            Self::BadRequest => (400, "Bad Request"),
            Self::NotAllowed => (405, "Method Not Allowed"),
            Self::NoRepository | Self::NoBranch | Self::NoCommit => (404, "Not Found"),
            Self::StaleHead => (409, "Conflict"),
            Self::NothingToCommit | Self::NotADocument => (422, "Unprocessable Content"),
            Self::TooLarge => (413, "Content Too Large"),
            Self::LengthRequired => (411, "Length Required"),
            Self::Failed => (500, "Internal Server Error"),
        }
    }

    fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.word() == word)
    }
}

/// An operation's refusal: which one, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub kind: Kind,
    pub why: String,
}

/// The first word of a refusal's body.
const REFUSED: &str = "refused";

impl Refusal {
    /// A refusal of `kind`, saying `why`.
    #[must_use]
    pub fn new(kind: Kind, why: impl Into<String>) -> Self {
        Self { kind, why: why.into() }
    }

    /// The body on the wire: `refused <word>`, then the sentence.
    #[must_use]
    pub fn render(&self) -> String {
        format!("{REFUSED} {}\n{}\n", self.kind.word(), self.why.trim_end())
    }

    /// Read a refusal's body back, or `None` when it is not one.
    #[must_use]
    pub fn parse(body: &str) -> Option<Self> {
        let (first, rest) = body.split_once('\n').unwrap_or((body, ""));
        let word = first.trim_end_matches('\r').strip_prefix(REFUSED)?.strip_prefix(' ')?;
        let kind = Kind::from_word(word)?;
        Some(Self::new(kind, rest.trim_end()))
    }
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} ({})", self.why.trim_end(), self.kind.word())
    }
}

// ---------------------------------------------------------------------------
// requests
// ---------------------------------------------------------------------------

/// `POST /fleet-as-code/head`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadRequest {
    pub repository: RepoName,
    pub branch: Branch,
}

/// `POST /fleet-as-code/machines`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachinesRequest {
    pub repository: RepoName,
    pub commit: CommitId,
}

/// `POST /fleet-as-code/file`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRequest {
    pub repository: RepoName,
    pub commit: CommitId,
    pub path: DocumentPath,
}

/// `POST /fleet-as-code/commit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRequest {
    pub repository: RepoName,
    pub branch: Branch,
    /// The tip the caller read. The commit is made only if the branch still
    /// names it.
    pub expected_head: CommitId,
    pub path: DocumentPath,
    pub message: String,
    pub document: String,
}

/// One request, of any operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Head(HeadRequest),
    Machines(MachinesRequest),
    File(FileRequest),
    Commit(CommitRequest),
}

/// The line words a request carries.
const REPOSITORY: &str = "repository";
const BRANCH: &str = "branch";
const COMMIT: &str = "commit";
const EXPECTED_HEAD: &str = "expected-head";
const PATH: &str = "path";
const MESSAGE_BYTES: &str = "message-bytes";
const DOCUMENT_BYTES: &str = "document-bytes";

impl Request {
    /// The operation this request is for.
    #[must_use]
    pub const fn operation(&self) -> Operation {
        match self {
            Self::Head(_) => Operation::Head,
            Self::Machines(_) => Operation::Machines,
            Self::File(_) => Operation::File,
            Self::Commit(_) => Operation::Commit,
        }
    }

    /// The body on the wire.
    #[must_use]
    pub fn render(&self) -> Vec<u8> {
        let line = |word: &str, value: &str| format!("{word} {value}\n");
        let text = match self {
            Self::Head(r) => line(REPOSITORY, r.repository.as_str()) + &line(BRANCH, r.branch.as_str()),
            Self::Machines(r) => line(REPOSITORY, r.repository.as_str()) + &line(COMMIT, r.commit.as_str()),
            Self::File(r) => {
                line(REPOSITORY, r.repository.as_str())
                    + &line(COMMIT, r.commit.as_str())
                    + &line(PATH, &r.path.as_path())
            }
            Self::Commit(r) => {
                line(REPOSITORY, r.repository.as_str())
                    + &line(BRANCH, r.branch.as_str())
                    + &line(EXPECTED_HEAD, r.expected_head.as_str())
                    + &line(PATH, &r.path.as_path())
                    + &line(MESSAGE_BYTES, &r.message.len().to_string())
                    + &line(DOCUMENT_BYTES, &r.document.len().to_string())
                    + "\n"
                    + &r.message
                    + &r.document
            }
        };
        text.into_bytes()
    }

    /// Read a request's body for `operation`.
    ///
    /// # Errors
    ///
    /// [`Kind::BadRequest`] for a line this build does not read, a word given
    /// twice, a value missing or refused by its own rule, or bytes that are not
    /// UTF-8; [`Kind::TooLarge`] past a limit. **The whole request is refused**
    /// rather than read around.
    pub fn parse(operation: Operation, body: &[u8]) -> Result<Self, Refusal> {
        if body.len() > MAX_REQUEST_BYTES {
            return Err(Refusal::new(
                Kind::TooLarge,
                format!("a request of {} bytes is past the {MAX_REQUEST_BYTES}-byte limit", body.len()),
            ));
        }
        let text = core::str::from_utf8(body)
            .map_err(|_| Refusal::new(Kind::BadRequest, "a request body is UTF-8 text, and this one is not"))?;

        let (head, payload) = match text.split_once("\n\n") {
            Some((head, payload)) => (head, Some(payload)),
            None => (text, None),
        };
        let mut words: Vec<(&str, &str)> = Vec::new();
        for raw in head.lines() {
            let line = raw.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            let (word, value) = line.split_once(' ').ok_or_else(|| {
                Refusal::new(Kind::BadRequest, format!("{line:?} is not a `word value` line"))
            })?;
            if words.iter().any(|(seen, _)| *seen == word) {
                return Err(Refusal::new(Kind::BadRequest, format!("`{word}` is given twice")));
            }
            words.push((word, value));
        }

        let allowed: &[&str] = match operation {
            Operation::Head => &[REPOSITORY, BRANCH],
            Operation::Machines => &[REPOSITORY, COMMIT],
            Operation::File => &[REPOSITORY, COMMIT, PATH],
            Operation::Commit => &[REPOSITORY, BRANCH, EXPECTED_HEAD, PATH, MESSAGE_BYTES, DOCUMENT_BYTES],
        };
        if let Some((word, _)) = words.iter().find(|(word, _)| !allowed.contains(word)) {
            return Err(Refusal::new(
                Kind::BadRequest,
                format!("`{word}` is not a line {} reads", operation.route()),
            ));
        }
        let get = |word: &str| {
            words.iter().find(|(seen, _)| *seen == word).map(|(_, value)| *value).ok_or_else(|| {
                Refusal::new(Kind::BadRequest, format!("{} needs a `{word}` line", operation.route()))
            })
        };
        if operation != Operation::Commit && payload.is_some_and(|p| !p.trim().is_empty()) {
            return Err(Refusal::new(
                Kind::BadRequest,
                format!("{} carries no payload after its lines", operation.route()),
            ));
        }

        let repository = repository(get(REPOSITORY)?)?;
        Ok(match operation {
            Operation::Head => Self::Head(HeadRequest { repository, branch: Branch::new(get(BRANCH)?)? }),
            Operation::Machines => {
                Self::Machines(MachinesRequest { repository, commit: CommitId::new(get(COMMIT)?)? })
            }
            Operation::File => Self::File(FileRequest {
                repository,
                commit: CommitId::new(get(COMMIT)?)?,
                path: DocumentPath::new(get(PATH)?)?,
            }),
            Operation::Commit => {
                let branch = Branch::new(get(BRANCH)?)?;
                let expected_head = CommitId::new(get(EXPECTED_HEAD)?)?;
                let path = DocumentPath::new(get(PATH)?)?;
                let size = |word: &str, limit: usize| -> Result<usize, Refusal> {
                    let value = get(word)?;
                    let n = value.parse::<usize>().map_err(|_| {
                        Refusal::new(Kind::BadRequest, format!("`{word} {value}` is not a number of bytes"))
                    })?;
                    if n > limit {
                        return Err(Refusal::new(
                            Kind::TooLarge,
                            format!("`{word} {n}` is past the {limit}-byte limit"),
                        ));
                    }
                    Ok(n)
                };
                let message_bytes = size(MESSAGE_BYTES, MAX_MESSAGE_BYTES)?;
                let document_bytes = size(DOCUMENT_BYTES, MAX_DOCUMENT_BYTES)?;
                let payload = payload.ok_or_else(|| {
                    Refusal::new(Kind::BadRequest, "a commit's lines end with a blank line, then the message and the document")
                })?;
                if payload.len() != message_bytes + document_bytes {
                    return Err(Refusal::new(
                        Kind::BadRequest,
                        format!(
                            "the lines declare {message_bytes} bytes of message and {document_bytes} of \
                             document, and {} bytes follow them",
                            payload.len()
                        ),
                    ));
                }
                let (message, document) = payload
                    .split_at_checked(message_bytes)
                    .ok_or_else(|| Refusal::new(Kind::BadRequest, "the message's length splits a character"))?;
                if message.trim().is_empty() {
                    return Err(Refusal::new(Kind::BadRequest, "a commit needs a message"));
                }
                Self::Commit(CommitRequest {
                    repository,
                    branch,
                    expected_head,
                    path,
                    message: message.to_owned(),
                    document: document.to_owned(),
                })
            }
        })
    }
}

// ---------------------------------------------------------------------------
// answers
// ---------------------------------------------------------------------------

/// The answer to [`Operation::Head`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub commit: CommitId,
    /// The commit's whole message, trailers included.
    pub message: String,
}

impl Head {
    /// `commit <id>`, then the message.
    #[must_use]
    pub fn render(&self) -> String {
        format!("{COMMIT} {}\n{}", self.commit.as_str(), self.message)
    }

    /// Read it back.
    ///
    /// # Errors
    ///
    /// A sentence naming what did not read.
    pub fn parse(body: &str) -> Result<Self, String> {
        let (first, message) = body.split_once('\n').unwrap_or((body, ""));
        let id = first
            .strip_prefix(COMMIT)
            .and_then(|rest| rest.strip_prefix(' '))
            .ok_or_else(|| format!("a head answer opens with `commit <id>`, and this one opens {first:?}"))?;
        let commit = CommitId::new(id).map_err(|refusal| refusal.why)?;
        Ok(Self { commit, message: message.to_owned() })
    }
}

/// The answer to [`Operation::Machines`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Machines {
    /// Every label named by a `fleet/machines/<label>.toml` file, sorted as git
    /// lists a tree.
    pub labels: Vec<String>,
    /// Files in `fleet/machines/` ending in `.toml` whose name is not a label
    /// [`check_label`] accepts, each written as Rust's debug form of the name.
    pub ignored: Vec<String>,
}

const MACHINE: &str = "machine";
const IGNORED: &str = "ignored";

impl Machines {
    /// `machine <label>` and `ignored <name>` lines.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        for label in &self.labels {
            out.push_str(&format!("{MACHINE} {label}\n"));
        }
        for name in &self.ignored {
            out.push_str(&format!("{IGNORED} {name}\n"));
        }
        out
    }

    /// Read it back.
    ///
    /// # Errors
    ///
    /// A line this build does not read, or a label that is not one.
    pub fn parse(body: &str) -> Result<Self, String> {
        let mut out = Self::default();
        for line in body.lines() {
            if line.is_empty() {
                continue;
            }
            match line.split_once(' ') {
                Some((MACHINE, label)) => {
                    check_label(label).map_err(|refusal| refusal.why)?;
                    out.labels.push(label.to_owned());
                }
                Some((IGNORED, name)) => out.ignored.push(name.to_owned()),
                _ => return Err(format!("{line:?} is not a line a machines answer carries")),
            }
        }
        Ok(out)
    }
}

/// The answer to [`Operation::File`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum File {
    /// The commit holds the document, and this is its text.
    Present(String),
    /// The commit holds no file at that path. An ordinary answer: a machine may be added
    /// to the repository after the commit a baseline names, or removed from it.
    Absent,
}

const PRESENT: &str = "present";
const ABSENT: &str = "absent";

impl File {
    /// `present <bytes>` then the text, or `absent`.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Present(text) => format!("{PRESENT} {}\n{text}", text.len()),
            Self::Absent => format!("{ABSENT}\n"),
        }
    }

    /// Read it back, checking the declared length against the text.
    ///
    /// # Errors
    ///
    /// A first line that is neither, or a length that disagrees with what
    /// arrived.
    pub fn parse(body: &str) -> Result<Self, String> {
        let (first, text) = body.split_once('\n').unwrap_or((body, ""));
        if first == ABSENT {
            return Ok(Self::Absent);
        }
        let declared = first
            .strip_prefix(PRESENT)
            .and_then(|rest| rest.strip_prefix(' '))
            .and_then(|n| n.parse::<usize>().ok())
            .ok_or_else(|| format!("a file answer opens with `present <bytes>` or `absent`, and this one opens {first:?}"))?;
        if declared != text.len() {
            return Err(format!(
                "the answer declares a document of {declared} bytes and {} arrived, so nothing is read from it",
                text.len()
            ));
        }
        Ok(Self::Present(text.to_owned()))
    }
}

/// The answer to [`Operation::Commit`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Committed {
    /// The new tip of the branch.
    pub commit: CommitId,
}

impl Committed {
    /// `commit <id>`.
    #[must_use]
    pub fn render(&self) -> String {
        format!("{COMMIT} {}\n", self.commit.as_str())
    }

    /// Read it back.
    ///
    /// # Errors
    ///
    /// A sentence naming what did not read.
    pub fn parse(body: &str) -> Result<Self, String> {
        let first = body.lines().next().unwrap_or_default();
        let id = first
            .strip_prefix(COMMIT)
            .and_then(|rest| rest.strip_prefix(' '))
            .ok_or_else(|| format!("a commit answer is `commit <id>`, and this one is {first:?}"))?;
        Ok(Self { commit: CommitId::new(id).map_err(|refusal| refusal.why)? })
    }
}

#[cfg(test)]
#[path = "fleet_as_code_tests.rs"]
mod tests;
