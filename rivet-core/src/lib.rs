//! Rivet's operations, as types.
//!
//! **This crate performs no I/O.** `ADR-014` §3 states the rule so it has
//! teeth: the forbidden edges are absent from `Cargo.toml`, so they fail to
//! compile rather than failing review. The standing check is `ADR-014` §3.1 —
//! `cargo test -p rivet-core` must need no network, no filesystem beyond
//! temporary paths, no device and no root. If it starts needing one, the
//! boundary has leaked.
//!
//! What lives here is *what should be true*. What performs it lives in an
//! adapter, and at Stage 0 the only caller is `cmd/rivet-initd`.

#![forbid(unsafe_code)]

pub mod account;
/// **The machine's artifact store** — `ADR-052` §7. A digest-keyed object
/// store under `/state`, and the only place an install can name.
pub mod artifact;
// **One format, one implementation, both directions** — created 2026-09-01.
// The encoder and the decoder were written in modules that never mention each
// other, with a third copy of the alphabet in a test helper. `ADR-011`: one
// fact, one owner. See the module for what the merge had to carry across and
// must not have loosened.
/// **What the applications on this machine offer to back up** — `ADR-051`.
/// Derived from signed manifests, published root-owned, and validated at the
/// FORMAT rather than the schema.
pub mod backup;
pub mod base64;
pub mod bluetooth;
// **The applications a card carries** — `ADR-050` §4, 2026-09-07. The names
// `cmd/rivet-image`, `cmd/rivet-initd` and `cmd/rivet-sysd` have to agree on
// for a payload to travel from a boot partition to an install. `ADR-011`: two
// spellings of `app-<product>.rvt` would fail as *nothing happened*.
pub mod carried;
// **What an application may reach, derived from what it declared** — `ADR-042`,
// 2026-09-04. Pure: a policy in Rivet's own verbs, with the Landlock mapping in
// `cmd/rivet-sysd` where a platform type is allowed to be named.
pub mod confinement;
// **A LAN-facing listener's connection slots, and one source's share of them**
// — the rule every listener applies (`ADR-035` §12). The module's own
// documentation says the rest, where its links resolve.
pub mod connection_slots;
// **Where a program goes to be given a surface** — `ADR-058` §4, 2026-09-20.
// Three strings and no protocol: the conversation on that socket is Wayland's,
// and `rivet_core::surface` was removed the same day for proposing a second
// one. Here because both ends must agree — `ADR-011`.
pub mod compositor;
pub mod control;
// **What a fleet server does with one machine's document** — `ADR-044`,
// `.rulings/` R-145. Pure: three sets of leaves in, a verdict out. It is here
// rather than in `cmd/rivet-fleetd` because a rule that lives in a `main.rs`
// is a rule only ever exercised against a board.
pub mod converge;
// **Whether a request that changes something was caused by another site** —
// `.rulings/` R-186 D1. Pure header logic, asked by `rivet-proxyd` for every
// application and by `rivet-fleetd` for its own endpoints; one copy of a
// security rule rather than two that drift.
pub mod cross_site;
pub mod deploy;
pub mod display;
pub mod etc;
// **Fleet-as-code's git operations on a fleet server's control plane** —
// `ADR-044`, `ADR-035`. The routes, formats, path rules and refusals both
// `rivet-mcpd`, which runs git, and `rivet-fleetd`, which asks, read.
pub mod fleet_as_code;
// **A fleet's own root, minted on its fleet server** — `ADR-063` §4. The
// request a card carries, where the root rests, and who is handed the leaf.
pub mod fleet_root;
// **What a machine tells an application about itself** — `ADR-057`,
// `.rulings/` R-199 D4. The fact set and its wire form, both halves.
//
// A comment and not a doc comment: a `///` here is MERGED with the module's own
// header and resolved in THIS file's scope, which silently breaks every
// intra-doc link the module writes about its own types.
pub mod facts;
pub mod generated;
pub mod git;
pub mod health;
pub mod hostname;
pub mod identifier;
pub mod journal;
pub mod input;
// **Two desired-state documents, compared leaf by leaf.** Moved out of
// `cmd/rivet-fleet/src/diff.rs` when `ADR-044` gave it a second caller; the
// TOML reader that fills it stays in `rivet-config`, because this crate
// depends on nothing that touches the world.
pub mod leaf;
pub mod kiosk;
pub mod json;
pub mod mount;
pub mod net;
pub mod operation;
pub mod package;
pub mod partlabel;
// The passkey enrollment code: its file, its lifetime and its spelling, which
// `apply passkey-code` writes and `rivet-proxyd` reads (`ADR-063` §4).
pub mod passkey;
pub mod repo;
pub mod repo_delete;
// **A fleet server's repository objects** — `.rulings/` R-184 D1 and D2. The
// routes, the listing and the prune rule both ends of the control plane read,
// for a store kept in `rivet-fleetd`'s own data directory.
pub mod repository_objects;
pub mod secret;
pub mod service;
pub mod name_service;
pub mod mdns;
pub mod proxy;
pub mod settings;
pub mod shutdown;
pub mod shellwords;
pub mod ssid;
pub mod staging;
pub mod state;
pub mod timezone;
pub mod uninstall;
// **Removing an installed library on purpose**: `apply uninstall-library`'s
// request channel, `crate::uninstall`'s twin, and the decision both its halves
// make.
pub mod uninstall_library;
pub mod value;

pub use operation::{ErrorCode, Operation, OperationError, Verb};
pub use value::Value;

/// Every way an operation in this crate can fail.
///
/// Deliberately closed, per `ADR-006` §6.4's closed-vocabulary rule: this is
/// exactly the kind of enumeration that grows one convenient value at a time
/// until it means nothing.
///
/// **Four variants were added 2026-08-23** with the wired bring-up plan
/// ([`net`]). The closed-vocabulary rule is a bar to clear, not a freeze, and
/// the bar it sets is that each variant name a *distinct thing an operator got
/// wrong* rather than a place in the code where a check happened. Each one
/// below carries the offending value, because `AGENTS.md` §5 — report what the
/// system said — applies just as much to what the *operator* said.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CoreError {
    /// A required mount point is not an absolute path.
    NotAbsolute { path: &'static str },
    /// A network interface name the kernel would not accept.
    InvalidInterfaceName { name: String, why: net::NameFault },
    /// A wireless network name a radio could not carry — `crate::ssid`.
    ///
    /// **Carries the LENGTH and not the name.** Every other variant here quotes
    /// its input, and this one must not: the document an SSID is read from is
    /// the one `ADR-020` §4 is about, and a refusal that echoed a line of it is
    /// one edit away from echoing the wrong line.
    InvalidSsid {
        length: usize,
        detail: &'static str,
    },
    /// Text that was meant to be `A.B.C.D/N` and is not.
    MalformedAddress { text: String },
    /// A prefix length above 32. Separate from [`Self::MalformedAddress`]
    /// because the text parsed fine and the *number* is the mistake, and an
    /// operator who typed `/42` needs to be told that rather than that their
    /// address is unreadable.
    PrefixTooLong { prefix_len: u8 },
    /// A default route's next hop that is not on the interface's own subnet.
    GatewayOffLink {
        gateway: std::net::Ipv4Addr,
        subnet: net::Ipv4Cidr,
    },
    /// A subnet's network or broadcast address offered as a host address.
    NotAHostAddress {
        address: net::Ipv4Cidr,
        role: net::AddressRole,
    },
}

impl core::fmt::Display for CoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotAbsolute { path } => {
                write!(f, "mount target is not an absolute path: {path}")
            }
            Self::InvalidSsid { length, detail } => {
                write!(f, "a wireless network name of {length} bytes is not usable: {detail}")
            }
            Self::InvalidInterfaceName { name, why } => {
                write!(f, "{name:?} is not a usable interface name: {why}")
            }
            Self::MalformedAddress { text } => {
                write!(
                    f,
                    "{text:?} is not an address and prefix length (A.B.C.D/N)"
                )
            }
            Self::PrefixTooLong { prefix_len } => {
                write!(
                    f,
                    "prefix length /{prefix_len} is longer than an IPv4 address"
                )
            }
            Self::GatewayOffLink { gateway, subnet } => write!(
                f,
                "gateway {gateway} is not on {subnet} — a default route cannot reach a next hop that is not on-link"
            ),
            Self::NotAHostAddress { address, role } => {
                write!(f, "{address} is {role}, not a host address")
            }
        }
    }
}

impl std::error::Error for CoreError {}
