//! The services table: **what the operator asks of a service, never what the
//! service is.**
//!
//! `ADR-017` §4.1 ruled the split this module exists to enforce, and its
//! consequences list names the thing built here — *"`ADR-007`'s schema gains a
//! services table carrying enable/disable and per-service settings — the
//! operator's whole surface."*
//!
//! | Question | Authority |
//! | :-- | :-- |
//! | *What is `rivet-netd`?* — program, restart policy, how readiness is known, **which sockets it listens on** | the **definition**: the image for a system service, the signed manifest for an application (`ADR-015` §3.2) |
//! | *Should `rivet-netd` run? Under what limit?* | **this table** — `ADR-007` desired state |
//!
//! # The table lost a key on 2026-08-24, and gained a reason
//!
//! ~~`enabled` required; `listen` and `memory` optional.~~ **Superseded
//! 2026-08-24** by the maintainer's split-by-direction ruling
//! (`ADR-007` §7.2, `ADR-015` §3.2). The struck line is left readable
//! because it is what the record said for two days and a reader coming from
//! `ADR-007` §7.2's older text needs to find the correction here.
//!
//! The conflict it resolves was real rather than a wording problem: `ADR-007`
//! §7.2 put `listen` and `memory` in **this** table, `ADR-015` §3.2 put the same
//! two keys in the **signed manifest**, both ADRs were accepted, and `ADR-011`
//! says both cannot be the authority. The ruling:
//!
//! > **The manifest owns what the application *is*** — restart policy,
//! > readiness contract, the sockets it listens on. **The document owns what
//! > the operator *limits*** — the memory cap. **Where both name `memory`, the
//! > lower wins.**
//!
//! So the operator's surface is now **`enabled` and `memory`**, and `listen`
//! joins [`Service::DEFINITION_KEYS`] — refused with an explanation rather than
//! as an unknown key, because an operator who reaches for it is following two
//! accepted ADRs and a bare "unknown key" would leave them stuck.
//!
//! # `cpu` joined the same way, 2026-09-09, and this module does not re-argue it
//!
//! `ADR-007` §7 and `ADR-015` §3 gave CPU the identical split, ruled by
//! the identical argument: the manifest's `cpu` is a **request**, this
//! table's is a **cap**, and [`rivet_core::service::effective_cpu`] takes the
//! lower exactly as [`rivet_core::service::effective_memory`] does. There was
//! never a `listen`-shaped conflict to resolve for `cpu` — no ADR ever put it
//! in the manifest alone — so nothing here is a correction the way `listen`'s
//! departure was; `cpu` simply arrived where `memory` already stood.
//!
//! **The direction is the test.** `ADR-015` §4.1 protects an application's
//! *behaviour* from being rewritten outside its signature; a memory cap runs the
//! other way — the machine's owner protecting the machine **from** the
//! application — so `ADR-007` §7.2 is right to grant it and nothing is weakened
//! by it living out here.
//!
//! # Where the line is, and the test that draws it
//!
//! "Enabled, settings, limits" (`ADR-017` §4) is a description, not a rule, and
//! a description will not survive the next agent who wants somewhere convenient
//! to put a field. The rule this module applies instead, and the reason each
//! clause is there:
//!
//! > **A key belongs in this table only if all three hold.**
//! >
//! > 1. **Changing it cannot change which code runs.** `ADR-015` §4.1 exists so
//! >    that a signed artifact's behaviour cannot be rewritten outside the
//! >    signature. An operator who could set `exec` would be redefining what a
//! >    signed application executes, which destroys that guarantee outright.
//! >    `ADR-015` §3.2 makes the same point from the other side: there is
//! >    deliberately no `exec` field *anywhere*, because the artifact is a Cargo
//! >    package and the binary *is* the crate's binary.
//! > 2. **It is a policy the machine's owner is entitled to set, not a fact the
//! >    artifact's author asserted.** A memory cap is a policy: the owner of the
//! >    Pi decides how much of it any one thing may have. A restart policy is a
//! >    fact about the program — whether a clean exit means "finished" or
//! >    "crashed" is knowable only to whoever wrote it — which is why `ADR-017`
//! >    §4.1 put `restart` and `ready` inside the signature rather than here.
//! > 3. **Rivet enforces it from outside the service.** A memory cap is the
//! >    operator's because Rivet writes it to a cgroup (`ADR-015` §5.1); the
//! >    service does not read it and cannot refuse it. An `exec` path fails this
//! >    test as well as the first two: nothing outside the process can enforce
//! >    which program it already is.
//!
//! **Clause 3 used to admit `listen` and the 2026-08-24 ruling overrules it.**
//! That is worth stating precisely rather than quietly dropping, because clause
//! 3 was called "the sharp test" here and it gave the wrong answer: Rivet does
//! open the socket and pass the descriptor (`ADR-015` §5.2), so a port passes
//! clause 3 — and it fails **clause 2**, which is the one that decides. *Which*
//! sockets a program is served on is a fact its author asserted about the
//! program, not a policy of the machine's owner; a program handed a descriptor
//! for a port its author never declared has had its behaviour changed from
//! outside the signature. Three clauses, all of which must hold, and `listen`
//! holds only two.
//!
//! # What is deliberately absent
//!
//! * **`exec`, `args`, `env`, `user`** — clause 1. See
//!   [`Service::DEFINITION_KEYS`]; these are refused with an explanation rather
//!   than as merely unknown keys, because an operator who reaches for them has
//!   a wrong model and a bare "unknown key" would not correct it.
//! * **`listen`** — clause 2, ruled 2026-08-24. In the manifest
//!   (`rivet_core::package::application::Manifest`), inside the signature.
//! * **`restart`** — `ADR-017` §3.5's three values live in the manifest,
//!   inside the signature (`ADR-015` §4.1).
//! * **`ready`** — and this is the question `ADR-017` §4.2 raises, answered
//!   here rather than left ambiguous. **The three readiness values do not
//!   interact with this table at all.** `none`, `notify` and `listening`
//!   describe *how a program signals that it is serving*: `notify` requires the
//!   program to call `sd_notify`, `listening` requires it to accept on a
//!   pre-created socket, `none` says it does neither. An operator who could
//!   write `ready = "notify"` for a program that never calls it would
//!   make it permanently not-ready; one who could write `ready = "none"` for a
//!   service that does notify would make Rivet report it ready before it was
//!   serving, which is exactly the dishonesty `ADR-017` §3.3 exists to prevent.
//!   Both are facts about the program, so both stay in the definition.
//! * **`network`, `state`, `provides`, `connects`** — these are permission
//!   *requests* in `ADR-015` §3.2, and `ADR-015` §3.3 makes the manifest the
//!   request while the operator does the granting through `plan`/`apply`.
//!   **Where a grant is recorded is not decided anywhere in the record**, so
//!   nothing is invented here: they fall through as ordinary unknown keys and
//!   are listed in the report as needing a ruling.
//!
//! # Absence is only allowed to mean something when it means exactly one thing
//!
//! That rule decides which keys are required, and it is `ADR-007` §3.6.5 —
//! *Rivet does not accept an input it intends to discard* — read the other way
//! round.
//!
//! * **The whole `services` table may be absent**, and means one thing: the
//!   operator has configured no service. Unambiguous, so it is allowed.
//! * **`enabled` is required in every service that appears.** It has no
//!   counterpart in the definition — checked: `ADR-015` §3.2's manifest lists
//!   `kind`, `listen`, `network`, `state`, `provides`, `connects`, `memory`,
//!   `restart` and `ready`, and no `enabled` — so an absent `enabled` is not
//!   "inherit the artifact's value", it is *nobody said*. `ADR-017` §6 makes
//!   this key the entire control surface ("to stop a service, set
//!   `enabled = false` and apply"), so a document that names a service without
//!   saying whether it should run has not stated desired state, which is the
//!   document's only job (`ADR-007` §3.3).
//! * **`memory` is optional**, and absent means one thing: *the operator has
//!   capped nothing, so the artifact's own request stands.* That is a fact about
//!   the document rather than a guess, because the key **does** have a
//!   counterpart in the manifest — and since 2026-08-24 the two meeting is a
//!   ruled outcome rather than an open question. `rivet_core::service::
//!   effective_memory` is the rule: **the lower wins.**
//! * **`cpu` is optional on `memory`'s footing exactly**, since 2026-09-09.
//!   Absent means *the operator has capped nothing, so the artifact's own
//!   request stands* — the identical sentence, about a different resource —
//!   and `rivet_core::service::effective_cpu` is the identical rule.
//!   **There is deliberately no default**: a machine with one application
//!   should be able to give it the whole processor, and a manufactured
//!   default would change every already-provisioned machine's behaviour the
//!   moment it updated. Absent means `cpu.max` is left at `max`.
//!
//! ~~Note the distinction between an absent `listen` and an empty one.~~
//! **Removed with the key**, 2026-08-24. The three-way absence existed only
//! because two authorities could each state a value; with one authority there is
//! nothing for an omission to defer to.
//!
//! # What this module does not do: merge
//!
//! It holds what the *document* says and nothing else. `ADR-017` §4.1 gives the
//! merge to the supervisor — *"the supervisor sees definitions ∪ desired state
//! and converges"* — and doing it here would make `rivet-config` a second
//! authority over values the signed artifact owns, which is the `ADR-011`
//! failure this whole split was ruled to avoid.
//!
//! **What changed on 2026-08-24 is that the merge now has a rule to apply.**
//! This crate still does not perform it: `rivet_core::service::
//! effective_memory` states it, `cmd/rivet-sysd` performs it, and this
//! module reports what the operator wrote and nothing else.
//!
//! # Unknown *service names* are accepted; unusable ones are refused
//!
//! `AGENTS.md` §3 says refuse rather than guess, and the interesting part is
//! that accepting an unrecognised name is not a guess.
//!
//! 1. **This crate cannot know what exists.** `ADR-014` §3.1 forbids it any
//!    I/O, so it can read neither `rivet-sysd`'s compiled-in table nor
//!    an installed application's manifest. The only way to refuse would be a
//!    hardcoded list here — which would be a second authority over what
//!    services exist, one ADR away from the authority `ADR-017` §4.1 just
//!    placed in the image and the signed manifest.
//! 2. **A document for a machine that does not yet have the service is the
//!    normal case, not an error.** `ADR-007` §3.6.3 makes the document an input
//!    artifact authored anywhere — a workstation, a git repository, an agent's
//!    scratch space — and §3.4's fleet layer applies one document across
//!    several machines. Refusing here would make it impossible to write
//!    configuration for a service you are about to install.
//! 3. **The refusal still happens, one layer up, where the information is.**
//!    `apply` holds the document *and* the definitions, so it can say "you
//!    configured `rivet-netdd`; this machine has `rivet-netd`" — a diagnosis this
//!    crate could never produce. That is a real obligation on `apply` and it is
//!    named in the report.
//!
//! What *is* refused here is a name that could not be a service name on any
//! machine: [`ServiceName`] enforces it, because `ADR-017` §3.1.3 makes the
//! name a cgroup path component, `ADR-017` §3.4 makes it part of a notify
//! socket's identity, and `ADR-015` §7 makes it an installed application's
//! directory. That check needs no knowledge of the machine, so it belongs in the
//! domain — it moved to `rivet_core::service` on 2026-08-24, when the third of
//! those became true, and is re-exported here unchanged.
//!
//! # Secrecy
//!
//! Every refusal in this module carries a [`crate::Location`] and a rule, never
//! the text it rejected — `ADR-020` §4, since the provisioning file carries a
//! wireless passphrase. [`crate::ConfigError::InvalidKey`]'s explanatory field is
//! `&'static str` precisely so that it *cannot* carry input.

/// The name of a service, and how much memory it may have.
///
/// **Moved to `rivet-core` on 2026-08-24** and re-exported here, so that a
/// document, an installed application's manifest and the supervisor's merge all
/// speak one vocabulary. `rivet-config` depends on `rivet-core` and
/// `ADR-014` §3.2 makes that direction permanent, so the domain is the only
/// place all three can reach.
///
/// `Endpoint` and `Transport` were here too and are **gone rather than moved**:
/// with `listen` out of the document (see this module's header) the only thing
/// that names a socket is the manifest, and
/// `rivet_core::package::application::Socket` is its type.
pub use rivet_core::service::{CpuLimit, MemoryLimit, ServiceName};

/// What the operator asks of one service.
///
/// Read this module's documentation before adding a field. Every key here
/// satisfies all three clauses of the rule stated there, and the value of the
/// rule is entirely in it having been applied to the keys that are *not* here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    name: ServiceName,
    enabled: bool,
    memory: Option<MemoryLimit>,
    cpu: Option<CpuLimit>,
    critical: bool,
}

impl Service {
    /// Every key a service entry may declare, in schema order.
    ///
    /// One list, read by the parser and by the error it produces — `ADR-011`:
    /// exactly one thing is authoritative, and it says so at the moment the
    /// answer is needed.
    ///
    /// **Two, since 2026-08-24.** It was three; `listen` left for the manifest.
    ///
    /// **The list itself moved to `rivet_core::service::OPERATOR_KEYS` on
    /// 2026-09-05** and this is that list, not a copy of it. A second reader
    /// appeared: `rivet_core::settings::render` refuses a key of this table
    /// written under `[applications.<name>]` and names this table in the
    /// refusal, and it cannot see this crate. Two lists spelling one fact is
    /// what `ADR-011` is about.
    pub const DECLARED_KEYS: &'static [&'static str] = rivet_core::service::OPERATOR_KEYS;

    /// Keys that belong to the **definition** and are refused with an
    /// explanation rather than as merely unknown.
    ///
    /// Three groups, and all are read out of the record rather than guessed:
    ///
    /// * `restart`, `ready`, `kind` are in `ADR-015` §3.2's manifest, and
    ///   `ADR-017` §4.1 put the first two there *specifically* so they could
    ///   not be rewritten outside the signature.
    /// * **`listen`, ruled into the manifest 2026-08-24.** It is the one key in
    ///   this list that used to be *declared* here, so an operator meeting the
    ///   refusal is not making a mistake — they are reading `ADR-007` §7.2's
    ///   older text, or `ADR-015` §5.2's sentence about port 443. The
    ///   explanation has to say that, which is why
    ///   [`crate::ConfigError::NotConfigurable`] carries one rather than naming a
    ///   category.
    /// * `exec`, `program`, `command`, `args`, `env`, `user` are in no manifest
    ///   at all — `ADR-015` §3.2 says there is "deliberately no `exec` field"
    ///   because the binary is the crate's binary. They are listed because they
    ///   are what an operator carrying systemd habits will reach for first, and
    ///   a bare "unknown key" would leave that model uncorrected.
    ///
    /// `network`, `state`, `provides` and `connects` are deliberately **not**
    /// here. They are permission requests (`ADR-015` §3.3) and where the
    /// operator's *grant* is recorded is undecided, so they fall through as
    /// ordinary unknown keys rather than being pronounced on.
    pub const DEFINITION_KEYS: &'static [&'static str] = &[
        "args", "command", "env", "exec", "kind", "listen", "program", "ready", "restart", "user",
    ];

    /// Why one particular definition key is not the operator's, in a sentence
    /// the refusal carries.
    ///
    /// **One sentence per key rather than one for the class**, since 2026-08-24.
    /// Every other key in [`Self::DEFINITION_KEYS`] has always been the
    /// definition's, so an operator who writes one has a wrong model and the
    /// general sentence corrects it. `listen` is different in kind: it *was*
    /// declared here until the split-by-direction ruling, and both `ADR-007`
    /// §7.2 and `ADR-015` §5.2 told an operator to write it. A refusal that gave
    /// them the general sentence would read as Rivet being wrong rather than
    /// as a decision, and they would have nowhere to go.
    #[must_use]
    pub const fn why_not_configurable(key: &str) -> &'static str {
        match key.as_bytes() {
            b"listen" => {
                "Ruled 2026-08-24: which sockets a service is served on is a fact its author \
                 asserted about the program, so it lives in the signed manifest and an operator \
                 cannot move it. This key WAS accepted here until that ruling, and ADR-007 §7.2 \
                 and ADR-015 §5.2 both still read as though it were."
            }
            b"restart" | b"ready" => {
                "Whether a clean exit means finished or crashed, and how a program declares that \
                 it is serving, are knowable only to whoever wrote it (ADR-017 §4.1)."
            }
            b"exec" | b"program" | b"command" | b"args" | b"env" | b"user" => {
                "There is no such field anywhere, not even in the manifest: the artifact is a \
                 Cargo package, so the binary IS the crate's binary (ADR-015 §3.2)."
            }
            _ => {
                "It ships inside the signature so that what a signed artifact does cannot be \
                 rewritten from outside it (ADR-015 §4.1)."
            }
        }
    }

    /// Build a service entry. The parser is the only caller today.
    pub(crate) const fn new(
        name: ServiceName,
        enabled: bool,
        memory: Option<MemoryLimit>,
        cpu: Option<CpuLimit>,
        critical: bool,
    ) -> Self {
        Self {
            name,
            enabled,
            memory,
            cpu,
            critical,
        }
    }

    /// Which service this configures.
    pub const fn name(&self) -> &ServiceName {
        &self.name
    }

    /// Whether the operator wants it running.
    ///
    /// `ADR-017` §6: this is configuration, not an operation — setting it false
    /// and applying is how a service is stopped for good. `apply service-stop`
    /// pauses an application until it is started again or the machine reboots,
    /// and never changes this value.
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    /// The memory cap, if the operator stated one.
    ///
    /// **A cap, not a grant** (ruled 2026-08-24). It can only tighten what the
    /// artifact asked for. Where the artifact asked for nothing it replaces
    /// the default limit, higher or lower (`ADR-015` §3);
    /// `rivet_core::service::effective_application_memory` is the rule and
    /// the supervisor applies it.
    pub const fn memory(&self) -> Option<MemoryLimit> {
        self.memory
    }

    /// The CPU cap, if the operator stated one — [`Self::memory`]'s footing
    /// exactly, ruled the same way on 2026-09-09.
    ///
    /// **A cap, not a grant.** It can only tighten what the artifact asked
    /// for; `rivet_core::service::effective_cpu` is the rule and the
    /// supervisor applies it.
    pub const fn cpu(&self) -> Option<CpuLimit> {
        self.cpu
    }

    /// **Whether this machine's owner considers this service critical** —
    /// `ADR-045` §4, added 2026-09-08.
    ///
    /// It decides one thing: whether this service's health moves the MACHINE's
    /// answer (`rivet_core::health::machine`). It starts nothing, stops
    /// nothing, and changes no code.
    ///
    /// **It is not a manifest key and must not become one.** An application
    /// that could declare itself critical would be setting its own severity,
    /// and *an author who can set their own severity will set it high, and
    /// every author will*. The same binary is critical on the machine that
    /// serves the house and unimportant on a bench, and only the document knows
    /// which machine this is.
    ///
    /// **It defaults to false**, which is `ADR-045` §7's protection against a
    /// commit gate that never passes — and it is the reason absence is allowed
    /// to mean something here where `enabled`'s is not: the operator has said
    /// nothing about importance, and *nothing you told me to watch* is a
    /// complete, unambiguous reading.
    pub const fn critical(&self) -> bool {
        self.critical
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn no_declared_key_can_change_what_a_service_executes() {
        // The rule in this module's documentation, stated as a test rather than
        // as a paragraph. If someone adds `exec` to `DECLARED_KEYS`, this fails.
        for forbidden in [
            "exec",
            "program",
            "command",
            "args",
            "argv",
            "env",
            "user",
            "uid",
            "group",
            "workdir",
            "working-directory",
            "path",
            "binary",
            "restart",
            "ready",
        ] {
            assert!(
                !Service::DECLARED_KEYS.contains(&forbidden),
                "`{forbidden}` is a service definition, not configuration — ADR-017 §4.1"
            );
        }
    }

    /// **The 2026-08-24 ruling, as a test — and 2026-09-09's addition to it.**
    /// `listen` is the manifest's, and the operator's surface is `enabled`,
    /// `memory`, `cpu` and `critical`.
    #[test]
    fn the_operators_surface_is_enabled_the_two_caps_and_critical() {
        assert_eq!(
            Service::DECLARED_KEYS,
            &["enabled", "memory", "cpu", "critical"]
        );
        assert!(
            Service::DEFINITION_KEYS.contains(&"listen"),
            "listen moved to the manifest on 2026-08-24 and must be refused with an explanation"
        );
    }

    #[test]
    fn the_two_key_lists_do_not_overlap() {
        // A key that is both declared and refused would make the parser's
        // behaviour depend on the order it happens to check them.
        for declared in Service::DECLARED_KEYS {
            assert!(
                !Service::DEFINITION_KEYS.contains(declared),
                "`{declared}` is in both lists"
            );
        }
    }

    #[test]
    fn the_definition_keys_include_the_ones_adr_017_4_1_moved_into_the_signature() {
        for key in ["restart", "ready"] {
            assert!(
                Service::DEFINITION_KEYS.contains(&key),
                "{key} is not refused"
            );
        }
    }

    /// The document's half of the split, from the outside: a service entry says
    /// whether it runs and how much it may have, and can say nothing else.
    #[test]
    fn an_entry_carries_a_name_a_boolean_and_at_most_a_cap() {
        let name = ServiceName::new("photo-index").expect("valid");
        let uncapped = Service::new(name.clone(), true, None, None, false);
        assert_eq!(uncapped.name(), &name);
        assert!(uncapped.enabled());
        assert_eq!(uncapped.memory(), None);
        assert_eq!(uncapped.cpu(), None);
        assert!(!uncapped.critical());

        let cap = MemoryLimit::parse("64M").expect("valid");
        let cpu_cap = CpuLimit::parse("0.5").expect("valid");
        let capped = Service::new(name, false, Some(cap), Some(cpu_cap), true);
        assert!(!capped.enabled());
        assert_eq!(capped.memory(), Some(cap));
        assert_eq!(capped.cpu(), Some(cpu_cap));
        assert!(capped.critical());
    }
}
