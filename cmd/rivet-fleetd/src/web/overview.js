// **The machine status dashboard** - the page this portal grew out of.
//
// Unchanged in substance by the management surface: it still asks the machine
// questions and renders the machine's own answers. What moved is that it is
// now one VIEW among several rather than the whole of `draw`.
//
// # Wave 6 (`docs/PLAN-Feature-Set-2026-09-12.md` §8), built 2026-09-13
//
// The maintainer's own review of the first cut (`BRIEFING-001`) asked for
// eight things here: drop the disk cards, add uptime, the booted slot, the
// Rivet version, and thermals, explain `phase` instead of showing a bare
// word, fold `auto_run` under `mode`, drop "sampled twice", and make the
// process list sortable and filterable. All eight are below.
//
// # The Rivet version — R33, `.rulings/` R-163, 2026-09-13
//
// Built after this page shipped without it: no `get` subject answered a
// build or image version, so the Boot card said so rather than showing
// nothing. `get status` now carries `version`, read once from the same
// string `rivet --version` prints, and the Boot card shows it — ABSENT
// rather than a guess on a supervisor older than this field, which reads no
// differently from the machine that could not answer at all.

/// The phase word IS the supervisor's (`AGENTS.md` §5, `ADR-017` §3.2) and is
/// never replaced — this is a short gloss shown BESIDE it, not instead of it.
/// A word this build does not recognise is shown with no gloss rather than a
/// guess.
const PHASE_GLOSS = {
  running: "keeping services where the document asks for them",
  // An older image's word for `running` (`ADR-032` §8).
  converging: "keeping services where the document asks for them",
  "shutting-down": "stopping services",
  finished: "every service is down",
};

function phaseLine(phase) {
  const gloss = PHASE_GLOSS[phase];
  return gloss ? phase + " — " + gloss : phase;
}

/// **Per-process CPU share, from two samples of `get process`.**
///
/// A single reading carries only `user_millis`/`system_millis` — cumulative
/// counters since the process started — and a per-process percentage needs the
/// difference between two readings divided by the time between them, exactly
/// as the machine-wide Processor card already does with `get cpu`. `elapsedMs`
/// is measured in the browser (`performance.now()`), because nothing on this
/// hardware's clock is trustworthy for a short interval either (`ADR-021`).
///
/// A pid absent from either sample, or whose counters went backwards (the pid
/// was reused between samples), is left out rather than guessed at.
function processCpuShares(before, after, elapsedMs) {
  const shares = new Map();
  if (!(elapsedMs > 0)) return shares;
  const prior = new Map();
  for (const p of (before.processes || [])) {
    if (p.user_millis === undefined || p.system_millis === undefined) continue;
    prior.set(p.pid, p.user_millis + p.system_millis);
  }
  for (const p of (after.processes || [])) {
    if (p.user_millis === undefined || p.system_millis === undefined) continue;
    const was = prior.get(p.pid);
    if (was === undefined) continue;
    const deltaMs = (p.user_millis + p.system_millis) - was;
    if (deltaMs < 0) continue;
    shares.set(p.pid, (deltaMs / elapsedMs) * 100);
  }
  return shares;
}

/// Kernel threads dropped unless asked for, and a name substring match applied
/// otherwise. Pure, so the table body can be rebuilt from this without asking
/// the machine again — sorting and filtering are properties of what already
/// arrived, never a reason to re-read it (this page's own header, "nothing is
/// cached and nothing is summarised", is about the MACHINE'S facts; this is
/// the browser's own list of what to show of them).
function filterProcesses(list, text, includeKernel) {
  const needle = (text || "").trim().toLowerCase();
  return list.filter((p) =>
    (includeKernel || !p.kernel) && (!needle || p.name.toLowerCase().includes(needle)));
}

/// One comparator per sortable column. Each treats a missing value as the
/// lowest, so a process the machine could not time (no `cpuPct`) sorts after
/// the ones it could rather than in an arbitrary place.
const PROCESS_SORTS = {
  name: (a, b) => a.name.localeCompare(b.name),
  state: (a, b) => a.state.localeCompare(b.state) || a.name.localeCompare(b.name),
  threads: (a, b) => (b.threads || 0) - (a.threads || 0),
  resident: (a, b) => (b.resident_bytes || 0) - (a.resident_bytes || 0),
  cpu: (a, b) => (b.cpuPct ?? -1) - (a.cpuPct ?? -1),
};

/// How many rows the table shows. A machine has no upper bound on process
/// count and this page is "brief, concise, at a glance" — the filter is how an
/// operator looking for one process in particular gets past the cap.
const PROCESS_LIMIT = 14;

VIEWS.overview = {
  async draw(body, d) {
    const grid = el("div", "grid");
    body.append(grid);
    const waiting = el("div", "panel");
    waiting.append(el("div", "sub", "asking " + d.name + "…"));
    grid.append(waiting);

    // **Two samples of `cpu` and of `process`, bracketing everything else.**
    // The machine-wide share (Processor card) and every per-process share
    // (Process list) both need a before-and-after; asking them first and last
    // gives the widest gap the rest of this draw affords for free.
    // `bootInfo`, not `boot` — `core.js` already has a global `boot()`, the
    // page's own bootstrap, and this is `get boot`'s answer.
    let status, mem, bootInfo, slot, thermal, store, cpu1, procs1, cpu2, procs2, sampleStart, sampleEnd;
    try {
      cpu1 = await get(d.id, "cpu");
      procs1 = await get(d.id, "process");
      sampleStart = performance.now();
      status = await get(d.id, "status");
      mem = await get(d.id, "memory");
      bootInfo = await get(d.id, "boot");
      slot = await get(d.id, "slot");
      thermal = await get(d.id, "thermal");
      // **R35, `.rulings/` R-163.** BRIEFING-001 dropped both disk cards; the
      // maintainer never asked for `/` and did not mean to lose `/state` too.
      store = await get(d.id, "storage");
      cpu2 = await get(d.id, "cpu");
      procs2 = await get(d.id, "process");
      sampleEnd = performance.now();
    } catch (e) {
      grid.textContent = "";
      const p = el("div", "panel wide");
      p.append(el("h3", null, "This machine did not answer"), el("div", "fail", e.message));
      p.append(el("div", "note", "The register still holds its record; only the machine is silent."));
      grid.append(p);
      return;
    }
    grid.textContent = "";

    // ---- status -------------------------------------------------------------
    const services = status.services || [];
    const running = services.filter((s) => s.state === "started").length;
    const failed = services.filter((s) => s.state === "failed");
    // **The machine says what it is; this page does not work it out.**
    // `status.mode` is `kiosk`, `headless` or `unknown`, and the third one is
    // the reason this is not a boolean: a machine that could not read its own
    // document has not been shown to be headless.
    const kioskMode = status.mode ?? "unknown";
    const isKiosk = kioskMode === "kiosk";
    const s = el("div");
    const mode = el("span", isKiosk ? "pill kiosk" : "pill", kioskMode);
    const modeRow = el("div", "kv");
    modeRow.append(el("span", "k", "Mode")); modeRow.append(mode);
    if (isKiosk && status.locked) modeRow.append(el("span", "pill kiosk", "locked"));
    s.append(kv("Reachable", "up"), modeRow);
    // **Directly under Mode, kiosk only** — BRIEFING-001's own suggestion:
    // "Kiosk Autorun = aura-renderer", following "Mode = Kiosk" rather than
    // sitting apart from it as a fact named "Runs".
    if (isKiosk) {
      s.append(status.auto_run
        ? kv("Kiosk autorun", status.auto_run)
        : kv("Kiosk autorun", status.locked
          ? "nothing — locked with no auto-run"
          : "nothing — the launcher's menu",
          status.locked ? "fail" : undefined));
    }
    s.append(kv("Phase", phaseLine(status.phase || "—")));
    // **`mode = "kiosk"` with no `[kiosk]` table** — `.rulings/` R-164:
    // tolerated and reported, not refused. `isKiosk` is already true here (the
    // machine still answers `kiosk`), so this is what tells the operator the
    // asked-for mode has nothing configuring it.
    if (isKiosk && status.kiosk_unconfigured_because) {
      s.append(kv("Kiosk not configured", status.kiosk_unconfigured_because, "fail"));
    }
    if (kioskMode === "unknown" && status.mode_unknown_because) {
      s.append(kv("Mode unknown", status.mode_unknown_because, "fail"));
    }
    s.append(kv("Services", running + " of " + services.length + " started"));
    if (failed.length) s.append(kv("Failed", failed.map((f) => f.name).join(", "), "fail"));
    grid.append(panel("Status", s));

    // ---- boot: uptime and slot, in place of the disk cards -----------------
    const bootBox = el("div");
    if (slot.running) {
      bootBox.append(kv("Slot", slot.running + (slot.inactive ? " (" + slot.inactive + " is the other)" : "")));
    } else {
      bootBox.append(kv("Slot", "unknown — see Power", "fail"));
    }
    if (slot.in_trial_boot === true) {
      const committed = slot.committed;
      bootBox.append(kv("Trial", committed === true ? "committed" : committed === false ? "not yet committed" : "unmeasured",
        committed === false ? "warn" : undefined));
    }
    bootBox.append(kv("Uptime", duration(bootInfo.uptime_seconds)));
    // **`status.version` is ABSENT, not `""`, on a supervisor older than R33**
    // — the same reading a machine that never answered `status` at all would
    // give, so the two cases say the same thing rather than one of them
    // looking like a real (if empty) answer.
    bootBox.append(status.version
      ? kv("Rivet version", status.version)
      : kv("Rivet version", "not answered by this machine", "sub"));
    grid.append(panel("Boot", bootBox));

    // ---- thermal — BRIEFING-001's "Missing: thermals?" ---------------------
    // **A spot reading, not a soak.** `AGENTS.md` §9.1 and `thermal`'s own
    // `means` field are both explicit that a comfortable number here rules
    // nothing out; the SINCE-BOOT throttle flags are what would, and this
    // card shows them only when the firmware actually answered them —
    // ABSENT FLAGS ARE NOT CLEAR FLAGS.
    const thermalBox = el("div");
    const zone = (thermal.zones || [])[0];
    if (zone) {
      thermalBox.append(el("div", "big", (zone.millidegrees / 1000).toFixed(1) + "°C"));
      thermalBox.append(el("div", "sub", zone.name));
    } else {
      thermalBox.append(el("div", "big", "—"));
      thermalBox.append(el("div", "sub", "no thermal zone found"));
    }
    if (thermal.throttle) {
      const bad = thermal.throttle.under_voltage_since_boot || thermal.throttle.throttled_since_boot;
      thermalBox.append(kv("Since boot", bad ? "under-voltage or throttled" : "clean",
        bad ? "fail" : undefined));
    } else {
      thermalBox.append(kv("Since boot", "not measured", "sub"));
    }
    grid.append(panel("Thermal", thermalBox));

    // ---- cpu ------------------------------------------------------------
    const dBusy = (cpu2.total.busy_millis - cpu1.total.busy_millis);
    const dTotal = (cpu2.total.total_millis - cpu1.total.total_millis);
    const cpuBox = el("div");
    if (dTotal > 0) {
      const share = (dBusy / dTotal) * 100;
      cpuBox.append(el("div", "big", pct(share)));
      const bar = el("div", "bar"); const fill = el("i");
      fill.style.width = Math.max(0, Math.min(100, share)) + "%";
      bar.append(fill); cpuBox.append(bar);
      cpuBox.append(el("div", "sub", cpu2.processors + " processors"));
    } else {
      cpuBox.append(el("div", "big", "—"));
      cpuBox.append(el("div", "sub", "the two samples were the same tick"));
    }
    grid.append(panel("Processor", cpuBox));

    // ---- memory -----------------------------------------------------------
    const used = mem.total_bytes - mem.available_bytes;
    const memBox = meter(used, mem.total_bytes);
    memBox.append(el("div", "sub", bytes(mem.cached_bytes) + " cached"));
    grid.append(panel("Memory", memBox));

    // ---- /state — R35, `.rulings/` R-163 -----------------------------------
    // **One card, and it is `/state`'s, never `/`'s.** `BRIEFING-001` dropped
    // both of the old page's disk cards; the maintainer's own review only ever
    // objected to the pair of them repeating what `df` already says, and R35
    // asked for exactly one back — the volume applications and updates land
    // on, which is the one an operator actually runs out of room on.
    //
    // **`holds`, not `mount-point`.** `get storage`'s own field says which of
    // Rivet's locations a filesystem is carrying, and `/state` is not
    // guaranteed its own mount point — on a machine where it shares one with
    // `/`, matching `mount-point == "/state"` would find nothing and this
    // card would silently vanish. `holds` is the fact that survives either
    // layout.
    const stateFs = (store.filesystems || []).find((fs) => (fs.holds || []).includes("/state"));
    const stateBox = el("div");
    if (stateFs) {
      const gauge = meter(stateFs["used-bytes"], stateFs["total-bytes"]);
      stateBox.append(gauge);
      stateBox.append(el("div", "sub",
        stateFs.fstype + (stateFs["read-only"] ? " · read-only" : "")));
    } else {
      stateBox.append(el("div", "sub", "no filesystem on this machine reports holding /state"));
    }
    grid.append(panel("/state", stateBox));

    // ---- processes ----------------------------------------------------------
    const cpuShares = processCpuShares(procs1, procs2, sampleEnd - sampleStart);
    const everyProcess = (procs2.processes || []).map((p) => ({ ...p, cpuPct: cpuShares.get(p.pid) }));

    let sortKey = "resident";
    let filterText = "";
    let includeKernel = false;

    const controls = el("div", "toolbar");
    const filterInput = el("input");
    filterInput.type = "search";
    filterInput.placeholder = "filter by name…";
    const kernelLabel = el("label", null);
    const kernelCheck = el("input");
    kernelCheck.type = "checkbox";
    kernelLabel.append(kernelCheck, document.createTextNode(" include kernel threads"));
    controls.append(el("label", null, "Processes"), filterInput, kernelLabel);

    const tableHost = el("div");
    const countLine = el("div", "note");
    const procPanelBody = el("div");
    procPanelBody.append(controls, tableHost, countLine);
    grid.append(panel("Processes", procPanelBody, true));

    const COLUMNS = [
      { key: "name", label: "Process" },
      { key: "state", label: "State" },
      { key: "threads", label: "Threads" },
      { key: "cpu", label: "CPU", num: true },
      { key: "resident", label: "Resident", num: true },
    ];

    function renderProcessTable() {
      const filtered = filterProcesses(everyProcess, filterText, includeKernel);
      const sorted = filtered.slice().sort(PROCESS_SORTS[sortKey]);
      const shown = sorted.slice(0, PROCESS_LIMIT);

      const table = el("table");
      const head = el("tr");
      head.append(el("th", null, "PID"));
      for (const col of COLUMNS) {
        const th = el("th", col.num ? "num" : null,
          col.label + (sortKey === col.key ? " ▾" : ""));
        th.style.cursor = "pointer";
        th.onclick = () => { sortKey = col.key; renderProcessTable(); };
        head.append(th);
      }
      table.append(head);
      for (const p of shown) {
        const tr = el("tr");
        tr.append(el("td", "mono", String(p.pid)), el("td", null, p.name),
          el("td", null, p.state), el("td", "num", String(p.threads)));
        tr.append(el("td", "num", p.cpuPct === undefined ? "—" : pct(p.cpuPct)));
        tr.append(el("td", "num", bytes(p.resident_bytes)));
        table.append(tr);
      }
      tableHost.textContent = "";
      tableHost.append(table);
      countLine.textContent = "Showing " + shown.length + " of " + filtered.length +
        " (" + everyProcess.length + " total" +
        (includeKernel ? "" : ", kernel threads excluded") + "). CPU is two samples, " +
        Math.round(sampleEnd - sampleStart) + " ms apart.";
    }

    filterInput.oninput = () => { filterText = filterInput.value; renderProcessTable(); };
    kernelCheck.onchange = () => { includeKernel = kernelCheck.checked; renderProcessTable(); };
    renderProcessTable();
  },
};
