//! `cargo rivet identity new` — mint a device's TLS identity.
//!
//! **The fleet tool generates the pair. `ADR-025` §3 requires it**, and gives
//! the reason: *"a device generating its own keypair must get the public half
//! to the dashboard somehow, which needs the dashboard reachable at first boot
//! — the exact circularity that enrolling-before-boot exists to avoid."*
//! `ADR-019` §2 says the same from the other end — *"credentials never traverse
//! a network to enroll — they are written to a card by the operator's own
//! tooling"*.
//!
//! So a Rivet machine never mints its own identity, and `rivetos-tls` — which
//! both this tool and `cmd/rivet-mcpd` link — cannot mint one at all. This is
//! the only place in the workspace that can.
//!
//! # Why the certificate is minted here and not derived on the board
//!
//! It could be. The certificate is a container (`ADR-035` §7.2) and a board
//! holding the seed could build one deterministically at every boot. It is not
//! done, for a dependency reason that is worth stating rather than leaving as
//! an inference: minting needs `rcgen`, and `rcgen` brings `time` and seven
//! other crates. **A date library, on a card, to write a validity window
//! `rivetos-tls` forbids anyone from reading** would be a strange thing to
//! carry — and `ADR-009` §5's cross-compilation gate applies to every one of
//! those crates for no benefit.
//!
//! # What this does NOT do
//!
//! **It does not write to a register and it does not know what a fleet is.**
//! `ADR-034` §10.4's revisit trigger is *"the first credential in the store
//! that is not a shared network credential"*, and a device private key is
//! exactly that — so putting one in `cmd/rivet-fleet`'s credential store is a
//! decision `AGENTS.md` §1 reserves to the maintainer, not a detail of this
//! command. What this prints is the PUBLIC half, which is not a credential and
//! fires no trigger: `ADR-025` §3 has the fleet retain only that.

use std::path::{Path, PathBuf};

/// Which of the two things `identity` does.
///
/// **`Show` reads and never writes**, and that is why it can sit beside a verb
/// that refuses to overwrite. It answers *what is this identity's public key*
/// from the certificate on disk — the same question [`Verb::New`] answers once,
/// at mint time, in output that scrolls away.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Verb {
    /// Mint a new keypair and certificate.
    New,
    /// Print an existing identity's public key.
    ///
    /// This closes a gap `PLAN.md` §87 hit while proving phase 3 on a board: a
    /// `peers` file and a `--pin` are both hex public keys, `ADR-025` §3 has
    /// the fleet keep only the public half, and until this verb existed the
    /// only way to recover one from a minted identity was to decode the
    /// certificate DER by hand. An operator who cannot read a key back cannot
    /// compose a `peers` file, which makes the key unusable for its one job.
    Show,
}

/// What `identity` was asked to do.
#[derive(Debug, PartialEq, Eq)]
pub struct IdentityArgs {
    /// Mint, or read back.
    pub verb: Verb,
    /// The directory to write `device.seed` and `device.crt` into.
    pub directory: PathBuf,
    /// The name that goes in the certificate's subject `commonName`.
    ///
    /// **Not an authority, and not cosmetic either** — the distinction matters
    /// and this record got it wrong until 2026-09-06. `rivetos-tls` never reads
    /// it and nothing authenticates by it: the pin is the identity, and a name
    /// deciding anything would be a second authority for one question
    /// (`ADR-011`). But it is **read by people**: `rivet-proxyd`'s index prints
    /// the caller's as *"Authenticated as: …"*, and the journal records it
    /// beside every change that caller makes.
    ///
    /// So it carries no authority and is still load-bearing, which is why it
    /// may contain spaces and is asked for rather than assumed.
    pub name: String,
}

pub fn usage() -> String {
    "cargo rivet identity new  --directory DIR [--name NAME]\n\
     cargo rivet identity show --directory DIR\n\
     \n\
     Mint a device's TLS identity — an Ed25519 keypair and the self-signed\n\
     certificate that carries it (ADR-035 §7.2, ADR-025 §2).\n\
     \n\
     \x20 --directory DIR   where to write device.seed and device.crt. Both are\n\
     \x20                   refused if they already exist; this never overwrites\n\
     \x20                   an identity, because doing so would silently lock out\n\
     \x20                   every peer that had pinned the old one.\n\
     \x20 --name NAME       the certificate's subject commonName. Nothing\n\
     \x20                   AUTHENTICATES by it — the pin is the identity — but\n\
     \x20                   people READ it: the machine's index page prints an\n\
     \x20                   operator's, and the journal records it beside every\n\
     \x20                   change. Spaces are fine. Default: rivet-device.\n\
     \n\
     `new` prints the PUBLIC key as hex, which is what goes in the other end's\n\
     pin file. The private half stays in device.seed and is never printed.\n\
     \n\
     `show` prints that same key again, read out of device.crt. It writes\n\
     nothing and never opens device.seed — so it is how to recover a pin for\n\
     a peers file once the minting output has scrolled away.\n\
     pin file. The private half stays in device.seed and is never printed.\n\
     \n\
     THE CERTIFICATE'S VALIDITY WINDOW IS NOT A TRUST CONDITION. ADR-035 §7.1\n\
     requires the machine to say which of ignore-it or set-it-far-out it does,\n\
     and Rivet IGNORES it — rivetos-tls never reads notBefore or notAfter.\n"
        .to_string()
}

/// # Errors
///
/// If an argument is unknown or missing its value.
pub fn parse(argv: &[String]) -> Result<IdentityArgs, String> {
    let mut directory: Option<PathBuf> = None;
    let mut name: Option<String> = None;

    // **The verb is required and there is no default.** `new` writes a keypair
    // and `show` reads one; a bare `identity --directory DIR` defaulting to
    // either would eventually mint where a read was meant.
    let (verb, mut i) = match argv.first().map(String::as_str) {
        Some("new") => (Verb::New, 1),
        Some("show") => (Verb::Show, 1),
        None | Some("-h") | Some("--help") => return Err(usage()),
        Some(other) => {
            return Err(format!("{other:?} is not an identity verb\n\n{}", usage()))
        }
    };

    while i < argv.len() {
        match argv[i].as_str() {
            "--directory" => {
                i += 1;
                directory = Some(PathBuf::from(argv.get(i).ok_or("--directory needs a path")?));
            }
            "--name" => {
                i += 1;
                name = Some(argv.get(i).ok_or("--name needs a value")?.clone());
            }
            "-h" | "--help" => return Err(usage()),
            other => return Err(format!("{other:?} is not an option\n\n{}", usage())),
        }
        i += 1;
    }

    if verb == Verb::Show && name.is_some() {
        return Err("--name is meaningless with show: the name is already in the \
                    certificate, and nothing reads it — ADR-011, the pin is the \
                    identity."
            .to_string());
    }

    Ok(IdentityArgs {
        verb,
        directory: directory.ok_or_else(|| format!("--directory is required\n\n{}", usage()))?,
        name: name.unwrap_or_else(|| "rivet-device".to_string()),
    })
}

/// Mint the pair and write both files.
///
/// # Errors
///
/// If either file exists already, if the directory cannot be written, or if
/// the key cannot be generated.
pub fn run(args: &IdentityArgs) -> Result<String, String> {
    if args.verb == Verb::Show {
        return show(args);
    }
    let seed_at = args.directory.join(rivetos_tls::SEED_FILE);
    let cert_at = args.directory.join(rivetos_tls::CERTIFICATE_FILE);

    // **Checked before anything is generated.** Minting and then refusing to
    // write would leave a key in this process's memory that nothing can use,
    // and — worse — a half-written identity if only one file existed.
    for path in [&seed_at, &cert_at] {
        if path.exists() {
            return Err(format!(
                "{} already exists, and this refuses to overwrite an identity.\n\n\
                 Replacing one silently locks out every peer that pinned the old key \
                 (ADR-025 §2), and on a headless board that is unrecoverable without \
                 rewriting the card. Move the directory aside if you mean to re-provision.",
                path.display()
            ));
        }
    }
    std::fs::create_dir_all(&args.directory)
        .map_err(|e| format!("cannot create {}: {e}", args.directory.display()))?;

    // `rcgen` takes its randomness from `ring`, which takes it from the
    // operating system — `ADR-001` §7.3's property exactly, and the same
    // source `key new` uses through `getrandom`.
    let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
        .map_err(|e| format!("cannot generate an Ed25519 keypair: {e}"))?;
    let mut params = rcgen::CertificateParams::new(vec![args.name.clone()])
        .map_err(|e| format!("cannot build the certificate's parameters: {e}"))?;
    // **The name has to be written into the SUBJECT, and this line is what
    // does it.** `CertificateParams::new` takes *subject alternative names*;
    // the distinguished name is left at rcgen's own default, so before
    // 2026-09-06 every identity this verb minted carried a `commonName` of
    // `rcgen self signed cert` — including the operator's, whose name
    // `rivet-proxyd`'s index prints as *"Authenticated as: …"*. The `--name`
    // flag was documented as cosmetic and was in fact not reaching the field
    // anybody reads.
    //
    // The SAN is left exactly as it was. It is what a *client* verifies a
    // board's certificate against, and changing what goes in it is a change to
    // whether TLS succeeds — a separate decision from what a person is called.
    let mut subject = rcgen::DistinguishedName::new();
    subject.push(rcgen::DnType::CommonName, args.name.clone());
    params.distinguished_name = subject;
    // **A leaf, and never an authority.** `ADR-035` §7.2: a self-signed leaf is
    // the whole certificate, and `rivetos-tls` refuses a chain outright.
    params.is_ca = rcgen::IsCa::NoCa;
    let certificate = params
        .self_signed(&pair)
        .map_err(|e| format!("cannot self-sign the certificate: {e}"))?;

    let pkcs8 = pair.serialize_der();
    let seed = seed_of(&pkcs8)?;
    let der = certificate.der().to_vec();

    let public = rivetos_tls::public_key_in_certificate(&der).ok_or_else(|| {
        "the certificate just minted carries no Ed25519 public key, which should be impossible \
         and means rivetos-tls and rcgen disagree about the encoding"
            .to_string()
    })?;

    write_secret(&seed_at, &seed)?;
    std::fs::write(&cert_at, &der)
        .map_err(|e| format!("cannot write {}: {e}", cert_at.display()))?;

    Ok(format!(
        "identity  {name}\n\
         seed      {seed_at}  ({note}{bytes} bytes)\n\
         cert      {cert_at}  ({cert_bytes} bytes, self-signed, no CA)\n\
         \n\
         public key — put this in the OTHER end's pin file:\n\
         \n\
         \x20 {hex}\n\
         \n\
         The private half is in device.seed and was not printed. ADR-025 §3 has the\n\
         fleet retain only the public key; this output is that half.\n\
         \n\
         Expiry is not a trust condition — ADR-035 §7.1, and rivetos-tls ignores the\n\
         validity window entirely rather than pretending to check a clock ADR-021\n\
         says is wrong at boot.\n",
        name = args.name,
        seed_at = seed_at.display(),
        cert_at = cert_at.display(),
        note = permission_note(),
        bytes = seed.len(),
        cert_bytes = der.len(),
        hex = rivetos_tls::encode_hex(&public),
    ))
}

/// The seed inside a PKCS#8 blob.
///
/// **`rcgen` returns PKCS#8 v2, which appends the PUBLIC key after the private
/// one**, so the last thirty-two bytes are the wrong half — and writing them
/// produces a `device.seed` that `rustls` rejects with `KeyMismatch` the first
/// time a board tries to serve with it. The private key is the thirty-two bytes
/// after the `04 22 04 20` that opens its OCTET STRING.
fn seed_of(pkcs8: &[u8]) -> Result<[u8; rivetos_tls::KEY_BYTES], String> {
    let at = pkcs8
        .windows(4)
        .position(|w| w == [0x04, 0x22, 0x04, 0x20])
        .ok_or("rcgen produced a PKCS#8 blob with no Ed25519 private key octet string in it")?
        + 4;
    pkcs8
        .get(at..at + rivetos_tls::KEY_BYTES)
        .and_then(|s| <[u8; rivetos_tls::KEY_BYTES]>::try_from(s).ok())
        .ok_or_else(|| "rcgen's PKCS#8 blob ends before the seed does".to_string())
}

/// Write the seed as tightly as the platform allows — `key.rs`'s argument, and
/// the same code, because a seed is a seed.
#[cfg(unix)]
fn write_secret(path: &Path, seed: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    file.write_all(seed).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    file.sync_all().map_err(|e| format!("cannot flush {}: {e}", path.display()))
}

/// The same, where there is no mode to set in the `open`.
#[cfg(not(unix))]
fn write_secret(path: &Path, seed: &[u8]) -> Result<(), String> {
    use std::io::Write as _;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    file.write_all(seed).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    file.sync_all().map_err(|e| format!("cannot flush {}: {e}", path.display()))
}

#[cfg(unix)]
const fn permission_note() -> &'static str {
    "mode 0600, "
}

/// **Windows has no mode and this does not pretend to set one** — `key.rs`'s
/// note, for the same reason: the development host is Windows (`AGENTS.md`
/// §10.5) and a report claiming a protection the platform did not apply is
/// worse than one that says nothing.
#[cfg(not(unix))]
const fn permission_note() -> &'static str {
    "no mode set — this platform has none, the directory's ACL is the protection; "
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn parsed(words: &[&str]) -> Result<IdentityArgs, String> {
        parse(&words.iter().map(|w| (*w).to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn a_directory_is_required_and_the_name_has_a_default() {
        assert!(parsed(&["new"]).expect_err("no directory").contains("--directory is required"));
        let args = parsed(&["new", "--directory", "/tmp/x"]).expect("a directory is enough");
        assert_eq!(args.name, "rivet-device");
    }

    /// A directory no other test and no earlier run will be using.
    ///
    /// **The process id alone is not enough on Windows**, which recycles them
    /// freely — a name built from the pid collided with a directory left by an
    /// earlier `cargo test`, and `write_secret`'s `create_new` failed on a file
    /// this test believed it had just removed. `crate::scratch` closes that for
    /// every helper at once: it wipes the per-process root as it creates it, so
    /// a recycled pid inherits an empty one. The counter still separates calls
    /// *within* a run.
    fn scratch(what: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        crate::scratch::absent(&format!("{what}-{}", NEXT.fetch_add(1, Ordering::Relaxed)))
    }

    /// **`show` prints the same key `new` did.** They are two readings of one
    /// certificate and a disagreement would mean an operator composing a
    /// `peers` file from the wrong half of a pair — every peer locked out, with
    /// no error until a handshake fails on a headless board.
    #[test]
    fn show_prints_the_key_new_printed() {
        let dir = scratch("id-show");
        let minted = run(&IdentityArgs {
            verb: Verb::New,
            directory: dir.clone(),
            name: "a-board".to_string(),
        })
        .expect("a fresh directory");

        let shown = run(&IdentityArgs {
            verb: Verb::Show,
            directory: dir.clone(),
            name: "rivet-device".to_string(),
        })
        .expect("an identity that was just minted");

        let key = rivetos_tls::encode_hex(
            &rivetos_tls::public_key_in_certificate(
                &std::fs::read(dir.join(rivetos_tls::CERTIFICATE_FILE)).expect("the cert"),
            )
            .expect("an Ed25519 key"),
        );
        assert!(minted.contains(&key), "new did not print the key");
        assert!(shown.contains(&key), "show did not print the key");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The private half is not printed, and this is the test that says so.**
    /// `show` reads only the certificate; a seed reaching stdout would be a
    /// key in somebody's scrollback and in every CI log that ran it.
    #[test]
    fn show_never_prints_the_private_half() {
        let dir = scratch("id-priv");
        run(&IdentityArgs {
            verb: Verb::New,
            directory: dir.clone(),
            name: "a-board".to_string(),
        })
        .expect("a fresh directory");

        let seed = std::fs::read(dir.join(rivetos_tls::SEED_FILE)).expect("the seed");
        let shown = run(&IdentityArgs {
            verb: Verb::Show,
            directory: dir.clone(),
            name: "rivet-device".to_string(),
        })
        .expect("an identity");
        assert!(
            !shown.contains(&rivetos_tls::encode_hex(&seed)),
            "show printed the private seed"
        );
        assert!(!shown.contains("device.seed  "), "show named the seed as a file it read");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A directory that is not an identity says so, rather than printing
    /// nothing — which an operator reads as *this identity has no key*.
    #[test]
    fn show_on_a_directory_with_no_certificate_says_what_is_wrong() {
        let dir = scratch("id-none");
        let refusal = run(&IdentityArgs {
            verb: Verb::Show,
            directory: dir.clone(),
            name: "rivet-device".to_string(),
        })
        .expect_err("there is no certificate there");
        assert!(refusal.contains("CERTIFICATE"), "{refusal}");
    }

    /// The verb is required, and an unknown one is refused by name.
    #[test]
    fn the_verb_is_required_and_show_takes_no_name() {
        let of = |v: &[&str]| parse(&v.iter().map(|s| (*s).to_string()).collect::<Vec<_>>());
        assert_eq!(of(&["show", "--directory", "d"]).expect("valid").verb, Verb::Show);
        assert_eq!(of(&["new", "--directory", "d"]).expect("valid").verb, Verb::New);
        assert!(of(&["--directory", "d"]).is_err(), "a bare directory must not default");
        assert!(of(&["mint", "--directory", "d"]).is_err(), "unknown verb");
        let refusal = of(&["show", "--directory", "d", "--name", "x"]).expect_err("refused");
        assert!(refusal.contains("meaningless"), "{refusal}");
    }

    /// **The whole point, end to end**: mint, then read it back with the same
    /// code `cmd/rivet-mcpd` uses, and confirm the printed key is the one in
    /// the certificate.
    ///
    /// **This carried no `#[test]` until 2026-09-05** and had never run. The
    /// doc comment above it, and the attribute that should have been on it, had
    /// both been left stranded above `show_prints_the_key_new_printed` — which
    /// therefore carried two — so `rustc` reported a duplicated attribute and a
    /// dead function, and the strongest test in this file was the one nobody
    /// was running. `AGENTS.md` §4: *a test suite that cannot be run will rot
    /// silently.* This one could not even be reached.
    #[test]
    fn a_minted_identity_is_one_rivetos_tls_reads_back() {
        let dir = scratch("identity");
        let args = IdentityArgs { verb: Verb::New, directory: dir.clone(), name: "a-board".to_string() };

        let report = run(&args).expect("minting");
        let identity = rivetos_tls::read_identity(&dir).expect("what was just written");
        let key = identity.public_key().expect("an Ed25519 key");
        assert!(
            report.contains(&rivetos_tls::encode_hex(&key)),
            "the printed key is not the certificate's: {report}"
        );
        assert_eq!(identity.seed.len(), rivetos_tls::KEY_BYTES);

        // And the pair actually agrees — rustls checks this, and it is the
        // check that catches PKCS#8 v2's trailing public key being taken for
        // the seed.
        rivetos_tls::server_config(&identity, vec![key], rivetos_tls::provider())
            .expect("the certificate and the seed are halves of one key");

        // Never overwrites.
        let again = run(&args).expect_err("a second mint must refuse");
        assert!(again.contains("already exists"), "{again}");
        assert!(again.contains("locks out every peer"), "{again}");

        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// **`--name` reaches the `commonName`, and until 2026-09-06 it did not.**
    ///
    /// `CertificateParams::new` takes *subject alternative names*; the subject
    /// itself was left at rcgen's default, so every identity this verb had ever
    /// minted carried a `commonName` of `rcgen self signed cert`. The flag was
    /// documented as cosmetic, so nobody looked — and by then `rivet-proxyd`'s
    /// index was printing that field as *"Authenticated as: …"*.
    ///
    /// **The space is deliberate.** The maintainer's stated intent is to type
    /// his own name, so a subject that could not carry one would be a
    /// requirement failing quietly rather than loudly.
    #[test]
    fn the_name_given_is_the_common_name_written() {
        let dir = scratch("common-name");
        let args = IdentityArgs {
            verb: Verb::New,
            directory: dir.clone(),
            name: "Your Name".to_string(),
        };
        run(&args).expect("minting");

        let der = std::fs::read(dir.join(rivetos_tls::CERTIFICATE_FILE)).expect("a certificate");
        assert_eq!(
            rivetos_tls::subject_common_name(&der).as_deref(),
            Some("Your Name"),
            "the name given is not the name in the subject"
        );

        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// The private half is never in the report. This is the test that stops a
    /// convenience print from leaking one.
    #[test]
    fn the_report_never_contains_the_private_half() {
        let dir = scratch("identity-p");
        let args = IdentityArgs { verb: Verb::New, directory: dir.clone(), name: "a-board".to_string() };
        let report = run(&args).expect("minting");
        let identity = rivetos_tls::read_identity(&dir).expect("what was written");
        assert!(
            !report.contains(&rivetos_tls::encode_hex(&identity.seed)),
            "the seed reached the report"
        );
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}

/// Print an existing identity's public key, reading only the certificate.
///
/// **`device.seed` is never opened.** The public key is in the certificate, so
/// reading it needs no access to the private half — and a read-back verb that
/// touched the secret would be one an operator could not run casually.
///
/// # Errors
///
/// If the certificate is missing or carries no Ed25519 key. Both say what is
/// wrong rather than printing nothing, because an empty answer here reads as
/// *this identity has no key* rather than *this is not an identity*.
fn show(args: &IdentityArgs) -> Result<String, String> {
    let cert_at = args.directory.join(rivetos_tls::CERTIFICATE_FILE);
    let der = std::fs::read(&cert_at).map_err(|e| {
        format!(
            "cannot read {}: {e}\n\n\
             `identity show` reads the CERTIFICATE, which is where the public key is. \
             A directory holding only device.seed is a half-written identity, and this \
             deliberately does not open the private half to derive one.",
            cert_at.display()
        )
    })?;

    let public = rivetos_tls::public_key_in_certificate(&der).ok_or_else(|| {
        format!(
            "{} carries no Ed25519 public key.\n\n\
             Rivet's control plane pins Ed25519 and nothing else (ADR-025 §2), so a \
             certificate of another kind is not one this fleet can use even though it \
             may be a perfectly valid certificate.",
            cert_at.display()
        )
    })?;

    Ok(format!(
        "cert      {cert_at}  ({bytes} bytes)\n\
         \n\
         public key — this is the pin, and it is what goes in a peers file:\n\
         \n\
         \x20 {hex}\n\
         \n\
         Read from the certificate. device.seed was not opened, and the private half\n\
         is not printed by any verb.\n",
        cert_at = cert_at.display(),
        bytes = der.len(),
        hex = rivetos_tls::encode_hex(&public),
    ))
}
