//! **The Armada model's enrollment client** — `apply enroll-member`, sent over
//! the SAME pinned control plane [`crate::secrets`] already speaks.
//!
//! `ADR-059`'s Armada model needs a member's register record on the fleet
//! server's own register before that server will treat it as one of its own.
//! This is the host-side half of that: it renders a [`rivetos_register::Device`]
//! to the exact bytes [`rivetos_register::read_device`] parses back, hex-encodes
//! them the same way [`crate::secrets`] hex-encodes a credential, and sends
//! them as one `apply enroll-member <id> --hex <hex> confirm` — never a browser
//! certificate, never `:4443`, never `ureq`. Sockets are opened by
//! `crate::run::execute` and not by this module, so enrollment inherits the
//! control plane's pinning rather than carrying a second copy of it.
//!
//! # The wire contract is frozen, and this module does not choose it
//!
//! ```text
//! apply enroll-member <id> --hex <hex> confirm
//! ```
//!
//! `<id>` is the member's register identifier — `ADR-025` §4's sixteen
//! lowercase-hex characters, [`rivetos_register::Device::id`]. `<hex>` is the
//! member's own register-record TOML, hex-encoded with
//! [`rivet_core::package::key_material::to_hex`] — the SAME encoder
//! [`crate::secrets`] uses for `install-secret`, for `ADR-020` §4's reason
//! that lives there: one encoder, so a truncated credential and a truncated
//! record fail the same way rather than two.
//!
//! # What this module does not do
//!
//! **It does not resolve an endpoint.** The caller passes a resolved
//! [`rivet_remote::endpoint::Endpoint`] — `--host`/`--identity`/`--pin`,
//! already read and paired — exactly as `crate::secrets::run` receives one.
//! Inventing a second resolution here would be a second authority for how a
//! machine is reached (`ADR-011`).
//!
//! **It does not mint an identifier or a pin.** Both are the caller's, read
//! off whatever produced the [`rivetos_register::Device`] this sends —
//! provisioning a member, or an operator hand-composing one. This module only
//! carries the record to the fleet server's register.

use rivet_remote::endpoint::Endpoint;
use rivetos_register::Device;

/// The verb this module sends. Part of the wire contract; never renamed
/// without renaming the machine-side operation in the same change.
const SUBJECT: &str = "enroll-member";

/// Enroll `device` into the fleet server reachable at `endpoint`.
///
/// Renders `device` to its register-record TOML, hex-encodes those bytes, and
/// sends `apply enroll-member <id> --hex <hex> confirm` over `endpoint`'s
/// pinned control plane — `crate::secrets::run`'s shape exactly, for one file
/// instead of three.
///
/// # Errors
///
/// `endpoint.host` is `None` (there is nowhere to send this — the same
/// refusal `crate::secrets::run` makes for the same reason); the device does
/// not render to TOML (only possible if a field stops being representable);
/// the machine's own refusal, unaltered; or an answer this end cannot read as
/// being about the device it just sent.
pub(crate) fn enroll_member(endpoint: &Endpoint, device: &Device) -> Result<String, String> {
    if endpoint.host.is_none() {
        return Err(
            "--host is required to enroll a member: without it this end has nowhere to send \
             the record and nothing has been sent."
                .to_owned(),
        );
    }

    let text = render_device(device)?;
    let hex = rivet_core::package::key_material::to_hex(text.as_bytes());
    let words = enroll_words(endpoint, &device.id, &hex);

    let answer = crate::run::execute(&words).map_err(|e| {
        format!(
            "{}: {e}\n\nNothing was enrolled. Run the same command again once the cause is \
             fixed — enrolling replaces what is there for this id.",
            device.id
        )
    })?;
    read_answer(&answer, &device.id, text.len())
}

/// **The word vector, built and never handed to a shell.**
///
/// The same shape [`crate::secrets::run`] builds for `install-secret`: the
/// endpoint's three flags, in the order `--host`, `--identity`, `--pin`, each
/// only when the endpoint carries it — [`Endpoint::open`] is what actually
/// requires the pair, and this function does not repeat that check — then `--`
/// and the command itself.
fn enroll_words(endpoint: &Endpoint, id: &str, hex: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    if let Some(host) = &endpoint.host {
        words.push("--host".to_owned());
        words.push(host.clone());
    }
    if let Some(identity) = &endpoint.identity {
        words.push("--identity".to_owned());
        words.push(identity.display().to_string());
    }
    if let Some(pin) = &endpoint.pin {
        words.push("--pin".to_owned());
        words.push(pin.clone());
    }
    words.extend(["--", "apply", SUBJECT, id, "--hex", hex, "confirm"].map(str::to_owned));
    words
}

/// `device`, as the bytes [`rivetos_register::read_device`] parses back.
///
/// **The same call `cmd/rivet-fleet`'s `get_device` already makes** —
/// `toml_edit::ser::to_string_pretty` on one [`Device`] — rather than a second
/// implementation of `rivetos_register::render`'s per-file serialisation,
/// which is private to that crate for exactly this reason: a second copy here
/// would eventually render one field differently than the register's own
/// writer does, and the two would only disagree the day someone read a record
/// this module sent.
fn render_device(device: &Device) -> Result<String, String> {
    toml_edit::ser::to_string_pretty(device).map_err(|e| {
        format!(
            "{}: this record will not render to TOML ({e}). That can only happen if a field \
             on the device stopped being representable — nothing has been sent.",
            device.id
        )
    })
}

/// One line out of the machine's answer, and the same two checks
/// [`crate::secrets`]' own `read_answer` makes and for the same reasons. (A
/// link to that function itself cannot resolve from here: it is private to
/// its module, and it takes a type that is too.)
///
/// **The answer is checked to be about the device this asked about.** One
/// connection carries several calls, and an answer bound to the wrong one
/// would report a success about a device nobody named.
///
/// **The byte count is the check worth having.** It is what the machine
/// DECODED, so comparing it against what was sent proves the hex survived the
/// trip — a truncated record enrolls cleanly and the fleet server refuses to
/// reach it later with a message about parsing, which is the wrong end to
/// find that out.
fn read_answer(answer: &str, id: &str, sent_bytes: usize) -> Result<String, String> {
    use rivet_core::json::Json;

    let json = rivet_core::json::parse(answer).map_err(|why| {
        format!(
            "{id}: the machine's answer is not JSON this build can read ({why}). It said:\n\
             {answer}"
        )
    })?;
    let string = |key: &str| json.get(key).and_then(Json::as_str).map(str::to_owned);
    let integer = |key: &str| json.get(key).and_then(Json::as_integer);

    match string("id").as_deref() {
        Some(named) if named == id => {}
        Some(named) => {
            return Err(format!(
                "this asked about `{id}` and the machine answered about `{named}`. Nothing is \
                 claimed about either."
            ));
        }
        None => {
            return Err(format!(
                "{id}: the machine's answer names no id, so this build cannot tell what it is \
                 about. It said:\n{answer}"
            ));
        }
    }

    let wrote = integer("bytes").ok_or_else(|| {
        format!(
            "{id}: the machine's answer carries no byte count, so nothing here can say the \
             record arrived whole. It said:\n{answer}"
        )
    })?;
    let sent = i64::try_from(sent_bytes).unwrap_or(i64::MAX);
    if wrote != sent {
        return Err(format!(
            "{id}: {sent} bytes were sent and the machine decoded {wrote}. The record on that \
             fleet server is NOT the record on this end; do not treat this device as enrolled \
             until this is understood."
        ));
    }

    Ok(format!("{id:<20} {wrote:>5} bytes enrolled into the register\n"))
}

#[cfg(test)]
#[path = "enroll_tests.rs"]
mod enroll_tests;
