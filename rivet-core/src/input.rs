//! **Pressing a button, from somewhere that has no buttons** — `ADR-012` §9.
//!
//! `ADR-012` is called *The Agent as a First-Class Operator*, and on 2026-09-01
//! it was found not to be true: an agent built the display-lease mechanism and
//! could not test it, because `select` is the launcher's to send, the launcher
//! sends it when somebody presses a button, and there was no way to produce
//! one.
//!
//! # The vocabulary is the human vocabulary
//!
//! No more and no less. A person at a Rivet kiosk has a directional pad, ENTER,
//! BACK, a long BACK and a screenshot control; that is exactly what is here.
//!
//! **There is deliberately no text entry**, and that is parity rather than a
//! restriction — *no human has one either*, because a Rivet kiosk has no
//! keyboard surface to be equal to. The rule is parity, and parity is the
//! ceiling as well as the floor: the day a person gets text entry, so does an
//! agent, and the argument will already have been made.
//!
//! # Both ends parse from HERE
//!
//! `cmd/rivet-launcher` reads these off a socket and `cmd/rivet` writes them,
//! for `rivet_core::kiosk`'s reason: two spellings of one vocabulary is two
//! things that come to disagree (`ADR-011`).

/// Where the launcher listens for actions.
///
/// **The endpoint name, not a path.** The launcher declares
/// `provides = ["socket:input"]` and the supervisor binds it, so where it lives
/// is Rivet's business and not the application's (`ADR-015` §5.3).
pub const ENDPOINT: &str = "input";

/// The application that provides it.
///
/// A constant rather than a literal because the supervisor has to find the
/// socket by the same name the launcher declared, and a second spelling is the
/// failure `ADR-011` names.
pub const PROVIDER: &str = "rivet-launcher";

/// The most an action line may be. Generous: the longest word here is nine
/// characters and this leaves room for a vocabulary that grows.
pub const ACTION_MAX_BYTES: usize = 64;

/// One thing a person could have pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Up,
    Down,
    Left,
    Right,
    /// Choose what is focused.
    Enter,
    /// A short press of the back control.
    Back,
    /// The back control held — **a different event, not a longer one**. It is
    /// the way out of a running application (`PLAN.md` §104.36.4).
    LongBack,
    /// Take a picture of the screen (`PLAN.md` §104.39).
    Capture,
}

impl Action {
    /// Every action, in the order a person would describe them.
    pub const ALL: [Self; 8] = [
        Self::Up,
        Self::Down,
        Self::Left,
        Self::Right,
        Self::Enter,
        Self::Back,
        Self::LongBack,
        Self::Capture,
    ];

    /// The wire form, and the word an operator types.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::Left => "left",
            Self::Right => "right",
            Self::Enter => "enter",
            Self::Back => "back",
            Self::LongBack => "long-back",
            Self::Capture => "capture",
        }
    }

    /// Read one.
    ///
    /// `None` rather than a default: an action this build does not know is a
    /// caller asking for something, and guessing at `enter` would press a
    /// button nobody asked for.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        let word = word.trim();
        Self::ALL.into_iter().find(|a| a.as_str() == word)
    }

    /// Every word, for a refusal that lists what it would have accepted.
    #[must_use]
    pub fn vocabulary() -> String {
        Self::ALL
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The line sent over the socket, newline included.
    ///
    /// **The close is the frame**, as every other socket in this project has
    /// it, so the newline is for a person reading a capture rather than for the
    /// parser.
    #[must_use]
    pub fn render(self) -> String {
        let mut line = self.as_str().to_string();
        line.push('\n');
        line
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    #[test]
    fn every_action_round_trips() {
        for action in Action::ALL {
            assert_eq!(Action::parse(action.as_str()), Some(action));
            assert_eq!(Action::parse(action.render().trim()), Some(action));
        }
    }

    /// A word this build does not know must not become a button press.
    #[test]
    fn an_unknown_word_is_refused_rather_than_guessed() {
        for word in ["", "ENTER", "press", "type hello", "enter enter"] {
            assert_eq!(Action::parse(word), None, "{word:?} was accepted");
        }
    }

    /// Surrounding space is a transport artefact, not a different action.
    #[test]
    fn surrounding_space_is_ignored() {
        assert_eq!(Action::parse("  enter\n"), Some(Action::Enter));
    }

    /// **`back` and `long-back` are different events**, not one held longer,
    /// and a vocabulary that let them collide would lose the way out of a
    /// running application.
    #[test]
    fn back_and_long_back_are_distinct() {
        assert_ne!(Action::Back, Action::LongBack);
        assert_ne!(Action::Back.as_str(), Action::LongBack.as_str());
        assert_eq!(Action::parse("long-back"), Some(Action::LongBack));
    }

    #[test]
    fn the_vocabulary_lists_every_action() {
        let listed = Action::vocabulary();
        for action in Action::ALL {
            assert!(listed.contains(action.as_str()), "{listed} omits {action:?}");
        }
    }

    /// **No text entry, and this is the guard on it.** `ADR-012` §9.3: the rule
    /// is parity with what a person can press, and a Rivet kiosk has no
    /// keyboard surface. An action that carried a payload would be the first
    /// step to one.
    #[test]
    fn no_action_carries_a_payload() {
        for action in Action::ALL {
            let word = action.as_str();
            assert!(
                !word.contains(char::is_whitespace),
                "{word:?} has a second word, which is the shape a payload would take"
            );
        }
    }
}
