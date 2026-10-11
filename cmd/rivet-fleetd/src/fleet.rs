//! **The register, as this program sees it** — `ADR-034` §2's store, read from
//! disk and rendered for a browser.
//!
//! # Why a fleet server holds it at all
//!
//! `ADR-034` §2.2 offered two modes and the maintainer dropped Local on
//! 2026-09-01, because it no longer made sense (.rulings/ R-279). With Local
//! gone the register lives on a Rivet machine, and this is the program that
//! owns that directory.
//!
//! **§2.3 already ruled what that makes the workstation: a client.** *"In
//! Remote mode the workstation application is a client. It does not open the
//! register's files, and there is no 'sync' — a sync is two authorities plus a
//! reconciliation policy."* So `cmd/rivet-provision` will enroll a member by
//! asking this program, not by writing these files over a share.
//!
//! # What this module does NOT do, and it is the important part
//!
//! **It does not invent entries.** A fleet server that met an empty directory
//! and minted a record for itself would be manufacturing the one thing
//! `ADR-034` §3.5 says it cannot have: a device record is written by the tool
//! that GENERATED the device's keypair, and this program generated nobody's.
//!
//! An absent register is therefore an *empty* register, which
//! `rivetos_register::store::load` already returns, and which §3.3 makes
//! recoverable — *"a register can be rebuilt by asking the fleet."*
//!
//! # The bootstrap, stated so nobody builds the wrong thing
//!
//! The first record — the fleet server's own — is written by
//! `cmd/rivet-provision` onto the card, because at that moment there is no
//! server listening to be told. Every record after it arrives over the control
//! plane. **Enrollment is one operation with two writes and only the transport
//! differs by stage.**

use rivet_core::journal::write_json_string;
use rivetos_register::{Device, Register, store};

/// Where the register lives inside this application's data directory.
///
/// `RIVET_APP_DATA/register/`, holding `register.toml` and `devices/<id>.toml`
/// exactly as `rivetos_register` lays them out — the format did not change by
/// moving onto a machine, which is what `ADR-034` §10.2 ruled and what makes a
/// register written by a workstation readable here.
pub const REGISTER_DIR: &str = "register";

/// Read the register this machine holds.
///
/// # Errors
///
/// A sentence naming the path. **A missing directory is not one of them** — see
/// the module note.
pub fn load() -> Result<Register, String> {
    let dir = rivet_sdk::app::data().map_err(|e| e.to_string())?;
    let root = dir.join(REGISTER_DIR);
    store::load(&root).map_err(|e| format!("{}: {e}", root.display()))
}

/// Push `"name": <string>` or `"name": null`.
fn text(out: &mut String, name: &str, value: Option<&str>) {
    out.push('"');
    out.push_str(name);
    out.push_str("\":");
    match value {
        Some(v) => write_json_string(v, out),
        None => out.push_str("null"),
    }
}

/// One device, as the sidebar needs it.
///
/// **Public facts only, because that is all a register holds** (`ADR-034` §3).
/// The pin is a public key and §3.6 is why publishing it costs nothing — it is
/// what a client *checks*, not what it presents.
///
/// `address` is [`Device::dial_host`]'s answer, and its refusal is carried
/// through as a string rather than dropped: a device the register cannot place
/// must appear in the list saying so, or an operator sees a machine that is
/// simply missing and has nothing to act on.
///
/// **`dial_host`, not `address`.** A DHCP member's record carries no
/// interface address — that alone is not a reason to call it unreachable, it
/// is reachable at `<name>.local` once `rivet-mdnsd` answers for it.
/// `unreachable` is reserved for what `dial_host` still refuses: several
/// addresses on one record, or a name that cannot become a hostname.
fn device(out: &mut String, d: &Device) {
    out.push('{');
    text(out, "id", Some(&d.id));
    out.push(',');
    text(out, "name", Some(&d.name));
    out.push(',');
    text(out, "role", d.role.as_deref());
    out.push(',');
    text(out, "location", d.location.as_deref());
    out.push(',');
    text(out, "pin", d.pin.as_deref());
    out.push(',');
    match d.dial_host() {
        Ok(address) => {
            text(out, "address", Some(&address));
            out.push(',');
            text(out, "unreachable", None);
        }
        Err(why) => {
            text(out, "address", None);
            out.push(',');
            text(out, "unreachable", Some(&why));
        }
    }
    out.push_str(",\"port\":");
    out.push_str(&d.control_plane_port().to_string());
    out.push('}');
}

/// The whole register, as JSON — plus one boolean this server was told about
/// ITSELF, `R39`.
///
/// `fleet_as_code` is `true` when THIS server's own document carries
/// `[applications.rivet-fleetd] fleet-repository` — `crate::code::setting::
/// REPOSITORY`. It is not a second opinion about anybody's state (`ADR-043`
/// §2's principle 2): it is the one setting the Config screen needs in order
/// to say WHY Save is disabled, rather than offering a button that always
/// fails once pressed.
///
/// Hand-written rather than derived: this program does not carry `serde_json`,
/// and `rivet_core::journal::write_json_string` is the workspace's tested
/// RFC 8259 escaper — a device label is a human's text and goes through it.
#[must_use]
pub fn as_json(register: &Register, fleet_as_code: bool) -> String {
    let mut out = String::from("{\"fleet\":{");
    text(&mut out, "name", register.fleet.name.as_deref());
    out.push_str("},\"fleetAsCode\":");
    out.push_str(if fleet_as_code { "true" } else { "false" });
    out.push_str(",\"devices\":[");
    // `Register::devices` is a `BTreeMap`, so this order is the identifier
    // order and is the same on every run — the property `ADR-034` §10.3 asks
    // of the files, holding for the wire too.
    for (n, d) in register.devices.values().enumerate() {
        if n > 0 {
            out.push(',');
        }
        device(&mut out, d);
    }
    out.push_str("]}");
    out
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use rivetos_register::Interface;

    use super::*;

    fn empty_device(id: &str, name: &str) -> Device {
        Device {
            id: id.to_owned(),
            name: name.to_owned(),
            ..Device::default()
        }
    }

    /// **The gap this change closes.** A DHCP member's record carries no
    /// interface address, and this sidebar used to render it `"unreachable"`
    /// even though it is up and answering at `<name>.local`. It must now
    /// render that host as `address`, with `unreachable` absent.
    #[test]
    fn a_dhcp_member_renders_dot_local_and_is_not_unreachable() {
        let mut register = Register::empty("/reg");
        register
            .enroll(empty_device("0011223344556677", "kitchen-pi"))
            .expect("fresh");
        let json = as_json(&register, false);
        assert!(json.contains("\"address\":\"kitchen-pi.local\""), "{json}");
        assert!(json.contains("\"unreachable\":null"), "{json}");
    }

    /// A record carrying more than one address is still refused, never
    /// guessed at, and this sidebar still renders it `unreachable` — the one
    /// genuine `dial_host` refusal `device()` still needs to carry through.
    #[test]
    fn a_multi_address_device_still_renders_unreachable() {
        let mut device = empty_device("8899aabbccddeeff", "garage-pi");
        device.interfaces.insert(
            "eth0".to_owned(),
            Interface {
                method: "static".to_owned(),
                address: Some("192.168.50.10/24".to_owned()),
                gateway: None,
            },
        );
        device.interfaces.insert(
            "wlan0".to_owned(),
            Interface {
                method: "static".to_owned(),
                address: Some("192.168.50.11/24".to_owned()),
                gateway: None,
            },
        );
        let mut register = Register::empty("/reg");
        register.enroll(device).expect("fresh");
        let json = as_json(&register, false);
        assert!(json.contains("\"address\":null"), "{json}");
        assert!(!json.contains("\"unreachable\":null"), "{json}");
    }
}
