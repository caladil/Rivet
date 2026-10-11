//! **A `rivetos_addon::Field` set, rendered and validated without knowing what
//! it describes** — `.rulings/` R-197 D3, the reason this module exists at
//! all.
//!
//! # The shape to avoid
//!
//! Nothing in this file may special-case any addon, or any class. What it
//! knows is [`FieldKind`]'s five variants and nothing else — not a key, not a
//! label, not which addon is loaded, not what a value measures. Neither does
//! anything else in this program for the reading card (`.rulings/` R-375 D1);
//! this module is the proof that the portal does not need any of it to render
//! a form or to refuse a bad one. `schema::tests` pins that with a field set this module has
//! never seen — see `an_addon_this_module_has_never_seen_renders_correctly`.
//!
//! # Every control is named `field.<key>`
//!
//! Not the addon's bare key, because a page may one day hold more than one
//! addon's section and two addons are free to both call a field `enabled`.
//! The prefix is also what lets [`validate_fields`] find exactly the inputs it
//! put there and nothing a different section of the same `POST` happens to
//! share a name with.
//!
//! # A `Secret` field is typed here and stored — the maintainer's ruling, 2026-09-22
//!
//! [`render_field`] draws a password input, and [`store_secrets`] writes what
//! was typed to the platform's own secret file through
//! [`rivet_sdk::secrets::install`] — `RIVET_APP_DATA/<name>` at `0o600`, the
//! identical artifact `rivet apply install-secret` writes and
//! [`rivet_sdk::secrets::get_bytes`] reads back. This is not a second, weaker
//! store: it is the platform's defined location, written through the same name
//! rule and the same mode the operator's channel uses, from inside the
//! application that owns it.
//!
//! This reverses `.rulings/` R-197 D3, which held that a credential must never
//! reach a `POST` body and that a `Secret` was a *state* the operator installed
//! from the control plane. The maintainer ruled otherwise: there **will** be a
//! dialog, addon-agnostic, that takes a username, a region and a password and
//! stores them. A person typing their own credential into their own machine's
//! portal is what the dialog is for, and the bytes land exactly where the
//! platform already keeps that named secret.
//!
//! **The secret is still not an addon *setting*.** [`validate_fields`] does not
//! put it in the map handed to `Configure`; [`store_secrets`] writes it to the
//! file, and `crate::addon::material` resolves it back by name — so the addon
//! reads its credential exactly as before and is told nothing about how it was
//! written.
//!
//! # Bounds are the addon's own, and they are checked here — before a
//! `Configure`
//!
//! [`validate_fields`] refuses a submission against the exact `min`, `max`,
//! `max_len` and `options` [`Description::fields`] stated, and returns the
//! first complaint rather than a partial map. An addon that then refused one
//! of these values anyway would be refusing something this host already knew
//! was fine — a round trip that should never happen, and the reason this
//! function exists rather than letting `Request::Configure`'s answer be the
//! first place a bad number is caught.

use rivetos_addon::{Field, FieldKind};
use serde_json::{Map, Value};

use super::form::Form;
use super::html::escape;

/// The prefix every rendered control's `name` carries — see the module note.
const PREFIX: &str = "field.";

/// Every field of a described set, as one HTML fragment.
///
/// `current` is keyed by [`Field::key`], exactly as
/// [`rivetos_addon::Request::Configure::settings`] is. A key this map does not
/// carry — no `Configure` has been saved yet, or a newer `Describe` added a
/// field — is filled from the field's own stated default rather than left
/// blank, which is what lets a fresh install show a sensible form before
/// anybody has saved anything.
///
/// **`hidden` names [`Field::key`]s whose control is rendered but starts
/// `display:none`.** This is the caller's reason and not this module's —
/// nothing here learns what any key MEANS by it, only that this particular
/// one was asked to start out of sight, exactly as opaque a fact as `current`
/// keyed by the same strings already is. The control is rendered rather than
/// left out entirely so that a page's own script can reveal it without a
/// round trip to the server — see `crate::portal::html::dashboard`'s note on
/// the indoor TempMon name, the one caller with a key to give it today.
#[must_use]
pub fn render_fields(fields: &[Field], current: &Map<String, Value>, hidden: &[&str]) -> String {
    let mut out = String::new();
    for field in fields {
        // The field's own default when nothing was saved —
        // `crate::addon::field_default`, the same function that completes a
        // `Configure`, so the page shows exactly what the addon is sent.
        let value = current
            .get(&field.key)
            .cloned()
            .unwrap_or_else(|| crate::addon::field_default(&field.kind));
        let starts_hidden = hidden.contains(&field.key.as_str());
        out.push_str(&render_field(field, &value, starts_hidden));
    }
    out
}

/// One field, as the control its [`FieldKind`] names.
///
/// `starts_hidden` is [`render_fields`]'s own parameter, one key at a time —
/// see its doc. It adds nothing but an inline style to the same wrapper this
/// function already draws, so a caller's script can find the control by its
/// ordinary `id` and toggle the very attribute this sets.
fn render_field(field: &Field, current: &Value, starts_hidden: bool) -> String {
    let name = format!("{PREFIX}{}", field.key);
    let id = name.replace('.', "-");
    let label = escape(&field.label);
    let help = if field.help.is_empty() {
        String::new()
    } else {
        format!("<p class=\"hint\">{}</p>\n", escape(&field.help))
    };
    let style = if starts_hidden { " style=\"display:none\"" } else { "" };

    match &field.kind {
        FieldKind::Bool { default } => {
            let checked = current.as_bool().unwrap_or(*default);
            format!(
                "<div class=\"toggle-row\"{style}>\n\
                 <input id=\"{id}\" type=\"checkbox\" name=\"{name}\" value=\"1\" {}>\n\
                 <label for=\"{id}\">{label}</label>\n\
                 </div>\n\
                 {help}",
                if checked { "checked" } else { "" },
            )
        }
        FieldKind::Number { default, min, max, unit } => {
            let value = current.as_f64().unwrap_or(*default);
            let unit_suffix = if unit.is_empty() {
                String::new()
            } else {
                format!(" ({})", escape(unit))
            };
            format!(
                "<div class=\"field\"{style}>\n\
                 <label for=\"{id}\">{label}{unit_suffix}</label>\n\
                 <input id=\"{id}\" type=\"number\" name=\"{name}\" \
                 min=\"{min}\" max=\"{max}\" step=\"any\" value=\"{value}\">\n\
                 {help}</div>\n"
            )
        }
        FieldKind::Choice { default, options } => {
            let current_value = current.as_str().map_or_else(|| default.clone(), str::to_string);
            let mut option_tags = String::new();
            for (value, option_label) in options {
                option_tags.push_str(&format!(
                    "<option value=\"{}\" {}>{}</option>\n",
                    escape(value),
                    if *value == current_value { "selected" } else { "" },
                    escape(option_label),
                ));
            }
            format!(
                "<div class=\"field\"{style}>\n\
                 <label for=\"{id}\">{label}</label>\n\
                 <select id=\"{id}\" name=\"{name}\">\n{option_tags}</select>\n\
                 {help}</div>\n"
            )
        }
        FieldKind::Text { default, max_len } => {
            let value = current.as_str().map_or_else(|| default.clone(), str::to_string);
            format!(
                "<div class=\"field\"{style}>\n\
                 <label for=\"{id}\">{label}</label>\n\
                 <input id=\"{id}\" type=\"text\" name=\"{name}\" maxlength=\"{max_len}\" \
                 value=\"{}\">\n\
                 {help}</div>\n",
                escape(&value),
            )
        }
        // **A password input, whose stored value is never sent back to the
        // page.** The control is always empty on render — a browser is told
        // whether one is stored, not what it is. [`store_secrets`] reads it on
        // submit; see the module note.
        FieldKind::Secret { name: secret_name } => {
            // Keyed by `field.key`, like every other control — the secret's own
            // name (`FieldKind::Secret.name`) is what `store_secrets` writes
            // under, and it never appears in the page. Whether one is stored is
            // read from the file itself, never from `current` (which carries
            // only the addon's settings, never a secret).
            let stored = rivet_sdk::secrets::get_bytes(secret_name).is_some();
            let placeholder = if stored {
                "stored — leave blank to keep it"
            } else {
                "not stored"
            };
            let note = if stored {
                "A value is stored on this machine. Leave this blank to keep it, or type a new \
                 one to replace it."
            } else {
                "Typed here and stored on this machine."
            };
            format!(
                "<div class=\"field\"{style}>\n\
                 <label for=\"{id}\">{label}</label>\n\
                 <input id=\"{id}\" type=\"password\" name=\"{name}\" autocomplete=\"off\" \
                 placeholder=\"{placeholder}\">\n\
                 <p class=\"hint\">{note}</p>\n\
                 {help}</div>\n",
            )
        }
    }
}

/// **Validate a submission against the addon's own stated bounds, and refuse
/// before anything is sent to it.**
///
/// Every non-secret field in `fields` is read from `submitted` under
/// `field.<key>` and checked against exactly the rule its [`FieldKind`]
/// carries — the same `min`/`max`, the same `options`, the same `max_len` a
/// person was shown. The first field that fails ends the call: a partial map
/// with one field silently dropped would be a setting that looks saved and is
/// not.
///
/// A [`FieldKind::Secret`] contributes nothing to this map — see the module
/// note. Its `field.<key>` is read by [`store_secrets`], which writes it to the
/// platform secret file; validation of a credential is only its size, which
/// [`rivet_sdk::secrets::install`] enforces, so there is nothing to check here.
///
/// # Errors
///
/// One sentence, naming the field's label and what was wrong with it. Safe to
/// show a person directly: it is built entirely from the addon's own
/// `Describe` and what they typed, never from anything this host holds.
pub fn validate_fields(fields: &[Field], submitted: &Form) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    for field in fields {
        let name = format!("{PREFIX}{}", field.key);
        match &field.kind {
            FieldKind::Bool { .. } => {
                out.insert(field.key.clone(), Value::Bool(submitted.value(&name) == "1"));
            }
            FieldKind::Number { min, max, .. } => {
                let raw = submitted.value(&name).trim();
                let parsed: f64 = raw
                    .parse()
                    .map_err(|_| format!("{}: {raw:?} is not a number", field.label))?;
                if parsed < *min || parsed > *max {
                    return Err(format!(
                        "{}: {parsed} is outside the range {min}–{max} the addon allows",
                        field.label
                    ));
                }
                out.insert(field.key.clone(), serde_json::json!(parsed));
            }
            FieldKind::Choice { options, .. } => {
                let raw = submitted.value(&name);
                if !options.iter().any(|(value, _)| value == raw) {
                    return Err(format!(
                        "{}: {raw:?} is not one of the options the addon offered",
                        field.label
                    ));
                }
                out.insert(field.key.clone(), Value::String(raw.to_string()));
            }
            FieldKind::Text { max_len, .. } => {
                let raw = submitted.value(&name);
                if raw.chars().count() > *max_len {
                    return Err(format!(
                        "{}: longer than the {max_len} characters the addon allows",
                        field.label
                    ));
                }
                out.insert(field.key.clone(), Value::String(raw.to_string()));
            }
            // A secret is not an addon setting: it is written to the platform
            // secret file by [`store_secrets`], not carried in the map handed
            // to `Configure`. So it contributes nothing here — see the module
            // note.
            FieldKind::Secret { .. } => {}
        }
    }
    Ok(out)
}

/// **Write every `Secret` field the form filled, to the platform secret file.**
///
/// Walks the same described set as [`render_fields`] and [`validate_fields`],
/// and for each [`FieldKind::Secret`] reads `field.<key>` from `submitted`:
///
/// * **empty** — nothing typed, so the stored secret is left exactly as it is.
///   This is what lets a person save the rest of a form without re-entering a
///   credential already stored, and why the rendered control is always blank
///   rather than pre-filled.
/// * **non-empty** — the bytes are written under the secret's own name
///   ([`FieldKind::Secret`]'s `name`, not `field.key`) through
///   [`rivet_sdk::secrets::install`]: `RIVET_APP_DATA/<name>` at `0o600`, the
///   same file `rivet apply install-secret` writes and
///   `crate::addon::material` resolves back by name.
///
/// Returns whether anything was stored, so the caller can treat a changed
/// credential the way it treats a changed setting — a fresh `Configure` with
/// the new material, which is what clears a source's backoff.
///
/// # A value is never trimmed
///
/// A password may begin or end with a space and mean it, so what a person typed
/// is stored verbatim. Only a truly empty control means *keep what is there*.
///
/// # Errors
///
/// A sentence naming the field's label, when [`rivet_sdk::secrets::install`]
/// refuses the value — in practice only a credential past the platform's size
/// cap. The message never contains the value. A secret written before a later
/// field failed stays written, exactly as `install-secret` run twice would
/// leave it.
pub fn store_secrets(fields: &[Field], submitted: &Form) -> Result<bool, String> {
    let mut stored = false;
    for field in fields {
        let FieldKind::Secret { name: secret_name } = &field.kind else {
            continue;
        };
        let typed = submitted.value(&format!("{PREFIX}{}", field.key));
        if typed.is_empty() {
            continue;
        }
        rivet_sdk::secrets::install(secret_name, typed.as_bytes())
            .map_err(|e| format!("{}: {e}", field.label))?;
        stored = true;
    }
    Ok(stored)
}

#[cfg(test)]
mod tests;
