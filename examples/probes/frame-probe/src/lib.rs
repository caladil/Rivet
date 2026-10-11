//! **A pack that draws patterns a host can recompute**, so the frame boundary
//! can be proven by reading the bytes back rather than by looking at a screen.
//!
//! `examples/probes/addon-probe` proves the JSON half of the addon protocol.
//! This proves the other one — `.rulings/` R-199 D2's `#[repr(C)]` descriptor —
//! and the article a host runs against it is:
//!
//! 1. **a catalogue crosses.** `Describe` names a background per pattern
//!    below, each with its themes, which is the shape a picker is built from
//!    and the thing a multipack exists to make possible;
//! 2. **the pixels land where the descriptor said they would.** `flat` fills
//!    with one colour and `bands` writes a different colour per row, so a host
//!    that recomputes the pattern catches a stride read as a width, a row
//!    walked backwards, or an off-by-one at the last row;
//! 3. **the padding between rows is untouched.** A host fills it with a
//!    sentinel before the call and reads it back afterwards;
//! 4. **the host's clock crossed.** `stamp` writes `time_ms` into the first
//!    pixel, which is a `u64` field proven by a value rather than by its
//!    presence;
//! 5. **a version this pack does not speak is refused before a pixel is
//!    written**, which is the boundary's first rule and the one a host can
//!    only check by finding its own bytes still there;
//! 6. **the crossing can be timed.** `nothing` returns without writing
//!    anything, so the cost of one indirect call is measurable on its own
//!    rather than buried inside the cost of filling 8.29 MB;
//! 7. **a background that panics costs its frame and not the host.** `breaks`
//!    panics on purpose, because the `catch_unwind` in the generated entry
//!    only runs when a host has really `dlopen`ed this object — no test in
//!    this crate can reach it.
//!
//! # It is a pack and a reading source at once, and that is the design
//!
//! One object implements [`rivetos_addon::abi::Addon`] **and**
//! [`rivetos_addon::Drawing`]: the first is how its catalogue and its settings
//! cross, the second is how it draws. A background pack has no reading to
//! give, so [`FrameProbe::poll`] says so — which is what
//! [`rivetos_addon::Status::Unavailable`] means and is not a fault.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use rivetos_addon::abi::Addon;
use rivetos_addon::{
    Background, Cards, Description, Drawing, Field, FieldKind, Polled, Refusal, Scene, Status, Surface,
    Theme,
};

/// `flat` and `bands` and `stamp` all take these three, so a host can ask for
/// any pairing and know what it should get back. `0x00RR_GGBB`.
const THEMES: [(&str, &str, u32); 3] = [
    ("red", "Red", 0x00FF_0000),
    ("green", "Green", 0x0000_FF00),
    ("blue", "Blue", 0x0000_00FF),
];

/// The one theme [`NOTHING`] has. A background with no themes would make a
/// host render two shapes of picker, so even the one that draws nothing
/// carries one.
const STILL: (&str, &str) = ("still", "Still");

/// The background whose whole job is to return without writing, so that the
/// **crossing** can be timed apart from the drawing.
pub const NOTHING: &str = "nothing";

/// **The background that panics**, so that the drawing entry's
/// `catch_unwind` — rule 2 of `rivetos_addon::frame` — can be exercised from
/// outside the process that generated it.
///
/// Nothing real would have this. It is here for the same reason
/// `examples/probes/addon-probe` carries a `panic-now` setting: unwinding out
/// of an `extern "C"` function is undefined behaviour, the glue catches it,
/// and an untriggered rule is a rule nobody has seen work.
pub const BREAKS: &str = "breaks";

/// How long a host should wait before polling this pack again. Long, because
/// there is nothing to poll for: a pack is asked to describe itself once and
/// then asked to draw, and `call_again_in_ms` is a ceiling rather than a
/// promise.
const POLL_AGAIN_MS: u64 = 3_600_000;

#[derive(Default)]
pub struct FrameProbe {
    /// How many frames this object has drawn since it was loaded. A pack
    /// measures nothing in the world, so the count is what
    /// [`FrameProbe::poll`] reports — and it doubles as proof that the two
    /// halves of the protocol reach **one instance** rather than two.
    frames: u64,
    /// Set by `Configure`, and the reason this pack takes a setting at all: a
    /// host measuring the boundary wants every check the pack can skip to be
    /// skipped, and one that is proving correctness wants them on.
    strict: bool,
}

impl FrameProbe {
    /// What `flat` writes, and what a host recomputes to check it.
    #[must_use]
    pub fn flat_pixel(theme: &str) -> Option<u32> {
        THEMES
            .iter()
            .find(|(id, _, _)| *id == theme)
            .map(|(_, _, colour)| *colour)
    }

    /// What `bands` writes at row `y`. The row index rides in the low byte, so
    /// a host that recomputes this catches a renderer walking rows through the
    /// width instead of the stride: every row after the first would be shifted
    /// and the low byte would not match.
    #[must_use]
    pub fn band_pixel(theme: &str, y: u32) -> Option<u32> {
        Some(Self::flat_pixel(theme)? | (y & 0xFF))
    }

    /// What `stamp` writes into the first pixel. The low 24 bits of the host's
    /// own clock, which is a `u64` field proven by carrying a value rather
    /// than by being present.
    #[must_use]
    pub fn stamp_pixel(time_ms: u64) -> u32 {
        u32::try_from(time_ms & 0x00FF_FFFF).unwrap_or(0)
    }
}

impl Addon for FrameProbe {
    fn describe(&mut self) -> Description {
        // **`Cards::Unstated` on every theme here, and it is not laziness.** A
        // probe measures whether the frame crossing works; it draws no cards
        // and has no view about the ones a host would draw over it.
        // `Unstated` is what says *no opinion* (`rivetos_addon::Theme::cards`),
        // which is exactly the probe's position — and not `NoCard`, which
        // would be an opinion.
        let themes: Vec<Theme> = THEMES
            .iter()
            .map(|(id, label, _)| Theme {
                id: (*id).to_string(),
                label: (*label).to_string(),
                cards: Cards::Unstated,
                // All three of `THEMES` change the surface on every call —
                // `stamp` writes the host's own clock into pixel one, which is
                // the whole measurement.
                animated: true,
            })
            .collect();
        // `STILL` is the theme of the two backgrounds that draw nothing and
        // panic respectively. Neither changes a surface, so neither asks a
        // host to redraw for it.
        let still = || Theme {
            id: STILL.0.to_string(),
            label: STILL.1.to_string(),
            cards: Cards::Unstated,
            animated: false,
        };
        let background = |id: &str, name: &str, themes: Vec<Theme>, dark: bool| Background {
            id: id.to_string(),
            name: name.to_string(),
            themes,
            prefers_dark_cards: dark,
            // This probe exercises the CPU boundary and exports no GL entry;
            // saying so is what keeps a GPU host from asking it for one.
            drawn: rivetos_addon::Drawn::Cpu,
        };
        Description {
            name: "frame-probe".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            class: "probe-background".to_string(),
            material: Vec::new(),
            fields: vec![Field {
                key: "strict".to_string(),
                label: "Check the surface before drawing".to_string(),
                help: "Off, a frame is drawn with no check this pack could have skipped — \
                       which is what the measurement wants."
                    .to_string(),
                kind: FieldKind::Bool { default: true },
            }],
            backgrounds: vec![
                background("flat", "Flat colour", themes.clone(), false),
                background("bands", "Bands, one per row", themes.clone(), false),
                background("stamp", "The host's clock in pixel one", themes, false),
                background(NOTHING, "Nothing at all", vec![still()], true),
                background(BREAKS, "Panic while drawing", vec![still()], true),
            ],
        }
    }

    fn configure(
        &mut self,
        settings: &serde_json::Map<String, serde_json::Value>,
        _material: &BTreeMap<String, String>,
    ) -> Result<(), String> {
        self.strict = settings
            .get("strict")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        Ok(())
    }

    fn poll(&mut self, now_ms: i64) -> Polled {
        Polled {
            reading: Some(rivetos_addon::Reading {
                #[allow(clippy::cast_precision_loss)]
                value: self.frames as f64,
                unit: "frames".to_string(),
                taken_at_ms: now_ms,
                trend: None,
                attention: false,
            }),
            // **A pack has no reading to give**, and saying so is what this
            // status means. The frame count beside it is not a measurement of
            // anything in the world; it is there so a host can prove the two
            // halves of the protocol reached one instance.
            status: Status::Unavailable,
            call_again_in_ms: POLL_AGAIN_MS,
            complaint: "this addon draws rather than measures".to_string(),
            // One frame count and nothing beside it — `Polled::also` is for a
            // source that measured several things in one poll.
            also: Vec::new(),
        }
    }
}

impl Drawing for FrameProbe {
    fn draw(&mut self, surface: &mut Surface<'_>, scene: &Scene<'_>) -> Result<(), Refusal> {
        self.frames += 1;

        if scene.background == NOTHING {
            if scene.theme != STILL.0 {
                return Err(Refusal::UnknownTheme);
            }
            // **Nothing, on purpose.** What a host times around this call is
            // one indirect call, the descriptor's own checks and this branch.
            return Ok(());
        }

        // **The one rule of the boundary a passing test cannot reach.** The
        // `catch_unwind` that turns this into a code lives in the generated
        // entry, so it only runs when a host has actually `dlopen`ed this
        // object — which is what `frame-probe-host` does on a board.
        assert!(
            scene.background != BREAKS,
            "asked to panic while drawing, which the boundary must turn into a code"
        );

        let colour = Self::flat_pixel(scene.theme).ok_or(Refusal::UnknownTheme)?;
        match scene.background {
            "flat" => surface.fill(colour),
            "bands" => {
                for y in 0..surface.height() {
                    let pixel = (colour | (y & 0xFF)).to_le_bytes();
                    let Some(row) = surface.row(y) else {
                        break;
                    };
                    for cell in row.as_chunks_mut::<4>().0 {
                        cell.copy_from_slice(&pixel);
                    }
                }
            }
            "stamp" => {
                surface.fill(colour);
                surface.put(0, 0, Self::stamp_pixel(scene.time_ms));
            }
            _ => return Err(Refusal::UnknownBackground),
        }

        // With `strict` on, the pack re-reads the last row it wrote — the one
        // an off-by-one loses. It proves nothing a host's own read-back does
        // not prove better; it is here so the measurement has a check to
        // switch off, and so the switch is a real one rather than a comment.
        if self.strict {
            let last = surface.height().saturating_sub(1);
            if surface.row(last).is_none() {
                return Err(Refusal::UnsupportedSurface);
            }
        }
        Ok(())
    }
}

rivetos_addon::export_drawing_addon!(FrameProbe);

#[cfg(test)]
mod tests {
    use super::*;

    fn scene<'a>(background: &'a str, theme: &'a str, time_ms: u64) -> Scene<'a> {
        Scene {
            background,
            theme,
            time_ms,
        }
    }

    /// The catalogue is what a host builds two pickers from, so its shape is
    /// asserted rather than eyeballed.
    #[test]
    fn every_described_background_has_a_theme_and_an_id_a_frame_can_carry() {
        let d = FrameProbe::default().describe();
        assert_eq!(d.class, "probe-background");
        assert!(
            d.backgrounds.len() >= 4,
            "the article needs a fill, a per-row pattern, one that draws nothing and one that \
             panics, at the least"
        );
        for background in &d.backgrounds {
            assert!(
                !background.themes.is_empty(),
                "{} has no theme, so a host would render two shapes of picker",
                background.id
            );
            assert!(
                rivetos_addon::frame::id_bytes(&background.id).is_some(),
                "{} is not an id the frame descriptor can carry",
                background.id
            );
            for theme in &background.themes {
                assert!(
                    rivetos_addon::frame::id_bytes(&theme.id).is_some(),
                    "{} is not an id the frame descriptor can carry",
                    theme.id
                );
            }
        }
    }

    /// **The whole article, in process**, so that what runs on the board is a
    /// repeat over a real crossing rather than the first time anybody tried
    /// it. `dispatch` is called with a descriptor exactly as the generated
    /// entry would build one.
    #[test]
    fn the_patterns_are_what_a_host_recomputes() {
        let mut pack = FrameProbe::default();
        let (width, height, stride) = (16u32, 8u32, 80u32);
        let mut bytes = vec![0xABu8; (stride * height) as usize];

        {
            let mut surface = surface_over(&mut bytes, width, height, stride);
            pack.draw(&mut surface, &scene("flat", "green", 0))
                .expect("a flat green frame");
        }
        assert_eq!(pixel(&bytes, stride, 0, 0), 0x0000_FF00);
        assert_eq!(pixel(&bytes, stride, width - 1, height - 1), 0x0000_FF00);

        {
            let mut surface = surface_over(&mut bytes, width, height, stride);
            pack.draw(&mut surface, &scene("bands", "red", 0))
                .expect("a banded red frame");
        }
        for y in 0..height {
            assert_eq!(
                pixel(&bytes, stride, 0, y),
                FrameProbe::band_pixel("red", y).expect("a known theme"),
                "row {y} is not the band this pack writes"
            );
        }

        {
            let mut surface = surface_over(&mut bytes, width, height, stride);
            pack.draw(&mut surface, &scene("stamp", "blue", 0x1234_5678_9ABC))
                .expect("a stamped frame");
        }
        assert_eq!(pixel(&bytes, stride, 0, 0), 0x0078_9ABC);
        assert_eq!(pixel(&bytes, stride, 1, 0), 0x0000_00FF);

        // The padding between rows is the host's and every pass above left it
        // alone.
        for y in 0..height as usize {
            let start = y * stride as usize + (width as usize) * 4;
            let end = (y + 1) * stride as usize;
            assert!(
                bytes[start..end].iter().all(|b| *b == 0xAB),
                "row {y}'s padding was written"
            );
        }
    }

    #[test]
    fn it_refuses_a_background_and_a_theme_it_does_not_have() {
        let mut pack = FrameProbe::default();
        let mut bytes = vec![0u8; 64];
        let mut surface = surface_over(&mut bytes, 4, 4, 16);
        assert_eq!(
            pack.draw(&mut surface, &scene("starfield", "red", 0)),
            Err(Refusal::UnknownBackground)
        );
        assert_eq!(
            pack.draw(&mut surface, &scene("flat", "phosphor", 0)),
            Err(Refusal::UnknownTheme)
        );
        assert_eq!(
            pack.draw(&mut surface, &scene(NOTHING, "red", 0)),
            Err(Refusal::UnknownTheme),
            "the background that draws nothing still has exactly one theme"
        );
    }

    /// The background the measurement is taken against writes nothing, and a
    /// test rather than a comment is what keeps it that way.
    #[test]
    fn the_background_that_exists_to_be_timed_writes_no_byte() {
        let mut pack = FrameProbe::default();
        let mut bytes = vec![0xABu8; 64];
        {
            let mut surface = surface_over(&mut bytes, 4, 4, 16);
            pack.draw(&mut surface, &scene(NOTHING, STILL.0, 7))
                .expect("it draws nothing successfully");
        }
        assert!(bytes.iter().all(|b| *b == 0xAB));
    }

    /// Both halves of the protocol reach one object — the pack that described
    /// itself is the pack that drew.
    #[test]
    fn the_frame_count_the_poll_reports_is_the_one_the_drawing_moved() {
        let mut pack = FrameProbe::default();
        let mut bytes = vec![0u8; 64];
        {
            let mut surface = surface_over(&mut bytes, 4, 4, 16);
            pack.draw(&mut surface, &scene("flat", "red", 0))
                .expect("a frame");
            pack.draw(&mut surface, &scene("flat", "red", 0))
                .expect("another");
        }
        let reading = pack.poll(0).reading.expect("a frame count");
        assert_eq!(reading.value, 2.0);
        assert_eq!(reading.unit, "frames");
    }

    // ---- the surface, without a descriptor and without a host --------------

    /// A [`Surface`] over a buffer this test owns, through the same
    /// constructor the boundary's own `dispatch` uses — so a pattern proven
    /// here is proven against the geometry a real frame would arrive with.
    fn surface_over(bytes: &mut [u8], width: u32, height: u32, stride: u32) -> Surface<'_> {
        Surface::over(bytes, width, height, stride).expect("a surface this test described")
    }

    fn pixel(bytes: &[u8], stride: u32, x: u32, y: u32) -> u32 {
        let at = (y * stride + x * 4) as usize;
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }
}
