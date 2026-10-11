//! **Sign a stand-in application, and write the store that trusts it** — for
//! `tools/qemu/adr-042-proof.sh` and nothing else.
//!
//! # Why this exists
//!
//! `cmd/rivet-sysd`'s `verify` module checks every application against a
//! retained signed header and the machine's trust store *before every launch*
//! (`ADR-015` §4.2, `ADR-028` §5.5.4). `tools/qemu/run.sh --app` deliberately
//! leaves an application behind without either — it says so on the console,
//! *"NOT signed, NOT verified"* — which was fine while the harness only needed
//! the supervisor to *define* a service from a manifest, and stopped being fine
//! the moment a proof needed the supervisor to actually **start** one.
//!
//! So this writes the two files `--app` cannot: the 4096-byte header at
//! `rivet_core::package::install::HEADER_NAME`, and a trust store at
//! `rivet_core::package::trust_store::PATH`.
//!
//! # It is a FIXTURE and its key is not a key
//!
//! **The seed is thirty-two identical bytes and it is in this file.**
//! `AGENTS.md` §8.3 forbids committing credentials; this is not one, for the
//! same reason `cmd/rivet-sysd`'s own `verify::fixture::seed` is not — it
//! signs nothing that any machine outside this harness trusts, and a real
//! machine's trust store does not carry the public key it derives. The
//! maintainer's actual application seed is at a path this program never opens
//! and never asks for.
//!
//! Nothing here is a second implementation: the header comes from
//! `rivet_core::package::write::Builder`, the signature from
//! `rivet_crypto::Ed25519Signer` and the store from
//! `rivet_core::package::trust_store::TrustStore` — the same three the real
//! tools use (`ADR-011`).
//!
//! # Usage
//!
//! ```text
//! rivet-fixture-signer <name> <program> <manifest> <header-out> <trust-out>
//! rivet-fixture-signer trust-store <out> <label>:<kind[,kind]>:<public-key> …
//! ```
//!
//! # The second form, and why it is here rather than in a shell script
//!
//! `tools/qemu/adr-052-proof.sh` needs a machine that trusts a **real** key —
//! the public half in `keys/`, whose seal `cargo rivet package` applies — so
//! that `cmd/rivet-deploy` can verify a real artifact out of the machine's
//! artifact store. `tools/qemu/run.sh --trust` takes a store file and its own
//! help refuses to compose one, for `ADR-011`'s reason: a shell script emitting
//! a security-relevant format would be a second producer of it.
//!
//! `cmd/rivet-image` is the first producer and it writes the store **into a card
//! image**, which is two gigabytes and a Buildroot tree away from a proof that
//! needs thirty-two bytes. So the second form here is the same
//! `rivet_core::package::trust_store::TrustStore` and the same
//! `rivet_core::package::key_material` parser that tool uses, writing the file
//! alone — **not a second implementation, and not a shell script's idea of the
//! format.**
//!
//! It signs nothing and reads no seed: a public key goes in, a trust store comes
//! out.

use rivet_core::package::application::{TargetClass, VARIANT_NONE};
use rivet_core::package::format::HEADER_BYTES;
use rivet_core::package::kind::Kind;
use rivet_core::package::port::{MaySign, PublicKey};
use rivet_core::package::trust_store::{Entry, TrustStore};
use rivet_core::package::write::{Builder, NewMember};
use rivet_crypto::{Blake3, Ed25519Signer, SEED_BYTES};

/// **Not a key.** See the module note: thirty-two identical bytes, so that a
/// failure reproduces and so that nothing about this fixture resembles a
/// secret.
const FIXTURE_SEED: [u8; SEED_BYTES] = [0x2a; SEED_BYTES];

const USAGE: &str = "usage: rivet-fixture-signer <name> <program> <manifest> <header-out> \
                     <trust-out>\n       rivet-fixture-signer trust-store <out> \
                     <label>:<kind[,kind]>:<public-key> …\n";

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("trust-store") {
        return trust_store(&args[1..]);
    }
    let [name, program_path, manifest_path, header_out, trust_out] = args.as_slice() else {
        eprint!("{USAGE}");
        return std::process::ExitCode::FAILURE;
    };

    let program = match std::fs::read(program_path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("rivet-fixture-signer: {program_path}: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    // **The manifest bytes as they will be INSTALLED.** The verifier compares
    // the program member's digest with the file on disk and does not re-read
    // the manifest, but a package whose manifest member did not match the file
    // beside it would be a fixture that is subtly not what an installer leaves,
    // and the next person to extend this would inherit the discrepancy.
    let manifest = match std::fs::read(manifest_path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("rivet-fixture-signer: {manifest_path}: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    let mut digest = Blake3::new();
    // `aarch64`, because that is what the guest is and what the harness builds
    // for. A header naming another architecture would be refused, which is the
    // correct behaviour and not the thing under test here.
    let mut builder = match Builder::new(name, "0.0.1", "aarch64", None, Kind::application()) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("rivet-fixture-signer: header: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let manifest_member = NewMember::stored(
        rivet_core::package::application::MANIFEST_NAME,
        TargetClass::Manifest.as_byte(),
        VARIANT_NONE,
        &manifest,
        &mut digest,
    );
    let program_member = NewMember::stored(
        name,
        TargetClass::Program.as_byte(),
        VARIANT_NONE,
        &program,
        &mut digest,
    );
    let (Ok(manifest_member), Ok(program_member)) = (manifest_member, program_member) else {
        eprintln!("rivet-fixture-signer: a member could not be described");
        return std::process::ExitCode::FAILURE;
    };
    if builder.add(manifest_member).is_err() || builder.add(program_member).is_err() {
        eprintln!("rivet-fixture-signer: a member could not be added");
        return std::process::ExitCode::FAILURE;
    }

    let mut signer = Ed25519Signer::from_seed(&FIXTURE_SEED);
    let whole = match builder.seal_in_memory(&mut digest, &mut signer, &[&manifest, &program]) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("rivet-fixture-signer: sealing: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    if whole.len() < HEADER_BYTES {
        eprintln!("rivet-fixture-signer: the sealed package is shorter than one header");
        return std::process::ExitCode::FAILURE;
    }

    let store = TrustStore::new(vec![Entry {
        key: rivet_crypto::public_key_for(&FIXTURE_SEED),
        label: "qemu-fixture".to_string(),
        may_sign: MaySign::these(&[Kind::application()]),
    }]);
    let store_bytes = match store.to_bytes() {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("rivet-fixture-signer: trust store: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    if let Err(e) = std::fs::write(header_out, &whole[..HEADER_BYTES]) {
        eprintln!("rivet-fixture-signer: {header_out}: {e}");
        return std::process::ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(trust_out, &store_bytes) {
        eprintln!("rivet-fixture-signer: {trust_out}: {e}");
        return std::process::ExitCode::FAILURE;
    }
    println!(
        "rivet-fixture-signer: signed `{name}` ({} program bytes) — header at {header_out}, \
         trust store at {trust_out}",
        program.len()
    );
    std::process::ExitCode::SUCCESS
}

/// **A trust store from public keys, and nothing else** — the second form in the
/// module note.
///
/// Each argument is `<label>:<kind[,kind]>:<path>`, which is
/// `cmd/rivet-image --trusted-key`'s spelling on purpose: the two files that
/// produce this format should not need a reader to learn two grammars.
///
/// Nothing here signs and nothing here reads a seed. What it writes is what
/// `tools/qemu/run.sh --trust` places at
/// `rivet_core::package::trust_store::PATH`, so a guest can verify an artifact
/// signed by a key whose public half is in `keys/`.
fn trust_store(args: &[String]) -> std::process::ExitCode {
    let Some((out, specs)) = args.split_first() else {
        eprint!("{USAGE}");
        return std::process::ExitCode::FAILURE;
    };
    if specs.is_empty() {
        eprintln!(
            "rivet-fixture-signer: trust-store with no keys would write a store that trusts \
             NOBODY, and a machine carrying one refuses every install. Name at least one."
        );
        return std::process::ExitCode::FAILURE;
    }

    let mut entries = Vec::new();
    for spec in specs {
        // Split from the LEFT twice rather than on every colon: a path on
        // Windows carries one and a label never does.
        let Some((label, rest)) = spec.split_once(':') else {
            eprintln!("rivet-fixture-signer: {spec:?} is not <label>:<kinds>:<public-key>");
            return std::process::ExitCode::FAILURE;
        };
        let Some((kinds, path)) = rest.split_once(':') else {
            eprintln!("rivet-fixture-signer: {spec:?} is not <label>:<kinds>:<public-key>");
            return std::process::ExitCode::FAILURE;
        };
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                eprintln!("rivet-fixture-signer: {path}: {e}");
                return std::process::ExitCode::FAILURE;
            }
        };
        // `rivet_core::package::key_material`, so 32 raw bytes and 64 hex
        // characters both work and mean the same thing here as they do to
        // `cargo rivet package` and `cmd/rivet-image`.
        let raw = match rivet_core::package::key_material::parse(&bytes) {
            Ok(raw) => raw,
            Err(e) => {
                eprintln!("rivet-fixture-signer: the key at {path}: {e}");
                return std::process::ExitCode::FAILURE;
            }
        };
        let mut parsed = Vec::new();
        for k in kinds.split(',') {
            match Kind::parse(k) {
                Ok(kind) => parsed.push(kind),
                Err(e) => {
                    eprintln!("rivet-fixture-signer: {label}: {k:?} is not a kind: {e}");
                    return std::process::ExitCode::FAILURE;
                }
            }
        }
        entries.push(Entry {
            key: PublicKey(raw),
            label: label.to_string(),
            may_sign: MaySign::these(&parsed),
        });
    }

    let store = TrustStore::new(entries);
    let bytes = match store.to_bytes() {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("rivet-fixture-signer: trust store: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    if let Err(e) = std::fs::write(out, &bytes) {
        eprintln!("rivet-fixture-signer: {out}: {e}");
        return std::process::ExitCode::FAILURE;
    }
    println!(
        "rivet-fixture-signer: {} entries, {} bytes, at {out}",
        store.entries().len(),
        bytes.len()
    );
    for entry in store.entries() {
        let kinds: Vec<&str> = entry.may_sign.kinds().iter().map(Kind::as_str).collect();
        println!("  {} may sign {}", entry.label, kinds.join(", "));
    }
    std::process::ExitCode::SUCCESS
}
