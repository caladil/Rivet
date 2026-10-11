//! `cargo rivet fleet root <host> [--out DIR]` — **fetch a fleet-root mode
//! fleet's root over the pinned control plane and save it for a device's trust
//! store.** `ADR-063` §4.
//!
//! The machine answers `rivet get fleet-root`: the root's certificate as PEM
//! and the SHA-256 of its DER. The pin this workstation reached the machine
//! with is what vouches for the certificate; the fingerprint is recomputed
//! here and a disagreement refuses before anything is written, so what is
//! saved is exactly what the machine holds.
//!
//! Two files are written, one certificate in the two encodings devices ask
//! for: `<fleet>-root.crt` (PEM — Android's *Install a certificate → CA
//! certificate*, and `certutil`) and `<fleet>-root.cer` (DER — what Windows
//! opens on a double-click). The fleet's name is the root's common name.

use std::path::PathBuf;

use rivet_remote::endpoint::Endpoint;

use crate::json::Json;

#[derive(Debug)]
pub(crate) struct RootArgs {
    host: String,
    out: PathBuf,
    endpoint: Endpoint,
}

fn usage() -> String {
    "usage: cargo rivet fleet root <host> [--out DIR] [--host HOST:PORT] [--identity DIR] [--pin HEX]\n\
     \n\
     ADR-063 §4 — fetch the root a fleet-root mode fleet server minted, over the\n\
     pinned control plane, and save it as <fleet>-root.crt (PEM) and <fleet>-root.cer\n\
     (DER) in DIR (default: the current directory). Prints the SHA-256 fingerprint a\n\
     trust store shows beside it. Reads the machine and changes nothing on it.\n\
     \n\
     <host> is looked up in the register, any flag given winning over what it holds.\n\
     With all three of --host, --identity and --pin the register is not read, so a\n\
     machine no register holds can be asked; <host> then only names the output.\n"
        .to_owned()
}

/// `args[0]` is `"root"`.
///
/// # Errors
///
/// No host, a second one, an unknown flag, or half of `--identity`/`--pin`.
pub(crate) fn parse(args: &[String]) -> Result<RootArgs, String> {
    let mut endpoint = Endpoint::default();
    let mut out = None;
    let mut host = None;
    let mut index = 1;
    while index < args.len() {
        if let Some(next) = endpoint.read(args, index)? {
            index = next;
            continue;
        }
        match args[index].as_str() {
            "-h" | "--help" => return Err(usage()),
            "--out" => {
                let dir = args.get(index + 1).ok_or_else(|| format!("--out needs a directory\n\n{}", usage()))?;
                out = Some(PathBuf::from(dir));
                index += 2;
            }
            other if other.starts_with("--") => {
                return Err(format!("{other:?} is not an option of `fleet root`\n\n{}", usage()));
            }
            other => {
                if host.is_some() {
                    return Err(format!("`fleet root` takes one machine, and {other:?} is a second\n\n{}", usage()));
                }
                host = Some(other.to_owned());
                index += 1;
            }
        }
    }
    endpoint.paired()?;
    let host = host.ok_or_else(|| format!("`fleet root` needs a machine\n\n{}", usage()))?;
    Ok(RootArgs { host, out: out.unwrap_or_else(|| PathBuf::from(".")), endpoint })
}

/// # Errors
///
/// The machine could not be reached, holds no root, or answered a
/// certificate whose fingerprint does not match; or a file could not be
/// written.
pub(crate) fn run(args: &RootArgs) -> Result<String, String> {
    let endpoint = endpoint_for(args, &crate::device::default_register_dir)?;
    let transport = endpoint.open()?.ok_or_else(|| {
        format!("--identity and --pin are required: `fleet root` speaks ADR-035's control plane.\n\n{}", usage())
    })?;
    use rivet_remote::tls::CallsTools as _;
    let call = rivet_remote::command::translate(&["get", "fleet-root"])?;
    let answer = transport.call_tool(&call.tool, &call.arguments)?;
    let fetched = read_answer(&answer)?;
    save(&fetched, &args.out, &args.host)
}

/// The machine to ask.
///
/// **With all three of `--host`, `--identity` and `--pin`, those are the
/// machine and the register is not read** — `cargo rivet run --host …`'s
/// rule — so a machine no register holds can still be asked; the positional
/// machine then only names the output files when the root carries no common
/// name. Otherwise the positional machine is looked up in the register, with
/// any flag given winning over what it holds.
///
/// `register_root` is consulted only in the second case — a test passes a
/// scratch one, and the real run passes `crate::device::default_register_dir`.
///
/// # Errors
///
/// The register's refusals, when it is consulted.
fn endpoint_for(
    args: &RootArgs,
    register_root: &dyn Fn() -> Result<PathBuf, String>,
) -> Result<Endpoint, String> {
    let given = &args.endpoint;
    if given.host.is_some() && given.identity.is_some() && given.pin.is_some() {
        return Ok(given.clone());
    }
    super::push::resolve_endpoint(Some(&args.host), given, register_root)
        .map_err(|e| e.replace("`fleet push`", "`fleet root`"))
}

/// A root as the machine answered it, checked.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Fetched {
    pub pem: String,
    pub der: Vec<u8>,
    pub sha256: String,
}

/// Read `get fleet-root`'s answer, and recompute its fingerprint.
///
/// # Errors
///
/// Not JSON; `present` false (named as an answer, not a fault); no
/// certificate; a certificate that is not one PEM block; or a fingerprint the
/// machine answered that is not the SHA-256 of the certificate it answered.
pub(crate) fn read_answer(answer: &str) -> Result<Fetched, String> {
    let value = crate::json::parse(answer)
        .map_err(|e| format!("the machine's answer to `get fleet-root` is not JSON: {e}"))?;
    if value.get("present").and_then(Json::as_bool) != Some(true) {
        return Err(format!(
            "NOTHING WAS SAVED. The machine holds no fleet root: it is not the server of a \
             fleet-root mode fleet, or its first boot has not minted one. It said: {answer}"
        ));
    }
    let pem = value
        .get("certificate")
        .and_then(Json::as_str)
        .ok_or_else(|| format!("the answer has no certificate: {answer}"))?
        .to_owned();
    let claimed = value.get("sha256").and_then(Json::as_str).unwrap_or_default().to_owned();
    let der = rivet_fleet_root::der_of_pem(&pem)
        .ok_or_else(|| "NOTHING WAS SAVED. The certificate the machine answered is not PEM".to_owned())?;
    let sha256 = rivet_fleet_root::fingerprint(&der);
    if sha256 != claimed {
        return Err(format!(
            "NOTHING WAS SAVED. The machine answered fingerprint {claimed:?} for a certificate \
             whose SHA-256 here is {sha256}. Something between the machine's store and this \
             answer changed it."
        ));
    }
    Ok(Fetched { pem, der, sha256 })
}

/// Write the two files and say how to install them.
///
/// # Errors
///
/// A file that cannot be written.
pub(crate) fn save(fetched: &Fetched, out: &std::path::Path, host: &str) -> Result<String, String> {
    let fleet = common_name(&fetched.der).unwrap_or_else(|| host.to_owned());
    let safe: String = fleet
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect();
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let pem_path = out.join(format!("{safe}-root.crt"));
    let der_path = out.join(format!("{safe}-root.cer"));
    std::fs::write(&pem_path, &fetched.pem).map_err(|e| format!("{}: {e}", pem_path.display()))?;
    std::fs::write(&der_path, &fetched.der).map_err(|e| format!("{}: {e}", der_path.display()))?;
    Ok(format!(
        "fleet {fleet:?}'s root, fetched from {host} over the pinned control plane\n\
         \x20 SHA-256  {}\n\
         \x20 saved    {}  (PEM)\n\
         \x20          {}  (DER)\n\
         \n\
         Compare the fingerprint with the one your device shows before trusting it.\n\
         \x20 Windows (this user):  certutil -user -addstore Root \"{}\"\n\
         \x20 Android:  Settings > Security > Encryption & credentials > Install a certificate\n\
         \x20           > CA certificate, and pick {}\n",
        colon_hex(&fetched.sha256),
        pem_path.display(),
        der_path.display(),
        der_path.display(),
        pem_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
    ))
}

/// `ab:cd:…`, upper-case, as Windows and Android print a fingerprint.
fn colon_hex(hex: &str) -> String {
    hex.as_bytes()
        .chunks(2)
        .map(|pair| String::from_utf8_lossy(pair).to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join(":")
}

/// The subject's common name, read with `rivetos-tls`'s reader.
fn common_name(der: &[u8]) -> Option<String> {
    rivetos_tls::subject_common_name(der)
}

#[cfg(test)]
#[path = "fleet_root_tests.rs"]
mod tests;
