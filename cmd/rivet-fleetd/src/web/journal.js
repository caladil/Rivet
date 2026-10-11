// **The journal** - `ADR-022` §9's query, in a browser.
//
// # What the operation does and does not do, because the difference is the
// # whole design of this page
//
// `rivet get log <boot> <limit> <cursor>` selects by **boot** and by **how
// many**, and by nothing else. `ADR-022` §9 lists severity, service,
// correlation and actor; `rivet_core::journal::Filter` implements every one of
// them, and none of them is wired to an argument — `ADR-006` §6.4's positional
// shape would have needed five optional positions, and the machine has a shell
// on it that does the same job better.
//
// So: **the machine selects, and this page filters what it selected.** That is
// a real difference and it is stated on the page rather than only here, because
// the failure it produces is the expensive one — see below.
//
// # The 200-record default, and what it cost once already
//
// A caller who names no number gets 200, counting from the END. `returned` less
// than `held` is ALSO what naming one boot out of thirty looks like, so a reader
// cannot tell an answer that was cut from one that was selected. On 2026-08-31 a
// search for `vc4` against that default came back empty — because a boot's
// kernel records are at its START and the default keeps the newest — and the
// conclusion drawn was that the machine had no kernel log at all. A whole
// `get kernel-log` subject was built and reverted before that was found.
//
// Three things follow, and they are the rules this viewer is built to:
//
// 1. **This page never sends a bare `get log`.** The limit is always explicit
//    and always shown in the control that chose it.
// 2. **`truncated` is rendered loudly whenever the machine sends it.** It is the
//    field that says WHICH of the two happened, and it appears only when the
//    limit was the answer.
// 3. **A filter that matches nothing says how many records it looked at and how
//    many the machine holds.** "No records" and "no records among the 200 you
//    fetched" are different sentences and only one of them is true.
//
// # Following
//
// `cmd/rivet`'s `follow` module holds a cursor and calls again; `ADR-035` §4A
// is the cursor. This does the same thing on a timer, which is **polling and is
// named as polling on the page**. The operation answers once from what is on
// the card at the moment it is asked — following is a property of the transport
// and never of the operation, so a view that called itself live would be
// claiming something the machine does not offer.

/// How often the follow asks again, and how many records it takes each time.
const FOLLOW_MS = 3000;
const FOLLOW_LIMIT = "500";

/// The limits offered. `all` is the word that turns the limit off — a word
/// rather than `0`, because *give me nothing* and *give me everything* must not
/// be the same request.
const LIMITS = ["200", "500", "2000", "10000", "all"];

/// Severity as `ADR-022` §3.2 spells it, most serious first.
const SEVERITIES = ["error", "warn", "info", "debug"];

VIEWS.journal = {
  async draw(body, d) {
    const state = {
      boot: "latest", limit: "2000", sev: new Set(SEVERITIES),
      svc: "", corr: "", text: "", records: [], answer: null, cursor: null,
      following: false, timer: null,
    };
    onLeave(() => { if (state.timer) clearInterval(state.timer); state.timer = null; });

    const wrap = el("div", "panel wide");
    const bar = el("div", "toolbar");
    const status = el("div", "count");
    const rows = el("div", "jscroll");
    const notes = el("div");

    // ---- the machine's selection ------------------------------------------
    const bootPick = el("select");
    const later = el("option", null, "latest boot");
    later.value = "latest";
    bootPick.append(later);
    const every = el("option", null, "every boot the journal holds");
    every.value = "all";
    bootPick.append(every);

    const limitPick = el("select");
    for (const l of LIMITS) {
      const o = el("option", null, l === "all" ? "no limit" : "last " + l + " records");
      o.value = l;
      if (l === state.limit) o.selected = true;
      limitPick.append(o);
    }

    const reread = el("button", "op", "Read");
    reread.type = "button";

    const follow = el("button", "op", "Follow");
    follow.type = "button";

    bar.append(el("label", null, "The machine selects"), bootPick, limitPick, reread, follow);

    // ---- what this page filters -------------------------------------------
    const filters = el("div", "toolbar");
    filters.append(el("label", null, "This page filters"));
    for (const sev of SEVERITIES) {
      const b = el("button", "op", sev);
      b.type = "button";
      b.setAttribute("aria-current", "true");
      b.onclick = () => {
        if (state.sev.has(sev)) state.sev.delete(sev); else state.sev.add(sev);
        b.setAttribute("aria-current", String(state.sev.has(sev)));
        paint();
      };
      filters.append(b);
    }
    const svcPick = el("select");
    const corrBox = el("input");
    corrBox.type = "text"; corrBox.placeholder = "correlation"; corrBox.size = 18;
    const textBox = el("input");
    textBox.type = "search"; textBox.placeholder = "text in the message"; textBox.size = 26;
    svcPick.onchange = () => { state.svc = svcPick.value; paint(); };
    corrBox.oninput = () => { state.corr = corrBox.value.trim(); paint(); };
    textBox.oninput = () => { state.text = textBox.value.trim().toLowerCase(); paint(); };
    filters.append(svcPick, corrBox, textBox);

    wrap.append(bar, filters, status, notes, rows);
    body.append(wrap);

    // ---- reading ----------------------------------------------------------
    async function read(append) {
      const words = ["log", state.boot, append ? FOLLOW_LIMIT : state.limit];
      if (append && state.cursor) words.push(state.cursor);
      const answer = await get(d.id, ...words);
      state.answer = answer;
      const fresh = (answer.records || []).map(parseRecord).filter((r) => r);
      state.records = append ? state.records.concat(fresh).slice(-20000) : fresh;
      // ABSENT means there is no new position to report, and the caller should
      // keep the one it has rather than treating absence as a reset.
      if (answer.cursor) state.cursor = answer.cursor;
      fillBoots(bootPick, answer, state);
      fillServices(svcPick, state);
      paint();
    }

    function refresh() {
      status.textContent = "asking " + d.name + "…";
      read(false).catch((e) => { status.textContent = ""; notes.textContent = ""; notes.append(said("bad", e.message)); });
    }

    reread.onclick = () => { state.boot = bootPick.value; state.limit = limitPick.value; state.cursor = null; refresh(); };
    bootPick.onchange = () => { state.boot = bootPick.value; state.cursor = null; refresh(); };
    limitPick.onchange = () => { state.limit = limitPick.value; state.cursor = null; refresh(); };

    follow.onclick = () => {
      state.following = !state.following;
      follow.setAttribute("aria-current", String(state.following));
      follow.textContent = state.following ? "Following — stop" : "Follow";
      if (state.timer) { clearInterval(state.timer); state.timer = null; }
      if (state.following) {
        state.timer = setInterval(() => {
          read(true).catch(() => { /* one poll that failed is not a reason to stop */ });
        }, FOLLOW_MS);
      }
    };

    // ---- painting ---------------------------------------------------------
    function paint() {
      const shown = state.records.filter((r) =>
        state.sev.has(r.sev || "info")
        && (!state.svc || r.svc === state.svc)
        && (!state.corr || (r.corr || "").includes(state.corr))
        && (!state.text || (r.msg || "").toLowerCase().includes(state.text)));

      rows.textContent = "";
      const table = el("table", "jrows");
      for (const r of shown) table.append(recordRow(r));
      rows.append(table);
      rows.scrollTop = rows.scrollHeight;

      const a = state.answer || {};
      status.textContent =
        shown.length + " shown of " + state.records.length + " fetched; this machine holds " +
        (a.held ?? "—") + " records in " + (a.boots || []).length + " boots" +
        (state.following ? " · following, polling every " + (FOLLOW_MS / 1000) + "s" : "");

      notes.textContent = "";
      // **The truncation, loudly.** It appears only when the limit was the
      // answer, and it is the field that separates "cut" from "selected".
      if (a.truncated) notes.append(said("bad", a.truncated));
      if (a.unreadable) {
        notes.append(said("bad",
          a.unreadable + " line(s) in this journal could not be read, and they are not dropped " +
          "silently: " + ((a["unreadable-detail"] || []).join("; ") || "no detail was given") +
          ". A non-zero count usually means a NEWER Rivet wrote them."));
      }
      if (a.since === "boot-gone") {
        notes.append(said("bad",
          "The cursor named a boot this journal no longer holds, so it could not be placed and " +
          "the answer holds everything the other inputs selected rather than nothing."));
      }
      if (!shown.length && state.records.length) {
        notes.append(said("",
          "Nothing matches among the " + state.records.length + " records fetched. That is not " +
          "the same as nothing in the journal — this machine holds " + (a.held ?? "—") +
          ", and only what the boot selector and the limit chose is on this page."));
      }
      if (a.source) {
        notes.append(el("div", "note",
          "Read from " + a.source + " — " + ((a.files || []).length) + " file(s). The boot " +
          "selector and the limit are the MACHINE'S selection; severity, service, correlation " +
          "and text are this page's, applied to what arrived."));
      }
    }

    refresh();
  },
};

function parseRecord(line) {
  try { return JSON.parse(line); } catch (e) { return null; }
}

function recordRow(r) {
  const tr = el("tr", "sev-" + (r.sev || "info"));
  // `ts` is the board's wall clock and `mono` is the monotonic counter. Both
  // are shown: on a Pi with no RTC the wall clock is wrong until something
  // corrects it and then JUMPS, and `mono` is true either way (ADR-021).
  tr.append(el("td", "when", (r.ts || "") + "  " + (r.mono ?? "")));
  tr.append(el("td", "sev", r.sev || ""));
  tr.append(el("td", "svc", r.svc || ""));
  tr.append(el("td", "msg", r.msg === undefined ? JSON.stringify(r) : r.msg));
  return tr;
}

/// Every boot the machine says it still holds, oldest first, as the machine
/// spells them. Retention drops the oldest file, so this is what it HAS rather
/// than what it has ever had.
function fillBoots(pick, answer, state) {
  const boots = answer.boots || [];
  const already = new Set(Array.from(pick.options).map((o) => o.value));
  for (const b of boots) {
    if (already.has(b)) continue;
    const o = el("option", null, b);
    o.value = b;
    pick.append(o);
  }
  pick.value = state.boot;
}

function fillServices(pick, state) {
  const seen = Array.from(new Set(state.records.map((r) => r.svc).filter((s) => s))).sort();
  const chosen = state.svc;
  pick.textContent = "";
  const any = el("option", null, "every service");
  any.value = "";
  pick.append(any);
  for (const s of seen) {
    const o = el("option", null, s);
    o.value = s;
    pick.append(o);
  }
  pick.value = chosen;
  // A service filter that survives a re-read only if that service is still in
  // what arrived — otherwise it would silently show nothing.
  if (pick.value !== chosen) { state.svc = ""; pick.value = ""; }
}
