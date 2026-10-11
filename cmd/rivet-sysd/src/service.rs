//! What the supervisor knows about a service, and how it decides what to do.
//!
//! # This is an internal representation, not a format — read this before adding a field
//!
//! `ADR-017` §4 is unambiguous that there are **no unit files** and that the
//! operator never writes one: a system service is defined by Rivet in the
//! image, an application service by its own manifest (`ADR-015` §3.2), and the
//! desired-state document (`ADR-007`) is the only authority over what should be
//! running.
//!
//! ~~**What the record does not yet say is what any of those definitions
//! contain.**~~ **Superseded 2026-08-23.** `ADR-017` §4.1 answered it — a
//! system service definition is *compiled into `rivet-sysd`*, and an
//! application's lives in `[package.metadata.rivet]` inside the signature —
//! and `rivet-config` grew `ADR-007` §7.2's `[services]` table on the same
//! day. Both were already true when the paragraph below was written; it went
//! stale between the ruling and the code, which is `AGENTS.md` §7 in its other
//! direction, and it is struck rather than deleted for that reason.
//!
//! [`crate::definitions`] is now where the compiled-in table lives, and
//! [`crate::document`] is the union of it with desired state that `ADR-017`
//! §4.1 calls for. What follows is the check that was made at the time:
//!
//! 1. `git ls-files` matching `service` finds `docs/ADR-017` and this crate,
//!    and nothing else — no schema file, no example document.
//! 2. Grepping `rivet-config/src` and `rivet-core/src` for `service` finds
//!    two hits, both of them a comment in `rivet-core::mount` explaining why
//!    `/run` is not `noexec`. `rivet-config` declares exactly one key today,
//!    the schema version, and says so in its own module documentation.
//!
//! `ADR-015` §3.2's application manifest is the closest thing that exists, and
//! it describes sandboxing and sockets — `kind`, `listen`, `network`, `state`,
//! `memory` — with **no** field for what to execute, and none for `ADR-017`
//! §3.5's restart policy.
//!
//! So [`ServiceDef`] is deliberately a Rust struct that nothing parses. It is
//! the smallest thing the supervision machinery in [`crate::supervise`] can be
//! written and tested against, and it is **not** a proposal for a file format.
//! Inventing one here would create exactly the second authority `ADR-011`
//! forbids, and it would be the second authority *over configuration*, which is
//! the case `ADR-006` §2 names as the concrete failure the whole design exists
//! to avoid.
//!
//! **That last paragraph is the half that survived the ruling, and it is now a
//! rule rather than a caution.** `ADR-017` §4.1 made "a Rust struct that nothing
//! parses" the *answer* for system services rather than a stopgap: a data file
//! under `/system` would buy nothing a constant does not — the filesystem is
//! read-only `erofs`, so changing one means rebuilding the image either way —
//! while costing a second schema and a second parser. Nothing here is
//! deserialised, and nothing here should be.
//!
//! The one field that arrived with the definitions is
//! [`ServiceDef::consequence`], and its own documentation says why it is a
//! definition key rather than a configuration one.

/// Where a definition came from, and therefore what is known about it.
///
/// **Added 2026-08-24** with the split-by-direction ruling. `ADR-017` §4 always had two origins — *"system services are defined
/// by Rivet itself, in the image"*, *"application services come from the
/// application's own manifest"* — and until an application could actually be
/// defined there was only one of them to hold.
///
/// It is not decoration. Three things differ by origin and each of them is a
/// sentence somebody has to be able to read on a console:
///
/// 1. **What is authoritative for the definition.** The image, or a signed
///    manifest on `/state`.
/// 2. **Whether the program can appear later.** `/system` is read-only `erofs`
///    (`ADR-018` §3), so a missing system binary is permanent and
///    [`crate::supervise`] refuses it once rather than retrying. `/state` is
///    writable, and an application's binary really can arrive while the machine
///    runs.
/// 3. **What is not yet enforced about it.** `ADR-015` §4.2 requires an
///    application's signature to be checked *at every launch* and nothing does
///    that; `ADR-015` §6.1 requires that to be visible rather than silent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Compiled into this binary and shipped in the image (`ADR-017` §4.1).
    Image,
    /// Read from an installed application's manifest
    /// (`rivet_core::package::application`).
    Application,
}

impl Origin {
    /// A word for a journal record.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Application => "application",
        }
    }
}

/// What to do when a service exits. `ADR-017` §3.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    /// Restart however it exited.
    Always,
    /// Restart only when it did not exit cleanly.
    OnFailure,
    /// Never restart. A `oneshot` (`ADR-015` §3.2's `kind`) is this.
    Never,
}

impl RestartPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::OnFailure => "on-failure",
            Self::Never => "never",
        }
    }

    /// The three spellings `ADR-017` §3.5 uses, and nothing else.
    ///
    /// A closed vocabulary per `ADR-006` §6.4: an unknown spelling is refused
    /// rather than defaulted, because a service that silently became
    /// `on-failure` when the operator wrote `onfailure` is a machine whose
    /// behaviour does not match what anyone wrote down.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "always" => Some(Self::Always),
            "on-failure" => Some(Self::OnFailure),
            "never" => Some(Self::Never),
            _ => None,
        }
    }

    /// `ADR-017` §3.5 applied to one exit.
    pub fn wants_restart(self, status: crate::kernel::ExitStatus) -> bool {
        match self {
            Self::Always => true,
            Self::OnFailure => !status.is_success(),
            Self::Never => false,
        }
    }
}

/// How the supervisor learns that a service is *ready*, as distinct from
/// *started*. `ADR-017` §3.3.
///
/// The distinction is the ADR's own honesty rule applied to itself: reporting a
/// service as up when it is not yet serving is the same dishonesty `AGENTS.md`
/// §6 rejects.
///
/// **Which of these a given service uses is a service-definition question the
/// record does not answer.** `ADR-017` §3.3 says supplying readiness is the
/// platform crate's job (`ADR-013`), and the platform crate is ruled to emerge
/// at Stage 3 and does not exist. So every service Rivet ships today must be
/// [`Readiness::OnStart`] — none of them can declare readiness, whether or not
/// they could be made to — and a Rivet service written against the platform
/// crate will be [`Readiness::Notify`]. Nothing written down says where that is
/// recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    /// The service declares readiness over `NOTIFY_SOCKET` (`ADR-017` §3.4).
    Notify,
    /// Nothing declares readiness, so the supervisor reports `running` and
    /// never `running, ready`. It does **not** assume readiness — that is the
    /// distinction §3.3 exists to preserve.
    OnStart,
    /// The manifest asked for `ADR-017` §4.2's `listening`, and **this build
    /// cannot supply it.**
    ///
    /// Added 2026-08-24, when a manifest could first say so. §3.2's pre-created
    /// socket and §3.4's `LISTEN_FDS` handoff are not built, so there is no
    /// socket for a connection to arrive on and the service can never be
    /// reported ready.
    ///
    /// **It is a distinct variant rather than being folded into
    /// [`Readiness::OnStart`]**, and the difference is the whole of `ADR-015`
    /// §6.1: `OnStart` says *this program has no readiness signal*, which is a
    /// true statement about a service that never claimed one and a false one
    /// about an application that declared `listening`. Collapsing them would
    /// make the machine
    /// **agree** with a manifest it cannot honour. Everything downstream treats
    /// it as `OnStart`; only the report differs, and that is the point.
    ListeningUnsupported,
}

impl Readiness {
    /// Whether this service will ever declare readiness to this supervisor.
    ///
    /// One place to ask, so that adding a fourth value cannot silently mean
    /// "notifies" somewhere nobody updated.
    pub const fn notifies(self) -> bool {
        matches!(self, Self::Notify)
    }
}

/// One service, as the supervisor holds it. See this module's documentation
/// before treating this as a schema: it is not one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDef {
    /// Unique. Also the cgroup name (`ADR-017` §3.1.3) and the notify socket
    /// name, which is why [`ServiceDef::validate`] refuses a name containing a
    /// path separator.
    pub name: String,
    pub program: crate::kernel::Program,
    /// `ADR-017` §6 — `enabled` is configuration, not an operation, and no
    /// operation writes it. A stop an operator holds on an application lives
    /// beside the definition, in `crate::supervise::Managed::asked`.
    pub enabled: bool,
    pub restart: RestartPolicy,
    pub readiness: Readiness,
    /// **What the machine loses while this service is not running**, in a
    /// sentence somebody reads at a serial console with no other diagnostic
    /// channel.
    ///
    /// Added 2026-08-23 with the first real definitions. It is the same
    /// mechanism `ADR-017` §3.1.6 introduced for a missing cgroup2 and §3.1.7
    /// reused for a missing `/state` — `consequence_of` — and it is here for
    /// the reason those two sections give: a component reporting that it is
    /// degraded, in neutral terms, tells the reader nothing about whether to
    /// care. *"`rivet-mcpd` could not be started"* and *"there is no control
    /// plane, so a headless board cannot be reached at all"* are the same fact
    /// and only one of them is actionable.
    ///
    /// It is a **definition** key by `ADR-017` §4.1's three-clause rule: it is
    /// a fact the artifact's author asserts about their own program, not a
    /// policy the machine's owner sets, and nothing outside the process could
    /// enforce it. It never reaches `ADR-007`'s schema.
    ///
    /// Empty is allowed and means *nothing has been written down*, which is
    /// what a `--dev-service` harness entry is.
    pub consequence: String,

    // -- the 2026-08-24 split ----------------------
    /// Where this definition came from. See [`Origin`].
    pub origin: Origin,

    /// **Which volume this application is installed on** — the root directory,
    /// not the name. `ADR-031` §8 item 1, 2026-08-25.
    ///
    /// `None` for an `Origin::Image` service, which is compiled into this binary
    /// and is on no volume at all. `Some` for an application, and today it is
    /// always `rivet_core::service::BUILT_IN_VOLUME`, because
    /// `crate::applications::discover` is given one root to walk and nothing
    /// attaches a second (`ADR-031` §11 items 1, 2 and 8 are unruled).
    ///
    /// It is carried on the definition rather than recomputed because an
    /// application's paths are no longer a function of its name alone: the data
    /// directory, the program, the manifest and the `/etc` home field all need
    /// it, and re-deriving it in four places would be four chances to disagree
    /// about where an application lives (`ADR-011`).
    pub volume: Option<String>,

    /// The sockets the **manifest** asked Rivet to open — never the
    /// document's, since 2026-08-24.
    ///
    /// Empty for every system service: a system service reads its own address
    /// out of the desired-state document at a path this supervisor hands it,
    /// which `crate::definitions` records as a thing that should stop being
    /// true the day socket activation is built (`ADR-015` §5.2 — Rivet
    /// pre-creates the socket and passes the descriptor, so the service need
    /// not know the port).
    ///
    /// **Nothing opens these.** `ADR-017` §3.2's pre-created socket and §3.4's
    /// `LISTEN_FDS` handoff are not built, so this is a declaration the
    /// supervisor reports and does not honour (`ADR-015` §6.1). It is carried
    /// rather than dropped for two reasons: the report has to name what it is
    /// not doing, and two applications claiming one port is a collision only
    /// something holding every definition at once can see —
    /// `crate::applications::refuse_colliding_sockets`.
    pub listen: Vec<rivet_core::package::application::Socket>,

    /// **Whether the manifest declares a `proxy-path`**, which decides where
    /// [`ServiceDef::listen`]'s tcp and udp sockets bind — `127.0.0.1` unless a
    /// socket is `:direct` (`ADR-046` §7, `.rulings/` R-188,
    /// `rivet_core::package::application::Socket::bind_address`).
    ///
    /// Carried on the definition rather than re-read from the manifest at each
    /// start, for [`ServiceDef::volume`]'s reason, and so that a reinstall that
    /// adds or drops the route is a changed definition. `false` for every image
    /// service, which has no manifest and no route.
    pub proxied: bool,

    /// `provides = ["socket:<name>"]` — `ADR-015` §5.3, implemented
    /// 2026-08-31, chown-based (`crate::endpoints`).
    ///
    /// Each entry becomes a unix socket the supervisor binds and hands over as
    /// a descriptor, exactly like [`ServiceDef::listen`] — it IS a listen, and
    /// `crate::applications` turns it into one rather than building a second
    /// path to the same syscall. What `provides` adds is that the socket's
    /// **file** may then be granted to a consumer.
    pub provides: Vec<rivet_core::package::application::Endpoint>,

    /// `connects = ["<provider>:<name>"]` — the other half.
    ///
    /// A request, not a grant. `crate::endpoints::resolve` decides whether it
    /// becomes one, and it can refuse for three reasons that a single
    /// definition cannot see: no such provider, that provider does not offer
    /// it, or another application already holds it.
    pub connects: Vec<rivet_core::package::application::Endpoint>,

    /// The device classes this service's manifest declared — `ADR-015` §6.2.
    ///
    /// **Always empty for an image service**, and that is not an oversight: the
    /// image's own services run as root and already reach every node, so a
    /// declaration would grant them nothing. The capability describes what an
    /// *application* may do (§6.3).
    ///
    /// Like `listen`, a conflict here is invisible to any one definition — only
    /// something holding every definition at once can see two claimants of one
    /// screen (`crate::devices::contested`).
    pub devices: Vec<rivet_core::package::application::Device>,

    /// **The per-machine material this application's manifest declares it
    /// needs before it can work** — `ADR-055` §3, straight from the manifest
    /// like `devices` beside it.
    ///
    /// **Always empty for an image service**, for `devices`'s reason: no
    /// manifest, nothing declared. `crate::supervise::start` checks every file
    /// each kind names against the application's data directory before it
    /// spawns anything, and holds the service `waiting-for-material`
    /// (`ADR-055` §5) rather than starting a process that would find its
    /// credentials missing.
    pub material: Vec<rivet_core::package::application::Material>,

    /// What the **manifest** asked for, before the operator's cap is applied.
    ///
    /// `None` for a system service, which has no manifest to ask with. Kept
    /// beside [`ServiceDef::memory`] rather than recovered from it because the
    /// merge runs once per boot over a definition that may already have been
    /// merged in a test, and a rule that could not be re-applied to its own
    /// output would be one this crate had to remember not to call twice.
    pub requested_memory: Option<rivet_core::service::MemoryLimit>,

    /// **The limit an application gets when its manifest asks for none** —
    /// `rivet_core::service::default_application_memory`, half of this
    /// machine's memory. `None` for an image service, which has no manifest,
    /// and on a machine whose memory could not be read.
    pub default_memory: Option<rivet_core::service::MemoryLimit>,

    /// The cap that applies, and which side of the split it came from.
    ///
    /// `rivet_core::service::effective_memory` computes it — **the lower
    /// wins** — and `crate::document::apply_desired_state` is where the
    /// operator's half arrives. Until then it is the manifest's request alone,
    /// which is what a machine with no document should run under.
    pub memory: rivet_core::service::EffectiveMemory,

    /// [`ServiceDef::requested_memory`]'s footing, for CPU — `ADR-007` §7
    /// and `ADR-015` §3's 2026-09-09 amendment. `None` for a system
    /// service, for the identical reason.
    pub requested_cpu: Option<rivet_core::service::CpuLimit>,

    /// [`ServiceDef::memory`]'s footing, for CPU: `rivet_core::service::
    /// effective_cpu` computes it and `crate::document::apply_desired_state`
    /// is where the operator's half arrives, on the identical schedule.
    pub cpu: rivet_core::service::EffectiveCpu,

    /// **Whether the operator's document marks this service `critical`** —
    /// `ADR-045` §4, added 2026-09-08.
    ///
    /// It decides exactly one thing: whether this service's health moves the
    /// machine's (`rivet_core::health::machine`). It starts nothing and stops
    /// nothing, so unlike [`ServiceDef::enabled`] it can never contradict
    /// `decide` — which is why the convergence pass does not look at it at all.
    ///
    /// **It is the document's and never the manifest's**, so it arrives in
    /// `crate::document::apply_desired_state` beside `enabled` and defaults to
    /// `false` for a machine with no document. An application that could
    /// declare itself critical would be setting its own severity, which is
    /// what `ADR-045` §2 and §4.3 take away from application authors.
    pub critical: bool,
}

/// The fields a service **defined by the image** always has the same way.
///
/// Written for struct-update syntax — `ServiceDef { name: …, ..IMAGE_DEFINED }`
/// — so that the three fields the 2026-08-24 split added do not have to be
/// spelled at every definition, and so that adding a fourth is one edit.
///
/// **Not a `Default` impl**, and the difference is the point: three of these are
/// empty *because a system service has no manifest and sits on no volume*, and a
/// `Default` would give an application the same values silently. `crate::applications` fills all
/// three from the manifest and never uses this.
///
/// Every field it does not carry is one the caller must state. `name` is empty
/// here and [`ServiceDef::validate`] refuses an empty name, so a definition that
/// forgot to override it fails loudly at start-up rather than becoming a
/// nameless service.
pub const IMAGE_DEFINED: ServiceDef = ServiceDef {
    name: String::new(),
    program: crate::kernel::Program {
        path: String::new(),
        args: Vec::new(),
        env: Vec::new(),
        // **uid 0, and for a system service that is the answer rather than the
        // default.** `rivet-netd` speaks netlink, `rivet-timed` sets the wall
        // clock and `rivet-mcpd` execs the CLI that installs payloads and
        // reboots the machine; each is uid 0's work and a system service given
        // an identity here would start and be unable to do its job. `ADR-015`
        // §5.1's per-application uid is about *applications*, which
        // `crate::applications` fills in — and about the image services
        // `rivet_core::service::IMAGE_SERVICE_HOMES` names, whose definitions
        // override this field because nothing they do needs root (`ADR-060`).
        identity: None,
        confinement: None,
        groups: Vec::new(),
        capabilities: Vec::new(),
    },
    enabled: false,
    restart: RestartPolicy::Never,
    readiness: Readiness::OnStart,
    consequence: String::new(),
    origin: Origin::Image,
    // Compiled into this binary, so it is on no volume — `ADR-031` §8 item 1.
    volume: None,
    // A system service reads its own address out of the desired-state document
    // today, not from here — `crate::definitions` records that as a thing that
    // should stop being true when socket activation is built.
    listen: Vec::new(),
    // No manifest, so no `proxy-path` — `ADR-046` §7.
    proxied: false,
    // An image service provides nothing to an application and connects to
    // nothing. `ADR-015` §5.3 is application-to-application; a system service
    // runs as uid 0 and reaches any socket already, so a grant would be a
    // no-op dressed as a permission.
    provides: Vec::new(),
    connects: Vec::new(),
    devices: Vec::new(),
    // No manifest, so nothing declared — `ADR-055` §3.
    material: Vec::new(),
    // No manifest, so nothing requested. An operator's cap on a system service
    // therefore stands alone, which is `ADR-007` §7.2's grant unchanged.
    requested_memory: None,
    // The default is an application's; an image service's memory is the
    // image's own business and the operator's.
    default_memory: None,
    memory: rivet_core::service::EffectiveMemory::UNSTATED,
    // `requested_memory`/`memory`'s footing, for CPU.
    requested_cpu: None,
    cpu: rivet_core::service::EffectiveCpu::UNSTATED,
    // **The operator's word, and nobody has said it yet** — `ADR-045` §4's
    // default. An image service is not critical by virtue of being one: the
    // document is what decides, on this machine.
    critical: false,
};

/// Why a [`ServiceDef`] was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefError {
    EmptyName,
    /// The name is used as a path component for the cgroup and the notify
    /// socket, so a separator in it would escape both.
    NameNotAPathComponent {
        name: String,
    },
    EmptyProgram {
        name: String,
    },
    /// `PATH` lookup is a shell's job. A supervisor that resolved a bare name
    /// would run whatever the environment pointed at, which is not a decision
    /// the machine's configuration made.
    ProgramNotAbsolute {
        name: String,
        program: String,
    },
    Duplicate {
        name: String,
    },
    /// An `Origin::Application` definition with no uid of its own, which would
    /// execute a binary out of writable `/state` as root. Added 2026-08-24 with
    /// `ADR-015` §5.1's identity.
    ApplicationWithoutAnIdentity {
        name: String,
    },
    /// An image service carrying a ruleset without both an identity and a
    /// volume — `ADR-060`. A ruleset on uid 0 confines a program that can undo
    /// it, and a home with no volume has nowhere to be.
    ConfinedImageServiceWithoutAHome {
        name: String,
    },
    /// A capability kept by something other than an image service. An
    /// application is handed none of root's powers.
    CapabilityOutsideAnImageService {
        name: String,
    },
}

impl core::fmt::Display for DefError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptyName => write!(f, "a service must have a name"),
            Self::NameNotAPathComponent { name } => write!(
                f,
                "service name {name:?} is not a single path component; it names a cgroup and a socket"
            ),
            Self::EmptyProgram { name } => write!(f, "service {name:?} has no program to run"),
            Self::ProgramNotAbsolute { name, program } => write!(
                f,
                "service {name:?} names {program:?}, which is not an absolute path"
            ),
            Self::Duplicate { name } => write!(f, "service {name:?} is defined more than once"),
            Self::ApplicationWithoutAnIdentity { name } => write!(
                f,
                "application service {name:?} has no uid of its own, so it would run as root out \
                 of writable /state (ADR-015 §5.1)"
            ),
            Self::ConfinedImageServiceWithoutAHome { name } => write!(
                f,
                "image service {name:?} carries a ruleset without both a uid of its own and a \
                 volume, so it would be confined as root or given a home that is nowhere \
                 (ADR-060)"
            ),
            Self::CapabilityOutsideAnImageService { name } => write!(
                f,
                "service {name:?} keeps a capability but is not an image service, so the list \
                 would hand an application part of root (ADR-017 §3)"
            ),
        }
    }
}

impl ServiceDef {
    /// Check the invariants a caller would otherwise have to trust.
    ///
    /// `AGENTS.md` §3 — refuse rather than guess. Every one of these would
    /// otherwise fail much later, in a place where the cause is invisible: a
    /// name with a `/` in it silently creates a nested cgroup, and a relative
    /// program is resolved against whatever the supervisor's working directory
    /// happens to be.
    pub fn validate(&self) -> Result<(), DefError> {
        if self.name.is_empty() {
            return Err(DefError::EmptyName);
        }
        if self.name.contains('/') || self.name == "." || self.name == ".." {
            return Err(DefError::NameNotAPathComponent {
                name: self.name.clone(),
            });
        }
        if self.program.path.is_empty() {
            return Err(DefError::EmptyProgram {
                name: self.name.clone(),
            });
        }
        if !self.program.path.starts_with('/') {
            return Err(DefError::ProgramNotAbsolute {
                name: self.name.clone(),
                program: self.program.path.clone(),
            });
        }
        // **The one that stops an application quietly running as root.**
        // `crate::applications` fills this in for every definition it makes, so
        // an `Origin::Application` without one is a definition that reached
        // this list by some other route — a hand-built one in a test, or a
        // future installer. Either way, starting it would be uid 0 executing a
        // binary out of writable `/state`, which is the exact state the
        // 2026-08-24 ruling was made to end.
        //
        // Refusing rather than deriving the identity here: this function's job
        // is to check invariants a caller would otherwise have to trust, and a
        // `validate` that repaired what it found would let the real defect —
        // whatever built the definition — go on existing.
        if self.origin == Origin::Application && self.program.identity.is_none() {
            return Err(DefError::ApplicationWithoutAnIdentity {
                name: self.name.clone(),
            });
        }
        // **An image service that runs in user space has both halves or
        // neither** (`ADR-060`). Its ruleset is composed from a home under a
        // volume and applied after the drop to its uid; with either missing the
        // definition is one somebody half-wrote, and refusing it here is
        // cheaper than finding out from an `EACCES` on a board.
        if self.origin == Origin::Image
            && self.program.confinement.is_some()
            && (self.program.identity.is_none() || self.volume.is_none())
        {
            return Err(DefError::ConfinedImageServiceWithoutAHome {
                name: self.name.clone(),
            });
        }
        // **A kept capability is an image service's**, in one of two shapes:
        // under its own uid, keeping what it names across the drop; or as uid
        // 0, with the bounding set reduced to what it names and `NoNewPrivs`
        // set (`crate::kernel::Kernel::spawn`). Every application keeps none.
        // A ruleset on uid 0 is refused above, so the second shape is never
        // also confined.
        if !self.program.capabilities.is_empty() && self.origin != Origin::Image {
            return Err(DefError::CapabilityOutsideAnImageService {
                name: self.name.clone(),
            });
        }
        Ok(())
    }
}

/// The numbers `ADR-017` §3.5 asks for without naming.
///
/// > "exponential backoff and a give-up threshold. After the threshold the
/// > supervisor stops trying, marks the service failed, and reports it."
///
/// **Every value here was chosen by the implementation, not read out of the
/// record**, and they are gathered into one struct so that they are one thing
/// to rule on rather than six constants scattered through a loop. The reasoning
/// for each is on the field; the report accompanying this change asks for a
/// ruling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The first backoff. Short enough that a service failing on a transient
    /// precondition — which `ADR-017` §3.2's convergence model makes the
    /// *normal* case at boot, not an error — comes back quickly.
    pub backoff_base_ms: u64,
    /// The ceiling. `ADR-017` §3.5's hazard is SD-card wear from a restart
    /// loop, so the interval has to reach something slow.
    pub backoff_cap_ms: u64,
    /// Consecutive failures before the service is marked failed and left
    /// alone. `ADR-017` §3.5: "the difference between a broken service and a
    /// broken machine".
    pub give_up_after: u32,
    /// How long a service must stay up before its failure count is forgiven.
    ///
    /// **Not in the record, and load-bearing.** Without it the counter is
    /// cumulative over the machine's whole uptime, so a service that crashes
    /// once a week is eventually marked permanently failed — a supervisor that
    /// gives up on a *working* service. With it, "five failures" means five in
    /// quick succession, which is what §3.5 is plainly about.
    pub success_window_ms: u64,
    /// How long to wait for a [`Readiness::Notify`] service to declare itself
    /// ready before saying it has not.
    ///
    /// Nothing is killed when this expires. A service that is slow to become
    /// ready is not a service that has failed, and `ADR-017` §3.2's whole model
    /// is that things retry until their preconditions hold.
    pub ready_timeout_ms: u64,
    /// How long a service gets after `SIGTERM` before `SIGKILL`.
    pub stop_grace_ms: u64,
    /// How long after `SIGKILL` before the supervisor stops waiting.
    ///
    /// It then abandons the process rather than blocking shutdown for ever —
    /// which is safe precisely because of `ADR-017` §3.1.1: an abandoned child
    /// reparents to PID 1, which reaps it.
    pub kill_grace_ms: u64,
    /// How often to re-ask the cgroup about a service the supervisor adopted
    /// rather than started (`ADR-017` §3.1.2).
    pub adopt_poll_ms: u64,
    /// **How often a service held `waiting-for-material` is looked at again**
    /// — `ADR-055` §5.
    ///
    /// `apply install-secret` writes a credential straight into an
    /// application's data directory and wakes nobody (`rivet-core`'s
    /// `install_secret` module says so in its own answer) — so unlike an
    /// install or a reload, nothing on this machine tells the supervisor the
    /// file arrived. This is therefore a poll, not a push, on the same footing
    /// as [`Self::adopt_poll_ms`]. The number is cheap to pick: the check is a
    /// handful of `stat`s, not a write, so none of the restart backoff's
    /// SD-card argument (`ADR-017` §3) applies to it.
    pub material_poll_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            backoff_base_ms: 250,
            backoff_cap_ms: 30_000,
            give_up_after: 5,
            success_window_ms: 60_000,
            ready_timeout_ms: 30_000,
            // **From `rivet_core::shutdown`, not written here** — `.rulings/`
            // R-141. PID 1's own grace is computed from these two, and it can
            // only stay longer than them if there is one place they live.
            stop_grace_ms: rivet_core::shutdown::SERVICE_STOP_GRACE_MS,
            kill_grace_ms: rivet_core::shutdown::SERVICE_KILL_GRACE_MS,
            adopt_poll_ms: 1_000,
            material_poll_ms: 5_000,
        }
    }
}

impl Limits {
    /// The delay before attempt `attempt`, counting the first restart as 1.
    ///
    /// Doubling, capped, and saturating — a shift by 64 is undefined in C and
    /// panics in debug Rust, and `attempt` is driven by however many times a
    /// service has failed, which is not a number this code gets to bound.
    pub fn backoff_ms(&self, attempt: u32) -> u64 {
        if attempt == 0 {
            return 0;
        }
        let shift = attempt - 1;
        let scaled = if shift >= 63 {
            u64::MAX
        } else {
            self.backoff_base_ms.saturating_mul(1u64 << shift)
        };
        scaled.min(self.backoff_cap_ms)
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::kernel::{ExitStatus, Program};

    fn program(path: &str) -> Program {
        Program {
            path: path.to_string(),
            args: Vec::new(),
            env: Vec::new(),
            identity: None,
            confinement: None,
            groups: Vec::new(),
            capabilities: Vec::new(),
        }
    }

    fn def(name: &str, path: &str) -> ServiceDef {
        ServiceDef {
            name: name.to_string(),
            program: program(path),
            enabled: true,
            restart: RestartPolicy::Always,
            readiness: Readiness::OnStart,
            consequence: String::new(),
            ..IMAGE_DEFINED
        }
    }

    // -- restart policy -----------------------------------------------------

    #[test]
    fn always_restarts_whatever_happened() {
        for status in [
            ExitStatus::Exited(0),
            ExitStatus::Exited(1),
            ExitStatus::Signalled(11),
            ExitStatus::Other,
        ] {
            assert!(RestartPolicy::Always.wants_restart(status), "{status}");
        }
    }

    #[test]
    fn on_failure_leaves_a_clean_exit_alone_and_restarts_everything_else() {
        assert!(!RestartPolicy::OnFailure.wants_restart(ExitStatus::Exited(0)));
        assert!(RestartPolicy::OnFailure.wants_restart(ExitStatus::Exited(1)));
        assert!(RestartPolicy::OnFailure.wants_restart(ExitStatus::Signalled(9)));
    }

    #[test]
    fn never_restarts_nothing_including_a_crash() {
        assert!(!RestartPolicy::Never.wants_restart(ExitStatus::Exited(0)));
        assert!(!RestartPolicy::Never.wants_restart(ExitStatus::Signalled(11)));
    }

    #[test]
    fn the_three_spellings_are_the_ones_adr_017_3_5_uses() {
        assert_eq!(RestartPolicy::parse("always"), Some(RestartPolicy::Always));
        assert_eq!(
            RestartPolicy::parse("on-failure"),
            Some(RestartPolicy::OnFailure)
        );
        assert_eq!(RestartPolicy::parse("never"), Some(RestartPolicy::Never));
        for round_trip in [
            RestartPolicy::Always,
            RestartPolicy::OnFailure,
            RestartPolicy::Never,
        ] {
            assert_eq!(RestartPolicy::parse(round_trip.as_str()), Some(round_trip));
        }
    }

    #[test]
    fn a_near_miss_spelling_is_refused_rather_than_defaulted() {
        for wrong in ["onfailure", "on_failure", "Always", "", "restart"] {
            assert_eq!(RestartPolicy::parse(wrong), None, "{wrong:?} was accepted");
        }
    }

    // -- validation ---------------------------------------------------------

    #[test]
    fn a_plain_definition_validates() {
        assert_eq!(def("netd", "/usr/bin/rivet-netd").validate(), Ok(()));
    }

    #[test]
    fn a_nameless_service_is_refused() {
        assert_eq!(def("", "/bin/true").validate(), Err(DefError::EmptyName));
    }

    /// The name is a cgroup path component and a socket filename. A separator
    /// in it would silently create a nested cgroup under a different service.
    #[test]
    fn a_name_that_is_not_one_path_component_is_refused() {
        for bad in ["a/b", "..", ".", "../escape"] {
            assert!(
                matches!(
                    def(bad, "/bin/true").validate(),
                    Err(DefError::NameNotAPathComponent { .. })
                ),
                "{bad:?} was accepted as a service name"
            );
        }
    }

    #[test]
    fn a_service_with_nothing_to_run_is_refused() {
        assert!(matches!(
            def("x", "").validate(),
            Err(DefError::EmptyProgram { .. })
        ));
    }

    /// `PATH` resolution is a shell's job. A supervisor that did it would run
    /// whatever the inherited environment pointed at.
    #[test]
    fn a_relative_program_is_refused_rather_than_resolved() {
        assert!(matches!(
            def("x", "rivet-netd").validate(),
            Err(DefError::ProgramNotAbsolute { .. })
        ));
        assert!(matches!(
            def("x", "./rivet-netd").validate(),
            Err(DefError::ProgramNotAbsolute { .. })
        ));
    }

    /// An error a human cannot act on is not much better than no error, so
    /// every refusal has to carry the service and the offending value.
    #[test]
    fn every_refusal_names_the_service_and_the_value_it_is_about() {
        for (definition, needles) in [
            (def("a/b", "/bin/true"), vec!["a/b"]),
            (def("x", ""), vec!["\"x\""]),
            (def("y", "rel"), vec!["\"y\"", "rel"]),
        ] {
            let rendered = definition
                .validate()
                .err()
                .map_or_else(|| "<accepted>".to_string(), |e| e.to_string());
            for needle in needles {
                assert!(
                    rendered.contains(needle),
                    "{rendered:?} does not mention {needle:?}"
                );
            }
        }
        assert_eq!(
            DefError::EmptyName.to_string(),
            "a service must have a name"
        );
    }

    // -- backoff ------------------------------------------------------------

    #[test]
    fn backoff_doubles_from_the_base() {
        let l = Limits::default();
        assert_eq!(l.backoff_ms(1), 250);
        assert_eq!(l.backoff_ms(2), 500);
        assert_eq!(l.backoff_ms(3), 1_000);
        assert_eq!(l.backoff_ms(4), 2_000);
    }

    #[test]
    fn backoff_stops_at_the_cap_rather_than_growing_for_ever() {
        let l = Limits::default();
        assert_eq!(l.backoff_ms(20), l.backoff_cap_ms);
        assert_eq!(l.backoff_ms(1_000), l.backoff_cap_ms);
    }

    /// A shift by 64 or more is undefined in C and panics in debug Rust. The
    /// attempt count is driven by how often a service failed, which this code
    /// does not get to bound.
    #[test]
    fn an_absurd_attempt_count_saturates_instead_of_overflowing() {
        let l = Limits::default();
        assert_eq!(l.backoff_ms(63), l.backoff_cap_ms);
        assert_eq!(l.backoff_ms(64), l.backoff_cap_ms);
        assert_eq!(l.backoff_ms(u32::MAX), l.backoff_cap_ms);
    }

    #[test]
    fn attempt_zero_has_no_delay_because_it_is_the_first_start() {
        assert_eq!(Limits::default().backoff_ms(0), 0);
    }

    #[test]
    fn a_cap_below_the_base_still_caps() {
        let l = Limits {
            backoff_base_ms: 1_000,
            backoff_cap_ms: 100,
            ..Limits::default()
        };
        assert_eq!(l.backoff_ms(1), 100);
    }

    #[test]
    fn the_defaults_are_ordered_the_way_the_design_needs() {
        let l = Limits::default();
        assert!(l.backoff_base_ms < l.backoff_cap_ms);
        assert!(
            l.give_up_after > 0,
            "a threshold of 0 never starts anything"
        );
        // The success window must exceed the largest backoff, or a service that
        // is merely slow to fail resets its own counter and never gives up.
        assert!(l.success_window_ms > l.backoff_cap_ms);
        assert!(l.kill_grace_ms < l.stop_grace_ms);
    }

    // -- the identity, ruled 2026-08-24 -------------------------------------

    /// **The structural guard.** `crate::applications` fills the identity in
    /// for every definition it makes, so an `Origin::Application` without one
    /// arrived by some other route — and starting it would be uid 0 executing a
    /// binary out of writable `/state`, which is the state the ruling ended.
    #[test]
    fn an_application_service_with_no_identity_of_its_own_is_refused() {
        let mut d = def("photo-index", "/state/app/photo-index/current/bin/photo-index");
        d.origin = Origin::Application;
        // An application is on a volume, and `finish_uninstalls` refuses to
        // remove one whose volume it cannot name (`ADR-031` §8 item 1).
        d.volume = Some(rivet_core::service::BUILT_IN_VOLUME.to_string());
        assert_eq!(
            d.validate(),
            Err(DefError::ApplicationWithoutAnIdentity {
                name: "photo-index".to_string()
            })
        );
        // With one, it is accepted — or the assertion above would pass for a
        // rule that refuses every application.
        d.program.identity = Some(rivet_core::service::app_identity(
            &rivet_core::service::ServiceName::new("photo-index").expect("valid"),
        ));
        assert_eq!(d.validate(), Ok(()));
    }

    /// The same definition **as a system service** is fine without one, or the
    /// rule above would refuse every service the image defines — each of which
    /// runs as uid 0 deliberately (`IMAGE_DEFINED`).
    #[test]
    fn a_system_service_needs_no_identity() {
        let d = def("rivet-netd", "/system/bin/rivet-netd");
        assert_eq!(d.origin, Origin::Image);
        assert_eq!(d.validate(), Ok(()));
    }

    /// **A confined image service has a uid and a volume, or it is refused** —
    /// `ADR-060`. Each half missing is refused on its own, and both present is
    /// accepted, or the rule would refuse the proxy it was written for.
    #[test]
    fn a_confined_image_service_without_a_uid_or_a_home_is_refused() {
        let name = rivet_core::service::ServiceName::new("rivet-proxyd").expect("valid");
        let volume = rivet_core::service::BUILT_IN_VOLUME;
        let mut d = def("rivet-proxyd", "/system/bin/rivet-proxyd");
        d.program.confinement = Some(rivet_core::confinement::image_service_policy(
            "/system/bin/rivet-proxyd",
            volume,
            &name,
            None,
            &[],
        ));
        let refused = Err(DefError::ConfinedImageServiceWithoutAHome {
            name: "rivet-proxyd".to_string(),
        });

        d.volume = Some(volume.to_string());
        assert_eq!(d.validate(), refused, "a ruleset on uid 0 confines nothing");

        d.volume = None;
        d.program.identity = Some(rivet_core::service::app_identity(&name));
        assert_eq!(d.validate(), refused, "a home with no volume is nowhere");

        d.volume = Some(volume.to_string());
        assert_eq!(d.validate(), Ok(()));
    }

    /// **A kept capability belongs to an image service, and to nothing
    /// else.** Two shapes are accepted: a homed service keeping what it
    /// names across its uid change (`rivet-timed`), and a service that stays
    /// uid 0 with its bounding set reduced to what it names (`rivet-netd`).
    /// An application holding any piece of root is refused, with or without
    /// a uid of its own.
    #[test]
    fn a_kept_capability_outside_an_image_service_is_refused() {
        let name = rivet_core::service::ServiceName::new("rivet-timed").expect("valid");
        let mut d = def("rivet-timed", "/system/bin/rivet-timed");
        d.program.capabilities = vec![crate::kernel::Capability::SYS_TIME];
        let refused = Err(DefError::CapabilityOutsideAnImageService {
            name: "rivet-timed".to_string(),
        });

        assert_eq!(d.validate(), Ok(()), "uid 0 with a reduced bounding set");

        d.program.identity = Some(rivet_core::service::app_identity(&name));
        assert_eq!(d.validate(), Ok(()), "a homed service keeping one");

        d.origin = Origin::Application;
        assert_eq!(d.validate(), refused, "an application keeps no capability");

        d.program.identity = None;
        assert_eq!(
            d.validate(),
            Err(DefError::ApplicationWithoutAnIdentity {
                name: "rivet-timed".to_string()
            }),
            "and an application as root is refused before its capabilities are"
        );
    }
}
