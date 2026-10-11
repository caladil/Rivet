//! **The bulk endpoint — `ADR-035` §3.2, phase 2.**
//!
//! > A system update is **683,631,164 bytes** and the board has **926,680 kB**
//! > of RAM. JSON-RPC means base64 in a string — roughly a third larger, and
//! > buffered.
//! >
//! > So the control plane needs two endpoints on one listener: **MCP** for
//! > operations … and **a bulk endpoint** — an HTTP `PUT`/`GET` with a content
//! > digest — for payloads.
//!
//! `crate::dispatch` is the other half and arrived in phase 1 so this would be
//! an arm to fill rather than a restructure.
//!
//! # The shape, which `ADR-035` §3.2 left to whoever built it
//!
//! ```text
//! PUT /payload/<file-name> HTTP/1.1
//! Rivet-Blake3: <64 hex characters>
//! Content-Length: <n>
//!
//! <n bytes>
//! ```
//!
//! **The digest is a header and not the path**, which is the one place this
//! departs from the obvious content-addressed design. `ADR-027` §3.2.1 makes a
//! payload's *name* a claim about the commit that built it, and
//! `cmd/rivet-deploy` reports that name; a content-addressed path would throw
//! it away and leave the installer describing an update by hash. So the name
//! is the path, and the digest travels beside it.
//!
//! # The bytes land in the artifact store — 2026-09-12, `ADR-052` §7
//!
//! **Until that day they landed in `/state` beside everything else**, and
//! `rivet apply install <path> confirm` then named the file. That made this
//! endpoint and the install operation two halves of a general *"put a file
//! anywhere the installer can read, then install that path"* mechanism — and
//! the path half is what `ADR-052` removed, because it meant an artifact could
//! reach a machine without ever having been in the fleet repository.
//!
//! So this endpoint writes into `rivet_core::artifact`'s store, keyed by the
//! digest it is already computing. **Nothing about the transfer changed**: same
//! request shape, same streaming, same refusal on a digest mismatch, and the
//! `ADR-027` name is still kept — it is the name of the one file inside the
//! object's directory. What changed is that the endpoint now produces the one
//! thing an install can name.
//!
//! **This is the sole place where the two halves meet.** The
//! digest is stated by the sender, verified against the body as it arrives, and
//! then used as the key — so a caller cannot store bytes under a name that is
//! not their hash, and an install naming a digest gets the bytes whose hash it
//! is.
//!
//! **A `PUT` with no digest is refused.** Not defaulted, not warned about: a
//! payload nobody checked is one this machine has no reason to write to a card
//! it boots from. This digest is about the *transfer*, which is the part a
//! signature does not cover; the signature is [`Door`]'s question, below.
//!
//! # Nothing enters the store unverified — the door, 2026-09-12
//!
//! **The maintainer's ruling:** however the check is built, the store holds
//! only packages that have passed it (.rulings/ R-308). Until that day this
//! endpoint named an artifact the moment its digest matched, and only the
//! installer, later, checked its signature. So a refused package stayed in the store, and one carrying a high
//! version held one of its product's three retention slots for as long as
//! nothing higher arrived — because retention only ever evicts to make room.
//!
//! **Now a transfer passes two gates, in this order, before it is renamed into
//! `objects/`:**
//!
//! 1. **The digest** — the body's BLAKE3 against the header, as it arrives. A
//!    corrupted or truncated transfer fails here and **no signature work is
//!    done**: the refusal is about the wire, and saying so is what stops an
//!    operator being sent to look at a key that is fine.
//! 2. **The door** — [`TrustStoreDoor`], which asks
//!    `rivet_core::package::verify::artifact` of the complete file: the header
//!    signed by a key this machine's trust store holds, that key entitled to sign
//!    this kind, and every member's bytes the ones the signature covers — then
//!    `verify::architecture`, that the artifact was built for the architecture
//!    this machine's image records, and `verify::not_an_image_service`, that an
//!    application does not claim a name this machine's image already defines
//!    (`PLAN-Feature-Set-2026-09-12` §3). Those are the same functions
//!    `cmd/rivet-deploy` calls — one implementation consulted at two points,
//!    which is what `ADR-029` §11 asks for and not what it forbids (`ADR-052` §7
//!    argues it).
//!
//! An artifact the door refuses is removed from `incoming/` and never has a
//! name in `objects/`, so it never occupies a slot and retention needs no case
//! for it. The answer is a **422** that says the transfer was intact and the
//! *artifact* was refused, so a client can report exactly that rather than a
//! transfer failure.
//!
//! **What the door costs on a 700 MB system update, stated rather than
//! discovered:** one more read of the bytes just written, and one more BLAKE3
//! pass over them, before the `201` — the body is held in
//! `incoming/<digest>/` while it happens, where nothing that resolves a digest
//! looks, and nothing is held in memory beyond one 64 KiB buffer and the 4 KiB
//! header. It is not measured on a board. It is done after the transfer rather
//! than folded into it because member boundaries live in the header, and
//! reading them before the signature has checked out would be a second parser
//! upstream of the verifier (`ADR-029` §5).
//!
//! # Streaming, and what would happen if it were not
//!
//! The body is read in chunks, hashed as it arrives, and written as it arrives.
//! Nothing here ever holds the payload: at 683 MB on a 926 MB board, buffering
//! it would mean the OOM killer choosing between this daemon and the supervisor.
//! `PLAN.md` §57 watched exactly that happen to `git` on this hardware.
//!
//! # Where the bytes land, and why not where the caller says
//!
//! `/state/artifacts/objects/<digest>/<file-name>` — a place this machine
//! computes, not a path the caller chooses. A `PUT` that could name any path is
//! a remote filesystem with a digest, and `rivet-remote`'s own `put_document`
//! already carries the argument for refusing that shape: *"the one thing a
//! fleet tool has business replacing wholesale is the document it composed"*.
//! Here it is narrower still, and narrower than it was: the caller now chooses
//! only the **name**, and where the bytes go is decided by their own hash.

use std::collections::HashSet;
use std::io::Write as _;
use std::sync::{Condvar, Mutex, OnceLock};

use rivet_core::artifact;

use rivet_core::package::port::Digest as _;
use rivet_core::package::verify;

use crate::beneath::{self, Dir};

/// The path prefix a payload `PUT` uses.
pub const PAYLOAD_PREFIX: &str = "/payload/";

/// The header carrying the sender's BLAKE3 of the body.
pub const DIGEST_HEADER: &str = "rivet-blake3";


/// The largest body this will accept, in bytes.
///
/// **Larger than any payload this project builds, and finite.** A system update
/// is 683,631,164 bytes; this is a little over 1 GiB, so a legitimate payload
/// fits and a caller cannot fill `/state` by streaming for ever. `/state` on the
/// smallest supported card is 232 MiB (`PLAN.md` §13.4), so the filesystem will
/// usually refuse first — and that refusal is reported rather than turned into
/// a silent truncation.
pub const MAX_BYTES: u64 = 1_200_000_000;

/// What a request line and headers said, once read.
#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub target: String,
    pub digest: Option<String>,
    pub length: Option<u64>,
    /// Read for `crate::git`, which must hand it to a CGI, and unread here —
    /// the payload endpoint takes what it is given and checks the DIGEST.
    pub content_type: Option<String>,
    /// `Content-Encoding`, read for `crate::git` alone, which must tell
    /// `git-http-backend` a body is gzipped — `git` compresses a fetch's
    /// negotiation once it passes 1 KiB, and a compressed body handed on as
    /// though it were plain reaches `upload-pack` as `bad line length
    /// character` (found 2026-09-14 by `tools/qemu/adr-035-git-proof.sh`).
    pub content_encoding: Option<String>,
    /// Whether the body is chunked.
    ///
    /// **Recorded so it can be REFUSED, not so it can be decoded.** Nothing
    /// here implements `Transfer-Encoding: chunked`; `git` switches to it for a
    /// request body over `http.postBuffer` (1 MB by default), and a chunked
    /// body handed on unread would reach `git-http-backend` with its length
    /// prefixes still in it — a corrupt push rather than an error
    /// (`PLAN.md` §88.11).
    pub chunked: bool,
}

/// A response, ready to write.
#[derive(Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub reason: &'static str,
    pub body: String,
}

impl Response {
    /// The bytes on the wire.
    #[must_use]
    pub fn render(&self) -> String {
        format!(
            "HTTP/1.1 {} {}\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {}",
            self.status,
            self.reason,
            self.body.len(),
            self.body
        )
    }

    fn refuse(status: u16, reason: &'static str, body: impl Into<String>) -> Self {
        Self { status, reason, body: body.into() }
    }
}

/// Parse a request line and headers out of what has been read so far.
///
/// `None` means the head is not complete yet and more should be read. `Some`
/// carries the request and **the byte offset the body starts at**, because the
/// first read almost always contains some of it.
///
/// # Errors
///
/// If the request line is malformed, or a header this endpoint acts on cannot
/// be read.
pub fn parse_head(buffer: &[u8]) -> Result<Option<(Request, usize)>, Response> {
    let Some(end) = find_head_end(buffer) else {
        return Ok(None);
    };
    let head = String::from_utf8_lossy(&buffer[..end]);
    let mut lines = head.split("\r\n");

    let first = lines.next().unwrap_or_default();
    let mut parts = first.split(' ');
    let (Some(method), Some(target), Some(version)) =
        (parts.next(), parts.next(), parts.next())
    else {
        return Err(Response::refuse(
            400,
            "Bad Request",
            format!("{first:?} is not a request line — expected `METHOD target HTTP/1.1`\n"),
        ));
    };
    if !version.starts_with("HTTP/1.") {
        return Err(Response::refuse(
            505,
            "HTTP Version Not Supported",
            format!("this endpoint speaks HTTP/1.1 and the request said {version:?}\n"),
        ));
    }

    let mut digest = None;
    let mut length = None;
    let mut content_type = None;
    let mut content_encoding = None;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let (name, value) = (name.trim().to_ascii_lowercase(), value.trim());
        if name == DIGEST_HEADER {
            digest = Some(value.to_ascii_lowercase());
        } else if name == "content-type" {
            content_type = Some(value.to_string());
        } else if name == "content-encoding" {
            content_encoding = Some(value.to_string());
        } else if name == "transfer-encoding" {
            chunked = value.to_ascii_lowercase().contains("chunked");
        } else if name == "content-length" {
            let Ok(n) = value.parse::<u64>() else {
                return Err(Response::refuse(
                    400,
                    "Bad Request",
                    format!("Content-Length: {value:?} is not a number of bytes\n"),
                ));
            };
            // **A second Content-Length is refused whether or not it agrees
            // with the first.** Two fields that disagree are the classic
            // smuggling shape — this endpoint and whatever sits in front of
            // it could honour different ones and disagree about where the
            // body ends. Two that happen to agree are refused too, rather
            // than trusted because they matched: nothing downstream of this
            // parser should ever have to decide which of two headers with
            // the same name was the real one.
            if let Some(already) = length {
                return Err(Response::refuse(
                    400,
                    "Bad Request",
                    format!(
                        "two Content-Length headers, {already} and {n}: this request has no \
                         one length, refused before a byte of body is read rather than \
                         guessed at.\n"
                    ),
                ));
            }
            length = Some(n);
        }
    }

    // **Both framings at once is the same shape rivet-proxyd refuses**, and
    // for the same reason: `Transfer-Encoding` and `Content-Length` are two
    // disagreeing claims about where the body ends, and which one a
    // component downstream honours is exactly what a smuggled request hides
    // behind. Refused here, before a byte of body is read, rather than left
    // for the digest check further on to catch as a corrupted transfer.
    if chunked && length.is_some() {
        return Err(Response::refuse(
            400,
            "Bad Request",
            "both Transfer-Encoding and Content-Length: two framings is no framing.\n"
                .to_string(),
        ));
    }

    Ok(Some((
        Request {
            method: method.to_ascii_uppercase(),
            target: target.to_string(),
            digest,
            length,
            chunked,
            content_type,
            content_encoding,
        },
        end + 4,
    )))
}

/// Where the blank line that ends the head is.
fn find_head_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Check everything about a request that can be checked before a byte of body
/// is read.
///
/// **Every refusal here happens before anything is written**, which is the
/// point: a `PUT` that was always going to be refused should not first spend a
/// board's disk and a caller's minutes streaming 683 MB.
///
/// # Errors
///
/// A [`Response`] to send instead of accepting the body.
pub fn accept(request: &Request) -> Result<(String, [u8; 32], u64), Response> {
    if request.method != "PUT" {
        return Err(Response::refuse(
            405,
            "Method Not Allowed",
            format!(
                "this endpoint takes PUT and the request was {}. ADR-035 §3.2's GET — reading a \
                 payload back — is not built. §4's smart HTTP for git IS served on this same \
                 listener, and only under a repository's own paths: \
                 /<repository>.git/info/refs, /git-upload-pack and /git-receive-pack.\n",
                request.method
            ),
        ));
    }
    accept_under(request, PAYLOAD_PREFIX)
}

/// [`accept`]'s checks after the method, for a `PUT` under `prefix` — the
/// machine's artifact store's `/payload/`, or a fleet server's
/// `rivet_core::repository_objects::PREFIX`. **One set of refusals for both**,
/// so an object sent to either is named, digested and bounded by the same rule.
///
/// # Errors
///
/// A [`Response`] to send instead of accepting the body.
pub fn accept_under(request: &Request, prefix: &str) -> Result<(String, [u8; 32], u64), Response> {
    let Some(name) = request.target.strip_prefix(prefix) else {
        return Err(Response::refuse(
            404,
            "Not Found",
            format!(
                "this endpoint serves {prefix}<file-name> and the request asked for {:?}\n",
                request.target
            ),
        ));
    };
    let name = safe_name(name)?;

    let Some(digest) = &request.digest else {
        return Err(Response::refuse(
            400,
            "Bad Request",
            format!(
                "no {DIGEST_HEADER} header. A payload nobody checked is one this machine has no \
                 reason to write, and the digest is refused rather than defaulted because a \
                 transfer that silently corrupted is exactly what it exists to catch. Send the \
                 BLAKE3 of the body as 64 hex characters.\n"
            ),
        ));
    };
    let expected = hex32(digest).ok_or_else(|| {
        Response::refuse(
            400,
            "Bad Request",
            format!("{DIGEST_HEADER}: {digest:?} is not 64 hex characters\n"),
        )
    })?;

    let Some(length) = request.length else {
        return Err(Response::refuse(
            411,
            "Length Required",
            "no Content-Length. This endpoint does not accept a chunked or unbounded body: it \
             writes to a card, and a write with no stated size is one nothing can refuse before \
             it fills the filesystem.\n"
                .to_string(),
        ));
    };
    if length > MAX_BYTES {
        return Err(Response::refuse(
            413,
            "Content Too Large",
            format!(
                "Content-Length is {length} and this endpoint accepts at most {MAX_BYTES}. The \
                 largest payload this project builds is a system update at about 684 MB.\n"
            ),
        ));
    }

    Ok((name, expected, length))
}

/// A file name that cannot escape the store it lands in — the machine's
/// `rivet_core::artifact` store, or a fleet server's repository store.
///
/// **A closed rule rather than a blocklist.** `..` is the obvious attack and is
/// not the only one; what this accepts is the shape `cargo rivet package`
/// actually produces, and anything else is refused whether or not it is
/// dangerous. `ADR-027` §7 fixes that shape, so nothing legitimate is excluded.
///
/// # It must end in `.rvt`, and that is not decoration
///
/// **It used to be the only thing standing between a `PUT` and any file on
/// `/state`.** Until 2026-09-12 this endpoint wrote into `/state` itself, so
/// every name accepted here was a file in the machine's state directory beside
/// the ones mechanisms own — and a name like `deploy-request`, which is letters
/// and a hyphen, passed every other clause of the rule.
///
/// `/state/deploy-request` is the **install channel** — `rivet-sysd` reads it
/// on its next convergence pass and starts an installer from what it names,
/// with the session that asked recorded in it (`ADR-028` §4.5).
/// `rivet apply edit` refuses to touch it by name, and
/// `rivet_core::operation::edit::reserved_paths` says exactly why: *"a request
/// written by hand would name whoever it liked"*. One door was locked and this
/// one was open — a PUT could write that file, forge the principal and skip
/// `rivet apply install` entirely. `/state/clock` and `/state/log` were
/// reachable the same way. Found in the audit of 2026-09-03 (`PLAN.md` §123,
/// register R-31).
///
/// **The store made that hazard structural rather than conventional**, and the
/// rule is kept anyway. A `PUT` now writes under
/// `/state/artifacts/objects/<digest>/`, a directory this machine names from
/// the body's own hash, so no accepted name can be a file any mechanism reads:
/// `deploy-request` there is `deploy-request` inside somebody's object
/// directory, which nothing looks at. The name rule survives because it is
/// still the thing that stops a path separator, and because a store full of
/// artifacts named however a caller liked would be a store whose retention
/// could not group them (`rivet_core::artifact::product_of`).
///
/// **Requiring the suffix is a closed rule and a blocklist is not**, which is
/// the whole argument of the paragraph above: a reserved-name list would have
/// to grow every time `/state` does, and the one file it forgot would be the
/// one that mattered. Every artifact this endpoint exists to receive is a
/// `.rvt` — applications and system updates alike, from one constant
/// (`rivet_core::package::kind::SUFFIX`) — so nothing legitimate is excluded.
///
/// The suffix must be a suffix **of something**: `.rvt` alone is a leading dot
/// and is already refused, and the length test says so rather than leaving it
/// to be inferred from another clause.
fn safe_name(name: &str) -> Result<String, Response> {
    let ok = !name.is_empty()
        && name.len() <= 128
        && name != "."
        && name != ".."
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        && !name.starts_with('.')
        // **And it must be a PAYLOAD** — see below.
        && name.len() > rivet_core::package::kind::SUFFIX.len()
        && name.ends_with(rivet_core::package::kind::SUFFIX);
    if ok {
        Ok(name.to_string())
    } else {
        Err(Response::refuse(
            400,
            "Bad Request",
            format!(
                "{name:?} is not a name this endpoint will write. It takes letters, digits, \
                 `.`, `-` and `_`, at most 128 of them, not beginning with a dot, and ending \
                 in `{}` — the shape `cargo rivet package` produces (ADR-027 §7). No path \
                 separators: a PUT that could name a directory would be a remote filesystem, \
                 and this writes into a store keyed by the body's own digest and nowhere \
                 else.\n",
                rivet_core::package::kind::SUFFIX
            ),
        ))
    }
}

/// 64 hex characters as 32 bytes.
fn hex32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Hex, for a refusal that has to show what it computed.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether a complete, digest-checked artifact may be in this machine's store.
///
/// **A port with one production implementation, [`TrustStoreDoor`]**, and it is
/// a port for the reason [`artifact::Store`] is a parameter: so a test can
/// assert what happens around the decision — that a corrupted transfer never
/// reaches it, and that its refusal leaves nothing behind — without composing a
/// signed artifact for every test about a socket.
pub trait Door {
    /// Judge the file at `path`, which holds `length` bytes whose digest has
    /// already matched.
    ///
    /// `Ok` carries one line for the answer, saying what was verified.
    ///
    /// # Errors
    ///
    /// A [`Refusal`], which says whose fault it is.
    fn admit(&self, path: &str, length: u64) -> Result<String, Refusal>;
}

/// Why the door did not admit an artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// **The artifact does not verify.** Sending it again is refused again, and
    /// a copy of it this store already holds is garbage by the ruling — so it
    /// is removed.
    Artifact(String),
    /// **This machine could not decide** — its trust store is missing, corrupt
    /// or empty, or the bytes could not be read back. Nothing about the
    /// artifact is known, so nothing already held is removed.
    Machine(String),
}

/// **The door: `rivet_core::package::verify::artifact`, against the trust store
/// in `/system`.**
///
/// # Where the trust store comes from
///
/// `rivet_core::package::trust_store::PATH` — `/system/trust`, which
/// `cmd/rivet-image` writes into the read-only system partition and
/// `cmd/rivet-deploy` reads for every install. This daemon runs as uid 0 with
/// no confinement (`cmd/rivet-sysd`'s definition argues why), so it can read
/// that file exactly as the installer does, and it already links
/// `rivetos-crypto` for the transfer digest — so the door needed no new
/// dependency and no new privilege.
///
/// **Read on every admission and never cached**, for `ADR-028` §5's reason:
/// revocation is an image update that replaces the file.
///
/// **No `--trusted-key` here**, and that is not an omission. The installer's
/// override exists for a card built `--no-trust` and is given on its command
/// line; the supervised install path never passes one, so an artifact that only
/// an override would verify is one the store could never lead to an install of.
/// A `--no-trust` machine therefore admits nothing, and says so.
///
/// # And the architecture, from the image — 2026-09-12
///
/// `PLAN-Feature-Set-2026-09-12` §3: an artifact this machine cannot execute
/// is garbage under the same ruling, so the door asks
/// `rivet_core::package::verify::architecture` too, against
/// `rivet_core::package::arch::PATH` — the token the image recorded, never the
/// compiler's, which says `arm` where `ADR-027` says `armv6`. Read on every
/// admission for the trust store's reason, and a machine whose record cannot be
/// read admits nothing, as the machine's fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustStoreDoor {
    trust_store: String,
    arch: String,
    purpose: Purpose,
}

/// **What the admitted artifact is for**, which decides the questions asked
/// after authenticity.
///
/// # Why a fleet server's store does not refuse another architecture
///
/// `.rulings/` R-184 D1's store holds what the fleet repository's manifest
/// names **for the fleet**, and a fleet can hold a Pi Zero (`armv6`) beside a
/// Pi 4 (`aarch64`). An object this server cannot execute is not garbage there:
/// it is what a member of another architecture will fetch (`ADR-052` §9 step 5),
/// and refusing it would leave a mixed fleet's manifest permanently unsyncable.
/// So the architecture is **recorded** in the answer, never refused, and the
/// member that installs it asks its own image's question at its own door.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// This machine's own artifact store: an artifact this machine would
    /// install — its architecture, and no image service's name.
    Install,
    /// A fleet server's repository store: any kind the manifest may name
    /// (`rivet_core::repository_objects::holds_kind`), of any architecture,
    /// and no image service's name.
    Mirror,
}

impl TrustStoreDoor {
    /// The door on this machine, reading its own trust store and its own
    /// architecture record.
    #[must_use]
    pub fn on_this_machine() -> Self {
        Self::at(
            rivet_core::package::trust_store::PATH,
            rivet_core::package::arch::PATH,
        )
    }

    /// The door of a fleet server's repository store on this machine — the same
    /// trust store and the same verifier, asked for [`Purpose::Mirror`].
    #[must_use]
    pub fn mirror_on_this_machine() -> Self {
        Self::on_this_machine().for_purpose(Purpose::Mirror)
    }

    /// A door reading the trust store and the architecture record at these
    /// paths. For a test.
    #[must_use]
    pub fn at(trust_store: impl Into<String>, arch: impl Into<String>) -> Self {
        Self {
            trust_store: trust_store.into(),
            arch: arch.into(),
            purpose: Purpose::Install,
        }
    }

    /// This door, asked for `purpose`.
    #[must_use]
    pub const fn for_purpose(mut self, purpose: Purpose) -> Self {
        self.purpose = purpose;
        self
    }
}

impl Door for TrustStoreDoor {
    fn admit(&self, path: &str, length: u64) -> Result<String, Refusal> {
        let mut digest = rivet_crypto::Blake3::new();
        let verifier = rivet_crypto::Ed25519Verifier::new();
        let trust = verify::trust_set(std::fs::read(&self.trust_store), &mut digest)
            .map_err(|why| Refusal::Machine(why.to_string()))?;
        // **A mirror does not need the record to decide**, so one that cannot be
        // read is said in the answer rather than refusing an artifact whose
        // architecture is not a gate there.
        let arch = match (rivet_core::package::arch::read(std::fs::read(&self.arch)), self.purpose) {
            (Ok(arch), _) => Some(arch),
            (Err(why), Purpose::Install) => return Err(Refusal::Machine(why.to_string())),
            (Err(_), Purpose::Mirror) => None,
        };

        let file = std::fs::File::open(path)
            .map_err(|e| Refusal::Machine(format!("cannot read {path} back to verify it: {e}")))?;
        let mut source = FileSource { file };
        let opened = verify::artifact(&mut source, length, &mut digest, &verifier, &trust)
            .map_err(|fault| match fault {
                verify::Fault::Artifact(why) => Refusal::Artifact(why.to_string()),
                verify::Fault::Medium { reading, error } => Refusal::Machine(format!(
                    "cannot read {} of {path} back to verify it: {error}",
                    match reading {
                        verify::Reading::Header => "the header".to_string(),
                        verify::Reading::Member(index) => format!("member {index}"),
                    }
                )),
            })?;
        let header = opened.package().header();
        // **After authenticity, never before**: a header nobody this machine
        // trusts signed says nothing worth comparing, and the refusal for it is
        // the signature's.
        // **A card is never installed on a machine** (`ADR-063` §6, `ADR-028`
        // §5): it is decompressed once, on the workstation that writes it. The
        // system key is trusted to sign cards so a fleet server's store can hold
        // them, and this machine's own store asks the kind as well as the trust,
        // so that entitlement admits no card here.
        if self.purpose == Purpose::Install && *header.kind() == rivet_core::package::kind::Kind::card() {
            return Err(Refusal::Artifact(
                "its signed header says its kind is `card`, and a card is written to a card on a \
                 workstation, never installed on a machine: this machine's own store holds what \
                 it installs. A fleet server's repository store holds cards (PUT \
                 /repository-objects/)."
                    .to_owned(),
            ));
        }
        let built_for = match (self.purpose, &arch) {
            (Purpose::Install, Some(arch)) => {
                verify::architecture(opened.package(), arch).map_err(|why| {
                    Refusal::Artifact(format!("{why} — this machine executes {arch:?}"))
                })?;
                String::new()
            }
            (Purpose::Install, None) => String::new(),
            (Purpose::Mirror, _) => {
                if !rivet_core::repository_objects::holds_kind(header.kind()) {
                    return Err(Refusal::Artifact(format!(
                        "its signed header says its kind is `{}`, which this build cannot open, \
                         and a fleet server's repository store holds only the kinds it can: \
                         applications, libraries, apks, system updates and cards (ADR-063 §6)",
                        header.kind().as_str()
                    )));
                }
                // **The kind's own reader, as the installer opens it** — a
                // structure that reader refuses is garbage on every member too.
                // The architecture is not asked of it: see [`Purpose`].
                mirror_structure(opened.package()).map_err(Refusal::Artifact)?;
                mirror_arch_line(header.arch(), arch.as_deref())
            }
        };
        // **The same door, the same reason** — `PLAN-Feature-Set-2026-09-12`
        // §3, `.rulings/` R-156 R10. An application whose name an
        // image service already holds is garbage under the maintainer's
        // no-garbage ruling exactly as a wrong architecture is: this machine
        // would refuse it every boot and never start it, and there is no
        // uninstall that can name the shadow once it exists.
        verify::not_an_image_service(opened.package())
            .map_err(|why| Refusal::Artifact(why.to_string()))?;

        // **The same door, for a name this project's own machinery already
        // means something else by** — `.rulings/` R-210 D21. An account, a
        // group, an uninstall keyword, or the deploy/supervisor/kernel-log/
        // ingest service name is not merely a shadow of a service; it is a
        // name `getpwnam` or a journal reader already resolves to something
        // else.
        verify::not_a_reserved_name(opened.package())
            .map_err(|why| Refusal::Artifact(why.to_string()))?;

        let label = trust
            .find(&header.key_id())
            .map_or("a key this machine trusts", |entry| entry.label());
        if self.purpose == Purpose::Mirror {
            return Ok(format!(
                "\x20 verified signed by {label:?}, which this machine's trust store permits to \
                 sign {kind}, and all {members} member(s) are the bytes that signature covers. \
                 Checked before the object was named (ADR-052 §7); a member's installer checks \
                 again.\n\
                 {built_for}",
                kind = header.kind().as_str(),
                members = opened.package().members().len(),
            ));
        }
        let arch = arch.unwrap_or_default();
        Ok(format!(
            "\x20 verified signed by {label:?}, which this machine's trust store permits to sign \
             {kind}, and all {members} member(s) are the bytes that signature covers; built for \
             {arch}, which this machine executes. Checked before the artifact was named (ADR-052 \
             §7); the installer checks again when it runs.\n",
            kind = header.kind().as_str(),
            members = opened.package().members().len(),
        ))
    }
}

/// Every kind a fleet server's store holds, opened by its kind's own reader.
///
/// **A system update is opened as a payload with no reserved names.** Which
/// boot-directory names a platform reserves is the installing machine's
/// answer (`ADR-029` §4), asked again at its own door; the mirror asks what
/// every machine would — one system member, every slot-variant file present
/// for both slots — and is not the machine the payload is for, as with the
/// architecture (see [`Purpose`]).
///
/// **A card has no reader beyond the container**: its signature and every
/// member's digest are what `verify::artifact` has already checked, and it is
/// decompressed only on the workstation that writes it. What is asked here is
/// that it carries the image at all.
fn mirror_structure(package: &rivet_core::package::read::Package) -> Result<(), String> {
    use rivet_core::package::kind::Kind;
    let kind = package.header().kind();
    let opened = if *kind == Kind::library() {
        rivet_core::package::library::Library::open(package).map(|_| ())
    } else if *kind == Kind::apk() {
        rivet_core::package::apk::Apk::open(package).map(|_| ())
    } else if *kind == Kind::update() {
        rivet_core::package::update::Payload::open(package, &[]).map(|_| ())
    } else if *kind == Kind::card() {
        if package.members().is_empty() {
            return Err("this card carries no member, so there is no image in it to write".to_owned());
        }
        Ok(())
    } else {
        rivet_core::package::application::Application::open(package).map(|_| ())
    };
    opened.map_err(|why| why.to_string())
}

/// The architecture line a mirror's answer carries — recorded, never refused
/// (see [`Purpose`]).
fn mirror_arch_line(built: &str, arch: Option<&str>) -> String {
    match arch {
        Some(arch) if arch == built => {
            format!("\x20 arch     built for {built}, which this machine executes.\n")
        }
        Some(arch) => format!(
            "\x20 arch     built for {built}; this machine executes {arch}. HELD ANYWAY: a \
             fleet server keeps what its members run, and a member of that architecture asks \
             its own image's question when it installs.\n"
        ),
        None => format!(
            "\x20 arch     built for {built}; this machine's architecture record could not be \
             read, and a fleet server's store does not decide on it.\n"
        ),
    }
}

/// A file, as `rivet_core::package::verify` reads an artifact.
struct FileSource {
    file: std::fs::File,
}

impl verify::Source for FileSource {
    type Error = std::io::Error;

    fn read_range(
        &mut self,
        offset: u64,
        len: u64,
        sink: &mut dyn FnMut(&[u8]) -> bool,
    ) -> std::io::Result<()> {
        use std::io::{Read as _, Seek as _, SeekFrom};
        self.file.seek(SeekFrom::Start(offset))?;
        let mut buffer = vec![0u8; 64 * 1024];
        let mut left = len;
        while left > 0 {
            let want = usize::try_from(left.min(64 * 1024)).unwrap_or(64 * 1024);
            let got = self.file.read(&mut buffer[..want])?;
            if got == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    format!("the file ended {left} byte(s) short of what the header says"),
                ));
            }
            left -= got as u64;
            if !sink(&buffer[..got]) {
                return Ok(());
            }
        }
        Ok(())
    }
}

/// The answer for an artifact the door did not admit.
///
/// **422 for the artifact's fault and 500 for the machine's**, and the
/// distinction is the one a client reports: *the machine refused this artifact*
/// is not *the transfer failed*, and a client that said the second would send
/// an operator to retry something that will be refused identically.
///
/// `held` is whether the refusal is about a copy this store already had, which
/// changes what was done with it.
fn refused_at_the_door(
    shelf: Shelf,
    refusal: Refusal,
    name: &str,
    digest: &artifact::Digest,
    held: bool,
) -> Response {
    if shelf == Shelf::Repository {
        return refused_for_the_repository(refusal, name, digest, held);
    }
    match refusal {
        Refusal::Artifact(why) => Response::refuse(
            422,
            "Unprocessable Content",
            format!(
                "REFUSED AT THE DOOR: {name} is not an artifact this machine would install, so \
                 it is NOT IN THIS MACHINE'S ARTIFACT STORE.\n\
                 \x20 why      {why}\n\
                 \x20 transfer intact — the bytes this machine holds are the ones whose digest \
                 you sent ({digest}). The refusal is about the ARTIFACT, not the wire: sending \
                 it again is refused again.\n\
                 {kept}\
                 \x20 rule     no unverified artifact enters the store (ADR-052 §7). The \
                 check is rivet_core::package::verify, the one the installer runs.\n",
                kept = if held {
                    "\x20 removed  this store already held that digest, from before artifacts were \
                     verified at the door or under a trust store since replaced. It could not \
                     have been installed, and it has been REMOVED so it no longer holds a \
                     retention slot.\n"
                } else {
                    "\x20 kept     nothing. It was never named in the store, so it holds no \
                     retention slot and cannot be installed.\n"
                },
            ),
        ),
        Refusal::Machine(why) => Response::refuse(
            500,
            "Internal Server Error",
            format!(
                "REFUSED: this machine could not verify {name}, and it admits nothing into its \
                 artifact store that it has not verified.\n\
                 \x20 why      {why}\n\
                 {kept}",
                kept = if held {
                    "\x20 kept     the copy already in the store is left where it is: nothing is \
                     known about the artifact, only about this machine.\n"
                } else {
                    "\x20 kept     nothing.\n"
                },
            ),
        ),
    }
}

/// [`refused_at_the_door`] for a fleet server's repository store.
fn refused_for_the_repository(
    refusal: Refusal,
    name: &str,
    digest: &artifact::Digest,
    held: bool,
) -> Response {
    let kept = if held {
        "\x20 removed  this store already held that digest, under a trust store since replaced \
         or from before this door; it has been REMOVED so no member is offered it.\n"
    } else {
        "\x20 kept     nothing. It was never named in the store.\n"
    };
    match refusal {
        Refusal::Artifact(why) => Response::refuse(
            422,
            "Unprocessable Content",
            format!(
                "REFUSED AT THE DOOR: {name} is not an object this fleet server keeps, so it is \
                 NOT IN ITS REPOSITORY STORE.\n\
                 \x20 why      {why}\n\
                 \x20 transfer intact — the bytes this machine holds are the ones whose digest \
                 you sent ({digest}). The refusal is about the ARTIFACT, not the wire: sending \
                 it again is refused again.\n\
                 {kept}\
                 \x20 rule     nothing unverified enters the store (ADR-052 §7, .rulings/ R-184 \
                 D1). The check is rivet_core::package::verify, the one an installer runs.\n",
            ),
        ),
        Refusal::Machine(why) => Response::refuse(
            500,
            "Internal Server Error",
            format!(
                "REFUSED: this machine could not verify {name}, and it admits nothing into a \
                 fleet server's repository store that it has not verified.\n\
                 \x20 why      {why}\n\
                 {}",
                if held {
                    "\x20 kept     the copy already in the store is left where it is.\n"
                } else {
                    "\x20 kept     nothing.\n"
                },
            ),
        ),
    }
}

/// **Which store a transfer lands in**, and so what its answer says and whether
/// retention runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shelf {
    /// `rivet_core::artifact`'s store: three newest of each product, and an
    /// install is what comes next.
    Artifacts,
    /// A fleet server's repository store, `rivet_core::repository_objects`:
    /// **no retention**, because what it keeps is what the fleet repository's
    /// manifest names and a prune decides that (`.rulings/` R-184 D1), never a
    /// count per product.
    Repository,
}

/// One store's root, paired with a digest inside it — what a [`Claim`] owns
/// exclusively while it lives.
type ClaimKey = (String, artifact::Digest);

/// The [`ClaimKey`]s currently owned by a [`Claim`], and the [`Condvar`] a
/// second [`Claim::take`] for the same key waits on.
///
/// **Keyed by the store's root as well as the digest.** [`receive_onto`] is
/// reached for two different stores — the machine's own artifact store and a
/// fleet server's repository store, see [`Shelf`] — and a caller that happens
/// to send the same bytes to both has no reason to make one wait on the
/// other: they do not share a directory.
fn in_flight() -> &'static (Mutex<HashSet<ClaimKey>>, Condvar) {
    static IN_FLIGHT: OnceLock<(Mutex<HashSet<ClaimKey>>, Condvar)> = OnceLock::new();
    IN_FLIGHT.get_or_init(|| (Mutex::new(HashSet::new()), Condvar::new()))
}

/// Exclusive ownership of one [`ClaimKey`], for as long as this value lives.
///
/// **Held across the whole of one [`receive_onto`] call** — its
/// receive, verify, publish and cleanup — so two uploads of the same digest
/// into the same store cannot share the mutable `incoming/<digest>/` that
/// call builds. [`Claim::take`] blocks while another [`Claim`] holds the same
/// key, and resumes once it is dropped: on an ordinary return, on an early
/// `return` partway through, or on a panic unwinding through it. A different
/// key — a different digest, or the same digest in a different store — never
/// waits at all.
///
/// The transaction that was waiting resumes into the same [`receive_onto`]
/// that always ran. That function already has a fast path for a digest that
/// turns out to be published — the block just below reads it — so this adds
/// mutual exclusion and nothing else: if the first transaction published,
/// this one takes that fast path and answers "already held"; if the first
/// failed, nothing was published and this one starts clean.
struct Claim {
    key: ClaimKey,
}

impl Claim {
    /// Blocks until `(root, digest)` is not held by another [`Claim`], then
    /// holds it.
    fn take(root: &str, digest: &artifact::Digest) -> Self {
        let key: ClaimKey = (root.to_string(), digest.clone());
        let (mutex, condvar) = in_flight();
        let mut held = mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while held.contains(&key) {
            held = condvar.wait(held).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        held.insert(key.clone());
        Self { key }
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        let (mutex, condvar) = in_flight();
        let mut held = mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        held.remove(&self.key);
        drop(held);
        // **`notify_all`, not `notify_one`.** This one `Condvar` serves every
        // key this daemon ever claims, so a waiter parked on some other
        // digest is on the same condition variable; waking only one risks
        // waking the wrong digest's waiter while this one's stays parked.
        // Every waiter re-checks its own key under the mutex regardless, so
        // waking all of them costs a handful of uncontended lock attempts,
        // never a wrong wakeup.
        condvar.notify_all();
    }
}

/// Receive a body of exactly `length` bytes, hashing and writing as it arrives.
///
/// `already` is whatever the head parse had over. `more` is called for the rest.
///
/// **It writes beside the target and renames**, so a transfer that dies leaves
/// no file the installer could pick up. `cmd/rivet-initd`'s `confirm` and
/// `rivet-store` use the same shape, and `PLAN.md` §78's provisioning made the
/// argument in the other direction: order is the whole of the safety.
///
/// # Errors
///
/// A [`Response`] describing what went wrong, with the partial file removed.
/// `store` is where the object lands. **A parameter and not a constant**, for
/// the reason `rivet_core::artifact::Store` gives at length: this function
/// creates directories and `prune` **removes** them, so a test that reached the
/// machine's own store would delete a board's artifacts. `crate::main` passes
/// `Store::on_this_machine()`; the tests pass a scratch directory, which is what
/// makes everything below this line reachable from a development host.
///
/// `door` decides whether the complete, digest-checked body may be named —
/// `crate::main` passes [`TrustStoreDoor::on_this_machine`]. **It is asked only
/// after the digest has matched**, and it is asked again of a copy the store
/// already holds, so that a `201` is never this machine vouching for bytes it has
/// not verified.
///
/// # Why nothing sweeps a store written before the door
///
/// A store filled by a build from before 2026-09-12 can hold an artifact nothing
/// verified, and none is swept out at start-up. **Decided rather than
/// forgotten**, for three reasons:
///
/// 1. **It is permanent code for a state that exists once.** The one real store
///    is `rivet-01`'s, and `PLAN-Feature-Set-2026-09-12` §3.3d records what it
///    holds: artifacts that were each deployed and then installed, which the
///    installer does not do for one that fails verification. A QEMU guest's
///    store is tmpfs and gone at power-off. §3.3a refused a compatibility shim
///    on the same grounds.
/// 2. **It costs a read and a hash of every object**, up to three 700 MB system
///    updates, on every start of the daemon that is a headless board's only way
///    in — or a marker saying it has run, which is state that can disagree with
///    the store.
/// 3. **It deletes, unattended**, and the moment a sweep is most likely to be
///    wrong is the moment a trust store is being replaced.
///
/// What happens instead costs nothing until it matters: a held object is judged
/// by the door **whenever it is sent again**, and removed if the artifact itself
/// fails (never for the machine's fault). An object nobody sends again is one
/// nobody installs — and if somebody asks to, the installer refuses it.
pub fn receive(
    store: &artifact::Store,
    door: &dyn Door,
    name: &str,
    expected: [u8; 32],
    length: u64,
    already: &[u8],
    more: &mut dyn FnMut(&mut [u8]) -> std::io::Result<usize>,
) -> Result<Response, Response> {
    receive_onto(store, door, Shelf::Artifacts, name, expected, length, already, more)
}

/// [`receive`], onto `shelf` — the same streaming, digest, door and
/// directory-rename for a fleet server's repository store, with its own answer
/// and no retention.
///
/// # Errors
///
/// As [`receive`].
#[allow(clippy::too_many_arguments)]
pub fn receive_onto(
    store: &artifact::Store,
    door: &dyn Door,
    shelf: Shelf,
    name: &str,
    expected: [u8; 32],
    length: u64,
    already: &[u8],
    more: &mut dyn FnMut(&mut [u8]) -> std::io::Result<usize>,
) -> Result<Response, Response> {
    // **The digest the caller stated is the key**, and it is known before a byte
    // arrives. That is what makes a content-addressed store possible on a
    // streaming transfer at all: the destination can be built up front, and a
    // body whose hash turns out to disagree is thrown away below, before
    // anything is renamed into the store.
    let Some(digest) = artifact::Digest::new(&hex(&expected)) else {
        // Unreachable: `accept` parsed these 32 bytes out of 64 hex characters
        // and `hex` renders them lowercase. Refused rather than unwrapped,
        // because the alternative is a panic in the daemon that serves the
        // control plane.
        return Err(Response::refuse(
            500,
            "Internal Server Error",
            "the digest this machine parsed does not render back as one\n".to_string(),
        ));
    };

    // **Exclusive ownership of this digest, in this store, from here to every
    // return below — see [`Claim`].** Without it, two connections racing the
    // same digest share the mutable `incoming/<digest>/` this function is
    // about to build: `File::create` on the same path truncates whichever
    // opened second, one transfer's failure cleanup removes the directory a
    // sibling transfer is still writing into, and a rename can publish the
    // directory with an unrelated member still incomplete inside it.
    let _claim = Claim::take(store.root(), &digest);

    // **The store, opened once and held** — see `crate::beneath`. Every entry
    // below is made, created, renamed and removed relative to this descriptor
    // and never followed if it is a link, so a store an application can
    // rearrange (a fleet server's repository store sits in `rivet-fleetd`'s own
    // data directory) cannot send root's writes anywhere else.
    let root = match Dir::anchor(store.root()) {
        Ok(root) => root,
        Err(e) => return Err(unusable(store.root(), &e)),
    };
    let objects = match root.open("objects") {
        Ok(objects) => Some(objects),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(unusable(&store.objects_dir(), &e)),
    };
    let digest_name = digest.as_str();

    let object_dir = store.object_dir(&digest);
    let held_dir = match objects.as_ref().map(|objects| objects.open(digest_name)) {
        None => None,
        Some(Ok(held)) => Some(held),
        Some(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => None,
        Some(Err(e)) => return Err(unusable(&object_dir, &e)),
    };
    // **Already held is success, not a conflict.** A re-sent artifact is the
    // ordinary shape of a retried deploy, and the bytes cannot differ: the name
    // is their hash.
    //
    // **The body is READ OUT anyway, and that is not waste** — see [`drain`].
    //
    // **And the held copy goes through the door before this answers `201`.**
    // A store written before 2026-09-12 can hold an artifact nothing verified,
    // and one admitted since can have been signed by a key an image update has
    // revoked. Answering "already held" would be this machine vouching for it.
    // So the copy is judged by the same door as an arrival — which also means
    // re-sending a stale artifact is what removes it, with no sweep to run.
    if let (Some(held_dir), Some(objects)) = (held_dir, objects.as_ref()) {
        let unread = length.saturating_sub(already.len() as u64);
        let drained = drain(unread, more);
        let held = held_dir
            .names()
            .ok()
            .and_then(|names| names.into_iter().next())
            .map(|file| store.object_path(&digest, &file));
        drop(held_dir);
        let Some(held) = held else {
            return Err(refused_at_the_door(
                shelf,
                Refusal::Machine(format!(
                    "{object_dir} exists and holds no file, so there is nothing to verify"
                )),
                name,
                &digest,
                true,
            ));
        };
        let held_len = std::fs::metadata(&held).map_or(length, |m| m.len());
        return match door.admit(&held, held_len) {
            Ok(verified) => {
                Ok(already_held(shelf, store, &digest, name, length, drained, &verified))
            }
            Err(refusal) => {
                if matches!(refusal, Refusal::Artifact(_)) {
                    let _ = objects.remove(digest_name);
                }
                Err(refused_at_the_door(shelf, refusal, name, &digest, true))
            }
        };
    }

    let incoming_dir = store.incoming_for(&digest);
    let final_path = store.object_path(&digest, name);
    let temporary = store.incoming_path(&digest, name);

    let incoming = match root.make("incoming") {
        Ok(incoming) => incoming,
        Err(e) => return Err(unusable(&store.incoming_dir(), &e)),
    };
    // **Whatever is already at `incoming/<digest>` goes first**, link or
    // directory: the [`Claim`] means no transfer of this digest is running, so
    // anything there is a dead transfer's leftovers or something nobody here
    // made, and the transfer is built in a directory made fresh below.
    match incoming.remove(digest_name) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(unusable(&incoming_dir, &e)),
    }
    let building = match incoming.make(digest_name) {
        Ok(building) => building,
        Err(e) => {
            return Err(Response::refuse(
                500,
                "Internal Server Error",
                format!("cannot create {incoming_dir}: {e}\n"),
            ));
        }
    };

    let mut file = building.create_new(name).map_err(|e| {
        let _ = incoming.remove(digest_name);
        Response::refuse(500, "Internal Server Error", format!("cannot create {temporary}: {e}\n"))
    })?;
    drop(building);
    let mut hasher = rivet_crypto::Blake3::new();
    hasher.start();

    // **The whole incoming directory goes, not just the file in it.** A
    // transfer that died leaving an empty `incoming/<digest>/` would be
    // invisible to everything that resolves a digest — the store is
    // `objects/`, and `incoming/` is deliberately a sibling — but it would
    // accumulate, one directory per abandoned transfer, on the partition least
    // able to afford it.
    let fail = |file: std::fs::File, response: Response| -> Response {
        drop(file);
        let _ = incoming.remove(digest_name);
        response
    };

    let mut written: u64 = 0;
    let take = |n: u64| usize::try_from(n.min(64 * 1024)).unwrap_or(64 * 1024);

    let first = already.len().min(usize::try_from(length).unwrap_or(usize::MAX));
    if first > 0 {
        if let Err(e) = file.write_all(&already[..first]) {
            return Err(fail(file, write_failed(&temporary, &e)));
        }
        hasher.write(&already[..first]);
        written += first as u64;
    }

    let mut chunk = vec![0u8; 64 * 1024];
    while written < length {
        let want = take(length - written);
        let got = match more(&mut chunk[..want]) {
            Ok(0) => {
                return Err(fail(
                    file,
                    Response::refuse(
                        400,
                        "Bad Request",
                        format!(
                            "the connection closed after {written} of {length} bytes. Nothing \
                             was kept: a partial payload under the final name is one the \
                             installer would pick up.\n"
                        ),
                    ),
                ));
            }
            Ok(n) => n,
            Err(e) => {
                return Err(fail(
                    file,
                    Response::refuse(
                        400,
                        "Bad Request",
                        format!("the transfer failed after {written} of {length} bytes: {e}\n"),
                    ),
                ));
            }
        };
        if let Err(e) = file.write_all(&chunk[..got]) {
            return Err(fail(file, write_failed(&temporary, &e)));
        }
        hasher.write(&chunk[..got]);
        written += got as u64;
    }

    let actual = hasher.finish();
    if actual != expected {
        return Err(fail(
            file,
            Response::refuse(
                409,
                "Conflict",
                format!(
                    "the body's BLAKE3 is not what the {DIGEST_HEADER} header said, so the \
                     transfer corrupted and NOTHING WAS KEPT.\n\
                     \x20 sent      {}\n\
                     \x20 received  {}\n",
                    hex(&expected),
                    hex(&actual)
                ),
            ),
        ));
    }

    if let Err(e) = file.sync_all() {
        return Err(fail(file, write_failed(&temporary, &e)));
    }
    drop(file);

    // **The door, and it is AFTER the digest and BEFORE the name.** After,
    // because a transfer that corrupted is a fault in the wire and must be
    // reported as one — no signature work is done on bytes that are not the
    // ones that were sent. Before, because the name is what makes an artifact
    // part of the store: resolvable by an install, counted by retention. An
    // artifact the door refuses leaves `incoming/` with nothing behind it.
    let verified = match door.admit(&temporary, length) {
        Ok(verified) => verified,
        Err(refusal) => {
            let _ = incoming.remove(digest_name);
            return Err(refused_at_the_door(shelf, refusal, name, &digest, false));
        }
    };

    // **The unit of atomicity is the DIRECTORY, not the file** — see
    // `rivet_core::artifact`. An object appears in the store by one `rename(2)`
    // of a directory that is already complete, so nothing that resolves a digest
    // can ever see a partial artifact or an artifact with no name.
    let objects = match objects {
        Some(objects) => objects,
        None => match root.make("objects") {
            Ok(objects) => objects,
            Err(e) => {
                let _ = incoming.remove(digest_name);
                return Err(Response::refuse(
                    500,
                    "Internal Server Error",
                    format!("cannot create {}: {e}\n", store.objects_dir()),
                ));
            }
        },
    };
    if let Err(e) = incoming.rename_to(digest_name, &objects, digest_name) {
        // A concurrent transfer of the same artifact landing first is not a
        // failure — the bytes are identical by construction.
        if objects.open(digest_name).is_ok() {
            let _ = incoming.remove(digest_name);
            // Nothing to drain: the loop above read the body whole. And the
            // door has just admitted these very bytes.
            return Ok(already_held(shelf, store, &digest, name, length, 0, &verified));
        }
        let _ = incoming.remove(digest_name);
        return Err(Response::refuse(
            500,
            "Internal Server Error",
            format!("cannot place {final_path}: {e}\n"),
        ));
    }
    // **The rename made durable**, as far as this platform lets a directory be
    // synced. The file's bytes were `fsync`ed before the door; this is the
    // directory entry that names them. Best effort: a directory that cannot be
    // synced (not a thing Windows offers) changes nothing about a transfer that
    // has already landed.
    let _ = objects.sync();

    if shelf == Shelf::Repository {
        return Ok(Response {
            status: 201,
            reason: "Created",
            body: format!(
                "{final_path}\n\
                 \x20 bytes   {length}\n\
                 \x20 blake3  {}\n\
                 \x20 checked this machine hashed what it received and it matches what you sent.\n\
                 {verified}\
                 \x20 stored  in this fleet server's repository store, keyed by that digest \
                 (.rulings/ R-184 D1). Nothing is installed from it, and no retention count \
                 applies: a prune removes only what the fleet repository's manifest stops \
                 naming.\n",
                hex(&actual),
            ),
        });
    }

    // **Retention runs here because here is where the store grows.** It is the
    // only moment it can, so it is the only moment worth checking — and a prune
    // that failed must not fail the transfer, which has already succeeded.
    let pruned = prune(store, &digest);

    Ok(Response {
        status: 201,
        reason: "Created",
        body: format!(
            "{final_path}\n\
             \x20 bytes   {length}\n\
             \x20 blake3  {}\n\
             \x20 checked this machine hashed what it received and it matches what you sent.\n\
             {verified}\
             \x20 stored  in this machine's artifact store, keyed by that digest. An install \
             names the digest and nothing else (ADR-052 §7).\n\
             {pruned}\
             \x20 next    nothing has been installed. `rivet apply install {} confirm`\n",
            hex(&actual),
            digest,
        ),
    })
}

/// Read the rest of a body this machine is not going to keep, and throw it away.
///
/// Answers how many bytes were discarded, which is fewer than `unread` when the
/// peer went away mid-drain. **A failed drain is not a failure of anything**:
/// the artifact is in the store, and the answer above it is true whether or not
/// the remainder of the body ever arrives.
///
/// # Why a machine that has already answered still reads the body
///
/// **Because closing a connection with unread bytes still arriving is a RESET,
/// and a reset destroys the answer this machine has just written.** Measured on
/// `rivet-01`, 2026-09-12: the client reported *"the transfer failed after
/// 196608 of 402198 bytes: An established connection was aborted"* — the same
/// offset on two runs — while this machine logged `201 Created` for every push
/// and its store held the artifact complete. The transfer had worked. The
/// operator saw a failure, and the install that should have followed was never
/// asked for.
///
/// The mechanism is TCP's and not this project's: a socket closed while its
/// receive queue holds bytes the application never read sends an `RST` instead
/// of finishing the `FIN` exchange, and a peer that is mid-`write` sees
/// `ECONNRESET`/`WSAECONNABORTED` rather than the response already sitting in
/// its receive buffer. Three 64 KiB writes had landed before the reset got back.
///
/// **The answer does not survive the reset, which is why this side is the fix.**
/// Measured on a Windows host over loopback, 2026-09-12: with a server that
/// answered and closed without draining, the client's write failed with
/// `WinError 10053, "An established connection was aborted"` — the board's own
/// wording — and reading afterwards returned **zero bytes and the same error**,
/// twice out of two. With the same server draining first, the client sent the
/// whole body and read the `201`. A reset discards a receive queue, so a client
/// cannot repair this from its end; `rivet_remote::tls::transfer` reads the
/// answer anyway because the rule is right, not because it recovers this case.
///
/// So the short-circuit above stays — the bytes are not written twice, hashed
/// twice, or `fsync`ed twice, which is where the real cost of a re-send is — and
/// what it no longer does is hang up on somebody mid-sentence. What this costs
/// is the wire time of a body nobody wanted, on the ONE path where the machine
/// already knows the answer.
///
/// **The cheaper fix is `Expect: 100-continue` and it is deliberately not taken
/// here.** RFC 9110 §10.1.1's handshake is exactly this question — *do you want
/// this body* — and it would save the transfer rather than merely making it
/// harmless. It is not taken because it is a *negotiation*: both ends have to
/// know it, a client that waited for a `100` from a machine that will never send
/// one would hang, and the version-crossing story for that is a larger decision
/// than this defect earns. A drain is one-sided, and one-sidedness is the
/// property that makes it deployable on its own. The handshake stays available
/// on the day a re-send of 683 MB is worth saving rather than merely worth
/// surviving.
fn drain(unread: u64, more: &mut dyn FnMut(&mut [u8]) -> std::io::Result<usize>) -> u64 {
    let mut sink = vec![0u8; 64 * 1024];
    let mut discarded = 0u64;
    while discarded < unread {
        let want = usize::try_from((unread - discarded).min(64 * 1024)).unwrap_or(64 * 1024);
        match more(&mut sink[..want]) {
            Ok(0) | Err(_) => break,
            Ok(n) => discarded += n as u64,
        }
    }
    discarded
}

/// The answer for an artifact this machine already had.
///
/// **201 rather than 200 or 409**, and the reason is what the caller does next:
/// `cargo rivet deploy` asks for an install by digest immediately afterwards,
/// and the state it needs — *this machine holds these bytes* — is the same
/// whether this transfer put them there or an earlier one did. A 409 would make
/// a retried deploy look like a failure it is not.
///
/// `drained` is what [`drain`] discarded. It is reported rather than kept quiet
/// because an operator watching a deploy send 683 MB for nothing deserves to
/// read why, and because a count of zero where the body was not empty is how a
/// future defect in the drain would show itself.
///
/// `verified` is the door's line about the held copy, which [`receive`] has
/// judged before calling this.
fn already_held(
    shelf: Shelf,
    store: &artifact::Store,
    digest: &artifact::Digest,
    name: &str,
    length: u64,
    drained: u64,
    verified: &str,
) -> Response {
    let next = match shelf {
        Shelf::Artifacts => {
            format!("\x20 next    nothing has been installed. `rivet apply install {digest} confirm`\n")
        }
        Shelf::Repository => {
            "\x20 stored  already, in this fleet server's repository store (.rulings/ R-184 D1).\n"
                .to_owned()
        }
    };
    Response {
        status: 201,
        reason: "Created",
        body: format!(
            "{path}\n\
             \x20 bytes   {length} (not written — this machine already holds this digest)\n\
             \x20 blake3  {digest}\n\
             \x20 checked nothing was compared byte by byte and nothing needed to be: the \
             store is keyed by the hash of the contents, so an object under this name is \
             these bytes.\n\
             {verified}\
             {drained_line}\
             {next}",
            path = store.object_path(digest, name),
            drained_line = if drained == 0 {
                String::new()
            } else {
                format!(
                    "\x20 drained {drained} further byte(s) were received and discarded. This \
                     machine answered before the body was finished and then read it out anyway: \
                     hanging up on a peer mid-body RESETS the connection, which destroys this \
                     answer on its way back.\n"
                )
            },
        ),
    }
}

/// **Three newest per product** — `rivet_core::artifact::KEEP`, applied.
///
/// Returns a line for the response body, or an empty string when nothing was
/// dropped. Every failure here is **silent to the caller and non-fatal**: the
/// transfer succeeded, the store is merely larger than it should be, and turning
/// a successful 700 MB upload into an error because a directory would not list
/// is the wrong trade by a wide margin. What a failed prune costs is disk; what
/// failing the transfer costs is the deploy.
///
/// **Arrival order comes from the object directory's `mtime`**, which is what
/// `Stored::added` is documented to be. `ADR-021`: a Pi's clock is a floor that
/// never goes backwards, which orders arrivals within a machine's life and is
/// exactly as much as this needs.
///
/// `arriving` is the object this transfer has just stored, and
/// `rivet_core::artifact::evictions` never drops it. **That is not a
/// belt-and-braces flourish**: four artifacts written inside one clock tick have
/// the same `mtime`, so without it a deploy could transfer 700 MB, have the
/// prune delete exactly those bytes, and then ask the machine to install a
/// digest it no longer held.
fn prune(store: &artifact::Store, arriving: &artifact::Digest) -> String {
    let Ok(entries) = std::fs::read_dir(store.objects_dir()) else {
        return String::new();
    };

    let mut held: Vec<artifact::Stored> = Vec::new();
    for entry in entries.flatten() {
        let Some(digest) = artifact::Digest::new(&entry.file_name().to_string_lossy()) else {
            // Not a digest, so nothing this store put here and nothing it will
            // remove. Left alone rather than tidied: a directory nobody
            // recognises is somebody's, and `rivet_core::repo`'s rule about
            // never deleting an undeclared directory is the same instinct.
            continue;
        };
        let added = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        let Ok(mut files) = std::fs::read_dir(store.object_dir(&digest)) else {
            continue;
        };
        let Some(Ok(file)) = files.next() else {
            continue;
        };
        held.push(artifact::Stored {
            digest,
            name: file.file_name().to_string_lossy().into_owned(),
            added,
        });
    }

    let dropped: Vec<String> = artifact::evictions(&held, artifact::KEEP, Some(arriving))
        .into_iter()
        .filter(|stored| std::fs::remove_dir_all(store.object_dir(&stored.digest)).is_ok())
        .map(|stored| stored.name.clone())
        .collect();

    if dropped.is_empty() {
        String::new()
    } else {
        format!(
            "\x20 pruned  {} — this store keeps the {} newest of each product\n",
            dropped.join(", "),
            artifact::KEEP,
        )
    }
}

/// The refusal for a store directory this transfer cannot use, saying so
/// plainly when what stands there is a link or not a directory at all.
fn unusable(path: &str, e: &std::io::Error) -> Response {
    let body = if beneath::is_not_real(e) {
        format!(
            "{path} is a symbolic link or not a directory, so nothing was created through it and \
             this object was not received\n"
        )
    } else {
        format!("cannot use {path}: {e}\n")
    };
    Response::refuse(500, "Internal Server Error", body)
}

fn write_failed(path: &str, e: &std::io::Error) -> Response {
    Response::refuse(
        507,
        "Insufficient Storage",
        format!(
            "cannot write {path}: {e}\n\
             \x20 note  /state on a card built at the layout's minimum is 232 MiB and a system \
             update is about 684 MB (PLAN.md §13.4). Nothing was kept.\n"
        ),
    )
}

#[cfg(test)]
#[path = "bulk_tests.rs"]
mod tests;
