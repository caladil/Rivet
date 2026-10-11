//! Hermetic unit tests for `crate::armada` — the pure parts only, per this
//! worktree's brief: no terminal, no board, and never the maintainer's real
//! Armada Registry or device register. Every test that touches disk hands a
//! scratch directory to the function under test explicitly, mirroring
//! `crate::device`'s own tests module header.

// A test that cannot fail loudly is not a test — `crate::device`'s tests
// module takes the same allowance for the same reason.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use rivetos_register::{Device, FleetRole, Interface, Register};
use std::sync::atomic::{AtomicU32, Ordering};

/// A scratch directory under the OS temp dir, removed when it drops. Never
/// the maintainer's real register or Armada Registry — every test here
/// builds its own and points the function under test at it explicitly.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        Self(crate::scratch::dir(&format!("armada-test-{label}-{n}")))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|w| (*w).to_string()).collect()
}

/// A Server-role device with a static, routable interface address — the same
/// shape `crate::device::tests::device_with_address` fixtures, plus a role,
/// so resolving its host through [`crate::device::expand`] touches no
/// network (a recorded interface address is dialled as-is; only a DHCP
/// record with none falls back to mDNS).
fn server_device(id: &str, name: &str, listen: &str, address: &str) -> Device {
    let mut device = Device {
        id: id.to_owned(),
        name: name.to_owned(),
        fleet_role: Some(FleetRole::Server),
        listen: Some(listen.to_owned()),
        pin: Some("c0ffee".repeat(11)[..64].to_owned()),
        ..Device::default()
    };
    device.interfaces.insert(
        "eth0".to_owned(),
        Interface { method: "static".to_owned(), address: Some(format!("{address}/24")), gateway: None },
    );
    device
}

fn member_device(id: &str, name: &str) -> Device {
    Device { id: id.to_owned(), name: name.to_owned(), fleet_role: Some(FleetRole::Member), ..Device::default() }
}

fn register_with(devices: Vec<Device>) -> Scratch {
    let scratch = Scratch::new("register");
    let mut register = Register::empty(&scratch.0);
    for device in devices {
        register.enroll(device).expect("fresh");
    }
    rivetos_register::store::save(&register).expect("writable");
    scratch
}

fn a_fleet(name: &str, host: &str, pin: &str) -> Fleet {
    Fleet {
        name: name.to_owned(),
        server: Server { host: host.to_owned(), pin: pin.to_owned(), operator_key: None },
    }
}

// ---------------------------------------------------------------------------
// default_dir_with — the pure half of default_dir; never touches real env
// ---------------------------------------------------------------------------

#[test]
fn default_dir_composes_home_slash_dot_rivet_slash_armada() {
    let dir = default_dir_with(Some(PathBuf::from("/home/joel/.rivet")))
        .expect("a home was given");
    assert_eq!(dir, PathBuf::from("/home/joel/.rivet/armada"));
}

#[test]
fn default_dir_with_no_home_at_all_is_refused_by_name() {
    let refused = default_dir_with(None).expect_err("no HOME or USERPROFILE");
    assert!(refused.contains("HOME"), "{refused}");
    assert!(refused.contains("USERPROFILE"), "{refused}");
}

// ---------------------------------------------------------------------------
// flag_value — reading crate::device::expand's own output back
// ---------------------------------------------------------------------------

#[test]
fn flag_value_reads_the_word_after_the_flag() {
    let expanded = words(&["--host", "192.168.50.10:7777", "--pin", "ab"]);
    assert_eq!(flag_value(&expanded, "--host"), Some("192.168.50.10:7777".to_string()));
    assert_eq!(flag_value(&expanded, "--pin"), Some("ab".to_string()));
}

#[test]
fn flag_value_is_none_when_the_flag_is_absent_or_trailing_with_no_value() {
    let expanded = words(&["--host", "x"]);
    assert_eq!(flag_value(&expanded, "--pin"), None);
    assert_eq!(flag_value(&words(&["--host"]), "--host"), None);
}

// ---------------------------------------------------------------------------
// fleet_from_device — the Device -> Fleet mapping, never re-deriving
// crate::device::expand's own resolution
// ---------------------------------------------------------------------------

#[test]
fn fleet_from_device_takes_its_name_host_and_pin_off_the_devices_own_record() {
    let scratch = register_with(vec![server_device(
        "0011223344556677",
        "rivet-fleet-a",
        "0.0.0.0:7777",
        "192.168.50.20",
    )]);
    let fleet = fleet_from_device("rivet-fleet-a", &scratch.0).expect("resolves without a network hop");
    assert_eq!(
        fleet,
        Fleet {
            name: "rivet-fleet-a".to_string(),
            server: Server {
                host: "192.168.50.20:7777".to_string(),
                pin: "c0ffee".repeat(11)[..64].to_string(),
                // This server's record carries no fleet-operator key, so the
                // Fleet records none — the pre-field, backward-compatible case.
                operator_key: None,
            },
        }
    );
}

/// **A server whose record carries a fleet-operator key hands it to the
/// Fleet.** `docs/PLAN.md` §183.2: this is the key a member joining this Fleet
/// must be staged to trust, and `fleet_from_device` is the one place it crosses
/// from the device record into the Armada.
#[test]
fn fleet_from_device_carries_the_servers_operator_key_when_the_record_has_one() {
    let mut device = server_device("0011223344556677", "rivet-fleet-a", "0.0.0.0:7777", "192.168.50.20");
    device.operator_key = Some("cd".repeat(32));
    let scratch = register_with(vec![device]);
    let fleet = fleet_from_device("rivet-fleet-a", &scratch.0).expect("resolves without a network hop");
    assert_eq!(fleet.server.operator_key.as_deref(), Some("cd".repeat(32).as_str()));
}

#[test]
fn fleet_from_device_on_an_unknown_name_is_refused_naming_known_devices() {
    let scratch = register_with(vec![server_device(
        "0011223344556677",
        "rivet-fleet-a",
        "0.0.0.0:7777",
        "192.168.50.20",
    )]);
    let refused = fleet_from_device("nonesuch", &scratch.0).expect_err("no such device");
    assert!(refused.contains("nonesuch"), "{refused}");
    assert!(refused.contains("rivet-fleet-a"), "{refused}");
}

// ---------------------------------------------------------------------------
// resolve_server_device — the role gate `add` applies before fleet_from_device
// ---------------------------------------------------------------------------

#[test]
fn resolve_server_device_accepts_a_server_role_device() {
    let scratch = register_with(vec![server_device(
        "0011223344556677",
        "rivet-fleet-a",
        "0.0.0.0:7777",
        "192.168.50.20",
    )]);
    let register = rivetos_register::store::load(&scratch.0).expect("readable");
    let resolved =
        resolve_server_device(&register, &scratch.0, "rivet-fleet-a").expect("a Server device");
    assert_eq!(resolved, "rivet-fleet-a");
}

#[test]
fn resolve_server_device_refuses_a_member_role_device_by_name() {
    let scratch = register_with(vec![member_device("8899aabbccddeeff", "kitchen-pi")]);
    let register = rivetos_register::store::load(&scratch.0).expect("readable");
    let refused =
        resolve_server_device(&register, &scratch.0, "kitchen-pi").expect_err("a Member, not a Server");
    assert!(refused.contains("kitchen-pi"), "{refused}");
    assert!(refused.contains("not a Server"), "{refused}");
}

#[test]
fn resolve_server_device_refuses_an_unknown_name_listing_what_it_holds() {
    let scratch = register_with(vec![
        server_device("0011223344556677", "rivet-fleet-a", "0.0.0.0:7777", "192.168.50.20"),
        member_device("8899aabbccddeeff", "kitchen-pi"),
    ]);
    let register = rivetos_register::store::load(&scratch.0).expect("readable");
    let refused =
        resolve_server_device(&register, &scratch.0, "nonesuch").expect_err("no such device");
    assert!(refused.contains("nonesuch"), "{refused}");
    assert!(refused.contains("rivet-fleet-a"), "{refused}");
    assert!(refused.contains("kitchen-pi"), "{refused}");
}

// ---------------------------------------------------------------------------
// render_list — pure, given a fixture Armada
// ---------------------------------------------------------------------------

#[test]
fn render_list_of_an_empty_armada_says_so_and_names_how_to_add_one() {
    let armada = Armada::empty(PathBuf::from("/scratch/armada"));
    let text = render_list(&armada);
    assert!(text.contains("/scratch/armada"), "{text}");
    assert!(text.contains("Add a Fleet"), "{text}");
    assert!(text.contains("armada add"), "{text}");
}

/// **Name and host, and nothing about Rivet Monitor.** Whether a host is
/// monitored or backed up is Rivet Monitor's own per-host setting (`ADR-059`
/// §6); a column here would display a value nothing reads.
#[test]
fn render_list_shows_name_and_host_per_fleet_and_no_monitor_or_backup_column() {
    let mut armada = Armada::empty(PathBuf::from("/scratch/armada"));
    let fleet = a_fleet("rivet-fleet-a", "192.168.50.20:7777", "ab");
    armada.fleets.insert(fleet.name.clone(), fleet);
    let text = render_list(&armada);
    assert!(text.contains("rivet-fleet-a"), "{text}");
    assert!(text.contains("192.168.50.20:7777"), "{text}");
    assert!(!text.contains("monitor"), "{text}");
    assert!(!text.contains("backup"), "{text}");
}

// ---------------------------------------------------------------------------
// add_fleet_record / forget_fleet_record / settings —
// round trips against a scratch Armada Registry, never the real one
// ---------------------------------------------------------------------------

#[test]
fn add_fleet_record_writes_a_new_fleet_and_reads_it_back() {
    let scratch = Scratch::new("armada-add-new");
    let report = add_fleet_record(&scratch.0, a_fleet("rivet-fleet-a", "192.168.50.20:7777", "ab"))
        .expect("writable");
    assert!(report.contains("Added Fleet"), "{report}");
    let armada = load(&scratch.0).expect("readable");
    assert_eq!(armada.fleets.get("rivet-fleet-a").map(|f| f.server.host.clone()), Some("192.168.50.20:7777".to_string()));
}

#[test]
fn add_fleet_record_over_an_existing_fleet_refreshes_its_server() {
    let scratch = Scratch::new("armada-add-refresh");
    add_fleet_record(&scratch.0, a_fleet("rivet-fleet-a", "192.168.50.20:7777", "ab")).expect("writable");

    let report = add_fleet_record(&scratch.0, a_fleet("rivet-fleet-a", "192.168.50.21:7777", "cd"))
        .expect("writable");
    assert!(report.contains("Refreshed Fleet"), "{report}");

    let armada = load(&scratch.0).expect("readable");
    let fleet = armada.fleets.get("rivet-fleet-a").expect("still there");
    assert_eq!(fleet.server.host, "192.168.50.21:7777", "the server was refreshed");
    assert_eq!(fleet.server.pin, "cd");
}

#[test]
fn forget_fleet_record_removes_it_and_is_idempotent() {
    let scratch = Scratch::new("armada-forget");
    add_fleet_record(&scratch.0, a_fleet("rivet-fleet-a", "192.168.50.20:7777", "ab")).expect("writable");
    let report = forget_fleet_record(&scratch.0, "rivet-fleet-a").expect("forgettable");
    assert!(report.contains("Forgot Fleet"), "{report}");
    let armada = load(&scratch.0).expect("readable");
    assert!(!armada.fleets.contains_key("rivet-fleet-a"));

    // Forgetting an already-absent Fleet is not an error — `store::forget`'s
    // own rule, restated: the requested state is the state.
    forget_fleet_record(&scratch.0, "rivet-fleet-a").expect("idempotent");
}

/// **`armada forget ../armada` must not delete the Armada's own settings
/// file.** `../armada` is exactly the record name whose computed path lands
/// on `armada.toml` itself (`fleets/../armada.toml`); this drives the same
/// parser `cargo rivet armada forget` uses, then the same store-backed
/// function `run` dispatches to, and proves the settings file this scratch
/// Armada started with survives untouched.
#[test]
fn armada_forget_through_the_parsed_cli_argument_refuses_a_name_that_escapes_fleets() {
    let args = parse(&words(&["forget", "../armada"])).expect("parses");
    let SubVerb::Forget { name } = args.verb else {
        panic!("expected Forget");
    };
    assert_eq!(name, "../armada", "the parser must hand the argument through unchanged");

    let scratch = Scratch::new("armada-forget-traversal");
    let settings = store::settings_path(&scratch.0);
    std::fs::write(&settings, "keep = 9\n").expect("write the Armada's own settings file");

    let err = forget_fleet_record(&scratch.0, &name)
        .expect_err("a name that escapes fleets/ must be refused");
    assert!(err.contains("../armada"), "{err}");
    assert_eq!(
        std::fs::read_to_string(&settings).expect("armada.toml must still be there"),
        "keep = 9\n",
        "the Armada's own settings file must be untouched"
    );
}

#[test]
fn set_backup_dir_and_set_keep_records_persist() {
    let scratch = Scratch::new("armada-settings");
    set_backup_dir_record(&scratch.0, Path::new("/backups/armada")).expect("writable");
    set_keep_record(&scratch.0, 7).expect("writable");
    let armada = load(&scratch.0).expect("readable");
    assert_eq!(armada.settings.backup_dir, Some(PathBuf::from("/backups/armada")));
    assert_eq!(armada.settings.keep, 7);
}

// ---------------------------------------------------------------------------
// run_interview_with — only the no-terminal refusal is testable without a
// real terminal; the menu loop itself is unproven by this suite (see the
// worktree's brief and this crate's final report).
// ---------------------------------------------------------------------------

#[test]
fn the_interview_refuses_with_no_terminal_and_names_every_scripted_verb() {
    let refused = run_interview_with(false).expect_err("no terminal");
    for verb in ["armada list", "armada add", "armada forget", "armada set-backup-dir", "armada set-keep"] {
        assert!(refused.contains(verb), "{refused}");
    }
    assert!(!refused.contains("toggle"), "{refused}");
}

// ---------------------------------------------------------------------------
// the scripted twin's parser
// ---------------------------------------------------------------------------

#[test]
fn parse_list_takes_no_arguments() {
    let args = parse(&words(&["list"])).expect("parses");
    assert!(matches!(args.verb, SubVerb::List));
}

#[test]
fn parse_list_with_an_extra_argument_is_refused() {
    let refused = parse(&words(&["list", "extra"])).expect_err("no arguments");
    assert!(refused.contains("extra"), "{refused}");
}

#[test]
fn parse_add_takes_one_device_name() {
    let args = parse(&words(&["add", "rivet-fleet-a"])).expect("parses");
    assert!(matches!(args.verb, SubVerb::Add { device } if device == "rivet-fleet-a"));
}

#[test]
fn parse_add_with_no_device_name_is_refused() {
    let refused = parse(&words(&["add"])).expect_err("needs a device name");
    assert!(refused.contains("needs"), "{refused}");
}

#[test]
fn parse_add_with_two_names_is_refused() {
    let refused = parse(&words(&["add", "a", "b"])).expect_err("one device only");
    assert!(refused.contains('b'), "{refused}");
}

#[test]
fn parse_forget_takes_one_fleet_name() {
    let args = parse(&words(&["forget", "rivet-fleet-a"])).expect("parses");
    assert!(matches!(args.verb, SubVerb::Forget { name } if name == "rivet-fleet-a"));
}

/// **The retired Rivet Monitor verbs are refused as unknown verbs.** Whether a
/// host is monitored or backed up is Rivet Monitor's own per-host switch
/// (`ADR-059` §6), so the Armada has no verb for it; a script still calling
/// one gets the ordinary unknown-verb refusal rather than a silent success.
#[test]
fn parse_refuses_the_retired_toggle_monitor_and_toggle_backup_as_unknown_verbs() {
    for verb in ["toggle-monitor", "toggle-backup"] {
        let refused = parse(&words(&[verb, "rivet-fleet-a"])).expect_err("not a verb any more");
        assert!(refused.contains(&format!("`armada {verb}` is not a verb")), "{refused}");
    }
}

#[test]
fn parse_set_backup_dir_takes_one_path() {
    let args = parse(&words(&["set-backup-dir", "/backups/armada"])).expect("parses");
    assert!(matches!(args.verb, SubVerb::SetBackupDir { path } if path == Path::new("/backups/armada")));
}

#[test]
fn parse_set_keep_takes_one_whole_number() {
    let args = parse(&words(&["set-keep", "5"])).expect("parses");
    assert!(matches!(args.verb, SubVerb::SetKeep { keep } if keep == 5));
}

#[test]
fn parse_set_keep_with_a_non_number_is_refused() {
    let refused = parse(&words(&["set-keep", "five"])).expect_err("not a number");
    assert!(refused.contains("five"), "{refused}");
}

#[test]
fn parse_with_no_verb_at_all_is_refused_naming_the_verbs() {
    let refused = parse(&[]).expect_err("needs a verb");
    assert!(refused.contains("list"), "{refused}");
    assert!(refused.contains("add"), "{refused}");
}

#[test]
fn parse_an_unknown_verb_is_refused_by_name() {
    let refused = parse(&words(&["frobnicate"])).expect_err("not a verb");
    assert!(refused.contains("frobnicate"), "{refused}");
}

#[test]
fn parse_help_hands_back_usage() {
    let refused = parse(&words(&["-h"])).expect_err("usage, not a success");
    assert!(refused.contains("cargo rivet armada"), "{refused}");
}

// ---------------------------------------------------------------------------
// run — the scripted verbs actually reach the mutation, end to end
// ---------------------------------------------------------------------------

#[test]
fn usage_names_every_scripted_verb_and_the_default_location() {
    let text = usage();
    for verb in ["list", "add", "forget", "set-backup-dir", "set-keep"] {
        assert!(text.contains(verb), "{text}");
    }
    assert!(!text.contains("toggle"), "{text}");
    assert!(text.contains("~/.rivet/armada"), "{text}");
}
