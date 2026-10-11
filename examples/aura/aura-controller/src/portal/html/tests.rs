//! **The escaping, the substitution, and what each page actually says.**
//!
//! The pages are asserted by looking for the strings a person would look for,
//! not by comparing whole documents: a test that pinned every byte would fail
//! on a reworded hint and prove nothing about the values.
//!
//! # The reading card's section is asserted through a FABRICATED `Description`
//!
//! `description()` below is not any real addon — it is a small field set
//! built for this file, the same discipline `crate::portal::schema::tests`
//! uses and for the same reason: a test that only ever rendered one real
//! addon's fields could not tell *this page is generic* from *this page
//! happens to work for that addon*.

use super::*;

fn config() -> Config {
    Config {
        zip_code: "20500".to_string(),
        ..Config::default()
    }
}

/// **A background pack's catalogue, fabricated for this file** — the same
/// discipline `description()` below follows, and for the same reason.
///
/// Two backgrounds and three themes, with ids and names deliberately *unlike*
/// the shipped pack's beyond the two that have to match a saved config. A test
/// that rendered only `aura-background-pack`'s own catalogue could not tell
/// *this page builds its pickers from whatever it was given* from *this page
/// happens to work for the pack we ship*, which is the entire property T7
/// added.
fn catalogue() -> Description {
    let theme = |id: &str, label: &str| rivetos_addon::Theme {
        id: id.to_string(),
        label: label.to_string(),
        cards: rivetos_addon::Cards::Unstated,
        animated: true,
    };
    Description {
        name: "test-pack".to_string(),
        version: "0.1.0".to_string(),
        class: "aura-background".to_string(),
        material: Vec::new(),
        fields: Vec::new(),
        backgrounds: vec![
            rivetos_addon::Background {
                id: "digital-rain".to_string(),
                name: "Digital Rain".to_string(),
                themes: vec![
                    theme("phosphor-green", "Phosphor Green"),
                    theme("ice-blue", "Ice Blue"),
                ],
                prefers_dark_cards: false,
                drawn: rivetos_addon::Drawn::Cpu,
            },
            rivetos_addon::Background {
                id: "boxes".to_string(),
                name: "Boxes".to_string(),
                themes: vec![theme("dark", "Dark")],
                prefers_dark_cards: false,
                // R-207 moves Boxes to the GPU and keeps its CPU path.
                drawn: rivetos_addon::Drawn::Either,
            },
        ],
    }
}

/// A field set built for this file — a Bool, a Number and a Secret — standing
/// in for whatever a real addon's `Describe` would answer. Nothing about it
/// is any real source.
fn description() -> Description {
    Description {
        name: "Test Source".to_string(),
        version: "0.1.0".to_string(),
        class: "test-reading-source".to_string(),
        material: vec!["test.env".to_string()],
        // A reading source draws nothing; the pack's catalogue is a background
        // addon's half of `Describe` and is empty here for the same reason
        // every reading source leaves it empty.
        backgrounds: Vec::new(),
        fields: vec![
            rivetos_addon::Field {
                key: "enabled_note".to_string(),
                label: "Show a note".to_string(),
                help: String::new(),
                kind: rivetos_addon::FieldKind::Bool { default: false },
            },
            rivetos_addon::Field {
                key: "threshold".to_string(),
                label: "Threshold".to_string(),
                help: "A made-up bound, for this file only.".to_string(),
                kind: rivetos_addon::FieldKind::Number {
                    default: 10.0,
                    min: 0.0,
                    max: 100.0,
                    unit: "widgets".to_string(),
                },
            },
            rivetos_addon::Field {
                key: "cred".to_string(),
                label: "Test credential".to_string(),
                help: String::new(),
                kind: rivetos_addon::FieldKind::Secret {
                    name: "test.env".to_string(),
                },
            },
        ],
    }
}

fn current() -> serde_json::Map<String, serde_json::Value> {
    let mut current = serde_json::Map::new();
    current.insert("enabled_note".to_string(), serde_json::json!(true));
    current.insert("threshold".to_string(), serde_json::json!(42.0));
    current
}

/// [`dashboard`] with BOTH roles answering the same way.
///
/// The page takes one `Description` and one value map per role, and almost
/// every test here is about something neither of them decides — escaping, a
/// section, a marker left unfilled. Passing the same pair twice keeps those
/// tests saying what they are about; the tests that care which role is which
/// call `dashboard` directly.
fn dash(
    config: &Config,
    description: Option<&Description>,
    current: &serde_json::Map<String, serde_json::Value>,
    renderer_connected: bool,
    flash: &str,
    section: &str,
    zone: &str,
) -> String {
    let catalogue = catalogue();
    dashboard(
        config,
        description,
        current,
        description,
        current,
        None,
        Some(&catalogue),
        renderer_connected,
        flash,
        section,
        zone,
    )
}

fn page(config: &Config) -> String {
    dash(
        config,
        Some(&description()),
        &current(),
        true,
        "",
        "general",
        "CDT (UTC−05:00)",
    )
}

// ---- Escaping -------------------------------------------------------------

#[test]
fn every_character_that_could_close_a_tag_or_an_attribute_is_escaped() {
    assert_eq!(
        escape(r#"<script>alert("x" + 'y') && z</script>"#),
        "&lt;script&gt;alert(&quot;x&quot; + &#39;y&#39;) &amp;&amp; z&lt;/script&gt;"
    );
}

#[test]
fn ordinary_text_passes_through_untouched() {
    assert_eq!(escape("Living Room — Küche"), "Living Room — Küche");
}

/// **A value from a form lands in HTML**, and there is no *this one is safe*
/// case. A sensor name is the shortest path from a text input to a page.
#[test]
fn a_sensor_name_carrying_markup_reaches_the_page_escaped() {
    let config = Config {
        indoor_sensor: "<img src=x onerror=alert(1)>".to_string(),
        ..config()
    };
    let rendered = page(&config);
    assert!(
        !rendered.contains("<img src=x"),
        "a script reached the page unescaped"
    );
    assert!(rendered.contains("&lt;img src=x onerror=alert(1)&gt;"));
}

/// **The weather contact is shown in its own field, escaped**, so saving the
/// page posts back what is stored rather than clearing it — the form submits
/// every section at once.
#[test]
fn the_weather_contact_is_rendered_into_its_field_escaped() {
    let marked = Config {
        weather_contact: "ops@example.com\"><b>".to_string(),
        ..config()
    };
    let rendered = page(&marked);
    assert!(
        rendered.contains(
            r#"name="weather_contact" value="ops@example.com&quot;&gt;&lt;b&gt;""#
        ),
        "the contact is not in its field, escaped"
    );

    let empty = page(&Config {
        weather_contact: String::new(),
        ..config()
    });
    assert!(empty.contains(r#"name="weather_contact" value="""#));
}

// ---- Substitution ---------------------------------------------------------

#[test]
fn a_marker_is_replaced_by_its_value() {
    assert_eq!(
        fill("a{{X}}b", &[("X", "1".to_string())]),
        "a1b"
    );
}

/// **One pass**, so a substituted value cannot be substituted into. Escaped
/// text can legitimately contain a brace.
#[test]
fn a_value_containing_a_marker_is_not_substituted_into() {
    let out = fill(
        "{{A}}|{{B}}",
        &[("A", "{{B}}".to_string()), ("B", "second".to_string())],
    );
    assert_eq!(out, "{{B}}|second");
}

/// **A marker nobody filled is left as written**, not blanked: it is visible on
/// the page, which is how it gets noticed. A silent empty string is how a field
/// goes missing for a month.
#[test]
fn an_unfilled_marker_survives_so_that_somebody_sees_it() {
    assert_eq!(fill("a{{MISSING}}b", &[]), "a{{MISSING}}b");
}

#[test]
fn an_unterminated_marker_does_not_eat_the_rest_of_the_page() {
    assert_eq!(fill("a{{X", &[("X", "1".to_string())]), "a{{X");
}

// ---- The dashboard --------------------------------------------------------

/// Every marker in the template must be filled, or a person sees `{{...}}`.
#[test]
fn the_dashboard_leaves_no_marker_unfilled() {
    let rendered = page(&config());
    assert!(
        !rendered.contains("{{"),
        "an unfilled marker reached the page: {:?}",
        rendered
            .split("{{")
            .nth(1)
            .map(|rest| rest.split("}}").next().unwrap_or(""))
    );
}

/// **And with no addon open at all** — the ordinary state of a fresh machine —
/// which exercises the `READING_UNAVAILABLE` branch instead of
/// `READING_FIELDS`.
#[test]
fn the_dashboard_leaves_no_marker_unfilled_with_no_addon_open() {
    let rendered = dashboard(&config(), None, &serde_json::Map::new(), None, &serde_json::Map::new(), None, None, true,
        "",
        "general",
        "UTC",
    );
    assert!(!rendered.contains("{{"), "an unfilled marker reached the page");
    assert!(rendered.contains("No reading source is installed"));
}

/// **The indoor section is the addon's, not this file's.**
///
/// Until 2026-09-20 it was three inputs written into `index.html` by name —
/// `tempmon_url`, `tempmon_name`, `indoor_sensor` — with hints in this
/// repository explaining a service this repository no longer talks to. It is
/// rendered from whatever the installed `aura-indoor-source` describes now, so
/// what proves the change is that a FABRICATED field set reaches the page: the
/// keys here belong to no real addon.
#[test]
fn the_indoor_section_is_rendered_from_what_the_addon_describes() {
    let rendered = dash(
        &config(),
        Some(&description()),
        &current(),
        true,
        "",
        "weather",
        "UTC",
    );
    assert!(
        rendered.contains("threshold"),
        "the described field set did not reach the page"
    );
    assert!(
        !rendered.contains("name=\"tempmon_url\""),
        "a hard-coded indoor input is still in the template"
    );
    assert!(
        !rendered.contains("name=\"indoor_sensor\""),
        "a hard-coded indoor input is still in the template"
    );
}

/// A field set shaped like the indoor role's own `Describe` — not
/// `aura-indoor-source`'s actual wording, which lives in a repository of its
/// own, but carrying the same KEYS `crate::addon::indoor_key` sends on the
/// wire (`url`, `verify_as`, `sensor`), because those are what make hiding
/// `verify_as` by key mean anything to test at all.
fn indoor_fields_description() -> Description {
    let text_field = |key: &str, label: &str| rivetos_addon::Field {
        key: key.to_string(),
        label: label.to_string(),
        help: String::new(),
        kind: rivetos_addon::FieldKind::Text { default: String::new(), max_len: 200 },
    };
    Description {
        name: "test-indoor-source".to_string(),
        version: "0.1.0".to_string(),
        class: "test-indoor".to_string(),
        material: Vec::new(),
        backgrounds: Vec::new(),
        fields: vec![
            text_field("url", "TempMon URL"),
            text_field("verify_as", "TempMon name"),
            text_field("sensor", "Sensor"),
        ],
    }
}

/// **`.rulings/` R-219: the TempMon name control is hidden while the
/// URL is loopback** — the ordinary state for a fresh install, where the
/// only correct name is this board's own and there is nothing for an
/// operator to type. The control itself still reaches the page (see the
/// next test's contrast) so a script can reveal it without a round trip.
#[test]
fn the_tempmon_name_field_starts_hidden_while_the_url_is_loopback() {
    let config = Config { tempmon_url: "https://127.0.0.1:8081".to_string(), ..config() };
    let indoor = indoor_fields_description();
    let rendered = dashboard(
        &config,
        None,
        &serde_json::Map::new(),
        Some(&indoor),
        &serde_json::Map::new(),
        None,
        Some(&catalogue()),
        true,
        "",
        "weather",
        "UTC",
    );
    assert!(
        rendered.contains(
            "<div class=\"field\" style=\"display:none\">\n<label for=\"field-verify_as\">"
        ),
        "a loopback URL must hide the TempMon name field: {rendered}"
    );
    // The URL and sensor fields are unrelated to the one hidden key and must
    // render exactly as they always have.
    assert!(rendered.contains("<div class=\"field\">\n<label for=\"field-url\">"), "{rendered}");
}

/// **The same field, shown, for a URL that names another machine** — the
/// contrast that proves hiding is keyed on the URL and not applied
/// unconditionally.
#[test]
fn the_tempmon_name_field_is_shown_for_a_non_loopback_url() {
    let config = Config { tempmon_url: "https://rivet-01.local:8081".to_string(), ..config() };
    let indoor = indoor_fields_description();
    let rendered = dashboard(
        &config,
        None,
        &serde_json::Map::new(),
        Some(&indoor),
        &serde_json::Map::new(),
        None,
        Some(&catalogue()),
        true,
        "",
        "weather",
        "UTC",
    );
    // Not a bare `!contains("display:none")`: the template's OWN comment
    // explaining this mechanism says that phrase in prose (see `index.html`),
    // so it is on every render regardless of loopback-ness. The precise
    // marker is the one the hidden-case test above asserts the presence of.
    assert!(
        !rendered.contains(
            "<div class=\"field\" style=\"display:none\">\n<label for=\"field-verify_as\">"
        ),
        "a non-loopback URL must not hide the TempMon name field: {rendered}"
    );
    assert!(rendered.contains("<div class=\"field\">\n<label for=\"field-verify_as\">"), "{rendered}");
}

/// **`.rulings/` R-219: a plain `http://` URL hides the field exactly like a
/// loopback one does, even though the host names another machine** — there
/// is no certificate on the wire for either scheme to verify a name against,
/// so `config::tempmon_name_is_asked` says no regardless of the host. The
/// same host over `https://` shows the field, which is the contrast that
/// proves the scheme is what decided it and not the host.
#[test]
fn the_tempmon_name_field_is_hidden_for_plain_http_and_shown_for_https_at_the_same_host() {
    let indoor = indoor_fields_description();
    let hidden_marker =
        "<div class=\"field\" style=\"display:none\">\n<label for=\"field-verify_as\">";

    let http_config = Config { tempmon_url: "http://192.168.1.5:8081".to_string(), ..config() };
    let http_rendered = dashboard(
        &http_config,
        None,
        &serde_json::Map::new(),
        Some(&indoor),
        &serde_json::Map::new(),
        None,
        Some(&catalogue()),
        true,
        "",
        "weather",
        "UTC",
    );
    assert!(
        http_rendered.contains(hidden_marker),
        "a plain http:// URL must hide the TempMon name field: {http_rendered}"
    );

    let https_config = Config { tempmon_url: "https://192.168.1.5:8081".to_string(), ..config() };
    let https_rendered = dashboard(
        &https_config,
        None,
        &serde_json::Map::new(),
        Some(&indoor),
        &serde_json::Map::new(),
        None,
        Some(&catalogue()),
        true,
        "",
        "weather",
        "UTC",
    );
    assert!(
        !https_rendered.contains(hidden_marker),
        "the same host over https:// must show the TempMon name field: {https_rendered}"
    );
}

/// **A refusal reaches the page in the exact words the resolver gave it,
/// escaped like any other value that reaches this page** — this function
/// draws no wording of its own for the reason, so a person reading the hint
/// reads `crate::config::effective_tempmon_name`'s own sentence, and the
/// apostrophe in it is why this fixture carries one: the module's own rule
/// is that everything reaching the page goes through [`escape`].
#[test]
fn a_refusal_message_is_shown_as_the_indoor_name_hint() {
    let indoor = indoor_fields_description();
    let rendered = dashboard(
        &config(),
        None,
        &serde_json::Map::new(),
        Some(&indoor),
        &serde_json::Map::new(),
        Some("enter that machine's own name so its certificate can be verified"),
        Some(&catalogue()),
        true,
        "",
        "weather",
        "UTC",
    );
    assert!(
        rendered.contains("enter that machine&#39;s own name so its certificate can be verified"),
        "{rendered}"
    );
}

/// **No hint at all reads as no paragraph at all** — an empty `<p>` sitting
/// in the markup would be a tell that something upstream forgot to fill it
/// in, exactly as an unfilled `{{MARKER}}` would be.
#[test]
fn no_hint_means_no_indoor_name_hint_paragraph() {
    let rendered = dash(
        &config(),
        Some(&description()),
        &current(),
        true,
        "",
        "weather",
        "UTC",
    );
    assert!(
        !rendered.contains("color:#e0a020\"></p>"),
        "an empty hint paragraph reached the page: {rendered}"
    );
}

/// **A role nobody filled says so, per role.**
///
/// The two notices are separate because the two roles are: a machine with a
/// thermometer and no reading source is ordinary, and a page that answered for
/// both at once could not describe it.
#[test]
fn an_indoor_role_with_no_addon_says_so_without_hiding_the_reading_one() {
    let rendered = dashboard(
        &config(),
        None,
        &serde_json::Map::new(),
        None,
        &serde_json::Map::new(),
        None,
        Some(&catalogue()),
        true,
        "",
        "weather",
        "UTC",
    );
    assert!(rendered.contains("No indoor reading source is installed"));
    assert!(rendered.contains("No reading source is installed"));
}

/// **Appearance never reaches the server, and the page must not post it.**
///
/// How this page is painted is a preference of the browser reading it, kept in
/// that browser's storage. A configuration is what the APPLIANCE is — the
/// weather's unit decides what the wall renders and is posted with everything
/// else — and the difference is whose preference it is, not which page it is
/// on.
///
/// The control therefore carries no `name`: a `<select>` without one is never
/// submitted, which is what stops a viewing preference from being written into
/// a machine's configuration and then restored onto somebody else's browser
/// from a backup.
#[test]
fn appearance_is_a_browser_preference_and_is_never_posted() {
    let rendered = page(&config());
    assert!(
        rendered.contains(r#"<select id="theme">"#),
        "the appearance control is missing"
    );
    assert!(
        !rendered.contains(r#"name="theme""#),
        "the appearance control has a name, so it would be posted and stored \
         on the machine"
    );
    // And nothing renders the attribute server-side: the inline head script
    // sets it from `localStorage` before the first paint.
    assert!(
        !rendered.contains("data-theme=\"{{"),
        "an unfilled theme marker reached the page"
    );
}

#[test]
fn the_editor_leaves_no_marker_unfilled() {
    let rendered = schedule_editor(Some(&catalogue()), None, &ScheduledEvent::default());
    assert!(!rendered.contains("{{"), "an unfilled marker reached the editor");
}

#[test]
fn the_renderer_status_says_which_way_round_it_is() {
    let connected = dashboard(&config(), Some(&description()), &current(), Some(&description()), &current(), None, Some(&catalogue()), true, "", "", "UTC");
    assert!(connected.contains("Renderer connected"), "{connected}");
    assert!(connected.contains("status-dot connected"));

    let offline = dash(&config(), Some(&description()), &current(), false, "", "", "UTC");
    assert!(offline.contains("Renderer offline"));
    assert!(offline.contains("status-dot offline"));
}

/// The zone is shown where the Go portal had a picker, and the command that
/// changes it is on the page — because Aura no longer owns the setting.
#[test]
fn the_timezone_is_shown_read_only_and_names_where_it_is_set() {
    let rendered = dashboard(&config(), Some(&description()), &current(), Some(&description()), &current(), None, Some(&catalogue()), true, "", "", "CDT (UTC−05:00)");
    assert!(rendered.contains("CDT (UTC−05:00)"), "{rendered}");
    assert!(rendered.contains("rivet apply edit timezone"));
    assert!(
        !rendered.contains("name=\"timezone\""),
        "a timezone form field survived"
    );
}

/// The CEC controls are gone and the page says where the setting went.
#[test]
fn the_display_section_points_at_the_machines_display_table() {
    let rendered = page(&config());
    assert!(rendered.contains("rivet apply edit display.off"), "{rendered}");
    assert!(!rendered.contains("cec_hdmi_input"), "a CEC field survived");
    assert!(!rendered.contains("action=\"/cec\""), "a CEC form survived");
}

#[test]
fn a_flash_is_shown_when_there_is_one_and_the_div_is_absent_when_there_is_not() {
    let with = dashboard(&config(), Some(&description()), &current(), Some(&description()), &current(), None, Some(&catalogue()), true, "Settings saved.", "", "UTC");
    assert!(with.contains("<div class=\"flash\">Settings saved.</div>"));

    let without = page(&config());
    assert!(!without.contains("class=\"flash\""));
}

/// **The links are relative**, because the proxy strips its prefix and this
/// program never learns its own base path.
#[test]
fn every_link_and_action_is_relative() {
    let rendered = page(&config());
    assert!(rendered.contains("href=\"static/style.css\""));
    assert!(rendered.contains("action=\"config\""));
    assert!(rendered.contains("href=\"schedule/new\""));
    assert!(
        !rendered.contains("\"/static/"),
        "an absolute asset link survived"
    );
    assert!(
        !rendered.contains("action=\"/"),
        "an absolute form action survived"
    );
}

#[test]
fn the_current_values_are_selected_and_the_current_flags_are_checked() {
    let config = Config {
        screensaver: "digital-rain".to_string(),
        temp_unit: "metric".to_string(),
        use_24_hour: true,
        show_date: false,
        ..config()
    };
    let rendered = dash(&config, Some(&description()), &current(), true, "", "", "UTC");
    assert!(rendered.contains("<option value=\"digital-rain\" selected>"));
    assert!(rendered.contains("<option value=\"metric\"   selected>"));
    assert!(rendered.contains("name=\"use_24_hour\" value=\"1\" checked"));
    assert!(rendered.contains("name=\"show_date\" value=\"1\" >"));
}

// ---- The reading card's section, built from a fabricated Description -------

/// **The page a person would use has the addon's own field set on it** —
/// built by `crate::portal::schema`, not by anything in this file. This is
/// the assertion that the wiring, and not just the renderer in isolation,
/// produces a real page.
#[test]
fn the_reading_section_carries_the_described_fields() {
    let rendered = page(&config());
    assert!(rendered.contains("Show a note"), "{rendered}");
    assert!(rendered.contains("Threshold"), "{rendered}");
    assert!(rendered.contains("A made-up bound, for this file only."), "{rendered}");
    assert!(rendered.contains("Test credential"), "{rendered}");
}

/// A `Secret` field draws a password input to type into, and the old
/// install-secret CLI hint is gone (the maintainer's ruling, 2026-09-22).
#[test]
fn the_reading_section_draws_a_password_input_for_the_secret_field() {
    let rendered = page(&config());
    assert!(
        rendered.contains("name=\"field.cred\""),
        "the Secret field must render its password input: {rendered}"
    );
    assert!(rendered.contains("type=\"password\""), "{rendered}");
    assert!(
        !rendered.contains("install-secret"),
        "the old CLI hint must be gone: {rendered}"
    );
}

/// The dashboard summary card reflects the enable switch and whether an addon
/// is open — never a credential's completeness, which is not a concept this
/// program has any more.
#[test]
fn the_dash_card_says_whether_a_reading_source_is_installed() {
    let off = dash(
        &Config { reading_enabled: false, ..config() },
        Some(&description()),
        &current(),
        true,
        "",
        "",
        "UTC",
    );
    assert!(off.contains("Off"), "{off}");

    let no_addon = dash(
        &Config { reading_enabled: true, ..config() },
        None,
        &serde_json::Map::new(),
        true,
        "",
        "",
        "UTC",
    );
    assert!(no_addon.contains("No reading source installed"), "{no_addon}");

    let on = dash(
        &Config { reading_enabled: true, ..config() },
        Some(&description()),
        &current(),
        true,
        "",
        "",
        "UTC",
    );
    assert!(
        on.contains("<div class=\"dash-value\">On</div>"),
        "the card says it is on, and shows no setting's value: {on}"
    );
}

/// **The section is titled with the addon's own name** — `.rulings/` R-375
/// D1: the host has no word for what the card shows, so the heading, the
/// navigation button's label, the overview card and the save button all
/// carry whatever the installed addon calls itself.
#[test]
fn the_reading_section_is_titled_with_the_addons_own_name() {
    let rendered = page(&config());
    assert!(
        rendered.contains("<div class=\"section-title\">Test Source</div>"),
        "the heading is the addon's name: {rendered}"
    );
    assert!(rendered.contains("data-tip=\"Test Source\""), "{rendered}");
    assert!(rendered.contains("<div class=\"dash-label\">Test Source</div>"), "{rendered}");
    assert!(rendered.contains(">Save Test Source</button>"), "{rendered}");
}

/// **With nothing installed, the section is titled with a neutral word**, and
/// with an addon whose name is blank, the same — never an empty heading.
#[test]
fn with_no_addon_the_reading_section_is_titled_reading() {
    assert_eq!(reading_title(None), "Reading");
    let blank = Description {
        name: "   ".to_string(),
        ..description()
    };
    assert_eq!(reading_title(Some(&blank)), "Reading");
    assert_eq!(reading_title(Some(&description())), "Test Source");

    let rendered = dashboard(
        &config(),
        None,
        &serde_json::Map::new(),
        None,
        &serde_json::Map::new(),
        None,
        None,
        true,
        "",
        "reading",
        "UTC",
    );
    assert!(rendered.contains("<div class=\"section-title\">Reading</div>"), "{rendered}");
}

/// **An addon's name reaches the page escaped**, in the heading and in the
/// attributes alike — it is text from an addon, and there is no *this one is
/// safe* case.
#[test]
fn an_addons_name_carrying_markup_is_escaped_wherever_it_lands() {
    let hostile = Description {
        name: "<b>\"x\"</b>".to_string(),
        ..description()
    };
    let rendered = dash(&config(), Some(&hostile), &current(), true, "", "", "UTC");
    assert!(!rendered.contains("<b>\"x\"</b>"), "an addon's name reached the page raw");
    assert!(rendered.contains("data-tip=\"&lt;b&gt;&quot;x&quot;&lt;/b&gt;\""), "{rendered}");
}

/// **The section's controls post under the host's own names**: the enable
/// switch is `reading_enabled`, and every addon field is `field.<key>`.
#[test]
fn the_reading_switch_posts_as_reading_enabled() {
    let rendered = page(&Config { reading_enabled: true, ..config() });
    assert!(
        rendered.contains("name=\"reading_enabled\" value=\"1\" checked"),
        "{rendered}"
    );
    assert!(rendered.contains("id=\"reading\" class=\"section\""), "{rendered}");
}

/// **An empty sensor nickname is the default and says what it means**, rather
/// than leaving the overview card blank.
#[test]
fn an_empty_sensor_nickname_reads_as_the_first_sensor() {
    let rendered = page(&Config {
        indoor_enabled: true,
        indoor_sensor: String::new(),
        ..config()
    });
    assert!(rendered.contains("<div class=\"dash-value\">First sensor</div>"), "{rendered}");
}

// ---- The schedule table ---------------------------------------------------

fn event(time: &str, days: &[&str], actions: Vec<ScheduledAction>) -> ScheduledEvent {
    ScheduledEvent {
        time: time.to_string(),
        days: days.iter().map(|d| (*d).to_string()).collect(),
        actions,
    }
}

fn action(kind: &str, value: &str, theme: &str) -> ScheduledAction {
    ScheduledAction {
        kind: kind.to_string(),
        value: value.to_string(),
        theme: theme.to_string(),
    }
}

#[test]
fn an_empty_schedule_says_so_instead_of_drawing_an_empty_table() {
    let rendered = page(&config());
    assert!(rendered.contains("No events."));
    assert!(!rendered.contains("<table"));
}

#[test]
fn a_schedule_row_reads_as_a_person_would_write_it() {
    let config = Config {
        schedule: vec![event(
            "19:00",
            &["Mon", "Fri"],
            vec![
                action("screensaver", "digital-rain", "ice-blue"),
                action("brightness", "20", ""),
            ],
        )],
        ..config()
    };
    let rendered = dash(&config, Some(&description()), &current(), true, "", "", "UTC");
    assert!(rendered.contains("<strong>19:00</strong>"));
    assert!(rendered.contains("Mon, Fri"));
    assert!(rendered.contains("Screensaver: Digital Rain — Ice Blue; Brightness: 20%"));
    assert!(rendered.contains("schedule/edit?index=0"));
    assert!(rendered.contains("schedule/delete?index=0"));
}

/// No days means every day, and the table has to say that rather than leave a
/// blank cell.
#[test]
fn a_row_with_no_days_says_every_day() {
    let config = Config {
        schedule: vec![event("07:00", &[], vec![action("display", "on", "")])],
        ..config()
    };
    let rendered = dash(&config, Some(&description()), &current(), true, "", "", "UTC");
    assert!(rendered.contains("Every day"));
    assert!(rendered.contains("Display: On"));
}

/// A count of one does not get an `s`.
#[test]
fn the_event_count_is_pluralised() {
    let one = Config {
        schedule: vec![event("07:00", &[], vec![action("display", "on", "")])],
        ..config()
    };
    assert!(
        dash(&one, Some(&description()), &current(), true, "", "", "UTC").contains("1 event<")
    );

    let none = config();
    assert!(
        dash(&none, Some(&description()), &current(), true, "", "", "UTC").contains("0 events")
    );
}

// ---- The editor -----------------------------------------------------------

#[test]
fn a_new_event_is_titled_new_and_carries_an_index_of_minus_one() {
    let rendered = schedule_editor(Some(&catalogue()), None, &ScheduledEvent::default());
    assert!(rendered.contains("<h1>New Event</h1>"));
    assert!(rendered.contains("name=\"index\" value=\"-1\""));
}

#[test]
fn an_existing_event_is_titled_edit_and_carries_its_index() {
    let rendered = schedule_editor(Some(&catalogue()), Some(3), &event("19:00", &[], vec![]));
    assert!(rendered.contains("<h1>Edit Event</h1>"));
    assert!(rendered.contains("name=\"index\" value=\"3\""));
    assert!(rendered.contains("value=\"19:00\""));
}

/// **A new event starts with one row**, or the editor is a page with nothing to
/// fill in.
#[test]
fn an_event_with_no_actions_still_draws_a_row() {
    let rendered = schedule_editor(Some(&catalogue()), None, &ScheduledEvent::default());
    assert!(rendered.contains("action.0.type"));
    assert!(rendered.contains("let actionCount = 1;"));
}

#[test]
fn the_days_a_row_carries_are_ticked_and_the_rest_are_not() {
    let rendered = schedule_editor(Some(&catalogue()), Some(0), &event("07:00", &["Mon", "Fri"], vec![]));
    assert!(rendered.contains("value=\"Mon\" checked"));
    assert!(rendered.contains("value=\"Fri\" checked"));
    assert!(rendered.contains("value=\"Sat\" >"));
}

#[test]
fn an_actions_current_type_value_and_theme_are_all_selected() {
    let rendered = schedule_editor(
        Some(&catalogue()),
        Some(0),
        &event(
            "19:00",
            &[],
            vec![action("screensaver", "boxes", "dark")],
        ),
    );
    assert!(rendered.contains("<option value=\"screensaver\" selected>"));
    assert!(rendered.contains("<option value=\"boxes\" selected>"));
    assert!(rendered.contains("<option value=\"dark\" selected>"));
}

/// A non-screensaver action has no themes, and the select is hidden rather than
/// drawn empty.
#[test]
fn a_theme_select_is_hidden_for_an_action_that_has_none() {
    let rendered = schedule_editor(Some(&catalogue()), Some(0), &event("19:00", &[], vec![action("brightness", "50", "")]));
    assert!(rendered.contains("class=\"theme-select\" style=\"display:none\""));
    assert!(rendered.contains("<option value=\"50\" selected>50%</option>"));
}

/// **There is no CEC row**, in the markup or in the script.
#[test]
fn the_editor_offers_no_cec_action() {
    let rendered = schedule_editor(Some(&catalogue()), None, &ScheduledEvent::default());
    assert!(!rendered.contains("value=\"cec\""), "a CEC option survived");
    assert!(!rendered.contains("CEC / TV"), "a CEC label survived");
    assert!(!rendered.contains("active_source"), "a CEC value survived");
}

/// Every page under `/schedule/` sits one level down, so `../` is right from
/// both of them — which is the reason the editor takes `?index=`.
#[test]
fn the_editors_links_climb_exactly_one_level() {
    let rendered = schedule_editor(Some(&catalogue()), None, &ScheduledEvent::default());
    assert!(rendered.contains("href=\"../static/style.css\""));
    assert!(rendered.contains("action=\"../schedule/save\""));
    assert!(rendered.contains("href=\"../?section=schedule\""));
}

// ---- The labels -----------------------------------------------------------

/// **A pair is named from the catalogue it was given, not from a table here.**
///
/// The labels below are the fabricated `catalogue()`'s, which is the point:
/// this used to be a ten-arm match in `html.rs` that could only ever name the
/// backgrounds the binary was built knowing about.
#[test]
fn every_screensaver_and_theme_pair_has_a_label_a_person_can_read() {
    let c = catalogue();
    let c = Some(&c);
    assert_eq!(
        screensaver_label(c, "digital-rain", "ice-blue"),
        "Digital Rain — Ice Blue"
    );
    assert_eq!(
        screensaver_label(c, "digital-rain", "phosphor-green"),
        "Digital Rain — Phosphor Green"
    );
    assert_eq!(screensaver_label(c, "boxes", "dark"), "Boxes — Dark");
    assert_eq!(screensaver_label(c, "", ""), "—");
}

/// **A pair the catalogue does not carry is shown by its ids rather than
/// blanked** — and escaped, because it came from a config file.
///
/// This is the ordinary state of things for the seconds between saving a
/// selection and installing the pack that has it, so it reads as
/// `starfield / amber` and not as an error.
#[test]
fn an_unknown_screensaver_is_named_and_escaped() {
    let c = catalogue();
    let c = Some(&c);
    assert_eq!(screensaver_label(c, "starfield", "amber"), "starfield / amber");
    assert_eq!(screensaver_label(c, "<b>", ""), "&lt;b&gt;");
    // And with no pack installed at all, every pair falls back the same way.
    assert_eq!(
        screensaver_label(None, "digital-rain", "ice-blue"),
        "digital-rain / ice-blue"
    );
}

/// **A theme label containing `</script>` cannot end the page's script
/// block.** A pack's names are somebody else's text arriving in a
/// `<script>`, where HTML escaping is the wrong escaping entirely.
#[test]
fn a_theme_label_cannot_break_out_of_the_script_block() {
    let mut c = catalogue();
    c.backgrounds[0].themes[0].label = "</script><script>alert(1)</script>".to_string();
    let json = screensaver_themes_json(Some(&c));
    assert!(
        !json.contains("</script>"),
        "a closing tag survived into the script block: {json}"
    );
    assert!(json.contains("<\\/script>"), "{json}");
}

/// With no pack installed the page gets an empty object and no picker, rather
/// than a broken script or a picker that reaches nothing — **and one sentence
/// in the picker's place saying why the screen stays black and what to
/// install**, so the missing picker is not a mystery. A pack that describes no
/// backgrounds is the same answer.
#[test]
fn no_pack_means_no_picker_an_empty_theme_table_and_a_sentence_saying_why() {
    assert_eq!(screensaver_themes_json(None), "{}");
    let mut empty = catalogue();
    empty.backgrounds.clear();
    for fields in [screensaver_fields(None, &config()), screensaver_fields(Some(&empty), &config())] {
        assert!(!fields.contains("<select"), "{fields}");
        assert!(
            fields.contains(
                "No background pack is installed on this machine, so the screen stays black. \
                 Install the <code>aura-background</code> library to choose a screensaver."
            ),
            "{fields}"
        );
    }
}

/// **With a pack installed, the picker, and the missing-pack sentence is
/// absent.**
#[test]
fn a_pack_means_the_picker_and_no_missing_pack_sentence() {
    let c = catalogue();
    let fields = screensaver_fields(Some(&c), &config());
    assert!(fields.contains("<select id=\"screensaver\""), "{fields}");
    assert!(fields.contains("<select id=\"screensaver_theme\""), "{fields}");
    assert!(!fields.contains("No background pack"), "{fields}");
}

/// An action type this build has not heard of is still listed, so a schedule
/// written by a newer build is legible rather than half-blank.
#[test]
fn an_unknown_action_type_is_still_readable_in_the_table() {
    assert_eq!(
        format_actions(None, &[action("teleport", "yes", "")]),
        "teleport: yes"
    );
}

// ---- A page to look at ------------------------------------------------------

/// **Not part of `cargo test`'s ordinary run.** `T5`'s brief asks for a real
/// rendered page to look at rather than trusting substring assertions alone —
/// the same discipline `aura-renderer`'s tests use when they write a `.ppm`.
/// This writes the dashboard, built from a reading source of the ordinary
/// **shape** — a unit Choice, two Number thresholds, an account name and a
/// Secret — to a file beside the crate so a person, or this agent, can open it.
///
/// **The shape is realistic; the source is invented.** A river gauge stands in
/// for whatever a real addon measures, because the host does not know and a
/// fixture is the last place worth teaching it (`.rulings/` R-375 D1). What a
/// reader is checking is that the described fields render as controls, that
/// the heading is the addon's own name and that the Secret renders as a
/// password input, and none of those depends on the wording.
/// Run explicitly:
///
/// ```text
/// cargo test --test-threads=1 -- --ignored render_the_dashboard_to_a_file
/// ```
#[test]
#[ignore = "writes a file for a human to look at; not part of the ordinary suite"]
fn render_the_dashboard_to_a_file() {
    let real_shaped = Description {
        name: "River Level".to_string(),
        version: "0.1.0".to_string(),
        class: "aura-reading-source".to_string(),
        material: vec!["gauge.password".to_string()],
        backgrounds: Vec::new(),
        fields: vec![
            rivetos_addon::Field {
                key: "unit".to_string(),
                label: "Unit".to_string(),
                help: String::new(),
                kind: rivetos_addon::FieldKind::Choice {
                    default: "m".to_string(),
                    options: vec![
                        ("m".to_string(), "Meters".to_string()),
                        ("ft".to_string(), "Feet".to_string()),
                    ],
                },
            },
            rivetos_addon::Field {
                key: "low_m".to_string(),
                label: "Low".to_string(),
                help: "Below this the gauge reads low. Always meters, whatever the unit above."
                    .to_string(),
                kind: rivetos_addon::FieldKind::Number {
                    default: 0.5,
                    min: 0.0,
                    max: 10.0,
                    unit: "m".to_string(),
                },
            },
            rivetos_addon::Field {
                key: "high_m".to_string(),
                label: "High".to_string(),
                help: "Above this the gauge reads high, and the card marks it.".to_string(),
                kind: rivetos_addon::FieldKind::Number {
                    default: 4.0,
                    min: 0.0,
                    max: 10.0,
                    unit: "m".to_string(),
                },
            },
            rivetos_addon::Field {
                key: "username".to_string(),
                label: "Username".to_string(),
                help: "The gauge service's account.".to_string(),
                kind: rivetos_addon::FieldKind::Text {
                    default: String::new(),
                    max_len: 254,
                },
            },
            rivetos_addon::Field {
                key: "password".to_string(),
                label: "Password".to_string(),
                help: "Stored on this machine and never shown.".to_string(),
                kind: rivetos_addon::FieldKind::Secret {
                    name: "gauge.password".to_string(),
                },
            },
        ],
    };
    let mut saved = serde_json::Map::new();
    saved.insert("unit".to_string(), serde_json::json!("ft"));
    saved.insert("high_m".to_string(), serde_json::json!(3.5));
    // The page shows what the first `Configure` would send: what was saved,
    // and each field's own default for what was not.
    let current = crate::addon::settings_for(&real_shaped.fields, &saved);

    let config = Config {
        zip_code: "20500".to_string(),
        indoor_sensor: "Sensor A".to_string(),
        schedule: vec![event(
            "19:00",
            &["Mon", "Fri"],
            vec![action("screensaver", "digital-rain", "ice-blue")],
        )],
        ..Config::default()
    };
    let rendered = dash(
        &config,
        Some(&real_shaped),
        &current,
        true,
        "",
        "reading",
        "CDT (UTC−05:00)",
    );
    // The real page reaches `static/style.css` through the route the portal
    // serves it on; a file opened directly on a laptop has no such route, so
    // the stylesheet is inlined here — and only here — for a human to see
    // what the real page looks like rather than its unstyled markup.
    let rendered = rendered.replacen(
        "<link rel=\"stylesheet\" href=\"static/style.css\">",
        &format!("<style>{STYLE}</style>"),
        1,
    );
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("portal_preview.html");
    std::fs::write(&out, rendered).expect("write the preview file");
    println!("wrote {}", out.display());
}
