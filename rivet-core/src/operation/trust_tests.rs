//! What `get trust` answers, and the two things it must never answer.
//!
//! **Every fixture here is hand-composed and labelled as such** (`AGENTS.md`
//! §4). The keys are invented byte patterns and the ids are not BLAKE3 of
//! anything — nothing in `rivet-core` can hash, and this module tests the
//! *reporting* rather than the derivation. That an adapter's id really is
//! `BLAKE3` of the key it sits beside is `cmd/rivet/src/trust.rs`'s test,
//! which drives it against `rivet-crypto`'s implementation of the port.
//!
//! What a board would still settle, past both: whether `/system/trust` on a
//! running Rivet parses at all. `AGENTS.md` §6.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::machine::{MachineError, TrustRead, TrustedKey};
use crate::package::port::{KeyId, PublicKey};

struct Fake(Result<TrustRead, MachineError>);

impl Fake {
    fn reading(read: TrustRead) -> Self {
        Self(Ok(read))
    }
    fn refusing(why: &str) -> Self {
        Self(Err(MachineError::Refused(why.to_string())))
    }
}

/// **One port answered and every other one refused** (`AGENTS.md` §3). Written
/// out rather than reached through `refuses_to_observe!`, because that macro
/// implements the whole trait and there would be nothing left to override.
impl crate::operation::machine::Observed for Fake {
    fn read_trust(&mut self) -> Result<TrustRead, MachineError> {
        match &self.0 {
            Ok(v) => Ok(v.clone()),
            Err(e) => Err(MachineError::Refused(e.why().to_string())),
        }
    }
    /// **Refuses rather than answering an idle machine.**
    fn read_cpu(&mut self) -> Result<crate::operation::machine::CpuRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering that this machine has no block devices.**
    fn read_devices(&mut self) -> Result<crate::operation::machine::DeviceRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering an empty network.**
    fn read_network(&mut self) -> Result<crate::operation::machine::NetworkRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than naming a slot.**
    fn read_slots(&mut self) -> Result<crate::operation::machine::SlotRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering an uptime of zero.**
    fn read_boot(&mut self) -> Result<crate::operation::machine::BootRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering with unset throttle flags.**
    fn read_thermal(&mut self) -> Result<crate::operation::machine::ThermalRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_processes(&mut self) -> Result<crate::operation::machine::ProcessRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_memory(&mut self) -> Result<crate::operation::machine::MemoryRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_desired_state(&mut self) -> Result<Option<String>, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn read_journal(&mut self) -> Result<crate::operation::machine::JournalRead, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn read_filesystems(&mut self) -> Result<crate::operation::machine::StorageRead, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn configuration_schema(&mut self) -> Value {
        Value::str("<not this test's port>")
    }
    fn supervisor_status(&mut self) -> Result<crate::operation::status::Status, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
}

/// Hand-composed. The bytes are a fill pattern, not a key anybody holds.
fn key(fill: u8, label: &str, kinds: &[&str]) -> TrustedKey {
    TrustedKey {
        id: KeyId([fill ^ 0x5a; 32]),
        key: PublicKey([fill; 32]),
        label: label.to_string(),
        may_sign: kinds.iter().map(|k| (*k).to_string()).collect(),
    }
}

fn store(keys: Vec<TrustedKey>) -> TrustRead {
    TrustRead {
        source: crate::package::trust_store::PATH.to_string(),
        keys,
    }
}

fn answer(read: TrustRead) -> Value {
    perform(&[SUBJECT], &mut Fake::reading(read)).expect("a reading is reported")
}

fn field<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
    let Value::Object(pairs) = value else {
        panic!("not an object: {value:?}");
    };
    pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| v)
}

/// **The ordinary reading**, and the field that makes the subject worth having:
/// the id a payload header names, in full.
#[test]
fn an_ordinary_store_reports_every_key_with_its_id_key_label_and_entitlement() {
    let read = store(vec![
        key(0x11, "rivet-system", &["update"]),
        key(0x22, "joel-apps", &["application"]),
    ]);
    let value = answer(read);

    assert_eq!(field(&value, "subject"), Some(&Value::str("trust")));
    assert_eq!(
        field(&value, "read_from"),
        Some(&Value::str(crate::package::trust_store::PATH))
    );
    assert_eq!(field(&value, "keys"), Some(&Value::Integer(2)));

    let Some(Value::List(entries)) = field(&value, "trusted") else {
        panic!("no trusted list: {value:?}");
    };
    assert_eq!(entries.len(), 2);
    assert_eq!(
        field(&entries[0], "label"),
        Some(&Value::str("rivet-system"))
    );
    assert_eq!(
        field(&entries[0], "may_sign"),
        Some(&Value::List(vec![Value::str("update")]))
    );
    assert_eq!(
        field(&entries[1], "label"),
        Some(&Value::str("joel-apps"))
    );
}

/// **The id is the WHOLE id.** `KeyId`'s `Display` prints eight bytes and an
/// ellipsis, because a journal line has room for eight bytes — and a truncated
/// value in a machine-readable answer whose only purpose is to be compared
/// against a header's field is worse than no value at all.
#[test]
fn the_key_id_is_reported_in_full_and_not_as_the_eight_byte_display_form() {
    let value = answer(store(vec![key(0x11, "rivet-system", &["update"])]));
    let Some(Value::List(entries)) = field(&value, "trusted") else {
        panic!("no trusted list");
    };
    let Some(Value::Str(id)) = field(&entries[0], "id") else {
        panic!("no id");
    };
    assert_eq!(id.len(), 64, "the id is not 32 bytes of hex: {id}");
    assert!(!id.contains('…'), "the id was truncated: {id}");
    assert_eq!(id, &crate::package::key_material::to_hex(&[0x11u8 ^ 0x5a; 32]));

    // The public key travels beside it, because the two answer different
    // questions: the id is what a HEADER names and the key is what an operator
    // has in a `.pub` file.
    let Some(Value::Str(k)) = field(&entries[0], "key") else {
        panic!("no key");
    };
    assert_eq!(k, &crate::package::key_material::to_hex(&[0x11u8; 32]));
    assert_ne!(id, k, "the id must not be the key itself");
}

/// **An empty store is a READING, not a failure.** `ADR-028` §5.5.5's
/// `--no-trust` image is exactly this, and the answer has to say what it costs
/// rather than looking like a machine nobody asked about.
#[test]
fn a_store_with_no_keys_is_reported_as_a_machine_that_trusts_nobody() {
    let value = answer(store(vec![]));
    assert_eq!(field(&value, "keys"), Some(&Value::Integer(0)));
    assert_eq!(field(&value, "trusted"), Some(&Value::List(vec![])));
    let Some(Value::Str(means)) = field(&value, "means") else {
        panic!("no means");
    };
    assert!(means.contains("TRUSTS NOBODY"), "{means}");
    assert!(means.contains("--no-trust"), "{means}");
    // And it says that this is not the unreadable case, because those two are
    // the pair a reader most needs kept apart.
    assert!(means.contains("could not be READ"), "{means}");
}

/// **A store that cannot be read is a refusal**, in `cmd/rivet-deploy`'s words
/// for the same file: *"a store that cannot be read is not a store that trusts
/// nobody."* This subject never turns one into the other.
#[test]
fn a_store_that_cannot_be_read_refuses_and_carries_the_machines_own_words() {
    let error = perform(
        &[SUBJECT],
        &mut Fake::refusing("/system/trust: No such file or directory"),
    )
    .expect_err("an unreadable store was reported as a reading");
    assert_eq!(error.code(), ErrorCode::MachineRefused);
    assert_eq!(error.subject(), Some(SUBJECT));
    assert!(
        error.message().contains("No such file or directory"),
        "{}",
        error.message()
    );
}

/// Every entitlement empty is the state a **revoked** entry is in before the
/// image carrying its removal is built (`ADR-028` §5.5.4), and it must not read
/// as an ordinary store — from an installer's point of view it is the same
/// position as an empty one.
#[test]
fn keys_entitled_to_nothing_are_said_to_be_entitled_to_nothing() {
    let value = answer(store(vec![key(0x33, "revoked", &[])]));
    let Some(Value::Str(means)) = field(&value, "means") else {
        panic!("no means");
    };
    assert!(means.contains("NOT ONE OF THEM MAY SIGN ANYTHING"), "{means}");
    assert!(means.contains("ADR-028 §5.5.4"), "{means}");

    let Some(Value::List(entries)) = field(&value, "trusted") else {
        panic!("no trusted list");
    };
    assert_eq!(
        field(&entries[0], "may_sign"),
        Some(&Value::List(vec![])),
        "an empty entitlement must be an empty list, not an absent field"
    );
}

/// **The caveat that is in every answer.** A trust store says which signatures
/// will be *considered*; it does not say what will install. A reader who took
/// this for the second would re-sign an artifact that was refused for its arch.
#[test]
fn every_answer_says_that_nothing_here_verifies_anything() {
    for read in [
        store(vec![]),
        store(vec![key(0x11, "rivet-system", &["update"])]),
        store(vec![key(0x33, "revoked", &[])]),
    ] {
        let value = answer(read);
        let Some(Value::Str(means)) = field(&value, "means") else {
            panic!("no means");
        };
        assert!(
            means.contains("NOTHING HERE VERIFIES ANYTHING"),
            "{means}"
        );
        assert!(means.contains("ADR-029 §9.2"), "{means}");
    }
}

/// **Two records for one key stay two records.** `TrustSet::trust` folds a
/// duplicate, which is right for a verifier and wrong for a report: a store
/// carrying one key twice is a fact about that file, and answering *one key*
/// about it would be `AGENTS.md` §5's guess that succeeds.
#[test]
fn a_store_carrying_one_key_twice_reports_two_records() {
    let value = answer(store(vec![
        key(0x11, "rivet-system", &["update"]),
        key(0x11, "rivet-system", &["update"]),
    ]));
    assert_eq!(field(&value, "keys"), Some(&Value::Integer(2)));
    let Some(Value::List(entries)) = field(&value, "trusted") else {
        panic!("no trusted list");
    };
    assert_eq!(entries.len(), 2);
}

/// A second word is refused rather than ignored — `super::storage`'s rule, and
/// the refusal names what the caller probably meant instead.
#[test]
fn a_second_word_is_refused_and_says_what_reads_an_artifact_instead() {
    let error = perform(
        &[SUBJECT, "photo-index.rvt"],
        &mut Fake::reading(store(vec![])),
    )
    .expect_err("a second word was accepted");
    assert_eq!(error.code(), ErrorCode::Arguments);
    assert!(error.message().contains("cargo rivet inspect"), "{}", error.message());
    assert!(error.message().contains("verifies nothing"), "{}", error.message());
}

/// The declared outputs and the result must agree — the same property every
/// other subject's `debug_assert!` checks, asserted here so a release build is
/// covered too.
#[test]
fn the_result_carries_only_fields_this_subject_declares_in_the_order_it_declares_them() {
    let value = answer(store(vec![key(0x11, "rivet-system", &["update"])]));
    let Value::Object(pairs) = &value else {
        panic!("not an object");
    };
    let fields: Vec<(&'static str, Value)> = OUTPUTS
        .iter()
        .filter_map(|f| {
            pairs
                .iter()
                .find(|(k, _)| *k == f.name)
                .map(|(_, v)| (f.name, v.clone()))
        })
        .collect();
    assert_eq!(
        fields.len(),
        pairs.len(),
        "the result carries a key OUTPUTS does not declare: {pairs:?}"
    );
    assert!(crate::operation::get::in_declared_order(
        &fields,
        OUTPUTS.iter().map(|f| f.name)
    ));
}
