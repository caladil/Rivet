//! **Reaching a running Rivet from a host.**
//!
//! Extracted from `cmd/cargo-rivet` on 2026-08-26, and the reason is a ruling
//! rather than tidiness. `ADR-034` §11.5:
//!
//! > `cmd/cargo-rivet` is **binary-only** — no `[lib]` target — and its
//! > push-verify-install logic is a module inside the binary crate. `ADR-014`
//! > makes `cmd/*` composition roots, so the logic does not belong in a `cmd/`
//! > crate at all. **It is extracted into a library crate that both
//! > `cargo-rivet` and `rivet-fleet` depend on**, which is `ADR-011` applied:
//! > one implementation of "put a payload on a machine", not two.
//!
//! The maintainer's view of the alternative, recorded in the same section
//! (`.rulings/` R-330): code that is reachable as a crate is not shelled out to,
//! and doing so would be silly.
//!
//! # What is here and what deliberately is not
//!
//! Here: the [`deploy::Remote`] port and its one implementation, the
//! push-check-install sequence, the `enabled` edit, and `ADR-027` §7's artifact
//! naming in both directions.
//!
//! **Not here: any command line.** `cargo rivet deploy` parses argv by hand and
//! `rivet-fleet` uses `clap` (`ADR-034` §11.1) — two parsers over one
//! vocabulary, which is that section's ruling. `cargo rivet deploy` builds a
//! [`deploy::DeployArgs`] and hands it to [`deploy::run`]; `rivet-fleet` calls
//! the [`deploy::Remote`] port's document methods and never runs a deploy,
//! because one deployment surface is the ruling (`PLAN-Feature-Set-2026-09-12`
//! §3).
//!
//! **Not here: a signing key.** `cargo rivet` holds one and this crate never
//! sees it; `ADR-015` §4's first bullet is that the private key never leaves
//! the build machine, and a library two tools link is the wrong place for it.
//! What this crate handles is a payload that has *already* been signed.
//!
//! **Not here: a second transport.** `ADR-034` §11.5.1 rules that every remote
//! human tool is a control-plane client — *there is nothing left at the far
//! end* of anything else — so [`deploy::Remote`] has exactly one
//! implementation, and a dead one kept beside it is a thing an agent reaches
//! for because it is there.
//!
//! # This is a host crate, and the austerity argument does not reach it
//!
//! `ADR-001`'s dependency austerity is about **what reaches a device**.
//! Nothing here does. That is the same distinction `ADR-034` §11.2 makes the
//! `README.md` crate count carry a denominator for: a workspace number and an
//! image number are different numbers, and this crate moves only the first.

pub mod deploy;
pub mod name;

/// **Reaching a machine over `ADR-035`'s control plane** — mutually
/// authenticated, pinned TLS, and the transport a deploy runs on.
pub mod tls;

/// **One catalogue operation, in the machine's own words, over that control
/// plane.** `cargo rivet run`'s translation, and the answer to the one thing
/// `deploy` could not do: ask a board a question that is not part of a deploy.
pub mod command;

/// **`--host`, `--identity` and `--pin`** — the three flags that name a machine
/// and the trust in it, in one spelling for every tool that reaches one.
///
/// It was `cmd/cargo-rivet`'s until 2026-08-29, shared by `deploy` and `run`.
/// `ADR-034` §11.5.1 made `rivet-fleet` a control-plane client too, and
/// `ADR-014` makes `cmd/*` a composition root — *"not where an implementation
/// two tools share belongs"*. So it moved here rather than being spelled a
/// second time in a second binary, which is `ADR-011` applied to three flags.
pub mod endpoint;
