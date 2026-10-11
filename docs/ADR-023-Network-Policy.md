# ADR-023: Network Policy

* **Status:** Accepted — ruled 2026-08-22. **One rule of it is built; the rest
  is not — see §3A.** The rulings stand, and every row of §3's table other
  than the one §3A names is a design written in the present tense.
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

**The maintainer's question was whether a firewall means another C dependency,
and whether he would end up writing rules. His hope was that a user interacts
only with the unified config and admin surface, and with requests to grant
permissions — he does not want to sit around writing firewall rules, and asked
whether an application's manifest could tell the operating system which
permissions and ports it requires.** *(`.rulings/` R-042.)*

Both answers are better than hoped. **Rivet has no firewall configuration
surface at all** — not a nicer one, none — and there is no C daemon.

---

## 2. No daemon: nftables is kernel state over netlink

Unlike `wpa_supplicant` (`ADR-004`), this is not a daemon problem. **nftables is
configured over netlink**, the same mechanism `rivet-adapter-net` already uses
for addresses and routes.

So there is no `nft` binary in the image, nothing to supervise, and no control
socket to wrap. The messages can be written in Rust, either by hand beside
`rivet-adapter-net`'s own rtnetlink code or with a licence-clean message crate.
The two libraries most often named for this are both copyleft: `rustables` is
GPL-3.0, and the bindings over `libnftnl` link GPL-2.0 and LGPL-2.1 C
libraries. `ADR-010` §2 forbids linking either. There is no C *program* either
way. `docs/design-candidates/Nftables-Library-Survey.md` weighs the options.

Two properties matter and both fit decisions already taken:

* **A ruleset is replaced atomically**, in one netlink transaction. There is
  never a half-applied firewall — which is exactly `apply`'s model
  (`ADR-015` §7).
* **The `inet` family covers IPv4 and IPv6 in one table.** Handling both is
  therefore free if done now and a hole if forgotten. §7.

---

## 3. The ruleset is generated, and this is the fourth instance

`ADR-015` §3.2 already has applications declare what they need, and
`ADR-015` §5.2 already has Rivet open their listening sockets. **Rivet
therefore already knows exactly what every application requires, from a file it
already reads.**

> **The ruleset is generated from manifests and grants, exactly as `/etc` is
> generated from configuration.**

That is the same pattern as command output (`ADR-006` §4), `/etc`
(`ADR-007` §3.6) and the journal (`ADR-022` §2). Fourth instance, no new idea.

| Declared | Becomes |
| :-- | :-- |
| `network = "none"` | no interfaces at all — nothing to filter |
| `network = "group:media"` | shared namespace; nothing crosses out of it |
| `listen = ["tcp:8080"]` | inbound permitted to that port and nothing else |
| `network = "egress"` | outbound permitted, with replies |
| **anything undeclared** | **denied** |
| **`proxy-path`, and its routed port** | **a new loopback connection to that port is accepted only from `rivet-proxyd`'s uid and root — BUILT, §3A** |

The last row is the one rule built so far. The proxy exists so an application
is reached at a memorable `<host>/<application>` address, and its gate
(`ADR-046` §4) means nothing if another program on the machine can open the
application's port on loopback directly. **The rule guards only the routed
port**: an application's further ports, its `:direct` ports, and every port of
an application with no `proxy-path` are its own to declare, and none of them is
in the table. *(`.rulings/` R-210, R-211.)*

**Default deny is enforceable precisely because everything that needs a port
already declares one.** On a conventional system default-deny is aspirational,
because nothing knows what the software needs.

### 3A What is built, and what is not — added 2026-08-30, and this section is what a reader needs first

*(Numbered `3A` rather than `3.2` because it belongs before §3.1 in reading order
and nothing is ever renumbered — `ADR-015` §3.2A and `ADR-035` §4A are the same
device.)*

**§3's table describes what a declaration WILL become, except its last row,
which is built.**

| | |
| :-- | :-- |
| the guard on proxied ports — §3's last row | **built.** `rivet-adapter-net`'s `firewall` module, applied by `cmd/rivet-sysd` (§9). One table, `inet rivet`, one chain on the output hook with policy `accept`, and per routed port two rules, IPv4 `127.0.0.0/8` and IPv6 `::1`: a new TCP connection is dropped unless its socket's uid is `rivet-proxyd`'s or root's |
| the rest of the generator — `listen`, `egress`, default deny | **does not exist.** Only the guard above is generated |
| a network namespace, so `network = "none"` and `network = "group:…"` mean something | **does not exist.** `cmd/rivet-sysd` says so at every application launch |
| the grant half of §4 — *exposed on lan* in a `plan` | **does not exist.** `plan` has no exposure grant to show, because there is nothing to apply it to |

**What this means for somebody writing an application:** a port your
application binds is reachable from wherever the machine is reachable, and
nothing you do or do not grant changes that — **except that the port the proxy
routes to is reachable on loopback only by the proxy and by root.** A `listen`
port an application declares beyond that is its own.

**The guard's shape, and why each part is what it is:**

* **Root is admitted** because refusing it would protect nothing — a root
  process can delete the table — and because the control plane runs as root
  and relays backups to an application's routed port (`ADR-051`).
* **`drop`, not `reject`.** `reject` is a kernel module on the target kernel
  (`CONFIG_NFT_REJECT=m`) and a card carries no modules, so a rule naming it
  would be refused and no table would exist. The cost is that a refused caller
  waits out its own connection timeout instead of failing at once.
* **The chain's policy is `accept`**, so a defect in the table can refuse only
  what its rules name and cannot cut the machine off its network. **The control
  plane's port is never guarded**, whatever a manifest declares.
* **The table is replaced whole, in one transaction**, and nothing Rivet sends
  names any other table. It is read back from the kernel after every apply and
  the journal reports the reading, not the request. A refused transaction is
  applied not at all and is journalled; the machine goes on.
* **The policy is derived, never stored**: from the installed manifests at
  boot, after a finished install, and after an uninstall.

**Why the tense is the defect rather than any particular ruling.** A design
written as *"a declared port is localhost-only"* reads, to a reader who arrives
needing an answer, as a statement about the machine in front of them — and
`ADR-012` §7.4's doctrine is that a confident sentence a reader cannot test is
the expensive kind of wrong. §6.1 of `ADR-015` requires that an unenforced
declaration be **visible**, and this section is that visibility for this ADR.

**The rest of the generator is not queued work.** The namespaces are `ADR-015`
§6's **Next** stage, and the remaining rows of §3 build on the same table and
transaction when they are taken up.

### 3.1 `egress` requires connection tracking

Worth stating because it is the detail that makes the table above work: outbound
permission is meaningless without `ct state established,related accept`, or
replies never return. That means `nf_conntrack` in the kernel configuration, and
it joins Landlock, cgroups v2 and `nf_tables` on the list of flags to confirm.

---

## 4. The operator's only decision: how far a port reaches

An application can say "I need to bind 8080". It cannot sensibly say whether
that should be reachable from the network — **that is an operator judgement and
the one least appropriate to delegate to a package.**

**Ruled 2026-08-22: a declared port is localhost-only unless exposure is
granted.** Installing something never silently opens a port to the network.

**The ruling stands and the tense is wrong — corrected 2026-08-30.** Read it as
*a declared port WILL BE localhost-only*. Today installing something and enabling
it does open its port to the network, because §3A's ruleset generator does not
exist; that is the gap this ruling names, not a description of it being closed.

It costs one decision per exposed service, in the diff already being confirmed
(`ADR-015` §5.4):

```
+ app "metrics-collector" v0.3.1
    requests: bind tcp:8080
    grants:   listen 0.0.0.0:8080  (exposed on lan)
```

So the entire firewall interaction is answering "how far does this reach?" once,
at install. There are no rules, ever.

### 4.1 The bootstrap exception to a default-deny firewall

A consequence that would otherwise brick the first boot: **whatever makes a
headless board reachable is a declared listener like anything else**, and under
§4's default it would bind localhost-only — making a freshly flashed, headless
Pi unreachable by the only means of reaching it.

**The image defaults layer (`ADR-007` §3.4, layer 2) grants that one exposure.**
It is a grant like any other, visible in `--explain` and removable by an operator
who has another way in, rather than a special case hidden in code.

**Today that listener is `rivet-mcpd` on 7777**, and `ADR-017` §6.2 and §6.2.1
are where the bootstrap argument is made and proven. This section is about the
*shape* of the exception rather than about any one program, because the subject
has already moved once: a default-deny firewall must except whatever makes a
headless board reachable at all, and that stays true the day §3A's generator is
written.

---

## 5. No escape hatch

**Ruled 2026-08-22: there is no raw-rules field**, and nothing but the generator
may write the ruleset.

If something cannot be expressed, **the configuration schema gains a typed
field.** A raw-rules escape hatch would mean two things own the firewall, and
the generated half could no longer be reasoned about alone — which is precisely
the second authority `ADR-011` forbids, and precisely the argument `ADR-008` §5
already made about never adding a file to `/etc`.

A gap here is a schema bug, and should be fixed as one.

---

## 6. Reach is one dimension, not zones

**Ruled 2026-08-22: local versus exposed, with no per-interface trust zones.**

A Pi on a home network has one usable link at a time, so zones would be
machinery for a distinction that does not yet exist. Adding them later is a
schema addition — `exposed` becoming `exposed-on: [lan, fleet]` — rather than a
redesign, because the grant is already a typed field rather than a rule.

---

## 7. IPv6 is covered from the first ruleset

Not an enhancement. A generated ruleset that covers only IPv4 is a hole that
nobody notices until it is exploited, and the `inet` family makes covering both
the *same* work rather than twice the work.

Stated explicitly because "IPv6 later" is the single most common way this goes
wrong on small systems.

---

## 8. Enforcement: namespaces first, rules second

Two mechanisms are available and they are not interchangeable.

| | Network namespace | Rules matching UID / cgroup |
| :-- | :-- | :-- |
| Isolation | total — the app cannot see the network | app sees interfaces; attempts are denied |
| Cost | `veth` pair, addressing and NAT per app | none — the app is simply filtered |

**Decision:** namespaces for `none` and `group:` — where isolation is the entire
point and there is no egress plumbing to build — and **shared namespace with
nftables matching on socket UID or cgroup for `egress`**, where the application
needs real network access anyway and per-app `veth` and NAT is a great deal of
per-start plumbing for a modest gain.

This is available only because every application already has its own UID
(`ADR-015` §5.1) and its own cgroup (`ADR-017` §3.1.3). Two decisions taken for
other reasons pay for a third.

Full per-application namespaces with egress remain the stronger endpoint if it
is ever wanted, and moving there changes the adapter and nothing above it.

---

## 9. Where it lives

The nftables messages, the transaction and the read-back are in
**`rivet-adapter-net`**, which already owns netlink; they are written by hand
there, with no nftables library (§2, §10 item 1). **`cmd/rivet-sysd`** derives
the policy and applies it.

**The supervisor, not `rivet-netd`, because the policy is a fact about
installed applications.** The supervisor already reads every installed manifest
to write the proxy's routing table (`ADR-046` §5), in the same pass, and it is
the process that sees an install finish and an uninstall complete. `rivet-netd`
runs once at boot and reads the network document; handing it the policy would
mean a second reader of the manifests, or re-running the interface bring-up on
every install. The supervisor's `Kernel` port only opens the socket; every
decision about the bytes is the adapter's, tested there against a fake socket.

The firewall is **not** a service (`ADR-017` §2). It is kernel state, applied
when the installed set changes and reconciled like anything else.

---

## 10. Open

1. **Which nftables crate — settled: none.** The messages are written by hand
   in `rivet-adapter-net`, as `docs/design-candidates/Nftables-Library-Survey.md`
   recommended under `ADR-009`'s survey-first rule. It keeps the adapter
   building and testing on every development host and adds no dependency.
   *(`.rulings/` R-248.)*
2. **Kernel flags — CONFIRMED 2026-08-22 by the audit in
   `ADR-024` §8.1**, run against a Raspberry Pi 4 on stock Raspberry Pi OS
   (`6.18.34+rpt-rpi-v8`). This ADR's own flags all pass: `CONFIG_NF_TABLES=m`,
   `CONFIG_NF_TABLES_INET=y`, `CONFIG_NF_CONNTRACK=m`. §3.1's requirement is
   satisfiable on a stock kernel today.

   **But the two flags this item borrowed from `ADR-015` §5.1 did not pass**, and
   that is the part worth carrying back here: `CONFIG_SECURITY_LANDLOCK` is not
   set at all, and cgroups v2 is mounted but its **memory controller is disabled
   on the kernel command line** (`cgroup_disable=memory`). The custom kernel is
   consequently scheduled rather than optional (`ADR-024` §8.1).

   Two consequences for this ADR specifically. **`nf_tables` and `nf_conntrack`
   are modules**, which contradicts `ADR-024` §4's "everything is built in" — so
   the generated ruleset works on a stock kernel but the no-modules goal needs
   the custom build. And **the flags being present is not the same as the
   ruleset being right**: what settles a rule is applying it to a running
   kernel and reading it back. The guard on proxied ports (§3A) has been, on a
   Raspberry Pi 4 running this project's own kernel, where `/proc/config.gz`
   reads `CONFIG_NF_TABLES=y`, `CONFIG_NF_TABLES_INET=y`, `CONFIG_NFT_CT=y` and
   `CONFIG_NFT_REJECT=m`; the read-back is kept as a test fixture in
   `rivet-adapter-net`. The rest of §3's table has not.
3. **Whether `group:` namespaces need egress**, which would reintroduce the
   `veth` and NAT plumbing §8 avoids for the single-application case.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### In §2 — the two libraries it named were never licence-checked

**Changed 2026-09-26**, when the survey §10 item 1 asked for was carried out.
§2 read:

> ~~Rust options exist at both altitudes — `rustables` targets pure Rust over
> netlink, and thin bindings over `libnftnl` exist if that proves unreliable.
> The worst case is a small C *library*; there is no C *program*.~~

Both options are copyleft. `rustables` is GPL-3.0-or-later, and `libnftnl` and
`libmnl` are GPL-2.0 and LGPL-2.1. `ADR-010` §2 forbids linking any of them, so
neither was ever an option. §2 now names what is.

### In §4.1 — the heading named one program as the requirement

**SPENT 2026-08-30**, when the program it named left the image. The heading had
asserted that a *particular* listener must be exposed in the image defaults,
which made a durable rule read as a fact about one component.

The section is kept because its *reasoning* — that a default-deny firewall must
have an exception for whatever makes a headless board reachable at all —
survives its subject and applies unchanged to the control plane.

### In §9 — `rivet-netd` was to drive the firewall

**Changed 2026-10-04**, when the first rule was built. §9 read:

> ~~**`rivet-netd`** is the service that drives it, as it already owns
> interfaces and addressing.~~

That was written when `rivet-netd` was expected to become a long-running daemon.
It runs once at boot and reads only the network document, while the first rule
is derived from installed manifests and must follow installs and uninstalls —
which the supervisor already reads and sees. §9 now gives the argument.

### In §3A and the status line — nothing was built

**Changed 2026-10-04**, when the guard on proxied ports was built
(`.rulings/` R-210 D2, R-248). The status line read
~~**NOTHING IN IT IS BUILT — see §3A, added 2026-08-30.**~~ and §3A's table
said no ruleset generator and no nftables code existed. The table now names
what is built and what is still not.

### In §10 item 1 — the crate was the maintainer's to choose

**Settled 2026-10-04** (`.rulings/` R-248): no crate; the messages are written
by hand, as the survey recommended.

### In §10 item 2 — kernel flags were to be confirmed

**Confirmed 2026-08-22** by the audit in `ADR-024` §8.1. The superseded item
title read:

~~Kernel flags to confirm~~
