//! **What this pack has, on both sides of the addon boundary.**
//!
//! One table, `ROWS`, used three ways:
//!
//! * [`describe`] groups it by background id into the
//!   [`rivetos_addon::Description::backgrounds`] list a host reads once, for
//!   its picker (`docs/design-candidates/Background-Addons-2026-09-19.md`
//!   §3 — the field that makes a multipack possible, R-199 D1);
//! * [`resolve`] looks a `(background id, theme id)` pair back up into the
//!   [`crate::background::Background`] value [`crate::render::render_frame`]
//!   actually draws, on every frame;
//! * `crate::evidence` walks it to capture and hash every frame of every
//!   combination.
//!
//! **One table and not three**, so a background added to the picker cannot be
//! forgotten from what draws, or the reverse — `ADR-011`'s rule, applied to a
//! catalogue instead of to a runtime value.
//!
//! The **first ten** rows are the same ten `examples/aura/aura-renderer/src/
//! evidence.rs`'s `BACKGROUNDS` table lists, in the same order, under the
//! same ids — and they must stay that way, byte for byte on screen, which
//! `evidence.rs`'s baseline comparison is what enforces. The **last twelve**
//! are T8's four new backgrounds; see [`ROWS`].
//!
//! # The card appearance lives here too, as of 2026-09-20
//!
//! A row carries [`Row::cards`] — what a card drawn *over* this combination
//! should look like — because `aura-renderer` stopped holding that table when
//! it became a host (T7). The values are
//! `examples/aura/aura-renderer/src/widgets.rs`'s `card_theme`, constant for
//! constant, in the same sense §5 of the design meant when the drawing moved:
//! the renderer was the authority and this is now.
//!
//! **Why the pack answers this at all**: legibility is a joint property of
//! the program drawing the ground and the program drawing the widgets, and
//! only one of them knows what the ground is doing. A host holding the table
//! is a host that must be edited before this pack may grow an eleventh row,
//! which is the duplication a pack exists to remove.

use crate::background::{Background, BoxPalette, RainPalette, Solid};
use crate::designed::{AuroraPalette, ContourPalette, LifeStyle, StarPalette};
use rivetos_addon::{
    Background as ProtoBackground, CardAppearance, Cards, Drawn, Refusal, Theme as ProtoTheme,
};

/// One row of this pack's catalogue.
///
/// **A struct rather than the tuple this was**, since the row grew a sixth and
/// seventh member: `(&str, &str, &str, &str, Background)` read well enough,
/// and `(&str, &str, &str, &str, Background, Cards, bool)`
/// would have put a bare `false` at the end of ten lines with nothing on it
/// saying which question it answers.
pub(crate) struct Row {
    /// `digital-rain` — what crosses in a frame descriptor and what an
    /// operator's saved choice is.
    pub(crate) background_id: &'static str,
    /// `Digital Rain` — what a picker shows.
    pub(crate) background_label: &'static str,
    /// `ice-blue`.
    pub(crate) theme_id: &'static str,
    /// `Ice Blue`.
    pub(crate) theme_label: &'static str,
    /// The value [`crate::render::render_frame`] draws.
    pub(crate) background: Background,
    /// The card a host should draw over this combination, or
    /// [`Cards::NoCard`] for the C++'s `card = nullptr` — no rectangle at all,
    /// the widget's ink used raw. Solid Black is the one row that wants that,
    /// and it is Aura's default picture.
    ///
    /// **Never [`Cards::Unstated`]**: this pack has an opinion about every
    /// row, and `every_row_states_its_card` holds it to that.
    pub(crate) cards: Cards,
    /// `isAnimated()`. Solid does not change between selections; the other two
    /// are the whole screen changing thirty times a second.
    pub(crate) animated: bool,
}

/// The dark rows' signal colours — `widgets.rs`'s `ALERT_DARK` and
/// `CAUTION_DARK`, which were `CardTheme`'s in-class defaults.
const ALERT_DARK: [u8; 3] = [255, 69, 58];
const CAUTION_DARK: [u8; 3] = [255, 197, 0];
/// The two light rows' — `kSolidWhite` and `kBoxesIce`. Not a preference:
/// `[255, 69, 58]` on a white ground does not read.
const ALERT_LIGHT: [u8; 3] = [186, 26, 26];
const CAUTION_LIGHT: [u8; 3] = [150, 95, 0];

/// `widgets.rs`'s `dark()` — a card with the dark signal colours and no
/// rainbow, which is six of the nine rows that have a card at all.
const fn dark(bg: [u8; 3], bg_alpha: u8, text: [u8; 3]) -> Cards {
    Cards::Card(CardAppearance {
        bg,
        bg_alpha,
        text,
        rainbow: false,
        alert: ALERT_DARK,
        caution: CAUTION_DARK,
    })
}

/// A transparent card with black ink and the light signal colours —
/// `kSolidWhite` and `kBoxesIce`, which are the two bright grounds.
const fn light() -> Cards {
    Cards::Card(CardAppearance {
        bg: [0, 0, 0],
        bg_alpha: 0,
        text: [0, 0, 0],
        rainbow: false,
        alert: ALERT_LIGHT,
        caution: CAUTION_LIGHT,
    })
}

/// `kCardPride` — the six-stop gradient card, black ink, dark signal colours.
///
/// **`bg_alpha` is 0 and that is not a contradiction.** `rainbow` replaces the
/// fill rather than tinting it, so the flat colour underneath is never drawn;
/// the zero is what a host falling back to a flat fill would (correctly) draw
/// as nothing. `widgets.rs` says *alpha 190* in a comment above code that
/// passes 0, and the code is the port's authority.
const fn pride() -> Cards {
    Cards::Card(CardAppearance {
        bg: [0, 0, 0],
        bg_alpha: 0,
        text: [0, 0, 0],
        rainbow: true,
        alert: ALERT_DARK,
        caution: CAUTION_DARK,
    })
}

/// **The alpha a card needs over Aurora** — T8, 2026-09-20.
///
/// Aurora has no black ground at all: it paints a lit gradient into every one
/// of two million pixels, and the
/// brightest stop of the Ember ramp is `[245, 180, 80]`. The 155 the ported
/// rows use is tuned against Digital Rain, which paints itself black every
/// frame and then lights a few per cent of it; at 155 over Aurora the
/// gradient reads straight through the card and the clock stops being
/// legible. This is a genuinely opaque card, and the small remaining
/// translucency is only so the card does not look pasted on.
const CARD_OVER_AURORA: u8 = 235;

/// **The alpha a card needs over Conway's Game of Life.**
///
/// At 160x90 on a 1080p panel a cell is twelve pixels square, and a dense
/// cluster is a solid block of `[190, 205, 247]` — in the corners, which is
/// exactly where Aura's cards sit. Less than Aurora's, because Life is black
/// wherever it is not alive and the card is not fighting a full-screen
/// gradient; considerably more than Digital Rain's 155, because a rain cell
/// is a thin glyph and a Life cell is a filled square.
const CARD_OVER_LIFE: u8 = 190;

/// **The alpha a card needs over Starfield.**
///
/// Starfield is the emptiest picture in the pack — a few hundred one- and
/// two-pixel plots on black, so at any moment well under a tenth of one per
/// cent of the screen is lit. A card at 155 would be a visible grey slab on
/// black for no benefit. This is just enough to stop a near star crossing a
/// glyph, and reads as a vignette rather than as a panel.
const CARD_OVER_STARS: u8 = 110;

/// Shorthand for the rows below, which would otherwise be twenty-two
/// seven-line literals.
const fn row(
    background_id: &'static str,
    background_label: &'static str,
    theme_id: &'static str,
    theme_label: &'static str,
    background: Background,
    cards: Cards,
    animated: bool,
) -> Row {
    Row {
        background_id,
        background_label,
        theme_id,
        theme_label,
        background,
        cards,
        animated,
    }
}

/// This pack's whole catalogue. The first ten rows' ids and labels are the
/// maintainer's ruling of 2026-09-01, carried in
/// `examples/aura/aura-renderer/src/theme.rs`'s `BACKGROUND_PALETTES`; their
/// card appearances are `widgets.rs`'s `card_theme`, ported here 2026-09-20.
///
/// **The last twelve are T8's** — Starfield, Aurora, Contours and Conway's
/// Game of Life, to `docs/design-candidates/Background-Addons-2026-09-19.md`
/// §6. They are appended rather than interleaved, and that is load-bearing in
/// one direction only: `describe` groups by id in first-seen order, so a
/// picker lists the four ported-family backgrounds in the order Aura has
/// always shown them and the new ones after. `resolve` and `evidence.rs` do
/// not care about order at all.
pub(crate) const ROWS: [Row; 22] = [
    // `default: card = nullptr`. Aura's out-of-the-box picture, and the only
    // row with no card at all — asked for, as `null` on the wire, rather than
    // left to `prefers_dark_cards`.
    row(
        "solid",
        "Solid",
        "black",
        "Black",
        Background::Solid(Solid::Black),
        Cards::NoCard,
        false,
    ),
    // `kSolidWhite` — the one bright Solid, and the row the old
    // per-background `prefers_dark_cards` got wrong by construction.
    row(
        "solid",
        "Solid",
        "white",
        "White",
        Background::Solid(Solid::White),
        light(),
        false,
    ),
    // `kSolidNavy`
    row(
        "solid",
        "Solid",
        "navy",
        "Navy",
        Background::Solid(Solid::Navy),
        dark([0, 0, 0], 0, [180, 210, 240]),
        false,
    ),
    // `kCardGrn`
    row(
        "digital-rain",
        "Digital Rain",
        "phosphor-green",
        "Phosphor Green",
        Background::DigitalRain(RainPalette::PhosphorGreen),
        dark([0, 8, 2], 155, [180, 255, 180]),
        true,
    ),
    // `kCardIce`
    row(
        "digital-rain",
        "Digital Rain",
        "ice-blue",
        "Ice Blue",
        Background::DigitalRain(RainPalette::IceBlue),
        dark([3, 5, 14], 155, [190, 205, 247]),
        true,
    ),
    // `kCardPride`
    row(
        "digital-rain",
        "Digital Rain",
        "born-this-way",
        "Born This Way",
        Background::DigitalRain(RainPalette::BornThisWay),
        pride(),
        true,
    ),
    // `kBoxesIce` — the second bright ground.
    row(
        "boxes",
        "Boxes",
        "ice-blue",
        "Ice Blue",
        Background::Boxes(BoxPalette::IceBlue),
        light(),
        true,
    ),
    // `kBoxesGreen`
    row(
        "boxes",
        "Boxes",
        "chartreuse",
        "Chartreuse",
        Background::Boxes(BoxPalette::Chartreuse),
        dark([0, 30, 8], 155, [180, 255, 180]),
        true,
    ),
    // `kBoxesDark`
    row(
        "boxes",
        "Boxes",
        "dark",
        "Dark",
        Background::Boxes(BoxPalette::Dark),
        dark([0, 0, 0], 0, [255, 255, 255]),
        true,
    ),
    // `kCardPride` again — Boxes and Digital Rain share the pride card.
    row(
        "boxes",
        "Boxes",
        "born-this-way",
        "Born This Way",
        Background::Boxes(BoxPalette::BornThisWay),
        pride(),
        true,
    ),
    // --- T8 -------------------------------------------------------------
    // Starfield. The theme is the star colour and the drift rate (§6), and
    // the card is the lightest in the pack — see `CARD_OVER_STARS`.
    row(
        "starfield",
        "Starfield",
        "white",
        "White",
        Background::Starfield(StarPalette::White),
        dark([0, 0, 0], CARD_OVER_STARS, [235, 240, 248]),
        true,
    ),
    row(
        "starfield",
        "Starfield",
        "amber",
        "Amber",
        Background::Starfield(StarPalette::Amber),
        dark([6, 4, 0], CARD_OVER_STARS, [250, 225, 180]),
        true,
    ),
    row(
        "starfield",
        "Starfield",
        "ice-blue",
        "Ice Blue",
        Background::Starfield(StarPalette::IceBlue),
        dark([0, 3, 10], CARD_OVER_STARS, [190, 205, 247]),
        true,
    ),
    // Aurora. The theme is the palette ramp (§6). **Every Aurora row gets an
    // opaque card** — see `CARD_OVER_AURORA`; this is the one background in
    // the pack where a transparent card would be unreadable.
    row(
        "aurora",
        "Aurora",
        "green-violet",
        "Green Violet",
        Background::Aurora(AuroraPalette::GreenViolet),
        dark([4, 6, 10], CARD_OVER_AURORA, [215, 240, 225]),
        true,
    ),
    row(
        "aurora",
        "Aurora",
        "deep-blue",
        "Deep Blue",
        Background::Aurora(AuroraPalette::DeepBlue),
        dark([3, 5, 12], CARD_OVER_AURORA, [205, 220, 245]),
        true,
    ),
    row(
        "aurora",
        "Aurora",
        "ember",
        "Ember",
        Background::Aurora(AuroraPalette::Ember),
        dark([10, 5, 3], CARD_OVER_AURORA, [250, 225, 205]),
        true,
    ),
    // Contours. The theme is the line colour and the contour spacing (§6).
    // The ground is black wherever there is no line, so these take Digital
    // Rain's 155 — the same picture behind the card, near enough.
    row(
        "contours",
        "Contours",
        "phosphor-green",
        "Phosphor Green",
        Background::Contours(ContourPalette::PhosphorGreen),
        dark([0, 8, 2], 155, [180, 255, 180]),
        true,
    ),
    row(
        "contours",
        "Contours",
        "ice-blue",
        "Ice Blue",
        Background::Contours(ContourPalette::IceBlue),
        dark([3, 5, 14], 155, [190, 205, 247]),
        true,
    ),
    row(
        "contours",
        "Contours",
        "ember",
        "Ember",
        Background::Contours(ContourPalette::Ember),
        dark([12, 5, 0], 155, [255, 215, 180]),
        true,
    ),
    // **Conway's Game of Life, and the long label is deliberate.** The id
    // stays `life` — `rivetos_addon::protocol::Background` says why: the id
    // is what crosses in the frame descriptor and what an operator's saved
    // choice is, so it is fixed for the life of the product, and a 32-byte
    // `ID_BYTES` field would not carry the long form anyway. The name is for
    // a person and is never compared to anything. The maintainer asked for
    // the attribution on 2026-09-20, so that the man who devised the rules is
    // credited in the name (.rulings/ R-349). Do not shorten it back to `Life`
    // for tidiness.
    //
    // The three themes are the CELL SHAPE rather than §6's colour and fade,
    // also the maintainer's, 2026-09-20; `crate::designed::LifeStyle` carries
    // that departure and what `Pulse`'s radius means. All three share one
    // card, because all three draw the same colour at the same density and
    // the shape of a cell does not change what a card over it needs.
    row(
        "life",
        "Conway's Game of Life",
        "squares",
        "Squares",
        Background::Life(LifeStyle::Squares),
        dark([2, 4, 10], CARD_OVER_LIFE, [190, 205, 247]),
        true,
    ),
    row(
        "life",
        "Conway's Game of Life",
        "dots",
        "Dots",
        Background::Life(LifeStyle::Dots),
        dark([2, 4, 10], CARD_OVER_LIFE, [190, 205, 247]),
        true,
    ),
    row(
        "life",
        "Conway's Game of Life",
        "pulse",
        "Pulse",
        Background::Life(LifeStyle::Pulse),
        dark([2, 4, 10], CARD_OVER_LIFE, [190, 205, 247]),
        true,
    ),
];

/// **Whether the cards drawn over a background should be dark**, for a host
/// too old to read [`ProtoTheme::cards`].
///
/// This is now a *fallback field* and no longer how this pack answers the
/// question — [`Row::cards`] does, per theme, which is the granularity the
/// answer actually has. The value below is the one this pack shipped with and
/// it is deliberately unchanged: a host old enough to read this field is a
/// host that would be given a different picture by changing it.
///
/// Digital Rain paints its own black ground every frame regardless of theme
/// (`background.rs`'s `DigitalRain::draw`), and every Boxes palette is a dark
/// or mid-toned ramp, so `false` fits both. `solid` genuinely varies, and
/// `false` is right for `Solid::Black` — the default — and wrong only for an
/// operator who has explicitly picked White. **That wrongness is what
/// [`Row::cards`] exists to end**, and a current host never reaches this
/// function.
///
/// **Aurora is the background that finally needed a different answer**, added
/// with T8. The six ported ids keep `false` — their value is unchanged and
/// deliberately so — but Aurora paints a lit gradient into every pixel and
/// has no black ground anywhere, so an old host that drew a plain card over
/// it would draw an unreadable one. Starfield, Contours and Life are all
/// black wherever they are not lit, which is most of them, so `false` is
/// right for those three for Digital Rain's reason.
fn prefers_dark_cards(background_id: &str) -> bool {
    background_id == "aurora"
}

/// **Where each background can be drawn** — `.rulings/` R-206 D2, which makes
/// this a declaration a host reads rather than a capability it discovers by
/// failing.
///
/// **R-207's three, and `Either` rather than `Gpu` for every one of them.**
/// R-206 D3 keeps the CPU rasteriser for the machine with no V3D, so each of
/// these can be asked either way and the host chooses.
///
/// The other four are `Cpu`, each for its own reason. Digital Rain and
/// Starfield are R-207's CPU ships — Starfield deliberately, because it
/// measured 3.86 ms on the GPU against 4.11 on the CPU and a tie is not a
/// reason to move. Aurora is R-207's skip. Solid is one colour and has nothing
/// to move.
///
/// **`gpu::tests::the_catalogue_does_not_promise_a_path_that_is_not_there`
/// holds this function to the code**: every id that claims `on_gpu` must
/// actually produce something to upload. A catalogue that over-promises is
/// believed — a host binds a framebuffer, asks, and reads the refusal as a
/// fault in the platform rather than as a pack describing itself wrongly.
fn drawn(background_id: &str) -> Drawn {
    match background_id {
        "contours" | "life" | "boxes" => Drawn::Either,
        _ => Drawn::Cpu,
    }
}

/// This pack's catalogue, grouped by background id — what
/// [`crate::BackgroundPack`]'s `Addon::describe` returns.
#[must_use]
pub fn describe() -> Vec<ProtoBackground> {
    let mut out: Vec<ProtoBackground> = Vec::new();
    for row in &ROWS {
        let theme = ProtoTheme {
            id: row.theme_id.to_string(),
            label: row.theme_label.to_string(),
            cards: row.cards,
            animated: row.animated,
        };
        if let Some(existing) = out.iter_mut().find(|b| b.id == row.background_id) {
            existing.themes.push(theme);
        } else {
            out.push(ProtoBackground {
                id: row.background_id.to_string(),
                name: row.background_label.to_string(),
                themes: vec![theme],
                prefers_dark_cards: prefers_dark_cards(row.background_id),
                drawn: drawn(row.background_id),
            });
        }
    }
    out
}

/// The value [`crate::render::render_frame`] draws for one `(background,
/// theme)` id pair — `crate::background::Screensaver` is built from this.
///
/// # Errors
///
/// [`Refusal::UnknownBackground`] when `background_id` names nothing in
/// [`ROWS`] at all; [`Refusal::UnknownTheme`] when it does but `theme_id`
/// does not — the same two-step distinction
/// `rivetos_addon::frame::REFUSED_BACKGROUND` / `REFUSED_THEME` carries
/// across the wire, so a host can tell the two apart.
pub fn resolve(background_id: &str, theme_id: &str) -> Result<Background, Refusal> {
    if !ROWS.iter().any(|r| r.background_id == background_id) {
        return Err(Refusal::UnknownBackground);
    }
    ROWS.iter()
        .find(|r| r.background_id == background_id && r.theme_id == theme_id)
        .map(|r| r.background)
        .ok_or(Refusal::UnknownTheme)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The catalogue groups by background id into exactly the seven this
    /// pack has, each with its own list of themes and no empty one — a host
    /// renders two pickers from this and an empty theme list would render one
    /// shape of picker for this background and another shape for the rest.
    ///
    /// **Seven and twenty-two since T8**, up from three and ten.
    #[test]
    fn the_catalogue_groups_into_seven_backgrounds_each_with_its_themes() {
        let backgrounds = describe();
        assert_eq!(backgrounds.len(), 7);
        assert_eq!(
            backgrounds.iter().map(|b| b.themes.len()).sum::<usize>(),
            ROWS.len(),
            "grouping lost or duplicated a row"
        );
        for background in &backgrounds {
            assert!(!background.themes.is_empty(), "{} has no theme", background.id);
            assert!(
                rivetos_addon::frame::id_bytes(&background.id).is_some(),
                "{} is not an id the frame descriptor can carry",
                background.id
            );
            for theme in &background.themes {
                assert!(
                    rivetos_addon::frame::id_bytes(&theme.id).is_some(),
                    "{}/{} is not an id the frame descriptor can carry",
                    background.id,
                    theme.id
                );
            }
        }
        let solid = backgrounds.iter().find(|b| b.id == "solid").expect("solid");
        assert_eq!(solid.themes.len(), 3);
        let rain = backgrounds
            .iter()
            .find(|b| b.id == "digital-rain")
            .expect("digital-rain");
        assert_eq!(rain.themes.len(), 3);
        let boxes = backgrounds.iter().find(|b| b.id == "boxes").expect("boxes");
        assert_eq!(boxes.themes.len(), 4);
        for id in ["starfield", "aurora", "contours", "life"] {
            let background = backgrounds.iter().find(|b| b.id == id).expect(id);
            assert_eq!(background.themes.len(), 3, "{id} should have three themes");
        }
        // **The attribution, asserted.** The maintainer asked for the long
        // name on 2026-09-20 and the argument for it is beside the rows; a
        // later tidy-up that shortened the label would otherwise pass every
        // other test in this file.
        let life = backgrounds.iter().find(|b| b.id == "life").expect("life");
        assert_eq!(life.name, "Conway's Game of Life");
    }

    /// Every id `resolve` accepts round-trips to the exact value the
    /// evidence baseline expects, and the two refusals are the right one
    /// each.
    #[test]
    fn resolve_answers_every_catalogue_entry_and_refuses_the_right_thing() {
        assert_eq!(resolve("solid", "navy"), Ok(Background::Solid(Solid::Navy)));
        assert_eq!(
            resolve("digital-rain", "born-this-way"),
            Ok(Background::DigitalRain(RainPalette::BornThisWay))
        );
        assert_eq!(resolve("boxes", "dark"), Ok(Background::Boxes(BoxPalette::Dark)));
        assert_eq!(
            resolve("life", "pulse"),
            Ok(Background::Life(LifeStyle::Pulse))
        );
        // **`starfield` used to be this test's unknown background**, chosen
        // when it was the most plausible thing this pack did not have. T8
        // built it, so the stand-in has to be something else — which is the
        // failure mode a test written against an absence has.
        assert_eq!(resolve("mandelbrot", "navy"), Err(Refusal::UnknownBackground));
        assert_eq!(resolve("solid", "phosphor-green"), Err(Refusal::UnknownTheme));
        assert_eq!(resolve("life", "born-this-way"), Err(Refusal::UnknownTheme));
    }

    /// **Only `solid` is still**, and it is the whole of what `animated`
    /// answers. A host reads this to decide whether to redraw a screen its
    /// own dirty flag says is unchanged, so a Boxes row that said `false`
    /// would freeze the wall and a Solid row that said `true` would redraw an
    /// unchanging picture thirty times a second all night.
    ///
    /// **Stated as a property over the table and not as a list of the ids
    /// that move**, which is why T8 added twelve rows without touching the
    /// assertion below: all four of its backgrounds animate, and a list would
    /// have had to be edited four times to say so. The eighth background this
    /// pack grows will be covered by this test on the day it is written.
    ///
    /// Note that `Life` steps four times a second rather than thirty and is
    /// still `animated`. The field answers *"does this picture change on its
    /// own"*, not *"does it change on every frame"* — the fade between steps
    /// changes every frame regardless, and a host told `false` would show a
    /// frozen board.
    #[test]
    fn solid_is_the_only_background_that_does_not_animate() {
        for row in &ROWS {
            let expected = row.background_id != "solid";
            assert_eq!(
                row.animated, expected,
                "{}/{} answers the wrong thing for `animated`",
                row.background_id, row.theme_id
            );
        }
    }

    /// **The card appearance is the renderer's `card_theme`, constant for
    /// constant** — the same standard §5 of the design set for the drawing
    /// itself. Three representative rows are spelled out in full rather than
    /// spot-checked on one field: a test asserting `bg` alone would pass for
    /// a port that dropped `caution`, which is exactly the kind of drift
    /// moving this table across could introduce.
    #[test]
    fn every_card_appearance_is_the_renderers_own() {
        let card = |background: &str, theme: &str| {
            ROWS.iter()
                .find(|r| r.background_id == background && r.theme_id == theme)
                .expect("a row this catalogue has")
                .cards
        };

        // `default: card = nullptr` — no rectangle, the widget's ink raw.
        assert_eq!(card("solid", "black"), Cards::NoCard);

        // `kCardIce` — blue-tinted ink on near-black at the load-bearing 155.
        assert_eq!(
            card("digital-rain", "ice-blue"),
            Cards::Card(CardAppearance {
                bg: [3, 5, 14],
                bg_alpha: 155,
                text: [190, 205, 247],
                rainbow: false,
                alert: [255, 69, 58],
                caution: [255, 197, 0],
            })
        );

        // `kBoxesIce` — a bright ground, so black ink and the LIGHT signal
        // colours. This is the pair a single `prefers_dark_cards` bool could
        // not express, because the other three Boxes themes are dark.
        assert_eq!(
            card("boxes", "ice-blue"),
            Cards::Card(CardAppearance {
                bg: [0, 0, 0],
                bg_alpha: 0,
                text: [0, 0, 0],
                rainbow: false,
                alert: [186, 26, 26],
                caution: [150, 95, 0],
            })
        );
    }

    /// **Both Born This Way rows are one card**, drawn over two different
    /// backgrounds — `widgets.rs` matched them with a single `|` arm and this
    /// table has to keep them equal by value now that it cannot by syntax.
    #[test]
    fn the_two_pride_rows_carry_the_same_card() {
        let rain = ROWS
            .iter()
            .find(|r| r.background_id == "digital-rain" && r.theme_id == "born-this-way")
            .expect("digital-rain/born-this-way");
        let boxes = ROWS
            .iter()
            .find(|r| r.background_id == "boxes" && r.theme_id == "born-this-way")
            .expect("boxes/born-this-way");
        assert_eq!(rain.cards, boxes.cards);
        let Cards::Card(card) = rain.cards else {
            panic!("the pride rows carry a card");
        };
        assert!(card.rainbow, "and it is the gradient one");
    }

    /// **This pack states a card for every row** — none is left to a host's
    /// fallback, because the fallback is a plain card the pack did not choose.
    #[test]
    fn every_row_states_its_card() {
        for row in &ROWS {
            assert!(
                !row.cards.is_unstated(),
                "{}/{} leaves its card to the host",
                row.background_id,
                row.theme_id
            );
        }
    }

    /// **No row asks for no card over a background that prefers dark cards.**
    ///
    /// That is the one combination a host built before `rivetos_addon::Cards`
    /// reads differently: it takes the `null` as *no opinion* and draws its
    /// dark fallback card. Every row this pack has today reads the same to an
    /// old host and a new one, and this holds the pack to that until somebody
    /// decides an old host's plainer picture is acceptable for a new row.
    #[test]
    fn no_row_asks_for_no_card_where_an_older_host_would_draw_one() {
        for row in &ROWS {
            if row.cards == Cards::NoCard {
                assert!(
                    !prefers_dark_cards(row.background_id),
                    "{}/{} asks for no card over a background an older host draws a card on",
                    row.background_id,
                    row.theme_id
                );
            }
        }
    }

    /// **Solid Black crosses as `"cards":null` and every other theme as an
    /// object** — the bytes this pack's `Describe` answer actually carries,
    /// read back as a host reads them.
    #[test]
    fn solid_black_crosses_as_null_and_every_other_theme_as_a_card() {
        let bytes = rivetos_addon::protocol::encode(rivetos_addon::Response::Described(
            rivetos_addon::Description {
                name: "aura-background-pack".to_string(),
                version: "0.0.0".to_string(),
                class: "aura-background".to_string(),
                fields: Vec::new(),
                material: Vec::new(),
                backgrounds: describe(),
            },
        ))
        .expect("encodes");
        let text = String::from_utf8(bytes.clone()).expect("JSON is text");
        assert!(
            text.contains(r#"{"id":"black","label":"Black","cards":null"#),
            "{text}"
        );
        let back: rivetos_addon::protocol::Envelope<rivetos_addon::Response> =
            rivetos_addon::protocol::decode(&bytes).expect("decodes");
        let rivetos_addon::Response::Described(d) = back.body else {
            panic!("not a description");
        };
        for background in &d.backgrounds {
            for theme in &background.themes {
                let expected_none = background.id == "solid" && theme.id == "black";
                match theme.cards {
                    Cards::NoCard => assert!(expected_none, "{}/{}", background.id, theme.id),
                    Cards::Card(_) => assert!(!expected_none, "{}/{}", background.id, theme.id),
                    Cards::Unstated => panic!("{}/{} said nothing", background.id, theme.id),
                }
            }
        }
    }

    /// **A bright ground gets the light signal colours, and only a bright
    /// ground does.** Stated as a property over the whole table rather than
    /// row by row, so a background added later cannot quietly pick red-on-white
    /// — which is the failure `Theme::cards` was added to make impossible for
    /// backgrounds the renderer carries no knowledge of, because it keeps no
    /// list of them.
    #[test]
    fn a_card_with_black_ink_carries_the_light_signal_colours() {
        for row in &ROWS {
            let Cards::Card(card) = row.cards else { continue };
            if card.rainbow {
                // The gradient card is its own case: black ink over six stops
                // that are all saturated, where the dark signal colours read.
                continue;
            }
            let ink_is_black = card.text == [0, 0, 0];
            let signals_are_light = card.alert == ALERT_LIGHT && card.caution == CAUTION_LIGHT;
            assert_eq!(
                ink_is_black, signals_are_light,
                "{}/{} mixes black ink with dark signal colours, or the reverse",
                row.background_id, row.theme_id
            );
        }
    }
}
