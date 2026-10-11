//! **The six things a launcher needs, from a keyboard OR a gamepad.**
//!
//! `PLAN.md` §104.18, ruled by the maintainer before a controller test. This is
//! deliberately not a general input library: it is the exact vocabulary named
//! for a kiosk launcher — up, down, left, right, enter, back — and decoding
//! precisely that turns a controller session from *"the descriptor delivered 48
//! bytes"* into *"the D-pad works and so does A"*.
//!
//! # Where this came from, and why it is here rather than shared
//!
//! `examples/probes/kiosk-demo` wrote and proved this vocabulary on hardware, in its
//! own separate workspace. It is **ported** here rather than factored into
//! `rivet-core`, and that is a decision with a cost: there are two copies.
//!
//! The reason is that `examples/` is where an application author looks to see
//! what a kiosk application does, and this crate is the platform's own consumer
//! of the same rules. Putting it in `rivet-core` would be the right answer the
//! day a THIRD consumer appears, and `docs/design-candidates/rivet-launcher.md`
//! §9 records that this is a known duplication and not an oversight. The tests
//! below are the same tests, so the two copies cannot drift silently — one of
//! them fails.
//!
//! # Nothing here opens a device
//!
//! Every function is a pure decode over bytes the caller already has. That is
//! what makes it testable on a host with no `/dev/input` at all, which is where
//! it is tested.

/// One of the six actions a launcher understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Up,
    Down,
    Left,
    Right,
    Enter,
    /// A short press of the back control, decided **on release**.
    Back,
    /// **Take a picture of the screen** — `PLAN.md` §104.39.
    ///
    /// Not navigation, and deliberately not on a button any menu uses: START on
    /// a pad and PrtScr on a keyboard, which are the two controls a person
    /// already reaches for when they want to capture something.
    ///
    /// It is in this vocabulary rather than handled beside it because every
    /// other control the launcher reads is here, and a second place to look for
    /// "what does this button do" is how the two come to disagree.
    Capture,

    /// The back control held past [`HOLD`] — a DIFFERENT event, not a longer
    /// one, and the whole reason [`Action::Back`] waits for the release.
    ///
    /// The maintainer named the hazard before it could be got wrong: the test
    /// bed has to show whether the red button was pressed or held, because the
    /// two must be detected as different events (.rulings/ R-334).
    BackHeld,
}

impl Action {
    /// What a person reads. Kept because `PLAN.md` §104.18 makes *two people
    /// agreeing about what happened* the point — one reading a journal, one
    /// holding a pad in another room.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Up => "UP",
            Self::Down => "DOWN",
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
            Self::Enter => "ENTER",
            Self::Capture => "CAPTURE",
            Self::Back => "BACK",
            Self::BackHeld => "LONG BACK",
        }
    }
}

/// `linux/input-event-codes.h`, the handful this needs.
pub const EV_KEY: u16 = 0x01;
/// `linux/input-event-codes.h`, the handful this needs.
pub const EV_ABS: u16 = 0x03;

/// How long the back control must be held to be a different event.
pub const HOLD: std::time::Duration = std::time::Duration::from_secs(2);

/// One evdev record is 24 bytes on a 64-bit kernel: a 16-byte `timeval`, then
/// `type`, `code` and `value`.
pub const EVENT_SIZE: usize = 24;

/// Decode one evdev event, or `None` if it is not one of the six.
///
/// **A press and not a release.** `value` is 1 for a press and 2 for autorepeat
/// on a key, 0 for release; a menu that moved on both would move twice per
/// button. The hat axes are the exception and are handled on their own terms:
/// they report a DIRECTION rather than a state, and returning to centre is `0`,
/// which is the release.
#[must_use]
pub fn action_of(kind: u16, code: u16, value: i32) -> Option<Action> {
    match kind {
        EV_KEY if value == 1 || value == 2 => match code {
            // Keyboard.
            103 => Some(Action::Up),       // KEY_UP
            108 => Some(Action::Down),     // KEY_DOWN
            105 => Some(Action::Left),     // KEY_LEFT
            106 => Some(Action::Right),    // KEY_RIGHT
            28 | 96 => Some(Action::Enter), // KEY_ENTER, KEY_KPENTER
            // **BACK is deliberately absent**, and it is the only one that is.
            // A short press and a long one have to be DIFFERENT events, so
            // neither can be decided when the control goes down — see
            // [`back_edge`] and [`HOLD`].
            //
            // Gamepad buttons. BTN_SOUTH is `A` on an Xbox pad and the bottom
            // face button everywhere else, which is the one every console UI
            // uses to confirm.
            0x130 => Some(Action::Enter), // BTN_SOUTH / BTN_A
            // START on a pad, PrtScr on a keyboard — `PLAN.md` §104.39.
            0x13b => Some(Action::Capture), // BTN_START
            99 => Some(Action::Capture),    // KEY_SYSRQ
            // BTN_EAST / BTN_B is BACK and is handled on its edges.
            // Pads that expose the D-pad as buttons rather than as a hat.
            0x220 => Some(Action::Up),    // BTN_DPAD_UP
            0x221 => Some(Action::Down),  // BTN_DPAD_DOWN
            0x222 => Some(Action::Left),  // BTN_DPAD_LEFT
            0x223 => Some(Action::Right), // BTN_DPAD_RIGHT
            _ => None,
        },
        // The D-pad as a hat, which is what an Xbox pad sends through `xpad`.
        EV_ABS => match (code, value) {
            (0x11, v) if v < 0 => Some(Action::Up), // ABS_HAT0Y
            (0x11, v) if v > 0 => Some(Action::Down),
            (0x10, v) if v < 0 => Some(Action::Left), // ABS_HAT0X
            (0x10, v) if v > 0 => Some(Action::Right),
            _ => None,
        },
        _ => None,
    }
}

/// **Is this the BACK control, and is it going down or coming up?**
///
/// [`action_of`] deliberately ignores releases, because a menu that moved on
/// both edges would move twice per button. The hold-to-exit gesture needs both
/// edges and nothing else does, so it asks separately rather than widening that.
///
/// `Some(true)` is pressed, `Some(false)` is released, `None` is *not the back
/// control, or an autorepeat that says nothing new about the edge*.
///
/// The codes are `KEY_ESC` (1), `KEY_BACKSPACE` (14) and `BTN_EAST` (0x131).
#[must_use]
pub fn back_edge(kind: u16, code: u16, value: i32) -> Option<bool> {
    if kind != EV_KEY || !(code == 1 || code == 14 || code == 0x131) {
        return None;
    }
    match value {
        1 => Some(true),
        0 => Some(false),
        _ => None,
    }
}

/// Every whole record in a buffer, as `(type, code, value)`.
///
/// **Read from the buffer rather than transmuted.** A `#[repr(C)]` struct and a
/// pointer cast would be the same thing with an alignment assumption attached,
/// and this crate denies `unsafe_code`.
///
/// A partial record at the end of a read is **dropped rather than misread** —
/// `chunks_exact` is what guarantees it.
pub fn events_in(buffer: &[u8]) -> impl Iterator<Item = (u16, u16, i32)> + '_ {
    buffer.as_chunks::<EVENT_SIZE>().0.iter().map(|e| {
        (
            u16::from_ne_bytes([e[16], e[17]]),
            u16::from_ne_bytes([e[18], e[19]]),
            i32::from_ne_bytes([e[20], e[21], e[22], e[23]]),
        )
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **The launcher's whole vocabulary, from a keyboard.** These are the six
    /// the maintainer named, and the test exists so that a controller session on
    /// a board is a CONFIRMATION rather than the first time anybody found out
    /// what the mapping was.
    #[test]
    fn a_keyboard_produces_every_launcher_action() {
        let press = |code| action_of(EV_KEY, code, 1);
        assert_eq!(press(103), Some(Action::Up));
        assert_eq!(press(108), Some(Action::Down));
        assert_eq!(press(105), Some(Action::Left));
        assert_eq!(press(106), Some(Action::Right));
        assert_eq!(press(28), Some(Action::Enter));
    }

    /// **BACK is not in `action_of`, and that is the design.** A short press and
    /// a long one must be different events, so neither can be decided when the
    /// control goes down.
    #[test]
    fn the_back_control_produces_no_action_when_it_goes_down() {
        for code in [1u16, 14, 0x131] {
            assert_eq!(
                action_of(EV_KEY, code, 1),
                None,
                "{code:#x} decided BACK on the press"
            );
        }
        assert_eq!(back_edge(EV_KEY, 1, 1), Some(true), "keyboard ESC down");
        assert_eq!(back_edge(EV_KEY, 0x131, 1), Some(true), "pad B down");
        assert_eq!(back_edge(EV_KEY, 0x131, 0), Some(false), "pad B up");
        assert_eq!(
            back_edge(EV_KEY, 0x131, 2),
            None,
            "autorepeat is not an edge"
        );
        assert_eq!(back_edge(EV_KEY, 103, 1), None, "UP is not the back control");
    }

    /// The two are named differently, because a person reading a screen across a
    /// room is the whole point of the distinction.
    #[test]
    fn a_short_and_a_long_back_are_different_words() {
        assert_ne!(Action::Back.word(), Action::BackHeld.word());
        assert_eq!(Action::BackHeld.word(), "LONG BACK");
    }

    /// **And from a gamepad, through both shapes a D-pad takes.** An Xbox pad on
    /// `xpad` sends the D-pad as a hat AXIS; other pads send real buttons. A
    /// launcher cannot care which is plugged in.
    #[test]
    fn a_gamepad_produces_every_launcher_action_through_either_dpad_shape() {
        assert_eq!(action_of(EV_ABS, 0x11, -1), Some(Action::Up));
        assert_eq!(action_of(EV_ABS, 0x11, 1), Some(Action::Down));
        assert_eq!(action_of(EV_ABS, 0x10, -1), Some(Action::Left));
        assert_eq!(action_of(EV_ABS, 0x10, 1), Some(Action::Right));
        assert_eq!(action_of(EV_KEY, 0x220, 1), Some(Action::Up));
        assert_eq!(action_of(EV_KEY, 0x223, 1), Some(Action::Right));
        assert_eq!(action_of(EV_KEY, 0x130, 1), Some(Action::Enter));
    }

    /// **A release is not a press**, or a menu moves twice for every button.
    /// Autorepeat (`2`) IS a press, so a held direction keeps scrolling.
    #[test]
    fn a_key_release_produces_nothing_and_autorepeat_produces_a_press() {
        assert_eq!(action_of(EV_KEY, 103, 0), None, "release");
        assert_eq!(action_of(EV_KEY, 103, 2), Some(Action::Up), "autorepeat");
    }

    /// **A hat returning to centre is nothing**, which is the same rule stated on
    /// an axis: `0` is the release.
    #[test]
    fn a_hat_returning_to_centre_produces_nothing() {
        assert_eq!(action_of(EV_ABS, 0x10, 0), None);
        assert_eq!(action_of(EV_ABS, 0x11, 0), None);
    }

    /// The analogue sticks are deliberately NOT mapped. A launcher driven by a
    /// stick moves on noise, because a resting stick reports small non-zero
    /// values forever and this build has no deadzone.
    #[test]
    fn the_analogue_sticks_are_not_a_dpad() {
        assert_eq!(action_of(EV_ABS, 0x00, -32000), None, "ABS_X");
        assert_eq!(action_of(EV_ABS, 0x01, 32000), None, "ABS_Y");
    }

    /// One 24-byte record, read out of a buffer exactly as the kernel wrote it.
    #[test]
    fn an_evdev_record_is_read_out_of_its_twenty_four_bytes() {
        let mut buf = [0u8; 48];
        buf[16..18].copy_from_slice(&EV_KEY.to_ne_bytes());
        buf[18..20].copy_from_slice(&103u16.to_ne_bytes());
        buf[20..24].copy_from_slice(&1i32.to_ne_bytes());
        buf[40..42].copy_from_slice(&EV_KEY.to_ne_bytes());
        buf[42..44].copy_from_slice(&28u16.to_ne_bytes());
        buf[44..48].copy_from_slice(&1i32.to_ne_bytes());
        let got: Vec<Action> = events_in(&buf)
            .filter_map(|(k, c, v)| action_of(k, c, v))
            .collect();
        assert_eq!(got, vec![Action::Up, Action::Enter]);
    }

    /// A partial record at the end of a read is dropped rather than misread.
    #[test]
    fn a_partial_trailing_record_is_not_decoded() {
        let buf = [0u8; 30];
        assert_eq!(events_in(&buf).count(), 1, "24 whole bytes, 6 left over");
    }

    /// **Two seconds is the ruling**, not a tunable. Asserted so that a change
    /// to it is a change somebody made on purpose.
    #[test]
    fn the_hold_is_two_seconds() {
        assert_eq!(HOLD, std::time::Duration::from_secs(2));
    }
}
