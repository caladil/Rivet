//! Tests for [`crate::diff`] — `ADR-034` §6.2.
//!
//! **The case at the top is the one that was live on real hardware.** Against
//! the board on 2026-08-29 the composed document was missing `[control-plane]`
//! and `[services.rivet-mcpd]` at once, and one `apply document` would have
//! dropped the listener to loopback and turned the control plane off. Every
//! other test here exists to keep that one honest — mostly by proving the
//! refusal does *not* fire on the ordinary path, which is how a refusal earns
//! being read.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

// The comparison's own vocabulary, from the crate that owns it since
// 2026-09-05 — see this module's parent for why it moved.
use rivet_core::leaf::{Change, EMPTY, Leaf, Leaves, Scalar};

use crate::register::{Fleet, Interface};

/// A record complete enough to compose a document from.
fn device() -> Device {
    let mut device = Device {
        id: "0011223344556677".to_owned(),
        name: "kitchen-pi".to_owned(),
        ..Device::default()
    };
    device.interfaces.insert(
        "eth0".to_owned(),
        Interface {
            method: "static".to_owned(),
            address: Some("192.168.1.50/24".to_owned()),
            gateway: Some("192.168.1.1".to_owned()),
        },
    );
    device
}

fn composed(device: &Device) -> Leaves {
    let text = crate::compose::provisioning_toml(device, &Fleet::default())
        .expect("a composable record");
    leaves(&text).expect("canonical TOML")
}

fn machine(text: &str) -> Leaves {
    let document = rivet_config::Document::parse(text).expect("a document the board would take");
    leaves(&document.to_toml()).expect("canonical TOML")
}

// ---------------------------------------------------------------------------
// The measured case — ADR-034 6.2
// ---------------------------------------------------------------------------

/// **The scenario that was true of the only Rivet in existence**, 2026-08-29.
///
/// A register holding neither `listen` nor `services.rivet-mcpd`, against a
/// machine whose document has both. Applying it would have bound the control
/// plane to loopback AND turned it off, on a headless board. Both are named.
#[test]
fn a_register_lacking_listen_and_lacking_mcpd_refuses_and_names_both() {
    let device = device();
    assert!(device.listen.is_none(), "the register must not hold listen");
    assert!(
        !device.services.contains_key("rivet-mcpd"),
        "the register must not hold the service"
    );

    let board = machine(
        "version = 1\n\
         hostname = \"kitchen-pi\"\n\
         \n[services.rivet-mcpd]\nenabled = true\n\
         \n[network.eth0]\nmethod = \"static\"\naddress = \"192.168.1.50/24\"\n\
         gateway = \"192.168.1.1\"\n\
         \n[control-plane]\nlisten = \"0.0.0.0:7777\"\n",
    );

    let found = between(&board, &composed(&device));
    let refusal = refusal(&device, &found).expect("both losses must refuse");

    assert!(refusal.contains("control-plane.listen"), "{refusal}");
    assert!(refusal.contains("0.0.0.0:7777"), "{refusal}");
    assert!(refusal.contains("services.rivet-mcpd.enabled"), "{refusal}");
    // The remedy, per ADR-034 6.2 — the loss without it sends an operator to
    // edit the machine, which is the wrong side.
    assert!(
        refusal.contains("set listen --device kitchen-pi --listen 0.0.0.0:7777"),
        "{refusal}"
    );
    assert!(
        refusal.contains("set service --device kitchen-pi --service rivet-mcpd --enabled true"),
        "{refusal}"
    );
    // And what each loss costs, which is why this case is the one in the ADR.
    assert!(refusal.contains("LOOPBACK"), "{refusal}");
    assert!(refusal.contains("DOES NOT START"), "{refusal}");
    assert!(refusal.contains("NOTHING HAS BEEN WRITTEN"), "{refusal}");
}

/// **And the normal path stays normal.** A register that accounts for
/// everything the machine has applies with no refusal — `ADR-034` §6.2's
/// failure mode is a guard that fires on every ordinary reconfiguration, which
/// is how a refusal stops being read.
#[test]
fn a_complete_register_applies_without_refusing() {
    let mut device = device();
    device.listen = Some("0.0.0.0:7777".to_owned());
    device.services.insert("rivet-mcpd".to_owned(), true);

    let ours = composed(&device);
    // The machine is running exactly what this register composes — which is
    // what a board looks like the second time you apply to it.
    let text = crate::compose::provisioning_toml(&device, &Fleet::default()).expect("composable");
    let theirs = machine(&text);

    let found = between(&theirs, &ours);
    assert_eq!(found.losses, Vec::new(), "nothing should be lost");
    assert_eq!(found.changes, Vec::new(), "nothing should move");
    assert_eq!(refusal(&device, &found), None);
}

/// A register that holds the machine's values and **disagrees about one** still
/// applies. §6.2.1: a differing value is the operator's intent, and refusing
/// here would refuse every ordinary reconfiguration.
#[test]
fn a_changed_value_is_reported_and_does_not_refuse() {
    let mut device = device();
    device.listen = Some("0.0.0.0:7777".to_owned());
    device.services.insert("rivet-mcpd".to_owned(), false);

    let mut running = device.clone();
    running.services.insert("rivet-mcpd".to_owned(), true);
    let text = crate::compose::provisioning_toml(&running, &Fleet::default()).expect("composable");

    let found = between(&machine(&text), &composed(&device));
    assert_eq!(found.losses, Vec::new(), "a change is not a loss");
    assert_eq!(
        found.changes,
        vec![Change {
            path: vec!["services".to_owned(), "rivet-mcpd".to_owned(), "enabled".to_owned()],
            machine: "true".to_owned(),
            register: "false".to_owned(),
        }]
    );
    assert_eq!(refusal(&device, &found), None);
    let said = changes(&found).expect("a change is reported even though it does not refuse");
    assert!(said.contains("services.rivet-mcpd.enabled"), "{said}");
    assert!(said.contains("true -> false"), "{said}");
}

/// **An empty table is a declaration and flattens to a leaf.** A table written
/// with nothing under it is a statement the operator made deliberately;
/// flattened as nothing at all it would be invisible here and applying a
/// document that dropped it would look like a no-op.
#[test]
fn an_empty_table_is_a_leaf_rather_than_nothing() {
    let found = leaves("[volumes.data]\n").expect("TOML");
    assert_eq!(
        found.get(&vec!["volumes".to_owned(), "data".to_owned()]),
        Some(&Leaf::Many(Vec::new()))
    );

    let diff = between(&found, &Leaves::new());
    assert_eq!(diff.losses.len(), 1);
    assert_eq!(diff.losses[0].value, EMPTY);
}

/// A key an operator wrote as a dotted key, an inline table or a `[table]`
/// header is **one leaf at one path**. The comparison is over the document,
/// and how somebody typed it is not a value.
#[test]
fn three_spellings_of_one_key_flatten_to_one_path() {
    let want = vec![vec!["control-plane".to_owned(), "listen".to_owned()]];
    for text in [
        "[control-plane]\nlisten = \"0.0.0.0:7777\"\n",
        "control-plane = { listen = \"0.0.0.0:7777\" }\n",
        "control-plane.listen = \"0.0.0.0:7777\"\n",
    ] {
        let found = leaves(text).unwrap_or_else(|e| panic!("{text} did not parse: {e}"));
        assert_eq!(found.keys().cloned().collect::<Vec<_>>(), want, "{text}");
        assert_eq!(
            found.values().cloned().collect::<Vec<_>>(),
            vec![Leaf::One(Scalar::string("0.0.0.0:7777"))],
            "{text}"
        );
    }
}

/// A scalar arrives **unquoted**, because the value goes straight into the
/// command line the refusal prints and a quoted one would be pasted with its
/// quotes.
#[test]
fn a_scalar_is_flattened_as_an_operator_would_type_it() {
    let found = leaves("a = \"text\"\nb = true\nc = 7\n").expect("TOML");
    assert_eq!(found[&vec!["a".to_owned()]], Leaf::One(Scalar::string("text")));
    assert_eq!(found[&vec!["b".to_owned()]], Leaf::One(Scalar::bare("true")));
    assert_eq!(found[&vec!["c".to_owned()]], Leaf::One(Scalar::bare("7")));
}

/// Adding is what applying a document is for, so it is not reported as
/// anything. **Directional, deliberately.**
#[test]
fn a_value_the_register_adds_is_not_a_finding() {
    let theirs = leaves("version = 1\n").expect("TOML");
    let ours = leaves("version = 1\n\n[control-plane]\nlisten = \"0.0.0.0:7777\"\n").expect("TOML");
    assert_eq!(between(&theirs, &ours), Diff::default());
}

/// Text that is not TOML is a refusal carrying the parser's own words, not a
/// silent empty comparison that would report no losses (`AGENTS.md` §5).
#[test]
fn text_that_is_not_toml_refuses_rather_than_comparing_as_empty() {
    let error = leaves("this is not = = toml\n").expect_err("not TOML");
    assert!(error.contains("not TOML"), "{error}");
}

// ---------------------------------------------------------------------------
// The remedies — ADR-034 6.2 requires the command beside the loss
// ---------------------------------------------------------------------------

fn loss(path: &[&str], value: &str) -> Loss {
    Loss {
        path: path.iter().map(|s| (*s).to_owned()).collect(),
        value: value.to_owned(),
        whole: true,
    }
}

/// Every remedy this tool can name is a command that **actually exists in the
/// operation table**. A refusal naming a command the tool does not have would
/// be worse than one naming none.
#[test]
fn every_command_a_remedy_names_is_an_operation_this_tool_has() {
    let device = device();
    let cases = [
        loss(&["control-plane", "listen"], "0.0.0.0:7777"),
        loss(&["services", "rivet-mdnsd", "enabled"], "true"),
        loss(&["services", "rivet-mdnsd", "memory"], "64M"),
    ];
    for case in &cases {
        let Remedy::Command(text) = remedy(&device, case) else {
            panic!("{} should have a command", case.where_it_is());
        };
        for line in text.lines() {
            let words: Vec<&str> = line.split_whitespace().collect();
            let Some(index) = words.iter().position(|w| *w == "rivet-fleet") else {
                continue;
            };
            let verb = words[index + 1];
            let subject = words[index + 2];
            let found = rivet_core::operation::Verb::ALL
                .iter()
                .copied()
                .find(|v| v.as_str() == verb)
                .and_then(|v| crate::operation::find(v, subject));
            assert!(
                found.is_some(),
                "`{verb} {subject}` is named by a remedy and is not in the operation table"
            );
        }
    }
}

/// **Where there is no command, the refusal says so** rather than naming one
/// that would do something else. `add repo` mints a new identifier — the same
/// shape as `add identity` versus `set pin` (`ADR-034` §6.1.1) — and telling
/// an operator to run it would lose the identifier the machine already serves.
#[test]
fn a_loss_with_no_verb_says_so_rather_than_naming_the_wrong_one() {
    let device = device();
    let by_hand = [
        (loss(&["repos", "notes", "id"], "abcdef0123456789"), "MINTS A NEW IDENTIFIER"),
        (loss(&["network", "eth0", "address"], "10.0.0.2/24"), "no verb writes an interface"),
        (loss(&["volumes", "data", "guid"], "0000"), "NO FIELD FOR VOLUMES"),
        (loss(&["hostname"], "elsewhere"), "the composed hostname IS the device's label"),
    ];
    for (case, expected) in &by_hand {
        let found = remedy(&device, case);
        assert!(
            matches!(found, Remedy::ByHand(_)),
            "{} should have no command",
            case.where_it_is()
        );
        assert!(found.said().contains(expected), "{}", found.said());
    }
}

/// The refusal names **every** loss, not a count and not the first — `ADR-034`
/// §6.2: *"an operator who has to discover them one at a time will stop before
/// the last."*
#[test]
fn the_refusal_names_every_loss_individually() {
    let device = device();
    let found = Diff {
        losses: vec![
            loss(&["control-plane", "listen"], "0.0.0.0:7777"),
            loss(&["services", "rivet-mdnsd", "enabled"], "true"),
            loss(&["repos", "notes", "id"], "abcdef0123456789"),
        ],
        changes: Vec::new(),
    };
    let refusal = refusal(&device, &found).expect("three losses refuse");
    for path in ["control-plane.listen", "services.rivet-mdnsd.enabled", "repos.notes.id"] {
        assert!(refusal.contains(path), "{path} is missing from:\n{refusal}");
    }
    // And it points at the one command that clears all three at once.
    assert!(refusal.contains("add document --device kitchen-pi --confirm"), "{refusal}");
}

/// Nothing lost is no refusal at all — the guard is silent on the path it does
/// not own.
#[test]
fn no_losses_is_no_refusal() {
    assert_eq!(refusal(&device(), &Diff::default()), None);
    assert_eq!(changes(&Diff::default()), None);
}
