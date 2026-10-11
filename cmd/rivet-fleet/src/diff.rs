//! **What writing the register's document would take away** — `ADR-034` §6.2.
//!
//! > `apply document` reads the machine's current document, diffs it against
//! > the composed one, and REFUSES if the write would remove or change
//! > anything the register does not account for — naming each loss and the
//! > command that would record it.
//!
//! # Why the danger is real and was measured
//!
//! [`crate::compose`] is a pure function of the register, so **anything on a
//! machine the register does not hold is absent from the composed document** —
//! and `apply document` writes that document whole. Against the live board on
//! 2026-08-29 the composed document was missing `[control-plane]` *and*
//! `[services.rivet-mcpd]`: one apply would have dropped the listener to
//! loopback and turned the control plane off, on a headless machine, at once.
//!
//! # The comparison is over the DOCUMENT, and only over the document
//!
//! Two TOML documents: the one the machine holds and the one the register
//! composes. **This is not drift detection over machine state.**
//! `crate::member::listen_drift` is §11.5.2.1's narrower check and stays
//! separate — it compares one register field against one document field and
//! reports; this compares every leaf of two documents and refuses.
//!
//! # An absence is never intent, and that is the whole asymmetry
//!
//! A leaf the register does not hold is missing from the composed document
//! because **nobody recorded it**, never because somebody asked for it to go.
//! The register has no way to spell *remove this*: undeclaring and never having
//! declared render identically. So a removal is unambiguous and is refused.
//!
//! A leaf whose value differs is the opposite — a register saying `enabled =
//! false` where the machine says `true` is the ordinary reconfiguration this
//! tool exists to perform. §6.2 left that open; see §6.2.1 for the decision and
//! its argument. Changes are **reported and applied**, so an operator sees what
//! the write moves without a refusal firing on every ordinary run.
//!
//! # Half of this module moved out on 2026-09-05, and the halves are different
//! kinds of thing
//!
//! `ADR-044` gave the comparison a **second caller**: a fleet server converging
//! a machine onto a document a git repository holds needs to say which keys
//! drifted and to what, which is the same question this module has been
//! answering for the register. Two implementations of it would be the second
//! authority `ADR-011` is about.
//!
//! So the vocabulary and the comparison went where both can reach them —
//! `rivet_core::leaf` for the types and the diff, `rivet_config::leaves` for
//! the TOML reader — and they are re-exported here unchanged, so nothing that
//! spells `diff::Loss` had to move. **What stayed is what is the register's
//! alone**: [`remedy`], [`cost`] and [`refusal`] are all about
//! `rivet-fleet`'s own commands and its own record, and a fleet server has no
//! use for any of them.

use crate::register::Device;

// The comparison, from where both callers can reach it. Re-exported rather
// than re-spelled: `crate::main` and `crate::diff_tests` say `diff::Loss` in a
// dozen places and the type they mean did not change.
pub use rivet_config::leaves::leaves;
pub use rivet_core::leaf::{Diff, Loss, between};

/// How to put one lost value into the register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Remedy {
    /// A command that records it. `ADR-034` §6.2: *"the command that would
    /// record it"*.
    Command(String),
    /// **No command records it**, and this says what does instead.
    ///
    /// Said rather than omitted, and rather than named as a command that would
    /// do something else. `AGENTS.md` §5 — a plausible answer that is wrong
    /// does not fail, it succeeds and is acted on.
    ByHand(String),
}

impl Remedy {
    /// The text of it, whichever kind it is.
    #[must_use]
    pub fn said(&self) -> &str {
        match self {
            Self::Command(text) | Self::ByHand(text) => text,
        }
    }

    /// The word the refusal puts in front of it.
    ///
    /// **`NO VERB` is shouted and `record it` is not**, because the two are
    /// different amounts of work for the operator and the loud one is the one
    /// that needs a decision rather than a paste.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Command(_) => "record it",
            Self::ByHand(_) => "NO VERB",
        }
    }
}

/// **The command that records one loss, or the reason there is none.**
///
/// `ADR-034` §6.2 requires the remedy beside the loss, and gives the reason:
///
/// > A refusal that named the loss without the remedy would leave an operator
/// > to work out that the register is the thing to change, and the natural
/// > wrong move from there is to edit the machine instead, which widens the
/// > drift the refusal exists to catch.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn remedy(device: &Device, loss: &Loss) -> Remedy {
    let name = &device.name;
    let value = &loss.value;
    let segments: Vec<&str> = loss.path.iter().map(String::as_str).collect();
    match segments.as_slice() {
        ["control-plane", "listen"] => Remedy::Command(format!(
            "rivet-fleet set listen --device {name} --listen {value} --confirm"
        )),
        // `SPEC-05` §5's own gate, so a loss here is not only a REMOVAL but a
        // document that will refuse to compose as FINAL until it is answered
        // - `set timezone` is the verb for a record with no machine to adopt
        // one from.
        ["timezone"] => Remedy::Command(format!(
            "rivet-fleet set timezone --device {name} --timezone {value}"
        )),
        // `SPEC-05` §5's other kiosk fact, `set pin`'s reason for existing at
        // all: `add document` fills this from a machine that already answers
        // it, and a brand-new record has no machine to adopt from yet.
        ["kiosk", "auto-run"] => Remedy::Command(format!(
            "rivet-fleet set kiosk-auto-run --device {name} --application {value}"
        )),
        ["services", service, "enabled"] => Remedy::Command(format!(
            "rivet-fleet set service --device {name} --service {service} --enabled {value}"
        )),
        // `--enabled` is required by the operation table, so a memory cap
        // cannot be recorded on its own. The placeholder is written out rather
        // than guessed at: this tool does not know what the operator wants the
        // service to be, and `enabled` is the more consequential half.
        ["services", service, "memory"] => Remedy::Command(format!(
            "rivet-fleet set service --device {name} --service {service} --enabled \
             <true|false> --memory {value}"
        )),
        // **`add repo` MINTS.** It is `add identity` versus `set pin` again
        // (`ADR-034` §6.1.1): the verb that exists makes a new identifier, and
        // the machine already serves a directory under this one. Naming it
        // here would be naming a command that produces a different outcome
        // from the one the operator is being told to reach.
        ["repos", repo, "id"] => Remedy::ByHand(format!(
            "`add repo --device {name} --name {repo}` MINTS A NEW IDENTIFIER and would not \
             record this one - the machine already serves a directory under it. Put `{repo} \
             = \"{value}\"` under `[repos]` in the device record, or let `add document` do \
             it."
        )),
        // **`[network.resolver]` is not an interface**, and must be matched
        // before the generic arm below - `ADR-034` §6 is the whole reason this
        // arm exists: a resolver's `nameservers` and `search` used to fall
        // into the generic arm's guidance, which told an operator to put them
        // under `[interfaces.resolver]` - a table with no field for either.
        ["network", "resolver", field @ ("nameservers" | "search")] => Remedy::Command(format!(
            "rivet-fleet add document --device {name} --confirm   - records it into the \
             device's own [resolver] table (ADR-034 6); there is no standalone `set` verb for \
             `{field}` yet"
        )),
        ["network", interface, field] => Remedy::ByHand(format!(
            "no verb writes an interface. Put `{field} = \"{value}\"` under \
             `[interfaces.{interface}]` in the device record, or let `add document` do it."
        )),
        // The composed hostname is the device's LABEL and there is no second
        // field for it — `crate::compose::hostname` has the argument. So a lost
        // hostname means the label is not a valid one, and the fix is the label.
        ["hostname"] => Remedy::ByHand(format!(
            "the composed hostname IS the device's label, and `{name}` is not one a machine \
             can carry - so nothing is composed and the machine's `{value}` would go. \
             Rename the record: `name` in the device record, to something \
             rivet_core::hostname accepts."
        )),
        ["volumes", volume, ..] => Remedy::ByHand(format!(
            "THE REGISTER HAS NO FIELD FOR VOLUMES. ADR-034 \u{a7}6's table does not list \
             them and nothing here can record `{volume}`. Growing the record is a ruling, \
             not a fix; until then this machine's volume declaration cannot survive an \
             `apply document` from the register."
        )),
        _ => Remedy::ByHand(format!(
            "the register has no field for `{}`, so nothing here records it. `add document \
             --device {name}` names everything it cannot record, which is the list this \
             belongs to.",
            loss.where_it_is()
        )),
    }
}

/// What losing this particular value costs, where the cost is not obvious.
///
/// **Only two paths carry one, and they are the two that were measured on the
/// board** (`ADR-034` §6.2). Nothing here invents a consequence for a value
/// whose consequence has not been established — a refusal that editorialised
/// about every leaf would be a refusal nobody finishes reading.
#[must_use]
fn cost(loss: &Loss) -> Option<String> {
    let segments: Vec<&str> = loss.path.iter().map(String::as_str).collect();
    match segments.as_slice() {
        ["control-plane", "listen"] => Some(format!(
            "losing it drops the listener to {} - LOOPBACK - and nothing on your network, \
             this tool included, could reach the machine again.",
            rivet_config::control_plane::DEFAULT_LISTEN
        )),
        ["services", "rivet-mcpd", "enabled"] if loss.value == "true" => Some(
            "losing it reverts `enabled` to the image's default. On a card built before this \
             service was turned on by default that is `false`, and THE CONTROL PLANE DOES \
             NOT START - which is the only way this tool reaches the machine."
                .to_owned(),
        ),
        _ => None,
    }
}

/// **The refusal**, or `None` when nothing would be lost.
///
/// Every loss individually, what the machine has, and the remedy — `ADR-034`
/// §6.2 requires all three, and requires them per loss rather than as a count:
///
/// > an operator who has to discover them one at a time will stop before the
/// > last.
#[must_use]
pub fn refusal(device: &Device, diff: &Diff) -> Option<String> {
    if diff.losses.is_empty() {
        return None;
    }
    let mut out = format!(
        "REFUSED - writing the register's document over this machine's would REMOVE {} \
         value(s) the register does not hold. NOTHING HAS BEEN WRITTEN and the machine is \
         exactly as it was.\n\
         \n\
         Each one below is on the machine and not in the register, so `compose` does not \
         carry it and writing that document takes it away. An absent value in the register \
         is never a request to remove - undeclaring and never having declared look the same \
         from here - which is why this refuses rather than warns (ADR-034 6.2).\n",
        diff.losses.len()
    );
    for loss in &diff.losses {
        out.push_str(&format!("\n  {}\n", loss.where_it_is()));
        out.push_str(&format!(
            "    machine   {}{}\n",
            loss.value,
            if loss.whole {
                ""
            } else {
                "   (one entry of a list; the others are in the register)"
            }
        ));
        let found = remedy(device, loss);
        // Nine wide, so the three labels line up under each other and a reader
        // scanning the column sees which losses have a command and which do
        // not without reading the sentences.
        out.push_str(&format!("    {:<9} {}\n", found.label(), found.said()));
        if let Some(said) = cost(loss) {
            out.push_str(&format!("    cost      {said}\n"));
        }
    }
    out.push_str(&format!(
        "\nOr record what CAN be recorded in one go:\n\
         \x20 rivet-fleet add document --device {} --confirm\n\
         which reads this machine, writes into the register only what the register lacks, \
         and names anything it cannot record. It changes nothing on the machine.\n\
         \n\
         The other way out is to decide the value should go - and there is no way to say \
         that. Removing it from the machine is `rivet apply edit` at the machine's own \
         prompt; this tool composes from the register and the register cannot express an \
         absence on purpose.\n",
        device.name
    ));
    Some(out)
}

/// What the write would MOVE, listed and then done.
///
/// See §6.2.1: a change is the operator's intent and does not refuse, so this
/// is a report rather than a gate. It is written into the same report the
/// transfer goes into, before the transfer, so a reader sees it whether or not
/// the machine then answers.
#[must_use]
pub fn changes(diff: &Diff) -> Option<String> {
    if diff.changes.is_empty() {
        return None;
    }
    let mut out = format!(
        "changing {} value(s) on this machine - the register's answer wins, which is what \
         this operation is for:\n",
        diff.changes.len()
    );
    for change in &diff.changes {
        out.push_str(&format!(
            "\x20 {}   {} -> {}\n",
            change.path.join("."),
            change.machine,
            change.register
        ));
    }
    Some(out)
}

#[cfg(test)]
#[path = "diff_tests.rs"]
mod tests;
