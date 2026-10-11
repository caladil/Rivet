// **The page's spine** - the transport, the catalogue binding, and the router.
//
// # Every number on this page is the machine's own
//
// `rivet-fleetd` forwards bytes and this file asks for them. Nothing here
// computes a figure about somebody else's state, and nothing here holds one
// between machines: `select` throws the previous machine's answers away rather
// than reusing them, because a fleet server that remembered a number would be a
// second opinion about a device it does not own (ADR-011).
//
// # The word vector, and why the tool list is read from the machine
//
// An operation's inputs are POSITIONAL - `rivet-core`'s `Input` says the name
// is a label for a position - and an MCP tool call names them. So
// `apply restart confirm` is not `{subject:"restart",confirmation:"confirm"}`:
// it is the word vector `["restart","confirm"]`, which binds to `subject` and
// then to the NEXT declared input whatever that is called.
//
// `rivet_remote::command::bind` is the rule this reproduces, and it reproduces
// it against **the machine's own `tools/list`** rather than against a table
// written here. Two reasons, and the second is the one that decides it:
//
// 1. A table here would be a second copy of the catalogue, which is the defect
//    `ADR-011` is about and which `rivet_remote` refuses for the same reason.
// 2. **A fleet holds machines running different builds.** Each answers with the
//    catalogue generated from ITS OWN types, so the only correct binding for a
//    given machine is the one that machine published.

const $ = (s, r) => (r || document).querySelector(s);
const el = (t, c, x) => {
  const n = document.createElement(t);
  if (c) n.className = c;
  if (x !== undefined) n.textContent = x;
  return n;
};
const bytes = (n) => {
  if (n === null || n === undefined) return "—";
  const u = ["B", "KiB", "MiB", "GiB", "TiB"]; let i = 0, v = Number(n);
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return (v < 10 && i > 0 ? v.toFixed(1) : Math.round(v)) + " " + u[i];
};
const pct = (n) => (n === null || n === undefined) ? "—" : Math.round(n) + "%";
/// Whole seconds into `1d 2h 3m` — used against `get boot`'s `uptime_seconds`,
/// which is the trustworthy duration on hardware with no real-time clock
/// (`ADR-021`; a wall-clock timestamp here would not be). Falls to whole
/// seconds only for an interval under a minute, so a machine seconds off a
/// reboot does not read as "0m".
const duration = (totalSeconds) => {
  if (totalSeconds === null || totalSeconds === undefined) return "—";
  let s = Math.max(0, Math.floor(Number(totalSeconds)));
  const d = Math.floor(s / 86400); s -= d * 86400;
  const h = Math.floor(s / 3600); s -= h * 3600;
  const m = Math.floor(s / 60); s -= m * 60;
  const parts = [];
  if (d) parts.push(d + "d");
  if (d || h) parts.push(h + "h");
  if (d || h || m) parts.push(m + "m");
  if (!parts.length) parts.push(s + "s");
  return parts.join(" ");
};

let devices = [], selected = null, view = "overview";

// **R39.** Whether THIS fleet server's own document carries `fleet-repository`
// — read once at `boot()`, the same request that already reads the register.
// Not a second opinion about a machine (`ADR-043` §2's principle 2): this is
// what `GET /api/fleet` says about the SERVER answering the request, on the
// same footing as everything else that endpoint has always answered.
let fleetAsCode = false;

// The selected machine's own tool list, as IT published it. Dropped and read
// again on every `select` - see the header.
let tools = null;

// ---------------------------------------------------------------------------
// the transport
// ---------------------------------------------------------------------------

/// **`.rulings/` R-215: a machine's sidebar dot, kept current rather than
/// decided once at boot.** `rpc` is the one function every call this page
/// makes to a machine passes through — `callTool` (every `get`/`run`) and
/// `readTools` (`tools/list`) alike — so marking the dot here, rather than in
/// each caller, is the whole fix and adds no second call: whatever the
/// Overview was already going to ask for is what refreshes it.
function dotFor(id) {
  return $('.card[data-id="' + CSS.escape(id) + '"] .dot');
}
function markUp(id) {
  const dot = dotFor(id);
  if (dot) { dot.className = "dot up"; dot.title = ""; }
}
function markDown(id, message) {
  const dot = dotFor(id);
  if (dot) { dot.className = "dot down"; dot.title = message || ""; }
}

/// One JSON-RPC message to one machine, through the fleet server.
async function rpc(id, body) {
  try {
    const r = await fetch("/mcp/" + encodeURIComponent(id), {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
    const outer = await r.json();
    if (outer.error) throw new Error(outer.error.message || "refused");
    markUp(id);
    return outer.result;
  } catch (e) {
    markDown(id, e.message);
    throw e;
  }
}

/// **A tool call, and whether the machine refused it.**
///
/// `cmd/rivet`'s MCP adapter makes a refusal a SUCCESSFUL call carrying
/// `isError`, so that an agent can tell *Rivet is broken* from *Rivet said no*.
/// A caller that read only the text would treat a refusal as an answer - and
/// for `apply`, the refusal IS the plan, so collapsing the two would throw away
/// the thing the operator has to read.
async function callTool(id, tool, args) {
  const result = await rpc(id, {
    jsonrpc: "2.0", id: 1, method: "tools/call",
    params: { name: tool, arguments: args },
  });
  const text = result && result.content && result.content[0] && result.content[0].text;
  if (text === undefined) throw new Error("the machine sent no content");
  let value;
  try {
    value = JSON.parse(text);
  } catch (e) {
    throw new Error("the machine's answer is not JSON this page can read: " + text.slice(0, 400));
  }
  return { refused: result.isError === true, value };
}

/// The machine's published tool list, keyed by verb.
async function readTools(id) {
  const result = await rpc(id, { jsonrpc: "2.0", id: 1, method: "tools/list" });
  const out = {};
  for (const t of (result.tools || [])) {
    // `rivet_get`, `rivet_apply`, `rivet_explain` - the prefix is the adapter's.
    const verb = String(t.name).replace(/^rivet_/, "");
    const schema = t.inputSchema || {};
    // Declaration ORDER is the binding order and JSON objects preserve it for
    // string keys, which is what `rivet-core` relies on when it renders the
    // schema and what `rivet_remote::command::bind` walks.
    const names = Object.keys(schema.properties || {});
    const several = names.filter((n) => (schema.properties[n] || {}).type === "array");
    out[verb] = { names, several };
  }
  return out;
}

/// **`rivet_remote::command::bind`, in a browser.**
///
/// Words fill the declared inputs left to right; the one input a verb may
/// declare several-word absorbs whatever is left after reserving a word for
/// each remaining single-word input. Returns the `arguments` object.
function bindWords(verb, words) {
  if (!tools || !tools[verb]) {
    throw new Error(
      "this machine has not published a `" + verb + "` tool, so there is nothing to bind " +
      "these words to and nothing was sent. `tools/list` is what was read.");
  }
  const { names, several } = tools[verb];
  const args = {};
  let left = words.slice();
  for (let at = 0; at < names.length && left.length; at++) {
    const name = names[at];
    if (several.includes(name)) {
      const reserved = names.slice(at + 1).filter((n) => !several.includes(n)).length;
      const take = Math.max(0, left.length - reserved);
      if (take > 0) { args[name] = left.slice(0, take); left = left.slice(take); }
    } else {
      args[name] = left[0];
      left = left.slice(1);
    }
  }
  if (left.length) {
    throw new Error(
      "`" + verb + "` declares " + names.length + " inputs — " + names.join(", ") +
      " — and there is nowhere to put " + left.map((w) => "`" + w + "`").join(", ") +
      ". Nothing was sent.");
  }
  return args;
}

/// Run one catalogue operation and hand back what the machine said.
async function run(id, verb, ...words) {
  return callTool(id, "rivet_" + verb, bindWords(verb, words.map(String)));
}

/// A read. Throws the machine's own refusal, because a `get` that was refused
/// has no answer to render.
async function get(id, ...words) {
  const { refused, value } = await run(id, "get", ...words);
  if (refused) throw new Error(value.message || JSON.stringify(value));
  return value;
}

// ---------------------------------------------------------------------------
// the shell
// ---------------------------------------------------------------------------

/// The pages reachable from the operation row. Each view registers itself here
/// so that adding one is one file and one entry.
const VIEWS = {};

function cardFor(d) {
  const b = el("button", "card");
  b.type = "button";
  b.dataset.id = d.id;
  const top = el("div", "top");
  top.append(el("span", "dot"), el("span", "name", d.name));
  if (d.role) top.append(el("span", "role", d.role));
  const sub = el("div", "sub", (d.address || "no address") + (d.location ? " · " + d.location : ""));
  b.append(top, sub);
  b.onclick = () => select(d.id);
  return b;
}

/// The boot-time first read of each device — `rpc` (via `callTool`) is what
/// actually sets the dot now, so this only has to make the call and swallow
/// whatever it throws, so one unreachable machine does not stop the rest of
/// `boot`'s loop from getting their own turn.
async function probe(d) {
  try {
    await callTool(d.id, "rivet_get", { subject: "status" });
  } catch (e) { /* rpc already marked the dot down with this same message */ }
}

async function boot() {
  const reg = await (await fetch("/api/fleet")).json();
  devices = reg.devices || [];
  fleetAsCode = reg.fleetAsCode === true;
  const host = $("#cards");
  host.textContent = "";
  if (!devices.length) {
    host.append(el("div", "sub", "The register is empty. rivet-provision writes the first record."));
    return;
  }
  devices.forEach((d) => host.append(cardFor(d)));
  for (const d of devices) { await probe(d); }   // one at a time: the server is one thread
  if (devices.length) select(devices[0].id);
}

function select(id) {
  selected = id;
  view = "overview";
  tools = null;
  document.querySelectorAll(".card").forEach((c) =>
    c.setAttribute("aria-current", String(c.dataset.id === id)));
  render();
}

function go(next) {
  view = next;
  render();
}

/// The device record the register holds for whatever is selected.
function current() {
  return devices.find((d) => d.id === selected);
}

/// **The operation row.** Top right of the machine header, as asked for.
///
/// Every entry is either a VIEW or an ACT. A view swaps what is under the
/// header; an act runs on the page it was pressed from, so the capture can be
/// taken without leaving the dashboard.
// **No `screenshot` entry.** A capture is now a button on the row it belongs
// to, on the Software page — `web/software.js` §"the capture" — so the
// dedicated page and this row's fifth entry both went in the same change
// (`AGENTS.md` §2 rule 5: removing a feature removes its prose too).
const OPERATIONS = [
  { key: "overview", label: "Overview" },
  { key: "software", label: "Software" },
  { key: "journal", label: "Journal" },
  { key: "config", label: "Config" },
  { key: "power", label: "Power", danger: true },
];

function operationRow() {
  const row = el("div", "ops");
  for (const op of OPERATIONS) {
    const b = el("button", op.danger ? "op danger" : "op", op.label);
    b.type = "button";
    b.setAttribute("aria-current", String(view === op.key));
    b.onclick = () => go(op.key);
    row.append(b);
  }
  return row;
}

/// What has to be stopped when the page under the header is replaced.
///
/// The journal's follow is a timer, and a timer whose view has gone on polling
/// a machine nobody is looking at is a page that keeps asking questions after
/// it has been closed.
let leaving = [];
function onLeave(stop) { leaving.push(stop); }

function render() {
  for (const stop of leaving) { try { stop(); } catch (e) { /* it is going anyway */ } }
  leaving = [];

  const d = current();
  const main = $("#main");
  main.textContent = "";
  if (!d) { main.append(el("div", "empty", "Select a machine.")); return; }

  const head = el("header", "machine");
  head.append(el("h1", null, d.name));
  if (d.address) head.append(el("span", "where", d.address + ":" + d.port));
  head.append(operationRow());
  main.append(head);
  main.append(el("div", "idline", d.id + (d.location ? "  ·  " + d.location : "")));

  const body = el("div");
  main.append(body);

  if (!d.address) {
    const p = el("div", "panel");
    p.append(el("h3", null, "Unreachable"), el("div", "fail", d.unreachable || "no address"));
    body.append(p);
    return;
  }

  const waiting = el("div", "panel");
  waiting.append(el("div", "sub", "asking " + d.name + "…"));
  body.append(waiting);

  // **The tool list first, then the view.** Nothing below can compose a command
  // until the machine has said what its operations take.
  const wanted = view;
  readTools(d.id).then((published) => {
    tools = published;
    if (view !== wanted || selected !== d.id) return;   // they moved on
    body.textContent = "";
    VIEWS[wanted].draw(body, d);
  }).catch((e) => {
    if (view !== wanted || selected !== d.id) return;
    body.textContent = "";
    const p = el("div", "panel wide");
    p.append(el("h3", null, "This machine did not answer"), el("div", "fail", e.message));
    p.append(el("div", "note",
      "The register still holds its record; only the machine is silent. Nothing on this page " +
      "can be composed without the catalogue the machine publishes, so no operation is offered."));
    body.append(p);
  });
}

function panel(title, body, wide) {
  const p = el("div", wide ? "panel wide" : "panel");
  p.append(el("h3", null, title));
  p.append(body);
  return p;
}

function kv(k, v, cls) {
  const row = el("div", "kv");
  row.append(el("span", "k", k), el("span", cls || "", v));
  return row;
}

function meter(usedBytes, totalBytes) {
  const wrap = el("div");
  const share = totalBytes ? (usedBytes / totalBytes) * 100 : 0;
  wrap.append(el("div", "big", pct(share)));
  const bar = el("div", "bar"); const fill = el("i");
  fill.style.width = Math.max(0, Math.min(100, share)) + "%";
  bar.append(fill); wrap.append(bar);
  wrap.append(el("div", "sub", bytes(usedBytes) + " of " + bytes(totalBytes)));
  return wrap;
}

/// A working note that replaces itself with whatever the caller resolves to.
function loading(where, what) {
  where.textContent = "";
  where.append(el("div", "sub", what));
}
