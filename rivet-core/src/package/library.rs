//! What the metadata means for the `library` kind — **Shared Rivets**.
//!
//! Ruled 2026-08-31, from
//! `docs/design-candidates/Shared-Libraries-and-the-Build-Target.md` §4.4. A
//! library is *"a payload, not a new distribution channel"*: the same `.rvt`
//! container, the same header, the same signature, the same trust store, and
//! the same verifier. What differs is the interpretation — which is `ADR-029`
//! §11's whole argument, now made by a third case.
//!
//! # The four decisions this module holds
//!
//! | | |
//! | :-- | :-- |
//! | **Content addressing** | a library installs to `<volume>/lib/<name>-<version>-<digest>/`, never to a bare name and **never to `current`** |
//! | **The digest is the artifact's own** | [`identity`] is BLAKE3 over the signed region of the header, which carries every member's digest — so the path names the exact bytes and nothing has to be recomputed from parts |
//! | **A closure is declared, not discovered** | an application (or another library) names the references it needs; the installer refuses what it cannot satisfy, **naming what is missing** |
//! | **A reference is opaque** | the string in a manifest *is* the directory name. Nothing recomposes one from parts, so there is no second spelling to disagree |
//!
//! ## Why `current` is refused rather than merely unused
//!
//! §4.4: *"Never `current`; a library has no single current version, which is
//! the entire point."* An application's `current` link exists because exactly
//! one version of a program runs. Two applications needing two versions of one
//! library is the **ordinary** case this design exists to make ordinary, so a
//! link that selected one would be a conflict wearing a symlink.
//!
//! ## What content addressing buys, stated so it is not read as decoration
//!
//! §4.3.1 item 3 — Flatpak's bill. When a vulnerability lands, somebody has to
//! answer *"which machines are running a copy of this"*. A bare-name install
//! path makes that unanswerable; a digest in the path makes it a directory
//! listing. That is the argument for this being decided **before** the first
//! library ships rather than after.
//!
//! # What this does NOT provide, and it matters
//!
//! **Isolation.** [`crate::service`]'s resolution hands an application the
//! directories of its declared closure and nothing else, which decides what the
//! loader *finds*. It does not stop a program that wants to from opening a path
//! it can read: `<volume>/lib` is root-owned and world-searchable, because the
//! unprivileged uid an application runs as has to traverse it (`ADR-015` §4.2's
//! same argument about `/state/app`).
//!
//! Enforced resolution in the loader is Android's linker namespaces, and it
//! needs the namespace machinery `ADR-015` §6 records as not existing on this
//! build. **So this is resolution, not a sandbox**, and saying so is `ADR-005`
//! §5.4's rule: a partial promise about isolation is worse than none.
//!
//! **A stable platform ABI.** [`PUBLIC_LIBRARIES`] is the whole of what the
//! image promises. Everything else an application links comes from its own
//! closure, which is Android's `public.libraries.txt` lesson and §4.3's
//! *"subtract generality — but only the platform's half of it"*.

use crate::package::error::PackageError;
use crate::package::kind::Kind;
use crate::package::read::{Member, Package};

/// The library manifest, beside the objects it describes.
///
/// The same name an application's manifest has, deliberately: one name for
/// *"the typed metadata of this artifact"* across every kind, so that a reader
/// looking at an unpacked tree does not have to know the kind to find it.
pub const MANIFEST_NAME: &str = "manifest.toml";

/// Where library artifacts are installed, under a volume.
///
/// A sibling of `app` rather than a child of it (`ADR-015` §2, `ADR-031` §3):
/// a library is not owned by any one application, and putting it under one
/// would make the first installer of a shared library its landlord.
pub const LIB_DIR: &str = "lib";

/// The directory inside a library's install tree that holds the objects.
///
/// `<volume>/lib/<reference>/lib/libfoo.so.1`. The inner `lib` looks redundant
/// and is not: it is the directory a loader is pointed at, and keeping the
/// manifest and the retained header **outside** it means the loader's search
/// path contains objects and nothing else.
pub const OBJECT_DIR: &str = "lib";

/// The most objects one library artifact may carry.
///
/// The container's own [`crate::package::format::MAX_MEMBERS`] is the real
/// ceiling; this is the smaller, kind-specific one, and it exists so that a
/// refusal names *this* limit rather than a header's.
pub const MAX_OBJECTS: usize = 16;

/// The most references a closure may declare.
pub const MAX_CLOSURE: usize = 32;

/// The digest, as hex, in a [`Reference`].
///
/// The full 32 bytes rather than a prefix. A truncated digest would be shorter
/// and would invite the argument about how short is safe; the argument is worth
/// more than the characters.
pub const DIGEST_HEX: usize = 64;

/// The longest a [`Reference`] may be.
///
/// [`crate::package::application::MAX_VALUE`] is 96 and a reference is
/// `<name>-<version>-<64 hex>`, so a perfectly ordinary
/// `hello-rivet-0.1.0-g1641a7f-2c2548db-…` would not fit it. This is that bound
/// raised for the two keys that carry references, rather than raised for every
/// value — a manifest's other fields have no reason to grow.
pub const MAX_REFERENCE: usize = 160;

/// What the **image** promises an application it may link against.
///
/// Android's `public.libraries.txt` (§4.2), and deliberately tiny: *"every
/// symbol the platform promises is one it must keep promising across image
/// updates"* (§4.3). Everything else an application needs comes from its own
/// closure, where the operator owns it and an image update cannot move it.
///
/// ~~**musl's `libc.so` is the whole list.**~~ It was, until Mesa.
///
/// # What is on this list, restated — 2026-08-31
///
/// The list grew three times in one day and each addition arrived with its own
/// argument, which is how a principle turns into a pile of exceptions. The
/// maintainer asked for the rule instead: Rivet has to welcome C programs, so
/// the list is restated as the libraries a program cannot do without
/// (`.rulings/` R-355).
///
/// **Two groups, and each has a test a future candidate either passes or does
/// not.**
///
/// ## 1. The language runtime — what a compiled program IS
///
/// `libc.so`, `libgcc_s.so.1`, `libstdc++.so.6`.
///
/// **The test: could an application avoid this by writing different code?** For
/// these, no. They are not libraries an author chose; they are what the
/// compiler emits calls to before it emits anything else — unwinding, atomics
/// helpers, the C++ ABI, the C library itself. A C or C++ program that reached
/// this machine without them is not a program that failed to link something, it
/// is a program that cannot exist here.
///
/// This is the group that keeps Rivet welcoming to C and C++ programs, which is
/// the maintainer's point. A platform that shipped a GPU stack and no C++
/// runtime would be one where every C++ application carried its own
/// `libstdc++` — several megabytes each, several copies, and a version skew
/// nobody can see.
///
/// ## 2. The kernel's own userspace half — what must match the kernel
///
/// `libdrm.so.2`, `libEGL.so.1`, `libGLESv2.so.2`, `libgbm.so.1`.
///
/// **The test: must its version match the kernel's?** For these, yes.
/// `libdrm` *is* the DRM interface in userspace and its ioctls are the kernel's;
/// Mesa's GPU stack sits on top of it and speaks the same ABI. The maintainer
/// admitted Mesa on the ground that its version is dictated by the kernel's
/// (`.rulings/` R-355), and that clause is the whole of this group's
/// justification — it is why they belong in the image at all: `ADR-018`'s A/B
/// replaces them and the kernel **together, whole**, which a signed library
/// `.rvt` under `/state` could never do, because the operator owns its
/// lifetime.
///
/// # What this list is NOT
///
/// **Everything else is a choice, and a choice belongs to whoever made it.** A
/// JSON parser, an image codec, a database, Aura's own libraries: the
/// application picked one, could have picked another, and nothing about the
/// machine says which. Those arrive as signed library `.rvt`s in the
/// application's closure (`SPEC-02`), where an operator can see what is
/// installed and answer *which machines are running a copy of this*.
///
/// **And being ON the card is not being promised.** `libexpat.so.1` and
/// `libgallium-26.0.1.so` are both in `/lib` because Mesa needs them, and
/// neither is here. An application that links `libexpat` directly must ship its
/// own, even though a copy is sitting in the same directory the loader will
/// search. That is deliberate: what the platform *promises* is a contract it
/// cannot withdraw, and Mesa's private dependencies are Mesa's to change.
///
/// # The promise cannot be withdrawn, which is why the test exists
///
/// §4.3: *"every symbol the platform promises is one it must keep promising
/// across image updates."* `library/tests.rs` asserts this list exactly, so
/// adding to it is a deliberate act with a failing assertion in front of it —
/// and that tripwire has now fired twice and put the question to the maintainer
/// both times, which is the whole of what it is for.
pub const PUBLIC_LIBRARIES: &[&str] = &[
    // 1. The language runtime.
    "libc.so",
    "libgcc_s.so.1",
    "libstdc++.so.6",
    // 2. The kernel's userspace half.
    "libdrm.so.2",
    "libEGL.so.1",
    "libGLESv2.so.2",
    "libgbm.so.1",
];

/// The path the loader is found at, and the one every dynamic object names in
/// its `PT_INTERP`.
///
/// Not a choice: it is what `aarch64-linux-gcc` writes into a binary it links,
/// and the file has to be at that path or the kernel cannot start the program.
/// It is on the read-only root, so it arrives with an image and is A/B
/// protected like everything else there (`ADR-018`).
pub const LOADER_PATH: &str = "/lib/ld-musl-aarch64.so.1";

/// Byte 64 of a member record, for a `library`.
///
/// Kind-defined, exactly as [`crate::package::application::TargetClass`] and
/// [`crate::package::update::TargetClass`] are — the same byte meaning a third
/// thing, which is `ADR-029` §11 at its narrowest point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetClass {
    /// [`MANIFEST_NAME`]. Exactly one, and it is member 0.
    Manifest = 0,
    /// A shared object. **One or more** — unlike an application, which has
    /// exactly one program, a library artifact may carry a `.so` and the
    /// versioned name beside it.
    Object = 1,
    /// Anything else the artifact carries. Zero or more.
    Asset = 2,
}

impl TargetClass {
    #[must_use]
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Manifest),
            1 => Some(Self::Object),
            2 => Some(Self::Asset),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_byte(self) -> u8 {
        self as u8
    }
}

/// One element of a declared closure — and the directory the library installs
/// to, spelled once.
///
/// `<name>-<version>-<digest>`, where `<digest>` is [`DIGEST_HEX`] lowercase hex
/// characters. **The string is the identity.** Nothing recomposes one from
/// parts at install time or at start time, because two spellings of one
/// reference is exactly the class of bug content addressing exists to remove.
///
/// It is parseable *from the right* — the digest is the last [`DIGEST_HEX`]
/// characters and the `-` before it separates them from `<name>-<version>` —
/// so a reader can still say which library a directory holds. That is a
/// convenience for a person, and [`Reference::name_and_version`] is the only
/// thing that uses it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reference(String);

/// Why a reference could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceFault {
    /// Longer than [`MAX_REFERENCE`], or empty.
    BadLength,
    /// A byte outside `[A-Za-z0-9._-]`, or a leading `.` or `-`.
    BadCharacter,
    /// No `-<64 lowercase hex>` at the end.
    NoDigest,
    /// Nothing before the digest — `-<64 hex>` on its own names no library.
    NoName,
}

impl Reference {
    /// Compose one from a name, a version and the artifact's [`identity`].
    ///
    /// The **only** constructor that builds a reference from parts, and it lives
    /// beside the parser so that what is written and what is accepted cannot
    /// drift apart.
    pub fn compose(name: &str, version: &str, digest: &[u8; 32]) -> Result<Self, ReferenceFault> {
        let mut hex = String::with_capacity(DIGEST_HEX);
        for byte in digest {
            hex.push(nibble(byte >> 4));
            hex.push(nibble(byte & 0x0f));
        }
        Self::parse(&format!("{name}-{version}-{hex}"))
    }

    /// Read one back. See the type's own documentation for why this is strict.
    pub fn parse(text: &str) -> Result<Self, ReferenceFault> {
        check_component(text)?;
        if text.len() < DIGEST_HEX + 2 {
            return Err(ReferenceFault::NoDigest);
        }
        let (head, digest) = text.split_at(text.len() - DIGEST_HEX);
        if !digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(ReferenceFault::NoDigest);
        }
        let head = head.strip_suffix('-').ok_or(ReferenceFault::NoDigest)?;
        if head.is_empty() {
            return Err(ReferenceFault::NoName);
        }
        Ok(Self(text.to_string()))
    }

    /// The string, which is also the directory name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `<name>-<version>`, for a person reading a listing. Never used to
    /// resolve anything.
    #[must_use]
    pub fn name_and_version(&self) -> &str {
        // parse() guaranteed the shape, so this cannot underflow.
        &self.0[..self.0.len() - DIGEST_HEX - 1]
    }

    /// The digest, as the hex it was written in.
    #[must_use]
    pub fn digest_hex(&self) -> &str {
        &self.0[self.0.len() - DIGEST_HEX..]
    }

    /// `<name>` and `<version>`, split apart — for **retention** and for
    /// nothing else. `SPEC-02` §12, `.rulings/` R-202.
    ///
    /// # This does not make a reference composable
    ///
    /// `SPEC-02` §3 is emphatic that nothing recomposes a reference from parts,
    /// and this does not: it reads, it is fallible, and no constructor takes
    /// what it returns. What it is for is the one question retention has to ask
    /// — *are these two directories two builds of one library, and which is
    /// newer* — which cannot be answered without splitting the head.
    ///
    /// # The split IS `ADR-027` §7's rule, not a second spelling of it
    ///
    /// `crate::artifact::version_token_at` decides which `-`-separated
    /// token starts the version, and this calls it over the same tokens
    /// [`crate::artifact::parse_name`] would split a payload's file name
    /// into. One implementation read back at two call sites is `ADR-011`'s
    /// whole argument: a library reference and a payload name that disagreed
    /// about where a product ends would be retention grouping one of them
    /// wrongly.
    ///
    /// **A product whose own name is itself shaped like a version is still
    /// the one case this cannot get right, and the cost is bounded.** A
    /// library named so that an earlier token is three dot-separated digit
    /// runs — `opengl-4.5.1-shim-1.0.0-<digest>` — reads as product `opengl`
    /// at version `4.5.1-shim-1.0.0`; that version is then unreadable, so
    /// `crate::artifact::evicted` retains it apart from everything it cannot
    /// order and it is never evicted by a version it was not compared with.
    /// The same is already true of a payload's file name, for the same
    /// reason: one rule, one blind spot, rather than two.
    ///
    /// `None` for a head carrying nothing of the kind — `libgreet-<digest>` names
    /// no version — and the caller then treats the whole head as a product of
    /// its own, which is the safe direction.
    #[must_use]
    pub fn product_and_version(&self) -> Option<(&str, &str)> {
        let head = self.name_and_version();
        let tokens: Vec<&str> = head.split('-').collect();
        let version_at = crate::artifact::version_token_at(&tokens)?;
        // The byte each product token occupies plus the hyphen after it —
        // `version_token_at` answers in tokens, and this is only the
        // arithmetic to turn that back into the offset `head` is sliced at.
        let at: usize = tokens[..version_at].iter().map(|t| t.len() + 1).sum();
        Some((&head[..at - 1], &head[at..]))
    }

    /// **Which library product this build is a build of** — the `<name>` half
    /// of [`Self::product_and_version`], or the whole head when it carries no
    /// version, which is then a product of its own.
    ///
    /// The grouping retention keeps one build per (`SPEC-02` §12) and the one
    /// `apply uninstall-library <product>` selects by: two questions about one
    /// grouping, so one function answers both and they cannot disagree about
    /// which directories are builds of one library.
    #[must_use]
    pub fn product(&self) -> &str {
        self.product_and_version()
            .map_or_else(|| self.name_and_version(), |(product, _)| product)
    }
}

/// **The rules every library name obeys, digest or no digest** — the bound on
/// length, the alphabet `[A-Za-z0-9._-]`, no leading `.` or `-`, and no `..`.
///
/// [`Reference::parse`] applies these before it looks for a digest, and
/// `crate::uninstall_library` applies them to a bare product name, so a name an
/// operator types is held to exactly the rules a reference's name half obeys.
/// Whatever passes is one path component and cannot traverse: the alphabet has
/// no `/`, and `..` is refused by name.
///
/// # Errors
///
/// [`ReferenceFault::BadLength`] for an empty text or one longer than
/// [`MAX_REFERENCE`], and [`ReferenceFault::BadCharacter`] for the rest.
pub fn check_component(text: &str) -> Result<(), ReferenceFault> {
    if text.is_empty() || text.len() > MAX_REFERENCE {
        return Err(ReferenceFault::BadLength);
    }
    // A leading `.` or `-` for the same reason `ServiceName` refuses one:
    // this becomes a directory name, and both spellings mean something to
    // a shell, to a glob and to a person reading a listing.
    if text.starts_with('.') || text.starts_with('-') {
        return Err(ReferenceFault::BadCharacter);
    }
    for byte in text.bytes() {
        let ok = byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'_' || byte == b'-';
        if !ok {
            return Err(ReferenceFault::BadCharacter);
        }
    }
    // `..` would be a traversal in a path composed from this. The character
    // set above already forbids `/`, so this is the only spelling left.
    if text.contains("..") {
        return Err(ReferenceFault::BadCharacter);
    }
    Ok(())
}

impl core::fmt::Display for Reference {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

const fn nibble(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        _ => (b'a' + value - 10) as char,
    }
}

/// The artifact's identity — BLAKE3 over the signed region of its header.
///
/// **Why this region and not the whole file.** `ADR-029` §3 signs
/// `[0 .. SIGNED_BYTES)`, and that region carries the product, the version, the
/// kind, the architecture **and every member's digest**. So a digest over it
/// covers the whole artifact transitively, is computable from 4 KiB rather than
/// from the payload, and is *the same number the signature already attests to*.
/// A second hash over the whole file would be a second identity for one thing,
/// which is `ADR-011`'s objection in miniature.
///
/// It is computed by whoever holds the header: `cargo rivet package` prints it
/// so an author can declare it, and the installer recomputes it rather than
/// believing a filename.
pub fn identity(head: &[u8], digest: &mut dyn crate::package::port::Digest) -> Option<[u8; 32]> {
    let signed = head.get(..crate::package::format::SIGNED_BYTES)?;
    digest.start();
    digest.write(signed);
    Some(digest.finish())
}

// --- the manifest ----------------------------------------------------------

const KEYS: [&str; 5] = [
    "kind", "soname", "needs",
    // **`PLAN-Feature-Set-2026-09-12` §6.2's addon primitive, `ADR-053`,
    // 2026-09-13** — the class of addon this library provides. Appended at
    // the end for the reason an application manifest's keys are: the
    // canonical order is this array, and a key at the end cannot make an
    // older manifest out-of-order.
    //
    // **OPTIONAL, and its absence means *this library provides no addon
    // class*** — the closed end of the same axis `crate::package::
    // application::Manifest::accepts_class` sits on. No existing `.rvt` is
    // invalidated: a library built before this key existed is still resolved
    // by every explicit `needs` and `library` reference exactly as before,
    // and simply joins no application by class.
    "provides-class",
    // **`.rulings/` R-201, 2026-09-19** — which of this library's objects is
    // the addon. Appended after `provides-class` for that key's own reason,
    // one section later: the canonical order is this array and a key at the
    // end cannot put an already-signed manifest out of order.
    //
    // Named for the key its value is drawn from rather than for the concept
    // it serves: the value must be one of `soname`'s entries, character for
    // character, and saying `addon-soname` puts that cross-reference in the
    // name where a reader meets it before they meet the checker.
    "addon-soname",
];

/// A library's typed metadata.
///
/// Deliberately small. An application's manifest is a **permission request**
/// (`ADR-015` §3.3) and grows keys as the sandbox grows; a library asks for
/// nothing and runs as nobody. What it has to say is what it *provides* and
/// what it in turn *needs*.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    soname: Vec<String>,
    needs: Vec<Reference>,
    /// **The class of addon this library provides** —
    /// `PLAN-Feature-Set-2026-09-12` §6.2, `ADR-053`.
    ///
    /// `None` is the ordinary answer and means *this library joins no
    /// application by class*, which is what every library built before
    /// `ADR-053` says and what a library that is only ever named in an
    /// explicit `library` closure still says. Matched against an
    /// application's [`crate::package::application::Manifest::
    /// accepts_class`] — both signed declarations must agree, character for
    /// character, or nothing joins.
    provides_class: Option<String>,
    /// **Which of [`Self::soname`]'s objects is the addon** — `.rulings/`
    /// R-201.
    ///
    /// A library artifact may carry sixteen objects and exactly one of them
    /// is the thing a host loads; the rest are its own private dependencies.
    /// This names that one, by the same string `soname` lists it under.
    ///
    /// `None` is legal in exactly one shape — **a library carrying one
    /// object**, where there is nothing to choose between and a key would be
    /// a second spelling of a fact the manifest already states once. A
    /// library that carries several and provides a class is refused without
    /// it; see `check_addon_choice` below for the whole of the rule and the
    /// argument behind it.
    addon_soname: Option<String>,
}

/// Why a library manifest could not be read. Mirrors
/// [`crate::package::application::ManifestFault`] and is a separate type for
/// the reason the kinds are separate modules: the keys are different, so a
/// shared error would name keys that do not exist in half its uses.
///
/// **Not `Copy` since `.rulings/` R-201.** Three of these carry the object
/// names the manifest declares, because the question an author has the
/// instant one of them fires is *which one should I have said*, and a
/// refusal that cannot answer it makes them go and unpack the artifact to
/// find out. A clone of a handful of short strings on a path that is already
/// refusing is not a cost worth trading that for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestFault {
    NotText,
    NotKeyValue,
    UnknownKey,
    MissingKey { key: &'static str },
    OutOfOrder { key: &'static str },
    BadValue { key: &'static str },
    ListTooLong { key: &'static str },
    /// `soname` was present and empty. A library that provides no object is not
    /// a library, and the emptiness would only be discovered by a loader.
    NoSoname,
    ValueTooLong,
    BadCharacter,
    TooLarge,
    BadReference { fault: ReferenceFault },
    /// **A library carrying several objects declared a class and named none
    /// of them** — `.rulings/` R-201.
    ///
    /// Carries every object the library declares, in the order it declares
    /// them, so the refusal can print the list the author has to choose from.
    AddonObjectUnnamed { objects: Vec<String> },
    /// `addon-soname` names an object this library does not carry.
    ///
    /// Carries both sides for the same reason: a typo in one of sixteen
    /// filenames is found by reading the two lists beside each other, and the
    /// refusal is the cheapest place to put them.
    AddonObjectNotCarried {
        named: String,
        objects: Vec<String>,
    },
    /// `addon-soname` without `provides-class`.
    ///
    /// Naming an addon object in a library that joins no application by class
    /// names a thing nothing will ever load. It is refused rather than
    /// ignored on `ADR-015` §6's rule: a declaration that cannot be honoured
    /// is said so at the moment it is made, and the alternative here is an
    /// author who believes they have shipped an addon and has not.
    AddonObjectWithoutClass { named: String },
}

impl core::fmt::Display for ManifestFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotText => write!(f, "it is not printable text"),
            Self::NotKeyValue => write!(f, "a line is neither blank, a comment, nor `key = value`"),
            Self::UnknownKey => write!(f, "a key this build does not define"),
            Self::MissingKey { key } => write!(f, "`{key}` is required and is absent"),
            Self::OutOfOrder { key } => {
                write!(f, "`{key}` appears twice or out of the canonical order")
            }
            Self::BadValue { key } => write!(f, "`{key}` does not hold a value it may hold"),
            Self::ListTooLong { key } => write!(f, "`{key}` has more entries than are allowed"),
            Self::NoSoname => write!(
                f,
                "`soname` is empty, and a library that provides no object is not a library"
            ),
            Self::ValueTooLong => write!(f, "a value is longer than its key permits"),
            Self::BadCharacter => write!(f, "a value holds a byte no manifest may hold"),
            Self::TooLarge => write!(f, "the manifest is larger than any manifest may be"),
            Self::BadReference { fault } => write!(f, "a library reference is unreadable: {fault}"),
            Self::AddonObjectUnnamed { objects } => write!(
                f,
                "this library provides an addon class and carries {} objects, so it must say \
                 which one of them is the addon: `addon-soname` names it, and the objects it \
                 carries are {}",
                objects.len(),
                objects.join(", ")
            ),
            Self::AddonObjectNotCarried { named, objects } => write!(
                f,
                "`addon-soname` names {named:?}, which is not an object this library carries; \
                 it carries {}",
                objects.join(", ")
            ),
            Self::AddonObjectWithoutClass { named } => write!(
                f,
                "`addon-soname` names {named:?} in a library that provides no addon class, so \
                 nothing would ever load it — a library names its addon object because it \
                 provides a class, never on its own"
            ),
        }
    }
}

impl core::fmt::Display for ReferenceFault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadLength => write!(f, "it is empty, or longer than {MAX_REFERENCE} bytes"),
            Self::BadCharacter => write!(
                f,
                "it holds a byte outside [A-Za-z0-9._-], or begins with `.` or `-`, or contains `..`"
            ),
            Self::NoDigest => write!(
                f,
                "it does not end in `-` followed by {DIGEST_HEX} lowercase hex characters"
            ),
            Self::NoName => write!(f, "there is nothing before the digest, so it names no library"),
        }
    }
}

impl Manifest {
    /// A library providing `soname` and needing nothing.
    pub fn new(soname: Vec<String>) -> Result<Self, ManifestFault> {
        if soname.is_empty() {
            return Err(ManifestFault::NoSoname);
        }
        if soname.len() > MAX_OBJECTS {
            return Err(ManifestFault::ListTooLong { key: "soname" });
        }
        for name in &soname {
            check_soname(name)?;
        }
        Ok(Self {
            soname,
            needs: Vec::new(),
            provides_class: None,
            addon_soname: None,
        })
    }

    pub fn with_needs(mut self, needs: Vec<Reference>) -> Result<Self, ManifestFault> {
        if needs.len() > MAX_CLOSURE {
            return Err(ManifestFault::ListTooLong { key: "needs" });
        }
        needs.clone_into(&mut self.needs);
        Ok(self)
    }

    /// Declare the class of addon this library provides, and **which of its
    /// objects is that addon** — `PLAN-Feature-Set-2026-09-12` §6.2,
    /// `ADR-053`, `.rulings/` R-201.
    ///
    /// # Why the two arrive together rather than as two setters
    ///
    /// They are one decision — *this library provides class C through object
    /// O* — and the rule R-201 lays down spans both of them plus the `soname`
    /// list `new` already took. Two setters would be checkable only in one
    /// call order: `with_provides_class` on a multi-object library would have
    /// to refuse for naming no object, or accept and leave the invariant to
    /// something later. One call has every part of the question in front of
    /// it, so a manifest this type will not [`render`](Self::render) is a
    /// manifest it never built.
    ///
    /// `(None, None)` is a library that joins no application by class, which
    /// is every library emitted before `ADR-053` and most emitted since.
    ///
    /// # Errors
    ///
    /// [`ManifestFault::BadValue`] for a class that is not
    /// [`crate::package::application::is_class_name`], or longer than
    /// [`crate::package::application::MAX_VALUE`]; and whatever
    /// `check_addon_choice` — this module's own, below — refuses about the
    /// pairing, which is the whole of `.rulings/` R-201 and is documented
    /// there rather than restated here.
    pub fn with_addon(
        mut self,
        class: Option<String>,
        addon_soname: Option<String>,
    ) -> Result<Self, ManifestFault> {
        if let Some(text) = &class {
            check_class(text)?;
        }
        check_addon_choice(&self.soname, class.as_deref(), addon_soname.as_deref())?;
        self.provides_class = class;
        self.addon_soname = addon_soname;
        Ok(self)
    }

    #[must_use]
    pub fn soname(&self) -> &[String] {
        &self.soname
    }

    #[must_use]
    pub fn needs(&self) -> &[Reference] {
        &self.needs
    }

    /// The class of addon this library provides, if it provides one —
    /// `PLAN-Feature-Set-2026-09-12` §6.2.
    ///
    /// **`None` is *joins no application by class*.** See the field's own
    /// documentation for what a match requires.
    #[must_use]
    pub fn provides_class(&self) -> Option<&str> {
        self.provides_class.as_deref()
    }

    /// The `addon-soname` key exactly as it was written, or absent.
    ///
    /// **Absent is not *no addon*** — a one-object library that provides a
    /// class writes no key and still has an addon object. Ask
    /// [`Self::addon_object`] for that; this is the key, for a reader or a
    /// renderer that wants the manifest's own words.
    #[must_use]
    pub fn addon_soname(&self) -> Option<&str> {
        self.addon_soname.as_deref()
    }

    /// **The object a host is joined to, if this library has one** —
    /// `.rulings/` R-201.
    ///
    /// The whole of the rule, in one place, so that no caller re-derives it:
    ///
    /// * a library that provides no class has no addon object, whatever it
    ///   carries — `None`;
    /// * a library that provides a class and names one object has that one;
    /// * a library that provides a class, carries exactly one object and
    ///   names none has that object, because there was nothing to choose
    ///   between and R-201 asks for no key where no choice exists.
    ///
    /// The remaining shape — a class, several objects and no name — is
    /// refused by [`Self::parse`] and by [`Self::with_addon`], so it does not
    /// reach here. It is answered `None` rather than unwrapped for the reason
    /// this crate answers every such shape that way: a panic in the
    /// supervisor is a worse way to find out that a grammar widened than a
    /// library that quietly joins nothing.
    #[must_use]
    pub fn addon_object(&self) -> Option<&str> {
        self.provides_class.as_ref()?;
        match &self.addon_soname {
            Some(named) => Some(named),
            None => match self.soname.as_slice() {
                [only] => Some(only),
                _ => None,
            },
        }
    }

    /// The canonical text — the bytes that go into [`MANIFEST_NAME`].
    ///
    /// `kind` is written even though there is exactly one legal value, for
    /// `ADR-029` §11's reason one level in: an unpacked tree on a board is read
    /// by a person, and a manifest that did not say what it was would be
    /// distinguishable from an application's only by which keys are absent.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("# Rivet library manifest — Shared Rivets, SPEC-05 §4.\n");
        out.push_str("# Emitted by `cargo rivet package --library`; not written by hand.\n");
        out.push_str("kind = \"library\"\n");
        render_list(&mut out, "soname", &self.soname);
        let needs: Vec<String> = self.needs.iter().map(|r| r.as_str().to_string()).collect();
        render_list(&mut out, "needs", &needs);
        // **Written last and only when set**, `soname` and `needs`' being
        // required is why this is the one that is not: a library built
        // before `ADR-053` renders and parses byte-identically.
        if let Some(class) = &self.provides_class {
            out.push_str(&format!("provides-class = \"{class}\"\n"));
        }
        // **The key is written only when it was given**, `.rulings/` R-201: a
        // one-object library needs none, so writing one out of the `soname`
        // list would put a second copy of a fact into the manifest and make
        // the absence of the key mean nothing.
        if let Some(soname) = &self.addon_soname {
            out.push_str(&format!("addon-soname = \"{soname}\"\n"));
        }
        out
    }

    /// Read the canonical text back, on
    /// [`crate::package::application::Manifest::parse`]'s terms: a subset of
    /// TOML, accepting exactly what [`Self::render`] emits and nothing wider.
    pub fn parse(bytes: &[u8]) -> Result<Self, ManifestFault> {
        if bytes.len() > crate::package::application::MAX_MANIFEST_BYTES {
            return Err(ManifestFault::TooLarge);
        }
        let text = core::str::from_utf8(bytes).map_err(|_| ManifestFault::NotText)?;

        let mut kind_seen = false;
        let mut soname: Option<Vec<String>> = None;
        let mut needs: Option<Vec<Reference>> = None;
        // `ADR-053`'s footing: absent means this library joins no application
        // by class.
        let mut provides_class: Option<String> = None;
        // `.rulings/` R-201. Absent is only legal for a library with one
        // object; `check_addon_choice` below is where that is decided, once
        // every key has been read, because the answer needs all three.
        let mut addon_soname: Option<String> = None;
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
                    if scalar(name, value, crate::package::application::MAX_VALUE)? != "library" {
                        return Err(ManifestFault::BadValue { key: name });
                    }
                    kind_seen = true;
                }
                "soname" => {
                    let mut names = Vec::new();
                    for element in
                        elements(name, value, crate::package::application::MAX_VALUE, MAX_OBJECTS)?
                    {
                        check_soname(element)?;
                        names.push(element.to_string());
                    }
                    if names.is_empty() {
                        return Err(ManifestFault::NoSoname);
                    }
                    soname = Some(names);
                }
                "needs" => {
                    let mut refs = Vec::new();
                    for element in elements(name, value, MAX_REFERENCE, MAX_CLOSURE)? {
                        refs.push(
                            Reference::parse(element)
                                .map_err(|fault| ManifestFault::BadReference { fault })?,
                        );
                    }
                    needs = Some(refs);
                }
                "provides-class" => {
                    let text = scalar(
                        name,
                        value,
                        crate::package::application::MAX_VALUE,
                    )?;
                    check_class(text)?;
                    provides_class = Some(text.to_string());
                }
                "addon-soname" => {
                    // Held to a SONAME's rules before it is compared to the
                    // list, so that a value that could never be a filename is
                    // refused as the malformed value it is rather than as a
                    // missing member of `soname`.
                    let text =
                        scalar(name, value, crate::package::application::MAX_VALUE)?;
                    check_soname(text)?;
                    addon_soname = Some(text.to_string());
                }
                _ => return Err(ManifestFault::UnknownKey),
            }
        }

        if !kind_seen {
            return Err(ManifestFault::MissingKey { key: "kind" });
        }
        let soname = soname.ok_or(ManifestFault::MissingKey { key: "soname" })?;
        // **Checked here rather than in the `addon-soname` arm**, `.rulings/`
        // R-201: the rule reads three keys at once, and no arm in the loop
        // above has all three in front of it — `soname` may not have been
        // read when the class was, and a manifest that arrived with the keys
        // in some other order would have been refused as out of order before
        // it got here anyway.
        check_addon_choice(&soname, provides_class.as_deref(), addon_soname.as_deref())?;
        Ok(Self {
            soname,
            needs: needs.ok_or(ManifestFault::MissingKey { key: "needs" })?,
            // `ok_or`-free, `ADR-053`'s footing: absence means joins no
            // application by class.
            provides_class,
            addon_soname,
        })
    }
}

/// **A library that provides a class NAMES the object it joins by** —
/// `.rulings/` R-201, 2026-09-19.
///
/// The one place the rule is written, so that the emitter, the parser and
/// [`Manifest::addon_object`] cannot come to three slightly different
/// readings of it.
///
/// # What it refuses, and why each one
///
/// * **A class, several objects, and no `addon-soname`.** This is the ruling.
///   The rule it replaced took the first entry of `soname`, which made a
///   build product's ordering into a contract two signed manifests depend on
///   — and it fails silently and much later: a reordered build emits a
///   perfectly valid artifact that joins the wrong object, and nothing says
///   so until something behaves oddly on a board. The refusal names every
///   object the library carries, because the question the author has at that
///   instant is *which one should I have said*.
/// * **An `addon-soname` naming an object the library does not carry.** A
///   name with nothing behind it would install, sit in the right directory
///   and be handed to a host that cannot open it — `ADR-015` §6's moment to
///   say so is here, where the list to compare against is in the same file.
/// * **An `addon-soname` with no class.** Nothing would ever load it. See
///   [`ManifestFault::AddonObjectWithoutClass`].
///
/// # What it deliberately allows
///
/// **A class, one object, and no key.** There is nothing to choose between,
/// so a key would be a second spelling of a fact the manifest already states
/// — and it is the shape every addon in this project ships as today, which
/// is why R-201 needs no compatibility shim to keep them working.
fn check_addon_choice(
    soname: &[String],
    class: Option<&str>,
    addon_soname: Option<&str>,
) -> Result<(), ManifestFault> {
    match (class, addon_soname) {
        (None, Some(named)) => Err(ManifestFault::AddonObjectWithoutClass {
            named: named.to_string(),
        }),
        (_, Some(named)) => match soname.iter().any(|s| s == named) {
            true => Ok(()),
            false => Err(ManifestFault::AddonObjectNotCarried {
                named: named.to_string(),
                objects: soname.to_vec(),
            }),
        },
        (Some(_), None) if soname.len() > 1 => Err(ManifestFault::AddonObjectUnnamed {
            objects: soname.to_vec(),
        }),
        (_, None) => Ok(()),
    }
}

/// The rule an addon class name is held to, restated for a library's own
/// error type — `crate::package::application::is_class_name`'s predicate,
/// applied on both sides of `ADR-053`'s match.
fn check_class(text: &str) -> Result<(), ManifestFault> {
    if text.is_empty() || text.len() > crate::package::application::MAX_VALUE {
        return Err(ManifestFault::BadValue {
            key: "provides-class",
        });
    }
    if !crate::package::application::is_class_name(text) {
        return Err(ManifestFault::BadValue {
            key: "provides-class",
        });
    }
    Ok(())
}

/// A SONAME is a filename the loader will look for, so it is held to a
/// filename's rules and no wider. No `/`, no `.` or `..`, printable ASCII.
fn check_soname(text: &str) -> Result<(), ManifestFault> {
    const KEY: &str = "soname";
    if text.is_empty() || text.len() > crate::package::application::MAX_VALUE {
        return Err(ManifestFault::BadValue { key: KEY });
    }
    if text == "." || text == ".." || text.starts_with('.') || text.starts_with('-') {
        return Err(ManifestFault::BadValue { key: KEY });
    }
    for byte in text.bytes() {
        let ok = byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'_' || byte == b'-' || byte == b'+';
        if !ok {
            return Err(ManifestFault::BadCharacter);
        }
    }
    Ok(())
}

fn render_list(out: &mut String, key: &str, values: &[String]) {
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

fn scalar<'a>(key: &'static str, value: &'a str, cap: usize) -> Result<&'a str, ManifestFault> {
    let inner = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .ok_or(ManifestFault::BadValue { key })?;
    if inner.len() > cap {
        return Err(ManifestFault::ValueTooLong);
    }
    for byte in inner.bytes() {
        if !(0x21..=0x7e).contains(&byte) || byte == b'"' || byte == b'\\' {
            return Err(ManifestFault::BadCharacter);
        }
    }
    Ok(inner)
}

fn elements<'a>(
    key: &'static str,
    value: &'a str,
    cap: usize,
    max: usize,
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
        out.push(scalar(key, part, cap)?);
        if out.len() > max {
            return Err(ManifestFault::ListTooLong { key });
        }
    }
    Ok(out)
}

// --- the package as a library ----------------------------------------------

/// A [`Package`] read as a library.
///
/// Holding one means the kind is `library`, the header carries no `platform`
/// (an architecture-shaped artifact, exactly as an application is — `ADR-027`
/// §9), member 0 is the manifest, and there is at least one object. It does
/// **not** mean any member's content has been checked; that is
/// [`Package::verify_member`], per member, at the point it is about to be used.
#[derive(Debug, PartialEq, Eq)]
pub struct Library<'a> {
    package: &'a Package,
    manifest: &'a Member,
    objects: Vec<&'a Member>,
}

impl<'a> Library<'a> {
    pub fn open(package: &'a Package) -> Result<Self, PackageError> {
        package.require_kind(&Kind::library())?;

        if let Some(found) = package.header().platform() {
            return Err(PackageError::UnexpectedPlatform {
                found: found.to_string(),
            });
        }

        let mut manifest: Option<&Member> = None;
        let mut objects: Vec<&Member> = Vec::new();

        let mut manifests = 0usize;
        for member in package.members() {
            if member.variant() != crate::package::application::VARIANT_NONE {
                return Err(PackageError::UnknownVariant {
                    index: member.index(),
                    found: member.variant(),
                });
            }
            let class = TargetClass::from_byte(member.target_class()).ok_or(
                PackageError::UnknownTargetClass {
                    index: member.index(),
                    found: member.target_class(),
                },
            )?;
            match class {
                TargetClass::Manifest => {
                    manifests += 1;
                    if manifests > 1 {
                        return Err(PackageError::ManifestMemberCount { found: manifests });
                    }
                    if member.index() != 0 {
                        return Err(PackageError::ManifestNotFirst {
                            index: member.index(),
                        });
                    }
                    if member.name() != MANIFEST_NAME {
                        return Err(PackageError::ManifestMisnamed {
                            name: member.name().to_string(),
                        });
                    }
                    manifest = Some(member);
                }
                TargetClass::Object => {
                    objects.push(member);
                    if objects.len() > MAX_OBJECTS {
                        return Err(PackageError::ObjectMemberCount {
                            found: objects.len(),
                        });
                    }
                }
                TargetClass::Asset => {}
            }
        }

        let manifest = manifest.ok_or(PackageError::ManifestMemberCount { found: 0 })?;
        if objects.is_empty() {
            return Err(PackageError::ObjectMemberCount { found: 0 });
        }
        Ok(Self {
            package,
            manifest,
            objects,
        })
    }

    #[must_use]
    pub fn package(&self) -> &'a Package {
        self.package
    }

    /// The library's name, from the header's `product`. An application takes
    /// its name from the same field and for the same reason: the installer has
    /// no directory yet, because it is the thing deciding on one.
    #[must_use]
    pub fn name(&self) -> &'a str {
        self.package.header().product()
    }

    #[must_use]
    pub fn version(&self) -> &'a str {
        self.package.header().version()
    }

    #[must_use]
    pub fn manifest_member(&self) -> &'a Member {
        self.manifest
    }

    #[must_use]
    pub fn objects(&self) -> &[&'a Member] {
        &self.objects
    }

    /// Every member that is neither the manifest nor an object.
    pub fn assets(&self) -> impl Iterator<Item = &'a Member> + '_ {
        self.package
            .members()
            .iter()
            .filter(|m| m.target_class() == TargetClass::Asset.as_byte())
    }

    /// The same architecture check an application makes, and it is the machine's
    /// question rather than the artifact's: a signature says an artifact is
    /// ours, never that it is for this board (`ADR-015` §4.4).
    pub fn accepts(&self, arch: &str) -> Result<(), PackageError> {
        let found = self.package.header().arch();
        if found == arch {
            return Ok(());
        }
        // The same variant `Application::accepts` uses, with a library's noun.
        // A second variant meaning the same thing would be a second vocabulary
        // for one refusal, which is `ADR-006` §6.4's objection.
        Err(PackageError::WrongKind {
            expected: format!("a library for arch {arch:?}"),
            found: format!("one for {found:?}"),
        })
    }
}

#[cfg(test)]
mod tests;
