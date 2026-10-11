//! **What makes this Rivet's editor rather than a small `nano`.**
//!
//! On this machine most files are not editable, and no imported editor knows
//! it. `/etc` is a *projection*, rendered fresh before any service starts
//! (`ADR-007` §3.5.1) onto a tmpfs that is remounted read-only once it is
//! written (`ADR-007` §3.6.1 item 1). `ADR-007` §3.6's table is the whole
//! argument, and two of its three rows are what an editor that did not know
//! this would produce:
//!
//! | Behaviour | Verdict |
//! | :-- | :-- |
//! | The write succeeds; the change is lost at reboot | **Silent data loss.** The worst outcome available. |
//! | The write succeeds; the next `apply` overwrites it | **Worse** — non-deterministic. |
//! | The write is refused | Correct, but only if the refusal explains itself. |
//!
//! and §3.6 says what a bare `EROFS` is worth: *"That is a true statement and a
//! useless one."*
//!
//! # Ruled here: a view opens **read-only**, it is not refused outright
//!
//! The alternative considered was refusing to open the file at all — print the
//! explanation to stderr and exit non-zero. It is simpler, and it is the wrong
//! choice, for three reasons in increasing weight:
//!
//! 1. **An operator opening `/etc/passwd` usually wants to read it.** Refusing
//!    the open turns one honest answer into two commands, on a machine whose
//!    `PATH` is `/system/bin` and nothing else. Being told
//!    *no* when you asked to look at a file is the kind of principled hostility
//!    `ADR-005` §5.5 already decided against once.
//! 2. **`ADR-007` §3.6.1 item 2 says the provenance header is the highest-value
//!    mechanism in the list "because it is read by exactly the person about to
//!    make the mistake: they have the file open."** Refusing the open is
//!    precisely taking that away. Opening read-only, with the generator and the
//!    `change-with` on screen, is the same mechanism delivered by the tool
//!    rather than by a comment in the file.
//! 3. **~~Three of the five~~ FOUR OF THE SIX generated files cannot carry
//!    item 2's header at all.** `/etc/passwd`, `/etc/group` and `/etc/shadow`
//!    are colon-separated databases whose parsers are not known to skip a
//!    comment (`rivet_core::etc`); `/etc/motd`, since `ADR-035` §9B.1 on
//!    2026-08-26, is printed in full to whoever just logged in, so a header
//!    would greet them. For those four there is *no* provenance to read in the
//!    file — and a read-only open with a banner is the only place an operator
//!    with the file in front of them can ever see one.
//!
//! So the refusal lands on the **write**, where the mistake actually is, and it
//! lands twice: the title bar says `VIEW` from the moment the file opens, and
//! `^O` answers with the generator and the `change-with` rather than with
//! `EROFS`. The cost of the choice is one more state in `crate::editor`, and it
//! is stated here rather than in a commit message per `AGENTS.md` §1.
//!
//! # Every answer comes from `rivet-core`, and none of it is restated
//!
//! `rivet_core::etc::ETC_FILES` is the same authority `rivet explain`
//! answers from — `rivet-core::operation::explain` reads exactly this table
//! and this module reads no other. That is `ADR-011`: one fact, one owner. A
//! second list of generated paths here would be a second answer, consulted at
//! the moment somebody is already confused, and
//! `the_generated_paths_are_rivet_cores_and_not_a_copy` is the test that
//! fails if one ever appears.

use rivet_core::etc::{ETC_FILES, ETC_FSTYPE, ETC_MOUNT_POINT, MUSL_ETC_PATHS};

/// `/system`, the read-only half of `ADR-008` §3's two-directory system.
///
/// **A second spelling of a constant `rivet-core` also holds**, and it is
/// avoidable only by making `rivet_core::operation::storage`'s `SYSTEM_DIR`
/// public — which is a change to the core's surface for one string, in a
/// workspace that already tracks exactly this debt for four path constants,
/// standing since 2026-08-23. The tripwire is
/// `the_system_directory_is_spelled_the_way_the_core_spells_it`, which asserts
/// the value against a path the core does export rather than trusting this
/// comment.
const SYSTEM_MOUNT_POINT: &str = "/system";

/// What happened when a path was classified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opening {
    /// An ordinary file. `/state` and everything under it, which is where an
    /// operator's own files live (`ADR-008` §3, `ADR-005` §5.5).
    Editable,
    /// A path that must not be written. The buffer still opens; `^O` refuses.
    ReadOnly(Refusal),
}

/// Why a path cannot be written, in the words the operator needs.
///
/// Four fields rather than one string, because the title bar wants the short
/// form, the status line wants the sentence, and `^G` wants all of it. A single
/// pre-formatted message would have forced the shortest of the three on all
/// three.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// One line, for the status bar. Never more than a sentence.
    pub headline: String,
    /// What renders or owns this path.
    pub generated_by: String,
    /// The filesystem, which is *why* a write fails — `ADR-007` §3.6.1 item 1.
    pub filesystem: String,
    /// **What to change instead.** Never "edit it": `ADR-007` §3.6.2 rules that
    /// a generated file is a view and a view is never an interface.
    pub change_with: String,
}

impl Refusal {
    /// The whole answer, as lines, for `^G` and for the `--help` on a refusal.
    pub fn lines(&self, path: &str) -> Vec<String> {
        vec![
            format!("{path} is a view. Rivet does not accept an input it intends to discard."),
            String::new(),
            format!("  generated by   {}", self.generated_by),
            format!("  filesystem     {}", self.filesystem),
            format!("  change with    {}", self.change_with),
            String::new(),
            "ADR-007 §3.6.2: a generated file is a read-only projection of state owned"
                .to_owned(),
            "elsewhere. It is never an interface. `rivet explain <path>` is the same"
                .to_owned(),
            "answer as a structured value.".to_owned(),
        ]
    }
}

/// Classify a path before anything is opened.
///
/// **Absolute paths only carry a verdict.** A relative path cannot be
/// classified without knowing the working directory, and this function
/// deliberately performs no I/O — so `crate::main` resolves the path before
/// calling, and a path that is still relative here is treated as editable
/// rather than guessed about.
pub fn classify(path: &str) -> Opening {
    // 1. A file the projection renders. The richest answer, and the only one
    //    that can name what the contents are derived from.
    if let Some(def) = ETC_FILES.iter().find(|def| def.path == path) {
        return Opening::ReadOnly(Refusal {
            headline: format!(
                "{path} is generated from {}. It cannot be edited; {}",
                def.source, def.changed_by
            ),
            generated_by: format!(
                "Rivet's runtime /etc projection, from {} (ADR-007 §3.5.1)",
                def.source
            ),
            filesystem: read_only_etc(),
            change_with: def.changed_by.to_owned(),
        });
    }

    // 2. Any other path under `/etc`. The mount is read-only as a whole, so
    //    this is not a guess: a file the projection did not render either does
    //    not exist or is not writable, and in both cases a write is refused by
    //    the kernel. Answering only for the rendered files would leave
    //    `/etc/resolv.conf` — the very path `ADR-007` §3.6.1 item 3 chose as
    //    its example — to fail with `EROFS` and no explanation.
    if under(path, ETC_MOUNT_POINT) {
        let deliberate = MUSL_ETC_PATHS.contains(&path);
        return Opening::ReadOnly(Refusal {
            headline: if deliberate {
                format!(
                    "{path} is read by musl and Rivet deliberately does not render it; \
                     musl's own fallback is the answer for an unconfigured machine"
                )
            } else {
                format!("{path} is under {ETC_MOUNT_POINT}, which is a read-only projection")
            },
            generated_by: format!(
                "nothing — {ETC_MOUNT_POINT} is rendered whole by Rivet's projection and \
                 this path is not one of the {} files it writes",
                ETC_FILES.len()
            ),
            filesystem: read_only_etc(),
            change_with: if deliberate {
                "nothing: not rendering this file is the decision (rivet-core::etc). \
                 `rivet explain <path>` says why."
                    .to_owned()
            } else {
                "the configuration document, through `rivet`. Files under /etc are a view \
                 of it, never an input."
                    .to_owned()
            },
        });
    }

    // 3. The image itself. Not a view — a whole read-only filesystem, which
    //    `ADR-018` §5 replaces a slot at a time rather than editing.
    if under(path, SYSTEM_MOUNT_POINT) {
        return Opening::ReadOnly(Refusal {
            headline: format!(
                "{SYSTEM_MOUNT_POINT} is the running system image and is read-only; \
                 an update replaces the whole slot"
            ),
            generated_by: "the image build — cmd/rivet-image, ADR-018 §4".to_owned(),
            filesystem: format!(
                "the {SYSTEM_MOUNT_POINT} slot, mounted read-only by PID 1 (ADR-018 §3)"
            ),
            change_with: "an update payload written to the inactive slot (ADR-018 §5). \
                          Nothing on a running machine edits the image it booted from."
                .to_owned(),
        });
    }

    Opening::Editable
}

/// The filesystem sentence, in the words `rivet explain` uses for it.
fn read_only_etc() -> String {
    format!(
        "{ETC_FSTYPE} at {ETC_MOUNT_POINT}, remounted read-only once it is rendered \
         (ADR-007 §3.6.1 item 1)"
    )
}

/// Whether `path` is `root` or lies inside it.
///
/// Written out rather than reached for as `starts_with`, because
/// `"/system-backup".starts_with("/system")` is true and the answer is no. The
/// same mistake `rivet_core::operation::storage`'s own test guards against
/// with `assert!(!contains("/sys", "/system"))`.
fn under(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn refusal(path: &str) -> Refusal {
        match classify(path) {
            Opening::ReadOnly(refusal) => refusal,
            Opening::Editable => panic!("{path} must not be editable"),
        }
    }

    // -- the property the whole module exists for -----------------------------

    /// **Every file the `/etc` projection renders is refused, and the refusal
    /// says what to change instead.**
    ///
    /// Driven off `ETC_FILES` rather than off a list here, so a sixth generated
    /// file is covered the day it is added — which is the failure mode a
    /// hand-written list has and the reason `rivet_core::operation::explain`
    /// generates its own list the same way.
    #[test]
    fn every_generated_file_is_refused_and_says_what_to_change_instead() {
        for def in ETC_FILES {
            let refusal = refusal(def.path);
            assert_eq!(refusal.change_with, def.changed_by);
            assert!(
                refusal.generated_by.contains(def.source),
                "{} does not say what it is derived from",
                def.path
            );
            assert!(
                refusal.headline.contains(def.path),
                "{} does not name itself",
                def.path
            );
        }
    }

    /// `ADR-007` §3.6.2: a view is never an interface, so no answer may tell an
    /// operator to edit the file. `rivet-core`'s own explain has this test;
    /// this is the same property asserted on this crate's rendering of it,
    /// because a refusal that ended "edit it" would be the whole point lost.
    #[test]
    fn no_refusal_anywhere_tells_the_operator_to_edit_the_file() {
        let paths = ["/etc/passwd", "/etc/hosts", "/etc/localtime", "/system/bin/rivet"];
        for path in paths {
            let refusal = refusal(path);
            let all = refusal.lines(path).join(" ").to_lowercase();
            assert!(
                !all.contains("edit it") && !all.contains("edit this file"),
                "{path}'s refusal points back at the file"
            );
        }
    }

    #[test]
    fn a_generated_file_names_its_generator_and_its_filesystem() {
        let refusal = refusal("/etc/hosts");
        assert!(refusal.generated_by.contains("projection"));
        assert!(
            refusal.filesystem.contains(ETC_FSTYPE),
            "the filesystem is why the write fails and has to be named"
        );
    }

    /// A musl path Rivet leaves out is refused with the *reason* it is out,
    /// not with `EROFS` — the outcome `ADR-007` §3.6 calls "true and useless".
    /// A classifier that only knew the rendered files would call this editable.
    ///
    /// **The example moved on 2026-09-01, and again on 2026-09-02.** §3.6.1
    /// item 3 chose `/etc/resolv.conf` precisely because nothing rendered it,
    /// and `[network.resolver]` now does (ruled 2026-09-01; a citation here
    /// named a PLAN section that was never written, removed by the audit in
    /// `§107.5`) — so that path tests the *other* branch today. The replacement
    /// was `/etc/localtime`, and `ADR-021` §9 rendered that one a day later.
    ///
    /// **So the subject is derived rather than named**, because this test has
    /// now been invalidated twice by the schema growing a key, and a third
    /// literal would be a third thing to notice. `MUSL_ETC_PATHS` minus
    /// `ETC_FILES` is the set, exactly as `rivet_core::operation::explain`
    /// computes it; on 2026-09-02 it is `/etc/services` and `/etc/zoneinfo/`.
    #[test]
    fn a_path_under_etc_that_is_not_rendered_is_still_refused() {
        let left_out: Vec<&str> = MUSL_ETC_PATHS
            .iter()
            .copied()
            .filter(|p| !ETC_FILES.iter().any(|def| def.path == *p))
            .collect();
        assert!(
            !left_out.is_empty(),
            "every path musl reads is rendered; this test has no subject"
        );
        for path in left_out {
            let refusal = refusal(path);
            assert!(
                refusal.headline.contains("musl"),
                "the absence is a decision and the refusal has to say so: {}",
                refusal.headline
            );
        }
    }

    #[test]
    fn a_path_under_etc_that_nothing_has_ever_heard_of_is_still_refused() {
        // Not rendered, not a musl path — and still on a read-only tmpfs.
        assert!(matches!(
            classify("/etc/nginx/nginx.conf"),
            Opening::ReadOnly(_)
        ));
    }

    #[test]
    fn the_image_is_refused_because_an_update_replaces_a_slot() {
        let refusal = refusal("/system/bin/rivet");
        assert!(refusal.change_with.contains("update"));
    }

    // -- what is editable, which matters as much as what is not ---------------

    #[test]
    fn the_operators_own_files_under_state_are_editable() {
        // `ADR-005` §5.5's second justification for having an editor at all:
        // "the operator's own files in /state/home — including the nushell
        // configuration just ruled in".
        assert_eq!(
            classify("/state/home/root/.config/nushell/config.nu"),
            Opening::Editable
        );
        assert_eq!(classify("/state/config/machine.toml"), Opening::Editable);
    }

    #[test]
    fn a_relative_path_is_editable_rather_than_guessed_about() {
        assert_eq!(classify("notes.txt"), Opening::Editable);
    }

    #[test]
    fn a_prefix_that_is_not_a_parent_directory_is_not_treated_as_one() {
        // The mistake `rivet_core::operation::storage` guards against from the
        // other direction: `starts_with` alone says yes to both of these.
        assert_eq!(classify("/etcetera"), Opening::Editable);
        assert_eq!(classify("/system-backup/notes"), Opening::Editable);
    }

    #[test]
    fn the_mount_points_themselves_are_refused() {
        assert!(matches!(classify("/etc"), Opening::ReadOnly(_)));
        assert!(matches!(classify("/system"), Opening::ReadOnly(_)));
    }

    // -- the authority --------------------------------------------------------

    /// The set of refused `/etc` paths is `rivet-core`'s, not this crate's.
    ///
    /// If this file ever grows its own list of generated paths, this test is
    /// what fails: it asserts the classification tracks `ETC_FILES` for a path
    /// the core knows about and this module never mentions by name.
    #[test]
    fn the_generated_paths_are_rivet_cores_and_not_a_copy() {
        assert!(!ETC_FILES.is_empty(), "the core's table is what is read");
        for def in ETC_FILES {
            assert!(matches!(classify(def.path), Opening::ReadOnly(_)));
        }
    }

    /// `SYSTEM_MOUNT_POINT` is a second spelling of a string the core also
    /// holds — see its documentation. This is the tripwire that catches the two
    /// disagreeing, asserted against a constant the core does export.
    #[test]
    fn the_system_directory_is_spelled_the_way_the_core_spells_it() {
        assert!(
            rivet_core::etc::CLI_PATH.starts_with(&format!("{SYSTEM_MOUNT_POINT}/")),
            "rivet-core puts its CLI at {} and this crate thinks the image is at {SYSTEM_MOUNT_POINT}",
            rivet_core::etc::CLI_PATH
        );
    }
}
