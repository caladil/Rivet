// **The confirmation gate** - `ADR-012` §7.1, in a browser.
//
// > An operation whose `confirms` is true CHANGES THIS MACHINE and does nothing
// > without the literal last word `confirm` - without it, it answers with its
// > plan.
//
// That is the machine's own sentence, out of `get catalogue`. This module is
// the whole of what the portal adds to it: it sends the command WITHOUT the
// confirmation, renders the plan the machine answered with, and sends it again
// with the word only after somebody has pressed a button beside that plan.
//
// # Nothing here composes a consequence
//
// Every line rendered below is a string the machine wrote - the consequence,
// each effect, what is NOT touched, when it takes effect, and the exact command
// that does it. A web page that summarised a reboot's consequences would be a
// second account of them, and the operator would be confirming against the
// wrong one. So the renderer walks the plan's fields and prints them; it has no
// vocabulary of its own for what an operation does.
//
// # A bare button is worse than the CLI, and that is the point
//
// `cargo rivet run -- apply restart` prints a plan and refuses. A portal that
// fired a reboot from one click would be a WORSE operator surface than the
// command line it is meant to improve on, which is the argument for building
// this before building any button that uses it.

/// The refusal code that carries a plan. `rivet_core::operation::ErrorCode`.
const CONFIRMATION_REQUIRED = "E_CONFIRMATION_REQUIRED";

/// The literal last word. `rivet_core::operation::apply::CONFIRMATION`.
const CONFIRM_WORD = "confirm";

/// Render one plan into `into`, with the buttons that act on it.
///
/// `words` is the command WITHOUT the confirmation - `["restart"]`,
/// `["service-restart", "aura-vitals"]`. `after` is handed the machine's answer
/// when the operator confirms.
function planGate(into, id, words, after) {
  into.textContent = "";
  into.append(el("div", "sub", "asking the machine what `apply " + words.join(" ") + "` would do…"));

  run(id, "apply", ...words).then(({ refused, value }) => {
    into.textContent = "";
    if (!refused) {
      // **A confirming operation that did something unasked.** Not reachable
      // through any operation in this build, and rendered rather than ignored
      // because if it ever happens the operator has to be told that the machine
      // acted without being confirmed.
      const box = el("div", "said bad");
      box.append(el("div", "fail",
        "This operation ANSWERED rather than planning, which means it acted without the " +
        "confirmation word. That is a defect in the machine's build, not in this page."));
      box.append(el("pre", "mono", JSON.stringify(value, null, 2)));
      into.append(box);
      return;
    }
    if (value.error !== CONFIRMATION_REQUIRED || !value.plan) {
      // **The machine said no for some other reason**, and its reason is the
      // useful thing. A page that turned this into "failed" would throw away
      // the only sentence that says what to do instead.
      into.append(refusal(value));
      return;
    }
    into.append(planBox(id, words, value.plan, after));
  }).catch((e) => {
    into.textContent = "";
    into.append(said("bad", e.message));
  });
}

/// The machine's refusal, in the machine's words.
function refusal(value) {
  const box = el("div", "said bad");
  box.append(el("div", "fail", value.error || "refused"));
  box.append(el("div", null, value.message || JSON.stringify(value)));
  return box;
}

/// A plain block carrying one sentence.
function said(cls, text) {
  const box = el("div", "said " + cls);
  box.append(el("div", null, text));
  return box;
}

function planBox(id, words, plan, after) {
  const box = el("div", "plan");
  box.append(el("h4", null, "Nothing has changed yet"));
  box.append(el("div", "consequence", "This " + (plan.consequence || "changes this machine") + "."));

  const what = el("div");
  what.append(el("div", "sub", "What would change"));
  const list = el("ul");
  for (const e of (plan.effects || [])) {
    // `act`, `target`, `from`, `to`, `note` - the machine's own fields. Joined
    // rather than paraphrased: `rivet_core::operation::plan::Effect::sentence`
    // composes the prose form for the CLI and this is the same five values.
    let line = e.act + " " + e.target;
    if (e.from !== undefined && e.to !== undefined) line += " — " + e.from + " → " + e.to;
    else if (e.to !== undefined) line += " → " + e.to;
    else if (e.from !== undefined) line += " — it is " + e.from;
    const li = el("li", null, line);
    if (e.note) li.append(el("div", "sub", e.note));
    list.append(li);
  }
  if (!(plan.effects || []).length) {
    list.append(el("li", "fail", "nothing is listed, which is a defect in this operation"));
  }
  what.append(list);
  box.append(what);

  if ((plan.untouched || []).length) {
    const not = el("div");
    not.append(el("div", "sub", "What is NOT touched"));
    const ul = el("ul");
    for (const u of plan.untouched) ul.append(el("li", null, u));
    not.append(ul);
    box.append(not);
  }

  const when = plan["takes-effect"] || {};
  const whenLine = el("div", "note");
  whenLine.textContent = "Takes effect: " + (when.when || "not stated") +
    (when["requires-reboot"] === true ? " — a restart is required" : "") +
    (when["requires-reboot"] === false ? " — no restart is needed" : "") +
    (when.because ? " (" + when.because + ")" : "");
  // `requires-reboot` ABSENT is a third answer and must not read as "no".
  if (when["requires-reboot"] === undefined) whenLine.className = "note warn";
  box.append(whenLine);

  if (plan["confirm-with"]) {
    box.append(el("code", "cmd", plan["confirm-with"]));
  }

  const acts = el("div", "acts");
  const yes = el("button", "go", "Confirm");
  yes.type = "button";
  const no = el("button", "cancel", "Cancel");
  no.type = "button";
  acts.append(yes, no);
  box.append(acts);

  no.onclick = () => { box.replaceWith(said("", "Cancelled. Nothing was sent.")); };
  yes.onclick = () => {
    yes.disabled = true; no.disabled = true;
    yes.textContent = "sending…";
    run(id, "apply", ...words, CONFIRM_WORD).then(({ refused, value }) => {
      const out = refused ? refusal(value) : done(value);
      box.replaceWith(out);
      if (!refused && after) after(value);
    }).catch((e) => { box.replaceWith(said("bad", e.message)); });
  };
  return box;
}

/// What the machine answered once it had done the thing.
///
/// Rendered as its own fields rather than as a sentence this page composed:
/// `apply restart` answers `{"subject":"restart","doing":…}` and an operator
/// reading "restarting" instead of the machine's own word would be reading this
/// page's belief rather than the machine's report (`AGENTS.md` §5).
function done(value) {
  const box = el("div", "said good");
  for (const [k, v] of Object.entries(value)) {
    if (typeof v === "object") continue;
    box.append(kv(k, String(v)));
  }
  return box;
}
