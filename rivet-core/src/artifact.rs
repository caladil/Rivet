//! **The machine's artifact store** — a digest-keyed object store under
//! [`crate::state::ARTIFACTS`], holding what this machine installed and what is
//! one step from being installed. `ADR-052` §7.
//!
//! # What it is for, in one sentence
//!
//! `ADR-052`'s organising rule is *"nothing reaches a device without passing
//! through the repository"*, and a machine that could be handed a path could
//! always be handed a path to something the repository never saw. So the
//! machine keeps its own store, the transfer fills it, and
//! `crate::operation::install` names an object in it. **A digest is the only
//! thing an install can be asked for**, and this module is where a digest
//! becomes a place on a card.
//!
//! # This module performs no I/O, and that is the whole of its shape
//!
//! It is `crate::repo`'s posture: the arithmetic between what is declared and
//! what is there, with every syscall in the caller. Three callers do the I/O
//! and they do not share a port — `cmd/rivet-mcpd` writes with `std::fs`,
//! `cmd/rivet` writes through its own `RequestStore`, and `cmd/rivet-sysd`
//! reads through its `Kernel`. Giving this module a fourth port so it could
//! call them would be a seam with three adapters and nothing behind it; giving
//! it the *paths* and the *decisions* is the part all three need and the part
//! that must not be spelled three times.
//!
//! # The layout
//!
//! ```text
//! /state/artifacts/
//!   objects/<digest>/<name>.rvt      a stored artifact
//!   incoming/<digest>/<name>.rvt     a transfer that has not finished
//! ```
//!
//! **One directory per object, holding one file.** Two decisions are in that:
//!
//! * **The digest is the directory and not the file**, so an object arrives by
//!   `rename(2)` of a directory that is already complete. A transfer that dies
//!   leaves something under `incoming/` that nothing resolves and a sweep can
//!   remove; it can never leave a half-written file under a name an installer
//!   would pick up. `cmd/rivet-mcpd`'s bulk endpoint made the same argument for
//!   writing beside the target and renaming, and this is that argument with the
//!   unit of atomicity moved up one level so that the *name* lands with the
//!   bytes.
//! * **The `ADR-027` name is kept**, rather than the file being called
//!   `<digest>.rvt`. `ADR-027` §3.2.1 makes a payload's name a claim about the
//!   commit that built it, and `cmd/rivet-mcpd`'s bulk endpoint already refused
//!   to throw that away — *"a content-addressed path would … leave the
//!   installer describing an update by hash"*. The digest is the key and the
//!   name is what the key is holding; both survive.
//!
//! # Retention — three newest per product
//!
//! [`KEEP`](crate::artifact::KEEP). The reason is the maintainer's: **rolling back becomes local**
//! rather than a trip to the workstation, and growth is bounded at three times
//! the installed footprint rather than being unbounded. `R-135` predicted this
//! rule would be needed and explicitly did not make it; eighty-six payloads had
//! to be removed by hand from a build directory before it was.
//!
//! ### "Newest" is the highest VERSION, and arrival is only the tie-break
//!
//! **Retention chooses between versions of one product, so it orders by
//! version.** `ADR-027` §7's name carries one, [`parse_name`](crate::artifact::parse_name)
//! reads it back, and [`version_order`](crate::artifact::version_order) compares
//! its numeric core componentwise — so `0.3.10` is newer than `0.3.9`, which
//! every string comparison gets backwards.
//!
//! **Only the numeric core is ordered, and the rest of the version deliberately
//! is not.** `ADR-027` §3 appends a `-g<short>`, a `-dirty` and a payload digest
//! as **build metadata** — dash-separated rather than SemVer's `+` because `+`
//! is a nuisance in a URL and on a FAT partition, not because those tokens are
//! pre-releases. Two dirty builds of one commit have no order, which is why the
//! payload digest exists to tell them apart, and a tag has no order against a
//! commit carrying the same crate version. A comparison that invented one would
//! be confidently wrong, and reading `-g3d28ede` as a SemVer pre-release would
//! order a described build **below** the bare tag, which is backwards. So
//! everything from the first `-` onward is ignored for ordering and two equal
//! cores fall through to arrival.
//!
//! **[`Stored::added`](crate::artifact::Stored::added) is the tie-break, not the
//! rule.** It is whatever ordinal the caller can observe — the object
//! directory's modification time, on a real machine. `ADR-021` is why it is
//! enough for a tie-break and could not be the
//! rule: a Pi has no RTC, so its clock is a **floor** restored from
//! `/state/clock` with one-second granularity, and a burst of deploys over a LAN
//! lands inside one second of it. Every artifact in the burst then carries the
//! same `added`, and a rule that ordered by it would be ordering by whatever
//! broke the tie. Ties in `added` are broken by digest, so that one answer comes
//! out rather than whichever the filesystem listed first — and because a digest
//! bears no relation to a version, that tie-break is only ever safe beneath the
//! version, never in place of it.
//!
//! **Re-storing an old artifact does NOT make it the newest.** An operator who
//! sends `0.1.0` back to a board already holding `0.2.0`, `0.3.0` and `0.4.0`
//! still keeps the build he just sent — the arriving object wins its own group,
//! below — and what falls off is `0.2.0`, the lowest version of the rest, rather
//! than `0.4.0`, the one he is most likely to want next.
//!
//! ### A name this build cannot parse is its own product
//!
//! [`product_of`](crate::artifact::product_of) falls back to the whole file name. That is the safe
//! direction — an unparseable name forms a group of one and is never counted
//! against some other product's three — and the cost is stated rather than
//! hidden: a caller that stored many differently-named unparseable artifacts
//! would fill `/state`, and nothing here would drop any of them. Whoever can do
//! that already holds a client certificate this fleet issued.
//!
//! ### A version this build cannot order is retained APART from those it can
//!
//! To reach this case a name must have parsed as `ADR-027` §7's grammar and
//! then carried a version whose core is not dot-separated numbers.
//! `cargo rivet package` composes its version from a crate version, which Cargo
//! requires to begin `MAJOR.MINOR.PATCH`, so what it composes always reads;
//! `cargo rivet update` takes an update's version as typed — so `0.1.84rc1` on a
//! 700 MB system update is the realistic way one arrives.
//!
//! **It keeps its own [`KEEP`](crate::artifact::KEEP), ordered by arrival, and
//! is never counted against the readable versions' three.** Both simpler
//! answers put an unreadable version in the same order as the readable ones,
//! and both fail the store's purpose:
//!
//! * **Below every readable version** makes *deleted first* the silent answer to
//!   *not understood*.
//! * **Above every readable version** pins it. The arrival outranks it, so the
//!   lowest *readable* version is what each later deploy evicts — and after
//!   enough deploys the store holds the running version, the unreadable ones,
//!   and nothing to roll back to, with every eviction reported by name and
//!   looking orderly.
//!
//! Retained apart, neither kind ever costs the other a place: readable versions
//! keep their three by version, unreadable ones age out among themselves by
//! arrival, because they carry no version order to go by.
//!
//! **The cost, stated:** a product's footprint in the store is bounded at twice
//! [`KEEP`](crate::artifact::KEEP) rather than once, and it reaches that only
//! when three or more unreadable versions of one product have been sent to one
//! machine.
//!
//! ### A genuine stray high version holds its place, and that is the cost of this order
//!
//! Ordering by version means **the highest version stays**, whether or not it
//! is the build anybody wants next. An artifact signed by nobody this machine
//! trusts can no longer be one of them — it is refused before it is named, below
//! — so what remains is a *genuine* artifact numbered above the release line: a
//! test build `0.9.9`, signed with the real key, for a product shipping `0.1.x`.
//! It keeps one of its product's three until something higher arrives, leaving
//! one rollback target rather than two. The store verifies that an artifact is
//! authentic, not that it is the one an operator meant, and the rule accepts
//! that rather than guessing.
//!
//! # What is true of everything in this store
//!
//! **Nothing enters it unverified**, ruled by the maintainer on 2026-09-12:
//! the store is to hold no unverified object, whichever way the check is
//! shared (`.rulings/` R-308). `ADR-052` §7 is the decision. The one writer
//! that names an object, `cmd/rivet-mcpd`'s bulk endpoint, checks two things
//! before the `rename(2)` that names it: the transfer's digest, which is a
//! statement about the wire, and then `crate::package::verify::artifact`
//! against this machine's trust store — the header signed by a key it holds,
//! that key entitled to the kind, every member the bytes the signature covers.
//! **That is the installer's own implementation, not a copy of it**, so
//! `ADR-029` §11's rule against a second verifier is kept rather than bent: one
//! implementation, consulted at two points, cannot disagree with itself.
//!
//! **Three limits, stated so the sentence above is not read for more than it
//! says:**
//!
//! * **A verdict is true of the trust store it was reached against.** An image
//!   update can replace that store, so an object admitted under an old one may
//!   not verify under the new — which is one reason the installer checks again.
//! * **The boundary against root on `/state` is unchanged.** Anything running
//!   as uid 0 can write under `objects/` without the endpoint, which is the
//!   other reason the installer keeps its check.
//! * **A store written by a build from before 2026-09-12 was not checked.** No
//!   sweep runs over it — see the endpoint's `receive` — and a held object is
//!   judged by the door whenever it is sent again, and removed if it fails.
//!
//! **It holds no card images.** `ADR-052` §2 intends them and nothing
//! implements them: an image is not a `.rvt`, so accepting one means a second
//! signature check and a second kind of entry. Recorded as intent rather than
//! built.
//!
//! # Picking a "newest" for a LISTING is not retention, and needs its own rule
//!
//! Retention above decides what a store may DELETE. Several callers elsewhere
//! in this tree instead LIST or PRESELECT a "newest" build without deleting
//! anything — a provisioning menu, a deploy interview, an ambiguity refusal's
//! suggestion — and none of them has [`crate::artifact::Stored`] entries to
//! hand [`crate::artifact::evicted`]; each holds only a manifest's own text
//! fields. [`crate::artifact::newest_cmp`] is that same version-then-arrival
//! rule, restated for those three fields, so a listing and a deletion cannot
//! quietly disagree about which build is newest (`.rulings/` R-213).

use crate::state::ARTIFACTS;

/// How many artifacts of one product this store keeps.
///
/// **Three**, and the module header argues it. It is a constant rather than a
/// setting because a machine that could be configured to keep one would be a
/// machine whose rollback depends on a value somebody edited.
pub const KEEP: usize = 3;

/// A store, rooted somewhere.
///
/// # The root is a parameter, and `ADR-052`'s own History says why
///
/// That document records what it cost the last time a root was resolved by the
/// code that used it: running `cargo test` once created a live `~/.rivet/fleet`
/// on the machine that ran it, and the `#[cfg(test)]` gate that stopped it
/// compiled the feature away so that three hundred tests ran with the thing
/// under test absent.
///
/// The same trap is here in a sharper form, because this store is a directory a
/// privileged daemon **deletes from**: a test that reached
/// `/state/artifacts/objects` on a developer's machine would find nothing, and
/// on a board would find the fleet's artifacts and prune them. So the root is
/// threaded in, [`Store::on_this_machine`] is the one place the real constant
/// appears, and a test hands a temporary directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    root: String,
}

impl Store {
    /// The store on the machine this code is running on.
    ///
    /// **The only place [`crate::state::ARTIFACTS`] is read**, so a caller that
    /// wants a different root cannot get this one by accident and a caller that
    /// wants this one says so.
    #[must_use]
    pub fn on_this_machine() -> Self {
        Self::at(ARTIFACTS)
    }

    /// A store rooted at `root`. For a test, and for nothing else yet.
    #[must_use]
    pub fn at(root: impl Into<String>) -> Self {
        Self {
            root: root.into(),
        }
    }

    /// Where this store lives.
    #[must_use]
    pub fn root(&self) -> &str {
        &self.root
    }

    /// The directory holding stored objects.
    #[must_use]
    pub fn objects_dir(&self) -> String {
        format!("{}/objects", self.root)
    }

    /// The directory holding transfers that have not landed.
    ///
    /// **A sibling of [`Store::objects_dir`] and not a child**, so that a
    /// listing of the objects directory is a listing of what this machine holds
    /// — with nothing half-written in it to be filtered out by a reader who
    /// remembered to.
    #[must_use]
    pub fn incoming_dir(&self) -> String {
        format!("{}/incoming", self.root)
    }

    /// The directory `digest` occupies once it has landed.
    #[must_use]
    pub fn object_dir(&self, digest: &Digest) -> String {
        format!("{}/{digest}", self.objects_dir())
    }

    /// The directory a transfer of `digest` is built in.
    #[must_use]
    pub fn incoming_for(&self, digest: &Digest) -> String {
        format!("{}/{digest}", self.incoming_dir())
    }

    /// Where the artifact itself sits, given the name it arrived under.
    ///
    /// **`name` is not checked here**, because there is exactly one producer of
    /// it — `cmd/rivet-mcpd`'s `safe_name`, which is a closed rule rather than a
    /// blocklist — and a second, weaker check in this crate would be the shape
    /// `ADR-028` §2 forbids one layer down: two answers to *is this name safe*,
    /// of which the weaker eventually gets believed. Callers that read a name
    /// back off a filesystem get it from a directory listing, which cannot
    /// contain a separator.
    #[must_use]
    pub fn object_path(&self, digest: &Digest, name: &str) -> String {
        format!("{}/{name}", self.object_dir(digest))
    }

    /// Where a transfer writes, given the name it will land under.
    #[must_use]
    pub fn incoming_path(&self, digest: &Digest, name: &str) -> String {
        format!("{}/{name}", self.incoming_for(digest))
    }
}

/// A BLAKE3 digest that has been checked, and the only way to build a path into
/// this store.
///
/// # Why a type rather than a `&str` and a rule
///
/// Because the rule is *"64 lowercase hex characters"* and the consequence of
/// forgetting it is a path built out of whatever a caller was given —
/// `/state/artifacts/objects/../../secrets`, arriving at a `rename(2)` in a
/// process running as uid 0. A function that takes a `&str` and validates it
/// internally would be the same thing with the check in a place a later caller
/// can route around; this way there is no path-producing function that can be
/// reached without the check having happened.
///
/// **Lowercase only, and it is not tidiness.** The digest is a directory name,
/// and two spellings of one digest would be two directories holding the same
/// artifact on a case-sensitive filesystem and one directory that resolves
/// under one spelling and not the other on a case-insensitive one. One
/// spelling, refused rather than normalised, so a caller that sent the other is
/// told rather than silently agreed with. Every producer in this project
/// already writes lowercase — `cmd/rivet-mcpd`'s `hex`, `rivet_remote`'s, and
/// `crate::package`'s.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Digest(String);

/// How many characters a digest has. BLAKE3 is 32 bytes.
pub const DIGEST_CHARS: usize = 64;

impl Digest {
    /// A digest, or `None` for anything that is not one.
    #[must_use]
    pub fn new(text: &str) -> Option<Self> {
        let ok = text.len() == DIGEST_CHARS
            && text.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'));
        ok.then(|| Self(text.to_string()))
    }

    /// The 64 characters.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for Digest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------------
// `ADR-027` §7's name, read back
// ---------------------------------------------------------------------------

/// The `<arch>` tokens `ADR-027` §3.1 lists.
///
/// **The values of `rivet_remote::name::arch_for`'s map, and that crate's own
/// test asserts every arm of it produces a token in here** — so a fourth target
/// added there cannot be forgotten here. The list lives in this crate because
/// this is the crate a *machine* links; `arch_for` maps Rust target triples,
/// which is a workstation's question.
pub const ARCH_TOKENS: &[&str] = &["aarch64", "x86_64", "armv6"];

/// What an artifact's file name says about it.
///
/// **The name, not the container.** `ADR-029` §11 allows exactly one verifier of
/// a `.rvt`, and this reads none of one: it is the inverse of the function that
/// *composed* the name, and what it is trusted with is bounded by what it is
/// used for — grouping objects for retention, and telling an operator what they
/// are about to install before they confirm. The authoritative product, version
/// and kind are in the signed header and the installer takes them from there,
/// exactly as it always did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name {
    /// `<product>` — the part before the version.
    pub product: String,
    /// `<version>`, including any `-g<short>` and `-dirty` (`ADR-027` §3.2).
    pub version: String,
    /// `<arch>` — one of [`ARCH_TOKENS`].
    pub arch: String,
    /// `<platform>`, which a system update carries and an application never
    /// does (`ADR-027` §9 item 3).
    pub platform: Option<String>,
    /// `<kind>` — `application`, `update` or `library`.
    pub kind: String,
}

/// Read `ADR-027` §7's grammar back out of a file name.
///
/// `None` for a name this project did not compose, which is the safe answer
/// everywhere it is used.
///
/// # How it splits a name a `-` runs right through
///
/// `rivet-todo-0.1.1-g3d28ede-aarch64-application.rvt` has a hyphen in the
/// product, two in the version, and one separating every field. So it is read
/// from the **right**, where the fields are fixed:
///
/// 1. the last token is `<kind>`;
/// 2. scanning left, the last token that is an `<arch>` ends the variable
///    part — anything between it and the kind is `<platform>`;
/// 3. what remains is `<product>-<version>`, and `<version>` is a crate
///    version, which Cargo requires to begin `MAJOR.MINOR.PATCH` — three
///    dot-separated runs of ASCII digits. The first token that **opens**
///    with that shape starts it, which is not the same as the first token
///    that merely begins with a digit: in `sensor-2-0.1.0-...` the `2`
///    begins with a digit but is not a version, so it stays in the product
///    and `0.1.0` is what starts the version.
#[must_use]
pub fn parse_name(file_name: &str) -> Option<Name> {
    let stem = file_name.strip_suffix(crate::package::kind::SUFFIX)?;
    let tokens: Vec<&str> = stem.split('-').collect();
    // `<product>-<version>-<arch>-<kind>` is the shortest form there is.
    if tokens.len() < 4 {
        return None;
    }
    let kind = (*tokens.last()?).to_string();
    let arch_at = tokens
        .iter()
        .rposition(|token| ARCH_TOKENS.contains(token))?;
    // The arch cannot be the last token — the kind is — and there has to be a
    // product and a version in front of it.
    if arch_at + 1 >= tokens.len() || arch_at < 2 {
        return None;
    }
    let version_at = version_token_at(&tokens[..arch_at])?;
    let platform = &tokens[arch_at + 1..tokens.len() - 1];
    Some(Name {
        product: tokens[..version_at].join("-"),
        version: tokens[version_at..arch_at].join("-"),
        arch: tokens[arch_at].to_string(),
        platform: (!platform.is_empty()).then(|| platform.join("-")),
        kind,
    })
}

/// Where a crate version starts, among `-`-separated `tokens`: the index of
/// the first one that [`opens_a_crate_version`], never token zero — a match
/// there would leave nothing in front of it to be a product, so `None` is the
/// answer for that case exactly as for no match at all.
///
/// **The one place this position is decided, for both readers of `ADR-027`
/// §7's rule.** [`parse_name`] calls this over a payload file name's tokens
/// and `crate::package::library::Reference::product_and_version` calls it
/// over a library reference's — two names with two different tails glued on
/// after the version, but one question in front of it, so it is answered
/// once. `ADR-011` is why: a payload and a library reference that disagreed
/// about where a product ends would be retention grouping one of them
/// wrongly.
pub(crate) fn version_token_at(tokens: &[&str]) -> Option<usize> {
    let at = tokens.iter().position(|token| opens_a_crate_version(token))?;
    (at != 0).then_some(at)
}

/// Whether `token` opens with a crate version: three dot-separated runs of
/// ASCII digits, such as `0.1.0` or `1.10.2`.
///
/// **Only the third run needs to start with a digit, not consist of one.**
/// `cargo rivet package` always composes a token that is exactly its crate
/// version, but `cargo rivet update` takes a version as typed, and
/// `0.1.84rc1` on a system update is a realistic one — so `84rc1` still
/// counts, which is what "opens with" rather than "is" means here.
fn opens_a_crate_version(token: &str) -> bool {
    fn is_digit_run(s: &str) -> bool {
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
    }
    let mut parts = token.splitn(3, '.');
    let (Some(major), Some(minor), Some(patch)) = (parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let patch_opens_with_a_digit = patch.as_bytes().first().is_some_and(u8::is_ascii_digit);
    is_digit_run(major) && is_digit_run(minor) && patch_opens_with_a_digit
}

/// The product a file name belongs to, for grouping.
///
/// **Falls back to the whole name**, which the module header argues: an
/// unparseable name forms a group of one and is never counted against a real
/// product's three.
#[must_use]
pub fn product_of(file_name: &str) -> String {
    parse_name(file_name).map_or_else(|| file_name.to_string(), |name| name.product)
}

// ---------------------------------------------------------------------------
// Ordering two versions of one product
// ---------------------------------------------------------------------------

/// Where one artifact's version sits relative to another's.
///
/// # Why this is a type and not a `Vec<u64>`
///
/// Because *"this version cannot be ordered"* is an answer retention has to act
/// on, and an answer encoded as an empty vector or a zero would be the same
/// answer as `0.0.0` — the lowest thing in the group, and therefore the first
/// thing deleted. The module header argues the direction; the type is what stops
/// the other direction being reached by accident.
#[derive(Debug, Clone)]
pub enum VersionOrder {
    /// Dot-separated numbers, compared **componentwise** rather than as text.
    ///
    /// Held as read and zero-extended at comparison time, so `0.3` and `0.3.0`
    /// are one version rather than two.
    Numbered(Vec<u64>),
    /// A version whose core this build could not read as numbers.
    ///
    /// **Retention never compares it with a [`VersionOrder::Numbered`]** — the
    /// two are retained apart, which the module header argues. It is ordered
    /// above them here only so that the type has a total order, and a caller
    /// that finds itself relying on that direction has put the two kinds back in
    /// one order and should read the header first.
    Unreadable,
}

/// **Equality is [`Ord`]'s, not the representation's.** `0.3` and `0.3.0` are
/// held as vectors of different lengths and are one version, so a derived
/// `PartialEq` would call them unequal while `cmp` called them equal — which is
/// the inconsistency `Ord`'s own contract forbids and which a sort is entitled
/// to rely on.
impl PartialEq for VersionOrder {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == core::cmp::Ordering::Equal
    }
}

impl Eq for VersionOrder {}

impl Ord for VersionOrder {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        use core::cmp::Ordering;
        match (self, other) {
            (Self::Unreadable, Self::Unreadable) => Ordering::Equal,
            (Self::Unreadable, Self::Numbered(_)) => Ordering::Greater,
            (Self::Numbered(_), Self::Unreadable) => Ordering::Less,
            (Self::Numbered(ours), Self::Numbered(theirs)) => {
                // Zero-extended rather than lexicographic: `Vec`'s own `Ord`
                // would put `0.3` below `0.3.0`, which are the same version.
                let width = ours.len().max(theirs.len());
                for at in 0..width {
                    let ours = ours.get(at).copied().unwrap_or(0);
                    let theirs = theirs.get(at).copied().unwrap_or(0);
                    match ours.cmp(&theirs) {
                        Ordering::Equal => {}
                        decided => return decided,
                    }
                }
                Ordering::Equal
            }
        }
    }
}

impl PartialOrd for VersionOrder {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Order an `ADR-027` §3 version field.
///
/// **Everything from the first `-` onward is discarded**, because that is where
/// the build metadata begins — `-g<short>`, `-dirty`, and the payload digest a
/// dirty build carries — and none of it carries an order. The module header
/// states the case against inventing one.
///
/// What remains is dot-separated numbers. Anything else — a component that is
/// not a number, one that overflows a `u64`, an empty string — is
/// [`VersionOrder::Unreadable`], said here rather than left to arise from a
/// parse that happened to fail.
#[must_use]
pub fn version_order(version: &str) -> VersionOrder {
    let core = version.split('-').next().unwrap_or(version);
    let mut parts = Vec::new();
    for component in core.split('.') {
        match component.parse::<u64>() {
            Ok(number) => parts.push(number),
            Err(_) => return VersionOrder::Unreadable,
        }
    }
    if parts.is_empty() {
        return VersionOrder::Unreadable;
    }
    VersionOrder::Numbered(parts)
}

/// Order the version an `ADR-027` §7 file name carries.
///
/// A name this project did not compose has no version to read, so it is
/// [`VersionOrder::Unreadable`] — which costs it nothing, because [`product_of`]
/// has already made it a group of one.
#[must_use]
pub fn version_order_of(file_name: &str) -> VersionOrder {
    parse_name(file_name).map_or(VersionOrder::Unreadable, |name| version_order(&name.version))
}

// ---------------------------------------------------------------------------
// Retention
// ---------------------------------------------------------------------------

/// What retention needs to know about a thing it may drop.
///
/// # Why a trait and not a second copy of the loop
///
/// [`evicted`] is the arithmetic `ADR-052` §7 argues over eight paragraphs
/// — group by product, hold the unreadable versions apart, order by version and
/// never by arrival, keep the object that just landed whatever its timestamp
/// says. **`SPEC-02` §12 needed exactly that arithmetic for a second store**,
/// the library directories under `<volume>/lib` (`.rulings/` R-202), and the
/// two differ only in how a name is split and what the last tie-break is.
///
/// A second spelling of a rule with four special cases in it is a second place
/// for one of them to be forgotten, and the one that would be forgotten is the
/// one that took a hardware run to find: `arriving`. So the rule is written
/// once and each store says what its entries *are*.
pub trait Retained {
    /// Which product's group this entry is retained within.
    ///
    /// **Never `None` and never a shared bucket for the unreadable**: a name
    /// this build cannot split forms a group of one, so it is never counted
    /// against a real product's quota and never evicted by one.
    fn product(&self) -> String;

    /// Where this entry sits among its product's versions.
    fn version_order(&self) -> VersionOrder;

    /// When it arrived, as any ordinal that increases. **A tie-break and not
    /// the rule** — see the module header on `ADR-021`, and never compared
    /// across machines.
    fn added(&self) -> u64;

    /// A name unique within one store, and the last tie-break.
    ///
    /// It exists so that the order is **total**: without it two entries equal
    /// on product, version and arrival would be ordered by however the
    /// filesystem listed them, and the same store would answer differently on
    /// two runs. It is also what `arriving` is matched against.
    fn identity(&self) -> &str;
}

/// So that a caller which has already **filtered** its store can still ask.
///
/// `SPEC-02` §12's guard is a filter, and a filter over a slice produces
/// references rather than values. Without this a caller would have to clone
/// every surviving entry to ask a question about which of them to drop.
impl<T: Retained + ?Sized> Retained for &T {
    fn product(&self) -> String {
        (**self).product()
    }

    fn version_order(&self) -> VersionOrder {
        (**self).version_order()
    }

    fn added(&self) -> u64 {
        (**self).added()
    }

    fn identity(&self) -> &str {
        (**self).identity()
    }
}

/// One object, as a caller that listed the store observed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    /// The directory's name, checked.
    pub digest: Digest,
    /// The one file inside it.
    pub name: String,
    /// When it arrived, as any ordinal that increases. On a machine this is the
    /// object directory's modification time in seconds; in a test it is
    /// whatever the test says. **A tie-break and not the rule** — see the module
    /// header on `ADR-021`, and never compared across machines.
    pub added: u64,
}

impl Stored {
    /// Which product's three this object is one of.
    #[must_use]
    pub fn product(&self) -> String {
        product_of(&self.name)
    }

    /// Where this object sits among its product's versions.
    #[must_use]
    pub fn version_order(&self) -> VersionOrder {
        version_order_of(&self.name)
    }
}

impl Retained for Stored {
    // The free functions rather than the inherent methods of the same names:
    // an inherent `product` and a trait `product` in one scope is a path a
    // later reader has to resolve, and one resolution of it is infinite
    // recursion.
    fn product(&self) -> String {
        product_of(&self.name)
    }

    fn version_order(&self) -> VersionOrder {
        version_order_of(&self.name)
    }

    fn added(&self) -> u64 {
        self.added
    }

    /// **The digest, and not the `ADR-027` name.** A store keyed by digest can
    /// hold two objects whose names agree — a dirty build sent twice is the
    /// ordinary way — so the name is not unique and would leave the order
    /// partial exactly where the tie-break is needed.
    fn identity(&self) -> &str {
        self.digest.as_str()
    }
}

/// The objects a store should drop, given everything it holds.
///
/// **It returns what to REMOVE rather than what to keep**, because the caller's
/// next act is a sequence of removals and a function that answered the other way
/// would make every caller compute the complement — three times, in three
/// crates, with one of them eventually getting it inverted.
///
/// Per product, readable versions keep `keep` and unreadable versions keep
/// their own `keep`, each newest first by [`Stored::version_order`], ties broken
/// by [`Stored::added`] and then by digest — deterministic, so two runs over the
/// same store produce the same answer and a test can assert one.
///
/// # Why version and not arrival
///
/// **Arrival cannot separate a burst.** A board's clock has one-second
/// granularity (`ADR-021`), and several deploys over a LAN land inside one
/// second, so every artifact in the burst carries the same `added`. Ordering by
/// arrival then means ordering by the digest beneath it — a content hash with
/// no relation to a version — and that keeps an older version while deleting a
/// newer one about as often as not.
///
/// **Why that is worse than untidy.** The store exists so a rollback is local.
/// An operator who deploys twice quickly and then wants the version before last
/// could find exactly that one gone with an older one kept, and because each
/// eviction is reported by name, nothing would look wrong.
///
/// # `arriving` is never dropped, and that is a correctness rule
///
/// **Found by a test on 2026-09-12, before any of this ran on a machine.** Four
/// artifacts of one product created within the same second have the same
/// `added` — `ADR-021` again, and on ext4 it is merely likely rather than
/// certain — so the tie-break by digest decided which fell off, and one time in
/// four that is *the one that has just been transferred*. A deploy would then
/// upload 700 MB, delete it, and ask the machine to install a digest it no
/// longer held.
///
/// A finer clock would make that rarer and would not make it impossible, so the
/// fix is not a clock. The caller names the object it has just stored, and that
/// object is ordered ahead of everything else in its product's group: it **is**
/// the newest, whatever the filesystem's timestamps say, because the caller
/// watched it arrive.
///
/// `None` is for a prune that is not an arrival — a sweep, a caller that is
/// tidying — and there is no such caller yet.
///
/// `keep == 0` drops everything, `arriving` included, which is a caller's
/// decision to make and not one this function second-guesses; [`KEEP`] is what
/// every real caller passes.
#[must_use]
pub fn evictions<'a>(
    stored: &'a [Stored],
    keep: usize,
    arriving: Option<&Digest>,
) -> Vec<&'a Stored> {
    evicted(stored, keep, arriving.map(Digest::as_str))
}

/// [`evictions`] for any store whose entries say what they are.
///
/// The rule is [`evictions`]'s in full, and that documentation is the argument
/// for every line of this one; what is written here is only what a *second*
/// store had to know. `SPEC-02` §12 is that store — the library directories
/// under `<volume>/lib`, whose entries are
/// [`crate::package::install::HeldLibrary`].
///
/// # What this deliberately does not take
///
/// **There is no "protected" parameter**, although `SPEC-02` §12's guard needs
/// one: a library named in some installed application's declared closure is
/// never evicted, whatever version it is. That guard is a *filter on the
/// input*, applied by the caller before it asks — see
/// [`crate::package::install::library_evictions`] — rather than a fifth case
/// inside the comparator.
///
/// The difference is not stylistic. A protected entry held back *inside* this
/// function would still occupy a place in its product's group, so protecting an
/// old library would evict a newer one to pay for it. Taking it off the table
/// first means a declared library costs its product's quota nothing, which is
/// what the guard is for.
#[must_use]
pub fn evicted<'a, T: Retained>(held: &'a [T], keep: usize, arriving: Option<&str>) -> Vec<&'a T> {
    let mut products: Vec<String> = held.iter().map(Retained::product).collect();
    products.sort();
    products.dedup();

    let mut dropped: Vec<&T> = Vec::new();
    for product in products {
        // The key is computed once per entry rather than inside the comparator,
        // which would re-parse a name `n log n` times.
        let entries: Vec<(bool, VersionOrder, &T)> = held
            .iter()
            .filter(|entry| entry.product() == product)
            .map(|entry| {
                (
                    arriving == Some(entry.identity()),
                    entry.version_order(),
                    entry,
                )
            })
            .collect();
        // Readable and unreadable versions are retained APART, each keeping
        // `keep` — the module header argues it. Within the unreadable group
        // every version compares equal, so the one comparator below orders it
        // by arrival and needs no second spelling.
        let (mut readable, mut unreadable): (Vec<_>, Vec<_>) = entries
            .into_iter()
            .partition(|(_, version, _)| !matches!(version, VersionOrder::Unreadable));
        for group in [&mut readable, &mut unreadable] {
            // Newest first: the arrival, then version descending, then `added`
            // descending, then identity descending — so the order is total, does
            // not depend on how the filesystem listed, and cannot put the object
            // that just landed at the end of it.
            group.sort_by(|(a_arrival, a_version, a), (b_arrival, b_version, b)| {
                b_arrival
                    .cmp(a_arrival)
                    .then_with(|| b_version.cmp(a_version))
                    .then_with(|| b.added().cmp(&a.added()))
                    .then_with(|| b.identity().cmp(a.identity()))
            });
            dropped.extend(group.drain(..).skip(keep).map(|(_, _, entry)| entry));
        }
    }
    // One order out, whatever order came in.
    dropped.sort_by(|a, b| a.identity().cmp(b.identity()));
    dropped
}

// ---------------------------------------------------------------------------
// Comparing two builds of one product for "which is newest" — for a LISTING
// or a PRESELECTION, never a deletion (that is retention, above)
// ---------------------------------------------------------------------------

/// What [`newest_cmp`] needs from one build, borrowed rather than owned.
///
/// Every caller already holds these three facts on its own manifest-entry
/// type — `Entry`'s shape differs across `cmd/cargo-rivet`, `cmd/rivet-image`
/// and `cmd/rivet-provision` — so this takes the facts a comparison needs
/// rather than the type that happens to carry them.
#[derive(Debug, Clone, Copy)]
pub struct BuildKey<'a> {
    /// The `ADR-027` §3 version field — what [`version_order`] reads.
    pub version: &'a str,
    /// `ADR-052` §5's ISO-8601 stamp, so a plain `str` compare is
    /// chronological. **A tie-break, never the rule** — [`Stored::added`]'s
    /// own posture, restated here because a manifest's `added` is text this
    /// workstation wrote, not that type's ordinal.
    pub added: &'a str,
    /// The final, deterministic tie-break — a digest in every caller.
    pub identity: &'a str,
}

/// Whether `a` or `b` is the newer build of ONE product: `Ordering::Greater`
/// when `a` is newer, reading exactly like comparing two plain numbers where
/// bigger means newer. So `iter().max_by(|a, b| newest_cmp(key(a), key(b)))`
/// picks the single newest, and a newest-first LISTING reverses it —
/// `slice::sort_by(|a, b| newest_cmp(key(a), key(b)).reverse())` — because
/// `sort_by` is ascending and "newest first" is descending by recency.
/// **Deliberately the opposite convention from [`evicted`]'s own closure**,
/// which compares `b` against `a` so it can sort ascending directly; a
/// comparator meant to be reached through [`Iterator::max_by`] earns its
/// keep by reading the ordinary way instead: paired with `max_by`, the
/// reverse convention silently picks the OLDEST build, which is the defect
/// this comparator exists to prevent.
///
/// Ordered by [`version_order`] first; **on a tie, the build added to its
/// repository more recently** wins, and only then `identity` — so two builds
/// sharing both a version and a one-second-granularity `added` stamp still
/// resolve to one deterministic answer rather than whichever a caller's
/// iterator happened to list first.
///
/// # Why the tie-break exists at all
///
/// `.rulings/` R-213: several builds commonly share one version's numeric
/// core, differing only in the build metadata [`version_order`] discards
/// (`-g<hash>`, `-dirty`) — a workspace version bumped once and built several
/// times over. Ordering by [`version_order`] alone leaves every one of them
/// tied, and a stable sort then keeps whichever a manifest lists first —
/// which is the OLDEST of them on every manifest reached this way, because
/// each is appended to on publish and never reordered. **This is retention's
/// own [`evicted`] rule, restated for a caller with no [`Retained`] entries to
/// hand it** — only the three plain fields [`BuildKey`] asks for.
#[must_use]
pub fn newest_cmp(a: BuildKey<'_>, b: BuildKey<'_>) -> core::cmp::Ordering {
    version_order(a.version)
        .cmp(&version_order(b.version))
        .then_with(|| a.added.cmp(b.added))
        .then_with(|| a.identity.cmp(b.identity))
}

#[cfg(test)]
#[path = "artifact_tests.rs"]
mod tests;
