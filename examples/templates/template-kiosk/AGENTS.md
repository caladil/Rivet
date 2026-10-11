# AGENTS.md

A Rivet kiosk application, started from one of Rivet's templates. It deploys
to a Rivet machine with `cargo rivet install`.

**Before the first deploy, run it once without `confirm`:**

    cargo rivet install <device> --manifest-path Cargo.toml

That changes nothing, connects to nothing and reads no key, and it ends with
**NEEDS**: the machine the real run will reach, the files it will read (two of
them hold private keys), what it will write, and what happens on the machine.
If your environment does not allow all of that, ask for exactly that list.
Then run the same command with `confirm` as its last word, and the steps it
prints after the install.

This template links system libraries, so building it needs the cross
toolchain its `.cargo/config.toml` names; the reasoning is in that file.

The procedure, and why each step is there, is the Rivet repository's
`skills/deploying-an-application/SKILL.md`.
