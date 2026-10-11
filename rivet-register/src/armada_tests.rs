//! The Armada Registry's tests — `crate::store`'s round-trip properties, one
//! level up. Hermetic: a real scratch directory under the OS temp dir, removed
//! at the end, the same `Scratch` the register's own store tests use rather
//! than a `tempfile` dependency.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::fs;
use std::path::PathBuf;

use super::store;
use super::{Armada, DEFAULT_KEEP, Fleet, Server, Settings};

/// A real directory under the OS temporary directory, removed at the end.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("rivet-armada-{label}-{n}"));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("a scratch directory");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fleet(name: &str, host: &str) -> Fleet {
    Fleet {
        name: name.to_owned(),
        server: Server { host: host.to_owned(), pin: "ab".repeat(16), operator_key: None },
    }
}

#[test]
fn an_armada_round_trips_through_the_store() {
    let scratch = Scratch::new("roundtrip");
    let mut armada = Armada::empty(scratch.0.clone());
    armada.settings = Settings { backup_dir: Some(scratch.0.join("backups")), keep: 5 };
    let a = fleet("home", "rivet-01.local:7777");
    armada.fleets.insert(a.name.clone(), a);
    armada.fleets.insert("lab".to_owned(), fleet("lab", "rivet-lab.local:7777"));

    store::save(&armada).expect("writes");
    let back = store::load(&scratch.0).expect("reads");
    assert_eq!(back, armada);
}

/// A first run has nothing to read and must still work.
#[test]
fn an_empty_registry_loads_as_default() {
    let scratch = Scratch::new("empty");
    let root = scratch.0.join("never-written");
    let armada = store::load(&root).expect("reads");
    assert_eq!(armada, Armada::empty(root));
    assert_eq!(armada.settings.keep, DEFAULT_KEEP);
    assert!(armada.fleets.is_empty());
}

/// A Fleet filed under a name other than its own is a hand-rename, and refused
/// rather than trusted.
#[test]
fn a_fleet_filed_under_the_wrong_name_is_refused() {
    let scratch = Scratch::new("misfiled");
    let dir = store::fleets_dir(&scratch.0);
    fs::create_dir_all(&dir).expect("dir");
    // The record says `home`; the file is named `elsewhere`.
    let text = toml_edit::ser::to_string_pretty(&fleet("home", "h:7777")).expect("renders");
    fs::write(dir.join("elsewhere.toml"), text).expect("writes");
    let err = store::load(&scratch.0).expect_err("refused");
    assert!(matches!(err, crate::RegisterError::Malformed { .. }), "{err:?}");
}

/// **A record that still carries the retired `monitor`/`backup` keys loads,
/// and saving it again drops them.** Rivet Monitor's own per-host switches are
/// the only authority for monitoring and backup (`ADR-059` §6), so the keys
/// mean nothing to the Armada — but a file written while they existed must not
/// stop the Armada loading. This pins `Fleet`'s lack of `deny_unknown_fields`.
#[test]
fn a_record_carrying_the_retired_monitor_and_backup_keys_loads_and_resaves_without_them() {
    let scratch = Scratch::new("retired-keys");
    let dir = store::fleets_dir(&scratch.0);
    fs::create_dir_all(&dir).expect("dir");
    let pin = "aa".repeat(32);
    fs::write(
        dir.join("home.toml"),
        format!(
            "name = \"home\"\nmonitor = false\nbackup = false\n\n[server]\n\
             host = \"192.0.2.10:7777\"\npin = \"{pin}\"\noperator_key = \"{key}\"\n",
            key = "bb".repeat(32),
        ),
    )
    .expect("writes");

    let armada = store::load(&scratch.0).expect("a record with the retired keys still reads");
    let home = &armada.fleets["home"];
    assert_eq!(home.name, "home");
    assert_eq!(home.server.host, "192.0.2.10:7777");
    assert_eq!(home.server.pin, pin);
    assert_eq!(home.server.operator_key.as_deref(), Some("bb".repeat(32).as_str()));

    store::save(&armada).expect("writes");
    let text = fs::read_to_string(dir.join("home.toml")).expect("the fleet's file");
    assert!(!text.contains("monitor"), "{text}");
    assert!(!text.contains("backup"), "{text}");
    assert!(text.contains("192.0.2.10:7777"), "{text}");
    assert_eq!(store::load(&scratch.0).expect("re-reads"), armada);
}

/// **A Fleet's `operator_key` round-trips through the store when present.**
/// This is the key a member joining the Fleet must be staged to trust
/// (`docs/PLAN.md` §183.2); losing it in the store would silently reopen the
/// enroll trust gap.
#[test]
fn a_fleets_operator_key_round_trips_when_present() {
    let scratch = Scratch::new("operator-key-present");
    let mut armada = Armada::empty(scratch.0.clone());
    let mut a = fleet("home", "rivet-01.local:7777");
    a.server.operator_key = Some("cd".repeat(32));
    armada.fleets.insert(a.name.clone(), a);

    store::save(&armada).expect("writes");
    let back = store::load(&scratch.0).expect("reads");
    assert_eq!(back, armada);
    assert_eq!(
        back.fleets["home"].server.operator_key.as_deref(),
        Some("cd".repeat(32).as_str()),
    );
}

/// **A record written before `operator_key` existed omits it, and must still
/// load — with the key absent, never invented.** `skip_serializing_if` also
/// means such a Fleet re-saves without the key rather than materialising an
/// empty one.
#[test]
fn operator_key_defaults_to_absent_when_a_record_omits_it() {
    let scratch = Scratch::new("operator-key-absent");
    let dir = store::fleets_dir(&scratch.0);
    fs::create_dir_all(&dir).expect("dir");
    fs::write(
        dir.join("home.toml"),
        "name = \"home\"\n[server]\nhost = \"h:7777\"\npin = \"abcd\"\n",
    )
    .expect("writes");
    let armada = store::load(&scratch.0).expect("reads");
    assert_eq!(armada.fleets["home"].server.operator_key, None);

    // And re-rendering omits the key entirely rather than writing an empty one.
    let rendered = store::render(&armada).expect("renders");
    let (_, text) = rendered
        .iter()
        .find(|(p, _)| p.ends_with("home.toml"))
        .expect("the fleet's file");
    assert!(!text.contains("operator_key"), "{text}");
    assert!(!text.contains("operator-key"), "{text}");
}

/// `armada.toml` that names no `keep` gets [`DEFAULT_KEEP`], not zero.
#[test]
fn settings_keep_defaults_to_three_when_absent() {
    let scratch = Scratch::new("keep");
    fs::write(store::settings_path(&scratch.0), "").expect("writes empty settings");
    let armada = store::load(&scratch.0).expect("reads");
    assert_eq!(armada.settings.keep, DEFAULT_KEEP);
    assert_eq!(armada.settings.backup_dir, None);
}

// ---------------------------------------------------------------------------
// a Fleet name that is not exactly one plain path component
// ---------------------------------------------------------------------------

/// **`forget` and `save` refuse a name that escapes `fleets/`, before
/// touching anything.** A sentinel `armada.toml` — the file `../armada` would
/// actually select — and a sibling Fleet record both stay byte-for-byte
/// unchanged across every refusal.
#[test]
fn forget_and_save_refuse_a_name_that_escapes_the_fleets_directory() {
    let scratch = Scratch::new("unsafe-names");
    let settings = store::settings_path(&scratch.0);
    fs::write(&settings, "keep = 4\n").expect("write the settings sentinel");
    let dir = store::fleets_dir(&scratch.0);
    fs::create_dir_all(&dir).expect("fleets dir");
    let sibling = dir.join("safe.toml");
    let sibling_text = toml_edit::ser::to_string_pretty(&fleet("safe", "h:7777")).expect("renders");
    fs::write(&sibling, &sibling_text).expect("write the sibling sentinel");

    let mut bad_names = vec![
        "../armada".to_owned(),
        "a/b".to_owned(),
        scratch.0.join("elsewhere").to_string_lossy().into_owned(),
    ];
    if cfg!(windows) {
        bad_names.push("C:\\evil".to_owned());
    }

    for name in &bad_names {
        let err =
            store::forget(&scratch.0, name).expect_err(&format!("{name:?} must be refused by forget"));
        assert!(matches!(err, crate::RegisterError::InvalidName { .. }), "{name:?}: {err:?}");

        let mut armada = Armada::empty(scratch.0.clone());
        armada.fleets.insert(name.clone(), fleet(name, "h:7777"));
        let err = store::save(&armada).expect_err(&format!("{name:?} must be refused by save"));
        assert!(matches!(err, crate::RegisterError::InvalidName { .. }), "{name:?}: {err:?}");

        assert_eq!(
            fs::read_to_string(&settings).expect("settings sentinel readable"),
            "keep = 4\n",
            "{name:?} must not have touched armada.toml"
        );
        assert_eq!(
            fs::read_to_string(&sibling).expect("sibling sentinel readable"),
            sibling_text,
            "{name:?} must not have touched the sibling record"
        );
    }
}

/// **Ordinary names still work** — the non-regression control for the
/// boundary check above.
#[test]
fn forget_and_save_still_work_for_an_ordinary_name() {
    let scratch = Scratch::new("ordinary-name");
    let mut armada = Armada::empty(scratch.0.clone());
    armada.fleets.insert("home".to_owned(), fleet("home", "rivet-01.local:7777"));
    store::save(&armada).expect("an ordinary name saves");
    let back = store::load(&scratch.0).expect("and reads back");
    assert_eq!(back.fleets["home"].server.host, "rivet-01.local:7777");

    store::forget(&scratch.0, "home").expect("and forgets");
    assert!(store::load(&scratch.0).expect("reads").fleets.is_empty());
}

/// **Forgetting an absent record that is itself a safe name is not an
/// error** — `crate::store::forget`'s own rule, restated here: the requested
/// state is the state.
#[test]
fn forgetting_an_absent_safe_fleet_is_not_an_error() {
    let scratch = Scratch::new("forget-absent-safe");
    store::forget(&scratch.0, "never-existed").expect("absence of a safe name is not an error");
}
