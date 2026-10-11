# Nftables Library Survey

* **Status:** Proposed — survey for the maintainer's choice
* **Date:** 2026-09-26
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Scope

`ADR-023` §10 item 1 leaves one question open before the generated firewall can
hold its first rule: which crate builds and sends the nf_tables netlink
messages — pure Rust, or a thin binding over `libnftnl`. `.rulings/` R-211
sends the work here first, because the choice adds a dependency, and that
decision is the maintainer's. This document is the survey `ADR-009` §3 asks
for before such a choice is made. **It adopts nothing.** No `Cargo.toml` in
this tree changes because of it, and nothing here was built or run against
this repository.

Two things this document deliberately does not do, so the boundary is
explicit:

* It does not design the ruleset generator itself — `ADR-023` §9's job for
  `rivet-adapter-net`, once a library is chosen.
* It does not touch `ADR-023` §8's network-namespace half of enforcement
  (`none` and `group:` isolation). The rule this survey measures every
  candidate against is the other half of §8 — rules matching socket UID in a
  shared namespace.

## 2. The requirement

Stated in the general shape every candidate is measured against, rather than
naming one particular instance, so the requirement still reads correctly after
that instance changes:

> A new TCP connection on the loopback addresses (IPv4 `127.0.0.0/8`, IPv6
> `::1`) to one of a set of ports is accepted only when the connecting
> socket's owner UID matches a given UID, and is refused otherwise.

The instance this begins with names one service's UID and its backends'
ports *(`.rulings/` R-211.)*; the requirement above is what a candidate has to
express regardless of which service or ports fill it in. `ADR-023` §2's own
pattern already generalises this way — a declared surface becomes a generated
ruleset — and this is that pattern applied to one rule.

Broken into the primitives `ADR-023` §8 and §9 assign to nftables, on the
output path, in the `inet` family, with the whole table replaced atomically in
one batch:

* **`meta skuid`** — the socket-owner test.
* **`ip daddr` / `ip6 daddr`, with a prefix** — the loopback match, in both
  families from one ruleset (`ADR-023` §7).
* **`tcp dport`, against a set** — the port or ports, anonymous or named.
* **`ct state new`, or a SYN-flag match if connection tracking is
  unavailable** — `ADR-023` §3 already rules that `egress` needs conntrack;
  this rule needs at least "new" to tell a connection attempt from an
  established one.
* **`reject` versus `drop`** for the refused case — both are surveyed below;
  which one a rule actually uses is a separate decision from this one.
* **A batch transaction**: begin, delete table, add table/chain/set/rules,
  end — so the whole table is replaced atomically every time `apply` runs
  (`ADR-023` §2).

## 3. The target kernel already carries this

Read directly from `tools/buildroot/linux/rivet.config`, the fragment the
custom kernel is built from: `CONFIG_NETFILTER=y`, `CONFIG_NETFILTER_ADVANCED=y`,
`CONFIG_NF_TABLES=y`, `CONFIG_NF_TABLES_INET=y`, `CONFIG_NF_CONNTRACK=y`,
`CONFIG_NFT_CT=y`, `CONFIG_VETH=y` and `CONFIG_PACKET=y` — every one `=y`, none
`=m`, consistent with `ADR-024` §4's rule that this build carries no loadable
modules. The fragment's own comment against `CONFIG_NFT_CT` names it as the
flag the `ct` expression needs beyond bare connection tracking, and records it
as an addition to `ADR-024` §3's table still to be folded in there.

That settles the fallback question from §2: this rule can use `ct state new`
directly on the kernel this project ships. The SYN-flag fallback stays in the
requirement stated above only because a survey answering "can a candidate
express connection-tracking state" should also record what the fallback would
be if that ever changed.

## 4. Method

`ADR-009` §3's ladder, descending: an existing Rust crate, then an existing C
component wrapped, then writing it — a step down needs a reason recorded
alongside it. §5 gives two gates a candidate has to clear before its coverage
is worth reading at all: it must cross-compile to
`aarch64-unknown-linux-musl`, and it costs more — not fatally — if it does not
also build on the Windows development host, because `AGENTS.md` §10.5 names
that host and `AGENTS.md` §4 names what happens to a suite that cannot run
there. `ADR-009` §3's own worked surveys are argued from measurement rather
than description, and every claim below names how it was checked: a
crate's own crates.io or docs.rs page, its repository, its source, or the
kernel's own uapi headers — never memory, and never something this survey
could not reach without building inside this repository or adding a
dependency to it.

One gate this survey needs that `ADR-023` §10's earlier crate mentions did not
carry a licence check against: `ADR-010` §2.1's rule on linking.

> **Aggregate copyleft. Never link it.** A GPL or LGPL library statically
> linked into a `rivet-*` binary is the one mistake that would force the
> licensing question. Every other arrangement — executing a GPL program,
> driving one over a socket, shipping one in the image — is aggregation.

`deny.toml`'s allowlist already enforces this for every dependency in the
tree, and carries no GPL, LGPL or AGPL term. Two of this survey's six
candidates fail this gate outright, below.

## 5. Candidates

### 5.1 `rustables` — pure Rust, stopped by licence

`ADR-023` §2 names `rustables` as the pure-Rust option without checking its
licence; this survey is that check.

**Licence: `GPL-3.0-or-later`.** Confirmed three ways: crates.io's own package
API (`max_stable_version 0.9.0`, `license: GPL-3.0-or-later`), the `lib.rs`
aggregator's page for the same crate, and the crate's repository page itself
(GitLab, `rustwall/rustables`, licence shown as "GNU General Public License
v3.0 or later"). Every one of its eleven published versions, from `0.8.0`
(2023-01-09) to `0.9.0` (2026-09-21, five days before this survey), carries the
same licence — this is not a recent change. Statically linking a
`GPL-3.0-or-later` crate into a `rivet-*` binary is exactly the case `ADR-010`
§2.1 names, so adopting this crate needs the maintainer to reverse that rule
for one dependency, which this survey does not do on its own.

**Coverage, for the record, since the technical shape is otherwise
relevant.** It is a fork of `mullvad/nftnl-rs` (its own crates.io description
says so), reworked to speak netlink directly rather than binding `libnftnl`,
with a `Batch` type described as an atomic set of operations. Its `expr`
module (read from its docs.rs page) publishes `Meta`, `Cmp`, `Payload`,
`Lookup`, `Immediate`, `Bitwise`, `Reject`, `Counter`, `Log`, `Nat`,
`Masquerade`, `Verdict`, and an escape hatch (`RawExpression` /
`ExpressionRaw`) for anything else. Its published module index does not list
a dedicated connection-tracking (`ct`) type, so `ct state` would go through
that escape hatch — this was checked one way (the docs.rs module index)
rather than `AGENTS.md` §7's usual two independent ways, since the licence
already settles the candidate.

**Weight:** seven direct and three build dependencies by its own manifest
(`bitflags`, `ipnetwork`, `libc`, `log`, `nix`, `rustables-macros`,
`thiserror`) — light, and beside the point here.

**Verdict: not a candidate without a fresh licensing ruling.**

### 5.2 `nftnl` + `mnl` (the Mullvad family), over `libnftnl` and `libmnl` — stopped by the same rule, one layer down

`ADR-023` §2's other named option — "thin bindings over `libnftnl`" — is this
family: `nftnl`/`nftnl-sys` and `mnl`/`mnl-sys`, all four maintained at
`github.com/mullvad/nftnl-rs` and `github.com/mullvad/mnl-rs`.

**The Rust layer's licence is clean.** `nftnl` `0.9.4` (crates.io, updated
2026-08-11), `nftnl-sys` `0.6.4` (links the native library `nftnl`, updated
2025-11-26), `mnl` `0.3.1` (updated 2026-02-10) and `mnl-sys` `0.2.2` (links
`libmnl`, updated 2025-10-29) are all `MIT OR Apache-2.0` / `MIT/Apache-2.0`.
Recent activity on the two safe-wrapper crates suggests real maintenance.

**The C libraries they bind are not clean, and this is what actually
decides it.** `libnftnl`'s own `COPYING` file (read from a mirror of the
upstream `netfilter.org` source tree) is the plain GNU General Public
License, Version 2 — corroborated by Repology's and multiple distributions'
package listings, which describe it as GPLv2 or GPLv2-or-later. `libmnl`'s own
`COPYING` (same method, a different mirror) is the GNU Lesser General Public
License, Version 2.1 — corroborated by Gentoo's package page, which lists its
licence as `LGPL-2.1`. `nftnl-sys` statically links `libnftnl`; `mnl-sys`
statically links `libmnl`; `libnftnl` itself depends on `libmnl`. Any one of
these linked into a `rivet-*` binary is precisely `ADR-010` §2.1's forbidden
case, and it is forbidden regardless of how permissively the Rust wrapper
around it is licensed — the obligation attaches to what is linked, not to the
crate that calls it.

**What it would cost if the maintainer chose to reverse that rule for this
one case, stated for completeness:** Buildroot already carries `package/libmnl`
and `package/libnftnl` — both are sourced from Buildroot's own package list,
and `libnftnl`'s Buildroot recipe already declares `libmnl` as a build
dependency — so the Buildroot-recipe cost `ADR-009` §5 asks about is close to
nothing; this project would be contributing configuration to an existing
recipe, the shape `ADR-009` §6 already prefers. Whether Buildroot's existing
recipes support the static, `aarch64-unknown-linux-musl` build `ADR-003` §4
requires of every C mechanism was not checked, because the licence question
above makes it moot without a ruling.

**Verdict: not a candidate without a fresh licensing ruling — one level
lower than §5.1, in the C library rather than the Rust crate.**

### 5.3 `netlink-packet-netfilter` (the `rust-netlink` organisation) — the strongest licence-clean candidate

The same GitHub organisation `ADR-009` §3 already surveyed once for rtnetlink
(`netlink-packet-route`, `netlink-sys`, rejected there on the Windows
development-host gate) publishes a sibling crate scoped to netfilter,
including nf_tables.

**Licence: `MIT`.** Confirmed from crates.io's package API for
`netlink-packet-netfilter` `0.4.0`, updated 2026-08-22 — four weeks before
this survey. Its own dependencies, read from its `Cargo.toml`
(`netlink-packet-core 0.9.0`, `bitflags 2.10.0`, `libc 0.2.77`, `derive_more
0.99.16`, `zerocopy 0.8`), are each `MIT`, `MIT OR Apache-2.0`, or (for
`zerocopy`) `BSD-2-Clause OR Apache-2.0 OR MIT` — every one already on
`deny.toml`'s allowlist. `derive_more` is pinned to its older `0.99` line
rather than the current `2.x`; that line's own licence was not independently
re-checked beyond crates.io's per-version record.

**Coverage, read directly from its source tree rather than assumed from the
name:** `src/nftables/` holds `table/`, `chain/`, `rule/`, `set/`,
`set_element/` and `attributes/` submodules, plus its own `message.rs`.
`attributes/expression/` holds `bitwise.rs`, `cmp.rs`, `immediate.rs`,
`lookup.rs`, `meta.rs`, `payload.rs` and `register.rs`. Its `meta.rs` defines
a `MetaKey` enum whose `SocketUserId` variant is documented against
`NFT_META_SKUID` — this rule's socket-owner test, by name. `ip`/`ip6 daddr`
with a prefix is `Payload` (reads the header bytes) plus `Bitwise` (masks
them) plus `Cmp` (compares them) — all three present. `tcp dport` against a
set is `Payload` plus `Lookup` — present. Its `src/none/` module — the
`NFNL_SUBSYS_NONE` subsystem, checked against `nfnetlink.h` below — defines a
`ControlMessage` enum with `BatchBegin` and `BatchEnd` variants, so the atomic
transaction wrapper this rule needs is modelled directly, not left to a
caller to hand-encode.

**What is not modelled, checked two ways:** the `attributes/expression/`
directory listing carries no `ct.rs` and no `reject.rs`, and the
`Expressions` enum in `attributes/expression.rs` (`Bitwise`, `Cmp`,
`Immediate`, `Lookup`, `Meta`, `Payload`, plus `Unknown` and an `Other{
expression_type, attributes }` catch-all) has no `Ct` or `Reject` variant
either. Both would need to go through that `Other` catch-all, encoding their
attributes as raw values the way `ADR-023`'s own generator would need to for
anything this crate does not model — a bounded, contained amount of hand
work rather than a rewrite.

**Dev-host buildability — the gate that sank this organisation's rtnetlink
crates, checked directly rather than assumed to recur.** `ADR-009` §3
rejected `netlink-packet-route` + `netlink-sys` because the latter calls
Linux-only `libc` symbols with no `cfg` gate, and fails to compile on
`x86_64-pc-windows-msvc`. `netlink-packet-netfilter`'s own `Cargo.toml` does
**not** depend on `netlink-sys` at all — it is a message-encoding crate, not a
transport. Reading its `lib.rs`, `nlas.rs` and `message.rs` directly finds no
`libc::` usage and no `cfg(unix)`/`cfg(windows)` gating; its
`NetfilterProtoFamily` constants (`NFPROTO_INET`, `NFPROTO_IPV4`, …) are
hand-written from the kernel's own uapi header rather than taken from `libc`
— the same design `rivet-adapter-net/src/wire.rs`'s own module documentation
already argues for and already practises for rtnetlink. `netlink-sys`'s own
`socket.rs`, read directly, does call `libc::sockaddr_nl`, `libc::PF_NETLINK`,
`libc::SOL_NETLINK` and `libc::NETLINK_PKTINFO` with no portability gate, so
it would fail on Windows exactly as its rtnetlink-side sibling did — but
nothing requires taking it. Pairing `netlink-packet-netfilter`'s message
types with **this crate's own existing `socket.rs`** (already Linux-only by
design, already opening and binding an `AF_NETLINK` socket, needing only a
new protocol constant for `NETLINK_NETFILTER` in place of `NETLINK_ROUTE`)
reuses the wire/socket split `ADR-009` §3 already established here, and
avoids `netlink-sys` entirely.

**Measured on the Windows development host: it does not build.** The reading
above covered six files and missed a seventh module. A throwaway crate outside
this repository, depending on `netlink-packet-netfilter = "=0.4.0"` and
nothing else, fails `cargo check` on `x86_64-pc-windows-msvc` with 44 errors,
every one of them in the crate's `nflog` module. That module takes
`NFNETLINK_V0`, `NFULNL_*` and `NFULA_*` from `libc` with no platform gate, and
`libc` defines those constants on Linux only. `netlink-packet-core`, the
crate's one netlink dependency, builds there cleanly. So avoiding
`netlink-sys` is not enough: the crate itself fails the gate `ADR-009` §3
applied to its rtnetlink sibling.

It could still be taken as a **Linux-only dependency**, declared under a
`cfg(target_os = "linux")` target table so the firewall code compiles only
there. Everything else in `rivet-adapter-net` would still build on the Windows
host, and the firewall's own tests would run in the build container, where
this project's other Linux-only crates are already tested.

**Verdict: licence-clean, and the nearest to ready, but only as a Linux-only
dependency.** It covers most of §2's list by name, including the batch
transaction.

### 5.4 `neli` — rejected again, and more clearly than the first time

`ADR-009` §3 already measured `neli` `0.7` for rtnetlink on 2026-08-23 and
rejected it: it resolves to 33 crates including `parking_lot` (a lock
implementation, in a single-threaded program) and `serde` (which
`rivet-config` deliberately keeps out of this workspace), and it failed to
compile on the Windows development host with 485 errors. Read from crates.io
on 2026-09-26 for this survey: the published crate is `0.7.4`
(`BSD-3-Clause`, updated 2026-01-28), still the `0.7` line that ADR measured
— this survey did not re-run the compile check, since doing so would mean
adding the dependency to a repository this work package may not touch, and
instead relies on `ADR-009`'s own recorded, dated measurement.

`neli` is a generic netlink attribute/socket library with no nf_tables
vocabulary of its own — no table, chain, rule, set or expression types.
Adopting it for this rule would mean writing every `NFT_MSG_*`/`NFTA_*`
constant and expression encoder by hand on top of it anyway, which is §5.6's
work, while also carrying `neli`'s own weight and its recorded host-build
failure. It offers nothing here that §5.6 does not offer for less.

**Verdict: rejected, for the reason `ADR-009` §3 already gave, sharper here
because there is no nf_tables-specific value to weigh against the cost.**

### 5.5 JSON over the `nft` binary, and one crate found in passing — rejected on principle

The `nftables` crate (`nftables-rs`, `MIT OR Apache-2.0`, `0.6.3`, last
updated 2025-08-15) drives `nft`'s own JSON input/output. `ADR-023` §2 is
explicit that there is no `nft` binary in the image and nothing to supervise;
this candidate's entire design assumes one exists, so it is rejected on that
architectural ground regardless of its technical merit, exactly as the work
package instructing this survey asks.

One more crate turned up while searching for this family: `jip-nftables`
(`MIT OR Apache-2.0`), which advertises reading and writing nf_tables
directly over `NETLINK_NETFILTER` with no C FFI — the same shape as §5.3.
Its `created_at` and `updated_at` on crates.io are identical
(2026-04-18), meaning one release and no update since, and its download count
(259 total, 140 in the most recent period) is small next to every other
candidate here. Named for completeness; a single-release crate with no
revision history is not weighed as a serious alternative to §5.3 in this
survey.

### 5.6 Hand-written in `rivet-adapter-net` — the rung-3 candidate

`rivet-adapter-net`'s own rtnetlink implementation, read directly, already
gives a concrete floor and ceiling for what this would cost, rather than a
guess.

**What is already there and would need nothing:** `port.rs`'s `SendRecv`
trait and its `transact`/`dump`/`expect_ack` functions operate on message
bytes and a sequence number; nothing in that file names `NETLINK_ROUTE` or
any rtnetlink type, so it applies to nf_tables unchanged. `socket.rs`'s
`NetlinkSocket` opens an `AF_NETLINK` socket against whichever protocol
constant `wire.rs` supplies; supporting nf_tables needs one new constant
(`NETLINK_NETFILTER`) in place of `NETLINK_ROUTE`, and no other change to
that file's six `unsafe` blocks.

**What would be new**, sized against the kernel's own uapi headers
(`include/uapi/linux/netfilter/nf_tables.h` and `nfnetlink.h`, read from the
current upstream kernel source): a 4-byte `nfgenmsg` header
(`nfgen_family`, `version`, a big-endian `res_id`) following the existing
16-byte `nlmsghdr`; a netlink message type composed as
`(subsystem << 8) | operation`, with `NFNL_SUBSYS_NFTABLES = 10`;
message-type constants for `NEWTABLE`/`NEWCHAIN`/`NEWRULE`/`NEWSET`/
`NEWSETELEM` and their `DEL*` counterparts, plus `NFNL_MSG_BATCH_BEGIN`/`END`
for the transaction wrapper; attribute tables for table, chain, rule, set and
set-element objects (`nft_table_attributes` alone has seven, excluding its
`UNSPEC`/`__MAX` sentinels, and `nft_rule_attributes` eleven); and encoders
for the eight expressions §2
needs — `meta` (with `NFT_META_SKUID` among roughly three dozen keys),
`cmp`, `payload`, `lookup`, `immediate`, `bitwise`, `ct` (`NFT_CT_STATE`
among roughly two dozen keys) and `reject` (with its `ICMP_UNREACH`/
`TCP_RST`/`ICMPX_UNREACH` types) — plus the recurring "value or verdict"
data union those expressions share. None of this is present in `wire.rs`:
its `push_attr` writes one flat, unnested attribute at a time, and
nf_tables' expression list is a nest of attributes inside attributes, which
needs a new "open a nest, remember where, patch its length once the contents
are written" helper alongside it.

**A rough size, stated as an order of magnitude rather than a delivered
count**: `ADR-009` §3 already flagged nftables as "an order of magnitude more
wire surface" than rtnetlink's four requests, before this survey went
looking for the specifics above. `wire.rs`'s rtnetlink support is under 600
lines including its doc comments, covering four message types, two reply
shapes and no nesting; matching the eight expressions and five object kinds
above, plus the nesting helper and its own tests, in this crate's existing
style (roughly as much test code as production code, going by `wire.rs` and
`port.rs`) is several times that surface before `rivet-netd`'s own
ruleset-generation logic sits on top of it.

**Cost:** no new dependency, no licence question, and the closest fit to
this crate's existing architecture and test style — the fake-`SendRecv`
pattern `testnetlink.rs` already established for rtnetlink applies unchanged.
The cost is the size above, in a protocol area this crate's tests do not
exercise, so any subtle nf_tables wire-format mistake is one this survey's
own reading, not an existing test, would have to have caught.

**Verdict: the clean rung-3 fallback.** Zero dependency and no licence risk,
at the cost of the most new code of any candidate here, in code this project
would own and have to keep correct against a protocol it has not tested
against a kernel yet.

## 6. Testing this without root

This question does not distinguish the candidates from each other — it is a
property of nf_tables over netlink, not of which library builds the bytes —
so it is answered once, here, rather than per candidate.

**The container this project already tests in runs unprivileged.**
`tools/container/Dockerfile`'s own comment states it runs with no
`--privileged`, no `--cap-add`, no `/dev/loop` and no `--device`, as an
ordinary UID; `tools/container/build-card.sh`'s comment says the same. The
actual invocation in `tools/container/exec.sh` — `docker run --rm $USER_ARG -w
/rivet $MOUNTS $ENVS "$IMAGE" "$@"` — adds no capability or privilege flag at
run time either.

**Wire-format tests need none of that, for any candidate.** Encoding a
request into bytes and comparing it against a fixed expected array, or
decoding a hand-built reply and checking what comes out, is exactly what
`rivet-adapter-net`'s existing rtnetlink tests already do against a fake
`SendRecv` (`testnetlink.rs`) rather than a real socket. The same pattern
applies unchanged to nf_tables messages, whichever candidate builds them, and
needs no capability, no root and no kernel.

**A real kernel-level test — actually loading a table and observing it match
or refuse a connection — needs `CAP_NET_ADMIN`,** which the container above
does not grant. The standard way to get one without host root is an
unprivileged user-and-network namespace pair (`CLONE_NEWUSER` combined with
`CLONE_NEWNET`, the mechanism behind `unshare --user --net`): an ordinary
process gains full network-admin rights inside a namespace nothing else can
see, without any capability on the host outside it. This is unrelated to
`rivet.config`'s own `CONFIG_USER_NS` being off — that setting governs the
`aarch64` card kernel this ruleset would eventually run under, not the `x86`
container the test suite already runs in. Whether the container host's
own kernel permits unprivileged user namespaces was not checked in this
survey. Wiring a namespaced integration test into the container is a
test-harness change beyond a library survey, and is not attempted here.

## 7. Comparison

| Candidate | Licence | Clears `ADR-010` §2.1 / `deny.toml` | Table/chain/rule/set/batch | `ct state` | `reject` | New dependencies (rough) | Dev-host build |
| :-- | :-- | :-- | :-- | :-- | :-- | :-- | :-- |
| `rustables` | `GPL-3.0-or-later` | **No** | yes | via raw escape hatch | modelled | ~10, by its own manifest | not checked — moot |
| `nftnl`+`mnl` / `libnftnl`+`libmnl` | Rust layer MIT/Apache-2.0; `libnftnl` GPL-2.0, `libmnl` LGPL-2.1 | **No** (the linked C libraries) | yes | native | native | Rust layer light; two C libraries | not checked — moot |
| `netlink-packet-netfilter` | MIT | Yes | yes, incl. batch begin/end | via raw escape hatch | via raw escape hatch | ~5 direct, all allow-listed | **fails**: 44 errors, all in its `nflog` module's Linux-only `libc` constants; usable only as a Linux-only dependency |
| `neli` | BSD-3-Clause | Yes | no (generic library) | would be hand-built on top | would be hand-built on top | 33, incl. `parking_lot`/`serde` (`ADR-009` §3) | measured failing, `ADR-009` §3 |
| `nftables` (JSON via `nft`) | MIT OR Apache-2.0 | Yes | n/a — rejected on architecture | n/a | n/a | n/a | rejected before this question applies |
| Hand-written, no dependency | n/a | Yes | to be written | to be written | to be written | 0 | unchanged — this crate already builds there |

## 8. Recommendation

**Recommended: the hand-written extension of `rivet-adapter-net` in §5.6.**
This survey's first choice depended on `netlink-packet-netfilter` building on
the Windows development host, and measured there, it does not (§5.3). By the
survey's own rule, that makes the hand-written extension the recommendation.
It is also how this crate already speaks rtnetlink, for the same reasons
`ADR-009` §3 gave: no dependency, a build on both hosts, and tests against the
fake `SendRecv` that need no kernel.
* **Cost:** the largest amount of new protocol code of any option, in a wire
  format proven only once it has run against a real kernel.
* **Still to be written whichever option is chosen:** `rivet-netd`'s
  ruleset-generation logic.

**Alternative: `netlink-packet-netfilter` as a Linux-only dependency.**
* **Benefit:** the least new code. The crate already models tables, chains,
  rules, sets, the batch transaction, and most of the expressions §2 needs.
* **Cost:**
  * One new dependency tree, all on `deny.toml`'s allowlist, which brings two
    procedural macros into the build (`derive_more` 0.99, `zerocopy-derive`).
  * `ct` and `reject` still have to be hand-encoded through its raw escape
    hatch.
  * The firewall code and its tests compile only on Linux. They run in the
    build container, as the supervisor's already do, and not on the Windows
    host.

**A middle course: vendor the crate's `nftables` module (MIT) without
`nflog`.** It builds on both hosts and reuses the encoders. The cost is a
trimmed copy of an upstream crate to keep in step by hand, the maintenance
`vendor/`'s one existing entry already shows.

**Not a candidate without a fresh licensing ruling:** `rustables` (§5.1) or
the `nftnl`/`mnl` binding over `libnftnl`/`libmnl` (§5.2). Either would need
the maintainer to explicitly accept a GPL or LGPL library statically linked
into a `rivet-*` binary for this one case — reversing `ADR-010` §2.1 rather
than applying it — which is a licensing-policy decision this survey is not
positioned to make.

**Rejected:** `neli` (§5.4), for carrying no nf_tables-specific value against
a weight and a host-build failure `ADR-009` §3 already measured once; and the
JSON-over-`nft` crate family (§5.5), which `ADR-023` §2 forecloses on
architectural grounds regardless of technical merit.

## 9. What this survey could not verify

* **No dependency was added and nothing was compiled or run in this
  repository**, per this work package's own rules. Every coverage and
  buildability claim above is read from a crate's published crates.io or
  docs.rs page, its repository, its source, or the kernel's own uapi headers,
  and says which; none is from memory.
* **`netlink-packet-netfilter`'s Windows build was measured after this survey
  was first written**, in a throwaway crate outside the repository. It fails
  (§5.3). No build for `aarch64-unknown-linux-musl` was attempted, although a
  Linux-only dependency would need one.
* **Whether the container host's own kernel permits an unprivileged
  user-and-network namespace pair was not checked**, so §6's proposed route to
  a root-free kernel-level test is unconfirmed.
* **Buildroot's existing `libmnl`/`libnftnl` recipes were confirmed to exist,
  not to produce a static `aarch64-unknown-linux-musl` build.** The licensing
  question in §5.2 makes that check moot for this survey, so it was not
  pursued further.
* **Neither `rustables`' nor `netlink-packet-netfilter`'s `unsafe` surface was
  counted.** Both are, by their own descriptions and the module structure read
  above, safe abstractions over raw netlink bytes and (for `rustables`) a
  socket, in the same spirit as this crate's own `wire.rs`/`socket.rs` split;
  neither was audited line by line.
