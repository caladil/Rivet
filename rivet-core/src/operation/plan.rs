//! **`ADR-006` §6.5's framework: a mutating operation is a type that computes
//! its diff separately from applying it.**
//!
//! > Section 6.2 requires that every mutating operation support a dry run. That
//! > requirement is only credible if it is structurally impossible to omit:
//! > `rivet-core` defines a mutating operation as a type that **computes its
//! > diff separately from applying it**, so a command that cannot be dry-run
//! > does not compile rather than merely being undocumented.
//!
//! Until 2026-08-26 that framework did not exist and `ADR-006` §8.9's closing
//! list said so. Seven `apply` subjects each supplied the property **by hand**,
//! and each put its plan in the human `message` of an
//! [`ErrorCode::ConfirmationRequired`] refusal. `PLAN.md` §22.3 priced that:
//! *"an agent can branch on the code and re-issue the command but cannot read
//! the diff structurally"*, and `ADR-012` §3 makes plan-then-apply the **safety**
//! mechanism rather than the friendly one — so an agent asserting *this change
//! is the one I intended* against a prose sentence is doing string matching on
//! the safety mechanism.
//!
//! # How the property is made structural rather than promised
//!
//! [`Change`] has an associated type, and that is the whole trick:
//!
//! ```text
//! type Prepared;
//! type Applied;
//! fn plan(&self, board) -> Result<(Plan, Self::Prepared), OperationError>;
//! fn apply(self, prepared: Self::Prepared, board) -> Result<Self::Applied, OperationError>;
//! ```
//!
//! **[`Change::apply`] cannot be called without a `Self::Prepared`, and the only
//! thing that produces one is [`Change::plan`].** A mutating operation that
//! skipped its dry run would have to conjure a value of a type it does not
//! construct. That is §6.5's *"does not compile"* rather than a convention a
//! reviewer has to notice — the same discipline `ADR-006` §3's dependency-graph
//! gate uses, where the build refuses instead of a person refusing.
//!
//! [`resolve`] is the one driver, and it plans on **both** paths. Every subject
//! had already discovered separately that it must — `super::stop` states the
//! argument and the other four cite it — *"a plan that had not checked would
//! describe a shutdown, take the confirmation, and only then discover that this
//! is not a machine it may stop"*. That is now a property of the driver instead
//! of a paragraph repeated five times.
//!
//! # The plan is the structured payload `ADR-006` §4.1 already asked for
//!
//! > Every failure carries a **stable machine-readable code**, a **structured
//! > payload**, and a **human string** — in that order of authority.
//!
//! [`OperationError`] carried the code and the string, and its only structured
//! field was the subject. A [`Plan`] rides in the second slot now, so this is
//! §4.1 being *filled in* rather than widened.
//!
//! **And the human string is generated from the plan.** [`Plan::narrative`] is
//! the only writer of a confirmation message in this workspace. That is
//! `ADR-011` in its usual form — the alternative is a typed diff beside a
//! hand-written sentence, two accounts of one change, and the sentence is the
//! one an operator reads.
//!
//! # `requires-reboot`, and why it is not a `bool`
//!
//! `ADR-034` §5.3 rules it *"not optional"*, and names the hazard from
//! `PLAN.md` §0.0.4: **a configuration that is wrong in a way indistinguishable
//! from success until the next boot.** An operator who disables something, sees
//! a success, and finds it still running has been told something false by
//! omission.
//!
//! A `bool` cannot carry that honestly, because of `apply edit`. It writes an
//! **arbitrary path** — its safety story is replace-exactly-once and a reserved
//! deny-list (`ADR-006` §8.9), not a schema — so Rivet does not know what the
//! file it just changed is read by, or when. `false` would claim the change is
//! live and `true` would claim a restart is needed; both are assertions this
//! build cannot make, and §5.3 exists precisely to stop a plan asserting
//! something it does not know. [`TakesEffect::NotAsserted`] is the third state,
//! and it **carries why** so that the answer is a fact about the operation
//! rather than a shrug.
//!
//! Two states would have been enough for the six subjects that are not `edit`.
//! Making the seventh lie to keep the type small is the trade this module
//! refuses.

use super::machine::Board;
use super::{ErrorCode, OperationError, Verb};
use crate::value::Value;

// ---------------------------------------------------------------------------
// What a change does
// ---------------------------------------------------------------------------

/// **What one [`Effect`] does to the thing it names.** Four words, closed.
///
/// `ADR-006` §6.4's closed-vocabulary rule applied one level down: this is
/// exactly the kind of enumeration that grows one convenient value at a time
/// until an agent branching on it learns nothing. The bar for a fifth is that it
/// names a *distinct thing that happens to a target* — not a place in the code,
/// and not a shade of one of these.
///
/// [`Self::Request`] is the one that carries a real distinction rather than a
/// grammatical one: five of the seven `apply` subjects **do not do the thing**.
/// They write a request another process reads, and that process may refuse, be
/// slow, or not be running. A plan that rendered `power-off` as *"this machine
/// powers off"* would be describing an outcome this operation cannot promise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// Something that exists takes a different value.
    Set,
    /// Something that is not there will be — a file, a directory, an entry.
    Create,
    /// Something that is there will not be.
    Remove,
    /// **Another process is asked.** This operation records the ask and returns;
    /// what happens next is that process's, and the plan says so rather than
    /// claiming the outcome.
    Request,
}

impl Act {
    /// The wire form. Part of the interface contract, like every other string an
    /// agent branches on (`ADR-006` §4.1) — never renamed.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Set => "set",
            Self::Create => "create",
            Self::Remove => "remove",
            Self::Request => "request",
        }
    }

    /// Every act, so a catalogue and a test can walk them.
    pub const ALL: &'static [Act] = &[Act::Set, Act::Create, Act::Remove, Act::Request];
}

impl core::fmt::Display for Act {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// **One thing a change does**, as fields rather than as a sentence.
///
/// `target` is the thing acted on and it is written the way an operator would
/// have to type it or look for it — an absolute path, a schema path, the name of
/// the process being asked. Never a description of one: a plan whose `target`
/// is *"the boot configuration"* is prose wearing a field name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effect {
    pub act: Act,
    pub target: String,
    /// What is there now. `None` when there is nothing — a [`Act::Create`], or a
    /// [`Act::Request`], which has no before.
    ///
    /// **Omitted rather than emitted empty**, which is `crate::value`'s standing
    /// convention: two ways to say *there is nothing here* is `ADR-011` §3 in
    /// miniature.
    pub from: Option<String>,
    /// What will be there. `None` for a [`Act::Remove`], which has no after.
    pub to: Option<String>,
    /// The one thing the fields cannot carry, when there is one.
    ///
    /// **Not a place to move the plan back into prose.** If it is answering
    /// *what changes*, it belongs in the fields or in a second [`Effect`]; this
    /// is for what an operator needs to *decide* — which mechanism reads the
    /// file, what a request is queued behind, what is irreversible.
    pub note: Option<String>,
}

impl Effect {
    pub fn set(target: impl Into<String>, from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            act: Act::Set,
            target: target.into(),
            from: Some(from.into()),
            to: Some(to.into()),
            note: None,
        }
    }

    pub fn create(target: impl Into<String>) -> Self {
        Self {
            act: Act::Create,
            target: target.into(),
            from: None,
            to: None,
            note: None,
        }
    }

    pub fn remove(target: impl Into<String>) -> Self {
        Self {
            act: Act::Remove,
            target: target.into(),
            from: None,
            to: None,
            note: None,
        }
    }

    /// Another process is asked to do something. `target` is **who is asked**,
    /// `to` is what they are asked for.
    pub fn request(target: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            act: Act::Request,
            target: target.into(),
            from: None,
            to: Some(to.into()),
            note: None,
        }
    }

    /// What is there now, for an act that has a before.
    #[must_use]
    pub fn was(mut self, from: impl Into<String>) -> Self {
        self.from = Some(from.into());
        self
    }

    /// What will be there, for an act that has an after.
    #[must_use]
    pub fn becomes(mut self, to: impl Into<String>) -> Self {
        self.to = Some(to.into());
        self
    }

    /// See [`Self::note`] for what belongs here and what does not.
    #[must_use]
    pub fn noting(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    /// This effect as a sentence, for [`Plan::narrative`].
    fn sentence(&self) -> String {
        let mut text = match (self.act, &self.from, &self.to) {
            (Act::Set, Some(from), Some(to)) => {
                format!("`{}` becomes {to} — it is {from} now", self.target)
            }
            (Act::Set, None, Some(to)) => format!("`{}` becomes {to}", self.target),
            (Act::Create, _, Some(to)) => format!("`{}` is created, holding {to}", self.target),
            (Act::Create, _, None) => format!("`{}` is created", self.target),
            (Act::Remove, Some(from), _) => format!("`{}` is removed — it is {from}", self.target),
            (Act::Remove, None, _) => format!("`{}` is removed", self.target),
            (Act::Request, _, Some(to)) => format!("`{}` is asked to {to}", self.target),
            // A `Set` with no `to` and a `Request` with nothing asked are both
            // effects that say nothing. Rendered rather than refused, because a
            // plan that panicked would take PID 1's dependency tree with it —
            // and because the words below are visibly wrong, which is what a
            // defect should look like.
            (act, _, None) => format!("`{}` — {act}, with nothing said about what", self.target),
        };
        if let Some(note) = &self.note {
            text.push_str(" (");
            text.push_str(note);
            text.push(')');
        }
        text
    }

    fn value(&self) -> Value {
        let mut fields = vec![
            ("act", Value::str(self.act.as_str())),
            ("target", Value::str(self.target.clone())),
        ];
        if let Some(from) = &self.from {
            fields.push(("from", Value::str(from.clone())));
        }
        if let Some(to) = &self.to {
            fields.push(("to", Value::str(to.clone())));
        }
        if let Some(note) = &self.note {
            fields.push(("note", Value::str(note.clone())));
        }
        Value::Object(fields)
    }
}

// ---------------------------------------------------------------------------
// When it takes effect — ADR-034 §5.3
// ---------------------------------------------------------------------------

/// **When the change becomes true of the running machine.** `ADR-034` §5.3.
///
/// See this module's documentation for why there are three states and not two.
/// The short form: `apply edit` writes an arbitrary path and cannot know, and a
/// type that forced it to claim one or the other would make the plan lie about
/// the one thing §5.3 exists to stop it lying about.
/// **It is `Copy` and its reason is `&'static str`**, which is not economy: the
/// same three states answer for a **setting in the configuration schema**
/// (`rivet_config::schema::Property`), where the table is `const` and a `String`
/// could not appear. `ADR-034` §5.3 names three consumers of this one fact — a
/// plan, the register's interface, and an agent — and one type for all three is
/// the whole of `ADR-011` applied to it. Every reason this build has is a
/// literal; the day one is not, this widens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakesEffect {
    /// The machine behaves differently as soon as this returns.
    Immediately,
    /// **The change is written and is not live.** This is `ADR-034` §5.3's whole
    /// case: the state that is *indistinguishable from success until the next
    /// boot*.
    OnNextRestart,
    /// **Rivet does not know**, and says which fact it is missing.
    ///
    /// `because` is a fact about this operation — *"this writes an arbitrary
    /// path and nothing in this build says what reads it"* — never an apology.
    NotAsserted { because: &'static str },
}

impl TakesEffect {
    /// The wire form an agent branches on. Part of the interface contract.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Immediately => "immediately",
            Self::OnNextRestart => "on-next-restart",
            Self::NotAsserted { .. } => "not-asserted",
        }
    }

    /// **`ADR-034` §5.3's question, answered as a three-state.**
    ///
    /// `Some(true)` means a restart is needed, `Some(false)` that it is not, and
    /// `None` that this build cannot say. A caller that treats `None` as `false`
    /// has reintroduced exactly the omission §5.3 forbids, which is why this
    /// returns an `Option` rather than defaulting.
    pub const fn requires_reboot(&self) -> Option<bool> {
        match self {
            Self::Immediately => Some(false),
            Self::OnNextRestart => Some(true),
            Self::NotAsserted { .. } => None,
        }
    }

    fn sentence(&self) -> String {
        match self {
            Self::Immediately => "as soon as this command returns".to_owned(),
            Self::OnNextRestart => {
                "ON THE NEXT RESTART, not now — until then this machine keeps behaving the way \
                 it does today"
                    .to_owned()
            }
            Self::NotAsserted { because } => format!(
                "NOT ASSERTED — {because}, so whether a restart is needed is not something this \
                 build can tell you"
            ),
        }
    }

    fn value(&self) -> Value {
        let mut fields = vec![("when", Value::str(self.as_str()))];
        // Omitted rather than null when it is not asserted, so a caller reading
        // `requires-reboot` gets a `bool` or gets nothing — never a third
        // spelling of "no".
        if let Some(needed) = self.requires_reboot() {
            fields.push(("requires-reboot", Value::Bool(needed)));
        }
        if let Self::NotAsserted { because } = self {
            fields.push(("because", Value::str(*because)));
        }
        Value::Object(fields)
    }
}

// ---------------------------------------------------------------------------
// The plan
// ---------------------------------------------------------------------------

/// **What a mutating operation would do, as a type.** `ADR-006` §6.5.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The subject as the operator named it — `trial-boot b`, `power-off`, the
    /// path an edit lands on. Carried into the refusal's `subject` field, so it
    /// is what they typed rather than a paraphrase (`AGENTS.md` §5).
    subject: String,
    /// One clause saying what kind of change this is, for the first sentence of
    /// [`Self::narrative`]: *"stops the machine"*, *"installs software on this
    /// machine"*. The operator's reason to pause.
    consequence: String,
    effects: Vec<Effect>,
    /// **What an operator might reasonably think changes and does not.**
    ///
    /// A field rather than a note because these are the sentences the
    /// hand-written plans worked hardest at — *"The `[all]` default is not
    /// touched"*, *"NOTHING IS RESTARTED"*, *"NOTHING HAS BEEN SENT"* — and they
    /// are the half an agent cannot infer from the effects.
    untouched: Vec<String>,
    takes_effect: TakesEffect,
    /// The exact command that does it. Generated by [`Self::confirmed_by`] from
    /// the words the operator used, never written out.
    confirm_with: String,
}

impl Plan {
    /// A plan for `subject`, whose consequence is `consequence`.
    ///
    /// Both halves are required because a plan with neither is a plan that says
    /// nothing, and the type is the place to make that impossible rather than
    /// the review.
    pub fn new(subject: impl Into<String>, consequence: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
            consequence: consequence.into(),
            effects: Vec::new(),
            untouched: Vec::new(),
            takes_effect: TakesEffect::Immediately,
            confirm_with: String::new(),
        }
    }

    #[must_use]
    pub fn doing(mut self, effect: Effect) -> Self {
        self.effects.push(effect);
        self
    }

    /// See [`Self::untouched`]. Takes what an operator would otherwise assume.
    #[must_use]
    pub fn leaving(mut self, untouched: impl Into<String>) -> Self {
        self.untouched.push(untouched.into());
        self
    }

    #[must_use]
    pub fn taking_effect(mut self, when: TakesEffect) -> Self {
        self.takes_effect = when;
        self
    }

    /// **The command that does it, assembled from the words the operator typed.**
    ///
    /// `words` is the invocation without the confirmation — so
    /// `["trial-boot", "b"]` produces `rivet apply trial-boot b confirm`. The
    /// verb and the confirmation word come from the one place each is defined,
    /// which is why no operation spells this sentence out any more: five of them
    /// used to, and five of them would have had to change together if the word
    /// ever did.
    #[must_use]
    pub fn confirmed_by<S: AsRef<str>>(mut self, words: &[S]) -> Self {
        let mut line = format!("rivet {}", Verb::Apply);
        for word in words {
            line.push(' ');
            line.push_str(word.as_ref());
        }
        line.push(' ');
        line.push_str(super::apply::CONFIRMATION);
        self.confirm_with = line;
        self
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }

    pub fn consequence(&self) -> &str {
        &self.consequence
    }

    pub fn effects(&self) -> &[Effect] {
        &self.effects
    }

    pub fn untouched(&self) -> &[String] {
        &self.untouched
    }

    pub const fn takes_effect(&self) -> &TakesEffect {
        &self.takes_effect
    }

    pub fn confirm_with(&self) -> &str {
        &self.confirm_with
    }

    /// **The human string, generated.** `ADR-006` §4.1's third field, and the
    /// only writer of one in this workspace — see the module documentation.
    ///
    /// The five sections are the ones the seven hand-written plans converged on
    /// independently, which is the evidence that they are the right five: what
    /// this is, what changes, what does not, when it lands, and how to say yes.
    pub fn narrative(&self) -> String {
        let mut text = format!(
            "this {consequence}, so it is not done without confirmation (ADR-012 §7.1). ",
            consequence = self.consequence
        );

        text.push_str("WHAT WOULD CHANGE: ");
        if self.effects.is_empty() {
            // Not reachable through `Change`, which refuses an empty plan — see
            // `resolve`. Written anyway rather than left to `join` producing an
            // empty sentence, because a plan that says nothing must LOOK wrong.
            text.push_str("nothing is listed, which is a defect in this operation");
        } else {
            text.push_str(
                &self
                    .effects
                    .iter()
                    .map(Effect::sentence)
                    .collect::<Vec<_>>()
                    .join("; "),
            );
        }
        text.push_str(". ");

        if !self.untouched.is_empty() {
            text.push_str("WHAT IS NOT TOUCHED: ");
            text.push_str(&self.untouched.join("; "));
            text.push_str(". ");
        }

        text.push_str("WHEN IT TAKES EFFECT: ");
        text.push_str(&self.takes_effect.sentence());
        text.push_str(". ");

        text.push_str("NOTHING HAS CHANGED YET. TO DO IT: repeat the command with `");
        text.push_str(super::apply::CONFIRMATION);
        text.push_str("` as the last word — `");
        text.push_str(&self.confirm_with);
        text.push('`');
        text
    }

    /// The plan as the structured payload a refusal carries.
    ///
    /// The narrative is **not** in here: it is the refusal's `message`, which is
    /// `ADR-006` §4.1's third field and the place a human string belongs. A plan
    /// that carried its own prose would put the same sentence in the payload and
    /// beside it.
    pub fn value(&self) -> Value {
        Value::object(vec![
            ("subject", Value::str(self.subject.clone())),
            ("consequence", Value::str(self.consequence.clone())),
            (
                "effects",
                Value::List(self.effects.iter().map(Effect::value).collect()),
            ),
            ("untouched", Value::strings(self.untouched.clone())),
            ("takes-effect", self.takes_effect.value()),
            ("confirm-with", Value::str(self.confirm_with.clone())),
        ])
    }
}

/// A plan becomes the refusal that carries it, and **nothing else constructs an
/// [`ErrorCode::ConfirmationRequired`]** — `only_a_plan_produces_a_confirmation_
/// required_refusal` in `super` is what holds that.
impl From<Plan> for OperationError {
    fn from(plan: Plan) -> Self {
        let subject = plan.subject.clone();
        let message = plan.narrative();
        // Built as `Unsafe` and re-coded by `carrying`, which is not a trick to
        // route around `OperationError::refuse`'s assertion but the reason that
        // assertion can exist at all: `carrying` is private, this is its only
        // caller, and so a `ConfirmationRequired` without a plan cannot be
        // spelled anywhere in this crate. The intermediate code is never
        // observed.
        OperationError::about(ErrorCode::Unsafe, subject, message).carrying(plan)
    }
}

// ---------------------------------------------------------------------------
// The framework
// ---------------------------------------------------------------------------

/// **A mutating operation.** `ADR-006` §6.5's *"type that computes its diff
/// separately from applying it"*, and the associated type is what makes the
/// separation structural — see the module documentation.
pub trait Change {
    /// **What [`Self::plan`] worked out that [`Self::apply`] needs.**
    ///
    /// `()` when applying needs nothing the plan did not, which is most of them.
    /// It is `super::edit`'s reason for existing: that operation reads a file,
    /// finds its one match and builds the replacement in order to describe the
    /// change, and doing it twice would be a second read and a window between
    /// them in which the file could differ from the one described.
    type Prepared;

    /// **What this operation returns when it has done the thing.**
    ///
    /// A per-subject type rather than a [`Value`], so that the framework costs
    /// nothing that was already there: `super::trial::Armed`,
    /// `super::stop::Stopping` and the rest are typed results *"so that a future
    /// adapter reading this operation programmatically gets fields rather than a
    /// document it has to index into"*, and a driver that flattened them to
    /// [`Value`] would have taken that away in exchange for a dry run.
    type Applied;

    /// **What this would do. Reads the machine; changes nothing.**
    ///
    /// Run on the confirmed path too, by [`resolve`] — so an interlock written
    /// here runs before any change, once, for every subject. Five subjects each
    /// carried a paragraph explaining why they did that by hand; it is the
    /// driver's now.
    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, Self::Prepared), OperationError>;

    /// **Do it.** Consumes the change and the plan's working, so neither can be
    /// applied twice and neither can be applied without the other.
    fn apply(
        self,
        prepared: Self::Prepared,
        board: &mut dyn Board,
    ) -> Result<Self::Applied, OperationError>;
}

/// **The one driver.** Plan, then either refuse with the plan or apply it.
///
/// Every mutating operation reaches its change through this function, which is
/// what makes the dry run impossible to omit rather than merely required. It
/// also owns the two properties the seven subjects each owned separately: the
/// interlocks run before anything changes, and the confirmation word decides
/// which of two things happens and nothing else does.
pub fn resolve<C: Change>(
    change: C,
    confirmed: bool,
    board: &mut dyn Board,
) -> Result<C::Applied, OperationError> {
    let (plan, prepared) = change.plan(board)?;
    debug_assert!(
        !plan.effects.is_empty(),
        "a change with no effects is a plan that says nothing: {}",
        plan.subject
    );
    if !confirmed {
        return Err(plan.into());
    }
    change.apply(prepared, board)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn a_plan() -> Plan {
        Plan::new("trial-boot b", "changes what the machine does on its next reset")
            .doing(Effect::set(
                "/boot/autoboot.txt [tryboot] boot_partition",
                "4",
                "6",
            ))
            .leaving("the `[all]` default, so an ordinary reset still boots slot a")
            .taking_effect(TakesEffect::OnNextRestart)
            .confirmed_by(&["trial-boot", "b"])
    }

    /// The command in the narrative is the command that does it, assembled from
    /// the one definition of the verb and the one definition of the word.
    #[test]
    fn the_confirmation_line_is_built_from_the_words_the_operator_typed() {
        let plan = a_plan();
        assert_eq!(plan.confirm_with(), "rivet apply trial-boot b confirm");
        assert!(plan.narrative().contains("rivet apply trial-boot b confirm"));
    }

    /// `ADR-034` §5.3: a change that is not live must say so, and an operator
    /// must not have to infer it from the absence of a sentence.
    #[test]
    fn a_change_that_is_not_live_says_so_in_the_payload_and_in_the_prose() {
        let plan = a_plan();
        assert_eq!(plan.takes_effect().requires_reboot(), Some(true));
        assert!(plan.narrative().contains("ON THE NEXT RESTART"));

        let Value::Object(fields) = plan.value() else {
            panic!("a plan is an object");
        };
        let when = fields
            .iter()
            .find(|(name, _)| *name == "takes-effect")
            .map(|(_, v)| v)
            .expect("a plan carries when it takes effect");
        assert_eq!(
            *when,
            Value::object(vec![
                ("when", Value::str("on-next-restart")),
                ("requires-reboot", Value::Bool(true)),
            ])
        );
    }

    /// The third state carries its reason, and **omits** `requires-reboot`
    /// rather than answering `false` — see [`TakesEffect::requires_reboot`].
    #[test]
    fn a_change_this_build_cannot_place_omits_the_field_rather_than_guessing() {
        let when = TakesEffect::NotAsserted {
            because: "this writes a path nothing in this build claims to read",
        };
        assert_eq!(when.requires_reboot(), None);
        assert_eq!(
            when.value(),
            Value::object(vec![
                ("when", Value::str("not-asserted")),
                (
                    "because",
                    Value::str("this writes a path nothing in this build claims to read")
                ),
            ])
        );
    }

    /// `ADR-006` §4.1's three fields, in its order of authority, and the plan in
    /// the second slot rather than only in the third.
    #[test]
    fn the_refusal_carries_the_plan_as_a_payload_and_not_only_as_prose() {
        let plan = a_plan();
        let refusal: OperationError = plan.clone().into();
        assert_eq!(refusal.code(), ErrorCode::ConfirmationRequired);
        assert_eq!(refusal.subject(), Some("trial-boot b"));
        assert_eq!(refusal.plan(), Some(&plan));
        assert_eq!(refusal.message(), plan.narrative());
    }

    /// The prose is generated, so every section an operator relies on is present
    /// for every plan rather than for the ones whose author remembered.
    #[test]
    fn every_narrative_carries_the_five_sections() {
        let narrative = a_plan().narrative();
        for section in [
            "WHAT WOULD CHANGE:",
            "WHAT IS NOT TOUCHED:",
            "WHEN IT TAKES EFFECT:",
            "NOTHING HAS CHANGED YET.",
            "TO DO IT:",
        ] {
            assert!(narrative.contains(section), "{section} is missing: {narrative}");
        }
    }

    /// A plan with nothing to leave alone omits that section rather than
    /// printing an empty one.
    #[test]
    fn a_plan_with_nothing_untouched_omits_the_section() {
        let plan = Plan::new("power-off", "stops the machine")
            .doing(Effect::request("pid 1", "power the machine off and stay off"))
            .confirmed_by(&["power-off"]);
        assert!(!plan.narrative().contains("WHAT IS NOT TOUCHED"));
        assert!(plan.narrative().contains("`pid 1` is asked to"));
    }

    /// Each act renders as itself, and a note rides beside the fields rather
    /// than replacing them.
    #[test]
    fn each_act_renders_as_the_thing_it_is() {
        assert_eq!(
            Effect::set("a", "1", "2").sentence(),
            "`a` becomes 2 — it is 1 now"
        );
        assert_eq!(Effect::create("a").sentence(), "`a` is created");
        assert_eq!(
            Effect::create("a").becomes("nothing yet").sentence(),
            "`a` is created, holding nothing yet"
        );
        assert_eq!(Effect::remove("a").sentence(), "`a` is removed");
        assert_eq!(
            Effect::request("pid 1", "stop").noting("it may refuse").sentence(),
            "`pid 1` is asked to stop (it may refuse)"
        );
    }

    /// The wire strings are the interface contract (`ADR-006` §4.1). This is
    /// what makes renaming one a decision rather than a refactor that compiles.
    #[test]
    fn the_act_and_timing_words_are_the_contract_and_this_is_the_list() {
        assert_eq!(
            Act::ALL.iter().map(|a| a.as_str()).collect::<Vec<_>>(),
            ["set", "create", "remove", "request"]
        );
        assert_eq!(TakesEffect::Immediately.as_str(), "immediately");
        assert_eq!(TakesEffect::OnNextRestart.as_str(), "on-next-restart");
        assert_eq!(
            TakesEffect::NotAsserted {
                because: ""
            }
            .as_str(),
            "not-asserted"
        );
    }

    /// An absent `from`, `to` or `note` is **omitted**, never emitted empty —
    /// `crate::value`'s standing convention, asserted rather than assumed.
    #[test]
    fn an_effect_omits_the_fields_it_has_no_answer_for() {
        assert_eq!(
            Effect::remove("/state/app/thing").value(),
            Value::object(vec![
                ("act", Value::str("remove")),
                ("target", Value::str("/state/app/thing")),
            ])
        );
    }
}
