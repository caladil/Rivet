# APK fixtures for `cargo-rivet`'s APK reader

Vectors for `cmd/cargo-rivet/src/apk_read.rs` (`ADR-061` §4). The manifest and
resource-table bytes in them come from two real debug builds; **every
signature and certificate in them was made by `apksigner` with throwaway
keys**, so no build's own signing certificate is committed. Every
hand-composed vector the tests use is built in code by `apk_read_tests.rs` and
labelled there; nothing in this directory is hand-composed.

## The two throwaway keys

Minted with `keytool` (the JDK bundled with Android Studio) in a temporary
directory, used for the signing below, and deleted afterwards:

| key | subject | certificate SHA-256 |
| :-- | :-- | :-- |
| one | `CN=Rivet Test One` | `de920ba08faf66400adf285fef796a374da7111a98b7ba0bdd087f2cd69ef06f` |
| two | `CN=Rivet Test Two` | `acd2503f132b6970932f302fd40ee91df3030bdda60037c9bac2c192056d740f` |

Both are RSA-2048, `-startdate "2000/01/01 00:00:00" -validity 36500`, so each
certificate's notBefore is 2000-01-01 (keytool writes local midnight; the
files hold `000101060000Z`) and its notAfter 2099-12-07. **Those two times are
the only times in these files**, and `no_fixture_carries_a_real_date_or_the_debug_certificate`
holds them to it by scanning every DER time field. The zip entries carry the
fixed 1981-01-01 timestamp Android's build tools write.

## `hc-writer-derived.apk`, `rivet-harness-derived.apk`

**Manifest and resource table derived from debug builds of two of the
maintainer's own Android applications** — a Trends data-writer
tool and the Rivet Android test harness. The ignored test
`derive_a_fixture_from_a_real_apk` writes an **unsigned** zip holding:

* `AndroidManifest.xml` — the original entry's bytes, **verbatim**, as they lay
  in the build's zip (deflated), with its original CRC-32 and sizes;
* `resources.arsc` — the original table's bytes, verbatim. In
  `hc-writer-derived.apk` it is **stored**, as the build stored it; in
  `rivet-harness-derived.apk` the same bytes are **deflated** by the deriving
  test (`RIVET_APK_FIXTURE_DEFLATE_ARSC`) to keep the fixture small, which also
  puts a real table through the inflate path.

Before writing, that test reads the new zip with the original build's own
signing block put back in, and asserts it reads to exactly what the original
does. The unsigned zip was then signed with key one:

```text
apksigner sign --ks one.p12 --v1-signing-enabled false --v2-signing-enabled true
               --v3-signing-enabled false --out <fixture> <unsigned zip>
```

### What Android's own tools printed

`aapt2 dump badging` (build-tools 36.0.0) on the ORIGINAL builds — and the
same `package`, `minSdkVersion`, `targetSdkVersion` and `application-label`
lines again on each committed fixture:

| field | hc-writer | rivet-harness |
| :-- | :-- | :-- |
| `package: name` | `io.github.caladil.hcwriter` | `io.github.caladil.rivetharness` |
| `versionCode` | `1` | `16` |
| `versionName` | `0.0.1` | `0.1.16` |
| `minSdkVersion` | `31` | `31` |
| `targetSdkVersion` | `36` | `36` |
| `application-label` | `HC Writer` | `Rivet Pair Harness` |

`apksigner verify --print-certs -v` (build-tools 36.0.0) on each committed
fixture: verifies with v2 only; one signer, `CN=Rivet Test One`, certificate
SHA-256 `de920ba08faf66400adf285fef796a374da7111a98b7ba0bdd087f2cd69ef06f`.

The two manifests hold their labels the two ways a manifest can:
hc-writer's `android:label` is a literal string, and the harness's is a
reference (`@0x7f0c001c`) resolved through `resources.arsc`.

## `rotated-v3.apk`, `rotated-v31.apk`

**Real `apksigner` output with a real proof-of-rotation lineage, key one to
key two.** The input was hc-writer's manifest alone in an unsigned zip, written
by the ignored test `derive_an_unsigned_manifest_only_zip_from_a_real_apk`.
The keys were linked with `apksigner rotate --old-signer <one> --new-signer
<two>`, and the input signed with `apksigner sign --ks <one> --next-signer --ks
<two> --lineage … --min-sdk-version 28 --v1-signing-enabled false`:

* `rotated-v3.apk` with `--rotation-min-sdk-version 28`: the rotation is in the
  **v3** block;
* `rotated-v31.apk` with apksigner's default rotation SDK (33): the **v3.1**
  block names key two for SDK 33 and above, and the v3 block names key one for
  SDK 24–32.

`apksigner lineage --print-certs` on `rotated-v3.apk`, and `apksigner verify
--print-certs --min-sdk-version 28` on both:

| | certificate SHA-256 |
| :-- | :-- |
| lineage signer #1 (key one, the old key) | `de920ba08faf66400adf285fef796a374da7111a98b7ba0bdd087f2cd69ef06f` |
| lineage signer #2 (key two), the current signer | `acd2503f132b6970932f302fd40ee91df3030bdda60037c9bac2c192056d740f` |

They carry no `resources.arsc`, and need none: hc-writer's label is a
literal, so each reads whole — hc-writer's manifest values, and the rotated
key's signer and lineage.

## Reading a real build

The ignored test `a_real_apk_named_by_the_environment_reads` reads a whole
APK named by `RIVET_APK_FIXTURE`, printing what it found in `aapt2`'s and
`apksigner`'s terms. Real builds are read from wherever they are; they are
never committed.
