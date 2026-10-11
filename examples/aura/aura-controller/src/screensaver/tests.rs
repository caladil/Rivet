#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
use super::*;

/// **Every pair the old portal could save reaches the id the shipped pack
/// publishes.**
///
/// The expected values are `aura-background-pack`'s `catalogue.rs` `ROWS`,
/// read off that table. If this test and that table ever disagree, an operator
/// who updates a board comes back to a refused selection and a screen that
/// kept whatever was on it — which is a defect that appears on a wall days
/// after the update that caused it.
#[test]
fn every_saved_pair_migrates_to_a_real_id() {
    let cases = [
        // (old name, old theme, new background, new theme)
        ("solid", "black_white", "solid", "black"),
        ("solid", "", "solid", "black"),
        ("solid", "white", "solid", "white"),
        ("solid", "navy", "solid", "navy"),
        ("matrix", "", "digital-rain", "phosphor-green"),
        ("matrix", "ice_blue", "digital-rain", "ice-blue"),
        ("matrix", "pride", "digital-rain", "born-this-way"),
        ("boxes", "", "boxes", "ice-blue"),
        ("boxes", "green", "boxes", "chartreuse"),
        ("boxes", "dark", "boxes", "dark"),
        ("boxes", "pride", "boxes", "born-this-way"),
    ];
    for (name, theme, want_background, want_theme) in cases {
        let (background, theme_id) = migrate(name, theme);
        assert_eq!(
            (background.as_str(), theme_id.as_str()),
            (want_background, want_theme),
            "`{name}`/`{theme}` migrated wrong"
        );
    }
}

/// **`black_white` is the one that would have been missed.**
///
/// It is `Config::default`'s value, so it is what every board that never
/// changed its screensaver holds — and it names no theme at all. A migration
/// that let it fall through would send `solid`/`black_white` to a pack that
/// has `solid`/`black`, and the most common configuration in existence would
/// be the one that broke.
#[test]
fn the_default_config_value_is_not_left_to_fall_through() {
    assert_eq!(
        migrate("solid", "black_white"),
        ("solid".to_string(), "black".to_string())
    );
}

/// **A current id survives a second pass unchanged**, which is what makes this
/// safe to run on every read rather than once behind a version flag.
///
/// Without this property a config saved after T7 and read again would have its
/// ids re-translated, and `digital-rain` — not being one of the three old
/// family names — would take the fall-through arm for its theme.
#[test]
fn migrating_an_already_migrated_pair_changes_nothing() {
    for (background, theme) in [
        ("solid", "black"),
        ("solid", "white"),
        ("digital-rain", "phosphor-green"),
        ("digital-rain", "ice-blue"),
        ("digital-rain", "born-this-way"),
        ("boxes", "ice-blue"),
        ("boxes", "chartreuse"),
        ("boxes", "dark"),
        ("boxes", "born-this-way"),
    ] {
        assert_eq!(
            migrate(background, theme),
            (background.to_string(), theme.to_string()),
            "`{background}`/`{theme}` did not survive a second migration"
        );
    }
}

/// **A background id this build cannot recognise passes through.**
///
/// The point of T7: the controller renders its pickers from what the pack
/// describes, so an id it cannot recognise is the ordinary case rather than an
/// error. A migration that refused here would put back the coupling the whole
/// change removed.
#[test]
fn an_unknown_background_is_not_this_functions_business() {
    assert_eq!(
        migrate("starfield", "amber"),
        ("starfield".to_string(), "amber".to_string())
    );
}

#[test]
fn on_off_is_the_schemas_two_words() {
    assert_eq!(on_off(true), "on");
    assert_eq!(on_off(false), "off");
}
