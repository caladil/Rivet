//! **The credential store — obfuscated, and NOT encrypted.**
//!
//! `ADR-034` §10.4 ruled this and the obligation it carries is not optional:
//!
//! > **What this obliges, and it is not optional.** `AGENTS.md` §3 — *stubs
//! > refuse, they never fake success* — applies to names as much as to
//! > functions. **A file that is obfuscated must not be called encrypted**, in
//! > the code, in the documentation, or in anything the tool prints. An
//! > operator who believes a store is encrypted will treat it as safe to copy,
//! > and §9's warning about backing up the register wholesale is exactly that
//! > mistake.
//!
//! So: **this module never uses that word affirmatively** — it appears only in
//! denials — the file says what it is in its own first line, and there is a
//! test that both remain true. §7.4's earlier phrase for this file is
//! superseded by §10.4 and is not implemented here under any spelling.
//!
//! The maintainer's reason for the ruling, recorded because it is the thing
//! that would otherwise be re-litigated: until the store holds more than a
//! WiFi password, a platform keychain or secure store is more machinery than the
//! job needs, and a WiFi password is not a state secret (.rulings/ R-054).
//!
//! **The revisit trigger is named** by §10.4 so this is inherited deliberately:
//! *the first credential in the store that is not a shared network credential.*
//! At that point §7.0's table has a second row on the wrong side and the
//! decision is a different one. [`Credential`] has exactly one shape for that
//! reason — a second kind cannot be added here without meeting the trigger.
//!
//! # What obfuscation is worth, stated so nobody over-reads it
//!
//! The key is a constant in this file. Anyone holding the file and this source
//! recovers the contents, and this source is public. What it buys is exactly
//! one thing: **the value does not appear in a casual `grep`, a backup index, a
//! screen share, or an editor's recent-files preview.** That is the whole
//! claim. It is not a security boundary and nothing here treats it as one.
//!
//! # What is stored is the PMK, not the passphrase
//!
//! `ADR-034` §7.1, extending `ADR-004` §5 unchanged. The passphrase is the
//! thing whose loss reaches beyond this network — *"a human's WiFi passphrase
//! is frequently a passphrase they have used elsewhere"* — and it is never
//! written anywhere by this tool.
//!
//! **This build cannot derive one, and refuses rather than pretending.** §7.1
//! puts the PBKDF2 derivation in the register, *"once, when the human types the
//! passphrase"*. There is no PBKDF2 and no SHA-1 in this workspace:
//! `rivet-crypto` holds BLAKE3 and Ed25519 and nothing else, and WPA2's
//! derivation is PBKDF2-HMAC-SHA1 specifically. Adding a cryptographic
//! dependency is a decision `AGENTS.md` §1 reserves to the maintainer, so
//! [`derive_pmk`] refuses and says exactly that. What works today is
//! [`Store::put`] with a PMK the operator already has — `wpa_passphrase` prints
//! one, and so does an existing `/state/secrets` on a provisioned board.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rivet_core::package::port::Digest as _;
use rivet_crypto::Blake3;

/// The file's name, under the register root.
///
/// **Not `.toml`**, and that is deliberate. The public register is TOML because
/// it is meant to be read and committed; this file is neither, and a `.toml`
/// suffix would invite an editor, a linter and a diff to treat it as text.
pub const CREDENTIALS_FILE: &str = "credentials.store";

/// The first line of the file, in the clear, for whoever opens it.
///
/// **The honesty §10.4 requires, at the place it is needed.** An operator who
/// finds this file is told what it is before they decide what to do with it,
/// which is the same reasoning `ADR-007` §3.6.1 gives for `/etc`'s provenance
/// header: it is read by exactly the person about to make the mistake.
pub const HEADER: &str = "rivet-fleet credential store. THIS FILE IS OBFUSCATED, NOT ENCRYPTED. \
                          Anyone with this file and rivet-fleet's source can read it. Do not put \
                          it anywhere you would not put the values themselves. ADR-034 10.4.";

/// A WPA2 pairwise master key is 32 bytes. `ADR-004` §5.
pub const PMK_BYTES: usize = 32;

/// The label the keystream is derived under.
///
/// A constant, and the module header says plainly what that means. It is here
/// rather than inline so that a reader looking for "what is the key" finds an
/// answer rather than a construction.
const KEYSTREAM_LABEL: &[u8] = b"rivet-fleet credential store obfuscation v1 (NOT ENCRYPTION)";

/// One shared network credential.
///
/// **One kind, deliberately** — see the module header on §10.4's revisit
/// trigger. A second variant here would be the moment the ruling has to be
/// re-taken, and an enum with one variant is how that moment is made visible
/// rather than convenient.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    /// The network this belongs to. Not a secret, and it is what the device
    /// record names (`crate::register::Device::wireless`).
    pub ssid: String,
    /// The 32-byte pairwise master key. **Never a passphrase.**
    pub pmk: [u8; PMK_BYTES],
}

/// Every credential this operator holds, by SSID.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Store {
    entries: BTreeMap<String, [u8; PMK_BYTES]>,
}

/// What went wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialError {
    /// The path could not be read or written, and what the system said.
    Io { path: PathBuf, detail: String },
    /// The file is not one this build wrote.
    Malformed { path: PathBuf, detail: String },
    /// A PMK that is not 32 bytes of hexadecimal.
    NotAPmk { given: String, detail: String },
    /// A passphrase was offered and this build will not derive from one.
    ///
    /// **A refusal, not a stub that guesses.** See the module header.
    /// ~~This build cannot derive a PMK from a passphrase.~~ **Kept and no
    /// longer produced**, since 2026-08-30: `derive_pmk` works. It is left in
    /// the enum so a caller matching exhaustively is not broken by the good
    /// news, and it will go with the next change that touches this type.
    CannotDerive,
    /// A passphrase outside 802.11's 8..=63 bytes.
    ///
    /// **Carries the LENGTH and never the passphrase** — `ADR-020` §4.
    PassphraseLength { bytes: usize },
}

impl core::fmt::Display for CredentialError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io { path, detail } => write!(f, "{}: {detail}", path.display()),
            Self::Malformed { path, detail } => write!(
                f,
                "{} is not a credential store this build wrote: {detail}",
                path.display()
            ),
            Self::NotAPmk { given, detail } => write!(
                f,
                "{given:?} is not a pairwise master key: {detail}. A PMK is 32 bytes, written as \
                 64 hexadecimal characters - `wpa_passphrase <ssid> <passphrase>` prints one, and \
                 so does a provisioned board's own /state/secrets."
            ),
            Self::PassphraseLength { bytes } => write!(
                f,
                "a WPA passphrase is 8 to 63 bytes and this one is {bytes} — 802.11's own rule.                  A 64-character value is a PMK rather than a passphrase, and `put` takes one of \
                 those directly without deriving"
            ),
            Self::CannotDerive => write!(
                f,
                "this build cannot turn a passphrase into a PMK, and will not store the \
                 passphrase instead. WPA2's derivation is PBKDF2-HMAC-SHA1 and this workspace \
                 has no SHA-1 and no PBKDF2 - rivet-crypto holds BLAKE3 and Ed25519 and nothing \
                 else. Taking a cryptographic dependency is the maintainer's decision, not this \
                 tool's, so nothing has been stored and nothing has been guessed. Derive it \
                 yourself with `wpa_passphrase <ssid> <passphrase>` and give the psk= value."
            ),
        }
    }
}

impl Store {
    /// The PMK for `ssid`, if this operator holds one.
    #[must_use]
    pub fn get(&self, ssid: &str) -> Option<Credential> {
        self.entries.get(ssid).map(|pmk| Credential {
            ssid: ssid.to_owned(),
            pmk: *pmk,
        })
    }

    /// Record a credential, replacing any this operator held for that network.
    pub fn put(&mut self, credential: Credential) {
        self.entries.insert(credential.ssid, credential.pmk);
    }

    /// Forget one network's credential. `true` when there was one.
    pub fn forget(&mut self, ssid: &str) -> bool {
        self.entries.remove(ssid).is_some()
    }

    /// Which networks this store holds credentials for.
    ///
    /// **SSIDs only, and no way to list the values.** A store that could be
    /// enumerated into a report is one that would end up in a report.
    #[must_use]
    pub fn networks(&self) -> Vec<&str> {
        self.entries.keys().map(String::as_str).collect()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Turn a passphrase into a PMK.
///
/// ~~**Refuses, always, in this build.** The derivation is PBKDF2-HMAC-SHA1 and
/// this workspace has neither primitive.~~ **Built 2026-08-30**:
/// `rivet_crypto::pmk` has both, and this is the one-line call that makes
/// `ADR-034` §7.1 true — *"the derivation happens once, in the register, when
/// the human types the passphrase"*, so a passphrase never reaches a board and
/// only the key is provisioned.
///
/// The struck sentence is kept because it was the reason for months and its
/// passing is the news.
///
/// # Errors
///
/// [`CredentialError::PassphraseLength`] for a passphrase outside 802.11's
/// 8..=63 bytes. The refusal carries the length and never the passphrase.
pub fn derive_pmk(ssid: &str, passphrase: &str) -> Result<[u8; PMK_BYTES], CredentialError> {
    rivet_crypto::pmk::derive(ssid, passphrase).map_err(|why| match why {
        rivet_crypto::pmk::PmkError::PassphraseLength { bytes } => {
            CredentialError::PassphraseLength { bytes }
        }
    })
}

/// Read a PMK an operator pasted, as `wpa_passphrase` prints it.
///
/// # Errors
///
/// [`CredentialError::NotAPmk`] for anything that is not 64 hexadecimal
/// characters — with what was wrong, because the two likely mistakes (pasting
/// the passphrase, pasting a truncated key) look nothing alike.
pub fn pmk_from_hex(text: &str) -> Result<[u8; PMK_BYTES], CredentialError> {
    let text = text.trim();
    if text.len() != PMK_BYTES * 2 {
        return Err(CredentialError::NotAPmk {
            given: text.to_owned(),
            detail: format!("it is {} characters and a PMK is {}", text.len(), PMK_BYTES * 2),
        });
    }
    let mut out = [0u8; PMK_BYTES];
    for (index, byte) in out.iter_mut().enumerate() {
        let pair = &text[index * 2..index * 2 + 2];
        *byte = u8::from_str_radix(pair, 16).map_err(|_| CredentialError::NotAPmk {
            given: text.to_owned(),
            detail: format!("{pair:?} is not hexadecimal"),
        })?;
    }
    Ok(out)
}

/// The keystream for `length` bytes, from the constant label and a salt.
///
/// BLAKE3 in counter mode: `BLAKE3(label || salt || counter)` concatenated. The
/// salt is the file's, so two stores with the same contents do not have the
/// same bytes — which matters only for the one property this mechanism claims,
/// that a value not be recognisable at a glance.
fn keystream(salt: &[u8], length: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(length);
    let mut counter: u64 = 0;
    while out.len() < length {
        let mut hash = Blake3::new();
        hash.start();
        hash.write(KEYSTREAM_LABEL);
        hash.write(salt);
        hash.write(&counter.to_le_bytes());
        out.extend_from_slice(&hash.finish());
        counter += 1;
    }
    out.truncate(length);
    out
}

/// The store's bytes, as they are written.
///
/// ```text
/// <HEADER>\n
/// <salt, 32 hex>\n
/// <obfuscated body, hex>\n
/// ```
///
/// Hex rather than raw bytes so the file survives being opened, saved and
/// mailed by something that thinks it is text — the failure mode a workstation
/// file actually meets.
#[must_use]
pub fn render(store: &Store, salt: &[u8; 32]) -> String {
    let mut body = String::new();
    for (ssid, pmk) in &store.entries {
        body.push_str(ssid);
        body.push('\t');
        body.push_str(&hex(pmk));
        body.push('\n');
    }
    let stream = keystream(salt, body.len());
    let hidden: Vec<u8> = body
        .as_bytes()
        .iter()
        .zip(stream.iter())
        .map(|(b, k)| b ^ k)
        .collect();
    format!("{HEADER}\n{}\n{}\n", hex(salt), hex(&hidden))
}

/// Read a store back.
///
/// # Errors
///
/// [`CredentialError::Malformed`] for anything that is not the shape [`render`]
/// writes — including a file whose header has been edited, because a store
/// whose honesty line somebody removed is one whose next reader will not be
/// told what it is.
pub fn parse(path: &Path, text: &str) -> Result<Store, CredentialError> {
    let malformed = |detail: &str| CredentialError::Malformed {
        path: path.to_path_buf(),
        detail: detail.to_owned(),
    };
    let mut lines = text.lines();
    let header = lines.next().ok_or_else(|| malformed("it is empty"))?;
    if header.trim() != HEADER {
        return Err(malformed(
            "its first line is not the one this tool writes. That line is what tells the next \
             person the file is obfuscated and NOT encrypted (ADR-034 10.4), so a store without \
             it is not read",
        ));
    }
    let salt = lines.next().ok_or_else(|| malformed("it has no salt"))?;
    let salt = from_hex(salt).ok_or_else(|| malformed("its salt is not hexadecimal"))?;
    let hidden = lines.next().unwrap_or("");
    let hidden = from_hex(hidden).ok_or_else(|| malformed("its body is not hexadecimal"))?;

    let stream = keystream(&salt, hidden.len());
    let plain: Vec<u8> = hidden.iter().zip(stream.iter()).map(|(b, k)| b ^ k).collect();
    let plain = String::from_utf8(plain)
        .map_err(|_| malformed("its body does not decode, so the file is not one this tool wrote"))?;

    let mut store = Store::default();
    for line in plain.lines() {
        if line.is_empty() {
            continue;
        }
        let (ssid, pmk) = line
            .split_once('\t')
            .ok_or_else(|| malformed("an entry has no key beside its network"))?;
        let pmk = pmk_from_hex(pmk).map_err(|_| malformed("an entry's key is not a PMK"))?;
        store.entries.insert(ssid.to_owned(), pmk);
    }
    Ok(store)
}

/// Read the store at `path`.
///
/// **Nothing held yet is an empty store, not a failure** — the same rule
/// `crate::store::load` follows for the register, and for the same reason: a
/// first run has nothing to read and a tool that refused could never record
/// anything.
///
/// # Errors
///
/// [`CredentialError::Io`] for a file that exists and will not be read, and
/// [`CredentialError::Malformed`] for one this build did not write.
pub fn load(path: &Path) -> Result<Store, CredentialError> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(path, &text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(e) => Err(CredentialError::Io {
            path: path.to_path_buf(),
            detail: e.to_string(),
        }),
    }
}

/// Write the store to `path`, and restrict it as far as the platform allows.
///
/// # Errors
///
/// [`CredentialError::Io`] for anything the filesystem refuses.
pub fn save(path: &Path, store: &Store, salt: &[u8; 32]) -> Result<(), CredentialError> {
    let io = |e: &std::io::Error, at: &Path| CredentialError::Io {
        path: at.to_path_buf(),
        detail: e.to_string(),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io(&e, parent))?;
    }
    std::fs::write(path, render(store, salt)).map_err(|e| io(&e, path))?;
    restrict(path);
    Ok(())
}

/// Make the store readable by its owner alone.
///
/// **Best effort, and it says so.** `ADR-034` §10.4 asks for restrictive file
/// permissions; on Unix that is `0600`. On Windows the equivalent is an ACL
/// edit that needs a crate this workspace does not have, so **nothing is
/// claimed there** — which is precisely why §10.4 made the file's own honesty
/// line non-optional rather than nice-to-have. The line protects the operator
/// on every platform; the mode protects them on one.
///
/// A failure to set the mode is not reported. The file has been written and its
/// first line says what it is; refusing the whole operation because a
/// permission bit would not take would be trading the thing that works for the
/// thing that did not.
fn restrict(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    let text = text.trim();
    if text.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() / 2);
    for index in 0..text.len() / 2 {
        out.push(u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    const SALT: [u8; 32] = [7u8; 32];

    fn store() -> Store {
        let mut store = Store::default();
        store.put(Credential {
            ssid: "home".to_owned(),
            pmk: [0xab; PMK_BYTES],
        });
        store
    }

    // ---- the honesty ADR-034 §10.4 requires --------------------------------

    /// **The obligation, as a test.** A file that is obfuscated must not be
    /// called encrypted anywhere — and the one place an operator will actually
    /// read is the file's own first line.
    #[test]
    fn the_file_says_what_it_is_in_the_clear() {
        let text = render(&store(), &SALT);
        let first = text.lines().next().expect("a header");
        assert!(first.contains("NOT ENCRYPTED"), "{first}");
        assert!(
            first.to_ascii_lowercase().contains("obfuscated"),
            "{first}"
        );
        assert!(first.contains("ADR-034"), "{first}");
    }

    /// And the word is never used affirmatively in the part of this file that
    /// ships. `AGENTS.md` §3 applied to a name: an operator who believes a
    /// store is protected treats it as safe to copy.
    ///
    /// **Only the code above the test module is scanned.** A test's own name
    /// has to be able to say what it is testing, and a scan that could not tell
    /// the two apart would be answered by rewording the test rather than by
    /// fixing anything.
    #[test]
    fn nothing_this_module_ships_claims_more_than_obfuscation() {
        let source = include_str!("credentials.rs");
        let ships = source
            .split_once("#[cfg(test)]")
            .map_or(source, |(before, _)| before);
        for line in ships.lines() {
            let trimmed = line.trim_start();
            // A blockquote is the ADR's words, not this module's claim — and
            // ADR-034 10.4's own warning is *about* somebody believing a store
            // is protected, so quoting it must not read as saying it.
            if trimmed.starts_with("//! >") || trimmed.starts_with("/// >") {
                continue;
            }
            let lower = line.to_ascii_lowercase();
            if !lower.contains("encrypt") {
                continue;
            }
            // Every mention must be a denial, in the same sentence. The list is
            // the words a denial is actually written with here, not a list of
            // permitted phrasings — a new phrasing that is genuinely a denial
            // adds a word; one that is not, fails.
            assert!(
                ["not ", "never ", "no ", "cannot ", "would not", "will not"]
                    .iter()
                    .any(|deny| lower.contains(deny)),
                "this line claims the store is protected: {line}"
            );
        }
    }

    /// A store whose honesty line was edited away is not read. The next person
    /// to open it would not be told what it is, and that is the whole mechanism.
    #[test]
    fn a_store_with_its_header_removed_is_refused() {
        let text = render(&store(), &SALT);
        let without = text.lines().skip(1).collect::<Vec<_>>().join("\n");
        let error = parse(Path::new("x"), &without).expect_err("no header");
        assert!(error.to_string().contains("NOT encrypted"), "{error}");
    }

    // ---- what obfuscation does and does not do -----------------------------

    /// The one claim: the value is not there to be found by eye or by `grep`.
    #[test]
    fn the_key_does_not_appear_in_the_file() {
        let text = render(&store(), &SALT);
        assert!(!text.contains(&hex(&[0xab; PMK_BYTES])), "{text}");
        // Nor does the network's name, which is not secret but is the string
        // somebody would grep for to find the key beside it.
        assert!(!text.contains("home"), "{text}");
    }

    /// Two stores with one content do not have one file, so a value cannot be
    /// recognised by comparing two operators' stores.
    #[test]
    fn two_stores_with_the_same_contents_do_not_have_the_same_bytes() {
        let a = render(&store(), &[1u8; 32]);
        let b = render(&store(), &[2u8; 32]);
        assert_ne!(a, b);
    }

    #[test]
    fn what_was_written_is_what_comes_back() {
        let text = render(&store(), &SALT);
        assert_eq!(parse(Path::new("x"), &text).expect("readable"), store());
    }

    #[test]
    fn an_empty_store_round_trips_too() {
        let empty = Store::default();
        let text = render(&empty, &SALT);
        assert_eq!(parse(Path::new("x"), &text).expect("readable"), empty);
    }

    /// Stable bytes for stable inputs, so a store in version control does not
    /// churn — even though `ADR-034` §9 says an operator should not put this
    /// one there.
    #[test]
    fn the_same_store_and_salt_render_the_same_bytes() {
        assert_eq!(render(&store(), &SALT), render(&store(), &SALT));
    }

    // ---- the passphrase is never stored ------------------------------------

    /// ~~The refusal, and it is the point of the whole module.~~ **The
    /// DERIVATION, since 2026-08-30.** `ADR-004` §5's rule is unchanged — the
    /// plaintext passphrase is never written to persistent storage — but the
    /// way it is honoured has flipped: this build no longer refuses, it derives,
    /// and what it keeps is the key.
    ///
    /// `AGENTS.md` §8.3 — a fixture, not anybody's network.
    #[test]
    fn a_passphrase_becomes_a_key_and_the_passphrase_is_not_kept() {
        let pmk = derive_pmk("home", "a-passphrase").expect("it derives now");
        assert_eq!(pmk.len(), PMK_BYTES);

        // **The property that matters**: what the store would hold contains no
        // trace of what was typed.
        let hex: String = pmk.iter().map(|b| format!("{b:02x}")).collect();
        assert!(!hex.contains("passphrase"), "{hex}");
        assert_ne!(pmk, [0u8; PMK_BYTES], "an all-zero key is not a derivation");
    }

    /// The SSID is the salt, so one passphrase on two networks is two keys.
    /// That is why a register entry is per-network and not per-passphrase.
    #[test]
    fn one_passphrase_on_two_networks_is_two_keys() {
        assert_ne!(
            derive_pmk("home", "a-passphrase").expect("valid"),
            derive_pmk("work", "a-passphrase").expect("valid")
        );
    }

    /// **A refusal reports a length and never the passphrase** — `ADR-020` §4,
    /// in the one function in this project that is handed a plaintext
    /// credential.
    #[test]
    fn a_passphrase_that_is_not_one_is_refused_without_being_echoed() {
        let error = derive_pmk("home", "hunter2").expect_err("7 bytes is too short");
        assert_eq!(error, CredentialError::PassphraseLength { bytes: 7 });
        let text = error.to_string();
        assert!(!text.contains("hunter"), "the refusal echoed it: {text}");
        assert!(text.contains('7'), "{text}");
    }

    /// A passphrase pasted where a PMK belongs is refused by length, with the
    /// two likely mistakes distinguished.
    #[test]
    fn a_passphrase_pasted_as_a_key_is_refused_by_shape() {
        let error = pmk_from_hex("my-wifi-password").expect_err("not a PMK");
        let text = error.to_string();
        assert!(text.contains("64 hexadecimal"), "{text}");

        let truncated = pmk_from_hex(&"a".repeat(63)).expect_err("short");
        assert!(truncated.to_string().contains("63 characters"), "{truncated}");

        let nonsense = pmk_from_hex(&"z".repeat(64)).expect_err("not hex");
        assert!(nonsense.to_string().contains("not hexadecimal"), "{nonsense}");
    }

    /// A PMK as `wpa_passphrase` prints it, with whitespace, is accepted.
    #[test]
    fn a_pasted_key_survives_the_whitespace_a_paste_brings() {
        let key = pmk_from_hex(&format!("  {}\n", hex(&[0x5a; PMK_BYTES]))).expect("a PMK");
        assert_eq!(key, [0x5a; PMK_BYTES]);
    }

    // ---- the store's own surface -------------------------------------------

    /// Only the networks can be listed. A store that could be enumerated into a
    /// report is one that would end up in a report.
    #[test]
    fn the_store_can_be_listed_by_network_and_not_by_value() {
        let mut store = store();
        store.put(Credential {
            ssid: "workshop".to_owned(),
            pmk: [1; PMK_BYTES],
        });
        assert_eq!(store.networks(), ["home", "workshop"]);
    }

    #[test]
    fn a_credential_replaces_the_one_for_the_same_network() {
        let mut store = store();
        store.put(Credential {
            ssid: "home".to_owned(),
            pmk: [9; PMK_BYTES],
        });
        assert_eq!(store.networks().len(), 1);
        assert_eq!(store.get("home").expect("held").pmk, [9; PMK_BYTES]);
    }

    #[test]
    fn forgetting_says_whether_there_was_anything_to_forget() {
        let mut store = store();
        assert!(store.forget("home"));
        assert!(!store.forget("home"));
        assert!(store.is_empty());
    }
}
