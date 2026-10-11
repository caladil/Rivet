// **Configuration** — the desired-state document for a machine, drawn from
// the SCHEMA THE MACHINE ITSELF PUBLISHES — plan §5.3, `.rulings/` R-162
// (Wave 3c).
//
// # No new route, and no second opinion about somebody else's state
//
// This view asks the TARGET MACHINE `get schema` through the transport every
// other page already uses (`POST /mcp/<device>`, see `web/core.js`'s `get`),
// and renders exactly the JSON Schema that machine answers with. It never
// reads a compiled-in schema of this server's own — that was the September
// 10th approach and it was rejected in full (`STATE.md` §0, plan §5.3):
// a fleet server and a board it manages can run different builds, and a form
// generated from the WRONG one is a form that lies about what a document may
// hold.
//
// The two functions that decide what the form looks like — `schemaFields`
// (declaration order, table/named-table recursion) and `extractGates`
// (`if`/`then`/`else` conditions) — are a straight transliteration of
// `cmd/rivet-fleetd/src/formfields.rs`, which exists ONLY so `cargo test` can
// hold the same algorithm against the schema `rivet-config` really emits.
// That Rust is never called at run time (this program still grows no new
// route); it is the tested reference this file is written to match, and its
// own module documentation says so at more length.
//
// # Save commits to the FLEET REPOSITORY, not to the machine — R39
//
// **This changed 2026-09-13** (`.rulings/` R-167 R39) and it is the one thing
// about this screen that is NOT still true of Wave 3c's original build. Save
// used to send `apply edit` straight to the machine; that ran against
// `ADR-044` §1.1's own single source of truth, which this screen is now built
// to keep: a machine's desired state lives in the fleet repository and the
// machine converges from it (`crate::code`'s poller, already built and
// running for every other write this server makes). So Save reads the whole
// document with `get desired-state` exactly as before, composes the WHOLE new
// document text from the form exactly as before, and — this is the only line
// that changed — POSTs it to `/api/commit/<device>` instead of driving
// `apply edit` on the machine. `apply edit` still exists; it is simply no
// longer what this button presses.
//
// **With no fleet repository configured, Save is disabled.** `fleetAsCode`
// (`web/core.js`, read once at `boot()` from `GET /api/fleet`) is this
// server's own `fleet-repository` setting — not a second opinion about a
// machine, `ADR-043` §2's principle 2, but the one fact THIS server needs to
// say why its own button does nothing. The reading, the ordering and the
// conditions below are unaffected: the form still reads and renders with no
// repository configured, and only the write is turned off.
//
// **The commit message names what was touched.** `touched` collects every
// field path an operator actually changed as they change it — the same
// fields `setField` would have written — and Save composes the message from
// that set rather than diffing two whole documents server-side: the client
// already knows precisely what changed, because it is the thing that changed
// it.
//
// **The credential guard is server-side** (`cmd/rivet-fleetd/src/commit.rs`)
// and this screen does not duplicate it — a browser is not where a security
// boundary belongs, and the same document reaches the same guard whether it
// arrived from this form or from `curl`.
//
// # What is preserved untouched, and why that is the safety property
//
// The TOML reader below (`parseDocument`) keeps every line it does not
// recognise as a schema-declared key — comments, blank lines, and whole
// sections this schema knows nothing about (`[applications.rivet-fleetd]`'s
// `fleet-repository`, say) — verbatim, and `serializeDocument` writes them
// back unchanged. Only a key this form actually rendered and the operator
// actually touched is ever rewritten. That is deliberate: a generic form
// walking a schema that (`applications`, `ADR-038` §4.2) KNOWS it cannot see
// the full shape of every table must never regenerate a section it does not
// understand, or it would silently drop settings no schema here declares.
//
// # `mode` and `[kiosk]` — R-164, R-165
//
// `mode` is rendered like any other field, in its own declared position.
// Changing it never touches `[kiosk]`, and `[kiosk]`'s own fields are always
// offered, whatever `mode` says — R-164: "changing `mode` never deletes the
// `[kiosk]` table, and a headless machine's `[kiosk]` table stays editable."
// The one thing added beyond walking the schema is `kioskModeNotice`, which
// says plainly — never refuses — when the document reads `mode = "kiosk"`
// with no `[kiosk]` table at all: a mode asked for and not configured.

// ---------------------------------------------------------------------------
// The schema walk — mirrors `cmd/rivet-fleetd/src/formfields.rs::walk`.
// ---------------------------------------------------------------------------

/// One property, in the position the schema declared it. `shape.kind` is
/// `"scalar"`, `"table"` or `"namedTables"` — see the Rust module for why a
/// `Table`'s `additionalProperties` is the boolean `false` and a
/// `NamedTables`' is the entry schema itself, which is the one thing this
/// function tells them apart by. `required` is this table's OWN
/// unconditional `required` array — `rivet_config::schema::table`'s
/// `Requirement::Always` list — read here rather than assumed, so a field a
/// build actually requires is never offered a blank "not set" the machine
/// would refuse.
function schemaFields(schemaNode) {
  const properties = (schemaNode && schemaNode.properties) || null;
  if (!properties || typeof properties !== "object") {
    throw new Error("this schema object carries no `properties`");
  }
  const gates = extractGates(schemaNode.allOf);
  const required = Array.isArray(schemaNode.required) ? schemaNode.required : [];
  const out = [];
  for (const key of Object.keys(properties)) {
    out.push(fieldFrom(key, properties[key], gates[key] || null, required.includes(key)));
  }
  return out;
}

function fieldFrom(key, schema, gate, required) {
  const prompt = schema.prompt || "";
  const description = schema.description || "";
  const jsonType = schema.type || "";
  let shape;
  if (jsonType === "object") {
    const additional = schema.additionalProperties;
    if (additional && typeof additional === "object") {
      shape = { kind: "namedTables", fields: schemaFields(additional) };
    } else {
      shape = { kind: "table", fields: schemaFields(schema) };
    }
  } else {
    shape = { kind: "scalar", jsonType, enumValues: Array.isArray(schema.enum) ? schema.enum : [] };
  }
  return { key, prompt, description, gate, shape, required };
}

/// Every gate an `allOf` block declares, keyed by the property it gates —
/// `formfields.rs::extract_gates`, read the same way: a property named in
/// `then.required` OR in `else.not.anyOf[].required` is gated on the `if`
/// block's one named sibling equalling its `const`.
function extractGates(allOf) {
  const out = {};
  if (!Array.isArray(allOf)) return out;
  for (const item of allOf) {
    const ifProps = item && item.if && item.if.properties;
    if (!ifProps || typeof ifProps !== "object") continue;
    for (const on of Object.keys(ifProps)) {
      const when = ifProps[on] && ifProps[on].const;
      if (typeof when !== "string") continue;
      const gate = { on, when };
      const thenRequired = (item.then && item.then.required) || [];
      for (const k of thenRequired) out[k] = gate;
      const anyOf = (item.else && item.else.not && item.else.not.anyOf) || [];
      for (const clause of anyOf) {
        for (const k of (clause && clause.required) || []) out[k] = gate;
      }
    }
  }
  return out;
}

/// Whether a gated field's condition is satisfied against the sibling values
/// gathered so far in ITS OWN table — `formfields.rs::applies`.
function gateApplies(gate, values) {
  if (!gate) return true;
  return values[gate.on] === gate.when;
}

/// **R-164, R-165.** The one mismatch a document may carry, made visible and
/// never refused — `formfields.rs::kiosk_mode_notice`, the same rule.
function kioskModeNotice(mode, kioskTablePresent) {
  if (mode === "kiosk" && !kioskTablePresent) {
    return "`mode` is set to `kiosk`, but this document has no `[kiosk]` table yet — nothing is " +
      "configured for it. Configure `[kiosk]` below, or change `mode`.";
  }
  return null;
}

// ---------------------------------------------------------------------------
// A small TOML reader and writer — enough for this schema's own shapes, and
// deliberately no more. See this file's header for what it preserves.
// ---------------------------------------------------------------------------

/// Parse `text` into `{ order: [sectionPath,…], sections: Map<path, {header,
/// entries}> }`. `""` is the top-level section, with `header: null`. Every
/// `entries` item is `{type:"header"}` (never appears; the header is its own
/// field), `{type:"kv", key, raw, value}` — `value` is the DISPLAY STRING,
/// already unquoted — or `{type:"other", raw}` for anything this reader does
/// not recognise as a schema-shaped key, kept to be written back unchanged.
function parseDocument(text) {
  const lines = text.split(/\r\n|\n/);
  if (text.endsWith("\n") && lines[lines.length - 1] === "") lines.pop();

  const sections = new Map();
  const order = [];
  function ensure(path) {
    if (!sections.has(path)) {
      sections.set(path, { header: path === "" ? null : "[" + path + "]", entries: [] });
      order.push(path);
    }
    return sections.get(path);
  }
  let current = ensure("");

  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    const trimmed = line.trim();
    const header = /^\[([^[\]]+)\]\s*$/.exec(trimmed);
    if (header) {
      current = ensure(header[1].trim());
      i += 1;
      continue;
    }
    const kv = /^([A-Za-z0-9_-]+)\s*=\s*(.*)$/.exec(line);
    if (kv) {
      const key = kv[1];
      let valueText = kv[2];
      let raw = line;
      // A triple-quoted string may span lines. Consume forward to the
      // closing `"""`.
      if (valueText.trimStart().startsWith('"""') && !closesTripleQuote(valueText)) {
        const collected = [line];
        i += 1;
        while (i < lines.length && !lines[i].includes('"""')) {
          collected.push(lines[i]);
          i += 1;
        }
        if (i < lines.length) collected.push(lines[i]);
        raw = collected.join("\n");
        valueText = raw.slice(raw.indexOf("=") + 1).trim();
      }
      current.entries.push({ type: "kv", key, raw, value: displayValue(valueText) });
      i += 1;
      continue;
    }
    current.entries.push({ type: "other", raw: line });
    i += 1;
  }
  return { order, sections };
}

function closesTripleQuote(valueText) {
  const t = valueText.trimStart();
  return t.length > 3 && t.endsWith('"""') && t !== '"""';
}

/// A parsed value token to the string a form field shows and edits.
function displayValue(token) {
  const t = token.trim();
  if (t.startsWith('"""')) {
    let inner = t.slice(3, t.endsWith('"""') && t.length > 5 ? t.length - 3 : t.length);
    // TOML's own rule for a multi-line basic string: a newline immediately
    // after the opening delimiter is trimmed and carries no content of its
    // own. `tomlLiteral` below always writes one when it re-opens the
    // string, so stripping it here is what makes the two agree with each
    // other on a round trip rather than growing a blank line every save.
    if (inner.startsWith("\n")) inner = inner.slice(1);
    else if (inner.startsWith("\r\n")) inner = inner.slice(2);
    return inner;
  }
  if (t.startsWith('"')) {
    const end = t.lastIndexOf('"');
    return unescapeToml(end > 0 ? t.slice(1, end) : t.slice(1));
  }
  if (t === "true" || t === "false") return t;
  if (/^-?\d+$/.test(t)) return t;
  if (t.startsWith("[") && t.endsWith("]")) {
    return t
      .slice(1, -1)
      .split(",")
      .map((s) => s.trim())
      .filter((s) => s.length > 0)
      .map((s) => (s.startsWith('"') ? unescapeToml(s.slice(1, s.length - 1)) : s))
      .join(", ");
  }
  return t;
}

function unescapeToml(s) {
  return s.replace(/\\(.)/g, (_, c) => ({ n: "\n", t: "\t", r: "\r", '"': '"', "\\": "\\" }[c] || c));
}

const TOML_ESCAPES = { "\\": "\\\\", '"': '\\"', "\n": "\\n", "\t": "\\t", "\r": "\\r" };
function escapeToml(s) {
  return s.replace(/[\\"\n\t\r]/g, (c) => TOML_ESCAPES[c]);
}

/// The display string a form field holds, back to a legal TOML value token
/// for `jsonType` — the write half of `displayValue`.
function tomlLiteral(displayString, jsonType) {
  if (jsonType === "boolean") return displayString === "true" ? "true" : "false";
  if (jsonType === "integer") return /^-?\d+$/.test(displayString) ? displayString : "0";
  if (jsonType === "array") {
    const items = displayString.split(",").map((s) => s.trim()).filter((s) => s.length > 0);
    return "[" + items.map((s) => '"' + escapeToml(s) + '"').join(", ") + "]";
  }
  if (displayString.includes("\n")) return '"""\n' + displayString + '\n"""';
  return '"' + escapeToml(displayString) + '"';
}

function ensureSection(doc, path) {
  if (!doc.sections.has(path)) {
    doc.sections.set(path, { header: path === "" ? null : "[" + path + "]", entries: [] });
    doc.order.push(path);
  }
  return doc.sections.get(path);
}

function sectionValues(doc, path) {
  const section = doc.sections.get(path);
  const out = {};
  if (!section) return out;
  for (const e of section.entries) if (e.type === "kv") out[e.key] = e.value;
  return out;
}

function setField(doc, path, key, displayString, jsonType, optional) {
  if (optional && displayString === "") {
    const section = doc.sections.get(path);
    if (section) section.entries = section.entries.filter((e) => !(e.type === "kv" && e.key === key));
    return;
  }
  const section = ensureSection(doc, path);
  const raw = key + " = " + tomlLiteral(displayString, jsonType);
  const existing = section.entries.find((e) => e.type === "kv" && e.key === key);
  if (existing) {
    existing.raw = raw;
    existing.value = displayString;
  } else {
    section.entries.push({ type: "kv", key, raw, value: displayString });
  }
}

function removeSection(doc, path) {
  if (!doc.sections.has(path)) return;
  doc.sections.delete(path);
  doc.order = doc.order.filter((p) => p !== path && !p.startsWith(path + "."));
}

/// Every named entry currently under `topLevel.` — `"network"` finds
/// `"network.eth0"`, `"network.resolver"`, and so on, in the order they
/// appear in the document (not the order the operator typed them, which
/// this reader never knew).
function namedEntries(doc, topLevel) {
  const prefix = topLevel + ".";
  return doc.order.filter((p) => p.startsWith(prefix)).map((p) => p.slice(prefix.length));
}

function serializeDocument(doc) {
  const parts = [];
  for (const path of doc.order) {
    const section = doc.sections.get(path);
    if (!section) continue;
    if (section.header) parts.push(section.header);
    for (const e of section.entries) parts.push(e.raw);
  }
  return parts.join("\n") + "\n";
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

VIEWS.config = {
  async draw(body, d) {
    const wrap = el("div", "config-view");
    body.append(wrap);
    loading(wrap, "Asking " + d.name + " for its configuration schema…");

    let schemaAnswer;
    let docAnswer;
    try {
      schemaAnswer = await get(d.id, "schema");
      docAnswer = await get(d.id, "desired-state");
    } catch (e) {
      wrap.textContent = "";
      wrap.append(panel("Failed to load", el("div", "fail", e.message), true));
      return;
    }

    let fields;
    try {
      fields = schemaFields(schemaAnswer.schema);
    } catch (e) {
      wrap.textContent = "";
      wrap.append(panel(
        "This machine's schema could not be read",
        el("div", "fail", e.message),
        true,
      ));
      return;
    }

    const originalText = docAnswer.present ? docAnswer.contents : "";
    const doc = parseDocument(originalText);

    // **R39.** Every field path an operator actually changes, in the order
    // they touch it — `fieldRow`'s `onchange` and the named-entry add/remove
    // buttons below all push into this. Save composes the commit message
    // from it rather than diffing two whole documents.
    const touched = new Set();

    wrap.textContent = "";
    const form = el("form");
    form.onsubmit = (ev) => ev.preventDefault();
    wrap.append(form);

    if (!fleetAsCode) {
      form.append(el("div", "note warn",
        "Save is disabled: this fleet server has no repository configured. Set " +
        "`fleet-repository` under [applications.rivet-fleetd] in THIS SERVER'S OWN document " +
        "(the one at its own Config screen, not " + d.name + "'s) to turn it on."));
    }

    // **Every scalar top-level field shares one panel** ("Configuration");
    // every `[table]` and named-table section gets its own — the same flat
    // list of `.panel` cards every other view on this page builds, none of
    // them nested inside another. `renderTopLevel` appends into `form`
    // directly for the latter and into `scalars` for the former.
    const scalars = el("div");
    for (const field of fields) renderTopLevel(form, doc, field, scalars, touched);
    if (scalars.childElementCount) form.prepend(panel("Configuration", scalars));

    const saveBar = el("div", "save-bar");
    const where = el("div");
    const saveBtn = el("button", "op", "Review the change");
    saveBtn.type = "button";
    saveBtn.disabled = !fleetAsCode;
    saveBtn.onclick = () => {
      if (!form.reportValidity()) return;
      const newText = serializeDocument(doc);
      if (newText === originalText) {
        where.textContent = "";
        where.append(el("div", "sub", "Nothing on this screen was changed."));
        return;
      }
      // **Disabled and left disabled once a plan is asked for** — the same
      // convention `web/power.js` and `web/software.js` use: the box that
      // appears in `where` offers Commit and Cancel, and a fresh "Review the
      // change" would need this view redrawn, not this button re-armed.
      saveBtn.disabled = true;
      where.textContent = "";

      const summary = touched.size ? [...touched].join(", ") : "the configuration";
      const message = "Edit " + d.name + "'s configuration: " + summary;

      // **The same visual shape `planGate`'s `planBox` uses** (`web/plan.js`)
      // — a `.plan` card, `.acts` for its buttons — even though this is not
      // a machine's own plan: there is no plan to ask for here, because
      // committing to git is not an operation any machine's catalogue has.
      // What is shown is composed by THIS screen rather than quoted from a
      // machine, which is why it says so plainly rather than borrowing
      // `planGate`'s "Nothing has changed yet" heading.
      const box = el("div", "plan");
      box.append(el("h4", null, "Nothing has been committed yet"));
      box.append(el("div", "consequence",
        "This commits to the fleet repository, on the branch this server is configured " +
        "with. " + d.name + " converges from it on its next cycle — nothing is sent to " +
        d.name + " directly."));
      box.append(el("div", "note", message));
      const acts = el("div", "acts");
      const goBtn = el("button", "go", "Commit");
      goBtn.type = "button";
      const cancelBtn = el("button", "cancel", "Cancel");
      cancelBtn.type = "button";
      cancelBtn.onclick = () => { where.textContent = ""; saveBtn.disabled = false; };
      goBtn.onclick = async () => {
        goBtn.disabled = true;
        cancelBtn.disabled = true;
        goBtn.textContent = "committing…";
        try {
          const r = await fetch("/api/commit/" + encodeURIComponent(d.id), {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
              expected: docAnswer.present ? originalText : null,
              document: newText,
              message,
            }),
          });
          const outer = await r.json();
          if (outer.error) throw new Error(outer.error.message || "refused");
          where.textContent = "";
          where.append(el("div", "note",
            "Committed " + String(outer.commit || "").slice(0, 12) + " to the fleet " +
            "repository. " + d.name + " converges from it on the next poll — or press " +
            "\"Re-read the configuration\" below after the next `rivet-fleetd converge`."));
        } catch (e) {
          where.textContent = "";
          where.append(panel("Failed to commit", el("div", "fail", e.message), true));
          saveBtn.disabled = false;
        }
      };
      acts.append(goBtn, cancelBtn);
      box.append(acts);
      where.append(box);
    };
    saveBar.append(saveBtn);
    form.append(saveBar, where);

    wrap.append(panel("Re-read the configuration", reloadBox(d), true));
  },
};

/// One top-level property — a plain field, a `[table]`, or a named-table
/// section. Top-level gates are evaluated against the document's OTHER
/// top-level values; nothing in this schema gates a top-level key today, but
/// the same function handles it either way rather than special-casing "none
/// yet". A scalar's row goes into `scalars` (one shared panel); everything
/// else gets its own panel appended straight into `form`.
function renderTopLevel(form, doc, field, scalars, touched) {
  const topValues = sectionValues(doc, "");
  if (!gateApplies(field.gate, topValues)) return;

  if (field.shape.kind === "scalar") {
    // **R-168.** `version` is the schema version this build reads
    // (`SCHEMA_VERSION`), never a setting — fixed by rebuilding, not by
    // typing. Detected from the schema's own `const` rather than the key
    // name alone, so a build that stops constraining it stops disabling it.
    const readOnly = field.key === "version";
    scalars.append(fieldRow(field, doc, "", { readOnly }, touched));
    return;
  }

  if (field.shape.kind === "table") {
    const box = el("div");
    const values = sectionValues(doc, field.key);
    for (const nested of field.shape.fields) {
      if (!gateApplies(nested.gate, values)) continue;
      box.append(fieldRow(nested, doc, field.key, {}, touched));
    }
    if (field.key === "kiosk") {
      const notice = kioskModeNotice(topValues.mode, doc.sections.has("kiosk"));
      if (notice) box.prepend(el("div", "note warn", notice));
    }
    form.append(panel(field.prompt || field.key, box));
    return;
  }

  // namedTables
  if (field.shape.fields.length === 0) {
    // `applications`, and anything else this schema declares no shape for —
    // ADR-038 §4.2. Nothing safe to offer; the document round-trip still
    // preserves whatever is there untouched.
    return;
  }
  const box = el("div");
  // **`[network.resolver]` is a reserved sub-table with a shape of its own**
  // (`nameservers`, `search`) — the schema's own description of `network`
  // names it explicitly, but `additionalProperties` still publishes ONE
  // entry shape (`INTERFACE`) for every key, because JSON Schema has no way
  // to say "except this one name". Offering `resolver` the interface fields
  // would show the wrong form for it, so it is left off this list — the
  // same "nothing safe to offer" rule as a `NamedTables` with no fields at
  // all, applied to one reserved name instead of a whole table. The
  // document round-trip still preserves it untouched.
  const reserved = field.key === "network" ? ["resolver"] : [];
  const entries = namedEntries(doc, field.key).filter((name) => !reserved.includes(name));
  for (const name of entries) {
    box.append(namedEntryBox(field, doc, name, touched));
  }
  box.append(addEntryRow(field, doc, box, touched));
  form.append(panel(field.prompt || field.key, box));
}

function namedEntryBox(field, doc, name, touched) {
  const path = field.key + "." + name;
  const wrap = el("div", "named-entry");
  const head = el("div", "named-entry-head");
  head.append(el("strong", null, name));
  const del = el("button", "op danger small", "Remove");
  del.type = "button";
  del.onclick = () => {
    removeSection(doc, path);
    wrap.remove();
    touched.add(path + " (removed)");
  };
  head.append(del);
  wrap.append(head);
  const values = sectionValues(doc, path);
  for (const nested of field.shape.fields) {
    if (!gateApplies(nested.gate, values)) continue;
    wrap.append(fieldRow(nested, doc, path, {}, touched));
  }
  return wrap;
}

function addEntryRow(field, doc, box, touched) {
  const row = el("div", "named-entry-add");
  const nameInput = el("input");
  nameInput.type = "text";
  nameInput.placeholder = "New " + field.key.replace(/s$/, "") + " name…";
  const addBtn = el("button", "op small", "Add");
  addBtn.type = "button";
  const reserved = field.key === "network" ? ["resolver"] : [];
  addBtn.onclick = () => {
    const name = nameInput.value.trim();
    if (!name || reserved.includes(name) || namedEntries(doc, field.key).includes(name)) return;
    ensureSection(doc, field.key + "." + name);
    const entry = namedEntryBox(field, doc, name, touched);
    row.before(entry);
    touched.add(field.key + "." + name + " (added)");
    nameInput.value = "";
  };
  row.append(nameInput, addBtn);
  return row;
}

/// One field's label, help text and input — the schema's `prompt` as the
/// label and `description` as the help text, exactly as plan §5.2/§5.3 ask.
function fieldRow(field, doc, sectionPath, opts, touched) {
  const row = el("div", "field");
  const label = el("label", null, field.prompt || field.key);
  row.append(label);

  const current = sectionValues(doc, sectionPath)[field.key] || "";
  const jsonType = field.shape.jsonType;
  // **Required only where the schema's own unconditional `required` names
  // this key.** A conditionally-required key — `address` under `method =
  // "static"` — is not in that list (`rivet_config::schema::table` only
  // collects `Requirement::Always`), so it is offered without the browser's
  // native validation; the schema's own `if`/`then` is what governs it, and
  // this form does not re-derive a second copy of that rule for the HTML
  // attribute. What IS enforced here is the plain case: a field every
  // document must carry, whatever else it says.
  const required = Boolean(field.required) && !opts.readOnly;
  let input;
  if (jsonType === "boolean" || field.shape.enumValues.length > 0) {
    input = el("select");
    const options = jsonType === "boolean" ? ["true", "false"] : field.shape.enumValues;
    if (!options.includes(current) && current !== "") {
      input.append(new Option(current, current));
    }
    if (!required || current === "") input.append(new Option(required ? "(choose one)" : "(not set)", ""));
    for (const opt of options) input.append(new Option(opt, opt));
    input.value = current;
  } else if (current.includes("\n")) {
    // **A `<textarea>`, not `<input type=text>`.** A single-line text input
    // cannot produce a newline for `tomlLiteral` to notice and triple-quote
    // — Enter does not reach the value of an `<input>`. Anything whose
    // CURRENT value already carries a newline gets the same treatment
    // rather than silently losing it to `displayValue`'s collapse.
    input = el("textarea");
    input.value = current;
    input.rows = Math.max(3, current.split("\n").length);
  } else {
    input = el("input");
    input.type = "text";
    input.value = current;
  }
  input.disabled = Boolean(opts.readOnly);
  input.required = required;
  if (opts.readOnly) {
    // **The maintainer asked that it be more obvious the version cannot be
    // edited** (`.rulings/` R-168). A disabled `<select>`/`<input>` alone reads as
    // "this happens to be empty", not "this cannot be changed here" — so a
    // field marked `readOnly` gets a whole sentence of its own, not just a
    // small trailing label.
    row.classList.add("field-locked");
    label.append(el("span", "sub locked", " — fixed by this build, not editable here"));
    row.append(el("div", "note",
      "This is the settings FORMAT version this build reads, not a setting on " +
      "the machine. It changes only when the software does."));
  }
  if (required) label.append(el("span", "sub", " (required)"));

  input.onchange = () => {
    setField(doc, sectionPath, field.key, input.value, jsonType, !required);
    touched.add(sectionPath ? sectionPath + "." + field.key : field.key);
  };
  row.append(input);
  if (field.description) row.append(el("div", "help", field.description));
  return row;
}

/// `apply reload` — what makes an edited document true of a running machine.
/// Unchanged in substance from before Wave 6 moved it here.
function reloadBox(d) {
  const wrap = el("div");
  wrap.append(el("div", null,
    "A change to the document is not a change to the machine. This asks the supervisor to read " +
    "the document again."));
  wrap.append(el("div", "note",
    "A reload applies everything except the services that keep this machine reachable, and " +
    "reports what it withheld rather than discarding it. A memory cap is withheld too: a new " +
    "one is taken only at the next boot."));
  const where = el("div");
  const press = el("button", "op", "Re-read the configuration");
  press.type = "button";
  press.style.marginTop = "12px";
  press.onclick = () => { press.disabled = true; planGate(where, d.id, ["reload"]); };
  wrap.append(press, where);
  return wrap;
}
