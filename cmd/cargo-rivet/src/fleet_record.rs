//! `cargo rivet fleet record [<host>] [--host …] [--identity …] [--pin …]
//! [--at DIR] [confirm]` — `ADR-052` §7, R-233: write into a host's record
//! which system update its running slot holds, once that slot has committed.
//!
//! # What it reads, and why that is enough
//!
//! The machine retains the signed header of whatever its installer wrote into
//! each slot (`rivet_core::deploy::SLOT_HEADERS`), and two artifacts with one
//! header are one artifact. So the build a slot holds is found by comparing
//! that header, as bytes, with the head of each system update this repository
//! holds. Nothing is parsed and nothing is taken from memory of an earlier
//! install.
//!
//! # It records nothing that has not committed
//!
//! `get slot` must answer a running slot and `committed: true`, or nothing is
//! written. Only the RUNNING slot's entry is written: the inactive slot may
//! hold a build that was installed and never trialled, and no reading
//! distinguishes that from the build it rolls back to. The other slot keeps
//! whatever the record already said, which is the build that slot held when
//! it last committed and was recorded. A host recorded at each commit
//! therefore names both builds a machine can boot.
//!
//! # It never invents a record
//!
//! The host is a device the register already holds: named, or found by the
//! pin `--pin` gives. A machine the register does not hold is refused, and
//! nothing is asked of it.

use std::path::Path;

use rivet_core::json::Json;
use rivet_remote::endpoint::Endpoint;
use rivet_remote::tls::CallsTools;
use rivetos_register::Register;

use super::{Entry, OBJECTS_DIR, read_manifest};

/// The kind a system update is published as.
const UPDATE_KIND: &str = "update";

/// Which device in `register` a record is written to: the one `device`
/// names, or the one whose pin is `given`'s `--pin`.
///
/// # Errors
///
/// The name resolves to nothing or to several; no device name and no pin; or
/// no device, or more than one, carries that pin.
pub(super) fn which_record(register: &Register, device: Option<&str>, given: &Endpoint) -> Result<String, String> {
    if let Some(name) = device {
        return register.resolve(name).map(|d| d.id.clone()).map_err(|e| {
            format!("{e}. A host's record is the register's; this never invents one. Nothing was recorded.")
        });
    }
    let Some(pin) = given.pin.as_deref() else {
        return Err("`fleet record` needs a host the register holds: name it, or give --pin so its \
                    record can be found by the machine's own key. Nothing was recorded."
            .to_owned());
    };
    let holders: Vec<&rivetos_register::Device> = register
        .devices
        .values()
        .filter(|d| d.pin.as_deref().is_some_and(|p| p.eq_ignore_ascii_case(pin)))
        .collect();
    match holders.as_slice() {
        [one] => Ok(one.id.clone()),
        [] => Err(format!(
            "no record in the register at {} carries the pin {pin}, so there is no host's record to \
             write into. A record is the register's (`rivet-fleet add device`, or provisioning) and \
             this never invents one (R-233). Nothing was recorded.",
            register.root.display()
        )),
        several => {
            let mut names: Vec<&str> = several.iter().map(|d| d.name.as_str()).collect();
            names.sort_unstable();
            Err(format!(
                "{} records carry the pin {pin} ({}); this will not choose between them. Name the \
                 host instead. Nothing was recorded.",
                several.len(),
                names.join(", ")
            ))
        }
    }
}

/// The running slot, read from `get slot`, when it has committed.
///
/// # Errors
///
/// The machine did not answer, named no running slot, or has not committed
/// it — each a refusal that writes nothing.
fn committed_slot(board: &dyn CallsTools) -> Result<String, String> {
    let answer = rivet_remote::command::perform(board, &["get", "slot"])?;
    let json = rivet_core::json::parse(&answer)
        .map_err(|why| format!("the machine's answer to `get slot` is not JSON this build reads ({why})"))?;
    let running = json.get("running").and_then(Json::as_str).filter(|s| *s == "a" || *s == "b").ok_or_else(|| {
        format!(
            "NOTHING WAS RECORDED: `get slot` names no running slot, so which build is running \
             cannot be said. The machine's own words: {}",
            json.get("means").and_then(Json::as_str).unwrap_or("(none)")
        )
    })?;
    match json.get("committed").and_then(Json::as_bool) {
        Some(true) => Ok(running.to_owned()),
        Some(false) => Err(format!(
            "NOTHING WAS RECORDED: slot {running} is running and NOT committed — a trial, which \
             rolls back at the next ordinary restart. A host's record names only what has \
             committed (R-233). Run this again once `get slot` reports `committed: true`."
        )),
        None => Err(format!(
            "NOTHING WAS RECORDED: `get slot` could not say whether slot {running} has committed, \
             and a build that may be a trial is not recorded (R-233)."
        )),
    }
}

/// The header the machine retains for `slot`'s build: exactly
/// `rivet_core::package::format::HEADER_BYTES` of it.
///
/// # Errors
///
/// No retained header (a slot no installer has written, such as a freshly
/// imaged card's), one of the wrong length, or an answer this cannot read.
fn retained_header(board: &dyn CallsTools, slot: &str) -> Result<Vec<u8>, String> {
    let path = format!(
        "{}/{slot}/{}",
        rivet_core::deploy::SLOT_HEADERS,
        rivet_core::package::install::HEADER_NAME
    );
    let answer = rivet_remote::command::perform(board, &["get", "file", &path])?;
    let json = rivet_core::json::parse(&answer)
        .map_err(|why| format!("the machine's answer about {path} is not JSON this build reads ({why})"))?;
    if json.get("path").and_then(Json::as_str) != Some(path.as_str()) {
        return Err(format!("this asked about {path} and the machine answered about something else"));
    }
    if json.get("present").and_then(Json::as_bool) != Some(true) {
        return Err(format!(
            "NOTHING WAS RECORDED: the machine retains no header for slot {slot} ({path} is not \
             there). The installer writes one for every build it puts in a slot, so this slot was \
             written some other way — a card image — and which update it holds cannot be read."
        ));
    }
    let content = json.get("content").and_then(Json::as_str).ok_or_else(|| {
        format!("the machine would not give the contents of {path}")
    })?;
    let bytes = match json.get("content_encoding").and_then(Json::as_str) {
        Some("base64") => rivet_core::base64::decode(content)
            .ok_or_else(|| format!("{path}: the machine said base64 and it does not decode"))?,
        Some("text") => content.as_bytes().to_vec(),
        other => return Err(format!("{path}: content_encoding {other:?} is not one this build reads")),
    };
    if bytes.len() != rivet_core::package::format::HEADER_BYTES {
        return Err(format!(
            "NOTHING WAS RECORDED: {path} holds {} bytes and a header is {}",
            bytes.len(),
            rivet_core::package::format::HEADER_BYTES
        ));
    }
    Ok(bytes)
}

/// The system update in `root` whose first bytes are `header`.
///
/// # Errors
///
/// The manifest does not read, or no system update this repository holds has
/// that header.
fn update_with_header(root: &Path, header: &[u8], slot: &str) -> Result<Entry, String> {
    let entries = read_manifest(root)?;
    for entry in entries.iter().filter(|e| e.kind == UPDATE_KIND) {
        let object = root.join(OBJECTS_DIR).join(&entry.digest);
        let Ok(mut file) = std::fs::File::open(&object) else {
            continue;
        };
        let mut head = vec![0u8; header.len()];
        if std::io::Read::read_exact(&mut file, &mut head).is_ok() && head == header {
            return Ok(entry.clone());
        }
    }
    Err(format!(
        "NOTHING WAS RECORDED: slot {slot}'s build is not a system update the fleet repository at \
         {} holds — none of its updates has the header the machine retains. Publish that update \
         here (`cargo rivet fleet publish FILE.rvt --trusted-key PATH --at {}`) and run this again.",
        root.display(),
        root.display()
    ))
}

/// **Record what `board`'s running, committed slot holds** in the register
/// device `id`, then project the register into `root`.
///
/// Without `confirm` it reads the machine and says what it would record, and
/// writes nothing.
///
/// # Errors
///
/// Every refusal above, and a register that cannot be saved or projected —
/// the projection is what a publish's prune reads, so one that did not land
/// is a failure rather than a note.
pub(super) fn record(
    root: &Path,
    register: &mut Register,
    id: &str,
    board: &dyn CallsTools,
    confirm: bool,
) -> Result<String, String> {
    let name = register
        .devices
        .get(id)
        .map(|d| d.name.clone())
        .ok_or_else(|| format!("internal error: the register holds no device {id}"))?;
    let slot = committed_slot(board)?;
    let header = retained_header(board, &slot)?;
    let update = update_with_header(root, &header, &slot)?;
    let what = format!("{} {} ({})", update.product, update.version, update.digest);

    let device = register
        .devices
        .get_mut(id)
        .ok_or_else(|| format!("internal error: the register holds no device {id}"))?;
    let other = if slot == "a" { "b" } else { "a" };
    let other_line = match device.system_slots.get(other) {
        Some(digest) => format!("slot {other} stays recorded as {digest}"),
        None => format!(
            "slot {other} has nothing recorded, and only a running, committed slot is ever \
             recorded: it is named once it has run and committed"
        ),
    };
    if device.system_slots.get(&slot) == Some(update.digest.as_str()) {
        return Ok(format!(
            "unchanged  {name}'s record already names slot {slot} as {what}; {other_line}.\n"
        ));
    }
    if !confirm {
        return Ok(format!(
            "would record  {name} slot {slot} (running, committed) = {what}; {other_line}.\n\
             Nothing was written — pass `confirm` as the last word to record it.\n"
        ));
    }

    device.system_slots.set(&slot, &update.digest)?;
    rivetos_register::store::save(register)
        .map_err(|e| format!("the register at {} could not be saved: {e}. Nothing was recorded.", register.root.display()))?;
    let projected = rivetos_fleet_repo::record_register(root, register);
    if let rivetos_fleet_repo::Outcome::Refused(why) = &projected {
        return Err(format!(
            "the register records {name} slot {slot} = {what}, and the fleet repository at {} \
             did NOT take the projection: {why}. A publish's prune reads the repository's copy, so \
             until it lands that update is not protected. Run this again once the cause is fixed.",
            root.display()
        ));
    }
    Ok(format!(
        "recorded  {name} slot {slot} (running, committed) = {what}; {other_line}.\n\
         {projected}\n\
         A publish into this repository never prunes a digest a host's record names (ADR-052 §7).\n"
    ))
}

/// The composition root: load the register, find the host's record, open the
/// pinned transport, and [`record`].
///
/// # Errors
///
/// The register does not read, [`which_record`]'s, the endpoint's, or
/// [`record`]'s.
pub(super) fn run(root: &Path, device: Option<&str>, given: &Endpoint, confirm: bool) -> Result<String, String> {
    given.paired()?;
    let register_root = crate::device::default_register_dir()?;
    let mut register = rivetos_register::store::load(&register_root).map_err(|e| {
        format!("could not read the register at {}: {e}. Nothing was recorded.", register_root.display())
    })?;
    let id = which_record(&register, device, given)?;
    let endpoint = super::push::resolve_endpoint(device, given, &|| Ok(register_root.clone()))
        .map_err(|e| e.replace("`fleet push`", "`fleet record`"))?;
    let transport = endpoint.open()?.ok_or_else(|| {
        "no operator identity to present: `fleet record` reads the machine over ADR-035's control \
         plane and has no second transport. --identity DIR names one."
            .to_owned()
    })?;
    record(root, &mut register, &id, &transport, confirm)
}

#[cfg(test)]
#[path = "fleet_record_tests.rs"]
mod tests;
