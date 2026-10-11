// **Software** - what is installed on a machine, and what the operator has
// asked it to run.
//
// # Installing is not running, and the page keeps them apart
//
// `ADR-015` §3.2.2: **the manifest owns what an application IS** - its restart
// policy, what it listens on, what it connects to - and **the document owns
// whether the operator wants it running**. Installing a `.rvt` starts nothing;
// `enabled` is a key in `/state/config/machine.toml` and it is the operator's.
//
// So a row here carries two facts that a page which blended them could not
// tell apart: what the machine HAS, out of `/state/app`, and what the machine
// has been TOLD, out of its document. A thing can be installed and disabled, or
// enabled and not installed - the latter is ordinary, because a fleet layer
// writes configuration for software it is about to install.
//
// # Where each column comes from, and nothing is computed here
//
// | Column | The machine's answer it comes from |
// | :-- | :-- |
// | name, state, restarts, why it failed | `get status` |
// | installed | `get file /state/app` - one directory per installed application |
// | kind, listens, connects, provides | `get file /state/app/<n>/current/manifest.toml` |
// | enabled | `get status`, and `get desired-state` for the text that sets it |
// | version, installed, location | `get file` on `current` and the version directory it names (§"the drawer") |
// | size | `get file` on the version directory's own `install-size` record (R34, `.rulings/` R-163) |
// | shared Rivets | `get file /state/lib` |
//
// The manifests are read ONE AT A TIME, after the table is on screen, because
// the fleet server holds one connection per request and a machine with twenty
// applications would otherwise be twenty seconds of blank page. The drawer's
// own extra reads follow the same rule, lazily, on the row it was opened for.
//
// # System extensions
//
// `ADR-016`'s extension is designed and is not built: this build's package
// kinds are `update`, `application` and `library` (`rivet_core::package::kind`),
// there is no extension token and nothing installs one. The section says that
// rather than being absent, because a missing section reads as an oversight.
//
// # The drawer carries more than the row, on purpose — Wave 6, 2026-09-13
//
// `BRIEFING-001`'s complaint was exact: the expanded view repeated the
// collapsed one and added nothing but two buttons. The fix is not to remove
// the drawer, it is to put in it what was genuinely missing — the package's
// own version, when it was installed, its size and its location on disk, and
// the manifest's declarations spelled out rather than only summarised as
// pills. `installFacts` below is that read; it runs once, lazily, the first
// time a row is opened, on `current` — the symlink the installer names the
// running version with — and the version directory it points at.
//
// # The size is read, not walked — R34, `.rulings/` R-163, 2026-09-13
//
// This used to sum the version directory one level deep itself, because
// `get file`'s own `size_bytes` is a directory's inode weight and not what is
// inside it — and said so, because a directory nested deeper than one level
// was named but not counted. The installer now records the true total at
// install time (it already reads every file to write it, so it sums there),
// and this reads that record instead: one more `get file`, on the version
// directory's own `install-size`, and no walk at all. **An application
// installed before this existed has no record and says so — there is no
// fallback walk** (`.rulings/` R-163.1).
//
// # ONE capture for the machine, not one per application — 2026-09-20
//
// The maintainer's instruction was to take the captures off each application
// and keep a single capture button somewhere (.rulings/ R-332).
//
// ~~Every row gets a `Capture` button, which is what `BRIEFING-001` asked for:
// the table was already the place that told an operator which application to
// capture.~~ **That question
// stopped having an answer on 2026-09-20.** `ADR-058` replaced the DRM lease
// with a compositor, so:
//
// * **an application cannot photograph the screen any more**, only its own
//   surface — and under a compositor there may be several, with the launcher
//   behind whatever is running. A per-application capture was always the
//   narrower thing and only looked like a screenshot because nothing else was
//   ever on screen;
// * **`rivet-displayd` composites everything and can be asked at any moment**,
//   by an operator, without the drawing program's cooperation — which is the
//   capture worth having when a program is wedged (`ADR-058` §5.3);
// * so there is one file, `/state/app/rivet-displayd/data/screen.qoi`, and one
//   button. A column of buttons all reading the same file would be a menu of
//   identical choices.
//
// The row's *"this is the frame from the last time X held the display"* note
// went with it. It answered a question about leases; there are none.
//
// **The button PRESSES, then fetches** — the maintainer's ruling, 2026-09-22.
// It once only converted the frame already on disk and printed the `apply input
// capture` command for the operator to run; that was a screenshot button that
// took no screenshot. It now presses, waits for the new frame to land (the
// press returns before the compositor writes it), shows it and downloads it. A
// picture of *this* moment is the moment somebody asked about by pressing.

/// Where the installer puts one directory per application.
/// `rivet_core::service::APP_ROOT`.
const APP_ROOT = "/state/app";

/// Where a shared Rivet is installed. `rivet_core::package::library::LIB_DIR`
/// under `BUILT_IN_VOLUME`.
const LIB_ROOT = "/state/lib";

/// The desired-state document. `get desired-state` reports the path it read.
const DOCUMENT = "/state/config/machine.toml";

/// **The machine's own words for a service that keeps it reachable.**
///
/// `rivet_core::operation::service_restart::caveat` opens with this phrase and
/// attaches it to a plan only for the services in the supervisor's
/// `REACHABILITY_SET`. Matching on it is how this page learns the membership
/// from the machine instead of holding a second copy of the list — `ADR-011`.
///
/// **It is prose and that is a real weakness**, stated here rather than hidden:
/// the field `Restarting::reachability` exists, crosses the supervisor's socket
/// as a boolean, and is folded into the result's `next` sentence instead of
/// being published in `service-restart`'s OUTPUTS. Until it is published there
/// is no structural signal to read, and a reworded caveat would make this page
/// stop warning. See ADR-043 §7 for the ruling this wants.
const REACHABILITY_PHRASE = "keep this machine reachable";

VIEWS.software = {
  async draw(body, d) {
    const grid = el("div", "grid");
    body.append(grid);
    loading(grid, "asking " + d.name + " what it has…");

    let status, apps, libs, doc;
    try {
      status = await get(d.id, "status");
      apps = await get(d.id, "file", APP_ROOT);
      libs = await get(d.id, "file", LIB_ROOT);
      doc = await get(d.id, "desired-state");
    } catch (e) {
      grid.textContent = "";
      grid.append(panel("This machine did not answer", el("div", "fail", e.message), true));
      return;
    }
    grid.textContent = "";

    const installed = new Set((apps.entries || [])
      .filter((e) => e.type === "directory").map((e) => e.name));
    const services = status.services || [];
    const known = new Set(services.map((s) => s.name));

    // A machine may hold an application the document has never mentioned, and
    // the supervisor only knows the ones it has a definition for. Both belong
    // in the list; the difference is a column.
    const rows = services.map((s) => ({
      name: s.name, service: s, installed: installed.has(s.name),
    }));
    for (const name of installed) {
      if (!known.has(name)) rows.push({ name, service: null, installed: true });
    }
    rows.sort((a, b) => a.name.localeCompare(b.name));

    // **What the machine says about its own screen**, carried through rather
    // than worked out here — `ADR-011`, and the same rule the `mode` column
    // already obeys after this page once decided kiosk-ness by string-matching
    // a document. A supervisor older than this page sends neither field, both
    // are `undefined`, and no badge is drawn.
    const display = { master: status.display_master, leased: status.display_leased };

    grid.append(panel("The screen", screenPanel(d), true));
    grid.append(panel("Applications",
      softwareTable(d, rows.filter((r) => r.installed), doc, true, display), true));
    grid.append(panel("System services — part of the card image",
      softwareTable(d, rows.filter((r) => !r.installed), doc, false, display), true));
    grid.append(panel("Shared Rivets", sharedRivets(libs), true));
    grid.append(panel("System extensions", extensions(), true));
  },
};

// ---------------------------------------------------------------------------
// the table
// ---------------------------------------------------------------------------

function softwareTable(d, rows, doc, areApplications, display = {}) {
  const wrap = el("div");
  if (!rows.length) {
    wrap.append(el("div", "sub", "None."));
    return wrap;
  }
  const table = el("table");
  const head = el("tr");
  ["", "Name", "Enabled", "State", "What it is"].forEach((h) => head.append(el("th", null, h)));
  head.append(el("th", "num", "Restarts"));
  table.append(head);

  for (const row of rows) {
    const s = row.service;
    const tr = el("tr", "row");
    tr.append(el("td", null, "▸"));
    tr.append(el("td", null, row.name));

    const en = el("td");
    if (!s) en.append(el("span", "pill", "not defined"));
    else en.append(el("span", s.enabled ? "pill on" : "pill off", s.enabled ? "enabled" : "disabled"));
    tr.append(en);

    const st = el("td");
    if (!s) st.append(el("span", "sub", "—"));
    else st.append(el("span", s.state === "failed" ? "pill bad" : "pill", s.state));
    // **The screen, in the STATE column and not beside the manifest pills.**
    //
    // Every pill in "what it is" is read out of the manifest — it says what an
    // application ASKED FOR. These two are read out of `get status` and say what
    // the machine IS DOING, and putting them among the declarations is how a
    // reader ends up treating one as the other. On this very board four
    // services declare `devices display` and exactly one is drawing.
    //
    // **Taken from the machine's answer, never derived.** Not from `auto_run`,
    // not from `devices`, not from *is it started* — see the `display_leased`
    // note in `rivet-core`'s status OUTPUTS for the rollback that argument
    // comes from. If an older supervisor sends neither field, both are
    // undefined and no badge appears, which is the correct silence.
    if (display.master && row.name === display.master) {
      st.append(document.createTextNode(" "), el("span", "pill live", "display master"));
    }
    if (display.leased && row.name === display.leased) {
      st.append(document.createTextNode(" "), el("span", "pill live", "display leased"));
    }
    tr.append(st);

    // Filled in by the manifest walk below, for an application. A system
    // service has no manifest on the machine — its definition is compiled into
    // the supervisor — so the cell says so rather than staying blank.
    const what = el("td", "wrap");
    what.dataset.manifest = row.name;
    what.append(el("span", "sub", areApplications ? "reading its manifest…" : "compiled into the supervisor"));
    tr.append(what);

    tr.append(el("td", "num", s ? String(s.restarts ?? 0) : "—"));

    const drawer = el("tr", "drawer");
    drawer.hidden = true;
    const cell = el("td");
    cell.colSpan = 7;
    drawer.append(cell);

    let opened = false;
    const openDrawer = () => {
      drawer.hidden = false;
      tr.classList.add("open");
      tr.firstChild.textContent = "▾";
      if (!opened) { opened = true; drawRow(cell, d, row, doc); }
      return cell;
    };

    table.append(tr);
    table.append(drawer);

    tr.onclick = () => {
      if (drawer.hidden) { openDrawer(); return; }
      drawer.hidden = true;
      tr.classList.remove("open");
      tr.firstChild.textContent = "▸";
    };
  }
  wrap.append(table);

  if (areApplications) {
    wrap.append(el("div", "note",
      "Installed under " + APP_ROOT + ". Installing does not run anything: `enabled` lives in " +
      "the machine's document and is yours."));
    // One at a time, after the table exists. See the header.
    readManifests(d, rows.map((r) => r.name), wrap);
  } else {
    wrap.append(el("div", "note",
      "These are defined by the card image rather than installed. `enabled` is still a key in " +
      "the same document and is changed the same way."));
  }
  return wrap;
}

/// Walk the applications, filling in what each manifest says.
///
/// **Sequential on purpose.** `rivet-fleetd` opens one control-plane connection
/// per request, and a machine with twenty applications asked in parallel is
/// twenty connections at once against a limit of sixteen.
async function readManifests(d, names, wrap) {
  for (const name of names) {
    const cell = wrap.querySelector('[data-manifest="' + CSS.escape(name) + '"]');
    if (!cell) continue;
    try {
      const answer = await get(d.id, "file", APP_ROOT + "/" + name + "/current/manifest.toml");
      cell.textContent = "";
      if (!answer.present || answer.content === undefined) {
        cell.append(el("span", "sub", answer.content_omitted || "no manifest at that path"));
        continue;
      }
      const m = readManifest(answer.content);
      for (const pill of manifestPills(m)) cell.append(pill, document.createTextNode(" "));
      cell.dataset.text = answer.content;
    } catch (e) {
      cell.textContent = "";
      cell.append(el("span", "sub fail", e.message));
    }
  }
}

/// The manifest's keys, read as the flat TOML `cargo rivet package` emits.
///
/// Deliberately not a TOML parser: this reads `key = value` at the top level of
/// a file this project generates, refuses nothing, and reports what it found.
/// Anything it cannot read stays absent rather than becoming a guess.
function readManifest(text) {
  const out = {};
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (!line || line.startsWith("#") || line.startsWith("[")) continue;
    const at = line.indexOf("=");
    if (at < 0) continue;
    const key = line.slice(0, at).trim();
    let value = line.slice(at + 1).trim();
    if (value.startsWith("[")) {
      out[key] = value.slice(1, value.lastIndexOf("]")).split(",")
        .map((v) => v.trim().replace(/^"|"$/g, "")).filter((v) => v.length);
    } else {
      out[key] = value.replace(/^"|"$/g, "");
    }
  }
  return out;
}

function manifestPills(m) {
  const out = [];
  if (m.kind) out.push(el("span", m.kind === "kiosk" ? "pill kiosk" : "pill", m.kind));
  if ((m.listen || []).length) {
    out.push(el("span", "pill on", "network service · " + m.listen.join(" ")));
  }
  if ((m.provides || []).length) out.push(el("span", "pill", "provides " + m.provides.join(" ")));
  if ((m.connects || []).length) out.push(el("span", "pill", "connects " + m.connects.join(" ")));
  if ((m.library || []).length) out.push(el("span", "pill", "library " + m.library.join(" ")));
  if ((m.devices || []).length) out.push(el("span", "pill", "devices " + m.devices.join(" ")));
  if (!out.length) out.push(el("span", "sub", "a manifest with nothing declared"));
  return out;
}

// ---------------------------------------------------------------------------
// one row, opened
// ---------------------------------------------------------------------------

function drawRow(cell, d, row, doc) {
  cell.textContent = "";
  const s = row.service;

  if (s && s.reason) {
    cell.append(said("bad", s.reason));
  }

  // ---- the package: version, install date, size, location -----------------
  // **Installed rows only** — a system service is compiled into the image and
  // has no version directory under APP_ROOT to read.
  if (row.installed) {
    const pkg = el("div");
    pkg.append(el("h4", null, "Package"), el("div", "sub", "reading " + currentLink(row.name) + "…"));
    cell.append(pkg);
    installFacts(pkg, d, row.name);
  }

  // ---- the manifest's own contract, spelled out rather than only pills ----
  // Installed rows only — a system service has no manifest.toml on the
  // machine, exactly as the collapsed row's "What it is" column already says.
  if (row.installed) {
    const manifest = el("div");
    manifest.append(el("h4", null, "Manifest"), el("div", "sub", "reading its manifest…"));
    cell.append(manifest);
    fillManifestDrawer(d, row.name, manifest);
  }

  const facts = el("div");
  facts.append(el("h4", null, "What this machine says"));
  if (s) {
    facts.append(kv("State", s.state));
    facts.append(kv("Enabled in the document", String(s.enabled)));
    if (s.pid !== undefined) facts.append(kv("PID", String(s.pid)));
    facts.append(kv("Restarts", String(s.restarts ?? 0)));
    facts.append(kv("Ready", String(s.ready)));
  } else {
    facts.append(el("div", "sub",
      "Installed, and the supervisor has no definition for it — nothing in the document names " +
      "it, so it has never been started. Enabling it below is what gives it one."));
  }
  cell.append(facts);

  // ---- enable / disable ---------------------------------------------------
  const toggle = el("div");
  toggle.style.marginTop = "16px";
  toggle.append(el("h4", null, "Enabled"));
  cell.append(toggle);
  enableBox(toggle, d, row, doc);

  // ---- restart ------------------------------------------------------------
  if (s && s.enabled) {
    const restart = el("div");
    restart.style.marginTop = "16px";
    restart.append(el("h4", null, "Restart this service"));
    const where = el("div");
    const press = el("button", "op", "Restart " + row.name);
    press.type = "button";
    press.onclick = () => { press.disabled = true; planGate(where, d.id, ["service-restart", row.name]); };
    restart.append(press, where);
    cell.append(restart);
  }
}

/// **The enable/disable control, and the reason it is sometimes not offered.**
function enableBox(into, d, row, doc) {
  const text = doc.contents;
  if (typeof text !== "string") {
    into.append(said("bad",
      "This machine did not hand over its configuration document, so there is no text to " +
      "change and nothing is offered. `get desired-state` said: " +
      JSON.stringify(doc).slice(0, 300)));
    return;
  }
  const found = findEnabled(text, row.name);
  if (found.duplicate) {
    into.append(said("bad",
      "`[services." + row.name + "]` appears more than once in " + DOCUMENT + ". This page will " +
      "not edit a document it cannot place a change in exactly once."));
    return;
  }

  const now = found.absent ? false : (found.noKey ? false : found.value);
  const wanted = !now;

  const line = el("div");
  line.append(el("span", "sub",
    found.absent
      ? "The document does not mention this service, so it is disabled by omission."
      : (found.noKey
        ? "`[services." + row.name + "]` is in the document with no `enabled` key."
        : "The document says `enabled = " + String(found.value) + "`.")));
  into.append(line);

  const where = el("div");
  const press = el("button", wanted ? "op" : "op danger", wanted ? "Enable" : "Disable");
  press.type = "button";
  press.style.marginTop = "10px";
  into.append(press, where);

  if (!wanted) {
    // **A disable is the direction with the hazard**, so before offering one
    // this asks the machine what it says about restarting the service — which
    // is where the reachability caveat lives. See `REACHABILITY_PHRASE`.
    press.disabled = true;
    press.textContent = "checking what this machine says…";
    reachabilityCaveat(d.id, row.name).then((caveat) => {
      if (caveat) {
        press.remove();
        into.append(said("bad",
          "This service is not offered a disable here. The machine says: " + caveat));
        into.append(el("div", "note",
          "A reload withholds `enabled` for these and reports it. A restart withholds nothing " +
          "— so a document that disables the way in takes it away at the next restart, and on a " +
          "headless board there is no second way in. That is why this is not a button."));
        return;
      }
      press.disabled = false;
      press.textContent = "Disable";
    }).catch(() => {
      // The probe itself failed. Offer the change, and say that the check did
      // not answer — a silent green light would be worse than the button.
      press.disabled = false;
      press.textContent = "Disable";
      into.append(el("div", "note warn",
        "This machine did not answer the check for whether disabling this service would take " +
        "away the way in. The change below is still offered; read its plan."));
    });
  }

  press.onclick = () => {
    press.disabled = true;
    const change = editWords(text, row.name, found, wanted);
    if (change.why) { where.append(said("bad", change.why)); return; }
    planGate(where, d.id, change.words, () => {
      where.append(el("div", "note",
        "The document is changed and this machine is NOT. `enabled` is applied when the " +
        "supervisor re-reads the document — the Re-read the configuration panel below — or at " +
        "the next boot."));
    });
  };
}

/// Whether the machine attaches its reachability caveat to this service.
///
/// Returns the machine's own sentence, or `null`. A service the supervisor will
/// not plan a restart for — because the document disables it, or it does not
/// know the name — answers `null` too: there is nothing to warn about, because
/// a service that is already disabled cannot be disabled again.
async function reachabilityCaveat(id, name) {
  const { refused, value } = await run(id, "apply", "service-restart", name);
  if (!refused || !value.plan) return null;
  const all = (value.plan.untouched || []).join(" ");
  return all.includes(REACHABILITY_PHRASE)
    ? (value.plan.untouched || []).find((u) => u.includes(REACHABILITY_PHRASE))
    : null;
}

// ---------------------------------------------------------------------------
// composing the edit
// ---------------------------------------------------------------------------

/// Where `[services.<name>]`'s `enabled` is in the document, exactly.
function findEnabled(text, name) {
  const lines = text.split("\n");
  const header = "[services." + name + "]";
  let at = -1;
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].trim() !== header) continue;
    if (at >= 0) return { duplicate: true };
    at = i;
  }
  if (at < 0) return { absent: true };
  for (let i = at + 1; i < lines.length; i++) {
    const t = lines[i].trim();
    if (t.startsWith("[")) break;
    const m = /^enabled\s*=\s*(true|false)\s*$/.exec(t);
    if (m) return { header: at, key: i, lines, value: m[1] === "true" };
  }
  return { noKey: true, header: at, lines };
}

/// **The words that change it**, and the check that they can only land in one
/// place.
///
/// `apply edit --expect <text> --replace <text>` refuses text that occurs zero
/// times or more than once — the operation's own guard. This counts as well,
/// before anything is sent, because a refusal the page can predict is one the
/// operator should not have to read a plan to discover.
function editWords(text, name, found, wanted) {
  if (found.absent) {
    // Nothing to replace. `--append` adds the table, and it is offered only
    // when the document does not mention the service AT ALL — appending a
    // second `[services.<name>]` would make the document refuse to parse.
    return {
      words: ["edit", DOCUMENT, "--append",
        "\n[services." + name + "]\nenabled = " + String(wanted) + "\n"],
    };
  }
  const lines = found.lines;
  const from = found.header;
  const to = found.noKey ? found.header : found.key;
  const expect = lines.slice(from, to + 1).join("\n");
  const replace = found.noKey
    ? lines[from] + "\nenabled = " + String(wanted)
    : lines.slice(from, to).concat([
      lines[to].replace(/(true|false)(\s*)$/, String(wanted) + "$2"),
    ]).join("\n");

  const occurrences = text.split(expect).length - 1;
  if (occurrences !== 1) {
    return {
      why: "the text this change would replace occurs " + occurrences + " times in " + DOCUMENT +
        ", and `apply edit` replaces text that occurs exactly once. Nothing was sent.",
    };
  }
  return { words: ["edit", DOCUMENT, "--expect", expect, "--replace", replace] };
}

// ---------------------------------------------------------------------------
// the rest of the page
// ---------------------------------------------------------------------------

function sharedRivets(libs) {
  const wrap = el("div");
  if (!libs.present) {
    wrap.append(el("div", "sub",
      "Nothing is at " + LIB_ROOT + " on this machine, which is what a machine with no shared " +
      "Rivet installed looks like. The installer creates it with the first one."));
    return wrap;
  }
  const entries = (libs.entries || []).filter((e) => e.type === "directory");
  if (!entries.length) {
    wrap.append(el("div", "sub", LIB_ROOT + " exists and holds nothing."));
    return wrap;
  }
  const table = el("table");
  const head = el("tr");
  ["Reference", "Type"].forEach((h) => head.append(el("th", null, h)));
  table.append(head);
  for (const e of entries) {
    const tr = el("tr");
    tr.append(el("td", "mono", e.name), el("td", null, "shared Rivet"));
    table.append(tr);
  }
  wrap.append(table);
  wrap.append(el("div", "note",
    "NOTHING RUNS A LIBRARY. It is there for the applications that declare it, and it has no " +
    "`enabled` — there is nothing to enable."));
  return wrap;
}

function extensions() {
  const wrap = el("div");
  wrap.append(el("div", "sub",
    "None, and this build cannot have one. The package kinds this build knows are `update`, " +
    "`application` and `library`; there is no extension token, and no operation installs one. " +
    "This section is here so that its absence is stated rather than inferred."));
  return wrap;
}

// **`apply reload` moved to the Config screen** — `BRIEFING-001`: "does
// re-read the configuration really belong on this screen? Or does it belong
// under Config" — see `web/config.js`.

// ---------------------------------------------------------------------------
// the drawer's package facts: version, installed, size, location
// ---------------------------------------------------------------------------

/// `<APP_ROOT>/<name>/current` — the symlink the installer swaps to select a
/// version. Its target IS the version, because the version directory is named
/// for it (`rivet_core::package::install`); reading the link costs one `get
/// file` and needs no manifest field this build does not have.
function currentLink(name) {
  return APP_ROOT + "/" + name + "/current";
}

/// Read `current`, the version directory it names, and its recorded size,
/// then render Version / Installed / Installed size / Location into `into`.
///
/// **The size is READ, never walked** — R34, `.rulings/` R-163. The installer
/// sums every file it writes as it writes it and records the total at
/// `<versionDir>/install-size`; this reads that one file instead of visiting
/// the directory itself. `<APP_ROOT>/<name>/data` is never part of the sum —
/// `install-size` is the installer's own record of what IT wrote, and the
/// data directory is not its business (`ADR-015`) — so this is the
/// *installed* size, not how much the running application has since written.
///
/// **Absent means "not recorded", never zero.** An application installed
/// before this record existed has no `install-size` file, and `get file`
/// answers `present: false` for it — read plainly rather than defaulted to
/// `0`, which would claim an empty install.
async function installFacts(into, d, name) {
  const header = () => into.append(el("h4", null, "Package"));
  try {
    const current = await get(d.id, "file", currentLink(name));
    if (!current.present || current.type !== "symlink" || !current.link_target) {
      into.textContent = ""; header();
      into.append(el("div", "sub", "No `current` version is selected for this application."));
      return;
    }
    const version = current.link_target;
    const versionDir = APP_ROOT + "/" + name + "/" + version;
    const dir = await get(d.id, "file", versionDir);
    if (!dir.present || dir.type !== "directory") {
      into.textContent = ""; header();
      into.append(el("div", "sub", versionDir + " is not there, though `current` names it."));
      return;
    }

    const sizeRecord = await get(d.id, "file", versionDir + "/install-size");
    const recordedBytes = sizeRecord.present && typeof sizeRecord.content === "string"
      ? Number.parseInt(sizeRecord.content, 10)
      : NaN;

    into.textContent = ""; header();
    into.append(kv("Version", version));
    into.append(kv("Installed", dir.modified_ms === undefined ? "—" : new Date(dir.modified_ms).toLocaleString() +
      " — this board's own clock, and a Pi has no RTC until something has corrected it"));
    into.append(kv("Installed size", Number.isFinite(recordedBytes)
      ? bytes(recordedBytes)
      : "not recorded — this version was installed before this machine kept that record",
      Number.isFinite(recordedBytes) ? undefined : "sub"));
    into.append(kv("Location", versionDir));
  } catch (e) {
    into.textContent = ""; header();
    into.append(el("div", "sub fail", e.message));
  }
}

// ---------------------------------------------------------------------------
// the drawer's manifest contract, spelled out rather than only pills
// ---------------------------------------------------------------------------

async function fillManifestDrawer(d, name, into) {
  try {
    const answer = await get(d.id, "file", APP_ROOT + "/" + name + "/current/manifest.toml");
    into.textContent = "";
    into.append(el("h4", null, "Manifest"));
    if (!answer.present || answer.content === undefined) {
      into.append(el("div", "sub", answer.content_omitted || "no manifest at that path"));
      return;
    }
    const m = readManifest(answer.content);
    const pills = el("div");
    for (const pill of manifestPills(m)) pills.append(pill, document.createTextNode(" "));
    into.append(pills);
    into.append(el("pre", "mono manifest-text", answer.content));
  } catch (e) {
    into.textContent = "";
    into.append(el("h4", null, "Manifest"), el("div", "sub fail", e.message));
  }
}

// ---------------------------------------------------------------------------
// the capture, on the row — see this file's header for why it presses, waits
// and downloads
// ---------------------------------------------------------------------------

/// **Where the compositor writes its capture** — `rivet_core::compositor`'s
/// `CAPTURE_FILE`, inside `rivet-displayd`'s own data directory.
///
/// It was every application's own directory until `ADR-058` §5.3; see this
/// file's header for why there is one now.
const CAPTURE_PATH = APP_ROOT + "/rivet-displayd/data/screen.qoi";

/// The machine's screen: one button that takes a fresh frame and downloads it.
///
/// **It presses the capture.** Until 2026-09-22 this button only fetched the
/// frame already on disk and told the operator to run `rivet apply input
/// capture confirm` themselves — which the maintainer named for what it is: a
/// button that hands you a chore. It now does the whole act — press, wait for
/// the new frame, show it and download it — because that is what a Screenshot
/// button is.
function screenPanel(d) {
  const wrap = el("div");
  wrap.append(el("div", "sub",
    "The compositor's own frame — everything on the screen, whatever is drawing it."));

  const button = el("button", "op", "Screenshot");
  button.type = "button";
  const where = el("div");
  where.style.marginTop = "12px";
  button.onclick = async () => {
    button.disabled = true;
    where.textContent = "";
    loading(where, "taking a screenshot…");
    try {
      await takeScreenshot(where, d);
    } catch (e) {
      where.textContent = "";
      where.append(said("bad", e.message));
    } finally {
      button.disabled = false;
    }
  };
  wrap.append(button, where);
  return wrap;
}

/// Sleep between capture-file polls.
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/// `apply input capture` returns BEFORE the compositor writes the frame, so the
/// file lands a beat later. These bound the wait: twenty quarter-second polls
/// is five seconds — far more than a compositor needs, and short enough that a
/// wedged one is reported rather than hung on.
const CAPTURE_POLLS = 20;
const CAPTURE_POLL_MS = 250;

/// Press capture, wait for the new frame to land, then show and download it.
async function takeScreenshot(where, d) {
  // The frame already on disk, by modified time, so a fresh one can be told
  // from it. Read one byte of content and all of the stat — enough for the
  // timestamp without pulling a whole screen down just to poll.
  let before = 0;
  try {
    const meta = await get(d.id, "file", CAPTURE_PATH, "0", "1");
    if (meta.present === true) before = meta.modified_ms || 0;
  } catch (e) {
    // No capture yet, or the path is not readable — `before` stays 0 and any
    // frame that appears is newer than it.
  }

  // The press itself. A refusal here is the machine's own words.
  const pressed = await run(d.id, "apply", "input", "capture", "confirm");
  if (pressed.refused) {
    throw new Error(pressed.value.message || "the machine refused the capture.");
  }

  // Wait for the frame on disk to change — see CAPTURE_POLLS. Comparing the
  // modified time is what defeats the race the memory of this project keeps
  // naming: a fetch straight after the press returns the PREVIOUS frame.
  let landed = false;
  for (let i = 0; i < CAPTURE_POLLS; i++) {
    await sleep(CAPTURE_POLL_MS);
    try {
      const meta = await get(d.id, "file", CAPTURE_PATH, "0", "1");
      if (meta.present === true && (meta.modified_ms || 0) > before) {
        landed = true;
        break;
      }
    } catch (e) {
      // A transient read error is not a verdict; keep waiting.
    }
  }
  if (!landed) {
    throw new Error(
      "the machine did not write a new frame within " +
      ((CAPTURE_POLLS * CAPTURE_POLL_MS) / 1000) + " seconds — is anything drawing?");
  }

  loading(where, "downloading the frame…");
  const raw = await fetchCaptureBytes(d);
  if (!raw) { throw new Error("the frame vanished before it could be read."); }
  const image = decodeQoi(raw);

  const canvas = el("canvas");
  canvas.width = image.width;
  canvas.height = image.height;
  canvas.className = "shot";
  canvas.getContext("2d").putImageData(
    new ImageData(image.pixels, image.width, image.height), 0, 0);
  where.textContent = "";
  where.append(canvas);
  downloadCanvas(canvas, (d.name || d.id) + "-screenshot.png");
}

/// Read the whole capture file, following its windows until it says it is
/// complete — a full-screen frame is larger than one answer carries, so a
/// single read would hand back a truncated QOI.
async function fetchCaptureBytes(d) {
  const parts = [];
  let offset = 0;
  for (;;) {
    const answer = await get(d.id, "file", CAPTURE_PATH, String(offset));
    if (answer.present !== true) return null;
    if (answer.content === undefined || answer.content_encoding !== "base64") {
      throw new Error("what is at that path is not a QOI capture this page can read.");
    }
    const chunk = fromBase64(answer.content);
    if (!chunk) throw new Error("the capture did not decode.");
    parts.push(chunk);
    if (answer.content_complete === true) break;
    const advanced = answer.content_bytes || chunk.length;
    if (!advanced) break;
    offset = (answer.content_offset || 0) + advanced;
  }
  const total = parts.reduce((n, p) => n + p.length, 0);
  const out = new Uint8Array(total);
  let at = 0;
  for (const p of parts) { out.set(p, at); at += p.length; }
  return out;
}

/// Turn a rendered canvas into a PNG and hand it to the browser as a download.
function downloadCanvas(canvas, filename) {
  canvas.toBlob((blob) => {
    if (!blob) return;
    const url = URL.createObjectURL(blob);
    const a = el("a");
    a.href = url;
    a.download = filename;
    document.body.append(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
  }, "image/png");
}

/// RFC 4648 §4, into bytes. `null` when it does not decode.
function fromBase64(text) {
  try {
    const binary = atob(text);
    const out = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
    return out;
  } catch (e) {
    return null;
  }
}

// ---------------------------------------------------------------------------
// QOI
// ---------------------------------------------------------------------------
//
// The same decoder as `cmd/cargo-rivet/src/qoi.rs`, opcode for opcode, against
// the same encoder — `rivet-sdk/src/qoi.rs`. **The two eight-bit opcodes are
// tested before the two-bit tags**, because `0xfe & 0xc0` is `OP_RUN` and a
// decoder that matched on the tag first would read every literal colour as a
// run of 63 pixels. Everything here refuses rather than guessing: a wrong
// guess does not fail, it succeeds and hands back a plausible picture of
// nothing.

const QOI_HEADER = 14;
const QOI_END = 8;
const QOI_OP_RGB = 0xfe;
const QOI_OP_RGBA = 0xff;
const QOI_OP_INDEX = 0x00;
const QOI_OP_DIFF = 0x40;
const QOI_OP_LUMA = 0x80;
const QOI_OP_RUN = 0xc0;

function decodeQoi(b) {
  if (b.length < QOI_HEADER + QOI_END) {
    throw new Error("this is " + b.length + " bytes and the smallest possible QOI file is " +
      (QOI_HEADER + QOI_END) + ".");
  }
  if (b[0] !== 0x71 || b[1] !== 0x6f || b[2] !== 0x69 || b[3] !== 0x66) {
    throw new Error("this does not begin with `qoif`, so either it is a different file or it " +
      "did not arrive whole.");
  }
  const view = new DataView(b.buffer, b.byteOffset, b.byteLength);
  const width = view.getUint32(4);
  const height = view.getUint32(8);
  const channels = b[12];
  const colourspace = b[13];
  if (!width || !height) throw new Error("the header says " + width + "×" + height + ", and a picture with no pixels is not one.");
  if (channels !== 3 && channels !== 4) throw new Error("the header says " + channels + " channels, and QOI has 3 or 4.");
  if (colourspace > 1) throw new Error("the header's colourspace byte is " + colourspace + ", and QOI defines 0 and 1.");

  const count = width * height;
  const out = new Uint8ClampedArray(count * 4);
  const table = new Uint8Array(64 * 4);
  let r = 0, g = 0, bl = 0, a = 255;
  let at = QOI_HEADER, wrote = 0;

  const byteAt = (i) => {
    if (i >= b.length) {
      throw new Error("a chunk starting at byte " + i + " reads past the end of the file, which " +
        "is " + b.length + " bytes. What arrived is a TRUNCATED capture.");
    }
    return b[i];
  };
  const put = () => {
    const o = wrote * 4;
    out[o] = r; out[o + 1] = g; out[o + 2] = bl; out[o + 3] = a;
    wrote++;
  };

  while (wrote < count) {
    const tag = byteAt(at); at++;
    if (tag === QOI_OP_RGB) {
      r = byteAt(at); g = byteAt(at + 1); bl = byteAt(at + 2); at += 3;
    } else if (tag === QOI_OP_RGBA) {
      r = byteAt(at); g = byteAt(at + 1); bl = byteAt(at + 2); a = byteAt(at + 3); at += 4;
    } else if ((tag & 0xc0) === QOI_OP_RUN) {
      const run = (tag & 0x3f) + 1;
      if (wrote + run > count) {
        throw new Error("a run of " + run + " pixels starts at pixel " + wrote + " of " + count +
          ", so it runs off the end of the image.");
      }
      for (let i = 0; i < run; i++) put();
      continue;
    } else if ((tag & 0xc0) === QOI_OP_INDEX) {
      const s = (tag & 0x3f) * 4;
      r = table[s]; g = table[s + 1]; bl = table[s + 2]; a = table[s + 3];
    } else if ((tag & 0xc0) === QOI_OP_DIFF) {
      r = (r + ((tag >> 4) & 0x03) - 2) & 0xff;
      g = (g + ((tag >> 2) & 0x03) - 2) & 0xff;
      bl = (bl + (tag & 0x03) - 2) & 0xff;
    } else {
      // OP_LUMA, the fourth and last two-bit tag.
      const dg = (tag & 0x3f) - 32;
      const second = byteAt(at); at++;
      r = (r + dg - 8 + ((second >> 4) & 0x0f)) & 0xff;
      g = (g + dg) & 0xff;
      bl = (bl + dg - 8 + (second & 0x0f)) & 0xff;
    }
    const slot = ((r * 3 + g * 5 + bl * 7 + a * 11) % 64) * 4;
    table[slot] = r; table[slot + 1] = g; table[slot + 2] = bl; table[slot + 3] = a;
    put();
  }

  const tail = b.subarray(at);
  const END = [0, 0, 0, 0, 0, 0, 0, 1];
  if (tail.length !== QOI_END || END.some((v, i) => tail[i] !== v)) {
    throw new Error("the last pixel is decoded at byte " + at + " and the " + tail.length +
      " bytes after it are not QOI's end marker. Either this file carries more image than its " +
      "header admits, or something appended to it.");
  }
  return { width, height, channels, pixels: out };
}
