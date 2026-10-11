# SPEC-06: The Fleet Server's Gantry Socket

* **Status:** Normative, and built: `cmd/rivet-fleetd/src/gantry.rs` is the
  provider, and `tools/qemu/gantry-probe` is a minimal consumer.
* **Date:** 2026-09-27
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Decided by:** `ADR-061` §6 (`.rulings/` R-222, R-223)
* **Provider:** `rivet-fleetd`, endpoint `gantry`
* **Consumer:** the `gantry` application, and no other
* **Protocol version:** 1

---

## 1. Overview

`rivet-fleetd` owns the fleet server's register and its repository store.
`gantry` needs to read the `apk` artifacts in that store and to read and change
the register's phone records. This document is the wire format between the two:
precise enough to implement either end and to test each against bytes the
other wrote.

The transport is a Unix stream socket that the supervisor creates and grants
(`ADR-015`). Nothing here is reachable from a network.

---

## 2. Admission

1. `rivet-fleetd` declares `provides = ["socket:gantry"]`; `gantry` declares
   `connects = ["rivet-fleetd:gantry"]`. The endpoint is single-consumer.
2. On every accepted connection `rivet-fleetd` reads `SO_PEERCRED` and compares
   the uid with `rivet_core::service::app_identity("gantry")`.
3. **A connection from any other uid** gets exactly one reply line,
   `{"ok":false,"error":"refused","message":"…"}`, and is closed. Its request is
   not read.
4. A refusal is journalled at `warn` with the peer's uid and pid.

---

## 3. Framing

1. **A request is one line**: a UTF-8 JSON object terminated by `\n`, at most
   4096 bytes including the terminator. A longer line is answered `too-long`,
   and the connection is closed, because the reader cannot tell where the next
   request would begin.
2. **A reply is one line**: a UTF-8 JSON object terminated by `\n`.
3. **Except `member`**, whose successful reply is one line
   `{"ok":true,"length":N}` followed by exactly `N` raw bytes and nothing else.
   A member is sent as bytes rather than base64 because an APK is megabytes.
4. **A connection carries any number of requests, one at a time.** A client
   sends a request, reads its whole reply including any raw bytes, then may send
   the next. Nothing is pipelined.
5. Either side may close between requests. `rivet-fleetd` closes a connection
   that has sent nothing for 60 seconds, and one whose reply it could not
   write for 60 seconds, so a client that stops reading a member does not hold
   a thread for ever.
6. **Every request carries `"v":1`.** A request with another value, or none, is
   answered `version` naming the versions this provider speaks.

---

## 4. Replies

A success is `{"ok":true, …}` with the fields each operation names.

A failure is `{"ok":false,"error":CODE,"message":TEXT}`. `CODE` is one of the
tokens below and is what a program branches on. `TEXT` is a sentence for a
person and is never parsed.

| code | meaning |
| :-- | :-- |
| `refused` | the caller is not `gantry` (§2) |
| `too-long` | the request line exceeded 4096 bytes |
| `malformed` | the line is not a JSON object, or a field has the wrong type |
| `version` | `v` is absent or not spoken |
| `unknown-op` | `op` names no operation |
| `unknown-artifact` | no held `apk` has that digest |
| `unknown-member` | that artifact has no member of that name |
| `unknown-phone` | no phone has that identity |
| `bad-key` | the public key is not a DER SubjectPublicKeyInfo of an EC P-256 key |
| `bad-label` | the label is empty, longer than 64 characters, or holds a control character |
| `exists` | a phone with that key is already enrolled |
| `store` | the store or register could not be read or written; `message` names the path |

After any failure except `refused` and `too-long`, the connection stays usable.

**Which failure is reported when several apply**, so both ends agree:

* `v` is checked before `op`: a `v` that is not the integer `1` — `"1"` and
  `1.5` included — is `version`. An absent or non-string `op` is `malformed`.
* A request with a duplicated key is `malformed`. An extra field an operation
  does not name is ignored, except `identity` in an `enrol`, which is
  `malformed`.
* `member`: `unknown-artifact` before `unknown-member`.
* `enrol`: `bad-key` before `bad-label` before `exists`.
* `rename`: `unknown-phone` before `bad-label`. An identity that is not even
  well-formed is `unknown-phone`, since no phone can have it.

---

## 5. Operations

Every request is `{"v":1,"op":NAME, …}`.

### 5.1 `fleet`

Request: `{"v":1,"op":"fleet"}`

Reply: `{"ok":true,"name":FLEET_NAME}`, the name recorded in the register's
`register.toml`, or `null` when it records none. It reads `register.toml`
alone, so an unreadable device record does not fail it.

### 5.2 `artifacts`

Request: `{"v":1,"op":"artifacts"}`

Reply: `{"ok":true,"artifacts":[{"digest":HEX64,"header":BASE64}, …]}`

* One entry per object in the store whose header's `kind` is `apk`.
* `digest` is the lowercase hex BLAKE3 of the whole `.rvt`, the name the store
  keys it by.
* `header` is the standard base64, padded, of the artifact's 4096-byte header,
  exactly as stored. Gantry parses it with the `ADR-029` layout and verifies it
  itself; this socket asserts only that the store admitted the object.
* Sorted by `digest`. An empty store is an empty array.

### 5.3 `member`

Request: `{"v":1,"op":"member","digest":HEX64,"name":MEMBER}`

`MEMBER` is one of `app.apk`, `apk.json`, `icon.png`.

Reply: `{"ok":true,"length":N}\n` followed by the member's `N` stored bytes.
`icon.png` absent from an artifact is `unknown-member`.

### 5.4 `phones`

Request: `{"v":1,"op":"phones"}`

Reply: `{"ok":true,"phones":[PHONE, …]}`, sorted by `identity`, where `PHONE` is

    {"identity":B64URL,"label":TEXT,"public_key":BASE64,"enrolled":RFC3339}

* `identity` is base64url, unpadded, of the SHA-256 of the DER
  SubjectPublicKeyInfo.
* `public_key` is standard base64, padded, of that DER.
* `enrolled` is UTC, to the second, with a `Z` suffix.

### 5.5 `enrol`

Request: `{"v":1,"op":"enrol","public_key":BASE64,"label":TEXT}`

Reply: `{"ok":true,"phone":PHONE}`, the record as written.

* `rivet-fleetd` derives `identity` from `public_key`. A request carrying an
  `identity` field is `malformed`.
* `enrolled` is the provider's clock at the write.
* The write is `register/phones/<identity>.toml`, atomically replaced.

### 5.6 `rename`

Request: `{"v":1,"op":"rename","identity":B64URL,"label":TEXT}`

Reply: `{"ok":true,"phone":PHONE}`. Only `label` changes.

### 5.7 `revoke`

Request: `{"v":1,"op":"revoke","identity":B64URL}`

Reply: `{"ok":true}`. The phone's file is removed. Revoking an identity that is
not enrolled is `unknown-phone`, so a caller learns that its view was stale.

---

## 6. What each end must not assume

* **Gantry must not cache the phone list across requests that authenticate a
  phone.** A revocation takes effect at the next `phones`, and a cached copy
  would keep a revoked phone working.
* **`rivet-fleetd` reads the store and the register at every request.** Two
  writers touch `register/phones/` — this socket and the control plane's
  `apply rename-phone` / `apply revoke-phone` (`ADR-061` §6.3) — and each
  operation touches one file atomically, so neither may hold a copy.
* **The key check is structural.** `bad-key` refuses anything but the 91-byte
  DER SubjectPublicKeyInfo of an uncompressed P-256 point; it does not check
  that the point lies on the curve. Gantry proves possession of the key during
  pairing, which a point off the curve cannot pass.
* **The digest is the store's name for the object**, which the store's door
  verified as the BLAKE3 of the `.rvt` when it admitted it (`ADR-052`); the
  socket does not recompute it.
* **The header is not trusted because the socket returned it.** Gantry
  verifies the artifact's signature itself before it offers an APK to a phone.

---

## 7. Conformance

An implementation of either end is tested against the other end's bytes:

1. **Provider:** a scripted client under `gantry`'s uid, driving every
   operation and every failure code in §4, and a client under any other uid
   refused before its request is read.
2. **Consumer:** a transcript fixture of replies written by the provider,
   labelled as provider-generated, replayed to Gantry's client code.
3. **A restart of `rivet-fleetd` between an `enrol` and a `phones`** returns
   the phone, because the record is on disk and nothing is held in memory.
