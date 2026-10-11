//! **What the card carries FOR an application that is not installed yet** —
//! `ADR-049`, ruled 2026-09-07.
//!
//! # The sentence this module exists to make true
//!
//! `SPEC-05` §1 is one requirement of the maintainer's: once the device has
//! booted, `fleetd` is up and running in the way `mcpd` is (`.rulings/` R-303).
//! Reaching that state by hand on 2026-09-07 took three secret installations,
//! two directory creations, three file writes, five ownership changes and two
//! service restarts, every one typed over the control plane after the card had
//! already booted. This is the format that puts all of it on the card instead.
//!
//! # The mechanism is `--identity`'s, one hop longer
//!
//! `cmd/rivet-image`'s `--identity DIR` already states the pattern: material is
//! written to the BOOT partition, PID 1 copies it into `/state` before any
//! service runs, and the secret half is destroyed on the card (`ADR-019` §3).
//! **Nothing here invents a second one.**
//!
//! What identity does not have to solve is ownership. An application's data
//! directory is mode `0700` owned by that application's own uid (`ADR-015` §5),
//! and the uid is derived by the supervisor at discovery — it is not knowable
//! when the card is written and not knowable by PID 1 either. So the courier
//! needs one hop more:
//!
//! ```text
//!   boot partition -> PID 1 -> /state/staging -> the SUPERVISOR delivers into
//!                                                each application's data
//!                                                directory, with the uid it
//!                                                has just allocated
//! ```
//!
//! **And the wait between the two hops is not a detail.** None of the
//! applications a fleet server runs is on the card — they arrive as signed
//! payloads afterwards — so an application's share of the staging usually has
//! to sit under `/state` until an install happens. [`staged_bundle`] is where
//! it sits, and the supervisor delivers it at the discovery pass that first
//! sees the application installed, which is the same pass that creates the
//! directory it goes into.
//!
//! **The last hop is not decoration.** On 2026-09-07 the register and the
//! credentials were placed by hand as root, and `rivet-fleetd` could not read
//! one file of it. It logged NOTHING, because `rivetos_register::store::load`
//! treats a missing directory as an empty register by design — correct for a
//! first run and silent for this.
//!
//! # Why the format is not TOML
//!
//! This crate performs no I/O and takes no dependency beyond `secrecy` and
//! `zeroize` (`ADR-014` §3), and both readers are the two programs that must not
//! grow a parser they cannot test: PID 1 and the supervisor. A line is an
//! application name, a relative path and hex — three fields, one space between
//! them, longest last. It is diffable, it is greppable, and the whole grammar
//! fits in [`parse`].
//!
//! **Hex rather than raw bytes**, for `ADR-020` §11's reason and one more: the
//! bundle sits on a FAT partition where a NUL is a hazard to every tool that
//! might read it, and a `server.key` is PEM while an `operator-device.crt.hex`
//! is already hex text. One encoding for both ends the question.
//!
//! # What it does NOT carry
//!
//! **No mode, and no owner.** Every staged file lands mode [`FILE_MODE`] inside
//! a directory the application owns and nobody else can open, so a per-file
//! mode would be a second, weaker answer to a question `ADR-015` has already
//! settled. Naming the owner would be worse: the card cannot know the uid, and
//! a number written at card time that later disagreed with the supervisor's
//! would produce exactly the silent unreadable directory this exists to end.

use crate::service::ServiceName;

/// The staging bundle's name on the boot partition.
///
/// **`.txt` because it is text**, and because a person who finds it on a card
/// in a workstation should be able to open it without guessing. It is not
/// `.toml`: nothing parses it as TOML and a name that promised otherwise would
/// invite somebody to try.
pub const BUNDLE_FILE: &str = "staged.txt";

/// The schema version the first line declares.
pub const VERSION: u32 = 1;

/// Where the bundle is unpacked to, between PID 1 and the supervisor.
pub const STAGING_DIR: &str = crate::state::STAGING;

/// The mode every staging directory is created with.
///
/// [`crate::service::DATA_DIR_MODE`]'s value and its reason: the contents are
/// one application's private material, and until the supervisor has chowned
/// them they belong to root.
pub const DIR_MODE: u32 = 0o700;

/// The mode every staged file is written with, in staging and after delivery.
pub const FILE_MODE: u32 = 0o600;

/// The longest a bundle may be, in bytes.
///
/// **A limit, because PID 1 reads this into memory before `/state` is even
/// certain to be writable.** Five megabytes is four orders of magnitude more
/// than the fleet server's own set needs — `SPEC-05` §10 measures it at under
/// two kilobytes — and still small enough that a truncated or corrupted card
/// cannot make PID 1 the reason a board does not boot.
pub const MAX_BUNDLE_BYTES: usize = 5 * 1024 * 1024;

/// One file, addressed to one application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Staged {
    /// The application whose data directory this belongs in.
    pub application: String,
    /// Where inside that data directory, relative and never absolute.
    pub path: String,
    /// The contents, decoded.
    pub bytes: Vec<u8>,
}

impl Staged {
    /// Build one, checking both names.
    ///
    /// # Errors
    ///
    /// [`Error::Application`] for a name that is not a service name, and
    /// [`Error::Path`] for a relative path this refuses — see [`check_path`].
    pub fn new(application: &str, path: &str, bytes: Vec<u8>) -> Result<Self, Error> {
        if ServiceName::new(application).is_none() {
            return Err(Error::Application {
                name: application.to_owned(),
            });
        }
        check_path(path)?;
        Ok(Self {
            application: application.to_owned(),
            path: path.to_owned(),
            bytes,
        })
    }

}

/// **Where one application's share waits between PID 1 and the supervisor.**
///
/// A bundle per application rather than an unpacked tree, and the reason is the
/// supervisor rather than PID 1. Delivery happens inside the discovery pass
/// that first sees an application installed, and that pass has a `Kernel` port
/// with no recursive directory walker in it — `list_dir` answers directories
/// only and `list_dir_entries` is documented as *"used for `ADR-015` §6's
/// device grant and nothing else … it is not a general directory walker and
/// should not become the easy way to have one."*
///
/// One `read_file_bytes` and one [`parse`] is the whole read, the paths are
/// checked by the parser rather than by a walk that could meet a symlink, and
/// what is pending for an application is one file an operator can `get file`.
/// Delivery ends with one `remove_file`.
#[must_use]
pub fn staged_bundle(application: &str) -> String {
    format!("{STAGING_DIR}/{application}{BUNDLE_SUFFIX}")
}

/// What [`staged_bundle`] puts on the end of an application's name.
pub const BUNDLE_SUFFIX: &str = ".txt";

/// Split a bundle into one bundle per application, in name order.
///
/// **What PID 1 writes**, and it is here rather than in PID 1 so that the
/// splitting and the reading are the same module's business (`ADR-011`).
#[must_use]
pub fn by_application(items: &[Staged]) -> Vec<(String, Vec<Staged>)> {
    let mut names: Vec<&str> = items.iter().map(|i| i.application.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    names
        .into_iter()
        .map(|name| {
            let mine: Vec<Staged> = items
                .iter()
                .filter(|i| i.application == name)
                .cloned()
                .collect();
            (name.to_owned(), mine)
        })
        .collect()
}

/// **Every directory `path` needs, outermost first**, relative to the root it
/// will be created under.
///
/// `register/devices/rivet-01.toml` answers `["register", "register/devices"]`.
/// The caller joins each to whichever root it is writing into, so one function
/// serves both the staging tree and the data directory and the two cannot
/// disagree about the shape.
#[must_use]
pub fn parents(path: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut so_far = String::new();
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop();
    for part in parts {
        if !so_far.is_empty() {
            so_far.push('/');
        }
        so_far.push_str(part);
        out.push(so_far.clone());
    }
    out
}

/// **What a staged path may be**, and it is deliberately narrow.
///
/// The supervisor writes these paths as root, joined onto a directory it owns.
/// A `..` in one would put a file anywhere on the machine, and a leading `/`
/// would ignore the join entirely — so this is the security boundary of the
/// whole mechanism and it is enforced at parse time rather than at write time,
/// where a caller could forget to ask.
///
/// # Errors
///
/// [`Error::Path`], naming the rule that was broken.
pub fn check_path(path: &str) -> Result<(), Error> {
    let refuse = |why: &'static str| {
        Err(Error::Path {
            path: path.to_owned(),
            why,
        })
    };
    if path.is_empty() {
        return refuse("it is empty");
    }
    if path.len() > 200 {
        return refuse("it is longer than 200 bytes");
    }
    if !path.is_ascii() {
        return refuse("a staged path is ASCII");
    }
    if path.chars().any(|c| c.is_ascii_control()) {
        return refuse("it contains a control character");
    }
    if path.chars().any(char::is_whitespace) {
        return refuse("it contains whitespace, and the fields on a line are space-separated");
    }
    if path.starts_with('/') {
        return refuse("it is absolute, and a staged path is relative to a data directory");
    }
    if path.contains('\\') {
        return refuse("it contains a backslash, which is not a path separator here");
    }
    for part in path.split('/') {
        if part.is_empty() {
            return refuse("it has an empty component, so it is not a path this can join");
        }
        if part == "." || part == ".." {
            return refuse(
                "it contains `.` or `..`, and a staged path may not leave the directory it is \
                 delivered into",
            );
        }
    }
    Ok(())
}

/// Why a bundle was refused.
///
/// **Every variant carries the text it objected to** except the hex, which may
/// be a private key: [`Error::Hex`] names the line and the reason and never the
/// value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The first non-comment line was not `version = N`.
    NoVersion,
    /// A version this build does not know.
    Version { given: String },
    /// A line that is not two or three fields. Two is a file of zero bytes:
    /// see [`parse`].
    Fields { line: usize, fields: usize },
    /// A name that is not a service name.
    Application { name: String },
    /// A path this refuses. See [`check_path`].
    Path { path: String, why: &'static str },
    /// Hex that would not decode. **The value is not quoted.**
    Hex { line: usize, why: &'static str },
    /// Two lines addressed the same file.
    Duplicate { application: String, path: String },
    /// More bytes than [`MAX_BUNDLE_BYTES`].
    TooLarge { bytes: usize },
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoVersion => write!(
                f,
                "the first line of a staging bundle is `version = {VERSION}` and this one has none"
            ),
            Self::Version { given } => write!(
                f,
                "this build reads staging bundles at version {VERSION} and this one declares \
                 {given:?}"
            ),
            Self::Fields { line, fields } => write!(
                f,
                "line {line} has {fields} space-separated field(s) and a staged file is three, \
                 or two for an empty file: <application> <relative/path> [<hex>]"
            ),
            Self::Application { name } => write!(
                f,
                "{name:?} is not a service name, so no application answers to it. {}",
                ServiceName::RULE
            ),
            Self::Path { path, why } => {
                write!(f, "{path:?} is not a staged path: {why}")
            }
            Self::Hex { line, why } => write!(
                f,
                "line {line}'s contents are not hex: {why}. The value is not quoted here because \
                 a staged file may be a private key"
            ),
            Self::Duplicate { application, path } => write!(
                f,
                "{application} is given {path} twice, and a bundle that says two things about one \
                 file has no single meaning"
            ),
            Self::TooLarge { bytes } => write!(
                f,
                "the bundle is {bytes} bytes and the limit is {MAX_BUNDLE_BYTES}. PID 1 reads it \
                 whole, before /state is known to be writable"
            ),
        }
    }
}

/// Render a bundle.
///
/// **Sorted by application and then by path**, so that two runs of the same
/// provisioning answers produce the same bytes and a card image stays
/// reproducible.
#[must_use]
pub fn render(items: &[Staged]) -> String {
    let mut sorted: Vec<&Staged> = items.iter().collect();
    sorted.sort_by(|a, b| {
        a.application
            .cmp(&b.application)
            .then_with(|| a.path.cmp(&b.path))
    });

    let mut out = String::new();
    out.push_str(
        "# Generated by rivet-provision. Material this machine's applications are\n\
         # handed BEFORE they first start: certificates, keys and the fleet register.\n\
         #\n\
         # PID 1 copies this to /state/staging on the first boot and then OVERWRITES\n\
         # AND UNLINKS this file, because it carries private keys (ADR-019 §3). The\n\
         # supervisor delivers each application's share into the data directory it\n\
         # owns, with the uid it has just allocated, and deletes the staged copy.\n\
         #\n\
         # One file per line: <application> <relative/path> <hex of the contents>\n",
    );
    out.push_str(&format!("version = {VERSION}\n"));
    for item in sorted {
        out.push_str(&format!(
            "{} {} {}\n",
            item.application,
            item.path,
            hex(&item.bytes)
        ));
    }
    out
}

/// Read a bundle.
///
/// **A two-field line is a file of zero bytes**, the exact inverse of what
/// [`render`] writes for an empty [`Staged`] entry — see this function's body
/// for why the third field goes empty rather than missing.
///
/// # Errors
///
/// [`Error`], and a refusal is always the whole bundle: a partially applied
/// staging is a machine in a state nobody designed, and the card still holds
/// the material so a corrected card is the repair.
pub fn parse(text: &str) -> Result<Vec<Staged>, Error> {
    if text.len() > MAX_BUNDLE_BYTES {
        return Err(Error::TooLarge { bytes: text.len() });
    }

    let mut items: Vec<Staged> = Vec::new();
    let mut version_seen = false;

    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let number = n.saturating_add(1);

        if !version_seen {
            let Some(rest) = line.strip_prefix("version") else {
                return Err(Error::NoVersion);
            };
            let Some(given) = rest.trim_start().strip_prefix('=') else {
                return Err(Error::NoVersion);
            };
            let given = given.trim();
            if given.parse::<u32>() != Ok(VERSION) {
                return Err(Error::Version {
                    given: given.to_owned(),
                });
            }
            version_seen = true;
            continue;
        }

        let fields: Vec<&str> = line.split_whitespace().collect();
        // A line of exactly two fields is a file of zero bytes: `render`
        // writes `<application> <path> <hex>` for every entry, the hex of
        // zero bytes is the empty string, and this parser trims the whole
        // line before splitting it — so the third field does not go
        // missing, it goes empty, and `split_whitespace` never produces an
        // empty trailing field to see that with.
        let (application, path, encoded) = match fields.as_slice() {
            [application, path, encoded] => (*application, *path, *encoded),
            [application, path] => (*application, *path, ""),
            _ => {
                return Err(Error::Fields {
                    line: number,
                    fields: fields.len(),
                });
            }
        };
        let bytes = unhex(encoded).map_err(|why| Error::Hex { line: number, why })?;
        let staged = Staged::new(application, path, bytes)?;
        if items
            .iter()
            .any(|i| i.application == staged.application && i.path == staged.path)
        {
            return Err(Error::Duplicate {
                application: staged.application,
                path: staged.path,
            });
        }
        items.push(staged);
    }

    if !version_seen {
        return Err(Error::NoVersion);
    }
    Ok(items)
}

/// Lower-case hex, the encoding [`crate::secret::Secret::to_stored`] uses.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, byte| {
        use core::fmt::Write as _;
        // Writing to a String cannot fail; the result is dropped rather than
        // unwrapped so this cannot become a panic inside PID 1.
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// The inverse, refusing rather than guessing.
fn unhex(text: &str) -> Result<Vec<u8>, &'static str> {
    if !text.len().is_multiple_of(2) {
        return Err("it has an odd number of characters");
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(text.len() / 2);
    let mut i = 0;
    while i < bytes.len() {
        let hi = bytes.get(i).copied().ok_or("it ended early")?;
        let lo = bytes.get(i + 1).copied().ok_or("it ended early")?;
        let value = nibble(hi)?
            .checked_mul(16)
            .ok_or("it is not hex")?
            .checked_add(nibble(lo)?)
            .ok_or("it is not hex")?;
        out.push(value);
        i += 2;
    }
    Ok(out)
}

fn nibble(c: u8) -> Result<u8, &'static str> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err("it contains a character that is not a hex digit"),
    }
}

#[cfg(test)]
#[path = "staging_tests.rs"]
mod tests;
