//! **What the launcher may offer**, asked of the supervisor rather than read
//! off the disk.
//!
//! `PLAN.md` §104.36. The maintainer's rule, ruled 2026-08-31 (.rulings/
//! R-321): an application is *enabled* when it is on the launcher's list of
//! apps, and the launcher can then have one auto-run; an application can be
//! installed and not enabled, in which case the launcher does not offer it.
//!
//! So an application is **offered** when two things are true at once: the
//! document says `enabled`, and its own signed manifest declares
//! `devices = ["display"]`. Neither alone is enough — a daemon that is enabled
//! has no business on a screen list, and a display application that is not
//! enabled is installed and deliberately not offered.
//!
//! **That rule has not changed. What changed is who applies it.**
//!
//! # This module used to read the document, and it said it should not
//!
//! ~~Why this reads files instead of asking the supervisor.~~ **Superseded
//! 2026-09-05, `ADR-042` §8.2.** Until this date the module opened
//! `/state/config/machine.toml`, walked `/state/app`, and read every installed
//! application's `manifest.toml` for itself. Its own header said what that was:
//!
//! > *"`docs/design-candidates/rivet-launcher.md` §7 argues the opposite, and
//! > it is right about the end state: the supervisor holds every definition at
//! > once, it already owns what "installed" means, and a second reader of the
//! > document is exactly the two-authority problem `ADR-011` names. A `get
//! > kiosk` on the control socket is where this belongs. **That verb does not
//! > exist yet, and this is written to be deleted when it does.**"*
//!
//! It also named the cost, correctly: *"this reads the document a second time
//! and can therefore disagree with the supervisor about what is enabled."*
//!
//! # `ADR-042` is what made the verb exist
//!
//! A Landlock-confined application may read what it was handed and nothing
//! else, and neither the operator's document nor another application's
//! installed files is something the launcher was handed. On the first confined
//! boot, on hardware, all three reads were refused:
//!
//! ```text
//! rivet-launcher: WARNING - /state/config/machine.toml could not be read: Permission denied (os error 13)
//! rivet-launcher: 0 application(s) offered:
//! ```
//!
//! — an empty menu, and `[kiosk] auto-run` never honoured, so a wall showed a
//! blank launcher instead of a dashboard.
//!
//! The two ways out were a hole in the confinement for the one application that
//! is also the display arbiter, or the verb this module had been waiting for.
//! `rivet_core::kiosk::Request::Offer` is that verb, and it removes a **second
//! reader** as well as a second grant — so the fix is the one `ADR-011` wanted
//! anyway and Landlock is what forced it to be paid for.
//!
//! # What is left here
//!
//! The translation from what the supervisor said into what the launcher draws,
//! and nothing that touches a file. The rule above is applied on the
//! supervisor's side by `Supervisor::offered_displays`, which reads the same
//! two facts out of the definitions it already holds — including the
//! *launcher is never on its own list* exclusion, which it makes by the
//! `connects = ["rivet-sysd:kiosk"]` declaration rather than by a name.

/// One application the launcher may put on its list.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Offer {
    /// The service name, which is also what `select` names.
    pub name: String,
}

/// The launcher's own service name.
///
/// **Found on the board**: the first run listed `rivet-launcher` among its own
/// choices, because it is enabled and declares a display like everything else
/// it offers. Choosing it would be asking the supervisor to restart the process
/// doing the asking.
///
/// **The supervisor excludes it now** — `Supervisor::offered_displays`, by the
/// kiosk `connects` declaration rather than by a name. This constant is kept as
/// a **belt** on that brace: it costs one comparison, and the failure it guards
/// is a launcher that restarts itself when somebody presses a button, which is
/// not a failure worth trusting one implementation with.
pub const SELF_NAME: &str = "rivet-launcher";

/// **What this machine is for**, as the supervisor answered it.
///
/// The whole of what the launcher used to derive from the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    /// Every offered application, sorted by name.
    ///
    /// **Sorted here, not trusted from the wire.** The order is what a person
    /// navigates, and a menu whose entries moved between boots is one nobody
    /// can learn. The supervisor's order is its definition order, which is the
    /// document's, which is not stable in the way a menu needs.
    pub offered: Vec<Offer>,
    /// `[kiosk] locked` — there is no way back to this menu.
    pub locked: bool,
    /// `[kiosk] auto-run`, where the document names one.
    pub auto_run: Option<String>,
    /// **Why this machine's mode could not be established**, or `None`.
    ///
    /// `Some` exactly when the supervisor answered `unknown`, which means it
    /// was never told what the document says. That is not `headless` and must
    /// not be drawn as one: a machine whose mode is unknown might be locked,
    /// and the launcher says so rather than assuming the safe-looking half.
    pub why_unknown: Option<String>,
}

impl Menu {
    /// **The menu a launcher that could not ask has to work with.**
    ///
    /// Empty, unlocked, no auto-run, and *saying* the mode is unknown. The last
    /// field is the whole reason this is a named constructor rather than
    /// `Default`: an unanswered `offer` and a genuinely headless machine
    /// produce the same three values and mean opposite things, which is the
    /// distinction `kiosk_or_why` used to draw between `Ok(None)` and `Err`.
    #[must_use]
    pub fn unanswered(why: String) -> Self {
        Self {
            offered: Vec::new(),
            locked: false,
            auto_run: None,
            why_unknown: Some(why),
        }
    }
}

/// Turn the supervisor's answer into the menu, or say why there is none.
///
/// # Errors
///
/// The supervisor's own sentence, unedited (`AGENTS.md` §5), when it answered
/// something other than an offer — which is a refusal, or a supervisor too old
/// to know the verb.
pub fn from_answer(answer: &rivet_core::kiosk::Answer) -> Result<Menu, String> {
    match answer {
        rivet_core::kiosk::Answer::Offer {
            mode,
            locked,
            auto_run,
            offered,
        } => {
            let mut list: Vec<Offer> = offered
                .iter()
                .filter(|name| name.as_str() != SELF_NAME)
                .map(|name| Offer { name: name.clone() })
                .collect();
            list.sort();
            Ok(Menu {
                offered: list,
                locked: *locked,
                auto_run: auto_run.clone(),
                why_unknown: if mode == rivet_core::operation::status::MODE_UNKNOWN {
                    Some(
                        "the supervisor answered `unknown`, which is what it says before \
                         anything has told it what the document holds — so this machine may be \
                         locked and this launcher cannot tell"
                            .to_string(),
                    )
                } else {
                    None
                },
            })
        }
        rivet_core::kiosk::Answer::Refused { why } => Err(why.clone()),
        other => Err(format!(
            "the supervisor answered `{}`, which is not an answer to `offer`. A supervisor that \
             does not know the verb is one older than ADR-042 §8.2, and this launcher cannot \
             read the document for itself",
            other.render().trim_end()
        )),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;
    use rivet_core::kiosk::Answer;

    fn offer(mode: &str, locked: bool, auto: Option<&str>, names: &[&str]) -> Answer {
        Answer::Offer {
            mode: mode.to_string(),
            locked,
            auto_run: auto.map(std::string::ToString::to_string),
            offered: names.iter().map(|n| (*n).to_string()).collect(),
        }
    }

    /// The menu is sorted here rather than trusted from the wire, because the
    /// order is what a person navigates.
    #[test]
    fn the_menu_is_sorted_whatever_order_it_arrived_in() {
        let menu = from_answer(&offer(
            "kiosk",
            false,
            None,
            &["kiosk-demo", "aura-renderer", "template-kiosk"],
        ))
        .expect("an offer");
        let names: Vec<&str> = menu.offered.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["aura-renderer", "kiosk-demo", "template-kiosk"]);
    }

    /// The launcher does not offer itself, which the board found before any
    /// test did. The supervisor excludes it too; this is the belt.
    #[test]
    fn the_launcher_is_never_on_its_own_list() {
        let menu = from_answer(&offer("kiosk", false, None, &["a-kiosk", SELF_NAME]))
            .expect("an offer");
        let names: Vec<&str> = menu.offered.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, ["a-kiosk"], "the launcher offered itself");
    }

    /// **`locked` comes from the supervisor's parse and from nowhere else.**
    ///
    /// The launcher had its own reader until 2026-09-05 and `.rulings/` R-137
    /// records what that cost: a document that stopped parsing unlocked a
    /// locked kiosk, because the launcher's `None` was indistinguishable from a
    /// machine with no `[kiosk]` table. There is now one reader, so there is
    /// no second answer to differ.
    #[test]
    fn the_lock_and_the_auto_run_are_the_supervisors_answer() {
        let menu = from_answer(&offer("kiosk", true, Some("aura-renderer"), &["aura-renderer"]))
            .expect("an offer");
        assert!(menu.locked);
        assert_eq!(menu.auto_run.as_deref(), Some("aura-renderer"));
        assert!(menu.why_unknown.is_none());
    }

    /// **`unknown` is not `headless`**, and a launcher that drew it as one
    /// would show a menu on a machine that might be locked.
    #[test]
    fn a_supervisor_that_was_never_told_is_not_a_headless_machine() {
        let unknown = from_answer(&offer("unknown", false, None, &[])).expect("an offer");
        let headless = from_answer(&offer("headless", false, None, &[])).expect("an offer");
        assert!(
            unknown.why_unknown.is_some(),
            "an unknown mode reads as an ordinary unlocked machine, which is the fail-open the \
             launcher used to have in a different place"
        );
        assert!(headless.why_unknown.is_none());
    }

    /// A refusal carries the supervisor's own sentence, and an answer to some
    /// other verb is not silently read as an empty menu.
    #[test]
    fn anything_that_is_not_an_offer_is_an_error_carrying_what_was_said() {
        let refused = from_answer(&Answer::Refused {
            why: "the supervisor said no".to_string(),
        })
        .expect_err("a refusal is not a menu");
        assert_eq!(refused, "the supervisor said no");

        let wrong = from_answer(&Answer::Taken {
            what: "kiosk-demo holds the screen".to_string(),
        })
        .expect_err("an answer to another verb is not a menu");
        assert!(
            wrong.contains("kiosk-demo holds the screen"),
            "the sentence the supervisor actually sent has to reach the operator: {wrong}"
        );
    }

    /// **An unanswered `offer` says so.** The launcher cannot fall back to
    /// reading the document — it is refused it — so the honest empty menu has
    /// to carry the reason with it.
    #[test]
    fn an_unanswered_offer_is_distinguishable_from_an_empty_machine() {
        let could_not_ask = Menu::unanswered("the socket was not there".to_string());
        let genuinely_empty = from_answer(&offer("headless", false, None, &[])).expect("an offer");
        assert_eq!(could_not_ask.offered, genuinely_empty.offered);
        assert!(could_not_ask.why_unknown.is_some());
        assert!(genuinely_empty.why_unknown.is_none());
    }
}
