# `tools/debug/` — the on-device debugger

`ADR-005` §7. Two mechanisms, both Buildroot's, both static `aarch64` musl:

| Binary | What it answers | Stripped size |
| :-- | :-- | :-- |
| `gdbserver` | *what was this Rust program doing when it stopped* | 1 010 936 bytes |
| `strace` | *which syscall did that C mechanism actually fail on* | 1 521 600 bytes |

Measured 2026-08-24 against Buildroot 2026.02.3 and GDB 15.2, by `stat` on the
produced files rather than estimated. Both go in **both** A/B slots
(`ADR-018` §4), so a card that carries them carries 4 946 KiB more than one
that does not — which is **+2 224 019 bytes, +7.7 %** on the compressed
artifact `ADR-018` §10 tracks. Two images were built and their `/system/bin`
read back with `debugfs`; `ADR-005` §7.4 has both listings.

**`ltrace` is not here and cannot be.** `package/ltrace/Config.in` carries
`depends on !BR2_STATIC_LIBS` and `depends on BR2_TOOLCHAIN_USES_UCLIBC ||
BR2_TOOLCHAIN_USES_GLIBC`, both inherited from elfutils, and Rivet is static
musl by `ADR-003` §4. kconfig will not offer the symbol. It would also have
nothing to do: `ltrace` traces calls into *dynamic* libraries and this image has
none.

---

## The script

```
./build.sh                       drive Buildroot, strip, verify, install
```

`build.sh` is the recipe. **There is no session driver here**, and that is a
gap rather than a decision: the one that existed drove a login session, which
`ADR-035` left the image without. What replaces it has to start a debugger
through the control plane, and nothing does yet.

---

## What debugging a Rivet machine actually requires

### 1. The card must carry `nu`

**This is the finding, and it was found by running the session rather than by
reading anything.**

A debugger has to be *started*, and the only thing on a Rivet machine that can
start one is the operator's login shell. The dual-login-shell ruling makes root's shell
`nu` when the image has one and `cmd/rivet` when it does not — and
`cmd/rivet` refuses to execute any program but itself, by ruling. On a card
without `nu`, this is the whole of what a debugging session looks like:

```console
$ <ask the machine to run> /system/bin/gdbserver :1234 /system/bin/rivet --version
{"error":"E_NOT_A_SHELL","subject":"/system/bin/gdbserver","message":"rivet
 cannot execute programs. It is the login shell and the only
 command it answers is `rivet` (or `/system/bin/rivet`)."}
$ echo $?
127
```

That refusal is correct and is `ADR-005` §5.1 working. What follows from it is
that **`--gdbserver` without `--nushell` produces a card whose debugger is
unreachable**.

### 2. The binary must carry symbols, and the shipped one does not

`Cargo.toml`'s `[profile.release]` sets `strip = true`. Three cases, all three
run on 2026-08-24 under `tools/qemu/run.sh`:

| The CLI was built | gdb can | Locals |
| :-- | :-- | :-- |
| `--release` — **what a card carries** | nothing. `0x22d0d0 in ?? ()`, `Function "rivet::render" not defined.` | — |
| `--release`, `strip=false`, `debug=2` | break, stop, name the source line, walk the stack | every one `<optimized out>` |
| the dev profile | all of that | **readable** |

The first case is worth refusing before anything boots, naming the line of
`Cargo.toml` that caused it. The second is a real session; the third is what
the transcript below shows.

**The middle row is the one worth deciding about.** Debug information for a
shipped binary does not have to be *on the card* — `gdb` resolves symbols on the
host, from a local copy of the same build, and the target needs only the
stripped executable. What Rivet does not have today is anywhere to keep that
unstripped copy, because `strip = true` strips the only artifact `cargo`
produces. `ADR-005` §7.5 records the options; none has been ruled.

---

## The session, 2026-08-24

Run under `tools/qemu/run.sh --net --forward-port 21234:1234`, Rivet's own
kernel, `gdbserver` 15.2 on the guest and the Bootlin toolchain's
`aarch64-linux-gdb` 15.2 on the WSL host. The CLI is the dev profile.

On the guest, started by `nu`:

```text
Process /system/bin/rivet created; pid = 76
Listening on port 1234
Remote debugging from host 10.0.2.2, port 51716
```

On the host:

```text
Breakpoint 1 at 0x26a924: file cmd/rivet/src/main.rs, line 272.

Breakpoint 1, rivet::render (value=0x7ffffff5d0, output=rivet::cli::Output::Json)
    at cmd/rivet/src/main.rs:272
272	    match output {
#0  rivet::render (value=0x7ffffff5d0, output=rivet::cli::Output::Json)
    at cmd/rivet/src/main.rs:272
#1  0x26a9e0 in rivet::produce (value=0x7ffffff5d0, output=rivet::cli::Output::Json)
    at cmd/rivet/src/main.rs:252
#2  0x26a10c in rivet::run (argv0=..., args=..., stdout_is_terminal=false, machine=...)
    at cmd/rivet/src/main.rs:224
#3  0x26a68c in rivet::main () at cmd/rivet/src/main.rs:158
value = 0x7ffffff5d0
output = rivet::cli::Output::Json
$1 = rivet::cli::Output::Json
$2 = rivet_core::value::Value::Object(alloc::vec::Vec<(&str, rivet_core::value::Value), …> {…, len: 6})
[Inferior 1 (process 76) exited normally]
```

`$1` is the variable read: the enum variant `cmd/rivet` chose between
`ADR-006` §4's two readings of one structure, named rather than shown as an
integer. `$2` is the six-field `Value` `--version` was about to render.

---

## The case for `strace`, in one paragraph

Within a minute of existing, `strace` corrected a claim this project had written
down about a vendored C mechanism's key generator. The record said the second
run was refused by an `O_CREAT | O_EXCL` on the file the caller named; the trace
showed the `O_EXCL` was real but on a `.tmp<pid>` file, with the refusal
happening one step later at the rename — so the outcome was as recorded and the
mechanism was not, and the message an operator saw named *moving* rather than
*creating*.

That is the whole case: the record was derived from reading someone else's
source, it was subtly wrong, and no amount of Rivet logging could have shown it.

---

## Two things that will cost you an hour

**You cannot `strace` a debugger.** `strace -f gdbserver …` makes the inferior's
own `ptrace(PTRACE_TRACEME)` fail:

```text
82    ptrace(PTRACE_TRACEME)            = -1 EPERM (Operation not permitted)
```

because strace is already tracing it. The `EPERM` is strace's doing, not a
Rivet one, and it looks exactly like a kernel that forbids `ptrace`. Trace
one or the other.

**Wait for the listener, never for a number of seconds.** `gdbserver` on
emulated aarch64 takes longer to start than any constant anyone picks, and the
symptom of guessing is

```text
Remote communication error.  Target disconnected: error while reading:
Connection reset by peer.
```

which reads as a broken `gdbserver` and is a `gdbserver` that has not bound yet.
Poll the guest's own `/proc/net/tcp` for `00000000:04D2 00000000:0000 0A`
instead. There is no `cat` on a Rivet, so the read is `nu`'s `open --raw`.
