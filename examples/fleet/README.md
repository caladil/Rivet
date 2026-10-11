# A fleet repository

**This is the README a fleet's own repository carries**, not documentation
about Rivet. Copy it into the repository you create and edit the parts that are
about your fleet; `docs/ADR-044` is the design and stays here.

---

## What this repository is

One desired-state document per machine, and **a push is what changes the
fleet.**

```
fleet/
  machines/
    rivet-01.toml       the desired state, exactly the document that machine reads
    kitchen-pi.toml
README.md               this file
```

A file under `fleet/machines/` is the same TOML the machine keeps at
`/state/config/machine.toml`. There is no second format and no translation
layer: what is in this repository is what is on the machine.

The filename is the machine's **label** — the name a person uses — and the
fleet server resolves it in the register, refusing an ambiguous one rather than
choosing.

## How a machine gets in here

**Copy its current document in, verbatim.**

```console
$ rivet get desired-state
```

prints exactly the file this repository holds. Commit that, push, and the first
cycle finds the machine already agrees: it is adopted, and nothing is written to
it. Every change after that is a diff somebody can read.

**A composed file is refused.** A fleet server that has never converged a
machine will not write a document it has not first seen the machine agree with —
adopting a machine onto a file nobody compared is the blind overwrite the whole
mechanism exists to prevent.

## What happens after a push

`rivet-fleetd` on the fleet server polls this repository. There is no hook here
and nothing in this repository runs anything.

**This repository lives on the fleet server**, as a `[repos.<name>]` its own
document declares, and `[applications.rivet-fleetd] fleet-repository` names it.
You clone it from, and push it to, that server's control plane. It is not the
`fleet` repository `cargo rivet fleet push` fills.

For each machine it finds, it reads the machine's current document over the
control plane and compares:

| | |
| :-- | :-- |
| the machine already says this | nothing happens, and nothing is said |
| the machine is exactly what was last pushed to it | the new document is written |
| **the machine has been changed by hand since** | **REFUSED**, naming every key and both values |
| this server has never converged this machine | **REFUSED** — see above |
| the machine is switched off | **not a failure.** It converges when it is reachable |

A push does not have to wait for the next cycle:

```console
$ rivet-fleetd converge --dry-run     # what it would do, touching nothing
$ rivet-fleetd converge               # do it now
```

## When a push is refused for drift

The refusal names the keys and both values. Usually the machine is right and
the repository is stale, and the fix is to put the change in here — **a value
only a machine knows is a value the next apply deletes.**

When the repository really should win, say so in the commit message of the push
that should win:

```
The kitchen pi goes back to the fleet resolver

Fleet-Override: overwrite-drift kitchen-pi
```

It belongs to that one commit and it names that one machine. It is deliberately
not a file: a file would be a standing permission that authorised every future
push as well, and the push nobody thought about is the one the refusal is
protecting.

A machine whose label contains a space is named in double quotes,
`Fleet-Override: overwrite-drift "kitchen pi"`. The refusal prints the line
already in that form, so copying it is enough.

## Never put a secret in here

A document carries a **reference** to a secret and never the secret. The schema
has no key a credential could be written under, so a file with one is refused
at the fleet server before it reaches a machine — but the refusal is a backstop
and not a reason to try.

A wireless passphrase, or anything like it, goes onto its machine with

```console
$ rivet apply install-secret <name> ...
```

and this repository never sees it.

## What this does not do

* **It is not a rollout system.** No staged rollouts, no canaries, no health
  gates, and no ordering between machines. `docs/ADR-044` §7 argues what
  ordering ought to be and does not build it.
* **It does not replace the control plane.** It produces document writes over
  it.
* **It does nothing for a machine that is off.** The document is the intent.
