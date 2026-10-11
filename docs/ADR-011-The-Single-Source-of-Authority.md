# ADR-011: The Single Source of Authority

* **Status:** Accepted — named by the maintainer, 2026-08-22. Latent in
  `ADR-001` through `ADR-010`; stated here once so it stops being rediscovered.
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

This ADR records no new decision. It names the one that has been made ten times
already.

Every substantive question in this design record turned out to be the same
question wearing different clothes, and it was answered the same way each time
without the pattern being visible. **The maintainer named it while ruling on
`ADR-007` section 3.6: what the project is doing, throughout, is simply being
honest about the single source of authority.** *(`.rulings/` R-027.)*

That is a better statement of Rivet's design thesis than anything written in
the ten ADRs it summarises, and it belongs at the front of the record rather
than being re-derived per decision.

---

## 2. The principle

Two parts. The first is a design constraint; the second is what makes it worth
anything.

> **1. Exactly one thing is authoritative for any given fact.**
>
> **2. The system says which one, at the moment the answer is needed.**

Part one alone is achievable and useless. The Registry has a single authority
and is opaque about it; a read-only `/etc` has a single authority and lies about
it if it discards writes in silence. **Authority without legibility is just a
different way to be confusing.**

Part two is where the mechanisms live — per-value provenance, `--explain`,
provenance headers in generated files, typed errors that name a field, refusing
an input rather than discarding it.

---

## 3. The failure mode it names

**Two things that both look authoritative.** Every piece of prior art examined
in this record fails in exactly this way:

| System | The two authorities |
| :-- | :-- |
| `/etc` | the file, the running process's loaded copy, and whichever tool last rewrote it — three, in practice |
| Windows Registry | the key, and whatever the application actually reads or caches |
| Any cached kernel verdict | the kernel, and the component's stale copy |
| A CLI that owns its own logic | the CLI, and the API added later — `ADR-006` section 2 |
| A distribution upgrade | the vendor's default and the local edit, mixed in one directory — `ADR-008` section 3 |

None of these is a bad decision made once. Each is the accumulated result of
never having said which one wins.

---

## 4. Where it was already decided

Read as a map of the record. Each row is the same decision.

| ADR | The fact | Its single authority | The honesty mechanism |
| :-- | :-- | :-- | :-- |
| `ADR-001` | what the operator sees | Rivet, never a C mechanism | mechanisms are wrapped, never exposed |
| `ADR-003` | what a binary links | the binary itself, statically | `ldd` reports no dynamic executable |
| `ADR-004` | wireless association | `wpa_supplicant`, driven over a socket | its events are translated, not cached |
| `ADR-005` §5.4 | POSIX behaviour | **nothing** — so it is absent, not approximated | an absent shell fails unambiguously |
| `ADR-006` | administrative operations | `rivet-core` | interfaces are adapters; the graph enforces it |
| `ADR-007` §3.3 | desired state / observed state | the document / the kernel — **never both for either** | observed state is never persisted |
| `ADR-007` §3.4 | an effective value | the highest layer that sets it | `--explain` names the layer |
| `ADR-007` §3.6 | generated files | the configuration; the file is a view | read-only, plus a provenance header |
| `ADR-008` §3 | image content / machine content | `/system` / `/state`, never mixed | an update replaces one and cannot touch the other |
| `ADR-009` §4 | Rivet's types | Rivet, never a dependency | a crate's types never reach the surface |
| `ADR-010` §2 | a binary's licence | what is linked, not what is aggregated | `cargo-deny` gates it in CI |
| `AGENTS.md` §5 | system state | the kernel | verdicts are read live, never cached |

Twelve rows, one decision. The consistency was not planned, which is the
strongest available argument that it is the right axis.

---

## 5. How to use it

**For a new decision.** Ask the two questions before designing anything:

1. *What fact is at stake, and what is authoritative for it?*
2. *How does a person or an agent find that out, at the moment they need to?*

A design that cannot answer the second has not finished, however clean the
first is.

**For a review.** The defect to look for is not complexity, it is **a second
authority** — a cache, a copy, a parallel format, a convenience writer, a
dependency's type on the surface. Each arrives individually reasonable, and
`ADR-006` section 1.1 is the account of what fifty years of individually
reasonable additions produce.

**For an agent working in this repository.** When two parts of the record seem
to conflict, this is the tie-breaker: prefer the reading under which exactly one
thing is authoritative and the system can say so.

---

## 6. What this ADR is not

It is not a licence to remove things for tidiness. It says *one* authority per
fact, not *one* component per system. Rivet runs `wpa_supplicant`, BusyBox,
and the Linux kernel, and each is authoritative for something —
`ADR-001` section 4.2 is explicit that this is correct rather than tolerated.

The rule is violated by a second authority for the *same* fact, never by a
large number of components each owning their own.
