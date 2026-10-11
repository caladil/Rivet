// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use super::*;

/// A fresh, empty directory nothing else is using.
fn scratch(label: &str) -> PathBuf {
    crate::scratch::dir(&format!(
        "machine-secrets-{label}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ))
}

/// A THROWAWAY identity, minted by `cargo rivet identity new` into a scratch
/// directory: `(seed, certificate, public key hex)`.
fn mint(label: &str) -> (Vec<u8>, Vec<u8>, String) {
    let dir = scratch(&format!("identity-{label}"));
    let words: Vec<String> = ["new", "--directory", &dir.display().to_string(), "--name", label]
        .iter()
        .map(|w| (*w).to_string())
        .collect();
    crate::identity::run(&crate::identity::parse(&words).expect("identity args"))
        .expect("a throwaway identity");
    let seed = std::fs::read(dir.join(rivetos_tls::SEED_FILE)).expect("seed");
    let certificate = std::fs::read(dir.join(rivetos_tls::CERTIFICATE_FILE)).expect("cert");
    let key = rivetos_tls::public_key_in_certificate(&certificate)
        .map(|pin| rivetos_tls::encode_hex(&pin))
        .expect("a key");
    std::fs::remove_dir_all(&dir).ok();
    (seed, certificate, key)
}

/// The fictional passphrase every backup here is sealed under.
const PASSPHRASE: &str = "machine-secrets test passphrase, fictional and fixed";
const WRONG: &str = "not the passphrase this backup was sealed under";

/// **A passphrase source that counts how often it is asked** — so a test can
/// say a refusal came before anyone would have been asked to type.
struct Typist {
    passphrase: &'static str,
    asked: Cell<usize>,
}

impl Typist {
    const fn typing(passphrase: &'static str) -> Self {
        Self { passphrase, asked: Cell::new(0) }
    }

    fn ask(&self) -> impl Fn() -> Result<String, String> + '_ {
        || {
            self.asked.set(self.asked.get() + 1);
            Ok(self.passphrase.to_string())
        }
    }
}

/// **A fake board at the [`CallsTools`] boundary** — `fleet_drift_tests.rs`'s
/// pattern. It holds `/state/secrets` as a map and answers the two calls these
/// verbs make — `get file` and `apply restore-secret` — in the shapes
/// `rivet_core::operation`'s `file` and `restore_secret` produce.
struct FakeBoard {
    /// `None` is a board with no `/state/secrets` at all.
    store: RefCell<Option<BTreeMap<String, Vec<u8>>>>,
    /// Entries that are not regular files, by name and type.
    others: Vec<(String, String)>,
    /// A refusal every `apply` gets.
    refuse_apply: Option<String>,
    /// Every call, as the words it carried.
    calls: RefCell<Vec<Vec<String>>>,
}

impl FakeBoard {
    fn holding(files: &[(&str, &[u8])]) -> Self {
        let map = files
            .iter()
            .map(|(n, b)| ((*n).to_string(), b.to_vec()))
            .collect();
        Self {
            store: RefCell::new(Some(map)),
            others: Vec::new(),
            refuse_apply: None,
            calls: RefCell::new(Vec::new()),
        }
    }

    fn file(&self, name: &str) -> Option<Vec<u8>> {
        self.store.borrow().as_ref().and_then(|m| m.get(name).cloned())
    }

    fn applies(&self) -> usize {
        self.calls
            .borrow()
            .iter()
            .filter(|w| w.first().map(String::as_str) == Some(RESTORE_SUBJECT))
            .count()
    }

    fn fetched_contents(&self) -> Vec<String> {
        self.calls
            .borrow()
            .iter()
            .filter(|w| w.first().map(String::as_str) == Some("file"))
            .filter_map(|w| w.get(1).cloned())
            .filter(|p| p != STORE)
            .collect()
    }
}

/// The words a composed call carries, in declaration order.
fn words_of(arguments: &str) -> Vec<String> {
    let json = rivet_core::json::parse(arguments).expect("the call's arguments are JSON");
    let Json::Object(fields) = json else { panic!("not an object: {arguments}") };
    let mut out = Vec::new();
    for (_, value) in fields {
        match value {
            Json::Str(s) => out.push(s),
            Json::Array(items) => out.extend(items.into_iter().filter_map(|i| i.as_str().map(str::to_string))),
            other => panic!("unexpected {other:?}"),
        }
    }
    out
}

impl CallsTools for FakeBoard {
    fn call_tool(&self, tool: &str, arguments: &str) -> Result<String, String> {
        let words = words_of(arguments);
        self.calls.borrow_mut().push(words.clone());
        match (tool, words.first().map(String::as_str)) {
            ("rivet_get", Some("file")) => {
                let path = words.get(1).expect("a path").clone();
                let store = self.store.borrow();
                if path == STORE {
                    let Some(map) = store.as_ref() else {
                        return Ok(format!(r#"{{"subject":"file","path":"{path}","present":false}}"#));
                    };
                    let mut entries: Vec<String> = map
                        .iter()
                        .map(|(n, b)| format!(r#"{{"name":"{n}","type":"file","size_bytes":{}}}"#, b.len()))
                        .collect();
                    entries.extend(
                        self.others
                            .iter()
                            .map(|(n, t)| format!(r#"{{"name":"{n}","type":"{t}","size_bytes":0}}"#)),
                    );
                    return Ok(format!(
                        r#"{{"subject":"file","path":"{path}","present":true,"type":"directory","entry_count":{},"entries":[{}],"entries_truncated":false}}"#,
                        entries.len(),
                        entries.join(",")
                    ));
                }
                let name = path.strip_prefix(&format!("{STORE}/")).expect("under the store");
                match store.as_ref().and_then(|m| m.get(name)) {
                    None => Ok(format!(r#"{{"subject":"file","path":"{path}","present":false}}"#)),
                    Some(bytes) => Ok(format!(
                        r#"{{"subject":"file","path":"{path}","present":true,"type":"file","size_bytes":{},"content":"{}","content_encoding":"base64"}}"#,
                        bytes.len(),
                        rivet_core::base64::encode(bytes)
                    )),
                }
            }
            ("rivet_apply", Some(RESTORE_SUBJECT)) => {
                if let Some(why) = &self.refuse_apply {
                    return Err(why.clone());
                }
                assert_eq!(words.get(2).map(String::as_str), Some("--hex"), "{words:?}");
                assert_eq!(words.last().map(String::as_str), Some("confirm"), "{words:?}");
                let name = words[1].clone();
                let bytes = rivet_core::secret::bytes_from_hex(&words[3]).expect("hex");
                let len = bytes.len();
                self.store
                    .borrow_mut()
                    .as_mut()
                    .expect("a store")
                    .insert(name.clone(), bytes);
                Ok(format!(r#"{{"subject":"restore-secret","name":"{name}","bytes":{len}}}"#))
            }
            _ => Err(format!("FakeBoard does not answer {tool} {words:?}")),
        }
    }
}

fn endpoint(pin: &str) -> Endpoint {
    Endpoint {
        host: Some("127.0.0.1:7777".to_string()),
        identity: Some(PathBuf::from("./operator")),
        pin: Some(pin.to_string()),
    }
}

fn backup_args(pin: &str, into: &Path, confirm: bool) -> BackupArgs {
    BackupArgs {
        endpoint: endpoint(pin),
        into: into.to_path_buf(),
        confirm,
    }
}

fn restore_args(pin: &str, from: &Path, replace: bool, confirm: bool) -> RestoreArgs {
    RestoreArgs {
        endpoint: endpoint(pin),
        from: from.to_path_buf(),
        replace,
        confirm,
    }
}

/// A board's store as a real one looks: identity, peers naming `operator`,
/// and a wireless key in hex.
struct Fixture {
    seed: Vec<u8>,
    certificate: Vec<u8>,
    board: String,
    operator: String,
    peers: Vec<u8>,
    psk: Vec<u8>,
}

fn fixture(label: &str) -> Fixture {
    let (seed, certificate, board) = mint(&format!("{label}-board"));
    let (_, _, operator) = mint(&format!("{label}-operator"));
    Fixture {
        peers: format!("# the operator\n{operator}\n").into_bytes(),
        psk: b"6d792d6e6574776f726b2d6b6579".to_vec(),
        seed,
        certificate,
        board,
        operator,
    }
}

impl Fixture {
    fn board(&self) -> FakeBoard {
        FakeBoard::holding(&[
            (rivetos_tls::SEED_FILE, &self.seed),
            (rivetos_tls::CERTIFICATE_FILE, &self.certificate),
            (PEERS, &self.peers),
            ("wlan0.psk", &self.psk),
        ])
    }

    /// A finished backup of [`Self::board`], sealed under [`PASSPHRASE`] in a
    /// scratch directory.
    fn backup(&self, label: &str) -> PathBuf {
        let into = scratch(label).join("backup");
        let typist = Typist::typing(PASSPHRASE);
        backup_via(&backup_args(&self.board, &into, true), &self.board(), 1_788_000_000, &typist.ask())
            .expect("a backup");
        into
    }
}

/// Open a backup with [`PASSPHRASE`], hand its plaintext to `edit`, and seal
/// the result back in place under the same passphrase — **somebody holding
/// the passphrase** writing a backup whose inside is wrong: the way past the
/// AEAD to the checks that come after it.
fn reseal(from: &Path, edit: impl FnOnce(Vec<u8>) -> Vec<u8>) {
    let at = from.join(BACKUP_FILE);
    let sealed = std::fs::read(&at).expect("the backup");
    let plaintext = envelope::open(SECRETS_ENVELOPE, &sealed, PASSPHRASE).expect("opens");
    let resealed = envelope::seal(SECRETS_ENVELOPE, &edit(plaintext), PASSPHRASE).expect("seals");
    std::fs::write(&at, resealed).expect("write back");
}

fn restore(f: &Fixture, from: &Path, board: &FakeBoard, typing: &'static str) -> Result<String, String> {
    let typist = Typist::typing(typing);
    restore_via(&restore_args(&f.board, from, true, true), board, &f.operator, &typist.ask())
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

// ---------------------------------------------------------------------------
// backup-secrets
// ---------------------------------------------------------------------------

/// **Without `confirm`, no secret crosses the wire and no passphrase is asked
/// for.** Only the listing and the (public) certificate are read.
#[test]
fn the_backup_plan_reads_no_secret_asks_no_passphrase_and_writes_nothing() {
    let f = fixture("plan");
    let board = f.board();
    let into = scratch("plan").join("backup");
    let typist = Typist::typing(PASSPHRASE);
    let plan = backup_via(&backup_args(&f.board, &into, false), &board, 0, &typist.ask()).expect("a plan");
    assert!(plan.contains(&f.board), "{plan}");
    assert!(plan.contains("wlan0.psk"), "{plan}");
    assert!(plan.contains("passphrase you type twice"), "{plan}");
    assert!(plan.contains("Add `confirm`"), "{plan}");
    assert_eq!(
        board.fetched_contents(),
        vec![format!("{STORE}/{}", rivetos_tls::CERTIFICATE_FILE)],
        "only the certificate may be read on the plan path"
    );
    assert_eq!(typist.asked.get(), 0);
    assert!(!into.exists(), "the plan wrote something");
}

/// **The backup is one encrypted file, and no secret — nor its name — is in
/// it readably.**
#[test]
fn a_confirmed_backup_is_one_sealed_file_with_nothing_readable_in_it() {
    let f = fixture("backup");
    let into = f.backup("backup");
    let entries: Vec<String> = std::fs::read_dir(&into)
        .expect("the backup directory")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, vec![BACKUP_FILE.to_string()], "nothing but the envelope, no .partial");

    let sealed = std::fs::read(into.join(BACKUP_FILE)).expect("the envelope");
    assert_eq!(&sealed[..envelope::MAGIC_LEN], b"RVTSECR");
    for needle in [
        f.seed.as_slice(),
        f.psk.as_slice(),
        f.peers.as_slice(),
        b"wlan0.psk".as_slice(),
        f.board.as_bytes(),
        blake3_hex(&f.psk).as_bytes(),
    ] {
        assert!(!contains(&sealed, needle), "readable in the envelope: {needle:?}");
    }

    // And inside, byte for byte, with the board recorded.
    let plaintext = envelope::open(SECRETS_ENVELOPE, &sealed, PASSPHRASE).expect("opens");
    let (manifest, files) = unarchive(&plaintext).expect("an archive");
    assert_eq!(manifest.board_key, f.board);
    assert_eq!(manifest.taken_unix_seconds, 1_788_000_000);
    for (name, bytes) in [
        (rivetos_tls::SEED_FILE, &f.seed),
        (rivetos_tls::CERTIFICATE_FILE, &f.certificate),
        (PEERS, &f.peers),
        ("wlan0.psk", &f.psk),
    ] {
        let held = files.iter().find(|(n, _)| n == name).map(|(_, b)| b);
        assert_eq!(held, Some(bytes), "{name}");
    }
}

/// **The report says the backup is encrypted and what a forgotten passphrase
/// costs, and prints no digest of a secret.**
#[test]
fn the_backup_report_warns_and_prints_no_digest() {
    let f = fixture("report");
    let into = scratch("report").join("backup");
    let typist = Typist::typing(PASSPHRASE);
    let report = backup_via(&backup_args(&f.board, &into, true), &f.board(), 0, &typist.ask())
        .expect("a backup");
    assert!(report.contains("ENCRYPTED"), "{report}");
    assert!(report.contains("FORGOTTEN PASSPHRASE"), "{report}");
    assert!(!report.contains(&blake3_hex(&f.psk)), "{report}");
    assert_eq!(typist.asked.get(), 1);
}

/// **A prompt that refuses — no terminal, two entries that differ — fetches
/// no secret and writes nothing.**
#[test]
fn a_refused_passphrase_fetches_no_secret_and_writes_nothing() {
    let f = fixture("no-passphrase");
    let board = f.board();
    let into = scratch("no-passphrase").join("backup");
    let refused = backup_via(&backup_args(&f.board, &into, true), &board, 0, &|| {
        Err("the two entries did not match. Nothing was written.".to_string())
    })
    .expect_err("refused");
    assert!(refused.contains("did not match"), "{refused}");
    assert_eq!(
        board.fetched_contents(),
        vec![format!("{STORE}/{}", rivetos_tls::CERTIFICATE_FILE)],
        "no secret may cross the wire before a passphrase exists"
    );
    assert!(!into.exists());
}

#[test]
fn a_directory_that_already_holds_a_backup_is_refused_before_anything_is_read() {
    let f = fixture("exists");
    let into = f.backup("exists");
    let board = f.board();
    let typist = Typist::typing(PASSPHRASE);
    let refused = backup_via(&backup_args(&f.board, &into, true), &board, 0, &typist.ask()).expect_err("refused");
    assert!(refused.contains("already holds a backup"), "{refused}");
    assert!(board.calls.borrow().is_empty(), "the board was asked something");
    assert_eq!(typist.asked.get(), 0);
}

/// An unencrypted backup an earlier build left in `--into` is a backup too —
/// refused, not written beside.
#[test]
fn a_directory_holding_an_unencrypted_backup_is_refused_for_a_new_one() {
    let f = fixture("exists-old");
    let into = scratch("exists-old");
    std::fs::write(into.join(UNENCRYPTED_MANIFEST), b"format = rivet-machine-secrets 1\n").expect("write");
    let board = f.board();
    let typist = Typist::typing(PASSPHRASE);
    let refused = backup_via(&backup_args(&f.board, &into, true), &board, 0, &typist.ask()).expect_err("refused");
    assert!(refused.contains("already holds a backup"), "{refused}");
    assert!(board.calls.borrow().is_empty());
}

/// **A board whose certificate names another key is not the board `--pin`
/// names** — refused before `confirm`.
#[test]
fn a_certificate_that_disagrees_with_the_pin_refuses_the_backup() {
    let f = fixture("disagree");
    let (_, _, other) = mint("disagree-other");
    let into = scratch("disagree").join("backup");
    let typist = Typist::typing(PASSPHRASE);
    let refused = backup_via(&backup_args(&other, &into, false), &f.board(), 0, &typist.ask()).expect_err("refused");
    assert!(refused.contains(&f.board) && refused.contains(&other), "{refused}");
    assert!(!into.exists());
    assert_eq!(typist.asked.get(), 0);
}

#[test]
fn an_entry_that_is_not_a_file_refuses_the_whole_backup() {
    let f = fixture("dir-entry");
    let mut board = f.board();
    board.others.push(("nested".to_string(), "directory".to_string()));
    let into = scratch("dir-entry").join("backup");
    let typist = Typist::typing(PASSPHRASE);
    let refused = backup_via(&backup_args(&f.board, &into, true), &board, 0, &typist.ask()).expect_err("refused");
    assert!(refused.contains("nested") && refused.contains("directory"), "{refused}");
    assert!(!into.exists());
}

#[test]
fn a_board_with_no_store_is_refused() {
    let f = fixture("no-store");
    let board = f.board();
    *board.store.borrow_mut() = None;
    let into = scratch("no-store").join("backup");
    let typist = Typist::typing(PASSPHRASE);
    let refused = backup_via(&backup_args(&f.board, &into, true), &board, 0, &typist.ask()).expect_err("refused");
    assert!(refused.contains("no /state/secrets"), "{refused}");
}

#[test]
fn a_file_too_large_to_put_back_refuses_the_backup() {
    let f = fixture("too-large");
    let big = vec![b'a'; rivet_core::operation::SECRET_MAX_BYTES + 1];
    let board = FakeBoard::holding(&[(rivetos_tls::CERTIFICATE_FILE, &f.certificate), ("big", &big)]);
    let into = scratch("too-large").join("backup");
    let typist = Typist::typing(PASSPHRASE);
    let refused = backup_via(&backup_args(&f.board, &into, true), &board, 0, &typist.ask()).expect_err("refused");
    assert!(refused.contains("could never be restored"), "{refused}");
}

// ---------------------------------------------------------------------------
// restore-secrets — the round trip
// ---------------------------------------------------------------------------

/// **The round trip.** A board that lost two files and had one changed gets
/// the backup's bytes back — with `--replace` — and the identical files are
/// not written at all.
#[test]
fn a_backup_restores_onto_the_same_board_and_reads_back_identical() {
    let f = fixture("round-trip");
    let from = f.backup("round-trip");

    let board = FakeBoard::holding(&[
        (rivetos_tls::SEED_FILE, &f.seed),
        (rivetos_tls::CERTIFICATE_FILE, &f.certificate),
        ("wlan0.psk", b"deadbeef"),
        ("unrelated", b"left alone"),
    ]);

    // The plan first: nothing written, every file's fate named — and the
    // passphrase asked for, because the plan compares the backup's bytes.
    let typist = Typist::typing(PASSPHRASE);
    let plan = restore_via(&restore_args(&f.board, &from, true, false), &board, &f.operator, &typist.ask())
        .expect("a plan");
    assert_eq!(typist.asked.get(), 1);
    assert!(plan.contains("identical, skipped"), "{plan}");
    assert!(plan.contains("REPLACE"), "{plan}");
    assert!(plan.contains("write"), "{plan}");
    assert!(plan.contains("left alone"), "{plan}");
    assert!(plan.contains("NOTHING WILL BE RESTARTED"), "{plan}");
    assert_eq!(board.applies(), 0);

    let report = restore(&f, &from, &board, PASSPHRASE).expect("restores");
    assert_eq!(board.file(PEERS).as_deref(), Some(f.peers.as_slice()));
    assert_eq!(board.file("wlan0.psk").as_deref(), Some(f.psk.as_slice()));
    assert_eq!(board.file("unrelated").as_deref(), Some(b"left alone".as_slice()));
    assert_eq!(board.applies(), 2, "only the missing and the differing file are written");
    assert!(report.contains("read back identical"), "{report}");
    assert!(report.contains("NOTHING WAS RESTARTED"), "{report}");
}

/// **A differing file needs `--replace`**, and without it nothing at all is
/// written — not even the files that would only have been added.
#[test]
fn a_differing_file_without_replace_writes_nothing() {
    let f = fixture("no-replace");
    let from = f.backup("no-replace");
    let board = FakeBoard::holding(&[
        (rivetos_tls::SEED_FILE, &f.seed),
        (rivetos_tls::CERTIFICATE_FILE, &f.certificate),
        ("wlan0.psk", b"deadbeef"),
    ]);
    let typist = Typist::typing(PASSPHRASE);
    let refused = restore_via(&restore_args(&f.board, &from, false, true), &board, &f.operator, &typist.ask())
        .expect_err("refused");
    assert!(refused.contains("wlan0.psk") && refused.contains("--replace"), "{refused}");
    assert_eq!(board.applies(), 0);
    assert_eq!(board.file(PEERS), None);
}

// ---------------------------------------------------------------------------
// restore-secrets — the envelope's refusals
// ---------------------------------------------------------------------------

/// **R-185 D1's refusal, and the AEAD's property.** A wrong passphrase and a
/// byte altered in the envelope are refused with the same words, and the
/// board is asked nothing either way.
#[test]
fn a_wrong_passphrase_and_a_tampered_backup_refuse_identically_and_send_nothing() {
    let f = fixture("wrong-or-tampered");
    let from = f.backup("wrong-or-tampered");
    let board = f.board();
    let wrong = restore(&f, &from, &board, WRONG).expect_err("a wrong passphrase");

    let at = from.join(BACKUP_FILE);
    let mut sealed = std::fs::read(&at).expect("read");
    let middle = sealed.len() - 20;
    sealed[middle] ^= 0x01;
    std::fs::write(&at, &sealed).expect("tamper");
    let tampered = restore(&f, &from, &board, PASSPHRASE).expect_err("tampered");

    assert_eq!(wrong, tampered, "the two refusals must not be told apart");
    assert!(wrong.contains("wrong passphrase") && wrong.contains("NOTHING WAS SENT"), "{wrong}");
    assert!(wrong.contains("FORGOTTEN PASSPHRASE"), "{wrong}");
    assert!(board.calls.borrow().is_empty(), "the board was asked something");
}

/// A truncated envelope is refused before a passphrase is asked for — it is
/// not a finished file of this kind at all.
#[test]
fn a_truncated_backup_is_refused_before_a_passphrase_is_asked_for() {
    let f = fixture("truncated");
    let from = f.backup("truncated");
    let at = from.join(BACKUP_FILE);
    let sealed = std::fs::read(&at).expect("read");
    std::fs::write(&at, &sealed[..sealed.len() - 1]).expect("truncate");
    let board = FakeBoard::holding(&[]);
    let typist = Typist::typing(PASSPHRASE);
    let refused = restore_via(&restore_args(&f.board, &from, true, true), &board, &f.operator, &typist.ask())
        .expect_err("truncated");
    assert!(refused.contains("truncated") && refused.contains("NOTHING WAS SENT"), "{refused}");
    assert_eq!(typist.asked.get(), 0);
    assert!(board.calls.borrow().is_empty());
}

/// **The unencrypted layout an earlier build wrote is refused**, naming what
/// to do, before a passphrase is asked for and before the board is asked
/// anything.
#[test]
fn an_unencrypted_backup_from_an_earlier_build_is_refused() {
    let f = fixture("unencrypted");
    let from = scratch("unencrypted");
    std::fs::create_dir_all(from.join(UNENCRYPTED_FILES_DIR)).expect("files/");
    std::fs::write(from.join(UNENCRYPTED_FILES_DIR).join("wlan0.psk"), &f.psk).expect("write");
    std::fs::write(
        from.join(UNENCRYPTED_MANIFEST),
        format!("format = rivet-machine-secrets 1\nboard-key = {}\n", f.board),
    )
    .expect("write");
    let board = f.board();
    let typist = Typist::typing(PASSPHRASE);
    let refused = restore_via(&restore_args(&f.board, &from, true, true), &board, &f.operator, &typist.ask())
        .expect_err("refused");
    assert!(refused.contains("UNENCRYPTED") && refused.contains("take a new one"), "{refused}");
    assert_eq!(typist.asked.get(), 0);
    assert!(board.calls.borrow().is_empty());
}

#[test]
fn a_directory_with_no_backup_file_is_not_a_backup() {
    let f = fixture("no-file");
    let from = f.backup("no-file");
    std::fs::remove_file(from.join(BACKUP_FILE)).expect("remove");
    let refused = restore(&f, &from, &f.board(), PASSPHRASE).expect_err("refused");
    assert!(refused.contains("not a finished"), "{refused}");
}

/// A seed backup handed to `restore-secrets` is the wrong kind of envelope.
#[test]
fn a_seed_backup_envelope_is_not_a_secrets_backup() {
    let f = fixture("seed-envelope");
    let from = scratch("seed-envelope");
    let seed_kind = Kind { magic: b"RVTSEED", noun: "a seed backup" };
    let sealed = envelope::seal(seed_kind, &[1u8; 32], PASSPHRASE).expect("seals");
    std::fs::write(from.join(BACKUP_FILE), sealed).expect("write");
    let typist = Typist::typing(PASSPHRASE);
    let refused = restore_via(&restore_args(&f.board, &from, true, true), &f.board(), &f.operator, &typist.ask())
        .expect_err("refused");
    assert!(refused.contains("not a Rivet machine-secrets backup"), "{refused}");
    assert_eq!(typist.asked.get(), 0);
}

// ---------------------------------------------------------------------------
// restore-secrets — every refusal still applies after decryption
// ---------------------------------------------------------------------------

/// **ADR-030 §5 and R-185 D2.** A backup from one board, restored with the
/// pin of another, is refused after it opens and before the board is asked
/// anything.
#[test]
fn a_backup_from_a_different_board_is_refused_after_decryption_and_nothing_is_sent() {
    let f = fixture("other-board");
    let from = f.backup("other-board");
    let (seed, certificate, other_board) = mint("other-board-target");
    let board = FakeBoard::holding(&[
        (rivetos_tls::SEED_FILE, &seed),
        (rivetos_tls::CERTIFICATE_FILE, &certificate),
    ]);
    let typist = Typist::typing(PASSPHRASE);
    let refused = restore_via(&restore_args(&other_board, &from, true, true), &board, &f.operator, &typist.ask())
        .expect_err("a different board");
    assert_eq!(typist.asked.get(), 1, "the binding is inside the envelope");
    assert!(refused.contains("DIFFERENT BOARD"), "{refused}");
    assert!(refused.contains(&f.board) && refused.contains(&other_board), "{refused}");
    assert!(board.calls.borrow().is_empty(), "the board was asked something");
    assert_eq!(board.file(rivetos_tls::SEED_FILE), Some(seed));
}

/// **`cargo rivet recover` refuses another board's secrets before the card is
/// written** (R-235): the check it runs once the passphrase is typed opens
/// this module's own backup, and a backup of the machine being recovered
/// passes it.
#[test]
fn recover_refuses_a_secrets_backup_of_another_board_and_accepts_its_own() {
    let f = fixture("recover-binding");
    let from = f.backup("recover-binding");
    let (_, _, other_board) = mint("recover-binding-other");
    let typist = Typist::typing(PASSPHRASE);
    crate::recover::secrets_belong_to(&from, &f.board, &typist.ask()).expect("its own backup");
    let refused = crate::recover::secrets_belong_to(&from, &other_board, &typist.ask())
        .expect_err("another board's backup");
    assert!(refused.contains("DIFFERENT BOARD") && refused.contains("NOTHING WAS WRITTEN"), "{refused}");
    assert!(refused.contains(&f.board) && refused.contains(&other_board), "{refused}");
    let wrong = Typist::typing(WRONG);
    assert!(crate::recover::secrets_belong_to(&from, &f.board, &wrong.ask()).is_err());
}

/// **A file whose bytes disagree with the manifest's size is refused**, even
/// inside a correctly sealed envelope.
#[test]
fn an_archive_whose_bytes_the_manifest_does_not_account_for_is_refused() {
    let f = fixture("unaccounted");
    let from = f.backup("unaccounted");
    reseal(&from, |mut plaintext| {
        plaintext.push(b'x');
        plaintext
    });
    let board = FakeBoard::holding(&[]);
    let refused = restore(&f, &from, &board, PASSPHRASE).expect_err("refused");
    assert!(refused.contains("accounts for") && refused.contains("NOTHING WAS SENT"), "{refused}");
    assert!(board.calls.borrow().is_empty());
}

/// **A same-length alteration is caught by the digest** — the case a size
/// check alone would pass.
#[test]
fn an_archive_whose_file_disagrees_with_its_digest_is_refused() {
    let f = fixture("digest");
    let from = f.backup("digest");
    reseal(&from, |mut plaintext| {
        let last = plaintext.len() - 1;
        plaintext[last] ^= 0x01;
        plaintext
    });
    let board = f.board();
    let refused = restore(&f, &from, &board, PASSPHRASE).expect_err("refused");
    assert!(refused.contains("BLAKE3") && refused.contains("NOTHING WAS SENT"), "{refused}");
    assert!(board.calls.borrow().is_empty());
}

/// **Editing the manifest's board key to match another board does not get a
/// backup past the refusal**: the backup's own seed and certificate still name
/// the board it came from.
#[test]
fn a_manifest_edited_to_name_another_board_is_refused_by_the_backups_own_files() {
    let f = fixture("relabelled");
    let from = f.backup("relabelled");
    let (_, _, other) = mint("relabelled-other");
    reseal(&from, |plaintext| {
        let at = plaintext
            .windows(f.board.len())
            .position(|w| w == f.board.as_bytes())
            .expect("the key in the manifest");
        let mut edited = plaintext;
        edited[at..at + other.len()].copy_from_slice(other.as_bytes());
        edited
    });
    let board = FakeBoard::holding(&[]);
    let typist = Typist::typing(PASSPHRASE);
    let refused = restore_via(&restore_args(&other, &from, true, true), &board, &f.operator, &typist.ask())
        .expect_err("refused");
    assert!(refused.contains("not what it says it is"), "{refused}");
    assert!(board.calls.borrow().is_empty());
}

/// **The lock-out refusal, workstation side.** A backup whose `peers` does not
/// name the identity doing the restore is refused before anything is sent.
#[test]
fn a_backup_whose_peers_would_lock_out_this_workstation_is_refused() {
    let f = fixture("lockout");
    let from = f.backup("lockout");
    let (_, _, stranger) = mint("lockout-stranger");
    let board = f.board();
    let typist = Typist::typing(PASSPHRASE);
    let refused = restore_via(&restore_args(&f.board, &from, true, true), &board, &stranger, &typist.ask())
        .expect_err("refused");
    assert!(refused.contains(PEERS) && refused.contains(&stranger), "{refused}");
    assert!(board.calls.borrow().is_empty());
}

/// **The grammar refusal, workstation side.** A backup whose `peers` names
/// the identity doing the restore ON ITS OWN LINE, plus one line the board's
/// own reader would refuse, must still be refused before anything is sent —
/// a membership-only check (does any line match my key) would have let this
/// one through, because the caller's own key IS present.
#[test]
fn a_backup_whose_peers_has_a_malformed_line_beside_the_callers_key_is_refused() {
    let mut f = fixture("malformed-peers");
    f.peers = format!("{}\nnot-a-key\n", f.operator).into_bytes();
    let from = f.backup("malformed-peers");
    let board = f.board();
    let typist = Typist::typing(PASSPHRASE);
    let refused = restore_via(&restore_args(&f.board, &from, true, true), &board, &f.operator, &typist.ask())
        .expect_err("refused");
    assert!(refused.contains(PEERS), "{refused}");
    assert!(board.calls.borrow().is_empty(), "{:?}", board.calls.borrow());
}

/// The board's own refusal reaches the operator, with what was already written.
#[test]
fn a_board_that_refuses_the_write_is_reported() {
    let f = fixture("board-refuses");
    let from = f.backup("board-refuses");
    let mut board = FakeBoard::holding(&[(rivetos_tls::CERTIFICATE_FILE, &f.certificate)]);
    board.refuse_apply = Some("E_UNSAFE REFUSED /state/secrets is not there".to_string());
    let refused = restore(&f, &from, &board, PASSPHRASE).expect_err("refused");
    assert!(refused.contains("E_UNSAFE"), "{refused}");
    assert!(refused.contains("nothing rolls back"), "{refused}");
}

// ---------------------------------------------------------------------------
// The manifest, the archive, the wire, the vocabulary
// ---------------------------------------------------------------------------

#[test]
fn the_manifest_round_trips_and_refuses_what_it_does_not_understand() {
    let manifest = Manifest {
        board_key: "ab".repeat(32),
        host: "rivet-01.local:7777".to_string(),
        taken_unix_seconds: 42,
        files: vec![Entry {
            name: "device.seed".to_string(),
            bytes: 32,
            blake3: "cd".repeat(32),
        }],
    };
    let rendered = manifest.render();
    let text = rendered.strip_suffix(&format!("{END}\n")).expect("ends with the end line");
    assert_eq!(Manifest::parse(text).expect("parses"), manifest);

    for (broken, why) in [
        (text.replace(FORMAT, "rivet-machine-secrets 1"), "format"),
        (format!("{text}unknown = 1\n"), "not a key"),
        (format!("{text}board-key = {}\n", "ab".repeat(32)), "twice"),
        (text.replace(&"cd".repeat(32), "zz"), "BLAKE3"),
        (format!("{text}file = {} 1 ../escape\n", "ef".repeat(32)), "secret's name"),
        (format!("{text}file = {} 1 device.seed\n", "ef".repeat(32)), "named twice"),
    ] {
        let refused = Manifest::parse(&broken).expect_err(why);
        assert!(refused.contains(why), "{why}: {refused}");
    }
}

#[test]
fn an_archive_round_trips_and_one_with_no_end_line_is_refused() {
    let files = vec![
        ("a.psk".to_string(), b"one".to_vec()),
        ("b.psk".to_string(), b"\nend\ntwo".to_vec()),
    ];
    let manifest = Manifest {
        board_key: "ab".repeat(32),
        host: "h:7777".to_string(),
        taken_unix_seconds: 1,
        files: files
            .iter()
            .map(|(n, b)| Entry { name: n.clone(), bytes: b.len() as u64, blake3: blake3_hex(b) })
            .collect(),
    };
    let (back, held) = unarchive(&archive(&manifest, &files)).expect("unarchives");
    assert_eq!(back, manifest);
    assert_eq!(held, files, "a file holding the end line's own bytes is still read whole");

    let refused = unarchive(b"format = rivet-machine-secrets 2\n").expect_err("no end line");
    assert!(refused.contains("`end`"), "{refused}");
}

/// **What is sent is what the board's subject parses**: the word vector
/// `rivet_remote` composes for a restore binds the name, the flag, the hex and
/// the confirmation where `apply restore-secret` reads them.
#[test]
fn the_restore_call_binds_as_the_board_reads_it() {
    let call = rivet_remote::command::translate(&[
        "apply",
        RESTORE_SUBJECT,
        "device.seed",
        "--hex",
        "00ff",
        "confirm",
    ])
    .expect("translates");
    assert_eq!(call.tool, "rivet_apply");
    assert_eq!(
        words_of(&call.arguments),
        vec![RESTORE_SUBJECT, "device.seed", "--hex", "00ff", "confirm"]
    );
}

/// Two crates spell `peers`; they must agree.
#[test]
fn the_peers_name_is_the_one_rivet_mcpd_reads() {
    assert_eq!(PEERS, rivetos_tls::PEERS_FILE);
}

#[test]
fn both_verbs_require_a_directory_and_both_halves_of_the_trust() {
    let words = |list: &[&str]| -> Vec<String> { list.iter().map(|w| (*w).to_string()).collect() };
    let refused = parse_backup(&words(&["--host", "h:7777", "--identity", "i", "--pin", "ab"]))
        .expect_err("no --into");
    assert!(refused.contains("--into is required"), "{refused}");
    let refused = parse_restore(&words(&["--host", "h:7777", "--identity", "i", "--pin", "ab"]))
        .expect_err("no --from");
    assert!(refused.contains("--from is required"), "{refused}");
    let refused = parse_backup(&words(&["--host", "h:7777", "--identity", "i", "--into", "d"]))
        .expect_err("no pin");
    assert!(refused.contains("--pin"), "{refused}");
    let refused = parse_backup(&words(&["--into", "d", "--replace"])).expect_err("not a backup flag");
    assert!(refused.contains("--replace"), "{refused}");
    let parsed = parse_restore(&words(&[
        "--host", "h:7777", "--identity", "i", "--pin", "ab", "--from", "d", "--replace", "confirm",
    ]))
    .expect("valid");
    assert!(parsed.replace && parsed.confirm);
}

/// **There is no passphrase flag**, on either verb, and the refusal says so.
#[test]
fn neither_verb_takes_a_passphrase_flag() {
    let words = |list: &[&str]| -> Vec<String> { list.iter().map(|w| (*w).to_string()).collect() };
    let refused = parse_backup(&words(&["--into", "d", "--passphrase", "hunter2"])).expect_err("no flag");
    assert!(refused.contains("no --passphrase flag"), "{refused}");
    let refused = parse_restore(&words(&["--from", "d", "--passphrase", "hunter2"])).expect_err("no flag");
    assert!(refused.contains("no --passphrase flag"), "{refused}");
}

#[test]
fn the_usage_says_the_backup_is_encrypted_what_forgetting_costs_and_that_restore_restarts_nothing() {
    for usage in [backup_usage(), restore_usage()] {
        assert!(usage.contains("FORGOTTEN PASSPHRASE"), "{usage}");
        assert!(!usage.contains("NOT ENCRYPTED"), "{usage}");
    }
    assert!(backup_usage().contains("typed twice"));
    assert!(restore_usage().contains("NOTHING IS RESTARTED"));
    assert!(restore_usage().contains("different board"));
}
