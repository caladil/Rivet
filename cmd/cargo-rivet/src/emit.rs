//! Assembling and signing one `.rvt`.
//!
//! **The container writer is `rivet_core::package::write` and this is not a
//! second one.** `ADR-011`, and the writer's own module documentation says why:
//! the fact being kept in one place is *where the signature starts and what it
//! covers*, and a second spelling of `4032` maintained by a different person
//! from the one who maintains the verifier is how the two diverge. What lives
//! here is the I/O the core is forbidden to perform — opening files, streaming
//! them past a [`Digest`], and writing bytes in the order the header says.
//!
//! # Nothing is held in memory that does not have to be
//!
//! `Builder::seal` returns the 4096-byte header and nothing else, precisely so
//! a writer need not hold a payload. An application's binary is megabytes
//! rather than the 640 MiB a system update is, so holding one would work — and
//! the shape is the same either way, so it is done the way that keeps working.
//! Each member is read twice: once to digest it, once to copy it.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use rivet_core::package::format::DIGEST_BYTES;
use rivet_core::package::kind::Kind;
use rivet_core::package::port::{Digest, Sign};
use rivet_core::package::write::{Builder, NewMember};

/// A megabyte at a time — the same chunk `cmd/rivet`'s `measure hash` reads a
/// partition in.
const CHUNK: usize = 1024 * 1024;

/// Where a member's bytes come from.
pub enum Source {
    /// The manifest, which is generated rather than read.
    Bytes(Vec<u8>),
    /// The program, or an asset.
    File(PathBuf),
}

pub struct Part {
    pub name: String,
    pub class: u8,
    /// Byte 65 of the member record. `rivet_core::package::application`'s
    /// `VARIANT_NONE` for an application, and
    /// `rivet_core::package::update::SlotVariance` for an update payload.
    ///
    /// **It is a parameter rather than a constant here because it is half of a
    /// member's identity.** `ADR-029` §13.2 item 4: a payload carries
    /// `cmdline.txt` twice, so name alone cannot be the key — and a writer that
    /// hard-coded zero could not express the update kind at all.
    pub variant: u8,
    pub source: Source,
}

/// What was written, for the caller to report.
pub struct Written {
    pub path: PathBuf,
    pub total_len: u64,
    /// The digest of the **whole file**, taken as it was written.
    ///
    /// Not part of the format and not consulted by any verifier — the header's
    /// signature is what makes a payload trustworthy (`ADR-027` §5: the name is
    /// for humans; the manifest is the authority). This is here for the one job
    /// a whole-file digest is honest for: telling whether the bytes that arrived
    /// somewhere are the bytes that left, which is `PLAN.md` §0.8 link 5 and has
    /// no integrity check of its own.
    pub digest: [u8; DIGEST_BYTES],
    pub members: Vec<(String, u64)>,
}

/// The four header fields an artifact is identified by.
///
/// Gathered into one value rather than passed as four, because they travel
/// together everywhere — into the header, into `ADR-027`'s filename, and into
/// the report — and a caller that could pass them in the wrong order would put
/// an arch in a version field with no type to stop it.
///
/// **`platform` is `None` for an application and `Some` for an update**, which
/// is the maintainer's 2026-08-24 ruling rather than a field left unset: an application
/// is architecture-shaped and an update is firmware-shaped, so the two kinds
/// have different grammars (`ADR-027` §3, amended 2026-08-24 — *"a field either
/// says something or it does not exist for that kind"*).
///
/// ~~There is no `platform`~~ — struck 2026-08-24, when `cargo rivet update`
/// arrived and needed one. The sentence was true of the only kind this module
/// could emit when it was written.
pub struct Identity<'a> {
    pub product: &'a str,
    pub version: &'a str,
    pub arch: &'a str,
    pub platform: Option<&'a str>,
    pub kind: Kind,
}

/// Digest every part, seal the header, then write the file.
///
/// `reserved` is what the target platform reserves on a boot-directory target —
/// `ADR-029` §4, and **the port's answer rather than this crate's**. It is empty
/// for an application, which has no boot directory to collide with. Passing it
/// here rather than filtering at the caller means a reserved name that survived
/// the caller's own filter is refused by the builder with the file still named,
/// which is §4's *"the refusal is of the whole payload, not of the member"*.
pub fn write(
    out: &Path,
    identity: &Identity<'_>,
    parts: &[Part],
    reserved: &[&str],
    digest: &mut dyn Digest,
    signer: &mut dyn Sign,
) -> Result<Written, String> {
    let mut builder = Builder::new(
        identity.product,
        identity.version,
        identity.arch,
        identity.platform,
        identity.kind.clone(),
    )
    .map_err(|e| format!("this artifact cannot be named: {e}"))?
    .reserving(reserved);

    for part in parts {
        let (length, member_digest) = match &part.source {
            Source::Bytes(bytes) => (bytes.len() as u64, digest.of(bytes)),
            Source::File(path) => stream(path, digest)?,
        };
        let member = NewMember::streamed(
            &part.name,
            part.class,
            part.variant,
            0,
            member_digest,
            length,
            length,
        )
        .map_err(|e| format!("{:?} cannot be a member: {e}", part.name))?;
        builder
            .add(member)
            .map_err(|e| format!("{:?} cannot be added: {e}", part.name))?;
    }

    let header = builder
        .seal(digest, signer)
        .map_err(|e| format!("the header cannot be sealed: {e}"))?;

    if let Some(parent) = out.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    let file = File::create(out).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    let mut sink = BufWriter::new(file);

    // The whole-file digest is taken here, on the bytes actually written, rather
    // than computed from the parts. A digest of what a writer *meant* to write
    // is a digest of nothing.
    let mut whole = rivet_crypto::Blake3::new();
    whole.start();

    sink.write_all(&header)
        .map_err(|e| format!("cannot write the header: {e}"))?;
    whole.write(&header);

    let mut written = 0u64;
    for part in parts {
        match &part.source {
            Source::Bytes(bytes) => {
                sink.write_all(bytes)
                    .map_err(|e| format!("cannot write {:?}: {e}", part.name))?;
                whole.write(bytes);
                written += bytes.len() as u64;
            }
            Source::File(path) => {
                let mut source =
                    File::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                let mut buffer = vec![0u8; CHUNK];
                loop {
                    let read = source
                        .read(&mut buffer)
                        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                    if read == 0 {
                        break;
                    }
                    sink.write_all(&buffer[..read])
                        .map_err(|e| format!("cannot write {:?}: {e}", part.name))?;
                    whole.write(&buffer[..read]);
                    written += read as u64;
                }
            }
        }
    }
    sink.flush()
        .map_err(|e| format!("cannot flush {}: {e}", out.display()))?;
    let file = sink
        .into_inner()
        .map_err(|e| format!("cannot finish {}: {e}", out.display()))?;
    // A payload that is still in the host's page cache when the build reports
    // success is a payload a crash loses, and the caller is about to copy it
    // somewhere. Cheap here; not cheap to discover later.
    file.sync_all()
        .map_err(|e| format!("cannot sync {}: {e}", out.display()))?;

    let expected = builder.total_len();
    let actual = header.len() as u64 + written;
    if actual != expected {
        // A file whose length disagrees with its own header is one the verifier
        // refuses (`ADR-029` §5), so producing one and reporting success would
        // move the failure onto a board. It can only happen if a member changed
        // on disk between being digested and being copied, which is a real thing
        // on a build host running a watch task.
        return Err(format!(
            "the header describes {expected} bytes and {actual} were written; a member changed \
             underneath this build"
        ));
    }

    Ok(Written {
        path: out.to_path_buf(),
        total_len: actual,
        digest: whole.finish(),
        members: parts
            .iter()
            .zip(builder.members())
            .map(|(part, member)| (part.name.clone(), member.stored_len()))
            .collect(),
    })
}

fn stream(path: &Path, digest: &mut dyn Digest) -> Result<(u64, [u8; DIGEST_BYTES]), String> {
    let mut file = File::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut buffer = vec![0u8; CHUNK];
    let mut length = 0u64;
    digest.start();
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        digest.write(&buffer[..read]);
        length += read as u64;
    }
    Ok((length, digest.finish()))
}
