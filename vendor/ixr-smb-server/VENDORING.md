# `ixr-smb-server` — vendored and owned

* **Status:** Owned by this project. Ruled 2026-09-04 (`.rulings/` R-134).
* **Upstream:** `ixr-smb-server` 0.5.1, crates.io — itself a fork of
  `smb-server`, at <https://github.com/Bebbssos/rust-smb-server>
* **Upstream commit:** `06cc1df79e95929c680913cc2d72a0479855af4c`, from the
  `.cargo_vcs_info.json` crates.io ships inside the package. Kept, and it is the
  only thing here that can answer *what exactly was forked* — a version number
  cannot, because a version can be yanked and republished.
* **Removed from the copy:** `.cargo-ok` (a registry marker, meaningless
  outside `~/.cargo`), upstream's `.github/workflows/` (inert here and
  misleading), and upstream's `.gitignore` — **that one mattered**: it lists
  `Cargo.lock`, and this crate is now its own workspace whose lock this project
  needs tracked for a reproducible build.
* **Licence:** MIT, upstream's. `LICENSE` beside this file is upstream's own and
  is unmodified.
* **Consumer:** `cmd/rivet-smbd`, by path. There is no version to pin any more:
  this directory *is* what gets built, and `cargo update` cannot move it.

---

## 1. Why this is here

`PLAN.md` §23.5 recommended vendor-and-own and the recommendation sat unruled
for two weeks. What turned it into a ruling was arithmetic: ~~**this crate's
first real consumer has found four defects in it.**~~ **Six are now known: two
that upstream had already fixed before 0.5.1, and four fixed here — the two
signing defects that prompted the ruling, and two more found the same day by
auditing for the species of the first two.**

| Found | What | Where |
| :-- | :-- | :-- |
| ~~§23.5~~ | ~~Two remote denial-of-service bugs — client-controlled lengths reaching `Vec::reserve_exact` before validation, fatal under `panic = "abort"`~~ | ~~upstream, unfixed at 0.5.1~~ **Wrong on the second column — see below.** |
| §122 | A chained response signed over fewer bytes than the client verifies | `src/dispatch.rs`, fixed here |
| §122 | A chained request verified over bytes the server itself rewrote | `src/dispatch.rs`, fixed here |
| §23.5's class, found here 2026-09-04 | A four-byte frame header commits 16 MiB before the bytes arrive | `src/conn/reader.rs`, fixed here |
| §23.5's class, found here 2026-09-04 | A pre-authentication `NegotiateContextCount` reserves for contexts that cannot be present | `src/proto/messages/negotiate.rs`, fixed here |

**The struck row misread which upstream it meant, corrected 2026-09-04.** The
row is left readable because the mistake is instructive and because `AGENTS.md`
§7 is about exactly this — a claim of absence, here the absence of a *fix*,
asserted from a document rather than from the source.

`§23.5` says *"the fork exists because the first real consumer immediately
found two remote denial-of-service bugs"*. The fork is **this crate**;
`README.md` beside this file says so in its first line — *"a fork with fixes for
internxt-cli-rust"* — and the upstream it forked is `smb-server`, the repository
named in `[package] repository`. So the two bugs were fixed **before** 0.5.1 was
published, by the author of the thing this project vendored, and vendoring them
in was never the outstanding work.

The fix is `src/proto/messages/bounded.rs`, upstream's own file, together with
five regression tests in `create.rs`, `ioctl.rs`, `query_info.rs`, `set_info.rs`
and `write.rs` that each feed a multi-GiB length into a short packet and assert
a parse error. It bounds a declared count by *the bytes physically present in
the frame*, which is the principle the two rows added below reuse.

Verified two independent ways on 2026-09-04, per `AGENTS.md` §7, **on this
directory as it stood before hunks C and D**: `diff -r` against the crates.io
`.crate` tarball for 0.5.1, extracted fresh, reported **`Cargo.toml` and
`src/dispatch.rs` as the only files that differ**, and the same diff against the
unpacked registry source agreed. `bounded.rs` appears in neither as an addition,
so it is upstream's. §2 has what the same diff says now.

Nothing suggests six is the last, and the two that made it six were found by
auditing for the species of the two that upstream had already fixed — which is
the argument for owning it, arriving from an unexpected direction.
Upstream is young — 121 downloads, 8 stars, no commits in four months — so a
patch sent there has no timetable, and a fix that exists and is proven had
nowhere to live.

**Sending hunks A to D upstream is still worth doing** and is not an
alternative to this. Even if they land tomorrow, this project needs a copy it
controls in the meantime, which is the whole of the argument.

## 2. What is changed, and nothing else is

**The hunks in the table below, and nothing else.** Everything else in this
tree is 0.5.1 byte for byte, deliberately, so that `diff` against the published
crate shows only what this project decided. The manifest carries a
`[workspace]` table and a header comment; those are packaging, not behaviour.
Tests a hunk added are part of that hunk, and so is the one upstream test hunk
F changed the meaning of.

| Hunk | File | What |
| :-- | :-- | :-- |
| **A** | `src/dispatch.rs` | A chained response is signed over the bytes the client verifies |
| **B** | `src/dispatch.rs` | A chained request is verified as it was received |
| **C** | `src/conn/reader.rs` | A frame is allocated as it arrives, not as it is declared |
| **D** | `src/proto/messages/negotiate.rs` | A declared context count is bounded by the bytes present |
| **E** | `src/handlers/session_setup.rs` | An authenticated session requires signing |
| **F** | `src/proto/auth/ntlm.rs` | An authenticated logon requires the NTLM MIC |
| **G** | `src/handlers/negotiate.rs`, `src/handlers/ioctl.rs`, `src/conn/state.rs` | NEGOTIATE says signing is required when no session can be anonymous |

Every hunk carries a comment at the change. A and B are marked **THE FIX** and
**THE SECOND FIX**; C to G name this file as well, so a reader who arrives in
the source rather than here can tell the code is not upstream's without
diffing. E, F and G carry the maintainer's ruling that the file share requires
signing and the NTLM MIC on authenticated sessions and says plainly that
anonymous traffic is unsigned (`.rulings/` R-210, D6) — `cmd/rivet-smbd`'s
start-up warning carries that last part.

### Hunk A — a chained response is signed over the bytes the client verifies

`stitch_responses` writes `NextCommand` as the **8-byte-aligned** sub-response
length and pads the buffer to match, then signed over the **unpadded** length. A
receiver splits by `NextCommand` and verifies the whole declared span, padding
included. Whenever a non-final sub-response was not already a multiple of 8, the
two MACs covered different byte counts.

MS-SMB2 3.1.4.1 is the authority. The fix records the aligned length for every
sub-response except the last.

### Hunk B — a chained request is verified as it was received

`dispatch_compound` substitutes the previous operation's `SessionId`, `TreeId`
and `FileId` into a `RELATED_OPERATIONS` request, in place of the `0xFF…`
placeholders the client sent — and then verified the signature over the
substituted copy. The client signed the placeholders.

MS-SMB2 3.3.5.2.4 verifies the message **as received**; substitution is
processing and must come after. `dispatch_one` now takes the received bytes
alongside the working copy.

### The order they were found in is the interesting part

**A hid B.** A client verifies a response signature *before* it reads the status
code, so B's `STATUS_ACCESS_DENIED` arrived inside a response A had mis-signed
and the client never saw it. Four builds against one unchanged probe:

| Build | Result | What the failures said |
| :--- | :--- | :--- |
| pristine 0.5.1 | **4 / 14** | `Invalid data: signature verification failed` |
| **A only** | **4 / 14** | `Protocol error: STATUS_ACCESS_DENIED` — same count, different error |
| **B only** | **6 / 14** | `stat` recovers; the rest still fail on the signature |
| **A + B** | **14 / 14** | — |

The middle two rows are the argument. A-only changes the *error* without
changing the *count*, which is what a masked second defect looks like from
outside; B-only recovers exactly `stat`, whose chained response lengths are
already 8-aligned so it never needed A.

### Hunk C — a frame is allocated as it arrives, not as it is declared

`read_one_frame` read the four-byte Direct-TCP header, took the 24-bit payload
length, and did `vec![0u8; len]` before reading a single payload byte. The
field's maximum is `MAX_FRAME_PAYLOAD` — 16 MiB - 1 — so **four bytes from an
unauthenticated peer committed 16 MiB**, on a connection that had not
negotiated, authenticated or sent anything else, and `accept()` is not
rate-limited. Under the release profile's `panic = "abort"` an allocation
failure is `handle_alloc_error`, which is not catchable: it takes the daemon and
every other connected client.

This is `§23.5`'s species exactly — a client-controlled length reaching an
allocation before validation — at the one call site `bounded.rs` cannot reach,
because the frame reader is an async socket read rather than a binrw parse over
a buffer that already exists.

The fix applies the same bound rather than a second mechanism (`ADR-011`): the
payload is read 64 KiB at a time, so the server holds only what the peer has
actually sent. **It does not cap the frame** — a peer willing to send 16 MiB
still gets 16 MiB, and every legal SMB2 frame is unaffected — which is why it
carries no interop risk and needed no negotiated limit to compare against.

### Hunk D — a declared context count is bounded by the bytes present

`NegotiateContext::parse_list` did `Vec::with_capacity(count as usize)` from the
`NegotiateContextCount` of a **NEGOTIATE request** — the first message on a
connection, processed before any authentication. `count` is a `u16`, so a
~70-byte packet reserved room for 65,535 entries of 32 bytes each: about 2 MiB,
a ~30,000× amplification, and the loop then failed on the first missing byte
having already allocated it.

A `NEGOTIATE_CONTEXT` cannot be shorter than 8 bytes on the wire (`ContextType`,
`DataLength`, `Reserved`, no data), so a count above `buf.len() / 8` describes
contexts that cannot be there. `capacity_hint` applies that, and a truthful
count in a buffer large enough to hold it is unchanged.

Lower severity than C by an order of magnitude, and fixed in the same pass
because it is the same sentence in a different file.

### Hunk E — an authenticated session requires signing

`SESSION_SETUP` created every session with `signing_required = false`, and the
dispatcher refuses an unsigned request only when that field is true. So on any
session, a request with its `SMB2_FLAGS_SIGNED` bit cleared was never verified:
anyone on the path could strip the flag, alter the request, and have it served
under the authenticated user. The field is now true for every session whose
identity is not anonymous. An anonymous session stays unsigned, because it has
no session key to sign with.

### Hunk F — an authenticated logon requires the NTLM MIC

`NtlmServer::authenticate` checked the MIC only when the AUTHENTICATE message
happened to carry one, decided by the layout of that message. A logon with no
MIC was accepted, and nothing bound the flags the NEGOTIATE and CHALLENGE
messages agreed on to the key that came out. Now an authenticated logon must
set `MsvAvFlags` bit 0x2 in its NTLMv2 client challenge **and** carry a MIC
that verifies; either missing is `STATUS_LOGON_FAILURE`.

The promise is read from the client challenge, not from the message layout,
because the client challenge is covered by NTProofStr: once the proof has
verified, the flag is the client's own and cannot have been removed in transit
to make the MIC look optional. MS-NLMP asks a client to send a MIC when the
server's target information carries a timestamp, and this server's CHALLENGE
always does; it is a SHOULD, not a MUST, so a client that does not is refused
rather than accommodated. Which real clients send one is not established here
— see §5.2.

Upstream's `server_round_trip_authenticates_user` asserted that a logon with
the right password and no MIC is accepted. It now asserts the refusal, and the
accepting case lives in `an_authenticated_logon_with_a_valid_mic_is_accepted`.

### Hunk G — NEGOTIATE says signing is required when no session can be anonymous

A client decides whether its session must sign from the `SecurityMode` the
server's NEGOTIATE response sends (MS-SMB2 3.2.5.2). Upstream always sent
`SIGNING_ENABLED` alone, so after hunk E a client that signs only when asked
would log on and then have every request refused. The response now adds
`SIGNING_REQUIRED` exactly when the server has no public share — which is
every `rivet-smbd` started with `--user`, since that binary makes all its
shares one kind or the other. With a public share it stays clear: an anonymous
Linux cifs mount does not send enough NTLM material to derive matching SMB3
signing keys, and an anonymous session has nothing to sign with anyway. Hunk E
still requires signing of any authenticated session on such a server.

The value is decided per connection and kept on it, so
`FSCTL_VALIDATE_NEGOTIATE_INFO` repeats what that connection's NEGOTIATE said
even if the share configuration changes in between — a mismatch there makes a
Windows client tear the connection down.

## 3. WARNING — apply both or neither

**This is about A and B. C and D are independent of them and of each other.**

**With only B, a 1 MiB write is reported to the client as failed and the bytes
land on disk anyway.** Pristine 0.5.1 fails closed: the request is refused and
the file is left at 0 bytes. A half-applied patch is therefore *worse than
none*, because any consumer retrying on a reported failure duplicates completed
work.

If a future change touches one hunk, it touches both.

## 4. What owning it obliges

1. **Its tests run on every pass.** `tools/test-all.sh` discovers this directory
   as a nested workspace — `vendor/` used to be excluded from that walk and no
   longer is, for exactly this reason. `cargo test --manifest-path
   vendor/ixr-smb-server/Cargo.toml` is the count.
2. **It stays out of the root workspace.** `Cargo.toml`'s `exclude` keeps
   `[workspace.lints]` off foreign code, which would otherwise produce hundreds
   of warnings that say nothing about correctness and push `tools/lint-baseline`
   up by that much.
3. **Upstream is still worth watching.** A release that fixes these properly is
   a reason to re-base, not a reason to stop owning it. Record what was compared
   in `PLAN.md` when that happens.
4. ~~**The two DoS bugs from §23.5 are NOT fixed here.** They were found by
   reading and are unfixed upstream at 0.5.1. They are the obvious next thing to
   do with this directory now that there is somewhere to do it.~~
   **Struck 2026-09-04: they were fixed before 0.5.1 shipped, by the fork this
   directory *is*.** §1 has the correction and how it was checked. What that
   left genuinely open — an audit of the same species across the rest of the
   crate — was done the same day and produced hunks C and D. **That audit is
   now the thing worth repeating on a re-base**, not the two named bugs.

   What it looked at, so a later pass does not start from nothing:
   every `reserve_exact` / `reserve` / `with_capacity` in `src/`, every
   `#[br(count = …)]`, and every length taken off the wire. Two findings
   remain unfixed and are recorded rather than repaired, because both are
   `u16`-bounded and neither amplifies the way C and D did:

   * **`LockRequest.lock_count`** — a `u16` count of 24-byte `LockElement`s,
     so ~1.5 MiB, and it is **post-authentication**. `#[br(count)]`, unbounded
     by bytes present.
   * **Response bodies carry `u32` counts** — `ChangeNotifyResponse`,
     `CreateResponse`, `IoctlResponse`, `QueryDirectoryResponse`,
     `QueryInfoResponse`, `ReadResponse`, `ErrorResponse` — but the server
     writes those and never parses one, so no client input reaches them.
     **If this crate ever grows a client, that stops being true.**

## 5. Proven, and by what method

`PLAN.md` §122.3. Run as a signed `.rvt` on a Raspberry Pi 4, not in emulation:

| | |
| :--- | :--- |
| Machine | `Raspberry Pi 4 Model B Rev 1.1`, kernel `6.12.61-v8`, read from `/proc/device-tree/model` by the probe |
| Binary | static `aarch64` musl, no `INTERP` |
| Outcome | **14 of 14**, `exit 0` in 1,024 ms |
| The archive sequence | 1 MiB written to a temporary name in 244 ms, renamed into place, read back byte-identical in 248 ms, then deleted |

**Not proven: anything across a NIC.** Both sides of every run were in one
process over loopback, because `rivet-smbd` takes its whole configuration from
`argv` and the supervisor starts an application with none — `PLAN.md` §122.5.

### 5.1 Hunks C and D — what the tests establish, and what they do not

Both were added 2026-09-04 and **neither has been near a board or a real
client.** Stating that precisely, because a unit test on an allocation bound is
easy to over-read:

| Proven | How |
| :--- | :--- |
| The frame reader never reserves a declared length it has not received | `a_declared_length_is_not_allocated_before_the_bytes_arrive` asserts on `Vec::capacity` after a short read. Restoring the one-shot allocation was tried: **that one test fails, alone, saying `reserved 16777215 bytes for a payload of which only 10 were sent`**, and the other 158 still pass |
| Chunking does not change what a complete frame reads back as | three round-trip tests across a chunk boundary, on an exact chunk, and on an empty frame; plus the three pre-existing end-to-end tests, which drive real frames through `read_one_frame` |
| A truncated frame and a clean EOF still behave as before | two tests on the error and `Ok(None)` paths |
| A lying `NegotiateContextCount` is bounded by the bytes present | `a_declared_context_count_is_bounded_by_the_bytes_present`. Reverting `capacity_hint` to `count as usize` was tried: **that one test fails, alone** |

**Not proven, and what would close it:**

* **That either bug was ever exploitable in `rivet-smbd` as deployed.** Nobody
  pointed a hostile client at a board. Closing it means a probe that opens a
  socket to a running `rivet-smbd` and sends a 4-byte header claiming 16 MiB,
  N times, while `rivet get status` is watched for the daemon restarting.
* **That the 64 KiB chunk costs nothing measurable on a Pi 4.** The reasoning
  is that a 1 MiB write becomes a handful of geometric `reserve` calls. It has
  not been timed. `PLAN.md` §122.3's probe reports a 1 MiB write and read-back
  in milliseconds and is the instrument to re-run.
* **Hunk D's bound at its call site.** The test covers `capacity_hint`; that
  `parse_list` passes it `buf.len()` is proven only by reading, and by the
  negotiate round-trip and end-to-end tests continuing to pass.

### 5.2 Hunks E, F and G — what the tests establish, and what they do not

The client in these tests is **hand-composed** from MS-NLMP and MS-SMB2
(`ntlm::test_client`): it proves the server agrees with this crate's reading
of the specifications, and nothing about any real client.

| Proven | How |
| :--- | :--- |
| An unsigned request on an authenticated session is refused, and the same request signed is served | `an_authenticated_session_refuses_an_unsigned_request_and_serves_a_signed_one`, over a real loopback socket. **It failed before hunk E**: the unsigned `TREE_CONNECT` was answered `STATUS_SUCCESS` |
| A signed request altered after signing is refused | `an_altered_signed_request_is_refused` — passed before and after; it guards the check the flag used to skip |
| A logon with no MIC, or a promised MIC that is not there, is refused; a wrong MIC is refused; a right one is accepted and yields the client's session key | `an_authenticated_logon_without_a_mic_is_refused`, `a_promised_mic_that_is_not_there_is_refused`, `a_wrong_mic_is_refused`, `an_authenticated_logon_with_a_valid_mic_is_accepted`, and `a_logon_without_a_mic_is_refused_at_session_setup` over a socket. **The two refusals and the socket test failed before hunk F**, each accepting the logon |
| The `MsvAvFlags` reader stops on short, truncated and flag-less lists | `client_av_flags_reads_only_what_is_present` |
| NEGOTIATE requires signing exactly when no share is public | `negotiate_requires_signing_exactly_when_no_session_can_be_anonymous`. **It failed before hunk G**, reading `1` where `3` was wanted |
| Anonymous sessions still work unsigned | the three pre-existing `tests/` end-to-end tests, which run anonymous sessions against public shares, still pass |

**Not proven, and what would close it:**

* **That any real client still logs on.** No Windows, macOS, Samba or Linux
  cifs client has been pointed at a build carrying E to G. Windows 11 with
  `RequireSecuritySignature: True` mounted this crate before them and signs
  everything anyway; whether every client sends the MIC hunk F now demands is
  the open question, **Linux cifs above all**. Closing it means mounting a
  `rivet-smbd --user` share from each client the maintainer uses and running
  §5's probe.
* **That `SIGNING_REQUIRED` in NEGOTIATE changes nothing for a client that
  already signed.** Expected from MS-SMB2, not observed.
* **SMB 3.1.1.** The socket tests negotiate 2.1. Under 3.1.1 the signing key
  is derived after the pre-authentication hash, and hunk E reaches it through
  the same `signing_required` field, but no test drives a 3.1.1 session.
