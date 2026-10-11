//! **The two pages, and the escaping that keeps them safe.**
//!
//! # Placeholders, not a template engine
//!
//! `assets/index.html` and `assets/schedule_edit.html` are the Go templates
//! with every `{{.Action}}` replaced by a `{{NAME}}` marker, and [`fill`]
//! substitutes. It is a deliberately small mechanism: there are no loops and no
//! conditionals in the files, because everything that loops or branches is
//! built here in Rust where it can be tested.
//!
//! The alternative was a template crate. It would have meant a second syntax to
//! be wrong in, a build step, and — for a page whose only repetition is one
//! table — nothing gained.
//!
//! # Everything that reaches a page goes through [`escape`]
//!
//! A zip code, a sensor name, a TempMon URL and an addon's account name all
//! arrive from a form and all land in HTML — and so does an addon's own name,
//! which titles a section. There is no *this one is safe* case here, and the
//! escaping is applied at the point of substitution rather than at the
//! point of storage — because the config is also written to JSON, sent over a
//! socket, and put in log lines, and a value escaped for one of those is wrong
//! in the others.
//!
//! # The links are RELATIVE, and that is `ADR-046` §8's first bullet
//!
//! `rivet-proxyd` mounts this at `/aura/` and **strips the prefix**, so this
//! program never learns its own base path — `R-133`'s two-sided declaration is
//! recorded and unbuilt. An absolute `/config` would therefore be wrong the
//! moment the portal is reached through the proxy rather than on its own port.
//! Relative links resolve against whatever the browser is showing, which is the
//! one thing that is right in both cases.
//!
//! **It is why the schedule editor takes `?index=`** and not the Go routes'
//! `/schedule/edit/3`: every page then sits at the same depth, so one `../`
//! works from all of them. A page a directory deeper would need a different
//! prefix in its stylesheet link, which is the kind of thing that is wrong only
//! when reached the other way.

use rivetos_addon::Description;

use super::schema;
use crate::config::{Config, ScheduledAction, ScheduledEvent};

/// The page a person configures Aura from.
const INDEX: &str = include_str!("assets/index.html");
/// The schedule event editor.
const EDITOR: &str = include_str!("assets/schedule_edit.html");
/// Served at `static/style.css`, byte for byte as the Go portal serves it.
pub const STYLE: &str = include_str!("assets/style.css");

/// The three-letter days a schedule is written in, in the order the editor
/// draws them — **Monday first**, unlike `crate::clock`'s Sunday-first table,
/// because that is how a person reads a week.
const EDITOR_DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// **The heading of the corner reading card's section**, and the word used for
/// it everywhere else on the page.
///
/// The addon's own name when one is installed and it gave one — `Describe`
/// carries a product name for a person and nothing more specific — and
/// otherwise the neutral word `Reading`. **The host has no word of its own for
/// what the card shows** (`.rulings/` R-375 D1), so nothing in this file or in
/// `assets/index.html` names it; a heading that did would be the host knowing.
///
/// Returned unescaped, like every other value here: [`fill`]'s callers escape
/// at the point of substitution.
#[must_use]
pub fn reading_title(description: Option<&Description>) -> String {
    description
        .map(|d| d.name.trim())
        .filter(|name| !name.is_empty())
        .map_or_else(|| "Reading".to_string(), str::to_string)
}

/// HTML-escape, for text and for attribute values alike.
///
/// Both single and double quotes, because the templates use both kinds and a
/// helper that only handled one would be safe in the file it was written for
/// and unsafe in the next one.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Replace every `{{NAME}}` in `template` with its value.
///
/// **One pass, so a substituted value cannot be substituted into.** Escaped
/// text can legitimately contain a brace, and a naive repeated `replace` would
/// treat a person's sensor name as a marker.
fn fill(template: &str, values: &[(&str, String)]) -> String {
    let mut out = String::with_capacity(template.len() + 2048);
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            break;
        };
        let name = &after[..close];
        let replacement = values
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str());
        match replacement {
            Some(value) => {
                out.push_str(&rest[..open]);
                out.push_str(value);
            }
            // **A marker nobody filled is left as written**, not blanked. It is
            // visible on the page, which is how it gets noticed and fixed; a
            // silent empty string is how a field goes missing for a month.
            None => out.push_str(&rest[..open + 2 + close + 2]),
        }
        rest = &after[close + 2..];
    }
    out.push_str(rest);
    out
}

/// `selected` or nothing, for an `<option>`.
fn selected(current: &str, value: &str) -> String {
    if current == value { "selected" } else { "" }.to_string()
}

/// `checked` or nothing, for a checkbox.
fn checked(flag: bool) -> String {
    if flag { "checked" } else { "" }.to_string()
}

/// How a screensaver and its theme read on the dashboard.
///
/// **From the pack's own catalogue since T7**, where it used to be a ten-arm
/// table here. The pack publishes a display name for every background and
/// every theme; a portal that translated ids to names itself could not name
/// one the pack grew after this binary was built, which is the case the whole
/// change exists for.
///
/// A pair the catalogue does not carry falls back to the raw ids. That is not
/// a failure worth a special sentence on a dashboard: it happens for the few
/// seconds between saving a selection and the pack that has it being
/// installed, and it reads as `starfield / amber` rather than as an error.
fn screensaver_label(catalogue: Option<&Description>, name: &str, theme: &str) -> String {
    if name.is_empty() {
        return "—".to_string();
    }
    let named = catalogue.and_then(|d| {
        let background = d.backgrounds.iter().find(|b| b.id == name)?;
        let theme = background.themes.iter().find(|t| t.id == theme)?;
        Some(format!("{} — {}", background.name, theme.label))
    });
    match named {
        Some(label) => escape(&label),
        None if theme.is_empty() => escape(name),
        None => escape(&format!("{name} / {theme}")),
    }
}

/// What stands in the picker's place on a machine with no background pack.
const NO_PACK: &str = "<p class=\"hint\">No background pack is installed on this machine, so \
                       the screen stays black. Install the <code>aura-background</code> library \
                       to choose a screensaver.</p>";

/// **The two `<select>`s, built from the catalogue** — what fills
/// `{{SCREENSAVER_FIELDS}}`.
///
/// No `<select>` at all for a machine with no pack installed (or a pack that
/// describes no backgrounds): the renderer on such a machine draws black and
/// has nothing to choose between, so offering a picker would be offering
/// settings that reach a program which cannot act on them. One sentence
/// stands in its place saying so and naming the library to install, so the
/// missing picker is explained rather than silently absent.
///
/// The theme `<select>` is left empty here and filled by the page's own
/// `updateTheme()`, from `{{SCREENSAVER_THEMES_JSON}}`, because which themes
/// are legal depends on which background is selected *in the browser* — a
/// thing the server cannot know between requests.
fn screensaver_fields(catalogue: Option<&Description>, config: &Config) -> String {
    let Some(description) = catalogue.filter(|d| !d.backgrounds.is_empty()) else {
        return NO_PACK.to_string();
    };

    let mut options = String::new();
    for background in &description.backgrounds {
        options.push_str(&format!(
            "<option value=\"{}\" {}>{}</option>",
            escape(&background.id),
            selected(&config.screensaver, &background.id),
            escape(&background.name),
        ));
    }

    format!(
        "<div class=\"field\">\n\
         <label for=\"screensaver\">Screensaver</label>\n\
         <select id=\"screensaver\" name=\"screensaver\" \
         onchange=\"updateTheme()\">{options}</select>\n\
         </div>\n\
         <div class=\"field\" id=\"theme-field\">\n\
         <label for=\"screensaver_theme\" id=\"theme-label\">Theme</label>\n\
         <select id=\"screensaver_theme\" name=\"screensaver_theme\" \
         data-current=\"{}\"></select>\n\
         </div>",
        escape(&config.screensaver_theme),
    )
}

/// **The catalogue as the page's own `ssThemes` object** — what fills
/// `{{SCREENSAVER_THEMES_JSON}}`.
///
/// `{ "digital-rain": [{"v":"ice-blue","l":"Ice Blue"}, …], … }`.
///
/// Built with `serde_json` rather than by concatenating strings, because a
/// theme label is a pack author's text going into a `<script>` block, where
/// HTML escaping is the wrong escaping and `</script>` inside a string would
/// end the block. `serde_json` escapes for JavaScript, and the `/` in any
/// closing tag is escaped below so the parser cannot see one.
fn screensaver_themes_json(catalogue: Option<&Description>) -> String {
    let Some(description) = catalogue else {
        return "{}".to_string();
    };
    let mut map = serde_json::Map::new();
    for background in &description.backgrounds {
        let themes: Vec<serde_json::Value> = background
            .themes
            .iter()
            .map(|theme| {
                serde_json::json!({ "v": theme.id, "l": theme.label })
            })
            .collect();
        map.insert(background.id.clone(), serde_json::Value::Array(themes));
    }
    // **`</script>` cannot be allowed to appear**, whatever a pack calls its
    // themes. Escaping the slash is valid JSON and valid JavaScript and means
    // the HTML parser never sees a closing tag.
    serde_json::to_string(&serde_json::Value::Object(map))
        .unwrap_or_else(|_| "{}".to_string())
        .replace("</", "<\\/")
}

/// A schedule row's days, as a person reads them.
fn format_days(days: &[String]) -> String {
    if days.is_empty() {
        return "Every day".to_string();
    }
    escape(&days.join(", "))
}

/// A schedule row's actions, as a person reads them.
fn format_actions(catalogue: Option<&Description>, actions: &[ScheduledAction]) -> String {
    let parts: Vec<String> = actions
        .iter()
        .map(|action| match action.kind.as_str() {
            // **The same naming the dashboard uses, through the same
            // function.** This arm held its own two tables — a third and
            // fourth copy of the pack's catalogue, in the same file as the
            // first — and they had already drifted: `green` read as *Matrix
            // Green* here and the palette had been named *Chartreuse* since
            // 2026-09-01.
            "screensaver" => format!(
                "Screensaver: {}",
                screensaver_label(catalogue, &action.value, &action.theme)
            ),
            "brightness" => format!("Brightness: {}%", escape(&action.value)),
            "display" => {
                let label = match action.value.as_str() {
                    "on" | "wake" => "On",
                    "off" | "sleep" => "Off",
                    other => other,
                };
                format!("Display: {}", escape(label))
            }
            "weather" => {
                let label = match action.value.as_str() {
                    "on" => "On",
                    "off" => "Off",
                    other => other,
                };
                format!("Weather: {}", escape(label))
            }
            other => format!("{}: {}", escape(other), escape(&action.value)),
        })
        .collect();
    parts.join("; ")
}

/// The schedule table, or the line that says there is nothing in it.
fn schedule_table(catalogue: Option<&Description>, events: &[ScheduledEvent]) -> String {
    if events.is_empty() {
        return "<p class=\"sched-empty\">No events. Add one to automate the screensaver, \
                the brightness and the weather panel.</p>"
            .to_string();
    }
    let mut out = String::from(
        "<table class=\"sched-table\">\n\
         <thead><tr><th>Time</th><th>Days</th><th>Actions</th><th></th></tr></thead>\n\
         <tbody>\n",
    );
    for (index, event) in events.iter().enumerate() {
        out.push_str(&format!(
            "<tr>\n\
             <td><strong>{time}</strong></td>\n\
             <td>{days}</td>\n\
             <td>{actions}</td>\n\
             <td class=\"sched-actions\">\n\
             <a href=\"schedule/edit?index={index}\" title=\"Edit\">\
             <button class=\"btn-icon\" type=\"button\" aria-label=\"Edit event\">\
             <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"14\" height=\"14\" \
             viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" \
             stroke-linecap=\"round\" stroke-linejoin=\"round\">\
             <path d=\"M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7\"/>\
             <path d=\"M18.5 2.5a2.121 2.121 0 0 1 3 3L12 15l-4 1 1-4 9.5-9.5z\"/></svg>\
             </button></a>\n\
             <form method=\"POST\" action=\"schedule/delete?index={index}\" \
             style=\"display:inline\">\
             <button class=\"btn-icon danger\" type=\"submit\" aria-label=\"Delete event\" \
             onclick=\"return confirm('Delete this event?')\">\
             <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"14\" height=\"14\" \
             viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" \
             stroke-linecap=\"round\" stroke-linejoin=\"round\">\
             <polyline points=\"3 6 5 6 21 6\"/>\
             <path d=\"M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2\"/>\
             <line x1=\"10\" y1=\"11\" x2=\"10\" y2=\"17\"/>\
             <line x1=\"14\" y1=\"11\" x2=\"14\" y2=\"17\"/></svg>\
             </button></form>\n\
             </td>\n</tr>\n",
            time = escape(&event.time),
            days = format_days(&event.days),
            actions = format_actions(catalogue, &event.actions),
        ));
    }
    out.push_str("</tbody>\n</table>\n");
    out
}

/// What a person sees at `/`.
///
/// `description` is what the reading card's addon `Describe` answered, cached
/// for the life of the load — `None` when no addon of that class is open, which
/// is either a machine nobody has installed one on or the brief window before
/// it answers. `current` is that addon's own settings, keyed by
/// [`rivetos_addon::Field::key`], and where they came from is
/// **`crate::addon`'s business and not this function's**: everything here
/// does with them is hand them to [`schema::render_fields`], which knows
/// [`rivetos_addon::FieldKind`] and nothing else — see that module's note on
/// why that is the whole point of it.
///
/// Eleven arguments, because the page draws from that many separate sources —
/// the stored configuration, two addons' descriptions and settings, a hint
/// when something about the indoor one could not be resolved, the background
/// pack, the renderer's link, and the request's flash, section and zone — and
/// a struct gathering them would name a thing that is not a thing.
#[allow(clippy::too_many_arguments)]
pub fn dashboard(
    config: &Config,
    description: Option<&Description>,
    current: &serde_json::Map<String, serde_json::Value>,
    indoor_description: Option<&Description>,
    indoor_current: &serde_json::Map<String, serde_json::Value>,
    // **`.rulings/` R-219.** The message
    // `crate::config::effective_tempmon_name` refused with, or `None` when
    // it resolved one — never shown merely because an operator's own entry
    // is absent for a machine with no addon installed at all
    // (`INDOOR_UNAVAILABLE` below is that case). `crate::portal` is the one
    // place that calls that function and reads `rivet_sdk::facts` through
    // it; this function only renders what it decided.
    indoor_name_hint: Option<&str>,
    // `backgrounds` is what the background pack described, or `None` on a
    // machine with no pack installed (T7). Every screensaver name, theme name
    // and option on this page comes from it and from nowhere else.
    backgrounds: Option<&Description>,
    renderer_connected: bool,
    flash: &str,
    section: &str,
    zone: &str,
) -> String {
    let events = config.schedule.len();
    // `.rulings/` R-219: the one predicate that decides whether the indoor
    // addon's own `verify_as` control is shown at all —
    // `crate::config::tempmon_name_is_asked`, the same question the page's
    // own `syncTempmonNameField` script answers for the live URL field.
    // Computed here rather than threaded in as a twelfth argument, because
    // `config` — and so `config.tempmon_url` — is already in hand.
    let tempmon_name_hidden = !crate::config::tempmon_name_is_asked(&config.tempmon_url);
    let hidden_indoor_keys: &[&str] = if tempmon_name_hidden { &["verify_as"] } else { &[] };
    let title = escape(&reading_title(description));

    let values: Vec<(&str, String)> = vec![
        (
            "FLASH",
            if flash.is_empty() {
                String::new()
            } else {
                format!("<div class=\"flash\">{}</div>", escape(flash))
            },
        ),
        (
            "RENDERER_CLASS",
            if renderer_connected { "connected" } else { "offline" }.to_string(),
        ),
        (
            "RENDERER_TEXT",
            if renderer_connected { "connected" } else { "offline" }.to_string(),
        ),
        (
            "DASH_SCREENSAVER",
            screensaver_label(backgrounds, &config.screensaver, &config.screensaver_theme),
        ),
        ("BRIGHTNESS", config.brightness.to_string()),
        (
            "DASH_WEATHER",
            if config.weather_enabled {
                format!("On · {}", escape(&config.zip_code))
            } else {
                "Off".to_string()
            },
        ),
        (
            "DASH_INDOOR",
            match (config.indoor_enabled, config.indoor_sensor.as_str()) {
                (false, _) => "Off".to_string(),
                // Empty is the default and means the first sensor reported —
                // said in words, because a blank card reads as broken.
                (true, "") => "First sensor".to_string(),
                (true, sensor) => escape(sensor),
            },
        ),
        // The card's own heading, as the section and the nav button carry it.
        ("READING_TITLE", title),
        (
            "DASH_READING",
            // **No setting's value is shown.** What one means is the addon's
            // business, and the dashboard says only what the host decides:
            // whether the card is on and whether anything fills it.
            match (config.reading_enabled, description) {
                (false, _) => "Off".to_string(),
                (true, None) => "No reading source installed".to_string(),
                (true, Some(_)) => "On".to_string(),
            },
        ),
        (
            "DASH_CLOCK",
            format!(
                "{}{}",
                if config.use_24_hour { "24-hour" } else { "12-hour" },
                if config.show_date { " · Date" } else { "" }
            ),
        ),
        ("DASH_TIMEZONE", escape(zone)),
        (
            "DASH_SCHEDULE",
            format!("{events} event{}", if events == 1 { "" } else { "s" }),
        ),
        ("ZIP_CODE", escape(&config.zip_code)),
        ("CHK_USE_24_HOUR", checked(config.use_24_hour)),
        ("CHK_SHOW_DATE", checked(config.show_date)),
        // **No `THEME` value, deliberately.** Appearance is the browser's
        // preference, read from its own `localStorage` and applied before this
        // page paints; the server neither stores it nor knows it.
        ("SCREENSAVER_FIELDS", screensaver_fields(backgrounds, config)),
        ("SCREENSAVER_THEMES_JSON", screensaver_themes_json(backgrounds)),
        ("CHK_WEATHER", checked(config.weather_enabled)),
        ("SEL_UNIT_IMPERIAL", selected(&config.temp_unit, "imperial")),
        ("SEL_UNIT_METRIC", selected(&config.temp_unit, "metric")),
        ("WEATHER_CONTACT", escape(&config.weather_contact)),
        ("CHK_INDOOR", checked(config.indoor_enabled)),
        (
            "INDOOR_FIELDS",
            match indoor_description {
                // **`.rulings/` R-219:** the addon's own `verify_as` key is
                // the TempMon name control, and it starts hidden whenever
                // `crate::config::tempmon_name_is_asked` says no — see
                // `tempmon_name_hidden` above and
                // `crate::portal::schema::render_fields`'s own doc on what
                // naming a key here does and does not mean.
                Some(description) => {
                    schema::render_fields(&description.fields, indoor_current, hidden_indoor_keys)
                }
                None => String::new(),
            },
        ),
        (
            "INDOOR_UNAVAILABLE",
            match indoor_description {
                Some(_) => String::new(),
                None => "<p class=\"hint\" style=\"color:#e0a020\">No indoor reading source \
                         is installed for this class, so there is nothing here to \
                         configure yet. Install one and this section fills itself in \
                         from what it describes — nothing on this page is specific to \
                         any one addon.</p>"
                    .to_string(),
            },
        ),
        // **`.rulings/` R-219: refuse rather than guess, on the page itself,
        // in `effective_tempmon_name`'s own words.** `indoor_name_hint` is
        // `Some` only when that function refused — never merely because a
        // name is not asked for at all (loopback, or a plain `http://` URL),
        // where empty is exactly right and `crate::portal` does not even ask.
        (
            "INDOOR_NAME_HINT",
            match indoor_name_hint {
                Some(message) => format!(
                    "<p class=\"hint\" style=\"color:#e0a020\">{}</p>",
                    escape(message)
                ),
                None => String::new(),
            },
        ),
        ("CHK_READING", checked(config.reading_enabled)),
        (
            "READING_FIELDS",
            match description {
                Some(description) => schema::render_fields(&description.fields, current, &[]),
                None => String::new(),
            },
        ),
        (
            "READING_UNAVAILABLE",
            match description {
                Some(_) => String::new(),
                None => "<p class=\"hint\" style=\"color:#e0a020\">No reading source is \
                         installed for this class, so there is nothing here to \
                         configure yet. Install one and this section fills itself in \
                         from what it describes — nothing on this page is specific to \
                         any one addon.</p>"
                    .to_string(),
            },
        ),
        // Into a JavaScript string literal, so the escaping that matters is
        // the quote and the backslash rather than the angle bracket. The value
        // is one of a fixed set of section names, and anything else is
        // discarded by the router before it reaches here — but writing it
        // through the same escape is what stops that being load-bearing.
        ("SECTION", escape(section)),
        ("SCHEDULE", schedule_table(backgrounds, &config.schedule)),
    ];
    fill(INDEX, &values)
}

/// The schedule event editor, for a new event or an existing one.
pub fn schedule_editor(
    // What the background pack described, for the screensaver rows — T7.
    backgrounds: Option<&Description>,
    index: Option<usize>,
    event: &ScheduledEvent,
) -> String {
    let mut days = String::new();
    for day in EDITOR_DAYS {
        let ticked = event.days.iter().any(|d| d.eq_ignore_ascii_case(day));
        days.push_str(&format!(
            "<div class=\"day-check\">\n\
             <input type=\"checkbox\" name=\"days\" value=\"{day}\" {}>\n\
             <span>{day}</span>\n\
             </div>\n",
            checked(ticked)
        ));
    }

    // An event with no actions would draw an empty editor with nothing to
    // fill in, so a new one starts with a row.
    let existing;
    let actions = if event.actions.is_empty() {
        existing = vec![ScheduledAction::default()];
        &existing
    } else {
        &event.actions
    };

    let mut rows = String::new();
    for (position, action) in actions.iter().enumerate() {
        rows.push_str(&action_row(backgrounds, position, action));
    }

    fill(
        EDITOR,
        &[
            (
                "TITLE",
                if index.is_some() { "Edit Event" } else { "New Event" }.to_string(),
            ),
            (
                "INDEX",
                index.map_or_else(|| "-1".to_string(), |i| i.to_string()),
            ),
            ("TIME", escape(&event.time)),
            ("DAYS", days),
            ("ACTIONS", rows),
            ("ACTION_COUNT", actions.len().to_string()),
        ],
    )
}

/// One editable action.
fn action_row(
    catalogue: Option<&Description>,
    position: usize,
    action: &ScheduledAction,
) -> String {
    // **No CEC row.** `crate::config` argues it: a kiosk application holds the
    // display lease and cannot reach a television, so those hours belong to the
    // machine document's `[display]` table.
    let types = [
        ("display", "Display"),
        ("screensaver", "Screensaver"),
        ("brightness", "Brightness"),
        ("weather", "Weather"),
    ];
    let mut type_options = String::new();
    for (value, label) in types {
        type_options.push_str(&format!(
            "<option value=\"{value}\" {}>{label}</option>",
            selected(&action.kind, value)
        ));
    }

    // **The screensaver's two lists come from the pack**, since T7 — the
    // fourth and fifth copies of that catalogue used to be right here, and
    // `("boxes", "green") => "Matrix Green"` was already stale against a
    // palette renamed *Chartreuse* in 2026-09-01. Built as owned pairs rather
    // than the `&'static` slices the other kinds use, because a pack's names
    // are not known until it has described itself.
    let backgrounds: Vec<(String, String)> = match (action.kind.as_str(), catalogue) {
        ("screensaver", Some(d)) => d
            .backgrounds
            .iter()
            .map(|b| (b.id.clone(), b.name.clone()))
            .collect(),
        _ => Vec::new(),
    };
    let fixed: &[(&str, &str)] = match action.kind.as_str() {
        // Handled above; this arm is unreachable for `screensaver`.
        "screensaver" => &[],
        "brightness" => &[("25", "25%"), ("50", "50%"), ("75", "75%"), ("100", "100%")],
        // `display` and `weather` take the same pair, and so does an action
        // type this build has not heard of — which is the safe guess, because
        // both of the retired ones took it too.
        _ => &[("on", "On"), ("off", "Off")],
    };
    let mut value_options = String::new();
    for (value, label) in backgrounds
        .iter()
        .map(|(v, l)| (v.as_str(), l.as_str()))
        .chain(fixed.iter().copied())
    {
        value_options.push_str(&format!(
            "<option value=\"{}\" {}>{}</option>",
            escape(value),
            selected(&action.value, value),
            escape(label),
        ));
    }

    let themes: Vec<(String, String)> = match (action.kind.as_str(), catalogue) {
        ("screensaver", Some(d)) => d
            .backgrounds
            .iter()
            .find(|b| b.id == action.value)
            .map(|b| {
                b.themes
                    .iter()
                    .map(|t| (t.id.clone(), t.label.clone()))
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let mut theme_options = String::new();
    for (value, label) in &themes {
        theme_options.push_str(&format!(
            "<option value=\"{}\" {}>{}</option>",
            escape(value),
            selected(&action.theme, value),
            escape(label),
        ));
    }
    let theme_style = if themes.is_empty() { "display:none" } else { "" };

    format!(
        "<div class=\"action-row\" data-index=\"{position}\">\n\
         <select name=\"action.{position}.type\" class=\"type-select\" \
         onchange=\"onTypeChange(this)\">{type_options}</select>\n\
         <select name=\"action.{position}.value\" class=\"value-select\" \
         onchange=\"onValueChange(this)\">{value_options}</select>\n\
         <select name=\"action.{position}.theme\" class=\"theme-select\" \
         style=\"{theme_style}\">{theme_options}</select>\n\
         <button type=\"button\" class=\"remove-btn\" onclick=\"removeAction(this)\">\
         Remove</button>\n\
         </div>\n"
    )
}

#[cfg(test)]
mod tests;
