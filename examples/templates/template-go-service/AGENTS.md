# AGENTS.md

A Rivet application written in Go, started from one of Rivet's templates. It
is built with Go, packaged with `cargo rivet package`, and deployed to a Rivet
machine with `cargo rivet install`.

    sh build.sh
    cargo rivet package --binary ./template-go-service

`cargo rivet package` signs the payload, so it reads the application signing
seed, a private key.

**Before the first deploy, run the install once without `confirm`:**

    cargo rivet install <device> --rvt <the .rvt package wrote>

That changes nothing, connects to nothing and reads no key, and it ends with
**NEEDS**: the machine the real run will reach, the files it will read (two of
them hold private keys), what it will write, and what happens on the machine.
If your environment does not allow all of that, ask for exactly that list.
Then run the same command with `confirm` as its last word, and the steps it
prints after the install.

The procedure, and why each step is there, is the Rivet repository's
`skills/deploying-an-application/SKILL.md`.
