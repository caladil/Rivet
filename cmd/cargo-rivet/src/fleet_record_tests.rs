// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::cell::RefCell;
use std::path::PathBuf;

use rivet_core::package::format::HEADER_BYTES;
use rivetos_register::Device;

use super::*;

const ID: &str = "0011223344556677";
const PIN: &str = "ce043141e072b6c27e929c313ebd8b9b8e2025eaecf2d3d9720f784adf135c0e";

fn scratch(label: &str) -> PathBuf {
    crate::scratch::dir(&format!("fleet-record-{label}"))
}

/// A machine answering `get slot` and `get file` for each slot's retained
/// header. Hand-composed answers in the shape `rivet_core::operation::slot`
/// and `file` compose.
struct Board {
    running: &'static str,
    committed: Option<bool>,
    headers: Vec<(&'static str, Vec<u8>)>,
    asked: RefCell<Vec<String>>,
}

impl CallsTools for Board {
    fn call_tool(&self, _tool: &str, arguments: &str) -> Result<String, String> {
        self.asked.borrow_mut().push(arguments.to_owned());
        if arguments.contains(r#""subject":"slot""#) {
            let committed = self.committed.map_or_else(String::new, |c| format!(r#","committed":{c}"#));
            return Ok(format!(
                r#"{{"subject":"slot","running":"{r}","inactive":"x","in_trial_boot":false{committed},"means":"fixture"}}"#,
                r = self.running
            ));
        }
        for (slot, header) in &self.headers {
            let path = format!("/state/slots/{slot}/package-header");
            if arguments.contains(&path) {
                return Ok(format!(
                    r#"{{"subject":"file","path":"{path}","present":true,"type":"file","size_bytes":{},"content":"{}","content_encoding":"base64"}}"#,
                    header.len(),
                    rivet_core::base64::encode(header)
                ));
            }
        }
        if let Some(slot) = ["a", "b"].iter().find(|s| arguments.contains(&format!("/state/slots/{s}/"))) {
            return Ok(format!(r#"{{"subject":"file","path":"/state/slots/{slot}/package-header","present":false}}"#));
        }
        Err(format!("this board does not answer {arguments}"))
    }
}

/// An update object of `HEADER_BYTES` of `fill` followed by a body, published
/// as a manifest entry — hand-composed: retention and the header match read
/// only the manifest's fields and the object's first bytes.
fn update_entry(root: &Path, fill: u8, version: &str) -> (String, Vec<u8>) {
    let header = vec![fill; HEADER_BYTES];
    let mut bytes = header.clone();
    bytes.extend_from_slice(b"the system member");
    let digest = format!("{:02x}", fill).repeat(32);
    std::fs::create_dir_all(root.join(OBJECTS_DIR)).unwrap();
    std::fs::write(root.join(OBJECTS_DIR).join(&digest), &bytes).unwrap();
    let mut entries = read_manifest(root).unwrap();
    entries.push(Entry {
        digest: digest.clone(),
        product: "rivet".to_owned(),
        version: version.to_owned(),
        arch: "aarch64".to_owned(),
        platform: Some("rpi4".to_owned()),
        kind: UPDATE_KIND.to_owned(),
        size: bytes.len() as u64,
        added: "2026-01-01T00:00:00Z".to_owned(),
        schema_version: None,
    });
    super::super::commit_manifest(root, &entries, &[], "fixture").unwrap();
    (digest, header)
}

/// A repository, and a register beside it holding one device with `PIN`.
fn rig(label: &str) -> (PathBuf, Register) {
    let root = scratch(label);
    super::super::init(&root).unwrap();
    let mut register = Register::empty(scratch(&format!("{label}-register")));
    register
        .enroll(Device {
            id: ID.to_owned(),
            name: "dev-board".to_owned(),
            pin: Some(PIN.to_owned()),
            ..Device::default()
        })
        .unwrap();
    rivetos_register::store::save(&register).unwrap();
    (root, register)
}

fn projected(root: &Path) -> rivetos_register::SystemSlots {
    let register = rivetos_register::store::load(&root.join(rivetos_fleet_repo::REGISTER_DIR)).unwrap();
    register.devices.get(ID).map(|d| d.system_slots.clone()).unwrap_or_default()
}

/// **A committed slot is recorded, in the register and its projection, and
/// the other slot keeps what it had** — the record then names both builds.
#[test]
fn a_committed_running_slot_is_recorded_and_the_other_slot_is_kept() {
    let (root, mut register) = rig("committed");
    let (old, _) = update_entry(&root, 0x11, "0.1.1");
    let (new, new_header) = update_entry(&root, 0x22, "0.1.2");
    register.devices.get_mut(ID).unwrap().system_slots.set("a", &old).unwrap();

    let board = Board { running: "b", committed: Some(true), headers: vec![("b", new_header)], asked: RefCell::default() };
    let report = record(&root, &mut register, ID, &board, true).expect("recorded");
    assert!(report.contains("recorded"), "{report}");

    let slots = projected(&root);
    assert_eq!(slots.b.as_deref(), Some(new.as_str()), "{report}");
    assert_eq!(slots.a.as_deref(), Some(old.as_str()), "the other slot keeps its entry");
    let saved = rivetos_register::store::load(&register.root).unwrap();
    assert_eq!(saved.devices[ID].system_slots, slots, "the register itself holds it too");
}

/// **An uncommitted trial records nothing** — R-233: a host's record names
/// only what has committed.
#[test]
fn an_uncommitted_install_records_nothing() {
    let (root, mut register) = rig("uncommitted");
    let (_, header) = update_entry(&root, 0x33, "0.1.3");
    for committed in [Some(false), None] {
        let board = Board { running: "b", committed, headers: vec![("b", header.clone())], asked: RefCell::default() };
        let refused = record(&root, &mut register, ID, &board, true).expect_err("not committed");
        assert!(refused.contains("NOTHING WAS RECORDED"), "{refused}");
    }
    assert!(register.devices[ID].system_slots.is_empty());
    assert!(projected(&root).is_empty(), "nothing projected");
    let saved = rivetos_register::store::load(&register.root).unwrap();
    assert!(saved.devices[ID].system_slots.is_empty(), "nothing saved");
}

/// Without `confirm` the machine is read and nothing is written.
#[test]
fn without_confirm_nothing_is_written() {
    let (root, mut register) = rig("plan");
    let (digest, header) = update_entry(&root, 0x44, "0.1.4");
    let board = Board { running: "a", committed: Some(true), headers: vec![("a", header)], asked: RefCell::default() };
    let plan = record(&root, &mut register, ID, &board, false).expect("a plan");
    assert!(plan.contains("would record"), "{plan}");
    assert!(plan.contains(&digest), "{plan}");
    assert!(register.devices[ID].system_slots.is_empty());
    assert!(projected(&root).is_empty());
}

/// A slot with no retained header, or a build this repository does not hold,
/// records nothing: neither can say which update it is.
#[test]
fn an_unreadable_or_unpublished_build_records_nothing() {
    let (root, mut register) = rig("unknown");
    update_entry(&root, 0x55, "0.1.5");
    let bare = Board { running: "a", committed: Some(true), headers: vec![], asked: RefCell::default() };
    let refused = record(&root, &mut register, ID, &bare, true).expect_err("no header");
    assert!(refused.contains("retains no header"), "{refused}");

    let elsewhere = Board { running: "a", committed: Some(true), headers: vec![("a", vec![0x99; HEADER_BYTES])], asked: RefCell::default() };
    let refused = record(&root, &mut register, ID, &elsewhere, true).expect_err("not published here");
    assert!(refused.contains("fleet publish"), "{refused}");
    assert!(register.devices[ID].system_slots.is_empty());
}

/// **A record is found, never invented**: by name, or by the machine's pin;
/// a pin no record carries is refused before the machine is asked anything.
#[test]
fn a_record_is_found_by_name_or_pin_and_never_invented() {
    let (_, register) = rig("which");
    assert_eq!(which_record(&register, Some("dev-board"), &Endpoint::default()).unwrap(), ID);
    let by_pin = Endpoint { pin: Some(PIN.to_uppercase()), ..Endpoint::default() };
    assert_eq!(which_record(&register, None, &by_pin).unwrap(), ID);
    let stranger = Endpoint { pin: Some("ab".repeat(32)), ..Endpoint::default() };
    let refused = which_record(&register, None, &stranger).expect_err("a pin no record carries");
    assert!(refused.contains("never invents"), "{refused}");
    assert!(which_record(&register, Some("nonesuch"), &Endpoint::default()).is_err());
}

/// `fleet record` takes push's grammar: a device or the three flags, `--at`,
/// and a trailing `confirm`; `--prune` is sync's alone.
#[test]
fn the_verb_parses_with_pushs_grammar() {
    let words = |w: &[&str]| w.iter().map(|s| (*s).to_owned()).collect::<Vec<String>>();
    match super::super::parse(&words(&["record", "dev-board", "--at", "./f", "confirm"])).expect("parses") {
        super::super::FleetArgs { at, verb: super::super::SubVerb::Record { device, confirm, .. } } => {
            assert_eq!(device.as_deref(), Some("dev-board"));
            assert!(confirm);
            assert_eq!(at, Some(PathBuf::from("./f")));
        }
        _ => panic!("not a record"),
    }
    assert!(super::super::parse(&words(&["record", "dev-board", "--prune"])).is_err());
}

/// **What `fleet record` writes, a publish keeps** — the record and retention
/// tied together: four more updates published after a commit is recorded
/// leave both recorded digests in the repository and prune the unrecorded.
#[test]
fn a_recorded_update_survives_the_publishes_that_follow() {
    let (root, mut register) = rig("survives");
    let (a, a_header) = update_entry(&root, 0x01, "0.1.1");
    let (b, b_header) = update_entry(&root, 0x02, "0.1.2");
    let first = Board { running: "a", committed: Some(true), headers: vec![("a", a_header)], asked: RefCell::default() };
    record(&root, &mut register, ID, &first, true).expect("slot a");
    let second = Board { running: "b", committed: Some(true), headers: vec![("b", b_header)], asked: RefCell::default() };
    record(&root, &mut register, ID, &second, true).expect("slot b");

    let mut entries = read_manifest(&root).unwrap();
    for (fill, version) in [(0x03, "0.1.3"), (0x04, "0.1.4"), (0x05, "0.1.5"), (0x06, "0.1.6")] {
        let digest = update_entry(&root, fill, version).0;
        entries = read_manifest(&root).unwrap();
        let named: std::collections::BTreeSet<String> = super::super::named_by_hosts(&root).unwrap().into_keys().collect();
        let (dropped, _) = super::super::prune_keeping_three(&mut entries, &digest, &named);
        if !dropped.is_empty() {
            super::super::commit_manifest(&root, &entries, &[], "prune").unwrap();
        }
    }
    let digests: Vec<&str> = entries.iter().map(|e| e.digest.as_str()).collect();
    assert!(digests.contains(&a.as_str()), "{digests:?}");
    assert!(digests.contains(&b.as_str()), "{digests:?}");
    assert_eq!(entries.len(), 5, "three newest plus the two recorded: {digests:?}");
}
