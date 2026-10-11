//! **The one place this system destroys a repository.**
//!
//! `cmd/rivet` writes a confirmed request onto `/state` and returns;
//! this reads it, checks the two things only a machine can check, and removes
//! the directory. `rivet_core::repo_delete` carries the argument for the split
//! and the interlocks; `crate::uninstall_request` is the module this is shaped
//! after, down to the order of operations.
//!
//! # The removal lives here and not in `crate::repos`
//!
//! Deliberately, and it is the whole reason this is a separate module.
//! `crate::repos` converges the repository root on every boot, from the
//! document — and it asserts, against its own source, that neither
//! `remove_file` nor `crate::reclaim` appears in it. That
//! assertion is what makes *"undeclaring a repository never deletes one"* a
//! property of the code rather than a promise about it.
//!
//! So the destructive call is over here, reached only by a request an operator
//! confirmed out loud, and no edit to a document can arrive at it.
//!
//! # Two checks, and only this process can make either
//!
//! 1. **Still declared?** If `[repos]` names it, the next convergence recreates
//!    it — so the delete would appear to have failed, and an operator would
//!    reasonably conclude the command is broken and reach for `rm -rf`. Refused
//!    with the order to do it in.
//! 2. **The right repository?** The request carries the identifier the operator
//!    expected; the directory carries its own. If they differ this is not the
//!    repository he meant, and it is not touched. A name is a label, labels get
//!    reused, and this is the one operation where being wrong is permanent.
//!
//! # The request is removed before the work, not after
//!
//! `crate::uninstall_request`'s rule and its reason: acting on a request that
//! stays on the card is how one delete becomes every convergence pass. A crash
//! between the removal and the work costs **one** delete, which an operator can
//! see did not happen and can ask for again. The other order costs the disk.

use rivet_core::{git, repo, repo_delete};

use crate::journal::{Severity, Value};
use crate::kernel::{Errno, Kernel};
use crate::supervise::{Phase, Supervisor};

/// Reads the request path once per convergence pass.
#[derive(Default)]
pub struct Watch {
    /// Whether an episode of trouble about the request file has been reported.
    /// Once is information; once per pass is noise.
    trouble_reported: bool,
}

impl Watch {
    pub fn new() -> Self {
        Self::default()
    }

    /// `document` is needed for the first check and is the reason this takes
    /// one at all: *is it still declared?* cannot be answered from the request.
    pub fn poll(
        &mut self,
        k: &mut dyn Kernel,
        sup: &mut Supervisor,
        document: &rivet_config::Document,
    ) {
        if sup.phase() != Phase::Converging {
            return;
        }

        let bytes = match k.file_size(repo_delete::REQUEST_PATH) {
            Ok(bytes) => bytes,
            // The ordinary state of every machine that is not destroying
            // anything. Silent, deliberately.
            Err(Errno::ENOENT) => {
                self.trouble_reported = false;
                return;
            }
            Err(e) => {
                self.report_trouble(k, sup, "could not be examined", &e.to_string());
                return;
            }
        };

        if bytes > repo_delete::REQUEST_MAX_BYTES {
            self.refuse(
                k,
                sup,
                &format!(
                    "it is {bytes} bytes and a request may be at most {} — so it was NOT read \
                     into this process at all",
                    repo_delete::REQUEST_MAX_BYTES
                ),
            );
            return;
        }

        let contents = match k.read_file(repo_delete::REQUEST_PATH) {
            Ok(contents) => contents,
            Err(e) => {
                self.refuse(k, sup, &format!("it could not be read: {e}"));
                return;
            }
        };

        let request = match repo_delete::parse_request(&contents) {
            Ok(request) => request,
            Err(problem) => {
                self.refuse(k, sup, &problem.why());
                return;
            }
        };

        // **Before anything is removed**, so a crash costs one delete rather
        // than a loop. See this module's documentation.
        if let Err(e) = k.remove_file(repo_delete::REQUEST_PATH) {
            self.report_trouble(
                k,
                sup,
                "was read but could not be removed, so NOTHING WAS DESTROYED — acting on a \
                 request that stays on the card is how one delete becomes every convergence pass",
                &e.to_string(),
            );
            return;
        }
        self.trouble_reported = false;

        self.perform(k, sup, document, &request);
    }

    /// The two checks, and then the removal.
    fn perform(
        &mut self,
        k: &mut dyn Kernel,
        sup: &mut Supervisor,
        document: &rivet_config::Document,
        request: &repo_delete::Request,
    ) {
        let name = request.name.as_str();

        // **Check one: is it still declared?**
        if document.repos().iter().any(|r| r.name().as_str() == name) {
            self.refuse(
                k,
                sup,
                &format!(
                    "`{name}` is STILL DECLARED in this machine's document, so the next \
                     convergence would create it again and the delete would look as though it \
                     had failed. Undeclare it first — `rivet-fleet remove repo --device <d> \
                     --repo {name} --confirm`, then `apply document` — and ask again."
                ),
            );
            return;
        }

        let dir = git::repository_dir(name);

        // **Check two: is it the repository he meant?**
        let id_path = format!("{dir}/{}", repo::ID_FILE);
        match k.read_file(&id_path) {
            Ok(text) if text.trim() == request.id => {}
            Ok(text) => {
                self.refuse(
                    k,
                    sup,
                    &format!(
                        "the directory at {dir} carries `{}` and the request asked to destroy \
                         `{}`. THIS IS NOT THE REPOSITORY THAT WAS MEANT and it has not been \
                         touched. A name is a label and labels get reused (ADR-025 4.1); this \
                         check is the whole reason the identifier is typed.",
                        text.trim(),
                        request.id,
                    ),
                );
                return;
            }
            Err(Errno::ENOENT) => {
                // Not a fault. It is the state after a successful delete, and
                // the state of a machine that never had this repository.
                sup.note(
                    k,
                    Severity::Info,
                    "a repository this machine does not have was asked to be destroyed, so \
                     there was nothing to do",
                    vec![
                        ("repository".to_string(), Value::Str(name.to_string())),
                        ("id".to_string(), Value::Str(request.id.clone())),
                        ("path".to_string(), Value::Str(dir)),
                    ],
                );
                return;
            }
            Err(e) => {
                self.refuse(
                    k,
                    sup,
                    &format!(
                        "the identifier at {id_path} could not be read ({e}), so this machine \
                         cannot tell whether the directory is the repository that was meant. \
                         NOTHING HAS BEEN DESTROYED, which is the only safe answer to that \
                         question."
                    ),
                );
                return;
            }
        }

        // **The one destructive call in this system**, reached only here.
        //
        // **The rename is the destruction** (`ADR-017` §3): the repository is
        // renamed aside under the repository root, so nothing — git, the
        // repository convergence, an operator — finds it under its name from
        // the moment this reports it, and a forked helper deletes the history
        // afterwards. A rename either happens or does not, so a failure here
        // leaves the repository whole.
        match crate::reclaim::set_aside(k, crate::supervise::SELF_NAME, &dir) {
            Ok(done) => sup.note(
                k,
                Severity::Warn,
                "a repository was DESTROYED, with every commit in it. This is not recoverable \
                 from this machine",
                vec![
                    ("repository".to_string(), Value::Str(name.to_string())),
                    ("id".to_string(), Value::Str(request.id.clone())),
                    ("path".to_string(), Value::Str(dir)),
                ]
                .into_iter()
                .chain(done.unforked.map(|e| {
                    (
                        "deletion".to_string(),
                        Value::Str(format!(
                            "the helper to delete what was set aside could not start ({e}); \
                             the next start deletes it"
                        )),
                    )
                }))
                // `ADR-028` §4.5's attribution, from the ONE place that spells
                // it — `crate::deploy_request::attribution` carries why.
                .chain(crate::deploy_request::attribution(&request.principal))
                .collect::<Vec<_>>(),
            ),
            Err(e) => sup.note(
                k,
                Severity::Error,
                "a repository could not be destroyed, and it is still there whole under its \
                 name — the rename that removes it did not happen",
                vec![
                    ("repository".to_string(), Value::Str(name.to_string())),
                    ("path".to_string(), Value::Str(dir)),
                    ("errno".to_string(), Value::Str(e.to_string())),
                ],
            ),
        }
    }

    /// A request that is there and cannot be honoured. **The file is removed**,
    /// because a request that was refused for a reason will be refused for the
    /// same reason next pass.
    fn refuse(&mut self, k: &mut dyn Kernel, sup: &mut Supervisor, why: &str) {
        let removed = k.remove_file(repo_delete::REQUEST_PATH).is_ok();
        sup.note(
            k,
            Severity::Error,
            "REFUSED the repository-delete request, and NOTHING HAS BEEN DESTROYED",
            vec![
                (
                    "path".to_string(),
                    Value::Str(repo_delete::REQUEST_PATH.to_string()),
                ),
                ("detail".to_string(), Value::Str(why.to_string())),
                // Said out loud, because a refusal that left the file behind
                // repeats every pass and an operator needs to know which.
                ("cleared".to_string(), Value::Bool(removed)),
            ],
        );
        self.trouble_reported = true;
    }

    /// Trouble with the file itself rather than with its contents. Reported
    /// once per episode.
    fn report_trouble(
        &mut self,
        k: &mut dyn Kernel,
        sup: &mut Supervisor,
        what: &str,
        detail: &str,
    ) {
        if self.trouble_reported {
            return;
        }
        self.trouble_reported = true;
        sup.note(
            k,
            Severity::Warn,
            &format!(
                "the repository-delete request at {} {what}",
                repo_delete::REQUEST_PATH
            ),
            vec![("detail".to_string(), Value::Str(detail.to_string()))],
        );
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::journal::Journal;
    use crate::service::Limits;
    use crate::supervise;
    use crate::testkernel::{FakeKernel, Script};

    fn supervisor() -> Supervisor {
        let journal = Journal::new("boot".to_string(), 7, supervise::ACTOR.to_string());
        Supervisor::new(Vec::new(), Limits::default(), journal)
            .unwrap_or_else(|e| unreachable!("{e}"))
    }

    const A: &str = "4f2c9a01bd7e3865";
    const B: &str = "a1b2c3d4e5f60718";
    const DIR: &str = "/state/git/example-repo.git";

    fn document(toml: &str) -> rivet_config::Document {
        rivet_config::Document::parse(toml).expect("the fixture is a valid document")
    }

    /// A machine with the repository on its disk and a confirmed request
    /// waiting for it.
    fn machine(id_on_disk: Option<&str>, request: Option<&str>) -> FakeKernel {
        let mut k = FakeKernel::new();
        if let Some(id) = id_on_disk {
            // The directory AND its identifier: the fake models both, and a
            // test that seeded only the file would prove nothing about the
            // removal - a rename answers ENOENT for a directory it has never
            // been told about, which is what a real one does.
            k.dirs_mut().insert(DIR.to_string(), Vec::new());
            k.dirs_mut().insert(
                git::REPOSITORY_ROOT.to_string(),
                vec!["example-repo.git".to_string()],
            );
            k.files_mut().insert(
                format!("{DIR}/{}", repo::ID_FILE),
                Script::always(Ok(format!("{id}\n"))),
            );
        }
        if let Some(line) = request {
            k.files_mut()
                .insert(repo_delete::REQUEST_PATH.to_string(), Script::always(Ok(line.to_string())));
            k.sizes_mut()
                .insert(repo_delete::REQUEST_PATH.to_string(), line.len() as u64);
        }
        k
    }

    fn asking(name: &str, id: &str) -> String {
        repo_delete::render_request(
            &rivet_core::repo::RepoName::new(name).expect("a name"),
            id,
            &rivet_core::deploy::Principal::no_session(),
        )
    }

    fn run(k: &mut FakeKernel, doc: &rivet_config::Document) -> Vec<String> {
        let mut sup = supervisor();
        Watch::new().poll(k, &mut sup, doc);
        k.records.iter().map(|r| format!("{} {:?}", r.msg, r.fields)).collect()
    }

    /// **The happy path, and it is the one that destroys something.**
    #[test]
    fn an_undeclared_repository_whose_identifier_matches_is_destroyed() {
        let mut k = machine(Some(A), Some(&asking("example-repo", A)));
        let said = run(&mut k, &document("version = 1\n"));

        assert_eq!(k.removed_paths(), [DIR], "{said:?}");
        assert!(said.iter().any(|m| m.contains("DESTROYED")), "{said:?}");
        // The request is gone, so it does not run again next pass.
        assert!(k.removed.iter().any(|p| p == repo_delete::REQUEST_PATH), "{:?}", k.removed);
    }

    /// **Check one.** A still-declared repository is refused with the order to
    /// do it in — the next convergence would recreate it and the delete would
    /// look as though it had failed.
    #[test]
    fn a_still_declared_repository_is_refused_and_not_touched() {
        let mut k = machine(Some(A), Some(&asking("example-repo", A)));
        let said = run(
            &mut k,
            &document(&format!("version = 1\n[repos.example-repo]\nid = \"{A}\"\n")),
        );

        assert!(k.removed_paths().is_empty(), "{:?}", k.removed_paths());
        assert!(said.iter().any(|m| m.contains("STILL DECLARED")), "{said:?}");
        assert!(said.iter().any(|m| m.contains("remove repo")), "{said:?}");
    }

    /// **Check two, and the reason the identifier is typed at all.** An
    /// operator who undeclared `example-repo`, made a new one, and then ran a
    /// command he typed ten minutes ago destroys nothing.
    #[test]
    fn a_directory_carrying_a_different_identifier_is_not_touched() {
        let mut k = machine(Some(B), Some(&asking("example-repo", A)));
        let said = run(&mut k, &document("version = 1\n"));

        assert!(k.removed_paths().is_empty(), "{:?}", k.removed_paths());
        assert!(said.iter().any(|m| m.contains("NOT THE REPOSITORY")), "{said:?}");
        assert!(said.iter().any(|m| m.contains(A) && m.contains(B)), "{said:?}");
    }

    /// A repository that is not there is nothing to do, not a fault: it is the
    /// state after a successful delete and the state of a machine that never
    /// had it.
    #[test]
    fn a_repository_this_machine_does_not_have_is_not_an_error() {
        let mut k = machine(None, Some(&asking("example-repo", A)));
        let said = run(&mut k, &document("version = 1\n"));

        assert!(k.removed_paths().is_empty(), "{:?}", k.removed_paths());
        assert!(said.iter().any(|m| m.contains("nothing to do")), "{said:?}");
    }

    /// An unreadable identifier is refused. **The only safe answer** to *is
    /// this the right repository* when the question cannot be answered.
    #[test]
    fn an_unreadable_identifier_destroys_nothing() {
        let mut k = machine(None, Some(&asking("example-repo", A)));
        k.files_mut().insert(
            format!("{DIR}/{}", repo::ID_FILE),
            Script::always(Err(Errno::EIO)),
        );
        let said = run(&mut k, &document("version = 1\n"));

        assert!(k.removed_paths().is_empty(), "{:?}", k.removed_paths());
        assert!(said.iter().any(|m| m.contains("cannot tell")), "{said:?}");
    }

    /// **A file that is not a request destroys nothing**, and is cleared so it
    /// does not repeat every pass.
    #[test]
    fn a_file_that_is_not_a_request_destroys_nothing() {
        for junk in ["", "hello", "-- ../../system 4f2c9a01bd7e3865\n"] {
            let mut k = machine(Some(A), Some(junk));
            let said = run(&mut k, &document("version = 1\n"));
            assert!(k.removed_paths().is_empty(), "{junk:?}: {:?}", k.removed_paths());
            assert!(said.iter().any(|m| m.contains("REFUSED")), "{junk:?}: {said:?}");
        }
    }

    /// **The request is removed BEFORE the work**, so a crash costs one delete
    /// rather than every convergence pass.
    #[test]
    fn the_request_is_cleared_before_anything_is_destroyed() {
        let mut k = machine(Some(A), Some(&asking("example-repo", A)));
        run(&mut k, &document("version = 1\n"));

        let cleared = k
            .removed
            .iter()
            .position(|p| p == repo_delete::REQUEST_PATH)
            .expect("the request was cleared");
        // Nothing else was removed as a file, and the tree removal is a
        // different call - so the ordering claim is about which call came
        // first, and the fake records both in order.
        assert_eq!(cleared, 0, "{:?}", k.removed);
        assert_eq!(k.removed_paths(), [DIR]);
    }

    /// **A destroyed repository is gone at the rename, and a helper deletes
    /// it** — `ADR-017` §3. When the destruction is reported nothing is left
    /// under its name for git or the repository convergence to find, and the
    /// loop deleted nothing itself, however large the history was.
    #[test]
    fn a_destroyed_repository_is_renamed_aside_and_deleted_by_a_helper() {
        let mut k = machine(Some(A), Some(&asking("example-repo", A)));
        let said = run(&mut k, &document("version = 1\n"));

        assert_eq!(
            k.reclaims,
            [(
                crate::supervise::SELF_NAME.to_string(),
                git::REPOSITORY_ROOT.to_string(),
                rivet_core::service::RECLAIM_MARK.to_string()
            )],
            "{said:?}"
        );
        assert_eq!(k.list_dir(git::REPOSITORY_ROOT), Ok(vec![]));
        assert!(said.iter().any(|m| m.contains("DESTROYED")), "{said:?}");
    }


    /// An ordinary machine with no request says nothing at all.
    #[test]
    fn a_machine_with_no_request_says_nothing() {
        let mut k = machine(Some(A), None);
        let said = run(&mut k, &document("version = 1\n"));
        assert!(said.is_empty(), "{said:?}");
        assert!(k.removed_paths().is_empty());
    }
}
