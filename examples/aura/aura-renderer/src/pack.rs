//! **The background pack, hosted** — T7 of
//! `docs/PLAN-Addon-Feature-Set-2026-09-19.md`.
//!
//! Until 2026-09-20 this program carried its own copy of every background it
//! could draw, in `src/background.rs`, beside a second copy of the same code
//! in `examples/aura/aura-background-pack`. The pack was installed on the
//! board and **nothing loaded it**: it had not even compiled for a day and no
//! build gate noticed, because nothing depended on it. That is what this
//! module ends.
//!
//! # Two hosts, one object
//!
//! `aura-controller` opens the same class to read its catalogue and render two
//! pickers, and never draws a pixel. This program opens it to draw. Neither
//! knows about the other, and `Describe` is cheap and has no side effect, so
//! both asking is ordinary rather than something to coordinate —
//! `docs/design-candidates/Background-Addons-2026-09-19.md` §2.
//!
//! # What this program stopped knowing
//!
//! The ten-way table of card appearances in `widgets.rs` went with the
//! drawing. It had to: a renderer holding that table is a renderer that must
//! be edited before the pack may grow an eleventh background, which is the
//! duplication the pack exists to remove. `rivetos_addon::Theme::cards` is
//! where it lives now, per theme, which is the granularity the answer has —
//! see that field's own note.
//!
//! # The trial, and why a display program especially wants one
//!
//! A loaded addon runs in this address space. One that writes through a wild
//! pointer takes down **the process holding the DRM lease**, and the screen it
//! was drawing is the whole product. `rivetos_addon::trial` cannot prevent the
//! first crash — nothing at this boundary can — but withholding the
//! confirmation is the rollback, so the second start draws black and says why
//! instead of looping. `aura-controller/src/addon.rs` argues this at length
//! and this module follows it deliberately rather than inventing a second
//! shape for the same mechanism.

use rivetos_addon::frame::Scene;
use rivetos_addon::host::{Loaded, OpenFailure, Target};
use rivetos_addon::{CardAppearance, Cards, Description, Request, Response};

/// The class this program accepts. Named once.
///
/// **Not the library's name** — `aura-background-pack` is the artifact;
/// `aura-background` is the role it fills, and `ADR-053` joins by role. A
/// second pack with different backgrounds would provide the same class and
/// this program would need no change at all.
pub const CLASS: &str = "aura-background";

/// What this program starts drawing before a controller has said otherwise.
///
/// `cpp/src/main.cpp` starts at plain black and the portal is how a person
/// changes it; this port has run Digital Rain since 2026-09-02, because the
/// maintainer's complaint that opened the work was that *Digital Rain is
/// named but does not run*. The controller sends the real choice on connect,
/// so this is only ever seen for the fraction of a second before that — and
/// on a machine where no controller was granted at all.
pub const DEFAULT: Selection = Selection {
    background: "digital-rain",
    theme: "phosphor-green",
};

/// **A background and a theme, as ids.**
///
/// Borrowed rather than owned because the pair is either a constant in this
/// file or lives in the [`Description`] for as long as the pack is open; the
/// one case that needs an owned copy is a controller directive, which
/// `crate::controller` owns and hands over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    /// `digital-rain`.
    pub background: &'static str,
    /// `phosphor-green`.
    pub theme: &'static str,
}

/// **An open pack, and what it said about itself.**
pub struct Pack {
    loaded: Loaded,
    described: Description,
}

/// **What identifies the object [`Pack::open`] is about to load**, for
/// `rivetos_addon::trial::left_behind` to compare a stale note against — the
/// same helper `aura-controller/src/addon.rs`'s `Source::open` uses, and for
/// the same reason: it has to be knowable before the note is written, and the
/// note is written before the object is opened.
///
/// Exactly one match gets that object's own reference directory under
/// `/state/lib`. Zero or several fall back to the class's derived soname:
/// `Loaded::open` is about to refuse either on its own grounds and clear the
/// note right back off, so nothing downstream reads this identity then.
fn identity_for(class: &str, joined: &[rivetos_addon::host::Installed]) -> String {
    match joined {
        [only] => only.identity(),
        _ => rivetos_addon::abi::soname_for_class(class),
    }
}

/// Why no pack is drawing.
///
/// Two variants and not one, because they call for different sentences in the
/// journal and an operator acts on them differently: `Absent` is an ordinary
/// machine with no pack installed, and `Refused` is something wrong.
pub enum NotOpened {
    /// No library of this class is installed. **Not a fault** — `ADR-053`
    /// joins by class and says nothing when none matches, exactly as a machine
    /// with no reading source draws no corner reading card.
    Absent(String),
    /// A pack is there and could not be used.
    Refused(String),
}

impl std::fmt::Display for NotOpened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Absent(s) | Self::Refused(s) => f.write_str(s),
        }
    }
}

impl Pack {
    /// Write the trial note, open the pack, describe it, clear the note.
    ///
    /// The order is the mechanism — see the module header, and
    /// `aura-controller/src/addon.rs`'s `Source::open`, which this follows.
    ///
    /// # Errors
    ///
    /// [`NotOpened`].
    pub fn open() -> Result<Self, NotOpened> {
        let data_dir = rivet_sdk::app::data().map_err(|e| {
            NotOpened::Refused(format!(
                "no data directory, so the trial note has nowhere to go and no background pack \
                 will be loaded: {e}"
            ))
        })?;

        let joined = rivetos_addon::host::installed(CLASS);
        let identity = identity_for(CLASS, &joined);

        match rivetos_addon::trial::left_behind(&data_dir, CLASS, &identity) {
            rivetos_addon::trial::LeftBehind::Clear => {}
            rivetos_addon::trial::LeftBehind::Same => {
                return Err(NotOpened::Refused(format!(
                    "SKIPPED — the {CLASS} pack was being loaded when this renderer last \
                     stopped, so it is not loaded now and the screen will be black. That is the \
                     trial note doing its job: a pack that faults while drawing takes the \
                     process holding the display with it, and this is what stops the second \
                     one. Reinstalling the pack clears it."
                )));
            }
            rivetos_addon::trial::LeftBehind::Unidentified => {
                return Err(NotOpened::Refused(format!(
                    "SKIPPED — the {CLASS} pack was being loaded when this renderer last \
                     stopped, so it is not loaded now and the screen will be black. This note \
                     predates identity tracking, so a reinstall cannot be told apart from the \
                     crash that wrote it; delete {} to clear it.",
                    rivetos_addon::trial::note_path(&data_dir, CLASS).display()
                )));
            }
            rivetos_addon::trial::LeftBehind::Different => {
                if let Err(e) = rivetos_addon::trial::forgive(&data_dir, CLASS) {
                    return Err(NotOpened::Refused(format!(
                        "REFUSED a stale trial note for {CLASS} names an object other than what \
                         is installed now, and could not be cleared to load the new one: {e}"
                    )));
                }
            }
        }

        rivetos_addon::trial::begin(&data_dir, CLASS, &identity).map_err(|e| {
            NotOpened::Refused(format!(
                "REFUSED to load the {CLASS} pack without recording it first: {e}"
            ))
        })?;

        let settle = |data_dir: &std::path::Path| {
            let _ = rivetos_addon::trial::settled(data_dir, CLASS);
        };

        let loaded = match Loaded::open(CLASS) {
            Ok(loaded) => loaded,
            Err(e @ OpenFailure::NotLoaded { .. }) => {
                settle(&data_dir);
                return Err(NotOpened::Absent(format!("{e}")));
            }
            Err(e) => {
                settle(&data_dir);
                return Err(NotOpened::Refused(format!("{e}")));
            }
        };

        // **A pack that cannot draw is refused here rather than on the first
        // frame.** `Loaded::draws` reads the object's own frame-ABI symbol, so
        // this catches a reading source installed under this class — which
        // would otherwise describe itself perfectly and then refuse every
        // frame, thirty times a second, with the screen black throughout.
        if !loaded.draws() {
            settle(&data_dir);
            return Err(NotOpened::Refused(format!(
                "REFUSED — {} provides {CLASS} but exports no frame entry point, so it cannot \
                 draw. A reading source installed under a drawing class is the usual cause",
                loaded.soname()
            )));
        }

        let described = match loaded.call(&Request::Describe) {
            Ok(Response::Described(d)) => d,
            Ok(Response::Fault(f)) => {
                settle(&data_dir);
                return Err(NotOpened::Refused(format!(
                    "{} could not describe itself: {} — {}",
                    loaded.soname(),
                    f.kind,
                    f.sentence
                )));
            }
            Ok(other) => {
                settle(&data_dir);
                return Err(NotOpened::Refused(format!(
                    "{} answered Describe with {other:?}",
                    loaded.soname()
                )));
            }
            Err(e) => {
                settle(&data_dir);
                return Err(NotOpened::Refused(format!(
                    "{} could not be asked to describe itself: {e}",
                    loaded.soname()
                )));
            }
        };

        if described.class != CLASS {
            settle(&data_dir);
            return Err(NotOpened::Refused(format!(
                "REFUSED — {} was opened as {CLASS} and calls itself {}",
                loaded.soname(),
                described.class
            )));
        }

        // **A pack with an empty catalogue is refused.** It would leave every
        // selection unknown and every frame black, and the sentence an
        // operator needs is this one rather than a thousand refusals a minute.
        if described.backgrounds.is_empty() {
            settle(&data_dir);
            return Err(NotOpened::Refused(format!(
                "REFUSED — {} describes no backgrounds at all, so there is nothing to draw",
                loaded.soname()
            )));
        }

        if let Err(e) = rivetos_addon::trial::settled(&data_dir, CLASS) {
            return Err(NotOpened::Refused(format!(
                "{} loaded and answered, but the trial note could not be cleared, so the next \
                 start will skip it: {e}",
                loaded.soname()
            )));
        }

        Ok(Self { loaded, described })
    }

    /// What this pack can draw — what the journal reports at start, and what
    /// a selection is checked against.
    pub const fn described(&self) -> &Description {
        &self.described
    }

    /// The library this pack came from, for the journal.
    pub fn soname(&self) -> &str {
        self.loaded.soname()
    }

    /// **What a person would call this pair**, or `None` when the pack does
    /// not have it. `Digital Rain / Ice Blue`.
    pub fn label(&self, background: &str, theme: &str) -> Option<String> {
        let b = self.described.backgrounds.iter().find(|b| b.id == background)?;
        let t = b.themes.iter().find(|t| t.id == theme)?;
        Some(format!("{} / {}", b.name, t.label))
    }

    /// **The card a host should draw over this pair**, and whether the pair
    /// animates — [`appearance_in`], over what this pack described.
    pub fn appearance(&self, background: &str, theme: &str) -> Option<Appearance> {
        appearance_in(&self.described, background, theme)
    }

    /// **Whether this object has a GPU entry point at all** — the symbol,
    /// not the catalogue.
    ///
    /// Two different questions, and a host needs both: this one says whether
    /// `draw_gl` may be called, and [`Appearance::on_gpu`] says whether the
    /// chosen background wants it. `.rulings/` R-207's pack answers `true`
    /// here and `false` for four of its seven backgrounds.
    pub fn draws_gl(&self) -> bool {
        self.loaded.draws_gl()
    }

    /// **Draw one frame into the framebuffer the host has already bound.**
    ///
    /// The caller owns the EGL context and the target — `.rulings/` R-206 D1
    /// — so this composes a descriptor and nothing else.
    ///
    /// # Errors
    ///
    /// A sentence for the journal, on the same terms as [`Pack::draw`]: the
    /// caller must not log it every frame.
    pub fn draw_gl(
        &self,
        target: &rivetos_addon::GlTarget,
        selection: (&str, &str),
        time_ms: u64,
    ) -> Result<(), String> {
        let scene = Scene {
            background: selection.0,
            theme: selection.1,
            time_ms,
        };
        self.loaded
            .draw_gl(target, &scene)
            .map_err(|e| format!("{} refused the GPU frame: {e}", self.loaded.soname()))
    }

    /// Whether the pack has this pair at all.
    pub fn has(&self, background: &str, theme: &str) -> bool {
        self.described
            .backgrounds
            .iter()
            .any(|b| b.id == background && b.themes.iter().any(|t| t.id == theme))
    }

    /// **Draw one frame into `canvas`.**
    ///
    /// `time_ms` is this host's clock and the pack derives its own `dt` from
    /// consecutive values, which is what lets a pack that steps a simulation
    /// at four steps a second do so without this program knowing.
    ///
    /// # Errors
    ///
    /// A sentence for the journal. **The caller must not log it every frame** —
    /// thirty a second buries everything else in a rolling ~1 MB journal
    /// (`ADR-022` §8), which is the trap `rivet-your-own-queries-flood-the-
    /// journal` names one level up.
    pub fn draw(
        &self,
        canvas: &mut [u8],
        width: u32,
        height: u32,
        stride: u32,
        selection: (&str, &str),
        time_ms: u64,
    ) -> Result<(), String> {
        let mut target = Target::new(
            canvas,
            width,
            height,
            stride,
            rivetos_addon::frame::FORMAT_XRGB8888,
        )
        .map_err(|e| format!("the canvas is not a surface this frame can use: {e}"))?;
        let scene = Scene {
            background: selection.0,
            theme: selection.1,
            time_ms,
        };
        self.loaded
            .draw(&mut target, &scene)
            .map_err(|e| format!("{} refused the frame: {e}", self.loaded.soname()))
    }
}

/// **The card a host should draw over a pair, and whether the pair animates**
/// — the two facts this program used to answer for itself.
///
/// `None` for the whole pair when the pack does not have it, so *unknown* is
/// never mistaken for *no card*.
///
/// # Three answers from the pack
///
/// The pack's [`Cards`] is taken as it says (`rivetos_addon::Theme::cards`
/// has the three states):
///
/// * a card is drawn as described;
/// * **no card** (`null` on the wire) draws no rectangle and leaves the
///   widgets' ink as it is — the C++'s `card = nullptr`, and what Solid Black
///   asks for — **whatever the background prefers**;
/// * **no opinion** (the key omitted, as a pack built before `Theme::cards`
///   sends it) falls back to the older `Background::prefers_dark_cards`.
///
/// # The fallback, and why it is not silently a light card
///
/// Rather than invent an appearance for a pack with no opinion, this builds
/// the plainest card that honours `prefers_dark_cards`. The picture is duller
/// than a current pack's and it is legible, which is the right direction to
/// fail in.
fn appearance_in(described: &Description, background: &str, theme: &str) -> Option<Appearance> {
    let b = described.backgrounds.iter().find(|b| b.id == background)?;
    let t = b.themes.iter().find(|t| t.id == theme)?;
    let card = match t.cards {
        Cards::Card(card) => Some(card),
        Cards::NoCard => None,
        Cards::Unstated if b.prefers_dark_cards => Some(FALLBACK_DARK_CARD),
        Cards::Unstated => None,
    };
    Some(Appearance {
        card,
        animated: t.animated,
        on_gpu: b.drawn.on_gpu(),
    })
}

/// What a host needs to know about a pair beyond how to draw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Appearance {
    /// The card, or `None` for no rectangle at all.
    pub card: Option<CardAppearance>,
    /// Whether the screen changes when nothing else has — see
    /// `rivetos_addon::Theme::animated`.
    pub animated: bool,
    /// **Whether this background can be drawn through the GPU boundary** —
    /// `rivetos_addon::Drawn::on_gpu`, which is per BACKGROUND rather than
    /// per pack (`.rulings/` R-206 D2).
    ///
    /// A host with no GPU surface ignores it. One with a surface uses it to
    /// choose between `Pack::draw_gl` and `Pack::draw`, which is the whole of
    /// R-207 as a run-time decision: Contours, Life and Boxes say `true` and
    /// Digital Rain, Starfield, Aurora and Solid say `false`, in one pack,
    /// switchable by an operator while the machine runs.
    pub on_gpu: bool,
}

/// **The plainest legible dark card**, for a pack too old to describe one.
///
/// Deliberately not a copy of one of Aura's ten: those belong to the pack now,
/// and reproducing `kCardIce` here would recreate the table this program just
/// stopped holding. This is a neutral card that reads, and nothing more.
const FALLBACK_DARK_CARD: CardAppearance = CardAppearance {
    bg: [0, 0, 0],
    bg_alpha: 155,
    text: [235, 235, 235],
    rainbow: false,
    alert: [255, 69, 58],
    caution: [255, 197, 0],
};

#[cfg(test)]
mod tests {
    use super::*;
    use rivetos_addon::{Background, Theme};

    /// The same property `aura-controller/src/addon.rs`'s test of its own
    /// `identity_for` pins: one joined library gets its own reference
    /// directory, and that string changes when the reference does.
    #[test]
    fn identity_for_one_joined_library_is_its_own_reference() {
        let joined = vec![rivetos_addon::host::Installed {
            class: CLASS.to_string(),
            path: std::path::PathBuf::from(
                "/state/lib/aura-background-pack-1.0.0-aaaa/lib/libaura-background.so.1",
            ),
        }];
        assert_eq!(
            identity_for(CLASS, &joined),
            "aura-background-pack-1.0.0-aaaa"
        );
    }

    /// Neither zero nor several joined libraries panics or invents a wrong
    /// answer — `Loaded::open` is about to refuse both on its own grounds.
    #[test]
    fn identity_for_anything_but_one_joined_library_falls_back_to_the_soname() {
        assert_eq!(
            identity_for(CLASS, &[]),
            rivetos_addon::abi::soname_for_class(CLASS)
        );
        let two = vec![
            rivetos_addon::host::Installed {
                class: CLASS.to_string(),
                path: std::path::PathBuf::from("/state/lib/a/lib/libaura-background.so.1"),
            },
            rivetos_addon::host::Installed {
                class: CLASS.to_string(),
                path: std::path::PathBuf::from("/state/lib/b/lib/libaura-background.so.1"),
            },
        ];
        assert_eq!(
            identity_for(CLASS, &two),
            rivetos_addon::abi::soname_for_class(CLASS)
        );
    }

    fn described(themes: Vec<Theme>, prefers_dark: bool) -> Description {
        Description {
            name: "test-pack".to_string(),
            version: "0.1.0".to_string(),
            class: CLASS.to_string(),
            fields: Vec::new(),
            material: Vec::new(),
            backgrounds: vec![Background {
                id: "digital-rain".to_string(),
                name: "Digital Rain".to_string(),
                themes,
                prefers_dark_cards: prefers_dark,
                drawn: rivetos_addon::protocol::Drawn::Cpu,
            }],
        }
    }

    /// `Pack` holds a `Loaded`, which needs a real object — so the lookups are
    /// tested through [`appearance_in`], the function `Pack::appearance`
    /// calls, against a `Description` directly.
    fn appearance_of(d: &Description, background: &str, theme: &str) -> Option<Appearance> {
        appearance_in(d, background, theme)
    }

    /// A `Describe` answer as a pack sends it, decoded the way `Loaded::call`
    /// decodes one. Hand-composed: the spellings are the three
    /// `rivetos_addon::Cards` documents, over a background that prefers dark
    /// cards — the one background where *no card* and *no opinion* differ.
    fn described_from_bytes() -> Description {
        let bytes = br#"{"v":1,"answer":"described","name":"test-pack","version":"0.1.0",
            "class":"aura-background","backgrounds":[{"id":"aurora","name":"Aurora",
            "prefers_dark_cards":true,"themes":[
              {"id":"silent","label":"Silent","animated":true},
              {"id":"bare","label":"Bare","cards":null,"animated":true},
              {"id":"ember","label":"Ember","animated":true,"cards":{"bg":[10,5,3],
                "bg_alpha":235,"text":[250,225,205],"rainbow":false,
                "alert":[255,69,58],"caution":[255,197,0]}}]}]}"#;
        let back: rivetos_addon::protocol::Envelope<Response> =
            rivetos_addon::protocol::decode(bytes).expect("decodes");
        match back.body {
            Response::Described(d) => d,
            other => panic!("not a description: {other:?}"),
        }
    }

    /// **An explicit `null` draws no card, even over a background that
    /// prefers dark ones.** This is the answer a pack had no way to give
    /// while `null` and an omitted key read the same.
    #[test]
    fn an_explicit_null_draws_no_card_over_a_background_that_prefers_dark() {
        let d = described_from_bytes();
        let got = appearance_of(&d, "aurora", "bare").expect("the pair");
        assert_eq!(got.card, None);
        assert!(got.animated);
    }

    /// **An omitted `cards` over the same background still takes the
    /// fallback** — an older pack's picture does not change.
    #[test]
    fn an_omitted_card_over_the_same_background_still_takes_the_fallback() {
        let d = described_from_bytes();
        let got = appearance_of(&d, "aurora", "silent").expect("the pair");
        assert_eq!(got.card, Some(FALLBACK_DARK_CARD));
    }

    /// **A described card wins over the background's preference**, from bytes.
    #[test]
    fn a_described_card_from_bytes_is_taken_as_given() {
        let d = described_from_bytes();
        let got = appearance_of(&d, "aurora", "ember").expect("the pair");
        assert_eq!(
            got.card,
            Some(CardAppearance {
                bg: [10, 5, 3],
                bg_alpha: 235,
                text: [250, 225, 205],
                rainbow: false,
                alert: [255, 69, 58],
                caution: [255, 197, 0],
            })
        );
    }

    /// **A current pack's own card is used verbatim.**
    #[test]
    fn a_described_card_is_taken_as_given() {
        let card = CardAppearance {
            bg: [3, 5, 14],
            bg_alpha: 155,
            text: [190, 205, 247],
            rainbow: false,
            alert: [255, 69, 58],
            caution: [255, 197, 0],
        };
        let d = described(
            vec![Theme {
                id: "ice-blue".to_string(),
                label: "Ice Blue".to_string(),
                cards: Cards::Card(card),
                animated: true,
            }],
            false,
        );
        let got = appearance_of(&d, "digital-rain", "ice-blue").expect("the pair");
        assert_eq!(got.card, Some(card));
        assert!(got.animated);
    }

    /// **An older pack that says only `prefers_dark_cards` gets a legible
    /// card, not no card.** The wrong direction here is a light card over a
    /// bright background, which is unreadable; a plain dark one is merely
    /// duller than the pack would have chosen.
    #[test]
    fn a_pack_with_no_card_but_a_dark_preference_gets_the_fallback() {
        let d = described(
            vec![Theme {
                id: "ice-blue".to_string(),
                label: "Ice Blue".to_string(),
                cards: Cards::Unstated,
                animated: false,
            }],
            true,
        );
        let got = appearance_of(&d, "digital-rain", "ice-blue").expect("the pair");
        assert_eq!(got.card, Some(FALLBACK_DARK_CARD));
    }

    /// **No card, asked for, is no card at all** — the C++'s
    /// `card = nullptr`. Solid Black depends on it: a card drawn there would
    /// add a rectangle its design does not have.
    #[test]
    fn a_pack_asking_for_no_card_draws_no_card() {
        let d = described(
            vec![Theme {
                id: "black".to_string(),
                label: "Black".to_string(),
                cards: Cards::NoCard,
                animated: false,
            }],
            false,
        );
        let got = appearance_of(&d, "digital-rain", "black").expect("the pair");
        assert_eq!(got.card, None);
    }

    /// **No opinion and no dark preference is no card too** — the older
    /// pack's Solid Black, which said nothing and relied on the fallback.
    #[test]
    fn a_pack_with_no_opinion_and_no_preference_draws_no_card() {
        let d = described(
            vec![Theme {
                id: "black".to_string(),
                label: "Black".to_string(),
                cards: Cards::Unstated,
                animated: false,
            }],
            false,
        );
        let got = appearance_of(&d, "digital-rain", "black").expect("the pair");
        assert_eq!(got.card, None);
    }

    /// A pair the pack does not have is `None` for the whole appearance, so a
    /// caller cannot mistake *unknown* for *no card*.
    #[test]
    fn an_unknown_pair_has_no_appearance() {
        let d = described(Vec::new(), false);
        assert!(appearance_of(&d, "digital-rain", "ice-blue").is_none());
        assert!(appearance_of(&d, "starfield", "white").is_none());
    }
}
