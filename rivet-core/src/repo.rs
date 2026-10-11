//! **What to do about the repositories on a disk**, decided without touching
//! one.
//!
//! `crate::git` says where they live; `rivet_config::repos` says which ones an
//! operator wants. This is the arithmetic between the two, and it performs no
//! I/O so that every branch of it is reachable from a test with no filesystem
//! and no git.
//!
//! # The whole idea in one paragraph
//!
//! A repository's **identifier** is the repository (`ADR-025` §4.1); its
//! **name** is a directory. The identifier is written inside the directory, in
//! a file git never touches, so a machine can answer *which repository is
//! this* by reading a file rather than by trusting where it happens to be. Once
//! it can do that, a rename is a `rename(2)` instead of a delete and a create.
//!
//! # Four things can be true of one repository, and only four
//!
//! | on disk | declared | [`Action`] |
//! | :-- | :-- | :-- |
//! | nowhere | yes | [`Action::Create`] |
//! | under another name | yes | [`Action::Rename`] |
//! | under this name | yes | nothing |
//! | under this name, **different id** | yes | [`Action::Collision`] |
//!
//! And one thing can be true of a directory nobody declared: **nothing happens
//! to it.** [`Action::Undeclared`] is a report, not a plan. The difference
//! between *"I removed a line from a file"* and *"I destroyed the only copy of
//! a year's work"* must not be a diff nobody read carefully.
//!
//! # Why a collision is refused rather than resolved
//!
//! `example-repo.git` exists and carries id `A`; the document says
//! `example-repo` is id `B`. Every available resolution is worse than refusing:
//! moving `A` out of the way invents a name nobody chose, overwriting it
//! destroys data, and serving `A` under `B`'s name makes the identifier a lie.
//! So the machine reports and does nothing — `ADR-011`, in the one place two
//! answers to *which repository is this* can actually meet.

/// A repository's name.
///
/// **In `rivet-core` since 2026-08-26**, and it moved on the day the second
/// consumer appeared — `crate::repo_delete`, whose parser is the boundary a
/// hostile file crosses. A request naming `../../system` must never become a
/// path, and the check that stops it cannot live in a crate the core may not
/// depend on. `rivet_config::repos` re-exports it, exactly as it re-exports
/// `AccountName`.
///
/// **This is a directory name and a URL path component**, which is what the
/// grammar is chosen to survive. `cmd/rivet-gitd::request::repository_name`
/// refuses anything else on the wire; this refuses it in the document, and the
/// two must agree — a name a document accepted and the shell refused would be a
/// repository nobody could reach and nothing would report.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepoName(String);

impl RepoName {
    /// The longest a name may be.
    ///
    /// **255 bytes is the filesystem's limit**, not a taste. `ext4` and every
    /// other filesystem Rivet will meet cap a single directory entry there, and
    /// the projection appends `.git` — so the limit is stated against the name
    /// with the suffix already subtracted, because a refusal at `mkdir` time on
    /// a headless board is a worse place to learn it.
    pub const MAX_BYTES: usize = 255 - 4;

    /// The rule, in the words a refusal uses.
    pub const RULE: &'static str =
        "1 to 251 bytes of lowercase letters, digits, `-` and `_`. It may \
         not begin with `-`, which git would read as an option, and it may not contain `.` — \
         TOML would read `[repos.a.b]` as a table inside a table rather than as a repository \
         named `a.b`, so a dotted name would mean something the operator did not write";

    /// The characters a name may contain.
    ///
    /// **Lowercase only**, for `AccountName`'s reason at one remove: this is a
    /// directory on a filesystem that may or may not fold case, and two
    /// spellings of one repository is the failure the whole id scheme exists to
    /// prevent. It is also what a client types, and `Example-Repo` versus
    /// `example-repo` resolving differently on two machines is not a thing to
    /// discover during a push.
    ///
    /// # `.` is refused, and it was refused by a test rather than by foresight
    ///
    /// The first draft allowed it, and the round-trip fixture `[repos.a-b_c.d1]`
    /// failed — **TOML reads a bare dotted key as table nesting**, so that line
    /// declares a table `d1` inside a table `a-b_c`, not a repository named
    /// `a-b_c.d1`. The quoted form `[repos."a-b_c.d1"]` would work, which means
    /// the document's meaning would depend on whether the operator quoted a
    /// key — and the unquoted spelling would fail with an error about a key
    /// called `d1` that he never wrote.
    ///
    /// So it is refused. `ServiceName`, `AccountName` and `InterfaceName` all
    /// refuse it already; a repository named `config.nvim` is a real thing to
    /// give up, and it is given up for a document that means one thing.
    fn is_allowed(b: u8) -> bool {
        b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_')
    }

    /// Read a name, or refuse it.
    #[must_use]
    pub fn new(text: &str) -> Option<Self> {
        if text.is_empty() || text.len() > Self::MAX_BYTES {
            return None;
        }
        if !text.bytes().all(Self::is_allowed) {
            return None;
        }
        // `-` would be an option to git. `.`, `..` and a leading `.` are all
        // already unreachable: `is_allowed` refuses the character outright, and
        // so is `x.git` — the machine appends that suffix itself.
        if text.starts_with('-') {
            return None;
        }
        Some(Self(text.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for RepoName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The file inside a bare repository that says which repository it is.
///
/// **`rivet-id`, at the top of the bare directory.** git's own layout is a
/// fixed set of names — `HEAD`, `config`, `objects`, `refs`, `hooks`,
/// `branches`, `info`, `description` — and it neither creates nor reads
/// anything called this, so the file is invisible to every git operation and
/// survives every one of them.
///
/// **Not `git config rivet.id`**, which would have been the idiomatic place:
/// reading it means running git, and a supervisor that had to fork a process to
/// find out what is on its own disk could not answer during early boot, could
/// not answer if the binary were missing, and would turn a directory listing
/// into N subprocesses. It is also a file an operator can edit into a
/// collision, which is exactly why the collision case above is handled rather
/// than assumed away.
pub const ID_FILE: &str = "rivet-id";

/// The default branch a new repository gets.
///
/// **`main`.** git's own compiled-in default is still `master`, and
/// `git init --bare` on a machine with no user configuration would produce a
/// repository whose `HEAD` points at a branch nobody in this project uses. It
/// is passed explicitly on every creation rather than configured globally,
/// because a global setting is a thing that can be absent.
pub const DEFAULT_BRANCH: &str = "main";

/// One repository as it exists on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnDisk {
    /// The directory name **without** the `.git` suffix.
    pub name: String,
    /// What its [`ID_FILE`] said, or `None` if it had none or could not be
    /// read.
    ///
    /// **`None` is not an error here.** A directory under the repository root
    /// with no identifier is something Rivet did not create — a leftover, a
    /// hand-made repository, a half-finished copy — and the answer is the same
    /// as for any undeclared repository: leave it alone and say so.
    pub id: Option<String>,
}

/// What the machine should do about one repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// It does not exist. `git init --bare -b main`, then write the id.
    Create { name: String, id: String },
    /// It exists under another name. `rename(2)`.
    Rename { from: String, to: String, id: String },
    /// The name is taken by a different repository. **Do nothing and say so.**
    Collision { name: String, wanted: String, found: Option<String> },
    /// A directory nobody declared. **A report, never a plan.**
    Undeclared { name: String, id: Option<String> },
}

impl Action {
    /// Whether this action changes anything on disk.
    ///
    /// The two that do not are the two that matter most to a reader, which is
    /// why this exists rather than a `matches!` at each call site.
    #[must_use]
    pub const fn changes_the_disk(&self) -> bool {
        matches!(self, Self::Create { .. } | Self::Rename { .. })
    }

    /// A sentence for the journal, naming what was wrong rather than only that
    /// something was.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Create { name, .. } => format!(
                "`{name}` is declared and does not exist, so this machine is creating it"
            ),
            Self::Rename { from, to, .. } => format!(
                "`{from}` and `{to}` are one repository under two names - the identifier says so \
                 - so this machine is moving it rather than making a second one"
            ),
            Self::Collision { name, wanted, found } => format!(
                "`{name}` is declared as `{wanted}` and the directory already there is {}. \
                 NOTHING HAS BEEN DONE. Moving it aside would invent a name nobody chose, \
                 overwriting it would destroy data, and serving it under this name would make \
                 the identifier a lie. Rename one of the two in the document, or move the \
                 directory by hand.",
                match found {
                    Some(id) => format!("`{id}`"),
                    None => "not a repository this machine created".to_owned(),
                }
            ),
            Self::Undeclared { name, .. } => format!(
                "`{name}` is on this disk and is not in the document. IT HAS BEEN LEFT EXACTLY \
                 WHERE IT IS: undeclaring a repository never deletes one, because the difference \
                 between removing a line from a file and destroying the only copy of a year's \
                 work must not be a diff nobody read. To destroy it, ask for that."
            ),
        }
    }
}

/// One repository as the document declares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declared {
    pub name: String,
    pub id: String,
}

/// Decide what to do, given what is declared and what is on disk.
///
/// **Deterministic and order-independent in its result**, though the returned
/// order follows the declarations and then the leftovers, so that a journal
/// reads the way an operator's document does.
#[must_use]
pub fn plan(declared: &[Declared], on_disk: &[OnDisk]) -> Vec<Action> {
    let mut actions = Vec::new();
    let mut accounted: Vec<&str> = Vec::new();

    for want in declared {
        // The identifier is the repository, so it is looked up first and the
        // name is looked up only to find out whether it is already right.
        let here = on_disk.iter().find(|d| d.id.as_deref() == Some(want.id.as_str()));
        let at_the_name = on_disk.iter().find(|d| d.name == want.name);

        match (here, at_the_name) {
            // Already where it should be.
            (Some(found), _) if found.name == want.name => {
                accounted.push(&found.name);
            }
            // Somewhere else, and the wanted name is free: move it.
            (Some(found), None) => {
                accounted.push(&found.name);
                actions.push(Action::Rename {
                    from: found.name.clone(),
                    to: want.name.clone(),
                    id: want.id.clone(),
                });
            }
            // Somewhere else, and the wanted name is taken by something else.
            (Some(found), Some(blocker)) => {
                accounted.push(&found.name);
                accounted.push(&blocker.name);
                actions.push(Action::Collision {
                    name: want.name.clone(),
                    wanted: want.id.clone(),
                    found: blocker.id.clone(),
                });
            }
            // Not on this disk, and the name is free: create it.
            (None, None) => actions.push(Action::Create {
                name: want.name.clone(),
                id: want.id.clone(),
            }),
            // Not on this disk, and the name is taken by something else.
            (None, Some(blocker)) => {
                accounted.push(&blocker.name);
                actions.push(Action::Collision {
                    name: want.name.clone(),
                    wanted: want.id.clone(),
                    found: blocker.id.clone(),
                });
            }
        }
    }

    // Everything left is reported and untouched.
    for found in on_disk {
        if !accounted.contains(&found.name.as_str()) {
            actions.push(Action::Undeclared {
                name: found.name.clone(),
                id: found.id.clone(),
            });
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn want(name: &str, id: &str) -> Declared {
        Declared { name: name.to_owned(), id: id.to_owned() }
    }

    fn have(name: &str, id: Option<&str>) -> OnDisk {
        OnDisk { name: name.to_owned(), id: id.map(ToOwned::to_owned) }
    }

    const A: &str = "4f2c9a01bd7e3865";
    const B: &str = "a1b2c3d4e5f60718";

    #[test]
    fn a_declared_repository_that_is_not_there_is_created() {
        assert_eq!(
            plan(&[want("example-repo", A)], &[]),
            vec![Action::Create { name: "example-repo".to_owned(), id: A.to_owned() }]
        );
    }

    #[test]
    fn a_repository_already_where_it_belongs_is_left_alone() {
        assert_eq!(plan(&[want("example-repo", A)], &[have("example-repo", Some(A))]), vec![]);
    }

    /// **The whole reason the table carries an identifier.** Rename the table,
    /// keep the id, and the machine moves the directory — rather than creating
    /// an empty repository and reporting the old one as undeclared, which is
    /// what a name-keyed projection would do with somebody's history.
    #[test]
    fn a_renamed_declaration_moves_the_directory_rather_than_making_a_second_one() {
        let actions = plan(&[want("new-name", A)], &[have("old-name", Some(A))]);
        assert_eq!(
            actions,
            vec![Action::Rename {
                from: "old-name".to_owned(),
                to: "new-name".to_owned(),
                id: A.to_owned(),
            }]
        );
        // And specifically NOT these two, which is the failure being prevented.
        assert!(!actions.iter().any(|a| matches!(a, Action::Create { .. })));
        assert!(!actions.iter().any(|a| matches!(a, Action::Undeclared { .. })));
    }

    /// **Undeclaring deletes nothing**, and there is no `Action` that could.
    #[test]
    fn an_undeclared_repository_is_reported_and_never_removed() {
        let actions = plan(&[], &[have("example-repo", Some(A))]);
        assert_eq!(
            actions,
            vec![Action::Undeclared {
                name: "example-repo".to_owned(),
                id: Some(A.to_owned()),
            }]
        );
        assert!(!actions[0].changes_the_disk());
        assert!(actions[0].describe().contains("LEFT EXACTLY WHERE IT IS"));
    }

    /// A directory Rivet did not create is an undeclared repository, not a
    /// fault — a leftover, a hand-made repository, a half-finished copy.
    #[test]
    fn a_directory_with_no_identifier_is_left_alone_like_any_other() {
        let actions = plan(&[], &[have("mystery", None)]);
        assert_eq!(actions, vec![Action::Undeclared { name: "mystery".to_owned(), id: None }]);
    }

    /// **Every resolution is worse than refusing**, so the machine does
    /// nothing and says which two identifiers met.
    #[test]
    fn a_name_claimed_by_a_different_repository_is_refused_and_nothing_is_touched() {
        let actions = plan(&[want("example-repo", B)], &[have("example-repo", Some(A))]);
        assert_eq!(
            actions,
            vec![Action::Collision {
                name: "example-repo".to_owned(),
                wanted: B.to_owned(),
                found: Some(A.to_owned()),
            }]
        );
        assert!(!actions[0].changes_the_disk());
        let said = actions[0].describe();
        assert!(said.contains("NOTHING HAS BEEN DONE"), "{said}");
        assert!(said.contains(A) && said.contains(B), "{said}");
    }

    /// A rename **into** an occupied name is the same refusal, and the
    /// repository being moved is not touched either.
    #[test]
    fn a_rename_onto_an_occupied_name_moves_nothing() {
        let actions = plan(
            &[want("taken", A)],
            &[have("original", Some(A)), have("taken", Some(B))],
        );
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::Collision { .. }));
        // Neither directory is reported as undeclared: both were accounted for,
        // and reporting the one being moved would read as "about to be lost".
        assert!(!actions.iter().any(|a| matches!(a, Action::Undeclared { .. })));
    }

    /// A directory that exists with no identifier, under a name the document
    /// claims, is a collision rather than a silent adoption — **adopting it
    /// would hand somebody's stray directory an identifier and a URL.**
    #[test]
    fn an_unidentified_directory_under_a_declared_name_is_not_adopted() {
        let actions = plan(&[want("example-repo", A)], &[have("example-repo", None)]);
        assert_eq!(
            actions,
            vec![Action::Collision {
                name: "example-repo".to_owned(),
                wanted: A.to_owned(),
                found: None,
            }]
        );
        assert!(actions[0].describe().contains("not a repository this machine created"));
    }

    /// A whole machine at once: one settled, one created, one moved, one
    /// leftover — and the order follows the document, then the leftovers.
    #[test]
    fn a_mixed_machine_produces_one_action_per_repository_in_document_order() {
        let actions = plan(
            &[want("settled", A), want("fresh", "0000000000000001"), want("moved", B)],
            &[have("settled", Some(A)), have("was-moved", Some(B)), have("leftover", None)],
        );
        assert_eq!(
            actions,
            vec![
                Action::Create { name: "fresh".to_owned(), id: "0000000000000001".to_owned() },
                Action::Rename {
                    from: "was-moved".to_owned(),
                    to: "moved".to_owned(),
                    id: B.to_owned(),
                },
                Action::Undeclared { name: "leftover".to_owned(), id: None },
            ]
        );
    }

    /// **Converging.** Running the plan and asking again produces nothing —
    /// which is what makes this safe to run on every boot.
    #[test]
    fn a_plan_applied_leaves_nothing_to_do() {
        let declared = [want("example-repo", A), want("notes", B)];
        let settled = [have("example-repo", Some(A)), have("notes", Some(B))];
        assert_eq!(plan(&declared, &settled), vec![]);
    }

    #[test]
    fn an_ordinary_repository_name_is_accepted() {
        for good in ["example-repo", "notes", "a", "rivet", "my_repo", "v1-2", "x9"] {
            assert!(RepoName::new(good).is_some(), "{good:?}");
        }
    }

    /// **The grammar must agree with `cmd/rivet-gitd`'s.** A name the document
    /// accepted and the shell refused would be a repository nobody could reach
    /// and nothing would report.
    #[test]
    fn nothing_this_accepts_is_something_the_shell_would_refuse() {
        for good in ["example-repo", "notes", "my_repo", "v1-2"] {
            let name = RepoName::new(good).expect(good);
            // The shell's own rules, restated as the properties they are: one
            // component, no traversal, not an option.
            assert!(!name.as_str().contains('/'));
            assert!(!name.as_str().contains(".."));
            assert!(!name.as_str().starts_with('-'));
        }
    }

    /// A name git would read as an option, or a filesystem would hide, or that
    /// names a directory rather than a repository.
    #[test]
    fn a_name_that_is_not_a_usable_directory_is_refused() {
        for bad in [
            "",
            "-x",              // git would read it as an option
            "Example",         // uppercase: two spellings of one repository
            "has space",
            "has/slash",
            "has:colon",
            "emoji-\u{1f600}",
        ] {
            assert_eq!(RepoName::new(bad), None, "{bad:?}");
        }
    }

    /// **A dot is refused, and a test is why.** TOML reads `[repos.a.b]` as a
    /// table inside a table, so a dotted name would mean something the operator
    /// did not write — and the unquoted spelling fails with an error about a
    /// key he never typed.
    ///
    /// It also disposes of `.`, `..`, a leading dot and the `.git` suffix in
    /// one rule rather than four.
    #[test]
    fn a_dot_is_refused_because_toml_would_read_it_as_nesting() {
        for bad in ["a.b", "v1.2", ".", "..", ".hidden", "example.git", "config.nvim"] {
            assert_eq!(RepoName::new(bad), None, "{bad:?}");
        }
        assert!(RepoName::RULE.contains("TOML"));
        assert!(RepoName::new("example").is_some());
    }

    /// The limit has the suffix already subtracted, because a refusal at
    /// `mkdir` time on a headless board is a worse place to learn it.
    #[test]
    fn the_length_limit_leaves_room_for_the_suffix() {
        assert_eq!(RepoName::MAX_BYTES + crate::git::BARE_SUFFIX.len(), 255);
        assert!(RepoName::new(&"a".repeat(RepoName::MAX_BYTES)).is_some());
        assert_eq!(RepoName::new(&"a".repeat(RepoName::MAX_BYTES + 1)), None);
    }

    /// The default branch is `main`, passed explicitly on every creation. git's
    /// own compiled-in default is still the other one.
    #[test]
    fn a_new_repository_gets_the_branch_this_project_uses() {
        assert_eq!(DEFAULT_BRANCH, "main");
    }

    /// The identifier file is not a name git uses, so no git operation touches
    /// it — asserted against git's actual bare layout.
    #[test]
    fn the_identifier_file_is_not_a_name_git_owns() {
        for gits in [
            "HEAD", "config", "objects", "refs", "hooks", "branches", "info", "description",
            "packed-refs", "FETCH_HEAD", "ORIG_HEAD",
        ] {
            assert_ne!(ID_FILE, gits);
        }
    }
}
