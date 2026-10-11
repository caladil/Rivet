//! TOML text in, [`Document`] out — or a refusal that names the field.
//!
//! `ADR-007` §6 item 2 ruled the format: TOML. §3.1 is careful about what that
//! means — TOML is a *representation*, and this module is where it stops being
//! one and becomes the typed store that is the authority.
//!
//! # Why the walk is hand-written
//!
//! `serde` with `deny_unknown_fields` would reject an unknown key too, but it
//! reports which key only inside a message string, so a typed
//! [`ConfigError::UnknownKey`] would have to be recovered by matching on a
//! dependency's prose. `ADR-009` §3.4 is explicit that a dependency reaching
//! Rivet's configuration schema is the expensive kind, and §4 forbids its
//! types appearing on Rivet's surface at all. The walk below is a few lines
//! per field and every diagnostic it produces is Rivet's own.
//!
//! `toml_edit` is used only as a parser: it produces a document that keeps the
//! byte span of every key and value, which is what lets an error carry a line
//! and column instead of quoting the text at it (see [`crate::error`]).

// `toml_edit::Document` keeps the byte span of every key and value; the
// mutable `DocumentMut` discards them. Aliased because `Document` is also the
// name of the thing this module produces, and the two must not be confusable.
use toml_edit::{Document as TomlDocument, Item, Table, TableLike};

use std::net::Ipv4Addr;

use rivet_core::CoreError;
use rivet_core::net::{InterfaceName, Ipv4Cidr, StaticAddressing};

use crate::document::{Document, SCHEMA_VERSION, SchemaVersion};
use crate::error::{ConfigError, Location};
use crate::network::{Addressing, Interface, Method};
use crate::volumes::{Guid, Volume, VolumeName};
use crate::services::{CpuLimit, MemoryLimit, Service, ServiceName};
use crate::control_plane::ControlPlane;
use crate::dns::{Dns, Label, Record, Ttl, Zone};
use crate::kiosk::Kiosk;
use crate::resolver::{Resolver, SearchDomain};
use crate::applications::{ApplicationSettings, Applications, Ignored};
use crate::display::Display;
use crate::splash::Splash;
use rivet_core::ssid::Ssid;
use rivet_core::hostname::Hostname;
use rivet_core::display::{Hours, TimeOfDay};
use rivet_core::timezone::Timezone;
use crate::repos::{Repo, RepoId, RepoName};
use crate::carry::Carry;
use crate::fleet_role::FleetRole;
use crate::machine_mode::MachineMode;

/// Parse the single configuration document.
pub(crate) fn document(src: &str) -> Result<Document, ConfigError> {
    let parsed = TomlDocument::parse(src).map_err(|e| ConfigError::Malformed {
        at: e.span().map(|s| Location::of(src, s.start)),
        // The parser's own words, per `AGENTS.md` §5. `TomlError::message` is
        // built from the grammar — what was expected where — and does not
        // include the input, which is what keeps a passphrase out of the
        // message (`ADR-020` §4).
        detail: e.message().to_owned(),
    })?;

    let root = parsed.as_table();

    // Unknown keys are refused before anything is read, so that a misspelled
    // `verison` is reported as the typo it is rather than as a missing
    // `version` — the operator sees the mistake they actually made.
    refuse_unknown_keys(src, root, "", Document::DECLARED_KEYS)?;

    let version = read_schema_version(src, root)?;
    let services = read_services(src, root)?;
    let (network, resolver) = read_network(src, root)?;
    let volumes = read_volumes(src, root)?;
    let control_plane = read_control_plane(src, root)?;
    let splash = read_splash(src, root)?;
    let kiosk = read_kiosk(src, root)?;
    let hostname = read_hostname(src, root)?;
    let timezone = read_timezone(src, root)?;
    let display = read_display(src, root)?;
    let applications = read_applications(src, root)?;
    let repos = read_repos(src, root)?;
    let carry = read_carry(src, root)?;
    let mode = read_machine_mode(src, root)?;
    let fleet_role = read_fleet_role(src, root)?;
    let dns = read_dns(src, root)?;

    Ok(Document::new(
        version, services, network, volumes, control_plane, splash, resolver, kiosk,
        hostname,
        timezone,
        display,
        applications,
        repos,
        carry,
        mode,
        fleet_role,
        dns,
    ))
}

/// [`crate::applications::parse_only`]'s implementation — [`read_applications`]
/// run against just enough of a parsed table to reach `[applications]`,
/// without [`document`]'s `refuse_unknown_keys` or any of its other fields.
pub(crate) fn applications_only(src: &str) -> Result<Applications, ConfigError> {
    let parsed = TomlDocument::parse(src).map_err(|e| ConfigError::Malformed {
        at: e.span().map(|s| Location::of(src, s.start)),
        detail: e.message().to_owned(),
    })?;
    let root = parsed.as_table();
    read_applications(src, root)
}

/// `ADR-007` §3.6.5: *unknown configuration fields are rejected at parse time
/// with a typed error naming the field, never ignored.*
///
/// Reports the first offender in document order, so the operator fixes the one
/// they can see rather than the last one in a hash iteration.
fn refuse_unknown_keys(
    src: &str,
    table: &dyn TableLike,
    prefix: &str,
    declared: &'static [&'static str],
) -> Result<(), ConfigError> {
    for (name, _) in table.iter() {
        if declared.contains(&name) {
            continue;
        }
        return Err(ConfigError::UnknownKey {
            path: path(prefix, name),
            at: key_location(src, table, name),
            expected: declared,
        });
    }
    Ok(())
}

/// Where a key appears, when the parser recorded it.
fn key_location(src: &str, table: &dyn TableLike, name: &str) -> Option<Location> {
    table
        .get_key_value(name)
        .and_then(|(k, _)| k.span())
        .map(|s| Location::of(src, s.start))
}

/// Where a value appears, falling back to its key when the value has no span of
/// its own — an implicit table (`[services.rivet-apid]` creates one for
/// `services`) has no text to point at.
fn value_location(src: &str, table: &dyn TableLike, name: &str) -> Option<Location> {
    table
        .get(name)
        .and_then(toml_edit::Item::span)
        .map(|s| Location::of(src, s.start))
        .or_else(|| key_location(src, table, name))
}

/// Read the required `version` key.
fn read_schema_version(src: &str, root: &Table) -> Result<SchemaVersion, ConfigError> {
    const PATH: &str = "version";
    const EXPECTED: &str = "an integer, the schema version this document is written against";

    let Some(item) = root.get(PATH) else {
        return Err(ConfigError::MissingKey {
            path: PATH.to_owned(),
            expected: EXPECTED,
        });
    };

    let at = item.span().map(|s| Location::of(src, s.start));

    let Some(raw) = item.as_integer() else {
        return Err(ConfigError::WrongType {
            path: PATH.to_owned(),
            at,
            expected: EXPECTED,
            found: item.type_name(),
        });
    };

    let Ok(found) = u32::try_from(raw) else {
        return Err(ConfigError::InvalidValue {
            path: PATH.to_owned(),
            at,
            detail: format!("{raw} is not a schema version; versions count up from 1"),
        });
    };

    if found == 0 {
        return Err(ConfigError::InvalidValue {
            path: PATH.to_owned(),
            at,
            detail: "0 is not a schema version; versions count up from 1".to_owned(),
        });
    }

    if found != SCHEMA_VERSION.get() {
        return Err(ConfigError::UnsupportedSchemaVersion {
            found,
            supported: SCHEMA_VERSION.get(),
            at,
        });
    }

    Ok(SchemaVersion::new(found))
}

/// Read the optional `[services]` table — the operator's whole surface over
/// services, per `ADR-017` §4.1.
///
/// **Absent is a complete answer**, and the only one it can be: the operator
/// has configured no service. That is [`crate::services`]'s rule that absence
/// is allowed to mean something only when it means exactly one thing, and it is
/// why `enabled` inside a named service is required while this table is not.
fn read_services(src: &str, root: &Table) -> Result<Vec<Service>, ConfigError> {
    const PATH: &str = "services";
    const EXPECTED: &str = "a table of services, one sub-table per service name";

    let Some(item) = root.get(PATH) else {
        return Ok(Vec::new());
    };

    let Some(table) = item.as_table_like() else {
        return Err(ConfigError::WrongType {
            path: PATH.to_owned(),
            at: value_location(src, root, PATH),
            expected: EXPECTED,
            found: item.type_name(),
        });
    };

    let mut services = Vec::new();

    // ~~Which endpoint each service has claimed so far~~ — **removed
    // 2026-08-24** with the `listen` key (the maintainer's 2026-08-24 ruling). Two services
    // claiming one socket is still two authorities over one fact and is still
    // forbidden; what changed is that this crate can no longer be the thing that
    // notices, because the claims are now in two signed manifests rather than in
    // one document. `cmd/rivet-sysd` holds every definition at once and
    // is where the check went.

    for (name, entry) in table.iter() {
        let key_path = path(PATH, name);

        // The name is checked, the *existence* of the service is not — this
        // crate does no I/O and cannot know what is installed. See
        // `crate::services` for why that is a decision rather than a gap.
        let Some(service_name) = ServiceName::new(name) else {
            return Err(ConfigError::InvalidKey {
                path: key_path,
                at: key_location(src, table, name),
                rule: ServiceName::RULE,
            });
        };

        let Some(settings) = entry.as_table_like() else {
            return Err(ConfigError::WrongType {
                path: key_path,
                at: value_location(src, table, name),
                expected: "a table of settings for one service",
                found: entry.type_name(),
            });
        };

        services.push(read_service(src, settings, service_name, &key_path)?);
    }

    Ok(services)
}

/// Read one `[services.<name>]` entry.
///
/// Unknown keys first, then the required key, then the optional ones — the same
/// order as the document as a whole, and for the same reason: an operator who
/// wrote `exec` should be told about `exec`, not about a missing `enabled`.
fn read_service(
    src: &str,
    table: &dyn TableLike,
    name: ServiceName,
    prefix: &str,
) -> Result<Service, ConfigError> {
    refuse_keys_that_are_not_the_operators(src, table, prefix)?;
    let enabled = read_enabled(src, table, prefix)?;
    let memory = read_memory(src, table, prefix)?;
    let cpu = read_cpu(src, table, prefix)?;
    let critical = read_critical(src, table, prefix)?;
    Ok(Service::new(name, enabled, memory, cpu, critical))
}

/// `critical` — optional, and **absent means `false`** (`ADR-045` §4).
///
/// It is optional where `enabled` is required because absence means exactly one
/// thing here: the operator has said nothing about this service's importance,
/// and *nothing you told me to watch is wrong* is the honest reading of a
/// machine that was told to watch nothing. `ADR-045` §7 depends on that default
/// — a gate that requires six things to be true is a gate that never passes.
fn read_critical(src: &str, table: &dyn TableLike, prefix: &str) -> Result<bool, ConfigError> {
    const KEY: &str = "critical";
    const EXPECTED: &str =
        "true or false — whether this service's health moves the MACHINE's answer (ADR-045 §4)";

    let Some(item) = table.get(KEY) else {
        return Ok(false);
    };

    let Some(critical) = item.as_bool() else {
        return Err(ConfigError::WrongType {
            path: path(prefix, KEY),
            at: value_location(src, table, KEY),
            expected: EXPECTED,
            found: item.type_name(),
        });
    };

    Ok(critical)
}

/// Three outcomes rather than two: declared, part of the definition, or unknown.
///
/// The middle one is the whole point of `ADR-017` §4.1. An operator who writes
/// `exec` has a wrong model rather than a typo, and a message saying only
/// "unknown key" would leave it wrong — see [`ConfigError::NotConfigurable`].
fn refuse_keys_that_are_not_the_operators(
    src: &str,
    table: &dyn TableLike,
    prefix: &str,
) -> Result<(), ConfigError> {
    for (name, _) in table.iter() {
        if Service::DECLARED_KEYS.contains(&name) {
            continue;
        }
        let at = key_location(src, table, name);
        let path = path(prefix, name);
        if Service::DEFINITION_KEYS.contains(&name) {
            return Err(ConfigError::NotConfigurable {
                path,
                at,
                why: Service::why_not_configurable(name),
                expected: Service::DECLARED_KEYS,
            });
        }
        return Err(ConfigError::UnknownKey {
            path,
            at,
            expected: Service::DECLARED_KEYS,
        });
    }
    Ok(())
}

/// `enabled` — required. `ADR-017` §6 makes it the entire control surface, and
/// the definition has no counterpart to inherit from, so absent means *nobody
/// said* rather than *leave it as it was*.
fn read_enabled(src: &str, table: &dyn TableLike, prefix: &str) -> Result<bool, ConfigError> {
    const KEY: &str = "enabled";
    const EXPECTED: &str = "true or false — whether this service should be running";

    let key_path = path(prefix, KEY);

    let Some(item) = table.get(KEY) else {
        return Err(ConfigError::MissingKey {
            path: key_path,
            expected: EXPECTED,
        });
    };

    let Some(enabled) = item.as_bool() else {
        return Err(ConfigError::WrongType {
            path: key_path,
            at: value_location(src, table, KEY),
            expected: EXPECTED,
            found: item.type_name(),
        });
    };

    Ok(enabled)
}

// ---------------------------------------------------------------------------
// A function that used to be here: read_listen — deleted 2026-08-24
// ---------------------------------------------------------------------------
//
// It read `listen = ["tcp:22"]`, distinguished an absent array from an empty
// one, and refused two services claiming one socket. All of it was correct for
// a document that owned the key, and the key is no longer the document's
// (the maintainer's 2026-08-24 ruling). `listen` now reaches
// `refuse_keys_that_are_not_the_operators` and is refused with the explanation
// `Service::why_not_configurable` carries.
//
// **What was lost with it, said rather than quietly dropped:** the
// two-services-one-socket check. It was cheap here because one document held
// every claim; the claims are now spread across signed manifests that this
// crate cannot read, so the check moved to the supervisor, which holds them all
// at once. `crate::services`'s header carries the same note from the other
// side.

/// `memory` — optional. Absent means the operator stated no cap, so the
/// artifact's own request stands; resolving the two is the supervisor's job
/// (`ADR-017` §4.1), not this crate's.
fn read_memory(
    src: &str,
    table: &dyn TableLike,
    prefix: &str,
) -> Result<Option<MemoryLimit>, ConfigError> {
    const KEY: &str = "memory";
    const EXPECTED: &str = "a memory limit written as a string, for example \"128M\"";

    let key_path = path(prefix, KEY);

    let Some(item) = table.get(KEY) else {
        return Ok(None);
    };

    let at = value_location(src, table, KEY);

    let Some(text) = item.as_str() else {
        return Err(ConfigError::WrongType {
            path: key_path,
            at,
            expected: EXPECTED,
            found: item.type_name(),
        });
    };

    // As in `read_listen`: the rule, never the text.
    let Some(limit) = MemoryLimit::parse(text) else {
        return Err(ConfigError::InvalidValue {
            path: key_path,
            at,
            detail: format!("is not a memory limit; {}", MemoryLimit::RULE),
        });
    };

    Ok(Some(limit))
}

/// `cpu` — optional, on [`read_memory`]'s footing exactly. Absent means the
/// operator stated no cap, so the artifact's own request stands; resolving
/// the two is `rivet_core::service::effective_cpu`'s job and
/// `cmd/rivet-sysd`'s to perform, not this crate's — `ADR-007` §7 and
/// `ADR-015` §3's 2026-09-09 amendment.
fn read_cpu(src: &str, table: &dyn TableLike, prefix: &str) -> Result<Option<CpuLimit>, ConfigError> {
    const KEY: &str = "cpu";
    const EXPECTED: &str = "a decimal number of cores written as a string, for example \"0.5\"";

    let key_path = path(prefix, KEY);

    let Some(item) = table.get(KEY) else {
        return Ok(None);
    };

    let at = value_location(src, table, KEY);

    let Some(text) = item.as_str() else {
        return Err(ConfigError::WrongType {
            path: key_path,
            at,
            expected: EXPECTED,
            found: item.type_name(),
        });
    };

    let Some(limit) = CpuLimit::parse(text) else {
        return Err(ConfigError::InvalidValue {
            path: key_path,
            at,
            detail: format!("is not a cpu limit; {}", CpuLimit::RULE),
        });
    };

    Ok(Some(limit))
}

/// Read the optional `[network]` table — how this machine is told its own
/// address, per the maintainer's ruling of 2026-08-23 (option 2).
///
/// **Absent is a complete answer**: the operator has configured no interface.
/// See [`crate::network`] for why `method` inside an interface is nonetheless
/// required — absence is allowed to mean something only when it means exactly
/// one thing.
#[allow(clippy::type_complexity)]
fn read_network(
    src: &str,
    root: &Table,
) -> Result<(Vec<Interface>, Option<Resolver>), ConfigError> {
    const PATH: &str = "network";
    // **"plus one reserved word", and the reserved word is named by the type**
    // — `Resolver::TABLE`. Ruled 2026-09-01: the resolver lives inside
    // `[network]` because that is where an operator looks when DNS is broken,
    // at the cost of an interface literally called `resolver` being
    // unconfigurable. See `crate::resolver` for the whole trade.
    const EXPECTED: &str =
        "a table of interfaces, one sub-table per interface name, plus the optional \
         reserved sub-table `resolver`";

    let Some(item) = root.get(PATH) else {
        return Ok((Vec::new(), None));
    };

    let Some(table) = item.as_table_like() else {
        return Err(ConfigError::WrongType {
            path: PATH.to_owned(),
            at: value_location(src, root, PATH),
            expected: EXPECTED,
            found: item.type_name(),
        });
    };

    let mut resolver = None;
    let mut interfaces = Vec::new();
    // Which host address each interface has claimed so far, in document order.
    // See `read_addressing` for why one document may not claim one twice.
    let mut claimed: Vec<(Ipv4Addr, InterfaceName)> = Vec::new();

    for (name, entry) in table.iter() {
        let key_path = path(PATH, name);

        // **Checked before the name is read as an interface**, because
        // `resolver` IS a valid interface name — that is the whole reason it
        // has to be reserved rather than merely happening not to collide.
        if name == Resolver::TABLE {
            resolver = Some(read_resolver(src, table, &key_path, entry)?);
            continue;
        }

        // The name is checked against the kernel's own `dev_valid_name()`
        // rules, which `rivet-core` owns. Whether the interface *exists* is
        // not checked and cannot be: this crate performs no I/O (`ADR-014`
        // §3.1) and the kernel is the authority for that (`AGENTS.md` §5).
        let Ok(interface_name) = InterfaceName::new(name) else {
            return Err(ConfigError::InvalidKey {
                path: key_path,
                at: key_location(src, table, name),
                rule: InterfaceName::RULE,
            });
        };

        let Some(settings) = entry.as_table_like() else {
            return Err(ConfigError::WrongType {
                path: key_path,
                at: value_location(src, table, name),
                expected: "a table of settings for one interface",
                found: entry.type_name(),
            });
        };

        let addressing =
            read_addressing(src, settings, &key_path, &interface_name, &mut claimed)?;
        let ssid = read_ssid(src, settings, &key_path)?;
        interfaces.push(Interface::new(interface_name, addressing, ssid));
    }

    Ok((interfaces, resolver))
}

/// `[network.resolver]` — the statically-set nameservers and search list.
///
/// `crate::resolver` carries why this is desired state while an interface's
/// leased `dns` is not, and why the file it becomes has to exist at all.
fn read_resolver(
    src: &str,
    network: &dyn TableLike,
    key_path: &str,
    entry: &Item,
) -> Result<Resolver, ConfigError> {
    let Some(table) = entry.as_table_like() else {
        return Err(ConfigError::WrongType {
            path: key_path.to_owned(),
            at: value_location(src, network, Resolver::TABLE),
            expected: "a table of resolver settings",
            found: entry.type_name(),
        });
    };
    refuse_unknown_keys(src, table, key_path, Resolver::DECLARED_KEYS)?;

    let mut nameservers = Vec::new();
    if let Some(item) = table.get("nameservers") {
        let field = path(key_path, "nameservers");
        let list = item.as_array().ok_or_else(|| ConfigError::WrongType {
            path: field.clone(),
            at: value_location(src, table, "nameservers"),
            expected: Resolver::RULE,
            found: item.type_name(),
        })?;
        for value in list {
            let text = value.as_str().ok_or_else(|| ConfigError::WrongType {
                path: field.clone(),
                at: value_location(src, table, "nameservers"),
                expected: Resolver::RULE,
                found: value.type_name(),
            })?;
            let address: Ipv4Addr = text.parse().map_err(|_| ConfigError::InvalidValue {
                path: field.clone(),
                at: value_location(src, table, "nameservers"),
                detail: Resolver::RULE.to_owned(),
            })?;
            nameservers.push(address);
        }
        // **Refused rather than truncated.** musl reads three and ignores the
        // rest, and `ADR-007` §3.6 forbids accepting an input the system
        // discards — a fourth nameserver that is written and never asked is a
        // configuration an operator would believe.
        if nameservers.len() > Resolver::MAX {
            return Err(ConfigError::InvalidValue {
                path: field,
                at: value_location(src, table, "nameservers"),
                detail: Resolver::RULE.to_owned(),
            });
        }
    }

    let mut search = Vec::new();
    if let Some(item) = table.get("search") {
        let field = path(key_path, "search");
        let list = item.as_array().ok_or_else(|| ConfigError::WrongType {
            path: field.clone(),
            at: value_location(src, table, "search"),
            expected: SearchDomain::RULE,
            found: item.type_name(),
        })?;
        for value in list {
            let text = value.as_str().ok_or_else(|| ConfigError::WrongType {
                path: field.clone(),
                at: value_location(src, table, "search"),
                expected: SearchDomain::RULE,
                found: value.type_name(),
            })?;
            let domain = SearchDomain::new(text).ok_or_else(|| ConfigError::InvalidValue {
                path: field.clone(),
                at: value_location(src, table, "search"),
                detail: SearchDomain::RULE.to_owned(),
            })?;
            search.push(domain);
        }
        // musl bounds the search list by BYTES, so this counts what will be
        // rendered rather than how many were written.
        let rendered: usize = search
            .iter()
            .map(|d| d.as_str().len() + 1)
            .sum::<usize>()
            .saturating_sub(1);
        if rendered > Resolver::SEARCH_BYTES {
            return Err(ConfigError::InvalidValue {
                path: field,
                at: value_location(src, table, "search"),
                detail: SearchDomain::RULE.to_owned(),
            });
        }
    }

    Ok(Resolver::new(nameservers, search))
}


/// `ssid` — the wireless network this interface joins, `ADR-004`.
///
/// **Optional, and its presence is what makes an interface wireless.**
/// `ADR-007` §7 forbids inferring `method` from the presence of `address`
/// because a DHCP intent beside a stale `address` would have nowhere to be
/// written; absence may mean something only when it means exactly one thing.
/// An absent SSID does: no interface joins a network without naming one, so
/// it means a wired interface and nothing else.
///
/// **There is no key for the credential and there must not be.** The key that
/// joins is a PMK derived from the passphrase and this SSID, kept in
/// `ADR-020`'s store under the interface's name (`ADR-004` §5), and a schema
/// with no key to write it under is what makes a document safe to read, diff
/// and paste into a bug report. An SSID is not a credential: it travels in the clear
/// whenever a device joins.
fn read_ssid(
    src: &str,
    settings: &dyn TableLike,
    key_path: &str,
) -> Result<Option<Ssid>, ConfigError> {
    const KEY: &str = "ssid";

    let Some(item) = settings.get(KEY) else {
        return Ok(None);
    };
    let full = path(key_path, KEY);
    let Some(text) = item.as_str() else {
        return Err(ConfigError::WrongType {
            path: full,
            at: value_location(src, settings, KEY),
            expected: Ssid::RULE,
            found: item.type_name(),
        });
    };
    // **The refusal carries the RULE and never the name.** `ADR-020` §4: the
    // document this was read from may hold other things, and an error that
    // echoed a line of it is one edit away from echoing the wrong one.
    Ssid::new(text).map(Some).map_err(|_| ConfigError::InvalidValue {
        path: full,
        at: value_location(src, settings, KEY),
        detail: Ssid::RULE.to_owned(),
    })
}

/// Read the optional `[volumes]` table — the storage this machine's
/// applications live on. `ADR-031` §7.1.
///
/// **Absent is a machine with only its built-in volume**, which is every
/// Rivet today, and is a different state from a table declaring none. Both
/// produce an empty list here and `ADR-031` §7.5 is why that is right: a
/// declaration is a statement about what should be attached, and a machine
/// with nothing declared is asking for nothing.
fn read_volumes(src: &str, root: &Table) -> Result<Vec<Volume>, ConfigError> {
    const PATH: &str = "volumes";
    const EXPECTED: &str = "a table of volumes, one sub-table per volume name";

    let Some(item) = root.get(PATH) else {
        return Ok(Vec::new());
    };

    let Some(table) = item.as_table_like() else {
        return Err(ConfigError::WrongType {
            path: PATH.to_owned(),
            at: value_location(src, root, PATH),
            expected: EXPECTED,
            found: item.type_name(),
        });
    };

    let mut volumes = Vec::new();

    for (name, entry) in table.iter() {
        let key_path = path(PATH, name);

        // The name becomes a directory under which programs are found and run,
        // so it is checked here rather than where a path is built from it.
        // Whether the volume EXISTS is not checked and cannot be: this crate
        // performs no I/O (`ADR-014` §3.1), and `ADR-031` §4.2 makes resolving
        // a GUID the machine's answer at the moment it is asked.
        let volume_name = VolumeName::new(name)
            .map_err(|fault| fault.into_error(key_path.clone(), key_location(src, table, name)))?;

        let Some(settings) = entry.as_table_like() else {
            return Err(ConfigError::WrongType {
                path: key_path,
                at: value_location(src, table, name),
                expected: "a table of settings for one volume",
                found: entry.type_name(),
            });
        };

        // **Unknown keys first**, which is this module's order everywhere and
        // is `ADR-031` §7.2's whole safety argument: `device`, `path` and
        // `label` are the keys a tired operator reaches for, and each is
        // refused by name here rather than silently ignored.
        refuse_unknown_keys(src, settings, &key_path, Volume::DECLARED_KEYS)?;

        let guid = read_guid(src, settings, &key_path)?;
        volumes.push(Volume::new(volume_name, guid));
    }

    // The fault no single entry can have. See `crate::volumes::check_together`.
    if let Err(fault) = crate::volumes::check_together(&volumes) {
        return Err(fault.into_error(PATH.to_owned(), value_location(src, root, PATH)));
    }

    Ok(volumes)
}

/// `guid` — required, and the only key a volume has. `ADR-031` §4.
fn read_guid(src: &str, table: &dyn TableLike, prefix: &str) -> Result<Guid, ConfigError> {
    const KEY: &str = "guid";
    const EXPECTED: &str = "the volume's partition GUID, the full 36-character form";

    let key_path = path(prefix, KEY);

    let Some(item) = table.get(KEY) else {
        return Err(ConfigError::MissingKey {
            path: key_path,
            expected: EXPECTED,
        });
    };

    let at = value_location(src, table, KEY);

    let Some(text) = item.as_str() else {
        return Err(ConfigError::WrongType {
            path: key_path,
            at,
            expected: EXPECTED,
            found: item.type_name(),
        });
    };

    Guid::new(text).map_err(|fault| fault.into_error(key_path, at))
}

/// Read one `[network.<name>]` entry.
///
/// Unknown keys first, then the required `method`, then whatever that method
/// admits — the same order as everywhere else in this module, and for the same
/// reason: an operator who wrote a key that does not exist should be told about
/// that key, not about a different one that is missing.
fn read_addressing(
    src: &str,
    table: &dyn TableLike,
    prefix: &str,
    interface: &InterfaceName,
    claimed: &mut Vec<(Ipv4Addr, InterfaceName)>,
) -> Result<Addressing, ConfigError> {
    refuse_unknown_keys(src, table, prefix, Interface::DECLARED_KEYS)?;

    match read_method(src, table, prefix)? {
        Method::Dhcp => {
            refuse_static_keys_under_dhcp(src, table, prefix)?;
            Ok(Addressing::Dhcp)
        }
        Method::Static => {
            let address = read_address(src, table, prefix)?;
            let gateway = read_gateway(src, table, prefix)?;

            // Two interfaces holding one address is two authorities over one
            // fact (`ADR-011`), and it is detectable here with no knowledge of
            // the machine at all — exactly like two services claiming one
            // socket in `read_listen`. Compared on the host address rather
            // than the whole CIDR, because `10.0.0.2/24` and `10.0.0.2/16` are
            // the same address on two links and the conflict is the address.
            let host = address.address();
            if let Some((_, owner)) = claimed.iter().find(|(a, _)| *a == host) {
                return Err(ConfigError::InvalidValue {
                    path: path(prefix, "address"),
                    at: value_location(src, table, "address"),
                    // `owner` is a key the operator chose and `host` is
                    // rendered from the parsed type, so neither can carry
                    // arbitrary document text (`ADR-020` §4).
                    detail: format!(
                        "claims {host}, which interface `{owner}` already claims; one address \
                         cannot be on two interfaces"
                    ),
                });
            }
            claimed.push((host, interface.clone()));

            // `rivet-core`'s own two rules — a gateway that is not on-link,
            // and a network or broadcast address offered as a host address.
            // Run here rather than left to `apply` because `ADR-007` §3.2 wants
            // a bad document rejected *at the edge* rather than producing a
            // machine that half-works, and because on a headless board the
            // late failure arrives over the link being brought up.
            let plan = StaticAddressing {
                interface: interface.clone(),
                address,
                gateway,
            };
            if let Err(why) = plan.validate() {
                // The key each rule is about, so the operator is sent to the
                // line they must change. `StaticAddressing::validate` checks
                // the address first on purpose — the gateway verdict is
                // computed *from* the address — and this mapping preserves
                // that.
                let key = match why {
                    CoreError::GatewayOffLink { .. } => "gateway",
                    _ => "address",
                };
                return Err(ConfigError::InvalidValue {
                    path: path(prefix, key),
                    at: value_location(src, table, key),
                    // `CoreError`'s message for both of these renders only
                    // parsed `Ipv4Addr` and `Ipv4Cidr` values, which cannot
                    // produce arbitrary bytes — the same exception
                    // `crate::services` makes for `Endpoint`'s `Display`.
                    detail: why.to_string(),
                });
            }

            Ok(Addressing::Static { address, gateway })
        }
    }
}

/// `method` — required. See [`crate::network`] for why it is a key rather than
/// an inference from which other keys are present.
fn read_method(src: &str, table: &dyn TableLike, prefix: &str) -> Result<Method, ConfigError> {
    const KEY: &str = "method";
    const EXPECTED: &str = "`dhcp` or `static` — how this interface gets its address";

    let key_path = path(prefix, KEY);

    let Some(item) = table.get(KEY) else {
        return Err(ConfigError::MissingKey {
            path: key_path,
            expected: EXPECTED,
        });
    };

    let at = value_location(src, table, KEY);

    let Some(text) = item.as_str() else {
        return Err(ConfigError::WrongType {
            path: key_path,
            at,
            expected: EXPECTED,
            found: item.type_name(),
        });
    };

    // The rule, never the text — `ADR-020` §4.
    let Some(method) = Method::parse(text) else {
        return Err(ConfigError::InvalidValue {
            path: key_path,
            at,
            detail: format!("is not a method; {}", Method::RULE),
        });
    };

    Ok(method)
}

/// Under `method = "dhcp"`, a static address is a contradiction rather than a
/// spare key.
///
/// `ADR-007` §3.6.5 is the whole argument: Rivet does not accept an input it
/// intends to discard. The lease carries an address and a gateway, so an
/// `address` written beside `method = "dhcp"` would have no effect — and a key
/// that loads successfully and does nothing is the same defect as a misspelled
/// one that loads successfully.
///
/// Reported in [`Interface::STATIC_ONLY_KEYS`] order rather than document
/// order, so that an operator who wrote both is sent to the address — which is
/// the one they meant something by.
fn refuse_static_keys_under_dhcp(
    src: &str,
    table: &dyn TableLike,
    prefix: &str,
) -> Result<(), ConfigError> {
    for key in Interface::STATIC_ONLY_KEYS {
        if table.get(key).is_some() {
            return Err(ConfigError::InvalidValue {
                path: path(prefix, key),
                at: value_location(src, table, key),
                detail: format!(
                    "belongs to `method = \"static\"`, and this interface asks for \
                     `method = \"dhcp\"`; a lease carries the {key}, so this line would have no \
                     effect. State one or the other"
                ),
            });
        }
    }
    Ok(())
}

/// `address` — required under `method = "static"`, and its absence is not a
/// shorthand for a lease.
fn read_address(src: &str, table: &dyn TableLike, prefix: &str) -> Result<Ipv4Cidr, ConfigError> {
    const KEY: &str = "address";
    const EXPECTED: &str = "an address written as a string, for example \"192.168.1.50/24\"";

    let key_path = path(prefix, KEY);

    let Some(item) = table.get(KEY) else {
        return Err(ConfigError::MissingKey {
            path: key_path,
            expected: EXPECTED,
        });
    };

    let at = value_location(src, table, KEY);

    let Some(text) = item.as_str() else {
        return Err(ConfigError::WrongType {
            path: key_path,
            at,
            expected: EXPECTED,
            found: item.type_name(),
        });
    };

    // `Ipv4Cidr::parse`'s own error carries the offending text, which this
    // crate may not print (`ADR-020` §4), so the rule is quoted instead. The
    // rule lives on the type in `rivet-core`, not here — `ADR-011`.
    Ipv4Cidr::parse(text).map_err(|_| ConfigError::InvalidValue {
        path: key_path,
        at,
        detail: format!("is not an address; {}", Ipv4Cidr::RULE),
    })
}

/// `gateway` — optional under `method = "static"`.
///
/// Absent means one thing, and `rivet-core::net::StaticAddressing` already
/// says what: bring the interface up and address it without making it the way
/// off this machine.
fn read_gateway(
    src: &str,
    table: &dyn TableLike,
    prefix: &str,
) -> Result<Option<Ipv4Addr>, ConfigError> {
    const KEY: &str = "gateway";
    const EXPECTED: &str = "a host address written as a string, for example \"192.168.1.1\"";

    let key_path = path(prefix, KEY);

    let Some(item) = table.get(KEY) else {
        return Ok(None);
    };

    let at = value_location(src, table, KEY);

    let Some(text) = item.as_str() else {
        return Err(ConfigError::WrongType {
            path: key_path,
            at,
            expected: EXPECTED,
            found: item.type_name(),
        });
    };

    // Not `Ipv4Cidr::parse`: a gateway is a host, not a subnet, and an operator
    // who wrote `"192.168.1.1/24"` is told the shape is wrong rather than
    // having the prefix quietly dropped. `cmd/rivet-netd` made the same
    // distinction on `argv`; the rule now lives on the type.
    let Ok(gateway) = text.parse::<Ipv4Addr>() else {
        return Err(ConfigError::InvalidValue {
            path: key_path,
            at,
            detail: format!("is not a gateway; {}", StaticAddressing::GATEWAY_RULE),
        });
    };

    Ok(Some(gateway))
}

/// Join a dotted path, so a nested key reports `net.wlan.ssid` rather than
/// `ssid`. Trivial today because the schema is one level deep; it is here so
/// that the second level does not invent its own convention.
fn path(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}.{name}")
    }
}

/// Read the optional `[repos]` table — `crate::repos`, ruled 2026-08-26.
///
/// One sub-table per repository, carrying exactly one key. **The name is
/// checked against `RepoName` and the id against `RepoId`**, and both refusals
/// quote the published rule rather than restating it.
///
/// **Two repositories may not share an id**, and that is refused here rather
/// than by the projection: it is the one thing about this table that cannot be
/// seen one entry at a time, and a machine asked to put one repository in two
/// places would have to pick, which is the supervisor deciding what the
/// operator meant.
fn read_repos(src: &str, root: &Table) -> Result<Vec<Repo>, ConfigError> {
    const PATH: &str = "repos";
    const EXPECTED: &str = "a table of repositories, one sub-table per repository";

    let Some(item) = root.get(PATH) else {
        return Ok(Vec::new());
    };
    let Some(table) = item.as_table_like() else {
        return Err(ConfigError::WrongType {
            path: PATH.to_owned(),
            at: value_location(src, root, PATH),
            expected: EXPECTED,
            found: item.type_name(),
        });
    };

    let mut out: Vec<Repo> = Vec::new();
    for (name, entry) in table.iter() {
        let repo_path = path(PATH, name);

        let Some(repo_name) = RepoName::new(name) else {
            return Err(ConfigError::InvalidValue {
                path: repo_path,
                at: value_location(src, table, name),
                detail: format!("`{name}` is not a repository name; it must be {}", RepoName::RULE),
            });
        };

        let Some(fields) = entry.as_table_like() else {
            return Err(ConfigError::WrongType {
                path: repo_path,
                at: value_location(src, table, name),
                expected: "a table of settings for this repository",
                found: entry.type_name(),
            });
        };

        refuse_unknown_keys(src, fields, &repo_path, Repo::DECLARED_KEYS)?;
        let id = read_repo_id(src, fields, &repo_path)?;

        // Two names for one repository is a machine that would have to choose
        // where to put it.
        if let Some(other) = out.iter().find(|r| r.id() == &id) {
            return Err(ConfigError::InvalidValue {
                path: path(&repo_path, "id"),
                at: value_location(src, fields, "id"),
                detail: format!(
                    "`{id}` is already `{}`'s identifier. An identifier IS the repository \
                     (ADR-025 4.1), so two names carrying one would be asking this machine to \
                     serve one repository from two directories - and it would have to choose. \
                     To RENAME `{}`, change its table name and leave the identifier alone; to \
                     add a second repository, mint a second identifier.",
                    other.name(),
                    other.name(),
                ),
            });
        }
        out.push(Repo::new(repo_name, id));
    }
    Ok(out)
}

/// `id` — **required inside a repository that appears.**
///
/// Absent is refused rather than minted: a machine that invented an identifier
/// would have invented a *new repository*, and the next `get document` would
/// show the operator an id he never wrote and cannot have meant.
fn read_repo_id(src: &str, fields: &dyn TableLike, repo_path: &str) -> Result<RepoId, ConfigError> {
    const KEY: &str = "id";
    let field_path = path(repo_path, KEY);

    let Some(item) = fields.get(KEY) else {
        return Err(ConfigError::MissingKey {
            path: field_path,
            expected: "the identifier `rivet-fleet add repo` minted for this repository. It is \
                       what makes a rename a rename rather than a delete and a create (ADR-025 \
                       4.1), and this machine will not invent one - an invented identifier is a \
                       new repository",
        });
    };
    let Some(text) = item.as_str() else {
        return Err(ConfigError::WrongType {
            path: field_path,
            at: value_location(src, fields, KEY),
            expected: "a quoted identifier",
            found: item.type_name(),
        });
    };
    RepoId::new(text).ok_or_else(|| ConfigError::InvalidValue {
        path: field_path,
        at: value_location(src, fields, KEY),
        detail: format!("`{text}` is not an identifier; {}", RepoId::RULE),
    })
}

/// Read the optional `[carry]` table — `crate::carry`, `.rulings/` R-159 R20.
///
/// One sub-table per product name, carrying exactly one key. **The name is
/// checked against `ServiceName`**, the identical grammar `[services.<name>]`
/// uses — a pin names the same product a service entry enables.
fn read_carry(src: &str, root: &Table) -> Result<Vec<Carry>, ConfigError> {
    const PATH: &str = "carry";
    const EXPECTED: &str = "a table of carried products, one sub-table per product name";

    let Some(item) = root.get(PATH) else {
        return Ok(Vec::new());
    };
    let Some(table) = item.as_table_like() else {
        return Err(ConfigError::WrongType {
            path: PATH.to_owned(),
            at: value_location(src, root, PATH),
            expected: EXPECTED,
            found: item.type_name(),
        });
    };

    let mut out: Vec<Carry> = Vec::new();
    for (name, entry) in table.iter() {
        let carry_path = path(PATH, name);

        let Some(product) = ServiceName::new(name) else {
            return Err(ConfigError::InvalidKey {
                path: carry_path,
                at: key_location(src, table, name),
                rule: ServiceName::RULE,
            });
        };

        let Some(fields) = entry.as_table_like() else {
            return Err(ConfigError::WrongType {
                path: carry_path,
                at: value_location(src, table, name),
                expected: "a table of settings for this carried product",
                found: entry.type_name(),
            });
        };

        refuse_unknown_keys(src, fields, &carry_path, Carry::DECLARED_KEYS)?;
        let version = read_carry_version(src, fields, &carry_path)?;
        out.push(Carry::new(product, version));
    }
    Ok(out)
}

/// Read the optional `[dns]` table — `crate::dns`, `ADR-047` §8.
///
/// **`zone` is required once the table appears**, because a record is a label
/// and a label names nothing until it is placed under a zone. `ttl` and
/// `negative-ttl` are optional and are kept as written, so the canonical text
/// preserves an omission and a later change to a default reaches it.
fn read_dns(src: &str, root: &Table) -> Result<Option<Dns>, ConfigError> {
    const PATH: &str = "dns";
    const EXPECTED: &str = "a table naming the zone this machine answers for and its records";

    let Some(item) = root.get(PATH) else {
        return Ok(None);
    };
    let Some(table) = item.as_table_like() else {
        return Err(ConfigError::WrongType {
            path: PATH.to_owned(),
            at: value_location(src, root, PATH),
            expected: EXPECTED,
            found: item.type_name(),
        });
    };
    refuse_unknown_keys(src, table, PATH, Dns::DECLARED_KEYS)?;

    let zone_path = path(PATH, "zone");
    let Some(zone_item) = table.get("zone") else {
        return Err(ConfigError::MissingKey {
            path: zone_path,
            expected: Zone::RULE,
        });
    };
    let Some(zone_text) = zone_item.as_str() else {
        return Err(ConfigError::WrongType {
            path: zone_path,
            at: value_location(src, table, "zone"),
            expected: Zone::RULE,
            found: zone_item.type_name(),
        });
    };
    let Some(zone) = Zone::new(zone_text) else {
        return Err(ConfigError::InvalidValue {
            path: zone_path,
            at: value_location(src, table, "zone"),
            detail: format!("`{zone_text}` is not a zone this machine may answer for; it must be {}", Zone::RULE),
        });
    };

    let ttl = read_dns_ttl(src, table, PATH, "ttl")?;
    let negative_ttl = read_dns_ttl(src, table, PATH, "negative-ttl")?;

    let mut records = Vec::new();
    if let Some(item) = table.get("records") {
        let records_path = path(PATH, "records");
        let Some(entries) = item.as_table_like() else {
            return Err(ConfigError::WrongType {
                path: records_path,
                at: value_location(src, table, "records"),
                expected: "a table of records, one sub-table per label",
                found: item.type_name(),
            });
        };
        for (name, entry) in entries.iter() {
            let record_path = path(&records_path, name);
            let Some(label) = Label::new(name) else {
                return Err(ConfigError::InvalidKey {
                    path: record_path,
                    at: key_location(src, entries, name),
                    rule: Label::RULE,
                });
            };
            let Some(fields) = entry.as_table_like() else {
                return Err(ConfigError::WrongType {
                    path: record_path,
                    at: value_location(src, entries, name),
                    expected: "a table holding this name's address",
                    found: entry.type_name(),
                });
            };
            refuse_unknown_keys(src, fields, &record_path, Record::DECLARED_KEYS)?;
            let address_path = path(&record_path, "address");
            let Some(address_item) = fields.get("address") else {
                return Err(ConfigError::MissingKey {
                    path: address_path,
                    expected: Record::ADDRESS_RULE,
                });
            };
            let Some(address_text) = address_item.as_str() else {
                return Err(ConfigError::WrongType {
                    path: address_path,
                    at: value_location(src, fields, "address"),
                    expected: Record::ADDRESS_RULE,
                    found: address_item.type_name(),
                });
            };
            let Ok(address) = address_text.parse::<Ipv4Addr>() else {
                return Err(ConfigError::InvalidValue {
                    path: address_path,
                    at: value_location(src, fields, "address"),
                    detail: format!("`{address_text}` is not {}", Record::ADDRESS_RULE),
                });
            };
            let ttl = read_dns_ttl(src, fields, &record_path, "ttl")?;
            records.push(Record::new(label, address, ttl));
        }
    }
    Ok(Some(Dns::new(zone, ttl, negative_ttl, records)))
}

/// One optional TTL inside `[dns]` or a `[dns.records.<label>]`.
fn read_dns_ttl(
    src: &str,
    fields: &dyn TableLike,
    prefix: &str,
    key: &str,
) -> Result<Option<Ttl>, ConfigError> {
    let Some(item) = fields.get(key) else {
        return Ok(None);
    };
    let Some(raw) = item.as_integer() else {
        return Err(ConfigError::WrongType {
            path: path(prefix, key),
            at: value_location(src, fields, key),
            expected: Ttl::RULE,
            found: item.type_name(),
        });
    };
    Ttl::new(raw).map(Some).ok_or_else(|| ConfigError::InvalidValue {
        path: path(prefix, key),
        at: value_location(src, fields, key),
        detail: format!("{raw} is not {}", Ttl::RULE),
    })
}

/// `version` — **required inside a `[carry.<product>]` entry that appears.**
///
/// Absent is refused rather than defaulted: a pin with no version would be a
/// pin to nothing, indistinguishable from not writing the table at all
/// (`ADR-007` §3).
fn read_carry_version(
    src: &str,
    fields: &dyn TableLike,
    carry_path: &str,
) -> Result<String, ConfigError> {
    const KEY: &str = "version";
    let field_path = path(carry_path, KEY);

    let Some(item) = fields.get(KEY) else {
        return Err(ConfigError::MissingKey {
            path: field_path,
            expected: "the published version this product must be carried at, for example \
                       \"0.1.0\" — the same string `cargo rivet fleet publish` recorded",
        });
    };
    let Some(text) = item.as_str() else {
        return Err(ConfigError::WrongType {
            path: field_path,
            at: value_location(src, fields, KEY),
            expected: "a quoted version string",
            found: item.type_name(),
        });
    };
    if text.is_empty() {
        return Err(ConfigError::InvalidValue {
            path: field_path,
            at: value_location(src, fields, KEY),
            detail: "a version pin cannot be empty".to_owned(),
        });
    }
    Ok(text.to_owned())
}

/// Read the optional `hostname` key — `crate::hostname`, ruled 2026-08-26.
///
/// **`Ok(None)` is not "no name"**, and this is the third key in one day to
/// make that distinction. An absent key means *the operator said nothing*, so
/// the machine derives one from something only it can say about itself
/// (`rivet_core::hostname::derived`) — a machine nobody has named is still a
/// machine somebody has to reach.
///
/// **A top-level scalar rather than a table**, unlike `[kiosk]`. There is
/// exactly one fact here and a table with one key in it is a table that will
/// grow a second key for the wrong reason.
fn read_hostname(src: &str, root: &Table) -> Result<Option<Hostname>, ConfigError> {
    const KEY: &str = "hostname";

    let Some(item) = root.get(KEY) else {
        return Ok(None);
    };
    let Some(text) = item.as_str() else {
        return Err(ConfigError::WrongType {
            path: KEY.to_owned(),
            at: value_location(src, root, KEY),
            expected: Hostname::RULE,
            found: item.type_name(),
        });
    };
    Hostname::new(text).map(Some).ok_or_else(|| ConfigError::InvalidValue {
        path: KEY.to_owned(),
        at: value_location(src, root, KEY),
        detail: format!("`{text}` is not a hostname; it must be {}", Hostname::RULE),
    })
}

/// Read the optional `timezone` key — `rivet_core::timezone`, ruled 2026-09-02.
///
/// **Mirrors [`read_hostname`] exactly**, and that is the point rather than
/// laziness: both are one optional top-level scalar carrying one fact, and
/// `ADR-007` §7 gains nothing from two shapes for one shape of key.
///
/// **`Ok(None)` is UTC**, and it is the one absence in this parser that means a
/// *value* rather than a derivation or a fallback somebody else applies. See
/// `rivet_config::Document::timezone`.
///
/// **The refusal names the value.** `ADR-034` §5.3 forbids reporting a setting
/// as applied when it is not, and a zone name musl cannot map is the sharpest
/// case of that in the schema: nothing fails, the clock is simply UTC, and the
/// document says otherwise. So a value that cannot be a zone name is refused
/// here — where the operator is looking at it — rather than discovered months
/// later. What cannot be refused here is a well-formed name for a zone this
/// image has no file for; `rivet_core::timezone` says why, and the projection
/// on the machine is what reports it.
fn read_timezone(src: &str, root: &Table) -> Result<Option<Timezone>, ConfigError> {
    const KEY: &str = "timezone";

    let Some(item) = root.get(KEY) else {
        return Ok(None);
    };
    let Some(text) = item.as_str() else {
        return Err(ConfigError::WrongType {
            path: KEY.to_owned(),
            at: value_location(src, root, KEY),
            expected: Timezone::RULE,
            found: item.type_name(),
        });
    };
    Timezone::new(text).map(Some).ok_or_else(|| ConfigError::InvalidValue {
        path: KEY.to_owned(),
        at: value_location(src, root, KEY),
        detail: format!("`{text}` is not a time zone; it must be {}", Timezone::RULE),
    })
}

/// Read the optional top-level `mode` key — `crate::machine_mode`, added
/// 2026-09-13 (plan §5.2 item 2, `.rulings/` R-162).
///
/// **Named for the type it reads rather than the key**, so that a reader
/// does not have to guess which `mode` this function means.
///
/// **`Ok(None)` means the operator said nothing**, and — unlike
/// [`read_hostname`] and [`read_timezone`] — nothing is derived and nothing
/// else is composed from it: see `crate::machine_mode` for why.
fn read_machine_mode(src: &str, root: &Table) -> Result<Option<MachineMode>, ConfigError> {
    const KEY: &str = "mode";

    let Some(item) = root.get(KEY) else {
        return Ok(None);
    };
    let at = value_location(src, root, KEY);
    let Some(text) = item.as_str() else {
        return Err(ConfigError::WrongType {
            path: KEY.to_owned(),
            at,
            expected: MachineMode::RULE,
            found: item.type_name(),
        });
    };
    // The rule, never the text — `ADR-020` §4.
    let Some(mode) = MachineMode::parse(text) else {
        return Err(ConfigError::InvalidValue {
            path: KEY.to_owned(),
            at,
            detail: format!("is not a mode; {}", MachineMode::RULE),
        });
    };
    Ok(Some(mode))
}

/// Read the optional `fleet-role` key — `crate::fleet_role`, added
/// 2026-09-13 (plan §5.2 item 2, `.rulings/` R-162).
///
/// **Mirrors [`read_mode`] exactly**: an enumerated top-level scalar with no
/// counterpart to be inherited from, so absence is *the operator said
/// nothing* rather than a value.
///
/// # The retired `"stand-alone"` spelling still loads
///
/// A document written before `FleetRole::StandAlone` was removed (a ratchet
/// increment, `rivetos_register`'s own copy having gone first) may still
/// carry `fleet-role = "stand-alone"` on disk. It named no behaviour of its
/// own even when the variant existed — [`crate::fleet_role::listen_default`]
/// and [`crate::fleet_role::role_services`] paired it with `None` in every
/// arm — so the document must still parse, as `fleet_role: None`, exactly as
/// an absent key does, rather than being refused. This is checked BEFORE
/// [`FleetRole::parse`], which — like the register's own `FleetRole::parse`
/// — no longer recognises the word at all: that function is where an
/// operator's *new* word is judged, and this is where an *old document's* is
/// kept working, the same split `rivetos_register`'s
/// `deserialize_fleet_role` draws for the identical field.
fn read_fleet_role(src: &str, root: &Table) -> Result<Option<FleetRole>, ConfigError> {
    const KEY: &str = "fleet-role";

    let Some(item) = root.get(KEY) else {
        return Ok(None);
    };
    let at = value_location(src, root, KEY);
    let Some(text) = item.as_str() else {
        return Err(ConfigError::WrongType {
            path: KEY.to_owned(),
            at,
            expected: FleetRole::RULE,
            found: item.type_name(),
        });
    };
    if text == "stand-alone" {
        return Ok(None);
    }
    // The rule, never the text — `ADR-020` §4.
    let Some(role) = FleetRole::parse(text) else {
        return Err(ConfigError::InvalidValue {
            path: KEY.to_owned(),
            at,
            detail: format!("is not a fleet role; {}", FleetRole::RULE),
        });
    };
    Ok(Some(role))
}

/// `[applications.<name>]` — what each application is set to. `ADR-038` §4,
/// added 2026-09-02.
///
/// **This is the only reader in this crate that does not refuse what it does
/// not understand**, and the asymmetry is the ruling rather than an oversight.
///
/// An unrecognised key is an outright refusal everywhere
/// else, on the argument that a document silently carrying something nothing
/// reads is worse than one that will not load. `ADR-038` §4.4 inverts it here:
///
/// * `PLAN.md` §104.19 records that a document a machine will not parse can
///   **strand it**, and refusing to boot over a misspelled brightness key would
///   be a catastrophic answer to a trivial mistake.
/// * Elsewhere an unknown key means *this document is from a future
///   `rivet-config` cannot understand*. Here the platform explicitly does not
///   own the schema, so it means only that **one application was told something
///   it does not understand**.
///
/// # What it still refuses
///
/// The SHAPE. `applications` must be a table of tables — anything else is not
/// a misspelled setting, it is a document that does not mean what it says, and
/// that is `rivet-config`'s own schema again.
///
/// # What it keeps that it cannot check
///
/// **Every well-formed `key = "value"`, including for an application this
/// machine has never installed.** This crate runs on a workstation composing a
/// document for a machine it may never see, so it cannot know what is installed
/// or what a manifest declares. Checking a key against a declaration is the
/// supervisor's, at reload, where the manifests are — `ADR-038` §4.3's *"the
/// check moves, it does not vanish"*.
fn read_applications(src: &str, root: &Table) -> Result<Applications, ConfigError> {
    const PATH: &str = "applications";

    let Some(item) = root.get(PATH) else {
        return Ok(Applications::default());
    };
    let Some(table) = item.as_table_like() else {
        return Err(ConfigError::WrongType {
            path: PATH.to_owned(),
            at: value_location(src, root, PATH),
            expected: "a table of application names, each holding that application's settings",
            found: item.type_name(),
        });
    };

    let mut apps = Vec::new();
    let mut ignored = Vec::new();
    for (name, entry) in table.iter() {
        let Some(settings) = entry.as_table_like() else {
            // A shape failure, not a setting failure - but still reported
            // rather than fatal, because the containment ADR-038 §4.4 buys is
            // worth more than a refusal here would be.
            ignored.push(Ignored {
                application: name.to_string(),
                key: String::new(),
                why: Ignored::NOT_FLAT,
            });
            continue;
        };
        let mut values = std::collections::BTreeMap::new();
        for (key, value) in settings.iter() {
            match value.as_str() {
                Some(text) => {
                    values.insert(key.to_string(), text.to_string());
                }
                None => ignored.push(Ignored {
                    application: name.to_string(),
                    key: key.to_string(),
                    why: if value.as_table_like().is_some() {
                        Ignored::NOT_FLAT
                    } else {
                        // **Not always `NOT_TEXT`.** `enabled = true` earns a
                        // reason that names `[services.<name>]`, because the
                        // repair `NOT_TEXT` suggests — quote it — produces a
                        // document that loads and an application that is still
                        // off. See `Ignored::BELONGS_TO_THE_SERVICE`.
                        Ignored::why_not_text(key)
                    },
                }),
            }
        }
        // **An application named with an empty table is kept**, because it is a
        // statement: somebody opened it, and a document that dropped it would
        // not round-trip. `crate::serialise` writes it back.
        apps.push(ApplicationSettings::new(name.to_string(), values));
    }
    Ok(Applications::new(apps, ignored))
}

/// `[display]` - the hours the screen is lit. Added 2026-09-02, `ADR-039` §3.1.
///
/// **Both keys or neither**, which differs from `[splash]` above and matches
/// `[control-plane]`: those two keys are independent switches and these two are
/// one statement. `off` alone is not a partial answer, it is a screen with no
/// way back on - `crate::display` has the argument.
fn read_display(src: &str, root: &Table) -> Result<Option<Hours>, ConfigError> {
    const PATH: &str = "display";

    let Some(item) = root.get(PATH) else {
        return Ok(None);
    };
    let Some(table) = item.as_table_like() else {
        return Err(ConfigError::WrongType {
            path: PATH.to_owned(),
            at: value_location(src, root, PATH),
            expected: Display::EXPECTED,
            found: item.type_name(),
        });
    };
    refuse_unknown_keys(src, table, PATH, Display::DECLARED_KEYS)?;

    // **Both keys are required once the table exists**, which is
    // `read_control_plane`'s rule rather than `read_splash`'s: those two keys
    // are independent switches and either alone is a complete statement, while
    // these two are one statement in two halves. A screen told when to go dark
    // and never when to come back is one somebody has to visit, so the way to
    // say nothing is to omit the table.
    let read = |key: &'static str| -> Result<TimeOfDay, ConfigError> {
        let Some(item) = table.get(key) else {
            return Err(ConfigError::MissingKey {
                path: path(PATH, key),
                expected: Display::PAIR_RULE,
            });
        };
        let text = item.as_str().ok_or_else(|| ConfigError::WrongType {
            path: path(PATH, key),
            at: value_location(src, table, key),
            expected: TimeOfDay::RULE,
            found: item.type_name(),
        })?;
        TimeOfDay::parse(text)
            .map(Ok)
            .unwrap_or_else(|| Err(ConfigError::InvalidValue {
                path: path(PATH, key),
                at: value_location(src, table, key),
                detail: format!("`{text}` is not a time of day; it must be {}", TimeOfDay::RULE),
            }))
    };
    let off = read("off")?;
    let on = read("on")?;

    Hours::new(off, on)
        .map(Some)
        .ok_or_else(|| ConfigError::InvalidValue {
            path: PATH.to_owned(),
            at: value_location(src, root, PATH),
            detail: format!(
                "`off` and `on` are both {off}, which is a window of no width - it says \
                 neither \"never dark\" (omit `[display]`) nor \"never lit\", so it is refused \
                 rather than guessed at"
            ),
        })
}

/// `[splash]` — what the boot screen may say. Added 2026-08-30.
///
/// **Both keys are optional inside a table that is itself optional**, which
/// differs from `[control-plane]` below on purpose. That table has ONE key, so
/// an empty one states nothing and refusing it is right. This one has two
/// independent switches, and `[splash]` carrying only `endpoint = false` is a
/// perfectly clear statement about one of them.
fn read_splash(src: &str, root: &Table) -> Result<Option<Splash>, ConfigError> {
    const PATH: &str = "splash";
    const EXPECTED: &str = "a table of boot-screen settings";

    let Some(item) = root.get(PATH) else {
        return Ok(None);
    };
    let Some(table) = item.as_table_like() else {
        return Err(ConfigError::WrongType {
            path: PATH.to_owned(),
            at: value_location(src, root, PATH),
            expected: EXPECTED,
            found: item.type_name(),
        });
    };
    refuse_unknown_keys(src, table, PATH, Splash::DECLARED_KEYS)?;

    let default = Splash::default();
    let read = |key: &'static str, fallback: bool| -> Result<bool, ConfigError> {
        let Some(item) = table.get(key) else {
            return Ok(fallback);
        };
        item.as_bool().ok_or_else(|| ConfigError::WrongType {
            path: path(PATH, key),
            at: value_location(src, table, key),
            expected: Splash::RULE,
            found: item.type_name(),
        })
    };
    let endpoint = read("endpoint", default.endpoint())?;
    let version = read("version", default.version())?;
    Ok(Some(Splash::new(endpoint, version)))
}

/// `[kiosk]` — which application gets the screen. Added 2026-08-31.
///
/// **An empty `[kiosk]` is accepted and means nothing**, on `[splash]`'s rule
/// rather than `[control-plane]`'s: this table's one key is optional, so a
/// document that opens the table and says nothing has stated no more than one
/// that never opened it. `crate::serialise` writes neither.
fn read_kiosk(src: &str, root: &Table) -> Result<Option<Kiosk>, ConfigError> {
    const PATH: &str = "kiosk";
    const EXPECTED: &str = "a table naming which application holds the display";

    let Some(item) = root.get(PATH) else {
        return Ok(None);
    };
    let Some(table) = item.as_table_like() else {
        return Err(ConfigError::WrongType {
            path: PATH.to_owned(),
            at: value_location(src, root, PATH),
            expected: EXPECTED,
            found: item.type_name(),
        });
    };
    refuse_unknown_keys(src, table, PATH, Kiosk::DECLARED_KEYS)?;

    let auto_run = match table.get("auto-run") {
        None => None,
        Some(item) => Some(
            item.as_str()
                .ok_or_else(|| ConfigError::WrongType {
                    path: path(PATH, "auto-run"),
                    at: value_location(src, table, "auto-run"),
                    expected: Kiosk::RULE,
                    found: item.type_name(),
                })?
                .to_owned(),
        ),
    };
    let locked = match table.get("locked") {
        None => false,
        Some(item) => item.as_bool().ok_or_else(|| ConfigError::WrongType {
            path: path(PATH, "locked"),
            at: value_location(src, table, "locked"),
            expected: Kiosk::LOCKED_RULE,
            found: item.type_name(),
        })?,
    };
    let role = |key: &str| -> Result<Option<String>, ConfigError> {
        match table.get(key) {
            None => Ok(None),
            Some(item) => item
                .as_str()
                .map(|s| Some(s.to_owned()))
                .ok_or_else(|| ConfigError::WrongType {
                    path: path(PATH, key),
                    at: value_location(src, table, key),
                    expected: Kiosk::ROLE_RULE,
                    found: item.type_name(),
                }),
        }
    };
    let launcher = role("launcher")?;
    let compositor = role("compositor")?;
    Ok(Some(
        if locked {
            Kiosk::locked_on(auto_run)
        } else {
            Kiosk::new(auto_run)
        }
        .with_launcher(launcher)
        .with_compositor(compositor),
    ))
}

/// `[control-plane]` — `ADR-035` §7.2, added 2026-08-28.
///
/// **The address moved here from a build constant** the day TLS made a
/// non-loopback bind legal — `crate::control_plane` carries that argument and
/// the constant it replaced.
fn read_control_plane(src: &str, root: &Table) -> Result<Option<ControlPlane>, ConfigError> {
    const PATH: &str = "control-plane";
    const EXPECTED: &str = "a table of control-plane settings";
    const KEY: &str = "listen";

    let Some(item) = root.get(PATH) else {
        return Ok(None);
    };
    let Some(table) = item.as_table_like() else {
        return Err(ConfigError::WrongType {
            path: PATH.to_owned(),
            at: value_location(src, root, PATH),
            expected: EXPECTED,
            found: item.type_name(),
        });
    };
    refuse_unknown_keys(src, table, PATH, ControlPlane::DECLARED_KEYS)?;

    let key_path = path(PATH, KEY);
    let Some(item) = table.get(KEY) else {
        // **A table with nothing in it has stated no desired state**: absent
        // means nobody said, and the way to say nothing is to omit the table.
        return Err(ConfigError::MissingKey {
            path: key_path,
            expected: crate::control_plane::LISTEN_RULE,
        });
    };
    let Some(text) = item.as_str() else {
        return Err(ConfigError::WrongType {
            path: key_path,
            at: value_location(src, table, KEY),
            expected: crate::control_plane::LISTEN_RULE,
            found: item.type_name(),
        });
    };
    if !ControlPlane::is_address(text) {
        return Err(ConfigError::InvalidValue {
            path: key_path,
            at: value_location(src, table, KEY),
            detail: format!(
                "{text:?} is not {rule}",
                rule = crate::control_plane::LISTEN_RULE
            ),
        });
    }
    Ok(Some(ControlPlane::new(text.to_owned())))
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// A minimal valid document. Hand-composed, and labelled as such per
    /// `AGENTS.md` §4 — there is no other implementation of this format to
    /// capture a vector from, because Rivet is the only thing that writes it.
    const MINIMAL: &str = "version = 1\n";

    // ---- what is accepted -------------------------------------------------

    #[test]
    fn the_minimal_document_parses() {
        let doc = document(MINIMAL).expect("the minimal document is valid");
        assert_eq!(doc.version(), SCHEMA_VERSION);
    }

    #[test]
    fn comments_and_blank_lines_are_not_content() {
        let src = "\n# Rivet provisioning\n\nversion = 1   # the schema\n\n";
        let doc = document(src).expect("comments are not content");
        assert_eq!(doc.version(), SCHEMA_VERSION);
    }

    #[test]
    fn a_document_written_by_a_windows_editor_parses() {
        // `ADR-002` §4.1 — the boot partition is edited from the Windows host,
        // so CRLF is the normal case rather than the odd one.
        let doc = document("version = 1\r\n").expect("CRLF is what the host writes");
        assert_eq!(doc.version(), SCHEMA_VERSION);
    }

    #[test]
    fn parsing_the_same_text_twice_gives_the_same_document() {
        assert_eq!(document(MINIMAL), document(MINIMAL));
    }

    // ---- what is refused --------------------------------------------------

    #[test]
    fn an_empty_document_is_refused_rather_than_defaulted() {
        assert_eq!(
            document(""),
            Err(ConfigError::MissingKey {
                path: "version".to_owned(),
                expected:
                    "an integer, the schema version this document is written against",
            })
        );
    }

    #[test]
    fn a_document_of_only_comments_is_refused() {
        assert!(matches!(
            document("# nothing here\n"),
            Err(ConfigError::MissingKey { .. })
        ));
    }

    #[test]
    fn an_unknown_key_is_named_rather_than_ignored() {
        let src = "version = 1\nnonsense = true\n";
        let Err(ConfigError::UnknownKey { path, at, expected }) = document(src) else {
            panic!("an unknown key must be refused");
        };
        assert_eq!(path, "nonsense");
        assert_eq!(at, Some(Location { line: 2, column: 1 }));
        assert_eq!(expected, Document::DECLARED_KEYS);
    }

    #[test]
    fn an_unknown_table_is_an_unknown_key_too() {
        let src = "version = 1\n\n[net.wlan]\nssid = \"x\"\n";
        let Err(ConfigError::UnknownKey { path, at, .. }) = document(src) else {
            panic!("an undeclared table must be refused");
        };
        // The whole `net` subtree is undeclared, so `net` is the honest name
        // for what is wrong; naming `net.wlan.ssid` would imply `net` is fine.
        assert_eq!(path, "net");
        assert_eq!(at, Some(Location { line: 3, column: 2 }));
    }

    /// `[greeting]` was removed outright — schema, parser, serialiser, the
    /// `/etc/motd` projection, `rivet-fleet`'s compose/diff and the register's
    /// own field — 2026-09-13 (`.rulings/` R-168.1): refuse it, since no
    /// existing document needs accommodating. No compatibility shim and no
    /// schema version bump, so a document that still carries the table is
    /// refused by the ordinary unknown-key path, exactly as any other table
    /// this schema does not declare would be.
    /// This is the only mention of `greeting` left in this crate's code —
    /// break it by re-adding the schema key and watch it fail
    /// (`AGENTS.md` §4).
    #[test]
    fn a_document_carrying_a_removed_greeting_table_is_refused_as_unknown() {
        let src = "version = 1\n[greeting]\nmode = \"brief\"\n";
        let Err(ConfigError::UnknownKey { path, .. }) = document(src) else {
            panic!("a [greeting] table must be refused, not silently read");
        };
        assert_eq!(path, "greeting");
    }

    // -----------------------------------------------------------------------
    // `[network.resolver]` — ruled 2026-09-01 (a citation here named a PLAN
    // section that was never written)
    // -----------------------------------------------------------------------

    /// The reserved sub-table is read as a resolver and NOT as an interface,
    /// which is the whole exception this table's shape buys.
    #[test]
    fn the_reserved_resolver_table_is_not_read_as_an_interface() {
        let doc = document(
            "version = 1\n\
             [network.eth0]\n\
             method = \"static\"\n\
             address = \"192.168.50.66/24\"\n\
             [network.resolver]\n\
             nameservers = [\"192.168.50.1\", \"1.1.1.1\"]\n\
             search = [\"lan\"]\n",
        )
        .expect("a document with a resolver");

        let names: Vec<&str> = doc.network().iter().map(|i| i.name().as_str()).collect();
        assert_eq!(names, ["eth0"], "`resolver` was read as an interface");

        let resolver = doc.resolver().expect("the table was read");
        assert_eq!(
            resolver.nameservers(),
            [
                core::net::Ipv4Addr::new(192, 168, 50, 1),
                core::net::Ipv4Addr::new(1, 1, 1, 1)
            ]
        );
        assert_eq!(resolver.search().len(), 1);
        assert_eq!(resolver.search()[0].as_str(), "lan");
        assert_eq!(doc.nameservers().len(), 2);
    }

    /// A document with no resolver says nothing, which is what leaves a DHCP
    /// lease free to answer.
    #[test]
    fn a_document_with_no_resolver_names_no_nameserver() {
        let doc = document("version = 1\n").expect("a bare document");
        assert!(doc.resolver().is_none());
        assert!(doc.nameservers().is_empty());
    }

    /// musl reads three. A fourth is refused rather than written and ignored —
    /// `ADR-007` §3.6 forbids accepting an input the system discards.
    #[test]
    fn a_fourth_nameserver_is_refused_rather_than_truncated() {
        let doc = document(
            "version = 1\n\
             [network.resolver]\n\
             nameservers = [\"1.1.1.1\", \"1.0.0.1\", \"8.8.8.8\", \"8.8.4.4\"]\n",
        );
        assert!(
            matches!(doc, Err(ConfigError::InvalidValue { .. })),
            "a fourth nameserver was accepted: {doc:?}"
        );
    }

    /// `resolv.conf` has no quoting, so a search value carrying a newline is
    /// not a bad domain — it is a second directive, and the one it would write
    /// is a nameserver nobody asked for.
    #[test]
    fn a_search_domain_may_not_smuggle_a_nameserver() {
        let doc = document(
            "version = 1\n\
             [network.resolver]\n\
             search = [\"lan\\nnameserver 6.6.6.6\"]\n",
        );
        assert!(
            matches!(doc, Err(ConfigError::InvalidValue { .. })),
            "a search domain with a newline in it was accepted: {doc:?}"
        );
    }

    /// A misspelled key inside the table is refused by name, on the rule that
    /// a setting which parses and does nothing is worse than one that fails.
    #[test]
    fn an_unknown_resolver_key_is_refused_by_name() {
        let doc = document(
            "version = 1\n\
             [network.resolver]\n\
             nameserver = [\"1.1.1.1\"]\n",
        );
        let Err(ConfigError::UnknownKey { path, .. }) = doc else {
            panic!("a misspelled resolver key must be refused: {doc:?}");
        };
        assert_eq!(path, "network.resolver.nameserver");
    }

    /// `ADR-007` §3.1: identical state, identical bytes. The resolver has to
    /// survive a round trip through the serialiser or a machine that rewrites
    /// its own document would lose its nameservers.
    #[test]
    fn a_resolver_survives_a_round_trip() {
        let text = "version = 1\n\
                    [network.eth0]\n\
                    method = \"static\"\n\
                    address = \"192.168.50.66/24\"\n\
                    [network.resolver]\n\
                    nameservers = [\"192.168.50.1\"]\n\
                    search = [\"lan\", \"home.arpa\"]\n";
        let first = document(text).expect("a document with a resolver");
        let rendered = crate::serialise::document(&first);
        let second = document(&rendered).expect("the rendered document parses");
        assert_eq!(
            first.resolver(),
            second.resolver(),
            "the resolver did not survive:\n{rendered}"
        );
        assert_eq!(rendered, crate::serialise::document(&second));
    }

    /// **Absence is UTC and is not an error**, which is the whole of what a
    /// machine that has never been told a zone should do.
    #[test]
    fn a_document_that_says_nothing_about_time_keeps_utc() {
        let doc = document("version = 1\n").expect("a document with no timezone");
        assert_eq!(doc.timezone(), None);
    }

    #[test]
    fn a_zone_name_survives_a_round_trip_and_keeps_its_case() {
        let doc = document("version = 1\ntimezone = \"America/Chicago\"\n").expect("a zone");
        assert_eq!(
            doc.timezone().map(rivet_core::timezone::Timezone::as_str),
            Some("America/Chicago")
        );
        let rendered = crate::serialise::document(&doc);
        assert_eq!(document(&rendered).expect("re-parses").timezone(), doc.timezone());
    }

    /// **The refusal names the value the operator typed.** `ADR-034` §5.3: a
    /// setting must not be reported as applied when it is not, and a zone musl
    /// cannot map produces no failure at all — just a UTC clock under a
    /// document that says otherwise. This is where that is stopped.
    #[test]
    fn a_value_that_cannot_be_a_zone_name_is_refused_by_name() {
        for bad in [
            "america chicago",
            "CST6CDT,M3.2.0/2,M11.1.0/2",
            "/etc/passwd",
            "../../etc/shadow",
            "",
        ] {
            let src = format!("version = 1\ntimezone = \"{bad}\"\n");
            let Err(err) = document(&src) else {
                panic!("{bad:?} was not refused");
            };
            let ConfigError::InvalidValue { ref path, .. } = err else {
                panic!("{bad:?} was refused as {err:?}, not as an invalid value");
            };
            assert_eq!(path, "timezone");
            let sentence = err.to_string();
            assert!(
                sentence.contains(bad),
                "the refusal does not quote the value: {sentence}"
            );
            assert!(
                sentence.contains("America/Chicago"),
                "the refusal does not show what one looks like: {sentence}"
            );
        }
    }

    #[test]
    fn a_timezone_of_the_wrong_type_is_refused_rather_than_coerced() {
        let Err(ConfigError::WrongType { path, found, .. }) = document("version = 1\ntimezone = 7\n")
        else {
            panic!("an integer timezone must be refused");
        };
        assert_eq!(path, "timezone");
        assert_eq!(found, "integer");
    }

    #[test]
    fn a_dotted_key_under_an_undeclared_table_is_refused() {
        assert!(matches!(
            document("version = 1\nnet.dns = \"1.1.1.1\"\n"),
            Err(ConfigError::UnknownKey { .. })
        ));
    }

    #[test]
    fn a_misspelled_required_key_reports_the_typo_not_the_absence() {
        // This is the case the ordering in `document` exists for. Reporting
        // "version is missing" would send the operator to add a second key
        // beside the one they meant to write.
        let Err(ConfigError::UnknownKey { path, .. }) = document("verison = 1\n") else {
            panic!("a typo must be refused as a typo");
        };
        assert_eq!(path, "verison");
    }

    #[test]
    fn observed_state_has_nowhere_to_go() {
        // `ADR-007` §3.3 — the document never contains what the machine *is*.
        // The rule is structural: there is no such field, so a document that
        // tries to carry one is refused like any other unknown key.
        for src in [
            "version = 1\n[status]\nlink = \"up\"\n",
            "version = 1\nobserved_hostname = \"pi\"\n",
        ] {
            assert!(
                matches!(document(src), Err(ConfigError::UnknownKey { .. })),
                "{src:?} was not refused"
            );
        }
    }

    #[test]
    fn a_version_of_the_wrong_type_is_refused_rather_than_coerced() {
        let Err(ConfigError::WrongType { path, found, at, .. }) = document("version = \"1\"\n")
        else {
            panic!("a string version must be refused");
        };
        assert_eq!(path, "version");
        assert_eq!(found, "string");
        assert_eq!(at, Some(Location { line: 1, column: 11 }));
    }

    #[test]
    fn a_float_version_is_refused() {
        let Err(ConfigError::WrongType { found, .. }) = document("version = 1.0\n") else {
            panic!("a float version must be refused");
        };
        assert_eq!(found, "float");
    }

    #[test]
    fn a_table_where_a_version_belongs_is_refused() {
        let Err(ConfigError::WrongType { found, .. }) = document("[version]\nmajor = 1\n") else {
            panic!("a table version must be refused");
        };
        assert_eq!(found, "table");
    }

    #[test]
    fn a_negative_version_is_refused() {
        let Err(ConfigError::InvalidValue { path, detail, .. }) = document("version = -1\n") else {
            panic!("a negative version must be refused");
        };
        assert_eq!(path, "version");
        assert!(detail.contains("-1"), "{detail:?}");
    }

    #[test]
    fn version_zero_is_refused_because_versions_count_from_one() {
        assert!(matches!(
            document("version = 0\n"),
            Err(ConfigError::InvalidValue { .. })
        ));
    }

    #[test]
    fn a_version_larger_than_the_type_is_refused_rather_than_wrapping() {
        assert!(matches!(
            document("version = 4294967296\n"),
            Err(ConfigError::InvalidValue { .. })
        ));
    }

    #[test]
    fn an_unreadable_schema_version_is_refused_not_guessed_at() {
        let Err(ConfigError::UnsupportedSchemaVersion { found, supported, .. }) =
            document("version = 2\n")
        else {
            panic!("an unknown schema version must be refused");
        };
        assert_eq!(found, 2);
        assert_eq!(supported, SCHEMA_VERSION.get());
    }

    #[test]
    fn malformed_toml_is_refused_with_a_position() {
        let Err(ConfigError::Malformed { at, detail }) = document("version = \n") else {
            panic!("a truncated value must be refused");
        };
        assert!(at.is_some(), "the parser recorded no position");
        assert!(!detail.is_empty(), "the parser said nothing");
    }

    #[test]
    fn a_duplicate_key_is_refused_rather_than_last_one_winning() {
        // Two answers for one fact is the defect `ADR-011` names. TOML forbids
        // it; this test is here so that a future change of parser cannot
        // quietly start accepting it.
        assert!(matches!(
            document("version = 1\nversion = 1\n"),
            Err(ConfigError::Malformed { .. })
        ));
    }

    #[test]
    fn a_refusal_never_quotes_the_line_it_failed_on() {
        // `ADR-020` §4 — the provisioning file will carry a passphrase, and an
        // error that echoes its input is that passphrase in a log. Break this
        // by adding the offending text to a message and this test fails.
        let secret = "correct-horse-battery-staple";
        let src = format!("version = 1\npassphrase = \"{secret}\"\n");
        let Err(e) = document(&src) else {
            panic!("an unknown key must be refused");
        };
        let rendered = format!("{e}\n{e:?}");
        assert!(
            !rendered.contains(secret),
            "the diagnostic leaked the value: {rendered}"
        );
        // It still has to say *which* key, or the refusal is useless.
        assert!(rendered.contains("passphrase"), "{rendered}");
    }

    // ---- services: what the operator may say ------------------------------

    /// Hand-composed, and labelled as such per `AGENTS.md` §4. The service
    /// names are read from `ADR-017` §2.1's inventory rather than invented.
    const WITH_SERVICES: &str = "\
version = 1

[services.rivet-apid]
enabled = true

[services.rivet-netd]
enabled = true

[services.rivet-timed]
enabled = false
memory  = \"32M\"
";

    #[test]
    fn a_document_with_no_services_table_configures_no_services() {
        let doc = document(MINIMAL).expect("valid");
        assert!(doc.services().is_empty());
    }

    #[test]
    fn an_empty_services_table_is_accepted_and_configures_nothing() {
        let doc = document("version = 1\n[services]\n").expect("an empty table is a valid answer");
        assert!(doc.services().is_empty());
    }

    #[test]
    fn every_service_in_the_document_is_read() {
        let doc = document(WITH_SERVICES).expect("the fixture is valid");
        let names: Vec<&str> = doc.services().iter().map(|s| s.name().as_str()).collect();
        // Sorted, per `ADR-007` §3.1 — not in the order the text happened to
        // list them.
        assert_eq!(names, ["rivet-apid", "rivet-netd", "rivet-timed"]);
    }

    #[test]
    fn enabled_is_read_as_written_both_ways() {
        let doc = document(WITH_SERVICES).expect("valid");
        assert!(doc.service("rivet-apid").expect("present").enabled());
        assert!(!doc.service("rivet-timed").expect("present").enabled());
    }

    /// **The ruling, from the parser's side.** `listen` was the operator's
    /// until 2026-08-24 — `ADR-015` §5.2's *"'may listen on 443' becomes a line
    /// in the desired-state document"* — and is the manifest's now. A document
    /// that still states one is refused, and the refusal has to say that it was
    /// once right rather than treating the operator as confused.
    #[test]
    fn a_port_in_the_document_is_refused_and_the_refusal_names_the_ruling() {
        let src = "version = 1\n[services.rivet-apid]\nenabled = true\nlisten = [\"tcp:22\"]\n";
        let Err(e) = document(src) else {
            panic!("`listen` is the manifest's since 2026-08-24 and must be refused")
        };
        assert!(matches!(e, ConfigError::NotConfigurable { .. }), "{e:?}");
        let rendered = e.to_string();
        assert!(rendered.contains("services.rivet-apid.listen"), "{rendered}");
        assert!(rendered.contains("2026-08-24"), "{rendered}");
        assert!(rendered.contains("signed manifest"), "{rendered}");
        // And it must still say what the operator MAY write, or they are stuck.
        assert!(rendered.contains("enabled"), "{rendered}");
        assert!(rendered.contains("memory"), "{rendered}");
    }

    #[test]
    fn a_memory_cap_the_operator_set_is_read_in_bytes() {
        let doc = document(WITH_SERVICES).expect("valid");
        assert_eq!(
            doc.service("rivet-timed").expect("present").memory().map(MemoryLimit::bytes),
            Some(32 * 1024 * 1024)
        );
    }

    #[test]
    fn an_unstated_setting_is_none_rather_than_a_value_nobody_wrote() {
        let doc = document(WITH_SERVICES).expect("valid");
        let netd = doc.service("rivet-netd").expect("present");
        assert_eq!(netd.memory(), None, "the document states no cap for netd");
    }

    #[test]
    fn an_inline_table_is_the_same_document_as_a_section_header() {
        // TOML spells one thing two ways. Refusing a valid spelling would be
        // refusing a formatting choice, which is not Rivet's business.
        let sections = document("version = 1\n[services.x]\nenabled = true\n").expect("valid");
        let inline = document("version = 1\nservices = { x = { enabled = true } }\n")
            .expect("an inline table is the same document");
        assert_eq!(sections, inline);
        let dotted =
            document("version = 1\nservices.x.enabled = true\n").expect("dotted keys too");
        assert_eq!(sections, dotted);
    }

    #[test]
    fn a_service_the_machine_may_not_have_is_accepted_rather_than_guessed_about() {
        // This crate does no I/O (`ADR-014` §3.1), so it cannot know what is
        // installed; a hardcoded list here would be a second authority over
        // what services exist, which `ADR-017` §4.1 placed in the image and the
        // signed manifest. `apply` holds both and owes the existence check.
        let doc = document("version = 1\n[services.photo-index]\nenabled = true\n")
            .expect("configuration may be written for a service about to be installed");
        assert_eq!(doc.services().len(), 1);
    }

    // ---- services: what the operator may not say --------------------------

    #[test]
    fn naming_a_program_to_execute_is_refused_as_a_definition_not_as_a_typo() {
        // The line this whole table exists to hold. `ADR-015` §4.1: an operator
        // who could set this could redefine what a signed application executes.
        let src = "version = 1\n[services.rivet-apid]\nenabled = true\nexec = \"/bin/sh\"\n";
        let Err(ConfigError::NotConfigurable { path, at, expected, .. }) = document(src) else {
            panic!("`exec` must be refused as a definition key");
        };
        assert_eq!(path, "services.rivet-apid.exec");
        assert_eq!(at, Some(Location { line: 4, column: 1 }));
        assert_eq!(expected, Service::DECLARED_KEYS);
    }

    #[test]
    fn every_definition_key_is_refused_with_the_explanation_rather_than_bare() {
        for key in Service::DEFINITION_KEYS {
            let src = format!("version = 1\n[services.x]\nenabled = true\n{key} = \"anything\"\n");
            let Err(ConfigError::NotConfigurable { path, .. }) = document(&src) else {
                panic!("`{key}` must be refused as a definition key");
            };
            assert_eq!(path, format!("services.x.{key}"));
        }
    }

    #[test]
    fn a_restart_policy_is_refused_because_it_lives_inside_the_signature() {
        // `ADR-017` §4.1 moved `restart` and `ready` into `ADR-015` §3.2's
        // manifest specifically so they could not be rewritten outside it.
        for key in ["restart", "ready"] {
            let src = format!("version = 1\n[services.x]\nenabled = true\n{key} = \"always\"\n");
            assert!(
                matches!(document(&src), Err(ConfigError::NotConfigurable { .. })),
                "`{key}` was accepted as configuration"
            );
        }
    }

    #[test]
    fn a_key_that_is_neither_declared_nor_a_definition_is_an_ordinary_unknown_key() {
        // `network`, `state`, `provides` and `connects` are permission requests
        // (`ADR-015` §3.3) and where the operator's *grant* is recorded is
        // undecided, so nothing is pronounced on them here.
        for key in ["network", "state", "provides", "connects", "enable", "nonsense"] {
            let src = format!("version = 1\n[services.x]\nenabled = true\n{key} = \"y\"\n");
            let Err(ConfigError::UnknownKey { path, expected, .. }) = document(&src) else {
                panic!("`{key}` must be refused");
            };
            assert_eq!(path, format!("services.x.{key}"));
            assert_eq!(expected, Service::DECLARED_KEYS);
        }
    }

    #[test]
    fn a_definition_key_is_refused_before_the_missing_required_key() {
        // The operator wrote `exec`; telling them `enabled` is missing would
        // send them to fix the wrong thing.
        let src = "version = 1\n[services.x]\nexec = \"/bin/sh\"\n";
        assert!(matches!(
            document(src),
            Err(ConfigError::NotConfigurable { .. })
        ));
    }

    #[test]
    fn a_service_that_does_not_say_whether_it_should_run_is_refused() {
        // `ADR-017` §6 makes `enabled` the entire control surface, and the
        // definition has no counterpart to inherit from — checked against
        // `ADR-015` §3.2's manifest, which has no `enabled`. So absent means
        // nobody said, and `AGENTS.md` §3 refuses rather than guessing.
        let Err(ConfigError::MissingKey { path, expected }) =
            document("version = 1\n[services.rivet-apid]\nmemory = \"64M\"\n")
        else {
            panic!("a service with no `enabled` must be refused");
        };
        assert_eq!(path, "services.rivet-apid.enabled");
        assert!(expected.contains("true or false"), "{expected:?}");
    }

    #[test]
    fn an_empty_service_entry_is_refused_rather_than_meaning_nothing() {
        assert!(matches!(
            document("version = 1\n[services.rivet-apid]\n"),
            Err(ConfigError::MissingKey { .. })
        ));
    }

    #[test]
    fn enabled_of_the_wrong_type_is_refused_rather_than_coerced() {
        for (src, found) in [
            ("version = 1\n[services.x]\nenabled = \"true\"\n", "string"),
            ("version = 1\n[services.x]\nenabled = 1\n", "integer"),
        ] {
            let Err(ConfigError::WrongType { path, found: f, .. }) = document(src) else {
                panic!("{src:?} must be refused");
            };
            assert_eq!(path, "services.x.enabled");
            assert_eq!(f, found);
        }
    }

    #[test]
    fn a_services_key_that_is_not_a_table_is_refused() {
        let Err(ConfigError::WrongType { path, found, .. }) = document("version = 1\nservices = 1\n")
        else {
            panic!("a scalar `services` must be refused");
        };
        assert_eq!(path, "services");
        assert_eq!(found, "integer");
    }

    #[test]
    fn a_service_that_is_not_a_table_is_refused() {
        let Err(ConfigError::WrongType { path, found, .. }) =
            document("version = 1\n[services]\nrivet-apid = true\n")
        else {
            panic!("a scalar service must be refused");
        };
        assert_eq!(path, "services.rivet-apid");
        assert_eq!(found, "boolean");
    }

    #[test]
    fn a_service_name_that_would_escape_its_cgroup_is_refused() {
        // `ADR-017` §3.1.3 — the name is a cgroup path component, so `a/b`
        // would silently nest one service's cgroup inside another's.
        let Err(ConfigError::InvalidKey { path, at, rule }) =
            document("version = 1\n[services.\"a/b\"]\nenabled = true\n")
        else {
            panic!("a name with a separator must be refused");
        };
        assert_eq!(path, "services.a/b");
        assert!(at.is_some(), "the parser recorded no position");
        assert!(rule.contains("path separator"), "{rule:?}");
    }

    #[test]
    fn an_unusable_service_name_is_refused_however_it_is_written() {
        for name in ["\"\"", "\"..\"", "\"a b\"", "\"-lead\"", "\"café\""] {
            let src = format!("version = 1\n[services.{name}]\nenabled = true\n");
            assert!(
                matches!(document(&src), Err(ConfigError::InvalidKey { .. })),
                "{name} was accepted as a service name"
            );
        }
    }

    // Six tests stood here until 2026-08-24 and are deleted with the key they
    // were about: an endpoint that is not `tcp:<port>`, a `listen` that is not
    // an array of strings, one socket claimed by two services, the same claim
    // with one service disabled, one service listing a socket twice, and two
    // services on different ports.
    //
    // **The first two moved rather than died**: the grammar is
    // `rivet_core::package::application::Socket`'s now, and it is tested
    // there. The last four were about a whole document's claims being
    // comparable in one place, which is exactly what a document holding every
    // service's ports made possible and what the ruling gave up — see
    // `read_listen`'s deletion note above, and
    // `cmd/rivet-sysd`'s `applications` module, which is where a
    // collision can still be seen.

    #[test]
    fn a_memory_limit_that_is_not_one_is_refused_without_echoing_it() {
        for bad in ["lots", "128m", "0", "-1", "128 M", "128MB"] {
            let src = format!("version = 1\n[services.x]\nenabled = true\nmemory = \"{bad}\"\n");
            let Err(ConfigError::InvalidValue { path, detail, .. }) = document(&src) else {
                panic!("{bad:?} must be refused as a memory limit");
            };
            assert_eq!(path, "services.x.memory");
            assert!(detail.contains("multiples of 1024"), "{detail:?}");
        }
    }

    #[test]
    fn a_memory_limit_written_as_a_number_is_refused_rather_than_assumed_to_be_bytes() {
        // `ADR-015` §3.2 writes it as a string. Accepting an integer would be a
        // second spelling of one value, and the unit would be assumed.
        let Err(ConfigError::WrongType { path, found, .. }) =
            document("version = 1\n[services.x]\nenabled = true\nmemory = 128\n")
        else {
            panic!("an integer memory limit must be refused");
        };
        assert_eq!(path, "services.x.memory");
        assert_eq!(found, "integer");
    }

    #[test]
    fn a_service_defined_twice_is_refused_rather_than_last_one_winning() {
        assert!(matches!(
            document("version = 1\n[services.x]\nenabled = true\n[services.x]\nenabled = false\n"),
            Err(ConfigError::Malformed { .. })
        ));
    }

    #[test]
    fn observed_state_has_nowhere_to_go_inside_a_service_either() {
        // `ADR-007` §3.3 — the document says what the machine should be, never
        // what it is. There is no field for a pid, a state or a uptime.
        for key in ["pid", "state", "running", "ready_at", "uptime", "cgroup"] {
            let src = format!("version = 1\n[services.x]\nenabled = true\n{key} = \"y\"\n");
            assert!(
                matches!(
                    document(&src),
                    Err(ConfigError::UnknownKey { .. } | ConfigError::NotConfigurable { .. })
                ),
                "`{key}` was accepted into the desired-state document"
            );
        }
    }

    #[test]
    fn a_services_refusal_never_quotes_the_text_it_failed_on() {
        // The same property as `a_refusal_never_quotes_the_line_it_failed_on`,
        // extended to every string-valued key added by the services table. The
        // provisioning file carries a passphrase (`ADR-020` §4).
        //
        // A service *name* is deliberately outside this rule, because it is a
        // key rather than a value — see
        // `an_unusable_service_name_is_still_named_because_a_key_is_not_a_value`.
        let secret = "correct-horse-battery-staple";
        let sources = [
            // an endpoint that is not one
            format!("version = 1\n[services.x]\nenabled = true\nlisten = [\"{secret}\"]\n"),
            // a memory limit that is not one
            format!("version = 1\n[services.x]\nenabled = true\nmemory = \"{secret}\"\n"),
            // an undeclared key holding one
            format!("version = 1\n[services.x]\nenabled = true\npassphrase = \"{secret}\"\n"),
            // a definition key holding one
            format!("version = 1\n[services.x]\nenabled = true\nexec = \"{secret}\"\n"),
        ];
        for src in sources {
            let Err(e) = document(&src) else {
                panic!("{src:?} must be refused");
            };
            let rendered = format!("{e}\n{e:?}");
            assert!(
                !rendered.contains(secret),
                "the diagnostic leaked the value: {rendered}"
            );
        }
    }

    /// The secrecy rule (`ADR-020` §4) is about **values**, and this crate has
    /// always echoed **keys** — `an_unknown_key_message_lists_what_is_accepted_instead`
    /// requires it, because a refusal that will not say which key is useless.
    ///
    /// A service name is a key: the operator chose it, it names a cgroup and a
    /// socket, and it is not a place a credential can be. So a refused name is
    /// reported in full, and this test records that as a decision rather than
    /// leaving it to be discovered as an inconsistency.
    #[test]
    fn an_unusable_service_name_is_still_named_because_a_key_is_not_a_value() {
        let Err(e) = document("version = 1\n[services.\"bad name\"]\nenabled = true\n") else {
            panic!("an unusable name must be refused");
        };
        assert!(e.to_string().contains("bad name"), "{e}");
    }

    #[test]
    fn a_services_document_written_by_a_windows_editor_parses() {
        // `ADR-002` §4.1 — the boot partition is edited from the Windows host.
        let doc = document("version = 1\r\n[services.x]\r\nenabled = true\r\n")
            .expect("CRLF is what the host writes");
        assert_eq!(doc.services().len(), 1);
    }

    #[test]
    fn parsing_the_same_services_twice_gives_the_same_document() {
        assert_eq!(document(WITH_SERVICES), document(WITH_SERVICES));
    }

    #[test]
    fn the_order_services_appear_in_the_text_does_not_change_the_document() {
        // `ADR-007` §3.1 — the store is the authority and identical state must
        // be identical.
        let a = "version = 1\n[services.a]\nenabled = true\n[services.b]\nenabled = false\n";
        let b = "version = 1\n[services.b]\nenabled = false\n[services.a]\nenabled = true\n";
        assert_eq!(document(a), document(b));
    }

    // ---- network: how the machine is told its own address -----------------

    /// Hand-composed, and labelled as such per `AGENTS.md` §4. The interface
    /// names are the two a Raspberry Pi 4 Model B actually reported on
    /// 2026-08-22 (`ADR-004` §8.1, §8.2), not two names chosen to pass a test.
    const WITH_NETWORK: &str = "\
version = 1

[network.eth0]
method  = \"static\"
address = \"192.168.1.50/24\"
gateway = \"192.168.1.1\"

[network.wlan0]
method = \"dhcp\"
";

    #[test]
    fn a_document_with_no_network_table_configures_no_interface() {
        assert!(document(MINIMAL).expect("valid").network().is_empty());
    }

    #[test]
    fn an_empty_network_table_is_accepted_and_configures_nothing() {
        let doc = document("version = 1\n[network]\n").expect("an empty table is a valid answer");
        assert!(doc.network().is_empty());
    }

    #[test]
    fn every_interface_in_the_document_is_read_in_canonical_order() {
        let doc = document(WITH_NETWORK).expect("the fixture is valid");
        let names: Vec<&str> = doc.network().iter().map(|i| i.name().as_str()).collect();
        assert_eq!(names, ["eth0", "wlan0"]);
    }

    #[test]
    fn a_static_interface_becomes_the_cores_own_plan() {
        let doc = document(WITH_NETWORK).expect("valid");
        let plan = doc
            .interface("eth0")
            .and_then(Interface::static_plan)
            .expect("eth0 is static");
        assert_eq!(plan.interface.as_str(), "eth0");
        assert_eq!(plan.address.to_string(), "192.168.1.50/24");
        assert_eq!(plan.gateway.map(|g| g.to_string()).as_deref(), Some("192.168.1.1"));
    }

    /// The ruling's second sentence, as a test. DHCP parses and validates; it
    /// is not refused for being unimplemented, because that would conflate the
    /// schema with today's capability.
    #[test]
    fn a_document_asking_for_dhcp_parses_and_validates() {
        let doc = document(WITH_NETWORK).expect("valid");
        let wlan0 = doc.interface("wlan0").expect("present");
        assert_eq!(wlan0.addressing(), &Addressing::Dhcp);
        assert_eq!(wlan0.addressing().method(), Method::Dhcp);
        // And there is no plan for it, rather than an empty one.
        assert_eq!(wlan0.static_plan(), None);
    }

    #[test]
    fn a_static_interface_with_no_gateway_is_a_complete_statement() {
        let doc = document("version = 1\n[network.eth0]\nmethod=\"static\"\naddress=\"10.0.0.2/24\"\n")
            .expect("an addressed interface that is not the way off this machine");
        let plan = doc
            .interface("eth0")
            .and_then(Interface::static_plan)
            .expect("static");
        assert_eq!(plan.gateway, None);
    }

    #[test]
    fn the_order_interfaces_appear_in_the_text_does_not_change_the_document() {
        // `ADR-007` §3.1 — identical state, identical bytes.
        let a = "version = 1\n[network.eth0]\nmethod=\"dhcp\"\n[network.wlan0]\nmethod=\"dhcp\"\n";
        let b = "version = 1\n[network.wlan0]\nmethod=\"dhcp\"\n[network.eth0]\nmethod=\"dhcp\"\n";
        assert_eq!(document(a), document(b));
    }

    #[test]
    fn an_inline_network_table_is_the_same_document_as_a_section_header() {
        let sections = document("version = 1\n[network.eth0]\nmethod = \"dhcp\"\n").expect("valid");
        let inline = document("version = 1\nnetwork = { eth0 = { method = \"dhcp\" } }\n")
            .expect("an inline table is the same document");
        assert_eq!(sections, inline);
        let dotted = document("version = 1\nnetwork.eth0.method = \"dhcp\"\n").expect("dotted too");
        assert_eq!(sections, dotted);
    }

    #[test]
    fn a_network_document_written_by_a_windows_editor_parses() {
        // `ADR-002` §4.1 — the boot partition is edited from the Windows host.
        let doc = document("version = 1\r\n[network.eth0]\r\nmethod = \"dhcp\"\r\n")
            .expect("CRLF is what the host writes");
        assert_eq!(doc.network().len(), 1);
    }

    // ---- network: the contradiction, refused rather than resolved ---------

    /// **The refusal this table exists to make.** `ADR-007` §3.6.5 — Rivet
    /// does not accept an input it intends to discard. Break the check and this
    /// fails, which is what makes it a test rather than a paragraph.
    #[test]
    fn asking_for_dhcp_and_a_static_address_is_refused_rather_than_ranked() {
        let src = "version = 1\n[network.eth0]\nmethod = \"dhcp\"\naddress = \"10.0.0.2/24\"\n";
        let Err(ConfigError::InvalidValue { path, at, detail }) = document(src) else {
            panic!("a contradictory interface must be refused");
        };
        assert_eq!(path, "network.eth0.address");
        assert_eq!(at, Some(Location { line: 4, column: 11 }));
        assert!(detail.contains("static"), "{detail:?}");
        assert!(detail.contains("dhcp"), "{detail:?}");
        // It must say what to do, or the operator is stuck with a verdict.
        assert!(detail.contains("one or the other"), "{detail:?}");
    }

    #[test]
    fn a_gateway_beside_dhcp_is_refused_too_because_the_lease_carries_one() {
        let src = "version = 1\n[network.eth0]\nmethod = \"dhcp\"\ngateway = \"10.0.0.1\"\n";
        let Err(ConfigError::InvalidValue { path, detail, .. }) = document(src) else {
            panic!("a gateway under dhcp must be refused");
        };
        assert_eq!(path, "network.eth0.gateway");
        assert!(detail.contains("lease"), "{detail:?}");
    }

    /// When both static-only keys appear under `dhcp`, the address is reported.
    /// It is the one the operator meant something by, and the gateway is
    /// usually there because the address is.
    #[test]
    fn a_contradiction_naming_both_keys_reports_the_address() {
        let src = "version = 1\n[network.eth0]\nmethod=\"dhcp\"\ngateway=\"10.0.0.1\"\naddress=\"10.0.0.2/24\"\n";
        let Err(ConfigError::InvalidValue { path, .. }) = document(src) else {
            panic!("must be refused");
        };
        assert_eq!(path, "network.eth0.address");
    }

    // ---- network: everything else that is refused -------------------------

    #[test]
    fn an_interface_that_does_not_say_how_it_is_addressed_is_refused() {
        // Absence would have to mean either "dhcp" or "the operator forgot",
        // and it cannot mean both — see `crate::network`.
        let Err(ConfigError::MissingKey { path, expected }) =
            document("version = 1\n[network.eth0]\naddress = \"10.0.0.2/24\"\n")
        else {
            panic!("an interface with no `method` must be refused");
        };
        assert_eq!(path, "network.eth0.method");
        assert!(expected.contains("dhcp"), "{expected:?}");
        assert!(expected.contains("static"), "{expected:?}");
    }

    #[test]
    fn an_empty_interface_entry_is_refused_rather_than_meaning_dhcp() {
        assert!(matches!(
            document("version = 1\n[network.eth0]\n"),
            Err(ConfigError::MissingKey { .. })
        ));
    }

    #[test]
    fn a_static_interface_with_no_address_is_refused() {
        let Err(ConfigError::MissingKey { path, .. }) =
            document("version = 1\n[network.eth0]\nmethod = \"static\"\n")
        else {
            panic!("static with no address must be refused");
        };
        assert_eq!(path, "network.eth0.address");
    }

    #[test]
    fn a_method_outside_the_vocabulary_is_refused_without_echoing_it() {
        for bad in ["DHCP", "Static", "auto", "manual", "", "dhcpv6"] {
            let src = format!("version = 1\n[network.eth0]\nmethod = \"{bad}\"\n");
            let Err(ConfigError::InvalidValue { path, detail, at }) = document(&src) else {
                panic!("{bad:?} must be refused as a method");
            };
            assert_eq!(path, "network.eth0.method");
            assert!(at.is_some(), "no position for {bad:?}");
            assert!(detail.contains("dhcp"), "{detail:?}");
            assert!(detail.contains("static"), "{detail:?}");
        }
    }

    #[test]
    fn a_method_of_the_wrong_type_is_refused_rather_than_coerced() {
        for (src, found) in [
            ("version = 1\n[network.eth0]\nmethod = true\n", "boolean"),
            ("version = 1\n[network.eth0]\nmethod = 1\n", "integer"),
        ] {
            let Err(ConfigError::WrongType { path, found: f, .. }) = document(src) else {
                panic!("{src:?} must be refused");
            };
            assert_eq!(path, "network.eth0.method");
            assert_eq!(f, found);
        }
    }

    #[test]
    fn an_address_that_is_not_one_is_refused_without_echoing_it() {
        for bad in ["10.0.0.2", "192.168.001.1/24", "10.0.0.1/33", "", "nonsense", "10.0.0.1/"] {
            let src =
                format!("version = 1\n[network.eth0]\nmethod=\"static\"\naddress = \"{bad}\"\n");
            let Err(ConfigError::InvalidValue { path, detail, at }) = document(&src) else {
                panic!("{bad:?} must be refused as an address");
            };
            assert_eq!(path, "network.eth0.address");
            assert!(at.is_some(), "no position for {bad:?}");
            assert!(detail.contains("A.B.C.D/N"), "{detail:?}");
        }
    }

    #[test]
    fn an_address_written_as_a_number_is_refused_rather_than_guessed_at() {
        let Err(ConfigError::WrongType { path, found, .. }) =
            document("version = 1\n[network.eth0]\nmethod=\"static\"\naddress = 10\n")
        else {
            panic!("a numeric address must be refused");
        };
        assert_eq!(path, "network.eth0.address");
        assert_eq!(found, "integer");
    }

    /// A gateway with a prefix is a shape error. Dropping the prefix silently
    /// would accept a line the operator meant differently — the same rule
    /// `cmd/rivet-netd` applied to `--gateway`.
    #[test]
    fn a_gateway_written_as_a_subnet_is_refused() {
        let src = "version = 1\n[network.eth0]\nmethod=\"static\"\naddress=\"10.0.0.2/24\"\ngateway=\"10.0.0.1/24\"\n";
        let Err(ConfigError::InvalidValue { path, detail, .. }) = document(src) else {
            panic!("a gateway with a prefix must be refused");
        };
        assert_eq!(path, "network.eth0.gateway");
        assert!(detail.contains("no prefix length"), "{detail:?}");
    }

    /// `rivet-core`'s own two rules, reached from the document. These are the
    /// mistakes that would otherwise be discovered as a refused `RTM_NEWROUTE`
    /// after the address was already assigned, on a headless machine, over the
    /// link being brought up.
    #[test]
    fn a_gateway_outside_the_subnet_is_refused_at_parse_time() {
        let src = "version = 1\n[network.eth0]\nmethod=\"static\"\naddress=\"192.168.1.50/24\"\ngateway=\"192.168.2.1\"\n";
        let Err(ConfigError::InvalidValue { path, detail, at }) = document(src) else {
            panic!("an off-link gateway must be refused");
        };
        assert_eq!(path, "network.eth0.gateway");
        assert!(at.is_some(), "the parser recorded no position");
        assert!(detail.contains("192.168.2.1"), "{detail:?}");
        assert!(detail.contains("on-link"), "{detail:?}");
    }

    #[test]
    fn a_network_or_broadcast_address_is_refused_at_parse_time() {
        for bad in ["192.168.1.0/24", "192.168.1.255/24"] {
            let src = format!(
                "version = 1\n[network.eth0]\nmethod=\"static\"\naddress=\"{bad}\"\ngateway=\"192.168.1.1\"\n"
            );
            let Err(ConfigError::InvalidValue { path, detail, .. }) = document(&src) else {
                panic!("{bad} must be refused as a host address");
            };
            // The address, not the gateway — the gateway verdict is computed
            // from the address, so naming it would report a consequence.
            assert_eq!(path, "network.eth0.address");
            assert!(detail.contains("not a host address"), "{detail:?}");
        }
    }

    #[test]
    fn one_address_cannot_be_claimed_by_two_interfaces() {
        let src = "version = 1\n\
                   [network.eth0]\nmethod=\"static\"\naddress=\"10.0.0.2/24\"\n\
                   [network.wlan0]\nmethod=\"static\"\naddress=\"10.0.0.2/16\"\n";
        let Err(ConfigError::InvalidValue { path, detail, .. }) = document(src) else {
            panic!("two interfaces claiming one address must be refused");
        };
        assert_eq!(path, "network.wlan0.address");
        assert!(detail.contains("10.0.0.2"), "{detail:?}");
        assert!(detail.contains("`eth0`"), "{detail:?}");
    }

    #[test]
    fn two_interfaces_on_different_addresses_are_fine() {
        let src = "version = 1\n\
                   [network.eth0]\nmethod=\"static\"\naddress=\"10.0.0.2/24\"\n\
                   [network.wlan0]\nmethod=\"static\"\naddress=\"10.0.1.2/24\"\n";
        assert_eq!(document(src).expect("valid").network().len(), 2);
    }

    #[test]
    fn an_unknown_key_inside_an_interface_is_named_rather_than_ignored() {
        for key in ["dns", "mtu", "metric", "dhcp", "ip", "netmask", "nonsense"] {
            let src = format!("version = 1\n[network.eth0]\nmethod=\"dhcp\"\n{key} = \"y\"\n");
            let Err(ConfigError::UnknownKey { path, expected, .. }) = document(&src) else {
                panic!("`{key}` must be refused");
            };
            assert_eq!(path, format!("network.eth0.{key}"));
            assert_eq!(expected, Interface::DECLARED_KEYS);
        }
    }

    #[test]
    fn observed_state_has_nowhere_to_go_inside_an_interface_either() {
        // `ADR-007` §3.3 — the document says what the machine should be, never
        // what it is. There is no field for a carrier, a MAC or a lease.
        for key in ["carrier", "state", "link", "mac", "index", "lease_expires", "up"] {
            let src = format!("version = 1\n[network.eth0]\nmethod=\"dhcp\"\n{key} = \"y\"\n");
            assert!(
                matches!(document(&src), Err(ConfigError::UnknownKey { .. })),
                "`{key}` was accepted into the desired-state document"
            );
        }
    }

    #[test]
    fn an_interface_name_the_kernel_would_refuse_is_refused_here() {
        // `rivet-core` owns the rule — `dev_valid_name()` in `net/core/dev.c`
        // — and this crate quotes it rather than re-deriving it.
        let Err(ConfigError::InvalidKey { path, at, rule }) =
            document("version = 1\n[network.\"eth/0\"]\nmethod = \"dhcp\"\n")
        else {
            panic!("a name with a separator must be refused");
        };
        assert_eq!(path, "network.eth/0");
        assert!(at.is_some(), "the parser recorded no position");
        assert!(rule.contains("dev_valid_name"), "{rule:?}");
    }

    #[test]
    fn an_unusable_interface_name_is_refused_however_it_is_written() {
        for name in ["\"\"", "\".\"", "\"..\"", "\"eth 0\"", "\"eth:0\"", "\"aaaaaaaaaaaaaaaa\""] {
            let src = format!("version = 1\n[network.{name}]\nmethod = \"dhcp\"\n");
            assert!(
                matches!(document(&src), Err(ConfigError::InvalidKey { .. })),
                "{name} was accepted as an interface name"
            );
        }
    }

    #[test]
    fn a_network_key_that_is_not_a_table_is_refused() {
        let Err(ConfigError::WrongType { path, found, .. }) = document("version = 1\nnetwork = 1\n")
        else {
            panic!("a scalar `network` must be refused");
        };
        assert_eq!(path, "network");
        assert_eq!(found, "integer");
    }

    #[test]
    fn an_interface_that_is_not_a_table_is_refused() {
        let Err(ConfigError::WrongType { path, found, .. }) =
            document("version = 1\n[network]\neth0 = \"dhcp\"\n")
        else {
            panic!("a scalar interface must be refused");
        };
        assert_eq!(path, "network.eth0");
        assert_eq!(found, "string");
    }

    #[test]
    fn an_interface_defined_twice_is_refused_rather_than_last_one_winning() {
        assert!(matches!(
            document(
                "version = 1\n[network.eth0]\nmethod=\"dhcp\"\n[network.eth0]\nmethod=\"static\"\n"
            ),
            Err(ConfigError::Malformed { .. })
        ));
    }

    #[test]
    fn a_network_refusal_never_quotes_the_text_it_failed_on() {
        // The same property the services table has, extended to every
        // string-valued key this table adds. The provisioning file that carries
        // these interfaces also carries a wireless passphrase (`ADR-020` §4).
        let secret = "correct-horse-battery-staple";
        let sources = [
            format!("version = 1\n[network.eth0]\nmethod = \"{secret}\"\n"),
            format!("version = 1\n[network.eth0]\nmethod=\"static\"\naddress = \"{secret}\"\n"),
            format!(
                "version = 1\n[network.eth0]\nmethod=\"static\"\naddress=\"10.0.0.2/24\"\ngateway = \"{secret}\"\n"
            ),
            format!("version = 1\n[network.eth0]\nmethod=\"dhcp\"\npassphrase = \"{secret}\"\n"),
        ];
        for src in sources {
            let Err(e) = document(&src) else {
                panic!("{src:?} must be refused");
            };
            let rendered = format!("{e}\n{e:?}");
            assert!(
                !rendered.contains(secret),
                "the diagnostic leaked the value: {rendered}"
            );
        }
    }

    #[test]
    fn parsing_the_same_network_twice_gives_the_same_document() {
        assert_eq!(document(WITH_NETWORK), document(WITH_NETWORK));
    }

    // ---- the boundary this crate is on the right side of ------------------

    #[test]
    fn parsing_touches_no_filesystem_and_no_clock() {
        // `ADR-014` §3.1's standing check, stated as a test rather than as an
        // intention: every fixture in this module is a string literal. If this
        // module ever needs a path, the boundary has leaked.
        let doc = document(MINIMAL).expect("valid");
        assert_eq!(doc.version().get(), 1);
    }

    // ---- volumes ----------------------------------------------------------

    /// A well-formed GUID, in the form `blkid` prints. Hand-composed, and
    /// deliberately not a real one: `0fc63daf-8483-4772-8e79-3d69d8477de4` would
    /// have read plausibly and is the Linux filesystem *type* GUID, which no
    /// partition ever has as its unique GUID. A fixture a reader might copy
    /// should not teach the wrong constant.
    const GUID: &str = "1a2b3c4d-5e6f-4071-8293-a4b5c6d7e8f9";

    #[test]
    fn a_declared_volume_is_read_with_its_guid_lowercased() {
        let src = format!("version = 1\n[volumes.bulk]\nguid = \"{}\"\n", GUID.to_uppercase());
        let doc = document(&src).expect("a well-formed volume");
        assert_eq!(doc.volumes().len(), 1);
        assert_eq!(doc.volumes()[0].name().as_str(), "bulk");
        // Lowercased at construction, so two documents naming one partition in
        // two cases are one value — which is what makes the duplicate check
        // below able to see them.
        assert_eq!(doc.volumes()[0].guid().as_str(), GUID);
    }

    #[test]
    fn no_volumes_table_and_an_empty_one_are_the_same_answer() {
        let absent = document("version = 1\n").expect("valid");
        let empty = document("version = 1\n[volumes]\n").expect("valid");
        assert!(absent.volumes().is_empty());
        assert_eq!(absent, empty);
    }

    #[test]
    fn volumes_come_back_in_name_order_whatever_order_they_were_written_in() {
        let src = format!(
            "version = 1\n[volumes.zulu]\nguid = \"{GUID}\"\n\
             [volumes.alpha]\nguid = \"9f8e7d6c-5b4a-4392-8170-6f5e4d3c2b1a\"\n"
        );
        let doc = document(&src).expect("two well-formed volumes");
        let names: Vec<&str> = doc.volumes().iter().map(|v| v.name().as_str()).collect();
        assert_eq!(names, ["alpha", "zulu"]);
    }

    /// **`ADR-031` §7.2's whole safety argument.** `device`, `path` and `label`
    /// are the three keys a tired operator reaches for, and each names a way of
    /// identifying storage that is *not* stable across a reboot. Refused by
    /// name here rather than ignored, so the mistake is visible at the moment
    /// it is made rather than at the moment a disk enumerates differently.
    #[test]
    fn the_keys_an_operator_reaches_for_instead_of_a_guid_are_refused_by_name() {
        for key in ["device", "path", "label", "uuid"] {
            let src =
                format!("version = 1\n[volumes.bulk]\nguid = \"{GUID}\"\n{key} = \"/dev/sda2\"\n");
            let err = document(&src).expect_err("an unknown key is refused");
            let text = err.to_string();
            assert!(text.contains(key), "{key}: {text}");
        }
    }

    #[test]
    fn a_volume_without_a_guid_is_refused_rather_than_guessed_at() {
        assert_eq!(
            document("version = 1\n[volumes.bulk]\n"),
            Err(ConfigError::MissingKey {
                path: "volumes.bulk.guid".to_owned(),
                expected: "the volume's partition GUID, the full 36-character form",
            })
        );
    }

    #[test]
    fn the_built_in_volumes_name_cannot_be_declared_again() {
        let src = format!("version = 1\n[volumes.state]\nguid = \"{GUID}\"\n");
        let err = document(&src).expect_err("`state` is the built-in volume");
        assert!(err.to_string().contains("built-in"), "{err}");
    }

    #[test]
    fn a_name_that_is_not_one_path_component_is_refused() {
        for name in ["\"a/b\"", "\"..\"", "\".hidden\""] {
            let src = format!("version = 1\n[volumes.{name}]\nguid = \"{GUID}\"\n");
            assert!(
                document(&src).is_err(),
                "{name} became a directory name it should not have"
            );
        }
    }

    /// The fault no single entry can have, which is why it is checked once the
    /// whole table has been read.
    #[test]
    fn two_volumes_declaring_one_guid_are_refused() {
        let src = format!(
            "version = 1\n[volumes.bulk]\nguid = \"{GUID}\"\n\
             [volumes.media]\nguid = \"{}\"\n",
            GUID.to_uppercase()
        );
        let err = document(&src).expect_err("one device cannot have two names");
        let text = err.to_string();
        assert!(text.contains("bulk") && text.contains("media"), "{text}");
    }

    #[test]
    fn a_guid_that_is_the_wrong_shape_is_refused_with_the_rule_the_schema_publishes() {
        for bad in ["1a2b3c4d5e6f4071-8293-a4b5c6d7e8f9", "not-a-guid", "1a2b3c4d"] {
            let src = format!("version = 1\n[volumes.bulk]\nguid = \"{bad}\"\n");
            assert!(document(&src).is_err(), "{bad:?} was accepted as a GUID");
        }
    }

    #[test]
    fn a_volumes_table_that_is_not_a_table_is_refused() {
        let err = document("version = 1\nvolumes = \"bulk\"\n").expect_err("not a table");
        assert!(err.to_string().contains("volumes"), "{err}");
    }

    /// The two tables are independent, and a document may carry both. This is
    /// the case a real provisioning file will be: `rivet-netd` enabled in
    /// `[services]` and the interface it converges in `[network]`.
    #[test]
    fn a_document_may_configure_services_and_interfaces_at_once() {
        let src = "version = 1\n\
                   [services.rivet-netd]\nenabled = true\n\
                   [network.eth0]\nmethod=\"static\"\naddress=\"192.168.1.50/24\"\n";
        let doc = document(src).expect("both tables together are a valid document");
        assert!(doc.service("rivet-netd").is_some_and(Service::enabled));
        assert!(doc.interface("eth0").is_some());
    }

    // -----------------------------------------------------------------------
    // `[applications.<name>]` — ADR-038 §4
    // -----------------------------------------------------------------------

    /// **A bad setting must NEVER refuse the document**, and this is the
    /// containment that makes the region survivable — `ADR-038` §4.4.
    ///
    /// Everywhere else in this crate an unrecognised key is fatal.
    /// In here it is not, because `PLAN.md` §104.19 records
    /// that a document a machine will not parse can **strand it**, and
    /// refusing to boot over a misspelled brightness key would be a
    /// catastrophic answer to a trivial mistake.
    #[test]
    fn nothing_under_an_application_can_refuse_the_document() {
        let cases = [
            // A value that is not text - ADR-038 §10.2 keeps values textual.
            "version = 1\n[applications.a]\nbrightness = 60\n",
            "version = 1\n[applications.a]\nenabled = true\n",
            "version = 1\n[applications.a]\nratio = 1.5\n",
            "version = 1\n[applications.a]\nmany = [\"a\"]\n",
            // A nested table, which is a shape nothing delivers.
            "version = 1\n[applications.a.deeper]\nkey = \"v\"\n",
            // A key nothing declares. This crate cannot know that and keeps it;
            // the machine is where it is checked.
            "version = 1\n[applications.a]\nnonsense-key = \"v\"\n",
            // An application that does not exist. KEPT, because provisioning a
            // fleet before the software is installed is what the region is for.
            "version = 1\n[applications.never-installed]\nk = \"v\"\n",
        ];
        for text in cases {
            let doc = crate::parse::document(text);
            assert!(
                doc.is_ok(),
                "a setting refused the whole document, which ADR-038 §4.4 forbids: {text:?} \
                 gave {:?}",
                doc.err()
            );
        }
    }

    /// **What could not be carried is REPORTED**, not silently dropped — the
    /// other half of §4.4. An operator meets it in `get status` and in the
    /// reload's answer.
    #[test]
    fn what_could_not_be_carried_is_reported_by_application_and_key() {
        let doc = crate::parse::document(
            "version = 1\n[applications.aura-renderer]\nbrightness = \"60\"\nbad = 60\n",
        )
        .expect("must not refuse");
        let apps = doc.applications();
        // The good one survived.
        assert_eq!(
            apps.get("aura-renderer").and_then(|a| a.get("brightness")),
            Some("60")
        );
        // The bad one is named, with its application and a reason.
        let ignored = apps.ignored();
        assert_eq!(ignored.len(), 1, "{ignored:?}");
        assert_eq!(ignored[0].application, "aura-renderer");
        assert_eq!(ignored[0].key, "bad");
        assert!(ignored[0].why.contains("string"), "{}", ignored[0].why);
    }

    /// **`enabled = true` under an application is told which table it belongs
    /// to**, added 2026-09-05 after a port lost a boot to the generic message.
    ///
    /// The old reason was true and its suggested repair works — quote the value
    /// and the document loads — and the application is still not running. So
    /// the reader gets one step further and then has to find out from the
    /// journal.
    #[test]
    fn a_service_key_written_under_an_application_names_the_right_table() {
        for key in rivet_core::service::OPERATOR_KEYS {
            let text = format!("version = 1\n[applications.tempmon]\n{key} = true\n");
            let doc = crate::parse::document(&text).expect("must not refuse the document");
            let ignored = doc.applications().ignored();
            assert_eq!(ignored.len(), 1, "{ignored:?}");
            assert_eq!(ignored[0].key, *key);
            assert_eq!(
                ignored[0].why,
                crate::applications::Ignored::BELONGS_TO_THE_SERVICE,
                "`{key}` earned the generic quoted-string message"
            );
            assert!(ignored[0].why.contains("[services.<name>]"), "{}", ignored[0].why);
        }
        // And an ordinary mistyped value still earns the ordinary reason, which
        // now shows the repair rather than only naming the rule.
        let doc = crate::parse::document("version = 1\n[applications.a]\nbrightness = 60\n")
            .expect("must not refuse");
        let ignored = doc.applications().ignored();
        assert_eq!(ignored[0].why, crate::applications::Ignored::NOT_TEXT);
        assert!(ignored[0].why.contains('"'), "{}", ignored[0].why);
    }

    /// **A table for an application this machine does not have is KEPT.**
    /// `ADR-034` §5.1's common case is a machine that has never been powered
    /// on, and `ADR-038` §4.1 wants a setting written by `rivet-provision` at
    /// card-write time. A parser that dropped it would delete exactly that.
    #[test]
    fn settings_for_an_application_that_is_not_installed_survive() {
        let text = "version = 1\n[applications.not-yet]\nk = \"v\"\n";
        let doc = crate::parse::document(text).expect("must not refuse");
        assert_eq!(doc.applications().get("not-yet").and_then(|a| a.get("k")), Some("v"));
        assert!(doc.applications().ignored().is_empty());
    }

    /// The SHAPE is still this crate's, and a scalar where a table belongs is
    /// a document that does not mean what it says.
    #[test]
    fn applications_itself_must_still_be_a_table() {
        assert!(crate::parse::document("version = 1\napplications = \"nope\"\n").is_err());
    }
}
