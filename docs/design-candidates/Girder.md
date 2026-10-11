# Girder: what four Rivet applications built four times, and which half is the SDK's

* **Status:** Design candidate — proposed, not accepted
* **Date:** 2026-09-07
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Why it is in Rivet's record:** `STATE.md` §3.4a charters Girder as the
  accumulation of shared logic the maintainer's applications grew on this
  platform, and rules the razor that splits it from the SDK. This document is
  the survey that charter asks for: what is duplicated, what kind of duplication
  each instance is, and which side of the razor each falls on.
* **Method:** every claim in §2 was produced on 2026-09-07 by reading the files
  and diffing them, with comments and blank lines stripped so a wording change
  could not read as agreement. The commands are beside the numbers. Nothing in
  any repository was modified; the adopters were read as snapshots.

---

## 1. Context

### 1.1 The razor, and the test that sharpens it

`STATE.md` §3.4a rules it, and this document argues none of it:

* *What is an abstraction that Rivet OS provides you* → **the SDK.**
* *What is domain logic about how the maintainer structures programs on Rivet
  OS* → **Girder.**
* Applied **per function, and never per file.**

The operable test is `examples/templates/`. A template is what a stranger copies
to start, so it demonstrates the platform: **if a thing belongs in a template,
it belongs in the SDK.** A template that linked a preference library would teach
somebody else's taste as though it were the contract.

That test does real work below. Four of the fourteen findings in §2 are settled
by it alone, and in three of those the deciding evidence is that
`examples/templates/template-network-service/src/main.rs` **already contains the
thing** — written independently, in Rivet's own tree, by an author who was
demonstrating the platform rather than encoding a preference.

### 1.2 Why the hole exists, and `ADR-036` §2.1 named it in advance

`ignition` is barred from every Rivet composition root, and the bar is a
dependency-graph fact rather than a convention. Each of `cmd/tempmon-rivet`,
`cmd/trends-rivet` and `cmd/reliquary-rivet` lists it by name in the block of
its manifest that enumerates what is deliberately absent — *"ADR-002 §3's fifth
gate, from the other side"* in TempMon's words, and the same clause in the other
two. What each of those roots needed and could take from no library is the
subject of §2.

`ADR-036` §2.1 predicted the consequence exactly, while ruling that the SDK must
come before the next port:

> An application ported before the contract exists does not consume the
> contract — it invents one, and the next application copies it.

**This audit is the measurement of that sentence.** The three ports were carried
out against an SDK that had a journal, a handed environment and — from
`ADR-040` — a TLS builder, and against no shared answer at all for the twenty
lines on either side of each of those. So each port invented one, and the next
port copied it: §2's evidence is that the copying is literal, down to the
line breaks.

### 1.3 What was surveyed

| Adopter | Where | Shape |
| :-- | :-- | :-- |
| TempMon | `crates/tempmon-adapter-rivet`, `cmd/tempmon-rivet` | adapter crate + second composition root |
| Trends | `server/rivet/` — a second workspace | adapter crate + second composition root |
| Reliquary | `crates/reliquary-adapter-rivet`, `cmd/reliquary-rivet` | adapter crate + second composition root, **beside a Windows adapter** |
| A fourth adopter | `cmd/<adopter>-rivet` | **composition root alone; the SDK is named there** |
| Aura | `examples/aura/` — in this repository | `aura-controller` takes `serve`; `aura-renderer` takes `screen` |
| Rivet's own fixtures | `examples/templates/`, `examples/probes/` | the SDK-side control group |

**The fourth adopter, a single-binary web service, is one more than `STATE.md` §3.4a lists.** It matters out
of proportion to its size, because it is the control: it has no adapter crate,
so it shows what an application takes from the SDK when there is no shared layer
to reach for, and what that costs it is set out under row 12 below.

---

## 2. The inventory

**The kind column is the finding.** `STATE.md` §3.4a's own lesson is that a
matching filename proves nothing, so each row says which of three things was
measured:

* **code** — the same executable Rust, in two or more trees;
* **explanation** — different code re-deriving the same fact about Rivet in
  prose, which is a documentation defect wearing a duplication costume;
* **architecture** — the same shape reached independently, which is the design
  working and is a finding only in the sense that it should be left alone.

| # | The problem | Solved in | Kind | Verdict |
| :-- | :-- | :-- | :-- | :-- |
| 1 | `tracing` events onto `rivet_sdk::journal` | TempMon, Trends, Reliquary — `journal.rs` | **code**, 85 identical lines | **Girder**, Rivet layer |
| 2 | `ServiceHost` / `ServiceWork` / `Ready` / `Stopping` port vocabulary | TempMon, Reliquary — `*-core/src/ports.rs` | **code**, four identical aliases and one trait signature | **Girder**, neutral core |
| 3 | `RivetServiceHost` — the SIGTERM half of that port | TempMon, Reliquary — `service_host.rs` | **code**, 32 of 34 lines | **Girder**, Rivet layer |
| 4 | A SIGTERM future that parks rather than resolving when the handler fails | TempMon, Trends, Reliquary, **and the template** | **code + explanation**, 4 sites | **SDK** |
| 5 | Handed descriptor → non-blocking listener for an async runtime | Trends `handed.rs`, **and the template** | **code**, and an `ADR-040` §10.4 friction already | **SDK** |
| 6 | The names `tls-cert`, `tls-key`, `tls-root` | 7 independent declarations | **code**, three string constants | **SDK** |
| 7 | The `apply install-secret` paragraph a TLS failure prints | TempMon, Trends, the fourth adopter | **explanation**, near-verbatim | **SDK**, with row 6 |
| 8 | The mTLS caller-gate test harness — `Fleet`, `exchange`, `fixture` | TempMon, Trends, the fourth adopter | **code**, 171 of 184 lines | **SDK**, as test support |
| 9 | `tcp:8080` in a manifest arrives as `tcp-8080` | 5 sites, each with its own constant and its own test | **code + explanation** | **SDK** |
| 10 | An absent `RIVET_APP_DATA` is a startup refusal | TempMon, Trends, Reliquary — `handed.rs` | **explanation**; the accessor is the SDK's | **split** — accessor SDK (built), policy Girder |
| 11 | A setting that will not parse is reported and ignored | three implementations of one policy | **code**, three shapes | **Girder**, neutral core |
| 12 | A TLS accept loop: handshake completed before the request line is read | the fourth adopter's `server.rs`, Aura's `portal.rs` | **code**, two implementations | **Girder**, Rivet layer |
| 13 | `APP_NAME` stated rather than derived, with its test | 3 adapters, the template, Aura | **explanation** | **documentation** — `SPEC-03` |
| 14 | *What Ignition does that has no meaning here*, as a table | 3 adapter `lib.rs`; *why not the config crate* in 3 roots | **explanation** | **documentation** — `SPEC-03` |
| — | `network = "none"`, `library = []`, and their manifest arguments | all five | **architecture** | **leave alone** |
| — | Hexagonal layout; one crate per platform; the isolation gate | all five | **architecture** | **leave alone** |
| — | Trends' asynchronous `ServiceHost` against the other two's synchronous one | Trends | **a real divergence** | **leave alone** |
| — | The off-platform refusal text in `unsupported.rs` and its twins | TempMon, Reliquary | **architecture** | **leave alone** |
| — | The enrollment-code ceremony | Trends, Reliquary | **code + explanation** | **Girder**, neutral core — after `Phone-Pairing.md` Phase A |

### 2.1 Row 1 — `journal.rs` is verbatim duplication, and this was the undetermined signal

`STATE.md` §3.4a leaves `journal.rs` open and asks for the three to be read side
by side. They were, and the answer is unambiguous.

    for f in tempmon trends reliquary; do
      sed -n '/^use std::fmt::Write/,$p' <that repo>/journal.rs |
        grep -v -E '^\s*(//|$)' > c-$f.rs
    done
    diff c-tempmon.rs c-trends.rs      # first difference: line 87
    diff c-trends.rs c-reliquary.rs    # first difference: line 104

**Every line of executable Rust before the test module — 85 lines, comments and
blank lines removed — is identical in all three.** `install`, the
`JournalLayer`, the five-levels-onto-four `severity` mapping, `RecordVisitor`,
and all six arms of the `Visit` implementation, including the `i64::try_from`
branch that turns a `u64` past `i64::MAX` into a string rather than a wrong
number. Trends and Reliquary agree for 103 lines; they diverge only where Trends
adds a test asserting that a `Secret` reaches the journal redacted.

Every difference the diff reports in that span is a comment. TempMon's says the
workspace *warns* on `unwrap_used` where Reliquary's says it *denies* — which is
true of each, and is the sort of drift a shared crate ends.

**This is the strongest single finding in the audit and it is not close.** It is
also the one that most clearly falls to Girder rather than the SDK, and the
reason is the template: `template-network-service` uses `rivet_sdk::journal`
**directly** and links no `tracing` at all. Aura does the same. So *use
`tracing`, and bridge it* is a decision three of five applications took and two
did not — a preference, precisely encoded.

### 2.2 Rows 2 and 3 — `service_host.rs` is two-way duplication plus one genuine divergence

`STATE.md` §3.4a is right that a matching filename is not duplication, and the
reading it prompts is more useful than either "all three" or "none".

**TempMon and Reliquary are near-identical code.** Both implement a synchronous
`ServiceHost` over a `ServiceWork` closure, both hand it `let ready =
Box::new(|| {})`, and both build the same `Box::pin(async { … })` around
`tokio::signal::unix::signal`:

    diff <(tempmon service_host.rs, comments stripped) \
         <(reliquary service_host.rs, comments stripped)

reports two changes across 34 lines: the `use` line naming each project's core,
and the two lines of the off-Linux refusal string that name each project's other
binary. **32 of 34 lines are the same.**

**And the port they implement is duplicated too**, one layer further in.
`Ready`, `Stopping`, `ServiceWork` and `ServiceHost::run` are byte-identical
declarations in `tempmon-core/src/ports.rs` and `reliquary-core/src/ports.rs`.
Only the doc comments differ, and they differ by naming systemd on one side and
the Service Control Manager on the other — two readings of one contract.

**Trends is the divergence, and it is real.** Its `ServiceHost` is
`#[async_trait]`, takes an `Arc<dyn Service>` rather than a closure, and reports
`Shutdown::HostStopping` into a vocabulary the other two lack. Its adapter is
therefore not a copy of anything; it is a third answer to the same question.

**So the shape of row 2 and row 3 together is:** two projects share a port and
its Rivet implementation outright, a third has a port of its own, and all three
re-derive the same Rivet fact in prose. That fact — *the supervisor opened the
socket, so `ready = "listening"` is the whole readiness protocol, and there is
nothing to announce* — appears three times in three module headers, each with
its own comparison table against a Windows or systemd twin. **That half is a
documentation gap and `SPEC-03` §5 is where it belongs**, whatever happens to
the code.

### 2.3 Row 4 — the SIGTERM future is in a template, so it is the SDK's

Four programs install a `SIGTERM` handler, and all four reach the same
non-obvious conclusion about the failure case: **if the handler cannot be
installed, park for ever rather than resolving**, because a resolved future is
indistinguishable from a clean stop and would take a healthy service off the air
on a machine where nothing asked it to.

    grep -rl "SignalKind::terminate" <the four trees>

TempMon, Reliquary and Trends each state that argument in a comment.
`examples/templates/template-network-service`'s `stopping` states it too, adds
`tokio::signal::ctrl_c` beside it — *"both, because a template that only handled
one would teach the wrong lesson"* — and is the fourth independent derivation.

**The template settles it.** A stop signal is what a Rivet supervisor sends
(`ADR-017` §4.1); reacting to it correctly is a platform fact, not a taste. The
SDK carries no signal helper today, and a search of `rivet-sdk/src` for
`SIGTERM` returns only prose about unrelated `EINTR` handling.

### 2.4 Row 5 — the non-blocking conversion is already a logged SDK friction

`Socket::into_tcp_listener` hands back a blocking `std::net::TcpListener`;
`tokio::net::TcpListener::from_std` requires a non-blocking one and neither
checks nor refuses. `SDK-Adoption-Friction.md` §2.2 records this from the Trends
port and `ADR-040` §10.4 owns it; `ADR-040` §10.6 leaves the second, named
constructor open.

**What this audit adds is the count.** Trends' `handed::listener` makes the call
and explains it at length. The template makes the call. Reliquary's and
TempMon's `listener` functions do **not** — both hand back a blocking listener,
and both composition roots are asynchronous. That is the failure the friction
note describes, alive in two trees, invisible at one connection at a time.

**A paragraph did not exist to be missed the first time and a name cannot be
missed.** `ADR-040` §10.6's second constructor is the fix, and the evidence for
it is now two adopters rather than one.

### 2.5 Rows 6 and 7 — three strings, seven declarations, kept in step by prose

`tls-cert`, `tls-key` and `tls-root` are the names an operator types into
`rivet apply install-secret`, and every program that serves TLS on a Rivet needs
all three.

    git grep -n '"tls-root"' -- '*.rs'

Seven independent declarations, three of them outside this repository:

| Site | Declared as |
| :-- | :-- |
| `cmd/cargo-rivet/src/secrets.rs` | `CERTIFICATE`, `KEY`, `AUTHORITY` |
| `cmd/rivet-proxyd/src/main.rs` | `TLS_CERT`, `TLS_KEY`, `TLS_ROOT` |
| `examples/aura/aura-controller/src/portal.rs` | `CERTIFICATE`, `KEY`, `AUTHORITY` |
| `examples/aura/aura-controller/src/tempmon.rs` | the same three again, for the client half |
| TempMon `handed.rs` | `TLS_CERT`, `TLS_KEY`, `TLS_ROOT` |
| Trends `handed.rs` | the same |
| the fourth adopter's `cmd/<adopter>-rivet/src/tls.rs` | the same |

`rivet_sdk::serve::Tls::in_directory` takes the two names as parameters and
`requiring_a_caller_certificate_from` takes the third, so the SDK is where the
convention is consumed and it declares none of the three. Every site instead
carries a comment asserting the agreement — Aura's says *"`rivet-proxyd` calls
its own copies `tls-cert`, `tls-key` and `tls-root`, and so do `<adopter>-rivet`,
`tempmon-rivet` and `trends-rivet`"*, which is a sentence naming four other
programs in order to keep one string in step.

**That is `ADR-011`'s subject exactly**, and it is the same tell
`Phone-Pairing.md` §1.1 identifies in the pairing constants: two codebases
keeping a contract in step through prose. Here it is seven, one of them the tool
that writes the files.

**Row 7 travels with it.** When the files are missing, TempMon, Trends and
the fourth adopter each append a near-verbatim three-line `install-secret` recipe to the
SDK's own refusal, and each explains in a comment that `install-secret` exists
rather than `apply edit` because a file written as root leaves the daemon
reading `EACCES` on its own key. That paragraph is a fact about the machine. It
belongs in the refusal the SDK already produces, once.

### 2.6 Row 8 — the caller-gate test harness is copied three times

The `ADR-046` §4.2 gate is one SDK line to build. Proving it took each adopter
about 180 lines of certificate machinery, and they are the same 180 lines.

    diff <(tempmon tls.rs tests, stripped) <(trends tls.rs tests, stripped)
    # 52 differing lines across 184 + 210

By the arithmetic of that diff, **171 of TempMon's 184 stripped test lines
appear identically in Trends'**, and reading the difference shows almost all of
it to be `rustfmt` line-breaking. The fourth adopter's copy shares 144 lines with TempMon's
by the same measure. All three carry `Fleet::mint` with ECDSA P-256, no
`extendedKeyUsage`, and a name-constrained root; `der_chain`; `der_key`;
`client_config` over `rivet_sdk::serve::provider()`; `exchange`, including the
comment about `close_notify` and why the server half's verdict is discarded; and
`fixture`.

All three also carry the mutation test —
`without_the_requirement_the_same_caller_would_have_been_served` — which builds
the ungated configuration and asserts a `200`, so that the three refusals beside
it are demonstrated to be caused by the line under test. **That test is the best
thing in any of these three files and it exists in triplicate.**

**Verdict SDK, and specifically as test support rather than as production
code.** A fleet-shaped certificate authority and a one-exchange TLS harness are
facts about what `rivet-provision` mints and what `rustls` does with it. Nothing
in them is a preference. `rivet-sdk/src/serve/tests.rs` already mints
certificates the same way for the same reason, which is an eighth copy inside
the SDK itself.

### 2.7 Row 9 — the `tcp:` to `tcp-` mangling is a platform fact with five hand-written answers

A manifest declares `listen = ["tcp:8080"]`; `LISTEN_FDNAMES` is
colon-separated, so the name that arrives is `tcp-8080`. Every adopter carries a
constant for the hyphenated form, and four of the five carry a test asserting
that the constant contains no colon. Aura's `portal::LISTEN_NAME` states it and
adds *"TempMon's adapter carries the same hyphen and the same note"* — a
cross-project citation, in a comment, about a string.

`rivet_sdk::listen::socket(name)` takes the wire name and the SDK documents
`LISTEN_FDNAMES_ENV`, so the SDK owns both halves of the transformation and
exposes neither. **A function that turns a manifest `listen` entry into the name
the supervisor will hand over is the SDK's**, and it retires five constants and
four tests.

### 2.8 Rows 10 and 11 — the razor's hard case, and it does split

`STATE.md` §3.4a names `handed.rs` as the hard case and says it splits cleanly.
Reading it, that is right, and the split is finer than the file.

**The accessor is the SDK's and is already correct.** `rivet_sdk::app::data()`
returns a `Result` and `rivet_sdk::settings::all()` answers empty for a file
that is absent. Both are right for a platform library.

**The policy is the maintainer's, and all three adopters override the same one
in the same direction.** Trends' heading is *"Absent is a startup failure, and
that is not the SDK's default"*; Reliquary's heading is the same sentence,
byte for byte; TempMon's is *"Absent is not a failure, with one exception"* and
the exception is `data_dir`. Three projects, one policy: **an absent data
directory is fatal, and an absent setting is ordinary.** The reason each gives
is its own — a year of personal records, the only complete copy, two years of
history — and the rule is identical.

**Row 11 is the other half and is where the duplication is code rather than
policy.** *A setting that will not parse is reported and ignored* is `ADR-038`
§4.4, and each adopter implements it separately:

| | Function | Extra policy |
| :-- | :-- | :-- |
| TempMon | `number(values, key, default) -> u32` | whitespace-only text is `None` |
| Trends | `whole_number` over `positive(raw, default)` | **zero is refused** — a reader pool of zero answers nothing |
| Reliquary | an inline loop over two `(key, &mut slot)` pairs | — |

Three shapes, one rule, and one of the three has thought a step further than
the other two. `rivet_sdk::settings::get_parsed` exists and returns
`Option<T>` — it cannot warn, because warning is the caller's decision about a
journal it owns. **So the parse belongs to the SDK, the warn-and-default belongs
to Girder, and Trends' refusal of zero is what a shared function would give
the other two for free.**

### 2.9 Row 12, and what the fourth adopter proves by lacking a shared layer

The fourth adopter has no adapter crate: `cmd/<adopter>-rivet`
names `rivetos-sdk` directly. Two consequences are visible in its source and
both are evidence.

**It hand-writes the accept loop, and so does Aura.** `ADR-040` §3 rules that
the SDK *ships the pieces and never the loop*, so `read_head`, `read_body`,
`Bounds` and `Limit` are the SDK's and the `for stream in listener.incoming()`
around them is the application's. The fourth adopter's `server.rs` and Aura's
`portal::start` are two independent assemblies of the same five pieces, and both
reach the same non-obvious conclusion: **complete the TLS handshake explicitly
before reading a byte of HTTP**, so that an unverified caller is refused as an
unverified caller rather than reported from inside the head parser as a
malformed request. The fourth adopter states it in its module header; Aura's is the same
arrangement.

**This is Girder's, not the SDK's**, and `ADR-040` §3 is the reason: the
loop is ruled out of the SDK on the argument that an SDK owning the accept loop
could not be used by a program that already has one. That ruling stands. A
preference library is bound by nothing of the kind — it may be opinionated,
because nothing is forced to link it.

**And the fourth adopter installs `tracing_subscriber::fmt::init()` and no journal
bridge.** Its `tracing` events therefore reach the journal only as captured
stdout, which `ADR-022` §4 forwards as a `msg` string. Every structured field
that row 1's bridge would have preserved arrives as text inside a sentence.
Nobody decided that; it is what an author does when the shared thing is in three
other repositories.

### 2.10 What is left alone, and why each

A survey that lists only candidates for extraction is a sales pitch. These were
examined and should not move.

* **The manifest arguments.** `network = "none"`, `library = []`, `state =
  "private"`, `restart = "on-failure"` appear in every `[package.metadata.rivet]`
  table with a paragraph of reasoning each. The paragraphs are similar because
  the applications are similar, but each is a **declaration about that
  application** — `ADR-015` §3.3 makes the manifest the permission request, and
  a permission request is per-program by construction. Aura's `network =
  "egress"` is the proof: same table, different answer, because that program
  fetches weather.
* **The isolation gate.** Each adapter manifest carries a comment naming itself
  as the single crate that may declare `rivetos-sdk`, and naming that project's
  other gated dependencies. The *pattern* recurs; the *content* is each
  project's own crate list, and `ADR-014` §3's point is that the gate is a
  dependency-graph fact rather than a sentence. A shared crate would still sit
  behind each project's own gate and would not replace one.
* **Trends' asynchronous `ServiceHost`.** Converging it on TempMon's and
  Reliquary's synchronous port would be a change to Trends' domain crate to suit
  a library, which inverts the dependency direction the whole layout exists to
  protect.
* **The off-platform refusals.** `unsupported.rs` and the `cfg(not(target_os =
  "linux"))` arms each name that project's other binary — *elsewhere use
  `tempmon`*, *on Windows use `reliquary --console`*. The rule is shared and
  already stated (`AGENTS.md` §3: a stub refuses, never fakes success); the
  sentence is per-application and is the useful part.
* **Hexagonal layout itself.** Four projects reached the same core/adapter shape
  because the house standard says to. That is architecture, and extracting it is
  not a thing one can do.

### 2.11 One instance of what explanation-duplication costs, already realised

The three adapter manifests each carry a paragraph explaining why
`rivetos-sdk` is reached by a relative path. TempMon's paragraph is current and
records what was actually tested. Reliquary's paragraph states a reason that the
tree has since moved past, and reads as authoritative because it is written in
the same careful voice.

Neither copy is careless. **That is the point**: one fact, written three times by
three readers, drifts because there is nowhere for it to live once. It is
`AGENTS.md` §2 rule 4's subject — correct the claim, not the paragraph — and the
cheapest form of the correction is for the claim to have a single home.

---

## 3. Girder is two crates, and the evidence is in the trees

**Two: a platform-neutral core, and a Rivet-facing layer that depends on it and
on `rivetos-sdk`.** Three independent facts force the split, and each is
checkable.

### 3.1 The three facts

1. **`rivetos-sdk` does not compile on Windows.** TempMon's adapter manifest
   states it and acts on it: the dependency sits under
   `[target."cfg(target_os = \"linux\")".dependencies]`, with the reason —
   `display` and `schedule` are declared unconditionally and use `std::os::fd`,
   `recvmsg`, the `CMSG_*` macros and `localtime_r`; `bluetooth` and `listen`
   use `std::os::unix::net`; nothing in the crate is `cfg(unix)`-gated. The fourth adopter's
   manifest gates it the same way for the same stated reason. So **merely naming
   the SDK as an ordinary dependency costs a workspace its Windows build**, and
   a single Girder crate that named it could not be linked by
   `reliquary-adapter-windows` or by anything in Trends' `server/` workspace.

2. **Reliquary is dual-platform in one workspace.** `reliquary-adapter-windows`
   sits beside `reliquary-adapter-rivet`, both implementing the *same*
   `ServiceHost` trait from `reliquary-core`. Row 2 of §2 is a port that both
   platforms need; row 3 is an implementation only one of them can compile.
   **A single crate cannot hold both.**

3. **Trends' `server/` workspace must resolve on a machine with no Rivet.** Its
   root manifest excludes `rivet/`, and its stated reason is that the exclusion
   is what keeps the workspace resolvable
   `on a machine that has never heard of Rivet`. Cargo resolves every member
   before it builds any, so a member with an unresolvable path dependency takes
   `cargo metadata` down for the whole workspace — including the
   `cargo test -p trends-core` that is that project's strongest portability
   gate, which would then report a portability break in somebody else's
   repository. **Anything Girder offers `trends-core` or `trends-api` must
   therefore link no Rivet crate at all.**

### 3.2 What goes in each

| | `girder` — neutral core | `girder-rivet` — the Rivet layer |
| :-- | :-- | :-- |
| **Depends on** | nothing platform-shaped; compiles on Windows and on a machine with no Rivet checkout | `girder`, `rivetos-sdk`, `tokio`, `tracing`; Linux-only, like every adapter that names the SDK |
| **Holds** | the `ServiceHost` / `ServiceWork` / `Ready` / `Stopping` port vocabulary and its error type (§2.2, row 2) | `RivetServiceHost` (row 3) |
| | the setting-value policy as pure functions over `&BTreeMap<String, String>` and `Option<&str>` — parse, trim, refuse zero, report the refusal (row 11) | the `tracing` → `rivet_sdk::journal` layer (row 1) |
| | the enrollment-code protocol, when `Phone-Pairing.md` Phase C reaches it | the fatal-on-absent data-directory policy over the SDK's accessor (row 10) |
| | | a TLS accept loop over `read_head` / `read_body` / `Bounds` / `Limit` (row 12) |
| **Second consumer, today** | `reliquary-adapter-windows`; `trends-core`'s workspace; `cmd/tempmon` | every `cmd/*-rivet` |

**The neutral core is small on purpose**, and its smallness is what makes it
linkable. `ADR-013` §2.1's placement test is the discipline: a thing in the
neutral core must be something a project would want *whether or not* it ran on
Rivet. Rows 2 and 11 pass that; row 1 does not, because a `Journal` is Rivet's.

### 3.3 The named-crate question this raised — RULED 2026-09-07

`ADR-013` decided this problem once already, ruled a three-home split and a
placement test, and named the platform crate **`rebar`** — *"the library that
software built for Rivet links against in order to be a well-behaved citizen of
a Rivet host"*, to receive configuration, emit structured logs, be supervised
and expose its own operations. `ADR-013` §7 item 1 ruled it emerges at Stage 3,
on the reasoning that there was no second consumer to shape it.

**`rivetos-sdk` has since been built and does that crate's job**, and `ADR-036`
named neither `rebar` nor `ADR-013` when it built it. **The maintainer retired
the name on being shown this**, 2026-09-07: Girder is the better name, and what
`ADR-013` ruled is to be redistributed to whichever of the SDK and Girder each
part belongs to.

**`rebar` was never anything but a name in a document.** It appears in no
tracked path and in no `.rs` or manifest at any commit — checked both ways, by
`git log --name-only -- "*rebar*"` and by `git log -S rebar -- "*.rs" "*.toml"`
— so retiring it costs nothing and moves no code. **It is retired, not live:
anything a reader meets that proposes it is superseded by this section.**

Both consequences this document listed have been taken, in `ADR-013` itself:

* **`ADR-013` §6 is the reconciliation.** `rivetos-sdk` is the platform crate;
  §6.2 and §6.3 — the naming argument and the `ferris` withdrawal — are retired
  with tombstones, and `ADR-013`'s `## History` says why a naming argument for a
  crate that exists under another name settles nothing. **What survived is
  §6.1's scope constraint and §6.4's language-neutrality line**, and §6.4's
  operations row is the one part of the app-side contract that still has no
  crate behind it.
* **`ADR-013` §2.1's placement test has the fourth row**, with the template test
  and the several-adopters-overriding-one-default test written under it as the
  discipline that keeps *the maintainer's own programs* from stretching.

**And `ADR-013` §7 item 1's tripwire was re-read against the tree, which is
where this section's own recommendation led.** It has fired, and not for the
SDK: `cmd/rivet-initd`, `cmd/rivet-sysd` and `cmd/rivet-timed` each declare a
`Kernel` port with an identical `Errno`, and the doc comments in those files
have been accumulating the evidence for it. Those are Rivet's own boot
binaries, which no application links, so the duplication is Rivet's to answer
and belongs to neither the SDK nor Girder. **Nothing in §2's inventory touches
it**, and it is named here so the next reader does not mistake it for a Girder
row.

---

## 4. Which findings are SDK gaps rather than preferences

`STATE.md` §3.4a's sharpest instruction is that three adopters overriding one
SDK default is evidence about the **SDK**, and that telling that apart from a
preference is what the audit is for. Six findings are gaps, and each was
identified by one of two tests.

**Test one: is it in a template?** Rows 4, 5 and 9 are.
`examples/templates/template-network-service` installs a SIGTERM handler with
the parking argument, converts a handed descriptor to a non-blocking listener,
and the whole example set carries the hyphenated listen name. A stranger copying
that template is copying the platform, so those three are the SDK's by
`STATE.md` §3.4a's own test.

**Test two: does the SDK already own both halves of the fact and expose
neither?** Rows 6, 7 and 8 are.

* The SDK's `serve::Tls` consumes the three secret names and `cargo-rivet`'s
  `secrets` verb writes them, so Rivet owns the convention at both ends and
  seven programs each declare it.
* The SDK produces the refusal an unreadable certificate earns, and three
  programs append the same recipe to it.
* The SDK mints fleet-shaped certificates in its own tests and three adopters
  mint them again.

**Feature-gating is available and is the answer to the obvious objection.**
`rivetos-sdk` already ships `draw`, `screen`, `serve` and `bluetooth` on a stated
principle — *a network service has no business linking `libdrm`*, and a kiosk
application has none linking `rustls`. So "broader SDK" is not a single lever.
Rows 6, 7 and 8 sit behind `serve`, which the applications that want them
already enable; row 8 is dev-only and links into no consumer at all. Rows 4, 5
and 9 add no dependency to anything.

**Two findings are the shape of `handed.rs` and are not gaps.** Row 10's
accessor is the SDK's and is correct as it stands: `app::data()` returning a
`Result` is the right default for a library that cannot know whether an absent
directory is fatal for its caller. What three adopters override is the
*consequence*, and three personal-data and history-keeping services agreeing that an
absent directory is fatal is the maintainer's taste about his own programs — the
same SDK serving a program that could reasonably continue would want the
`Result`. Row 11 is the same shape: `settings::get_parsed` is right to answer
`Option`, because warning is a decision about a journal the caller owns.

**And one finding is a documentation gap with no API behind it.** The readiness
argument of §2.2 — *the supervisor opened the socket, so there is nothing to
announce* — is re-derived in three module headers because `SPEC-03` §5 states
the lifecycle without stating the contrast that every author arriving from
systemd or the Service Control Manager needs. Rows 13 and 14 are the same:
`APP_NAME` stated rather than derived, and the table of what Ignition does that
has no meaning on a Rivet. Those are three paragraphs in `SPEC-03`, not three
lines of Rust.

---

## 5. The authentication surface

Four mechanisms are alive across the adopters, and they are usually described as
three. Distinguishing the fourth is what makes the question answerable.

| | Mechanism | Who it identifies | Where it is decided |
| :-- | :-- | :-- | :-- |
| 1 | Operator mTLS at `rivet-proxyd` | a **person** | `ADR-046` §4.1 — the machine's gate, in front of every route |
| 2 | The backend's caller gate | the **proxy** | `ADR-046` §4.2 — one SDK line in every application |
| 3 | `proxy-auth = "none"` plus a bearer `write-token` | **nobody**, then a **holder of a secret** | TempMon's manifest and its `ADR-038` setting |
| 4 | Enrollment code → device token | a **device that cannot hold a fleet certificate** | Trends and Reliquary; `Phone-Pairing.md` |

### 5.1 What decides which, and it is one question

**Can the caller be issued a certificate by the fleet authority?**

* **The proxy can, and always has one.** So mechanism 2 is universal and
  unconditional. Every application behind the gate requires a caller certificate
  chaining to the fleet root, and no application checks *which* — there is one
  operator and `ADR-046` §4.4 does not forward an identity, so a backend that
  pinned the proxy's leaf would break on the first reissue while asserting
  something the design does not promise. The fourth adopter's `tls.rs` and Trends' both say
  so in as many words.
* **An operator can, and mechanism 1 is where it is checked.** `ADR-046` §3 is
  the reason it can be checked in exactly one place: path routing means reading
  the request line, reading the request line means terminating TLS, and whoever
  terminates TLS is who the client certificate was presented to. **An
  application must not attempt this**, and every adopter that wrote the sentence
  wrote it to explain why it had removed something — Aura's portal retired a
  shared password and its HMAC cookie to `ADR-046` §4.2 rather than wrapping TLS
  around them.
* **A phone cannot.** That is the whole of mechanism 4's justification, and
  `Phone-Pairing.md` is its plan. Nothing about it generalises to a caller that
  *could* hold a certificate.
* **Mechanism 3 is not a third scheme; it is what `proxy-auth = "none"` costs.**
  TempMon's route is reachable without an operator certificate by ruling, so the
  dashboard stays visible on the LAN. That was sound while every route was
  read-only, and a whole-database restore reachable the same way is a different
  proposition — so `write-token` closes the write half without narrowing the
  read half. **The token is the second gate a relaxed route owes**, and it
  follows the relaxation rather than standing beside it.

### 5.2 What a new adopter should get

**Mechanisms 1 and 2, and nothing else, unless it can name a caller that cannot
hold a fleet certificate.**

The fourth adopter is the worked example and it is the model to copy: `settings = []`,
`proxy-auth` absent so the default `operator` applies, `proxy-path = "<adopter>"`,
and a `tls.rs` whose whole policy is `requiring_a_caller_certificate_from`. That
is an authenticated administrative surface on a LAN with no token, no session,
no cookie and no login page.

Two rules follow, and both are worth writing into `SPEC-03` §8 where an author
will meet them:

1. **`proxy-auth = "none"` is the one manifest key whose absence is the safe
   reading** (`ADR-046` §4.3.1 — it only ever relaxes). An application that
   writes it owes a second gate on every route that changes state, and TempMon's
   `write-token` is what that looks like.
2. **`requiring_a_caller_certificate_from`, never
   `asking_for_a_caller_certificate_from`.** The two differ by one word and by
   whether the server serves a caller that declines. Three adopters wrote a
   comment about it and three wrote the mutation test described under row 8 to prove the
   difference is real. That test is the argument for row 8's verdict: the
   substitution *looks right in review*, and the only thing that catches it is a
   harness nobody should have to write four times.

---

## 6. Order of extraction

**The SDK rows go first, in this repository, against Aura and the templates.
Girder's first content follows, led by TempMon.** The reasoning is below,
and it differs from `STATE.md` §3.4a's recommendation in one specific way that
§8 states plainly.

| Step | What | Where it is proved | Why here |
| :-- | :-- | :-- | :-- |
| **1** | Rows 6, 7, 9 — the secret names, the `install-secret` refusal, the listen-name transform | Aura, both halves, plus `rivet-proxyd` and `cargo-rivet` | Additive, retires seven declarations and four tests, and every consumer is in this repository |
| **2** | Row 8 — the caller-gate harness as SDK test support | `rivet-sdk/src/serve/tests.rs`, which already mints the certificates | Dev-only, links into no consumer, and it is what lets steps 4–6 be verified rather than reviewed |
| **3** | Rows 4, 5 — the stop signal and the async-ready listener | `template-network-service` adopts them, which is the proof they were the platform's | `ADR-040` §10.6 already has the second half open; the template is the acceptance test |
| **4** | `girder` — the neutral core: rows 2 and 11 | **TempMon**, with `cmd/tempmon` (the Windows and general-Linux root) as the neutral-side consumer | The only step that touches a domain crate, and TempMon's is the one whose port already matches Reliquary's |
| **5** | `girder-rivet` — rows 1, 3, 10, and then row 12 | **TempMon**, then **Reliquary** | Row 3 is 32 identical lines away from Reliquary's; row 1 is 85 |
| **6** | Trends | last | Its port diverges (§2.2) and it holds live personal data |

### 6.1 Aura is right for step 1 and wrong for step 4, and the difference is measurable

`STATE.md` §3.4a nominates Aura to convert first: in-repo, no personal data, and
the one adopter taking both SDK surfaces. **Two of those three reasons hold and
the third does not, for the Girder half.**

`examples/aura/aura-controller` and `aura-renderer` carry **no `handed.rs`, no
`journal.rs`, no `service_host.rs`, and no adapter crate at all.** Aura uses
`rivet_sdk::journal::Journal` directly at fifteen call sites, links no `tracing`,
and has no `ServiceHost` port to implement. So of the five findings destined for
Girder, Aura exercises exactly one — row 12's accept loop.

What Aura *does* hold is the SDK half, twice over: the three secret names in two
separate modules, the hyphenated listen name with a comment citing TempMon, a
`serve::Tls` gate, and a TLS accept loop assembled from the SDK's pieces. **It
is the best first target in the repository for steps 1 to 3 and a poor one for
step 4**, and the in-repo, no-personal-data, both-surfaces argument is exactly
right about which steps those are.

### 6.2 Why TempMon leads the Girder half

* **Its adapter is Reliquary's twin**, measured: 32 of 34 lines in
  `service_host.rs`, 85 in `journal.rs`. Whatever shape works for TempMon works
  for Reliquary with a `use` line and a sentence changed, so step 5 is one
  design and two applications of it.
* **It holds temperatures**, which is the smallest blast radius of the three
  out-of-repo adopters.
* **It has a regression check nothing else has.** `STATE.md` §0.2 records a
  proven restore — 3,177 readings out and back in, against a board. A
  restructuring of TempMon's adapter that survives that has been tested against
  something.
* **It is on `D:` and it is deployed**, so the container build loop that
  `STATE.md` §2 documents applies to it without qualification.

---

## 7. What this proposes NOT to do, and the risks

### 7.1 Five things to refuse

1. **Do not restructure Trends or Reliquary unattended.** `STATE.md` §3.4
   already rules this for the pairing work and the reason is the same here: both
   hold live personal data, and a shared-crate extraction touches the
   composition root, the journal and the startup refusal — every path a failure
   would travel on. Step 6 is last for that reason and needs the maintainer's
   go-ahead rather than his keyboard.
2. **Do not extract `handed.rs` as a file.** Most of it is per-application: the
   setting names, the port, the database filename, and — importantly — the
   refusal text an operator reads. `STATE.md` §3.4a's *per function, not per
   file* is the whole instruction, and this file is where it bites hardest. What
   moves out of it is one policy (row 10) and one parser (row 11); what stays is
   everything an operator would recognise as belonging to that application.
3. **Do not put the accept loop in the SDK.** `ADR-040` §3 rules that the SDK
   ships the pieces and never the loop, on the argument that an SDK owning the
   loop could not be used by a program that already has one. Row 12 is
   Girder's precisely because a preference library is allowed to be
   opinionated: an application that disagrees links the SDK's pieces directly,
   as the fourth adopter and Aura do today.
4. **Do not let a template link Girder.** This is the razor's own test read
   backwards, and it is also the guard that keeps the split honest afterwards:
   if a future finding cannot be demonstrated without a template linking the
   preference library, the finding is the SDK's.
5. **Do not extract the enrollment protocol before `Phone-Pairing.md` Phase A.**
   That document's first phase writes the protocol down as conformance vectors
   in both repositories, from the implementations that exist, precisely so that
   a shared library rests on a proven agreement rather than an assumed one.
   That sequencing governs here.

### 7.2 Four risks, stated before anyone is committed

* **A shared crate must serve four different lint postures.** TempMon's
  workspace warns on `unwrap_used`; Reliquary's denies it. `AGENTS.md` §10.5
  records what a denied lint does to a test module in this repository. A shared
  crate has to satisfy the strictest consumer, and it should be written that way
  from the first commit rather than tightened later.
* **Girder inherits whatever arrangement reaches `rivetos-sdk`.** The Rivet
  layer names the SDK, so every consumer of it acquires that edge. `STATE.md`
  §0.1 states what that arrangement is and that it is settled; the point here is
  only that a second crate multiplies it rather than replacing it, and that the
  neutral core deliberately does not.
* **The measurement is destroyed by using it early.** `ADR-036` §2.1's argument
  is that a port ported *after* the contract is an instrument, and one ported
  before invents a contract the next port copies. This audit is the readout of
  that instrument for three ports. Extracting Girder before the SDK rows
  land would encode today's accidents one layer up, and the next adopter would
  copy them from a library instead of from a sibling — which is worse, because a
  library reads as a decision.
* **`STATE.md` §3.4a places this after the fleet server migration**, and nothing here argues with
  that. Steps 1 to 3 are additive changes inside this repository and are the only
  part that could reasonably be taken opportunistically; steps 4 to 6 are a
  project.

---

## 8. Corrections this audit owes `STATE.md` §3.4a

Listed rather than made, because a design candidate's changes belong with the
ruling. None of them overturns the charter; all four sharpen it.

1. **There are four out-of-repo adopters, not three.**
   `cmd/<adopter>-rivet` names `rivetos-sdk` directly with no adapter crate. It is
   the most informative of the four for the SDK half, because it shows what an
   author takes when there is no shared layer within reach: it duplicates the
   TLS harness and the secret names, and it installs no journal bridge at all.
2. **The byte-identical sentence is in a different pair than recorded.**
   `handed.rs`'s opening sentence — *"What the supervisor passed, given names"* —
   is identical in all three. The sentence after it is identical in **TempMon and
   Trends** and differs in Reliquary by two characters. What is identical in
   **Trends and Reliquary** is the second heading, *"Absent is a startup failure,
   and that is not the SDK's default"*. The claim is true in substance and the
   pair it names is not the pair the bytes name — which is worth correcting
   because the pairing is the evidence for who copied from whom.
3. **`tls.rs` is four files, not two, and they are in different kinds of
   crate.** TempMon's and Trends' are in the adapter; the fourth adopter's is in the
   composition root; Aura's equivalent is inline in `portal.rs` and again in
   `tempmon.rs`. The duplication that matters in them is the **test harness**
   as set out under row 8, not the three-line builder.
4. **Aura converts first for the SDK half and last-but-one for the Girder
   half** (§6.1). It carries none of the three files §3.4a names. The
   in-repo, no-personal-data, both-surfaces argument is right; what it selects is
   steps 1 to 3.

And one correction owed to `ADR-013` rather than to `STATE.md`: §3.3 above.
**It was made on 2026-09-07 and is no longer owed.** `ADR-013` §6 now names
`rivetos-sdk` as the platform crate it called for, §2.1's placement test carries
the fourth row and the discipline under it, and `ADR-036` §2.2 states which of
`ADR-013`'s rulings bind the SDK. The name `rebar` is retired — §3.3 records
that it never existed outside the document.

---

## 9. What was NOT measured, named rather than implied

* **Nothing was compiled, and nothing was run.** Every finding in §2 is a
  reading of source and a diff. `AGENTS.md` §6 is explicit about what that is
  worth: these are claims about text, and a claim that an extracted crate
  behaves as the four copies do is one only a test suite in each adopter can
  settle.
* **Reliquary and Trends were read as snapshots** and their test suites were not
  run. A second agent was writing pairing conformance files into both while this
  survey was taken; anything under that subject was left alone and is outside
  §2's inventory.
* **`examples/probes/` was enumerated and not read line by line.** The templates
  were, because §1.1's test turns on them; the probes are single-question
  fixtures and would change no verdict.
* **No line count is a proposal.** *85 lines* and *32 of 34* say how much text
  agrees, not how much a shared crate would be. An extraction with a trait, a
  builder and a refusal type may be larger than any one copy, and that is
  ordinary rather than a defect.
* **The Go adopter was not surveyed.** `examples/templates/template-go-service`
  exists and `SPEC-03` §7 covers language bindings; a Rust preference library
  reaches none of it, and whether the SDK rows of §4 need a language-neutral
  statement is a separate question this document leaves open.
