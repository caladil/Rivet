//! **What is on the wall right now**, and when each of it stops being true.
//!
//! Four slots, at most one card each. Everything that reaches this type came
//! through [`aura_card::Message`] — the four sockets and the renderer's own
//! clock alike — so nothing downstream can tell where a card came from, which
//! is `SPEC-04` §4.2's whole point.

use std::time::Instant;

use aura_card::{Card, Message, Slot};

/// A card and the moment it stops being true.
#[derive(Debug, Clone)]
struct Held {
    card: Card,
    expires_at: Instant,
}

/// The four slots.
#[derive(Debug, Default)]
pub struct Store {
    /// Indexed by [`Slot`]'s position in [`Slot::ALL`].
    slots: [Option<Held>; 4],
    /// Set whenever anything visible changed, cleared by the drawing loop.
    ///
    /// **Redraw on change**, which is Aura's existing behaviour — the C++
    /// renderer's `needs_redraw`. It is not the whole story: an animated
    /// background forces a redraw every frame regardless (`ss_animated` in
    /// `cpp/src/main.cpp`), and the background is drawn *behind* the cards
    /// during ordinary use rather than instead of them. So this flag only buys
    /// a still screen when the background is a solid colour — which is exactly
    /// what it buys in the C++ renderer, and is worth having for that case.
    dirty: bool,
}

fn index(slot: Slot) -> usize {
    match slot {
        Slot::Primary => 0,
        Slot::Secondary => 1,
        Slot::Tertiary => 2,
        Slot::Quaternary => 3,
    }
}

impl Store {
    /// Apply a message that arrived on `granted`'s socket.
    ///
    /// # Why the socket decides the slot
    ///
    /// A producer names its slot on the wire *and* was granted exactly one
    /// endpoint. If those disagree the card is refused: a producer that could
    /// name any slot could paint over a neighbour it was never granted, and the
    /// socket is the half of that pair the platform vouches for.
    ///
    /// Returns `Err` with a sentence for the journal.
    pub fn apply(&mut self, granted: Slot, message: &Message, now: Instant) -> Result<(), String> {
        if message.slot() != granted {
            return Err(format!(
                "a producer granted `{}` sent a card for `{}`; the socket decides the slot, \
                 so this is refused rather than drawn somewhere it was not invited",
                granted.as_str(),
                message.slot().as_str()
            ));
        }
        match message {
            Message::Card(card) => {
                self.slots[index(granted)] = Some(Held {
                    card: card.clone(),
                    expires_at: now + std::time::Duration::from_secs(u64::from(card.ttl_seconds)),
                });
            }
            Message::Withdraw { .. } => self.slots[index(granted)] = None,
        }
        self.dirty = true;
        Ok(())
    }

    /// Drop every card that has stopped being true.
    ///
    /// Returns the slots that were cleared, so the caller can say so once in
    /// the journal — an expiry is a producer that went quiet, and that is worth
    /// a line rather than a silently emptying screen.
    pub fn expire(&mut self, now: Instant) -> Vec<Slot> {
        let mut gone = Vec::new();
        for slot in Slot::ALL {
            let at = index(slot);
            if self.slots[at]
                .as_ref()
                .is_some_and(|held| now >= held.expires_at)
            {
                self.slots[at] = None;
                self.dirty = true;
                gone.push(slot);
            }
        }
        gone
    }

    /// What is in a slot.
    #[must_use]
    pub fn get(&self, slot: Slot) -> Option<&Card> {
        self.slots[index(slot)].as_ref().map(|held| &held.card)
    }

    /// Whether anything changed since the last [`Store::clean`].
    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Mark the screen as drawn.
    pub const fn clean(&mut self) {
        self.dirty = false;
    }

    /// Force a redraw although nothing in here changed.
    ///
    /// The one case the flag cannot see for itself: the burn-in rotation moves
    /// every card without altering any of them. A method rather than a caller
    /// poking the field, so that the set of things that can dirty the screen
    /// stays enumerable in one file.
    #[allow(dead_code)]
    pub const fn touch(&mut self) {
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;
    use aura_card::{Block, Reading};
    use std::time::Duration;

    fn card(slot: Slot, ttl: u32) -> Message {
        Message::Card(Card {
            slot,
            ttl_seconds: ttl,
            blocks: vec![Block::Reading(Reading {
                value: Some("1".to_string()),
                ..Reading::default()
            })],
        })
    }

    /// **The answer to "what happens when an addon goes silent".** Nothing
    /// arrives, the TTL runs out, the slot empties — and the emptying is
    /// reported, because a card that vanished is a producer that stopped.
    #[test]
    fn a_card_that_is_no_longer_true_is_withdrawn_and_the_slot_is_named() {
        let mut store = Store::default();
        let start = Instant::now();
        store
            .apply(Slot::Tertiary, &card(Slot::Tertiary, 60), start)
            .expect("granted");
        assert!(store.get(Slot::Tertiary).is_some());
        assert!(store.expire(start + Duration::from_secs(59)).is_empty());
        assert_eq!(
            store.expire(start + Duration::from_secs(60)),
            vec![Slot::Tertiary]
        );
        assert!(store.get(Slot::Tertiary).is_none());
    }

    /// A producer cannot paint over a neighbour by naming its slot.
    #[test]
    fn a_card_for_a_slot_the_producer_was_not_granted_is_refused() {
        let mut store = Store::default();
        let why = store
            .apply(Slot::Tertiary, &card(Slot::Primary, 60), Instant::now())
            .expect_err("the socket decides");
        assert!(why.contains("tertiary") && why.contains("primary"), "{why}");
        assert!(store.get(Slot::Primary).is_none());
        assert!(store.get(Slot::Tertiary).is_none());
    }

    #[test]
    fn a_withdrawal_empties_the_slot_before_the_ttl_does() {
        let mut store = Store::default();
        let now = Instant::now();
        store
            .apply(Slot::Secondary, &card(Slot::Secondary, 9999), now)
            .expect("granted");
        store
            .apply(
                Slot::Secondary,
                &Message::Withdraw {
                    slot: Slot::Secondary,
                },
                now,
            )
            .expect("granted");
        assert!(store.get(Slot::Secondary).is_none());
    }

    /// The flag exists to keep a still screen still. It must be set by every
    /// path that changes what is drawn, and by nothing else.
    #[test]
    fn every_change_marks_the_screen_dirty_and_nothing_else_does() {
        let mut store = Store::default();
        let now = Instant::now();
        assert!(!store.is_dirty());
        store
            .apply(Slot::Primary, &card(Slot::Primary, 1), now)
            .expect("granted");
        assert!(store.is_dirty());
        store.clean();
        assert!(store.expire(now).is_empty());
        assert!(!store.is_dirty(), "an expiry that did not happen is not a change");
        store.expire(now + Duration::from_secs(2));
        assert!(store.is_dirty(), "an expiry IS a change");
    }
}
