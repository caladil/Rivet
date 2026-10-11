//! The `[repos]` table: **which git repositories this machine serves.**
//!
//! Ruled 2026-08-26, on `ADR-032` §4.2's account and `PLAN.md` §46's shell.
//!
//! ```toml
//! [repos.example-repo]
//! id = "4f2c9a01bd7e3865"
//!
//! [repos.notes]
//! id = "a1b2c3d4e5f60718"
//! ```
//!
//! # The id is the repository; the name is where it answers
//!
//! `ADR-025` §4.1 ruled this for a fleet member — *"a stable opaque identifier
//! is authoritative … the name is a mutable label"* — and the maintainer
//! applied it here on 2026-08-26, because a repository has more than once had
//! to be renamed (`.rulings/` R-357).
//!
//! So a rename is one edit. `[repos.old-name]` becomes `[repos.new-name]` with
//! the **same id**, and the machine moves the directory. Without the id that
//! edit is indistinguishable from *delete one repository and create another*,
//! and the difference between those two is somebody's history.
//!
//! # Why the directory is named by the name and not by the id
//!
//! Because whatever serves a repository must not read this document. A client
//! asks for a repository **by name**; if the directory were
//! `4f2c9a01bd7e3865.git` the serving program would need a name-to-id lookup,
//! which means reading the desired state, which means the program that stands
//! between a network and a filesystem grows a parser and a second authority
//! over what a repository is.
//!
//! **The id lives inside the repository instead**, in a file the projection
//! writes and git never touches, so the machine can answer *which repository is
//! this directory* without anything else having to.
//!
//! # What declaring a repository does, and the one thing it never does
//!
//! `ADR-031` §7.5: *a schema says what may be asked for; an apply says what can
//! be done.* Declaring one **creates** it — `git init --bare`, once, converging.
//! Renaming one **moves** it.
//!
//! **Undeclaring one deletes nothing.** A repository whose declaration is gone
//! is left exactly where it is and reported, because the difference between
//! *"I removed a line from a file"* and *"I destroyed the only copy of a year's
//! work"* must not be a diff nobody read carefully. Destroying one is a
//! separate, explicit, confirmed operation — which is `ADR-012` §7.1's rule
//! about the actions that cannot be taken back.
//!
//! # There is no access control here, and that is a ruling
//!
//! Ruled 2026-08-26: **no per-repository access control.** Anything that can
//! reach this machine's repositories can reach all of them.
//!
//! The reasoning is that the two halves cost very differently. An access rule is
//! **cheap to add later** — a field on this struct, which is what this schema
//! does every day — so buying it before there is a second party for it to be
//! about would be a mechanism for a case that cannot arise, and `ADR-007` §3.6
//! refuses a key nothing would read.

/// A repository's name — **`rivet_core::repo::RepoName`, re-exported.**
///
/// The definition lives in `rivet-core` for `ServiceName`'s reason exactly, and
/// it moved on the day the second consumer appeared:
/// `rivet_core::repo_delete`'s parser has to refuse a name that could traverse,
/// and that check cannot live in a crate the core may not depend on. Two
/// spellings of *what a valid repository name is* would be two answers to a
/// question a `remove_tree` depends on.
pub use rivet_core::repo::RepoName;

/// The suffix the machine appends — `rivet_core::git`'s, not a second copy.
pub use rivet_core::git::BARE_SUFFIX;

/// A repository's identifier — **the repository itself.**
///
/// `rivet_core::identifier`'s grammar, which is `ADR-025` §4.1's and is shared
/// with a fleet member's. See the module header for why a repository has one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepoId(String);

impl RepoId {
    /// The rule, in the words a refusal uses.
    pub const RULE: &'static str = rivet_core::identifier::RULE;

    /// Read an identifier, or refuse it.
    ///
    /// **Minted, never chosen.** `rivet-fleet add repo` produces one; a
    /// hand-written value is refused here rather than after something
    /// references it.
    #[must_use]
    pub fn new(text: &str) -> Option<Self> {
        rivet_core::identifier::is_identifier(text).then(|| Self(text.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for RepoId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One repository this machine serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    name: RepoName,
    id: RepoId,
}

impl Repo {
    /// Every key this table declares, in schema order.
    ///
    /// **One key, and `id` is required.** A repository without one could not be
    /// renamed — which is the entire reason the table has this shape — and
    /// `ADR-007` §3.6 refuses a document it would have to guess about.
    pub const DECLARED_KEYS: &'static [&'static str] = &["id"];

    /// Build a repository. The parser is the only caller today.
    pub(crate) const fn new(name: RepoName, id: RepoId) -> Self {
        Self { name, id }
    }

    /// Where this repository answers. **A mutable label** — see the header.
    #[must_use]
    pub const fn name(&self) -> &RepoName {
        &self.name
    }

    /// Which repository this is. **Authoritative, and never edited.**
    #[must_use]
    pub const fn id(&self) -> &RepoId {
        &self.id
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **The grammar is the core's now**, and this is the one property this
    /// crate still owns: that the re-export is the same type the parser uses.
    /// The grammar's own tests moved with it.
    #[test]
    fn the_name_this_crate_publishes_is_the_one_the_core_enforces() {
        assert!(RepoName::new("example-repo").is_some());
        assert_eq!(RepoName::new("Example.Repo"), None);
        assert_eq!(RepoName::RULE, rivet_core::repo::RepoName::RULE);
    }

    /// An id is minted, never chosen — a hand-written one is refused here
    /// rather than after something references it.
    #[test]
    fn an_identifier_is_the_one_the_register_mints() {
        assert!(RepoId::new("4f2c9a01bd7e3865").is_some());
        for bad in ["", "4f2c9a01", "4F2C9A01BD7E3865", "not-hex-at-all!!", "my-repo"] {
            assert_eq!(RepoId::new(bad), None, "{bad:?}");
        }
    }

    /// **One grammar, two subjects.** A register that accepted an id a document
    /// refused is the failure the move into `rivet-core` prevents.
    #[test]
    fn a_repository_id_and_a_device_id_are_one_grammar() {
        let minted = rivet_core::identifier::from_entropy(&[1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(RepoId::new(&minted).is_some(), "{minted}");
        assert_eq!(RepoId::RULE, rivet_core::identifier::RULE);
    }

    /// `id` is required, and it is the whole table. A repository without one
    /// could not be renamed, which is why the table has this shape at all.
    #[test]
    fn the_table_declares_exactly_one_key() {
        assert_eq!(Repo::DECLARED_KEYS, ["id"]);
    }
}
