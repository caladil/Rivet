//! Where a service's fields come from — the 2026-08-24 split, as types.
//!
//! # The ruling this module exists to enforce
//!
//! `ADR-007` §7.2 put a service's `listen` and `memory` in the **operator's
//! document**. `ADR-015` §3.2 put the same two keys in the **signed manifest**.
//! Both ADRs were accepted, and `ADR-011` says both cannot be the authority for
//! one fact. That is a contradiction in the record rather than a gap in it, and
//! the maintainer ruled it on 2026-08-24:
//!
//! > **The manifest owns what the application *is*** — restart policy,
//! > readiness contract, the sockets it listens on. **The document owns what
//! > the operator *limits*** — the memory cap. **Where both name `memory`, the
//! > lower wins.**
//!
//! **The direction is the test, and it is checkable rather than tasteful.**
//! `ADR-015` §4.1 protects an application's *behaviour* from being rewritten
//! outside the signature: a manifest that could be edited after signing could
//! ask for a privileged port or a wider grant, and the signature would still
//! verify. A memory cap runs the other way — it is the machine's owner
//! protecting the machine **from** the application — so nothing about it is
//! weakened by living outside the signature, and `ADR-007` §7.2 is right to
//! grant it.
//!
//! | Key | Was | Is |
//! | :-- | :-- | :-- |
//! | `restart`, `ready` | manifest (`ADR-017` §4.1) | manifest — unchanged |
//! | `listen` | **both** — `ADR-007` §7.2 *and* `ADR-015` §3.2 | **manifest only** |
//! | `memory` | **both** | **both, deliberately**: a request in the manifest, a cap in the document, and [`effective_memory`] takes the lower |
//! | `enabled` | document (`ADR-017` §6) | document — unchanged, and it has no manifest counterpart |
//!
//! `listen` is the key that actually moved, and it moved *away* from the
//! operator. That reverses `ADR-015` §5.2's sentence — *"'may listen on 443'
//! therefore becomes a line in the desired-state document"* — and the reversal
//! is deliberate rather than overlooked; `ADR-015` §3.2 and `ADR-017` §4.1 carry
//! the amendment, and `PLAN.md` §0.4.3 records the edge it leaves open.
//!
//! # What is here, and what is deliberately elsewhere
//!
//! **The manifest itself is [`crate::package::application`]'s**, not this
//! module's. It is a build product read out of a signed artifact, so the type
//! that parses it belongs with the verifier that authenticates it — and a
//! second manifest type here would be the `ADR-011` defect this module was
//! written to remove, reintroduced two files away from it.
//!
//! What this module owns is the part that is **neither** the artifact **nor**
//! the document, and therefore has nowhere else to live:
//!
//! * [`MemoryLimit`] — a `128M` with a **number** behind it. The artifact's
//!   `Memory` deliberately refuses to convert (see its own documentation),
//!   because comparing two limits is meaningless until the unit is settled.
//!   This is where the unit is settled, and §"Binary, not decimal" says on
//!   whose authority.
//! * [`effective_memory`] — *the lower wins*, which is the ruling's third
//!   sentence and the only place two authorities are allowed to meet.
//! * [`ServiceName`] — one name that is a cgroup, a notify socket **and** an
//!   installed application's directory.
//! * The paths an installed application occupies, so that no crate spells one
//!   twice (`ADR-011`: a path spelled in two crates is two answers to one question).
//! * [`app_identity`] — **who an installed application runs as**, added
//!   2026-08-24. `ADR-015` §5.1's first sandbox mechanism, and the fourth thing
//!   an application's name is: a uid. It is here rather than in the supervisor
//!   because three things need the same answer and must not each compute their
//!   own — the supervisor that drops privilege, [`crate::etc`]'s `/etc/passwd`
//!   projection, and the installer that will one day `chown` an unpacked tree.
//!
//! Three crates need this vocabulary and no two of them may depend on each
//! other in the direction that would let one own it: `rivet-config` parses the
//! operator's document, `cmd/rivet-sysd` holds the compiled-in system
//! definitions and does the merge, and `rivet-core` is what both already
//! depend on — `ADR-014` §3.2 makes the direction `rivet-config →
//! rivet-core` **permanent**, because Cargo forbids a cycle. So the domain
//! owns the words.
//!
//! Nothing here performs I/O and nothing here parses TOML.

use crate::package::application::{Cpu, Manifest, Memory};

/// A service's name, checked to be usable as one.
///
/// The name is not decoration. `ADR-017` §3.1.3 makes cgroup membership the
/// authority for what is running and the service name is the cgroup's path
/// component; `ADR-017` §3 makes it part of the `NOTIFY_SOCKET` identity. A
/// name containing a separator would silently nest one service's cgroup inside
/// another's, and a name containing a control character would produce a path no
/// `AF_UNIX` address can hold.
///
/// **For an application it is a third thing**, from 2026-08-24: the directory
/// under [`APP_ROOT`] the application is installed into. That is what makes the
/// name *observed* rather than declared — `crate::package::application`'s
/// manifest deliberately carries no `name`, so a machine cannot hold two answers
/// to what an application is called.
///
/// So the rule is a small closed set rather than "whatever TOML allows", which
/// is `ADR-006` §6.4's discipline applied to an identifier: every real name in
/// the record — `rivet-netd`, `rivet-timed`, `rivet-apid`, `rivet-mcpd`
/// (`ADR-017` §2.1) — satisfies it, as does every Cargo package name, which is
/// what an application service is named after (`ADR-015` §3.2).
///
/// **The length limit is chosen here, not read out of the record.** 64
/// characters leaves room inside `sockaddr_un`'s 108-byte `sun_path` for a
/// runtime directory prefix and a suffix.
///
/// *Moved here from `rivet-config` on 2026-08-24*, when an application's
/// installed directory became the third thing this name is. It is re-exported
/// there, so the schema's own vocabulary is unchanged.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServiceName(String);

impl ServiceName {
    /// The longest name accepted. See the type documentation for why there is
    /// a limit at all and why this number.
    pub const MAX_LEN: usize = 64;

    /// The whole rule, in one sentence, for a diagnostic to quote.
    ///
    /// `&'static str` rather than a formatted message on purpose: a refusal
    /// built from this cannot carry any part of the document.
    pub const RULE: &'static str = "a service name is 1 to 64 characters of ASCII letters, digits, `-`, `_` or `.`, \
         starting with a letter or a digit; it names a cgroup and a socket, so it cannot \
         contain a path separator";

    /// Accept a name, or refuse it.
    ///
    /// Refusing is a statement about the *name*, never about whether the
    /// service exists — `rivet-config`'s `services` module argues at length
    /// why an unrecognised service name is accepted.
    #[must_use]
    pub fn new(name: &str) -> Option<Self> {
        if name.is_empty() || name.len() > Self::MAX_LEN {
            return None;
        }
        let mut chars = name.chars();
        let first = chars.next()?;
        if !first.is_ascii_alphanumeric() {
            return None;
        }
        if !chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.') {
            return None;
        }
        Some(Self(name.to_owned()))
    }

    /// The name as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Display for ServiceName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

/// **Every key `[services.<name>]` declares** — the operator's control surface
/// over a service, as opposed to an application's own settings.
///
/// `rivet_config::Service::DECLARED_KEYS` **is** this list rather than a copy of
/// it (`ADR-011`), and it lives down here because a second reader needs it:
/// [`crate::settings::render`] refuses a key that belongs to this table and says
/// which table it belongs to. That refusal is the whole reason the list moved on
/// 2026-09-05 — a port wrote `enabled` under `[applications.<name>]`, was told
/// only that the value must be quoted and then that the key was undeclared, and
/// lost a boot to two true sentences neither of which named `[services.<name>]`.
///
/// **Four keys since 2026-09-09, and the value of the list is in what is NOT
/// in it.** `rivet_config::services` has the rule that keeps it that way, and
/// `critical` satisfies all three of its clauses: it changes no code, it is a
/// policy the machine's owner is entitled to set — the same binary is critical
/// on the machine that serves the house and unimportant on a bench — and Rivet
/// enforces it from outside the service, in [`crate::health::machine`]'s fold.
///
/// **`critical` is deliberately NOT a manifest key** (`ADR-045` §4): an
/// application that could declare itself critical would be setting its own
/// severity, which is the thing that document spends §2 and §4.3 taking away
/// from application authors.
///
/// **`cpu` joins `memory` rather than growing a fourth clause of its own** —
/// `ADR-007` §7 and `ADR-015` §3, both amended 2026-09-09. It satisfies
/// exactly the two clauses `memory` does and for the identical reason:
/// changing it cannot change which code runs, it is a policy the machine's
/// owner is entitled to set, and Rivet enforces it from outside the service —
/// a `cpu.max` the service does not read and cannot refuse. **This module
/// does not re-derive that argument; it mirrors [`MemoryLimit`]'s at every
/// layer**, because a second argument for one ruling is `ADR-011`'s failure
/// with extra words. See [`CpuLimit`] and [`effective_cpu`].
pub const OPERATOR_KEYS: &[&str] = &["enabled", "memory", "cpu", "critical"];

// ---------------------------------------------------------------------------
// Where an application lives on the machine
// ---------------------------------------------------------------------------

/// The **built-in volume's** root directory. `ADR-031` §2.1, §11 item 7.
///
/// A Rivet always has this one, it needs no declaration, and its root is
/// literally `/state` rather than something under a uniform mount point — so a
/// machine with no declared volume has a layout byte-identical to the one every
/// board already carries, and there is no migration to get wrong. `ADR-031` §11
/// item 7 states the cost it accepts in exchange: one visible irregularity.
pub const BUILT_IN_VOLUME: &str = "/state";

/// Where installed applications are unpacked **on a given volume**.
/// `ADR-015` §2's table and §7, generalised by `ADR-031` §8 item 1.
///
/// **This was the constant `APP_ROOT` until 2026-08-25.** It is a function
/// because an application no longer necessarily lives on the system device
/// (`ADR-031` §3), and every caller that passes [`BUILT_IN_VOLUME`] is marking a
/// place where a volume will eventually have to come from. That is the point of
/// making the change before there is anything to attach: the gap is legible in
/// the call sites rather than inferred later.
#[must_use]
pub fn app_root(volume: &str) -> String {
    format!("{volume}/app")
}

/// `/state/app` — the built-in volume's application root, as a `&'static str`.
///
/// **Two spellings of one path, held together by a tripwire** in this module's
/// tests rather than by care. It exists because three `const` tables need a
/// `&'static str` and cannot call a function: `crate::generated`'s catalogue,
/// `crate::operation::edit`'s reserved list, and this module's own tests. Where
/// a `String` is allowed, call [`app_root`] — the constant is not the general
/// answer and a caller reaching for it on a machine with volumes is a bug.
pub const APP_ROOT: &str = "/state/app";

/// The name of the symlink `ADR-015` §7 swaps to make an install atomic.
///
/// > *"Unpack to `/state/app/<name>/<version>/`, verify the signature, then swap
/// > a symlink — a `rename`, not a copy."*
///
/// The ADR names the mechanism and not the link, so **the word is chosen here**
/// and it is the one part of this layout the record does not state. `PLAN.md`
/// §0.4.3 carries it.
pub const CURRENT_LINK: &str = "current";

/// The directory an application's binary is installed into.
///
/// `ADR-015` §3.2: *"There is deliberately no `exec` field. The artifact is a
/// Cargo package, so the binary **is** the crate's binary."* So the program is
/// derived from the name rather than declared, and no manifest can name a
/// program the signature never covered.
pub const BIN_DIR: &str = "bin";

/// `<volume>/app/<name>` — everything installed for one application.
#[must_use]
pub fn app_dir(volume: &str, name: &ServiceName) -> String {
    format!("{}/{name}", app_root(volume))
}

/// `<volume>/app/<name>/current` — the version that is live right now.
#[must_use]
pub fn current_dir(volume: &str, name: &ServiceName) -> String {
    format!("{}/{CURRENT_LINK}", app_dir(volume, name))
}

/// `<volume>/app/<name>/current/manifest.toml` — the installed copy of the
/// artifact's manifest member.
///
/// **The filename is `crate::package::application::MANIFEST_NAME`**, read from
/// there rather than spelled again, because these are the same file: an
/// installer writes the verified member out and this is where it lands. A
/// tripwire in this module's tests holds the two together.
#[must_use]
pub fn manifest_path(volume: &str, name: &ServiceName) -> String {
    format!(
        "{}/{}",
        current_dir(volume, name),
        crate::package::application::MANIFEST_NAME
    )
}

/// `<volume>/app/<name>/current/bin/<name>` — what the supervisor executes.
///
/// Derived, never declared. See [`BIN_DIR`].
#[must_use]
pub fn program_path(volume: &str, name: &ServiceName) -> String {
    format!("{}/{BIN_DIR}/{name}", current_dir(volume, name))
}

// ---------------------------------------------------------------------------
// The image's own services — PLAN-Feature-Set-2026-09-12 §3
// ---------------------------------------------------------------------------

/// `rivet-netd`. Named here, once, rather than as a literal in
/// `cmd/rivet-sysd`'s compiled service list, on [`crate::bluetooth::PROVIDER`]'s
/// argument: a name two crates might otherwise spell separately is `ADR-011`'s
/// failure waiting to happen, and this is the one an application's name is
/// checked against ([`IMAGE_SERVICE_NAMES`]) as well as the one the supervisor
/// starts.
pub const NETD_SERVICE_NAME: &str = "rivet-netd";

/// `rivet-timed`. See [`NETD_SERVICE_NAME`].
pub const TIMED_SERVICE_NAME: &str = "rivet-timed";

/// Where `cmd/rivet-image` installs `cmd/rivet-timed`, and what
/// `cmd/rivet-sysd` executes. Here, on [`PROXYD_PATH`]'s footing, because
/// [`IMAGE_SERVICE_HOMES`] names it.
pub const TIMED_PATH: &str = "/system/bin/rivet-timed";

/// `rivet-mdnsd`. See [`NETD_SERVICE_NAME`].
pub const MDNSD_SERVICE_NAME: &str = "rivet-mdnsd";

/// Where `cmd/rivet-image` installs `cmd/rivet-mdnsd`, and what
/// `cmd/rivet-sysd` executes. Spelled once here, on [`PROXYD_PATH`]'s footing.
pub const MDNSD_PATH: &str = "/system/bin/rivet-mdnsd";

/// `rivet-mcpd`. See [`NETD_SERVICE_NAME`].
pub const MCPD_SERVICE_NAME: &str = "rivet-mcpd";

/// `rivet-proxyd`, the machine's front door — `ADR-060`. See
/// [`NETD_SERVICE_NAME`].
pub const PROXYD_SERVICE_NAME: &str = "rivet-proxyd";

/// Where `cmd/rivet-image` installs `cmd/rivet-proxyd`, and what
/// `cmd/rivet-sysd` executes.
///
/// **In `rivet-core`, unlike the other `/system/bin` paths**, which are each
/// spelled twice and held together by tripwires because a binary crate has no
/// library target. Both crates that need this one already depend on this crate,
/// so it is spelled once and neither side can drift from the other.
pub const PROXYD_PATH: &str = "/system/bin/rivet-proxyd";

/// `rivet-dnsd`, the name service — `ADR-047`. See [`NETD_SERVICE_NAME`].
pub const DNSD_SERVICE_NAME: &str = "rivet-dnsd";

/// Where `cmd/rivet-image` installs `cmd/rivet-dnsd`, and what
/// `cmd/rivet-sysd` executes. Spelled once here, on [`PROXYD_PATH`]'s footing.
pub const DNSD_PATH: &str = "/system/bin/rivet-dnsd";

/// Where `cmd/rivet-image` installs `cmd/rivet-bluetoothd`, the BLE observer,
/// and what `cmd/rivet-sysd` executes. Spelled once here, on [`PROXYD_PATH`]'s
/// footing, because [`IMAGE_SERVICE_HOMES`] names it; its name is
/// `crate::bluetooth::PROVIDER`.
pub const BLUETOOTHD_PATH: &str = "/system/bin/rivet-bluetoothd";

/// Where `cmd/rivet-image` installs `cmd/rivet-displayd`, the compositor, and
/// what `cmd/rivet-sysd` executes. Spelled once here, on [`PROXYD_PATH`]'s
/// footing; its name is `crate::compositor::PROVIDER`.
pub const DISPLAYD_PATH: &str = "/system/bin/rivet-displayd";

/// **An image service that runs under its own identity, with a home under
/// `<volume>/app/<name>`** — `ADR-060`.
///
/// The ordinary image service runs as root with no confinement and no home
/// (`cmd/rivet-sysd`'s `IMAGE_DEFINED`). One of these is the other shape: part
/// of the operating system and versioned with it, but run in user space exactly
/// as an application is — the uid [`app_identity`] derives from its name, a
/// ruleset from `crate::confinement::image_service_policy`, and the data,
/// scratch, settings and routing paths an application has, at the same places.
/// That sameness is what lets a program move from being carried to being built
/// in without its files moving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageServiceHome {
    /// The service's name, which is also the directory under `<volume>/app`.
    pub name: &'static str,
    /// The program on the system slot.
    pub program: &'static str,
    /// The settings keys it understands — the declaration an application makes
    /// in its manifest's `settings`, which an image service has no manifest to
    /// make. `cmd/rivet-sysd` delivers `[applications.<name>]` against this.
    pub settings: &'static [&'static str],
}

/// Every [`ImageServiceHome`]. See that type.
pub const IMAGE_SERVICE_HOMES: &[ImageServiceHome] = &[
    ImageServiceHome {
        name: PROXYD_SERVICE_NAME,
        program: PROXYD_PATH,
        // The name the applications behind it present, whether it still
        // accepts the operator's client certificate, and how long a client
        // has to finish its handshake — `cmd/rivet-proxyd`.
        settings: &[
            crate::proxy::UPSTREAM_NAME_SETTING,
            crate::proxy::OPERATOR_CERTIFICATE_SETTING,
            crate::proxy::HANDSHAKE_SECONDS_SETTING,
        ],
    },
    ImageServiceHome {
        name: DNSD_SERVICE_NAME,
        program: DNSD_PATH,
        // None: what it serves is the document's `[dns]` table, which is
        // handed to it as `crate::name_service`'s projection rather than as
        // flat settings (`ADR-047` §6).
        settings: &[],
    },
    ImageServiceHome {
        name: crate::compositor::PROVIDER,
        program: DISPLAYD_PATH,
        // None: it enumerates the DRM device rather than being told which one,
        // and takes its sockets from the supervisor (`ADR-058` §4).
        settings: &[],
    },
    ImageServiceHome {
        name: MDNSD_SERVICE_NAME,
        program: MDNSD_PATH,
        // None: what it announces is handed to it as `crate::mdns`'s
        // projection rather than as flat settings (`ADR-017` §3).
        settings: &[],
    },
    ImageServiceHome {
        name: TIMED_SERVICE_NAME,
        program: TIMED_PATH,
        // None: its servers are on its argument vector (`ADR-021` §7).
        settings: &[],
    },
    ImageServiceHome {
        name: crate::bluetooth::PROVIDER,
        program: BLUETOOTHD_PATH,
        // None: it asks the kernel for its adapter and is handed its socket
        // (`ADR-041` §6).
        settings: &[],
    },
];

/// The [`ImageServiceHome`] for this name, if the image defines one.
#[must_use]
pub fn image_service_home(name: &str) -> Option<&'static ImageServiceHome> {
    IMAGE_SERVICE_HOMES.iter().find(|home| home.name == name)
}

/// **Every service `cmd/rivet-image` bakes into every card, whatever add-ons it
/// carries** — the names `cmd/rivet-sysd`'s compiled `definitions::system_services`
/// starts, gathered here because that function belongs to a binary crate with no
/// library target and is not reachable from anywhere else.
///
/// This is what an installed application's name is refused against
/// (`crate::package::verify::not_an_image_service`, `PLAN-Feature-Set-2026-09-12`
/// §3, `.rulings/` R-156 R10) at the same two points a wrong
/// architecture already is: `cmd/rivet-mcpd`'s door, and `cmd/rivet-deploy`'s
/// installer as defence in depth. Without the check, an application under one of
/// these names installs, is refused every boot by `cmd/rivet-sysd`'s own
/// discovery — *"an installed application has the same name as a service this
/// machine already defines"* — and never runs; worse, `Supervisor::begin_uninstall`
/// finds the image's service of that name first and cannot remove the shadow.
///
/// **`rivet-launcher` is on this list and stays there.** `ADR-050` §2 moved it
/// from an application to an image service; the one copy already installed under
/// that name on a machine predates the ruling and is a fact about that machine's
/// history, not something this list is asked to accommodate (`.rulings/` R-156
/// R10 — it leaves with the next re-mint).
pub const IMAGE_SERVICE_NAMES: &[&str] = &[
    NETD_SERVICE_NAME,
    TIMED_SERVICE_NAME,
    MDNSD_SERVICE_NAME,
    MCPD_SERVICE_NAME,
    crate::bluetooth::PROVIDER,
    crate::input::PROVIDER,
    // **`rivet-displayd`, the compositor** — `cmd/rivet-sysd`'s
    // `definitions::displayd` is `..IMAGE_DEFINED` at `/system/bin/rivet-displayd`,
    // so it is baked into every card exactly as the launcher and mdnsd are. It
    // was absent here, which made `Document::carried_services` treat a kiosk
    // member's compositor as an application the fleet repository had to carry —
    // so `cargo rivet provision` refused a kiosk card for want of a published
    // `rivet-displayd` that is not an application and never had a manifest.
    crate::compositor::PROVIDER,
    // **`rivet-proxyd`, the front door** — `ADR-060`. Here, it is no longer
    // carried, published or installed: `Document::carried_services` drops it,
    // and an `.rvt` under its name is refused at the install door.
    PROXYD_SERVICE_NAME,
    // **`rivet-dnsd`, the name service** — `ADR-047`. Baked into every card and
    // shipped disabled; the document turns it on.
    DNSD_SERVICE_NAME,
];

/// The start of every **DHCP client service's** name: one per interface the
/// document addresses by DHCP, `rivet-dhcp-<interface>` — [`dhcp_service_name`].
pub const DHCP_SERVICE_PREFIX: &str = "rivet-dhcp-";

/// The start of every **wireless supplicant service's** name: one per
/// interface the document joins to a network, `rivet-wpa-<interface>` —
/// [`supplicant_service_name`].
pub const SUPPLICANT_SERVICE_PREFIX: &str = "rivet-wpa-";

/// **The supervised DHCP client for `interface`.** `cmd/rivet-sysd` derives
/// one from the document's `[network]` table for each interface with
/// `method = "dhcp"`, and `cmd/rivet-netd` names it when it is waiting on
/// that client's lease. One spelling, here, so the two cannot disagree.
#[must_use]
pub fn dhcp_service_name(interface: &str) -> String {
    format!("{DHCP_SERVICE_PREFIX}{interface}")
}

/// **The supervised supplicant for `interface`** — [`dhcp_service_name`]'s
/// footing, for an interface the document gives an `ssid`.
#[must_use]
pub fn supplicant_service_name(interface: &str) -> String {
    format!("{SUPPLICANT_SERVICE_PREFIX}{interface}")
}

/// **Whether `name` is one of the network's derived services** — a DHCP client
/// or a supplicant, whichever interface it is for.
///
/// These are image services in every respect but one: their names are made
/// from the document rather than listed, so [`IMAGE_SERVICE_NAMES`] cannot
/// hold them. This is the same refusal by prefix, and an installed
/// application under one of these names is refused for
/// [`IMAGE_SERVICE_NAMES`]'s reason.
#[must_use]
pub fn is_network_mechanism_service(name: &str) -> bool {
    name.starts_with(DHCP_SERVICE_PREFIX) || name.starts_with(SUPPLICANT_SERVICE_PREFIX)
}

/// **Every application name reserved because this project's own machinery
/// already means something else by it** — `.rulings/` R-210 D21.
///
/// **A superset of [`IMAGE_SERVICE_NAMES`], answering a different question.**
/// `IMAGE_SERVICE_NAMES` is about a service the image itself runs; this is
/// about a name that already resolves to something else entirely —
/// `getpwnam`, the uninstall grammar, a journal reader — so an application
/// under one of these would not merely collide with a service, it would BE
/// indistinguishable from that other thing. A name only has to appear in one
/// of the two lists to be refused
/// ([`crate::package::verify::not_an_image_service`],
/// [`crate::package::verify::not_a_reserved_name`]).
///
/// **Built by iterating the tables that own each name, never by restating
/// them**, so a name added to one of those tables becomes reserved the
/// moment it exists: [`crate::etc::SYSTEM_IDENTITIES`] and
/// [`crate::etc::SYSTEM_GROUPS`] are the image's own account tables — `root`,
/// `tty` and `repo`, today — and the uninstall grammar's two keywords
/// (`crate::operation::UNINSTALL_CONFIRMATION`, `crate::uninstall::PURGE`)
/// are read from the constants that grammar itself parses against, not a
/// second copy of them.
///
/// The remaining four join by name because each already has exactly one
/// spelling and this is it: [`crate::deploy::SERVICE_NAME`] (the local sender
/// a deploy report may claim), [`crate::journal::SUPERVISOR_SVC`] (the
/// supervisor's own journal records), [`crate::journal::KERNEL_SVC`] (the
/// kernel's, relayed) and [`crate::journal::INGEST_SVC`] (an ingested record
/// with no usable name of its own).
#[must_use]
pub fn reserved_application_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> =
        crate::etc::SYSTEM_IDENTITIES.iter().map(|identity| identity.name).collect();
    names.extend(crate::etc::SYSTEM_GROUPS.iter().map(|group| group.name));
    names.push(crate::operation::UNINSTALL_CONFIRMATION);
    names.push(crate::uninstall::PURGE);
    names.push(crate::deploy::SERVICE_NAME);
    names.push(crate::journal::SUPERVISOR_SVC);
    names.push(crate::journal::KERNEL_SVC);
    names.push(crate::journal::INGEST_SVC);
    names
}

/// Whether `name` is reserved — see [`reserved_application_names`].
#[must_use]
pub fn is_reserved_application_name(name: &str) -> bool {
    reserved_application_names().contains(&name)
}

/// **The first release whose image carries `rivet-proxyd`** — `ADR-060` §5.
///
/// Provisioning refuses a server or member card whose image is older: such an
/// image has no proxy, and a card can no longer carry one, so the machine would
/// come up with no front door. A patch release, `MAJOR.MINOR.PATCH`, compared by
/// [`release_at_least`].
pub const PROXY_IN_IMAGE_SINCE: &str = "0.1.114";

/// Whether `version` is at least `floor`, both `MAJOR.MINOR.PATCH` with any
/// suffix after the patch number ignored (`0.1.114-3-gabc1234` is `0.1.114`).
///
/// `None` when either does not start with three dotted numbers — a version
/// that cannot be read is not guessed at (`AGENTS.md` §5).
#[must_use]
pub fn release_at_least(version: &str, floor: &str) -> Option<bool> {
    fn triple(text: &str) -> Option<(u64, u64, u64)> {
        let text = text.strip_prefix('v').unwrap_or(text);
        let mut parts = text.splitn(3, '.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let rest = parts.next()?;
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            return None;
        }
        Some((major, minor, digits.parse().ok()?))
    }
    Some(triple(version)? >= triple(floor)?)
}

// ---------------------------------------------------------------------------
// The unit, and the rule the ruling turns on
// ---------------------------------------------------------------------------

/// How much memory a service may have — **with a number behind it.**
///
/// A cgroups v2 `memory.max`, which `ADR-015` §5.1 already requires and which
/// `ADR-017` §3.1.3 already relies on for observing what is running.
///
/// **The one key the 2026-08-24 ruling leaves on both sides**, and deliberately:
/// the manifest's value is the application's *request* (`ADR-015` §3.3 makes the
/// manifest the permission request) and the document's is the operator's *cap*.
/// [`effective_memory`] reconciles them, and the rule is *the lower wins*.
///
/// # Binary, not decimal — and on whose authority
///
/// `ADR-015` §3.2 writes `memory = "128M"` and never says what `M` means;
/// No formal ruling has closed the question. `crate::package::application`
/// refuses to convert because of it, and it is right to: for a *build product*
/// there is no cost to carrying the text and a real cost to guessing.
///
/// Here there is no way to avoid answering, because *the lower wins* is a
/// comparison and a comparison needs numbers. So this type answers it, and the
/// answer is **1024**, on three grounds rather than on taste:
///
/// 1. **It is what already shipped.** `rivet-config` has parsed and
///    round-tripped the operator's `memory` this way since 2026-08-22, and its
///    canonical serialisation depends on it. Changing it would silently
///    re-interpret every document already written.
/// 2. The value ends up in `memory.max`, which counts **bytes**, and every
///    neighbouring system an operator will have used — systemd, Docker, cgroup
///    tooling — reads these suffixes as binary multiples.
/// 3. On a machine with between 512 MiB and 8 GiB of RAM, the 7% difference in a
///    memory *cap* is the difference between a service that fits and one the
///    kernel kills.
///
/// **It remains the maintainer's to overturn** — `AGENTS.md` §1 — and `PLAN.md`
/// §0.4.3 says what overturning it would cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MemoryLimit(u64);

impl MemoryLimit {
    /// The whole rule, for a diagnostic to quote without quoting the input.
    pub const RULE: &'static str = "a memory limit is a whole number of bytes with an optional `K`, `M` or `G` suffix \
         counting in multiples of 1024 — for example `128M` — and it must be greater than zero";

    /// Parse `128M`, `64K`, `1G`, or a bare byte count.
    ///
    /// The suffixes are case-sensitive and there is one spelling of each
    /// (`ADR-006` §6.4). Zero is refused because a service that may allocate
    /// nothing is not a limited service, it is a broken one, and `ADR-007`
    /// §3.6.5 forbids accepting an input whose effect is not what was written.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (digits, multiplier) = match text.as_bytes().last()? {
            b'K' => (&text[..text.len() - 1], 1024_u64),
            b'M' => (&text[..text.len() - 1], 1024_u64 * 1024),
            b'G' => (&text[..text.len() - 1], 1024_u64 * 1024 * 1024),
            _ => (text, 1_u64),
        };
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        // One number, one spelling — the same canonicality argument the
        // artifact's own `Memory::parse` makes about a leading zero.
        if digits.len() > 1 && digits.starts_with('0') {
            return None;
        }
        let count: u64 = digits.parse().ok()?;
        let bytes = count.checked_mul(multiplier)?;
        if bytes == 0 {
            return None;
        }
        Some(Self(bytes))
    }

    /// Give an artifact's `memory` request a number.
    ///
    /// This is the **one** place the two representations meet, and it is one
    /// function rather than a conversion scattered through callers precisely so
    /// that the unit above has a single site. `None` means the artifact carried
    /// a shape this build cannot value — which `Memory::parse` has already made
    /// nearly impossible, since the two grammars differ only in that this one
    /// refuses a zero.
    #[must_use]
    pub fn of(request: &Memory) -> Option<Self> {
        Self::parse(request.as_str())
    }

    /// The limit in bytes, which is what a cgroup wants.
    #[must_use]
    pub const fn bytes(self) -> u64 {
        self.0
    }
}

/// The one spelling this limit has.
///
/// **The largest suffix that divides the value exactly**, so that `128M` is
/// written `128M` and `1023` is written `1023`. That is a total function of the
/// value with exactly one result, which is what `ADR-007` §3.1 asks of a
/// canonical representation; it is not "the spelling the operator used", which
/// is a fact about the text and is deliberately not preserved.
///
/// It round-trips through [`MemoryLimit::parse`] by construction — the rendered
/// digits never lead with a zero and are never zero, which are the two spellings
/// `parse` refuses — and `rivet-config`'s `serialise` tests hold that.
impl core::fmt::Display for MemoryLimit {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        const G: u64 = 1024 * 1024 * 1024;
        const M: u64 = 1024 * 1024;
        const K: u64 = 1024;
        let (count, suffix) = match self.0 {
            n if n.is_multiple_of(G) => (n / G, "G"),
            n if n.is_multiple_of(M) => (n / M, "M"),
            n if n.is_multiple_of(K) => (n / K, "K"),
            n => (n, ""),
        };
        write!(f, "{count}{suffix}")
    }
}

/// Which side of the split a service's effective memory cap came from.
///
/// Reported rather than only the number, because `ADR-011` part 2 is the half
/// that makes part 1 worth anything: *the system says which one, at the moment
/// the answer is needed.* An operator who set `64M` and sees a service capped at
/// `64M` learns nothing about whether their value was honoured or merely
/// happened to match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemorySource {
    /// Neither the manifest nor the document named one, so there is no cap.
    Unstated,
    /// Only the application asked, so its request stands.
    Manifest,
    /// Only the operator asked, so the cap is theirs.
    Document,
    /// Both named one and the **operator's** was lower.
    DocumentLower,
    /// Both named one and the **application's** was lower.
    ManifestLower,
    /// Both named the same number. Which one "won" is not a meaningful
    /// question, and saying either would invite a reader to believe the other
    /// had been overridden.
    Agreed,
    /// Neither the application's manifest nor the operator's document named a
    /// limit, so the application runs under the default one,
    /// [`default_application_memory`].
    Default,
}

impl MemorySource {
    /// A word for a journal record.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unstated => "unstated",
            Self::Manifest => "manifest",
            Self::Document => "document",
            Self::DocumentLower => "document-lower",
            Self::ManifestLower => "manifest-lower",
            Self::Agreed => "agreed",
            Self::Default => "default",
        }
    }

    /// Whether the operator's cap changed the outcome. The one question an
    /// operator reading `rivet status` is actually asking.
    #[must_use]
    pub const fn operator_tightened(self) -> bool {
        matches!(self, Self::Document | Self::DocumentLower)
    }
}

impl core::fmt::Display for MemorySource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The cap that actually applies, and where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveMemory {
    /// `None` means *no cap*, which is a real answer and is not a zero.
    pub limit: Option<MemoryLimit>,
    /// Which side of the split it came from.
    pub source: MemorySource,
}

impl EffectiveMemory {
    /// Nothing stated anywhere.
    pub const UNSTATED: Self = Self {
        limit: None,
        source: MemorySource::Unstated,
    };
}

/// **The lower wins** — the 2026-08-24 ruling, as a function.
///
/// `request` is the application's, out of its signed manifest (`ADR-015` §3.2).
/// `cap` is the operator's, out of the desired-state document (`ADR-007` §7.2).
///
/// # Why the minimum, in both directions
///
/// * **The operator's lower value must win**, or a memory cap is not a cap: an
///   application could grant itself any amount by asking for it, and the key
///   `ADR-007` §7.2 exists to give the machine's owner would do nothing.
/// * **The application's lower value must also win**, and this is the half that
///   is easy to get wrong. An operator who writes `memory = "512M"` for an
///   application that asked for `128M` has not *granted* it 512 MiB — they have
///   said "no more than 512 MiB", and the application's own statement about how
///   much it needs is the tighter of the two. Raising it would be the document
///   rewriting a manifest value from outside the signature, which is exactly
///   what `ADR-015` §4.1 forbids and what the split-by-direction ruling exists
///   to prevent.
///
/// So the operator's key can only ever make a service's cap **tighter**, never
/// looser, which is what *"a cap runs the other way"* means in one line of code.
#[must_use]
pub fn effective_memory(request: Option<MemoryLimit>, cap: Option<MemoryLimit>) -> EffectiveMemory {
    match (request, cap) {
        (None, None) => EffectiveMemory::UNSTATED,
        (Some(r), None) => EffectiveMemory {
            limit: Some(r),
            source: MemorySource::Manifest,
        },
        (None, Some(c)) => EffectiveMemory {
            limit: Some(c),
            source: MemorySource::Document,
        },
        (Some(r), Some(c)) => EffectiveMemory {
            limit: Some(if c.bytes() < r.bytes() { c } else { r }),
            source: match c.bytes().cmp(&r.bytes()) {
                core::cmp::Ordering::Less => MemorySource::DocumentLower,
                core::cmp::Ordering::Greater => MemorySource::ManifestLower,
                core::cmp::Ordering::Equal => MemorySource::Agreed,
            },
        },
    }
}

/// **The memory limit an application gets when its manifest asks for none:
/// half of the machine's memory**, or `None` for a machine reporting less
/// than two bytes.
///
/// A fraction rather than a number, so it scales from a 1 GB Pi to a machine
/// with sixteen times that. Half leaves the other half for the image's
/// services and every other application, so one application that grows
/// without bound meets its own cgroup's limit before the kernel has to choose
/// what to kill, and the supervisor, the control plane and its neighbours keep
/// running.
#[must_use]
pub const fn default_application_memory(machine_bytes: u64) -> Option<MemoryLimit> {
    let half = machine_bytes / 2;
    if half == 0 { None } else { Some(MemoryLimit(half)) }
}

/// The machine's memory, in bytes, from `/proc/meminfo`'s text: its
/// `MemTotal` line, which the kernel writes in kibibytes. `None` for text
/// without that line, or with one that will not read.
#[must_use]
pub fn meminfo_total_bytes(meminfo: &str) -> Option<u64> {
    meminfo.lines().find_map(|line| {
        let rest = line.strip_prefix("MemTotal:")?;
        let kib = rest.trim().strip_suffix("kB")?.trim().parse::<u64>().ok()?;
        kib.checked_mul(1024)
    })
}

/// [`effective_memory`] for an **application**, whose manifest's silence is
/// not "no limit" but [`default_application_memory`].
///
/// Three layers, each speaking only where the one before it is silent:
///
/// * **The manifest's request** meets the operator's cap by
///   [`effective_memory`]'s rule, the lower wins, exactly as for any service.
/// * **Where the manifest asks for nothing, the operator's cap replaces the
///   default in either direction.** The default is the system's guess on the
///   operator's behalf, not a statement from the application, so nothing
///   outside the signature is being raised when the operator's own number sits
///   above it: the machine's owner has the final word on their own machine.
/// * **The default applies only where neither has said anything.**
///
/// `default` is `None` on a machine whose memory could not be read, and the
/// application then runs under the operator's cap alone.
#[must_use]
pub fn effective_application_memory(
    request: Option<MemoryLimit>,
    default: Option<MemoryLimit>,
    cap: Option<MemoryLimit>,
) -> EffectiveMemory {
    match (request, default, cap) {
        (None, Some(default), None) => EffectiveMemory {
            limit: Some(default),
            source: MemorySource::Default,
        },
        _ => effective_memory(request, cap),
    }
}

/// What the *manifest* asks for, as a number — or `None` if it asked for
/// nothing.
///
/// The convenience that keeps [`MemoryLimit::of`]'s single site from being
/// re-derived at every call: a manifest's `memory` is optional, and an
/// unvaluable one is indistinguishable here from an absent one. A caller that
/// needs to tell those apart reads `Manifest::memory` itself.
#[must_use]
pub fn requested_memory(manifest: &Manifest) -> Option<MemoryLimit> {
    manifest.memory().and_then(MemoryLimit::of)
}

// ---------------------------------------------------------------------------
// CPU — the gap named in `ADR-015` §5's table, closed 2026-09-09, and it
// MIRRORS `MemoryLimit` at every layer rather than being designed afresh
// ---------------------------------------------------------------------------

/// How much CPU a service may have — **with a number behind it**, on
/// [`MemoryLimit`]'s footing and for the identical reason: comparing two caps
/// is meaningless until the unit is settled, and this is where it is settled.
///
/// A cgroups v2 `cpu.max`. `ADR-007` §7 and `ADR-015` §3 give `cpu` the
/// same split `memory` already has — the manifest's is a **request**, the
/// document's is a **cap**, and where both name one, [`effective_cpu`] takes
/// the lower — because the same argument that ruled `memory` that way rules
/// `cpu` identically: an operator's key can only ever make a service's
/// effective limit *tighter*, never raise a value the signature covers
/// (`ADR-015` §4).
///
/// # Cores, not microseconds — and the arithmetic, worked
///
/// `ADR-015` §3 and `ADR-007` §7 both write `cpu = "0.5"`, because a
/// number of cores is what an operator means and a raw scheduler quota is
/// not. cgroups v2 wants two numbers instead: `cpu.max` holds
/// `"<quota> <period>"`, both in microseconds. This type is where the one is
/// turned into the other, once, rather than at every caller that would
/// otherwise have to re-derive it.
///
/// [`Self::PERIOD_US`] fixes the period at 100000 µs (100 ms) — the kernel's
/// own default period, and the value every worked `cpu.max` example uses.
/// With the period fixed, a quota is simply `cores × PERIOD_US`:
///
/// | Written | Cores | Quota (`cores × 100000`) | `cpu.max` |
/// | :-- | :-- | :-- | :-- |
/// | `cpu = "0.5"` | 0.5 | 50000 | `50000 100000` |
/// | `cpu = "2"` | 2 | 200000 | `200000 100000` |
/// | `cpu = "0.25"` | 0.25 | 25000 | `25000 100000` |
///
/// A quota larger than the period is not a mistake and is not refused: cgroups
/// v2 defines it as permission to use more than one CPU's worth of time inside
/// each period, which is exactly what `cpu = "2"` asks for on a board with
/// more than one core.
///
/// # No default, on `AGENTS.md` §1.1's third row
///
/// Nothing in this build ever manufactures a `cpu` value: absent means
/// `cpu.max` is left at `max`, the kernel's own uncapped answer — precisely
/// [`MemoryLimit`]'s rule, and for the identical reason. A machine with one
/// application should let it have the whole processor, and a default cap
/// invented here would change how every already-provisioned machine behaves
/// the moment it updated, with nobody watching. That is a change to a default
/// every existing manifest inherits, which is the maintainer's to make and not
/// this module's — see `AGENTS.md` §1.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CpuLimit(u64);

impl CpuLimit {
    /// The period `cpu.max`'s quota is measured against. **Fixed, not
    /// configured** — there is no key anywhere that changes it, so one
    /// worked table (the type's own documentation) describes every machine.
    /// 100000 rather than some other number because it is the kernel
    /// scheduler's own default period, and every `cpu.max` example in the
    /// kernel's documentation and in this file uses it.
    pub const PERIOD_US: u64 = 100_000;

    /// **The smallest quota Linux will accept, at any period.** cgroup v2's
    /// `cpu.max` refuses a quota under one millisecond outright — the
    /// scheduler's own minimum bandwidth period, applied whichever period
    /// `cpu.max`'s second field names. [`Self::parse`] does not enforce this:
    /// it accepts a quota as fine as one microsecond, because the same
    /// grammar is shared with the desired-state document and a document key
    /// is allowed to tighten a manifest's request further still. So a value
    /// this type happily represents can still be one the kernel will not
    /// keep, and this constant is what lets a caller tell the two apart and
    /// say why, rather than reporting an unexplained refusal.
    pub const KERNEL_MIN_QUOTA_US: u64 = 1_000;

    /// The whole rule, for a diagnostic to quote without quoting the input.
    pub const RULE: &'static str = "a cpu limit is a decimal number of cores, for example \"0.5\" or \"2\", \
         with at most 5 digits after the point and no trailing zero, and it must be greater than zero";

    /// Parse `0.5`, `2`, `0.25`, or a bare whole number of cores.
    ///
    /// # The grammar, and why five decimal digits and not fewer or more
    ///
    /// A whole part with [`MemoryLimit::parse`]'s own digit rule — non-empty,
    /// all ASCII digits, no leading zero unless the whole part is exactly
    /// `0` — and an optional `.` followed by 1 to 5 digits that must not end
    /// in `0`. The trailing-zero refusal is the same canonicality argument
    /// `MemoryLimit::parse` makes about a leading zero: `0.50` and `0.5`
    /// would otherwise be two spellings of one value, and `2.0` a second
    /// spelling of `2`.
    ///
    /// **Five digits and not more, because [`Self::PERIOD_US`] is 100000 and
    /// a sixth decimal digit asks for a quota smaller than one
    /// microsecond** — a value this period cannot represent at all.
    /// Rounding it would be `AGENTS.md` §5's wrong guess succeeding: the
    /// parser would not fail, it would accept a number the operator did not
    /// write.
    ///
    /// Zero is refused for [`MemoryLimit::parse`]'s reason: a service that
    /// may have no CPU at all is not a limited service, it is one that can
    /// never run.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (whole, frac) = match text.split_once('.') {
            Some((w, f)) => (w, Some(f)),
            None => (text, None),
        };
        if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        if whole.len() > 1 && whole.starts_with('0') {
            return None;
        }
        let whole_val: u64 = whole.parse().ok()?;

        let frac_scaled = match frac {
            None => 0,
            Some(f) => {
                if f.is_empty() || f.len() > 5 || !f.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                if f.ends_with('0') {
                    return None;
                }
                let value: u64 = f.parse().ok()?;
                // `f` has `f.len()` digits and needs `5 - f.len()` more zeros
                // to reach `PERIOD_US`'s five places of precision — checked
                // above to be in `1..=5`, so this cannot underflow.
                value.checked_mul(10_u64.pow(u32::try_from(5 - f.len()).ok()?))?
            }
        };

        let quota = whole_val
            .checked_mul(Self::PERIOD_US)?
            .checked_add(frac_scaled)?;
        if quota == 0 {
            return None;
        }
        Some(Self(quota))
    }

    /// Give an artifact's `cpu` request a number, on [`MemoryLimit::of`]'s
    /// footing.
    #[must_use]
    pub fn of(request: &Cpu) -> Option<Self> {
        Self::parse(request.as_str())
    }

    /// The quota in microseconds against [`Self::PERIOD_US`] — what
    /// `cpu.max`'s first field wants.
    #[must_use]
    pub const fn quota_us(self) -> u64 {
        self.0
    }

    /// **Whether the kernel will refuse this cap outright** — `.rulings/`
    /// R-210 D22, [`Self::KERNEL_MIN_QUOTA_US`]'s own reason.
    ///
    /// The one comparison every writer of a `cpu` cap calls — the packager,
    /// an operator's `apply edit` of the machine document, `rivet-fleetd`'s
    /// commit path and the card writer among them — so no two writers can
    /// drift into disagreeing about where the line is. [`Self::parse`] does not enforce
    /// this on purpose (its own documentation says why): a boot-time parse of
    /// a value already installed or already saved must keep succeeding, so
    /// this is checked at the point something NEW is about to be written,
    /// never at the point something already written is read back.
    #[must_use]
    pub const fn is_below_kernel_minimum(self) -> bool {
        self.0 < Self::KERNEL_MIN_QUOTA_US
    }

    /// **The one explanation every refusal of a cap
    /// [`Self::is_below_kernel_minimum`] quotes** — `.rulings/` R-210 D22.
    /// Named once here rather than composed separately at each writer, so a
    /// later change to the reasoning or the numbers reaches all of them.
    ///
    /// Says nothing about WHERE the cap was written — a manifest key, a
    /// document key — because that framing differs by caller and belongs in
    /// the sentence around this one.
    #[must_use]
    pub fn kernel_refusal(self) -> String {
        format!(
            "a quota of {} microsecond(s) against the {}-microsecond period is below the \
             kernel's {}-microsecond minimum at any period — a board would accept it and \
             enforce nothing, refusing the cap itself with no cause named.",
            self.0,
            Self::PERIOD_US,
            Self::KERNEL_MIN_QUOTA_US,
        )
    }
}

/// The one spelling this limit has, on [`MemoryLimit`]'s [`core::fmt::Display`]
/// footing: the canonical form round-trips through [`CpuLimit::parse`] by
/// construction, because the digits it writes never end in `0` and the whole
/// part never leads with one — the two spellings `parse` refuses.
impl core::fmt::Display for CpuLimit {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let whole = self.0 / Self::PERIOD_US;
        let frac = self.0 % Self::PERIOD_US;
        if frac == 0 {
            write!(f, "{whole}")
        } else {
            // Pad to five digits so every place means what `parse` gave it,
            // then trim the trailing zeros `parse` refuses — the fixed-point
            // fraction's version of `MemoryLimit::fmt`'s "largest suffix that
            // divides the value exactly".
            let padded = format!("{frac:05}");
            let trimmed = padded.trim_end_matches('0');
            write!(f, "{whole}.{trimmed}")
        }
    }
}

/// Which side of the split a service's effective CPU cap came from — on
/// [`MemorySource`]'s footing, word for word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuSource {
    /// Neither the manifest nor the document named one, so there is no cap.
    Unstated,
    /// Only the application asked, so its request stands.
    Manifest,
    /// Only the operator asked, so the cap is theirs.
    Document,
    /// Both named one and the **operator's** was lower.
    DocumentLower,
    /// Both named one and the **application's** was lower.
    ManifestLower,
    /// Both named the same number. Which one "won" is not a meaningful
    /// question, and saying either would invite a reader to believe the other
    /// had been overridden.
    Agreed,
}

impl CpuSource {
    /// A word for a journal record.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unstated => "unstated",
            Self::Manifest => "manifest",
            Self::Document => "document",
            Self::DocumentLower => "document-lower",
            Self::ManifestLower => "manifest-lower",
            Self::Agreed => "agreed",
        }
    }

    /// Whether the operator's cap changed the outcome — [`MemorySource::
    /// operator_tightened`]'s question, asked about the other resource.
    #[must_use]
    pub const fn operator_tightened(self) -> bool {
        matches!(self, Self::Document | Self::DocumentLower)
    }
}

impl core::fmt::Display for CpuSource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The CPU cap that actually applies, and where it came from — [`EffectiveMemory`]'s footing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveCpu {
    /// `None` means *no cap*, which is a real answer and is not a zero.
    pub limit: Option<CpuLimit>,
    /// Which side of the split it came from.
    pub source: CpuSource,
}

impl EffectiveCpu {
    /// Nothing stated anywhere.
    pub const UNSTATED: Self = Self {
        limit: None,
        source: CpuSource::Unstated,
    };
}

/// **The lower wins** — `ADR-007` §7 and `ADR-015` §3's 2026-09-09
/// amendment, as a function. Identical to [`effective_memory`] with `CpuLimit`
/// in place of `MemoryLimit`, because the ruling it applies is the same
/// ruling: see [`effective_memory`]'s own documentation for the argument in
/// both directions. This function does not repeat it.
#[must_use]
pub fn effective_cpu(request: Option<CpuLimit>, cap: Option<CpuLimit>) -> EffectiveCpu {
    match (request, cap) {
        (None, None) => EffectiveCpu::UNSTATED,
        (Some(r), None) => EffectiveCpu {
            limit: Some(r),
            source: CpuSource::Manifest,
        },
        (None, Some(c)) => EffectiveCpu {
            limit: Some(c),
            source: CpuSource::Document,
        },
        (Some(r), Some(c)) => EffectiveCpu {
            limit: Some(if c.quota_us() < r.quota_us() { c } else { r }),
            source: match c.quota_us().cmp(&r.quota_us()) {
                core::cmp::Ordering::Less => CpuSource::DocumentLower,
                core::cmp::Ordering::Greater => CpuSource::ManifestLower,
                core::cmp::Ordering::Equal => CpuSource::Agreed,
            },
        },
    }
}

/// What the *manifest* asks for, as a number — or `None` if it asked for
/// nothing. [`requested_memory`]'s footing.
#[must_use]
pub fn requested_cpu(manifest: &Manifest) -> Option<CpuLimit> {
    manifest.cpu().and_then(CpuLimit::of)
}

/// **Which side of the display arbitration a service is on** — `ADR-036` §3.1,
/// amended 2026-09-04 (`.rulings/` R-138).
///
/// One machine has one arbiter and any number of lessees, and the difference is
/// not a matter of degree: the arbiter opens a display node and holds DRM master
/// for the life of the machine; a lessee is *handed* a descriptor cut from that
/// master's card and never opens a display node at all.
///
/// **It is not written in a manifest and must not be.** An application that
/// could ask to be the arbiter could ask for the screen, which is the whole of
/// what `ADR-015` §6.2 arbitrates. The supervisor derives it from what a
/// definition already declares — see `cmd/rivet-sysd`'s `Supervisor` — so there
/// is one spelling of the fact rather than two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayRole {
    /// **Holds DRM master and leases the display out.** One per machine, and
    /// today that is `rivet-launcher`.
    Arbiter,
    /// **Is handed a lease and draws through it.** Every kiosk application.
    Lessee,
}

/// Which device nodes a declared class covers — `ADR-015` §6.2, **kiosk
/// mode**, 2026-08-31.
///
/// # A class, expanded HERE and nowhere else
///
/// The manifest names a class and never a path
/// ([`crate::package::application::Device`]), so this function is the whole of
/// the mapping. It is the security boundary of the capability: a key that took
/// a path would be a key that could ask for `/dev/mem`, and this is what makes
/// sure it cannot.
///
/// # Some of these do not exist, and that is expected rather than an error
///
/// `/dev/dri/*` is absent on every board built to date — the kernel builds DRM
/// as a **module** and Rivet loads none (`PLAN.md` §103.12). `/dev/input/js*`
/// likewise. The grant walks what is present and says what it granted; a node
/// that is not there is not a failure, because devtmpfs creates nodes for the
/// hardware that exists and an application asking for `input` on a board with
/// no keyboard is not wrong.
///
/// **Globs rather than literals for the same reason.** How many
/// `/dev/input/event*` a board has depends on what is plugged into it, and a
/// fixed list would grant three and miss the fourth.
///
/// # Why `/dev/fb0` and `/dev/dri` are one class
///
/// They are two ways to reach one screen. A machine that granted the
/// framebuffer to one application and the GPU to another would have two owners
/// of one display, which is precisely what §6.2 point 3 refuses.
///
/// # And why one class is nevertheless two SETS — 2026-09-04
///
/// The paragraph above is about the *class an author declares*, and it still
/// stands: there is one `display`, and an author writes it once. What changed is
/// what the class expands to, which depends on which side of the arbitration the
/// declaring service is on — see [`DisplayRole`].
///
/// A **lessee** is handed a descriptor and never opens a display node by path,
/// so the only node it has any use for is the render node: `card*` and
/// `/dev/fb0` are two doors around the lease it was handed. Granting them made
/// `ADR-036` §3.1's *"no compatibility path"* untrue, because the door was open
/// and the kernel would have let it through.
#[must_use]
pub const fn device_globs(
    device: crate::package::application::Device,
    role: DisplayRole,
) -> &'static [&'static str] {
    use crate::package::application::Device;
    match device {
        // `/dev/fb0` is what a Rivet has today (`CONFIG_FB_SIMPLE`); `/dev/dri`
        // is what it will have when DRM stops being a module.
        //
        // The arbiter's set is the one this function always returned, and it is
        // unchanged on purpose: narrowing the ARBITER would take the screen away
        // from the one program that is supposed to own it.
        Device::Display => match role {
            DisplayRole::Arbiter => &["/dev/fb0", "/dev/dri/card*", "/dev/dri/renderD*"],
            DisplayRole::Lessee => &["/dev/dri/renderD*"],
        },
        // evdev is the interface every modern consumer uses — including
        // Raylib's DRM backend, which reads it directly and needs no libinput
        // and no udev. `mice`/`mouse*` are the legacy multiplexers and `js*`
        // the legacy joystick interface; they are included because a program
        // that wants them cannot be told to want something else.
        Device::Input => &[
            "/dev/input/event*",
            "/dev/input/mice",
            "/dev/input/mouse*",
            "/dev/input/js*",
        ],
        // **The render node and nothing else, whatever the role** —
        // `.rulings/` R-206 D4.
        //
        // This is deliberately not `match role`. `Device::Display` has two arms
        // because the arbiter modesets and a lessee does not; a render node
        // cannot modeset at all, so there is no second set to give anybody and
        // an arbiter that also declares `gpu` is asking for what it already has
        // from the arm above.
        //
        // `every_glob_is_a_render_node` below gates this the same way it gates
        // the lessee's: the tree stops compiling if a `card*` or a `/dev/fb0`
        // is ever added here, which is the whole reason a GPU token can be
        // handed out without reopening `ADR-015` §6.
        Device::Gpu => &["/dev/dri/renderD*"],
    }
}

/// Whether a node [`device_globs`] expanded to is a DRM **render** node — the
/// one device the supervisor shares through `crate::etc::RENDER_GID` rather
/// than hands to a single uid. Matched by the same prefix the const gates
/// below hold the lessee and `gpu` arms to.
#[must_use]
pub fn is_render_node(path: &str) -> bool {
    starts_with(path, RENDER_NODE_PREFIX)
}

/// The path every DRM render node starts with.
const RENDER_NODE_PREFIX: &[u8] = b"/dev/dri/renderD";

/// **The gate on the narrowing, and it is the strongest one available** —
/// `AGENTS.md` §3's third rank, a build that refuses rather than a human
/// noticing.
///
/// A lessee's set may hold render nodes and nothing else. Writing that as a
/// runtime test would put it in a suite somebody can choose not to run; as a
/// `const` assertion the tree does not compile, on any target, the moment a
/// `card*` or a `/dev/fb0` is added back to the lessee arm above.
///
/// It is deliberately a check on the SHAPE of the paths rather than an equality
/// against a second copy of the list — a test that compares a constant to a
/// duplicate of itself proves the two were typed the same way, and the property
/// that matters is *no display node reaches a lessee*.
const fn every_glob_is_a_render_node(globs: &[&str]) -> bool {
    let mut i = 0;
    while i < globs.len() {
        if !starts_with(globs[i], RENDER_NODE_PREFIX) {
            return false;
        }
        i += 1;
    }
    true
}

/// `str::starts_with` is not `const`; this is, so the gates below and
/// [`is_render_node`] read the one prefix the same way.
const fn starts_with(text: &str, prefix: &[u8]) -> bool {
    let text = text.as_bytes();
    if text.len() < prefix.len() {
        return false;
    }
    let mut i = 0;
    while i < prefix.len() {
        if text[i] != prefix[i] {
            return false;
        }
        i += 1;
    }
    true
}

const _: () = assert!(
    every_glob_is_a_render_node(device_globs(
        crate::package::application::Device::Display,
        DisplayRole::Lessee,
    )),
    "a lessee's display grant may cover render nodes only: /dev/dri/card* and /dev/fb0 are doors \
     around the lease it was handed, and granting one makes ADR-036 3.1's `no compatibility path` \
     untrue"
);

/// **The same gate on the GPU token, in BOTH roles** — `.rulings/` R-206 D4.
///
/// `Device::Gpu` exists so that an application which draws with the V3D does
/// not have to ask for `display` to get a render node. The entire safety of
/// that rests on one property: **this token never widens to a card node.** An
/// application holding `gpu` must be unable to modeset, because if it could,
/// `ADR-058`'s *"an application does not get the screen"* would be a manifest
/// keyword away from false.
///
/// Both roles are asserted rather than one, because unlike `Display` this arm
/// does not branch on the role — and an agent adding a branch later would
/// otherwise have a gate covering only the half they did not change.
const _: () = assert!(
    every_glob_is_a_render_node(device_globs(
        crate::package::application::Device::Gpu,
        DisplayRole::Lessee,
    )) && every_glob_is_a_render_node(device_globs(
        crate::package::application::Device::Gpu,
        DisplayRole::Arbiter,
    )),
    "the `gpu` device grant may cover render nodes only. A card node here would let any \
     application declare its way to DRM master, which is exactly what ADR-015 6.2 and ADR-058 \
     exist to prevent"
);

/// Split a [`device_globs`] pattern into the directory it lives in and the
/// name pattern inside it, or `None` if it is not a shape this understands.
///
/// **Deliberately not a glob engine.** Every pattern is a compile-time constant
/// in [`device_globs`] and every one is either a literal or `prefix*` in a
/// fixed directory. A general matcher would be a parser standing between an
/// application's request and a device node, which is the last place to add one.
#[must_use]
pub fn split_device_glob(pattern: &str) -> Option<(&str, &str)> {
    let (dir, name) = pattern.rsplit_once('/')?;
    if dir.is_empty() || name.is_empty() {
        return None;
    }
    // A `*` anywhere but the end, or more than one, is not a shape this
    // expands — and since the patterns are constants, reaching this is a defect
    // in `device_globs` rather than anything a caller did.
    match name.find('*') {
        Some(at) if at + 1 != name.len() => None,
        _ => Some((dir, name)),
    }
}

/// Does `entry` match a name pattern from [`split_device_glob`]?
#[must_use]
pub fn device_name_matches(pattern: &str, entry: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => entry.starts_with(prefix),
        None => entry == pattern,
    }
}

/// Where the supervisor puts the listening sockets it opens for a service.
///
/// `ADR-015` §5.3: *"Rivet creates the socket … so where it lives is not the
/// application's to say."* This is where it lives, and an application is never
/// told: it receives a **descriptor** (`ADR-017` §3's `LISTEN_FDS`), so the
/// path is Rivet's private business on the listening side.
///
/// A sibling of `crate::service`'s notify directory — `cmd/rivet-sysd`'s
/// `NOTIFY_DIR` is `/run/rivet/notify` — because it is the same kind of thing:
/// a socket the supervisor owns on a service's behalf, on a tmpfs, gone at the
/// next boot.
pub const LISTEN_DIR: &str = "/run/rivet/listen";

/// `/run/rivet/listen/<service>`.
#[must_use]
pub fn listen_dir(name: &ServiceName) -> String {
    format!("{LISTEN_DIR}/{}", name.as_str())
}

/// The mode a **unix** listening socket is created with, and it is the closed
/// end of the axis on purpose.
///
/// `0600`, root-owned. Nothing but root can connect, which means an
/// application-to-application connection does **not** work today — and that is
/// the honest state rather than an oversight. `ADR-015` §5.3's grant is a bind
/// mount into the mount namespaces of applications that declared and were
/// granted the connection, and this build has no mount namespaces
/// (`ADR-015` §5.1). Until it does, there is no way to express *"B may connect
/// to A"*, so any wider mode would be granting it to **everyone**.
///
/// **systemd's default for a unix socket is `0666` and this deliberately is
/// not.** Its whole purpose there is that other things connect, and it has
/// `SocketUser=`/`SocketMode=` to scope that. Rivet has the declaration
/// (`provides`/`connects`) and not the enforcement, and `ADR-015` §3.3's rule
/// is that a default leans closed.
pub const LISTEN_SOCKET_MODE: u32 = 0o600;

/// Where the unix socket for `provides = ["socket:<endpoint>"]` is created.
///
/// The provider never opens this path — it is handed the descriptor, exactly as
/// `listen` is (`ADR-015` §5.2). The path exists for the **consumer**, which is
/// a client and has nothing to be handed.
///
/// `ADR-015` §5.3: *"where it lives is not the application's to say"*, which is
/// why this is a function of the provider's name and the declared endpoint and
/// not anything a manifest carries.
#[must_use]
pub fn provided_socket_path(provider: &ServiceName, endpoint: &str) -> String {
    format!("{}/{endpoint}", listen_dir(provider))
}

/// What a consumer is told about the endpoints it was granted.
///
/// Space-separated `owner:name=path` entries, in the order the manifest
/// declared them — the same shape as `ADR-017` §3's `LISTEN_FDNAMES`, for the
/// same reason: an application should be able to find one endpoint among
/// several without counting.
///
/// # Why an environment variable and not a fixed path
///
/// The consumer cannot derive the path. `provided_socket_path` is a function of
/// the **provider's** name, and a consumer that hardcoded it would be depending
/// on where Rivet happens to put sockets — the thing §5.3 says is not the
/// application's to know. Telling it removes the coupling in the direction that
/// keeps working when the layout changes.
///
/// **Aura is the worked example.** Its renderer used to hardcode
/// `/run/aura/kiosk.sock` (`cpp/src/main.cpp:211`), which was not
/// where a Rivet puts it; it now reads `RIVET_CONNECT` with that path as the
/// fallback (`Aura@7fd9e26`, `cpp/src/ipc_service.hpp`).
pub const CONNECT_ENV: &str = "RIVET_CONNECT";

/// Where an application's own installed files are — its `current` directory.
///
/// **Added 2026-08-31, `PLAN.md` §104.29.** An application is installed at
/// `<volume>/app/<name>/current` and its assets are written there beside the
/// program (`package::install`), flat, because an asset name may not carry a
/// separator. Nothing told the program where that was, so an application with
/// assets could only find them by a relative path — and the working directory
/// is `/`, which is not it.
///
/// Aura is the case that found this: every font failed to open and the renderer
/// segfaulted a second later, on a board, with a GPU and a live IPC connection
/// and nothing else wrong.
///
/// # Why this and not an `env` key in the manifest
///
/// A manifest that could set arbitrary environment variables is a manifest that
/// can reach past what a signature means: `LD_PRELOAD` and `LD_LIBRARY_PATH`
/// are environment, and `ADR-015` §3.2's rule is that a manifest may not name a
/// program. This is the opposite shape — one variable, composed by the machine
/// from the application's own name, that an author cannot influence and does
/// not need to.
pub const APP_DIR_ENV: &str = "RIVET_APP_DIR";

/// Where an application may **write** — its `data` directory.
///
/// **Added 2026-09-01, `PLAN.md` §104.30.** [`APP_DIR_ENV`] names `current`,
/// which is the installed version and is read-only: right for the assets that
/// arrived in the package, wrong for anything the program produces. `ADR-015`
/// §3.2A makes [`data_dir`] the **only** path an application can write, so an
/// application that needs to keep something needs to be told this one too.
///
/// # Two variables and not one, because they are two different promises
///
/// `current` changes when a new version is installed and its contents are the
/// signed package's. `data` is a **sibling** of the version directories
/// precisely so that installing a new version replaces the program and leaves
/// what the program saved. Collapsing them would make an upgrade delete the
/// user's settings, which is the failure `data_dir`'s placement exists to
/// prevent.
///
/// Aura is again the case that found it: its controller defaulted its
/// credentials to `/etc/aura/relay.env` and its configuration to
/// `$HOME/.config/aura/config.json`. On a Rivet `/etc` is a **generated tmpfs,
/// remounted read-only** (`ADR-007` §3.5) and `$HOME` is unset, so the second
/// resolved to `/.config/...` on the read-only system slot. Neither could ever
/// be written and nothing said so — the portal offered a form that silently
/// kept nothing.
pub const APP_DATA_ENV: &str = "RIVET_APP_DATA";

/// Where this application's **settings file** is — `crate::settings::path`.
///
/// **Added 2026-09-04, when the file left `RIVET_APP_DATA`.** It used to sit
/// inside the directory [`APP_DATA_ENV`] names, so an application could compose
/// the path from what it already had; it is now beside `current`, in the
/// root-owned tree, and nothing an application is handed points at that
/// directory.
///
/// # Told, not derived, and that is the rule rather than a convenience
///
/// The Rivet SDK's first article (`ADR-036` §2) is *"an application declares what it needs
/// and is HANDED it. It does not go and take one"*, and `crate::service`'s
/// endpoint list already answers the same question the same way — **told rather
/// than assumed, so the layout can change without every consumer changing with
/// it**. An application that reached for `$RIVET_APP_DATA/../settings` would be
/// hard-coding this layout a third time, and this file has now moved once.
///
/// # It names a FILE and not a directory
///
/// [`APP_DIR_ENV`] and [`APP_DATA_ENV`] name directories because an application
/// puts several things in each. There is exactly one settings file, so naming
/// the directory would leave the filename to be agreed twice — which is the
/// disagreement `rivet-sdk`'s agreement test exists to catch, and the cheaper
/// answer is to not have it.
///
/// # An absent variable is no settings, never an error
///
/// `ADR-036` §8.5.2: **unreadable and absent are the same answer.** A process
/// this supervisor did not start has no settings, and so does one on a machine
/// where the delivery failed; both behave as though every value were unset.
pub const APP_SETTINGS_ENV: &str = "RIVET_APP_SETTINGS";

/// Where this application may put a **scratch file** — [`tmp_dir`].
///
/// **Added 2026-09-07**, with `TMPDIR` set to the same value beside it. The two
/// are not redundant and the pair is the whole point:
///
/// * **`TMPDIR` is what the world already reads.** SQLite, libc's `tmpfile`,
///   Rust's `std::env::temp_dir`, every archiver and every image codec. Setting
///   it is what makes a confined application work without having been written
///   for Rivet at all — which is the difference between an adopter porting a
///   library and an adopter discovering `SQLITE_CANTOPEN` on a board.
/// * **`RIVET_APP_TMP` is the one this project names**, on `APP_DIR_ENV`'s and
///   [`APP_DATA_ENV`]'s footing, because `ADR-036` §2's first article is *told
///   rather than derived*. An application that wanted its scratch directory
///   explicitly — to size it, to sweep it, to put a named subdirectory in it —
///   should not have to read a POSIX convention and hope this machine honours
///   it.
///
/// # Why the supervisor sets `TMPDIR` at all, having set nothing before
///
/// It set nothing, and the default was not "the system temp directory" but
/// **six failures in a row** — `SQLITE_TMPDIR`, `TMPDIR`, `/var/tmp`,
/// `/usr/tmp`, `/tmp` and the working directory, none of them writable under
/// the ruleset or present in the image. An unset variable was therefore not a
/// neutral choice; it was a promise the machine could not keep, kept quiet
/// until something reached for it. [`tmp_dir`] has the account.
pub const APP_TMP_ENV: &str = "RIVET_APP_TMP";

/// The POSIX variable set to the same value as [`APP_TMP_ENV`].
///
/// Named here rather than spelled at the one place it is pushed, so that the
/// agreement test in `rivet-sdk` can hold the two ends together the way it does
/// for every other handed value.
pub const TMPDIR_ENV: &str = "TMPDIR";

/// **Which class-matched libraries this application was joined to, and where
/// each one's object is** — the enumeration channel,
/// `PLAN-Addon-Feature-Set-2026-09-19` T2.
///
/// `<class>=<absolute path to the object>`, colon-separated, sorted by
/// reference. [`addon_env`] composes it and `rivetos_addon::host::installed`
/// reads it back.
///
/// # What was wrong with deriving the name instead
///
/// `ADR-053` puts every class-matched library's directory on
/// `LD_LIBRARY_PATH` and stops there, so until 2026-09-19 a host learned
/// **where** to look and never **which** objects were there. The missing half
/// was a naming rule — an addon of class `c` had to be the object
/// `lib<c>.so.1` — and a naming rule makes **two libraries of one class a
/// filename collision**. That is tolerable for a reading source, which
/// `.rulings/` R-200 D1 makes one-per-class by giving every role its own class
/// name; it is wrong for a background pack, where R-199 D1 makes a second pack
/// the case to support. So the supervisor says what it joined instead of
/// leaving the host to guess a filename from a class.
///
/// # Colon, because that is this project's separator for a list in the
/// environment
///
/// `LISTEN_FDNAMES` is colon-separated (`ADR-017` §3, `rivet-sdk`'s
/// `listen` module), so a reader of this project already knows what a colon
/// means in a handed variable. **No token may contain one**: a class is the
/// alphabet `rivetos_addon::abi::class_is_nameable` allows, which has no
/// colon in it, and an object path is composed from `crate::package::install`'s
/// layout out of a reference and a soname, neither of which may contain one
/// either. [`addon_env`] asserts it rather than trusting the sentence.
///
/// # An absent variable, never an empty one
///
/// [`library_path`]'s reasoning, and one more of its own: a host that reads an
/// empty `RIVET_ADDONS` and one that reads none must reach the same
/// conclusion — *nothing of my class is installed* — and an empty string is a
/// value a careless split turns into one nameless entry.
pub const ADDONS_ENV: &str = "RIVET_ADDONS";

/// Render the value of [`CONNECT_ENV`] for a set of granted endpoints.
///
/// `None` when nothing was granted, on [`library_path`]'s reasoning: an empty
/// variable is a value a program can misread, and an absent one is not.
#[must_use]
pub fn connect_env(granted: &[(String, String, String)]) -> Option<String> {
    if granted.is_empty() {
        return None;
    }
    Some(
        granted
            .iter()
            .map(|(owner, name, path)| format!("{owner}:{name}={path}"))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// The value of `LD_LIBRARY_PATH` for an application, or `None` if it declared
/// no closure. **Shared Rivets, 2026-08-31.**
///
/// The object directories of the manifest's declared closure, in the order the
/// author wrote them, joined by `:`. Nothing else is ever in it — no default
/// path, no `/system/lib`, and no directory belonging to another application.
///
/// # What this does and does not enforce
///
/// It decides what the loader **finds**, and it is the whole of the resolution
/// mechanism. It is **not isolation**: `<volume>/lib` is root-owned and
/// world-searchable, because the unprivileged uid an application runs as has to
/// traverse it, so a program that wanted to could open a library its closure
/// does not name. Enforced resolution is Android's linker namespaces and needs
/// the namespace machinery `ADR-015` §6 records as not existing here.
///
/// Saying so is `ADR-005` §5.4's rule: a partial promise about isolation is
/// worse than none. What this **does** promise is that an application's declared
/// dependencies resolve to the exact bytes it declared, which is the property
/// content addressing exists for.
///
/// # `None` rather than an empty string
///
/// An empty `LD_LIBRARY_PATH` is not the same as an absent one — the loader
/// reads it as a single empty path, which means the current directory. For a
/// program whose working directory is not its own, that is a genuine hazard and
/// not a tidiness point, so a closure of nothing produces no variable at all.
#[must_use]
pub fn library_path(volume: &str, manifest: &Manifest) -> Option<String> {
    let closure = manifest.library();
    if closure.is_empty() {
        return None;
    }
    Some(
        closure
            .iter()
            .map(|reference| crate::package::install::library_object_dir(volume, reference))
            .collect::<Vec<_>>()
            .join(":"),
    )
}

/// One library this machine has installed, as far as an addon class cares
/// about it — `PLAN-Feature-Set-2026-09-12` §6.2, `ADR-053`.
///
/// The adapter that scans `<volume>/lib` builds these; nothing in
/// `rivet-core` reads a directory (`ADR-014` §3). `class` is `None` for a
/// library whose own manifest names none — carried rather than filtered out
/// before this type exists, so [`addon_library_path`] is where *"a library
/// with no class is not joined"* is decided and tested, once, in the crate
/// that has no I/O to get wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledLibrary {
    pub reference: crate::package::library::Reference,
    pub class: Option<String>,
    /// **The object inside the directory** — the one the library's own
    /// manifest says is its addon
    /// ([`crate::package::library::Manifest::addon_object`]), read from the
    /// same manifest `class` comes from and in the same pass.
    ///
    /// The reference above names a *directory*, which was the whole of what
    /// `ADR-053` needed while a host derived the filename from the class.
    /// [`addon_env`] names a *file*, so it needs this.
    ///
    /// # Which of sixteen, and who decides
    ///
    /// `SPEC-02` §2 lets a library artifact carry one to sixteen objects,
    /// because a library may ship its own private dependencies beside the
    /// thing it provides. Exactly one of them is the addon and **the library
    /// names it** — `.rulings/` R-201, `SPEC-02` §11 — in an
    /// `addon-soname` key it may omit only when it carries a single object
    /// and there is therefore nothing to choose between. Enumerating all of
    /// them was never an option: it would make a single library look like
    /// several addons of one class and trip
    /// `rivetos_addon::host::Loaded::open`'s refusal over an artifact nobody
    /// installed twice.
    ///
    /// **The shape of this field did not change when that rule did**, and
    /// that is the point of it being one name rather than a list: what moved
    /// is where the adapter reads the name from, not what a name means here.
    ///
    /// # `None`, and why it is carried rather than refused
    ///
    /// For `class`'s reason: this type reports what an adapter found, and the
    /// crate with no I/O decides what it means. `None` is the ordinary
    /// answer for every library that provides no class at all, which is most
    /// of them. A library that provides one and still answers `None` has an
    /// object nothing can name, so [`addon_env`] leaves it out while
    /// [`addon_library_dirs`] still grants its directory, and the adapter
    /// says so once at `Info` — a shape the type admits rather than one this
    /// build's manifest grammar can produce, because that grammar refuses a
    /// class without an addon object. Saying so in code costs one `Option`;
    /// saying so with an `unwrap` costs a panic in the supervisor the first
    /// time the grammar widens.
    pub soname: Option<String>,
}

/// The addon segment of `LD_LIBRARY_PATH` — every installed library whose
/// declared class matches ANY of this application's
/// [`Manifest::accepts_classes`], and nothing else.
/// `PLAN-Feature-Set-2026-09-12` §6.2, `ADR-053`; the list widening is
/// `.rulings/` R-162 R28.
///
/// **Nothing joins by being present.** Three ways this returns `None`, each
/// the closed end of `ADR-015` §3's permission-request axis and each
/// load-bearing:
///
/// * the application's accepted-class list is empty;
/// * an installed library declares no class (`class: None`);
/// * an installed library's class matches none of the accepted list, however
///   close any one entry is — this is a byte-for-byte comparison per entry,
///   on [`crate::package::library::Reference`]'s footing.
///
/// Only when the library's one declared class appears somewhere in the
/// application's list does it join, which is the whole of `ADR-053`'s rule
/// (widened by R28) stated as code rather than as a sentence a reviewer has
/// to re-derive. **A repeated entry in the accepted list cannot join one
/// installed library twice** — this walks `installed`, not the accepted
/// list, so a duplicate class name has nothing to multiply.
///
/// # Why this is a separate function from `library_path` rather than a fold
/// into it
///
/// `library_path` resolves a *declared* closure — references the manifest
/// names by exact bytes, checked at install (`SPEC-02` §5) and never
/// searched for. This resolves a *class*: nothing in the manifest names these
/// libraries, so the set can only be found by asking every installed library
/// what it provides, which is a different question with a different failure
/// mode (an unmatched class is silent, not a refusal). Keeping them apart
/// means a reader of either function sees one rule rather than two folded
/// into an `if`.
///
/// # Determinism
///
/// [`addon_matches`] sorts by reference, so two runs over one directory
/// listing produce the same string regardless of the order the adapter's
/// `list_dir` happened to return.
#[must_use]
pub fn addon_library_path(
    volume: &str,
    manifest: &Manifest,
    installed: &[InstalledLibrary],
) -> Option<String> {
    let dirs = addon_library_dirs(volume, manifest, installed);
    if dirs.is_empty() {
        return None;
    }
    Some(dirs.join(":"))
}

/// **Every installed library this application's accepted classes join, in the
/// order they will be spoken about** — and the reason this is one function
/// rather than three is a defect that reached a board.
///
/// Three things are composed from this list and they must not be able to
/// disagree:
///
/// 1. [`addon_library_path`], which is what the loader **searches**;
/// 2. `crate::confinement::policy`'s ruleset, which is what the kernel lets
///    the loader **open**;
/// 3. [`addon_env`], which is what the host is **told is there**.
///
/// On 2026-09-19 the first two disagreed. The first article
/// (`examples/probes/addon-probe-host`) was handed a correct path to a real
/// object on `rivet-06f5c` and the `dlopen` failed anyway, because the policy
/// granted the DECLARED closure and nothing else — the search path and the
/// confinement disagreed, which is the one thing the comment above that loop
/// said could not happen. The third was added the same day and joins them on
/// the same footing: a directory reaches the loader's path exactly when it
/// reaches the ruleset and exactly when it is named.
///
/// # Determinism
///
/// Sorted by reference, so two runs over one directory listing produce the
/// same list regardless of the order the adapter's `list_dir` happened to
/// return — `ADR-007` §3's determinism argument, applied to a set built from a
/// directory rather than from a manifest's own list. Sorting **here** rather
/// than in each composer is what makes the three agree on order as well as on
/// membership.
#[must_use]
pub fn addon_matches<'a>(
    manifest: &Manifest,
    installed: &'a [InstalledLibrary],
) -> Vec<&'a InstalledLibrary> {
    let accepts = manifest.accepts_classes();
    if accepts.is_empty() {
        return Vec::new();
    }
    let mut matches: Vec<&InstalledLibrary> = installed
        .iter()
        .filter(|lib| {
            lib.class
                .as_deref()
                .is_some_and(|class| accepts.iter().any(|a| a == class))
        })
        .collect();
    matches.sort_by(|a, b| a.reference.cmp(&b.reference));
    matches
}

/// **The matched libraries' object directories** — [`addon_library_path`]'s
/// entries before they are joined, and `crate::confinement::policy`'s grants.
///
/// See [`addon_matches`] for why both come through one list.
#[must_use]
pub fn addon_library_dirs(
    volume: &str,
    manifest: &Manifest,
    installed: &[InstalledLibrary],
) -> Vec<String> {
    addon_matches(manifest, installed)
        .into_iter()
        .map(|lib| crate::package::install::library_object_dir(volume, &lib.reference))
        .collect()
}

/// **The value of [`ADDONS_ENV`]** — every joined library named by its class
/// and by the absolute path of its object, or `None` when nothing joined.
///
/// `<class>=<path>`, colon-separated, in [`addon_matches`]' order. The class
/// is the library's own declared class rather than the entry from the
/// application's accepted list that matched it, because those are the same
/// bytes — matching is exact (`SPEC-02` §11) — and the library's is the one
/// a host can compare against what the object then says it is when asked to
/// describe itself.
///
/// # A library with no object is left out rather than named emptily
///
/// [`InstalledLibrary::soname`] has the account. An entry `class=` with
/// nothing after it would be a path a host would try to open, and the failure
/// would arrive from the loader rather than from the machine that already knew.
///
/// # Why this is a variable and not a call
///
/// The same argument `ADR-036` §2 makes for every other handed value: an
/// application is told what it has at the moment it is started, by the thing
/// that decided it. A call would make the answer depend on the supervisor
/// still answering — `.rulings/` R-199 D5's objection to an instrument that
/// depends on what it measures — and the answer cannot change while the
/// process runs anyway, because `LD_LIBRARY_PATH` and the ruleset are fixed at
/// `exec` alongside it.
#[must_use]
pub fn addon_env(
    volume: &str,
    manifest: &Manifest,
    installed: &[InstalledLibrary],
) -> Option<String> {
    let entries: Vec<String> = addon_matches(manifest, installed)
        .into_iter()
        .filter_map(|lib| {
            let class = lib.class.as_deref()?;
            let soname = lib.soname.as_deref()?;
            let dir = crate::package::install::library_object_dir(volume, &lib.reference);
            let entry = format!("{class}={dir}/{soname}");
            // **No token may contain the separator**, the rule `LISTEN_FDNAMES`
            // already lives under. Everything on the left of the `=` is a class
            // and everything on the right is composed from a reference and a
            // soname, and none of those three alphabets has a colon in it — so
            // an entry that contains one means a grammar widened somewhere
            // upstream, and the honest answer is to drop the entry rather than
            // to hand a host a list it will split in the wrong place.
            (!entry.contains(':')).then_some(entry)
        })
        .collect();
    if entries.is_empty() {
        return None;
    }
    Some(entries.join(":"))
}

// ---------------------------------------------------------------------------
// Who an application runs as
// ---------------------------------------------------------------------------

/// The lowest uid an application may be given.
///
/// **4000, and the number is read out of the record rather than chosen here.**
/// `ADR-015` §5.4's worked example of the grant an operator confirms is
/// `grants:   uid 4021, private mount ns, no capabilities, 128M memory cap`.
/// That is the only application uid the record has ever written down, so the
/// range it implies is the range this uses.
///
/// It also has to miss three things that are already claimed, and it does:
/// **0** is `root` and every file in the image (`crate::etc::SYSTEM_IDENTITIES`);
/// **1–999** is the conventional system range, which musl and every
/// tool an operator has used before treat as "not a person"; and **1000–3999**
/// is where human accounts conventionally start, which is where `ADR-008`
/// §7.3's deferred human half will have to put them.
pub const APP_UID_BASE: u32 = 4000;

/// The highest uid an application may be given.
///
/// `65533`, which stops **two** short of `u16::MAX` rather than one, and both
/// gaps are deliberate:
///
/// * `65535` is `(uid_t)-1`, the value `setuid(2)` and `chown(2)` use to mean
///   *"do not change this"*. A process given it would not fail; it would run as
///   whatever it already was, which on this path is uid 0.
/// * `65534` is `nobody` on essentially every Unix. Rivet does not define it
///   (`crate::etc::SYSTEM_IDENTITIES` is one row long and says why), but an
///   application silently sharing an identity with the conventional
///   nothing-user is the kind of coincidence nobody looks for afterwards.
///
/// The range is wide on purpose — see [`app_identity`] on collisions. A narrow
/// span looks tidier and makes a collision *likely* rather than remote.
pub const APP_UID_LAST: u32 = 65_533;

/// How many identities [`app_identity`] can produce.
pub const APP_UID_SPAN: u32 = APP_UID_LAST - APP_UID_BASE + 1;

/// Who an application runs as: a uid, and a group of its own.
///
/// **`gid` equals `uid`, and that is the user-private-group idiom rather than
/// laziness.** A per-application group with no other members means the
/// application's own files are reachable by exactly one identity, and it means
/// there is never a question about which supplementary groups a service
/// inherited — the supervisor sets the list to *empty* before it sets the gid.
/// Android does the same thing for the same reason (`ADR-015` §5.1: *"Exactly
/// Android's primary mechanism"*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AppIdentity {
    pub uid: u32,
    pub gid: u32,
}

/// The identity an application called `name` has — **on this machine and on
/// every other one, for ever.**
///
/// # Derived, not allocated, and that is the decision
///
/// `ADR-015` §5.1 requires *"UID/GID per application"* and `ADR-008` §7.3
/// requires that a service account **arrive with the service that needs it**
/// and not be pre-created. Neither says where the number comes from. Two shapes
/// answer it and this takes the first; `PLAN.md` §0.11 states the other and
/// what it would cost, because `AGENTS.md` §1 makes the choice the maintainer's
/// to overturn rather than an agent's to bury.
///
/// | | **Derived — this** | **Allocated and recorded** |
/// | :-- | :-- | :-- |
/// | Stable across a reboot | by construction — it is a function | as long as the record survives |
/// | Stable across a reinstall of the same application | **yes** | only if the record is not part of what was removed |
/// | Needs state | none | a file, an allocator, and a lock |
/// | Two applications can collide | **yes, and it must be detected** | no |
/// | Authority for "what uid is `photo-index`" | its **name** | a file, which is a second answer beside the name |
///
/// The property that decides it is the one the failure is about: an application
/// whose uid changed between boots cannot read its own files, and *the machine
/// would not report anything* — it would report `EACCES` from inside the
/// application, at a moment nobody is watching. A pure function of the name has
/// no way to produce that, and `AGENTS.md` §5's *derive, never store* points the
/// same way.
///
/// # The collision, said plainly
///
/// Two different names can hash to one uid, and two applications sharing a uid
/// is not a weak sandbox — it is **no sandbox between those two**, since each
/// can read and rewrite the other's private data and each answers the other's
/// `SO_PEERCRED` as itself (`ADR-015` §5.3). So it is **detected and the second
/// application is refused**, by whoever holds every definition at once —
/// `cmd/rivet-sysd`'s `applications` module, which is the same place
/// and the same argument as the socket-collision check beside it.
///
/// [`identities_collide`] is the predicate. The refusal is not optional and it
/// is not a warning: `ADR-015` §6.1 asks that an unenforceable declaration be
/// reported rather than silently accepted, and *starting* an application whose
/// isolation has silently failed is the wrong side of that rule.
///
/// # This is not a security primitive, and the distinction matters
///
/// `fnv1a64` is a *dispersal* function, not a cryptographic one, and
/// `rivet-core` could not use `rivet-crypto`'s BLAKE3 anyway — its
/// `[dependencies]` table is empty by rule (`AGENTS.md` §10.1). Nothing here
/// resists an adversary who chooses a name to collide with an existing
/// application's, and nothing needs to: putting an application on a Rivet
/// requires a signature from the trust set (`ADR-015` §4), so the party who
/// chooses names is the party who is trusted, and a chosen collision is caught
/// by exactly the check an accidental one is.
#[must_use]
pub fn app_identity(name: &ServiceName) -> AppIdentity {
    // `% APP_UID_SPAN` on a `u64` then narrowed: the remainder is below
    // `APP_UID_SPAN`, which is a `u32`, so the cast cannot truncate. Written
    // with `try_from` rather than `as` because `rivet-core` denies nothing
    // and gains nothing from a silent cast.
    let offset =
        u32::try_from(fnv1a64(name.as_str().as_bytes()) % u64::from(APP_UID_SPAN)).unwrap_or(0);
    let id = APP_UID_BASE + offset;
    AppIdentity { uid: id, gid: id }
}

/// **The group that may connect to a SHARED endpoint** — `.rulings/` R-206,
/// and `Endpoint::shared` is where the need for it is argued.
///
/// An ordinary endpoint is granted by chowning its socket to the one consumer.
/// A shared one cannot be: a compositor exists so that several applications
/// hold surfaces, and a `chown` names one uid. So a shared socket is
/// `root:<this gid>` at `0660`, and every consumer the supervisor grants is
/// spawned with this as its **one** supplementary group.
///
/// **The declaration stays enforcement, which is the point.** An application
/// that did not put the endpoint in its `connects` is not given the group and
/// cannot connect — the same property the chown had, held by a different
/// mechanism. What is emphatically NOT done here is `0666`: every application
/// on the machine is granted `/run/rivet` by its Landlock ruleset, so a
/// world-writable socket would turn the manifest declaration from a capability
/// into a comment.
///
/// # Derived from BOTH names, in the applications' own span
///
/// [`app_identity`]'s argument applies unchanged — derived, never stored, so a
/// reboot cannot move it and no record can be lost. It hashes
/// `"<provider>:<endpoint>"` rather than either half, because two providers may
/// each offer an endpoint called `wayland` and those are different groups.
///
/// # It shares the application span, so collisions are possible and refused
///
/// An endpoint group can land on an application's gid, or on another shared
/// endpoint's group. Either is an ambiguity [`identities_collide`] cannot see,
/// because that predicate compares application names only: an application
/// whose own gid is an endpoint's group passes the socket's `0660` check
/// without declaring `connects`, and two endpoints on one group admit each
/// other's consumers. `cmd/rivet-sysd`'s `applications` module therefore
/// compares every application gid and every shared endpoint group it can see
/// before anything is started, and refuses the definition that would make one
/// gid mean two things. The number itself cannot move to avoid a clash: like
/// [`app_identity`], it is an on-disk contract.
#[must_use]
pub fn endpoint_gid(provider: &str, endpoint: &str) -> u32 {
    let mut key = String::with_capacity(provider.len() + endpoint.len() + 1);
    key.push_str(provider);
    key.push(':');
    key.push_str(endpoint);
    let offset = u32::try_from(fnv1a64(key.as_bytes()) % u64::from(APP_UID_SPAN)).unwrap_or(0);
    APP_UID_BASE + offset
}

/// Whether two application names claim one identity.
///
/// `ADR-011` in the small: one uid is one thing, and two names that produce it
/// are two authorities over *who this process is*. See [`app_identity`] for why
/// this can happen at all and what the caller must do about it.
///
/// A name never collides with itself — that is the same application, discovered
/// twice, which is a different fault and one the filesystem cannot produce.
#[must_use]
pub fn identities_collide(a: &ServiceName, b: &ServiceName) -> bool {
    a != b && app_identity(a) == app_identity(b)
}

/// FNV-1a, 64-bit, in full — the constants are the published ones.
///
/// Written out rather than depended on because `rivet-core` has an empty
/// `[dependencies]` table by rule (`AGENTS.md` §10.1), and chosen over anything
/// larger because what is wanted is *dispersal of short ASCII names*, which is
/// the case FNV was designed for and is measurably good at. See
/// [`app_identity`] for why this is deliberately not a cryptographic hash.
///
/// **It is part of the on-disk contract even though nothing writes it down.**
/// Changing this function changes every application's uid, which orphans every
/// application's data directory on every machine already running one. Treat it
/// as a format.
pub(crate) const fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(PRIME);
        i += 1;
    }
    hash
}

/// The directory name an application's own writable state lives under.
///
/// `ADR-015` §3.2's `state = "private"` says an application *"gets
/// `/state/app/<name>`, mode 700"*, and §7 then puts the installed **version**
/// under that same directory behind a `current` link. Both cannot be the whole
/// of `/state/app/<name>`: one is replaced wholesale by an update and the other
/// must survive it.
///
/// So the writable half is a sibling of the versions, and **this name is chosen
/// here** — the fourth name `PLAN.md` §0.4.3(d) records the record not giving,
/// alongside `current`, `bin` and `manifest.toml`. It is the installer's ADR
/// that should eventually state it.
pub const DATA_DIR: &str = "data";

/// `<volume>/app/<name>/data` — the one directory an application may write.
///
/// Owned by [`app_identity`]'s uid and gid, mode `0700`: reachable by that
/// application and by root, and by nothing else. It is **outside** the `current`
/// link on purpose, so that installing a new version replaces the program and
/// leaves the data where it was.
#[must_use]
pub fn data_dir(volume: &str, name: &ServiceName) -> String {
    format!("{}/{DATA_DIR}", app_dir(volume, name))
}

/// The mode [`data_dir`] is created with. `ADR-015` §3.2: *"mode 700"*.
pub const DATA_DIR_MODE: u32 = 0o700;

/// `tmp` — the scratch directory's name beside `data` and `current`.
///
/// **Ruled 2026-09-07**, after a whole-database restore on the test board died
/// with `SQLITE_CANTOPEN`. An application is confined to one writable directory
/// and, until this existed, was never told that it was the only one: every C
/// and POSIX convention for *somewhere to put a scratch file* pointed at a path
/// the Landlock ruleset refuses. SQLite looks in `SQLITE_TMPDIR`, `TMPDIR`,
/// `/var/tmp`, `/usr/tmp`, `/tmp` and then the working directory, and on a
/// Rivet **all six failed at once** — the two variables unset, two directories
/// absent from the image, `/tmp` present and `1777` but outside the ruleset,
/// and the working directory the read-only `current`.
///
/// **The cost was never the refusal, which is correct.** It was that what
/// surfaces is an error from whichever library wanted the file — here a result
/// code naming no path and reading like a corrupt database.
pub const TMP_DIR: &str = "tmp";

/// `<volume>/app/<name>/tmp` — where an application may put a scratch file.
///
/// A **sibling of [`data_dir`] rather than a child of it**, and that placement
/// is the decision. Pointing `TMPDIR` inside the data directory would have cost
/// nothing and worked today, but it makes an application's persistent state
/// double as its scratch space: a crash leaves debris in the directory holding
/// the history, the writes land on the same SD-card wear budget, and nothing
/// could ever empty one without risking the other. Separate, it can be emptied
/// wholesale — and of the two placements considered, this is the one that can
/// later become a tmpfs.
///
/// # It is a PLAIN DIRECTORY on `/state`, and a tmpfs only on three conditions
///
/// A tmpfs is the better endgame and this layout is chosen to leave room for
/// it: a kernel-enforced `size=`, no SD-card wear, and reclamation at unmount
/// rather than by anybody remembering. **Three things have to be settled
/// first**, and none of them is settled by wanting it:
///
/// * **A size**, which is a manifest key, which is a change to the application
///   contract and so a ruling of its own.
/// * **What overflow means.** A tmpfs converts a wear problem into an
///   out-of-memory one on a 4 GB board. Today a restore that needs more scratch
///   than expected is slow; on a tmpfs it is `ENOSPC`, and nobody has yet
///   measured how large an application's scratch actually gets.
/// * **A mount per application**, re-established every boot and torn down on
///   uninstall. A directory simply exists, and on a machine with no mount
///   namespaces (`ADR-042`) every one of those mounts is global.
///
/// # It is emptied at the lifecycle edges, and NOT on a timer
///
/// `crate::service` does not do the emptying; `rivet-sysd` does, immediately
/// before a spawn and immediately after an exit. Those are the two moments the
/// supervisor knows for certain that nothing is using the directory.
///
/// **A periodic sweep was considered and rejected.** `ADR-039` §2 separates a
/// predicate over the clock from an event that fires, and this is neither: the
/// moment it should happen is a lifecycle transition the supervisor already
/// observes, so it needs none of a scheduler's missed-occurrence or overlap
/// semantics. Worse, a timed sweep has to GUESS whether a file is in use by a
/// live process, and guessing wrong deletes a running operation's scratch file
/// mid-write. If an application ever leaks scratch while it is up, the answer
/// is a size cap — which refuses the write and lets the application see it —
/// and not a deleter that corrupts an operation silently.
#[must_use]
pub fn tmp_dir(volume: &str, name: &ServiceName) -> String {
    format!("{}/{TMP_DIR}", app_dir(volume, name))
}

/// The mode [`tmp_dir`] is created with — [`DATA_DIR_MODE`]'s, for its reason.
pub const TMP_DIR_MODE: u32 = 0o700;

/// **The mark a thing carries between its removal and its deletion** —
/// `ADR-017` §3.
///
/// `rivet-sysd` removes a directory by renaming it, inside the directory that
/// held it, to a name carrying this mark, and a short-lived helper deletes it
/// afterwards. A removed application, library or repository is renamed to
/// `.reclaim-<ms>-<n>`, beginning with the mark; an application's emptied
/// scratch directory to `tmp.reclaim-<ms>-<n>`.
///
/// **A leading `.` is what makes a removed thing's name unable to collide with
/// a live one.** No live name under an application root, a library root or a
/// repository root can begin with one: [`ServiceName`],
/// `crate::package::library::Reference` and `crate::repo::RepoName` each
/// require a letter or a digit first, and an installed version may not carry
/// the mark at all.
pub const RECLAIM_MARK: &str = ".reclaim-";

/// **Whether a directory entry is a removed thing awaiting deletion** rather
/// than a live name — see [`RECLAIM_MARK`].
///
/// Every reader that lists a directory a removal renames inside passes over
/// such an entry without a word: it is gone from the machine's point of view
/// from the moment it was renamed.
#[must_use]
pub fn is_set_aside(entry: &str) -> bool {
    entry.starts_with(RECLAIM_MARK)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn limit(text: &str) -> MemoryLimit {
        MemoryLimit::parse(text).expect("a valid fixture")
    }

    fn svc(name: &str) -> ServiceName {
        ServiceName::new(name).expect("a valid fixture name")
    }

    // ---- the ruling -------------------------------------------------------

    /// **The ruling itself, as a test.** Ruled 2026-08-24: where both name
    /// `memory`, the lower wins — whichever side it is on.
    #[test]
    fn the_lower_memory_value_wins_whichever_side_states_it() {
        let low = limit("64M");
        let high = limit("512M");

        let operator_is_lower = effective_memory(Some(high), Some(low));
        assert_eq!(operator_is_lower.limit, Some(low));
        assert_eq!(operator_is_lower.source, MemorySource::DocumentLower);
        assert!(operator_is_lower.source.operator_tightened());

        let application_is_lower = effective_memory(Some(low), Some(high));
        assert_eq!(
            application_is_lower.limit,
            Some(low),
            "an operator's higher cap must not raise an application's own request — that would \
             be the document rewriting a signed value (ADR-015 §4.1)"
        );
        assert_eq!(application_is_lower.source, MemorySource::ManifestLower);
        assert!(!application_is_lower.source.operator_tightened());
    }

    #[test]
    fn one_side_alone_is_the_answer_and_says_which_side() {
        let l = limit("128M");
        assert_eq!(
            effective_memory(Some(l), None),
            EffectiveMemory {
                limit: Some(l),
                source: MemorySource::Manifest
            }
        );
        assert_eq!(
            effective_memory(None, Some(l)),
            EffectiveMemory {
                limit: Some(l),
                source: MemorySource::Document
            }
        );
    }

    /// No cap is a real answer. It must never become a cap of zero, which is a
    /// service that may allocate nothing.
    #[test]
    fn neither_side_stating_one_is_no_cap_rather_than_a_cap_of_zero() {
        assert_eq!(effective_memory(None, None), EffectiveMemory::UNSTATED);
        assert_eq!(EffectiveMemory::UNSTATED.limit, None);
    }

    /// Both naming the same number is neither side overriding the other, and
    /// saying "the document won" would invite a reader to believe the
    /// application had asked for something else.
    #[test]
    fn two_equal_values_are_reported_as_agreement_rather_than_as_an_override() {
        let l = limit("128M");
        assert_eq!(
            effective_memory(Some(l), Some(l)).source,
            MemorySource::Agreed
        );
    }

    /// The effective cap can only ever tighten as the operator lowers theirs —
    /// the property *"a cap runs the other way"* is really making.
    #[test]
    fn an_operators_cap_can_only_tighten_never_loosen() {
        let request = limit("256M");
        let mut previous = u64::MAX;
        for text in ["1G", "512M", "256M", "255M", "64M", "1"] {
            let cap = limit(text);
            let effective = effective_memory(Some(request), Some(cap))
                .limit
                .expect("both sides stated one");
            assert!(
                effective.bytes() <= request.bytes(),
                "{text} raised the application's own request to {effective}"
            );
            assert!(
                effective.bytes() <= previous,
                "{text} loosened a tighter cap"
            );
            previous = effective.bytes();
        }
    }

    // ---- cpu: the same ruling, mirrored rather than re-derived ------------

    fn cores(text: &str) -> CpuLimit {
        CpuLimit::parse(text).expect("a valid fixture")
    }

    /// `effective_cpu`'s version of `the_lower_memory_value_wins_whichever_
    /// side_states_it` — same ruling, `ADR-007` §7 and `ADR-015` §3's
    /// 2026-09-09 amendment, same test shape.
    #[test]
    fn the_lower_cpu_value_wins_whichever_side_states_it() {
        let low = cores("0.5");
        let high = cores("2");

        let operator_is_lower = effective_cpu(Some(high), Some(low));
        assert_eq!(operator_is_lower.limit, Some(low));
        assert_eq!(operator_is_lower.source, CpuSource::DocumentLower);
        assert!(operator_is_lower.source.operator_tightened());

        let application_is_lower = effective_cpu(Some(low), Some(high));
        assert_eq!(
            application_is_lower.limit,
            Some(low),
            "an operator's higher cap must not raise an application's own request — that would \
             be the document rewriting a signed value (ADR-015 §4)"
        );
        assert_eq!(application_is_lower.source, CpuSource::ManifestLower);
        assert!(!application_is_lower.source.operator_tightened());
    }

    #[test]
    fn one_cpu_side_alone_is_the_answer_and_says_which_side() {
        let c = cores("1.5");
        assert_eq!(
            effective_cpu(Some(c), None),
            EffectiveCpu {
                limit: Some(c),
                source: CpuSource::Manifest
            }
        );
        assert_eq!(
            effective_cpu(None, Some(c)),
            EffectiveCpu {
                limit: Some(c),
                source: CpuSource::Document
            }
        );
    }

    /// No cap is a real answer, on `neither_side_stating_one_is_no_cap_
    /// rather_than_a_cap_of_zero`'s footing: `cpu.max` stays `max` rather
    /// than becoming a quota of zero, which would be a service that can
    /// never be scheduled.
    #[test]
    fn neither_cpu_side_stating_one_is_no_cap_rather_than_a_cap_of_zero() {
        assert_eq!(effective_cpu(None, None), EffectiveCpu::UNSTATED);
        assert_eq!(EffectiveCpu::UNSTATED.limit, None);
    }

    #[test]
    fn two_equal_cpu_values_are_reported_as_agreement_rather_than_as_an_override() {
        let c = cores("1");
        assert_eq!(effective_cpu(Some(c), Some(c)).source, CpuSource::Agreed);
    }

    /// `an_operators_cap_can_only_tighten_never_loosen`'s property, asked
    /// about a core count instead of a byte count.
    #[test]
    fn an_operators_cpu_cap_can_only_tighten_never_loosen() {
        let request = cores("2");
        let mut previous = u64::MAX;
        for text in ["4", "2", "1.5", "1", "0.5", "0.00001"] {
            let cap = cores(text);
            let effective = effective_cpu(Some(request), Some(cap))
                .limit
                .expect("both sides stated one");
            assert!(
                effective.quota_us() <= request.quota_us(),
                "{text} raised the application's own request to {effective}"
            );
            assert!(
                effective.quota_us() <= previous,
                "{text} loosened a tighter cap"
            );
            previous = effective.quota_us();
        }
    }

    /// The worked table in [`CpuLimit`]'s own documentation, as a test rather
    /// than a comment somebody could let drift.
    #[test]
    fn the_worked_arithmetic_from_cores_to_a_quota_is_exactly_what_the_documentation_says() {
        let cases = [
            ("0.5", 50_000),
            ("2", 200_000),
            ("0.25", 25_000),
            ("1", 100_000),
        ];
        for (text, quota_us) in cases {
            assert_eq!(
                CpuLimit::parse(text).map(CpuLimit::quota_us),
                Some(quota_us),
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_cpu_limit_of_zero_is_refused_however_it_is_spelled() {
        for zero in ["0", "0.0", "0.00000"] {
            assert_eq!(CpuLimit::parse(zero), None, "{zero:?} was accepted");
        }
    }

    #[test]
    fn a_cpu_fraction_has_one_spelling() {
        for wrong in [
            "0.50", "2.0", "02", "1.", ".5", "1.000000", "-1", "1 ", " 1", "1,5", "",
        ] {
            assert_eq!(CpuLimit::parse(wrong), None, "{wrong:?} was accepted");
        }
    }

    /// Five digits is the finest grain [`CpuLimit::PERIOD_US`] can represent;
    /// a sixth is a value this period cannot hold rather than one that gets
    /// rounded into being.
    #[test]
    fn a_sixth_decimal_digit_is_refused_rather_than_rounded() {
        assert_eq!(CpuLimit::parse("0.000001"), None);
        assert_eq!(CpuLimit::parse("0.123456"), None);
    }

    #[test]
    fn a_cpu_limit_spells_itself_with_no_trailing_zero() {
        let cases = [
            ("1", "1"),
            ("0.5", "0.5"),
            ("2", "2"),
            ("0.25", "0.25"),
            ("1.5", "1.5"),
            ("0.00001", "0.00001"),
        ];
        for (wrote, canonical) in cases {
            assert_eq!(cores(wrote).to_string(), canonical, "{wrote:?}");
        }
    }

    /// `every_spelling_a_limit_renders_is_one_it_parses_back_to`'s property,
    /// asked of a quota instead of a byte count: `parse` then `Display` then
    /// `parse` again must land on the exact quota that started, for every
    /// quota one of these WRITTEN spellings can produce.
    #[test]
    fn every_cpu_spelling_a_limit_renders_is_one_it_parses_back_to() {
        for text in [
            "1", "2", "0.5", "0.25", "1.5", "0.00001", "0.99999", "1000",
        ] {
            let l = CpuLimit::parse(text).unwrap_or_else(|| panic!("{text} is a valid fixture"));
            let rendered = l.to_string();
            assert!(!rendered.ends_with('0') || !rendered.contains('.'), "{text:?} rendered as {rendered:?}");
            assert_eq!(
                CpuLimit::parse(&rendered).map(CpuLimit::quota_us),
                Some(l.quota_us()),
                "{text:?} rendered as {rendered:?}, which does not parse back to the same quota"
            );
        }
    }

    #[test]
    fn a_cpu_limit_that_would_overflow_is_refused_rather_than_wrapping() {
        assert_eq!(CpuLimit::parse("18446744073709551615"), None);
        assert_eq!(CpuLimit::parse("99999999999999999999999"), None);
    }

    // ---- D22 (`.rulings/` R-210): a cap the kernel will not keep ----------

    /// **`CpuLimit::parse` still accepts a quota under the kernel's
    /// minimum** — pinned so nobody later "fixes" the parser and strands a
    /// machine whose document or manifest already carries one. `.rulings/`
    /// R-210 D22 refuses this at the writers, not here: a boot-time parse of
    /// an already-installed manifest or an already-saved document must keep
    /// succeeding.
    #[test]
    fn parse_still_accepts_a_quota_under_the_kernel_minimum() {
        let limit = CpuLimit::parse("0.00001").expect("one microsecond of quota still parses");
        assert_eq!(limit.quota_us(), 1);
        assert!(limit.is_below_kernel_minimum());
    }

    /// The predicate every writer calls, at the boundary: one microsecond
    /// under the minimum is below it, the minimum itself is not, and one
    /// microsecond over is not either.
    #[test]
    fn is_below_kernel_minimum_is_exact_at_the_boundary() {
        let below = CpuLimit::parse("0.00999").expect("999 microseconds parses");
        assert_eq!(below.quota_us(), CpuLimit::KERNEL_MIN_QUOTA_US - 1);
        assert!(below.is_below_kernel_minimum());

        let at = CpuLimit::parse("0.01").expect("1000 microseconds parses");
        assert_eq!(at.quota_us(), CpuLimit::KERNEL_MIN_QUOTA_US);
        assert!(!at.is_below_kernel_minimum());

        let above = CpuLimit::parse("0.01001").expect("1001 microseconds parses");
        assert_eq!(above.quota_us(), CpuLimit::KERNEL_MIN_QUOTA_US + 1);
        assert!(!above.is_below_kernel_minimum());
    }

    /// **The one explanation every writer quotes** — it names the quota this
    /// value comes to and the kernel's own floor, so a refusal at packaging,
    /// at `apply edit`, and at `rivet-fleetd`'s commit all read the same
    /// reason.
    #[test]
    fn kernel_refusal_names_the_quota_and_the_minimum() {
        let limit = CpuLimit::parse("0.00001").expect("one microsecond of quota still parses");
        let why = limit.kernel_refusal();
        assert!(why.contains('1'), "{why}");
        assert!(why.contains(&CpuLimit::PERIOD_US.to_string()), "{why}");
        assert!(why.contains(&CpuLimit::KERNEL_MIN_QUOTA_US.to_string()), "{why}");
    }

    // ---- the unit ---------------------------------------------------------

    /// The two grammars are the same one, so a manifest an artifact accepts is
    /// one this can value. If `Memory::parse` ever widens, this fails rather
    /// than a machine silently getting no cap where a cap was asked for.
    #[test]
    fn every_shape_the_artifact_accepts_is_one_this_module_can_value() {
        for text in ["1", "1K", "128M", "1G", "4096", "65535G"] {
            let request = Memory::parse(text).expect("the artifact accepts it");
            assert!(
                MemoryLimit::of(&request).is_some(),
                "{text} is a manifest Rivet could not value"
            );
        }
    }

    /// And the one place they differ, stated rather than discovered: a zero.
    /// The artifact's parser takes the shape; this one refuses a limit that
    /// would let a service allocate nothing.
    #[test]
    fn a_request_of_zero_is_valued_at_nothing_rather_than_at_zero_bytes() {
        let zero = Memory::parse("0").expect("a shape the artifact accepts");
        assert_eq!(MemoryLimit::of(&zero), None);
    }

    #[test]
    fn the_records_own_spelling_of_a_memory_limit_parses() {
        // `ADR-015` §3.2 writes exactly this.
        assert_eq!(
            MemoryLimit::parse("128M").map(MemoryLimit::bytes),
            Some(128 * 1024 * 1024)
        );
    }

    #[test]
    fn the_suffixes_count_in_multiples_of_1024() {
        assert_eq!(MemoryLimit::parse("1").map(MemoryLimit::bytes), Some(1));
        assert_eq!(MemoryLimit::parse("1K").map(MemoryLimit::bytes), Some(1024));
        assert_eq!(
            MemoryLimit::parse("1M").map(MemoryLimit::bytes),
            Some(1024 * 1024)
        );
        assert_eq!(
            MemoryLimit::parse("1G").map(MemoryLimit::bytes),
            Some(1024 * 1024 * 1024)
        );
    }

    #[test]
    fn a_memory_limit_of_zero_is_refused_however_it_is_spelled() {
        for zero in ["0", "0K", "0M", "0G"] {
            assert_eq!(MemoryLimit::parse(zero), None, "{zero:?} was accepted");
        }
    }

    #[test]
    fn a_memory_suffix_has_one_spelling() {
        for wrong in [
            "128m", "128k", "128g", "128MB", "128 M", "128MiB", "M", "", "0128M",
        ] {
            assert_eq!(MemoryLimit::parse(wrong), None, "{wrong:?} was accepted");
        }
    }

    #[test]
    fn a_memory_limit_spells_itself_with_the_largest_suffix_that_divides_it() {
        let cases = [
            ("1", "1"),
            ("1023", "1023"),
            ("1024", "1K"),
            ("1025", "1025"),
            ("134217728", "128M"),
            ("128M", "128M"),
            ("1024M", "1G"),
            ("1G", "1G"),
            ("2G", "2G"),
        ];
        for (wrote, canonical) in cases {
            assert_eq!(limit(wrote).to_string(), canonical, "{wrote:?}");
        }
    }

    #[test]
    fn every_spelling_a_limit_renders_is_one_it_parses_back_to() {
        let interesting = [
            1_u64,
            2,
            1023,
            1024,
            1025,
            4096,
            1024 * 1024 - 1,
            1024 * 1024,
            128 * 1024 * 1024,
            1024 * 1024 * 1024,
            3 * 1024 * 1024 * 1024,
            u64::MAX,
        ];
        for bytes in interesting {
            let l = MemoryLimit::parse(&bytes.to_string())
                .unwrap_or_else(|| panic!("{bytes} is a valid byte count"));
            let text = l.to_string();
            assert!(!text.starts_with('0'), "{bytes} rendered as {text:?}");
            assert_eq!(
                MemoryLimit::parse(&text),
                Some(l),
                "{bytes} rendered as {text:?}, which does not parse back"
            );
        }
    }

    #[test]
    fn a_memory_limit_that_would_overflow_is_refused_rather_than_wrapping() {
        assert_eq!(MemoryLimit::parse("18446744073709551615G"), None);
        assert_eq!(MemoryLimit::parse("99999999999999999999999"), None);
    }

    // ---- names ------------------------------------------------------------

    #[test]
    fn every_service_name_the_record_actually_uses_is_accepted() {
        // `ADR-017` §2.1's inventory, plus the Cargo package name shape an
        // application service is named after (`ADR-015` §3.2).
        for name in [
            "rivet-mcpd",
            "rivet-netd",
            "rivet-timed",
            "rivet-apid",
            "photo-index",
            "a",
            "app_1",
            "app.v2",
            "9lives",
        ] {
            assert!(
                ServiceName::new(name).is_some(),
                "{name:?} is a real service name and was refused"
            );
        }
    }

    #[test]
    fn a_name_that_would_escape_its_cgroup_its_socket_or_its_directory_is_refused() {
        // `ADR-017` §3.1.3 — the name is a cgroup path component, and since
        // 2026-08-24 a directory under `/state/app` as well. A separator nests
        // one service's cgroup inside another's, or escapes the app root.
        for bad in ["a/b", ".", "..", "../escape", "/abs", "a\\b"] {
            assert_eq!(ServiceName::new(bad), None, "{bad:?} was accepted");
        }
    }

    #[test]
    fn a_name_that_is_not_a_usable_filename_is_refused() {
        for bad in [
            "", " ", "a b", "a\nb", "a\0b", "a\u{7f}b", "café", "-lead", "_lead", ".lead",
        ] {
            assert_eq!(ServiceName::new(bad), None, "{bad:?} was accepted");
        }
    }

    #[test]
    fn a_name_too_long_for_a_unix_socket_path_is_refused() {
        let longest = "a".repeat(ServiceName::MAX_LEN);
        assert!(ServiceName::new(&longest).is_some());
        let too_long = "a".repeat(ServiceName::MAX_LEN + 1);
        assert_eq!(ServiceName::new(&too_long), None);
    }

    #[test]
    fn a_name_is_reported_exactly_as_written() {
        let n = ServiceName::new("rivet-netd").expect("valid");
        assert_eq!(n.as_str(), "rivet-netd");
        assert_eq!(n.to_string(), "rivet-netd");
    }

    // ---- paths ------------------------------------------------------------

    /// Every path an installed application has, from one name. The program is
    /// **derived** — `ADR-015` §3.2's "no `exec` field" is a property of this
    /// function existing rather than a rule somebody remembers.
    #[test]
    fn an_applications_paths_all_come_from_its_name() {
        let name = ServiceName::new("photo-index").expect("a real Cargo package name");
        let v = BUILT_IN_VOLUME;
        assert_eq!(app_dir(v, &name), "/state/app/photo-index");
        assert_eq!(current_dir(v, &name), "/state/app/photo-index/current");
        assert_eq!(
            manifest_path(v, &name),
            "/state/app/photo-index/current/manifest.toml"
        );
        assert_eq!(
            program_path(v, &name),
            "/state/app/photo-index/current/bin/photo-index"
        );
        // And every one of them is under the application root, which is what
        // `ServiceName`'s refusal of a separator is protecting.
        for path in [
            app_dir(v, &name),
            current_dir(v, &name),
            manifest_path(v, &name),
            program_path(v, &name),
        ] {
            assert!(path.starts_with(APP_ROOT), "{path} escaped {APP_ROOT}");
        }
    }

    /// **The tripwire the two spellings of `/state/app` need.**
    ///
    /// `APP_ROOT` is a `&'static str` because three `const` tables cannot call
    /// a function; `app_root` is the general answer. `ADR-011` is why they
    /// cannot be allowed to drift, and this is the whole of what holds them
    /// together.
    #[test]
    fn the_constant_and_the_function_spell_the_built_in_volumes_app_root_the_same() {
        assert_eq!(APP_ROOT, app_root(BUILT_IN_VOLUME));
    }

    /// **The property `ADR-031` §8 item 1 promises**, asserted rather than
    /// claimed: nothing moved. A machine with no declared volume has the layout
    /// it had before this function existed, byte for byte.
    #[test]
    fn the_built_in_volume_puts_every_path_exactly_where_it_was() {
        let name = ServiceName::new("photo-index").expect("valid");
        let v = BUILT_IN_VOLUME;
        assert_eq!(app_dir(v, &name), "/state/app/photo-index");
        assert_eq!(data_dir(v, &name), "/state/app/photo-index/data");
    }

    /// A declared volume's paths are the same shape one level over.
    ///
    /// `/volumes/<name>` is `ADR-031` §11 item 8's *recommendation* and not a
    /// ruling, so this test names a root rather than deriving one — nothing in
    /// the tree may spell a mount point until the maintainer picks it.
    #[test]
    fn another_volumes_paths_are_the_same_shape_one_level_over() {
        let name = ServiceName::new("photo-index").expect("valid");
        assert_eq!(
            program_path("/somewhere/bulk", &name),
            "/somewhere/bulk/app/photo-index/current/bin/photo-index"
        );
        assert_eq!(
            data_dir("/somewhere/bulk", &name),
            "/somewhere/bulk/app/photo-index/data"
        );
    }

    /// **The tripwire.** The installed manifest and the artifact's manifest
    /// member are the same file, so they must have the same name — `ADR-011`,
    /// and a second spelling of one path is two answers to one question.
    #[test]
    fn the_installed_manifest_is_the_artifacts_manifest_member() {
        let name = ServiceName::new("x").expect("valid");
        assert!(
            manifest_path(BUILT_IN_VOLUME, &name)
                .ends_with(crate::package::application::MANIFEST_NAME),
            "the installed manifest and the packaged one have different names"
        );
    }

    // ---- who an application runs as ---------------------------------------

    /// **The property the whole scheme exists for.** A service that came back
    /// with a different uid after a reboot could not read its own files, and
    /// nothing would report it — the `EACCES` happens inside the application.
    #[test]
    fn one_name_has_one_identity_and_asking_twice_gives_the_same_answer() {
        let name = svc("photo-index");
        let first = app_identity(&name);
        // A second `ServiceName`, built from the same text by the same route a
        // later boot would build it: read off a directory entry.
        let second = app_identity(&svc("photo-index"));
        assert_eq!(first, second);
        assert_eq!(first.uid, first.gid, "an application's group is its own");
    }

    /// The range, from both ends, and over every name shape a `ServiceName`
    /// admits rather than over one example.
    #[test]
    fn every_identity_lands_inside_the_range_and_misses_root_nobody_and_minus_one() {
        let mut names = vec![
            "a".to_string(),
            "z9".to_string(),
            "photo-index".to_string(),
            "rivet.thing_1".to_string(),
            "9".to_string(),
        ];
        // A name at the length limit, since that is the input most likely to be
        // handled differently by accident.
        names.push("n".repeat(ServiceName::MAX_LEN));
        for i in 0..2000u32 {
            names.push(format!("app{i}"));
        }
        for text in &names {
            let id = app_identity(&svc(text));
            assert_eq!(id.uid, id.gid, "{text}");
            assert!(
                (APP_UID_BASE..=APP_UID_LAST).contains(&id.uid),
                "{text} produced uid {}",
                id.uid
            );
            assert_ne!(id.uid, 0, "{text} would have run as root");
            assert_ne!(id.uid, 65_534, "{text} took the conventional `nobody`");
            assert_ne!(
                id.uid, 65_535,
                "{text} took (uid_t)-1, which means `no change`"
            );
        }
    }

    /// The range misses what is already claimed. Stated as a test rather than
    /// only in prose, because `APP_UID_BASE` is the kind of constant somebody
    /// lowers to make a console line tidier.
    #[test]
    fn the_range_starts_above_root_the_system_range_and_the_human_range() {
        // Each of these is a uid something else already owns, or a range
        // something else conventionally owns. Enumerated rather than written as
        // three constant comparisons, because a comparison between two
        // constants is a lint and — more to the point — because listing what is
        // being avoided is the part a reader needs.
        for (claimed, whose) in [
            (0_u32, "root, and every file in the image"),
            (1, "the conventional system range"),
            (999, "the top of the conventional system range"),
            (
                1000,
                "where human accounts conventionally start — ADR-008 §7.3's deferred half",
            ),
            (3999, "the last uid left to the human half"),
            (65_534, "the conventional `nobody`"),
            (65_535, "(uid_t)-1, which setuid(2) reads as `no change`"),
        ] {
            assert!(
                !(APP_UID_BASE..=APP_UID_LAST).contains(&claimed),
                "an application could take uid {claimed}, which is {whose}"
            );
        }
        // And the one uid the record has ever written down for an application
        // — `ADR-015` §5.4's `uid 4021` — is inside it.
        assert!((APP_UID_BASE..=APP_UID_LAST).contains(&4021));
    }

    /// **A collision is real, not hypothetical**, and this is the test that
    /// says so. `app_identity` maps an unbounded set of names onto a finite
    /// range, so two names must collide somewhere; a scheme documented as
    /// having a collision case and unable to produce one would mean the
    /// refusal path in `cmd/rivet-sysd` had never been exercised
    /// against anything real (`AGENTS.md` §4).
    ///
    /// The pair is **found by search over the actual function**, not written
    /// down, so it cannot go stale against a change to the hash — it would
    /// simply find a different pair, and the day it finds none is the day this
    /// fails.
    #[test]
    fn two_names_can_claim_one_identity_and_the_predicate_says_so() {
        let mut seen: Vec<(u32, String)> = Vec::new();
        let mut found: Option<(String, String)> = None;
        'outer: for i in 0..20_000u32 {
            let text = format!("app{i}");
            let uid = app_identity(&svc(&text)).uid;
            for (other_uid, other) in &seen {
                if *other_uid == uid {
                    found = Some((other.clone(), text.clone()));
                    break 'outer;
                }
            }
            seen.push((uid, text));
        }
        let (a, b) = found.expect(
            "no two of twenty thousand names collided, which cannot happen over a range of \
             61534 — the hash or the range has changed shape",
        );
        assert!(identities_collide(&svc(&a), &svc(&b)), "{a} vs {b}");
        assert_eq!(app_identity(&svc(&a)), app_identity(&svc(&b)));
    }

    /// A name does not collide with itself. That is the same application
    /// discovered twice, which the filesystem cannot produce and which must not
    /// read as two applications fighting over one uid.
    #[test]
    fn a_name_does_not_collide_with_itself() {
        assert!(!identities_collide(
            &svc("photo-index"),
            &svc("photo-index")
        ));
        // And two names that genuinely differ usually do not collide, or the
        // predicate would be useless. One pair is enough to say the answer is
        // not a constant `true`.
        assert!(!identities_collide(&svc("photo-index"), &svc("metrics")));
    }

    /// The data directory is a **sibling** of the versions, not inside them.
    /// An update replaces `current`; if the data lived under it, every update
    /// would silently discard everything the application had written.
    #[test]
    fn an_applications_data_survives_the_version_it_was_written_by() {
        let name = svc("photo-index");
        assert_eq!(data_dir(BUILT_IN_VOLUME, &name), "/state/app/photo-index/data");
        assert!(data_dir(BUILT_IN_VOLUME, &name).starts_with(&app_dir(BUILT_IN_VOLUME, &name)));
        assert!(
            !data_dir(BUILT_IN_VOLUME, &name).starts_with(&current_dir(BUILT_IN_VOLUME, &name)),
            "an update would take the data with it"
        );
        assert_eq!(DATA_DIR_MODE, 0o700, "ADR-015 §3.2 says mode 700");
    }

    /// FNV-1a's published test vectors, so that "the constants are the
    /// published ones" is checked rather than asserted. Taken from the FNV
    /// reference (Fowler/Noll/Vo), not from this implementation's own output —
    /// `AGENTS.md` §4: a round trip against oneself proves nothing about a
    /// format, and this hash **is** a format (see [`fnv1a64`]).
    #[test]
    fn the_hash_agrees_with_fnv1as_published_vectors() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    // ---- addon classes — PLAN-Feature-Set-2026-09-12 §6.2, ADR-053 ---------

    /// `crate::package::library::tests`' own `hex` fixture, restated: a
    /// single repeated byte is all a reference's digest half needs to be.
    fn reference(name: &str, digest_byte: u8) -> crate::package::library::Reference {
        crate::package::library::Reference::parse(&format!(
            "{name}-1.0.0-{}",
            format!("{digest_byte:02x}").repeat(32)
        ))
        .expect("a valid fixture reference")
    }

    fn accepting(classes: &[&str]) -> Manifest {
        Manifest::new(crate::package::application::ProgramKind::Daemon)
            .with_accepts_classes(classes.iter().map(ToString::to_string).collect())
            .expect("valid")
    }

    fn installed(name: &str, digest_byte: u8, class: Option<&str>) -> InstalledLibrary {
        InstalledLibrary {
            reference: reference(name, digest_byte),
            class: class.map(str::to_string),
            soname: Some(format!("{name}.so.1")),
        }
    }

    /// **Two signed declarations agree, and a library joins.**
    #[test]
    fn a_matching_class_is_made_available() {
        let manifest = accepting(&["temperature-sensor"]);
        let libs = vec![installed("libtemp", 0x11, Some("temperature-sensor"))];
        assert_eq!(
            addon_library_path(BUILT_IN_VOLUME, &manifest, &libs),
            Some(crate::package::install::library_object_dir(
                BUILT_IN_VOLUME,
                &reference("libtemp", 0x11)
            ))
        );
    }

    /// A different string, however close, does not join — this is exact
    /// equality, not a prefix or a fuzzy match.
    #[test]
    fn a_mismatched_class_is_not_joined() {
        let manifest = accepting(&["temperature-sensor"]);
        let libs = vec![installed("libtemp", 0x11, Some("temperature-sensors"))];
        assert_eq!(addon_library_path(BUILT_IN_VOLUME, &manifest, &libs), None);
    }

    /// A library that names no class is invisible to this mechanism, even
    /// when an application happens to accept a class with the same name as
    /// the library's product.
    #[test]
    fn a_library_with_no_class_is_not_joined() {
        let manifest = accepting(&["temperature-sensor"]);
        let libs = vec![installed("libtemp", 0x11, None)];
        assert_eq!(addon_library_path(BUILT_IN_VOLUME, &manifest, &libs), None);
    }

    /// An application that accepts no class is unchanged by whatever is
    /// installed, including a library that would otherwise match everything.
    #[test]
    fn an_application_accepting_no_class_is_unchanged() {
        let manifest = Manifest::new(crate::package::application::ProgramKind::Daemon);
        assert!(manifest.accepts_classes().is_empty());
        let libs = vec![installed("libtemp", 0x11, Some("temperature-sensor"))];
        assert_eq!(addon_library_path(BUILT_IN_VOLUME, &manifest, &libs), None);
    }

    /// An empty list, given explicitly through the builder rather than left
    /// at the default, joins nothing either — `.rulings/` R-162 R28's own
    /// acceptance test, not just the constructor default above.
    #[test]
    fn an_explicitly_empty_class_list_joins_nothing() {
        let manifest = accepting(&[]);
        let libs = vec![installed("libtemp", 0x11, Some("temperature-sensor"))];
        assert_eq!(addon_library_path(BUILT_IN_VOLUME, &manifest, &libs), None);
    }

    /// **R-162 R28: an application accepting two classes joins a library of
    /// each.**
    #[test]
    fn an_application_accepting_two_classes_joins_a_library_of_each() {
        let manifest = accepting(&["temperature-sensor", "humidity-sensor"]);
        let libs = vec![
            installed("libtemp", 0x11, Some("temperature-sensor")),
            installed("libhumid", 0x22, Some("humidity-sensor")),
        ];
        let joined = addon_library_path(BUILT_IN_VOLUME, &manifest, &libs)
            .expect("both installed libraries match one of the two accepted classes");
        assert!(joined.contains(
            crate::package::install::library_object_dir(BUILT_IN_VOLUME, &reference("libtemp", 0x11))
                .as_str()
        ));
        assert!(joined.contains(
            crate::package::install::library_object_dir(BUILT_IN_VOLUME, &reference("libhumid", 0x22))
                .as_str()
        ));
    }

    /// **R-162 R28: a library whose class is in neither accepted class is not
    /// joined**, even though other installed libraries do match.
    #[test]
    fn a_library_matching_neither_of_two_accepted_classes_is_not_joined() {
        let manifest = accepting(&["temperature-sensor", "humidity-sensor"]);
        let libs = vec![
            installed("libtemp", 0x11, Some("temperature-sensor")),
            installed("libpressure", 0x33, Some("pressure-sensor")),
        ];
        let joined = addon_library_path(BUILT_IN_VOLUME, &manifest, &libs)
            .expect("the matching library still joins");
        assert!(joined.contains(
            crate::package::install::library_object_dir(BUILT_IN_VOLUME, &reference("libtemp", 0x11))
                .as_str()
        ));
        assert!(!joined.contains("libpressure"), "{joined}");
    }

    /// **R-162 R28: duplicates in the accepted list don't double-join.** One
    /// installed library of the repeated class must appear exactly once in
    /// the joined path, not twice.
    #[test]
    fn duplicate_classes_in_the_accepted_list_do_not_double_join_a_library() {
        let manifest = accepting(&["temperature-sensor", "temperature-sensor"]);
        let libs = vec![installed("libtemp", 0x11, Some("temperature-sensor"))];
        let joined = addon_library_path(BUILT_IN_VOLUME, &manifest, &libs)
            .expect("the one installed library still matches");
        let entry = crate::package::install::library_object_dir(
            BUILT_IN_VOLUME,
            &reference("libtemp", 0x11),
        );
        assert_eq!(joined, entry, "the entry must appear exactly once, not twice");
    }

    /// Two matching libraries join in a stable order regardless of the order
    /// a directory listing happened to return them in.
    #[test]
    fn two_matches_join_sorted_by_reference_regardless_of_input_order() {
        let manifest = accepting(&["temperature-sensor"]);
        let a = installed("libtemp-a", 0x11, Some("temperature-sensor"));
        let b = installed("libtemp-b", 0x22, Some("temperature-sensor"));

        let forward = addon_library_path(BUILT_IN_VOLUME, &manifest, &[a.clone(), b.clone()]);
        let backward = addon_library_path(BUILT_IN_VOLUME, &manifest, &[b, a]);
        assert_eq!(forward, backward, "input order must not affect the result");
        assert!(
            forward.expect("both libraries match").contains("libtemp-a")
        );
    }

    // -----------------------------------------------------------------------
    // The enumeration channel — `PLAN-Addon-Feature-Set-2026-09-19` T2.
    //
    // These are the no-I/O half of the proof. The other half is two real
    // libraries of one class installed on a board, because a rule that could
    // not express two libraries is not disproved by a test that only ever
    // builds one.
    // -----------------------------------------------------------------------

    fn object(name: &str, digest_byte: u8) -> String {
        format!(
            "{}/{name}.so.1",
            crate::package::install::library_object_dir(
                BUILT_IN_VOLUME,
                &reference(name, digest_byte)
            )
        )
    }

    /// **The whole shape of the variable**, on the ordinary one-addon case.
    #[test]
    fn a_joined_library_is_named_by_its_class_and_the_path_of_its_object() {
        let manifest = accepting(&["temperature-sensor"]);
        let libs = vec![installed("libtemp", 0x11, Some("temperature-sensor"))];
        assert_eq!(
            addon_env(BUILT_IN_VOLUME, &manifest, &libs),
            Some(format!("temperature-sensor={}", object("libtemp", 0x11)))
        );
    }

    /// **The case the naming rule could not express**, and the reason this
    /// channel exists at all — `.rulings/` R-199 D1. Two libraries of one
    /// class, both named, each by its own object.
    #[test]
    fn two_libraries_of_one_class_are_both_named() {
        let manifest = accepting(&["aura-background-pack"]);
        let libs = vec![
            installed("packone", 0x11, Some("aura-background-pack")),
            installed("packtwo", 0x22, Some("aura-background-pack")),
        ];
        let value = addon_env(BUILT_IN_VOLUME, &manifest, &libs).expect("both join");
        let entries: Vec<&str> = value.split(':').collect();
        assert_eq!(
            entries,
            vec![
                format!("aura-background-pack={}", object("packone", 0x11)),
                format!("aura-background-pack={}", object("packtwo", 0x22)),
            ],
            "two libraries of one class are two entries, not one collision"
        );
    }

    /// Nothing joined, nothing said — [`ADDONS_ENV`]'s own doc has why an
    /// empty string would be worse than no variable.
    #[test]
    fn nothing_joined_means_no_variable_at_all() {
        let manifest = accepting(&["temperature-sensor"]);
        let libs = vec![installed("libother", 0x11, Some("humidity-sensor"))];
        assert_eq!(addon_env(BUILT_IN_VOLUME, &manifest, &libs), None);
        assert_eq!(
            addon_env(BUILT_IN_VOLUME, &accepting(&[]), &libs),
            None,
            "an application accepting no class is told nothing"
        );
    }

    /// A library with no object cannot be named, and is left out rather than
    /// named emptily — while its directory still reaches the loader and the
    /// ruleset, because those two are about a directory and this is about a
    /// file.
    #[test]
    fn a_library_with_no_object_is_left_out_of_the_naming_but_not_of_the_path() {
        let manifest = accepting(&["temperature-sensor"]);
        let mut lib = installed("libtemp", 0x11, Some("temperature-sensor"));
        lib.soname = None;
        let libs = vec![lib];
        assert_eq!(addon_env(BUILT_IN_VOLUME, &manifest, &libs), None);
        assert_eq!(
            addon_library_dirs(BUILT_IN_VOLUME, &manifest, &libs).len(),
            1,
            "the directory still joins: nothing about the path depends on the object's name"
        );
    }

    /// **The three composers agree, which is the reason they share a
    /// function.** Every named object must sit in a directory that is on the
    /// search path, and there must be exactly as many directories as there are
    /// matched libraries.
    #[test]
    fn every_named_object_sits_in_a_directory_the_loader_may_search() {
        let manifest = accepting(&["aura-background-pack", "temperature-sensor"]);
        let libs = vec![
            installed("packone", 0x11, Some("aura-background-pack")),
            installed("libtemp", 0x22, Some("temperature-sensor")),
            installed("libnope", 0x33, Some("pressure-sensor")),
        ];
        let dirs = addon_library_dirs(BUILT_IN_VOLUME, &manifest, &libs);
        let named = addon_env(BUILT_IN_VOLUME, &manifest, &libs).expect("two join");
        assert_eq!(dirs.len(), 2, "the unmatched library joins neither");
        for entry in named.split(':') {
            let (_, path) = entry.split_once('=').expect("every entry is class=path");
            assert!(
                dirs.iter().any(|dir| path.starts_with(&format!("{dir}/"))),
                "{path} is named but its directory is not on the path: {dirs:?}"
            );
        }
    }

    /// Input order must not move an entry, for the reason
    /// `addon_library_path` sorts: a variable that depended on `readdir` would
    /// be a machine whose hosts saw their addons in a different order between
    /// two boots of the same state.
    #[test]
    fn the_naming_is_sorted_by_reference_regardless_of_input_order() {
        let manifest = accepting(&["aura-background-pack"]);
        let a = installed("packone", 0x11, Some("aura-background-pack"));
        let b = installed("packtwo", 0x22, Some("aura-background-pack"));
        assert_eq!(
            addon_env(BUILT_IN_VOLUME, &manifest, &[a.clone(), b.clone()]),
            addon_env(BUILT_IN_VOLUME, &manifest, &[b, a])
        );
    }

    /// An entry that would break the reader is dropped rather than handed
    /// over — [`ADDONS_ENV`]'s *no token may contain the separator*, proven
    /// rather than asserted in prose.
    #[test]
    fn an_entry_containing_the_separator_is_dropped_rather_than_handed_over() {
        let manifest = accepting(&["temperature-sensor"]);
        let mut lib = installed("libtemp", 0x11, Some("temperature-sensor"));
        lib.soname = Some("lib:temp.so.1".to_string());
        assert_eq!(addon_env(BUILT_IN_VOLUME, &manifest, &[lib]), None);
    }

    /// **A release is compared by its numbers, and a describe suffix is not a
    /// newer release** — the comparison provisioning's `ADR-060` refusal uses.
    #[test]
    fn a_release_is_compared_by_its_three_numbers() {
        assert_eq!(release_at_least("0.1.114", PROXY_IN_IMAGE_SINCE), Some(true));
        assert_eq!(release_at_least("0.1.115", "0.1.114"), Some(true));
        assert_eq!(release_at_least("0.2.0", "0.1.114"), Some(true));
        assert_eq!(release_at_least("v0.1.114", "0.1.114"), Some(true));
        assert_eq!(release_at_least("0.1.114-3-gabc1234", "0.1.114"), Some(true));
        assert_eq!(release_at_least("0.1.113", "0.1.114"), Some(false));
        assert_eq!(
            release_at_least("0.1.113-9-gabc1234-dirty", "0.1.114"),
            Some(false),
            "a build after 0.1.113 is still 0.1.113 until it is released as more"
        );
        assert_eq!(release_at_least("0.1.99", "0.1.114"), Some(false), "not a string compare");
        assert_eq!(release_at_least("unknown", "0.1.114"), None);
        assert_eq!(release_at_least("0.1", "0.1.114"), None);
    }

    /// Every image service with a home is an image service, so an application
    /// under its name is refused at the install door and never carried.
    #[test]
    fn every_image_service_with_a_home_is_an_image_service() {
        for home in IMAGE_SERVICE_HOMES {
            assert!(IMAGE_SERVICE_NAMES.contains(&home.name), "{}", home.name);
            assert!(home.program.starts_with("/system/bin/"), "{}", home.program);
            assert!(ServiceName::new(home.name).is_some(), "{}", home.name);
        }
    }

    // ---- D4: the default memory limit ---------------------------------------

    /// **Half the machine**, read off a Pi 4's real `/proc/meminfo` line.
    #[test]
    fn an_application_asking_for_no_limit_gets_half_the_machine() {
        let total = meminfo_total_bytes("MemTotal:         922976 kB\nMemFree:  1 kB\n")
            .expect("a MemTotal line");
        assert_eq!(total, 922_976 * 1024);
        let effective =
            effective_application_memory(None, default_application_memory(total), None);
        assert_eq!(effective.limit.map(MemoryLimit::bytes), Some(922_976 * 512));
        assert_eq!(effective.source, MemorySource::Default);
    }

    /// The manifest's own request is untouched by the default, whichever way.
    #[test]
    fn a_manifest_that_asks_keeps_what_it_asked_for() {
        let default = default_application_memory(1024 * 1024 * 1024);
        for asked in [limit("64M"), limit("2G")] {
            let effective = effective_application_memory(Some(asked), default, None);
            assert_eq!(effective.limit, Some(asked));
            assert_eq!(effective.source, MemorySource::Manifest);
        }
    }

    /// **The operator's cap replaces the default in either direction**: below
    /// it and above it, the document's number is the one that applies, and it
    /// is reported as the operator's.
    #[test]
    fn the_operators_cap_replaces_the_default_either_way() {
        let default = default_application_memory(1024 * 1024 * 1024);
        let lower = effective_application_memory(None, default, Some(limit("64M")));
        assert_eq!(lower.limit, Some(limit("64M")));
        assert_eq!(lower.source, MemorySource::Document);
        let higher = effective_application_memory(None, default, Some(limit("900M")));
        assert_eq!(higher.limit, Some(limit("900M")));
        assert_eq!(higher.source, MemorySource::Document);
    }

    /// **A manifest's own request still meets the cap by the lower-wins rule**;
    /// the default plays no part once the manifest has spoken.
    #[test]
    fn a_manifest_request_still_meets_the_cap_lower_wins() {
        let default = default_application_memory(1024 * 1024 * 1024);
        let raised = effective_application_memory(Some(limit("128M")), default, Some(limit("900M")));
        assert_eq!(raised.limit, Some(limit("128M")));
        assert_eq!(raised.source, MemorySource::ManifestLower);
        let lowered = effective_application_memory(Some(limit("128M")), default, Some(limit("64M")));
        assert_eq!(lowered.limit, Some(limit("64M")));
        assert_eq!(lowered.source, MemorySource::DocumentLower);
    }

    /// A machine whose memory could not be read applies no default, and says
    /// nothing it does not know.
    #[test]
    fn no_known_machine_memory_means_no_default() {
        assert_eq!(meminfo_total_bytes("MemFree: 1 kB\n"), None);
        assert_eq!(default_application_memory(1), None);
        assert_eq!(
            effective_application_memory(None, None, None),
            EffectiveMemory::UNSTATED
        );
    }

    /// **A removed thing's name is never a live one.** An application, a
    /// library and a repository each refuse a name carrying the mark, so the
    /// rename that removes one can never land on, or be mistaken for, another.
    #[test]
    fn no_live_name_can_be_a_set_aside_name() {
        let aside = format!("{RECLAIM_MARK}1700-3");
        assert!(is_set_aside(&aside));
        assert!(ServiceName::new(&aside).is_none(), "an application could be named {aside}");
        assert!(
            crate::repo::RepoName::new(&aside).is_none(),
            "a repository could be named {aside}"
        );
        assert!(
            crate::package::library::Reference::parse(&aside).is_err(),
            "a library could be named {aside}"
        );
        for live in ["a", "0.1.0", "data", "current", "tmp", "tmp.reclaim-1-1", "x.reclaim-1"] {
            assert!(!is_set_aside(live), "{live} read as set aside");
        }
    }
}
