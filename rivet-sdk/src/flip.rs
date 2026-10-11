//! **The page-flip bookkeeping, with no C in it** — `PLAN.md` §104.33.4.
//!
//! [`crate::screen`] presents a frame by drawing into the page the display is
//! not reading and flipping to it at the next vertical blank. *Which* page that
//! is, what a completed flip does to the answer, and what happens when a flip is
//! refused, is all decided here — and none of it touches `libdrm`.
//!
//! # Why it is a separate file
//!
//! **So it can be tested.** `screen.rs` links `libdrm`, and a Rivet's `libdrm`
//! lives in the aarch64 sysroot: the development host has none, so a test binary
//! built with the `draw` feature does not link there —
//!
//! ```text
//! rust-lld: error: unable to find library -ldrm
//! ```
//!
//! — which is the failure `AGENTS.md` §10 warns about in its own section on
//! building applications. `AGENTS.md` §4: *a test suite that cannot be run will
//! rot silently.* Everything in this file is reachable from a plain
//! `cargo test -p rivetos-sdk` on any host, because the module is compiled under
//! `cfg(test)` whether or not `draw` is on.
//!
//! It is a real separation and not only a testing convenience: this is policy,
//! `screen.rs` is mechanism, and the policy is the half that has judgement in
//! it.

/// `DRM_EVENT_FLIP_COMPLETE`, from `drm/drm.h` line 973.
pub(crate) const EVENT_FLIP_COMPLETE: u32 = 0x02;

/// `struct drm_event` — `drm/drm.h` line 967. Two `__u32`: `type`, then
/// `length`, where **the length includes these eight bytes**.
pub(crate) const EVENT_HEADER: usize = 8;

/// How long a `blit` waits for a flip to complete before giving up on that one
/// frame.
///
/// **200 ms, and the number is argued rather than round.** A flip lands at a
/// vertical blank, so the honest upper bound is one or two refresh intervals:
/// 17–33 ms at 60 Hz, and 42–83 ms at the 24 Hz a cinema mode can ask for. Two
/// hundred is comfortably above the slowest of those and an order of magnitude
/// above the common case, so it cannot fire on a display that is merely slow.
///
/// **The lower bound on the choice is a hang.** A flip that never completes must
/// not stop a kiosk for ever — the application would be alive, drawing, and
/// showing nothing, which is the worst of the available failures because it
/// looks like a crash and is not one. At 200 ms a display that has stopped
/// delivering events costs five frames a second and a line in the journal
/// instead of a machine somebody has to power-cycle to diagnose.
///
/// Gated with [`Flip`] below: these two are the only things in this file that
/// nothing but `screen` reads, so without them the crate would warn about dead
/// code in a `cargo test` with no `screen`.
#[cfg(feature = "screen")]
pub(crate) const FLIP_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(200);

/// What became of one attempt to put a page on the glass.
#[cfg(feature = "screen")]
pub(crate) enum Flip {
    /// The kernel said the flip completed.
    Done,
    /// The flip was scheduled and no completion event arrived in time. See
    /// `Screen::blit` for why this still swaps.
    Missed(String),
    /// The flip was refused. The screen degrades to one page.
    Refused(String),
}

/// **Which page is on the glass, and which one a `blit` draws into.**
///
/// Split out of `Screen` because it is the whole of the bookkeeping and none of
/// the FFI. Every state transition `screen.rs` makes is a method here, and
/// `mod tests` below exercises all of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rotation {
    /// The index in `Screen::pages` of the page the scanout engine is reading.
    pub(crate) front: usize,
    /// `Some` once this screen is presenting from a single page, carrying the
    /// sentence that says why. Set at construction if the second page could not
    /// be had, and later if the driver refuses a flip.
    ///
    /// **It never goes back to `None`.** A driver that refused one flip will
    /// refuse the next, and a screen that alternated between tearing and not
    /// would be harder to diagnose than one that simply tears.
    pub(crate) single_because: Option<String>,
}

impl Rotation {
    /// Two pages, page 0 scanning out — which is the one `drmModeSetCrtc` was
    /// given.
    pub(crate) const fn double() -> Self {
        Self { front: 0, single_because: None }
    }

    /// One page. `why` is a sentence for an operator.
    pub(crate) fn single(why: String) -> Self {
        Self { front: 0, single_because: Some(why) }
    }

    /// Whether a flip is going to be attempted at all.
    pub(crate) const fn is_double(&self) -> bool {
        self.single_because.is_none()
    }

    /// **The page a `blit` copies into**: the back one if there is one, and
    /// otherwise the page being scanned out, tearing and all.
    pub(crate) const fn target(&self) -> usize {
        if self.is_double() { 1 - self.front } else { self.front }
    }

    /// The flip completed: what was the back page is now on the glass.
    pub(crate) fn presented(&mut self) {
        if self.is_double() {
            self.front = 1 - self.front;
        }
    }

    /// **Fall back to one page**, permanently, and say why.
    ///
    /// The page that is scanning out stays the front page, so the caller's next
    /// `blit` writes into the memory the hardware is reading — which is the
    /// pre-2026-09-01 behaviour, and is exactly what "degraded" means here.
    ///
    /// Returns whether this was the transition. A second call changes nothing
    /// and reports `false`, so the caller prints one line rather than one a
    /// frame.
    pub(crate) fn degrade(&mut self, why: String) -> bool {
        if self.single_because.is_some() {
            return false;
        }
        self.single_because = Some(why);
        true
    }

    /// The sentence for `Screen::what`.
    pub(crate) fn describe(&self) -> String {
        match &self.single_because {
            None => "double-buffered, page-flipped at vblank".to_string(),
            Some(why) => format!("SINGLE-BUFFERED so the display WILL TEAR: {why}"),
        }
    }
}

/// **Walk a buffer of `struct drm_event`s and say whether a flip completed.**
///
/// Taken as a slice rather than read from the descriptor here, so it can be
/// tested against bytes. The layout is `drm/drm.h`'s: two `__u32`s, where
/// `length` counts the header as well.
///
/// `from_ne_bytes` and not `from_le_bytes`: the kernel writes these in the
/// machine's own byte order, so native is the correct reading on any host and
/// little-endian is merely the correct reading on this one.
pub(crate) fn flip_completed(events: &[u8]) -> bool {
    let mut at = 0usize;
    while at + EVENT_HEADER <= events.len() {
        let kind =
            u32::from_ne_bytes([events[at], events[at + 1], events[at + 2], events[at + 3]]);
        let length = u32::from_ne_bytes([
            events[at + 4],
            events[at + 5],
            events[at + 6],
            events[at + 7],
        ]) as usize;
        if kind == EVENT_FLIP_COMPLETE {
            return true;
        }
        // A length that does not advance would spin for ever. The kernel does
        // not write one; refusing to trust that costs one comparison.
        if length < EVENT_HEADER {
            return false;
        }
        at += length;
    }
    false
}

/// **What can be tested without a display, and what deliberately cannot.**
///
/// Everything here is the *bookkeeping*: which page a `blit` targets, that the
/// two alternate, that a failure degrades once and stays degraded, and that the
/// event walker reads `struct drm_event` the way `drm/drm.h` lays it out.
///
/// **None of it proves the absence of tearing, and no test can.** A capture
/// reads a buffer; tearing is an artefact of what the scanout engine puts on
/// the glass, so it exists only where no program can look. That takes a person
/// in front of the screen, and saying so is worth more than a test that implies
/// otherwise.
#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::{EVENT_FLIP_COMPLETE, Rotation, flip_completed};

    /// `DRM_EVENT_VBLANK`, `drm/drm.h` line 972 — the event this module must
    /// NOT mistake for a completed flip.
    const EVENT_VBLANK: u32 = 0x01;

    /// One `struct drm_event_vblank`'s worth of bytes: the two-`u32` header
    /// followed by the payload the kernel writes, in the machine's own byte
    /// order. Thirty-two bytes is that struct's size — `drm/drm.h` line 976.
    fn event(kind: u32) -> Vec<u8> {
        let length: u32 = 32;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&kind.to_ne_bytes());
        bytes.extend_from_slice(&length.to_ne_bytes());
        bytes.resize(length as usize, 0);
        bytes
    }

    #[test]
    fn a_double_buffered_screen_draws_into_the_page_that_is_not_scanning_out() {
        let r = Rotation::double();
        assert_eq!(r.front, 0, "drmModeSetCrtc was given page 0");
        assert_eq!(r.target(), 1, "so a blit must draw into page 1");
        assert!(r.is_double());
    }

    /// **The two pages genuinely alternate.** This is the property a screenshot
    /// cannot show and the one the whole change turns on: a blit must never
    /// target the page the previous flip put on the glass.
    #[test]
    fn successive_blits_alternate_between_the_two_pages() {
        let mut r = Rotation::double();
        let mut targeted = Vec::new();
        let mut on_the_glass = Vec::new();
        for _ in 0..8 {
            let t = r.target();
            targeted.push(t);
            r.presented();
            on_the_glass.push(r.front);
            assert_eq!(
                r.front, t,
                "the page just flipped to must be the page that was drawn into"
            );
        }
        assert_eq!(targeted, vec![1, 0, 1, 0, 1, 0, 1, 0]);
        assert_eq!(on_the_glass, vec![1, 0, 1, 0, 1, 0, 1, 0]);
    }

    /// The fallback path: the second buffer could not be allocated, so there is
    /// one page and every blit goes into it.
    #[test]
    fn a_screen_with_one_page_always_draws_into_the_page_being_scanned_out() {
        let mut r = Rotation::single("no dumb buffer at 1920x1080: -12".to_string());
        assert!(!r.is_double());
        for _ in 0..8 {
            assert_eq!(r.target(), 0, "there is only one page to draw into");
            // A single-buffered screen never flips, but a `presented` reaching
            // here must not start rotating to a page that is not there.
            r.presented();
            assert_eq!(r.front, 0);
        }
    }

    /// A refused flip degrades the screen, and the frame the caller drew then
    /// belongs in the page that IS being scanned out.
    #[test]
    fn a_refused_flip_degrades_once_and_redirects_the_frame() {
        let mut r = Rotation::double();
        assert_eq!(r.target(), 1);

        assert!(r.degrade("drmModePageFlip said -22".to_string()));
        assert!(!r.is_double());
        assert_eq!(r.target(), 0, "the frame must be re-copied into the front page");

        assert!(
            !r.degrade("something else".to_string()),
            "a second degradation is not a transition and must not print again"
        );
        assert!(
            r.single_because.as_deref().unwrap().contains("-22"),
            "the FIRST reason is the one worth keeping: it is what actually went wrong"
        );
    }

    #[test]
    fn the_record_says_which_of_the_two_a_screen_is() {
        assert!(Rotation::double().describe().contains("page-flipped"));
        let degraded = Rotation::single("the display class was not granted".to_string());
        let said = degraded.describe();
        assert!(said.contains("TEAR"), "an operator must be able to tell {said:?} apart");
        assert!(said.contains("the display class was not granted"));
    }

    #[test]
    fn a_flip_complete_event_is_recognised() {
        assert!(flip_completed(&event(EVENT_FLIP_COMPLETE)));
    }

    #[test]
    fn a_vblank_event_is_not_a_completed_flip() {
        assert!(!flip_completed(&event(EVENT_VBLANK)));
    }

    /// The read returns whole events and there may be more than one of them.
    #[test]
    fn a_flip_behind_another_event_is_still_found() {
        let mut buf = event(EVENT_VBLANK);
        buf.extend_from_slice(&event(EVENT_FLIP_COMPLETE));
        assert!(flip_completed(&buf));
    }

    #[test]
    fn nothing_is_read_out_of_a_short_or_empty_buffer() {
        assert!(!flip_completed(&[]));
        assert!(!flip_completed(&[0u8; 7]), "a header needs eight bytes");
    }

    /// A length that does not advance would spin the walker for ever. The
    /// kernel does not write one; this proves the guard against it works, which
    /// is cheaper than trusting it.
    #[test]
    fn an_event_claiming_no_length_terminates_the_walk() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&EVENT_VBLANK.to_ne_bytes());
        buf.extend_from_slice(&0u32.to_ne_bytes());
        buf.extend_from_slice(&event(EVENT_FLIP_COMPLETE));
        assert!(
            !flip_completed(&buf),
            "the walk stops at the malformed event rather than reading past it"
        );
    }
}
