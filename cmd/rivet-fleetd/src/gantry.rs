//! **The `gantry` socket** — `SPEC-06`, decided by `ADR-061` §6.
//!
//! `gantry` hands APKs to phones and pairs them. It needs to read the `apk`
//! artifacts in this server's repository-object store and to read and change
//! the register's phone records; both are inside this program's own data
//! directory, and this socket is the one way a second application reaches them.
//!
//! # Two halves, and only one of them touches a socket
//!
//! [`Provider::answer`] is the protocol: one request line in, one reply out,
//! every operation and every failure code of `SPEC-06` §4 and §5, with no socket
//! in sight. [`converse`] is the framing over any byte stream — the line limit,
//! the member's raw bytes. [`serve`] is the thin edge: accept, check the peer's
//! uid, hand the connection to [`converse`].
//!
//! # Nothing is cached
//!
//! Every request reads the store's `objects/` and the register's `phones/`
//! afresh (`SPEC-06` §6). A second writer of `phones/` exists — the control
//! plane's rename and revoke — and each operation touches one file atomically,
//! so a copy held here would be the stale one.
//!
//! # What this asserts about an artifact, and what it does not
//!
//! The store admitted the object; this socket says so and nothing more. The
//! header's `kind` and member table are read **without** checking the
//! signature — this program is confined and cannot read the machine's trust
//! store — and `gantry` verifies the header itself before it offers anything to
//! a phone (`SPEC-06` §6).

use std::fs::File;
use std::io::{Read, Seek as _, SeekFrom, Write};
use std::path::{Path, PathBuf};

use rivet_core::artifact::{Digest, Store};
use rivet_core::journal::write_json_string;
use rivet_core::json::Json;
use rivet_core::package::apk::{APK_NAME, ICON_NAME, METADATA_NAME};
use rivet_core::package::format::{
    HEADER_BYTES, KIND_BYTES, M_COMPRESSION, M_NAME, M_NAME_BYTES, M_OFFSET, M_STORED_LEN,
    MAGIC, MAX_MEMBERS, MEMBER_RECORD_BYTES, MEMBER_TABLE_OFFSET, OFF_KIND, OFF_MEMBER_COUNT,
    read_name, read_text, read_u32, read_u64,
};
use rivet_core::package::{COMPRESSION_NONE, Kind};
use rivetos_register::phones::{self, Phone, PhoneError};

/// The endpoint's declared name: `provides = ["socket:gantry"]`.
pub const ENDPOINT: &str = "gantry";

/// The one application this endpoint answers.
pub const CONSUMER: &str = "gantry";

/// The protocol version this provider speaks.
pub const VERSION: i64 = 1;

/// The longest request line, terminator included — `SPEC-06` §3.
pub const MAX_REQUEST: usize = 4096;

/// How long a connection may send nothing before it is closed — `SPEC-06` §3.
pub const IDLE: std::time::Duration = std::time::Duration::from_secs(60);

/// The members an `apk` artifact may carry — `ADR-061` §3.
pub const MEMBERS: [&str; 3] = [APK_NAME, METADATA_NAME, ICON_NAME];

/// The DER prefix of a SubjectPublicKeyInfo holding an uncompressed EC P-256
/// point: the `id-ecPublicKey` and `prime256v1` object identifiers and a
/// 66-byte BIT STRING, whose content is `04` and the point's 64 bytes.
const P256_SPKI_PREFIX: [u8; 26] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
    0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
];

/// The length of such a SubjectPublicKeyInfo: the prefix, `04`, and 64 bytes.
pub const P256_SPKI_BYTES: usize = 91;

/// A refusal: the code a program branches on and a sentence for a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub code: &'static str,
    pub message: String,
}

impl Failure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// The reply line, terminator included.
    #[must_use]
    pub fn line(&self) -> String {
        let mut out = String::from("{\"ok\":false,\"error\":");
        write_json_string(self.code, &mut out);
        out.push_str(",\"message\":");
        write_json_string(&self.message, &mut out);
        out.push_str("}\n");
        out
    }
}

/// What one request is answered with.
#[derive(Debug)]
pub enum Answer {
    /// One reply line, terminator included.
    Line(String),
    /// A member: the `{"ok":true,"length":N}` line, then exactly `length`
    /// bytes read from `file`, which is already positioned at the member.
    Member {
        head: String,
        file: File,
        length: u64,
    },
}

/// The protocol, over one data directory.
#[derive(Debug, Clone)]
pub struct Provider {
    /// This application's `RIVET_APP_DATA`: the register and the store are
    /// both inside it.
    data: PathBuf,
    /// Seconds since the Unix epoch, for an enrollment's time.
    clock: fn() -> u64,
}

/// The system clock, in whole seconds.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Provider {
    /// A provider over `data`, enrolling phones at the system clock's time.
    #[must_use]
    pub fn new(data: PathBuf) -> Self {
        Self { data, clock: now }
    }

    /// The same, with the clock supplied, so a test can name the time.
    #[cfg(test)]
    #[must_use]
    pub fn with_clock(data: PathBuf, clock: fn() -> u64) -> Self {
        Self { data, clock }
    }

    /// The register's root: `register.toml`, `devices/` and `phones/`.
    fn register_root(&self) -> PathBuf {
        self.data.join(crate::fleet::REGISTER_DIR)
    }

    /// The repository-object store.
    fn store(&self) -> Store {
        Store::at(
            self.data
                .join(rivet_core::repository_objects::DIR)
                .to_string_lossy()
                .into_owned(),
        )
    }

    /// Answer one request line, its terminator already removed.
    #[must_use]
    pub fn answer(&self, line: &[u8]) -> Answer {
        match self.dispatch(line) {
            Ok(answer) => answer,
            Err(failure) => Answer::Line(failure.line()),
        }
    }

    fn dispatch(&self, line: &[u8]) -> Result<Answer, Failure> {
        let text = std::str::from_utf8(line)
            .map_err(|_| Failure::new("malformed", "the request line is not UTF-8"))?;
        let request = rivet_core::json::parse(text)
            .map_err(|e| Failure::new("malformed", format!("the request is not JSON: {e}")))?;
        if !matches!(request, Json::Object(_)) {
            return Err(Failure::new(
                "malformed",
                format!("the request is {}, not a JSON object", request.kind()),
            ));
        }
        let twice = request.duplicate_keys();
        if !twice.is_empty() {
            return Err(Failure::new(
                "malformed",
                format!("the request carries {} more than once", twice.join(", ")),
            ));
        }
        if request.get("v").and_then(Json::as_integer) != Some(VERSION) {
            return Err(Failure::new(
                "version",
                format!("this provider speaks protocol version {VERSION} only, and says so as \"v\":{VERSION}"),
            ));
        }
        let op = text_field(&request, "op")?;
        match op {
            "fleet" => self.fleet(),
            "artifacts" => self.artifacts(),
            "member" => self.member(&request),
            "phones" => self.phones(),
            // The op's wire name is SPEC-06's, spelled as the phones already
            // send it; the handler's own name is not on the wire.
            "enrol" => self.enroll(&request),
            "rename" => self.rename(&request),
            "revoke" => self.revoke(&request),
            other => Err(Failure::new(
                "unknown-op",
                format!(
                    "{other:?} is not an operation; this provider answers fleet, artifacts, \
                     member, phones, enrol, rename and revoke"
                ),
            )),
        }
    }

    fn fleet(&self) -> Result<Answer, Failure> {
        let fleet = rivetos_register::store::load_fleet(&self.register_root())
            .map_err(|e| Failure::new("store", e.to_string()))?;
        let mut out = String::from("{\"ok\":true,\"name\":");
        match fleet.name.as_deref() {
            Some(name) => write_json_string(name, &mut out),
            None => out.push_str("null"),
        }
        out.push_str("}\n");
        Ok(Answer::Line(out))
    }

    fn artifacts(&self) -> Result<Answer, Failure> {
        let held = self.held_apks()?;
        let mut out = String::from("{\"ok\":true,\"artifacts\":[");
        for (index, apk) in held.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str("{\"digest\":");
            write_json_string(apk.digest.as_str(), &mut out);
            out.push_str(",\"header\":");
            write_json_string(&rivet_core::base64::encode(&apk.header), &mut out);
            out.push('}');
        }
        out.push_str("]}\n");
        Ok(Answer::Line(out))
    }

    fn member(&self, request: &Json) -> Result<Answer, Failure> {
        let digest = text_field(request, "digest")?;
        let name = text_field(request, "name")?;
        let unknown_artifact =
            || Failure::new("unknown-artifact", format!("no held apk has the digest {digest:?}"));
        let digest = Digest::new(digest).ok_or_else(unknown_artifact)?;
        let apk = self.held_apk(&digest)?.ok_or_else(unknown_artifact)?;
        if !MEMBERS.contains(&name) {
            return Err(Failure::new(
                "unknown-member",
                format!(
                    "{name:?} is not a member an apk carries; they are {}",
                    MEMBERS.join(", ")
                ),
            ));
        }
        let entry = member_entry(&apk.header, name).ok_or_else(|| {
            Failure::new(
                "unknown-member",
                format!("the apk {} carries no {name}", digest.as_str()),
            )
        })?;
        let path = apk.path.display().to_string();
        if entry.compression != COMPRESSION_NONE {
            return Err(Failure::new(
                "store",
                format!("{path}: {name} is compressed, which no apk member may be"),
            ));
        }
        let mut file = File::open(&apk.path)
            .map_err(|e| Failure::new("store", format!("{path}: {e}")))?;
        let file_len = file
            .metadata()
            .map_err(|e| Failure::new("store", format!("{path}: {e}")))?
            .len();
        if entry.offset.checked_add(entry.length).is_none_or(|end| end > file_len) {
            return Err(Failure::new(
                "store",
                format!("{path}: {name} runs past the end of the file"),
            ));
        }
        file.seek(SeekFrom::Start(entry.offset))
            .map_err(|e| Failure::new("store", format!("{path}: {e}")))?;
        Ok(Answer::Member {
            head: format!("{{\"ok\":true,\"length\":{}}}\n", entry.length),
            file,
            length: entry.length,
        })
    }

    fn phones(&self) -> Result<Answer, Failure> {
        let all = phones::list(&self.register_root()).map_err(store_failure)?;
        let mut out = String::from("{\"ok\":true,\"phones\":[");
        for (index, phone) in all.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(&phone_json(phone));
        }
        out.push_str("]}\n");
        Ok(Answer::Line(out))
    }

    fn enroll(&self, request: &Json) -> Result<Answer, Failure> {
        if request.get("identity").is_some() {
            return Err(Failure::new(
                "malformed",
                "an enrol request carries no identity: it is derived from the public key",
            ));
        }
        let encoded = text_field(request, "public_key")?;
        let label = text_field(request, "label")?;
        let key = rivet_core::base64::decode(encoded).ok_or_else(|| {
            Failure::new("bad-key", "public_key is not standard padded base64")
        })?;
        if !is_p256_spki(&key) {
            return Err(Failure::new(
                "bad-key",
                format!(
                    "public_key is not the DER SubjectPublicKeyInfo of an uncompressed EC P-256 \
                     key, which is exactly {P256_SPKI_BYTES} bytes"
                ),
            ));
        }
        phones::check_label(label).map_err(label_failure)?;
        let identity = identity_of(&key);
        let root = self.register_root();
        match phones::read(&root, &identity) {
            Ok(_) => {
                return Err(Failure::new(
                    "exists",
                    format!("a phone with this key is already enrolled, as {identity}"),
                ));
            }
            Err(PhoneError::NoSuchPhone { .. }) => {}
            Err(other) => return Err(store_failure(other)),
        }
        let phone = Phone::new(&identity, label, key, &rfc3339_seconds((self.clock)()))
            .map_err(store_failure)?;
        phones::write(&root, &phone).map_err(store_failure)?;
        Ok(Answer::Line(format!("{{\"ok\":true,\"phone\":{}}}\n", phone_json(&phone))))
    }

    fn rename(&self, request: &Json) -> Result<Answer, Failure> {
        let identity = text_field(request, "identity")?;
        let label = text_field(request, "label")?;
        let root = self.register_root();
        let mut phone = phones::read(&root, identity).map_err(phone_failure)?;
        phones::check_label(label).map_err(label_failure)?;
        label.clone_into(&mut phone.label);
        phones::write(&root, &phone).map_err(store_failure)?;
        Ok(Answer::Line(format!("{{\"ok\":true,\"phone\":{}}}\n", phone_json(&phone))))
    }

    fn revoke(&self, request: &Json) -> Result<Answer, Failure> {
        let identity = text_field(request, "identity")?;
        phones::remove(&self.register_root(), identity).map_err(phone_failure)?;
        Ok(Answer::Line("{\"ok\":true}\n".to_owned()))
    }

    /// Every held object whose header's kind is `apk`, sorted by digest.
    fn held_apks(&self) -> Result<Vec<HeldApk>, Failure> {
        let dir = self.store().objects_dir();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Failure::new("store", format!("{dir}: {e}"))),
        };
        let mut out = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| Failure::new("store", format!("{dir}: {e}")))?;
            let Some(digest) = Digest::new(&entry.file_name().to_string_lossy()) else {
                continue;
            };
            if let Some(apk) = self.held_apk(&digest)? {
                out.push(apk);
            }
        }
        out.sort_by(|a, b| a.digest.as_str().cmp(b.digest.as_str()));
        Ok(out)
    }

    /// The object held under `digest`, if there is one and its kind is `apk`.
    fn held_apk(&self, digest: &Digest) -> Result<Option<HeldApk>, Failure> {
        let dir = PathBuf::from(self.store().object_dir(digest));
        let Some(path) = object_file(&dir)? else {
            return Ok(None);
        };
        let Some(header) = read_header(&path)? else {
            return Ok(None);
        };
        Ok(is_apk(&header).then(|| HeldApk {
            digest: digest.clone(),
            path,
            header,
        }))
    }
}

/// One `apk` object the store holds.
struct HeldApk {
    digest: Digest,
    path: PathBuf,
    header: [u8; HEADER_BYTES],
}

/// Where one member lies in its object, from the header's member table.
struct MemberEntry {
    offset: u64,
    length: u64,
    compression: u8,
}

/// The `.rvt` in a digest's directory: the store keeps exactly one, and the
/// first by name is taken if a directory somehow holds more.
fn object_file(dir: &Path) -> Result<Option<PathBuf>, Failure> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Failure::new("store", format!("{}: {e}", dir.display()))),
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "rvt"))
        .collect();
    files.sort();
    Ok(files.into_iter().next())
}

/// The first [`HEADER_BYTES`] of an object, or `None` for a file too short to
/// hold them, which is no artifact at all.
fn read_header(path: &Path) -> Result<Option<[u8; HEADER_BYTES]>, Failure> {
    let failure = |e: std::io::Error| Failure::new("store", format!("{}: {e}", path.display()));
    let mut file = File::open(path).map_err(failure)?;
    let mut header = [0u8; HEADER_BYTES];
    match file.read_exact(&mut header) {
        Ok(()) => Ok(Some(header)),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
        Err(e) => Err(failure(e)),
    }
}

/// Whether a header is an `ADR-029` package whose kind is `apk`.
fn is_apk(header: &[u8; HEADER_BYTES]) -> bool {
    header.starts_with(&MAGIC)
        && read_text(&header[OFF_KIND..OFF_KIND + KIND_BYTES])
            .ok()
            .flatten()
            == Some(Kind::apk().as_str())
}

/// The member table's record for `name`.
fn member_entry(header: &[u8; HEADER_BYTES], name: &str) -> Option<MemberEntry> {
    let count = usize::try_from(read_u32(header, OFF_MEMBER_COUNT)?)
        .unwrap_or(MAX_MEMBERS)
        .min(MAX_MEMBERS);
    (0..count).find_map(|index| {
        let at = MEMBER_TABLE_OFFSET + index * MEMBER_RECORD_BYTES;
        let record = header.get(at..at + MEMBER_RECORD_BYTES)?;
        if read_name(&record[M_NAME..M_NAME + M_NAME_BYTES]).ok()? != name {
            return None;
        }
        Some(MemberEntry {
            offset: read_u64(record, M_OFFSET)?,
            length: read_u64(record, M_STORED_LEN)?,
            compression: record[M_COMPRESSION],
        })
    })
}

/// Whether `der` is exactly a SubjectPublicKeyInfo of an uncompressed P-256
/// point. Structural only: the point is not checked to lie on the curve.
#[must_use]
pub fn is_p256_spki(der: &[u8]) -> bool {
    der.len() == P256_SPKI_BYTES
        && der.starts_with(&P256_SPKI_PREFIX)
        && der.get(P256_SPKI_PREFIX.len()) == Some(&0x04)
}

/// A phone's identity: base64url, unpadded, of the SHA-256 of its key's DER.
#[must_use]
pub fn identity_of(der: &[u8]) -> String {
    use sha2::Digest as _;
    let digest: [u8; 32] = sha2::Sha256::digest(der).into();
    phones::identity_from_sha256(&digest)
}

/// `seconds` since the epoch as RFC 3339 UTC to the second, `Z` suffixed.
fn rfc3339_seconds(seconds: u64) -> String {
    rivet_core::journal::rfc3339(seconds.saturating_mul(1000)).replace(".000Z", "Z")
}

/// A string field, or `malformed` naming it.
fn text_field<'a>(request: &'a Json, name: &str) -> Result<&'a str, Failure> {
    match request.get(name) {
        Some(Json::Str(text)) => Ok(text),
        Some(other) => Err(Failure::new(
            "malformed",
            format!("\"{name}\" is {}, not a string", other.kind()),
        )),
        None => Err(Failure::new("malformed", format!("the request carries no \"{name}\""))),
    }
}

/// A phone record as `SPEC-06` §5 spells a `PHONE`.
fn phone_json(phone: &Phone) -> String {
    let mut out = String::from("{\"identity\":");
    write_json_string(&phone.identity, &mut out);
    out.push_str(",\"label\":");
    write_json_string(&phone.label, &mut out);
    out.push_str(",\"public_key\":");
    write_json_string(&rivet_core::base64::encode(&phone.public_key), &mut out);
    out.push_str(",\"enrolled\":");
    write_json_string(&phone.enrolled, &mut out);
    out.push('}');
    out
}

fn store_failure(error: PhoneError) -> Failure {
    Failure::new("store", error.to_string())
}

fn label_failure(error: PhoneError) -> Failure {
    match error {
        PhoneError::BadLabel { .. } => Failure::new("bad-label", error.to_string()),
        other => store_failure(other),
    }
}

/// A read or remove by identity: an identity that names no phone, whether or
/// not it could name one, is `unknown-phone`.
fn phone_failure(error: PhoneError) -> Failure {
    match error {
        PhoneError::NoSuchPhone { .. } | PhoneError::BadIdentity { .. } => {
            Failure::new("unknown-phone", error.to_string())
        }
        other => store_failure(other),
    }
}

/// Serve requests on one connection until it closes — `SPEC-06` §3.
///
/// A line longer than [`MAX_REQUEST`] is answered `too-long` and the
/// connection ends, because nothing marks where the next request would begin.
/// A connection that ends mid-line is simply closed.
///
/// # Errors
///
/// The stream's own read or write error, which ends the connection; a read
/// timeout is how an idle connection ends.
pub fn converse<S: Read + Write>(provider: &Provider, stream: &mut S) -> std::io::Result<()> {
    let mut pending: Vec<u8> = Vec::with_capacity(MAX_REQUEST);
    let mut chunk = [0u8; MAX_REQUEST];
    loop {
        let line: Vec<u8> = loop {
            if let Some(end) = pending.iter().position(|b| *b == b'\n') {
                break pending.drain(..=end).collect();
            }
            if pending.len() >= MAX_REQUEST {
                let failure = Failure::new(
                    "too-long",
                    format!(
                        "a request line is at most {MAX_REQUEST} bytes including its newline, so \
                         this connection is closed"
                    ),
                );
                stream.write_all(failure.line().as_bytes())?;
                return stream.flush();
            }
            let room = MAX_REQUEST - pending.len();
            let read = match stream.read(&mut chunk[..room]) {
                Ok(0) => return Ok(()),
                Ok(read) => read,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            pending.extend_from_slice(&chunk[..read]);
        };
        match provider.answer(&line[..line.len() - 1]) {
            Answer::Line(text) => stream.write_all(text.as_bytes())?,
            Answer::Member { head, file, length } => {
                stream.write_all(head.as_bytes())?;
                let sent = std::io::copy(&mut file.take(length), stream)?;
                if sent != length {
                    // The file shrank under the read. The client was promised
                    // `length` bytes and cannot be given them, and nothing
                    // could tell it where the next reply begins.
                    return Err(std::io::Error::other(format!(
                        "a member promised {length} bytes and {sent} could be read"
                    )));
                }
            }
        }
        stream.flush()?;
    }
}

/// Who a connection must come from, and how long it may sit silent.
#[derive(Debug, Clone, Copy)]
pub struct Gate {
    /// The only uid answered: `app_identity("gantry")` in production.
    pub uid: u32,
    /// How long a connection may send nothing.
    pub idle: std::time::Duration,
}

impl Gate {
    /// The gate `SPEC-06` §2 and §3 name: [`CONSUMER`]'s uid, [`IDLE`].
    #[must_use]
    pub fn for_consumer() -> Option<Self> {
        let name = rivet_core::service::ServiceName::new(CONSUMER)?;
        Some(Self {
            uid: rivet_core::service::app_identity(&name).uid,
            idle: IDLE,
        })
    }
}

/// Accept connections on `listener` for as long as it stays open, one thread
/// each.
#[cfg(target_os = "linux")]
pub fn serve(listener: &std::os::unix::net::UnixListener, provider: &Provider, gate: Gate) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        let provider = provider.clone();
        std::thread::spawn(move || one(stream, &provider, gate));
    }
}

/// One connection: the uid check before anything is read, then [`converse`].
#[cfg(target_os = "linux")]
fn one(mut stream: std::os::unix::net::UnixStream, provider: &Provider, gate: Gate) {
    let peer = rivet_sdk::peer::credentials(&stream);
    if let Some(refusal) = refusal(peer.as_ref().ok(), gate.uid) {
        let (uid, pid) = peer.as_ref().map_or((-1, -1), |c| (i64::from(c.uid), i64::from(c.pid)));
        let why = match &peer {
            Ok(_) => "the caller is not gantry".to_owned(),
            Err(e) => format!("the caller's credentials could not be read: {e}"),
        };
        rivet_sdk::journal::Journal::for_service(rivet_core::repository_objects::APPLICATION)
            .emit(
                rivet_sdk::journal::Severity::Warn,
                &format!("gantry socket: refused a connection: {why}"),
                &[
                    ("peer_uid", rivet_sdk::journal::Value::Int(uid)),
                    ("peer_pid", rivet_sdk::journal::Value::Int(pid)),
                ],
            );
        let _ = stream.write_all(refusal.line().as_bytes());
        let _ = stream.flush();
        return;
    }
    if stream.set_read_timeout(Some(gate.idle)).is_err()
        || stream.set_write_timeout(Some(gate.idle)).is_err()
    {
        return;
    }
    if let Err(why) = converse(provider, &mut stream) {
        println!("rivet-fleetd: gantry socket: connection ended: {why}");
    }
}

/// The refusal a peer gets, or `None` for the one uid this endpoint answers.
/// Credentials that could not be read are refused: nothing about the caller
/// is known, so it is not known to be `gantry`.
#[cfg(target_os = "linux")]
fn refusal(peer: Option<&rivet_sdk::peer::Credentials>, uid: u32) -> Option<Failure> {
    if peer.is_some_and(|c| c.uid == uid) {
        return None;
    }
    Some(Failure::new(
        "refused",
        "this socket answers the gantry application only, and this connection is not from it",
    ))
}

#[cfg(all(test, target_os = "linux"))]
#[path = "gantry_tests.rs"]
mod tests;
