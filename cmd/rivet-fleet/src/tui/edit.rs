//! **Changing something** — the draft a person builds, and the dialog that
//! asks before it leaves this machine.
//!
//! The maintainer described the flow on 2026-08-27: the terminal interface
//! should flow, so that a setting change followed by `<save>` may pop up a
//! confirmation of what is changing from and to, and ask whether to apply it to
//! the device now or later (.rulings/ R-329).
//!
//! # A draft, rather than writing as you type
//!
//! Every change goes into a [`Draft`] and nothing touches the register until
//! `save`. That is what makes the from-and-to confirmation answerable: the
//! dialog can show **from** and **to** because the *from* is still there.
//!
//! It also means `esc` is a real escape. A person who toggles four switches and
//! changes their mind has changed nothing, which is not true of an interface
//! that writes each keystroke through.
//!
//! # Two steps, not one, and `ADR-012` §7.1 is why
//!
//! Saving writes the **register**. Applying tells the **machine**. They are
//! separate because they always have been — `ADR-031` §7.5's *a schema says
//! what may be asked for; an apply says what can be done* — and the dialog is
//! how a declarative system feels immediate without becoming an imperative one.
//! `later` is not a lesser answer: it is the CLI's ordinary behaviour, reached
//! by a key.
//!
//! # No default, ruled
//!
//! The maintainer chose it: **`A` or `L` explicitly, `Esc` cancels.** Nothing is
//! focused, so `enter` does nothing at all. One keystroke of friction on every
//! save, in exchange for never pushing to a board because a person was
//! finishing a different thought.

use std::collections::BTreeMap;

use crate::register::Device;

/// One thing that would change, in the words the dialog shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// What is changing, as a person would name it — not a schema path.
    pub field: String,
    pub from: String,
    pub to: String,
}

impl Change {
    /// The line the dialog shows.
    #[must_use]
    pub fn line(&self) -> String {
        format!("{}:  {} → {}", self.field, self.from, self.to)
    }
}

/// Edits a person has made and not yet saved.
///
/// **Keyed to one device by identifier.** Opening a different device abandons
/// the draft rather than carrying it, because a draft that followed the cursor
/// would apply a change to a machine nobody was looking at when they made it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub device: String,
    /// Services whose switch has been flipped, and to what.
    services: BTreeMap<String, bool>,
}

impl Draft {
    #[must_use]
    pub fn for_device(id: &str) -> Self {
        Self {
            device: id.to_owned(),
            services: BTreeMap::new(),
        }
    }

    /// Whether anything is drafted at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.services.is_empty()
    }

    /// What this draft says a service's switch is, falling back to the record.
    ///
    /// **The screen draws this, not the record**, so a flipped switch looks
    /// flipped immediately — which is the whole of the flow he asked for
    /// (.rulings/ R-329).
    #[must_use]
    pub fn service(&self, device: &Device, name: &str) -> bool {
        self.services
            .get(name)
            .copied()
            .unwrap_or_else(|| device.services.get(name).copied().unwrap_or(false))
    }

    /// Flip a switch.
    ///
    /// **A value flipped back to what the record says stops being a change**,
    /// rather than becoming a change to the same value — otherwise the dialog
    /// would ask about `on → on`, which is a question with no content.
    pub fn toggle_service(&mut self, device: &Device, name: &str) {
        let now = self.service(device, name);
        let recorded = device.services.get(name).copied().unwrap_or(false);
        if now == recorded {
            self.services.insert(name.to_owned(), !now);
        } else {
            self.services.remove(name);
        }
    }

    /// Whether one service differs from the record.
    #[must_use]
    pub fn service_changed(&self, name: &str) -> bool {
        self.services.contains_key(name)
    }

    /// Everything this draft would change, for the dialog.
    ///
    /// **Ordered**, because the dialog is read and a list that reordered itself
    /// between two saves would be two different questions.
    #[must_use]
    pub fn changes(&self, device: &Device) -> Vec<Change> {
        self.services
            .iter()
            .map(|(name, wanted)| Change {
                field: format!("service {name}"),
                from: switch_word(device.services.get(name).copied().unwrap_or(false)),
                to: switch_word(*wanted),
            })
            .collect()
    }

    /// Write this draft into a device record.
    ///
    /// **Takes the record rather than the register**, so the caller does the
    /// saving and this cannot write a file by surprise.
    pub fn apply_to(&self, device: &mut Device) {
        for (name, wanted) in &self.services {
            device.services.insert(name.clone(), *wanted);
        }
    }
}

fn switch_word(on: bool) -> String {
    if on { "on".to_string() } else { "off".to_string() }
}

/// What a person is being asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    pub device: String,
    pub changes: Vec<Change>,
}

/// What they answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Write the register **and** tell the machine.
    ApplyNow,
    /// Write the register only. The machine finds out at the next apply.
    Later,
    /// Change nothing. The draft survives, so a person who hit the wrong key
    /// has not lost their edits.
    Cancel,
    /// A key the dialog does not answer to. **Nothing is focused**, so this is
    /// what `enter` gets — ruled by the maintainer 2026-08-27.
    Nothing,
}

/// Read a keypress as an answer.
///
/// **`enter` is [`Answer::Nothing`] on purpose.** No button is focused, so the
/// key a person presses without reading does not reach a board.
#[must_use]
pub fn answer(key: char) -> Answer {
    match key.to_ascii_lowercase() {
        'a' => Answer::ApplyNow,
        'l' => Answer::Later,
        _ => Answer::Nothing,
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn device() -> Device {
        let mut device = Device {
            id: "an-id".to_string(),
            name: "board".to_string(),
            ..Device::default()
        };
        device.services.insert("rivet-mdnsd".to_string(), true);
        device.services.insert("rivet-timed".to_string(), false);
        device
    }

    /// **A draft changes nothing until it is saved**, which is what makes `esc`
    /// a real escape.
    #[test]
    fn drafting_does_not_touch_the_record() {
        let device = device();
        let mut draft = Draft::for_device(&device.id);
        draft.toggle_service(&device, "rivet-mdnsd");

        assert!(!draft.service(&device, "rivet-mdnsd"), "the draft flipped it");
        assert_eq!(
            device.services.get("rivet-mdnsd"),
            Some(&true),
            "the record was touched"
        );
    }

    /// The screen draws the draft, so a flipped switch looks flipped at once.
    #[test]
    fn the_draft_is_what_the_screen_shows() {
        let device = device();
        let mut draft = Draft::for_device(&device.id);
        assert!(draft.service(&device, "rivet-mdnsd"), "starts from the record");
        draft.toggle_service(&device, "rivet-mdnsd");
        assert!(!draft.service(&device, "rivet-mdnsd"));
    }

    /// **Flipping back is not a change.** Otherwise the dialog asks about
    /// `on → on`, which is a question with no content.
    #[test]
    fn a_switch_flipped_back_stops_being_a_change() {
        let device = device();
        let mut draft = Draft::for_device(&device.id);

        draft.toggle_service(&device, "rivet-mdnsd");
        assert_eq!(draft.changes(&device).len(), 1);

        draft.toggle_service(&device, "rivet-mdnsd");
        assert!(draft.is_empty(), "{:?}", draft.changes(&device));
        assert!(!draft.service_changed("rivet-mdnsd"));
    }

    /// **The dialog can say from and to**, because the record is still intact —
    /// which is the reason for drafting rather than writing through.
    #[test]
    fn a_change_says_what_it_was_and_what_it_would_be() {
        let device = device();
        let mut draft = Draft::for_device(&device.id);
        draft.toggle_service(&device, "rivet-timed");

        let changes = draft.changes(&device);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].from, "off");
        assert_eq!(changes[0].to, "on");
        assert!(changes[0].line().contains("rivet-timed"), "{}", changes[0].line());
        assert!(changes[0].line().contains('→'));
    }

    /// Saving writes what was drafted and nothing else.
    #[test]
    fn saving_writes_exactly_what_was_drafted() {
        let mut device = device();
        let mut draft = Draft::for_device(&device.id);
        draft.toggle_service(&device, "rivet-timed");
        draft.apply_to(&mut device);

        assert_eq!(device.services.get("rivet-timed"), Some(&true));
        // Untouched, because it was not drafted.
        assert_eq!(device.services.get("rivet-mdnsd"), Some(&true));
    }

    /// **`enter` reaches no board.** Ruled 2026-08-27: no default, `A` or `L`
    /// explicitly — so the key a person presses without reading does nothing.
    #[test]
    fn the_dialog_has_no_default_and_enter_is_not_an_answer() {
        assert_eq!(answer('a'), Answer::ApplyNow);
        assert_eq!(answer('A'), Answer::ApplyNow);
        assert_eq!(answer('l'), Answer::Later);
        assert_eq!(answer('L'), Answer::Later);
        // Everything else, including anything a person might mash.
        for key in ['\n', '\r', ' ', 'y', 'n', 'x', '1'] {
            assert_eq!(answer(key), Answer::Nothing, "{key:?} answered the dialog");
        }
    }

    /// A draft belongs to one device, so a change cannot be applied to a
    /// machine nobody was looking at when they made it.
    #[test]
    fn a_draft_names_the_device_it_is_about() {
        let draft = Draft::for_device("4f2c9a01bd7e3865");
        assert_eq!(draft.device, "4f2c9a01bd7e3865");
    }
}
