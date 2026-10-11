//! **The old screensaver vocabulary, translated once on the way in.**
//!
//! Until T7 this module folded two config keys into one of ten wire words —
//! *Digital Rain* + *Ice Blue* became `construct_ice` — and the renderer held
//! the matching table to unfold them. Between them those two tables were the
//! reason a new background could not be added without editing two programs,
//! which is exactly what `examples/aura/aura-background-pack` exists to end.
//!
//! So the fold is gone. `set_screensaver` now carries the background id and
//! the theme id the pack itself publishes, and neither program holds a list of
//! what those can be.
//!
//! # What is left here, and why it is not nothing
//!
//! **A config file on a board already holds the old spellings.** `matrix` and
//! `ice_blue` are in `config.json` on `rivet-01` right now, written by a
//! portal that spelled them that way. [`migrate`] translates them to
//! `digital-rain` and `ice-blue` as the file is read, and the next save writes
//! the new spelling.
//!
//! **Nothing is lost and nothing is rewritten behind anybody's back**: an
//! operator's saved choice survives an update, which is the whole requirement.
//! A value this table does not recognise passes through unchanged, because it
//! is either already a current id or one belonging to a pack this build has
//! never heard of — and in both cases the pack is the right judge, not this
//! function.
//!
//! **This table is closed.** It has ten entries and will never have eleven: a
//! background the pack grows has no old spelling, by definition. When no board
//! anywhere holds a pre-T7 `config.json`, the whole module can go.

/// Translate a saved `(screensaver, screensaver_theme)` pair into the ids the
/// pack publishes.
///
/// Unrecognised values pass through untouched — see the module note.
#[must_use]
pub fn migrate(name: &str, theme: &str) -> (String, String) {
    let background = match name {
        // The old portal's three families. `matrix` was always the display
        // name's opposite — `crate::portal` showed *Digital Rain* and stored
        // this — and the pack settled the question by publishing the id a
        // person would recognise.
        "matrix" => "digital-rain",
        "solid" | "boxes" => name,
        other => other,
    };

    let theme = match (name, theme) {
        // **`black_white` is the pre-T7 default and names no theme at all.**
        // It is what `Config::default` wrote, and the fold read it as *not
        // white and not navy*, i.e. black. Spelling it out rather than letting
        // it fall through is the difference between a board coming back on
        // Solid Black and coming back on a refused selection.
        ("solid", "black_white" | "" | "black") => "black",
        ("solid", "white") => "white",
        ("solid", "navy") => "navy",

        // Digital Rain's default was Classic Green, which the fold reached by
        // falling through.
        ("matrix", "ice_blue") => "ice-blue",
        ("matrix", "pride") => "born-this-way",
        ("matrix", _) => "phosphor-green",

        // Boxes' default was Ice Blue, likewise. `green` is the maintainer's
        // `Chartreuse`, renamed when the palettes were named in 2026-09-01 and
        // never renamed in the portal.
        ("boxes", "green") => "chartreuse",
        ("boxes", "pride") => "born-this-way",
        // **The current ids, listed before the fallback, and this is not
        // redundant.** `boxes` is its own id as well as an old family name, so
        // a config saved AFTER T7 comes through here again — and without these
        // arms `chartreuse` would hit `("boxes", _)` and be reset to Ice Blue
        // on every load. `migrating_an_already_migrated_pair_changes_nothing`
        // is the test that caught exactly that, and it caught it because it
        // asserts a property rather than the eleven old spellings.
        //
        // `solid` needs no such list because its three current ids — `black`,
        // `white`, `navy` — are each already named above. `digital-rain` needs
        // none because its name is not one of the three old family names, so a
        // migrated pair never reaches these arms at all.
        ("boxes", "ice-blue" | "chartreuse" | "dark" | "born-this-way") => theme,
        ("boxes", _) => "ice-blue",

        // A background this table does not know: its theme is not this
        // table's to interpret either.
        (_, other) => other,
    };

    (background.to_string(), theme.to_string())
}

/// `"on"` or `"off"`, which is what `show_weather` takes.
///
/// A helper rather than an inline conditional because it appears in three
/// places — the replay on connect, the scheduled action, and the portal — and
/// three spellings of the same two words is how one of them becomes `"true"`.
pub const fn on_off(flag: bool) -> &'static str {
    if flag { "on" } else { "off" }
}

#[cfg(test)]
mod tests;
