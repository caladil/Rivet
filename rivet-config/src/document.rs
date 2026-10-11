//! The single configuration document.
//!
//! `ADR-007` §3.1 splits the store from the representation: **these types are
//! the store and they are the authority.** TOML is one representation of them,
//! and the parser in [`crate::parse`] is the only way in.


use crate::error::ConfigError;
use crate::network::Interface;
use crate::volumes::Volume;
use crate::carry::Carry;
use crate::fleet_role::FleetRole;
use crate::machine_mode::MachineMode;
use crate::services::Service;
use crate::control_plane::ControlPlane;
use crate::dns::Dns;
use crate::kiosk::Kiosk;
use crate::resolver::Resolver;
use crate::splash::Splash;
use rivet_core::hostname::Hostname;
use crate::applications::Applications;
use rivet_core::display::Hours;
use rivet_core::timezone::Timezone;
use crate::repos::Repo;

/// The schema version this build of Rivet reads and writes.
///
/// See [`SchemaVersion`] for why a document has to declare one.
pub const SCHEMA_VERSION: SchemaVersion = SchemaVersion(1);

/// Which version of the schema a document is written against.
///
/// `ADR-007` §3.2 requires that "a document from an older version is migrated
/// explicitly rather than silently misread", and §4.2 accepts that migrations
/// are then owed for ever. Neither is possible unless the document says which
/// version it is, so the declaration is required from the first document
/// Rivet ever reads — a document that does not declare one can only be
/// guessed at, and `AGENTS.md` §3 forbids guessing.
///
/// **Derived, not quoted.** The record mandates versioning; it does not spell
/// out this field. See the crate documentation for what is settled and what is
/// not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SchemaVersion(u32);

impl SchemaVersion {
    /// Name a schema version.
    pub const fn new(version: u32) -> Self {
        Self(version)
    }

    /// The version as a number.
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl core::fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Rivet's configuration document: **what the machine should be.**
///
/// `ADR-007` §3.3 is the rule this type exists to make structural rather than
/// remembered:
///
/// > The configuration document says what the machine should be. It never
/// > contains what the machine is.
///
/// So there is no field here for an interface's link state, an address the
/// kernel assigned, or a process that is running. Observed state is read live
/// at the moment it is asked for and is never persisted — it has nowhere to be
/// persisted *to*, which is the point.
///
/// # Why it holds so little
///
/// `ADR-007` §4.3: *"Stage 0 configures almost nothing. The commitment at
/// Stage 0 is only that configuration is typed, that there is one document, and
/// that observed state is never written into it."* The design record names
/// wireless credentials, a fleet endpoint and a device identity as the
/// provisioning file's eventual contents (`ADR-019` §3) and a hostname as an
/// example machine-layer value (`ADR-019` §3.2) — every one of them belonging
/// to Stage 2 or later. **No Stage 0 field is specified anywhere in the
/// record**, so none is invented here.
///
/// The one exception is [`Document::services`], which `ADR-017` §4.1 ruled into
/// existence on 2026-08-22 — its consequences list says in as many words that
/// *"`ADR-007`'s schema gains a services table carrying enable/disable and
/// per-service settings"*. See [`crate::services`] for the boundary it enforces
/// and for what it deliberately refuses to carry.
///
/// **Amended 2026-08-23: [`Document::network`].** The maintainer ruled
/// option 2 — a `[network]` table plus first-boot provisioning
/// — so the paragraph above is now one stage out of date about *when* an
/// address arrives, and correctly so: `ADR-004` §6's provisioning file is the
/// only way to tell a headless board anything on its first boot, and it is
/// read as one of these documents. See [`crate::network`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    version: SchemaVersion,
    services: Vec<Service>,
    network: Vec<Interface>,
    /// `ADR-031` §7.1's `[volumes.<name>]`. Empty is a machine with only its
    /// built-in volume, which is every Rivet today — and it is the same
    /// state as a `[volumes]` table declaring none, deliberately: `ADR-031`
    /// §7.5 makes a declaration a statement about what should be attached, so
    /// asking for nothing and saying nothing are one thing.
    volumes: Vec<Volume>,
    /// Where `rivet-mcpd` listens — `crate::control_plane`.
    ///
    /// **`None` is the loopback default and not `off`.** Whether the service
    /// runs at all is `[services.rivet-mcpd]`'s `enabled`, and the two are kept
    /// apart deliberately: *where* it listens and *whether* it listens are
    /// different questions.
    control_plane: Option<ControlPlane>,
    /// What the boot screen may say — `crate::splash`.
    ///
    /// **`None` means the document did not mention it**, which is not "show
    /// nothing": see [`Document::effective_splash`].
    splash: Option<Splash>,
    /// The statically-set nameservers — `crate::resolver`. `None` when the
    /// document has no `[network.resolver]`, which is what leaves a DHCP lease
    /// free to answer instead.
    resolver: Option<Resolver>,

    /// Which application holds the display — `crate::kiosk`.
    ///
    /// **Absent means no application takes the screen**, which is what every
    /// machine built before 2026-08-31 did and is not the same as naming one
    /// that is not installed. See [`Document::auto_run`].
    kiosk: Option<Kiosk>,
    /// What this machine calls itself — `rivet_core::hostname`, ruled
    /// 2026-08-26.
    ///
    /// **`None` is not "no name"**, and it is the third field in this struct to
    /// carry that distinction. An absent key is *the operator said nothing*, so
    /// the machine derives one; see [`Document::hostname`]. There is no way to
    /// ask for a machine with no name at all, deliberately: `(none)` is what a
    /// Linux kernel holds when nobody has called `sethostname(2)`, and it is a
    /// state rather than a request.
    hostname: Option<Hostname>,
    /// What local time this machine keeps — `rivet_core::timezone`, ruled
    /// 2026-09-02.
    ///
    /// **`None` is UTC, and unlike [`Self::hostname`] nothing is derived.** A
    /// machine cannot say which zone it is in from anything it can read about
    /// itself — a serial number, a MAC address and a kernel command line all
    /// know nothing about geography — so there is no per-machine fact to fall
    /// back to and inventing one would be guessing. UTC is what musl keeps when
    /// it has no tzfile (`rivet_core::timezone::UNCONFIGURED`), so absence here
    /// means the machine behaves exactly as every Rivet did before this key
    /// existed.
    ///
    /// **A name, not an offset.** See `rivet_core::timezone` for why the schema
    /// takes an IANA identifier rather than a POSIX `TZ` string, and for what
    /// this type deliberately cannot check.
    timezone: Option<Timezone>,
    /// `[display]` - the hours the screen is lit, ruled 2026-09-02
    /// (`ADR-039` §3.1).
    ///
    /// **`None` is a screen that is never switched off by schedule**, which is
    /// what every Rivet did before this key existed. It is not "unknown" and
    /// nothing is derived: a machine cannot infer a building's opening hours
    /// from anything it can read about itself, and guessing would switch
    /// somebody's screens off.
    ///
    /// **The pair is held as a pair.** `crate::display` argues why `off`
    /// without `on` is refused rather than defaulted, and
    /// `rivet_core::display::Hours` will not construct a window of zero width -
    /// so the states this field can hold are exactly the states that mean
    /// something.
    display: Option<Hours>,
    /// `[applications.<name>]` — what each application is set to, `ADR-038` §4.
    ///
    /// **The one region whose schema this crate does not own.** Its keys come
    /// from a signed artifact's manifest, so `crate::applications` carries
    /// whatever well-formed `key = "value"` it is given — including for an
    /// application this machine has never installed, which is exactly what
    /// provisioning before first boot requires (`ADR-034` §5.1).
    ///
    /// **And a bad key here is never fatal.** An
    /// unrecognised key is an outright refusal everywhere else in this document;
    /// `ADR-038` §4.4 inverts it here, because refusing to boot over a
    /// misspelled brightness key would be a catastrophic answer to a trivial
    /// mistake (`PLAN.md` §104.19). What could not be carried is reported
    /// through `Applications::ignored`.
    applications: Applications,

    /// `[repos]` — which git repositories this machine serves. Ruled
    /// 2026-08-26.
    ///
    /// **Absent and empty are one state**: *this machine
    /// serves no repositories*. There is no leave-the-directory-alone case to
    /// distinguish, because undeclaring never deletes anything anyway -
    /// `crate::repos` has the argument, and it is the sharpest instance of it
    /// in the schema.
    repos: Vec<Repo>,

    /// `[carry]` — which published version of a carried product this
    /// machine's card must carry. `crate::carry`, `.rulings/` R-159 R20.
    ///
    /// **Absent and empty are one state**: every carried product resolves
    /// unpinned. See `crate::carry`'s header for why this is a table of its
    /// own rather than a fourth key on `[services.<name>]`.
    carry: Vec<Carry>,

    /// `mode` — whether this machine drives a screen. `crate::machine_mode`,
    /// added 2026-09-13 (plan §5.2 item 2, `.rulings/` R-162).
    ///
    /// **`None` means the operator said nothing, and nothing is derived**:
    /// see the field's own module for why this crate composes nothing else
    /// from it. It is recorded intent, checked against `[kiosk]` by a reader
    /// rather than by this crate.
    mode: Option<MachineMode>,

    /// `fleet-role` — which of `crate::fleet_role::FleetRole`'s two roles
    /// this machine is. Added 2026-09-13 (plan §5.2 item 2, `.rulings/`
    /// R-162).
    ///
    /// **`None` means the operator said nothing**, and applies no role
    /// default at all — the same absence rule `rivetos_register::Device::
    /// fleet_role`'s own field carries, mirrored here. See
    /// [`Document::effective_control_plane`] and [`Document::role_services`]
    /// for what a role, once stated, supplies as a default.
    fleet_role: Option<FleetRole>,

    /// `[dns]` — the name service's zone and records, `crate::dns`.
    ///
    /// **`None` means the document states no zone**, which is every machine
    /// that does not run the name service.
    dns: Option<Dns>,
}

impl Document {
    /// Every key this document declares at the top level, in the order the
    /// schema lists them.
    ///
    /// One list, used both by the parser and by the error it produces
    /// (`ADR-011`: exactly one thing is authoritative, and it says so at the
    /// moment the answer is needed).
    ///
    /// **Reordered 2026-09-13** (plan §5.2 item 1, `.rulings/` R-162) to match
    /// `cmd/rivet-provision`'s own question order — `hostname`, `network`
    /// (`Addressing`/`Address`/`Gateway`), `mode` (`Mode`), `fleet-role`
    /// (`Fleet`), `timezone`, then `kiosk` (`Auto-run`) — so that a generated
    /// interview (plan §5.2 item 3) can walk this array and reproduce today's
    /// order without a second, hand-maintained list. The keys with no question
    /// of their own keep their prior relative order, appended after. This is
    /// a reordering of a `const` array only: it changes nothing about what a
    /// document may contain or how the parser reads one, so every document
    /// written against the old order still parses (`schema.rs`'s
    /// `an_existing_document_with_no_new_keys_still_parses_unchanged` is the
    /// guard).
    pub const DECLARED_KEYS: &'static [&'static str] =
        &[
            "version", "hostname", "network", "mode", "fleet-role", "timezone",
            "kiosk",
            "services", "volumes",
            "repos", "control-plane", "splash", "display",
            "applications",
            "carry",
            "dns",
        ];

    /// The schema version this document declares.
    pub const fn version(&self) -> SchemaVersion {
        self.version
    }

    /// Every service the operator has configured, ordered by name.
    ///
    /// **Ordered, because `ADR-007` §3.1 requires that two machines in the same
    /// state produce byte-identical documents.** The order services appear in
    /// the text is a fact about the text, not about the state, so it is not
    /// preserved — the store is the authority and the store is sorted.
    ///
    /// Empty is a complete answer: it means the operator has configured no
    /// service, not that services are unknown. What each named service *is*
    /// lives in the image or in a signed manifest (`ADR-017` §4.1) and this
    /// crate cannot see it, so nothing here should be read as a claim that a
    /// named service exists.
    pub fn services(&self) -> &[Service] {
        &self.services
    }

    /// The operator's configuration for one service, if the document carries
    /// any.
    pub fn service(&self, name: &str) -> Option<&Service> {
        self.services
            .binary_search_by(|s| s.name().as_str().cmp(name))
            .ok()
            .and_then(|i| self.services.get(i))
    }

    /// The enabled services that are not image services — the ones a card
    /// must CARRY as signed payloads rather than merely switch on.
    ///
    /// `ADR-050` §5.1 decides this by the machine's role: a fleet server's
    /// document enables `rivet-fleetd`, a kiosk's enables
    /// its auto-run application, and `cmd/rivet-provision`'s own
    /// `services_for` is what puts each one in `[services.<name>] enabled =
    /// true` in the first place (`PLAN-Feature-Set-2026-09-12` §4). Rather
    /// than re-deriving the role here — which this document does not even
    /// have a field for — this reads the one fact the document already states
    /// unambiguously: which services the operator wants running.
    ///
    /// **`rivet_core::service::IMAGE_SERVICE_NAMES` is the line**, the same
    /// list an installed application's name is already refused against. A
    /// name on it — `rivet-proxyd` among them, `ADR-060` — ships inside
    /// `/system` and needs nothing carried; a name off it — `rivet-fleetd`, an
    /// addon's kiosk application — is installed rather than baked in, so
    /// enabling it without carrying its
    /// payload would compose a document promising a service the card cannot
    /// start on its own.
    ///
    /// Ordered by name, on [`Document::services`]'s own footing: the source
    /// list is already sorted, so filtering it preserves that order rather
    /// than needing a second sort.
    #[must_use]
    pub fn carried_services(&self) -> Vec<&Service> {
        self.services
            .iter()
            .filter(|s| s.enabled())
            .filter(|s| {
                !rivet_core::service::IMAGE_SERVICE_NAMES.contains(&s.name().as_str())
            })
            .collect()
    }

    /// Every product this document pins a version for, ordered by name.
    ///
    /// `crate::carry`. Empty is a complete answer: the operator has pinned
    /// nothing, not that pinning is unavailable.
    #[must_use]
    pub fn carry(&self) -> &[Carry] {
        &self.carry
    }

    /// Which version `product` is pinned to, if this document names one.
    ///
    /// **Read by `cmd/rivet-image --fleet-repository`, and by nothing that
    /// runs on a booted machine** — `crate::carry`'s header argues why: a pin
    /// decides what a NEW card carries, and carrying has already happened by
    /// the time anything on that card's machine is running. A name this
    /// document does not carry (`Document::carried_services` is the
    /// authority on that) may still appear here inertly — see `crate::carry`
    /// for why that is allowed rather than refused.
    #[must_use]
    pub fn carried_version(&self, product: &str) -> Option<&str> {
        self.carry.iter().find(|c| c.product().as_str() == product).map(Carry::version)
    }

    /// Every interface the operator has configured, ordered by name.
    ///
    /// Ordered for the same reason [`Document::services`] is: `ADR-007` §3.1
    /// requires that two machines in the same state produce byte-identical
    /// documents, and the order interfaces appear in the text is a fact about
    /// the text.
    ///
    /// Empty is a complete answer: the operator has configured no interface.
    /// It is not a claim that the machine has none — that is the kernel's to
    /// answer and this crate performs no I/O (`ADR-014` §3.1).
    /// Every declared volume, in name order. `ADR-031` §7.1.
    ///
    /// **A declaration, not an attachment.** Whether the GUID resolves to a
    /// device is the machine's answer at the moment it is asked (`ADR-031`
    /// §4.2) and this crate performs no I/O; `ADR-031` §6 is what an absent
    /// volume means.
    #[must_use]
    pub fn volumes(&self) -> &[Volume] {
        &self.volumes
    }

    pub fn network(&self) -> &[Interface] {
        &self.network
    }

    /// The operator's configuration for one interface, if the document carries
    /// any.
    pub fn interface(&self, name: &str) -> Option<&Interface> {
        self.network
            .binary_search_by(|i| i.name().as_str().cmp(name))
            .ok()
            .and_then(|i| self.network.get(i))
    }

    /// Parse a document from TOML text.
    ///
    /// **This is not I/O.** Opening the boot partition and reading the bytes is
    /// I/O and belongs to an adapter at the composition root; turning those
    /// bytes into these types belongs here (`ADR-014` §3).
    ///
    /// Every refusal names what was wrong — see [`ConfigError`]. Nothing is
    /// defaulted around: `ADR-007` §3.6.5, *Rivet does not accept an input it
    /// intends to discard*.
    pub fn parse(src: &str) -> Result<Self, ConfigError> {
        crate::parse::document(src)
    }

    /// Render the document as the one text that represents it.
    ///
    /// **This is not I/O either**, and for the same reason [`Document::parse`]
    /// is not: producing bytes is this crate's, writing them to `/state/config`
    /// is `rivet-store`'s (`ADR-014` §3).
    ///
    /// `ADR-007` §3.1 requires that "two machines with the same state produce
    /// byte-identical documents", so this is a total function of the value and
    /// of nothing else — no clock, no hostname, no build identifier. It cannot
    /// fail: every value in a [`Document`] was validated on the way in, and
    /// there is no state this type can hold that has no representation. See
    /// [`crate::serialise`] for what is derived rather than quoted from the
    /// record, and for the property tests.
    pub fn to_toml(&self) -> String {
        crate::serialise::document(self)
    }

    /// Build a document directly, for a caller that is not reading text.
    ///
    /// The **named** tables are sorted here rather than being trusted from the
    /// caller, so that the binary searches above and `ADR-007` §3.1's canonical
    /// ordering hold however the document was built.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        version: SchemaVersion,
        mut services: Vec<Service>,
        mut network: Vec<Interface>,
        mut volumes: Vec<Volume>,
        control_plane: Option<ControlPlane>,
        splash: Option<Splash>,
        resolver: Option<Resolver>,
        kiosk: Option<Kiosk>,
        hostname: Option<Hostname>,
        timezone: Option<Timezone>,
        display: Option<Hours>,
        applications: Applications,
        mut repos: Vec<Repo>,
        mut carry: Vec<Carry>,
        // Added 2026-09-13 (plan §5.2 item 2, `.rulings/` R-162), appended
        // rather than interleaved: `Document::new` is called from exactly two
        // places in this crate (`crate::parse` and this module's own tests),
        // so widening the end of the list is a smaller diff than reordering
        // every existing call site for a cosmetic match to `DECLARED_KEYS`.
        mode: Option<MachineMode>,
        fleet_role: Option<FleetRole>,
        dns: Option<Dns>,
    ) -> Self {
        services.sort_by(|a, b| a.name().cmp(b.name()));
        network.sort_by(|a, b| a.name().cmp(b.name()));
        // Sorted for the same reason the other two are: a document read and
        // written back must be byte-identical (`ADR-007` §3.1), and a TOML
        // table has no order a parser is obliged to preserve.
        volumes.sort_by(|a, b| a.name().cmp(b.name()));
        // And repositories, for the third time and the same reason. Left
        // unsorted once until a test caught it: `mut` on the parameter is an
        // intention rather than a guarantee, which is what
        // `repos_are_ordered_by_name_however_they_were_written` is for.
        repos.sort_by(|a, b| a.name().cmp(b.name()));
        // And carry entries, on the identical footing: `ADR-007` §3 again.
        carry.sort_by(|a, b| a.product().cmp(b.product()));
        Self {
            version,
            services,
            network,
            volumes,
            control_plane,
            splash,
            resolver,
            kiosk,
            hostname,
            timezone,
            display,
            applications,
            repos,
            carry,
            mode,
            fleet_role,
            dns,
        }
    }

    /// The `[dns]` table as written, or `None` when the document had none —
    /// `crate::dns`.
    #[must_use]
    pub const fn dns(&self) -> Option<&Dns> {
        self.dns.as_ref()
    }

    /// Where `rivet-mcpd` should listen, as the operator stated it.
    ///
    /// **`None` means the operator said nothing** and the caller applies
    /// [`ControlPlane::default`] — see [`Document::effective_control_plane`].
    pub const fn control_plane(&self) -> Option<&ControlPlane> {
        self.control_plane.as_ref()
    }

    /// Where `rivet-mcpd` listens, with the default applied.
    ///
    /// **The one place that default is written**, which is `ADR-011` in the
    /// small: a caller that invented its own would be a second answer to
    /// *where does the control plane listen*.
    ///
    /// **Consults [`Document::fleet_role`] only when the document says
    /// nothing explicit** — `[control-plane]` absent — and never otherwise:
    /// `ADR-004` §6's default is a role's, and `SPEC-05` §5's own words for
    /// the identical register field are *"the device wins"*. A document that
    /// states `[control-plane] listen = "…"` explicitly is read exactly as
    /// before, whatever role it also names, so this cannot disagree with an
    /// operator who wrote the table themselves (`crate::fleet_role`'s module
    /// documentation).
    #[must_use]
    pub fn effective_control_plane(&self) -> ControlPlane {
        if let Some(explicit) = &self.control_plane {
            return explicit.clone();
        }
        match crate::fleet_role::listen_default(self.fleet_role) {
            Some(role_default) => ControlPlane::new(role_default.to_owned()),
            None => ControlPlane::default(),
        }
    }

    /// What this document says about which of `crate::fleet_role::FleetRole`'s
    /// two roles this machine is, as the operator stated it.
    ///
    /// **`None` means the operator said nothing**, and applies no role
    /// default at all — the same absence rule
    /// `rivetos_register::Device::fleet_role` already carries.
    #[must_use]
    pub const fn fleet_role(&self) -> Option<FleetRole> {
        self.fleet_role
    }

    /// Every service name this document's role turns on **when the document
    /// says nothing else about that service** — `ADR-050` §5's role table,
    /// read through [`Document::fleet_role`].
    ///
    /// **A default a caller layers under `[services.<name>].enabled`**, on
    /// [`Document::effective_control_plane`]'s footing: this returns the
    /// role's names and never consults [`Document::services`] itself, so an
    /// explicit `[services.<name>] enabled = false` always wins wherever a
    /// caller applies the layering — mirroring
    /// `cmd/rivet-fleet/src/compose.rs::services`'s identical merge for the
    /// register. See `crate::fleet_role`'s module documentation for why the
    /// document itself does not perform that merge: `[carry]`,
    /// `Document::carried_services` and every other reader of
    /// `[services.<name>]` keeps reading exactly what the operator wrote,
    /// unchanged by this method's existence.
    #[must_use]
    pub const fn role_services(&self) -> &'static [&'static str] {
        crate::fleet_role::role_services(self.fleet_role)
    }

    /// What this document says about whether it drives a screen, as the
    /// operator stated it. `crate::machine_mode`.
    ///
    /// **`None` means the operator said nothing**, and nothing is derived —
    /// see `crate::machine_mode`'s module documentation for why this crate
    /// composes nothing else from it.
    #[must_use]
    pub const fn mode(&self) -> Option<MachineMode> {
        self.mode
    }

    /// The `[network.resolver]` table as written, or `None` when the document
    /// had none — which is the signal that a DHCP lease may supply the answer.
    #[must_use]
    pub fn resolver(&self) -> Option<&Resolver> {
        self.resolver.as_ref()
    }

    /// The nameservers this document sets, or an empty slice.
    ///
    /// Flattens the two ways of saying nothing — no table, and a table naming
    /// none — because every caller treats them identically: neither is an
    /// answer, and both leave the lease to it.
    #[must_use]
    pub fn nameservers(&self) -> &[core::net::Ipv4Addr] {
        self.resolver
            .as_ref()
            .map_or(&[], crate::resolver::Resolver::nameservers)
    }

    /// The `[kiosk]` table as written, or `None` when the document had none.
    #[must_use]
    pub fn kiosk(&self) -> Option<&Kiosk> {
        self.kiosk.as_ref()
    }

    /// **Which application should hold the display**, if any.
    ///
    /// `None` covers both *no `[kiosk]` table* and *a table that named nothing*,
    /// because the two mean the same thing to a supervisor: no application takes
    /// the screen. They differ only in the bytes, which `crate::serialise`
    /// preserves.
    #[must_use]
    pub fn auto_run(&self) -> Option<&str> {
        self.kiosk.as_ref().and_then(Kiosk::auto_run)
    }

    /// What the boot screen may say, with the document's silence resolved.
    ///
    /// **Absent means BOTH LINES**, not neither — `crate::splash` has the
    /// argument: a headless board whose address came from DHCP is otherwise a
    /// machine nobody can find without reading a router's lease table.
    #[must_use]
    pub fn effective_splash(&self) -> Splash {
        self.splash.unwrap_or_default()
    }

    /// What the document SAID about the boot screen, or `None` if it was
    /// silent. [`Document::effective_splash`] is the resolved answer.
    #[must_use]
    pub const fn splash(&self) -> Option<Splash> {
        self.splash
    }

    /// What this machine calls itself, as the operator stated it.
    ///
    /// **`None` means the operator said nothing**, and the caller derives a
    /// name from something only that machine can say about itself. The fallback
    /// is not here because it needs a fact this crate does not have — see
    /// `rivet_core::hostname::derived`, which takes it as an argument for
    /// exactly that reason.
    pub const fn hostname(&self) -> Option<&Hostname> {
        self.hostname.as_ref()
    }

    /// What local time this machine keeps, as the operator stated it.
    ///
    /// **`None` means UTC and not "unknown"**, which is the one place in this
    /// type where an absent optional resolves to a value rather than to a
    /// fallback somebody has to compute. `rivet_core::timezone::UNCONFIGURED`
    /// is that value, and it is musl's own — a machine with no `/etc/localtime`
    /// keeps UTC whether or not anything asked it to, so there is nothing for a
    /// caller to decide. See the field for why nothing is derived here the way
    /// a hostname is.
    pub const fn timezone(&self) -> Option<&Timezone> {
        self.timezone.as_ref()
    }

    /// The hours this machine's screen is lit, or `None` for always.
    ///
    /// **`None` is "no schedule", not "unknown"** - see the field. A caller
    /// asking *should the screen be on right now* treats absence as yes, which
    /// is what `rivet_sdk::display` does and the only safe direction: a bug
    /// that leaves a screen lit is visible, and one that leaves a building dark
    /// at nine in the morning is a phone call.
    pub const fn display(&self) -> Option<&Hours> {
        self.display.as_ref()
    }

    /// What each application is set to — `ADR-038` §4.
    ///
    /// Empty on a machine nobody has configured, which is every machine before
    /// somebody writes a table. See the field for why a bad key in here does
    /// not refuse the document.
    pub const fn applications(&self) -> &Applications {
        &self.applications
    }

    /// Every repository this machine serves, ordered by name.
    ///
    /// **Ordered by name and identified by id**, which is not a contradiction:
    /// the order is the document's canonical form (`ADR-007` 3.1) and the
    /// identity is `crate::repos::RepoId`. A projection that matched on the
    /// name would rename by destroying and recreating.
    #[must_use]
    pub fn repos(&self) -> &[Repo] {
        &self.repos
    }

    /// **The name service's projection of this document** — `ADR-047` §6.
    ///
    /// A document holding this one's `version` and `[dns]` table, with
    /// `hostname` set to the name the machine actually carries, and nothing
    /// else. `rivet-dnsd` runs confined and may not read the machine document,
    /// so the supervisor renders this with [`Document::to_toml`] into the
    /// service's home and the service reads it with [`Document::parse`]: one
    /// grammar and one reader for the table, in both directions.
    ///
    /// **`hostname` is the machine's name, not the operator's statement.** A
    /// machine whose document names none still has a derived one, and that is
    /// the name the zone's SOA and NS records must carry, so the caller passes
    /// what it set rather than what the document said. `None` is a machine
    /// with no name at all, which the service refuses rather than inventing one.
    ///
    /// A document with no `[dns]` table projects to one without it, and the
    /// service then refuses with its own reason.
    #[must_use]
    pub fn dns_projection(&self, hostname: Option<&Hostname>) -> Self {
        Self {
            version: self.version,
            hostname: hostname.cloned(),
            dns: self.dns.clone(),
            ..Self::default()
        }
    }
}

/// The schema-defaults layer — layer 1 of `ADR-007` §3.4, whose source is
/// "the Rust types".
///
/// It is the only layer that needs no store behind it, because it *is* the
/// types. The other three (image defaults, fleet, machine) are persisted and
/// are Stage 2–3 work per `ADR-007` §5; nothing here merges layers.
///
/// **Every optional table here is `None`, and that is the safe direction rather
/// than an omission.** `cmd/rivet-sysd` falls back to this value when a machine
/// has no stored document or when the stored one cannot be read (`ADR-018` §8),
/// so a default that *asserted* something would let a card fault change a
/// machine's behaviour at the moment somebody needed to repair it. `None` means
/// *nobody said*, and the field notes below say what each one leaves alone.
impl Default for Document {
    fn default() -> Self {
        // No volumes, and nothing else declared: a machine that
        // has lost its document must not have applications quietly relocated
        // to storage nobody declared. It has its built-in volume and nothing
        // else, which is a Rivet that works.
        Self::new(
            SCHEMA_VERSION,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            // splash: the document said nothing, so both lines are drawn.
            None,
            // resolver: no nameservers. A default that named one would make a
            // machine with a lost document resolve through somebody's server on
            // the strength of a missing file.
            None,
            // kiosk: nothing takes the display, which is every machine before
            // 2026-08-31 and is what a lost document must fall back to. A
            // default that named an application would give the screen away on
            // the strength of a file that is missing.
            None,
            // timezone: UTC, which is what musl keeps with no /etc/localtime
            // and is therefore not a choice this default is making. A machine
            // that has lost its document keeps the clock every Rivet kept
            // before the key existed.
            None,
            // display: no schedule, so the screen is never switched off. The
            // safe direction and the one every Rivet had before 2026-09-02: a
            // machine that has lost its document must not switch a building's
            // screens off on the strength of a missing file, and a screen that
            // stays lit when it should not is visible to somebody.
            None,
            // applications: nothing configured. A machine that has lost its
            // document must not hand an application a setting nobody wrote.
            Applications::default(),
            // repos: none served. Declaring one creates it, so a default that
            // named any would make a lost document into a `git init`.
            Vec::new(),
            // carry: nothing pinned. A default that pinned a version would
            // outlive the release it named the moment the fleet published a
            // newer one, on a machine with no document to say otherwise.
            Vec::new(),
            // mode: nothing said. A machine that has lost its document must
            // not have a screen policy invented on the strength of a missing
            // file — see `crate::machine_mode` for why this composes nothing
            // anyway.
            None,
            // fleet_role: nothing said, which applies no role default at all
            // — `Document::effective_control_plane` and
            // `Document::role_services` both fall through to their own
            // no-role answers, exactly as every Rivet before this key
            // existed.
            None,
            // dns: no zone. A machine that has lost its document answers for
            // no names rather than for names nobody wrote.
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::network::Addressing;
    use rivet_core::net::{InterfaceName, Ipv4Cidr};

    fn service(n: &str) -> Service {
        Service::new(
            crate::services::ServiceName::new(n).expect("a valid fixture name"),
            true,
            None,
            None,
            false,
        )
    }

    fn interface(n: &str) -> Interface {
        Interface::new(
            InterfaceName::new(n).expect("a valid fixture name"),
            Addressing::Dhcp,
            None,
        )
    }

    #[test]
    fn the_schema_defaults_layer_is_at_the_current_version() {
        assert_eq!(Document::default().version(), SCHEMA_VERSION);
    }

    #[test]
    fn the_declared_keys_are_the_ones_the_type_actually_has() {
        // If a field is added to `Document` without being added here, the
        // parser will refuse a document that sets it, and the failure will look
        // like a schema bug rather than an omission. This is the cheap guard
        // that keeps the two in step (`AGENTS.md` §4).
        assert_eq!(
            Document::DECLARED_KEYS,
            [
                "version", "hostname", "network", "mode", "fleet-role", "timezone",
                "kiosk",
                "services", "volumes",
                "repos",
                "control-plane",
                "splash",
                "display",
                "applications",
                "carry",
                "dns"
            ]
        );
    }

    // ---- `mode` and `fleet-role` — plan §5.2 item 2, `.rulings/` R-162 -------

    #[test]
    fn a_document_with_no_mode_and_no_fleet_role_answers_none_for_both() {
        assert_eq!(Document::default().mode(), None);
        assert_eq!(Document::default().fleet_role(), None);
        let doc = crate::parse::document("version = 1\n").expect("the minimal document");
        assert_eq!(doc.mode(), None);
        assert_eq!(doc.fleet_role(), None);
    }

    /// **The hard constraint**: a document shaped like one written before
    /// these two keys existed — `rivet-01`'s own shape, without reading the
    /// real board or the real register — still parses, and carries neither
    /// key. Modelled on `the_example_document_a_card_operator_copies_actually_
    /// parses` below: a hand-composed fixture standing in for a real one.
    #[test]
    fn a_document_shaped_like_one_written_before_these_keys_existed_still_parses() {
        let src = "version = 1\n\
                    hostname = \"rivet-01\"\n\
                    timezone = \"America/Chicago\"\n\
                    \n[network.eth0]\n\
                    method = \"dhcp\"\n\
                    \n[services.rivet-mcpd]\n\
                    enabled = true\n\
                    \n[services.aura-renderer]\n\
                    enabled = true\n\
                    \n[kiosk]\n\
                    auto-run = \"aura-renderer\"\n";
        let doc = crate::parse::document(src).expect("a pre-existing document must still parse");
        assert_eq!(doc.mode(), None);
        assert_eq!(doc.fleet_role(), None);
        assert_eq!(doc.hostname().map(|h| h.as_str()), Some("rivet-01"));
        assert_eq!(doc.auto_run(), Some("aura-renderer"));
        // And it renders back to bytes that parse to the same value —
        // `ADR-007` §3 — so the new keys' absence is not merely tolerated on
        // the way in but preserved on the way out.
        let rendered = doc.to_toml();
        assert_eq!(crate::parse::document(&rendered).expect("re-parses"), doc);
        assert!(!rendered.contains("mode ="), "{rendered}");
        assert!(!rendered.contains("fleet-role"), "{rendered}");
    }

    #[test]
    fn a_mode_and_a_fleet_role_round_trip() {
        let src = "version = 1\nmode = \"kiosk\"\nfleet-role = \"server\"\n";
        let doc = crate::parse::document(src).expect("a document naming both");
        assert_eq!(doc.mode(), Some(crate::machine_mode::MachineMode::Kiosk));
        assert_eq!(doc.fleet_role(), Some(crate::fleet_role::FleetRole::Server));
        let rendered = doc.to_toml();
        assert_eq!(crate::parse::document(&rendered).expect("re-parses"), doc);
    }

    /// `ADR-004` §6, mirrored: a server or a member's document, with no
    /// EXPLICIT `[control-plane]`, resolves to every interface; a
    /// stand-alone's, or one naming no role, keeps loopback.
    #[test]
    fn effective_control_plane_falls_back_to_the_roles_default_only_when_absent() {
        let server = crate::parse::document("version = 1\nfleet-role = \"server\"\n")
            .expect("a server document");
        assert_eq!(server.effective_control_plane().listen(), "0.0.0.0:7777");

        let member = crate::parse::document("version = 1\nfleet-role = \"member\"\n")
            .expect("a member document");
        assert_eq!(member.effective_control_plane().listen(), "0.0.0.0:7777");

        let stand_alone = crate::parse::document("version = 1\nfleet-role = \"stand-alone\"\n")
            .expect("a stand-alone document");
        assert_eq!(
            stand_alone.effective_control_plane().listen(),
            crate::control_plane::DEFAULT_LISTEN
        );

        let roleless = crate::parse::document("version = 1\n").expect("a roleless document");
        assert_eq!(roleless.effective_control_plane().listen(), crate::control_plane::DEFAULT_LISTEN);
    }

    /// **The ratchet's regression guard.** A document written before
    /// `FleetRole::StandAlone` was removed may still carry
    /// `fleet-role = "stand-alone"` on disk (`rivetos_register`'s own copy
    /// went first, and `a_record_with_the_retired_stand_alone_role_still_
    /// loads_as_none` is its sibling test). It named no behaviour of its own
    /// even when the variant existed — every caller that matched on the role
    /// paired it with `None` — so the document must still parse, as
    /// `fleet_role: None`, rather than refuse; and having parsed that way, it
    /// renders back out with no `fleet-role` key at all, the same as any
    /// other document that never named a role.
    #[test]
    fn a_document_with_the_retired_stand_alone_fleet_role_still_parses_as_none() {
        let doc = crate::parse::document("version = 1\nfleet-role = \"stand-alone\"\n")
            .expect("a document with the retired role still loads");
        assert_eq!(doc.fleet_role(), None);
        let rendered = doc.to_toml();
        assert!(!rendered.contains("fleet-role"), "{rendered}");
    }

    /// **The device wins** — `SPEC-05` §5's own words for the register's
    /// identical merge, mirrored: an EXPLICIT `[control-plane]` is read
    /// exactly as written, whatever role the document also names, so a role
    /// default can never contradict an operator who wrote the table
    /// themselves.
    #[test]
    fn an_explicit_control_plane_table_always_wins_over_the_role_default() {
        let src = "version = 1\nfleet-role = \"server\"\n\n[control-plane]\nlisten = \"127.0.0.1:9\"\n";
        let doc = crate::parse::document(src).expect("an explicit override beside a role");
        assert_eq!(doc.effective_control_plane().listen(), "127.0.0.1:9");
    }

    /// `ADR-050` §5's role table, read back through the document rather than
    /// the register.
    #[test]
    fn role_services_matches_the_role_table() {
        let server = crate::parse::document("version = 1\nfleet-role = \"server\"\n")
            .expect("a server document");
        assert_eq!(server.role_services(), &["rivet-fleetd", "rivet-mdnsd", "rivet-proxyd"]);

        let member = crate::parse::document("version = 1\nfleet-role = \"member\"\n")
            .expect("a member document");
        assert_eq!(member.role_services(), &["rivet-mdnsd", "rivet-proxyd"]);

        let roleless = crate::parse::document("version = 1\n").expect("a roleless document");
        assert!(roleless.role_services().is_empty());
    }

    #[test]
    fn a_document_with_no_services_configured_is_a_complete_answer() {
        assert!(Document::default().services().is_empty());
        assert_eq!(Document::default().service("logd"), None);
    }

    #[test]
    fn a_document_with_no_network_configured_is_a_complete_answer() {
        assert!(Document::default().network().is_empty());
        assert_eq!(Document::default().interface("eth0"), None);
    }

    /// **A machine document without a role carries nothing** — `Document::default()`
    /// enables no service at all, so there is no name to resolve out of a fleet
    /// repository (`PLAN-Feature-Set-2026-09-12` §4).
    #[test]
    fn a_document_with_no_services_enabled_carries_nothing() {
        assert!(Document::default().carried_services().is_empty());
        let doc = crate::parse::document("version = 1\n").expect("the minimal document");
        assert!(doc.carried_services().is_empty());
    }

    /// **`ADR-050` §5.1 and `ADR-060`: a fleet server carries `fleetd`, and the
    /// image services `proxyd` and `mdnsd` need nothing carried.** The document
    /// does not name a role; this is what `cmd/rivet-provision`'s `services_for`
    /// leaves for `carried_services` to read back.
    #[test]
    fn a_fleet_servers_document_carries_fleetd_and_not_the_image_services() {
        let src = "version = 1\n\
                    [services.rivet-fleetd]\nenabled = true\n\
                    [services.rivet-proxyd]\nenabled = true\n\
                    [services.rivet-mdnsd]\nenabled = true\n";
        let doc = crate::parse::document(src).expect("a server document");
        let names: Vec<&str> = doc.carried_services().iter().map(|s| s.name().as_str()).collect();
        assert_eq!(names, ["rivet-fleetd"]);
    }

    /// A service the operator has switched off is not something the card must
    /// carry, whatever it is named.
    #[test]
    fn a_disabled_service_is_not_carried_even_if_it_is_an_application() {
        let src = "version = 1\n[services.rivet-fleetd]\nenabled = false\n";
        let doc = crate::parse::document(src).expect("a document with fleetd disabled");
        assert!(doc.carried_services().is_empty());
    }

    /// An image service is never carried, whatever the document says about it
    /// — `rivet_core::service::IMAGE_SERVICE_NAMES` is the one list this reads
    /// against, and `rivet-mcpd` is on it.
    #[test]
    fn an_image_service_is_never_carried() {
        let src = "version = 1\n[services.rivet-mcpd]\nenabled = true\n";
        let doc = crate::parse::document(src).expect("a document enabling mcpd");
        assert!(doc.carried_services().is_empty());
    }

    /// A document with no `[carry]` table pins nothing — `crate::carry`'s
    /// empty-and-absent-are-one-state rule, and `.rulings/` R-159 R20's
    /// unpinned case starts here.
    #[test]
    fn a_document_with_no_carry_table_pins_nothing() {
        assert!(Document::default().carry().is_empty());
        assert_eq!(Document::default().carried_version("rivet-fleetd"), None);
        let doc = crate::parse::document("version = 1\n").expect("the minimal document");
        assert_eq!(doc.carried_version("rivet-fleetd"), None);
    }

    /// **R-159 R20's pinned case.** A `[carry.<product>]` entry's `version`
    /// round-trips through `carried_version`, and a name this document does
    /// not carry is a separate, absent answer.
    #[test]
    fn a_carry_entry_pins_the_version_carried_version_reads_back() {
        let src = "version = 1\n[carry.rivet-fleetd]\nversion = \"0.1.0\"\n";
        let doc = crate::parse::document(src).expect("a document pinning fleetd");
        assert_eq!(doc.carried_version("rivet-fleetd"), Some("0.1.0"));
        assert_eq!(doc.carried_version("rivet-proxyd"), None);
    }

    #[test]
    fn services_are_ordered_by_name_whatever_order_they_arrived_in() {
        // `ADR-007` §3.1 — identical state, identical bytes. The order in the
        // text is a fact about the text.
        let doc = Document::new(
            SCHEMA_VERSION,
            vec![service("logd"), service("apid"), service("netd")],
            Vec::new(),
            Vec::new(),
            None,
            None,
            // splash: the document said nothing, so both lines are drawn.
            None,
            // resolver: no nameservers set.
            None,
            // kiosk: nothing holds the display.
            None,
            // timezone: nothing said, so the machine keeps UTC — the one
            // absent optional here that resolves to a value rather than to a
            // derivation, because a machine cannot say which zone it is in.
            None,
            // display: no schedule, so the screen is never switched off.
            None,
            Applications::default(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            // dns: no zone.
            None,
        );
        let names: Vec<&str> = doc.services().iter().map(|s| s.name().as_str()).collect();
        assert_eq!(names, ["apid", "logd", "netd"]);
    }

    #[test]
    fn interfaces_are_ordered_by_name_whatever_order_they_arrived_in() {
        let doc = Document::new(
            SCHEMA_VERSION,
            Vec::new(),
            vec![interface("wlan0"), interface("eth0"), interface("lo")],
            Vec::new(),
            None,
            None,
            // splash: the document said nothing, so both lines are drawn.
            None,
            // resolver: no nameservers set.
            None,
            // kiosk: nothing holds the display.
            None,
            // timezone: nothing said, so the machine keeps UTC — the one
            // absent optional here that resolves to a value rather than to a
            // derivation, because a machine cannot say which zone it is in.
            None,
            // display: no schedule, so the screen is never switched off.
            None,
            Applications::default(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            // dns: no zone.
            None,
        );
        let names: Vec<&str> = doc.network().iter().map(|i| i.name().as_str()).collect();
        assert_eq!(names, ["eth0", "lo", "wlan0"]);
    }

    #[test]
    fn an_interface_can_be_looked_up_by_name_and_a_missing_one_is_none() {
        let doc = Document::new(
            SCHEMA_VERSION,
            Vec::new(),
            vec![
                interface("wlan0"),
                Interface::new(
                    InterfaceName::new("eth0").expect("valid"),
                    Addressing::Static {
                        address: Ipv4Cidr::parse("192.168.1.50/24").expect("valid"),
                        gateway: None,
                    },
                    None,
                ),
            ],
            Vec::new(),
            None,
            None,
            // splash: the document said nothing, so both lines are drawn.
            None,
            // resolver: no nameservers set.
            None,
            // kiosk: nothing holds the display.
            None,
            // timezone: nothing said, so the machine keeps UTC — the one
            // absent optional here that resolves to a value rather than to a
            // derivation, because a machine cannot say which zone it is in.
            None,
            // display: no schedule, so the screen is never switched off.
            None,
            Applications::default(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            // dns: no zone.
            None,
        );
        assert_eq!(
            doc.interface("eth0")
                .and_then(Interface::static_plan)
                .map(|p| p.address.to_string()),
            Some("192.168.1.50/24".to_owned())
        );
        assert_eq!(
            doc.interface("wlan0").map(|i| i.addressing().method()),
            Some(crate::network::Method::Dhcp)
        );
        assert_eq!(doc.interface("eth1"), None);
        assert_eq!(doc.interface(""), None);
    }

    #[test]
    fn a_service_can_be_looked_up_by_name_and_a_missing_one_is_none() {
        let doc = Document::new(
            SCHEMA_VERSION,
            vec![service("logd"), service("apid"), service("netd")],
            Vec::new(),
            Vec::new(),
            None,
            None,
            // splash: the document said nothing, so both lines are drawn.
            None,
            // resolver: no nameservers set.
            None,
            // kiosk: nothing holds the display.
            None,
            // timezone: nothing said, so the machine keeps UTC — the one
            // absent optional here that resolves to a value rather than to a
            // derivation, because a machine cannot say which zone it is in.
            None,
            // display: no schedule, so the screen is never switched off.
            None,
            Applications::default(),
            Vec::new(),
            Vec::new(),
            None,
            None,
            // dns: no zone.
            None,
        );
        for name in ["apid", "logd", "netd"] {
            assert_eq!(
                doc.service(name).map(|s| s.name().as_str()),
                Some(name),
                "{name} was not found"
            );
        }
        assert_eq!(doc.service("timed"), None);
        assert_eq!(doc.service(""), None);
    }

    /// **`.rulings/` R-210 D22 — PINNED so nobody later "fixes" this
    /// parser.** A `cpu` cap under the kernel's representable minimum is
    /// refused at the writers now (`cargo rivet package`, `apply edit`,
    /// `rivet-fleetd`'s commit path, `cargo rivet provision`), never here: a
    /// machine already booted with a document carrying one — written before
    /// this ruling, or by an older build — must keep reading its own
    /// document back rather than being stranded at boot.
    #[test]
    fn a_document_already_carrying_a_cpu_cap_under_the_kernel_minimum_still_parses() {
        let text = "version = 1\n[services.photo-index]\nenabled = true\ncpu = \"0.005\"\n";
        let doc = Document::parse(text).expect("a below-minimum cap must still parse");
        let limit = doc.service("photo-index").and_then(crate::services::Service::cpu).expect("cpu was set");
        assert!(limit.is_below_kernel_minimum(), "{limit}");
    }

    #[test]
    fn a_schema_version_reports_the_number_it_was_given() {
        assert_eq!(SchemaVersion::new(7).get(), 7);
        assert_eq!(SchemaVersion::new(7).to_string(), "7");
    }

    #[test]
    fn schema_versions_order_so_a_migration_can_tell_older_from_newer() {
        assert!(SchemaVersion::new(1) < SchemaVersion::new(2));
    }

    /// **The name service is handed its table and its name, and nothing else
    /// the document says** — `ADR-047` §6. The projection renders with the
    /// document's serialiser and parses back with its parser to the same
    /// value, so the service reads it exactly as it read the whole document.
    #[test]
    fn the_dns_projection_carries_version_hostname_and_dns_and_nothing_else() {
        let doc = Document::parse(
            "version = 1\nhostname = \"Kitchen-Pi\"\ntimezone = \"Europe/London\"\n\
             [services.rivet-dnsd]\nenabled = true\n\
             [applications.photo-index]\nalbum = \"holidays\"\n\
             [dns]\nzone = \"f.home.arpa\"\n[dns.records.printer]\naddress = \"10.0.0.40\"\n",
        )
        .expect("valid");
        let named = Hostname::new("dev-01").expect("a name");
        let projection = doc.dns_projection(Some(&named));

        assert_eq!(projection.version(), doc.version());
        assert_eq!(projection.hostname(), Some(&named), "the name the machine carries");
        assert_eq!(projection.dns(), doc.dns());
        assert!(projection.services().is_empty());
        assert_eq!(projection.applications().all().count(), 0);
        assert_eq!(projection.timezone(), None);

        let text = projection.to_toml();
        assert!(!text.contains("photo-index"), "{text}");
        assert!(!text.contains("timezone"), "{text}");
        let back = Document::parse(&text).expect("the projection is a document");
        assert_eq!(back, projection);
        assert_eq!(back.to_toml(), text, "byte-identical a second time");
    }

    /// No name, and no table, project as absences rather than as defaults —
    /// the service refuses each by its own reason.
    #[test]
    fn a_projection_without_a_name_or_a_table_says_so() {
        let doc = Document::parse("version = 1\nhostname = \"kitchen-pi\"\n").expect("valid");
        let projection = doc.dns_projection(None);
        assert_eq!(projection.hostname(), None, "the document's name is not the machine's");
        assert_eq!(projection.dns(), None);
        assert_eq!(Document::parse(&projection.to_toml()).expect("valid"), projection);
    }
}

#[cfg(test)]
mod example_document_tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::Document;

    /// **The example a card operator copies must parse**, and it caught a real
    /// defect on its first run. It is the whole of the guard against the
    /// example drifting out of the schema: nothing else reads that file, so a
    /// key removed here would leave a document on disk that this build refuses
    /// and nobody would find out until a card was written from it.
    #[test]
    fn the_provisioning_example_a_card_operator_copies_actually_parses() {
        let text = include_str!("../../tools/card/provisioning.example.toml");
        let document = Document::parse(text).expect("the example document must parse");
        assert!(
            !document.network().is_empty(),
            "the example names no interface, so a card built from it takes no address"
        );
    }
}
