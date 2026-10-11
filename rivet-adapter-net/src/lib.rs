//! Rivet's netlink adapter: interfaces, addresses and routes.
//!
//! `ADR-014` §2 names this crate's territory in six words — *"netlink,
//! addressing, routes, DHCP, mDNS"* — and `PLAN.md` §2.2 names the part of it
//! Stage 1 needs: **`eth0` up, address and route via netlink from Rust.** That
//! is what is here. DHCP is [`dhcp`], which refuses; mDNS is the responder's
//! socket, [`mdns`].
//!
//! `ADR-023` §9 puts the nftables ruleset here too, for the good reason that
//! nftables is also kernel state over netlink: [`nft`] is the wire format and
//! [`firewall`] is Rivet's one table, written by hand beside the rtnetlink
//! code rather than through a library (`ADR-023` §2).
//!
//! ## The shape of this crate
//!
//! | Module | Holds |
//! | :-- | :-- |
//! | [`wire`] | rtnetlink's wire format: constants, encoding, and refusing a bad reply. No syscalls, not Linux-only |
//! | [`port`] | the seam: the `SendRecv` port, `Errno`, `Failure`, and reading an acknowledgement |
//! | [`link`] | finding an interface by name, and setting `IFF_UP` |
//! | [`address`] | assigning an IPv4 address |
//! | [`route`] | installing the default route |
//! | [`wired`] | the whole bring-up in order, and the verdict it produces |
//! | [`dhcp`] | the seam `udhcpc` will arrive through. **It refuses**, and the type forbids it doing otherwise |
//! | [`nft`] | nf_tables' wire format: the batch, objects, expressions, and reading a rule back. No syscalls |
//! | [`firewall`] | Rivet's one nftables table: the policy, the transaction that replaces it, and the read-back |
//! | [`mdns`] | the mDNS responder's UDP socket and `uname(2)`. Linux only for the syscalls, which carry their own `unsafe` blocks |
//! | `socket` | the real `AF_NETLINK` socket. Linux only |
//! | `testnetlink` | the fake, under `cfg(test)` |
//!
//! The split is the one `cmd/rivet-initd` established and it is not tidiness.
//! Every branch that matters at Stage 1 — an interface that is not there, one
//! that is already up, an address already assigned, a route already present,
//! `EPERM` from a process without `CAP_NET_ADMIN`, a kernel reply that does not
//! parse — needs a kernel and a network to reach, and **at Stage 1 it cannot be
//! reached from the emulator either.** `PLAN.md` §0 records why: the Pi kernel
//! binds no virtio device under QEMU's `-M virt`, so there is no emulated
//! interface to bring up. Putting the decisions above a port is what makes them
//! reachable from a fake.
//!
//! ## Why nothing here depends on a netlink crate
//!
//! `ADR-009` §3 says survey before writing and record the reason for the step
//! taken. This is that record. It is longer than the equivalent passage in
//! `cmd/rivet-initd` because the answer here is rung 3 of `ADR-009` §3's
//! ladder and the ADR names netlink specifically as a rung-1 problem — §3.1
//! calls a netlink binding *"a solved problem that someone else maintains"* and
//! §3.4's take-freely column lists *"platform bindings (netlink, ioctl)"*. **A
//! step down the ladder against a named example needs more than a preference,
//! and the case is set out below so the next reader can disagree with it.**
//!
//! Everything asserted here was checked by resolving and compiling the crates
//! on 2026-08-23, not read from memory.
//!
//! ### `rtnetlink` — rejected: it is an async API, not an async option
//!
//! The obvious candidate, and the one `ADR-009` §3's table names. `rtnetlink`
//! 0.23.0's entire surface is `handle.link().set(…).execute().await`, and
//! `new_connection()` — the only constructor — **does not exist unless
//! `tokio_socket` or `smol_socket` is enabled**. That is not inferred from the
//! manifest; a program calling it with `default-features = false` fails to
//! compile with *"cannot find function `new_connection` in crate
//! `rtnetlink`"*. With defaults off, `futures-channel` and `futures-util`
//! remain non-optional, so eight crates and a proc macro arrive whether or not
//! a runtime does.
//!
//! `ADR-021` §4.1 rejected `ntpd-rs` on exactly this ground — a non-optional
//! `tokio`, for a job that is one round trip — and `ADR-009` §3.3 names *"a
//! crate that pulls a large runtime behind it"* and *"wrong altitude: a crate
//! written for an application, used in PID 1"* as the two ways reuse quietly
//! costs more than it saves. The precedent applies here more sharply than it
//! did there: `rivet-netd`'s Stage 1 job is four synchronous requests at
//! boot, in a process supervised by `ADR-017`'s own supervisor, in an image
//! whose entire premise (`ADR-003`) is a small static binary.
//!
//! ### `neli` — rejected: weight and vocabulary
//!
//! Pure Rust, synchronous, and genuinely capable. It resolves to **33 crates**,
//! including `parking_lot` — a lock implementation, in a program that is single
//! threaded — and `serde`, which arrives through `neli-proc-macros`.
//! `rivet-config` went to some length to keep `serde` out of this workspace
//! and wrote down why; pulling it back in through a netlink binding would
//! undo that without anyone deciding to. Its API is also heavily generic and
//! derive-driven, which makes it the hardest of the three to wall behind
//! Rivet's own types (`ADR-009` §3.4).
//!
//! ### `netlink-packet-route` + `netlink-sys` — the close call
//!
//! This is the option that deserved to win, and the decision against it is the
//! one worth arguing with. Thirteen crates, no async runtime, MIT throughout,
//! a synchronous `Socket`, and `netlink-packet-route` is exactly the "protocol
//! implementation, parser, format" that `ADR-009` §3.4 says to take freely.
//!
//! **It is rejected on one measured fact: neither crate builds for the
//! development host.** `cargo check` on `x86_64-pc-windows-msvc` fails with
//! twelve errors in `netlink-packet-route`'s `address_family_fallback.rs`
//! (`cannot find value AF_INET in crate libc`, and the same for `AF_LOCAL`,
//! `AF_UNIX`, `AF_INET6`); `neli` fails the same way with 485. Taking either
//! makes this crate Linux-only, and therefore makes **its whole test suite
//! unrunnable on the machine it is written on** — `AGENTS.md` §10.5 records
//! that the development host is Windows, and `AGENTS.md` §4 records what
//! happens to a suite that cannot be run: *it will rot silently*.
//!
//! That cost is not hypothetical. `cmd/rivet-initd` and `cmd/rivet-sysd`
//! already cannot execute their tests here — they are Linux-only by nature,
//! since a syscall port has to name syscalls. **A netlink adapter does not have
//! to be**, because netlink is bytes, and this crate is built so that the wire
//! format, every decision and every failure path compile and run on the host,
//! with only `socket` needing a kernel. That property is what the dependency
//! would have spent.
//!
//! ### What was actually written, and how small it is
//!
//! Stage 1 sends **four requests** — `RTM_GETLINK` by name, `RTM_NEWLINK` with
//! `IFF_UP`, `RTM_NEWADDR`, `RTM_NEWROUTE` — and reads two kinds of reply. The
//! payloads are fixed-layout C structs of 16, 8 and 12 bytes plus four-byte
//! aligned TLVs. That is struct packing, not a protocol with a parser's worth
//! of edge cases, and netlink checks it: a malformed request comes back as
//! `NLMSG_ERROR` with an errno, immediately and synchronously, rather than
//! being accepted and misbehaving later.
//!
//! ### The tripwire, so this decision is revisited rather than inherited
//!
//! **The reasoning above is scoped to Stage 1's four requests. It does not
//! generalise, and two things that are already in the record would overturn
//! it:**
//!
//! * **Parsing broadcasts.** Watching `RTMGRP_LINK` for carrier changes, or
//!   reading what a DHCP client did, means decoding messages Rivet did not
//!   construct — the full `IFLA_*` and `IFA_*` attribute space, which is a
//!   parser and is exactly where a bespoke one is *"worse in ways that surface
//!   as intermittent failures years later"* (`ADR-009` §3.3).
//! * **nftables.** An nftables ruleset is an order of magnitude more wire
//!   surface than anything here. Its own survey,
//!   `docs/design-candidates/Nftables-Library-Survey.md`, was run, and the
//!   messages are hand-written in [`nft`] for the reasons it gives
//!   (`ADR-023` §2).
//!
//! When either arrives, re-run this survey rather than extending what is
//! written here by habit. The wall is already in the right place for that:
//! `ADR-009` §3.4 is satisfied because no netlink type reaches `rivet-core`
//! or Rivet's configuration — [`wired`] returns Rivet's own outcomes — so
//! swapping the implementation is a change to three modules and not a redesign.

pub mod address;
pub mod dhcp;
pub mod firewall;
pub mod lease;
pub mod link;
pub mod mdns;
pub mod nft;
pub mod port;
pub mod psk;
pub mod route;
pub mod supplicant;
pub mod wire;
pub mod wired;

// The only Linux-only module, which is the whole point of the split above.
#[cfg(target_os = "linux")]
pub mod socket;

#[cfg(test)]
mod testnetlink;

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    /// The survey in this module's documentation makes claims that will look
    /// like prose to a future reader and are in fact load-bearing. `AGENTS.md`
    /// §4 asks for a cheap guard rather than an acknowledgement, and the
    /// cheapest one available is that the dependency the survey rejects is
    /// still absent — checked against the manifest, which is the thing that
    /// would change if somebody added it.
    ///
    /// This is weaker than proving the reasoning is still right, and it is not
    /// nothing: it fails the moment a netlink crate is added without the survey
    /// above being revisited, which is the failure mode `ADR-009` §3 exists to
    /// prevent.
    #[test]
    fn no_netlink_crate_has_been_added_without_revisiting_the_survey() {
        let manifest = include_str!("../Cargo.toml");
        // Only the dependency lines matter; the survey summary in the comments
        // names all four crates on purpose.
        let declared: Vec<&str> = manifest
            .lines()
            .map(str::trim)
            .filter(|line| !line.starts_with('#') && line.contains('='))
            .collect();
        for rejected in ["rtnetlink", "neli", "netlink-packet", "netlink-sys"] {
            assert!(
                !declared.iter().any(|line| line.starts_with(rejected)),
                "{rejected} was added as a dependency; the survey in src/lib.rs must be revisited and corrected, not left standing"
            );
        }
    }
}
