//! `ADR-027`'s grammar, composed by the tool rather than typed by the caller.
//!
//! `ADR-027` §7 rules that the tool composes the name, because the caller is
//! *"the authority for three facts the caller does not own"*, and §7 records
//! that `cmd/rivet-image` does not yet do so. **This tool does**, from the
//! first artifact it writes, and there is no override — an override is a
//! supported way to make the name disagree with the contents.
//!
//! ```text
//! <product>-<version>-<arch>-<kind>.rvt
//! hello-0.1.0-g7b84d4b-dirty-aarch64-application.rvt
//! ```
//!
//! **No `<platform>` component.** The maintainer's 2026-08-24 ruling, written into
//! `ADR-027` §9 item 3: an application is architecture-shaped rather than
//! firmware-shaped. §3's *"no field ever empty or omitted"* is not violated by
//! this — the field is not empty, it is not part of an application's grammar at
//! all, which is the distinction §3 was drawing when it said a field with
//! nothing to say means the grammar is wrong.

use std::path::Path;
use std::process::Command;

use rivet_core::package::kind::{Kind, SUFFIX};
use rivet_core::package::port::Digest;
use rivet_crypto::Blake3;

/// `<arch>` for a target triple. `ADR-027` §3.1: *"will this binary execute"*.
///
/// **A table, and unknown triples are refused.** `aarch64-unknown-linux-musl`
/// and `arm-unknown-linux-musleabihf` are both "a Raspberry Pi" and their first
/// components are not the tokens `ADR-027` §3.1 lists — `arm` is not `armv6`.
/// Splitting the triple at the first `-` would put the wrong token in a name
/// for the one target the ADR names the Pi Zero W by, so it is not done
/// (`AGENTS.md` §5 — refuse rather than guess).
pub fn arch_for(triple: &str) -> Option<&'static str> {
    match triple {
        "aarch64-unknown-linux-musl" => Some("aarch64"),
        "x86_64-unknown-linux-musl" => Some("x86_64"),
        // `ADR-002` §2's deferred compatibility target. Named here because the
        // mapping is the non-obvious one, not because anything has built for it.
        "arm-unknown-linux-musleabihf" => Some("armv6"),
        _ => None,
    }
}

/// The version field: the crate's version, plus what `ADR-027` §3.2 and §3.2.1
/// require of a build that is not from a clean tag.
///
/// > **The absence of a SHA is therefore a claim**: that this artifact came
/// > from a tag.
///
/// and
///
/// > `g1319389` asserts *these bytes came from commit 1319389*, and if
/// > uncommitted changes were compiled into them the assertion is false.
///
/// So: not a git work tree at all → the bare version, because there is no claim
/// to make. On a tag with a clean tree → the bare version, which is §3.2's
/// claim and is true. Anything else → `-g<short>`, and `-dirty` after it when
/// `git status --porcelain` is non-empty.
///
/// **`git` is asked rather than a library**, and it is asked *in the crate's own
/// directory*. `AGENTS.md` §10.5 records that WSL's `git` and Windows' `git`
/// disagree about whether this tree is dirty and that **Windows is right** — so
/// which `git` is on the PATH decides the answer, and a build run under WSL
/// would stamp `-dirty` on every artifact for ever. The tool cannot detect that
/// for the caller; [`Version::how`] reports what it asked and what it was told,
/// so the caller can see it rather than discover it in a filename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    text: String,
    how: String,
    /// Whether `-dirty` is in [`Self::text`], so [`Self::distinguished_by`] does
    /// not have to read it back out of a string it composed. A parse of one's
    /// own output is a second answer to a fact one already had.
    dirty: bool,
}

/// How many hex characters of the payload's digest a `-dirty` version carries.
///
/// **Eight, and it is a budget rather than a preference.** `ADR-029` §3.1 makes
/// the header's `version` field 32 bytes and refuses a longer one rather than
/// truncating, so the whole of `<crate>-g<7>-dirty-<n>` has to fit: a three-part
/// SemVer of up to seven characters leaves exactly this much. Four bytes of
/// BLAKE3 is not a collision-resistance claim and is not made to be one — what
/// it has to do is tell two builds of one working tree apart on one operator's
/// bench, which is `ADR-027` §3.2.2's whole scope.
pub const DIRTY_DIGEST_CHARS: usize = 8;

/// Where one payload member's bytes come from, for [`Version::distinguished_by`].
///
/// The two arms are the two shapes a packager has: the manifest is **generated**
/// and is already in memory, and a program or an asset is a **file** that may be
/// megabytes and is streamed. `cmd/cargo-rivet`'s `emit::Source` is the same
/// split for the same reason; this is not that type, because the writer lives in
/// the tool and the naming lives here.
#[derive(Debug, Clone, Copy)]
pub enum Content<'a> {
    /// Bytes the packager composed.
    Bytes(&'a [u8]),
    /// A file on disk, read once and not held.
    File(&'a Path),
}

/// One member of the payload a version is going to name.
///
/// `name`, `class` and `variant` are the member's **identity** in the container
/// (`ADR-029` §13.2 item 4) and all three go into the digest: two payloads that
/// differ only in which class a byte-identical member was filed under are two
/// different payloads, and a version that could not tell them apart would be
/// this section's defect wearing a third face.
#[derive(Debug, Clone, Copy)]
pub struct Member<'a> {
    pub name: &'a str,
    pub class: u8,
    pub variant: u8,
    pub content: Content<'a>,
}

impl Version {
    pub fn compose(crate_version: &str, at: &Path) -> Self {
        let dirty = git(at, &["status", "--porcelain"]).is_some_and(|out| !out.is_empty());
        Self::compose_with(crate_version, at, dirty, "git status --porcelain, in the container")
    }

    /// Like [`Self::compose`], with the DIRTY determination supplied by the
    /// caller instead of asked of `git status --porcelain` in `at`'s directory.
    ///
    /// **The escape hatch `cargo rivet package` did not have.** `AGENTS.md`
    /// §10.5: `git status --porcelain` (and `git describe --dirty`) disagree
    /// between Windows' git and one run in this project's WSL or build
    /// container — a checkout that is genuinely clean reads as modified inside
    /// the container because of line endings, so a payload built there carries
    /// `-dirty` on every build for ever, asserting uncommitted changes that do
    /// not exist. `tools/card/bump.sh` already escapes this for the system
    /// image by taking the WHOLE describe string from Windows git; this is the
    /// same escape for the one fact that actually disagrees, because
    /// `git rev-parse` and `git describe --exact-match` do not — only the
    /// working-tree check does, so only it is worth letting the caller supply.
    ///
    /// **This does not change what a clean or a dirty version looks like** —
    /// `ADR-027` §3.2's grammar is `compose`'s untouched, and this only changes
    /// where the boolean it branches on comes from. A caller with nothing to
    /// override keeps calling [`Self::compose`], which keeps asking the git it
    /// runs with — wrong in the safe direction (it can call a clean tree dirty,
    /// never a dirty one clean), and this does not touch that default.
    pub fn compose_with_dirty(crate_version: &str, at: &Path, dirty: bool) -> Self {
        Self::compose_with(crate_version, at, dirty, "supplied by the caller, not asked of git")
    }

    fn compose_with(crate_version: &str, at: &Path, dirty: bool, dirty_source: &str) -> Self {
        let Some(head) = git(at, &["rev-parse", "--short=7", "HEAD"]) else {
            return Self {
                text: crate_version.to_string(),
                how: "not a git work tree, so the version makes no claim about a commit".into(),
                dirty: false,
            };
        };
        let tagged = git(at, &["describe", "--exact-match", "--tags", "HEAD"]);

        match (&tagged, dirty) {
            (Some(tag), false) => Self {
                text: crate_version.to_string(),
                how: format!("HEAD is tag {tag} and the tree is clean, so no SHA is carried"),
                dirty: false,
            },
            _ => {
                let mut text = format!("{crate_version}-g{head}");
                let mut how = format!("built from {head}");
                if dirty {
                    text.push_str("-dirty");
                    how.push_str(&format!(
                        ", with uncommitted tracked changes (ADR-027 §3.2.1; dirty read from {dirty_source})"
                    ));
                }
                if tagged.is_some() && dirty {
                    how.push_str("; HEAD is tagged but the tree is not clean");
                }
                Self { text, how, dirty }
            }
        }
    }

    /// **Make a `-dirty` version unique, by a digest of the payload it names.**
    /// `ADR-027` §3.2.2, ruled 2026-08-29 and **amended 2026-09-04**.
    ///
    /// §3.2.1 made `-dirty` a claim — *there were uncommitted changes* — and
    /// stopped there, so every build of a dirty tree at one commit produced the
    /// **same version and the same artifact name**. Two artifacts built from two
    /// different working trees were then indistinguishable by anything a machine
    /// can read: same file name, same header `version`, same
    /// `/state/app/<name>/<version>` directory. A redeploy of changed code looked
    /// exactly like a redeploy of identical bytes.
    ///
    /// ~~So a dirty version carries [`DIRTY_DIGEST_CHARS`] hex characters of
    /// **BLAKE3 of the program being packaged**.~~ **Superseded 2026-09-04: of
    /// the WHOLE PAYLOAD.** The first fix hashed the program alone, and the
    /// manifest is the half the machine acts on — so editing only the manifest,
    /// rebuilding and deploying produced the same version, and `ADR-015` §7.2's
    /// installer refused the install as already present. The change silently did
    /// not happen. Assets have the same shape and `.rulings/` R-142 makes the
    /// asset set *whatever is in a directory*, which widens it rather than
    /// narrowing it.
    ///
    /// The algorithm is unchanged — BLAKE3 through `rivet-core`'s `Digest` port
    /// and `rivet-crypto`'s implementation, `ADR-029` §7.2, never a second one.
    ///
    /// ```text
    /// hello-0.1.0-g7b84d4b-dirty-1a2b3c4d-aarch64-application.rvt
    /// ```
    ///
    /// # The inputs are ordered, and that is half the fix
    ///
    /// A digest over a set has to fix the order or it is not a function of the
    /// set — **two identical payloads would then produce two versions**, which is
    /// the same defect pointing the other way and is worse for being
    /// intermittent. `ADR-027` §3.2.2 as amended therefore states the framing
    /// rather than leaving it to the caller:
    ///
    /// * members are sorted by `(name, class, variant)`, which is a member's
    ///   whole identity in the container, so the caller's order cannot reach the
    ///   answer — and `cmd/cargo-rivet` reads its assets out of a **directory**
    ///   (`.rulings/` R-142), where the order is the filesystem's;
    /// * each member contributes `name`, a `NUL`, `class`, `variant` and then the
    ///   32-byte BLAKE3 of its own content. A member name is printable ASCII
    ///   (`rivet_core::package::format::read_name`), so it cannot contain the
    ///   `NUL` and the framing is unambiguous — two members cannot be run
    ///   together into a third spelling.
    ///
    /// **A clean version is returned untouched, and that is the point.** §3.2's
    /// absence of a SHA is a claim about a tag and §3.2's presence of one is a
    /// claim about a commit; both are already unique statements about bytes that
    /// are in the history. Only `-dirty` names bytes nothing else records, so
    /// only `-dirty` needs the digest. [`Self::how`] says which happened.
    ///
    /// # Errors
    ///
    /// If any member's file cannot be read. It is **not** hashed optimistically
    /// and it is not skipped: a dirty version with no digest is the exact string
    /// this section exists to stop being emitted, so failing to read the bytes
    /// has to stop the build rather than fall back to it (`AGENTS.md` §5).
    pub fn distinguished_by(mut self, payload: &[Member<'_>]) -> Result<Self, String> {
        if !self.dirty {
            return Ok(self);
        }
        let digest = payload_digest(payload)?;
        let short: String = digest
            .iter()
            .flat_map(|b| [b >> 4, b & 0x0f])
            .take(DIRTY_DIGEST_CHARS)
            .map(|nibble| char::from_digit(u32::from(nibble), 16).unwrap_or('0'))
            .collect();
        self.text.push('-');
        self.text.push_str(&short);
        let mut named: Vec<&str> = payload.iter().map(|m| m.name).collect();
        named.sort_unstable();
        self.how.push_str(&format!(
            "; the version carries {short}, the first {DIRTY_DIGEST_CHARS} hex characters of \
             BLAKE3 of the whole payload ({}), so that two builds of two different dirty trees \
             cannot share one name (ADR-027 §3.2.2, amended 2026-09-04 to cover the manifest and \
             the assets and not the program alone)",
            named.join(", ")
        ));
        Ok(self)
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Whether this version asserts uncommitted changes — `ADR-027` §3.2.1.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// What was asked and what came back — printed, so the claim in the name is
    /// visible rather than inferred.
    pub fn how(&self) -> &str {
        &self.how
    }
}

/// **BLAKE3 of a whole payload** — `ADR-027` §3.2.2 as amended 2026-09-04.
///
/// Two levels rather than one long stream, and the reason is the framing: a
/// member's content has no length known before it is read, so a single stream
/// would either need the length up front — which is a second read and a race
/// against it — or leave two members runnable together into a third spelling.
/// Digesting each member's content on its own gives a **fixed-size** 32 bytes to
/// put in the outer hash, and the outer hash then carries the identity beside
/// it.
///
/// The order is this function's, never the caller's. See
/// [`Version::distinguished_by`] for why that is half of the fix.
fn payload_digest(payload: &[Member<'_>]) -> Result<[u8; 32], String> {
    let mut ordered: Vec<&Member<'_>> = payload.iter().collect();
    ordered.sort_by(|a, b| {
        (a.name, a.class, a.variant).cmp(&(b.name, b.class, b.variant))
    });

    let mut outer = Blake3::new();
    outer.start();
    for member in ordered {
        // `name` then a NUL: a member name is printable ASCII by
        // `rivet_core::package::format::read_name`, so this separator cannot
        // occur inside one and the encoding is prefix-free.
        outer.write(member.name.as_bytes());
        outer.write(&[0]);
        outer.write(&[member.class, member.variant]);
        let inner = match member.content {
            Content::Bytes(bytes) => Blake3::new().of(bytes),
            Content::File(path) => crate::deploy::blake3_of(path)?.0,
        };
        outer.write(&inner);
    }
    Ok(outer.finish())
}

fn git(at: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(at)
        .args(args)
        // stdin is not inherited from a terminal here on purpose: `git` must
        // never be able to prompt for anything during a build.
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `<product>-<version>-<arch>[-<platform>]-<kind>.rvt`.
///
/// **One function for both grammars, and the `Option` is the ruling.**
/// `ADR-027` §3 as amended 2026-08-24: *"a field either says something or it
/// does not exist for that kind."* An application passes `None` and an update
/// passes the `<platform>` token its `BootSlots` implementor answered
/// (`ADR-027` §3.1 — *"`<platform>` names the boot contract"*), so the two
/// grammars cannot drift apart into two spellings of the same `-`.
pub fn artifact(
    product: &str,
    version: &str,
    arch: &str,
    platform: Option<&str>,
    kind: &Kind,
) -> String {
    match platform {
        Some(platform) => format!("{product}-{version}-{arch}-{platform}-{kind}{SUFFIX}"),
        None => format!("{product}-{version}-{arch}-{kind}{SUFFIX}"),
    }
}

/// Every `<arch>` token [`artifact`] can compose, for reading one back.
///
/// **The values of [`arch_for`]'s map, not a second list.** A test asserts that
/// every arm of that function produces a token in here, so a fourth target
/// cannot be added there and forgotten here.
///
/// **It moved into `rivet-core` on 2026-09-12** and is aliased rather than
/// copied. A machine now reads `ADR-027` names too — `rivet_core::artifact`
/// groups its store by product — and the list that says which tokens are
/// architectures had to be somewhere both a workstation and a board link.
/// `arch_for` stays here, because mapping a Rust target triple is a question
/// only a machine that compiles things has.
#[cfg(test)]
const ARCH_TOKENS: &[&str] = rivet_core::artifact::ARCH_TOKENS;

/// The product and the kind of an artifact, read back out of its file name.
///
/// # Why reading a file name is allowed here and nowhere else
///
/// A `.rvt`'s authoritative product and kind are in its **signed header**, and
/// `ADR-029` §11 is emphatic that there is one verifier: a second reader of the
/// container format is exactly what that section refuses. This does not add one.
/// It is the **inverse of [`artifact`]**, which is a function this crate owns —
/// `ADR-027` §7 gives the tool the name — so this reads back only what this tool
/// wrote, and a round-trip test holds the two together.
///
/// **What it is used for bounds what it may be trusted with.**
/// `cargo rivet deploy --enable` needs a service name *before* the payload is
/// sent, so that the document says what should run before the machine has
/// anything to run. The worst a wrong answer can do is put a `[services.<name>]`
/// table in the document for a service that never appears — visible, inert, and
/// removable. Nothing about what is *installed* is decided here: the board
/// verifies the header and takes the name from it, as it always did.
///
/// `None` for a name this tool did not compose, which is the safe answer:
/// `--enable` refuses rather than guessing.
///
/// # How it splits a name a `-` runs right through
///
/// `rivet-todo-0.1.1-g3d28ede-aarch64-application.rvt` has a hyphen in the
/// product, two in the version, and one separating every field. So it is read
/// from the **right**, where the fields are fixed:
///
/// 1. the last token is `<kind>`;
/// 2. scanning left, the first token that is an `<arch>` ends the variable part
///    — anything between it and the kind is `ADR-027` §3's optional
///    `<platform>`, which an application never has;
/// 3. what remains is `<product>-<version>`, and `<version>` is a crate
///    version, which Cargo requires to begin `MAJOR.MINOR.PATCH` — three
///    dot-separated runs of ASCII digits. The first token that opens with
///    that shape starts it: in `sensor-2-0.1.0-...`, `2` begins with a digit
///    but is not shaped like a version, so it stays in the product.
///
/// # One reader, and it is not this one any more
///
/// **The grammar moved into `rivet_core::artifact::parse_name` on 2026-09-12**
/// and this delegates. A machine now reads these names as well — the artifact
/// store groups its objects by product to retain three of each — and two
/// readers of one grammar is `ADR-011` §3's *"two accounts of one fact"* with a
/// pruning decision on the end of it: a workstation and a board disagreeing
/// about where a product's name stops would be a board deleting the wrong
/// artifact.
#[must_use]
pub fn product_and_kind(file_name: &str) -> Option<(String, String)> {
    let name = rivet_core::artifact::parse_name(file_name)?;
    Some((name.product, name.kind))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn the_name_has_no_platform_component() {
        let name = artifact("hello", "0.1.0", "aarch64", None, &Kind::application());
        assert_eq!(name, "hello-0.1.0-aarch64-application.rvt");
        // The ruling is an absence, so it is asserted rather than assumed.
        assert_eq!(name.matches('-').count(), 3);
    }

    /// The other half of the same ruling, and it points the other way: an
    /// update **does** carry `<platform>`, because a system image is
    /// firmware-shaped (`ADR-027` §3.1, `ADR-029` §8's own example).
    #[test]
    fn an_updates_name_does_carry_a_platform_component() {
        let name = artifact("rivet", "0.2.0", "aarch64", Some("rpi4"), &Kind::update());
        // ADR-029 §8's worked example, byte for byte.
        assert_eq!(name, "rivet-0.2.0-aarch64-rpi4-update.rvt");
        assert_eq!(name.matches('-').count(), 4);
    }

    #[test]
    fn an_unknown_triple_is_refused_rather_than_split() {
        assert_eq!(arch_for("aarch64-unknown-linux-musl"), Some("aarch64"));
        assert_eq!(arch_for("arm-unknown-linux-musleabihf"), Some("armv6"));
        assert_eq!(arch_for("aarch64-unknown-linux-gnu"), None);
        assert_eq!(arch_for("riscv64gc-unknown-linux-musl"), None);
        // The one that a split-at-the-first-dash implementation gets wrong.
        assert_ne!(arch_for("arm-unknown-linux-musleabihf"), Some("arm"));
    }

    #[test]
    fn a_directory_that_is_not_a_repository_makes_no_claim() {
        let version = Version::compose("0.1.0", Path::new("/"));
        // `/` is inside no work tree on either host this project builds on. If
        // that ever stops being true the assertion below is what says so.
        assert_eq!(version.as_str(), "0.1.0");
        assert!(version.how().contains("not a git work tree"));
    }

    /// Asked of **this** repository, so the claim the name makes is exercised
    /// rather than described.
    ///
    /// **There are three outcomes, not two, and the third was found by a build
    /// rather than reasoned about** — corrected 2026-08-24. This assertion used
    /// to require that a bare version be explained by a *tag*, on the belief
    /// that `CARGO_MANIFEST_DIR` is always inside a readable work tree. It is
    /// not: built inside a **git worktree created by Windows `git` and compiled
    /// under WSL**, every `git` call in [`Version::compose`] answers
    ///
    /// ```text
    /// fatal: not a git repository: (null)
    /// ```
    ///
    /// because `.git` there is a *file* holding `gitdir: C:/…` and WSL's `git`
    /// cannot follow a Windows path. That is the same seam `AGENTS.md` §10.5
    /// already records between the two `git`s, arriving from a third direction,
    /// and the tool's own behaviour is correct throughout — it makes **no
    /// claim** when it cannot ask, which is `AGENTS.md` §5. The test was the
    /// thing that was wrong.
    ///
    /// So all three branches are asserted, and each is required to *say which
    /// one it was* — the property that actually matters is that the version and
    /// the explanation agree, not which of the three a given host produces.
    #[test]
    fn a_build_from_this_tree_says_which_claim_its_version_makes() {
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let version = Version::compose("0.1.0", here);
        let carries_sha = version.as_str().starts_with("0.1.0-g");
        let bare = version.as_str() == "0.1.0";
        assert!(carries_sha || bare, "{}", version.as_str());
        if bare {
            // A clean tag, or no work tree to ask. Both are honest bare
            // versions and they are distinguishable in the report.
            assert!(
                version.how().contains("tag") || version.how().contains("not a git work tree"),
                "{}",
                version.how()
            );
        } else {
            assert!(version.how().contains("built from"), "{}", version.how());
        }
    }

    // ---- a dirty version is unique — ADR-027 §3.2.2 ------------------------

    /// A `Version` in the state [`Version::compose`] leaves a dirty tree in,
    /// built by hand so the test does not need a dirty repository to exist.
    fn dirty(with_sha: &str) -> Version {
        Version {
            text: format!("{with_sha}-dirty"),
            how: "built from deadbee, with uncommitted tracked changes (ADR-027 §3.2.1)"
                .to_string(),
            dirty: true,
        }
    }

    fn write_temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        // `AGENTS.md` §8.3: the suite mints what it needs into a temp directory
        // and commits nothing. The name is per-test, so two of these cannot
        // collide inside one run.
        let at = std::env::temp_dir().join(format!("rivet-name-{name}.bin"));
        std::fs::write(&at, bytes).expect("a temporary file");
        at
    }

    /// The two members every application package has, as
    /// [`Version::distinguished_by`] takes them.
    fn payload<'a>(manifest: &'a [u8], program: &'a Path) -> Vec<Member<'a>> {
        vec![
            Member {
                name: "manifest.toml",
                class: 1,
                variant: 0,
                content: Content::Bytes(manifest),
            },
            Member {
                name: "hello",
                class: 2,
                variant: 0,
                content: Content::File(program),
            },
        ]
    }

    /// **The defect this section exists for**, stated as the property it must
    /// have: two builds of two different working trees at one commit must not
    /// produce one version.
    #[test]
    fn two_dirty_builds_of_different_bytes_do_not_share_a_version() {
        let one = write_temp("one", b"the program as it was");
        let two = write_temp("two", b"the program after an edit");

        let first = dirty("0.1.0-g7b84d4b")
            .distinguished_by(&payload(b"kind = \"daemon\"\n", &one))
            .expect("the payload is readable");
        let second = dirty("0.1.0-g7b84d4b")
            .distinguished_by(&payload(b"kind = \"daemon\"\n", &two))
            .expect("the payload is readable");

        assert_ne!(
            first.as_str(),
            second.as_str(),
            "two different dirty builds shared a version, which is ADR-027 §3.2.2's defect"
        );
        // Both still say `-dirty`: the claim §3.2.1 makes is unchanged and only
        // narrowed to a particular set of bytes.
        assert!(first.as_str().starts_with("0.1.0-g7b84d4b-dirty-"), "{}", first.as_str());
        assert!(second.as_str().starts_with("0.1.0-g7b84d4b-dirty-"), "{}", second.as_str());

        let _ = std::fs::remove_file(&one);
        let _ = std::fs::remove_file(&two);
    }

    /// **The defect the amendment of 2026-09-04 exists for**, and the one that
    /// was hit: the program is untouched and only the MANIFEST changed, which is
    /// the half the machine acts on. Under the first fix both builds produced one
    /// version, `ADR-015` §7.2 refused the install as already present, and the
    /// change silently did not happen.
    #[test]
    fn changing_only_the_manifest_changes_the_version() {
        let program = write_temp("manifest-only", b"the program, unchanged");

        let before = dirty("0.1.0-g7b84d4b")
            .distinguished_by(&payload(b"kind = \"daemon\"\nrestart = \"always\"\n", &program))
            .expect("readable");
        let after = dirty("0.1.0-g7b84d4b")
            .distinguished_by(&payload(b"kind = \"daemon\"\nrestart = \"never\"\n", &program))
            .expect("readable");

        assert_ne!(
            before.as_str(),
            after.as_str(),
            "a manifest-only edit produced the same version, so the install would be refused as \
             already present and the change would silently not happen"
        );

        let _ = std::fs::remove_file(&program);
    }

    /// **And an asset-only edit too**, which `.rulings/` R-142 makes the wider
    /// case: `assets/` ships everything in the directory, so a typeface or a
    /// card definition can change with neither the program nor the manifest.
    #[test]
    fn changing_only_an_asset_changes_the_version() {
        let program = write_temp("asset-program", b"the program, unchanged");
        let asset_before = write_temp("asset-before", b"a font, as it was");
        let asset_after = write_temp("asset-after", b"a font, subset differently");

        let with = |asset: &Path| {
            let mut parts = payload(b"kind = \"daemon\"\n", &program);
            parts.push(Member {
                name: "font.ttf",
                class: 3,
                variant: 0,
                content: Content::File(asset),
            });
            dirty("0.1.0-g7b84d4b")
                .distinguished_by(&parts)
                .expect("readable")
                .as_str()
                .to_string()
        };

        assert_ne!(with(&asset_before), with(&asset_after));

        for at in [&program, &asset_before, &asset_after] {
            let _ = std::fs::remove_file(at);
        }
    }

    /// The other half, and it is the one that makes the first mean something:
    /// **the same bytes must produce the same version.** A digest that varied
    /// per build would make every redeploy look like new code, which is the same
    /// failure pointing the other way.
    #[test]
    fn the_same_bytes_twice_produce_the_same_dirty_version() {
        let at = write_temp("same", b"identical bytes");
        let first = dirty("0.1.0-g7b84d4b")
            .distinguished_by(&payload(b"kind = \"daemon\"\n", &at))
            .expect("readable");
        let second = dirty("0.1.0-g7b84d4b")
            .distinguished_by(&payload(b"kind = \"daemon\"\n", &at))
            .expect("readable");
        assert_eq!(first.as_str(), second.as_str());
        let _ = std::fs::remove_file(&at);
    }

    /// **The determinism the amendment owes**, asserted explicitly rather than
    /// inferred from the test above.
    ///
    /// `cmd/cargo-rivet` reads its assets out of a DIRECTORY (`.rulings/` R-142
    /// ships everything in it), so the order members reach this function is the
    /// filesystem's. A digest that depended on it would give two identical
    /// payloads two versions — the same defect wearing the other face, and worse
    /// for being intermittent.
    #[test]
    fn the_digest_does_not_depend_on_the_order_the_members_are_given() {
        let program = write_temp("order-program", b"program bytes");
        let one = write_temp("order-asset-a", b"asset a");
        let two = write_temp("order-asset-b", b"asset b");

        let manifest = b"kind = \"daemon\"\n";
        let members = |flip: bool| {
            let mut parts = payload(manifest, &program);
            let mut assets = vec![
                Member { name: "a.png", class: 3, variant: 0, content: Content::File(&one) },
                Member { name: "b.png", class: 3, variant: 0, content: Content::File(&two) },
            ];
            if flip {
                assets.reverse();
                parts.reverse();
            }
            parts.extend(assets);
            parts
        };

        let straight = dirty("0.1.0-g7b84d4b")
            .distinguished_by(&members(false))
            .expect("readable");
        let shuffled = dirty("0.1.0-g7b84d4b")
            .distinguished_by(&members(true))
            .expect("readable");
        assert_eq!(
            straight.as_str(),
            shuffled.as_str(),
            "the same payload in a different order produced a different version"
        );

        for at in [&program, &one, &two] {
            let _ = std::fs::remove_file(at);
        }
    }

    /// **Two members cannot be run together into a third spelling.** The framing
    /// separates a name from what follows it with a `NUL`, which a member name
    /// cannot contain, so moving a character across the boundary changes the
    /// digest rather than being absorbed by it.
    #[test]
    fn a_character_moved_across_the_name_boundary_changes_the_digest() {
        let one = payload_digest(&[Member {
            name: "ab",
            class: 0,
            variant: 0,
            content: Content::Bytes(b"c"),
        }])
        .expect("bytes are readable");
        let two = payload_digest(&[Member {
            name: "a",
            class: 0,
            variant: 0,
            content: Content::Bytes(b"bc"),
        }])
        .expect("bytes are readable");
        assert_ne!(one, two);
    }

    /// **A clean version is not touched.** §3.2's bare version and §3.2's
    /// `-g<sha>` are both already unique statements about bytes that are in the
    /// history; only `-dirty` names bytes nothing records.
    #[test]
    fn a_clean_version_carries_no_digest() {
        let at = write_temp("clean", b"whatever");
        let tagged = Version {
            text: "0.1.0".to_string(),
            how: "HEAD is tag v0.1.0 and the tree is clean, so no SHA is carried".to_string(),
            dirty: false,
        };
        let after = tagged
            .clone()
            .distinguished_by(&payload(b"kind = \"daemon\"\n", &at))
            .expect("readable");
        assert_eq!(after, tagged, "a clean version was rewritten");
        let _ = std::fs::remove_file(&at);
    }

    /// **A member that cannot be read stops the build.** Falling back to the
    /// bare `-dirty` string would emit the exact name §3.2.2 exists to stop.
    #[test]
    fn a_program_that_cannot_be_read_is_refused_rather_than_skipped() {
        let missing = std::env::temp_dir().join("rivet-name-no-such-program-at-all");
        let _ = std::fs::remove_file(&missing);
        let refusal = dirty("0.1.0-g7b84d4b")
            .distinguished_by(&payload(b"kind = \"daemon\"\n", &missing))
            .expect_err("a missing program was hashed");
        assert!(refusal.contains("cannot read"), "{refusal}");
    }

    /// **The budget, asserted rather than assumed.** `ADR-029` §3.1 refuses a
    /// `version` longer than 32 bytes rather than truncating it, and a truncated
    /// digest would put two builds back on one name — so the worst realistic
    /// composition has to fit with room to spare.
    #[test]
    fn the_longest_ordinary_dirty_version_fits_the_headers_field() {
        let at = write_temp("budget", b"bytes");
        // A three-part SemVer at the width this project's own crates reach.
        let composed = dirty("10.20.30-g7b84d4b")
            .distinguished_by(&payload(b"kind = \"daemon\"\n", &at))
            .expect("readable");
        assert!(
            composed.as_str().len() <= rivet_core::package::format::VERSION_BYTES,
            "{} is {} bytes and the header field is {}",
            composed.as_str(),
            composed.as_str().len(),
            rivet_core::package::format::VERSION_BYTES
        );
        let _ = std::fs::remove_file(&at);
    }

    /// The digest is reported, not merely applied — [`Version::how`] is what a
    /// build prints, and a claim in a name that is not explained anywhere is
    /// what §3.2.1's own amendment was written about. **It names the members**,
    /// so a reader can see that the manifest is in the digest rather than having
    /// to take this module's word for it.
    #[test]
    fn the_report_says_where_the_digest_came_from() {
        let at = write_temp("report", b"bytes");
        let composed = dirty("0.1.0-g7b84d4b")
            .distinguished_by(&payload(b"kind = \"daemon\"\n", &at))
            .expect("readable");
        assert!(composed.how().contains("BLAKE3"), "{}", composed.how());
        assert!(composed.how().contains("ADR-027 §3.2.2"), "{}", composed.how());
        assert!(composed.how().contains("manifest.toml"), "{}", composed.how());
        let _ = std::fs::remove_file(&at);
    }

    /// A distinguished dirty name must still read back to the product and kind
    /// it was composed from — the extra token lands in the variable region and
    /// `product_and_kind` scans from the right.
    #[test]
    fn a_distinguished_dirty_name_still_reads_back() {
        let name = artifact(
            "rivet-todo",
            "0.1.1-g3d28ede-dirty-1a2b3c4d",
            "aarch64",
            None,
            &Kind::application(),
        );
        let (product, kind) = product_and_kind(&name).expect("this tool composed it");
        assert_eq!(product, "rivet-todo");
        assert_eq!(kind, Kind::application().to_string());
    }

    // ---- reading a name back -----------------------------------------------

    /// **The anti-drift property.** `ARCH_TOKENS` is a second list of the same
    /// values `arch_for` maps to, and this is what stops it being a second
    /// *authority*: every triple that function knows must produce a token this
    /// one contains.
    #[test]
    fn every_arch_the_composer_knows_is_one_the_reader_knows() {
        for triple in [
            "aarch64-unknown-linux-musl",
            "x86_64-unknown-linux-musl",
            "arm-unknown-linux-musleabihf",
        ] {
            let arch = arch_for(triple).expect("a known triple");
            assert!(
                ARCH_TOKENS.contains(&arch),
                "{triple} maps to {arch}, which product_and_kind cannot read back"
            );
        }
    }

    /// The round trip, over names with a hyphen in every field that may have
    /// one. This is the whole of what holds the reader to the composer.
    #[test]
    fn a_name_this_tool_composed_reads_back_to_what_composed_it() {
        let cases = [
            ("hello", "0.1.0", "aarch64", None, Kind::application()),
            ("rivet-todo", "0.1.1-g3d28ede", "aarch64", None, Kind::application()),
            ("a-b-c-d", "1.2.3", "x86_64", None, Kind::application()),
            ("rivet", "0.0.1-9d3f3a0", "aarch64", Some("rpi4"), Kind::update()),
            ("rivet", "0.0.1", "armv6", Some("rpi4"), Kind::update()),
            // A digit-led token in the product must stay in the product: `2`
            // begins with a digit but is not shaped like a crate version, and
            // `3d` isn't either, so neither is mistaken for the version start.
            ("sensor-2", "0.1.0", "aarch64", None, Kind::application()),
            ("3d-probe", "0.1.0", "aarch64", None, Kind::application()),
        ];
        for (product, version, arch, platform, kind) in cases {
            let name = artifact(product, version, arch, platform, &kind);
            let (read_product, read_kind) =
                product_and_kind(&name).unwrap_or_else(|| panic!("{name} did not read back"));
            assert_eq!(read_product, product, "{name}");
            assert_eq!(read_kind, kind.to_string(), "{name}");
        }
    }

    /// A name this tool did not write must be refused rather than guessed at:
    /// `--enable` puts the answer in the operator's document.
    #[test]
    fn a_name_this_tool_did_not_compose_is_refused() {
        for bad in [
            "notanrvt.tar",
            "hello.rvt",
            "hello-0.1.0.rvt",
            "hello-0.1.0-sparc-application.rvt",
            "0.1.0-1.2.3-aarch64-application.rvt",
            "hello-world-aarch64-application.rvt",
        ] {
            assert!(
                product_and_kind(bad).is_none(),
                "{bad:?} was read as a name this tool composed"
            );
        }
    }
}
