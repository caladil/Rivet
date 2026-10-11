# Design candidate: Rivet in a Container — a read-only audit of `rivet-sysd`

* **Status:** Audit only. NOT a proposal, NOT an ADR. Read-only survey of the
  tree as it stands, to answer one question before any design work.
* **Date:** 2026-10-02
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Builds on:** `ADR-014` (workspace/crate layout), `ADR-017` (services and
  supervision), `ADR-015` (packaging and sandboxing), `ADR-026` (static musl),
  `Shared-Libraries-and-the-Build-Target.md`

---

## 1. The question

Could a Rivet `.rvt` — a network service such as a Trends or TempMon *server* —
run in a Docker container on a PC, driven by a `docker-sysd` that takes
`rivet-sysd`'s role, so an adopter runs the server without the Windows adapter it
carries today?

The maintainer's sharper framing: *the hexagonal architecture separates the
logic from the platform, so shouldn't that already make this a composition root
rather than a fork?* This audit tests that hypothesis against the code.

**The answer is yes, with one structural caveat.** The logic↔platform seam is
clean and held; the logic was simply never packaged where a second composition
root can reach it. Details below, each verified against the tree on 2026-10-02.

---

## 2. Headline: the separation held on one axis, and not on the other

There are two different separations in play, and only one is the architecture's:

* **Logic ↔ platform (the hexagonal axis): CLEAN.** The supervisor's domain —
  discovery, applying the desired-state document, delivering settings, the
  convergence engine, process supervision, health — is written against the
  `Kernel` trait (`cmd/rivet-sysd/src/kernel.rs`) and `rivet-core` types. The
  platform wall holds: the only `libc::` outside `kernel.rs` and the Landlock
  syscall module `landlock.rs` is in **test code** (errno constants in
  `supervise.rs` tests). Networking is not here at all — `rivet-sysd` never
  speaks netlink or DHCP; that is `cmd/rivet-netd`'s, and `rivet-sysd` only reads
  a lease file another process wrote. 27 of the crate's modules take
  `&mut dyn Kernel` or `impl Kernel`, which is the domain expressed against the
  seam exactly as the architecture intends.

* **Logic ↔ packaging (not the hexagonal axis): the caveat.** `cmd/rivet-sysd`
  is a `[[bin]]` with `path = "src/main.rs"` and **no `lib.rs`, no library
  target.** Every line of the supervisor — `supervise.rs` (12,362 lines),
  `applications.rs` (3,335), `service.rs`, `definitions.rs`, `runtime.rs`, the
  rest — is reachable only from `main.rs`. **Nothing outside the binary can link
  it.** So "share the logic with `docker-sysd`" is not blocked by platform
  coupling; it is blocked by the logic living in a binary. The fix is extraction
  into a library, not a redesign.

This is why the maintainer's intuition is right *and* the caution stands: the
architecture did its job on the axis it governs. Packaging is a separate
decision that was made the other way — reasonably, since there has only ever
been one composition root to serve.

---

## 3. The `Kernel` trait, classified

The trait is ~60 methods and is the single I/O port (`ADR-009` §3.4's wall). A
container is still Linux, so most of it works **unchanged**; a `DockerKernel`
would reuse `LibcKernel` for the majority and override a clear minority. Grouped
by what a container does with each:

| Group | Methods | In a container |
| :-- | :-- | :-- |
| **Filesystem** | `read_file`/`_bytes`, `write_file`, `append_file`, `mkdir_p`, `mkdir_mode`, `chown`, `chmod`, `share_tree`, `file_exists`, `is_symlink`, `file_size`, `modified_secs`, `sync_file`/`_dir`, `rename`, `remove_file`/`_tree`, `list_dir`/`_entries`, `read_mounts` | **works** — `/state` is a mounted volume |
| **Process** | `spawn`, `run_to_completion`, `reap_ready`, `signal_process`, `observe`, `arm_signals`, `wait` | **works** — `docker-sysd` is PID 1 and reaps, which the engine already does |
| **Sockets/endpoints** | `open_journal_ingest`, `open_query`, `open_facts`, `open_health`/`ask_health`/`close_health`, `close_notify`, `close_listeners`, the `answer_*`/`drain_*` | **works** — ordinary unix sockets |
| **Time/journal** | `now_ms`, `wall_ms`, `set_wall_ms`, `local_time`, `emit` | **works** (`set_wall_ms` likely a no-op; the container clock is the host's) |
| **Confinement** | `landlock_abi` + `landlock.rs` | **best-effort** — Landlock is a kernel feature; works if the kernel supports it and seccomp does not block the syscalls. A decision (§ Questions). |
| **Resource limits** | `cgroup_hierarchy`, `place_in_cgroup`, `cgroup_kill`, `apply_memory_limit`, `apply_cpu_limit` | **needs a decision** — cgroup delegation inside a container is restricted; Docker enforces limits at the container boundary instead |
| **Machine/host** | `set_hostname` | the container's hostname is Docker's — no-op or refuse |
| **Kernel log** | `open_kmsg`, `drain_kmsg` | a container cannot read the host `/dev/kmsg` — skip |
| **Boot medium** | `grow_state` | **does not apply** — no partition; `/state` is a volume — no-op |
| **Display/kiosk** | `take_kiosk_lease`, `open_display`/`close_display`, `answer_kiosk`/`drain_kiosk` | **does not apply** — no DRM, no compositor — **refuse** |
| **CEC** | `cec_send`, `cec_poll` | no `/dev/cecN` — no-op/refuse |

So roughly four-fifths of the port is reused verbatim; the remainder is
hardware, boot, display, and host-identity, which a server in a container does
not have. **Stubs refuse or no-op; they never fake success** (the standing
rule) — a `docker-sysd` that pretended to confine or to limit memory would
report a safety it does not have.

---

## 4. The composition root interleaves two kinds of work

`main.rs`'s start-up sequence mixes supervision that any host needs with
machine-only concerns that a container does not:

* **Universal (keep):** discover applications, retire carried image services,
  apply the desired-state document, provide application homes, publish the OS
  version, deliver settings, reconcile the proxy's listen layout, converge, then
  the event loop of `wait` → events → `converge`.
* **Machine-only (drop in a container):** `grow_state` (partition grow),
  `kiosk_selection::restore` and `splash` (display), the RTC-correction clock
  path, CEC/display-hours, and — depending on the role — git-repository
  convergence for a fleet server.

A `docker-sysd` needs its **own, smaller composition root** that keeps the first
list and omits the second. That root is new code; the modules it drives are not.

---

## 5. What is genuinely not sysd's problem

Three things an adopter's Windows adapter conflates with "the server" are not
`rivet-sysd`'s and simply fall to Docker:

* **Networking.** `rivet-netd` owns netlink/DHCP/addresses. In a container,
  Docker provides the interface, the address and `/etc/resolv.conf`. The
  supervisor's `/etc` projection that fills `resolv.conf` from a lease is
  redundant here and would be skipped.
* **The control plane.** `rivet-mcpd` is a separate binary. Whether a container
  needs it is a product decision (§ Questions) — a single-service container may
  be configured declaratively and need no control plane at all.
* **A/B, the card, tryboot, provisioning, the fleet root.** None of it applies;
  a container image is replaced by `docker pull`, not by an update payload.

---

## 6. The build target

`Shared-Libraries-and-the-Build-Target.md` and `ADR-026` already establish the
musl-static direction. Two targets serve two purposes, from the same
`docker-sysd`:

* **`x86_64-unknown-linux-musl`, native** — the adopter's actual goal: a TempMon
  or Trends server on any PC at native speed. The build system already
  cross-compiles; a second target is mechanical.
* **`aarch64-unknown-linux-musl`, under emulation** — run the *bit-identical*
  artifact that ships to the Pi, in a container, for testing. Lighter than the
  QEMU harness (no full-system boot), so a faster replicate/test loop than
  anything today.

---

## 7. Findings, in one place

1. The supervisor's logic↔platform separation is clean and verified; the
   hexagonal architecture held.
2. The one structural blocker is packaging: the logic is in a binary crate with
   no library target, so a second composition root cannot link it. Extraction
   into a library (working name `rivet-supervisor`) is the enabling work.
3. The `Kernel` port is ~80% reused verbatim in a container; the minority to
   override is hardware/boot/display/host-identity, all of which refuse or no-op.
4. `docker-sysd` is a new, smaller composition root over the extracted library,
   plus a `DockerKernel` that is mostly `LibcKernel`.
5. Networking, A/B, the card, and provisioning are not in scope — Docker or
   irrelevance covers them.
6. Kiosk applications are out; this is for network services.
7. Two build targets (x86_64 native, aarch64 emulated) serve PC-hosting and
   hardware-parity testing respectively.

**The risk to manage, named for the design work:** two composition roots over
one manifest contract must never become two interpretations of a manifest
(`ADR-011`). The extraction has to keep the library the single authority, with
`docker-sysd` a strict subset that refuses what it cannot honor.

See `QUESTIONS.md` in this folder for the decisions this opens.
