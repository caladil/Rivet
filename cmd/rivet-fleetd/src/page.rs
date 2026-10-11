//! **The page** — a sidebar of the fleet, and an administration surface for one
//! machine.
//!
//! # Two authorities, kept visibly apart
//!
//! The sidebar is the **register**: what the fleet was told. The dashboard is
//! the **machine**: what it says about itself right now. `ADR-011` gives a fact
//! one home, and a page that merged them would have no way to show their
//! disagreement — which is the interesting case, because a device in the
//! register that does not answer is exactly what an operator needs to see.
//!
//! So a card is drawn from `/api/fleet` and is never blank; its status dot is
//! drawn from that machine's own answer and starts as *unknown*.
//!
//! # Nothing is cached and nothing is summarised
//!
//! Every number on the dashboard is the machine's own JSON, parsed in the
//! browser. `rivet-fleetd` forwards bytes (`node`), and this file asks for
//! them. A fleet server that computed a utilisation figure would be a second
//! opinion about somebody else's state.
//!
//! **The management surface added on 2026-09-05 did not cost either principle,
//! and `ADR-043` is where that was argued rather than assumed.** Three pages
//! were added — software, power and the journal — and the server grew
//! **no new route**: every one of them is `POST /mcp/<device>` carrying a
//! catalogue operation, and every consequence an operator reads before
//! confirming is a string the machine composed. The one place a summary was
//! tempting is the journal's filters, and they are applied to records the
//! browser already holds and labelled as this page's rather than the machine's
//! — see `web/journal.js`.
//!
//! **A fourth page, Screenshot, existed from 2026-09-05 and was retired
//! 2026-09-13** (`docs/PLAN-Feature-Set-2026-09-12.md` §8): a dedicated page
//! whose only content was a picker and a fetch had nothing in it that the
//! software table did not already know.
//!
//! **What replaced it was a button on every row, and that was retired too, on
//! 2026-09-20.** `ADR-058` made the screen a compositor's, so an application
//! can no longer photograph anything but its own surface and there is one
//! capture for the machine — one panel on the Software page, reading
//! `rivet-displayd`'s file. The picker the first page needed and the column
//! the second one grew were both answering *which application?*, and that
//! question no longer has an answer. See `web/software.js`.
//!
//! # What is in `web/`, and why it is not in this file
//!
//! This file used to hold the whole client as two Rust raw strings. It is now
//! an assembler over `web/`, which is ordinary CSS and JavaScript compiled in
//! with `include_str!`. Two reasons, and the second is not cosmetic:
//!
//! 1. The client outgrew a string literal the moment it had five pages.
//! 2. **Each file carries its own reasoning where a reader will find it.** The
//!    argument for how the capture button presses, waits and downloads belongs
//!    at the top of `web/software.js`, not in a Rust doc comment three
//!    hundred lines from the code it governs.
//!
//! The order below is the order they are concatenated into one `<script>`, and
//! it is a dependency order: `core` declares the transport and the `VIEWS`
//! registry, `plan` declares the confirmation gate, and every page uses both.
//!
//! # The brand
//!
//! `ADR-037` ruled the accent and `docs/brand/HANDOFF.md` carries the measured
//! contrast: `#C06139` on dark chrome is 4.51, which is why the accent is
//! `accent` and not the ruled `accent_on_light` `#B0512F` — that one reads 3.65
//! on this ground, the family's weakest dark number.
//!
//! The lockup is `include_str!`d from `docs/brand/` rather than pasted, so the
//! generator stays the single authority for it (`ADR-011` again). It is a build
//! product of `build-brand.py`; copying it here would fork it at the first
//! regeneration.

/// The dark horizontal lockup, straight from the brand kit.
const LOCKUP: &str = include_str!("../../../docs/brand/logo-h-dark.svg");

/// The whole stylesheet.
const STYLE: &str = include_str!("web/page.css");

/// **The client, in dependency order.** See the module documentation.
///
/// A slice rather than one `concat!`, so that a missing file is a compile error
/// naming the file rather than a page that renders and does nothing.
const SCRIPT: &[&str] = &[
    include_str!("web/core.js"),
    include_str!("web/plan.js"),
    include_str!("web/overview.js"),
    include_str!("web/software.js"),
    include_str!("web/power.js"),
    include_str!("web/journal.js"),
    include_str!("web/config.js"),
    include_str!("web/start.js"),
];

/// Build the page.
///
/// **The lockup's XML declaration is stripped.** `build-brand.py` writes a
/// standalone SVG document, and `<?xml …?>` is a parse error inline in HTML —
/// the browser recovers, but it recovers by guessing. Removed here rather than
/// asking the brand kit to emit a fragment, because a standalone file is the
/// right thing for a standalone file to be.
#[must_use]
pub fn html() -> String {
    let mark = LOCKUP.trim_start();
    let mark = mark
        .strip_prefix("<?xml")
        .and_then(|rest| rest.split_once("?>"))
        .map_or(mark, |(_, after)| after);
    format!("{HEAD}{STYLE}{BETWEEN}{mark}{TAIL}{}\n</script></body></html>", SCRIPT.join("\n"))
}

const HEAD: &str = r##"<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Rivet fleet</title>
<style>
"##;

const BETWEEN: &str = r##"</style></head><body>
<aside>
<div class="lockup">
"##;

/// **The supervisor link is a plain `<a href>` and not a view**, and that is the
/// whole point of it — `ADR-056` §3.
///
/// Every other entry in this sidebar is a `VIEWS` entry that fetches from a
/// machine over the control plane. This one is a navigation to a page the server
/// renders from its own `/proc`, so it still works when the script on this page
/// has nothing to talk to — which is the only circumstance in which anybody
/// wants it.
const TAIL: &str = r##"
</div>
<h2>Machines</h2>
<div id="cards"><div class="sub">reading the register…</div></div>
<h2>This machine</h2>
<p class="sub"><a href="/supervisor">Is the supervisor alive?</a><br>
Reads this machine's own <code>/proc</code>. Asks nothing, and takes a second.</p>
</aside>
<main id="main"><div class="empty">Select a machine.</div></main>
<script>
"##;

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **Every view the operation row offers has a page behind it.**
    ///
    /// The row is a list in `core.js` and each page registers itself into
    /// `VIEWS` from its own file. Nothing at run time checks that the two
    /// agree, and the failure is a button that clears the screen — so the check
    /// is here, against the shipped text, and it fails if a file is dropped
    /// from `SCRIPT` or a view is renamed on one side only.
    #[test]
    fn every_operation_in_the_row_has_a_view_registered_for_it() {
        let page = html();
        for key in ["overview", "software", "journal", "config", "power"] {
            assert!(
                page.contains(&format!("{{ key: \"{key}\"")),
                "`{key}` is not in the operation row"
            );
            assert!(
                page.contains(&format!("VIEWS.{key} = {{")),
                "`{key}` is in the operation row and no file registers a view for it"
            );
        }
    }

    /// **The screenshot page is gone, and stays gone.** Retired
    /// 2026-09-13 (`docs/PLAN-Feature-Set-2026-09-12.md` §8) and never
    /// reinstated by `ADR-058`'s change, which made the capture one PANEL on
    /// the Software page rather than a column of buttons — a different shape
    /// from both, and still not a sixth operation-row entry.
    ///
    /// `decodeQoi` and the rest of the decoder did not leave with the page —
    /// they moved to `web/software.js`, and
    /// `the_whole_client_is_in_the_one_document` below still checks for
    /// `decodeQoi` by name, wherever it lives.
    #[test]
    fn the_screenshot_page_was_retired_and_did_not_come_back() {
        let page = html();
        assert!(
            !page.contains("{ key: \"screenshot\""),
            "the retired screenshot entry is back in the operation row"
        );
        assert!(
            !page.contains("VIEWS.screenshot = {"),
            "the retired screenshot view is back"
        );
    }

    /// **A Power button is its icon alone, and its name is its tooltip.** Ruled
    /// by the maintainer 2026-09-13: each card reads title, button, text, and
    /// the title already names the operation, so the label on the button went.
    /// It did not vanish: it is the button's `title` and `aria-label`, which is
    /// how a reader who cannot see the icon still learns what it does.
    #[test]
    fn a_power_button_is_its_icon_with_its_name_as_tooltip_and_accessible_name() {
        let page = html();
        assert!(
            !page.contains("op.icon + \" \" + op.label"),
            "a Power button carries its label beside its icon again"
        );
        assert!(
            page.contains("press.title = op.label;"),
            "a Power button has no tooltip naming it"
        );
        assert!(
            page.contains("press.setAttribute(\"aria-label\", op.label);"),
            "a Power button has no accessible name"
        );
        let actions = page.find("box.append(actions, el(\"div\", \"power-what\", op.what));");
        assert!(
            actions.is_some(),
            "a Power card no longer puts its button above the sentence saying what it does"
        );
    }

    /// The page is one document: the style, the lockup and every script are in
    /// it, and none of them is fetched. `ADR-035`'s control plane carries one
    /// answer per request and a page that asked for six assets would be six
    /// connections against a limit of sixteen.
    ///
    /// # `RIVET_DUMP_PAGE` prints it, and that is how it gets LOOKED at
    ///
    /// **A page that compiles is not a page that renders**, and this program
    /// cannot be started without a supervisor handing it a socket on 4443 — so
    /// there is no way to see what `html` builds except to build it here and
    /// print it. `AGENTS.md` §6 is why that is worth a line of test code:
    ///
    /// ```console
    /// $ tools/container/exec.sh env RIVET_DUMP_PAGE=1 \
    ///     cargo test -p rivetos-fleetd the_whole_client -- --nocapture
    /// ```
    ///
    /// Everything from `<!doctype html>` to `</html>` is the document a browser
    /// would be served, byte for byte. Point something at it that can answer
    /// `GET /api/fleet` and `POST /mcp/<device>` and the page is inspectable
    /// with no board and no fleet server.
    #[test]
    fn the_whole_client_is_in_the_one_document() {
        let page = html();
        if std::env::var_os("RIVET_DUMP_PAGE").is_some() {
            println!("{page}");
        }
        assert!(page.starts_with("<!doctype html>"), "{}", &page[..40]);
        assert!(page.ends_with("</script></body></html>"));
        assert!(page.contains("--accent:#c06139"), "the stylesheet is not in the page");
        assert!(page.contains("<svg"), "the lockup is not in the page");
        assert!(!page.contains("<?xml"), "the lockup's XML declaration was not stripped");
        for marker in [
            "function bindWords",
            "function planGate",
            "function decodeQoi",
            // Wave 6 (`docs/PLAN-Feature-Set-2026-09-12.md` §8), built
            // 2026-09-13: uptime, the process filter/sort and the drawer's
            // package facts and capture button are all real functions, not
            // just prose.
            "const duration",
            "function processCpuShares",
            "function filterProcesses",
            "function installFacts",
            "function screenPanel",
            "function takeScreenshot",
            // Wave 3c (`docs/PLAN-Feature-Set-2026-09-12.md` §5.3,
            // `.rulings/` R-162): the schema-driven configuration screen.
            "function schemaFields",
            "function extractGates",
            "function kioskModeNotice",
            "function parseDocument",
            "function serializeDocument",
        ] {
            assert!(page.contains(marker), "{marker} is not in the page");
        }
        // **`start.js`, by name.** It is the only file whose absence produces a
        // page that renders perfectly and does nothing at all — no register,
        // no machines, no error. Every other file is reachable from the two
        // assertions above or from the view-registration test.
        assert!(page.contains("boot().catch"), "nothing starts this page");
        // One script element, not eight.
        assert_eq!(page.matches("<script>").count(), 1);
    }

    /// **No ADR citation reaches an operator's screen** — plan §8, "Everywhere":
    /// *"No ADR subsection references in operator-facing text."* This does not
    /// (and cannot, without a JS parser) assert that NO string anywhere in the
    /// shipped source cites an ADR — a `//` comment citing one is correct and
    /// expected, `AGENTS.md` §2 rule 8.
    ///
    /// **What it checks instead, and why not the citations themselves:** five
    /// notes an operator reads (the disable caveat, the reload note, the
    /// extensions note, the restart description and the trial box) used to
    /// carry an ADR citation inline and were rewritten to drop it. Asserting
    /// the OLD citation is gone by spelling it out here would put a citation
    /// shape back into a tracked `.rs` file — exactly what `claims.py`'s
    /// `deep-citations` and `citations` ratchets exist to catch — so this
    /// instead asserts the REPLACEMENT wording is present, which is only true
    /// once the citation-bearing sentence beside it is gone.
    #[test]
    fn operator_text_carries_no_adr_citation() {
        let page = html();
        for present in [
            "A reload withholds `enabled` for these and reports it. A restart withholds nothing",
            "Rolls back an uncommitted trial boot.",
            "The package kinds this build knows are `update`,",
            "A reload applies everything except the services that keep this machine reachable",
            "PID 1 does that on its own, once",
        ] {
            assert!(page.contains(present), "expected the rewritten operator text: {present}");
        }
    }

    /// **The journal viewer never sends a bare `get log`.**
    ///
    /// `LIMIT_DEFAULT` is 200 counting from the END, and a search against it
    /// comes back empty on a machine that has been up ten minutes because a
    /// boot's kernel records are at its start. `PLAN.md` §104.10 is what that
    /// cost once. This asserts the two properties that stop it recurring: the
    /// limit is always one of the offered words, and `all` is among them.
    #[test]
    fn the_journal_always_names_a_limit_and_can_turn_it_off() {
        let page = html();
        assert!(page.contains(r#"const LIMITS = ["200", "500", "2000", "10000", "all"]"#));
        assert!(page.contains("a.truncated"), "the truncation is not rendered");
    }

    /// **R33, `.rulings/` R-163: the Boot card reads `status.version` and only
    /// falls back to "not answered" when the machine did not send one.**
    ///
    /// This cannot start a machine and read a rendered card, so it checks the
    /// same way `the_journal_always_names_a_limit_and_can_turn_it_off` above
    /// does — the shipped source, for the two lines that decide between the
    /// real value and the fallback.
    #[test]
    fn the_boot_card_reads_the_machines_own_version() {
        let page = html();
        assert!(
            page.contains(r#"kv("Rivet version", status.version)"#),
            "the Boot card no longer reads status.version"
        );
        assert!(
            page.contains("not answered by this machine"),
            "the fallback for a machine that sent no version line is gone"
        );
    }

    /// **R34, `.rulings/` R-163: the Software drawer reads the installer's
    /// recorded size and does not walk the version directory.**
    ///
    /// The one-level walk this replaced named itself `skippedDeeper` when it
    /// under-counted; its return would be a silent regression to the
    /// approximation R34 was ruled to end, so its absence is asserted by name
    /// alongside the presence of the record it replaced it with.
    #[test]
    fn the_software_drawer_reads_the_recorded_size_and_does_not_walk() {
        let page = html();
        assert!(
            page.contains(r#"get(d.id, "file", versionDir + "/install-size")"#),
            "the drawer no longer reads the installer's recorded size"
        );
        assert!(
            !page.contains("skippedDeeper"),
            "the one-level directory walk R34 replaced is back"
        );
        assert!(
            page.contains("not recorded — this version was installed before this machine kept \
                            that record"),
            "an application installed before R34 must say its size is not recorded, not show 0"
        );
    }

    /// **R35, `.rulings/` R-163: exactly one disk card, and it is `/state`'s.**
    ///
    /// `BRIEFING-001` dropped both of the old page's disk cards; R35 asked for
    /// one back, for the volume applications and updates land on — not `/`,
    /// which is the distinction plan §8 draws by name.
    #[test]
    fn overview_shows_exactly_one_disk_card_and_it_is_states() {
        let page = html();
        assert!(
            page.contains(r#"panel("/state", stateBox)"#),
            "the /state card R35 asked for is not on the page"
        );
        assert!(
            page.contains(r#"(fs.holds || []).includes("/state")"#),
            "the /state card must find its filesystem by what it HOLDS, not by its mount point, \
             or it goes silent on a machine where /state shares a filesystem with /"
        );
        assert_eq!(
            page.matches("get(d.id, \"storage\")").count(),
            1,
            "storage should be read exactly once, for the /state card alone"
        );
    }

    /// **Wave 3c, `.rulings/` R-162: the ad hoc TOML editor is gone, replaced
    /// by the schema-driven screen plan §5.3 asks for.**
    ///
    /// The `config` entry on the operation row stays — the screen behind it
    /// changed, not its existence — so this checks for the SPECIFIC things
    /// that must not survive: the route that talked to the fleet-as-code git
    /// repository with no confirmation at all, and the hand-rolled parser
    /// that only understood one level of section nesting. `AGENTS.md` §2
    /// rule 5: removing a feature removes its prose in the same change, and
    /// this is the check that the prose actually left.
    #[test]
    fn the_ad_hoc_toml_editor_is_gone() {
        let page = html();
        assert!(
            !page.contains("/api/config/"),
            "the retired ad hoc config route is still referenced in the shipped page"
        );
        assert!(
            !page.contains("Basic TOML parser strictly for Rivet's flat structure"),
            "the old hand-rolled TOML parser is still in the page"
        );
        assert!(
            !page.contains("New service name..."),
            "the old ad hoc service editor is still in the page"
        );
    }

    /// **R-164, R-165: the portal makes the `mode`/`[kiosk]` mismatch
    /// visible and never refuses it.** `mode = "kiosk"` with no `[kiosk]`
    /// table is the one combination the maintainer ruled tolerated-and-shown
    /// rather than silent or refused — this is the sentence that says so,
    /// present in the shipped page rather than only in `formfields.rs`'s own
    /// tests, which cannot see this file's JavaScript at all.
    #[test]
    fn the_kiosk_mode_mismatch_is_made_visible_not_refused() {
        let page = html();
        assert!(
            page.contains("has no `[kiosk]` table yet"),
            "the R-164 mismatch notice is not in the page"
        );
    }

    /// **`.rulings/` R-215: `rpc` — the one function every call this page
    /// makes to a machine passes through, `callTool` and `readTools` alike —
    /// marks the sidebar dot on every call, not only at boot.**
    ///
    /// This cannot start a page and click a card (`the_whole_client_is_in_
    /// the_one_document`'s own doc comment says why), so it checks the
    /// shipped source for the two calls that decide it: a successful `rpc`
    /// marks the dot up before it returns, and a failed one marks it down,
    /// with the thrown message as the tooltip, before re-throwing — so
    /// nothing downstream loses the failure `rpc`'s own callers already
    /// handle.
    #[test]
    fn every_call_through_rpc_refreshes_the_devices_dot() {
        let page = html();
        assert!(page.contains("function markUp(id)"), "there is no shared 'up' path for the dot");
        assert!(
            page.contains("function markDown(id, message)"),
            "there is no shared 'down' path for the dot"
        );

        // `rpc`'s own body: `markUp` before the success return, `markDown`
        // before the re-throw — checked by ORDER within the function rather
        // than an exact adjacent string, so this does not depend on how the
        // two lines happen to be spaced or indented.
        let rpc_start = page.find("async function rpc(id, body)").expect("rpc is not in the page");
        let rpc_body = &page[rpc_start..];
        let rpc_end = rpc_body.find("\nasync function callTool").unwrap_or(rpc_body.len());
        let rpc_body = &rpc_body[..rpc_end];

        let mark_up = rpc_body.find("markUp(id)").expect("rpc's success path does not mark the dot up");
        let success_return =
            rpc_body.find("return outer.result").expect("rpc has no success return to mark before");
        assert!(mark_up < success_return, "the dot must be marked up BEFORE rpc returns, not after");

        let mark_down = rpc_body
            .find("markDown(id, e.message)")
            .expect("rpc's failure path does not mark the dot down");
        let rethrow = rpc_body.rfind("throw e;").expect("rpc does not re-throw what it caught");
        assert!(mark_down < rethrow, "the dot must be marked down BEFORE rpc re-throws, not after");
    }

    /// **The fix replaced `probe`'s own copy of the dot logic — it did not add
    /// a second one beside it.** Before R-215, `probe` read `d.id` into its
    /// own selector; `dotFor` (the shared reader every mark now goes through)
    /// reads the plain `id` every OTHER caller already has, which is what
    /// tells the two apart in the shipped text.
    #[test]
    fn probe_no_longer_carries_its_own_copy_of_the_dot_selector() {
        let page = html();
        assert!(
            !page.contains(r#"$('.card[data-id="' + CSS.escape(d.id) + '"] .dot')"#),
            "probe still selects the dot itself instead of going through the shared dotFor"
        );
        assert!(
            page.contains(r#"$('.card[data-id="' + CSS.escape(id) + '"] .dot')"#),
            "there is no single shared reader of a machine's dot element"
        );
    }

    /// **No second call was added.** Clicking a card already led to
    /// `readTools` and the Overview's own `get(d.id, "status")` — R-215 asks
    /// that whichever of those happens to run first is what refreshes the
    /// dot, not a third read added purely to answer this ruling.
    #[test]
    fn selecting_a_machine_adds_no_new_status_read() {
        let page = html();
        assert_eq!(
            page.matches("subject: \"status\"").count(),
            1,
            "a second, dedicated status read was added instead of reusing the Overview's own call"
        );
    }

    /// **R-215 rules a periodic re-check OUT, explicitly** — this asserts the
    /// negative directly rather than trusting that no later change adds one
    /// quietly. `core.js` alone: `journal.js`'s follow poll and
    /// `software.js`'s install-progress sleep already use both names for
    /// reasons R-215 does not touch, so the whole page is not the right scope
    /// to check.
    #[test]
    fn core_js_adds_no_timer_to_recheck_the_dot() {
        let core = SCRIPT[0];
        assert!(!core.contains("setInterval"), "a periodic re-check was added; R-215 asked for none");
        assert!(!core.contains("setTimeout"), "a periodic re-check was added; R-215 asked for none");
    }
}
