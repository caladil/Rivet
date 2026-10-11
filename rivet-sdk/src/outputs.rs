//! **What a display descriptor's connectors amount to, with no C in it.**
//!
//! [`crate::screen`] walks the connectors a descriptor lists and takes the first
//! one that is connected, reports a mode, and names an encoder. When none of
//! them is, *why* is the sentence an operator reads — and until 2026-09-05 there
//! was one sentence for every reason:
//!
//! ```text
//! a leased display: no connected output with a mode — is a cable in?
//! ```
//!
//! # The day that sentence cost
//!
//! It was emitted by `aura-renderer` on `rivet-01` while the cable had not
//! moved, on the same boot on which the launcher had just said
//! `TOOK THE DISPLAY - /dev/dri/card0 connector 42 crtc 112 mode 1920x1080`.
//! The descriptor it had been handed was a **revoked lease**: the launcher cut
//! one, the supervisor wrote it into the application's display channel, and the
//! launcher revoked it before the application was ever started. A revoked lease
//! is emptied of its objects, so the ioctls still answer and they describe a
//! display with **no connectors at all** — a condition that has nothing to do
//! with a cable, and that the message named as a cable.
//!
//! The reading that sentence invited was that the Landlock confinement shipped
//! in the same image had broken the display. It had not; eleven seconds later
//! the same binary, under the same ruleset, drew at 1920x1080.
//!
//! `AGENTS.md` §5: *report what the system said, not what it was expected to
//! say.* Counting the four things separately is what lets the report do that.
//!
//! # Why it is a separate file
//!
//! **So it can be tested.** `screen.rs` links `libdrm`, which lives only in the
//! aarch64 sysroot, so a host test binary built with the `screen` feature does
//! not link — `rust-lld: error: unable to find library -ldrm`. This module is
//! compiled under `cfg(test)` whether or not `screen` is on, so everything in it
//! is reachable from a plain `cargo test -p rivetos-sdk` on any host. It is
//! [`crate::flip`]'s argument, applied to the other half of `screen.rs` that has
//! judgement in it.

/// Where the descriptor came from, for the half of the sentence that differs.
///
/// A `bool` would do and is not used: the two cases produce *different
/// diagnoses* from the same counts, and a reader of the call site should be
/// able to see which one is being asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Held {
    /// A descriptor that arrived on `RIVET_DISPLAY` — a DRM lease.
    Leased,
    /// A device node this process opened by path.
    Opened,
}

/// What one pass over a descriptor's connectors found.
///
/// Every field is a count of something the kernel answered, and each narrows
/// the one above it: `described <= listed`, `connected <= described`, and so on.
/// The point of keeping all four rather than a boolean is that **the step where
/// the funnel closes is the diagnosis.**
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Survey {
    /// Connectors `drmModeGetResources` put in the list.
    pub(crate) listed: usize,
    /// Of those, the ones `drmModeGetConnector` would describe.
    pub(crate) described: usize,
    /// Of those, the ones reporting `DRM_MODE_CONNECTED`.
    pub(crate) connected: usize,
    /// Of those, the ones carrying at least one mode.
    pub(crate) with_mode: usize,
    /// Of those, the ones naming an encoder — which is how [`crate::screen`]
    /// reaches a CRTC.
    pub(crate) with_encoder: usize,
}

impl Survey {
    /// **Why no output could be driven**, as a sentence for a person.
    ///
    /// Only called when nothing usable was found, so `with_encoder` being zero
    /// is the caller's precondition rather than something to assert here.
    pub(crate) fn why(&self, held: Held) -> String {
        if self.listed == 0 {
            return match held {
                // The one this module was written for.
                Held::Leased => "this lease carries NO CONNECTOR AT ALL. The descriptor \
                     arrived and the ioctls answered — the lease is simply empty, which is \
                     what a REVOKED lease looks like from the inside: the lessor takes the \
                     objects back and every ioctl afterwards describes a display with no \
                     outputs. THIS IS NOT A CABLE. The usual cause is a lease that was cut, \
                     written to this application's display channel, and revoked before this \
                     process was ever started to read it"
                    .to_string(),
                Held::Opened => "this device lists no connectors, so there is nothing on it \
                     to scan out with. A render node cannot modeset and a card node without \
                     the display driver bound answers like this"
                    .to_string(),
            };
        }
        if self.described == 0 {
            return format!(
                "{} connector(s) are listed and the kernel would describe none of them, so \
                 nothing here can be asked whether it has a display on the end",
                self.listed
            );
        }
        if self.connected == 0 {
            // **The original sentence, kept, where it is the right question.**
            return format!(
                "{} connector(s), and not one of them reports anything plugged in — is a \
                 cable in?",
                self.described
            );
        }
        if self.with_mode == 0 {
            return format!(
                "{} connected output(s) and not one of them reports a mode, so there is no \
                 timing to drive them with. A connector whose probe has not run yet reads \
                 like this",
                self.connected
            );
        }
        format!(
            "{} connected output(s) with a mode, and not one of them names an encoder, so \
             there is no CRTC to reach the panel through",
            self.with_mode
        )
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::{Held, Survey};

    /// **The regression this module exists for.**
    ///
    /// A revoked lease answers every ioctl and lists nothing. The sentence must
    /// say that, and must not send the reader to look at a cable — which is
    /// what it did on 2026-09-05, on a board whose cable had not moved and
    /// whose launcher had set a 1920x1080 mode through the same connector one
    /// millisecond earlier.
    #[test]
    fn an_empty_lease_is_reported_as_a_lease_and_not_as_a_cable() {
        let said = Survey::default().why(Held::Leased);
        assert!(
            said.contains("REVOKED"),
            "an empty lease did not name a revocation: {said}"
        );
        assert!(
            said.contains("NOT A CABLE"),
            "an empty lease did not rule the cable out: {said}"
        );
        assert!(
            !said.contains("is a cable in?"),
            "an empty lease still asks the cable question: {said}"
        );
    }

    /// The other zero-connector case is a different machine and reads
    /// differently: a node this process opened itself and that has no outputs
    /// is not a lease and cannot have been revoked.
    #[test]
    fn an_opened_node_with_no_connectors_is_not_described_as_a_revoked_lease() {
        let said = Survey::default().why(Held::Opened);
        assert!(
            !said.contains("lease"),
            "an opened node was described as a lease: {said}"
        );
        assert!(
            said.contains("no connectors"),
            "an opened node with no connectors did not say so: {said}"
        );
    }

    /// **And the cable question survives where it is the right one.** A
    /// descriptor that lists connectors and finds nothing plugged into any of
    /// them IS a cable question, and replacing every message with a lease
    /// diagnosis would be the same defect pointing the other way.
    #[test]
    fn connectors_present_and_none_connected_still_asks_about_the_cable() {
        let survey = Survey {
            listed: 3,
            described: 3,
            ..Survey::default()
        };
        for held in [Held::Leased, Held::Opened] {
            let said = survey.why(held);
            assert!(
                said.contains("is a cable in?"),
                "{held:?}: a disconnected output stopped asking about the cable: {said}"
            );
            assert!(
                said.contains('3'),
                "{held:?}: the count the kernel gave is not in the sentence: {said}"
            );
        }
    }

    /// A connected output with no mode is the probe, not the cable.
    #[test]
    fn connected_with_no_mode_names_the_missing_mode() {
        let survey = Survey {
            listed: 1,
            described: 1,
            connected: 1,
            ..Survey::default()
        };
        let said = survey.why(Held::Leased);
        assert!(said.contains("no timing"), "{said}");
        assert!(!said.contains("is a cable in?"), "{said}");
    }

    /// A connected output with a mode and no encoder is the last step of the
    /// funnel, and it is the one a lease cut without its encoder would produce.
    #[test]
    fn a_mode_with_no_encoder_names_the_encoder() {
        let survey = Survey {
            listed: 1,
            described: 1,
            connected: 1,
            with_mode: 1,
            with_encoder: 0,
        };
        let said = survey.why(Held::Leased);
        assert!(said.contains("encoder"), "{said}");
        assert!(said.contains("CRTC"), "{said}");
        assert!(!said.contains("is a cable in?"), "{said}");
    }

    /// **Each step of the funnel produces its OWN sentence**, and no two are the
    /// same. Without this the four branches above could all be right about their
    /// own keyword and still collapse to one message for a reader.
    #[test]
    fn every_step_of_the_funnel_says_something_different() {
        let stages = [
            Survey::default(),
            Survey { listed: 2, ..Survey::default() },
            Survey { listed: 2, described: 2, ..Survey::default() },
            Survey { listed: 2, described: 2, connected: 1, ..Survey::default() },
            Survey {
                listed: 2,
                described: 2,
                connected: 1,
                with_mode: 1,
                with_encoder: 0,
            },
        ];
        let said: Vec<String> = stages.iter().map(|s| s.why(Held::Leased)).collect();
        for (i, a) in said.iter().enumerate() {
            for (j, b) in said.iter().enumerate() {
                assert!(
                    i == j || a != b,
                    "stage {i} and stage {j} produce the same sentence: {a}"
                );
            }
        }
    }
}
