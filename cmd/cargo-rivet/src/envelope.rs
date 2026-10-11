//! **The passphrase envelope** — one format, one implementation, for every
//! backup this tool encrypts under a typed passphrase. `.rulings/` R-183 D3
//! ruled it for the two signing seeds (`crate::key_backup`); R-185 D1 rules the
//! same format and the same typed prompt for a board's secrets
//! (`crate::machine_secrets`). `ADR-009` §3 holds the crate survey underneath.
//!
//! # The layout, versioned and self-describing
//!
//! ```text
//! offset  bytes  field
//! 0       7      magic — names WHAT the envelope holds (see below)
//! 7       1      format version — 1
//! 8       1      KDF id — 1 = Argon2id (RFC 9106)
//! 9       4      Argon2 m_cost, KiB, little-endian
//! 13      4      Argon2 t_cost, little-endian
//! 17      4      Argon2 p_cost, little-endian
//! 21      1      salt length
//! 22      *      salt
//! *       1      cipher id — 1 = XChaCha20-Poly1305
//! *       1      nonce length
//! *       *      nonce
//! *       4      ciphertext length, little-endian
//! *       *      ciphertext (the plaintext, sealed)
//! ```
//!
//! This is byte for byte the layout `cargo rivet key backup` wrote before this
//! module existed, when the code lived inside `key_backup.rs`; a test below
//! opens an envelope that code produced, so a seed backup taken before the
//! hoist stays readable.
//!
//! **Every field up to and including the nonce is the AEAD's associated data**
//! — authenticated but not secret, so a corrupted or hand-edited parameter is
//! refused the same way a wrong passphrase is, rather than silently deriving a
//! different key from tampered salt or Argon2 costs.
//!
//! # A caller chooses the magic, and nothing else
//!
//! A seed backup is `RVTSEED`; a board's secrets backup is `RVTSECR`. The magic
//! sits inside the associated data, so it is authenticated like every other
//! header field — and a secrets backup handed to `key restore` (or a seed
//! backup handed to `restore-secrets`) is refused by name as the wrong kind of
//! file before anyone is asked for a passphrase. Everything else — the KDF, its
//! costs, the cipher — is fixed here, for every caller.
//!
//! # What a wrong passphrase and a tampered file both look like
//!
//! **Identical, on purpose.** An AEAD's whole security property is that
//! authentication failure does not distinguish "the key was wrong" from "the
//! ciphertext was altered" — telling them apart would leak which one happened,
//! an oracle a real attacker could use. Both are `OpenError::WrongPassphraseOrTampered`.
//!
//! # The passphrase, never on the command line
//!
//! Read with no echo, and only from a real terminal — no `--passphrase` flag
//! and no environment variable exist here, and none should be added without a
//! ruling: a flag or a variable puts a high-value secret in a process's argv or
//! its environment block, both readable by anything else running as the same
//! user. Without a terminal this refuses by name rather than silently falling
//! back to a pipe that was never asked for.

use std::io::IsTerminal as _;

use argon2::Argon2;
use chacha20poly1305::aead::{Aead as _, Payload};
use chacha20poly1305::{KeyInit as _, XChaCha20Poly1305, XNonce};

/// The magic's width. Every caller's magic is exactly this long.
pub const MAGIC_LEN: usize = 7;

const VERSION: u8 = 1;
const KDF_ARGON2ID: u8 = 1;
const CIPHER_XCHACHA20POLY1305: u8 = 1;

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;

/// RFC 9106 §4's second recommended Argon2id setting — *"if much memory is not
/// available"* — chosen over the first (2 GiB) because this runs once, by hand,
/// on whatever workstation the operator is holding, and 64 MiB is a trivial
/// cost there either way. Fixed rather than tunable: a backup file with weaker
/// parameters than the format most recently chose is exactly what
/// "self-describing" protects against, and a knob nobody has a reason to turn
/// is `AGENTS.md` §5's problem waiting to happen.
const ARGON2_M_COST_KIB: u32 = 65_536;
const ARGON2_T_COST: u32 = 3;
const ARGON2_P_COST: u32 = 4;

/// The most any header may ask for before [`parse`] refuses it. The costs are
/// read before anything authenticates them, so without a ceiling a hand-edited
/// file could name gigabytes of memory and exhaust the workstation instead of
/// being refused. Every file this format writes carries exactly the costs above;
/// the ceilings leave room for a future format to raise them, and no more.
const ARGON2_M_COST_CEILING_KIB: u32 = 1_048_576;
const ARGON2_T_COST_CEILING: u32 = 16;
const ARGON2_P_COST_CEILING: u32 = 16;

/// The sentence every encrypting verb prints, before the prompt and after the
/// write. R-183 D3 and R-185 D1 both rule that the tool says it.
pub const FORGOTTEN: &str =
    "A FORGOTTEN PASSPHRASE MEANS THIS BACKUP CANNOT BE RESTORED. There is no recovery route \
     around it.";

/// What one kind of envelope holds: its magic, and how a refusal names it.
#[derive(Debug, Clone, Copy)]
pub struct Kind {
    pub magic: &'static [u8; MAGIC_LEN],
    /// A noun phrase for refusals, e.g. `"a Rivet seed backup file"`.
    pub noun: &'static str,
}

// ---------------------------------------------------------------------------
// Sealing
// ---------------------------------------------------------------------------

/// Encrypt `plaintext` under `passphrase`, with fresh random salt and nonce —
/// `getrandom`, the same call `cargo rivet key new` already makes to mint a
/// seed. **This is not that.** `ADR-028`'s note that this project takes no
/// randomness source so that a tool can save a person a shell command is about
/// *minting key material* — the thing a seed IS. A salt and a nonce are
/// encryption *parameters*: neither is secret, neither is signed with, and
/// using a fixed or reused one is the actual hazard (a reused nonce breaks the
/// AEAD outright).
///
/// # Errors
///
/// The operating system would not supply randomness; a plaintext larger than
/// the 32-bit length field can state.
pub fn seal(kind: Kind, plaintext: &[u8], passphrase: &str) -> Result<Vec<u8>, String> {
    let mut salt = [0u8; SALT_LEN];
    getrandom::fill(&mut salt)
        .map_err(|e| format!("the operating system would not supply a random salt: {e}"))?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce_bytes)
        .map_err(|e| format!("the operating system would not supply a random nonce: {e}"))?;

    let key = derive_key(passphrase, &salt, ARGON2_M_COST_KIB, ARGON2_T_COST, ARGON2_P_COST)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let nonce = XNonce::from_slice(&nonce_bytes);

    let mut header = Vec::with_capacity(64);
    header.extend_from_slice(kind.magic);
    header.push(VERSION);
    header.push(KDF_ARGON2ID);
    header.extend_from_slice(&ARGON2_M_COST_KIB.to_le_bytes());
    header.extend_from_slice(&ARGON2_T_COST.to_le_bytes());
    header.extend_from_slice(&ARGON2_P_COST.to_le_bytes());
    #[allow(clippy::cast_possible_truncation)] // SALT_LEN is a fixed 16.
    header.push(SALT_LEN as u8);
    header.extend_from_slice(&salt);
    header.push(CIPHER_XCHACHA20POLY1305);
    #[allow(clippy::cast_possible_truncation)] // NONCE_LEN is a fixed 24.
    header.push(NONCE_LEN as u8);
    header.extend_from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, Payload { msg: plaintext, aad: &header })
        .map_err(|_| "encryption failed".to_owned())?;
    let length = u32::try_from(ciphertext.len()).map_err(|_| {
        format!(
            "{} bytes is more than one envelope can carry",
            plaintext.len()
        )
    })?;

    let mut out = header;
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

fn derive_key(passphrase: &str, salt: &[u8], m: u32, t: u32, p: u32) -> Result<[u8; KEY_LEN], String> {
    let params = argon2::Params::new(m, t, p, Some(KEY_LEN))
        .map_err(|e| format!("invalid Argon2 parameters: {e}"))?;
    let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = [0u8; KEY_LEN];
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| format!("key derivation failed: {e}"))?;
    Ok(key)
}

// ---------------------------------------------------------------------------
// Opening
// ---------------------------------------------------------------------------

/// Why an envelope did not open.
#[derive(Debug, PartialEq, Eq)]
pub enum OpenError {
    /// Not a passphrase problem — the file itself is not this kind, is
    /// truncated, or names a version/KDF/cipher this build does not speak.
    Format(String),
    /// The AEAD refused to open. Either the passphrase is wrong or the file
    /// was altered since it was written; an AEAD's own security property is
    /// that those two are indistinguishable (see the module doc).
    WrongPassphraseOrTampered,
}

impl core::fmt::Display for OpenError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Format(why) => write!(f, "{why}"),
            Self::WrongPassphraseOrTampered => write!(
                f,
                "wrong passphrase, or this file has been altered since it was written — refused. \
                 {FORGOTTEN}"
            ),
        }
    }
}

/// An envelope whose header parsed, before it is opened. [`parse`] needs no
/// passphrase, so a caller can refuse a file that is not this kind at all
/// before asking anyone to type one.
#[derive(Debug)]
pub struct Parsed<'a> {
    header: &'a [u8],
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
    salt: &'a [u8],
    nonce: &'a [u8],
    ciphertext: &'a [u8],
}

/// Read the next `n` bytes of `file` starting at `*at`, advancing `*at` past
/// them. A plain function rather than a closure over `at`: a closure that both
/// reads and mutates a captured `at` cannot also hand back a borrow of `file`
/// that outlives the next call.
fn take<'a>(file: &'a [u8], at: &mut usize, n: usize) -> Result<&'a [u8], OpenError> {
    let truncated = || OpenError::Format("the file is truncated".to_owned());
    let end = at.checked_add(n).ok_or_else(truncated)?;
    let slice = file.get(*at..end).ok_or_else(truncated)?;
    *at = end;
    Ok(slice)
}

/// Parse `file`'s header as an envelope of `kind`.
///
/// # Errors
///
/// [`OpenError::Format`] for wrong magic, a truncated file, bytes after the
/// ciphertext, or an unsupported version, KDF or cipher.
pub fn parse(kind: Kind, file: &[u8]) -> Result<Parsed<'_>, OpenError> {
    let mut at = 0usize;
    if take(file, &mut at, MAGIC_LEN)? != kind.magic {
        return Err(OpenError::Format(format!("this is not {} (bad magic)", kind.noun)));
    }
    let version = take(file, &mut at, 1)?[0];
    if version != VERSION {
        return Err(OpenError::Format(format!(
            "this backup was written in format version {version}, and this build only reads \
             version {VERSION}"
        )));
    }
    let kdf_id = take(file, &mut at, 1)?[0];
    if kdf_id != KDF_ARGON2ID {
        return Err(OpenError::Format(format!(
            "this backup names KDF id {kdf_id}, which this build does not implement"
        )));
    }
    let u32_at = |at: &mut usize| -> Result<u32, OpenError> {
        let bytes: [u8; 4] = take(file, at, 4)?.try_into().unwrap_or_default();
        Ok(u32::from_le_bytes(bytes))
    };
    let m_cost = u32_at(&mut at)?;
    let t_cost = u32_at(&mut at)?;
    let p_cost = u32_at(&mut at)?;
    if m_cost > ARGON2_M_COST_CEILING_KIB
        || t_cost > ARGON2_T_COST_CEILING
        || p_cost > ARGON2_P_COST_CEILING
    {
        return Err(OpenError::Format(format!(
            "this backup asks for Argon2 costs (memory {m_cost} KiB, time {t_cost}, parallelism \
             {p_cost}) above what this format allows (memory {ARGON2_M_COST_CEILING_KIB} KiB, \
             time {ARGON2_T_COST_CEILING}, parallelism {ARGON2_P_COST_CEILING}), so it was not \
             written by this tool — refused before deriving any key"
        )));
    }
    let salt_len = take(file, &mut at, 1)?[0] as usize;
    let salt = take(file, &mut at, salt_len)?;
    let cipher_id = take(file, &mut at, 1)?[0];
    if cipher_id != CIPHER_XCHACHA20POLY1305 {
        return Err(OpenError::Format(format!(
            "this backup names cipher id {cipher_id}, which this build does not implement"
        )));
    }
    let nonce_len = take(file, &mut at, 1)?[0] as usize;
    let nonce = take(file, &mut at, nonce_len)?;
    if nonce.len() != NONCE_LEN {
        return Err(OpenError::Format(
            "this backup's nonce length is wrong for its own cipher id".to_owned(),
        ));
    }
    let header_end = at;
    let ciphertext_len = u32_at(&mut at)? as usize;
    let ciphertext = take(file, &mut at, ciphertext_len)?;
    if at != file.len() {
        return Err(OpenError::Format(format!(
            "the file carries {} byte(s) after its ciphertext, which this format never writes",
            file.len() - at
        )));
    }
    Ok(Parsed {
        header: &file[..header_end],
        m_cost,
        t_cost,
        p_cost,
        salt,
        nonce,
        ciphertext,
    })
}

impl Parsed<'_> {
    /// Derive the key from `passphrase` and this file's own salt and costs, and
    /// open the ciphertext.
    ///
    /// # Errors
    ///
    /// [`OpenError::Format`] when the file's own Argon2 parameters are invalid;
    /// [`OpenError::WrongPassphraseOrTampered`] when the AEAD will not open.
    pub fn open(&self, passphrase: &str) -> Result<Vec<u8>, OpenError> {
        let key = derive_key(passphrase, self.salt, self.m_cost, self.t_cost, self.p_cost)
            .map_err(|e| OpenError::Format(format!("this backup's key derivation failed: {e}")))?;
        let cipher = XChaCha20Poly1305::new((&key).into());
        let nonce = XNonce::from_slice(self.nonce);
        cipher
            .decrypt(nonce, Payload { msg: self.ciphertext, aad: self.header })
            .map_err(|_| OpenError::WrongPassphraseOrTampered)
    }
}

/// [`parse`] then [`Parsed::open`], for a caller that has the passphrase
/// already.
///
/// # Errors
///
/// Either's.
pub fn open(kind: Kind, file: &[u8], passphrase: &str) -> Result<Vec<u8>, OpenError> {
    parse(kind, file)?.open(passphrase)
}

// ---------------------------------------------------------------------------
// The terminal
// ---------------------------------------------------------------------------

fn refuse_without_terminal(verb: &str) -> Result<(), String> {
    if std::io::stdin().is_terminal() {
        return Ok(());
    }
    Err(format!(
        "this run has no terminal to ask a passphrase at, so nothing was written and nothing was \
         read from stdin. Run `{verb}` from an interactive terminal. There is no --passphrase \
         flag and no environment variable — a high-value secret does not belong in a process's \
         argv or its environment block."
    ))
}

/// Say [`FORGOTTEN`], then read a new passphrase twice with no echo, and refuse
/// if the two do not match, it is empty, or there is no terminal to ask at.
/// `what` names the backup in the prompt (`"seed backup"`); `verb` names the
/// command in the no-terminal refusal.
///
/// # Errors
///
/// No real terminal (stdin is a pipe, a redirect, a CI runner); the two
/// entries differ; an empty entry; or the terminal itself could not be read.
pub fn read_new(what: &str, verb: &str) -> Result<String, String> {
    refuse_without_terminal(verb)?;
    eprintln!("{FORGOTTEN}");
    let first = rpassword::prompt_password(format!("passphrase for this {what}: "))
        .map_err(|e| format!("cannot read the passphrase: {e}"))?;
    let second = rpassword::prompt_password("type it again to confirm: ")
        .map_err(|e| format!("cannot read the passphrase: {e}"))?;
    if first != second {
        return Err("the two entries did not match. Nothing was written.".to_owned());
    }
    if first.is_empty() {
        return Err("an empty passphrase protects nothing. Nothing was written.".to_owned());
    }
    Ok(first)
}

/// Read the passphrase once, for a restore — there is nothing to confirm
/// against here, only the file's own AEAD tag.
///
/// # Errors
///
/// No real terminal, or the terminal could not be read.
pub fn read_existing(what: &str, verb: &str) -> Result<String, String> {
    refuse_without_terminal(verb)?;
    rpassword::prompt_password(format!("passphrase for this {what}: "))
        .map_err(|e| format!("cannot read the passphrase: {e}"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    const KIND: Kind = Kind { magic: b"RVTTEST", noun: "a test envelope" };
    const OTHER: Kind = Kind { magic: b"RVTOTHR", noun: "another test envelope" };
    const A_PASSPHRASE: &str = "correct horse battery staple, fictional and fixed";
    const A_DIFFERENT_PASSPHRASE: &str = "a different fictional passphrase entirely";

    /// **An envelope `key_backup.rs` wrote BEFORE this module existed**, at
    /// commit `730c8725`: its own `encrypt` sealing the 32 bytes `0, 1, … 31`
    /// under the passphrase below. If this stops opening, every seed backup an
    /// operator already holds has stopped opening with it.
    const PRE_HOIST_SEED_ENVELOPE: &str = "525654534545440101000001000300000004000000106e094d30087a5c1c81a3d88cce1c2c400118a800a772e677d31a6420d05430f86775d6c73805f1c2890930000000ef9c7f2dd760dc0d566643bdb237fdd9b0bad10fe5905ec4be61b03361bfffd6ad1ade5ffccc1725802a06b96945d3df";
    const PRE_HOIST_PASSPHRASE: &str = "pre-hoist test vector, fictional";

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
            .collect()
    }

    #[test]
    fn an_envelope_written_before_the_hoist_still_opens() {
        let file = unhex(PRE_HOIST_SEED_ENVELOPE);
        let seed = Kind { magic: b"RVTSEED", noun: "a Rivet seed backup file" };
        let opened = open(seed, &file, PRE_HOIST_PASSPHRASE).expect("the pre-hoist envelope opens");
        let expected: Vec<u8> = (0u8..32).collect();
        assert_eq!(opened, expected);
    }

    /// **And this build writes the same layout**: the header bytes up to the
    /// salt, which carry no randomness, are identical to the pre-hoist file's.
    #[test]
    fn this_build_writes_the_pre_hoist_header() {
        let old = unhex(PRE_HOIST_SEED_ENVELOPE);
        let seed = Kind { magic: b"RVTSEED", noun: "a Rivet seed backup file" };
        let new = seal(seed, &[0u8; 32], "anything").expect("seals");
        assert_eq!(new.len(), old.len(), "a 32-byte plaintext makes the same size of file");
        assert_eq!(new[..22], old[..22], "magic, version, KDF, costs and salt length");
    }

    #[test]
    fn a_plaintext_of_any_length_round_trips() {
        for len in [0usize, 1, 32, 4096, 70_000] {
            let plaintext: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let envelope = seal(KIND, &plaintext, A_PASSPHRASE).expect("seals");
            assert_eq!(open(KIND, &envelope, A_PASSPHRASE).expect("opens"), plaintext, "{len}");
        }
    }

    #[test]
    fn two_seals_of_the_same_plaintext_differ() {
        let a = seal(KIND, b"same", A_PASSPHRASE).expect("seals");
        let b = seal(KIND, b"same", A_PASSPHRASE).expect("seals");
        assert_ne!(a, b, "salt and nonce must be fresh every time");
    }

    #[test]
    fn a_wrong_passphrase_and_a_flipped_ciphertext_byte_refuse_identically() {
        let envelope = seal(KIND, b"a secret", A_PASSPHRASE).expect("seals");
        let wrong = open(KIND, &envelope, A_DIFFERENT_PASSPHRASE).expect_err("wrong passphrase");
        let mut tampered = envelope.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0xFF;
        let altered = open(KIND, &tampered, A_PASSPHRASE).expect_err("tampered");
        assert_eq!(wrong, OpenError::WrongPassphraseOrTampered);
        assert_eq!(wrong, altered);
        assert_eq!(wrong.to_string(), altered.to_string());
        assert!(wrong.to_string().contains("FORGOTTEN PASSPHRASE"), "{wrong}");
    }

    /// Flipping a byte of the salt — authenticated, not secret — must not
    /// silently derive a different key and appear to work.
    #[test]
    fn a_tampered_salt_byte_is_refused() {
        let mut envelope = seal(KIND, b"a secret", A_PASSPHRASE).expect("seals");
        envelope[22] ^= 0xFF; // 7+1+1+4+4+4+1: the first salt byte.
        assert_eq!(open(KIND, &envelope, A_PASSPHRASE), Err(OpenError::WrongPassphraseOrTampered));
    }

    #[test]
    fn a_header_naming_costs_above_the_ceiling_is_refused_before_any_key_is_derived() {
        // Offsets: 7 magic + 1 version + 1 KDF = 9 for m_cost, 13 for t_cost, 17 for p_cost.
        for offset in [9usize, 13, 17] {
            let mut envelope = seal(KIND, b"a secret", A_PASSPHRASE).expect("seals");
            envelope[offset..offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            match parse(KIND, &envelope) {
                Err(OpenError::Format(why)) => assert!(why.contains("above what this format allows"), "{why}"),
                other => panic!("a cost of u32::MAX at offset {offset} gave {other:?}"),
            }
        }
    }

    #[test]
    fn another_kind_of_envelope_is_a_format_refusal_naming_the_kind() {
        let envelope = seal(OTHER, b"a secret", A_PASSPHRASE).expect("seals");
        match parse(KIND, &envelope) {
            Err(OpenError::Format(why)) => assert!(why.contains("not a test envelope"), "{why}"),
            other => panic!("expected a format refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_truncated_file_is_a_format_refusal_at_every_length() {
        let envelope = seal(KIND, b"a secret of some length", A_PASSPHRASE).expect("seals");
        for len in 0..envelope.len() {
            match parse(KIND, &envelope[..len]) {
                Err(OpenError::Format(_)) => {}
                other => panic!("a buffer truncated to {len} gave {other:?}"),
            }
        }
    }

    #[test]
    fn bytes_after_the_ciphertext_are_a_format_refusal() {
        let mut envelope = seal(KIND, b"a secret", A_PASSPHRASE).expect("seals");
        envelope.push(0);
        match parse(KIND, &envelope) {
            Err(OpenError::Format(why)) => assert!(why.contains("after its ciphertext"), "{why}"),
            other => panic!("expected a format refusal, got {other:?}"),
        }
    }

    #[test]
    fn an_unsupported_version_is_refused_by_name() {
        let mut envelope = seal(KIND, b"a secret", A_PASSPHRASE).expect("seals");
        envelope[MAGIC_LEN] = 99;
        match parse(KIND, &envelope) {
            Err(OpenError::Format(why)) => assert!(why.contains("version 99"), "{why}"),
            other => panic!("expected a format refusal, got {other:?}"),
        }
    }

    /// Without a terminal on stdin both prompts refuse — by name, and before
    /// reading anything. Skipped when somebody runs the suite from a real
    /// terminal, where the prompt would sit waiting for them.
    #[test]
    fn without_a_terminal_both_prompts_refuse_by_name() {
        if std::io::stdin().is_terminal() {
            return;
        }
        let refused = read_new("test backup", "cargo rivet test").expect_err("no terminal");
        assert!(refused.contains("no terminal") && refused.contains("cargo rivet test"), "{refused}");
        let refused = read_existing("test backup", "cargo rivet test").expect_err("no terminal");
        assert!(refused.contains("no --passphrase flag"), "{refused}");
    }
}
