# SPEC-03: The Rivet SDK — what an application may assume

* **Status:** Normative for `0.1.56` and later. Every row marked *built* was
  proven on hardware between 2026-08-31 and 2026-09-01; every row marked
  *unbuilt* says so. **§3's contract was verified against a running
  application's `/proc/<pid>/environ`** — see §3.0.
* **Date:** 2026-09-01
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Artifact kind:** `application`
* **Decides:** nothing. `ADR-036` is the decision; this is what an author must
  do about it.
* **Builds on:** `ADR-015` (packaging and sandboxing), `ADR-017` (services and
  supervision), `ADR-022` (logging and the journal), `ADR-036` (the SDK),
  `SPEC-01` §5 (what this build is and is not), `SPEC-02` (shared libraries)

---

## 1. Scope

**This is the contract between a Rivet and a program that runs on it.** It is
language-neutral by construction: everything below is an environment variable, a
file descriptor, a path, or a line of JSON, and `§7` records which languages
have a binding today.

It is **not** a description of `rivetos-sdk`. That crate is one implementation of
this document, for Rust. A Go implementation was written independently from this
same contract and is `examples/templates/template-go-service/rivet.go`; it needed
about two hundred lines. **If consuming this contract needs materially more than
that in a new language, the contract is wrong** — that is the standard this
document holds itself to.

### 1.1 What an author should read first

`SPEC-01` §5.2 — the list of what a Rivet **does not have**. A crate or module
that reaches for a shell, `$HOME`, an environment variable a manifest set, D-Bus,
`/usr/share/zoneinfo`, or `pkg-config` for the target **compiles perfectly and
fails on a board**. Nothing in this document works around any of it.

---

## 2. The first article

> **An application declares what it needs and is HANDED it. It does not go and
> take one.**

Ruled by the maintainer, 2026-09-01 (`ADR-036` §2). Every section below is an instance.

The contrapositive is the part that costs an afternoon when it is discovered
rather than read: **a program that goes and takes a resource is doing the thing
this platform arranges not to need.**

* Binding its own port — the supervisor already bound it, and an application's
  uid cannot bind below 1024 anyway (`§3.1`).
* Opening `/dev/dri/card0` by path — `EACCES`, and there is no compatibility
  path (`§3.7`).
* Guessing another application's socket path — it is told (`§3.4`).
* Choosing where to write — it is told (`§3.3`).

---

## 3. What arrives

Everything in this section is composed by the supervisor and delivered before
`execve`. **An application receives no other environment**: `HOME=/`,
`TERM=linux`, and what is below. There is no `env` key in a manifest and no way
to add one (`SPEC-01` §5.2).

**One row is handed over without being asked for**, and it is the only one:
`§3.10`'s health channel. Everything else here is either always present or the
consequence of a manifest key.

| What | How | State |
| :-- | :-- | :-- |
| listening sockets | descriptors from 3, `LISTEN_FDS`, `LISTEN_PID`, `LISTEN_FDNAMES` | **built** |
| its own installed files | `RIVET_APP_DIR` | **built** |
| somewhere to write | `RIVET_APP_DATA` | **built** |
| another application's socket | `RIVET_CONNECT` | **built** |
| its shared libraries | `LD_LIBRARY_PATH` | **built** (`SPEC-02`) |
| the journal | stdout/stderr, and `/run/rivet/journal` | **built** |
| its display | a DRM lease, over the socket in `RIVET_DISPLAY` | **built** 2026-09-01; the re-issue contract completed 2026-09-03 — `§3.7` |
| its settings | `<RIVET_APP_DATA>/settings`, rewritten every convergence | **built** — `§3.8` |
| when the screen is dark | `/run/rivet/display-hours` | **built** — `§3.9` |
| a socket to answer *how are you* on | a reserved `LISTEN_FDNAMES` entry, `rivet-health` | **built** 2026-09-09 — `§3.10`, `ADR-045` §3.1 |

### 3.0 The whole environment, read off a running application

Not an illustration. This is `/proc/<pid>/environ` of
`template-network-service` on the maintainer's board, 2026-09-01, read over the
control plane while it was serving:

```text
HOME=/
TERM=linux
RIVET_APP_DIR=/state/app/template-network-service/current
RIVET_APP_DATA=/state/app/template-network-service/data
RIVET_CONNECT=template-go-service:api=/run/rivet/listen/template-go-service/api
LISTEN_FDNAMES=tcp-8088
LISTEN_FDS=1
LISTEN_PID=229
```

**That is everything.** Eight variables, two of which are constants. There is no
`PATH`, no `USER`, no `LANG`, and no way for a manifest to add one.

Note `LISTEN_FDNAMES=tcp-8088` where the manifest said `tcp:8088` — §3.1 — and
that `LISTEN_PID` equals the pid, which is the check §3.1 requires.

**On a build from 2026-09-09 or later the same application reads
`LISTEN_FDNAMES=tcp-8088:rivet-health` and `LISTEN_FDS=2`.** The capture above
is kept as measured rather than edited, because it is evidence; §3.10 is the
change, and it is the reason the reserved entry is **appended** — every index
this application already had is still that index.

This application declared no `library`, so it received no `LD_LIBRARY_PATH` at
all: an absent variable rather than an empty one, throughout. It is not a kiosk
application either, so there is no `RIVET_DISPLAY` — §3.7. **Absent, not
empty**, is the rule for every variable here.

### 3.1 Listening sockets

`ADR-015` §5.2. The supervisor opens every socket the manifest declares, calls
`listen`, and passes the descriptors. **The application never binds and never
holds `CAP_NET_BIND_SERVICE`** — which is how an unprivileged uid serves port 80.

| Variable | Meaning |
| :-- | :-- |
| `LISTEN_FDS` | how many were passed. Descriptors are `3 .. 3+n`, in the manifest's order |
| `LISTEN_PID` | the pid they were passed to |
| `LISTEN_FDNAMES` | one name per descriptor, **colon-separated**, same order |

**`LISTEN_PID` must be checked.** It survives `exec`, so a child the application
spawns would otherwise believe it owns its parent's descriptors. If it is set and
does not equal this process's pid, the application has been handed nothing.
If it is **absent**, treat the descriptors as this process's — that is a program
run by hand, with no supervisor to claim them.

**A name is never the manifest's spelling.** `ADR-015` §3.2 writes `tcp:8080`
and `unix:api`; `LISTEN_FDNAMES` is colon-separated, so those would arrive as
*four* names rather than two. The wire forms are:

| Manifest | Name on the wire |
| :-- | :-- |
| `listen = ["tcp:8080"]` | `tcp-8080` |
| `listen = ["tcp:8443:direct"]` | `tcp-8443` |
| `provides = ["socket:api"]` | `api` |

**Where a TCP or UDP socket is bound is not the application's to know, and it
changes nothing it does.** An application that declares a `proxy-path` has its
ports bound on `127.0.0.1`, reachable only through `rivet-proxyd` and from the
machine itself; one declared `:direct` is bound on every address, and so is
every port of an application with no `proxy-path` (`ADR-046` §7.1). The
descriptor and its name are the same either way.

A `provides` entry is folded into the same listen set and passed the same way —
see `§3.4`.

Fewer names than descriptors is possible and not an error; the descriptor is the
thing that works. More names than descriptors is a supervisor defect; ignore the
extras rather than inventing sockets.

**Do not clear these variables.** `sd_listen_fds`'s `unset_environment` habit is
how a second caller concludes it was passed nothing. Read once, keep the result.

### 3.2 `RIVET_APP_DIR` — the application's own files

Where this application was installed: `<volume>/app/<name>/current`, with its
assets written flat beside the program.

**The working directory is `/`, which is not it.** An application with assets can
find them no other way. This is the case that produced a segfault on a board with
a working GPU and a live IPC connection: every font failed to open because a
relative path resolved against `/`.

### 3.3 `RIVET_APP_DATA` — the one writable directory

`/state/app/<name>/data`, mode `0700`, owned by the application's own uid
(`ADR-015` §3.2A).

**It is the only writable path an application has.** `/` is read-only, `/etc` is
a read-only projection, `/tmp` and `/run` are tmpfs that do not survive a boot.
`/state/app/<name>` is the application's *tree* — root-owned, `0755` so the
drop-to-uid can traverse it — and writing there fails.

**It survives an A/B slot change.** Measured 2026-09-01: a counter written here
read back correctly after a system update and a reboot into the other slot,
because `/state` is independent of the slots (`ADR-018` §4).

### 3.4 `RIVET_CONNECT` — another application's socket

`ADR-015` §5.3. An application declares `provides = ["socket:<name>"]`; another
declares `connects = ["<owner>:<name>"]`; the supervisor **chowns the socket
file to the consumer's uid** for as long as the consumer runs.

Format — **space-separated**, each entry `owner:name=path`:

```text
RIVET_CONNECT=template-go-service:api=/run/rivet/listen/template-go-service/api
```

Absent when nothing was granted, rather than empty: an empty variable is a value
a program can misread.

**The path is told and must never be guessed.** Aura's renderer was the
cautionary case — it hardcoded `/run/aura/kiosk.sock`, which was not where a
Rivet puts it, until it was patched to read `RIVET_CONNECT` instead
(`Aura@7fd9e26`).

**One consumer per endpoint**, because the grant is a `chown` and a `chown`
names one uid. When more than one application declares `connects` on the same
endpoint, every one of them is refused by name and none is started (`ADR-015`
§5.3), unless the provider declares the endpoint `:shared`.

**A `connects` that cannot be satisfied stops the service STARTING.** Measured
2026-09-01 (`PLAN.md` §105.7.2):

> this application declares `connects` and the connection was refused, so it was
> NOT started. Starting it anyway would produce a program failing at a socket
> nobody bound

So `connects` is a request in the sense that **the operator decides** — it is not
optional to the program that declared it. **Declare only what you need.**

### 3.5 `LD_LIBRARY_PATH` — the declared closure

Composed from the manifest's `library` entries and from nothing else — no default
path, no `/system/lib`, no other application's directory. `SPEC-02` is the whole
of it.

A statically linked program declares `library = []` and receives no
`LD_LIBRARY_PATH` at all. That is the ordinary case: `CGO_ENABLED=0` Go, and Rust
with `-sys` crates that vendor their C.

### 3.6 The journal

`ADR-022`. **Two paths, and they answer different questions.**

**(a) stdout and stderr** are captured for any process the supervisor spawned.
Zero dependencies, works before anything is initialised, carries no severity.
This is the floor and it is always available.

**(b) The ingest socket** — a unix stream socket at `/run/rivet/journal`, mode
`0666`. It takes a *structured* record and works for **any** process, including
one the supervisor never started.

The `0666` is deliberate (`ADR-022` §11.4): every application has its own uid, so
a socket only root could reach would answer the question for one account. What
prevents forgery is not the mode — the supervisor stamps `SO_PEERCRED`'s
kernel-verified `ingest_uid`, `ingest_gid` and `ingest_pid` on every record.

**Protocol.** Connect, write **one** JSON line, close. There is no reply and
nothing to read. Do not retry: a supervisor that is not listening is down or
restarting, and a program that blocked retrying would be worse than one that
printed the record. **Fall back to stderr**, which is path (a).

| Limit | Value |
| :-- | :-- |
| maximum record | 16 KiB (`INGEST_MAX_BYTES`) |
| deadline to deliver a whole record | 2 s |
| concurrent unfinished connections held | 16 |

**The record is a JSON object followed by `\n`, and it must be complete:**

```json
{"boot":"…","mono":12.345,"sev":"info","svc":"my-service","corr":"","actor":"my-service","msg":"listening","any_field":1}
```

`boot`, `mono`, `sev`, `svc`, `corr`, `actor` and `msg` are **all required**.
`sev` is one of `debug`, `info`, `warn`, `error`.

> **A line missing a column is not rejected — it becomes a `warn` record
> carrying the raw bytes.** An encoder that drifted would not fail; it would
> quietly turn every record into a complaint about itself while the log still
> looked busy. Emit every column even though five are overwritten.

**What a sender may claim.** `svc` only. The journal stamps `boot`, `mono`, `ts`,
`corr` and `actor` itself and demotes a sender's to `emitted_*` fields. A `svc`
that is not a valid service name is recorded as `svc_claimed` and the record is
attributed to `ingest`.

**Reserved field names** — using one puts the key on the line twice, which RFC
8259 lets a parser resolve either way. Drop and count them instead:
`ts`, `boot`, `mono`, `sev`, `svc`, `corr`, `actor`, `msg`, `ingest_uid`,
`ingest_gid`, `ingest_pid`, `ingest_overrode`, `ingest_unparsed`,
`ingest_truncated`, `emitted_boot`, `emitted_corr`, `emitted_mono`,
`svc_claimed`.

### 3.7 The display — `RIVET_DISPLAY`

`ADR-036` §2, §106. **Every kiosk application is a DRM lessee.**
`rivet-launcher` holds master for the life of the machine and leases the display
out; an application that opens `/dev/dri/card0` by path gets `EACCES`, and there
is no compatibility path.

**That last clause has been true since 2026-09-04 and was not before**
(`ADR-036` §3.1.1, `.rulings/` R-138). `ADR-015` §6.2's `display` class used to
expand to `/dev/fb0`, `/dev/dri/card*` and `/dev/dri/renderD*` for every
declaring application, so a lessee owned the card node and the open **succeeded**
— it became DRM master the moment the launcher's own master lapsed. What
`devices = ["display"]` grants a lessee now is the render node and nothing else:

| Granted to a lessee | Not granted to a lessee |
| :-- | :-- |
| `/dev/dri/renderD*` — no modesetting rights, and it cannot disturb whatever owns the screen | `/dev/dri/card*` and `/dev/fb0` — two doors around the lease, and the reason the clause above was untrue |

The arbiter — the one application declaring `connects = ["rivet-sysd:kiosk"]`,
which is `rivet-launcher` — still gets all three, because it cannot arbitrate a
device it cannot open. **The distinction is derived from that declaration and is
not a manifest key**; an application that could ask for the arbiter's grant could
ask for the screen.

**`RIVET_DISPLAY` names a socket, not a card.** It is a descriptor number, and
what arrives on that socket is a **DRM lease descriptor** — one at start, and
**another whenever the launcher re-issues**.

| | |
| :-- | :-- |
| the variable | a descriptor number, e.g. `RIVET_DISPLAY=4` |
| what it names | a `SOCK_SEQPACKET` unix socket, one per application |
| what arrives on it | one lease descriptor per message, by `SCM_RIGHTS` |
| when the first arrives | before or shortly after `execve`; the read blocks |
| when a later one arrives | when the launcher re-issues after revoking |
| when the socket closes | the launcher has gone; nothing more is coming |

The descriptor number follows the listening sockets: `3 .. 3+n` are those, and
this is the next.

#### Why a channel and not simply a descriptor

Because a revoked lease **cannot be resumed** — the lessee's descriptor dies
with the revocation. So answering *"no, don't exit"* to an exit confirmation
means delivering a **new** lease, and `ADR-036` §8.1 makes accepting one part of
what declaring `devices = ["display"]` means.

A variable read once at start could not deliver that. It would ship the
obligation without the means, which is why the mechanism is a socket
(`PLAN.md` §106.2).

#### What an application owes in exchange

**It must be able to tear its rendering down and build it again.** For a CPU
renderer on a dumb buffer that is unmap, remap, redraw. For a GPU renderer it is
the whole EGL/GBM context, which is the expensive half — and the reason
`ADR-036` §8.1 records a risk against raylib rather than pretending there is
none.

#### An application that was not given a screen

`RIVET_DISPLAY` is **absent** for everything that is not currently on the
display, which is most things most of the time. That is an ordinary state, not a
fault, and the SDK reports it as `NotGranted` by name rather than defaulting to
opening a card.

#### Rust

**Corrected 2026-09-03** (`PLAN.md` §123, Wave 5). The sketch this replaces used
`next()` for the re-issue too, and a frame loop cannot: `next()` blocks, so an
application that called it between frames would stop drawing in order to find
out whether it should be drawing. There was no non-blocking way to ask, and
`screen::handed()` — the one call this specification tells an application to
make — dropped the channel entirely, so in practice no application could honour
the obligation above at all.

```rust
// `handed()` returns BOTH: a screen to draw on now, and the channel the
// next one arrives on. Keeping the channel is not optional.
let handed = rivet_sdk::screen::handed()?;
let channel = handed.channel;
let mut screen = handed.screen;

loop {
    // Once a frame, and it does not block: `Ok(None)` is "not yet".
    match channel.try_next() {
        Ok(None) => {}
        // Assigning over `screen` drops the old one, which frees the revoked
        // descriptor and its buffers.
        Ok(Some(lease)) => screen = rivet_sdk::screen::take_leased(lease)?,
        // The launcher has gone. Exiting is correct; `restart` decides whether
        // this program comes back.
        Err(rivet_sdk::display::DisplayError::Closed) => return Ok(()),
        Err(why) => eprintln!("the display channel: {why}"),
    }
    screen.blit(&canvas);
}
```

`examples/templates/template-kiosk` is that loop in a program that runs.

### 3.8 Settings — a file, not a variable

`ADR-038`. An application that declares `settings` in its manifest is handed
`<RIVET_APP_DATA>/settings`: `key = value` lines, rewritten by the supervisor on
every convergence from the `[applications.<name>]` table of the configuration
document.

| | |
| :-- | :-- |
| where | `settings`, inside `RIVET_APP_DATA` — there is no variable |
| when | written before the application starts, and again on every reload |
| written even when empty | yes; a removed setting must be seen to be gone |
| who may write it | the supervisor, and the application (it owns the directory) |
| what happens if the application writes it | it costs that application its own settings until the next convergence, and nothing else — **the document is the authority** (`ADR-038` §7) |

**The supervisor writes it with `O_NOFOLLOW` and refuses a symlink, a hard link
or a FIFO** (`PLAN.md` §123, Wave 2.1). This is the one file root writes below a
directory an unprivileged uid owns, and following what it found there was a
write anywhere on the machine.

### 3.9 Display hours — `/run/rivet/display-hours`

`ADR-039`. A file, not a variable, and readable by anything: two fields on one
line — the hour the screen goes dark and the hour it comes back, in the
machine's local time — written from the `[display]` table of the document.
`rivet_sdk::schedule::Hours::parse` reads it, and the agreement test in that
crate is what keeps the writer and the reader honest.

Absent when the document sets no window, which means *always lit*.

Keep the `Channel` for the life of the process. An application that read one
descriptor and dropped it would work until the first re-issue and then never
draw again.

**Other languages have no binding for this yet** — §7.

### 3.10 The health channel — `rivet-health`, and every application has one

`ADR-045` §3.1.2. **A reserved `LISTEN_FDNAMES` entry.** The supervisor binds a
unix stream socket, calls `listen`, and passes the descriptor exactly as it
passes `tcp-8080` — **whether or not the manifest asked for anything**.

| | |
| :-- | :-- |
| name | `rivet-health`, **always last** in `LISTEN_FDNAMES` |
| the manifest declares it | **no.** `listen = ["unix:rivet-health"]` is *refused at packaging time* |
| who gets one | every **application**. Not the image's own services |
| the request | `get status\n` |
| the answer | `HEALTH=<healthy\|degraded\|down>[ <sentence>]`, one line |
| the cadence | the **supervisor's**, about every 30 s |
| the patience | the **supervisor's**, about 2 s |

**It is not a new mechanism and that is the whole design.** An application is
handed things four ways — the environment, file descriptors, stdout, the
filesystem — plus `NOTIFY_SOCKET`, which is a fifth only because it is
`sd_notify`'s protocol. A sixth would be what a fair reviewer objects to, so
this reuses the one that already exists. **The number that has to stay small is
the count of MECHANISMS, not the count of things handed.**

**Last, and never first.** Descriptors are `3 .. 3+n` in the manifest's order
(§3.1). The reserved entry is appended, so every index a manifest implies is
where it was; a build that prepended it would move all of them, silently, at
the next boot.

**Answer promptly and do not compute in the handler.** The supervisor gives up
after its patience and reports the service as not answering, which `get status`
shows as `unknown`. So the handler must read a conclusion something else keeps
current — a lock another thread holds, a database open, a network call, and a
healthy application looks wedged, intermittently.

**Say how you ARE; you cannot say how bad it is.** There is no severity here and
there is deliberately no way to send one. Whether a degraded service is worth
waking somebody is the operator's, written as `critical` in the machine's
document (`ADR-045` §4.3) — the same binary is critical on the machine that
serves the house and unimportant on a bench.

**The optional sentence is for a person.** At most 120 bytes of UTF-8,
truncated rather than refused, carried verbatim, **never branched on** by
anything downstream, and rendered as text and never as markup.

**An application that answers nothing is `unknown`**, which is different from
`healthy` and different from `down`, and is what a program run by hand also is.
Never answering is a valid way to be an application; it just means nothing
above can tell whether it is working.

**`HEALTH=` on `NOTIFY_SOCKET` still works and means *ask me now*.** It is a
hint, not a report: the state sent with it is discarded and the poll is the
truth. Use it when a conclusion should be seen before the next interval.

`rivet_sdk::health::Answering` is the Rust binding — `handed()`, then `serve()`
on a thread, or `answer_one()` inside a poll loop of your own. **Other languages
have no binding for this yet** (§7); the exchange is four lines of any of them.

### 3.11 The facts call — `/run/rivet/facts`, and what a machine says about itself

`ADR-057`. **The other direction from §3.10**: there the supervisor asks and the
application answers; here the application asks and the supervisor answers. It is
the supported way to learn what machine a program is running on.

| | |
| :-- | :-- |
| the socket | `/run/rivet/facts`, a bound **datagram**, `0666` |
| the manifest declares it | **no.** Every application may ask; there is nothing to request |
| the request | `get facts\n`, one datagram |
| the answer | one datagram of `\n`-terminated lines, ending `end` |
| the patience | the **caller's**, 2 s |
| what it costs when nobody asks | nothing at all |

**It is a datagram and not a stream, deliberately.** A stream socket's listen
backlog is what a supervisor wedges on, so an application's `connect` could
block with no bound and the supervisor would have one more queue to accept on.
Neither exists here: the call is bounded at both ends and there is no state a
caller can hold the supervisor with, which is why the socket can be open to
every application rather than chowned to one.

**Silence means the supervisor has stopped.** A supervisor that is turning
answers inside one pass of its own loop. So a call that returns nothing within
the patience is a **reading**, not an error to retry into a spinner:
`rivet_sdk::facts::NotAnswered::Silent` is that reading, and it is separate from
*there is no socket here* and from *this application could not ask*.

**What the answer carries:** the machine's name, the running image's version and
the A/B slot; the supervisor's phase; the folded machine health; the kiosk mode
as one of three words; and one row per service with its name, whether the
document enables it, the supervisor's own state word, its pid, its restart
count, its readiness, its health word and how old that health word is.

**What it deliberately does not carry**, so an author does not go looking:
uptime, storage, the SoC temperature, the network address, any journal record,
any health caption, and the supervisor's own liveness. `ADR-057` §4 is the rule
behind the list — nothing in the answer makes the supervisor open a file, walk a
directory or ask a service — and §4.4 is why another application's prose stays
on the operator's side of the line.

**Every fact carries how old it is**, as an age the supervisor computes when it
answers rather than as an instant, so no clock has to be agreed between two
processes. The three facts that are fixed for the life of the boot — name,
version, slot — carry no age, because they cannot have one.

**The answer's first line carries a protocol number.** A reader that does not
know it refuses the whole answer rather than reading a field out of the wrong
position; a *line kind* it does not know is skipped, because within one protocol
a new line can only be an addition.

`rivet_sdk::facts::ask()` is the Rust binding. **Other languages have no binding
for this yet** (§7); the exchange is a bound datagram socket, one `sendto`, one
`recvfrom` with a timeout, and a line split.

### 3.12 The supervisor's own state — `/proc`, and only for a named program

`ADR-057` §5. **The one question the supervisor may not be asked about itself.**
An instrument may not depend on the thing it measures, so a program that has to
say whether the supervisor has stopped reads `/proc` directly, in its own
process, under a grant of two leaves: the supervisor's own `/proc` directory
with read and not list, and `/proc/uptime`.

**There is no manifest key for it and there will not be one.** The programs that
get it are a list of names in the platform —
`rivet_core::confinement::SUPERVISOR_WINDOWS` — because a capability an author
can ask for is one that will be asked for. **An application that is not on that
list is refused every file, and the binding reports that as a refusal rather
than as a supervisor that has gone.**

**One sample of a counter says nothing.** The question is whether the loop is
turning and a turning loop is a *difference*: take two samples and compare them.
`rivet_sdk::supervisor::Watch` is the Rust binding — `begin()` at startup, so
the pid the supervisor had then is held, and `read()` as often as a screen
refreshes.

**It can say whether the supervisor has stopped. It cannot reliably say where.**
`wchan` is world-readable by mode and its contents are gated on
`ptrace_may_access`, so an application reading the root-owned supervisor is
handed the single character `0` — which is also what a task that is not parked
reads. `syscall` is mode `0400` root and is refused outright. The binding
reports the first as *indeterminate* and never as a symbol, because one
character carrying two answers is not a reading.

## 4. The manifest

`[package.metadata.rivet]` in `Cargo.toml` — read by `cargo rivet package`, and
for a non-Rust program the `Cargo.toml` carries the manifest and builds nothing
(`--binary` takes an already-built executable).

**The manifest IS the permission request** (`ADR-015` §3.3), and a declaration is
**two-sided**: the platform provides plumbing, the application takes on
obligations (`ADR-036` §8A).

| Key | Requests | Obliges the application to |
| :-- | :-- | :-- |
| `kind` | `daemon` \| `oneshot` \| `socket-activated` | be that shape |
| `listen` | sockets opened and passed — on `127.0.0.1` beside a `proxy-path`, unless `:direct` | read `LISTEN_FDS`; never bind |
| `provides` | a socket another application may be granted | serve it |
| `connects` | another application's socket | read `RIVET_CONNECT`; **and it will not start without it** |
| `state` | `private` → `RIVET_APP_DATA` | write nowhere else |
| `network` | `none` \| `group:<name>` \| `egress` | — |
| `memory` | a cgroup limit | — |
| `restart` | `always` \| `on-failure` \| `never` | — |
| `ready` | `none` \| `notify` \| `listening` | signal readiness if `notify` |
| `library` | the closure → `LD_LIBRARY_PATH` | name only what it links |
| `devices` | device nodes — and `["display"]` is what makes an application a **kiosk** one | accept a re-issued display descriptor (§3.7) |
| `title` | what a launcher shows on a card | — |
| `material` | the per-machine material this application needs, by KIND — `["fleet-tls"]` is `tls-cert`, `tls-key` and `tls-root` | read them out of `RIVET_APP_DATA` under exactly those names |

**`material` asks a person, not the platform** (`ADR-055` §3). The files are
about one machine and one of them is a private key, so no payload carries them:
the operator mints the machine's leaf with `rivet-provision ca server` and
installs it with `cargo rivet secrets`. What the declaration buys is that
`cargo rivet deploy` **refuses to put the application into service without
them**, before it sends anything, naming both commands — instead of a
crash-loop read out of a journal. An application declaring nothing is deployed
exactly as it always was.

### 4.1 Two are enforced; the rest are reported as unenforced

`SPEC-01` §5.3. **Do not read this table as a sandbox.**

| | |
| :-- | :-- |
| **enforced** | the **identity** — its own uid, derived from its name, never root |
| **enforced** | **launch-time verification** — signature re-checked and the program hashed before every start |
| **enforced** | the **declared closure** — `LD_LIBRARY_PATH` from it and nothing else |
| *not enforced* | `restart`, `ready`, `state`, `network`, `memory` |

There is no mount namespace, no network namespace, no Landlock and no seccomp.
**Unix permissions on its own data directory are the whole of what separates one
application from another.**

### 4.2 A manifest key is a contract with the MACHINE

`cargo rivet package` accepting a key says **nothing** about whether the image
will. Measured 2026-09-01: a payload was refused *after* its signature and every
member digest verified, because the running image had never heard of `title`
(`PLAN.md` §105.2).

**Declare only what your application's kind needs.** A network service does not
need a card, and `title` cost a deploy cycle to learn that.

### 4.3 A manifest-only change cannot be installed

`ADR-027` §3.2.2 makes a dirty-tree version a digest of the **program**. Editing
only the manifest therefore leaves the version string identical, and the install
is refused — nothing is ever unpacked over an existing version directory
(`ADR-015` §7.2).

Uninstall first, or change the program. Recorded as a defect in `PLAN.md`
§105.3.4: the manifest is in the payload and is the half the machine acts on.

---

## 5. Lifecycle

1. **Start.** The supervisor composes the environment, opens the declared
   sockets, drops to the application's uid, and `execve`s. Anything that fails
   here fails **before** the program runs — a port already in use means the
   service is not started at all, with one journal record saying so, rather than
   a half-started process.
2. **Ready.** `ready = "listening"` needs nothing from the application: the
   supervisor holds the socket and can see it answering. `ready = "notify"` is
   the systemd convention.
3. **Stop.** `SIGTERM`. Finish what is in flight and exit. `SIGHUP` is **not** a
   stop — it conventionally means *reload*.
4. **Restart.** Governed by `restart`. **A changed manifest applies at the next
   start**, not to a running process: a running program keeps the environment and
   the descriptors it was `exec`d with and cannot be given others.

---

## 6. What is known to work

`SPEC-01` §5.2 says what is absent. This is the other half, and **the rule for
entry is that it ran on the board** — not that it compiled.

| Crate / toolchain | Where it runs | What it settles |
| :-- | :-- | :-- |
| `tokio` (multi-thread) | `rivet-smbd`, `rivet-mdnsd`, `rivet-timed`, both templates | async runtimes |
| `axum` + `hyper` + `tower` | `template-network-service` | an ordinary Rust web stack |
| `rusqlite` + `bundled` | `sqlite-demo`, `template-network-service` | a `-sys` crate compiling **C from source**, WAL on `/state` |
| `rustls` + `ring` | `rivet-tls`, `rivet-mcpd` | pure-Rust TLS, and `ring`'s assembly |
| `serde`, `clap` | throughout | unremarkable, which is the point |
| Go, `CGO_ENABLED=0` | `template-go-service` | a static binary, cross-compiled **without a container** |
| C, dynamic | `libgreet` + `greeter` | a shared library through the declared closure |
| GLES on V3D | `mesa-demo` | the GPU |

### 6.1 Go specifics

* `CGO_ENABLED=0 GOOS=linux GOARCH=arm64` — static, no loader, no libc, and
  **no container needed**: a pure-Go cross-compile is two variables.
* **`-tags timetzdata` is not optional and its absence is silent.** A Rivet has
  no `/usr/share/zoneinfo`, so `time.LoadLocation` falls back to UTC on a machine
  whose owner set a timezone. Nothing fails; the clock is just wrong.

  **Unchanged by `ADR-021` §9 (2026-09-02), and the distinction matters.** A
  Rivet's document now carries a `timezone` key and the supervisor projects
  `/etc/localtime` from it, so `time.Local` — and therefore a bare
  `time.Now().Format(...)` — resolves correctly with no build tag at all. What
  still needs `timetzdata` is looking a zone up **by name**: the database lives
  at `/system/share/zoneinfo`, which is not one of Go's search paths and is not
  an application's to read. `SPEC-01` §5.2.1 is the table.
* `os.UserConfigDir()` and friends resolve through `$HOME`, which is `/` and
  read-only. Use `RIVET_APP_DATA`.

### 6.2 The asymmetry is about dependencies, not languages

Go needed no container; Rust needed one — because `rusqlite` compiles C. A Rust
program with no `-sys` crate would not need it either. Say it that way to an
author, or they will draw the wrong conclusion about the languages.

---

## 7. Language bindings

| Language | Binding | State |
| :-- | :-- | :-- |
| **Rust** | `rivetos-sdk` (library `rivet_sdk`) — in this workspace | `app`, `listen`, `journal`, `display` built; introspection not |
| **Go** | `rivet.go`, ~200 lines, **copied** into each application | works; `examples/templates/template-go-service` |
| **C / C++** | none | `PLAN.md` §105.4A defers a C++ kiosk demo, after which this row should be filled |

**`rivetos-sdk` is not the specification.** It re-declares this document's
constants rather than owning them, and `rivet-sdk/tests/agreement.rs` asserts
they still match `rivet-core` — including that a journal record it encodes parses
back identically, because a drifted path fails loudly while a drifted encoding
does not fail at all.

**Whether the Go binding becomes a module is open.** Two hundred lines copied
into every Go application is a cost this document should stop paying, and the
same question will arrive for C. Recorded rather than settled — it is
`ADR-036`'s to answer.

---

## 8. Refusals an author will meet

Every one of these is the platform working. They are collected because each was
met in practice and the message is the diagnosis.

| Refusal | Means |
| :-- | :-- |
| *this service declares listening sockets and this machine could not open them, so it was NOT started* … `Address in use` | a port is taken. **A fact about the machine, not the artifact** — and caught before the program runs |
| *this application declares `connects` and the connection was refused, so it was NOT started* | the endpoint is not provided, or the provider is not installed |
| *wants X:Y, and X is installed but its manifest does not declare `provides = ["socket:Y"]`* | the two manifests disagree about the endpoint's name |
| *the manifest cannot be read: a key is not one this build defines* | the **image** predates the key. `§4.2` |
| *… is already on this machine, and nothing is ever unpacked over a directory that is already there* | the version is unchanged. `§4.3` |
| *`RIVET_APP_DATA` is not set* (from the SDK) | not started by a Rivet supervisor. **The SDK refuses rather than defaulting**, because a silent fallback puts data where nobody is looking |

---

## 9. Conformance

An application conforms to this specification when:

1. It **binds no socket** it declared, and reads `LISTEN_FDS` — checking
   `LISTEN_PID`.
2. It **writes only** under `RIVET_APP_DATA`.
3. It **resolves its own files** through `RIVET_APP_DIR` rather than a relative
   path.
4. It **resolves a peer's socket** through `RIVET_CONNECT` rather than a literal.
5. It **reports a missing handed variable by name** rather than falling back to
   a default that appears to work.
6. It **says things to the journal** — stdout at minimum — and emits complete
   records if it uses the ingest socket.
7. It **stops on `SIGTERM`** and finishes what is in flight.
8. Its manifest **declares only what its kind needs**.
9. It links **nothing of Rivet's internals**. `rivet-core` is the operating
   system; an application that links it has stopped being an application.
10. If it draws, it takes its display from `RIVET_DISPLAY` and **never opens a
    card by path** — and it accepts a second descriptor on that channel, because
    that is what `devices = ["display"]` promises on its behalf.

`examples/templates/` are the worked answers, in Rust and in Go.
