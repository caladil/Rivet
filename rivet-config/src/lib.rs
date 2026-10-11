//! Rivet's configuration schema: the types, and eventually their generated
//! schema. `ADR-007` is the model — a typed store, TOML as a *representation*
//! rather than the authority, and a declared schema that decides what a valid
//! document is.
//!
//! Like `rivet-core`, this crate performs no I/O. **Parsing a string is not
//! I/O; opening the file it came from is**, and that belongs to an adapter at
//! the composition root. `ADR-014` §3.1 is the standing check:
//! `cargo test -p rivet-config` needs no network, no filesystem beyond
//! temporary paths, no device and no root — every fixture in this crate is a
//! string literal.
//!
//! ```
//! use rivet_config::{Document, SCHEMA_VERSION};
//!
//! let doc = Document::parse("version = 1\n")?;
//! assert_eq!(doc.version(), SCHEMA_VERSION);
//!
//! // Nothing is defaulted around. A misspelling is a refusal that names it.
//! let err = Document::parse("verison = 1\n").unwrap_err();
//! assert!(err.to_string().contains("verison"));
//!
//! // The operator says whether a service runs, and how much memory it may
//! // have. What the service *is* is not theirs to write — `ADR-017` §4.1 — and
//! // since 2026-08-24 that includes which sockets it is served on.
//! let doc = Document::parse(
//!     "version = 1\n\n[services.rivet-netd]\nenabled = true\nmemory = \"64M\"\n",
//! )?;
//! let netd = doc.service("rivet-netd").expect("configured above");
//! assert!(netd.enabled());
//! assert_eq!(netd.memory().map(|m| m.to_string()), Some("64M".to_owned()));
//!
//! // A table this schema does not declare is a refusal that names it, rather
//! // than a key that is read and then discarded (`ADR-007` §3.6).
//! let err = Document::parse("version = 1\n[nonesuch]\nkey = []\n").unwrap_err();
//! assert!(err.to_string().contains("nonesuch"));
//! # Ok::<(), rivet_config::ConfigError>(())
//! ```
//!
//! # What Stage 0 commits to, and what it does not
//!
//! `ADR-007` §5 puts exactly three things at Stage 0, and all three are here:
//! configuration is typed and the Rust types are the authority; there is one
//! document and no second on-disk form of any setting; observed state is never
//! written into it. Layering with per-value provenance is Stage 2–3 and
//! `plan`/`apply`/`export` is Stage 3 — neither is built here, and `ADR-007`
//! §5 notes that layers can be added later *provided values are typed*, which
//! is the property this crate exists to establish.
//!
//! # A gap in the record, reported rather than filled
//!
//! `PLAN.md` §2.1 asks that PID 1 "reads the single configuration document",
//! and `ADR-006` §8 item 5 asks that there be one document with a schema "from
//! the first line of configuration read". **Neither says what a Stage 0
//! document contains.** Searching the record for the provisioning file's
//! contents finds only later-stage material: wireless credentials, a fleet
//! endpoint and a device identity (`ADR-019` §3, `ADR-004` §6), and a hostname
//! offered as an example of a machine-layer value (`ADR-019` §3.2). Every one
//! of those is Stage 2 or later, and `ADR-007` §4.3 says plainly that Stage 0
//! configures almost nothing.
//!
//! So the schema here declares exactly one key — the schema version — and that
//! one is *derived* rather than quoted: `ADR-007` §3.2 requires that an older
//! document be "migrated explicitly rather than silently misread", which is
//! impossible unless the document declares which version it is. See
//! [`SchemaVersion`]. The field's name, and whether Stage 0 should carry
//! anything else, are the maintainer's to rule on.
//!
//! **Amended 2026-08-22: the gap closed for services and only for services.**
//! `ADR-017` §4.1 was ruled that day and its consequences list names this
//! crate's next key in as many words — *"`ADR-007`'s schema gains a services
//! table carrying enable/disable and per-service settings — the operator's
//! whole surface."* So the document now declares two keys, `version` and
//! `services`, and the second is quoted from the record rather than derived.
//!
//! The interesting part of that table is what it refuses. `ADR-017` §4.1 split
//! two authorities that had been conflated: **what a service *is*** — its
//! program, its restart policy, how its readiness is known — ships inside the
//! signed artifact that provides it, and **what the operator wants of it** is
//! this document. An operator who could write the first would be able to
//! redefine what a signed application executes, which is exactly the guarantee
//! `ADR-015` §4.1 exists to provide. [`services`] states the rule that draws
//! that line, applies it key by key, and says why each excluded key is
//! excluded.
//!
//! **Amended 2026-08-24: the line moved by one key, and the conflict it
//! resolves was real.** `ADR-007` §7.2 and `ADR-015` §3.2 both declared `listen`
//! and `memory`; both ADRs were accepted and `ADR-011` says both cannot be the
//! authority. The maintainer ruled it by direction, 2026-08-24
//! (`ADR-007` §7.2.1): **the manifest owns what the application is — restart
//! policy, readiness contract, sockets — and the document owns what the
//! operator limits, the memory cap. Where both name `memory`, the lower wins.**
//! So this crate's `[services]` table is `enabled` and `memory`, `listen` is
//! refused with the ruling and its date, and the lower-wins rule itself is
//! `rivet_core::service::effective_memory` rather than anything here — see
//! [`services`], which argues why this crate does not merge.
//!
//! **Amended 2026-08-23: the third key is `network`.** The maintainer ruled
//! option 2 — a `[network]` table in the document, plus
//! first-boot provisioning through the file `ADR-004` §6 already reads from the
//! boot partition. Until that ruling `cmd/rivet-netd` took `--address` and
//! `--gateway` on `argv`, which made a service's argument list a second
//! authority for what this machine's address is; `ADR-011` forbids that, and
//! the same ruling had been made the same day over `/etc`'s generators.
//!
//! **DHCP is a value of the table, not a later feature.** The ruling added that
//! many devices will still be addressed by DHCP (`.rulings/` R-267), which folds
//! the rejected third option in as a *value* rather than leaving it a
//! competing design. ~~A document may ask for it today and will parse and validate; it
//! then fails at apply, because `ADR-004` §8's `udhcpc` is not integrated.~~
//! **`udhcpc` was integrated on 2026-08-30 and a document asking for `dhcp`
//! now takes a lease** — struck rather than deleted, because the split between
//! validating here and failing there was deliberate and its ending is the
//! point: the schema did not have to change when the capability arrived, which
//! is exactly what `ADR-007` §3.2 versions the schema to avoid. See
//! [`network`] for the surface and the argument for it against `ADR-007`
//! §3.6.5.
//!
//! # The store this schema writes into
//!
//! `ADR-014` §2 lists `rivet-store` — persistence of desired state to
//! `/state/config` — and it exists: `ADR-004` §6 step 2 has an implementation,
//! where PID 1 seeds `/state/config/machine.toml` from the provisioning
//! document when nothing is stored there yet (`ADR-017` §3.1.8).
//!
//! **What this crate gained for it** is [`Document::to_toml`] and
//! [`crate::serialise`], the other direction of the parser: `ADR-007` §3.1 requires a
//! canonical text form, and the crate that owns the schema owns both directions
//! of its text form. It is still not I/O — see [`crate::serialise`] for why the
//! renderer is not in the persistence adapter.
//!
//! **What is still not built, said plainly.** `cmd/rivet-netd` is told a
//! document path on its command line by whoever starts it, and nothing yet
//! points it at the stored document; `ADR-007` §3.4's image-defaults and fleet
//! layers have no store and nothing merges layers. Neither is stubbed
//! (`AGENTS.md` §3).

#![forbid(unsafe_code)]

mod document;
mod error;
pub mod applications;
pub mod carry;
// **Fleet convergence exercised end to end.** A document's text, through
// this crate's own reader, into `rivet_core::converge::decide`. Kept apart
// from `leaves`'s own tests: those are about what a document's text becomes;
// these are about what a fleet server then does with it.
#[cfg(test)]
mod converge_tests;
pub mod control_plane;
pub mod display;
// `[dns]` — the name service's table, `ADR-047` §8.
pub mod dns;
pub mod fleet_role;
// A generated interview over `schema::DOCUMENT` — plan §5.2 item 3
// (`.rulings/` R-162, Wave 3b). `cmd/rivet-provision` and `cargo rivet
// provision`'s orchestrator both generate from this rather than each
// carrying their own list.
pub mod interview;
pub mod machine_mode;
pub mod network;
pub mod kiosk;
// **A document's text, as `rivet_core::leaf`'s leaves** — the reader half of
// the comparison two programs now make. `ADR-034` §6.2 and `ADR-044` §5.
pub mod leaves;
pub mod splash;
mod parse;
pub mod repos;
pub mod resolver;
pub mod schema;
pub mod serialise;
pub mod services;
pub mod volumes;

pub use document::{Document, SCHEMA_VERSION, SchemaVersion};
pub use error::{ConfigError, Location};
pub use fleet_role::FleetRole;
pub use machine_mode::MachineMode;
pub use network::{Addressing, Interface, Method};
pub use schema::schema;
pub use serialise::CANONICAL_HEADER;
pub use services::{MemoryLimit, Service, ServiceName};
