// **Power** - restart, trial-restart, power-off, and the A/B slot.
//
// Four operations that change a machine and one that reads it. Every button
// here goes through `planGate`, so none of them does anything until the machine
// has said what it would do and somebody has pressed Confirm beside that
// sentence.
//
// # The word the maintainer could not remember is `commit`, and NO OPERATION
// # PERFORMS IT
//
// This was checked rather than assumed, two ways, because `AGENTS.md` §7 is
// about exactly this kind of sentence:
//
// * `get catalogue` on a running board lists `apply`'s subjects, and they are
//   `trial-boot`, `power-off`, `restart`, `trial-restart`, `install`, `edit`,
//   `uninstall`, `repo-delete`, `reload`, `service-restart`, `input`,
//   `create-file`, `create-directory`, `set-mode`, `set-owner`, `rename`,
//   `remove-file`, `remove-directory` and `install-secret`. None of them
//   commits a trial boot.
// * `rivet-core/src/operation/` holds one file per subject and there is no
//   commit among them.
//
// **PID 1 does it, and it does it on its own.** `cmd/rivet-initd`'s `confirm`
// module runs four gates - the mount phase, the document, the supervisor, and
// whether ANYTHING HAS REACHED THE CONTROL PLANE on this boot - and then makes
// the running slot the default. A trial nobody connects to is never committed,
// and `ADR-018` §5.1 rolls it back on the next reset with no userspace
// involvement at all. That is the safety property, not a fault.
//
// So the honest thing for a portal to offer is not a Commit button that would
// have nothing to call. It is:
//
// 1. `get slot`, so the operator can see that this boot is a trial at all, and
// 2. the machine's OWN RECORD of what PID 1 decided, out of its journal.
//
// # And that record is at the START of the boot, which is a trap with a name
//
// `rivet get log` keeps the NEWEST records when a limit cuts, and PID 1 writes
// its confirmation in the first minute of a boot. So a search for it against
// the 200-record default comes back empty on a machine that has been up ten
// minutes, and reads as *this boot was never committed* - which is the exact
// shape of the failure `PLAN.md` §104.10 records, pointed at a more expensive
// conclusion. The button below asks for `all` and says why.

/// What PID 1 writes when it has decided about a trial boot.
/// `cmd/rivet-initd`'s `Confirmation::summary` composes every one of these.
const CONFIRMATION_MARKS = ["confirm", "commit", "tryboot", "trial boot", "TRIAL BOOT"];

VIEWS.power = {
  async draw(body, d) {
    const grid = el("div", "grid");
    body.append(grid);

    let slot;
    try {
      slot = await get(d.id, "slot");
    } catch (e) {
      grid.append(panel("This machine did not answer", el("div", "fail", e.message), true));
      return;
    }

    // ---- where this machine is booted from --------------------------------
    // **Narrow, not wide** — BRIEFING-001: reading this card on a wide screen
    // was hard, and the fix is the same one the Software drawer needed. The
    // long prose paragraph the machine calls `means` is dropped from here for
    // the same reason: every fact it explains is already a row above it, and
    // `AGENTS.md` §5 is served by the rows carrying the machine's own words,
    // not by also carrying its own essay about them.
    const s = el("div");
    s.append(kv("Running slot", slot.running || "— the two authorities disagree",
      slot.running ? undefined : "fail"));
    s.append(kv("Inactive slot", slot.inactive || "—"));
    if (slot.agree === false) {
      s.append(kv("Agree", "NO — nothing should be armed", "fail"));
    }
    if (slot.in_trial_boot === true) {
      s.append(kv("This boot", "trial", "warn"));
      if (slot.committed !== undefined) {
        s.append(kv("Committed", String(slot.committed), slot.committed ? undefined : "warn"));
      }
    } else if (slot.in_trial_boot === false) {
      s.append(kv("This boot", "ordinary"));
    }
    const slotPanel = panel("A/B slot", s);
    slotPanel.classList.add("narrow");
    grid.append(slotPanel);

    // ---- the trial boot, and who commits it -------------------------------
    if (slot.in_trial_boot === true) {
      grid.append(panel("Is this trial committed?", trialBox(d), true));
    }

    // ---- the four operations ------------------------------------------------
    // **Title, button, text — and the button is the icon alone**, ruled by the
    // maintainer 2026-09-13: the card's title already names the operation, so
    // a label on the button beside it said the same word twice. The name moves
    // to the button's tooltip and its accessible name, which is where a reader
    // who cannot see the icon finds it.
    for (const op of powerOperations(slot)) {
      const box = el("div");
      const where = el("div");
      const actions = el("div", "power-actions");
      const press = el("button", op.danger ? "op icon danger" : "op icon", op.icon);
      press.type = "button";
      press.title = op.label;
      press.setAttribute("aria-label", op.label);
      press.onclick = () => { press.disabled = true; planGate(where, d.id, op.words); };
      actions.append(press);
      box.append(actions, el("div", "power-what", op.what));
      if (op.refuse) {
        press.disabled = true;
        box.append(el("div", "note warn", op.refuse));
      }
      box.append(where);
      grid.append(panel(op.label, box));
    }
  },
};

/// The four, with the sentence that says which is which.
///
/// **`restart` restarts the MACHINE and `service-restart` restarts ONE
/// SERVICE** - the catalogue's own warning, and the reason the service restart
/// lives on the Software page rather than here.
function powerOperations(slot) {
  const inactive = slot.inactive;
  return [
    {
      key: "restart", label: "Restart", icon: "⟲", words: ["restart"], danger: true,
      what: "Restarts the whole machine. Rolls back an uncommitted trial boot.",
    },
    {
      key: "trial-restart", label: "Trial restart", icon: "⇄", words: ["trial-restart"], danger: true,
      what: "Restarts once into whatever slot is armed below. Rolls back on the next reset " +
        "unless something commits it.",
    },
    {
      key: "power-off", label: "Power off", icon: "⏻", words: ["power-off"], danger: true,
      what: "Stops the machine. There is no remote way to turn it back on.",
    },
    {
      key: "trial-boot", label: "Arm a trial boot", icon: "⚙",
      words: inactive ? ["trial-boot", inactive] : ["trial-boot"],
      danger: false,
      what: inactive
        ? "Arms slot " + inactive + " for one boot. Does not restart anything."
        : "Arms the inactive slot for one boot.",
      refuse: inactive ? null
        : "No inactive slot to arm — the two authorities above disagree about which is running.",
    },
  ];
}

/// **The machine's own record of what PID 1 decided about this trial.**
function trialBox(d) {
  const box = el("div");
  box.append(el("div", null,
    "Nothing an operator can send commits a trial boot — PID 1 does that on its own, once " +
    "something has reached the control plane on this boot. A trial nobody connects to rolls " +
    "back at the next reset."));

  const where = el("div");
  const press = el("button", "op", "Read the confirmation out of the journal");
  press.type = "button";
  press.style.marginTop = "12px";
  press.onclick = () => {
    press.disabled = true;
    loading(where, "reading this boot's whole journal — the confirmation is written in its " +
      "first minute, and a limited read keeps the NEWEST records, so a limit would miss it…");
    // `latest` and `all`: see this file's header for what a default limit costs
    // here specifically.
    get(d.id, "log", "latest", "all").then((answer) => {
      where.textContent = "";
      const lines = (answer.records || [])
        .map((line) => { try { return JSON.parse(line); } catch (e) { return null; } })
        .filter((r) => r && typeof r.msg === "string" && r.msg.includes("rivet-initd"))
        .filter((r) => CONFIRMATION_MARKS.some((m) => r.msg.toLowerCase().includes(m.toLowerCase())));

      const committed = lines.find((r) => r.msg.includes("boot confirmed:"));
      if (committed) {
        where.append(said("good", committed.msg));
        where.append(el("div", "note",
          "That is PID 1's own line. This slot is the default now, and an ordinary restart " +
          "stays on it."));
      } else if (lines.length) {
        where.append(said("bad",
          "No `boot confirmed:` line is in this boot's journal. What PID 1 did say about it:"));
      } else {
        where.append(said("bad",
          "Nothing PID 1 wrote about a boot confirmation is in this boot's journal at all. " +
          "That is worth understanding before this machine is restarted: an uncommitted trial " +
          "rolls back."));
      }
      if (lines.length) {
        const table = el("table", "jrows");
        for (const r of lines) {
          const tr = el("tr");
          tr.append(el("td", "when", (r.mono ?? "").toString()), el("td", "msg", r.msg));
          table.append(tr);
        }
        where.append(table);
      }
      where.append(el("div", "note",
        "Read from " + (answer.returned ?? 0) + " records of the " + (answer.held ?? 0) +
        " this machine holds, with no limit applied."));
    }).catch((e) => {
      where.textContent = "";
      where.append(said("bad", e.message));
    });
  };
  box.append(press, where);
  return box;
}
