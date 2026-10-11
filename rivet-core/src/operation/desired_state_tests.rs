//! What `get document` answers, and the two states it must not confuse.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::machine::MachineError;

/// A port that answers exactly what a test sets, and refuses everything else —
/// this subject reaches one method and a fake that answered the others would be
/// claiming things about a machine that is not there.
struct Fake(Result<Option<String>, MachineError>);

impl crate::operation::machine::Observed for Fake {
    /// **Refuses rather than answering that this machine trusts nobody.**
    /// An empty trust store is a REAL reading -- it is what `ADR-028` §5.5.5's
    /// `--no-trust` image produces -- and this fake has not taken it. The
    /// direction is the dangerous one: *"trusts nobody"* is the answer
    /// somebody acts on by re-signing an artifact.
    fn read_trust(&mut self) -> Result<crate::operation::machine::TrustRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering an idle machine.** Zeroed counters
    /// are what a machine that has run nothing since it booted looks like,
    /// and a load average of zero is what an idle one looks like — both are
    /// readings, and this fake has taken neither (`AGENTS.md` §3, §5).
    fn read_cpu(&mut self) -> Result<crate::operation::machine::CpuRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering that this machine has no block
    /// devices**, which is not a state a machine that booted off one can
    /// be in.
    fn read_devices(&mut self) -> Result<crate::operation::machine::DeviceRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering an empty network.** No interfaces at
    /// all reads as a machine with none, which is not a state a running
    /// Linux is in.
    fn read_network(&mut self) -> Result<crate::operation::machine::NetworkRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than naming a slot.** Naming the wrong one is how
    /// an update overwrites the running system, so a fake that observes
    /// nothing names none.
    fn read_slots(&mut self) -> Result<crate::operation::machine::SlotRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering an uptime of zero.** `0` would say
    /// this machine booted a moment ago, which is a reading nobody took.
    fn read_boot(&mut self) -> Result<crate::operation::machine::BootRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering with unset flags.** Reporting a
    /// `Throttle` of all-`false` would assert that this machine has never
    /// been under-volted since it booted — the one claim this subject must
    /// never make without a measurement (`AGENTS.md` §3, §5).
    fn read_thermal(
        &mut self,
    ) -> Result<crate::operation::machine::ThermalRead, MachineError> {
        Err(MachineError::Refused(
                "not this fake's port; unread throttle flags are never reported as false"
                    .to_string(),
        ))
    }
    fn read_processes(
        &mut self,
    ) -> Result<crate::operation::machine::ProcessRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_desired_state(&mut self) -> Result<Option<String>, MachineError> {
        match &self.0 {
            Ok(v) => Ok(v.clone()),
            Err(e) => Err(MachineError::Refused(e.why().to_string())),
        }
    }
    fn read_memory(
        &mut self,
    ) -> Result<crate::operation::machine::MemoryRead, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn read_journal(
        &mut self,
    ) -> Result<crate::operation::machine::JournalRead, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn read_filesystems(
        &mut self,
    ) -> Result<crate::operation::machine::StorageRead, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn configuration_schema(&mut self) -> Value {
        Value::str("<not this test's port>")
    }
    fn supervisor_status(
        &mut self,
    ) -> Result<crate::operation::status::Status, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
}


/// One field out of the result object.
fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    match value {
        Value::Object(fields) => fields
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v)
            .unwrap_or_else(|| panic!("no field {key:?} in {value:?}")),
        other => panic!("not an object: {other:?}"),
    }
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    match field(value, key) {
        Value::Str(s) => s,
        other => panic!("{key} is not a string: {other:?}"),
    }
}

fn flag(value: &Value, key: &str) -> bool {
    match field(value, key) {
        Value::Bool(b) => *b,
        other => panic!("{key} is not a bool: {other:?}"),
    }
}

fn number(value: &Value, key: &str) -> i64 {
    match field(value, key) {
        Value::Integer(n) => *n,
        other => panic!("{key} is not an integer: {other:?}"),
    }
}

fn get(fake: &mut Fake, arguments: &[&str]) -> Result<Value, OperationError> {
    perform(arguments, fake)
}

/// The ordinary answer, and the one `--enable` over TLS is waiting for.
#[test]
fn a_document_is_returned_verbatim() {
    let text_in = "[services.rivet-mdnsd]\nenabled = true\n";
    let mut fake = Fake(Ok(Some(text_in.to_string())));
    let value = get(&mut fake, &["desired-state"]).expect("a document");
    assert!(flag(&value, "present"));
    assert_eq!(text(&value, "contents"), text_in);
    assert_eq!(text(&value, "path"), MACHINE_DOCUMENT);
    assert_eq!(number(&value, "bytes"), text_in.len() as i64);
}

/// **Verbatim means verbatim.** A caller composing `apply edit --expect` is
/// matching against what this returned, so normalising whitespace or dropping
/// a comment here would make that edit refuse against text this verb showed it.
#[test]
fn comments_and_whitespace_survive() {
    let text_in = "# why this machine exists\n\n[services.x]   \nenabled  =  true\n";
    let mut fake = Fake(Ok(Some(text_in.to_string())));
    let value = get(&mut fake, &["desired-state"]).expect("a document");
    assert_eq!(text(&value, "contents"), text_in, "the document was not returned verbatim");
}

/// **No document is not an error**, and this is the distinction the subject
/// exists to keep: a first boot has none (`ADR-004` §6), and reporting an
/// ordinary state as a fault would send an operator looking for a broken card.
#[test]
fn no_document_is_an_ordinary_answer() {
    let mut fake = Fake(Ok(None));
    let value = get(&mut fake, &["desired-state"]).expect("absence is not an error");
    assert!(!flag(&value, "present"));
    assert_eq!(number(&value, "bytes"), 0);
    assert_eq!(text(&value, "contents"), "");
}

/// ...but a document that is THERE and unreadable is. The two must not collapse
/// into one answer: the first means "nothing to read", the second means
/// "something is wrong with this machine".
#[test]
fn an_unreadable_document_is_an_error_and_not_an_absence() {
    let mut fake = Fake(Err(MachineError::Refused("permission denied".to_string())));
    let error = get(&mut fake, &["desired-state"]).expect_err("must not report absence");
    assert_eq!(error.code(), ErrorCode::MachineRefused);
}

/// An empty document is present. `present` is what separates it from absence,
/// because this value model has no null and `contents` is `""` for both.
#[test]
fn an_empty_document_is_present() {
    let mut fake = Fake(Ok(Some(String::new())));
    let value = get(&mut fake, &["desired-state"]).expect("an empty file is a file");
    assert!(flag(&value, "present"), "an empty file is still a file");
    assert_eq!(number(&value, "bytes"), 0);
}

/// A second word is refused rather than ignored — `schema`'s precedent.
#[test]
fn a_second_word_is_refused() {
    let mut fake = Fake(Ok(Some("x".to_string())));
    let error = get(&mut fake, &["desired-state", "services"]).expect_err("refused");
    assert_eq!(error.code(), ErrorCode::Arguments);
}
