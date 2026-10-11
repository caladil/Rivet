//! The `application` kind: what an application's metadata *means*.
//!
//! `ADR-029` §11 rules one container for three artifact kinds, differing in
//! *"the **interpretation** of the metadata, not its shape"*. [`update`] was the
//! first interpretation; this is the second, and it exists because the
//! maintainer's 2026-08-24 ruling settled `ADR-027` §9 items 2–4:
//!
//! > `.rvt` for all three kinds, `<kind>` inside the file as `ADR-029` §8
//! > already has it, and **no `<platform>` for applications or extensions**. An
//! > application is architecture-shaped, not firmware-shaped.
//!
//! **Nothing in [`crate::package::read`] or [`crate::package::write`] changed to
//! add this kind.** That is §11 being true rather than aspirational: the second
//! interpretation is a second module here and the verifier is untouched.
//!
//! [`update`]: crate::package::update
//!
//! # What a package is — `ADR-015` §3
//!
//! > **A package is one static binary, a typed manifest, optional assets, and a
//! > signature.**
//!
//! Which maps onto three [`TargetClass`] values and nothing else. The signature
//! is the container's, so it is not a member; `ADR-015` §4.1's *"binary and
//! manifest are signed as one unit"* is `ADR-029` §5's transitive coverage —
//! every member's digest is in the signed header, so a rewritten manifest is a
//! refusal at [`crate::package::read::Package::verify_member`] rather than a
//! silently-widened permission request.
//!
//! # Four things this module decides that no ADR did
//!
//! Each is recorded here at its definition, each is small, and **each is the
//! maintainer's to overturn** — the same posture `ADR-029` §13.2 takes.
//!
//! 1. **The manifest is a member, not a header field.** `ADR-029` §11 says an
//!    application manifest *"can"* fit a fixed record table; it does not say it
//!    must, and the header has no room — its 4096 bytes are spoken for. A member
//!    named [`MANIFEST_NAME`] is covered by the signature transitively, which is
//!    all §4.1 asks for, and it costs no format change.
//! 2. **The manifest carries no name and no version.** They are the header's
//!    `product` and `version`, which every kind already has and which
//!    [`crate::package::read::Header`] already exposes. Putting them in the
//!    manifest as well would be two authorities over one fact (`ADR-011`) inside
//!    a single signed artifact — and the two could disagree. **The consequence
//!    is a real constraint and it is not hypothetical:** `product` is 16 bytes,
//!    so an application named `metrics-collector` — `ADR-015` §5.4's own worked
//!    example, seventeen characters — cannot be packaged. It is refused with the
//!    limit named rather than truncated, and the choice between widening the
//!    field (which costs `format_version` 2) and moving identity into the
//!    manifest is the maintainer's to rule, not this module's, and it is not
//!    yet ruled.
//! 3. **The manifest member must be member 0.** An installer reads what an
//!    application *is* before it streams a binary that may be megabytes; a
//!    manifest that could be last would make that impossible for a reader that
//!    cannot seek. It is one comparison in [`Application::open`].
//! 4. **The canonical manifest text is emitted and parsed here, and it is a
//!    strict subset of TOML rather than TOML.** `rivet-core`'s
//!    `[dependencies]` table is empty by rule (`ADR-014` §3), so a real TOML
//!    parser cannot live here — and the artifact is a *build product*, never
//!    hand-written, so the grammar can be as narrow as the emitter. It is
//!    TOML-shaped so that a person who opens one is not surprised, and
//!    [`Manifest::parse`] refuses everything the emitter does not produce,
//!    including a key out of order. See [`Manifest::render`].
//!
//! # What this module deliberately does not do
//!
//! * **It does not sandbox anything.** `ADR-015` §5's mechanisms are the
//!   supervisor's, and `ADR-015` §6.1 requires an unenforceable declaration to
//!   be *reported* rather than silently accepted. This module parses the
//!   request; nothing yet grants it.
//! * **It does not resolve `memory` against the operator's cap.** The
//!   maintainer's 2026-08-24 ruling splits the two by direction — the manifest owns what the
//!   application *is*, the document owns what the operator *limits* — and rules
//!   that where they meet, **the lower wins**. The lower of what, exactly, needs
//!   `128M` to have a meaning, and no formal ruling settles it — `ADR-015` §3.2
//!   never said whether it is binary or decimal. So [`Memory`] carries the
//!   string and refuses to convert it, for the reason
//!   [`crate::package::read::Header::version`] refuses to order two versions.
//! * **It does not install.** There is no installer for any kind (`ADR-029` §12
//!   item 4 is ruled — a new supervised binary — and unwritten), and there is no
//!   way to tell a Rivet about a service it was not compiled with
//!   (`PLAN.md` §0.8, link 7).

use crate::package::error::PackageError;
use crate::package::kind::Kind;
use crate::package::library;
use crate::package::read::{Member, Package, VerifiedMember};

/// The member holding [`Manifest`]'s canonical text.
///
/// Spelled `.toml` because the bytes are valid TOML and a person who opens one
/// should not have to be told what they are looking at. **It is not general
/// TOML** — see [`Manifest::parse`].
pub const MANIFEST_NAME: &str = "manifest.toml";

/// The largest manifest [`Manifest::parse`] will read.
///
/// Bounded because it is parsed from bytes somebody else wrote. The canonical
/// form of the largest manifest this build can emit — every list at
/// [`MAX_LIST`] entries of [`MAX_VALUE`] bytes — is under 12 KiB, so this is
/// generous by a factor of five and still a bound.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;

/// The largest program a supervisor will read in order to hash it.
///
/// **16 MiB, and the number is a policy rather than a measurement.** The
/// largest thing this project has packaged is `nu` at 44 MB, which is on
/// `/system` and is not an application; `rivet-todo` is 1.07 MB. A bound exists
/// so that a `/state` somebody has filled cannot make the supervisor allocate
/// it, and it must be raised deliberately rather than by a program that grew —
/// which is why exceeding it is a **refusal to launch** and says so, rather
/// than a warning and a spawn.
///
/// # Why it lives here and not in the supervisor
///
/// It was `cmd/rivet-sysd/src/verify.rs`, which is the only place that
/// *enforces* it — and that is exactly what made it useless to the author of an
/// oversized program. `cargo rivet deploy` packaged, signed, transferred and
/// installed a 16.5 MiB binary without comment, and the limit was first heard
/// of as `File too large (os error 27)` from a supervisor refusing to launch
/// it, on a different machine, minutes later (observed 2026-09-01,
/// `rivet-fleetd`). `ADR-011` gives a fact one home; both the tool that builds
/// a program and the supervisor that launches it need this one, so its home is
/// the crate they share.
///
/// **A binary is over this because it carries debug symbols far more often than
/// because it is large.** The `rivet-fleetd` that provoked this was 17,266,848
/// bytes and 1,495,720 after `strip` — 92% of what was signed and shipped was
/// symbols the device has no use for.
pub const MAX_PROGRAM_BYTES: usize = 16 * 1024 * 1024;

/// The most entries a manifest list may have.
pub const MAX_LIST: usize = 32;

/// The most bytes a single scalar or list element may have.
pub const MAX_VALUE: usize = 96;

/// **What a setting may be called** — `ADR-038` §3.
///
/// Lowercase letters, digits and hyphens; it must begin with a letter. The same
/// shape a manifest's other names take, and narrow for a reason that is not
/// tidiness: a setting name is written by an operator into a machine document
/// and read back by a program, so every character it admits is a character two
/// implementations have to agree about. Underscores and dots are refused
/// because `brightness_level` and `brightness.level` would be two spellings
/// nobody chose between.
pub const SETTING_RULE: &str = "a setting name is lowercase letters, digits and hyphens, \
                                beginning with a letter";

/// Whether `text` is a name a manifest may declare.
#[must_use]
pub fn is_setting_name(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_VALUE
        && text.starts_with(|c: char| c.is_ascii_lowercase())
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// **The words [`is_credential_like_name`] matches anywhere in a settings
/// name, case-insensitively** — `ADR-038` §5, `.rulings/` R-210 D16.
///
/// A credential is an installed secret (`apply install-secret`) and never a
/// document setting (`ADR-038` §5). `key` is not in this list: matched
/// anywhere, it would also catch `keyboard-layout` and `monkey`, so it is
/// checked separately, as a whole word, by
/// [`contains_key_as_whole_word`].
const CREDENTIAL_SUBSTRINGS: &[&str] = &["token", "secret", "password", "passwd", "credential"];

/// The sentence every refusal of a credential-shaped setting quotes —
/// `ADR-038` §5.
pub const CREDENTIAL_SETTING_RULE: &str = "a credential is an installed secret, never a \
                                           setting — install it with `apply install-secret` \
                                           and read it back with `rivet_sdk::secrets::get`, \
                                           not `rivet_sdk::settings`";

/// Whether `text` is a settings name that reads like a credential rather than
/// an ordinary setting — `ADR-038` §5, `.rulings/` R-210 D16.
///
/// **The one rule every check of a setting's name uses**, whether the name
/// comes from a manifest (this module's own builder and parser,
/// `cmd/cargo-rivet`'s packager) or from a fleet document's free-text
/// `[applications.*]` region (`rivet-fleetd`, which has no setting-name
/// alphabet of its own to lean on and calls this crate for the one rule).
/// That is also why the match is case-insensitive rather than tied to this
/// crate's own all-lowercase alphabet ([`is_setting_name`]): a document key
/// can be spelled `API_TOKEN` or `apiKey`.
///
/// `token`, `secret`, `password`, `passwd` and `credential` match ANYWHERE in
/// `text`. `key` matches only as a WHOLE WORD (see this module's own
/// `contains_key_as_whole_word`), so `keyboard-layout` and `monkey` are not
/// credential-shaped merely for containing those three letters in sequence.
/// `secretary` and `tokenizer` ARE credential-shaped under this rule —
/// `secret` and `token` still appear in them, and only `key` gets the
/// whole-word exemption.
///
/// This does not claim to be a complete filter — an author can still call a
/// credential `otp` and hand it to `settings` — only that the common
/// spellings this project has actually seen (TempMon's own `write-token`
/// among them) are caught before a signed payload or a machine document ever
/// carries one.
#[must_use]
pub fn is_credential_like_name(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    CREDENTIAL_SUBSTRINGS.iter().any(|needle| lower.contains(needle)) || contains_key_as_whole_word(text)
}

/// Whether `text` contains `key`, case-insensitively, as a WHOLE WORD — the
/// one needle [`is_credential_like_name`] cannot match as a plain substring
/// without also catching `keyboard-layout` and `monkey`.
///
/// A word boundary is any byte that is not an ASCII letter or digit (so `-`,
/// `_`, `.` and the like all count), and also a lowercase-to-uppercase
/// transition, so `apiKey` reads as `api` then `Key` with no separator
/// between them at all. This is a byte scan over ASCII alone: both alphabets
/// this rule is ever checked against are ASCII — a manifest's own
/// setting-name alphabet ([`is_setting_name`]) and a TOML key.
fn contains_key_as_whole_word(text: &str) -> bool {
    let bytes = text.as_bytes();
    let is_word_byte = |b: u8| b.is_ascii_alphanumeric();
    let is_transition = |before: u8, after: u8| before.is_ascii_lowercase() && after.is_ascii_uppercase();
    let mut start = 0;
    while start + 3 <= bytes.len() {
        let is_key = bytes[start].eq_ignore_ascii_case(&b'k')
            && bytes[start + 1].eq_ignore_ascii_case(&b'e')
            && bytes[start + 2].eq_ignore_ascii_case(&b'y');
        if is_key {
            let end = start + 3;
            let left_boundary = start == 0
                || !is_word_byte(bytes[start - 1])
                || is_transition(bytes[start - 1], bytes[start]);
            let right_boundary = end == bytes.len()
                || !is_word_byte(bytes[end])
                || is_transition(bytes[end - 1], bytes[end]);
            if left_boundary && right_boundary {
                return true;
            }
        }
        start += 1;
    }
    false
}

/// **What a proxy path may be** — `R-133`, 2026-09-03.
///
/// The same alphabet as [`SETTING_RULE`] and [`crate::service::ServiceName`],
/// which is what makes *mount it at its own name* always legal.
pub const PROXY_PATH_RULE: &str = "a proxy path is ONE segment of lowercase letters, digits and \
                                   hyphens, beginning with a letter — no slashes, so a route \
                                   cannot escape its own prefix";

/// **What an addon class name may be** — `PLAN-Feature-Set-2026-09-12` §6.2,
/// `ADR-053`.
///
/// [`SETTING_RULE`]'s alphabet again, and kept as a separate predicate for
/// [`is_proxy_path`]'s reason: this and a setting name are two rules about two
/// different things that happen to agree today, and collapsing them would mean
/// a later change to one silently changing the other (`ADR-011`).
///
/// **Free text, not reverse-DNS, and that is `ADR-053`'s decision rather than
/// an oversight.** A class is compared for exact equality by two signed
/// declarations that must already agree on every other byte of what they
/// mean, so a namespacing scheme buys nothing here that the two authors did
/// not already have to coordinate directly — unlike a socket name or a proxy
/// path, which are compared against strangers. The rule stays the one this
/// manifest already uses everywhere else a short machine-read token appears.
pub const CLASS_RULE: &str = "an addon class name is lowercase letters, digits and hyphens, \
                              beginning with a letter";

/// Whether `text` is a name an addon class may be called.
#[must_use]
pub fn is_class_name(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_VALUE
        && text.starts_with(|c: char| c.is_ascii_lowercase())
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Whether `text` is a path a manifest may ask to be mounted at.
///
/// **Identical to [`is_setting_name`] today, and kept separate on purpose.**
/// They are two rules about two different things that happen to agree, and
/// collapsing them would mean a later change to one silently changing the
/// other — `ADR-011` pointed at a predicate rather than a value.
#[must_use]
pub fn is_proxy_path(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_VALUE
        && text.starts_with(|c: char| c.is_ascii_lowercase())
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// **The paths this machine keeps for its own pages** — `ADR-046` §4.
///
/// `sign-in` is where a person signs in and enrolls a passkey, at the same
/// address on every machine, so no application may be mounted there. Every
/// door that admits a `proxy-path` asks [`is_mountable_proxy_path`], which
/// reads this one list: the manifest builder packaging uses, the parser an
/// installing machine and its supervisor read with, and the proxy reading its
/// routing table.
///
/// The proxy's other reserved prefix, `/.rivet/`, needs no entry here: no
/// `proxy-path` can begin with a dot.
pub const RESERVED_PROXY_PATHS: &[&str] = &["sign-in"];

/// What an author asking for a reserved path is told.
pub const RESERVED_PROXY_PATH_RULE: &str = "the proxy-path names a path this machine keeps for \
                                            its own sign-in pages, and no application may be \
                                            mounted there";

/// Whether `text` is one of [`RESERVED_PROXY_PATHS`].
#[must_use]
pub fn is_reserved_proxy_path(text: &str) -> bool {
    RESERVED_PROXY_PATHS.contains(&text)
}

/// **Whether an application may be mounted at `text`**: a legal
/// [`is_proxy_path`] that is not [`is_reserved_proxy_path`].
///
/// A per-path prefix under an application's own mount is not asked this:
/// `/<proxy-path>/sign-in` is inside that application and reserves nothing.
#[must_use]
pub fn is_mountable_proxy_path(text: &str) -> bool {
    is_proxy_path(text) && !is_reserved_proxy_path(text)
}

/// The most segments a per-path rule's prefix may name — `ADR-048` §4.
///
/// A bound rather than a taste: the prefix is compared segment by segment on
/// every request, and a manifest is bytes somebody else wrote. Eight is deeper
/// than any URL an application on this machine has asked to distinguish and is
/// still a number.
pub const MAX_PROXY_PREFIX_SEGMENTS: usize = 8;

/// **What a per-path prefix may be** — `ADR-048` §4.
pub const PROXY_PREFIX_RULE: &str = "a proxy auth prefix is one or more segments of lowercase \
                                     letters, digits and hyphens, each beginning with a letter, \
                                     joined by single slashes — no leading or trailing slash, and \
                                     no dot segment, so a rule cannot name anything outside the \
                                     application it belongs to";

/// Whether `text` is a prefix a manifest may attach an authentication rule to.
///
/// **Relative to the application's own [`is_proxy_path`] mount**, so `pair`
/// means `/<proxy-path>/pair` and never `/pair`. A leading slash is refused
/// rather than trimmed: the two spellings would be one rule with two texts, and
/// the canonical manifest may hold only one (`ADR-011`).
///
/// `..` and `.` cannot be spelled, because [`is_proxy_path`] requires every
/// segment to begin with a letter. That is what lets the matcher compare
/// segments without normalising anything.
#[must_use]
pub fn is_proxy_prefix(text: &str) -> bool {
    if text.is_empty() || text.len() > MAX_VALUE {
        return false;
    }
    let mut segments = 0usize;
    for segment in text.split('/') {
        segments += 1;
        if segments > MAX_PROXY_PREFIX_SEGMENTS || !is_proxy_path(segment) {
            return false;
        }
    }
    true
}

/// Byte 64 of a member record, for an `application`.
///
/// `ADR-015` §3's three parts, in the order they are written. The byte is
/// kind-defined — [`crate::package::update::TargetClass`] is the same byte
/// meaning something else — which is `ADR-029` §11's *"the interpretation of the
/// metadata, not its shape"* at its narrowest point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetClass {
    /// [`MANIFEST_NAME`]. Exactly one, and it is member 0.
    Manifest = 0,
    /// The static binary. Exactly one.
    ///
    /// **There is no `exec` field anywhere**, which is `ADR-015` §3.2's rule and
    /// its reason: *"a service definition that could name an arbitrary program
    /// would let a signed manifest launch something the signature never
    /// covered."* The program is the member with this class, and there is one.
    Program = 1,
    /// `ADR-015` §3's *"optional assets"*. Zero or more.
    Asset = 2,
}

impl TargetClass {
    #[must_use]
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Manifest),
            1 => Some(Self::Program),
            2 => Some(Self::Asset),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_byte(self) -> u8 {
        self as u8
    }
}

/// The variant byte an application's members must carry.
///
/// `ADR-029` §3 calls byte 65 `slot_variance` and
/// [`crate::package::read::Member::variant`] renames it, because *slot* is the
/// update kind's word. An application has no slots and no A/B, so **every
/// member's variant is zero and any other value is refused**. That is not
/// fussiness: the byte is half of a member's identity, so a builder that set it
/// could ship two members with the same name and no reader would have a rule for
/// choosing between them.
pub const VARIANT_NONE: u8 = 0;

// --- the manifest ----------------------------------------------------------

/// `ADR-015` §3.2's `kind` — what shape of program this is.
///
/// Not to be confused with [`Kind`], which is the *container's* kind. The
/// collision is `ADR-015` §3.2's spelling and is kept rather than improved,
/// because renaming a field an accepted ADR writes out in full would be this
/// module deciding something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramKind {
    Daemon,
    Oneshot,
    SocketActivated,
}

/// `ADR-015` §3.2's `restart`, and `ADR-017` §4.1's policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restart {
    Always,
    OnFailure,
    Never,
}

/// `ADR-015` §3.2's `ready`, and `ADR-017` §4.2's readiness contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ready {
    /// Started is ready. The supervisor waits for nothing.
    None,
    /// `ADR-017` §3.4's notify socket.
    Notify,
    /// Ready when the socket Rivet opened for it is being accepted on.
    Listening,
}

/// `ADR-015` §3.2's `network`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Network {
    /// `ADR-015` §5.1: *"an application with `network = "none"` has no
    /// interfaces at all"*.
    None,
    Egress,
    /// `ADR-015` §5.3's *"network groups — the pragmatic path"*: a namespace
    /// shared with the others in the named group.
    Group(String),
}

/// `ADR-015` §3.2's `state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateGrant {
    /// No writable directory at all.
    None,
    /// `/state/app/<name>`, mode 700 (`ADR-015` §2, §5.1).
    Private,
}

/// `ADR-015` §6.2's declared device capability — **kiosk mode**, 2026-08-31.
///
/// > An application that draws **declares it** in its manifest, beside
/// > `network` and `state`.
///
/// # Why a CLASS and never a path
///
/// A manifest is a permission request written by whoever built the artifact
/// (§3.3), so a key that took a device *path* would be a key that could ask for
/// `/dev/mem`. A class is a fixed vocabulary this build defines, and the paths
/// it covers are `crate::service::device_paths`'s — decided by Rivet, on the
/// machine, at the moment of the grant.
///
/// # Why this one is enforceable when most of §6's table is not
///
/// §6.2, measured: `/dev/fb0` is `rw------- root root`, and every
/// `/dev/input/event*` is the same (read off a board 2026-08-31). An
/// application runs as its own uid (§5.1.1), so it **already cannot open
/// them** — the kernel refuses with no Rivet code involved. That falls out of
/// `ADR-024`'s devtmpfs with **no udev**: nothing relaxes the kernel's default
/// the way a distribution's `0660 root:video` rule would.
///
/// So this needs no namespace, no cgroup and no socket passing. **A device node
/// is a filesystem object and Unix has arbitrated those since the beginning.**
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Device {
    /// The screen. `/dev/fb0`, and `/dev/dri/*` on a machine that has it.
    ///
    /// **Exclusive — one owner at a time, ruled** (§6.2 point 3). Two programs
    /// writing a framebuffer interleave, and with fbdev neither is wrong
    /// because there is no protocol to violate; refusing the second is the
    /// only place the question can be settled.
    Display,
    /// Keyboards, mice, touchscreens and gamepads. `/dev/input/*`.
    ///
    /// **Also exclusive, and that is a decision beyond what §6.2 ruled.** That
    /// section argues exclusivity for the display from interleaved *writes*,
    /// which does not carry: evdev broadcasts to every reader, so a second
    /// reader is not wrong in the way a second writer is — it is a keylogger,
    /// and §6.1 records that there is no isolation between applications to stop
    /// one. `ADR-015` §3.3's rule is that a default leans closed, and relaxing
    /// this later costs nothing while tightening it would take a capability
    /// away. **Flagged for the maintainer rather than assumed.**
    Input,
    /// **The GPU and not the screen** — `/dev/dri/renderD*`. `.rulings/` R-206
    /// D4, ruled 2026-09-20.
    ///
    /// A render node draws; it cannot modeset and cannot disturb whoever is
    /// scanning out. That is what makes it safe to declare on its own, and it
    /// is why this is a separate token rather than a mode of [`Self::Display`]:
    /// the whole of `ADR-058` is that an application does not get the screen,
    /// and drawing with the V3D changes nothing about that. An application
    /// holding this has no card node, cannot page-flip, and reaches the glass
    /// only by handing `rivet-displayd` a dmabuf.
    ///
    /// **`ADR-058` §3 emptied a lessee's device grants** on the reasoning
    /// that *"a lessee that does not talk to Mesa … needs no render node
    /// either"* — which is true, and conditional, and the condition stops
    /// holding for a pack that draws on the GPU. This token is that condition
    /// written as a declaration instead of left as an assumption.
    ///
    /// **NOT exclusive, unlike the two above**, and it is the first class that
    /// is not. A render node is opened by many processes at once by design — it
    /// is how every GL program on an ordinary system works — and neither
    /// argument for exclusivity carries here: there is no framebuffer whose
    /// writes would interleave, and no broadcast event stream to eavesdrop on.
    /// Two programs rendering is two programs rendering.
    Gpu,
}

impl Device {
    /// Every class, in the order a manifest writes them.
    pub const ALL: &'static [Device] = &[Device::Display, Device::Input, Device::Gpu];

    /// The token in a manifest.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Display => "display",
            Self::Input => "input",
            Self::Gpu => "gpu",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|d| d.as_str() == text)
    }

    /// Whether two services may hold this class at once.
    ///
    /// **[`Self::Gpu`] is the first class that answers `false`**, which is what
    /// this method was written in advance for (`.rulings/` R-206 D4). A render
    /// node has no framebuffer whose writes would interleave and no broadcast
    /// event stream to read, so neither of the arguments that make the other
    /// two exclusive applies to it.
    #[must_use]
    pub const fn exclusive(self) -> bool {
        match self {
            Self::Display | Self::Input => true,
            Self::Gpu => false,
        }
    }
}

impl core::fmt::Display for Device {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// **A kind of PER-MACHINE material an application needs before it can work** —
/// `ADR-055` §3.
///
/// # Why a kind and never a file list
///
/// The files of a kind travel together and an application holding two of three
/// serves nothing, so a per-file declaration would let a manifest ask for a
/// private key with no certificate — a state nobody designed. It also keeps the
/// *contents* of a kind the platform's: the day a fourth file joins the set,
/// every manifest already declaring the kind gets it, with no artifact rebuilt.
///
/// # Why it is declared rather than inferred
///
/// The same pair is a *client* certificate as well as a server one (`ADR-046`
/// §4), so a program that serves no route can need it and a program that
/// serves one can be satisfied without it. An inference from `proxy-path` would
/// be right four times in five and wrong in the fifth — the system asserting
/// something about an application that the application never said, which is
/// what `ADR-015` §3 forbids in the other direction.
///
/// [`Material::files`] is the list, and it is the same one
/// `cargo rivet secrets` installs and `ADR-049` §8 stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Material {
    /// The three TLS files a machine's applications are handed: the leaf the
    /// machine serves, its private key, and the fleet authority a caller is
    /// checked against.
    ///
    /// **One leaf per MACHINE and not per application.** Every backend answers
    /// to the machine's one fleet name, and the proxy verifies that one name
    /// against all of them (`ADR-046` §4), so two leaves for one machine
    /// would be `ADR-011` in the place it is hardest to debug.
    FleetTls,
}

impl Material {
    /// Every kind this build defines, in the order a manifest writes them.
    pub const ALL: &'static [Material] = &[Material::FleetTls];

    /// The token in a manifest.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FleetTls => "fleet-tls",
        }
    }

    /// The names the files land under in the application's data directory.
    ///
    /// **The platform's names, not the application's.** `rivet-fleetd` reads
    /// `server.crt`, `server.key` and `ca.crt` and is staged at card time by
    /// `ADR-049` §8 instead; widening this key to carry per-application names
    /// to satisfy that one case would let every other manifest ask for a
    /// private key under a name of its choosing.
    #[must_use]
    pub const fn files(self) -> &'static [&'static str] {
        match self {
            Self::FleetTls => &["tls-cert", "tls-key", "tls-root"],
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|m| m.as_str() == text)
    }
}

impl core::fmt::Display for Material {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// **Why a declared kind of material is not minted by a deploy** — `ADR-055`
/// §6.1 option B, `.rulings/` R-194 D1.
///
/// Quoted where an operator meets the refusal, so that the sentence lives once.
pub const MATERIAL_RULE: &str = "per-machine material is minted by the operator and never by a \
                                 deploy: a tool that opens the fleet authority on an ordinary run \
                                 is a tool that opens the fleet authority, and no later refusal \
                                 walks that back (ADR-055 §6)";

/// **Whether a route needs an operator's certificate** — `ADR-046` §4.3,
/// `R-152`.
///
/// `rivet-proxyd` terminates TLS for every application on the machine, so it is
/// the only place an operator is authenticated for HTTP. Which routes that
/// applies to is a property of the route, and — like every other column in
/// `crate::proxy`'s table — it is derived from the signed manifest rather than
/// configured. An application that wants to be public says so in the artifact
/// that was signed, and changing it means shipping a new one.
///
/// # Absence is [`ProxyAuth::Operator`], and that is the whole design of the key
///
/// `ADR-046` §4.3: *"a manifest key that must be present to **tighten**
/// security is a key an author forgets and a machine that is quietly open"*.
/// So this key only ever **relaxes**, its absence is the closed end of
/// `ADR-015` §3.3's axis, and every manifest emitted before 2026-09-05 means
/// *require an operator* without having said anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProxyAuth {
    /// The proxy refuses a caller that presents no certificate chaining to the
    /// fleet authority. **The default, and what an absent key means.**
    #[default]
    Operator,
    /// The route is served to anyone who can reach the machine's 443.
    ///
    /// `.rulings/` R-152's TempMon case: an application may be served over TLS
    /// without authenticating its callers. It is still TLS — the machine has one
    /// certificate and one front door — it is the *caller's* certificate that
    /// is not asked for.
    None,
    /// Forwarded exactly as [`Self::None`]: no caller certificate is asked for
    /// and the `Authorization` header passes through unchanged. **A per-path
    /// verdict only** — `ADR-048` §4, `ADR-061` §7, `.rulings/` R-221.
    ///
    /// What differs from [`Self::None`] is what the declaration *says*: the
    /// application checks a credential of its own at this prefix, so a reader
    /// auditing a manifest, the journal or the index can tell a public path
    /// from an application-authenticated one without reading the application.
    App,
    /// Forwarded exactly as [`Self::None`] on 443, **and also served over
    /// plain HTTP on port 80** to `GET` and `HEAD` only. **A per-path verdict
    /// only** — `ADR-048` §4, `ADR-061` §7, `.rulings/` R-221.
    ///
    /// The only thing that opens port 80: a machine with no `Plain` rule
    /// anywhere in its routing table binds no listener there at all.
    Plain,
}

impl ProxyAuth {
    /// The token in a manifest.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Operator => "operator",
            Self::None => "none",
            Self::App => "app",
            Self::Plain => "plain",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "operator" => Some(Self::Operator),
            "none" => Some(Self::None),
            "app" => Some(Self::App),
            "plain" => Some(Self::Plain),
            _ => None,
        }
    }

    /// Whether the proxy must refuse a caller with no operator certificate.
    ///
    /// **A method rather than a comparison at every call site**, because the
    /// question a proxy asks is *"do I refuse this?"* and a caller that spelled
    /// it `== ProxyAuth::Operator` would have to be revisited if a third value
    /// ever arrived. [`Self::App`] and [`Self::Plain`] answer `false`, exactly
    /// as [`Self::None`] does — both are forwarded with no caller certificate
    /// asked for.
    #[must_use]
    pub const fn requires_operator(self) -> bool {
        matches!(self, Self::Operator)
    }

    /// **Whether this verdict may answer for a WHOLE route** —
    /// `ADR-046` §4: the bare `proxy-auth` key's grammar is `operator` and
    /// `none` only. [`Self::App`] and [`Self::Plain`] exist to answer for a
    /// *prefix* (`ADR-048` §4): a route is what a proxy's index and journal
    /// summarise as gated-or-public, and an application-authenticated or
    /// plain-HTTP door is a property of one path under it, never of the whole
    /// surface.
    #[must_use]
    pub const fn allowed_for_whole_route(self) -> bool {
        matches!(self, Self::Operator | Self::None)
    }
}

impl core::fmt::Display for ProxyAuth {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// **One prefix under an application's route, and what it asks of a caller** —
/// `ADR-048` §4.
///
/// [`ProxyAuth`] answers for a whole application; this answers for a part of
/// one. A service that must show a gated page to an operator and an open door to
/// a phone declares one listening socket and says which prefix is which, rather
/// than opening a second socket the proxy does not cover.
///
/// # Both directions, and only one of them is dangerous
///
/// A rule may **relax** ([`ProxyAuth::None`]) or **tighten**
/// ([`ProxyAuth::Operator`]) relative to the application's own
/// [`Manifest::proxy_auth`]. Tightening is free to get wrong — the worst
/// outcome is a refused request. Relaxing is the direction that costs the
/// machine, which is why every default in this file leans the other way and why
/// the matcher compares whole segments rather than characters: an open
/// `reliquary/pair` must not also open `reliquary/pairings`.
///
/// # It is inside the signature, like every other permission request
///
/// An application *asks* to be reachable without a caller certificate; an
/// operator cannot *grant* it. Changing which prefix is open means shipping a
/// new artifact that somebody signed, which is what makes a relaxation
/// something no text edit on the machine can produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyRule {
    auth: ProxyAuth,
    prefix: String,
}

impl ProxyRule {
    /// A rule, or `None` if `prefix` is not an [`is_proxy_prefix`].
    #[must_use]
    pub fn new(auth: ProxyAuth, prefix: &str) -> Option<Self> {
        is_proxy_prefix(prefix).then(|| Self {
            auth,
            prefix: prefix.to_string(),
        })
    }

    /// `<auth>:<prefix>`, as a manifest and the routing table both spell it.
    ///
    /// **The verdict comes first**, matching `tcp:8080` and `photo-index:api`:
    /// the discriminant leads, so a reader scanning a column sees what a rule
    /// *does* before they see what it applies to.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (auth, prefix) = text.split_once(':')?;
        Self::new(ProxyAuth::parse(auth)?, prefix)
    }

    #[must_use]
    pub fn auth(&self) -> ProxyAuth {
        self.auth
    }

    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// How many segments the prefix names — the depth a longest-match compares.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.prefix.split('/').count()
    }
}

impl core::fmt::Display for ProxyRule {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}:{}", self.auth, self.prefix)
    }
}

/// **What a backup or restore path may be** — `ADR-051`.
///
/// The same grammar as [`PROXY_PREFIX_RULE`], and deliberately the same
/// predicate: a backup route is a prefix under this application's own
/// `proxy-path`, exactly as a per-path authentication rule is. Two rules about
/// two things that happen to agree would drift; one rule cannot. `restore-path`
/// (`ADR-051` §12) takes it a third time, for the same reason.
pub const BACKUP_PATH_RULE: &str = "a backup or restore path is one or more segments of lowercase \
                                    letters, digits and hyphens, each beginning with a letter, \
                                    joined by single slashes, and it is RELATIVE to this \
                                    application's proxy-path — no leading or trailing slash, and \
                                    no dot segment";

/// **Why a `restore-path` on an open route is refused** — `ADR-051` §12.
///
/// A restore REPLACES what an application holds. `rivet-proxyd` is where a
/// machine authenticates an HTTP caller (`ADR-046` §4), so a restore route it
/// serves to anybody would let anybody on the network replace an application's
/// data. The manifest refuses the declaration rather than
/// trusting every application to gate the write itself.
pub const RESTORE_AUTH_RULE: &str = "a restore path must require the operator's certificate: \
                                     proxy-auth must be operator (its default) at that path, or \
                                     a proxy-auth-paths rule `operator:<prefix>` must cover it, \
                                     because a restore replaces the application's data and \
                                     rivet-proxyd is where the caller is authenticated";

/// **What one fetch of a backup returns** — `ADR-051`.
///
/// A FORMAT and never a schema. The maintainer's line, 2026-09-08
/// (`.rulings/` R-306): whether a file is a well-formed database or a
/// well-formed CSV is as far as a client needs to go.
///
/// A file format is stable and a schema is not, so a check on the format cannot
/// go stale while a check on the tables goes stale at every migration. And the
/// format is **declared by the application** rather than known by whatever
/// fetches it, which is what makes an application migrating CSV to SQLite carry
/// its own answer across the version boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupFormat {
    /// A SQLite database file. `SQLite format 3\0`, and a header that says how
    /// long the whole file is.
    Sqlite,
    /// Comma-separated text. **No magic and no length claim** — see
    /// `crate::backup::validate`, which says so rather than pretending.
    Csv,
    /// One JSON document.
    Json,
    /// A POSIX tar archive.
    Tar,
}

impl BackupFormat {
    /// Every format, in the order a manifest may write them.
    pub const ALL: &'static [BackupFormat] = &[
        BackupFormat::Sqlite,
        BackupFormat::Csv,
        BackupFormat::Json,
        BackupFormat::Tar,
    ];

    /// The token in a manifest and in the derived table.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Csv => "csv",
            Self::Json => "json",
            Self::Tar => "tar",
        }
    }

    /// What a stored file is called, after the dot.
    ///
    /// **Not `db` for SQLite**, because the extension is what a person sorting
    /// a directory reads and `sqlite` says which of several things a `.db` is.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Csv => "csv",
            Self::Json => "json",
            Self::Tar => "tar",
        }
    }

    /// **The `Content-Type` `cargo rivet restore` sends this format with** —
    /// `ADR-051` §12's contract.
    ///
    /// Sent so that a handler CAN look, and stated in the contract as
    /// something a handler must not rely on: what an application checks is the
    /// body, because a header is a claim and the body is the thing.
    /// `application/vnd.sqlite3` is the registered type, and the one TempMon's
    /// own `GET /api/v1/backup` already answers with.
    #[must_use]
    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Sqlite => "application/vnd.sqlite3",
            Self::Csv => "text/csv",
            Self::Json => "application/json",
            Self::Tar => "application/x-tar",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|f| f.as_str() == text)
    }
}

impl core::fmt::Display for BackupFormat {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// **The backup surface an application offers, and there are two shapes** —
/// `ADR-051`.
///
/// # They are two keys because they are a PROTOCOL difference
///
/// Not a format difference. [`Self::Artifact`] is *one route, one fetch, one
/// whole thing*; [`Self::Feed`] is *a cursor, a low-water mark and items that
/// arrive one at a time*. A client written for one cannot read the other by
/// changing a string, so a single key with four artifact values and a fifth
/// meaning *actually, resume* would be a value that changes what every other
/// field means.
///
/// # Which is why this is an enum and the manifest refuses both
///
/// The mutual exclusion is structural here rather than checked: there is no
/// value of this type that carries a format and a feed at once, so no code
/// downstream has to decide which one wins.
///
/// # `restore-path` lives INSIDE the artifact shape, and that is the rule
///
/// `ADR-051` §12. A restore sends back the bytes one fetch of the artifact
/// returned, and `cargo rivet restore` checks them against `backup-format`
/// before it sends anything. A restore with no artifact beside it has no format
/// to check against, and a feed has none at all — so the field is on
/// [`Self::Artifact`] and nowhere else, and no value of this type can carry a
/// restore without the format it is checked against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupSurface {
    /// `backup-path` and `backup-format` — one fetch returns the whole thing —
    /// and optionally `restore-path`, where the application accepts that thing
    /// back.
    Artifact {
        path: String,
        format: BackupFormat,
        /// `restore-path`, relative to `proxy-path`. `None` is *this
        /// application accepts no restore*, which is what every manifest
        /// emitted before the key existed says.
        restore: Option<String>,
    },
    /// `backup-feed` — a resumable collection, fetched by cursor.
    Feed { path: String },
}

impl BackupSurface {
    /// The route, relative to the application's `proxy-path`.
    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            Self::Artifact { path, .. } | Self::Feed { path } => path,
        }
    }

    /// The restore route, relative to the application's `proxy-path`, or
    /// `None` when the application accepts no restore — `ADR-051` §12.
    #[must_use]
    pub fn restore_path(&self) -> Option<&str> {
        match self {
            Self::Artifact { restore, .. } => restore.as_deref(),
            Self::Feed { .. } => None,
        }
    }

    /// The format one fetch returns, or `None` for a feed.
    ///
    /// **`None` is not *unknown*, it is *not that shape*.** A feed's items are
    /// named by a digest of themselves, so there is no format to validate —
    /// see `crate::backup::validate`.
    #[must_use]
    pub fn format(&self) -> Option<BackupFormat> {
        match self {
            Self::Artifact { format, .. } => Some(*format),
            Self::Feed { .. } => None,
        }
    }
}

/// `ADR-015` §3.2's `memory` — **a request, and a string.**
///
/// # There is already an authority for this and it is in another crate
///
/// `rivet_config::services::MemoryLimit` parses the same grammar, **rules the
/// multipliers binary** — *"counting in multiples of 1024"* — and holds a `u64`.
/// That is the operator's side of the maintainer's 2026-08-24 ruling, and it is a real
/// answer to the still-open unit question, already in the tree.
///
/// **This type does not restate it, because restating it is the drift
/// `ADR-011` exists to prevent.** `rivet-core` may not depend on
/// `rivet-config` — `ADR-014` §3.2 makes the edge run the other way and calls
/// it permanent — so the multiplier cannot be *reached* from here, and copying
/// it would put a second spelling of `1024` in a crate that cannot be told when
/// the first one changes. The fix is a standing recommendation, still
/// unruled, which this is now a second argument for: move the schema
/// types down into `rivet-core` and let both sides name one `MemoryLimit`.
///
/// Until then this type validates the *shape* and carries the text, and the
/// conversion is deliberately absent rather than merely unwritten. No formal
/// ruling settles whether `ADR-015` §3.2's
/// `128M` is binary or decimal, and the maintainer's 2026-08-24 ruling makes the
/// operator's cap the authority with **the lower of the two winning** — a
/// comparison that cannot be made until the unit is ruled. A parser that guessed
/// would not fail; it would succeed, at 128 000 000 or 134 217 728, and the
/// difference is 6 MiB of somebody's service (`AGENTS.md` §5).
///
/// So this type validates the *shape* — decimal digits then an optional single
/// `K`, `M` or `G` — and carries the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Memory(String);

impl Memory {
    /// Validate the shape and keep the text.
    pub fn parse(text: &str) -> Result<Self, ManifestFault> {
        let (digits, suffix) = match text.as_bytes().last() {
            Some(b'K' | b'M' | b'G') => (&text[..text.len() - 1], true),
            _ => (text, false),
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ManifestFault::BadValue { key: "memory" });
        }
        // A leading zero is refused for the reason two spellings of a version
        // are: `0128M` and `128M` would be the same request written two ways,
        // and the artifact is a build product with no excuse for either.
        if digits.len() > 1 && digits.starts_with('0') {
            return Err(ManifestFault::BadValue { key: "memory" });
        }
        let _ = suffix;
        Ok(Self(text.to_string()))
    }

    /// The text, exactly as it was written. **Never a byte count** — see the
    /// type's own documentation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `ADR-015` §3's `cpu`, added 2026-09-09 — a request, and a string, on
/// [`Memory`]'s footing exactly.
///
/// # This mirrors `Memory`, and for the identical reason
///
/// `rivet_config::services::CpuLimit` — really `rivet_core::service::
/// CpuLimit`, re-exported there — parses the same decimal-cores grammar and
/// holds a number of microseconds against a fixed period. That is the
/// operator's side of `ADR-007` §7 and `ADR-015` §3's 2026-09-09
/// amendment, which gave `cpu` the same split `memory` already has: the
/// manifest's is a request, the document's is a cap, and the lower wins.
///
/// **This type does not restate the arithmetic, for `Memory`'s reason.**
/// `rivet-core` may not depend on `rivet-config` (`ADR-014` §3), so the
/// conversion cannot be reached from here even though — unlike `Memory`'s
/// binary-vs-decimal question — there is no open question about what `cpu`
/// means: a decimal number of cores has one reading. What is missing here is
/// not a ruling, only the crate that is allowed to own the arithmetic; see
/// [`crate::service::CpuLimit`] for where it lives.
///
/// So this type validates the *shape* — decimal digits, an optional single
/// `.` followed by more decimal digits — and carries the text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cpu(String);

impl Cpu {
    /// Validate the shape and keep the text.
    ///
    /// The shape [`crate::service::CpuLimit::parse`] accepts, restated as a
    /// grammar rather than re-derived as arithmetic — this function refuses
    /// nothing that parser would also refuse, and a fixture in
    /// `rivet-core/src/service.rs` holds the two together the way
    /// `every_shape_the_artifact_accepts_is_one_this_module_can_value` does
    /// for `Memory`.
    pub fn parse(text: &str) -> Result<Self, ManifestFault> {
        let (whole, frac) = match text.split_once('.') {
            Some((w, f)) => (w, Some(f)),
            None => (text, None),
        };
        if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ManifestFault::BadValue { key: "cpu" });
        }
        // The leading-zero refusal `Memory::parse` makes about `0128M`, here
        // about `02`: one number, one spelling.
        if whole.len() > 1 && whole.starts_with('0') {
            return Err(ManifestFault::BadValue { key: "cpu" });
        }
        if let Some(f) = frac {
            if f.is_empty() || f.len() > 5 || !f.bytes().all(|b| b.is_ascii_digit()) {
                return Err(ManifestFault::BadValue { key: "cpu" });
            }
            // `0.50` and `0.5` would otherwise be two spellings of one value,
            // and `2.0` a second spelling of `2` — the same canonicality
            // argument as the leading zero above, applied to the other end
            // of the number.
            if f.ends_with('0') {
                return Err(ManifestFault::BadValue { key: "cpu" });
            }
        }
        Ok(Self(text.to_string()))
    }

    /// The text, exactly as it was written. **Never a quota** — see the
    /// type's own documentation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `ADR-015` §3.2's manifest, and §3.3's permission request — they are the same
/// document, which is the point:
///
/// > There is no second permissions file. What an application declares is what
/// > it is asking for.
///
/// **What is here and what is not is the maintainer's 2026-08-24 ruling:**
/// *"the manifest owns what the application **is** — restart policy, readiness
/// contract, sockets. The document owns what the operator **limits** — the
/// memory cap."* The direction is the test, and `ADR-015` §4.1 is why: this
/// document is inside the signature, so behaviour cannot be rewritten outside
/// it; a cap runs the other way and is `ADR-007` §7.2's to grant.
///
/// [`Self::memory`] is on the manifest side of that line only as a *request* —
/// see [`Memory`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    kind: ProgramKind,
    restart: Restart,
    ready: Ready,
    network: Network,
    state: StateGrant,
    memory: Option<Memory>,
    listen: Vec<Socket>,
    provides: Vec<Endpoint>,
    connects: Vec<Endpoint>,

    /// **What a person should call this**, or `None` for *use the service name*.
    ///
    /// `PLAN.md` §104.42. The maintainer's reason (`.rulings/` R-360): an
    /// executable's name is not always what should be shown to a person, because
    /// the two seldom coincide. `aura-renderer` is what a supervisor starts;
    /// **Aura** is what the thing is.
    ///
    /// It is in the MANIFEST and therefore inside the signature, so a name on a
    /// screen is one the author of the artifact chose — not one an operator
    /// could set to something else after the fact.
    title: Option<String>,
    /// **The settings this application understands** — `ADR-038` §3, by name.
    ///
    /// Empty is *accepts none*, and absent renders as empty: unlike
    /// [`Self::title`] there is no third state to distinguish, because an
    /// application that declared an empty list and one that declared nothing
    /// both take no settings.
    ///
    /// **Keys ship with the application; VALUES do not** (`ADR-038` §3.3).
    /// Adding a setting is a change to the contract and rides under the
    /// signature; changing what it is set to is an operator's act on one
    /// machine and must not require rebuilding anything.
    settings: Vec<String>,
    /// **Where `rivet-proxyd` mounts this application** — `R-133`, 2026-09-03.
    ///
    /// One path segment, no slashes: `reliquary` becomes
    /// `https://<host>/reliquary/`. `None` means NOT PROXIED, and the
    /// application is reached on its own port exactly as before.
    ///
    /// **It is a request, not a grant.** Two applications may ask for one
    /// path; the manifest is signed and an operator cannot edit it, so the
    /// collision is resolved where the routing table is built rather than
    /// here.
    proxy_path: Option<String>,
    /// **Whether that route needs an operator's certificate** — `ADR-046` §4.3.
    ///
    /// Meaningless without [`Self::proxy_path`], and [`Manifest::render`] does
    /// not write it without one — a manifest that relaxed a route it does not
    /// have would be a relaxation waiting to become live the day somebody added
    /// a path.
    proxy_auth: ProxyAuth,
    /// **What individual prefixes under that route ask of a caller** —
    /// `ADR-048` §4.
    ///
    /// Empty is the ordinary answer and means *the whole route takes
    /// [`Self::proxy_auth`]*, which is every manifest emitted before this key
    /// existed. Held sorted by prefix and deduplicated by
    /// [`Manifest::with_proxy_auth_paths`], so two orderings of one intent
    /// cannot produce two artifacts and no prefix can carry two verdicts.
    ///
    /// Meaningless without [`Self::proxy_path`], for [`Self::proxy_auth`]'s
    /// reason, and refused there rather than ignored.
    proxy_auth_paths: Vec<ProxyRule>,
    /// **The declared closure** — Shared Rivets, 2026-08-31.
    ///
    /// Every library artifact this program needs, by
    /// [`crate::package::library::Reference`], which is content-addressed and
    /// therefore names exact bytes rather than a version somebody may resolve
    /// differently later.
    ///
    /// It is inside the signature for `ADR-015` §4.1's reason, and the reason is
    /// sharper here than for any other key: a closure that could be rewritten
    /// outside the signature would let somebody point a correctly-signed program
    /// at code its author never saw.
    library: Vec<library::Reference>,
    /// **The device classes this application asks for** — `ADR-015` §6.2.
    ///
    /// Inside the signature, like every other permission request: a capability
    /// that could be added outside it would let somebody hand a
    /// correctly-signed program the screen its author never asked for.
    ///
    /// Held sorted and deduplicated by [`Manifest::with_devices`], so two
    /// spellings of one request cannot produce two artifacts.
    devices: Vec<Device>,
    /// **The backup surface this application offers** — `ADR-051`.
    ///
    /// `None` is the ordinary answer and means *this application offers
    /// nothing to back up*, which is what every manifest emitted before
    /// 2026-09-08 says. It is the closed end of `ADR-015` §3's axis: an
    /// application that says nothing is not fetched from.
    ///
    /// **Meaningless without [`Self::proxy_path`]**, for [`Self::proxy_auth`]'s
    /// reason and more sharply: a backup surface is a ROUTE, and a route with
    /// no mount is a declaration that names nothing anything can reach.
    /// [`Manifest::parse`] refuses the pair rather than ignoring it, which is
    /// the same defect as `SPEC-05` §5's `rivet-proxyd` enabled with no
    /// `upstream-name`.
    backup: Option<BackupSurface>,
    /// **`ADR-015` §3's `cpu` request, added 2026-09-09** — [`Cpu`]'s
    /// footing exactly matches [`Self::memory`]'s: `None` is the ordinary
    /// answer and means *this application has asked for no particular share
    /// of the processor*, which is what every manifest emitted before today
    /// says. `rivet_core::service::effective_cpu` reconciles it with the
    /// operator's cap, on [`Self::memory`]'s [`crate::service::
    /// effective_memory`] footing.
    cpu: Option<Cpu>,
    /// **The classes of library this application accepts** —
    /// `PLAN-Feature-Set-2026-09-12` §6.2, `ADR-053`.
    ///
    /// **A list**, since `.rulings/` R-162 R28 — reversing `ADR-053` §4's
    /// original "one class, not a list": no signed manifest carried the key
    /// before the ruling, so the list form is what every manifest ships from
    /// here on, with no compatibility shim for the single-string form it
    /// replaces. An application joins a library whose `provides-class` is
    /// ANY of these.
    ///
    /// `Vec::new()` is the ordinary answer and means *this application
    /// takes no addon of any class*, which is what every manifest emitted
    /// before `ADR-053` says and what an application with only an explicit
    /// [`Self::library`] closure still says. It is the closed end of
    /// `ADR-015` §3's permission-request axis, on [`Self::proxy_path`]'s
    /// footing: a capability
    /// a manifest could gain by an install elsewhere on the machine would be
    /// a capability outside the signature, so the machine only ever widens
    /// what is available to a program that asked for it here, inside the
    /// signature.
    accepts_class: Vec<String>,
    /// **The per-machine material this application needs before it can work** —
    /// `ADR-055` §3, [`Material`].
    ///
    /// `Vec::new()` is the ordinary answer and means *this application needs
    /// nothing placed on the machine for it*, which is what every manifest
    /// emitted before `ADR-055` says. It is the closed end of `ADR-015` §3's
    /// permission-request axis, like every other optional key here: an absent
    /// declaration asks for nothing, and deploy provides nothing and says
    /// nothing.
    ///
    /// Held sorted and deduplicated by [`Manifest::with_material`], so two
    /// spellings of one request cannot produce two artifacts.
    material: Vec<Material>,
}

/// One entry of `ADR-015` §3.2's `listen` — a socket **Rivet** opens and
/// passes in.
///
/// `ADR-015` §5.2: *"rather than granting `CAP_NET_BIND_SERVICE`, Rivet opens
/// the listening socket and passes the file descriptor to the application"*. So
/// this is a request for a descriptor, not a permission to bind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Socket {
    /// `tcp:<port>`, 1..=65535. Bound on `127.0.0.1` for a proxied
    /// application and on every address otherwise — [`Socket::bind_address`].
    Tcp(u16),
    /// `unix:<name>` — a name, never a path. `ADR-015` §5.3 has Rivet create
    /// it and bind-mount it, so where it lives is not the application's to say.
    Unix(String),
    /// `udp:<port>`, 1..=65535 — `PLAN.md` §150.1, 2026-09-06.
    ///
    /// **A plain datagram socket and nothing more.** `bind()` only, exactly
    /// like [`Socket::Tcp`] — no `SO_BROADCAST`, no `IP_PKTINFO`, no socket
    /// option this build sets on the application's behalf. `PLAN.md` §150.1
    /// found a service that would have wanted those (a DHCP responder replying
    /// to a client with no address yet) and the maintainer dropped it rather
    /// than teach the supervisor a second, protocol-shaped path — `ADR-017`
    /// §4.1 keeps the supervisor from learning what a socket is *for*. A
    /// consumer that needs interface metadata or a broadcast reply binds its
    /// own socket; this variant is for the ordinary case, a service that reads
    /// a datagram and replies to its sender, which is what named DNS needs.
    Udp(u16),
    /// `tcp:<port>:direct` — [`Socket::Tcp`], declared reachable without
    /// `rivet-proxyd` by an application that has a `proxy-path` (`ADR-046` §7,
    /// `.rulings/` R-188).
    ///
    /// **A separate variant rather than a field on `Tcp`**, so that every
    /// exhaustive match in the tree had to be told about it: a match that
    /// silently treated a direct port as an ordinary one would bind it where
    /// the signature said not to, or route to it where [`DIRECT_RULE`] refuses.
    /// [`Socket::port`] and [`Socket::is_tcp`] are for a caller that wants the
    /// number and not the reach.
    TcpDirect(u16),
    /// `udp:<port>:direct` — [`Socket::TcpDirect`]'s declaration for a
    /// datagram socket. Nothing proxies UDP, so no UDP port is ever a route's
    /// backend; the flag exists so that one rule binds every declared port of a
    /// proxied application, whatever its transport ([`Socket::bind_address`]).
    UdpDirect(u16),
}

/// **Why a `:direct` flag can be refused** — `ADR-046` §7, `.rulings/` R-188.
///
/// Quoted by `cargo rivet package`, which is where an author meets it.
pub const DIRECT_RULE: &str = "`:direct` marks a port of a PROXIED application (one declaring \
                               proxy-path) that must be reached without rivet-proxyd, such as a \
                               phone's listener. It is refused on the route's backend — the \
                               first tcp port — because a routed port reachable directly \
                               bypasses the proxy's operator gate and cross-site guard; and it is \
                               refused without a proxy-path, where every port already binds on \
                               every address";

impl Socket {
    /// The port, for a TCP or UDP socket whatever its reach. `None` for a unix
    /// socket.
    #[must_use]
    pub fn port(&self) -> Option<u16> {
        match self {
            Self::Tcp(port) | Self::TcpDirect(port) | Self::Udp(port) | Self::UdpDirect(port) => {
                Some(*port)
            }
            Self::Unix(_) => None,
        }
    }

    /// Whether this is a TCP socket, either reach.
    #[must_use]
    pub fn is_tcp(&self) -> bool {
        matches!(self, Self::Tcp(_) | Self::TcpDirect(_))
    }

    /// Whether the manifest declared this socket `:direct`.
    #[must_use]
    pub fn is_direct(&self) -> bool {
        matches!(self, Self::TcpDirect(_) | Self::UdpDirect(_))
    }

    /// **Where `rivet-sysd` binds this socket** — `ADR-046` §7, `.rulings/`
    /// R-188. `None` for a unix socket, which has a path and no address.
    ///
    /// `proxied` is whether the application's signed manifest declares a
    /// `proxy-path` ([`Manifest::proxied`]). The rule, and it is the whole of
    /// it:
    ///
    /// * **A proxied application's port binds `127.0.0.1`**, unless it is
    ///   `:direct`. `rivet-proxyd` dials loopback, so the route still reaches
    ///   it; nothing off the machine does, so the proxy's operator gate and
    ///   cross-site guard cannot be walked around.
    /// * **A `:direct` port binds every address.** [`DIRECT_RULE`] keeps the
    ///   flag off the route's backend.
    /// * **An application with no `proxy-path` binds every address**, as every
    ///   declared port did before R-188. It has no gate to bypass — its own
    ///   port is its only door — and `rivet-proxyd` (443) and `rivet-fleetd`
    ///   (4443) are two such applications, so binding loopback here would shut
    ///   the operator out of both on any machine running a `.rvt` built before
    ///   the flag existed.
    #[must_use]
    pub fn bind_address(&self, proxied: bool) -> Option<std::net::Ipv4Addr> {
        match self {
            Self::Unix(_) => None,
            Self::Tcp(_) | Self::Udp(_) if proxied => Some(std::net::Ipv4Addr::LOCALHOST),
            Self::Tcp(_) | Self::Udp(_) | Self::TcpDirect(_) | Self::UdpDirect(_) => {
                Some(std::net::Ipv4Addr::UNSPECIFIED)
            }
        }
    }

    /// The manifest's own spelling — `tcp:8443:direct`, `unix:api`.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::Tcp(port) => format!("tcp:{port}"),
            Self::TcpDirect(port) => format!("tcp:{port}:direct"),
            Self::Udp(port) => format!("udp:{port}"),
            Self::UdpDirect(port) => format!("udp:{port}:direct"),
            Self::Unix(name) => format!("unix:{name}"),
        }
    }
}

/// One entry of `provides` or `connects` — `<owner>:<name>` for a connection,
/// `socket:<name>` for something provided.
///
/// `ADR-015` §5.3: an application declares `provides = ["socket:api"]` and
/// another declares `connects = ["photo-index:api"]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    owner: String,
    name: String,
    shared: bool,
}

impl Endpoint {
    /// `socket` for a `provides`, or the providing application's name for a
    /// `connects`.
    #[must_use]
    pub fn owner(&self) -> &str {
        &self.owner
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// **Whether MANY applications may connect to this endpoint at once** —
    /// `provides = ["socket:<name>:shared"]`, added 2026-09-20 for
    /// `.rulings/` R-206.
    ///
    /// # Why an endpoint needs this at all
    ///
    /// `ADR-015` §5 grants an endpoint to **one** consumer, and the grant is
    /// a `chown` — so one is not a policy, it is what the mechanism can
    /// express. That is right for almost everything here: `aura-controller`'s
    /// `kiosk.sock` carries one renderer's state, and a second reader of it
    /// would be a bug nobody declared.
    ///
    /// **It is wrong for a compositor, and that is the whole of `ADR-058`.**
    /// A compositor exists so that several applications hold surfaces and it
    /// arbitrates between them; an endpoint that only one application may
    /// reach makes that impossible by construction. Found on `rivet-01`,
    /// 2026-09-20, when the second application ever to want a surface was
    /// refused by name:
    ///
    /// ```text
    /// gpu-surface wants rivet-displayd:wayland, which is already granted to
    /// aura-renderer. An endpoint has one consumer: the grant is a chown
    /// ```
    ///
    /// **Nobody had noticed because the only other client was root.**
    /// `rivet-launcher` is an image service and runs as uid 0, so it connects
    /// through a `0600` socket owned by somebody else without ever testing the
    /// rule. The first application with an
    /// identity of its own found it immediately.
    ///
    /// # What it does NOT do
    ///
    /// **It is not `0666` and the declaration is still enforcement.** A shared
    /// socket is `root:<endpoint gid>` at `0660`, and a consumer is spawned
    /// with that gid as its one supplementary group *because its manifest
    /// declared the endpoint and the supervisor granted it*. An application
    /// that did not declare it still cannot connect. `crate::service`'s
    /// `endpoint_gid` derives the group and `cmd/rivet-sysd` applies it.
    ///
    /// **The provider decides, not the consumer.** `shared` is meaningful only
    /// on a `provides`; a `connects` that carries it is refused, because
    /// whether an endpoint can take two readers is a fact about what is
    /// listening and never about who is asking.
    #[must_use]
    pub const fn shared(&self) -> bool {
        self.shared
    }
}

/// Why a manifest could not be read.
///
/// Every variant names the key or the line, per `AGENTS.md` §5 — a build tool
/// that says "bad manifest" sends its reader back to the file with nothing to
/// look for. **No variant carries a value**, deliberately: a manifest is a
/// build product and its faults are the emitter's, so the key is the useful
/// half; and `AGENTS.md` §8.3 binds anything that reaches a console capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestFault {
    /// Bytes that are not UTF-8, or that carry a byte no manifest may hold.
    NotText,
    /// A line that is neither blank, nor a comment, nor `key = value`.
    NotKeyValue,
    /// A key this build does not define.
    UnknownKey,
    /// A key that must be present and is not.
    MissingKey { key: &'static str },
    /// A key appearing after a key that must follow it, or twice. The canonical
    /// order is [`Manifest::render`]'s, and a manifest is emitted rather than
    /// written, so anything else is a second producer.
    OutOfOrder { key: &'static str },
    /// A value that is not one of the words this key admits.
    BadValue { key: &'static str },
    /// A value outside `0x21..=0x7e`, or holding a quote or a backslash. The
    /// emitter has no escapes, so the parser has none to get wrong.
    BadCharacter,
    /// A scalar or an element longer than [`MAX_VALUE`].
    ValueTooLong,
    /// A `proxy-path` in [`RESERVED_PROXY_PATHS`] — `ADR-046` §4.
    ReservedProxyPath,
    /// More than [`MAX_LIST`] entries.
    ListTooLong { key: &'static str },
    /// A declared `settings` name that reads like a credential —
    /// [`is_credential_like_name`], `ADR-038` §5, `.rulings/` R-169.
    CredentialLikeSetting,
    /// Longer than [`MAX_MANIFEST_BYTES`].
    TooLarge,
}

impl core::fmt::Display for ManifestFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotText => write!(f, "it is not printable text"),
            Self::NotKeyValue => write!(f, "a line is not `key = value`"),
            Self::UnknownKey => write!(f, "a key is not one this build defines"),
            Self::MissingKey { key } => write!(f, "the {key} key is absent and is required"),
            Self::OutOfOrder { key } => write!(
                f,
                "the {key} key repeats or is out of the canonical order a manifest is emitted in"
            ),
            Self::BadValue { key } => write!(f, "the {key} key's value is not one it admits"),
            Self::BadCharacter => write!(f, "a value holds a character no manifest may carry"),
            Self::ValueTooLong => write!(f, "a value is longer than {MAX_VALUE} bytes"),
            Self::ReservedProxyPath => f.write_str(RESERVED_PROXY_PATH_RULE),
            Self::ListTooLong { key } => write!(f, "the {key} list is longer than {MAX_LIST}"),
            Self::CredentialLikeSetting => f.write_str(CREDENTIAL_SETTING_RULE),
            Self::TooLarge => write!(f, "it is longer than {MAX_MANIFEST_BYTES} bytes"),
        }
    }
}

/// The keys, in the order [`Manifest::render`] writes them and
/// [`Manifest::parse`] demands them.
const KEYS: [&str; 23] = [
    "kind", "restart", "ready", "network", "state", "memory", "listen", "provides", "connects",
    // **Shared Rivets, 2026-08-31.** Last, so that every manifest emitted before
    // this date parses under the ordering rule unchanged — the canonical order
    // is this array and a key appended to it cannot make an older manifest
    // out-of-order. A key inserted in the middle would have.
    //
    // It is still a REQUIRED key, so an older manifest is refused for the
    // separate reason that it is missing. That is deliberate and it is
    // `ADR-015` §4.2.1's migration argument again: an application installed
    // before a check existed is reinstalled rather than grandfathered, because
    // a closure that may be absent is a closure nothing can rely on.
    "library",
    // **`ADR-015` §6.2's declared device capability, 2026-08-31.** Appended for
    // `library`'s reason: the canonical order is this array, and a key added to
    // the end cannot make an older manifest out-of-order. It is REQUIRED, so an
    // older manifest is refused for being incomplete rather than accepted with
    // a guess — a capability that defaulted to absent would be indistinguishable
    // from one an author never considered.
    "devices",
    // **A name for a person, 2026-09-01** — `PLAN.md` §104.42, and appended for
    // `library`'s reason.
    //
    // OPTIONAL, unlike the two above, and that is the difference worth stating:
    // a closure or a capability that defaulted to absent would be
    // indistinguishable from one an author never considered, and both are
    // security-relevant. A missing title is neither — it means *call it by its
    // service name*, which is what every screen did before this key existed.
    "title",
    // **`ADR-038` §3's declaration, 2026-09-02** — the settings this
    // application understands, by name. Appended for `library`'s reason: the
    // canonical order is this array, and a key at the end cannot make an older
    // manifest out-of-order.
    //
    // **OPTIONAL, like `title` and unlike `library` and `devices`.** Those two
    // are security-relevant, so an absent one would be indistinguishable from
    // one an author never considered and both had to be required — which is
    // why every package had to be rebuilt when they arrived. This is not: an
    // application that declares no settings **accepts none**, which is the
    // closed end of the axis (`ADR-015` §3.3) and is what every application
    // did before this key existed. So no existing `.rvt` is invalidated.
    "settings",
    // **`R-133`'s path route, 2026-09-03** — where `rivet-proxyd` mounts this
    // application under the machine's one name. Appended for `library`'s
    // reason: the canonical order is this array, and a key at the end cannot
    // make an older manifest out-of-order.
    //
    // **OPTIONAL, and its absence means *not proxied*** — the closed end of
    // `ADR-015` §3.3's axis, like `settings` and unlike `library` and
    // `devices`. An application that says nothing is reached on its own port
    // exactly as it is today, so no existing `.rvt` is invalidated and nothing
    // is mounted by accident.
    //
    // **It is a REQUEST and the document grants it.** Two applications may
    // both ask for `/aura`; the manifest is signed and an operator cannot edit
    // it, so the collision is resolved where every other grant is — see
    // `crate::proxy`, which refuses the pair by name rather than picking one.
    "proxy-path",
    // **`R-152`'s gate, 2026-09-05** — whether that route needs an operator's
    // certificate. Appended for `library`'s reason: the canonical order is this
    // array, and a key at the end cannot make an older manifest out-of-order.
    //
    // **OPTIONAL, and its absence means REQUIRE AN OPERATOR** — which is the
    // opposite direction to every other optional key here and is the point of
    // the key. `ADR-046` §4.3: a key that must be present to *tighten* security
    // is one an author forgets, and a machine that is then quietly open. So
    // this one only ever relaxes, and no manifest emitted before today becomes
    // less safe by being read by a build that understands it.
    //
    // Its only value is therefore `none`; see `ProxyAuth`.
    "proxy-auth",
    // **`ADR-048` §4's per-path rules** — which prefixes UNDER that route ask
    // something different of a caller. Appended for `library`'s reason: the
    // canonical order is this array, and a key at the end cannot make an older
    // manifest out-of-order.
    //
    // **OPTIONAL, and its absence means *the whole route takes `proxy-auth`***,
    // which is what every manifest emitted before this key existed says and
    // what the key it widens still says on its own. So the widening adds a
    // state rather than reinterpreting one, and no `.rvt` already signed means
    // anything different by being read by a build that understands this.
    //
    // A rule here may relax OR tighten; see `ProxyRule` for why only one of
    // those directions had to be designed carefully.
    "proxy-auth-paths",
    // **`ADR-051`'s backup surface, 2026-09-08** — the three keys an
    // application uses to OFFER its data to whoever is backing this machine up.
    // Appended for `library`'s reason: the canonical order is this array, and
    // keys at the end cannot make an older manifest out-of-order.
    //
    // **All three are OPTIONAL and absence is *offers nothing***, which is the
    // closed end of `ADR-015` §3's axis and what every manifest emitted
    // before today says. No `.rvt` already signed means anything different by
    // being read by a build that understands these.
    //
    // **`backup-path` and `backup-format` are one declaration**, and either
    // without the other is refused: a route with no format is bytes nothing can
    // check past their length, and a format with no route is a claim about
    // something unreachable.
    //
    // **`backup-feed` is the OTHER shape and the manifest refuses both.** It is
    // a protocol difference rather than a format one — a cursor, a low-water
    // mark, items named by their own digest — so a manifest that declared an
    // artifact and a feed would be two contracts with no rule for choosing.
    "backup-path",
    "backup-format",
    "backup-feed",
    // **`ADR-015` §3's `cpu` request, 2026-09-09** — `memory`'s sibling,
    // appended for `library`'s reason: the canonical order is this array, and
    // a key at the end cannot make an older manifest out-of-order.
    //
    // **It is NOT next to `memory`**, and that placement is deliberate rather
    // than an oversight: `memory` earned its position in the ORIGINAL nine
    // because this array's order is also the order `Manifest::render` writes
    // in, and moving `cpu` up beside it would insert a key in the MIDDLE of
    // an array every manifest ever emitted was checked against — exactly
    // what this comment, repeated at every entry below `memory`, exists to
    // forbid.
    //
    // **OPTIONAL, and its absence means *this application has asked for no
    // particular share of the processor*** — the closed end of `ADR-015`
    // §3.3's axis, like `memory` itself. No existing `.rvt` is invalidated.
    "cpu",
    // **`PLAN-Feature-Set-2026-09-12` §6.2's addon primitive, `ADR-053`,
    // 2026-09-13 — a LIST since `.rulings/` R-162 R28, 2026-09-13** — the
    // classes of library this application accepts. Appended for `library`'s
    // reason: the canonical order is this array, and a key at the end
    // cannot make an older manifest out-of-order.
    //
    // **OPTIONAL, and its absence means *this application takes no addon of
    // any class*** — the closed end of `ADR-015` §3's permission-request
    // axis, like every other optional key here. No existing `.rvt` is
    // invalidated: an
    // application built before this key existed still resolves its explicit
    // `library` closure exactly as before, and simply joins no addon.
    "accepts-class",
    // **`ADR-051` §12's restore route, `.rulings/` R-183 D1, 2026-09-14** —
    // where the application accepts its own backup back. Appended for
    // `library`'s reason: the canonical order is this array, and a key at the
    // end cannot make an older manifest out-of-order. It is NOT beside
    // `backup-format`, for `cpu`'s reason above.
    //
    // **OPTIONAL, and its absence means *accepts no restore***, the closed end
    // of `ADR-015` §3's axis. It is meaningful only beside `backup-path` and
    // `backup-format` — the format is what a restore is checked against — so
    // the parser refuses it without them, beside a feed, and on a route
    // `rivet-proxyd` serves without an operator certificate.
    "restore-path",
    // **`ADR-055` §3's declared per-machine material, 2026-09-15** — the
    // KINDS of material this application needs placed on the machine before it
    // can work. Appended for `library`'s reason: the canonical order is this
    // array, and a key at the end cannot make an older manifest out-of-order.
    //
    // **OPTIONAL, and its absence means *needs nothing placed***, the closed
    // end of `ADR-015` §3's axis. A required key would refuse every artifact
    // already published, and an application that says nothing here means what
    // every application meant before this key existed.
    //
    // **A kind this build does not define is REFUSED rather than ignored.**
    // The vocabulary is the platform's ([`Material::ALL`]), so an unknown token
    // is an author's typo, and a manifest whose unreadable half was skipped
    // would ask for material nothing ever provides — silently.
    "material",
];

impl Manifest {
    /// The smallest manifest that is legal: a daemon that restarts on failure,
    /// is ready when it starts, has **no network at all**, gets its private
    /// directory, requests no memory and declares no sockets.
    ///
    /// **The defaults lean at the closed end of every axis**, which is the only
    /// direction a default may lean in a permission request (`ADR-015` §3.3).
    /// They are applied by the *build tool* when a field is absent from
    /// `Cargo.toml`; the artifact itself always spells every one of them, so
    /// nothing on a machine ever depends on a default table matching the one the
    /// artifact was built against.
    #[must_use]
    pub fn new(kind: ProgramKind) -> Self {
        Self {
            kind,
            restart: Restart::OnFailure,
            ready: Ready::None,
            network: Network::None,
            state: StateGrant::Private,
            memory: None,
            listen: Vec::new(),
            provides: Vec::new(),
            connects: Vec::new(),
            title: None,
            proxy_path: None,
            // The closed end of `ADR-046` §4's axis, and the one axis whose
            // closed end is the *absent* key rather than the present one.
            proxy_auth: ProxyAuth::Operator,
            // Empty is *the whole route takes the line above*, and is the
            // closed end of `ADR-048` §4's axis for the same reason.
            proxy_auth_paths: Vec::new(),
            settings: Vec::new(),
            // Empty is the closed end of this axis too: an application that
            // declares no library needs none, which is what a static binary is.
            library: Vec::new(),
            // And the closed end of §6.2's: no device at all. Every application
            // written before 2026-08-31 wanted none, and most after will.
            devices: Vec::new(),
            // The closed end of `ADR-051`'s axis: an application that says
            // nothing offers nothing, and nothing fetches from it.
            backup: None,
            // `memory`'s closed end, on the same footing: no request means no
            // particular share of the processor, which is every manifest
            // emitted before 2026-09-09.
            cpu: None,
            // The closed end of `ADR-053`'s axis: an application that names no
            // class takes no addon, which is every manifest emitted before
            // that decision and every application that only uses an explicit
            // `library` closure. A list since `.rulings/` R-162 R28; empty is
            // still the closed end.
            accepts_class: Vec::new(),
            // The closed end of `ADR-055` §3's axis: an application that names
            // no material needs nothing placed on the machine for it, which is
            // every manifest emitted before that decision.
            material: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_restart(mut self, restart: Restart) -> Self {
        self.restart = restart;
        self
    }
    #[must_use]
    pub fn with_ready(mut self, ready: Ready) -> Self {
        self.ready = ready;
        self
    }
    #[must_use]
    pub fn with_network(mut self, network: Network) -> Self {
        self.network = network;
        self
    }
    #[must_use]
    pub fn with_state(mut self, state: StateGrant) -> Self {
        self.state = state;
        self
    }
    #[must_use]
    pub fn with_memory(mut self, memory: Option<Memory>) -> Self {
        self.memory = memory;
        self
    }
    /// [`Self::with_memory`]'s footing, for [`Cpu`].
    #[must_use]
    pub fn with_cpu(mut self, cpu: Option<Cpu>) -> Self {
        self.cpu = cpu;
        self
    }

    /// Declare the classes of library this application accepts —
    /// `PLAN-Feature-Set-2026-09-12` §6.2, `ADR-053`, a list since
    /// `.rulings/` R-162 R28.
    ///
    /// **Duplicates are not refused.** Matching (`crate::service::
    /// addon_library_path`) walks the installed libraries, not this list, so
    /// a repeated class cannot make one installed library join twice —
    /// there is nothing here for a duplicate-refusal to protect.
    ///
    /// # Errors
    ///
    /// [`ManifestFault::ListTooLong`] beyond [`MAX_LIST`];
    /// [`ManifestFault::BadCharacter`] for a class that is not
    /// [`is_class_name`], and [`ManifestFault::ValueTooLong`] beyond
    /// [`MAX_VALUE`].
    pub fn with_accepts_classes(mut self, classes: Vec<String>) -> Result<Self, ManifestFault> {
        if classes.len() > MAX_LIST {
            return Err(ManifestFault::ListTooLong { key: "accepts-class" });
        }
        for class in &classes {
            if class.len() > MAX_VALUE {
                return Err(ManifestFault::ValueTooLong);
            }
            if !is_class_name(class) {
                return Err(ManifestFault::BadCharacter);
            }
        }
        self.accepts_class = classes;
        Ok(self)
    }

    /// Sockets Rivet opens and passes in — `ADR-015` §5.2.
    pub fn with_listen(mut self, listen: Vec<Socket>) -> Result<Self, ManifestFault> {
        bound_list("listen", listen.len())?;
        self.listen = listen;
        Ok(self)
    }

    pub fn with_provides(mut self, provides: Vec<Endpoint>) -> Result<Self, ManifestFault> {
        bound_list("provides", provides.len())?;
        self.provides = provides;
        Ok(self)
    }

    /// Declare the per-machine material — `ADR-055` §3. Sorted and
    /// deduplicated, on [`Manifest::with_devices`]'s footing: two orderings of
    /// one request are one artifact rather than two.
    #[must_use]
    pub fn with_material(mut self, mut material: Vec<Material>) -> Self {
        material.sort_unstable();
        material.dedup();
        self.material = material;
        self
    }

    /// Declare the device classes. Sorted and deduplicated, so that two
    /// orderings of one request are one artifact rather than two.
    #[must_use]
    pub fn with_devices(mut self, mut devices: Vec<Device>) -> Self {
        devices.sort_unstable();
        devices.dedup();
        self.devices = devices;
        self
    }

    /// Declare the closure. See [`Manifest::library`].
    pub fn with_library(mut self, library: Vec<library::Reference>) -> Result<Self, ManifestFault> {
        if library.len() > library::MAX_CLOSURE {
            return Err(ManifestFault::ListTooLong { key: "library" });
        }
        self.library = library;
        Ok(self)
    }

    /// Set the human-readable name.
    ///
    /// Refused when longer than [`MAX_VALUE`] or when it carries a character no
    /// manifest may hold, which is the same rule every other value obeys — a
    /// title reaches a screen and a package list, and neither wants a newline.
    /// Declare the settings this application understands — `ADR-038` §3.
    ///
    /// # Errors
    ///
    /// [`ManifestFault::BadCharacter`] for a name that is not
    /// [`is_setting_name`], and [`ManifestFault::ListTooLong`] beyond
    /// [`MAX_LIST`]. A DUPLICATE is refused too: two declarations of one name
    /// is not a longer list, it is a manifest that cannot say which it meant.
    /// [`ManifestFault::CredentialLikeSetting`] for a name
    /// [`is_credential_like_name`] — `ADR-038` §5, `.rulings/` R-169:
    /// a credential is an installed secret and never a setting.
    pub fn with_settings(mut self, settings: Vec<String>) -> Result<Self, ManifestFault> {
        if settings.len() > MAX_LIST {
            return Err(ManifestFault::ListTooLong { key: "settings" });
        }
        for (index, name) in settings.iter().enumerate() {
            if !is_setting_name(name) {
                return Err(ManifestFault::BadCharacter);
            }
            if is_credential_like_name(name) {
                return Err(ManifestFault::CredentialLikeSetting);
            }
            if settings[..index].contains(name) {
                return Err(ManifestFault::BadCharacter);
            }
        }
        self.settings = settings;
        Ok(self)
    }

    /// The settings this application understands, in the order it declared
    /// them. Empty is *accepts none*.
    #[must_use]
    pub fn settings(&self) -> &[String] {
        &self.settings
    }

    /// Whether this application declared `name` — the check `ADR-038` §3.1
    /// item 1 makes at COMPOSITION time, so a misspelled key is refused on a
    /// workstation in front of the person who typed it rather than at runtime
    /// on a board in a hallway.
    #[must_use]
    pub fn declares_setting(&self, name: &str) -> bool {
        self.settings.iter().any(|declared| declared == name)
    }

    /// **Where `rivet-proxyd` mounts this application** — `R-133`.
    ///
    /// # Errors
    ///
    /// [`ManifestFault::BadCharacter`] for anything but a single lower-case
    /// path segment, and [`ManifestFault::ValueTooLong`] for an empty or
    /// oversized one.
    ///
    /// # The shape is deliberately narrower than a URL path
    ///
    /// One segment, `[a-z0-9-]`, no leading or trailing `/` and no `/` inside.
    /// Three reasons, and none of them is tidiness:
    ///
    /// 1. **`..` and `//` cannot be spelled**, so a route cannot be made to
    ///    escape its own prefix. A proxy that had to normalise a path before
    ///    trusting it is a proxy with a traversal bug waiting in it.
    /// 2. **It matches `crate::service::ServiceName`'s alphabet**, so the
    ///    obvious default — mount an application at its own name — is always
    ///    legal and needs no transformation.
    /// 3. **Nesting is a routing decision, not an application's.** An
    ///    application that could ask for `a/b/c` would be asserting a shape of
    ///    the machine's URL space that no other application can see, and two of
    ///    them could overlap without either being a duplicate.
    pub fn with_proxy_path(mut self, path: Option<String>) -> Result<Self, ManifestFault> {
        if let Some(text) = &path {
            if text.is_empty() || text.len() > MAX_VALUE {
                return Err(ManifestFault::ValueTooLong);
            }
            if !is_proxy_path(text) {
                return Err(ManifestFault::BadCharacter);
            }
            if is_reserved_proxy_path(text) {
                return Err(ManifestFault::ReservedProxyPath);
            }
        }
        self.proxy_path = path;
        Ok(self)
    }

    /// The path this application asked to be mounted at, if it asked.
    ///
    /// **`None` is *not proxied*** and is what every manifest emitted before
    /// `R-133` means. It is deliberately not defaulted to the service name: a
    /// default would mount every application on the machine the moment a proxy
    /// was enabled, which is the opposite of `ADR-015` §3.3's closed end.
    #[must_use]
    pub fn proxy_path(&self) -> Option<&str> {
        self.proxy_path.as_deref()
    }

    /// **Whether this application is proxied** — it declares a `proxy-path` —
    /// which is what decides where its ports bind ([`Socket::bind_address`],
    /// `ADR-046` §7).
    ///
    /// A request for a route and not a promise of one: two applications
    /// asking for one path both lose it (`crate::proxy`), and a machine may
    /// carry no proxy at all. Either way the answer here stands, because what
    /// the signature says must not depend on what else is installed.
    #[must_use]
    pub fn proxied(&self) -> bool {
        self.proxy_path.is_some()
    }

    /// **The port `rivet-proxyd` forwards this application's route to** —
    /// its FIRST TCP socket, when it declares a `proxy-path`.
    ///
    /// One computation for the three places that need it (`ADR-011`): the
    /// supervisor deriving the routing table, and the two readers refusing
    /// [`DIRECT_RULE`]'s bypass. `None` for an application with no route, and
    /// for one that asked for a route and declared no TCP socket.
    #[must_use]
    pub fn route_port(&self) -> Option<u16> {
        self.proxy_path.as_ref()?;
        self.listen.iter().find(|s| s.is_tcp()).and_then(Socket::port)
    }

    /// **Whether a `:direct` flag in this manifest is one [`DIRECT_RULE`]
    /// refuses** — on the route's backend, or with no `proxy-path` at all.
    ///
    /// Asked after the whole manifest is read, by [`Manifest::parse`] and by
    /// `cargo rivet package`, because `listen` and `proxy-path` are separate
    /// keys and a builder that checked here would depend on which setter ran
    /// first — [`Manifest::with_backup`]'s reason.
    #[must_use]
    pub fn refuses_direct(&self) -> bool {
        if !self.proxied() {
            return self.listen.iter().any(Socket::is_direct);
        }
        self.listen
            .iter()
            .find(|s| s.is_tcp())
            .is_some_and(Socket::is_direct)
    }

    /// **Whether this application's route needs an operator's certificate** —
    /// `ADR-046` §4.3.
    ///
    /// There is no `Option` and there is no third state: a manifest that says
    /// nothing requires an operator, which is what every manifest emitted
    /// before 2026-09-05 means and what a machine must assume of one it cannot
    /// ask.
    #[must_use]
    pub fn with_proxy_auth(mut self, auth: ProxyAuth) -> Self {
        self.proxy_auth = auth;
        self
    }

    /// What the route asks of its callers, where no per-path rule says
    /// otherwise. [`ProxyAuth::Operator`] unless the manifest relaxed it.
    #[must_use]
    pub fn proxy_auth(&self) -> ProxyAuth {
        self.proxy_auth
    }

    /// **The per-path rules** — `ADR-048` §4.
    ///
    /// Sorted by prefix and deduplicated here, for the reason
    /// [`Self::with_devices`] sorts: a manifest has one legitimate producer and
    /// it is the emitter, so two spellings of one intent would be two
    /// artifacts, and a prefix carrying two verdicts would make *what does this
    /// route ask of a caller at /pair* have two answers (`ADR-011`).
    ///
    /// **A duplicate prefix is refused rather than resolved**, whatever the two
    /// verdicts are. Picking one would mean the answer depended on the order an
    /// author happened to write two lines in, which is the property the matcher
    /// exists not to have.
    pub fn with_proxy_auth_paths(mut self, rules: Vec<ProxyRule>) -> Result<Self, ManifestFault> {
        bound_list("proxy-auth-paths", rules.len())?;
        let mut canonical = rules;
        canonical.sort_by(|a, b| a.prefix.cmp(&b.prefix));
        let before = canonical.len();
        canonical.dedup_by(|a, b| a.prefix == b.prefix);
        if canonical.len() != before {
            return Err(ManifestFault::BadValue {
                key: "proxy-auth-paths",
            });
        }
        self.proxy_auth_paths = canonical;
        Ok(self)
    }

    /// What individual prefixes under this route ask of a caller.
    ///
    /// **Empty is the ordinary answer** and means the whole route takes
    /// [`Self::proxy_auth`].
    #[must_use]
    pub fn proxy_auth_paths(&self) -> &[ProxyRule] {
        &self.proxy_auth_paths
    }

    /// **Declare the backup surface** — `ADR-051`.
    ///
    /// # Errors
    ///
    /// [`ManifestFault::ValueTooLong`] for an empty or oversized path, and
    /// [`ManifestFault::BadCharacter`] for one that is not
    /// [`is_proxy_prefix`] — see [`BACKUP_PATH_RULE`].
    ///
    /// **It does not check for a `proxy-path` here**, and that asymmetry is
    /// deliberate: this is the builder, and `cargo rivet package` reads the two
    /// keys out of a table in whatever order the table holds them, so a check
    /// here would depend on which setter ran first. [`Manifest::render`] does
    /// not write a backup key without a route and [`Manifest::parse`] refuses
    /// one, so the artifact cannot carry the pair however it was built.
    ///
    /// **A `restore-path` is checked by the same predicate** — `ADR-051` §12 —
    /// and, for the reason above, whether its route requires an operator is
    /// NOT checked here: `proxy-auth` and `proxy-auth-paths` may be set after
    /// this. [`Manifest::parse`] refuses an open restore route, and
    /// `cargo rivet package` asks [`Manifest::requires_operator_at`] before it
    /// emits one.
    pub fn with_backup(mut self, backup: Option<BackupSurface>) -> Result<Self, ManifestFault> {
        if let Some(surface) = &backup {
            for path in core::iter::once(surface.path()).chain(surface.restore_path()) {
                if path.is_empty() || path.len() > MAX_VALUE {
                    return Err(ManifestFault::ValueTooLong);
                }
                if !is_proxy_prefix(path) {
                    return Err(ManifestFault::BadCharacter);
                }
            }
        }
        self.backup = backup;
        Ok(self)
    }

    /// **Whether `rivet-proxyd` requires an operator's certificate at `path`**,
    /// a path relative to this application's `proxy-path`.
    ///
    /// The same longest-match the proxy runs on a live request —
    /// [`crate::proxy::requires_operator_under`] — over this manifest's own
    /// `proxy-auth` and `proxy-auth-paths`, so what the manifest checks at
    /// package time and what the proxy enforces are one computation rather
    /// than two that agree today (`ADR-011`). `ADR-051` §12 is the caller: a
    /// restore route must answer `true`.
    #[must_use]
    pub fn requires_operator_at(&self, path: &str) -> bool {
        crate::proxy::requires_operator_under(
            self.proxy_auth.requires_operator(),
            &self.proxy_auth_paths,
            path,
        )
    }

    /// The backup surface this application offers, if it offers one.
    ///
    /// **`None` is *offers nothing***, and it is what every manifest emitted
    /// before `ADR-051` means. `ADR-030` excluded application data from
    /// Rivet's own backup — *"remains the operator's"* — so this is the
    /// application's own answer to a question the platform deliberately does
    /// not answer for it.
    #[must_use]
    pub fn backup(&self) -> Option<&BackupSurface> {
        self.backup.as_ref()
    }

    pub fn with_title(mut self, title: Option<String>) -> Result<Self, ManifestFault> {
        if let Some(text) = &title {
            check_title(text)?;
        }
        self.title = title;
        Ok(self)
    }

    /// What a person should call this, or the service name if it said nothing.
    ///
    /// **Takes the fallback rather than returning an `Option`**, because every
    /// caller wants a string to draw and a caller that forgot the fallback
    /// would draw nothing.
    #[must_use]
    pub fn title<'a>(&'a self, service_name: &'a str) -> &'a str {
        self.title.as_deref().unwrap_or(service_name)
    }

    pub fn with_connects(mut self, connects: Vec<Endpoint>) -> Result<Self, ManifestFault> {
        bound_list("connects", connects.len())?;
        self.connects = connects;
        Ok(self)
    }

    #[must_use]
    pub fn kind(&self) -> ProgramKind {
        self.kind
    }
    #[must_use]
    pub fn restart(&self) -> Restart {
        self.restart
    }
    #[must_use]
    pub fn ready(&self) -> Ready {
        self.ready
    }
    #[must_use]
    pub fn network(&self) -> &Network {
        &self.network
    }
    #[must_use]
    pub fn state(&self) -> StateGrant {
        self.state
    }
    #[must_use]
    pub fn memory(&self) -> Option<&Memory> {
        self.memory.as_ref()
    }
    /// [`Self::memory`]'s footing, for [`Cpu`].
    #[must_use]
    pub fn cpu(&self) -> Option<&Cpu> {
        self.cpu.as_ref()
    }

    /// The classes of library this application accepts —
    /// `PLAN-Feature-Set-2026-09-12` §6.2, a list since `.rulings/` R-162
    /// R28.
    ///
    /// **Empty is *takes no addon*.** The supervisor's discovery matches this
    /// against an installed library's [`crate::package::library::Manifest::
    /// provides_class`]; a match requires the library's one class to appear
    /// somewhere in this list, character for character.
    #[must_use]
    pub fn accepts_classes(&self) -> &[String] {
        &self.accepts_class
    }
    #[must_use]
    pub fn listen(&self) -> &[Socket] {
        &self.listen
    }
    #[must_use]
    pub fn provides(&self) -> &[Endpoint] {
        &self.provides
    }
    #[must_use]
    pub fn connects(&self) -> &[Endpoint] {
        &self.connects
    }

    /// The device classes this application asks for — `ADR-015` §6.2.
    ///
    /// **Empty is the ordinary answer.** A program that neither draws nor reads
    /// input declares nothing, which is every Rivet application to date.
    #[must_use]
    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// The declared closure — every library artifact this program needs.
    ///
    /// **Empty is the ordinary answer**, and it means the program is statically
    /// linked, which is every Rivet application built before 2026-08-31 and most
    /// of them after (`ADR-003` §5).
    #[must_use]
    pub fn library(&self) -> &[library::Reference] {
        &self.library
    }

    /// The per-machine material this application needs — `ADR-055` §3.
    ///
    /// **Empty is the ordinary answer**, and it means the application needs
    /// nothing placed on the machine before it works, which is every manifest
    /// emitted before 2026-09-15 and every application that neither serves
    /// HTTPS nor calls one that does.
    #[must_use]
    pub fn material(&self) -> &[Material] {
        &self.material
    }

    /// The canonical text — the bytes that go into [`MANIFEST_NAME`].
    ///
    /// **One line per key, in `KEYS`' order, every list present even when it
    /// is empty.** `memory` is the one key that may be absent, because absent is
    /// the only way to say *no request* and `0` would be a request for nothing.
    ///
    /// Two machines given the same manifest must produce the same bytes, for
    /// `ADR-007` §3.1's reason applied to an artifact: the digest of this member
    /// is in a signed header, so a second spelling of the same manifest is a
    /// different artifact.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("# Rivet application manifest — ADR-015 §3.2.\n");
        out.push_str("# Emitted by `cargo rivet package`; not written by hand.\n");
        line(
            &mut out,
            "kind",
            match self.kind {
                ProgramKind::Daemon => "daemon",
                ProgramKind::Oneshot => "oneshot",
                ProgramKind::SocketActivated => "socket-activated",
            },
        );
        line(
            &mut out,
            "restart",
            match self.restart {
                Restart::Always => "always",
                Restart::OnFailure => "on-failure",
                Restart::Never => "never",
            },
        );
        line(
            &mut out,
            "ready",
            match self.ready {
                Ready::None => "none",
                Ready::Notify => "notify",
                Ready::Listening => "listening",
            },
        );
        let network = match &self.network {
            Network::None => "none".to_string(),
            Network::Egress => "egress".to_string(),
            Network::Group(name) => format!("group:{name}"),
        };
        line(&mut out, "network", &network);
        line(
            &mut out,
            "state",
            match self.state {
                StateGrant::None => "none",
                StateGrant::Private => "private",
            },
        );
        if let Some(memory) = &self.memory {
            line(&mut out, "memory", memory.as_str());
        }
        let listen: Vec<String> = self.listen.iter().map(Socket::render).collect();
        list(&mut out, "listen", &listen);
        let provides: Vec<String> = self.provides.iter().map(Endpoint::render).collect();
        list(&mut out, "provides", &provides);
        let connects: Vec<String> = self.connects.iter().map(Endpoint::render).collect();
        list(&mut out, "connects", &connects);
        let library: Vec<String> = self
            .library
            .iter()
            .map(|r| r.as_str().to_string())
            .collect();
        list(&mut out, "library", &library);
        let devices: Vec<String> = self.devices.iter().map(|d| d.to_string()).collect();
        list(&mut out, "devices", &devices);
        // **Written last and only when set**, which is what lets a manifest
        // emitted before this key existed parse unchanged: the canonical order
        // is `KEYS` and this is the end of it.
        if let Some(title) = &self.title {
            out.push_str(&format!("title = \"{title}\"\n"));
        }
        // **Written only when there are any**, so a manifest emitted before
        // this key existed renders and parses byte-identically. An empty list
        // and an absent key are one state - see the field.
        if !self.settings.is_empty() {
            list(&mut out, "settings", &self.settings);
        }
        // **Written only when set**, for `settings`' reason: a manifest emitted
        // before `R-133` renders and parses byte-identically, and an absent key
        // means NOT PROXIED rather than proxied at the empty path.
        if let Some(path) = &self.proxy_path {
            out.push_str(&format!("proxy-path = \"{path}\"\n"));
            // **Written only to RELAX, and only when there is a route to relax**
            // — `ADR-046` §4.3. Emitting `proxy-auth = "operator"` would be
            // spelling the default, which makes two byte-strings for one
            // manifest; emitting it with no `proxy-path` would be a relaxation
            // attached to no route, which `Manifest::parse` refuses.
            if self.proxy_auth != ProxyAuth::Operator {
                out.push_str(&format!("proxy-auth = \"{}\"\n", self.proxy_auth));
            }
            // **Written only when there are any**, so a manifest emitted before
            // `ADR-048` renders and parses byte-identically. An empty list and
            // an absent key are one state — see the field.
            if !self.proxy_auth_paths.is_empty() {
                let rules: Vec<String> = self
                    .proxy_auth_paths
                    .iter()
                    .map(ToString::to_string)
                    .collect();
                list(&mut out, "proxy-auth-paths", &rules);
            }
            // **`ADR-051`, and INSIDE the route's block for `proxy-auth`'s
            // reason**: a backup route is relative to `proxy-path`, so one
            // written without a route would name nothing until somebody added
            // the path beside it — a key that becomes live by an edit elsewhere
            // in the file. `Manifest::parse` refuses that pair; this never
            // emits it.
            match &self.backup {
                Some(BackupSurface::Artifact { path, format, .. }) => {
                    line(&mut out, "backup-path", path);
                    line(&mut out, "backup-format", format.as_str());
                }
                Some(BackupSurface::Feed { path }) => line(&mut out, "backup-feed", path),
                None => {}
            }
        }
        // **Written last and only when set**, `title`'s reason: `cpu` sits at
        // the end of `KEYS` (see that array's own comment for why it is not
        // beside `memory`), so a manifest emitted before this key existed
        // renders and parses byte-identically.
        if let Some(cpu) = &self.cpu {
            line(&mut out, "cpu", cpu.as_str());
        }
        // **Written only when there are any**, `settings`' reason:
        // `accepts-class` sits at the end of `KEYS`, so a manifest emitted
        // before `ADR-053` — or before `.rulings/` R-162 R28 made it a
        // list — renders and parses byte-identically.
        if !self.accepts_class.is_empty() {
            list(&mut out, "accepts-class", &self.accepts_class);
        }
        // **`ADR-051` §12, written last and only when set**, `cpu`'s reason:
        // `restore-path` sits at the end of `KEYS`, so a manifest emitted before
        // it existed renders and parses byte-identically. Guarded by the route
        // for the backup keys' reason — the backup keys are not written without
        // one, and a restore path relative to nothing must not be either.
        if self.proxy_path.is_some()
            && let Some(path) = self.backup.as_ref().and_then(BackupSurface::restore_path)
        {
            line(&mut out, "restore-path", path);
        }
        // **`ADR-055` §3, written only when there are any**, `settings`'
        // reason: `material` sits at the end of `KEYS`, so a manifest emitted
        // before this key existed renders and parses byte-identically. It is
        // NOT guarded by the route — the same three files are a client
        // certificate for an application that serves nothing.
        if !self.material.is_empty() {
            let material: Vec<String> = self.material.iter().map(ToString::to_string).collect();
            list(&mut out, "material", &material);
        }
        out
    }

    /// Read the canonical text back.
    ///
    /// # This is a subset of TOML and not TOML
    ///
    /// It accepts exactly what [`Self::render`] emits: blank lines, `#`
    /// comments, and `key = "scalar"` or `key = ["a", "b"]` with the keys in
    /// `KEYS`' order, one space either side of the `=`, `, ` between elements,
    /// no escapes and no character outside `0x21..=0x7e` inside a value.
    ///
    /// **That narrowness is the point rather than a shortcut.** A manifest is a
    /// build product; the only thing that legitimately produces one is the
    /// emitter above. A parser wider than its emitter is a second grammar with
    /// no second producer, and the thing it would accept is somebody else's
    /// bytes — which for a *permission request* is exactly where leniency is
    /// most expensive (`ADR-015` §4.1).
    pub fn parse(bytes: &[u8]) -> Result<Self, ManifestFault> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(ManifestFault::TooLarge);
        }
        let text = core::str::from_utf8(bytes).map_err(|_| ManifestFault::NotText)?;

        let mut kind: Option<ProgramKind> = None;
        let mut restart: Option<Restart> = None;
        let mut ready: Option<Ready> = None;
        let mut network: Option<Network> = None;
        let mut state: Option<StateGrant> = None;
        let mut memory: Option<Memory> = None;
        let mut listen: Option<Vec<Socket>> = None;
        let mut provides: Option<Vec<Endpoint>> = None;
        let mut connects: Option<Vec<Endpoint>> = None;
        let mut library_refs: Option<Vec<library::Reference>> = None;
        let mut devices: Option<Vec<Device>> = None;
        // Optional: absence means "call it by its service name".
        //
        // **This was `let title = None;` with no match arm until 2026-09-05**,
        // and the comment where this one is said `title` was *"parsed by
        // `with_title` from the table rather than in the match below"*. That
        // was not true: `with_title` is the BUILDER's, used by
        // `cargo rivet package` to put a title INTO a manifest, and nothing
        // ever took one back out. So `title` was in `KEYS` — which made the
        // ordering check accept it — and in no arm of the match, which made the
        // catch-all refuse it as *a key is not one this build defines*.
        //
        // **Every `.rvt` declaring a title was unreadable by every build**, and
        // nothing noticed because no application had ever set one. The first to
        // try was `aura-controller` on 2026-09-05; the board refused the
        // payload and so did the tool that had just written it.
        //
        // The signal was there: an `unused mut` warning, silenced on
        // 2026-09-03 by removing the `mut` and writing a reason. The warning
        // was telling the truth and the reason was the guess.
        let mut title: Option<String> = None;
        let mut proxy_path: Option<String> = None;
        // **Not an `Option`.** A manifest that never mentions this key requires
        // an operator, so there is no *unset* state to carry — `ADR-046` §4.3.
        let mut proxy_auth = ProxyAuth::Operator;
        // Not an `Option` either: an empty list is *the whole route takes
        // `proxy-auth`*, which is what a manifest that never mentions this key
        // says — `ADR-048` §4.
        let mut proxy_auth_paths: Vec<ProxyRule> = Vec::new();
        let mut settings: Vec<String> = Vec::new();
        // `ADR-051`'s three keys are read separately and reconciled below,
        // because the ordering rule reads one line at a time and the constraint
        // between them is about the SET.
        let mut backup_path: Option<String> = None;
        let mut backup_format: Option<BackupFormat> = None;
        let mut backup_feed: Option<String> = None;
        // `ADR-051` §12's fourth, reconciled with the three below for the same
        // reason: the constraint is about the set.
        let mut restore_path: Option<String> = None;
        // `memory`'s footing: absent means no particular share requested.
        let mut cpu: Option<Cpu> = None;
        // `ADR-053`'s footing: absent means no addon of any class is taken.
        // A list since `.rulings/` R-162 R28; empty is the same closed end.
        let mut accepts_class: Vec<String> = Vec::new();
        // `ADR-055` §3's footing: absent means nothing has to be placed on the
        // machine before this application works.
        let mut material: Vec<Material> = Vec::new();

        // The index into KEYS of the last key seen. A key must be strictly
        // after it, which enforces "canonical order" and "at most once" with one
        // comparison and no set.
        let mut seen: Option<usize> = None;

        for raw in text.split('\n') {
            let raw = raw.strip_suffix('\r').unwrap_or(raw);
            if raw.is_empty() || raw.starts_with('#') {
                continue;
            }
            let (key, value) = raw.split_once(" = ").ok_or(ManifestFault::NotKeyValue)?;
            let at = KEYS
                .iter()
                .position(|k| *k == key)
                .ok_or(ManifestFault::UnknownKey)?;
            let name = KEYS[at];
            if seen.is_some_and(|last| at <= last) {
                return Err(ManifestFault::OutOfOrder { key: name });
            }
            seen = Some(at);

            match name {
                "kind" => {
                    kind = Some(match scalar(name, value)? {
                        "daemon" => ProgramKind::Daemon,
                        "oneshot" => ProgramKind::Oneshot,
                        "socket-activated" => ProgramKind::SocketActivated,
                        _ => return Err(ManifestFault::BadValue { key: name }),
                    });
                }
                "restart" => {
                    restart = Some(match scalar(name, value)? {
                        "always" => Restart::Always,
                        "on-failure" => Restart::OnFailure,
                        "never" => Restart::Never,
                        _ => return Err(ManifestFault::BadValue { key: name }),
                    });
                }
                "ready" => {
                    ready = Some(match scalar(name, value)? {
                        "none" => Ready::None,
                        "notify" => Ready::Notify,
                        "listening" => Ready::Listening,
                        _ => return Err(ManifestFault::BadValue { key: name }),
                    });
                }
                "network" => network = Some(parse_network(scalar(name, value)?)?),
                "state" => {
                    state = Some(match scalar(name, value)? {
                        "none" => StateGrant::None,
                        "private" => StateGrant::Private,
                        _ => return Err(ManifestFault::BadValue { key: name }),
                    });
                }
                "memory" => memory = Some(Memory::parse(scalar(name, value)?)?),
                "listen" => {
                    let mut sockets = Vec::new();
                    for element in elements(name, value)? {
                        sockets.push(parse_socket(element)?);
                    }
                    listen = Some(sockets);
                }
                "provides" => {
                    let mut ends = Vec::new();
                    for element in elements(name, value)? {
                        ends.push(Endpoint::parse(name, element)?);
                    }
                    provides = Some(ends);
                }
                "settings" => {
                    let mut declared = Vec::new();
                    for element in elements(name, value)? {
                        if !is_setting_name(element) {
                            return Err(ManifestFault::BadCharacter);
                        }
                        // `ADR-038` §5, `.rulings/` R-169: a credential is an
                        // installed secret and never a setting, checked HERE
                        // too — a manifest read off a card goes through this
                        // parser, never through `with_settings`, so the
                        // packaging-side check alone would only bind an
                        // application built after this landed.
                        if is_credential_like_name(element) {
                            return Err(ManifestFault::CredentialLikeSetting);
                        }
                        if declared.contains(&element.to_string()) {
                            return Err(ManifestFault::BadCharacter);
                        }
                        declared.push(element.to_string());
                    }
                    settings = declared;
                }
                "connects" => {
                    let mut ends = Vec::new();
                    for element in elements(name, value)? {
                        ends.push(Endpoint::parse(name, element)?);
                    }
                    connects = Some(ends);
                }
                "library" => {
                    // The one list whose elements may exceed `MAX_VALUE`: a
                    // reference is a name, a version and 64 hex characters.
                    // `library::MAX_REFERENCE` is that bound, raised for this
                    // key alone rather than for every value.
                    let mut refs = Vec::new();
                    for element in long_elements(name, value, library::MAX_REFERENCE)? {
                        refs.push(
                            library::Reference::parse(element)
                                .map_err(|_| ManifestFault::BadValue { key: name })?,
                        );
                    }
                    if refs.len() > library::MAX_CLOSURE {
                        return Err(ManifestFault::ListTooLong { key: name });
                    }
                    library_refs = Some(refs);
                }
                "devices" => {
                    let mut asked = Vec::new();
                    for element in elements(name, value)? {
                        asked.push(
                            Device::parse(element).ok_or(ManifestFault::BadValue { key: name })?,
                        );
                    }
                    // The emitter sorts and dedups, so anything else is a
                    // second producer — and a duplicate would make "how many
                    // devices did this ask for" have two answers.
                    let mut canonical = asked.clone();
                    canonical.sort_unstable();
                    canonical.dedup();
                    if canonical != asked {
                        return Err(ManifestFault::BadValue { key: name });
                    }
                    devices = Some(asked);
                }
                "title" => {
                    // **Not `scalar`, and that is the second half of the same
                    // defect.** `scalar` applies `check_value`, which is the
                    // TOKEN rule: `0x21..=0x7e`, no space. Every other scalar
                    // in this file is a word — `daemon`, `egress`, `aura` — so
                    // refusing a space is right for them and wrong here. A
                    // title is prose for a person to read, `with_title` has
                    // always allowed `0x20`, and `Aura Controller` would
                    // otherwise be a value this file could write and could not
                    // read back.
                    let text = value
                        .strip_prefix('"')
                        .and_then(|rest| rest.strip_suffix('"'))
                        .ok_or(ManifestFault::BadValue { key: name })?;
                    check_title(text)?;
                    title = Some(text.to_string());
                }
                "proxy-path" => {
                    let text = scalar(name, value)?;
                    if !is_proxy_path(text) {
                        return Err(ManifestFault::BadValue { key: name });
                    }
                    if is_reserved_proxy_path(text) {
                        return Err(ManifestFault::ReservedProxyPath);
                    }
                    proxy_path = Some(text.to_string());
                }
                "proxy-auth" => {
                    let parsed = ProxyAuth::parse(scalar(name, value)?)
                        .ok_or(ManifestFault::BadValue { key: name })?;
                    // `ADR-046` §4's grammar is `operator` and `none`
                    // only — `app` and `plain` answer for a PREFIX
                    // (`ADR-048` §4), never for the whole route.
                    if !parsed.allowed_for_whole_route() {
                        return Err(ManifestFault::BadValue { key: name });
                    }
                    proxy_auth = parsed;
                }
                "proxy-auth-paths" => {
                    let mut asked = Vec::new();
                    for element in elements(name, value)? {
                        asked.push(
                            ProxyRule::parse(element)
                                .ok_or(ManifestFault::BadValue { key: name })?,
                        );
                    }
                    // The emitter sorts by prefix and refuses a duplicate, so
                    // anything else is a second producer — and a prefix with
                    // two verdicts would make *what does this route ask at
                    // /pair* depend on which line came first.
                    let mut canonical = asked.clone();
                    canonical.sort_by(|a, b| a.prefix.cmp(&b.prefix));
                    canonical.dedup_by(|a, b| a.prefix == b.prefix);
                    if canonical != asked {
                        return Err(ManifestFault::BadValue { key: name });
                    }
                    proxy_auth_paths = asked;
                }
                "backup-path" => {
                    let text = scalar(name, value)?;
                    if !is_proxy_prefix(text) {
                        return Err(ManifestFault::BadValue { key: name });
                    }
                    backup_path = Some(text.to_string());
                }
                "backup-format" => {
                    backup_format = Some(
                        BackupFormat::parse(scalar(name, value)?)
                            .ok_or(ManifestFault::BadValue { key: name })?,
                    );
                }
                "backup-feed" => {
                    let text = scalar(name, value)?;
                    if !is_proxy_prefix(text) {
                        return Err(ManifestFault::BadValue { key: name });
                    }
                    backup_feed = Some(text.to_string());
                }
                "cpu" => cpu = Some(Cpu::parse(scalar(name, value)?)?),
                "accepts-class" => {
                    let mut classes = Vec::new();
                    for element in elements(name, value)? {
                        if !is_class_name(element) {
                            return Err(ManifestFault::BadValue { key: name });
                        }
                        classes.push(element.to_string());
                    }
                    accepts_class = classes;
                }
                // `backup-path`'s predicate, a third time — `ADR-051` §12.
                "restore-path" => {
                    let text = scalar(name, value)?;
                    if !is_proxy_prefix(text) {
                        return Err(ManifestFault::BadValue { key: name });
                    }
                    restore_path = Some(text.to_string());
                }
                // `devices`' shape, a second time — `ADR-055` §3. An unknown
                // kind is refused rather than skipped: the vocabulary is the
                // platform's, so a token this build does not define is a typo,
                // and a manifest whose unreadable half was ignored would ask
                // for material nothing ever provides.
                "material" => {
                    let mut asked = Vec::new();
                    for element in elements(name, value)? {
                        asked.push(
                            Material::parse(element)
                                .ok_or(ManifestFault::BadValue { key: name })?,
                        );
                    }
                    // The emitter sorts and dedups, so anything else is a
                    // second producer — `devices`' rule and its reason.
                    let mut canonical = asked.clone();
                    canonical.sort_unstable();
                    canonical.dedup();
                    if canonical != asked {
                        return Err(ManifestFault::BadValue { key: name });
                    }
                    material = asked;
                }
                // KEYS and this match are the same list; a key that reached here
                // would be one added to KEYS and nowhere else.
                _ => return Err(ManifestFault::UnknownKey),
            }
        }

        // **A relaxation attached to no route is refused** — `ADR-046` §4.3.
        // `proxy-auth = "none"` without a `proxy-path` says nothing today and
        // would say *serve me to anybody* the moment somebody added the path,
        // which is a security-relevant key changing meaning by an edit
        // elsewhere in the file. It is also what a manifest looks like when an
        // author deleted the wrong line.
        //
        // **The same holds for a per-path rule** (`ADR-048` §4), and more
        // sharply: a rule names a prefix *under* a route, so one attached to no
        // route names nothing at all until a path appears beside it.
        if proxy_path.is_none() && (proxy_auth != ProxyAuth::Operator || !proxy_auth_paths.is_empty())
        {
            return Err(ManifestFault::MissingKey { key: "proxy-path" });
        }

        // **`ADR-051`'s three keys reconciled, and every combination that is
        // not one of the two shapes is a refusal.**
        //
        // 1. **Both shapes at once.** A manifest declaring an artifact and a
        //    feed is two contracts with no rule for choosing between them, and
        //    picking one would make what a client fetches depend on which line
        //    a reader met first.
        // 2. **Half the artifact shape.** A route with no format is bytes
        //    nothing can check past their length — which is the rung that
        //    catches truncation, but the only one — and a format with no route
        //    is a claim about something unreachable.
        // 3. **Any of them with no `proxy-path`.** Identical to `ADR-046`
        //    §4.3's refusal above and for a sharper reason: a backup path is
        //    RELATIVE to the mount, so with no mount it names nothing at all
        //    until somebody adds one — a declaration that becomes live by an
        //    edit elsewhere in the file. `SPEC-05` §5 has the same shape:
        //    enabling `rivet-proxyd` without `upstream-name` is a service with
        //    no way to reach what it is for.
        //
        // 4. **`restore-path` beside anything but the artifact shape** —
        //    `ADR-051` §12. A restore is checked against `backup-format`
        //    before it is sent, so with no artifact there is nothing to check
        //    it against: absent altogether names the key that is missing, and
        //    beside a feed (which has no format by construction) it is a value
        //    this manifest cannot admit.
        let backup = match (backup_path, backup_format, backup_feed) {
            (None, None, None) if restore_path.is_some() => {
                return Err(ManifestFault::MissingKey { key: "backup-path" });
            }
            (None, None, None) => None,
            (Some(path), Some(format), None) => Some(BackupSurface::Artifact {
                path,
                format,
                restore: restore_path,
            }),
            (None, None, Some(_)) if restore_path.is_some() => {
                return Err(ManifestFault::BadValue {
                    key: "restore-path",
                });
            }
            (None, None, Some(path)) => Some(BackupSurface::Feed { path }),
            (Some(_), None, None) => {
                return Err(ManifestFault::MissingKey {
                    key: "backup-format",
                });
            }
            (None, Some(_), None) => {
                return Err(ManifestFault::MissingKey { key: "backup-path" });
            }
            _ => {
                return Err(ManifestFault::BadValue {
                    key: "backup-feed",
                });
            }
        };
        if backup.is_some() && proxy_path.is_none() {
            return Err(ManifestFault::MissingKey { key: "proxy-path" });
        }

        let manifest = Self {
            kind: kind.ok_or(ManifestFault::MissingKey { key: "kind" })?,
            restart: restart.ok_or(ManifestFault::MissingKey { key: "restart" })?,
            ready: ready.ok_or(ManifestFault::MissingKey { key: "ready" })?,
            network: network.ok_or(ManifestFault::MissingKey { key: "network" })?,
            state: state.ok_or(ManifestFault::MissingKey { key: "state" })?,
            memory,
            listen: listen.ok_or(ManifestFault::MissingKey { key: "listen" })?,
            provides: provides.ok_or(ManifestFault::MissingKey { key: "provides" })?,
            connects: connects.ok_or(ManifestFault::MissingKey { key: "connects" })?,
            library: library_refs.ok_or(ManifestFault::MissingKey { key: "library" })?,
            devices: devices.ok_or(ManifestFault::MissingKey { key: "devices" })?,
            // No `ok_or`: absence means "call it by its service name".
            title,
            settings,
            // No `ok_or`: absence means NOT PROXIED, which is what every
            // manifest emitted before `R-133` means.
            proxy_path,
            // No `ok_or` either, and for the OPPOSITE reason: absence means
            // REQUIRE AN OPERATOR — see `ProxyAuth`.
            proxy_auth,
            // And absence here means *the whole route takes the line above*.
            proxy_auth_paths,
            // And absence here means *offers nothing to back up*, which is
            // `ADR-030`'s exclusion still standing for every application that
            // has not answered it.
            backup,
            // `memory`'s `ok_or`-free footing: absence means no particular
            // share of the processor was asked for.
            cpu,
            // `ADR-053`'s `ok_or`-free footing: absence (an empty list, since
            // `.rulings/` R-162 R28) means no addon of any class is taken.
            accepts_class,
            // `ADR-055` §3's, on the same footing: absence means nothing has
            // to be placed on the machine before this application works.
            material,
        };

        // 5. **A restore route `rivet-proxyd` serves without an operator
        //    certificate is refused** — `ADR-051` §12, [`RESTORE_AUTH_RULE`].
        //    Asked after the whole manifest is read because the answer is the
        //    proxy's longest-match over `proxy-auth` AND `proxy-auth-paths`,
        //    and it is the same function the proxy runs on the live request.
        if let Some(path) = manifest.backup().and_then(BackupSurface::restore_path)
            && !manifest.requires_operator_at(path)
        {
            return Err(ManifestFault::BadValue {
                key: "restore-path",
            });
        }

        // 6. **A `:direct` flag the platform cannot honour is refused** —
        //    `ADR-046` §7, [`DIRECT_RULE`]. On the route's backend it is the
        //    bypass `.rulings/` R-188 closed, and a card-read manifest goes
        //    through this parser and never through `cargo rivet package`, so
        //    the packaging check alone would bind only applications built
        //    after it. With no route it is `proxy-auth`'s refusal above: a
        //    declaration attached to nothing, and a second spelling of a port
        //    that binds every address anyway.
        if manifest.refuses_direct() {
            return Err(if manifest.proxied() {
                ManifestFault::BadValue { key: "listen" }
            } else {
                ManifestFault::MissingKey { key: "proxy-path" }
            });
        }
        Ok(manifest)
    }

    /// What `ADR-015` §6.1 requires to be shown: every declaration this build
    /// cannot enforce, in the order the manifest writes them.
    ///
    /// > Accepting a manifest that requests something this version cannot
    /// > enforce would tell the operator an application is sandboxed when it is
    /// > not — which is worse than not offering the field.
    ///
    /// **Today that is all of them**, because nothing installs and nothing runs
    /// an application (`PLAN.md` §0.8, links 6 and 7). This function exists so
    /// that the list shrinks by deletion as enforcement arrives, rather than by
    /// somebody remembering to write it.
    #[must_use]
    pub fn unenforced(&self) -> Vec<&'static str> {
        // **`network` for EVERY value, and `none` most of all** — `PLAN.md`
        // §123.2 item 3, and it is the correction this function most needed.
        //
        // It used to read `if !matches!(self.network, Network::None)`, so the
        // one value that was never reported as unenforced was `none` — which
        // `ADR-015` §5.1 defines as *"no interfaces at all"* and which is
        // therefore the value an operator is most likely to trust. There is no
        // network namespace on this build (`ADR-015` §6) and `cmd/rivet-sysd`
        // contains no reference to `Network` at all, so `none` confines exactly
        // as much as `egress` does: nothing.
        //
        // What that cost: `deploy/aura/controller` declared no `network` key,
        // took the `none` default, and fetched the National Weather Service
        // every twenty minutes (`PLAN.md` §112.5). The build printed nothing,
        // the start printed nothing, and the operator read `none`. `ADR-015`
        // §6.1 — *"an unenforced declaration must be visible"* — was being
        // honoured for four values out of five, which is the same as not being
        // honoured, because the exception was the reassuring one.
        // ~~`state`~~ **removed 2026-09-04**, the fourth deletion, and the
        // first one that is about a *kernel* rather than about plumbing.
        // `ADR-042`'s Landlock ruleset grants `RIVET_APP_DATA` write and grants
        // write nowhere else, so `state = "private"` now describes what the
        // machine does: an application that opens any other path for writing
        // is refused with `EACCES` by the kernel, including `/tmp`, which is
        // mode 1777 and which Unix permissions never separated two
        // applications on at all.
        //
        // **What it does NOT promise is that the rest of the filesystem is
        // hidden.** There is no mount namespace; the paths are there and a
        // process running as uid 0 reaches every one of them. That is stated
        // where a reader meets it — `cmd/rivet-sysd`'s own ENFORCED record —
        // rather than by leaving this in a list of things that do nothing,
        // because a list that is wrong in the cautious direction teaches a
        // reader that the list is approximate.
        let mut out = vec!["ready", "network"];
        // ~~`restart`~~ **removed 2026-09-03**, the third deletion, and the
        // same audit found it: `cmd/rivet-sysd/src/applications.rs` maps every
        // `Restart` variant into the service definition's `RestartPolicy`, and
        // `crate::supervise::decide` acts on it. It has been enforced for as
        // long as applications have been started. A list that is wrong in the
        // *cautious* direction still costs something — it teaches a reader that
        // the list is approximate, which is exactly what stops them believing
        // the entries that matter.
        if self.memory.is_some() {
            out.push("memory");
        }
        // ~~`listen`~~ **removed 2026-08-31.** `ADR-015` §5.2's descriptor
        // handoff is built: `cmd/rivet-sysd` opens every socket declared here
        // and passes it on `ADR-017` §3.4's `LISTEN_FDS`. This function exists
        // so the list shrinks **by deletion** as enforcement arrives rather
        // than by somebody remembering to write it, and this is the first
        // deletion it has had.
        // ~~`provides` and `connects`~~ **removed 2026-08-31**, the second
        // deletion. `ADR-015` §5.3 is built, and the sentence above is why the
        // shape of what was built has to be read before this is taken as the
        // ADR's full promise: the grant is a **chown of the socket file to one
        // consumer's uid**, not the ADR's bind mount into a mount namespace,
        // because this build has no mount namespaces. See
        // `cmd/rivet-sysd/src/endpoints.rs`.
        //
        // It is listed as enforced rather than unenforced because the thing an
        // operator would be misled about — *"is this declaration doing
        // anything?"* — now has the answer yes: a declared endpoint is created,
        // granted to exactly one declared consumer, and a second consumer is
        // refused by name. What it does NOT promise is isolation, which
        // `crate::service::LISTEN_SOCKET_MODE` and the supervisor's own
        // ENFORCED record both state at the point a reader meets them.
        out
    }
}

impl Endpoint {
    /// `<owner>:<name>`, both non-empty and both lowercase-ish printable.
    ///
    /// # Neither half may hold `/`, and that became load-bearing
    ///
    /// **Tightened 2026-08-31**, the day `provides`/`connects` were built. Until `provides` was
    /// implemented this was an inert declaration and a slash in it reached
    /// nothing; `parse_socket` already refused one for `unix:` and this did not,
    /// which was a difference with no consequence.
    ///
    /// It has one now. `rivet_core::service::provided_socket_path` interpolates
    /// the endpoint name into a path under `LISTEN_DIR`, so
    /// `provides = ["socket:../../../system/bin/rivet"]` would name a file
    /// outside the directory the supervisor is entitled to create, unlink and
    /// **chown to an application's uid**. The manifest is inside the signature,
    /// so this is not a hostile-input path — but `ADR-011` is that a value is
    /// validated where it is parsed and not where it is used, and a rule that
    /// holds only as long as nobody builds the feature is not a rule.
    /// **The third part is `shared`, and only a `provides` may carry it** —
    /// see [`Self::shared`]. The shape follows `listen`'s `tcp:8443:direct`,
    /// which is the precedent for a modifier suffix in this grammar rather
    /// than a new key.
    ///
    /// A `connects` carrying it is refused. Whether an endpoint can take two
    /// readers is a fact about what is listening; letting the asker assert it
    /// would be a consumer granting itself a property of the provider.
    pub fn parse(key: &'static str, text: &str) -> Result<Self, ManifestFault> {
        let (owner, rest) = text
            .split_once(':')
            .ok_or(ManifestFault::BadValue { key })?;
        let (name, shared) = match rest.split_once(':') {
            Some((name, "shared")) => (name, true),
            // Any other third part is a value this build does not define, and
            // a silently ignored modifier is worse than a refusal: it reads as
            // a granted capability and is not one.
            Some(_) => return Err(ManifestFault::BadValue { key }),
            None => (rest, false),
        };
        if !is_path_component(owner) || !is_path_component(name) {
            return Err(ManifestFault::BadValue { key });
        }
        if shared && key != "provides" {
            return Err(ManifestFault::BadValue { key });
        }
        Ok(Self {
            owner: owner.to_string(),
            name: name.to_string(),
            shared,
        })
    }

    fn render(&self) -> String {
        if self.shared {
            format!("{}:{}:shared", self.owner, self.name)
        } else {
            format!("{}:{}", self.owner, self.name)
        }
    }
}

/// What a title may be.
///
/// **One function, called by both the builder and the parser**, because they
/// are two representations of one rule and `ADR-011` is about exactly that.
/// Until 2026-09-05 there was only the builder's copy and the parser had none
/// at all — see [`Manifest::parse`]'s `title` arm for what that cost.
///
/// Printable ASCII, no quote and no backslash: the value is written into a
/// `key = "value"` line with no escaping, so a quote would end the string early
/// and a backslash would start an escape the reader does not implement. A
/// newline would end the line.
///
/// # Errors
///
/// [`ManifestFault::ValueTooLong`] for an empty or oversized title —
/// **empty is refused** because a title that renders as nothing is a package
/// list with a blank row, and omitting the key already means *call it by its
/// service name*. [`ManifestFault::BadCharacter`] otherwise.
fn check_title(text: &str) -> Result<(), ManifestFault> {
    if text.is_empty() || text.len() > MAX_VALUE {
        return Err(ManifestFault::ValueTooLong);
    }
    if !text.bytes().all(|b| (0x20..=0x7e).contains(&b))
        || text.contains('"')
        || text.contains('\\')
    {
        return Err(ManifestFault::BadCharacter);
    }
    Ok(())
}

fn parse_network(text: &str) -> Result<Network, ManifestFault> {
    const KEY: &str = "network";
    match text {
        "none" => Ok(Network::None),
        "egress" => Ok(Network::Egress),
        _ => match text.split_once(':') {
            Some(("group", name)) if !name.is_empty() && !name.contains(':') => {
                Ok(Network::Group(name.to_string()))
            }
            _ => Err(ManifestFault::BadValue { key: KEY }),
        },
    }
}

/// `<port>` or `<port>:direct` — the part of a `tcp:` or `udp:` entry after the
/// transport. The port is `None` when it cannot be a descriptor's port; the
/// flag is whether `:direct` followed (`ADR-046` §7). Any other suffix is
/// `None`, so a misspelled `:drect` is refused rather than read as ordinary.
///
/// Shared by [`Manifest::parse`] and `cargo rivet package`'s reader, so the two
/// readers of this grammar cannot disagree about it (`ADR-011`).
#[must_use]
pub fn parse_port_and_reach(text: &str) -> Option<(u16, bool)> {
    let (port, direct) = match text.split_once(':') {
        None => (text, false),
        Some((port, "direct")) => (port, true),
        Some(_) => return None,
    };
    // Parsed rather than trusted: `0` is a request for an ephemeral port, which
    // is not a thing an application can be passed a descriptor for, and
    // `99999` is a typo that must not survive into a signed artifact.
    let number: u32 = port.parse().ok()?;
    if number == 0 || number > 65535 || port.len() > 1 && port.starts_with('0') {
        return None;
    }
    Some((number as u16, direct))
}

fn parse_socket(text: &str) -> Result<Socket, ManifestFault> {
    const KEY: &str = "listen";
    match text.split_once(':') {
        Some(("tcp", rest)) => match parse_port_and_reach(rest) {
            Some((port, false)) => Ok(Socket::Tcp(port)),
            Some((port, true)) => Ok(Socket::TcpDirect(port)),
            None => Err(ManifestFault::BadValue { key: KEY }),
        },
        // Same rule as `tcp`, and the same reason: a descriptor is what is
        // being requested, and a port of `0` or one that does not fit `u16`
        // cannot be one.
        Some(("udp", rest)) => match parse_port_and_reach(rest) {
            Some((port, false)) => Ok(Socket::Udp(port)),
            Some((port, true)) => Ok(Socket::UdpDirect(port)),
            None => Err(ManifestFault::BadValue { key: KEY }),
        },
        // **The reserved health channel, refused by name** — `ADR-045` §3.
        // Every service is handed an answering socket under that
        // `LISTEN_FDNAMES` entry whether or not its manifest asks, so an
        // application declaring it would be handed two descriptors both
        // claiming to be it — and would accept on whichever the supervisor
        // appended last. The collision is silent, which is why it is refused
        // here rather than resolved at the handoff.
        Some(("unix", name)) if name == crate::health::CHANNEL_NAME => {
            Err(ManifestFault::BadValue { key: KEY })
        }
        Some(("unix", name)) if is_path_component(name) && !name.contains(':') => {
            Ok(Socket::Unix(name.to_string()))
        }
        _ => Err(ManifestFault::BadValue { key: KEY }),
    }
}

/// **A name that will be interpolated into a path, and must therefore BE a
/// name** — not a way of spelling somewhere else.
///
/// Both `Endpoint::parse` and `parse_socket`'s `unix:` arm reach
/// `rivet_core::service::provided_socket_path`, which joins the value onto
/// `LISTEN_DIR`. Whatever comes out, the supervisor will create, unlink and
/// **chown to an application's uid**.
///
/// # `.` and `..` are not filtered by refusing `/`
///
/// Refusing a slash was added 2026-08-31 and reads as though it settles the
/// question. It does not: `provides = ["socket:.."]` holds no slash and names
/// `LISTEN_DIR`'s **parent**, so the supervisor would chown `/run/rivet` itself
/// to a consumer. `socket:.` names the directory. Found in the audit of
/// 2026-09-03 (`PLAN.md` §123, register R-30), and it was in both parsers —
/// the audit named only the first.
///
/// # This is not a hostile-input path, and the rule still holds
///
/// The manifest is inside the signature, so reaching this needs a signing key.
/// The reason to refuse anyway is the one the slash check already gives:
/// `ADR-011` puts validation where a value is PARSED rather than where it is
/// used, and a rule that holds only as long as nobody builds the next feature
/// is not a rule. A NUL is refused for the same reason — it cannot survive a
/// `CString` at the point of use, and failing there says nothing useful about
/// the manifest that carried it.
fn is_path_component(text: &str) -> bool {
    !text.is_empty()
        && text != "."
        && text != ".."
        && !text.contains('/')
        && !text.contains('\0')
}

fn bound_list(key: &'static str, len: usize) -> Result<(), ManifestFault> {
    if len > MAX_LIST {
        return Err(ManifestFault::ListTooLong { key });
    }
    Ok(())
}

fn line(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push_str(" = \"");
    out.push_str(value);
    out.push_str("\"\n");
}

fn list(out: &mut String, key: &str, values: &[String]) {
    out.push_str(key);
    out.push_str(" = [");
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push('"');
        out.push_str(value);
        out.push('"');
    }
    out.push_str("]\n");
}

/// `"…"`, with the quotes, and nothing that would need an escape inside them.
fn scalar<'a>(key: &'static str, value: &'a str) -> Result<&'a str, ManifestFault> {
    let inner = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .ok_or(ManifestFault::BadValue { key })?;
    check_value(inner)?;
    Ok(inner)
}

/// `[]` or `["a", "b"]`, with the brackets.
fn elements<'a>(key: &'static str, value: &'a str) -> Result<Vec<&'a str>, ManifestFault> {
    let inner = value
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
        .ok_or(ManifestFault::BadValue { key })?;
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for part in inner.split(", ") {
        out.push(scalar(key, part)?);
        if out.len() > MAX_LIST {
            return Err(ManifestFault::ListTooLong { key });
        }
    }
    Ok(out)
}

/// [`elements`], for the one key whose entries are longer than [`MAX_VALUE`].
///
/// A separate function rather than a parameter on `elements`, so that the wider
/// bound is reachable only from the key that argued for it — a cap that any
/// caller could raise is not a cap.
fn long_elements<'a>(
    key: &'static str,
    value: &'a str,
    cap: usize,
) -> Result<Vec<&'a str>, ManifestFault> {
    let inner = value
        .strip_prefix('[')
        .and_then(|v| v.strip_suffix(']'))
        .ok_or(ManifestFault::BadValue { key })?;
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for part in inner.split(", ") {
        let text = part
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .ok_or(ManifestFault::BadValue { key })?;
        if text.len() > cap {
            return Err(ManifestFault::ValueTooLong);
        }
        for byte in text.bytes() {
            if !(0x21..=0x7e).contains(&byte) || byte == b'"' || byte == b'\\' {
                return Err(ManifestFault::BadCharacter);
            }
        }
        out.push(text);
        if out.len() > MAX_LIST {
            return Err(ManifestFault::ListTooLong { key });
        }
    }
    Ok(out)
}

fn check_value(text: &str) -> Result<(), ManifestFault> {
    if text.len() > MAX_VALUE {
        return Err(ManifestFault::ValueTooLong);
    }
    for byte in text.bytes() {
        if !(0x21..=0x7e).contains(&byte) || byte == b'"' || byte == b'\\' {
            return Err(ManifestFault::BadCharacter);
        }
    }
    Ok(())
}

// --- the package as an application -----------------------------------------

/// A [`Package`] read as an application.
///
/// Holding one means the kind is `application`, the header carries no
/// `platform` (the maintainer's 2026-08-24 ruling), every member's class and variant are
/// values this build defines, member 0 is the manifest, and there is exactly one
/// program. It does **not** mean any member's content has been checked — that is
/// [`Package::verify_member`], per member, at the point it is about to be used.
#[derive(Debug, PartialEq, Eq)]
pub struct Application<'a> {
    package: &'a Package,
    manifest: &'a Member,
    program: &'a Member,
}

impl<'a> Application<'a> {
    /// Read `package` as an application.
    pub fn open(package: &'a Package) -> Result<Self, PackageError> {
        package.require_kind(&Kind::application())?;

        // The maintainer's 2026-08-24 ruling: no `<platform>` for an application. Enforced
        // rather than merely conventional — a header carrying one would be an
        // artifact from a builder that had not read the ruling, and the field is
        // inside the signature, so refusing costs nothing and accepting would
        // make the ruling advisory.
        if let Some(found) = package.header().platform() {
            return Err(PackageError::UnexpectedPlatform {
                found: found.to_string(),
            });
        }

        let mut manifest: Option<&Member> = None;
        let mut program: Option<&Member> = None;
        let mut program_count = 0usize;
        let mut manifest_count = 0usize;

        for member in package.members() {
            let index = member.index();
            let class = TargetClass::from_byte(member.target_class()).ok_or(
                PackageError::UnknownTargetClass {
                    index,
                    found: member.target_class(),
                },
            )?;
            if member.variant() != VARIANT_NONE {
                return Err(PackageError::UnknownVariant {
                    index,
                    found: member.variant(),
                });
            }
            match class {
                TargetClass::Manifest => {
                    manifest_count += 1;
                    if index != 0 {
                        return Err(PackageError::ManifestNotFirst { index });
                    }
                    if member.name() != MANIFEST_NAME {
                        return Err(PackageError::ManifestMisnamed {
                            name: member.name().to_string(),
                        });
                    }
                    manifest = Some(member);
                }
                TargetClass::Program => {
                    program_count += 1;
                    program = Some(member);
                }
                TargetClass::Asset => {}
            }
        }

        if manifest_count != 1 {
            return Err(PackageError::ManifestMemberCount {
                found: manifest_count,
            });
        }
        if program_count != 1 {
            return Err(PackageError::ProgramMemberCount {
                found: program_count,
            });
        }
        let manifest = manifest.ok_or(PackageError::ManifestMemberCount { found: 0 })?;
        let program = program.ok_or(PackageError::ProgramMemberCount { found: 0 })?;
        Ok(Self {
            package,
            manifest,
            program,
        })
    }

    #[must_use]
    pub fn package(&self) -> &'a Package {
        self.package
    }

    /// The application's name — the header's `product`, and the directory
    /// `ADR-015` §2 installs it at.
    #[must_use]
    pub fn name(&self) -> &'a str {
        self.package.header().product()
    }

    /// The application's version — the header's `version`.
    #[must_use]
    pub fn version(&self) -> &'a str {
        self.package.header().version()
    }

    #[must_use]
    pub fn manifest_member(&self) -> &'a Member {
        self.manifest
    }

    /// The static binary. Exactly one, by [`Self::open`]'s check.
    #[must_use]
    pub fn program_member(&self) -> &'a Member {
        self.program
    }

    /// `ADR-015` §3's *"optional assets"*, in file order.
    pub fn assets(&self) -> impl Iterator<Item = &'a Member> {
        let members: &'a [Member] = self.package.members();
        members
            .iter()
            .filter(|m| m.target_class() == TargetClass::Asset.as_byte())
    }

    /// Parse the manifest — **and only from bytes that have been verified.**
    ///
    /// The [`VerifiedMember`] is not decoration. `ADR-015` §4.1's whole argument
    /// is that a manifest outside the signature could be rewritten to ask for
    /// `network = "egress"` and still verify; a parser that took `&[u8]` would
    /// let a caller re-introduce that by reading the member before checking it.
    /// The token cannot be constructed outside
    /// [`crate::package::read::MemberStream::finish`], so this signature makes
    /// the ordering structural rather than remembered.
    pub fn read_manifest(
        &self,
        verified: VerifiedMember<'_>,
        bytes: &[u8],
    ) -> Result<Manifest, PackageError> {
        if verified.member().index() != self.manifest.index() {
            return Err(PackageError::WrongMemberVerified {
                expected: self.manifest.index(),
                found: verified.member().index(),
            });
        }
        Manifest::parse(bytes).map_err(|fault| PackageError::MalformedManifest { fault })
    }

    /// `ADR-029` F6 for an application: refuse on `arch` before anything is
    /// installed.
    ///
    /// **There is no platform comparison and that is the ruling, not an
    /// omission** — ruled by the maintainer, 2026-08-24. [`Self::open`] has already
    /// refused a payload that carries one at all.
    pub fn accepts(&self, arch: &str) -> Result<(), PackageError> {
        let header = self.package.header();
        if header.arch() != arch {
            return Err(PackageError::WrongKind {
                expected: format!("an application for arch {arch:?}"),
                found: format!("one for {:?}", header.arch()),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
