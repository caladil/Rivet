//! `cargo rivet fleet phones list | rename | revoke` — `ADR-061` §6, the
//! agent's surface over the phones a fleet server's `gantry` socket also
//! serves.
//!
//! # Where the format lives, and why it is here rather than on the board
//!
//! `rivet-core`'s `get phones` hands back each phone's record as raw bytes,
//! never parsed — `rivet_core::operation::phones`'s own documentation is the
//! whole argument: `rivet-core` cannot depend on `rivetos-register`
//! (`ADR-014` §3), so it holds no opinion about the register's TOML. This
//! crate already depends on `rivetos-register` (`fleet.rs`'s own register
//! reads), so the decoding happens here: `list` renders each record's label,
//! key and enrollment time for a person to read; `rename` decodes the CURRENT
//! record, changes only the label, and re-renders the WHOLE record — the
//! same shape `super::enroll_member` already renders a device to, hex-
//! encoded the same way, and sent the same way (`--hex`, `confirm` last).
//!
//! # No `<host>`, unlike `push`/`sync`
//!
//! A phone belongs to THE fleet's own server (`ADR-061` §5), so there is
//! nothing to name: `super::resolve_fleet_server` finds the one register
//! record with `role = "fleet-server"`, exactly as `fleet publish --sync`
//! finds it, and `--host`/`--identity`/`--pin` override what it resolves.

use rivet_remote::endpoint::Endpoint;

use crate::json::Json;

/// What `cargo rivet fleet phones` was asked to do.
#[derive(Debug)]
pub(crate) enum PhonesVerb {
    List,
    /// `rename <identity> <label> [confirm]`.
    Rename {
        identity: String,
        label: String,
        confirm: bool,
    },
    /// `revoke <identity> [confirm]`.
    Revoke { identity: String, confirm: bool },
}

#[derive(Debug)]
pub(crate) struct PhonesArgs {
    verb: PhonesVerb,
    endpoint: Endpoint,
}

fn usage() -> String {
    "usage: cargo rivet fleet phones list   [--host HOST:PORT] [--identity DIR] [--pin HEX]\n\
     \x20      cargo rivet fleet phones rename <identity> <label>\n\
     \x20                                [--host HOST:PORT] [--identity DIR] [--pin HEX] [confirm]\n\
     \x20      cargo rivet fleet phones revoke <identity>\n\
     \x20                                [--host HOST:PORT] [--identity DIR] [--pin HEX] [confirm]\n\
     \n\
     ADR-061 §6 — the same phones a fleet server's `gantry` socket serves an\n\
     Android app. The fleet server is found, never named: the one register\n\
     record with role = \"fleet-server\". `rename`/`revoke` WITHOUT `confirm`\n\
     print the machine's plan and change nothing.\n"
        .to_owned()
}

/// `args[0]` is `\"phones\"`; `args[1]` is `list`/`rename`/`revoke`.
///
/// # Errors
///
/// No sub-verb, an unrecognised one, a wrong count of positional arguments,
/// an unrecognised flag, or `--identity`/`--pin` given without the other.
pub(crate) fn parse(args: &[String]) -> Result<PhonesArgs, String> {
    let Some(sub) = args.get(1).map(String::as_str) else {
        return Err(format!(
            "`fleet phones` needs a verb: list, rename or revoke.\n\n{}",
            usage()
        ));
    };

    let mut endpoint = Endpoint::default();
    let mut confirm = false;
    let mut positionals: Vec<String> = Vec::new();
    let mut index = 2;
    while index < args.len() {
        if let Some(next) = endpoint.read(args, index)? {
            index = next;
            continue;
        }
        match args[index].as_str() {
            "-h" | "--help" => return Err(usage()),
            "confirm" => {
                confirm = true;
                index += 1;
            }
            other if other.starts_with("--") => {
                return Err(format!(
                    "{other:?} is not an option of `fleet phones {sub}`\n\n{}",
                    usage()
                ));
            }
            other => {
                positionals.push(other.to_owned());
                index += 1;
            }
        }
    }
    endpoint.paired()?;

    let verb = match sub {
        "list" => {
            if let Some(extra) = positionals.first() {
                return Err(format!(
                    "`fleet phones list` takes no positional argument and {extra:?} is one.\n\n{}",
                    usage()
                ));
            }
            if confirm {
                return Err(
                    "`fleet phones list` reads and changes nothing, so it takes no `confirm`."
                        .to_owned(),
                );
            }
            PhonesVerb::List
        }
        "rename" => {
            let mut words = positionals.into_iter();
            let identity = words.next().ok_or_else(|| {
                format!(
                    "`fleet phones rename` needs an identity and a label.\n\n{}",
                    usage()
                )
            })?;
            let label = words.next().ok_or_else(|| {
                format!("`fleet phones rename {identity}` needs a label.\n\n{}", usage())
            })?;
            if let Some(extra) = words.next() {
                return Err(format!(
                    "`fleet phones rename` takes an identity and a label, and {extra:?} is a \
                     third word.\n\n{}",
                    usage()
                ));
            }
            PhonesVerb::Rename { identity, label, confirm }
        }
        "revoke" => {
            let mut words = positionals.into_iter();
            let identity = words
                .next()
                .ok_or_else(|| format!("`fleet phones revoke` needs an identity.\n\n{}", usage()))?;
            if let Some(extra) = words.next() {
                return Err(format!(
                    "`fleet phones revoke` takes one identity, and {extra:?} is a second word.\n\n{}",
                    usage()
                ));
            }
            PhonesVerb::Revoke { identity, confirm }
        }
        "-h" | "--help" => return Err(usage()),
        other => {
            return Err(format!(
                "`fleet phones {other}` is not a verb. `fleet phones` has list, rename and \
                 revoke.\n\n{}",
                usage()
            ));
        }
    };
    Ok(PhonesArgs { verb, endpoint })
}

/// The fleet's own server, resolved from the workstation's register —
/// `super::resolve_fleet_server`'s rule, applied here the way
/// `super::sync_to_fleet_server` applies it. `given`'s `--host`/`--identity`/
/// `--pin` win over what the register resolves.
///
/// # Errors
///
/// `given.paired()`'s (half of `--identity`/`--pin` given); the register
/// cannot be read; `super::resolve_fleet_server`'s (none, or more than
/// one, record with `role = "fleet-server"`); or `super::push::resolve_endpoint`'s.
fn resolve(given: &Endpoint) -> Result<Endpoint, String> {
    given.paired()?;
    let register_root = crate::device::default_register_dir()?;
    let register = rivetos_register::store::load(&register_root).map_err(|e| {
        format!(
            "could not read the register at {}: {e}. Nothing was asked.",
            register_root.display()
        )
    })?;
    let device = super::resolve_fleet_server(&register).map_err(|e| e.replace("--sync", "`fleet phones`"))?;
    let name = device.name.clone();
    super::push::resolve_endpoint(Some(&name), given, &|| Ok(register_root.clone()))
        .map_err(|e| e.replace("`fleet push`", "`fleet phones`"))
}

/// # Errors
///
/// [`resolve`]'s, no transport (missing identity/pin), or the machine's own
/// refusal — including the plan a `rename`/`revoke` prints without `confirm`.
pub(crate) fn run(args: &PhonesArgs) -> Result<String, String> {
    let endpoint = resolve(&args.endpoint)?;
    let transport = endpoint.open()?.ok_or_else(|| {
        format!(
            "--identity and --pin are required: `fleet phones` speaks ADR-035's control plane \
             and has no second transport.\n\n{}",
            usage()
        )
    })?;
    match &args.verb {
        PhonesVerb::List => list(&transport),
        PhonesVerb::Rename { identity, label, confirm } => {
            rename(&transport, identity, label, *confirm)
        }
        PhonesVerb::Revoke { identity, confirm } => revoke(&transport, identity, *confirm),
    }
}

/// Hexadecimal to the bytes it spells. This crate's own small decoder —
/// `rivet_core::operation::install_secret::decode` is `rivet-core`'s and not
/// visible here (`ADR-014` §3 keeps that crate's internals to itself).
///
/// # Errors
///
/// An odd number of characters, or one that is not `0-9a-fA-F`.
fn decode_hex(hex: &str) -> Result<Vec<u8>, String> {
    let hex = hex.trim();
    if !hex.len().is_multiple_of(2) {
        return Err(format!("{} is an odd number of hex characters", hex.len()));
    }
    (0..hex.len())
        .step_by(2)
        .map(|at| {
            u8::from_str_radix(&hex[at..at + 2], 16)
                .map_err(|_| format!("{:?} is not two hexadecimal characters", &hex[at..at + 2]))
        })
        .collect()
}

/// One phone, decoded from `get phones`'s answer for it.
fn decode_phone(identity: &str, record_hex: &str) -> Result<rivetos_register::phones::Phone, String> {
    let bytes = decode_hex(record_hex)
        .map_err(|e| format!("phone {identity}'s record is not valid hexadecimal: {e}"))?;
    let text = String::from_utf8(bytes)
        .map_err(|e| format!("phone {identity}'s record is not UTF-8 text: {e}"))?;
    let path = std::path::PathBuf::from(format!("{identity}.toml"));
    rivetos_register::phones::read_record(&path, &text)
        .map_err(|e| format!("phone {identity}'s record could not be read: {e}"))
}

/// Every phone `get phones` answered with, decoded.
fn decoded_phones(answer: &str) -> Result<Vec<rivetos_register::phones::Phone>, String> {
    let value = crate::json::parse(answer)
        .map_err(|e| format!("the machine's answer to `get phones` is not valid JSON: {e}"))?;
    let phones = value.get("phones").and_then(Json::as_array).ok_or_else(|| {
        format!("the machine's answer to `get phones` has no `phones` array: {answer}")
    })?;
    phones
        .iter()
        .map(|phone| {
            let identity = phone.get("identity").and_then(Json::as_str).ok_or_else(|| {
                format!("an entry in `get phones`'s answer has no `identity`: {answer}")
            })?;
            let record = phone.get("record").and_then(Json::as_str).ok_or_else(|| {
                format!("phone {identity} in `get phones`'s answer has no `record`: {answer}")
            })?;
            decode_phone(identity, record)
        })
        .collect()
}

fn list(transport: &dyn rivet_remote::tls::CallsTools) -> Result<String, String> {
    let call = rivet_remote::command::translate(&["get", "phones"])?;
    let answer = transport.call_tool(&call.tool, &call.arguments)?;
    let phones = decoded_phones(&answer)?;
    if phones.is_empty() {
        return Ok("no phones are enrolled.\n".to_owned());
    }
    let mut lines: Vec<String> = phones
        .iter()
        .map(|phone| {
            format!(
                "{}  {:?}  enrolled {}  key {}",
                phone.identity,
                phone.label,
                phone.enrolled,
                rivet_core::base64::encode(&phone.public_key),
            )
        })
        .collect();
    lines.push(String::new());
    Ok(lines.join("\n"))
}

/// **Never sent unconfirmed.** `apply`'s `change` input is `several` and
/// `confirmation` is the one single-word input left after it, so
/// `rivet_remote::command::translate`'s binder — `ADR-011`'s one binder,
/// documented in that crate rather than duplicated here — reserves the LAST
/// word of what is left for `confirmation` no matter what it is: give it no
/// trailing word at all and the reservation eats the tail of the hex instead
/// (`--hex`'s VALUE, not the flag), corrupting the very payload this sends.
/// `enroll-member`'s own workstation half (`super::enroll_member`'s
/// `enroll_words`) takes the same way out for the same shape: send `confirm`
/// unconditionally, and preview LOCALLY from the record this already had to
/// read, instead of asking the board for a plan it cannot safely request.
fn rename(
    transport: &dyn rivet_remote::tls::CallsTools,
    identity: &str,
    label: &str,
    confirm: bool,
) -> Result<String, String> {
    let list_call = rivet_remote::command::translate(&["get", "phones"])?;
    let list_answer = transport.call_tool(&list_call.tool, &list_call.arguments)?;
    let phones = decoded_phones(&list_answer)?;
    let current = phones.iter().find(|p| p.identity == identity).ok_or_else(|| {
        format!(
            "no phone is enrolled under the identity {identity:?}. `cargo rivet fleet phones \
             list` shows what is enrolled."
        )
    })?;

    if !confirm {
        return Ok(format!(
            "would rewrite {identity}'s label from {:?} to {label:?}. Nothing was sent — pass \
             `confirm` as the last word to do it.\n",
            current.label,
        ));
    }

    let renamed = rivetos_register::phones::Phone::new(
        &current.identity,
        label,
        current.public_key.clone(),
        &current.enrolled,
    )
    .map_err(|e| e.to_string())?;
    let path = std::path::PathBuf::from(format!("{identity}.toml"));
    let text = rivetos_register::phones::render(&renamed, &path).map_err(|e| e.to_string())?;
    let hex = rivet_core::package::key_material::to_hex(text.as_bytes());

    let words: Vec<&str> = vec!["apply", "rename-phone", identity, "--hex", &hex, "confirm"];
    let call = rivet_remote::command::translate(&words)?;
    transport.call_tool(&call.tool, &call.arguments)
}

fn revoke(
    transport: &dyn rivet_remote::tls::CallsTools,
    identity: &str,
    confirm: bool,
) -> Result<String, String> {
    let mut words: Vec<&str> = vec!["apply", "revoke-phone", identity];
    if confirm {
        words.push("confirm");
    }
    let call = rivet_remote::command::translate(&words)?;
    transport.call_tool(&call.tool, &call.arguments)
}

#[cfg(test)]
#[path = "fleet_phones_tests.rs"]
mod tests;
