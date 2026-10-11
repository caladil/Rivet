//! **A bare device name, resolved against `rivet-fleet`'s own register** —
//! `PLAN-Feature-Set-2026-09-12` §7.1.
//!
//! Today `cargo rivet run` and `cargo rivet install` need `--host`,
//! `--identity` and a sixty-four-character `--pin` on every invocation, and
//! the register `cmd/rivet-fleet` maintains already holds the first and the
//! third for every enrolled device. This module is what lets `run <device>`
//! and `install <device>` supply them.
//!
//! # A rewrite of the words, not of the parser
//!
//! `expand` (below) runs *before* `run::parse` or `deploy::parse` ever sees the
//! argument vector: it recognises a bare leading word as a device name, looks
//! it up, and returns a new word list with `--host`, `--identity` and `--pin`
//! filled in for whichever of the three was not already given. Neither
//! parser changes at all — this crate's two existing, tested command lines
//! are exactly as they were, and every existing test of them is unaffected.
//!
//! **Explicit flags always win.** A word list that already has `--host`,
//! `--identity` or `--pin` in it keeps exactly what it has; the register only
//! ever fills in what is missing (`PLAN-Feature-Set-2026-09-12` §7.1's own
//! words: *"Explicit `--host`/`--pin`/`--identity` still work and win over
//! the register"*).
//!
//! # Which identity — the register does not say
//!
//! [`rivetos_register::Device`] carries no identity field at all — `ADR-034`
//! §3 is exact about why: the register holds no secret and presents no
//! private material, and an operator's own identity is exactly that. What
//! decides it here is the convention this project already documents in
//! several places before this change existed — `ADR-019`'s enrollment table,
//! `README.md`, `skills/reading-a-board/SKILL.md`, and `cmd/rivet-reset`'s own
//! `identity_directory` — that an operator's control-plane identity lives at
//! `~/.rivet/identity/operator`. `default_identity_dir` (below) is that path,
//! restated rather than imported (`cmd/rivet-reset` has no library target),
//! and it is used only when `--identity` was not already given.
//!
//! # Where the register lives
//!
//! `cmd/rivet-fleet` has no library target either, so `default_register_dir`
//! (below) restates the shape of `cmd/rivet-fleet/src/config.rs`'s
//! `Config::register_dir` — the same eleven lines `cmd/rivet-reset`'s
//! `workstation::default_register` already restates for the same reason, with
//! the same three constants.
//!
//! # A record with no address: `<name>.local`, asked of the workstation
//!
//! A DHCP machine's record carries no address, because the register cannot
//! know a lease. Such a machine runs `rivet-mdnsd`, which announces
//! `<hostname>.local` — and the register's label IS the hostname
//! (`cmd/rivet-fleet`'s `compose::hostname`). So `dial_address` (below) asks
//! `rivetos_register::Device::dial_host` for the host to ask about — the one
//! place the `<name>.local` derivation and hostname validation are spelled,
//! shared with `cmd/rivet-fleetd`'s own reachability rendering since
//! 2026-09-22 — and then asks the workstation's own resolver for it on the
//! record's port. Windows and Linux both answer `.local` through their
//! system mDNS, so this module carries no mDNS client of its own. It prints
//! one line naming the name asked and the address used, so a wrong answer
//! from the LAN is visible before anything is sent. Built from the
//! maintainer's first provisioning run on 2026-09-14, where every
//! device-name verb refused his DHCP fleet server (`.rulings/` R-189 D1: a
//! failure in his run is diagnosed from that failure).
//!
//! **IPv4 is preferred when both families answer**, because a composed
//! `listen` of `0.0.0.0:<port>` binds IPv4 alone; an answer with only IPv6 in
//! it is used as it stands, and a machine not listening there fails at the
//! connection, which names the address.
//!
//! # Never touched by a test
//!
//! `default_register_dir` and `expand_from_default` (both below) are the only
//! two functions in this module that read Ignition's configuration or the
//! real register, and `expand` is the one that reaches the workstation's
//! resolver. Every test below builds its own register in a scratch
//! directory and calls `expand_with`, with a resolver it supplies — see
//! `AGENTS.md` §9's operational-safety rule and this worktree's own brief:
//! *"Never read or write the maintainer's real register."*

use std::net::{SocketAddr, ToSocketAddrs as _};
use std::path::{Path, PathBuf};

/// `cmd/rivet-fleet/src/config.rs`'s `APP_NAME`.
const FLEET_APP_NAME: &str = "rivet-fleet";
/// Its `ENV_PREFIX` — `RIVET_FLEET_REGISTER` names a register for one run.
const FLEET_ENV_PREFIX: &str = "RIVET_FLEET";
/// Its `DEFAULT_REGISTER_DIR`.
const FLEET_REGISTER_DIR: &str = "register";

/// Where `rivet-fleet` keeps its register, resolved the way that tool
/// resolves it.
///
/// **Never called by a test** — see this module's header.
///
/// # Errors
///
/// Ignition could not find a configuration directory at all, which is a
/// platform question this end refuses rather than guesses past.
pub fn default_register_dir() -> Result<PathBuf, String> {
    #[derive(Default, serde::Serialize, serde::Deserialize)]
    struct AsFleetSeesIt {
        #[serde(default)]
        register: Option<PathBuf>,
    }
    let loaded: AsFleetSeesIt = ignition::config::Loader::new(FLEET_APP_NAME)
        .env_prefix(FLEET_ENV_PREFIX)
        .load();
    if let Some(named) = loaded.register {
        return Ok(named);
    }
    let beside = ignition::config::config_path(FLEET_APP_NAME).ok_or_else(|| {
        "could not find rivet-fleet's own configuration directory, so there is nowhere to look \
         for its register. Pass --host, --identity and --pin explicitly instead of a device \
         name."
            .to_owned()
    })?;
    let parent = beside
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", beside.display()))?;
    Ok(parent.join(FLEET_REGISTER_DIR))
}

/// **The convention this project already documents in several places**:
/// `ADR-019`'s enrollment table, `README.md`, `skills/reading-a-board/SKILL.md`,
/// and `cmd/rivet-reset`'s own `identity_directory` — an operator's
/// control-plane identity is `~/.rivet/identity/operator`. See this module's
/// header for why nothing on the register decides it.
#[must_use]
pub fn default_identity_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".rivet").join("identity").join("operator"))
}

/// Whether `args`'s first word is a device name rather than a flag — the
/// grammar `run` and `install` share: a bare leading word that is not
/// `confirm` (`install`'s own last-word confirmation).
#[must_use]
pub(crate) fn leading_device_name(args: &[String]) -> Option<&str> {
    match args.first() {
        Some(first) if !first.starts_with('-') && first != "confirm" => Some(first.as_str()),
        _ => None,
    }
}

/// Whether `flag` already appears in `region`.
fn already_given(region: &[String], flag: &str) -> bool {
    region.iter().any(|a| a == flag)
}

/// **Fill in `--host`, `--identity` and `--pin` from `name`'s record in the
/// register at `root`, for whichever of the three `args` does not already
/// give explicitly.**
///
/// `args` is everything AFTER the device name. The region searched for an
/// already-given flag stops at `run`'s `--` separator when one is present —
/// matching `run::parse`'s own reading, which never looks past it — and is
/// the whole of `args` otherwise, matching `deploy::parse`'s grammar, which
/// has no separator of that kind and reads `--host`/`--identity`/`--pin`
/// wherever they appear.
///
/// New flags are PREPENDED, never appended: `install`'s `confirm` must stay
/// the last word (`ADR-012` §7), and prepending is the one position that is
/// right for both grammars.
///
/// # Errors
///
/// The register at `root` cannot be read; `name` answers to no device in it
/// (the refusal lists every name the register holds); the record's address is
/// ambiguous (`rivetos_register::Device::address`'s own refusal) or missing
/// and `<name>.local` does not resolve; or, once the register has been
/// consulted, there is still no pin for this run — recorded nowhere and not
/// given with `--pin`.
pub fn expand(name: &str, args: &[String], root: &Path) -> Result<Vec<String>, String> {
    expand_with(name, args, root, &system_resolve)
}

/// A name and a port in, every address the resolver answered out — the seam
/// [`expand_with`] takes so a test never reaches a real network.
pub type Resolve<'a> = dyn Fn(&str, u16) -> std::io::Result<Vec<SocketAddr>> + 'a;

/// The workstation's own resolver, which is what answers `.local` on Windows
/// and Linux alike.
fn system_resolve(host: &str, port: u16) -> std::io::Result<Vec<SocketAddr>> {
    (host, port).to_socket_addrs().map(Iterator::collect)
}

/// **Where to dial `device`**, as `HOST:PORT`.
///
/// A record that carries an address answers from the record, exactly as it
/// always has, and `resolve` is never called. A record with none asks
/// `resolve` for `<name>.local` — this module's header says why.
///
/// **The address is only where to dial; the pin is still the trust.** Whatever
/// `.local` answers, `expand_with` hands the register's pin to `--pin`, and
/// the handshake refuses a machine that does not hold that key (`ADR-025` §2).
/// A wrong or hostile mDNS answer can make a connection fail; it cannot make
/// this tool trust the machine that answered.
fn dial_address(
    device: &rivetos_register::Device,
    resolve: &Resolve<'_>,
    for_a_plan: bool,
) -> Result<String, String> {
    let port = device.control_plane_port();
    if device.interfaces.values().any(|interface| interface.address.is_some()) {
        return Ok(format!("{}:{port}", device.address()?));
    }
    let name = &device.name;
    // `rivetos_register::Device::dial_host` is the one place `<name>.local`
    // is derived and `name` validated as a hostname — `cmd/rivet-fleetd`
    // needs the same derivation now, which is why it moved off this crate.
    // Its refusal states only the fact; the `--host` advice below is this
    // command line's own, not something the shared method can assume.
    let local = device.dial_host().map_err(|why| {
        format!("{why} Give --host HOST:PORT to name the machine directly. Nothing has been sent.")
    })?;
    let unresolved = |why: String| {
        format!(
            "{name}'s record carries no address (a DHCP machine), so this asked this \
             workstation's resolver for {local}, and {why}. A machine that is powered, on this \
             LAN and running rivet-mdnsd answers that name; otherwise give --host HOST:PORT to \
             name it directly. Nothing has been sent."
        )
    };
    let answered = match resolve(&local, port) {
        Ok(answered) if !answered.is_empty() => answered,
        // **A plan connects to nothing, so a name that does not resolve from
        // here is not a reason to refuse one.** It is named instead: a plan
        // run where there is no network is the moment its NEEDS matter most.
        failed if for_a_plan => {
            let why = failed.map_or_else(|e| e.to_string(), |_| "no address was answered".to_owned());
            eprintln!(
                "cargo-rivet: {name} has no address in the register and {local} did not resolve \
                 from here ({why}); the plan names it, and the run with `confirm` needs it to resolve"
            );
            return Ok(format!("{local}:{port}"));
        }
        Ok(_) => return Err(unresolved("it answered no address at all".to_owned())),
        Err(e) => return Err(unresolved(format!("it did not resolve: {e}"))),
    };
    let chosen = answered
        .iter()
        .find(|address| address.is_ipv4())
        .or_else(|| answered.first())
        .ok_or_else(|| unresolved("it answered no address at all".to_owned()))?;
    eprintln!("cargo-rivet: {name} has no address in the register; {local} resolved to {chosen}");
    Ok(chosen.to_string())
}

/// [`expand`], with the resolver a record with no address is looked up
/// through supplied by the caller.
///
/// # Errors
///
/// [`expand`]'s.
pub fn expand_with(
    name: &str,
    args: &[String],
    root: &Path,
    resolve: &Resolve<'_>,
) -> Result<Vec<String>, String> {
    expand_resolving(name, args, root, resolve, false)
}

/// [`expand_with`] for a PLAN, which connects to nothing: a record with no
/// address whose `<name>.local` does not resolve from here is kept as that
/// name and its port, and said so, rather than refused.
///
/// # Errors
///
/// [`expand_with`]'s, except that a name which does not resolve is not one.
pub fn expand_for_a_plan_with(
    name: &str,
    args: &[String],
    root: &Path,
    resolve: &Resolve<'_>,
) -> Result<Vec<String>, String> {
    expand_resolving(name, args, root, resolve, true)
}

/// [`expand_with`] and [`expand_for_a_plan_with`]'s one body.
fn expand_resolving(
    name: &str,
    args: &[String],
    root: &Path,
    resolve: &Resolve<'_>,
    for_a_plan: bool,
) -> Result<Vec<String>, String> {
    let flag_region: &[String] = match args.iter().position(|a| a == "--") {
        Some(at) => &args[..at],
        None => args,
    };

    let register = rivetos_register::store::load(root)
        .map_err(|e| format!("could not read the register at {}: {e}", root.display()))?;
    let device = register.resolve(name).map_err(|_| {
        let mut names: Vec<&str> = register.devices.values().map(|d| d.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        if names.is_empty() {
            format!(
                "{name:?} answers to no device, and the register at {} holds none at all.",
                root.display()
            )
        } else {
            format!(
                "{name:?} answers to no device in the register at {}. It holds: {}.",
                root.display(),
                names.join(", ")
            )
        }
    })?;

    let mut prefix: Vec<String> = Vec::new();

    if !already_given(flag_region, "--host") {
        prefix.push("--host".to_owned());
        prefix.push(dial_address(device, resolve, for_a_plan)?);
    }

    if !already_given(flag_region, "--identity")
        && let Some(dir) = default_identity_dir()
    {
        prefix.push("--identity".to_owned());
        prefix.push(dir.display().to_string());
    }

    if !already_given(flag_region, "--pin") {
        match device.pin.as_deref().filter(|p| !p.is_empty()) {
            Some(pin) => {
                prefix.push("--pin".to_owned());
                prefix.push(pin.to_owned());
            }
            None => {
                return Err(format!(
                    "{name} has no control-plane key recorded in the register at {} and none \
                     was given with --pin, so there is nothing to pin and this end will not \
                     trust whatever answers at that address (ADR-025 §2).\n\
                     \x20 fix  `rivet-fleet set pin --device {name} --pin <hex>` records one \
                     this machine already has, or pass --pin <hex> for this run alone.",
                    root.display()
                ));
            }
        }
    }

    let mut out = prefix;
    out.extend(args.iter().cloned());
    Ok(out)
}

/// **[`expand`], reached through the register `rivet-fleet` actually uses.**
///
/// `Ok(args.to_vec())` unchanged when `args` names no leading device — the
/// ordinary case, and the one where nothing here touches a filesystem at all.
///
/// **Never called by a test** — see this module's header.
///
/// # Errors
///
/// [`default_register_dir`]'s, or [`expand`]'s.
pub fn expand_from_default(args: &[String]) -> Result<Vec<String>, String> {
    let Some(name) = leading_device_name(args) else {
        return Ok(args.to_vec());
    };
    let root = default_register_dir()?;
    expand(name, &args[1..], &root)
}

/// [`expand_from_default`] for a PLAN — see [`expand_for_a_plan_with`].
///
/// **Never called by a test** — see this module's header.
///
/// # Errors
///
/// [`expand_from_default`]'s, except that a name which does not resolve is
/// not one.
pub fn expand_from_default_for_a_plan(args: &[String]) -> Result<Vec<String>, String> {
    let Some(name) = leading_device_name(args) else {
        return Ok(args.to_vec());
    };
    let root = default_register_dir()?;
    expand_for_a_plan_with(name, &args[1..], &root, &system_resolve)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use rivetos_register::{Device, Interface, Register};
    use std::fs;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A scratch directory under the OS temp dir, removed when it drops.
    /// **Never the maintainer's real register** — every test here builds its
    /// own and points `expand` at it explicitly.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            Self(crate::scratch::dir(&format!("device-test-{label}-{n}")))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|w| (*w).to_string()).collect()
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

    fn device_with_address(id: &str, name: &str, listen: &str, address: &str) -> Device {
        let mut device = Device {
            id: id.to_owned(),
            name: name.to_owned(),
            listen: Some(listen.to_owned()),
            pin: Some("c0ffee".repeat(11)[..64].to_owned()),
            ..Device::default()
        };
        device.interfaces.insert(
            "eth0".to_owned(),
            Interface {
                method: "static".to_owned(),
                address: Some(format!("{address}/24")),
                gateway: None,
            },
        );
        device
    }

    // ---- the case named explicitly in the plan: listen 0.0.0.0 -------------

    /// **`rivet-01`'s own shape.** A `listen` of `0.0.0.0:<port>` means dial
    /// the device's recorded INTERFACE address on that port, never the literal
    /// `0.0.0.0` — `PLAN-Feature-Set-2026-09-12` §7.1 names this case
    /// explicitly and asks for it to be tested on its own.
    #[test]
    fn a_listen_of_all_interfaces_dials_the_recorded_interface_address() {
        let scratch = register_with(vec![device_with_address(
            "0011223344556677",
            "rivet-01",
            "0.0.0.0:7777",
            "192.168.50.10",
        )]);
        let expanded =
            expand("rivet-01", &words(&["--", "get", "storage"]), &scratch.0).expect("resolves");
        assert_eq!(
            expanded,
            words(&[
                "--host",
                "192.168.50.10:7777",
                "--identity",
                &default_identity_dir().expect("HOME or USERPROFILE is set in this environment").display().to_string(),
                "--pin",
                &"c0ffee".repeat(11)[..64],
                "--",
                "get",
                "storage",
            ])
        );
    }

    // ---- explicit flags win --------------------------------------------------

    #[test]
    fn an_explicit_host_is_kept_and_the_register_only_fills_identity_and_pin() {
        let scratch = register_with(vec![device_with_address(
            "0011223344556677",
            "rivet-01",
            "0.0.0.0:7777",
            "192.168.50.10",
        )]);
        let expanded = expand(
            "rivet-01",
            &words(&["--host", "10.0.0.9:9999", "--", "get", "storage"]),
            &scratch.0,
        )
        .expect("resolves");
        // The explicit host survives, unaltered, and the register's address
        // is nowhere in the result.
        assert!(expanded.iter().any(|a| a == "10.0.0.9:9999"));
        assert!(!expanded.iter().any(|a| a == "192.168.50.10:7777"));
        assert!(expanded.iter().any(|a| a == "--pin"));
    }

    #[test]
    fn an_explicit_identity_and_pin_are_kept_and_only_the_host_is_filled() {
        let scratch = register_with(vec![device_with_address(
            "0011223344556677",
            "rivet-01",
            "0.0.0.0:7777",
            "192.168.50.10",
        )]);
        let expanded = expand(
            "rivet-01",
            &words(&["--identity", "./mine", "--pin", "ab", "--", "get", "storage"]),
            &scratch.0,
        )
        .expect("resolves");
        assert!(expanded.iter().any(|a| a == "./mine"));
        assert!(expanded.iter().any(|a| a == "ab"));
        assert!(!expanded.iter().any(|a| a == &"c0ffee".repeat(11)[..64]));
        assert!(expanded.iter().any(|a| a == "--host"));
    }

    // ---- an unknown name is refused by name, and lists what it knows -------

    #[test]
    fn a_name_not_in_the_register_is_refused_and_the_refusal_lists_known_names() {
        let scratch = register_with(vec![
            device_with_address("0011223344556677", "rivet-01", "0.0.0.0:7777", "192.168.50.10"),
            device_with_address("8899aabbccddeeff", "kitchen-pi", "0.0.0.0:7777", "192.168.50.11"),
        ]);
        let refused = expand("nonesuch", &words(&["--", "get", "storage"]), &scratch.0)
            .expect_err("no such device");
        assert!(refused.contains("nonesuch"), "{refused}");
        assert!(refused.contains("rivet-01"), "{refused}");
        assert!(refused.contains("kitchen-pi"), "{refused}");
    }

    #[test]
    fn a_name_when_the_register_holds_nothing_at_all_says_so() {
        let scratch = Scratch::new("empty");
        let refused = expand("rivet-01", &words(&["--", "get", "storage"]), &scratch.0)
            .expect_err("no such device");
        assert!(refused.contains("rivet-01"), "{refused}");
        assert!(refused.contains("holds none at all"), "{refused}");
    }

    // ---- no pin recorded and none given is a refusal, not a silent gap ----

    #[test]
    fn a_device_with_no_pin_recorded_and_none_given_is_refused() {
        let mut device = device_with_address(
            "0011223344556677",
            "rivet-01",
            "0.0.0.0:7777",
            "192.168.50.10",
        );
        device.pin = None;
        let scratch = register_with(vec![device]);
        let refused = expand("rivet-01", &words(&["--", "get", "storage"]), &scratch.0)
            .expect_err("no pin anywhere");
        assert!(refused.contains("no control-plane key recorded"), "{refused}");
        assert!(refused.contains("--pin"), "{refused}");
    }

    /// An explicit `--pin` still works even when the register holds none.
    #[test]
    fn an_explicit_pin_supplies_what_the_register_does_not_have() {
        let mut device = device_with_address(
            "0011223344556677",
            "rivet-01",
            "0.0.0.0:7777",
            "192.168.50.10",
        );
        device.pin = None;
        let scratch = register_with(vec![device]);
        let expanded = expand(
            "rivet-01",
            &words(&["--pin", "ab", "--", "get", "storage"]),
            &scratch.0,
        )
        .expect("an explicit pin is enough");
        assert!(expanded.iter().any(|a| a == "ab"));
    }

    // ---- deploy's grammar: no `--` separator, confirm stays last -----------

    #[test]
    fn deploy_style_args_with_no_separator_are_scanned_whole_and_confirm_stays_last() {
        let scratch = register_with(vec![device_with_address(
            "0011223344556677",
            "rivet-01",
            "0.0.0.0:7777",
            "192.168.50.10",
        )]);
        let expanded =
            expand("rivet-01", &words(&["--bin", "todo", "confirm"]), &scratch.0).expect("resolves");
        assert_eq!(expanded.last().map(String::as_str), Some("confirm"), "{expanded:?}");
        assert!(expanded.iter().any(|a| a == "--host"));
        assert!(expanded.iter().any(|a| a == "--bin"));
    }

    // ---- `leading_device_name` and the whole-vector entry point -----------

    #[test]
    fn a_word_starting_with_a_dash_is_never_a_device_name() {
        assert_eq!(leading_device_name(&words(&["--host", "pi:7777"])), None);
        assert_eq!(leading_device_name(&words(&["--"])), None);
    }

    #[test]
    fn the_bare_word_confirm_is_never_a_device_name() {
        assert_eq!(leading_device_name(&words(&["confirm"])), None);
    }

    #[test]
    fn a_bare_leading_word_is_the_device_name() {
        assert_eq!(leading_device_name(&words(&["rivet-01", "--", "get", "storage"])), Some("rivet-01"));
    }

    #[test]
    fn expand_from_default_is_a_no_op_when_no_device_name_leads() {
        let args = words(&["--host", "pi:7777", "--", "get", "storage"]);
        assert_eq!(expand_from_default(&args), Ok(args));
    }

    // ---- ambiguous / missing address on the record --------------------------

    // ---- a record with no address: `<name>.local` through the resolver -----

    /// `rivet-08ecc`'s own shape, from the maintainer's first provisioning
    /// run: a DHCP interface, so no address, and a routable `listen`.
    fn dhcp_device(name: &str) -> Device {
        let mut device = Device {
            id: "0011223344556677".to_owned(),
            name: name.to_owned(),
            listen: Some("0.0.0.0:7777".to_owned()),
            pin: Some("ab".repeat(32)),
            ..Device::default()
        };
        device.interfaces.insert(
            "eth0".to_owned(),
            Interface { method: "dhcp".to_owned(), address: None, gateway: None },
        );
        device
    }

    fn answers(list: &[&str]) -> std::io::Result<Vec<SocketAddr>> {
        Ok(list.iter().map(|a| a.parse().expect("a socket address")).collect())
    }

    #[test]
    fn a_record_with_no_address_dials_what_its_local_name_resolves_to_with_the_registers_pin() {
        let scratch = register_with(vec![dhcp_device("rivet-08ecc")]);
        let asked = std::cell::RefCell::new(Vec::new());
        let resolve = |host: &str, port: u16| {
            asked.borrow_mut().push(format!("{host}:{port}"));
            answers(&["[fe80::1]:7777", "192.168.50.66:7777"])
        };
        let expanded = expand_with("rivet-08ecc", &words(&["--", "get", "status"]), &scratch.0, &resolve)
            .expect("resolves by name");
        assert_eq!(asked.borrow().as_slice(), ["rivet-08ecc.local:7777"]);
        let host = expanded.iter().position(|a| a == "--host").expect("a host was filled");
        // IPv4 is preferred even when the resolver answered IPv6 first.
        assert_eq!(expanded[host + 1], "192.168.50.66:7777");
        let pin = expanded.iter().position(|a| a == "--pin").expect("a pin was filled");
        assert_eq!(expanded[pin + 1], "ab".repeat(32), "the register's pin is still the trust");
    }

    #[test]
    fn an_answer_with_only_ipv6_in_it_is_used() {
        let scratch = register_with(vec![dhcp_device("rivet-08ecc")]);
        let resolve = |_: &str, _: u16| answers(&["[fd00::66]:7777"]);
        let expanded = expand_with("rivet-08ecc", &words(&["--", "get", "status"]), &scratch.0, &resolve)
            .expect("resolves by name");
        assert!(expanded.iter().any(|a| a == "[fd00::66]:7777"), "{expanded:?}");

        // A link-local answer carries a scope, and the text handed to --host
        // must read back as the same address when the transport dials it.
        let scoped = SocketAddr::V6(std::net::SocketAddrV6::new("fe80::66".parse().expect("v6"), 7777, 0, 12));
        let resolve = move |_: &str, _: u16| Ok(vec![scoped]);
        let expanded = expand_with("rivet-08ecc", &words(&["--", "get", "status"]), &scratch.0, &resolve)
            .expect("resolves by name");
        let host = expanded.iter().position(|a| a == "--host").expect("a host was filled");
        assert_eq!(expanded[host + 1].parse::<SocketAddr>().expect("reads back"), scoped);
    }

    #[test]
    fn an_unresolvable_local_name_is_refused_naming_the_name_tried_and_host() {
        let scratch = register_with(vec![dhcp_device("rivet-08ecc")]);
        let failing = |_: &str, _: u16| -> std::io::Result<Vec<SocketAddr>> {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no such host"))
        };
        let refused = expand_with("rivet-08ecc", &words(&["--", "get", "status"]), &scratch.0, &failing)
            .expect_err("nothing answered");
        assert!(refused.contains("rivet-08ecc.local"), "{refused}");
        assert!(refused.contains("--host HOST:PORT"), "{refused}");

        let silent = |_: &str, _: u16| answers(&[]);
        let refused = expand_with("rivet-08ecc", &words(&["--", "get", "status"]), &scratch.0, &silent)
            .expect_err("an empty answer is not an address");
        assert!(refused.contains("rivet-08ecc.local"), "{refused}");
        assert!(refused.contains("--host HOST:PORT"), "{refused}");
    }

    /// **A plan names a name it could not resolve rather than refusing.** A
    /// plan connects to nothing, and one run with no network at all is the
    /// case its NEEDS exist for; the pin is still the register's.
    #[test]
    fn a_plan_keeps_an_unresolvable_local_name_and_its_port() {
        let scratch = register_with(vec![dhcp_device("rivet-08ecc")]);
        let failing = |_: &str, _: u16| -> std::io::Result<Vec<SocketAddr>> {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no such host"))
        };
        let expanded = expand_for_a_plan_with("rivet-08ecc", &words(&["confirm"]), &scratch.0, &failing)
            .expect("a plan does not need the name to resolve");
        let host = expanded.iter().position(|a| a == "--host").expect("a host was filled");
        assert_eq!(expanded[host + 1], "rivet-08ecc.local:7777");
        let pin = expanded.iter().position(|a| a == "--pin").expect("a pin was filled");
        assert_eq!(expanded[pin + 1], "ab".repeat(32));

        let silent = |_: &str, _: u16| answers(&[]);
        let expanded = expand_for_a_plan_with("rivet-08ecc", &words(&[]), &scratch.0, &silent)
            .expect("an empty answer is not a refusal for a plan either");
        assert!(expanded.iter().any(|a| a == "rivet-08ecc.local:7777"), "{expanded:?}");
    }

    #[test]
    fn an_explicit_host_wins_and_the_resolver_is_never_asked() {
        let scratch = register_with(vec![dhcp_device("rivet-08ecc")]);
        let never = |host: &str, _: u16| -> std::io::Result<Vec<SocketAddr>> {
            panic!("an explicit --host must not be resolved past, but {host} was asked")
        };
        let expanded = expand_with(
            "rivet-08ecc",
            &words(&["--host", "192.168.50.66:7777", "--", "get", "status"]),
            &scratch.0,
            &never,
        )
        .expect("the explicit host is enough");
        assert_eq!(expanded.iter().filter(|a| *a == "--host").count(), 1, "{expanded:?}");
        assert!(expanded.iter().any(|a| a == "192.168.50.66:7777"));
    }

    #[test]
    fn a_record_with_an_address_never_asks_the_resolver() {
        let scratch = register_with(vec![device_with_address(
            "0011223344556677",
            "rivet-01",
            "0.0.0.0:7777",
            "192.168.50.10",
        )]);
        let never = |host: &str, _: u16| -> std::io::Result<Vec<SocketAddr>> {
            panic!("a recorded address must be dialled as it stands, but {host} was asked")
        };
        let expanded = expand_with("rivet-01", &words(&["--", "get", "storage"]), &scratch.0, &never)
            .expect("resolves from the record");
        assert!(expanded.iter().any(|a| a == "192.168.50.10:7777"), "{expanded:?}");
    }

    #[test]
    fn a_record_with_no_address_and_a_label_that_is_not_a_hostname_is_refused() {
        let mut device = dhcp_device("kitchen pi");
        device.interfaces.clear();
        let scratch = register_with(vec![device]);
        let never = |host: &str, _: u16| -> std::io::Result<Vec<SocketAddr>> {
            panic!("no .local name exists for this label, but {host} was asked")
        };
        let refused = expand_with("kitchen pi", &words(&["--", "get", "storage"]), &scratch.0, &never)
            .expect_err("no name to ask for");
        assert!(refused.contains("carries no address"), "{refused}");
        assert!(refused.contains("--host HOST:PORT"), "{refused}");
    }
}
