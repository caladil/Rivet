# `keys/` — the public keys a card is built to trust

Everything in this directory is a **public** key: 64 hex characters and a
newline, or the 32 raw bytes. Nothing here is a secret and nothing here can sign
anything.

A Rivet image carries a trust store saying which keys the machine accepts and
what each may sign (`ADR-028` §5.5). `tools/card/build.sh` composes it from
this directory when the image is built:

| File | Label | May sign |
| :-- | :-- | :-- |
| `rivet-system.pub` | `rivet-system` | `update`, `extension`, `card` |
| `rivet-app.pub` | `rivet-app` | `application`, `library`, `apk` |

`RIVET_TRUSTED_KEY` and `RIVET_APP_KEY` name other files instead; the container
build mounts only this repository, so a key has to live inside it.

## A fresh clone has no keys here, and that is deliberate

A card trusts exactly the keys it was built with, so the keys in this directory
decide whose signed updates and applications every card built from this tree
will accept. **They have to be yours.** Until they are, the card build refuses:
a card that cannot verify an update can never be repaired over the wire, and
that is better found at build time than on a board (`ADR-028` §5.5.5).

```console
$ cargo rivet setup
```

mints a **system** seed and an **application** seed into `~/.rivet/keys`, writes
each public half beside its seed, and prints what to copy into this directory.
Copy the two `.pub` files here as `rivet-system.pub` and `rivet-app.pub`, or
under other names with the two variables above pointing at them.

**The two seeds stay separate** (`ADR-028` §5.5): the machine that signs an
application must not be able to ship an operating system, and the system key is
not entitled to sign an application.

To derive the public half of a seed you already hold:

```console
$ cargo rivet key path/to/rivet-app.seed --out keys/rivet-app.pub
```

It reads the seed and never prints it, and it prints the **key id** — the BLAKE3
digest of the public key — which is what an `.rvt` header names and what a
machine's refusal prints, so it is how a board says which key it wanted.

## What is never here

**No private seed.** A seed never enters the repository, in any commit; it stays
on the machine that signs. `.gitignore` refuses `*.key` and `*.seed` as a
backstop, not as the plan.

**No root key.** `ADR-028` §5.6 designs one and deliberately does not build it:
a root buys an offline anchor, and a root kept on the same machine as the key it
delegates to buys nothing. When it arrives, the delegation is a payload member
and changes no artifact already produced (§5.6.1).

## A board that already exists

**A board accepts the keys that were in its image's trust store, and nothing
else.** The store lives on the read-only `/system` slot, and no operation adds a
key to a running machine (`ADR-028` §5.5.4). So a seed you generate today is
useless against a board built yesterday with somebody else's keys: it will
refuse everything that seed signs, naming a key id it does not hold.

Ask a machine what it trusts with `rivet get trust`. If you hold no seed for any
of those keys, the answer is to build and write a new card against keys you do
hold.

## Replacing a key

There is no revocation list and no expiry, and both are ruled out (`ADR-028`
§5.5.4): an expired trust entry does not fail safe on a headless board — it
removes the ability to update the machine that needs updating — and a CRL is
infrastructure that fails open when nobody runs it.

**Revocation is an A/B image update**: ship an image whose trust store no longer
holds the key.
