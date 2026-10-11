//! `rivet schema` — the configuration document's JSON Schema, **generated
//! from the types**.
//!
//! `ADR-006` §7:
//!
//! > * **`rivet schema`** emits the JSON Schema of the configuration document,
//! >   generated from the types rather than maintained beside them.
//! >
//! > If these are generated from the same types the core uses, they cannot go
//! > stale. If they are hand-maintained documents, they will be wrong within a
//! > month and worse than absent, because they will be trusted.
//!
//! **Ruled 2026-08-22: required from Stage 1.** `ADR-007` §3.2 makes the same
//! demand from the other end — the schema "is generated from the Rust types,
//! never maintained beside them, so it cannot drift" — and names
//! self-description as one of the four things it buys.
//!
//! # What "generated from the types" means here, precisely
//!
//! This workspace has no `serde` and no `schemars`, deliberately:
//! `rivet-config`'s `Cargo.toml` records the reasoning and says in as many
//! words that whether the schema types carry serde derives is *"the maintainer's
//! call, not a side effect of this crate"*. So there is no reflection to
//! generate from, and the honest description of what happens instead is:
//!
//! * **The key names are the parser's own lists.** [`DOCUMENT`], [`SERVICE`],
//!   [`INTERFACE`] carries no key names of its own — every one is
//!   asserted equal, in order, to the `DECLARED_KEYS` the parser reads and the
//!   unknown-key refusal quotes. There is still exactly one list of keys per
//!   table (`ADR-011`).
//! * **The value rules are the parser's own diagnostics.** Every `rule` below is
//!   the same `RULE` constant a refusal quotes — `Method::RULE`,
//!   `MemoryLimit::RULE`, `AuthorizedKey::RULE`. A schema
//!   describing a value differently from the error the operator gets is exactly
//!   the "trusted and wrong" document §7 refuses.
//! * **Required-ness and type are asserted against the parser's behaviour**, not
//!   copied from it: for every declared property this module's tests feed the
//!   parser a document with the key, without it, and with a value of the wrong
//!   type, and check that the parser agrees with what the schema published.
//!
//! That is weaker than reflection and it is the strongest thing available
//! without taking a dependency decision that is not an agent's to take. It is
//! written down here rather than left to be inferred.
//!
//! # The emitted schema is necessary and not sufficient, and it says so
//!
//! JSON Schema can express *this key is a string*. It cannot express **a
//! gateway is on the interface's own subnet**, and there are five such rules
//! in `rivet-config` — `ADR-007` §7.3's four refusals and §7.7.2's key
//! structure. A schema that stayed quiet about them would tell an agent its
//! document was valid when the machine will refuse it, which is the failure
//! mode `ADR-006` §7 is about.
//!
//! **A sixth joined this list until 2026-09-13 and left it**: `ADR-007` §7's
//! `method = "dhcp"` carrying a static `address`/`gateway`. Plan §5.2 item 1
//! (`.rulings/` R-162) asks that a conditional rule appear in the emitted
//! JSON Schema in a standard way a generic form renderer can follow, and
//! `if`/`then`/`else` says the identical thing `Requirement::Conditional`'s
//! English already did — see this module's private `Conditional` struct —
//! so the schema now enforces this one structurally rather than only
//! describing it.
//!
//! So the emitted document carries [`NOT_THE_WHOLE_OF_VALIDITY`] on its root
//! description, naming `rivet-config` as the authority (`ADR-011` §2's second
//! clause: the system says which one, at the moment the answer is needed).
//!
//! **A limitation of this build, stated rather than discovered.** Nothing in
//! this workspace can validate the emitted document *as JSON Schema* — there is
//! no validator here and adding one is a dependency decision. The tests assert
//! its structure and its agreement with the parser; they do not assert that a
//! third-party validator reads it the way this module intends. `AGENTS.md` §4:
//! a round trip against one's own writer proves the two agree and nothing about
//! the format.

use rivet_core::Value;
use rivet_core::net::Ipv4Cidr;
use rivet_core::operation::plan::TakesEffect;

use crate::document::SCHEMA_VERSION;
use crate::network::Method;
use crate::resolver::{Resolver, SearchDomain};
use crate::services::{CpuLimit, MemoryLimit};
use crate::volumes::Guid;

/// The JSON Schema dialect this document is written against.
///
/// 2020-12 because it is the current one; nothing here uses a keyword that
/// distinguishes the drafts, so the choice costs nothing and is stated rather
/// than omitted — a document with no `$schema` is one a validator has to guess
/// the dialect of.
pub const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

/// The sentence that keeps the emitted schema honest about its own reach.
///
/// See the module documentation. The six refusals are the ones
/// `rivet-config` performs that no JSON Schema keyword can express.
pub const NOT_THE_WHOLE_OF_VALIDITY: &str = "\
Structural validity here is necessary and NOT sufficient. rivet-config also refuses: a gateway \
that is not on its interface's own subnet; a subnet's network or broadcast address offered as a \
host address; the same host address on two interfaces; and an interface name the kernel's \
dev_valid_name() would reject. (A `method = \"dhcp\"` interface carrying `address` or `gateway` \
is refused too, ADR-007 §7.4 — but that one the `allOf`/`if`/`then`/`else` below already states \
structurally, so it is not repeated here as a limitation.) rivet-config is the authority for \
what a valid document is (ADR-011); this schema is a description of it.";

/// **Why most settings in this build answer `requires-reboot: true`, and why
/// two do not, for two unrelated reasons.** `ADR-034` §5.3, and the one place
/// this reason is written.
///
/// ~~It is a fact about *this build*, not about the design, and it is one fact
/// rather than thirty: **nothing re-reads the configuration document while it is
/// running.** … `ADR-032` §3.2's `SIGHUP` — *re-read the document* — is ruled and
/// not built, and it is what will move some of these to `immediately`.~~
///
/// **`SIGHUP` landed 2026-08-26 and moved exactly one key.** The reason still
/// holds for the rest: pid 1 reads the document at boot, the supervisor reads it
/// when it starts and compiles its service set then, and `rivet-netd` is handed
/// the path when it is started. What changed is that `crate::schema::SERVICE`'s
/// `enabled` is now re-read on demand — for every service except the two a
/// running machine may not stop (`ADR-032` §3.2.1).
///
/// **`carry.*.version` answers `not-asserted` for a second, unrelated
/// reason**, added 2026-09-13 (`.rulings/` R-159 R20): it is not something a
/// running machine ever re-reads at all, on any schedule — `cmd/rivet-image
/// --fleet-repository` reads it, and does so before the machine a pinned
/// card will become exists. Neither `immediately` nor `on-next-restart`
/// describes a key with no running reader.
///
/// Written on the root description rather than repeated into every property,
/// which would be `ADR-011` §3 twelve times over. Each [`Property`] still
/// carries its own [`TakesEffect`], ~~because the answer is a property of the
/// setting and the day two settings differ is the day this sentence stops being
/// the whole of it~~ — **and that day was two days later**, which is the
/// argument for the per-property field rather than against it.
pub const NOTHING_RE_READS: &str = "\
WHEN A CHANGE TAKES EFFECT: every property below carries `takes-effect` and, where this build \
can assert it, `requires-reboot` (ADR-034 §5.3). Most answer `on-next-restart`, for one fact \
rather than one per key: pid 1 reads this document at boot, the supervisor reads it when it \
starts and compiles its service set then, and rivet-netd is handed the path when it is started \
— so editing those keys changes what the NEXT boot does and nothing about the machine you are \
talking to. THE EXCEPTION IS `services.*.enabled`: a SIGHUP reload re-reads this document and \
applies that key immediately (ADR-032 §3.2), except for `rivet-mcpd` and `rivet-netd`, whose \
value a running machine will not take because stopping either takes away the means of undoing \
it (ADR-032 §3.2.1). That key therefore answers `not-asserted` and says why, rather than \
publishing an answer that is false for two of the services it covers. `carry.*.version` \
answers `not-asserted` too, for an unrelated reason: it is read only when `cmd/rivet-image \
--fleet-repository` builds a NEW card from this document, never by a machine that is already \
running — there is no restart of an EXISTING machine that re-selects what it carries \
(ADR-050 §5, R-159 R20).";

/// Whether a key must be present, and what decides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    /// Present in every valid document, or in every entry of its table.
    Always,
    /// The operator may say nothing, and saying nothing means exactly one thing
    /// — which is `crate::services`' rule for when absence is allowed to mean
    /// anything at all.
    Optional,
    /// Required only under a stated condition.
    ///
    /// JSON Schema's `required` array cannot carry a condition, and expressing
    /// one with `oneOf` would put a second statement of the rule in a document
    /// nothing in this workspace can validate. So the condition is published in
    /// the key's description and the parser stays the authority.
    Conditional(&'static str),
}

impl Requirement {
    /// Whether this key belongs in JSON Schema's unconditional `required` list.
    const fn unconditional(self) -> bool {
        matches!(self, Self::Always)
    }
}

/// What a key's value may be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A TOML integer, optionally pinned to one value.
    Integer { constant: Option<i64> },
    Boolean,
    /// A string, optionally drawn from a closed vocabulary (`ADR-006` §6.4).
    Text { one_of: &'static [&'static str] },
    /// An array of strings.
    TextArray,
    /// A table whose keys the **operator** names — `[services.rivet-apid]`,
    /// `[network.eth0]` — each entry holding the given properties.
    NamedTables { entry: &'static [Property] },
    /// A table whose keys the operator names, where each ENTRY is also a
    /// table of the operator's own naming — `[applications.<name>]`
    /// `<setting> = "<value>"`. Neither level has a `&'static [Property]` to
    /// close it with: the outer name is the operator's and the inner one
    /// comes from an installed application's own manifest, a set this crate
    /// cannot see (`ADR-038` §4.2). Every value is a plain string, which is
    /// the whole vocabulary the parser keeps.
    NamedTablesOfStrings,
    /// [`Self::NamedTables`], plus one property that is not an operator-named
    /// entry at all — `[network.resolver]` beside every `[network.<iface>]`
    /// (`crate::resolver`). `reserved` publishes its own schema, description
    /// and prompt; every other key the operator writes still falls to
    /// `entry`, exactly as plain [`Self::NamedTables`] does.
    NamedTablesWithReserved {
        entry: &'static [Property],
        reserved: &'static Property,
    },
    /// A table with a fixed set of keys.
    Table { properties: &'static [Property] },
}

/// One key of the schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Property {
    /// **Never written here twice.** The tests assert the names of each table's
    /// properties are exactly its `DECLARED_KEYS`, in order.
    pub name: &'static str,
    pub requirement: Requirement,
    pub kind: Kind,
    /// **When a change to this setting becomes true of the running machine.**
    /// `ADR-034` §5.3, added 2026-08-26.
    ///
    /// > `requires-reboot` is a property of the setting, **in the schema** — not
    /// > a note in a message and not something the operator is expected to
    /// > remember.
    ///
    /// The hazard §5.3 names is `PLAN.md` §0.0.4's: **a configuration that is
    /// wrong in a way indistinguishable from success until the next boot.** An
    /// operator who disables something, sees a success and finds it still
    /// running has been told something false by omission — and an *agent* has no
    /// way at all to find out, because nothing it can read says so.
    ///
    /// [`rivet_core::operation::plan::TakesEffect`] rather than a `bool` of this
    /// crate's own, and that is the point of §5.3: one fact with three consumers
    /// — a plan, the fleet register's interface, and an agent branching
    /// structurally — is one type. A second spelling here would be `ADR-011` §3
    /// with the two halves a crate boundary apart.
    ///
    /// ~~**Every property in this build answers [`TakesEffect::OnNextRestart`]**,
    /// and [`NOTHING_RE_READS`] is the one place the reason is written. That is
    /// a fact about *this build* rather than about the design: `ADR-032` §3.2's
    /// `SIGHUP` is ruled and not built, and it is what will move some of these.~~
    ///
    /// **`SIGHUP` landed 2026-08-26 and moved exactly one**, which is what that
    /// sentence was written to make happen — `every_setting_publishes_when_a_
    /// change_to_it_takes_effect` failed until this changed, so the reload could
    /// not ship while the schema still called its settings `on-next-restart`.
    ///
    /// `services.*.enabled` is now [`TakesEffect::NotAsserted`], and **that is
    /// the honest answer rather than a hedge**: a reload applies it to most
    /// services and withholds it for `crate::schema`'s reachability set —
    /// `rivet-mcpd` and `rivet-netd`, whose `enabled` a running machine may not
    /// take because stopping them takes away the means of undoing that
    /// (`ADR-032` §3.2.1). So the answer depends on **which service** the
    /// property sits under, and this schema deliberately does not know service
    /// names: `Kind::NamedTables` has one `entry` list for every key the
    /// operator names. `Immediately` would be false for two services and
    /// `OnNextRestart` false for the rest; `NotAsserted` with the reason is the
    /// only one of the three that is true, and it is `ADR-006` §6.5.2's
    /// argument reaching the schema.
    pub takes_effect: TakesEffect,
    /// What the key means, in the vocabulary `ADR-007` §7 uses for it.
    pub description: &'static str,
    /// The value's own rule, taken from the constant a refusal quotes, so the
    /// schema and the error an operator gets cannot describe the value
    /// differently.
    pub rule: Option<&'static str>,
    /// **The short form an interview or a form asks with** — added 2026-09-13
    /// (plan §5.2 item 1, `.rulings/` R-162).
    ///
    /// `description` is written for a reader deciding whether a value is
    /// right; `prompt` is written for a reader who has not typed anything
    /// yet. The two answer different questions and a generic form renderer
    /// (plan §5.2 item 3) needs both — a full `description` as a question
    /// reads like `rivet get schema`'s own prose read aloud, which is exactly
    /// the "appending schema-derived questions after hand-written ones"
    /// mismatch plan §5.1 names. Every property carries one, including a
    /// table's own (`[kiosk]`, `[control-plane]`) — a generated interview
    /// asking "configure this section?" needs a sentence too.
    pub prompt: &'static str,
}

/// A conditional rule between sibling keys in one table, expressed as JSON
/// Schema's own `if`/`then`/`else` rather than only in a property's English
/// description — added 2026-09-13 (plan §5.2 item 1, `.rulings/` R-162) so a
/// generic form renderer (plan §5.2 item 3) can follow the same rule this
/// crate's parser enforces, rather than parsing the description's prose.
///
/// **This does not invent a new rule.** `ADR-007` §7 is the parser's own
/// answer for what happens when a document disagrees with itself: *"refused,
/// not resolved — not by precedence, not by 'the more specific key wins', and
/// not by ignoring the unused key."* `if`/`then`/`else` states that same
/// refusal structurally: `then_required` is what the parser demands when `on
/// == when`, and `else_forbidden` is what it refuses outright otherwise —
/// [`INTERFACE`]'s `method`/`address`/`gateway` is where the parser already
/// draws this line, and this struct only republishes it.
struct Conditional {
    /// The sibling property whose value decides this rule. Always a
    /// `Kind::Text` with a closed vocabulary in every case this schema has
    /// today.
    on: &'static str,
    /// The value of `on` under which `then_required` is required.
    when: &'static str,
    /// Properties required when `on == when`.
    then_required: &'static [&'static str],
    /// Properties refused outright (must be entirely absent) when `on !=
    /// when`.
    else_forbidden: &'static [&'static str],
}

/// `network.<name>.method` decides `address` and `gateway` — `ADR-007` §7.
const INTERFACE_CONDITIONALS: &[Conditional] = &[Conditional {
    on: "method",
    when: "static",
    then_required: &["address"],
    else_forbidden: &["address", "gateway"],
}];

/// Which conditionals apply to a table, found by the identity of its
/// `properties`/`entry` slice — every table this schema declares is a single
/// `'static` constant, so pointer identity is exact and cheap, and a table
/// with no conditional rule (every table but two, today) costs one
/// comparison and an empty slice.
fn conditionals_for(properties: &'static [Property]) -> &'static [Conditional] {
    if core::ptr::eq(properties.as_ptr(), INTERFACE.as_ptr()) {
        INTERFACE_CONDITIONALS
    } else {
        &[]
    }
}

/// **The gate that governs `name` within `properties`, if any** — `(on,
/// when)`, read as *"`name` is asked and written only when the sibling
/// property `on` equals `when`"*.
///
/// **Republishes [`Conditional`]; does not invent a new one.** A property
/// gates on its conditional's `then_required` OR `else_forbidden` list —
/// `address` is in both (required under `static`, forbidden under `dhcp`);
/// `gateway` is only in the second (optional under `static`, still forbidden
/// under `dhcp`) — either membership means the same thing to an interview:
/// do not ask, and do not write, unless `on == when`.
fn gate(properties: &'static [Property], name: &str) -> Option<(&'static str, &'static str)> {
    conditionals_for(properties)
        .iter()
        .find(|c| c.then_required.contains(&name) || c.else_forbidden.contains(&name))
        .map(|c| (c.on, c.when))
}

/// `gate`, named by which table `name` lives in rather than by the table's
/// own slice — exposed for `crate::interview` (plan §5.2 item 3, `.rulings/`
/// R-162) so a generated interview can skip a gated question exactly where
/// the parser would refuse or discard it — the gateway under `method =
/// "dhcp"` (`ADR-007` §7).
///
/// **Named rather than passed as a slice, and that is deliberate.** `const
/// INTERFACE` is re-materialised at each point of use, and a
/// slice reference that travelled through a caller's own `const` (as
/// `crate::interview::EXPANSIONS` does) is not guaranteed to be the same
/// allocation `conditionals_for`'s pointer check compares against — measured
/// directly: `gate` called with a slice that arrived this way missed every
/// time. Every call inside this module passes `INTERFACE`
/// directly, which is the one shape `conditionals_for`'s pointer check was
/// ever proven against, so `named_gate` is the one door a caller outside
/// this module uses instead of re-deriving that fragility for itself.
#[must_use]
pub fn named_gate(table: &str, name: &str) -> Option<(&'static str, &'static str)> {
    match table {
        "network" => gate(INTERFACE, name),
        _ => None,
    }
}

/// One conditional rule, as JSON Schema.
fn conditional_schema(c: &Conditional) -> Value {
    let mut block = vec![(
        "if",
        Value::object(vec![
            (
                "properties",
                Value::object(vec![(c.on, Value::object(vec![("const", Value::str(c.when))]))]),
            ),
            ("required", Value::strings([c.on])),
        ]),
    )];
    if !c.then_required.is_empty() {
        block.push((
            "then",
            Value::object(vec![("required", Value::strings(c.then_required.iter().copied()))]),
        ));
    }
    if !c.else_forbidden.is_empty() {
        block.push((
            "else",
            Value::object(vec![(
                "not",
                Value::object(vec![(
                    "anyOf",
                    Value::List(
                        c.else_forbidden
                            .iter()
                            .map(|k| Value::object(vec![("required", Value::strings([*k]))]))
                            .collect(),
                    ),
                )]),
            )]),
        ));
    }
    Value::Object(block)
}

/// `[services.<name>]` — `ADR-007` §7.2, `ADR-017` §4.1.
pub const SERVICE: &[Property] = &[
    Property {
        name: "enabled",
        requirement: Requirement::Always,
        kind: Kind::Boolean,
        // **The one property a reload moves**, and the one whose answer depends
        // on which service it sits under — see `Property::takes_effect`.
        takes_effect: TakesEffect::NotAsserted {
            because: "a SIGHUP reload applies this to most services immediately (ADR-032 §3.2), \
                      and WITHHOLDS it for `rivet-mcpd` and `rivet-netd`, which a running machine \
                      may not stop because that would take away the means of undoing it \
                      (ADR-032 §3.2.1, ADR-018 §8). Those two take the document's answer at the \
                      next boot. Which of the two this key is depends on the service it is \
                      under, and this schema does not know service names",
        },
        description: "Whether this service should run.",
        rule: None,
        prompt: "Should this service run?",
    },
    // `listen` was here until 2026-08-24 and is now the signed manifest's
    // (the maintainer's 2026-08-24 ruling). A published schema that still declared it
    // would be telling an operator to write a key this build refuses — which is
    // §7's "trusted and wrong" document, in the one direction that is worse
    // than silence.
    Property {
        name: "memory",
        requirement: Requirement::Optional,
        kind: Kind::Text { one_of: &[] },
        takes_effect: TakesEffect::OnNextRestart,
        description: "A cap on the memory this service may have. It can only TIGHTEN what the \
                      service's own manifest requested, never raise it — where both name one, \
                      the lower wins (ruled 2026-08-24).",
        rule: Some(MemoryLimit::RULE),
        prompt: "A memory cap for this service, or blank for none:",
    },
    Property {
        name: "cpu",
        requirement: Requirement::Optional,
        // **`memory`'s footing exactly, on `ADR-007` §7 and `ADR-015` §3's
        // 2026-09-09 amendment.** Written into a cgroup at the next start,
        // exactly where `memory` is, so `takes-effect` is the same answer for
        // the same reason: `crate::supervise::apply_cap` writes it into the
        // cgroup that a spawn creates, and a reload cannot move a value
        // already written to a running process's own cgroup.
        kind: Kind::Text { one_of: &[] },
        takes_effect: TakesEffect::OnNextRestart,
        description: "A cap on the CPU this service may have, as a decimal number of cores — \
                      \"0.5\" is half a core. It can only TIGHTEN what the service's own \
                      manifest requested, never raise it — where both name one, the lower wins \
                      (ADR-007 §7, ADR-015 §3, ruled 2026-09-09). There is no default: \
                      absent leaves the service uncapped.",
        rule: Some(CpuLimit::RULE),
        prompt: "A CPU cap for this service, or blank for none:",
    },
    Property {
        name: "critical",
        requirement: Requirement::Optional,
        kind: Kind::Boolean,
        // **Immediately**, and it is one of the few keys for which that is
        // unambiguous: nothing is started or stopped by it, so a reload has
        // nothing to withhold. It changes only what the machine's own health
        // answer folds over, which is recomputed on every `get status`.
        takes_effect: TakesEffect::Immediately,
        description: "Whether this service's health moves the MACHINE's health. Defaults to \
                      false. A `degraded` weather probe must not make a machine `degraded` when \
                      everything that matters is fine, so only services marked here roll up \
                      (ADR-045 §4). It is the OPERATOR's word and deliberately not a manifest \
                      key: the same binary is critical on the machine that serves the house and \
                      unimportant on a bench, and an application that could declare itself \
                      critical would be setting its own severity (ADR-045 §4).",
        rule: None,
        prompt: "Should this service's health affect the machine's health?",
    },
];

/// `[network.<name>]` — `ADR-007` §7.3.
pub const INTERFACE: &[Property] = &[
    Property {
        name: "method",
        requirement: Requirement::Always,
        kind: Kind::Text {
            one_of: Method::ALL,
        },
        takes_effect: TakesEffect::OnNextRestart,
        description: "How this interface is addressed. Required rather than inferred from the \
                      presence of `address`, because an inferred method has nowhere for the \
                      operator's intent to be written (ADR-007 §7.4).",
        rule: Some(Method::RULE),
        prompt: "How does this machine get an address?",
    },
    Property {
        name: "address",
        requirement: Requirement::Conditional(
            "required under `method = \"static\"`, and refused under `method = \"dhcp\"`",
        ),
        kind: Kind::Text { one_of: &[] },
        takes_effect: TakesEffect::OnNextRestart,
        description: "The host address and its subnet.",
        rule: Some(Ipv4Cidr::RULE),
        prompt: "The address, with its prefix, e.g. 192.168.1.50/24:",
    },
    Property {
        name: "gateway",
        requirement: Requirement::Conditional(
            "optional under `method = \"static\"`, and refused under `method = \"dhcp\"`",
        ),
        kind: Kind::Text { one_of: &[] },
        // `ADR-007` §7.3's own words for this row.
        takes_effect: TakesEffect::OnNextRestart,
        description: "The default route's next hop: a bare host address with no prefix. Absent \
                      means this interface is addressed but is not the way off this machine.",
        rule: None,
        prompt: "The router, or blank for none:",
    },

    Property {
        name: "ssid",
        requirement: Requirement::Optional,
        kind: Kind::Text { one_of: &[] },
        takes_effect: TakesEffect::OnNextRestart,
        description: "The wireless network this interface joins (ADR-004), before it is \
                      addressed. Its PRESENCE is what makes an interface wireless - there is no \
                      `wireless = true`. That inference is sound where inferring `method` from \
                      `address` is not (ADR-007 §7.4): no interface can join a network without \
                      naming one, so an absent SSID means exactly one thing, a wired interface. \
                      THERE IS NO KEY FOR THE PASSPHRASE and there must not be: what joins is a \
                      PMK derived from the passphrase AND this SSID, kept in ADR-020's store under \
                      the interface's name (ADR-004 §5, ADR-007 §7.3), which is what makes a \
                      document safe to read, diff and paste into a bug report. A changed SSID \
                      therefore needs a PMK derived for it. An SSID is not a credential: it \
                      travels in the clear whenever a device joins.",
        rule: Some(rivet_core::ssid::Ssid::RULE),
        prompt: "The wireless network name, or blank for a wired interface:",
    },
];

/// `[network.resolver]`'s own two keys — `crate::resolver::Resolver`'s
/// `DECLARED_KEYS`, restated as schema properties rather than copied, on
/// every other table's footing. Both optional: an operator may set neither,
/// either or both, and a table that states nothing is not the same as a
/// table that is absent (`crate::resolver`).
const RESOLVER: &[Property] = &[
    Property {
        name: "nameservers",
        requirement: Requirement::Optional,
        kind: Kind::TextArray,
        takes_effect: TakesEffect::OnNextRestart,
        description: "The IPv4 nameservers this machine asks, in the order they will be \
                      asked. Absent or empty defers to whatever a DHCP lease supplies.",
        rule: Some(Resolver::RULE),
        prompt: "The nameservers to ask, in order, or blank to accept a lease's own:",
    },
    Property {
        name: "search",
        requirement: Requirement::Optional,
        kind: Kind::TextArray,
        takes_effect: TakesEffect::OnNextRestart,
        description: "Domains appended to an unqualified name before it is asked. Absent or \
                      empty defers to whatever a DHCP lease supplies.",
        rule: Some(SearchDomain::RULE),
        prompt: "Search domains to append to an unqualified name, or blank for none:",
    },
];

/// The reserved child `[network.resolver]` is, wherever `network`'s own
/// entry schema is built (`property`) and wherever it is walked (`paths`).
/// `crate::resolver::Resolver::TABLE` names it, so the parser's reserved
/// word and the schema's reserved property cannot spell it two ways.
const RESOLVER_TABLE: Property = Property {
    name: Resolver::TABLE,
    requirement: Requirement::Optional,
    kind: Kind::Table { properties: RESOLVER },
    takes_effect: TakesEffect::OnNextRestart,
    description: "The nameservers and search domains this machine is TOLD to use, as opposed \
                  to any it might lease — `crate::resolver` carries the whole trade. Reserved \
                  out of the interface names above, so an interface may not be called \
                  `resolver`. ABSENT OR EMPTY both state nothing, which is what lets a DHCP \
                  lease answer instead.",
    rule: None,
    prompt: "Set the nameservers or search domains this machine is told to use?",
};

/// `[volumes.<name>]` — `ADR-031` §7.1.
pub const VOLUME: &[Property] = &[Property {
    name: "guid",
    requirement: Requirement::Always,
    kind: Kind::Text { one_of: &[] },
    // `ADR-031` §7.2 is why this is the only key, and why the schema saying
    // so matters: `device`, `path` and `label` are the three keys an operator
    // reaches for, and `additionalProperties: false` is what tells a generator
    // not to emit them before the machine refuses them by name.
    takes_effect: TakesEffect::OnNextRestart,
    description: "Which partition this volume is. The GUID rather than a device path, because \
                  /dev/sda2 is a statement about enumeration order and a GUID is a statement \
                  about the partition (ADR-031 §4).",
    rule: Some(Guid::RULE),
    prompt: "The partition's GUID:",
}];


/// `[control-plane]` - `ADR-035` §7.2, added 2026-08-28.
const CONTROL_PLANE: &[Property] = &[Property {
    name: "listen",
    requirement: Requirement::Always,
    kind: Kind::Text { one_of: &[] },
    takes_effect: TakesEffect::OnNextRestart,
    description: "Where rivet-mcpd binds. A NON-LOOPBACK address is accepted here and \
                  refused by the program unless it also has a TLS identity (ADR-035 §7.2) \
                  - this document says WHERE to listen, and what a machine will do without \
                  an identity is the program's answer rather than the schema's. It takes \
                  effect when the service next starts: a listener does not move while it \
                  is bound.",
    rule: Some(crate::control_plane::LISTEN_RULE),
    prompt: "Where should the control plane listen?",
}];

/// `[splash]` — what the boot screen may say. Added 2026-08-30.
///
/// **Both default to `true`, and that is the interesting half.** A screen that
/// says nothing useful would be the safe default only if the screen were a
/// liability, and it is not: a headless board whose address came from DHCP is
/// otherwise a machine nobody can find without reading a router's lease table.
/// `[display]` - the hours the screen is lit. `ADR-039` §3.1, added 2026-09-02.
///
/// **Both are `Always`**, which is how "both or neither" is expressed without
/// a condition: the TABLE is optional, and a table that exists carries both.
/// `[control-plane]` does the same with its one key, and it is why an empty
/// `[display]` is refused rather than accepted as silence - the way to say
/// nothing is to omit the table.
const DISPLAY: &[Property] = &[
    Property {
        name: "off",
        requirement: Requirement::Always,
        kind: Kind::Text { one_of: &[] },
        takes_effect: TakesEffect::OnNextRestart,
        description: "The LOCAL time of day the screen goes dark, as HH:MM on a 24-hour \
                      clock. REQUIRES `on`: a screen told when to go dark and never when to \
                      come back is one somebody has to visit, so `off` alone is refused \
                      rather than defaulted. LOCAL and not UTC - it is read against the \
                      `timezone` key, because an operator who says `off at seven` means the \
                      clock on the wall, and a UTC schedule is an hour wrong for half the \
                      year in most of the world.",
        rule: Some(rivet_core::display::TimeOfDay::RULE),
        prompt: "What local time should the screen go dark?",
    },
    Property {
        name: "on",
        requirement: Requirement::Always,
        kind: Kind::Text { one_of: &[] },
        takes_effect: TakesEffect::OnNextRestart,
        description: "The LOCAL time of day the screen comes back. REQUIRES `off`. The \
                      window MAY CROSS MIDNIGHT and normally does: `off = \"19:00\"` with \
                      `on = \"07:00\"` is dark overnight, which is the case this table was \
                      added for. The two may not be EQUAL - a window of no width says \
                      neither `never dark` (leave the table out) nor `never lit`, so it is \
                      refused rather than guessed at.",
        rule: Some(rivet_core::display::TimeOfDay::RULE),
        prompt: "What local time should the screen come back?",
    },
];

const SPLASH: &[Property] = &[
    Property {
        name: "endpoint",
        requirement: Requirement::Optional,
        kind: Kind::Boolean,
        takes_effect: TakesEffect::OnNextRestart,
        description: "Whether the boot screen names the address and port to reach this \
                      machine on. DEFAULT TRUE. It is drawn when the address LANDS rather \
                      than when the screen is first painted: pid 1 starts the splash before \
                      the supervisor, so on a DHCP boot there is no address for another ten \
                      to forty seconds, and the line appears then. Setting this false is for \
                      a screen somewhere a passer-by can read it - it is NOT a control over \
                      anything reachable on the network, since the address is broadcast by \
                      rivet-mdnsd and answered by ARP regardless.",
        rule: Some(crate::splash::Splash::RULE),
        prompt: "Show the machine's address and port on the boot screen?",
    },
    Property {
        name: "version",
        requirement: Requirement::Optional,
        kind: Kind::Boolean,
        takes_effect: TakesEffect::OnNextRestart,
        description: "Whether the boot screen names the version. DEFAULT TRUE. Same argument \
                      as `endpoint`: it stops a passer-by with a camera and nothing else, \
                      because the version is in every `get machine` reply.",
        rule: Some(crate::splash::Splash::RULE),
        prompt: "Show the version on the boot screen?",
    },
];

/// `[kiosk]` — which application holds the display. Added 2026-08-31.
///
/// **One key holding one name, because two applications naming themselves the
/// auto-run is a state that must not exist.** A per-service `auto-run = true`
/// could say it, and something would then have to decide which won.
pub const KIOSK: &[Property] = &[
    Property {
        name: "auto-run",
        requirement: Requirement::Optional,
        kind: Kind::Text { one_of: &[] },
        takes_effect: TakesEffect::OnNextRestart,
        description: "The name of the installed application that gets the SCREEN. Every other \
                      application declaring a display is OFFERED and not started - `enabled` \
                      puts one on the launcher's list, and this names the one of them that \
                      runs. AN APPLICATION NAMED HERE THAT IS NOT `enabled` DOES NOT RUN: \
                      this selects from what `enabled` offers rather than overriding it, and \
                      the contradiction resolves as an empty screen and a journal record \
                      naming both keys. A name matching nothing installed is KEPT rather than \
                      refused, on ADR-007 §3.6.3's rule that a document is authored anywhere \
                      - what it costs is a screen with nothing on it, which is visible.",
        rule: Some(crate::kiosk::Kiosk::RULE),
        prompt: "Which application takes the screen, or blank for none yet:",
    },
    // `locked` - PLAN.md 108.2, ruled 2026-09-01.
    Property {
        name: "locked",
        requirement: Requirement::Optional,
        kind: Kind::Boolean,
        takes_effect: TakesEffect::OnNextRestart,
        description: "Whether there is any way back to the launcher's menu. FALSE (the \
                      default) is an ordinary machine: the auto-run application takes the \
                      screen and holding the back control returns to the menu. TRUE is a \
                      KIOSK IN A PUBLIC PLACE: the menu is never reachable, the held back \
                      control does nothing, and neither does `apply input long-back` - an \
                      agent may press what a person may press and no more (ADR-012 9.3), \
                      so the lock does not weaken for whoever is asking. AN OPERATOR IS \
                      NOT LOCKED OUT: the control plane can clear this key. IF THE \
                      APPLICATION WILL NOT STAY UP the launcher shows an error screen \
                      naming it and a code, because it has nothing else it may display - \
                      it does not fall back to the menu, which would let a crash defeat \
                      the lock. TRUE WITH NO `auto-run` is kept rather than refused and \
                      resolves as that error screen.",
        rule: Some(crate::kiosk::Kiosk::LOCKED_RULE),
        prompt: "Lock the menu away so nobody can leave the kiosk application?",
    },
    Property {
        name: "launcher",
        requirement: Requirement::Optional,
        kind: Kind::Text { one_of: &[] },
        takes_effect: TakesEffect::OnNextRestart,
        description: "The service that is this machine's LAUNCHER: the one given the kiosk \
                      socket, and the screen itself when no compositor is enabled. DEFAULT \
                      `rivet-launcher`, the image's own. A service must also declare connects \
                      = [\"rivet-sysd:kiosk\"] to be given the role; any other service declaring \
                      it is refused the socket and the journal names both, so an application \
                      cannot take the menu away by claiming it. Read once at boot.",
        rule: Some(crate::kiosk::Kiosk::ROLE_RULE),
        prompt: "Which service is the launcher, or blank for the image's own:",
    },
    Property {
        name: "compositor",
        requirement: Requirement::Optional,
        kind: Kind::Text { one_of: &[] },
        takes_effect: TakesEffect::OnNextRestart,
        description: "The service that is this machine's COMPOSITOR, which arbitrates the \
                      display whenever it is enabled. DEFAULT `rivet-displayd`, the image's own. \
                      A service must also declare provides = [\"socket:wayland\"] to be given \
                      the screen; any other service declaring it is given only what an \
                      ordinary application is. A desktop machine names its own compositor \
                      here. Read once at boot.",
        rule: Some(crate::kiosk::Kiosk::ROLE_RULE),
        prompt: "Which service is the compositor, or blank for the image's own:",
    },
];

/// One repository, as a schema — `crate::repos`, ruled 2026-08-26.
///
/// **One key, and it is the identifier**, because that is the only fact about a
/// repository that cannot be derived. Where it answers is the table's own name;
/// where it lives on disk, what its default branch is and who may reach it are
/// all the projection's, derived or ruled elsewhere - and `ADR-007` §3.6
/// refuses a key it would discard.
pub const REPO: &[Property] = &[Property {
    name: "id",
    requirement: Requirement::Always,
    kind: Kind::Text { one_of: &[] },
    takes_effect: TakesEffect::OnNextRestart,
    description: "Which repository this is. MINTED, NEVER CHOSEN - `rivet-fleet add repo` \
                  produces one and a hand-written value is refused. THE IDENTIFIER IS THE \
                  REPOSITORY AND THE TABLE NAME IS A LABEL (ADR-025 4.1): to RENAME one, \
                  change the table name and leave this alone, and the machine moves the \
                  directory. Change this instead and you have asked for a different \
                  repository.",
    rule: Some(crate::repos::RepoId::RULE),
    prompt: "The repository's minted identifier:",
}];

/// One carried product's version pin, as a schema — `crate::carry`,
/// `.rulings/` R-159 R20.
///
/// **One key, and it is the version**, on [`REPO`]'s footing exactly: it is
/// the only fact about a pin that cannot be derived. Which product it names is
/// the table's own name, and whether that version has actually been published
/// is a fleet repository's question, not this schema's (`ADR-014` §3 — this
/// crate performs no I/O).
pub const CARRY: &[Property] = &[Property {
    name: "version",
    requirement: Requirement::Always,
    kind: Kind::Text { one_of: &[] },
    // **`not-asserted`, and for a reason this schema has not needed before.**
    // Every other `not-asserted` key (`services.*.enabled`) answers that way
    // because the honest answer depends on WHICH service a running machine is
    // asked about. This one is different in kind: it is read only once, by
    // `cmd/rivet-image --fleet-repository`, when a NEW CARD is built from this
    // document — never by a machine that is already running. There is no
    // restart of an EXISTING machine that re-selects what it carries, so
    // neither `immediately` nor `on-next-restart` is true of it, and
    // `on-next-restart`'s own definition ("the NEXT BOOT" of the machine this
    // document is talking to) would be a claim this key cannot back up.
    takes_effect: TakesEffect::NotAsserted {
        because: "read only when `cmd/rivet-image --fleet-repository` builds a NEW card from \
                  this document, to choose among several published versions of the named \
                  product (ADR-050 §5, R-159 R20). A machine that is already running never \
                  re-reads it: there is no restart of an EXISTING machine that re-selects what \
                  it carries, only a new card built from this document",
    },
    description: "Which published version of this product the card must carry, when the fleet \
                  repository holds more than one (`cmd/rivet-image --fleet-repository`, ADR-050 \
                  §5). Absent and exactly one version is published: that one is carried. \
                  Absent and several are published: refused by name, naming every candidate and \
                  suggesting the newest. Pinned and the version is not published: refused by \
                  name (`AGENTS.md` §5 — refuse rather than guess).",
    rule: None,
    prompt: "Which published version to pin, or blank to resolve automatically:",
}];

/// One name in the name service's table — `crate::dns::Record`, `ADR-047` §8.
pub const DNS_RECORD: &[Property] = &[
    Property {
        name: "address",
        requirement: Requirement::Always,
        kind: Kind::Text { one_of: &[] },
        takes_effect: TakesEffect::OnNextRestart,
        description: "The IPv4 address this name answers with. The router owns addresses and \
                      Rivet owns names (ADR-047 §1), so this is the address the router's \
                      reservation gives the machine, written here by hand.",
        rule: Some(crate::dns::Record::ADDRESS_RULE),
        prompt: "The address this name answers with:",
    },
    Property {
        name: "ttl",
        requirement: Requirement::Optional,
        kind: Kind::Integer { constant: None },
        takes_effect: TakesEffect::OnNextRestart,
        description: "This record's own TTL, overriding `[dns] ttl`. It exists for one move: \
                      lower it the day before the address changes, change it, put it back \
                      (ADR-047 §8). A client already holding the record keeps it for the OLD \
                      TTL, so lowering it after the change does nothing.",
        rule: Some(crate::dns::Ttl::RULE),
        prompt: "This record's TTL in seconds, or blank for the table's:",
    },
];

/// `[dns]` — the name service's zone, its two TTLs and its records.
/// `crate::dns`, `ADR-047` §8.
pub const DNS: &[Property] = &[
    Property {
        name: "zone",
        requirement: Requirement::Always,
        kind: Kind::Text { one_of: &[] },
        takes_effect: TakesEffect::OnNextRestart,
        description: "The zone every record is placed under, such as `my-fleet.home.arpa`. A \
                      name under it that the table does not hold is answered NXDOMAIN by this \
                      machine and is NOT forwarded, so a gap in the zone cannot leak to a \
                      public resolver (ADR-047 §7).",
        rule: Some(crate::dns::Zone::RULE),
        prompt: "The zone this machine answers for:",
    },
    Property {
        name: "ttl",
        requirement: Requirement::Optional,
        kind: Kind::Integer { constant: None },
        takes_effect: TakesEffect::OnNextRestart,
        description: "How long other machines may cache a record from this table. DEFAULT 300 \
                      (ADR-047 §8): a record that outlives a change leaves the network wrong \
                      with no way to force it, and one fetched often costs a query the board \
                      answers from memory.",
        rule: Some(crate::dns::Ttl::RULE),
        prompt: "How long a record may be cached, in seconds:",
    },
    Property {
        name: "negative-ttl",
        requirement: Requirement::Optional,
        kind: Kind::Integer { constant: None },
        takes_effect: TakesEffect::OnNextRestart,
        description: "How long other machines may cache the ABSENCE of a name in the zone. \
                      DEFAULT 60 (ADR-047 §8), and it governs whether ADDING a name works: a \
                      device that asked before the name existed keeps the absence this long.",
        rule: Some(crate::dns::Ttl::RULE),
        prompt: "How long an absent name may be cached, in seconds:",
    },
    Property {
        name: "records",
        requirement: Requirement::Optional,
        kind: Kind::NamedTables { entry: DNS_RECORD },
        takes_effect: TakesEffect::OnNextRestart,
        description: "One sub-table per name, keyed by its label: `[dns.records.arbor]` is \
                      `arbor.<zone>`. A-records only. The name service reads this table when it \
                      starts, so a change is served after `apply service-restart rivet-dnsd` or \
                      the next boot.",
        rule: None,
        prompt: "Add a name to the table?",
    },
];

/// The document's top-level keys — `ADR-007` §7.
///
/// **Ordered to match `cmd/rivet-provision`'s interview**, added 2026-09-13
/// (plan §5.2 item 1, `.rulings/` R-162): `hostname` (`Hostname`), `network`
/// (`Addressing`/`Address`/`Gateway`), `mode` (`Mode`), `fleet-role`
/// (`Fleet`), `timezone` (`Timezone`), `kiosk` (`Auto-run`). `version` stays
/// first — it precedes every bare key in the rendered TOML, `crate::serialise`
/// is why, and it is the one key every document has always required. Keys
/// with no question of their own follow, in their prior relative order.
/// `the_document_order_matches_the_interview_order` below pins this sequence
/// so a generated interview (plan §5.2 item 3) has something to check itself
/// against.
pub const DOCUMENT: &[Property] = &[
    Property {
        name: "version",
        requirement: Requirement::Always,
        kind: Kind::Integer {
            // Derived from the one constant this build reads and writes, never
            // written out as a literal: a schema announcing a version this
            // build does not accept is the drift `ADR-007` §3.2 versions the
            // schema to avoid.
            constant: Some(SCHEMA_VERSION.get() as i64),
        },
        takes_effect: TakesEffect::OnNextRestart,
        description: "Which version of this schema the document is written against. Required so \
                      that an older document is migrated explicitly rather than silently misread \
                      (ADR-007 §3.2).",
        rule: None,
        prompt: "Schema version (not asked; always the current one):",
    },
    // The seventh top-level key, 2026-08-26 — `PLAN.md` §39.5.1 and
    // `rivet_core::hostname`. It closes an absence recorded twice and acted on
    // neither time: `rivet_core::etc` has had a `/etc/hosts` row blocked on it
    // since 2026-08-23, and `PLAN.md` §23.2 asserted the key already existed.
    Property {
        name: "hostname",
        requirement: Requirement::Optional,
        kind: Kind::Text { one_of: &[] },
        // **`on-next-restart`, and it could be `immediately`.** `sethostname(2)`
        // takes effect the moment it returns — but `/etc/hosts` is projected
        // when the supervisor starts and is not re-projected by a reload, and
        // mdnsd would have to be told. Three readers of one name, two of which
        // wait for a boot, is `on-next-restart` for the name as a whole. Saying
        // `immediately` because one reader is fast would be the "applied when
        // it is not" state ADR-034 §5.3 exists to prevent.
        takes_effect: TakesEffect::OnNextRestart,
        description: "What this machine calls itself. ABSENT IS NOT NAMELESS: a document with \
                      no `hostname` derives one from something only this machine can say about \
                      itself, so an unconfigured board is still individually reachable rather \
                      than answering to the kernel default `(none)`. ONE NAME, THREE READERS \
                      — sethostname(2), /etc/hosts, and mdnsd's <hostname>.local (ADR-011).",
        rule: Some(rivet_core::hostname::Hostname::RULE),
        prompt: "The machine's name on the network:",
    },
    Property {
        name: "network",
        requirement: Requirement::Optional,
        kind: Kind::NamedTablesWithReserved { entry: INTERFACE, reserved: &RESOLVER_TABLE },
        takes_effect: TakesEffect::OnNextRestart,
        description: "One sub-table per interface name, PLUS the reserved sub-table `resolver`. \
                      `[network.resolver]` takes `nameservers` (at most three IPv4 \
                      addresses, asked in the order written, because musl reads no more) \
                      and `search` (domains appended to an unqualified name). Naming \
                      none projects NO /etc/resolv.conf at all, which is not the same \
                      as an empty one: musl falls back to 127.0.0.1, where nothing \
                      listens, so a lookup fails fast instead of answering from a file \
                      nobody configured. An interface name this machine does not have is \
                      NOT refused here — the kernel is the authority for which interfaces \
                      exist, and a document written for a machine that has not been built yet \
                      is the normal case (ADR-007 §7.3).",
        rule: None,
        prompt: "How is this machine addressed?",
    },
    // `mode` — the fourth top-level key, added 2026-09-13 (plan §5.2 item 2,
    // `.rulings/` R-162). See `crate::machine_mode` for why this composes
    // nothing else in this crate.
    Property {
        name: "mode",
        requirement: Requirement::Optional,
        kind: Kind::Text {
            one_of: crate::machine_mode::MachineMode::ALL,
        },
        // **`on-next-restart`, on `hostname`'s footing.** Nothing in this
        // crate re-reads it, and a reader that does (a portal, a generated
        // interview) reads the stored document at the moment it is asked —
        // there is no running consumer for a reload to notify.
        takes_effect: TakesEffect::OnNextRestart,
        description: "Whether this machine drives a screen. Recorded intent, not an enforced \
                      one: `[kiosk]`'s own presence already says whether an application takes \
                      the screen (ADR-004 §6), so a document naming `kiosk` here with no \
                      `[kiosk]` table, or `headless` with one, is accepted rather than refused \
                      — see `crate::machine_mode` for why this crate does not arbitrate the two.",
        rule: Some(crate::machine_mode::MachineMode::RULE),
        prompt: "Does this machine drive a screen?",
    },
    // `fleet-role` — the fifth top-level key, added 2026-09-13 (plan §5.2
    // item 2, `.rulings/` R-162). `crate::fleet_role`.
    Property {
        name: "fleet-role",
        requirement: Requirement::Optional,
        kind: Kind::Text {
            one_of: crate::fleet_role::FleetRole::ALL,
        },
        takes_effect: TakesEffect::OnNextRestart,
        description: "Which of SPEC-05 §5's two roles this machine is. A DEFAULT, NEVER AN \
                      ASSERTION: `server` or `member` supplies `0.0.0.0:7777` for \
                      `[control-plane] listen` and turns on `ADR-050` §5's service set — \
                      `rivet-fleetd` for a server, `rivet-proxyd` and `rivet-mdnsd` for either \
                      — but ONLY where the document says nothing explicit itself, on the \
                      identical footing SPEC-05 §5 already rules for the fleet register's own \
                      `fleet_role` field: \"the device wins.\" An explicit `[control-plane]` or \
                      `[services.<name>]` entry is read exactly as written, whatever role is \
                      named here, so this key can never contradict one an operator wrote by \
                      hand.",
        rule: Some(crate::fleet_role::FleetRole::RULE),
        prompt: "Which fleet role is this machine?",
    },
    // The twelfth top-level key, 2026-09-02 — `rivet_core::timezone`, and it
    // closes the LAST row of `rivet_core::etc`'s blocked-on-a-key table that a
    // schema key could close: *"what timezone this machine keeps → the document
    // (ADR-021) → no schema key yet"*, carried since 2026-08-23. `ADR-021` §9
    // is the ruling; `PLAN.md` §110.7 item 1 is what it unblocks.
    //
    // **Modelled on `hostname` above, deliberately**: one optional top-level
    // scalar carrying one fact. A `[time]` table would be a table with one key
    // in it, which is a table that grows a second key for the wrong reason —
    // and `rivet-timed`'s servers and poll interval already have a home in
    // `[services.rivet-timed]`.
    Property {
        name: "timezone",
        requirement: Requirement::Optional,
        kind: Kind::Text { one_of: &[] },
        // **`on-next-restart`, and here that is not a near miss the way it is
        // for `hostname`.** The name reaches /etc/localtime, which the
        // supervisor projects once when it starts and does not re-project on a
        // SIGHUP reload; and every reader of a timezone is a process that
        // already resolved it — musl caches the zone per process after the
        // first localtime() call, Go resolves time.Local once, and an
        // application drawing a clock read it at start-up. So even a
        // re-projection would leave every RUNNING program on the old zone.
        // Answering `immediately` because the file changed would be exactly
        // the applied-when-it-is-not state ADR-034 §5.3 exists to prevent.
        takes_effect: TakesEffect::OnNextRestart,
        description: "What local time this machine keeps. ABSENT IS UTC, and unlike `hostname` \
                      NOTHING IS DERIVED: a machine cannot say which zone it is in from \
                      anything it can read about itself, and UTC is what musl keeps with no \
                      /etc/localtime — so saying nothing leaves the clock exactly where every \
                      Rivet had it before this key existed. A NAME AND NOT AN OFFSET: the \
                      rules belong to the zone database on the image, which an A/B update \
                      replaces, because jurisdictions change daylight saving and a document \
                      that hard-coded a rule would be an hour wrong for a few weeks a year \
                      for ever. A well-formed name for a zone THIS IMAGE HAS NO FILE FOR is \
                      not refused here — which zones exist is a property of the image, not of \
                      the document (ADR-007 §7.3) — and the machine reports it when the \
                      projection cannot copy it, leaving the clock at UTC.",
        rule: Some(rivet_core::timezone::Timezone::RULE),
        prompt: "The IANA time zone this machine keeps, e.g. America/Chicago:",
    },
    // `[kiosk]` - which application holds the display. Added 2026-08-31 on the
    // maintainer's ruling (`.rulings/` R-321): several kiosk applications can be
    // installed and enabled, and only one of them is the auto-launch.
    Property {
        name: "kiosk",
        requirement: Requirement::Optional,
        kind: Kind::Table { properties: KIOSK },
        takes_effect: TakesEffect::OnNextRestart,
        description: "Which of the offered applications gets the screen. THIS TABLE CHANGED \
                      WHAT `enabled` MEANS for an application that declares a display: it \
                      now means OFFERED - on the launcher's list - rather than RUNNING. \
                      Several may be enabled at once, which is the ordinary case; exactly one \
                      is named here and exactly one runs. ABSENT MEANS NO APPLICATION TAKES \
                      THE SCREEN, which is what every machine before this table did.",
        rule: None,
        prompt: "Configure which application takes the screen?",
    },
    Property {
        name: "services",
        requirement: Requirement::Optional,
        kind: Kind::NamedTables { entry: SERVICE },
        takes_effect: TakesEffect::OnNextRestart,
        description: "One sub-table per service name, holding what the operator wants of it. \
                      What a service *is* — its program, its restart policy, how readiness is \
                      known — ships inside the signed artifact that provides it and is refused \
                      here (ADR-017 §4.1).",
        rule: None,
        prompt: "Configure any services individually?",
    },
    Property {
        name: "volumes",
        requirement: Requirement::Optional,
        kind: Kind::NamedTables { entry: VOLUME },
        takes_effect: TakesEffect::OnNextRestart,
        description: "One sub-table per volume name: storage other than the built-in /state \
                      that applications may be installed on. Absent, and a `[volumes]` table \
                      declaring none, are the same answer — a machine asking for nothing \
                      beyond what it always has. A GUID this machine cannot find is NOT \
                      refused here: which devices are attached is the kernel's answer at the \
                      moment it is asked, not a property of the document (ADR-031 §4.2).",
        rule: None,
        prompt: "Attach any storage volumes?",
    },
    // `[repos]` - ruled 2026-08-26.
    Property {
        name: "repos",
        requirement: Requirement::Optional,
        kind: Kind::NamedTables { entry: REPO },
        takes_effect: TakesEffect::OnNextRestart,
        description: "Which git repositories this machine serves. Declaring one CREATES it \
                      (`git init --bare`, converging); renaming the table with the same `id` \
                      MOVES it. UNDECLARING ONE DELETES NOTHING - it is left where it is and \
                      reported, because the difference between removing a line from a file and \
                      destroying the only copy of a year's work must not be a diff nobody read. \
                      Destroying one is `apply repo-delete`, a separate confirmed operation \
                      (ADR-012 7.1) which requires that it be UNDECLARED first and that its \
                      identifier be typed. \
                      THERE IS NO PER-REPOSITORY ACCESS CONTROL: anything that can reach this \
                      machine's repositories can reach every repository here.",
        rule: None,
        prompt: "Serve any git repositories from this machine?",
    },
    // `[control-plane]` - ADR-035 §7.2, added 2026-08-28. The address moved
    // here from cmd/rivet-sysd's MCPD_LISTEN the day TLS made a non-loopback
    // bind legal, which that constant's own documentation said it would.
    Property {
        name: "control-plane",
        requirement: Requirement::Optional,
        kind: Kind::Table {
            properties: CONTROL_PLANE,
        },
        takes_effect: TakesEffect::OnNextRestart,
        description: "Where this machine's MCP control plane listens (ADR-035 §7.2). \
                      ABSENT IS NOT `off`: a document with no `[control-plane]` table gets \
                      the loopback default, which is the address the build constant this \
                      key replaced had. WHETHER it runs at all is \
                      `[services.rivet-mcpd]`'s `enabled` - where it listens and whether \
                      it listens are different questions and are not collapsed.",
        rule: None,
        prompt: "Override where the control plane listens?",
    },
    // `[splash]` - what the boot screen may say. Added 2026-08-30 on the
    // maintainer's requirement (`.rulings/` R-343): a third line of text that
    // says where to reach the machine's admin interface, with an option to
    // suppress it, or the version, for security reasons.
    Property {
        name: "splash",
        requirement: Requirement::Optional,
        kind: Kind::Table { properties: SPLASH },
        takes_effect: TakesEffect::OnNextRestart,
        description: "What the boot screen is allowed to say about this machine. ABSENT IS \
                      NOT `quiet`: a document with no `[splash]` table draws both the version \
                      and the endpoint, because a headless board whose address came from DHCP \
                      is otherwise a machine nobody can find without a router's lease table. \
                      Suppressing a line is the deliberate act and appears here where it can \
                      be reviewed.",
        rule: None,
        prompt: "Change what the boot screen shows?",
    },
    // `[display]` - the hours the screen is lit. Added 2026-09-02 on the
    // maintainer's ruling (`ADR-039`, `.rulings/` R-295), from the case that
    // forced it: a client who wants all twenty information displays in a
    // building dark from 7pm, to save power, and lit again from 7am without
    // anyone touching them.
    //
    // **The MACHINE's and not the application's**, which is the whole of
    // `ADR-039` §1.2: the person who wants the building dark is facilities, and
    // a schedule owned by whatever happens to be drawing is lost when the
    // application is swapped, absent while it is restarting, and absent again
    // whenever the machine has fallen back to the launcher.
    Property {
        name: "display",
        requirement: Requirement::Optional,
        kind: Kind::Table { properties: DISPLAY },
        // **`on-next-restart`, and it is the PREDICATE that is continuous
        // rather than the delivery.** `rivet_core::display::Hours::is_on` is
        // re-evaluated every pass, so once a reader has the window it needs no
        // prompting - but the window reaches a reader the way `timezone` does,
        // through a projection made when the supervisor starts. Until something
        // re-projects on reload AND every reader re-reads, answering
        // `immediately` would be the applied-when-it-is-not state `ADR-034`
        // §5.3 exists to prevent. `ADR-039` §11 item 5 carries the gap.
        takes_effect: TakesEffect::OnNextRestart,
        description: "The hours this machine's screen is lit. ABSENT IS ALWAYS ON, which is \
                      what every Rivet did before this table existed, and nothing is derived: \
                      a machine cannot infer a building's hours from anything it can read \
                      about itself. BOTH `off` AND `on` OR NEITHER. The window is in LOCAL \
                      time and MAY CROSS MIDNIGHT. IT IS THE MACHINE'S AND NOT THE \
                      APPLICATION'S: it applies to the launcher and to whichever kiosk \
                      application holds the display, so swapping the application does not \
                      change the policy, and an application that is crashed or restarting \
                      cannot leave a building lit all night.",
        rule: None,
        prompt: "Set hours the screen switches off?",
    },
    // `[applications.<name>]` - what each application is set to. Added
    // 2026-09-02, `ADR-038` §4.
    //
    // **The one key in this schema whose contents this crate does not
    // describe**, and `ADR-038` §4.2 calls it the decision in that document
    // most likely to be wrong. It is bounded three ways: confined to one
    // namespace and never top-level, checked against the installed
    // application's declaration ON THE MACHINE, and - §4.4 - never fatal.
    Property {
        name: "applications",
        requirement: Requirement::Optional,
        kind: Kind::NamedTablesOfStrings,
        // **`on-next-restart`, and honestly rather than hopefully.** `ADR-038`
        // §10.4 is OPEN on whether a changed setting restarts the application
        // or is re-read by it, and recommends *the application says which, and
        // the default is restart*. Until that is built, restart is what
        // happens, so restart is what this says - answering `immediately`
        // would be the applied-when-it-is-not state `ADR-034` §5.3 prevents.
        takes_effect: TakesEffect::OnNextRestart,
        description: "What each application is set to. THE KEYS ARE NOT THIS SCHEMA'S: they \
                      come from the `settings` list in the application's own manifest, inside \
                      its signed .rvt, and are checked against it ON THE MACHINE at reload. A \
                      key this machine cannot match to an installed declaration is REPORTED \
                      AND IGNORED, never fatal - the one place in this document where an \
                      unrecognised key does not refuse the whole file, because refusing to \
                      boot over a misspelled brightness value would be a catastrophic answer \
                      to a trivial mistake. VALUES ARE TEXT, for a first version. A table for \
                      an application this machine does not have is KEPT, because provisioning \
                      a fleet before the software is installed is the case this region exists \
                      for.",
        rule: None,
        prompt: "Set any application-specific values?",
    },
    // `[carry]` - which published version of a carried product this card
    // must carry. Added 2026-09-13, `.rulings/` R-159 R20.
    Property {
        name: "carry",
        requirement: Requirement::Optional,
        kind: Kind::NamedTables { entry: CARRY },
        // The table's own declaration is on `repos`/`services`'s footing:
        // what actually departs from `on-next-restart` is the one key inside
        // it, named in [`CARRY`].
        takes_effect: TakesEffect::OnNextRestart,
        description: "One sub-table per product name, pinning the version `cmd/rivet-image \
                      --fleet-repository` must resolve for it out of the fleet repository \
                      (ADR-050 §5). A product named here that this machine's role does not \
                      carry (`services.<name>.enabled`) pins nothing and is not an error.",
        rule: None,
        prompt: "Pin any carried product's version?",
    },
    // `[dns]` - the name service's table, `ADR-047` §8.
    Property {
        name: "dns",
        requirement: Requirement::Optional,
        kind: Kind::Table { properties: DNS },
        takes_effect: TakesEffect::OnNextRestart,
        description: "The names this machine answers for when `rivet-dnsd` runs, which is only \
                      where `[services.rivet-dnsd] enabled = true`. Every other name is \
                      forwarded upstream (ADR-047 §3). ABSENT MEANS NO ZONE: an enabled name \
                      service with no `[dns]` table refuses to start.",
        rule: None,
        prompt: "Configure the names this machine answers for?",
    },
];

/// **The JSON Schema of the configuration document.**
///
/// A [`Value`] rather than a string, because `ADR-006` §4 makes the structure
/// primary and the two renderings readers of it — a schema emitted as text
/// would be a second producer, and the human rendering would have nothing to
/// read from.
pub fn schema() -> Value {
    // The dialect, the title and the honesty clause come first, so that a
    // reader opening the document sees what it is before it sees what is in it.
    let mut fields = vec![
        ("$schema", Value::str(DIALECT)),
        (
            "title",
            Value::str(format!(
                "Rivet configuration document, schema version {SCHEMA_VERSION}"
            )),
        ),
        (
            "description",
            Value::str(format!(
                "The single desired-state document (ADR-007). Generated from rivet-config's \
                 types; never hand-maintained. {NOT_THE_WHOLE_OF_VALIDITY} {NOTHING_RE_READS}"
            )),
        ),
    ];
    fields.extend(table(DOCUMENT));
    Value::Object(fields)
}

/// **R-168.** Overlay the zone names a specific machine's own image can
/// resolve onto `timezone`'s `enum`, so a form built from the result offers a
/// dropdown instead of free text.
///
/// **Pure, on purpose.** `rivet-config` performs no I/O (`ADR-014` §3), so
/// this never reads `rivet_core::etc::ZONEINFO_DIR` itself — the caller does
/// that (`cmd/rivet`'s `PiMachine::configuration_schema`, the composition
/// root that already has both a real filesystem and this crate) and hands the
/// names over. `timezone`'s own declaration carries `Kind::Text { one_of: &[]
/// }` precisely so `property` emits no `enum` for it unadorned — this
/// function is the one place that ever changes, and it changes a [`Value`]
/// already built rather than reaching back into [`Property`]'s `'static`
/// slices, which cannot hold a list read off a filesystem at request time.
///
/// **`zones` empty leaves the schema exactly as [`schema`] built it** — a
/// free-text field, with `timezone`'s own description still explaining the
/// IANA-name rule. That is the honest answer when a machine's own database
/// cannot be read: a crash would be worse than the free-text field this build
/// already offered before R-168, and a wrong guess would be worse than
/// either (`AGENTS.md` §5).
#[must_use]
pub fn with_timezone_zones(schema: Value, zones: &[String]) -> Value {
    if zones.is_empty() {
        return schema;
    }
    overlay_property(schema, "properties", |properties| {
        overlay_property(properties, "timezone", |timezone| match timezone {
            Value::Object(mut fields) => {
                fields.retain(|(k, _)| *k != "enum");
                fields.push(("enum", Value::strings(zones.iter().cloned())));
                Value::Object(fields)
            }
            other => other,
        })
    })
}

/// Replace the value at `key` in an object with what `f` returns, leaving
/// every other field and their order untouched. A value that is not an
/// object is handed back unchanged rather than panicking — the caller above
/// only ever applies this to a schema this same module just built, but a
/// silent no-op is the honest answer if that ever stops being true, on the
/// same footing every other reader in this crate refuses rather than guesses.
fn overlay_property(value: Value, key: &str, f: impl FnOnce(Value) -> Value) -> Value {
    let Value::Object(fields) = value else {
        return value;
    };
    let mut out = Vec::with_capacity(fields.len());
    let mut f = Some(f);
    for (k, v) in fields {
        if k != key {
            out.push((k, v));
            continue;
        }
        match f.take() {
            Some(apply) => out.push((k, apply(v))),
            None => out.push((k, v)),
        }
    }
    Value::Object(out)
}

/// A table with a fixed set of keys, as the fields of a JSON Schema object.
///
/// `additionalProperties: false` because `ADR-007` §3.6.5 refuses an unknown key
/// rather than ignoring it, and a schema that permitted one would describe a
/// document the parser rejects.
///
/// Returns the fields rather than a [`Value`] so that a fixed sub-table can be
/// spliced into a property's own schema without unwrapping an object that is
/// known to be one.
fn table(properties: &'static [Property]) -> Vec<(&'static str, Value)> {
    let required: Vec<&str> = properties
        .iter()
        .filter(|p| p.requirement.unconditional())
        .map(|p| p.name)
        .collect();

    let mut fields = vec![
        ("type", Value::str("object")),
        ("additionalProperties", Value::Bool(false)),
    ];
    // Omitted when empty rather than emitted as `[]`: `"required": []` is a
    // keyword that says nothing.
    if !required.is_empty() {
        fields.push(("required", Value::strings(required)));
    }
    fields.push((
        "properties",
        Value::Object(properties.iter().map(|p| (p.name, property(p))).collect()),
    ));
    // Conditional rules, expressed structurally — see `Conditional`. Omitted
    // when empty rather than emitted as `[]`, on `required`'s footing above.
    let conditionals = conditionals_for(properties);
    if !conditionals.is_empty() {
        fields.push((
            "allOf",
            Value::List(conditionals.iter().map(conditional_schema).collect()),
        ));
    }
    fields
}

/// One property's sub-schema.
fn property(p: &Property) -> Value {
    let mut fields = match p.kind {
        Kind::Integer { constant } => {
            let mut f = vec![("type", Value::str("integer"))];
            if let Some(value) = constant {
                f.push(("const", Value::Integer(value)));
            }
            f
        }
        Kind::Boolean => vec![("type", Value::str("boolean"))],
        Kind::Text { one_of } => {
            let mut f = vec![("type", Value::str("string"))];
            if !one_of.is_empty() {
                f.push(("enum", Value::strings(one_of.iter().copied())));
            }
            f
        }
        Kind::TextArray => vec![
            ("type", Value::str("array")),
            (
                "items",
                Value::object(vec![("type", Value::str("string"))]),
            ),
        ],
        Kind::NamedTables { entry } => vec![
            ("type", Value::str("object")),
            // The operator names these keys, so there is no `properties` list
            // to write: every key is valid and every *value* is an entry.
            ("additionalProperties", Value::Object(table(entry))),
        ],
        Kind::NamedTablesOfStrings => vec![
            ("type", Value::str("object")),
            // Two levels of keys neither of which this crate names, so
            // neither has a `&'static [Property]` to close a `properties`
            // list with — the operator names the outer one and an installed
            // application's own manifest names the inner one. Every value is
            // a plain string, on `read_applications`' own footing.
            (
                "additionalProperties",
                Value::object(vec![
                    ("type", Value::str("object")),
                    (
                        "additionalProperties",
                        Value::object(vec![("type", Value::str("string"))]),
                    ),
                ]),
            ),
        ],
        Kind::NamedTablesWithReserved { entry, reserved } => vec![
            ("type", Value::str("object")),
            // The one reserved name publishes its own schema; every other
            // key the operator writes still falls to `additionalProperties`,
            // on plain `NamedTables`' footing above.
            (
                "properties",
                Value::object(vec![(reserved.name, property(reserved))]),
            ),
            ("additionalProperties", Value::Object(table(entry))),
        ],
        Kind::Table { properties } => table(properties),
    };
    fields.push(("description", Value::str(describe(p))));
    // **Distinct from `description`, added 2026-09-13** (plan §5.2 item 1,
    // `.rulings/` R-162) — see `Property::prompt`.
    fields.push(("prompt", Value::str(p.prompt)));
    fields.extend(when(p.takes_effect));
    Value::Object(fields)
}

/// **`ADR-034` §5.3's answer, as schema keywords.**
///
/// `takes-effect` and `requires-reboot` are **extension keywords** — JSON Schema
/// ignores keywords it does not know, which is what makes this legal, and the
/// module documentation already says nothing here validates the emitted document
/// as JSON Schema.
///
/// **They are spelled exactly as a plan spells them**
/// (`rivet_core::operation::plan::TakesEffect::value`), rather than given an
/// `x-`prefixed name of their own. §5.3's whole design is one fact reaching
/// three consumers; two spellings of it — one for a plan and one for the schema
/// — would be `ADR-011` §3 with the halves a crate apart, and an agent would
/// have to know both.
///
/// `requires-reboot` is **omitted** rather than emitted `false` when this build
/// cannot assert it, which is `crate::value`'s standing rule and is the point of
/// [`TakesEffect::requires_reboot`] returning an `Option`: a consumer that read
/// a missing key as `false` would have reintroduced the omission §5.3 forbids.
fn when(takes_effect: TakesEffect) -> Vec<(&'static str, Value)> {
    let mut fields = vec![("takes-effect", Value::str(takes_effect.as_str()))];
    if let Some(needed) = takes_effect.requires_reboot() {
        fields.push(("requires-reboot", Value::Bool(needed)));
    }
    if let TakesEffect::NotAsserted { because } = takes_effect {
        fields.push(("because", Value::str(because)));
    }
    fields
}

/// A property's description: what it means, then the rule its value obeys —
/// the same sentence a refusal quotes.
fn describe(p: &Property) -> String {
    let mut text = p.description.to_owned();
    if let Requirement::Conditional(condition) = p.requirement {
        text.push_str(&format!(" This key is {condition}."));
    }
    if let Some(rule) = p.rule {
        text.push_str(&format!(" Rule: {rule}."));
    }
    text
}

/// Every property in the schema, paired with the dotted path it sits at.
///
/// `*` stands for a key the operator names. Used by this module's tests to
/// prove their fixture table covers the whole schema, and by anything else that
/// wants to walk it.
pub fn paths() -> Vec<(String, &'static Property)> {
    fn walk(prefix: &str, properties: &'static [Property], out: &mut Vec<(String, &'static Property)>) {
        for p in properties {
            let path = if prefix.is_empty() {
                p.name.to_owned()
            } else {
                format!("{prefix}.{}", p.name)
            };
            match p.kind {
                Kind::NamedTables { entry } => {
                    out.push((path.clone(), p));
                    walk(&format!("{path}.*"), entry, out);
                }
                Kind::NamedTablesWithReserved { entry, reserved } => {
                    out.push((path.clone(), p));
                    let reserved_path = format!("{path}.{}", reserved.name);
                    out.push((reserved_path.clone(), reserved));
                    if let Kind::Table { properties } = reserved.kind {
                        walk(&reserved_path, properties, out);
                    }
                    walk(&format!("{path}.*"), entry, out);
                }
                Kind::Table { properties } => {
                    out.push((path.clone(), p));
                    walk(&path, properties, out);
                }
                _ => out.push((path, p)),
            }
        }
    }
    let mut out = Vec::new();
    walk("", DOCUMENT, &mut out);
    out
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::document::Document;
    use crate::network::Interface;
    use crate::services::Service;
    
    fn fields(value: &Value) -> &[(&'static str, Value)] {
        match value {
            Value::Object(fields) => fields,
            other => panic!("expected an object, got {other:?}"),
        }
    }

    fn get<'a>(value: &'a Value, key: &str) -> &'a Value {
        fields(value)
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v)
            .unwrap_or_else(|| panic!("no {key} in {value:?}"))
    }

    // ---- the prose an operator reads ------------------------------------------

    /// **No description or prompt carries a run of spaces**, which is what a
    /// lost `\` line continuation leaves behind: the newline goes and the next
    /// line's indentation stays as text. `tools/source/prose.py` scans for the
    /// same thing but only on a line that opens the literal, so a run on a
    /// continuation line is invisible to it; this walks the values instead.
    #[test]
    fn no_description_or_prompt_carries_a_run_of_spaces() {
        for (path, p) in paths() {
            for (what, text) in [("description", p.description), ("prompt", p.prompt)] {
                assert!(
                    !text.contains("      "),
                    "{path}'s {what} carries a run of spaces: {text}"
                );
            }
        }
    }

    // ---- the key names are the parser's own lists ---------------------------

    /// **The anti-drift property, and the only one that makes "generated from
    /// the types" true rather than claimed.** A key described here that the
    /// parser does not read, or read there and not described, is precisely the
    /// document `ADR-006` §7 says will be "trusted and wrong".
    #[test]
    fn every_tables_properties_are_exactly_its_declared_keys_in_order() {
        for (declared, described, what) in [
            (Document::DECLARED_KEYS, DOCUMENT, "the document"),
            (Service::DECLARED_KEYS, SERVICE, "a service"),
            (Interface::DECLARED_KEYS, INTERFACE, "an interface"),
            (crate::volumes::Volume::DECLARED_KEYS, VOLUME, "a volume"),
            (Resolver::DECLARED_KEYS, RESOLVER, "a resolver"),
            (crate::dns::Dns::DECLARED_KEYS, DNS, "the dns table"),
            (crate::dns::Record::DECLARED_KEYS, DNS_RECORD, "a dns record"),
        ] {
            let names: Vec<&str> = described.iter().map(|p| p.name).collect();
            assert_eq!(names, declared, "{what}");
        }
    }

    /// **The pin plan §5.2 item 1 asks for**: `DOCUMENT`'s declaration order,
    /// stated explicitly, so a generated interview (item 3) has something to
    /// generate against rather than a comment. Matches
    /// `cmd/rivet-provision`'s own question order — `Hostname`, `Addressing`
    /// (`network`), `Mode`, `Fleet` (`fleet-role`), `Timezone`, `Auto-run`
    /// (`kiosk`) — with `version` first (never asked; always the current
    /// schema version) and every key with no question of its own following.
    /// Break this by moving one entry and watch it fail (`AGENTS.md` §4).
    #[test]
    fn the_document_order_matches_the_interview_order() {
        let names: Vec<&str> = DOCUMENT.iter().map(|p| p.name).collect();
        assert_eq!(
            names,
            [
                "version", "hostname", "network", "mode", "fleet-role", "timezone", "kiosk",
                "services", "volumes", "repos", "control-plane", "splash", "display",
                "applications", "carry", "dns",
            ]
        );
    }

    /// **Item 1's third requirement**: a prompt an interview can ask with,
    /// distinct from the long description a reader consults afterwards. Every
    /// property, at every level — a table included, since a generated
    /// interview asking whether to configure `[kiosk]` at all needs a
    /// sentence too.
    #[test]
    fn every_property_has_a_prompt_distinct_from_its_description() {
        for (path, property) in paths() {
            assert!(!property.prompt.is_empty(), "{path} has no prompt");
            assert_ne!(
                property.prompt, property.description,
                "{path}'s prompt is just its description repeated"
            );
        }
    }

    /// The vocabulary of a closed-set key is the parser's, not a copy of it.
    #[test]
    fn the_enumerated_values_are_the_ones_the_parser_accepts() {
        let method = INTERFACE
            .iter()
            .find(|p| p.name == "method")
            .expect("declared above");
        assert_eq!(method.kind, Kind::Text { one_of: Method::ALL });
        for spelling in Method::ALL {
            assert!(Method::parse(spelling).is_some(), "{spelling} does not parse");
        }
    }

    /// The version the schema publishes is the version this build reads. A
    /// literal here could name a version nothing accepts.
    #[test]
    fn the_schema_pins_the_version_this_build_actually_reads() {
        let schema = schema();
        let version = get(get(&schema, "properties"), "version");
        assert_eq!(
            get(version, "const"),
            &Value::Integer(i64::from(SCHEMA_VERSION.get()))
        );
        // And the parser refuses every other one.
        let wrong = SCHEMA_VERSION.get() + 1;
        assert!(Document::parse(&format!("version = {wrong}\n")).is_err());
    }

    // ---- required-ness and type, checked against the parser -----------------

    /// One property, one fixture: a document that sets it, a document that does
    /// not, and a document that sets it to the wrong kind of value.
    struct Fixture {
        path: &'static str,
        with: &'static str,
        without: &'static str,
        mistyped: &'static str,
    }

    // **Ordered to match `paths()`, which is `DOCUMENT`'s own declaration
    // order (`there_is_a_fixture_for_every_property_the_schema_publishes`
    // asserts the two lists equal, entry for entry) — and `DOCUMENT`'s order
    // was itself reordered 2026-09-13 to match `cmd/rivet-provision`'s
    // interview (plan §5.2 item 1, `.rulings/` R-162).** See
    // `the_document_order_matches_the_interview_order` below for the pinned
    // top-level sequence this produces.
    const FIXTURES: &[Fixture] = &[
        Fixture {
            path: "version",
            with: "version = 1\n",
            without: "",
            mistyped: "version = \"1\"\n",
        },
        Fixture {
            path: "hostname",
            with: "version = 1\nhostname = \"kitchen-pi\"\n",
            without: "version = 1\n",
            mistyped: "version = 1\nhostname = 7\n",
        },
        Fixture {
            path: "network",
            with: "version = 1\n[network.eth0]\nmethod = \"dhcp\"\n",
            without: "version = 1\n",
            mistyped: "version = 1\nnetwork = \"eth0\"\n",
        },
        Fixture {
            path: "network.resolver",
            with: "version = 1\n[network.resolver]\nnameservers = [\"192.0.2.53\"]\n",
            without: "version = 1\n",
            mistyped: "version = 1\n[network]\nresolver = \"192.0.2.53\"\n",
        },
        Fixture {
            path: "network.resolver.nameservers",
            with: "version = 1\n[network.resolver]\nnameservers = [\"192.0.2.53\"]\n",
            without: "version = 1\n[network.resolver]\nsearch = [\"example.test\"]\n",
            mistyped: "version = 1\n[network.resolver]\nnameservers = \"192.0.2.53\"\n",
        },
        Fixture {
            path: "network.resolver.search",
            with: "version = 1\n[network.resolver]\nsearch = [\"example.test\"]\n",
            without: "version = 1\n[network.resolver]\nnameservers = [\"192.0.2.53\"]\n",
            mistyped: "version = 1\n[network.resolver]\nsearch = \"example.test\"\n",
        },
        Fixture {
            path: "network.*.method",
            with: "version = 1\n[network.eth0]\nmethod = \"dhcp\"\n",
            without: "version = 1\n[network.eth0]\naddress = \"10.0.0.2/24\"\n",
            mistyped: "version = 1\n[network.eth0]\nmethod = 1\n",
        },
        Fixture {
            path: "network.*.address",
            with: "version = 1\n[network.eth0]\nmethod = \"static\"\naddress = \"10.0.0.2/24\"\n",
            // Absent under `dhcp`, which is a valid document — the conditional
            // requirement is what `Requirement::Conditional` publishes.
            without: "version = 1\n[network.eth0]\nmethod = \"dhcp\"\n",
            mistyped: "version = 1\n[network.eth0]\nmethod = \"static\"\naddress = 10\n",
        },
        Fixture {
            path: "network.*.gateway",
            with: "version = 1\n[network.eth0]\nmethod = \"static\"\naddress = \"10.0.0.2/24\"\n\
                   gateway = \"10.0.0.1\"\n",
            without: "version = 1\n[network.eth0]\nmethod = \"static\"\naddress = \"10.0.0.2/24\"\n",
            mistyped: "version = 1\n[network.eth0]\nmethod = \"static\"\naddress = \"10.0.0.2/24\"\n\
                       gateway = 10\n",
        },
        Fixture {
            path: "network.*.ssid",
            with: "version = 1\n[network.wlan0]\nmethod = \"dhcp\"\n\
                   ssid = \"an-example-network\"\n",
            without: "version = 1\n[network.wlan0]\nmethod = \"dhcp\"\n",
            mistyped: "version = 1\n[network.wlan0]\nmethod = \"dhcp\"\n\
                       ssid = 42\n",
        },
        // `mode` and `fleet-role` — plan §5.2 item 2, `.rulings/` R-162.
        Fixture {
            path: "mode",
            with: "version = 1\nmode = \"kiosk\"\n",
            without: "version = 1\n",
            mistyped: "version = 1\nmode = 7\n",
        },
        Fixture {
            path: "fleet-role",
            with: "version = 1\nfleet-role = \"server\"\n",
            without: "version = 1\n",
            mistyped: "version = 1\nfleet-role = 7\n",
        },
        Fixture {
            path: "timezone",
            with: "version = 1\ntimezone = \"America/Chicago\"\n",
            without: "version = 1\n",
            mistyped: "version = 1\ntimezone = 7\n",
        },
        // `[kiosk]` - which application holds the display. Added 2026-08-31.
        //
        // `mistyped` is a BARE key rather than a wrong value inside the table,
        // matching `control-plane` below: the table itself has to be a table,
        // and the parser's refusal for that is the one this proves.
        Fixture {
            path: "kiosk",
            with: "version = 1\n[kiosk]\nauto-run = \"kiosk-demo\"\n",
            without: "version = 1\n",
            mistyped: "version = 1\nkiosk = \"kiosk-demo\"\n",
        },
        Fixture {
            path: "kiosk.auto-run",
            with: "version = 1\n[kiosk]\nauto-run = \"kiosk-demo\"\n",
            without: "version = 1\n[kiosk]\n",
            mistyped: "version = 1\n[kiosk]\nauto-run = 7\n",
        },
        // `locked` - PLAN.md 108.2. `without` carries `auto-run` rather
        // than being an empty table, so what is proved absent is THIS key
        // and not the table around it.
        Fixture {
            path: "kiosk.locked",
            with: "version = 1\n[kiosk]\nauto-run = \"kiosk-demo\"\nlocked = true\n",
            without: "version = 1\n[kiosk]\nauto-run = \"kiosk-demo\"\n",
            mistyped: "version = 1\n[kiosk]\nlocked = \"yes\"\n",
        },
        Fixture {
            path: "kiosk.launcher",
            with: "version = 1\n[kiosk]\nauto-run = \"kiosk-demo\"\nlauncher = \"my-launcher\"\n",
            without: "version = 1\n[kiosk]\nauto-run = \"kiosk-demo\"\n",
            mistyped: "version = 1\n[kiosk]\nlauncher = 7\n",
        },
        Fixture {
            path: "kiosk.compositor",
            with: "version = 1\n[kiosk]\nauto-run = \"kiosk-demo\"\ncompositor = \"cosmic-comp\"\n",
            without: "version = 1\n[kiosk]\nauto-run = \"kiosk-demo\"\n",
            mistyped: "version = 1\n[kiosk]\ncompositor = true\n",
        },
        Fixture {
            path: "services",
            with: "version = 1\n[services.rivet-apid]\nenabled = true\n",
            without: "version = 1\n",
            mistyped: "version = 1\nservices = 3\n",
        },
        Fixture {
            path: "services.*.enabled",
            with: "version = 1\n[services.rivet-apid]\nenabled = true\n",
            without: "version = 1\n[services.rivet-apid]\nmemory = \"1M\"\n",
            mistyped: "version = 1\n[services.rivet-apid]\nenabled = \"yes\"\n",
        },
        Fixture {
            path: "services.*.memory",
            with: "version = 1\n[services.rivet-apid]\nenabled = true\nmemory = \"128M\"\n",
            without: "version = 1\n[services.rivet-apid]\nenabled = true\n",
            mistyped: "version = 1\n[services.rivet-apid]\nenabled = true\nmemory = 128\n",
        },
        Fixture {
            path: "services.*.cpu",
            with: "version = 1\n[services.rivet-apid]\nenabled = true\ncpu = \"0.5\"\n",
            without: "version = 1\n[services.rivet-apid]\nenabled = true\n",
            mistyped: "version = 1\n[services.rivet-apid]\nenabled = true\ncpu = 1\n",
        },
        Fixture {
            path: "services.*.critical",
            with: "version = 1\n[services.rivet-apid]\nenabled = true\ncritical = true\n",
            without: "version = 1\n[services.rivet-apid]\nenabled = true\n",
            mistyped: "version = 1\n[services.rivet-apid]\nenabled = true\ncritical = \"yes\"\n",
        },
        Fixture {
            path: "volumes",
            with: "version = 1\n[volumes.bulk]\nguid = \"1a2b3c4d-5e6f-4071-8293-a4b5c6d7e8f9\"\n",
            without: "version = 1\n",
            mistyped: "version = 1\nvolumes = \"bulk\"\n",
        },
        Fixture {
            path: "volumes.*.guid",
            with: "version = 1\n[volumes.bulk]\nguid = \"1a2b3c4d-5e6f-4071-8293-a4b5c6d7e8f9\"\n",
            without: "version = 1\n[volumes.bulk]\n",
            mistyped: "version = 1\n[volumes.bulk]\nguid = 4772\n",
        },
        Fixture {
            path: "repos",
            with: "version = 1\n[repos.example-repo]\nid = \"4f2c9a01bd7e3865\"\n",
            without: "version = 1\n",
            mistyped: "version = 1\nrepos = \"example-repo\"\n",
        },
        Fixture {
            path: "repos.*.id",
            with: "version = 1\n[repos.example-repo]\nid = \"4f2c9a01bd7e3865\"\n",
            without: "version = 1\n[repos.example-repo]\n",
            mistyped: "version = 1\n[repos.example-repo]\nid = 7\n",
        },
        Fixture {
            path: "control-plane",
            with: "version = 1
[control-plane]
listen = \"127.0.0.1:7777\"
",
            without: "version = 1
",
            mistyped: "version = 1
control-plane = \"127.0.0.1:7777\"
",
        },
        Fixture {
            path: "control-plane.listen",
            with: "version = 1
[control-plane]
listen = \"127.0.0.1:7777\"
",
            without: "version = 1
[control-plane]
",
            mistyped: "version = 1
[control-plane]
listen = 7777
",
        },
        Fixture {
            path: "splash",
            with: "version = 1
[splash]
endpoint = false
",
            without: "version = 1
",
            mistyped: "version = 1
splash = false
",
        },
        Fixture {
            path: "splash.endpoint",
            with: "version = 1
[splash]
endpoint = false
",
            without: "version = 1
[splash]
version = false
",
            mistyped: "version = 1
[splash]
endpoint = \"no\"
",
        },
        Fixture {
            path: "splash.version",
            with: "version = 1
[splash]
version = false
",
            without: "version = 1
[splash]
endpoint = false
",
            mistyped: "version = 1
[splash]
version = 3
",
        },
        Fixture {
            path: "display",
            with: "version = 1
[display]
off = \"19:00\"
on = \"07:00\"
",
            without: "version = 1
",
            mistyped: "version = 1
display = \"19:00\"
",
        },
        Fixture {
            path: "display.off",
            with: "version = 1
[display]
off = \"19:00\"
on = \"07:00\"
",
            // The table present with the other half only, which the parser
            // refuses by name - `off` and `on` are one statement.
            without: "version = 1
[display]
on = \"07:00\"
",
            mistyped: "version = 1
[display]
off = 7
on = \"07:00\"
",
        },
        Fixture {
            path: "display.on",
            with: "version = 1
[display]
off = \"19:00\"
on = \"07:00\"
",
            without: "version = 1
[display]
off = \"19:00\"
",
            mistyped: "version = 1
[display]
off = \"19:00\"
on = 7
",
        },
        Fixture {
            path: "applications",
            with: "version = 1\n[applications.aura-renderer]\nbrightness = \"60\"\n",
            without: "version = 1\n",
            mistyped: "version = 1\napplications = \"aura-renderer\"\n",
        },
        Fixture {
            path: "carry",
            with: "version = 1\n[carry.rivet-fleetd]\nversion = \"0.1.0\"\n",
            without: "version = 1\n",
            mistyped: "version = 1\ncarry = \"rivet-fleetd\"\n",
        },
        Fixture {
            path: "carry.*.version",
            with: "version = 1\n[carry.rivet-fleetd]\nversion = \"0.1.0\"\n",
            without: "version = 1\n[carry.rivet-fleetd]\n",
            mistyped: "version = 1\n[carry.rivet-fleetd]\nversion = 7\n",
        },
        // `[dns]` - `ADR-047` §8.
        Fixture {
            path: "dns",
            with: "version = 1\n[dns]\nzone = \"f.home.arpa\"\n",
            without: "version = 1\n",
            mistyped: "version = 1\ndns = \"f.home.arpa\"\n",
        },
        Fixture {
            path: "dns.zone",
            with: "version = 1\n[dns]\nzone = \"f.home.arpa\"\n",
            without: "version = 1\n[dns]\nttl = 300\n",
            mistyped: "version = 1\n[dns]\nzone = 7\n",
        },
        Fixture {
            path: "dns.ttl",
            with: "version = 1\n[dns]\nzone = \"f.home.arpa\"\nttl = 300\n",
            without: "version = 1\n[dns]\nzone = \"f.home.arpa\"\n",
            mistyped: "version = 1\n[dns]\nzone = \"f.home.arpa\"\nttl = \"300\"\n",
        },
        Fixture {
            path: "dns.negative-ttl",
            with: "version = 1\n[dns]\nzone = \"f.home.arpa\"\nnegative-ttl = 60\n",
            without: "version = 1\n[dns]\nzone = \"f.home.arpa\"\n",
            mistyped: "version = 1\n[dns]\nzone = \"f.home.arpa\"\nnegative-ttl = \"60\"\n",
        },
        Fixture {
            path: "dns.records",
            with: "version = 1\n[dns]\nzone = \"f.home.arpa\"\n[dns.records.a]\naddress = \"10.0.0.1\"\n",
            without: "version = 1\n[dns]\nzone = \"f.home.arpa\"\n",
            mistyped: "version = 1\n[dns]\nzone = \"f.home.arpa\"\nrecords = \"a\"\n",
        },
        Fixture {
            path: "dns.records.*.address",
            with: "version = 1\n[dns]\nzone = \"f.home.arpa\"\n[dns.records.a]\naddress = \"10.0.0.1\"\n",
            without: "version = 1\n[dns]\nzone = \"f.home.arpa\"\n[dns.records.a]\nttl = 60\n",
            mistyped: "version = 1\n[dns]\nzone = \"f.home.arpa\"\n[dns.records.a]\naddress = 10\n",
        },
        Fixture {
            path: "dns.records.*.ttl",
            with: "version = 1\n[dns]\nzone = \"f.home.arpa\"\n[dns.records.a]\naddress = \"10.0.0.1\"\nttl = 60\n",
            without: "version = 1\n[dns]\nzone = \"f.home.arpa\"\n[dns.records.a]\naddress = \"10.0.0.1\"\n",
            mistyped: "version = 1\n[dns]\nzone = \"f.home.arpa\"\n[dns.records.a]\naddress = \"10.0.0.1\"\nttl = \"60\"\n",
        },
    ];

    fn fixture(path: &str) -> &'static Fixture {
        FIXTURES
            .iter()
            .find(|f| f.path == path)
            .unwrap_or_else(|| panic!("no fixture for {path}"))
    }

    /// The coverage guard. A property added to the schema with no fixture is a
    /// property whose published required-ness and type nothing has checked
    /// against the parser — which is how a generated document starts lying.
    #[test]
    fn there_is_a_fixture_for_every_property_the_schema_publishes() {
        let declared: Vec<String> = paths().into_iter().map(|(path, _)| path).collect();
        let covered: Vec<String> = FIXTURES.iter().map(|f| f.path.to_owned()).collect();
        assert_eq!(covered, declared);
    }

    /// Every fixture's `with` document must actually parse, or the two tests
    /// below are comparing against nonsense.
    #[test]
    fn every_fixture_that_sets_its_key_is_a_document_the_parser_accepts() {
        for f in FIXTURES {
            assert!(
                Document::parse(f.with).is_ok(),
                "{}: {:?} does not parse: {:?}",
                f.path,
                f.with,
                Document::parse(f.with).err()
            );
        }
    }

    /// **The published `required` list is the parser's behaviour, not a claim
    /// about it.** `Always` must be refused when absent; `Optional` and
    /// `Conditional` must be accepted, because JSON Schema's `required` array
    /// cannot carry a condition and publishing one there would refuse valid
    /// documents.
    #[test]
    fn the_parser_agrees_with_every_published_requirement() {
        for (path, property) in paths() {
            let f = fixture(&path);
            let got = Document::parse(f.without);
            match property.requirement {
                Requirement::Always => assert!(
                    got.is_err(),
                    "{path} is published as required and a document without it parsed"
                ),
                Requirement::Optional | Requirement::Conditional(_) => assert!(
                    got.is_ok(),
                    "{path} is not published as unconditionally required, and a document \
                     without it was refused: {:?}",
                    got.err()
                ),
            }
        }
    }

    /// And the published type is the parser's too: a value of the wrong kind is
    /// refused, with the key named.
    #[test]
    fn the_parser_refuses_a_value_of_a_kind_the_schema_does_not_publish() {
        for (path, _) in paths() {
            let f = fixture(&path);
            let leaf = path.rsplit('.').next().unwrap_or(&path);
            let err = Document::parse(f.mistyped)
                .expect_err(&format!("{path}: {:?} should not parse", f.mistyped));
            assert!(
                err.to_string().contains(leaf),
                "{path}: the refusal does not name {leaf}: {err}"
            );
        }
    }

    // ---- the emitted document ------------------------------------------------

    #[test]
    fn the_root_is_a_closed_object_naming_its_dialect_and_its_required_keys() {
        let schema = schema();
        assert_eq!(get(&schema, "$schema"), &Value::str(DIALECT));
        assert_eq!(get(&schema, "type"), &Value::str("object"));
        assert_eq!(get(&schema, "additionalProperties"), &Value::Bool(false));
        assert_eq!(get(&schema, "required"), &Value::strings(["version"]));
    }

    /// `ADR-007` §3.6.5 refuses an unknown key rather than ignoring it, at
    /// every level. A schema that allowed one anywhere would describe a
    /// document the machine rejects.
    ///
    /// **One shape is not "ignoring a key": `properties` naming a reserved
    /// child beside an `additionalProperties` that is a schema of its own.**
    /// `network` is the one table like this — `resolver` is named, and every
    /// other key an operator writes still lands on the interface schema,
    /// which this same walk visits and still requires to be closed in its
    /// own right. Nothing there goes unvalidated; only a bare `properties`
    /// with no `additionalProperties` schema behind it — `false`, or absent
    /// entirely, which JSON Schema also reads as "anything goes" — would
    /// mean a whole table was left open.
    #[test]
    fn every_table_in_the_schema_is_closed_because_the_parser_refuses_a_stray_key() {
        fn walk(value: &Value, where_: &str) {
            let f = fields(value);
            if f.iter().any(|(k, _)| *k == "properties") {
                match f.iter().find(|(k, _)| *k == "additionalProperties").map(|(_, v)| v) {
                    // Closed: nothing beyond `properties` is valid.
                    Some(Value::Bool(false)) => {}
                    // Open only to a schema of its own — walked below, and
                    // required there to be closed wherever IT enumerates a
                    // fixed set of keys.
                    Some(Value::Object(_)) => {}
                    other => panic!("{where_} accepts an unknown key: {other:?}"),
                }
            }
            for (key, child) in f {
                if matches!(child, Value::Object(_)) {
                    walk(child, &format!("{where_}/{key}"));
                }
            }
        }
        walk(&schema(), "");
        // And the parser really does refuse one, at each level.
        for src in [
            "version = 1\nnope = 1\n",
            "version = 1\n[services.a]\nenabled = true\nnope = 1\n",
            "version = 1\n[network.eth0]\nmethod = \"dhcp\"\nnope = 1\n",
            "version = 1\n[network.resolver]\nnope = 1\n",
            "version = 1\n[volumes.media]\nguid = \"6f2c9a01-bd7e-4865-9a1b-2c3d4e5f6071\"\nnope = 1\n",
        ] {
            assert!(Document::parse(src).is_err(), "{src:?} was accepted");
        }
    }

    /// Every key the schema publishes carries a description, and where the
    /// value has a rule the description is **the parser's own sentence** — so an
    /// operator reading the schema and an operator reading a refusal are told
    /// the same thing.
    #[test]
    fn every_property_describes_itself_using_the_rule_a_refusal_would_quote() {
        for (path, property) in paths() {
            let text = describe(property);
            assert!(!text.is_empty(), "{path} has no description");
            if let Some(rule) = property.rule {
                assert!(text.contains(rule), "{path} does not carry its own rule");
            }
            if let Requirement::Conditional(condition) = property.requirement {
                assert!(text.contains(condition), "{path} hides its condition");
            }
        }
    }

    /// **Every setting says when a change to it takes effect.** `ADR-034` §5.3,
    /// which rules that *"a setting that does not take effect immediately must
    /// say so at the moment it is changed"* and puts the answer in the schema
    /// rather than in a message.
    ///
    /// A property with no answer cannot exist — [`TakesEffect`] has no absent
    /// state and the field is not optional — so what this asserts is the part a
    /// type cannot: that the answer is **published**, and that it is the answer
    /// this build's behaviour actually justifies.
    #[test]
    fn every_setting_publishes_when_a_change_to_it_takes_effect() {
        // ~~Nothing re-reads the document while it is running — see
        // NOTHING_RE_READS, which is the one place that reason is written.
        // **This assertion is the one that fails when SIGHUP lands**
        // (`ADR-032` §3.2), which is the point: the settings it moves to
        // `immediately` must be moved here in the same change, not discovered
        // later by an operator whose service did not restart.~~
        //
        // **It fired on 2026-08-26, the same day it was written, and it did its
        // job**: `SIGHUP` landed and this test refused the change until
        // `services.*.enabled` said something true. Kept in its new shape rather
        // than deleted, because the next thing to move a setting — the query
        // socket, an `apply` that writes the document — must clear it too.
        //
        // The list is written out rather than derived. A derived expectation
        // would agree with whatever the table said and assert nothing, which is
        // the one thing a contract test must not do.
        let expected: &[(&str, TakesEffect)] = &[
            (
                "services.*.enabled",
                TakesEffect::NotAsserted { because: "" },
            ),
            // **`immediately`, and it is asserted rather than hedged** — added
            // 2026-09-08 with `ADR-045` §4. A `SIGHUP` reload rewrites every
            // definition's `critical` in `cmd/rivet-sysd/src/document.rs`, and
            // there is nothing for it to withhold: the key starts nothing and
            // stops nothing, so `enabled`'s reachability carve-out — the whole
            // reason that one cannot answer — does not arise. The machine's
            // health is folded fresh on every `get status`, so the next answer
            // after the reload already uses it.
            ("services.*.critical", TakesEffect::Immediately),
            // Added 2026-09-13, `.rulings/` R-159 R20 — see `CARRY`'s own
            // documentation for why this key answers `not-asserted` too, for
            // a reason unrelated to `services.*.enabled`'s.
            (
                "carry.*.version",
                TakesEffect::NotAsserted { because: "" },
            ),
        ];
        for (path, property) in paths() {
            let want = expected
                .iter()
                .find(|(p, _)| *p == path)
                .map_or(TakesEffect::OnNextRestart, |(_, t)| *t);
            match (want, property.takes_effect) {
                // Compared by STATE rather than by value, because the reason is
                // prose and pinning it here would make this a second copy of it.
                (TakesEffect::NotAsserted { .. }, TakesEffect::NotAsserted { because }) => {
                    assert!(
                        !because.is_empty(),
                        "{path} answers `not-asserted` and gives no reason, which is a shrug \
                         rather than an answer (ADR-034 §5.3)"
                    );
                }
                (a, b) => assert_eq!(
                    a, b,
                    "{path} claims an answer this build does not justify"
                ),
            }
            // And whatever the answer, `requires-reboot` is present when it can
            // be asserted and ABSENT when it cannot — never `false` by default.
            let emitted = when(property.takes_effect);
            let published = emitted.iter().any(|(k, _)| *k == "requires-reboot");
            assert_eq!(
                published,
                property.takes_effect.requires_reboot().is_some(),
                "{path} publishes requires-reboot inconsistently with what it can assert"
            );
        }
    }

    /// And the root says **why**, once, rather than once per key.
    ///
    /// ~~`NOTHING RE-READS THIS DOCUMENT`~~ — the sentence this asserted was
    /// true for two days. `SIGHUP` landed 2026-08-26 and the root now has to
    /// carry **both** halves: the default, and the key that departs from it.
    /// A test that had only been loosened would have stopped noticing the
    /// difference, so it asserts each exception by name.
    ///
    /// **A second exception joined 2026-09-13** (`.rulings/` R-159 R20),
    /// unrelated to `SIGHUP`: `carry.*.version` is `not-asserted` because
    /// nothing running ever reads it at all, not because a reload treats it
    /// specially.
    #[test]
    fn the_schema_says_when_a_change_takes_effect_and_names_every_exception() {
        let root = schema();
        let Value::Str(description) = get(&root, "description") else {
            panic!("the root must describe itself");
        };
        // The three readers, named rather than gestured at, because "changes
        // what the next boot does" is only actionable if you know who read it.
        for reader in ["pid 1", "the supervisor", "rivet-netd"] {
            assert!(description.contains(reader), "{reader} is not named");
        }
        // The mechanism that makes the first exception an exception.
        assert!(description.contains("SIGHUP"), "{description}");
        // And that exception itself, both halves: the key it applies to, and the
        // two services it is withheld for. An operator who read only the first
        // would stop the wrong service and find it still running.
        assert!(description.contains("services.*.enabled"), "{description}");
        for withheld in ["rivet-mcpd", "rivet-netd"] {
            assert!(
                description.contains(withheld),
                "{withheld} is not named as withheld: {description}"
            );
        }
        // The second exception, named by itself rather than folded into the
        // first — the two answer `not-asserted` for different reasons.
        assert!(description.contains("carry.*.version"), "{description}");
    }

    /// The emitted property carries the keywords, spelled **exactly as a plan
    /// spells them** — `ADR-034` §5.3's one fact, three consumers.
    #[test]
    fn the_emitted_schema_and_a_plan_spell_the_answer_the_same_way() {
        let fields = |p: &Property| match property(p) {
            Value::Object(fields) => fields,
            other => panic!("a property is an object, got {other:?}"),
        };

        // `memory`: an answer this build CAN assert, so both keys appear.
        let memory = fields(&SERVICE[1]);
        assert_eq!(SERVICE[1].name, "memory");
        assert!(
            memory
                .iter()
                .any(|(k, v)| *k == "takes-effect" && *v == Value::str("on-next-restart")),
            "{memory:?}"
        );
        assert!(
            memory
                .iter()
                .any(|(k, v)| *k == "requires-reboot" && *v == Value::Bool(true)),
            "{memory:?}"
        );

        // `enabled`: an answer it cannot, since 2026-08-26's reload. The key is
        // ABSENT and the reason is published in its place.
        let enabled = fields(&SERVICE[0]);
        assert_eq!(SERVICE[0].name, "enabled");
        assert!(
            enabled
                .iter()
                .any(|(k, v)| *k == "takes-effect" && *v == Value::str("not-asserted")),
            "{enabled:?}"
        );
        assert!(
            !enabled.iter().any(|(k, _)| *k == "requires-reboot"),
            "a property this build cannot place must OMIT the key, never answer false: {enabled:?}"
        );
        assert!(
            enabled.iter().any(|(k, _)| *k == "because"),
            "{enabled:?}"
        );

        // And every spelling comes from `rivet_core`'s own type, which is what
        // makes a plan and this schema unable to disagree — ADR-034 §5.3.
        assert_eq!(TakesEffect::OnNextRestart.as_str(), "on-next-restart");
        assert_eq!(TakesEffect::OnNextRestart.requires_reboot(), Some(true));
        assert_eq!(
            TakesEffect::NotAsserted { because: "" }.requires_reboot(),
            None
        );
    }

    /// `requires-reboot` is **omitted** rather than answered `false` when this
    /// build cannot assert it. A consumer reading a missing key as `false` would
    /// have reintroduced exactly the omission §5.3 forbids, which is why
    /// `TakesEffect::requires_reboot` returns an `Option`.
    #[test]
    fn a_setting_this_build_could_not_place_would_omit_the_field_rather_than_guess() {
        let emitted = when(TakesEffect::NotAsserted {
            because: "nothing in this build says what reads it",
        });
        assert_eq!(
            emitted,
            vec![
                ("takes-effect", Value::str("not-asserted")),
                (
                    "because",
                    Value::str("nothing in this build says what reads it")
                ),
            ]
        );
    }

    /// **The honesty clause.** Six of `rivet-config`'s refusals cannot be
    /// expressed in JSON Schema, and a document that stayed quiet about them
    /// would tell an agent its input was valid when the machine will refuse it.
    #[test]
    fn the_schema_says_that_passing_it_is_not_the_same_as_being_accepted() {
        let schema = schema();
        let Value::Str(description) = get(&schema, "description") else {
            panic!("the root must describe itself");
        };
        assert!(description.contains("NOT sufficient"), "{description}");
        assert!(description.contains("rivet-config is the authority"), "{description}");
        // Each of the four, named rather than gestured at.
        for refusal in [
            "subnet",
            "broadcast",
            "two interfaces",
            "dev_valid_name",
        ] {
            assert!(description.contains(refusal), "{refusal} is not named: {description}");
        }
    }

    /// And each of those four really is a refusal, rather than a sentence
    /// about one. Every document here is structurally valid against the
    /// emitted schema's `properties`/`type`/`required` and is refused by the
    /// parser anyway — which is the whole reason the clause above has to be
    /// there. **The dhcp+address contradiction is deliberately NOT one of
    /// these any more**: `the_interface_conditional_matches_what_the_parser_
    /// refuses` below proves the schema's own `if`/`then`/`else` catches it
    /// structurally instead.
    #[test]
    fn each_refusal_the_schema_cannot_express_is_one_the_parser_performs() {
        for (why, src) in [
            (
                "a gateway off-link",
                "version = 1\n[network.eth0]\nmethod = \"static\"\naddress = \"10.0.0.2/24\"\n\
                 gateway = \"192.168.1.1\"\n"
                    .to_owned(),
            ),
            (
                "a broadcast address",
                "version = 1\n[network.eth0]\nmethod = \"static\"\naddress = \"10.0.0.255/24\"\n"
                    .to_owned(),
            ),
            (
                "one address on two interfaces",
                "version = 1\n[network.eth0]\nmethod = \"static\"\naddress = \"10.0.0.2/24\"\n\
                 [network.eth1]\nmethod = \"static\"\naddress = \"10.0.0.2/16\"\n"
                    .to_owned(),
            ),
            (
                "a name the kernel would refuse",
                "version = 1\n[network.\"eth/0\"]\nmethod = \"dhcp\"\n".to_owned(),
            ),
        ] {
            assert!(
                Document::parse(&src).is_err(),
                "{why} was accepted, so the schema's honesty clause is describing a refusal \
                 that does not happen"
            );
        }
    }

    /// Rendering is a total function of the types and of nothing else — no
    /// clock, no map iteration order. `ADR-007` §3.1's property, applied to the
    /// artifact that describes the document rather than to the document.
    #[test]
    fn the_schema_is_the_same_value_every_time_it_is_generated() {
        assert_eq!(schema(), schema());
    }

    // ---- conditional rules, plan §5.2 item 1 ---------------------------------

    /// `network.<name>`'s `properties` object carries the `if`/`then`/`else`
    /// `crate::network` argues for: required under `static`, refused under
    /// `dhcp`, never resolved by precedence — `ADR-007` §7.
    #[test]
    fn the_interface_table_publishes_its_conditional_as_if_then_else() {
        let entry = schema_for_named_table("network");
        let all_of = get(&entry, "allOf");
        let Value::List(blocks) = all_of else {
            panic!("network's conditional must be a list of if/then/else blocks: {all_of:?}");
        };
        assert_eq!(blocks.len(), 1, "network has exactly one conditional group: {blocks:?}");
        let block = &blocks[0];
        let if_ = get(block, "if");
        // `if`: method must be present and equal to "static".
        let method_const = get(get(if_, "properties"), "method");
        assert_eq!(get(method_const, "const"), &Value::str("static"));
        assert_eq!(get(if_, "required"), &Value::strings(["method"]));
        // `then`: address is required.
        assert_eq!(get(get(block, "then"), "required"), &Value::strings(["address"]));
        // `else`: neither address nor gateway may be present.
        let else_ = get(block, "else");
        let not = get(else_, "not");
        let Value::List(any_of) = get(not, "anyOf") else {
            panic!("the else branch must forbid a list of keys: {not:?}");
        };
        let forbidden: Vec<&Value> = any_of
            .iter()
            .map(|v| get(v, "required"))
            .collect();
        assert!(forbidden.contains(&&Value::strings(["address"])), "{any_of:?}");
        assert!(forbidden.contains(&&Value::strings(["gateway"])), "{any_of:?}");
    }

    /// **Every table with no conditional rule publishes no `allOf` at all** —
    /// `services`, `volumes`, `[control-plane]` and the rest. `"required": []`
    /// is a keyword that says nothing; an empty `allOf` would be the same
    /// mistake for a different keyword.
    #[test]
    fn a_table_with_no_conditional_rule_publishes_no_all_of() {
        for name in ["services", "volumes", "control-plane", "splash", "kiosk", "display"] {
            let entry = if DOCUMENT
                .iter()
                .find(|p| p.name == name)
                .is_some_and(|p| matches!(p.kind, Kind::NamedTables { .. }))
            {
                schema_for_named_table(name)
            } else {
                schema_for_table(name)
            };
            assert!(
                !fields(&entry).iter().any(|(k, _)| *k == "allOf"),
                "{name} publishes a conditional it does not have: {entry:?}"
            );
        }
    }

    /// **The mechanism agrees with the parser it republishes** — proven
    /// against the exact document `crate::network`'s own header calls out as
    /// the sharp edge: `method = "dhcp"` beside a static `address`.
    #[test]
    fn the_interface_conditional_matches_what_the_parser_refuses() {
        // `if` triggers: method is static, so `then` applies and address is
        // required.
        let static_no_address = "version = 1\n[network.eth0]\nmethod = \"static\"\n";
        assert!(Document::parse(static_no_address).is_err(), "address must be required under static");

        // `if` does not trigger: method is dhcp, so `else` applies and address
        // is forbidden outright — this is `ADR-007` §7's own sharp edge.
        let dhcp_with_address =
            "version = 1\n[network.eth0]\nmethod = \"dhcp\"\naddress = \"10.0.0.2/24\"\n";
        assert!(Document::parse(dhcp_with_address).is_err(), "address must be forbidden under dhcp");

        let dhcp_with_gateway =
            "version = 1\n[network.eth0]\nmethod = \"dhcp\"\ngateway = \"10.0.0.1\"\n";
        assert!(Document::parse(dhcp_with_gateway).is_err(), "gateway must be forbidden under dhcp");

        // And the accepted shape on each side of the condition.
        assert!(Document::parse("version = 1\n[network.eth0]\nmethod = \"dhcp\"\n").is_ok());
        assert!(
            Document::parse(
                "version = 1\n[network.eth0]\nmethod = \"static\"\naddress = \"10.0.0.2/24\"\n"
            )
            .is_ok()
        );
    }

    /// The `network.<name>` entry schema — inside `properties.network.
    /// additionalProperties`, which is where a `NamedTables` entry's own
    /// `properties`/`allOf` live (`Kind::NamedTables`'s comment in `property`).
    fn schema_for_named_table(name: &str) -> Value {
        let root = schema();
        let prop = get(get(&root, "properties"), name);
        get(prop, "additionalProperties").clone()
    }

    /// A fixed `Kind::Table`'s own schema, e.g. `[kiosk]` — its
    /// `properties`/`allOf` sit directly on the property object.
    fn schema_for_table(name: &str) -> Value {
        let root = schema();
        get(get(&root, "properties"), name).clone()
    }

    // ---- application settings are text, and are not a closed table ----------

    /// **One application's own table accepts any setting name, string
    /// values only** — the exact shape `read_applications` keeps, published
    /// rather than contradicted by an empty closed table.
    #[test]
    fn each_applications_entry_accepts_only_string_valued_settings() {
        let entry = schema_for_named_table("applications");
        assert_eq!(
            entry,
            Value::object(vec![
                ("type", Value::str("object")),
                (
                    "additionalProperties",
                    Value::object(vec![("type", Value::str("string"))]),
                ),
            ])
        );
    }

    /// The parser's own behaviour, which the schema above now matches
    /// instead of contradicting: multiple application names, arbitrary
    /// setting names, and an empty settings table are all kept.
    #[test]
    fn the_parser_keeps_arbitrary_application_settings_the_schema_now_matches() {
        for src in [
            "version = 1\n[applications.aura-renderer]\nbrightness = \"60\"\n",
            "version = 1\n[applications.aura-renderer]\nbrightness = \"60\"\n\
             [applications.other-app]\nvolume = \"11\"\n",
            "version = 1\n[applications.aura-renderer]\n",
        ] {
            assert!(
                Document::parse(src).is_ok(),
                "{src:?}: {:?}",
                Document::parse(src).err()
            );
        }
    }

    // ---- the resolver is not an interface ------------------------------------

    /// **`network.resolver` publishes its own two keys, closed to anything
    /// else, and is not put through the interface's `required: [method]`.**
    /// Every other key an operator writes still falls to the interface
    /// schema, `required` intact.
    #[test]
    fn network_publishes_resolver_as_its_own_property_not_an_interface() {
        let network = schema_for_table("network");
        let resolver = get(get(&network, "properties"), "resolver");

        let resolver_properties = get(resolver, "properties");
        assert!(fields(resolver_properties).iter().any(|(k, _)| *k == "nameservers"));
        assert!(fields(resolver_properties).iter().any(|(k, _)| *k == "search"));
        assert_eq!(get(resolver, "additionalProperties"), &Value::Bool(false));
        assert!(
            !fields(resolver).iter().any(|(k, _)| *k == "required"),
            "resolver has no key the interface's `required: [method]` would wrongly demand: \
             {resolver:?}"
        );

        // Every other key the operator writes is still an interface, with
        // `required: [method]` exactly as before.
        let interface_entry = schema_for_named_table("network");
        assert_eq!(get(&interface_entry, "required"), &Value::strings(["method"]));
    }

    /// The parser's own behaviour, which the schema above now matches: every
    /// shape the resolver table accepts, and the interface rule it does not
    /// borrow.
    #[test]
    fn the_parser_accepts_every_shape_the_resolver_schema_now_describes() {
        for src in [
            // Both keys together — the published fixture.
            "version = 1\n[network.resolver]\nnameservers = [\"192.0.2.53\"]\n\
             search = [\"example.test\"]\n",
            // Nameservers only.
            "version = 1\n[network.resolver]\nnameservers = [\"192.0.2.53\"]\n",
            // Search only.
            "version = 1\n[network.resolver]\nsearch = [\"example.test\"]\n",
            // Empty — states nothing, which is what lets a lease answer instead.
            "version = 1\n[network.resolver]\n",
            // Beside an ordinary interface.
            "version = 1\n[network.resolver]\nnameservers = [\"192.0.2.53\"]\n\
             [network.eth0]\nmethod = \"static\"\naddress = \"10.0.0.2/24\"\n",
        ] {
            assert!(
                Document::parse(src).is_ok(),
                "{src:?}: {:?}",
                Document::parse(src).err()
            );
        }

        // `method` is an interface key, not a resolver key.
        assert!(Document::parse("version = 1\n[network.resolver]\nmethod = \"dhcp\"\n").is_err());
        // Ordinary interface validation is untouched: `static` still needs an
        // address.
        assert!(Document::parse("version = 1\n[network.eth0]\nmethod = \"static\"\n").is_err());
        // An unknown resolver key is still refused by name.
        assert!(Document::parse("version = 1\n[network.resolver]\nnope = 1\n").is_err());
    }

    /// A published path is one an operator can write. `*` is this module's
    /// notation for a key the operator names, and the rest must be real keys.
    #[test]
    fn the_published_paths_are_the_documents_own_shape() {
        let paths: Vec<String> = paths().into_iter().map(|(p, _)| p).collect();
        assert!(paths.contains(&"version".to_owned()));
        assert!(paths.contains(&"services.*.enabled".to_owned()));
        assert!(paths.contains(&"network.*.method".to_owned()));
        assert!(paths.contains(&"control-plane.listen".to_owned()));
        // **Every published path is one the parser accepts**, which is the half
        // that is easy to lose: a schema advertising a table this build has
        // removed would be telling an operator to write a document the parser
        // refuses, and nothing else would notice.
        for path in &paths {
            assert!(
                Document::DECLARED_KEYS.contains(&path.split('.').next().unwrap_or_default()),
                "the schema publishes {path:?}, which is not a key the document declares"
            );
        }
    }

    // ---- R-168: `with_timezone_zones` --------------------------------------

    /// **An empty list is a no-op.** A machine whose zone database could not
    /// be read must answer exactly what [`schema`] alone would have — a
    /// free-text field, never a crash and never a guessed list.
    #[test]
    fn an_empty_zone_list_leaves_the_schema_unchanged() {
        assert_eq!(with_timezone_zones(schema(), &[]), schema());
    }

    /// **The zones handed over become exactly `timezone`'s `enum`.**
    #[test]
    fn the_given_zones_become_the_timezone_enum() {
        let zones = vec!["America/Chicago".to_owned(), "UTC".to_owned()];
        let overlaid = with_timezone_zones(schema(), &zones);
        let timezone = get(get(&overlaid, "properties"), "timezone");
        assert_eq!(
            get(timezone, "enum"),
            &Value::strings(["America/Chicago", "UTC"])
        );
    }

    /// **Nothing else about the property is disturbed** — its description,
    /// its `takes-effect`, its own rule sentence all survive, because an
    /// operator reading the form still needs them.
    #[test]
    fn overlaying_zones_touches_only_the_enum() {
        let plain = get(get(&schema(), "properties"), "timezone").clone();
        let zones = vec!["UTC".to_owned()];
        let overlaid = with_timezone_zones(schema(), &zones);
        let with_zones = get(get(&overlaid, "properties"), "timezone");
        assert_eq!(get(with_zones, "type"), get(&plain, "type"));
        assert_eq!(get(with_zones, "description"), get(&plain, "description"));
        assert_eq!(get(with_zones, "takes-effect"), get(&plain, "takes-effect"));
    }

    /// **Every other property is untouched, in the same order.** A function
    /// that overlays one property must not reorder or drop its neighbours —
    /// `ADR-015` §4 wants the same inputs to produce the same bytes, and a
    /// reordering would be exactly the kind of drift that breaks.
    #[test]
    fn overlaying_zones_leaves_every_other_property_exactly_as_it_was() {
        let plain = schema();
        let overlaid = with_timezone_zones(schema(), &["UTC".to_owned()]);
        let plain_props = fields(get(&plain, "properties"));
        let overlaid_props = fields(get(&overlaid, "properties"));
        assert_eq!(
            plain_props.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            overlaid_props.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
        );
        for (key, value) in plain_props {
            if *key == "timezone" {
                continue;
            }
            let (_, other) = overlaid_props
                .iter()
                .find(|(k, _)| k == key)
                .unwrap_or_else(|| panic!("{key} vanished"));
            assert_eq!(value, other, "{key} changed and was not the one overlaid");
        }
    }

    /// **Calling it twice does not accumulate a second `enum`.** A caller
    /// that re-read the database on some later request and overlaid again —
    /// which nothing does today, but nothing here should assume it never will
    /// — must still get one `enum`, not two competing ones.
    #[test]
    fn overlaying_twice_replaces_rather_than_duplicates() {
        let once = with_timezone_zones(schema(), &["UTC".to_owned()]);
        let twice = with_timezone_zones(once, &["America/Chicago".to_owned()]);
        let timezone = get(get(&twice, "properties"), "timezone");
        assert_eq!(get(timezone, "enum"), &Value::strings(["America/Chicago"]));
    }
}
