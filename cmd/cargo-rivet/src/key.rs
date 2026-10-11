//! Reading a key off disk, and publishing the public half of one.
//!
//! `ADR-015` §4.3 makes the maintainer the gatekeeper; `ADR-028` §5.2 rules
//! where a *machine's* root set lives, and §5.5 what a trust entry is. **None
//! of them says where a developer's signing key lives**, and `ADR-028` §5.6
//! defers the root key deliberately — it should be generated on the day it will
//! actually be kept offline, because the offline discipline *is* the feature.
//!
//! So this module still has **no default location**, looks in no well-known
//! directory and reads no environment variable. Every one of those absences is
//! a place a design would have to be decided, and a tool that quietly picked
//! one would have decided it (`AGENTS.md` §1).
//!
//! # ~~It still generates nothing~~ — `key new`, ruled 2026-08-28
//!
//! ~~`rivet-crypto`'s Cargo.toml records that **there is no RNG in this
//! project's dependency graph at all**. Adding one so that a tool can say
//! `key new` would put a randomness source in the graph to save a person one
//! shell command.~~
//!
//! **`ADR-001` §7.** That rule lived here, in `rivet-crypto`'s comment block
//! and in `PLAN.md` §57.3, and in no ADR — and it was false besides:
//! `Cargo.lock` already carried three `getrandom` entries. More to the point,
//! **`getrandom` is not an RNG.** It is a shim over `getrandom(2)` and
//! `BCryptGenRandom`; it generates nothing and asks the kernel, which is
//! precisely what the sentence below still says.
//!
//! **The operating system makes the randomness.** `key new` does not change
//! that — `head -c 32 /dev/urandom` and `getrandom(2)` are the same kernel
//! call. What it changes is that the procedure for the highest-value secret in
//! this project is now *recorded and reproducible* instead of living in one
//! person's shell history, and that it works on the development host, which is
//! Windows and has no `/dev/urandom`.
//!
//! ```text
//! cargo rivet key new keys/system.seed             # asks the kernel
//! cargo rivet key keys/system.seed --out keys/rivet-system.pub
//! ```
//!
//! `ADR-029` §7.1's reason for Ed25519 is untouched and is a property rather
//! than a policy: **it needs no randomness at signing or at verification**, so
//! the update path still has no entropy requirement at all.
//!
//! `AGENTS.md` §8.3: the seed is never committed, and the test suite mints its
//! own into a temporary directory. The **public** key is not a secret and is
//! exactly the thing an image is built to carry (`ADR-028` §5.5.6).

use std::path::{Path, PathBuf};

use rivet_core::package::format::PUBLIC_KEY_BYTES;
use rivet_core::package::key_material;
use rivet_core::package::port::{Digest as _, Sign as _};
use rivet_crypto::{Blake3, Ed25519Signer};

/// 32 bytes, as raw bytes or as 64 hex characters.
///
/// Two encodings because the two ways a person actually produces 32 bytes
/// produce different files — `head -c 32 /dev/urandom > k` gives raw bytes and
/// anything that pastes a key gives hex.
///
/// **The rule lives in `rivet_core::package::key_material`**, not here, since
/// 2026-08-24: `cmd/rivet-image` had grown a second reader that took raw
/// bytes only, and two answers to *what does a key file look like* is
/// `ADR-011` §3. This is the I/O half and nothing more.
pub fn read_32(path: &Path, what: &str) -> Result<[u8; 32], String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("cannot read the {what} {}: {e}", path.display()))?;
    key_material::parse(&bytes).map_err(|e| format!("the {what} {}: {e}", path.display()))
}

/// Hex, for printing a **public** key. Never used on a seed.
pub fn hex(bytes: &[u8]) -> String {
    key_material::to_hex(bytes)
}

/// **Ask the operating system for a signing seed.** `ADR-001` §7.4 item 1.
///
/// # What this is and is not
///
/// It is 32 bytes from `getrandom(2)` on Linux and `BCryptGenRandom` on
/// Windows, written to a file. **It is not a generator**: nothing here
/// stretches, seeds, or iterates anything, and the crate that fetches them
/// exists only to make that one call on whichever platform this is.
///
/// # Three refusals, and the first is the one that matters
///
/// **An existing file is never overwritten.** A signing seed is the only
/// secret in this project whose loss cannot be repaired from anywhere: every
/// board that trusts its public half will refuse every future update, and
/// there is no backup by design (`AGENTS.md` §8.3 keeps it out of the
/// repository). `key`'s own `--out` guard uses the same argument one artifact
/// over, and this one is worse — a public key that is overwritten can be
/// re-derived from the seed, and a seed that is overwritten is gone.
///
/// **A short read is a refusal.** `getrandom` fills the buffer or fails; a
/// partial fill would produce a seed with predictable bytes in it that looks
/// exactly like a good one.
///
/// **The bytes are never printed.** Not to the terminal, not into the report,
/// not on the error path.
fn new_seed(path: &Path) -> Result<String, String> {
    if path.exists() {
        return Err(format!(
            "{} already exists, and `key new` will not overwrite a seed. Every board that \
             trusts this key would refuse every future update, and there is no backup — \
             AGENTS.md §8.3 keeps seeds out of this repository on purpose. Move it aside \
             if you genuinely mean to replace it.",
            path.display()
        ));
    }

    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed)
        .map_err(|e| format!("the operating system would not supply 32 random bytes: {e}"))?;

    write_seed(path, &seed)?;

    // Derived immediately, so the answer carries the thing an image has to be
    // built to trust — and so a seed that cannot produce a key is found here
    // rather than on the day something is signed.
    let signer = Ed25519Signer::from_seed(&seed);
    let public = signer.public_key();
    let key_id = Blake3::new().of(&public.0);

    Ok(format!(
        "seed        {} (32 bytes, from the operating system, NEVER printed)\n\
         public key  {}\n\
         key id      {}\n\
         \n\
         {}\
         Publish the public half and keep the seed off every machine that does not sign:\n\
         \x20   cargo rivet key {} --out keys/<name>.pub\n",
        path.display(),
        hex(&public.0),
        hex(&key_id),
        permission_note(path),
        path.display(),
    ))
}

/// Write the seed, as tightly as the platform allows.
///
/// On Unix the mode is set **in the `open`**, not afterwards, because a
/// `chmod` after the fact leaves a window in which the file exists and is
/// readable — and `create_new` makes the whole thing refuse rather than
/// truncate if something appeared between the check above and here.
#[cfg(unix)]
fn write_seed(path: &Path, seed: &[u8; 32]) -> Result<(), String> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    file.write_all(seed)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    file.sync_all()
        .map_err(|e| format!("cannot flush {}: {e}", path.display()))
}

/// The same, where there is no mode to set in the `open`.
///
/// `create_new` still refuses an existing file. The ACL is the directory's,
/// which is why the report says so rather than implying a protection this
/// cannot give.
#[cfg(not(unix))]
fn write_seed(path: &Path, seed: &[u8; 32]) -> Result<(), String> {
    use std::io::Write as _;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    file.write_all(seed)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    file.sync_all()
        .map_err(|e| format!("cannot flush {}: {e}", path.display()))
}

/// What the file's protection actually is, said rather than assumed.
#[cfg(unix)]
const fn permission_note(_path: &Path) -> &'static str {
    "mode 0600, set in the open rather than after it.\n\n"
}

/// **Windows has no mode and this does not pretend to set one.**
#[cfg(not(unix))]
const fn permission_note(_path: &Path) -> &'static str {
    "THE FILE INHERITS ITS DIRECTORY'S ACL. This tool sets no permission on Windows and\n\
     will not claim to. Check it, and remove inheritance if the directory is not already\n\
     private to you.\n\n"
}

/// What `cargo rivet key` was asked to do.
pub struct KeyArgs {
    /// The **private seed** to derive from. Read and never printed.
    pub seed: PathBuf,
    /// Where to write the public key, as 64 hex characters and a newline.
    pub out: Option<PathBuf>,
}

pub fn parse(args: &[String]) -> Result<KeyArgs, String> {
    let mut seed: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                let raw = args.get(i).ok_or("--out needs a path")?;
                out = Some(PathBuf::from(raw));
            }
            other if other.starts_with('-') => {
                return Err(format!("{other:?} is not an option of `key`"));
            }
            other => {
                if seed.is_some() {
                    return Err(format!(
                        "`key` takes one seed and was given a second, {other:?}"
                    ));
                }
                seed = Some(PathBuf::from(other));
            }
        }
        i += 1;
    }

    Ok(KeyArgs {
        seed: seed.ok_or(
            "`key` needs the path to a 32-byte private seed. It derives the PUBLIC key from it \
             and prints that; the seed itself is never printed and never written anywhere.",
        )?,
        out,
    })
}

/// Derive and publish the public half of a signing key.
///
/// **The seed never reaches the report, the terminal or a file.** What comes
/// back is the public key and the key id — the two things a person needs in
/// order to build an image that trusts this key and to recognise it in a
/// refusal.
pub fn run(args: &KeyArgs) -> Result<String, String> {
    // `new` is a WORD in the seed position rather than a flag, because it is a
    // different operation and not a modifier of this one: `key <seed>` reads
    // and `key new <path>` writes, and a `--new` would read as though the two
    // shared a subject. A file genuinely called `new` is reachable as `./new`.
    if args.seed.as_os_str() == "new" {
        let Some(out) = &args.out else {
            return Err(
                "`key new` needs the path to write the seed to:                  `cargo rivet key new --out keys/system.seed`. There is no default \
                 location and no environment variable — every one of those absences \
                 is a design decision nobody has taken (ADR-001 §7.5)."
                    .to_owned(),
            );
        };
        return new_seed(out);
    }

    let seed = read_32(&args.seed, "signing key")?;
    let signer = Ed25519Signer::from_seed(&seed);
    let public = signer.public_key();
    let key_id = Blake3::new().of(&public.0);

    let hex_key = hex(&public.0);
    let written = match &args.out {
        Some(path) => {
            if path.exists() {
                return Err(format!(
                    "{} already exists. A public key is written once and read forever; \
                     overwriting one silently is how an image comes to trust a key nobody \
                     holds. Move it aside if you mean to replace it.",
                    path.display()
                ));
            }
            std::fs::write(path, format!("{hex_key}\n"))
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            format!("written to  {}\n", path.display())
        }
        None => String::new(),
    };

    Ok(format!(
        "public key  {hex_key}\n\
         key id      {}\n\
         {written}\n\
         The key id is what an .rvt header names and what a refusal prints, so it is how a\n\
         board says WHICH key it wanted. Build an image that trusts this key with\n\
         rivet-image's --trusted-key <label>:<kinds>:<path> (ADR-028 §5.5).\n\
         \n\
         The seed itself was read and not printed. Never commit one (AGENTS.md §8.3).\n",
        hex(&key_id),
    ))
}

// A compile-time tie between this module's `32` and the format's, so the two
// cannot drift apart silently.
const _: () = assert!(PUBLIC_KEY_BYTES == 32);

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// RFC 8032 §7.1's first vector: this seed has this public key, and the
    /// answer comes from the RFC rather than from this project (`AGENTS.md` §4).
    const SEED: [u8; 32] = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];
    const PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

    fn scratch(name: &str) -> PathBuf {
        crate::scratch::dir(&format!("key-{name}"))
    }

    #[test]
    fn the_public_key_of_the_rfcs_seed_is_the_rfcs_public_key() {
        let dir = scratch("rfc");
        let seed = dir.join("seed");
        std::fs::write(&seed, SEED).expect("write");
        let report = run(&KeyArgs {
            seed,
            out: Some(dir.join("out.pub")),
        })
        .expect("a valid seed");
        assert!(report.contains(PUBLIC), "{report}");
        let written = std::fs::read_to_string(dir.join("out.pub")).expect("read");
        assert_eq!(written, format!("{PUBLIC}\n"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// **The seed must not appear in the output.** It is the one byte string in
    /// this tool that must never reach a terminal, a log or a file, and the
    /// check is cheap enough to be permanent.
    #[test]
    fn the_report_never_contains_the_seed() {
        let dir = scratch("secret");
        let seed = dir.join("seed");
        std::fs::write(&seed, SEED).expect("write");
        let report = run(&KeyArgs { seed, out: None }).expect("a valid seed");
        assert!(!report.contains(&hex(&SEED)), "the seed was printed");
        assert!(report.contains("not printed"), "{report}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A public key is written once. Overwriting one silently is how an image
    /// comes to trust a key nobody holds.
    #[test]
    fn an_existing_public_key_is_refused_rather_than_overwritten() {
        let dir = scratch("exists");
        let seed = dir.join("seed");
        std::fs::write(&seed, SEED).expect("write");
        let out = dir.join("out.pub");
        std::fs::write(&out, "something else\n").expect("write");
        let refusal = run(&KeyArgs {
            seed,
            out: Some(out.clone()),
        })
        .expect_err("an existing file was overwritten");
        assert!(refusal.contains("already exists"), "{refusal}");
        assert_eq!(
            std::fs::read_to_string(&out).expect("read"),
            "something else\n"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A hex seed file works exactly as a raw one does — the same key material
    /// rule as everywhere else.
    #[test]
    fn a_hex_seed_gives_the_same_public_key_as_a_raw_one() {
        let dir = scratch("hex");
        let seed = dir.join("seed.hex");
        std::fs::write(&seed, format!("{}\n", hex(&SEED))).expect("write");
        let report = run(&KeyArgs { seed, out: None }).expect("a hex seed");
        assert!(report.contains(PUBLIC), "{report}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_seed_argument_is_refused_and_says_what_it_wanted() {
        let refusal = parse(&[]).err().expect("no seed was accepted");
        assert!(refusal.contains("32-byte"), "{refusal}");
        let refusal = parse(&["a".to_owned(), "b".to_owned()])
            .err()
            .expect("two seeds were accepted");
        assert!(refusal.contains("second"), "{refusal}");
    }

    // -- `key new`, ADR-001 §7.4 item 1 ------------------------------------

    /// A path in a fresh directory that does NOT exist yet — `key new` refuses
    /// an existing file, so every test here needs a name nothing has written.
    fn unwritten(name: &str) -> PathBuf {
        scratch(&format!("new-{name}")).join("system.seed")
    }

    #[test]
    fn key_new_writes_thirty_two_bytes_from_the_operating_system() {
        let path = unwritten("writes");
        let report = new_seed(&path).expect("the OS supplied bytes");
        let bytes = std::fs::read(&path).expect("the seed is on disk");
        assert_eq!(bytes.len(), 32, "a seed is 32 bytes and nothing else");
        // All-zero is what a buffer that was never filled looks like, and it
        // is a seed that would sign perfectly well and be trivially forged.
        assert_ne!(bytes, vec![0u8; 32], "the buffer was not filled");
        assert!(report.contains("public key"), "{report}");
    }

    /// **The refusal that matters.** Every board trusting the public half would
    /// refuse every future update, and there is no backup by design.
    #[test]
    fn key_new_refuses_to_overwrite_an_existing_seed() {
        let path = unwritten("overwrite");
        new_seed(&path).expect("the first one is written");
        let before = std::fs::read(&path).expect("it is there");

        let err = new_seed(&path).expect_err("a second call must refuse");
        assert!(err.contains("will not overwrite"), "{err}");
        assert_eq!(
            std::fs::read(&path).expect("still there"),
            before,
            "the refusal must not have touched the file"
        );
    }

    /// It is randomness, not a constant. Two seeds from two calls differ —
    /// which is the one property a stubbed-out implementation would fail.
    #[test]
    fn two_seeds_are_not_the_same_seed() {
        let a = unwritten("a");
        let b = unwritten("b");
        new_seed(&a).expect("a");
        new_seed(&b).expect("b");
        assert_ne!(
            std::fs::read(&a).expect("a"),
            std::fs::read(&b).expect("b"),
            "two calls produced identical seeds"
        );
    }

    /// **The seed never reaches the report.** Not the terminal, not a log, not
    /// an error path — the whole point of deriving the public half here is
    /// that the private half has nowhere to leak to.
    #[test]
    fn the_seed_itself_is_never_in_the_answer() {
        let path = unwritten("secret");
        let report = new_seed(&path).expect("written");
        let bytes = std::fs::read(&path).expect("the seed");
        assert!(!report.contains(&hex(&bytes)), "the seed was printed as hex");
        // And no run of its bytes, in case a future edit prints it another way.
        for window in bytes.windows(8) {
            assert!(!report.contains(&hex(window)), "part of the seed was printed");
        }
    }

    /// The seed a fresh `key new` writes is one `key` can read back, and both
    /// paths agree about the public half.
    #[test]
    fn a_generated_seed_derives_the_same_public_key_through_the_read_path() {
        let path = unwritten("roundtrip");
        let report = new_seed(&path).expect("written");
        let read_back = run(&KeyArgs {
            seed: path,
            out: None,
        })
        .expect("the seed reads back");

        let public = read_back
            .lines()
            .find(|l| l.starts_with("public key"))
            .expect("a public key line");
        assert!(report.contains(public.trim()), "{report}\n---\n{read_back}");
    }
}
